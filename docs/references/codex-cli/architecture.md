# Codex CLI — Claude 程式碼編輯器 App Server 架構文檔

## 1. 專案概覽

**Codex CLI** 是由 Anthropic 開發的 Claude 程式碼編輯器，核心組件包括：

- **app-server**：JSON-RPC 2.0 協議的中央伺服器（Rust 實作）
- **app-server-protocol**：通訊協議定義與序列化
- **app-server-client**：用戶端通訊庫
- **ansi-escape**：ANSI 序列解析（終端輸出格式化）

本文檔重點關注 **app-server** 組件，這是 Codex 的核心執行引擎。

### 核心特性
- JSON-RPC 2.0 標準通訊協議
- WebSocket 與 stdio 雙重傳輸支援
- 非同步多執行緒架構（Tokio）
- 配置分層系統（Config Layer Stack）
- 訊息多路分發與執行狀態管理
- MCP 伺服器集成
- 即時追蹤與日誌記錄

### 版本資訊
- **架構**：多進程/多執行緒（Tokio 非同步）
- **通訊協議**：JSON-RPC 2.0
- **內部類型系統**：強類型 Rust

---

## 2. 目錄結構

```
codex-cli/
├── codex-rs/
│   ├── app-server/                      # 主伺服器實作
│   │   ├── src/
│   │   │   ├── main.rs                  # CLI 入口、命令列參數
│   │   │   ├── lib.rs                   # 主公共 API
│   │   │   ├── message_processor.rs     # JSON-RPC 訊息處理核心
│   │   │   ├── transport.rs             # WebSocket/stdio 傳輸層
│   │   │   ├── outgoing_message.rs      # 出站訊息路由
│   │   │   ├── thread_state.rs          # 執行緒狀態管理
│   │   │   ├── thread_status.rs         # 執行緒狀態監控
│   │   │   ├── config_api.rs            # 配置 API（RPC）
│   │   │   ├── command_exec.rs          # 指令執行引擎
│   │   │   ├── dynamic_tools.rs         # 動態工具載入
│   │   │   ├── error_code.rs            # JSON-RPC 錯誤碼定義
│   │   │   ├── filters.rs               # 配置過濾與檢驗
│   │   │   ├── fuzzy_file_search.rs     # 模糊檔案搜尋
│   │   │   ├── models.rs                # 資料結構定義
│   │   │   ├── in_process.rs            # 同進程執行模式
│   │   │   ├── server_request_error.rs  # 伺服器錯誤類型
│   │   │   ├── app_server_tracing.rs    # 追蹤與日誌設定
│   │   │   ├── bespoke_event_handling.rs # 特殊事件處理
│   │   │   ├── codex_message_processor.rs # Codex 特定邏輯
│   │   │   ├── external_agent_config_api.rs # 外部 Agent API
│   │   │   └── serde_multipart.rs       # Multipart 序列化
│   │   ├── tests/
│   │   │   ├── common/                  # 測試工具
│   │   │   ├── suite/v2/                # 集成測試（100+ 個）
│   │   │   └── all.rs                   # 測試入口
│   │   └── Cargo.toml
│   │
│   ├── app-server-protocol/             # 通訊協議定義
│   │   ├── src/
│   │   │   ├── lib.rs                   # 協議主模組
│   │   │   ├── protocol/
│   │   │   │   ├── v1.rs                # 版本 1 協議
│   │   │   │   ├── v2.rs                # 版本 2 協議（當前）
│   │   │   │   ├── common.rs            # 共有類型
│   │   │   │   ├── mod.rs               # 協議模組管理
│   │   │   │   ├── mappers.rs           # 類型映射
│   │   │   │   ├── thread_history.rs    # 執行緒歷史記錄
│   │   │   │   └── serde_helpers.rs     # 序列化輔助
│   │   │   ├── jsonrpc_lite.rs          # JSON-RPC 核心
│   │   │   ├── export.rs                # 類型匯出
│   │   │   └── schema_fixtures.rs       # 測試 fixture
│   │   └── Cargo.toml
│   │
│   ├── app-server-client/               # 用戶端庫
│   │   ├── src/lib.rs
│   │   └── Cargo.toml
│   │
│   ├── ansi-escape/                     # ANSI 序列處理
│   │   ├── src/lib.rs
│   │   └── Cargo.toml
│   │
│   └── Cargo.toml                       # 工作區管理
│
├── integration-tests/                   # 端到端測試
├── evals/                               # 評估腳本
└── packages/                            # TypeScript 套件
```

---

## 3. 核心 Trait 與結構

### 3.1 主要結構

```rust
// lib.rs - 整個應用程式的核心結構
pub async fn run_main_with_transport(
    arg0_paths: Arg0DispatchPaths,
    cli_overrides: CliConfigOverrides,
    loader_overrides: LoaderOverrides,
    is_in_process: bool,
    transport: AppServerTransport,
) -> Result<()>
```

### 3.2 訊息處理架構

```rust
// message_processor.rs - 中央訊息分發器
pub struct MessageProcessor {
    task_store: Arc<TaskStore>,
    config: Arc<RwLock<Config>>,
    thread_states: Arc<Mutex<HashMap<ThreadId, ThreadState>>>,
    message_sender: OutgoingMessageSender,
}

impl MessageProcessor {
    pub async fn process_message(
        &self,
        json_rpc: JSONRPCMessage,
    ) -> Result<Option<OutgoingEnvelope>>
}
```

### 3.3 傳輸層

```rust
// transport.rs - 多路傳輸支援
pub enum AppServerTransport {
    Stdio(StdioTransport),      // stdio:// 協議
    WebSocket(WsTransport),     // ws://HOST:PORT 協議
}

pub async fn start_stdio_connection(
    processor: Arc<MessageProcessor>,
) -> Result<()>

pub async fn start_websocket_acceptor(
    processor: Arc<MessageProcessor>,
    addr: SocketAddr,
) -> Result<()>
```

### 3.4 執行緒狀態管理

```rust
// thread_state.rs
pub struct ThreadState {
    pub id: ThreadId,
    pub conversation_history: Vec<Message>,
    pub current_task: Option<Task>,
    pub metadata: Metadata,
    pub status: ThreadStatus,
}

// thread_status.rs
pub enum ThreadStatus {
    Ready,
    Running,
    WaitingForInput,
    Error(String),
}
```

### 3.5 配置系統

```rust
// config.rs - 分層配置
pub struct Config {
    pub layers: Vec<ConfigLayer>,
    pub merged: ConfigMap,
    pub warnings: Vec<ConfigWarning>,
}

// 配置層堆疊（優先序）
// 1. CLI 覆蓋
// 2. 環境變數
// 3. 使用者配置 (~/.config)
// 4. 工作區配置 (.codex/config)
// 5. 預設值
```

### 3.6 JSON-RPC 通訊

```rust
// jsonrpc_lite.rs
pub struct JSONRPCMessage {
    pub jsonrpc: String,        // "2.0"
    pub id: Option<Id>,         // 序列號
    pub method: String,         // 方法名稱
    pub params: Value,          // 參數
}

pub struct JSONRPCResponse {
    pub jsonrpc: String,
    pub id: Option<Id>,
    pub result: Option<Value>,
    pub error: Option<JSONRPCError>,
}
```

---

## 4. 啟動流程

### 4.1 服務初始化序列

```
┌──────────────────────────────────────┐
│ main.rs - 程式進入點                  │
└────────────┬─────────────────────────┘
             │
             ▼
┌──────────────────────────────────────┐
│ 1. 命令列解析 (AppServerArgs)         │
│    --listen stdio:// (預設)           │
│    或 ws://127.0.0.1:8000            │
└────────────┬─────────────────────────┘
             │
             ▼
┌──────────────────────────────────────┐
│ 2. 配置系統初始化                     │
│    - 載入 ConfigLayerStack           │
│    - 合併多層配置                     │
│    - 驗證配置                         │
│    - 發出警告                         │
└────────────┬─────────────────────────┘
             │
             ▼
┌──────────────────────────────────────┐
│ 3. 認證系統初始化                     │
│    - AuthManager 建立                 │
│    - 簽名密鑰載入                     │
└────────────┬─────────────────────────┘
             │
             ▼
┌──────────────────────────────────────┐
│ 4. 日誌系統初始化                     │
│    - tracing 訂閱器設定               │
│    - JSON/預設格式選擇                │
│    - 日誌級別設定                     │
└────────────┬─────────────────────────┘
             │
             ▼
┌──────────────────────────────────────┐
│ 5. 訊息處理器建立                     │
│    - ThreadState 管理                 │
│    - 命令註冊 (command_registry)      │
│    - 動態工具載入                     │
└────────────┬─────────────────────────┘
             │
             ▼
┌──────────────────────────────────────┐
│ 6. 傳輸層啟動                         │
│    選項 A: stdio 連線                 │
│    選項 B: WebSocket 監聽             │
└────────────┬─────────────────────────┘
             │
             ▼
┌──────────────────────────────────────┐
│ 7. 主事件迴圈                         │
│    - 接受傳入訊息                     │
│    - 分發至訊息處理器                 │
│    - 路由出站訊息                     │
│    - 監控和日誌                       │
└──────────────────────────────────────┘
```

### 4.2 請求處理流程

```
┌──────────────────────────────────────┐
│ 傳入 JSON-RPC 訊息                    │
│ (客戶端 via stdio/ws)                 │
└────────────┬─────────────────────────┘
             │
             ▼
┌──────────────────────────────────────┐
│ Transport Layer 解析                  │
│ - stdio: 逐行讀取 JSON                │
│ - ws: WebSocket 訊框                  │
└────────────┬─────────────────────────┘
             │
             ▼
┌──────────────────────────────────────┐
│ JSON-RPC 解驗                         │
│ - 驗證 jsonrpc="2.0"                 │
│ - 檢查必需欄位                        │
│ - ID 序列化                           │
└────────────┬─────────────────────────┘
             │
             ▼
┌──────────────────────────────────────┐
│ MessageProcessor::process_message    │
│ - 查詢執行緒狀態                      │
│ - 呼叫對應的 RPC 處理器               │
└────────────┬─────────────────────────┘
             │
        ┌────┴─────────────────┬───────┐
        │                      │       │
        ▼                      ▼       ▼
   ┌────────┐          ┌──────────┐  ┌──────────┐
   │Config  │          │Execution │  │Other     │
   │API     │          │API       │  │Methods   │
   └────┬───┘          └─────┬────┘  └─────┬────┘
        │                    │             │
        ▼                    ▼             ▼
   ┌───────────────────────────────────────┐
   │ 執行處理器邏輯                         │
   │ - 配置更新 / 執行指令 / 線程操作      │
   └────────────┬────────────────────────┘
                │
                ▼
        ┌──────────────────────┐
        │ 結果構造              │
        │ JSONRPCResponse      │
        └────────┬─────────────┘
                 │
                 ▼
        ┌──────────────────────┐
        │ OutgoingEnvelope     │
        │ (ConnectionId + msg) │
        └────────┬─────────────┘
                 │
                 ▼
        ┌──────────────────────┐
        │ 回傳至客戶端          │
        │ (via transport)       │
        └──────────────────────┘
```

---

## 5. 資料流 ASCII 圖

### 5.1 完整訊息週期（雙向）

```
                        Client
                          │
                          ▼
        ┌─────────────────────────────────┐
        │   stdio/WebSocket Transport     │
        └────────────┬────────────────────┘
                     │
    ┌────────────────┴────────────────┐
    │                                 │
    ▼                                 ▼
 stdin/                          WebSocket
 stdout                          Frames
    │                                 │
    └────────────────┬────────────────┘
                     │
                     ▼
        ┌─────────────────────────────────┐
        │  JSON-RPC Message Parser        │
        │  (jsonrpc_lite)                 │
        └────────────┬────────────────────┘
                     │
                     ▼
        ┌─────────────────────────────────┐
        │  MessageProcessor               │
        │  - Route by method              │
        │  - Lookup ThreadState           │
        └────────────┬────────────────────┘
                     │
        ┌────────────┴────────────────────┐
        │                                 │
        ▼                                 ▼
    ┌────────────┐              ┌─────────────────┐
    │ Config API │              │ Execution API   │
    │ (RPC)      │              │ (RPC)           │
    └─────┬──────┘              └────────┬────────┘
          │                              │
          ▼                              ▼
    ┌──────────────┐        ┌────────────────────┐
    │ config.rs    │        │ command_exec.rs    │
    │ - 層堆疊     │        │ - 執行指令         │
    │ - 合併配置   │        │ - 狀態轉換         │
    │ - 驗證       │        │ - MCP 呼叫         │
    └──────┬───────┘        └───────┬────────────┘
           │                        │
           └────────────┬───────────┘
                        │
                        ▼
             ┌──────────────────────┐
             │ JSONRPCResponse      │
             │ + ThreadState 更新    │
             └──────────┬───────────┘
                        │
                        ▼
             ┌──────────────────────┐
             │ OutgoingEnvelope     │
             │ (ConnectionId, msg)  │
             └──────────┬───────────┘
                        │
        ┌───────────────┴──────────────┐
        │                              │
        ▼                              ▼
  ┌──────────────┐            ┌──────────────┐
  │ stdio stdout │            │ WebSocket    │
  │              │            │ Frame        │
  └──────┬───────┘            └───────┬──────┘
         │                            │
         └──────────────┬─────────────┘
                        │
                        ▼
                    Client
```

### 5.2 配置層堆疊（優先序）

```
  ┌─────────────────────────────────┐
  │ CLI 覆蓋 (CliConfigOverrides)    │  ◄─── 最高優先序
  └──────────────┬──────────────────┘
                 │
  ┌──────────────▼──────────────────┐
  │ 環境變數 (env::var)              │
  └──────────────┬──────────────────┘
                 │
  ┌──────────────▼──────────────────┐
  │ 使用者配置                       │
  │ (~/.config/codex)               │
  └──────────────┬──────────────────┘
                 │
  ┌──────────────▼──────────────────┐
  │ 工作區配置                       │
  │ (project/.codex/config.toml)     │
  │ + 託管配置 (managed)             │
  └──────────────┬──────────────────┘
                 │
  ┌──────────────▼──────────────────┐
  │ 預設值 (defaults.toml)           │  ◄─── 最低優先序
  └─────────────────────────────────┘
```

---

## 6. 子系統清單

### P0 優先級（核心必需）

| 子系統 | 檔案位置 | 責任 | 狀態 |
|-------|--------|------|------|
| **訊息處理器** | `message_processor.rs` | JSON-RPC 請求分發、RPC 路由 | 穩定 |
| **傳輸層** | `transport.rs` | stdio/WebSocket 通訊 | 穩定 |
| **出站路由** | `outgoing_message.rs` | 多連線訊息路由、广播 | 穩定 |
| **執行緒狀態** | `thread_state.rs` | 對話狀態管理、歷史記錄 | 穩定 |
| **JSON-RPC** | `jsonrpc_lite.rs` | JSON-RPC 2.0 編解碼 | 穩定 |
| **協議定義** | `protocol/v2.rs` | 訊息協議規範 | 穩定 |

### P1 優先級（高級功能）

| 子系統 | 檔案位置 | 責任 | 狀態 |
|-------|--------|------|------|
| **配置系統** | `config_api.rs` | 分層配置管理、驗證 | 穩定 |
| **命令執行** | `command_exec.rs` | 指令分發、環境執行 | 穩定 |
| **動態工具** | `dynamic_tools.rs` | 工具載入、註冊 | 穩定 |
| **MCP 集成** | tests/suite/v2/mcp_* | MCP 伺服器呼叫 | 穩定 |
| **認證系統** | app_server_tracing.rs | 用戶認證、授權 | 穩定 |

### P2 優先級（支援/測試）

| 子系統 | 檔案位置 | 責任 | 狀態 |
|-------|--------|------|------|
| **模糊搜尋** | `fuzzy_file_search.rs` | 檔案名稱模糊搜尋 | 穩定 |
| **錯誤碼** | `error_code.rs` | JSON-RPC 錯誤碼常數 | 穩定 |
| **過濾器** | `filters.rs` | 配置驗證與過濾 | 穩定 |
| **追蹤日誌** | `app_server_tracing.rs` | 結構化日誌、性能追蹤 | 穩定 |
| **特殊事件** | `bespoke_event_handling.rs` | 特定事件類型處理 | 穩定 |

---

## 7. 關鍵設計模式

### 7.1 訊息流控制（多任務同時執行）

```
Single stdio/WebSocket Connection
        │
        ├─→ Message Dispatcher
        │        │
        │        ├─→ Thread A (Conversation 1)
        │        │      │
        │        │      └─→ State: ThreadState[A]
        │        │
        │        ├─→ Thread B (Conversation 2)
        │        │      │
        │        │      └─→ State: ThreadState[B]
        │        │
        │        └─→ Global State
        │               │
        │               └─→ Config, Auth, Tools
        │
        └─→ Outbound Router
               │
               └─→ Response Back to Client
```

### 7.2 配置層疊（Config Layer Stack）

```rust
// 配置應用的優先序
let config = Config::new()
    .layer(cli_overrides)           // 最高
    .layer(env_vars)
    .layer(user_config)
    .layer(workspace_config)
    .layer(defaults);               // 最低

// 查詢時自動使用最高優先序層的值
```

### 7.3 非同步任務管理（Tokio）

```
Main Event Loop (tokio::select!)
    │
    ├─→ Inbound: Channel<JSONRPCMessage>
    │       │
    │       └─→ Process Message
    │
    ├─→ Outbound: Channel<OutgoingEnvelope>
    │       │
    │       └─→ Send to Client
    │
    └─→ Periodic Tasks (cron, heartbeat)
```

---

## 8. RPC API 方法清單

### 配置相關

| 方法 | 參數 | 返回 | 說明 |
|------|------|------|------|
| `config/get` | `key: string` | `value: any` | 取得配置值 |
| `config/set` | `key: string, value: any` | `ok: bool` | 設定配置值 |
| `config/list` | `pattern: string?` | `entries: []` | 列出配置 |
| `config/validate` | `config: object` | `errors: []` | 驗證配置 |

### 執行相關

| 方法 | 參數 | 返回 | 說明 |
|------|------|------|------|
| `thread/start` | `prompt: string` | `threadId: string` | 開始新執行緒 |
| `thread/resume` | `threadId: string` | `state: object` | 恢復執行緒 |
| `thread/list` | 無 | `threads: []` | 列出所有執行緒 |
| `thread/status` | `threadId: string` | `status: object` | 查詢狀態 |
| `turn/start` | `threadId, input` | `response: string` | 發送使用者輸入 |
| `turn/interrupt` | `threadId` | `ok: bool` | 中斷執行 |

### 工具相關

| 方法 | 參數 | 返回 | 說明 |
|------|------|------|------|
| `tools/list` | `filter: string?` | `tools: []` | 列出可用工具 |
| `tools/call` | `name, args` | `result: any` | 直接呼叫工具 |
| `tools/describe` | `name: string` | `schema: object` | 工具描述 |

---

## 9. 常見使用模式

### 9.1 基本交互流程

```javascript
// 1. 開始新執行緒
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "thread/start",
  "params": {
    "contextId": "user-123",
    "metadata": {}
  }
}

// 服務器回應
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "threadId": "thread-abc",
    "state": "ready"
  }
}

// 2. 發送提示
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "turn/start",
  "params": {
    "threadId": "thread-abc",
    "input": "Write a Python function to calculate fibonacci"
  }
}

// 服務器回應（串流）
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "state": "running",
    "output": "def fibonacci(n):\n    ..."
  }
}
```

### 9.2 配置管理

```javascript
// 取得配置
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "config/get",
  "params": { "key": "editor.tabSize" }
}

// 設定配置
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "config/set",
  "params": {
    "key": "editor.theme",
    "value": "dark"
  }
}
```

### 9.3 工具呼叫

```javascript
// 列出工具
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "tools/list"
}

// 呼叫工具
{
  "jsonrpc": "2.0",
  "id": 6,
  "method": "tools/call",
  "params": {
    "name": "execute_command",
    "args": {
      "command": "npm test"
    }
  }
}
```

---

## 10. 執行時考量

### 10.1 成本分析

| 操作 | 成本等級 | 說明 |
|------|--------|------|
| JSON-RPC 解析 | 低 | 純序列化 |
| 線程狀態查詢 | 低 | 雜湊表查詢 |
| 配置層合併 | 中 | 可能需要深層複製 |
| 命令執行 | 高 | 外部進程/MCP 呼叫 |
| 檔案操作 | 高 | I/O 阻塞 |

### 10.2 效能優化

1. **訊息批次處理**：多個 RPC 呼叫應該批次化
2. **配置快取**：頻繁查詢的配置應快取
3. **執行緒池**：使用 Tokio 執行緒池管理
4. **異步 I/O**：所有 I/O 應使用非同步

### 10.3 記憶體管理

```rust
// ThreadState 自動清理
pub struct ThreadState {
    pub conversation_history: Vec<Message>, // 可能很大
    // 應實作 LRU 快取防止記憶體洩漏
}
```

---

## 11. 集成點（與 clawtex-core 相關）

### 11.1 作為代碼執行後端

```
clawtex-core
    │
    ├─→ Code Generation Request
    │        │
    │        ▼
    │   codex-cli app-server
    │   (via JSON-RPC over WebSocket/stdio)
    │        │
    │        ├─→ Configure Environment
    │        ├─→ Execute Command
    │        └─→ Stream Results Back
    │
    └─→ Integrate Results
```

### 11.2 與 clawtex MCP 的關係

```
clawtex-core MCP Tools
    │
    └─→ Delegate to codex-cli
        (via JSON-RPC)

        codex-cli MCP Servers
        (集成外部工具)
        └─→ Execute Actual Commands
```

---

## 12. 依賴關係圖

```
app-server
├── tokio              # 非同步執行時
├── reqwest            # HTTP 用戶端
├── serde/serde_json   # 序列化
├── toml               # TOML 配置解析
├── tracing            # 結構化日誌
├── app-server-protocol # 通訊協議
├── app-server-client  # 用戶端庫
├── ansi-escape        # ANSI 處理
└── [其他工具]
    ├── uuid           # ID 生成
    ├── clap           # CLI 參數解析
    └── ...
```

---

## 13. 測試策略

### 13.1 單元測試

- 位置：各模組內 `#[cfg(test)]`
- 範圍：JSON-RPC 解析、配置驗證、狀態轉換

### 13.2 集成測試

- 位置：`tests/suite/v2/` 目錄
- 範圍：**100+ 測試**涵蓋：
  - 線程生命週期
  - 配置系統
  - MCP 集成
  - WebSocket/stdio 傳輸
  - 並發控制
  - 錯誤恢復

### 13.3 端到端測試

- 位置：`integration-tests/` 目錄
- 範圍：完整工作流程測試

---

## 14. 錯誤處理策略

### 14.1 JSON-RPC 錯誤碼

```rust
pub const PARSE_ERROR: i32 = -32700;       // 無效 JSON
pub const INVALID_REQUEST: i32 = -32600;   // 無效請求
pub const METHOD_NOT_FOUND: i32 = -32601;  // 方法不存在
pub const INVALID_PARAMS: i32 = -32602;    // 無效參數
pub const INTERNAL_ERROR: i32 = -32603;    // 內部錯誤
pub const INPUT_TOO_LARGE_ERROR_CODE: i32 = -32613;
```

### 14.2 錯誤回應

```javascript
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32602,
    "message": "Invalid params",
    "data": {
      "details": "Missing required field: threadId"
    }
  }
}
```

---

## 15. 監控與可觀測性

### 15.1 日誌等級

```bash
# 預設
RUST_LOG=codex_app_server=info

# 詳細調試
RUST_LOG=codex_app_server=debug,tokio=trace

# JSON 格式
LOG_FORMAT=json
```

### 15.2 追蹤指標

- 訊息延遲
- 執行緒執行時間
- 配置層合併時間
- 外部 API 呼叫計數

---

## 16. 安全考量

### 16.1 認證

```rust
pub struct AuthManager {
    // 驗證客戶端
    pub fn verify_client(&self, token: &str) -> Result<User>;
}
```

### 16.2 授權

- 執行緒隔離：每個執行緒有獨立狀態
- 配置限制：某些配置不允許遠程修改
- 命令白名單：可限制可執行命令

### 16.3 輸入驗證

- JSON 大小限制：`INPUT_TOO_LARGE_ERROR_CODE`
- 參數驗證：`validate_params()`
- 文件路徑限制：沙箱化

---

## 17. 故障排除

### 17.1 常見問題

| 問題 | 原因 | 解決方案 |
|------|------|---------|
| 連線掛起 | 訊息格式錯誤 | 檢查 JSON-RPC 格式 |
| 配置不生效 | 優先序錯誤 | 檢查配置層堆疊順序 |
| 執行緒泄漏 | 狀態未清理 | 確保呼叫 thread/cleanup |
| 性能下降 | 配置層過多 | 減少配置層數量 |

### 17.2 調試技巧

```bash
# 啟用詳細日誌
RUST_LOG=debug cargo run -- --listen stdio://

# JSON 格式日誌
LOG_FORMAT=json RUST_LOG=debug cargo run -- --listen stdio://

# 在客戶端測試
echo '{"jsonrpc":"2.0","id":1,"method":"ping"}' | cargo run -- --listen stdio://
```

---

## 18. 小結

Codex CLI app-server 是高可靠的代碼編輯 JSON-RPC 伺服器，特徵包括：

1. **標準化通訊**：完全遵循 JSON-RPC 2.0
2. **多重傳輸**：stdio 與 WebSocket 雙支持
3. **分層架構**：清晰的責任邊界
4. **非同步優先**：全 Tokio 基礎
5. **配置靈活**：層疊式配置系統
6. **高度可測試**：100+ 集成測試

與 clawtex-core 集成時，可作為代碼執行與工具呼叫的後端，支持複雜的多執行緒對話流程。

