# Cline 架構文檔

## 專案概覽

Cline 是一款 **VS Code 原生擴展的 AI 編碼助手**，集成在 VS Code 側邊欄中，以人機互動的方式執行複雜開發任務。不同於完全自主的 Agent，Cline 在每次檔案修改、終端命令前要求用戶審批，確保安全可控。

**核心定位**：
- IDE 集成的 AI 編碼助手（VS Code 專有）
- 人機互動（每步需審批）
- 支援文件編輯、終端命令、瀏覽器自動化
- MCP（Model Context Protocol）工具擴展
- 多模型支援（Claude、GPT、Gemini 等）

**技術棧**：
- 主要語言：TypeScript（extension + webview）
- UI 框架：VS Code Webview + React
- 通信協議：gRPC + Protocol Buffers
- 工具接口：MCP（stdio + HTTP）
- 資料層：SQLite（for history）

---

## 目錄結構

```
cline/
├── src/                           # 主擴展代碼
│   ├── extension.ts               # VS Code 擴展入點
│   ├── common.ts                  # 跨平台初始化
│   ├── core/
│   │   ├── controller/            # 業務邏輯層（P0）
│   │   │   ├── task/              # 任務執行引擎
│   │   │   │   ├── index.ts       # Task Runner（編排）
│   │   │   │   ├── tools/         # 工具 handlers
│   │   │   │   └── assistant-message/  # LLM 回應解析
│   │   │   ├── state/             # 狀態管理
│   │   │   └── commands/          # VS Code 命令
│   │   ├── prompts/               # System Prompt 引擎（P0）
│   │   │   ├── system-prompt/     # Prompt 生成
│   │   │   ├── variants/          # 模型族特化（generic、next-gen、xs）
│   │   │   ├── templates/         # Prompt 樣板
│   │   │   └── commands.ts        # Slash 命令
│   │   ├── api/                   # LLM API 適配層
│   │   │   ├── providers/         # 各提供商實現
│   │   │   └── index.ts           # 提供商工廠
│   │   ├── storage/               # 狀態持久化
│   │   ├── hooks/                 # 自訂業務邏輯
│   │   └── mcp/                   # MCP 客戶端
│   ├── shared/
│   │   ├── api.ts                 # API 類型定義
│   │   ├── proto/                 # Protocol Buffers 定義
│   │   ├── tools.ts               # 工具列表與類型
│   │   ├── services/              # Logger、Auth 等
│   │   └── storage/               # 跨平台存儲適配
│   ├── hosts/                     # 平台特化層
│   │   ├── vscode/                # VS Code 特定實現
│   │   ├── cli/                   # CLI 模式（開發中）
│   │   ├── jetbrains/             # JetBrains IDE（計畫中）
│   │   └── host-provider.ts       # 平台抽象
│   ├── services/
│   │   ├── auth/                  # 認證服務
│   │   ├── telemetry/             # 遙測
│   │   ├── uri/                   # URI 處理
│   │   └── test/
│   └── utils/                     # 工具函數
├── webview-ui/                    # React 前端
│   ├── src/
│   │   ├── components/
│   │   │   ├── chat/              # 聊天 UI
│   │   │   ├── settings/          # 設置面板
│   │   │   ├── task/              # 任務進度顯示
│   │   │   └── browser/           # 瀏覽器視圖
│   │   ├── context/               # React Context
│   │   ├── services/              # API 通信
│   │   └── styles/                # Tailwind CSS
│   └── build/                     # 編譯輸出
├── cli/                           # CLI 子項目
│   ├── src/
│   │   ├── main.ts                # CLI 入口
│   │   └── components/            # Ink UI 組件
│   └── dist/                      # 編譯產物
├── proto/                         # Protocol Buffers 定義（P0）
│   ├── cline/
│   │   ├── task.proto             # 任務消息
│   │   ├── models.proto           # 模型配置
│   │   ├── state.proto            # 全局狀態
│   │   └── ui.proto               # UI 消息
│   └── cline/                     # 共享 proto
├── src/generated/                 # 生成的 gRPC 代碼
├── tests/                         # 單元測試
├── evals/                         # 評估與測試
├── docs/                          # 文檔
└── package.json                   # 根工作區配置
```

---

## 核心 Trait/Struct（TypeScript）

### 1. Task Runner 類（任務編排）

```typescript
// 核心編排引擎
class TaskRunner {
  async executeTask(prompt: string): Promise<void>

  // 內部流程
  - buildSystemPrompt()    // 根據模型族選擇 prompt
  - callLLM()              // 流式調用 LLM API
  - parseToolUse()         // 提取工具調用
  - executeTools()         // 執行文件/命令/瀏覽器操作
  - collectToolResults()   // 彙總結果
  - loopUntilComplete()    // 循環直到任務完成或中止
}
```

### 2. ExtensionMessage 類（消息協議）

```typescript
// gRPC 消息體（通過 Protocol Buffers 定義）
interface ClineMessage {
  type: ClineSay  // 消息類型枚舉

  // 各類型對應的負載
  text?: string
  toolUseId?: string
  toolName?: string
  toolInput?: object
  toolResult?: string

  // UI 狀態
  isError?: boolean
  isActionable?: boolean
}

// 枚舉示例
enum ClineSay {
  TEXT = 0
  USER_FEEDBACK = 1
  TOOL_USE = 2
  TOOL_RESULT = 3
  ERROR = 4
  COMPLETION = 5
  // ... 30+ 類型
}
```

### 3. GlobalState 界面（狀態管理）

```typescript
interface GlobalState {
  // API 配置
  selectedProvider: "anthropic" | "openai" | "gemini" | ...
  selectedModelId: string
  apiConfiguration: ApiConfiguration  // provider-specific 配置

  // 用戶設置
  customInstructions: string
  allowedCommands: string[]
  budget: number

  // 會話數據
  taskHistory: TaskHistoryItem[]
  currentSession: Session

  // 功能開關
  enableBrowser: boolean
  enableMCP: boolean
  mcpServers: MCPServer[]
}

// 存儲位置
// - VS Code: context.globalState（同步到雲）
// - CLI: ~/.cline/data/state.json
// - JetBrains: IDE 配置系統
```

### 4. API Handler 工廠

```typescript
interface LLMHandler {
  sendMessage(messages: Message[]): AsyncIterator<string>
  supportsToolUse(): boolean
  getModel(): ModelInfo
}

// 實現類族
class AnthropicHandler implements LLMHandler { ... }
class OpenAIHandler implements LLMHandler { ... }
class GeminiHandler implements LLMHandler { ... }
class LocalOllamaHandler implements LLMHandler { ... }
```

### 5. MCP 客戶端

```typescript
interface MCPClient {
  start(serverConfig: MCPServerConfig): Promise<void>
  callTool(toolName: string, args: object): Promise<object>
  close(): Promise<void>
}

// Transport: stdio（默認）或 HTTP
// 協議：JSON-RPC 2.0
```

---

## 啟動流程

### 1. VS Code 擴展激活

```
activate(context: ExtensionContext)
  → setupHostProvider()
    - 初始化 VS Code 特定的 host bridge
  → cleanupLegacyVSCodeStorage()
    - 遷移舊數據格式
  → createStorageContext()
    - 初始化跨平台存儲層（file + globalState）
  → StateManager.initialize()
    - 從磁盤加載狀態到內存緩存
  → registerCommands()
    - 註冊所有 VS Code 命令
  → createWebviewProvider()
    - 初始化 Webview（React UI）
  → MCP Registry.discover()
    - 掃描並初始化 MCP 服務器
  → registerEventListeners()
    - 監聽文件變化、終端輸出等
```

### 2. Webview 初始化

```
WebviewProvider.resolveWebviewPanel()
  → React App 挂載（webview-ui/src/App.tsx）
  → createServiceClient()
    - 初始化 gRPC 客戶端（通過 vscode.postMessage）
  → loadExtensionState()
    - 從 StateManager 獲取全局狀態
  → initializeChat()
    - 加載上次會話或新建空對話
  → setupMessageListener()
    - 監聽後端發送的 ClineMessage
```

### 3. 任務執行流程

```
用戶輸入提示詞
  → Controller.submitTask(prompt)
    ↓
TaskRunner.executeTask()
  → buildSystemPrompt()
    - 根據 selectedProvider + selectedModelId 選擇 prompt variant
    - 載入組件（rules、capabilities、editing_files、tools）
    - 注入用戶 customInstructions
    ↓
  → callLLM()
    - 通過 handler.sendMessage() 流式調用
    - 接收 streamed chunks
    ↓
  → parseToolUse()
    - 提取 <function_calls> 塊
    - 解析工具名稱和參數
    ↓
  → executeTools()（每個工具都需用戶審批）
    - file_read → 讀取文件
    - file_write → 創建/覆蓋文件
    - file_edit → 替換代碼片段
    - bash → 執行終端命令
    - browser → 打開瀏覽器、點擊、屏幕截圖
    ↓
  → collectResults()
    - 彙總工具結果
    ↓
  → loopUntilComplete()
    - 繼續循環直到：
      1. LLM 回覆完成（stop_reason: end_turn）
      2. 用戶中止任務
      3. 上下文窗口用盡
```

---

## 資料流 ASCII 圖

```
┌──────────────────────────────────────────────────┐
│            VS Code 主編輯器窗口                   │
│                                                  │
│  [Cline 側邊欄]  [文件編輯器]  [終端]            │
│       ↓                                          │
└──────────┬───────────────────────────────────────┘
           │
    ┌──────↓──────────────────────────────────────┐
    │      Webview (React)                         │
    │  ┌────────────────────────────────────────┐ │
    │  │ ChatInput  ← 用戶提示                   │ │
    │  │ ChatHistory ← 消息流                   │ │
    │  │ ToolApproval ← 審批對話框              │ │
    │  └────┬───────────────────────────────────┘ │
    │       │                                      │
    │  gRPC Client (nice-grpc)                    │
    └──────┬──────────────────────────────────────┘
           │
    ┌──────↓──────────────────────────────────────┐
    │        Extension Controller                  │
    │   ┌──────────────────────────────────────┐  │
    │   │  StateManager  (globalState cache)   │  │
    │   │  - selectedProvider                  │  │
    │   │  - apiConfiguration                  │  │
    │   │  - taskHistory                       │  │
    │   └──────┬──────────────────────────────┘  │
    │          ↓                                  │
    │   ┌─────────────────────────────────────┐  │
    │   │   Task Runner                       │  │
    │   │  ┌──────────────────────────────┐  │  │
    │   │  │ buildSystemPrompt()          │  │  │
    │   │  │ - Load variant prompt        │  │  │
    │   │  │ - Inject custom rules        │  │  │
    │   │  │ - Format tools list          │  │  │
    │   │  └──────────┬───────────────────┘  │  │
    │   │             ↓                       │  │
    │   │  ┌──────────────────────────────┐  │  │
    │   │  │ Handler (API adapter)        │  │  │
    │   │  │ - AnthropicHandler           │  │  │
    │   │  │ - OpenAIHandler              │  │  │
    │   │  │ - GeminiHandler              │  │  │
    │   │  └──────────┬───────────────────┘  │  │
    │   └─────────────┼────────────────────────┘  │
    └──────────────────┼───────────────────────────┘
                       │
    ┌──────────────────↓─────────────────────────┐
    │        外部 LLM API                        │
    │                                           │
    │  Claude API  │  OpenAI API  │ Gemini API │
    │                                           │
    │  返回 Tool Use 塊 + 文本回覆               │
    └──────────────────┬───────────────────────┘
                       │
    ┌──────────────────↓─────────────────────────┐
    │    Tool Executors（內置工具）             │
    │                                           │
    │  ┌─────────────────────────────────────┐ │
    │  │ FileOperationTool                   │ │
    │  │ - read file                         │ │
    │  │ - write file                        │ │
    │  │ - edit file (partial)               │ │
    │  └─────────────────────────────────────┘ │
    │                                           │
    │  ┌─────────────────────────────────────┐ │
    │  │ BashTool                            │ │
    │  │ - execute command in terminal       │ │
    │  └─────────────────────────────────────┘ │
    │                                           │
    │  ┌─────────────────────────────────────┐ │
    │  │ BrowserTool                         │ │
    │  │ - launch headless browser           │ │
    │  │ - take screenshot                  │ │
    │  │ - click/type/scroll                 │ │
    │  └─────────────────────────────────────┘ │
    │                                           │
    │  ┌─────────────────────────────────────┐ │
    │  │ MCPTools (動態)                    │ │
    │  │ - delegate to MCP servers           │ │
    │  └─────────────────────────────────────┘ │
    │                                           │
    └───────────────────────────────────────────┘
```

---

## 子系統清單

### P0 - 核心系統（必須）

| 子系統 | 文件 | 功能 | 狀態 |
|--------|------|------|------|
| **Task Runner** | `src/core/task/index.ts` | 任務編排、LLM 調用迴路 | ✅ 成熟 |
| **System Prompt** | `src/core/prompts/` | Prompt 生成、多族支援 | ✅ 成熟 |
| **API Handler** | `src/core/api/` | 多提供商適配 | ✅ 成熟 |
| **State Manager** | `src/core/storage/` | 跨平台狀態管理 | ✅ 成熟 |
| **Tool Executor** | `src/core/task/tools/` | 工具執行引擎 | ✅ 成熟 |
| **Protocol Buffers** | `proto/cline/*.proto` | gRPC 消息定義 | ✅ 成熟 |
| **Webview** | `webview-ui/src/` | React UI 前端 | ✅ 成熟 |

### P1 - 增強功能（重要）

| 子系統 | 文件 | 功能 | 狀態 |
|--------|------|------|------|
| **MCP 客戶端** | `src/core/mcp/` | 模型上下文協議 | ✅ 成熟 |
| **認證服務** | `src/services/auth/` | 多提供商授權 | ✅ 成熟 |
| **VS Code 主機** | `src/hosts/vscode/` | IDE 整合 | ✅ 成熟 |
| **瀏覽器自動化** | `src/core/task/tools/browser/` | Puppeteer 整合 | ✅ 成熟 |
| **終端命令執行** | `src/core/task/tools/bash/` | 沙箱命令執行 | ✅ 成熟 |
| **遙測** | `src/services/telemetry/` | PostHog 集成 | ✅ 成熟 |

### P2 - 實驗功能（可選）

| 子系統 | 文件 | 功能 | 狀態 |
|--------|------|------|------|
| **CLI 模式** | `cli/src/` | 終端界面（Ink） | 🔄 開發中 |
| **JetBrains 支援** | `src/hosts/jetbrains/` | IDE 擴展 | 📋 計畫中 |
| **評估框架** | `evals/` | 基準測試 | 🔄 研究中 |
| **Code Review** | `src/core/task/review/` | AI 代碼審查 | 🔄 開發中 |

---

## 關鍵設計模式

### 1. 多族 Prompt 變體（P0）

```typescript
// variants/generic/config.ts
export const config = {
  tools: [TOOL_READ_FILE, TOOL_WRITE_FILE, ...],
  rules: GENERIC_RULES
}

// variants/next-gen/config.ts
export const config = {
  tools: [TOOL_READ_FILE, TOOL_WRITE_FILE, ...],
  rules: NEXT_GEN_OPTIMIZED_RULES
}

// variants/xs/config.ts - 輕量化版本
export const config = {
  tools: [TOOL_READ_FILE, TOOL_WRITE_FILE],  // 最小工具集
  rules: XS_CONDENSED_RULES
}
```

**優勢**：同一邏輯支援 10+ 種模型，只需調整 prompt

### 2. gRPC 消息傳遞

```typescript
// Proto 定義
message ClineMessage {
  ClineSay type = 1;
  string text = 2;
  string tool_use_id = 3;
  string tool_name = 4;
  bytes tool_input = 5;  // JSON 序列化
}

// 通過 vscode.postMessage → nice-grpc 客戶端
// 實現了高效的主進程 ↔ Webview 通信
```

### 3. 人機互動審批

```typescript
// 每個工具執行都需要用戶確認
approvalGate(toolName, input) {
  showApprovalDialog()  // 顯示審批對話框
  awaitUserDecision()   // 阻塞等待
  if (approved) {
    executeToolWithSandbox()
  }
}
```

### 4. 雙層狀態管理

```typescript
// 層1：globalState（VS Code 的加密存儲）
context.globalState.get("selectedProvider")

// 層2：StateManager 緩存（內存快速訪問）
stateManager.getGlobalStateKey("selectedProvider")

// 避免每次都讀磁盤，同時保證持久化
```

---

## 與其他 Agents 的差異

| 特性 | Cline | Aider | OpenCode |
|------|-------|-------|----------|
| **形態** | IDE 擴展 | CLI 工具 | CLI 工具 |
| **UI** | Webview (React) | 終端 (TUI) | 終端 (Bubble Tea) |
| **人機互動** | 每步審批 | 交互式對話 | 自主執行 |
| **模型支援** | 15+ | 40+ | 12+ |
| **編輯方式** | 代碼片段替換 | 多格式選擇 | Diff 文件 |
| **擴展能力** | MCP + 自訂 | 無 | LSP 集成 |
| **代碼審查** | ✅ 計畫中 | ❌ | ❌ |

---

## Proto 重點

**關鍵設計決策**：
- 使用 Protocol Buffers 而非 JSON，為未來的高性能編織準備
- 通過 `npm run protos` 自動生成 TypeScript 類型
- 添加新的提供商時，需在 3 處更新 proto conversion 層（見 `.clinerules/general.md`）

---

## 總結

Cline 是 **IDE 原生的人機互動 AI 編碼助手**，其架構的核心特色是：

1. **多族 Prompt 變體**：同一任務引擎支援 10+ 種模型
2. **人機審批迴路**：每個重要操作都需用戶確認，確保安全
3. **模塊化適配層**：易於添加新的 IDE 支援（VS Code、JetBrains、CLI）
4. **MCP 擴展性**：通過標準協議無限擴展工具能力

相比 Aider（終端互動配對編程）和 OpenCode（自主代理），Cline 將控制權牢牢握在用戶手中，同時提供強大的 IDE 集成和審批機制。

