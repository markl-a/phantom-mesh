# Gemini CLI — Google 代碼代理 Agent-to-Agent 伺服器架構文檔

## 1. 專案概覽

**Gemini CLI** 是由 Google 開發的代碼生成 CLI 工具，核心組件包括：

- **a2a-server**：Agent-to-Agent (A2A) 通訊伺服器（TypeScript/Node.js 實作）
- **gemini-cli-core**：Gemini 特定邏輯（TypeScript 實作）
- **集成測試**：100+ 端到端測試
- **評估工具**：AI 代理性能評估框架

本文檔重點關注 **a2a-server** 組件，這是 Gemini CLI 的核心執行引擎和 A2A 通訊層。

### 核心特性
- Agent-to-Agent (A2A) SDK 通訊協議
- Express.js 型 HTTP API 伺服器
- 非同步任務執行與狀態管理
- 外部代理委託（SubAgent）支援
- 流式輸出支援
- 配置與環境管理
- MCP 工具集成
- 認證與授權（Bearer/Basic Auth）

### 版本資訊
- **語言**：TypeScript/Node.js
- **框架**：Express.js + A2A SDK
- **協議版本**：0.3.0
- **代理協議版本**：v0.0.2

---

## 2. 目錄結構

```
gemini-cli/
├── packages/
│   ├── a2a-server/                      # 主伺服器實作
│   │   ├── src/
│   │   │   ├── index.ts                 # 公共 API 入口
│   │   │   ├── agent/
│   │   │   │   ├── executor.ts          # CoderAgentExecutor 核心
│   │   │   │   ├── task.ts              # Task 狀態管理
│   │   │   │   ├── task-event-driven.ts # 事件驅動模式
│   │   │   │   └── [tests]              # 執行器測試
│   │   │   ├── commands/
│   │   │   │   ├── command-registry.ts  # 命令註冊與查詢
│   │   │   │   ├── extensions.ts        # 外部命令載入
│   │   │   │   ├── init.ts              # 初始化指令
│   │   │   │   ├── memory.ts            # 記憶體管理
│   │   │   │   ├── restore.ts           # 恢復操作
│   │   │   │   └── types.ts             # 命令類型定義
│   │   │   ├── config/
│   │   │   │   ├── config.ts            # 配置載入與管理
│   │   │   │   ├── settings.ts          # 使用者設定
│   │   │   │   ├── extension.ts         # 外部擴展配置
│   │   │   │   └── [tests]              # 配置測試
│   │   │   ├── http/
│   │   │   │   ├── app.ts               # Express 應用主體
│   │   │   │   ├── server.ts            # HTTP 伺服器啟動
│   │   │   │   ├── endpoints.ts         # 端點定義
│   │   │   │   ├── requestStorage.ts    # 請求管理
│   │   │   │   └── [tests]              # HTTP 測試
│   │   │   ├── persistence/
│   │   │   │   ├── gcs.ts               # Google Cloud Storage
│   │   │   │   └── [tests]              # 持久化測試
│   │   │   ├── utils/
│   │   │   │   ├── logger.ts            # 日誌工具
│   │   │   │   ├── executor_utils.ts    # 執行工具函式
│   │   │   │   └── [其他工具]
│   │   │   ├── types.ts                 # 全域型別定義
│   │   │   └── [其他支援檔案]
│   │   ├── tests/
│   │   │   ├── [單元測試]
│   │   │   └── [...test files]
│   │   ├── package.json                 # Node.js 依賴
│   │   └── tsconfig.json               # TypeScript 配置
│   │
│   ├── gemini-cli-core/                 # Gemini 特定邏輯
│   │   ├── src/
│   │   │   ├── config/
│   │   │   ├── tools/
│   │   │   ├── extensions/
│   │   │   └── [Gemini 相關實作]
│   │   └── package.json
│   │
│   └── [其他套件]
│
├── integration-tests/                   # 端到端測試 (50+)
│   ├── *.test.ts                        # 各項功能測試
│   ├── test-helper.ts                   # 測試工具
│   ├── globalSetup.ts                   # 全域設定
│   └── vitest.config.ts                 # Vitest 配置
│
├── evals/                               # AI 評估工具 (20+)
│   ├── *.eval.ts                        # 各項評估測試
│   ├── test-helper.ts                   # 評估工具函式
│   └── vitest.config.ts
│
├── .gemini/                             # Gemini 組態
│   ├── config.yaml                      # 主要設定
│   ├── settings.json                    # 使用者設定
│   ├── commands/                        # 自訂命令
│   └── skills/                          # AI 技能定義
│
└── package.json                         # 工作區根配置
```

---

## 3. 核心 Trait 與結構（TypeScript 介面）

### 3.1 主要結構

```typescript
// executor.ts - 代理執行核心
export class CoderAgentExecutor implements AgentExecutor {
  private tasks: Map<string, TaskWrapper> = new Map();
  private executingTasks = new Set<string>();

  constructor(private taskStore?: TaskStore) {}

  async execute(
    task: SDKTask,
    eventBus?: ExecutionEventBus,
  ): Promise<AgentExecutionEvent[]>

  async reconstruct(
    sdkTask: SDKTask,
    eventBus?: ExecutionEventBus,
  ): Promise<TaskWrapper>
}
```

### 3.2 核心介面（來自 A2A SDK）

```typescript
// @a2a-js/sdk 中定義
interface AgentExecutor {
  execute(
    task: Task,
    eventBus?: ExecutionEventBus,
  ): Promise<AgentExecutionEvent[]>;

  reconstruct(
    sdkTask: Task,
    eventBus?: ExecutionEventBus,
  ): Promise<any>;
}

interface TaskStore {
  save(task: Task): Promise<void>;
  load(taskId: string): Promise<Task>;
  delete(taskId: string): Promise<void>;
  list(): Promise<Task[]>;
}

interface ExecutionEventBus {
  emit(event: AgentExecutionEvent): void;
  subscribe(
    handler: (event: AgentExecutionEvent) => void
  ): Unsubscribe;
}
```

### 3.3 任務包裝與轉換

```typescript
// executor.ts
class TaskWrapper {
  task: Task;
  agentSettings: AgentSettings;

  constructor(task: Task, agentSettings: AgentSettings) {
    this.task = task;
    this.agentSettings = agentSettings;
  }

  toSDKTask(): SDKTask {
    const persistedState: PersistedStateMetadata = {
      _agentSettings: this.agentSettings,
      _taskState: this.task.taskState,
    };

    return {
      id: this.task.id,
      contextId: this.task.contextId,
      kind: 'task',
      status: {
        state: this.task.taskState,
        timestamp: new Date().toISOString(),
      },
      metadata: setPersistedState({}, persistedState),
      history: [],
      artifacts: [],
    };
  }
}
```

### 3.4 應用組態

```typescript
// http/app.ts
const coderAgentCard: AgentCard = {
  name: 'Gemini SDLC Agent',
  description: 'An agent that generates code...',
  url: 'http://localhost:41242/',
  provider: {
    organization: 'Google',
    url: 'https://google.com',
  },
  protocolVersion: '0.3.0',
  version: '0.0.2',
  capabilities: {
    streaming: true,
    pushNotifications: false,
    stateTransitionHistory: true,
  },
  securitySchemes: {
    bearerAuth: { type: 'http', scheme: 'bearer' },
    basicAuth: { type: 'http', scheme: 'basic' },
  },
  security: [{ bearerAuth: [] }, { basicAuth: [] }],
};
```

### 3.5 配置與環境

```typescript
// config/config.ts
export interface Config {
  // Gemini 模型設定
  geminiApiKey?: string;
  modelName: string;  // e.g., "gemini-2.0-flash"

  // 工具配置
  tools: Map<string, ToolConfig>;
  extensions: Extension[];

  // 執行環境
  workspaceRoot: string;
  targetDir: string;

  // MCP 伺服器
  mcpServers: MCPServerConfig[];
}

export interface AgentSettings {
  contextId: string;
  targetDir?: string;
  metadata?: Record<string, any>;
}
```

---

## 4. 啟動流程

### 4.1 應用初始化序列

```
┌────────────────────────────────────────┐
│ index.ts / main entry point             │
└─────────────┬──────────────────────────┘
              │
              ▼
┌────────────────────────────────────────┐
│ 1. 環境設定 (loadEnvironment)           │
│    - 讀取 .env 檔案                     │
│    - 設定工作區根目錄                   │
│    - 初始化 Gemini API 金鑰             │
└─────────────┬──────────────────────────┘
              │
              ▼
┌────────────────────────────────────────┐
│ 2. 配置載入 (loadConfig)                │
│    - 讀取 .gemini/config.yaml           │
│    - 讀取 .gemini/settings.json         │
│    - 合併使用者設定                     │
│    - 驗證配置完整性                     │
└─────────────┬──────────────────────────┘
              │
              ▼
┌────────────────────────────────────────┐
│ 3. 擴展載入 (loadExtensions)            │
│    - 掃描 .gemini/skills/               │
│    - 載入自訂命令/工具                  │
│    - 註冊至 commandRegistry             │
└─────────────┬──────────────────────────┘
              │
              ▼
┌────────────────────────────────────────┐
│ 4. Gemini 核心初始化                    │
│    - 建立 Config 物件                   │
│    - 初始化 SimpleExtensionLoader       │
│    - 設定 GitService (若需要)           │
└─────────────┬──────────────────────────┘
              │
              ▼
┌────────────────────────────────────────┐
│ 5. Express 應用設定 (createApp)         │
│    - 設定中介軟體 (logging, auth)      │
│    - 註冊路由                           │
│    - 建立 AgentCard                     │
└─────────────┬──────────────────────────┘
              │
              ▼
┌────────────────────────────────────────┐
│ 6. Agent 執行器初始化                   │
│    - 建立 CoderAgentExecutor            │
│    - 選擇 TaskStore (GCS/InMemory)      │
│    - 設定 ExecutionEventBus             │
└─────────────┬──────────────────────────┘
              │
              ▼
┌────────────────────────────────────────┐
│ 7. HTTP 伺服器啟動                      │
│    - 監聽指定埠（預設 41242）           │
│    - 準備接受 HTTP 請求                │
└────────────────────────────────────────┘
```

### 4.2 請求處理流程

```
┌──────────────────────────────────────┐
│ HTTP 請求到達                         │
│ POST /agent/tasks                     │
└────────────┬─────────────────────────┘
             │
             ▼
┌──────────────────────────────────────┐
│ 1. 認證驗證                           │
│    - 檢查 Authorization header        │
│    - Bearer Token / Basic Auth        │
│    - 回傳 User 物件                   │
└────────────┬─────────────────────────┘
             │
             ▼
┌──────────────────────────────────────┐
│ 2. 請求路由                           │
│    - 分類請求類型                     │
│    - A2A Express Router 分發           │
└────────────┬─────────────────────────┘
             │
        ┌────┴─────────────────┬──────────┐
        │                      │          │
        ▼                      ▼          ▼
   ┌────────────┐      ┌──────────────┐ ┌──────────┐
   │Task Start  │      │Task Resume   │ │Status    │
   │(新任務)     │      │(恢復任務)     │ │Query     │
   └────┬───────┘      └──────┬───────┘ └────┬─────┘
        │                     │              │
        ▼                     ▼              ▼
   ┌────────────────────────────────────────┐
   │ 3. 代理執行器 (CoderAgentExecutor)      │
   │    - 載入/重建 TaskWrapper              │
   │    - 執行代理邏輯                       │
   │    - 發送事件至 EventBus               │
   └────────────┬─────────────────────────┘
                │
                ▼
        ┌──────────────────────┐
        │ 4. 結果構造           │
        │ AgentExecutionEvent   │
        │ + 流式輸出（若存在）   │
        └────────┬──────────────┘
                 │
                 ▼
        ┌──────────────────────┐
        │ 5. HTTP 回應         │
        │ 200 OK / 500 Error   │
        └──────────────────────┘
```

---

## 5. 資料流 ASCII 圖

### 5.1 完整代理執行週期

```
              User / Parent Agent
                      │
                      ▼
        ┌─────────────────────────────┐
        │ HTTP Server (Express.js)     │
        │ Port 41242                  │
        └──────────┬──────────────────┘
                   │
        ┌──────────▼──────────┐
        │ A2A Express Router  │
        │ (from @a2a-js SDK) │
        └──────────┬──────────┘
                   │
        ┌──────────▼──────────────────────┐
        │ CoderAgentExecutor              │
        │ - Task Lifecycle Management     │
        │ - Event Emission                │
        └──────────┬──────────────────────┘
                   │
    ┌──────────────┼──────────────────┐
    │              │                  │
    ▼              ▼                  ▼
┌────────┐   ┌─────────┐        ┌──────────┐
│Config  │   │Tools    │        │MCP       │
│Load    │   │Registry │        │Servers   │
└───┬────┘   └────┬────┘        └────┬─────┘
    │             │                  │
    │             │                  │
    └─────────────┼──────────────────┘
                  │
                  ▼
        ┌──────────────────────┐
        │ Gemini Model API     │
        │ (claude / gemini-2.0)│
        └──────────────────────┘
```

### 5.2 任務狀態轉換

```
                    ┌─────────────┐
                    │   CREATED   │
                    └──────┬──────┘
                           │
                           ▼
                    ┌─────────────┐
    ┌──────────────▶│  RUNNING    │◀──────────────┐
    │               └──────┬──────┘               │
    │                      │                      │
    │              ┌───────┴────────┐             │
    │              │                │             │
    │              ▼                ▼             │
    │         ┌────────┐       ┌─────────┐       │
    │         │SUSPEND │       │  ABORT  │       │
    │         └────┬───┘       └────┬────┘       │
    │              │                │             │
    │              └────────┬───────┘             │
    │                       │                     │
    │                       ▼                     │
    │              ┌──────────────┐               │
    └──────────────│   RESUMED    │───────────────┘
                   └──────┬───────┘
                          │
                   ┌──────┴─────────┐
                   │                │
                   ▼                ▼
              ┌────────┐       ┌──────────┐
              │ SUCCESS│       │FAILED    │
              └────────┘       └──────────┘
```

### 5.3 配置載入堆疊

```
  ┌─────────────────────────────────┐
  │ 環境變數 (process.env)           │  ◄─── 最高優先序
  └──────────────┬──────────────────┘
                 │
  ┌──────────────▼──────────────────┐
  │ .gemini/config.yaml              │
  │ (專案級配置)                      │
  └──────────────┬──────────────────┘
                 │
  ┌──────────────▼──────────────────┐
  │ .gemini/settings.json            │
  │ (使用者偏好)                      │
  └──────────────┬──────────────────┘
                 │
  ┌──────────────▼──────────────────┐
  │ .gemini/skills/*/SKILL.md        │
  │ (AI 技能定義)                     │
  └──────────────┬──────────────────┘
                 │
  ┌──────────────▼──────────────────┐
  │ 內置預設值                        │  ◄─── 最低優先序
  └─────────────────────────────────┘
```

---

## 6. 子系統清單

### P0 優先級（核心必需）

| 子系統 | 檔案位置 | 責任 | 狀態 |
|-------|--------|------|------|
| **代理執行器** | `agent/executor.ts` | 任務執行、狀態管理 | 穩定 |
| **Express 應用** | `http/app.ts` | HTTP 路由、認證、AgentCard | 穩定 |
| **A2A 整合** | `http/server.ts` | A2A SDK 伺服器啟動 | 穩定 |
| **配置系統** | `config/config.ts` | 配置載入與驗證 | 穩定 |
| **命令註冊** | `commands/command-registry.ts` | 命令定義與查詢 | 穩定 |
| **持久化** | `persistence/gcs.ts` | 任務儲存（GCS/Memory） | 穩定 |

### P1 優先級（高級功能）

| 子系統 | 檔案位置 | 責任 | 狀態 |
|-------|--------|------|------|
| **任務管理** | `agent/task.ts` | 任務狀態、生命週期 | 穩定 |
| **事件驅動** | `agent/task-event-driven.ts` | 非同步事件流 | 穩定 |
| **擴展載入** | `config/extension.ts` | 自訂命令載入 | 穩定 |
| **MCP 集成** | gemini-cli-core | MCP 工具呼叫 | 穩定 |
| **Gemini 核心** | gemini-cli-core | Gemini 特定實作 | 穩定 |

### P2 優先級（支援/測試）

| 子系統 | 檔案位置 | 責任 | 狀態 |
|-------|--------|------|------|
| **日誌記錄** | `utils/logger.ts` | 結構化日誌 | 穩定 |
| **請求管理** | `http/requestStorage.ts` | 保存進行中的請求 | 穩定 |
| **工具函式** | `utils/executor_utils.ts` | 輔助函式 | 穩定 |
| **認證** | `http/app.ts` | Bearer/Basic Auth | 穩定 |

---

## 7. 關鍵設計模式

### 7.1 代理執行生命週期

```typescript
// 1. 建立執行器
const executor = new CoderAgentExecutor(taskStore);

// 2. 執行任務
const events = await executor.execute(task, eventBus);

// 3. 監聽事件
eventBus.subscribe((event) => {
  console.log('Agent event:', event);
});

// 4. 恢復任務
const reconstructed = await executor.reconstruct(sdkTask);
```

### 7.2 配置隔離與注入

```typescript
// 配置作為依賴注入
const config = await getConfig(agentSettings, taskId);

// 不同代理可有不同配置
const agent1Config = await getConfig(settings1, 'task-1');
const agent2Config = await getConfig(settings2, 'task-2');
```

### 7.3 非同步流處理

```typescript
// 使用 EventBus 處理異步結果
let result: AgentExecutionEvent[] = [];

eventBus.subscribe((event) => {
  result.push(event);
});

await executor.execute(task, eventBus);
```

### 7.4 TaskWrapper 轉換模式

```typescript
// 內部 Task ↔ SDK Task 轉換
class TaskWrapper {
  toSDKTask(): SDKTask {
    // 將內部狀態轉換為 SDK 格式
    // 支援多種執行模式
  }
}
```

---

## 8. HTTP API 端點

### 代理管理

| 端點 | 方法 | 參數 | 返回 | 說明 |
|------|------|------|------|------|
| `/agent/info` | GET | 無 | AgentCard | 代理資訊卡 |
| `/agent/tasks` | POST | TaskStartPayload | TaskResponse | 建立任務 |
| `/agent/tasks/:id` | GET | taskId | TaskResponse | 查詢任務 |
| `/agent/tasks/:id/resume` | POST | TaskResumePayload | TaskResponse | 恢復任務 |

### 流式輸出

| 端點 | 方法 | 參數 | 返回 | 說明 |
|------|------|------|------|------|
| `/agent/tasks/:id/stream` | GET | taskId | Server-Sent Events | 串流任務輸出 |

### 配置與工具

| 端點 | 方法 | 參數 | 返回 | 說明 |
|------|------|------|------|------|
| `/tools/list` | GET | 無 | Tool[] | 可用工具列表 |
| `/config/get` | GET | key | value | 取得配置 |
| `/config/set` | POST | key, value | ok | 設定配置 |

---

## 9. 常見使用模式

### 9.1 基本任務執行

```typescript
// 1. 啟動 Express 伺服器
const app = createCoderAgentApp();
app.listen(41242);

// 2. 客戶端傳送請求
const response = await fetch('http://localhost:41242/agent/tasks', {
  method: 'POST',
  headers: {
    'Content-Type': 'application/json',
    'Authorization': 'Bearer token'
  },
  body: JSON.stringify({
    contextId: 'user-123',
    prompt: 'Write a Python function for Fibonacci',
    metadata: {}
  })
});

// 3. 伺服器執行代理
const result = await response.json();
// {
//   taskId: 'task-abc',
//   state: 'running',
//   output: '...'
// }
```

### 9.2 恢復暫停的任務

```typescript
// 恢復中斷的任務
const resumeResponse = await fetch(
  'http://localhost:41242/agent/tasks/task-abc/resume',
  {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Authorization': 'Bearer token'
    },
    body: JSON.stringify({
      userInput: 'Continue with optimization'
    })
  }
);
```

### 9.3 使用認證

```typescript
// Bearer Token
const headers = {
  'Authorization': 'Bearer valid-token'
};

// 或 Basic Auth
const credentials = Buffer.from('admin:password').toString('base64');
const headers2 = {
  'Authorization': `Basic ${credentials}`
};
```

### 9.4 串流輸出

```typescript
// 使用 Server-Sent Events
const eventSource = new EventSource(
  'http://localhost:41242/agent/tasks/task-abc/stream',
  {
    headers: { 'Authorization': 'Bearer token' }
  }
);

eventSource.addEventListener('message', (event) => {
  console.log('Output:', event.data);
});
```

---

## 10. 執行時考量

### 10.1 成本分析

| 操作 | 成本等級 | 說明 |
|------|--------|------|
| HTTP 路由 | 低 | Express 中介軟體 |
| 配置載入 | 中 | 檔案 I/O + YAML/JSON 解析 |
| 代理執行 | 高 | 調用 Gemini API |
| 任務序列化 | 中 | TaskWrapper 轉換 |
| 事件發送 | 低 | 記憶體操作 |

### 10.2 效能優化

1. **配置快取**：避免重複解析
2. **任務存儲選擇**：
   - 開發：InMemoryTaskStore
   - 生產：GCSTaskStore（持久化）
3. **串流輸出**：避免緩衝整個結果
4. **工具快取**：快取工具元資訊

### 10.3 記憶體管理

```typescript
// 任務自動清理
private tasks: Map<string, TaskWrapper> = new Map();

// 定期清理完成的任務
setInterval(() => {
  for (const [id, task] of this.tasks) {
    if (task.isCompleted()) {
      this.tasks.delete(id);
    }
  }
}, 5 * 60 * 1000);  // 5 分鐘
```

---

## 11. 集成點（與 clawtex-core 相關）

### 11.1 作為代碼代理後端

```
clawtex-core
    │
    ├─→ Code Generation Request
    │        │
    │        ▼
    │   gemini-cli a2a-server
    │   (HTTP 請求)
    │        │
    │        ├─→ Configure Environment
    │        ├─→ Load Extensions
    │        └─→ Call Gemini API
    │
    └─→ Integrate Generated Code
```

### 11.2 與 MCP 的關係

```
clawtex-core MCP
    │
    └─→ Delegate to gemini-cli
        (via HTTP A2A)

        gemini-cli Agent
        ├─→ Gemini Model (為主)
        ├─→ MCP Tools (補充)
        └─→ Custom Skills
```

### 11.3 代理委託（SubAgent）

```
Primary Agent (clawtex)
    │
    └─→ Delegate to Secondary Agent
        (gemini-cli)
            │
            └─→ Delegate to Tertiary Agent
                (if needed)
```

---

## 12. 依賴關係圖

```
a2a-server
├── @a2a-js/sdk           # Agent-to-Agent SDK
│   ├── @a2a-js/sdk/server
│   └── @a2a-js/sdk/express
├── express                # Web 框架
├── @google/gemini-cli-core # Gemini 特定邏輯
├── uuid                   # ID 生成
├── typescript             # 型別系統
└── [其他依賴]
    ├── dotenv
    ├── winston (日誌)
    ├── jest/vitest (測試)
    └── ...
```

---

## 13. 測試策略

### 13.1 單元測試

- 位置：各模組 `.test.ts` 檔案
- 範圍：
  - 配置載入
  - 命令註冊
  - TaskWrapper 轉換
  - 認證邏輯

### 13.2 集成測試

- 位置：`integration-tests/` 目錄
- 範圍：**50+ 測試**涵蓋：
  - HTTP 端點
  - 代理執行流程
  - 流式輸出
  - 認證與授權
  - 配置驗證
  - 工具呼叫

### 13.3 評估測試

- 位置：`evals/` 目錄
- 範圍：**20+ 評估**涵蓋：
  - 代碼生成品質
  - 代理自主性
  - 錯誤恢復

---

## 14. 錯誤處理

### 14.1 HTTP 錯誤碼

| 碼 | 說明 | 範例 |
|----|------|------|
| 200 | 成功 | 任務建立成功 |
| 400 | 不良請求 | 無效參數 |
| 401 | 未授權 | 缺少認證 |
| 403 | 禁止 | 權限不足 |
| 404 | 未找到 | 任務不存在 |
| 500 | 伺服器錯誤 | 代理執行失敗 |

### 14.2 錯誤回應

```json
{
  "error": {
    "code": "EXECUTION_FAILED",
    "message": "Agent execution failed",
    "details": {
      "taskId": "task-abc",
      "reason": "Gemini API timeout"
    }
  }
}
```

---

## 15. 監控與可觀測性

### 15.1 日誌級別

```typescript
// 使用 logger.ts
logger.info('Task started', { taskId, contextId });
logger.debug('Executing tool', { tool, args });
logger.error('Execution failed', { taskId, error });
```

### 15.2 追蹤指標

- 任務執行時間
- Gemini API 延遲
- 工具呼叫計數
- 代理成功率

### 15.3 結構化日誌

```json
{
  "timestamp": "2025-03-13T10:30:00Z",
  "level": "info",
  "taskId": "task-abc",
  "event": "task_started",
  "duration_ms": 1234
}
```

---

## 16. 安全考量

### 16.1 認證機制

```typescript
// Bearer Token
if (auth.startsWith('Bearer ')) {
  const token = auth.substring(7);
  if (token === 'valid-token') {
    return { userName: 'bearer-user', isAuthenticated: true };
  }
}

// Basic Auth
if (auth.startsWith('Basic ')) {
  const credentials = Buffer.from(
    auth.substring(6),
    'base64'
  ).toString();
  if (credentials === 'admin:password') {
    return { userName: 'basic-user', isAuthenticated: true };
  }
}
```

### 16.2 授權檢查

- 每個請求驗證使用者身份
- 執行環境隔離（每個代理獨立）
- 工具呼叫白名單

### 16.3 秘密管理

- API 金鑰儲存在環境變數
- 不在日誌中列印敏感資訊
- 使用 .env.example 提示設定

---

## 17. 故障排除

### 17.1 常見問題

| 問題 | 原因 | 解決方案 |
|------|------|---------|
| 代理無回應 | Gemini API 逾時 | 增加超時時間、檢查 API 額度 |
| 配置不生效 | 載入順序錯誤 | 檢查 .gemini/ 檔案位置 |
| 認證失敗 | Token 無效 | 驗證 Authorization 標頭 |
| 工具找不到 | 擴展未載入 | 確保 .gemini/skills/ 存在 |

### 17.2 調試技巧

```bash
# 啟用詳細日誌
DEBUG=gemini:* npm start

# 列印配置
npm run config:print

# 測試端點
curl -H "Authorization: Bearer token" \
  http://localhost:41242/agent/info
```

---

## 18. 部署考量

### 18.1 環境變數

```bash
# 必需
GEMINI_API_KEY=your-api-key

# 選擇性
NODE_ENV=production
PORT=41242
LOG_LEVEL=info
TASK_STORE=gcs  # or memory
```

### 18.2 生產配置

```typescript
// package.json
{
  "scripts": {
    "start": "node dist/server.js",
    "build": "tsc",
    "dev": "ts-node src/server.ts"
  },
  "engines": {
    "node": ">=18.0.0"
  }
}
```

### 18.3 容器化

```dockerfile
FROM node:18-alpine
WORKDIR /app
COPY package*.json ./
RUN npm ci --only=production
COPY dist ./dist
EXPOSE 41242
CMD ["npm", "start"]
```

---

## 19. 小結

Gemini CLI a2a-server 是高度靈活的代碼生成代理框架，特徵包括：

1. **Agent-to-Agent 協議**：標準化代理通訊
2. **HTTP 優先**：Express.js 易於集成
3. **非同步管理**：完整的任務生命週期
4. **配置靈活**：分層配置系統
5. **認證安全**：Bearer + Basic Auth 支援
6. **可擴展**：自訂命令與技能
7. **生產就緒**：100+ 測試涵蓋

與 clawtex-core 集成時，可作為高級代碼生成專門代理，支持複雜的多代理協調場景。

