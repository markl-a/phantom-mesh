# Mastra 深度技術分析

> 分析日期: 2026-03-12
> 原始碼位置: `LLM-Cluster-Project/references/mastra/`
> Mastra 是 Gatsby.js 團隊打造的 TypeScript-native AI Agent 框架，核心特色為 Observational Memory（觀察式記憶），可將 token 成本壓縮 3-40 倍，在 LongMemEval 基準測試中取得 95% 的成績。

---

## 目錄

1. [專案結構](#1-專案結構)
2. [進入點與啟動流程](#2-進入點與啟動流程)
3. [核心架構](#3-核心架構)
4. [Observational Memory（最重要）](#4-observational-memory觀察式記憶)
5. [記憶體架構](#5-記憶體架構)
6. [與其他框架的整合](#6-與其他框架的整合)
7. [值得採用的關鍵模式](#7-值得採用的關鍵模式)
8. [移植至 clawtex-core (Rust) 的實作計畫](#8-移植至-clawtex-core-rust-的實作計畫)

---

## 1. 專案結構

### Monorepo 佈局

Mastra 是以 `pnpm` + `Turborepo` 管理的大型 monorepo，頂層目錄結構如下：

```
mastra/
├── packages/           # 核心套件
│   ├── core/           # @mastra/core — Agent、Tool、Workflow、Storage、Memory 基礎
│   ├── memory/         # @mastra/memory — Memory 實作（含 Observational Memory）
│   ├── rag/            # RAG pipeline
│   └── evals/          # 評估框架
├── auth/               # 認證整合（auth0, clerk, supabase, firebase, workos, better-auth）
├── client-sdks/        # 客戶端 SDK（ai-sdk, client-js, react）
├── deployers/          # 部署器（cloudflare, vercel, netlify, cloud）
├── stores/             # 儲存後端（pg, mongodb, mssql, libsql, upstash）
├── voice/              # 語音整合
├── workflows/          # 工作流引擎（含 inngest 整合）
├── workspaces/         # Workspace 抽象（daytona sandbox 等）
├── integrations/       # 第三方整合
├── observability/      # 可觀測性（OTel 整合）
├── pubsub/             # Pub/Sub 事件系統
├── server-adapters/    # 伺服器適配器（Hono 等）
├── templates/          # 專案模板（weather-agent, chat-with-youtube 等）
├── examples/           # 範例專案
├── e2e-tests/          # 端對端測試
└── docs/               # 文件網站
```

### 關鍵套件依賴關係

```
@mastra/core (基礎)
  ├── Agent、Tool、Workflow 定義
  ├── Memory 抽象 (MastraMemory)
  ├── Storage 抽象 (MastraCompositeStore)
  ├── Processor 系統 (Input/Output processors)
  └── LLM Model Router

@mastra/memory (記憶體實作)
  ├── Memory class (extends MastraMemory)
  ├── ObservationalMemory (Processor)
  ├── WorkingMemory
  └── SemanticRecall

@mastra/core/processors
  ├── MessageHistory
  ├── WorkingMemory
  └── SemanticRecall
```

### 核心原始碼位置

| 元件 | 路徑 |
|------|------|
| Mastra 類 | `packages/core/src/mastra/index.ts` |
| Agent 類 | `packages/core/src/agent/agent.ts` |
| Tool 系統 | `packages/core/src/tools/tool.ts` |
| Workflow | `packages/core/src/workflows/workflow.ts` |
| Memory 抽象 | `packages/core/src/memory/memory.ts` |
| Memory 實作 | `packages/memory/src/index.ts` |
| Observational Memory | `packages/memory/src/processors/observational-memory/` |
| Storage 抽象 | `packages/core/src/storage/` |
| Processor 系統 | `packages/core/src/processors/` |

---

## 2. 進入點與啟動流程

### Mastra 初始化

Mastra 的核心初始化在 `packages/core/src/mastra/index.ts` 的 `Mastra` class 中完成。它是一個中央依賴注入容器：

```typescript
// packages/core/src/mastra/index.ts (第 294 行起)
export class Mastra<
  TAgents extends Record<string, Agent<any>>,
  TWorkflows extends Record<string, AnyWorkflow>,
  TVectors extends Record<string, MastraVector<any>>,
  // ... 多重泛型參數
> {
  #agents: TAgents;
  #workflows: TWorkflows;
  #storage?: MastraCompositeStore;
  #memory?: TMemory;
  #tools?: TTools;
  #processors?: TProcessors;
  #mcpServers?: TMCPServers;
  // ...
}
```

### Agent 初始化流程

Agent 的典型建立方式：

```typescript
// 來自 packages/core/src/agent/agent.ts (第 147 行)
import { Agent } from '@mastra/core/agent';
import { Memory } from '@mastra/memory';

const agent = new Agent({
  id: 'my-agent',
  name: 'My Agent',
  instructions: 'You are a helpful assistant',
  model: 'openai/gpt-5',           // 字串格式: provider/model
  tools: {
    calculator: calculatorTool,     // Tool 物件
  },
  memory: new Memory(),             // 可選的記憶體
  inputProcessors: [om],            // 輸入處理器
  outputProcessors: [om],           // 輸出處理器
});
```

啟動流程概覽：

1. `Mastra` 建構子接收 agents、tools、workflows、storage 等配置
2. 各 Agent 透過 `__registerMastra(mastra)` 注入 Mastra 實例
3. Memory 透過 `__registerMastra(mastra)` 獲取 storage 參考
4. Processor 系統在 Agent 呼叫 `stream()` / `generate()` 時自動啟動
5. 每次 Agent 迴圈步驟都執行 `processInputStep()` → LLM 呼叫 → `processOutputResult()`

---

## 3. 核心架構

### 3.1 Agent 系統

**定義方式**：Agent 由 `AgentConfig` 配置物件驅動。

```typescript
// packages/core/src/agent/agent.ts
export class Agent<TAgentId, TTools, TOutput> extends MastraBase {
  // 核心方法
  async generate(messages, options?)    // 一次性生成
  async stream(messages, options?)      // 串流生成

  // 內部運作
  // 1. Processor Runner 呼叫所有 inputProcessors (含 ObservationalMemory)
  // 2. LLM 呼叫（透過 Model Router 路由至正確 provider）
  // 3. Tool 執行迴圈（若 LLM 回傳 tool calls）
  // 4. Processor Runner 呼叫所有 outputProcessors
}
```

**Model Router 模式**：模型以 `provider/model` 字串格式指定，例如 `'openai/gpt-5'`、`'google/gemini-2.5-flash'`、`'anthropic/claude-opus-4'`。`ModelRouterLanguageModel` 在 runtime 解析並路由至正確 provider。

### 3.2 Tool 系統

```typescript
// packages/core/src/tools/tool.ts (第 504 行)
import { createTool } from '@mastra/core/tools';
import { z } from 'zod';

const weatherTool = createTool({
  id: 'get-weather',
  description: 'Get current weather for a location',
  inputSchema: z.object({
    location: z.string(),
  }),
  outputSchema: z.object({
    temperature: z.number(),
    conditions: z.string(),
  }),
  execute: async ({ context, inputData }) => {
    // Tool 邏輯
    return { temperature: 22, conditions: 'sunny' };
  },
});
```

Tool 來源包括：
- **直接指派 tools**: Agent config 中的 `tools` 物件
- **Toolsets**: 動態 tool 集合
- **Memory tools**: 由 Memory 提供的 working memory 更新 tool
- **MCP tools**: 透過 MCP Server 的外部 tool
- **Workspace tools**: 檔案操作、命令執行等 workspace tools

### 3.3 Provider 整合

Mastra 使用「Model Router」模式而非直接 provider 對接：

```
Agent.model = "openai/gpt-5"
                ↓
         ModelRouterLanguageModel
                ↓
         resolveModelConfig()
                ↓
    Provider-specific SDK adapter (ai-sdk compatible)
```

支援的 provider（透過 `provider/model` 字串）：
- OpenAI (`openai/gpt-5`, `openai/gpt-4o`)
- Google (`google/gemini-2.5-flash`, `google/gemini-3-pro`)
- Anthropic (`anthropic/claude-opus-4`)
- 以及 ai-sdk 支援的所有 provider

### 3.4 Workflow 系統

Mastra 的 Workflow 基於 step-based 執行，支援 suspend/resume：

```typescript
// packages/core/src/workflows/workflow.ts
import { createWorkflow, createStep } from '@mastra/core/workflows';

const myWorkflow = createWorkflow({
  id: 'my-workflow',
  steps: [
    createStep({
      id: 'step1',
      inputSchema: z.object({ query: z.string() }),
      outputSchema: z.object({ result: z.string() }),
      execute: async ({ inputData, suspend }) => {
        // 可暫停等待人工核准
        const approval = await suspend({ reason: 'Need human approval' });
        return { result: 'done' };
      },
    }),
  ],
});
```

---

## 4. Observational Memory（觀察式記憶）

> **這是 Mastra 最核心的創新。** 整個系統設計巧妙，值得深入理解。

### 4.1 三代理人架構 (Three-Agent Architecture)

Observational Memory 在主要 Agent 之外引入兩個「心靈」代理人：

```
┌──────────────────────────────────────────────┐
│                  Actor (主 Agent)              │
│  看到：觀察記錄 + 建議的延續 + 未觀察的最近訊息  │
└──────────┬────────────────┬───────────────────┘
           │                │
    ┌──────▼──────┐  ┌──────▼──────┐
    │  Observer    │  │  Reflector  │
    │ (觀察者)     │  │ (反思者)    │
    │ 提取觀察     │  │ 壓縮觀察    │
    │ 30K token    │  │ 40K token   │
    │ 門檻觸發     │  │ 門檻觸發    │
    └─────────────┘  └─────────────┘
```

**核心檔案位置**：
```
packages/memory/src/processors/observational-memory/
├── observational-memory.ts    # 主 Processor 類別（~3500 行）
├── observer-agent.ts          # Observer 的 system prompt 與解析器
├── reflector-agent.ts         # Reflector 的 system prompt 與解析器
├── types.ts                   # 型別定義（所有 data parts）
├── thresholds.ts              # 動態門檻計算
├── token-counter.ts           # Token 計數器（含圖片估算）
├── markers.ts                 # 觀察邊界標記工廠
├── operation-registry.ts      # 進程級操作追蹤
├── date-utils.ts              # 相對時間格式化
└── repro-capture.ts           # 除錯重現擷取
```

### 4.2 Observer Agent（觀察者）

**檔案**: `packages/memory/src/processors/observational-memory/observer-agent.ts`

Observer 是一個專門的 Agent，負責觀察對話歷史並提取結構化觀察記錄。

#### System Prompt 結構

Observer 的 system prompt 分為四個核心部分：

**1. EXTRACTION_INSTRUCTIONS**（提取指導）— 定義「觀察什麼」：

```
CRITICAL: DISTINGUISH USER ASSERTIONS FROM QUESTIONS
- "I have two kids" → 🔴 (14:30) User stated has two kids    (斷言)
- "Can you help me with X?" → 🔴 (15:00) User asked help with X (提問)

STATE CHANGES AND UPDATES:
- "I'm switching from A to B" → "User is switching from A to B"

TEMPORAL ANCHORING:
- 每個觀察有兩個潛在時間戳
  1. BEGINNING: 訊息發送時間（始終包含）
  2. END: 被參考的時間（僅在有相對時間參考時）

PRESERVE UNUSUAL PHRASING:
- BAD: User exercised.
- GOOD: User stated they did a "movement session" (their term for exercise).

PRESERVING DETAILS IN ASSISTANT-GENERATED CONTENT:
- 推薦清單保留區分每個項目的關鍵屬性
- 名稱、帳號、識別碼必須保留
- 數值/技術結果保留具體數值
```

**2. OUTPUT_FORMAT**（輸出格式）— 定義「怎麼輸出」：

```xml
使用優先級別：
- 🔴 High: 明確的使用者事實、偏好、達成的目標、關鍵上下文
- 🟡 Medium: 專案細節、學到的資訊、工具結果
- 🟢 Low: 細微細節、不確定的觀察

<observations>
Date: Dec 4, 2025
* 🔴 (14:30) User prefers direct answers
* 🟡 (14:32) Agent debugging auth issue
  * -> ran git status, found 3 modified files
  * -> viewed auth.ts:45-60, found missing null check
</observations>

<current-task>
State the current task(s) explicitly.
</current-task>

<suggested-response>
Hint for the agent's immediate next message.
</suggested-response>
```

**3. GUIDELINES**（指導原則）— 定義「品質標準」：

```
- Be specific enough for the assistant to act on
- Add 1 to 5 observations per exchange
- Use terse language to save tokens
- Do not add repetitive observations
- Group repeated similar actions under a single parent with sub-bullets
- User messages are always 🔴 priority
```

#### Observer 呼叫機制

```typescript
// observational-memory.ts 第 1665 行
private async callObserver(
  existingObservations: string | undefined,
  messagesToObserve: MastraDBMessage[],
  abortSignal?: AbortSignal,
  options?: { skipContinuationHints?: boolean; requestContext?: RequestContext },
) {
  const agent = this.getObserverAgent();

  const observerMessages = [
    { role: 'user', content: buildObserverTaskPrompt(existingObservations, options) },
    buildObserverHistoryMessage(messagesToObserve),
  ];

  const streamResult = await agent.stream(observerMessages, {
    modelSettings: { ...this.observationConfig.modelSettings },
    providerOptions: this.observationConfig.providerOptions,
  });

  const result = await streamResult.getFullOutput();
  const parsed = parseObserverOutput(result.text);

  // 偵測退化重複（degenerate repetition）並重試一次
  if (parsed.degenerate) {
    result = await doGenerate();
    parsed = parseObserverOutput(result.text);
  }

  return {
    observations: parsed.observations,
    currentTask: parsed.currentTask,
    suggestedContinuation: parsed.suggestedContinuation,
  };
}
```

### 4.3 Reflector Agent（反思者）

**檔案**: `packages/memory/src/processors/observational-memory/reflector-agent.ts`

Reflector 負責「元觀察」——當觀察記錄本身增長過大時，將它們重組為更精煉的版本。

#### System Prompt 核心

```
You are the memory consciousness of an AI assistant.
Your memory observation reflections will be the ONLY information
the assistant has about past interactions with this user.

Your reason for existing is to reflect on all the observations,
re-organize and streamline them, and draw connections and conclusions.

IMPORTANT: your reflections are THE ENTIRETY of the assistants memory.
Any information you do not add to your reflections will be immediately forgotten.
```

#### 壓縮級別（Compression Levels）

Reflector 使用漸進式壓縮策略。如果第一次壓縮不夠，會逐級加壓：

| 級別 | 目標細節度 | 策略 |
|------|-----------|------|
| Level 0 | 10/10 | 無壓縮指導（首次反思） |
| Level 1 | 8/10 | 輕度壓縮：開頭更聚合，結尾保留細節 |
| Level 2 | 6/10 | 積極壓縮：大幅聚合，移除冗餘 |
| Level 3 | 4/10 | 極限壓縮：前 50-70% 摘要化，僅保留關鍵事實 |

```typescript
// reflector-agent.ts 第 138 行
export const COMPRESSION_GUIDANCE: Record<0 | 1 | 2 | 3, string> = {
  0: '',
  1: `Your current detail level was a 10/10, lets aim for a 8/10 detail level.`,
  2: `Your current detail level was a 10/10, lets aim for a 6/10 detail level.`,
  3: `Your current detail level was a 10/10, lets aim for a 4/10 detail level.`,
};
```

#### 壓縮驗證

```typescript
// reflector-agent.ts 第 334 行
export function validateCompression(reflectedTokens: number, targetThreshold: number): boolean {
  return reflectedTokens < targetThreshold;
}
```

Reflector 呼叫流程：
1. 以 Level 0 開始反思
2. 如果壓縮後 token 數仍超過門檻，升級到 Level 1 重試
3. 最高重試至 Level 3
4. 每次重試都檢測「退化重複」（degenerate repetition）

### 4.4 Token 門檻與觸發機制

**檔案**: `packages/memory/src/processors/observational-memory/thresholds.ts`

#### 預設配置

```typescript
// observational-memory.ts 第 251 行
export const OBSERVATIONAL_MEMORY_DEFAULTS = {
  observation: {
    model: 'google/gemini-2.5-flash',
    messageTokens: 30_000,          // 30K tokens 觸發 Observer
    modelSettings: {
      temperature: 0.3,
      maxOutputTokens: 100_000,
    },
    providerOptions: {
      google: { thinkingConfig: { thinkingBudget: 215 } },
    },
    maxTokensPerBatch: 10_000,
    bufferTokens: 0.2,              // 每 20% 緩衝一次
    bufferActivation: 0.8,          // 啟用後保留 20% 原始訊息
  },
  reflection: {
    model: 'google/gemini-2.5-flash',
    observationTokens: 40_000,      // 40K tokens 觸發 Reflector
    modelSettings: {
      temperature: 0,               // 反思需要最大一致性
      maxOutputTokens: 100_000,
    },
    providerOptions: {
      google: { thinkingConfig: { thinkingBudget: 1024 } },
    },
    bufferActivation: 0.5,          // 50% 時開始背景反思
  },
};
```

#### 動態門檻（Share Token Budget）

當 `shareTokenBudget: true` 時，訊息和觀察可以共享 token 預算：

```typescript
// thresholds.ts 第 28 行
export function calculateDynamicThreshold(
  threshold: number | ThresholdRange,
  currentObservationTokens: number,
): number {
  if (typeof threshold === 'number') return threshold;

  // 總預算 = 訊息門檻 + 觀察門檻（例如 30K + 40K = 70K）
  const totalBudget = threshold.max;
  const baseThreshold = threshold.min;

  // 有效門檻 = 總預算 - 當前觀察 tokens
  // 但永遠不低於基礎門檻
  return Math.max(totalBudget - currentObservationTokens, baseThreshold);
}
```

範例（30K:40K 門檻，70K 總預算）：
- 0 觀察 → 訊息可用 ~70K
- 10K 觀察 → 訊息可用 ~60K
- 40K 觀察 → 訊息回到 ~30K

### 4.5 非同步緩衝機制（Async Buffering）

這是 Observational Memory 最精巧的設計之一。在主門檻到達之前，系統就開始在背景預先計算觀察。

#### 觀察緩衝流程

```
Token 累積                           緩衝事件
────────────────────────────────────────────────────
0K         ───────────────────────── (閒置)
6K (20%)   ───────────────────────── 🔄 Buffer #1 開始（背景）
12K (40%)  ───────────────────────── 🔄 Buffer #2 開始
18K (60%)  ───────────────────────── 🔄 Buffer #3 開始
24K (80%)  ───────────────────────── 🔄 Buffer #4 開始
                                      (間距減半 → 更細粒度)
27K (90%)  ───────────────────────── 🔄 Buffer #5 開始
30K (100%) ───────────────────────── ⚡ 立即啟用！
                                      (無需等待 LLM 呼叫)
```

```typescript
// observational-memory.ts 第 620 行
private shouldTriggerAsyncObservation(
  currentTokens: number,
  lockKey: string,
  record: ObservationalMemoryRecord,
  messageTokensThreshold?: number,
): boolean {
  const bufferTokens = this.observationConfig.bufferTokens!;

  // 接近門檻時，間距減半以產生更細粒度的 chunks
  const rampPoint = messageTokensThreshold
    ? messageTokensThreshold - bufferTokens * 1.1
    : Infinity;
  const effectiveBufferTokens = currentTokens >= rampPoint
    ? bufferTokens / 2
    : bufferTokens;

  // 計算是否跨越新的間隔邊界
  const currentInterval = Math.floor(currentTokens / effectiveBufferTokens);
  const lastInterval = Math.floor(lastBoundary / effectiveBufferTokens);

  return currentInterval > lastInterval;
}
```

#### 啟用機制（Activation）

```
bufferActivation: 0.8 表示：
- 啟用 80% 的 buffer 內容
- 保留 20% 的原始訊息（retention floor）

例：messageTokens = 30K, bufferActivation = 0.8
→ retention floor = 30K * (1 - 0.8) = 6K
→ 啟用時清除 ~24K 的原始訊息，保留 ~6K
```

#### Chunk 邊界選擇

```typescript
// thresholds.ts 第 115 行
export function calculateProjectedMessageRemoval(
  chunks: BufferedObservationChunk[],
  bufferActivation: number,
  messageTokensThreshold: number,
  currentPendingTokens: number,
): number {
  const retentionFloor = resolveRetentionFloor(bufferActivation, messageTokensThreshold);
  const targetMessageTokens = Math.max(0, currentPendingTokens - retentionFloor);

  // 找到最接近目標的 chunk 邊界
  // 偏向「多移除一點」（over）而非「少移除一點」（under）
  // 但有保護措施防止過度移除
  // ...
}
```

### 4.6 壓縮比率：如何達到 3-40x

壓縮比率取決於對話內容類型：

| 場景 | 原始 Tokens | 觀察 Tokens | 壓縮比 | 說明 |
|------|------------|------------|--------|------|
| 重複工具呼叫 | 30,000 | ~750 | 40x | 大量 tool call 聚合為「Agent 呼叫 X 工具 5 次」 |
| 程式碼除錯 | 30,000 | ~2,000 | 15x | 保留關鍵發現，壓縮中間步驟 |
| 一般對話 | 30,000 | ~5,000 | 6x | 保留使用者偏好與事實 |
| 資訊密集 | 30,000 | ~10,000 | 3x | 大量具體數據需要保留 |

壓縮的核心機制：

1. **觀察階段**：30K 原始對話 → ~5K 結構化觀察（6x 初始壓縮）
2. **反思階段**：40K 累積觀察 → ~20K 精煉觀察（2x 追加壓縮）
3. **漸進式壓縮**：反思可重複 4 次（Level 0→3），每次進一步壓縮
4. **時序優先**：舊觀察壓縮更積極，新觀察保留更多細節

### 4.7 Prompt Caching 與穩定上下文

Observational Memory 的設計天然支援 prompt caching：

```
┌─────────────────────────────────────────┐
│ System prompt (固定)                     │  ← 可快取
├─────────────────────────────────────────┤
│ Observations (觀察記錄)                  │  ← 相對穩定，可快取
│ <observations>                          │
│   Date: Jan 15, 2026 (2 months ago)     │
│   * 🔴 User prefers TypeScript          │
│   ...                                   │
│ </observations>                         │
├─────────────────────────────────────────┤
│ Continuation hint (延續提示)             │  ← 較穩定
├─────────────────────────────────────────┤
│ Recent unobserved messages (最近訊息)    │  ← 變動區域
└─────────────────────────────────────────┘
```

觀察記錄注入方式（`injectObservationsIntoContext`）：

```typescript
// observational-memory.ts 中的常量
export const OBSERVATION_CONTEXT_PROMPT =
  `The following observations block contains your memory of past conversations with this user.`;

export const OBSERVATION_CONTEXT_INSTRUCTIONS = `
IMPORTANT: When responding, reference specific details from these observations.
KNOWLEDGE UPDATES: prefer the MOST RECENT information.
PLANNED ACTIONS: If the date has passed, assume they completed the action.
MOST RECENT USER INPUT: Treat as highest-priority signal.`;

// 注入格式
// ${OBSERVATION_CONTEXT_PROMPT}
// <observations>
// ${observations_with_relative_time}
// </observations>
// ${OBSERVATION_CONTEXT_INSTRUCTIONS}
```

**Continuation Hint**（延續提示）防止 Agent 在觀察啟用後行為不自然：

```typescript
export const OBSERVATION_CONTINUATION_HINT = `
This message is not from the user, the conversation history grew too long
and wouldn't fit in context! Thankfully the entire conversation is stored
in your memory observations. Please continue from where the observations
left off. Do not refer to your "memory observations" directly, the user
doesn't know about them, they are your memories!
...
NOTE: Any messages following this system reminder are newer than your memories.`;
```

### 4.8 LongMemEval 95% 的設計決策

1. **使用者斷言優先**：Observer 區分 assertion vs question，Reflector 在聚合時保留 assertion
2. **時序錨定**：所有觀察附帶精確時間戳 + 相對時間（`addRelativeTimeToObservations`）
3. **狀態變更追蹤**：「User switched from A to B」明確標記舊資訊被取代
4. **精確語言保留**：保留使用者原始用詞（「movement session」而非「exercise」）
5. **detail-level 觀察**：推薦清單保留每項的區分特徵，而非僅記錄「推薦了 5 間旅館」
6. **數值精確**：保留具體數字（「43.7% faster load times」）
7. **分割多事件**：單一陳述中的多個事件拆分為獨立觀察，各自附帶時間標記
8. **退化偵測**：`detectDegenerateRepetition()` 偵測 LLM 輸出的重複迴圈並重試

### 4.9 processInputStep 完整流程

```
processInputStep(step N)
│
├── STEP 1: 載入歷史訊息（step 0 only）
│   └── loadHistoricalMessagesIfNeeded()
│
├── STEP 1b: 載入其他 thread 的上下文（resource scope）
│   └── loadOtherThreadsContext()
│
├── STEP 1c: 啟用緩衝觀察（step 0 only）
│   ├── 計算 totalPendingTokens vs threshold
│   ├── if (pendingTokens >= threshold)
│   │   └── tryActivateBufferedObservations()
│   │       ├── 從 buffer 中選擇 chunks（chunk 邊界選擇）
│   │       ├── 合併觀察到 activeObservations
│   │       ├── 更新 lastObservedAt 游標
│   │       └── 移除已觀察的訊息
│   └── 檢查是否需要反思
│       └── maybeReflect() → callReflector()
│
├── STEP 1d: 反思檢查（step 0 only）
│   └── if (obsTokens > reflectionThreshold)
│       └── maybeReflect()
│
├── STEP 2: 檢查門檻 & 觸發觀察
│   ├── 計算 totalPendingTokens vs threshold
│   ├── ASYNC BUFFERING: 門檻以下時
│   │   └── shouldTriggerAsyncObservation()
│   │       └── startAsyncBufferedObservation()（背景）
│   ├── THRESHOLD REACHED (step > 0):
│   │   ├── handleThresholdReached()
│   │   │   ├── 嘗試啟用 buffer → tryActivateBufferedObservations()
│   │   │   ├── 或同步觀察 → callObserver() (blocking)
│   │   │   └── 更新 storage record
│   │   └── cleanupAfterObservation()（移除已觀察訊息）
│   └── ASYNC REFLECTION: 觀察 tokens 超過啟用點
│       └── maybeAsyncReflect()
│
├── STEP 3: 注入觀察到上下文
│   └── injectObservationsIntoContext()
│       ├── 載入 activeObservations
│       ├── addRelativeTimeToObservations()（加入相對時間）
│       ├── optimizeObservationsForContext()（根據可用空間最佳化）
│       ├── 在訊息前插入觀察 + continuation hint
│       └── 注入其他 thread 的上下文（resource scope）
│
├── STEP 4: 過濾已觀察的訊息
│   └── filterAlreadyObservedMessages()
│       ├── marker boundary pruning（step 0）
│       └── timestamp-based filtering（step > 0）
│
└── STEP 5: 發送最終狀態
    └── emitStepProgress()（DataOmStatusPart）
```

### 4.10 processOutputResult 流程

```
processOutputResult()
│
├── 檢查 readOnly
├── 收集未儲存的訊息（input + response）
├── saveMessagesWithSealedIdTracking()
│   ├── 跳過已 seal 且無觀察標記的訊息（已由 buffer 儲存）
│   └── upsert 有觀察標記的訊息
└── 返回 messageList
```

---

## 5. 記憶體架構

### 記憶體類型

Mastra 有三種記憶體機制：

| 類型 | 用途 | 實作 |
|------|------|------|
| Message History | 最近 N 筆訊息 | `MessageHistory` processor |
| Working Memory | 結構化的當前狀態 | `WorkingMemory` processor（tool-call 模式） |
| Observational Memory | 長期對話壓縮 | `ObservationalMemory` processor |
| Semantic Recall | 語義相似性搜索 | `SemanticRecall` processor |

### Memory 與 Agent Loop 的整合

```typescript
// 在 Agent 的每次迴圈步驟中
// (packages/core/src/agent/agent.ts)

// 1. 解析 processors（含 memory 提供的 processors）
const inputProcessors = await memory.getInputProcessors(configuredProcessors, context);
const outputProcessors = await memory.getOutputProcessors(configuredProcessors, context);

// 2. ProcessorRunner 執行流程
//    對每個 step:
//    a) 執行所有 inputProcessors.processInputStep()
//    b) LLM 呼叫
//    c) 執行所有 outputProcessors.processOutputResult()
```

### ObservationalMemory 取代 MessageHistory

當 ObservationalMemory 啟用時，它會接管 MessageHistory 的職責：

```typescript
// packages/core/src/memory/memory.ts 第 680 行
// Check if ObservationalMemory is present — it handles its own message loading and saving
const hasObservationalMemory =
  configuredProcessors.some(p => p.id === 'observational-memory') ||
  isObservationalMemoryEnabled(effectiveConfig.observationalMemory);

// Skip MessageHistory if ObservationalMemory handles message saving
if (!hasMessageHistory && !hasObservationalMemory) {
  processors.push(new MessageHistory({ storage: memoryStore }));
}
```

### 儲存層

Observational Memory 的儲存記錄結構 (`ObservationalMemoryRecord`)：

```typescript
interface ObservationalMemoryRecord {
  id: string;
  threadId: string | null;
  resourceId: string;
  scope: 'thread' | 'resource';
  activeObservations: string;          // 當前活躍的觀察文字
  observationTokenCount: number;       // 觀察的 token 數
  pendingMessageTokens: number;        // 待觀察的訊息 token 數
  lastObservedAt: Date;                // 最後觀察的時間游標
  observedMessageIds: string[];        // 已觀察的訊息 ID 列表
  generationCount: number;             // 反思代數（每次反思 +1）
  isObserving: boolean;                // 觀察進行中旗標
  isReflecting: boolean;               // 反思進行中旗標
  isBufferingObservation: boolean;     // 緩衝觀察進行中旗標
  isBufferingReflection: boolean;      // 緩衝反思進行中旗標
  lastBufferedAtTokens: number;        // 最後緩衝時的 token 數
  bufferedObservationChunks: BufferedObservationChunk[];  // 緩衝的觀察 chunks
  bufferedReflection: string | null;   // 緩衝的反思結果
  config: object;                      // 配置快照
  observedTimezone: string;            // 觀察時區
}
```

---

## 6. 與其他框架的整合

### Vercel AI SDK 整合

Mastra 的 client SDK 提供 AI SDK 相容層：

**位置**: `client-sdks/ai-sdk/` 和 `client-sdks/react/`

```
client-sdks/
├── ai-sdk/         # @mastra/ai-sdk — Vercel AI SDK adapter
├── client-js/      # @mastra/client-js — JavaScript client
└── react/          # @mastra/react — React hooks
    └── src/lib/ai-sdk/memory/
        └── resolveInitialMessages.ts  # 記憶體訊息解析
```

### LangChain 相容性

Mastra 本身不直接包含 LangChain plugin，但其 Tool 系統與 Vercel AI SDK 相容，可透過 AI SDK 的 LangChain 互操作層橋接。

---

## 7. 值得採用的關鍵模式

### 7.1 Observational Memory 設計（取代 context_compactor.rs）

**現況**: clawtex-core 的 `context_compactor.rs` 使用三級壓縮（Light/Medium/Aggressive），但這是基於文字截斷的粗糙方法。

**Mastra 優勢**:
- **語義壓縮** vs 截斷壓縮：Observer 理解對話語義，Reflector 進行有意義的重組
- **漸進式** vs 一次性：buffer → observe → reflect 的多階段漸進壓縮
- **可恢復** vs 不可逆：觀察記錄持久化到 storage，反思也保留完整歷史
- **非同步** vs 同步：背景緩衝避免阻塞主 Agent 迴圈

### 7.2 Observer/Reflector 雙代理人模式

這是整個系統最優雅的設計。關鍵洞察：

1. **關注點分離**：Observer 負責「看到什麼」，Reflector 負責「理解什麼」
2. **不同的溫度設定**：Observer 用 `temperature: 0.3`（需要創造性提取），Reflector 用 `temperature: 0`（需要最大一致性）
3. **不同的 thinking budget**：Observer 用 215 tokens 思考，Reflector 用 1024 tokens 思考
4. **互相了解**：Reflector 的 prompt 包含 Observer 的 extraction instructions，所以理解觀察是怎麼產生的

### 7.3 Memory 作為 Append-Only Log with GC

觀察記錄的生命週期：

```
Observation v1 (gen 0)
  ├── 新觀察追加...
  ├── 新觀察追加...
  └── 達到 40K → Reflection → Observation v2 (gen 1)
                                ├── 新觀察追加...
                                └── 達到 40K → Reflection → Observation v3 (gen 2)
                                                            └── ...
```

每次反思就是一次 GC：
- 保留所有重要資訊
- 壓縮舊的觀察
- 保留新的觀察細節
- `generationCount` 追蹤反思代數

### 7.4 Marker-Based Observation Boundaries

使用 data parts 標記觀察邊界是一個巧妙的設計：

```
Message parts: [text, text, 🏷️START, 🏷️END, text, text]
                              │         │
                              └─────────┘
                              已觀察的部分
```

這允許：
- 單一訊息中的 part-level 觀察追蹤
- 正在進行的觀察偵測（START without END）
- 失敗恢復（START + FAILED）
- 跨頁面重載的標記持久化

### 7.5 Sealed Messages 機制

為了避免非同步緩衝與訊息更新之間的競爭條件，Mastra 引入了 message sealing：

```typescript
// 標記訊息為 sealed，防止新的 parts 被合併進來
msg.content.metadata.mastra.sealed = true;
lastPart.metadata.mastra.sealedAt = Date.now();
```

當 AI SDK 嘗試合併具有相同 ID 的 sealed 訊息時，MessageList 會建立一個新訊息，只包含 seal boundary 之後的 parts。

---

## 8. 移植至 clawtex-core (Rust) 的實作計畫

### Phase 1: 核心資料結構（2-3 天）

#### 1.1 ObservationalMemoryRecord

```rust
// src/observational_memory/record.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationalMemoryRecord {
    pub id: String,
    pub thread_id: Option<String>,
    pub resource_id: String,
    pub scope: ObservationalMemoryScope,
    pub active_observations: String,
    pub observation_token_count: u32,
    pub pending_message_tokens: u32,
    pub last_observed_at: Option<DateTime<Utc>>,
    pub observed_message_ids: Vec<String>,
    pub generation_count: u32,
    pub is_observing: bool,
    pub is_reflecting: bool,
    pub is_buffering_observation: bool,
    pub is_buffering_reflection: bool,
    pub last_buffered_at_tokens: u32,
    pub buffered_observation_chunks: Vec<BufferedObservationChunk>,
    pub buffered_reflection: Option<String>,
    pub observed_timezone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ObservationalMemoryScope {
    Thread,
    Resource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferedObservationChunk {
    pub observations: String,
    pub message_tokens: u32,
    pub observation_tokens: u32,
    pub message_ids: Vec<String>,
    pub message_token_counts: HashMap<String, u32>,
    pub current_task: Option<String>,
    pub suggested_continuation: Option<String>,
}
```

#### 1.2 配置結構

```rust
// src/observational_memory/config.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationalMemoryConfig {
    pub observation: ObservationConfig,
    pub reflection: ReflectionConfig,
    pub scope: ObservationalMemoryScope,
    pub share_token_budget: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationConfig {
    pub model: String,                    // e.g., "google/gemini-2.5-flash"
    pub message_tokens: u32,              // 預設 30_000
    pub temperature: f32,                 // 預設 0.3
    pub max_output_tokens: u32,           // 預設 100_000
    pub buffer_tokens_ratio: f32,         // 預設 0.2
    pub buffer_activation: f32,           // 預設 0.8
    pub block_after_multiplier: f32,      // 預設 1.2
    pub instruction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionConfig {
    pub model: String,
    pub observation_tokens: u32,          // 預設 40_000
    pub temperature: f32,                 // 預設 0.0
    pub max_output_tokens: u32,           // 預設 100_000
    pub buffer_activation: f32,           // 預設 0.5
    pub block_after_multiplier: f32,      // 預設 1.2
    pub instruction: Option<String>,
}

impl Default for ObservationalMemoryConfig {
    fn default() -> Self {
        Self {
            observation: ObservationConfig {
                model: "google/gemini-2.5-flash".into(),
                message_tokens: 30_000,
                temperature: 0.3,
                max_output_tokens: 100_000,
                buffer_tokens_ratio: 0.2,
                buffer_activation: 0.8,
                block_after_multiplier: 1.2,
                instruction: None,
            },
            reflection: ReflectionConfig {
                model: "google/gemini-2.5-flash".into(),
                observation_tokens: 40_000,
                temperature: 0.0,
                max_output_tokens: 100_000,
                buffer_activation: 0.5,
                block_after_multiplier: 1.2,
                instruction: None,
            },
            scope: ObservationalMemoryScope::Thread,
            share_token_budget: false,
        }
    }
}
```

### Phase 2: Observer 與 Reflector Prompts（1 天）

```rust
// src/observational_memory/observer.rs

pub fn build_observer_system_prompt(instruction: Option<&str>) -> String {
    format!(
        r#"You are the memory consciousness of an AI assistant. Your observations will be the ONLY information the assistant has about past interactions with this user.

Extract observations that will help the assistant remember:

{OBSERVER_EXTRACTION_INSTRUCTIONS}

=== OUTPUT FORMAT ===
{OBSERVER_OUTPUT_FORMAT}

=== GUIDELINES ===
{OBSERVER_GUIDELINES}

{custom_instruction}

Remember: These observations are the assistant's ONLY memory. Make them count."#,
        custom_instruction = instruction
            .map(|i| format!("\n=== CUSTOM INSTRUCTIONS ===\n\n{i}"))
            .unwrap_or_default()
    )
}

pub fn parse_observer_output(output: &str) -> ObserverResult {
    // 使用 regex 解析 <observations>, <current-task>, <suggested-response>
    // 偵測退化重複
    // 清理觀察行
    // ...
}

// src/observational_memory/reflector.rs

pub fn build_reflector_system_prompt(instruction: Option<&str>) -> String { /* ... */ }
pub fn build_reflector_prompt(observations: &str, compression_level: u8) -> String { /* ... */ }
pub fn parse_reflector_output(output: &str) -> ReflectorResult { /* ... */ }
pub fn validate_compression(reflected_tokens: u32, target_threshold: u32) -> bool {
    reflected_tokens < target_threshold
}
```

### Phase 3: Token 計數與門檻（1 天）

```rust
// src/observational_memory/token_counter.rs

use tiktoken_rs::p50k_base;  // 或 tokenx crate

pub struct TokenCounter {
    encoding: CoreBPE,
}

impl TokenCounter {
    pub fn count_string(&self, text: &str) -> u32 {
        self.encoding.encode_ordinary(text).len() as u32
    }

    pub fn count_messages(&self, messages: &[Message]) -> u32 {
        messages.iter().map(|m| self.count_message(m)).sum()
    }
}

// src/observational_memory/thresholds.rs

pub fn calculate_dynamic_threshold(
    message_tokens: ThresholdRange,
    current_observation_tokens: u32,
) -> u32 {
    let total_budget = message_tokens.max;
    let base_threshold = message_tokens.min;
    std::cmp::max(total_budget.saturating_sub(current_observation_tokens), base_threshold)
}

pub fn resolve_retention_floor(buffer_activation: f32, message_tokens_threshold: u32) -> u32 {
    if buffer_activation >= 1000.0 {
        return buffer_activation as u32;
    }
    let ratio = buffer_activation.clamp(0.0, 1.0);
    (message_tokens_threshold as f32 * (1.0 - ratio)) as u32
}
```

### Phase 4: 核心 Processor 邏輯（3-4 天）

```rust
// src/observational_memory/processor.rs

use tokio::sync::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

pub struct ObservationalMemory {
    config: ObservationalMemoryConfig,
    storage: Arc<dyn MemoryStorage>,
    token_counter: TokenCounter,
    locks: Mutex<HashMap<String, ()>>,
    // 靜態狀態（跨實例共享）
    buffering_ops: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
    last_buffered_boundary: Arc<Mutex<HashMap<String, u32>>>,
}

impl ObservationalMemory {
    /// 每個 agent 迴圈步驟的輸入處理
    pub async fn process_input_step(
        &self,
        messages: &mut Vec<Message>,
        thread_id: &str,
        resource_id: &str,
        step_number: u32,
    ) -> Result<()> {
        let mut record = self.get_or_create_record(thread_id, resource_id).await?;

        // Step 1: 載入歷史（step 0 only）
        if step_number == 0 {
            self.load_historical_messages(messages, thread_id, &record).await?;
        }

        // Step 1c: 啟用緩衝觀察（step 0 only）
        if step_number == 0 {
            let pending = self.token_counter.count_messages(messages);
            let threshold = self.calculate_threshold(&record);
            if pending >= threshold {
                record = self.try_activate_buffered(&record, messages).await?;
            }
        }

        // Step 2: 檢查門檻
        let pending = self.calculate_pending_tokens(messages, &record);
        let threshold = self.calculate_threshold(&record);

        // 非同步緩衝（門檻以下）
        if pending < threshold {
            if self.should_trigger_async_observation(pending, &record) {
                self.start_async_buffered_observation(&record, messages).await;
            }
        }

        // 同步觀察（門檻以上，step > 0）
        if step_number > 0 && pending >= threshold {
            record = self.handle_threshold_reached(&record, messages).await?;
        }

        // Step 3: 注入觀察
        self.inject_observations(messages, &record, thread_id).await?;

        // Step 4: 過濾已觀察訊息
        self.filter_observed_messages(messages, &record);

        Ok(())
    }

    /// Agent 迴圈結束時的輸出處理
    pub async fn process_output_result(
        &self,
        messages: &[Message],
        thread_id: &str,
        resource_id: &str,
    ) -> Result<()> {
        // 儲存未保存的訊息
        self.save_messages(messages, thread_id, resource_id).await?;
        Ok(())
    }

    /// 呼叫 Observer Agent
    async fn call_observer(
        &self,
        existing_observations: Option<&str>,
        messages_to_observe: &[Message],
    ) -> Result<ObserverResult> {
        let system_prompt = build_observer_system_prompt(
            self.config.observation.instruction.as_deref()
        );
        let task_prompt = build_observer_task_prompt(existing_observations);
        let history_message = format_messages_for_observer(messages_to_observe);

        let response = self.call_llm(
            &self.config.observation.model,
            &system_prompt,
            &[task_prompt, history_message],
            self.config.observation.temperature,
        ).await?;

        let parsed = parse_observer_output(&response);

        // 退化偵測與重試
        if parsed.degenerate {
            let response = self.call_llm(/* retry */).await?;
            let parsed = parse_observer_output(&response);
            if parsed.degenerate {
                return Err(anyhow!("Observer produced degenerate output after retry"));
            }
        }

        Ok(parsed)
    }

    /// 呼叫 Reflector Agent（含漸進壓縮）
    async fn call_reflector(
        &self,
        observations: &str,
        target_threshold: u32,
    ) -> Result<ReflectorResult> {
        let system_prompt = build_reflector_system_prompt(
            self.config.reflection.instruction.as_deref()
        );

        for level in 0..=3u8 {
            let prompt = build_reflector_prompt(observations, level);
            let response = self.call_llm(
                &self.config.reflection.model,
                &system_prompt,
                &[prompt],
                self.config.reflection.temperature,
            ).await?;

            let parsed = parse_reflector_output(&response);
            if parsed.degenerate { continue; }

            let reflected_tokens = self.token_counter.count_string(&parsed.observations);
            if validate_compression(reflected_tokens, target_threshold) {
                return Ok(parsed);
            }
        }

        Err(anyhow!("Reflector failed to compress after 4 attempts"))
    }
}
```

### Phase 5: 整合至 Agent Runtime（2 天）

```rust
// src/agent_runtime.rs 中的修改

impl AgentRuntime {
    pub async fn run_streaming(&self, /* ... */) -> Result<()> {
        let om = if self.config.observational_memory.enabled {
            Some(ObservationalMemory::new(
                self.config.observational_memory.clone(),
                self.memory_storage.clone(),
            ))
        } else {
            None
        };

        let mut step = 0;
        loop {
            // OM 輸入處理
            if let Some(ref om) = om {
                om.process_input_step(&mut messages, thread_id, resource_id, step).await?;
            }

            // LLM 呼叫
            let response = self.call_llm(&messages).await?;

            // 處理 tool calls
            // ...

            // OM 輸出處理
            if let Some(ref om) = om {
                om.process_output_result(&messages, thread_id, resource_id).await?;
            }

            step += 1;
        }
    }
}
```

### Phase 6: Storage 層（1 天）

```rust
// src/observational_memory/storage.rs

#[async_trait]
pub trait ObservationalMemoryStorage: Send + Sync {
    async fn get_record(
        &self,
        thread_id: Option<&str>,
        resource_id: &str,
    ) -> Result<Option<ObservationalMemoryRecord>>;

    async fn initialize_record(
        &self,
        input: CreateObservationalMemoryInput,
    ) -> Result<ObservationalMemoryRecord>;

    async fn update_active_observations(
        &self,
        record_id: &str,
        observations: &str,
        token_count: u32,
        observed_at: DateTime<Utc>,
        observed_message_ids: &[String],
    ) -> Result<ObservationalMemoryRecord>;

    async fn create_reflection_generation(
        &self,
        record_id: &str,
        observations: &str,
        token_count: u32,
    ) -> Result<ObservationalMemoryRecord>;

    async fn swap_buffered_to_active(
        &self,
        record_id: &str,
        chunks_to_activate: &[BufferedObservationChunk],
    ) -> Result<ObservationalMemoryRecord>;

    // ... 更多方法
}
```

### Phase 7: agents.toml 配置整合（0.5 天）

```toml
# ~/.clawtex/agents.toml

[observational_memory]
enabled = true
scope = "thread"                    # "thread" | "resource"
share_token_budget = false

[observational_memory.observation]
model = "google/gemini-2.5-flash"
message_tokens = 30000
temperature = 0.3
buffer_tokens_ratio = 0.2
buffer_activation = 0.8
# instruction = "Custom Observer instructions"

[observational_memory.reflection]
model = "google/gemini-2.5-flash"
observation_tokens = 40000
temperature = 0.0
buffer_activation = 0.5
# instruction = "Custom Reflector instructions"
```

### 預估時程

| Phase | 內容 | 預估天數 |
|-------|------|---------|
| 1 | 核心資料結構 | 2-3 |
| 2 | Observer/Reflector Prompts | 1 |
| 3 | Token 計數與門檻 | 1 |
| 4 | 核心 Processor 邏輯 | 3-4 |
| 5 | Agent Runtime 整合 | 2 |
| 6 | Storage 層 | 1 |
| 7 | 配置整合 | 0.5 |
| **總計** | | **10.5-12.5 天** |

### 移植優先順序建議

1. **MVP（5 天）**: Phase 1 + 2 + 3 + 簡化版 Phase 4（無 async buffering）
   - 同步觀察 + 反思即可提供 3-10x 壓縮
   - 可立即取代 `context_compactor.rs`

2. **Full Feature（追加 5-7 天）**: Phase 4（async buffering）+ Phase 5 + 6 + 7
   - 非同步緩衝提升使用者體驗（零阻塞觀察）
   - 完整的 storage 持久化

### 關鍵注意事項

1. **Rust 的 async 特性**：Mastra 使用 JavaScript 的 `void` fire-and-forget async 呼叫。在 Rust 中需要使用 `tokio::spawn` 搭配 `Arc` 共享狀態。

2. **靜態狀態管理**：Mastra 使用 `static` maps 跨 OM 實例共享狀態。在 Rust 中可使用 `lazy_static!` + `Arc<Mutex<HashMap>>` 或直接在 `AgentRuntime` 層級持有。

3. **Token 計數**：Mastra 使用 `tokenx` 和 `xxhash-wasm`。Rust 可使用 `tiktoken-rs` 或 `tokenizers` crate。

4. **Provider 路由**：Observer/Reflector 預設使用 `google/gemini-2.5-flash`（因為速度快且便宜）。在 clawtex 中可透過既有的 `ProviderRouter` 路由。

5. **degenerate detection**：需要移植 `detectDegenerateRepetition` 函式，這是防止 LLM 進入重複迴圈的關鍵保護。

---

## 附錄：檔案路徑索引

所有路徑相對於 `C:\Users\m4932\Desktop\adreanalai\LLM-Cluster-Project\references\mastra\`

| 檔案 | 用途 |
|------|------|
| `packages/memory/src/processors/observational-memory/observational-memory.ts` | OM 主類別（~3500 行） |
| `packages/memory/src/processors/observational-memory/observer-agent.ts` | Observer system prompt 與解析器 |
| `packages/memory/src/processors/observational-memory/reflector-agent.ts` | Reflector system prompt 與解析器 |
| `packages/memory/src/processors/observational-memory/types.ts` | 所有 OM 型別定義 |
| `packages/memory/src/processors/observational-memory/thresholds.ts` | 動態門檻計算 |
| `packages/memory/src/processors/observational-memory/token-counter.ts` | Token 計數器（含圖片估算） |
| `packages/memory/src/processors/observational-memory/markers.ts` | 觀察邊界標記工廠 |
| `packages/memory/src/processors/observational-memory/operation-registry.ts` | 進程級操作追蹤 |
| `packages/memory/src/processors/observational-memory/date-utils.ts` | 相對時間格式化與日期解析 |
| `packages/memory/src/processors/observational-memory/index.ts` | OM 套件入口 |
| `packages/memory/src/index.ts` | Memory 套件主入口（含 OM 整合） |
| `packages/core/src/memory/memory.ts` | MastraMemory 抽象基類 |
| `packages/core/src/agent/agent.ts` | Agent 類別定義 |
| `packages/core/src/tools/tool.ts` | Tool 系統（createTool） |
| `packages/core/src/mastra/index.ts` | Mastra 中央配置類 |
| `packages/core/src/workflows/workflow.ts` | Workflow 系統 |
| `packages/core/src/processors/` | Processor 系統基礎 |
