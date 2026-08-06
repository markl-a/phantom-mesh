# 變更日誌（Changelog）

spectyn-mesh 的所有重要變更都記錄於此。

格式依據 [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)（保持變更日誌規範）。

## [未發布（Unreleased）]

### 安全性（C9 / T78 — V10 HIGH-5，2026-05-16）

- **HIGH（高風險）** `scripts/windows-bootstrap.ps1`：移除了硬編碼（hardcoded，寫死在程式中）的 `ssh-ed25519` 公鑰，該公鑰原本在每次 `irm | iex` 執行時都會被悄悄寫入 `C:\ProgramData\ssh\administrators_authorized_keys`。SSH 金鑰安裝現在改為透過新的 `-AddSshKey "<pubkey>"` 參數選擇性啟用（opt-in），該參數會發出醒目的 `Write-Warning`（警告訊息）說明如何移除金鑰。預設行為不安裝任何金鑰。

### 安全性（T7b — Claude 完整稽核 2026-05-15）

- **CRITICAL（嚴重）** `core/src/main.rs::agent_run`、`agent_run_async`：現在要求 `X-Cluster-Auth` HMAC（雜湊訊息驗證碼）（T13-N1）。
- **CRITICAL（嚴重）** `core/src/serve.rs::mcp_http`（`POST /mcp`）：現在要求 HMAC；封堵了透過未驗證 `tools/call` 造成的 RCE（遠端程式碼執行）（T13-N2）。
- **HIGH（高風險）** `core/src/serve.rs::ws_upgrade`（`GET /ws`）：現在在升級（upgrade）時要求 HMAC（T13-N3）。
- **HIGH（高風險）** `core/src/serve.rs::api_onboarding`（`POST /api/onboarding`）：現在要求 HMAC；封堵了 CORS（跨來源資源共用）寬鬆設定造成的設定寫入漏洞（T13-N4）。
- **HIGH（高風險）** `core/src/serve.rs::onboarding_token` + `onboarding_config`：現在要求 HMAC；封堵了憑證外洩（credential exfil）（T13-N5）。
- **HIGH（高風險）** `core/src/tools/web_fetch.rs`、`core/src/tools/http_client.rs::{get,post}`：新增 SSRF（伺服器端請求偽造）防護；設定 `SPECTYN_FETCH_ALLOW_LOCAL=1` 可重新允許（opt back in）（T13-N6）。
- **HIGH（高風險）** `core/src/tools/fs.rs::rename_file`：`dst`（目的路徑）現在會經由工作區綁定（workspace-bound）的 `safe_path` 路由（T13-N7）。
- **HIGH（高風險）** `core/src/main.rs::conversation_reset`、`workspaces_rename`、`workspaces_add_tag`、`tasks_cancel`、`tasks_resume`：現在要求 HMAC（T13-N1 後續）。
- 新增共用模組 `core/src/auth_gate.rs`，將 `require_cluster_auth` 抽出，讓 `spectyn serve` 路由器（router）與守護程序（daemon）路由器共用同一份 HMAC 檢查。

## [0.1.0-alpha] - 2026-05-01

### 新增 — 2026-04-27 衝刺（sprint）（REPL 使用體驗、web 前端、TUI、自我演化）

**REPL 使用體驗（UX）**
- 狀態列顯示 agent（代理）/ cost（成本）/ session（工作階段）/ `· PLAN` 模式
- `/cmd` 與 `@path/to/...` 的 Tab 自動補全
- Ctrl-C 取消進行中的 LLM（大型語言模型）串流（REPL 仍保持運作）
- 串流中的 Markdown 渲染：項目符號、編號清單、引用區塊、連結、行內 code span、圍欄式程式碼區塊（fenced blocks）
- 斜線指令（Slash commands）：`/show <n>`（展開已擷取的工具輸出）、`/perm ask|allow|deny|list|reset`、`/density compact|full`、`/theme <name>`、`/resume <prefix>`、`/plan`（真正的閘控 gating — 在輸入 "go" 前拒絕工具）、`/agent`、`/agents`、`/todo`
- 透過結尾 `\` 的多行輸入
- `@image.png` 將 PNG/JPG 作為多模態（multimodal）`image_url` 附加（OpenAI / Gemini / Anthropic 自動塑形）；一次性（one-shot）模式也會展開 `@file`

**Web 前端**
- xterm.js 終端機面板，支援 ANSI 串流
- **Cmd+K**（Ctrl+K）指令面板（Terminal / Tasks / Sessions / Cost / Settings / Help / Reload）
- Info 分頁子面板：Todo、Sessions、Cost、**Tools**（已擷取的工具呼叫歷史）
- 側邊欄即時的對等節點 ping 點（peer-ping dots）（綠 / 紅 / 灰）
- `@image` 多模態附加在瀏覽器終端機中也可使用

**工具（Tools）** — 總數現為 45（+5）
- `web_fetch` — HTML → 純文字
- `bash_run_background`、`bash_output`、`bash_kill` — 長時間執行的 shell 控制代碼（handles）
- `ask_user` — 暫停 agent，透過 stdin 向人類提問

**TUI（終端機使用者介面）** — `spectyn tui` 開啟一個全螢幕 ratatui 介面（持續存在的輸入框、可捲動的對話記錄、狀態列、斜線指令）

**自我迭代（Self-iteration）** — `spectyn evolve` 已在此 repo 上端到端（end-to-end）驗證：在 Groq 免費方案（free tier）以 $0 成本自主修正了一個 `core/src/cost.rs` 警告（見 `docs/SELF-EVOLVE.md`）

### 修正 — 2026-04-27
- B1：agent 模型回退（fallback）現在會在路由省略模型時尊重各供應商（per-provider）的預設值
- B2：opencode 模型名稱正規化（normalization）
- `max_tokens` 預設值由 256 → 4096（解除對推理模型的阻擋）
- `AGENTS.md` 與 `SPECTYN.md` 一起自工作目錄自動載入

### 新增 — agent 執行時（runtime）
- 多 LLM 供應商回退（Anthropic、OpenCode、OpenAI 相容、Gemini、Groq、Ollama）
- 透過 `spectyn mcp` 提供的 45 個 MCP 協議工具（讀取、寫入、shell、grep、fetch、硬體掃描、鷹架 scaffold、mesh 操作等）
- 30 輪的 agentic 工具呼叫迴圈，具備停滯偵測（stall detection）
- 具 token 感知的上下文壓縮（context compaction）（預設 80K token 預算）
- 即時成本追蹤，含各模型（per-model）價格表

### 新增 — 介面（第 2 天 + 第 3 天衝刺）
- **Claude Code 風格的 REPL**（`spectyn`）：
  - rustyline 編輯器，含 12 個斜線指令（/help /clear /exit /add /cost /tools /sessions /session /list /init /model /compact）
  - **串流輸出**，含行內工具呼叫（`● tool(args) … ✓ result`）
  - 透過結尾 `\` 接續的**多行輸入**
  - `@<path>` 檔案內嵌（inlining）
  - 精緻的歡迎橫幅（welcome banner），顯示供應商數量、cluster（叢集）對等節點、agent、session、目錄
- **首次執行的終端機上手精靈（onboarding wizard）** — 當找不到 agents.toml 時自動提示
- **瀏覽器式上手（onboarding）**（`spectyn onboarding`）— 自動啟動 serve、開啟瀏覽器至設定頁、等待設定寫入
- **內嵌式 web 儀表板**（`spectyn serve` → `http://localhost:7878`）：
  - 單頁應用程式（Single-page app）：頂部狀態列、含 cluster 節點的側邊欄、分頁式主面板（Terminal | Tasks | Settings）
  - 透過 Server-Sent Events（伺服器推送事件，SSE）的串流聊天
  - 設定頁 = web 上手表單，含儲存時合併（merge-on-save）（保留 cluster 對等節點 + agent 定義）
- **MCP stdio 伺服器**（`spectyn mcp`） — 可隨插即用（drop-in）的 subagent（子代理），適用於 Claude Code、Cursor、任何 MCP 客戶端
- **Codex 相容的 WebSocket JSON-RPC**（`spectyn serve` → `ws://host:7878/ws`）

### 新增 — mesh（網狀網路）
- 透過 HTTP cluster 的 P2P（點對點）運算 mesh，採用 SHA-256 HMAC 驗證
- 用於跨節點任務指派的非同步工作佇列（async job queue）（POST /rpc/task/assign → job_id 輪詢 polling）
- `spectyn evolve --distributed` 跨 cluster 的平行 agent 群集（swarm）
- `spectyn coordinator` 透過 mDNS 的零設定（zero-config）對等節點探索
- 持久化的對話歷史（每個 chat_id 一份 JSONL）

### 新增 — 整合（integrations）
- Telegram 機器人頻道，含使用者允許清單（allowlist）
- SPECTYN.md 與 AGENTS.md 專案上下文檔案（自工作目錄自動載入）
- Tauri 桌面應用程式（macOS、Linux、Windows），採用 React 前端
- Tauri Android 應用程式（前景服務工作者 foreground service worker）
- Tauri iOS 應用程式（透過免費 Apple 開發者憑證側載 sideload）
- 跨平台二進位檔（Mac arm64、Win x86_64、Linux arm64/x86_64、Android arm64/armv7/i686/x86_64、iOS arm64）

### 安全性
- Shell 指令封鎖清單（blocklist）（rm -rf /、fork 炸彈、curl|sh 等）
- 路徑遍歷（path traversal）防護（safe_path() 正規化 canonicalization）
- 常數時間（constant-time）HMAC 比對（subtle crate）
- 要求 cluster 驗證（無預設密鑰回退 no default secret fallback）
- Telegram allowed_users 強制執行

### 基礎設施（Infrastructure）
- 適用所有平台的 GitHub Actions CI（持續整合）（`ci-fast` / `ci-medium` / `ci-desktop` / `ci-mobile`）
- 守護程序（daemon）、桌面、行動裝置的發布工作流程（`release-daemon` / `release-desktop` / `release-mobile`）
- 透過 `cross` 的交叉編譯（cross-compilation）（Cross.toml：Android aarch64/armv7、Windows x86_64、Linux aarch64/armv7/x86_64）
- Tailscale mesh 設定腳本
- 整合測試 + 冒煙測試（smoke test）套件（`scripts/integration-test.sh`、`scripts/smoke-test.sh`）
- 開源前檢查清單（`scripts/pre-open-source-checklist.sh`）
- 用於密鑰掃描的 gitleaks pre-commit hook（提交前掛鉤）
