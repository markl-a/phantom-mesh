# Spectyn Mesh — 架構（依實作現況）

[English version](ARCHITECTURE.md)

> 本文件描述 `core/src/` **已實作**的架構，對應 **v0.6.0（2026-07）**。
> 誠實標記：🟢 穩定且有測試 · 🟡 可用、仍在演進 · 🧪 實驗性 / feature-gate 之後。
> 各子系統的深入筆記在 [`docs/architecture/`](architecture/)。

---

## 1. 總覽

Spectyn Mesh 是**單一 Rust 執行檔**（`spectyn`），同時是個人 AI runtime 的三種形態：
互動式 CLI/TUI、常駐 daemon、mesh 對等節點——全在同一顆 binary 裡。
你自己的多台機器（Windows / macOS / Linux / Android / iOS 客戶端）透過 Tailscale
或任何共享網路組成私有 **mesh**；任務以共享的 HMAC cluster secret 驗證，
並路由到最適合的節點執行。

```
┌────────────────────────────── spectyn 節點 ──────────────────────────────┐
│                                                                          │
│  入口           spectyn (TUI) · repl · exec · serve · mcp · evolve ·     │
│                 swarm · service · status · inbox …                       │
│                                                                          │
│  serve (Axum)   /ws  /api/*  /rpc/*  /mcp  ·  /m = 行動戰情室 PWA        │
│                 （manifest + service worker → 可安裝、全螢幕）           │
│                                                                          │
│                    ┌────────────────────────────┐                        │
│                    │        AgentRuntime        │  工具呼叫迴圈          │
│                    └──────────────┬─────────────┘                        │
│                                   │                                      │
│       ┌───────────────────────────┴───────────────────────────┐          │
│       │   provider resolver · 失效轉移鏈 · circuit breaker    │          │
│       └──────┬──────────────────────────────────┬─────────────┘          │
│              │ HTTP providers                   │ 訂閱制 CLI 後端        │
│  openai 相容 / gemini / groq / OAuth 裝置流程 / │（L0 cli_session PTY）  │
│  本地 Ollama · 額外家族 🧪（mistral, cohere,    │ claude · codex ·       │
│  fireworks, together, nvidia, perplexity,       │ gemini-cli · opencode  │
│  xai, ai21）在 feature flag 之後                │ … 磁碟上零 API key     │
│                                                                          │
│  工具層         約 60 個內建工具 + cluster RPC，全部經過：               │
│                 tool_gate（行程級）· `Tool(specifier)` 權限 DSL ·        │
│                 專案信任 · 節點能力宣告                                  │
│                                                                          │
│  治理           風險分級的待審批佇列（批准/停止）·                       │
│                 governed_run 飛行紀錄器：簽章逐字稿 + 任務事件 +         │
│                 審批決定                                                 │
│                                                                          │
│  身分/加密      每裝置 64-byte root IKM（identity.key）+ ed25519 簽章    │
│                 金鑰 · age + HKDF-SHA256 · OS 金鑰庫切換                 │
│                 （macOS/iOS Keychain 🟢 · Win DPAPI 🟢 · Android 🟡）      │
│                                                                          │
│  記憶           SQLite FTS5 自有記憶——跨 session 回想，預設開啟、        │
│                 有 kill switch                                           │
│                                                                          │
│  MCP            server（stdio + /mcp）：曝露工具/記憶/叢集 ·             │
│                 client：外部 MCP server 變成 agent 工具                  │
│                                                                          │
│  頻道           remote_control：Telegram 🟢 · Slack 🟢 · WhatsApp 樁 ·   │
│                 persona 綁定 · 限流 · webhook 驗證                       │
│                                                                          │
│  自我改進       evolve（測試驅動修復迴圈）· autoevolve 常駐 ·            │
│                 檢查點 / 重播 / 跨節點交接                               │
│                                                                          │
│  多節點         mesh 對等管理 · swarm 扇出 · crew 多 CLI 管線 ·          │
│                 fleet 共享待辦開發迴圈                                   │
└──────────────────────────────────────────────────────────────────────────┘
          │ Tailscale VPN（或任何可達 IP），HMAC 驗證 │
     ┌────▼─────┐        ┌──────────┐        ┌───────────┐
     │  節點 B  │  ···   │  節點 C  │  ···   │ 手機 /    │
     └──────────┘        └──────────┘        │ Web PWA   │
                                             └───────────┘
```

## 2. 核心迴圈 — `agent.rs` / `runtime.rs` 🟢

`AgentRuntime` 驅動 LLM 對話：提示 → 模型 → 工具呼叫 → 結果 → 模型，
有回合上限、停滯偵測與 context 壓縮。Session 落盤持久化（`session.rs`，JSONL），
`/compact` 用 LLM 摘要舊回合。`context.rs` 管 workspace 範圍；`cost.rs` 追蹤
每輪與累計成本（REPL 的 `/cost`）。

## 3. Providers — `providers/` 🟢

一個 trait、兩個家族：

- **HTTP providers** —— 預設核心（OpenAI 相容、Gemini、Groq、訂閱帳號的 OAuth
  裝置流程、本地 Ollama），另有一組額外轉接家族（Mistral、Cohere、Fireworks、
  Together、NVIDIA、Perplexity、xAI、AI21…）鎖在 `experimental-extra-providers`
  🧪 之後。金鑰只來自環境變數或內建 vault，絕不進 repo。
- **訂閱制 CLI 後端**（`claude_agent`、`codex_agent`、`opencode_agent`…）——
  spectyn 透過 **L0 `cli_session` 底座**（PTY 橋）驅動本機已登入的 coding-agent
  CLI。完全不儲存 API key；吃到飽訂閱的邊際成本 = $0。

`resolver.rs` 建立明確的失效轉移順序，含 429/5xx 重試退避與熔斷器。`credential_scanner.rs` 偵測主機上已登入的訂閱 CLI
（onboarding 用——產出的 `agents.toml` 只含 provider *type*）。

## 4. 工具與強制 — `tools/`、`tool_gate.rs`、`permission.rs`、`capabilities/` 🟢

約 60 個內建工具（shell、檔案、搜尋、git、記憶、web…）加 cluster-RPC 工具。
每次呼叫都經過單一**行程級 tool gate**：

1. `permission.rs` —— Claude Code 風格的 `Tool(specifier)` allow / ask / deny 規則
2. `project_trust.rs` —— 逐目錄信任，先信任才談強制
3. `capabilities/` —— 任務可宣告 `required_caps`；節點宣告偵測到的硬體/OS 能力，
   接不了的工作直接拒絕

## 5. Serve daemon 與客戶端 — `serve.rs`、`web/` 🟢

`spectyn serve` 啟動 Axum 伺服器：WebSocket（`/ws`）、REST（`/api/*`）、叢集 RPC
（`/rpc/*`）、MCP 端點，以及 **`/m` 行動戰情室**——真正可安裝的 PWA（standalone
顯示、service worker），呈現節點網格、編排方案、即時成本、MCP 工具曝露與治理
飛行紀錄器。原生客戶端：Tauri 桌面 App（`app/`）、生成的 iOS Xcode 專案、Telegram。

## 6. Mesh — `mesh.rs`、`swarm.rs`、`crew/`、`fleet/` 🟡

節點以共享 **HMAC cluster secret** 驗證（未設定 ⇒ 拒絕，fail-closed）。
對等管理器追蹤健康度，依負載與能力路由任務。`swarm` 把一個提示扇出到所有在線
節點並綜整答案。`crew` 組合多 CLI 管線（不同廠牌 CLI 擔任 writer / reviewer）。
`fleet` 實作共享待辦開發迴圈（原子認領 → 實作 → 驗證交付 → 交叉審查）——
spectyn 自己的開發就跑在這上面。

## 7. 治理 — `approval.rs`、`governed_run/` 🟢

被評為高風險級別的工具動作（例如風險分級為 `execute_high` 的 shell 指令）進入**待審批佇列**，在 TUI、Web 主控台
與手機 App 上呈現（批准/停止）。每次受治理的執行都寫入**飛行紀錄器**：簽章
逐字稿、任務事件、審批決定與其強制模式（`auto`、`pre_action_blocking`）——
事後可稽核。

## 8. 身分、加密與 vault — `identity*.rs`、`encryption_wire.rs`、`vault/` 🟢

每台裝置產生一把 **64-byte root IKM**（`identity.key`），旁邊另有獨立的
**ed25519 簽章金鑰**。事件內容以 **age** 加密，金鑰經 **HKDF-SHA256** 衍生。根身分正從 0600 檔案切換到 **OS 原生金鑰庫**——
macOS/iOS Keychain 與 Windows DPAPI 已落地，Android Keystore 進行中——
遷移與還原保證 byte-identical（還原後衍生出*同一把* event key，絕不重生新 key）。
`broker_vault_wire.rs` 為選配的雲端 broker 包裝每裝置密鑰。

## 9. 記憶 — FTS5 自有記憶 🟢

分兩層。簡單的 agent 工具（`memory_store` / `memory_recall`）把 JSON key-value
存到 `~/.spectyn-mesh/` 底下。**自有記憶（owned-memory）**層則把事件、capture
與技能索引進 SQLite FTS5（event storage / skill / capture 各管線），供跨 session
回想——預設開啟、有 kill switch。這就是「越用越懂你」的那一層。

## 10. MCP — `mcp.rs` / `mcp_client.rs` 🟢

Spectyn 同時是 **MCP server**（stdio 供 Claude Desktop / Cursor，另有 HTTP `/mcp`
——曝露工具、記憶與叢集派送）與 **MCP client**（外部 MCP server 變成 agent 迴圈
裡的工具）。衛星生態（secops、finance、quant、tutor…）就是經由這個介面接入。

## 11. 遠端控制頻道 — `remote_control/` 🧪

聊天頻道作為 mesh 的遙控器，逐頻道 feature-gate：**Telegram 已上線**（long-poll
bot、agent dispatcher、媒體處理），**Slack 已上線**（外送 `chat.postMessage`、
HMAC 驗證的入站 webhook）；WhatsApp 仍是編譯檢查的樁，鎖在自己的 flag 後面。底下是共用的 `Channel` trait、逐頻道 **persona**
綁定與 token-bucket 限流。

## 12. 自我改進 — `evolve*`、`autoevolve` 🟡

`spectyn evolve` 跑測試驅動的修復迴圈（可跨 mesh 分散執行，含 LLM 評審團與技能
萃取）。`autoevolve` 是常駐形態：監看 → 修復 → 測試轉綠自動 commit，整合 OS
排程器並留 JSONL 紀錄。檢查點可列出、重播、交接給其他節點。

## 13. Life-Node 層 — `life_node/` 🟡

個人資料平面：多模態 capture（影像/語音/文字，經 Groq、Gemini、Ollama 含
fallback）、focus / food / habit 管線、每日回顧、教練推送——Life Track 功能，
騎在同一套 runtime、儲存與加密之上。

## 14. Skillbank 🧪

實驗性的六步技能閉環（策展 → 儲存 → 回想 → 組合 → 驗證 → 演化），在
`experimental-skillbank` 之後；calculator / unit-convert / json-query 這批工具
就住在這裡，配硬性測試閘門。

---

### 新貢獻者建議閱讀順序

`lib.rs` → `agent.rs` → `providers/resolver.rs` → `tool_gate.rs` → `serve.rs` →
`mesh.rs` → `governed_run/` → `identity_wire.rs`。
