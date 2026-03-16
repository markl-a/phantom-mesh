# Cline 深度技術分析

> 分析日期：2026-03-12
> 專案位置：`LLM-Cluster-Project/references/cline/`
> 版本：CLI v2.7.0 / VSCode Extension

---

## 目錄

1. [專案結構](#1-專案結構)
2. [入口點與啟動流程](#2-入口點與啟動流程)
3. [核心架構](#3-核心架構)
4. [瀏覽器整合](#4-瀏覽器整合)
5. [MCP 支援](#5-mcp-支援)
6. [權限與核准系統](#6-權限與核准系統)
7. [上下文管理](#7-上下文管理)
8. [跨平台架構](#8-跨平台架構)
9. [值得採納的關鍵模式](#9-值得採納的關鍵模式)
10. [對 clawtex-core 的借鑒意義](#10-對-clawtex-core-的借鑒意義)

---

## 1. 專案結構

### 頂層目錄樹

```
cline/
├── src/                          # 核心共用程式碼 (TypeScript)
│   ├── extension.ts              # VSCode 擴充套件入口
│   ├── common.ts                 # 跨平台共用初始化
│   ├── core/                     # 核心邏輯層
│   │   ├── api/                  # LLM Provider 抽象
│   │   │   ├── index.ts          # ApiHandler 介面 + 工廠函式
│   │   │   ├── providers/        # 30+ 個 provider 實作
│   │   │   ├── transform/        # 格式轉換 (Anthropic/OpenAI/Gemini/Ollama)
│   │   │   └── retry.ts          # 帶裝飾器的重試邏輯
│   │   ├── task/                 # Agent 任務迴圈核心
│   │   │   ├── index.ts          # Task 類別 — 主迴圈
│   │   │   ├── ToolExecutor.ts   # 工具執行器
│   │   │   ├── TaskState.ts      # 任務狀態管理
│   │   │   ├── tools/            # 工具處理器系統
│   │   │   │   ├── handlers/     # 個別工具 Handler
│   │   │   │   ├── ToolExecutorCoordinator.ts  # 工具路由
│   │   │   │   ├── ToolValidator.ts            # 參數驗證
│   │   │   │   ├── autoApprove.ts              # 自動核准邏輯
│   │   │   │   └── subagent/     # 子代理系統
│   │   │   ├── focus-chain/      # 焦點鏈 (任務追蹤)
│   │   │   └── StreamResponseHandler.ts
│   │   ├── controller/           # Controller 層 — 介面橋接
│   │   │   ├── index.ts          # Controller 主類別
│   │   │   ├── grpc-handler.ts   # gRPC 路由處理
│   │   │   ├── account/          # 帳號認證
│   │   │   ├── mcp/              # MCP 伺服器管理
│   │   │   ├── models/           # 模型列表取得
│   │   │   ├── state/            # 狀態訂閱
│   │   │   └── ui/               # UI 事件
│   │   ├── webview/              # WebviewProvider 基底類別
│   │   ├── context/              # 上下文管理
│   │   │   ├── context-management/  # ContextManager (截斷/壓縮)
│   │   │   ├── context-tracking/    # 追蹤器 (檔案/模型/環境)
│   │   │   └── instructions/        # 使用者指令 (Rules/Skills)
│   │   ├── assistant-message/    # 助理訊息解析
│   │   │   ├── parse-assistant-message.ts  # XML 標籤解析器 V2
│   │   │   └── diff.ts                     # SEARCH/REPLACE diff 引擎
│   │   ├── prompts/              # System Prompt 模組化系統
│   │   │   ├── system-prompt/    # 組件 + 變體 + 模板
│   │   │   └── responses.ts      # 格式化回應
│   │   ├── hooks/                # Hook 系統 (TaskStart/TaskCancel 等)
│   │   ├── permissions/          # 命令權限控制
│   │   ├── ignore/               # .clineignore 支援
│   │   ├── storage/              # 持久化 (StateManager/disk)
│   │   └── workspace/            # 多根目錄工作區
│   ├── hosts/                    # 平台宿主抽象層
│   │   ├── host-provider.ts      # HostProvider 單例 — 平台注入核心
│   │   ├── host-provider-types.ts
│   │   ├── vscode/               # VSCode 宿主實作
│   │   │   ├── VscodeWebviewProvider.ts
│   │   │   ├── VscodeDiffViewProvider.ts
│   │   │   └── terminal/         # VSCode 終端管理
│   │   └── external/             # 外部宿主 (JetBrains/CLI gRPC)
│   │       ├── ExternalWebviewProvider.ts
│   │       ├── ExternalDiffviewProvider.ts
│   │       └── host-bridge-client-manager.ts
│   ├── integrations/             # 整合層
│   │   ├── editor/               # 編輯器 (DiffViewProvider 基底)
│   │   ├── terminal/             # 終端抽象 + Standalone 實作
│   │   ├── checkpoints/          # Git checkpoint 系統
│   │   └── claude-code/          # Claude Code 整合
│   ├── services/                 # 服務層
│   │   ├── browser/              # BrowserSession (Puppeteer)
│   │   ├── mcp/                  # McpHub (MCP 客戶端)
│   │   ├── telemetry/            # 遙測 (PostHog/OpenTelemetry)
│   │   ├── tree-sitter/          # AST 解析 (程式碼定義)
│   │   └── auth/                 # 認證服務
│   ├── shared/                   # 共用型別與工具
│   │   ├── api.ts                # Provider/Model 型別定義
│   │   ├── tools.ts              # ClineDefaultTool 列舉
│   │   ├── ExtensionMessage.ts   # 訊息型別
│   │   ├── proto/                # Protobuf 型別
│   │   ├── storage/              # 儲存型別
│   │   └── net.ts                # Proxy 感知的 fetch/axios
│   └── utils/                    # 通用工具
├── cli/                          # CLI 獨立套件
│   ├── src/
│   │   ├── index.ts              # CLI 入口 (Commander.js + React Ink)
│   │   ├── agent/                # ClineAgent (headless)
│   │   ├── acp/                  # ACP 模式 (Agent Communication Protocol)
│   │   ├── controllers/          # CLI 宿主橋接 (stub 實作)
│   │   └── components/           # TUI 元件 (React Ink)
│   └── package.json              # bin: cline -> dist/cli.mjs
├── webview-ui/                   # React WebView UI
│   └── src/                      # Vite + React + TailwindCSS
├── proto/                        # Protobuf 定義
├── evals/                        # 評估測試
└── testing-platform/             # 測試基礎設施
```

### 關鍵數據

| 指標 | 數值 |
|------|------|
| LLM Provider | 30+ (Anthropic, OpenAI, Gemini, Ollama, Groq, Bedrock, etc.) |
| 內建工具 | 25+ (ClineDefaultTool 列舉) |
| 平台支援 | VSCode Extension + CLI (React Ink) + JetBrains (gRPC 外部宿主) |
| 通訊協議 | gRPC over VSCode message passing / gRPC over stdio |
| 前端 | React WebView (VSCode/JetBrains) + React Ink TUI (CLI) |

---

## 2. 入口點與啟動流程

### 2.1 VSCode 擴充套件入口

**檔案**: `src/extension.ts`

啟動流程分為五個階段：

```
activate(context)
  1. setupHostProvider(context)          // 注入 VSCode 宿主實作
  2. cleanupLegacyVSCodeStorage(context) // 遷移舊資料
  3. exportVSCodeStorageToSharedFiles()  // 匯出到 ~/.cline/data/
  4. initialize(storageContext)          // common.ts — 跨平台初始化
  5. 註冊 VSCode 特有的命令和提供者
```

`setupHostProvider()` 是關鍵 — 它透過 `HostProvider.initialize()` 注入所有平台特定的工廠函式：

```typescript
// src/extension.ts:620-631
HostProvider.initialize(
    createWebview,           // () => new VscodeWebviewProvider(context)
    createDiffView,          // () => new VscodeDiffViewProvider()
    createCommentReview,     // () => getVscodeCommentReviewController()
    createTerminalManager,   // () => new VscodeTerminalManager()
    vscodeHostBridgeClient,  // gRPC bridge for VSCode
    () => {},                // logger
    getCallbackUrl,          // OAuth callback URL
    getBinaryLocation,       // ripgrep binary path
    context.extensionUri.fsPath,
    context.globalStorageUri.fsPath,
)
```

### 2.2 通用初始化 (common.ts)

**檔案**: `src/common.ts`

```typescript
export async function initialize(storageContext: StorageContext): Promise<WebviewProvider> {
    Logger.subscribe(...)              // 設定日誌輸出
    ClineEndpoint.initialize(...)       // 讀取設定檔
    StateManager.initialize(...)        // 初始化狀態管理器
    ErrorService.initialize()           // 錯誤服務
    PostHogClientProvider.getInstance() // 遙測客戶端

    const webview = HostProvider.get().createWebviewProvider()  // 建立 WebView
    // WebviewProvider 建構式中自動建立 Controller

    syncWorker().init(...)              // 背景同步
    ClineTempManager.startPeriodicCleanup() // 臨時檔案清理

    return webview
}
```

### 2.3 CLI 入口

**檔案**: `cli/src/index.ts`

CLI 使用 Commander.js 解析命令列參數，React Ink 渲染 TUI：

```typescript
// cli/src/index.ts
import { Command } from "commander"
import { render } from "ink"
import { App } from "./components/App"

// CLI 提供自己的 HostProvider 實作
import { createCliHostBridgeProvider } from "./controllers"
import { CliWebviewProvider } from "./controllers/CliWebviewProvider"
import { FileEditProvider } from "@/integrations/editor/FileEditProvider"
import { StandaloneTerminalManager } from "@/integrations/terminal/standalone/StandaloneTerminalManager"
```

CLI 透過 stub 實作取代 VSCode 特有的服務：
- `CliDiffServiceClient` — 檔案差異操作的無操作 stub
- `CliWebviewProvider` — 控制台輸出代替 WebView
- `StandaloneTerminalManager` — 直接 child_process 取代 VSCode 終端
- `FileEditProvider` — 背景檔案編輯 (不使用 VSCode diff 編輯器)

---

## 3. 核心架構

### 3.1 Agent 迴圈

Agent 迴圈是 Cline 的心臟。架構為三層：

```
Controller → Task → ToolExecutor
```

#### Task 類別 (`src/core/task/index.ts`)

Task 類別管理單一對話任務的完整生命週期。核心流程：

```
startTask(task, images, files)
  ├── 初始化 ClineIgnoreController
  ├── 清空對話歷史
  ├── 執行 TaskStart hook
  ├── 執行 UserPromptSubmit hook
  └── initiateTaskLoop(userContent)

initiateTaskLoop(userContent)     <-- 外層 while 迴圈
  while (!abort) {
    didEndLoop = recursivelyMakeClineRequests(userContent)
    if (didEndLoop) break
    userContent = formatResponse.noToolsUsed()  // 強制繼續
    consecutiveMistakeCount++
  }

recursivelyMakeClineRequests(userContent)  <-- API 呼叫 + 工具執行
  ├── 檢查 abort 旗標
  ├── 遞增 API 請求計數
  ├── 檢查連續錯誤次數上限
  ├── 管理 checkpoint (git commit)
  ├── 判斷是否需要壓縮上下文
  ├── 載入上下文 (mentions, slash commands, environment)
  ├── 建構 system prompt
  ├── 呼叫 LLM API (串流)
  ├── 解析助理回應 (parseAssistantMessageV2)
  ├── 對每個工具呼叫 → ToolExecutor.executeTool()
  ├── 收集工具結果
  └── 遞迴呼叫自身 (帶工具結果作為 userContent)
```

關鍵設計：
- **Mutex 保護** — 所有狀態修改透過 `withStateLock()` (p-mutex)
- **串流處理** — `StreamResponseHandler` + `StreamChunkCoordinator` 處理 SSE 串流
- **中止機制** — `taskState.abort` 旗標 + `AbortController` for hooks
- **錯誤限制** — `consecutiveMistakeCount` 到達閾值時暫停要求使用者介入

```typescript
// src/core/task/index.ts:150-170
export class Task {
    readonly taskId: string
    taskState: TaskState
    private stateMutex = new Mutex()

    private async withStateLock<T>(fn: () => T | Promise<T>): Promise<T> {
        return await this.stateMutex.withLock(fn)
    }

    // 核心依賴
    api: ApiHandler
    terminalManager: ITerminalManager
    browserSession: BrowserSession
    contextManager: ContextManager
    private diffViewProvider: DiffViewProvider
    private toolExecutor: ToolExecutor
    private commandPermissionController: CommandPermissionController
    // ...
}
```

### 3.2 Provider 抽象

**檔案**: `src/core/api/index.ts`

Cline 使用統一的 `ApiHandler` 介面抽象所有 LLM 提供者：

```typescript
// src/core/api/index.ts:52-57
export interface ApiHandler {
    createMessage(
        systemPrompt: string,
        messages: ClineStorageMessage[],
        tools?: ClineTool[],
        useResponseApi?: boolean
    ): ApiStream
    getModel(): ApiHandlerModel
    getApiStreamUsage?(): Promise<ApiStreamUsageChunk | undefined>
    abort?(): void
}

export interface ApiHandlerModel {
    id: string
    info: ModelInfo
}
```

工廠函式 `createHandlerForProvider()` 根據 provider ID 建立對應的 Handler：

```typescript
// src/core/api/index.ts:75-80
function createHandlerForProvider(
    apiProvider: string | undefined,
    options: Omit<ApiConfiguration, "apiProvider">,
    mode: Mode,
): ApiHandler {
    switch (apiProvider) {
        case "anthropic": return new AnthropicHandler({...})
        case "openrouter": return new OpenRouterHandler({...})
        case "ollama":     return new OllamaHandler({...})
        case "gemini":     return new GeminiHandler({...})
        // ... 30+ providers
    }
}
```

#### 已支援的 Provider (30+)

| 類別 | Provider |
|------|----------|
| 商業 API | Anthropic, OpenAI, OpenAI Native, OpenAI Codex, Gemini, Groq, Mistral, DeepSeek, xAI, Moonshot, Doubao, Minimax, Qwen, QwenCode |
| 雲端 | AWS Bedrock, Google Vertex, Huawei Cloud MaaS, SAP AI Core |
| 路由/聚合 | OpenRouter, LiteLLM, Requesty, AIhubmix, Vercel AI Gateway, AskSage, Hicap |
| 本地 | Ollama, LM Studio |
| 社群 | Together, Fireworks, Cerebras, SambaNova, NousResearch, Nebius, HuggingFace, Baseten |
| 特殊 | Claude Code, Cline (自營), VSCode LM, OCA, Dify, ZAI |

#### Provider 格式轉換

`src/core/api/transform/` 包含多種格式轉換器：

- `anthropic-format.ts` — Anthropic Messages API 格式
- `openai-format.ts` — OpenAI Chat Completions 格式
- `openai-response-format.ts` — OpenAI Responses API 格式
- `gemini-format.ts` — Google Gemini 格式
- `ollama-format.ts` — Ollama 本地格式
- `o1-format.ts` / `r1-format.ts` — 推理模型特殊格式
- `stream.ts` — 統一串流型別 `ApiStream`

#### 重試機制

```typescript
// src/core/api/retry.ts — 裝飾器模式
@withRetry()
async *createMessage(...): ApiStream {
    // 自動重試帶指數退避
}
```

### 3.3 工具系統

Cline 的工具系統經過精心設計，支援串流更新、自動核准、Hook 系統等特性。

#### 工具列舉 (`src/shared/tools.ts`)

```typescript
export enum ClineDefaultTool {
    ASK = "ask_followup_question",
    ATTEMPT = "attempt_completion",
    BASH = "execute_command",
    FILE_EDIT = "replace_in_file",
    FILE_READ = "read_file",
    FILE_NEW = "write_to_file",
    SEARCH = "search_files",
    LIST_FILES = "list_files",
    LIST_CODE_DEF = "list_code_definition_names",
    BROWSER = "browser_action",
    MCP_USE = "use_mcp_tool",
    MCP_ACCESS = "access_mcp_resource",
    MCP_DOCS = "load_mcp_documentation",
    NEW_TASK = "new_task",
    PLAN_MODE = "plan_mode_respond",
    ACT_MODE = "act_mode_respond",
    TODO = "focus_chain",
    WEB_FETCH = "web_fetch",
    WEB_SEARCH = "web_search",
    CONDENSE = "condense",
    SUMMARIZE_TASK = "summarize_task",
    REPORT_BUG = "report_bug",
    NEW_RULE = "new_rule",
    APPLY_PATCH = "apply_patch",
    GENERATE_EXPLANATION = "generate_explanation",
    USE_SKILL = "use_skill",
    USE_SUBAGENTS = "use_subagents",
}
```

#### Handler 架構 (`src/core/task/tools/`)

```
ToolExecutorCoordinator          # 路由器 — 將工具名映射到 Handler
  ├── IToolHandler               # 基本介面: execute() + getDescription()
  ├── IPartialBlockHandler       # 串流中的 partial block 處理
  └── IFullyManagedTool          # 完整管理 — 自行處理核准流程

具體 Handler (src/core/task/tools/handlers/):
  ├── WriteToFileToolHandler     # write_to_file + replace_in_file + new_rule
  ├── ReadFileToolHandler        # read_file
  ├── ExecuteCommandToolHandler  # execute_command (shell)
  ├── BrowserToolHandler         # browser_action
  ├── SearchFilesToolHandler     # search_files (ripgrep)
  ├── ListFilesToolHandler       # list_files (glob)
  ├── ListCodeDefinitionNamesToolHandler  # tree-sitter AST
  ├── UseMcpToolHandler          # MCP 工具呼叫
  ├── AccessMcpResourceHandler   # MCP 資源存取
  ├── WebFetchToolHandler        # HTTP GET
  ├── WebSearchToolHandler       # 網路搜尋
  ├── AttemptCompletionHandler   # 任務完成
  ├── AskFollowupQuestionHandler # 追問
  ├── SubagentToolHandler        # 子代理派遣
  ├── ApplyPatchHandler          # 統一 diff patch
  └── ...
```

**ToolValidator** (`src/core/task/tools/ToolValidator.ts`) 提供輕量驗證：

```typescript
export class ToolValidator {
    constructor(private readonly clineIgnoreController: ClineIgnoreController) {}

    assertRequiredParams(block: ToolUse, ...params: ToolParamName[]): ValidationResult
    checkClineIgnorePath(relPath: string): ValidationResult
}
```

#### 工具執行流程

```
LLM 回應串流
  ↓
parseAssistantMessageV2()  → AssistantMessageContent[]
  ↓
對每個 ToolUse:
  ├── partial=true → handlePartialBlock() // 串流 UI 更新
  └── partial=false → execute()
       ├── ToolValidator 驗證參數
       ├── AutoApprove 檢查是否自動核准
       ├── 若需核准 → ask("tool", ...) → 等待使用者回應
       ├── 執行工具邏輯
       ├── Hook 系統 (pre/post tool hooks)
       └── 返回 ToolResponse
```

### 3.4 Diff/Edit 系統

Cline 使用 SEARCH/REPLACE 區塊格式進行檔案編輯：

**檔案**: `src/core/assistant-message/diff.ts`

```
------- SEARCH
原始程式碼
=======
替換後程式碼
+++++++ REPLACE
```

解析器支援：
- 精確字串匹配
- `lineTrimmedFallbackMatch()` — 忽略縮排的模糊匹配
- 多段 SEARCH/REPLACE 區塊
- 行號追蹤 (`getLineNumberFromCharIndex()`)

**DiffViewProvider** (`src/integrations/editor/DiffViewProvider.ts`) 是抽象基底類別：

```typescript
export abstract class DiffViewProvider {
    editType?: "create" | "modify" | "delete"
    isEditing = false
    originalContent: string | undefined

    public async open(relPath: string, options?): Promise<void> {
        // 開啟檔案、偵測編碼、建立目錄、取得診斷資訊
    }

    protected abstract openDiffEditor(): Promise<void>     // 平台特定
    protected abstract scrollEditorToLine(line: number): Promise<void>

    // 串流更新
    public async update(content: string, isNewFile: boolean): Promise<void>

    // 儲存或還原
    public async saveChanges(): Promise<...>
    public async revertChanges(): Promise<void>
}
```

平台實作：
- **VSCode**: `VscodeDiffViewProvider` — 使用 VSCode Diff Editor
- **CLI/背景**: `FileEditProvider` — 直接檔案寫入，不開啟編輯器
- **JetBrains**: `ExternalDiffviewProvider` — gRPC 橋接到 JetBrains IDE

**WriteToFileToolHandler** (`src/core/task/tools/handlers/WriteToFileToolHandler.ts`) 同時處理 `write_to_file`、`replace_in_file` 和 `new_rule` 三種工具，利用 `constructNewFileContent()` 應用 SEARCH/REPLACE diff。

---

## 4. 瀏覽器整合

**檔案**: `src/services/browser/BrowserSession.ts`

Cline 使用 **Puppeteer-core** 控制瀏覽器：

```typescript
import { Browser, connect, launch, Page } from "puppeteer-core"
import * as chromeLauncher from "chrome-launcher"

export class BrowserSession {
    private browser?: Browser
    private page?: Page

    // Chrome 發現策略 (依優先順序)：
    // 1. 使用者設定的路徑 (browserSettings.chromeExecutablePath)
    // 2. 系統 Chrome (chrome-launcher)
    // 3. 內建 Chromium (puppeteer-core-resolver)
    async getDetectedChromePath(): Promise<{ path: string; isBundled: boolean }>
}
```

瀏覽器操作作為工具暴露 (`browser_action`)：

```typescript
// BrowserToolHandler 支援的操作：
type BrowserAction = "launch" | "click" | "type" | "scroll_down" | "scroll_up"
    | "screenshot" | "close" | "navigate" | "go_back" | "select_option"
```

特點：
- 預設使用 Chrome 除錯埠 9222
- 支援遠端瀏覽器連線
- WebP 截圖格式 (可設定)
- 連線超時與重試
- 使用者可自訂啟動參數
- 遙測追蹤瀏覽器會話時長

---

## 5. MCP 支援

**檔案**: `src/services/mcp/McpHub.ts`

McpHub 是 MCP 伺服器的中央管理器：

```typescript
import { Client } from "@modelcontextprotocol/sdk/client/index.js"
import { SSEClientTransport } from "@modelcontextprotocol/sdk/client/sse.js"
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js"
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js"

export class McpHub {
    connections: McpConnection[] = []

    // 支援三種傳輸方式：
    // 1. Stdio — 本地子程序
    // 2. SSE — Server-Sent Events
    // 3. StreamableHTTP — HTTP 串流

    // 功能：
    // - 列出工具 (ListToolsResultSchema)
    // - 呼叫工具 (CallToolResultSchema)
    // - 存取資源 (ReadResourceResultSchema)
    // - 列出提示 (ListPromptsResultSchema)
    // - OAuth 認證 (McpOAuthManager)
    // - 設定檔監控 (chokidar file watcher)
    // - 自動重連 (StreamableHttpReconnectHandler)
}
```

設定檔格式：
```json
{
  "mcpServers": {
    "server-name": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-xxx"],
      "env": { "API_KEY": "..." }
    }
  }
}
```

MCP 工具在 Agent 迴圈中通過兩個 Handler 暴露：
- `UseMcpToolHandler` — 呼叫 MCP 伺服器的工具
- `AccessMcpResourceHandler` — 讀取 MCP 資源
- `LoadMcpDocumentationHandler` — 載入 MCP 文件

特色功能：
- **OAuth 支援** — `McpOAuthManager` 處理 OAuth 2.0 流程
- **自動核准** — 可設定個別 MCP 工具的自動核准
- **Marketplace** — MCP 伺服器市集整合
- **設定變更監控** — 使用 chokidar 監控設定檔變更，自動重連

---

## 6. 權限與核准系統

Cline 的權限系統是其最精緻的設計之一，具有多層次的控制：

### 6.1 AutoApprove 類別

**檔案**: `src/core/task/tools/autoApprove.ts`

```typescript
export class AutoApprove {
    private stateManager: StateManager

    shouldAutoApproveTool(toolName: ClineDefaultTool): boolean | [boolean, boolean] {
        // 三層核准邏輯：
        // 1. yoloModeToggled — 全自動模式
        // 2. autoApproveAllToggled — 全部自動核准
        // 3. 個別工具設定 — autoApprovalSettings.actions.*

        // 返回值：
        // boolean — 簡單的是/否
        // [boolean, boolean] — [本地自動核准, 外部自動核准]
    }

    async shouldAutoApproveToolWithPath(
        blockname: ClineDefaultTool,
        path: string | undefined
    ): Promise<boolean> {
        // 結合路徑檢查：
        // - 工作區內的檔案 → 使用 autoApproveLocal 設定
        // - 工作區外的檔案 → 使用 autoApproveExternal 設定
        // - 多根目錄場景 → 檢查所有工作區
    }
}
```

### 6.2 核准設定粒度

```typescript
autoApprovalSettings: {
    actions: {
        readFiles: boolean,           // 讀取檔案
        readFilesExternally: boolean, // 讀取工作區外的檔案
        editFiles: boolean,           // 編輯檔案
        editFilesExternally: boolean, // 編輯工作區外的檔案
        executeSafeCommands: boolean, // 安全指令
        executeAllCommands: boolean,  // 所有指令
        useBrowser: boolean,          // 瀏覽器
        useMcp: boolean,              // MCP 工具
    },
    enableNotifications: boolean,     // 系統通知
}
```

### 6.3 Ask/Say 機制

工具執行時，透過 Task 的 `ask()` 方法暫停執行，等待使用者回應：

```typescript
// Task 內部
async ask(type: ClineAsk, text?: string, partial?: boolean): Promise<{
    response: ClineAskResponse  // "yesButtonClicked" | "noButtonClicked" | "messageResponse"
    text?: string
    images?: string[]
    files?: string[]
}>
```

Ask 類型包括：
- `"tool"` — 工具使用核准
- `"browser_action_launch"` — 瀏覽器啟動
- `"command"` — 終端指令核准
- `"completion_result"` — 任務完成確認
- `"followup"` — 追問回覆
- `"resume_task"` / `"resume_completed_task"` — 恢復任務
- `"mistake_limit_reached"` — 錯誤限制到達

### 6.4 .clineignore

**檔案**: `src/core/ignore/ClineIgnoreController.ts`

使用 `.clineignore` 檔案 (類似 `.gitignore` 語法) 禁止存取特定路徑：

```typescript
export class ClineIgnoreController {
    validateAccess(relPath: string): boolean
    // 被 ToolValidator.checkClineIgnorePath() 使用
}
```

### 6.5 命令權限

**檔案**: `src/core/permissions/CommandPermissionController.ts`

對 shell 指令進行額外的安全檢查。

### 6.6 YOLO 模式

當 `yoloModeToggled` 啟用時，所有工具自動核准。但若連續錯誤到達上限，會直接失敗而非暫停：

```typescript
if (this.stateManager.getGlobalSettingsKey("yoloModeToggled")) {
    const errorMessage = `[YOLO MODE] Task failed: Too many consecutive mistakes...`
    await this.say("error", errorMessage)
    return true // 結束任務迴圈
}
```

---

## 7. 上下文管理

### 7.1 ContextManager

**檔案**: `src/core/context/context-management/ContextManager.ts`

ContextManager 負責管理對話歷史的大小，防止超出模型的 context window：

```typescript
export class ContextManager {
    // 追蹤每個訊息的上下文變更歷史
    private contextHistoryUpdates: Map<number, [number, Map<number, ContextUpdate[]>]>

    // 判斷是否需要壓縮
    shouldCompactContextWindow(clineMessages, api, previousApiReqIndex): boolean

    // 嘗試最佳化檔案讀取以節省 token
    async attemptFileReadOptimization(...): Promise<boolean>

    // 計算下一個截斷範圍
    getNextTruncationRange(history, deletedRange, strategy): [number, number]

    // 標準截斷通知
    triggerApplyStandardContextTruncationNoticeChange(...)
}
```

### 7.2 上下文壓縮策略

Cline 使用多種策略管理上下文窗口：

1. **標準截斷** — 移除四分之一的早期對話 (`"quarter"` 策略)
2. **自動濃縮** (`useAutoCondense`) — 使用 `summarize_task` 工具產生摘要
3. **檔案讀取最佳化** — 重寫舊的檔案讀取回應以節省 token
4. **PreCompact Hook** — 壓縮前執行使用者自訂 hook

```typescript
// 上下文管理流程 (recursivelyMakeClineRequests 內):
if (useAutoCondense && isNextGenModelFamily(model.id)) {
    shouldCompact = this.contextManager.shouldCompactContextWindow(...)
    if (shouldCompact) {
        shouldCompact = await this.contextManager.attemptFileReadOptimization(...)
    }
}

if (shouldCompact) {
    userContent.push({ type: "text", text: summarizeTask(...) })
}
```

### 7.3 上下文追蹤器

三種追蹤器記錄任務執行過程中的元資料：

- **FileContextTracker** — 記錄已讀取和編輯的檔案
- **ModelContextTracker** — 記錄使用的模型和 provider
- **EnvironmentContextTracker** — 記錄執行環境資訊

### 7.4 Rules 系統

```
.clinerules/          # 專案級規則目錄
~/.cline/rules/       # 全域規則目錄
.cursorrules          # Cursor 相容規則
.windsurfrules        # Windsurf 相容規則
agents.md             # Agent 規則
```

`RuleContextBuilder` 負責彙整所有規則來源到 system prompt。

---

## 8. 跨平台架構

Cline 的跨平台設計是其最值得學習的架構模式。

### 8.1 HostProvider — 依賴注入核心

**檔案**: `src/hosts/host-provider.ts`

```typescript
export class HostProvider {
    private static instance: HostProvider | null = null

    createWebviewProvider: WebviewProviderCreator
    createDiffViewProvider: DiffViewProviderCreator
    createCommentReviewController: CommentReviewControllerCreator
    createTerminalManager: TerminalManagerCreator
    hostBridge: HostBridgeClientProvider
    getCallbackUrl: (path: string, preferredPort?: number) => Promise<string>
    getBinaryLocation: (name: string) => Promise<string>
    extensionFsPath: string
    globalStorageFsPath: string

    // 便捷存取器
    static get workspace() { return HostProvider.get().hostBridge.workspaceClient }
    static get env() { return HostProvider.get().hostBridge.envClient }
    static get window() { return HostProvider.get().hostBridge.windowClient }
    static get diff() { return HostProvider.get().hostBridge.diffClient }
}
```

### 8.2 HostBridge 介面

**檔案**: `src/hosts/host-provider-types.ts`

```typescript
export interface HostBridgeClientProvider {
    workspaceClient: WorkspaceServiceClientInterface  // 工作區操作
    envClient: EnvServiceClientInterface              // 環境操作
    windowClient: WindowServiceClientInterface        // 視窗操作
    diffClient: DiffServiceClientInterface            // Diff 操作
}
```

這些介面由 Protobuf 定義，不同平台提供不同實作：

### 8.3 平台實作對比

| 能力 | VSCode | CLI | JetBrains |
|------|--------|-----|-----------|
| WebView | `VscodeWebviewProvider` | `CliWebviewProvider` | `ExternalWebviewProvider` |
| Diff 編輯器 | `VscodeDiffViewProvider` | `FileEditProvider` | `ExternalDiffviewProvider` |
| 終端 | `VscodeTerminalManager` | `StandaloneTerminalManager` | `AcpTerminalManager` |
| Host Bridge | VSCode API 直接呼叫 | Stub 實作 (CLI 無操作) | gRPC 客戶端 |
| UI | React WebView | React Ink TUI | React WebView (gRPC 串流) |
| 資料儲存 | `~/.cline/data/` | `~/.cline/data/` | `~/.cline/data/` |

### 8.4 資料統一

所有平台共用 `~/.cline/data/` 目錄：

```typescript
// src/extension.ts:77-79
// 單次匯出 VSCode 的原生儲存到共享的檔案後端儲存。
// 之後所有平台 (VSCode, CLI, JetBrains) 都從 ~/.cline/data/ 讀取。
await exportVSCodeStorageToSharedFiles(context, storageContext)
```

### 8.5 gRPC/Protobuf 通訊

`proto/` 目錄包含所有 Protobuf 定義，用於：
- WebView ↔ Extension 通訊
- JetBrains Plugin ↔ Cline Core 通訊
- 型別安全的序列化/反序列化

---

## 9. 值得採納的關鍵模式

### 9.1 HostProvider 依賴注入模式

**模式**: 單例 + 工廠函式注入

```typescript
// 初始化時注入平台特定實作
HostProvider.initialize(
    createWebview,
    createDiffView,
    createTerminalManager,
    hostBridgeClient,
    // ...
)

// 使用時透過靜態方法存取
HostProvider.workspace.getWorkspacePaths({})
HostProvider.window.showMessage({...})
HostProvider.get().createTerminalManager()
```

**優勢**: 核心邏輯完全不依賴任何平台 API，極容易擴展到新平台。

### 9.2 多層次自動核准系統

```
YOLO Mode (全自動)
  ↓ 否
Auto Approve All (全部自動)
  ↓ 否
個別工具設定
  ├── 本地 vs 外部 路徑區分
  ├── 安全指令 vs 所有指令
  └── MCP 工具個別設定
```

**優勢**: 給使用者精確控制，同時提供便捷的全自動選項。

### 9.3 工具 Handler 分離模式

```typescript
interface IToolHandler {
    readonly name: ClineDefaultTool
    execute(config: TaskConfig, block: ToolUse): Promise<ToolResponse>
    getDescription(block: ToolUse): string
}

interface IPartialBlockHandler {
    handlePartialBlock(block: ToolUse, uiHelpers: StronglyTypedUIHelpers): Promise<void>
}

interface IFullyManagedTool extends IToolHandler, IPartialBlockHandler {
    // 完全自管理工具 — 處理自己的核准流程
}
```

**優勢**: 每個工具獨立管理自己的邏輯，串流 UI 更新和完整執行分離。

### 9.4 串流解析 (XML 標籤解析器 V2)

```typescript
// parseAssistantMessageV2 — 高效的單趟掃描
// 不使用逐字元累加器，而是用索引追蹤：
let currentTextContentStart = 0
let currentToolUseStart = 0
let currentParamValueStart = 0

// 預計算標籤 Map 實現快速查找
const toolUseOpenTags = new Map<string, string>()
for (const name of getToolUseNames()) {
    toolUseOpenTags.set(`<${name}>`, name)
}
```

**優勢**: 支援串流中的 partial 解析，可以在工具參數還在串流時就開始顯示 UI。

### 9.5 Checkpoint (Git 快照) 系統

每次 API 請求前自動建立 git commit，使用者可以隨時回滾到任一中間狀態：

```
src/integrations/checkpoints/
  ├── CheckpointTracker.ts        # 追蹤器
  ├── CheckpointGitOperations.ts  # Git 操作
  ├── MultiRootCheckpointManager.ts # 多根目錄
  └── factory.ts                  # 工廠建構
```

### 9.6 Hook 系統

支援使用者在關鍵時刻執行自訂腳本：

- `TaskStart` — 任務開始時
- `TaskCancel` — 任務取消時
- `UserPromptSubmit` — 使用者提交提示時
- `PreCompact` — 上下文壓縮前
- Per-tool hooks — 工具執行前後

### 9.7 Focus Chain (焦點鏈)

`src/core/task/focus-chain/` 實現任務進度追蹤：

- 維護待辦事項列表
- 追蹤已完成和未完成的項目
- 在上下文中提供結構化的任務狀態

---

## 10. 對 clawtex-core 的借鑒意義

### 10.1 直接可採用的設計

#### Provider 抽象統一化

Cline 的 `ApiHandler` 介面與 clawtex-core 的 `Provider` trait 相似，但 Cline 做得更好的是：
- **格式轉換層** (`transform/`) — 每個模型家族有獨立的格式轉換器
- **Responses API vs Messages API** — 明確區分新舊 API 格式
- **原生工具呼叫** — `enableNativeToolCalls` 可根據模型能力動態切換 XML/原生格式

**建議**: clawtex-core 應參考 `transform/` 層的設計，將格式轉換邏輯從 provider 中抽離。

#### 多層次核准系統

clawtex-core 的 `approval.rs` 是 Telegram 基礎的二元核准。可以借鑒 Cline 的：
- **AutoApprove 類別** — 路徑感知的自動核准
- **YOLO 模式** — 連續錯誤後自動失敗
- **個別工具粒度** — 不同工具不同權限
- **本地 vs 外部** — 工作區內外的差異化權限

**建議**: 在 `agents.toml` 中加入工具級別的核准設定。

#### Agent 迴圈的中止與恢復

Cline 的中止機制非常完善：
- `taskState.abort` 旗標在每個檢查點檢查
- `AbortController` 取消正在執行的 hook
- 5 階段中止流程 (檢查 → 設旗 → 取消 hook → 執行 TaskCancel → 清理)
- 中止後可恢復 (`resume_task`)

**建議**: clawtex-core 的 E-Stop 可以參考這個分階段中止設計。

### 10.2 架構模式的差異

| 面向 | Cline | clawtex-core |
|------|-------|-------------|
| 語言 | TypeScript | Rust |
| Agent 迴圈 | `Task.initiateTaskLoop()` while 迴圈 | `agent_runtime.rs` run_streaming() |
| Provider | 30+ Handler classes | 12 Provider trait impls |
| 工具 | XML 標籤解析 (parseAssistantMessageV2) | JSON function calling |
| 核准 | 多層次 AutoApprove | 二元 Telegram 核准 |
| 平台 | VSCode + CLI + JetBrains | Telegram Bot + HTTP API |
| 上下文 | ContextManager (截斷/濃縮) | ContextOptimizer (token 估算/修剪) |
| MCP | 完整 SDK (stdio/SSE/HTTP) | JSON-RPC 2.0 over stdio |

### 10.3 clawtex-core 可以學習的具體功能

1. **Checkpoint 系統** — git commit 快照用於任務回滾，clawtex 的 Hands 引擎可以在每個 phase 自動建立 checkpoint

2. **Focus Chain** — 結構化的待辦追蹤，比目前依賴 LLM 記憶更可靠

3. **SubAgent 系統** — Cline 的 `use_subagents` 工具和 `SubagentRunner` 實現了子代理派遣，與 clawtex 的 `delegate` 和 `delegate_to_provider` 工具類似但更結構化

4. **串流 UI 更新** — `handlePartialBlock()` 允許在工具參數還在串流時更新 UI，clawtex 的 Telegram 介面可以用 `edit_message()` 實現類似效果

5. **Hook 系統** — TaskStart/TaskCancel/PreCompact hooks 允許使用者在關鍵時刻注入自訂邏輯，clawtex 可以在 Hands 引擎中加入 phase hooks

6. **連續錯誤處理** — `consecutiveMistakeCount` + `maxConsecutiveMistakes` 防止 Agent 陷入迴圈，比 clawtex 的 `loop_detection.rs` 更簡單但同樣有效

7. **Proxy 感知的網路層** — `shared/net.ts` 統一處理 proxy/fetch/axios，clawtex 在企業環境部署時也需要此能力

### 10.4 clawtex-core 已經做得比 Cline 好的地方

1. **叢集系統** — Cline 是單機執行，clawtex 已有 `ClusterHub` + `ClusterWorker` 分散式架構
2. **收入追蹤** — `revenue_tracker.rs` + `cost_tracker.rs` 是 clawtex 獨有的
3. **Hands 工作流引擎** — TOML 驅動的多階段工作流比 Cline 的單一 Agent 迴圈更靈活
4. **加密秘密** — ChaCha20-Poly1305 加密比 Cline 的明文 API key 更安全
5. **Smart Routing** — 按複雜度自動分流到不同 provider，Cline 需要使用者手動切換

---

## 附錄：關鍵檔案路徑索引

| 功能 | 檔案路徑 |
|------|---------|
| VSCode 入口 | `src/extension.ts` |
| 跨平台初始化 | `src/common.ts` |
| HostProvider | `src/hosts/host-provider.ts` |
| HostBridge 型別 | `src/hosts/host-provider-types.ts` |
| Controller | `src/core/controller/index.ts` |
| Task (Agent 迴圈) | `src/core/task/index.ts` |
| ToolExecutor | `src/core/task/ToolExecutor.ts` |
| ToolExecutorCoordinator | `src/core/task/tools/ToolExecutorCoordinator.ts` |
| AutoApprove | `src/core/task/tools/autoApprove.ts` |
| API Handler 介面 | `src/core/api/index.ts` |
| Anthropic Provider | `src/core/api/providers/anthropic.ts` |
| 訊息解析器 V2 | `src/core/assistant-message/parse-assistant-message.ts` |
| Diff 引擎 | `src/core/assistant-message/diff.ts` |
| DiffViewProvider | `src/integrations/editor/DiffViewProvider.ts` |
| BrowserSession | `src/services/browser/BrowserSession.ts` |
| McpHub | `src/services/mcp/McpHub.ts` |
| ContextManager | `src/core/context/context-management/ContextManager.ts` |
| 工具列舉 | `src/shared/tools.ts` |
| CLI 入口 | `cli/src/index.ts` |
| CLI 宿主橋接 | `cli/src/controllers/index.ts` |
| CLI WebView | `cli/src/controllers/CliWebviewProvider.ts` |
| JetBrains WebView | `src/hosts/external/ExternalWebviewProvider.ts` |
| WebviewProvider 基底 | `src/core/webview/WebviewProvider.ts` |
| 寫入檔案 Handler | `src/core/task/tools/handlers/WriteToFileToolHandler.ts` |
| 瀏覽器 Handler | `src/core/task/tools/handlers/BrowserToolHandler.ts` |
| ToolValidator | `src/core/task/tools/ToolValidator.ts` |
| 代理 Fetch | `src/shared/net.ts` |

---

> 本分析基於 Cline 專案程式碼的靜態閱讀，所有檔案路徑相對於 `LLM-Cluster-Project/references/cline/`。
