# Swarm 架構掃描報告

**掃描日期**: 2026-03-13
**專案**: Swarm (Multi-Agent Orchestration Framework)
**語言**: Rust
**核心類型**: Agent-to-Agent (A2A) + 工作流管理 + MCP Runtime

---

## 1. 專案概覽

Swarm 是一個企業級的多代理編排框架，使用 Rust 實現。它提供了：

- **Agent-to-Agent (A2A) 通信**：HTTP + SSE 雙向通訊協議
- **工作流管理**：基於圖論的多步驟任務編排
- **MCP Runtime**：Model Context Protocol 客戶端集成
- **多代理協作**：BasicAgent、PlannerAgent、ExecutorAgent 三層架構
- **配置驅動**：TOML 配置文件定義代理、工作流、MCP 連接

**典型用場**：
- 複雜決策支援系統（Planner → Executor 模式）
- 多步工作流自動化
- 企業級代理間任務委派
- LLM + 工具編排系統

---

## 2. 目錄結構

```
swarm/
├── Cargo.toml (workspace)
├── agent_factory/                      # 代理工廠模式
│   ├── src/
│   │   ├── lib.rs
│   │   └── agent_factory.rs           # AgentFactory trait 實現
│   └── bin/launch_factory.rs
├── basic_agent/                        # 基礎代理
│   ├── src/
│   │   ├── lib.rs
│   │   └── business_logic/
│   │       ├── mod.rs
│   │       └── basic_agent.rs         # BasicAgent impl (LLM + MCP)
│   └── bin/
│       ├── basic_agent_launch.rs
│       └── simple_a2a_client.rs
├── mcp_runtime/                        # MCP 客戶端封裝
│   ├── src/
│   │   ├── lib.rs
│   │   ├── runtime/
│   │   │   ├── mod.rs
│   │   │   └── mcp_runtime.rs         # McpRuntime: SSE 客戶端
│   │   ├── mcp_client/
│   │   │   ├── mod.rs
│   │   │   └── mcp_client.rs
│   │   ├── mcp_agent_logic/
│   │   │   ├── mod.rs
│   │   │   ├── agent.rs              # McpAgent: 工具調用處理
│   │   │   └── process_response.rs
│   │   └── mcp_tools/
│   │       ├── mod.rs
│   │       └── tools.rs
├── workflow_management/                # 工作流編排引擎
│   ├── src/
│   │   ├── lib.rs
│   │   ├── graph/
│   │   │   ├── mod.rs
│   │   │   ├── graph_orchestrator.rs  # PlanExecutor (狀態機)
│   │   │   ├── a_star.rs             # A* 路徑規劃
│   │   │   └── config.rs
│   │   ├── agent_communication/
│   │   │   ├── mod.rs
│   │   │   └── agent_invoker.rs      # AgentInvoker trait
│   │   ├── tasks/
│   │   │   ├── mod.rs
│   │   │   ├── task_invoker.rs       # TaskInvoker trait
│   │   │   └── condition_evaluator.rs
│   │   └── tools/
│   │       ├── mod.rs
│   │       └── tool_invoker.rs       # ToolInvoker trait
├── planner_agent/                      # 規劃代理 (高層決策)
│   ├── src/
│   │   ├── lib.rs
│   │   └── business_logic/
│   │       ├── mod.rs
│   │       └── planner_agent.rs      # PlannerAgent: 生成執行計畫
│   └── bin/
│       ├── launch_planner_agent.rs
│       └── simple_workflow_agent_client.rs
├── executor_agent/                     # 執行代理 (執行計畫)
│   ├── src/
│   │   ├── lib.rs
│   │   └── business_logic/
│   │       ├── mod.rs
│   │       └── executor_agent.rs     # ExecutorAgent: 執行工作流
│   └── bin/launch_executor_agent.rs
├── resource_invoker/                   # 資源調用器
│   └── src/lib.rs
├── examples/
│   ├── a2a_agent_endpoint/            # RESTful A2A 端點示例
│   │   └── src/
│   │       ├── a2a_agent_endpoint.rs
│   │       └── api/
│   │           ├── mod.rs
│   │           └── endpoint.rs
│   ├── mcp_runtime_endpoint/           # MCP Runtime 端點示例
│   ├── mcp_server/                     # MCP 工具伺服器示例
│   │   ├── src/
│   │   │   ├── main-server.rs
│   │   │   ├── oauth-main-server.rs
│   │   │   └── common/
│   │   │       ├── mod.rs
│   │   │       ├── mcp_tools.rs
│   │   │       ├── customer_mcp_service.rs
│   │   │       ├── general_mcp_service.rs
│   │   │       ├── pdf_mcp_service.rs
│   │   │       ├── scrape_mcp_service.rs
│   │   │       ├── search_mcp_service.rs
│   │   │       └── weather_mcp_service.rs
│   └── mcp_client/
│       └── src/oauth-main-client.rs
├── configuration/                      # 配置管理
│   ├── agent_*.toml                    # 代理配置
│   ├── factory_config.toml
│   └── mcp_runtime_config.toml
└── documentation/
    ├── demo_factory/
    ├── demo_planner_executor_management/
    └── various_agents_configuration_files/
```

---

## 3. 核心 Trait / Struct

### 3.1 Agent 生命週期

```rust
// 來自 swarm_commons (外部 crate)
pub trait Agent: Send + Sync {
    async fn new(
        agent_config: AgentConfig,
        agent_api_key: String,
        mcp_runtime_details: Option<McpRuntimeDetails>,
        evaluation_service: Option<Arc<dyn EvaluationService>>,
        memory_service: Option<Arc<dyn MemoryService>>,
        discovery_service: Option<Arc<dyn DiscoveryService>>,
        workflow_service: Option<Arc<dyn WorkflowServiceApi>>,
    ) -> Result<Self>;

    async fn handle_request(
        &self,
        request: LlmMessage,
        metadata: Option<Map<String, Value>>,
    ) -> Result<ExecutionResult>;
}
```

### 3.2 BasicAgent（基礎代理）

```rust
pub struct BasicAgent {
    llm_interaction: ChatLlmInteraction,  // LLM 通話器（Anthropic/OpenAI）
    mcp_agent: Option<Arc<Mutex<McpAgent>>>,  // MCP 工具代理
}

// 實現流程：
// 1. 接收 LlmMessage (用戶請求)
// 2. 若有 MCP 代理：MCP LLM 調用 + 工具執行
// 3. 若無 MCP 代理：直接 LLM 調用
// 4. 返回 ExecutionResult
```

### 3.3 PlannerAgent（規劃代理）

```rust
pub struct PlannerAgent {
    agent_config: Arc<AgentConfig>,
    llm_interaction: ChatLlmInteraction,
    discovery_service: Arc<dyn DiscoveryService>,  // 服務發現（找 Executor）
    evaluation_service: Option<Arc<dyn EvaluationService>>,
    client: Arc<HttpClient>,  // A2A HTTP 客戶端
}

// 責任：
// - 接收高層任務 (e.g., "計劃年度預算")
// - LLM 生成詳細計畫 (結構化)
// - 通過 A2A 協議委派給 ExecutorAgent
// - 評估執行結果
```

### 3.4 ExecutorAgent（執行代理）

```rust
pub struct ExecutorAgent {
    agent_config: Arc<AgentConfig>,
    workflow_invokers: Arc<WorkFlowInvokers>,
    evaluation_service: Option<Arc<dyn EvaluationService>>,
}

pub struct WorkFlowInvokers {
    pub task_invoker: Arc<dyn TaskInvoker>,
    pub agent_invoker: Arc<dyn AgentInvoker>,
    pub tool_invoker: Arc<dyn ToolInvoker>,
}

// 責任：
// - 接收計畫 (Graph 結構)
// - 透過 PlanExecutor 執行工作流
// - 調用 TaskInvoker、AgentInvoker、ToolInvoker
// - 返回結果給 PlannerAgent
```

### 3.5 工作流編排 (PlanExecutor)

```rust
pub struct PlanExecutor {
    context: PlanContext,  // 狀態 + 圖 + 結果
    task_invoker: Arc<dyn TaskInvoker>,
    agent_invoker: Arc<dyn AgentInvoker>,
    tool_invoker: Arc<dyn ToolInvoker>,
    execution_queue: VecDeque<String>,  // 待執行節點隊列
    dependency_tracker: HashMap<String, usize>,  // 依賴計數
}

// 狀態機：
// Idle → Active → ...
// 每節點可以是：
//   - Task (技能調用)
//   - Agent (代理委派)
//   - Tool (工具使用)
//   - DirectToolUse (直接工具)
//   - DirectTaskExecution (直接技能)
```

### 3.6 MCP Runtime

```rust
pub struct McpRuntime {
    agent_mcp_config: McpRuntimeConfig,
    client: McpClient,  // RoleClient<SseClientTransport>
}

pub struct McpAgent {
    // 封裝 MCP 工具調用邏輯
    // 主要方法：call_tool(tool_name, params) -> Result<CallToolResult>
}

// 責任：
// - 連接至 MCP 伺服器 (SSE 傳輸)
// - 列舉可用工具 (list_tools)
// - 執行工具呼叫 (call_tool)
// - 支援 OAuth 認證
```

### 3.7 A2A 通訊協議

```rust
// 來自 a2a-rs
pub struct HttpClient {
    executor_url: String,  // e.g., "http://executor:8080"
}

pub struct AsyncA2AClient {
    // 管理 HTTP 連線
    // POST /api/tasks (創建任務)
    // GET /api/tasks/{id} (查詢狀態)
    // PUT /api/tasks/{id} (更新)
}

// 消息格式：
// {
//   "role": "user|assistant|system",
//   "content": "...",
//   "parts": [...]  // 多部分消息
// }
```

---

## 4. 啟動流程

### 4.1 BasicAgent 啟動

```
main (bin/basic_agent_launch.rs)
  ↓
AppContext 初始化 (配置 + 日誌)
  ↓
BasicAgent::new(
  - 讀取 AgentConfig (TOML)
  - 初始化 ChatLlmInteraction (API 金鑰)
  - 可選：初始化 McpAgent (連接 MCP 伺服器)
)
  ↓
HTTP 伺服器啟動 (Axum)
  ↓
監聽 POST /api/agents/{id}/handle_request
  ↓
調用 agent.handle_request(message)
  ↓
返回 ExecutionResult (JSON)
```

### 4.2 PlannerAgent → ExecutorAgent 流程

```
PlannerAgent::handle_request(high_level_task)
  ↓
LLM 調用：生成 Graph (計畫結構)
  ↓
A2A 客戶端通訊:
  POST /api/agents/executor/handle_request
  {
    "role": "user",
    "content": "<計畫 JSON>"
  }
  ↓
ExecutorAgent::handle_request(計畫)
  ↓
PlanExecutor::execute_plan()  [狀態機]
  ↓
迭代執行工作流節點:
  - Condition 評估
  - Task / Agent / Tool 調用
  - 結果累積到 activities_outcome
  ↓
返回執行結果
  ↓
PlannerAgent 評估結果
  ↓
最終返回給 Caller
```

### 4.3 MCP 工具調用流程

```
BasicAgent::handle_request()
  ↓
LLM 生成 ToolCall:
  {
    "name": "search_web",
    "arguments": {"query": "..."}
  }
  ↓
McpAgent::call_tool()
  ↓
McpRuntime::initialize_mcp_client_v2()
  ↓
連接 MCP 伺服器 (SSE)
  ↓
rmcp::RoleClient::call_tool()
  ↓
執行工具 (e.g., Web Search, PDF, Weather)
  ↓
返回 CallToolResult
  ↓
LLM 繼續迭代 (Agentic Loop)
```

---

## 5. 資料流 ASCII 圖

### 5.1 三層代理架構

```
┌────────────────────────────────────────────────────────┐
│                      End User                          │
└────────────────────┬─────────────────────────────────┘
                     │
                     ↓
      ┌──────────────────────────────┐
      │     BasicAgent (Simple)      │
      │  LLM + Optional MCP Tools    │
      └──────────────────────────────┘
                     │
         ┌───────────┴───────────┐
         ↓                       ↓
    ┌─────────────┐    ┌──────────────────┐
    │ PlannerAgent│    │ ExecutorAgent    │
    │   (Planner) │◄──►│  (Executor)      │
    └─────────────┘    └──────────────────┘
         │                      │
         │    A2A HTTP          │
         │    (Agent-to-Agent)  │
         │                      ↓
         │            ┌─────────────────┐
         └───────────►│ PlanExecutor    │
                      │  (Graph Engine) │
                      └────────┬────────┘
                               ↓
                    ┌──────────────────────┐
                    │  Invoker Ecosystem   │
                    │  ├─ TaskInvoker     │
                    │  ├─ AgentInvoker    │
                    │  └─ ToolInvoker     │
                    └──────────────────────┘
```

### 5.2 MCP 工具整合

```
┌────────────────┐
│  BasicAgent    │
│   + LLM        │
└────────┬───────┘
         │
         ↓
    ┌─────────────┐
    │ LLM Response│
    │ ToolCall[]  │
    └──────┬──────┘
           │
           ↓
    ┌─────────────────────┐
    │   McpAgent          │
    │  (工具調用器)        │
    └──────┬──────────────┘
           │ SSE
           ↓
    ┌──────────────────────┐
    │  MCP Server          │
    │  (工具提供者)         │
    │  ├─ Web Search       │
    │  ├─ PDF Extract      │
    │  ├─ Weather API      │
    │  └─ Scraping        │
    └─────────────────────┘
           │
           ↓
    ┌─────────────┐
    │   Result    │
    │  返回給 LLM  │
    └─────────────┘
```

### 5.3 工作流執行狀態機

```
                    ┌─────────┐
                    │  Idle   │
                    └────┬────┘
                         │
                         ↓
        ┌────────────────────────────────┐
        │      Active (執行中)            │
        │  1. 評估條件                     │
        │  2. 出隊依賴滿足的節點           │
        │  3. 調用適當的 Invoker          │
        │  4. 累積結果                     │
        └────────────────────────────────┘
                         │
           ┌─────────────┼─────────────┐
           ↓             ↓             ↓
      ┌────────┐   ┌────────┐   ┌────────┐
      │ Success│   │ Failed │   │ Error  │
      └────────┘   └────────┘   └────────┘
           │             │             │
           └─────────────┴─────────────┘
                         ↓
                    ┌─────────┐
                    │  Done   │
                    └─────────┘
```

---

## 6. 子系統清單

### P0 (核心 - 必須運作)

| 子系統 | 模組位置 | 責任 | 依賴 |
|------|---------|------|------|
| **Agent Trait** | swarm_commons (external) | 代理標準化介面 | 無 |
| **ChatLlmInteraction** | llm_api (external) | LLM API 通話 | OpenAI/Anthropic SDK |
| **BasicAgent** | basic_agent/ | 簡單代理實現 (LLM + MCP) | Agent, ChatLlmInteraction |
| **AgentFactory** | agent_factory/ | 工廠模式 - 創建代理 | Agent trait |
| **MCP Runtime** | mcp_runtime/ | MCP 伺服器客戶端 | rmcp (Model Context Protocol SDK) |
| **PlanExecutor** | workflow_management/graph/ | 工作流狀態機 | TaskInvoker, AgentInvoker, ToolInvoker |
| **A2A Communication** | a2a-rs (external) | Agent-to-Agent HTTP 協議 | reqwest, tokio |
| **PlannerAgent** | planner_agent/ | 規劃 + 委派 | Agent, DiscoveryService, A2AClient |
| **ExecutorAgent** | executor_agent/ | 執行工作流 | Agent, PlanExecutor, WorkFlowInvokers |

### P1 (重要 - 功能完整性)

| 子系統 | 模組位置 | 責任 | 依賴 |
|------|---------|------|------|
| **Configuration** | configuration/ | TOML 配置載入 | toml crate |
| **DiscoveryService** | swarm_commons (external) | 服務發現 (找 Executor URL) | 無 |
| **EvaluationService** | swarm_commons (external) | 評估執行品質 | 無 |
| **MemoryService** | swarm_commons (external) | 記憶管理 | 無 |
| **WorkFlowServiceApi** | swarm_commons (external) | 工作流編排 API | 無 |
| **A* Pathfinding** | workflow_management/graph/a_star.rs | 最優節點執行順序 | 無 |
| **Condition Evaluator** | workflow_management/tasks/ | 評估條件表達式 | serde_json |
| **TaskInvoker** | workflow_management/tasks/ | 技能呼叫抽象 | 無 |
| **AgentInvoker** | workflow_management/agent_communication/ | 代理呼叫抽象 | 無 |
| **ToolInvoker** | workflow_management/tools/ | 工具呼叫抽象 | 無 |

### P2 (可選 - 示例/文檔)

| 子系統 | 模組位置 | 責任 | 依賴 |
|------|---------|------|------|
| **A2A Endpoint** | examples/a2a_agent_endpoint/ | RESTful 代理端點 | Axum |
| **MCP Runtime Endpoint** | examples/mcp_runtime_endpoint/ | MCP 運行時 API | Axum, MCP SDK |
| **MCP Server** | examples/mcp_server/ | MCP 工具伺服器示例 | rmcp, reqwest |
| **Customer MCP Service** | examples/mcp_server/common/ | MCP 客戶服務工具 | 無 |
| **General MCP Service** | examples/mcp_server/common/ | MCP 通用工具 | 無 |
| **PDF MCP Service** | examples/mcp_server/common/ | PDF 解析工具 | 無 |
| **Search MCP Service** | examples/mcp_server/common/ | Web 搜尋工具 | 無 |
| **Weather MCP Service** | examples/mcp_server/common/ | 天氣 API 工具 | 無 |
| **Resource Invoker** | resource_invoker/ | 資源調用抽象 | 無 |
| **Documentation** | documentation/ | 配置示例 + 演示 | 無 |

---

## 7. 技術棧

- **主語言**：Rust 2021 Edition
- **異步運行時**：Tokio
- **Web 框架**：Axum (A2A 端點)
- **HTTP 客戶端**：Reqwest
- **配置管理**：TOML, Serde
- **日誌/追蹤**：Tracing, Tracing-Subscriber
- **MCP SDK**：rmcp (Model Context Protocol)
- **A2A SDK**：a2a-rs (Agent-to-Agent)
- **序列化**：Serde JSON
- **錯誤處理**：Anyhow, ThisError
- **UUID 生成**：uuid crate
- **認證**：OAuth2, OpenID Connect

---

## 8. 關鍵設計模式

### 8.1 Agent 統一介面
所有代理 (BasicAgent, PlannerAgent, ExecutorAgent) 實現共同的 `Agent` trait，支持可互換性。

### 8.2 工作流即圖
工作流表示為有向圖 (DAG)，每個節點是 Task/Agent/Tool，邊表示依賴。

### 8.3 Invoker 抽象
TaskInvoker、AgentInvoker、ToolInvoker 為 trait，允許多種實現 (HTTP、進程、本地)。

### 8.4 A2A 協議
BasicAgent ↔ PlannerAgent ↔ ExecutorAgent 通過 HTTP + A2A 標準通訊，支持跨進程/機器。

### 8.5 MCP 集成
MCP 工具以可插拔方式整合，BasicAgent 可選帶 MCP 運行時。

### 8.6 配置驅動
通過 TOML 配置：
- 代理模型、API 金鑰、MCP 連接
- 工作流圖定義
- 服務發現端點

---

## 9. 啟動命令示例

```bash
# 啟動 BasicAgent
cargo run --bin basic_agent_launch

# 啟動 PlannerAgent
cargo run --bin launch_planner_agent

# 啟動 ExecutorAgent
cargo run --bin launch_executor_agent

# 啟動 MCP 伺服器 (工具提供者)
cargo run --example mcp_server

# 構建並執行測試
cargo test
```

---

## 10. 擴展點

1. **新代理類型**：實現 Agent trait + 對應 config
2. **新工具**：實現 MCP Service (e.g., DataBase, FileSystem)
3. **新 Invoker**：實現 TaskInvoker/AgentInvoker/ToolInvoker
4. **新工作流**：定義 Graph TOML 並載入
5. **認證層**：通過 OAuth2 / API Key 管理

---

## 11. 已知限制

- 工作流圖目前不支持迴圈 (DAG 只)
- MCP 伺服器必須外部部署
- A2A 通訊基於 HTTP (無 WebSocket 重連)
- 無内建分散式交易支援 (需外部協調)

---

**文檔版本**: 1.0
**最後更新**: 2026-03-13
