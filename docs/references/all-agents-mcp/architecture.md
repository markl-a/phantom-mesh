# All-Agents-MCP 架構文檔

## 1. 專案概覽

**All-Agents-MCP** 是一個 stdio 型 MCP 伺服器，透過統一介面編排多個 AI CLI 代理 (Claude Code、Codex、Gemini CLI、Copilot CLI)。採用進程生成而非 OAuth 令牌劫持，完全相容 ToS 且無帳戶風險。支持 14 個工具、3 個資源、複雜度自動分類、並聯執行、遞迴調用防護，適合企業團隊無需污染共享 `CLAUDE.md` 的多代理工作流。

**核心優勢：**
- **進程編排，非令牌盜竊：** 呼叫官方 CLI 二進制檔，無 OAuth 令牌提取
- **ToS 相容：** 完全遵守各平臺條款，零帳戶風險
- **模塊化 MCP：** 企業共享 CLAUDE.md 不受污染
- **遞迴調用防護：** 自動排除呼叫者代理 (無限迴圈)
- **複雜度路由：** 自動選擇單代理或多代理編排
- **會話持久化：** 檔案型會話儲存，完整歷史追蹤

## 2. 目錄結構

```
all-agents-mcp/
├── src/                        # [P0] TypeScript 原始碼
│   ├── index.ts                # [P0] 進入點 (stdio transport)
│   ├── server.ts               # [P0] McpServer 工廠
│   │
│   ├── agents/                 # [P0] 代理抽象層
│   │   ├── types.ts            # Agent 型別定義
│   │   │   ├─ AgentId union (claude|codex|gemini|copilot)
│   │   │   ├─ IAgent 介面
│   │   │   ├─ AgentResponse, HealthStatus
│   │   │   └─ ExecutionOptions (timeout, models)
│   │   ├─ base-agent.ts        # 抽象基類
│   │   │   ├─ buildSpawnOptions() → CLI args
│   │   │   ├─ execute(input) → AgentResponse
│   │   │   ├─ parseOutput(raw) → string
│   │   │   └─ health() → HealthStatus
│   │   ├─ claude-agent.ts      # Claude 特定覆蓋
│   │   │   ├─ JSON 輸出解析
│   │   │   └─ Claude Code 特有參數
│   │   ├─ codex-agent.ts       # Codex/ChatGPT
│   │   │   ├─ temp 檔案 --output-last-message
│   │   │   └─ OpenAI API 應答
│   │   ├─ gemini-agent.ts      # Gemini CLI
│   │   │   └─ 環境變數模型選擇
│   │   ├─ copilot-agent.ts     # GitHub Copilot
│   │   └─ registry.ts          # 延遲初始化單例
│   │       ├─ agentFactories map
│   │       ├─ getAgent(id) → lazy instantiate
│   │       ├─ detectCaller() → env vars
│   │       └─ excludeCallerFromRegistry() [遞迴防護]
│   │
│   ├── orchestrator/           # [P1] 編排層
│   │   ├─ executor.ts          # 低階進程生成
│   │   │   ├─ spawnAgent(agent, input, options)
│   │   │   ├─ AbortController 超時
│   │   │   └─ child_process.spawn() + stdio 捕捉
│   │   ├─ parallel.ts          # 並聯執行
│   │   │   ├─ executeParallel(agents, input)
│   │   │   └─ Promise.allSettled() + aggregation
│   │   ├─ complexity.ts        # 複雜度評分
│   │   │   ├─ classifyTask(input) → simple|complex|large
│   │   │   ├─ 韓文 + 英文關鍵字啟發式
│   │   │   └─ token 基線 (500, 2000)
│   │   ├─ verifier.ts          # 跨模型驗證
│   │   │   ├─ verifySolution(agent, models)
│   │   │   └─ 並聯單代理多模型
│   │   └─ aggregator.ts        # 結果聚集
│   │       ├─ formatComparison() → Markdown 表格
│   │       ├─ formatVerification() → 驗證摘要
│   │       └─ formatDelegation() → 決策邏輯
│   │
│   ├── tools/                  # [P1] 14 個工具定義
│   │   ├─ ask-agent.ts         # 單代理查詢
│   │   ├─ ask-all.ts           # 多代理並聯
│   │   ├─ delegate.ts          # 智能路由
│   │   ├─ collaborate.ts       # 協作工作流
│   │   ├─ debug-with.ts        # 調試
│   │   ├─ explain-with.ts      # 解釋
│   │   ├─ refactor-with.ts     # 重構
│   │   ├─ review-code.ts       # 代碼審查
│   │   ├─ generate-test.ts     # 測試生成
│   │   ├─ fetch-page.ts        # Web 爬蟲 (Cheerio)
│   │   ├─ agent-health.ts      # 健康檢查
│   │   ├─ list-agents.ts       # 可用代理列表
│   │   ├─ list-models.ts       # 可用模型列表
│   │   └─ deploy.ts            # 部署協助
│   │
│   ├── resources/              # [P1] 3 個資源定義
│   │   ├─ agent-status.ts      # 當前代理狀態
│   │   ├─ session-history.ts   # 會話歷史記錄
│   │   └─ sessions-list.ts     # 所有會話列表
│   │
│   ├── session/                # [P1] 會話管理
│   │   ├─ store.ts             # 檔案型 JSON 儲存
│   │   │   ├─ ~.all-agents-mcp/sessions/
│   │   │   ├─ readSession(sessionId)
│   │   │   └─ appendEntry(sessionId, entry)
│   │   └─ types.ts             # SessionEntry, SessionStore
│   │
│   ├── config/                 # [P1] 配置管理
│   │   ├─ loader.ts            # 環境變數覆蓋
│   │   │   ├─ AA_MCP_LOG_LEVEL
│   │   │   ├─ AA_MCP_<AGENT>_TIMEOUT
│   │   │   ├─ AA_MCP_<AGENT>_MODELS
│   │   │   └─ Zod 驗證
│   │   ├─ models.json          # 模型定義 (name, timeoutSeconds)
│   │   └─ schema.ts            # Zod 型別定義
│   │
│   ├── utils/                  # [P2] 工具函式
│   │   ├─ logger.ts            # stderr 日誌 (MCP stdout 安全)
│   │   ├─ detect.ts            # 呼叫者偵測
│   │   │   ├─ detectCaller() → env vars
│   │   │   ├─ CALLER_ENV_MAP (各代理)
│   │   │   └─ process.env._ (進程路徑)
│   │   └─ which.ts             # CLI 二進制檢查
│   │
│   └── index.ts                # 導出主模組
│
├── dist/                       # [P2] 編譯輸出 (tsc)
├── test/                       # [P2] Vitest 測試
│   ├─ orchestrator/            # 編排測試
│   │   ├─ complexity.test.ts
│   │   ├─ aggregator.test.ts
│   │   └─ executor.test.ts
│   ├─ config/
│   │   └─ loader.test.ts
│   └─ tools/
│       ├─ fetch-page.test.ts
│       └─ ask-all.test.ts
│
├── package.json                # [P0] npm 專案定義
│   ├─ "type": "module" (ESM only)
│   ├─ dependencies: @modelcontextprotocol/sdk, zod, which
│   ├─ scripts: build, dev, lint, test
│   └─ exports
│
├── tsconfig.json               # TypeScript 配置 (Node16 module resolution)
├── biome.json                  # Biome 格式化 (Tab, 100 chars, double-quote)
├── vitest.config.ts            # 測試框架配置
│
├── .mcp.json                   # [P2] MCP 伺服器描述符
├── CLAUDE.md                   # Claude Code 指示 (AIDE v1.0 參考)
├── AIDE-REFERENCE.md           # AIDE 方法論 (v1.0)
├── CONTRIBUTING.md             # 貢獻指南
├── README.md                   # 快速開始
└── LICENSE                     # MIT 授權
```

## 3. 核心模組詳解

### 3.1 Agent 抽象層

```typescript
// AgentId 聯合型
type AgentId = "claude" | "codex" | "gemini" | "copilot"

// IAgent 介面
interface IAgent {
  agentId: AgentId
  health(): Promise<HealthStatus>
  execute(input: string, options?: ExecutionOptions): Promise<AgentResponse>
  buildSpawnOptions(): SpawnOptions
  parseOutput(raw: string): string
  defaultTimeoutSeconds: number
}

// 實現層次結構
BaseAgent (抽象)
  ├─ buildSpawnOptions() // CLI 特定
  ├─ execute() // 通用流程
  └─ parseOutput() // 提示符解析

ClaudeAgent
  ├─ parseOutput() 覆蓋 → JSON 格式
  └─ 特有參數: --with-tool

CodexAgent
  ├─ execute() 完全覆蓋 → temp 檔案模式
  └─ --output-last-message → response.json
```

### 3.2 遞迴調用防護

```typescript
// Registry: 呼叫者自動排除
const registry = getAgentRegistry()

// 偵測呼叫者
const caller = detectCaller()  // 檢查 process.env._
  → /path/to/claude → AgentId = "claude"
  → /path/to/codex  → AgentId = "codex"
  → (default)       → AgentId = undefined

// 返回可用代理 (排除呼叫者)
getAvailableAgents()
  ├─ 如果 caller === "claude"
  │   └─ 返回 [codex, gemini, copilot]  // 排除 claude
  └─ 否則
      └─ 返回所有已安裝的代理

// 效果
ask_all_agents(prompt)
  ├─ 在 Claude 中呼叫
  ├─ 檢測 caller = "claude"
  ├─ 自動排除 Claude (防無限迴圈)
  └─ 執行 [codex, gemini, copilot]
```

### 3.3 複雜度分類

```typescript
// classifyTask(input: string) → Complexity
// 啟發式: 關鍵字 + token 基線

韓文關鍵字 (예): 분석, 검토, 설계, ...
英文關鍵字: analyze, review, design, architect, implement, ...

評分邏輯:
  ├─ 3+ 關鍵字且長度 > 2000 → "large"
  ├─ 1+ 關鍵字且長度 > 500 → "complex"
  └─ 否則 → "simple"

路由決策:
  ├─ simple  → ask_agent(claude, prompt)
  ├─ complex → ask_all([claude, codex], prompt)
  └─ large   → ask_all([claude, codex, gemini], prompt)
```

### 3.4 並聯執行

```typescript
// executeParallel(agents, input) → Promise<Result[]>

Promise.allSettled([
  agent_a.execute(input, {timeout: 30s}),
  agent_b.execute(input, {timeout: 30s}),
  agent_c.execute(input, {timeout: 30s}),
])
  ↓
  ├─ fulfilled: AgentResponse
  ├─ rejected: Error (timeout or crash)
  └─ 聚集為 Markdown 比較表

格式化結果:
  | Agent  | Status | Output (摘要) |
  |--------|--------|--------------|
  | 🔴 Codex  | ✅     | ...          |
  | 🟡 Gemini | ✅     | ...          |
  | 🔵 Claude | ✅     | ...          |

超時管理:
  ├─ 全局預設: 120_000ms (2 分鐘)
  ├─ 模型覆蓋: models.json[model].timeoutSeconds
  ├─ 呼叫覆蓋: options.timeout (highest priority)
  └─ 環境覆蓋: AA_MCP_CLAUDE_TIMEOUT=60 (秒)
```

## 4. 啟動流程

### 4.1 MCP 伺服器初始化

```
Claude Code 或其他宿主
    ↓
mcp add all-agents-mcp -- npx -y all-agents-mcp
    ↓
npm install (自動)
    ↓
npm run build (tsc → dist/)
    ↓
node dist/index.js
    ├─ 載入 config/models.json
    ├─ 初始化 logger (stderr)
    ├─ 建立 McpServer 實例
    ├─ 註冊 14 個工具
    │   ├─ tool("ask_agent", ...)
    │   ├─ tool("ask_all", ...)
    │   ├─ tool("delegate", ...)
    │   └─ ... (共 14)
    ├─ 註冊 3 個資源
    │   ├─ resource("agent-status")
    │   ├─ resource("session-history")
    │   └─ resource("sessions-list")
    │
    └─ stdio transport 連接
        ├─ 監聽 stdin (JSON-RPC 請求)
        ├─ 寫入 stdout (JSON-RPC 回應)
        └─ 嚴禁任何非 MCP 輸出到 stdout
```

### 4.2 工具調用流程 (ask_all)

```
Claude: "Compare authentication approaches"
    ↓
MCP: tool_call("ask_all", {prompt: "..."})
    ↓
all-agents-mcp server
    ├─ 複雜度評分: "compare" + "approaches" → "complex"
    ├─ 自動路由: ask_all([claude, codex, gemini])
    │
    ├─ 並聯執行 (Promise.allSettled)
    │   ├─ spawnAgent(codex, prompt, {timeout: 30s})
    │   │   ├─ buildSpawnOptions() → [codex, --model, gpt-5.3, exec, prompt]
    │   │   ├─ child_process.spawn()
    │   │   ├─ 捕捉 stdout/stderr
    │   │   ├─ parseOutput() → markdown
    │   │   └─ 超時: AbortController
    │   │
    │   ├─ spawnAgent(gemini, prompt, {...})
    │   │
    │   └─ spawnAgent(claude, prompt, {...})
    │       └─ parseOutput() 特有: 處理 JSON
    │
    ├─ 聚集結果
    │   ├─ formatComparison()
    │   └─ 建立 Markdown 表格
    │
    ├─ 記錄會話
    │   └─ appendEntry(sessionId, {
    │       tool: "ask_all",
    │       input: prompt,
    │       agents: [codex, gemini, claude],
    │       results: [...],
    │       timestamp: now()
    │     })
    │
    └─ 返回結果
        ├─ Tool result: "| Agent | Output |\n..."
        └─ Claude 讀取 + 綜合
```

## 5. 資料流 ASCII 圖

### 5.1 多代理編排流程

```
宿主 (Claude Code)
    │
    ├─ "Design a REST API"
    │
    └─ MCP: ask_all({prompt: "..."})
        ↓
    all-agents-mcp server
        ├─ 複雜度: large
        ├─ 路由: [claude, codex, gemini]
        │
        ├─ 並聯發射:
        │   ├─ 🔵 Claude
        │   │   ├─ spawn claude
        │   │   ├─ REST API 最佳實踐
        │   │   └─ 超時: 120s
        │   │
        │   ├─ 🔴 Codex
        │   │   ├─ spawn codex --model gpt-5.3
        │   │   ├─ API 實現代碼
        │   │   └─ 超時: 30s
        │   │
        │   └─ 🟡 Gemini
        │       ├─ spawn gemini
        │       ├─ 替代設計方法
        │       └─ 超時: 30s
        │
        ├─ 聚集:
        │   ├─ Markdown 表格
        │   ├─ 決策矩陣
        │   └─ 推薦選項
        │
        └─ 返回 → Claude
            └─ 顯示結果
```

### 5.2 會話追蹤流程

```
~/.all-agents-mcp/sessions/
├─ [session-id]/
│   ├─ session.json (metadata)
│   └─ entries.jsonl (append-only)
│       ├─ {"tool": "ask_agent", "agents": ["claude"], ...}
│       ├─ {"tool": "ask_all", "agents": ["claude", "codex"], ...}
│       └─ (每次工具呼叫附加一行)

资源查询:
  ├─ resource("session-history", {sessionId})
  │   └─ 讀取 entries.jsonl → 完整歷史
  │
  └─ resource("sessions-list")
      └─ 列出所有 session-id (日期排序)
```

## 6. 子系統清單

### 6.1 P0 優先級 (核心)

| 子系統 | 功能 | 檔案 | 狀態 |
|--------|------|------|------|
| 進入點 | stdio 傳輸 | `index.ts` | ✅ |
| McpServer | 工具 + 資源註冊 | `server.ts` | ✅ |
| BaseAgent | 代理抽象 | `agents/base-agent.ts` | ✅ |
| Registry | 代理工廠 + 遞迴防護 | `agents/registry.ts` | ✅ |
| Executor | 進程生成 + 超時 | `orchestrator/executor.ts` | ✅ |
| ask_agent | 單代理工具 | `tools/ask-agent.ts` | ✅ |
| ask_all | 多代理工具 | `tools/ask-all.ts` | ✅ |

### 6.2 P1 優先級 (重要功能)

| 子系統 | 功能 | 檔案 | 狀態 |
|--------|------|------|------|
| Claude Agent | JSON 輸出解析 | `agents/claude-agent.ts` | ✅ |
| Codex Agent | temp 檔案模式 | `agents/codex-agent.ts` | ✅ |
| Gemini Agent | 環境變數模型 | `agents/gemini-agent.ts` | ✅ |
| Copilot Agent | GitHub CLI | `agents/copilot-agent.ts` | ✅ |
| Parallel | 並聯執行 | `orchestrator/parallel.ts` | ✅ |
| Complexity | 自動分類 | `orchestrator/complexity.ts` | ✅ |
| Aggregator | 結果聚集 | `orchestrator/aggregator.ts` | ✅ |
| Verifier | 跨模型驗證 | `orchestrator/verifier.ts` | ✅ |
| Session Store | 檔案持久化 | `session/store.ts` | ✅ |
| Config Loader | 環境變數覆蓋 | `config/loader.ts` | ✅ |
| 12 Tools | delegate, debug, explain, refactor, review, ... | `tools/` | ✅ |

### 6.3 P2 優先級 (增強功能)

| 子系統 | 功能 | 檔案 | 狀態 |
|--------|------|------|------|
| 3 Resources | session-history, sessions-list, agent-status | `resources/` | ✅ |
| Logger | stderr 日誌 (stdout 安全) | `utils/logger.ts` | ✅ |
| Caller Detection | 環境推理 | `utils/detect.ts` | ✅ |
| CLI Checker | which 檢測 | `utils/which.ts` | ✅ |
| Unit Tests | Vitest 測試套件 | `test/` | ✅ |
| Biome Lint | 自動格式化 | biome.json | ✅ |

## 7. 安全性 & 合規性

### 7.1 進程編排 (非 OAuth 盜竊)

```
正確方式 (all-agents-mcp):
  └─ 呼叫 /usr/bin/claude 或 /opt/codex
     ├─ 使用 official CLI
     ├─ 遵守各平臺 ToS
     └─ 零帳戶風險

不安全方式 (OpenCode 等):
  └─ 從 ~/.claude/ 或瀏覽器提取 OAuth 令牌
     ├─ 違反 ToS
     ├─ 帳戶風險 (禁用、封鎖)
     └─ 計費不可預測
```

### 7.2 遞迴調用防護

```typescript
// 場景: Claude 中 ask_all → 引用 Claude
ask_all("compare approaches")
  ├─ detectCaller() → "claude"
  ├─ getAvailableAgents()
  │   └─ 自動排除 claude (因為是呼叫者)
  └─ 返回 [codex, gemini, copilot]

// 結果: 無限迴圈防止
```

## 8. 技術棧

- **Language:** TypeScript (Node.js 22+)
- **Module System:** ESM (import/export)
- **MCP SDK:** @modelcontextprotocol/sdk v1.0+
- **Validation:** Zod (runtime schema)
- **Testing:** Vitest
- **Linter:** Biome (Tab, 100 chars, double-quote)
- **Process:** child_process.spawn + AbortController

## 9. 關鍵設計決策

1. **進程編排而非令牌盜竊：** 唯一的安全方式
2. **stderr 日誌而非 stdout：** MCP stdout 完整性
3. **遞迴防護自動化：** 無需使用者干預
4. **複雜度自動路由：** 無需手動選擇代理
5. **會話檔案持久化：** 完整審計跟蹤
6. **ESM + 嚴格模式：** 類型安全 + 模組隔離

## 10. 開發工作流

```bash
# 安裝依賴
npm install

# 開發模式
npm run dev  # tsc --watch

# 編譯
npm run build

# 測試
npm test

# 執行
npx all-agents-mcp
```

## 11. 環境變數

```bash
# 日誌級別
AA_MCP_LOG_LEVEL=debug|info|warn|error

# 代理特定超時 (秒)
AA_MCP_CLAUDE_TIMEOUT=120
AA_MCP_CODEX_TIMEOUT=60
AA_MCP_GEMINI_TIMEOUT=45

# 模型覆蓋
AA_MCP_CLAUDE_MODELS=claude-opus-4.6,claude-sonnet-3.5
AA_MCP_CODEX_MODELS=gpt-5.3-codex,gpt-4
```

## 12. 版本資訊

- **當前版本:** 1.2.2
- **發佈日期:** 2025-03-13
- **新功能:** 遞迴防護、複雜度自動分類、14 工具、3 資源
- **相容性:** Claude Code、Codex CLI、Gemini CLI、Copilot CLI
