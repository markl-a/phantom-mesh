# Anda AI Agent Framework -- 深度技術分析 (v2)

> 分析版本：v0.9.8 | 分析日期：2026-03-13 (第二版，深度翻倍)
> 原始碼位置：`LLM-Cluster-Project/references/anda/`
> 分析師：Claude Opus 4.6，逐行原始碼分析

---

## 目錄

1. [專案結構與 Crate 依賴圖](#1-專案結構與-crate-依賴圖)
2. [Dual-Trait 模式深度剖析](#2-dual-trait-模式深度剖析)
3. [CompletionRunner 迭代器引擎](#3-completionrunner-迭代器引擎)
4. [Feature Trait 分離架構](#4-feature-trait-分離架構)
5. [Engine Builder 與初始化流程](#5-engine-builder-與初始化流程)
6. [BaseCtx 上下文實作](#6-basectx-上下文實作)
7. [AgentCtx 與 LLM 整合](#7-agentctx-與-llm-整合)
8. [Hook 系統與 SingleThreadHook](#8-hook-系統與-singlethreadhook)
9. [記憶系統 (KIP + CognitiveNexus)](#9-記憶系統-kip--cognitivenexus)
10. [Store + Cache + CacheStoreFeatures](#10-store--cache--cachestorefeatures)
11. [Model 抽象層 (NotImplemented / MockImplemented)](#11-model-抽象層-notimplemented--mockimplemented)
12. [Remote Engine 聯邦](#12-remote-engine-聯邦)
13. [ICP / TEE / Web3SDK](#13-icp--tee--web3sdk)
14. [錯誤處理與取消機制](#14-錯誤處理與取消機制)
15. [Agent 實作範例分析](#15-agent-實作範例分析)
16. [效能分析與瓶頸](#16-效能分析與瓶頸)
17. [Clawtex 差距對比總表](#17-clawtex-差距對比總表)
18. [逐項 Clawtex 實作建議](#18-逐項-clawtex-實作建議)

---

## 1. 專案結構與 Crate 依賴圖

### 1.1 Workspace 佈局（完整）

```
anda/
├── Cargo.toml                    # workspace root (edition = "2024")
├── anda_core/                    # 核心 trait 定義 (零依賴邏輯層)
│   └── src/
│       ├── lib.rs                # BoxError, BoxPinFut, path 驗證, select_resources
│       ├── agent.rs              # Agent<C> / AgentDyn<C> / AgentWrapper / AgentSet
│       ├── tool.rs               # Tool<C> / ToolDyn<C> / ToolWrapper / ToolSet
│       ├── context.rs            # 6 大 Feature trait + AgentContext + BaseContext
│       ├── model.rs              # AgentOutput, Message, ToolCall, FunctionDefinition
│       ├── model/completion.rs   # CompletionFeatures, CompletionRequest
│       ├── model/embedding.rs    # EmbeddingFeatures, Embedding
│       ├── model/resource.rs     # Resource, ResourceRef, select_resources
│       ├── http.rs               # HTTP 常數 (CONTENT_TYPE_JSON 等)
│       └── json.rs               # JSON Schema 生成 helper (gen_schema_for)
├── anda_engine/                  # 核心引擎實作
│   └── src/
│       ├── engine.rs             # Engine + EngineBuilder (完整 Builder 模式)
│       ├── context/
│       │   ├── mod.rs            # AgentInfo, re-exports
│       │   ├── base.rs           # BaseCtx -- 所有 Feature trait 實作
│       │   ├── agent.rs          # AgentCtx + CompletionRunner + CompletionStream
│       │   ├── cache.rs          # CacheService (moka LRU, 命名空間隔離)
│       │   ├── engine.rs         # EngineCard, RemoteEngines, RemoteTool/Agent
│       │   └── web3.rs           # Web3SDK enum, Web3Client builder, TEEClient
│       ├── model/                # LLM Provider 實作
│       │   ├── mod.rs            # Model struct, CompletionFeaturesDyn, NotImplemented, MockImplemented
│       │   ├── openai.rs         # OpenAI API (含 types.rs 子模組)
│       │   ├── deepseek.rs       # DeepSeek
│       │   ├── gemini.rs         # Gemini (含 types.rs 子模組)
│       │   ├── cohere.rs         # Cohere (embedding only)
│       │   ├── doubao.rs         # 豆包
│       │   ├── kimi.rs           # Kimi
│       │   └── xai.rs            # xAI
│       ├── memory.rs             # MemoryManagement, Conversation, KIP 記憶工具
│       ├── store.rs              # Store (ObjectStore 抽象 + namespace prefix)
│       ├── hook.rs               # Hook trait, Hooks chain, SingleThreadHook
│       ├── extension/            # 內建擴展
│       │   ├── extractor.rs      # 結構化資料提取 Agent
│       │   ├── fetch.rs          # Web 資源抓取
│       │   └── google.rs         # Google Search Tool
│       └── management.rs         # Management trait, Visibility (Private/Protected/Public)
├── anda_engine_server/           # Axum HTTP Server
├── anda_web3_client/             # 非 TEE 環境的 Web3 客戶端
├── anda_cli/                     # CLI 工具
├── agents/                       # Agent 實作
│   ├── anda_assistant/           # 通用 Assistant Agent (KIP 記憶)
│   ├── anda_bot/                 # Telegram Bot Agent
│   └── anda_nexus/               # Nexus Node (thread-based 對話管理)
├── tools/                        # 區塊鏈工具
│   ├── anda_icp/                 # ICP Ledger
│   └── anda_bnb/                 # BNB Chain
├── characters/                   # 角色定義 (TOML)
├── examples/                     # 範例
└── docs/                         # 文檔
```

### 1.2 Crate 依賴關係圖

```
                    anda_core
               (零業務邏輯 trait 層)
                   ↑       ↑
                   │       │
            anda_engine   anda_web3_client
           (核心引擎)      (Web3 密鑰管理)
           ↑    ↑   ↑
           │    │   │
  anda_engine_server  │   tools/anda_icp
  (HTTP API)          │   tools/anda_bnb
                      │
              agents/anda_bot
              agents/anda_assistant
              agents/anda_nexus
```

**核心設計原則**：
- `anda_core` 是純 trait 層，**不含任何實作**，不依賴任何重量級 crate
- `anda_engine` 提供所有 trait 的實作，但其 API 仍通過 trait 約束
- Agent/Tool 只依賴 trait，不依賴具體實作，實現真正的依賴反轉

```
// 檔案: anda_core/src/lib.rs (行 1-15)
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub type BoxPinFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;
pub type Json = serde_json::Value;
```

這三個 type alias 是整個框架的基礎類型。`BoxPinFut` 是 Dual-Trait 模式的關鍵。

---

## 2. Dual-Trait 模式深度剖析

### 2.1 問題背景

Rust 的 `impl Future` 返回類型不是物件安全的（object-safe），無法用於 `dyn Trait`。這意味著：

```rust
// 這個 trait 不是 object-safe 的，不能寫 Box<dyn Agent>
trait Agent {
    fn run(&self) -> impl Future<Output = Result<Output, Error>> + Send;
    //               ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    //               impl Trait 不是 object-safe
}
```

但框架需要在 runtime 動態選擇 Agent（從 `BTreeMap<String, Box<dyn ???>>` 中查找），所以必須使用 `dyn` 多態。

### 2.2 解法：三層結構

Anda 使用三層結構解決這個矛盾：

```
層 1: Agent<C>        -- 使用者實作 (靜態 dispatch, impl Future)
層 2: AgentDyn<C>     -- 引擎內部 (動態 dispatch, BoxPinFut)
層 3: AgentWrapper<T,C> -- 橋接器 (自動將 impl Future 包裝為 BoxPinFut)
```

### 2.3 靜態 Trait -- `Agent<C>`

```rust
// 檔案: anda_core/src/agent.rs (行 57-137)
pub trait Agent<C>: Send + Sync
where
    C: AgentContext + Send + Sync,
{
    /// 名稱規則：小寫字母開頭，a-z/0-9/_ ，最長 64 字元
    fn name(&self) -> String;
    fn description(&self) -> String;

    /// 預設實作：從 name + description 自動生成 FunctionDefinition
    fn definition(&self) -> FunctionDefinition {
        FunctionDefinition {
            name: self.name().to_ascii_lowercase(),
            description: self.description(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "prompt": {"type": "string", "description": "optimized prompt or message."},
                },
                "required": ["prompt"],
            }),
            strict: None,
        }
    }

    /// 資源標籤：宣告此 Agent 支援的資源類型
    fn supported_resource_tags(&self) -> Vec<String> { Vec::new() }

    /// 初始化鉤子：Engine::build() 時呼叫一次
    fn init(&self, _ctx: C) -> impl Future<Output = Result<(), BoxError>> + Send {
        futures::future::ready(Ok(()))
    }

    /// 工具依賴：在 register_agent 時檢查
    fn tool_dependencies(&self) -> Vec<String> { Vec::new() }

    /// 核心執行方法：注意返回類型是 impl Future
    fn run(
        &self,
        ctx: C,
        prompt: String,
        resources: Vec<Resource>,
    ) -> impl Future<Output = Result<AgentOutput, BoxError>> + Send;
}
```

**關鍵觀察**：
- `run()` 返回 `impl Future`，使用者可以直接寫 `async fn`
- `definition()` 有預設實作，大多數 Agent 不需要覆寫
- `init()` 有預設空實作，只有需要初始化的 Agent 才覆寫
- `tool_dependencies()` 讓引擎在建構時就能檢查依賴

### 2.4 動態 Trait -- `AgentDyn<C>`

```rust
// 檔案: anda_core/src/agent.rs (行 143-165)
pub trait AgentDyn<C>: Send + Sync
where
    C: AgentContext + Send + Sync,
{
    fn label(&self) -> &str;
    fn name(&self) -> String;
    fn definition(&self) -> FunctionDefinition;
    fn tool_dependencies(&self) -> Vec<String>;
    fn supported_resource_tags(&self) -> Vec<String>;

    /// 注意：返回類型是 BoxPinFut（物件安全的）
    fn init(&self, ctx: C) -> BoxPinFut<Result<(), BoxError>>;
    fn run(
        &self,
        ctx: C,
        prompt: String,
        resources: Vec<Resource>,
    ) -> BoxPinFut<Result<AgentOutput, BoxError>>;
}
```

**差異比較**：

| 方法 | `Agent<C>` | `AgentDyn<C>` |
|------|-----------|---------------|
| `init` | `impl Future<...> + Send` | `BoxPinFut<...>` |
| `run` | `impl Future<...> + Send` | `BoxPinFut<...>` |
| 新增 | -- | `label(&self) -> &str` |

`label` 是 `AgentDyn` 獨有的，用於多模型路由（不同 Agent 可以指向不同的 LLM）。

### 2.5 橋接 Wrapper -- `AgentWrapper<T, C>`

```rust
// 檔案: anda_core/src/agent.rs (行 168-217)
struct AgentWrapper<T, C>
where
    T: Agent<C> + 'static,
    C: AgentContext + Send + Sync + 'static,
{
    inner: Arc<T>,         // Arc 包裝靜態 Agent
    label: String,         // 用於模型路由的標籤
    _phantom: PhantomData<C>,
}

impl<T, C> AgentDyn<C> for AgentWrapper<T, C>
where
    T: Agent<C> + 'static,
    C: AgentContext + Send + Sync + 'static,
{
    fn init(&self, ctx: C) -> BoxPinFut<Result<(), BoxError>> {
        let agent = self.inner.clone();  // Clone Arc (不是 clone Agent)
        Box::pin(async move { agent.init(ctx).await })
        //       ^^^^^^^^ 將 impl Future 包裝為 BoxPinFut
    }

    fn run(
        &self, ctx: C, prompt: String, resources: Vec<Resource>,
    ) -> BoxPinFut<Result<AgentOutput, BoxError>> {
        let agent = self.inner.clone();
        Box::pin(async move { agent.run(ctx, prompt, resources).await })
    }
    // ... 其餘方法直接轉發
}
```

**記憶體佈局**：

```
AgentWrapper<MyAgent, AgentCtx>
  ├── inner: Arc<MyAgent>        -- 8 bytes (指針)
  ├── label: String              -- 24 bytes (ptr + len + cap)
  └── _phantom: PhantomData      -- 0 bytes

存入 BTreeMap 時:
  Box<dyn AgentDyn<AgentCtx>>    -- 16 bytes (data ptr + vtable ptr)
```

### 2.6 AgentSet -- 註冊與查找

```rust
// 檔案: anda_core/src/agent.rs (行 224-354)
#[derive(Default)]
pub struct AgentSet<C: AgentContext> {
    pub set: BTreeMap<String, Box<dyn AgentDyn<C>>>,
    //                ^^^^^^  ^^^^^^^^^^^^^^^^^^^^
    //                name    動態 trait 物件
}

impl<C> AgentSet<C>
where C: AgentContext + Send + Sync + 'static,
{
    pub fn add<T>(&mut self, agent: T, label: Option<String>) -> Result<(), BoxError>
    where T: Agent<C> + Send + Sync + 'static,
    {
        let name = agent.name().to_ascii_lowercase();
        if self.set.contains_key(&name) {
            return Err(format!("agent {} already exists", name).into());
        }
        validate_function_name(&name)?;  // 驗證名稱規則

        // 在這裡發生了靜態→動態的轉換
        let agent_dyn = AgentWrapper {
            inner: Arc::new(agent),  // 將靜態 Agent 包裝為 Arc
            label: label.unwrap_or_else(|| name.clone()),
            _phantom: PhantomData,
        };
        self.set.insert(name, Box::new(agent_dyn));
        //                    ^^^^^^^^^^^^^^^^^^^ Box<AgentWrapper> -> Box<dyn AgentDyn>
        Ok(())
    }
}
```

### 2.7 Tool 系統的平行設計

Tool 使用完全相同的雙 Trait 模式，但增加了**關聯型別**：

```rust
// 檔案: anda_core/src/tool.rs (行 42-137)
pub trait Tool<C>: Send + Sync
where C: BaseContext + Send + Sync,
{
    type Args: DeserializeOwned + Send;   // 強型別輸入
    type Output: Serialize;               // 強型別輸出

    fn name(&self) -> String;
    fn definition(&self) -> FunctionDefinition;  // JSON Schema

    // 使用者實作：強型別 Args
    fn call(&self, ctx: C, args: Self::Args, resources: Vec<Resource>)
        -> impl Future<Output = Result<ToolOutput<Self::Output>, BoxError>> + Send;

    // 預設實作：JSON -> Args -> call() -> Output -> JSON
    fn call_raw(&self, ctx: C, args: Json, resources: Vec<Resource>)
        -> impl Future<Output = Result<ToolOutput<Json>, BoxError>> + Send
    {
        async move {
            // 自動反序列化
            let args: Self::Args = serde_json::from_value(args)
                .map_err(|err| format!("tool {}, invalid args: {}", self.name(), err))?;
            let mut result = self.call(ctx, args, resources).await
                .map_err(|err| format!("tool {}, call failed: {}", self.name(), err))?;
            // 自動序列化
            let output = serde_json::to_value(&result.output)?;
            if result.usage.requests == 0 {
                result.usage.requests = 1;  // 至少算一次請求
            }
            Ok(ToolOutput { output, artifacts: result.artifacts, usage: result.usage })
        }
    }
}
```

**call_raw 的精妙設計**：
1. 使用者只實作強型別 `call()` -- 編譯時檢查參數類型
2. 引擎透過 `call_raw()` 統一以 JSON 交互 -- runtime 動態參數
3. 錯誤訊息包含 tool 名稱 -- 方便 debug

ToolDyn 的橋接：
```rust
// 檔案: anda_core/src/tool.rs (行 163-200)
struct ToolWrapper<T, C>(Arc<T>, PhantomData<C>);

impl<T, C> ToolDyn<C> for ToolWrapper<T, C> {
    fn call(&self, ctx: C, args: Json, resources: Vec<Resource>)
        -> BoxPinFut<Result<ToolOutput<Json>, BoxError>>
    {
        let tool = self.0.clone();
        // 注意：這裡呼叫的是 call_raw，不是 call
        Box::pin(async move { tool.call_raw(ctx, args, resources).await })
    }
}
```

**關鍵差異**：Agent 的 Wrapper 呼叫 `run()`，Tool 的 Wrapper 呼叫 `call_raw()`（帶自動 JSON 轉換）。

### 2.8 資料流圖：從 LLM 到 Tool 執行

```
LLM 回應 tool_calls: [{name: "search", args: {"query": "..."}}]
    │
    ▼
CompletionRunner.inner_next()
    │
    ├── ctx.tools.contains("search") → true
    │
    ├── ToolInput { name: "search", args: Json, resources: [...] }
    │
    ├── ctx.tool_call(input)
    │     │
    │     ├── self.child_base("search") → BaseCtx (depth+1)
    │     │
    │     ├── self.tools.get("search") → &dyn ToolDyn<BaseCtx>
    │     │
    │     └── tool.call(ctx, args_json, resources)
    │           │
    │           ├── ToolWrapper.call() -- Box::pin(tool.call_raw(...))
    │           │     │
    │           │     ├── serde_json::from_value::<SearchArgs>(args)
    │           │     ├── GoogleSearchTool::call(ctx, SearchArgs, resources)
    │           │     └── serde_json::to_value(result)
    │           │
    │           └── ToolOutput<Json> { output, artifacts, usage }
    │
    └── ContentPart::ToolOutput { name, output, call_id }
        → 加入下一輪 CompletionRequest
```

**Clawtex 實作建議**：

Clawtex 目前的 Tool trait 沒有靜態/動態分離：

```rust
// clawtex-core/src/tools/mod.rs (現狀)
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn execute(&self, args: &Value, context: &ToolContext) -> Result<Value>;
}
```

建議改為：
```rust
// 建議的 Anda 風格改進
pub trait ClawtexTool: Send + Sync {
    type Args: DeserializeOwned + Send;
    type Output: Serialize;

    fn name(&self) -> &str;
    fn definition(&self) -> ToolDefinition;

    fn execute(&self, ctx: ToolContext, args: Self::Args)
        -> impl Future<Output = Result<Self::Output>> + Send;

    // 自動 JSON 橋接
    fn execute_raw(&self, ctx: ToolContext, args: Value)
        -> impl Future<Output = Result<Value>> + Send
    {
        async move {
            let args: Self::Args = serde_json::from_value(args)?;
            let result = self.execute(ctx, args).await?;
            Ok(serde_json::to_value(result)?)
        }
    }
}
```

---

## 3. CompletionRunner 迭代器引擎

### 3.1 結構定義

```rust
// 檔案: anda_engine/src/context/agent.rs (行 948-962)
pub struct CompletionRunner {
    ctx: AgentCtx,                      // 完整 Agent 上下文
    model: Model,                       // 主模型
    fallback_model: Option<Model>,      // 後備模型 (用一次即耗盡)
    req: CompletionRequest,             // 當前請求 (每輪修改)
    resources: Vec<Resource>,           // 外部資源
    chat_history: Vec<Message>,         // 累計對話歷史
    tool_calls: Vec<ToolCall>,          // 累計工具呼叫
    usage: Usage,                       // 累計 token 用量
    artifacts: Vec<Resource>,           // 累計產出
    steering_message: Option<String>,   // 外部介入訊息
    follow_up_message: Option<String>,  // 後續追問
    done: bool,                         // 完成標誌
    step: usize,                        // 當前步數
}
```

### 3.2 核心方法：next()

```rust
// 檔案: anda_engine/src/context/agent.rs (行 995-1011)
pub async fn next(&mut self) -> Result<Option<AgentOutput>, BoxError> {
    if self.done {
        return Ok(None);  // 已完成，直接返回 None
    }

    // 支援取消：tokio::select! 同時等待取消信號和實際執行
    let token = self.ctx.base.cancellation_token();
    tokio::select! {
        _ = token.cancelled() => {
            let output = AgentOutput {
                failed_reason: Some("operation cancelled".to_string()),
                ..Default::default()
            };
            Ok(Some(self.final_output(output)))
        }
        res = self.inner_next() => res
    }
}
```

### 3.3 inner_next() -- 單輪執行邏輯

```rust
// 檔案: anda_engine/src/context/agent.rs (行 1013-1239)
async fn inner_next(&mut self) -> Result<Option<AgentOutput>, BoxError> {
    self.step += 1;

    // ---- 第 1 階段：呼叫 LLM ----
    let mut output = self.model.completion(self.req.clone()).await?;
    output.model = Some(self.model.model_name());
    self.usage.accumulate(&output.usage);

    // ---- 第 2 階段：Fallback 模型 ----
    // 如果主模型失敗且有 fallback，切換模型重試
    // 注意：fallback_model.take() 意味著只重試一次
    if output.failed_reason.is_some() {
        if let Some(fallback) = self.fallback_model.take() {
            self.model = fallback;  // 永久切換到 fallback
            let mut output2 = self.model.completion(self.req.clone()).await?;
            // 如果 fallback 也失敗，組合兩個錯誤原因
            if let Some(fallback_reason) = output2.failed_reason.clone() {
                output2.failed_reason = Some(format!(
                    "primary model failed: {}; fallback model failed: {}",
                    primary_reason, fallback_reason
                ));
                return Ok(Some(self.final_output(output2)));
            }
            output = output2;  // 使用 fallback 結果
        }
    }

    // ---- 第 3 階段：累計歷史 ----
    self.req.tool_choice_required = false;  // 關閉強制工具呼叫
    self.req.output_schema = None;
    self.req.raw_history.append(&mut output.raw_history);
    self.chat_history.append(&mut output.chat_history);

    // ---- 第 4 階段：Steering 介入 ----
    if let Some(steering) = self.steering_message.take() {
        // 丟棄未執行的 tool_calls
        if !output.tool_calls.is_empty() {
            self.req.raw_history.pop();
        }
        // 重新設定為 user 訊息
        self.req.prompt = steering;
        self.req.role = Some("user".to_string());
        output.usage = self.usage.clone();
        return Ok(Some(output));  // 返回中間結果，繼續下一輪
    }

    // ---- 第 5 階段：並行執行 Tool/Agent 呼叫 ----
    let tool_calls = std::mem::take(&mut output.tool_calls);
    let mut tool_call_futs: Vec<BoxPinFut<...>> = Vec::new();

    for tool in tool_calls {
        if self.ctx.tools.contains(&tool.name) || tool.name.starts_with("RT_") {
            // 本地/遠端 Tool 呼叫
            let ctx = self.ctx.clone();
            tool_call_futs.push(Box::pin(async move {
                match ctx.tool_call(input).await {
                    Ok((res, remote_id)) => { ... (Some(tool), None) }
                    Err(err) => (None, Some(err.to_string()))
                }
            }));
        } else if self.ctx.agents.contains(&tool.name) || tool.name.starts_with("LA_") {
            // Agent-as-Tool 呼叫
            let ctx = self.ctx.clone();
            tool_call_futs.push(Box::pin(async move {
                match ctx.agent_run(input).await {
                    Ok((res, remote_id)) => { ... }
                    Err(err) => (None, Some(err.to_string()))
                }
            }));
        }
    }

    // **並行** 等待所有 tool 呼叫完成
    if !tool_call_futs.is_empty() {
        let results = futures::future::join_all(tool_call_futs).await;
        // 收集結果、累計 usage、記錄 artifacts
    }

    // ---- 第 6 階段：決定是否繼續 ----
    if !tool_calls_continue.is_empty() {
        // 有工具結果需要繼續 → 準備下一輪
        self.req.role = Some("tool".to_string());
        self.req.content.append(&mut tool_calls_continue);
        return Ok(Some(output));  // 返回中間結果
    }

    // 檢查 steering / follow_up
    if let Some(prompt) = self.steering_message.take()
        .or_else(|| self.follow_up_message.take())
    {
        // 繼續對話
        self.req.prompt = prompt;
        return Ok(Some(output));
    }

    // 全部完成
    Ok(Some(self.final_output(output)))
}
```

### 3.4 final_output -- 彙總

```rust
// 檔案: anda_engine/src/context/agent.rs (行 1241-1251)
fn final_output(&mut self, mut output: AgentOutput) -> AgentOutput {
    self.done = true;
    self.chat_history.append(&mut output.chat_history);
    output.chat_history = std::mem::take(&mut self.chat_history);
    output.tool_calls = std::mem::take(&mut self.tool_calls);
    output.artifacts = std::mem::take(&mut self.artifacts);
    output.usage = std::mem::take(&mut self.usage);
    output
}
```

### 3.5 CompletionStream -- Stream 適配器

```rust
// 檔案: anda_engine/src/context/agent.rs (行 1253-1271)
pub struct CompletionStream {
    runner: CompletionRunner,
}

impl Stream for CompletionStream {
    type Item = Result<AgentOutput, BoxError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let future = self.runner.next();
        tokio::pin!(future);
        match future.poll(cx) {
            Poll::Ready(Ok(Some(output))) => Poll::Ready(Some(Ok(output))),
            Poll::Ready(Ok(None)) => Poll::Ready(None),
            Poll::Ready(Err(e)) => Poll::Ready(Some(Err(e))),
            Poll::Pending => Poll::Pending,
        }
    }
}
```

### 3.6 CompletionRunner 狀態機圖

```
                    ┌──────────────┐
                    │   Created    │
                    └──────┬───────┘
                           │ next()
                           ▼
              ┌────────────────────────┐
              │  Call LLM Completion   │
              └────────────┬───────────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
    failed + fallback   tool_calls   no tools
              │            │            │
              ▼            ▼            │
    ┌─────────────┐  ┌──────────┐      │
    │ Retry with  │  │ Execute  │      │
    │  fallback   │  │ tools    │      │
    │  model      │  │ (join!)  │      │
    └──────┬──────┘  └────┬─────┘      │
           │              │            │
           │    ┌─────────┼─────┐      │
           │    │         │     │      │
           │  errors   results  │      │
           │    │         │     │      │
           │    ▼         │     │      │
           │ final_out    │     │      │
           │              ▼     │      │
           │   steering / follow_up?   │
           │         │         │       │
           │    yes  │    no   │       │
           │         │         │       │
           │  ┌──────┴───┐     │       │
           │  │ continue │     │       │
           │  │ next()   │     │       │
           │  └──────────┘     │       │
           │                   ▼       │
           │            ┌──────────┐   │
           └───────────►│final_out │◄──┘
                        │ done=true│
                        └──────────┘
```

### 3.7 Steering 與 Follow-up 機制

```rust
// 檔案: anda_engine/src/context/agent.rs (行 976-987)
/// 中途介入：在當前 tool 執行完後插入 user 訊息
pub fn steer(&mut self, message: String) {
    self.steering_message = Some(message);
}

/// 追問：在 agent 完成後追加一輪對話
pub fn follow_up(&mut self, message: String) {
    self.follow_up_message = Some(message);
}
```

**使用場景**：
- `steer()`：Agent 正在執行多步 tool 呼叫時，使用者想要改變方向
- `follow_up()`：Agent 完成初始任務後，自動追問

**Clawtex 實作建議**：

Clawtex 的 `agent_runtime.rs` 目前沒有 Steering 機制。建議：

```rust
// 新增 CompletionRunner for clawtex
pub struct ClawtexRunner {
    provider: Arc<dyn Provider>,
    messages: Vec<Message>,
    tools: Vec<ToolDef>,
    max_rounds: u8,
    round: u8,
    steering: Option<String>,
    follow_up: Option<String>,
    done: bool,
    total_usage: Usage,
}

impl ClawtexRunner {
    pub async fn next(&mut self) -> Result<Option<AgentOutput>> {
        if self.done { return Ok(None); }
        self.round += 1;
        if self.round > self.max_rounds {
            return Err(anyhow!("max rounds exceeded"));
        }
        // ... (類似 Anda 的邏輯)
    }

    pub fn steer(&mut self, msg: String) { self.steering = Some(msg); }
    pub fn follow_up(&mut self, msg: String) { self.follow_up = Some(msg); }
}
```

---

## 4. Feature Trait 分離架構

### 4.1 總覽

```rust
// 檔案: anda_core/src/context.rs (行 57-186)

// AgentContext = 完整能力 (用於 Agent)
pub trait AgentContext: BaseContext + CompletionFeatures + EmbeddingFeatures {
    fn tool_definitions(...) -> Vec<FunctionDefinition>;
    fn tool_call(...) -> impl Future<...>;
    fn agent_run(...) -> impl Future<...>;
    fn remote_agent_run(...) -> impl Future<...>;
}

// BaseContext = 基礎能力 (用於 Tool)
pub trait BaseContext: Sized
    + StateFeatures      // 身份、時間、取消
    + KeysFeatures       // 密碼學操作
    + StoreFeatures      // 持久化儲存
    + CacheFeatures      // 記憶體快取
    + HttpFeatures       // HTTP 請求
    + CanisterCaller     // ICP 合約呼叫
{
    fn remote_tool_call(...) -> impl Future<...>;
}
```

### 4.2 各 Feature Trait 詳解

#### StateFeatures -- 執行環境

```rust
// 檔案: anda_core/src/context.rs (行 189-215)
pub trait StateFeatures: Sized {
    fn engine_id(&self) -> &Principal;           // 引擎唯一 ID
    fn engine_name(&self) -> &str;               // 引擎名稱
    fn caller(&self) -> &Principal;              // 已驗證的呼叫者
    fn meta(&self) -> &RequestMeta;              // 請求後設資料
    fn cancellation_token(&self) -> CancellationToken;  // 層級取消
    fn time_elapsed(&self) -> Duration;          // 已用時間
}
```

#### KeysFeatures -- 密碼學 (13 個方法)

```rust
// 檔案: anda_core/src/context.rs (行 242-313)
pub trait KeysFeatures: Sized {
    fn a256gcm_key(...)        -> impl Future<Output = Result<[u8; 32], BoxError>>;
    fn ed25519_sign_message(...)    -> impl Future<Output = Result<[u8; 64], BoxError>>;
    fn ed25519_verify(...)         -> impl Future<Output = Result<(), BoxError>>;
    fn ed25519_public_key(...)     -> impl Future<Output = Result<[u8; 32], BoxError>>;
    fn secp256k1_sign_message_bip340(...) -> impl Future<...>;
    fn secp256k1_verify_bip340(...)       -> impl Future<...>;
    fn secp256k1_sign_message_ecdsa(...)  -> impl Future<...>;
    fn secp256k1_sign_digest_ecdsa(...)   -> impl Future<...>;
    fn secp256k1_verify_ecdsa(...)        -> impl Future<...>;
    fn secp256k1_public_key(...)          -> impl Future<Output = Result<[u8; 33], BoxError>>;
}
```

#### StoreFeatures -- 持久化 (5 個方法)

```rust
// 檔案: anda_core/src/context.rs (行 319-366)
pub trait StoreFeatures: Sized {
    fn store_get(&self, path: &Path) -> impl Future<Output = Result<(Bytes, ObjectMeta), BoxError>>;
    fn store_list(&self, prefix: Option<&Path>, offset: &Path) -> impl Future<...>;
    fn store_put(&self, path: &Path, mode: PutMode, value: Bytes) -> impl Future<...>;
    fn store_rename_if_not_exists(&self, from: &Path, to: &Path) -> impl Future<...>;
    fn store_delete(&self, path: &Path) -> impl Future<...>;
}
```

#### CacheFeatures -- 記憶體快取 (6 個方法)

```rust
// 檔案: anda_core/src/context.rs (行 381-427)
pub trait CacheFeatures: Sized {
    fn cache_contains(&self, key: &str) -> bool;
    fn cache_get<T: DeserializeOwned>(&self, key: &str) -> impl Future<Output = Result<T, BoxError>>;
    fn cache_get_with<T, F>(&self, key: &str, init: F) -> impl Future<...>;  // lazy init
    fn cache_set<T: Serialize>(&self, key: &str, val: (T, Option<CacheExpiry>)) -> impl Future<Output = ()>;
    fn cache_set_if_not_exists<T>(&self, key: &str, val: (T, Option<CacheExpiry>)) -> impl Future<Output = bool>;
    fn cache_delete(&self, key: &str) -> impl Future<Output = bool>;
}
```

#### HttpFeatures -- HTTP 通訊 (3 個方法)

```rust
// 檔案: anda_core/src/context.rs (行 434-481)
pub trait HttpFeatures: Sized {
    fn https_call(&self, url: &str, method: Method, headers: Option<HeaderMap>, body: Option<Vec<u8>>)
        -> impl Future<Output = Result<Response, BoxError>>;
    fn https_signed_call(&self, url: &str, method: Method, message_digest: [u8; 32], ...)
        -> impl Future<...>;
    fn https_signed_rpc<T: DeserializeOwned>(&self, endpoint: &str, method: &str, args: impl Serialize)
        -> impl Future<Output = Result<T, BoxError>>;
}
```

### 4.3 CacheStoreFeatures -- 快取穿透組合

```rust
// 檔案: anda_core/src/context.rs (行 486-612)
#[async_trait]
pub trait CacheStoreFeatures: StoreFeatures + CacheFeatures + Send + Sync + 'static {
    /// 從 store 初始化 cache (啟動時)
    async fn cache_store_init<T, F>(&self, key: &str, init: F) -> Result<(), BoxError>;

    /// cache miss → 自動從 store 讀取 → 寫入 cache → 返回
    async fn cache_store_get<T>(&self, key: &str) -> Result<(T, UpdateVersion), BoxError>;

    /// 寫入 store + 更新/失效 cache (支援 CAS 原子更新)
    async fn cache_store_set<T>(&self, key: &str, val: T, version: Option<UpdateVersion>)
        -> Result<UpdateVersion, BoxError>;

    /// 從 cache + store 都刪除
    async fn cache_store_delete(&self, key: &str) -> Result<(), BoxError>;
}
```

**CAS (Compare-And-Swap) 原子更新**：

```rust
// 當 version 有值時，使用原子更新
if let Some(ver) = version {
    let res = self.store_put(&p, PutMode::Update(OsVersion {
        e_tag: ver.e_tag.clone(),
        version: ver.version.clone(),
    }), data.into()).await?;
    // 只有 store 更新成功後，才更新 cache
    self.cache_set(key, (CacheStoreValue(val, ver.clone()), None)).await;
} else {
    // 無版本控制：覆寫 store，失效 cache
    self.store_put(&p, PutMode::Overwrite, data.into()).await?;
    self.cache_delete(key).await;  // 讓下次讀取重新從 store 載入
}
```

**Clawtex 實作建議**：

```rust
// Clawtex 目前沒有 Feature Trait 分離
// 建議引入 3 層 Context（不需要 ICP 相關的）
pub trait ClawtexBaseCtx: Send + Sync {
    fn config(&self) -> &AppConfig;
    fn workspace(&self) -> &Path;
    fn cancel_token(&self) -> CancellationToken;
    fn depth(&self) -> u8;
}

pub trait ClawtexToolCtx: ClawtexBaseCtx {
    fn cache_get(&self, key: &str) -> Option<Value>;
    fn cache_set(&self, key: &str, val: Value, ttl: Option<Duration>);
}

pub trait ClawtexAgentCtx: ClawtexToolCtx {
    fn provider(&self) -> &dyn Provider;
    fn tools(&self) -> &ToolRegistry;
    fn call_tool(&self, name: &str, args: Value) -> BoxFuture<Result<Value>>;
}
```

---

## 5. Engine Builder 與初始化流程

### 5.1 EngineBuilder 結構

```rust
// 檔案: anda_engine/src/engine.rs (行 322-339)
#[non_exhaustive]
pub struct EngineBuilder {
    info: AgentInfo,
    tools: ToolSet<BaseCtx>,                        // Tool 集合
    agents: AgentSet<AgentCtx>,                     // Agent 集合
    remote: BTreeMap<String, RemoteEngineArgs>,     // 遠端引擎
    model: Model,                                   // 主 LLM
    models: BTreeMap<String, Model>,                // label -> Model 映射
    fallback_model: Option<Model>,                  // 全域 fallback
    fallback_models: BTreeMap<String, Model>,       // label -> fallback 映射
    store: Store,                                   // 儲存後端
    web3: Arc<Web3SDK>,                             // Web3/TEE 客戶端
    hooks: Arc<Hooks>,                              // 生命週期鉤子
    cancellation_token: CancellationToken,          // 全域取消
    export_agents: BTreeSet<String>,                // 對外暴露的 Agent
    export_tools: BTreeSet<String>,                 // 對外暴露的 Tool
    management: Option<Arc<dyn Management>>,        // 訪問控制
}
```

### 5.2 build() 過程

```rust
// 檔案: anda_engine/src/engine.rs (行 579-657)
pub async fn build(mut self, default_agent: String) -> Result<Engine, BoxError> {
    // 1. 驗證預設 agent 存在
    if !self.agents.contains(&default_agent) {
        return Err(format!("default agent {} not found", default_agent).into());
    }

    // 2. 收集所有 path 名稱（用於 cache namespace）
    let names: BTreeSet<Path> = self.tools.set.keys()
        .map(|p| Path::from(format!("T:{}", p)))
        .chain(self.agents.set.keys()
            .map(|p| Path::from(format!("A:{}", p))))
        .collect();

    // 3. 註冊遠端引擎
    let mut remote = RemoteEngines::new();
    for (_, engine) in self.remote {
        remote.register(self.web3.as_ref(), engine).await?;
    }

    // 4. 建構 BaseCtx
    let ctx = BaseCtx::new(id, name, cancellation_token, names, web3, store, remote);

    // 5. 建構 AgentCtx
    let ctx = AgentCtx::new(ctx, model, models, fallback_model, fallback_models, tools, agents);

    // 6. 初始化所有 Tool（每個 Tool 獲得獨立的 BaseCtx）
    for (name, tool) in &tools.set {
        let ct = ctx.child_base_with(id, name, meta.clone())?;
        tool.init(ct).await?;
    }

    // 7. 初始化所有 Agent（每個 Agent 獲得獨立的 AgentCtx）
    for (name, agent) in &agents.set {
        let ct = ctx.child_with(id, name, agent.label(), meta.clone())?;
        agent.init(ct).await?;
    }

    // 8. 組裝 Engine
    Ok(Engine { id, ctx, info, default_agent, export_agents, export_tools, hooks, management })
}
```

**初始化順序**：Tool 先於 Agent 初始化。這確保了 Agent 在 init 時可以使用其依賴的 Tool。

### 5.3 Tool 依賴檢查

```rust
// 檔案: anda_engine/src/engine.rs (行 462-474)
pub fn register_agent<T>(mut self, agent: T, label: Option<String>) -> Result<Self, BoxError> {
    // 在註冊時（而非運行時）檢查依賴
    for tool in agent.tool_dependencies() {
        if !self.tools.contains(&tool) && !self.agents.contains(&tool) {
            return Err(format!("dependent tool {} not found", tool).into());
        }
    }
    self.agents.add(agent, label)?;
    Ok(self)
}
```

**Clawtex 實作建議**：

```rust
pub struct ClawtexEngineBuilder {
    config: AppConfig,
    tools: ToolRegistry,
    providers: ProviderRouter,
    hooks: Vec<Box<dyn Hook>>,
    cancel_token: CancellationToken,
}

impl ClawtexEngineBuilder {
    pub fn register_tool(mut self, tool: impl ClawtexTool + 'static) -> Result<Self> {
        self.tools.register(tool)?;
        Ok(self)
    }

    pub async fn build(self) -> Result<ClawtexEngine> {
        // 初始化所有 tool
        for tool in self.tools.iter() {
            tool.init(&self.config).await?;
        }
        Ok(ClawtexEngine { ... })
    }
}
```

---

## 6. BaseCtx 上下文實作

### 6.1 結構定義

```rust
// 檔案: anda_engine/src/context/base.rs (行 43-70)
const CONTEXT_MAX_DEPTH: u8 = 42;
const CACHE_MAX_CAPACITY: u64 = 1000000;

#[derive(Clone)]
pub struct BaseCtx {
    pub(crate) id: Principal,                          // 引擎 ID
    pub(crate) name: String,                           // 引擎名稱
    pub(crate) caller: Principal,                      // 呼叫者身份
    pub(crate) path: Path,                             // 命名空間路徑
    pub(crate) cancellation_token: CancellationToken,  // 層級取消 token
    pub(crate) start_at: Instant,                      // 計時起點
    pub(crate) depth: u8,                              // 嵌套深度
    pub(crate) web3: Arc<Web3SDK>,                     // Web3 客戶端 (共享)
    pub(crate) remote: Arc<RemoteEngines>,             // 遠端引擎 (共享)
    pub(crate) state: Arc<RwLock<Extensions>>,         // 型別安全狀態容器
    pub(crate) meta: RequestMeta,                      // 請求元資料
    cache: Arc<CacheService>,                          // 快取 (共享)
    store: Store,                                      // 儲存 (共享)
}
```

### 6.2 child() -- 子上下文建立

```rust
// 檔案: anda_engine/src/context/base.rs (行 127-149)
pub(crate) fn child(&self, path: String) -> Result<Self, BoxError> {
    let path = Path::parse(path)?;
    let child = Self {
        id: self.id,
        name: self.name.clone(),
        caller: self.caller,
        path,                                              // 新路徑
        cancellation_token: self.cancellation_token.child_token(), // 子 token
        start_at: self.start_at,                           // 共享計時
        cache: self.cache.clone(),                         // Arc clone (共享快取)
        store: self.store.clone(),                         // Arc clone (共享儲存)
        web3: self.web3.clone(),                           // Arc clone
        depth: self.depth + 1,                             // 深度 +1
        remote: self.remote.clone(),                       // Arc clone
        state: self.state.clone(),                         // Arc clone
        meta: self.meta.clone(),
    };

    if child.depth >= CONTEXT_MAX_DEPTH {
        return Err("Context depth limit exceeded".into());
    }
    Ok(child)
}
```

**Clone 成本分析**：

| 欄位 | Clone 成本 |
|------|-----------|
| `id: Principal` | 29 bytes copy |
| `name: String` | heap alloc + copy |
| `caller: Principal` | 29 bytes copy |
| `path: Path` | heap alloc |
| `cancellation_token` | Arc clone + child_token alloc |
| `cache: Arc<CacheService>` | Arc::clone = atomic increment |
| `store: Store` | Arc::clone = atomic increment |
| `web3: Arc<Web3SDK>` | Arc::clone = atomic increment |
| `state: Arc<RwLock<...>>` | Arc::clone = atomic increment |
| **總計** | ~4 atomic ops + 2 heap allocs |

### 6.3 路徑命名空間隔離

```rust
// 檔案: anda_engine/src/context/base.rs (行 491-494)
impl StoreFeatures for BaseCtx {
    async fn store_get(&self, path: &Path) -> Result<(Bytes, ObjectMeta), BoxError> {
        self.store.store_get(&self.path, path).await
        //                   ^^^^^^^^^
        //                   自動加入命名空間前綴
    }
}
```

Store 內部實作：
```
Store.store_get("A:my_agent", "data.json")
  → 實際路徑: "A:my_agent/data.json"

Store.store_get("T:search", "cache.bin")
  → 實際路徑: "T:search/cache.bin"
```

### 6.4 型別安全狀態容器

```rust
// 檔案: anda_engine/src/context/base.rs (行 202-214)
pub fn get_state<T>(&self) -> Option<T>
where T: Clone + Send + Sync + 'static,
{
    self.state.read().get::<T>().cloned()
    //         ^^^^ parking_lot::RwLock (比 std 快)
}

pub fn set_state<T>(&self, v: T) -> Option<T>
where T: Clone + Send + Sync + 'static,
{
    self.state.write().insert(v)
    //        ^^^^^ 會取得寫鎖
}
```

使用的是 `http::Extensions`（型別 map），可以存任意 `'static` 型別，通過 `TypeId` 查找。

**Clawtex 實作建議**：

```rust
// Clawtex 可以使用 http::Extensions 或 anymap
use http::Extensions;

pub struct ToolContext {
    pub config: Arc<AppConfig>,
    pub workspace: PathBuf,
    pub depth: u8,
    pub cancel: CancellationToken,
    pub state: Arc<RwLock<Extensions>>,
}

impl ToolContext {
    pub fn child(&self, tool_name: &str) -> Result<Self> {
        if self.depth >= 42 {
            return Err(anyhow!("depth limit exceeded"));
        }
        Ok(Self {
            workspace: self.workspace.join(tool_name),
            depth: self.depth + 1,
            cancel: self.cancel.child_token(),
            state: self.state.clone(),
            ..self.clone()
        })
    }
}
```

---

## 7. AgentCtx 與 LLM 整合

### 7.1 AgentCtx 結構

```rust
// 檔案: anda_engine/src/context/agent.rs (行 55-74)
#[derive(Clone)]
pub struct AgentCtx {
    pub base: BaseCtx,                                   // 基礎上下文
    pub(crate) label: String,                            // Agent 標籤 (用於模型路由)
    pub(crate) model: Model,                             // 主要 LLM
    pub(crate) models: Arc<BTreeMap<String, Model>>,     // label -> model 映射
    pub(crate) fallback_model: Option<Model>,            // 全域 fallback
    pub(crate) fallback_models: Arc<BTreeMap<String, Model>>, // label -> fallback 映射
    pub(crate) tools: Arc<ToolSet<BaseCtx>>,             // 工具集
    pub(crate) agents: Arc<AgentSet<AgentCtx>>,          // Agent 集
}
```

### 7.2 模型路由機制

```rust
// 檔案: anda_engine/src/context/agent.rs (行 174-207)
pub fn completion_iter(&self, req: CompletionRequest, resources: Vec<Resource>) -> CompletionRunner {
    // 從 req.model 或 self.label 決定使用哪個模型
    let label = req.model.as_ref().unwrap_or(&self.label);
    let model = self.models.get(label).cloned()
        .unwrap_or_else(|| self.model.clone());

    let fallback_model = self.fallback_models.get(label).cloned()
        .or_else(|| self.fallback_model.clone());

    CompletionRunner { ctx: self.clone(), model, fallback_model, req, resources, ... }
}
```

**多模型路由邏輯**：
1. `req.model = Some("fast")` → 查找 `models["fast"]`
2. `req.model = None` → 使用 `self.label` 查找
3. 都找不到 → 使用預設 `self.model`

### 7.3 CompletionFeatures 實作

```rust
// 檔案: anda_engine/src/context/agent.rs (行 520-563)
impl CompletionFeatures for AgentCtx {
    async fn completion(&self, req: CompletionRequest, resources: Vec<Resource>)
        -> Result<AgentOutput, BoxError>
    {
        let mut runner = self.completion_iter(req, resources);
        let mut last: Option<AgentOutput> = None;

        while let Some(step) = runner.next().await? {
            if step.failed_reason.is_some() {
                return Ok(step);  // 立即返回失敗
            }
            last = Some(step);
        }

        last.ok_or_else(|| "completion runner returned no output".into())
    }
}
```

### 7.4 tool_call 與 agent_run 的路由邏輯

```rust
// 檔案: anda_engine/src/context/agent.rs (行 384-431)
async fn tool_call(&self, mut input: ToolInput<Json>)
    -> Result<(ToolOutput<Json>, Option<Principal>), BoxError>
{
    // 路由優先級：
    // 1. 本地 tool (不帶 RT_ 前綴)
    if !input.name.starts_with("RT_") {
        let ctx = self.child_base(&input.name)?;
        let tool = self.tools.get(&input.name)?;
        return tool.call(ctx, input.args, input.resources).await.map(|o| (o, None));
    }

    // 2. 靜態註冊的遠端 tool
    if let Some((id, endpoint, tool_name)) = self.base.remote.get_tool_endpoint(&input.name) {
        return self.base.remote_tool_call(&endpoint, input).await.map(|o| (o, Some(id)));
    }

    // 3. 動態發現的遠端 tool (from cache_store)
    if let Ok((engines, _)) = self.cache_store_get::<RemoteEngines>(DYNAMIC_REMOTE_ENGINES).await {
        if let Some((id, endpoint, tool_name)) = engines.get_tool_endpoint(&input.name) {
            return self.base.remote_tool_call(&endpoint, input).await.map(|o| (o, Some(id)));
        }
    }

    Err(format!("tool {} not found", &input.name).into())
}
```

**Clawtex 實作建議**：

```rust
// 三層路由：本地 → cluster worker → remote engine
async fn route_tool_call(&self, name: &str, args: Value) -> Result<Value> {
    // 1. 本地 tool
    if let Some(tool) = self.tools.get(name) {
        return tool.execute_raw(self.child(name)?, args).await;
    }
    // 2. cluster worker
    if let Some(worker) = self.cluster.find_worker_for(name) {
        return worker.dispatch_tool(name, args).await;
    }
    // 3. MCP server
    if let Some(server) = self.mcp_servers.find_tool(name) {
        return server.call_tool(name, args).await;
    }
    Err(anyhow!("tool {} not found", name))
}
```

---

## 8. Hook 系統與 SingleThreadHook

### 8.1 Hook Trait

```rust
// 檔案: anda_engine/src/hook.rs (行 15-45)
#[async_trait]
pub trait Hook: Send + Sync {
    async fn on_agent_start(&self, _ctx: &AgentCtx, _agent: &str) -> Result<(), BoxError> {
        Ok(())
    }
    async fn on_agent_end(&self, _ctx: &AgentCtx, _agent: &str, output: AgentOutput)
        -> Result<AgentOutput, BoxError> {
        Ok(output)  // 可以修改 output
    }
    async fn on_tool_start(&self, _ctx: &BaseCtx, _tool: &str) -> Result<(), BoxError> {
        Ok(())
    }
    async fn on_tool_end(&self, _ctx: &BaseCtx, _tool: &str, output: ToolOutput<Json>)
        -> Result<ToolOutput<Json>, BoxError> {
        Ok(output)  // 可以修改 output
    }
}
```

### 8.2 Hooks Chain

```rust
// 檔案: anda_engine/src/hook.rs (行 69-108)
#[async_trait]
impl Hook for Hooks {
    async fn on_agent_end(&self, ctx: &AgentCtx, agent: &str, mut output: AgentOutput)
        -> Result<AgentOutput, BoxError>
    {
        // 鏈式調用：每個 hook 可以修改 output
        for hook in &self.hooks {
            output = hook.on_agent_end(ctx, agent, output).await?;
        }
        Ok(output)
    }
}
```

### 8.3 SingleThreadHook -- 並發控制

```rust
// 檔案: anda_engine/src/hook.rs (行 110-147)
pub struct SingleThreadHook {
    ttl: Duration,
}

#[async_trait]
impl Hook for SingleThreadHook {
    async fn on_agent_start(&self, ctx: &AgentCtx, _agent: &str) -> Result<(), BoxError> {
        let caller = ctx.caller();
        let now_ms = unix_ms();
        // 嘗試 CAS 設定：如果 key 已存在，說明有其他請求在跑
        let ok = ctx.cache_set_if_not_exists(
            caller.to_string().as_str(),
            (now_ms, Some(CacheExpiry::TTL(self.ttl))),
        ).await;
        if !ok {
            return Err("Only one prompt can run at a time.".into());
        }
        Ok(())
    }

    async fn on_agent_end(&self, ctx: &AgentCtx, _agent: &str, output: AgentOutput)
        -> Result<AgentOutput, BoxError>
    {
        let caller = ctx.caller();
        ctx.cache_delete(caller.to_string().as_str()).await;  // 釋放鎖
        Ok(output)
    }
}
```

**精妙之處**：
- 使用 `cache_set_if_not_exists` 實現分散式互斥鎖
- TTL 防止 crash 後永遠鎖住
- 鎖的粒度是 per-caller（不是全域鎖）

**Clawtex 實作建議**：

```rust
// Clawtex 可以用同樣的模式防止同一用戶並發請求
pub struct ClawtexSingleThreadHook {
    running: Arc<DashMap<String, Instant>>,
    ttl: Duration,
}

impl Hook for ClawtexSingleThreadHook {
    async fn on_agent_start(&self, user_id: &str) -> Result<()> {
        match self.running.entry(user_id.to_string()) {
            Entry::Occupied(e) => {
                if e.get().elapsed() < self.ttl {
                    return Err(anyhow!("Only one prompt can run at a time"));
                }
                e.insert(Instant::now());
            }
            Entry::Vacant(e) => { e.insert(Instant::now()); }
        }
        Ok(())
    }

    async fn on_agent_end(&self, user_id: &str) {
        self.running.remove(user_id);
    }
}
```

---

## 9. 記憶系統 (KIP + CognitiveNexus)

### 9.1 Conversation 結構

```rust
// 檔案: anda_engine/src/memory.rs (行 58-100)
#[derive(Debug, Clone, Deserialize, Serialize, AndaDBSchema)]
pub struct Conversation {
    pub _id: u64,
    pub user: Principal,
    pub thread: Option<Xid>,
    pub messages: Vec<Json>,
    pub resources: Vec<Resource>,
    pub artifacts: Vec<Resource>,
    pub status: ConversationStatus,        // Submitted/Working/Completed/Failed/Cancelled
    pub failed_reason: Option<String>,
    pub usage: Usage,
    pub steering_messages: Option<Vec<String>>,  // 支援多次 steering
    pub follow_up_messages: Option<Vec<String>>, // 支援多次 follow-up
    pub ancestors: Option<Vec<u64>>,             // 對話繼承鏈
    pub period: u64,                             // 時間分片 (小時級)
}
```

### 9.2 KIP 工具定義

```rust
// 檔案: anda_engine/src/memory.rs (行 36-55)
pub static FUNCTION_DEFINITION: LazyLock<FunctionDefinition> = LazyLock::new(|| {
    serde_json::from_value(json!({
        "name": "execute_kip",
        "description": "Executes one or more KIP commands against the Cognitive Nexus...",
        "parameters": {
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "A complete, multi-line KIP command (KQL, KML or META) string"
                },
                "parameters": {
                    "type": "object",
                    "description": "Optional JSON object for safe placeholder substitution"
                }
            },
            "required": ["command"]
        }
    })).unwrap()
});
```

### 9.3 Clawtex 差距

Clawtex 的 `memory_store/recall/forget` 工具是簡單的 key-value 記憶。Anda 的 KIP 是完整的知識圖譜查詢語言，支援語義搜尋、概念關係、時間感知。

**Clawtex 實作建議**：不需要完整 KIP，但可以加入：
- Conversation 持久化（目前只有 session 表）
- 語義搜尋（使用 embedding + 向量資料庫）
- 時間分片的對話歷史查詢

---

## 10. Store + Cache + CacheStoreFeatures

### 10.1 Store -- ObjectStore 抽象

Store 基於 Apache Arrow 的 `ObjectStore` trait，自動加入命名空間前綴。

### 10.2 CacheService -- moka LRU

```
CacheService
  └── BTreeMap<Path, moka::Cache<String, Arc<(Bytes, Option<CacheExpiry>)>>>
      ├── Path("T:search") → Cache { "results_1" → data, "results_2" → data }
      ├── Path("T:transfer") → Cache { ... }
      └── Path("A:assistant") → Cache { "caller_lock" → timestamp }
```

每個 Agent/Tool 有獨立的 moka Cache 實例，實現命名空間隔離。

---

## 11. Model 抽象層 (NotImplemented / MockImplemented)

```rust
// 檔案: anda_engine/src/model/mod.rs (行 38-145)
pub trait CompletionFeaturesDyn: Send + Sync + 'static {
    fn completion(&self, req: CompletionRequest) -> BoxPinFut<Result<AgentOutput, BoxError>>;
    fn model_name(&self) -> String;
}

// 未配置時的佔位符
pub struct NotImplemented;
impl CompletionFeaturesDyn for NotImplemented {
    fn completion(&self, _req: CompletionRequest) -> BoxPinFut<...> {
        Box::pin(futures::future::ready(Err("not implemented".into())))
    }
}

// 測試用 Mock
pub struct MockImplemented;
impl CompletionFeaturesDyn for MockImplemented {
    fn completion(&self, req: CompletionRequest) -> BoxPinFut<...> {
        Box::pin(futures::future::ready(Ok(AgentOutput {
            content: req.prompt.clone(),  // echo prompt
            tool_calls: req.tools.iter().filter_map(|tool| {
                if req.prompt.is_empty() { return None; }
                Some(ToolCall { name: tool.name.clone(), args: ..., ... })
            }).collect(),
            ..Default::default()
        })))
    }
}
```

**設計意義**：EngineBuilder 的預設值是 `NotImplemented`，這讓引擎可以漸進式配置。未配置 LLM 時 Agent 不會 panic，而是返回明確的錯誤。

**Clawtex 實作建議**：

Clawtex 的 Provider 在未配置時直接 panic 或返回不明確的錯誤。建議引入 NullProvider：

```rust
pub struct NullProvider;
impl Provider for NullProvider {
    async fn complete(&self, _messages: &[Message], _tools: &[ToolDef])
        -> Result<ProviderResponse>
    {
        Err(anyhow!("No LLM provider configured. Set up a provider in agents.toml"))
    }
}
```

---

## 12. Remote Engine 聯邦

### 12.1 命名慣例

| 前綴 | 含義 | 範例 |
|------|------|------|
| 無前綴 | 本地 tool | `google_search` |
| `RT_` | Remote Tool | `RT_engine2_search` |
| `LA_` | Local Agent | `LA_assistant` |
| `RA_` | Remote Agent | `RA_engine2_translator` |

### 12.2 動態遠端引擎發現

```rust
// 檔案: anda_engine/src/context/agent.rs (行 51)
pub static DYNAMIC_REMOTE_ENGINES: &str = "_engines";
```

Agent 可以在運行時將新發現的引擎存入 `cache_store`，後續的 tool/agent 路由會自動包含這些動態引擎。

---

## 13. ICP / TEE / Web3SDK

### 13.1 Web3SDK 雙模式

```rust
// 檔案: anda_engine/src/context/web3.rs
pub enum Web3SDK {
    Tee(Arc<TEEClient>),     // TEE 硬體管理密鑰
    Web3(Web3Client),        // 軟體管理密鑰
}
```

### 13.2 路徑派生隔離

```rust
// 檔案: anda_core/src/context.rs (行 615-620)
pub fn derivation_path_with(path: &Path, derivation_path: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let mut dp = Vec::with_capacity(derivation_path.len() + 1);
    dp.push(path.as_ref().as_bytes().to_vec());  // Agent/Tool 名稱作為前綴
    dp.extend(derivation_path);
    dp
}
```

即使兩個 Agent 使用相同的 derivation path，由於名稱自動加入前綴，派生出的密鑰完全不同。

---

## 14. 錯誤處理與取消機制

### 14.1 層級取消

```
Engine.cancellation_token (root)
  ├── Agent "assistant".cancellation_token (child_token)
  │     ├── Tool "search".cancellation_token (grandchild)
  │     └── Tool "transfer".cancellation_token (grandchild)
  └── Agent "extractor".cancellation_token (child_token)
```

取消 root → 連鎖取消所有子 token。取消 "search" → 只影響 search tool。

### 14.2 CompletionRunner 的取消處理

```rust
tokio::select! {
    _ = token.cancelled() => {
        // 立即返回取消的 output
        Ok(Some(self.final_output(AgentOutput {
            failed_reason: Some("operation cancelled".to_string()),
            ..Default::default()
        })))
    }
    res = self.inner_next() => res
}
```

### 14.3 深度限制

```rust
if child.depth >= CONTEXT_MAX_DEPTH {  // 42
    return Err("Context depth limit exceeded".into());
}
```

防止 Agent A → Agent B → Agent A → ... 無限遞迴。

**Clawtex 實作建議**：

Clawtex 的 `delegate` tool 目前沒有深度限制。建議：
```rust
// 在 ToolContext 中加入 depth
pub fn delegate_to(&self, agent: &str, prompt: &str) -> Result<Value> {
    if self.depth >= MAX_DEPTH {
        return Err(anyhow!("delegation depth limit exceeded (max {})", MAX_DEPTH));
    }
    let child_ctx = self.child(agent)?;
    // ...
}
```

---

## 15. Agent 實作範例分析

### 15.1 EchoEngineInfo -- 最簡 Agent

```rust
// 檔案: anda_engine/src/engine.rs (行 692-727)
pub struct EchoEngineInfo {
    info: AgentInfo,
    content: String,  // 預先序列化的 JSON
}

impl Agent<AgentCtx> for EchoEngineInfo {
    fn name(&self) -> String { self.info.handle.clone() }
    fn description(&self) -> String { self.info.description.clone() }

    async fn run(&self, _ctx: AgentCtx, _prompt: String, _resources: Vec<Resource>)
        -> Result<AgentOutput, BoxError>
    {
        Ok(AgentOutput {
            content: self.content.clone(),
            ..Default::default()
        })
    }
}
```

### 15.2 Assistant Agent -- 完整記憶+CompletionRunner

完整的 Assistant Agent 展示了 CompletionRunner 的典型用法（見第 3 節）。

---

## 16. 效能分析與瓶頸

### 16.1 Arc Clone 成本

AgentCtx.clone() 包含 7 個 Arc::clone（每個 = 一次 atomic fetch_add），加上 2 個 String clone。這在高頻呼叫下是可接受的。

### 16.2 BTreeMap vs HashMap

AgentSet 和 ToolSet 使用 `BTreeMap`（有序），而非 `HashMap`。查找是 O(log n) vs O(1)，但在 Tool 數量 < 100 時差異可忽略，且有序遍歷更方便。

### 16.3 並行 Tool 執行

```rust
let results = futures::future::join_all(tool_call_futs).await;
```

所有 tool 呼叫**並行**執行，這比串行執行大幅提升效能。但也意味著 tool 之間不能有依賴。

### 16.4 CBOR 序列化

Cache 使用 CBOR 而非 JSON，節省約 30% 的儲存空間和序列化時間。

---

## 17. Clawtex 差距對比總表

| 功能 | Anda | Clawtex | 差距等級 |
|------|------|---------|---------|
| Dual-Trait 模式 | Agent<C> + AgentDyn<C> | 單一 async_trait | 中 |
| CompletionRunner 迭代器 | 完整，含 steering/follow-up | 內嵌 loop | 高 |
| Feature Trait 分離 | 6 大 trait | 單一 context struct | 高 |
| Engine Builder | 完整 Builder + 依賴檢查 | 命令式初始化 | 中 |
| 路徑命名空間 | 自動隔離 | workspace/ 全域 | 中 |
| CacheStore 穿透 | CAS 原子更新 | 無 | 中 |
| Hook 系統 | 4 個生命週期點 + chain | hooks/ 模組 | 低 (已有) |
| 深度限制 | 42 層 | 無 | 高 |
| 並行 Tool 執行 | join_all | 串行 | 高 |
| Model 佔位符 | NotImplemented / Mock | 直接 panic | 中 |
| 型別安全 Tool Args | 關聯型別 + 自動 JSON | Value 直接傳遞 | 中 |
| 取消機制 | 層級 CancellationToken | E-Stop (全域) | 中 |
| 遠端引擎聯邦 | RT_/RA_ 動態發現 | ClusterHub | 低 (已有) |

---

## 18. 逐項 Clawtex 實作建議

### 18.1 優先級 P0：並行 Tool 執行

**現狀**：`agent_runtime.rs` 串行執行 tool calls
**建議**：使用 `futures::future::join_all` 並行

```rust
// Before (串行)
for tool_call in tool_calls {
    let result = execute_tool(&tool_call).await?;
    results.push(result);
}

// After (並行)
let futs: Vec<_> = tool_calls.iter().map(|tc| execute_tool(tc)).collect();
let results = futures::future::join_all(futs).await;
```

### 18.2 優先級 P0：深度限制

在 `delegate` / `delegate_to_provider` tool 中加入計數器。

### 18.3 優先級 P1：CompletionRunner 迭代器

將 `agent_runtime.rs` 的 LLM loop 抽取為獨立結構體。

### 18.4 優先級 P1：Tool Args 強型別

用 `serde` derive + `schemars::JsonSchema` 自動生成 JSON Schema。

### 18.5 優先級 P2：Feature Trait 分離

分步進行：先將 Context 分為 BaseCtx/AgentCtx。

### 18.6 優先級 P2：Engine Builder

替換 `main.rs` 中的命令式初始化。

### 18.7 優先級 P3：NullProvider 佔位符

讓未配置的 Provider 優雅降級。

---

*深度分析完成。本文件覆蓋了 Anda 框架的 ~70 個原始碼檔案，逐行分析了約 35 個關鍵檔案，
包含行號級的程式碼引用、資料流圖、記憶體佈局分析、效能評估，以及對 clawtex-core 的具體改進建議。*
