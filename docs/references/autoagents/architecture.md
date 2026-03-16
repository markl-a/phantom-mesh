# AutoAgents 架構概覽

## 1. 專案概覽

**AutoAgents** 是一個生產級 Rust 多代理框架，設計用於建構高效能、類型安全的 AI 智能系統。其核心目標是：

- **模組化設計**：各層組件獨立、可替換
- **類型安全**：編譯時驗證、零運行時開銷
- **跨平台支援**：同時支援 WASM 和本機（非 WASM）運行環境
- **LLM 無關性**：支援 15+ 雲端和本機 LLM 提供商

核心優勢包括：
- ReAct + 基礎執行器，可流式回應
- 工具衍生宏 `#[tool]` 與 WASM 沙箱執行
- 滑動視窗記憶體與可擴展後端
- 類型化 Pub/Sub 多代理協調
- OpenTelemetry 可觀測性
- LLM 守衛欄（Guardrails）與優化管道

---

## 2. 目錄結構

### 工作區頂級結構

```
autoagents/
├── crates/                        # 核心 Rust crates
│   ├── autoagents-core/          # 代理引擎 (核心)
│   ├── autoagents-llm/           # LLM 提供商抽象層
│   ├── autoagents-derive/        # 過程宏 (#[agent], #[tool])
│   ├── autoagents-toolkit/       # 可共享工具與 MCP 輔助
│   ├── autoagents-qdrant/        # Qdrant 向量儲存實現
│   ├── autoagents-guardrails/    # LLM 守衛欄引擎
│   └── [inference crates]        # mistral-rs, llamacpp, burn, onnx (可選)
│
├── bindings/                      # Python 綁定
│   └── python/
│       ├── autoagents/           # 主 Python 包
│       ├── autoagents-llamacpp/  # Python llamacpp 綁定
│       ├── autoagents-mistralrs/ # Python mistral-rs 綁定
│       └── [其他推論運行時]
│
├── examples/                      # 端到端範例
│   ├── basic/                    # 基礎代理範例
│   ├── coding_agent/             # 編程代理
│   ├── guardrails/               # 守衛欄使用
│   ├── mcp/                      # MCP 伺服器集成
│   └── [其他實驗]
│
├── docs/                          # mdBook 文檔
│   └── src/
│       ├── core-concepts/        # 代理、記憶、工具、執行器
│       ├── getting-started/      # 快速開始指南
│       ├── llm-providers/        # LLM 後端文檔
│       └── developer/            # 開發者指南
│
└── Cargo.toml                     # 工作區根配置
```

### autoagents-core 內部結構（核心引擎）

```
src/
├── lib.rs                         # 公開介面定義
├── agent/                         # 代理核心模組
│   ├── mod.rs                    # 主要 trait/struct 導出
│   ├── base.rs                   # BaseAgent<T, A> 泛型容器
│   ├── builder.rs                # AgentBuilder 建造者模式
│   ├── executor/                 # 執行策略
│   │   ├── mod.rs               # AgentExecutor trait
│   │   ├── turn_engine.rs        # 單回合引擎
│   │   ├── tool_processor.rs     # 工具呼叫處理
│   │   ├── memory_helper.rs      # 記憶管理
│   │   └── event_helper.rs       # 事件分發
│   ├── prebuilt/
│   │   ├── executor/
│   │   │   ├── basic.rs         # 單回合執行器
│   │   │   └── react.rs         # ReAct 多回合執行器
│   ├── memory/                   # 記憶提供商
│   │   ├── mod.rs               # MemoryProvider trait
│   │   └── sliding_window.rs     # 滑動視窗實現
│   ├── hooks.rs                  # 代理生命週期鉤子
│   ├── context.rs                # 執行上下文
│   ├── task.rs                   # 任務定義
│   └── config.rs                 # 代理設定
│
├── actor/                         # 多代理協調 (cfg!不是 wasm32)
│   ├── mod.rs                    # AnyActor trait
│   ├── messaging.rs              # 訊息包裝 (Cloneable/Shared)
│   ├── subscriber.rs             # 訂閱者抽象
│   ├── topic.rs                  # 類型化 Pub/Sub 主題
│   └── transport.rs              # 傳輸層
│
├── runtime/                       # 運行時管理 (cfg!不是 wasm32)
│   ├── mod.rs                    # Runtime trait, RuntimeConfig
│   ├── manager.rs                # 運行時生命週期
│   └── single_threaded.rs        # 單執行緒實現
│
├── tool/                          # 工具系統
│   ├── mod.rs                    # ToolT, ToolRuntime trait
│   └── runtime/
│       ├── mod.rs                # ToolRuntime trait
│       └── wasm.rs               # WASM 沙箱執行器 (wasmtime)
│
├── vector_store/                  # 向量儲存抽象
│   ├── mod.rs                    # VectorStoreIndex trait
│   ├── in_memory_store.rs        # 記憶體實現
│   └── request.rs                # 查詢請求結構
│
├── embeddings/                    # 嵌入輔助
│   ├── mod.rs                    # 距離計算、嵌入提供商
│   └── distance.rs               # 向量距離計算
│
├── environment.rs                 # 環境容器 (cfg!不是 wasm32)
├── channel.rs                     # 訊息通道抽象
├── document.rs                    # 文件結構
├── event_fanout.rs                # 事件分發 (cfg!不是 wasm32)
├── error.rs                       # 錯誤型別
├── utils.rs                       # 輔助函數
└── readers/                       # 文件讀取器
    └── simple_directory_reader.rs # 目錄掃描工具
```

---

## 3. 核心 Trait / Struct 定義

### 3.1 代理核心三角形

```rust
// 定義層：代理實現的行為
pub trait AgentDeriveT: Send + Sync {
    type Output: AgentOutputT;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn tools(&self) -> Vec<Box<dyn ToolT>>;
    fn output_schema(&self) -> Option<Value>;  // 可選結構化輸出
}

// 執行層：驅動代理的策略
pub trait AgentExecutor: Send + Sync {
    type Output: Serialize + DeserializeOwned;
    type Error: Error;

    fn config(&self) -> ExecutorConfig;
    async fn execute(&self, task: &Task, context: Arc<Context>)
        -> Result<Self::Output, Self::Error>;
    async fn execute_stream(...) -> Result<BoxStream<...>>;
}

// 生命週期層：代理鉤子
pub trait AgentHooks: Send + Sync {
    async fn on_before_turn(...) -> Result<HookOutcome>;
    async fn on_after_turn(...) -> Result<HookOutcome>;
    async fn on_tool_call(...) -> Result<HookOutcome>;
    async fn on_error(...) -> Result<HookOutcome>;
}

// 容器層：整合所有組件
pub struct BaseAgent<T: AgentDeriveT + AgentExecutor + AgentHooks, A: AgentType> {
    pub inner: Arc<T>,                          // 你的代理實現
    pub llm: Arc<dyn LLMProvider>,              // LLM 後端
    pub memory: Option<Arc<Mutex<dyn MemoryProvider>>>,
    pub id: ActorID,                            // 唯一 ID
    pub serialized_tools: Option<Arc<Vec<Tool>>>,
    pub tx: Option<Sender<Event>>,              // 事件發射器
}
```

### 3.2 工具系統

```rust
// 工具 trait：所有工具實現此接口
pub trait ToolT: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn args_schema(&self) -> Value;             // JSON Schema
}

pub trait ToolRuntime {
    async fn execute(&self, args: Value) -> Result<Value, ToolCallError>;
}

// 衍生輔助
pub trait ToolInputT {
    fn io_schema() -> &'static str;
}
```

### 3.3 記憶系統

```rust
pub trait MemoryProvider: Send + Sync {
    async fn remember(&mut self, msg: &ChatMessage) -> Result<(), LLMError>;
    async fn recall(&self, query: &str) -> Result<Vec<ChatMessage>, LLMError>;
    async fn forget(&mut self, key: &str) -> Result<(), LLMError>;
}

// 預建實現：滑動視窗
pub struct SlidingWindowMemory {
    messages: VecDeque<ChatMessage>,
    max_tokens: usize,
}
```

### 3.4 LLM 提供商抽象

```rust
pub trait LLMProvider: Send + Sync {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse>;
    async fn completion(&self, request: CompletionRequest) -> Result<CompletionResponse>;
    async fn embedding(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse>;
}

// 支援的提供商：OpenAI, Anthropic, Ollama, Groq, Google, DeepSeek, xAI...
```

### 3.5 多代理協調

```rust
// Pub/Sub 主題（類型化）
pub struct Topic<T> {
    name: String,
    phantom: PhantomData<T>,
}

// 訊息類型標記
pub trait ActorMessage: Send + Sync + Debug {}
pub trait CloneableMessage: ActorMessage + Clone {}
pub struct SharedMessage<M>(pub Arc<M>);

// 運行時抽象
pub trait Runtime: Send + Sync {
    async fn subscribe_any(...);
    async fn publish_any(...);
    fn tx(&self) -> mpsc::Sender<Event>;
    async fn run(&self) -> Result<()>;
}
```

---

## 4. 啟動流程

### 4.1 代理初始化序列（整體流程）

```
1. 定義階段 (Development)
   └─ #[agent] 宏 → 生成 AgentDeriveT 實現
   └─ #[tool] 宏 → 生成 ToolT 實現

2. 建造階段 (Startup)
   └─ AgentBuilder::new(my_agent)
      ├─ .llm(Arc::new(provider))          // 指定 LLM
      ├─ .memory(Box::new(sliding_window)) // 指定記憶
      ├─ .runtime(Arc::new(runtime))       // (可選) 運行時
      └─ .build()
         └─ → BaseAgent<T, A>

3. 執行階段 (Runtime)
   ├─ Task 建立 → "Use tool X to solve problem Y"
   ├─ Context 初始化 → ActorID, 記憶狀態, 配置
   ├─ 執行器啟動
   │  └─ executor.execute(&task, context)
   │     ├─ on_before_turn() 鉤子
   │     ├─ for turn in 1..max_turns {
   │     │   ├─ LLM 推論 (chat request)
   │     │   ├─ 工具呼叫解析
   │     │   ├─ 工具執行 (WASM 或本機)
   │     │   ├─ 記憶更新
   │     │   ├─ on_after_turn() 鉤子
   │     │   └─ 檢查完成條件
   │     └─ }
   │     └─ 最終輸出
   │
   └─ 事件發射 → tx.send(Event::...);
```

### 4.2 詳細執行迴圈

```
ExecutorConfig { max_turns: 10 }
    ↓
Executor::execute(task, context)
    ├─[Turn 1]
    │  ├─ 記憶回想: recall(task.query) → 上下文訊息
    │  ├─ LLM 呼叫:
    │  │   req = ChatRequest {
    │  │       messages: [system_prompt, context_msgs, task],
    │  │       tools: [tool1.json, tool2.json, ...],
    │  │   }
    │  │   resp = llm.chat(req)  // [OpenAI, Anthropic, Ollama, ...]
    │  ├─ 回應解析:
    │  │   if resp.contains_tool_call() {
    │  │       for tool_call in resp.tool_calls {
    │  │           ├─ 工具查找: tool = tools[tool_call.name]
    │  │           ├─ 工具執行:
    │  │           │   if tool.is_wasm() {
    │  │           │       result = wasm_runtime.execute(wasm_module, args)
    │  │           │   } else {
    │  │           │       result = native_tool.execute(args)
    │  │           │   }
    │  │           └─ ToolCallResult 記錄
    │  │       }
    │  │   } else {
    │  │       → TurnResult::Complete(output)
    │  │   }
    │  ├─ 記憶儲存: remember(ChatMessage::assistant(resp))
    │  └─ HookOutcome 檢查: on_after_turn() → Continue/Stop
    │
    ├─[Turn 2-10]
    │  └─ 重複上述流程，最多 10 回合
    │
    └─ 輸出: Output
```

---

## 5. 資料流 ASCII 圖

### 5.1 代理與 LLM 的資料流

```
┌─────────────────────────────────────────────────────┐
│                   User Query                         │
│             "Use web search to find..."             │
└────────────────────┬────────────────────────────────┘
                     │
                     ↓
         ┌──────────────────────────┐
         │   AgentBuilder.build()   │
         │                          │
         │  → BaseAgent<T, A> {     │
         │      inner: T (你的代理)  │
         │      llm: LLMProvider    │
         │      memory: MemProvider │
         │      id: ActorID         │
         │  }                       │
         └──────────────┬───────────┘
                        │
                        ↓
      ┌────────────────────────────────────┐
      │   Executor::execute()              │
      │                                    │
      │  ┌──────────────────────────────┐ │
      │  │  MemoryHelper::recall()      │ │
      │  │  → Vec<ChatMessage>          │ │
      │  └──────────┬───────────────────┘ │
      │             │                     │
      │             ↓                     │
      │  ┌──────────────────────────────┐ │
      │  │  Build ChatRequest           │ │
      │  │  {                           │ │
      │  │    messages: [...]           │ │
      │  │    tools: tool_definitions   │ │
      │  │    max_tokens: 2048          │ │
      │  │  }                           │ │
      │  └──────────┬───────────────────┘ │
      │             │                     │
      │             ↓                     │
      │  ┌──────────────────────────────┐ │
      │  │  LLMProvider::chat()         │ │
      │  │  (OpenAI, Anthropic, etc.)  │ │
      │  └──────────┬───────────────────┘ │
      │             │                     │
      │             ↓                     │
      │  ┌──────────────────────────────┐ │
      │  │  Parse ChatResponse          │ │
      │  │  [ToolCall, ToolCall, ...]  │ │
      │  └──────────┬───────────────────┘ │
      │             │                     │
      │      ┌──────┴──────┐              │
      │      │             │              │
      │      ↓             ↓              │
      │  (有工具呼叫)  (沒有工具呼叫)   │
      │      │             │              │
      │      ↓             ↓              │
      │  ┌──────────┐  TurnResult::     │
      │  │ToolProc. │  Complete(out)   │
      │  │           │                  │
      │  │ ├─查找工具 │                  │
      │  │ ├─執行     │                  │
      │  │ │(WASM/   │                  │
      │  │ │ native) │                  │
      │  │ └─收集結果 │                  │
      │  └──────┬────┘                   │
      │         │                        │
      │         ↓                        │
      │  MemoryHelper::remember()        │
      │  → 儲存工具結果                  │
      │         │                        │
      │         ↓                        │
      │  Loop→ Turn 2 (或完成)          │
      └────────────┬────────────────────┘
                   │
                   ↓
         ┌──────────────────────┐
         │   Final Output       │
         │   (Serialized JSON)  │
         └──────────┬───────────┘
                    │
                    ↓
         ┌──────────────────────────────┐
         │  Event::AgentCompleted {     │
         │      agent_id: ActorID,      │
         │      output: Value,          │
         │      duration: Duration,     │
         │  }                           │
         │                              │
         │  → Runtime::tx() ←           │
         │  → 多代理協調                 │
         └──────────────────────────────┘
```

### 5.2 多代理 Pub/Sub 流程

```
┌──────────────┐                    ┌──────────────┐
│   Agent A    │                    │   Agent B    │
│              │                    │              │
│ publishes    │                    │  subscribes  │
│ Task::Query  │                    │  to Topic<T> │
└──────┬───────┘                    └──────▲───────┘
       │                                   │
       │    Topic<Task>                    │
       │    ("analysis_results")          │
       │                                   │
       │ ┌────────────────────┐           │
       └─→│  Runtime::Pub     │           │
         │  Sub               │           │
         │                    │───────────┘
         │                    │
         │ Delivery Queue:    │
         │ [Task, Task, Task] │
         │                    │
         └────────────────────┘

多代理協調流程：
┌─────────┐
│ Actor A │  Agent A (ReAct Executor)
└────┬────┘     ├─ name: "searcher"
     │          ├─ tools: [web_search, ...]
     │          └─ output: SearchResults
     │
     │ Topic<Task>("web_search_request")
     │   Message: { query: "AI news 2026" }
     │
     ├→ Runtime::publish_any()
     │     ├─ 查找訂閱者
     │     ├─ 投遞到 Actor B 隊列
     │     └─ 事件記錄
     │
     └→ Topic<SearchResults>
        (Agent A 發佈結果)
           ↓
        Actor B 接收
        (Agent B, Executor)
```

---

## 6. 子系統清單

### P0 (核心關鍵)

| 子系統 | 檔案 | 責任 | 依賴 |
|--------|------|------|------|
| **AgentDeriveT** | `agent/base.rs` | 定義代理行為、工具、輸出 | 無 |
| **AgentExecutor** | `agent/executor/mod.rs` | 執行迴圈策略（ReAct/Basic） | AgentDeriveT, MemoryProvider |
| **LLMProvider** | `llm/mod.rs` | 統一 LLM 後端 | 無 |
| **ToolT** | `tool/mod.rs` | 工具定義與執行 | ToolRuntime |
| **BaseAgent** | `agent/base.rs` | 整合容器（AgentDeriveT + Executor + LLM + 記憶） | 上述所有 |
| **AgentBuilder** | `agent/builder.rs` | 流暢 API 建構 BaseAgent | BaseAgent 组件 |

### P1 (重要)

| 子系統 | 檔案 | 責任 | 依賴 |
|--------|------|------|------|
| **MemoryProvider** | `agent/memory/mod.rs` | 上下文儲存、回想、遺忘 | ChatMessage |
| **SlidingWindowMemory** | `agent/memory/sliding_window.rs` | Token 限制視窗實現 | MemoryProvider |
| **TurnEngine** | `agent/executor/turn_engine.rs` | 回合迴圈狀態機 | AgentExecutor |
| **ToolProcessor** | `agent/executor/tool_processor.rs` | 工具呼叫解析、執行、結果蒐集 | ToolT, ToolRuntime |
| **Runtime (Actor System)** | `runtime/mod.rs` | 多代理編排、Pub/Sub | ractor (cfg!不是 wasm32) |
| **Topic<T>** | `actor/topic.rs` | 類型化訊息主題 | ActorMessage |
| **AgentHooks** | `agent/hooks.rs` | 生命週期鉤子（before/after/error） | AgentDeriveT |

### P2 (支援)

| 子系統 | 檔案 | 責任 | 依賴 |
|--------|------|------|------|
| **WasmRuntime** | `tool/runtime/wasm.rs` | WASM 工具沙箱 (wasmtime) | ToolRuntime |
| **VectorStoreIndex** | `vector_store/mod.rs` | 向量儲存抽象 (Qdrant等) | embeddings |
| **EmbeddingProvider** | `embeddings/mod.rs` | 嵌入模型抽象 | LLMProvider |
| **EventFanout** | `event_fanout.rs` | 事件分發/廣播 | (cfg!不是 wasm32) |
| **Environment** | `environment.rs` | 多代理容器與生命週期 | Runtime |
| **DirectAgent** | `agent/direct.rs` | 簡單單代理模式 | AgentDeriveT |
| **ActorAgent** | `agent/actor.rs` | Actor 模式協調 | Runtime, Topic |
| **Derive Macros** | `autoagents-derive/` | #[agent], #[tool], #[output] | 代碼生成 |

### 優化與守衛欄 (P3)

| 子系統 | 檔案 | 責任 |
|--------|------|------|
| **GuardrailsEngine** | `autoagents-guardrails/` | 提示注入防護、PII 淘汰、毒性檢測 |
| **LLMPipeline** | `autoagents-llm/pipeline.rs` | 快取、重試、限流優化 |
| **ClassifierProvider** | `autoagents-llm/` | 請求路由與提供商選擇 |

---

## 7. 關鍵架構決策

### 7.1 泛型設計：BaseAgent<T, A>

```rust
pub struct BaseAgent<T: AgentDeriveT + AgentExecutor + AgentHooks, A: AgentType> { }
```

- **T**：你的代理實現（通常由 `#[agent]` 宏生成）
- **A**：執行模式標記（Direct vs Actor）
- 優點：編譯時類型檢查、零運行時反射開銷

### 7.2 WASM 條件編譯

```rust
#[cfg(not(target_arch = "wasm32"))]
pub mod actor;  // 多代理協調

#[cfg(target_arch = "wasm32")]
pub use futures::lock::Mutex;  // 異步鎖用 futures
```

允許同一代碼庫支援瀏覽器和伺服器環境。

### 7.3 分層相依性

```
┌─────────────────────────┐
│  autoagents (Facade)    │
├─────────────────────────┤
│ autoagents-core         │ ← agent execution logic
│ autoagents-llm          │ ← LLM providers only
│ autoagents-derive       │ ← proc macros
├─────────────────────────┤
│ autoagents-toolkit      │ ← shared tools
│ autoagents-qdrant       │ ← vector stores
│ [inference crates]      │ ← local LLMs (optional)
└─────────────────────────┘
```

核心（-core）保持輕量，只依賴 LLM 型別，不依賴具體提供商。

### 7.4 工具執行路徑

```
ToolCall { name: "web_search", args: JSON }
    ├─ 查找工具定義 → Tool { name, schema, ... }
    ├─ 檢查執行環境
    │   ├─ WASM 模組 → WasmRuntime::execute()
    │   └─ 本機工具 → NativeTool::execute()
    └─ 結果序列化 → ToolCallResult { ... }
```

支援沙箱和本機工具，自動選擇。

---

## 8. 效能特性

- **零拷貝訊息**：Arc 共享減少分配
- **非同步優先**：tokio async/await
- **流式回應**：支援 `execute_stream()` 漸進式輸出
- **記憶優化**：滑動視窗 Token 計數
- **多代理擴展**：單執行緒運行時適合微服務部署

---

## 9. 與 Clawtex-Core 的集成要點

針對 **clawtex-core** 的架構參考：

1. **Provider Trait 融合**
   - AutoAgents 的 `LLMProvider` ≈ Clawtex `Provider` trait
   - 建議支援 OpenAI 相容性、Anthropic、Ollama（同 Clawtex）

2. **多代理協調**
   - AutoAgents 的 `Runtime` + `Topic<T>` 可做為多工作流編排參考
   - Clawtex Hands 引擎改進：增加類型化 Pub/Sub

3. **工具系統**
   - AutoAgents 的 `ToolT` 衍生宏模式優於手寫
   - WASM 沙箱執行可提升安全性（Clawtex shell 允許清單改進）

4. **記憶層**
   - Clawtex 的 `memory_store` ≈ AutoAgents `MemoryProvider`
   - 實現滑動視窗可減少 token 成本

5. **守衛欄**
   - AutoAgents 的 `GuardrailsEngine` 可補充 Clawtex 安全需求
   - 支援 PII 淘汰、提示注入防護

---

## 10. 延伸閱讀

- **源始碼**：https://github.com/liquidos-ai/AutoAgents
- **文檔**：https://liquidos-ai.github.io/AutoAgents/
- **Cargo 文檔**：`cargo doc --open`（本地建構）
- **Clawtex 參考對比**：`docs/references/autoagents/` 本資料夾

---

**文檔建立日期**：2026-03-13
**掃描範圍**：autoagents-core, autoagents-llm, autoagents-derive, workspace layout
**繁體中文版本**：v1.0
