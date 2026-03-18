# Clawtex 8 機 AI 集群自動化部署系統設計

> 文件建立日期: 2026-03-05
> 作者: DevOps 工程設計
> 狀態: 設計完成，待實施

---

## 目錄

- [1. 集群拓撲與機器清單](#1-集群拓撲與機器清單)
- [2. 跨平台編譯策略](#2-跨平台編譯策略)
- [3. 部署方式決策](#3-部署方式決策)
- [4. 配置管理系統](#4-配置管理系統)
- [5. 滾動更新與回滾](#5-滾動更新與回滾)
- [6. Ollama 模型管理](#6-ollama-模型管理)
- [7. 自動化腳本完整實作](#7-自動化腳本完整實作)
- [8. Infrastructure as Code 選型](#8-infrastructure-as-code-選型)
- [9. 部署流程圖](#9-部署流程圖)
- [10. 監控與告警](#10-監控與告警)
- [11. 實施計劃](#11-實施計劃)

---

## 1. 集群拓撲與機器清單

### 1.1 現有機器

| 名稱 | 角色 | OS | 架構 | IP | 特殊能力 |
|------|------|-----|------|-----|---------|
| **Z13** | Hub (主控) | Windows 11 | x86_64 | 10.0.1.1 | NPU (50 TOPS), GPU 8060S, 64GB RAM |
| **M1** | Worker | macOS | aarch64 | 10.0.2.1 (Tailscale) | Apple Silicon 推理 |
| **Ayaneo** | Worker | Windows 11 | x86_64 | 10.0.1.4 | Edge GPU 推理 |
| **Acer** | Worker + 儲存 | Windows/Linux | x86_64 | 10.0.1.3 | 備份推理 + 大硬碟 |
| **機器5-8** | Worker | TBD | TBD | TBD | 未來擴展 |

### 1.2 角色定義

```
Hub (Z13):
  - 運行完整 clawtex-core (Telegram bot + HTTP API + Hands + Gateway)
  - 持有所有 API keys 和 secrets
  - ClusterRegistry 主節點
  - Cron 調度器
  - 成本/收入追蹤器
  - SoT 骨架生成協調器

Worker (M1/Ayaneo/Acer/...):
  - 運行 Ollama (模型推理)
  - 可選: 運行 clawtex-core worker 模式 (接收委派任務)
  - 不持有 API keys (透過 Hub 代理)
  - 定期向 Hub 回報健康狀態
```

### 1.3 網路架構

```
                    ┌──────────────┐
                    │   Internet   │
                    └──────┬───────┘
                           │
                    ┌──────┴───────┐
                    │  Tailscale   │
                    │  VPN Mesh    │
                    └──────┬───────┘
                           │
          ┌────────────────┼────────────────┐
          │                │                │
   ┌──────┴──────┐  ┌─────┴──────┐  ┌──────┴──────┐
   │   Z13 Hub   │  │  M1 Worker │  │ Ayaneo Wkr  │
   │ :7878 HTTP  │  │ :11434     │  │ :11434      │
   │ :11434 LLM  │  │ Ollama     │  │ Ollama      │
   │ Telegram Bot│  └────────────┘  └─────────────┘
   └──────┬──────┘
          │ LAN
   ┌──────┴──────┐
   │ Acer Worker │
   │ :11434      │
   │ Ollama      │
   └─────────────┘
```

---

## 2. 跨平台編譯策略

### 2.1 目標平台

| Target Triple | 用途 | 機器 |
|---------------|------|------|
| `x86_64-pc-windows-msvc` | Z13, Ayaneo, Acer (Windows) | 主要 |
| `aarch64-apple-darwin` | M1 Mac | 需要 |
| `x86_64-unknown-linux-gnu` | Acer (Linux), Docker, 未來 VPS | 需要 |
| `aarch64-unknown-linux-gnu` | 未來 ARM Linux (RPi 等) | 備用 |

### 2.2 編譯方案: 本地交叉編譯 + SSH 推送

**為什麼不用 GitHub Actions:**
- 這是私有集群，不需要公開 CI
- Z13 的 16C/32T CPU 編譯速度夠快
- 減少對外部服務的依賴
- 離線也能部署

**為什麼不用 Docker 交叉編譯:**
- Windows binary 不能在 Linux Docker 裡交叉編譯 MSVC target
- macOS binary 不能在 Docker 裡編譯 (Apple SDK 授權限制)

**最佳方案: 混合策略**

```bash
# Z13 (Windows) 本地編譯 Windows binary
cargo build --release  # x86_64-pc-windows-msvc

# M1 上 SSH 執行遠端編譯 (最可靠的 macOS binary 來源)
ssh m1 "cd /tmp/clawtex-core && cargo build --release"

# Linux binary: 用 cross 工具在 Z13 上交叉編譯
cross build --release --target x86_64-unknown-linux-gnu
```

### 2.3 依賴處理

clawtex-core 的依賴分析 (基於 Cargo.toml):

```
rusqlite: features=["bundled"]  → 自帶 SQLite C 源碼，不依賴系統 libsqlite3
reqwest:  features=["json","stream"] → 使用 rustls (Rust TLS)，不依賴 OpenSSL
chacha20poly1305: 純 Rust
axum, tokio: 純 Rust
```

**結論: clawtex-core 是完全自包含的 Rust binary，零 native 依賴。**
這大幅簡化了交叉編譯 -- 不需要處理 OpenSSL 或其他 C 庫。

唯一的運行時依賴是外部程式:
- `python3` + `playwright` (browser 工具)
- `pandoc` (PDF export)
- `ollama` (LLM 推理)

這些由各機器本地安裝，不隨 binary 打包。

### 2.4 cross 工具安裝

```bash
# 在 Z13 上安裝 cross (用 Docker 做 Linux 交叉編譯)
cargo install cross --git https://github.com/cross-rs/cross

# 用法
cross build --release --target x86_64-unknown-linux-gnu
cross build --release --target aarch64-unknown-linux-gnu
```

---

## 3. 部署方式決策

### 3.1 方案比較

| 方案 | 優點 | 缺點 | 適用場景 |
|------|------|------|---------|
| **A: SSH + SCP** | 最簡單、最快、零依賴 | 手動管理版本 | 8 台規模，首選 |
| **B: Docker** | 環境一致、好回滾 | 需要每台裝 Docker、Windows 體驗差 | Linux 伺服器群 |
| **C: Git pull + 本地編譯** | 保證原始碼一致 | 每台都要 Rust toolchain、編譯慢 | 開發環境 |

### 3.2 決定: 方案 A (SSH + SCP) 為主，方案 B (Docker) 為輔

**理由:**

1. **8 台規模不需要 Kubernetes/Docker Swarm** -- 過度工程化
2. **Windows 機器佔多數** -- Docker Desktop on Windows 性能差、授權問題
3. **clawtex-core 是靜態連結 binary** -- SCP 一個檔案就完事
4. **M1 Mac** -- Docker 在 ARM Mac 上跑 x86 有 Rosetta 開銷
5. **Acer 如果裝 Linux** -- 可以用 Docker 作為備選方案

### 3.3 部署架構

```
Z13 (Build Machine)
├── cargo build --release              → target/release/clawtex-core.exe (Windows)
├── ssh m1 "cargo build --release"     → 從 M1 拉回 binary (macOS)
└── cross build --release --target x86_64-unknown-linux-gnu → (Linux)

然後:
├── scp clawtex-core.exe → Ayaneo:/opt/clawtex/
├── scp clawtex-core.exe → Acer:/opt/clawtex/   (若 Windows)
├── scp clawtex-core   → Acer:/opt/clawtex/     (若 Linux)
└── scp clawtex-core   → M1:/opt/clawtex/       (macOS)
```

---

## 4. 配置管理系統

### 4.1 配置分層架構

```
配置層次:
├── cluster.toml          ← 集群級 (所有節點共用，Hub 生成)
├── agents.toml           ← 節點級 (每台不同，Hub 和 Worker 有差異)
├── hands/*.toml          ← Hub 專屬 (Hands 只在 Hub 執行)
└── .env                  ← 節點級 secrets (每台不同)
```

### 4.2 Hub vs Worker 配置差異

**Hub (Z13) 的 agents.toml:**

```toml
[core]
host = "0.0.0.0"
port = 7878
role = "hub"               # 標識為 Hub
db_path = "~/.clawtex/core.db"

[cluster]
# Hub 知道所有 Worker 的位置
workers = [
    { name = "m1",     host = "10.0.2.1", port = 11434 },
    { name = "ayaneo", host = "10.0.1.4", port = 11434 },
    { name = "acer",   host = "10.0.1.3", port = 11434 },
]

# Hub 有完整的 provider 列表（包含遠端 Ollama）
[providers.ollama-local]
type = "ollama"
url = "http://localhost:11434"
default_model = "qwen3:8b"

[providers.ollama-m1]
type = "ollama"
url = "http://10.0.2.1:11434"
default_model = "llama3.1:8b"

[providers.ollama-ayaneo]
type = "ollama"
url = "http://10.0.1.4:11434"
default_model = "qwen2.5:7b"

[providers.ollama-acer]
type = "ollama"
url = "http://10.0.1.3:11434"
default_model = "llama3.1:8b"

[providers.lmstudio]
type = "openai_compat"
url = "http://localhost:1234"

# Cloud providers (API keys only on Hub)
[providers.anthropic]
type = "anthropic"
api_key = "enc2:..."
default_model = "claude-sonnet-4-6"

[providers.gemini]
type = "gemini"
api_key = "enc2:..."

[telegram]
bot_token = "enc2:..."
allowed_users = [12345678]

[stripe]
secret_key = "enc2:..."

[search]
serper_api_key = "enc2:..."

# ... (所有 agents, hands 配置)
```

**Worker (Ayaneo) 的 agents.toml:**

```toml
[core]
host = "0.0.0.0"
port = 7878
role = "worker"              # 標識為 Worker
hub_url = "http://10.0.1.1:7878"  # 向 Hub 註冊用

[providers.ollama]
type = "ollama"
url = "http://localhost:11434"
default_model = "qwen2.5:7b"

# Worker 不需要:
# - telegram 配置
# - cloud API keys
# - hands 定義
# - stripe/render 配置
```

### 4.3 配置模板系統

在 Z13 上維護配置模板:

```
~/.clawtex/deploy/
├── templates/
│   ├── hub.agents.toml.template
│   ├── worker.agents.toml.template
│   └── .env.template
├── nodes/
│   ├── z13.env           # 節點特定變數
│   ├── m1.env
│   ├── ayaneo.env
│   └── acer.env
└── inventory.toml        # 集群清單
```

### 4.4 inventory.toml (集群清單)

```toml
# ~/.clawtex/deploy/inventory.toml
# 集群節點清單 — deploy.sh 讀取此檔

[hub]
name = "z13"
host = "localhost"
ssh = ""                          # Hub 不需要 SSH
os = "windows"
arch = "x86_64"
ollama_models = ["qwen3:8b", "qwen3-coder:30b"]

[[workers]]
name = "m1"
host = "10.0.2.1"
ssh = "mark@10.0.2.1"
os = "macos"
arch = "aarch64"
ollama_models = ["llama3.1:8b", "qwen2.5:14b"]
deploy_path = "/opt/clawtex"
ollama_port = 11434

[[workers]]
name = "ayaneo"
host = "10.0.1.4"
ssh = "user@10.0.1.4"
os = "windows"
arch = "x86_64"
ollama_models = ["qwen2.5:7b"]
deploy_path = "C:/clawtex"
ollama_port = 11434

[[workers]]
name = "acer"
host = "10.0.1.3"
ssh = "user@10.0.1.3"
os = "windows"
arch = "x86_64"
ollama_models = ["llama3.1:8b", "qwen2.5:13b"]
deploy_path = "C:/clawtex"
ollama_port = 11434
```

### 4.5 Secret 管理策略

```
原則:
1. API keys 只存在 Hub (Z13) 上
2. Hub 的 agents.toml 使用 enc2: 加密格式 (ChaCha20-Poly1305)
3. Worker 不需要 API keys — 它們只跑 Ollama
4. .env 檔案不進 git
5. 加密金鑰存在 ~/.clawtex/secret.key (自動生成，不進 git)

如果 Worker 需要 cloud API (未來):
  → Hub 代理模式: Worker 把請求送到 Hub，Hub 帶上 API key 轉發
  → 不在 Worker 上存任何 key
```

### 4.6 動態配置更新

目前 clawtex-core 啟動時讀取 agents.toml，需要重啟才能更新。

**Phase 1 (目前可行):** 配置更新時自動重啟服務
```bash
# deploy.sh 更新配置後會自動 restart
scp agents.toml worker:/opt/clawtex/agents.toml
ssh worker "systemctl restart clawtex-core"  # Linux
ssh worker "taskkill /f /im clawtex-core.exe && start clawtex-core.exe"  # Windows
```

**Phase 2 (未來):** 熱載入配置
```rust
// 需要在 clawtex-core 中加入:
// 1. SIGHUP handler (Linux/Mac) 或 named pipe (Windows) 觸發重新載入
// 2. 監聽 agents.toml 的 fs::watch
// 3. AtomicSwap 更新 provider router
```

---

## 5. 滾動更新與回滾

### 5.1 版本管理

每次 build 都標記版本:

```bash
# 用 git commit hash + 時間戳作為版本
VERSION=$(git rev-parse --short HEAD)
BUILD_TIME=$(date +%Y%m%d-%H%M%S)
RELEASE_TAG="${VERSION}-${BUILD_TIME}"

# 部署目錄結構 (每台 Worker 上)
/opt/clawtex/
├── bin/
│   ├── clawtex-core                    → symlink → releases/abc1234-20260305-143000/clawtex-core
│   └── clawtex-core.prev               → symlink → releases/def5678-20260304-120000/clawtex-core
├── releases/
│   ├── abc1234-20260305-143000/
│   │   └── clawtex-core
│   ├── def5678-20260304-120000/
│   │   └── clawtex-core
│   └── (保留最近 5 個版本)
├── config/
│   └── agents.toml
└── data/
    └── core.db
```

Windows 上等效結構:
```
C:\clawtex\
├── bin\
│   ├── clawtex-core.exe               → 當前版本
│   └── clawtex-core.prev.exe          → 上一版本
├── releases\
│   ├── abc1234-20260305-143000\
│   │   └── clawtex-core.exe
│   └── ...
├── config\
│   └── agents.toml
└── data\
    └── core.db
```

### 5.2 滾動更新流程

```
更新順序: Worker 們 → Hub (最後)

  ┌─────────────────────────────────────────────────┐
  │              滾動更新流程 (deploy.sh)              │
  ├─────────────────────────────────────────────────┤
  │                                                 │
  │  1. 編譯所有目標平台的 binary                      │
  │     ↓                                           │
  │  2. 對每台 Worker (按順序):                       │
  │     ├── 2a. 健康檢查 (確認節點線上)                │
  │     ├── 2b. 上傳新 binary 到 releases/           │
  │     ├── 2c. 停止舊服務                            │
  │     ├── 2d. 更新 symlink/rename                  │
  │     ├── 2e. 啟動新服務                            │
  │     ├── 2f. 等待 10 秒                           │
  │     ├── 2g. 健康檢查新服務                        │
  │     ├── 2h. 通過 → 繼續下一台                    │
  │     └── 2i. 失敗 → 回滾此台，中止更新             │
  │     ↓                                           │
  │  3. 所有 Worker 成功 → 更新 Hub (Z13)             │
  │     ↓                                           │
  │  4. 發送 Telegram 通知: 更新完成/失敗              │
  │                                                 │
  └─────────────────────────────────────────────────┘
```

### 5.3 回滾機制

```bash
# 回滾單台 Worker
./rollback.sh ayaneo

# 回滾流程:
# 1. 停止服務
# 2. 將 symlink 指向 .prev 版本
# 3. 啟動服務
# 4. 健康檢查
```

---

## 6. Ollama 模型管理

### 6.1 模型版本對齊策略

```toml
# ~/.clawtex/deploy/models.toml
# 定義每台機器應該有哪些模型

[models]
# 共用模型 (所有節點都要有)
common = ["qwen2.5:7b"]

# 節點特定模型
[models.z13]
extra = ["qwen3:8b", "qwen3-coder:30b", "llama3.1:8b"]

[models.m1]
extra = ["llama3.1:8b", "qwen2.5:14b"]

[models.ayaneo]
extra = []  # 記憶體小，只跑 common

[models.acer]
extra = ["llama3.1:8b", "qwen2.5:13b"]
```

### 6.2 磁碟空間管理

```bash
# 檢查各節點磁碟空間
# Ollama 模型典型大小:
#   qwen2.5:7b   ~4.7GB
#   llama3.1:8b  ~4.7GB
#   qwen2.5:14b  ~9.0GB
#   qwen3:8b     ~5.0GB

# 保守估計每台需要 20GB 磁碟空間給模型
# update-models.sh 會先檢查磁碟空間再拉模型
```

### 6.3 模型更新流程

```
update-models.sh 流程:

  1. 讀取 models.toml
  2. 對每台節點:
     ├── SSH 檢查 ollama list (已有模型)
     ├── 比對 models.toml (應有模型)
     ├── 缺少的 → ollama pull
     ├── 多餘的 → 提示 (不自動刪除，避免誤刪)
     └── 檢查磁碟空間 (< 10GB 警告)
  3. 驗證所有節點模型一致
  4. 輸出報告
```

---

## 7. 自動化腳本完整實作

所有腳本放在 `~/.clawtex/deploy/scripts/` 或 repo 的 `deploy/` 目錄。

### 7.1 deploy/inventory.sh (共用函數庫)

```bash
#!/usr/bin/env bash
# deploy/inventory.sh — 集群清單與共用函數
# 所有其他腳本 source 此檔

set -euo pipefail

# ══════════════════════════════════════════════════════════════════
# 集群節點定義
# ══════════════════════════════════════════════════════════════════

# 格式: NAME|SSH|OS|ARCH|DEPLOY_PATH|OLLAMA_PORT|ROLE
NODES=(
    "z13||windows|x86_64|C:/clawtex|11434|hub"
    "m1|mark@10.0.2.1|macos|aarch64|/opt/clawtex|11434|worker"
    "ayaneo|user@10.0.1.4|windows|x86_64|C:/clawtex|11434|worker"
    "acer|user@10.0.1.3|windows|x86_64|C:/clawtex|11434|worker"
)

# Ollama API endpoints (用於健康檢查)
declare -A OLLAMA_URLS=(
    [z13]="http://localhost:11434"
    [m1]="http://10.0.2.1:11434"
    [ayaneo]="http://10.0.1.4:11434"
    [acer]="http://10.0.1.3:11434"
)

# clawtex-core HTTP endpoints
declare -A CORE_URLS=(
    [z13]="http://localhost:7878"
    [m1]="http://10.0.2.1:7878"
    [ayaneo]="http://10.0.1.4:7878"
    [acer]="http://10.0.1.3:7878"
)

# ══════════════════════════════════════════════════════════════════
# 顏色輸出
# ══════════════════════════════════════════════════════════════════

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

log_info()  { echo -e "${BLUE}[INFO]${NC} $*"; }
log_ok()    { echo -e "${GREEN}[OK]${NC} $*"; }
log_warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
log_error() { echo -e "${RED}[ERROR]${NC} $*"; }
log_step()  { echo -e "${CYAN}[STEP]${NC} $*"; }

# ══════════════════════════════════════════════════════════════════
# 節點解析函數
# ══════════════════════════════════════════════════════════════════

get_node_field() {
    local node_str="$1"
    local field_num="$2"
    echo "$node_str" | cut -d'|' -f"$field_num"
}

get_name()        { get_node_field "$1" 1; }
get_ssh()         { get_node_field "$1" 2; }
get_os()          { get_node_field "$1" 3; }
get_arch()        { get_node_field "$1" 4; }
get_deploy_path() { get_node_field "$1" 5; }
get_ollama_port() { get_node_field "$1" 6; }
get_role()        { get_node_field "$1" 7; }

# 根據 OS 取得 binary 檔名
get_binary_name() {
    local os="$1"
    if [[ "$os" == "windows" ]]; then
        echo "clawtex-core.exe"
    else
        echo "clawtex-core"
    fi
}

# 根據 OS+ARCH 取得 Rust target triple
get_target() {
    local os="$1"
    local arch="$2"
    case "${os}-${arch}" in
        windows-x86_64)  echo "x86_64-pc-windows-msvc" ;;
        macos-aarch64)   echo "aarch64-apple-darwin" ;;
        linux-x86_64)    echo "x86_64-unknown-linux-gnu" ;;
        linux-aarch64)   echo "aarch64-unknown-linux-gnu" ;;
        *)               echo "unknown" ;;
    esac
}

# 在遠端節點執行命令
run_remote() {
    local ssh_target="$1"
    shift
    if [[ -z "$ssh_target" ]]; then
        # Hub (localhost)
        eval "$@"
    else
        ssh -o ConnectTimeout=10 -o StrictHostKeyChecking=no "$ssh_target" "$@"
    fi
}

# 健康檢查: Ollama API
check_ollama() {
    local url="$1"
    local timeout="${2:-5}"
    curl -sf --max-time "$timeout" "${url}/api/tags" > /dev/null 2>&1
}

# 健康檢查: clawtex-core HTTP API
check_core() {
    local url="$1"
    local timeout="${2:-5}"
    local response
    response=$(curl -sf --max-time "$timeout" "${url}/health" 2>/dev/null) || return 1
    echo "$response" | grep -q '"status":"ok"'
}

# 取得 Workers (排除 Hub)
get_workers() {
    for node in "${NODES[@]}"; do
        local role
        role=$(get_role "$node")
        if [[ "$role" == "worker" ]]; then
            echo "$node"
        fi
    done
}

# 取得 Hub
get_hub() {
    for node in "${NODES[@]}"; do
        local role
        role=$(get_role "$node")
        if [[ "$role" == "hub" ]]; then
            echo "$node"
            return
        fi
    done
}

# Telegram 通知
TELEGRAM_BOT_TOKEN="${TELEGRAM_BOT_TOKEN:-}"
TELEGRAM_CHAT_ID="${TELEGRAM_CHAT_ID:-}"

notify_telegram() {
    local message="$1"
    if [[ -n "$TELEGRAM_BOT_TOKEN" && -n "$TELEGRAM_CHAT_ID" ]]; then
        curl -sf -X POST \
            "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/sendMessage" \
            -d "chat_id=${TELEGRAM_CHAT_ID}" \
            -d "text=${message}" \
            -d "parse_mode=HTML" > /dev/null 2>&1 || true
    fi
}
```

### 7.2 deploy/deploy.sh (一鍵部署)

```bash
#!/usr/bin/env bash
# deploy/deploy.sh — 一鍵部署 clawtex-core 到所有節點
#
# 用法:
#   ./deploy.sh              # 部署到所有節點
#   ./deploy.sh --workers    # 只部署 Workers
#   ./deploy.sh --node m1    # 只部署指定節點
#   ./deploy.sh --build-only # 只編譯，不部署
#   ./deploy.sh --skip-build # 跳過編譯，只部署 (用上次的 binary)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/inventory.sh"

# ── 參數解析 ──────────────────────────────────────────────────────

BUILD_ONLY=false
SKIP_BUILD=false
WORKERS_ONLY=false
TARGET_NODE=""
CLAWTEX_SRC="${CLAWTEX_SRC:-$(cd "${SCRIPT_DIR}/.." && pwd)}"
BUILD_DIR="${CLAWTEX_SRC}/target"
RELEASE_DIR="${SCRIPT_DIR}/releases"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --build-only)  BUILD_ONLY=true; shift ;;
        --skip-build)  SKIP_BUILD=true; shift ;;
        --workers)     WORKERS_ONLY=true; shift ;;
        --node)        TARGET_NODE="$2"; shift 2 ;;
        --help|-h)
            echo "用法: deploy.sh [OPTIONS]"
            echo "  --build-only   只編譯，不部署"
            echo "  --skip-build   跳過編譯 (用上次的 binary)"
            echo "  --workers      只部署 Workers (不更新 Hub)"
            echo "  --node NAME    只部署指定節點"
            exit 0
            ;;
        *) log_error "未知參數: $1"; exit 1 ;;
    esac
done

# ── 版本資訊 ──────────────────────────────────────────────────────

cd "$CLAWTEX_SRC"
GIT_HASH=$(git rev-parse --short HEAD 2>/dev/null || echo "nogit")
BUILD_TIME=$(date +%Y%m%d-%H%M%S)
VERSION="${GIT_HASH}-${BUILD_TIME}"

log_info "Clawtex 部署開始"
log_info "版本: ${VERSION}"
log_info "源碼: ${CLAWTEX_SRC}"
echo ""

# ══════════════════════════════════════════════════════════════════
# Phase 1: 編譯
# ══════════════════════════════════════════════════════════════════

mkdir -p "$RELEASE_DIR/${VERSION}"

if [[ "$SKIP_BUILD" == "false" ]]; then
    log_step "Phase 1: 跨平台編譯"

    # ── Windows x86_64 (本地編譯) ────────────────────────────────
    log_info "編譯 Windows x86_64 (本地)..."
    cargo build --release 2>&1 | tail -3
    cp "${BUILD_DIR}/release/clawtex-core.exe" "${RELEASE_DIR}/${VERSION}/clawtex-core-windows-x86_64.exe"
    log_ok "Windows x86_64 binary 完成 ($(du -h "${RELEASE_DIR}/${VERSION}/clawtex-core-windows-x86_64.exe" | cut -f1))"

    # ── macOS aarch64 (遠端編譯) ─────────────────────────────────
    M1_SSH="mark@10.0.2.1"
    if ssh -o ConnectTimeout=5 "$M1_SSH" "true" 2>/dev/null; then
        log_info "編譯 macOS aarch64 (遠端 M1)..."
        # 同步源碼到 M1
        rsync -az --delete \
            --exclude target --exclude .git \
            "${CLAWTEX_SRC}/" "${M1_SSH}:/tmp/clawtex-build/"
        # 遠端編譯
        ssh "$M1_SSH" "cd /tmp/clawtex-build && cargo build --release 2>&1 | tail -3"
        # 拉回 binary
        scp "${M1_SSH}:/tmp/clawtex-build/target/release/clawtex-core" \
            "${RELEASE_DIR}/${VERSION}/clawtex-core-macos-aarch64"
        log_ok "macOS aarch64 binary 完成"
    else
        log_warn "M1 不可達，跳過 macOS 編譯"
    fi

    # ── Linux x86_64 (cross 交叉編譯) ───────────────────────────
    if command -v cross &>/dev/null; then
        log_info "編譯 Linux x86_64 (cross)..."
        cross build --release --target x86_64-unknown-linux-gnu 2>&1 | tail -3
        cp "${BUILD_DIR}/x86_64-unknown-linux-gnu/release/clawtex-core" \
            "${RELEASE_DIR}/${VERSION}/clawtex-core-linux-x86_64"
        log_ok "Linux x86_64 binary 完成"
    else
        log_warn "cross 未安裝，跳過 Linux 編譯。安裝: cargo install cross"
    fi

    echo ""
    log_ok "Phase 1 完成 — 所有 binary 在: ${RELEASE_DIR}/${VERSION}/"
    ls -lh "${RELEASE_DIR}/${VERSION}/"
    echo ""
fi

if [[ "$BUILD_ONLY" == "true" ]]; then
    log_info "--build-only 模式，跳過部署"
    exit 0
fi

# ══════════════════════════════════════════════════════════════════
# Phase 2: 滾動部署
# ══════════════════════════════════════════════════════════════════

log_step "Phase 2: 滾動部署"

DEPLOY_SUCCESS=0
DEPLOY_FAIL=0
FAILED_NODES=()

deploy_node() {
    local node="$1"
    local name ssh_target os arch deploy_path binary_name binary_src

    name=$(get_name "$node")
    ssh_target=$(get_ssh "$node")
    os=$(get_os "$node")
    arch=$(get_arch "$node")
    deploy_path=$(get_deploy_path "$node")
    binary_name=$(get_binary_name "$os")

    # 選擇正確的 binary
    case "${os}-${arch}" in
        windows-x86_64)  binary_src="${RELEASE_DIR}/${VERSION}/clawtex-core-windows-x86_64.exe" ;;
        macos-aarch64)   binary_src="${RELEASE_DIR}/${VERSION}/clawtex-core-macos-aarch64" ;;
        linux-x86_64)    binary_src="${RELEASE_DIR}/${VERSION}/clawtex-core-linux-x86_64" ;;
        *)
            log_error "[$name] 不支援的平台: ${os}-${arch}"
            return 1
            ;;
    esac

    if [[ ! -f "$binary_src" ]]; then
        log_error "[$name] Binary 不存在: $binary_src"
        return 1
    fi

    log_info "[$name] 開始部署 (${os}/${arch})..."

    # ── 2a. 建立目錄結構 ─────────────────────────────────────────
    if [[ -n "$ssh_target" ]]; then
        run_remote "$ssh_target" "mkdir -p '${deploy_path}/bin' '${deploy_path}/releases/${VERSION}' '${deploy_path}/config' '${deploy_path}/data'"
    fi

    # ── 2b. 上傳 binary ────────────────────────────────────────
    if [[ -n "$ssh_target" ]]; then
        log_info "[$name] 上傳 binary..."
        scp -q "$binary_src" "${ssh_target}:${deploy_path}/releases/${VERSION}/${binary_name}"
        if [[ "$os" != "windows" ]]; then
            run_remote "$ssh_target" "chmod +x '${deploy_path}/releases/${VERSION}/${binary_name}'"
        fi
    else
        # Hub: 本地複製
        mkdir -p "${deploy_path}/bin" "${deploy_path}/releases/${VERSION}"
        cp "$binary_src" "${deploy_path}/releases/${VERSION}/${binary_name}"
    fi

    # ── 2c. 停止舊服務 ──────────────────────────────────────────
    log_info "[$name] 停止舊服務..."
    if [[ "$os" == "windows" ]]; then
        run_remote "$ssh_target" "taskkill /f /im clawtex-core.exe 2>/dev/null || true" 2>/dev/null || true
    else
        run_remote "$ssh_target" "pkill -f clawtex-core 2>/dev/null || true" 2>/dev/null || true
    fi
    sleep 2

    # ── 2d. 更新 binary (保留舊版本用於回滾) ──────────────────────
    if [[ -n "$ssh_target" ]]; then
        run_remote "$ssh_target" "
            cd '${deploy_path}/bin' && \
            if [ -f '${binary_name}' ]; then cp '${binary_name}' '${binary_name}.prev'; fi && \
            cp '${deploy_path}/releases/${VERSION}/${binary_name}' '${binary_name}'
        "
    else
        cd "${deploy_path}/bin"
        if [[ -f "${binary_name}" ]]; then
            cp "${binary_name}" "${binary_name}.prev"
        fi
        cp "${deploy_path}/releases/${VERSION}/${binary_name}" "${binary_name}"
    fi

    # ── 2e. 啟動新服務 ──────────────────────────────────────────
    log_info "[$name] 啟動新服務..."
    local role
    role=$(get_role "$node")

    if [[ "$os" == "windows" ]]; then
        # Windows: 使用 start 在背景啟動
        if [[ "$role" == "hub" ]]; then
            run_remote "$ssh_target" "cd '${deploy_path}' && start /b bin/clawtex-core.exe --host 0.0.0.0 --config config/agents.toml daemon" &
        else
            run_remote "$ssh_target" "cd '${deploy_path}' && start /b bin/clawtex-core.exe --host 0.0.0.0 --config config/agents.toml daemon" &
        fi
    elif [[ "$os" == "macos" ]]; then
        # macOS: nohup 背景啟動
        run_remote "$ssh_target" "
            cd '${deploy_path}' && \
            nohup bin/clawtex-core --host 0.0.0.0 --config config/agents.toml daemon \
                > /tmp/clawtex-core.log 2>&1 &
        "
    else
        # Linux: systemd 或 nohup
        run_remote "$ssh_target" "
            if command -v systemctl &>/dev/null && [ -f /etc/systemd/system/clawtex-core.service ]; then
                sudo systemctl restart clawtex-core
            else
                cd '${deploy_path}' && \
                nohup bin/clawtex-core --host 0.0.0.0 --config config/agents.toml daemon \
                    > /tmp/clawtex-core.log 2>&1 &
            fi
        "
    fi

    # ── 2f. 等待啟動 ───────────────────────────────────────────
    log_info "[$name] 等待服務啟動..."
    sleep 10

    # ── 2g. 健康檢查 ───────────────────────────────────────────
    local core_url="${CORE_URLS[$name]:-}"
    if [[ -n "$core_url" ]]; then
        local retries=3
        local healthy=false
        for i in $(seq 1 $retries); do
            if check_core "$core_url"; then
                healthy=true
                break
            fi
            log_warn "[$name] 健康檢查 #${i} 失敗，重試..."
            sleep 5
        done

        if [[ "$healthy" == "true" ]]; then
            log_ok "[$name] 部署成功，服務健康"
            return 0
        else
            log_error "[$name] 健康檢查失敗！嘗試回滾..."
            # 回滾
            if [[ -n "$ssh_target" ]]; then
                run_remote "$ssh_target" "
                    cd '${deploy_path}/bin' && \
                    if [ -f '${binary_name}.prev' ]; then
                        cp '${binary_name}.prev' '${binary_name}'
                    fi
                "
            fi
            return 1
        fi
    else
        log_warn "[$name] 無 HTTP endpoint，跳過健康檢查"
        return 0
    fi
}

# ── 執行部署 ──────────────────────────────────────────────────────

# 決定要部署哪些節點
deploy_list=()
if [[ -n "$TARGET_NODE" ]]; then
    for node in "${NODES[@]}"; do
        if [[ "$(get_name "$node")" == "$TARGET_NODE" ]]; then
            deploy_list+=("$node")
        fi
    done
    if [[ ${#deploy_list[@]} -eq 0 ]]; then
        log_error "找不到節點: $TARGET_NODE"
        exit 1
    fi
elif [[ "$WORKERS_ONLY" == "true" ]]; then
    while IFS= read -r node; do
        deploy_list+=("$node")
    done < <(get_workers)
else
    # 先 Workers，後 Hub
    while IFS= read -r node; do
        deploy_list+=("$node")
    done < <(get_workers)
    hub=$(get_hub)
    if [[ -n "$hub" ]]; then
        deploy_list+=("$hub")
    fi
fi

for node in "${deploy_list[@]}"; do
    name=$(get_name "$node")
    echo ""
    echo "════════════════════════════════════════════"
    log_step "部署節點: $name"
    echo "════════════════════════════════════════════"

    if deploy_node "$node"; then
        ((DEPLOY_SUCCESS++))
    else
        ((DEPLOY_FAIL++))
        FAILED_NODES+=("$name")
        log_error "節點 $name 部署失敗，中止後續部署"
        break  # 滾動更新: 一台失敗就停止
    fi
done

# ══════════════════════════════════════════════════════════════════
# Phase 3: 報告
# ══════════════════════════════════════════════════════════════════

echo ""
echo "════════════════════════════════════════════"
log_step "部署報告"
echo "════════════════════════════════════════════"
echo "版本:    ${VERSION}"
echo "成功:    ${DEPLOY_SUCCESS} 台"
echo "失敗:    ${DEPLOY_FAIL} 台"
if [[ ${#FAILED_NODES[@]} -gt 0 ]]; then
    echo "失敗節點: ${FAILED_NODES[*]}"
fi
echo ""

# Telegram 通知
if [[ $DEPLOY_FAIL -eq 0 ]]; then
    notify_telegram "Clawtex 部署完成
版本: ${VERSION}
成功: ${DEPLOY_SUCCESS} 台"
    log_ok "部署完成"
else
    notify_telegram "Clawtex 部署有問題!
版本: ${VERSION}
成功: ${DEPLOY_SUCCESS} 台
失敗: ${DEPLOY_FAIL} 台 (${FAILED_NODES[*]})"
    log_error "部署有失敗節點"
    exit 1
fi
```

### 7.3 deploy/health-check.sh (健康檢查)

```bash
#!/usr/bin/env bash
# deploy/health-check.sh — 檢查所有節點狀態
#
# 用法:
#   ./health-check.sh              # 完整檢查
#   ./health-check.sh --quick      # 只檢查連通性
#   ./health-check.sh --json       # JSON 輸出 (供程式讀取)
#   ./health-check.sh --watch 30   # 每 30 秒重複檢查

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/inventory.sh"

QUICK=false
JSON_OUTPUT=false
WATCH_INTERVAL=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --quick) QUICK=true; shift ;;
        --json)  JSON_OUTPUT=true; shift ;;
        --watch) WATCH_INTERVAL="$2"; shift 2 ;;
        *) shift ;;
    esac
done

# ── 檢查函數 ──────────────────────────────────────────────────────

check_node() {
    local node="$1"
    local name ssh_target os ollama_url core_url
    name=$(get_name "$node")
    ssh_target=$(get_ssh "$node")
    os=$(get_os "$node")
    ollama_url="${OLLAMA_URLS[$name]:-}"
    core_url="${CORE_URLS[$name]:-}"

    local ssh_ok="false"
    local ollama_ok="false"
    local core_ok="false"
    local ollama_models=""
    local core_version=""
    local disk_free=""
    local ram_free=""

    # SSH 連通性 (Hub 跳過)
    if [[ -n "$ssh_target" ]]; then
        if ssh -o ConnectTimeout=5 -o StrictHostKeyChecking=no "$ssh_target" "true" 2>/dev/null; then
            ssh_ok="true"
        fi
    else
        ssh_ok="true"  # Hub 是本地
    fi

    # Ollama 檢查
    if [[ -n "$ollama_url" ]]; then
        local tags_json
        tags_json=$(curl -sf --max-time 5 "${ollama_url}/api/tags" 2>/dev/null) || true
        if [[ -n "$tags_json" ]]; then
            ollama_ok="true"
            ollama_models=$(echo "$tags_json" | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    models = [m['name'] for m in data.get('models', [])]
    print(', '.join(models))
except: print('parse error')
" 2>/dev/null || echo "unknown")
        fi
    fi

    # clawtex-core 檢查
    if [[ -n "$core_url" ]]; then
        local health_json
        health_json=$(curl -sf --max-time 5 "${core_url}/health" 2>/dev/null) || true
        if echo "$health_json" | grep -q '"status":"ok"' 2>/dev/null; then
            core_ok="true"
            core_version=$(echo "$health_json" | python3 -c "
import sys, json
try: print(json.load(sys.stdin).get('version','unknown'))
except: print('unknown')
" 2>/dev/null || echo "unknown")
        fi
    fi

    # 磁碟和記憶體 (非 quick 模式)
    if [[ "$QUICK" == "false" && "$ssh_ok" == "true" ]]; then
        if [[ -n "$ssh_target" ]]; then
            if [[ "$os" == "windows" ]]; then
                disk_free=$(run_remote "$ssh_target" "wmic logicaldisk where 'DeviceID=\"C:\"' get FreeSpace /format:value 2>/dev/null | grep FreeSpace" 2>/dev/null | sed 's/FreeSpace=//' | tr -d '\r\n' || echo "N/A")
                if [[ "$disk_free" =~ ^[0-9]+$ ]]; then
                    disk_free="$((disk_free / 1073741824))GB"
                fi
            else
                disk_free=$(run_remote "$ssh_target" "df -h / | tail -1 | awk '{print \$4}'" 2>/dev/null || echo "N/A")
                ram_free=$(run_remote "$ssh_target" "free -h 2>/dev/null | awk '/Mem:/{print \$7}'" 2>/dev/null || echo "N/A")
            fi
        fi
    fi

    # ── 輸出 ────────────────────────────────────────────────────
    if [[ "$JSON_OUTPUT" == "true" ]]; then
        echo "{\"name\":\"$name\",\"ssh\":$ssh_ok,\"ollama\":$ollama_ok,\"core\":$core_ok,\"models\":\"$ollama_models\",\"version\":\"$core_version\"}"
    else
        local status_icon
        if [[ "$ssh_ok" == "true" && "$ollama_ok" == "true" ]]; then
            status_icon="${GREEN}HEALTHY${NC}"
        elif [[ "$ssh_ok" == "true" ]]; then
            status_icon="${YELLOW}PARTIAL${NC}"
        else
            status_icon="${RED}DOWN${NC}"
        fi

        echo -e "  [$status_icon] $name ($os)"
        echo -e "    SSH: $(if [[ $ssh_ok == true ]]; then echo -e "${GREEN}OK${NC}"; else echo -e "${RED}FAIL${NC}"; fi)"
        echo -e "    Ollama: $(if [[ $ollama_ok == true ]]; then echo -e "${GREEN}OK${NC} (${ollama_models})"; else echo -e "${RED}FAIL${NC}"; fi)"
        if [[ -n "$core_url" ]]; then
            echo -e "    Core: $(if [[ $core_ok == true ]]; then echo -e "${GREEN}OK${NC} v${core_version}"; else echo -e "${YELLOW}OFF${NC}"; fi)"
        fi
        if [[ -n "$disk_free" ]]; then
            echo -e "    Disk: ${disk_free}"
        fi
        if [[ -n "$ram_free" ]]; then
            echo -e "    RAM: ${ram_free} available"
        fi
    fi
}

# ── 主迴圈 ────────────────────────────────────────────────────────

run_check() {
    if [[ "$JSON_OUTPUT" != "true" ]]; then
        echo ""
        echo "══════════════════════════════════════════"
        echo " Clawtex 集群健康檢查 — $(date '+%Y-%m-%d %H:%M:%S')"
        echo "══════════════════════════════════════════"
        echo ""
    else
        echo "["
    fi

    local first=true
    for node in "${NODES[@]}"; do
        if [[ "$JSON_OUTPUT" == "true" && "$first" != "true" ]]; then
            echo ","
        fi
        check_node "$node"
        first=false
    done

    if [[ "$JSON_OUTPUT" == "true" ]]; then
        echo "]"
    else
        echo ""
    fi
}

if [[ $WATCH_INTERVAL -gt 0 ]]; then
    while true; do
        clear
        run_check
        echo "下次檢查: ${WATCH_INTERVAL} 秒後 (Ctrl+C 退出)"
        sleep "$WATCH_INTERVAL"
    done
else
    run_check
fi
```

### 7.4 deploy/update-models.sh (模型管理)

```bash
#!/usr/bin/env bash
# deploy/update-models.sh — 一鍵更新所有節點的 Ollama 模型
#
# 用法:
#   ./update-models.sh                    # 同步所有節點模型
#   ./update-models.sh --node m1          # 只更新 M1
#   ./update-models.sh --pull qwen3:8b    # 在所有節點拉取指定模型
#   ./update-models.sh --remove unused    # 列出各節點未在清單中的模型
#   ./update-models.sh --status           # 顯示各節點模型列表

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/inventory.sh"

# ── 模型定義 (等同 models.toml) ─────────────────────────────────

declare -A NODE_MODELS=(
    [z13]="qwen3:8b qwen3-coder:30b llama3.1:8b"
    [m1]="llama3.1:8b qwen2.5:14b"
    [ayaneo]="qwen2.5:7b"
    [acer]="llama3.1:8b qwen2.5:13b"
)

# ── 參數 ──────────────────────────────────────────────────────────

TARGET_NODE=""
PULL_MODEL=""
STATUS_ONLY=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --node)    TARGET_NODE="$2"; shift 2 ;;
        --pull)    PULL_MODEL="$2"; shift 2 ;;
        --status)  STATUS_ONLY=true; shift ;;
        --help|-h)
            echo "用法: update-models.sh [OPTIONS]"
            echo "  --node NAME    只更新指定節點"
            echo "  --pull MODEL   在所有節點拉取指定模型"
            echo "  --status       只顯示模型列表"
            exit 0
            ;;
        *) shift ;;
    esac
done

# ── 函數 ──────────────────────────────────────────────────────────

get_remote_models() {
    local ollama_url="$1"
    curl -sf --max-time 10 "${ollama_url}/api/tags" 2>/dev/null | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    for m in data.get('models', []):
        name = m['name']
        size_gb = m.get('size', 0) / 1073741824
        print(f'{name}|{size_gb:.1f}GB')
except Exception as e:
    print(f'ERROR|{e}', file=sys.stderr)
" 2>/dev/null
}

pull_model_remote() {
    local ollama_url="$1"
    local model="$2"

    log_info "拉取模型: $model ..."

    # Ollama pull API
    curl -sf --max-time 3600 \
        -X POST "${ollama_url}/api/pull" \
        -d "{\"name\": \"${model}\", \"stream\": false}" \
        2>/dev/null

    return $?
}

# ── 主流程 ────────────────────────────────────────────────────────

echo ""
echo "══════════════════════════════════════════"
echo " Clawtex Ollama 模型管理"
echo "══════════════════════════════════════════"
echo ""

for node in "${NODES[@]}"; do
    name=$(get_name "$node")

    # 過濾節點
    if [[ -n "$TARGET_NODE" && "$name" != "$TARGET_NODE" ]]; then
        continue
    fi

    ollama_url="${OLLAMA_URLS[$name]:-}"
    if [[ -z "$ollama_url" ]]; then
        log_warn "[$name] 沒有 Ollama URL，跳過"
        continue
    fi

    echo "────────────────────────────────────────"
    log_step "[$name] ${ollama_url}"
    echo "────────────────────────────────────────"

    # 檢查 Ollama 是否在線
    if ! check_ollama "$ollama_url"; then
        log_error "[$name] Ollama 不可達"
        continue
    fi

    # 取得現有模型
    current_models=()
    while IFS='|' read -r model_name model_size; do
        current_models+=("$model_name")
        if [[ "$STATUS_ONLY" == "true" ]]; then
            echo "  - $model_name ($model_size)"
        fi
    done < <(get_remote_models "$ollama_url")

    if [[ "$STATUS_ONLY" == "true" ]]; then
        echo ""
        continue
    fi

    # 如果指定了 --pull，拉取單一模型
    if [[ -n "$PULL_MODEL" ]]; then
        if printf '%s\n' "${current_models[@]}" | grep -q "^${PULL_MODEL}$"; then
            log_ok "[$name] 已有 $PULL_MODEL"
        else
            if pull_model_remote "$ollama_url" "$PULL_MODEL"; then
                log_ok "[$name] 拉取 $PULL_MODEL 完成"
            else
                log_error "[$name] 拉取 $PULL_MODEL 失敗"
            fi
        fi
        continue
    fi

    # 同步模型清單
    expected_models_str="${NODE_MODELS[$name]:-}"
    if [[ -z "$expected_models_str" ]]; then
        log_warn "[$name] 沒有定義模型清單"
        continue
    fi

    read -ra expected_models <<< "$expected_models_str"

    # 找出缺少的模型
    for expected in "${expected_models[@]}"; do
        if printf '%s\n' "${current_models[@]}" | grep -q "^${expected}$"; then
            log_ok "[$name] $expected (已有)"
        else
            log_info "[$name] $expected (缺少，開始拉取...)"
            if pull_model_remote "$ollama_url" "$expected"; then
                log_ok "[$name] $expected 拉取完成"
            else
                log_error "[$name] $expected 拉取失敗"
            fi
        fi
    done

    # 找出多餘的模型 (只提示，不刪除)
    for current in "${current_models[@]}"; do
        if ! printf '%s\n' "${expected_models[@]}" | grep -q "^${current}$"; then
            log_warn "[$name] $current 不在清單中 (可手動 ollama rm)"
        fi
    done

    echo ""
done

log_ok "模型同步完成"
```

### 7.5 deploy/rollback.sh (回滾)

```bash
#!/usr/bin/env bash
# deploy/rollback.sh — 回滾指定節點到上一版本
#
# 用法:
#   ./rollback.sh <node_name>     # 回滾指定節點
#   ./rollback.sh --all           # 回滾所有節點

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/inventory.sh"

if [[ $# -lt 1 ]]; then
    echo "用法: rollback.sh <node_name|--all>"
    exit 1
fi

rollback_node() {
    local node="$1"
    local name ssh_target os deploy_path binary_name
    name=$(get_name "$node")
    ssh_target=$(get_ssh "$node")
    os=$(get_os "$node")
    deploy_path=$(get_deploy_path "$node")
    binary_name=$(get_binary_name "$os")

    log_step "[$name] 回滾..."

    # 檢查 .prev 存在
    local has_prev
    if [[ -n "$ssh_target" ]]; then
        has_prev=$(run_remote "$ssh_target" "test -f '${deploy_path}/bin/${binary_name}.prev' && echo yes || echo no")
    else
        if [[ -f "${deploy_path}/bin/${binary_name}.prev" ]]; then
            has_prev="yes"
        else
            has_prev="no"
        fi
    fi

    if [[ "$has_prev" != "yes" ]]; then
        log_error "[$name] 沒有可回滾的版本 (.prev 不存在)"
        return 1
    fi

    # 停止服務
    if [[ "$os" == "windows" ]]; then
        run_remote "$ssh_target" "taskkill /f /im clawtex-core.exe 2>/dev/null || true" 2>/dev/null || true
    else
        run_remote "$ssh_target" "pkill -f clawtex-core 2>/dev/null || true" 2>/dev/null || true
    fi
    sleep 2

    # 交換 binary
    if [[ -n "$ssh_target" ]]; then
        run_remote "$ssh_target" "
            cd '${deploy_path}/bin' && \
            mv '${binary_name}' '${binary_name}.failed' && \
            mv '${binary_name}.prev' '${binary_name}'
        "
    else
        cd "${deploy_path}/bin"
        mv "${binary_name}" "${binary_name}.failed"
        mv "${binary_name}.prev" "${binary_name}"
    fi

    # 重啟
    log_info "[$name] 啟動舊版本..."
    if [[ "$os" == "windows" ]]; then
        run_remote "$ssh_target" "cd '${deploy_path}' && start /b bin/clawtex-core.exe --host 0.0.0.0 --config config/agents.toml daemon" &
    else
        run_remote "$ssh_target" "cd '${deploy_path}' && nohup bin/clawtex-core --host 0.0.0.0 --config config/agents.toml daemon > /tmp/clawtex-core.log 2>&1 &"
    fi

    sleep 5

    # 健康檢查
    local core_url="${CORE_URLS[$name]:-}"
    if [[ -n "$core_url" ]] && check_core "$core_url"; then
        log_ok "[$name] 回滾成功"
    else
        log_warn "[$name] 回滾後健康檢查未通過，請手動檢查"
    fi
}

# 執行回滾
if [[ "$1" == "--all" ]]; then
    for node in "${NODES[@]}"; do
        rollback_node "$node"
    done
else
    found=false
    for node in "${NODES[@]}"; do
        if [[ "$(get_name "$node")" == "$1" ]]; then
            rollback_node "$node"
            found=true
            break
        fi
    done
    if [[ "$found" == "false" ]]; then
        log_error "找不到節點: $1"
        exit 1
    fi
fi
```

### 7.6 deploy/sync-config.sh (配置同步)

```bash
#!/usr/bin/env bash
# deploy/sync-config.sh — 同步配置到所有 Worker
#
# 用法:
#   ./sync-config.sh                   # 同步所有 Worker
#   ./sync-config.sh --node ayaneo     # 只同步指定節點

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/inventory.sh"

CONFIG_DIR="${SCRIPT_DIR}/config"
TARGET_NODE="${1:-}"
if [[ "$TARGET_NODE" == "--node" ]]; then
    TARGET_NODE="${2:-}"
fi

log_step "同步配置到 Workers"

for node in "${NODES[@]}"; do
    name=$(get_name "$node")
    role=$(get_role "$node")
    ssh_target=$(get_ssh "$node")
    deploy_path=$(get_deploy_path "$node")

    # 跳過 Hub (Hub 直接用本地配置)
    if [[ "$role" == "hub" ]]; then
        continue
    fi

    # 過濾節點
    if [[ -n "$TARGET_NODE" && "$name" != "$TARGET_NODE" ]]; then
        continue
    fi

    if [[ -z "$ssh_target" ]]; then
        continue
    fi

    log_info "[$name] 同步配置..."

    # 檢查 Worker 配置模板
    local worker_config="${CONFIG_DIR}/workers/${name}/agents.toml"
    if [[ ! -f "$worker_config" ]]; then
        log_warn "[$name] 配置不存在: $worker_config，使用通用 Worker 模板"
        worker_config="${CONFIG_DIR}/workers/default/agents.toml"
    fi

    if [[ ! -f "$worker_config" ]]; then
        log_error "[$name] 找不到任何可用配置"
        continue
    fi

    # 上傳配置
    scp -q "$worker_config" "${ssh_target}:${deploy_path}/config/agents.toml"
    log_ok "[$name] 配置已同步"

    # 重啟服務 (讓新配置生效)
    local os
    os=$(get_os "$node")
    if [[ "$os" == "windows" ]]; then
        run_remote "$ssh_target" "taskkill /f /im clawtex-core.exe 2>/dev/null; cd '${deploy_path}' && start /b bin/clawtex-core.exe --host 0.0.0.0 --config config/agents.toml daemon" 2>/dev/null &
    else
        run_remote "$ssh_target" "pkill -f clawtex-core 2>/dev/null; sleep 2; cd '${deploy_path}' && nohup bin/clawtex-core --host 0.0.0.0 --config config/agents.toml daemon > /tmp/clawtex-core.log 2>&1 &"
    fi

    log_ok "[$name] 服務已重啟"
done

log_ok "配置同步完成"
```

### 7.7 deploy/setup-node.sh (新節點初始化)

```bash
#!/usr/bin/env bash
# deploy/setup-node.sh — 初始化新的 Worker 節點
#
# 用法:
#   ./setup-node.sh <ssh_target> <node_name> <os>
#   例: ./setup-node.sh worker@10.0.168.1.120 newnode linux

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/inventory.sh"

if [[ $# -lt 3 ]]; then
    echo "用法: setup-node.sh <ssh_target> <node_name> <os>"
    echo "  os: linux | macos | windows"
    exit 1
fi

SSH_TARGET="$1"
NODE_NAME="$2"
NODE_OS="$3"

log_step "初始化新節點: $NODE_NAME ($NODE_OS)"

# 測試 SSH
log_info "測試 SSH 連線..."
if ! ssh -o ConnectTimeout=10 "$SSH_TARGET" "true" 2>/dev/null; then
    log_error "SSH 連線失敗: $SSH_TARGET"
    exit 1
fi
log_ok "SSH 連線成功"

case "$NODE_OS" in
    linux)
        log_info "安裝 Linux 依賴..."
        ssh "$SSH_TARGET" "bash -s" <<'REMOTE_SCRIPT'
set -e

# 安裝 Ollama
if ! command -v ollama &>/dev/null; then
    echo "[INFO] 安裝 Ollama..."
    curl -fsSL https://ollama.com/install.sh | sh
fi

# 設定 Ollama 監聽所有介面
sudo mkdir -p /etc/systemd/system/ollama.service.d
sudo tee /etc/systemd/system/ollama.service.d/override.conf > /dev/null <<EOF
[Service]
Environment="OLLAMA_HOST=0.0.0.0"
Environment="OLLAMA_KEEP_ALIVE=24h"
EOF
sudo systemctl daemon-reload
sudo systemctl restart ollama
sudo systemctl enable ollama

# 建立 clawtex 目錄
sudo mkdir -p /opt/clawtex/{bin,config,data,releases}
sudo chown -R $(whoami) /opt/clawtex

# 安裝 Python3 (browser/email 工具需要)
if ! command -v python3 &>/dev/null; then
    sudo apt-get update && sudo apt-get install -y python3 python3-pip
fi

# 建立 systemd 服務
sudo tee /etc/systemd/system/clawtex-core.service > /dev/null <<EOF
[Unit]
Description=Clawtex Core - LLM Agent Daemon
After=network.target ollama.service

[Service]
Type=simple
User=$(whoami)
WorkingDirectory=/opt/clawtex
ExecStart=/opt/clawtex/bin/clawtex-core --host 0.0.0.0 --config /opt/clawtex/config/agents.toml daemon
Restart=on-failure
RestartSec=10
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload

echo "[OK] Linux 節點初始化完成"
REMOTE_SCRIPT
        ;;

    macos)
        log_info "設定 macOS 依賴..."
        ssh "$SSH_TARGET" "bash -s" <<'REMOTE_SCRIPT'
set -e

# 安裝 Homebrew (如果沒有)
if ! command -v brew &>/dev/null; then
    /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
fi

# 安裝 Ollama
if ! command -v ollama &>/dev/null; then
    brew install ollama
fi

# 設定環境變數
grep -q OLLAMA_HOST ~/.zshrc 2>/dev/null || echo 'export OLLAMA_HOST=0.0.0.0' >> ~/.zshrc
grep -q OLLAMA_KEEP_ALIVE ~/.zshrc 2>/dev/null || echo 'export OLLAMA_KEEP_ALIVE=24h' >> ~/.zshrc

# 建立 clawtex 目錄
mkdir -p /opt/clawtex/{bin,config,data,releases}

# 安裝 Rust (用於遠端編譯)
if ! command -v cargo &>/dev/null; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi

echo "[OK] macOS 節點初始化完成"
REMOTE_SCRIPT
        ;;

    windows)
        log_info "Windows 節點需要手動設定:"
        echo ""
        echo "  1. 安裝 Ollama:  winget install Ollama.Ollama"
        echo "  2. 設定環境變數:"
        echo "     setx OLLAMA_HOST 0.0.0.0"
        echo "     setx OLLAMA_KEEP_ALIVE 24h"
        echo "  3. 建立目錄:  mkdir C:\\clawtex\\bin C:\\clawtex\\config C:\\clawtex\\data"
        echo "  4. 確保 SSH Server 已啟用 (Settings > Apps > Optional Features > OpenSSH Server)"
        echo "  5. 啟動 Ollama:  ollama serve"
        echo ""
        log_info "建立遠端目錄..."
        ssh "$SSH_TARGET" "mkdir -p C:/clawtex/bin C:/clawtex/config C:/clawtex/data C:/clawtex/releases" 2>/dev/null || true
        ;;
esac

log_ok "節點 $NODE_NAME 初始化完成"
echo ""
echo "下一步:"
echo "  1. 將此節點加入 inventory.sh 的 NODES 陣列"
echo "  2. 在 NODE_MODELS 中定義需要的模型"
echo "  3. 執行 ./update-models.sh --node $NODE_NAME"
echo "  4. 執行 ./deploy.sh --node $NODE_NAME"
```

### 7.8 deploy/cleanup.sh (清理舊版本)

```bash
#!/usr/bin/env bash
# deploy/cleanup.sh — 清理舊版本的 binary，保留最近 N 個
#
# 用法:
#   ./cleanup.sh           # 保留最近 5 個版本
#   ./cleanup.sh --keep 3  # 保留最近 3 個版本

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/inventory.sh"

KEEP=${2:-5}

log_step "清理舊版本 (保留最近 ${KEEP} 個)"

for node in "${NODES[@]}"; do
    name=$(get_name "$node")
    ssh_target=$(get_ssh "$node")
    os=$(get_os "$node")
    deploy_path=$(get_deploy_path "$node")

    log_info "[$name] 檢查 releases 目錄..."

    local count
    if [[ -n "$ssh_target" ]]; then
        count=$(run_remote "$ssh_target" "ls -1d '${deploy_path}/releases'/*/ 2>/dev/null | wc -l" 2>/dev/null || echo "0")
    else
        count=$(ls -1d "${deploy_path}/releases"/*/ 2>/dev/null | wc -l || echo "0")
    fi

    count=$(echo "$count" | tr -d '[:space:]')

    if [[ "$count" -le "$KEEP" ]]; then
        log_ok "[$name] ${count} 個版本，不需要清理"
        continue
    fi

    local to_delete=$((count - KEEP))
    log_info "[$name] ${count} 個版本，刪除最舊的 ${to_delete} 個"

    if [[ -n "$ssh_target" ]]; then
        run_remote "$ssh_target" "
            cd '${deploy_path}/releases' && \
            ls -1d */ | head -${to_delete} | while read d; do
                echo \"  刪除: \$d\"
                rm -rf \"\$d\"
            done
        "
    else
        cd "${deploy_path}/releases"
        ls -1d */ | head -"${to_delete}" | while read -r d; do
            echo "  刪除: $d"
            rm -rf "$d"
        done
    fi

    log_ok "[$name] 清理完成"
done
```

---

## 8. Infrastructure as Code 選型

### 8.1 決定: 純 Shell Script

**不用 Ansible 的理由:**
1. 只有 8 台機器 -- Ansible 的學習和維護成本超過收益
2. Windows 機器佔多數 -- Ansible 對 Windows 的支援需要 WinRM，設定複雜
3. 我們的操作很簡單: SCP + SSH + 啟動/停止服務
4. Shell script 可以直接 debug，團隊都看得懂
5. 不需要額外的 inventory file 格式 (YAML vs 我們的 bash arrays)

**不用 Terraform 的理由:**
1. 沒有雲端資源需要管理
2. 這是實體機器，Terraform 沒有用武之地

**不用 Salt/Puppet 的理由:**
1. 規模太小，不值得
2. 需要在所有機器上安裝 agent，增加複雜度

### 8.2 腳本架構

```
deploy/
├── inventory.sh         # 共用函數庫 + 節點清單 (所有腳本 source 此檔)
├── deploy.sh            # 一鍵部署 binary
├── health-check.sh      # 健康檢查
├── update-models.sh     # 模型管理
├── rollback.sh          # 回滾
├── sync-config.sh       # 配置同步
├── setup-node.sh        # 新節點初始化
├── cleanup.sh           # 清理舊版本
├── config/
│   ├── hub/
│   │   └── agents.toml  # Hub 專用配置
│   └── workers/
│       ├── default/
│       │   └── agents.toml  # Worker 通用配置
│       ├── m1/
│       │   └── agents.toml  # M1 專用配置
│       ├── ayaneo/
│       │   └── agents.toml
│       └── acer/
│           └── agents.toml
└── releases/            # 編譯好的 binary 存放目錄
    └── {version}/
        ├── clawtex-core-windows-x86_64.exe
        ├── clawtex-core-macos-aarch64
        └── clawtex-core-linux-x86_64
```

### 8.3 何時考慮升級到 Ansible

當滿足以下條件時:
- 節點數量超過 20 台
- 需要管理多種不同服務 (不只是 clawtex-core)
- 有多人 DevOps 團隊需要標準化工具
- 需要 idempotent 配置管理 (確保系統狀態一致)

---

## 9. 部署流程圖

### 9.1 完整部署流程

```
                    開發者推送代碼
                         │
                         ▼
            ┌────────────────────────┐
            │  Z13: cargo build      │
            │  (Windows x86_64)      │
            ├────────────────────────┤
            │  SSH M1: cargo build   │
            │  (macOS aarch64)       │
            ├────────────────────────┤
            │  cross build           │
            │  (Linux x86_64)        │
            └───────────┬────────────┘
                        │
                        ▼
            ┌────────────────────────┐
            │  releases/{version}/   │
            │  ├── ...windows.exe    │
            │  ├── ...macos          │
            │  └── ...linux          │
            └───────────┬────────────┘
                        │
          ┌─────────────┼─────────────┐
          │             │             │
          ▼             ▼             ▼
    ┌──────────┐  ┌──────────┐  ┌──────────┐
    │ Worker 1 │  │ Worker 2 │  │ Worker 3 │
    │  (M1)    │  │ (Ayaneo) │  │  (Acer)  │
    │          │  │          │  │          │
    │ 1.上傳   │  │ (等 W1   │  │ (等 W2   │
    │ 2.停止   │  │  完成)   │  │  完成)   │
    │ 3.替換   │  │          │  │          │
    │ 4.啟動   │  │ 重複     │  │ 重複     │
    │ 5.檢查   │  │ 1-5步    │  │ 1-5步    │
    │  ✓ PASS  │──▶          │──▶          │
    └──────────┘  └──────────┘  └──────────┘
                                     │
                                     ▼ (全部 Worker OK)
                              ┌──────────┐
                              │   Hub    │
                              │  (Z13)   │
                              │ 重複 1-5 │
                              └──────────┘
                                     │
                                     ▼
                        ┌────────────────────────┐
                        │ Telegram 通知:         │
                        │ "部署完成 v{version}"   │
                        │ "3/3 Workers + Hub OK" │
                        └────────────────────────┘
```

### 9.2 回滾流程

```
    健康檢查失敗
         │
         ▼
    ┌──────────────┐
    │ 停止新版本   │
    │              │
    │ .exe → .exe  │
    │ .prev → .exe │
    │ (交換)       │
    │              │
    │ 啟動舊版本   │
    │              │
    │ 健康檢查     │
    └──────┬───────┘
           │
     ┌─────┴─────┐
     │           │
     ▼           ▼
   OK          FAIL
   │           │
   ▼           ▼
  繼續      Telegram 告警
  (跳過      + 人工介入
  此節點)
```

### 9.3 模型同步流程

```
    update-models.sh
          │
          ▼
    讀取 NODE_MODELS
          │
    ┌─────┴──────────────────────────┐
    │  對每台節點:                    │
    │                                │
    │  ollama_url/api/tags           │
    │  → 取得現有模型                 │
    │                                │
    │  比對 NODE_MODELS              │
    │  → 缺少? ollama pull           │
    │  → 多餘? 提示 (不自動刪除)      │
    │                                │
    │  檢查磁碟空間                   │
    │  → < 10GB 警告                 │
    └────────────────────────────────┘
```

---

## 10. 監控與告警

### 10.1 監控項目

| 項目 | 檢查方式 | 頻率 | 告警閾值 |
|------|---------|------|---------|
| Ollama 可達 | curl /api/tags | 60s | 連續 3 次失敗 |
| clawtex-core 健康 | curl /health | 60s | 連續 2 次失敗 |
| 磁碟空間 | df / wmic | 300s | < 10GB |
| 模型列表一致 | ollama list vs models.toml | 3600s | 有差異 |
| 推理延遲 | curl /api/generate + 計時 | 300s | > 30s |

### 10.2 Cron 監控 (Hub 上執行)

```bash
# 加入 Hub 的 crontab (或 clawtex-core 的內建 cron)
# 每 5 分鐘執行健康檢查，失敗時發 Telegram
*/5 * * * * /opt/clawtex/deploy/health-check.sh --quick 2>&1 | grep -q "DOWN" && \
    /opt/clawtex/deploy/health-check.sh | \
    curl -s -X POST "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/sendMessage" \
        -d "chat_id=${TELEGRAM_CHAT_ID}" \
        -d "text=Clawtex 集群告警: 有節點 DOWN"
```

### 10.3 clawtex-core 內建監控 (未來)

在 clawtex-core 中加入 `/cluster/health` endpoint，定期 ping 所有已註冊節點:

```rust
// 未來可在 main.rs 加入的 cluster health 自動檢測
async fn cluster_health_loop(state: AppState) {
    loop {
        for node in state.cluster.status().await {
            let url = format!("http://{}:{}/health", node.host, node.port);
            match reqwest::get(&url).await {
                Ok(resp) if resp.status().is_success() => {
                    state.cluster.update_status(&node.name, "online").await;
                }
                _ => {
                    state.cluster.update_status(&node.name, "offline").await;
                    // 發 Telegram 告警
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}
```

---

## 11. 實施計劃

### 11.1 Phase 1: 基礎設施 (Day 1)

1. 在 Z13 上建立 `deploy/` 目錄結構
2. 寫入 `inventory.sh` (節點清單)
3. 寫入 `health-check.sh` 並測試
4. 確認所有節點 SSH 連通
5. 確認所有節點 Ollama 在線

### 11.2 Phase 2: 編譯與部署 (Day 1-2)

1. 安裝 `cross` 工具
2. 確認 M1 上有 Rust toolchain
3. 寫入 `deploy.sh` 並測試 (先 --build-only)
4. 測試部署到一台 Worker (--node m1)
5. 測試滾動部署到所有 Workers (--workers)
6. 測試完整部署 (包含 Hub)

### 11.3 Phase 3: 配置與模型 (Day 2)

1. 為每台 Worker 建立 agents.toml
2. 寫入 `sync-config.sh` 並測試
3. 定義 `NODE_MODELS`
4. 寫入 `update-models.sh` 並測試
5. 同步所有節點模型

### 11.4 Phase 4: 監控與自動化 (Day 3)

1. 設定 cron 健康檢查
2. 測試 Telegram 告警
3. 寫入 `rollback.sh` 並測試
4. 寫入 `cleanup.sh` 並測試
5. 文件化所有操作手冊

### 11.5 Phase 5: 擴展準備 (Day 3+)

1. 寫入 `setup-node.sh` 並測試
2. 測試加入新節點的完整流程
3. 文件化新節點上線 SOP

---

## 附錄

### A. 常用命令速查

```bash
# 完整部署
./deploy.sh

# 只部署到 Workers
./deploy.sh --workers

# 只部署到 M1
./deploy.sh --node m1

# 只編譯不部署
./deploy.sh --build-only

# 健康檢查
./health-check.sh

# 持續監控 (每 30 秒)
./health-check.sh --watch 30

# 同步模型
./update-models.sh

# 查看模型狀態
./update-models.sh --status

# 回滾指定節點
./rollback.sh ayaneo

# 清理舊版本
./cleanup.sh --keep 3
```

### B. 疑難排解

| 問題 | 原因 | 解決 |
|------|------|------|
| SSH 連不上 Windows | SSH Server 未啟用 | 設定 > 應用 > 選用功能 > OpenSSH Server |
| Ollama 連不上 | OLLAMA_HOST 沒設 | setx OLLAMA_HOST 0.0.0.0 + 重啟 ollama |
| M1 編譯失敗 | Rust 未安裝 | curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh |
| cross 編譯失敗 | Docker 未啟動 | 啟動 Docker Desktop |
| 磁碟空間不足 | 模型太多 | ollama rm <不需要的模型> |
| Windows binary 執行被擋 | Defender | 加入排除路徑: C:\clawtex |

### C. Linux systemd 服務模板

```ini
[Unit]
Description=Clawtex Core - LLM Agent Daemon
After=network.target ollama.service
Wants=ollama.service

[Service]
Type=simple
User=clawtex
Group=clawtex
WorkingDirectory=/opt/clawtex
ExecStart=/opt/clawtex/bin/clawtex-core --host 0.0.0.0 --config /opt/clawtex/config/agents.toml daemon
ExecReload=/bin/kill -HUP $MAINPID
Restart=on-failure
RestartSec=10
StartLimitIntervalSec=60
StartLimitBurst=5

# 環境
Environment=RUST_LOG=info
Environment=HOME=/opt/clawtex

# 安全
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
ReadWritePaths=/opt/clawtex

[Install]
WantedBy=multi-user.target
```

### D. Windows 開機自啟 (Task Scheduler)

```powershell
# 在 Worker Windows 機器上執行 (管理員 PowerShell):
$action = New-ScheduledTaskAction -Execute "C:\clawtex\bin\clawtex-core.exe" `
    -Argument "--host 0.0.0.0 --config C:\clawtex\config\agents.toml daemon" `
    -WorkingDirectory "C:\clawtex"
$trigger = New-ScheduledTaskTrigger -AtStartup
$settings = New-ScheduledTaskSettingsSet -DontStopOnIdleEnd -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1)
Register-ScheduledTask -TaskName "Clawtex Core" -Action $action -Trigger $trigger -Settings $settings -RunLevel Highest
```
