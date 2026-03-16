# IronClaw 架構文檔

## 1. 專案概覽

IronClaw 是 NEAR AI 開發的安全個人 AI 助手框架，採用防禦深度設計。核心特性包括多通道互動（CLI、Slack、Telegram、HTTP）、並行任務執行、可擴展工具系統（MCP、WASM、動態工具）、自我修復機制、提示注入防禦、Docker 沙箱隔離、以及從歷史數據持續學習的成本/時間估計引擎。架構遵循 NEAR AI 市場化框架，支持 PostgreSQL + libSQL 雙後端持久化，實現零信任安全模型。

---

## 2. 目錄結構（src/ 主要模組）

```
src/
├── agent/              ⭐ 核心代理迴圈、任務排程、自我修復
├── app.rs              應用啟動編排 (AppBuilder)
├── agent_loop.rs       Agent 主迴圈與事件調度
├── bootstrap.rs        基礎目錄解析、.env 加載
├── channels/           多通道輸入（CLI、HTTP、Slack、Telegram、Web、WASM）
├── cli/                命令行子命令（clap 派生）
├── config/             配置管理（環境變數、功能開關）
├── context/            任務上下文隔離 (JobContext、ContextManager)
├── db/                 ⭐ 雙後端持久化（PostgreSQL + libSQL）
├── estimation/         成本/時間/價值估計引擎（EMA 學習）
├── evaluation/         成功評估（規則/LLM 混合）
├── extensions/         WASM 擴展註冊與加載
├── history/            持久化層（儲存庫、分析）
├── hooks/              生命週期鉤點（BeforeInbound、BeforeToolCall、BeforeOutbound）
├── llm/                ⭐ 多提供商 LLM 集成（NEAR AI、OpenAI、Anthropic、Ollama、Bedrock）
├── observability/      可插拔事件記錄（noop、log、multi）
├── orchestrator/       容器編排器 HTTP API（Docker 沙箱管理）
├── pairing/            用戶配對/設備綁定流程
├── registry/           擴展清單與安裝程序
├── safety/             ⭐ 提示注入防禦（檢測、清理、政策、洩露檢測）
├── sandbox/            ⭐ Docker 容器沙箱（容器生命週期、網路代理、憑證注入）
├── secrets/            密鑰管理（AES-256-GCM、OS 鑰匙圈）
├── service.rs          OS 服務管理（launchd/systemd）
├── settings.rs         用戶設定持久化
├── setup/              7 步驟上線導覽程序
├── skills/             SKILL.md 提示擴展系統
├── tools/              ⭐ 可擴展工具系統（24+ 內建、MCP、WASM、動態構建）
├── tunnel/             隧道抽象（Cloudflare、ngrok、Tailscale）
├── util.rs             共享工具函數
├── webhooks/           HTTP 鉤點與事件廣播
├── worker/             ⭐ Docker 容器內執行（ProxyLlmProvider、任務委託）
├── workspace/          ⭐ 持久記憶系統（混合搜尋、身份文件、心跳）
├── lib.rs              函式庫根（模組聲明）
├── main.rs             進入點、CLI 分派、啟動
```

---

## 3. 核心 Trait/Struct（10 個關鍵類型）

### 3.1 Agent & 事件迴圈
**檔案**: `src/agent/agent_loop.rs`
```rust
pub struct Agent {
    deps: Arc<AgentDeps>,
    channel_manager: ChannelManager,
    session_manager: SessionManager,
}

pub struct AgentDeps {
    db: Arc<dyn Database>,
    llm: Arc<dyn LlmProvider>,
    tools: Arc<ToolRegistry>,
    safety: Arc<SafetyLayer>,
    scheduler: Arc<Scheduler>,
    // ...
}
```
**角色**: 主體事件迴圈，調度來自通道的訊息到 LLM 推理、工具執行。

### 3.2 LLM Provider
**檔案**: `src/llm/provider.rs`
```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError>;
    async fn complete_with_tools(&self, request: ToolCompletionRequest) -> Result<ToolCompletionResponse, LlmError>;
    fn model_name(&self) -> &str;
    fn cost_per_token(&self) -> (Decimal, Decimal);
}
```
**角色**: 多提供商抽象。實現包括 NearAiChatProvider、RigAdapter（OpenAI/Anthropic/Ollama）、BedrockProvider。支持電路斷路器、重試、容錯轉移、回應快取。

### 3.3 Tool Registry
**檔案**: `src/tools/tool.rs` + `src/tools/registry.rs`
```rust
#[async_trait]
pub trait Tool: Send + Sync {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError>;
    fn definition(&self) -> &ToolDefinition;
    fn approval_requirement(&self) -> ApprovalRequirement;
}

pub struct ToolRegistry {
    tools: Arc<RwLock<HashMap<String, Arc<dyn Tool>>>>,
}
```
**角色**: 24+ 內建工具（shell、file_read、web_search 等）+ WASM 動態工具 + MCP 客戶端工具。速率限制、超時、安全驗證。

### 3.4 Database Abstraction
**檔案**: `src/db/mod.rs`
```rust
#[async_trait]
pub trait Database: Send + Sync {
    // Sessions, jobs, costs, memory 等
    async fn get_session(&self, session_id: Uuid) -> Result<Option<Session>>;
    async fn save_job(&self, job: &Job) -> Result<()>;
    // ...
}
```
**角色**: PostgreSQL + libSQL 雙後端。所有新持久化特性必須支持兩個後端。實現模式遷移、連接池、事務管理。

### 3.5 Channel Abstraction
**檔案**: `src/channels/channel.rs`
```rust
#[async_trait]
pub trait Channel: Send + Sync {
    fn name(&self) -> &str;
    fn incoming_stream(&self) -> Pin<Box<dyn Stream<Item = IncomingMessage>>>;
    async fn send_response(&self, response: OutgoingResponse) -> Result<()>;
}
```
**角色**: 多通道適配器。實現包括 CliChannel、HttpChannel（Axum）、TelegramChannel、SlackChannel、WebChannel、WasmChannelRouter。

### 3.6 Scheduler & Job Manager
**檔案**: `src/agent/scheduler.rs` + `src/worker/job.rs`
```rust
pub struct Scheduler {
    jobs: Arc<RwLock<HashMap<Uuid, Worker>>>,
    subtasks: Arc<RwLock<HashMap<Uuid, TaskHandle>>>,
}

pub struct Worker {
    job_id: Uuid,
    sender: mpsc::Sender<WorkerMessage>,
    // ...
}
```
**角色**: 並行任務排程。全 LLM 驅動的 `jobs`（持久化）與輕量級 `subtasks`（工具執行/背景）。檢測卡住的任務，自我修復。

### 3.7 Safety Layer
**檔案**: `src/safety/mod.rs` + 子模組
```rust
pub struct SafetyLayer {
    sanitizer: Sanitizer,
    validator: Validator,
    policy_engine: PolicyEngine,
    leak_detector: LeakDetector,
}
```
**角色**: 提示注入防禦。模式檢測、內容轉義、憑證洩露檢測、機密狀態模式庫。逐層防禦，失敗開放（處理繼續）。

### 3.8 Workspace & Memory
**檔案**: `src/workspace/mod.rs`
```rust
pub struct Workspace {
    db: Arc<dyn Database>,
    embeddings: Arc<dyn EmbeddingProvider>,
}
```
**角色**: 持久記憶系統。混合搜尋（FTS + 向量 via RRF）。身份文件（AGENTS.md、SOUL.md、USER.md）注入到系統提示。心跳定期執行（預設 30 分鐘）。

### 3.9 Sandbox Manager
**檔案**: `src/sandbox/mod.rs`
```rust
pub struct SandboxManager {
    container_runner: ContainerRunner,
    proxy: NetworkProxy,
}
```
**角色**: Docker 容器沙箱。容器生命週期、資源限制、網路代理（域名白名單、憑證注入）、timeout 強制。容器內工具受限為 shell/file_ops/patch。

### 3.10 AppBuilder & Initialization
**檔案**: `src/app.rs`
```rust
pub struct AppBuilder { /* ... */ }
pub struct AppComponents {
    pub config: Config,
    pub db: Option<Arc<dyn Database>>,
    pub llm: Arc<dyn LlmProvider>,
    pub tools: Arc<ToolRegistry>,
    pub safety: Arc<SafetyLayer>,
    pub workspace: Option<Arc<Workspace>>,
    // ...
}
```
**角色**: 應用啟動編排。5 個初始化階段（配置 → DB → LLM → 工具 → 通道）。讓測試無需通道即可構建完整應用。

---

## 4. 啟動流程（main.rs → 初始化順序）

```
main() [同步]
  ↓
  1. dotenvy::dotenv() — 加載 .env 文件
  2. ironclaw::bootstrap::load_ironclaw_env() — 解析基礎目錄 (~/.ironclaw)
  ↓
  async_main() [Tokio 多執行緒運行時]
  ├─ Cli::parse() — clap 命令行解析
  │
  ├─ 非代理命令 (Tool/Config/Registry/Mcp/Memory/Pairing/Service/Doctor/Status)
  │  └─ 早期返回 (no app init)
  │
  ├─ Worker 命令 (ironclaw worker --job-id X --orchestrator-url)
  │  └─ worker::run_worker() (容器內模式)
  │
  └─ 代理命令 (默認或 /run 與 /repl)
     ↓
     AppBuilder::new(config)
       ├─ Phase 1: Config hydration & secrets inject
       ├─ Phase 2: Database init (PostgreSQL/libSQL)
       ├─ Phase 3: LLM provider chain (NEAR AI/OpenAI/Anthropic/Bedrock)
       ├─ Phase 4: Tool registry (builtin + WASM + MCP)
       ├─ Phase 5: Safety/workspace/hooks 初始化
       └─ .build() → AppComponents
     ↓
     AppComponents → [ChannelManager setup]
       ├─ CLI 通道 (Ratatui TUI)
       ├─ HTTP 通道 (Axum webhook server)
       ├─ Web 通道 (Web UI)
       └─ WASM 通道 (bundled extensions)
     ↓
     Agent::new(components, channel_manager)
       ├─ ChannelManager.merge_streams() — 多路複用通道
       └─ Agent.run() — 主事件迴圈
     ↓
     [Session manager starts pruning (每 10 分鐘)]
     [Heartbeat runs (預設 30 分鐘)]
     [Routine engine starts (cron + event triggers)]
     [Self-repair checks stuck jobs (預設 5 分鐘)]
```

---

## 5. 資料流 ASCII 圖

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          External Channels                                   │
│   Telegram  Slack  Discord  Web UI  CLI REPL  HTTP Webhook  WASM Browser  │
└──────────────┬────────────┬──────────────┬─────────────────┬──────────────┘
               │            │              │                 │
               └────────────┴──────────────┴─────────────────┘
                            ▼
                    ┌────────────────────┐
                    │ ChannelManager     │
                    │ (多路複用流)        │
                    └────────┬───────────┘
                             ▼
                    ┌──────────────────────────┐
                    │  Agent.run()             │
                    │  主事件迴圈              │
                    └────────┬─────────────────┘
                             ▼
                    ┌──────────────────────────┐
                    │ handle_message()         │
                    │ ├─ 提交解析              │
                    │ ├─ /commands 路由        │
                    │ └─ 跳過授權檢查          │
                    └────────┬─────────────────┘
                             ▼
            ┌────────────────────────────────────┐
            │  Session/Thread/Turn State Machine │
            │  (Idle → Processing → Complete)    │
            └────────────┬───────────────────────┘
                         ▼
        ┌────────────────────────────────────────────┐
        │  Dispatcher (dispatcher.rs)                │
        │  ├─ 技能選擇 (skill_selector.rs)          │
        │  ├─ BeforeInbound 鉤點                    │
        │  └─ run_agentic_loop(ChatDelegate)        │
        └────────────────┬────────────────────────┘
                         ▼
        ┌────────────────────────────────────────────┐
        │  run_agentic_loop() 共享引擎              │
        │  (agentic_loop.rs)                        │
        └────────┬──────────────────────┬──────────┘
                 ▼                      ▼
        ┌──────────────────┐  ┌──────────────────┐
        │  1. LLM Call     │  │ 2. 工具執行       │
        │  ├─ 成本保衛     │  │ ├─ 執行 (timeout) │
        │ CostGuard        │  │ ├─ 安全驗證       │
        │  └─ 電路斷路器   │  │ └─ 序列化結果    │
        └──────────────────┘  └──────────────────┘
                 │                      │
                 └──────────┬───────────┘
                            ▼
        ┌────────────────────────────────────────────┐
        │  Tool Execution (tools/execute.rs)        │
        │  ├─ Shell (rate-limited, allowlist)       │
        │  ├─ File ops (workspace sandbox)          │
        │  ├─ HTTP (proxy through sandbox)          │
        │  ├─ WASM tools (wasmtime)                 │
        │  └─ MCP (JSON-RPC over stdio/HTTP)        │
        └────────┬────────────────────────────────┘
                 ▼
        ┌────────────────────────────────────────────┐
        │  Safety Layer (post-tool)                  │
        │  ├─ Sanitizer (pattern redaction)         │
        │  ├─ Leak detector (credentials)           │
        │  └─ Policy engine (block/redact/allow)    │
        └────────┬────────────────────────────────┘
                 ▼
        ┌────────────────────────────────────────────┐
        │  Compaction (if context > 80%)            │
        │  ├─ MoveToWorkspace (trim recent)         │
        │  ├─ Summarize (LLM summary)               │
        │  └─ Truncate (drop old turns)             │
        └────────┬────────────────────────────────┘
                 ▼
        ┌────────────────────────────────────────────┐
        │  BeforeOutbound 鉤點 + Response Format    │
        │  ├─ Session 儲存                          │
        │  ├─ Undo checkpoint                       │
        │  └─ Turn 標記完成                         │
        └────────┬────────────────────────────────┘
                 ▼
        ┌────────────────────────────────────────────┐
        │  回傳通道                                  │
        │  ├─ Telegram/Slack (formatted)            │
        │  ├─ Web (SSE broadcast)                   │
        │  ├─ CLI (TUI 顯示)                        │
        │  └─ HTTP (JSON 回應)                      │
        └────────────────────────────────────────────┘

並行流（Scheduler）:
        ┌──────────────────┐
        │ /job 或 CreateJob │
        └────────┬─────────┘
                 ▼
        ┌──────────────────────────┐
        │ dispatch_job()           │
        ├─ 新 JobContext           │
        └────────┬─────────────────┘
                 ▼
        ┌──────────────────────────────┐
        │ Scheduler.schedule()         │
        │ (check-insert under lock)    │
        └────────┬────────────────────┘
                 ▼
        ┌──────────────────────────────┐
        │ Worker (src/worker/job.rs)   │
        │ ├─ JobDelegate              │
        │ ├─ run_agentic_loop()       │
        │ └─ planning support         │
        └────────┬────────────────────┘
                 ▼
        ┌──────────────────────────────┐
        │ SSE broadcast completion     │
        │ 或 Webhook callback         │
        └──────────────────────────────┘

容器沙箱流:
        ┌─────────────────────┐
        │ SandboxManager      │
        └────────┬────────────┘
                 ▼
        ┌──────────────────────────────┐
        │ ContainerRunner.create()     │
        │ ├─ Bollard Docker client     │
        │ ├─ Resource limits (mem/cpu) │
        │ └─ Timeout enforcement       │
        └────────┬────────────────────┘
                 ▼
        ┌──────────────────────────────┐
        │ NetworkProxy (HTTP server)   │
        │ ├─ Domain allowlist check    │
        │ ├─ Credential injection      │
        │ └─ Audit logging             │
        └────────┬────────────────────┘
                 ▼
        ┌──────────────────────────────┐
        │ Container LLM/Tool execution │
        │ (worker mode)                │
        └──────────────────────────────┘
```

---

## 6. 子系統清單（優先度分類）

### P0 (核心、每日活動)
| 模組 | 作用 | 重要性 |
|------|------|--------|
| `agent/agent_loop.rs` | 主事件迴圈、訊息調度 | **至關重要** — 代理流程入口 |
| `agent/dispatcher.rs` | 推理 → 工具執行迴圈 | **至關重要** — 轉換邏輯 |
| `llm/mod.rs` | 多提供商 LLM 鏈 | **至關重要** — 推理的大腦 |
| `tools/tool.rs` + `registry.rs` | 工具執行框架 | **至關重要** — 代理行動 |
| `safety/mod.rs` | 提示注入防禦 | **至關重要** — 安全邊界 |
| `db/mod.rs` | 持久化抽象 | **至關重要** — 狀態儲存 |
| `channels/channel.rs` | 通道多路複用 | **至關重要** — I/O 邊界 |
| `agent/scheduler.rs` | 並行任務排程 | **很重要** — 背景工作 |
| `sandbox/mod.rs` | Docker 隔離 | **很重要** — 信任邊界 |

### P1 (生產必需)
| 模組 | 作用 | 重要性 |
|------|------|--------|
| `agent/session.rs` | 會話/執行緒/轉換狀態 | **很重要** — 互動狀態機 |
| `workspace/mod.rs` | 記憶搜尋、身份 | **很重要** — 個人化基礎 |
| `hooks/mod.rs` | 生命週期擴展點 | **重要** — 集成掛鉤 |
| `config/mod.rs` | 配置與功能開關 | **重要** — 靈活部署 |
| `secrets/mod.rs` | 密鑰管理（AES-256-GCM） | **重要** — 敏感數據 |
| `estimation/mod.rs` | 成本/時間估計 | **重要** — 成本控制 |
| `approval.rs` | 核准閘門（人機互動） | **重要** — 安全檢查 |
| `agent/compaction.rs` | 上下文窗口管理 | **重要** — 長對話 |

### P2 (可選、使用案例特定)
| 模組 | 作用 | 重要性 |
|------|------|--------|
| `tunnel/mod.rs` | 公網暴露（Cloudflare/ngrok）| 可選 — 托管部署 |
| `skills/mod.rs` | SKILL.md 提示擴展 | 可選 — 領域特定 |
| `registry/mod.rs` | WASM 擴展清單 | 可選 — 動態載入 |
| `orchestrator/mod.rs` | 容器編排 API | 可選 — 分佈式模式 |
| `observability/mod.rs` | 事件記錄（noop/log/multi） | 可選 — 監測 |
| `setup/mod.rs` | 上線導覽 | 可選 — 初始配置 |
| `tools/mcp/mod.rs` | MCP 客戶端 | 可選 — 第三方工具 |
| `tools/wasm/mod.rs` | WASM 沙箱工具 | 可選 — 動態工具 |

### P3 (未來、實驗)
| 模組 | 作用 | 重要性 |
|------|------|--------|
| `evaluation/mod.rs` | 成功評估規則 | 實驗 — 成果衡量 |
| `extensions/mod.rs` | WASM 擴展管理器 | 實驗 — 動態載入 |
| `transcription/mod.rs` | 音頻轉錄 | 實驗 — 語音支援 |
| `document_extraction/mod.rs` | PDF/文檔萃取 | 實驗 — 文件理解 |

---

## 關鍵架構決定

1. **雙資料庫後端**: PostgreSQL（生產）+ libSQL/Turso（輕量）。新特性必須同時支持。
2. **防禦深度**: 7 層安全（輸入驗證 → 提示清理 → 工具速率限制 → 容器隔離 → 網路代理 → 憑證注入 → 輸出清理）。
3. **共享推理引擎**: `run_agentic_loop()` 被聊天、任務、容器三種委託共用。
4. **無 .unwrap()**: 所有生產代碼使用 `?` 與適當的錯誤映射。
5. **trait 為先**: 可擴展設計優於硬編碼。
6. **非同步 tokio**: 所有 I/O 均非同步；`Arc<T>` + `RwLock` 共享狀態。
7. **技能零通道**: 技能選擇確定性（無 LLM 呼叫），前 5 個符合條件的技能按檔案順序。
8. **工具工作流**: 執行後清理 → 政策引擎 → 洩露檢測 → 轉義輸出 → 回傳給 LLM。

---

**文檔版本**: 2025-02-01
**對應版本**: IronClaw v0.18.0
**語言**: 繁體中文 (Traditional Chinese)
