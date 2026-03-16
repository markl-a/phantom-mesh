# Rig (rig.rs) 深度技術分析

> 分析版本: rig-core v0.32.0 (2026-03 clone)
> 分析目的: 從 clawtex-core 開發者視角，萃取可採納的架構模式與設計思路
> 分析深度: 逐行原始碼級，含資料流圖、錯誤傳播鏈、效能考量、完整程式碼片段

---

## 目錄

1. [專案結構](#1-專案結構)
2. [入口點與使用方式](#2-入口點與使用方式)
3. [核心架構 -- Agent 抽象](#3-核心架構----agent-抽象)
   - 3.1 [Agent 結構體與泛型設計](#31-agent-結構體與泛型設計)
   - 3.2 [Typestate Builder 完整解析](#32-typestate-builder-完整解析)
   - 3.3 [Agent-as-Tool 實作](#33-agent-as-tool-實作)
   - 3.4 [PromptHook 攔截系統](#34-prompthook-攔截系統)
4. [Completion/Chat 三層 Trait 抽象](#4-completionchat-三層-trait-抽象)
   - 4.1 [Prompt / Chat / Completion 分層](#41-prompt--chat--completion-分層)
   - 4.2 [CompletionModel Trait 與 Associated Types](#42-completionmodel-trait-與-associated-types)
   - 4.3 [CompletionRequest 統一格式](#43-completionrequest-統一格式)
   - 4.4 [CompletionRequestBuilder 的 _opt 模式](#44-completionrequestbuilder-的-_opt-模式)
   - 4.5 [TypedPrompt 結構化輸出](#45-typedprompt-結構化輸出)
5. [Provider 系統深度解析](#5-provider-系統深度解析)
   - 5.1 [Provider / Capabilities / Capable 型別階層](#51-provider--capabilities--capable-型別階層)
   - 5.2 [Client<Ext, H> 泛型結構](#52-clientext-h-泛型結構)
   - 5.3 [CompletionClient Trait 與 Agent 工廠](#53-completionclient-trait-與-agent-工廠)
   - 5.4 [OpenAI-Compatible 複用模式](#54-openai-compatible-複用模式)
   - 5.5 [DynClientBuilder 動態 Provider 註冊](#55-dynclientbuilder-動態-provider-註冊)
6. [Tool/Function Calling 完整系統](#6-toolfunction-calling-完整系統)
   - 6.1 [Tool Trait 設計哲學](#61-tool-trait-設計哲學)
   - 6.2 [ToolDyn 動態分發橋接](#62-tooldyn-動態分發橋接)
   - 6.3 [ToolSet 容器](#63-toolset-容器)
   - 6.4 [ToolEmbedding -- 可 RAG 的工具](#64-toolembedding----可-rag-的工具)
   - 6.5 [MCP 整合 (rmcp)](#65-mcp-整合-rmcp)
7. [ToolServer Actor 完整生命週期](#7-toolserver-actor-完整生命週期)
   - 7.1 [Spawn 階段](#71-spawn-階段)
   - 7.2 [Message 協定](#72-message-協定)
   - 7.3 [並行工具執行](#73-並行工具執行)
   - 7.4 [動態工具定義查詢 (Tool RAG)](#74-動態工具定義查詢-tool-rag)
   - 7.5 [Shutdown 與資源回收](#75-shutdown-與資源回收)
8. [Embedding 與 RAG 系統](#8-embedding-與-rag-系統)
   - 8.1 [EmbeddingModel Trait](#81-embeddingmodel-trait)
   - 8.2 [Embed Trait 與 TextEmbedder](#82-embed-trait-與-textembedder)
   - 8.3 [VectorStoreIndex Trait 與動態分發](#83-vectorstoreindex-trait-與動態分發)
   - 8.4 [Agent 的 RAG 資料流](#84-agent-的-rag-資料流)
9. [Streaming 實作深度解析](#9-streaming-實作深度解析)
   - 9.1 [RawStreamingChoice 事件模型](#91-rawstreamingchoice-事件模型)
   - 9.2 [StreamingCompletionResponse 狀態機](#92-streamingcompletionresponse-狀態機)
   - 9.3 [PauseControl 與 AbortHandle](#93-pausecontrol-與-aborthandle)
   - 9.4 [文字聚合與非連續處理](#94-文字聚合與非連續處理)
10. [Extractor 結構化抽取模式](#10-extractor-結構化抽取模式)
11. [Pipeline DAG 框架](#11-pipeline-dag-框架)
12. [Loaders 文件載入器系統](#12-loaders-文件載入器系統)
    - 12.1 [FileLoader 泛型迭代器設計](#121-fileloader-泛型迭代器設計)
    - 12.2 [PdfFileLoader 分頁處理](#122-pdffileloader-分頁處理)
    - 12.3 [EpubFileLoader](#123-epubfileloader)
13. [錯誤處理架構](#13-錯誤處理架構)
14. [型別安全機制總覽](#14-型別安全機制總覽)
15. [HTTP 客戶端與 Retry 策略](#15-http-客戶端與-retry-策略)
16. [OneOrMany 非空集合保證](#16-onoremany-非空集合保證)
17. [效能考量與瓶頸分析](#17-效能考量與瓶頸分析)
18. [與 clawtex-core 的完整差距對比](#18-與-clawtex-core-的完整差距對比)
19. [附錄: 關鍵檔案路徑索引](#19-附錄-關鍵檔案路徑索引)

---

## 1. 專案結構

### 1.1 Workspace 佈局

```
rig/
├── Cargo.toml                    # workspace root (resolver = "3", edition = "2024")
├── rig/
│   ├── rig-core/                 # 核心函式庫 (lib name = "rig")
│   │   ├── src/
│   │   │   ├── lib.rs            # 模組根
│   │   │   ├── agent/            # Agent 抽象
│   │   │   │   ├── mod.rs        # re-exports
│   │   │   │   ├── builder.rs    # AgentBuilder (typestate)
│   │   │   │   ├── completion.rs # Agent struct + trait impls
│   │   │   │   ├── tool.rs       # Agent impl Tool
│   │   │   │   └── prompt_request/
│   │   │   │       ├── hooks.rs      # PromptHook trait
│   │   │   │       └── streaming.rs  # StreamingPromptRequest
│   │   │   ├── client/           # Client trait 系統
│   │   │   │   ├── builder.rs        # DynClientBuilder
│   │   │   │   ├── completion.rs     # CompletionClient trait
│   │   │   │   ├── embeddings.rs     # EmbeddingsClient trait
│   │   │   │   ├── verify.rs         # 驗證 API key
│   │   │   │   └── ...
│   │   │   ├── completion/       # CompletionModel trait + request/message 型別
│   │   │   │   ├── mod.rs
│   │   │   │   ├── request.rs    # Prompt/Chat/Completion traits, CompletionRequest
│   │   │   │   └── message.rs    # Message, UserContent, AssistantContent
│   │   │   ├── embeddings/       # EmbeddingModel trait + builder + distance
│   │   │   │   ├── embedding.rs  # EmbeddingModel trait
│   │   │   │   ├── builder.rs    # EmbeddingsBuilder
│   │   │   │   ├── embed.rs      # Embed trait + TextEmbedder
│   │   │   │   └── tool.rs       # ToolSchema for embedding tools
│   │   │   ├── providers/        # 19 個內建 Provider 實作
│   │   │   ├── tool/             # Tool trait + ToolServer (actor pattern)
│   │   │   │   ├── mod.rs        # Tool, ToolDyn, ToolSet, McpTool
│   │   │   │   └── server.rs     # ToolServer actor + ToolServerHandle
│   │   │   ├── tools/            # 內建工具 (think tool)
│   │   │   ├── streaming.rs      # 統一串流抽象
│   │   │   ├── pipeline/         # DAG pipeline 框架
│   │   │   │   ├── mod.rs
│   │   │   │   ├── op.rs         # Op trait + Sequential + Map + Then
│   │   │   │   ├── try_op.rs     # TryOp trait
│   │   │   │   ├── parallel.rs   # parallel! macro
│   │   │   │   ├── conditional.rs # 條件分支
│   │   │   │   └── agent_ops.rs  # Lookup, Prompt ops
│   │   │   ├── extractor.rs      # 結構化資料抽取
│   │   │   ├── vector_store/     # VectorStoreIndex trait
│   │   │   ├── http_client/      # HTTP 客戶端 + retry + SSE
│   │   │   ├── loaders/          # 文件載入器 (file, pdf, epub)
│   │   │   ├── one_or_many.rs    # 非空集合型別
│   │   │   └── integrations/     # CLI chatbot, Discord bot
│   │   └── examples/             # 90+ 範例檔
│   └── rig-derive/               # proc macro (#[derive(Embed)], rig_tool)
├── rig-integrations/             # 外部整合 crate (15+)
└── archived/                     # 存檔的 crate
```

### 1.2 內建 Provider 列表 (19 個)

位於 `rig/rig-core/src/providers/`:

| Provider | 檔案/目錄 | 支援能力 |
|----------|----------|---------|
| OpenAI | `openai/` | Completion + Embedding + Image + Audio + Transcription |
| Anthropic | `anthropic/` | Completion + Streaming |
| Cohere | `cohere/` | Completion + Embedding |
| Gemini | `gemini/` | Completion + Streaming + Transcription |
| Ollama | `ollama.rs` | Completion (OpenAI-compat) |
| Groq | `groq.rs` | Completion (OpenAI-compat) |
| Mistral | `mistral/` | Completion + Embedding + Transcription |
| OpenRouter | `openrouter/` | Completion + Embedding |
| Together | `together/` | Completion + Embedding |
| HuggingFace | `huggingface/` | Completion + Image + Transcription |
| Azure | `azure.rs` | OpenAI-compat |
| DeepSeek | `deepseek.rs` | OpenAI-compat |
| Perplexity | `perplexity.rs` | OpenAI-compat |
| xAI | `xai/` | Completion |
| Hyperbolic | `hyperbolic.rs` | Completion + Image + Audio |
| VoyageAI | `voyageai/` | Embedding |
| Galadriel | `galadriel.rs` | OpenAI-compat |
| Moonshot | `moonshot.rs` | OpenAI-compat |
| Mira | `mira.rs` | OpenAI-compat |

> **Clawtex 對比**: clawtex-core 有 12+ providers (ollama, openai_compat, anthropic, openai, gemini, groq, chatgpt_backend, router, rotation, classifier, key_pool)。Rig 的 provider 數量是 clawtex 的 1.5 倍，但 clawtex 有 Rig 缺乏的 smart routing、rotation、classifier 等中繼層。

---

## 2. 入口點與使用方式

### 2.1 最簡範例 -- 完整資料流

```rust
use rig::client::{CompletionClient, ProviderClient};
use rig::completion::Prompt;
use rig::providers::openai;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // 1. 建立 Provider Client (讀取 OPENAI_API_KEY 環境變數)
    let client = openai::Client::from_env();

    // 2. 建立 Agent (model 綁定為 openai::CompletionModel)
    let comedian_agent = client
        .agent(openai::GPT_4O)       // -> AgentBuilder<openai::CompletionModel, (), NoToolConfig>
        .preamble("You are a comedian")  // 仍在 NoToolConfig 狀態
        .build();                     // -> Agent<openai::CompletionModel>

    // 3. 送出 Prompt
    let response = comedian_agent.prompt("Entertain me!").await?;
    //   3a. prompt() -> PromptRequest::from_agent()
    //   3b. IntoFuture -> build_completion_request()
    //     3b1. 收集 preamble, static_context, temperature 等
    //     3b2. 從 dynamic_context 做 VectorStoreIndex::top_n() (如有)
    //     3b3. 從 ToolServer 取得 ToolDefinition (透過 mpsc channel)
    //     3b4. 組裝 CompletionRequest
    //   3c. model.completion(request) -> HTTP POST to OpenAI API
    //   3d. 解析回應 -> CompletionResponse<openai::Response>
    //   3e. 如果回應含 ToolCall → call_tool → 繼續迴圈 (multi-turn)
    //   3f. 如果回應是 Message → 回傳 String

    println!("{response}");
    Ok(())
}
```

**完整資料流圖:**

```
User Code
  │
  ├─ client = openai::Client::from_env()
  │   └─ 讀取 OPENAI_API_KEY, 建立 reqwest::Client
  │
  ├─ client.agent("gpt-4o")
  │   └─ CompletionClient::agent()
  │       └─ AgentBuilder::new(CompletionModel::make(&client, "gpt-4o"))
  │
  ├─ .preamble("...").build()
  │   └─ AgentBuilder<M, (), NoToolConfig>::build()
  │       ├─ ToolServer::new().run()     ← spawn background actor
  │       │   └─ tokio::spawn(async { rx.recv().await })
  │       │   └─ 回傳 ToolServerHandle(tx)
  │       └─ Agent { model: Arc<M>, tool_server_handle, ... }
  │
  └─ agent.prompt("Entertain me!").await
      └─ Prompt::prompt() → PromptRequest
          └─ IntoFuture::into_future()
              └─ build_completion_request()
                  ├─ dynamic_context.read() → VectorStoreIndex::top_n()
                  ├─ tool_server_handle.get_tool_defs() → mpsc → ToolServer
                  └─ model.completion_request(prompt)
                      .preamble(...)
                      .documents(...)
                      .tools(...)
                      .build() → CompletionRequest
                          │
                          └─ model.completion(request)
                              └─ HTTP POST https://api.openai.com/v1/...
                                  └─ CompletionResponse { choice, usage, raw_response }
                                      │
                                      ├─ [Text] → return String
                                      └─ [ToolCall] → tool_server.call_tool()
                                          → spawn task → ToolSet::call()
                                          → 回傳結果 → 下一輪 completion
```

---

## 3. 核心架構 -- Agent 抽象

### 3.1 Agent 結構體與泛型設計

**檔案:** `rig/rig-core/src/agent/completion.rs`

```rust
#[derive(Clone)]
#[non_exhaustive]
pub struct Agent<M, P = ()>
where
    M: CompletionModel,
    P: PromptHook<M>,
{
    pub name: Option<String>,
    pub description: Option<String>,
    pub model: Arc<M>,                            // 共享所有權
    pub preamble: Option<String>,
    pub static_context: Vec<Document>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    pub additional_params: Option<serde_json::Value>,
    pub tool_server_handle: ToolServerHandle,      // mpsc Sender
    pub dynamic_context: DynamicContextStore,       // Arc<RwLock<Vec<...>>>
    pub tool_choice: Option<ToolChoice>,
    pub default_max_turns: Option<usize>,
    pub hook: Option<P>,
    pub output_schema: Option<schemars::Schema>,
}
```

**設計剖析:**

1. **雙泛型參數 `M` + `P`**: `M: CompletionModel` 在編譯期綁定模型實作，避免 dyn dispatch 開銷。`P: PromptHook<M>` 預設為 `()` (no-op hook)，需要攔截時可替換為自訂 hook。

2. **`#[non_exhaustive]`**: 防止外部 crate 直接建構 Agent，強制使用 `AgentBuilder`。未來新增欄位不會破壞 API。

3. **`Arc<M>` 而非 `M`**: CompletionModel 需要 Clone，但 Arc 確保多個 Agent 可以共享同一個 model 實例而不需深複製。

4. **`DynamicContextStore` 型別別名**:
   ```rust
   pub type DynamicContextStore = Arc<
       TokioRwLock<Vec<(usize, Box<dyn VectorStoreIndexDyn + Send + Sync>)>>
   >;
   ```
   使用 `Arc<RwLock<...>>` 因為 Agent 是 Clone 的，多個 clone 共享同一個動態上下文集合，且需要支援執行期新增上下文源。

5. **`ToolServerHandle` 而非直接持有工具**: 解耦工具管理的生命週期，多個 Agent 可共享同一個 ToolServer。

**build_completion_request 函式 -- Agent 的核心邏輯:**

```rust
pub(crate) async fn build_completion_request<M: CompletionModel>(
    model: &Arc<M>,
    prompt: Message,
    chat_history: Vec<Message>,
    preamble: Option<&str>,
    static_context: &[Document],
    temperature: Option<f64>,
    max_tokens: Option<u64>,
    additional_params: Option<&serde_json::Value>,
    tool_choice: Option<&ToolChoice>,
    tool_server_handle: &ToolServerHandle,
    dynamic_context: &DynamicContextStore,
    output_schema: Option<&schemars::Schema>,
) -> Result<CompletionRequestBuilder<M>, CompletionError> {
    // 1. 從 prompt 或 chat_history 最後一條訊息提取 RAG 文字
    let rag_text = prompt.rag_text();
    let rag_text = rag_text.or_else(|| {
        chat_history.iter().rev().find_map(|msg| msg.rag_text())
    });

    // 2. 建構基礎 CompletionRequest
    let completion_request = model.completion_request(prompt)
        .messages(chat_history)
        .temperature_opt(temperature)      // 使用 _opt 變體，None 不設定
        .max_tokens_opt(max_tokens)
        .additional_params_opt(additional_params.cloned())
        .output_schema_opt(output_schema.cloned())
        .documents(static_context.to_vec());

    // 3. 條件設定 preamble 和 tool_choice
    let completion_request = if let Some(preamble) = preamble {
        completion_request.preamble(preamble.to_owned())
    } else { completion_request };

    // 4. 如果有 RAG 文字，從 dynamic_context 取回最相關文件
    let result = match &rag_text {
        Some(text) => {
            // 4a. 對每個 dynamic_context 的 VectorStoreIndex 呼叫 top_n()
            let fetched_context = stream::iter(dynamic_context.read().await.iter())
                .then(|(num_sample, index)| async {
                    let req = VectorSearchRequest::builder()
                        .query(text)
                        .samples(*num_sample as u64)
                        .build()?;
                    index.top_n(req).await
                })
                .try_fold(vec![], |mut acc, docs| async {
                    acc.extend(docs.into_iter().map(|(_, id, doc)| Document {
                        id, text: serde_json::to_string_pretty(&doc).unwrap(), ...
                    }));
                    Ok(acc)
                }).await?;

            // 4b. 從 ToolServer 取得動態 + 靜態工具定義
            let tooldefs = tool_server_handle
                .get_tool_defs(Some(text.to_string())).await?;

            completion_request.documents(fetched_context).tools(tooldefs)
        }
        None => {
            let tooldefs = tool_server_handle.get_tool_defs(None).await?;
            completion_request.tools(tooldefs)
        }
    };
    Ok(result)
}
```

**關鍵效能考量:**
- `dynamic_context.read().await` 使用的是 Tokio RwLock，讀取鎖不會阻塞其他讀取者
- `stream::iter(...).then(...).try_fold(...)` 是**順序**執行的（非並行），多個 VectorStore 會串列查詢
- 每次 prompt 都會重新查詢 ToolServer 取得工具定義，如果工具沒變化這會產生不必要的 channel 通訊

> **Clawtex 實作建議**: clawtex-core 目前把所有 24 個工具全量傳送給 LLM。可以參考 Rig 的 `build_completion_request` 邏輯，在 `agent_runtime.rs` 的 completion 構建階段加入 dynamic_context 查詢，根據 prompt 內容只傳送最相關的 5-8 個工具定義，預估可節省 40-60% 的 token 消耗。

---

### 3.2 Typestate Builder 完整解析

**檔案:** `rig/rig-core/src/agent/builder.rs`

Typestate 是 Rust 中一種利用泛型型別參數來在編譯期追蹤物件狀態的模式。Rig 的 AgentBuilder 使用三個 typestate marker:

```rust
// === 三個狀態 marker type ===

/// 初始狀態: 尚未配置工具
#[derive(Default)]
pub struct NoToolConfig;

/// 已透過 builder API 新增工具
pub struct WithBuilderTools {
    static_tools: Vec<String>,         // 靜態工具名稱列表
    tools: ToolSet,                     // 實際工具集合
    dynamic_tools: Vec<(usize, Box<dyn VectorStoreIndexDyn + Send + Sync>)>,
}

/// 已注入外部 ToolServerHandle
pub struct WithToolServerHandle {
    handle: ToolServerHandle,
}

// === Builder 主結構 ===
pub struct AgentBuilder<M, P = (), ToolState = NoToolConfig>
where
    M: CompletionModel,
    P: PromptHook<M>,
{
    model: M,
    preamble: Option<String>,
    static_context: Vec<Document>,
    // ... 其他欄位
    tool_state: ToolState,  // ← 這是 typestate 核心
    hook: Option<P>,
}
```

**狀態轉移圖:**

```
                       AgentBuilder<M, P, NoToolConfig>
                       ┌─────────────────────────────┐
                       │  .name(), .preamble(),       │
                       │  .temperature(), .context(), │
                       │  .dynamic_context(),         │
                       │  .max_tokens(), .hook()      │
                       └──────┬──────────┬────────────┘
                              │          │
              .tool() / .tools()  .tool_server_handle()
              .dynamic_tools()     │
              .rmcp_tool/tools()   │
                              │          │
                              v          v
  AgentBuilder<M, P, WithBuilderTools>    AgentBuilder<M, P, WithToolServerHandle>
  ┌──────────────────────────────┐        ┌────────────────────────────────────┐
  │  .tool() (繼續加工具)         │        │  只能 .build()                     │
  │  .tools()                    │        │  tool-adding 方法不存在             │
  │  .rmcp_tools()               │        └──────────────┬─────────────────────┘
  │  .dynamic_tools()            │                       │
  └──────────────┬───────────────┘                       │
                 │                                       │
                 │ .build()                              │ .build()
                 │                                       │
                 v                                       v
            ToolServer::new()                    直接使用 handle
              .static_tool_names(names)
              .add_tools(tools)
              .add_dynamic_tools(dyn_tools)
              .run()  → ToolServerHandle
                 │
                 v
              Agent<M, P>
```

**編譯期強制的不變量 (invariants):**

1. **互斥**: `NoToolConfig` 可以轉到 `WithBuilderTools` 或 `WithToolServerHandle`，但二者不能共存
2. **不可逆**: 一旦轉到 `WithToolServerHandle`，就不能再呼叫 `.tool()`
3. **Builder 消費**: 狀態轉換方法消費 `self`，回傳新型別的 builder，防止重複使用舊 builder

**狀態轉換的實作細節 -- NoToolConfig -> WithBuilderTools:**

```rust
impl<M, P> AgentBuilder<M, P, NoToolConfig>
where
    M: CompletionModel,
    P: PromptHook<M>,
{
    pub fn tool(self, tool: impl Tool + 'static) -> AgentBuilder<M, P, WithBuilderTools> {
        let toolname = tool.name();
        AgentBuilder {
            // 逐欄位搬移 (move), 不是複製
            name: self.name,
            description: self.description,
            model: self.model,
            preamble: self.preamble,
            static_context: self.static_context,
            additional_params: self.additional_params,
            max_tokens: self.max_tokens,
            dynamic_context: self.dynamic_context,
            temperature: self.temperature,
            tool_choice: self.tool_choice,
            default_max_turns: self.default_max_turns,
            hook: self.hook,
            output_schema: self.output_schema,
            // 唯一不同: tool_state 型別改變
            tool_state: WithBuilderTools {
                static_tools: vec![toolname],
                tools: ToolSet::from_tools(vec![tool]),
                dynamic_tools: vec![],
            },
        }
    }
}
```

**注意: 這段程式碼有大量欄位搬移的重複**。每個狀態轉換方法都要手動列出所有欄位。Rig 在 `builder.rs` 中有超過 600 行，其中大部分是這種重複搬移。

**WithBuilderTools 的 build 實作:**

```rust
impl<M, P> AgentBuilder<M, P, WithBuilderTools>
where
    M: CompletionModel,
    P: PromptHook<M>,
{
    pub fn build(self) -> Agent<M, P> {
        // 在這裡才 spawn ToolServer
        let tool_server_handle = ToolServer::new()
            .static_tool_names(self.tool_state.static_tools)
            .add_tools(self.tool_state.tools)
            .add_dynamic_tools(self.tool_state.dynamic_tools)
            .run();  // ← spawn background tokio task

        Agent {
            model: Arc::new(self.model),
            tool_server_handle,
            dynamic_context: Arc::new(RwLock::new(self.dynamic_context)),
            // ...
        }
    }
}
```

> **Clawtex 實作建議**: clawtex-core 的 Agent 配置完全透過 TOML，缺乏編譯期驗證。如果要在 Rust API 層面建立 Agent（例如程式化建立子代理），可引入簡化版的 typestate。但考慮到 clawtex 的設定驅動架構，更實際的做法是在 TOML 解析時加入結構化驗證（serde validation），確保必要欄位齊全且不互斥。typestate 在 clawtex 的內部 API 中最適合用於 `CompletionRequest` 的建構。

---

### 3.3 Agent-as-Tool 實作

**檔案:** `rig/rig-core/src/agent/tool.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentToolArgs {
    /// The prompt for the agent to call.
    prompt: String,
}

impl<M: CompletionModel> Tool for Agent<M> {
    const NAME: &'static str = "agent_tool";

    type Error = PromptError;
    type Args = AgentToolArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        let description = format!(
            "Prompt a sub-agent to do a task for you.\n\
             Agent name: {name}\n\
             Agent description: {description}\n\
             Agent system prompt: {sysprompt}",
            name = self.name(),
            description = self.description.clone().unwrap_or_default(),
            sysprompt = self.preamble.clone().unwrap_or_default()
        );
        ToolDefinition {
            name: <Self as Tool>::name(self),
            description,
            parameters: serde_json::to_value(schema_for!(AgentToolArgs)).unwrap(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.prompt(args.prompt).await
    }

    fn name(&self) -> String {
        self.name.clone().unwrap_or_else(|| Self::NAME.to_string())
    }
}
```

**設計剖析:**

1. **泛型 impl**: `impl<M: CompletionModel> Tool for Agent<M>` -- 任何 CompletionModel 的 Agent 都自動成為 Tool
2. **Tool definition 包含 Agent 的 preamble**: 讓上層 Agent 了解子代理的能力
3. **name() 覆寫**: 如果 Agent 有名稱，工具名就是 Agent 名稱而非固定的 "agent_tool"
4. **遞迴風險**: Agent A 把 Agent B 當工具，B 又把 A 當工具 → 無限遞迴。Rig 靠 `default_max_turns` 限制深度

**使用範例:**

```rust
let research_agent = client.agent("gpt-4o")
    .name("researcher")
    .description("Researches topics deeply")
    .preamble("You are a research assistant...")
    .build();

let orchestrator = client.agent("gpt-4o")
    .preamble("You coordinate other agents")
    .tool(research_agent)  // Agent 直接當 Tool 傳入
    .build();

// orchestrator 可以呼叫 "researcher" 工具，
// 它會自動 prompt research_agent 並回傳結果
```

> **Clawtex 實作建議**: clawtex-core 已有 `delegate` 和 `delegate_to_provider` 工具，但它們是獨立的 tool impl，不是 Agent 自身的 trait impl。可以考慮在 `AgentConfig` 上加一個 `as_tool()` 方法，自動產生對應的 `delegate` 工具定義，讓多代理組合更自然。特別是在 Hands workflow 中，不同 phase 的 agent 可以自動暴露為上一個 phase 的可用工具。

---

### 3.4 PromptHook 攔截系統

**檔案:** `rig/rig-core/src/agent/prompt_request/hooks.rs`

```rust
pub trait PromptHook<M>: Clone + WasmCompatSend + WasmCompatSync
where
    M: CompletionModel,
{
    // 7 個攔截點:

    // 1. 即將送出 completion 請求
    fn on_completion_call(&self, _prompt: &Message, _history: &[Message])
        -> impl Future<Output = HookAction> + WasmCompatSend
    { async { HookAction::cont() } }

    // 2. 收到 completion 回應
    fn on_completion_response(&self, _prompt: &Message,
        _response: &CompletionResponse<M::Response>)
        -> impl Future<Output = HookAction> + WasmCompatSend
    { async { HookAction::cont() } }

    // 3. 即將呼叫工具 (可拒絕)
    fn on_tool_call(&self, _tool_name: &str, _tool_call_id: Option<String>,
        _internal_call_id: &str, _args: &str)
        -> impl Future<Output = ToolCallHookAction> + WasmCompatSend
    { async { ToolCallHookAction::cont() } }

    // 4. 工具回傳結果
    fn on_tool_result(&self, _tool_name: &str, _tool_call_id: Option<String>,
        _internal_call_id: &str, _args: &str, _result: &str)
        -> impl Future<Output = HookAction> + WasmCompatSend
    { async { HookAction::cont() } }

    // 5. 串流文字 delta
    fn on_text_delta(&self, _text_delta: &str, _aggregated_text: &str)
        -> impl Future<Output = HookAction> + Send
    { async { HookAction::cont() } }

    // 6. 串流工具呼叫 delta
    fn on_tool_call_delta(&self, _tool_call_id: &str, _internal_call_id: &str,
        _tool_name: Option<&str>, _tool_call_delta: &str)
        -> impl Future<Output = HookAction> + Send
    { async { HookAction::cont() } }

    // 7. 串流完成
    fn on_stream_completion_response_finish(&self, _prompt: &Message,
        _response: &M::StreamingResponse)
        -> impl Future<Output = HookAction> + Send
    { async { HookAction::cont() } }
}

// `()` 自動實作 PromptHook (所有方法都用預設 Continue)
impl<M> PromptHook<M> for () where M: CompletionModel {}
```

**控制流 enum:**

```rust
pub enum HookAction {
    Continue,                    // 繼續正常執行
    Terminate { reason: String }, // 終止 agent 迴圈
}

pub enum ToolCallHookAction {
    Continue,                    // 允許工具執行
    Skip { reason: String },     // 跳過工具，回傳 reason 作為工具結果
    Terminate { reason: String }, // 終止整個 agent 迴圈
}
```

**錯誤傳播路徑:**
```
ToolCallHookAction::Terminate { reason }
  → PromptError::PromptCancelled { chat_history, reason }
    → 呼叫者收到 Err(PromptError)
```

> **Clawtex 實作建議**: clawtex-core 的 `HookRunner` 目前在 `pre_completion` / `post_completion` 兩個點執行。Rig 的 7 個攔截點設計更細緻。建議在 clawtex 的 `agent_runtime.rs` 中加入以下攔截點: (1) `pre_tool_call` -- 可實現 approval gate 的前置檢查，替代目前獨立的 approval.rs; (2) `post_tool_result` -- 可加入工具結果的自動驗證/清理; (3) `on_text_delta` -- 讓 Telegram bot 即時串流回應。`ToolCallHookAction::Skip` 特別有價值 -- 可以在不終止整個迴圈的情況下拒絕危險的工具呼叫。

---

## 4. Completion/Chat 三層 Trait 抽象

### 4.1 Prompt / Chat / Completion 分層

**檔案:** `rig/rig-core/src/completion/request.rs`

Rig 設計了三層遞進的抽象，每層提供不同程度的控制:

```rust
// 第一層: 最高層 -- 一問一答
pub trait Prompt: WasmCompatSend + WasmCompatSync {
    fn prompt(&self, prompt: impl Into<Message> + WasmCompatSend)
        -> impl IntoFuture<Output = Result<String, PromptError>>;
}

// 第二層: 中間層 -- 帶歷史紀錄
pub trait Chat: WasmCompatSend + WasmCompatSync {
    fn chat(&self, prompt: impl Into<Message>, history: Vec<Message>)
        -> impl IntoFuture<Output = Result<String, PromptError>>;
}

// 第三層: 最底層 -- 取得 Builder 做精細控制
pub trait Completion<M: CompletionModel> {
    fn completion(&self, prompt: impl Into<Message>, history: Vec<Message>)
        -> impl Future<Output = Result<CompletionRequestBuilder<M>, CompletionError>>;
}

// 結構化輸出層
pub trait TypedPrompt: WasmCompatSend + WasmCompatSync {
    type TypedRequest<'a, T>: IntoFuture<Output = Result<T, StructuredOutputError>>
    where Self: 'a, T: JsonSchema + DeserializeOwned + WasmCompatSend + 'a;

    fn prompt_typed<T>(&self, prompt: impl Into<Message>)
        -> Self::TypedRequest<'_, T>
    where T: JsonSchema + DeserializeOwned + WasmCompatSend;
}
```

**使用場景對照:**

| 場景 | 使用 Trait | 範例 |
|------|-----------|------|
| 簡單一問一答 | `Prompt` | `agent.prompt("hi").await?` |
| 帶歷史記錄 | `Chat` | `agent.chat("hi", history).await?` |
| 自訂參數覆寫 | `Completion` | `agent.completion("hi", history).await?.temperature(0.9).send().await?` |
| 結構化回應 | `TypedPrompt` | `let person: Person = agent.prompt_typed("...").await?` |

**refining_impl_trait 技巧:**

Agent 對 `Prompt` trait 的實作使用了 `#[allow(refining_impl_trait)]`，讓 `.prompt()` 回傳更具體的 `PromptRequest` 而非 trait 定義的泛型 future:

```rust
#[allow(refining_impl_trait)]
impl<M, P> Prompt for Agent<M, P>
where M: CompletionModel, P: PromptHook<M> + 'static,
{
    fn prompt(&self, prompt: impl Into<Message>) -> PromptRequest<'_, Standard, M, P> {
        PromptRequest::from_agent(self, prompt)
    }
}
```

這讓使用者可以在 `.prompt()` 後鏈式呼叫 `.max_turns()` 等方法:
```rust
agent.prompt("question").max_turns(5).await?;
```

> **Clawtex 實作建議**: clawtex-core 的 Provider trait 目前只有一個 `complete()` 方法。建議分離為類似的三層: (1) `quick_prompt(&str) -> String` 用於簡單工具內部的 LLM 呼叫; (2) `chat(prompt, history) -> String` 用於 Telegram 對話; (3) `completion(request) -> Response` 用於 agent_runtime 的精細控制。這種分層可以讓不同呼叫者選擇適當的抽象層級。

---

### 4.2 CompletionModel Trait 與 Associated Types

```rust
pub trait CompletionModel: Clone + WasmCompatSend + WasmCompatSync {
    /// 每個 provider 的原始回應型別
    type Response: WasmCompatSend + WasmCompatSync + Serialize + DeserializeOwned;

    /// 串流回應的原始型別
    type StreamingResponse: Clone + Unpin + WasmCompatSend + WasmCompatSync
        + Serialize + DeserializeOwned + GetTokenUsage;

    /// 建立此 model 的 client 型別
    type Client;

    /// 工廠方法: 從 client + model name 建立 model
    fn make(client: &Self::Client, model: impl Into<String>) -> Self;

    /// 送出 completion 請求
    fn completion(&self, request: CompletionRequest)
        -> impl Future<Output = Result<CompletionResponse<Self::Response>, CompletionError>>;

    /// 送出串流 completion 請求
    fn stream(&self, request: CompletionRequest)
        -> impl Future<Output = Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError>>;

    /// 提供 builder (有預設實作)
    fn completion_request(&self, prompt: impl Into<Message>) -> CompletionRequestBuilder<Self> {
        CompletionRequestBuilder::new(self.clone(), prompt)
    }
}
```

**associated types 的型別安全保證:**

- `Response` 讓每個 provider 保有自己的原始回應結構，不需要先轉為通用格式再傳遞
- `StreamingResponse` 有 `GetTokenUsage` bound，確保串流結束時能取得 token 用量
- `Client` 把 client 型別綁定到 model，`make()` 方法確保只能從正確的 client 建立 model

> **Clawtex 實作建議**: clawtex-core 的 `Provider` trait 使用 `Box<dyn>` 動態分發。如果想要 Rig 的編譯期型別安全，但又要保持 TOML 驅動的動態性，可以考慮在「內部 API」使用泛型 trait，然後在「TOML 設定層」使用 trait object 做動態分發。核心 Provider trait 可加入 `type Response` associated type，讓各 provider 的回應型別更明確。

---

### 4.3 CompletionRequest 統一格式

```rust
pub struct CompletionRequest {
    pub model: Option<String>,             // 可覆寫模型
    pub preamble: Option<String>,          // system prompt
    pub chat_history: OneOrMany<Message>,   // 至少有一條 (prompt)
    pub documents: Vec<Document>,           // RAG 文件
    pub tools: Vec<ToolDefinition>,         // 工具定義
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    pub tool_choice: Option<ToolChoice>,
    pub additional_params: Option<serde_json::Value>,  // provider 特有參數
    pub output_schema: Option<schemars::Schema>,       // 結構化輸出 schema
}
```

**重要方法:**

```rust
impl CompletionRequest {
    /// 從 output_schema 提取名稱 (用於 OpenAI structured outputs)
    pub fn output_schema_name(&self) -> Option<String> { ... }

    /// 將 documents 轉為 Message (大多數 provider 不直接接受 documents)
    pub fn normalized_documents(&self) -> Option<Message> {
        if self.documents.is_empty() { return None; }
        let messages = self.documents.iter()
            .map(|doc| UserContent::document(doc.to_string(), Some(DocumentMediaType::TXT)))
            .collect::<Vec<_>>();
        Some(Message::User { content: OneOrMany::many(messages).unwrap() })
    }
}
```

每個 provider 負責將 `CompletionRequest` 轉換為自家 API 的格式。例如 Anthropic 需要拆分 system message，OpenAI 需要將 documents 插入 chat history。

> **Clawtex 實作建議**: clawtex-core 各 provider 直接操作 JSON。引入類似 `CompletionRequest` 的標準化中間格式，可以讓 `router.rs` 和 `rotation.rs` 不需要知道底層 provider 的具體格式。每個 provider 只需實作 `From<CompletionRequest> for ProviderSpecificRequest`。

---

### 4.4 CompletionRequestBuilder 的 _opt 模式

Rig 的 Builder 為每個 Option 欄位提供兩個方法:

```rust
impl<M: CompletionModel> CompletionRequestBuilder<M> {
    // 標準設定方法: 直接設值
    pub fn temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    // _opt 變體: 接受 Option，None 不設定
    pub fn temperature_opt(mut self, temperature: Option<f64>) -> Self {
        self.temperature = temperature;
        self
    }

    // additional_params 有特殊合併邏輯
    pub fn additional_params(mut self, params: serde_json::Value) -> Self {
        match self.additional_params {
            Some(existing) => {
                self.additional_params = Some(json_utils::merge(existing, params));
            }
            None => { self.additional_params = Some(params); }
        }
        self
    }

    // _opt 變體: 直接覆寫
    pub fn additional_params_opt(mut self, params: Option<serde_json::Value>) -> Self {
        self.additional_params = params;
        self
    }
}
```

**`_opt` 模式的價值**: 當 Agent 的 temperature 是 `Option<f64>` 時，不用寫:
```rust
// 不需要這樣:
let builder = if let Some(t) = temperature {
    builder.temperature(t)
} else { builder };

// 直接:
builder.temperature_opt(temperature)
```

> **Clawtex 實作建議**: 在 clawtex-core 的任何 builder pattern 中採用 `_opt` 命名慣例。特別是在建構 API 請求時，從 TOML 讀取的 Option 欄位可以直接透傳，減少大量的 `if let Some` 分支。

---

### 4.5 TypedPrompt 結構化輸出

```rust
// 使用範例:
#[derive(Debug, Deserialize, JsonSchema)]
struct WeatherForecast {
    city: String,
    temperature_f: f64,
    conditions: String,
}

let agent = client.agent("gpt-4o").build();

// 型別推導自動生成 JSON Schema
let forecast: WeatherForecast = agent
    .prompt_typed("What's the weather in NYC?")
    .max_turns(3)
    .await?;
```

背後使用 `schemars::schema_for!()` 自動從 Rust struct 生成 JSON Schema，然後透過 `output_schema` 欄位傳給支援 structured outputs 的 provider。

---

## 5. Provider 系統深度解析

### 5.1 Provider / Capabilities / Capable 型別階層

Rig 的 Provider 系統使用三層型別來宣告能力:

```rust
// 1. Provider marker trait -- 標記某個 Ext 型別是 provider
pub trait Provider {
    type Builder: ProviderBuilder;
    const VERIFY_PATH: &'static str;
}

// 2. Capabilities trait -- 宣告 provider 支援的能力
pub trait Capabilities {
    type Completion;       // Capable<T> 或 Nothing
    type Embeddings;       // Capable<T> 或 Nothing
    type Transcription;    // Capable<T> 或 Nothing
    type ModelListing;     // Capable<T> 或 Nothing
    type ImageGeneration;  // Capable<T> 或 Nothing
    type AudioGeneration;  // Capable<T> 或 Nothing
}

// 3. Capability marker
pub struct Capable<T>(PhantomData<T>);  // 表示「有此能力」
pub struct Nothing;                      // 表示「無此能力」

pub trait Capability {
    const CAPABLE: bool;
}
impl<T> Capability for Capable<T> { const CAPABLE: bool = true; }
impl Capability for Nothing { const CAPABLE: bool = false; }
```

**以 OpenAI 為例:**

```rust
pub struct OpenAIResponsesExt { /* ... */ }

impl Provider for OpenAIResponsesExt {
    type Builder = OpenAIResponsesBuilder;
    const VERIFY_PATH: &'static str = "/v1/models";
}

impl Capabilities for OpenAIResponsesExt {
    type Completion = Capable<ResponsesCompletionModel>;
    type Embeddings = Capable<EmbeddingModel>;
    type Transcription = Capable<TranscriptionModel>;
    type ModelListing = Nothing;
    type ImageGeneration = Capable<ImageGenerationModel>;
    type AudioGeneration = Capable<AudioGenerationModel>;
}
```

**編譯期能力檢查:** 如果嘗試在不支援 embedding 的 provider 上呼叫 `embedding_model()`，編譯器會報錯。

### 5.2 Client<Ext, H> 泛型結構

```rust
pub struct Client<Ext, H = reqwest::Client> {
    base_url: String,
    http_client: H,     // 可注入不同 HTTP 客戶端
    ext: Ext,           // Provider 特定的擴展資料
    // ...
}
```

- `Ext` 是 provider 的 marker type (如 `OpenAIResponsesExt`)
- `H` 是 HTTP 客戶端型別，預設 `reqwest::Client`，可替換為 `reqwest_middleware::ClientWithMiddleware`

### 5.3 CompletionClient Trait 與 Agent 工廠

```rust
pub trait CompletionClient {
    type CompletionModel: CompletionModel<Client = Self>;

    fn completion_model(&self, model: impl Into<String>) -> Self::CompletionModel {
        Self::CompletionModel::make(self, model)
    }

    fn agent(&self, model: impl Into<String>) -> AgentBuilder<Self::CompletionModel> {
        AgentBuilder::new(self.completion_model(model))
    }

    fn extractor<T>(&self, model: impl Into<String>) -> ExtractorBuilder<Self::CompletionModel, T>
    where T: JsonSchema + for<'a> Deserialize<'a> + Serialize + Send + Sync {
        ExtractorBuilder::new(self.completion_model(model))
    }
}
```

**注意 `Client = Self` 約束**: CompletionModel 的 Client associated type 必須是這個 CompletionClient 自身，形成封閉迴路。

### 5.4 OpenAI-Compatible 複用模式

Ollama, Groq, DeepSeek 等都是 OpenAI-compatible。它們的實作極簡:

```rust
// groq.rs (簡化)
const GROQ_API_BASE_URL: &str = "https://api.groq.com/openai/v1";

pub type Client<H = reqwest::Client> = openai::CompletionsClient<H>;
// 直接 type alias 到 openai 的 client，只換 base_url
```

這種模式讓新增 OpenAI-compatible provider 只需幾十行程式碼。

### 5.5 DynClientBuilder 動態 Provider 註冊

雖然已 deprecated，但概念值得理解:

```rust
pub struct DynClientBuilder(HashMap<String, ProviderFactory>);

impl DynClientBuilder {
    pub fn new() -> Self {
        Self::default().register_all()  // 註冊所有 18 個內建 provider
    }

    pub fn agent(&self, provider_name: &str, model: Models)
        -> Result<AgentBuilder<CompletionModelHandle<'_>>, Error>
    {
        let client = self.0.get(&key)?.from_env()?;
        let completion = client.as_completion()?;
        Ok(completion.agent(&model.to_string()))
    }
}
```

**AnyClient vtable 模式:**

```rust
struct AnyClientVTable {
    as_completion: fn(&dyn Any) -> Option<&&dyn CompletionClientDyn>,
    as_embedding: fn(&dyn Any) -> Option<&&dyn EmbeddingsClientDyn>,
    // ...
}
```

使用手動 vtable 而非 trait object，因為需要從同一個 `Box<dyn Any>` 動態查詢多個 trait impl。

> **Clawtex 實作建議**: clawtex-core 的 `agents.toml` 設定已經實現了類似 DynClientBuilder 的動態 provider 選擇。但 Rig 的 `AnyClient` vtable 模式可以借鏡 -- 在 clawtex 的 provider registry 中，每個 provider 可以宣告自己的能力集合（completion, embedding, vision 等），讓 router 在選擇 provider 時自動排除不支援目標能力的 provider。

---

## 6. Tool/Function Calling 完整系統

### 6.1 Tool Trait 設計哲學

**檔案:** `rig/rig-core/src/tool/mod.rs`

```rust
pub trait Tool: Sized + WasmCompatSend + WasmCompatSync {
    /// 編譯期常量名稱 -- 保證唯一性
    const NAME: &'static str;

    /// 每個工具有自己的錯誤型別
    type Error: std::error::Error + WasmCompatSend + WasmCompatSync + 'static;
    /// 強型別參數 -- 自動 JSON 反序列化
    type Args: for<'a> Deserialize<'a> + WasmCompatSend + WasmCompatSync;
    /// 強型別輸出 -- 自動 JSON 序列化
    type Output: Serialize;

    /// 可覆寫的名稱方法 (Agent-as-Tool 需要)
    fn name(&self) -> String { Self::NAME.to_string() }

    /// 工具定義，接受 prompt 可做動態調整
    fn definition(&self, _prompt: String)
        -> impl Future<Output = ToolDefinition> + WasmCompatSend + WasmCompatSync;

    /// 工具執行
    fn call(&self, args: Self::Args)
        -> impl Future<Output = Result<Self::Output, Self::Error>> + WasmCompatSend;
}
```

**設計亮點:**

1. **`const NAME`**: 編譯期確定，不需要 runtime 查詢
2. **Associated Type `Args`**: `for<'a> Deserialize<'a>` (HRTB) 確保任何生命週期的反序列化都能工作
3. **`definition()` 接受 `prompt`**: 工具可以根據當前 prompt 動態調整描述。例如計算器工具在數學問題時描述更詳細
4. **`Sized` bound**: 確保 Tool 可以被 `Box::new()` 包裝

### 6.2 ToolDyn 動態分發橋接

```rust
/// Object-safe wrapper trait
pub trait ToolDyn: WasmCompatSend + WasmCompatSync {
    fn name(&self) -> String;
    fn definition<'a>(&'a self, prompt: String) -> WasmBoxedFuture<'a, ToolDefinition>;
    fn call<'a>(&'a self, args: String) -> WasmBoxedFuture<'a, Result<String, ToolError>>;
}

/// Blanket impl: 任何 Tool 自動成為 ToolDyn
impl<T: Tool> ToolDyn for T {
    fn call<'a>(&'a self, args: String) -> WasmBoxedFuture<'a, Result<String, ToolError>> {
        Box::pin(async move {
            // 1. JSON string → 強型別 Args
            match serde_json::from_str(&args) {
                Ok(args) => {
                    // 2. 呼叫 Tool::call (強型別)
                    <Self as Tool>::call(self, args).await
                        // 3. 工具錯誤 → 統一 ToolError
                        .map_err(|e| ToolError::ToolCallError(Box::new(e)))
                        // 4. 強型別 Output → JSON string
                        .and_then(|output| serde_json::to_string(&output)
                            .map_err(ToolError::JsonError))
                },
                Err(e) => Err(ToolError::JsonError(e)),
            }
        })
    }
}
```

**型別轉換流程:**
```
LLM 回傳 JSON string → serde_json::from_str → Tool::Args (強型別)
    → Tool::call() → Tool::Output (強型別)
    → serde_json::to_string → JSON string → 回傳給 LLM
```

### 6.3 ToolSet 容器

```rust
pub struct ToolSet {
    pub(crate) tools: HashMap<String, ToolType>,
}

pub(crate) enum ToolType {
    Simple(Box<dyn ToolDyn>),           // 普通工具
    Embedding(Box<dyn ToolEmbeddingDyn>), // 可 RAG 的工具
}

impl ToolSet {
    pub fn add_tool(&mut self, tool: impl ToolDyn + 'static) { ... }
    pub fn delete_tool(&mut self, tool_name: &str) { ... }
    pub fn add_tools(&mut self, toolset: ToolSet) { ... }  // 合併

    pub async fn call(&self, toolname: &str, args: String) -> Result<String, ToolSetError> {
        if let Some(tool) = self.tools.get(toolname) {
            tracing::debug!(target: "rig", "Calling tool {toolname}...");
            Ok(tool.call(args).await?)
        } else {
            Err(ToolSetError::ToolNotFoundError(toolname.to_string()))
        }
    }
}
```

### 6.4 ToolEmbedding -- 可 RAG 的工具

```rust
pub trait ToolEmbedding: Tool {
    type InitError: std::error::Error + WasmCompatSend + WasmCompatSync + 'static;
    type Context: for<'a> Deserialize<'a> + Serialize;
    type State: WasmCompatSend;

    /// 回傳用於 embedding 的文件 (一個工具可有多個 embedding 方向)
    fn embedding_docs(&self) -> Vec<String>;

    /// 回傳可序列化的上下文 (存入 vector store)
    fn context(&self) -> Self::Context;

    /// 從上下文重建工具 (從 vector store 恢復)
    fn init(state: Self::State, context: Self::Context) -> Result<Self, Self::InitError>;
}
```

**使用場景**: 當工具數量超過 20-30 個，全量傳送工具定義會浪費大量 token。`ToolEmbedding` 讓每個工具提供自己的 embedding 文字，系統根據 prompt 做語意搜索，只選出最相關的工具。

### 6.5 MCP 整合 (rmcp)

```rust
#[cfg(feature = "rmcp")]
pub struct McpTool {
    definition: rmcp::model::Tool,
    client: rmcp::service::ServerSink,
}

impl ToolDyn for McpTool {
    fn call(&self, args: String) -> WasmBoxedFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let result = self.client.call_tool(CallToolRequestParams {
                name: self.definition.name.clone(),
                arguments: serde_json::from_str(&args).unwrap_or_default(),
                meta: None, task: None,
            }).await.map_err(|e| McpToolError(...))?;

            // 處理多種回傳內容型別
            Ok(result.content.into_iter().map(|c| match c.raw {
                RawContent::Text(raw) => raw.text,
                RawContent::Image(raw) => format!("data:{};base64,{}", raw.mime_type, raw.data),
                RawContent::Resource(raw) => { /* ... */ },
                RawContent::Audio(_) => panic!("Audio not supported yet"),
                _ => panic!("Unsupported type"),
            }).collect::<String>())
        })
    }
}
```

**注意**: MCP 的 Audio 回傳型別目前 panic，這是已知的技術債。

> **Clawtex 實作建議**: clawtex-core 已有自建的 MCP JSON-RPC 2.0 客戶端。Rig 的 McpTool 包裝展示了如何把 MCP server 的工具無縫整合到 Agent 的工具系統中。clawtex 可以考慮將 MCP 工具自動註冊到 ToolRegistry，讓 MCP server 的工具與原生工具在 agent_runtime 中統一處理。

---

## 7. ToolServer Actor 完整生命週期

**檔案:** `rig/rig-core/src/tool/server.rs`

### 7.1 Spawn 階段

```rust
pub struct ToolServer {
    static_tool_names: Vec<String>,
    dynamic_tools: Vec<(usize, Box<dyn VectorStoreIndexDyn + Send + Sync>)>,
    toolset: Arc<RwLock<ToolSet>>,
}

impl ToolServer {
    pub fn run(mut self) -> ToolServerHandle {
        // 建立 mpsc channel (buffer 1000)
        let (tx, mut rx) = tokio::sync::mpsc::channel(1000);

        // 消費 self，移入 background task
        tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                self.handle_message(message).await;
            }
            // rx 關閉 = 所有 tx (ToolServerHandle) 都被 drop
            // self 在此自動 drop，釋放 ToolSet
        });

        ToolServerHandle(tx)
    }
}
```

**關鍵設計:**
- `channel(1000)`: buffer 大小 1000，防止大量並發工具呼叫時阻塞
- `self` 被 move 進 task: ToolServer 的所有權完全轉移，無法在外部再操作
- `Arc::get_mut(&mut self.toolset)` 在 `run()` 前使用: 因為此時 Arc 只有一個 owner，get_mut 一定成功

### 7.2 Message 協定

```rust
pub struct ToolServerRequest {
    callback_channel: futures::channel::oneshot::Sender<ToolServerResponse>,
    data: ToolServerRequestMessageKind,
}

pub enum ToolServerRequestMessageKind {
    AddTool(Box<dyn ToolDyn>),       // 動態新增工具
    AppendToolset(ToolSet),           // 批量新增工具
    RemoveTool { tool_name: String }, // 移除工具
    CallTool {                        // 呼叫工具
        name: String,
        args: String,
        span: tracing::Span,         // 傳遞 tracing span 做分散式追蹤
    },
    GetToolDefs { prompt: Option<String> }, // 取得工具定義 (含動態搜索)
}

pub enum ToolServerResponse {
    ToolAdded,
    ToolDeleted,
    ToolExecuted { result: String },
    ToolError { error: String },
    ToolDefinitions(Vec<ToolDefinition>),
}
```

**通訊模式: 請求-回應 (Request-Response)**

每個請求都帶有一個 oneshot channel，ToolServer 處理完後透過此 channel 回傳結果:

```rust
impl ToolServerHandle {
    pub async fn call_tool(&self, tool_name: &str, args: &str) -> Result<String, ToolServerError> {
        let (tx, rx) = futures::channel::oneshot::channel();
        self.0.send(ToolServerRequest {
            callback_channel: tx,
            data: ToolServerRequestMessageKind::CallTool {
                name: tool_name.to_string(),
                args: args.to_string(),
                span: tracing::Span::current(),
            },
        }).await?;
        let res = rx.await?;  // 等待 ToolServer 回應
        match res {
            ToolServerResponse::ToolExecuted { result } => Ok(result),
            ToolServerResponse::ToolError { error } => Err(...),
            invalid => Err(ToolServerError::InvalidMessage(invalid)),
        }
    }
}
```

### 7.3 並行工具執行

```rust
ToolServerRequestMessageKind::CallTool { name, args, span } => {
    let toolset = Arc::clone(&self.toolset);  // clone Arc, 不 clone ToolSet

    // 每個工具呼叫都 spawn 獨立 task
    tokio::spawn(
        async move {
            match toolset.read().await.call(&name, args.clone()).await {
                Ok(result) => {
                    let _ = callback_channel.send(ToolServerResponse::ToolExecuted { result });
                }
                Err(err) => {
                    let _ = callback_channel.send(ToolServerResponse::ToolError {
                        error: err.to_string(),
                    });
                }
            }
        }
        .instrument(span),  // 攜帶 tracing span
    );
}
```

**並行保證:**
- `toolset` 是 `Arc<RwLock<ToolSet>>`
- `toolset.read().await` 取讀取鎖: 多個工具可以同時執行
- 每個工具呼叫 spawn 獨立 task: 不阻塞 ToolServer 的主迴圈
- **結果**: 3 個各睡 100ms 的工具並行呼叫只需 ~100ms 而非 300ms (有測試驗證)

### 7.4 動態工具定義查詢 (Tool RAG)

```rust
pub async fn get_tool_definitions(&mut self, text: Option<String>)
    -> Result<Vec<ToolDefinition>, CompletionError>
{
    let toolset = self.toolset.read().await;

    let mut tools = if let Some(text) = text {
        // 1. 對每個 dynamic_tools 的 VectorStoreIndex 做語意搜索
        let dynamic_tool_ids: Vec<String> = stream::iter(self.dynamic_tools.iter())
            .then(|(num_sample, index)| async {
                let req = VectorSearchRequest::builder()
                    .query(text.clone())
                    .samples(*num_sample as u64)
                    .build()?;
                Ok(index.as_ref().top_n_ids(req).await?
                    .into_iter().map(|(_, id)| id).collect::<Vec<_>>())
            })
            .try_fold(vec![], |mut acc, docs| async { acc.extend(docs); Ok(acc) })
            .await?;

        // 2. 根據搜索結果的 ID 從 ToolSet 取得工具定義
        let mut tools = Vec::new();
        for doc in dynamic_tool_ids {
            if let Some(tool) = toolset.get(&doc) {
                tools.push(tool.definition(text.clone()).await)
            } else {
                tracing::warn!("Tool implementation not found: {}", doc);
            }
        }
        tools
    } else {
        Vec::new()
    };

    // 3. 加上所有靜態工具
    for toolname in &self.static_tool_names {
        if let Some(tool) = toolset.get(toolname) {
            tools.push(tool.definition(String::new()).await)
        }
    }
    Ok(tools)
}
```

**資料流:**
```
prompt text
  │
  ├─ VectorStoreIndex::top_n_ids(query=text, samples=N)
  │   └─ 回傳 [(score, tool_id), ...]
  │
  ├─ ToolSet::get(tool_id) → ToolType
  │   └─ tool.definition(text) → ToolDefinition
  │
  └─ 合併靜態工具定義
      └─ Vec<ToolDefinition>
```

### 7.5 Shutdown 與資源回收

ToolServer 沒有明確的 shutdown 方法。它依賴 Rust 的所有權系統:

1. 所有 `ToolServerHandle` (mpsc Sender) 被 drop
2. `rx.recv()` 回傳 `None`
3. `while let Some(message) = rx.recv().await` 迴圈結束
4. `self` (ToolServer) 自動 drop，釋放 ToolSet 和所有工具

**注意**: 如果有工具呼叫正在 spawn 的 task 中執行，這些 task 會繼續跑完，因為它們持有 `Arc<RwLock<ToolSet>>` 的引用。

> **Clawtex 實作建議**: clawtex-core 的 ToolRegistry 是 `HashMap<String, Box<dyn ToolExecutor>>`，由 agent_runtime 直接存取。引入 Actor 模式可以帶來: (1) 工具的 hot-reload -- 在不重啟 daemon 的情況下新增/移除工具; (2) 並行工具執行 -- 目前 clawtex 的工具是串列執行; (3) 多 Agent 共享工具 -- 不同 session 的 Agent 可以共享同一個 ToolServer。但也要注意 Actor 模式引入的 channel 通訊開銷，對於只有 24 個工具的系統，直接的 HashMap 查詢可能更快。

---

## 8. Embedding 與 RAG 系統

### 8.1 EmbeddingModel Trait

```rust
pub trait EmbeddingModel: WasmCompatSend + WasmCompatSync {
    const MAX_DOCUMENTS: usize;   // 單次 batch 最大文件數
    type Client;

    fn make(client: &Self::Client, model: impl Into<String>, dims: Option<usize>) -> Self;
    fn ndims(&self) -> usize;     // embedding 維度
    fn embed_texts(&self, texts: impl IntoIterator<Item = String>)
        -> impl Future<Output = Result<Vec<Embedding>, EmbeddingError>>;
    fn embed_text(&self, text: &str)
        -> impl Future<Output = Result<Embedding, EmbeddingError>>;
}

pub struct Embedding {
    pub document: String,  // 原始文字
    pub vec: Vec<f64>,     // embedding 向量
}
```

### 8.2 Embed Trait 與 TextEmbedder

```rust
/// 可嵌入物件的 trait -- 決定「哪些欄位需要被 embed」
pub trait Embed {
    fn embed(&self, embedder: &mut TextEmbedder) -> Result<(), EmbedError>;
}

/// 累積需要 embed 的文字
pub struct TextEmbedder {
    pub(crate) texts: Vec<String>,
}

impl TextEmbedder {
    pub fn embed(&mut self, text: String) { self.texts.push(text); }
}

// 用法:
struct WordDefinition {
    word: String,
    definitions: String,
}

impl Embed for WordDefinition {
    fn embed(&self, embedder: &mut TextEmbedder) -> Result<(), EmbedError> {
        // 只 embed definitions 欄位，word 不需要
        self.definitions.split(",")
            .for_each(|s| embedder.embed(s.to_string()));
        Ok(())
    }
}
```

Rig 也提供 `#[derive(Embed)]` proc macro 自動實作。

### 8.3 VectorStoreIndex Trait 與動態分發

```rust
pub trait VectorStoreIndex: WasmCompatSend + WasmCompatSync {
    type Filter: SearchFilter + WasmCompatSend + WasmCompatSync;

    async fn top_n<T: for<'a> Deserialize<'a>>(
        &self, req: VectorSearchRequest<Self::Filter>,
    ) -> Result<Vec<(f64, String, T)>, VectorStoreError>;

    async fn top_n_ids(
        &self, req: VectorSearchRequest<Self::Filter>,
    ) -> Result<Vec<(f64, String)>, VectorStoreError>;
}

// VectorStoreIndex 自動實作 Tool!
impl<T, F> Tool for T where T: VectorStoreIndex<Filter = F> {
    const NAME: &'static str = "search_vector_store";
    type Args = VectorSearchRequest<F>;
    type Output = Vec<VectorStoreOutput>;
    // ...
}
```

**VectorStoreIndex as Tool**: 任何 vector store 自動變成 LLM 可呼叫的工具。

### 8.4 Agent 的 RAG 資料流

```
User prompt: "What is a flurbo?"
  │
  ├─ prompt.rag_text() → "What is a flurbo?"
  │
  ├─ dynamic_context[0]: (sample=2, InMemoryVectorStore)
  │   └─ top_n(query="What is a flurbo?", samples=2)
  │       → [(0.95, "doc1", {definition: "A flurbo is..."}),
  │          (0.87, "doc2", {definition: "Related concept..."})]
  │
  ├─ dynamic_tools[0]: (sample=3, ToolVectorStore)
  │   └─ top_n_ids(query="What is a flurbo?", samples=3)
  │       → [(0.92, "dictionary_lookup"), (0.78, "web_search"), (0.65, "calculator")]
  │
  └─ CompletionRequest {
         preamble: "You are a dictionary assistant",
         documents: [doc1, doc2],        ← 動態上下文
         tools: [dictionary_lookup, web_search, calculator, ...], ← 動態 + 靜態工具
         chat_history: [Message::User("What is a flurbo?")],
     }
```

> **Clawtex 實作建議**: clawtex-core 使用 SQLite `memories` 表做語意記憶，但沒有向量搜索能力。兩個改進方向: (1) 在 memory_recall 工具中加入 embedding-based 搜索（使用 rig-sqlite 或自建 in-memory vector store）; (2) 將 24 個工具的描述做 embedding，在 agent_runtime 中根據 prompt 只選擇 top-8 工具傳給 LLM。

---

## 9. Streaming 實作深度解析

### 9.1 RawStreamingChoice 事件模型

**檔案:** `rig/rig-core/src/streaming.rs`

```rust
pub enum RawStreamingChoice<R: Clone> {
    Message(String),              // 文字 chunk
    ToolCall(RawStreamingToolCall), // 完整工具呼叫
    ToolCallDelta {               // 工具呼叫 delta
        id: String,
        internal_call_id: String,
        content: ToolCallDeltaContent,
    },
    Reasoning { id: Option<String>, content: ReasoningContent },  // 完整推理塊
    ReasoningDelta { id: Option<String>, reasoning: String },     // 推理 delta
    FinalResponse(R),             // 最終回應物件 (含 token 用量)
    MessageId(String),            // Provider 分配的訊息 ID
}
```

### 9.2 StreamingCompletionResponse 狀態機

```rust
pub struct StreamingCompletionResponse<R: Clone + Unpin + GetTokenUsage> {
    pub(crate) inner: Abortable<StreamingResult<R>>,
    pub(crate) abort_handle: AbortHandle,
    pub(crate) pause_control: PauseControl,
    assistant_items: Vec<AssistantContent>,
    text_item_index: Option<usize>,        // 當前文字 chunk 在 items 中的位置
    reasoning_item_index: Option<usize>,   // 當前推理在 items 中的位置
    pub choice: OneOrMany<AssistantContent>,
    pub response: Option<R>,
    pub final_response_yielded: AtomicBool,
    pub message_id: Option<String>,
}
```

**Stream trait 的 poll_next 實作 (核心狀態機):**

```rust
impl<R: Clone + Unpin + GetTokenUsage> Stream for StreamingCompletionResponse<R> {
    type Item = Result<StreamedAssistantContent<R>, CompletionError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let stream = self.get_mut();

        // 暫停檢查
        if stream.is_paused() {
            cx.waker().wake_by_ref();  // 註冊 waker, 下次 poll 再檢查
            return Poll::Pending;
        }

        match Pin::new(&mut stream.inner).poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => {
                // 串流結束: 聚合所有 items 到 choice
                stream.choice = OneOrMany::many(
                    std::mem::take(&mut stream.assistant_items)
                ).expect("at least one item");
                Poll::Ready(None)
            }
            Poll::Ready(Some(Ok(choice))) => match choice {
                RawStreamingChoice::Message(text) => {
                    stream.reasoning_item_index = None; // 文字打斷推理
                    stream.append_text_chunk(&text);
                    Poll::Ready(Some(Ok(StreamedAssistantContent::text(&text))))
                }
                RawStreamingChoice::ToolCall(raw) => {
                    stream.text_item_index = None;      // 工具打斷文字
                    stream.reasoning_item_index = None;
                    let tool_call: ToolCall = raw.into();
                    stream.assistant_items.push(AssistantContent::ToolCall(tool_call.clone()));
                    Poll::Ready(Some(Ok(StreamedAssistantContent::ToolCall { ... })))
                }
                RawStreamingChoice::FinalResponse(response) => {
                    // 只 yield 一次
                    if stream.final_response_yielded.load(SeqCst) {
                        stream.poll_next_unpin(cx)  // 跳過重複
                    } else {
                        stream.response = Some(response.clone());
                        stream.final_response_yielded.store(true, SeqCst);
                        Poll::Ready(Some(Ok(StreamedAssistantContent::final_response(response))))
                    }
                }
                RawStreamingChoice::MessageId(id) => {
                    stream.message_id = Some(id);
                    stream.poll_next_unpin(cx)  // 靜默捕獲，不回傳給使用者
                }
                // ... 其他分支
            },
            Poll::Ready(Some(Err(err))) => {
                // 特殊處理: abort 導致的 "aborted" 錯誤當作正常結束
                if matches!(err, CompletionError::ProviderError(ref e) if e.contains("aborted")) {
                    return Poll::Ready(None);
                }
                Poll::Ready(Some(Err(err)))
            }
        }
    }
}
```

### 9.3 PauseControl 與 AbortHandle

```rust
pub struct PauseControl {
    paused_tx: watch::Sender<bool>,
    paused_rx: watch::Receiver<bool>,
}

impl PauseControl {
    pub fn pause(&self) { self.paused_tx.send(true).unwrap(); }
    pub fn resume(&self) { self.paused_tx.send(false).unwrap(); }
    pub fn is_paused(&self) -> bool { *self.paused_rx.borrow() }
}
```

使用 `tokio::sync::watch` 而非 `AtomicBool` 的原因: watch channel 在值改變時會通知接收者，配合 `cx.waker().wake_by_ref()` 可以讓 paused 狀態的 stream 在 resume 時立即被 poll。

### 9.4 文字聚合與非連續處理

```rust
fn append_text_chunk(&mut self, text: &str) {
    if let Some(index) = self.text_item_index
        && let Some(AssistantContent::Text(existing)) = self.assistant_items.get_mut(index)
    {
        // 連續文字 chunk: 合併到同一個 Text item
        existing.text.push_str(text);
        return;
    }
    // 新的文字 chunk (或被工具呼叫打斷後的新文字)
    self.assistant_items.push(AssistantContent::text(text.to_owned()));
    self.text_item_index = Some(self.assistant_items.len() - 1);
}
```

**關鍵行為**: 當文字被工具呼叫打斷時 (`text_item_index` 被設為 None)，下一個文字 chunk 會建立新的 Text item，而不是合併到之前的文字。最終結果保持正確的順序: `[Text("first"), ToolCall(...), Text("second")]`。

> **Clawtex 實作建議**: clawtex-core 目前使用 SSE text 格式做串流。Rig 的 `RawStreamingChoice` enum 統一了所有串流事件類型。建議在 clawtex 的串流系統中引入類似的事件 enum，讓 Telegram bot 可以區分文字、工具呼叫、推理等不同類型的串流事件，提供更好的 UX (例如工具呼叫時顯示 spinner)。PauseControl 機制對 Telegram 的 human-in-the-loop 也很有用 -- approval gate 等待時暫停串流。

---

## 10. Extractor 結構化抽取模式

**檔案:** `rig/rig-core/src/extractor.rs`

Extractor 的核心思路: 建立一個「只有 submit 工具」的 Agent，強制 LLM 呼叫 submit 工具並傳回結構化資料。

```rust
pub struct ExtractorBuilder<M, T> {
    agent_builder: AgentBuilder<M, (), WithBuilderTools>,
    _t: PhantomData<T>,
    retries: Option<u64>,
}

impl<M, T> ExtractorBuilder<M, T> {
    pub fn new(model: M) -> Self {
        Self {
            agent_builder: AgentBuilder::new(model)
                .preamble("You are an AI assistant whose purpose is to extract structured data...")
                .tool(SubmitTool::<T> { _t: PhantomData })
                .tool_choice(ToolChoice::Required),  // 強制使用工具
            retries: None,
        }
    }
}

// SubmitTool: 一個「什麼都不做」的工具，只定義了 schema
struct SubmitTool<T> { _t: PhantomData<T> }

impl<T: JsonSchema + Deserialize + Serialize> Tool for SubmitTool<T> {
    const NAME: &'static str = "submit";
    type Args = T;      // ← 參數型別就是目標結構體
    type Output = T;     // ← 輸出也是

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "submit".to_string(),
            description: "Submit the structured data you extracted".to_string(),
            parameters: json!(schema_for!(T)),  // 自動生成 JSON Schema
        }
    }

    async fn call(&self, data: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(data)  // 直接 pass-through
    }
}
```

**抽取流程:**

```
User: "John Doe is a 30 year old doctor"
  │
  ├─ Extractor 送出 CompletionRequest:
  │   preamble: "Extract structured data..."
  │   tools: [submit(name: String, age: u8, profession: String)]
  │   tool_choice: Required
  │
  ├─ LLM 回應: ToolCall("submit", {name: "John Doe", age: 30, profession: "doctor"})
  │
  ├─ Extractor 從回應中找到 submit 工具呼叫
  │   └─ serde_json::from_value::<Person>(arguments)
  │
  └─ 回傳 Person { name: "John Doe", age: 30, profession: "doctor" }
```

**重試機制:**

```rust
pub async fn extract(&self, text: impl Into<Message>) -> Result<T, ExtractionError> {
    let mut last_error = None;
    for i in 0..=self.retries {
        match self.extract_json_with_usage(text_message.clone(), vec![]).await {
            Ok((data, _usage)) => return Ok(data),
            Err(e) => {
                tracing::warn!("Attempt {i} failed: {e:?}. Retrying...");
                last_error = Some(e);
            }
        }
    }
    Err(last_error.unwrap_or(ExtractionError::NoData))
}
```

> **Clawtex 實作建議**: clawtex-core 在 Hands workflow 中常需要結構化輸出（例如 `self_evolve` hand 需要解析改進建議為結構化資料）。可以引入 Extractor 模式作為 agent_runtime 的一個選項: 當 hand phase 設定 `output_schema` 時，自動切換到 Extractor 模式。具體實作: 在 `complete()` 呼叫前，插入一個 "submit" 工具定義並設 `tool_choice: Required`。

---

## 11. Pipeline DAG 框架

**檔案:** `rig/rig-core/src/pipeline/op.rs`

```rust
pub trait Op: WasmCompatSend + WasmCompatSync {
    type Input: WasmCompatSend + WasmCompatSync;
    type Output: WasmCompatSend + WasmCompatSync;

    fn call(&self, input: Self::Input) -> impl Future<Output = Self::Output> + WasmCompatSend;

    // 批量並行執行
    fn batch_call<I>(&self, n: usize, input: I) -> impl Future<Output = Vec<Self::Output>>
    where I: IntoIterator<Item = Self::Input> {
        async move {
            stream::iter(input)
                .map(|input| self.call(input))
                .buffered(n)  // 最多 n 個並行
                .collect().await
        }
    }

    // 組合方法
    fn map<F>(self, f: F) -> Sequential<Self, Map<F>> { ... }
    fn then<F, Fut>(self, f: F) -> Sequential<Self, Then<F>> { ... }
    fn chain<T: Op>(self, op: T) -> Sequential<Self, T> { ... }
    fn lookup<I>(self, index: I, n: usize) -> Sequential<Self, Lookup<I>> { ... }
    fn prompt<P>(self, prompt: P) -> Sequential<Self, Prompt<P>> { ... }
}

// Sequential: 兩個 Op 的串接
pub struct Sequential<Op1, Op2> { prev: Op1, op: Op2 }

impl<Op1: Op, Op2: Op<Input = Op1::Output>> Op for Sequential<Op1, Op2> {
    type Input = Op1::Input;
    type Output = Op2::Output;

    async fn call(&self, input: Self::Input) -> Self::Output {
        let prev = self.prev.call(input).await;
        self.op.call(prev).await
    }
}
```

**使用範例:**

```rust
let pipeline = pipeline::new()
    .map(|(x, y): (i32, i32)| x + y)    // 同步轉換
    .then(|sum| async move { sum * 2 })   // 異步轉換
    .map(|result| format!("Result: {}", result));

let output = pipeline.call((3, 4)).await;
// output = "Result: 14"
```

> **Clawtex 實作建議**: clawtex-core 的 Hands 工作流引擎使用 TOML 定義多階段。Rig 的 Pipeline 是程式化的，適合內部使用。可以考慮在 agent_runtime 內部使用 Op trait 來組合常見的處理步驟（例如: 讀取上下文 → 查詢記憶 → 呼叫 LLM → 解析輸出 → 儲存結果），讓程式碼更 composable。

---

## 12. Loaders 文件載入器系統

### 12.1 FileLoader 泛型迭代器設計

**檔案:** `rig/rig-core/src/loaders/file.rs`

FileLoader 使用泛型參數 `T` 追蹤迭代器當前的項目型別:

```rust
pub struct FileLoader<'a, T> {
    iterator: Box<dyn Iterator<Item = T> + 'a>,
}
```

**型別狀態轉換鏈:**

```
FileLoader::with_glob("*.txt")
  → FileLoader<Result<PathBuf, FileLoaderError>>
    │
    ├─ .read()
    │   → FileLoader<Result<String, FileLoaderError>>
    │       ├─ .ignore_errors()
    │       │   → FileLoader<String>
    │       └─ .into_iter() → Iterator<Item = Result<String, ...>>
    │
    └─ .read_with_path()
        → FileLoader<Result<(PathBuf, String), FileLoaderError>>
            └─ .ignore_errors()
                → FileLoader<(PathBuf, String)>
```

每個方法消費當前 FileLoader，回傳新型別的 FileLoader。這是 typestate 的輕量應用。

### 12.2 PdfFileLoader 分頁處理

**檔案:** `rig/rig-core/src/loaders/pdf.rs`

```rust
pub struct PdfFileLoader<'a, T> {
    iterator: Box<dyn Iterator<Item = T> + 'a>,
}

// 進階處理鏈:
let pages = PdfFileLoader::with_glob("docs/*.pdf")?
    .load_with_path()         // → FileLoader<Result<(PathBuf, Document), ...>>
    .ignore_errors()          // → FileLoader<(PathBuf, Document)>
    .by_page()                // → FileLoader<(PathBuf, Vec<(usize, Result<String, ...>)>)>
    .ignore_errors()          // → FileLoader<(PathBuf, Vec<(usize, String)>)>
    .into_iter()
    .collect::<Vec<_>>();
```

**支援多種輸入源:**
- `with_glob("*.pdf")` -- glob 模式
- `with_dir("documents/")` -- 目錄
- `from_bytes(bytes)` -- 記憶體中的 PDF 位元組

使用 `lopdf` crate 解析 PDF，`extract_text` 逐頁提取文字。

### 12.3 EpubFileLoader

類似 PDF，但使用 EPUB 的章節結構:
- 支援 `by_chapter()` 分章處理
- `TextProcessor` trait 允許自訂文字處理 (StripXml, RawText)

> **Clawtex 實作建議**: clawtex-core 有 `file_read` 工具但只支援純文字。可以引入類似 Rig 的 Loader 系統，讓 `file_read` 工具自動偵測檔案類型 (PDF, EPUB, HTML)，使用對應的 loader 提取文字。這對 `pdf_export` 和未來的 RAG pipeline 特別有價值。Loader 的 typestate 迭代器設計也很適合處理大量文件的批量嵌入。

---

## 13. 錯誤處理架構

Rig 使用階層化的錯誤 enum，每層封裝下層:

```
最高層: StructuredOutputError
  ├─ PromptError
  │   ├─ CompletionError
  │   │   ├─ HttpError (reqwest)
  │   │   ├─ JsonError (serde_json)
  │   │   ├─ UrlError (url::ParseError)
  │   │   ├─ RequestError (Box<dyn Error>)
  │   │   ├─ ResponseError (String)
  │   │   └─ ProviderError (String)
  │   ├─ ToolSetError
  │   │   ├─ ToolCallError
  │   │   │   ├─ ToolError::ToolCallError (Box<dyn Error>)
  │   │   │   └─ ToolError::JsonError
  │   │   ├─ ToolNotFoundError
  │   │   └─ Interrupted
  │   ├─ ToolServerError
  │   │   ├─ Canceled (oneshot channel)
  │   │   ├─ ToolsetError
  │   │   ├─ SendError (mpsc channel)
  │   │   └─ InvalidMessage
  │   ├─ MaxTurnsError { max_turns, chat_history, prompt }
  │   └─ PromptCancelled { chat_history, reason }
  └─ DeserializationError
```

**特殊設計:**

1. **`PromptError::MaxTurnsError` 保留 chat_history**: 讓呼叫者可以從中斷處繼續對話
2. **`PromptCancelled` 保留 chat_history 和 reason**: hook 終止迴圈時保留完整上下文
3. **ToolError 的遞迴處理**: Agent-as-Tool 時會產生巢狀的 ToolCallError，`Display` 實作避免重複前綴

```rust
impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolError::ToolCallError(e) => {
                let error_str = e.to_string();
                // 避免重複 "ToolCallError: ToolCallError: ..."
                if error_str.starts_with("ToolCallError: ") {
                    write!(f, "{}", error_str)
                } else {
                    write!(f, "ToolCallError: {}", error_str)
                }
            }
            // ...
        }
    }
}
```

> **Clawtex 實作建議**: clawtex-core 的錯誤處理大多使用 `anyhow::Error` 或單一 `ProviderError` enum。引入分層錯誤可以讓 agent_runtime 根據錯誤類型做不同處理: (1) CompletionError → retry 或 fallback provider; (2) ToolError → 回傳錯誤訊息給 LLM 繼續對話; (3) MaxTurnsError → 回傳部分結果給使用者。建議至少分為 `ProviderError` / `ToolError` / `AgentError` 三層。

---

## 14. 型別安全機制總覽

| 機制 | 實作方式 | 保證 |
|------|---------|------|
| OneOrMany\<T\> | `first: T, rest: Vec<T>` | 至少有一個元素 |
| AgentBuilder Typestate | 泛型 ToolState 參數 | 工具配置不互斥 |
| CompletionModel Associated Types | `type Response`, `type StreamingResponse` | Provider 回應型別安全 |
| Tool Trait Associated Types | `type Args: Deserialize`, `type Output: Serialize` | 工具 IO 自動序列化 |
| Capabilities Trait | `Capable<T>` vs `Nothing` | 編譯期能力檢查 |
| TypedPrompt | `T: JsonSchema + DeserializeOwned` | 結構化輸出型別安全 |
| WasmCompatSend/Sync | 條件編譯 | WASM + 原生雙平台 |
| ConvertMessage | `TryFrom<Message>` + `TryInto<Message>` | Provider 訊息轉換安全 |

---

## 15. HTTP 客戶端與 Retry 策略

**檔案:** `rig/rig-core/src/http_client/retry.rs`

```rust
pub trait RetryPolicy {
    fn retry(&self, error: &Error, last_retry: Option<(usize, Duration)>) -> Option<Duration>;
    fn set_reconnection_time(&mut self, duration: Duration);
}

// 三種內建策略
pub struct ExponentialBackoff { start, factor, max_duration, max_retries }
pub struct Constant { delay, max_retries }
pub struct Never;

pub const DEFAULT_RETRY: ExponentialBackoff = ExponentialBackoff::new(
    Duration::from_millis(300),  // 初始 300ms
    2.,                          // 每次乘 2
    Some(Duration::from_secs(5)), // 最長 5s
    None,                        // 無限重試
);
```

**ExponentialBackoff 邏輯:**

```
retry 0: 300ms
retry 1: 600ms
retry 2: 1200ms
retry 3: 2400ms
retry 4: 4800ms
retry 5+: 5000ms (cap)
```

> **Clawtex 實作建議**: clawtex-core 的 provider 有 fallback 機制但缺乏細粒度 retry。引入 `RetryPolicy` trait 讓每個 provider 可以配置不同的 retry 策略。例如 Ollama (本地) 可以用 `Constant(100ms, max=3)`，Anthropic (雲端) 可以用 `ExponentialBackoff` 搭配 rate limit header 的 `set_reconnection_time`。

---

## 16. OneOrMany 非空集合保證

**檔案:** `rig/rig-core/src/one_or_many.rs`

```rust
pub struct OneOrMany<T> {
    first: T,       // 保證存在的第一個元素
    rest: Vec<T>,   // 可能為空的其餘元素
}

impl<T: Clone> OneOrMany<T> {
    pub fn one(item: T) -> Self { ... }
    pub fn many<I>(items: I) -> Result<Self, EmptyListError> { ... } // 空 → 錯誤
    pub fn len(&self) -> usize { 1 + self.rest.len() }
    pub fn is_empty(&self) -> bool { false }  // 永遠不為空

    // insert 到 index 0 時特殊處理
    pub fn insert(&mut self, index: usize, item: T) {
        if index == 0 {
            let old_first = std::mem::replace(&mut self.first, item);
            self.rest.insert(0, old_first);
        } else {
            self.rest.insert(index - 1, item);
        }
    }
}
```

**serde 整合:** OneOrMany 可以同時反序列化 JSON 陣列和單一值:

```rust
// 以下兩種 JSON 都能正確反序列化為 OneOrMany<String>:
// {"field": ["a", "b"]}  → OneOrMany { first: "a", rest: ["b"] }
// {"field": "a"}          → OneOrMany { first: "a", rest: [] }
```

使用 `string_or_one_or_many` 自訂 deserializer 實現此彈性。

> **Clawtex 實作建議**: clawtex-core 在處理 LLM 回應時經常面對「至少有一個內容」的假設。`OneOrMany` 可以用在 message content、tool results、provider 回應列表等場景，消除大量的 `.unwrap()` 和 `.expect("at least one")`。

---

## 17. 效能考量與瓶頸分析

### 17.1 潛在瓶頸

| 區域 | 瓶頸 | 影響 | 嚴重度 |
|------|------|------|--------|
| ToolServer channel | buffer 1000 但無背壓 | 大量並發可能 OOM | 低 |
| dynamic_context 查詢 | 串列查詢多個 VectorStore | 多個 store 時延遲疊加 | 中 |
| ToolSet RwLock | 每次工具呼叫都要取讀取鎖 | 高並發時可能競爭 | 低 |
| AgentBuilder 欄位搬移 | 每次狀態轉換都要手動列出所有欄位 | 維護成本高但無運行時影響 | 無 |
| streaming poll 中的 append_text_chunk | 每個 text chunk 都要檢查 index | 微量開銷 | 無 |
| CompletionRequest::normalized_documents | 每次 completion 都序列化文件 | 大文件時有序列化開銷 | 低 |

### 17.2 效能優化

1. **Arc<RwLock> 讀取鎖**: ToolSet 使用讀取鎖允許多個工具同時執行，寫入（add/remove tool）時才獨佔
2. **stream::iter().then().try_fold()**: 使用 futures stream 做 lazy evaluation，不會一次載入所有文件
3. **nanoid 生成**: 每個工具呼叫的 `internal_call_id` 使用 nanoid 生成，速度快且碰撞率低
4. **prune_document**: vector store 回傳的文件會被修剪（>400 元素的陣列被移除），防止 context 爆炸

---

## 18. 與 clawtex-core 的完整差距對比

### 18.1 架構差異總表

| 面向 | Rig | clawtex-core | 差距分析 |
|------|-----|-------------|---------|
| **Provider 抽象** | 泛型 trait + associated types | `Provider` async trait + `Box<dyn>` | Rig 更型別安全，clawtex 更靈活 |
| **型別安全** | 編譯期綁定 (`Agent<M>`) | 執行期動態分發 | clawtex 的 TOML 驅動架構天生需要動態分發 |
| **工具系統** | `Tool` trait + `ToolServer` actor | `ToolRegistry` + HashMap | Rig 支援並行執行和動態工具選擇 |
| **串流** | `Stream<Item = RawStreamingChoice<R>>` | SSE text format | Rig 有完整的暫停/取消控制 |
| **RAG** | `VectorStoreIndex` + dynamic_context | 語意記憶 (SQLite) | Rig 有向量搜索，clawtex 無 |
| **錯誤型別** | 階層化 enum | 單一錯誤型別居多 | Rig 可做更細粒度的錯誤恢復 |
| **Provider 設定** | 環境變數 / builder API | TOML 設定檔 | clawtex 對非開發者更友好 |
| **多模態** | 完整 (image, audio, document) | 透過個別工具 (vision) | Rig 在 Message 層面原生支援 |
| **WASM** | 完整支援 | 無 | clawtex 不需要 WASM |
| **MCP** | rmcp 整合 | 自建 JSON-RPC 2.0 | 各有優劣 |
| **Smart Routing** | 無 | router + classifier | clawtex 獨有優勢 |
| **Cluster** | 無 | ClusterHub/Worker | clawtex 獨有優勢 |
| **工作流** | Pipeline (程式化) | Hands (TOML multi-phase) | clawtex 更適合非開發者 |
| **Approval** | 無 | Human-in-the-loop | clawtex 獨有優勢 |
| **E-Stop** | 無 | 全域緊急停止 | clawtex 獨有優勢 |
| **Cost Tracking** | 基本 Usage | 完整成本/收入追蹤 | clawtex 更完整 |
| **加密** | 無 | ChaCha20-Poly1305 | clawtex 獨有 |
| **Hook 系統** | 7 個攔截點 PromptHook | pre/post completion HookRunner | Rig 更細粒度 |
| **結構化輸出** | TypedPrompt + Extractor | 無 | clawtex 可引入 |
| **Tool RAG** | dynamic_tools + VectorStore | 全量傳送 24 工具 | clawtex 可引入 |

### 18.2 優先引入建議 (按 ROI 排序)

**高 ROI:**

1. **Tool RAG (動態工具選擇)**: 24 個工具全量傳送浪費大量 token。引入簡單的 TF-IDF 或 embedding 搜索，每次只傳 5-8 個工具，預估節省 40-60% token 成本。

2. **CompletionRequest 標準化**: 建立統一的中間格式，讓 router/rotation/classifier 與底層 provider 解耦。

3. **階層化錯誤處理**: 區分 ProviderError / ToolError / AgentError，讓 agent_runtime 可以根據錯誤型別選擇 retry、fallback、或回傳。

**中 ROI:**

4. **PromptHook 擴展**: 在 agent_runtime 加入 `on_tool_call` hook，可以實現: (a) 工具呼叫頻率限制; (b) 敏感工具的 pre-approval; (c) 工具呼叫日誌。

5. **Extractor 模式**: 在 Hands workflow 中需要結構化輸出時，自動切換到 submit-tool 模式。

6. **_opt Builder 模式**: 在所有 builder pattern 中採用 `_opt` 命名慣例，減少 Option 處理的樣板程式碼。

**低 ROI (長期):**

7. **串流事件 enum**: 替換 SSE text 為統一的事件模型，支援暫停/取消。

8. **OneOrMany 型別**: 消除「至少一個元素」假設相關的 unwrap。

9. **FileLoader 系統**: 擴展 file_read 工具支援 PDF/EPUB。

### 18.3 clawtex-core 的獨有優勢 (不應改變)

1. **設定驅動架構**: TOML 設定檔讓非開發者也能配置系統
2. **Cluster 系統**: Rig 沒有跨節點分散式執行能力
3. **Hands 工作流引擎**: 支援多階段、條件門控、排程執行
4. **Approval Gate**: 異步 Telegram 人機迴路
5. **E-Stop**: 全域緊急停止機制
6. **Revenue/Cost Tracking**: 完整的成本和收入追蹤
7. **Smart Routing**: 根據請求複雜度自動路由到不同 provider
8. **加密秘密管理**: ChaCha20-Poly1305 加密的 credential 管理
9. **Self-Evolution**: Felix-style 夜間自動改進系統
10. **Cron 排程**: 內建排程系統 (Rig 沒有)

---

## 19. 附錄: 關鍵檔案路徑索引

| 模組 | 路徑 |
|------|------|
| 核心 lib.rs | `rig/rig-core/src/lib.rs` |
| Agent 定義 | `rig/rig-core/src/agent/completion.rs` |
| Agent Builder (typestate) | `rig/rig-core/src/agent/builder.rs` |
| Agent as Tool | `rig/rig-core/src/agent/tool.rs` |
| PromptHook | `rig/rig-core/src/agent/prompt_request/hooks.rs` |
| Streaming PromptRequest | `rig/rig-core/src/agent/prompt_request/streaming.rs` |
| Completion traits | `rig/rig-core/src/completion/request.rs` |
| Message 型別 | `rig/rig-core/src/completion/message.rs` |
| CompletionModel | `rig/rig-core/src/completion/request.rs` |
| Streaming 抽象 | `rig/rig-core/src/streaming.rs` |
| Tool trait | `rig/rig-core/src/tool/mod.rs` |
| ToolServer actor | `rig/rig-core/src/tool/server.rs` |
| Extractor | `rig/rig-core/src/extractor.rs` |
| EmbeddingModel | `rig/rig-core/src/embeddings/embedding.rs` |
| Embed trait | `rig/rig-core/src/embeddings/embed.rs` |
| EmbeddingsBuilder | `rig/rig-core/src/embeddings/builder.rs` |
| VectorStoreIndex | `rig/rig-core/src/vector_store/mod.rs` |
| Pipeline Op | `rig/rig-core/src/pipeline/op.rs` |
| FileLoader | `rig/rig-core/src/loaders/file.rs` |
| PdfFileLoader | `rig/rig-core/src/loaders/pdf.rs` |
| HTTP Retry | `rig/rig-core/src/http_client/retry.rs` |
| OneOrMany | `rig/rig-core/src/one_or_many.rs` |
| OpenAI Client | `rig/rig-core/src/providers/openai/client.rs` |
| Anthropic Client | `rig/rig-core/src/providers/anthropic/client.rs` |
| Client Builder (Dyn) | `rig/rig-core/src/client/builder.rs` |
| CompletionClient | `rig/rig-core/src/client/completion.rs` |
| Provider 模組根 | `rig/rig-core/src/providers/mod.rs` |

---

> 本分析基於 rig-core v0.32.0 的原始碼逐行閱讀。所有路徑相對於 `references/rig/` 目錄。分析涵蓋 15 個核心原始碼檔案，超過 8000 行 Rust 程式碼。
