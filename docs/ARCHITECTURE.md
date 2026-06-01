# Phantom Mesh 架構

> 本文件描述**目前已實作**的 daemon（常駐服務）架構，內容依據 `core/src/` 中的
> 實際原始碼撰寫。它是開發者在本程式碼倉庫上工作時的權威參考。

---

## 1. 總覽

Phantom Mesh 是一個用 Rust 撰寫的分散式 AI agent（智慧代理）daemon（常駐服務）。
每個 node（節點）執行一個自包含的 HTTP 伺服器（Axum，連接埠 7878），對外提供一個
tool-calling（工具呼叫）的 agent 迴圈，背後可由任何 OpenAI 相容的 LLM（大型語言模型）
供應商驅動。多個 node 可以透過 Tailscale 或任何共享網路組成一個 P2P（點對點）
mesh（網狀網路）——任務以共享的 HMAC（雜湊訊息驗證碼）密鑰進行驗證，並轉發給
負載最低的 peer（對等節點）。客戶端可透過 HTTP REST、Tauri 桌面應用程式或
Telegram 連到 daemon。

```
┌─────────────────────────────────────────────────────────────┐
│                     phantom-mesh node                        │
│                                                              │
│  ┌──────────┐    ┌──────────────────┐   ┌────────────────┐  │
│  │  Tauri   │    │   Axum HTTP      │   │  Telegram Bot  │  │
│  │  Desktop │    │   :7878          │   │  (long-poll)   │  │
│  └────┬─────┘    └────────┬─────────┘   └──────┬─────────┘  │
│       │                   │                    │             │
│       └───────────────────▼────────────────────┘             │
│                      AgentRuntime                            │
│                  (up to 20-round tool loop)                  │
│                           │                                  │
│              ┌────────────▼────────────┐                     │
│              │  call_with_fallback()   │                     │
│              │  primary → others       │                     │
│              │  (exp. backoff, 429     │                     │
│              │   Retry-After honored)  │                     │
│              └────────────┬────────────┘                     │
│                           │ OpenAI-compat POST               │
│          ┌────────────────▼───────────────────┐              │
│          │   Configured LLM Provider(s)        │              │
│          │  openai / groq / gemini / compat    │              │
│          └─────────────────────────────────────┘              │
│                                                              │
│    ┌────────────────────────────────────────────────────┐    │
│    │            Tool Executor  (13 tools)                │    │
│    │  shell │ file_read │ file_write │ file_edit         │    │
│    │  content_search │ glob_search │ web_search          │    │
│    │  memory_store │ memory_recall                       │    │
│    │  git_status │ git_diff │ git_log │ git_commit        │    │
│    └────────────────────────────────────────────────────┘    │
│                                                              │
│  ┌────────────────┐  ┌───────────────┐  ┌───────────────┐   │
│  │ConversationStore│  │  CostTracker  │  │ClusterManager │   │
│  │ JSONL on disk  │  │ costs.json    │  │ /rpc/* routes │   │
│  └────────────────┘  └───────────────┘  └───────────────┘   │
└─────────────────────────────────────────────────────────────┘
              │  Tailscale VPN (or any reachable IP)  │
         ┌────▼────┐                            ┌─────▼────┐
         │ Node B  │                            │  Node C  │
         │  GCP    │                            │  iPhone  │
         └─────────┘                            └──────────┘
```

---

## 2. 核心元件

### AppState

唯一的共享狀態物件，透過 `axum::extract::State` 複製進每個 Axum handler（處理函式）。
它在 `main()` 中建構一次，由 `load_config_toml()` 填入資料，接著用 Arc 包裝，
因此複製的成本很低。

**主要欄位：**

| 欄位 | 型別 | 用途 |
|---|---|---|
| `agent_runtime` | `AgentRuntime` | 執行 tool-calling（工具呼叫）迴圈 |
| `llm_router` | `LLMRouter` | 保存供應商健康狀態摘要 |
| `tool_registry` | `ToolRegistry` | 已啟用工具名稱的清單 |
| `conversations` | `ConversationStore` | 每個對話的持久化歷史 |
| `cost_tracker` | `CostTracker` | 累計的 token（詞元）計量 |
| `cluster_manager` | `ClusterManager` | P2P（點對點）peer 管理器與驗證 |
| `job_store` | `JobStore` | 記憶體內的非同步工作狀態對照表 |
| `telegram_config` | `Option<TelegramConfig>` | 來自 agents.toml 的 bot 設定 |

**檔案：** `core/src/lib.rs`

---

### AgentRuntime

執行引擎。`AgentRuntime::run()` 驅動一場與 LLM 的多回合對話，在回合之間執行工具
呼叫，直到模型停止發出工具呼叫，或觸及防護上限為止。

**防護上限：**

- `MAX_ROUNDS = 20` — 工具呼叫回合的硬性上限
- `STALL_THRESHOLD = 2` — 連續多回合輸出相同時提早中斷迴圈
- `TOKEN_BUDGET = 60_000`（估計 token）— 觸發 context（上下文）壓縮

**Context（上下文）壓縮**（當估計 token 數 > 60 K 時觸發）：

```
system messages preserved
summary injection: "[Context compacted: N earlier messages dropped]"
last 12 conversation messages kept
any leading "tool" role messages stripped (would confuse the LLM)
```

**每回合的 tool-calling（工具呼叫）迴圈：**

```
call_with_fallback() → LLM response JSON
  ├─ record token usage → CostTracker
  ├─ extract content text (may be empty when tool calls present)
  └─ tool_calls array present?
       yes → execute_tool() for each call
             → append tool result messages
             → continue loop
       no  → stall check → break
```

**主要結構：** `AgentRuntime`、`AgentResult`

**檔案：** `core/src/lib.rs`（第 663–1649 行）

---

### LLMRouter + call_with_fallback()

`LLMRouter` 是包在 `Vec<ProviderHealthSummary>` 外的薄包裝，於設定載入時建立。
它的主要用途是向 dashboard（儀表板）回報供應商健康狀態。

實際的供應商選擇是在執行時於 `AgentRuntime::call_with_fallback()` 內完成：

```
provider_names = [agent.provider] + sorted(all_other_providers)

for each provider (attempt 0, 1, 2, …):
  1. look up api_key (direct or from env var)
  2. resolve endpoint URL:
       explicit url   → use as-is (append /v1/chat/completions if needed)
       type=openai    → https://api.openai.com/v1/chat/completions
       type=groq      → https://api.groq.com/openai/v1/chat/completions
       type=gemini    → https://generativelanguage.googleapis.com/v1beta/openai/...
       _              → https://openrouter.ai/api/v1/chat/completions
  3. exponential backoff before retries: 0s, 1s, 2s, 4s, …
  4. POST OpenAI-compat JSON body
  5. HTTP 2xx → return (response_json, model_name)
  6. HTTP 429 → honour Retry-After (≤30s), then try next provider
  7. HTTP 5xx → try next provider immediately
```

所有供應商都必須提供一個 OpenAI 相容的 `/v1/chat/completions` 端點。

**主要結構：** `LLMRouter`、`LLMRouterInner`、`ProviderHealthSummary`

**檔案：** `core/src/lib.rs`

---

### Tool Executor（工具執行器）

`execute_tool(name, args, tools_config)` 是一個大型的 `match` 區塊——沒有動態
dispatch（分派），沒有 trait 物件。每個工具都是嵌在某個 match 分支裡的一段
簡單非同步函式主體。

**13 種工具：**

| 工具 | 說明 |
|---|---|
| `shell` | 執行一個 shell 命令。封鎖清單（blocklist）會防範 `rm -rf /`、fork 炸彈等。以 `;`/`&&` 串接的命令序列會被拆開並逐一執行。預設逾時 30 秒。輸出在 20 K 字元處截斷。 |
| `file_read` | 讀取檔案。路徑經 `safe_path()` 解析（若存在則正規化）。 |
| `file_write` | 寫入檔案；自動建立上層目錄。 |
| `file_edit` | 在檔案中取代一段完全相符的字串；若相符次數 ≠ 1 則回報錯誤。 |
| `content_search` | 透過 `rg` 進行正規表示式／字面搜尋（找不到時退回 `grep`）。最多 50 個相符結果。 |
| `glob_search` | 透過 `find` 以 glob（萬用字元）樣式尋找檔案。排除 `node_modules`、`.git`、`target`。 |
| `web_search` | 若已設定 `brave_search_api_key` 則使用 Brave Search API，否則使用 DuckDuckGo 即時解答 API（不需金鑰）。 |
| `memory_store` | 將一組 key→value（鍵→值）寫入 `~/.phantom-mesh/memory.json`。 |
| `memory_recall` | 從 `~/.phantom-mesh/memory.json` 讀取某個 key。 |
| `git_status` | 對某個路徑執行 `git status --short`。 |
| `git_diff` | `git diff --stat`（可選 `--cached`，可選限定於某個檔案）。 |
| `git_log` | 對某個路徑執行 `git log --oneline -N`。 |
| `git_commit` | 對某個路徑執行 `git commit -am <message>`。 |

**檔案：** `core/src/lib.rs`（第 904–1313 行）

---

### ClusterManager

管理已設定的 peer（對等節點）。peer 在 `agents.toml` 中以純 URL 宣告——沒有自動
探索機制。Tailscale 或任何可路由的 IP 都是傳輸層。

**職責：**

- 為每個 peer 保留一份快取的 `Vec<PeerStatus>`（上線／離線、進行中的任務、運行時間）
- `ping_peer(url)` — POST `{peer}/rpc/ping`，更新快取狀態
- `refresh_all()` — 平行 ping 所有 peer
- `make_auth_token(body)` — SHA-256(`cluster_secret` ‖ `body`) → 十六進位字串
- `verify_auth(token, body)` — 透過 `subtle` crate 進行常數時間比對
- `assign_task_to_best_peer(agent, prompt)` — 挑選進行中任務最少的上線 peer，帶著 `X-Cluster-Auth` 標頭 POST 到它的 `/rpc/task/assign`

**主要結構：** `ClusterManager`、`ClusterConfig`、`PeerStatus`

**檔案：** `core/src/mesh.rs`

---

### ConversationStore

每個對話的持久化歷史，背後以磁碟上的 JSONL 檔案儲存。每個 `chat_id` 一個檔案；
每一行都是換行分隔的 JSON `ChatMessage` 物件。

**寫入路徑：** 先寫磁碟，再更新記憶體內快取（磁碟為權威來源）。

**讀取路徑：** 若 `chat_id` 不在快取中，先從磁碟載入快取，再回傳。

**儲存位置：** `~/.phantom-mesh/conversations/{chat_id}.jsonl`

`chat_id` 範例：
- `daemon` — 直接 HTTP 呼叫的預設值
- `tg:{telegram_chat_id}` — Telegram 對話
- `rpc` — 從 peer 節點收到的任務

**主要結構：** `ConversationStore`、`ChatMessage`（位於 `providers/traits.rs`）

**檔案：** `core/src/lib.rs`（第 534–637 行）

---

### CostTracker

累計所有 LLM 呼叫的 token（詞元）用量與美元成本估計。每次 `record()` 呼叫後
（以同步的 `fs::write`）將資料持久化到 `~/.phantom-mesh/costs.json`。

**價格表**（2026 年 4 月，每百萬 token——輸入／輸出）：

| 模型家族 | 輸入 | 輸出 |
|---|---|---|
| claude-opus-4 | $15 | $75 |
| claude-sonnet-4 | $3 | $15 |
| claude-haiku-4 | $0.80 | $4 |
| gpt-4o | $2.50 | $10 |
| gpt-4.1 | $2 | $8 |
| gemini-2.5-pro | $1.25 | $10 |
| gemini-2.0-flash | $0.10 | $0.40 |
| groq / llama | $0.05 | $0.08 |
| （預設值） | $1 | $3 |

**主要結構：** `CostTracker`、`CostTrackerInner`

**檔案：** `core/src/lib.rs`（第 417–506 行）

---

### TelegramBot

一個極簡的長輪詢（long-poll）Telegram bot，背後僅用 `reqwest`。未使用任何第三方
Telegram 函式庫。當有設定 `[telegram]` 時，輪詢迴圈會在一個由 `main()` 啟動的
`tokio::spawn` 工作中執行。

**輪詢迴圈（位於 `main.rs`）：**

```
loop:
  poll_updates(offset) → Vec<(chat_id, user_id, text, update_id)>
  for each update:
    if user not in allowed_users → skip (advance offset)
    load history from ConversationStore("tg:{chat_id}")
    AgentRuntime::run(agent_name, text, history, …, extra="Be concise.")
    send_message(chat_id, result.output)  // splits at 4000-char boundaries
  on error: sleep 5s, retry
```

訊息以 HTML 格式送出（`parse_mode: HTML`）。若 Telegram 端解析失敗，bot 會
自動以純文字重試。

**主要結構：** `TelegramBot`

**檔案：** `core/src/channels/telegram.rs`

---

### ProjectContext

從目前的工作目錄沿目錄樹往上走，尋找要注入 agent 系統提示（system prompt）的
專案 context（上下文）檔案。

**每個目錄的搜尋順序：**
1. `PHANTOM.md`
2. `.phantom-mesh/context.md`

在使用者的家目錄或檔案系統根目錄處停止。

**用法：** 載入的 context 字串會作為 `extra_context` 傳給 `AgentRuntime::run()`，
在那裡以兩個換行作為分隔符附加到系統提示之後。

**主要函式：** `load_project_context()`、`load_cwd_context()`、`load_from_path()`

**檔案：** `core/src/project_context.rs`

---

## 3. 請求流程

透過 `POST /agent/master/run` 的典型 agent 請求：

```
1.  HTTP POST /agent/{name}/run
      body: { "prompt": "...", "chat_id": "..." }

2.  agent_run() handler (main.rs)
      ├─ load conversation history from ConversationStore(chat_id)
      └─ call AgentRuntime::run_tracked(name, prompt, history, …)

3.  AgentRuntime::run_with_cost() — tool-calling loop begins
      ├─ look up agent config by name (fall back to "master")
      ├─ build tool_defs[] from agent.tools list (OpenAI function schemas)
      └─ assemble messages[]:
           [system]  agent.instructions
                     + CRITICAL RULES (if tools enabled)
                     + extra_context (PHANTOM.md if found)
           [history] prior ChatMessage turns
           [user]    current prompt

4.  Round 0: call_with_fallback(agent_cfg, messages, tool_defs)
      ├─ try agent's primary provider
      ├─ on failure: exponential backoff → try remaining providers in order
      └─ return (response_json, model_name)

5.  Record prompt_tokens + completion_tokens → CostTracker

6.  Parse response:
      ├─ content text → saved as final_output candidate
      └─ tool_calls[] present?

7a. No tool_calls → stall check → break loop
      final_output is the response text

7b. tool_calls present → for each call:
      execute_tool(fn_name, fn_args, tools_config) → result string
      truncate result to 20 K chars
      append { role:"tool", tool_call_id, content:result } to messages[]

8.  Back to step 4 with the extended messages[] (next round)
    Repeat until: no tool_calls | stall detected | MAX_ROUNDS (20) reached

9.  Return AgentResult { output, tool_calls_made, elapsed_secs }

10. agent_run() handler:
      ├─ append (user_msg, assistant_msg) to ConversationStore
      └─ return JSON { agent, output, tool_calls, elapsed }
```

---

## 4. P2P Mesh 協定

### Node Discovery（節點探索）

node（節點）是**以設定為基礎**——目前的實作中沒有自動的 mDNS 或 DNS-SD 探索。
每個 node 在 `agents.toml` 中明確列出它的 peer（對等節點）：

```toml
[cluster]
peers = ["http://100.64.0.2:7878", "http://100.64.0.3:7878"]
cluster_secret = "shared-hmac-key"
node_name = "my-node"
```

Tailscale 提供 VPN（虛擬私人網路）層，讓 node 能跨網路以穩定 IP 互相連通，
而不必對公開網際網路暴露連接埠。

### Authentication（驗證）

node 之間的每個 RPC 請求都必須包含一個 `X-Cluster-Auth` 標頭：

```
token = SHA-256(cluster_secret_bytes || request_body_bytes)
      formatted as lowercase hex
```

驗證採用常數時間比對（透過 `subtle` crate），以防範計時旁路（timing oracle）
攻擊。標頭遺漏或不正確的請求會以 HTTP 401 拒絕。若 `cluster_secret` 為空或缺漏，
則**所有**進入的叢集 RPC 請求一律拒絕。

### Task Assignment Protocol（任務指派協定）

```
Caller node                          Callee node
    │                                     │
    │  POST /rpc/task/assign              │
    │  X-Cluster-Auth: <token>            │
    │  { agent:"master", prompt:"..." }   │
    │─────────────────────────────────────▶│
    │                                     ├─ verify_auth()
    │                                     ├─ generate job_id (UUID v4)
    │                                     ├─ JobStore.insert(job_id, "running")
    │                                     ├─ tokio::spawn(AgentRuntime::run())
    │  202 Accepted                       │
    │  { "job_id": "uuid-..." }           │
    │◀─────────────────────────────────────│
    │                                     │
    │  (poll until done)                  │
    │  GET /rpc/task/status/{job_id}      │
    │─────────────────────────────────────▶│
    │  { status:"running" }               │
    │◀─────────────────────────────────────│
    │                                     │   (agent finishes)
    │  GET /rpc/task/status/{job_id}      │
    │─────────────────────────────────────▶│
    │  { status:"done", output:"..." }    │
    │◀─────────────────────────────────────│
```

### Peer Health（對等節點健康狀態）

`ClusterManager::refresh_all()` 透過 `POST {peer}/rpc/ping` 平行 ping 所有已設定的
peer。每個 peer 回應如下：

```json
{
  "name": "node-name",
  "version": "0.x.y",
  "uptime_secs": 3600,
  "active_tasks": 2,
  "online": true
}
```

`assign_task_to_best_peer()` 會選出 `active_tasks` 計數最低的上線 peer。

### RPC Endpoints（RPC 端點）

| 方法 | 路徑 | 用途 |
|---|---|---|
| POST | `/rpc/ping` | 回傳本 node 的狀態（不需驗證） |
| GET | `/rpc/peers` | 列出所有已設定的 peer 及其快取狀態 |
| POST | `/rpc/task/assign` | 接受來自某個 peer 的任務（需驗證） |
| GET | `/rpc/task/status/:job_id` | 輪詢非同步任務結果 |

---

## 5. 設定

所有設定都放在 `~/.phantom-mesh/agents.toml`（預設），或透過 `--config` 提供的路徑。

```toml
# ── Core server settings ──────────────────────────────────────────────────
[core]
host = "0.0.0.0"      # bind address (default: 0.0.0.0)
port = 7878           # HTTP port    (default: 7878)
hub_api_key = "..."   # optional key for external hub integrations

# ── LLM provider definitions ──────────────────────────────────────────────
# Keys under [providers.*] become provider names referenced in [agent.*].
[providers.anthropic]
type = "openai_compat"
url = "https://api.anthropic.com"          # optional; inferred from type if absent
api_key_env = "ANTHROPIC_API_KEY"          # env var name (preferred over api_key)
default_model = "claude-sonnet-4-5"        # used when this is a fallback provider

[providers.openai]
type = "openai"
api_key_env = "OPENAI_API_KEY"
default_model = "gpt-4o"

[providers.groq]
type = "groq"
api_key_env = "GROQ_API_KEY"
default_model = "llama-3.3-70b-versatile"

# ── Agent definitions ─────────────────────────────────────────────────────
# "master" is the default agent name used by the HTTP handler and Telegram.
[agent.master]
provider = "anthropic"                     # primary provider key
model = "claude-sonnet-4-5"               # model for this agent
instructions = "You are a helpful AI agent..."
tools = [                                  # controls which tools are available
  "shell", "file_read", "file_write", "file_edit",
  "content_search", "glob_search", "web_search",
  "memory_store", "memory_recall",
  "git_status", "git_diff", "git_log", "git_commit",
]

# ── Tool settings ─────────────────────────────────────────────────────────
[tools]
brave_search_api_key = "BSA-..."          # if set, web_search uses Brave instead of DDG

# ── Cluster / P2P mesh ────────────────────────────────────────────────────
[cluster]
node_name = "my-macbook"
peers = ["http://100.64.0.2:7878"]
cluster_secret = "change-me-before-production"

# ── Telegram bot ──────────────────────────────────────────────────────────
[telegram]
bot_token_env = "TELEGRAM_BOT_TOKEN"      # env var holding the bot token
allowed_users = [123456789]               # Telegram user IDs; empty = allow all
agent = "master"                          # which agent handles Telegram messages
```

**設定控制項一覽：**

| 區段 | 控制項 |
|---|---|
| `[core]` | 綁定位址、連接埠、可選的 hub API 金鑰 |
| `[providers.*]` | LLM 端點、API 金鑰、備援模型 |
| `[agent.*]` | 各 agent 的模型、供應商、系統提示、工具清單 |
| `[tools]` | 網路搜尋後端（Brave 或 DuckDuckGo） |
| `[cluster]` | peer URL、HMAC 密鑰、本機 node 名稱 |
| `[telegram]` | bot token、使用者允許清單、路由 agent |

---

## 6. 資料儲存

所有持久化狀態都放在 `~/.phantom-mesh/` 之下：

```
~/.phantom-mesh/
├── agents.toml                  — main configuration file
├── conversations/
│   ├── daemon.jsonl             — default HTTP chat history
│   ├── tg:123456789.jsonl       — Telegram chat (chat_id prefixed with "tg:")
│   └── rpc.jsonl                — tasks received from peer nodes
├── memory.json                  — key→value store written by memory_store tool
└── costs.json                   — cumulative token + USD cost tracking
```

### conversations/{chat_id}.jsonl

換行分隔的 JSON。每一行都是一個 `ChatMessage`：

```json
{"role":"user","content":"what is 2+2?"}
{"role":"assistant","content":"4"}
```

每次 agent 成功執行後附加。對某個 `chat_id` 首次存取時才惰性載入記憶體。
此快取為寫穿（write-through）：先寫磁碟，再更新記憶體內快取。

### memory.json

一個扁平的 JSON 物件。每次 `memory_store` 工具呼叫時以原子方式寫入
（整個檔案重寫）：

```json
{
  "project_name": "phantom-mesh",
  "last_deploy": "2026-04-20"
}
```

### costs.json

一個扁平的 JSON 物件，每次 LLM 呼叫後更新：

```json
{
  "total_usd": 0.0423,
  "requests": 17,
  "prompt_tokens": 42310,
  "completion_tokens": 8940
}
```

`total_usd` 在 API 回應中四捨五入到小數點後 4 位。
