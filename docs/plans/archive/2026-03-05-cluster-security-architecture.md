# Clawtex 8-Machine AI Cluster Security Architecture

Date: 2026-03-05
Author: Security Architecture Review
Status: Design Complete / Pending Implementation

---

## 1. Executive Summary

本文件為 Clawtex 8 機 AI 集群設計完整的安全架構。基於對現有程式碼的全面審計（`src/security/`、`src/tools/shell.rs`、`src/gateway.rs`、`src/telegram.rs`、`src/main.rs`），識別出以下關鍵風險區域並提出具體防護措施。

**當前安全狀態評估：中等風險**
- 已有：ChaCha20-Poly1305 加密、Shell 命令白名單、Telegram 用戶白名單、Credential Scrubbing、E-Stop、Autonomy Levels、RBAC 框架（未接線）
- 缺失：HTTP API 無認證、Ollama API 無保護、RBAC 未實際啟用、Secret rotation、網路隔離、稽核日誌

---

## 2. 威脅模型 (Threat Model)

### 2.1 資產清單

| 資產 | 位置 | 敏感度 | 說明 |
|------|------|--------|------|
| API Keys | `~/.clawtex/agents.toml` | 極高 | Telegram, Gemini, Twitter OAuth, Gmail SMTP, Stripe, Render |
| 加密金鑰 | `~/.clawtex/.secret_key` | 極高 | ChaCha20-Poly1305 主金鑰 |
| SSH 私鑰 | `~/.ssh/id_ed25519` | 極高 | 集群間認證 |
| 客戶資料 | SQLite DB (`core.db`) | 高 | Email 地址、Freelancing 聯絡人、CRM 資料 |
| 對話紀錄 | SQLite DB | 中 | 用戶與 Agent 的對話歷史 |
| 記憶資料 | `memory.db` | 中 | 語意記憶（可能含客戶資料） |
| 模型權重 | `/var/lib/ollama/models/` | 中 | 本地 LLM 模型 |
| 程式碼 | `clawtex-core/` | 中 | 業務邏輯、工具實作 |

### 2.2 威脅來源

| 威脅來源 | 可能性 | 影響 | 說明 |
|----------|--------|------|------|
| LAN 內部攻擊者 | 中 | 高 | 同一網路的裝置可存取 Ollama API |
| Telegram Bot Token 洩漏 | 中 | 極高 | 任何人可控制 Agent |
| Prompt Injection | 高 | 高 | LLM 輸出惡意指令 -> shell 執行 |
| SSH Key 被盜 | 低 | 極高 | 完整集群控制權 |
| HTTP API 未認證存取 | 高 | 高 | 任何人可觸發 Agent、Hand、E-Stop |
| 供應鏈攻擊 | 低 | 高 | 被污染的模型或依賴 |
| 內部人員 | 低 | 極高 | 擁有系統存取權的人員 |

### 2.3 攻擊面分析

```
                    Internet
                       |
            +----------+----------+
            |    Telegram API     |
            +----------+----------+
                       |
                  [Bot Token]        <-- 洩漏 = 完全控制
                       |
            +----------+----------+
            |    clawtex-core     |
            |    (Hub Machine)    |
            +----------+----------+
            |          |          |
       [HTTP:7878] [SSE/WS]  [Ollama:11434]
            |          |          |
    無認證 API    無認證     無認證, 0.0.0.0
            |          |          |
            +----------+----------+
                       |
                  LAN Network        <-- 同網路裝置可存取
                       |
            +----------+----------+
            |   7 Worker Machines  |
            |  (Ollama instances)  |
            +----------+----------+
                       |
              [SSH Key Auth Only]
```

---

## 3. Ollama API 安全

### 3.1 問題

Ollama 預設監聽 `0.0.0.0:11434`，無任何認證。LAN 上的任何裝置都可以：
- 呼叫 `/api/generate` 生成任意內容
- 呼叫 `/api/pull` 下載模型（佔用頻寬和儲存空間）
- 呼叫 `/api/delete` 刪除模型
- 透過大量請求進行 DoS 攻擊

### 3.2 方案比較

| 方案 | 優點 | 缺點 | 推薦 |
|------|------|------|------|
| **Tailscale ACL** | 零配置加密、自動 mTLS | 需要每台機器安裝 Tailscale | **首選** |
| **Nginx Reverse Proxy + API Key** | 靈活、可加 Rate Limit | 需維護 Nginx 設定、自簽憑證 | 備選 |
| **iptables/nftables** | 輕量、無額外元件 | 無加密、難維護、IP 可偽造 | 不推薦 |
| **Ollama OLLAMA_HOST=127.0.0.1** | 最簡單 | 只能本機存取、集群無法用 | 單機可用 |

### 3.3 推薦方案：Tailscale + Nginx API Key (雙層防護)

**第一層：Tailscale 網路隔離**

```bash
# 每台機器安裝 Tailscale
curl -fsSL https://tailscale.com/install.sh | sh
tailscale up --auth-key=tskey-auth-xxx

# Tailscale ACL Policy (JSON)
{
  "acls": [
    // Hub 可以存取所有 Worker 的 Ollama
    {
      "action": "accept",
      "src":    ["tag:hub"],
      "dst":    ["tag:worker:11434"]
    },
    // Hub SSH 到所有 Worker
    {
      "action": "accept",
      "src":    ["tag:hub"],
      "dst":    ["tag:worker:22"]
    },
    // Worker 之間互不通訊
    {
      "action": "deny",
      "src":    ["tag:worker"],
      "dst":    ["tag:worker:*"]
    }
  ],
  "tagOwners": {
    "tag:hub":    ["autogroup:admin"],
    "tag:worker": ["autogroup:admin"]
  }
}
```

**第二層：每台 Worker 的 Nginx API Key（防禦縱深）**

```nginx
# /etc/nginx/conf.d/ollama-proxy.conf
upstream ollama_backend {
    server 127.0.0.1:11434;
}

server {
    listen 11435 ssl;
    server_name _;

    # 自簽證書 (Tailscale 已加密，這層是防禦縱深)
    ssl_certificate     /etc/nginx/certs/ollama.crt;
    ssl_certificate_key /etc/nginx/certs/ollama.key;

    # API Key 驗證
    set $expected_key "CLAWTEX_OLLAMA_KEY_CHANGE_ME";

    location / {
        # 檢查 API Key
        if ($http_x_ollama_key != $expected_key) {
            return 401 '{"error": "unauthorized"}';
        }

        # 禁止危險操作（只允許 generate, chat, embed）
        # 禁止 pull, delete, copy, create
        location ~ ^/api/(pull|delete|copy|create) {
            return 403 '{"error": "forbidden"}';
        }

        proxy_pass http://ollama_backend;
        proxy_set_header Host $host;
        proxy_read_timeout 300s;  # LLM 推理可能很久
    }
}
```

**第三層：Ollama 自身綁定 localhost**

```bash
# /etc/systemd/system/ollama.service.d/override.conf
[Service]
Environment="OLLAMA_HOST=127.0.0.1:11434"
```

**Clawtex Provider 端修改**

```rust
// src/providers/ollama.rs — 新增 API Key header
fn build_request(&self, url: &str, body: &Value) -> reqwest::RequestBuilder {
    let mut req = self.client.post(url).json(body);
    if let Some(ref key) = self.api_key {
        req = req.header("X-Ollama-Key", key);
    }
    req
}
```

### 3.4 實施步驟

1. 所有 8 台機器安裝 Tailscale，設定 ACL
2. Worker 機器：Ollama 綁定 localhost + Nginx proxy + API Key
3. Hub 機器：`agents.toml` 中 Ollama URL 改為 Tailscale IP + port 11435
4. 驗證：從 Hub 可存取，從其他 LAN 裝置不可存取

---

## 4. Secret 管理

### 4.1 現狀分析

```
agents.toml (Hub):
  [telegram]
  bot_token = "7654321:AAH..."         # 明文！
  [twitter]
  consumer_key = "xxxx"                 # 明文！
  consumer_secret = "xxxx"              # 明文！
  [email]
  smtp_password = "app-password-here"   # 明文！

~/.clawtex/.secret_key:
  <hex-encoded 256-bit key>             # 檔案權限: 0600 (Unix) / ACL (Windows)
```

已有的 `SecretManager` (ChaCha20-Poly1305) 支持 `enc2:` 前綴加密值，但 **agents.toml 中大部分 secret 仍為明文**。

### 4.2 Secret 分類與存放策略

| Secret | 需要在 Hub | 需要在 Worker | 存放位置 |
|--------|-----------|--------------|----------|
| Telegram Bot Token | Yes | No | Hub `agents.toml` (enc2:) |
| Twitter OAuth | Yes | No | Hub `agents.toml` (enc2:) |
| Gmail SMTP Password | Yes | No | Hub `agents.toml` (enc2:) |
| Gemini/Groq API Key | Yes | No | Hub `agents.toml` (enc2:) |
| Stripe Secret Key | Yes | No | Hub `agents.toml` (enc2:) |
| Render API Key | Yes | No | Hub `agents.toml` (enc2:) |
| Ollama API Key | Yes | Yes (各自) | Hub: `agents.toml` (enc2:); Worker: Nginx config |
| SSH Private Key | Yes | No | `~/.ssh/id_ed25519` (0600) |
| ChaCha20 Master Key | Yes | No | `~/.clawtex/.secret_key` (0600) |

**關鍵原則：Worker 機器不存放任何 API Key（除了自身的 Ollama proxy key）。所有外部 API 呼叫由 Hub 發起。**

### 4.3 推薦方案：強制 enc2 加密 + 環境變數二選一

**Phase 1：強制加密現有明文 (立即實施)**

```bash
# 使用 CLI 加密所有 secret
clawtex-core encrypt-secret "7654321:AAH..."
# Output: enc2:a4b7c9d2e1f0...

# agents.toml 改為：
[telegram]
bot_token = "enc2:a4b7c9d2e1f0..."
```

新增啟動檢查：

```rust
// src/main.rs — 啟動時警告明文 secret
fn warn_plaintext_secrets(config: &toml::Value) {
    let sensitive_keys = [
        "bot_token", "consumer_key", "consumer_secret",
        "access_token", "access_token_secret", "smtp_password",
        "api_key", "secret_key",
    ];
    fn check(v: &toml::Value, path: &str, keys: &[&str], warnings: &mut Vec<String>) {
        match v {
            toml::Value::Table(t) => {
                for (k, val) in t {
                    let p = format!("{}.{}", path, k);
                    if keys.contains(&k.as_str()) {
                        if let toml::Value::String(s) = val {
                            if !s.starts_with("enc2:") && !s.is_empty() {
                                warnings.push(format!(
                                    "SECURITY WARNING: {} contains plaintext secret. Use `clawtex-core encrypt-secret` to encrypt it.",
                                    p
                                ));
                            }
                        }
                    }
                    check(val, &p, keys, warnings);
                }
            }
            _ => {}
        }
    }
    let mut warnings = Vec::new();
    check(config, "", &sensitive_keys, &mut warnings);
    for w in &warnings {
        tracing::warn!("{}", w);
    }
}
```

**Phase 2：環境變數覆寫 (Docker/CI 使用)**

已在 `docker-compose.yml` 中支持。確保環境變數優先於 TOML：

```rust
// Config resolution order:
// 1. Environment variable (e.g., TELEGRAM_BOT_TOKEN)
// 2. agents.toml value (decrypted if enc2:)
// 3. Default value
```

**Phase 3：Secret Rotation 策略**

| Secret | Rotation 週期 | 方法 |
|--------|--------------|------|
| Telegram Bot Token | 90 天 / 洩漏時 | BotFather `/revoke` + 更新 agents.toml |
| Twitter OAuth | 180 天 | Twitter Developer Portal 重新生成 |
| Gmail App Password | 90 天 | Google Account -> App Passwords |
| Stripe Secret Key | 90 天 | Stripe Dashboard -> Roll Key |
| Render API Key | 90 天 | Render Dashboard -> Regenerate |
| Ollama Proxy Key | 180 天 | 更新 Nginx config + agents.toml |
| ChaCha20 Master Key | 不輪換 | 輪換需重新加密所有 secret |
| SSH Key | 365 天 | `ssh-keygen` + 更新 authorized_keys |

自動提醒（整合到 cron）：

```toml
# 每月 1 號提醒 secret rotation
[[cron]]
name = "secret_rotation_reminder"
schedule = "0 9 1 * *"
action = { type = "agent", agent = "master", prompt = "Check if any API keys need rotation. Current rotation schedule: Telegram 90d, Twitter 180d, Gmail 90d, Stripe 90d, Render 90d, SSH 365d." }
```

---

## 5. 通訊加密

### 5.1 通訊矩陣

| 來源 | 目的 | 協議 | 加密 | 狀態 |
|------|------|------|------|------|
| User | Telegram API | HTTPS | TLS 1.3 | OK |
| Telegram API | clawtex-core | HTTPS (long-poll) | TLS 1.3 | OK |
| clawtex-core | Ollama (local) | HTTP | 無 | 可接受 (loopback) |
| clawtex-core | Ollama (Worker) | HTTP over Tailscale | WireGuard | OK (Tailscale) |
| clawtex-core | External APIs | HTTPS | TLS 1.3 | OK |
| clawtex-core | HTTP Client | HTTP | 無 | **高風險** |
| SSH | Workers | SSH | Ed25519 | OK |
| Browser | clawtex-core | HTTP/WS | 無 | **需修復** |

### 5.2 LAN 內 Ollama 通訊

**使用 Tailscale 後，LAN 內所有通訊自動走 WireGuard 加密。**不需要額外的 TLS。

若不使用 Tailscale，則需要 mTLS：

```bash
# 生成 CA
openssl req -x509 -newkey rsa:4096 -days 365 -nodes \
  -keyout ca.key -out ca.crt -subj "/CN=Clawtex CA"

# 為每台機器生成證書
for host in hub worker1 worker2 ...; do
    openssl req -newkey rsa:2048 -nodes -keyout ${host}.key \
      -out ${host}.csr -subj "/CN=${host}"
    openssl x509 -req -in ${host}.csr -CA ca.crt -CAkey ca.key \
      -CAcreateserial -out ${host}.crt -days 365
done
```

### 5.3 模型權重傳輸

```bash
# 使用 rsync over SSH (已有 Ed25519 key)
rsync -avz --progress \
  /var/lib/ollama/models/blobs/ \
  worker1:/var/lib/ollama/models/blobs/

# 或使用 Ollama 的 pull (從 Hub 作為 registry)
# Hub 上啟動 Ollama 做為 registry:
OLLAMA_HOST=0.0.0.0:11434  # 只在 Tailscale 網路上暴露
```

### 5.4 HTTP API 加密

clawtex-core 的 HTTP API (port 7878) 目前無 TLS。需要：

```nginx
# Hub 上的 Nginx (如果需要外部存取)
server {
    listen 443 ssl;
    ssl_certificate /etc/letsencrypt/live/clawtex.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/clawtex.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:7878;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }
}
```

---

## 6. HTTP API 認證 (Critical Gap)

### 6.1 現狀：幾乎全部無認證

目前的 HTTP 路由（`src/main.rs` L2032-2054）：

```
GET  /health              — 無認證 (可接受)
POST /llm/route           — 無認證 ❌ 高風險
POST /task                — 無認證 ❌
POST /task/:id/run        — 無認證 ❌
GET  /task/history         — 無認證 ❌
POST /agent/:name/run     — 無認證 ❌ 高風險
GET  /cluster/status      — 無認證 ❌
GET  /tools               — 無認證 ❌
GET  /hands               — 無認證 ❌
POST /hand/:name/run      — 無認證 ❌ 高風險
GET  /workspace/files     — 無認證 ❌
GET  /costs               — 無認證 ❌
GET  /revenue             — 無認證 ❌
GET  /dashboard           — Token (query param) ✓ (弱)
POST /estop               — 無認證 ❌ 高風險
DELETE /estop             — 無認證 ❌ 高風險
GET  /estop               — 無認證 ❌
SSE  /stream/agent/:name  — 無認證 ❌
WS   /ws/agent/:name      — 無認證 ❌
```

**只有 Dashboard 有 token 認證，且是以 query parameter 傳遞（會出現在 access log 中）。**

### 6.2 推薦方案：Bearer Token Middleware

```rust
// src/middleware/auth.rs — Axum middleware

use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::Response,
};

/// Routes that don't require authentication
const PUBLIC_ROUTES: &[&str] = &["/health"];

/// Extract and validate Bearer token
pub async fn require_auth(
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let path = request.uri().path().to_string();

    // Public routes skip auth
    if PUBLIC_ROUTES.iter().any(|r| path.starts_with(r)) {
        return Ok(next.run(request).await);
    }

    // Extract Bearer token
    let auth_header = request.headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !auth_header.starts_with("Bearer ") {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let token = &auth_header[7..];

    // Validate against configured API keys
    // (token validation logic here — compare against hashed tokens)
    if !validate_token(token) {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(next.run(request).await)
}
```

**agents.toml 配置：**

```toml
[core]
host = "127.0.0.1"
port = 7878

# API 認證 token (生成方式: openssl rand -hex 32)
api_tokens = [
    "enc2:xxxx",  # Hub 自身
    "enc2:yyyy",  # 外部整合 (CI/CD)
]
```

### 6.3 路由權限分級

| 權限等級 | 路由 | Token 類型 |
|----------|------|-----------|
| Public | `/health` | 無需 |
| Read | `/cluster/status`, `/tools`, `/hands`, `/costs`, `/revenue`, `/estop (GET)`, `/task/history` | read-only token |
| Execute | `/agent/*/run`, `/hand/*/run`, `/llm/route`, `/task`, `/task/*/run`, `/stream/*`, `/ws/*` | execute token |
| Admin | `/estop (POST/DELETE)`, `/dashboard`, `/workspace/files` | admin token |

---

## 7. RBAC 完整設計

### 7.1 現狀

`src/security/roles.rs` 已定義 `Role` (Owner/Admin/Operator/Viewer) 和 `RoleRegistry`，但 **未接線到任何地方**。Telegram handler (`src/main.rs`) 沒有檢查用戶角色。

### 7.2 Integration Plan

```rust
// AppState 新增 role_registry
struct AppState {
    // ... existing fields ...
    role_registry: Arc<RwLock<RoleRegistry>>,
}

// agents.toml 配置
[roles]
owner = "123456789"  # Telegram user ID
admins = ["987654321"]
operators = ["111222333"]
```

**Telegram Command 權限矩陣：**

| 指令 | Owner | Admin | Operator | Viewer |
|------|-------|-------|----------|--------|
| 一般對話 | Yes | Yes | Yes | Yes |
| `/status`, `/costs`, `/revenue` | Yes | Yes | Yes | Yes |
| `/hand`, `/hands` | Yes | Yes | Yes | No |
| `/estop`, `/resume` | Yes | Yes | Yes | No |
| `/cron add/remove` | Yes | Yes | No | No |
| `/setup` | Yes | No | No | No |
| `/product` | Yes | Yes | No | No |
| `/pipeline` | Yes | Yes | No | No |
| `/dashboard` | Yes | Yes | No | No |

**工具權限矩陣 (結合 AutonomyLevel)：**

| 工具 | Owner/Admin | Operator | Viewer | 需要 Approval |
|------|------------|----------|--------|--------------|
| shell | Yes | Yes (supervised) | No | Supervised 時 |
| file_read/glob/content_search | Yes | Yes | Yes | No |
| file_write/file_edit | Yes | Yes | No | Supervised 時 |
| web_search | Yes | Yes | Yes | No |
| http_request | Yes | Yes | No | Always (POST/PUT/DELETE) |
| email_send | Yes | No | No | Always |
| twitter | Yes | No | No | Always |
| blog_publish | Yes | No | No | Supervised 時 |
| memory_store/forget | Yes | Yes | No | No |
| memory_recall | Yes | Yes | Yes | No |
| delegate | Yes | Yes | No | No |
| computer_use/browser | Yes | No | No | Always |
| stripe | Yes | No | No | Always |
| render_deploy | Yes | No | No | Always |
| scaffold_saas | Yes | Yes | No | No |

### 7.3 Cron 任務權限

```rust
// Cron 任務以建立者的權限執行
struct CronJob {
    // ... existing fields ...
    created_by: String,      // Telegram user ID
    autonomy: AutonomyLevel, // 預設 Supervised
}

// 執行時檢查：
// 1. 建立者的 Role 是否仍有效
// 2. AutonomyLevel 限制工具使用
// 3. 需要 Approval 的工具 -> 發 Telegram 通知給 Owner
```

### 7.4 Hand 權限繼承

```rust
// Hand 執行繼承觸發者的權限
impl HandRunner {
    async fn run(&self, hand: &Hand, input: &str, triggered_by: &str) -> Result<Vec<PhaseOutput>> {
        let role = self.role_registry.get_role(triggered_by);
        let autonomy = match role {
            Role::Owner | Role::Admin => AutonomyLevel::Full,
            Role::Operator => AutonomyLevel::Supervised,
            Role::Viewer => AutonomyLevel::ReadOnly,
        };
        // Pass autonomy level to agent_runtime for this execution
        // ...
    }
}

// chain_to 繼承原始觸發者的權限
// Cron 觸發的 Hand 使用 cron 建立者的權限
```

---

## 8. 資料保護

### 8.1 客戶資料分類

| 類別 | 資料範例 | 保護等級 | 保留期限 |
|------|---------|----------|----------|
| PII (個資) | Email, 姓名, 電話 | 高（加密存儲） | 用途結束後 30 天 |
| 商業資料 | Freelancing 提案, 報價 | 中（存取控制） | 180 天 |
| 分析資料 | 市場分析, SEO 報告 | 低（一般保護） | 無限制 |
| 對話紀錄 | Telegram 對話 | 中 | 90 天自動清理 |

### 8.2 SQLite 資料庫加密

```rust
// 使用 SQLCipher 替代標準 SQLite
// Cargo.toml:
// rusqlite = { version = "0.31", features = ["bundled-sqlcipher"] }

// 開啟資料庫時設定加密金鑰
let conn = Connection::open(db_path)?;
conn.pragma_update(None, "key", &encryption_key)?;
```

### 8.3 記憶資料保護

```rust
// Memory store 中的 PII 標記
pub enum MemoryCategory {
    Core,
    Conversation,
    TaskResult,
    PII,               // 新增：包含個資的記憶
    Custom(String),
}

// PII 記憶自動加密存儲
impl MemoryStore {
    pub async fn store_pii(&self, key: &str, value: &str) -> Result<()> {
        let encrypted = self.secret_manager.encrypt(value)?;
        self.store(key, &encrypted, MemoryCategory::PII).await
    }

    // PII 記憶有自動過期
    pub async fn cleanup_expired_pii(&self) -> Result<usize> {
        // DELETE FROM memories WHERE category = 'pii' AND created_at < datetime('now', '-30 days')
    }
}
```

### 8.4 日誌中的敏感資料遮蔽

現有的 `scrub_credentials()` 只在工具輸出中使用。需要擴展到日誌層：

```rust
// src/logging.rs — 自訂 tracing layer
use tracing_subscriber::layer::SubscriberExt;

struct ScrubLayer;

impl<S: Subscriber> Layer<S> for ScrubLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        // 攔截日誌事件，對 message 欄位執行 scrub_credentials()
    }
}

// 初始化 tracing 時加入 ScrubLayer
tracing_subscriber::registry()
    .with(EnvFilter::from_default_env())
    .with(ScrubLayer)
    .with(tracing_subscriber::fmt::layer())
    .init();
```

**需要額外遮蔽的模式：**

```rust
// 新增到 SENSITIVE_PATTERNS regex
lazy_static! {
    static ref ADDITIONAL_PATTERNS: Vec<Regex> = vec![
        // Email 地址
        Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b").unwrap(),
        // 電話號碼 (台灣)
        Regex::new(r"\b09\d{8}\b").unwrap(),
        // 信用卡號
        Regex::new(r"\b\d{4}[\s-]?\d{4}[\s-]?\d{4}[\s-]?\d{4}\b").unwrap(),
        // Telegram user ID in logs (optional — 視需求)
        // Regex::new(r"user_id=\d{5,}").unwrap(),
    ];
}
```

### 8.5 GDPR / 台灣個資法合規

**台灣個人資料保護法 (個資法) 要求：**

| 要求 | 實施方案 | 狀態 |
|------|---------|------|
| 蒐集目的明確 | Outreach hand 說明蒐集目的 | 需實作 |
| 當事人同意 | Email 工具加入 opt-out 連結 | 需實作 |
| 資料最小化 | 只蒐集必要資料 | 部分實作 |
| 安全維護 | 加密存儲 + 存取控制 | 部分實作 |
| 資料刪除權 | `/forget <person>` Telegram 指令 | 需實作 |
| 資料可攜權 | `/export <person>` 匯出 JSON | 需實作 |
| 事故通報 | 72 小時內通報 (GDPR) | 需流程 |

**具體措施：**

```rust
// Email 工具必須包含 opt-out
impl EmailTool {
    fn append_footer(&self, body: &str) -> String {
        format!(
            "{}\n\n---\nTo unsubscribe: reply STOP\nData policy: https://your-domain.com/privacy",
            body
        )
    }
}

// Telegram 指令: /gdpr forget <email>
// 刪除所有包含該 email 的記憶 + 對話紀錄
```

---

## 9. Prompt Injection 防護

### 9.1 攻擊鏈分析

```
惡意 Prompt → LLM 推理 → 工具呼叫 → 系統操作
```

最危險路徑：
1. **Prompt Injection -> Shell Execution**：惡意輸入誘導 LLM 執行 `shell` 工具
2. **Indirect Prompt Injection**：web_search 回傳的網頁含惡意指令
3. **Tool Chaining**：先用 file_read 讀取 `.secret_key`，再用 http_request 外傳

### 9.2 現有防護

- Shell 命令白名單（`allowed_commands`）— 有效但可繞過（如 `python -c "import os; os.system('curl ...')"`)
- Workspace 限制（`workspace_only`）— 限制 file_read/write 路徑
- Credential Scrubbing — 防止 secret 出現在工具輸出中

### 9.3 額外防護措施

**9.3.1 Shell 工具加固**

```rust
// 禁止 shell 中的子命令注入
fn sanitize_command(cmd: &str) -> Result<&str, &str> {
    // 禁止管道、重導向、命令替換
    let dangerous_patterns = ["|", ">>", "<<", "$(", "`", "&&", "||", ";"];
    for pattern in &dangerous_patterns {
        if cmd.contains(pattern) {
            return Err("Command contains forbidden operator");
        }
    }
    Ok(cmd)
}

// 或改為：只允許單一命令 + 參數，不允許 shell expansion
// 使用 tokio::process::Command::new(base_cmd).args(split_args) 而非 sh -c
```

**9.3.2 Output Validation**

```rust
// 檢查 LLM 輸出中是否含有可疑的工具呼叫模式
fn detect_suspicious_tool_calls(tool_calls: &[ToolCall]) -> Vec<String> {
    let mut warnings = Vec::new();
    for tc in tool_calls {
        // 檢查 shell 命令是否嘗試存取敏感路徑
        if tc.name == "shell" {
            if let Some(cmd) = tc.args.get("command").and_then(|v| v.as_str()) {
                if cmd.contains(".secret_key") || cmd.contains("agents.toml") ||
                   cmd.contains("/etc/shadow") || cmd.contains("id_rsa") ||
                   cmd.contains("id_ed25519") {
                    warnings.push(format!("Suspicious shell command: {}", cmd));
                }
            }
        }
        // 檢查 http_request 是否嘗試外傳資料
        if tc.name == "http_request" {
            if let Some(url) = tc.args.get("url").and_then(|v| v.as_str()) {
                // 只允許白名單 URL
                if !is_allowed_url(url) {
                    warnings.push(format!("Suspicious HTTP request: {}", url));
                }
            }
        }
        // 檢查 file_read 是否嘗試讀取敏感檔案
        if tc.name == "file_read" {
            if let Some(path) = tc.args.get("path").and_then(|v| v.as_str()) {
                if path.contains(".secret_key") || path.contains("agents.toml") ||
                   path.contains(".ssh") || path.contains(".env") {
                    warnings.push(format!("Attempt to read sensitive file: {}", path));
                }
            }
        }
    }
    warnings
}
```

**9.3.3 Egress 控制**

```rust
// http_request 工具的 URL 白名單
[security]
allowed_http_domains = [
    "api.twitter.com",
    "api.stripe.com",
    "api.render.com",
    "api.telegram.org",
    "generativelanguage.googleapis.com",
    "api.groq.com",
    "api.openai.com",
    "api.anthropic.com",
]
```

---

## 10. Telegram Bot Token 洩漏應變計劃

### 10.1 偵測

```rust
// 監控異常活動
struct TelegramMonitor {
    // 追蹤每分鐘收到的訊息數
    message_rate: AtomicU64,
    // 追蹤來自未知用戶的請求
    unknown_user_attempts: AtomicU64,
}

// 如果 unknown_user_attempts > 10/min，發出警報
// 如果 message_rate > 100/min，可能是攻擊
```

### 10.2 應變步驟

1. **立即**：觸發 E-Stop (`/estop`)
2. **1 分鐘內**：在 BotFather 中 `/revoke` token
3. **5 分鐘內**：生成新 token，更新 `agents.toml`
4. **10 分鐘內**：檢查日誌確認攻擊者是否執行了任何工具
5. **如有工具執行**：
   - 檢查 shell 執行記錄
   - 檢查 file_read/write 操作
   - 檢查 http_request 外傳資料
   - 輪換所有可能洩漏的 API Key
6. **事後**：啟用未知用戶速率限制

### 10.3 預防

```toml
# agents.toml — 強制 user allowlist（已實作，確保非空）
[telegram]
bot_token = "enc2:..."
allowed_users = ["123456789"]  # 必須指定！空 = 拒絕所有
```

現有程式碼 (`telegram.rs` L112-128) 已實作 deny-by-default，但需要確認 **`allowed_users` 不為空** 時不會允許 `*` wildcard：

```rust
// 危險：不應允許 "*" wildcard
// 建議移除 wildcard 支援或加上 startup warning
if allowed == "*" {
    tracing::warn!("SECURITY: Telegram wildcard '*' allows ALL users. Consider restricting to specific user IDs.");
    return true;
}
```

---

## 11. SSH Key 安全

### 11.1 現有配置

- Ed25519 key (`~/.ssh/id_ed25519`)
- SSH agent forwarding (可能)
- `authorized_keys` 部署到所有 Worker

### 11.2 加固措施

```bash
# /etc/ssh/sshd_config (所有 Worker)
PermitRootLogin no
PasswordAuthentication no
PubkeyAuthentication yes
AuthorizedKeysFile .ssh/authorized_keys
MaxAuthTries 3
LoginGraceTime 30
AllowUsers clawtex

# 只允許 Tailscale IP 連入
ListenAddress 100.x.y.z  # Tailscale IP only

# 禁止 agent forwarding (防止 key 被中轉)
AllowAgentForwarding no
AllowTcpForwarding no
```

### 11.3 SSH Key 輪換自動化

```bash
#!/bin/bash
# scripts/rotate-ssh-keys.sh
# 在 Hub 上執行

NEW_KEY="$HOME/.ssh/id_ed25519_$(date +%Y%m)"
ssh-keygen -t ed25519 -f "$NEW_KEY" -N ""

# 部署到所有 Worker (需要舊 key 仍有效)
for worker in worker1 worker2 worker3 worker4 worker5 worker6 worker7; do
    ssh-copy-id -i "$NEW_KEY.pub" clawtex@$worker
done

# 驗證新 key 可用
for worker in worker1 worker2 worker3 worker4 worker5 worker6 worker7; do
    ssh -i "$NEW_KEY" clawtex@$worker "echo OK" || echo "FAILED: $worker"
done

# 移除舊 key (手動確認後執行)
echo "Run: ssh clawtex@<worker> 'sed -i.bak \"/<old-key-fingerprint>/d\" ~/.ssh/authorized_keys'"
```

---

## 12. 稽核日誌 (Audit Logging)

### 12.1 設計

```rust
// src/audit.rs
use chrono::Utc;
use rusqlite::{params, Connection};
use std::sync::Mutex;

pub struct AuditLog {
    conn: Mutex<Connection>,
}

#[derive(Debug)]
pub struct AuditEvent {
    pub timestamp: String,
    pub actor: String,          // Telegram user ID or "cron" or "system"
    pub action: String,         // "tool_execute", "hand_run", "estop", "config_change"
    pub resource: String,       // tool name, hand name, etc.
    pub details: String,        // JSON details
    pub result: String,         // "success", "denied", "error"
    pub ip_address: Option<String>,
}

impl AuditLog {
    pub async fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS audit_log (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp  TEXT NOT NULL,
                actor      TEXT NOT NULL,
                action     TEXT NOT NULL,
                resource   TEXT NOT NULL,
                details    TEXT DEFAULT '',
                result     TEXT NOT NULL,
                ip_address TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_log(timestamp);
            CREATE INDEX IF NOT EXISTS idx_audit_actor ON audit_log(actor);
            CREATE INDEX IF NOT EXISTS idx_audit_action ON audit_log(action);"
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn log(&self, event: AuditEvent) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO audit_log (timestamp, actor, action, resource, details, result, ip_address)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                event.timestamp, event.actor, event.action,
                event.resource, event.details, event.result, event.ip_address
            ],
        );
    }

    // 查詢最近 N 筆稽核記錄
    pub fn recent(&self, limit: usize) -> Vec<AuditEvent> { /* ... */ }

    // 查詢特定 actor 的記錄
    pub fn by_actor(&self, actor: &str, limit: usize) -> Vec<AuditEvent> { /* ... */ }
}
```

### 12.2 必須記錄的事件

| 事件 | 嚴重度 | 說明 |
|------|--------|------|
| 工具執行（shell, email, twitter, http_request, browser） | HIGH | 記錄命令/URL/收件人 |
| Hand 啟動/完成 | MEDIUM | 記錄 hand 名稱、觸發者、持續時間 |
| E-Stop 觸發/解除 | HIGH | 記錄觸發者 |
| 認證失敗 | HIGH | 記錄 IP、嘗試的 token/user |
| 設定變更 | HIGH | 記錄變更前後的值 (scrubbed) |
| Cron 任務執行 | MEDIUM | 記錄任務名稱、結果 |
| Approval 請求/回應 | HIGH | 記錄工具、描述、決定 |
| 未知用戶嘗試 | HIGH | 記錄 user ID、username |
| 檔案讀寫 | LOW | 記錄路徑 (workspace 外的需 HIGH) |
| Secret 解密 | MEDIUM | 記錄哪個 key 被解密（不記錄值） |

---

## 13. 合規考量

### 13.1 AI 工具使用合規

| 工具 | 合規風險 | 防護措施 |
|------|---------|----------|
| Claude Code (Anthropic) | 程式碼送到 Anthropic 伺服器 | 不送客戶資料、不送 secret |
| Gemini CLI (Google) | 同上 | 同上 |
| Codex (OpenAI) | 同上 | 同上 |
| Ollama (本地) | 無資料外傳 | 安全 |
| LM Studio (本地) | 無資料外傳 | 安全 |

**原則：涉及客戶 PII 的推理必須在本地 LLM 執行（Ollama/LM Studio）。**

```rust
// Provider router 的 PII 感知路由
impl ProviderRouter {
    fn route_for_task(&self, task: &str, contains_pii: bool) -> &str {
        if contains_pii {
            "ollama"  // 強制本地推理
        } else {
            "auto"    // 允許雲端 API
        }
    }
}
```

### 13.2 AI 生成內容著作權

- **Blog 文章**：AI 生成的內容在多數法域無著作權保護
- **Freelancing 提案**：AI 輔助撰寫，需人類審閱並修改
- **防護措施**：
  - Blog 文章加入 `ai_assisted: true` 元資料
  - Freelancing 提案需 Approval gate 人工審核

### 13.3 反垃圾郵件法規 (CAN-SPAM / 台灣)

| 要求 | 實施 |
|------|------|
| 明確發件人身份 | Email 工具設定固定 `from` 地址 |
| 退訂機制 | 每封郵件附加 opt-out 連結 |
| 不使用欺騙性標題 | LLM system prompt 中明確禁止 |
| 台灣：需有營業登記 | 商業用途需確認 |
| 速率限制 | Gmail: 500 封/天; 設定每小時上限 |

```rust
// Email 工具速率限制
[email]
max_per_hour = 20
max_per_day = 100
require_approval = true  # 每封 email 需要人工核准
```

---

## 14. 實施優先級與時間表

### Phase 1: Critical (第 1 週)

| 項目 | 工時 | 風險 |
|------|------|------|
| HTTP API Bearer Token 認證 | 4h | 消除無認證 API 存取 |
| 強制 enc2 加密所有明文 secret | 2h | 消除配置文件洩漏風險 |
| Ollama 綁定 localhost | 0.5h | 消除 LAN 內未授權存取 |
| RBAC 接線到 Telegram handler | 6h | 啟用角色權限控制 |
| Shell 命令子注入防護 | 2h | 防止管道/重導向繞過 |
| Startup plaintext secret warning | 1h | 提醒管理員加密 |

### Phase 2: Important (第 2 週)

| 項目 | 工時 | 風險 |
|------|------|------|
| Tailscale 部署 (8 台機器) | 4h | 集群網路加密隔離 |
| Nginx Ollama Proxy + API Key | 3h | Ollama API 認證 |
| 稽核日誌系統 | 4h | 可追蹤性 |
| 日誌 scrubbing layer | 3h | 防止 secret 洩漏到日誌 |
| file_read/shell 敏感路徑黑名單 | 2h | 防止讀取 secret 檔案 |
| HTTP egress URL 白名單 | 2h | 防止資料外傳 |

### Phase 3: Compliance (第 3-4 週)

| 項目 | 工時 | 風險 |
|------|------|------|
| SQLCipher 資料庫加密 | 4h | 靜態資料保護 |
| PII 記憶加密 + 自動過期 | 4h | 個資法合規 |
| Email opt-out 機制 | 2h | 反垃圾郵件合規 |
| `/gdpr forget` 指令 | 3h | 資料刪除權 |
| Blog AI 標記 | 1h | 著作權合規 |
| Secret rotation cron 提醒 | 1h | 持續安全維護 |
| SSH 加固 (所有 Worker) | 2h | 深度防禦 |
| Telegram unknown-user 速率限制 | 2h | Bot token 洩漏緩解 |

---

## 15. 安全監控儀表板

整合到現有 `/dashboard` 頁面：

```
Security Status:
  [OK] Telegram user allowlist: 1 user
  [OK] Secrets: 8/8 encrypted (enc2:)
  [OK] Ollama: bound to localhost
  [OK] Shell allowlist: 12 commands
  [WARN] HTTP API: auth disabled
  [WARN] RBAC: defined but not wired
  [OK] E-Stop: ready
  [OK] Credential scrubbing: enabled

Recent Security Events:
  2026-03-05 14:30 — shell executed: git status (user: 123456789)
  2026-03-05 14:25 — unknown user blocked: 999888777
  2026-03-05 14:20 — hand started: seo_content (user: 123456789)
```

---

## 16. 架構總圖

```
                         Internet
                            |
                    [Telegram API]
                            |
                      TLS 1.3 + Bot Token
                            |
              +─────────────+─────────────+
              |        Hub Machine         |
              |                            |
              |  clawtex-core (port 7878)  |
              |  ┌────────────────────┐    |
              |  │ Auth Middleware     │    |  Bearer Token
              |  │ RBAC Check         │    |  Role-based
              |  │ Approval Gate      │    |  Human-in-loop
              |  │ Audit Logger       │    |  All actions
              |  │ E-Stop             │    |  Emergency halt
              |  │ Credential Scrub   │    |  Output sanitize
              |  │ Secret Manager     │    |  ChaCha20-Poly1305
              |  └────────────────────┘    |
              |           |                |
              |  [Ollama localhost:11434]   |
              |                            |
              +────────────+───────────────+
                           |
                    Tailscale (WireGuard)
                           |
         ┌─────────┬───────+───────┬─────────┐
         |         |               |         |
    [Worker 1] [Worker 2] ... [Worker 7]
    Ollama     Ollama         Ollama
    localhost  localhost      localhost
       |          |              |
    Nginx      Nginx          Nginx
    API Key    API Key        API Key
    :11435     :11435         :11435

    Tailscale ACL:
    - Hub -> Worker:11435  ALLOW
    - Hub -> Worker:22     ALLOW
    - Worker -> Worker     DENY
    - * -> Worker:11434    DENY (localhost only)
```

---

## 17. 緊急應變流程 (Incident Response)

### Level 1: Bot Token 洩漏
1. `/estop` -> 停止所有操作
2. BotFather `/revoke` -> 失效舊 token
3. 檢查稽核日誌 -> 確認影響範圍
4. 更新 token + 重啟
5. 如有工具被執行 -> 升級到 Level 2

### Level 2: API Key 洩漏
1. E-Stop
2. 輪換所有受影響的 API Key
3. 檢查外部服務 (Stripe, Render) 是否有異常活動
4. 撤銷可疑的 Stripe payment links
5. 更新所有 agents.toml secret

### Level 3: SSH Key 洩漏 / 主機入侵
1. E-Stop 所有機器
2. 斷開受影響機器的網路
3. 從其他機器移除受影響的 authorized_keys
4. 生成新 SSH key pair
5. 重新部署 authorized_keys
6. 完整審計受影響機器

### Level 4: 資料外洩 (PII)
1. E-Stop
2. 72 小時內通報主管機關 (GDPR)
3. 通知受影響的個人
4. 確認外洩範圍和內容
5. 修復根因 + 加強防護

---

## 附錄 A: 現有安全元件程式碼位置

| 元件 | 檔案 | 狀態 |
|------|------|------|
| SecretManager (ChaCha20) | `src/security/secrets.rs` | 完成，已使用 |
| AutonomyLevel | `src/security/autonomy.rs` | 完成，未接線 |
| Role / RoleRegistry | `src/security/roles.rs` | 完成，未接線 |
| Credential Scrubbing | `src/tools/mod.rs` L148-170 | 完成，在工具輸出中使用 |
| Shell Allowlist | `src/tools/shell.rs` | 完成 |
| Telegram User Allowlist | `src/telegram.rs` L112-128 | 完成 |
| E-Stop | `src/estop.rs` | 完成 |
| Approval Gate | `src/approval.rs` | 完成 |
| Heartbeat | `src/estop.rs` L69-123 | 完成 |
| Dashboard Token | `src/main.rs` L457-461 | 部分（僅 dashboard） |
| HTTP API Auth | 無 | **缺失** |
| Audit Log | 無 | **缺失** |
| Network Isolation | 無 | **缺失** |
| Database Encryption | 無 | **缺失** |

## 附錄 B: agents.toml 安全配置範本

```toml
[core]
host = "127.0.0.1"
port = 7878
api_tokens = ["enc2:xxx"]  # HTTP API 認證

[telegram]
bot_token = "enc2:xxx"
allowed_users = ["123456789"]  # 只允許指定用戶

[roles]
owner = "123456789"
admins = []
operators = []

[security]
workspace_dir = "~/.clawtex/workspace"
workspace_only = true
allowed_commands = ["ls", "cat", "echo", "git", "python", "node", "npm", "cargo", "rustc", "find", "grep", "wc", "sort", "head", "tail"]
scrub_credentials = true
allowed_http_domains = ["api.twitter.com", "api.stripe.com", "api.render.com"]
sensitive_paths_blacklist = [".secret_key", "agents.toml", ".ssh", ".env"]

[email]
smtp_host = "smtp.gmail.com"
smtp_port = 587
smtp_username = "enc2:xxx"
smtp_password = "enc2:xxx"
max_per_hour = 20
max_per_day = 100
require_approval = true

[twitter]
consumer_key = "enc2:xxx"
consumer_secret = "enc2:xxx"
access_token = "enc2:xxx"
access_token_secret = "enc2:xxx"

[stripe]
secret_key = "enc2:xxx"

[render]
api_key = "enc2:xxx"

[audit]
enabled = true
db_path = "~/.clawtex/audit.db"
retention_days = 365
```
