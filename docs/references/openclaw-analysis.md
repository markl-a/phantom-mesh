# OpenClaw 深度技術分析 v2

> OpenClaw 是 Claude Code (Anthropic 官方 CLI agent) 的開源版本。
> 專案路徑: `references/openclaw/`
> 版本: 2026.3.11
> 分析日期: 2026-03-13 (v2 深度加倍版)
> 分析目的: 為 clawtex-core 提取可直接移植的架構模式與 Rust 實作建議

---

## 目錄

1. [專案結構](#1-專案結構)
2. [入口點與啟動流程](#2-入口點與啟動流程)
3. [核心架構 — Agent 迴圈](#3-核心架構--agent-迴圈)
4. [Pluggable ContextEngine (深度)](#4-pluggable-contextengine-深度)
5. [Tool Policy Pipeline (5 層) (深度)](#5-tool-policy-pipeline-5-層-深度)
6. [Skills 系統 (深度)](#6-skills-系統-深度)
7. [Auth Profile Rotation (深度)](#7-auth-profile-rotation-深度)
8. [Vector Memory — SQLite-vec + MMR (深度)](#8-vector-memory--sqlite-vec--mmr-深度)
9. [Subagent 深度控制 (深度)](#9-subagent-深度控制-深度)
10. [Gateway 多 Agent 綁定路由 (深度)](#10-gateway-多-agent-綁定路由-深度)
11. [Provider 整合](#11-provider-整合)
12. [串流處理](#12-串流處理)
13. [MCP 整合](#13-mcp-整合)
14. [工具系統](#14-工具系統)
15. [CLAUDE.md / 專案記憶](#15-claudemd--專案記憶)
16. [錯誤處理策略](#16-錯誤處理策略)
17. [效能分析](#17-效能分析)
18. [與 clawtex-core 的完整差距對比](#18-與-clawtex-core-的完整差距對比)
19. [附錄：關鍵檔案路徑對照](#19-附錄關鍵檔案路徑對照)

---

## 1. 專案結構

### 1.1 頂層目錄

```
openclaw/
├── src/                    # 主要原始碼 (TypeScript)
│   ├── agents/             # Agent 核心邏輯 (最關鍵的目錄, 500+ 檔案)
│   ├── cli/                # CLI 命令列入口
│   ├── commands/           # 命令實作 (onboard, doctor, setup, agents 等)
│   ├── config/             # 設定系統 (JSON5, Zod schema 驗證)
│   ├── context-engine/     # 可插拔上下文引擎 (6 檔案)
│   ├── providers/          # LLM Provider 認證 (Google, GitHub Copilot 等)
│   ├── security/           # 安全審計、路徑策略、秘密管理
│   ├── memory/             # 語意記憶 (SQLite-vec, 向量搜尋, 90+ 檔案)
│   ├── gateway/            # Gateway 伺服器 (HTTP/WebSocket, 90+ 檔案)
│   ├── routing/            # 訊息路由 + Agent 綁定
│   ├── sessions/           # 會話管理
│   ├── acp/                # Agent Client Protocol (ACP)
│   ├── channels/           # 頻道抽象 (telegram, discord, slack, signal, whatsapp)
│   ├── plugins/            # 插件系統
│   ├── hooks/              # Hook 系統
│   ├── process/            # 子行程管理
│   ├── infra/              # 基礎設施 (env, ports, binaries)
│   ├── logging/            # 結構化日誌
│   ├── media/              # 媒體處理 (圖片、PDF)
│   └── tui/                # Terminal UI
├── skills/                 # 內建 Skills (50+ 個)
├── extensions/             # 插件/擴充套件 (workspace packages)
├── apps/                   # 原生應用 (Android, iOS, macOS)
├── ui/                     # Web Dashboard UI
└── test/                   # 整合測試
```

### 1.2 技術棧

- **語言**: TypeScript (ESM, Node.js >= 22.16)
- **建置**: tsdown (基於 esbuild 的 bundler)
- **測試**: vitest (多重設定: unit, e2e, gateway, live)
- **套件管理**: pnpm (monorepo)

---

## 2. 入口點與啟動流程

### 2.1 主入口 `src/entry.ts`

```typescript
// src/entry.ts (L1-L15)
process.title = "openclaw";
ensureOpenClawExecMarkerOnProcess();
installProcessWarningFilter();
normalizeEnv();

// 快速路徑: --version 和 --help 不需載入完整 CLI
if (!tryHandleRootVersionFastPath(process.argv) &&
    !tryHandleRootHelpFastPath(process.argv)) {
  import("./cli/run-main.js")
    .then(({ runCli }) => runCli(process.argv))
    .catch(/* ... */);
}
```

### 2.2 啟動認證流程

```
啟動 -> loadConfig() -> 檢查認證
  -> auth-choice.ts 判斷使用哪個 Provider
  -> auth-choice.apply.anthropic.ts   (Anthropic API Key)
  -> auth-choice.apply.openai.ts      (OpenAI API Key)
  -> auth-choice.apply.ollama.ts      (本地 Ollama)
  -> auth-choice.apply.google-gemini-cli.ts (Google Gemini CLI OAuth)
  -> auth-choice.apply.github-copilot.ts   (GitHub Copilot OAuth)
  -> auth-choice.apply.huggingface.ts      (HuggingFace Token)
  -> auth-choice.apply.openrouter.ts       (OpenRouter API Key)
  -> auth-choice.apply.vllm.ts             (vLLM 本地)
  -> auth-choice.apply.minimax.ts          (MiniMax API Key)
  -> auth-choice.apply.byteplus.ts         (BytePlus/Volcengine API Key)
  -> auth-choice.apply.qwen-portal.ts      (Qwen Portal OAuth)
  -> bedrock-discovery.ts                  (AWS Bedrock IAM)
```

---

## 3. 核心架構 -- Agent 迴圈

### 3.1 Agentic Loop 資料流

```
使用者訊息
    │
    v
runEmbeddedPiAgent()                         [src/agents/pi-embedded-runner/run.ts]
    │
    ├── resolveModel() + getApiKeyForModel()
    ├── ensureContextEnginesInitialized()     [src/context-engine/init.ts]
    ├── resolveAuthProfileOrder()             [src/agents/auth-profiles/store.ts]
    │
    └── for (profile of profiles):
        │
        ├── runEmbeddedAttempt()              [src/agents/pi-embedded-runner/run/attempt.ts]
        │   │
        │   ├── prepareSessionManagerForRun()
        │   ├── resolveSandboxContext()
        │   ├── createOpenClawCodingTools()   [src/agents/openclaw-tools.ts]
        │   ├── buildEmbeddedSystemPrompt()   [src/agents/system-prompt.ts]
        │   ├── resolveBootstrapContextForRun()
        │   ├── resolveSkillsPromptForRun()
        │   ├── subscribeEmbeddedPiSession()  [src/agents/pi-embedded-subscribe.ts]
        │   │
        │   └── sessionManager.run({prompt, tools, systemPrompt})
        │       │
        │       └── [多輪工具呼叫迴圈]
        │           ├── 模型回傳 tool_use -> 執行工具
        │           ├── tool_result 附加到歷史
        │           ├── 迴圈偵測檢查           [src/agents/tool-loop-detection.ts]
        │           └── 重複直到 end_turn
        │
        ├── markAuthProfileGood(profile)      // 成功
        └── markAuthProfileFailure(profile)   // 失敗 -> 繼續下一個 profile
```

### 3.2 多輪工具呼叫迴圈偵測

```typescript
// src/agents/tool-loop-detection.ts (L12-L20)
export type LoopDetectorKind =
  | "generic_repeat"        // 相同工具+參數重複呼叫
  | "known_poll_no_progress" // 輪詢但無進展
  | "global_circuit_breaker" // 全域熔斷器 (30 次)
  | "ping_pong";            // A->B->A->B 來回切換

// 閾值: warning=10, critical=20, breaker=30
```

**資料流圖 -- 迴圈偵測**:

```
工具呼叫 #N
    │
    v
LoopDetector.check(toolName, args)
    │
    ├── generic_repeat: hash(toolName + args) 在滑動視窗中出現次數
    ├── known_poll_no_progress: 特定工具 (exec, process) 輸出無變化
    ├── ping_pong: 交替呼叫偵測 A->B->A->B
    └── global_circuit_breaker: 總呼叫次數 > 30
    │
    ├── count < warning(10)  -> 繼續
    ├── count >= warning(10) -> 注入警告訊息
    ├── count >= critical(20) -> 強制壓縮上下文
    └── count >= breaker(30)  -> 強制結束迴圈
```

**Clawtex 差距**: clawtex-core 的 `loop_detection.rs` 有 `generic_repeat` 和 `ping_pong`，缺少 `known_poll_no_progress` 和分級閾值。

> **Clawtex 實作建議**:
> ```rust
> // src/loop_detection.rs 新增
> pub enum LoopSeverity {
>     Warning,   // 10 次 -- 注入提醒
>     Critical,  // 20 次 -- 觸發 compaction
>     Breaker,   // 30 次 -- 強制終止
> }
>
> pub struct ProgressTracker {
>     last_output_hash: HashMap<String, u64>,
> }
>
> impl ProgressTracker {
>     pub fn check_progress(&mut self, tool: &str, output: &str) -> bool {
>         let hash = hash_output(output);
>         let prev = self.last_output_hash.insert(tool.to_string(), hash);
>         prev.map_or(true, |h| h != hash) // true = 有進展
>     }
> }
> ```

---

## 4. Pluggable ContextEngine (深度)

### 4.1 ContextEngine 介面定義

**檔案**: `src/context-engine/types.ts` (L68-L168)

完整介面含 10 個方法，其中 4 個必要 (`ingest`, `assemble`, `compact`, `info`)，6 個可選:

```typescript
// src/context-engine/types.ts (L62-L168, 完整介面)
export interface ContextEngine {
  readonly info: ContextEngineInfo;        // 引擎識別 + 元資料

  // ---- 生命週期 ----
  bootstrap?(params: {
    sessionId: string;
    sessionFile: string;
  }): Promise<BootstrapResult>;

  // ---- 訊息攝入 ----
  ingest(params: {
    sessionId: string;
    message: AgentMessage;
    isHeartbeat?: boolean;                 // 心跳訊息標記
  }): Promise<IngestResult>;

  ingestBatch?(params: {
    sessionId: string;
    messages: AgentMessage[];
    isHeartbeat?: boolean;
  }): Promise<IngestBatchResult>;

  // ---- 回合後鉤子 ----
  afterTurn?(params: {
    sessionId: string;
    sessionFile: string;
    messages: AgentMessage[];
    prePromptMessageCount: number;         // prompt 前的訊息數
    autoCompactionSummary?: string;        // 自動壓縮摘要
    isHeartbeat?: boolean;
    tokenBudget?: number;
    runtimeContext?: ContextEngineRuntimeContext;
  }): Promise<void>;

  // ---- 上下文組裝 ----
  assemble(params: {
    sessionId: string;
    messages: AgentMessage[];
    tokenBudget?: number;
  }): Promise<AssembleResult>;

  // ---- 上下文壓縮 ----
  compact(params: {
    sessionId: string;
    sessionFile: string;
    tokenBudget?: number;
    force?: boolean;
    currentTokenCount?: number;
    compactionTarget?: "budget" | "threshold";
    customInstructions?: string;
    runtimeContext?: ContextEngineRuntimeContext;
  }): Promise<CompactResult>;

  // ---- 子 Agent 生命週期 ----
  prepareSubagentSpawn?(params: {
    parentSessionKey: string;
    childSessionKey: string;
    ttlMs?: number;
  }): Promise<SubagentSpawnPreparation | undefined>;

  onSubagentEnded?(params: {
    childSessionKey: string;
    reason: SubagentEndReason;   // "deleted" | "completed" | "swept" | "released"
  }): Promise<void>;

  dispose?(): Promise<void>;
}
```

### 4.2 ContextEngine 註冊表 (Module-level Singleton)

**檔案**: `src/context-engine/registry.ts` (L1-L85)

```typescript
// registry.ts -- 使用 Symbol.for 實現跨 chunk 的 process-global 單例
const CONTEXT_ENGINE_REGISTRY_STATE = Symbol.for("openclaw.contextEngineRegistryState");

type ContextEngineRegistryState = {
  engines: Map<string, ContextEngineFactory>;
};

// 全域單例存取
function getContextEngineRegistryState(): ContextEngineRegistryState {
  const globalState = globalThis as typeof globalThis & {
    [CONTEXT_ENGINE_REGISTRY_STATE]?: ContextEngineRegistryState;
  };
  if (!globalState[CONTEXT_ENGINE_REGISTRY_STATE]) {
    globalState[CONTEXT_ENGINE_REGISTRY_STATE] = {
      engines: new Map<string, ContextEngineFactory>(),
    };
  }
  return globalState[CONTEXT_ENGINE_REGISTRY_STATE];
}

// 註冊/查詢
export function registerContextEngine(id: string, factory: ContextEngineFactory): void { ... }
export function getContextEngineFactory(id: string): ContextEngineFactory | undefined { ... }
export function listContextEngineIds(): string[] { ... }
```

### 4.3 引擎解析策略

```typescript
// registry.ts (L69-L85)
export async function resolveContextEngine(config?: OpenClawConfig): Promise<ContextEngine> {
  // 解析順序:
  // 1. config.plugins.slots.contextEngine (顯式覆蓋)
  // 2. defaultSlotIdForKey("contextEngine") -> "legacy"
  const engineId = slotValue?.trim() || defaultSlotIdForKey("contextEngine");

  const factory = getContextEngineRegistryState().engines.get(engineId);
  if (!factory) {
    throw new Error(`Context engine "${engineId}" is not registered. Available: ${...}`);
  }
  return factory();
}
```

### 4.4 初始化機制

```typescript
// src/context-engine/init.ts (L1-L23)
let initialized = false;

export function ensureContextEnginesInitialized(): void {
  if (initialized) return;
  initialized = true;
  registerLegacyContextEngine(); // 安全預設
  // 其他引擎由插件在 plugin load 時透過 api.registerContextEngine() 註冊
}
```

### 4.5 Context Compaction 策略

**檔案**: `src/agents/compaction.ts`

```typescript
// 壓縮常數
export const BASE_CHUNK_RATIO = 0.4;    // 基礎分塊比例
export const MIN_CHUNK_RATIO = 0.15;    // 最小分塊比例
export const SAFETY_MARGIN = 1.2;        // 20% 安全邊際

// 壓縮時的保留指令 (關鍵!)
const MERGE_SUMMARIES_INSTRUCTIONS = `
  Merge these partial summaries into a single cohesive summary.
  MUST PRESERVE:
  - Active tasks and their current status
  - Batch operation progress (e.g. "5/17 completed")
  - The last thing the user requested
  - Decisions made and their rationale
  - TODOs, open questions, and constraints
  - Commitments and follow-up actions
  - ALL opaque identifiers (UUIDs, hashes, URLs, file paths)
  PRIORITIZE recent context over older history.
`;
```

### 4.6 完整資料流圖

```
使用者訊息到達
    │
    v
ContextEngine.ingest({ message })     # 攝入新訊息
    │
    v
ContextEngine.assemble({              # 在 token 預算內組裝
  messages, tokenBudget               # 決定哪些訊息送給模型
})
    │
    v
    │<── AssembleResult {
    │      messages: [...],            # 排序後的訊息
    │      estimatedTokens: N,         # 估算 token 數
    │      systemPromptAddition?: "..."
    │    }
    │
    v
[模型呼叫 + 工具執行迴圈]
    │
    v
ContextEngine.afterTurn({             # 回合後鉤子
  messages,
  prePromptMessageCount,
  tokenBudget                          # 可觸發背景壓縮
})
    │
    v
[若 tokenCount > budget × 0.75]
    │
    v
ContextEngine.compact({               # 壓縮上下文
  force: false,
  compactionTarget: "threshold",
  customInstructions: MERGE_SUMMARIES_INSTRUCTIONS
})
    │
    v
CompactResult {
  ok: true,
  compacted: true,
  result: {
    summary: "...",
    tokensBefore: 180000,
    tokensAfter: 60000
  }
}
```

**Clawtex 差距**: clawtex-core 的 `context_compactor.rs` 是一個具體實作而非 trait，無法插拔替換。沒有 `afterTurn` 鉤子、沒有 `ingest/assemble` 分離、沒有 `ContextEngineInfo` 元資料。

> **Clawtex 實作建議**:
> ```rust
> // src/context_engine.rs -- 新增可插拔 trait
> use async_trait::async_trait;
>
> pub struct ContextEngineInfo {
>     pub id: String,
>     pub name: String,
>     pub owns_compaction: bool,
> }
>
> #[async_trait]
> pub trait ContextEngine: Send + Sync {
>     fn info(&self) -> &ContextEngineInfo;
>
>     async fn ingest(&self, session_id: &str, message: &Message) -> Result<bool>;
>
>     async fn assemble(
>         &self,
>         session_id: &str,
>         messages: &[Message],
>         token_budget: usize,
>     ) -> Result<AssembleResult>;
>
>     async fn compact(
>         &self,
>         session_id: &str,
>         token_budget: usize,
>         force: bool,
>     ) -> Result<CompactResult>;
>
>     async fn after_turn(
>         &self,
>         session_id: &str,
>         messages: &[Message],
>         pre_prompt_count: usize,
>     ) -> Result<()> {
>         Ok(()) // 預設空操作
>     }
> }
>
> // 註冊表
> pub struct ContextEngineRegistry {
>     engines: HashMap<String, Box<dyn Fn() -> Box<dyn ContextEngine>>>,
> }
> ```

---

## 5. Tool Policy Pipeline (5 層) (深度)

### 5.1 管線架構

**檔案**: `src/agents/tool-policy-pipeline.ts` (L1-L108)

OpenClaw 的工具策略是一個 **7 步管線**，每步可獨立過濾工具集:

```typescript
// tool-policy-pipeline.ts (L17-L63)
export function buildDefaultToolPolicyPipelineSteps(params): ToolPolicyPipelineStep[] {
  return [
    { policy: params.profilePolicy,          label: "tools.profile" },
    { policy: params.providerProfilePolicy,  label: "tools.byProvider.profile" },
    { policy: params.globalPolicy,           label: "tools.allow" },
    { policy: params.globalProviderPolicy,   label: "tools.byProvider.allow" },
    { policy: params.agentPolicy,            label: "agents.${id}.tools.allow" },
    { policy: params.agentProviderPolicy,    label: "agents.${id}.tools.byProvider.allow" },
    { policy: params.groupPolicy,            label: "group tools.allow" },
  ];
}
```

### 5.2 管線執行流程

```typescript
// tool-policy-pipeline.ts (L65-L108)
export function applyToolPolicyPipeline(params): AnyAgentTool[] {
  // 1. 收集核心工具名稱 (非插件)
  const coreToolNames = new Set(
    params.tools
      .filter((tool) => !params.toolMeta(tool))  // 非插件工具
      .map((tool) => normalizeToolName(tool.name))
  );

  // 2. 建立插件工具群組
  const pluginGroups = buildPluginToolGroups({ tools, toolMeta });

  // 3. 逐步過濾
  let filtered = params.tools;
  for (const step of params.steps) {
    if (!step.policy) continue;

    let policy = step.policy;
    if (step.stripPluginOnlyAllowlist) {
      // 防止只含插件名稱的 allowlist 意外禁用核心工具
      const resolved = stripPluginOnlyAllowlist(policy, pluginGroups, coreToolNames);
      if (resolved.unknownAllowlist.length > 0) {
        params.warn(`tools: ${step.label} allowlist contains unknown entries...`);
      }
      policy = resolved.policy;
    }

    const expanded = expandPolicyWithPluginGroups(policy, pluginGroups);
    filtered = expanded ? filterToolsByPolicy(filtered, expanded) : filtered;
  }
  return filtered;
}
```

### 5.3 管線資料流圖

```
所有工具 (24+)
    │
    ├─[Step 1] Profile Filter ─────────── tools.profile="coding"
    │   過濾為 coding profile 的工具 (exec, read, write, edit...)
    │
    ├─[Step 2] Provider Profile ────────── tools.byProvider.profile
    │   按 LLM Provider 過濾 (例: ollama 禁用 browser)
    │
    ├─[Step 3] Global Allow/Deny ──────── tools.allow=["*"], tools.deny=["cron"]
    │   全域黑白名單
    │
    ├─[Step 4] Provider Allow/Deny ─────── tools.byProvider.allow
    │   按 Provider 的黑白名單
    │
    ├─[Step 5] Agent Allow/Deny ────────── agents.researcher.tools.allow=["web_*"]
    │   按 Agent 的黑白名單 (Glob 模式!)
    │
    ├─[Step 6] Agent Provider Allow ────── agents.researcher.tools.byProvider.allow
    │   交叉: Agent + Provider 的策略
    │
    └─[Step 7] Group Policy ────────────── group tools.allow
        頻道群組的工具限制

結果: 最終可用工具集
```

### 5.4 Glob Pattern 匹配

**檔案**: `src/agents/glob-pattern.ts`

工具名稱支援 glob 模式匹配:

```typescript
// 支援的模式:
// "web_*"      -> 匹配 web_search, web_fetch
// "sessions_*" -> 匹配 sessions_spawn, sessions_send, sessions_list
// "*"          -> 匹配所有
```

### 5.5 插件工具保護機制

管線中的 `stripPluginOnlyAllowlist` 防止一個常見錯誤: 使用者在 allowlist 中只寫了插件工具名稱，導致核心工具被意外過濾掉:

```typescript
// 例: 使用者設定 tools.allow = ["my-plugin-tool"]
// 沒有 stripPluginOnlyAllowlist 保護: 只剩 my-plugin-tool，核心工具全被禁用
// 有保護: 偵測到 allowlist 全是未知名稱 -> 忽略此 allowlist，保留核心工具
```

### 5.6 子 Agent 工具策略

**檔案**: `src/agents/pi-tools.policy.ts` (L51-L101)

```typescript
// 永遠禁止的子 Agent 工具 (L51-L65)
const SUBAGENT_TOOL_DENY_ALWAYS = [
  "gateway",        // 系統管理
  "agents_list",    // 系統管理
  "whatsapp_login", // 互動式設定
  "session_status", // 主 Agent 協調用
  "cron",           // 排程
  "memory_search",  // 記憶 (透過 spawn prompt 傳遞)
  "memory_get",     // 記憶
  "sessions_send",  // 子 Agent 透過 announce 通訊
];

// 葉節點子 Agent 額外禁用 (L72-L76)
const SUBAGENT_TOOL_DENY_LEAF = [
  "subagents",        // 無法再生成子 Agent
  "sessions_list",
  "sessions_history",
  "sessions_spawn",
];

// 動態解析 deny list (L86-L94)
function resolveSubagentDenyList(depth: number, maxSpawnDepth: number): string[] {
  const isLeaf = depth >= Math.max(1, Math.floor(maxSpawnDepth));
  if (isLeaf) {
    return [...SUBAGENT_TOOL_DENY_ALWAYS, ...SUBAGENT_TOOL_DENY_LEAF];
  }
  return [...SUBAGENT_TOOL_DENY_ALWAYS]; // 編排者保留 spawn 能力
}
```

**Clawtex 差距**: clawtex-core 的 `tool_registry` 沒有管線式過濾，沒有 profile 概念，沒有 per-provider 工具限制，沒有 glob 模式，子 Agent (delegate) 沒有工具裁剪。

> **Clawtex 實作建議**:
> ```rust
> // src/tool_policy.rs -- 新增管線式工具過濾
> pub struct ToolPolicy {
>     pub allow: Option<Vec<String>>,  // None = 全部允許
>     pub deny: Vec<String>,
> }
>
> pub struct ToolPolicyStep {
>     pub policy: Option<ToolPolicy>,
>     pub label: String,
> }
>
> pub fn apply_tool_policy_pipeline(
>     tools: &[Box<dyn Tool>],
>     steps: &[ToolPolicyStep],
> ) -> Vec<&dyn Tool> {
>     let mut filtered: Vec<&dyn Tool> = tools.iter().map(|t| t.as_ref()).collect();
>     for step in steps {
>         if let Some(policy) = &step.policy {
>             filtered = filtered.into_iter().filter(|tool| {
>                 let name = tool.name();
>                 // deny 優先
>                 if policy.deny.iter().any(|d| glob_match(d, name)) {
>                     return false;
>                 }
>                 // allow 檢查
>                 match &policy.allow {
>                     Some(allow) => allow.iter().any(|a| glob_match(a, name)),
>                     None => true,
>                 }
>             }).collect();
>         }
>     }
>     filtered
> }
>
> fn glob_match(pattern: &str, name: &str) -> bool {
>     if pattern == "*" { return true; }
>     if pattern.ends_with('*') {
>         name.starts_with(&pattern[..pattern.len()-1])
>     } else {
>         name == pattern
>     }
> }
> ```

---

## 6. Skills 系統 (深度)

### 6.1 Skill 型別定義

**檔案**: `src/agents/skills/types.ts` (L1-L89)

```typescript
export type SkillEntry = {
  skill: Skill;                        // 來自 pi-coding-agent
  frontmatter: ParsedSkillFrontmatter; // YAML frontmatter
  metadata?: OpenClawSkillMetadata;    // OpenClaw 專用元資料
  invocation?: SkillInvocationPolicy;  // 呼叫策略
};

export type OpenClawSkillMetadata = {
  always?: boolean;            // 始終載入 (不受過濾影響)
  skillKey?: string;           // 唯一鍵值
  primaryEnv?: string;         // 主要環境變數 (如 GITHUB_TOKEN)
  emoji?: string;              // 表情符號標識
  homepage?: string;           // 首頁 URL
  os?: string[];               // 支援的作業系統 ["linux", "darwin", "win32"]
  requires?: {
    bins?: string[];           // 必需的二進位檔 (全部)
    anyBins?: string[];        // 任一即可
    env?: string[];            // 環境變數
    config?: string[];         // 設定項
  };
  install?: SkillInstallSpec[];  // 安裝指令
};

export type SkillInstallSpec = {
  kind: "brew" | "node" | "go" | "uv" | "download";
  formula?: string;   // brew formula
  package?: string;   // npm/go 套件
  bins?: string[];    // 安裝後的執行檔
  url?: string;       // download URL
  extract?: boolean;  // 是否解壓
};

export type SkillInvocationPolicy = {
  userInvocable: boolean;              // 使用者可透過 /command 呼叫
  disableModelInvocation: boolean;     // 禁止模型自動呼叫
};
```

### 6.2 Skill 載入管線 (6 層來源)

**檔案**: `src/agents/skills/workspace.ts` (L292-L527)

```
Skill 來源 (優先順序由低到高):
    │
    ├── [1] extra dirs      (config.skills.load.extraDirs)
    ├── [2] bundled          (openclaw 內建 50+ skills)
    ├── [3] managed          (~/.openclaw/skills/)
    ├── [4] agents-personal  (~/.agents/skills/)       -- 個人跨專案
    ├── [5] agents-project   ({workspace}/.agents/skills/) -- 專案級
    └── [6] workspace        ({workspace}/skills/)     -- 最高優先
    │
    v
合併 (Map<name, Skill>，後者覆蓋前者)
    │
    v
parseFrontmatter() + resolveOpenClawMetadata()
    │
    v
filterSkillEntries()
    ├── shouldIncludeSkill()     -- OS/依賴/設定檢查
    ├── normalizeSkillFilter()   -- agent 級過濾
    └── invocation policy        -- 是否允許模型呼叫
    │
    v
applySkillsPromptLimits()
    ├── maxSkillsInPrompt: 150
    ├── maxSkillsPromptChars: 30,000
    └── 二分搜尋找最大前綴
    │
    v
compactSkillPaths()              -- 路徑壓縮 (~ 替換，省 400-600 tokens)
    │
    v
formatSkillsForPrompt()          -- 注入 system prompt
```

### 6.3 路徑安全檢查

```typescript
// workspace.ts (L187-L221) -- 防止 symlink 逃逸
function resolveContainedSkillPath(params): string | null {
  const candidateRealPath = tryRealpath(params.candidatePath);
  if (!candidateRealPath) return null;

  // 確保解析後的路徑仍在 root 目錄內
  if (isPathInside(params.rootRealPath, candidateRealPath)) {
    return candidateRealPath;
  }

  // symlink 指向 root 外部 -> 拒絕並警告
  warnEscapedSkillPath({ source, rootDir, candidatePath, candidateRealPath });
  return null;
}
```

### 6.4 Skill 檔案大小限制

```typescript
// workspace.ts (L97-L100)
const DEFAULT_MAX_CANDIDATES_PER_ROOT = 300;
const DEFAULT_MAX_SKILLS_LOADED_PER_SOURCE = 200;
const DEFAULT_MAX_SKILLS_IN_PROMPT = 150;
const DEFAULT_MAX_SKILLS_PROMPT_CHARS = 30_000;
const DEFAULT_MAX_SKILL_FILE_BYTES = 256_000;    // 256KB per SKILL.md
```

### 6.5 二分搜尋 Token 預算

```typescript
// workspace.ts (L529-L565)
function applySkillsPromptLimits(params): { skillsForPrompt, truncated } {
  let skillsForPrompt = params.skills.slice(0, maxSkillsInPrompt);

  const fits = (skills) => formatSkillsForPrompt(skills).length <= maxSkillsPromptChars;

  if (!fits(skillsForPrompt)) {
    // 二分搜尋: 找到最大不超過 char 預算的前綴
    let lo = 0, hi = skillsForPrompt.length;
    while (lo < hi) {
      const mid = Math.ceil((lo + hi) / 2);
      if (fits(skillsForPrompt.slice(0, mid))) lo = mid;
      else hi = mid - 1;
    }
    skillsForPrompt = skillsForPrompt.slice(0, lo);
  }
}
```

**Clawtex 差距**: clawtex-core 沒有 Skills 系統。Hands 工作流是最接近的概念，但不具備: YAML frontmatter 元資料、依賴檢查、自動安裝、token 預算控制、路徑安全檢查。

> **Clawtex 實作建議**:
> ```rust
> // src/skills.rs -- 新增 Skill 系統
> use serde::Deserialize;
>
> #[derive(Deserialize)]
> pub struct SkillMetadata {
>     pub name: String,
>     pub description: String,
>     pub requires: Option<SkillRequires>,
>     pub os: Option<Vec<String>>,
> }
>
> #[derive(Deserialize)]
> pub struct SkillRequires {
>     pub bins: Option<Vec<String>>,
>     pub env: Option<Vec<String>>,
> }
>
> pub struct SkillEntry {
>     pub metadata: SkillMetadata,
>     pub content: String,
>     pub file_path: PathBuf,
> }
>
> pub fn load_skills(dirs: &[PathBuf], max_prompt_chars: usize) -> Vec<SkillEntry> {
>     let mut skills = Vec::new();
>     for dir in dirs {
>         for entry in std::fs::read_dir(dir).ok().into_iter().flatten() {
>             let skill_md = entry.path().join("SKILL.md");
>             if skill_md.exists() {
>                 if let Ok(content) = std::fs::read_to_string(&skill_md) {
>                     if content.len() <= 256_000 {
>                         if let Some(skill) = parse_skill(&content, &skill_md) {
>                             if skill_eligible(&skill) {
>                                 skills.push(skill);
>                             }
>                         }
>                     }
>                 }
>             }
>         }
>     }
>     truncate_to_budget(&mut skills, max_prompt_chars);
>     skills
> }
> ```

---

## 7. Auth Profile Rotation (深度)

### 7.1 AuthProfileStore 結構

**檔案**: `src/agents/auth-profiles/store.ts` (L1-L509)

```typescript
// auth-profiles/types.ts (概念型別)
type AuthProfileStore = {
  version: number;
  profiles: Record<string, AuthProfileCredential>;
  order?: Record<string, string[]>;      // 每 provider 的使用順序
  lastGood?: Record<string, string>;     // 每 provider 最後成功的 profile
  usageStats?: Record<string, ProfileUsageStats>;
};

type AuthProfileCredential =
  | { type: "api_key"; provider: string; key: string; keyRef?: string; }
  | { type: "oauth"; provider: string; access: string; refresh: string; expires: number; }
  | { type: "token"; provider: string; token: string; tokenRef?: string; };

type ProfileUsageStats = {
  lastUsed?: number;      // Unix timestamp
  successCount?: number;
  failureCount?: number;
  lastError?: string;
  cooldownUntil?: number; // 冷卻期到期時間
};
```

### 7.2 Profile 載入與合併

```typescript
// store.ts (L346-L456) -- 多層載入策略
export function loadAuthProfileStore(): AuthProfileStore {
  // 1. 嘗試載入 auth-profiles.json
  const asStore = loadCoercedStore(authPath);
  if (asStore) {
    syncExternalCliCredentials(asStore);  // 同步外部 CLI 工具認證
    return asStore;
  }

  // 2. 回退: 載入 legacy auth.json
  const legacy = coerceLegacyStore(loadJsonFile(resolveLegacyAuthStorePath()));
  if (legacy) {
    const store = { version: AUTH_STORE_VERSION, profiles: {} };
    applyLegacyStore(store, legacy);
    return store;
  }

  // 3. 空 store
  return { version: AUTH_STORE_VERSION, profiles: {} };
}

// 子 Agent 認證繼承 (L374-L401)
function loadAuthProfileStoreForAgent(agentDir?, options?): AuthProfileStore {
  // 子 Agent 沒有自己的認證? -> 從主 Agent 繼承
  if (agentDir && !readOnly) {
    const mainStore = loadAuthProfileStoreForAgent(undefined, options);
    if (Object.keys(mainStore.profiles).length > 0) {
      saveJsonFile(authPath, mainStore); // 克隆到子 Agent 目錄
      return mainStore;
    }
  }
}
```

### 7.3 Profile 輪轉策略

```typescript
// src/agents/auth-profiles/ -- 輪轉邏輯
// 解析順序:
// 1. order[provider] 中的顯式順序
// 2. lastGood[provider] 放到最前面
// 3. 按 lastUsed 時間排序 (最少使用優先 -> round-robin 效果)
// 4. 跳過 cooldownUntil > now 的 profile

// Cooldown 自動過期 (auth-profiles.cooldown-auto-expiry.test.ts)
// 失敗時設定 cooldownUntil = now + backoffMs
// backoffMs 隨失敗次數指數增長: 250 -> 500 -> 1000 -> 1500 (cap)
```

### 7.4 運行時快照 + 檔案鎖

```typescript
// store.ts (L22-L99) -- 執行緒安全的認證更新
const runtimeAuthStoreSnapshots = new Map<string, AuthProfileStore>();

export async function updateAuthProfileStoreWithLock(params) {
  return await withFileLock(authPath, AUTH_STORE_LOCK_OPTIONS, async () => {
    const store = ensureAuthProfileStore(params.agentDir);
    const shouldSave = params.updater(store);
    if (shouldSave) saveAuthProfileStore(store, params.agentDir);
    return store;
  });
}
```

### 7.5 外部 CLI 同步

```typescript
// external-cli-sync.ts -- 自動同步 claude, openai 等 CLI 的認證
function syncExternalCliCredentials(store): boolean {
  // 掃描 ~/.config/claude/, ~/.openai/ 等
  // 自動匯入找到的 API key/token
  // 不覆蓋已存在的 profile
}
```

**Clawtex 差距**: clawtex-core 的 `key_pool.rs` 有基本的 key 輪轉，但缺少: cooldown 自動過期、檔案鎖保護、外部 CLI 同步、per-provider 排序策略、usage stats 追蹤、子 Agent 認證繼承。

> **Clawtex 實作建議**:
> ```rust
> // src/providers/key_pool.rs 增強
> pub struct ProfileUsageStats {
>     pub last_used: Option<Instant>,
>     pub success_count: u64,
>     pub failure_count: u64,
>     pub cooldown_until: Option<Instant>,
> }
>
> impl KeyPool {
>     /// 自動移除已過期的 cooldown
>     pub fn expire_cooldowns(&mut self) {
>         let now = Instant::now();
>         for stats in self.usage_stats.values_mut() {
>             if let Some(until) = stats.cooldown_until {
>                 if now >= until {
>                     stats.cooldown_until = None;
>                 }
>             }
>         }
>     }
>
>     /// 設定帶指數退避的 cooldown
>     pub fn mark_failure(&mut self, key_id: &str) {
>         let stats = self.usage_stats.entry(key_id.to_string())
>             .or_insert_with(ProfileUsageStats::default);
>         stats.failure_count += 1;
>         let backoff_ms = (250 * 2u64.pow(stats.failure_count.min(3) as u32))
>             .min(1500);
>         stats.cooldown_until = Some(Instant::now() + Duration::from_millis(backoff_ms));
>     }
> }
> ```

---

## 8. Vector Memory -- SQLite-vec + MMR (深度)

### 8.1 記憶系統架構

**目錄**: `src/memory/` (90+ 檔案)

```
記憶系統元件:
├── manager.ts           # MemorySearchManager -- 主管理器
├── search-manager.ts    # 搜尋協調器
├── hybrid.ts            # 混合搜尋 (向量 + 全文)
├── sqlite-vec.ts        # SQLite-vec 擴充載入
├── sqlite.ts            # SQLite 操作
├── mmr.ts               # MMR 重排序
├── temporal-decay.ts    # 時間衰減
├── embeddings.ts        # 嵌入管理器
├── embeddings-openai.ts # OpenAI Embeddings
├── embeddings-voyage.ts # Voyage Embeddings
├── embeddings-gemini.ts # Gemini Embeddings
├── embeddings-ollama.ts # Ollama Embeddings
├── embeddings-mistral.ts # Mistral Embeddings
├── embeddings-remote-*.ts # 遠端 HTTP 嵌入
└── qmd-*.ts             # QMD (Query-Memory-Document) 管理
```

### 8.2 MMR (Maximal Marginal Relevance) 演算法

**檔案**: `src/memory/mmr.ts` (L1-L214)

```typescript
// mmr.ts -- 完整 MMR 實作

// MMR 公式: score = lambda * relevance - (1-lambda) * max_similarity_to_selected
// lambda = 0.7 (預設): 70% 相關性 + 30% 多樣性

export type MMRConfig = {
  enabled: boolean;  // 預設: false (需顯式啟用)
  lambda: number;    // 0 = 最大多樣性, 1 = 最大相關性, 預設: 0.7
};

// Jaccard 相似度計算
export function jaccardSimilarity(setA: Set<string>, setB: Set<string>): number {
  let intersectionSize = 0;
  const smaller = setA.size <= setB.size ? setA : setB;
  const larger = setA.size <= setB.size ? setB : setA;
  for (const token of smaller) {
    if (larger.has(token)) intersectionSize++;
  }
  const unionSize = setA.size + setB.size - intersectionSize;
  return unionSize === 0 ? 0 : intersectionSize / unionSize;
}

// MMR 重排序主函式
export function mmrRerank<T extends MMRItem>(items: T[], config): T[] {
  // 1. 預先 tokenize 所有項目 (效能優化)
  const tokenCache = new Map<string, Set<string>>();
  for (const item of items) {
    tokenCache.set(item.id, tokenize(item.content));
  }

  // 2. 正規化分數到 [0, 1]
  const maxScore = Math.max(...items.map(i => i.score));
  const minScore = Math.min(...items.map(i => i.score));
  const scoreRange = maxScore - minScore;

  // 3. 迭代選擇
  const selected: T[] = [];
  const remaining = new Set(items);
  while (remaining.size > 0) {
    let bestItem = null, bestMMRScore = -Infinity;
    for (const candidate of remaining) {
      const relevance = normalizeScore(candidate.score);
      const maxSim = maxSimilarityToSelected(candidate, selected, tokenCache);
      const mmrScore = lambda * relevance - (1 - lambda) * maxSim;
      if (mmrScore > bestMMRScore) {
        bestMMRScore = mmrScore;
        bestItem = candidate;
      }
    }
    if (bestItem) { selected.push(bestItem); remaining.delete(bestItem); }
    else break;
  }
  return selected;
}
```

### 8.3 時間衰減機制

**檔案**: `src/memory/temporal-decay.ts` (L1-L167)

```typescript
// 指數衰減: score * e^(-lambda * age_days)
// lambda = ln(2) / halfLifeDays
// halfLifeDays = 30 (預設): 30 天後分數降為一半

export function calculateTemporalDecayMultiplier(params): number {
  const lambda = Math.LN2 / params.halfLifeDays;
  return Math.exp(-lambda * Math.max(0, params.ageInDays));
}

// 日期化記憶: memory/2026-03-13.md -> 從路徑提取日期
// 常青記憶: MEMORY.md, memory/topic.md -> 不衰減
function isEvergreenMemoryPath(filePath: string): boolean {
  if (normalized === "MEMORY.md") return true;
  if (!normalized.startsWith("memory/")) return false;
  return !DATED_MEMORY_PATH_RE.test(normalized);
}
```

### 8.4 SQLite-vec 載入

```typescript
// sqlite-vec.ts (L1-L24)
export async function loadSqliteVecExtension(params) {
  const sqliteVec = await import("sqlite-vec");
  const extensionPath = params.extensionPath ?? sqliteVec.getLoadablePath();
  params.db.enableLoadExtension(true);
  sqliteVec.load(params.db);
  return { ok: true, extensionPath };
}
```

### 8.5 混合搜尋流程

```
查詢 "如何設定 Telegram bot?"
    │
    v
[查詢擴展] query-expansion.ts
    ├── 同義詞擴展
    └── 子查詢分解
    │
    v
[並行搜尋]
    ├── 向量搜尋 (SQLite-vec)
    │   ├── embed(query) -> float[384]
    │   └── SELECT *, vec_distance(embedding, ?) AS dist
    │       FROM memory_vectors ORDER BY dist LIMIT 20
    │
    └── 全文搜尋 (SQLite FTS5)
        └── SELECT * FROM memory_fts WHERE content MATCH ?
    │
    v
[合併 + 去重]
    │
    v
[時間衰減] temporal-decay.ts
    ├── 日期化記憶: score *= e^(-ln2/30 * age_days)
    └── 常青記憶: score 不變
    │
    v
[MMR 重排序] mmr.ts
    ├── lambda=0.7: 70% 相關 + 30% 多樣
    └── Jaccard 相似度去重
    │
    v
最終結果 (path#line 格式引用)
```

**Clawtex 差距**: clawtex-core 的 `memory.rs` 使用簡單的 key-value SQLite 存儲。沒有向量搜尋、沒有 MMR、沒有時間衰減、沒有混合搜尋、沒有查詢擴展。

> **Clawtex 實作建議**:
> ```rust
> // src/memory_search.rs -- 新增向量記憶搜尋
> use sqlite_vec::SqliteVec;
>
> pub struct VectorMemory {
>     db: Connection,
>     embedding_dim: usize,
> }
>
> impl VectorMemory {
>     pub fn search(&self, query_embedding: &[f32], limit: usize) -> Vec<MemoryResult> {
>         self.db.prepare(
>             "SELECT key, content, vec_distance_L2(embedding, ?1) as dist
>              FROM memory_vectors ORDER BY dist LIMIT ?2"
>         ).unwrap()
>         .query_map(params![query_embedding.as_bytes(), limit], |row| {
>             Ok(MemoryResult { key: row.get(0)?, content: row.get(1)?, score: 1.0 / (1.0 + row.get::<_, f64>(2)?) })
>         }).unwrap().filter_map(|r| r.ok()).collect()
>     }
> }
>
> // MMR 重排序
> pub fn mmr_rerank(items: &mut Vec<MemoryResult>, lambda: f64) {
>     let mut selected = Vec::new();
>     let mut remaining: HashSet<usize> = (0..items.len()).collect();
>     while !remaining.is_empty() {
>         let (best_idx, _) = remaining.iter().map(|&i| {
>             let relevance = items[i].score;
>             let max_sim = selected.iter()
>                 .map(|&j| jaccard_similarity(&items[i].tokens, &items[j].tokens))
>                 .fold(0.0f64, f64::max);
>             (i, lambda * relevance - (1.0 - lambda) * max_sim)
>         }).max_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).unwrap();
>         selected.push(best_idx);
>         remaining.remove(&best_idx);
>     }
> }
> ```

---

## 9. Subagent 深度控制 (深度)

### 9.1 深度解析機制

**檔案**: `src/agents/subagent-depth.ts` (L1-L176)

```typescript
// subagent-depth.ts -- 完整深度解析
export function getSubagentDepthFromSessionStore(
  sessionKey: string,
  opts?: { cfg?: OpenClawConfig; store?: Record<string, SessionDepthEntry> }
): number {
  const cache = new Map();
  const visited = new Set<string>();  // 防止循環

  const depthFromStore = (key: string): number | undefined => {
    if (visited.has(key)) return undefined;  // 循環保護
    visited.add(key);

    const entry = resolveEntryForSessionKey({ sessionKey: key, cfg, store, cache });

    // 1. 直接記錄的深度
    const storedDepth = normalizeSpawnDepth(entry?.spawnDepth);
    if (storedDepth !== undefined) return storedDepth;

    // 2. 透過 spawnedBy 遞迴查找
    const spawnedBy = normalizeSessionKey(entry?.spawnedBy);
    if (!spawnedBy) return undefined;

    const parentDepth = depthFromStore(spawnedBy);
    if (parentDepth !== undefined) return parentDepth + 1;

    // 3. 從 session key 解析 (回退)
    return getSubagentDepth(spawnedBy) + 1;
  };

  return depthFromStore(sessionKey) ?? fallbackDepth;
}
```

### 9.2 深度決策矩陣

```
深度 0: 主 Agent (root)
  ├── 所有工具可用
  ├── 可生成子 Agent
  └── 可存取記憶、排程

深度 1: 編排子 Agent (orchestrator, depth < maxSpawnDepth)
  ├── 禁用: gateway, agents_list, whatsapp_login, session_status,
  │         cron, memory_search, memory_get, sessions_send
  ├── 保留: sessions_spawn, subagents, sessions_list, sessions_history
  │         (可以繼續生成更深層子 Agent)
  └── 保留: exec, read, write, edit, web_search 等工作工具

深度 N: 葉節點子 Agent (depth >= maxSpawnDepth)
  ├── 禁用: 上述全部 + subagents, sessions_spawn,
  │         sessions_list, sessions_history
  ├── 保留: 純工作工具 (exec, read, write, web_search 等)
  └── 結果透過 announce chain 回報父 Agent
```

### 9.3 子 Agent 完成通知 (Push-based)

```
子 Agent 完成
    │
    v
subagent-announce.ts
    ├── captureSubagentCompletionReply()
    │   └── 收集子 Agent 的最終輸出
    ├── runSubagentAnnounceFlow()
    │   ├── 格式化完成訊息
    │   └── 附加上下文 (duration, token usage)
    └── 注入為 user message 到父 Agent 會話
        └── 父 Agent 自動收到通知，無需輪詢
```

### 9.4 子 Agent 能力解析

**檔案**: `src/agents/subagent-capabilities.ts`

```typescript
export type SubagentSessionRole = "orchestrator" | "leaf";

export function resolveStoredSubagentCapabilities(
  sessionKey: string,
  opts?: { cfg?: OpenClawConfig }
): { role: SubagentSessionRole } {
  // 從 session store 讀取 role
  // 或從深度計算推導
}
```

**Clawtex 差距**: clawtex-core 的 `delegate` 工具沒有深度限制、沒有工具裁剪、沒有 push-based 通知、沒有循環保護。

> **Clawtex 實作建議**:
> ```rust
> // src/tools/delegate.rs 增強
> const MAX_DELEGATE_DEPTH: u8 = 3;
>
> pub struct DelegateContext {
>     pub depth: u8,
>     pub parent_session: String,
> }
>
> impl DelegateTool {
>     fn resolve_available_tools(&self, depth: u8) -> Vec<String> {
>         let mut denied = vec![
>             "delegate_to_provider",
>             "memory_store", "memory_recall",
>         ];
>         if depth >= MAX_DELEGATE_DEPTH {
>             denied.extend_from_slice(&["delegate"]);
>         }
>         self.all_tools.iter()
>             .filter(|t| !denied.contains(&t.name()))
>             .map(|t| t.name().to_string())
>             .collect()
>     }
> }
> ```

---

## 10. Gateway 多 Agent 綁定路由 (深度)

### 10.1 路由綁定架構

**檔案**: `src/routing/resolve-route.ts` (L1-L804)
**檔案**: `src/routing/bindings.ts` (L1-L114)

OpenClaw 的 Gateway 支援根據頻道、帳號、群組、角色等條件將訊息路由到不同的 Agent:

```typescript
// 路由綁定設定範例:
{
  "bindings": [
    {
      "agentId": "support-bot",
      "match": {
        "channel": "discord",
        "peer": { "kind": "channel", "id": "123456789" }
      }
    },
    {
      "agentId": "dev-bot",
      "match": {
        "channel": "telegram",
        "accountId": "my-account"
      }
    },
    {
      "agentId": "general",
      "match": {
        "channel": "slack",
        "guildId": "T12345",
        "roles": ["admin"]
      }
    }
  ]
}
```

### 10.2 7 層路由匹配

```typescript
// resolve-route.ts (L723-L781) -- 7 層匹配優先順序
const tiers = [
  { matchedBy: "binding.peer",         // 精確 peer (群組/頻道) 匹配
    candidates: collectPeerIndexedBindings(index, peer) },
  { matchedBy: "binding.peer.parent",  // 線程父 peer 繼承
    candidates: collectPeerIndexedBindings(index, parentPeer) },
  { matchedBy: "binding.guild+roles",  // Guild + 角色匹配
    candidates: byGuildWithRoles },
  { matchedBy: "binding.guild",        // 純 Guild 匹配
    candidates: byGuild },
  { matchedBy: "binding.team",         // Team (Slack workspace) 匹配
    candidates: byTeam },
  { matchedBy: "binding.account",      // 帳號匹配
    candidates: byAccount },
  { matchedBy: "binding.channel",      // 頻道匹配 (最寬泛)
    candidates: byChannel },
];

// 未匹配任何綁定 -> 使用 default agent
return choose(resolveDefaultAgentId(cfg), "default");
```

### 10.3 效能最佳化 -- 多層快取

```typescript
// resolve-route.ts -- 三層快取策略
// 1. evaluatedBindingsCacheByCfg (WeakMap<Config, ...>)
//    -> 按 config 物件快取已解析的綁定
// 2. byChannelAccount cache (Map<string, EvaluatedBinding[]>)
//    -> 按 channel+account 快取合併後的綁定, 上限 2000 entries
// 3. resolvedRouteCacheByCfg (WeakMap<Config, ...>)
//    -> 按完整路由鍵快取最終結果, 上限 4000 entries

const MAX_EVALUATED_BINDINGS_CACHE_KEYS = 2000;
const MAX_RESOLVED_ROUTE_CACHE_KEYS = 4000;

// 快取 key 格式:
// "${channel}\t${accountId}\t${peer}\t${parentPeer}\t${guildId}\t${teamId}\t${roles}\t${dmScope}"
```

### 10.4 Session Key 建構

```typescript
// 不同路由產生不同的 session key:
// DM: agent:main:telegram:direct:user123
// 群組: agent:main:discord:group:channel456
// 線程: agent:main:slack:channel:thread789:parent:channel456
// 綁定: agent:support-bot:discord:channel:123456789
```

**Clawtex 差距**: clawtex-core 只有 Telegram 單頻道，沒有 multi-agent 路由。`providers/router.rs` 做的是 LLM provider 路由，不是 agent 路由。

> **Clawtex 實作建議**:
> ```rust
> // src/agent_router.rs -- 未來多頻道 Agent 路由
> pub struct AgentBinding {
>     pub agent_id: String,
>     pub match_rule: BindingMatch,
> }
>
> pub enum BindingMatch {
>     Channel(String),                          // 匹配頻道
>     ChannelGroup(String, String),            // 頻道 + 群組
>     ChannelAccount(String, String),          // 頻道 + 帳號
> }
>
> pub fn resolve_agent_route(
>     bindings: &[AgentBinding],
>     channel: &str,
>     group_id: Option<&str>,
>     account_id: Option<&str>,
> ) -> &str {
>     for binding in bindings {
>         if binding.match_rule.matches(channel, group_id, account_id) {
>             return &binding.agent_id;
>         }
>     }
>     "default"
> }
> ```

---

## 11. Provider 整合

### 11.1 支援的 Provider

| Provider | 檔案 | 認證方式 |
|----------|------|----------|
| Anthropic | `auth-choice.apply.anthropic.ts` | API Key |
| OpenAI | `auth-choice.apply.openai.ts` | API Key |
| Ollama | `auth-choice.apply.ollama.ts` | 本地 |
| Google Gemini | `auth-choice.apply.google-gemini-cli.ts` | CLI OAuth |
| GitHub Copilot | `auth-choice.apply.github-copilot.ts` | OAuth Token |
| HuggingFace | `auth-choice.apply.huggingface.ts` | API Token |
| OpenRouter | `auth-choice.apply.openrouter.ts` | API Key |
| BytePlus/Volcengine | `auth-choice.apply.byteplus.ts` | API Key |
| vLLM | `auth-choice.apply.vllm.ts` | 本地 |
| MiniMax | `auth-choice.apply.minimax.ts` | API Key |
| xAI (Grok) | `auth-choice.apply.xai.ts` | API Key |
| Qwen Portal | `auth-choice.apply.qwen-portal.ts` | OAuth |
| AWS Bedrock | `bedrock-discovery.ts` | AWS IAM |
| Chutes | `chutes-oauth.ts` | OAuth |

### 11.2 Failover 機制

```typescript
type FailoverReason =
  | "auth" | "billing" | "rate_limit" | "overload"
  | "context_overflow" | "timeout" | "unknown";

const OVERLOAD_FAILOVER_BACKOFF_POLICY = {
  initialMs: 250, maxMs: 1500, factor: 2, jitter: 0.2
};
```

---

## 12. 串流處理

### 12.1 串流狀態機

```typescript
// src/agents/pi-embedded-subscribe.ts
const state: EmbeddedPiSubscribeState = {
  assistantTexts: [],
  toolMetas: [],
  deltaBuffer: "",
  blockBuffer: "",
  blockState: {
    thinking: false,
    final: false,
    inlineCode: createInlineCodeState(),
  },
  compactionInFlight: false,
};
```

### 12.2 Provider 串流適配器

```
src/agents/pi-embedded-runner/
├── anthropic-stream-wrappers.ts   -- Anthropic SSE
├── openai-stream-wrappers.ts      -- OpenAI SSE
├── moonshot-stream-wrappers.ts    -- Moonshot SSE
├── proxy-stream-wrappers.ts       -- 代理串流
└── ollama-stream.ts               -- Ollama JSONL
```

---

## 13. MCP 整合

```typescript
// MCP 工具透過 Plugin SDK 暴露
// 設定:
{
  "plugins": {
    "my-mcp-server": {
      "command": "npx",
      "args": ["my-mcp-server"],
      "tools": { "allow": ["*"] }
    }
  }
}
```

---

## 14. 工具系統

### 14.1 完整工具清單

| 類別 | 工具名稱 | 檔案 |
|------|----------|------|
| 檔案 | `read`, `write`, `edit`, `apply_patch` | pi-coding-agent |
| 執行 | `exec`, `process` | `bash-tools.exec.ts`, `bash-tools.process.ts` |
| Web | `web_search`, `web_fetch` | `tools/web-search.ts`, `tools/web-fetch.ts` |
| 記憶 | `memory_search`, `memory_get` | `tools/memory-tool.ts` |
| 會話 | `sessions_list/history/send/spawn`, `subagents`, `session_status` | `tools/sessions-*.ts` |
| UI | `browser`, `canvas` | `tools/browser-tool.ts`, `tools/canvas-tool.ts` |
| 訊息 | `message` | `tools/message-tool.ts` |
| 排程 | `cron` | `tools/cron-tool.ts` |
| 節點 | `nodes` | `tools/nodes-tool.ts` |
| Agent | `agents_list` | `tools/agents-list-tool.ts` |
| 媒體 | `image`, `pdf`, `tts` | `tools/image-tool.ts` 等 |

### 14.2 工具結果截斷

```typescript
// pi-embedded-runner/tool-result-truncation.ts
truncateOversizedToolResultsInSession(session, tokenBudget);
// 防止單一工具結果 (如 cat 大檔案) 佔用過多上下文
```

---

## 15. CLAUDE.md / 專案記憶

### 15.1 Bootstrap 檔案層級

```
AGENTS.md      -- 專案級指令 (等同 CLAUDE.md)
SOUL.md        -- Agent 性格/身份
IDENTITY.md    -- Agent 身份資訊
USER.md        -- 使用者偏好
TOOLS.md       -- 工具使用指南
HEARTBEAT.md   -- 心跳/定時任務指令
BOOTSTRAP.md   -- 額外啟動上下文
MEMORY.md      -- 主記憶
memory/*.md    -- 分類記憶
```

---

## 16. 錯誤處理策略

### 16.1 分層錯誤處理

```
Provider 錯誤
  ├── isFailoverError(err) -> 嘗試下一個 profile
  │   ├── auth       -> 立即切換
  │   ├── billing    -> 立即切換
  │   ├── rate_limit -> backoff 後切換
  │   ├── overload   -> backoff 後切換
  │   └── timeout    -> 切換
  └── 非 failover -> throw 給上層

工具錯誤
  ├── 工具 execute 拋出異常 -> 回傳 error tool_result
  ├── 工具結果超大 -> 截斷
  └── 迴圈偵測觸發 -> 注入警告或強制終止

上下文錯誤
  ├── context_overflow -> 觸發 compact
  ├── compact 失敗 -> 回退到 legacy 引擎
  └── 低於 16K tokens -> 警告
```

---

## 17. 效能分析

### 17.1 Token 節省技巧

1. **Skill 路徑壓縮**: `~` 替換 home dir，每 skill 省 5-6 tokens，共省 400-600 tokens
2. **二分搜尋 token 預算**: O(log N) 找到最大 skill 前綴
3. **Bootstrap 檔案快取**: inode/mtime 驗證避免重複讀取，每檔最大 2MB
4. **MMR 預 tokenize**: 一次性 tokenize 所有項目，避免重複計算

### 17.2 快取策略

| 快取 | 類型 | 上限 | 淘汰策略 |
|------|------|------|----------|
| 路由快取 | WeakMap<Config, Map> | 4000 entries | 超出時清空 |
| 綁定快取 | WeakMap<Config, Map> | 2000 entries | 超出時清空 |
| Auth Store 快照 | Map<string, Store> | N/A | 顯式 replace |
| Vault 讀取快取 | OnceLock<RwLock<Map>> | N/A | 指紋變更時失效 |

---

## 18. 與 clawtex-core 的完整差距對比

### 18.1 功能差距矩陣

| 功能 | OpenClaw | clawtex-core | 差距等級 |
|------|----------|--------------|----------|
| **ContextEngine trait** | 完整可插拔 (10 方法) | 具體實作 (context_compactor.rs) | **P0** |
| **Tool Policy Pipeline** | 7 步管線, glob, profile | 簡單 allow/deny | **P0** |
| **Skills 系統** | 50+ skills, 6 層來源, token 預算 | 無 (Hands 不同概念) | **P1** |
| **Auth Profile Rotation** | cooldown, 檔案鎖, 外部同步 | key_pool 基本輪轉 | **P1** |
| **Vector Memory** | SQLite-vec, MMR, 時間衰減 | key-value SQLite | **P1** |
| **Subagent Depth** | 深度限制, 工具裁剪, push 通知 | delegate 無限制 | **P0** |
| **Gateway 路由** | 7 層匹配, 多 Agent, 快取 | 單 Telegram | **P2** |
| **工具結果截斷** | 有 | 無 | **P0** |
| **Compaction 識別碼保留** | 明確指令 | 無 | **P0** |
| **Bootstrap 檔案** | 8 種檔案, 快取, 2MB 上限 | agents.toml system_prompt | **P2** |
| **迴圈偵測分級** | 4 種偵測器, 3 級閾值 | 2 種偵測器 | **P1** |
| **ACP 協議** | 完整實作 | 無 | **P3** |

### 18.2 clawtex-core 的獨有優勢

| clawtex-core 獨有 | 說明 |
|-------------------|------|
| Hands 工作流引擎 | 多階段、可鏈接的自動化工作流 |
| 叢集系統 | Hub + Worker 分散式架構 |
| Smart Routing | 請求分類器 + 複雜度路由 |
| Revenue Pipeline | 完整的收入追蹤 + SaaS 自動化 |
| SoT 引擎 | Skeleton-of-Thought 並行生成 |
| 24 個內建工具 | 遠超 OpenClaw 的工具多樣性 |
| ChatGPT Backend | Codex CLI subprocess 存取 |
| Approval Gate | Telegram 人工審核閘門 |
| E-Stop | 緊急停止機制 |
| Cost/Revenue Tracking | SQLite 成本和收入追蹤 |

---

## 19. 附錄：關鍵檔案路徑對照

| OpenClaw 檔案 | clawtex-core 對應 | 差距 |
|--------------|-------------------|------|
| `src/context-engine/types.ts` | `src/context_compactor.rs` | 需重構為 trait |
| `src/context-engine/registry.ts` | (缺少) | 需新增 |
| `src/agents/tool-policy-pipeline.ts` | (缺少) | 需新增 |
| `src/agents/pi-tools.policy.ts` | (部分在 tool_registry) | 需增強 |
| `src/agents/skills/workspace.ts` | (缺少) | 需新增 |
| `src/agents/skills/types.ts` | (缺少) | 需新增 |
| `src/agents/auth-profiles/store.ts` | `src/providers/key_pool.rs` | 需增強 |
| `src/memory/mmr.ts` | (缺少) | 需新增 |
| `src/memory/temporal-decay.ts` | (缺少) | 需新增 |
| `src/memory/sqlite-vec.ts` | (缺少) | 需新增 |
| `src/agents/subagent-depth.ts` | (缺少) | 需新增 |
| `src/agents/subagent-spawn.ts` | `src/tools/delegate.rs` | 需增強 |
| `src/agents/subagent-announce.ts` | (缺少) | 需新增 |
| `src/routing/resolve-route.ts` | (缺少) | 長期需新增 |
| `src/routing/bindings.ts` | (缺少) | 長期需新增 |
| `src/agents/tool-loop-detection.ts` | `src/loop_detection.rs` | 需增強 |
| `src/agents/compaction.ts` | `src/context_compactor.rs` | 需增強 |
| `src/agents/bootstrap-files.ts` | (缺少) | 需新增 |
| `src/agents/context-window-guard.ts` | `src/context.rs` | 需增強 |
| `src/agents/system-prompt.ts` | `src/agent_runtime.rs` | 部分對應 |

---

## 附錄: 程式碼規模統計

- **原始碼**: `src/` 下約 1,500+ TypeScript 檔案
- **agents/ 目錄**: 約 500+ 檔案 (核心)
- **memory/ 目錄**: 約 90+ 檔案 (向量記憶)
- **gateway/ 目錄**: 約 90+ 檔案 (API 閘道)
- **Skills**: 50+ 內建
- **測試**: 每個模組都有共存測試 (`*.test.ts`)

---

*此分析基於 OpenClaw v2026.3.11 原始碼深度閱讀。分析覆蓋 context-engine (6 檔), skills (18 檔), memory (90+ 檔), routing (3 檔), auth-profiles (10+ 檔), tool-policy (5 檔), subagent (8 檔)。*
