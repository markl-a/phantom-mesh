# OpenFang 架構掃描

## 1. 專案概覽

**OpenFang** 是一個開源代理智能體作業系統（Agent Operating System），採用 Rust 實現，包含 14 個 crate。主要功能是統一管理多個 AI 代理的生命週期、記憶體、權限、排程及代理間通訊。

- **架構模式**：微內核 + 適配器層（Kernel + API + Channels）
- **運行模式**：Daemon（HTTP REST/WebSocket API）+ CLI 管理端
- **預設端口**：`127.0.0.1:4200`
- **配置文件**：`~/.openfang/config.toml`
- **測試覆蓋**：1744+ 單元測試
- **版本**：0.3.48

---

## 2. 目錄結構（14 個 Crate）

### 核心層（Core Kernel）
- **openfang-kernel** — 代理生命週期管理、調度、工作流引擎、事件匯流排、審批閘道、記憶體整合
- **openfang-types** — 共享數據類型（Agent、Event、Capability、Config、Memory）
- **openfang-runtime** — LLM 驅動、工具運行時、沙箱（WASM/Docker/Python）、MCP 客戶端、審計日誌

### API 層（HTTP/WebSocket）
- **openfang-api** — 路由、中介軟體、認證、速率限制、WebSocket 伺服器、OpenAI 相容端點

### 通道層（40+ 訊息整合）
- **openfang-channels** — Telegram、Discord、Slack、Teams、WhatsApp、Signal、Twitch、Twitter、Matrix、Mattermost、IRC、Email、IRC、Feishu、DingTalk、Line、Mastodon、Messenger、Reddit、Revolt、Viber、Flock、Guilded、Keybase、Nextcloud、Nostr、Pumble、Threema、Twist、WebEx、Gitter、Gotify、LinkedIn、Mumble（及更多）

### 記憶體層（三層存儲）
- **openfang-memory** — 結構化存儲（SQLite）、語義搜尋（LIKE/向量）、知識圖譜

### 延伸系統
- **openfang-extensions** — 憑證管理、OAuth、外掛登錄、健康檢查、保管庫
- **openfang-cli** — TUI 儀表板（代理、通道、工作流、日誌、安全、設定）
- **openfang-desktop** — Tauri 桌面應用、系統託盤、更新器、快捷鍵
- **openfang-migrate** — 資料遷移、報告
- **openfang-network** — A2A（代理-代理）通訊、OFP 協議（p2p 網路）

---

## 3. 核心 Trait 與 Struct

### 3.1 主要 Trait

1. **`KernelHandle`** （`openfang-runtime/kernel_handle.rs`）
   - 避免 runtime ↔ kernel 的循環依賴
   - 提供核心服務的異步訪問介面

2. **`LlmDriver`** （`openfang-runtime/llm_driver.rs`）
   - LLM 提供者的統一抽象（Groq、OpenAI、Claude、Ollama、Moonshot）
   - 方法：`async fn complete(request: CompletionRequest) -> Result<CompletionResponse>`

3. **`Memory`** （`openfang-types/memory.rs`）
   - 結構化 + 語義 + 知識圖譜存儲的統一介面
   - 支援鍵值、文本搜尋、實體-關係查詢

4. **`ChannelBridge`** （`openfang-channels/bridge.rs`）
   - 將外部平台訊息轉換為內部 `ChannelMessage` 事件
   - 實現：40+ 適配器（Discord、Slack、Telegram...）

5. **`Capability`** （`openfang-types/capability.rs`）
   - 定義代理的許可、工具存取、費用額度
   - 支援基於角色的存取控制（RBAC）

### 3.2 主要 Struct

1. **`OpenFangKernel`** （`openfang-kernel/kernel.rs`）
   - 中央協調器，組裝所有子系統
   - 欄位：registry、capabilities、event_bus、scheduler、memory、workflows、triggers、supervisor、background

2. **`AgentRegistry`** （`openfang-kernel/registry.rs`）
   - 代理元數據存儲（DashMap）
   - 支援即時查詢、過濾、訂閱更新

3. **`EventBus`** （`openfang-kernel/event_bus.rs`）
   - Pub/Sub 事件分發（Tokio broadcast channel）
   - 1000 條事件的歷史環形緩衝區
   - 支援播播、單播、代理級訂閱

4. **`TriggerEngine`** （`openfang-kernel/triggers.rs`）
   - 事件驅動的代理激活（Lifecycle、ContentMatch、MemoryUpdate...）
   - 模式匹配：正則表達式、子字符串、關鍵字

5. **`WorkflowEngine`** （`openfang-kernel/workflow.rs`）
   - 多步代理管道：序列、並行、條件分支、迴圈
   - 支援步驟間數據傳遞、變數儲存、條件跳過

6. **`MeteringEngine`** （`openfang-kernel/metering.rs`）
   - 追蹤每個代理的令牌使用、API 呼叫、成本
   - 支援全局 + 單代理預算限制、速率限制

7. **`MemorySubstrate`** （`openfang-memory/substrate.rs`）
   - 三層存儲的統一入口
   - 方法：store、retrieve、search_semantic、update_knowledge_graph

8. **`AppState`** （`openfang-api/routes.rs`）
   - HTTP 層與 kernel 的橋接
   - 欄位：kernel、bridge_manager、channels_config、shutdown_notify

9. **`Supervisor`** （`openfang-kernel/supervisor.rs`）
   - 進程健康檢查、重啟策略、資源監控
   - 支援心跳、故障檢測、優雅關閉

10. **`CapabilityManager`** （`openfang-kernel/capabilities.rs`）
    - 代理許可管理
    - 方法：grant、revoke、check_access

---

## 4. 啟動流程

```
openfang.exe start
  ↓
[CLI] → main() (openfang-cli/src/main.rs)
  ├─ 讀取 ~/.openfang/config.toml
  ├─ 驗證 API 密鑰、LLM 配置
  ├─ 初始化 KernelConfig
  ↓
[Runtime] → boot_kernel()
  ├─ 建立 LlmDriver（Groq/OpenAI/Ollama）
  ├─ 初始化 MemorySubstrate（SQLite）
  ├─ 启動 AgentRegistry
  ├─ 建立 EventBus（broadcast channel）
  ├─ 建立 TriggerEngine
  ├─ 建立 WorkflowEngine
  ├─ 建立 Supervisor
  ├─ 建立 MeteringEngine
  ├─ 建立 CapabilityManager
  ↓
[API] → build_router()
  ├─ 啟動 ChannelBridge（連結 Telegram、Discord 等）
  ├─ 建立 Axum 路由（GET /api/agents、POST /api/message...）
  ├─ 註冊 WebSocket 端點（/ws/agent/:id）
  ├─ 配置 CORS、中介軟體、速率限制
  ├─ 啟動 HTTP 伺服器（0.0.0.0:4200）
  ↓
[Daemon Ready]
  ├─ CLI 連結 HTTP API
  ├─ 開始監聽代理訊息、事件、工作流請求
  ├─ 寫入 ~/.openfang/daemon.json (PID、地址、版本)
  ↓
[持續運行]
  ├─ Scheduler 定期檢查 Cron 任務
  ├─ EventBus 分發事件 → TriggerEngine 匹配 → 激活代理
  ├─ WorkflowEngine 執行多步管道
  ├─ MeteringEngine 追蹤成本
  ├─ ChannelBridge 轉發外部訊息
```

---

## 5. 資料流 ASCII 圖

### 代理訊息流（Message Flow）

```
外部平台              OpenFang 核心                   LLM 提供者
(Telegram/Discord)
       │
       │ 訊息事件
       ├──────────────────→ ChannelBridge
       │                      ↓
       │                  ChannelMessage
       │                      ↓
       │                  EventBus.publish()
       │                      ↓
       │              TriggerEngine.match()
       │                      ↓
       │        (若匹配，激活相應代理)
       │                      ↓
       │              AgentRegistry.get(id)
       │                      ↓
       │          run_agent_loop()
       │              ↓           ↓           ↓
       │          [記憶體]    [工具檢查]  [LLM 驅動]
       │              ↓           ↓           ↓
       │          Memory    Tool Policy   LlmDriver
       │          Substrate  Evaluation    .complete()
       │              ↓           ↓           ↓
       │          ┌──────────────────────────────┐
       │          │  CompletionRequest           │
       │          │  {                           │
       │          │    model: "groq/mixtral",    │
       │          │    messages: [               │
       │          │      {role: user, content}   │
       │          │    ],                        │
       │          │    tools: [...]              │
       │          │  }                           │
       │          └──────────────────────────────┘
       │                      ↓
       │                    HTTP/gRPC
       ├──────────────────────────────────────────→ Groq/OpenAI/Claude
       │                                             (LLM complete())
       │                    HTTP/gRPC
       ├────────────────────────────────────────────← CompletionResponse
       │                      ↓
       │          run_agent_loop() 繼續
       │              ↓
       │          Tool Execution
       │          (file_read, web_search, ...)
       │              ↓
       │          (若需要) 遞迴 LLM 呼叫
       │              ↓
       │          反應產生
       │              ↓
       │          MeteringEngine 記錄
       │          (tokens, cost, duration)
       │              ↓
       │          Response 儲存至記憶體
       │              ↓
       │          Event 發佈至 EventBus
       │              ↓
       │          ChannelBridge 反向轉譯
       │              ↓
       ├──────────────────────→ 回傳訊息至外部平台
```

### 工作流執行流（Workflow Execution）

```
WorkflowEngine.run(workflow_id)
       ↓
[步驟 1: Agent A] ──→ input_data
       ↓
AgentRegistry.get(A)
       ↓
run_agent_loop(A, input_data)
       ↓
output_1
       ↓
[步驟 2: Agent B] ──→ output_1 + input_data
       ↓
(若條件 check: if output_1.contains("success"))
       ├─ True: 執行步驟 2
       └─ False: 跳至步驟 5
       ↓
output_2
       ↓
[步驟 3-4: 並行] ──┬→ Agent C (input: output_2)
       ├─output_3C
       │
       ├→ Agent D (input: output_2)
       └─output_3D
       ↓
合併結果 → final_output
       ↓
WorkflowRunId 標記完成
       ↓
EventBus.publish(WorkflowCompleted)
```

### 記憶體層結構

```
Memory Request
       ↓
MemorySubstrate.store(key, content, category)
       │
       ├─→ StructuredStore (SQLite KV)
       │   {key: "user:12345:preferences", value: "..."}
       │
       ├─→ SemanticStore (文本索引 + 向量)
       │   {embedding: [0.1, 0.2, ...], text: "..."}
       │
       └─→ KnowledgeGraph (實體-關係)
           {entity_1: "Alice", relation: "knows", entity_2: "Bob"}

Memory.retrieve(key) / search_semantic(query) / find_entities(pattern)
       ↓
Return cached results (in-process or remote Qdrant)
```

---

## 6. 子系統清單

### P0（關鍵）— 必須正常運作
| 子系統 | 位置 | 責任 |
|--------|------|------|
| **OpenFangKernel** | `openfang-kernel/kernel.rs` | 中央協調器、生命週期管理 |
| **AgentRegistry** | `openfang-kernel/registry.rs` | 代理元數據、狀態追蹤 |
| **EventBus** | `openfang-kernel/event_bus.rs` | 事件分發、代理激活 |
| **LlmDriver** | `openfang-runtime/llm_driver.rs` | LLM 呼叫、串流回應 |
| **Memory (3-layer)** | `openfang-memory/substrate.rs` | 結構化、語義、知識圖譜存儲 |
| **ChannelBridge** | `openfang-channels/bridge.rs` | 外部平台整合（Telegram、Discord...） |
| **HTTP API Server** | `openfang-api/server.rs` | REST/WebSocket 伺服器、路由 |
| **Config Loading** | `openfang-kernel/config.rs` | TOML 解析、環境變數覆蓋 |
| **Tool Runner** | `openfang-runtime/tool_runner.rs` | 工具執行、沙箱隔離 |

### P1（重要）— 支援進階功能
| 子系統 | 位置 | 責任 |
|--------|------|------|
| **WorkflowEngine** | `openfang-kernel/workflow.rs` | 多步管道、條件、並行執行 |
| **TriggerEngine** | `openfang-kernel/triggers.rs` | 事件驅動的代理激活 |
| **MeteringEngine** | `openfang-kernel/metering.rs` | 成本追蹤、預算限制、速率限制 |
| **Supervisor** | `openfang-kernel/supervisor.rs` | 進程健康檢查、重啟 |
| **CapabilityManager** | `openfang-kernel/capabilities.rs` | 權限管理、角色-代理映射 |
| **ApprovalGate** | `openfang-kernel/approval.rs` | 人工確認工作流、審計 |
| **AuthManager** | `openfang-kernel/auth.rs` | API 金鑰、會話管理、2FA |
| **BackgroundExecutor** | `openfang-kernel/background.rs` | 異步背景任務調度 |
| **Scheduler** | `openfang-kernel/scheduler.rs` | Cron 任務、定期執行 |
| **WasmSandbox** | `openfang-runtime/sandbox.rs` | WASM 外掛執行、資源隔離 |

### P2（優化）— 增強體驗
| 子系統 | 位置 | 責任 |
|--------|------|------|
| **AuditLog** | `openfang-runtime/audit.rs` | 請求日誌、稽核追蹤 |
| **ModelRouter** | `openfang-runtime/routing.rs` | 動態模型選擇、故障轉移 |
| **CompactionEngine** | `openfang-runtime/compactor.rs` | 上下文最佳化、長上下文壓縮 |
| **MCP Client** | `openfang-runtime/mcp.rs` | 模型上下文協議整合 |
| **Link Understanding** | `openfang-runtime/link_understanding.rs` | URL 預覽、內容擷取 |
| **ImageGeneration** | `openfang-runtime/image_gen.rs` | 圖像生成（DALL-E、Stable Diffusion） |
| **BrowserAutomation** | `openfang-runtime/browser.rs` | Playwright 整合、網頁互動 |
| **PythonRuntime** | `openfang-runtime/python_runtime.rs` | 內嵌 Python、代理腳本執行 |
| **DockerSandbox** | `openfang-runtime/docker_sandbox.rs` | Docker 隔離、代碼執行 |
| **LoopGuard** | `openfang-runtime/loop_guard.rs` | 無限迴圈檢測、重複執行防護 |
| **CLI TUI** | `openfang-cli/ui.rs` | 互動式儀表板 |
| **Desktop App** | `openfang-desktop/src/main.rs` | Tauri 桌面應用 |
| **ExtensionRegistry** | `openfang-extensions/registry.rs` | 第三方外掛管理 |
| **OAuth Flow** | `openfang-extensions/oauth.rs` | 代理的 OAuth 授權流 |

---

## 7. 關鍵設計模式

### 7.1 避免循環依賴
- **`KernelHandle` 抽象**：runtime 不直接導入 kernel，而是通過 trait 訪問
- **分層架構**：types ← kernel ← runtime ← api

### 7.2 配置驗證
- **三級覆蓋**：defaults → TOML → 環境變數
- **延遲初始化**：LLM driver 在啟動時才驗證 API 金鑰

### 7.3 記憶體三層設計
- **結構化**：快速 KV 查詢（session 狀態、偏好）
- **語義**：文本搜尋（過往對話、知識庫）
- **圖論**：實體-關係（組織結構、信任網路）

### 7.4 非同步範式
- **Tokio 執行時**：所有 I/O 非同步（HTTP、LLM、DB）
- **Broadcast Channel**：EventBus 多播事件，避免互斥鎖爭用

### 7.5 沙箱隔離
- **WASM**：外掛執行（輕量、跨平台）
- **Docker**：不可信代碼（資源隔離）
- **Python subprocess**：腳本執行（安全邊界）

---

## 8. 部署與運維

### 8.1 健康檢查
- **DAI HTTP 端點**：`GET /api/health`
- **Supervisor 心跳**：每 30 秒一次
- **LLM 驗證**：啟動時連結提供者

### 8.2 故障轉移
- **ModelRouter**：若 Groq 失敗 → 重試 OpenAI → Ollama 本地
- **ConnectionPool**：HTTP 客戶端池，自動重連
- **CircuitBreaker**：故障提供者暫時隔離

### 8.3 監控指標
- **MeteringEngine**：每個代理的 token 數、成本、延遲
- **EventBus 歷史**：1000 條事件環形緩衝
- **Audit Log**：完整的 API 請求、工具執行、異常

### 8.4 優雅關閉
- **shutdown_notify**：廣播關閉信號
- **存儲待処理任務**：寫入 SQLite
- **等待運行中工作流**：最多 30 秒超時

---

## 9. 與 Clawtex 的對應關係

| OpenFang 元件 | Clawtex 對應 |
|---------------|--------------|
| OpenFangKernel | clawtex-core daemon |
| AgentRegistry | agents.toml 中的代理定義 |
| ChannelBridge | Telegram 機器人、其他通道 |
| EventBus | 事件驅動的手（hands） |
| WorkflowEngine | 多相手工作流（TOML 定義） |
| MeteringEngine | ~.clawtex/costs.db |
| Memory (3-layer) | ~.clawtex/memory.db 等 |
| LlmDriver | providers 抽象（Ollama/OpenAI/Anthropic...） |
| Supervisor | daemon 進程監控 |

---

## 10. 測試與驗證

### 10.1 構建檢查表
```bash
cargo build --workspace --lib          # 編譯通過
cargo test --workspace                 # 1744+ 測試通過
cargo clippy --workspace --all-targets -- -D warnings  # 零警告
```

### 10.2 實時整合測試
1. 啟動 daemon：`GROQ_API_KEY=xxx target/release/openfang.exe start`
2. 驗證 API：`curl http://127.0.0.1:4200/api/health`
3. 列出代理：`curl http://127.0.0.1:4200/api/agents`
4. 發送訊息：`curl -X POST http://127.0.0.1:4200/api/agents/{id}/message`
5. 檢查成本：`curl http://127.0.0.1:4200/api/budget`

### 10.3 關鍵 API 端點
| 端點 | 方法 | 用途 |
|------|------|------|
| `/api/health` | GET | 基本健康檢查 |
| `/api/agents` | GET | 列出所有代理 |
| `/api/agents/{id}/message` | POST | 發送訊息（觸發 LLM） |
| `/api/budget` | GET/PUT | 全局預算 |
| `/api/network/status` | GET | OFP p2p 網路狀態 |
| `/api/a2a/agents` | GET | 外部 A2A 代理 |
| `/ws/agent/{id}` | WebSocket | 即時通訊 |

---

## 11. 常見陷阱與解決方案

| 陷阱 | 症狀 | 解決 |
|------|------|------|
| **API 金鑰缺失** | `MissingApiKey` 錯誤 | 設定 `GROQ_API_KEY` 或在 UI 配置 |
| **端點未註冊** | 404 Not Found | 檢查 `server.rs` 路由、`routes.rs` 實現 |
| **記憶體 DB 分離** | SQLite `:memory:` 進程隔離 | 測試用 `tempfile`，生產用檔案路徑 |
| **循環依賴編譯失敗** | 編譯錯誤 | 確保透過 `KernelHandle` 訪問內核 |
| **Config 反序列化失敗** | TOML 載入錯誤 | 檢查 `#[serde(default)]` 與 `Default impl` |
| **Windows 路徑問題** | 反斜槓路徑錯誤 | 使用 `Path::new()` 或統一正斜槓 |

---

**文檔版本**：OpenFang v0.3.48
**更新日期**：2026-03-13
**針對項目**：Clawtex ZeroClaw 架構參考
