# All Agents MCP -- 深度技術分析

> 作者: Dokkabei97
> 版本: v1.2.2
> 授權: MIT
> 來源: `LLM-Cluster-Project/references/all-agents-mcp/`
> 分析日期: 2026-03-12

---

## 目錄

1. [專案結構](#1-專案結構)
2. [進入點](#2-進入點)
3. [核心架構](#3-核心架構)
4. [Email 整合](#4-email-整合)
5. [協定細節](#5-協定細節)
6. [值得採用的模式](#6-值得採用的模式)
7. [與 clawtex-core 的關聯性](#7-與-clawtex-core-的關聯性)

---

## 1. 專案結構

### 語言與技術棧

- **語言**: TypeScript (嚴格模式, ES2022 target)
- **執行環境**: Node.js >= 22 (ESM-only, `"type": "module"`)
- **核心依賴**: 僅 3 個
  - `@modelcontextprotocol/sdk` ^1.12.1 -- MCP 伺服器 SDK
  - `which` ^5.0.0 -- CLI 二進制檔偵測
  - `zod` ^3.24.4 -- Schema 驗證
- **開發工具**: Biome (lint/format), Vitest (測試), TypeScript 5.8

### 目錄樹

```
all-agents-mcp/
├── src/
│   ├── index.ts              # 進入點 -- stdio transport 連接
│   ├── server.ts             # McpServer 工廠 -- 註冊所有 tools 與 resources
│   ├── agents/               # Agent 抽象層
│   │   ├── types.ts          #   IAgent 介面, AgentResponse, HealthStatus, AgentConfig
│   │   ├── base-agent.ts     #   抽象基底類別, spawn 邏輯
│   │   ├── claude-agent.ts   #   Claude Code 實作
│   │   ├── codex-agent.ts    #   Codex 實作 (override execute)
│   │   ├── gemini-agent.ts   #   Gemini CLI 實作
│   │   ├── copilot-agent.ts  #   Copilot CLI 實作
│   │   └── registry.ts       #   Lazy 單例 Registry + 遞迴呼叫防護
│   ├── tools/                # 14 個 MCP tool 定義 (一檔一 tool)
│   │   ├── ask-agent.ts      #   ask_agent -- 問特定 agent
│   │   ├── ask-all.ts        #   ask_all -- 平行問所有 agent
│   │   ├── delegate.ts       #   delegate_task -- 複雜度分析 + 路由
│   │   ├── collaborate.ts    #   collaborate -- 協作分析
│   │   ├── verify.ts         #   verify -- 跨模型驗證
│   │   ├── review-code.ts    #   review_code -- 程式碼審查
│   │   ├── debug-with.ts     #   debug_with -- 除錯
│   │   ├── explain-with.ts   #   explain_with -- 程式碼解說
│   │   ├── generate-test.ts  #   generate_test -- 測試產生
│   │   ├── refactor-with.ts  #   refactor_with -- 重構
│   │   ├── fetch-page.ts     #   fetch_page -- 網頁擷取 (Gemini CLI)
│   │   ├── list-agents.ts    #   list_agents -- 列出 agent
│   │   ├── list-models.ts    #   list_models -- 列出模型
│   │   └── agent-health.ts   #   agent_health -- 健康檢查
│   ├── orchestrator/         # 編排層
│   │   ├── executor.ts       #   spawnAgent() -- child_process.spawn 封裝
│   │   ├── parallel.ts       #   executeParallel() -- Promise.allSettled
│   │   ├── complexity.ts     #   analyzeComplexity() -- 關鍵字啟發式評分
│   │   ├── verifier.ts       #   crossVerify() -- 跨模型驗證
│   │   └── aggregator.ts     #   Markdown 格式化輸出
│   ├── resources/            # 3 個 MCP resource 定義
│   │   ├── sessions-list.ts  #   aa://sessions
│   │   ├── session-history.ts#   aa://session/{id}/history
│   │   └── agent-status.ts   #   aa://agents/status
│   ├── session/              # 檔案式 JSON session 儲存
│   │   ├── types.ts          #   Session, SessionEntry 型別
│   │   └── store.ts          #   CRUD + activeSessionId 管理
│   ├── config/               # 組態
│   │   ├── schema.ts         #   Zod schema 定義
│   │   ├── models.json       #   預設模型組態 (4 agent, 各含多模型)
│   │   └── loader.ts         #   環境變數覆蓋 + 快取載入
│   └── utils/                # 工具
│       ├── logger.ts         #   stderr-only 日誌 (保護 stdout MCP 通道)
│       └── detect.ts         #   CLI 偵測 + 呼叫者辨識
├── skills/                   # 8 個 Claude Plugin skill 定義
├── commands/                 # Claude Plugin 命令
├── hooks/                    # SessionStart hook (setup 檢查)
├── scripts/                  # check-setup.sh
├── .claude-plugin/           # Claude Plugin 元資料
├── .mcp.json                 # MCP 伺服器組態範本
└── .github/workflows/        # CI + npm publish
```

### 程式碼規模

- **原始碼檔案**: ~30 個 `.ts` 檔
- **測試檔案**: 5 個 (`*.test.ts`, 與原始碼並置)
- **外部依賴**: 3 個 (runtime), 4 個 (dev)
- **架構風格**: 極簡、模組化, 每檔 < 300 行

---

## 2. 進入點

### 啟動流程

**檔案**: `src/index.ts`

```typescript
#!/usr/bin/env node
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { createServer } from "./server.js";
import { logger } from "./utils/logger.js";

async function main(): Promise<void> {
    logger.info("Starting All-AGENTS-MCP server...");
    const server = createServer();
    const transport = new StdioServerTransport();
    await server.connect(transport);
    logger.info("ALL-AGENTS-MCP server connected via stdio transport");
}

main().catch((err) => {
    logger.error("Fatal error:", err);
    process.exit(1);
});
```

**關鍵設計**:

1. `createServer()` 建立 `McpServer` 實例並註冊所有 tools/resources
2. `StdioServerTransport` -- 使用 stdin/stdout 作為 JSON-RPC 傳輸層
3. `server.connect(transport)` -- 啟動 MCP 協定握手
4. 所有日誌寫入 **stderr** (絕不碰 stdout), 這是 MCP stdio 協定的硬性要求

### 伺服器工廠

**檔案**: `src/server.ts`

```typescript
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";

export function createServer(): McpServer {
    const server = new McpServer({
        name: "all-agents-mcp",
        version: "1.0.0",
    });

    // 註冊 14 個 tools
    registerAskAgentTool(server);
    registerAskAllTool(server);
    registerDelegateTaskTool(server);
    // ... 共 14 個

    // 註冊 3 個 resources
    registerSessionsListResource(server);
    registerSessionHistoryResource(server);
    registerAgentStatusResource(server);

    return server;
}
```

**模式**: 每個 tool/resource 都是一個獨立檔案, 匯出 `register*Tool(server)` 函數。server.ts 只做匯入 + 呼叫, 職責清晰。

### 安裝方式

```bash
# Claude Code 中使用
claude mcp add all-agents-mcp -- npx -y all-agents-mcp

# Codex 中使用
codex mcp add all-agents-mcp -- npx -y all-agents-mcp

# Gemini CLI 中使用
gemini mcp add all-agents-mcp npx -y all-agents-mcp
```

以 `npx -y all-agents-mcp` 啟動, npm 套件直接執行 `dist/index.js`。

---

## 3. 核心架構

### 3.1 MCP Server 實作

All Agents MCP 使用 `@modelcontextprotocol/sdk` 官方 SDK, 核心只需三行:

```typescript
const server = new McpServer({ name: "all-agents-mcp", version: "1.0.0" });
const transport = new StdioServerTransport();
await server.connect(transport);
```

SDK 完全抽象了 JSON-RPC 2.0 協定:
- **Tool 註冊**: `server.tool(name, description, zodSchema, handler)`
- **Resource 註冊**: `server.resource(name, uri, handler)` 或 `server.resource(name, ResourceTemplate, handler)`
- **傳輸層**: `StdioServerTransport` 處理 stdin/stdout 的 JSON-RPC 訊息序列化/反序列化

### 3.2 Tool 提供的功能

14 個 tools 分為四類:

#### 核心 Tools (4 個)

| Tool | 說明 |
|------|------|
| `ask_agent` | 問特定 agent (claude/codex/gemini/copilot), 可指定模型 |
| `ask_all` | 平行問所有可用 agent, 回傳比較結果 |
| `delegate_task` | 自動分析任務複雜度, 路由至單一或多 agent 平行執行 |
| `collaborate` | 與指定 agent 協作, 取得回應 + 綜合分析指引 |

#### 驗證 Tool (1 個)

| Tool | 說明 |
|------|------|
| `verify` | 跨模型驗證 -- 同一 agent 用不同模型跑同一 prompt, 比對一致性 |

#### 專業 Tools (5 個)

| Tool | 說明 |
|------|------|
| `review_code` | 程式碼審查 (bugs/security/performance/clarity) |
| `debug_with` | 除錯分析 (根因分析 + 修復步驟 + 預防建議) |
| `explain_with` | 程式碼解說 (brief/detailed 層級) |
| `generate_test` | 測試產生 (jest/vitest/pytest/kotest) |
| `refactor_with` | 重構建議 (performance/readability/modularity) |

#### 資訊 Tools (3 個)

| Tool | 說明 |
|------|------|
| `list_agents` | 列出所有已偵測的 agent 及可用狀態 |
| `list_models` | 列出各 agent 可用模型 |
| `agent_health` | 健康檢查 (可用性、認證狀態、延遲) |

#### Web Tool (1 個)

| Tool | 說明 |
|------|------|
| `fetch_page` | 透過 Gemini CLI 原生瀏覽能力擷取網頁內容 |

### 3.3 Tool 註冊模式 (核心模式)

每個 tool 遵循相同模式。以 `ask_agent` 為例:

**檔案**: `src/tools/ask-agent.ts`

```typescript
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { getAgent } from "../agents/registry.js";
import { formatSingleResponse } from "../orchestrator/aggregator.js";
import { addEntry, getOrCreateActiveSession } from "../session/store.js";

const SCHEMA = {
    agent: z.enum(["claude", "codex", "gemini", "copilot"]).describe("Target agent to query"),
    prompt: z.string().describe("The question or prompt to send"),
    model: z.string().optional().describe("Specific model to use"),
    context: z.string().optional().describe("Additional context to pipe via stdin"),
    timeout: z.number().optional().describe("Timeout in milliseconds"),
    analysisLevel: z.enum(["low", "medium", "high", "xhigh"]).optional()
        .describe("Analysis depth level (codex only)"),
};

export function registerAskAgentTool(server: McpServer): void {
    server.tool(
        "ask_agent",                    // tool 名稱
        "Ask a specific AI agent...",   // 描述
        SCHEMA,                         // Zod schema -> 自動轉為 JSON Schema
        async (params) => {             // handler
            const agent = getAgent(params.agent);
            if (!agent) {
                return {
                    content: [{ type: "text" as const, text: `Agent '${params.agent}' is not available.` }],
                };
            }

            const response = await agent.execute({ /* ... */ });

            // 記錄到 session
            const sessionId = getOrCreateActiveSession();
            addEntry(sessionId, { tool: "ask_agent", agent: response.agent, /* ... */ });

            return {
                content: [{ type: "text" as const, text: formatSingleResponse(response) }],
            };
        },
    );
}
```

**關鍵觀察**:
1. **Zod schema 直接傳入** -- SDK 會自動將 Zod schema 轉換為 JSON Schema, 用於 MCP `tools/list` 回應
2. **回傳格式**: `{ content: [{ type: "text", text: "..." }] }` -- MCP 標準 tool result 格式
3. **session 記錄**: 每次呼叫都透過 `addEntry()` 寫入 session store

### 3.4 Agent 整合架構

#### IAgent 介面

**檔案**: `src/agents/types.ts`

```typescript
export type AgentId = "claude" | "codex" | "gemini" | "copilot";

export interface IAgent {
    readonly id: AgentId;
    readonly displayName: string;
    readonly cliCommand: string;
    readonly supportsParallelExecution: boolean;

    isAvailable(): Promise<boolean>;
    getModels(): string[];
    getModelConfigs(): ModelConfig[];
    getDefaultModel(): string;
    execute(options: ExecutionOptions): Promise<AgentResponse>;
    healthCheck(): Promise<HealthStatus>;
}
```

#### BaseAgent 抽象類別

**檔案**: `src/agents/base-agent.ts`

```typescript
export abstract class BaseAgent implements IAgent {
    abstract readonly id: AgentId;
    abstract readonly displayName: string;
    abstract readonly cliCommand: string;

    async execute(options: ExecutionOptions): Promise<AgentResponse> {
        const model = options.model ?? this.getDefaultModel();
        const effectiveTimeout = options.timeout ?? this.getTimeoutForModel(model);
        const spawnOptions = this.buildSpawnOptions({ ...options, timeout: effectiveTimeout }, model);

        const result = await spawnAgent(spawnOptions);  // child_process.spawn

        if (result.timedOut) {
            return { agent: this.id, model, content: "", durationMs: result.durationMs, exitCode: 124, error: `Timed out...` };
        }

        const content = this.parseOutput(result.stdout, result.stderr);
        return { agent: this.id, model, content, durationMs: result.durationMs, exitCode: result.exitCode };
    }

    // 子類別需實作
    protected abstract buildSpawnOptions(options: ExecutionOptions, model: string): SpawnOptions;

    // 可覆寫的輸出解析
    protected parseOutput(stdout: string, _stderr: string): string {
        return stdout.trim();
    }
}
```

#### 各 Agent 實作差異

| Agent | CLI | 特殊處理 |
|-------|-----|----------|
| Claude | `claude -p <prompt> --output-format json --model <model>` | override `parseOutput()` 解析 JSON 輸出 |
| Codex | `codex exec --model <model> --output-last-message=<tmpfile> -` | override 整個 `execute()`, 使用臨時檔讀取結果 |
| Gemini | `gemini -m <model> -p <prompt> --output-format stream-json --sandbox` | override `parseOutput()` 解析 NDJSON stream |
| Copilot | `copilot --model <model> -p <prompt> --allow-all-tools --silent` | `supportsParallelExecution = false` |

**Claude Agent 特殊 parseOutput**:

```typescript
protected override parseOutput(stdout: string, stderr: string): string {
    try {
        const parsed = JSON.parse(stdout);
        if (parsed.result) return parsed.result;
        if (typeof parsed === "string") return parsed;
        return JSON.stringify(parsed, null, 2);
    } catch {
        return stdout.trim() || stderr.trim();
    }
}
```

**Codex Agent 特殊 execute** -- 使用臨時檔讀取輸出:

```typescript
override async execute(options: ExecutionOptions): Promise<AgentResponse> {
    const outputFile = join(tmpdir(), `codex-output-${randomUUID()}.txt`);
    const args = [
        "exec", "--model", model, "-c", `model_reasoning_effort=${analysisLevel}`,
        "--sandbox", "read-only", `--output-last-message=${outputFile}`, "-",
    ];
    const result = await spawnAgent({ command: this.cliCommand, args, stdin: prompt });
    const content = this.readOutputContent(outputFile, result.stdout.trim());
    // ...
}
```

#### 遞迴呼叫防護 (Anti-Recursion)

**檔案**: `src/agents/registry.ts` + `src/utils/detect.ts`

這是本專案最精巧的設計之一。當 all-agents-mcp 在某個 agent 內部運行時 (例如 Claude Code 呼叫 all-agents-mcp), 它會自動偵測呼叫者並將其排除, 防止無限遞迴。

```typescript
// detect.ts -- 三層優先級偵測
export function detectCaller(): AgentId | null {
    // 1. CLI 參數: --caller=claude
    const callerArg = process.argv.find((arg) => arg.startsWith("--caller="));
    if (callerArg) { /* ... */ }

    // 2. 環境變數: CLAUDECODE, CODEX_SANDBOX_TYPE, GEMINI_CLI, COPILOT_CLI
    for (const [envVar, agentId] of Object.entries(CALLER_ENV_MAP)) {
        if (process.env[envVar]) return agentId;
    }

    // 3. process.env._ fallback (最後執行的指令)
    const lastCmd = process.env._ ?? "";
    if (lastCmd.includes("claude")) return "claude";
    // ...
}
```

```typescript
// registry.ts -- 排除呼叫者
export function getAgentRegistry(): Map<AgentId, IAgent> {
    const caller = detectCaller();
    for (const [id, factory] of Object.entries(agentFactories)) {
        if (id === caller) continue;  // 跳過呼叫者
        registry.set(id as AgentId, factory());
    }
    return registry;
}
```

#### Process Executor

**檔案**: `src/orchestrator/executor.ts`

```typescript
export async function spawnAgent(options: SpawnOptions): Promise<SpawnResult> {
    const controller = new AbortController();
    return new Promise<SpawnResult>((resolve) => {
        const child = spawn(command, args, {
            cwd,
            env: env ? { ...process.env, ...env } : process.env,
            signal: controller.signal,
            stdio: [stdin === undefined ? "ignore" : "pipe", "pipe", "pipe"],
        });

        const timer = setTimeout(() => { timedOut = true; controller.abort(); }, timeout);

        child.stdout?.on("data", (chunk) => stdoutChunks.push(chunk));
        child.stderr?.on("data", (chunk) => stderrChunks.push(chunk));

        if (stdin !== undefined && child.stdin) {
            child.stdin.end(stdin);
        }

        child.on("close", (code) => {
            clearTimeout(timer);
            resolve(buildSpawnResult(stdoutChunks, stderr, startTime, code ?? 1, timedOut));
        });
    });
}
```

**設計要點**:
- `AbortController` 配合 `setTimeout` 實作逾時中斷
- stdin 可選注入 (用於傳遞 context)
- stdout/stderr 分別收集為 Buffer[]
- 所有錯誤都被 resolve (不 reject), 由 SpawnResult.exitCode/timedOut 表達

#### 複雜度分析與任務路由

**檔案**: `src/orchestrator/complexity.ts`

```typescript
export function analyzeComplexity(task: string): ComplexityResult {
    let score = 0;
    const reasons: string[] = [];

    // 提示長度: >1000 字 +3, >500 字 +1
    // 大型任務關鍵字 (韓文+英文): "refactor", "migration", "전체" 等 +3 each
    // 複雜任務關鍵字: "analyze", "compare", "optimize" 等 +1 each
    // 多任務標記: 編號列表、項目符號、連接詞 +2 each
    // 多檔案引用 (>3 個) +2

    if (score >= 6) level = "large";      // 平行分配給多 agent
    else if (score >= 3) level = "complex"; // 單一 agent, 加倍逾時
    else level = "simple";                 // 單一 agent, 標準逾時
}
```

### 3.5 Resource 定義

3 個 MCP resources:

```typescript
// 靜態 URI
server.resource("sessions-list", "aa://sessions", async (uri) => ({
    contents: [{ uri: uri.href, mimeType: "application/json", text: JSON.stringify(listSessions()) }],
}));

// 動態 URI template
server.resource("session-history",
    new ResourceTemplate("aa://session/{id}/history", { list: undefined }),
    async (uri, params) => {
        const session = getSession(params.id as string);
        return { contents: [{ uri: uri.href, mimeType: "application/json", text: JSON.stringify(session) }] };
    },
);

// Agent 狀態
server.resource("agent-status", "aa://agents/status", async (uri) => { /* ... */ });
```

### 3.6 Session 儲存

**檔案**: `src/session/store.ts`

- 檔案式 JSON 儲存: `~/.all-agents-mcp/sessions/{uuid}.json`
- 每個 MCP server 生命週期有一個 `activeSessionId`
- 每次 tool 呼叫都 `addEntry()` 記錄: tool 名稱、agent、model、prompt、response、耗時、exit code

```typescript
export interface SessionEntry {
    id: string;
    timestamp: string;
    tool: string;
    agent: string;
    model: string;
    prompt: string;
    response: string;
    durationMs: number;
    exitCode: number;
    error?: string;
}
```

### 3.7 組態系統

**三層組態**: models.json (預設) -> 環境變數 (覆蓋) -> CLI 參數 (呼叫時)

**檔案**: `src/config/models.json` (節錄)

```json
{
    "agents": {
        "claude": {
            "default": "claude-opus-4.6",
            "defaultTimeoutSeconds": 120,
            "models": [
                { "name": "claude-opus-4.6", "timeoutSeconds": 300 },
                { "name": "claude-sonnet-4.6" },
                { "name": "claude-haiku-4.5" }
            ]
        },
        "codex": {
            "default": "gpt-5.3-codex-spark",
            "defaultAnalysisLevel": "xhigh",
            "models": [
                { "name": "gpt-5.3-codex-spark", "timeoutSeconds": 480 },
                { "name": "gpt-5.3-codex", "timeoutSeconds": 480 }
            ]
        }
    }
}
```

**環境變數覆蓋**: `AA_MCP_{AGENT}_{FIELD}` 格式

```
AA_MCP_CLAUDE_DEFAULT=claude-sonnet-4.5
AA_MCP_CODEX_MODELS=gpt-5.3-codex,gpt-5.2-codex
AA_MCP_CODEX_ANALYSIS_LEVEL=medium
AA_MCP_CODEX_TIMEOUT=300
```

**逾時優先級**:
1. 呼叫者 `options.timeout`
2. `models.json` 中該模型的 `timeoutSeconds * 1000`
3. agent 的 `defaultTimeoutSeconds * 1000`
4. 全域預設 `120_000ms`

---

## 4. Email 整合

經過完整原始碼搜尋, **本專案不包含任何 email 功能**。唯一的 "email" 字串出現在 `.claude-plugin/marketplace.json` 中作為作者聯絡資訊:

```json
{
    "owner": {
        "name": "Dokkabei97",
        "email": "wkdrn970@gmail.com"
    }
}
```

如果 "latest fix was about email" 的資訊是指其他更新, 可能是:
- 修復了 marketplace.json 中的 email 欄位格式
- 在專案外部的相關 issue 中提及 email
- 與其他 reference repo 混淆

本專案的核心功能聚焦於 **CLI agent 編排**, 不涉及 SMTP/email 發送。

---

## 5. 協定細節

### 5.1 JSON-RPC 實作

All Agents MCP **不直接實作 JSON-RPC**。它完全依賴 `@modelcontextprotocol/sdk` 來處理:

- **JSON-RPC 2.0 訊息**: 由 `StdioServerTransport` 在 stdin/stdout 上處理
- **方法分派**: 由 `McpServer` 內部路由 `tools/call`, `tools/list`, `resources/read`, `resources/list` 等
- **Schema 轉換**: Zod schema 自動轉為 JSON Schema, 嵌入 `tools/list` 回應

開發者只需使用高階 API:

```typescript
// 註冊 tool -- SDK 自動處理 tools/list + tools/call
server.tool("tool_name", "description", zodSchema, handler);

// 註冊 resource -- SDK 自動處理 resources/list + resources/read
server.resource("name", "uri://path", handler);
```

### 5.2 Tool Schema 範例

以 `ask_agent` 為例, Zod schema:

```typescript
const SCHEMA = {
    agent: z.enum(["claude", "codex", "gemini", "copilot"]).describe("Target agent to query"),
    prompt: z.string().describe("The question or prompt to send"),
    model: z.string().optional().describe("Specific model to use"),
    context: z.string().optional().describe("Additional context"),
    timeout: z.number().optional().describe("Timeout in milliseconds"),
    analysisLevel: z.enum(["low", "medium", "high", "xhigh"]).optional()
        .describe("Analysis depth level (codex only)"),
};
```

SDK 將其轉換為 JSON Schema (在 `tools/list` 回應中):

```json
{
    "name": "ask_agent",
    "description": "Ask a specific AI agent a question.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "agent": { "type": "string", "enum": ["claude", "codex", "gemini", "copilot"] },
            "prompt": { "type": "string" },
            "model": { "type": "string" },
            "timeout": { "type": "number" }
        },
        "required": ["agent", "prompt"]
    }
}
```

### 5.3 Tool Result 格式

所有 tool handler 回傳統一格式:

```typescript
return {
    content: [
        {
            type: "text" as const,
            text: "Markdown formatted response...",
        },
    ],
};
```

這對應 MCP 協定的 `CallToolResult`:

```json
{
    "jsonrpc": "2.0",
    "id": 1,
    "result": {
        "content": [
            { "type": "text", "text": "..." }
        ]
    }
}
```

### 5.4 Resource URI Schema

```
aa://sessions                    -- 所有 session 列表
aa://session/{id}/history        -- 特定 session 歷史 (URI template)
aa://agents/status               -- agent 狀態
```

回傳格式:

```typescript
{
    contents: [{
        uri: "aa://agents/status",
        mimeType: "application/json",
        text: JSON.stringify(data, null, 2),
    }],
}
```

### 5.5 Logger -- 保護 stdout

**檔案**: `src/utils/logger.ts`

```typescript
function log(level: LogLevel, message: string, ...args: unknown[]): void {
    if (!shouldLog(level)) return;
    const timestamp = new Date().toISOString();
    const prefix = `[${timestamp}] [${level.toUpperCase()}]`;
    process.stderr.write(`${prefix} ${message} ${args.length > 0 ? JSON.stringify(args) : ""}\n`);
}
```

**這是 MCP stdio 伺服器最關鍵的不變量**: 所有日誌必須寫入 stderr。stdout 專屬於 JSON-RPC 訊息。任何意外的 stdout 輸出都會破壞 MCP 協定。

---

## 6. 值得採用的模式

### 6.1 一檔一 Tool 模式

每個 tool 是獨立檔案, 匯出 `register*Tool(server)` 函數:

```
src/tools/
├── ask-agent.ts     -> registerAskAgentTool(server)
├── ask-all.ts       -> registerAskAllTool(server)
├── delegate.ts      -> registerDelegateTaskTool(server)
└── ...
```

**優點**:
- 高度模組化, 新增/移除 tool 只需修改一個檔案 + server.ts 註冊
- 每個 tool 的 schema、handler、session 記錄自成一體
- 測試容易隔離

### 6.2 Zod Schema 驅動的 Tool 定義

```typescript
const SCHEMA = {
    agent: z.enum(["claude", "codex", "gemini", "copilot"]).describe("..."),
    prompt: z.string().describe("..."),
};
server.tool("name", "description", SCHEMA, handler);
```

Zod 同時提供:
- **型別安全**: handler 參數自動推導型別
- **輸入驗證**: SDK 自動驗證輸入
- **JSON Schema 產生**: 自動轉換為 MCP tools/list 回應中的 inputSchema
- **文件**: `.describe()` 成為 schema description

### 6.3 遞迴呼叫防護

三層偵測機制 (CLI arg > env var > process.env._), 在 registry 初始化時排除呼叫者。這對任何 multi-agent 系統都是必要的。

### 6.4 stderr-only 日誌

stdio MCP 伺服器的硬性要求。自訂 logger 強制所有輸出走 `process.stderr.write()`。

### 6.5 Template Method + Strategy 模式

`BaseAgent` 使用 Template Method:
- `execute()` 定義骨架流程
- `buildSpawnOptions()` 由子類別實作 (Strategy)
- `parseOutput()` 可選覆寫

Codex 更進一步 override 整個 `execute()`, 展現良好的開放/封閉原則。

### 6.6 Session 審計追蹤

每次 tool 呼叫都記錄完整的 request/response, 可透過 MCP resource 查詢。這提供了:
- 除錯能力
- 可觀測性
- 跨 tool 呼叫的上下文串聯

### 6.7 環境變數覆蓋組態

`AA_MCP_{AGENT}_{FIELD}` 的命名規範 + Zod 驗證 + `structuredClone` 深拷貝合併, 實作環境特定覆蓋而不修改預設組態。

### 6.8 Promise.allSettled 容錯平行執行

```typescript
const results = await Promise.allSettled(agents.map((agent) => agent.execute(options)));
```

不用 `Promise.all` (一個失敗全部失敗), 而是 `Promise.allSettled` (收集所有結果, 個別處理成功/失敗)。

### 6.9 Claude Plugin 生態整合

```json
// .claude-plugin/plugin.json
{
    "name": "all-agents-mcp",
    "description": "Multi AI CLI Agent Orchestration..."
}
```

```json
// hooks/hooks.json -- SessionStart 時檢查環境
{
    "hooks": {
        "SessionStart": [{
            "matcher": "startup",
            "hooks": [{ "type": "command", "command": "bash ${CLAUDE_PLUGIN_ROOT}/scripts/check-setup.sh" }]
        }]
    }
}
```

```markdown
<!-- skills/ask/SKILL.md -- Skill 定義 -->
---
name: ask
description: Ask a specific AI agent a question
argument-hint: <agent> <question>
---
# Instructions
1. Parse the user's input...
2. Call the MCP tool `mcp__all-agents-mcp__ask_agent`...
```

---

## 7. 與 clawtex-core 的關聯性

### 7.1 clawtex-core 可以採用的模式

#### a) 將 clawtex tools 暴露為 MCP Server

clawtex-core 目前有 24 個 tools (shell, file_read, file_write, web_search...)。完全可以將它們暴露為 MCP server, 讓 Claude Code / Codex / Gemini CLI 直接使用。

**Rust 實作方案**: 使用 JSON-RPC 2.0 over stdio, 不需要 Node.js SDK。

核心協定只需實作:
1. `initialize` -- 回傳 server capabilities
2. `tools/list` -- 回傳 tool 定義 (JSON Schema)
3. `tools/call` -- 執行 tool 並回傳結果
4. `resources/list` / `resources/read` (可選)

```rust
// 概念性 Rust 實作
struct McpServer {
    tools: Vec<ToolDefinition>,
}

impl McpServer {
    fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "initialize" => self.handle_initialize(request),
            "tools/list" => self.handle_tools_list(),
            "tools/call" => self.handle_tools_call(request.params),
            _ => JsonRpcResponse::error(-32601, "Method not found"),
        }
    }
}
```

#### b) 一檔一 Tool 模式

clawtex-core 的 `src/tools/` 已經是一檔一 tool, 但可以參考 all-agents-mcp 的 `register*Tool(server)` 模式, 統一 tool 註冊 API:

```rust
// 類似 all-agents-mcp 的模式
pub trait McpToolProvider {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    async fn execute(&self, params: serde_json::Value) -> McpToolResult;
}
```

#### c) Session 審計追蹤

clawtex-core 已有 cost_tracker 和 revenue_tracker, 但缺少 **tool 呼叫層級的審計追蹤**。all-agents-mcp 的 SessionEntry 模式 (記錄每次 tool 呼叫的完整 request/response) 值得採用:

```rust
struct ToolCallEntry {
    id: Uuid,
    timestamp: chrono::DateTime<Utc>,
    tool: String,
    params: serde_json::Value,
    result: String,
    duration_ms: u64,
    error: Option<String>,
}
```

#### d) 遞迴呼叫防護

clawtex-core 已有 loop_detection.rs, 但 all-agents-mcp 的 **呼叫者偵測** 模式是互補的。如果 clawtex-core 將來作為 MCP server 被其他 agent 呼叫, 需要類似的機制防止遞迴。

### 7.2 主要差異

| 面向 | all-agents-mcp | clawtex-core |
|------|----------------|-------------|
| 語言 | TypeScript / Node.js | Rust |
| 核心功能 | Agent CLI 編排 | 完整 AI daemon (Telegram, HTTP, tools, hands) |
| Agent 整合 | spawn CLI process | HTTP API 呼叫 LLM providers |
| MCP 角色 | **MCP Server** (被呼叫) | **MCP Client** (呼叫外部 server) |
| Tool 數量 | 14 (全部是 agent 編排) | 24 (檔案、搜尋、瀏覽器、社群等) |
| 狀態管理 | JSON 檔案 | SQLite + pgvector |
| 認證 | 各 CLI 自己管理 | agents.toml + encrypted secrets |

### 7.3 可行的整合路徑

1. **clawtex-core 作為 MCP Server**: 暴露 24 個 tools + cluster 資源, 讓 Claude Code/Codex/Gemini 直接使用 clawtex 的能力
2. **clawtex-core 作為 MCP Client + Server**: 現有的 MCP client (`src/mcp/mod.rs`) 呼叫外部 server (如 all-agents-mcp), 同時自身也暴露 MCP server
3. **stdio transport for clawtex**: 新增 `clawtex-core mcp-server` 子命令, 以 stdio 模式啟動, 用於 `claude mcp add clawtex -- clawtex-core mcp-server`

### 7.4 具體建議

1. **新增 `src/mcp/server.rs`**: 實作 MCP server 端 (JSON-RPC 2.0 over stdio)
2. **Tool 適配器**: 將 ToolRegistry 中的每個 tool 包裝為 MCP tool definition
3. **保護 stdout**: 如同 all-agents-mcp 的 stderr-only logger, stdio 模式下所有日誌必須走 stderr
4. **Resources**: 暴露 cluster 狀態、agent 健康、手牌列表等為 MCP resources
5. **Recursive guard**: 偵測 clawtex-core 是否在 Claude Code/Codex 內部運行, 避免 delegate tool 造成遞迴

---

## 附錄: 完整檔案索引

| 檔案 | 行數 | 說明 |
|------|------|------|
| `src/index.ts` | 22 | 進入點 |
| `src/server.ts` | 53 | MCP server 工廠 |
| `src/agents/types.ts` | 55 | 核心型別定義 |
| `src/agents/base-agent.ts` | 126 | 抽象基底 Agent |
| `src/agents/claude-agent.ts` | 37 | Claude Code 實作 |
| `src/agents/codex-agent.ts` | 100 | Codex 實作 (custom execute) |
| `src/agents/gemini-agent.ts` | 43 | Gemini CLI 實作 |
| `src/agents/copilot-agent.ts` | 37 | Copilot CLI 實作 |
| `src/agents/registry.ts` | 69 | Agent registry + 遞迴防護 |
| `src/tools/ask-agent.ts` | 75 | ask_agent tool |
| `src/tools/ask-all.ts` | 65 | ask_all tool |
| `src/tools/delegate.ts` | 124 | delegate_task tool |
| `src/tools/collaborate.ts` | 68 | collaborate tool |
| `src/tools/verify.ts` | 75 | verify tool |
| `src/tools/review-code.ts` | 81 | review_code tool |
| `src/tools/debug-with.ts` | 61 | debug_with tool |
| `src/tools/explain-with.ts` | 67 | explain_with tool |
| `src/tools/generate-test.ts` | 77 | generate_test tool |
| `src/tools/refactor-with.ts` | 75 | refactor_with tool |
| `src/tools/fetch-page.ts` | 104 | fetch_page tool |
| `src/tools/list-agents.ts` | 40 | list_agents tool |
| `src/tools/list-models.ts` | 55 | list_models tool |
| `src/tools/agent-health.ts` | 54 | agent_health tool |
| `src/orchestrator/executor.ts` | 87 | Process spawn 封裝 |
| `src/orchestrator/parallel.ts` | 46 | 平行執行引擎 |
| `src/orchestrator/complexity.ts` | 102 | 複雜度分析 |
| `src/orchestrator/verifier.ts` | 85 | 跨模型驗證 |
| `src/orchestrator/aggregator.ts` | 82 | Markdown 格式化 |
| `src/session/types.ts` | 20 | Session 型別 |
| `src/session/store.ts` | 78 | Session CRUD |
| `src/resources/sessions-list.ts` | 15 | sessions resource |
| `src/resources/session-history.ts` | 37 | session history resource |
| `src/resources/agent-status.ts` | 29 | agent status resource |
| `src/config/schema.ts` | 26 | Zod schema |
| `src/config/loader.ts` | 108 | 組態載入 + 環境覆蓋 |
| `src/config/models.json` | 62 | 預設模型組態 |
| `src/utils/logger.ts` | 29 | stderr-only logger |
| `src/utils/detect.ts` | 46 | CLI 偵測 + 呼叫者辨識 |
