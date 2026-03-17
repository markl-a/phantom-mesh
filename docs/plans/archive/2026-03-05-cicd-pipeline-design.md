# Clawtex-Core CI/CD 管線設計文件

> 日期：2026-03-05
> 狀態：設計完成，待實施
> 作者：CI/CD Pipeline Design

---

## 目錄

1. [現況分析](#1-現況分析)
2. [測試策略](#2-測試策略)
3. [CI/CD Pipeline 架構](#3-cicd-pipeline-架構)
4. [GitHub Actions 配置](#4-github-actions-配置)
5. [Mock 測試設計](#5-mock-測試設計)
6. [自動化部署](#6-自動化部署)
7. [代碼品質門檻](#7-代碼品質門檻)
8. [效能基準與回歸追蹤](#8-效能基準與回歸追蹤)
9. [Secret 管理](#9-secret-管理)
10. [實施時間表](#10-實施時間表)

---

## 1. 現況分析

### 專案規模
- **模組數量**：33 個公開模組（lib.rs re-export 計算）
- **原始碼檔案**：66 個含 `#[cfg(test)]` 的檔案
- **工具數量**：24 個（shell, file_read/write/edit, web_search, http_request 等）
- **Hand 數量**：13 個（lead, outreach, freelancer, seo_content 等）
- **Provider 數量**：7 個（ollama, openai_compat, anthropic, openai, gemini, groq, mock）
- **測試數量**：523 個（458 lib + 65 integration）
- **程式碼行數**：~6,000+ 行

### 目前缺失
- 無 CI/CD 管線
- 無格式化/lint 規範
- 無測試覆蓋率追蹤
- 無效能基準
- 無 Mock Provider（已在本次新增）
- 無自動部署流程

### 硬體環境
| 機器 | 角色 | 規格 | 用途 |
|------|------|------|------|
| Z13 | Hub / CI Runner | Ryzen AI MAX+ 395, 64GB | 主開發機、CI Runner、Production |
| Acer | Worker / Staging | - | Staging 環境 |
| Ayaneo | Worker | - | Worker 節點 |
| NUC | Worker | - | Worker 節點 |

---

## 2. 測試策略

### 2.1 測試金字塔

```
        ╱ E2E Tests ╲           (~10 個)    完整 Hand 執行 (Mock LLM)
       ╱─────────────╲
      ╱ Integration    ╲       (~65 個)    跨模組交互
     ╱─────────────────╲
    ╱   Unit Tests       ╲   (~458 個)    單一模組/函式
   ╱─────────────────────╲
  ╱  Static Analysis       ╲             clippy + rustfmt + cargo-deny
 ╱─────────────────────────╲
```

### 2.2 單元測試（目標覆蓋率）

| 模組 | 目前測試數 | 覆蓋率目標 | 優先級 |
|------|-----------|-----------|--------|
| `providers/traits.rs` | 11 | 90% | P0 |
| `providers/mock.rs` | 10 (新增) | 95% | P0 |
| `providers/router.rs` | ~8 | 80% | P0 |
| `providers/reliable.rs` | ~6 | 85% | P0 |
| `context.rs` | ~5 | 85% | P0 |
| `agent_runtime.rs` | ~10 | 75% | P1 |
| `dispatcher.rs` | ~8 | 85% | P0 |
| `tools/mod.rs` | ~15 | 80% | P1 |
| `tools/shell.rs` | ~5 | 70% | P1 |
| `tools/file_*.rs` | ~10 | 75% | P1 |
| `memory/` | ~12 | 80% | P1 |
| `conversation.rs` | ~6 | 75% | P2 |
| `hands/mod.rs` | ~8 | 75% | P1 |
| `cost_tracker.rs` | ~6 | 80% | P2 |
| `revenue_tracker.rs` | ~6 | 80% | P2 |
| `security/` | ~8 | 90% | P0 |
| `estop.rs` | ~5 | 95% | P0 |
| `skeleton.rs` | ~5 | 70% | P2 |
| **全專案** | **523** | **>=60% (Phase 1), >=75% (Phase 2)** | - |

### 2.3 整合測試（`tests/integration.rs`）

涵蓋的跨模組場景：

```
Provider Router + Context Optimizer + Agent Runtime
├── 路由選擇 → token 計算 → 訊息裁剪 → 呼叫 Provider
├── Fallback: 主 Provider 失敗 → 切換備用 Provider
├── Rate Limiting: 超過限制 → 排隊或拒絕
└── Cost Tracking: 每次呼叫記錄成本

Tool Registry + Agent Runtime + Memory
├── Tool 註冊 → Agent 使用 tool → 結果儲存到 Memory
├── E-Stop: 中途停止 → tool 不執行 → 回傳錯誤
└── Approval Gate: 高風險操作 → 等待核准

Hand Runner + Phase Execution + Tool Dispatch
├── 多 phase 執行 → phase 輸出作為下一 phase 輸入
├── Chain: Hand A 完成 → 自動觸發 Hand B
└── max_rounds 限制 → 防止無限迴圈
```

### 2.4 E2E 測試（完整 Hand 執行，Mock LLM）

```rust
// 範例：使用 MockProvider 測試 content hand 完整流程
#[tokio::test]
async fn test_content_hand_e2e() {
    // 1. 建立 MockProvider（腳本化回應）
    let provider = MockProvider::scripted(vec![
        // Phase 1: research — 呼叫 web_search 後回覆研究結果
        MockResponse::ToolCalls {
            content: String::new(),
            calls: vec![MockToolCall {
                id: "c1".into(),
                name: "web_search".into(),
                arguments: json!({"query": "rust async patterns"}),
            }],
        },
        MockResponse::Text("Research complete: found 5 patterns...".into()),
        // Phase 2: outline
        MockResponse::Text("# Outline\n1. Introduction\n2. Patterns\n3. Conclusion".into()),
        // Phase 3: write
        MockResponse::Text("# Rust Async Patterns\n\nFull article content...".into()),
        // Phase 4: publish
        MockResponse::Text("Published successfully".into()),
    ]);

    // 2. 建立 HandRunner + ToolRegistry（帶 mock tools）
    // 3. 執行 hand
    // 4. 驗證每個 phase 的輸出
    // 5. 驗證 workspace 產出的檔案
}
```

### 2.5 效能測試

使用 `criterion` crate，基準項目：

| 基準 | 描述 | 預期數值 |
|------|------|---------|
| `context_trim_100msg` | 裁剪 100 條訊息 | < 1ms |
| `context_trim_500msg` | 裁剪 500 條訊息 | < 5ms |
| `scrub_credentials` | 清除敏感資訊 | < 100us |
| `tool_dispatch_xml` | XML tool call 解析 | < 50us |
| `memory_store_recall` | 10 筆存取 | < 10ms |
| `conversation_100msg` | 100 條訊息存取 | < 50ms |
| `estop_check` | E-Stop 檢查 (atomic) | < 10ns |

回歸門檻：任何基準超過 **120%** 的基線值即報警。

---

## 3. CI/CD Pipeline 架構

### 3.1 整體流程

```
開發者 Push / PR
        │
        ▼
┌─────────────────────────────────────────┐
│              CI Pipeline                 │
│                                          │
│  fmt ──→ clippy ──→ build ──→ test      │
│           │                    │         │
│           │              ┌────┴────┐    │
│           │              │ coverage │    │
│           │              └────┬────┘    │
│           │                   │         │
│           └───────┬───────────┘         │
│                   │                      │
│            ┌──────┴──────┐              │
│            │  benchmark  │ (main only)  │
│            └──────┬──────┘              │
│                   │                      │
│            ┌──────┴──────┐              │
│            │ docker build│              │
│            └──────┬──────┘              │
│                   │                      │
│            ┌──────┴──────┐              │
│            │ cargo audit │              │
│            └─────────────┘              │
└─────────────────────────────────────────┘
        │ (CI 全部通過)
        ▼
┌─────────────────────────────────────────┐
│           Deploy Pipeline                │
│                                          │
│  Staging (Acer) ──→ E2E ──→ Production  │
│                              (Z13 Hub)   │
│                                  │       │
│                           Workers Update │
│                        (Acer/Ayaneo/NUC) │
└─────────────────────────────────────────┘
```

### 3.2 觸發規則

| 事件 | 觸發的 workflow |
|------|----------------|
| Push to `main` | CI (full) + Deploy to staging |
| Push to `ralph/*` | CI (full) |
| Push to `release/*` | CI (full) + Deploy to staging + production |
| Pull Request to `main` | CI (full, no deploy) |
| Schedule (每天 02:00 UTC) | Nightly (full test + pg feature + benchmark + deps) |
| Manual dispatch | Nightly |

### 3.3 矩陣策略

```yaml
strategy:
  matrix:
    os: [ubuntu-latest, windows-latest, macos-latest]
    rust: [stable]
    include:
      - os: ubuntu-latest
        rust: "1.75.0"  # MSRV
```

---

## 4. GitHub Actions 配置

### 4.1 Workflow 檔案

```
.github/
├── workflows/
│   ├── ci.yml          # 主 CI：fmt → clippy → test → coverage → bench → docker → audit
│   ├── deploy.yml      # 自動部署：staging → e2e → production → workers
│   └── nightly.yml     # 每夜：full test + pg feature + nightly rust + deps check
└── scripts/
    └── setup-self-hosted-runner.sh  # Runner 安裝腳本
```

### 4.2 Self-Hosted Runner

**建議架構：**

| Runner | 位置 | 標籤 | 用途 |
|--------|------|------|------|
| Z13-runner | Z13 本機 | `self-hosted, windows, x64, z13` | 部署觸發、Windows 原生測試 |
| Cloud runners | GitHub-hosted | `ubuntu-latest, macos-latest, windows-latest` | CI 主要測試 |

**為什麼用 GitHub-hosted + Z13 self-hosted 混合：**
- CI 測試用 GitHub-hosted → 不佔 Z13 資源、平行執行
- 部署用 Z13 self-hosted → 直接 SSH 到 staging/production
- Windows 原生 Ollama 整合測試用 Z13 → 有 GPU

**安裝 Self-Hosted Runner（在 Z13）：**
```bash
bash .github/scripts/setup-self-hosted-runner.sh <YOUR_GITHUB_PAT> z13-runner
```

### 4.3 快取策略

```yaml
- uses: Swatinem/rust-cache@v2
  with:
    shared-key: "test-${{ matrix.os }}-${{ matrix.rust }}"
    # 快取 ~/.cargo/registry、~/.cargo/git、target/
    # 依 Cargo.lock hash 作為 key
    # 跨 job 共用 (shared-key)
```

**預期快取效果：**
- 首次編譯：~5-8 分鐘（dependencies）
- 快取命中：~1-2 分鐘（增量編譯）
- 快取大小：~500MB-1GB

### 4.4 Secret 管理

在 GitHub repo Settings → Secrets and variables → Actions 中設定：

| Secret 名稱 | 用途 | 必要性 |
|-------------|------|--------|
| `STAGING_HOST` | Staging 機器 IP/hostname | Deploy |
| `STAGING_SSH_USER` | Staging SSH 使用者 | Deploy |
| `PROD_HOST` | Production 機器 IP/hostname | Deploy |
| `PROD_SSH_USER` | Production SSH 使用者 | Deploy |
| `TELEGRAM_BOT_TOKEN` | 部署通知 | Deploy (optional) |
| `TELEGRAM_CHAT_ID` | 通知目標 chat | Deploy (optional) |
| `CODECOV_TOKEN` | Codecov 上傳 | Coverage (optional) |
| `ACER_HOST` / `AYANEO_HOST` / `NUC_HOST` | Worker 節點位址 | Workers deploy |
| `ACER_SSH_USER` / `AYANEO_SSH_USER` / `NUC_SSH_USER` | Worker SSH 使用者 | Workers deploy |

**SSH Key 設定：**
```bash
# 在 Z13 上生成部署用 SSH key
ssh-keygen -t ed25519 -C "clawtex-deploy" -f ~/.ssh/clawtex_deploy

# 把公鑰加到各 worker 的 authorized_keys
ssh-copy-id -i ~/.ssh/clawtex_deploy.pub user@acer.local
ssh-copy-id -i ~/.ssh/clawtex_deploy.pub user@ayaneo.local
ssh-copy-id -i ~/.ssh/clawtex_deploy.pub user@nuc.local

# 把私鑰加到 GitHub Secrets → SSH_PRIVATE_KEY
```

---

## 5. Mock 測試設計

### 5.1 MockProvider（已實作）

位置：`src/providers/mock.rs`

支援的模式：

```rust
// 1. Echo — 回傳使用者訊息
let provider = MockProvider::echo();

// 2. Fixed — 固定回應
let provider = MockProvider::fixed("I am a helpful assistant");

// 3. Scripted — 腳本化回應（支援 tool call）
let provider = MockProvider::scripted(vec![
    MockResponse::ToolCalls { content: "".into(), calls: vec![...] },
    MockResponse::Text("Final answer".into()),
]);

// 4. Error — 模擬失敗
let provider = MockProvider::error("connection refused");

// 5. 帶延遲（模擬推理時間）
let provider = MockProvider::fixed("slow").with_latency(500);
```

**呼叫記錄（用於斷言）：**
```rust
let provider = MockProvider::echo();
// ... 執行測試 ...
assert_eq!(provider.call_count(), 3);
let record = provider.get_call(0).unwrap();
assert_eq!(record.model, "expected-model");
assert!(record.messages[0].content.contains("expected prompt"));
```

### 5.2 Mock Telegram API

**策略：** 不需要完整的 Telegram mock。Telegram 互動在 `telegram.rs` 中已經被良好封裝。CI 中跳過 Telegram 測試：

```rust
#[tokio::test]
#[cfg_attr(not(feature = "telegram_test"), ignore)]
async fn test_telegram_send_message() {
    // 只在本地手動測試時執行
}
```

如需自動測試 Telegram 邏輯，可使用 HTTP mock server：
```rust
// 未來可用 wiremock 或 httpmock
// let mock_server = MockServer::start().await;
// mock_server.register(Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)));
```

### 5.3 Mock External Tools

**策略：** Tool 執行在 `ToolRegistry::execute()` 中。CI 環境設定 `CLAWTEX_CI=1`，工具行為變更：

```rust
// 在 tool 實作中檢查 CI 環境
fn should_mock() -> bool {
    std::env::var("CLAWTEX_CI").is_ok()
}

// shell tool: CI 中拒絕危險指令
// web_search tool: CI 中回傳假結果
// browser tool: CI 中跳過 Playwright
// email tool: CI 中不真的發信
```

### 5.4 確定性輸出

MockProvider 已經是完全確定性的：
- 無隨機性（不使用 temperature）
- 腳本模式按索引依序回應
- Token 計算基於字串長度（非隨機）
- 呼叫記錄完整保存

---

## 6. 自動化部署

### 6.1 部署流程

```
CI Pass (main/release)
    │
    ▼
┌─ Deploy to Staging (Acer) ─┐
│  1. docker pull             │
│  2. docker compose up       │
│  3. health check (30s)      │
│  4. smoke tests             │
└────────────┬────────────────┘
             │ (smoke pass)
             ▼
┌─ Staging E2E Tests ────────┐
│  1. API endpoint tests      │
│  2. E-Stop lifecycle test   │
│  3. Hand list test          │
│  4. Tool registry test      │
└────────────┬────────────────┘
             │ (e2e pass, release/* only)
             ▼
┌─ Deploy to Production (Z13) ┐
│  1. docker pull              │
│  2. rolling update           │
│  3. health check             │
│  4. Telegram 通知            │
│  5. rollback on failure      │
└────────────┬─────────────────┘
             │ (prod healthy)
             ▼
┌─ Update Workers ────────────┐
│  滾動更新 (max-parallel: 1) │
│  acer → ayaneo → nuc        │
│  每台：pull → up → health   │
└─────────────────────────────┘
```

### 6.2 Rollback 策略

```bash
# 自動 rollback（deploy.yml 已包含）
if ! curl -sf http://localhost:7878/status; then
    docker compose rollback clawtex-core
    # 或者手動：
    # docker compose up -d --force-recreate --image <previous-sha>
fi
```

### 6.3 環境差異

| 設定 | Staging | Production |
|------|---------|------------|
| Docker image tag | `main` | `release-*` / semver |
| 資源限制 | 1G RAM, 1 CPU | 2G RAM, 2 CPU |
| RUST_LOG | `debug` | `info` |
| E-Stop 預設 | 停止 | 運行 |
| Cron jobs | 禁用 | 啟用 |

---

## 7. 代碼品質門檻

### 7.1 rustfmt 配置 (`rustfmt.toml`)

```toml
edition = "2021"
max_width = 100
tab_spaces = 4
imports_granularity = "Module"
group_imports = "StdExternalCrate"
reorder_imports = true
use_field_init_shorthand = true
use_try_shorthand = true
```

### 7.2 Clippy 配置 (`clippy.toml`)

```toml
too-many-arguments-threshold = 10
too-many-lines-threshold = 200
cognitive-complexity-threshold = 30
type-complexity-threshold = 300
```

CI 中的 clippy 等級：
```bash
cargo clippy --all-targets --all-features -- -D warnings -D clippy::all -W clippy::pedantic
```

### 7.3 cargo-deny 配置 (`deny.toml`)

- 授權白名單：MIT, Apache-2.0, BSD-2/3, ISC, Zlib
- 禁止 copyleft 授權
- 禁止已知漏洞
- 警告未維護的 crate

### 7.4 覆蓋率門檻

| 階段 | 最低覆蓋率 | 時間 |
|------|-----------|------|
| Phase 1 (現在) | 60% | 立即 |
| Phase 2 | 70% | +2 週 |
| Phase 3 | 75% | +4 週 |
| Phase 4 (目標) | 80% | +8 週 |

排除項目（不計入覆蓋率）：
- `src/main.rs` — 入口點，依賴外部服務
- `src/telegram.rs` — 需要真實 Bot Token

### 7.5 PR Review Checklist

```markdown
## PR Review Checklist

### 必要
- [ ] `cargo fmt --check` 通過
- [ ] `cargo clippy` 無 warning
- [ ] 所有現有測試通過
- [ ] 新功能有對應的單元測試
- [ ] 沒有引入新的 `unwrap()` (除非有充分理由)
- [ ] 沒有引入硬編碼的密碼/API key

### 建議
- [ ] 整合測試覆蓋新的跨模組交互
- [ ] 效能敏感的變更有 benchmark 數據
- [ ] 文件更新（如果 API 改變）
- [ ] MEMORY.md 更新（如果架構改變）

### 特殊
- [ ] Provider 變更：MockProvider 腳本更新
- [ ] Tool 變更：CI mock 行為更新
- [ ] Hand 變更：E2E 腳本更新
- [ ] 安全相關：security/ 模組的測試特別檢查
```

---

## 8. 效能基準與回歸追蹤

### 8.1 基準項目 (`benches/core_benchmarks.rs`)

已實作的基準組：

1. **context_optimizer** — 訊息裁剪 (10/50/100/500 訊息)
2. **credential_scrubbing** — 敏感資訊清洗 (short/long, clean/mixed)
3. **tool_dispatch** — Tool call 解析 (XML/JSON)
4. **memory_store** — 記憶體存取 (store + recall)
5. **conversation_store** — 對話存取 (100 messages)
6. **estop** — 緊急停止檢查 (atomic operation)

### 8.2 回歸追蹤

使用 `benchmark-action/github-action-benchmark` 自動追蹤：

```yaml
- uses: benchmark-action/github-action-benchmark@v1
  with:
    alert-threshold: '120%'  # 超過 20% 即警報
    fail-on-alert: true       # 回歸直接失敗
    comment-on-alert: true    # PR 上留言
```

基準數據存在 `gh-pages` 分支的 `dev/bench` 目錄，可透過 GitHub Pages 查看圖表。

### 8.3 本地執行

```bash
# 執行所有基準
cargo bench

# 執行特定基準組
cargo bench -- context_optimizer

# 查看 HTML 報告
open target/criterion/report/index.html
```

---

## 9. Secret 管理

### 9.1 GitHub Actions Secrets

```
Repository Settings → Secrets and variables → Actions

Secrets:
├── STAGING_HOST          = "192.168.x.x"
├── STAGING_SSH_USER      = "deploy"
├── PROD_HOST             = "192.168.x.x"
├── PROD_SSH_USER         = "deploy"
├── TELEGRAM_BOT_TOKEN    = "123456:ABC..."
├── TELEGRAM_CHAT_ID      = "-100..."
├── SSH_PRIVATE_KEY        = "-----BEGIN..."
├── ACER_HOST             = "192.168.x.x"
├── ACER_SSH_USER         = "deploy"
├── AYANEO_HOST           = "192.168.x.x"
├── AYANEO_SSH_USER       = "deploy"
├── NUC_HOST              = "192.168.x.x"
└── NUC_SSH_USER          = "deploy"

Variables:
├── RUST_LOG              = "info"
└── DOCKER_REGISTRY       = "ghcr.io/markl-a/clawtex"
```

### 9.2 本地開發 Secrets

使用 clawtex-core 自帶的加密機制（ChaCha20-Poly1305）：
```bash
clawtex-core encrypt-secret "my-api-key-value"
# 輸出: enc2:abcdef1234...
# 貼到 agents.toml 中
```

### 9.3 CI 環境變數

```yaml
env:
  CLAWTEX_CI: "1"           # 標記 CI 環境，tool mock 啟用
  RUST_BACKTRACE: 1          # 錯誤追蹤
  CARGO_TERM_COLOR: always   # 彩色輸出
  RUSTFLAGS: "-D warnings"   # 警告即錯誤
```

---

## 10. 實施時間表

### Phase 1：基礎 CI（1-2 天）

- [x] 建立 `.github/workflows/ci.yml`
- [x] 建立 `rustfmt.toml` + `clippy.toml`
- [x] 建立 `deny.toml`
- [x] 建立 MockProvider (`src/providers/mock.rs`)
- [x] 建立基準測試 (`benches/core_benchmarks.rs`)
- [x] 更新 `Cargo.toml`（criterion + bench 配置）
- [ ] 初始化 GitHub repo
- [ ] 推送程式碼
- [ ] 驗證 CI 流程

### Phase 2：部署自動化（2-3 天）

- [x] 建立 `.github/workflows/deploy.yml`
- [x] 建立 self-hosted runner 安裝腳本
- [ ] 在 Z13 安裝 self-hosted runner
- [ ] 設定 SSH keys + GitHub Secrets
- [ ] 在 Acer 上建立 staging 環境
- [ ] 驗證 staging 部署
- [ ] 驗證 production 部署

### Phase 3：品質提升（1 週）

- [x] 建立 `.github/workflows/nightly.yml`
- [ ] 補齊測試覆蓋率到 60%
- [ ] 啟用 Codecov
- [ ] 建立 GitHub Pages 效能追蹤頁面
- [ ] 為所有 tool 添加 CI mock 行為
- [ ] 添加 E2E 測試（使用 MockProvider）

### Phase 4：進階功能（2 週）

- [ ] 覆蓋率提升到 75%
- [ ] 添加更多基準（Provider routing, Hand execution）
- [ ] 設定 PR 自動 review（cargo-semver-checks）
- [ ] 設定 Dependabot
- [ ] 添加 release 自動化（changelog, tag, publish）

---

## 附錄 A：快速啟動指南

### 首次設定

```bash
# 1. 安裝工具
rustup component add rustfmt clippy
cargo install cargo-tarpaulin cargo-criterion cargo-deny cargo-audit

# 2. 本地驗證 CI 步驟
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo bench
cargo deny check
cargo audit

# 3. 推送到 GitHub
git init
git remote add origin https://github.com/markl-a/Clawtex.git
git add .
git commit -m "initial: clawtex-core with CI/CD pipeline"
git push -u origin main
```

### 日常開發

```bash
# 提交前檢查（模擬 CI）
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test

# 效能測試
cargo bench

# 覆蓋率報告
cargo tarpaulin --out html && open tarpaulin-report.html
```

## 附錄 B：故障排除

| 問題 | 解法 |
|------|------|
| Windows 上 `LNK1104` 鏈接錯誤 | `cargo clean` 或殺掉佔用測試 exe 的進程 |
| `Instant::now() - Duration` 溢出 | 使用 `checked_sub` |
| SQLite `:memory:` 多連線問題 | 用 `tempfile` |
| CI 上 Clippy pedantic 太嚴 | 在特定函式加 `#[allow(clippy::xxx)]` |
| 基準波動大 | 用 `--measurement-time 10` 增加測量時間 |
| Docker build 太慢 | 確保 layer cache 命中（先 COPY Cargo.toml） |
