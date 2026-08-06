# spectyn-mesh v0.1.0-alpha

## 未發行（Unreleased）

### 破壞性變更 — `/rpc/*` 與 `/api/chat` 現在需要 `cluster_secret`

三個端點（`/api/chat`、`/rpc/message`、`/rpc/task/assign`）先前在
`agents.toml` 中的 `[cluster].cluster_secret` 為空或缺漏時，會接受未經
驗證（unauthenticated，未通過身分驗證）的請求。它們現在會以
`403 Forbidden` 拒絕。設定 `cluster_secret` 來完成遷移，或設定
`SPECTYN_ALLOW_EMPTY_CLUSTER_SECRET=1` 以在「一個版本內」恢復舊有
（legacy，沿用既往）行為 — 這個環境變數（env-var）覆寫項目將在下一個
次要版本（minor）中移除。

儀表板（dashboard）上的 CORS（跨來源資源共用）也從「任何來源」
（`CorsLayer::permissive()`）收緊為同源（same-origin）。在遷移期間
設定 `SPECTYN_CORS_ALLOW_ANY=1` 以恢復舊有的寬鬆行為；同樣是一個版本
後即停用（sunset，落日退場）。

這兩個覆寫項目都會在每次 `spectyn serve` 啟動時記錄一行
`SECURITY WARNING:`，讓維運人員（operator）一眼就能看出他們的節點
（node）是否仍走在舊有路徑上。

來源：codex 安全稽核（security audit）2026-05-15（軌道 T7 — 6 個 HIGH
＋ 1 個 MEDIUM）。

### 破壞性變更 — 另外 11 個端點現在需要 `cluster_secret`（T7b）

繼 Claude 的完整安全稽核（軌道 T7b，2026-05-15）之後，另外被標記為
CRITICAL（嚴重）或 HIGH（高）的十一個端點，現在會拒絕未經驗證的
請求：

- `POST /agent/:name/run`、`/agent/:name/run-async`（daemon，常駐服務，T13-N1 CRITICAL）
- `POST /mcp`（`tools/call`，T13-N2 CRITICAL）
- `GET /ws` 升級（T13-N3 HIGH）
- `POST /api/onboarding`（T13-N4 HIGH）
- `GET /onboarding/token` ＋ `GET /onboarding/config`（T13-N5 HIGH）
- `POST /conversations/:cid/reset`、`/workspaces/:id/name`、
  `/workspaces/:id/tags`、`/tasks/:id/cancel`、`/tasks/:id/resume`
  （T13-N1 後續）

**單一版本遷移逃生口（escape hatch）：** 設定
`SPECTYN_ALLOW_EMPTY_CLUSTER_SECRET=1`（與 T7 相同的環境變數），常駐服務
會退回（fall back）到舊有的未驗證行為，並在啟動時發出明顯的警告。這個
覆寫項目將在 v0.6.0 中被移除。

**受影響的用戶端（clients）：** 內建於程式碼樹（in-tree）的網頁 UI
（`core/web/app.js`）、行動裝置 worker（工作節點）外殼、codex 相容的 WS
用戶端、MCP-over-HTTP 呼叫端。每一個都需要一行更新，以傳遞
`X-Cluster-Auth`（對請求主體以 `cluster_secret` 為金鑰計算的
HMAC-SHA256；GET 與 WS 升級的正規（canonical）主體為空位元組字串）。

### 破壞性變更 — SSRF 防護擴及全部 3 個 fetch 工具（T7b T13-N6）

`web_fetch`、`http_get`、`http_post` 現在會拒絕回送位址（loopback）／
私有 IPv4（10/8、172.16-31/12、192.168/16、169.254/16）與私有 IPv6
（`::1`、`fc00::/7`、`fe80::/10`）的 URL。設定
`SPECTYN_FETCH_ALLOW_LOCAL=1` 以允許它們（這個環境變數已隨
`tools/fetch.rs` 一起出貨，未有變動）。

### 破壞性變更 — `rename_file` 的 `dst` 現在受工作區（workspace）綁定（T7b T13-N7）

`tools/fs::rename_file` 先前將 `src` 綁定於工作區（T7），但 `dst` 是一個
無界限（unbounded）的 `PathBuf::from`。這個工具可能將工作區檔案移動到
磁碟上的任何位置。`dst` 現在會經過與 `src` 相同的 `safe_path` 輔助函式
路由；跨工作區的移動需要 `SPECTYN_EXTRA_ALLOWED_ROOTS` 包含目的地根目錄。

那 4 個 MEDIUM（中）等級的發現（T13-N8 `bash_run_background` 黑名單
繞過、T13-N9 缺少 `RequestBodyLimitLayer`、T13-N10 git 的 `-` 前綴參數
注入、T13-N11 `download_binary` 無界限主體）被追蹤為 v0.6.0 的後續議題。

來源：Claude 完整安全稽核 2026-05-15（軌道 T7b — 2 個 CRITICAL ＋
5 個 HIGH）。

---

> 一個開源 AI 代理（agent）執行環境（runtime）— Claude Code 風格的 REPL
> （讀取-求值-輸出迴圈）＋ MCP/WS 子代理（subagent）＋ 內嵌網頁儀表板，
> 設計用來在你所有的裝置上以網狀網路（mesh，網狀拓樸）運行。

## 它是什麼？

`spectyn` 是一個單一的 Rust 二進位檔，提供你：

- 在終端機中的**對話式 REPL**（Claude Code／Codex 風格）
- 一個 **MCP 伺服器** — 可直接套用（drop-in）的子代理，供 Claude Code、
  Cursor 或任何 MCP 主機（host）使用
- 一個 **WebSocket JSON-RPC 常駐服務（daemon）** — Codex 相容的用戶端介面
- 一個位於 `http://localhost:7878` 的**內嵌網頁儀表板** — 節點狀態、
  終端機、設定 — 可從網路上任何裝置存取
- 一個**網狀網路（mesh）** — 多個 `spectyn` 節點彼此發現並委派
  （delegate）工作

## 它有何不同？

不同於單機型的代理 CLI（命令列介面）：

- **跨你所有裝置運行** — macOS、Linux（x86_64/arm64）、Windows、
  Android（Tauri 前景服務）、iOS（Tauri 側載 sideload）
- **多供應商容錯切換（fallback）** — Groq、Gemini、OpenCode、Anthropic、
  OpenAI 相容、Ollama；在達到速率上限（rate-limit）／發生錯誤時自動容錯
  移轉（failover）
- **P2P 計算網狀網路** — 透過 HTTP+HMAC、非同步工作佇列（job queue）、
  最少負載（least-loaded）路由，將任務分散到各節點
- **本地優先（local-first）** — 無遙測（telemetry）；所有資料都在
  `~/.spectyn-mesh/` 內
- **既是子代理也是獨立程式** — 透過 MCP 內嵌進 Claude Code，或將它當作
  你自己的主要代理來運行

## 快速開始

```bash
# 1. Download a binary for your platform from the Releases page, then:
./spectyn              # terminal walks you through provider setup
                       # → drops into REPL with welcome banner

# Or open the web onboarding:
./spectyn onboarding   # spawns serve, opens browser to settings page

# Subagent for Claude Code: add to ~/.claude.json
#   "mcpServers": { "spectyn": { "command": "spectyn", "args": ["mcp"] } }
```

完整指南：[docs/INTEGRATIONS.md](docs/INTEGRATIONS.md) ·
快速上手：[docs/QUICKSTART.md](docs/QUICKSTART.md)

## 這個版本包含什麼

- `spectyn` — 具備串流、多行輸入、斜線指令、行內工具呼叫的 REPL
- `spectyn mcp` — MCP stdio 伺服器（40 個工具）
- `spectyn serve` — WebSocket ＋ 內嵌網頁儀表板（`http://host:7878`）
- `spectyn onboarding` — 以瀏覽器為基礎的設定精靈（wizard）
- `spectyn evolve --distributed` — 跨網狀網路的平行代理群（agent swarm）
- `spectyn coordinator` — 透過 mDNS 的零設定（zero-config）對等節點發現
- `spectyn swarm` / `spectyn peer` — 叢集（cluster）委派工具

## 平台

v0.1.0-alpha 的預建（pre-built）二進位檔：

- **macOS arm64**（`spectyn-aarch64-apple-darwin`）
- **Windows x86_64**（`spectyn-x86_64-pc-windows.exe`）
- **Linux arm64**（`spectyn-aarch64-unknown-linux`）
- **Android arm64**（`spectyn-aarch64-linux-android`）
- **iOS** — Tauri IPA（以免費 Apple 開發者憑證側載）

其他平台（Linux x86_64、Android armv7、Windows arm64）— 請以
`cargo build --release --target <triple>` 從原始碼建置。

## 已知限制（alpha）

- iOS 側載需要免費的 Apple 開發者憑證（每 7 天重新簽署一次）
- Windows 的 Groq 串流有一個每區塊（per-chunk）30 秒逾時的暫行解法
- Gemini 免費方案有每日配額（quota）限制
- 網頁儀表板的 Tasks（任務）分頁是一個佔位（placeholder，預留位置）（v0.2）
- Tauri 桌面應用程式的 React UI 與網頁儀表板是分開的；統一化列在 v0.2
  的藍圖（roadmap）上

## 接下來是什麼（v0.2）

- iOS Tauri 應用程式載入與 Mac/Win/Android 相同的網頁儀表板
- 網頁儀表板內的 xterm.js 終端機面板
- 網頁 Tasks 分頁 — 即時任務佇列＋歷史紀錄
- 用於工具隔離的 WASM 沙箱（sandbox）
- 工具權限提示（允許一次／永遠允許）
- REPL 輸出中的 Markdown 渲染
- MCP 用戶端模式（consume，取用其他 MCP 伺服器）

## 貢獻

請見 [CONTRIBUTING.md](CONTRIBUTING.md)。歡迎提出 Issue 與 PR。

## 安全性

發現安全問題了嗎？請見 [SECURITY.md](SECURITY.md)。
