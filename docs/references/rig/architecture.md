# Rig 框架架構掃描

## 1. 專案概覽

**Rig** 是一個 Rust 構建的 LLM 驅動應用框架，專注於模組化和人性化設計。它提供統一的 API 來整合多個 LLM 提供商、向量存儲和代理編排能力。

### 核心使命
- 支援 20+ 個 LLM 提供商（OpenAI、Anthropic、Gemini、Groq 等）
- 支援 10+ 個向量存儲後端（MongoDB、LanceDB、PostgreSQL、Qdrant 等）
- 提供 Agent、RAG 和工具調用的完整生態
- 實現 WASM 相容性
- GenAI Semantic Convention 兼容

### 專案規模
- **核心庫**：rig-core v0.32.0 （main workspace member）
- **整合包**：rig-integrations/ （包含 13+ 個向量存儲和提供商適配器）
- **語言**：Rust Edition 2024
- **特性驅動**：audio, image, derive, experimental, discord-bot, pdf, epub, rayon, wasm, rmcp

---

## 2. 目錄結構

```
rig/
├── rig/                                 # 核心包目錄
│   ├── rig-core/                       # 主要庫（v0.32.0）
│   │   ├── src/
│   │   │   ├── lib.rs                  # 主入口（107 行模組聲明）
│   │   │   ├── agent/                  # Agent 實現
│   │   │   │   ├── mod.rs              # Agent 和 AgentBuilder
│   │   │   │   ├── builder.rs          # Builder 模式實現
│   │   │   │   ├── completion.rs       # 完成集成
│   │   │   │   ├── tool.rs             # Tool 定義
│   │   │   │   └── prompt_request/     # Prompt 處理
│   │   │   │       ├── hooks.rs        # 請求 hooks
│   │   │   │       └── streaming.rs    # 流式處理
│   │   │   ├── completion/             # LLM 完成契約
│   │   │   │   ├── mod.rs              # Traits: CompletionModel, Chat, Prompt
│   │   │   │   ├── message.rs          # 消息類型
│   │   │   │   └── request.rs          # 完成請求
│   │   │   ├── embeddings/             # 嵌入模型契約
│   │   │   │   ├── mod.rs              # EmbeddingModel trait
│   │   │   │   ├── embedding.rs        # Embedding 向量
│   │   │   │   ├── embed.rs            # Embed trait
│   │   │   │   ├── builder.rs          # 建構器
│   │   │   │   ├── distance.rs         # 距離計算
│   │   │   │   └── tool.rs             # Embedding 工具
│   │   │   ├── providers/               # LLM 提供商實現
│   │   │   │   ├── mod.rs              # 模組樹狀結構
│   │   │   │   ├── anthropic/          # Claude 適配
│   │   │   │   │   ├── client.rs       # 客戶端實現
│   │   │   │   │   ├── streaming.rs    # 流式回應
│   │   │   │   │   └── decoders/       # SSE/JSONL 解析
│   │   │   │   ├── openai/             # GPT-4/GPT-5 適配
│   │   │   │   │   ├── client.rs
│   │   │   │   │   ├── embedding.rs
│   │   │   │   │   ├── audio_generation.rs
│   │   │   │   │   ├── image_generation.rs
│   │   │   │   │   ├── transcription.rs
│   │   │   │   │   └── responses_api/  # o1/o3 響應模式
│   │   │   │   ├── gemini/             # Google Gemini
│   │   │   │   ├── groq/               # Groq 推理
│   │   │   │   ├── cohere/             # Cohere
│   │   │   │   ├── mistral/            # Mistral AI
│   │   │   │   ├── ollama/             # 本地模型
│   │   │   │   ├── azure.rs            # Azure OpenAI
│   │   │   │   ├── deepseek/           # DeepSeek
│   │   │   │   ├── huggingface/        # HF Inference
│   │   │   │   ├── openrouter/         # OpenRouter 路由
│   │   │   │   ├── together/           # Together AI
│   │   │   │   ├── voyageai.rs         # Voyage Embeddings
│   │   │   │   ├── xai.rs              # xAI Grok
│   │   │   │   ├── perplexity.rs       # Perplexity 搜尋
│   │   │   │   ├── hyperbolic.rs       # Hyperbolic
│   │   │   │   ├── mira.rs             # Mira
│   │   │   │   ├── galadriel.rs        # Galadriel
│   │   │   │   └── moonshot.rs         # Moonshot
│   │   │   ├── vector_store/           # 向量存儲抽象
│   │   │   │   ├── mod.rs              # VectorStoreIndex trait
│   │   │   │   └── in_memory_store.rs  # 內存實現
│   │   │   ├── client/                 # 通用客戶端
│   │   │   │   ├── mod.rs              # Client<Ext, H> 泛型
│   │   │   │   ├── builder.rs          # ClientBuilder
│   │   │   │   ├── completion.rs       # CompletionClient trait
│   │   │   │   ├── embeddings.rs       # EmbeddingClient trait
│   │   │   │   ├── image_generation.rs # ImageGenerationClient
│   │   │   │   ├── audio_generation.rs # AudioGenerationClient
│   │   │   │   ├── transcription.rs    # TranscriptionClient
│   │   │   │   ├── model_listing.rs    # ModelListingClient
│   │   │   │   └── verify.rs           # 提供商驗證
│   │   │   ├── tool.rs                 # Tool trait 定義
│   │   │   ├── tools/                  # 工具實現庫
│   │   │   ├── pipeline/               # 管道編排
│   │   │   │   ├── mod.rs              # Pipeline trait
│   │   │   │   ├── op.rs               # Operation trait
│   │   │   │   ├── try_op.rs           # 可失敗操作
│   │   │   │   ├── parallel.rs         # 並行執行
│   │   │   │   ├── conditional.rs      # 條件分支
│   │   │   │   └── agent_ops.rs        # Agent 操作
│   │   │   ├── loaders/                # 文檔加載器
│   │   │   │   ├── file.rs             # 文件 I/O
│   │   │   │   ├── pdf.rs              # PDF 解析
│   │   │   │   └── epub/               # EPUB 支援
│   │   │   ├── model/                  # 模型元數據
│   │   │   │   └── listing.rs          # 列表 API
│   │   │   ├── http_client/            # HTTP 基礎設施
│   │   │   │   ├── mod.rs              # HTTP 客戶端
│   │   │   │   ├── retry.rs            # 重試邏輯
│   │   │   │   ├── sse.rs              # Server-Sent Event
│   │   │   │   └── multipart.rs        # 分段上傳
│   │   │   ├── audio_generation.rs     # 音頻模型契約
│   │   │   ├── image_generation.rs     # 圖像模型契約
│   │   │   ├── transcription.rs        # 轉錄模型契約
│   │   │   ├── extractor.rs            # 結構化提取
│   │   │   ├── evals.rs                # 評估框架（實驗性）
│   │   │   ├── integrations/           # 應用集成
│   │   │   │   ├── cli_chatbot.rs      # CLI 代理
│   │   │   │   └── discord_bot.rs      # Discord 整合
│   │   │   ├── streaming.rs            # 流式通用介面
│   │   │   ├── telemetry.rs            # OpenTelemetry 支援
│   │   │   ├── json_utils.rs           # JSON 工具函數
│   │   │   ├── one_or_many.rs          # 多態類型
│   │   │   ├── wasm_compat.rs          # WASM 相容層
│   │   │   └── prelude.rs              # 常用重新導出
│   │   ├── examples/                   # 30+ 個範例
│   │   │   ├── agent.rs                # 基礎 Agent
│   │   │   ├── agent_autonomous.rs     # 自主 Agent
│   │   │   ├── agent_evaluator_optimizer.rs  # 評估者-優化器
│   │   │   ├── agent_orchestrator.rs   # 編排多 Agent
│   │   │   ├── rag.rs                  # RAG 實現
│   │   │   ├── pdf_agent.rs            # PDF 查詢 Agent
│   │   │   ├── streaming.rs            # 流式完成
│   │   │   └── ...
│   │   └── Cargo.toml                  # 依賴和特性定義
│   └── rig-derive/                     # 巨集和衍生
│
├── rig-integrations/                   # 插件包
│   ├── rig-bedrock/                    # AWS Bedrock 提供商
│   ├── rig-vertexai/                   # Google Vertex AI 提供商
│   ├── rig-fastembed/                  # FastEmbed 嵌入提供商
│   ├── rig-gemini-grpc/                # Gemini gRPC 客戶端
│   ├── rig-mongodb/                    # MongoDB 向量存儲
│   ├── rig-postgres/                   # PostgreSQL/pgvector
│   ├── rig-sqlite/                     # SQLite 向量存儲
│   ├── rig-lancedb/                    # LanceDB 向量存儲
│   ├── rig-qdrant/                     # Qdrant 向量存儲
│   ├── rig-milvus/                     # Milvus 向量存儲
│   ├── rig-neo4j/                      # Neo4j 知識圖表
│   ├── rig-scylladb/                   # ScyllaDB 向量存儲
│   ├── rig-s3vectors/                  # AWS S3Vectors 存儲
│   ├── rig-vectorize/                  # Cloudflare Vectorize
│   └── rig-helixdb/                    # HelixDB 向量存儲
│
├── Cargo.toml                          # Workspace 定義
├── README.md                           # 主文檔
├── AGENTS.md                           # AI 貢獻指引
├── CONTRIBUTING.md                     # 開發指南
├── LICENSE                             # MIT 許可證
└── .github/                            # CI/CD 和配置
    ├── workflows/                      # GitHub Actions
    ├── instructions/                   # AI 指令
    ├── prompts/                        # AI 提示
    └── ISSUE_TEMPLATE/                 # PR 模板
```

---

## 3. 核心 Trait 和 Struct

### 3.1 基礎 Trait

```
CompletionModel<H>
├─ async fn completion(&self, request: Request) -> Result<Response>
├─ async fn stream(&self, request: Request) -> Result<Stream>
└─ fn model(&self) -> &str

EmbeddingModel<H>
├─ async fn embed_string(&self, text: &str) -> Result<Embedding>
├─ async fn embed_strings(&self, texts: &[&str]) -> Result<Vec<Embedding>>
└─ fn embed_queries(&self, ...) -> Result<Vec<Embedding>>

VectorStoreIndex
├─ async fn add(&mut self, documents: Vec<Document>) -> Result<()>
├─ async fn search(&self, query_embedding: &Embedding, limit: u32) -> Result<Vec<Document>>
└─ async fn list(&self) -> Result<Vec<Document>>

Tool
├─ fn name() -> &'static str
├─ fn definition() -> ToolDefinition
└─ async fn call(&self, args: Value) -> Result<Value>

Agent<M>
├─ async fn prompt(&self, prompt: &str) -> Result<String>
├─ async fn chat(&self, prompt: &str, messages: Vec<Message>) -> Result<String>
└─ async fn completion(...) -> Result<CompletionBuilder>
```

### 3.2 主要 Struct

```rust
// 提供商客戶端通用模式
pub struct Client<Ext = Nothing, H = reqwest::Client> {
    api_key: String,
    base_url: String,
    http_client: H,
    _marker: PhantomData<Ext>,
}

// Agent 構建器
pub struct AgentBuilder<M: CompletionModel> {
    model: M,
    preamble: Option<String>,
    context: Vec<String>,
    tools: Vec<Box<dyn Tool>>,
    temperature: Option<f32>,
    additional_params: Option<Value>,
}

// 完成請求
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
    pub tools: Option<Vec<ToolDefinition>>,
    pub tool_choice: Option<ToolChoice>,
}

// 訊息類型
pub enum Message {
    SystemMessage { content: String },
    UserMessage { content: String },
    AssistantMessage {
        content: String,
        tool_calls: Option<Vec<ToolCall>>,
    },
    ToolResultMessage {
        tool_use_id: String,
        content: String,
    },
}

// 嵌入式向量
pub struct Embedding {
    pub vec: Vec<f32>,
}

// 向量存儲文檔
pub struct Document {
    pub id: String,
    pub content: String,
    pub embedding: Embedding,
    pub metadata: Map<String, Value>,
}
```

### 3.3 能力系統

```rust
// 提供商聲明其能力
pub trait Capabilities<H> {
    type Completion;      // 使用 Capable<T> 或 Nothing
    type Embeddings;
    type ImageGeneration;
    type AudioGeneration;
    type Transcription;
    type ModelListing;
}

// 標記類型
pub struct Capable<T>(pub T);
pub struct Nothing;  // 不支援的能力
```

---

## 4. 啟動流程

```
┌─────────────────────────────────────────────────────────────┐
│ 1. 初始化提供商客戶端                                      │
├─────────────────────────────────────────────────────────────┤
│ let openai = openai::Client::from_env();                   │
│  ↓                                                           │
│  a) 讀取環境變數（OPENAI_API_KEY）                         │
│  b) 構建 HTTP 客戶端（reqwest::Client）                    │
│  c) 驗證連接（GET /models）                                 │
│  d) 返回 Client<OpenAIExt, reqwest::Client>                │
└─────────────────────────────────────────────────────────────┘
         ↓
┌─────────────────────────────────────────────────────────────┐
│ 2. 建立 Agent（生成器模式）                                │
├─────────────────────────────────────────────────────────────┤
│ let agent = openai.agent("gpt-4o")                         │
│   .preamble("You are a helpful assistant")                 │
│   .tool(my_tool)                                           │
│   .context(document)                                       │
│   .temperature(0.8)                                        │
│   .build()                                                 │
│  ↓                                                           │
│  a) 建立 AgentBuilder<CompletionModel>                     │
│  b) 儲存系統提示（preamble）                                │
│  c) 收集 Tool 和 Document                                   │
│  d) 組裝 Agent<CompletionModel>                            │
└─────────────────────────────────────────────────────────────┘
         ↓
┌─────────────────────────────────────────────────────────────┐
│ 3. 執行 Prompt/Chat                                        │
├─────────────────────────────────────────────────────────────┤
│ let response = agent.prompt("Question?").await            │
│  ↓                                                           │
│  a) 組裝消息向量                                            │
│     ├─ SystemMessage(preamble)                              │
│     ├─ ContextMessages(RAG 檢索)                            │
│     └─ UserMessage(prompt)                                  │
│  b) 構建 CompletionRequest                                 │
│  c) 附加工具定義（如果存在）                               │
│  d) HTTP POST 到 LLM 端點                                   │
│  e) 解析回應                                                │
│  f) 執行工具（如果需要）- 回環                             │
│  g) 返回最終文本                                            │
└─────────────────────────────────────────────────────────────┘
         ↓
┌─────────────────────────────────────────────────────────────┐
│ 4. RAG 增強（可選）                                        │
├─────────────────────────────────────────────────────────────┤
│ let store = VectorStoreIndex::add(documents).await         │
│  ↓                                                           │
│  a) 使用嵌入模型編碼文檔                                    │
│     embedding_model.embed_strings(texts)                   │
│  b) 儲存向量 + 中繼資料                                      │
│  c) 在代理檢索時：                                          │
│     store.search(query_embedding, limit=5)                 │
│  d) 將最相關文檔注入上下文                                  │
└─────────────────────────────────────────────────────────────┘
         ↓
┌─────────────────────────────────────────────────────────────┐
│ 5. 流式回應（可選）                                        │
├─────────────────────────────────────────────────────────────┤
│ let mut stream = agent.stream_prompt("Q").await           │
│ while let Some(chunk) = stream.next().await { ... }       │
│  ↓                                                           │
│  a) 開啟 HTTP SSE 連接                                      │
│  b) 逐個解析流事件                                         │
│  c) 支援文本和工具呼叫事件                                 │
│  d) 用戶可即時消費增量                                     │
└─────────────────────────────────────────────────────────────┘
```

---

## 5. 資料流 ASCII 圖

### 5.1 標準 Agent 流程

```
┌──────────────┐
│  用戶輸入    │
└──────┬───────┘
       │ "What is Rust?"
       ↓
┌──────────────────────────────────┐
│ Agent::prompt(input)             │
│ ├─ 組裝消息向量                   │
│ └─ 委派至 CompletionModel        │
└──────┬───────────────────────────┘
       │
       ↓
┌──────────────────────────────────┐
│ CompletionModel::completion()    │
│ ├─ 構建 HTTP 請求                 │
│ ├─ 新增 Bearer Token              │
│ └─ JSON 序列化 Request            │
└──────┬───────────────────────────┘
       │ POST /v1/chat/completions
       │ {
       │   "model": "gpt-4o",
       │   "messages": [...],
       │   "temperature": 0.7
       │ }
       ↓
┌──────────────────────────────────┐
│ HTTP 層 (reqwest)                 │
│ ├─ SSL/TLS 連接                   │
│ ├─ 重試邏輯                       │
│ └─ 超時處理                       │
└──────┬───────────────────────────┘
       │
       ↓ 網路
   [LLM API]
       ↑
       │ 200 OK
       │ {
       │   "choices": [{
       │     "message": {
       │       "content": "Rust is..."
       │     }
       │   }]
       │ }
       │
       ↓
┌──────────────────────────────────┐
│ 解析回應                          │
│ ├─ JSON 反序列化                  │
│ ├─ 提取 content                   │
│ └─ 工具呼叫偵測                   │
└──────┬───────────────────────────┘
       │
       ├─ 無工具呼叫 ──→ 返回文本
       │
       └─ 檢測工具呼叫 ──→ 分發至 Tool::call()
                           ↓
                      執行本地代碼
                           ↓
                      收集結果
                           ↓
                      迴圈: 新增 ToolResult 消息
                      重新調用 Agent
```

### 5.2 RAG 查詢流程

```
┌─────────────────────────────┐
│ 文檔索引 (離線)              │
├─────────────────────────────┤
│ Documents:                  │
│ ├─ "Rust Language Guide"    │
│ ├─ "Async/Await Tutorial"   │
│ └─ "Cargo Manifest"         │
└──────┬──────────────────────┘
       │
       ↓
┌──────────────────────────────┐
│ EmbeddingModel::embed_strings()
│ ├─ API 呼叫：text-embedding-3-small
│ └─ 返回 Vec<Embedding>       │
└──────┬───────────────────────┘
       │ [vec![0.1, -0.2, ...], ...]
       │
       ↓
┌──────────────────────────────┐
│ VectorStoreIndex::add()      │
│ ├─ 儲存 embedding + metadata │
│ └─ 索引已建立                 │
└──────┬───────────────────────┘
       │
       │ ═══════════════════════════
       │ (執行時查詢)
       │ ═══════════════════════════
       │
       ↓
┌──────────────────────────────┐
│ 使用者查詢："What is async?" │
└──────┬───────────────────────┘
       │
       ↓
┌──────────────────────────────┐
│ EmbeddingModel::embed_string()
│ (查詢本身)                   │
└──────┬───────────────────────┘
       │ query_embedding
       │
       ↓
┌──────────────────────────────┐
│ VectorStoreIndex::search()   │
│ ├─ 計算相似度 (cosine)        │
│ ├─ 檢索 top-5 文檔            │
│ └─ 返回 [Document, ...]     │
└──────┬───────────────────────┘
       │ [
       │   Document {
       │     id: "async-1",
       │     content: "async/await..."
       │   },
       │   ...
       │ ]
       │
       ↓
┌──────────────────────────────┐
│ Agent::prompt()              │
│ ├─ 插入檢索文檔至上下文       │
│ │  SystemMessage              │
│ │  ContextMessages (RAG docs) │
│ │  UserMessage (query)        │
│ └─ 調用 CompletionModel      │
└──────┬───────────────────────┘
       │
       ↓
    [LLM 使用上下文生成回應]
       │
       ↓
    返回答案
```

### 5.3 多提供商路由

```
┌──────────────────────────┐
│ 提供商池配置              │
├──────────────────────────┤
│ primary: OpenAI (GPT-4o) │
│ fallback_1: Gemini       │
│ fallback_2: Groq         │
└──────┬───────────────────┘
       │
       ↓
┌──────────────────────────────────┐
│ Agent::prompt(input)             │
└──────┬───────────────────────────┘
       │
       ↓
   嘗試 primary (OpenAI)
       │
       ├─ 成功 ──→ 返回結果
       │
       ├─ 速率限制 (429) ──→ 下一個
       │
       ├─ 超時 ──→ 下一個
       │
       └─ 失敗 ──→ 嘗試 fallback_1
              │
              ├─ Gemini 成功 ──→ 返回
              │
              └─ 失敗 ──→ 嘗試 fallback_2
                     │
                     ├─ Groq 成功 ──→ 返回
                     │
                     └─ 全部失敗 ──→ Error
```

---

## 6. 子系統清單

### P0 (關鍵路徑)

1. **完成模型 (CompletionModel Trait)**
   - 實現：OpenAI, Anthropic, Gemini, Groq, Mistral
   - 狀態：穩定
   - 關鍵檔案：`completion/mod.rs`, `providers/*/client.rs`

2. **嵌入模型 (EmbeddingModel Trait)**
   - 實現：OpenAI, Cohere, Mistral, VoyageAI, Together
   - 狀態：穩定
   - 關鍵檔案：`embeddings/mod.rs`, `providers/*/embedding.rs`

3. **Agent 編排**
   - 功能：Preamble, Tools, RAG, 流式, Chat 歷史
   - 狀態：穩定
   - 關鍵檔案：`agent/mod.rs`, `agent/builder.rs`

4. **HTTP 客戶端基礎設施**
   - 功能：重試, SSE, 多部分, 自定義 TLS
   - 狀態：穩定
   - 關鍵檔案：`http_client/mod.rs`

5. **提供商樹狀結構**
   - 18+ 提供商支援
   - 能力系統：聲明支援功能
   - 狀態：穩定 (定期新增提供商)

### P1 (高優先級)

1. **向量存儲抽象 (VectorStoreIndex)**
   - 核心 trait：`vector_store/mod.rs`
   - 實現：InMemory (rig-core) + 13 個整合包
   - 狀態：穩定
   - 關鍵檔案：`vector_store/mod.rs`, `rig-integrations/rig-*/`

2. **工具系統 (Tool Trait)**
   - 定義：`tool.rs`
   - 衍生巨集：`#[rig_tool]` (rig-derive)
   - 類型：結構化輸入, JSON 序列化
   - 狀態：穩定
   - 關鍵檔案：`tool.rs`, `rig-derive/`

3. **流式 API**
   - 支援：完成流, 工具呼叫流
   - 編碼：SSE (Server-Sent Events)
   - 狀態：穩定
   - 關鍵檔案：`streaming.rs`, `http_client/sse.rs`

4. **管道編排 (Pipeline)**
   - 操作：Sequential, Parallel, Conditional
   - 應用：多步驟 Agent 工作流
   - 狀態：穩定
   - 關鍵檔案：`pipeline/mod.rs`, `pipeline/op.rs`

5. **文檔加載器**
   - 支援：PDF (lopdf), EPUB, 純文本
   - 應用：RAG 文檔預處理
   - 狀態：穩定
   - 關鍵檔案：`loaders/pdf.rs`, `loaders/epub/`

### P2 (增強功能)

1. **音頻/影像生成**
   - 模型：OpenAI, Hyperbolic (音頻), Hugging Face
   - 特性：`audio`, `image` 功能標誌
   - 狀態：穩定
   - 關鍵檔案：`audio_generation.rs`, `image_generation.rs`

2. **轉錄能力 (Transcription)**
   - 提供商：OpenAI, Gemini, Mistral, Hugging Face
   - 格式：WAV, MP3, M4A
   - 狀態：穩定
   - 關鍵檔案：`transcription.rs`

3. **結構化提取 (Extractor)**
   - 用途：提取物件為 Rust struct
   - 使用：serde_json 驅動
   - 狀態：穩定
   - 關鍵檔案：`extractor.rs`

4. **應用整合**
   - Discord 機器人：`integrations/discord_bot.rs` (feature: discord-bot)
   - CLI 聊天機器人：`integrations/cli_chatbot.rs`
   - 狀態：穩定
   - 關鍵檔案：`integrations/`

5. **評估框架 (Evals)**
   - 用途：LLM 輸出評估
   - 特性：`experimental` 功能
   - 狀態：實驗性
   - 關鍵檔案：`evals.rs`

6. **WASM 相容性**
   - 特性：`wasm` 功能
   - 支援：核心庫 (無 syscall 依賴)
   - 狀態：穩定
   - 關鍵檔案：`wasm_compat.rs`

7. **遠端模型控制協議 (RMCP)**
   - 支援：MCP 客戶端整合
   - 特性：`rmcp` 功能
   - 狀態：穩定
   - 關鍵檔案：`Cargo.toml` (rmcp 依賴)

---

## 7. 提供商對應表

| 提供商 | 完成 | 嵌入 | 音頻 | 影像 | 轉錄 | 狀態 |
|--------|------|------|------|------|------|------|
| OpenAI | ✓ | ✓ | ✓ | ✓ | ✓ | 穩定 |
| Anthropic | ✓ | ✗ | ✗ | ✗ | ✗ | 穩定 |
| Google Gemini | ✓ | ✓ | ✓ | ✓ | ✓ | 穩定 |
| Groq | ✓ | ✗ | ✗ | ✗ | ✗ | 穩定 |
| Mistral | ✓ | ✓ | ✗ | ✗ | ✓ | 穩定 |
| Cohere | ✓ | ✓ | ✗ | ✗ | ✗ | 穩定 |
| Together AI | ✓ | ✓ | ✗ | ✗ | ✗ | 穩定 |
| Ollama | ✓ | ✓ | ✗ | ✗ | ✗ | 穩定 |
| xAI | ✓ | ✗ | ✗ | ✗ | ✗ | 穩定 |
| DeepSeek | ✓ | ✗ | ✗ | ✗ | ✗ | 穩定 |
| Voyageai | ✗ | ✓ | ✗ | ✗ | ✗ | 穩定 |
| Hugging Face | ✓ | ✗ | ✓ | ✓ | ✓ | 穩定 |
| Hyperbolic | ✓ | ✗ | ✓ | ✗ | ✗ | 穩定 |
| Azure OpenAI | ✓ | ✓ | ✓ | ✓ | ✓ | 穩定 |
| Perplexity | ✓ | ✗ | ✗ | ✗ | ✗ | 穩定 |
| OpenRouter | ✓ | ✓ | ✗ | ✗ | ✗ | 穩定 |
| Moonshot | ✓ | ✗ | ✗ | ✗ | ✗ | 穩定 |
| Galadriel | ✓ | ✗ | ✗ | ✗ | ✗ | 穩定 |
| Mira | ✓ | ✗ | ✗ | ✗ | ✗ | 穩定 |
| AWS Bedrock | ✓ | ✓ | ✗ | ✓ | ✓ | 整合包 |
| Google Vertex | ✓ | ✓ | ✗ | ✓ | ✓ | 整合包 |

---

## 8. 關鍵設計模式

### 8.1 通用客戶端架構

```rust
// 提供商擴展
pub struct Client<Ext = Nothing, H = reqwest::Client> {
    api_key: String,
    base_url: String,
    http_client: H,
}

// 能力聲明
impl<H> Capabilities<H> for OpenAIExt {
    type Completion = Capable<CompletionModel<H>>;
    type Embeddings = Capable<EmbeddingModel<H>>;
    type ImageGeneration = Capable<ImageGenerationModel<H>>;
}

// 使用 Phantom 類型隱藏複雜性
let openai = openai::Client::from_env(); // 自動推斷 <OpenAIExt, reqwest::Client>
```

### 8.2 Builder 模式

所有可配置類型遵循 builder 模式：
- 流暢 API
- 預設值安全
- 延遲驗證

### 8.3 Trait 驅動的可擴展性

新提供商：實現 `CompletionModel`, `EmbeddingModel`
新向量存儲：實現 `VectorStoreIndex`
新工具：實現 `Tool`
新管道：實現 `Pipeline` 和 `Operation`

### 8.4 非同步優先

- 所有 I/O 操作都是 `async`
- 使用 `tokio` 執行時
- 支援流式和批處理

---

## 9. 依賴關係概況

**核心依賴**：
- `tokio` - 非同步執行時
- `reqwest` - HTTP 客戶端 (支援 TLS 選擇)
- `serde`/`serde_json` - 序列化
- `tracing` - 可觀測性
- `thiserror` - 錯誤類型

**可選**：
- `rig-derive` - 巨集 (#[rig_tool], #[Embed])
- `rmcp` - 遠端模型控制協議
- `serenity` - Discord 整合
- `lopdf` / `epub` - 文檔格式
- `rayon` - 並行處理

---

## 10. 特性組合

```bash
# 最小化
cargo add rig-core

# 完整功能
cargo add rig-core --features "derive,pdf,epub,audio,image"

# WASM 應用
cargo add rig-core --features "wasm"

# 生產環境 (含中間件)
cargo add rig-core --features "reqwest-middleware-rustls"
```

---

## 11. 注意事項

1. **版本**: v0.32.0 (頻繁發布，未 1.0)
2. **破壞性變更**: 預期在小版本升級中
3. **AI 貢獻**: AGENTS.md 提供詳細指引
4. **測試**: 707+ 測試覆蓋
5. **社區**: Rust AI 開發中的標準選擇 (St Jude, Nethermind, Neon 使用中)

---

**文檔日期**: 2026-03-13
**掃描對象**: Rig v0.32.0 (github.com/0xPlaygrounds/rig)
