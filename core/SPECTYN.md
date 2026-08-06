# Project: spectyn-mesh (core)

## 概觀（Overview）

spectyn-mesh 是一套用 Rust 寫成的跨平台 AI agent（智能代理）mesh（網狀網路）。一個
長時間運行的 daemon（常駐服務，`core/`）在 `:7878` 上開放一組 HTTP API；agents 對一個
或多個 LLM（大型語言模型）provider（供應商，Anthropic、OpenAI-compat、Gemini）執行
工具增強的 LLM 迴圈，並具備自動 fallback（備援切換）。多個 node（節點）透過 Tailscale
連線，組成一個 P2P（點對點）運算 mesh。一個 Tauri + React 桌面應用程式（`app/`）以及一個
Telegram bot（機器人）channel（管道）構成其餘的對外介面。

設定檔位於 `~/.spectyn-mesh/agents.toml`（完整參考請見 `agents.toml.example`）。

## 建置與測試（Build & Test）

```bash
# Compile-check (fast; run this after every .rs edit)
cargo check --manifest-path core/Cargo.toml

# Full build
cargo build --manifest-path core/Cargo.toml

# Release build
cargo build --release --manifest-path core/Cargo.toml

# Run tests
cargo test --manifest-path core/Cargo.toml

# Run the REPL binary directly
cargo run --manifest-path core/Cargo.toml --bin spectyn-mesh
```

編輯任何 `.rs` 檔案後都務必執行 `cargo check`。本專案有一套小型測試套件；`cargo test` 是
提交（commit）前的把關閘門。

## 關鍵檔案（Key Files）

```
core/
  Cargo.toml                   — package manifest; add dependencies here
  src/
    main.rs                    — Axum HTTP server entry point; defines build_router()
    lib.rs                     — AppState, re-exports, JobStore; public API surface
    agent.rs                   — AgentRuntime, AgentEvent, the tool-call loop
    config.rs                  — AgentsConfig, AgentEntry, ProviderEntry, ToolsConfig
    context.rs                 — WorkspaceContext (cwd, git root, SPECTYN.md loader)
    cost.rs                    — CostTracker (token + USD accounting)
    session.rs                 — ConversationStore (JSONL persistence)
    streaming.rs               — SSE / streaming helpers
    scaffold.rs                — `spectyn init` SPECTYN.md generator
    project_context.rs         — SPECTYN.md / .spectyn-mesh/context.md loader
    mesh.rs                    — ClusterManager, PeerStatus, HMAC auth
    hardware.rs                — System hardware scan
    oauth.rs                   — OAuth2 Google/Apple (partial)
    bin/
      spectyn.rs               — `spectyn` CLI entry point (REPL + one-shot mode)
    channels/
      telegram.rs              — Long-poll Telegram bot channel
    providers/
      traits.rs                — ChatProvider trait, ChatMessage
      anthropic.rs             — Anthropic Claude
      openai.rs                — OpenAI-compatible (OpenRouter, Groq, XAI, Ollama)
      gemini.rs                — Google Gemini
      claude_cli.rs            — Claude CLI bridge provider
      credential_scanner.rs    — Scan env for API keys
    tools/
      mod.rs                   — Tool registry: execute() dispatch + schema() definitions
      shell.rs                 — shell — run arbitrary commands (with blocklist)
      file.rs                  — file_read, file_write, file_edit
      search.rs                — content_search (ripgrep), glob_search
      web.rs                   — web_search (Brave API + DuckDuckGo fallback)
      memory.rs                — memory_store, memory_recall (~/.spectyn-mesh/memory.json)
      git.rs                   — git_status, git_diff, git_log, git_commit
      fetch.rs                 — HTTP fetch tool
      fs.rs                    — Extended filesystem helpers
      ls.rs                    — Directory listing
      diff_view.rs             — Diff rendering
      patch.rs                 — Patch apply
      multi_edit.rs            — Batch file edits
      task.rs                  — Subtask spawning helpers
      diagnostic.rs            — Self-diagnostic tool
      http_client.rs           — Shared HTTP client utilities
```

## 架構決策（Architecture Decisions）

### 工具層（Tool Layer）

所有工具的派發（dispatch）都流經 `core/src/tools/mod.rs`：

- `execute(name, args, config)` — 透過對工具名稱做 `match`（模式比對）的非同步派發
- `schema(name)` — 回傳每個工具的 JSON Schema（送給 LLM）

**新增工具時：**
1. 建立 `core/src/tools/<toolname>.rs`，內含一個非同步處理函式（handler function）。
2. 在 `tools/mod.rs` 中以 `pub mod <toolname>;` 宣告它。
3. 在 `execute()` 函式中加入一個 match 分支（match arm）。
4. 加入一個 `schema()` match 分支，回傳合法的 JSON Schema 物件。
5. 執行 `cargo check` 驗證。

若沒有同時更新 `execute()` 與 `schema()`，會導致該工具悄悄無法被觸及，或對 LLM 不可見。

### Agent 迴圈（Agent Loop）

`agent.rs` 中的 `AgentRuntime` 驅動工具呼叫（tool-call）迴圈：

- `MAX_ROUNDS = 20` — 每次請求中工具呼叫迭代次數的硬上限
- `STALL_THRESHOLD = 2` — 連續幾輪沒有任何工具呼叫後迴圈即退出
- `TOKEN_BUDGET = 60_000` — context（上下文）壓縮（compaction）的觸發門檻
- Provider fallback（供應商備援）：發生 HTTP 錯誤 / 429 / 503 時，由主要供應商切換至設定順序中的下一個
- `MAX_RETRIES = 3` — 每個供應商在切換備援前的重試次數

### Provider 抽象層（Provider Abstraction）

`providers/traits.rs` 中的 `ChatProvider` trait（特徵）。每個 provider 都實作它。
`AgentsConfig.providers` 是一個 `HashMap<String, ProviderEntry>`；agents 以名稱參照
provider。Provider 類型由 `ProviderEntry.provider_type` 決定
（`"anthropic"`、`"openai_compat"`、`"gemini"`）。

### AppState

`AppState` 是 `Clone`（可複製）的 — 每個欄位都必須是 `Arc<…>` 或實作 `Clone`。
這是必要的，因為 Axum 以傳值（by value）方式將 state（狀態）傳給 handler（處理器）。

### Session 持久化（Session Persistence）

`ConversationStore` 將對話以 JSONL 檔案的形式寫入
`~/.spectyn-mesh/conversations/<session_id>.jsonl` 之下。該 store（儲存器）對 `JobStore`
強制執行 500 個工作（job）的逐出（eviction）上限。

### HTTP API

路由（route）定義於 `core/src/main.rs` 的 `build_router()` 中。
所有路由都前綴於 `:7878` 之下。前端 proxy（代理）在 `app/vite.config.ts` 中設定，
將請求轉發至 `http://localhost:7878`。

### P2P Mesh 認證（P2P Mesh Auth）

Cluster（叢集）node 透過 `X-Cluster-Auth: sha256(cluster_secret + body)` 進行認證。
Peer（對等節點）列在 `agents.toml` 的 `[cluster]` 中。
`POST /rpc/task/assign` 是非同步的 — 會立即回傳 `job_id`。
`GET /rpc/task/status/:job_id` 用於輪詢（poll）完成狀態。

## 程式撰寫慣例（Coding Conventions）

- **Commit 風格：** Conventional commits（慣例式提交）— `feat:`、`fix:`、`chore:`、`docs:`、`refactor:`
- **檔案編輯：** 修改既有檔案時，使用 `file_edit`（精確字串替換），而非 `file_write`
- **Cargo 相依套件：** 編輯 `core/Cargo.toml`，接著以 `cargo check` 驗證
- **暫存（Staging）：** 絕不使用 `git commit -am`；以 `git add <path>` 暫存特定檔案
- **不留孤兒工具呼叫：** 不要在工具呼叫進行到一半時結束對話回合
- **每個模組只負責一件事：** 將 provider、tool、channel 各自放在獨立檔案中

## 已知陷阱（Known Gotchas）

- 新增工具時，`tools/mod.rs` 必須在 `execute()` 與 `schema()` 兩處都更新 — 缺其一會造成靜默失敗或工具不可見。
- `AppState` 必須維持 `Clone`；將新 state 包進 `Arc<TokioRwLock<_>>` 是標準作法。
- `cargo test` 是把關閘門；`cargo check` 快速但不執行測試。
- `bin/spectyn.rs` 中的 `spectyn init` 子命令是一個 TODO stub（待辦樁）— 必須先把 `scaffold.rs` 接上才會運作。
- API 金鑰：設定中一律使用 `api_key_env = "VAR_NAME"`；絕不在 `agents.toml` 中內嵌金鑰。
- P2P RPC 需要 `cluster_secret`；沒有它的 node 會拒絕所有傳入的 cluster 認證。
- 在正式環境部署中，必須設定 Telegram 設定裡的 `allowed_users`。
- Provider 類型 `"anthropic"` 會自動加上 `anthropic-version` 標頭（header）；`"openai_compat"` 則不會。
- Context 壓縮在 `TOKEN_BUDGET = 60_000` token 時觸發；壓縮期間不可讓 tool_call 訊息變成孤兒（assistant 的 tool_call 與其對應的 tool 結果必須成對出現）。

## 測試策略（Testing Strategy）

- 單元測試位於 `core/tests/`（整合測試）以及行內的 `#[cfg(test)]` 模組中。
- 任何提交前的主要把關閘門是 `cargo test --manifest-path core/Cargo.toml`。
- 工具變更時，提交前先透過 REPL（`cargo run --bin spectyn-mesh`）手動操作該工具。
- Provider 變更時，使用一次性提示（one-shot prompt）：在設定好目標 provider 的情況下執行 `cargo run --bin spectyn-mesh "hello"`。
- Mesh / RPC 變更時，使用 `scripts/` 中的冒煙測試（smoke-test）腳本。

## 安全注意事項（Security Notes）

- 設定中使用 `api_key_env`，絕不內嵌金鑰。
- `cluster_secret` 為必填；在每個 node 的 `[cluster]` 中設定它。
- 在公開對外暴露 bot 之前，先設定 Telegram 設定中的 `allowed_users`。
- `shell` 工具有一套針對危險命令樣式的內部封鎖清單（blocklist）— 不要移除它。
- `[core]` 中的 `hub_api_key` 會啟用 HTTP API 上的 bearer-token（持有者權杖）認證；建議任何對外暴露的 node 都啟用。
