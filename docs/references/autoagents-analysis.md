# AutoAgents 深度技術分析 (v2)

> **專案**: AutoAgents v0.3.6 -- Rust 多代理框架
> **原始碼**: `LLM-Cluster-Project/references/autoagents/`
> **分析日期**: 2026-03-13
> **分析者**: clawtex-core 技術參考
> **深度**: Level 2 -- 含實際程式碼片段、資料流圖、錯誤處理路徑、效能特徵

---

## 目錄

1. [專案結構與 Crate 拓撲](#1-專案結構與-crate-拓撲)
2. [入口點與啟動流程](#2-入口點與啟動流程)
3. [Actor Model (Ractor) 深度解析](#3-actor-model-ractor-深度解析)
4. [LLMLayer Pipeline 深度解析](#4-llmlayer-pipeline-深度解析)
5. [`#[tool]` Proc Macro 完整展開](#5-tool-proc-macro-完整展開)
6. [AgentHooks 十個生命週期鉤子](#6-agenthooks-十個生命週期鉤子)
7. [WASM Sandbox 深度解析](#7-wasm-sandbox-深度解析)
8. [Three-Layer Streaming 深度解析](#8-three-layer-streaming-深度解析)
9. [Executor Pattern -- ReAct vs Basic](#9-executor-pattern----react-vs-basic)
10. [Guardrails Engine 深度解析](#10-guardrails-engine-深度解析)
11. [Memory Systems](#11-memory-systems)
12. [Multi-Agent Coordination](#12-multi-agent-coordination)
13. [Structured Output 型別安全](#13-structured-output-型別安全)
14. [錯誤處理架構](#14-錯誤處理架構)
15. [效能特徵分析](#15-效能特徵分析)
16. [與 clawtex-core 完整差距對比](#16-與-clawtex-core-完整差距對比)
17. [附錄: 關鍵檔案路徑索引](#附錄-關鍵檔案路徑索引)

---

## 1. 專案結構與 Crate 拓撲

AutoAgents 採用 Cargo workspace 多 crate 架構,劃分極為清晰:

```
autoagents/
├── Cargo.toml                          # workspace root, edition 2024
├── crates/
│   ├── autoagents/                     # 頂層 facade crate (prelude 重新匯出)
│   ├── autoagents-core/                # 核心: actor, agent, tool, runtime, memory
│   ├── autoagents-llm/                 # LLM 抽象層: backends, chat, completion, embedding
│   ├── autoagents-derive/              # proc-macro: #[agent], #[tool], AgentOutput, ToolInput
│   ├── autoagents-protocol/            # Event, Task, ToolCallResult 協定型別
│   ├── autoagents-toolkit/             # 內建工具: filesystem, search, mcp, document_parsing
│   ├── autoagents-guardrails/          # 護欄引擎: PII 過濾, 毒性偵測, prompt injection
│   ├── autoagents-llamacpp/            # llama.cpp 本地推理後端
│   ├── autoagents-mistral-rs/          # mistral.rs 本地推理後端
│   ├── autoagents-qdrant/              # Qdrant 向量資料庫整合
│   ├── autoagents-speech/              # STT/TTS (Parakeet, PocketTTS, Silero VAD)
│   └── autoagents-telemetry/           # 遙測 (Langfuse 整合)
├── examples/                           # 14+ 範例: basic, coding_agent, design_patterns, etc.
└── bindings/python/                    # PyO3 Python 綁定
```

### 1.1 Crate 依賴拓撲圖

```
                    ┌─────────────────────┐
                    │   autoagents        │  facade (pub use re-exports)
                    │   (prelude)         │
                    └──┬──────┬───────┬───┘
                       │      │       │
           ┌───────────┘      │       └────────────┐
           ▼                  ▼                     ▼
┌──────────────────┐  ┌──────────────┐  ┌──────────────────┐
│ autoagents-core  │  │autoagents-   │  │ autoagents-      │
│                  │  │derive        │  │ guardrails       │
│ actor/           │  │              │  │                  │
│ agent/           │◄─┤ #[agent]     │  │ InputGuard       │
│ tool/            │  │ #[tool]      │  │ OutputGuard      │
│ runtime/         │  │ AgentOutput  │  │ GuardrailsEngine │
│ environment.rs   │  │ ToolInput    │  │ EnforcementPolicy│
│ event_fanout.rs  │  │ AgentHooks   │  └──────┬───────────┘
│ vector_store/    │  └──────────────┘         │
└──────┬───────────┘                           │
       │                                       │
       ▼                                       ▼
┌──────────────────┐              ┌────────────────────┐
│ autoagents-llm   │◄─────────────┤ GuardrailsLayer    │
│                  │              │ implements LLMLayer │
│ LLMProvider trait│              └────────────────────┘
│ pipeline/        │
│ optim/           │
│   cache.rs       │
│   retry.rs       │
│   fallback.rs    │
│ backends/        │
│   openai.rs      │
│   anthropic.rs   │
│   ollama.rs      │
│   google.rs      │
│   groq.rs        │
│   deepseek.rs    │
│   ... (11 total) │
│ chat/mod.rs      │
│ embedding/       │
│ completion/      │
└──────┬───────────┘
       │
       ▼
┌──────────────────┐
│autoagents-       │
│protocol          │
│                  │
│ Event enum       │
│ SubmissionId     │
│ ActorID          │
│ RuntimeID        │
└──────────────────┘
```

### 1.2 關鍵依賴版本

| 依賴 | 版本 | 用途 |
|------|------|------|
| `ractor` | 0.15.7 | Actor 框架 (Erlang-style) |
| `tokio` | 1.43 | 非同步 runtime |
| `reqwest` | 0.13 | HTTP 客戶端 |
| `wasmtime` | 42.0 | WASM 沙箱執行環境 |
| `rmcp` | 0.15 | MCP (Model Context Protocol) |
| `minijinja` | 2.3 | 模板引擎 |
| `serde_json` | 1.x | JSON 序列化 |
| `futures` | 0.3 | Stream trait 抽象 |

> **Clawtex 對比**: clawtex-core 是單 crate 單體架構,所有功能在 `src/` 下。AutoAgents 的 workspace 分離讓每個子系統可以獨立版本演進。

---

## 2. 入口點與啟動流程

AutoAgents 本身不是 daemon -- 它是一個 **library framework**。使用者在自己的 `main.rs` 中組合元件。

### 2.1 Direct Agent (無 Actor 模式)

最簡單的用法,不需要 Runtime,直接呼叫 `agent.run(task)`:

```rust
// examples/basic/src/simple.rs
// 1. 建立 LLM Provider
let llm: Arc<dyn LLMProvider> = LLMBuilder::new()
    .backend(LLMBackend::OpenAI)
    .api_key("sk-...")
    .model("gpt-4o")
    .build();

// 2. 定義 Agent (透過 proc-macro)
#[agent(
    name = "math_agent",
    description = "Performs math operations",
    tools = [Addition, Multiplication],
    output = MathAgentOutput,
)]
#[derive(Default, Clone, AgentHooks)]
pub struct MathAgent {}

// 3. 包裝為 ReAct executor
let react_agent = ReActAgent::new(MathAgent {});

// 4. 使用 AgentBuilder 建構
let handle = AgentBuilder::<_, DirectAgent>::new(react_agent)
    .llm(llm)
    .memory(Box::new(SlidingWindowMemory::new(10)))
    .build()
    .await?;

// 5. 執行任務
let result = handle.agent.run(Task::new("What is 1 + 1?")).await?;
```

**資料流**:
```
Task::new("What is 1+1?")
  │
  ▼
BaseAgent<ReActAgent<MathAgent>, DirectAgent>::run()
  │
  ├─ on_run_start() hook → HookOutcome::Continue
  │
  ├─ ReActAgent::execute(task, context)
  │   │
  │   ├─ TurnEngine::new(TurnEngineConfig::react(10))
  │   │
  │   └─ for turn_index in 0..max_turns {
  │       │
  │       ├─ TurnEngine::run_turn()
  │       │   ├─ MemoryAdapter::recall() → 歷史訊息
  │       │   ├─ 組建 messages: [system, recalled..., user]
  │       │   ├─ llm.chat_with_tools(messages, tools, schema)
  │       │   ├─ 解析回應: text + tool_calls
  │       │   ├─ if tool_calls:
  │       │   │   ├─ hooks.on_tool_call() → Abort? skip
  │       │   │   ├─ hooks.on_tool_start()
  │       │   │   ├─ ToolProcessor::process_single_tool_call()
  │       │   │   ├─ hooks.on_tool_result() / on_tool_error()
  │       │   │   └─ MemoryAdapter::store() tool interaction
  │       │   └─ return TurnResult::Continue | Complete
  │       │
  │       ├─ TurnResult::Complete → break with output
  │       └─ TurnResult::Continue → next turn
  │   }
  │
  ├─ on_run_complete() hook
  │
  └─ return ReActAgentOutput { response, tool_calls, done: true }
```

### 2.2 Actor Agent (完整 Actor 模式)

```rust
// examples/basic/src/actor.rs
// 1. 建立 Runtime 與 Environment
let runtime = SingleThreadedRuntime::new(Some(10));
let mut environment = Environment::new(None);
environment.register_runtime(runtime.clone()).await?;

// 2. 建立 Topic (型別安全的 pub/sub 頻道)
let topic = Topic::<Task>::new("jobs");

// 3. 建構 Actor Agent
let handle = AgentBuilder::<_, ActorAgent>::new(ReActAgent::new(agent_impl))
    .llm(llm)
    .runtime(runtime.clone())
    .subscribe(topic)
    .build()
    .await?;

// 4. 透過 ActorRef 發送訊息
handle.addr().cast(Task::new("process this"))?;

// 5. 或透過 Topic 廣播
runtime.publish(&topic, Task::new("broadcast task")).await?;
```

> **Clawtex 對比**: clawtex-core 的 agent 沒有 Actor/Direct 雙模式。所有 agent 透過 Telegram 驅動,共用 `AppState`。AutoAgents 的分離設計讓測試極為方便 -- DirectAgent 不需要任何 Runtime 依賴。

---

## 3. Actor Model (Ractor) 深度解析

### 3.1 核心型別層級

AutoAgents 在 Ractor 的 Actor 模型之上建構了四層抽象:

```
Layer 4: AgentActor              (crates/autoagents-core/src/agent/actor.rs)
          implements ractor::Actor for BaseAgent
          ↓
Layer 3: Runtime / Environment   (crates/autoagents-core/src/runtime/)
          manages actor lifecycle + topic subscriptions
          ↓
Layer 2: Topic<M> / Subscriber   (crates/autoagents-core/src/actor/topic.rs)
          compile-time typed pub/sub
          ↓
Layer 1: AnyActor / Transport    (crates/autoagents-core/src/actor/mod.rs)
          type-erased message dispatch
```

### 3.2 AnyActor -- 型別擦除層 (完整原始碼)

```rust
// crates/autoagents-core/src/actor/mod.rs

/// 型別擦除的 Actor 介面。
/// 讓 Runtime 可以用統一介面管理不同訊息型別的 actor。
#[async_trait]
pub trait AnyActor: Send + Sync + Debug {
    async fn send_any(
        &self,
        msg: Arc<dyn Any + Send + Sync>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

// 為 CloneableMessage 型別的 ActorRef 實作 AnyActor
// 透過 downcast_ref 還原具體型別後呼叫 ractor 的 cast()
#[async_trait]
impl<M: CloneableMessage + 'static> AnyActor for ActorRef<M> {
    async fn send_any(
        &self,
        msg: Arc<dyn Any + Send + Sync>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let msg = msg.downcast_ref::<M>()
            .ok_or("Message type mismatch")?;  // 執行時型別檢查
        self.cast(msg.clone()).map_err(|e| e.into())
    }
}

// 為 SharedMessage (非 Clone) 型別的 ActorRef 特殊實作
// 不衝突因為 SharedMessage<M> 不實作 CloneableMessage
#[async_trait]
impl<M: Send + Sync + 'static> AnyActor for ActorRef<SharedMessage<M>> {
    async fn send_any(
        &self,
        msg: Arc<dyn Any + Send + Sync>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let shared_msg = msg.downcast_ref::<SharedMessage<M>>()
            .ok_or("Message type mismatch")?;
        // Clone SharedMessage (clones Arc, not M) -- O(1) 成本
        self.cast(shared_msg.clone()).map_err(|e| e.into())
    }
}
```

**關鍵設計**: 兩個 `impl AnyActor for ActorRef<_>` 不衝突,因為 `SharedMessage<M>` **刻意不實作** `CloneableMessage`。這是一個精巧的 trait coherence 利用。

### 3.3 訊息系統三層架構 (完整原始碼)

```rust
// crates/autoagents-core/src/actor/messaging.rs

/// Layer 1: 所有 Actor 訊息的基底 trait
pub trait ActorMessage: Send + Sync + 'static {}

/// Layer 2: 可 Clone 的訊息 (適用於 pub/sub 廣播)
pub trait CloneableMessage: ActorMessage + Clone {}

/// Layer 3: 零成本共享的非 Clone 訊息
/// 內部用 Arc<M> 包裝,Clone 只增加引用計數 (O(1))
pub struct SharedMessage<M> {
    inner: Arc<M>,
}

// 手動實作 Clone -- 不需要 M: Clone
impl<M> Clone for SharedMessage<M> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),  // 只複製 Arc 指標
        }
    }
}

impl<M> SharedMessage<M> {
    pub fn new(msg: M) -> Self {
        Self { inner: Arc::new(msg) }
    }

    pub fn inner(&self) -> &M {
        &self.inner
    }

    pub fn into_inner(self) -> Arc<M> {
        self.inner
    }
}

// SharedMessage<M> 實作 ActorMessage 但 *不* 實作 CloneableMessage
// 這避免了 AnyActor 的 impl 衝突
impl<M: Send + Sync + 'static> ActorMessage for SharedMessage<M> {}
```

**效能特徵**:
- `CloneableMessage`: 每次 publish 到 N 個訂閱者 = N 次 deep clone + N 次 `downcast_ref`
- `SharedMessage`: 每次 publish = N 次 Arc::clone (只有指標複製) + N 次 `downcast_ref`
- 對於大型非 Clone 訊息 (如 Task 含大量 context),SharedMessage 節省大量記憶體

### 3.4 Topic<M> -- 編譯時型別安全 Pub/Sub (完整原始碼)

```rust
// crates/autoagents-core/src/actor/topic.rs

/// 泛型 Topic -- 編譯時型別安全
/// PhantomData<M> 確保不同訊息型別的 Topic 在型別系統中不可互換
#[derive(Clone)]
pub struct Topic<M: ActorMessage> {
    name: String,
    id: Uuid,                    // 唯一識別 (同名 Topic 也有不同 id)
    _phantom: PhantomData<M>,    // 零大小, 只存在於型別系統
}

impl<M: ActorMessage> Topic<M> {
    pub fn new(name: impl Into<String>) -> Self {
        Topic {
            name: name.into(),
            id: Uuid::new_v4(),
            _phantom: PhantomData,
        }
    }

    pub fn type_id(&self) -> TypeId {
        TypeId::of::<M>()   // 用於 Runtime 的執行時型別路由
    }
}
```

**編譯時安全保證**:
```rust
let task_topic = Topic::<Task>::new("work");
let event_topic = Topic::<Event>::new("events");

// 這會編譯失敗:
// runtime.publish(&task_topic, Event::SomeEvent); // 型別不匹配!

// 這才合法:
runtime.publish(&task_topic, Task::new("work")).await?;
```

### 3.5 TypedSubscriber -- 訂閱者分發

```rust
// crates/autoagents-core/src/actor/subscriber.rs

/// CloneableMessage 的訂閱者 -- 訊息被 clone 給每個 actor
pub struct TypedSubscriber<M: CloneableMessage> {
    actors: Vec<Box<dyn AnyActor>>,
    _marker: PhantomData<M>,
}

impl<M: CloneableMessage + 'static> TypedSubscriber<M> {
    pub fn add(&mut self, actor: ActorRef<M>) {
        // ActorRef<M> 被 upcast 為 Box<dyn AnyActor>
        self.actors.push(Box::new(actor) as Box<dyn AnyActor>);
    }

    pub async fn publish(&self, message: M) {
        let arc_msg: Arc<dyn Any + Send + Sync> = Arc::new(message);
        for actor in &self.actors {
            // 每個 actor 收到相同的 Arc clone
            // AnyActor::send_any 內部 downcast 回 M
            let _ = actor.send_any(arc_msg.clone()).await;
        }
    }
}

/// SharedMessage 的訂閱者 -- 零成本共享
pub struct SharedSubscriber<M: Send + Sync + 'static> {
    actors: Vec<Box<dyn AnyActor>>,
    _marker: PhantomData<M>,
}

impl<M: Send + Sync + 'static> SharedSubscriber<M> {
    pub fn add(&mut self, actor: ActorRef<SharedMessage<M>>) {
        self.actors.push(Box::new(actor) as Box<dyn AnyActor>);
    }

    pub async fn publish(&self, message: SharedMessage<M>) {
        let arc_msg: Arc<dyn Any + Send + Sync> = Arc::new(message);
        for actor in &self.actors {
            let _ = actor.send_any(arc_msg.clone()).await;
        }
    }
}
```

### 3.6 Transport -- 位置透明性抽象

```rust
// crates/autoagents-core/src/actor/transport.rs

/// 訊息傳輸抽象 -- 為未來遠端 Actor 預留擴充點
#[async_trait]
pub trait Transport: Send + Sync + Debug {
    async fn send(
        &self,
        actor: &dyn AnyActor,
        msg: Arc<dyn Any + Send + Sync>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// 本地傳輸 -- 直接呼叫 AnyActor::send_any()
#[derive(Debug)]
pub struct LocalTransport;

#[async_trait]
impl Transport for LocalTransport {
    async fn send(
        &self,
        actor: &dyn AnyActor,
        msg: Arc<dyn Any + Send + Sync>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        actor.send_any(msg).await  // 零成本本地呼叫
    }
}
```

**擴充點**: 未來可以實作 `RemoteTransport` 透過網路發送訊息給遠端 Actor,而上層程式碼完全不需要改動。

### 3.7 與 Clawtex AgentEventBus 對比

```
┌──────────────────────────────────────────────────────────────────────────┐
│                   AutoAgents Actor System                               │
│                                                                         │
│  Topic<Task> ──→ TypedSubscriber<Task>                                  │
│       │              │                                                  │
│       │              ├─ ActorRef<Task> [Agent A] ← downcast + cast()    │
│       │              ├─ ActorRef<Task> [Agent B] ← downcast + cast()    │
│       │              └─ ActorRef<Task> [Agent C] ← downcast + cast()    │
│       │                                                                 │
│  Topic<Event> ──→ TypedSubscriber<Event>                                │
│       │              └─ ActorRef<Event> [Logger]                        │
│                                                                         │
│  優勢: 編譯時型別安全, 每 Topic 獨立路由, 零成本 SharedMessage          │
└──────────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────────┐
│                   Clawtex AgentEventBus                                  │
│                                                                         │
│  broadcast::Sender<AgentEvent> ──→ broadcast::Receiver [subscriber 1]    │
│                                ──→ broadcast::Receiver [subscriber 2]    │
│                                                                         │
│  所有事件混在同一個 channel: RunStarted, ToolCalled, Error, etc.          │
│                                                                         │
│  缺陷: 無型別分離, 每個 subscriber 必須 match 過濾, 無選擇性訂閱        │
└──────────────────────────────────────────────────────────────────────────┘
```

> **Clawtex 實作建議**:
> 1. 定義 `TypedEventChannel<T: Clone + Send>` 包裝 `broadcast::Sender<T>`,支援編譯時型別路由
> 2. 將 `AgentEvent` 拆分為 `ToolEvent`, `RunEvent`, `CostEvent` 等子型別,各自有獨立 channel
> 3. 為 cluster_hub 實作 `RemoteTransport` -- 將 AgentEvent 序列化後透過 HTTP 發送給 worker

---

## 4. LLMLayer Pipeline 深度解析

### 4.1 核心 Trait (完整原始碼)

```rust
// crates/autoagents-llm/src/pipeline/mod.rs

/// 中介層 trait -- 完全開放,外部 crate 可自由實作
/// 唯一簽名依賴是 Arc<dyn LLMProvider>,無內部耦合
pub trait LLMLayer: Send + Sync + 'static {
    /// 消耗 self 並產出包裝了 next 的 provider
    /// Box<Self> 允許 trait object 存儲同時消耗 self
    fn build(self: Box<Self>, next: Arc<dyn LLMProvider>) -> Arc<dyn LLMProvider>;
}
```

### 4.2 PipelineBuilder -- 洋蔥模型組裝器 (完整原始碼)

```rust
// crates/autoagents-llm/src/pipeline/mod.rs

pub struct PipelineBuilder {
    base: Arc<dyn LLMProvider>,
    layers: Vec<Box<dyn LLMLayer>>,
}

impl PipelineBuilder {
    pub fn new(provider: Arc<dyn LLMProvider>) -> Self {
        Self { base: provider, layers: Vec::new() }
    }

    /// 第一個 add 的 layer 是最外層 (第一個攔截請求)
    pub fn add_layer<L: LLMLayer>(mut self, layer: L) -> Self {
        self.layers.push(Box::new(layer));
        self
    }

    /// 反向組裝: 最後加的 layer 最先包裝 base
    pub fn build(self) -> Arc<dyn LLMProvider> {
        let mut provider = self.base;
        for layer in self.layers.into_iter().rev() {
            provider = layer.build(provider);
        }
        provider
    }
}
```

### 4.3 資料流圖: Pipeline 請求路徑

```
使用者呼叫: pipeline.chat_with_tools(messages, tools, schema)
     │
     ▼
┌────────────────────────┐
│    CacheLayer          │  ← 第一個 add_layer() = 最外層
│                        │
│  1. hash(messages) → key
│  2. read_lock → cache hit? → return cached
│  3. miss → delegate to next
│  4. write_lock → store result
│  5. streaming: TeeStream 透傳 + 累積 → cache on Done
└────────┬───────────────┘
         │ cache miss
         ▼
┌────────────────────────┐
│    RetryLayer          │
│                        │
│  1. attempt 0: call next
│  2. if retryable error:
│     sleep(backoff * 2^attempt ± jitter)
│     retry up to max_attempts
│  3. non-retryable error: propagate immediately
│  4. 不會重試: AuthError, InvalidRequest, JsonError
└────────┬───────────────┘
         │
         ▼
┌────────────────────────┐
│    FallbackLayer       │
│                        │
│  providers = [primary, fallback_1, fallback_2]
│  1. try primary
│  2. if fallbackable error → try fallback_1
│  3. if fallbackable error → try fallback_2
│  4. AuthError → 不 fallback, 直接返回錯誤
│  5. NoToolSupport → 會 fallback (讓本地模型 fallback 到雲端)
└────────┬───────────────┘
         │
         ▼
┌────────────────────────┐
│   GuardrailsLayer      │
│                        │
│  Input:                │
│   1. evaluate_input(messages) → Block/Sanitize/Audit
│   2. 通過 → delegate to next
│  Output:               │
│   3. get response from next
│   4. evaluate_output(response) → Block/Sanitize/Audit
│   5. 通過 → return response
└────────┬───────────────┘
         │
         ▼
┌────────────────────────┐
│   Base Provider        │
│   (OpenAI/Anthropic/   │
│    Ollama/Gemini/etc)  │
│                        │
│   HTTP POST → API      │
│   parse response       │
│   return ChatResponse  │
└────────────────────────┘
```

### 4.4 CacheLayer 深度 -- TeeStream 串流快取

CacheLayer 的串流快取設計特別精巧:

```rust
// crates/autoagents-llm/src/optim/cache.rs (概念簡化)

/// TeeStream: 透傳串流給消費者的同時累積 buffer
/// 當串流結束成功時,一次性寫入快取
/// 失敗時丟棄 buffer,不快取
struct TeeStream<S, T> {
    inner: S,                              // 原始串流
    buffer: Arc<Mutex<Vec<T>>>,            // 累積 buffer
    cache: Arc<RwLock<HashMap<u64, CacheEntry>>>,
    key: u64,
    ttl: Option<Duration>,
}

impl<S, T> Stream for TeeStream<S, T>
where S: Stream<Item = Result<T, LLMError>> + Unpin,
      T: Clone
{
    type Item = Result<T, LLMError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                self.buffer.lock().unwrap().push(chunk.clone());
                Poll::Ready(Some(Ok(chunk)))  // 透傳給消費者
            }
            Poll::Ready(None) => {
                // 串流結束 -- 寫入快取 (同步鎖, 不跨 await)
                let buffer = self.buffer.lock().unwrap().drain(..).collect();
                self.cache.write().unwrap().insert(self.key, CacheEntry {
                    chunks: buffer,
                    created_at: Instant::now(),
                    ttl: self.ttl,
                });
                Poll::Ready(None)
            }
            Poll::Ready(Some(Err(e))) => {
                // 錯誤 -- 丟棄 buffer, 不快取
                self.buffer.lock().unwrap().clear();
                Poll::Ready(Some(Err(e)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
```

**效能關鍵設計**:
- **讀鎖不跨 await**: cache 查詢用 `read_lock`,取得後立即 drop,再 await inner provider
- **寫鎖不跨 await**: inner call 完成後才取 `write_lock` 寫入
- **Single-flight**: 同一 key 的並發 miss 使用 per-key async mutex 合併,避免重複上游呼叫
- **Web search 永不快取**: `chat_with_web_search` 直接 delegate,因為結果是時效性的

### 4.5 RetryLayer 深度 -- 指數退避 + Full Jitter

```rust
// crates/autoagents-llm/src/optim/retry.rs

pub struct RetryConfig {
    pub max_attempts: u32,              // 總嘗試次數 (含首次), 預設 3
    pub initial_backoff: Duration,      // 首次重試延遲, 預設 200ms
    pub max_backoff: Duration,          // 延遲上限, 預設 30s
    pub jitter: bool,                   // Full jitter (防 thundering herd)
    pub retryable: fn(&LLMError) -> bool, // 可重試判斷函數
}

/// 指數退避計算: ceiling = min(max_backoff, initial * 2^attempt)
fn compute_backoff(config: &RetryConfig, attempt: u32) -> Duration {
    let initial_ns = config.initial_backoff.as_nanos().min(u64::MAX as u128) as u64;
    let multiplier = 1u64.checked_shl(attempt).unwrap_or(u64::MAX); // 防溢出
    let max_ns = config.max_backoff.as_nanos().min(u64::MAX as u128) as u64;
    let ceiling = Duration::from_nanos(initial_ns.saturating_mul(multiplier).min(max_ns));
    if config.jitter {
        jitter_duration(ceiling)  // [0, ceiling] 均勻分佈
    } else {
        ceiling
    }
}

/// 預設可重試判斷: 429/5xx/overloaded → 重試, auth/parse → 不重試
pub fn default_is_retryable(err: &LLMError) -> bool {
    match err {
        LLMError::HttpError(msg) | LLMError::ProviderError(msg) => {
            let m = msg.to_lowercase();
            m.contains("429") || m.contains("500") || m.contains("502")
            || m.contains("503") || m.contains("504") || m.contains("529") // Anthropic
            || m.contains("rate limit") || m.contains("overloaded")
        }
        LLMError::Generic(_) => true,
        LLMError::AuthError(_) | LLMError::InvalidRequest(_)
        | LLMError::GuardrailBlocked { .. } | LLMError::JsonError(_)
        | LLMError::ToolConfigError(_) | LLMError::NoToolSupport(_) => false,
    }
}

/// 核心重試迴圈 -- hot path (首次成功) 零分配
async fn retry_call<F, Fut, T>(config: &RetryConfig, mut f: F) -> Result<T, LLMError>
where F: FnMut() -> Fut, Fut: Future<Output = Result<T, LLMError>>
{
    let max = config.max_attempts.max(1);
    let mut attempt = 0u32;
    loop {
        match f().await {
            Ok(v) => return Ok(v),          // 首次成功 → 直接返回
            Err(e) if attempt + 1 < max && (config.retryable)(&e) => {
                let backoff = compute_backoff(config, attempt);
                log::warn!("LLM call failed (attempt {}/{}): {e}. Retrying in {backoff:?}.",
                    attempt + 1, max);
                tokio::time::sleep(backoff).await;
                attempt += 1;
            }
            Err(e) => return Err(e),        // 不可重試或已耗盡
        }
    }
}
```

### 4.6 FallbackLayer 深度 -- 多 Provider 降級

```rust
// crates/autoagents-llm/src/optim/fallback.rs

pub struct FallbackLayer {
    fallbacks: Vec<Arc<dyn LLMProvider>>,
    config: FallbackConfig,
}

impl LLMLayer for FallbackLayer {
    fn build(self: Box<Self>, next: Arc<dyn LLMProvider>) -> Arc<dyn LLMProvider> {
        // 預建 provider 列表: primary + fallbacks
        // 避免 hot path 上的分配
        let mut providers = Vec::with_capacity(1 + self.fallbacks.len());
        providers.push(next);          // primary 總是第一個
        providers.extend(self.fallbacks);
        Arc::new(FallbackProvider { providers, config: self.config })
    }
}

/// 核心降級迴圈 -- hot path (primary 成功) 只有一次 Arc clone + match
async fn try_fallback<F, Fut, T>(
    providers: &[Arc<dyn LLMProvider>],
    config: &FallbackConfig,
    mut f: F,
) -> Result<T, LLMError>
where F: FnMut(Arc<dyn LLMProvider>) -> Fut, Fut: Future<Output = Result<T, LLMError>>
{
    let mut last_err: Option<LLMError> = None;
    for (idx, provider) in providers.iter().enumerate() {
        match f(Arc::clone(provider)).await {
            Ok(v) => return Ok(v),
            Err(e) if (config.fallbackable)(&e) => {
                log::warn!("LLM [{idx}] failed: {e}. Trying next provider.");
                last_err = Some(e);
            }
            Err(e) => return Err(e),  // AuthError 等不可降級的錯誤
        }
    }
    Err(last_err.unwrap_or_else(|| LLMError::Generic("No providers available".into())))
}
```

**關鍵設計**: `NoToolSupport` 被標記為 **可降級**,這讓本地小模型可以 fallback 到雲端大模型處理 tool calling。

> **Clawtex 實作建議**:
> 1. 定義 `ProviderLayer` trait 取代目前 `ReliableProvider` 的 hardcoded circuit breaker
> 2. 實作 `RetryLayer` (帶 full jitter) + `FallbackLayer` + `CacheLayer`
> 3. 改 `ProviderRouter` 為 pipeline 組裝器: `PipelineBuilder::new(primary).add_layer(RetryLayer).add_layer(FallbackLayer::new(fallbacks)).build()`
> 4. 在 agents.toml 中用宣告式語法配置 pipeline:
>    ```toml
>    [providers.master.pipeline]
>    layers = ["retry(max=3)", "fallback(anthropic,groq)", "cache(ttl=3600)"]
>    ```

---

## 5. `#[tool]` Proc Macro 完整展開

### 5.1 使用者程式碼

```rust
// 步驟 1: 定義輸入型別 (derive ToolInput)
#[derive(Serialize, Deserialize, ToolInput, Debug)]
pub struct AdditionArgs {
    #[input(description = "Left Operand")]
    left: i64,
    #[input(description = "Right Operand")]
    right: i64,
}

// 步驟 2: 定義工具結構 (attribute macro #[tool])
#[tool(name = "Addition", description = "Add two numbers", input = AdditionArgs)]
struct Addition {}

// 步驟 3: 手動實作執行邏輯
#[async_trait]
impl ToolRuntime for Addition {
    async fn execute(&self, args: Value) -> Result<Value, ToolCallError> {
        let typed_args: AdditionArgs = serde_json::from_value(args)?;
        Ok(serde_json::json!(typed_args.left + typed_args.right))
    }
}
```

### 5.2 ToolInput derive 展開 (InputParser)

`#[derive(ToolInput)]` 經過 `InputParser` 處理,展開為:

```rust
// 編譯時產生的 JSON Schema 字串
impl ToolInputT for AdditionArgs {
    fn io_schema() -> &'static str {
        r#"{"properties":{"left":{"description":"Left Operand","type":"number"},"right":{"description":"Right Operand","type":"number"}},"required":["left","right"],"type":"object"}"#
    }
}
```

**InputParser 的型別映射** (`crates/autoagents-derive/src/tool/input.rs`):

```rust
fn get_base_json_type(&self, type_str: &str) -> JsonType {
    match type_str {
        "String" | "str" => JsonType::String,
        "i32" | "u32" | "f64" | "f32" | "u8" | "i64" | "u16"
        | "usize" | "isize" => JsonType::Number,
        "bool" => JsonType::Boolean,
        _ => JsonType::String,  // 自訂型別 fallback 到 string
    }
}
```

**Optional 處理**: `Option<T>` 欄位自動從 `required` 列表移除:

```rust
fn get_json_type(&mut self, field_type: &Type) -> Result<(JsonType, bool)> {
    // ... 如果是 Option<T>, 取出 T 的型別, 回傳 (type, optional=true)
    if segment.ident == "Option" {
        let (json_type, _) = self.get_json_type(inner)?;
        return Ok((json_type, true));  // 不加入 required
    }
}
```

**choice 列舉支援**:

```rust
#[derive(ToolInput)]
pub struct SearchArgs {
    #[input(description = "Search engine", choice = ["google", "bing", "duckduckgo"])]
    engine: String,
}
// 展開為: "engine": {"type": "string", "enum": ["google", "bing", "duckduckgo"]}
```

### 5.3 `#[tool]` attribute macro 展開 (ToolParser)

```rust
// crates/autoagents-derive/src/tool/mod.rs
// #[tool(name = "Addition", description = "Add two numbers", input = AdditionArgs)]
// struct Addition {}

// ============= 展開後 =============

struct Addition {}

impl autoagents::core::tool::ToolT for Addition {
    fn name(&self) -> &str {
        "Addition"
    }
    fn description(&self) -> &str {
        "Add two numbers"
    }
    fn args_schema(&self) -> serde_json::Value {
        let params_str = <AdditionArgs as ToolInputT>::io_schema();
        serde_json::from_str(params_str)
            .expect("Failed to parse parameters schema")
    }
}

impl std::fmt::Debug for Addition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Addition")  // 用工具名作為 Debug 輸出
    }
}
```

### 5.4 `#[agent]` attribute macro 展開

```rust
// #[agent(name = "math", description = "Math agent",
//         tools = [Addition, Multiplication], output = MathOutput)]
// struct MathAgent {}

// ============= 展開後 =============

struct MathAgent {}

impl autoagents::core::agent::AgentDeriveT for MathAgent {
    type Output = MathOutput;

    fn name(&self) -> &'static str { "math" }
    fn description(&self) -> &'static str { "Math agent" }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(<MathOutput>::structured_output_format())
    }

    fn tools(&self) -> Vec<Box<dyn autoagents::core::tool::ToolT>> {
        vec![
            Box::new(Addition {}) as Box<dyn autoagents::core::tool::ToolT>,
            Box::new(Multiplication {}) as Box<dyn autoagents::core::tool::ToolT>,
        ]
    }
}

impl std::fmt::Debug for MathAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "math")
    }
}
```

### 5.5 `#[derive(AgentHooks)]` 展開

```rust
// #[derive(AgentHooks)]
// struct MathAgent {}

// ============= 展開後 =============

#[::autoagents::async_trait]
impl ::autoagents::core::agent::AgentHooks for MathAgent {}
// 所有 10 個方法使用預設空實作
```

> **Clawtex 實作建議**:
> 1. clawtex 使用 TOML 配置定義 tool,不需要 proc-macro。但可以借鑒 ToolInput 的 JSON Schema 自動生成
> 2. 對於未來的 plugin 系統,可以用 `#[clawtex_tool]` macro 讓 Rust plugin 宣告式定義工具
> 3. Schema 生成邏輯 (type mapping, Option 處理, choice enum) 可以在 clawtex 的 tool registry 中複用

---

## 6. AgentHooks 十個生命週期鉤子

### 6.1 完整 Trait 定義與觸發時機

```rust
// crates/autoagents-core/src/agent/hooks.rs

#[derive(PartialEq)]
pub enum HookOutcome {
    Continue,  // 繼續執行
    Abort,     // 中止執行
}

#[async_trait]
pub trait AgentHooks: AgentDeriveT + Send + Sync {
    // ═══════════════════════════════════════════
    // Agent 生命週期 (2 hooks)
    // ═══════════════════════════════════════════

    /// #1 建立時回呼 -- AgentBuilder::build() 內部呼叫
    /// 觸發時機: BaseAgent::new() 完成後, Actor::spawn() 之前
    /// 用途: 初始化資源, 建立連線, 載入設定
    async fn on_agent_create(&self) {}

    /// #10 關閉時回呼 -- Actor post_stop() 內部呼叫
    /// 觸發時機: Actor 停止後 (只對 ActorAgent 有效, DirectAgent 無效)
    /// 用途: 清理資源, 關閉連線, 持久化狀態
    async fn on_agent_shutdown(&self) {}

    // ═══════════════════════════════════════════
    // 執行生命週期 (2 hooks, on_run_start 支援 Abort)
    // ═══════════════════════════════════════════

    /// #2 執行開始 -- BaseAgent::run() 進入後立即呼叫
    /// 觸發時機: task 接收後, executor.execute() 之前
    /// 返回 Abort → 整個 run 中止, 回傳 RunnableAgentError::Abort
    /// 用途: 權限檢查, rate limiting, 日誌記錄
    async fn on_run_start(&self, _task: &Task, _ctx: &Context) -> HookOutcome {
        HookOutcome::Continue
    }

    /// #3 執行完成 -- executor.execute() 返回 Ok 後呼叫
    /// 觸發時機: 最終結果產出後, 返回給呼叫者之前
    /// 用途: 結果驗證, 遙測上報, 計費記錄
    async fn on_run_complete(&self, _task: &Task, _result: &Self::Output, _ctx: &Context) {}

    // ═══════════════════════════════════════════
    // 回合生命週期 (2 hooks, 只對多回合 executor 有意義)
    // ═══════════════════════════════════════════

    /// #4 回合開始 -- TurnEngine::run_turn() 進入後呼叫
    /// 觸發時機: 每個 turn 的 LLM 呼叫之前
    /// 用途: 進度追蹤, context window 管理, 動態調整 prompt
    async fn on_turn_start(&self, _turn_index: usize, _ctx: &Context) {}

    /// #5 回合完成 -- TurnEngine::run_turn() 返回前呼叫
    /// 觸發時機: LLM 回應處理完畢, tool calls 執行完畢後
    /// 用途: 中間狀態記錄, 累積統計
    async fn on_turn_complete(&self, _turn_index: usize, _ctx: &Context) {}

    // ═══════════════════════════════════════════
    // 工具生命週期 (4 hooks, on_tool_call 支援 Abort)
    // ═══════════════════════════════════════════

    /// #6 工具呼叫決策 -- ToolProcessor 執行前呼叫
    /// 觸發時機: LLM 回傳 tool_call 後, 實際執行前
    /// 返回 Abort → 跳過此 tool call, 不執行, 不觸發 on_tool_start
    /// 用途: 工具白名單, 參數驗證, 審批閘門
    async fn on_tool_call(&self, _tool_call: &ToolCall, _ctx: &Context) -> HookOutcome {
        HookOutcome::Continue
    }

    /// #7 工具開始執行 -- on_tool_call 返回 Continue 後呼叫
    /// 觸發時機: 工具 execute() 之前
    /// 用途: 開始計時, 記錄調用參數
    async fn on_tool_start(&self, _tool_call: &ToolCall, _ctx: &Context) {}

    /// #8 工具執行成功 -- tool.execute() 返回 Ok 後呼叫
    /// 觸發時機: 工具成功執行後
    /// 用途: 結果快取, 結果轉換, 統計更新
    async fn on_tool_result(
        &self, _tool_call: &ToolCall, _result: &ToolCallResult, _ctx: &Context
    ) {}

    /// #9 工具執行失敗 -- tool.execute() 返回 Err 後呼叫
    /// 觸發時機: 工具執行失敗後
    /// 用途: 錯誤上報, fallback 策略, 重試決策
    async fn on_tool_error(&self, _tool_call: &ToolCall, _err: Value, _ctx: &Context) {}
}
```

### 6.2 Hooks 在 ToolProcessor 中的完整整合流程

```rust
// crates/autoagents-core/src/agent/executor/tool_processor.rs

pub(crate) async fn process_single_tool_call_with_hooks<H: AgentHooks>(
    hooks: &H,
    context: &Context,
    submission_id: SubmissionId,
    tools: &[Box<dyn ToolT>],
    call: &ToolCall,
    tx_event: &Option<mpsc::Sender<Event>>,
) -> Option<ToolCallResult> {

    // ┌─── Hook #6: on_tool_call (Abort gate) ───┐
    match hooks.on_tool_call(call, context).await {
        HookOutcome::Abort => {
            return None;  // 跳過整個 tool call
            // on_tool_start 不會被呼叫
            // on_tool_result/on_tool_error 不會被呼叫
        }
        HookOutcome::Continue => {}
    }
    // └──────────────────────────────────────────┘

    // ┌─── Hook #7: on_tool_start ───┐
    hooks.on_tool_start(call, context).await;
    // └──────────────────────────────┘

    // 實際執行工具
    let tool_context = ToolCallContext::new(submission_id, context.config().id);
    let result = Self::process_single_tool_call(tools, call, tool_context, tx_event).await;

    // ┌─── Hook #8 or #9: on_tool_result / on_tool_error ───┐
    if result.success {
        hooks.on_tool_result(call, &result, context).await;
    } else {
        hooks.on_tool_error(call, result.result.clone(), context).await;
    }
    // └─────────────────────────────────────────────────────┘

    Some(result)
}
```

### 6.3 Hook 觸發時序圖

```
BaseAgent::run(task)
│
├─#1 on_agent_create()        [只在 build 時呼叫一次]
│
├─#2 on_run_start(task, ctx)  ──→ Abort? → return Err(Abort)
│
├─ executor.execute(task, context)
│  │
│  ├─ Turn 0:
│  │  ├─#4 on_turn_start(0, ctx)
│  │  ├─ llm.chat_with_tools(...)
│  │  ├─ for each tool_call:
│  │  │  ├─#6 on_tool_call(call, ctx)  ──→ Abort? → skip this call
│  │  │  ├─#7 on_tool_start(call, ctx)
│  │  │  ├─ tool.execute(args)
│  │  │  ├─#8 on_tool_result(call, result, ctx)  [if success]
│  │  │  └─#9 on_tool_error(call, err, ctx)      [if failure]
│  │  └─#5 on_turn_complete(0, ctx)
│  │
│  ├─ Turn 1: (repeat)
│  │  └─ ...
│  │
│  └─ TurnResult::Complete(output)
│
├─#3 on_run_complete(task, output, ctx)
│
└─ return Ok(output)

[Actor shutdown]:
└─#10 on_agent_shutdown()     [只對 ActorAgent]
```

> **Clawtex 實作建議**:
> 1. clawtex 的 `agent_runtime.rs` 缺少 hook 系統。應在 tool 執行前加入 `on_tool_call` gate
> 2. 目前的 `approval.rs` 可以作為 `on_tool_call` hook 的實作 -- Abort 對應拒絕
> 3. 加入 `on_run_start` hook 用於 rate limiting (目前 `rate_limit` 在 `security.rs`)
> 4. hook 系統可以用 trait object `Vec<Box<dyn AgentHook>>` 實作,避免需要 derive macro

---

## 7. WASM Sandbox 深度解析

### 7.1 WasmRuntime 完整架構

```rust
// crates/autoagents-core/src/tool/runtime/wasm.rs

/// WASM 工具執行時 -- 基於 wasmtime capability-based 沙箱
pub struct WasmRuntime {
    engine: Engine,              // wasmtime 引擎 (可重用)
    module: Module,              // 編譯後的 WASM 模組
    config: WasmRuntimeConfig,   // alloc/execute/free 函數名稱
}

#[derive(Debug, Default)]
pub struct WasmRuntimeConfig {
    pub alloc_fn: String,        // WASM 記憶體分配函數名
    pub execute_fn: String,      // WASM 執行函數名
    pub free_fn: Option<String>, // 可選的記憶體釋放函數名
}
```

### 7.2 WASM 執行流程 -- JSON-in, JSON-out Protocol

```rust
impl WasmRuntime {
    pub fn run(&self, input: Value) -> Result<Value, WasmRuntimeError> {
        // ┌─── 1. 建立隔離的 Store (每次呼叫獨立) ───┐
        let mut store = Store::new(&self.engine, ());
        let linker = Linker::new(&self.engine);
        // └──────────────────────────────────────────┘

        // ┌─── 2. 實例化 WASM 模組 ───┐
        let instance = linker.instantiate(&mut store, &self.module)?;
        let memory = instance.get_memory(&mut store, "memory")
            .ok_or(WasmRuntimeError::MemoryAccess("No exported memory"))?;
        // └──────────────────────────┘

        // ┌─── 3. 取得型別安全的函數引用 ───┐
        let alloc: TypedFunc<i32, i32> =
            instance.get_typed_func(&mut store, &self.config.alloc_fn)?;
        let execute: TypedFunc<(i32, i32), i32> =
            instance.get_typed_func(&mut store, &self.config.execute_fn)?;
        let free: Option<TypedFunc<(i32, i32), ()>> = /* ... */;
        // └─────────────────────────────────┘

        // ┌─── 4. JSON → bytes → WASM memory ───┐
        let input_str = serde_json::to_string(&input)?;
        let input_bytes = input_str.as_bytes();

        // 在 WASM 記憶體中分配空間
        let ptr = alloc.call(&mut store, input_bytes.len() as i32)?;

        // 將 JSON bytes 寫入 WASM 記憶體
        memory.write(&mut store, ptr as usize, input_bytes)?;
        // └──────────────────────────────────────┘

        // ┌─── 5. 呼叫 WASM 函數 ───┐
        let result_ptr = execute.call(&mut store, (ptr, input_bytes.len() as i32))?;
        // └─────────────────────────┘

        // ┌─── 6. 讀取結果: [4 bytes length][result bytes] ───┐
        let mut len_buf = [0u8; 4];
        memory.read(&mut store, result_ptr as usize, &mut len_buf)?;
        let result_len = i32::from_le_bytes(len_buf) as usize;

        let mut result_bytes = vec![0u8; result_len];
        memory.read(&mut store, result_ptr as usize + 4, &mut result_bytes)?;
        // └───────────────────────────────────────────────────┘

        // ┌─── 7. bytes → JSON ───┐
        let json_str = String::from_utf8(result_bytes)?;
        let json_value = serde_json::from_str(&json_str)?;
        // └──────────────────────┘

        // ┌─── 8. 釋放 WASM 記憶體 (如果有 free 函數) ───┐
        if let Some(free_func) = free {
            free_func.call(&mut store, (result_ptr, (result_len + 4) as i32))?;
        }
        // └───────────────────────────────────────────────┘

        Ok(json_value)
    }
}
```

### 7.3 WASM 記憶體佈局

```
WASM Linear Memory:
┌──────────────────────────────────────────────────────┐
│  ... (WASM module's own data)                        │
├──────────────────────────────────────────────────────┤
│  Input (ptr, len):                                   │
│  ┌──────────────────────────────────────────────┐    │
│  │ {"left": 1, "right": 2}                      │    │
│  └──────────────────────────────────────────────┘    │
├──────────────────────────────────────────────────────┤
│  Result (result_ptr):                                │
│  ┌────────┬─────────────────────────────────────┐    │
│  │ len(4B)│ {"ok": true, "value": 3}            │    │
│  │ LE i32 │ (result_len bytes)                   │    │
│  └────────┴─────────────────────────────────────┘    │
└──────────────────────────────────────────────────────┘
```

### 7.4 安全性分析

| 安全特性 | 實作方式 |
|---------|---------|
| 記憶體隔離 | 每次 `run()` 建立新 `Store`,WASM 無法存取主機記憶體 |
| 無檔案系統存取 | `Linker` 未連結 WASI,WASM 無法存取檔案 |
| 無網路存取 | 無 socket 能力,WASM 無法發起網路請求 |
| CPU 限制 | wasmtime 支援 fuel-based execution limiting (未在此啟用) |
| 確定性 | WASM 執行是確定性的,同一輸入同一輸出 |

> **Clawtex 實作建議**:
> 1. clawtex 目前的 shell tool 用 `allowlist` 做安全控制,但仍然有逃逸風險
> 2. 可以為高風險工具 (shell, file_write) 加入 WASM 沙箱選項
> 3. 短期方案: 使用 `wasmtime` 的 fuel 限制防止無限迴圈
> 4. WASM 工具的 JSON-in/JSON-out protocol 可以直接複用 -- 與 clawtex 的 tool trait `fn execute(args: Value) -> Value` 完全匹配

---

## 8. Three-Layer Streaming 深度解析

### 8.1 三層串流架構圖

```
Layer 1: LLM Stream (per-token)
┌──────────────────────────────────────────────────────┐
│  ChatProvider::chat_stream_with_tools()              │
│                                                      │
│  Stream<Item = Result<StreamChunk, LLMError>>        │
│                                                      │
│  StreamChunk::Text("Hello ")                         │
│  StreamChunk::Text("world")                          │
│  StreamChunk::ReasoningContent("Let me think...")     │
│  StreamChunk::ToolUseStart { id, name }              │
│  StreamChunk::ToolUseInputDelta { partial_json }     │
│  StreamChunk::ToolUseComplete { tool_call }           │
│  StreamChunk::Done { stop_reason: "end_turn" }       │
│  StreamChunk::Usage(Usage { prompt: 100, ... })      │
└──────────┬───────────────────────────────────────────┘
           │
           ▼
Layer 2: TurnEngine Stream (per-turn)
┌──────────────────────────────────────────────────────┐
│  TurnEngine::run_turn_stream()                       │
│                                                      │
│  Stream<Item = Result<TurnDelta, TurnEngineError>>   │
│                                                      │
│  聚合 StreamChunk → TurnDelta:                       │
│  - Text chunks → TurnDelta::Text(accumulated)        │
│  - ToolUseComplete → execute tool → collect results  │
│    → TurnDelta::ToolResults(Vec<ToolCallResult>)     │
│  - Done → TurnDelta::Done(TurnResult<TurnEngineOutput>)
│                                                      │
│  轉換邏輯:                                           │
│  1. 收集所有 Text → 串接為完整回應                    │
│  2. 收集所有 ToolUseComplete → 執行工具               │
│  3. Done("end_turn") → TurnResult::Complete           │
│  4. Done("tool_use") → TurnResult::Continue           │
└──────────┬───────────────────────────────────────────┘
           │
           ▼
Layer 3: Agent Stream (per-agent-output)
┌──────────────────────────────────────────────────────┐
│  ReActAgent::execute_stream()                        │
│                                                      │
│  Stream<Item = Result<ReActAgentOutput, Error>>      │
│                                                      │
│  多回合串流迴圈:                                      │
│  for turn in 0..max_turns {                          │
│    let stream = engine.run_turn_stream(...)           │
│    while let Some(delta) = stream.next() {           │
│      match delta {                                   │
│        TurnDelta::Text(content) →                    │
│          tx.send(ReActAgentOutput {                   │
│            response: content,                        │
│            tool_calls: [],                           │
│            done: false,        ← 中間輸出             │
│          })                                          │
│        TurnDelta::ToolResults(results) →             │
│          accumulated_tool_calls.extend(results)      │
│          tx.send(ReActAgentOutput {                   │
│            tool_calls: accumulated.clone(),           │
│            done: false,                              │
│          })                                          │
│        TurnDelta::Done(Complete(output)) →           │
│          break (final output)                        │
│      }                                               │
│    }                                                 │
│  }                                                   │
│  tx.send(ReActAgentOutput { done: true, ... })       │
└──────────────────────────────────────────────────────┘
```

### 8.2 StreamChunk Enum -- LLM 層級 (完整定義)

```rust
// crates/autoagents-llm/src/chat/mod.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamChunk {
    /// 文字增量
    Text(String),

    /// 推理內容增量 (thinking/reasoning models)
    ReasoningContent(String),

    /// 工具呼叫開始 (含 ID 和名稱)
    ToolUseStart {
        index: usize,    // 在回應中的 content block 索引
        id: String,       // 工具呼叫唯一 ID
        name: String,     // 工具名稱
    },

    /// 工具輸入 JSON 增量 (部分 JSON 字串)
    ToolUseInputDelta {
        index: usize,
        partial_json: String,  // 增量 JSON fragment
    },

    /// 工具呼叫完成 (完整 ToolCall 已組裝)
    ToolUseComplete {
        index: usize,
        tool_call: ToolCall,   // 完整的工具呼叫 (id + name + args)
    },

    /// 串流結束
    Done {
        stop_reason: String,   // "end_turn" | "tool_use" | "max_tokens"
    },

    /// Usage 統計
    Usage(Usage),
}
```

### 8.3 TurnDelta Enum -- Turn Engine 層級

```rust
// crates/autoagents-core/src/agent/executor/turn_engine.rs

pub enum TurnDelta {
    /// 文字增量 (直接透傳給消費者)
    Text(String),

    /// 推理內容 (目前被 ReActAgent 忽略)
    ReasoningContent(String),

    /// 工具執行結果 (已執行完畢)
    ToolResults(Vec<ToolCallResult>),

    /// 回合結束 (Continue 或 Complete)
    Done(TurnResult<TurnEngineOutput>),
}
```

### 8.4 SSE 串流解析器 -- UTF-8 安全處理

AutoAgents 的 SSE parser 有一個重要的 UTF-8 安全處理:

```rust
// crates/autoagents-llm/src/chat/mod.rs

pub(crate) fn create_sse_stream<F>(response: Response, parser: F) -> Pin<Box<dyn Stream<...>>>
where F: Fn(&str) -> Result<Option<String>, LLMError> + Send + 'static
{
    response.bytes_stream()
        .scan((String::default(), Vec::default()), move |(buffer, utf8_buffer), chunk| {
            // UTF-8 安全: bytes 可能在多位元組字元中間被截斷
            utf8_buffer.extend_from_slice(&bytes);
            match String::from_utf8(utf8_buffer.clone()) {
                Ok(text) => {
                    buffer.push_str(&text);
                    utf8_buffer.clear();    // 全部有效 → 清空 buffer
                }
                Err(e) => {
                    let valid_up_to = e.utf8_error().valid_up_to();
                    if valid_up_to > 0 {
                        // 只消費有效的部分,保留不完整的多位元組字元
                        let valid = String::from_utf8_lossy(&utf8_buffer[..valid_up_to]);
                        buffer.push_str(&valid);
                        utf8_buffer.drain(..valid_up_to);
                    }
                    // 不完整的 UTF-8 留在 utf8_buffer 等待下一個 chunk
                }
            }
            // ... 按 "\n\n" 切割 SSE 事件
        })
}
```

**這解決了 clawtex 曾遇到的 UTF-8 截斷 panic 問題** (見 MEMORY.md)。

> **Clawtex 實作建議**:
> 1. clawtex 使用 `edit_message` 漸進式更新 Telegram 訊息,但沒有三層串流抽象
> 2. 應借鑒 TurnDelta 設計: 在 agent_runtime 的多回合迴圈中用 enum 區分中間/最終狀態
> 3. UTF-8 安全的 SSE parser 可以直接移植,取代目前的 `truncate_to_char_boundary` workaround
> 4. StreamChunk 的 ToolUseStart/InputDelta/Complete 三階段設計讓 UI 可以即時顯示工具呼叫進度

---

## 9. Executor Pattern -- ReAct vs Basic

### 9.1 AgentExecutor Trait

```rust
// crates/autoagents-core/src/agent/executor/mod.rs

#[async_trait]
pub trait AgentExecutor: Send + Sync + 'static {
    type Output: Serialize + DeserializeOwned + Clone + Send + Sync + Debug;
    type Error: Error + Send + Sync + 'static;

    fn config(&self) -> ExecutorConfig;

    async fn execute(
        &self, task: &Task, context: Arc<Context>,
    ) -> Result<Self::Output, Self::Error>;

    async fn execute_stream(
        &self, task: &Task, context: Arc<Context>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Self::Output, Self::Error>> + Send>>, Self::Error>;
}

/// 回合結果: 繼續或完成
pub enum TurnResult<T> {
    Continue(Option<T>),  // 有中間結果的繼續
    Complete(T),          // 最終完成
}
```

### 9.2 Wrapper Type Pattern -- Agent 定義與策略分離

```rust
/// ReActAgent<T>: 包裝任何 AgentDeriveT,提供多回合 + tool calling
pub struct ReActAgent<T: AgentDeriveT> {
    inner: Arc<T>,
    max_turns: usize,  // 預設 10
}

// Deref 透明委派: agent.name() → inner.name()
impl<T: AgentDeriveT> Deref for ReActAgent<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target { &self.inner }
}

// AgentDeriveT 委派
impl<T: AgentDeriveT> AgentDeriveT for ReActAgent<T> {
    type Output = <T as AgentDeriveT>::Output;
    fn name(&self) -> &str { self.inner.name() }
    fn description(&self) -> &str { self.inner.description() }
    fn tools(&self) -> Vec<Box<dyn ToolT>> { self.inner.tools() }
    fn output_schema(&self) -> Option<Value> { self.inner.output_schema() }
}

// AgentHooks 委派 (全部 10 個方法)
impl<T: AgentDeriveT + AgentHooks> AgentHooks for ReActAgent<T> {
    async fn on_agent_create(&self) { self.inner.on_agent_create().await }
    async fn on_run_start(&self, task: &Task, ctx: &Context) -> HookOutcome {
        self.inner.on_run_start(task, ctx).await
    }
    // ... 全部 10 個 hook 都委派
}
```

**使用模式**:
```rust
// 同一個 MyAgent 可以用不同 executor 跑
let react = ReActAgent::new(MyAgent {});   // 多回合 + tools
let basic = BasicAgent::new(MyAgent {});   // 單回合, 無 tools

// 甚至可以在執行時切換
let agent = if needs_tools {
    Box::new(ReActAgent::new(MyAgent {})) as Box<dyn AgentExecutor<...>>
} else {
    Box::new(BasicAgent::new(MyAgent {})) as Box<dyn AgentExecutor<...>>
};
```

### 9.3 TurnEngine -- 共用執行引擎

ReAct 和 Basic 共用 `TurnEngine`,差異僅在 config:

```rust
impl TurnEngineConfig {
    pub fn basic(max_turns: usize) -> Self {
        Self {
            max_turns,
            tool_mode: ToolMode::Disabled,      // 不使用工具
            stream_mode: StreamMode::Structured, // 結構化串流
            memory_policy: MemoryPolicy::basic(),
        }
    }

    pub fn react(max_turns: usize) -> Self {
        Self {
            max_turns,
            tool_mode: ToolMode::Enabled,        // 啟用工具
            stream_mode: StreamMode::Tool,        // 工具串流
            memory_policy: MemoryPolicy::react(),
        }
    }
}
```

> **Clawtex 實作建議**:
> 1. clawtex 的 `agent_runtime.rs` 將執行邏輯 (what) 和策略 (how) 耦合在一起
> 2. 應抽出 `ExecutorStrategy` trait,讓 hand phases 可以選擇不同策略
> 3. `TurnResult<T>` enum 比目前的 bool flag 更型別安全 -- 應引入

---

## 10. Guardrails Engine 深度解析

### 10.1 三層策略架構

```rust
// crates/autoagents-guardrails/src/policy.rs

/// 違規處理策略
#[derive(Debug, Clone, Copy, Default)]
pub enum EnforcementPolicy {
    #[default]
    Block,     // 阻擋: 返回 LLMError, 中止請求
    Sanitize,  // 清洗: 修改 input/output, 繼續處理
    Audit,     // 審計: 記錄警告, 不做任何修改, 繼續處理
}

/// 違規嚴重度
pub enum GuardSeverity { Low, Medium, High, Critical }

/// 違規類別
pub enum GuardCategory {
    PromptInjection, Toxicity, PII, Custom(String),
}
```

### 10.2 Guard Trait -- Input/Output 雙向防護

```rust
// crates/autoagents-guardrails/src/guard.rs

/// 請求護欄
#[async_trait]
pub trait InputGuard: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    async fn inspect(
        &self,
        input: &mut GuardedInput,    // 可變引用 -- 允許就地修改
        context: &GuardContext,
    ) -> Result<GuardDecision, GuardError>;
}

/// 回應護欄
#[async_trait]
pub trait OutputGuard: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    async fn inspect(
        &self,
        output: &mut GuardedOutput,  // 可變引用 -- 允許就地修改
        context: &GuardContext,
    ) -> Result<GuardDecision, GuardError>;
}

/// 護欄決策
pub enum GuardDecision {
    Pass,                                    // 通過
    Modify { violation: Option<GuardViolation> },  // 已修改, 繼續
    Reject(GuardViolation),                  // 拒絕, 需要策略處理
}
```

### 10.3 GuardrailsEngine -- Per-Guard Policy Override

```rust
// crates/autoagents-guardrails/src/engine.rs

// 每個 guard 可以覆蓋全域策略
struct InputGuardEntry {
    guard: Arc<dyn InputGuard>,
    policy_override: Option<EnforcementPolicy>,  // None = 使用全域
}

impl GuardrailsEngine {
    pub(crate) async fn evaluate_input(&self, input: &mut GuardedInput, context: &GuardContext)
        -> Result<(), LLMError>
    {
        for entry in &self.input_guards {
            let decision = entry.guard.inspect(input, context).await?;
            self.apply_input_decision(
                decision,
                input,
                entry.guard.name(),
                entry.policy_override.unwrap_or(self.policy),  // per-guard > global
                context,
            )?;
        }
        Ok(())
    }
}
```

### 10.4 內建 Guards

```rust
// PromptInjectionGuard -- 啟發式 prompt injection 偵測
pub struct PromptInjectionGuard {
    patterns: Vec<&'static str>,  // 預設 8 個模式
}
// 預設模式: "ignore previous instructions", "jailbreak",
//           "developer mode", "bypass safety", etc.

// RegexPiiRedactionGuard -- 正則 PII 去識別
// 偵測: email, phone, SSN, credit card, etc.
// 動作: Modify (替換為 [REDACTED])

// ToxicityGuard -- 毒性偵測
// 使用 LLM 評估文本毒性分數
```

### 10.5 Guardrails 作為 LLMLayer

```rust
// crates/autoagents-guardrails/src/layer.rs

impl LLMLayer for GuardrailsLayer {
    fn build(self: Box<Self>, next: Arc<dyn LLMProvider>) -> Arc<dyn LLMProvider> {
        Arc::new(GuardedProvider::new(next, self.engine.clone()))
    }
}

// GuardedProvider 包裝所有 ChatProvider 方法:
// 1. 將 messages → GuardedInput::Chat
// 2. evaluate_input(&mut input) → Block? return Err
// 3. delegate to inner provider
// 4. 將 response → GuardedOutput::Chat
// 5. evaluate_output(&mut output) → Block? return Err
// 6. return response
```

> **Clawtex 實作建議**:
> 1. clawtex 目前只有 `credential_scrubbing` (硬編碼正則)
> 2. 應實作 `GuardLayer` trait,支援 Block/Sanitize/Audit 三種策略
> 3. PromptInjectionGuard 可直接移植,加入到 provider pipeline
> 4. per-guard policy override 很實用 -- 例如 PII 用 Sanitize,injection 用 Block

---

## 11. Memory Systems

### 11.1 MemoryProvider Trait

```rust
// crates/autoagents-core/src/agent/memory/mod.rs

#[async_trait]
pub trait MemoryProvider: Send + Sync {
    async fn remember(&mut self, message: &ChatMessage) -> Result<(), LLMError>;
    async fn recall(&self, query: &str, limit: Option<usize>) -> Result<Vec<ChatMessage>, LLMError>;
    async fn clear(&mut self) -> Result<(), LLMError>;
    fn memory_type(&self) -> MemoryType;
    fn size(&self) -> usize;
    fn is_empty(&self) -> bool;
    fn needs_summary(&self) -> bool;          // 是否需要壓縮
    fn mark_for_summary(&mut self);           // 標記需要壓縮
    fn replace_with_summary(&mut self, summary: String);  // 用摘要替換
    fn get_event_receiver(&self) -> Option<broadcast::Receiver<MessageEvent>>;
    fn remember_with_role(&mut self, message: &ChatMessage, role: String) -> Result<(), LLMError>;
    fn clone_box(&self) -> Box<dyn MemoryProvider>;
    fn preload(&mut self, data: Vec<ChatMessage>) -> bool;
    fn export(&self) -> Vec<ChatMessage>;
}
```

### 11.2 MemoryPolicy -- 策略化記憶管理

```rust
pub struct MemoryPolicy {
    pub recall: bool,             // 執行前是否 recall 歷史
    pub recall_query: RecallQuery,  // 用什麼查詢 recall (Empty | Prompt)
    pub recall_limit: Option<usize>,
    pub store_user: bool,         // 存儲使用者訊息
    pub store_assistant: bool,    // 存儲助手訊息
    pub store_tool_interactions: bool, // 存儲工具互動
}

impl MemoryPolicy {
    pub fn basic() -> Self { /* recall=true, store_user=true, store_assistant=true */ }
    pub fn react() -> Self { /* 同上 + store_tool_interactions=true */ }
}
```

### 11.3 SlidingWindowMemory -- 具體實作深度解析

唯一內建的 MemoryProvider 實作,使用 `VecDeque` 實現高效 FIFO 視窗:

```rust
// crates/autoagents-core/src/agent/memory/sliding_window.rs

/// 溢出時的處理策略
#[derive(Debug, Clone)]
pub enum TrimStrategy {
    Drop,       // 丟棄最舊訊息 (FIFO)
    Summarize,  // 標記需要摘要壓縮
}

#[derive(Debug, Clone)]
pub struct SlidingWindowMemory {
    messages: VecDeque<ChatMessage>,  // O(1) 頭尾操作
    window_size: usize,
    trim_strategy: TrimStrategy,
    needs_summary: bool,
}
```

**關鍵實作細節 -- remember() 的雙策略分流**:

```rust
async fn remember(&mut self, message: &ChatMessage) -> Result<(), LLMError> {
    if self.messages.len() >= self.window_size {
        match self.trim_strategy {
            TrimStrategy::Drop => {
                self.messages.pop_front();     // O(1) 丟棄最舊
            }
            TrimStrategy::Summarize => {
                self.mark_for_summary();       // 不丟棄,標記待壓縮
            }
        }
    }
    self.messages.push_back(message.clone());  // O(1) 追加
    Ok(())
}
```

**Summarize 策略的三步流程**:

```
1. messages.len() >= window_size → mark_for_summary()  (needs_summary = true)
2. 外部系統偵測 needs_summary() == true → 用 LLM 生成摘要
3. replace_with_summary(summary) → clear() + push_back(assistant_summary)
```

```rust
pub fn replace_with_summary(&mut self, summary: String) {
    self.messages.clear();
    self.messages.push_back(
        ChatMessage::assistant().content(summary).build()
    );
    self.needs_summary = false;  // 重置標記
}
```

**recall() 的效率**: 使用 `VecDeque::range()` 實現 O(1) 切片:

```rust
pub fn recent_messages(&self, limit: usize) -> Vec<ChatMessage> {
    let len = self.messages.len();
    let start = len.saturating_sub(limit);  // 不會 underflow
    self.messages.range(start..).cloned().collect()
}
```

**持久化支援 -- preload/export**:

```rust
fn preload(&mut self, data: Vec<ChatMessage>) -> bool {
    self.messages.clear();
    for msg in data { self.messages.push_back(msg); }
    true
}

fn export(&self) -> Vec<ChatMessage> {
    Vec::from(self.messages.clone())
}
```

### 11.4 MemoryAdapter -- 策略化記憶管理層

**MemoryAdapter** 是 Memory 和 Executor 之間的橋接層,根據 **MemoryPolicy** 決定何時存取:

```rust
// crates/autoagents-core/src/agent/executor/memory_policy.rs

pub enum RecallQuery {
    Empty,    // 用空字串 recall (取全部)
    Prompt,   // 用使用者的 prompt 作為 recall 查詢
}

pub struct MemoryPolicy {
    pub recall: bool,               // 是否在執行前 recall
    pub recall_query: RecallQuery,
    pub recall_limit: Option<usize>,
    pub store_user: bool,           // 存儲使用者訊息
    pub store_assistant: bool,      // 存儲助手回覆
    pub store_tool_interactions: bool, // 存儲工具互動
}

pub struct MemoryAdapter {
    memory: Option<Arc<Mutex<Box<dyn MemoryProvider>>>>,
    policy: MemoryPolicy,
}
```

**MemoryAdapter 的核心方法**:

```rust
impl MemoryAdapter {
    pub async fn recall_messages(&self, task: &Task) -> Vec<ChatMessage> {
        if !self.policy.recall { return Vec::new(); }
        let Some(memory) = &self.memory else { return Vec::new(); };
        let query = match self.policy.recall_query {
            RecallQuery::Empty => "",
            RecallQuery::Prompt => task.prompt.as_str(),
        };
        memory.lock().await
            .recall(query, self.policy.recall_limit).await
            .unwrap_or_default()
    }

    pub async fn store_tool_interaction(&self, ...) {
        if !self.policy.store_tool_interactions { return; }
        // 存儲 2 條訊息:
        // 1. Assistant ToolUse (呼叫了什麼工具)
        // 2. Tool ToolResult (工具回傳了什麼)
    }
}
```

**資料流圖 -- MemoryAdapter 在 Executor 中的位置**:

```
Task 進入 Executor
      │
      ▼
MemoryAdapter.recall_messages(task)  ←── policy.recall == true?
      │
      ▼
recall 的訊息 + task.prompt → 組裝 messages[]
      │
      ▼
LLM Provider.chat(messages)
      │
      ▼
MemoryAdapter.store_user(task)       ←── policy.store_user == true?
      │
      ▼
MemoryAdapter.store_assistant(response)  ←── policy.store_assistant == true?
      │
      ▼ (如果有 tool calls)
MemoryAdapter.store_tool_interaction(calls, results)  ←── policy.store_tool_interactions?
```

### 11.5 MemoryHelper -- 低層級靜態工具

`MemoryHelper` 是 `MemoryAdapter` 之前的遺留 API,提供靜態方法:

```rust
// crates/autoagents-core/src/agent/executor/memory_helper.rs

pub struct MemoryHelper;

impl MemoryHelper {
    pub async fn store_tool_interaction(
        memory: &Option<Arc<Mutex<Box<dyn MemoryProvider>>>>,
        tool_calls: &[ToolCall],
        tool_results: &[ToolCallResult],
        response_text: &str,
    ) {
        if let Some(mem) = memory {
            let mut mem = mem.lock().await;
            // 記錄 assistant 呼叫了工具
            let _ = mem.remember(&ChatMessage {
                role: ChatRole::Assistant,
                message_type: MessageType::ToolUse(tool_calls.to_vec()),
                content: response_text.to_string(),
            }).await;
            // 記錄工具回傳結果
            let result_tool_calls = ToolProcessor::create_result_tool_calls(tool_calls, tool_results);
            let _ = mem.remember(&ChatMessage {
                role: ChatRole::Tool,
                message_type: MessageType::ToolResult(result_tool_calls),
                content: String::default(),
            }).await;
        }
    }
}
```

**設計演進**: `MemoryHelper` (靜態) → `MemoryAdapter` (實例化,帶 policy)。後者更優,因為:
1. policy 集中配置,不分散在每個呼叫點
2. `is_enabled()` 一次檢查,不需每次 `if let Some(mem) = memory`
3. 更好的 testability (可 mock MemoryAdapter)

### 11.6 MessageCondition -- 反應式記憶觸發

```rust
pub enum MessageCondition {
    Any,
    Eq(String),
    Contains(String),
    NotContains(String),
    RoleIs(String),
    RoleNot(String),
    LenGt(usize),
    Custom(Arc<dyn Fn(&ChatMessage) -> bool + Send + Sync>),
    Empty,
    All(Vec<MessageCondition>),    // AND 組合
    AnyOf(Vec<MessageCondition>),  // OR 組合
    Regex(String),
}
```

**matches() 的遞迴評估**:

```rust
impl MessageCondition {
    pub fn matches(&self, event: &MessageEvent) -> bool {
        match self {
            // ...基本條件...
            MessageCondition::All(inner) => inner.iter().all(|c| c.matches(event)),
            MessageCondition::AnyOf(inner) => inner.iter().any(|c| c.matches(event)),
            MessageCondition::Regex(regex) => Regex::new(regex)
                .map(|re| re.is_match(&event.msg.content))
                .unwrap_or(false),  // 無效 regex 靜默失敗,不 panic
        }
    }
}
```

**注意**: `Regex::new()` 在每次 `matches()` 時重新編譯。對高頻場景應使用 `lazy_static` 或預編譯。

> **Clawtex 實作建議**:
> 1. clawtex 的 MemoryStore 只有 store/recall/forget,缺少 summary 機制 -- 應引入 `TrimStrategy::Summarize` + `replace_with_summary` 與 `context_optimizer` 整合
> 2. **MemoryAdapter + MemoryPolicy** 模式非常適合 clawtex -- 可以為不同 agent 配置不同的記憶策略 (例如 master agent store_tool_interactions=true,而 quick-reply agent store_tool_interactions=false)
> 3. MessageCondition 的 `Custom(Arc<dyn Fn>)` 模式適合 clawtex 的 event-driven 架構
> 4. clawtex 的 memory.db SQLite 實作應增加 `preload/export` 方法,支持會話持久化和恢復
> 5. `VecDeque` 的 O(1) 頭尾操作比 `Vec` 的 O(n) `remove(0)` 更高效 -- 如果 clawtex 的記憶內部用 Vec,應改為 VecDeque

---

## 12. Multi-Agent Coordination

### 12.1 四種協調模式

```rust
// 1. Topic-based Pub/Sub
let analysis_topic = Topic::<Task>::new("analysis");
runtime.publish(&analysis_topic, Task::new("analyze this")).await?;

// 2. Direct Messaging
runtime.send_message(task, agent_a.addr()).await?;

// 3. Context.publish() -- Agent 內部發布到 Topic
// 在 AgentHooks 或 ToolRuntime 中:
context.publish(summary_topic, Task::new("summarize")).await?;

// 4. Design Patterns (examples/)
// chaining.rs    -- Agent A → Agent B → Agent C
// parallel.rs    -- Agent A + B + C 平行執行
// planning.rs    -- Planner Agent 分配子任務
// reflection.rs  -- Agent 自我反省循環
// routing.rs     -- Router Agent 分發到專家
```

### 12.2 Runtime 與 Environment

```rust
/// Runtime: 管理單一 actor 系統
pub trait Runtime: Send + Sync {
    fn id(&self) -> RuntimeID;
    async fn subscribe_any(&self, topic_name: &str, topic_type: TypeId, actor: Arc<dyn AnyActor>);
    async fn publish_any(&self, topic_name: &str, topic_type: TypeId, message: Arc<dyn Any>);
    fn tx(&self) -> mpsc::Sender<Event>;
    async fn run(&self) -> Result<()>;
    async fn stop(&self) -> Result<()>;
}

/// TypedRuntime: 自動實作的型別安全便利層
pub trait TypedRuntime: Runtime {
    async fn subscribe<M>(&self, topic: &Topic<M>, actor: ActorRef<M>);
    async fn publish<M>(&self, topic: &Topic<M>, message: M);
    async fn send_message<M>(&self, message: M, addr: ActorRef<M>);
}

// 自動實作: 所有 Runtime 都自動獲得 TypedRuntime
impl<T: Runtime + ?Sized> TypedRuntime for T {}
```

### 12.3 五種設計模式 -- 實際程式碼解析

AutoAgents 在 `examples/design_patterns/` 中提供了五種完整可執行的多代理協調模式:

#### 12.3.1 Chaining (鏈式串接)

```
Input → Agent1 (萃取) → Agent2 (轉換) → Output
```

**核心機制**: `on_run_complete` hook 內用 `ctx.publish()` 自動轉發結果到下一個 agent:

```rust
// examples/design_patterns/src/chaining.rs

#[agent(name = "agent_1",
    description = "Extract the technical specifications from the given text")]
pub struct Agent1 {}

#[async_trait]
impl AgentHooks for Agent1 {
    async fn on_run_complete(&self, _task: &Task, result: &Self::Output, ctx: &Context) {
        // Agent1 完成後,結果自動發送到 Agent2
        let _ = ctx.publish(
            Topic::<Task>::new("agent_2"),
            Task::new(result)
        ).await;
    }
}
```

**構建方式**: 每個 agent 訂閱自己的 topic,形成隱式鏈:

```rust
let _ = AgentBuilder::<_, ActorAgent>::new(agent1)
    .llm(llm.clone())
    .runtime(runtime.clone())
    .subscribe(topic1.clone())      // Agent1 訂閱 topic1
    .memory(sliding_window_memory.clone())
    .build().await?;

// 啟動鏈: 發布到第一個 agent 的 topic
runtime.publish(&topic1, Task::new("...")).await?;
```

#### 12.3.2 Parallel (平行扇出/扇入)

```
             Input Topic
                 │
        ╔════════╬════════╗
        ║        ║        ║
        ▼        ▼        ▼
    Summary  Questions  Terms    ← 三個 agent 平行執行
        ║        ║        ║
        ╚════════╬════════╝
                 │
           Synthesis Agent       ← 彙整所有結果
                 │
            Final Output
```

**關鍵**: 用 Event handler 收集平行結果,全部完成後觸發合成:

```rust
pub fn handle_events(
    mut event_stream: BoxEventStream<Event>,
    submission_id: SubmissionId,
    runtime: Arc<SingleThreadedRuntime>,
) {
    tokio::spawn(async move {
        let mut results: HashMap<String, String> = HashMap::new();
        let expected_keys = ["summarize", "questions", "key_terms"];

        while let Some(event) = event_stream.next().await {
            if let Event::TaskComplete { result, sub_id, actor_name, .. } = event {
                if sub_id == submission_id {
                    results.insert(actor_name.clone(), result.clone());
                }
                // 所有平行 agent 都完成時,觸發合成
                if expected_keys.iter().all(|k| results.contains_key(*k)) {
                    let payload = json!({
                        "summary": results.get("summarize"),
                        "questions": results.get("questions"),
                        "key_terms": results.get("terms"),
                    });
                    let _ = runtime.publish(
                        &Topic::<Task>::new("synthesis"),
                        Task::new(payload.to_string())
                    ).await;
                    results.clear();
                }
            }
        }
    });
}
```

**注意**: `#[derive(AgentHooks)]` 可用於不需要自訂 hooks 的 agent (如 SummarizeAgent):

```rust
#[agent(name = "summarize",
    description = "Summarize the following topic concisely.")]
#[derive(AgentHooks)]  // 自動生成空的 hooks impl
pub struct SummarizeAgent {}
```

#### 12.3.3 Routing (路由分發)

```
    User Request
         │
    Routing Agent
    (LLM 分類)
         │
    ╔════╬════╗
    ║    ║    ║
 Booking Info Unclear
 Handler Handler Handler
```

**使用 DirectAgent 而非 ActorAgent** -- 路由是同步操作,不需要 actor 開銷:

```rust
// 使用 DirectAgent 進行同步路由
let agent_handle = AgentBuilder::<_, DirectAgent>::new(agent)
    .llm(llm.clone())
    .memory(sliding_window_memory.clone())
    .build().await?;

// 同步呼叫,不經過 actor 系統
let result = agent_handle.agent.run(Task::new(task.clone())).await?;

// 根據 LLM 的分類結果路由
fn handle_routing(mode: String, request: String) -> String {
    match mode.as_ref() {
        "booker" => booking_handler(request),
        "info" => info_handler(request),
        "unclear" => unclear_handler(request),
        _ => String::from("Unknown routing mode"),
    }
}
```

#### 12.3.4 Reflection (反思迴圈)

```
        ╔══════════════════════════════════╗
        ║                                  ║
        ▼                                  ║
  CodeGenerator ──(code)──> CodeCritic    ║
        ▲                      │           ║
        ║                      ▼           ║
        ║          CODE_IS_PERFECT? ───yes──╝ → 結束
        ║                      │
        ║                     no
        ║                      │
        ╚────(critique)────────╝
              (最多 N 次迭代)
```

**迭代計數器**: 使用 `AtomicUsize` 跨 async 安全追蹤迭代次數:

```rust
pub struct CodeCritic {
    max_iterations: usize,
    current_iteration: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl AgentHooks for CodeCritic {
    async fn on_run_complete(&self, task: &Task, result: &Self::Output, ctx: &Context) {
        let iteration = self.current_iteration
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;

        if result.contains("CODE_IS_PERFECT") {
            // 完成! 環境自然結束
        } else if iteration < self.max_iterations {
            // 將 critique 回傳給 generator 進行改進
            let _ = ctx.publish(
                Topic::<Task>::new("code_generator"),
                Task::new(refinement_task),
            ).await;
        } else {
            // 達到最大迭代次數,強制結束
        }
    }
}
```

#### 12.3.5 Planning (策略規劃)

最複雜的模式 -- **自適應多步驟計劃與執行**:

```
Complex Task
     │
Strategic Planner
(生成 GOAL + STEPS + EXPECTED_OUTPUTS + SUCCESS_CRITERIA)
     │
Plan Created
     │
╔═══════════════╗
║ Plan Executor ║ ← 逐步執行
║ (step by step)║ ← 回報進度
╚═══════════════╝ ← 可請求修訂
     │
[SUCCESS]──→ 下一步 / 完成
[PARTIAL]──→ 修訂計劃
[BLOCKED]──→ 重新規劃
```

**自適應機制**: Executor 根據執行結果決定下一步:

```rust
#[async_trait]
impl AgentHooks for PlanExecutor {
    async fn on_run_complete(&self, task: &Task, result: &Self::Output, ctx: &Context) {
        let status = extract_status_from_result(result);
        match status.as_str() {
            "SUCCESS" => {
                if step_num < all_steps.len() {
                    // 繼續下一步
                    ctx.publish(Topic::<Task>::new("plan_executor"), next_step_task).await;
                } else {
                    // 全部完成
                }
            }
            "PARTIAL" | "BLOCKED" => {
                // 回到 Planner 重新規劃
                ctx.publish(
                    Topic::<Task>::new("strategic_planner"),
                    Task::new(revision_task),
                ).await;
            }
        }
    }
}
```

### 12.4 EventFanout -- 多消費者事件分發

```rust
// crates/autoagents-core/src/event_fanout.rs

pub(crate) struct EventFanout {
    tx: broadcast::Sender<Event>,
    _task: JoinHandle<()>,  // 背景任務: mpsc → broadcast 轉發
}

impl EventFanout {
    pub(crate) fn new(mut event_stream: BoxEventStream<Event>, buffer: usize) -> Self {
        let (tx, _) = broadcast::channel(buffer);
        let tx_clone = tx.clone();
        let task = tokio::spawn(async move {
            while let Some(event) = event_stream.next().await {
                let _ = tx_clone.send(event);
            }
        });
        Self { tx, _task: task }
    }

    pub(crate) fn subscribe(&self) -> BoxEventStream<Event> {
        let rx = self.tx.subscribe();
        let stream = BroadcastStream::new(rx)
            .filter_map(|item| async move { item.ok() });
        Box::pin(stream)
    }
}
```

### 12.5 EventHelper -- 事件發射便利層

```rust
// crates/autoagents-core/src/agent/executor/event_helper.rs

pub struct EventHelper;

impl EventHelper {
    pub async fn send(tx: &Option<mpsc::Sender<Event>>, event: Event) {
        if let Some(tx) = tx {
            let _ = tx.send(event).await;
        }
    }

    // 任務生命週期事件
    pub async fn send_task_started(tx, sub_id, actor_id, actor_name, task_description);
    pub async fn send_task_completed(tx, sub_id, actor_id, actor_name, result);
    pub async fn send_task_error(tx, sub_id, actor_id, error);

    // Turn 生命週期事件
    pub async fn send_turn_started(tx, sub_id, actor_id, turn_number, max_turns);
    pub async fn send_turn_completed(tx, sub_id, actor_id, turn_number, final_turn);

    // 串流事件
    pub async fn send_stream_chunk(tx, sub_id, chunk: LlmStreamChunk) {
        let chunk: StreamChunk = chunk.into();  // LLM 層 → Protocol 層轉換
        Self::send(tx, Event::StreamChunk { sub_id, chunk }).await;
    }
    pub async fn send_stream_tool_call(tx, sub_id, tool_call: Value);
    pub async fn send_stream_complete(tx, sub_id);
}
```

**Event 型別的完整生命週期**:

```
TaskStarted → TurnStarted → StreamChunk* → StreamToolCall* → TurnCompleted → TaskComplete
                    ↑                                               │
                    └──────────── (多回合循環) ─────────────────────┘
TaskStarted → TaskError  (如果執行失敗)
```

### 12.6 五種模式的對比矩陣

| 模式 | Agent 數量 | 通訊方式 | Runtime | 複雜度 | clawtex 對應 |
|------|-----------|---------|---------|-------|-------------|
| **Chaining** | 2+ | Topic pub/sub (隱式鏈) | ActorAgent | 低 | hands 的 phase 串接 |
| **Parallel** | 3+ (含 synthesis) | Event handler 收集 | ActorAgent | 中 | delegate 多個 agent |
| **Routing** | 1 router + N handlers | 同步 match | **DirectAgent** | 低 | classifier → provider |
| **Reflection** | 2 (generator + critic) | Topic 乒乓 | ActorAgent | 中 | self_evolve hand |
| **Planning** | 2 (planner + executor) | Topic + 自適應 | ActorAgent | 高 | 無直接對應 |

> **Clawtex 實作建議**:
> 1. clawtex 的 `delegate` tool 是字串級的 agent 委派,缺少型別安全
> 2. **Parallel 模式**的 Event handler 收集結果模式可直接用於 clawtex cluster_hub -- 多個 worker 平行處理,hub 收集結果後合成
> 3. **Routing 模式**的 DirectAgent (無 actor 開銷) 啟示: clawtex 的 classifier 不需要完整的 agent_runtime,可以用輕量同步呼叫
> 4. **Reflection 模式**的 `AtomicUsize` 迭代計數器 + `CODE_IS_PERFECT` 終止條件是 self_evolve hand 的完美參考
> 5. **Planning 模式**的三態 (SUCCESS/PARTIAL/BLOCKED) 自適應是 clawtex hands 目前缺少的 -- hands 只有 success/fail,缺少 "部分完成→修訂計劃" 路徑
> 6. EventFanout 的 mpsc→broadcast 模式可以用在 cluster_hub 的事件分發
> 7. EventHelper 的事件型別層級 (Task→Turn→Stream) 比 clawtex 的扁平 AgentEvent 更豐富 -- 可用於前端 UI 顯示執行進度

---

## 13. Structured Output 型別安全

### 13.1 AgentOutputT Trait

```rust
pub trait AgentOutputT: Serialize + DeserializeOwned + Send + Sync {
    fn output_schema() -> &'static str;          // 編譯時產生的 JSON Schema
    fn structured_output_format() -> Value;      // 給 LLM 的格式指令
}

// String 的預設實作 (無 schema constraint)
impl AgentOutputT for String {
    fn output_schema() -> &'static str { "{}" }
    fn structured_output_format() -> Value { Value::Null }
}
```

### 13.2 安全的輸出解析

```rust
impl ReActAgentOutput {
    /// 嘗試將 response 解析為結構化型別
    pub fn try_parse<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str::<T>(&self.response)
    }

    /// 解析或 fallback: 如果 JSON 解析失敗, 用 fallback 函數轉換原始文字
    pub fn parse_or_map<T, F>(&self, fallback: F) -> T
    where T: DeserializeOwned, F: FnOnce(&str) -> T
    {
        self.try_parse::<T>().unwrap_or_else(|_| fallback(&self.response))
    }
}
```

> **Clawtex 實作建議**:
> 1. clawtex 目前用 `serde_json::Value` 手動解析 LLM 輸出
> 2. `parse_or_map` 模式非常實用 -- LLM 不一定回傳合法 JSON

---

## 14. 錯誤處理架構

### 14.1 錯誤型別層級

```
LLMError (autoagents-llm)
├── HttpError(String)             -- 網路錯誤
├── ProviderError(String)         -- Provider 回傳的錯誤
├── Generic(String)               -- 通用錯誤
├── AuthError(String)             -- 認證失敗 (不可重試/不可降級)
├── InvalidRequest(String)        -- 無效請求 (不可重試/不可降級)
├── JsonError(String)             -- JSON 解析錯誤 (不可重試)
├── ToolConfigError(String)       -- 工具配置錯誤 (不可重試)
├── NoToolSupport(String)         -- Provider 不支援工具 (可降級!)
├── ResponseFormatError { .. }    -- 回應格式錯誤 (可降級)
├── GuardrailBlocked { .. }       -- 護欄阻擋 (不可重試)
└── GuardrailExecutionFailed { .. } -- 護欄執行失敗

TurnEngineError (autoagents-core)
├── LLMError(LLMError)           -- 來自 LLM 層
├── Aborted                      -- Hook 中止
└── Other(String)                -- 其他

ReActExecutorError (autoagents-core)
├── LLMError(LLMError)
├── MaxTurnsExceeded { max_turns }
├── Other(String)
├── EventError(SendError)
└── AgentOutputError(String)

RunnableAgentError (autoagents-core)
├── Abort                        -- on_run_start 返回 Abort
├── ExecutorError(String)
└── StreamError(String)

WasmRuntimeError (autoagents-core)
├── ModuleLoad(String)
├── Instantiation(String)
├── MemoryAccess(String)
├── JsonError(String)
├── Utf8Error(String)
├── FunctionError(String)
└── MissingSourceFile
```

### 14.2 錯誤分類與處理策略

| 錯誤型別 | 可重試 | 可降級 | 處理策略 |
|---------|--------|--------|---------|
| HttpError (429/5xx) | 是 | 是 | RetryLayer + FallbackLayer |
| AuthError | 否 | 否 | 直接返回,使用者需修正 API key |
| InvalidRequest | 否 | 否 | 直接返回,使用者需修正請求 |
| NoToolSupport | 否 | **是** | FallbackLayer 切到支援工具的 Provider |
| GuardrailBlocked | 否 | 否 | 直接返回,內容被阻擋 |
| Aborted (Hook) | N/A | N/A | Hook 主動中止,非錯誤 |
| MaxTurnsExceeded | N/A | N/A | 返回已累積的部分結果 |

> **Clawtex 實作建議**:
> 1. clawtex 的 `ProviderError` 較為扁平,應細分為 Auth/RateLimit/ServerError
> 2. `NoToolSupport` 作為可降級錯誤的設計很重要 -- 讓本地模型自動 fallback 到雲端

---

## 15. 效能特徵分析

### 15.1 Hot Path 效能 (首次成功)

| 元件 | Hot Path 開銷 |
|------|--------------|
| PipelineBuilder | 零 -- build 時一次性組裝,之後直接呼叫 |
| CacheLayer (hit) | 1x RwLock read + hash computation |
| CacheLayer (miss) | 1x RwLock read + inner call + 1x RwLock write |
| RetryLayer (success) | 1x match arm, 零分配 |
| FallbackLayer (primary success) | 1x Arc::clone + 1x match arm |
| GuardrailsLayer (pass) | N x guard.inspect() (同步, 低延遲) |
| Topic publish | N x Arc::clone + N x downcast_ref |
| SharedMessage clone | 1x atomic increment (O(1)) |

### 15.2 記憶體使用

| 元件 | 記憶體特徵 |
|------|-----------|
| SharedMessage | 所有訂閱者共享同一 Arc,不複製資料 |
| CacheLayer | HashMap + RwLock,可配置 max_size |
| TeeStream | 串流過程中累積 buffer,結束後寫入 cache |
| WasmRuntime | 每次 run() 建立新 Store,執行後釋放 |
| ToolProcessor | 順序執行 tool calls,不平行 |

### 15.3 並發特徵

| 元件 | 並發模型 |
|------|---------|
| Actor System | Ractor 管理 actor 排程,每個 actor 串行處理訊息 |
| CacheLayer | RwLock (不跨 await) + per-key async mutex (single-flight) |
| EventFanout | broadcast channel,多消費者無鎖讀取 |
| ToolProcessor | **順序**執行 tool calls (非平行) -- 潛在瓶頸 |

### 15.4 WASM 條件編譯策略

整個 codebase 大量使用 `#[cfg(not(target_arch = "wasm32"))]` 實現雙平台支援:

```rust
// WASM 環境下的替代
#[cfg(target_arch = "wasm32")]
use futures::lock::Mutex;           // 替代 tokio::sync::Mutex
#[cfg(target_arch = "wasm32")]
use futures::channel::mpsc;         // 替代 tokio::sync::mpsc
#[cfg(target_arch = "wasm32")]
use futures::SinkExt;               // mpsc send 需要 SinkExt
```

**條件編譯影響矩陣**:

| 模組 | Native (tokio) | WASM (futures) | 功能差異 |
|------|---------------|----------------|---------|
| Mutex | `tokio::sync::Mutex` | `futures::lock::Mutex` | 無 -- API 相容 |
| mpsc | `tokio::sync::mpsc` | `futures::channel::mpsc` | WASM 需要 `SinkExt` for `.send()` |
| broadcast | `tokio::sync::broadcast` | **不支援** | WASM 無 Event 系統 |
| EventHelper | `tx.send(event).await` | `// TODO: WASM 不支援` | WASM 靜默忽略事件 |
| sleep | `tokio::time::sleep` | 不可用 | 影響 RetryLayer backoff |

**EventHelper 在 WASM 的降級**:

```rust
pub async fn send(tx: &Option<mpsc::Sender<Event>>, event: Event) {
    if let Some(tx) = tx {
        #[cfg(not(target_arch = "wasm32"))]
        //TODO: WASM Targets currently does not support event handling
        let _ = tx.send(event).await;
    }
}
```

### 15.5 鎖策略深度分析

**CacheLayer 的鎖優化**: 避免持有 RwLock 跨 await 點

```
正確 (AutoAgents 做法):           錯誤 (常見錯誤):

let cached = {                    let guard = cache.read().await;
    cache.read().await.get(key)   if let Some(v) = guard.get(key) {
};                                    return v;  // guard 跨 await!
// guard 已釋放                   }
if cached.is_none() {             let result = inner.call().await;
    let result = inner.call()     guard.write().await.insert(key, result);
        .await;                   // 死鎖風險!
    cache.write().await
        .insert(key, result);
}
```

**MemoryAdapter 的鎖粒度**: 每個操作獨立獲取鎖

```rust
// 好: 細粒度鎖
pub async fn recall_messages(&self, task: &Task) -> Vec<ChatMessage> {
    memory.lock().await                    // 鎖 1: recall
        .recall(query, limit).await
        .unwrap_or_default()               // 鎖在 unwrap_or_default 前已釋放
}

pub async fn store_assistant(&self, response: &str) {
    let _ = memory.lock().await            // 鎖 2: 獨立於 recall
        .remember(&message).await;
}
```

### 15.6 效能瓶頸識別

| 瓶頸 | 位置 | 影響 | 改善建議 |
|------|------|------|---------|
| ToolProcessor 順序執行 | `tool_processor.rs` | N 個 tool call = N 倍延遲 | 可平行化獨立 tool calls |
| Regex 每次重新編譯 | `MessageCondition::Regex` | matches() 頻繁呼叫時 | 預編譯 + 快取 |
| ChatMessage clone | `SlidingWindowMemory` | 大量訊息時記憶體壓力 | 改用 `Arc<ChatMessage>` |
| VecDeque clone in export() | `sliding_window.rs` | export 時完整複製 | 改用 `Arc` 共享 |
| HashMap in Parallel | `handle_events()` | 每次 insert 可能 rehash | 預分配 capacity |

> **Clawtex 實作建議**:
> 1. clawtex 的 `tool_calls` 也是順序執行 -- 可參考 AutoAgents 的 ToolProcessor 但加入平行化選項
> 2. 鎖策略很重要 -- clawtex 的 provider cache 應確保不持有鎖跨 await
> 3. WASM 條件編譯模式對 clawtex 不重要 (daemon 不需要跑在瀏覽器),但**鎖優化模式**是通用的
> 4. 如果 clawtex 的 context_optimizer 用 Regex 偵測 pattern,應預編譯並快取

---

## 16. 與 clawtex-core 完整差距對比

### 16.1 架構對比矩陣

| 面向 | AutoAgents | clawtex-core | 差距評估 |
|------|-----------|--------------|---------|
| **Crate 結構** | 12 crate workspace | 單 crate 單體 | Medium -- 可後期拆分 |
| **並發模型** | Ractor Actor Model | tokio tasks + channels | Low -- tokio 已足夠 |
| **Agent 定義** | proc-macro (#[agent]) | TOML config (agents.toml) | 不同範式 -- clawtex 更靈活 |
| **Executor 策略** | ReAct/Basic 可插拔 | 內嵌在 agent_runtime | **High -- 應抽象** |
| **Tool 定義** | trait + proc-macro | trait + 手動 register | Medium |
| **LLM 抽象** | LLMProvider 組合 trait + Pipeline | Provider trait + Router | **High -- 應引入 Pipeline** |
| **重試** | RetryLayer (指數退避+jitter) | ReliableProvider (hardcoded) | **High -- 應重構** |
| **降級** | FallbackLayer (多 provider) | rotation provider (round-robin) | **High -- 語意不同** |
| **快取** | CacheLayer (LRU+TTL+TeeStream) | provider_cache (基本) | Medium |
| **護欄** | GuardrailsLayer (3策略+多guard) | credential_scrubbing | **High -- 應擴展** |
| **Memory** | MemoryProvider trait + Summary | MemoryStore (SQLite) | Medium |
| **Streaming** | 3-layer (StreamChunk→TurnDelta→Agent) | edit_message progressive | **High -- 應重構** |
| **Hook 系統** | 10 lifecycle hooks | 無 | **Critical -- 應引入** |
| **WASM 沙箱** | wasmtime capability-based | shell allowlist | Medium |
| **結構化輸出** | AgentOutputT + derive | serde_json::Value 手動 | Low -- 需求不同 |
| **型別安全事件** | Topic<M> compile-time | AgentEvent enum (untyped) | Medium |
| **測試** | 每個模組都有單元測試 | 709+ 整合測試 | 同等 |

### 16.2 優先移植清單

#### P0 (Critical -- 應立即引入)

**1. LLMLayer Pipeline**

目前 clawtex 的 `ReliableProvider` 將 circuit breaker、retry、cache 硬編碼在一起。應改為:

```rust
// 建議的 clawtex 實作
pub trait ProviderLayer: Send + Sync + 'static {
    fn build(self: Box<Self>, next: Arc<dyn Provider>) -> Arc<dyn Provider>;
}

pub struct ProviderPipeline {
    base: Arc<dyn Provider>,
    layers: Vec<Box<dyn ProviderLayer>>,
}

impl ProviderPipeline {
    pub fn build(self) -> Arc<dyn Provider> {
        let mut provider = self.base;
        for layer in self.layers.into_iter().rev() {
            provider = layer.build(provider);
        }
        provider
    }
}

// 使用:
let provider = ProviderPipeline::new(ollama)
    .add_layer(RetryLayer::new(RetryConfig { max_attempts: 3, .. }))
    .add_layer(FallbackLayer::new(vec![anthropic, groq]))
    .add_layer(CacheLayer::new(CacheConfig { ttl: 3600, .. }))
    .build();
```

**2. AgentHooks (至少 on_tool_call + on_run_start)**

```rust
// 建議的 clawtex 實作
#[async_trait]
pub trait AgentHook: Send + Sync {
    async fn on_run_start(&self, agent: &str, prompt: &str) -> HookOutcome {
        HookOutcome::Continue
    }
    async fn on_tool_call(&self, tool_name: &str, args: &Value) -> HookOutcome {
        HookOutcome::Continue
    }
    async fn on_tool_result(&self, tool_name: &str, result: &Value) {}
}

// 在 agent_runtime.rs 的 tool 執行前:
for hook in &self.hooks {
    if hook.on_tool_call(tool_name, args).await == HookOutcome::Abort {
        return Ok(ToolResult::Skipped);
    }
}
```

#### P1 (High -- 下一迭代)

**3. Guardrails 三策略架構**

```rust
// 建議的 clawtex 實作
pub enum GuardPolicy { Block, Sanitize, Audit }

pub trait InputGuard: Send + Sync {
    fn name(&self) -> &str;
    fn check(&self, messages: &[Message]) -> GuardDecision;
}

// 內建: PromptInjectionGuard, CredentialGuard (現有 credential_scrubbing)
```

**4. Three-Layer Streaming**

定義 `TurnDelta` enum 替代目前的 string 累積:

```rust
pub enum TurnDelta {
    Text(String),
    ToolStarted { name: String },
    ToolCompleted { name: String, result: Value },
    Done(AgentOutput),
}
```

#### P2 (Medium -- 長期改進)

**5. Typed Event Channels**

**6. Executor Strategy 抽象**

**7. MemoryPolicy -- Summary 機制**

### 16.3 不需要引入的部分

| 功能 | 原因 |
|------|------|
| Ractor 依賴 | clawtex 的 tokio task 模型已足夠,actor 框架太重 |
| PhantomData Agent 分型 | clawtex 只有一種 agent 模式 (Telegram 驅動) |
| WASM 支援 | clawtex 是 daemon,不需要跑在瀏覽器 |
| proc-macro (#[agent], #[tool]) | clawtex 使用 TOML 配置,更適合動態系統 |
| AgentOutputT derive | clawtex 的 hands 流程不需要編譯時 schema |
| PyO3 Python 綁定 | clawtex 專注 Rust,Python 整合透過 MCP |

---

## 附錄: 關鍵檔案路徑索引

### Core (autoagents-core)

| 檔案 | 用途 | 行數概估 |
|------|------|---------|
| `src/actor/mod.rs` | AnyActor trait + impl | 250 |
| `src/actor/messaging.rs` | ActorMessage, CloneableMessage, SharedMessage | 197 |
| `src/actor/topic.rs` | Topic<M> 型別安全 pub/sub | 205 |
| `src/actor/subscriber.rs` | TypedSubscriber, SharedSubscriber | 206 |
| `src/actor/transport.rs` | Transport trait, LocalTransport | 259 |
| `src/agent/base.rs` | BaseAgent<T, A> 核心結構 | ~300 |
| `src/agent/builder.rs` | AgentBuilder | ~200 |
| `src/agent/actor.rs` | AgentActor implements ractor::Actor | ~200 |
| `src/agent/direct.rs` | DirectAgent (無 actor) | ~150 |
| `src/agent/hooks.rs` | AgentHooks trait (10 hooks) | 52 |
| `src/agent/context.rs` | Context (執行時上下文) | 192 |
| `src/agent/output.rs` | AgentOutputT trait | ~80 |
| `src/agent/executor/mod.rs` | AgentExecutor trait, TurnResult | ~60 |
| `src/agent/executor/turn_engine.rs` | TurnEngine 共用執行引擎 | ~900 |
| `src/agent/executor/tool_processor.rs` | ToolProcessor 集中式工具處理 | 494 |
| `src/agent/executor/memory_policy.rs` | MemoryPolicy, MemoryAdapter | ~200 |
| `src/agent/prebuilt/executor/react.rs` | ReActAgent wrapper + executor | 917 |
| `src/agent/prebuilt/executor/basic.rs` | BasicAgent wrapper + executor | ~400 |
| `src/agent/executor/memory_helper.rs` | MemoryHelper 靜態工具 | 267 |
| `src/agent/executor/event_helper.rs` | EventHelper 事件發射 | 192 |
| `src/agent/memory/mod.rs` | MemoryProvider trait, MessageCondition | 906 |
| `src/agent/memory/sliding_window.rs` | SlidingWindowMemory + TrimStrategy | 459 |
| `src/tool/mod.rs` | ToolT + ToolRuntime traits | ~150 |
| `src/tool/runtime/wasm.rs` | WasmRuntime (wasmtime 沙箱) | 228 |
| `src/runtime/mod.rs` | Runtime + TypedRuntime traits | 225 |
| `src/environment.rs` | Environment (多 Runtime 管理) | ~150 |
| `src/event_fanout.rs` | EventFanout (mpsc→broadcast) | 65 |

### LLM (autoagents-llm)

| 檔案 | 用途 | 行數概估 |
|------|------|---------|
| `src/lib.rs` | LLMProvider super trait | ~100 |
| `src/pipeline/mod.rs` | LLMLayer trait, PipelineBuilder | 350 |
| `src/optim/cache.rs` | CacheLayer + TeeStream | ~1200 |
| `src/optim/retry.rs` | RetryLayer + 指數退避 | 804 |
| `src/optim/fallback.rs` | FallbackLayer + 多 provider 降級 | 821 |
| `src/chat/mod.rs` | ChatProvider, StreamChunk, SSE parser | 1008 |
| `src/backends/openai.rs` | OpenAI backend | ~500 |
| `src/backends/anthropic.rs` | Anthropic backend | ~500 |

### Derive (autoagents-derive)

| 檔案 | 用途 | 行數概估 |
|------|------|---------|
| `src/lib.rs` | 5 proc-macro 入口 | 47 |
| `src/agent/mod.rs` | #[agent] macro | 237 |
| `src/agent/output.rs` | AgentOutput derive | ~150 |
| `src/tool/mod.rs` | #[tool] macro | 52 |
| `src/tool/input.rs` | ToolInput derive + InputParser | 296 |
| `src/tool/field.rs` | Field schema attributes | 109 |
| `src/tool/json.rs` | JSON type enum | 17 |
| `src/tool/attr.rs` | Tool attribute parser | 134 |

### Guardrails (autoagents-guardrails)

| 檔案 | 用途 | 行數概估 |
|------|------|---------|
| `src/engine.rs` | GuardrailsEngine + Builder | 655 |
| `src/guard.rs` | InputGuard/OutputGuard traits + types | 281 |
| `src/layer.rs` | GuardrailsLayer (LLMLayer impl) | 24 |
| `src/guards/prompt_injection.rs` | PromptInjectionGuard | 130 |
| `src/guards/regex_pii_redaction.rs` | PII 去識別 | ~150 |
| `src/guards/toxicity.rs` | 毒性偵測 | ~100 |

### Examples (design patterns)

| 檔案 | 用途 | 行數 |
|------|------|------|
| `examples/design_patterns/src/chaining.rs` | 鏈式串接模式 | 134 |
| `examples/design_patterns/src/parallel.rs` | 平行扇出/扇入模式 | 260 |
| `examples/design_patterns/src/routing.rs` | LLM 路由分發模式 | 149 |
| `examples/design_patterns/src/reflection.rs` | 反思迴圈模式 | 263 |
| `examples/design_patterns/src/planning.rs` | 策略規劃自適應模式 | 399 |

所有路徑相對於 `LLM-Cluster-Project/references/autoagents/`。
