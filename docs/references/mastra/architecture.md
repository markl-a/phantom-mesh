# Mastra 架構文檔

## 專案概覽

Mastra 是一個現代 TypeScript 多功能 AI 框架，專為構建生產級 AI 應用和代理系統設計。採用模塊化架構、依賴注入模式，支持與 React/Next.js/Node.js 無縫集成，並提供完整的代理、工作流、記憶和可觀測性能力。

**核心特性**：
- TypeScript 優先：完整類型安全與開發者體驗
- 模型路由：40+ 提供商統一介面（OpenAI/Anthropic/Gemini/Groq）
- 代理系統：自主決策與工具調用
- 工作流引擎：圖論式步驟協調（.then/.branch/.parallel）
- 記憶架構：對話歷史、語義回憶、工作記憶
- MCP 支持：Model Context Protocol 伺服器
- 可觀測性：內建追蹤與評估框架
- Y Combinator W25 創業公司

**適用場景**：
- AI 聊天機器人與虛擬助手
- 多步驟自動化工作流
- 長期有狀態代理系統
- 與 React/Next.js UI 集成的 AI 應用
- 企業知識庫檢索系統

## 目錄結構

```
mastra/
├── packages/                      # 核心包集合
│   ├── core/                      # P0 Mastra 核心框架
│   │   ├── src/
│   │   │   ├── mastra/            # P0 Mastra 類與配置
│   │   │   │   ├── index.ts       # P0 export Mastra 主類
│   │   │   │   └── config.ts      # P0 MastraConfig 接口
│   │   │   │
│   │   │   ├── agent/             # P0 代理系統
│   │   │   │   ├── agent.ts       # P0 Agent 類與執行
│   │   │   │   ├── types.ts       # P0 AgentConfig, Tool[] 定義
│   │   │   │   └── executor.ts    # P0 代理執行循環
│   │   │   │
│   │   │   ├── workflows/         # P0 工作流引擎
│   │   │   │   ├── workflow.ts    # P0 Workflow 類
│   │   │   │   ├── step.ts        # P0 Step 定義
│   │   │   │   ├── runner.ts      # P0 工作流執行器
│   │   │   │   └── builders.ts    # P0 .then(), .branch(), .parallel()
│   │   │   │
│   │   │   ├── tools/             # P0 工具系統
│   │   │   │   ├── tool.ts        # P0 BaseTool 抽象類
│   │   │   │   ├── dynamic.ts     # P1 DynamicToolLoader（MCP/API）
│   │   │   │   └── registry.ts    # P0 工具註冊表
│   │   │   │
│   │   │   ├── memory/            # P0 多層記憶系統
│   │   │   │   ├── memory.ts      # P0 Memory 基類
│   │   │   │   ├── conversation.ts # P0 ConversationHistory
│   │   │   │   ├── working.ts     # P1 WorkingMemory（短期）
│   │   │   │   ├── semantic.ts    # P1 SemanticRecall（向量）
│   │   │   │   └── providers.ts   # P1 記憶提供商
│   │   │   │
│   │   │   ├── models/            # P0 模型路由層
│   │   │   │   ├── router.ts      # P0 ModelRouter （40+ 提供商）
│   │   │   │   ├── providers/     # P0 供應商適配器
│   │   │   │   │   ├── openai.ts  # P0 OpenAI 集成
│   │   │   │   │   ├── anthropic.ts # P0 Anthropic 集成
│   │   │   │   │   ├── gemini.ts  # P0 Google Gemini
│   │   │   │   │   ├── groq.ts    # P0 Groq Inferencing
│   │   │   │   │   └── [others]/  # P1 其他供應商
│   │   │   │   └── types.ts       # P0 ModelConfig, ModelMessage
│   │   │   │
│   │   │   ├── storage/           # P0 可插拔存儲層
│   │   │   │   ├── interface.ts   # P0 Storage 抽象介面
│   │   │   │   ├── memory.ts      # P0 MemoryStorage（測試）
│   │   │   │   └── providers.ts   # P1 DB/S3 提供商
│   │   │   │
│   │   │   ├── context/           # P0 請求上下文傳播
│   │   │   │   ├── context.ts     # P0 RequestContext 類
│   │   │   │   └── middleware.ts  # P1 上下文中間件
│   │   │   │
│   │   │   ├── server/            # P1 伺服器適配器
│   │   │   │   ├── express.ts     # P1 Express 適配器
│   │   │   │   └── nextjs.ts      # P1 Next.js 路由
│   │   │   │
│   │   │   └── types/             # P0 全局類型定義
│   │   │       ├── agent.ts       # P0 AgentState, AgentMessage
│   │   │       ├── workflow.ts    # P0 WorkflowState, StepResult
│   │   │       └── common.ts      # P0 MessageType, ToolCall
│   │   │
│   │   └── tests/
│   │
│   ├── memory/                    # P1 記憶獨立包
│   │   ├── src/
│   │   │   ├── conversation-history.ts # P1 對話記憶
│   │   │   ├── semantic-recall.ts     # P1 向量搜索回憶
│   │   │   └── working-memory.ts      # P1 工作記憶
│   │   └── tests/
│   │
│   ├── rag/                       # P1 RAG（檢索增強生成）
│   │   ├── src/
│   │   │   ├── retriever.ts       # P1 Retriever 接口
│   │   │   ├── embeddings.ts      # P1 Embedding 供應商
│   │   │   ├── vector-db.ts       # P1 Pinecone/Weaviate 集成
│   │   │   └── pipeline.ts        # P1 RAG Pipeline 協調
│   │   └── tests/
│   │
│   ├── evals/                     # P1 評估與基準
│   │   ├── src/
│   │   │   ├── evaluator.ts       # P1 Evaluator 框架
│   │   │   ├── metrics.ts         # P1 內建指標（BLEU/ROUGE）
│   │   │   └── datasets.ts        # P1 測試數據集
│   │   └── tests/
│   │
│   ├── cli/                       # P1 命令行工具
│   │   ├── src/
│   │   │   ├── commands/          # P1 CLI 命令
│   │   │   ├── generate.ts        # P1 代碼生成
│   │   │   └── dev.ts             # P1 開發伺服器
│   │   └── tests/
│   │
│   ├── integrations/              # P1 第三方集成
│   │   ├── slack/
│   │   ├── discord/
│   │   ├── email/
│   │   └── ...
│   │
│   └── observability/             # P1 可觀測性
│       ├── src/
│       │   ├── tracer.ts          # P1 追蹤收集器
│       │   ├── metrics.ts         # P1 指標記錄
│       │   └── exporters.ts       # P1 出口至 OpenTelemetry/DataDog
│       └── tests/
│
├── @deployers/                    # P1 部署適配器
│   ├── render/                    # P1 Render.com 部署
│   └── vercel/                    # P1 Vercel 部署
│
├── @server-adapters/              # P1 伺服器適配器
│   ├── express/
│   ├── fastify/
│   └── hono/
│
├── @stores/                       # P1 存儲實現
│   ├── postgres/
│   ├── mongodb/
│   └── drizzle/
│
├── @mcp/                          # P1 Model Context Protocol
│   ├── server/                    # P1 MCP 伺服器框架
│   └── client/                    # P1 MCP 客戶端
│
├── docs/                          # 文檔站點（Nextra）
├── examples/                      # 完整示例應用
├── pnpm-workspace.yaml            # P0 pnpm 工作區配置
└── turbo.json                     # P0 Turborepo 配置
```

## 核心 Class/Type

### Mastra（主類）

```typescript
class Mastra {
    /**中央配置與依賴注入樞紐*/

    // 配置
    config: MastraConfig              // P0 全局配置
    models: ModelRouter               // P0 模型路由
    storage: Storage                  // P0 存儲後端
    memory: MemoryProvider            // P0 記憶系統

    // 代理與工作流
    agents: Map<string, Agent>        // P0 註冊的代理
    workflows: Map<string, Workflow>  // P0 註冊的工作流

    // 初始化
    constructor(config: MastraConfig) {
        // P0 依賴注入
        this.models = new ModelRouter(config.models)
        this.storage = config.storage || new MemoryStorage()
        this.memory = new MemoryProvider(this.storage)
    }

    // 主要方法
    agent(name: string): Agent                    // P0 獲取代理
    workflow(name: string): Workflow              // P0 獲取工作流
    registerAgent(agent: Agent): void             // P0 註冊代理
    registerWorkflow(workflow: Workflow): void    // P0 註冊工作流

    // 伺服器
    server(adapter: "express" | "nextjs") -> Server  // P1 創建伺服器
}


// 配置接口
interface MastraConfig {
    models: {
        provider: "openai" | "anthropic" | ...   // P0
        apiKey: string                           // P0
        defaultModel: string                     // P0
    }
    storage?: StorageProvider                    // P1
    memory?: MemoryConfig                        // P1
}
```

### Agent（自主代理）

```typescript
class Agent {
    /**具有工具與記憶的自主決策單位"""

    // 識別與配置
    id: string                        // P0 代理 ID
    name: string                      // P0 代理名稱
    instructions: string              // P0 系統指令

    // 資源
    model: string                     // P0 使用的模型名稱
    tools: Tool[]                     // P0 可用工具列表
    memory: ConversationHistory       // P0 對話記憶

    // 行為控制
    config: AgentConfig               // P0 執行配置
    _executor: AgentExecutor          // P0 執行引擎

    // 初始化
    constructor(
        name: string,
        instructions: string,
        tools: Tool[],
        model: string = "default"
    ) {
        this.id = uuid()
        this.memory = new ConversationHistory()
        this._executor = new AgentExecutor(this)
    }

    // 主要方法
    async execute(input: string, context?: RequestContext): Promise<AgentResult> {
        // P0 執行推理循環
        return this._executor.run(input, context)
    }

    async stream(input: string): AsyncIterator<AgentEvent> {
        // P1 流式執行
        return this._executor.stream(input)
    }

    // 記憶管理
    async getMemory(query: string): Promise<string[]>     // P0 查詢記憶
    async rememberConversation(messages: Message[])       // P0 存儲對話
    async clearMemory(): Promise<void>                    // P0 清除記憶
}


// 代理結果
interface AgentResult {
    output: string                    // P0 最終輸出
    toolCalls: ToolCall[]             // P0 使用的工具調用
    messages: Message[]               // P0 完整對話歷史
    executionTime: number             // P1 執行時間（毫秒）
}


// 工具調用
interface ToolCall {
    toolName: string                  // P0 工具名稱
    arguments: Record<string, any>    // P0 參數
    result: string                    // P0 執行結果
    timestamp: Date                   // P0 時間戳
}
```

### Workflow（工作流）

```typescript
class Workflow {
    /**步驟式執行的有向圖工作流"""

    // 定義
    id: string                        // P0 工作流 ID
    name: string                      // P0 工作流名稱
    steps: Step[]                     // P0 步驟列表

    // 狀態
    graph: WorkflowGraph              // P0 執行圖
    _runner: WorkflowRunner           // P0 執行引擎

    // 初始化
    constructor(name: string) {
        this.id = uuid()
        this.steps = []
        this.graph = new WorkflowGraph()
        this._runner = new WorkflowRunner(this)
    }

    // 流利 API（Builder Pattern）
    addStep(name: string, handler: StepHandler): Step {
        // P0 添加步驟
        const step = new Step(name, handler)
        this.steps.push(step)
        return step
    }

    then<T>(handler: (prev: T) => T | Promise<T>): Workflow {
        // P0 順序步驟
        // 語法糖：return this.addStep(uuid(), handler)
        return this
    }

    branch<T>(
        condition: (state: T) => string,  // P1 條件函數 → 步驟名稱
        branches: Record<string, StepHandler>
    ): Workflow {
        // P1 條件分支
        return this
    }

    parallel<T>(
        handlers: StepHandler[]       // P1 並行執行
    ): Workflow {
        // P1 並行步驟
        return this
    }

    // 執行
    async execute(input: any, context?: RequestContext): Promise<WorkflowResult> {
        // P0 執行工作流
        return this._runner.run(input, context)
    }

    // 暫停與恢復
    async suspend(reason: string): Promise<void>         // P1 暫停執行
    async resume(approval?: any): Promise<void>          // P1 恢復執行
}


// 步驟
class Step {
    id: string                        // P0 步驟 ID
    name: string                      // P0 步驟名稱
    handler: StepHandler              // P0 執行函數
    retryPolicy?: RetryPolicy         // P1 重試策略
    timeout?: number                  // P1 超時時間

    constructor(name: string, handler: StepHandler) {
        this.id = uuid()
        this.name = name
        this.handler = handler
    }
}


// 工作流執行結果
interface WorkflowResult {
    status: "success" | "failed" | "suspended"  // P0 狀態
    output: any                       // P0 最終輸出
    steps: StepResult[]               // P0 各步驟結果
    executionTime: number             // P1 總執行時間
}

interface StepResult {
    stepName: string                  // P0 步驟名稱
    status: "completed" | "failed" | "skipped"  // P0 狀態
    output: any                       // P0 輸出
    duration: number                  // P1 執行時間
    error?: Error                     // P1 錯誤信息
}
```

### Tool（工具）

```typescript
class BaseTool {
    /**原子工具單位"""

    // 元數據
    id: string                        // P0 工具 ID
    name: string                      // P0 工具名稱
    description: string               // P0 功能描述
    schema: JSONSchema                // P0 參數 JSON Schema

    // 實現
    execute: (input: any) => Promise<string>  // P0 執行邏輯

    constructor(
        name: string,
        description: string,
        schema: JSONSchema,
        execute: Function
    ) {
        this.id = `tool_${name}`
        this.name = name
        this.description = description
        this.schema = schema
        this.execute = execute
    }

    // 主要方法
    async call(args: Record<string, any>): Promise<string> {
        // P0 驗證參數 → 執行
        return this.execute(args)
    }
}


// 動態工具加載器（P1）
class DynamicToolLoader {
    loadFromMCP(serverUrl: string): Promise<Tool[]>      // P1 MCP 伺服器
    loadFromAPI(apiSpec: OpenAPISpec): Promise<Tool[]>   // P1 OpenAPI
    loadFromAgent(agentId: string): Promise<Tool[]>      // P1 其他代理工具
}
```

### Memory（記憶系統）

```typescript
class Memory {
    /**多層記憶管理"""

    // 層級
    conversationHistory: ConversationHistory  // P0 對話記錄
    workingMemory: WorkingMemory              // P1 短期工作記憶
    semanticRecall: SemanticRecall           // P1 向量檢索記憶

    // 主要方法
    async recall(query: string): Promise<string[]> {
        // P0 查詢記憶（混合策略）
        const conversation = await conversationHistory.search(query)
        const semantic = await semanticRecall.query(query)
        return [...conversation, ...semantic]
    }

    async remember(content: string): Promise<void> {
        // P0 存儲信息
        await conversationHistory.add(content)
        await semanticRecall.embed(content)
    }

    // 工作記憶（短期）
    async setWorkingMemory(key: string, value: any): Promise<void>   // P1
    async getWorkingMemory(key: string): Promise<any>                 // P1
}

// 對話記憶
class ConversationHistory {
    messages: Message[]               // P0 消息列表

    async add(message: Message): Promise<void> {
        // P0 添加消息
        this.messages.push(message)
    }

    async search(query: string): Promise<Message[]> {
        // P0 向量或關鍵詞搜索
    }

    async clear(): Promise<void> {
        // P0 清除歷史
        this.messages = []
    }
}
```

### ModelRouter（模型路由）

```typescript
class ModelRouter {
    /**統一 40+ 模型供應商介面"""

    providers: Map<string, ModelProvider>    // P0 供應商集合
    defaultModel: string                     // P0 預設模型

    constructor(config: ModelConfig) {
        // P0 初始化供應商
        this.providers.set("openai", new OpenAIProvider())
        this.providers.set("anthropic", new AnthropicProvider())
        // ... 等等
    }

    async call(
        model: string,
        messages: ModelMessage[],
        options?: CallOptions
    ): Promise<string> {
        // P0 路由至相應供應商
        const provider = this.providers.get(model)
        return provider.call(messages, options)
    }

    async stream(
        model: string,
        messages: ModelMessage[]
    ): AsyncIterator<string> {
        // P1 流式調用
        const provider = this.providers.get(model)
        return provider.stream(messages)
    }
}
```

## 啟動流程

```
1. Mastra 實例初始化（應用啟動）
   ├─> new Mastra({
   │       models: { provider: "openai", apiKey: "..." },
   │       storage: new PostgresStorage(...),
   │       memory: { vectors: new PineconeVectorDB(...) }
   │   })
   ├─> 初始化依賴注入容器
   ├─> 連接模型供應商 API
   ├─> 初始化存儲後端
   └─> 準備就緒

2. 代理執行（Agent.execute）
   ├─> agent.execute("用戶輸入")
   ├─> AgentExecutor 啟動推理循環
   │   Loop: while not done:
   │   ├─> 讀取對話記憶：memory.recall(input)
   │   ├─> 構建消息列表：[system_msg, ...history, current_input]
   │   ├─> 調用模型：ModelRouter.call(agent.model, messages)
   │   ├─> LLM 響應？
   │   │   ├─> 包含工具調用→執行工具→反饋結果
   │   │   └─> 純文本→返回最終答案
   │   ├─> 存儲記憶：memory.remember(response)
   │   └─> 超出 max_iterations？終止
   └─> 返回 AgentResult

3. 工作流執行（Workflow.execute）
   ├─> workflow.execute(input)
   ├─> WorkflowRunner 編譯工作流圖
   ├─> 遍歷拓撲排序步驟
   │   Loop: for step in topological_order:
   │   ├─> 讀取前置步驟的輸出作為輸入
   │   ├─> 執行 step.handler(previous_output)
   │   ├─> 檢查條件分支（如有）
   │   │   ├─> condition_result = step.condition(state)
   │   │   └─> 選擇對應分支
   │   ├─> 並行步驟？並發執行 Promise.all()
   │   ├─> 檢查暫停點？中斷執行
   │   └─> 存儲步驟結果
   └─> 返回 WorkflowResult

4. 記憶查詢與存儲
   ├─> agent.memory.recall("相關上下文")
   │   ├─> 查詢對話歷史（BM25 / 向量相似度）
   │   ├─> 查詢語義向量（Pinecone / Weaviate）
   │   └─> 合併結果（混合搜索）
   ├─> agent.memory.remember(new_content)
   │   ├─> 添加至對話歷史
   │   ├─> 生成嵌入向量
   │   └─> 存儲至向量數據庫
   └─> 返回相關上下文

5. 工作流暫停與恢復（人類介入）
   ├─> workflow.suspend("需要人工審核")
   │   ├─> 保存當前狀態至存儲
   │   ├─> 返回暫停令牌
   │   └─> 等待人類輸入
   ├─> 人類檢查狀態與輸出
   ├─> workflow.resume(approval_result)
   │   ├─> 讀取暫存狀態
   │   ├─> 應用人類輸入
   │   └─> 從暫停點繼續執行
   └─> 返回最終結果

6. 伺服器集成（P1）
   ├─> mastra.server("express") → Express 伺服器
   │   ├─> POST /agents/:agent_id/run → agent.execute()
   │   ├─> GET /agents/:agent_id/memory → 檢索記憶
   │   ├─> POST /workflows/:workflow_id/run → workflow.execute()
   │   └─> POST /workflows/:workflow_id/suspend → 暫停
   ├─> mastra.server("nextjs") → Next.js Route Handler
   │   ├─> API 路由自動生成
   │   └─> 支持 ISR 與邊界計算
   └─> 完全類型安全（TypeScript）
```

## 資料流 ASCII 圖

### 代理執行流

```
User Input
    ↓
[Agent Executor]
    ├─> Memory.recall() ← [對話歷史 + 向量DB]
    ├─> 構建消息列表
    ├─> ModelRouter.call()
    │   ↓
    │   [LLM API]
    │   ├─> 工具調用？
    │   │   ├─> 是 → [Tool.execute()]
    │   │   │   ├─> 返回結果
    │   │   │   └─> 重新調用 LLM
    │   │   └─> 否 → 返回最終答案
    └─> Memory.remember() → [存儲]
        ↓
    Agent Output
```

### 工作流執行流

```
Input
  ↓
[Workflow Compiler] → Topological Sort
  ↓
Step 1 (Sequential)
  ├─> Check Condition?
  │   ├─> Branch A ⟶ Step 2A → Step 3
  │   └─> Branch B ⟶ Step 2B → Step 3
  ├─> Parallel Steps (2A, 2B 並行)
  ├─> Suspend Point?
  │   └─> 是 → Save State → Wait for Resume
  └─> Output
```

### 記憶混合查詢流

```
Query: "相關信息"
  ├─> [Conversation Search]
  │   └─> BM25/向量相似度 → [對話消息]
  └─> [Semantic Search]
      └─> Vector Embedding → Pinecone Query → [嵌入結果]
           ↓
       [Merge & Rank]
           ↓
       Final Context
```

## 子系統清單

### P0（核心必需）

| 子系統 | 模塊 | 責任 |
|--------|------|------|
| **Mastra 主類** | `mastra/` | 依賴注入與配置中樞 |
| **代理系統** | `agent/` | 自主推理循環與工具執行 |
| **工作流引擎** | `workflows/` | 步驟協調與狀態管理 |
| **工具系統** | `tools/` | 原子能力與執行 |
| **對話記憶** | `memory/conversation.ts` | 消息歷史與檢索 |
| **模型路由** | `models/router.ts` | 統一供應商介面 |
| **存儲** | `storage/` | 可插拔持久化後端 |
| **類型系統** | `types/` | TypeScript 完整型定義 |

### P1（企業級功能）

| 子系統 | 模塊 | 責任 |
|--------|------|------|
| **工作記憶** | `memory/working.ts` | 短期狀態管理 |
| **語義回憶** | `memory/semantic.ts` | 向量検索與檢索 |
| **RAG 管道** | `rag/` | 知識庫檢索增強 |
| **MCP 支持** | `@mcp/server` | Model Context Protocol 伺服器 |
| **評估框架** | `evals/` | 代理與工作流評估 |
| **可觀測性** | `observability/` | 追蹤、指標、日誌 |
| **伺服器適配器** | `@server-adapters/` | Express/Next.js/Fastify 集成 |
| **部署器** | `@deployers/` | Render/Vercel 自動化部署 |
| **CLI** | `cli/` | 代碼生成與開發工具 |
| **暫停與恢復** | `workflows/suspend.ts` | 人類介入與狀態恢復 |
| **第三方集成** | `integrations/` | Slack/Discord/Email |

### P2（未來擴展）

| 功能 | 說明 |
|------|------|
| **多模態代理** | 支持圖像、音頻、視頻處理 |
| **分佈式工作流** | 跨機器執行步驟 |
| **實時協作** | WebSocket 實時狀態同步 |
| **自適應工具選擇** | 根據上下文動態選擇工具 |
| **離線執行** | 邊界計算與本地代理 |
| **成本優化** | 自動模型選擇以節省成本 |

