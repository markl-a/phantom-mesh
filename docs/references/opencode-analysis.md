# OpenCode 深度技術分析

> 分析日期：2026-03-12
> 專案來源：`references/opencode/` (github.com/opencode-ai/opencode)
> 語言：Go 1.24 | 117K+ GitHub Stars
> 定位：終端機 AI 編碼助手 CLI，支援 75+ LLM 供應商

---

## 目錄

1. [專案結構](#1-專案結構)
2. [入口點與啟動流程](#2-入口點與啟動流程)
3. [核心架構](#3-核心架構)
   - 3.1 [Agent 迴圈](#31-agent-迴圈)
   - 3.2 [Provider 抽象層](#32-provider-抽象層)
   - 3.3 [工具系統](#33-工具系統)
   - 3.4 [LSP 整合](#34-lsp-整合)
4. [TUI 終端介面](#4-tui-終端介面)
5. [Git 整合](#5-git-整合)
6. [上下文管理](#6-上下文管理)
7. [Session / 歷史記錄](#7-session--歷史記錄)
8. [多模型支援](#8-多模型支援)
9. [值得採用的關鍵模式](#9-值得採用的關鍵模式)
10. [與 clawtex-core 的比較](#10-與-clawtex-core-的比較)

---

## 1. 專案結構

```
opencode/
├── main.go                      # 入口點（極簡，僅呼叫 cmd.Execute()）
├── cmd/
│   ├── root.go                  # Cobra CLI 命令定義 + TUI 啟動
│   └── schema/main.go           # JSON Schema 產生器
├── internal/
│   ├── app/
│   │   ├── app.go               # App 結構體：組合所有服務（Session、Message、Agent、LSP）
│   │   └── lsp.go               # LSP 客戶端初始化與工作區監視器
│   ├── completions/
│   │   └── files-folders.go     # 檔案/資料夾自動補全
│   ├── config/
│   │   ├── config.go            # 設定載入：Viper + JSON + 環境變數
│   │   └── init.go              # 專案初始化對話框邏輯
│   ├── db/
│   │   ├── connect.go           # SQLite 連線（go-sqlite3/WASM）
│   │   ├── embed.go             # 嵌入式 migration SQL
│   │   ├── migrations/          # Goose SQL migration 檔案
│   │   ├── sql/                 # sqlc 原始查詢
│   │   ├── *.sql.go             # sqlc 產生的型別安全查詢
│   │   └── models.go            # sqlc 產生的資料模型
│   ├── diff/
│   │   ├── diff.go              # Unified diff 產生
│   │   └── patch.go             # Patch 解析與套用
│   ├── fileutil/                # 檔案工具（跳過隱藏檔等）
│   ├── format/                  # 輸出格式化（text/json）+ Spinner
│   ├── history/
│   │   └── file.go              # 檔案版本歷史服務（SQLite）
│   ├── llm/
│   │   ├── agent/
│   │   │   ├── agent.go         # 核心 Agent 迴圈（processGeneration）
│   │   │   ├── agent-tool.go    # 子 Agent 工具（task agent 委派）
│   │   │   ├── mcp-tools.go     # MCP 工具整合（stdio / SSE）
│   │   │   └── tools.go         # 工具清單組裝（CoderAgentTools / TaskAgentTools）
│   │   ├── models/
│   │   │   ├── models.go        # Model 結構體 + SupportedModels 全域 map
│   │   │   ├── anthropic.go     # Anthropic 模型定義（Claude 3.5/3.7/4）
│   │   │   ├── openai.go        # OpenAI 模型定義（GPT-4.1, o1, o3, o4）
│   │   │   ├── gemini.go        # Gemini 模型定義
│   │   │   ├── copilot.go       # GitHub Copilot 模型（14 種，全免費）
│   │   │   ├── azure.go         # Azure OpenAI 模型
│   │   │   ├── groq.go          # Groq 模型
│   │   │   ├── openrouter.go    # OpenRouter 模型
│   │   │   ├── vertexai.go      # Google VertexAI 模型
│   │   │   ├── xai.go           # xAI (Grok) 模型
│   │   │   └── local.go         # 本地模型（OpenAI 相容端點）
│   │   ├── prompt/
│   │   │   ├── prompt.go        # Prompt 組裝：system prompt + context paths
│   │   │   ├── coder.go         # 主 Coder Agent prompt（Anthropic / OpenAI 兩版）
│   │   │   ├── task.go          # Task Agent prompt
│   │   │   ├── summarizer.go    # 摘要 Agent prompt
│   │   │   └── title.go         # 標題產生 prompt
│   │   ├── provider/
│   │   │   ├── provider.go      # Provider 介面 + 工廠函式 + baseProvider 泛型
│   │   │   ├── anthropic.go     # Anthropic SDK 客戶端（串流 + 重試 + 快取）
│   │   │   ├── openai.go        # OpenAI SDK 客戶端
│   │   │   ├── gemini.go        # Gemini API 客戶端（google.golang.org/genai）
│   │   │   ├── copilot.go       # GitHub Copilot API 客戶端
│   │   │   ├── azure.go         # Azure OpenAI 客戶端
│   │   │   ├── bedrock.go       # AWS Bedrock 客戶端
│   │   │   └── vertexai.go      # Google VertexAI 客戶端
│   │   └── tools/
│   │       ├── tools.go         # BaseTool 介面 + ToolInfo/ToolResponse 型別
│   │       ├── bash.go          # Shell 命令工具（安全過濾 + 權限閘門）
│   │       ├── edit.go          # 精確文字替換編輯
│   │       ├── write.go         # 整檔寫入
│   │       ├── view.go          # 檔案讀取（行號 + LSP 診斷）
│   │       ├── patch.go         # Unified diff patch 套用
│   │       ├── grep.go          # 內容搜尋（ripgrep 優先 + regex 回退）
│   │       ├── glob.go          # 檔案名稱 pattern 搜尋
│   │       ├── ls.go            # 目錄列表（樹狀）
│   │       ├── fetch.go         # HTTP URL 抓取
│   │       ├── diagnostics.go   # LSP 診斷查詢工具
│   │       ├── sourcegraph.go   # Sourcegraph 代碼搜尋
│   │       └── shell/shell.go   # 持久化 shell session 管理
│   ├── logging/                 # 結構化日誌 + PubSub 整合
│   ├── lsp/
│   │   ├── client.go            # LSP 客戶端：stdio 通訊 + 初始化 + 檔案管理
│   │   ├── handlers.go          # 伺服器請求處理（applyEdit、diagnostics）
│   │   ├── language.go          # 語言 ID 偵測
│   │   ├── methods.go           # LSP 方法呼叫封裝
│   │   ├── transport.go         # JSON-RPC 2.0 傳輸層
│   │   ├── protocol.go          # LSP 協定型別封裝
│   │   ├── protocol/            # 完整 LSP 協定型別定義（自動產生）
│   │   ├── util/edit.go         # 編輯操作工具
│   │   └── watcher/watcher.go   # 檔案系統監視器（fsnotify）
│   ├── message/
│   │   ├── message.go           # Message 服務 + 多型 ContentPart 序列化
│   │   ├── content.go           # ContentPart 介面（Text/Binary/ToolCall/ToolResult/Finish）
│   │   └── attachment.go        # 附件處理
│   ├── permission/
│   │   └── permission.go        # 權限閘門服務（Request → Grant/Deny 非同步）
│   ├── pubsub/
│   │   ├── broker.go            # 泛型 Pub/Sub Broker（channel-based）
│   │   └── events.go            # 事件型別定義
│   ├── session/
│   │   └── session.go           # Session CRUD 服務（SQLite）
│   ├── tui/
│   │   ├── tui.go               # 主 TUI model（Bubble Tea）
│   │   ├── components/
│   │   │   ├── chat/            # 聊天元件（訊息列表、編輯器、側邊欄）
│   │   │   ├── core/status.go   # 狀態列
│   │   │   ├── dialog/          # 對話框（權限、Session、命令、模型、主題）
│   │   │   ├── logs/            # 日誌檢視器
│   │   │   └── util/            # 簡易列表元件
│   │   ├── image/               # 終端圖片渲染
│   │   ├── layout/              # 容器、分割、覆蓋層佈局
│   │   ├── page/                # 頁面路由（Chat、Logs）
│   │   ├── styles/              # 樣式 + Markdown 渲染 + 圖標
│   │   ├── theme/               # 主題系統（10 種：OpenCode、Catppuccin、Dracula...）
│   │   └── util/                # TUI 工具函式
│   └── version/                 # 版本資訊
├── scripts/                     # 建置腳本
├── go.mod / go.sum              # Go 模組依賴
├── sqlc.yaml                    # sqlc 設定（SQLite）
└── opencode-schema.json         # 設定檔 JSON Schema
```

**關鍵統計**：
- Go 原始檔：約 85 個
- 總依賴：約 130 個（直接 + 間接）
- 資料庫：SQLite（透過 go-sqlite3 WASM 綁定，無 CGO）
- 測試：少量（`ls_test.go`、`prompt_test.go`、`theme_test.go`、`custom_commands_test.go`）

---

## 2. 入口點與啟動流程

### main.go（3 行）

```go
// main.go
func main() {
    defer logging.RecoverPanic("main", func() {
        logging.ErrorPersist("Application terminated due to unhandled panic")
    })
    cmd.Execute()
}
```

極簡入口，所有邏輯在 `cmd/root.go`。

### cmd/root.go 啟動順序

```
1. cobra.Command.RunE 觸發
2. 解析 flags：--debug, --cwd, --prompt, --output-format, --quiet
3. config.Load(cwd, debug)
   ├── configureViper() — 設定搜尋路徑：$HOME/.opencode.json, $XDG_CONFIG_HOME/opencode/
   ├── setDefaults(debug) — 設定預設值（data目錄、shell路徑、autoCompact）
   ├── readConfig() — 讀取全域設定
   ├── mergeLocalConfig() — 合併專案本地 .opencode.json
   ├── setProviderDefaults() — 按優先序偵測 API Key（Copilot > Anthropic > OpenAI > Gemini > Groq...）
   ├── viper.Unmarshal(cfg) — 反序列化到 Config struct
   ├── applyDefaultValues() — MCP 預設 type=stdio
   └── Validate() — 驗證 agent model、provider key、LSP config
4. db.Connect() — SQLite 連線 + goose migration
5. app.New(ctx, conn)
   ├── session.NewService(q)
   ├── message.NewService(q)
   ├── history.NewService(q, conn)
   ├── permission.NewPermissionService()
   ├── initTheme()
   ├── go initLSPClients(ctx) — 背景啟動所有 LSP 伺服器
   └── agent.NewAgent(AgentCoder, ...)
       ├── createAgentProvider(AgentCoder) — 建立主 Provider
       ├── createAgentProvider(AgentTitle) — 建立標題 Provider
       └── createAgentProvider(AgentSummarizer) — 建立摘要 Provider
6. go initMCPTools(ctx, app) — 背景初始化 MCP 工具（30秒逾時）
7. 分支：
   ├── prompt != "" → app.RunNonInteractive() — 非互動模式
   └── prompt == "" → 互動模式
       ├── zone.NewGlobal() — bubblezone 初始化
       ├── tea.NewProgram(tui.New(app), tea.WithAltScreen())
       ├── setupSubscriptions(app, ctx) — 訂閱 5 個事件頻道
       │   ├── logging.Subscribe
       │   ├── app.Sessions.Subscribe
       │   ├── app.Messages.Subscribe
       │   ├── app.Permissions.Subscribe
       │   └── app.CoderAgent.Subscribe
       ├── go goroutine 轉發 PubSub 事件 → TUI program.Send()
       └── program.Run() — 進入 Bubble Tea 主迴圈
```

### 非互動模式

```go
// app/app.go - RunNonInteractive
func (a *App) RunNonInteractive(ctx context.Context, prompt, format string, quiet bool) error {
    sess, _ := a.Sessions.Create(ctx, title)
    a.Permissions.AutoApproveSession(sess.ID)   // 自動批准所有權限
    done, _ := a.CoderAgent.Run(ctx, sess.ID, prompt)
    result := <-done                             // 阻塞等待完成
    fmt.Println(format.FormatOutput(content, outputFormat))
}
```

關鍵設計：非互動模式自動批准權限，允許 `opencode -p "..." -f json` 用於 CI/CD 流水線。

---

## 3. 核心架構

### 3.1 Agent 迴圈

**檔案**：`internal/llm/agent/agent.go`

Agent 核心是一個 **同步工具使用迴圈**（tool-use loop），不同於 clawtex-core 的非同步 agent_runtime：

```go
// agent.go - processGeneration 核心迴圈
func (a *agent) processGeneration(ctx, sessionID, content, attachmentParts) AgentEvent {
    // 1. 載入歷史訊息
    msgs, _ := a.messages.List(ctx, sessionID)

    // 2. 處理 summary（如果有 SummaryMessageID，截斷歷史）
    if session.SummaryMessageID != "" {
        msgs = msgs[summaryMsgIndex:]
        msgs[0].Role = message.User  // 摘要訊息作為新的用戶訊息
    }

    // 3. 建立用戶訊息，附加到歷史
    userMsg, _ := a.createUserMessage(ctx, sessionID, content, attachmentParts)
    msgHistory := append(msgs, userMsg)

    // 4. Tool-use 迴圈
    for {
        select {
        case <-ctx.Done():
            return a.err(ctx.Err())
        default:
        }

        // 串流 LLM 回應 + 收集工具呼叫
        agentMessage, toolResults, err := a.streamAndHandleEvents(ctx, sessionID, msgHistory)

        // 如果有工具呼叫，將結果附加到歷史，繼續迴圈
        if agentMessage.FinishReason() == FinishReasonToolUse && toolResults != nil {
            msgHistory = append(msgHistory, agentMessage, *toolResults)
            continue
        }

        // 否則返回最終回應
        return AgentEvent{Type: AgentEventTypeResponse, Message: agentMessage, Done: true}
    }
}
```

**streamAndHandleEvents 詳細流程**：

```go
func (a *agent) streamAndHandleEvents(ctx, sessionID, msgHistory) {
    // 1. 啟動 LLM 串流
    eventChan := a.provider.StreamResponse(ctx, msgHistory, a.tools)

    // 2. 建立空的 assistant message（持久化到 DB）
    assistantMsg, _ := a.messages.Create(ctx, sessionID, ...)

    // 3. 逐事件處理串流
    for event := range eventChan {
        processEvent(ctx, sessionID, &assistantMsg, event)
        // EventThinkingDelta → 追加推理內容
        // EventContentDelta → 追加文字內容
        // EventToolUseStart → 添加工具呼叫
        // EventToolUseStop → 完成工具呼叫
        // EventComplete → 設定完成原因、追蹤 token 使用
    }

    // 4. 逐一執行工具呼叫
    for i, toolCall := range assistantMsg.ToolCalls() {
        tool := findTool(toolCall.Name)
        toolResult, err := tool.Run(ctx, toolCall)
        // 權限被拒 → 立即終止所有後續工具
    }

    // 5. 建立工具結果訊息
    msg, _ := a.messages.Create(ctx, sessionID, toolResultParts)
    return assistantMsg, &msg, nil
}
```

**四種 Agent 類型**：

| Agent | 用途 | 工具 | Provider |
|-------|------|------|----------|
| `AgentCoder` | 主程式撰寫 | 全部 12 個 + MCP + LSP 診斷 | 完整模型 |
| `AgentTask` | 子任務（唯讀搜尋） | Glob, Grep, LS, Sourcegraph, View | 較小模型 |
| `AgentTitle` | 對話標題產生 | 無 | 小模型（maxTokens=80） |
| `AgentSummarizer` | 對話壓縮 | 無 | 中等模型 |

**子 Agent（agent-tool.go）設計**：

```go
// AgentTool 作為一個 Tool 被主 Agent 呼叫
func (b *agentTool) Run(ctx, call) (ToolResponse, error) {
    // 建立新 Agent 實例（AgentTask 類型）
    agent, _ := NewAgent(config.AgentTask, b.sessions, b.messages, TaskAgentTools(b.lspClients))
    // 建立子 Session
    session, _ := b.sessions.CreateTaskSession(ctx, call.ID, parentSessionID, "New Agent Session")
    // 執行子 Agent
    done, _ := agent.Run(ctx, session.ID, params.Prompt)
    result := <-done
    // 將成本累計到父 Session
    parentSession.Cost += updatedSession.Cost
    return tools.NewTextResponse(result.Message.Content().String()), nil
}
```

### 3.2 Provider 抽象層

**檔案**：`internal/llm/provider/provider.go`

#### Provider 介面

```go
type Provider interface {
    SendMessages(ctx context.Context, messages []message.Message, tools []tools.BaseTool) (*ProviderResponse, error)
    StreamResponse(ctx context.Context, messages []message.Message, tools []tools.BaseTool) <-chan ProviderEvent
    Model() models.Model
}
```

僅三個方法，極簡。

#### 泛型 baseProvider

```go
type ProviderClient interface {
    send(ctx context.Context, messages []message.Message, tools []tools.BaseTool) (*ProviderResponse, error)
    stream(ctx context.Context, messages []message.Message, tools []tools.BaseTool) <-chan ProviderEvent
}

type baseProvider[C ProviderClient] struct {
    options providerClientOptions
    client  C
}
```

使用 Go 泛型消除重複的委派代碼。`baseProvider` 負責清理空訊息後委派給具體客戶端。

#### 工廠函式

```go
func NewProvider(providerName models.ModelProvider, opts ...ProviderClientOption) (Provider, error) {
    switch providerName {
    case models.ProviderCopilot:
        return &baseProvider[CopilotClient]{client: newCopilotClient(opts)}, nil
    case models.ProviderAnthropic:
        return &baseProvider[AnthropicClient]{client: newAnthropicClient(opts)}, nil
    case models.ProviderOpenAI:
        return &baseProvider[OpenAIClient]{client: newOpenAIClient(opts)}, nil
    case models.ProviderGROQ:
        // Groq 重用 OpenAI 客戶端，僅更改 baseURL
        opts.openaiOptions = append(opts.openaiOptions, WithOpenAIBaseURL("https://api.groq.com/openai/v1"))
        return &baseProvider[OpenAIClient]{client: newOpenAIClient(opts)}, nil
    case models.ProviderOpenRouter:
        // OpenRouter 重用 OpenAI 客戶端，加自訂 headers
        opts.openaiOptions = append(opts.openaiOptions,
            WithOpenAIBaseURL("https://openrouter.ai/api/v1"),
            WithOpenAIExtraHeaders(map[string]string{"HTTP-Referer": "opencode.ai"}),
        )
        return &baseProvider[OpenAIClient]{client: newOpenAIClient(opts)}, nil
    case models.ProviderLocal:
        // 本地模型重用 OpenAI 客戶端
        opts.openaiOptions = append(opts.openaiOptions, WithOpenAIBaseURL(os.Getenv("LOCAL_ENDPOINT")))
        return &baseProvider[OpenAIClient]{client: newOpenAIClient(opts)}, nil
    // ...Azure, Bedrock, Gemini, VertexAI, XAI
    }
}
```

**支援 75+ Provider 的祕密**：大量 Provider 重用 `OpenAIClient`（Groq、OpenRouter、XAI、Local 都是 OpenAI 相容端點）。實際獨立實作只有 6 個：Anthropic、OpenAI、Gemini、Copilot、Azure、Bedrock、VertexAI。

#### Anthropic 實作要點（anthropic.go）

```go
// 快取控制：最後 3 條訊息標記 ephemeral 快取
if i > len(messages)-3 {
    content.OfText.CacheControl = anthropic.CacheControlEphemeralParam{Type: "ephemeral"}
}

// 工具最後一項加快取
if i == len(tools)-1 && !a.options.disableCache {
    toolParam.CacheControl = anthropic.CacheControlEphemeralParam{Type: "ephemeral"}
}

// 動態思考控制
if a.options.shouldThink != nil && a.options.shouldThink(messageContent) {
    thinkingParam = anthropic.ThinkingConfigParamOfEnabled(int64(maxTokens * 0.8))
    temperature = 1  // 思考模式需要 temperature=1
}

// 指數退避重試（429/529 錯誤）
backoffMs := 2000 * (1 << (attempts - 1))
jitterMs := int(float64(backoffMs) * 0.2)
```

#### Options 模式

```go
type ProviderClientOption func(*providerClientOptions)

// 每個 provider 有自己的 sub-options
type providerClientOptions struct {
    apiKey, model, maxTokens, systemMessage string
    anthropicOptions []AnthropicOption   // shouldThink, useBedrock, disableCache
    openaiOptions    []OpenAIOption      // baseURL, extraHeaders, reasoningEffort
    geminiOptions    []GeminiOption
    bedrockOptions   []BedrockOption
    copilotOptions   []CopilotOption
}
```

### 3.3 工具系統

**檔案**：`internal/llm/tools/tools.go`

#### BaseTool 介面

```go
type BaseTool interface {
    Info() ToolInfo
    Run(ctx context.Context, params ToolCall) (ToolResponse, error)
}

type ToolInfo struct {
    Name        string
    Description string
    Parameters  map[string]any    // JSON Schema 格式
    Required    []string
}

type ToolResponse struct {
    Type     toolResponseType  // "text" | "image"
    Content  string
    Metadata string            // JSON 字串，用於 TUI 顯示 diff 等
    IsError  bool
}
```

#### 12 個內建工具

| 工具 | 檔案 | 功能 | 權限 |
|------|------|------|------|
| `bash` | bash.go | Shell 命令執行 | 非唯讀命令需要 |
| `edit` | edit.go | 精確文字替換 | 需要 |
| `write` | write.go | 整檔覆蓋寫入 | 需要 |
| `patch` | patch.go | Unified diff patch | 需要 |
| `view` | view.go | 讀取檔案（帶行號） | 不需要 |
| `glob` | glob.go | 檔案名稱搜尋 | 不需要 |
| `grep` | grep.go | 內容搜尋 | 不需要 |
| `ls` | ls.go | 目錄列表 | 不需要 |
| `fetch` | fetch.go | HTTP URL 抓取 | 需要 |
| `diagnostics` | diagnostics.go | LSP 診斷查詢 | 不需要 |
| `sourcegraph` | sourcegraph.go | Sourcegraph 搜尋 | 不需要 |
| `agent` | agent-tool.go | 子 Agent 委派 | 不需要 |

#### Bash 工具安全設計

```go
// 禁止的命令（網路存取限制）
var bannedCommands = []string{
    "alias", "curl", "wget", "nc", "telnet", "chrome", "firefox", ...
}

// 安全唯讀命令（不需要權限確認）
var safeReadOnlyCommands = []string{
    "ls", "echo", "pwd", "git status", "git log", "git diff",
    "go version", "go test", "go build", ...
}

// 持久化 Shell Session
shell := shell.GetPersistentShell(config.WorkingDirectory())
stdout, stderr, exitCode, interrupted, err := shell.Exec(ctx, command, timeout)
```

**特點**：
- 持久化 shell session（環境變數、目錄狀態跨命令保留）
- 輸出截斷（30000 字元，中間截取）
- 逾時控制（預設 1 分鐘，最大 10 分鐘）

#### Edit 工具的檔案追蹤機制

```go
// 1. 修改前讀取驗證
if getLastReadTime(filePath).IsZero() {
    return error("you must read the file before editing it. Use the View tool first")
}
// 2. 修改時間檢查（防止衝突）
if modTime.After(lastRead) {
    return error("file has been modified since it was last read")
}
// 3. 唯一性檢查
index := strings.Index(oldContent, oldString)
lastIndex := strings.LastIndex(oldContent, oldString)
if index != lastIndex {
    return error("old_string appears multiple times")
}
// 4. 歷史版本記錄
e.files.Create(ctx, sessionID, filePath, oldContent)
e.files.CreateVersion(ctx, sessionID, filePath, newContent)
// 5. LSP 通知
waitForLspDiagnostics(ctx, filePath, e.lspClients)
```

#### MCP 工具整合

```go
// mcp-tools.go
func GetMcpTools(ctx context.Context, permissions permission.Service) []tools.BaseTool {
    for name, m := range config.Get().MCPServers {
        switch m.Type {
        case config.MCPStdio:
            c, _ := client.NewStdioMCPClient(m.Command, m.Env, m.Args...)
        case config.MCPSse:
            c, _ := client.NewSSEMCPClient(m.URL, client.WithHeaders(m.Headers))
        }
        // 列出工具、封裝為 mcpTool
        tools, _ := c.ListTools(ctx, mcp.ListToolsRequest{})
        for _, t := range tools.Tools {
            mcpTools = append(mcpTools, NewMcpTool(name, t, permissions, m))
        }
    }
}
```

MCP 工具名稱格式：`{mcpServerName}_{toolName}`（例如 `filesystem_read_file`）。每次執行都重新初始化 MCP 客戶端（非常規作法，但避免了連線狀態管理）。

### 3.4 LSP 整合

**檔案**：`internal/lsp/client.go`、`internal/app/lsp.go`

#### 架構

```
[Config: lsp.go] → map[string]LSPConfig
    ├── key: "go" → { command: "gopls", args: [...] }
    └── key: "typescript" → { command: "vtsls", args: [...] }

[App.initLSPClients] → 每個 LSP config 啟動一個 goroutine
    └── createAndStartLSPClient(ctx, name, command, args)
        ├── lsp.NewClient(ctx, command, args...) — 啟動子進程
        ├── InitializeLSPClient(ctx, workspaceDir) — LSP initialize
        ├── WaitForServerReady(ctx) — 輪詢等待就緒
        └── watcher.NewWorkspaceWatcher(lspClient) — fsnotify 檔案監視
```

#### LSP 客戶端特色

```go
// 智慧伺服器類型偵測
func (c *Client) detectServerType() ServerType {
    switch {
    case strings.Contains(cmdPath, "gopls"):      return ServerTypeGo
    case strings.Contains(cmdPath, "typescript"):  return ServerTypeTypeScript
    case strings.Contains(cmdPath, "rust-analyzer"): return ServerTypeRust
    case strings.Contains(cmdPath, "pyright"):     return ServerTypePython
    }
}

// 根據伺服器類型開啟關鍵設定檔
func (c *Client) openKeyConfigFiles(ctx context.Context) {
    switch serverType {
    case ServerTypeTypeScript:
        filesToOpen = []string{"tsconfig.json", "package.json", "jsconfig.json"}
    case ServerTypeGo:
        filesToOpen = []string{"go.mod", "go.sum"}
    case ServerTypeRust:
        filesToOpen = []string{"Cargo.toml", "Cargo.lock"}
    }
}
```

#### LSP 與工具的整合

編輯工具（edit, write, patch）在修改檔案後會：
1. 通知 LSP 伺服器檔案變更（`NotifyChange`）
2. 等待短暫時間讓 LSP 處理
3. 取得診斷資訊附加到工具回應中

```go
// tools/edit.go
waitForLspDiagnostics(ctx, params.FilePath, e.lspClients)
text += getDiagnostics(params.FilePath, e.lspClients)
```

這讓 LLM 能看到自己改動引起的編譯錯誤和類型錯誤，形成**自我修正迴圈**。

---

## 4. TUI 終端介面

**檔案**：`internal/tui/tui.go`

### 技術堆疊

| 函式庫 | 用途 |
|--------|------|
| `charmbracelet/bubbletea` | Elm 架構 TUI 框架 |
| `charmbracelet/lipgloss` | CSS-like 終端樣式 |
| `charmbracelet/bubbles` | 預建元件（textinput, key bindings） |
| `charmbracelet/glamour` | Markdown 渲染 |
| `lrstanley/bubblezone` | 滑鼠事件區域管理 |
| `alecthomas/chroma` | 語法高亮 |

### 架構

```
appModel (Bubble Tea Model)
├── pages: map[PageID]tea.Model
│   ├── ChatPage — 聊天頁面
│   │   ├── chat.Chat — 訊息列表
│   │   ├── chat.Editor — 輸入框
│   │   └── chat.Sidebar — 側邊欄
│   └── LogsPage — 日誌頁面
│       ├── logs.Table — 日誌表格
│       └── logs.Details — 日誌詳情
├── status: StatusCmp — 底部狀態列
├── dialogs: 9 個對話框
│   ├── PermissionDialog — 權限確認
│   ├── SessionDialog — Session 切換
│   ├── CommandDialog — 命令面板（Ctrl+K）
│   ├── ModelDialog — 模型選擇（Ctrl+O）
│   ├── FilepickerDialog — 檔案選擇（Ctrl+F）
│   ├── ThemeDialog — 主題切換（Ctrl+T）
│   ├── HelpDialog — 快捷鍵說明
│   ├── QuitDialog — 退出確認
│   ├── InitDialog — 專案初始化
│   └── MultiArgumentsDialog — 多參數輸入
└── 主題系統: 10 種主題
    OpenCode, Catppuccin, Dracula, Flexoki, Gruvbox,
    Monokai, OneDark, TokyoNight, Tron
```

### 事件流

```
PubSub Events → setupSubscriptions → channel → goroutine → program.Send(msg) → Update()
                                                                                    │
    ┌──────────────────────────────────────────────────────────────────────────────┘
    │
    v
appModel.Update(msg) {
    switch msg.(type) {
    case pubsub.Event[logging.LogMessage]:     → 更新狀態列
    case pubsub.Event[permission.Permission]:  → 顯示權限對話框
    case pubsub.Event[agent.AgentEvent]:       → 處理回應/摘要/自動壓縮
    case pubsub.Event[session.Session]:        → 更新 Session 資訊
    case tea.KeyMsg:                           → 分派快捷鍵
    }
}
```

### 自動壓縮機制

```go
// 當 token 使用達到 context window 的 95% 時自動觸發摘要
if payload.Done && payload.Type == agent.AgentEventTypeResponse {
    tokens := session.CompletionTokens + session.PromptTokens
    if (tokens >= int64(float64(contextWindow)*0.95)) && config.Get().AutoCompact {
        return a, util.CmdHandler(startCompactSessionMsg{})
    }
}
```

### 命令系統

```go
// 內建命令
model.RegisterCommand(dialog.Command{
    ID:    "init",
    Title: "Initialize Project",
    Handler: func(cmd) tea.Cmd {
        // 分析程式庫，建立 OpenCode.md 記憶檔
    },
})

model.RegisterCommand(dialog.Command{
    ID:    "compact",
    Title: "Compact Session",
    Handler: func(cmd) tea.Cmd {
        return startCompactSessionMsg{}
    },
})

// 自訂命令從 .opencode/commands/ 載入
customCommands, _ := dialog.LoadCustomCommands()
```

---

## 5. Git 整合

OpenCode 的 Git 整合**完全透過 bash 工具**，沒有原生 Git 庫。關鍵設計在 prompt 中：

```
# bash.go 中的 prompt（截取）
# Committing changes with git
1. 先執行 git status, git diff, git log（單一訊息三個工具呼叫）
2. 分析 staged changes，撰寫 commit message
3. 使用 HEREDOC 格式 git commit
4. pre-commit hook 失敗 → 重試一次
5. git status 確認

# Creating pull requests
1. git status + git diff + git log + git diff main...HEAD
2. 建立分支（如需要）
3. git commit
4. git push -u
5. gh pr create --title "..." --body "..."
```

**環境資訊注入**（coder.go）：

```go
func getEnvironmentInfo() string {
    cwd := config.WorkingDirectory()
    isGit := isGitRepo(cwd)          // 檢查 .git 目錄是否存在
    platform := runtime.GOOS
    date := time.Now().Format("1/2/2006")
    ls := tools.NewLsTool()
    r, _ := ls.Run(ctx, ToolCall{Input: `{"path":"."}`})  // 列出工作目錄
    return fmt.Sprintf(`
<env>
Working directory: %s
Is directory a git repo: %s
Platform: %s
Today's date: %s
</env>
<project>
%s
</project>`, cwd, isGit, platform, date, r.Content)
}
```

---

## 6. 上下文管理

### Context Paths 系統

```go
// config.go
var defaultContextPaths = []string{
    ".github/copilot-instructions.md",
    ".cursorrules",
    ".cursor/rules/",
    "CLAUDE.md", "CLAUDE.local.md",
    "opencode.md", "opencode.local.md",
    "OpenCode.md", "OpenCode.local.md",
}
```

專案根目錄中符合這些路徑的檔案會被自動讀取並注入到 system prompt 中。這些檔案使用 `sync.Once` 確保只讀取一次，並透過 goroutine 並行處理。

### Token 追蹤

```go
// agent.go - TrackUsage
cost := model.CostPer1MInCached/1e6*float64(usage.CacheCreationTokens) +
    model.CostPer1MOutCached/1e6*float64(usage.CacheReadTokens) +
    model.CostPer1MIn/1e6*float64(usage.InputTokens) +
    model.CostPer1MOut/1e6*float64(usage.OutputTokens)

sess.Cost += cost
sess.CompletionTokens = usage.OutputTokens + usage.CacheReadTokens
sess.PromptTokens = usage.InputTokens + usage.CacheCreationTokens
```

### 自動摘要壓縮

當 `autoCompact` 啟用（預設開啟）且 token 使用量超過 context window 的 95% 時，自動觸發：

```go
// 摘要流程
func (a *agent) Summarize(ctx, sessionID) error {
    msgs, _ := a.messages.List(ctx, sessionID)
    summarizePrompt := "Provide a detailed but concise summary..."
    response, _ := a.summarizeProvider.SendMessages(ctx, msgsWithPrompt, [])
    // 建立摘要訊息
    msg, _ := a.messages.Create(ctx, sessionID, summaryContent)
    oldSession.SummaryMessageID = msg.ID  // 標記摘要點
}

// 後續對話會從 SummaryMessageID 開始
if session.SummaryMessageID != "" {
    msgs = msgs[summaryMsgIndex:]
    msgs[0].Role = message.User  // 摘要作為新的起始訊息
}
```

### Anthropic 快取策略

```go
// 最後 3 條訊息標記 ephemeral cache
if i > len(messages)-3 {
    content.OfText.CacheControl = anthropic.CacheControlEphemeralParam{Type: "ephemeral"}
}
// system prompt 標記 ephemeral cache
System: []anthropic.TextBlockParam{{
    Text: systemMessage,
    CacheControl: anthropic.CacheControlEphemeralParam{Type: "ephemeral"},
}}
// 工具列表最後一項標記 ephemeral cache
if i == len(tools)-1 {
    toolParam.CacheControl = anthropic.CacheControlEphemeralParam{Type: "ephemeral"}
}
```

---

## 7. Session / 歷史記錄

### 資料庫

使用 **SQLite**（透過 `ncruces/go-sqlite3` WASM 綁定，無需 CGO），搭配 **sqlc** 產生型別安全的查詢代碼，**goose** 管理 migration。

### Session 模型

```go
type Session struct {
    ID               string
    ParentSessionID  string   // 子 Agent 的父 Session
    Title            string
    MessageCount     int64
    PromptTokens     int64
    CompletionTokens int64
    SummaryMessageID string   // 摘要截斷點
    Cost             float64  // 累計費用
    CreatedAt, UpdatedAt int64
}
```

### Message 模型

```go
type Message struct {
    ID, SessionID string
    Role          MessageRole    // user, assistant, tool
    Parts         []ContentPart  // 多型內容
    Model         models.ModelID
    CreatedAt, UpdatedAt int64
}

// ContentPart 介面的 7 種實作
type ContentPart interface { ... }
- TextContent       // 文字
- ReasoningContent  // 思考/推理
- BinaryContent     // 圖片等二進位
- ImageURLContent   // 圖片 URL
- ToolCall          // 工具呼叫
- ToolResult        // 工具結果
- Finish            // 完成標記（原因 + 時間戳）
```

### 檔案版本歷史

```go
type File struct {
    ID, SessionID, Path, Content, Version string
    CreatedAt, UpdatedAt int64
}
```

每次檔案修改都記錄版本（initial → v1 → v2 → ...），支援 Session 粒度的撤銷。

### PubSub 系統

```go
type Broker[T any] struct {
    subs     map[chan Event[T]]struct{}
    mu       sync.RWMutex
    done     chan struct{}
    subCount int
}

// 發布（非阻塞，滿了就丟棄）
func (b *Broker[T]) Publish(t EventType, payload T) {
    for _, sub := range subscribers {
        select {
        case sub <- event:
        default:  // 丟棄（不阻塞）
        }
    }
}
```

5 個 PubSub 頻道連接後端服務與 TUI：`logging`、`sessions`、`messages`、`permissions`、`coderAgent`。

---

## 8. 多模型支援

### 模型資料結構

```go
type Model struct {
    ID                  ModelID
    Name                string
    Provider            ModelProvider
    APIModel            string      // 實際 API 呼叫的模型名稱
    CostPer1MIn         float64
    CostPer1MOut        float64
    CostPer1MInCached   float64
    CostPer1MOutCached  float64
    ContextWindow       int64
    DefaultMaxTokens    int64
    CanReason           bool        // 支援推理/思考
    SupportsAttachments bool        // 支援圖片上傳
}
```

### 支援的 Provider 與模型數量

| Provider | 模型數 | 實作方式 |
|----------|--------|----------|
| GitHub Copilot | 14 | 自有客戶端（GitHub API） |
| Anthropic | 7 | 官方 SDK |
| OpenAI | ~10 | 官方 SDK |
| Gemini | ~5 | google.golang.org/genai |
| Groq | ~3 | OpenAI 相容端點 |
| OpenRouter | ~5 | OpenAI 相容端點 |
| xAI | ~3 | OpenAI 相容端點 |
| Azure OpenAI | ~5 | Azure SDK |
| AWS Bedrock | 1 | Anthropic SDK + Bedrock 配置 |
| VertexAI | ~3 | VertexAI SDK |
| Local | 任意 | OpenAI 相容端點 |

### 模型切換

```go
// TUI Ctrl+O 觸發
case dialog.ModelSelectedMsg:
    model, err := a.app.CoderAgent.Update(config.AgentCoder, msg.Model.ID)
    // Update 流程：
    // 1. 檢查 Agent 是否忙碌
    // 2. 更新 config 檔案
    // 3. 建立新 Provider 實例
    // 4. 替換 agent.provider

// config 持久化
func UpdateAgentModel(agentName, modelID) error {
    cfg.Agents[agentName] = Agent{Model: modelID, MaxTokens: ...}
    return updateCfgFile(func(config *Config) {
        config.Agents[agentName] = newAgentCfg
    })
}
```

### Provider 自動偵測優先序

```
1. GitHub Copilot（hosts.json / apps.json OAuth token）
2. Anthropic（ANTHROPIC_API_KEY）
3. OpenAI（OPENAI_API_KEY）
4. Gemini（GEMINI_API_KEY）
5. Groq（GROQ_API_KEY）
6. OpenRouter（OPENROUTER_API_KEY）
7. xAI（XAI_API_KEY）
8. AWS Bedrock（AWS credentials）
9. Azure OpenAI（AZURE_OPENAI_ENDPOINT）
10. VertexAI（VERTEXAI_PROJECT + VERTEXAI_LOCATION）
```

Copilot 排第一是因為許多開發者有 GitHub 訂閱但可能沒有其他 API key。

---

## 9. 值得採用的關鍵模式

### 9.1 泛型 PubSub Broker

```go
type Broker[T any] struct { ... }
```

極簡的泛型事件匯流排，clawtex-core 的 `agent_events.rs` 使用 `tokio::broadcast`，但 OpenCode 的泛型方式使每個服務自帶發布者，更解耦。

**Rust 轉譯建議**：
```rust
pub struct Broker<T: Clone + Send + 'static> {
    subscribers: Arc<RwLock<Vec<tokio::sync::mpsc::Sender<Event<T>>>>>,
}
```

### 9.2 Provider 工廠 + Options 模式

OpenAI 相容端點重用策略非常高效：Groq、OpenRouter、XAI、Local 全部共用同一個 `OpenAIClient`，僅透過 `WithOpenAIBaseURL()` 和 `WithOpenAIExtraHeaders()` 差異化。

**Rust 轉譯建議**：clawtex-core 已有 `openai_compat.rs`，可以進一步採用 builder pattern + trait object 統一。

### 9.3 Anthropic 快取策略

OpenCode 對 Anthropic API 的 prompt caching 做了精心優化：
- System prompt: ephemeral cache
- 最後 3 條訊息: ephemeral cache
- 工具列表最後一項: ephemeral cache

clawtex-core 目前未實作 Anthropic prompt caching。

### 9.4 LSP 即時診斷反饋

檔案修改 → LSP 通知 → 等待診斷 → 附加到工具回應。這形成了 **LLM 自我修正迴圈**，讓 AI 能立即看到自己的程式錯誤。

### 9.5 檔案讀取驗證

```go
if getLastReadTime(filePath).IsZero() {
    return error("you must read the file before editing it")
}
if modTime.After(lastRead) {
    return error("file has been modified since it was last read")
}
```

防止 LLM 在不了解檔案當前狀態的情況下盲目修改。

### 9.6 子 Agent 委派

`AgentTool` 啟動一個受限的子 Agent（只有唯讀工具），用於搜尋任務。這減少了主 Agent 的 context 消耗，同時避免了子 Agent 意外修改檔案。

### 9.7 權限系統的 PubSub 整合

```go
// 權限請求是阻塞的（等待 channel 回應）
func (s *permissionService) Request(opts) bool {
    respCh := make(chan bool, 1)
    s.pendingRequests.Store(permission.ID, respCh)
    s.Publish(pubsub.CreatedEvent, permission)  // TUI 顯示對話框
    return <-respCh                              // 等待用戶回應
}
```

工具執行在 goroutine 中阻塞，TUI 收到事件後顯示權限對話框，用戶操作後透過 channel 解鎖。

### 9.8 持久化 Shell Session

bash 工具使用持久化 shell（`shell/shell.go`），環境變數和工作目錄跨命令保留。clawtex-core 的 `shell` 工具每次都是新進程。

### 9.9 自動壓縮（Auto Compact）

context window 使用量達 95% 時自動摘要壓縮，而非丟棄早期訊息。clawtex-core 的 `context_compactor.rs` 有類似功能（Light/Medium/Aggressive），但未與 token 追蹤自動觸發。

### 9.10 SQLite + sqlc + goose 組合

型別安全的查詢（sqlc 產生）+ 嵌入式 migration（goose）+ 零 CGO（go-sqlite3 WASM）。三者搭配使資料層極度可靠。

---

## 10. 與 clawtex-core 的比較

### OpenCode 做得更好的方面

| 面向 | OpenCode | clawtex-core |
|------|----------|-------------|
| **LSP 整合** | 原生支援，工具修改後自動提供診斷反饋 | 無 LSP 整合 |
| **Anthropic 快取** | 精心的 ephemeral cache 策略（system prompt + 最後 3 訊息 + 工具） | 未實作 prompt caching |
| **檔案安全** | 讀取驗證 + 修改時間檢查 + 唯一性驗證 | 基本的 file_edit（無讀取前置驗證） |
| **TUI 體驗** | 精美的 Bubble Tea TUI（10 種主題、命令面板、模型切換） | 無 TUI（Telegram Bot 介面） |
| **Sub-Agent** | 受限的子 Agent（唯讀工具），自動成本累計 | delegate 工具，但子 Agent 有完整工具存取 |
| **Copilot 支援** | 原生支援 GitHub Copilot（免費模型，14 種） | 無 |
| **持久化 Shell** | Shell session 跨命令保留狀態 | 每次新進程 |
| **檔案版本歷史** | 每次修改記錄版本到 SQLite | 無版本歷史 |
| **自動壓縮觸發** | Token 用量 95% 自動觸發摘要 | 手動或需要自行整合 |
| **非互動模式** | `opencode -p "..." -f json` 支援 CI/CD | 無 CLI 非互動模式 |
| **MCP 支援** | 完整支援（stdio + SSE），使用 mcp-go 函式庫 | 自實作 JSON-RPC 2.0 stdio |
| **代碼搜尋** | Sourcegraph 整合 | 無 |

### clawtex-core 做得更好的方面

| 面向 | clawtex-core | OpenCode |
|------|-------------|----------|
| **工具數量** | 24 個工具 | 12 個工具 |
| **Provider 數量** | 14 個 provider 實作 | 8 個獨立實作 |
| **分散式** | ClusterHub/Worker 分散式架構 | 單機 |
| **工作流引擎** | Hands 引擎（10+ 工作流） | 無工作流 |
| **收入管道** | 完整的營利自動化（10 條路線） | 純開發工具 |
| **安全性** | ChaCha20-Poly1305 加密 + 憑證清洗 + E-Stop | 無加密，僅權限閘門 |
| **排程** | Cron 排程系統 | 無 |
| **社群工具** | Twitter、Blog、Email 自動化 | 無 |
| **API 閘道** | SSE + WebSocket + REST API | 無 HTTP API |
| **多 Agent** | 多 Agent 路由 + 智慧分類 | 僅 Coder + Task 兩層 |
| **SoT 引擎** | Skeleton-of-Thought 並行生成 | 無 |
| **NPU 支援** | AYANEO NPU 連接 | 無 |
| **測試** | 707+ 測試 | 約 5 個測試檔案 |
| **迴圈偵測** | GenericRepeat + PingPong + StaleResult | 無 |
| **回應快取** | LRU cache with TTL | 無 |
| **Hub Auth** | Bearer Token + auto-generated UUID | 無 |

### 兩者都缺少的

- **RAG / 向量搜尋**：兩者都未整合 embedding-based 檢索
- **多語言 Agent**：OpenCode 的 prompt 只有英文；clawtex-core 也是
- **Plugin 系統**：兩者都無正式的 plugin 架構（OpenCode 靠 MCP，clawtex-core 靠 Hands）
- **視覺化 debug**：兩者都未提供 step-by-step agent 執行追蹤 UI

### 建議從 OpenCode 移植到 clawtex-core 的功能

1. **LSP 整合**：為 `file_edit` 和 `file_write` 工具添加 LSP 診斷反饋，形成自我修正迴圈
2. **Anthropic Prompt Caching**：在 `anthropic.rs` 中實作 ephemeral cache 策略
3. **檔案讀取前置驗證**：`file_edit` 工具在修改前要求先讀取檔案
4. **持久化 Shell**：改進 `shell` 工具以保留跨命令狀態
5. **自動壓縮觸發**：將 `context_compactor.rs` 與 token 追蹤整合，自動觸發壓縮
6. **子 Agent 工具限制**：`delegate` 的子 Agent 應使用受限的工具集
7. **檔案版本歷史**：每次 `file_edit`/`file_write` 記錄版本，支援 session 粒度撤銷
8. **非互動 CLI 模式**：添加 `clawtex-core run -p "prompt" -f json` 支援
9. **Context Paths**：自動讀取 `CLAUDE.md`、`.cursorrules` 等專案指示檔

---

## 附錄：關鍵依賴版本

| 依賴 | 版本 | 用途 |
|------|------|------|
| `anthropics/anthropic-sdk-go` | v1.4.0 | Anthropic API |
| `openai/openai-go` | v0.1.0-beta.2 | OpenAI API |
| `google.golang.org/genai` | v1.3.0 | Gemini API |
| `mark3labs/mcp-go` | v0.17.0 | MCP 協定 |
| `charmbracelet/bubbletea` | v1.3.5 | TUI 框架 |
| `charmbracelet/lipgloss` | v1.1.0 | 終端樣式 |
| `ncruces/go-sqlite3` | v0.25.0 | SQLite WASM |
| `pressly/goose` | v3.24.2 | DB Migration |
| `spf13/cobra` | v1.9.1 | CLI 框架 |
| `spf13/viper` | v1.20.0 | 設定管理 |
| `fsnotify/fsnotify` | v1.8.0 | 檔案監視 |
| `aws/aws-sdk-go-v2` | v1.30.3 | AWS Bedrock |
| `Azure/azure-sdk-for-go` | v1.7.0 | Azure OpenAI |
