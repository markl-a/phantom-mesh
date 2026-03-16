# OpenCode 架構文檔

## 專案概覽

OpenCode 是一款 **Go 開發的終端 AI 編程助手**，提供交互式 TUI（Terminal User Interface）與完全自主的 AI Agent 編程能力。不同於 Cline（IDE 嵌入+審批）和 Aider（交互式配對），OpenCode 以 **自主決策代理** 的形式工作，內置 LSP（Language Server Protocol）支援和 MCP（Model Context Protocol）整合。

**核心定位**：
- 終端優先的自主 AI 編程代理
- TUI 界面（Bubble Tea 框架）
- LSP 集成（代碼智能）
- MCP 工具擴展
- 多提供商支援（OpenAI、Claude、Gemini、Azure、Groq 等）
- 會話管理與持久化（SQLite）

**技術棧**：
- 語言：Go 1.24
- UI 框架：Bubble Tea + Bubbles（TUI）
- 樣式：Lip Gloss + Glamour
- 數據庫：SQLite（go-sqlite3）
- 工具協議：MCP（JSON-RPC 2.0）
- LSP：標準 Language Server Protocol

---

## 目錄結構

```
opencode/
├── cmd/                           # CLI 入口點
│   ├── root.go                    # 主命令
│   └── schema/                    # Schema 生成工具
├── internal/                      # 內部模塊
│   ├── app/
│   │   ├── app.go                 # App 結構與初始化（P0）
│   │   └── lsp.go                 # LSP 客戶端管理
│   ├── config/
│   │   ├── config.go              # 配置加載
│   │   └── init.go                # 初始化設置向導
│   ├── db/
│   │   ├── db.go                  # SQLite 連接管理
│   │   ├── embed.go               # 嵌入式遷移
│   │   ├── models.go              # 數據模型
│   │   ├── sessions.sql.go        # 會話查詢（SQL-C）
│   │   ├── messages.sql.go        # 消息查詢
│   │   └── files.sql.go           # 文件追蹤查詢
│   ├── llm/                       # LLM 適配層（P0）
│   │   ├── agent/
│   │   │   ├── agent.go           # Agent 執行引擎
│   │   │   ├── agent-tool.go      # Agent 工具定義
│   │   │   ├── tools.go           # 內置工具列表
│   │   │   └── mcp-tools.go       # MCP 工具動態加載
│   │   ├── models/
│   │   │   ├── models.go          # 模型註冊表
│   │   │   ├── anthropic.go       # Claude 適配
│   │   │   ├── openai.go          # OpenAI 適配
│   │   │   ├── gemini.go          # Gemini 適配
│   │   │   ├── azure.go           # Azure OpenAI 適配
│   │   │   ├── groq.go            # Groq 適配
│   │   │   ├── xai.go             # X AI 適配
│   │   │   ├── openrouter.go      # OpenRouter 適配
│   │   │   ├── bedrock.go         # AWS Bedrock 適配
│   │   │   ├── vertexai.go        # Google Vertex AI 適配
│   │   │   ├── copilot.go         # Microsoft Copilot 適配
│   │   │   └── local.go           # 本地模型（Ollama）
│   │   ├── provider/              # 提供商實現
│   │   │   ├── provider.go        # 提供商接口
│   │   │   ├── anthropic.go       # 實現
│   │   │   ├── openai.go
│   │   │   └── ... (其他提供商)
│   │   ├── prompt/                # Prompt 生成
│   │   │   ├── prompt.go          # 基礎 prompt
│   │   │   ├── coder.go           # Coder 特化 prompt
│   │   │   ├── task.go            # 任務 prompt
│   │   │   ├── summarizer.go      # 摘要生成
│   │   │   └── title.go           # 標題生成
│   │   └── tools/                 # 工具實現（P0）
│   │       ├── tools.go           # 工具註冊
│   │       ├── bash.go            # Shell 執行
│   │       ├── file.go            # 文件操作
│   │       ├── edit.go            # 文件編輯
│   │       ├── glob.go            # 文件搜尋
│   │       ├── grep.go            # 內容搜尋
│   │       ├── fetch.go           # HTTP 請求
│   │       ├── view.go            # 文件預覽
│   │       ├── patch.go           # Patch 應用
│   │       ├── shell/             # Shell 交互
│   │       ├── diagnostics.go     # LSP 診斷
│   │       └── sourcegraph.go     # Code Search API
│   ├── lsp/                       # Language Server Protocol（P1）
│   │   ├── client.go              # LSP 客戶端
│   │   ├── protocol/              # LSP 協議實現
│   │   ├── handlers.go            # 消息處理
│   │   ├── methods.go             # 方法實現
│   │   ├── transport.go           # stdio 傳輸
│   │   └── watcher/               # 文件監視
│   ├── tui/                       # 終端 UI（P0）
│   │   ├── components/
│   │   │   ├── chat/              # 聊天區
│   │   │   │   ├── chat.go        # 聊天消息列表
│   │   │   │   ├── message.go     # 單條消息渲染
│   │   │   │   ├── editor.go      # 文本編輯器
│   │   │   │   ├── list.go        # 消息列表
│   │   │   │   └── sidebar.go     # 側邊欄
│   │   │   ├── dialog/            # 對話框
│   │   │   │   ├── complete.go    # 完成對話框
│   │   │   │   ├── commands.go    # 命令選擇
│   │   │   │   ├── custom_commands.go  # 自訂命令
│   │   │   │   └── arguments.go   # 參數輸入
│   │   │   └── core/              # 核心組件
│   │   │       └── status.go      # 狀態欄
│   │   └── theme/                 # 主題與樣式
│   ├── message/                   # 消息結構
│   │   ├── message.go             # 消息基類
│   │   ├── content.go             # 內容類型
│   │   └── attachment.go          # 附件
│   ├── session/                   # 會話管理
│   │   └── session.go             # 會話結構
│   ├── permission/                # 權限管理
│   │   └── permission.go          # 權限檢查
│   ├── pubsub/                    # 發布-訂閱
│   │   ├── broker.go              # 事件代理
│   │   └── events.go              # 事件定義
│   ├── logging/                   # 日誌
│   │   ├── logger.go              # Logger 實現
│   │   ├── message.go             # 日誌格式
│   │   └── writer.go              # 日誌輸出
│   ├── diff/                      # Diff 工具
│   │   ├── diff.go                # Diff 生成
│   │   └── patch.go               # Patch 應用
│   ├── format/                    # 格式化工具
│   │   ├── format.go              # 文本格式化
│   │   └── spinner.go             # 加載動畫
│   ├── fileutil/                  # 文件工具
│   │   └── fileutil.go            # 文件操作
│   ├── completions/               # 自動完成
│   │   └── files-folders.go       # 路徑完成
│   └── history/                   # 對話歷史
│       └── file.go                # 歷史持久化
├── go.mod                         # Go 模塊定義
├── go.sum                         # 依賴版本鎖定
└── Makefile / Dockerfile          # 構建配置
```

---

## 核心 Struct/Interface

### 1. App 結構（應用容器）

```go
type App struct {
  // 核心服務
  Sessions    session.Service       // 會話管理
  Messages    message.Service       // 消息存儲
  History     history.Service       // 文件變更歷史
  Permissions permission.Service    // 權限檢查

  // AI 代理
  CoderAgent  agent.Service         // 編碼代理引擎

  // 語言支援
  LSPClients  map[string]*lsp.Client // 語言特定的 LSP 客戶端

  // 並發控制
  clientsMutex sync.RWMutex
  watcherCancelFuncs []context.CancelFunc
  watcherWG sync.WaitGroup
}

// 初始化
func New(ctx context.Context, conn *sql.DB) (*App, error)
```

**職責**：
- 管理各子系統的生命週期
- 提供服務間的依賴注入
- 協調並發操作

### 2. Agent 服務（智能核心）

```go
type Agent interface {
  // 執行代理任務
  Execute(ctx context.Context, task string) error

  // 工具調用
  GetAvailableTools() []ToolDefinition
  CallTool(ctx context.Context, toolName string, args map[string]interface{}) (interface{}, error)
}

// 實現：agent.Service
type agentImpl struct {
  config   *AgentConfig
  llm      LLMProvider
  tools    ToolRegistry
  sessions session.Service
}
```

**流程**：
1. 接收用戶任務
2. 構建 system prompt（包括工具定義）
3. 流式調用 LLM
4. 解析工具調用
5. 執行工具（需用戶確認）
6. 彙總結果，繼續循環

### 3. Tool 定義

```go
type Tool interface {
  // 工具元信息
  Name() string
  Description() string
  InputSchema() map[string]interface{}

  // 執行
  Execute(ctx context.Context, args map[string]interface{}) (interface{}, error)
}

// 內置工具列表
const (
  TOOL_READ_FILE    = "read_file"
  TOOL_WRITE_FILE   = "write_file"
  TOOL_EDIT_FILE    = "edit_file"
  TOOL_BASH         = "bash"
  TOOL_GLOB         = "glob"
  TOOL_GREP         = "grep"
  TOOL_FETCH        = "fetch"
  TOOL_VIEW         = "view"
  TOOL_PATCH        = "patch"
  TOOL_DIAGNOSTICS  = "diagnostics"
)
```

### 4. LLM Provider 接口

```go
type LLMProvider interface {
  // 流式消息
  SendMessage(
    ctx context.Context,
    messages []Message,
    tools []ToolDefinition,
  ) (<-chan string, error)

  // 非流式
  SendMessageSync(
    ctx context.Context,
    messages []Message,
    tools []ToolDefinition,
  ) (string, error)
}

// 實現：anthropic.go、openai.go、gemini.go 等
```

### 5. Session 結構

```go
type Session struct {
  ID        string
  Title     string
  CreatedAt time.Time
  UpdatedAt time.Time

  // 模型配置
  ModelID    string
  ProviderID string

  // 消息隊列
  Messages   []*Message
}
```

### 6. LSP 客戶端

```go
type Client struct {
  // stdio 連接
  transport *StdioTransport

  // 支援的語言與功能
  capabilities *ServerCapabilities

  // 消息隊列
  requestID int64
}

// 主要方法
func (c *Client) Definition(file string, line, col int) (*Location, error)
func (c *Client) Hover(file string, line, col int) (*Hover, error)
func (c *Client) Diagnostics(file string) ([]*Diagnostic, error)
```

---

## 啟動流程

### 1. CLI 主入口

```go
cmd/root.go → main()
  → initConfig()
    - 加載 ~/.opencode/config.yaml
    - 設置默認模型、提供商
  → connectDB()
    - SQLite 連接 ~/.opencode/history.db
  → createApp()
    - App.New(ctx, conn)
  → initTUI()
    - Bubble Tea 初始化
    - 渲染 TUI 組件樹
  → app.Run(ctx)
    - 進入事件循環
```

### 2. App 初始化

```go
App.New(ctx, conn)
  → db.New(conn)
    - 創建 SQLite 查詢助手（sqlc）
  → createSessions()
    - session.NewService(q)
  → createMessages()
    - message.NewService(q)
  → createHistory()
    - history.NewService(q, conn)
  → initPermissions()
    - permission.NewPermissionService()
  → createLSPClients()
    - 後臺初始化 LSP 客戶端
  → createCoderAgent()
    - agent.NewAgent(config, services, tools)
  → return app
```

### 3. Agent 執行流程

```go
agent.Execute(ctx, userPrompt)
  → buildPrompt()
    - 加載 system prompt（coder.go）
    - 列舉可用工具定義
    - 注入會話歷史（摘要）
  → provider.SendMessage()
    - 流式調用 LLM API
    - 接收 streamed tokens
  → parseToolCalls()
    - 提取 <function_calls> 塊
    - 解析工具名稱 + 參數
  → executeTools()（逐個執行）
    - toolRegistry.Execute(toolName, args)
    - 收集執行結果
  → collectResults()
    - 彙總所有工具結果
    - 格式化為 LLM 可讀的形式
  → loopUntilCompletion()
    - 繼續循環直到：
      1. LLM 停止輸出（stop_reason）
      2. 用戶中止（Ctrl+C）
      3. 錯誤發生
  → saveToDB()
    - 保存消息與歷史
```

---

## 資料流 ASCII 圖

```
┌──────────────────────────────────────┐
│         終端 TUI (Bubble Tea)         │
│                                      │
│  ChatInput   ChatHistory             │
│     ↓            ↑                  │
│  [User Message] ← [AI Responses]    │
│     │                               │
└─────┼───────────────────────────────┘
      │
      ↓
┌──────────────────────────────────────┐
│         App 容器                     │
│  ┌────────────────────────────────┐ │
│  │  Services                      │ │
│  │  - Sessions                    │ │
│  │  - Messages                    │ │
│  │  - History                     │ │
│  │  - Permissions                 │ │
│  └────────────────────────────────┘ │
│                                      │
│  ┌────────────────────────────────┐ │
│  │  CoderAgent                    │ │
│  │  - buildPrompt()               │ │
│  │  - callLLM()                   │ │
│  │  - parseTools()                │ │
│  │  - executeTools()              │ │
│  │  - loop()                      │ │
│  └────────────────────────────────┘ │
│                                      │
│  ┌────────────────────────────────┐ │
│  │  Tool Registry                 │ │
│  │  ┌──────────────────────────┐ │ │
│  │  │ Internal Tools:          │ │ │
│  │  │ - read_file             │ │ │
│  │  │ - write_file            │ │ │
│  │  │ - bash                  │ │ │
│  │  │ - grep                  │ │ │
│  │  │ - edit_file             │ │ │
│  │  │ - patch                 │ │ │
│  │  └──────────────────────────┘ │ │
│  │  ┌──────────────────────────┐ │ │
│  │  │ MCP Tools (Dynamic)      │ │ │
│  │  │ - Delegate to MCP server │ │ │
│  │  └──────────────────────────┘ │ │
│  └────────────────────────────────┘ │
│                                      │
│  ┌────────────────────────────────┐ │
│  │  LLM Providers                 │ │
│  │  ┌──────────────────────────┐ │ │
│  │  │ Anthropic (Claude)       │ │ │
│  │  │ OpenAI (GPT)             │ │ │
│  │  │ Gemini                   │ │ │
│  │  │ Azure OpenAI             │ │ │
│  │  │ Groq                     │ │ │
│  │  │ ... (11 total)           │ │ │
│  │  └──────────────────────────┘ │ │
│  └────────────────────────────────┘ │
│                                      │
│  ┌────────────────────────────────┐ │
│  │  LSP Clients                   │ │
│  │  - TypeScript/JavaScript       │ │
│  │  - Python                      │ │
│  │  - Go                          │ │
│  │  - Rust                        │ │
│  └────────────────────────────────┘ │
└──────────────────────────────────────┘
      │
      ↓
┌──────────────────────────────────────┐
│     外部 LLM API                     │
│                                      │
│  Claude API  │  OpenAI API           │
│              │  Gemini API           │
│              │  Groq API             │
│              │  ... (8 more)         │
│                                      │
│  返回：Tool Calls + Text Response   │
└──────────────────────────────────────┘
      │
      ├→ Tool Execution
      │
      ↓
┌──────────────────────────────────────┐
│     SQLite 數據庫                    │
│  ~/.opencode/history.db             │
│                                      │
│  - sessions                         │
│  - messages                         │
│  - files                            │
│  - attachments                      │
└──────────────────────────────────────┘
```

---

## 子系統清單

### P0 - 核心系統（必須）

| 子系統 | 文件 | 功能 | 狀態 |
|--------|------|------|------|
| **App 容器** | `internal/app/app.go` | 生命週期管理、DI | ✅ 成熟 |
| **Agent 引擎** | `internal/llm/agent/agent.go` | 任務編排、工具調用迴路 | ✅ 成熟 |
| **LLM 適配** | `internal/llm/provider/*.go` | 11 個提供商實現 | ✅ 成熟 |
| **工具系統** | `internal/llm/tools/*.go` | 15+ 內置工具 | ✅ 成熟 |
| **TUI 界面** | `internal/tui/components/` | Bubble Tea 組件 | ✅ 成熟 |
| **SQLite DB** | `internal/db/` | 會話與消息持久化 | ✅ 成熟 |
| **Prompt 生成** | `internal/llm/prompt/` | 動態 system prompt | ✅ 成熟 |

### P1 - 增強功能（重要）

| 子系統 | 文件 | 功能 | 狀態 |
|--------|------|------|------|
| **LSP 集成** | `internal/lsp/` | 語言智能、代碼完成 | ✅ 成熟 |
| **MCP 支援** | `internal/llm/agent/mcp-tools.go` | 模型上下文協議 | ✅ 成熟 |
| **會話管理** | `internal/session/` | 多會話、狀態保存 | ✅ 成熟 |
| **文件監視** | `internal/lsp/watcher/` | 實時文件變化追蹤 | ✅ 成熟 |
| **Diff 工具** | `internal/diff/` | Diff 生成與應用 | ✅ 成熟 |
| **自訂命令** | `internal/tui/components/dialog/custom_commands.go` | 命令範本 | ✅ 成熟 |

### P2 - 實驗功能（可選）

| 子系統 | 文件 | 功能 | 狀態 |
|--------|------|------|------|
| **語音輸入** | N/A | 語音轉文字 | 📋 計畫中 |
| **Remote LSP** | N/A | 遠程語言服務 | 🔄 研究中 |
| **Web 界面** | N/A | HTTP API + Frontend | 📋 計畫中 |
| **代碼審查** | N/A | AI 代碼審查模式 | 📋 計畫中 |

---

## 關鍵設計模式

### 1. 服務注入（Dependency Injection）

```go
type Agent interface {
  Execute(ctx context.Context, task string) error
}

type agentImpl struct {
  config    *AgentConfig
  llm       LLMProvider       // 注入
  tools     ToolRegistry      // 注入
  sessions  session.Service   // 注入
}

// App 在構造時注入依賴
func New(ctx context.Context, conn *sql.DB) (*App, error) {
  sessions := session.NewService(q)
  tools := createToolRegistry()
  agent := agent.NewAgent(config, llm, tools, sessions)
  return &App{CoderAgent: agent, Sessions: sessions}
}
```

**優勢**：易於測試、解耦、替換實現

### 2. 工具註冊表

```go
type ToolRegistry struct {
  tools map[string]Tool
}

func (r *ToolRegistry) Register(tool Tool) {
  r.tools[tool.Name()] = tool
}

func (r *ToolRegistry) Execute(name string, args map[string]interface{}) (interface{}, error) {
  tool, ok := r.tools[name]
  if !ok {
    return nil, fmt.Errorf("unknown tool: %s", name)
  }
  return tool.Execute(ctx, args)
}
```

### 3. 流式 LLM 調用

```go
// 返回通道，逐步發送 tokens
func (p *Provider) SendMessage(
  ctx context.Context,
  messages []Message,
  tools []ToolDefinition,
) (<-chan string, error) {
  respCh := make(chan string, 10)

  go func() {
    defer close(respCh)
    stream := p.client.CreateMessageStream(messages, tools)
    for {
      select {
      case chunk := <-stream:
        respCh <- chunk
      case <-ctx.Done():
        return
      }
    }
  }()

  return respCh, nil
}
```

### 4. 數據庫遷移與查詢

```go
// 使用 sqlc 生成類型安全的查詢代碼
// db/sessions.sql.go 由 sqlc 生成
func (q *Queries) CreateSession(ctx context.Context, arg CreateSessionParams) (Session, error)
func (q *Queries) GetSessionByID(ctx context.Context, id string) (Session, error)

// 在 schema.sql 中定義 SQL，sqlc 自動生成 Go 代碼
```

---

## 與其他專案的差異

| 特性 | OpenCode | Aider | Cline |
|------|----------|-------|-------|
| **語言** | Go | Python | TypeScript |
| **UI** | TUI (Bubble Tea) | TUI (prompt_toolkit) | IDE Webview |
| **代理自主性** | 完全自主 | 交互式 | 人機審批 |
| **模型數** | 12+ | 40+ | 15+ |
| **LSP 集成** | ✅ | ❌ | ❌ |
| **MCP 支援** | ✅ | ❌ | ✅ |
| **會話持久化** | SQLite | 檔案系統 | VS Code 存儲 |
| **啟動速度** | 極快（Go 二進位） | 中等（Python 直譯） | 中等（Node.js） |

---

## 性能特點

- **超快啟動**：編譯後的 Go 二進製，毫秒級啟動
- **低資源佔用**：單個二進製（< 30MB），記憶體效率高
- **流式處理**：使用 Go 通道，高效的流式 I/O
- **並發工具執行**：支援多工具並行執行
- **LSP 智能**：本地語言服務，零延遲代碼完成

---

## 總結

OpenCode 是 **Go 構建的自主 AI 編程代理**，其架構的核心特色是：

1. **完全自主性**：Agent 在會話內自主決策和執行
2. **LSP + MCP**：雙協議整合，提供最強的工具擴展性
3. **TUI 友好**：Bubble Tea 提供流暢的終端體驗
4. **高效構建**：Go 的編譯效率和運行時性能
5. **多提供商**：支援 12+ 個 LLM 提供商

相比 Aider（交互式配對）和 Cline（人機審批），OpenCode 提供了 **最大化的自主性** 和 **最完整的工具生態**（LSP + MCP），適合那些信任 AI 自主決策的開發者。

