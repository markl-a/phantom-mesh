# Swarm 深度技術分析

> 分析日期: 2026-03-13 (v2 — 深度翻倍)
> 專案位置: `LLM-Cluster-Project/references/swarm/`
> 語言: Rust (Cargo workspace, edition 2024)
> 作者: fcn06 (GitHub)

---

## 目錄

1. [專案結構與 Crate 依賴圖](#1-專案結構與-crate-依賴圖)
2. [進入點與啟動流程](#2-進入點與啟動流程)
3. [DAG Workflow Engine — 拓撲排序 + 參數插值](#3-dag-workflow-engine--拓撲排序--參數插值)
4. [Three-tier Invoker 架構 (Agent/Tool/Task)](#4-three-tier-invoker-架構-agenttool-task)
5. [5-State Agent State Machine (MCP Agent)](#5-5-state-agent-state-machine-mcp-agent)
6. [LLM-as-Judge 雙層自我修正](#6-llm-as-judge-雙層自我修正)
7. [MCP Resilient Runtime](#7-mcp-resilient-runtime)
8. [Agent Factory 動態代理創建](#8-agent-factory-動態代理創建)
9. [通訊協議 (A2A + MCP)](#9-通訊協議-a2a--mcp)
10. [工作流 JSON Schema 與範例](#10-工作流-json-schema-與範例)
11. [錯誤處理深度分析](#11-錯誤處理深度分析)
12. [效能特徵分析](#12-效能特徵分析)
13. [Clawtex 差距對比與 Rust 實作建議](#13-clawtex-差距對比與-rust-實作建議)

---

## 1. 專案結構與 Crate 依賴圖

### 1.1 目錄樹

```
swarm/
  Cargo.toml                      # Workspace 根：定義所有成員 crate 及共享依賴
  configuration/                   # 所有代理的 TOML 配置檔 + LLM prompts
    agent_basic_config.toml        # BasicAgent 配置
    agent_planner_config.toml      # PlannerAgent 配置
    agent_executor_config.toml     # ExecutorAgent 配置
    agent_judge_config.toml        # Judge Agent 配置
    agent_identity_config.toml     # Identity Agent 配置（未來 SSI 用途）
    factory_config.toml            # AgentFactory 配置
    mcp_runtime_config.toml        # MCP Runtime 配置
    prompts/
      detailed_workflow_agent_prompt.txt   # 動態工作流生成的 prompt
      high_level_plan_workflow_agent_prompt.txt
      judge_agent_prompt.txt               # LLM-as-Judge 評估 prompt

  mcp_runtime/                     # MCP 客戶端 + 代理邏輯層
    src/
      lib.rs                       # 匯出 4 個子模組
      mcp_client/mcp_client.rs     # SSE 傳輸層 MCP 客戶端
      mcp_agent_logic/agent.rs     # McpAgent：狀態機 agent loop (5 狀態)
      mcp_agent_logic/process_response.rs  # LLM 回應處理 (finish_reason 分支)
      mcp_tools/tools.rs           # rmcp Tool -> LLM API Tool 轉碼
      runtime/mcp_runtime.rs       # McpRuntime：SSE 客戶端封裝 + 工具執行

  workflow_management/             # 工作流引擎核心
    src/
      graph/
        graph_orchestrator.rs      # PlanExecutor：DAG 執行器（核心，378 行）
        a_star.rs                  # A* 尋路演算法（66 行）
        config.rs                  # 從 JSON 載入 Graph
      tasks/
        task_invoker.rs            # TaskInvoker trait
        condition_evaluator.rs     # 條件邊評估器（== / != 判斷）
      agent_communication/
        agent_invoker.rs           # AgentInvoker trait
      tools/
        tool_invoker.rs            # ToolInvoker trait
    example_workflow/
      multi_agent_workflow.json    # 範例工作流 JSON

  basic_agent/                     # 基本 Domain Agent
    bin/basic_agent_launch.rs      # 啟動入口
    bin/simple_a2a_client.rs       # A2A 客戶端測試工具
    src/business_logic/basic_agent.rs  # BasicAgent 實作

  planner_agent/                   # 規劃者代理
    bin/launch_planner_agent.rs    # 啟動入口
    bin/simple_workflow_agent_client.rs
    src/business_logic/planner_agent.rs  # PlannerAgent（448 行）— 3 策略 + Judge

  executor_agent/                  # 執行者代理
    bin/launch_executor_agent.rs   # 啟動入口
    src/business_logic/executor_agent.rs  # ExecutorAgent + WorkFlowInvokers

  agent_factory/                   # 代理工廠
    bin/launch_factory.rs          # 完整 demo 啟動入口
    src/agent_factory.rs           # AgentFactory（344 行）— 含 Configurator trait

  resource_invoker/                # A2A/MCP/Task 調用器具體實作
    src/lib.rs                     # A2AAgentInvoker(208行), McpRuntimeToolInvoker, GreetTask

  examples/                        # 範例程式
    a2a_agent_endpoint/            # REST API 端點範例
    mcp_runtime_endpoint/          # MCP Runtime REST 端點
    mcp_server/                    # MCP Server 實作（天氣/客戶/搜索/PDF 工具）
    mcp_client/                    # OAuth MCP 客戶端範例

  documentation/                   # 文件 + demo 腳本 + 工作流 JSON Schema
    workflow_json_schema/workflow_json_schema_v2.json
```

### 1.2 Crate 依賴圖

```
                    swarm_commons (外部 git)
                   ╱    agent_core
                  ╱     configuration
                 ╱      llm_api
                ╱       agent_models
               ╱
              ╱    swarm_services (外部 git)
             ╱     agent_service_adapters
            ╱
           ╱
┌─────────────────────┐
│   workflow_management│ ← 純邏輯層：DAG + Invoker traits
│   (無外部依賴)       │
└────────┬────────────┘
         │ trait impls
         ▼
┌─────────────────────┐     ┌──────────────┐
│  resource_invoker   │────▶│  mcp_runtime │
│  (A2A + MCP + Task  │     │  (SSE 客戶端) │
│   具體實作)          │     └──────────────┘
└────────┬────────────┘
         │
    ┌────┴────┬──────────────┐
    ▼         ▼              ▼
basic_agent  planner_agent  executor_agent
    │         │              │
    └────┬────┴──────────────┘
         ▼
   agent_factory (統一啟動 + 管理)
```

### 1.3 關鍵外部依賴

| 依賴 | 用途 | 版本備註 |
|------|------|---------|
| `rmcp` (rust-sdk) | MCP 協議客戶端/伺服器，SSE 傳輸 | 最新 main |
| `a2a-rs` | Agent-to-Agent 協議（Google A2A v0.3），HTTP server/client | 含 TaskState |
| `swarm_commons` (git) | `agent_core`, `configuration`, `llm_api`, `agent_models` | 外部倉庫 |
| `swarm_services` (git) | `agent_service_adapters`（Discovery/Memory/Evaluation HTTP 適配器）| 外部倉庫 |
| `axum` | HTTP 框架（A2A Server） | |
| `tokio` | 非同步執行時 | |
| `redb` | 嵌入式資料庫（用於評估日誌） | |
| `regex` | 參數插值 `{{}}` 語法解析 | |
| `thiserror` | 結構化錯誤枚舉 | PlanExecutorError |
| `tracing` | 結構化日誌 | |
| `bon` | Builder pattern 生成 | |
| `uuid` | 請求/任務 ID 生成 | |

---

## 2. 進入點與啟動流程

Swarm 有兩條主要啟動路徑：

### 2.1 路徑一：獨立進程模式

每個代理作為獨立的 tokio 進程啟動，各自監聽不同 HTTP 端口。

**BasicAgent 啟動** (`basic_agent/bin/basic_agent_launch.rs`):

```rust
// 1. 解析 CLI 參數（config_file, log_level）
let args = Args::parse();

// 2. 從 TOML 載入配置
let basic_agent_config = AgentConfig::load_agent_config(&args.config_file)?;

// 3. 從環境變數讀取 LLM API Key
let agent_api_key = env::var("LLM_A2A_API_KEY")?;

// 4. 創建 Agent 實例（如果配置了 MCP，會連接 MCP Server）
let agent = BasicAgent::new(basic_agent_config.clone(), agent_api_key,
    None, None, None, None, None).await?;

// 5. 創建 A2A Server 並啟動 HTTP 服務
let server = AgentServer::<BasicAgent>::new(basic_agent_config, agent, None).await?;
server.start_http().await?;
```

**啟動時序圖**:

```
┌─────────┐    ┌───────────┐    ┌───────────┐    ┌───────────┐
│ CLI Args │───▶│ TOML Load │───▶│ Agent::new│───▶│AgentServer│
│  parse   │    │ AgentConfig│   │(+MCP opt) │    │ start_http│
└─────────┘    └───────────┘    └───────────┘    └───────────┘
                                      │                │
                              ┌───────┴───────┐   axum HTTP
                              │McpAgent::new()│   listener
                              │ SSE connect   │
                              │ list_tools()  │
                              └───────────────┘
```

### 2.2 路徑二：AgentFactory 模式

`agent_factory/bin/launch_factory.rs` — 一個進程管理所有代理的完整生命週期：

```rust
// agent_factory/bin/launch_factory.rs
#[tokio::main]
async fn main() {
    // 1. 載入 factory_config.toml
    let factory_config = FactoryConfig::load_factory_config(&args.config_file)?;

    // 2. 建立外部服務（Discovery、Memory、Evaluation）
    let discovery_service = setup_discovery_service(&url).await;
    let evaluation_service = setup_evaluation_service(&url).await;

    // 3. 註冊 Tasks 和 Tools 到 Discovery Service
    register_tasks(discovery_service.clone()).await?;
    register_tools(mcp_config_path, discovery_service.clone()).await?;

    // 4. 設定三層 Invokers
    let task_invoker = setup_task_invoker().await?;
    let tool_invoker = setup_tool_invoker(mcp_config_path).await?;
    let agent_invoker = setup_agent_invoker_v2(discovery_service.clone()).await?;

    // 5. 建立 AgentFactory
    let workflow_invokers = WorkFlowInvokers::init(
        task_invoker, agent_invoker, tool_invoker).await?;
    let agent_factory = AgentFactory::new(factory_config, discovery_service,
        memory_service, evaluation_service, Some(Arc::new(workflow_invokers)));

    // 6. 依序啟動代理（每個 tokio::spawn）
    // BasicAgent (with MCP) -> Executor -> Planner
    agent_factory.launch_agent(&config, Some(&mcp_config), AgentType::Specialist).await?;
    agent_factory.launch_agent(&config_executor, None, AgentType::Executor).await?;
    agent_factory.launch_agent(&config_planner, None, AgentType::Planner).await?;

    // 7. join_all 等待所有代理完成
    join_all(handles).await;
}
```

**資料流圖**:

```
           ┌──────────────────────────────────────────────┐
           │               AgentFactory                    │
           │                                               │
           │  FactoryConfig ──▶ create_agent_config()      │
           │       │                                       │
           │       ▼                                       │
           │  AgentConfigurator trait                       │
           │  ├── SpecialistAgentConfigurator              │
           │  ├── PlannerAgentConfigurator                 │
           │  └── ExecutorAgentConfigurator                │
           │       │                                       │
           │       ▼                                       │
           │  launch_agent()                               │
           │  ├── BasicAgent::new() → tokio::spawn         │
           │  ├── PlannerAgent::new() → tokio::spawn       │
           │  └── ExecutorAgent::new() → tokio::spawn      │
           │       │                                       │
           │       ▼                                       │
           │  refresh_agents() // 每次啟動後自動刷新        │
           └──────────────────────────────────────────────┘
```

每次啟動代理後，Factory 呼叫 `refresh_agents()` 更新 Discovery 的代理列表，確保新啟動的代理立即可被其他代理發現。

**Factory 啟動流程中的 Agent 刷新** (`agent_factory.rs` 第 325-332 行):

```rust
// 啟動後自動刷新代理列表
if let Some(ws_arc) = &self.workflow_service {
    let workflow_service_invoker = ws_arc.as_ref()
        .as_any()
        .downcast_ref::<WorkFlowInvokers>()
        .expect("WorkflowServiceApi is not a WorkFlowInvokers");
    workflow_service_invoker.refresh_agents().await?;
}
```

> 注意 `downcast_ref` 的使用 — Swarm 透過 `as_any()` 從 `dyn WorkflowServiceApi` 向下轉型到具體的 `WorkFlowInvokers`，這是 Rust 中在 trait object 上實現多態下行轉型的標準模式。

---

## 3. DAG Workflow Engine — 拓撲排序 + 參數插值

### 3.1 核心資料結構

**檔案**: `workflow_management/src/graph/graph_orchestrator.rs` 第 42-49 行

```rust
pub struct PlanExecutor {
    context: PlanContext,
    task_invoker: Arc<dyn TaskInvoker>,
    agent_invoker: Arc<dyn AgentInvoker>,
    tool_invoker: Arc<dyn ToolInvoker>,
    execution_queue: VecDeque<String>,            // 就緒佇列（入度歸零的節點）
    dependency_tracker: HashMap<String, usize>,   // 節點 -> 未完成依賴計數
}
```

**`PlanContext`** (定義於 `agent_models::graph::graph_definition`):

```rust
pub struct PlanContext {
    pub plan_state: PlanState,
    pub graph: Graph,                                    // 節點 + 邊
    pub current_step_id: Option<String>,                 // 當前正在執行的節點
    pub activities_outcome: HashMap<String, String>,     // 節點 ID -> 執行結果
    pub final_outcome: String,                           // 葉節點結果合併
    pub user_query: String,                              // 原始使用者查詢
}
```

### 3.2 六狀態執行狀態機

**檔案**: `graph_orchestrator.rs` 第 76-89 行

```rust
pub async fn execute_plan(&mut self)
    -> Result<(String, HashMap<String, String>), PlanExecutorError>
{
    self.context.plan_state = PlanState::Idle;
    loop {
        match self.context.plan_state.clone() {
            PlanState::Idle            => self.handle_idle_state()?,
            PlanState::Initializing    => self.handle_initializing_state()?,
            PlanState::DecidingNextStep => self.handle_deciding_next_step_state()?,
            PlanState::ExecutingStep   => self.handle_executing_step_state().await?,
            PlanState::Completed       => return self.handle_completion_state(),
            PlanState::Failed(reason)  => return self.handle_failure_state(reason),
            _                          => return Err(PlanExecutorError::InvalidState),
        }
    }
}
```

**狀態轉換圖**:

```
                    ┌───────────┐
                    │   Idle    │
                    └─────┬─────┘
                          │
                    ┌─────▼─────┐
                    │Initializing│  ← 計算每個節點的入度
                    │ (拓撲初始化)│    入度=0 加入 execution_queue
                    └─────┬─────┘    偵測環形依賴 → CyclicDependency
                          │
               ┌──────────▼──────────┐
               │  DecidingNextStep   │
               │ pop_front() 從佇列  │
               └──┬────────────┬─────┘
                  │            │
          佇列有節點     佇列空 + 全部完成
                  │            │
            ┌─────▼─────┐ ┌───▼───┐
            │ExecutingStep│ │Completed│
            │ (分派執行)  │ └───────┘
            └─────┬─────┘
                  │
        update_downstream_dependencies()
        減少下游節點入度，入度=0 → 加入佇列
                  │
                  └──▶ DecidingNextStep（迴圈）
```

### 3.3 拓撲排序初始化 — 入度計算與環偵測

**檔案**: `graph_orchestrator.rs` 第 96-120 行

```rust
fn handle_initializing_state(&mut self) -> Result<(), PlanExecutorError> {
    // 計算每個節點的入度（被幾條邊指向）
    for (node_id, _node) in &self.context.graph.nodes {
        let dep_count = self.context.graph.edges.iter()
            .filter(|e| e.target == *node_id)
            .count();
        self.dependency_tracker.insert(node_id.clone(), dep_count);
    }

    // 入度為 0 的節點 → 立即可執行
    for (node_id, count) in &self.dependency_tracker {
        if *count == 0 {
            self.execution_queue.push_back(node_id.clone());
        }
    }

    // 有節點但佇列空 → 環形依賴
    if self.execution_queue.is_empty() && !self.context.graph.nodes.is_empty() {
        return Err(PlanExecutorError::CyclicDependency);
    }

    self.context.plan_state = PlanState::DecidingNextStep;
    Ok(())
}
```

**效能分析**: 入度計算為 O(N * E)，其中 N = 節點數, E = 邊數。對於典型工作流（<20 個節點）完全足夠。若需擴展到大規模 DAG，可改為一次遍歷邊建立 `in_degree: HashMap`。

**環偵測限制**: 僅檢測「初始化時入度全非零」的情況。如果圖有部分可達的環（例如 A→B, B→C, C→B, 但 A 入度=0），則不會在初始化時偵測到，而是會在 `DecidingNextStep` 中因「佇列空但未全部完成」而進入 `Failed` 狀態。

### 3.4 依賴追蹤與下游更新

**檔案**: `graph_orchestrator.rs` 第 245-272 行

```rust
fn update_downstream_dependencies(
    &mut self,
    completed_node_id: &str,
    result: &str,
) -> Result<(), PlanExecutorError> {
    for edge in &self.context.graph.edges {
        if edge.source == *completed_node_id {
            // 條件邊評估
            let mut condition_met = true;
            if let Some(condition) = &edge.condition {
                let mut dependencies = HashMap::new();
                let result_value = serde_json::from_str(result)
                    .unwrap_or_else(|_| Value::String(result.to_string()));
                dependencies.insert(completed_node_id.to_string(), result_value);
                condition_met = evaluate_condition(condition, &dependencies);
            }

            if condition_met {
                if let Some(count) = self.dependency_tracker.get_mut(&edge.target) {
                    *count -= 1;       // 減少依賴計數
                    if *count == 0 {
                        self.execution_queue.push_back(edge.target.clone()); // 就緒！
                    }
                }
            }
        }
    }
    Ok(())
}
```

**關鍵設計**: 當條件邊評估為 `false` 時，下游節點的依賴計數不會減少，這表示該節點將永遠無法就緒。這是「條件分支」的實現方式 — 被跳過的分支上的所有節點都不會執行。但也意味著最終 `activities_outcome.len() != nodes.len()`，會導致 `DecidingNextStep` 判定為 `Failed`。

**改進機會**: 條件分支跳過的節點應標記為 "skipped" 而非讓整個計劃失敗。

### 3.5 三種活動類型的執行分派

**檔案**: `graph_orchestrator.rs` 第 137-243 行

```rust
async fn handle_executing_step_state(&mut self) -> Result<(), PlanExecutorError> {
    let node_id = self.context.current_step_id.as_ref().cloned()
        .ok_or(PlanExecutorError::InvalidState)?;
    let node = self.context.graph.nodes.get(&node_id).cloned()
        .ok_or_else(|| PlanExecutorError::MissingNode(node_id.clone()))?;

    let NodeType::Activity(original_activity) = &node.node_type;

    // 參數插值 — 用前序步驟的結果替換 {{}} 佔位符
    let activity = self.interpolate_parameters(original_activity)?;

    let result = match activity.activity_type {
        // 1. 委派給代理
        ActivityType::DelegationAgent => {
            let agent_id = activity.assigned_agent_id_preference.as_ref()
                .ok_or_else(|| PlanExecutorError::AgentRunnerNotFound(...))?
                .clone();
            let mut message = format!("Here is the user_query :");
            message.push_str(&activity.description);
            if let Some(context) = &activity.agent_context {
                message.push_str(&format!(
                    "\nHere are contextual information...: {}\n",
                    context.to_string()
                ));
            }
            let skill = activity.skill_to_use.clone().unwrap_or_default();
            self.agent_invoker.interact(agent_id, message, skill).await
                .map_err(|e| PlanExecutorError::ExecutionFailed(e.to_string()))?
                .to_string()
        }
        // 2. 直接工具調用
        ActivityType::DirectToolUse => {
            let tool_id = activity.tool_to_use.as_ref()
                .ok_or_else(|| PlanExecutorError::MissingTool(...))?
                .clone();
            let params = activity.tool_parameters.unwrap_or_else(|| Value::Null);
            self.tool_invoker.invoke(tool_id, &params).await
                .map_err(|e| PlanExecutorError::ExecutionFailed(e.to_string()))?
                .to_string()
        }
        // 3. 直接任務執行
        ActivityType::DirectTaskExecution => {
            let tasks = activity.tasks.as_ref()
                .ok_or_else(|| PlanExecutorError::MissingTask(...))?;
            let task_config = tasks.get(0)
                .ok_or_else(|| PlanExecutorError::MissingTask(...))?;
            let task_id = task_config.task_to_use.as_ref().cloned().unwrap_or_default();
            let params = task_config.task_parameters.clone();
            self.task_invoker.invoke(task_id, &params).await
                .map_err(|e| PlanExecutorError::ExecutionFailed(e.to_string()))?
                .to_string()
        }
    };

    // 儲存結果
    self.context.activities_outcome.insert(node_id.clone(), result.clone());
    // 更新下游依賴
    self.update_downstream_dependencies(&node_id, &result)?;
    self.context.plan_state = PlanState::DecidingNextStep;
    Ok(())
}
```

### 3.6 參數插值系統 (`{{}}` 語法)

**檔案**: `graph_orchestrator.rs` 第 308-377 行

```rust
fn interpolate_parameters(
    &self,
    activity: &Activity,
) -> Result<Activity, PlanExecutorError> {
    let mut hydrated_activity = activity.clone();
    let re = Regex::new(r"\{\{([^}]+)\}\}").unwrap();

    // 解析路徑：{{activity_id.activity_output}} → 取前序步驟結果
    let get_interpolated_value = |path: &str| -> Result<String, PlanExecutorError> {
        let source_id = path.split('.').next().unwrap_or("");
        if source_id.is_empty() {
            return Err(PlanExecutorError::InterpolationFailed(
                "Invalid interpolation path: empty source ID".to_string(),
            ));
        }
        self.context.activities_outcome.get(source_id).cloned()
            .ok_or_else(|| PlanExecutorError::InterpolationFailed(
                format!("Dependency result for '{}' not found for activity '{}'",
                    source_id, activity.id)
            ))
    };

    // 閉包：替換 JSON 物件中的 {{}} 佔位符
    let interpolator = |json_value: &mut Value| {
        if let Value::Object(map) = json_value {
            for (_, value) in map.iter_mut() {
                if let Value::String(s) = value {
                    if s.contains("{{") {
                        if let Some(caps) = re.captures(s) {
                            if let Some(path) = caps.get(1) {
                                match get_interpolated_value(path.as_str()) {
                                    Ok(interpolated_val) => {
                                        // 嘗試解析為 JSON，否則作為字串
                                        *value = serde_json::from_str(&interpolated_val)
                                            .unwrap_or(Value::String(interpolated_val));
                                    }
                                    Err(e) => {
                                        *value = Value::String(format!("ERROR: {}", e));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    };

    // 對三種參數欄位分別插值
    if let Some(tool_params) = &mut hydrated_activity.tool_parameters {
        interpolator(tool_params);    // 工具參數
    }
    if let Some(tasks) = &mut hydrated_activity.tasks {
        for task_config in tasks.iter_mut() {
            interpolator(&mut task_config.task_parameters);  // 任務參數
        }
    }
    if let Some(agent_context) = &mut hydrated_activity.agent_context {
        interpolator(agent_context);  // 代理上下文
    }

    Ok(hydrated_activity)
}
```

**插值限制分析**:
1. 只支援頂層字串值的替換（不支援巢狀 JSON 路徑如 `{{step1.response.data.name}}`）
2. `re.captures()` 只匹配第一個 `{{}}`，如果一個字串中有多個佔位符，只會替換第一個
3. 插值失敗時不會中斷執行，而是將 `ERROR:` 訊息注入值中 — 靜默失敗
4. `Regex::new()` 在每次呼叫時重新編譯 — 效能開銷（應使用 `lazy_static!` 或 `OnceLock`）

### 3.7 葉節點結果聚合

**檔案**: `graph_orchestrator.rs` 第 274-301 行

```rust
fn handle_completion_state(&mut self)
    -> Result<(String, HashMap<String, String>), PlanExecutorError>
{
    // 找出葉節點（不是任何邊的 source 的節點）
    let all_node_ids: HashSet<String> = self.context.graph.nodes.keys().cloned().collect();
    let source_node_ids: HashSet<String> =
        self.context.graph.edges.iter().map(|e| e.source.clone()).collect();
    let leaf_node_ids: Vec<String> =
        all_node_ids.difference(&source_node_ids).cloned().collect();

    // 合併葉節點結果
    let final_results: Vec<String> = leaf_node_ids.iter()
        .filter_map(|id| self.context.activities_outcome.get(id))
        .cloned()
        .collect();

    self.context.final_outcome = final_results.join("\n");
    Ok((self.context.final_outcome.clone(), self.context.activities_outcome.clone()))
}
```

**設計特點**: 最終結果是所有葉節點結果的 `join("\n")`。回傳 tuple 包含 final_outcome 和完整的 activities_outcome HashMap，讓呼叫者可以存取中間步驟的結果。

### 3.8 條件邊評估器

**檔案**: `workflow_management/src/tasks/condition_evaluator.rs` 全檔 (30 行)

```rust
pub fn evaluate_condition(condition: &str, dependencies: &HashMap<String, Value>) -> bool {
    let mut replaced_condition = condition.to_string();
    if let Some(result_value) = dependencies.values().next() {
        let result_str = result_value.to_string();
        replaced_condition = replaced_condition
            .replace("result", &format!("'{}'", result_str.trim_matches('"')));
    }

    if replaced_condition.contains("==") {
        let parts: Vec<&str> = replaced_condition.split("==").map(|s| s.trim()).collect();
        if parts.len() == 2 {
            return parts[0].trim_matches('\'') == parts[1].trim_matches('\'');
        }
    } else if replaced_condition.contains("!=") {
        let parts: Vec<&str> = replaced_condition.split("!=").map(|s| s.trim()).collect();
        if parts.len() == 2 {
            return parts[0].trim_matches('\'') != parts[1].trim_matches('\'');
        }
    }
    true  // 預設為 true（未知條件不阻擋執行）
}
```

**限制**: 僅支援 `==` 和 `!=` 兩種運算子，無 `>`, `<`, `contains`, `matches` 等。`dependencies.values().next()` 只取第一個值，多依賴時會忽略其他值。

### 3.9 A* 尋路演算法

**檔案**: `workflow_management/src/graph/a_star.rs` 全檔 (66 行)

```rust
pub fn a_star(graph: &Graph, start: &str, goal: &str) -> Option<Vec<String>> {
    let mut dist: HashMap<String, usize> = graph.nodes.keys()
        .map(|id| (id.clone(), usize::MAX)).collect();
    let mut came_from: HashMap<String, String> = HashMap::new();
    let mut pq = BinaryHeap::new();

    dist.insert(start.to_string(), 0);
    pq.push(State { cost: 0, position: start.to_string() });

    while let Some(State { cost, position }) = pq.pop() {
        if position == goal { /* reconstruct path */ }
        if cost > dist[&position] { continue; }
        for edge in &graph.edges {
            if edge.source == position {
                let new_cost = cost + 1;  // 邊權重固定為 1
                if new_cost < dist[&neighbor] { /* update */ }
            }
        }
    }
    None
}
```

**用途**: 主要用於分析/除錯，在工作流圖中尋找兩個節點之間的最短路徑。邊權重固定為 1。使用 `BinaryHeap` + 自訂 `State` struct 實現優先佇列。

---

## 4. Three-tier Invoker 架構 (Agent/Tool/Task)

### 4.1 Trait 定義

三個 trait 分別定義於 `workflow_management` crate 中：

**AgentInvoker** (`workflow_management/src/agent_communication/agent_invoker.rs`):

```rust
#[async_trait]
pub trait AgentInvoker: Send + Sync + 'static {
    async fn interact(&self, agent_id: String, message: String, skill: String)
        -> anyhow::Result<serde_json::Value>;
    fn as_any(&self) -> &dyn Any;  // 用於向下轉型
}
```

**ToolInvoker** (`workflow_management/src/tools/tool_invoker.rs`):

```rust
#[async_trait]
pub trait ToolInvoker: Send + Sync {
    async fn invoke(&self, tool_id: String, params: &Value)
        -> anyhow::Result<serde_json::Value>;
}
```

**TaskInvoker** (`workflow_management/src/tasks/task_invoker.rs`):

```rust
#[async_trait]
pub trait TaskInvoker: Send + Sync {
    async fn invoke(&self, task_id: String, params: &Value)
        -> anyhow::Result<Value>;
}
```

### 4.2 具體實作

**檔案**: `resource_invoker/src/lib.rs`

| Trait | 實作 | 行數 | 通訊方式 |
|-------|------|------|---------|
| `AgentInvoker` | `A2AAgentInvoker` | 第 31-208 行 | A2A HTTP → 遠端代理 |
| `ToolInvoker` | `McpRuntimeToolInvoker` | 第 231-308 行 | MCP SSE → MCP Server |
| `TaskInvoker` | `GreetTask` | 第 210-229 行 | 本地 Rust 函數 |

**A2AAgentInvoker 深度分析** (`resource_invoker/src/lib.rs` 第 31-208 行):

```rust
pub struct A2AAgentInvoker {
    agents_references: Arc<RwLock<Vec<AgentReference>>>,      // 動態代理列表
    client_agents: Arc<RwLock<HashMap<String, A2AAgentInteraction>>>,  // A2A 連接池
    evaluation_service: Option<Arc<dyn EvaluationService>>,
    memory_service: Option<Arc<dyn MemoryService>>,
    discovery_service_client: Arc<dyn DiscoveryService>,      // 用於刷新代理列表
}
```

**關鍵設計點**:
1. **RwLock 用於代理列表**: 讀操作（`interact`）使用讀鎖，寫操作（`refresh_agents`）使用寫鎖，允許多個並行的 `interact` 呼叫
2. **Discovery Service 驅動**: `new_with_discovery()` 在初始化時從 Discovery Service 獲取代理列表
3. **技能匹配**: `find_agent_with_skill()` 先按技能匹配，無匹配時回退到預設代理

**McpRuntimeToolInvoker** (`resource_invoker/src/lib.rs` 第 231-308 行):

```rust
pub struct McpRuntimeToolInvoker {
    mcp_runtime: Arc<McpRuntime>,
}

#[async_trait]
impl ToolInvoker for McpRuntimeToolInvoker {
    async fn invoke(&self, tool_id: String, params: &Value) -> anyhow::Result<Value> {
        let arguments_map = from_value(params.clone())?;
        let call_tool_request_param = CallToolRequestParam {
            name: tool_id.into(),
            arguments: Some(arguments_map),
        };
        let tool_result = self.mcp_runtime.get_client()?
            .call_tool(call_tool_request_param).await?;
        let tool_result_value = serde_json::to_value(&tool_result.content)?;
        Ok(tool_result_value)
    }
}
```

**工具轉碼**: `transcode_tools()` 方法將 `rmcp::model::Tool` 轉換為 `llm_api::tools::Tool`，處理 schema 結構差異：

```rust
pub fn transcode_tools(rmcp_tools: Vec<RmcpTool>) -> anyhow::Result<Vec<Tool>> {
    rmcp_tools.into_iter().map(|tool| {
        let properties_map = tool.input_schema.as_ref().clone();
        let properties = properties_map.get("properties");
        Ok(Tool {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: tool.name.to_string(),
                description: tool.description
                    .ok_or_else(|| anyhow!("Tool description missing"))?.to_string(),
                parameters: FunctionParameters {
                    r#type: "object".to_string(),
                    properties: properties.cloned()
                        .unwrap_or_else(|| Value::Object(Map::new())),
                    required: None,
                },
            },
        })
    }).collect()
}
```

### 4.3 WorkFlowInvokers 組合結構

**檔案**: `executor_agent/src/business_logic/executor_agent.rs` 第 31-65 行

```rust
#[derive(Clone)]
pub struct WorkFlowInvokers {
    pub task_invoker: Arc<dyn TaskInvoker>,
    pub agent_invoker: Arc<dyn AgentInvoker>,
    pub tool_invoker: Arc<dyn ToolInvoker>,
}

#[async_trait]
impl WorkflowServiceApi for WorkFlowInvokers {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    async fn refresh_agents(&self) -> anyhow::Result<()> {
        if let Some(a2a_invoker) = self.agent_invoker
            .as_any().downcast_ref::<A2AAgentInvoker>()
        {
            a2a_invoker.refresh_agents().await?;
        }
        Ok(())
    }
}
```

**這是 Swarm 的依賴注入樞紐** — 將三種 Invoker 組合為一個結構，實作 `WorkflowServiceApi` trait，使得 `ExecutorAgent` 可以透過 trait object 接收所有 Invokers。

---

## 5. 5-State Agent State Machine (MCP Agent)

### 5.1 狀態定義

**檔案**: `mcp_runtime/src/mcp_agent_logic/agent.rs` 第 17-23 行

```rust
#[derive(Clone, Debug)]
enum AgentState {
    Thinking,                          // 向 LLM 發送請求
    Executing(Choice),                 // 執行 LLM 請求的 tool calls
    Evaluating(Choice, Vec<Message>),  // 評估工具執行結果
    Correcting(String),                // 修正失敗的工具執行
    Finished,                          // 完成
}
```

### 5.2 完整狀態轉換圖

```
                    ┌──────────────────────────────────────────┐
                    │          MCP Agent State Machine          │
                    │                                          │
                    │  ┌──────────┐                            │
                    │  │ Thinking │◀─────────────────────┐     │
                    │  └────┬─────┘                      │     │
                    │       │                            │     │
                    │  ┌────┴──────────────────┐         │     │
                    │  │ finish_reason 判斷     │         │     │
                    │  │                       │         │     │
                    │  │ "tool_calls" ──▶ Executing      │     │
                    │  │                   │             │     │
                    │  │                   ▼             │     │
                    │  │              Evaluating         │     │
                    │  │              ┌───┴───┐          │     │
                    │  │     含 "unsatisfactory"  不含   │     │
                    │  │              │         │        │     │
                    │  │              ▼         └────────┘     │
                    │  │         Correcting                    │
                    │  │              │                        │
                    │  │              └────────▶ Thinking      │
                    │  │                                      │
                    │  │ "stop" ──────────────▶ Finished      │
                    │  │ 其他 ──────────────▶ Finished        │
                    │  └──────────────────────┘               │
                    └──────────────────────────────────────────┘
```

### 5.3 各狀態步驟深度分析

#### Thinking 步驟 (第 113-149 行)

```rust
async fn thinking_step(&mut self) -> anyhow::Result<AgentState> {
    let request_payload = ChatCompletionRequest {
        model: self.llm_interaction.model_id.clone(),
        messages: self.messages.clone(),
        temperature: Some(0.0),        // 確定性輸出
        max_tokens: Some(1024),        // 適中長度
        top_p: Some(1.0),
        stream: Some(false),           // 非串流
        tools: Some(self.llm_all_tool.clone()),  // 所有可用工具
        tool_choice: Some(ToolChoice::String(
            self.agent_mcp_config.agent_mcp_tool_choice_auto.clone(), // "auto"
        )),
    };

    let response = self.call_api_v2(&request_payload).await?;

    let choice = response.choices[0].clone();

    if choice.finish_reason == self.agent_mcp_config.agent_mcp_finish_reason_tool_calls {
        Ok(AgentState::Executing(choice))   // LLM 請求工具呼叫
    } else {
        self.messages.push(Message { /* assistant response */ });
        Ok(AgentState::Finished)            // LLM 生成了最終回應
    }
}
```

**注意**: `finish_reason` 比較使用可配置的字串（`agent_mcp_config.agent_mcp_finish_reason_tool_calls`），而非硬編碼。這使得不同 LLM API 的 finish_reason 格式差異可以透過配置處理。

#### Executing 步驟 (第 151-187 行)

```rust
async fn executing_step(&mut self, choice: &Choice) -> anyhow::Result<AgentState> {
    if let Some(tool_calls) = &choice.message.tool_calls {
        let mut tool_results: Vec<Message> = Vec::new();
        for tool_call in tool_calls {
            match execute_tool_call_v2(self.mcp_client.clone(), tool_call.clone()).await {
                Ok(result) => {
                    let result_content_str = serde_json::to_string(&result.content)?;
                    tool_results.push(Message {
                        role: self.agent_mcp_config.agent_mcp_role_tool.clone(),
                        content: Some(format!("Response from Tool: {}", result_content_str)),
                        tool_call_id: Some(tool_call.id.clone()),
                        tool_calls: None,
                    });
                }
                Err(e) => {
                    // 錯誤也作為工具結果返回（不中斷迴圈）
                    let error_content = json!({
                        "error": format!("Error executing tool '{}': {}", tool_call.id, e),
                        "tool_call_id": tool_call.id
                    });
                    tool_results.push(Message {
                        role: self.agent_mcp_config.agent_mcp_role_tool.clone(),
                        content: Some(error_content.to_string()),
                        tool_call_id: Some(tool_call.id.clone()),
                        tool_calls: None,
                    });
                }
            }
        }
        Ok(AgentState::Evaluating(choice.clone(), tool_results))
    } else {
        Ok(AgentState::Thinking)  // 無 tool_calls → 回到思考
    }
}
```

**錯誤恢復策略**: 工具執行失敗不會中斷狀態機，而是將錯誤訊息包裝為工具結果，讓 LLM 在後續步驟中自行決定如何處理。這是「讓 LLM 決定」模式的實現。

#### Evaluating 步驟 (第 189-238 行)

```rust
async fn evaluating_step(&mut self, choice: &Choice, tool_results: Vec<Message>)
    -> anyhow::Result<AgentState>
{
    // 構建評估消息序列
    let mut evaluation_messages = self.messages.clone();
    evaluation_messages.push(Message {
        role: choice.message.role.clone(),
        content: choice.message.content.clone(),
        tool_calls: choice.message.tool_calls.clone(),
        tool_call_id: None,
    });
    evaluation_messages.extend(tool_results.clone());

    // 注入評估 prompt
    evaluation_messages.push(Message {
        role: "system".to_string(),
        content: Some(self.agent_mcp_config.agent_mcp_evaluation_prompt.clone()),
        tool_call_id: None,
        tool_calls: None,
    });

    // 呼叫 LLM 進行評估（不帶工具 — 純文字回應）
    let request_payload = ChatCompletionRequest {
        // ...
        tools: None,       // 評估呼叫不使用工具
        tool_choice: None,
    };

    let response = self.call_api_v2(&request_payload).await?;

    if let Some(first_choice) = response.choices.get(0) {
        if let Some(content) = &first_choice.message.content {
            if content.contains("unsatisfactory") {
                return Ok(AgentState::Correcting(content.clone()));
            }
        }
    }

    // 滿意 → 將工具呼叫和結果加入歷史，繼續思考
    self.messages.push(/* assistant tool_calls message */);
    self.messages.extend(tool_results);
    Ok(AgentState::Thinking)
}
```

**自我評估 prompt** (`agent_factory.rs` 第 53 行):

```
"Please evaluate the previous tool results.
 If the results are satisfactory, respond with 'OK'.
 If they are unsatisfactory, provide a brief explanation of the issue."
```

**判斷邏輯**: 透過 `content.contains("unsatisfactory")` 做字串匹配。這比結構化 JSON 回應更簡單但也更脆弱 — 如果 LLM 用其他方式表達不滿意（如 "the result is incorrect"），會被視為滿意。

#### Correcting 步驟 (第 240-249 行)

```rust
async fn correcting_step(&mut self, issue: String) -> anyhow::Result<AgentState> {
    self.messages.push(Message {
        role: "system".to_string(),
        content: Some(format!(
            "{}\n The issue was: {}",
            self.agent_mcp_config.agent_mcp_correction_prompt, issue
        )),
        tool_call_id: None,
        tool_calls: None,
    });
    Ok(AgentState::Thinking)  // 回到思考，帶著修正指引
}
```

**修正 prompt** (`agent_factory.rs` 第 55 行):

```
"The previous tool execution failed.
 Please analyze the issue and try to correct it."
```

### 5.4 執行迴圈與上限控制

**檔案**: `agent.rs` 第 251-283 行

```rust
pub async fn execute_loop(&mut self) -> anyhow::Result<Option<Message>> {
    let mut final_message: Option<Message> = None;

    for loop_count in 0..self.agent_mcp_config.agent_mcp_max_loops {
        info!("Agent Loop Iteration: {}/{} - State: {:?}",
            loop_count + 1, self.agent_mcp_config.agent_mcp_max_loops, self.state);

        let next_state = match self.state.clone() {
            AgentState::Thinking    => self.thinking_step().await?,
            AgentState::Executing(choice) => self.executing_step(&choice).await?,
            AgentState::Evaluating(choice, results) =>
                self.evaluating_step(&choice, results).await?,
            AgentState::Correcting(issue) => self.correcting_step(issue).await?,
            AgentState::Finished => break,
        };
        self.state = next_state;
    }

    // 取最後的 assistant 消息作為最終回應
    if let Some(last_message) = self.messages.last() {
        if last_message.role == self.agent_mcp_config.agent_mcp_role_assistant {
            final_message = Some(last_message.clone());
        }
    }
    Ok(final_message)
}
```

**迴圈上限**: 由 `agent_mcp_max_loops` 配置控制（預設 5）。達到上限時靜默退出，不回報錯誤。

---

## 6. LLM-as-Judge 雙層自我修正

Swarm 有兩個獨立的自我修正層：

### 6.1 層 1：MCP Agent 的工具評估（即時修正）

如上節所述，McpAgent 的 Evaluating/Correcting 狀態處理單個工具呼叫的失敗。這是「細粒度」修正 — 針對一次工具調用。

### 6.2 層 2：Planner 的工作流評估（全域修正）

**檔案**: `planner_agent/src/business_logic/planner_agent.rs` 第 306-353 行

```rust
async fn handle_evaluation_and_retry(
    &self,
    request_id: &str,
    conversation_id: &str,
    original_user_query: &str,
    agent_input: String,
    agent_output: String,
    activities_outcome: HashMap<String, String>,
    retry_count: &mut u8,
) -> anyhow::Result<Option<String>> {
    if let Some(eval_service) = &self.evaluation_service {
        let data = AgentEvaluationLogData {
            agent_id: self.agent_config.agent_name.clone(),
            request_id: request_id.to_string(),
            conversation_id: conversation_id.to_string(),
            step_id: None,
            original_user_query: original_user_query.to_string(),
            agent_input,
            activities_outcome,
            agent_output: agent_output.clone(),
            context_snapshot: None,
            success_criteria: None,
        };

        let evaluation = eval_service.log_evaluation(data).await;
        match evaluation {
            Ok(eval) => {
                if eval.score < TRIGGER_RETRY && *retry_count < MAX_RETRIES {
                    // 分數 < 3 且重試次數 < 3 → 重試
                    *retry_count += 1;
                    let new_user_query = format!(
                        "{} (Previous attempt failed with feedback: {})",
                        original_user_query, eval.feedback
                    );
                    Ok(Some(new_user_query))  // 回傳修正後的 query
                } else if eval.score < TRIGGER_RETRY {
                    // 超過最大重試次數 → 徹底失敗
                    bail!("Plan execution failed after multiple retries");
                } else {
                    Ok(None)  // 分數夠高 → 成功
                }
            },
            Err(e) => {
                error!("Error during evaluation logging: {}", e);
                Ok(None)  // 評估服務本身出錯 → 視為成功（容錯）
            }
        }
    } else {
        Ok(None)  // 無評估服務 → 直接成功
    }
}
```

**常數** (`planner_agent.rs` 第 27-28 行):

```rust
const MAX_RETRIES: u8 = 3;
const TRIGGER_RETRY: u8 = 3;  // 分數低於 3 觸發重試
```

### 6.3 Judge Prompt 模板

**檔案**: `configuration/prompts/judge_agent_prompt.txt`

```
You are an expert AI evaluator. Your task is to assess the provided
'Agent Output' based on the 'Original User Query' and 'Context/Criteria'.
Original User Query: {}
Agent Input for this step: {}
Agent Output: {}
Context/Criteria: {}

Please provide a concise evaluation, focusing on:
1. Accuracy: Does the output correctly address the user's intent?
2. Completeness: Is all necessary information present?
3. Compliance: Does it meet any implicit or explicit constraints?
4. Areas for Improvement: What specifically could be done better?

Respond in a structured JSON format:
{
    "rating": "Good" | "Needs Improvement" | "Failed",
    "score": [1-10],
    "feedback": "Detailed textual feedback...",
    "suggested_correction": "If applicable, a corrected version..."
}
```

### 6.4 雙層修正對比

```
┌──────────────────────────────────────────────────────┐
│                   修正層對比                          │
├──────────────┬─────────────────┬─────────────────────┤
│              │ 層 1: MCP Agent │ 層 2: Planner Judge │
├──────────────┼─────────────────┼─────────────────────┤
│ 範圍         │ 單次工具呼叫    │ 整個工作流執行       │
│ 判斷方式     │ 字串匹配        │ 結構化 JSON score   │
│              │ "unsatisfactory"│ score < 3 觸發重試  │
│ 修正方式     │ 注入 correction │ 重新生成整個計劃    │
│              │ prompt          │ + 附加失敗反饋      │
│ 重試上限     │ max_loops       │ MAX_RETRIES = 3     │
│ 失敗處理     │ 靜默退出迴圈    │ bail! 中斷          │
│ 評估服務     │ 內建 LLM 呼叫  │ 外部 Evaluation Svc │
└──────────────┴─────────────────┴─────────────────────┘
```

---

## 7. MCP Resilient Runtime

### 7.1 McpRuntime 結構

**檔案**: `mcp_runtime/src/runtime/mcp_runtime.rs`

```rust
pub type McpClient = RunningService<RoleClient, InitializeRequestParam>;

pub struct McpRuntime {
    agent_mcp_config: McpRuntimeConfig,
    client: McpClient,
}
```

### 7.2 SSE 傳輸 + Bearer Auth

**檔案**: `mcp_runtime.rs` 第 40-70 行

```rust
pub async fn start_with_default_headers(
    uri: impl Into<Arc<str>>,
    api_key: Option<String>,
) -> Result<SseClientTransport<reqwest::Client>, SseTransportError<reqwest::Error>> {
    let mut headers = header::HeaderMap::new();
    headers.insert("X-MY-HEADER", header::HeaderValue::from_static("value"));

    let bearer = format!("Bearer {}", api_key.expect("API key required").as_str());
    let mut auth_value = header::HeaderValue::from_str(&bearer).unwrap();
    auth_value.set_sensitive(true);  // 在日誌中隱藏
    headers.insert(header::AUTHORIZATION, auth_value);

    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()?;

    SseClientTransport::start_with_client(client, SseClientConfig {
        sse_endpoint: uri.into(),
        ..Default::default()
    }).await
}
```

**安全注意**: `api_key.expect("")` 會在 API key 為 None 時 panic，不是優雅的錯誤處理。

### 7.3 工具執行與錯誤恢復

**檔案**: `mcp_runtime.rs` 第 116-155 行

```rust
pub async fn execute_tool_call_v2(&self, tool_call: ToolCall) -> anyhow::Result<CallToolResult> {
    let args: Result<serde_json::Value, _> = serde_json::from_str(&tool_call.function.arguments);

    let tool_result = match args {
        Ok(parsed_args) => {
            self.client.call_tool(CallToolRequestParam {
                name: Cow::Owned(tool_call.function.name.clone()),
                arguments: parsed_args.as_object().cloned(),
            }).await?
        }
        Err(e) => {
            tracing::error!("Failed to parse arguments for {}: {}", tool_call.function.name, e);
            // 參數解析失敗 → 回傳空結果（含 is_error 標記）
            CallToolResult {
                content: vec![],
                structured_content: None,
                is_error: Some(true),
                meta: None,
            }
        }
    };
    Ok(tool_result)
}
```

**錯誤恢復模式**: 參數解析失敗不會中斷整個代理迴圈，而是回傳一個帶 `is_error: true` 的空結果。LLM 會在後續迴圈中看到這個錯誤並嘗試修正。

### 7.4 LLM 回應處理

**檔案**: `mcp_runtime/src/mcp_agent_logic/process_response.rs`

```rust
pub fn process_response(
    loop_number: u32,
    choice: &Choice,
    messages: &mut Vec<Message>,
) -> AgentResponse {
    match choice.finish_reason.as_str() {
        "stop" => {
            // 最終文字回應
            AgentResponse { should_exit: true, nb_loop: loop_number,
                final_message: Some(Message { content: choice.message.content.clone(), ... }) }
        }
        "tool_calls" => {
            // 工具呼叫 — 加入歷史
            messages.push(Message { tool_calls: choice.message.tool_calls.clone(), ... });
            AgentResponse { should_exit: false, nb_loop: loop_number,
                final_message: Some(tool_call_message) }
        }
        _ => {
            // 未知 finish_reason — 安全退出
            eprintln!("Unhandled finish reason: {}", choice.finish_reason);
            AgentResponse { should_exit: true, ... }
        }
    }
}
```

---

## 8. Agent Factory 動態代理創建

### 8.1 Configurator Trait 模式

**檔案**: `agent_factory/src/agent_factory.rs` 第 63-143 行

```rust
trait AgentConfigurator {
    fn configure_agent_defaults(&self, builder: AgentConfigBuilder,
        factory_agent_config: &FactoryAgentConfig) -> Result<AgentConfigBuilder>;
}

struct SpecialistAgentConfigurator;
impl AgentConfigurator for SpecialistAgentConfigurator {
    fn configure_agent_defaults(&self, mut builder: AgentConfigBuilder,
        factory_agent_config: &FactoryAgentConfig) -> Result<AgentConfigBuilder>
    {
        builder = builder
            .agent_system_prompt("You are a helpful assistant.".to_string())
            .agent_discoverable(true)
            .agent_skill_id("generic_skill".to_string());

        // 按 Domain 進一步配置
        if let Some(domain) = &factory_agent_config.factory_agent_domains {
            match domain {
                AgentDomain::General  => { /* 已設定 */ },
                AgentDomain::Finance  => { builder = builder.agent_system_prompt("..."); },
                AgentDomain::Customer => { builder = builder.agent_system_prompt("..."); },
                AgentDomain::Weather  => { builder = builder.agent_system_prompt("..."); },
            }
        }
        Ok(builder)
    }
}
```

### 8.2 動態啟動流程

```rust
pub async fn launch_agent(&self,
    factory_agent_config: &FactoryAgentConfig,
    mcp_runtime_config: Option<&FactoryMcpRuntimeConfig>,
    agent_type: AgentType
) -> Result<JoinHandle<Result<()>>>
{
    let agent_config = self.create_agent_config(factory_agent_config)?;

    let handle = match agent_type {
        AgentType::Specialist => {
            let agent = BasicAgent::new(agent_config.clone(), ...).await?;
            Self::launch_agent_server(agent_config, agent, Some(discovery)).await
        },
        AgentType::Planner => {
            let agent = PlannerAgent::new(agent_config.clone(), ...).await?;
            Self::launch_agent_server(agent_config, agent, None).await
        },
        AgentType::Executor => {
            let agent = ExecutorAgent::new(agent_config.clone(), ...).await?;
            Self::launch_agent_server(agent_config, agent, None).await
        },
    };

    // 啟動後自動刷新代理列表
    if let Some(ws_arc) = &self.workflow_service {
        ws_arc.as_ref().as_any().downcast_ref::<WorkFlowInvokers>()
            .expect("...")
            .refresh_agents().await?;
    }

    Ok(handle)
}
```

---

## 9. 通訊協議 (A2A + MCP)

### 9.1 A2A 通訊流程

```
Planner ──[HTTP POST]──▶ Executor (A2A Server)
  │                           │
  │ send_task_message()       │ handle_request()
  │  task_id                  │   deserialize Graph
  │  Message { graph_json }   │   PlanExecutor::execute_plan()
  │                           │     ├── tool_invoker.invoke() → MCP Server
  │                           │     ├── agent_invoker.interact() → BasicAgent
  │                           │     └── task_invoker.invoke() → 本地函數
  │                           │
  │ ◀──[Task { status, msg }]─┘
  │
  │ TaskState::Completed → 提取結果
  │ TaskState::Failed → 報錯
```

### 9.2 MCP 通訊流程

```
McpAgent ──[SSE]──▶ MCP Server
  │                    │
  │ list_tools()       │ → 返回工具列表
  │ call_tool()        │ → 執行工具 → 返回結果
  │                    │
  │ 工具轉碼:          │
  │ rmcp::Tool →       │
  │ llm_api::Tool      │
```

---

## 10. 工作流 JSON Schema 與範例

### 10.1 Schema v2

**檔案**: `documentation/workflow_json_schema/workflow_json_schema_v2.json`

```json
{
  "plan_name": "string",
  "activities": [{
    "activity_type": "delegation_agent" | "direct_task_execution" | "direct_tool_use",
    "id": "string (唯一識別符)",
    "description": "string (人類可讀)",
    "type": "string (操作類型)",
    "agent": {
      "skill_to_use": "string | null",
      "assigned_agent_id_preference": "string | null",
      "agent_context": { /* 動態鍵值 */ }
    },
    "tools": [{
      "tool_to_use": "string | null",
      "tool_parameters": { /* 動態鍵值 */ }
    }],
    "dependencies": [{ "source": "string (上游 activity ID)" }],
    "expected_outcome": "string"
  }]
}
```

### 10.2 範例工作流

**檔案**: `workflow_management/example_workflow/multi_agent_workflow.json`

```
fetch_weather_forecast ─────┐
(DirectToolUse)              │
   get_current_weather       │
   params: {_location:"Boston"} │
                             │
                             ├──▶ compose_personalized_message ──▶ send_notification
                             │    (DelegationAgent)                (DelegationAgent)
fetch_customer_data ─────────┘    Basic_Agent                     Basic_Agent
(DelegationAgent)                 context: {{fetch_weather_forecast}}
Basic_Agent                                {{fetch_customer_data}}
```

---

## 11. 錯誤處理深度分析

### 11.1 PlanExecutorError 枚舉

**檔案**: `graph_orchestrator.rs` 第 16-40 行

```rust
#[derive(Error, Debug, PartialEq)]
pub enum PlanExecutorError {
    #[error("Missing node in graph: {0}")]
    MissingNode(String),
    #[error("Execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Invalid state transition")]
    InvalidState,
    #[error("Task invoker not found for skill: {0}")]
    TaskRunnerNotFound(String),
    #[error("Agent invoker not found: {0}")]
    AgentRunnerNotFound(String),
    #[error("Tool invoker not found: {0}")]
    ToolRunnerNotFound(String),
    #[error("Cyclic dependency detected")]
    CyclicDependency,
    #[error("Missing tool to use for DirectToolUse activity: {0}")]
    MissingTool(String),
    #[error("Missing skill to use for DirectTaskExecution activity: {0}")]
    MissingSkill(String),
    #[error("Parameter interpolation failed: {0}")]
    InterpolationFailed(String),
    #[error("Missing task to use for DirectTaskExecution activity: {0}")]
    MissingTask(String),
}
```

**優點**: 使用 `thiserror` 的 `#[derive(Error)]` 生成 `Display` 和 `Error` trait 實作，每個變體都有清晰的錯誤訊息。`PartialEq` derivation 使得測試中可以直接比較錯誤。

### 11.2 錯誤傳播路徑

```
Invoker 層:
  anyhow::Error → PlanExecutorError::ExecutionFailed(String)

Graph 層:
  節點不存在 → MissingNode
  環形依賴   → CyclicDependency
  狀態異常   → InvalidState

插值層:
  路徑解析失敗 → InterpolationFailed
  依賴結果不存在 → InterpolationFailed

MCP 層:
  LLM API 失敗    → anyhow::Error (上浮)
  工具執行失敗    → 包裝為 Message (不中斷)
  參數解析失敗    → CallToolResult { is_error: true }
```

---

## 12. 效能特徵分析

### 12.1 序列化開銷

- `clone()` 在狀態機中頻繁使用（`self.state.clone()`, `choice.clone()`）
- `self.messages.clone()` 在評估步驟中複製整個消息歷史
- `serde_json::to_string` / `from_str` 在每次插值時執行

### 12.2 並行度限制

- **DAG 執行器是單線程的**: `execution_queue` 是 `VecDeque`，一次只彈出一個節點。即使 DAG 中有可並行的分支（多個入度為 0 的節點），也是依序執行。
- **MCP Agent 是序列工具呼叫**: 即使 LLM 請求多個工具呼叫，也是 `for tool_call in tool_calls` 依序執行。

### 12.3 記憶體使用

- `activities_outcome: HashMap<String, String>` 儲存所有中間結果的完整字串
- `messages: Vec<Message>` 在 McpAgent 中累積所有歷史消息
- 無清理或壓縮機制 — 長時間運行可能 OOM

---

## 13. Clawtex 差距對比與 Rust 實作建議

### 13.1 DAG 工作流 vs 線性 Hands

**Swarm**: `PlanExecutor` — DAG 拓撲排序 + `{{}}` 插值 + 條件邊
**Clawtex**: `HandRunner` — 線性 phase 陣列，無分支/並行

```
Swarm DAG:
fetch_weather ──┐
                ├──▶ compose_message ──▶ send_email
fetch_customer ─┘

Clawtex Hands:
Phase 1 → Phase 2 → Phase 3 → Phase 4
```

**Clawtex 實作建議**:

在 `hand.toml` 中加入 `depends_on` 欄位：

```toml
[[phases]]
name = "fetch_weather"
depends_on = []

[[phases]]
name = "fetch_customer"
depends_on = []

[[phases]]
name = "compose"
depends_on = ["fetch_weather", "fetch_customer"]
prompt_template = "用 {{fetch_weather}} 和 {{fetch_customer}} 的結果組合訊息"
```

```rust
// hands/dag_runner.rs
use std::collections::{HashMap, VecDeque};

pub struct DagPhaseRunner {
    dependency_tracker: HashMap<String, usize>,
    execution_queue: VecDeque<String>,
    phase_outcomes: HashMap<String, String>,
}

impl DagPhaseRunner {
    pub fn initialize(&mut self, phases: &[Phase]) {
        for phase in phases {
            let dep_count = phase.depends_on.len();
            self.dependency_tracker.insert(phase.name.clone(), dep_count);
            if dep_count == 0 {
                self.execution_queue.push_back(phase.name.clone());
            }
        }
    }

    pub async fn execute_next(&mut self) -> Option<String> {
        let phase_name = self.execution_queue.pop_front()?;
        // 執行 phase...
        // 更新下游依賴
        Some(phase_name)
    }

    fn interpolate(&self, template: &str) -> String {
        let re = regex::Regex::new(r"\{\{(\w+)\}\}").unwrap();
        re.replace_all(template, |caps: &regex::Captures| {
            let key = &caps[1];
            self.phase_outcomes.get(key).cloned().unwrap_or_default()
        }).to_string()
    }
}
```

> **Clawtex 實作建議**: 在 `src/hands/runner.rs` 中引入 `DagPhaseRunner`，保持向後相容 — 當 `depends_on` 未指定時，自動推導為線性依賴。插值引擎應使用 `OnceLock<Regex>` 避免重複編譯。

---

### 13.2 Three-tier Invoker vs 扁平 Tool 系統

**Swarm**: `AgentInvoker` / `ToolInvoker` / `TaskInvoker` — 三個獨立 trait
**Clawtex**: `delegate` 工具 + `delegate_to_provider` 工具 — 作為 tool 的特例

```rust
// clawtex 建議的統一 Invoker trait
#[async_trait]
pub trait ResourceInvoker: Send + Sync {
    async fn invoke(&self, resource_id: &str, params: &serde_json::Value)
        -> Result<serde_json::Value>;
    fn resource_type(&self) -> ResourceType;
}

pub enum ResourceType {
    Agent,      // delegate → 另一個 agent
    Tool,       // 內建 24 tools
    Provider,   // delegate_to_provider → 其他 LLM
    Hand,       // chain_to → 另一個 workflow
    McpTool,    // MCP Client → 外部 MCP Server
}

// 為 HandRunner 的 DAG 引擎提供統一的調用介面
pub struct ClawtexInvokerBundle {
    agent_invoker: Arc<dyn ResourceInvoker>,
    tool_invoker: Arc<dyn ResourceInvoker>,
    provider_invoker: Arc<dyn ResourceInvoker>,
}
```

> **Clawtex 實作建議**: 統一 `delegate` 和 `delegate_to_provider` 為 `ResourceInvoker` trait，使 Hands workflow 引擎可以用同一介面調用不同類型的資源。保持現有的 tool trait 用於 agent_runtime，但在 Hands 層引入 ResourceInvoker 抽象。

---

### 13.3 Agent State Machine vs 線性 Loop

**Swarm**: 5-state enum (Thinking → Executing → Evaluating → Correcting → Finished)
**Clawtex**: `agent_runtime.rs` — loop + if/else

```rust
// clawtex 建議的 AgentState enum
pub enum AgentState {
    Thinking,
    ToolExecution { pending_calls: Vec<ToolCall> },
    Evaluating { results: Vec<ToolResult> },
    SelfCorrecting { issue: String, retry_count: u8 },
    Responding,
    WaitingApproval { tool_name: String, args: serde_json::Value },
    Completed,
}

impl AgentState {
    pub fn next_state(&self, event: AgentEvent) -> AgentState {
        match (self, event) {
            (Thinking, AgentEvent::ToolCallRequested(calls)) =>
                ToolExecution { pending_calls: calls },
            (Thinking, AgentEvent::TextResponse) =>
                Completed,
            (ToolExecution { .. }, AgentEvent::ToolsCompleted(results)) =>
                Evaluating { results },
            (Evaluating { .. }, AgentEvent::Satisfactory) =>
                Thinking,
            (Evaluating { .. }, AgentEvent::Unsatisfactory(issue)) =>
                SelfCorrecting { issue, retry_count: 0 },
            (SelfCorrecting { .. }, AgentEvent::CorrectionReady) =>
                Thinking,
            (_, AgentEvent::ApprovalRequired(tool, args)) =>
                WaitingApproval { tool_name: tool, args },
            (WaitingApproval { .. }, AgentEvent::Approved) =>
                ToolExecution { .. },
            _ => Completed,
        }
    }
}
```

> **Clawtex 實作建議**: 在 `agent_runtime.rs` 中引入 `AgentState` enum 和 `AgentEvent` enum，將現有的 loop + if/else 結構重構為明確的狀態機。這特別重要是因為 clawtex 有 `WaitingApproval` 狀態（Telegram 人機交互），Swarm 沒有的。

---

### 13.4 Discovery Service vs ClusterHub

**Swarm**: 外部 Discovery Service HTTP API → 動態代理發現
**Clawtex**: ClusterHub 內建 worker 註冊

```rust
// clawtex 建議擴展 ClusterHub 為 ResourceDiscovery
pub struct ResourceDiscovery {
    agents: HashMap<String, AgentCapability>,
    tools: HashMap<String, ToolCapability>,
    providers: HashMap<String, ProviderCapability>,
    workers: HashMap<String, WorkerInfo>,  // 現有的 ClusterHub 功能
}

impl ResourceDiscovery {
    pub async fn list_available_resources(&self) -> String {
        // 產生可用資源列表，注入 LLM prompt 用於動態計劃生成
        let mut result = String::new();
        for (name, cap) in &self.agents {
            result.push_str(&format!("Agent: {}, Skills: {}\n", name, cap.skills.join(",")));
        }
        for (name, cap) in &self.tools {
            result.push_str(&format!("Tool: {}, Desc: {}\n", name, cap.description));
        }
        result
    }
}
```

> **Clawtex 實作建議**: 擴展 `cluster_hub.rs` 增加 `list_available_resources()` 方法，讓 Hands workflow 可以動態發現可用的 agents、tools、providers。這使得 LLM 動態生成工作流成為可能（類似 Swarm 的 Planner 模式）。

---

### 13.5 LLM-as-Judge vs 現有的自我修正

**Swarm**: 結構化 JSON 評估 (score + feedback + suggested_correction)
**Clawtex**: `loop_detection.rs` + `self_correction`（簡單循環偵測）

```rust
// clawtex 建議的結構化評估
#[derive(Deserialize)]
pub struct HandPhaseEvaluation {
    pub rating: String,       // "Good" | "Needs Improvement" | "Failed"
    pub score: u8,            // 1-10
    pub feedback: String,
    pub suggested_correction: Option<String>,
}

pub async fn evaluate_phase_output(
    provider: &dyn Provider,
    user_query: &str,
    phase_output: &str,
    evaluation_prompt: &str,
) -> Result<HandPhaseEvaluation> {
    let prompt = evaluation_prompt
        .replacen("{}", user_query, 1)
        .replacen("{}", phase_output, 1);

    let response = provider.complete(&prompt).await?;
    let evaluation: HandPhaseEvaluation = serde_json::from_str(&response)?;
    Ok(evaluation)
}
```

> **Clawtex 實作建議**: 在 `src/hands/runner.rs` 中，每個 phase 完成後可選擇性地進行 LLM-as-Judge 評估。在 `hand.toml` 中增加 `[evaluation]` 段落配置評估條件。結合現有的 `costs.db` 記錄評估分數，用於 `self_evolve` 手的數據分析。

---

### 13.6 總體差距矩陣

| 特性 | Swarm | Clawtex | 差距 | 優先級 |
|------|-------|---------|------|--------|
| DAG 工作流 | 拓撲排序 + 條件邊 | 線性 phase | **大** | 高 |
| 參數插值 | `{{id.output}}` | 無 | **大** | 高 |
| Invoker 抽象 | 3-tier trait | 扁平 tool | 中 | 中 |
| Agent 狀態機 | 5-state enum | loop+if | 中 | 中 |
| LLM-as-Judge | 結構化 JSON | 循環偵測 | **大** | 高 |
| Discovery Service | 外部 HTTP | ClusterHub 基本 | 中 | 低 |
| MCP Runtime | SSE + 評估 | stdio MCP | 小 | 低 |
| Agent Factory | 動態創建 | 靜態配置 | 中 | 低 |
| 並行執行 | 單線程 DAG | 無 | 都需改進 | 中 |
| 錯誤恢復 | 錯誤注入 + 重試 | E-Stop | 不同策略 | — |

---

## 附錄：關鍵程式碼路徑索引

| 功能 | 檔案 | 行範圍 |
|------|------|--------|
| DAG 拓撲排序初始化 | `graph_orchestrator.rs` | 96-120 |
| DAG 執行分派 | `graph_orchestrator.rs` | 137-243 |
| 參數插值引擎 | `graph_orchestrator.rs` | 308-377 |
| 下游依賴更新 | `graph_orchestrator.rs` | 245-272 |
| 葉節點結果聚合 | `graph_orchestrator.rs` | 274-301 |
| 條件邊評估 | `condition_evaluator.rs` | 4-30 |
| A* 尋路 | `a_star.rs` | 27-66 |
| McpAgent 狀態機 | `agent.rs` | 17-23 (enum), 251-283 (loop) |
| Thinking 步驟 | `agent.rs` | 113-149 |
| Executing 步驟 | `agent.rs` | 151-187 |
| Evaluating 步驟 | `agent.rs` | 189-238 |
| Correcting 步驟 | `agent.rs` | 240-249 |
| LLM 回應處理 | `process_response.rs` | 13-100 |
| A2AAgentInvoker | `resource_invoker/lib.rs` | 31-208 |
| McpRuntimeToolInvoker | `resource_invoker/lib.rs` | 231-308 |
| GreetTask | `resource_invoker/lib.rs` | 210-229 |
| WorkFlowInvokers 組合 | `executor_agent.rs` | 31-65 |
| ExecutorAgent 接收 Graph | `executor_agent.rs` | 101-141 |
| PlannerAgent 策略決定 | `planner_agent.rs` | 423-433 |
| 動態計劃生成 | `planner_agent.rs` | 355-377 |
| Judge 評估重試 | `planner_agent.rs` | 306-353 |
| AgentFactory 啟動 | `agent_factory.rs` | 262-337 |
| Configurator trait | `agent_factory.rs` | 63-143 |
| MCP SSE 初始化 | `mcp_runtime.rs` | 40-103 |
| 工具轉碼 | `resource_invoker/lib.rs` | 255-290 |
| Judge prompt | `prompts/judge_agent_prompt.txt` | 全檔 |
| 工作流 JSON Schema | `workflow_json_schema_v2.json` | 全檔 |
