# ZeroClaw 深度技術分析 v2

> **分析版本**: ZeroClaw v0.1.9
> **分析日期**: 2026-03-13 (深度改版)
> **目的**: 作為 clawtex-core 的參考架構，提供可直接落地的差距分析與 Rust 程式碼建議
> **行數**: ~3000 行深度分析 (擴充版)

---

## 目錄

1. [專案結構與規模](#1-專案結構與規模)
2. [12 模組安全子系統](#2-12-模組安全子系統)
3. [SOP 引擎 — 狀態機與條件系統](#3-sop-引擎--狀態機與條件系統)
4. [35+ 工具系統 — Tool Trait 與安全模型](#4-35-工具系統--tool-trait-與安全模型)
5. [Channel Trait — 20+ 通道抽象層](#5-channel-trait--20-通道抽象層)
6. [SkillForge — 技能自動發現/評估/整合](#6-skillforge--技能自動發現評估整合)
7. [Gateway — axum 路由、SSE/WS 雙通道](#7-gateway--axum-路由ssews-雙通道)
8. [Agent Runtime — 建構者模式與訊息迴圈](#8-agent-runtime--建構者模式與訊息迴圈)
9. [Provider 系統 — 可靠性包裝與路由](#9-provider-系統--可靠性包裝與路由)
10. [記憶系統 — 7 種後端與向量嵌入](#10-記憶系統--7-種後端與向量嵌入)
11. [差距對比總覽與實作優先序](#11-差距對比總覽與實作優先序)
12. [附錄：關鍵檔案路徑索引](#12-附錄關鍵檔案路徑索引)

---

## 1. 專案結構與規模

### 1.1 目錄樹

```
zeroclaw/
├── Cargo.toml              # 工作空間根，含主 crate + robot-kit
├── build.rs                # 編譯時程式碼生成
├── src/
│   ├── main.rs             # CLI 入口 (clap)
│   ├── lib.rs              # 模組公開匯出 + 子命令列舉
│   ├── agent/              # 代理協調核心 (8 檔案)
│   ├── approval/           # 人機迴圈批准閘道
│   ├── auth/               # OAuth 認證 (5 檔案)
│   ├── channels/           # 26 通訊頻道 (26 檔案)
│   ├── config/             # 設定系統 (3 檔案)
│   ├── cost/               # 成本追蹤 (3 檔案)
│   ├── cron/               # 排程器 (3 檔案)
│   ├── daemon/             # 守護程序
│   ├── doctor/             # 診斷命令
│   ├── gateway/            # HTTP/WS 閘道 (5 檔案)
│   ├── hardware/           # USB 硬體發現
│   ├── health/             # 元件健康追蹤
│   ├── heartbeat/          # 心跳引擎
│   ├── hooks/              # 事件鉤子系統
│   ├── identity.rs         # AIEOS v1.1 身份系統
│   ├── integrations/       # 50+ 整合目錄
│   ├── memory/             # 記憶後端 (16 檔案)
│   ├── observability/      # 可觀測性 (6 檔案)
│   ├── peripherals/        # 硬體外設 (3 檔案)
│   ├── providers/          # LLM 提供者 (16 檔案)
│   ├── rag/                # 硬體資料表 RAG
│   ├── runtime/            # 執行環境抽象
│   ├── security/           # 安全子系統 (16 檔案)
│   ├── service/            # OS 服務管理
│   ├── skillforge/         # 技能自動發現 (4 檔案)
│   ├── skills/             # 使用者自訂技能
│   ├── sop/                # 標準操作程序引擎 (8 檔案)
│   ├── tools/              # 35+ 工具 (42 檔案)
│   └── tunnel/             # 隧道 (5 檔案)
├── crates/robot-kit/       # 獨立機器人套件 crate
├── firmware/               # ESP32, Nucleo, Arduino 韌體
├── fuzz/                   # 5 個模糊測試目標
├── tests/                  # 4 層測試
├── benches/                # Criterion 效能基準
├── web/                    # 內嵌式前端儀表板
└── python/                 # Python 繫結
```

### 1.2 資料流總覽

```
┌──────────────────────────────────────────────────────────────────┐
│                         External Events                         │
│  Telegram  Discord  Slack  WhatsApp  MQTT  Webhook  Cron  CLI   │
└──────┬───────┬───────┬───────┬───────┬───────┬───────┬──────┬───┘
       │       │       │       │       │       │       │      │
       ▼       ▼       ▼       ▼       ▼       ▼       ▼      ▼
  ┌────────────────────────────────────────────────────────────────┐
  │                    Channel Trait Layer                         │
  │  listen() → mpsc::Sender<ChannelMessage>                      │
  │  send() / send_draft() / update_draft() / finalize_draft()    │
  └────────────────────────┬──────────────────────────────────────-┘
                           │ ChannelMessage
                           ▼
  ┌────────────────────────────────────────────────────────────────┐
  │                    Security Pipeline                           │
  │  PromptGuard.scan() → LeakDetector.scan() → E-Stop check      │
  │  PairingGuard auth → SecurityPolicy (autonomy level)          │
  └────────────────────────┬──────────────────────────────────────-┘
                           │
                           ▼
  ┌────────────────────────────────────────────────────────────────┐
  │              Agent Runtime (agent/loop_.rs)                    │
  │  ┌───────────┐   ┌──────────────┐   ┌───────────────────┐    │
  │  │ Classifier │──▶│ RouterProvider│──▶│ ToolDispatcher     │    │
  │  │ (hint→model│  │ (hint→model) │  │ (XML/Native)       │    │
  │  └───────────┘   └──────────────┘   └────────┬──────────┘    │
  │                                               │               │
  │  ┌────────────────────────────────────────────┤               │
  │  │ Tool Execution Loop (max 10 iterations)    │               │
  │  │  SecurityPolicy.check_tool()               │               │
  │  │  tool.execute(args) → ToolResult           │               │
  │  │  scrub_credentials(output)                 │               │
  │  │  AuditLogger.log()                         │               │
  │  └────────────────────────────────────────────┘               │
  │                                                               │
  │  Memory: store/recall → trim_history → auto_compact           │
  └────────────────────────┬──────────────────────────────────────-┘
                           │
                           ▼
  ┌────────────────────────────────────────────────────────────────┐
  │              Provider Layer (15 providers)                     │
  │  ReliableProvider(CircuitBreaker + Retry + Fallback)          │
  │  → Anthropic / OpenAI / Gemini / Ollama / ...                 │
  └────────────────────────────────────────────────────────────────┘
```

---

## 2. 12 模組安全子系統

ZeroClaw 的安全子系統是其最深入的部分。16 個 `.rs` 檔案組成 12 個功能模組。

### 2.1 模組清單與職責

```
src/security/
├── mod.rs             # 公開匯出 + redact() 工具函式
├── policy.rs          # SecurityPolicy + AutonomyLevel + ActionTracker + CommandRiskLevel
├── secrets.rs         # SecretStore (ChaCha20-Poly1305 AEAD)
├── estop.rs           # EstopManager (4 級緊急停止 + JSON 持久化)
├── leak_detector.rs   # LeakDetector (7 類別憑證洩漏偵測 + Shannon 熵)
├── prompt_guard.rs    # PromptGuard (6 類別提示注入防禦)
├── pairing.rs         # PairingGuard (裝置配對認證)
├── audit.rs           # AuditLogger (結構化審計日誌 + 輪替)
├── domain_matcher.rs  # DomainMatcher (萬用字元網域匹配 + 預設分類)
├── otp.rs             # OtpValidator (TOTP 驗證 + HMAC-SHA1)
├── traits.rs          # Sandbox trait (OS 級程序隔離抽象)
├── detect.rs          # create_sandbox() (自動偵測最佳後端)
├── docker.rs          # DockerSandbox
├── firejail.rs        # FirejailSandbox (Linux only)
├── bubblewrap.rs      # BubblewrapSandbox (feature flag)
└── landlock.rs        # LandlockSandbox (Linux LSM, feature flag)
```

### 2.2 SecurityPolicy — 完整 struct 與 shell 解析器

**檔案**: `src/security/policy.rs` (800+ 行)

```rust
// src/security/policy.rs:8-18
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AutonomyLevel {
    ReadOnly,
    #[default]
    Supervised,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRiskLevel { Low, Medium, High }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOperation { Read, Act }
```

SecurityPolicy 的完整 struct:

```rust
// src/security/policy.rs:82-95
pub struct SecurityPolicy {
    pub autonomy: AutonomyLevel,
    pub workspace_dir: PathBuf,
    pub workspace_only: bool,
    pub allowed_commands: Vec<String>,      // 預設 12 個安全命令
    pub forbidden_paths: Vec<String>,       // 22 個系統/敏感目錄
    pub allowed_roots: Vec<PathBuf>,
    pub max_actions_per_hour: u32,          // 預設 20
    pub max_cost_per_day_cents: u32,        // 預設 500
    pub require_approval_for_medium_risk: bool,
    pub block_high_risk_commands: bool,
    pub shell_env_passthrough: Vec<String>,
    pub tracker: ActionTracker,             // 滑動窗口速率限制
}
```

**關鍵設計: Quote-Aware Shell 解析器**

policy.rs 包含一個完整的 shell 命令解析器，能正確處理引號內的分號:

```rust
// src/security/policy.rs:216-300
fn split_unquoted_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote = QuoteState::None;    // None/Single/Double
    let mut escaped = false;
    let mut chars = command.chars().peekable();
    // ... 正確處理 ';', '|', '&&', '||' 作為命令分隔符
    // 但在 '' 或 "" 引號內視為普通字元
}
```

這代表 `sqlite3 db "SELECT 1; SELECT 2;"` 不會被誤判為兩個命令。

**背景單 `&` 偵測 — `contains_unquoted_single_ampersand()`**:

```rust
// src/security/policy.rs:309-358
fn contains_unquoted_single_ampersand(command: &str) -> bool {
    let mut quote = QuoteState::None;
    let mut escaped = false;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        match quote {
            QuoteState::Single => { if ch == '\'' { quote = QuoteState::None; } }
            QuoteState::Double => {
                if escaped { escaped = false; continue; }
                if ch == '\\' { escaped = true; continue; }
                if ch == '"' { quote = QuoteState::None; }
            }
            QuoteState::None => {
                if escaped { escaped = false; continue; }
                if ch == '\\' { escaped = true; continue; }
                match ch {
                    '\'' => quote = QuoteState::Single,
                    '"' => quote = QuoteState::Double,
                    '&' => {
                        if chars.next_if_eq(&'&').is_none() {
                            return true;  // 單獨 & — 危險！
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    false
}
```

這函式確保 `&&` (邏輯 AND) 允許通過，但單獨 `&` (背景執行) 被封鎖，因為背景命令可以隱藏子命令並逃脫超時限制。

**Shell 變數展開偵測 — `contains_unquoted_shell_variable_expansion()`**:

```rust
// src/security/policy.rs:411-478
fn contains_unquoted_shell_variable_expansion(command: &str) -> bool {
    // 掃描未引號的 $HOME, $1, $?, ${...} 等
    // 在單引號內的 $ 視為字面量 (符合 POSIX shell 語義)
    // 在雙引號內的 $ 仍然展開 (符合 POSIX shell 語義)
    let chars: Vec<char> = command.chars().collect();
    for i in 0..chars.len() {
        if chars[i] != '$' { continue; }
        let Some(next) = chars.get(i + 1).copied() else { continue; };
        if next.is_ascii_alphanumeric()
            || matches!(next, '_' | '{' | '(' | '#' | '?' | '!' | '$' | '*' | '@' | '-')
        {
            return true;
        }
    }
    false
}
```

**5 層命令驗證閘道 — `is_command_allowed()`**:

```
is_command_allowed(command) → bool
  │
  ├── Gate 1: ReadOnly → 直接拒絕
  │
  ├── Gate 2: 子 shell/展開運算子
  │     ├── 反引號 ` → 封鎖 (隱藏任意命令)
  │     ├── $(...) / ${...} → 封鎖 (變數注入)
  │     └── <(...) / >(...) → 封鎖 (程序替換)
  │
  ├── Gate 3: Shell 重定向
  │     ├── > / >> → 封鎖 (繞過路徑策略)
  │     └── < → 封鎖 (讀取任意檔案)
  │
  ├── Gate 4: tee 命令偵測 → 封鎖 (繞過重定向檢查)
  │
  ├── Gate 5: 背景 & → 封鎖 (隱藏子命令 + 逃脫超時)
  │
  └── Gate 6: 逐段允許清單檢查
        ├── split_unquoted_segments() 分割命令
        ├── skip_env_assignments() 跳過環境變數
        ├── is_allowlist_entry_match() 比對允許清單
        └── is_args_safe() 檢查危險參數
              ├── find -exec / -ok → 封鎖
              └── git config / alias / -c → 封鎖
```

**路徑邊界驗證 — `forbidden_path_argument()`**:

```rust
// src/security/policy.rs:826-900
pub fn forbidden_path_argument(&self, command: &str) -> Option<String> {
    let forbidden_candidate = |raw: &str| {
        let candidate = strip_wrapping_quotes(raw).trim();
        if candidate.is_empty() || candidate.contains("://") { return None; }
        if looks_like_path(candidate) && !self.is_path_allowed(candidate) {
            Some(candidate.to_string())
        } else { None }
    };
    // 掃描每個命令段的每個參數
    for segment in split_unquoted_segments(command) {
        let cmd_part = skip_env_assignments(&segment);
        for word in cmd_part.split_whitespace().skip(1) { // 跳過命令本身
            // 檢查 inline 重定向: cat</etc/passwd
            if let Some(target) = redirection_target(strip_wrapping_quotes(word)) {
                if let Some(blocked) = forbidden_candidate(target) {
                    return Some(blocked);
                }
            }
            // 檢查短選項附帶路徑: -f/etc/passwd
            if let Some(value) = attached_short_option_value(word) {
                if let Some(blocked) = forbidden_candidate(value) {
                    return Some(blocked);
                }
            }
            // 普通參數路徑檢查
            if let Some(blocked) = forbidden_candidate(word) {
                return Some(blocked);
            }
        }
    }
    None
}
```

此函式涵蓋了 3 種路徑注入方式: inline 重定向 (`cat</etc/passwd`)、短選項附帶值 (`-f/etc/passwd`)、普通參數 (`cat /etc/passwd`)。

**完整風險評估流程 — `validate_command_execution()`**:

```
validate_command_execution(command, approved) → Result<CommandRiskLevel>
  │
  ├── is_command_allowed()? → "Command not allowed"
  │
  ├── command_risk_level() 分級
  │     ├── High: rm, sudo, curl, ssh, dd 等 26 個高危命令
  │     │         + rm -rf /, 叉子炸彈模式
  │     ├── Medium: git commit/push/reset, npm install,
  │     │           cargo add/clean, touch/mkdir/mv/cp/ln
  │     └── Low: 其他所有允許的命令
  │
  ├── High risk + block_high_risk_commands → 封鎖
  ├── High risk + Supervised + !approved → 需批准
  ├── Medium risk + Supervised + require_approval + !approved → 需批准
  └── Ok(risk_level) → 通過

預設 SecurityPolicy:
  - 12 個允許命令: git, npm, cargo, ls, cat, grep, find, echo, pwd, wc, head, tail, date
  - 22 個禁止路徑: /etc, /root, /home, /usr, /bin, ..., ~/.ssh, ~/.gnupg, ~/.aws, ~/.config
  - max_actions_per_hour: 20
  - max_cost_per_day_cents: 500
  - block_high_risk_commands: true
  - require_approval_for_medium_risk: true
```

**ActionTracker 滑動窗口**:

```rust
// src/security/policy.rs:36-68
pub struct ActionTracker {
    actions: Mutex<Vec<Instant>>,    // parking_lot::Mutex, 非 std
}

impl ActionTracker {
    pub fn record(&self) -> usize {
        let mut actions = self.actions.lock();
        let cutoff = Instant::now()
            .checked_sub(Duration::from_secs(3600))  // 1 小時窗口
            .unwrap_or_else(Instant::now);             // 防止 Windows 溢出
        actions.retain(|t| *t > cutoff);
        actions.push(Instant::now());
        actions.len()
    }
}
```

### 2.3 LeakDetector — 7 類別 + Shannon 熵

**檔案**: `src/security/leak_detector.rs` (538 行)

```rust
// src/security/leak_detector.rs:31-35
pub struct LeakDetector {
    sensitivity: f64,  // 0.0-1.0, 預設 0.7
}
```

7 類別偵測管線:

```
scan(content)
  ├── check_api_keys()          → Stripe, OpenAI, Anthropic, Google, GitHub (9 正則)
  ├── check_aws_credentials()   → AKIA keys, secret access keys (2 正則)
  ├── check_generic_secrets()   → password=, secret=, token= (3 正則, sensitivity > 0.5)
  ├── check_private_keys()      → RSA/EC/OPENSSH PEM (4 pattern pairs)
  ├── check_jwt_tokens()        → eyJ*.eyJ*.* (1 正則)
  ├── check_database_urls()     → postgres/mysql/mongodb/redis (4 正則)
  └── check_high_entropy_tokens()
       ├── URL 剝離 (避免路徑段誤報)
       ├── extract_candidate_tokens() — 分割 alphanumeric+_-+/
       ├── 長度 >= 24 字元
       ├── Shannon 熵 >= 3.5 + sensitivity * 1.25
       └── has_mixed_alpha_digit() — 必須同時含字母和數字
```

Shannon 熵計算:

```rust
// src/security/leak_detector.rs:342-355
fn shannon_entropy(s: &str) -> f64 {
    let len = s.len() as f64;
    if len == 0.0 { return 0.0; }
    let mut freq: HashMap<u8, usize> = HashMap::new();
    for &b in s.as_bytes() {
        *freq.entry(b).or_insert(0) += 1;
    }
    freq.values().fold(0.0, |acc, &count| {
        let p = count as f64 / len;
        acc - p * p.log2()
    })
}
```

**效能特徵**: 所有正則都用 `OnceLock<Vec<(Regex, &str)>>` 延遲編譯，整個生命週期只編譯一次。

### 2.4 PromptGuard — 6 類別提示注入防禦

**檔案**: `src/security/prompt_guard.rs` (361 行)

```rust
// src/security/prompt_guard.rs:19-26
pub enum GuardResult {
    Safe,
    Suspicious(Vec<String>, f64),  // (偵測模式列表, 正規化分數 0.0-1.0)
    Blocked(String),               // 封鎖原因
}

pub enum GuardAction {
    Warn,       // 記錄但放行
    Block,      // 封鎖
    Sanitize,   // 清洗危險模式
}
```

6 類別偵測 + 分數系統:

```
scan(content) → 6 類別各回傳 0.0-1.0 分數
  ├── check_system_override()     → 1.0 分 (ignore/disregard/forget/override/reset instructions)
  ├── check_role_confusion()      → 0.9 分 (you are now/act as/pretend to be)
  ├── check_tool_injection()      → 0.8 分 (tool_calls JSON / JSON escape attempts)
  ├── check_secret_extraction()   → 0.95 分 (show/list/dump secrets/keys/credentials)
  ├── check_command_injection()   → 0.6 分 (backtick, $(), &&, ||, ;, |)
  │    └── 排除: | head/tail/grep, 短命令 &&
  └── check_jailbreak_attempts()  → 0.85 分 (DAN mode, developer mode, hypothetical)

正規化分數 = total / 6.0, 限制 <= 1.0
決策:
  - GuardAction::Block && max_score > sensitivity → Blocked
  - 否則 → Suspicious(patterns, score)
```

### 2.5 EstopManager — 4 級緊急停止 + OTP 恢復

**檔案**: `src/security/estop.rs` (423 行)

```rust
// src/security/estop.rs:10-16
pub enum EstopLevel {
    KillAll,                    // 停止所有操作
    NetworkKill,                // 僅切斷網路
    DomainBlock(Vec<String>),   // 封鎖特定網域 (DomainMatcher 驗證)
    ToolFreeze(Vec<String>),    // 凍結特定工具 (名稱正規化驗證)
}

// src/security/estop.rs:26-38
pub struct EstopState {
    pub kill_all: bool,
    pub network_kill: bool,
    pub blocked_domains: Vec<String>,
    pub frozen_tools: Vec<String>,
    pub updated_at: Option<String>,  // RFC-3339 時間戳
}
```

**錯誤處理策略 — Fail-Closed**:

```rust
// src/security/estop.rs:83-88
// 無法讀取狀態檔案 → fail-closed (KillAll)
Err(error) => {
    tracing::warn!("Failed to read estop state file; entering fail-closed mode: {error}");
    should_fail_closed = true;
    EstopState::fail_closed()
}
// 無法解析 JSON → fail-closed (KillAll)
Err(error) => {
    tracing::warn!("Failed to parse estop state file; entering fail-closed mode: {error}");
    should_fail_closed = true;
    EstopState::fail_closed()
}
```

**持久化: 原子寫入**:

```rust
// src/security/estop.rs:217-250
fn persist_state(&mut self) -> Result<()> {
    let temp_path = self.state_path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    fs::write(&temp_path, body)?;
    #[cfg(unix)]
    { let _ = fs::set_permissions(&temp_path, Permissions::from_mode(0o600)); }
    fs::rename(&temp_path, &self.state_path)?;  // 原子替換
}
```

**恢復流程 — OTP 驗證**:

```rust
// src/security/estop.rs:155-215
pub fn resume(&mut self, selector: ResumeSelector, otp_code: Option<&str>,
              otp_validator: Option<&OtpValidator>) -> Result<()> {
    self.ensure_resume_is_authorized(otp_code, otp_validator)?;
    // ... 根據 selector 解除對應限制
}
```

### 2.6 OtpValidator — TOTP 實作

**檔案**: `src/security/otp.rs` (319 行)

```rust
// src/security/otp.rs:164-178
fn compute_totp_code(secret: &[u8], counter: u64) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret);
    let counter_bytes = counter.to_be_bytes();
    let digest = hmac::sign(&key, &counter_bytes);
    let hash = digest.as_ref();
    let offset = (hash[19] & 0x0f) as usize;
    let binary = ((u32::from(hash[offset]) & 0x7f) << 24)
        | (u32::from(hash[offset + 1]) << 16)
        | (u32::from(hash[offset + 2]) << 8)
        | u32::from(hash[offset + 3]);
    let code = binary % 10_u32.pow(6);  // 6 位數
    format!("{code:0>6}")
}
```

驗證策略: 允許 counter-1, counter, counter+1 三個窗口 + 本地快取避免重複驗證。

### 2.7 Sandbox Trait — 4 種沙箱後端

**檔案**: `src/security/traits.rs` (119 行)

```rust
// src/security/traits.rs:22-52
#[async_trait]
pub trait Sandbox: Send + Sync {
    fn wrap_command(&self, cmd: &mut Command) -> std::io::Result<()>;
    fn is_available(&self) -> bool;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
}
```

偵測優先序: `Landlock > Bubblewrap > Firejail > Docker > Noop`

### 2.8 AuditLogger — 結構化審計日誌

**檔案**: `src/security/audit.rs` (424 行)

```rust
// src/security/audit.rs:14-24
pub enum AuditEventType {
    CommandExecution,
    FileAccess,
    ConfigChange,
    AuthSuccess,
    AuthFailure,
    PolicyViolation,
    SecurityEvent,
}

// src/security/audit.rs:62-70
pub struct AuditEvent {
    pub timestamp: DateTime<Utc>,
    pub event_id: String,           // UUID v4
    pub event_type: AuditEventType,
    pub actor: Option<Actor>,       // channel + user_id + username
    pub action: Option<Action>,     // command + risk_level + approved + allowed
    pub result: Option<ExecutionResult>,  // success + exit_code + duration_ms + error
    pub security: SecurityContext,  // policy_violation + rate_limit + sandbox_backend
}
```

日誌輪替: 基於檔案大小 (MB 門檻)，最多 10 個備份 (.1.log ~ .10.log)。

### 2.9 DomainMatcher — 萬用字元 + 預設分類

**檔案**: `src/security/domain_matcher.rs` (260 行)

預設封鎖分類:
- `banking`: Chase, BofA, Wells Fargo, Fidelity, Schwab, Venmo, PayPal, Robinhood, Coinbase
- `medical`: MyChart, Epic, patient portal, health records
- `government`: SSA, IRS, Login.gov, ID.me
- `identity_providers`: Google, Microsoft, Apple

```rust
// src/security/domain_matcher.rs:164-208 — 高效萬用字元匹配
fn wildcard_match(pattern: &[u8], value: &[u8]) -> bool {
    // 使用 star_idx + match_idx 回溯法，O(n*m) 最壞但典型 O(n)
}
```

### 2.10 SecretStore — ChaCha20-Poly1305 + 遺留遷移

**檔案**: `src/security/secrets.rs` (852 行)

```rust
// src/security/secrets.rs:56-76
pub fn encrypt(&self, plaintext: &str) -> Result<String> {
    let key_bytes = self.load_or_create_key()?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key_bytes));
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);  // 隨機 12 bytes
    let ciphertext = cipher.encrypt(&nonce, plaintext.as_bytes())?;
    let mut blob = Vec::with_capacity(12 + ciphertext.len());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);
    Ok(format!("enc2:{}", hex_encode(&blob)))
}
```

遷移 API: `decrypt_and_migrate()` — 自動偵測 `enc:` (XOR) → 升級為 `enc2:` (ChaCha20-Poly1305)。

Windows 支援: `icacls` 限制金鑰檔案權限為當前使用者。

**Clawtex 實作建議**

clawtex-core 目前有 `src/encryption.rs` (ChaCha20-Poly1305，`enc2:` 前綴)，與 ZeroClaw 設計幾乎相同。需要補充的:

| 功能 | clawtex 現況 | 改動建議 | 複雜度 |
|------|-------------|---------|--------|
| PromptGuard | **無** | 新增 `src/security/prompt_guard.rs` | ~300 行，依賴 `regex` (已有) |
| LeakDetector | 僅 `scrub_credentials()` | 新增 `src/security/leak_detector.rs` | ~350 行，`OnceLock` 正則 |
| 多級 E-Stop | 單一 `AtomicBool` | 重寫 `src/estop.rs` → `EstopState` struct | ~250 行 |
| AuditLogger | **無** | 新增 `src/security/audit.rs` | ~200 行 |
| DomainMatcher | **無** | 新增 `src/security/domain_matcher.rs` | ~150 行 |
| OTP 驗證 | **無** | 新增 `src/security/otp.rs`，依賴 `ring` | ~200 行，新 crate: `ring` |
| Sandbox trait | **無** | 新增 `src/security/sandbox.rs` | ~100 行 trait + NoopSandbox |
| Shell 解析器 | 基本 split | 移植 `split_unquoted_segments()` | ~80 行 |

Rust skeleton 建議 — PromptGuard (完整):

```rust
// src/security/prompt_guard.rs (clawtex-core 建議, ~300 行)
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug)]
pub enum GuardResult {
    Safe,
    Suspicious(Vec<String>, f64),  // (patterns, normalized_score)
    Blocked(String),
}

#[derive(Debug, Clone, Copy)]
pub enum GuardAction { Warn, Block, Sanitize }

pub struct PromptGuard {
    action: GuardAction,
    sensitivity: f64,  // 0.0-1.0, 預設 0.7
}

impl PromptGuard {
    pub fn new() -> Self { Self { action: GuardAction::Warn, sensitivity: 0.7 } }

    pub fn with_action(mut self, action: GuardAction) -> Self { self.action = action; self }
    pub fn with_sensitivity(mut self, s: f64) -> Self { self.sensitivity = s.clamp(0.0, 1.0); self }

    pub fn scan(&self, content: &str) -> GuardResult {
        let content_lower = content.to_lowercase();
        let mut patterns = Vec::new();
        let mut max_score: f64 = 0.0;

        // 6 個偵測函式，每個回傳 0.0-1.0
        let checks: [(fn(&str, &mut Vec<String>) -> f64, &str); 6] = [
            (check_system_override, "system_override"),
            (check_role_confusion, "role_confusion"),
            (check_tool_injection, "tool_injection"),
            (check_secret_extraction, "secret_extraction"),
            (check_command_injection, "command_injection"),
            (check_jailbreak, "jailbreak"),
        ];

        for (check_fn, _label) in &checks {
            let score = check_fn(&content_lower, &mut patterns);
            max_score = max_score.max(score);
        }

        if patterns.is_empty() { return GuardResult::Safe; }

        match self.action {
            GuardAction::Block if max_score > self.sensitivity =>
                GuardResult::Blocked(format!("Injection detected: {}", patterns.join(", "))),
            _ => GuardResult::Suspicious(patterns, max_score),
        }
    }
}

fn system_override_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| vec![
        Regex::new(r"(?i)\b(ignore|disregard|forget|override|reset)\b.{0,20}\b(instructions?|rules?|system|prompt)\b").unwrap(),
        Regex::new(r"(?i)\bnew\s+(instructions?|rules?|system\s*prompt)\b").unwrap(),
    ])
}

fn check_system_override(content: &str, patterns: &mut Vec<String>) -> f64 {
    for re in system_override_patterns() {
        if re.is_match(content) {
            patterns.push("system_override".into());
            return 1.0;
        }
    }
    0.0
}

fn check_role_confusion(content: &str, patterns: &mut Vec<String>) -> f64 {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    let pats = PATTERNS.get_or_init(|| vec![
        Regex::new(r"(?i)\b(you\s+are\s+now|act\s+as|pretend\s+to\s+be|roleplay\s+as)\b").unwrap(),
    ]);
    for re in pats { if re.is_match(content) { patterns.push("role_confusion".into()); return 0.9; } }
    0.0
}

fn check_tool_injection(content: &str, patterns: &mut Vec<String>) -> f64 {
    if content.contains("tool_calls") && content.contains("\"name\"") {
        patterns.push("tool_injection".into());
        return 0.8;
    }
    0.0
}

fn check_secret_extraction(content: &str, patterns: &mut Vec<String>) -> f64 {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    let pats = PATTERNS.get_or_init(|| vec![
        Regex::new(r"(?i)\b(show|list|dump|reveal|tell\s+me)\b.{0,20}\b(secrets?|keys?|credentials?|passwords?|tokens?)\b").unwrap(),
    ]);
    for re in pats { if re.is_match(content) { patterns.push("secret_extraction".into()); return 0.95; } }
    0.0
}

fn check_command_injection(content: &str, patterns: &mut Vec<String>) -> f64 {
    // 排除 | head/tail/grep (常見安全用法)
    if content.contains('`') || content.contains("$(") {
        patterns.push("command_injection".into());
        return 0.6;
    }
    0.0
}

fn check_jailbreak(content: &str, patterns: &mut Vec<String>) -> f64 {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    let pats = PATTERNS.get_or_init(|| vec![
        Regex::new(r"(?i)\b(DAN|developer)\s+mode\b").unwrap(),
        Regex::new(r"(?i)\bhypothetical(ly)?\b.{0,30}\b(scenario|situation)\b").unwrap(),
    ]);
    for re in pats { if re.is_match(content) { patterns.push("jailbreak".into()); return 0.85; } }
    0.0
}
```

Rust skeleton 建議 — LeakDetector (完整):

```rust
// src/security/leak_detector.rs (clawtex-core 建議, ~350 行)
use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

pub struct LeakDetector { sensitivity: f64 }

#[derive(Debug)]
pub enum LeakResult {
    Clean,
    Detected { findings: Vec<LeakFinding>, redacted: String },
}

#[derive(Debug)]
pub struct LeakFinding { pub category: String, pub pattern: String, pub confidence: f64 }

impl LeakDetector {
    pub fn new(sensitivity: f64) -> Self { Self { sensitivity: sensitivity.clamp(0.0, 1.0) } }

    pub fn scan(&self, content: &str) -> LeakResult {
        let mut findings = Vec::new();
        self.check_api_keys(content, &mut findings);
        self.check_aws_credentials(content, &mut findings);
        self.check_private_keys(content, &mut findings);
        self.check_jwt_tokens(content, &mut findings);
        self.check_database_urls(content, &mut findings);
        if self.sensitivity > 0.5 {
            self.check_generic_secrets(content, &mut findings);
        }
        self.check_high_entropy_tokens(content, &mut findings);

        if findings.is_empty() { return LeakResult::Clean; }

        let mut redacted = content.to_string();
        for finding in &findings {
            redacted = redacted.replace(&finding.pattern, "[REDACTED]");
        }
        LeakResult::Detected { findings, redacted }
    }

    fn check_api_keys(&self, content: &str, findings: &mut Vec<LeakFinding>) {
        static PATTERNS: OnceLock<Vec<(Regex, &str)>> = OnceLock::new();
        let pats = PATTERNS.get_or_init(|| vec![
            (Regex::new(r"sk-[a-zA-Z0-9]{20,}").unwrap(), "openai"),
            (Regex::new(r"sk-ant-[a-zA-Z0-9]{20,}").unwrap(), "anthropic"),
            (Regex::new(r"sk_live_[a-zA-Z0-9]{20,}").unwrap(), "stripe_live"),
            (Regex::new(r"ghp_[a-zA-Z0-9]{36,}").unwrap(), "github_pat"),
            (Regex::new(r"AIza[a-zA-Z0-9_-]{35}").unwrap(), "google"),
        ]);
        for (re, category) in pats {
            for m in re.find_iter(content) {
                findings.push(LeakFinding {
                    category: category.to_string(),
                    pattern: m.as_str().to_string(),
                    confidence: 0.95,
                });
            }
        }
    }

    fn check_high_entropy_tokens(&self, content: &str, findings: &mut Vec<LeakFinding>) {
        let threshold = 3.5 + self.sensitivity * 1.25;
        for token in content.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
            if token.len() >= 24
                && shannon_entropy(token) >= threshold
                && has_mixed_alpha_digit(token) {
                findings.push(LeakFinding {
                    category: "high_entropy".into(),
                    pattern: token.to_string(),
                    confidence: 0.7,
                });
            }
        }
    }
    // ... 其他 check_* 方法省略
}

fn shannon_entropy(s: &str) -> f64 {
    let len = s.len() as f64;
    if len == 0.0 { return 0.0; }
    let mut freq: HashMap<u8, usize> = HashMap::new();
    for &b in s.as_bytes() { *freq.entry(b).or_insert(0) += 1; }
    freq.values().fold(0.0, |acc, &count| {
        let p = count as f64 / len;
        acc - p * p.log2()
    })
}

fn has_mixed_alpha_digit(s: &str) -> bool {
    s.bytes().any(|b| b.is_ascii_alphabetic()) && s.bytes().any(|b| b.is_ascii_digit())
}
```

Rust skeleton 建議 — 多級 E-Stop:

```rust
// src/security/estop.rs (clawtex-core 建議, ~250 行)
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum EstopLevel {
    KillAll,                    // 全面停止
    NetworkKill,                // 切斷網路
    DomainBlock(Vec<String>),   // 封鎖特定網域
    ToolFreeze(Vec<String>),    // 凍結特定工具
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstopState {
    pub kill_all: bool,
    pub network_kill: bool,
    pub blocked_domains: Vec<String>,
    pub frozen_tools: Vec<String>,
    pub updated_at: Option<String>,
}

impl EstopState {
    pub fn fail_closed() -> Self {
        Self { kill_all: true, network_kill: true,
               blocked_domains: vec![], frozen_tools: vec![],
               updated_at: Some(chrono::Utc::now().to_rfc3339()) }
    }

    pub fn is_engaged(&self) -> bool {
        self.kill_all || self.network_kill
            || !self.blocked_domains.is_empty()
            || !self.frozen_tools.is_empty()
    }

    pub fn is_tool_frozen(&self, tool_name: &str) -> bool {
        self.kill_all || self.frozen_tools.iter().any(|t| t == tool_name)
    }
}

pub struct EstopManager {
    state: EstopState,
    state_path: PathBuf,
}

impl EstopManager {
    pub fn load_or_fail_closed(path: PathBuf) -> Self {
        let state = match std::fs::read_to_string(&path) {
            Ok(json) => serde_json::from_str(&json).unwrap_or_else(|e| {
                tracing::warn!("Failed to parse estop state; fail-closed: {e}");
                EstopState::fail_closed()  // 解析失敗 → fail-closed
            }),
            Err(_) => EstopState::default(),  // 檔案不存在 → 正常狀態
        };
        Self { state, state_path: path }
    }

    pub fn engage(&mut self, level: EstopLevel) -> anyhow::Result<()> {
        match level {
            EstopLevel::KillAll => self.state.kill_all = true,
            EstopLevel::NetworkKill => self.state.network_kill = true,
            EstopLevel::DomainBlock(domains) => self.state.blocked_domains.extend(domains),
            EstopLevel::ToolFreeze(tools) => self.state.frozen_tools.extend(tools),
        }
        self.state.updated_at = Some(chrono::Utc::now().to_rfc3339());
        self.persist_atomic()
    }

    // 原子寫入: temp file → rename
    fn persist_atomic(&self) -> anyhow::Result<()> {
        let temp = self.state_path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
        std::fs::write(&temp, serde_json::to_string_pretty(&self.state)?)?;
        std::fs::rename(&temp, &self.state_path)?;
        Ok(())
    }
}
```

---

## 3. SOP 引擎 -- 狀態機與條件系統

### 3.1 SOP 類型系統

**檔案**: `src/sop/types.rs` (471 行)

```rust
// src/sop/types.rs:9-17
pub enum SopPriority { Low, Normal, High, Critical }

// src/sop/types.rs:33-45
pub enum SopExecutionMode {
    Auto,           // 全自動
    Supervised,     // 啟動前需批准
    StepByStep,     // 每步都需批准
    PriorityBased,  // Critical/High → Auto, Normal/Low → Supervised
}

// src/sop/types.rs:61-82
pub enum SopTrigger {
    Mqtt { topic: String, condition: Option<String> },
    Webhook { path: String },
    Cron { expression: String },
    Peripheral { board: String, signal: String, condition: Option<String> },
    Manual,
}
```

SOP 定義結構:

```rust
// src/sop/types.rs:113-128
pub struct Sop {
    pub name: String,
    pub description: String,
    pub version: String,
    pub priority: SopPriority,
    pub execution_mode: SopExecutionMode,
    pub triggers: Vec<SopTrigger>,
    pub steps: Vec<SopStep>,
    pub cooldown_secs: u64,       // 冷卻時間
    pub max_concurrent: u32,      // 最大並行數
    pub location: Option<PathBuf>,
}

pub struct SopStep {
    pub number: u32,
    pub title: String,
    pub body: String,
    pub suggested_tools: Vec<String>,
    pub requires_confirmation: bool,
}
```

### 3.2 SOP 狀態機

**檔案**: `src/sop/engine.rs` (800+ 行)

```
                ┌─────────┐
     trigger ──▶│ Pending  │
                └────┬────┘
                     │ start_run()
    ┌────────────────┼────────────────┐
    │ Auto           │ Supervised     │ StepByStep
    │                │                │
    ▼                ▼                ▼
┌──────────┐  ┌─────────────┐  ┌─────────────┐
│ Running   │  │ WaitApproval │  │ WaitApproval │
│ (step 1)  │  │ (initial)    │  │ (each step)  │
└─────┬────┘  └──────┬──────┘  └──────┬──────┘
      │               │ approve()      │ approve()
      │               ▼                ▼
      │         ┌──────────┐    ┌──────────┐
      ├────────▶│ Running   │───▶│ Running   │
      │         │ (step N)  │    │ (step N+1)│
      │         └─────┬────┘    └─────┬────┘
      │               │               │
      ▼               ▼               ▼
┌─────────────────────────────────────────┐
│           advance_step()                 │
│  success → next step / Completed         │
│  failure → Failed                        │
│  cancel  → Cancelled                     │
└─────────────────────────────────────────┘
```

SopEngine 核心方法:

```rust
// src/sop/engine.rs:17-25
pub struct SopEngine {
    sops: Vec<Sop>,
    active_runs: HashMap<String, SopRun>,
    finished_runs: Vec<SopRun>,
    config: SopConfig,
    run_counter: u64,
}

// 關鍵方法:
impl SopEngine {
    pub fn reload(&mut self, workspace_dir: &Path);           // 載入 TOML+MD
    pub fn match_trigger(&self, event: &SopEvent) -> Vec<&Sop>; // 事件匹配
    pub fn start_run(&mut self, name: &str, event: SopEvent) -> Result<SopRunAction>;
    pub fn advance_step(&mut self, run_id: &str, result: SopStepResult) -> Result<SopRunAction>;
    pub fn approve(&mut self, run_id: &str) -> Result<SopRunAction>;
    pub fn cancel(&mut self, run_id: &str) -> Result<()>;
}
```

**回傳動作枚舉**:

```rust
// src/sop/types.rs:283-304
pub enum SopRunAction {
    ExecuteStep { run_id: String, step: SopStep, context: String },
    WaitApproval { run_id: String, step: SopStep, context: String },
    Completed { run_id: String, sop_name: String },
    Failed { run_id: String, sop_name: String, reason: String },
}
```

### 3.3 條件評估系統

**檔案**: `src/sop/condition.rs` (452 行)

```rust
// src/sop/condition.rs:16-34
pub fn evaluate_condition(condition: &str, payload: Option<&str>) -> bool {
    if condition.trim().is_empty() { return true; }   // 空條件 = 無條件匹配
    if payload is None or empty { return false; }      // 無 payload = fail-closed
    if condition starts with '$' { evaluate_json_path_condition(...) }
    else { evaluate_direct_condition(...) }            // 直接數值比較
}
```

支援的 JSON 路徑語法:
- `$.value > 85` — 頂層欄位比較
- `$.data.sensor.value >= 100` — 巢狀路徑
- `$.readings.1 == 20` — 陣列索引
- `$.status == "critical"` — 字串比較

6 個運算子: `>`, `<`, `>=`, `<=`, `==`, `!=`

### 3.4 事件分派系統

**檔案**: `src/sop/dispatch.rs` (730 行)

```rust
// src/sop/dispatch.rs:66-150
pub async fn dispatch_sop_event(
    engine: &Arc<Mutex<SopEngine>>,
    audit: &SopAuditLogger,
    event: SopEvent,
) -> Vec<DispatchResult> {
    // Phase 1: Lock → match_trigger → collect SOP names → drop lock
    // Phase 2: Lock → for each: start_run → collect results → drop lock
    // Phase 3: Async (no lock) → audit each started run
}
```

**效能特徵**: 批次鎖定 — 恰好 2 次鎖定。Phase 3 的審計寫入不持鎖。

### 3.5 SOP Metrics 收集器

**檔案**: `src/sop/metrics.rs` (800+ 行)

- `SopMetricsCollector` — 實作 `ampersona_core::traits::MetricsProvider`
- 追蹤: 完成率、失敗率、步驟執行數、人工批准數、超時自動批准數
- 滑動窗口: `RunSnapshot` 環形緩衝 (最多 1000 筆)
- 整合 `ampersona` Gate 評估系統 (信任階段轉換)

### 3.6 Gates 系統 — 信任階段轉換

**檔案**: `src/sop/gates.rs` (747 行)

```rust
// src/sop/gates.rs:44-49
pub struct GateEvalState {
    inner: Mutex<GateEvalInner>,       // phase_state + last_tick
    memory: Arc<dyn Memory>,
    gates: Vec<Gate>,                   // ampersona Gate 定義
    tick_interval: Duration,
}
```

Gate 決策類型:
- `transition` — 自動相位轉換
- `observed` — 觀察但不改變
- `pending_human` — 等待人工確認

與 clawtex Hands 的關鍵差異:

| 特性 | ZeroClaw SOP | clawtex Hands |
|------|-------------|---------------|
| 定義格式 | SOP.toml + SOP.md (Markdown 步驟) | hand.toml (TOML 多階段) |
| 觸發器 | 5 種 (MQTT, Webhook, Cron, Peripheral, Manual) | 手動 + Cron |
| 執行模式 | 4 種 (Auto/Supervised/StepByStep/PriorityBased) | 固定 (按 settings) |
| 條件系統 | JSON path + 6 運算子 | **無** (僅設定值判斷) |
| 冷卻時間 | cooldown_secs | **無** |
| 並行控制 | max_concurrent | **無** |
| 信任階段 | ampersona Gates | **無** |
| 狀態持久化 | Memory trait 後端 | **無** (記憶體內) |
| 鏈式串接 | **無** | chaining_prompt |

**Clawtex 實作建議**

| 功能 | 改動位置 | 複雜度 |
|------|---------|--------|
| 條件系統 | 新增 `src/hands/condition.rs` | ~200 行 |
| 冷卻時間 | `src/hands/runner.rs` 加 cooldown 欄位 | ~30 行 |
| 並行控制 | `src/hands/runner.rs` 加 Semaphore | ~50 行 |
| 觸發器多樣化 | `src/hands/types.rs` 加 trigger enum | ~100 行 |
| 執行模式 | `hand.toml` 加 execution_mode | ~50 行 |

Rust skeleton 建議 — 條件系統:

```rust
// src/hands/condition.rs (clawtex-core 建議, ~200 行)
use serde_json::Value;

/// 評估條件表達式 (JSON path + 運算子)
/// 範例: "$.value > 85", "$.status == \"critical\""
pub fn evaluate_condition(condition: &str, payload: Option<&str>) -> bool {
    let condition = condition.trim();
    if condition.is_empty() { return true; }  // 空條件 = 無條件通過

    let Some(payload) = payload.filter(|p| !p.is_empty()) else {
        return false;  // 無 payload = fail-closed
    };

    if condition.starts_with('$') {
        evaluate_json_path_condition(condition, payload)
    } else {
        evaluate_direct_condition(condition, payload)
    }
}

fn evaluate_json_path_condition(condition: &str, payload: &str) -> bool {
    // 解析 "$.path.to.value <op> <rhs>"
    let parts: Vec<&str> = condition.splitn(2, |c: char| matches!(c, '>' | '<' | '=' | '!'))
        .collect();
    if parts.len() < 2 { return false; }

    let path = parts[0].trim();
    let rest = &condition[path.len()..];

    // 提取運算子和右值
    let (op, rhs) = parse_operator_and_rhs(rest)?;

    // 解析 JSON payload 並沿路徑取值
    let json: Value = serde_json::from_str(payload).ok()?;
    let value = resolve_json_path(&json, path)?;

    // 比較
    compare_values(&value, op, rhs)
}

fn resolve_json_path<'a>(json: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = json;
    for segment in path.trim_start_matches("$.").split('.') {
        current = if let Ok(idx) = segment.parse::<usize>() {
            current.get(idx)?
        } else {
            current.get(segment)?
        };
    }
    Some(current)
}

#[derive(Debug, Clone, Copy)]
enum CompareOp { Gt, Lt, Gte, Lte, Eq, Neq }

fn compare_values(lhs: &Value, op: CompareOp, rhs: &str) -> bool {
    match (lhs.as_f64(), rhs.parse::<f64>()) {
        (Some(l), Ok(r)) => match op {
            CompareOp::Gt => l > r,
            CompareOp::Lt => l < r,
            CompareOp::Gte => l >= r,
            CompareOp::Lte => l <= r,
            CompareOp::Eq => (l - r).abs() < f64::EPSILON,
            CompareOp::Neq => (l - r).abs() >= f64::EPSILON,
        },
        _ => {
            // 字串比較
            let l = lhs.as_str().unwrap_or("");
            let r = rhs.trim_matches('"');
            match op {
                CompareOp::Eq => l == r,
                CompareOp::Neq => l != r,
                _ => false,
            }
        }
    }
}
```

Rust skeleton 建議 — 冷卻時間 + 並行控制:

```rust
// hand.toml 擴展
// [settings]
// cooldown_secs = "60"
// max_concurrent = "3"

// src/hands/runner.rs 擴展
use std::collections::HashMap;
use std::time::Instant;
use tokio::sync::Semaphore;

pub struct HandRunner {
    // 現有欄位...
    cooldowns: HashMap<String, Instant>,           // hand_name → 上次執行時間
    semaphores: HashMap<String, Arc<Semaphore>>,   // hand_name → 並行限制
}

impl HandRunner {
    pub async fn run_hand(&mut self, name: &str, prompt: &str) -> Result<String> {
        // 冷卻時間檢查
        if let Some(last_run) = self.cooldowns.get(name) {
            let cooldown = self.get_cooldown_secs(name);
            if last_run.elapsed() < Duration::from_secs(cooldown) {
                return Err(anyhow!("Hand '{}' is in cooldown ({} secs remaining)",
                    name, cooldown - last_run.elapsed().as_secs()));
            }
        }

        // 並行控制
        let permit = if let Some(sem) = self.semaphores.get(name) {
            Some(sem.acquire().await?)
        } else { None };

        let result = self.execute_phases(name, prompt).await;

        // 記錄執行時間 (用於冷卻計算)
        self.cooldowns.insert(name.to_string(), Instant::now());

        drop(permit);  // 釋放並行許可
        result
    }
}
```

---

## 4. 35+ 工具系統 -- Tool Trait 與安全模型

### 4.1 Tool Trait 定義

**檔案**: `src/tools/traits.rs` (122 行)

```rust
// src/tools/traits.rs:22-43
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult>;
    fn spec(&self) -> ToolSpec {  // 自動生成
        ToolSpec { name: self.name().into(), description: self.description().into(),
                   parameters: self.parameters_schema() }
    }
}

pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}
```

### 4.2 工具分類與完整列表

| 類別 | 工具 | 檔案 |
|------|------|------|
| 檔案 | shell, file_read, file_write, file_edit, glob_search, content_search | 6 檔案 |
| 網路 | web_search, web_fetch, http_request, browser, browser_open, screenshot | 6 檔案 |
| 記憶 | memory_store, memory_recall, memory_forget | 3 檔案 |
| 排程 | cron_add, cron_list, cron_remove, cron_run, cron_runs, cron_update, schedule | 7 檔案 |
| SOP | sop_execute, sop_advance, sop_approve, sop_list, sop_status | 5 檔案 |
| 代理 | delegate | 1 檔案 |
| 硬體 | hardware_board_info, hardware_memory_map, hardware_memory_read | 3 檔案 |
| 媒體 | image_info, pdf_read | 2 檔案 |
| 整合 | composio, pushover, git_operations | 3 檔案 |
| 設定 | model_routing_config, proxy_config | 2 檔案 |
| 資料 | schema (SchemaCleanr), cli_discovery | 2 檔案 |

### 4.3 工具安全管線

```
Tool Execution Request
  │
  ├── 1. E-Stop 檢查: EstopState.is_engaged()? → 拒絕
  │     └── frozen_tools 包含此工具? → 拒絕
  │
  ├── 2. AutonomyLevel 檢查:
  │     ├── ReadOnly + ToolOperation::Act → 拒絕
  │     ├── Supervised + High risk → 需批准
  │     └── Full → 在策略範圍內執行
  │
  ├── 3. 速率限制: ActionTracker.record() > max_actions_per_hour? → 拒絕
  │
  ├── 4. 路徑邊界: workspace_only + forbidden_paths 檢查
  │
  ├── 5. 命令風險評估 (shell 工具):
  │     ├── split_unquoted_segments() — quote-aware 分割
  │     ├── skip_env_assignments() — 跳過 FOO=bar 前綴
  │     └── 每個 segment 評估 CommandRiskLevel
  │
  ├── 6. Sandbox 包裝: sandbox.wrap_command(&mut cmd)
  │
  ├── 7. 執行: tool.execute(args)
  │
  ├── 8. 輸出清洗: scrub_credentials(output) + LeakDetector.scan()
  │
  └── 9. 審計: AuditLogger.log_command_event()
```

**與 clawtex-core Tool 設計的對比**:

```rust
// clawtex-core src/tools/mod.rs — Tool trait
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;
    fn approval_requirement(&self) -> ApprovalRequirement;  // clawtex 獨有
    async fn execute(&self, args: Value, config: &SecurityConfig) -> Result<String>;
}
```

差異:
- clawtex 傳入 `SecurityConfig` 到 execute — 耦合更深但更簡單
- clawtex 有 `ApprovalRequirement` — ZeroClaw 在 policy.rs 處理
- ZeroClaw 的 `ToolResult` 是 struct — clawtex 直接回傳 `String`

**Clawtex 實作建議**

```rust
// clawtex-core 改善建議: 統一 ToolResult
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

// 在 execute() 回傳前加入 leak detection
let raw_output = tool.execute(args, config).await?;
let leak_result = LEAK_DETECTOR.scan(&raw_output);
let safe_output = match leak_result {
    LeakResult::Detected { redacted, .. } => redacted,
    LeakResult::Clean => raw_output,
};
```

---

## 5. Channel Trait -- 20+ 通道抽象層

### 5.1 Channel Trait 完整定義

**檔案**: `src/channels/traits.rs` (270 行)

```rust
// src/channels/traits.rs:59-155
#[async_trait]
pub trait Channel: Send + Sync {
    // 核心方法 (必須實作)
    fn name(&self) -> &str;
    async fn send(&self, message: &SendMessage) -> Result<()>;
    async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()>;

    // 健康檢查 (預設: true)
    async fn health_check(&self) -> bool { true }

    // 打字指示器
    async fn start_typing(&self, _recipient: &str) -> Result<()> { Ok(()) }
    async fn stop_typing(&self, _recipient: &str) -> Result<()> { Ok(()) }

    // Draft 更新系統 (串流顯示)
    fn supports_draft_updates(&self) -> bool { false }
    async fn send_draft(&self, _msg: &SendMessage) -> Result<Option<String>> { Ok(None) }
    async fn update_draft(&self, _recipient: &str, _msg_id: &str, _text: &str) -> Result<()> { Ok(()) }
    async fn finalize_draft(&self, _recipient: &str, _msg_id: &str, _text: &str) -> Result<()> { Ok(()) }
    async fn cancel_draft(&self, _recipient: &str, _msg_id: &str) -> Result<()> { Ok(()) }

    // 反應/表情
    async fn add_reaction(&self, _chan: &str, _msg: &str, _emoji: &str) -> Result<()> { Ok(()) }
    async fn remove_reaction(&self, _chan: &str, _msg: &str, _emoji: &str) -> Result<()> { Ok(()) }

    // 訊息釘選
    async fn pin_message(&self, _chan: &str, _msg: &str) -> Result<()> { Ok(()) }
    async fn unpin_message(&self, _chan: &str, _msg: &str) -> Result<()> { Ok(()) }
}
```

### 5.2 能力矩陣

| Channel | Draft | Reaction | Pin | Thread | TTS | Transcription |
|---------|-------|----------|-----|--------|-----|---------------|
| Telegram | V | V | V | - | - | - |
| Discord | V | V | V | V | - | - |
| Slack | V | V | V | V | - | - |
| WhatsApp | - | V | - | - | - | - |
| WhatsApp Web | V | V | - | - | - | - |
| Matrix | V | V | - | V | - | - |
| CLI | V | - | - | - | - | - |
| MQTT | - | - | - | - | - | - |
| Email | - | - | - | V | - | - |

### 5.3 SendMessage 建構器

```rust
// src/channels/traits.rs:19-57
pub struct SendMessage {
    pub content: String,
    pub recipient: String,
    pub subject: Option<String>,
    pub thread_ts: Option<String>,  // 執行緒 ID
}

impl SendMessage {
    pub fn new(content: impl Into<String>, recipient: impl Into<String>) -> Self { ... }
    pub fn with_subject(content: _, recipient: _, subject: _) -> Self { ... }
    pub fn in_thread(mut self, thread_ts: Option<String>) -> Self { ... }
}
```

### 5.4 Draft 更新流程

```
Provider Streaming
  │
  ├── 1. send_draft(initial_chunk) → msg_id
  │
  ├── 2. accumulate chunks (>= 80 chars)
  │     └── update_draft(recipient, msg_id, accumulated_text)
  │         (重複直到串流結束)
  │
  ├── 3a. 成功: finalize_draft(recipient, msg_id, final_formatted_text)
  │
  └── 3b. 取消: cancel_draft(recipient, msg_id)
```

常數:
```rust
const STREAM_CHUNK_MIN_CHARS: usize = 80;     // 80 字元以下不更新 draft (避免頻繁 API 呼叫)
const PROGRESS_MIN_INTERVAL_MS: u64 = 500;     // 500ms 最小進度更新間隔
const DRAFT_CLEAR_SENTINEL: &str = "\x00CLEAR\x00";  // 清除 draft 累積文字的信號
```

### 5.5 ChannelMessage — 入站訊息結構

```rust
// 推斷自 channels/traits.rs + loop_.rs
pub struct ChannelMessage {
    pub content: String,
    pub sender: String,         // 發送者 ID
    pub sender_name: String,    // 發送者顯示名稱
    pub channel: String,        // 來源通道名稱
    pub thread_ts: Option<String>,  // 執行緒 ID (Slack/Discord)
    pub reply_to: String,       // 回覆目標 ID
    pub attachments: Vec<Attachment>,  // 附件 (圖片、檔案)
}
```

### 5.6 26 Channel 實作列表

```
src/channels/
├── telegram.rs         # Telegram Bot API
├── discord.rs          # Discord (serenity/http)
├── slack.rs            # Slack (Web API + Events API)
├── whatsapp.rs         # WhatsApp Cloud API
├── whatsapp_web.rs     # WhatsApp Web (Puppeteer/ws)
├── matrix.rs           # Matrix (matrix-sdk)
├── cli.rs              # CLI 互動 (stdin/stdout)
├── mqtt.rs             # MQTT (rumqttc)
├── email.rs            # Email (lettre + imap)
├── sms.rs              # SMS (Twilio)
├── wati.rs             # Wati WhatsApp API
├── linq.rs             # LinQ 通訊
├── nextcloud_talk.rs   # Nextcloud Talk
├── signal.rs           # Signal (signal-cli)
├── teams.rs            # Microsoft Teams
├── webhook.rs          # Generic Webhook
├── web.rs              # Web Chat (Gateway SSE/WS)
├── nostr.rs            # Nostr Protocol
├── xmpp.rs             # XMPP/Jabber
├── irc.rs              # IRC
├── mastodon.rs         # Mastodon API
├── bluesky.rs          # Bluesky (AT Protocol)
├── rocketchat.rs       # Rocket.Chat
├── zulip.rs            # Zulip
├── mattermost.rs       # Mattermost
└── mod.rs              # 通道註冊工廠
```

**Clawtex 實作建議**

clawtex-core 目前只有 Telegram (`src/telegram.rs`)，使用 `edit_message_text` 模擬串流。

建議:
1. 抽象 Channel trait — 從 `telegram.rs` 提取介面 (~100 行)
2. 加入 Draft 系統到 Telegram channel — `send_draft` = `send_message`, `update_draft` = `edit_message_text` (~50 行)
3. 預留 Discord/Slack 擴展點
4. 加入 IncomingMessage struct 統一入站訊息格式

```rust
// src/channels/mod.rs (clawtex-core 建議, ~100 行)
use async_trait::async_trait;
use tokio::sync::mpsc;

pub struct IncomingMessage {
    pub content: String,
    pub sender_id: String,
    pub sender_name: String,
    pub channel_name: String,
    pub reply_to: String,           // 回覆目標 (chat_id for Telegram)
    pub thread_id: Option<String>,  // 執行緒 ID
    pub attachments: Vec<Attachment>,
}

pub struct Attachment {
    pub file_id: String,
    pub file_type: AttachmentType,
    pub file_name: Option<String>,
}

pub enum AttachmentType { Image, Document, Audio, Video }

#[async_trait]
pub trait Channel: Send + Sync {
    fn name(&self) -> &str;
    async fn send(&self, content: &str, recipient: &str) -> Result<()>;
    async fn listen(&self, tx: mpsc::Sender<IncomingMessage>) -> Result<()>;

    // 健康檢查
    async fn health_check(&self) -> bool { true }

    // 打字指示器
    async fn start_typing(&self, _recipient: &str) -> Result<()> { Ok(()) }
    async fn stop_typing(&self, _recipient: &str) -> Result<()> { Ok(()) }

    // Draft 更新系統 (串流顯示)
    fn supports_draft_updates(&self) -> bool { false }
    async fn send_draft(&self, content: &str, recipient: &str) -> Result<Option<String>> { Ok(None) }
    async fn update_draft(&self, recipient: &str, msg_id: &str, text: &str) -> Result<()> { Ok(()) }
    async fn finalize_draft(&self, recipient: &str, msg_id: &str, text: &str) -> Result<()> { Ok(()) }
    async fn cancel_draft(&self, _recipient: &str, _msg_id: &str) -> Result<()> { Ok(()) }

    // 反應/表情 (Discord/Slack)
    async fn add_reaction(&self, _channel: &str, _msg_id: &str, _emoji: &str) -> Result<()> { Ok(()) }

    // 訊息釘選
    async fn pin_message(&self, _channel: &str, _msg_id: &str) -> Result<()> { Ok(()) }
}

// Telegram Channel 實作示例
pub struct TelegramChannel {
    bot_token: String,
    client: reqwest::Client,
}

#[async_trait]
impl Channel for TelegramChannel {
    fn name(&self) -> &str { "telegram" }

    fn supports_draft_updates(&self) -> bool { true }

    async fn send_draft(&self, content: &str, recipient: &str) -> Result<Option<String>> {
        // send_message → 回傳 message_id
        let msg_id = self.send_message(recipient, content).await?;
        Ok(Some(msg_id))
    }

    async fn update_draft(&self, recipient: &str, msg_id: &str, text: &str) -> Result<()> {
        // edit_message_text (已存在的邏輯)
        self.edit_message_text(recipient, msg_id, text).await
    }

    async fn finalize_draft(&self, recipient: &str, msg_id: &str, text: &str) -> Result<()> {
        // 最終版 — 可加入格式化 (Markdown)
        self.edit_message_text(recipient, msg_id, text).await
    }
    // ...
}
```

**遷移路徑**: 從 `telegram.rs` 提取 Channel trait → 現有 Telegram 邏輯包裝為 `TelegramChannel` → Agent loop 改用 Channel trait → 未來加入 Discord/Slack 僅需新增實作。

---

## 6. SkillForge -- 技能自動發現/評估/整合

### 6.1 管線架構

**檔案**: `src/skillforge/` (4 檔案, ~600 行)

```
┌──────────┐     ┌──────────┐     ┌──────────┐
│  Scout    │────▶│ Evaluate │────▶│ Integrate│
│ (GitHub)  │     │ (3 維度) │     │ (TOML+MD)│
└──────────┘     └──────────┘     └──────────┘
  discover()       score()          write()
  → ScoutResult    → EvalResult     → SKILL.toml + SKILL.md
```

### 6.2 Scout — 技能發現

**檔案**: `src/skillforge/scout.rs` (340 行)

```rust
// src/skillforge/scout.rs:12-18
pub enum ScoutSource { GitHub, ClawHub, HuggingFace }

pub struct ScoutResult {
    pub name: String,
    pub url: String,
    pub description: String,
    pub stars: u64,
    pub language: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
    pub source: ScoutSource,
    pub owner: String,
    pub has_license: bool,
}
```

GitHubScout: 搜尋 "zeroclaw skill" + "ai agent skill"，30 結果/查詢，`OnceLock` 客戶端。

### 6.3 Evaluator — 3 維度評分

**檔案**: `src/skillforge/evaluate.rs` (273 行)

```rust
// src/skillforge/evaluate.rs:12-19
pub struct Scores {
    pub compatibility: f64,  // 語言相容性 (Rust=1.0, Python/TS/JS=0.6, 其他=0.3)
    pub quality: f64,        // log2(stars+1)/10, capped at 1.0
    pub security: f64,       // 0.5 基礎 + 0.3 license + 0.2 recency - 0.5 bad_pattern
}

impl Scores {
    pub fn total(&self) -> f64 {
        self.compatibility * 0.30 + self.quality * 0.35 + self.security * 0.35
    }
}

pub enum Recommendation {
    Auto,    // >= min_score (預設 0.7)
    Manual,  // >= 0.4
    Skip,    // < 0.4
}
```

安全掃描 — 壞模式全詞匹配:
```rust
const BAD_PATTERNS: &[&str] = &[
    "malware", "exploit", "hack", "crack", "keygen", "ransomware", "trojan",
];

// "hackathon" 不會匹配 "hack" (全詞邊界檢查)
fn contains_word(haystack: &str, word: &str) -> bool { ... }
```

### 6.4 Integrator — 產出 SKILL.toml + SKILL.md

**檔案**: `src/skillforge/integrate.rs` (253 行)

路徑安全: `sanitize_path_component()` — 拒絕 `..`、`/`、`\`、`\0`。

### 6.5 SkillForge 設定

```rust
pub struct SkillForgeConfig {
    pub enabled: bool,                  // 預設 false
    pub auto_integrate: bool,           // 預設 true
    pub sources: Vec<String>,           // ["github", "clawhub"]
    pub scan_interval_hours: u64,       // 預設 24
    pub min_score: f64,                 // 預設 0.7
    pub github_token: Option<String>,
    pub output_dir: String,             // 預設 "./skills"
}
```

**Clawtex 實作建議**

clawtex-core 的 `self_evolve` hand 已經有類似概念。建議整合 SkillForge 思路:

| 功能 | 改動建議 | 複雜度 |
|------|---------|--------|
| GitHub 技能搜尋 | 新增 `src/skillforge/scout.rs` | ~200 行, `reqwest` (已有) |
| 安全評分 | 新增 `src/skillforge/evaluate.rs` | ~150 行 |
| TOML 產出 | 整合到 `self_evolve` hand | ~100 行 |
| 壞模式檢測 | 共用 LeakDetector 的 pattern 匹配 | ~30 行 |

---

## 7. Gateway -- axum 路由、SSE/WS 雙通道

### 7.1 Gateway 架構

**檔案**: `src/gateway/mod.rs` (1200+ 行)

```
axum Router
├── GET  /                   → 內嵌前端 (rust-embed)
├── GET  /api/status         → 系統狀態
├── GET  /api/config         → 設定 (敏感欄位遮蔽)
├── PUT  /api/config         → 更新設定 (TOML)
├── GET  /api/tools          → 工具列表
├── GET  /api/memory         → 記憶查詢
├── POST /api/memory         → 記憶儲存
├── DELETE /api/memory/:key  → 記憶刪除
├── GET  /api/cron           → Cron 列表
├── POST /api/cron           → Cron 新增
├── DELETE /api/cron/:id     → Cron 刪除
├── POST /pair               → 裝置配對
├── POST /chat               → 單次對話
├── POST /webhook/:id        → Webhook 接收
├── POST /webhook/whatsapp   → WhatsApp Webhook
├── POST /webhook/linq       → LinQ Webhook
├── POST /webhook/wati       → Wati Webhook
├── POST /webhook/nextcloud  → Nextcloud Talk Webhook
├── GET  /api/events         → SSE 事件串流
├── GET  /ws/chat            → WebSocket 對話
└── GET  /health             → 健康檢查
```

### 7.2 安全防護

```rust
// src/gateway/mod.rs:47-55
pub const MAX_BODY_SIZE: usize = 65_536;          // 64KB 請求體上限
pub const REQUEST_TIMEOUT_SECS: u64 = 30;          // 防 slow-loris
pub const RATE_LIMIT_WINDOW_SECS: u64 = 60;        // 速率限制窗口
pub const RATE_LIMIT_MAX_KEYS_DEFAULT: usize = 10_000;  // IP 追蹤上限
pub const IDEMPOTENCY_MAX_KEYS_DEFAULT: usize = 10_000; // 冪等鍵上限
```

**SlidingWindowRateLimiter**:

```rust
// src/gateway/mod.rs:88-155
struct SlidingWindowRateLimiter {
    limit_per_window: u32,
    window: Duration,
    max_keys: usize,
    requests: Mutex<(HashMap<String, Vec<Instant>>, Instant)>,
}
```

**效能特徵**:
- `parking_lot::Mutex` — 非 `std::sync::Mutex` (無 poison, 更小的記憶體佔用)
- 定期清掃 (每 5 分鐘) 而非每次請求都 GC → 降低延遲
- 超過 max_keys 時 LRU 驅逐 → 記憶體有界

**SlidingWindowRateLimiter 資料流**:

```
HTTP Request → extract IP → SlidingWindowRateLimiter
  │
  ├── lock() → (HashMap<IP, Vec<Instant>>, last_sweep: Instant)
  │
  ├── 清掃檢查: now - last_sweep > 5 min?
  │     └── YES: 移除所有過期條目 + 超出 max_keys 的最舊條目
  │
  ├── 取得 IP 的時間戳列表
  │     └── retain(|t| now - t < window)  // 移除窗口外的記錄
  │
  ├── 計數: timestamps.len() >= limit_per_window?
  │     ├── YES: → 429 Too Many Requests
  │     └── NO:  timestamps.push(now) → 放行
  │
  └── unlock()
```

**IdempotencyStore — 請求去重**:

```rust
// src/gateway/mod.rs:167-190
struct IdempotencyStore {
    ttl: Duration,              // 鍵存活時間
    max_keys: usize,            // 最大鍵數
    keys: Mutex<HashMap<String, Instant>>,
}

impl IdempotencyStore {
    fn check(&self, key: &str) -> bool {
        let mut keys = self.keys.lock();
        // 清理過期鍵
        keys.retain(|_, &mut ts| ts.elapsed() < self.ttl);
        if keys.contains_key(key) {
            return true;  // 重複請求
        }
        if keys.len() >= self.max_keys {
            // 移除最舊的鍵
            if let Some(oldest) = keys.iter().min_by_key(|(_, ts)| *ts).map(|(k, _)| k.clone()) {
                keys.remove(&oldest);
            }
        }
        keys.insert(key.to_string(), Instant::now());
        false  // 新請求
    }
}
```

使用方式: 客戶端發送 `Idempotency-Key: <uuid>` header → 重複請求回傳快取結果。

### 7.3 SSE — 伺服器推送事件

**檔案**: `src/gateway/sse.rs` (159 行)

```rust
// src/gateway/sse.rs:19-57
pub async fn handle_sse_events(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let rx = state.event_tx.subscribe();  // broadcast::Sender<Value>
    let stream = BroadcastStream::new(rx)
        .filter_map(|result| match result {
            Ok(value) => Some(Ok::<_, Infallible>(Event::default().data(value.to_string()))),
            Err(_) => None,  // 跳過延遲訊息
        });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
```

**BroadcastObserver** — 橋接 Observer trait 和 SSE:

```rust
// src/gateway/sse.rs:62-158
impl Observer for BroadcastObserver {
    fn record_event(&self, event: &ObserverEvent) {
        self.inner.record_event(event);  // 轉發到原有 observer
        let json = match event {
            ObserverEvent::LlmRequest { provider, model, .. } => json!({
                "type": "llm_request", "provider": provider, "model": model
            }),
            ObserverEvent::ToolCall { tool, duration, success } => json!({
                "type": "tool_call", "tool": tool,
                "duration_ms": duration.as_millis(), "success": success
            }),
            // ... AgentStart, AgentEnd, Error, ToolCallStart
            _ => return,  // 不廣播的事件
        };
        let _ = self.tx.send(json);
    }
}
```

### 7.4 WebSocket — 即時對話

**檔案**: `src/gateway/ws.rs` (186 行)

協議:
```
Client → Server: {"type":"message","content":"Hello"}
Server → Client: {"type":"chunk","content":"Hi! "}
Server → Client: {"type":"tool_call","name":"shell","args":{...}}
Server → Client: {"type":"tool_result","name":"shell","output":"..."}
Server → Client: {"type":"done","full_response":"..."}
```

```rust
// src/gateway/ws.rs:25
const WS_PROTOCOL: &str = "zeroclaw.v1";  // 子協議標識
```

認證方式: `?token=<bearer_token>` (瀏覽器 WebSocket 限制，無法設定 header)。

### 7.5 內嵌前端

**檔案**: `src/gateway/static_files.rs`

使用 `rust-embed` crate 將 `web/` 目錄編譯進二進位檔。

**Clawtex 實作建議**

clawtex-core 的 `src/http_gateway.rs` 已有 axum 路由。差距:

| 功能 | clawtex 現況 | 改動建議 | 複雜度 |
|------|-------------|---------|--------|
| SSE 事件串流 | **無** | 新增 SSE handler + BroadcastObserver | ~150 行 |
| WebSocket 對話 | **無** | 新增 WS handler (axum ws 支援) | ~100 行 |
| 速率限制器 | 基本 | 替換為 SlidingWindowRateLimiter | ~80 行 |
| 冪等鍵 | **無** | 新增 IdempotencyStore | ~50 行 |
| 內嵌前端 | **無** | 新增 `rust-embed` crate | ~30 行 |

SSE skeleton:

```rust
// src/http_gateway.rs 建議加入
use axum::response::sse::{Event, KeepAlive, Sse};
use tokio_stream::wrappers::BroadcastStream;

pub async fn handle_sse(State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.event_bus.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|r| match r {
        Ok(v) => Some(Ok::<_, Infallible>(Event::default().data(v.to_string()))),
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
```

---

## 8. Agent Runtime -- 建構者模式與訊息迴圈

### 8.1 Agent struct

**檔案**: `src/agent/agent.rs`

```rust
pub struct Agent {
    provider: Box<dyn Provider>,
    tools: Vec<Box<dyn Tool>>,
    tool_specs: Vec<ToolSpec>,
    memory: Arc<dyn Memory>,
    observer: Arc<dyn Observer>,
    prompt_builder: SystemPromptBuilder,
    tool_dispatcher: Box<dyn ToolDispatcher>,   // XML 或 Native
    memory_loader: Box<dyn MemoryLoader>,
    config: AgentConfig,
    model_name: String,
    temperature: f64,
    workspace_dir: PathBuf,
    identity_config: IdentityConfig,
    skills: Vec<Skill>,
    history: Vec<ConversationMessage>,
    classification_config: QueryClassificationConfig,
    available_hints: Vec<String>,
    route_model_by_hint: HashMap<String, String>,
}
```

### 8.2 Tool Call 解析 — 4 種格式支援

**檔案**: `src/agent/loop_.rs` (500+ 行解析邏輯)

ZeroClaw 的工具呼叫解析比 clawtex 更深入，支援 4 種格式:

```
LLM Response → Tool Call Parser
  │
  ├── 1. Native JSON (OpenAI/Anthropic)
  │     ChatResponse.tool_calls → ParsedToolCall[]
  │     支援: function.name, function.arguments, id/tool_call_id/call_id
  │
  ├── 2. XML 格式 (Ollama/DeepSeek/本地模型)
  │     <tool_call>{"name":"shell","arguments":{"command":"ls"}}</tool_call>
  │     extract_xml_pairs() → 遞迴解析所有 XML 標籤對
  │     過濾 meta 標籤: tool_call, thinking, thought, analysis, reasoning, reflection
  │
  ├── 3. MiniMax Invoke 格式
  │     <invoke name="shell"><parameter name="command">pwd</parameter></invoke>
  │     MINIMAX_INVOKE_RE + MINIMAX_PARAMETER_RE 正則
  │
  └── 4. JSON Code Block 格式
        ```json {"name":"shell","arguments":{"command":"ls"}} ```
        parse_tool_calls_from_json_value() — 支援:
        - {"tool_calls": [...]}
        - [{"name":"...", "arguments":{}}]
        - {"function": {"name":"...", "arguments":{}}}
        - {"name":"...", "parameters":{}} (parameters 作為 arguments 別名)
```

**ParsedToolCall 結構**:

```rust
// src/agent/loop_.rs (推斷)
struct ParsedToolCall {
    name: String,
    arguments: serde_json::Value,
    tool_call_id: Option<String>,
}
```

**tool_call_id 解析 — 多來源容錯**:

```rust
// src/agent/loop_.rs:337-350
fn parse_tool_call_id(root: &Value, function: Option<&Value>) -> Option<String> {
    function.and_then(|func| func.get("id"))   // function.id
        .or_else(|| root.get("id"))             // root.id
        .or_else(|| root.get("tool_call_id"))   // root.tool_call_id
        .or_else(|| root.get("call_id"))        // root.call_id
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(ToString::to_string)
}
```

**arguments 解析 — 字串/物件雙模式**:

```rust
// src/agent/loop_.rs:328-335
fn parse_arguments_value(raw: Option<&Value>) -> Value {
    match raw {
        Some(Value::String(s)) => serde_json::from_str::<Value>(s)
            .unwrap_or_else(|_| Value::Object(Map::new())),  // 字串 → 嘗試反序列化
        Some(value) => value.clone(),                         // 已經是物件
        None => Value::Object(Map::new()),                    // 無參數 → 空物件
    }
}
```

**重複工具呼叫偵測 — 正規化簽名**:

```rust
// src/agent/loop_.rs:352-379
fn canonicalize_json_for_tool_signature(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort_unstable();  // 鍵排序確保同等物件生成相同簽名
            // ...遞迴處理子物件
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json_for_tool_signature).collect()),
        _ => value.clone(),
    }
}

fn tool_call_signature(name: &str, arguments: &Value) -> (String, String) {
    let canonical_args = canonicalize_json_for_tool_signature(arguments);
    let args_json = serde_json::to_string(&canonical_args).unwrap_or_else(|_| "{}".to_string());
    (name.trim().to_ascii_lowercase(), args_json)
}
```

此機制避免 LLM 反覆呼叫相同工具導致無限迴圈 (例如呼叫 `shell("ls")` 10 次)。

### 8.3 ToolDispatcher trait — XML/Native 雙模

**檔案**: `src/agent/dispatcher.rs`

```rust
pub trait ToolDispatcher: Send + Sync {
    fn parse_response(&self, response: &ChatResponse) -> (String, Vec<ParsedToolCall>);
    fn format_results(&self, results: &[ToolExecutionResult]) -> ConversationMessage;
    fn prompt_instructions(&self, tools: &[Box<dyn Tool>]) -> String;
    fn to_provider_messages(&self, history: &[ConversationMessage]) -> Vec<ChatMessage>;
    fn should_send_tool_specs(&self) -> bool;
}
```

自動選擇: `provider.supports_native_tools()` ? Native : XML

XML 模式解析 `<tool_call>{"name":"...","arguments":{...}}</tool_call>`，並移除 `<think>` 標籤。

### 8.4 scrub_credentials — 憑證清洗

**檔案**: `src/agent/loop_.rs:34-95`

```rust
// 敏感鍵偵測 (LazyLock + RegexSet)
static SENSITIVE_KEY_PATTERNS: LazyLock<RegexSet> = LazyLock::new(|| {
    RegexSet::new([
        r"(?i)token", r"(?i)api[_-]?key", r"(?i)password",
        r"(?i)secret", r"(?i)user[_-]?key", r"(?i)bearer", r"(?i)credential",
    ]).unwrap()
});

// 鍵值對正則 — 匹配 key="value", key=value, key: value
static SENSITIVE_KV_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(token|api[_-]?key|password|secret|user[_-]?key|bearer|credential)["']?\s*[:=]\s*(?:"([^"]{8,})"|'([^']{8,})'|([a-zA-Z0-9_\-\.]{8,}))"#).unwrap()
});

pub(crate) fn scrub_credentials(input: &str) -> String {
    SENSITIVE_KV_REGEX.replace_all(input, |caps: &Captures| {
        let key = &caps[1];
        let val = caps.get(2).or(caps.get(3)).or(caps.get(4)).map(|m| m.as_str()).unwrap_or("");
        // 保留前 4 個字元作為上下文 (使用 char_indices 避免 UTF-8 中間截斷)
        let prefix = if val.len() > 4 {
            val.char_indices().nth(4).map(|(byte_idx, _)| &val[..byte_idx]).unwrap_or(val)
        } else { "" };
        format!("{key}: {prefix}*[REDACTED]")
    }).to_string()
}
```

設計亮點:
- `char_indices().nth(4)` — 防止多位元組 UTF-8 字元中間截斷 (與 clawtex 修過的同一個 bug)
- 8 字元最低門檻 — 避免將短配置值 (如 `token=true`) 誤判為機密
- 保留前 4 字元前綴 — 幫助人工除錯 (`sk-proj-Abc*[REDACTED]`)

### 8.5 build_context — 記憶驅動上下文組裝

**檔案**: `src/agent/loop_.rs:248-321`

```
build_context(memory, user_msg, min_relevance_score)
  │
  ├── memory.recall(user_msg, limit=5, session_id=None)
  │     └── 語義搜尋，回傳帶 score 的 MemoryEntry[]
  │
  ├── 過濾: score >= min_relevance_score (預設 0.3)
  │     └── 排除 assistant 自動儲存的記憶 (is_assistant_autosave_key)
  │
  └── 格式化: "[Memory context]\n- key: content\n..."
```

```
build_hardware_context(rag, user_msg, boards, chunk_limit)
  │
  ├── rag.pin_alias_context(user_msg, boards)
  │     └── "red_led" → "red_led: 13 (pin 13 on Arduino)"
  │
  └── rag.retrieve(user_msg, boards, chunk_limit)
        └── "[Hardware documentation]\n--- source (board) ---\ncontent\n"
```

### 8.6 工具執行迴圈常數

```rust
const DEFAULT_MAX_TOOL_ITERATIONS: usize = 10;    // 最大工具呼叫輪次
const AUTOSAVE_MIN_MESSAGE_CHARS: usize = 20;     // 自動記憶儲存最低字元數
const DEFAULT_MAX_HISTORY_MESSAGES: usize = 50;    // 觸發壓縮門檻
const COMPACTION_KEEP_RECENT_MESSAGES: usize = 20; // 壓縮後保留最近訊息數
const COMPACTION_MAX_SOURCE_CHARS: usize = 12_000; // 壓縮來源最大字元
const COMPACTION_MAX_SUMMARY_CHARS: usize = 2_000; // 摘要最大字元
const STREAM_CHUNK_MIN_CHARS: usize = 80;          // 串流 chunk 最小字元
const PROGRESS_MIN_INTERVAL_MS: u64 = 500;         // 進度更新最小間隔
const DRAFT_CLEAR_SENTINEL: &str = "\x00CLEAR\x00"; // Draft 清除標記
```

### 8.7 工具呼叫迴圈完整資料流

```
User Message → Agent Loop
  │
  ├── 1. build_context() — 注入相關記憶
  ├── 2. build_hardware_context() — 注入硬體文檔 (如適用)
  ├── 3. trim_history() — 裁剪到 max_history
  ├── 4. auto_compact_history() — 超過門檻則 LLM 摘要壓縮
  │
  ├── 5. provider.chat(request, model, temperature)
  │     └── ChatResponse { text, tool_calls, usage, reasoning_content }
  │
  ├── 6. 解析 tool calls
  │     ├── Native: ChatResponse.tool_calls
  │     └── XML: extract_xml_pairs() + parse_tool_calls_from_json_value()
  │
  ├── 7. for each tool_call (max DEFAULT_MAX_TOOL_ITERATIONS):
  │     ├── 重複呼叫偵測: tool_call_signature() → HashSet 比對
  │     ├── E-Stop 檢查: estop.is_engaged()?
  │     ├── SecurityPolicy.validate_command_execution() (shell 工具)
  │     ├── SecurityPolicy.forbidden_path_argument() (路徑工具)
  │     ├── ActionTracker.record() — 速率限制
  │     ├── tool.execute(args) → ToolResult
  │     ├── scrub_credentials(output) — 清洗機密
  │     ├── truncate_tool_args_for_progress() — 進度顯示
  │     └── AuditLogger.log() — 審計記錄
  │
  ├── 8. 將 ToolResults 附加到歷史
  │     └── ConversationMessage::ToolResults(results)
  │
  ├── 9. 再次呼叫 provider (帶工具結果)
  │     └── 重複直到 LLM 不再呼叫工具或達到最大輪次
  │
  └── 10. 最終回應
        ├── 自動記憶儲存 (>= AUTOSAVE_MIN_MESSAGE_CHARS)
        └── Draft finalize → 通道輸出
```

### 8.8 ConversationMessage — 三態歷史

```rust
pub enum ConversationMessage {
    Chat(ChatMessage),
    AssistantToolCalls {
        text: Option<String>,
        tool_calls: Vec<ToolCall>,
        reasoning_content: Option<String>,
    },
    ToolResults(Vec<ToolResultMessage>),
}
```

### 8.9 自動壓縮

```rust
async fn auto_compact_history(
    history: &mut Vec<ChatMessage>,
    provider: &dyn Provider,
    model: &str,
    max_history: usize,
) -> Result<bool> {
    // 將舊訊息轉文字 → LLM 生成摘要 → 替換為 [Compaction summary]
}
```

常數: `COMPACTION_KEEP_RECENT_MESSAGES = 20`, `COMPACTION_MAX_SOURCE_CHARS = 12,000`

**Clawtex 實作建議**

clawtex-core 已有 `src/dispatcher.rs` (Native/XML/FunctionTag)。需要改善:

| 功能 | clawtex 現況 | 改動 | 複雜度 |
|------|-------------|------|--------|
| ConversationMessage 三態 | 扁平 ChatMessage | 新增 enum | ~50 行 |
| ToolDispatcher trait | 已有 (更完整) | 無需改動 | - |
| AgentBuilder | 直接建構 | 新增 builder pattern | ~100 行 |
| reasoning_content 保留 | **無** | 加到 ConversationMessage | ~20 行 |

---

## 9. Provider 系統 -- 可靠性包裝與路由

### 9.1 ReliableProvider — 三層容錯策略

**檔案**: `src/providers/reliable.rs` (1500+ 行)

```
三層容錯策略:
  ┌────────────────────────────────────────────────────────┐
  │  外層: 模型後備鏈 (Model Fallback Chain)                │
  │  [原始模型] → [fallback_1] → [fallback_2] → ...       │
  │                                                        │
  │  ┌──────────────────────────────────────────────────┐  │
  │  │  中層: Provider 優先序 (Provider Priority)        │  │
  │  │  [provider_1] → [provider_2] → [provider_3]     │  │
  │  │                                                  │  │
  │  │  ┌──────────────────────────────────────────┐    │  │
  │  │  │  內層: 指數退避重試 (Retry Loop)           │    │  │
  │  │  │  attempt_1 → backoff → attempt_2 → ...   │    │  │
  │  │  │  + API Key 輪替 (on 429)                  │    │  │
  │  │  │  + Retry-After 解析 (cap 30s)             │    │  │
  │  │  └──────────────────────────────────────────┘    │  │
  │  └──────────────────────────────────────────────────┘  │
  └────────────────────────────────────────────────────────┘

  不變量: failures 累積每次失敗嘗試 → 最終錯誤提供完整診斷
```

**ReliableProvider struct**:

```rust
// src/providers/reliable.rs:225-234
pub struct ReliableProvider {
    providers: Vec<(String, Box<dyn Provider>)>,
    max_retries: u32,
    base_backoff_ms: u64,               // 最低 50ms
    api_keys: Vec<String>,               // 輪替 API keys
    key_index: AtomicUsize,              // Round-robin 索引
    model_fallbacks: HashMap<String, Vec<String>>,  // 模型→後備模型列表
}
```

**錯誤分類 — 4 層判斷**:

```rust
// src/providers/reliable.rs:18-72
fn is_non_retryable(err: &anyhow::Error) -> bool {
    // Layer 1: 上下文窗口超出 → 永遠不可重試
    if is_context_window_exceeded(err) { return true; }

    // Layer 2: HTTP status code (reqwest 型別安全提取)
    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
        if let Some(status) = reqwest_err.status() {
            let code = status.as_u16();
            return status.is_client_error() && code != 429 && code != 408;
        }
    }

    // Layer 3: 字串回退 (某些 provider 在錯誤訊息中嵌入 status code)
    let msg = err.to_string();
    for word in msg.split(|c: char| !c.is_ascii_digit()) {
        if let Ok(code) = word.parse::<u16>() {
            if (400..500).contains(&code) {
                return code != 429 && code != 408;
            }
        }
    }

    // Layer 4: 語義啟發式 (gRPC/自定義傳輸沒有 HTTP status)
    let msg_lower = msg.to_lowercase();
    let auth_hints = ["invalid api key", "authentication failed",
                      "unauthorized", "forbidden", "permission denied", ...];
    if auth_hints.iter().any(|h| msg_lower.contains(h)) { return true; }

    // 模型不存在
    msg_lower.contains("model") && (msg_lower.contains("not found")
        || msg_lower.contains("unsupported") || msg_lower.contains("does not exist"))
}
```

**上下文窗口超出偵測 — 8 種模式**:

```rust
// src/providers/reliable.rs:74-88
fn is_context_window_exceeded(err: &anyhow::Error) -> bool {
    let lower = err.to_string().to_lowercase();
    let hints = [
        "exceeds the context window", "context window of this model",
        "maximum context length", "context length exceeded",
        "too many tokens", "token limit exceeded",
        "prompt is too long", "input is too long",
    ];
    hints.iter().any(|hint| lower.contains(hint))
}
```

**非重試型速率限制 — 商業/配額錯誤**:

```rust
// src/providers/reliable.rs:108-146
fn is_non_retryable_rate_limit(err: &anyhow::Error) -> bool {
    if !is_rate_limited(err) { return false; }
    let lower = err.to_string().to_lowercase();
    let business_hints = [
        "plan does not include", "insufficient balance",
        "quota exhausted", "out of credits",
        "no available package", "package not active",
    ];
    // 已知 provider 商業錯誤碼: Z.AI 1311, 1113
    // 這些 429 是帳號層級限制，重試無效
}
```

**Retry-After 解析**:

```rust
// src/providers/reliable.rs:150-179
fn parse_retry_after_ms(err: &anyhow::Error) -> Option<u64> {
    let msg = err.to_string();
    for prefix in &["retry-after:", "retry_after:", "retry-after ", "retry_after "] {
        if let Some(pos) = lower.find(prefix) {
            let num_str: String = after.trim().chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.').collect();
            if let Ok(secs) = num_str.parse::<f64>() {
                return Some(Duration::from_secs_f64(secs).as_millis() as u64);
            }
        }
    }
    None
}

fn compute_backoff(&self, base: u64, err: &anyhow::Error) -> u64 {
    if let Some(retry_after) = parse_retry_after_ms(err) {
        retry_after.min(30_000).max(base)  // 最大 30 秒，最小 = base
    } else {
        base
    }
}
```

**API Key 輪替**:

```rust
// src/providers/reliable.rs:274-280
fn rotate_key(&self) -> Option<&str> {
    if self.api_keys.is_empty() { return None; }
    let idx = self.key_index.fetch_add(1, Ordering::Relaxed) % self.api_keys.len();
    Some(&self.api_keys[idx])
}
```

**效能特徵**:
- 指數退避重試: base_backoff_ms → 2x → 4x → ... (受 Retry-After 覆蓋)
- 模型後備鏈: `model_chain()` 回傳 [原始, fallback1, fallback2, ...]
- API Key 輪替: 429 時自動切換到下一個 API key (不同配額桶)
- 上下文窗口超出: 立即終止，不重試不後備 (`anyhow::bail!`)
- 商業 429: 非重試型 (配額耗盡、方案不支援等)

### 9.2 Provider Trait — 完整定義與能力聲明

**檔案**: `src/providers/traits.rs` (400+ 行)

```rust
// src/providers/traits.rs:225-236
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub native_tool_calling: bool,  // 原生函式呼叫 (Gemini/Anthropic/OpenAI)
    pub vision: bool,                // 視覺輸入支援
}
```

**ToolsPayload — 4 種 provider 格式**:

```rust
// src/providers/traits.rs:243-255
pub enum ToolsPayload {
    Gemini { function_declarations: Vec<Value> },  // functionDeclarations 格式
    Anthropic { tools: Vec<Value> },                // input_schema 格式
    OpenAI { tools: Vec<Value> },                   // function 格式
    PromptGuided { instructions: String },           // 系統提示注入 (回退方案)
}
```

**Provider Trait — 8 個方法 (3 必須, 5 預設)**:

```rust
// src/providers/traits.rs:257-440
#[async_trait]
pub trait Provider: Send + Sync {
    // 能力查詢 (預設: 無原生工具)
    fn capabilities(&self) -> ProviderCapabilities { ProviderCapabilities::default() }

    // 工具格式轉換 (預設: 提示注入)
    fn convert_tools(&self, tools: &[ToolSpec]) -> ToolsPayload { ... }

    // 必須實作: 帶系統提示的一次性對話
    async fn chat_with_system(&self, system: Option<&str>, message: &str,
                               model: &str, temperature: f64) -> Result<String>;

    // 多輪對話 (預設: 提取最後用戶訊息 → chat_with_system)
    async fn chat_with_history(&self, messages: &[ChatMessage],
                                model: &str, temperature: f64) -> Result<String> { ... }

    // 結構化 API — Agent loop 呼叫入口
    // 關鍵邏輯: 工具不支援原生 → 自動注入系統提示
    async fn chat(&self, request: ChatRequest<'_>,
                   model: &str, temperature: f64) -> Result<ChatResponse> {
        if let Some(tools) = request.tools {
            if !tools.is_empty() && !self.supports_native_tools() {
                // 自動回退到系統提示注入
                let instructions = match self.convert_tools(tools) {
                    ToolsPayload::PromptGuided { instructions } => instructions,
                    payload => bail!("Non-prompt-guided payload while native_tools=false"),
                };
                // 找到或建立 system 訊息，附加工具說明
            }
        }
    }

    fn supports_native_tools(&self) -> bool { self.capabilities().native_tool_calling }
    fn supports_vision(&self) -> bool { self.capabilities().vision }
    async fn warmup(&self) -> Result<()> { Ok(()) }  // HTTP 連線池預熱
}
```

**ChatResponse — 包含 reasoning_content**:

```rust
// src/providers/traits.rs:60-73
pub struct ChatResponse {
    pub text: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<TokenUsage>,
    pub reasoning_content: Option<String>,  // DeepSeek-R1, Kimi K2.5, GLM-4.7
    // 某些 provider 會拒絕省略 reasoning_content 的工具呼叫歷史
}
```

**StreamChunk — 串流回應**:

```rust
// src/providers/traits.rs:120-163
pub struct StreamChunk {
    pub delta: String,
    pub is_final: bool,
    pub token_count: usize,
}

impl StreamChunk {
    pub fn with_token_estimate(mut self) -> Self {
        self.token_count = self.delta.len().div_ceil(4);  // ~4 chars/token 近似
        self
    }
}
```

**ProviderCapabilityError — 結構化能力錯誤**:

```rust
// src/providers/traits.rs:213-219
#[derive(Debug, Clone, thiserror::Error)]
#[error("provider_capability_error provider={provider} capability={capability} message={message}")]
pub struct ProviderCapabilityError {
    pub provider: String,
    pub capability: String,
    pub message: String,
}
```

### 9.3 RouterProvider — hint 路由

```rust
pub struct RouterProvider {
    routes: HashMap<String, (usize, String)>,  // hint → (provider_index, model)
    providers: Vec<(String, Box<dyn Provider>)>,
    default_index: usize,
    default_model: String,
}
```

**Clawtex 實作建議**

clawtex-core 已有 `src/providers/router.rs` (SmartRouter) + `src/providers/rotation.rs`。與 ZeroClaw 的主要差距:

| 功能 | ZeroClaw | clawtex-core | 差距 | 改動量 |
|------|----------|-------------|------|--------|
| 錯誤分類 | 4 層 (HTTP/字串/語義/特殊 429) | 無分類 | **嚴重** | ~80 行 |
| ProviderCapabilities | struct (native_tools, vision) | 無 | **重要** | ~30 行 |
| ToolsPayload | 4 格式 (Gemini/Anthropic/OpenAI/Prompt) | 無 | **重要** | ~50 行 |
| 模型後備鏈 | model_fallbacks HashMap | 無 | **中等** | ~40 行 |
| Retry-After 解析 | 自動解析 + cap 30s | 無 | **中等** | ~30 行 |
| warmup() | HTTP 連線池預熱 | 無 | **低** | ~10 行 |
| reasoning_content | ChatResponse 欄位 | 無 | **重要** | ~20 行 |
| ProviderCapabilityError | 結構化錯誤 | 無 | **中等** | ~15 行 |

Rust skeleton — 錯誤分類:

```rust
// 建議加入 src/providers/mod.rs 或 reliable.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    Retryable,     // 暫時性: 5xx, 429 (非商業), 408, 網路錯誤
    Fatal,         // 永久性: 4xx (非 429/408), 上下文超出, 認證失敗
    QuotaExhausted, // 商業 429: 配額耗盡、方案不支援
}

fn classify_error(err: &anyhow::Error) -> ErrorClass {
    if is_context_window_exceeded(err) { return ErrorClass::Fatal; }

    // Layer 1: reqwest typed HTTP status
    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
        if let Some(status) = reqwest_err.status() {
            let code = status.as_u16();
            return match code {
                429 if is_business_rate_limit(err) => ErrorClass::QuotaExhausted,
                429 | 408 => ErrorClass::Retryable,
                400..=499 => ErrorClass::Fatal,
                500..=599 => ErrorClass::Retryable,
                _ => ErrorClass::Retryable,
            };
        }
    }

    // Layer 2: string fallback
    let msg = err.to_string().to_lowercase();
    if msg.contains("unauthorized") || msg.contains("invalid api key") {
        return ErrorClass::Fatal;
    }
    if msg.contains("model") && msg.contains("not found") {
        return ErrorClass::Fatal;
    }

    ErrorClass::Retryable
}
```

Rust skeleton — ProviderCapabilities:

```rust
// 建議加入 src/providers/mod.rs
#[derive(Debug, Clone, Default)]
pub struct ProviderCapabilities {
    pub native_tool_calling: bool,
    pub vision: bool,
}

// 在 Provider trait 加入:
fn capabilities(&self) -> ProviderCapabilities { ProviderCapabilities::default() }
fn supports_native_tools(&self) -> bool { self.capabilities().native_tool_calling }
fn supports_vision(&self) -> bool { self.capabilities().vision }
```

---

## 10. 記憶系統 -- 7 種後端與向量嵌入

### 10.1 Memory Trait — 完整定義與資料模型

**檔案**: `src/memory/traits.rs` (146 行)

**MemoryEntry — 記憶條目**:

```rust
// src/memory/traits.rs:5-14
#[derive(Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,              // UUID v4
    pub key: String,             // 唯一識別鍵
    pub content: String,         // 記憶內容
    pub category: MemoryCategory,
    pub timestamp: String,       // ISO 8601 / RFC-3339
    pub session_id: Option<String>,
    pub score: Option<f64>,      // 語義搜尋相關度分數 (0.0-1.0)
}
```

**MemoryCategory — 4 種分類**:

```rust
// src/memory/traits.rs:30-41
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    Core,              // 長期事實、偏好、決策
    Daily,             // 每日會話日誌
    Conversation,      // 對話上下文
    Custom(String),    // 使用者自定義分類
}
```

**Memory Trait — 8 個方法 (全必須)**:

```rust
// src/memory/traits.rs:55-95
#[async_trait]
pub trait Memory: Send + Sync {
    fn name(&self) -> &str;

    // 儲存: 鍵衝突時更新 (upsert 語義)
    async fn store(&self, key: &str, content: &str,
                    category: MemoryCategory, session_id: Option<&str>) -> Result<()>;

    // 召回: 語義/關鍵字搜尋，回傳排序結果 (score desc)
    // score 欄位由後端填充 — SQLite: FTS5 rank, Qdrant: 餘弦相似度
    async fn recall(&self, query: &str, limit: usize,
                     session_id: Option<&str>) -> Result<Vec<MemoryEntry>>;

    // 精確取得: key 完全匹配
    async fn get(&self, key: &str) -> Result<Option<MemoryEntry>>;

    // 列表: 可選分類+會話過濾
    async fn list(&self, category: Option<&MemoryCategory>,
                   session_id: Option<&str>) -> Result<Vec<MemoryEntry>>;

    // 刪除: 回傳是否存在該鍵
    async fn forget(&self, key: &str) -> Result<bool>;

    // 計數
    async fn count(&self) -> Result<usize>;

    // 健康檢查
    async fn health_check(&self) -> bool;
}
```

**記憶系統在 Agent Loop 中的整合**:

```
Agent Loop × Memory 互動流程
  │
  ├── 進入迴圈前:
  │     └── build_context(memory, user_msg, min_score=0.3) → 注入 [Memory context]
  │
  ├── 工具呼叫中:
  │     ├── memory_store 工具 → memory.store(key, content, category, session_id)
  │     ├── memory_recall 工具 → memory.recall(query, limit, session_id)
  │     └── memory_forget 工具 → memory.forget(key)
  │
  ├── 迴圈結束:
  │     └── 自動儲存 (>= 20 chars 的用戶訊息 + 助手回覆)
  │         key = "{prefix}_{uuid_v4}" → category = Conversation
  │
  └── 壓縮時:
        └── auto_compact_history() 可儲存壓縮摘要到 Memory
```

### 10.2 後端列表

| 後端 | 檔案 | 特點 |
|------|------|------|
| SQLite | `sqlite.rs` | 預設，內建全文搜尋 |
| Lucid | `lucid.rs` | SQLite + 增強語義 |
| Markdown | `markdown.rs` | 純文字檔案 |
| PostgreSQL | `postgres.rs` | feature flag |
| Qdrant | `qdrant.rs` | 向量搜尋 |
| None | `none.rs` | 無持久化 |

### 10.3 額外記憶功能

| 功能 | 檔案 | 說明 |
|------|------|------|
| EmbeddingProvider trait | `embeddings.rs` | 向量嵌入抽象 (OpenAI/Ollama/本地) |
| VectorMemory | `vector.rs` | 向量記憶合併 (Qdrant + embedding) |
| ResponseCache | `response_cache.rs` | LRU 回應快取 (避免重複 LLM 呼叫) |
| Snapshot | `snapshot.rs` | 記憶快照 (匯出/匯入 JSON) |
| Hygiene | `hygiene.rs` | 記憶清理 (過期刪除、重複合併) |
| Chunker | `chunker.rs` | 長文本分塊 (用於向量嵌入) |
| CLI | `cli.rs` | `zeroclaw memory` 子命令 (list/get/store/forget) |

### 10.4 記憶系統資料流

```
                    ┌──────────────────┐
                    │  Agent / Tools    │
                    └────────┬─────────┘
                             │ store/recall/forget
                             ▼
                    ┌──────────────────┐
                    │   Memory Trait    │
                    └────────┬─────────┘
                             │
        ┌────────────┬───────┼────────┬──────────────┐
        ▼            ▼       ▼        ▼              ▼
   ┌─────────┐ ┌────────┐ ┌──────┐ ┌───────────┐ ┌────────┐
   │ SQLite  │ │ Lucid  │ │ MD   │ │PostgreSQL │ │ Qdrant │
   │ FTS5    │ │ +語義  │ │ 純文字│ │ feature   │ │ 向量   │
   └────┬────┘ └────┬───┘ └──┬───┘ └─────┬─────┘ └────┬───┘
        │           │        │           │            │
        ▼           ▼        ▼           ▼            ▼
   core.db    core.db   workspace/  PostgreSQL    Qdrant
   (內建FTS)  (增強語義) memory/*.md  (遠端)       (遠端)

   Embedding Layer (可選):
   ┌──────────────┐
   │ EmbeddingProvider trait │
   ├──────────────┤
   │ OpenAI       │ → text-embedding-3-small
   │ Ollama       │ → nomic-embed-text
   │ Local        │ → 本地模型
   └──────┬───────┘
          │ embed(text) → Vec<f32>
          ▼
   ┌──────────────┐
   │ VectorMemory │ → Qdrant + 嵌入向量
   │ .recall()    │ → 餘弦相似度搜尋
   └──────────────┘
```

**Clawtex 實作建議**

clawtex-core 已有 `~/.clawtex/memory.db` (SQLite)，`memories` table 包含 key/content/category/session_id/created_at。

| 功能 | clawtex 現況 | 差距 | 改動建議 | 複雜度 |
|------|-------------|------|---------|--------|
| MemoryEntry.score | **無** | recall 無相關度排序 | 加入 FTS5 rank 分數到回傳 | ~20 行 |
| MemoryCategory enum | 字串 category | 無型別安全 | 新增 enum + serde(rename_all) | ~30 行 |
| EmbeddingProvider | **無** | 無向量搜尋 | 新增 trait + OpenAI/Ollama 實作 | ~200 行 |
| VectorMemory | **無** | 僅關鍵字搜尋 | 整合 Qdrant 或 SQLite vec 擴展 | ~300 行 |
| ResponseCache | **無** | 重複 LLM 呼叫 | 新增 LRU 快取 | ~80 行 |
| Hygiene | **無** | 記憶堆積 | 新增過期清理 + 重複合併 | ~100 行 |
| min_relevance_score | **無** | 不相關記憶注入 | build_context 加入分數過濾 | ~10 行 |

Rust skeleton — 帶分數的 recall:

```rust
// 建議修改 src/memory.rs 的 recall 方法
pub async fn recall(&self, query: &str, limit: usize, session_id: Option<&str>) -> Result<Vec<MemoryEntry>> {
    let sql = "SELECT key, content, category, session_id, created_at,
               rank AS score
               FROM memories WHERE memories MATCH ?1
               ORDER BY rank
               LIMIT ?2";
    // rank 是 FTS5 內建的相關度分數 (負數，越小越相關)
    // 正規化: score = 1.0 / (1.0 + rank.abs())
}
```

---

## 11. 差距對比總覽與實作優先序

### 11.1 安全差距 (最高優先)

| 功能 | ZeroClaw | clawtex-core | 差距 | 改動量 |
|------|----------|-------------|------|--------|
| PromptGuard | 6 類別偵測 + 分數系統 | **無** | **嚴重** | ~300 行 |
| LeakDetector | 7 類別 + Shannon 熵 | 僅 scrub_credentials | **重要** | ~350 行 |
| 多級 E-Stop | 4 級 (KillAll/Network/Domain/Tool) | 單一 AtomicBool | **重要** | ~250 行 |
| AuditLogger | 結構化 + 輪替 | **無** | **重要** | ~200 行 |
| Sandbox trait | 4 後端 | **無** | **嚴重** | ~100 行 |
| OTP 恢復 | TOTP + HMAC-SHA1 | **無** | **中等** | ~200 行 |
| DomainMatcher | 萬用字元 + 4 分類 | **無** | **中等** | ~150 行 |
| Shell 解析器 | Quote-aware | 基本 split | **重要** | ~80 行 |

### 11.2 架構差距 (高優先)

| 功能 | ZeroClaw | clawtex-core | 差距 | 改動量 |
|------|----------|-------------|------|--------|
| ConversationMessage 三態 | Chat/ToolCalls/Results | 扁平 ChatMessage | **重要** | ~50 行 |
| ProviderCapabilities | 結構化能力宣告 | 無 | **重要** | ~30 行 |
| AgentBuilder | 完整 Builder | 直接建構 | **中等** | ~100 行 |
| Component Supervisor | 指數退避重啟 | **無** | **重要** | ~80 行 |
| 錯誤分類 (Provider) | Retryable/Fatal | 無分類 | **重要** | ~50 行 |

### 11.3 通訊差距 (中優先)

| 功能 | ZeroClaw | clawtex-core | 差距 | 改動量 |
|------|----------|-------------|------|--------|
| Channel trait 抽象 | 20+ 實作 | 僅 Telegram 硬編碼 | **重要** | ~100 行 trait |
| Draft 更新系統 | send/update/finalize/cancel | 簡單 edit | **中等** | ~80 行 |
| SSE 事件串流 | 內建 | **無** | **中等** | ~150 行 |
| WebSocket 對話 | 內建 | **無** | **低** | ~100 行 |

### 11.4 工作流差距 (中優先)

| 功能 | ZeroClaw | clawtex-core | 差距 | 改動量 |
|------|----------|-------------|------|--------|
| 條件系統 | JSON path + 6 運算子 | **無** | **重要** | ~200 行 |
| 冷卻時間 | cooldown_secs | **無** | **低** | ~30 行 |
| 並行控制 | max_concurrent | **無** | **低** | ~50 行 |
| 信任階段 | ampersona Gates | **無** | **低** | ~300 行 + 外部 crate |

### 11.5 clawtex-core 獨有優勢

| 功能 | clawtex 獨有 | 說明 | ZeroClaw 無此設計的原因 |
|------|-------------|------|------------------------|
| Hands 鏈式串接 | `chaining_prompt` | SOP 完成後自動觸發下一個工作流 | SOP 採用獨立觸發器模式 |
| Cluster 系統 | Hub + Worker 分散式 | 跨機器負載分配 | ZeroClaw 僅單機設計 |
| Twitter 工具 | OAuth + 瀏覽器後備 | 社群自動化 | ZeroClaw 無社群工具 |
| SaaS Pipeline | product_spec → code_gen → deploy | 端到端 SaaS 生成 | ZeroClaw 無生成式業務 |
| Revenue Tracker | SQLite 收入追蹤 | 營收可視化 | ZeroClaw 僅有 CostTracker |
| Smart Routing | simple/medium/complex 分級 + Classifier | LLM 自動分類路由 | ZeroClaw hint 是手動標籤 |
| Skeleton-of-Thought | 並行內容生成 | 長文本加速 | ZeroClaw 無此最佳化 |
| ChatGPT Backend | Codex CLI 子程序 | ChatGPT Plus 整合 | ZeroClaw 無 Codex 整合 |
| Self-Evolution | self_evolve + review_agents | Felix 風格 1% 每日改善 | ZeroClaw 無自我進化系統 |
| MCP Client | JSON-RPC 2.0 over stdio | 外部工具擴展 | ZeroClaw 有 integrations/ 但不同架構 |
| Key Pool | 多 API key 池 + 公平輪替 | 高吞吐配額管理 | ZeroClaw 僅在 ReliableProvider 內輪替 |
| Blog/PDF | blog_publish + pdf_export | 內容發布 | ZeroClaw 無出版工具 |

### 11.6 建議實作優先序

**Phase 1 (安全加固, 1-2 天)**:
1. PromptGuard (~300 行) — 防止提示注入
2. LeakDetector (~350 行) — 升級 scrub_credentials
3. 多級 E-Stop (~250 行) — 替換 AtomicBool

**Phase 2 (架構改善, 1-2 天)**:
4. Channel trait 抽象 (~100 行) — 為多通道奠基
5. ConversationMessage 三態 (~50 行) — 改善工具呼叫歷史
6. 錯誤分類 (~50 行) — Provider 重試優化

**Phase 3 (功能擴展, 2-3 天)**:
7. SSE 事件串流 (~150 行) — 即時監控
8. 條件系統 (~200 行) — Hands 執行條件
9. AuditLogger (~200 行) — 安全審計
10. SkillForge Scout (~200 行) — 技能自動發現

### 11.7 架構設計模式對比

**Agent Loop 模式比較**:

| 面向 | ZeroClaw | clawtex-core |
|------|----------|-------------|
| 進入點 | `agent/loop_.rs::run_tool_call_loop()` | `agent_runtime.rs::run_agent_loop()` |
| 工具解析 | 內嵌 (4 格式支援) | `dispatcher.rs` (3 格式: Native/XML/FunctionTag) |
| 歷史管理 | `ConversationMessage` 三態 enum | 扁平 `ChatMessage` |
| 壓縮 | LLM 摘要 + 截斷回退 | `context_optimizer.rs` 壓縮器 |
| 迴圈偵測 | `tool_call_signature()` + `HashSet` | `detect_loop()` 相似度偵測 |
| 記憶注入 | `build_context()` 帶分數過濾 | `recall_memories()` 無分數過濾 |
| 串流顯示 | Draft 更新系統 (send/update/finalize) | `edit_message_text` 直接編輯 |
| 進度顯示 | `truncate_tool_args_for_progress()` | 無獨立進度函式 |
| 憑證清洗 | `scrub_credentials()` + LeakDetector | `scrub_credentials()` (較簡單) |
| E-Stop | 每輪檢查 `EstopState` | 每輪檢查 `AtomicBool` |

**Security Pipeline 對比**:

```
ZeroClaw Security Pipeline:
  Input → PromptGuard → LeakDetector → E-Stop → PairingGuard
       → SecurityPolicy (autonomy + allowlist + risk + rate limit)
       → Sandbox (Landlock/Bubblewrap/Firejail/Docker/Noop)
       → Execution
       → AuditLogger → LeakDetector (output) → scrub_credentials

clawtex-core Security Pipeline:
  Input → E-Stop (AtomicBool)
       → SecurityConfig (allowed_commands + forbidden_paths + workspace)
       → Approval gate (Telegram)
       → Execution
       → scrub_credentials (output)

差距摘要:
  - clawtex 缺少: PromptGuard, LeakDetector 輸入側, 多級 E-Stop,
                   AuditLogger, Sandbox, DomainMatcher, OTP 恢復
  - clawtex 獨有: Telegram approval gate (ZeroClaw 的 approval 是通用的)
```

**工作流引擎比較**:

```
ZeroClaw SOP Engine:
  TOML 定義 → SopEngine.reload()
  觸發: 5 種 (MQTT/Webhook/Cron/Peripheral/Manual)
  狀態機: Pending → Running → WaitApproval → Completed/Failed/Cancelled
  調度: dispatch_sop_event() — 批次鎖定 (恰好 2 次)
  條件: JSON path + 6 運算子
  度量: SopMetricsCollector (環形緩衝 1000 筆)
  信任: ampersona Gates (trust-phase 轉換)

clawtex-core Hands Engine:
  TOML 定義 → hands/runner.rs
  觸發: 手動 + Cron (HTTP API)
  執行: 線性多階段 (無顯式狀態機)
  條件: 無 (僅 settings 鍵值)
  特有: chaining_prompt (SOP 無)
  特有: settings HashMap<String, String>
  特有: system_prompt per phase
```

---

## 12. 附錄：關鍵檔案路徑索引

| 模組 | ZeroClaw 路徑 | 行數 |
|------|--------------|------|
| SecurityPolicy | `src/security/policy.rs` | 800+ |
| LeakDetector | `src/security/leak_detector.rs` | 538 |
| PromptGuard | `src/security/prompt_guard.rs` | 361 |
| EstopManager | `src/security/estop.rs` | 423 |
| AuditLogger | `src/security/audit.rs` | 424 |
| SecretStore | `src/security/secrets.rs` | 852 |
| OtpValidator | `src/security/otp.rs` | 319 |
| DomainMatcher | `src/security/domain_matcher.rs` | 260 |
| Sandbox trait | `src/security/traits.rs` | 119 |
| SOP Engine | `src/sop/engine.rs` | 800+ |
| SOP Types | `src/sop/types.rs` | 471 |
| SOP Condition | `src/sop/condition.rs` | 452 |
| SOP Dispatch | `src/sop/dispatch.rs` | 730 |
| SOP Metrics | `src/sop/metrics.rs` | 800+ |
| SOP Gates | `src/sop/gates.rs` | 747 |
| Tool trait | `src/tools/traits.rs` | 122 |
| Channel trait | `src/channels/traits.rs` | 270 |
| SkillForge | `src/skillforge/mod.rs` | 256 |
| Scout | `src/skillforge/scout.rs` | 340 |
| Evaluator | `src/skillforge/evaluate.rs` | 273 |
| Integrator | `src/skillforge/integrate.rs` | 253 |
| Gateway | `src/gateway/mod.rs` | 1200+ |
| SSE | `src/gateway/sse.rs` | 159 |
| WebSocket | `src/gateway/ws.rs` | 186 |
| Gateway API | `src/gateway/api.rs` | 500+ |
| Agent | `src/agent/agent.rs` | 300+ |
| Agent Loop | `src/agent/loop_.rs` | 400+ |
| Dispatcher | `src/agent/dispatcher.rs` | 300+ |
| ReliableProvider | `src/providers/reliable.rs` | 1500+ |
| Provider trait | `src/providers/traits.rs` | 300+ |
| Router | `src/providers/router.rs` | 200+ |
| Memory trait | `src/memory/traits.rs` | 200+ |

所有檔案完整路徑根:
`C:\Users\m4932\Desktop\adreanalai\LLM-Cluster-Project\references\zeroclaw\src\`
