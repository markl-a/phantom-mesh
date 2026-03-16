# Gemini CLI 深度技術分析 v2

> 分析對象: `references/gemini-cli/` (v0.35.0-nightly, 2026-03-11)
> 分析日期: 2026-03-13 (深度重寫版)
> 目的: 從 clawtex-core 開發者視角，徹底拆解 Gemini CLI 的架構、模式與可借鑑之處
> 深度: 程式碼行號級引用、資料流圖、錯誤處理路徑、效能瓶頸、clawtex 差距對比

---

## 目錄

1. [專案結構](#1-專案結構)
2. [進入點與啟動流程](#2-進入點與啟動流程)
3. [CoreToolScheduler — Builder+Invoker 分離架構（深度）](#3-coretoolscheduler--builderinvoker-分離架構深度)
4. [Hooks 生命週期系統（深度）](#4-hooks-生命週期系統深度)
5. [1M Context Window 策略（深度）](#5-1m-context-window-策略深度)
6. [LLM-Based Loop Detection（深度）](#6-llm-based-loop-detection深度)
7. [MCP Auth Providers（深度）](#7-mcp-auth-providers深度)
8. [Agent Loop 三層架構](#8-agent-loop-三層架構)
9. [模型路由與分類器](#9-模型路由與分類器)
10. [沙箱與安全](#10-沙箱與安全)
11. [遙測與計費](#11-遙測與計費)
12. [子代理與擴展系統](#12-子代理與擴展系統)
13. [錯誤處理與韌性設計](#13-錯誤處理與韌性設計)
14. [效能分析](#14-效能分析)
15. [Clawtex 差距總覽與實作路線圖](#15-clawtex-差距總覽與實作路線圖)
16. [工具輸出遮罩服務（深度）](#16-工具輸出遮罩服務深度)
17. [執行生命週期服務（深度）](#17-執行生命週期服務深度)
18. [上下文記憶體管理（深度）](#18-上下文記憶體管理深度)
19. [Session 摘要服務（深度）](#19-session-摘要服務深度)
20. [沙箱管理深化分析](#20-沙箱管理深化分析)
21. [關鍵檔案索引](#21-關鍵檔案索引)

---

## 1. 專案結構

### Monorepo 架構

```
gemini-cli/
├── packages/
│   ├── cli/           # TUI 前端 (React/Ink 19)
│   ├── core/          # 核心邏輯 (無 UI 依賴)
│   ├── sdk/           # 程式化 SDK (GeminiCliAgent)
│   ├── a2a-server/    # Agent-to-Agent 伺服器
│   ├── devtools/      # 開發者工具面板
│   ├── test-utils/    # 測試工具
│   └── vscode-ide-companion/  # VSCode 整合
├── evals/             # 模型評估框架
├── integration-tests/ # 整合測試
├── schemas/           # JSON Schema 定義
├── sea/               # Single Executable Application 打包
└── docs/              # 文件
```

### 核心套件 (`packages/core/src/`) 完整目錄

```
core/src/
├── agents/            # 子代理系統 (12+ 檔案)
│   ├── auth-provider/ # 代理認證提供者 (factory, api-key, http, oauth2)
│   ├── browser/       # 瀏覽器代理 (Playwright)
│   ├── generalist-agent.ts
│   ├── codebase-investigator.ts
│   ├── cli-help-agent.ts
│   ├── agent-scheduler.ts
│   ├── registry.ts
│   └── local-executor.ts / remote-invocation.ts
├── availability/      # 模型可用性策略 (5 檔案)
├── billing/           # Google One AI 點數計費
├── code_assist/       # CodeAssist 伺服器 (OAuth/免費層)
│   ├── admin/         # Admin controls
│   ├── experiments/   # 實驗性旗標
│   └── oauth2.ts / server.ts / setup.ts
├── commands/          # 斜線指令 (/model, /clear, /bug 等)
├── config/            # 設定管理 (models, memory, storage, settings)
├── confirmation-bus/  # 工具確認匯流排 (MessageBus + 類型)
├── core/              # 核心引擎
│   ├── turn.ts        # Turn 串流抽象
│   ├── geminiChat.ts  # 對話管理
│   ├── coreToolScheduler.ts  # 工具排程器
│   ├── contentGenerator.ts   # LLM API 抽象
│   ├── baseLlmClient.ts      # 基底客戶端
│   ├── prompts.ts     # 系統提示詞
│   └── tokenLimits.ts # Token 上限定義
├── fallback/          # 模型降級處理
├── hooks/             # 生命週期鉤子系統 (8+ 檔案)
│   ├── hookSystem.ts
│   ├── hookRegistry.ts
│   ├── hookRunner.ts
│   ├── hookAggregator.ts
│   ├── hookPlanner.ts
│   ├── hookEventHandler.ts
│   ├── hookTranslator.ts
│   └── types.ts
├── mcp/               # MCP 認證提供者 (12+ 檔案)
│   ├── auth-provider.ts
│   ├── google-auth-provider.ts
│   ├── mcp-oauth-provider.ts
│   ├── sa-impersonation-provider.ts
│   ├── oauth-provider.ts
│   └── token-storage/ (base, file, keychain, hybrid)
├── policy/            # 策略引擎 (TOML 規則)
├── prompts/           # 系統提示詞管理
├── routing/           # 模型路由 (classifier, fallback, composite)
├── safety/            # 安全過濾
├── scheduler/         # 工具排程器元件
│   ├── tool-executor.ts
│   ├── tool-modifier.ts
│   ├── policy.ts
│   └── types.ts
├── services/          # 核心服務
│   ├── chatCompressionService.ts
│   ├── loopDetectionService.ts
│   ├── sandboxManager.ts
│   ├── environmentSanitization.ts
│   └── shellExecutionService.ts
├── skills/            # 技能載入器
├── telemetry/         # Clearcut 遙測 (40+ 事件)
├── tools/             # 內建工具 (18+)
│   ├── definitions/   # 工具定義 (model-family-sets)
│   ├── tools.ts       # DeclarativeTool 基底
│   ├── tool-registry.ts
│   ├── mcp-client.ts / mcp-tool.ts
│   ├── shell.ts, read-file.ts, write-file.ts, edit.ts
│   ├── glob.ts, grep.ts, ls.ts
│   ├── web-search.ts, web-fetch.ts
│   └── memoryTool.ts, write-todos.ts, ...
├── utils/             # 工具函式
└── voice/             # 語音介面
```

**關鍵觀察**：core 套件完全不依賴 UI，與 CLI (React/Ink) 完全分離。

> **Clawtex 實作建議**：clawtex-core 的 Telegram UI 與核心邏輯耦合較緊。應參考 Gemini CLI 的 core/cli 分離，將 `src/telegram.rs` 的業務邏輯提取到 `src/core/` 模組中。

---

## 2. 進入點與啟動流程

### CLI 入口

```typescript
// packages/cli/index.ts
#!/usr/bin/env -S node --no-warnings=DEP0040
import { main } from './src/gemini.js';
main().catch(async (error) => {
  const cleanupTimeout = setTimeout(() => {
    writeToStderr('Cleanup timed out, forcing exit...\n');
    process.exit(1);
  }, 5000);  // 清理超時 5 秒保護
  try { await runExitCleanup(); } catch { }
  finally { clearTimeout(cleanupTimeout); }
});
```

### 完整啟動序列

```
packages/cli/src/gemini.tsx — main()
│
├─ 1. startupProfiler.start('cli_startup')  // 啟動效能計時
├─ 2. patchStdio()                          // 攔截 stdout/stderr
├─ 3. setupSignalHandlers()                 // SIGINT/SIGTERM
├─ 4. loadSettings()                        // ~/.gemini/settings.json
├─ 5. loadTrustedFolders()                  // 信任資料夾白名單
├─ 6. cleanupCheckpoints()                  // 清理過期 checkpoint
├─ 7. parseArguments() (yargs)              // CLI 參數
├─ 8. loadCliConfig() → Config              // 建立核心 Config
├─ 9. refreshAuth()                         // OAuth/API Key 刷新
├─ 10. 沙箱判斷:
│     ├─ 需沙箱 → start_sandbox() (Docker/Seatbelt)
│     │     // 在沙箱中重新啟動自身（子程序模式）
│     └─ 不需沙箱 → relaunchAppInChildProcess()
│           // 調整 Node.js 堆積大小（系統記憶體 50%）
├─ 11. config.storage.initialize()          // 儲存初始化
├─ 12. PolicyEngine + PolicyUpdater         // TOML 策略引擎
├─ 13. initializeApp()                      // auth + theme + IDE
├─ 14. HookSystem.initialize()              // 載入所有 hooks
└─ 15. 模式分歧:
      ├─ 互動 → startInteractiveUI() (Ink React 渲染)
      └─ 非互動 → runNonInteractive()
```

### 記憶體管理

```typescript
// packages/cli/src/gemini.tsx
// 動態調整 Node.js 堆積大小為系統記憶體的 50%
const targetMaxOldSpaceSizeInMB = Math.floor(totalMemoryMB * 0.5);
if (targetMaxOldSpaceSizeInMB > currentMaxOldSpaceSizeMb) {
  return [`--max-old-space-size=${targetMaxOldSpaceSizeInMB}`];
}
```

### 沙箱自我重啟

```typescript
// packages/cli/src/gemini.tsx (~line 524-580)
if (!process.env['SANDBOX']) {
  const sandboxConfig = await loadSandboxConfig(settings.merged, argv);
  if (sandboxConfig) {
    // 在沙箱中重新啟動自身
    await relaunchOnExitCode(() =>
      start_sandbox(sandboxConfig, memoryArgs, partialConfig, sandboxArgs),
    );
    process.exit(0);
  }
}
```

> **Clawtex 實作建議**：
> 1. 沙箱自我重啟模式值得學習 — clawtex daemon 可以在檢測到不安全環境時自動重啟到沙箱中。
> 2. 動態記憶體調整對於 Rust 不太需要（無 GC），但 clawtex 呼叫外部工具時可以使用。

---

## 3. CoreToolScheduler — Builder+Invoker 分離架構（深度）

### 3.1 架構概覽

Gemini CLI 的工具系統採用 **Builder + Invocation** 模式，將工具的驗證、確認、執行三個關注點徹底分離：

```
DeclarativeTool (工具定義)
    │
    ├── getSchema(modelId?)           → FunctionDeclaration (給 LLM)
    │
    ├── build(params, config, signal) → ToolInvocation (驗證 + 建立)
    │     │
    │     ├── getDescription()                  → string (人類可讀描述)
    │     ├── shouldConfirmExecute(signal)       → ConfirmationDetails | false
    │     ├── execute(signal, updateOutput)      → ToolResult
    │     └── getPolicyUpdateOptions()           → PolicyUpdateOptions
    │
    └── Kind: Read | Edit | Delete | Execute | Search | Think | Agent | ...
```

### 3.2 CoreToolScheduler 核心結構

```typescript
// packages/core/src/core/coreToolScheduler.ts:102-168
export class CoreToolScheduler {
  // 靜態 WeakMap 防止重複訂閱 MessageBus
  private static subscribedMessageBuses = new WeakMap<
    MessageBus,
    (request: ToolConfirmationRequest) => void
  >();

  private toolCalls: ToolCall[] = [];           // 當前批次的所有工具呼叫
  private isFinalizingToolCalls = false;        // 是否正在完成批次
  private isScheduling = false;                 // 是否正在排程
  private isCancelling = false;                 // 是否正在取消
  private requestQueue: Array<{                 // 請求佇列
    request: ToolCallRequestInfo | ToolCallRequestInfo[];
    signal: AbortSignal;
    resolve: () => void;
    reject: (reason?: Error) => void;
  }> = [];
  private toolCallQueue: ToolCall[] = [];       // 待執行工具佇列
  private completedToolCallsForBatch: CompletedToolCall[] = [];  // 已完成
  private toolExecutor: ToolExecutor;           // 實際執行器
  private toolModifier: ToolModificationHandler; // 工具修改處理
}
```

### 3.3 工具呼叫狀態機

```
ToolCall 狀態轉換:

  ┌──────────────┐
  │  Validating  │ ← build() 驗證參數
  └──────┬───────┘
         │ 驗證成功
         ▼
  ┌──────────────┐
  │  Scheduled   │ ← 已排入佇列
  └──────┬───────┘
         │
    ┌────┴────┐
    ▼         ▼
┌────────┐ ┌──────────────────┐
│Executing│ │ AwaitingApproval │ ← shouldConfirmExecute() = true
└────┬───┘ └──────┬───────────┘
     │             │ 使用者決定
     │        ┌────┴─────┐
     │        ▼          ▼
     │    ┌─────────┐ ┌───────────┐
     │    │Executing│ │ Cancelled │
     │    └────┬────┘ └───────────┘
     │         │
     └────┬────┘
          │
    ┌─────┴──────┐
    ▼            ▼
┌─────────┐ ┌────────┐
│ Success │ │ Error  │
└─────────┘ └────────┘
```

```typescript
// packages/core/src/scheduler/types.ts
export enum CoreToolCallStatus {
  Validating = 'validating',      // 驗證中
  Scheduled = 'scheduled',        // 已排程
  AwaitingApproval = 'awaiting',  // 等待審批
  Executing = 'executing',        // 執行中
  Success = 'success',            // 成功
  Error = 'error',                // 錯誤
  Cancelled = 'cancelled',        // 已取消
}
```

### 3.4 工具分類 (Kind) 與並行策略

```typescript
// packages/core/src/tools/tools.ts
export enum Kind {
  Read = 'read',           // 唯讀 → 可並行
  Edit = 'edit',           // 編輯 → 需確認，循序
  Delete = 'delete',       // 刪除 → 需確認，循序
  Move = 'move',           // 移動 → 需確認，循序
  Search = 'search',       // 搜尋 → 可並行
  Execute = 'execute',     // 執行 → 需確認，循序
  Think = 'think',         // 思考 → 可並行（內部推理）
  Agent = 'agent',         // 子代理 → 循序
  Fetch = 'fetch',         // 網路 → 可並行
  Communicate = 'communicate',
  Plan = 'plan',           // 計畫
  SwitchMode = 'switch_mode',
  Other = 'other',
}
```

### 3.5 ToolExecutor — 實際執行流程

```typescript
// packages/core/src/scheduler/tool-executor.ts:59-100
export class ToolExecutor {
  async execute(context: ToolExecutionContext): Promise<CompletedToolCall> {
    const { call, signal, outputUpdateHandler, onUpdateToolCall } = context;

    // 1. 驗證 tool + invocation 存在
    if (!('tool' in call) || !call.tool || !('invocation' in call)) {
      throw new Error(`Cannot execute: Tool or Invocation missing.`);
    }

    // 2. 設定即時輸出回呼
    const liveOutputCallback = tool.canUpdateOutput && outputUpdateHandler
      ? (outputChunk) => outputUpdateHandler(callId, outputChunk)
      : undefined;

    // 3. 包裝在 DevTrace span 中
    return runInDevTraceSpan({
      operation: GeminiCliOperation.ToolCall,
      attributes: { tool_name, call_id, tool_description },
    }, async ({ metadata }) => {
      try {
        // 4. 透過 hooks 執行工具
        const result = await executeToolWithHooks(
          this.config, invocation, signal,
          liveOutputCallback, shellExecutionConfig,
        );

        // 5. 處理輸出截斷
        if (shouldTruncate(result)) {
          const { outputFile } = await saveTruncatedToolOutput(...);
          result.llmContent = formatTruncatedToolOutput(...);
        }

        // 6. 返回成功
        return { status: CoreToolCallStatus.Success, ... };
      } catch (error) {
        if (isAbortError(error)) {
          return { status: CoreToolCallStatus.Cancelled, ... };
        }
        return { status: CoreToolCallStatus.Error, ... };
      }
    });
  }
}
```

### 3.6 確認流程 — Policy Engine + MessageBus

```
模型請求工具呼叫
    │
    ▼
CoreToolScheduler.schedule(request, signal)
    │
    ├─ 1. tool.build(params) → ToolInvocation (驗證)
    │      失敗 → 直接返回 ErroredToolCall
    │
    ├─ 2. PolicyEngine.evaluate(toolName, params) → PolicyDecision
    │      ├─ ALLOW → 跳過確認
    │      ├─ DENY → 返回 ErroredToolCall
    │      └─ ASK_USER → 進入確認流程
    │
    ├─ 3. invocation.shouldConfirmExecute(signal) → ConfirmationDetails | false
    │      └─ 如果工具本身也要求確認
    │
    ├─ 4. MessageBus.publish(ToolConfirmationRequest)
    │      ├─ UI 層收到 → 顯示確認對話框
    │      └─ MessageBus.subscribe(ToolConfirmationResponse)
    │
    └─ 5. 使用者決定:
           ├─ ProceedOnce → 執行一次
           ├─ ProceedAlways → 執行 + 更新 PolicyEngine
           ├─ ProceedAlwaysAndSave → 執行 + 儲存到 TOML
           ├─ ProceedAlwaysTool → 永遠允許此工具
           ├─ ProceedAlwaysServer → 永遠允許此 MCP 伺服器
           ├─ ModifyWithEditor → 打開編輯器修改
           └─ Cancel → 取消
```

```typescript
// packages/core/src/tools/tools.ts
export enum ToolConfirmationOutcome {
  ProceedOnce = 'proceed_once',
  ProceedAlways = 'proceed_always',
  ProceedAlwaysAndSave = 'proceed_always_and_save',
  ProceedAlwaysServer = 'proceed_always_server',
  ProceedAlwaysTool = 'proceed_always_tool',
  ModifyWithEditor = 'modify_with_editor',
  Cancel = 'cancel',
}
```

### 3.7 MessageBus 解耦設計

```typescript
// packages/core/src/confirmation-bus/types.ts
export enum MessageBusType {
  TOOL_CONFIRMATION_REQUEST = 'tool_confirmation_request',
  TOOL_CONFIRMATION_RESPONSE = 'tool_confirmation_response',
}

// packages/core/src/confirmation-bus/message-bus.ts
export class MessageBus {
  subscribe(type: MessageBusType, handler: Function): void;
  publish(message: { type: MessageBusType; correlationId: string; ... }): Promise<void>;
}
```

**WeakMap 防重複訂閱**：

```typescript
// packages/core/src/core/coreToolScheduler.ts:103-167
// 使用 static WeakMap 確保每個 MessageBus 只訂閱一次
// 防止 React 重新渲染時建立多個 CoreToolScheduler 導致重複訂閱
private static subscribedMessageBuses = new WeakMap<MessageBus, Function>();

if (!CoreToolScheduler.subscribedMessageBuses.has(messageBus)) {
  const sharedHandler = (request: ToolConfirmationRequest) => {
    messageBus.publish({
      type: MessageBusType.TOOL_CONFIRMATION_RESPONSE,
      correlationId: request.correlationId,
      confirmed: false,
      requiresUserConfirmation: true,
    });
  };
  messageBus.subscribe(MessageBusType.TOOL_CONFIRMATION_REQUEST, sharedHandler);
  CoreToolScheduler.subscribedMessageBuses.set(messageBus, sharedHandler);
}
```

### 3.8 完整資料流圖

```
LLM 回應（含 function_calls）
    │
    ▼
Turn.run() → yield GeminiEventType.ToolCallRequest
    │
    ▼
CoreToolScheduler.schedule(request[], signal)
    │
    ├── 批次驗證: requests.map(req => tool.build(params))
    │   ├── 成功 → ValidatingToolCall → ScheduledToolCall
    │   └── 失敗 → ErroredToolCall
    │
    ├── 策略評估: PolicyEngine.evaluate(each)
    │   ├── ALLOW → 直接排入執行佇列
    │   ├── DENY → ErroredToolCall (PolicyDenialError)
    │   └── ASK_USER → WaitingToolCall → 等待 MessageBus 回應
    │
    ├── 執行（可並行）:
    │   ├── Read/Search/Fetch → Promise.all([...])  // 並行
    │   └── Edit/Delete/Execute → 循序                // 獨佔
    │
    ├── ToolExecutor.execute(call, signal)
    │   ├── executeToolWithHooks(config, invocation, signal)
    │   │   ├── hooks.fireBeforeToolEvent(name, input)
    │   │   ├── invocation.execute(signal, updateOutput)
    │   │   └── hooks.fireAfterToolEvent(name, input, response)
    │   │
    │   ├── 輸出截斷判斷
    │   └── 結果 → SuccessfulToolCall | ErroredToolCall | CancelledToolCall
    │
    └── onAllToolCallsComplete(completedCalls)
        │
        ▼
    GeminiChat.sendMessageStream(functionResponses)
        │
        ▼
    下一輪 Turn.run()
```

> **Clawtex 實作建議**：
> 1. clawtex 的工具目前是 `async fn execute(params) -> Result<String>`，沒有 build/validate 分離。建議引入：
>    ```rust
>    trait ClawtexTool {
>        fn build(&self, params: Value) -> Result<ToolInvocation>;
>    }
>    trait ToolInvocation {
>        fn needs_approval(&self) -> Option<ApprovalReason>;
>        async fn execute(&self, cancel: CancellationToken) -> Result<ToolOutput>;
>    }
>    ```
> 2. MessageBus 解耦是關鍵 — clawtex 的 approval 目前直接呼叫 Telegram API。引入 MessageBus 允許不同 UI (Telegram, HTTP, CLI) 統一處理確認。
> 3. `ToolConfirmationOutcome` 的 7 種結果比 clawtex 的 bool (approve/deny) 豐富得多。特別是 `ProceedAlwaysAndSave` 可以自動更新 `agents.toml`。
> 4. 工具 Kind 分類 + 並行策略與 Codex 的 `RwLock<()>` 方案互補 — Gemini 更語義化，Codex 更機械化。

---

## 4. Hooks 生命週期系統（深度）

### 4.1 Hook 系統架構

```
HookSystem (入口)
    │
    ├── HookRegistry        — 註冊/管理所有 hook
    │   ├── 從 settings.json 載入
    │   ├── 從 gemini-extension.json 載入
    │   └── 從工作區 .gemini/ 載入
    │
    ├── HookPlanner         — 根據事件類型規劃執行順序
    │   └── 按優先級排序 hook
    │
    ├── HookRunner          — 實際執行 hook (subprocess)
    │   └── spawn 子程序，透過 stdin/stdout 通訊
    │
    ├── HookAggregator      — 匯總多個 hook 的結果
    │   └── 合併 blocked/stopped/modified
    │
    └── HookEventHandler    — 事件分派 (13+ 事件)
        ├── fireBeforeToolEvent()
        ├── fireAfterToolEvent()
        ├── fireBeforeAgentEvent()
        ├── fireAfterAgentEvent()
        ├── fireBeforeModelEvent()
        ├── fireAfterModelEvent()
        ├── fireBeforeToolSelectionEvent()
        ├── fireSessionStartEvent()
        ├── fireSessionEndEvent()
        ├── firePreCompressEvent()
        ├── fireNotificationEvent()
        ├── fireToolConfirmationEvent()
        └── fireModelRerouteEvent()
```

### 4.2 HookSystem 初始化

```typescript
// packages/core/src/hooks/hookSystem.ts:149-168
export class HookSystem {
  private readonly hookRegistry: HookRegistry;
  private readonly hookRunner: HookRunner;
  private readonly hookAggregator: HookAggregator;
  private readonly hookPlanner: HookPlanner;
  private readonly hookEventHandler: HookEventHandler;

  constructor(config: Config) {
    this.hookRegistry = new HookRegistry(config);
    this.hookRunner = new HookRunner(config);
    this.hookAggregator = new HookAggregator();
    this.hookPlanner = new HookPlanner(this.hookRegistry);
    this.hookEventHandler = new HookEventHandler(
      config,
      this.hookPlanner,
      this.hookRunner,
      this.hookAggregator,
    );
  }

  async initialize(): Promise<void> {
    await this.hookRegistry.initialize();
  }
}
```

### 4.3 Hook 事件名稱（完整列舉）

```typescript
// packages/core/src/hooks/types.ts
export enum HookEventName {
  BeforeTool = 'before_tool',              // 工具執行前
  AfterTool = 'after_tool',               // 工具執行後
  BeforeAgent = 'before_agent',            // 代理處理前
  AfterAgent = 'after_agent',              // 代理處理後
  BeforeModel = 'before_model',            // 模型呼叫前
  AfterModel = 'after_model',              // 模型回應後
  BeforeToolSelection = 'before_tool_sel', // 工具選擇前
  SessionStart = 'session_start',          // 工作階段開始
  SessionEnd = 'session_end',              // 工作階段結束
  PreCompress = 'pre_compress',            // 壓縮前
  Notification = 'notification',           // 通知
}

export enum HookType {
  Command = 'command',     // 執行外部命令
  Script = 'script',       // 執行腳本
  Builtin = 'builtin',     // 內建 hook
}
```

### 4.4 BeforeModel Hook — 最強大的 hook

```typescript
// packages/core/src/hooks/hookSystem.ts:42-55
export interface BeforeModelHookResult {
  blocked: boolean;                      // 是否阻擋模型呼叫
  stopped?: boolean;                     // 是否停止整個執行
  reason?: string;                       // 阻擋原因
  syntheticResponse?: GenerateContentResponse;  // 替代回應（不呼叫 LLM）
  modifiedConfig?: GenerateContentConfig;       // 修改模型配置
  modifiedContents?: ContentListUnion;          // 修改輸入內容
}
```

**使用案例**：
- **內容審查**：BeforeModel hook 可以掃描輸入，阻擋敏感內容
- **成本控制**：檢查 token 預算，超限時提供合成回應
- **測試**：提供 mock 回應，不實際呼叫 API
- **內容修改**：在送入模型前注入額外上下文

### 4.5 AfterModel Hook

```typescript
// packages/core/src/hooks/hookSystem.ts:67-80
export interface AfterModelHookResult {
  response: GenerateContentResponse;  // 回應（原始或修改後）
  stopped?: boolean;                  // 是否停止執行
  blocked?: boolean;                  // 是否阻擋
  reason?: string;                    // 原因
}
```

### 4.6 HookEventHandler — 事件觸發

```typescript
// packages/core/src/hooks/hookEventHandler.ts:47-200
export class HookEventHandler {
  // 防重複報告失敗
  private readonly reportedFailures = new WeakMap<object, Set<string>>();

  // BeforeTool: 可阻擋工具執行
  async fireBeforeToolEvent(
    toolName: string,
    toolInput: Record<string, unknown>,
    mcpContext?: McpToolContext,
  ): Promise<AggregatedHookResult> {
    const input: BeforeToolInput = {
      ...this.createBaseInput(HookEventName.BeforeTool),
      tool_name: toolName,
      tool_input: toolInput,
      ...(mcpContext && { mcp_context: mcpContext }),
    };
    return this.executeHooks(HookEventName.BeforeTool, input, { toolName });
  }

  // AfterTool: 可修改工具輸出
  async fireAfterToolEvent(
    toolName: string,
    toolInput: Record<string, unknown>,
    toolResponse: Record<string, unknown>,
  ): Promise<AggregatedHookResult> { ... }

  // SessionStart: 工作階段開始時執行
  async fireSessionStartEvent(source: SessionStartSource): Promise<AggregatedHookResult> {
    const input: SessionStartInput = {
      ...this.createBaseInput(HookEventName.SessionStart),
      source,  // 'interactive' | 'non-interactive' | 'sdk'
    };
    return this.executeHooks(HookEventName.SessionStart, input, { trigger: source });
  }

  // SessionEnd: 工作階段結束時執行
  async fireSessionEndEvent(reason: SessionEndReason): Promise<AggregatedHookResult> { ... }

  // PreCompress: 壓縮前的最後機會
  async firePreCompressEvent(trigger: PreCompressTrigger): Promise<AggregatedHookResult> { ... }

  // BeforeToolSelection: 可修改工具列表
  async fireBeforeToolSelectionEvent(
    request: GenerateContentParameters,
  ): Promise<BeforeToolSelectionHookResult> { ... }

  // Notification: 通知 hook（工具確認等）
  async fireNotificationEvent(
    type: NotificationType,
    message: string,
    details: Record<string, unknown>,
  ): Promise<AggregatedHookResult> { ... }
}
```

### 4.7 Hook 序列化

```typescript
// packages/core/src/hooks/hookSystem.ts:86-127
// ToolCallConfirmationDetails 轉為可序列化格式
function toSerializableDetails(details: ToolCallConfirmationDetails): Record<string, unknown> {
  switch (details.type) {
    case 'edit':
      return { type, title, fileName, filePath, fileDiff, originalContent, newContent, isModifying };
    case 'exec':
      return { type, title, command, rootCommand };
    case 'mcp':
      return { type, title, serverName, toolName, toolDisplayName };
    case 'info':
      return { type, title, prompt, urls };
  }
}
```

### 4.8 Hook 配置格式

```json
// gemini-extension.json 中的 hook 配置
{
  "hooks": {
    "before_tool": [{
      "type": "command",
      "command": "node check-tool.js",
      "tools": ["shell", "write_file"],
      "timeout": 5000
    }],
    "after_model": [{
      "type": "script",
      "script": "validate-response.sh",
      "timeout": 3000
    }],
    "session_start": [{
      "type": "builtin",
      "name": "load-context"
    }]
  }
}
```

> **Clawtex 實作建議**：
> 1. clawtex 目前沒有 hook 系統。建議在 `agents.toml` 中加入 `[hooks]` 區段：
>    ```toml
>    [hooks]
>    before_tool = ["check_safety.sh"]
>    after_agent = ["log_response.sh"]
>    session_start = ["load_memory.sh"]
>    ```
> 2. **BeforeModel hook** 是最有價值的 — 可用於成本控制、內容審查、A/B 測試。
> 3. **BeforeToolSelection hook** 允許動態修改可用工具列表 — 這對 clawtex 的 Hands workflow 特別有用。
> 4. hook 的 subprocess 執行模式（stdin/stdout JSON）與 MCP 類似，可重用 clawtex 現有的 MCP 基礎設施。
> 5. 重要：Gemini CLI 的 hook 是**非阻塞預設**的 — 失敗不會中斷主流程，只記錄警告。

---

## 5. 1M Context Window 策略（深度）

### 5.1 Token 上限定義

```typescript
// packages/core/src/core/tokenLimits.ts
export const DEFAULT_TOKEN_LIMIT = 1_048_576;  // 1M tokens

export function tokenLimit(model: Model): TokenCount {
  switch (model) {
    case 'gemini-3-pro-preview':
    case 'gemini-3.1-pro-preview':
    case 'gemini-3-flash-preview':
    case 'gemini-2.5-pro':
    case 'gemini-2.5-flash':
    case 'gemini-2.5-flash-lite':
      return 1_048_576;  // 全部 1M
    default:
      return DEFAULT_TOKEN_LIMIT;
  }
}
```

### 5.2 壓縮觸發策略

```typescript
// packages/core/src/services/chatCompressionService.ts:40-51
const DEFAULT_COMPRESSION_TOKEN_THRESHOLD = 0.5;  // 50% 觸發壓縮
const COMPRESSION_PRESERVE_THRESHOLD = 0.3;        // 保留最近 30% 歷史
const COMPRESSION_FUNCTION_RESPONSE_TOKEN_BUDGET = 50_000;  // 函式回應預算
```

**觸發條件**：
```
當前 token 使用量 >= tokenLimit(model) × 0.5
  → 即 1M × 0.5 = 512K tokens 時觸發壓縮
```

### 5.3 壓縮分割點演算法

```typescript
// packages/core/src/services/chatCompressionService.ts:59-99
export function findCompressSplitPoint(
  contents: Content[],
  fraction: number,  // 0.3 = 保留最近 30%
): number {
  // 1. 計算每個 content 的字元數
  const charCounts = contents.map(c => JSON.stringify(c).length);
  const totalCharCount = charCounts.reduce((a, b) => a + b, 0);
  const targetCharCount = totalCharCount * fraction;

  // 2. 尋找分割點：累積字元超過 fraction 的第一個 user turn
  let lastSplitPoint = 0;
  let cumulativeCharCount = 0;
  for (let i = 0; i < contents.length; i++) {
    const content = contents[i];
    // 只在 user turn（非 functionResponse）處分割
    if (content.role === 'user' &&
        !content.parts?.some(part => !!part.functionResponse)) {
      if (cumulativeCharCount >= targetCharCount) {
        return i;  // 在此處分割
      }
      lastSplitPoint = i;
    }
    cumulativeCharCount += charCounts[i];
  }

  // 3. 安全檢查：不能在 functionCall 後面切割
  const lastContent = contents[contents.length - 1];
  if (lastContent?.role === 'model' &&
      !lastContent?.parts?.some(part => part.functionCall)) {
    return contents.length;  // 壓縮全部
  }

  return lastSplitPoint;  // 退回到最後一個安全分割點
}
```

**分割點規則**：
1. 只在 `user` turn 處分割（不切斷 model 回應）
2. 不在 `functionResponse` 處分割（不切斷 tool call/response 配對）
3. 如果找不到好的分割點，壓縮到最後一個安全位置

### 5.4 函式回應截斷 — 反向 Token 預算策略

```typescript
// packages/core/src/services/chatCompressionService.ts:120-200
async function truncateHistoryToBudget(
  history: readonly Content[],
  config: Config,
): Promise<Content[]> {
  let functionResponseTokenCounter = 0;
  const truncatedHistory: Content[] = [];

  // 反向迭代：從最新到最舊
  for (let i = history.length - 1; i >= 0; i--) {
    const content = history[i];

    for (let j = content.parts.length - 1; j >= 0; j--) {
      const part = content.parts[j];

      if (part.functionResponse) {
        const contentStr = extractResponseString(part.functionResponse.response);
        const tokens = estimateTokenCountSync([{ text: contentStr }]);

        if (functionResponseTokenCounter + tokens >
            COMPRESSION_FUNCTION_RESPONSE_TOKEN_BUDGET) {
          // 超出預算：截斷此回應
          const { outputFile } = await saveTruncatedToolOutput(
            contentStr,
            part.functionResponse.name ?? 'unknown_tool',
            config.getNextCompressionTruncationId(),
            config.storage.getProjectTempDir(),
          );
          // 保留最後 30 行 + 完整輸出路徑
          part.functionResponse.response = {
            output: formatTruncatedToolOutput(contentStr, outputFile, threshold),
          };
        }
        functionResponseTokenCounter += tokens;
      }
    }
    truncatedHistory.unshift(content);
  }
  return truncatedHistory;
}
```

**策略要點**：
1. **反向迭代**：最新的工具輸出優先保留完整（高保真上下文）
2. **Token 預算 50K**：超過預算的舊工具輸出被截斷
3. **截斷保存**：完整輸出存到臨時檔案，截斷版本含路徑引用
4. **最後 30 行**：截斷時保留尾部內容（通常是結果/錯誤）

### 5.5 壓縮狀態

```typescript
// packages/core/src/core/turn.ts
export enum CompressionStatus {
  COMPRESSED = 1,                        // 成功壓縮
  COMPRESSION_FAILED_INFLATED_TOKEN_COUNT, // 壓縮後反而更大
  COMPRESSION_FAILED_TOKEN_COUNT_ERROR,    // token 計數錯誤
  COMPRESSION_FAILED_EMPTY_SUMMARY,        // 空摘要
  NOOP,                                    // 不需要壓縮
  CONTENT_TRUNCATED,                       // 截斷降級
}
```

### 5.6 壓縮模型選擇

```typescript
// packages/core/src/services/chatCompressionService.ts:101-117
export function modelStringToModelConfigAlias(model: string): string {
  switch (model) {
    case 'gemini-3-pro-preview':     return 'chat-compression-3-pro';
    case 'gemini-3.1-pro-preview':   return 'chat-compression-3-pro';
    case 'gemini-3-flash-preview':   return 'chat-compression-3-flash';
    case 'gemini-2.5-pro':           return 'chat-compression-2.5-pro';
    case 'gemini-2.5-flash':         return 'chat-compression-2.5-flash';
    case 'gemini-2.5-flash-lite':    return 'chat-compression-2.5-flash-lite';
    default:                         return 'chat-compression-default';
  }
}
// 使用 Flash Lite 做壓縮（便宜且快速）
```

### 5.7 上下文溢出預警

```typescript
// packages/core/src/core/turn.ts
export type ServerGeminiContextWindowWillOverflowEvent = {
  type: GeminiEventType.ContextWindowWillOverflow;
  value: {
    estimatedRequestTokenCount: number;
    remainingTokenCount: number;
  };
};
// 在接近上限時提前發出警告事件
```

> **Clawtex 實作建議**：
> 1. clawtex 的 `context_compactor.rs` 應借鏡 Gemini 的**分割點演算法** — 只在 user turn 處切割，不切斷 tool call/response 配對。
> 2. **反向 Token 預算**策略非常巧妙 — 最新工具輸出保留完整，舊的截斷。clawtex 應引入類似機制。
> 3. 截斷工具輸出時**保存完整版本到臨時檔案**是好做法 — LLM 可以用 `file_read` 讀取。
> 4. 使用便宜的 Flash 模型做壓縮（而非用主模型）可節省成本。clawtex 可以用 `gemini-2.5-flash-lite` 做壓縮。
> 5. `CompressionStatus` 的 6 種狀態比 clawtex 的 `Result<(), Error>` 更精細，特別是 `COMPRESSION_FAILED_INFLATED_TOKEN_COUNT`（壓縮後反而更大）。

---

## 6. LLM-Based Loop Detection（深度）

### 6.1 三層偵測架構

```
┌──────────────────────────────────────────────┐
│        Layer 1: Tool Call Loop Detection     │
│  閾值: 5 次相同工具 + 相同參數 (SHA256 hash) │
│  偵測速度: 即時 (O(1))                       │
├──────────────────────────────────────────────┤
│        Layer 2: Content Loop Detection       │
│  閾值: 10 次相同內容 chunk                    │
│  chunk 大小: 50 字元                          │
│  最大歷史: 5000 字元                          │
│  特殊: 程式碼區塊內不偵測                     │
├──────────────────────────────────────────────┤
│        Layer 3: LLM-Based Loop Detection     │
│  啟動: 30 turns 後                            │
│  間隔: 動態 (5-15 turns)                      │
│  信心閾值: 0.9                                │
│  歷史窗口: 最近 20 turns                      │
│  雙重確認: 使用專用模型                       │
└──────────────────────────────────────────────┘
```

### 6.2 LoopDetectionService 核心

```typescript
// packages/core/src/services/loopDetectionService.ts:29-66
const TOOL_CALL_LOOP_THRESHOLD = 5;    // 工具呼叫重複閾值
const CONTENT_LOOP_THRESHOLD = 10;     // 內容重複閾值
const CONTENT_CHUNK_SIZE = 50;         // 內容 chunk 大小
const MAX_HISTORY_LENGTH = 5000;       // 最大串流歷史

const LLM_LOOP_CHECK_HISTORY_COUNT = 20;  // LLM 檢查的歷史窗口
const LLM_CHECK_AFTER_TURNS = 30;         // 30 turns 後啟動 LLM 檢查
const DEFAULT_LLM_CHECK_INTERVAL = 10;     // 預設檢查間隔
const MIN_LLM_CHECK_INTERVAL = 5;         // 最小間隔（高信心時加頻）
const MAX_LLM_CHECK_INTERVAL = 15;        // 最大間隔（低信心時減頻）
const LLM_CONFIDENCE_THRESHOLD = 0.9;     // 信心閾值

export class LoopDetectionService {
  private lastToolCallKey: string | null = null;
  private toolCallRepetitionCount: number = 0;
  private streamContentHistory = '';
  private contentStats = new Map<string, number[]>();
  private turnsInCurrentPrompt = 0;
  private llmCheckInterval = DEFAULT_LLM_CHECK_INTERVAL;  // 動態調整
  private lastCheckTurn = 0;
  private disabledForSession = false;
}
```

### 6.3 Layer 1 — 工具呼叫重複偵測

```typescript
// packages/core/src/services/loopDetectionService.ts:175-179, 308-320
private getToolCallKey(toolCall: { name: string; args: object }): string {
  const argsString = JSON.stringify(toolCall.args);
  const keyString = `${toolCall.name}:${argsString}`;
  return createHash('sha256').update(keyString).digest('hex');
}

private checkToolCallLoop(toolCall: { name: string; args: object }): boolean {
  const key = this.getToolCallKey(toolCall);
  if (this.lastToolCallKey === key) {
    this.toolCallRepetitionCount++;
  } else {
    this.lastToolCallKey = key;
    this.toolCallRepetitionCount = 1;
  }
  return this.toolCallRepetitionCount >= TOOL_CALL_LOOP_THRESHOLD;
}
```

**設計要點**：
- 使用 SHA256 hash 而非直接比較（節省記憶體，處理大參數）
- 只追蹤**連續**重複（不同工具呼叫會重置計數）
- 閾值 5 = 同一個 tool+args 連續出現 5 次才觸發

### 6.4 Layer 2 — 內容串流重複偵測

```typescript
// packages/core/src/services/loopDetectionService.ts:322-399
private checkContentLoop(content: string): boolean {
  // 偵測 markdown 結構（程式碼區塊、表格、列表等）
  const numFences = (content.match(/```/g) ?? []).length;
  const hasTable = /(^|\n)\s*(\|.*\||[|+-]{3,})/.test(content);
  const hasListItem = /(^|\n)\s*[*-+]\s/.test(content) || /(^|\n)\s*\d+\.\s/.test(content);
  const hasHeading = /(^|\n)#+\s/.test(content);

  // 遇到結構性元素時重置（避免誤判）
  if (numFences || hasTable || hasListItem || hasHeading || hasBlockquote || isDivider) {
    this.resetContentTracking();
  }

  // 程式碼區塊內不偵測
  this.inCodeBlock = numFences % 2 === 0 ? this.inCodeBlock : !this.inCodeBlock;
  if (this.inCodeBlock) return false;

  // 追加到歷史
  this.streamContentHistory += content;
  this.truncateAndUpdate();  // 超過 5000 字元時截斷
  return this.analyzeContentChunksForLoop();
}
```

**截斷時的索引調整**：

```typescript
// packages/core/src/services/loopDetectionService.ts:375-399
private truncateAndUpdate(): void {
  if (this.streamContentHistory.length <= MAX_HISTORY_LENGTH) return;

  const truncationAmount = this.streamContentHistory.length - MAX_HISTORY_LENGTH;
  this.streamContentHistory = this.streamContentHistory.slice(truncationAmount);
  this.lastContentIndex = Math.max(0, this.lastContentIndex - truncationAmount);

  // 調整所有儲存的 chunk 索引
  for (const [hash, oldIndices] of this.contentStats.entries()) {
    const adjustedIndices = oldIndices
      .map(index => index - truncationAmount)
      .filter(index => index >= 0);
    if (adjustedIndices.length > 0) {
      this.contentStats.set(hash, adjustedIndices);
    } else {
      this.contentStats.delete(hash);
    }
  }
}
```

### 6.5 Layer 3 — LLM 迴圈偵測

```typescript
// packages/core/src/services/loopDetectionService.ts:258-306
async turnStarted(signal: AbortSignal): Promise<LoopDetectionResult> {
  this.turnsInCurrentPrompt++;

  // 30 turns 後啟動，每 10 turns 檢查一次（動態調整）
  if (this.turnsInCurrentPrompt >= LLM_CHECK_AFTER_TURNS &&
      this.turnsInCurrentPrompt - this.lastCheckTurn >= this.llmCheckInterval) {
    this.lastCheckTurn = this.turnsInCurrentPrompt;
    const { isLoop, analysis, confirmedByModel } =
      await this.checkForLoopWithLLM(signal);
    if (isLoop) {
      this.loopDetected = true;
      this.lastLoopType = LoopType.LLM_DETECTED_LOOP;
      return { count: ++this.detectedCount, type, detail: analysis, confirmedByModel };
    }
  }
  return { count: 0 };
}
```

### 6.6 LLM 迴圈偵測系統提示

```typescript
// packages/core/src/services/loopDetectionService.ts:68-101
const LOOP_DETECTION_SYSTEM_PROMPT = `You are a diagnostic agent that determines
whether a conversational AI assistant is stuck in an unproductive loop.

## What constitutes an unproductive state
An unproductive state requires BOTH:
1. Repetitive pattern over at least 5 consecutive model actions
2. No net change or forward progress toward the user's goal

Patterns to look for:
- Alternating cycles with no net effect
- Semantic repetition with identical outcomes
- Stuck reasoning

## What is NOT an unproductive state
- Cross-file batch operations (different paths = distinct work)
- Incremental same-file edits (different line ranges)
- Sequential processing (different files)
- Retry with variation

## Argument analysis (critical)
Compare ARGUMENTS, not just tool names:
- Different file paths → different targets → NOT a loop
- Different line numbers → distinct edits → NOT a loop`;
```

### 6.7 LLM 輸出結構化

```typescript
// packages/core/src/services/loopDetectionService.ts:103-118
const LOOP_DETECTION_SCHEMA = {
  type: 'object',
  properties: {
    unproductive_state_analysis: {
      type: 'string',
      description: 'Your reasoning on if the conversation is looping.',
    },
    unproductive_state_confidence: {
      type: 'number',
      description: 'Confidence between 0.0 and 1.0.',
    },
  },
  required: ['unproductive_state_analysis', 'unproductive_state_confidence'],
};
// 信心 >= 0.9 才判定為迴圈
```

### 6.8 動態檢查間隔

```
confidence >= 0.7 → MIN_LLM_CHECK_INTERVAL (5 turns, 更頻繁)
confidence <= 0.3 → MAX_LLM_CHECK_INTERVAL (15 turns, 減頻)
otherwise         → DEFAULT_LLM_CHECK_INTERVAL (10 turns)
```

### 6.9 LoopType 分類

```typescript
// packages/core/src/telemetry/types.ts
export enum LoopType {
  CONSECUTIVE_IDENTICAL_TOOL_CALLS = 'tool_call_loop',
  CONTENT_CHANTING_LOOP = 'content_loop',
  LLM_DETECTED_LOOP = 'llm_detected_loop',
}
```

### 6.10 事件驅動偵測流程

```typescript
// packages/core/src/services/loopDetectionService.ts:186-246
addAndCheck(event: ServerGeminiStreamEvent): LoopDetectionResult {
  if (this.disabledForSession) return { count: 0 };
  if (this.loopDetected) return { count: this.detectedCount, ... };

  switch (event.type) {
    case GeminiEventType.ToolCallRequest:
      this.resetContentTracking();  // 工具呼叫重置內容追蹤
      isLoop = this.checkToolCallLoop(event.value);
      break;
    case GeminiEventType.Content:
      isLoop = this.checkContentLoop(event.value);
      break;
  }

  if (isLoop) {
    this.loopDetected = true;
    this.detectedCount++;
    logLoopDetected(this.config, new LoopDetectedEvent(...));
  }
  return isLoop ? { count, type, detail } : { count: 0 };
}
```

> **Clawtex 實作建議**：
> 1. clawtex 的 `loop_detection.rs` 目前只有簡單的重複檢測。應升級為三層架構：
>    - **Layer 1**：SHA256(tool+args) 連續重複計數（已有類似機制）
>    - **Layer 2**：新增串流內容重複偵測（content hash chunk）
>    - **Layer 3**：新增 LLM 二次確認（30 turns 後啟動）
> 2. **LLM 迴圈偵測的系統提示**非常精確 — 特別區分了「看起來重複但實際有進展」的情況。直接參考使用。
> 3. **動態檢查間隔**（根據信心調整頻率）是好的優化 — 避免在低可疑度時浪費 LLM 呼叫。
> 4. `disableForSession()` 允許使用者手動關閉偵測 — clawtex 也應提供此選項。
> 5. 程式碼區塊排除（`inCodeBlock`）避免程式碼結構被誤判為重複 — 重要的細節。

---

## 7. MCP Auth Providers（深度）

### 7.1 MCP 認證架構

```
┌──────────────────────────────────────────────┐
│              MCP 認證提供者架構                │
│                                              │
│  McpAuthProvider (介面)                       │
│    ├── GoogleCredentialProvider               │
│    │   └── Google ADC (Application Default)   │
│    ├── MCPOAuthClientProvider                 │
│    │   └── 標準 OAuth2 + PKCE 流程            │
│    ├── SAImpersonationProvider                │
│    │   └── Service Account 模擬              │
│    └── OAuthProvider (通用)                   │
│        └── 通用 OAuth2 流程                   │
│                                              │
│  Token Storage                               │
│    ├── KeychainTokenStorage                   │
│    │   └── 系統 keyring                       │
│    ├── FileTokenStorage                       │
│    │   └── 檔案系統（加密）                    │
│    └── HybridTokenStorage                     │
│        └── keychain + file 混合               │
└──────────────────────────────────────────────┘
```

### 7.2 McpAuthProvider 介面

```typescript
// packages/core/src/mcp/auth-provider.ts
export interface McpAuthProvider {
  readonly redirectUrl: string | URL;
  readonly clientMetadata: OAuthClientMetadata;
  clientInformation(): OAuthClientInformation | undefined;
  saveClientInformation(info: OAuthClientInformation): void;
  tokens(): OAuthTokens | undefined | Promise<OAuthTokens | undefined>;
  saveTokens?(tokens: OAuthTokens): void;
  redirectToAuthorization?(url: URL): Promise<void>;
  saveCodeVerifier?(verifier: string): void;
  codeVerifier?(): string;
  state?(): string;
}
```

### 7.3 GoogleCredentialProvider

```typescript
// packages/core/src/mcp/google-auth-provider.ts:19-100
const ALLOWED_HOSTS = [/^.+\.googleapis\.com$/, /^(.*\.)?luci\.app$/];

export class GoogleCredentialProvider implements McpAuthProvider {
  private readonly auth: GoogleAuth;
  private cachedToken?: OAuthTokens;
  private tokenExpiryTime?: number;

  constructor(private readonly config?: MCPServerConfig) {
    const url = this.config?.url || this.config?.httpUrl;
    // 安全檢查：只允許 Google 域名
    const hostname = new URL(url).hostname;
    if (!ALLOWED_HOSTS.some(pattern => pattern.test(hostname))) {
      throw new Error(`Host "${hostname}" is not allowed for Google Credential provider.`);
    }

    // 需要明確指定 scopes
    const scopes = this.config?.oauth?.scopes;
    if (!scopes || scopes.length === 0) {
      throw new Error('Scopes must be provided for Google Credentials provider');
    }
    this.auth = new GoogleAuth({ scopes });
  }

  async tokens(): Promise<OAuthTokens | undefined> {
    // 5 分鐘緩衝：在 token 到期前 5 分鐘刷新
    if (this.cachedToken && this.tokenExpiryTime &&
        Date.now() < this.tokenExpiryTime - FIVE_MIN_BUFFER_MS) {
      return this.cachedToken;
    }

    this.cachedToken = undefined;
    this.tokenExpiryTime = undefined;

    const client = await this.auth.getClient();
    const accessTokenResponse = await client.getAccessToken();

    if (!accessTokenResponse.token) {
      coreEvents.emitFeedback('error', 'Failed to get access token from Google ADC');
      return undefined;
    }

    return { access_token: accessTokenResponse.token, token_type: 'Bearer' };
  }
}
```

**安全特性**：
- 域名白名單（只允許 `*.googleapis.com` 和 `*.luci.app`）
- 5 分鐘 token 刷新緩衝
- 強制要求 scopes 配置

### 7.4 MCPOAuthClientProvider

```typescript
// packages/core/src/mcp/mcp-oauth-provider.ts:29-97
export class MCPOAuthClientProvider implements OAuthClientProvider {
  private _clientInformation?: OAuthClientInformation;
  private _tokens?: OAuthTokens;
  private _codeVerifier?: string;
  private _cbServer?: CallbackServer;  // 本地回呼伺服器

  constructor(
    private readonly _redirectUrl: string | URL,
    private readonly _clientMetadata: OAuthClientMetadata,
    private readonly _state?: string,
    private readonly _onRedirect: (url: URL) => void = (url) => {
      debugLogger.log(`Redirect to: ${url.toString()}`);
    },
  ) {}

  // PKCE 流程支援
  saveCodeVerifier(codeVerifier: string): void {
    this._codeVerifier = codeVerifier;
  }

  codeVerifier(): string {
    if (!this._codeVerifier) throw new Error('No code verifier saved');
    return this._codeVerifier;
  }

  async redirectToAuthorization(authorizationUrl: URL): Promise<void> {
    this._onRedirect(authorizationUrl);
  }

  // 本地回呼伺服器管理
  saveCallbackServer(server: CallbackServer): void {
    this._cbServer = server;
  }
  getSavedCallbackServer(): CallbackServer | undefined {
    return this._cbServer;
  }
}
```

### 7.5 SA Impersonation Provider

```typescript
// packages/core/src/mcp/sa-impersonation-provider.ts (概念化)
export class SAImpersonationProvider implements McpAuthProvider {
  // 使用 Service Account 模擬功能
  // 適用於企業環境中的 MCP 伺服器認證
  // 透過 Google Cloud IAM API 產生 access token
}
```

### 7.6 Token Storage 階層

```typescript
// packages/core/src/mcp/token-storage/

// 基底介面
export interface TokenStorage {
  get(key: string): Promise<OAuthTokens | undefined>;
  set(key: string, tokens: OAuthTokens): Promise<void>;
  delete(key: string): Promise<void>;
}

// Keychain (最安全)
export class KeychainTokenStorage implements TokenStorage {
  // 使用系統 keyring (macOS Keychain, Windows Credential Manager, Linux libsecret)
}

// File (降級方案)
export class FileTokenStorage implements TokenStorage {
  // 使用加密檔案儲存
  // 路徑: ~/.gemini/mcp-tokens/
}

// Hybrid (預設)
export class HybridTokenStorage implements TokenStorage {
  // 優先使用 keychain，失敗時降級到 file
  // 自動遷移 file → keychain
}
```

### 7.7 Auth Provider Factory

```typescript
// packages/core/src/agents/auth-provider/factory.ts
export function createAuthProvider(config: MCPServerConfig): McpAuthProvider {
  const authType = config.oauth?.authProvider;
  switch (authType) {
    case 'google':
      return new GoogleCredentialProvider(config);
    case 'oauth2':
      return new MCPOAuthClientProvider(redirectUrl, clientMetadata);
    case 'sa-impersonation':
      return new SAImpersonationProvider(config);
    case 'api-key':
      return new ApiKeyProvider(config);
    case 'http':
      return new HttpProvider(config);
    default:
      // 自動偵測
      if (config.url?.match(/googleapis\.com/)) {
        return new GoogleCredentialProvider(config);
      }
      return new MCPOAuthClientProvider(redirectUrl, clientMetadata);
  }
}
```

> **Clawtex 實作建議**：
> 1. clawtex 的 MCP 客戶端目前只支援 stdio 傳輸，沒有認證。應引入 auth provider 架構。
> 2. 對於連接 Google MCP 伺服器（如 Firebase、Vertex AI），`GoogleCredentialProvider` 可直接移植。
> 3. **Token Storage** 的三層策略（keychain → file → hybrid）比 clawtex 的 ChaCha20-Poly1305 加密更標準化。
> 4. **域名白名單**是重要安全措施 — 防止 MCP 配置被濫用為 credential 竊取載體。
> 5. 建議在 `agents.toml` 的 `[mcp_servers]` 區段加入 `auth_type` 欄位：
>    ```toml
>    [mcp_servers.firebase]
>    command = "firebase-mcp"
>    auth_type = "google"
>    scopes = ["https://www.googleapis.com/auth/firebase"]
>    ```

---

## 8. Agent Loop 三層架構

### 8.1 三層設計

```
Layer 3: CoreToolScheduler (最上層)
  └── 管理工具批次、確認、並行、尾呼叫
      │
Layer 2: GeminiChat (中間層)
  └── 對話歷史、串流重試、Hook 整合、模型選擇
      │
Layer 1: Turn (最底層)
  └── 單次 LLM 串流、事件產生
```

### 8.2 Turn — 串流抽象

```typescript
// packages/core/src/core/turn.ts
export class Turn {
  async *run(
    modelConfigKey: ModelConfigKey,
    req: PartListUnion,
    signal: AbortSignal,
  ): AsyncGenerator<ServerGeminiStreamEvent> {
    const responseStream = await this.chat.sendMessageStream(
      modelConfigKey, req, this.prompt_id, signal, role, displayContent
    );
    for await (const streamEvent of responseStream) {
      if (signal?.aborted) {
        yield { type: GeminiEventType.UserCancelled };
        return;
      }
      // 解析回應
      const text = getResponseText(resp);
      if (text) yield { type: GeminiEventType.Content, value: text, traceId };

      const functionCalls = resp.functionCalls ?? [];
      for (const fnCall of functionCalls) {
        yield { type: GeminiEventType.ToolCallRequest, value: fnCall };
      }
    }
  }
}
```

### 8.3 GeminiChat — 對話管理

```typescript
// packages/core/src/core/geminiChat.ts
export class GeminiChat {
  private history: Content[] = [];  // 完整歷史

  async sendMessageStream(
    modelConfigKey, message, prompt_id, signal, role, displayContent
  ): Promise<AsyncGenerator<StreamEvent>> {
    // 1. 等待前一個訊息完成
    await this.sendPromise;

    // 2. 記錄使用者輸入
    this.chatRecordingService.recordMessage({ model, type: 'user', content });

    // 3. 加入歷史
    this.history.push(userContent);

    // 4. 串流重試迴圈（最多 4 次，linear backoff）
    for (let attempt = 0; attempt < maxAttempts; attempt++) {
      try {
        const stream = await this.makeApiCallAndProcessStream(...);
        for await (const chunk of stream) {
          yield { type: StreamEventType.CHUNK, value: chunk };
        }
        return;
      } catch (error) {
        // 判斷是否可重試 (InvalidStreamError, 網路錯誤)
        // linear backoff: initialDelayMs * (attempt + 1)
      }
    }
  }

  // Curated vs Comprehensive 歷史
  getHistory(curated: boolean = false): readonly Content[] {
    if (curated) {
      // 只保留有效回應（用於 API 請求）
      return this.history.filter(isValid);
    }
    return this.history;  // 含無效回應的完整記錄
  }
}
```

### 8.4 GeminiEventType 完整列舉

```typescript
export enum GeminiEventType {
  Content = 'content',                    // 文字內容
  Thought = 'thought',                    // 思考鏈
  ToolCallRequest = 'tool_call_request',  // 工具呼叫請求
  ToolCallResponse = 'tool_call_response',// 工具呼叫結果
  ToolCallConfirmation = 'confirmation',  // 確認請求
  UserCancelled = 'user_cancelled',       // 使用者取消
  Error = 'error',                        // 錯誤
  ChatCompressed = 'chat_compressed',     // 歷史壓縮
  Finished = 'finished',                  // 完成
  LoopDetected = 'loop_detected',         // 迴圈偵測
  Citation = 'citation',                  // 引用
  Retry = 'retry',                        // 重試
  ContextWindowWillOverflow = 'overflow',  // 上下文溢出
  InvalidStream = 'invalid_stream',       // 無效串流
  ModelInfo = 'model_info',               // 模型資訊
  AgentExecutionStopped = 'stopped',      // Hook 停止
  AgentExecutionBlocked = 'blocked',      // Hook 阻擋
}
```

> **Clawtex 實作建議**：
> 1. clawtex 的 `agent_runtime.rs` 混合了 Turn 和 Chat 的職責。應分離為：
>    - `Turn`：單次 LLM 互動 + 工具執行迴圈
>    - `AgentChat`：歷史管理 + 重試 + 壓縮觸發
> 2. `AsyncGenerator` 事件流在 Rust 中對應 `futures::Stream` — 比 channel 更自然。
> 3. **Curated vs Comprehensive 歷史**是好的設計 — 允許 debug 用完整歷史，API 用精簡歷史。

---

## 9. 模型路由與分類器

### 9.1 策略鏈 (Composite Strategy)

```
CompositeRoutingStrategy
  ├── 1. OverrideStrategy (使用者明確指定 → 最高優先)
  ├── 2. ApprovalModeStrategy (根據審批模式選擇)
  ├── 3. ClassifierStrategy / NumericalClassifierStrategy / GemmaClassifierStrategy
  │      └── 使用小模型分類：flash vs pro
  ├── 4. FallbackStrategy (降級)
  └── 5. DefaultStrategy (Terminal, 必定返回)
```

```typescript
// packages/core/src/routing/routingStrategy.ts
export interface RoutingStrategy {
  readonly name: string;
  route(
    context: RoutingContext,
    config: Config,
    baseLlmClient: BaseLlmClient,
    localLiteRtLmClient: LocalLiteRtLmClient,
  ): Promise<RoutingDecision | null>;
}
```

### 9.2 模型別名

```typescript
// packages/core/src/config/models.ts
export const GEMINI_MODEL_ALIAS_AUTO = 'auto';
export const GEMINI_MODEL_ALIAS_PRO = 'pro';
export const GEMINI_MODEL_ALIAS_FLASH = 'flash';
export const GEMINI_MODEL_ALIAS_FLASH_LITE = 'flash-lite';
```

### 9.3 思考預算

```typescript
export const DEFAULT_THINKING_MODE = 8192;  // 限制思考 token 數
```

> **Clawtex 實作建議**：
> 1. clawtex 的 `ProviderRouter` 已有分類器，可加入 Gemini 的策略鏈模式（Composite Strategy）。
> 2. 思考預算（`thinking_budget`）應傳入支援思考鏈的模型。

---

## 10. 沙箱與安全

### 10.1 沙箱類型

| 平台 | 技術 | 說明 |
|------|------|------|
| macOS | sandbox-exec (Seatbelt) | .sb profile |
| Linux | Docker/Podman 容器 | 完整容器隔離 |
| 所有 | NoopSandboxManager | 僅環境變數消毒 |

### 10.2 環境變數消毒

```typescript
// packages/core/src/services/environmentSanitization.ts
export interface EnvironmentSanitizationConfig {
  allowedEnvironmentVariables: string[];
  blockedEnvironmentVariables: string[];
  enableEnvironmentVariableRedaction: boolean;
}
// 強制啟用，移除潛在的敏感環境變數
```

### 10.3 路徑存取驗證

```typescript
// packages/core/src/tools/shell.ts
const validationError = this.config.validatePathAccess(cwd);
if (validationError) {
  return { error: { type: ToolErrorType.PATH_NOT_IN_WORKSPACE } };
}
```

> **Clawtex 實作建議**：
> 1. clawtex 的 shell tool 應加入環境變數消毒 — 移除 `OPENAI_API_KEY`, `ANTHROPIC_API_KEY` 等敏感變數。
> 2. Docker 沙箱模式可作為 clawtex 的進階安全選項。

---

## 11. 遙測與計費

### 11.1 Clearcut 遙測

```typescript
// packages/core/src/telemetry/clearcut-logger/clearcut-logger.ts
export enum EventNames {
  START_SESSION = 'start_session',
  NEW_PROMPT = 'new_prompt',
  TOOL_CALL = 'tool_call',
  API_REQUEST = 'api_request',
  API_RESPONSE = 'api_response',
  LOOP_DETECTED = 'loop_detected',
  CHAT_COMPRESSION = 'chat_compression',
  // ... 40+ 個事件
}
```

### 11.2 計費

```typescript
// packages/core/src/billing/billing.ts
export type OverageStrategy = 'ask' | 'always' | 'never';

export const OVERAGE_ELIGIBLE_MODELS = new Set([
  'gemini-3-pro-preview',
  'gemini-3.1-pro-preview',
  'gemini-3-flash-preview',
]);
```

---

## 12. 子代理與擴展系統

### 12.1 子代理

```
packages/core/src/agents/
├── generalist-agent.ts        # 通用助手
├── codebase-investigator.ts   # 程式碼庫調查
├── cli-help-agent.ts          # CLI 幫助
├── browser/                   # 瀏覽器代理 (Playwright)
│   ├── browserAgentFactory.ts
│   ├── browserManager.ts
│   └── mcpToolWrapper.ts
├── agent-scheduler.ts         # 代理排程
├── registry.ts                # 代理註冊
├── local-executor.ts          # 本地執行
└── remote-invocation.ts       # 遠端呼叫
```

### 12.2 技能系統

```typescript
// packages/core/src/skills/skillManager.ts
export class SkillManager {
  async discoverSkills(storage, extensions, isTrusted) {
    // 優先順序（低→高）:
    // 1. 內建技能
    // 2. Extension 技能
    // 3. 使用者技能 (~/.gemini/skills/)
    // 4. 工作區技能 (.gemini/skills/) — 僅受信任
  }
}
```

### 12.3 SDK

```typescript
// packages/sdk/src/agent.ts
export class GeminiCliAgent {
  session(options?: { sessionId?: string }): GeminiCliSession;
  async resumeSession(sessionId: string): Promise<GeminiCliSession>;
}
```

### 12.4 A2A (Agent-to-Agent)

```
packages/a2a-server/
// Agent-to-Agent 通訊協定
// 允許多個 Gemini CLI 實例協作
```

> **Clawtex 實作建議**：
> 1. clawtex 的 `delegate` 工具已支援子代理，但缺少像 Gemini 的**代理註冊+排程**機制。
> 2. A2A 協定可用於 clawtex 的 cluster 節點間通訊。

---

## 13. 錯誤處理與韌性設計

### 13.1 串流重試

```
GeminiChat.sendMessageStream():
  attempt 1 → 失敗 (InvalidStreamError)
  delay(initialDelayMs × 1) → attempt 2
  delay(initialDelayMs × 2) → attempt 3
  delay(initialDelayMs × 3) → attempt 4
  全部失敗 → 拋出最後一個錯誤
```

### 13.2 壓縮降級

```
壓縮嘗試:
  1. 成功 → COMPRESSED
  2. 壓縮後更大 → COMPRESSION_FAILED_INFLATED → 截斷降級
  3. Token 計數錯誤 → COMPRESSION_FAILED_TOKEN_COUNT → 截斷降級
  4. 空摘要 → COMPRESSION_FAILED_EMPTY → 截斷降級
```

### 13.3 模型降級

```
FallbackStrategy:
  Pro 不可用 → Flash
  Flash 不可用 → Flash Lite
  Flash Lite 不可用 → 錯誤
```

### 13.4 清理超時

```typescript
// packages/cli/index.ts
const cleanupTimeout = setTimeout(() => {
  writeToStderr('Cleanup timed out, forcing exit...\n');
  process.exit(1);
}, 5000);  // 5 秒清理超時
```

---

## 14. 效能分析

### 14.1 記憶體管理

| 項目 | 策略 |
|------|------|
| Node.js 堆積 | 系統記憶體 50% |
| 串流歷史 | 5000 字元上限 |
| 函式回應 | 50K token 預算 |
| WeakMap | 防止 MessageBus 訂閱洩漏 |

### 14.2 串流效能

- AsyncGenerator 提供背壓控制
- 增量文字累積（不重建完整字串）
- 程式碼區塊排除減少誤判計算

### 14.3 壓縮效能

- 使用 Flash Lite（便宜且快速）做壓縮
- 反向 Token 預算避免重新計算
- 分割點演算法 O(N) 單遍掃描

---

## 15. Clawtex 差距總覽與實作路線圖

### 15.1 差距矩陣

| 功能 | Gemini CLI | Clawtex | 差距 | 優先級 |
|------|-----------|---------|------|--------|
| Builder+Invoker 工具 | 完整 | 直接 execute | **高** | P1 |
| Hooks 系統 | 13+ 事件 | 無 | **高** | P1 |
| 1M 上下文壓縮 | 分割點+截斷+預算 | 基本壓縮 | **中** | P1 |
| LLM 迴圈偵測 | 三層+動態間隔 | 簡單重複 | **中** | P2 |
| MCP Auth | 5 種 provider | 無認證 | **中** | P2 |
| MessageBus 確認 | 解耦 | Telegram 直接 | **中** | P1 |
| 策略鏈路由 | Composite | 單一路由 | **低** | P3 |
| 環境變數消毒 | 強制 | 無 | **中** | P1 |
| 子代理排程 | Registry+Scheduler | delegate 工具 | **低** | P3 |
| 工具 Kind 分類 | 12 種 | 無 | **中** | P2 |
| Curated/Comprehensive | 雙歷史 | 單歷史 | **低** | P3 |
| 思考預算 | 8192 token | 無 | **低** | P3 |

### 15.2 建議實作順序

```
Sprint 1 (1 週): 工具系統升級
  ├── P1: Builder+Invoker 分離 → src/tools/mod.rs
  ├── P1: MessageBus 確認解耦 → src/approval.rs
  └── P1: 環境變數消毒 → src/tools/shell.rs

Sprint 2 (1 週): 上下文與偵測
  ├── P1: 壓縮分割點演算法 → src/context_compactor.rs
  ├── P1: Hooks 基礎架構 → src/hooks/ (新模組)
  └── P2: LLM 迴圈偵測升級 → src/loop_detection.rs

Sprint 3 (1 週): 安全與認證
  ├── P2: MCP Auth Provider → src/mcp/auth.rs (新)
  ├── P2: 工具 Kind 分類 → src/tools/mod.rs
  └── P3: 策略鏈路由 → src/providers/router.rs
```

---

## 16. 工具輸出遮罩服務（深度）

### 16.1 Hybrid Backward Scanned FIFO 演算法

ToolOutputMaskingService 實現了一個精密的上下文窗口效率管理系統，使用「混合反向掃描 FIFO」演算法平衡上下文相關性與 token 節省。

```typescript
// packages/core/src/services/toolOutputMaskingService.ts:25-29
export const DEFAULT_TOOL_PROTECTION_THRESHOLD = 50000;    // 保護最新 50K token
export const DEFAULT_MIN_PRUNABLE_TOKENS_THRESHOLD = 30000; // 至少 30K 可修剪才觸發
export const DEFAULT_PROTECT_LATEST_TURN = true;
export const MASKING_INDICATOR_TAG = 'tool_output_masked';
export const TOOL_OUTPUTS_DIR = 'tool-outputs';
```

### 16.2 免除遮罩的工具

```typescript
// packages/core/src/services/toolOutputMaskingService.ts:37-43
const EXEMPT_TOOLS = new Set([
  ACTIVATE_SKILL_TOOL_NAME,     // 技能啟動結果永遠保留
  MEMORY_TOOL_NAME,             // 記憶存取結果永遠保留
  ASK_USER_TOOL_NAME,           // 使用者互動永遠保留
  ENTER_PLAN_MODE_TOOL_NAME,    // 計畫模式結果永遠保留
  EXIT_PLAN_MODE_TOOL_NAME,     // 計畫模式結果永遠保留
]);
```

**設計原理**：這些工具的輸出是「高信號」的 — 它們定義了對話的核心意圖和邏輯，如果被遮罩，模型很難恢復。相比之下，shell 日誌和檔案內容可以透過重新讀取來恢復。

### 16.3 反向掃描與保護窗口

```typescript
// packages/core/src/services/toolOutputMaskingService.ts:92-149
// 步驟 1: 決定掃描起點（如果保護最新回合，跳過最後一個訊息）
const scanStartIdx = maskingConfig.protectLatestTurn
  ? history.length - 2
  : history.length - 1;

// 步驟 2: 反向掃描，識別可修剪的工具輸出
for (let i = scanStartIdx; i >= 0; i--) {
  const content = history[i];
  for (let j = parts.length - 1; j >= 0; j--) {
    const part = parts[j];
    if (!part.functionResponse) continue;           // 只看工具回應
    if (EXEMPT_TOOLS.has(part.functionResponse.name)) continue;  // 跳過免除工具
    if (this.isAlreadyMasked(toolOutputContent)) continue;       // 跳過已遮罩

    if (!protectionBoundaryReached) {
      cumulativeToolTokens += partTokens;
      if (cumulativeToolTokens > toolProtectionThreshold) {
        protectionBoundaryReached = true;
        // 跨越保護邊界的 part 也是可修剪的
        prunableParts.push({ contentIndex: i, partIndex: j, ... });
      }
    } else {
      prunableParts.push({ contentIndex: i, partIndex: j, ... });
    }
  }
}
```

**遮罩觸發條件流程圖**：

```
歷史中的工具輸出
  │
  ├── 反向掃描（從最新到最舊）
  │     │
  │     ├── 最新 50K token 工具輸出 → 保護（不遮罩）
  │     │
  │     └── 超過 50K 後的工具輸出 → 標記為「可修剪」
  │           │
  │           ├── 可修剪總量 < 30K → 不觸發遮罩（效益不足）
  │           │
  │           └── 可修剪總量 >= 30K → 批量遮罩所有可修剪部分
  │                 │
  │                 ├── 內容寫入磁碟（tool-outputs/ 目錄）
  │                 └── 原位替換為 <tool_output_masked/> 標籤
```

**關鍵數字**：遮罩只在約 80K token 的可修剪工具輸出存在時才開始（50K 保護 + 30K 緩衝）。

> **Clawtex 實作建議**：
> 1. clawtex 的 `context_optimizer.rs` 使用全局截斷 — 應改為「保護窗口 + 可修剪」分層策略。
> 2. 免除遮罩的工具集合 — clawtex 的 `memory_store`/`memory_recall` 工具輸出也不應被壓縮。
> 3. 工具輸出寫入磁碟 + 原位替換標籤 — 允許需要時重新載入完整輸出，clawtex 可利用 `~/.clawtex/workspace/tool-outputs/`。
> 4. 批量觸發門檻（30K）避免頻繁小量遮罩 — clawtex 應引入類似的最小批量門檻。

---

## 17. 執行生命週期服務（深度）

### 17.1 ExecutionLifecycleService 架構

```typescript
// packages/core/src/services/executionLifecycleService.ts:89-108
const NON_PROCESS_EXECUTION_ID_START = 2_000_000_000;

export class ExecutionLifecycleService {
  private static readonly EXIT_INFO_TTL_MS = 5 * 60 * 1000;  // 5 分鐘退出資訊 TTL
  private static nextExecutionId = NON_PROCESS_EXECUTION_ID_START;

  private static activeExecutions = new Map<number, ManagedExecutionState>();
  private static activeResolvers = new Map<number, (result: ExecutionResult) => void>();
  private static activeListeners = new Map<number, Set<(event: ExecutionOutputEvent) => void>>();
  private static exitedExecutionInfo = new Map<number, { exitCode: number; signal?: number }>();
}
```

**核心設計特點**：
- 全 `static` 方法 — 全局唯一的執行管理器（Singleton 模式）
- `nextExecutionId` 從 20 億開始 — 避免與實際 PID 衝突
- `exitedExecutionInfo` 有 5 分鐘 TTL — 防止記憶體洩漏

### 17.2 執行方法抽象

```typescript
// packages/core/src/services/executionLifecycleService.ts:9-14
export type ExecutionMethod =
  | 'lydell-node-pty'     // lydell 維護的 node-pty fork（效能最佳）
  | 'node-pty'            // 原版 node-pty（終端模擬）
  | 'child_process'       // Node.js 內建（無 PTY）
  | 'remote_agent'        // 遠端代理執行
  | 'none';               // 虛擬執行（測試用）
```

### 17.3 執行輸出事件

```typescript
// packages/core/src/services/executionLifecycleService.ts:33-49
export type ExecutionOutputEvent =
  | { type: 'data'; chunk: string | AnsiOutput }    // 文字資料
  | { type: 'binary_detected' }                     // 偵測到二進位輸出
  | { type: 'binary_progress'; bytesReceived: number } // 二進位傳輸進度
  | { type: 'exit'; exitCode: number | null; signal: number | null }; // 退出
```

**二進位偵測**是一個重要的安全功能 — 防止模型嘗試解析二進位 blob 並耗盡上下文窗口。

### 17.4 取消與退出處理

```typescript
// packages/core/src/services/executionLifecycleService.ts:142-157
private static createAbortedResult(
  executionId: number,
  execution: ManagedExecutionState,
): ExecutionResult {
  const output = execution.getBackgroundOutput?.() ?? execution.output;
  return {
    rawOutput: Buffer.from(output, 'utf8'),
    output,
    exitCode: 130,   // 標準 SIGINT 退出碼
    signal: null,
    error: new Error('Operation cancelled by user.'),
    aborted: true,
    pid: executionId,
    executionMethod: execution.executionMethod,
  };
}
```

### 17.5 退出資訊 TTL 清理

```typescript
// packages/core/src/services/executionLifecycleService.ts:112-124
private static storeExitInfo(executionId: number, exitCode: number, signal?: number): void {
  this.exitedExecutionInfo.set(executionId, { exitCode, signal });
  setTimeout(() => {
    this.exitedExecutionInfo.delete(executionId);
  }, this.EXIT_INFO_TTL_MS).unref();  // .unref() 確保 timer 不阻止程序退出
}
```

`.unref()` 是一個關鍵細節 — 沒有它，5 分鐘的 timer 會阻止 Node.js 進程正常退出。

> **Clawtex 實作建議**：
> 1. clawtex 的 shell tool 缺乏統一的執行生命週期管理 — 應建立類似的 `ExecutionManager`。
> 2. 二進位偵測 — clawtex 的 shell tool 應檢測二進位輸出並截斷，避免模型浪費 context。
> 3. 背景執行支援 — clawtex 可利用此模式實現長時間運行的命令（如 `cargo build`）的背景執行。
> 4. 退出碼 130 (SIGINT) 的標準化 — clawtex 應統一使用 Unix 信號退出碼。

---

## 18. 上下文記憶體管理（深度）

### 18.1 ContextManager — 三層記憶體

```typescript
// packages/core/src/services/contextManager.ts:21-31
export class ContextManager {
  private readonly loadedPaths: Set<string> = new Set();
  private readonly loadedFileIdentities: Set<string> = new Set();
  private readonly config: Config;
  private globalMemory: string = '';       // Tier 1: 全域記憶（~/.gemini/GEMINI.md）
  private extensionMemory: string = '';    // Tier 2: 擴充套件記憶
  private projectMemory: string = '';      // Tier 3: 專案記憶（工作目錄的 GEMINI.md）
}
```

### 18.2 記憶體發現（三路並行）

```typescript
// packages/core/src/services/contextManager.ts:47-61
private async discoverMemoryPaths() {
  const [global, extension, project] = await Promise.all([
    getGlobalMemoryPaths(),                                    // Tier 1
    Promise.resolve(getExtensionMemoryPaths(extensionLoader)), // Tier 2
    this.config.isTrustedFolder()                              // Tier 3（受信任才載入）
      ? getEnvironmentMemoryPaths([...directories])
      : Promise.resolve([]),
  ]);
  return { global, extension, project };
}
```

**安全設計**：
- 專案記憶（Tier 3）只在信任資料夾中載入 — 防止惡意專案注入指令
- 使用 `Promise.all` 並行發現三層記憶體路徑

### 18.3 JIT 子目錄記憶體（Tier 4）

```typescript
// packages/core/src/services/contextManager.ts:129-160
async discoverContext(accessedPath: string, trustedRoots: string[]): Promise<string> {
  if (!this.config.isTrustedFolder()) return '';
  const result = await loadJitSubdirectoryMemory(
    accessedPath,
    trustedRoots,
    this.loadedPaths,
    this.loadedFileIdentities,  // 檔案身份去重（處理大小寫不敏感的檔案系統）
  );
  if (result.files.length === 0) return '';
  this.markAsLoaded(newFilePaths);
  for (const identity of result.fileIdentities) {
    this.loadedFileIdentities.add(identity);
  }
  return concatenateInstructions(result.files, this.config.getWorkingDir());
}
```

**JIT（Just-In-Time）記憶體**：
- 當工具存取某個路徑時，動態載入該路徑往上到專案根的所有 GEMINI.md
- 使用 `loadedFileIdentities` 去重 — 處理 Windows 上的大小寫不敏感檔案系統
- 結果透過 `concatenateInstructions` 合併為單一字串

### 18.4 記憶體變更事件

```typescript
// packages/core/src/services/contextManager.ts:162-166
private emitMemoryChanged(): void {
  coreEvents.emit(CoreEvent.MemoryChanged, {
    fileCount: this.loadedPaths.size,
  });
}
```

使用事件匯流排通知 UI 層記憶體已更新 — 可用於顯示載入狀態或計數。

> **Clawtex 實作建議**：
> 1. clawtex 目前只有一層記憶體（`memory_store`/`memory_recall`） — 應實現三層分層記憶（全域 + 專案 + JIT）。
> 2. 信任資料夾機制 — clawtex 的 workspace sandbox 已有類似概念，但專案記憶只在信任資料夾中載入的模式尚未實現。
> 3. 檔案身份去重 — Windows 上 `C:\Foo\bar.md` 和 `c:\foo\Bar.md` 是同一個檔案，clawtex 的 glob/content_search 工具應處理此情況。
> 4. JIT 子目錄記憶可用於 clawtex 的 hands — 每個 hand 的子目錄可以有自己的指令檔。

---

## 19. Session 摘要服務（深度）

### 19.1 SessionSummaryService 設計

```typescript
// packages/core/src/services/sessionSummaryService.ts:47-48
export class SessionSummaryService {
  constructor(private readonly baseLlmClient: BaseLlmClient) {}
}
```

使用 Gemini Flash Lite 生成一句話摘要 — 最大 80 字元。

### 19.2 滑動窗口選取

```typescript
// packages/core/src/services/sessionSummaryService.ts:74-86
let relevantMessages: MessageRecord[];
if (filteredMessages.length <= maxMessages) {
  relevantMessages = filteredMessages;
} else {
  // Sliding window: take the first and last messages.
  const firstWindowSize = Math.ceil(maxMessages / 2);
  const lastWindowSize = Math.floor(maxMessages / 2);
  const firstMessages = filteredMessages.slice(0, firstWindowSize);
  const lastMessages = filteredMessages.slice(-lastWindowSize);
  relevantMessages = firstMessages.concat(lastMessages);
}
```

**設計原理**：
- 預設最多 20 條訊息
- 使用 first + last 滑動窗口 — 保留開頭（目的）和結尾（最終狀態）
- 每條訊息截斷到 500 字元 — 防止 token 超限

### 19.3 超時與優雅降級

```typescript
// packages/core/src/services/sessionSummaryService.ts:109-161
const abortController = new AbortController();
const timeoutId = setTimeout(() => abortController.abort(), timeout);  // 預設 5 秒

try {
  const response = await this.baseLlmClient.generateContent({
    modelConfigKey: { model: 'summarizer-default' },
    contents,
    abortSignal: abortController.signal,
    promptId: 'session-summary-generation',
    role: LlmRole.UTILITY_SUMMARIZER,
  });
  // ... 清理摘要
} catch (error) {
  if (error instanceof Error && error.name === 'AbortError') {
    debugLogger.debug('[SessionSummary] Timeout generating summary');
  }
  return null;  // 優雅降級：摘要失敗不影響主流程
}
```

> **Clawtex 實作建議**：
> 1. clawtex 缺乏 session 摘要功能 — 可用於 Telegram 會話清單顯示。
> 2. 滑動窗口選取模式（first + last）— 比只取最後 N 條更能捕捉完整意圖。
> 3. 優雅降級（return null）— clawtex 的非關鍵路徑功能都應採用此模式。
> 4. `LlmRole.UTILITY_SUMMARIZER` — 用不同的模型角色區分工具用途 LLM 與主對話 LLM。

---

## 20. 沙箱管理深化分析

### 20.1 SandboxManager 介面

```typescript
// packages/core/src/services/sandboxManager.ts:45-50
export interface SandboxManager {
  prepareCommand(req: SandboxRequest): Promise<SandboxedCommand>;
}
```

### 20.2 NoopSandboxManager — 環境消毒

```typescript
// packages/core/src/services/sandboxManager.ts:56-78
export class NoopSandboxManager implements SandboxManager {
  async prepareCommand(req: SandboxRequest): Promise<SandboxedCommand> {
    const sanitizationConfig: EnvironmentSanitizationConfig = {
      allowedEnvironmentVariables:
        req.config?.sanitizationConfig?.allowedEnvironmentVariables ?? [],
      blockedEnvironmentVariables:
        req.config?.sanitizationConfig?.blockedEnvironmentVariables ?? [],
      enableEnvironmentVariableRedaction: true, // 強制開啟！
    };
    const sanitizedEnv = sanitizeEnvironment(req.env, sanitizationConfig);
    return { program: req.command, args: req.args, env: sanitizedEnv };
  }
}
```

**關鍵設計**：即使是「NoopSandboxManager」（無沙箱），仍然**強制**進行環境變數消毒。`enableEnvironmentVariableRedaction: true` 是硬編碼的 — 不允許繞過。

這意味著即使在非沙箱模式下，模型也無法透過 shell 命令讀取敏感環境變數（如 API keys）。

> **Clawtex 實作建議**：
> 1. clawtex 的 shell tool 直接繼承完整環境變數 — 應引入環境消毒作為**最低安全基線**。
> 2. 允許/阻止列表雙向過濾 — clawtex 的 `agents.toml` 應支援 `allowed_env_vars` 和 `blocked_env_vars`。
> 3. 即使無沙箱也消毒 — 這是防禦縱深的關鍵實踐，clawtex 應立即採用。

---

## 21. 關鍵檔案索引

| 元件 | 路徑 | 說明 |
|------|------|------|
| CLI 入口 | `packages/cli/index.ts` | React/Ink TUI 啟動器 |
| 主函式 | `packages/cli/src/gemini.tsx` | React 主元件 |
| **Turn (串流)** | `packages/core/src/core/turn.ts` | AsyncGenerator 回合串流 |
| **GeminiChat** | `packages/core/src/core/geminiChat.ts` | 對話管理核心 |
| **CoreToolScheduler** | `packages/core/src/core/coreToolScheduler.ts` | 工具排程、7-state 狀態機 |
| **ToolExecutor** | `packages/core/src/scheduler/tool-executor.ts` | 工具執行、Hooks 整合 |
| ToolModificationHandler | `packages/core/src/scheduler/tool-modifier.ts` | 工具參數修改 |
| 工具類型定義 | `packages/core/src/scheduler/types.ts` | ToolStatus、ToolResult |
| Policy 評估 | `packages/core/src/scheduler/policy.ts` | 工具存取策略 |
| ContentGenerator | `packages/core/src/core/contentGenerator.ts` | 內容生成管線 |
| BaseLlmClient | `packages/core/src/core/baseLlmClient.ts` | LLM 客戶端抽象 |
| Token 上限 | `packages/core/src/core/tokenLimits.ts` | 上下文窗口常數 |
| **HookSystem** | `packages/core/src/hooks/hookSystem.ts` | Hooks 入口、結果類型 |
| **HookEventHandler** | `packages/core/src/hooks/hookEventHandler.ts` | 13+ 事件觸發器 |
| HookRegistry | `packages/core/src/hooks/hookRegistry.ts` | Hook 註冊與發現 |
| HookRunner | `packages/core/src/hooks/hookRunner.ts` | Hook 執行引擎 |
| HookAggregator | `packages/core/src/hooks/hookAggregator.ts` | 多 Hook 結果聚合 |
| HookPlanner | `packages/core/src/hooks/hookPlanner.ts` | Hook 執行計畫 |
| Hook 類型 | `packages/core/src/hooks/types.ts` | Hook 事件枚舉 |
| **LoopDetectionService** | `packages/core/src/services/loopDetectionService.ts` | 三層迴圈偵測 |
| **ChatCompressionService** | `packages/core/src/services/chatCompressionService.ts` | 分割點壓縮 |
| **ToolOutputMaskingService** | `packages/core/src/services/toolOutputMaskingService.ts` | 反向掃描遮罩 |
| **ExecutionLifecycleService** | `packages/core/src/services/executionLifecycleService.ts` | 執行生命週期 |
| **ContextManager** | `packages/core/src/services/contextManager.ts` | 三層記憶體管理 |
| **SessionSummaryService** | `packages/core/src/services/sessionSummaryService.ts` | AI 摘要生成 |
| 沙箱管理 | `packages/core/src/services/sandboxManager.ts` | 環境消毒、NoopSandbox |
| 環境消毒 | `packages/core/src/services/environmentSanitization.ts` | 環境變數過濾 |
| Shell 執行 | `packages/core/src/services/shellExecutionService.ts` | Shell 命令執行 |
| 資料夾信任發現 | `packages/core/src/services/FolderTrustDiscoveryService.ts` | 專案信任判斷 |
| 檔案發現 | `packages/core/src/services/fileDiscoveryService.ts` | 工作區檔案掃描 |
| 檔案系統 | `packages/core/src/services/fileSystemService.ts` | 檔案 I/O 抽象 |
| Git 服務 | `packages/core/src/services/gitService.ts` | Git 操作包裝 |
| Keychain 服務 | `packages/core/src/services/keychainService.ts` | 安全 token 存儲 |
| 模型設定 | `packages/core/src/services/modelConfigService.ts` | 模型能力查詢 |
| 對話記錄 | `packages/core/src/services/chatRecordingService.ts` | 歷史持久化 |
| 追蹤器 | `packages/core/src/services/trackerService.ts` | 遙測追蹤 |
| **MCP Auth Provider** | `packages/core/src/mcp/auth-provider.ts` | 認證提供者基類 |
| **Google Auth** | `packages/core/src/mcp/google-auth-provider.ts` | ADC + 域名白名單 |
| **MCP OAuth** | `packages/core/src/mcp/mcp-oauth-provider.ts` | PKCE + callback server |
| SA Impersonation | `packages/core/src/mcp/sa-impersonation-provider.ts` | 服務帳號模擬 |
| OAuth Provider | `packages/core/src/mcp/oauth-provider.ts` | OAuth 2.0 基礎 |
| Token Storage | `packages/core/src/mcp/token-storage/` | 階層式 token 儲存 |
| 工具基礎 | `packages/core/src/tools/tools.ts` | 工具介面定義 |
| 工具註冊 | `packages/core/src/tools/tool-registry.ts` | 工具發現與登記 |
| 工具名稱常數 | `packages/core/src/tools/tool-names.ts` | 工具名稱字串常數 |
| Shell 工具 | `packages/core/src/tools/shell.ts` | Shell 命令工具 |
| MCP 客戶端 | `packages/core/src/tools/mcp-client.ts` | MCP 連線管理 |
| MCP 工具 | `packages/core/src/tools/mcp-tool.ts` | MCP 工具橋接 |
| MessageBus | `packages/core/src/confirmation-bus/message-bus.ts` | WeakMap 訂閱模式 |
| 確認類型 | `packages/core/src/confirmation-bus/types.ts` | 確認協議定義 |
| 模型定義 | `packages/core/src/config/models.ts` | 模型清單與能力 |
| 路由策略 | `packages/core/src/routing/routingStrategy.ts` | 策略鏈路由 |
| 子代理 | `packages/core/src/agents/generalist-agent.ts` | 通用子代理 |
| 代理排程 | `packages/core/src/agents/agent-scheduler.ts` | 子代理排程器 |
| 代理註冊 | `packages/core/src/agents/registry.ts` | 代理類型註冊 |
| 瀏覽器代理 | `packages/core/src/agents/browser/browserAgentFactory.ts` | 瀏覽器自動化 |
| Auth Provider Factory | `packages/core/src/agents/auth-provider/factory.ts` | 認證工廠 |
| 遙測 | `packages/core/src/telemetry/clearcut-logger/clearcut-logger.ts` | Clearcut 遙測 |
| 遙測類型 | `packages/core/src/telemetry/types.ts` | LlmRole 枚舉 |
| 計費 | `packages/core/src/billing/billing.ts` | 使用量計費 |
| 技能管理 | `packages/core/src/skills/skillManager.ts` | 技能載入與啟動 |
| 提示詞 | `packages/core/src/prompts/promptProvider.ts` | 系統提示管理 |
| CodeAssist | `packages/core/src/code_assist/codeAssist.ts` | 程式碼輔助 |
| SDK Agent | `packages/sdk/src/agent.ts` | GeminiCliAgent SDK |
| A2A Server | `packages/a2a-server/` | Agent-to-Agent 協議 |
| DevTools | `packages/devtools/` | 開發者工具面板 |
| VSCode Companion | `packages/vscode-ide-companion/` | VSCode 擴充套件 |
| 沙箱 (CLI) | `packages/cli/src/utils/sandbox.ts` | CLI 沙箱配置 |
| 信任資料夾 | `packages/cli/src/config/trustedFolders.ts` | 信任資料夾管理 |
| 事件匯流排 | `packages/core/src/utils/events.ts` | CoreEvent 事件定義 |
| Token 計算 | `packages/core/src/utils/tokenCalculation.ts` | Token 估算工具 |
| 記憶體發現 | `packages/core/src/utils/memoryDiscovery.ts` | GEMINI.md 發現 |
