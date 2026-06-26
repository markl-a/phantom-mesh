> ⚠️ 架構參考 only — 本檔早於 2026-05-19 Life-Node pivot,描述已實作 daemon 架構但非現行產品範圍。治理見 superpowers/GOVERNANCE.md。

# Phantom Mesh 架構

[English version](ARCHITECTURE.md)

> 本文件是 `ARCHITECTURE.md` 的繁體中文閱讀版。產品方向仍以
> [`superpowers/BIG-GOAL.md`](superpowers/BIG-GOAL.md)（已是繁中正本）為準。

## 1. 概觀

Phantom Mesh 是 local-first 的 AI agent runtime。它有三種使用方式：

1. 作為 Claude Code 風格的 standalone REPL：`phantom`
2. 作為其他 agent 的 subagent：`phantom mcp` 或 `phantom serve`
3. 作為跨裝置 mesh runtime：節點透過 RPC、capability routing 與 heartbeat 合作

主要 surfaces：

```text
CLI / REPL
    |
    v
AgentRuntime --> LLMRouter --> LLM providers
    |
    +--> Tool Executor --> shell / files / browser / MCP tools
    |
    +--> ConversationStore
    |
    +--> CostTracker
    |
    +--> ClusterManager --> peer discovery / RPC forwarding

Web / Tauri / Telegram
    |
    +--> 共用相同 runtime contracts
```

## 2. 核心元件

### AppState

`AppState` 是 server handlers 共用的 runtime state，持有：

- Agent runtime
- Provider router
- Tool registry
- Cluster manager
- Conversation store
- Cost tracker
- Config

HTTP、WebSocket、Telegram 與 MCP surfaces 都應重用相同 state，而不是各自實作
不同商業邏輯。

### AgentRuntime

`AgentRuntime` 負責：

1. 接收 prompt 與 session context。
2. 從 config 選擇 agent role。
3. 透過 `LLMRouter` 呼叫 provider。
4. 解析 tool calls 並交給 Tool Executor。
5. 將 tool results 回送模型。
6. Stream `AgentEvent` 給 REPL、web、MCP 或 Telegram。
7. 保存 conversation 與 cost。

Agent loop 必須 surface-neutral。同一個行為應該能從 CLI、web、Tauri 與 subagent
入口使用。

### LLMRouter 與 fallback

`LLMRouter` 將 providers 放在統一介面後面。每個 agent 可以指定 provider
優先順序；當 provider auth、rate-limit 或暫時性錯誤時，router 依規則 fallback。

需要區分：

- 可 fallback 的暫時錯誤，例如 429、timeout。
- 不應 fallback 的 input error，例如 malformed request。
- 必須清楚回報的 auth error。

### Tool Executor

Tool Executor 將模型要求的 tool call 對應到已註冊 capability。原則：

- 工具必須明確註冊。
- Permission 與 sandbox 規則在執行前檢查。
- 結果以結構化資料回送 agent loop。
- 工具失敗不能造成 runtime panic。

### ClusterManager

ClusterManager 負責 mesh：

- Node discovery
- Peer metadata 與 capability profile
- Heartbeat 與 health state
- HMAC-authenticated RPC
- Capability-aware forwarding
- Peer failure 與 recovery

單機路徑仍然重要，但不能為了單機 UX 犧牲 mesh correctness。

### ConversationStore

ConversationStore 保存 session history，支援：

- 建立與恢復 session
- Append-only message history
- CLI `--session <id>` resume
- Agent context reconstruction

目前部分 conversation storage 仍為 plaintext；完整加密移到 v0.7.0+。

### CostTracker

CostTracker 記錄 provider、model、token usage 與估算成本。它用於：

- CLI 顯示
- Provider fallback audit
- 成本分析
- 每次 session 與 task 的追蹤

### TelegramBot

Telegram 是遠端 surface，不是獨立 runtime。它接收 user message 後應呼叫相同
AgentRuntime，並保留 allowlist、persona、streaming 與 error recovery。

### ProjectContext

ProjectContext 將 cwd、repo metadata 與必要檔案提供給 agent。它不應在不同
surface 中產生不一致行為。

## 3. Request Flow

典型流程：

```text
使用者 prompt
    |
    v
Surface adapter（CLI / WS / MCP / Telegram）
    |
    v
AgentRuntime
    |
    +--> 建立或載入 session
    |
    +--> LLMRouter.call_with_fallback()
    |
    +--> 如有 tool call，交給 Tool Executor
    |        |
    |        +--> 執行 tool
    |        +--> 將結果送回 agent loop
    |
    +--> Stream AgentEvent
    |
    +--> 寫入 conversation 與 cost
```

跨主機工作會多一層：

```text
AgentRuntime
    |
    v
ClusterManager.select_best_peer(required_caps)
    |
    +--> 本機可執行：local tool path
    |
    +--> 遠端較適合：HMAC RPC forwarding
```

## 4. P2P Mesh Protocol

### Node Discovery

節點透過 local network discovery、Tailscale 或明確設定找到 peers。每個 peer
需要揭露：

- Node ID
- Host 與 port
- OS / platform
- Capability set
- Health state
- Last-seen timestamp

### Authentication

Cluster RPC 使用 shared secret 與 HMAC 驗證。Receiver 必須拒絕：

- 缺少 auth header
- 錯誤 secret
- Malformed body
- 無效 signature

### Task Assignment

Task assignment 流程：

1. Caller 建立 task payload。
2. 依 required capabilities 選擇 healthy peer。
3. 使用 HMAC 簽署 RPC request。
4. 遠端建立 job 並回傳 `job_id`。
5. Caller 輪詢 status 或等待結果。
6. Audit log 保存 forwarding 與結果。

### Peer Health

Heartbeat 週期性更新 peer state：

```text
Healthy --> missed heartbeats --> Unhealthy
Unhealthy --> heartbeat restored --> Healthy
```

Peer unhealthy 時，routing 必須回傳清楚錯誤或選擇其他 peer，不能永久等待。

### RPC Endpoints

核心 RPC surfaces 包含：

```text
POST /rpc/task/assign
GET  /rpc/task/status/<job_id>
GET  /rpc/peers
```

Life Node 與技能庫(skill bank) APIs 另外由 `serve.rs` 提供。

## 5. 設定

主要設定來源是 `agents.toml`。概念結構：

```toml
[core]
host = "127.0.0.1"
port = 7879

[providers.openai]
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[agent.master]
provider = "openai"
model = "example-model"

[tools]
shell = true

[cluster]
secret_env = "PHANTOM_CLUSTER_SECRET"

[telegram]
enabled = false
```

規則：

- API keys 使用 env 或使用者本機 config，不得 commit。
- Agent role 只引用 provider 名稱，不綁死 vendor。
- Cluster secret 不得寫入 repo。
- Surface adapters 不應自行建立另一套 config semantics。

## 6. 資料儲存

主要資料位於：

```text
~/.phantom-mesh/
├── agents.toml
├── identity.key
├── conversations/
├── events/
├── memory.db
├── costs.json
└── traces/
```

### conversations

保存 append-only chat history，支援 resume。完整加密延後到 v0.7.0+。

### events

Life Node events 依 UUID 分目錄保存。v0.6.0 已透過 age v1 at-rest encryption
保護 `meta`、modalities 與 analysis。

### memory.db

技能庫(skill bank) FTS5 memory backend。用於 skill store、recall 與搜尋。目前完整加密仍是
v0.7.0+ 工作。

### costs.json

保存 provider usage 與 cost telemetry。目前仍是 plaintext，後續需要納入
完整資料樹加密。

## 7. 架構守則

- 所有 surfaces 共用 runtime contracts。
- Provider、tool、channel 都必須可以替換。
- 優先處理 structured errors，不要 panic。
- Mesh behavior 必須有單機 integration 與跨主機 E2E。
- 新增 storage path 時，要明確決定 encryption、delete-all scope 與 migration。
- 新增 OS-specific 行為時，要評估其他平台 parity。
