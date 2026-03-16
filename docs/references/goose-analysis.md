# Goose (Block/Square) 深度技術分析 v2

> **版本**: v1.27.0 | **語言**: Rust (edition 2021) | **授權**: Apache-2.0
> **倉庫**: `references/goose/` | **分析日期**: 2026-03-13 | **分析深度**: L3 (原始碼級)

---

## 目錄

1. [專案結構與依賴圖](#1-專案結構與依賴圖)
2. [進入點與啟動流程](#2-進入點與啟動流程)
3. [Agent 核心迴圈 — 完整資料流](#3-agent-核心迴圈--完整資料流)
4. [ToolStream — Future + Notification 統一串流](#4-toolstream--future--notification-統一串流)
5. [Inspector Pipeline — 三層安全檢查](#5-inspector-pipeline--三層安全檢查)
6. [Extension Manager — 5 種 MCP Extension 的生命週期](#6-extension-manager--5-種-mcp-extension-的生命週期)
7. [Recipe 系統 — YAML + MiniJinja Template](#7-recipe-系統--yaml--minijinja-template)
8. [Lead-Worker 雙模型策略](#8-lead-worker-雙模型策略)
9. [Subagent Handler — 子代理隔離與錯誤冒泡](#9-subagent-handler--子代理隔離與錯誤冒泡)
10. [MOIM — 背景任務狀態注入](#10-moim--背景任務狀態注入)
11. [上下文管理 — 壓縮與 Token 計數](#11-上下文管理--壓縮與-token-計數)
12. [Provider 抽象與 Canonical Registry](#12-provider-抽象與-canonical-registry)
13. [Local Inference — llama.cpp 本地推理引擎](#13-local-inference--llamacpp-本地推理引擎)
14. [ACP — Agent Communication Protocol](#14-acp--agent-communication-protocol)
15. [錯誤處理策略全圖](#15-錯誤處理策略全圖)
16. [效能特徵與瓶頸分析](#16-效能特徵與瓶頸分析)
17. [Clawtex-Core 差距對比與具體實作建議](#17-clawtex-core-差距對比與具體實作建議)

---

## 1. 專案結構與依賴圖

### 1.1 Workspace 概覽

Goose 使用 Rust workspace 多 crate 架構，成員位於 `crates/*`:

```
goose/
  Cargo.toml              # workspace root
  crates/
    goose/                 # 核心引擎 (agent, providers, recipe, context, security)
    goose-cli/             # CLI 入口 (clap 解析, session 管理)
    goose-server/          # HTTP API 服務 (axum, SSE, WebSocket)
    goose-mcp/             # MCP 伺服器實作 (developer, memory, tutorial)
    goose-acp/             # ACP (Agent Communication Protocol) 適配器
    goose-acp-macros/      # ACP proc macros
    goose-test/            # 整合測試框架
    goose-test-support/    # 測試輔助工具
  ui/
    desktop/               # Electron 桌面 UI (React/TypeScript)
    acp/                   # ACP UI 元件
    text/                  # 文字 UI
  documentation/           # Docusaurus 文件站
  evals/                   # 評估基準 (open-model-gym)
  scripts/                 # 建置/部署腳本
  vendor/v8/               # V8 引擎 vendor (for local inference)
```

### 1.2 核心 Crate 模組結構

**檔案**: `crates/goose/src/`

```
src/
  agents/                     # Agent 核心迴圈 + 工具調度
    agent.rs                  # Agent struct, reply(), reply_internal() — 1500+ 行
    tool_execution.rs         # ToolCallResult, handle_approval, handle_frontend
    extension_manager.rs      # ExtensionManager, 1700+ 行, MCP 生命週期
    mcp_client.rs             # GooseClient (McpClientTrait), MCP sampling/elicitation
    subagent_handler.rs       # run_subagent_task(), SubagentRunParams
    subagent_task_config.rs   # TaskConfig (provider, extensions, max_turns)
    subagent_execution_tool/  # 子代理工具定義 + 通知事件
    platform_extensions/      # 內建平台工具
      developer/              # shell, edit, tree
        shell.rs              # Shell 執行 (跨平台, timeout, NO_WINDOW)
        edit.rs               # 檔案編輯
        tree.rs               # 目錄樹
      analyze/                # 程式碼分析 (tree-sitter, 10 語言)
        parser.rs             # TreeSitter 解析器
        graph.rs              # 依賴圖
        languages.rs          # Go/Java/JS/Kotlin/Python/Ruby/Rust/Swift/TS
      summon.rs               # load + delegate 統一入口 (800+ 行)
      todo.rs                 # 待辦事項管理
      tom.rs                  # Tool Output Manager
      chatrecall.rs           # 對話回憶
      code_execution.rs       # 程式碼執行 (Docker sandbox)
      summarize.rs            # 摘要工具
      apps.rs                 # 應用程式管理
      ext_manager.rs          # 動態 extension 管理工具
    container.rs              # Docker 容器隔離
    prompt_manager.rs         # 提示詞管理器 (system prompt 組裝)
    retry.rs                  # RetryManager + shell success checks
    large_response_handler.rs # 大型回應截斷 + 摘要
    extension_malware_check.rs# Extension 惡意軟體掃描
    moim.rs                   # MOIM (Model-Oriented Information Mesh)
    validate_extensions.rs    # Extension 驗證
    types.rs                  # SharedProvider, SessionConfig, RetryConfig
  providers/                  # LLM 供應商抽象 (~30 個)
    base.rs                   # Provider trait, MessageStream, ProviderUsage
    init.rs                   # ProviderRegistry 初始化 (OnceCell + RwLock)
    lead_worker.rs            # Lead-Worker 雙模型策略
    local_inference/          # 本地推論引擎
      inference_engine.rs     # llama.cpp binding, LoadedModel, generation_loop
      inference_emulated_tools.rs  # 模擬 tool calling
      inference_native_tools.rs    # 原生 tool calling
      local_model_registry.rs     # 本地模型註冊表
      hf_models.rs            # HuggingFace 模型下載
      tool_parsing.rs         # 工具解析
    canonical/                # 標準化模型資料庫
      registry.rs             # CanonicalModelRegistry
      model.rs                # CanonicalModel 結構
      data/                   # JSON 模型元數據
    formats/                  # API 格式轉換 (anthropic, openai, google, bedrock...)
    declarative/              # JSON 宣告式 provider (groq, deepseek, mistral...)
    errors.rs                 # ProviderError enum
    retry.rs                  # 指數退避重試
    toolshim.rs               # 工具 shim (為不支援 tool 的模型模擬)
  recipe/                     # YAML Recipe 系統
    mod.rs                    # Recipe struct, RecipeBuilder, SubRecipe
    build_recipe/             # Recipe 建置流程, 參數解析
    template_recipe.rs        # MiniJinja 範本渲染
    validate_recipe.rs        # Recipe 驗證
    local_recipes.rs          # 本地 recipe 發現
    recipe_extension_adapter.rs # Extension 配置適配
  context_mgmt/               # 上下文壓縮 + Token 管理
    mod.rs                    # compact_messages(), check_if_compaction_needed()
  conversation/               # Conversation + Message 型別
  security/                   # 安全掃描 (prompt injection 偵測)
  permission/                 # 權限管理 (inspector, judge, store)
  session/                    # Session 持久化 (SQLite)
  execution/                  # 執行管理器 (AgentManager)
  tool_inspection.rs          # ToolInspector trait, ToolInspectionManager
  tool_monitor.rs             # RepetitionInspector (重複呼叫偵測)
  token_counter.rs            # tiktoken-rs Token 計數 + DashMap 快取
  prompt_template.rs          # MiniJinja 範本引擎
  scheduler.rs                # 排程系統
```

### 1.3 Crate 間依賴圖

```
                    goose-cli ──────────┐
                        │               │
                    goose-server ───┐    │
                        │          │    │
                    goose-acp ─────┼────┤
                        │          │    │
                    ┌───▼──────────▼────▼───┐
                    │       goose (核心)      │
                    │  agents / providers /   │
                    │  recipe / context_mgmt  │
                    └───────────┬────────────┘
                                │
                    ┌───────────▼────────────┐
                    │     goose-mcp           │
                    │  (developer, memory,    │
                    │   tutorial extensions)  │
                    └────────────────────────┘
                                │
                         rmcp (MCP SDK)
                         sacp (ACP SDK)
                         llama-cpp-2 (local LLM)
```

### 1.4 關鍵依賴

| 依賴 | 版本 | 用途 |
|------|------|------|
| `rmcp` | 1.2.0 | MCP 協定 SDK (JSON-RPC 2.0, stdio/SSE/streamable-http) |
| `sacp` | 10.1.0 | ACP 協定 (Agent Communication Protocol) |
| `axum` | 0.8 | HTTP 伺服器框架 |
| `tokio` | 1.49 | 非同步執行環境 |
| `tiktoken-rs` | - | OpenAI 相容 Token 計數 |
| `tree-sitter-*` | - | 程式碼分析 (10 語言) |
| `minijinja` | - | 範本引擎 (Recipe 參數化) |
| `llama-cpp-2` | - | 本地推理 (GGUF 模型) |
| `opentelemetry` | 0.31 | 可觀測性 (OTLP) |
| `schemars` | 1.0 | JSON Schema 生成 |
| `async-stream` | - | 非同步串流巨集 |
| `dashmap` | - | 無鎖並行 HashMap (Token 快取) |

---

## 2. 進入點與啟動流程

### 2.1 CLI 入口

**檔案**: `crates/goose-cli/src/main.rs` (L1-L12)

```rust
#[tokio::main]
async fn main() -> Result<()> {
    if let Err(e) = goose_cli::logging::setup_logging(None) {
        eprintln!("Warning: Failed to initialize logging: {}", e);
    }
    let result = cli().await;
    if goose::otel::otlp::is_otlp_initialized() {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        goose::otel::otlp::shutdown_otlp();
    }
    result
}
```

啟動序列:
1. **setup_logging()** -- tracing-subscriber 初始化
2. **cli()** -- clap 解析命令列
3. **OTLP 延遲 shutdown** -- 等待 100ms 確保 traces 全部匯出

### 2.2 Server 入口 (Desktop UI 後端)

**檔案**: `crates/goose-server/src/state.rs`

```rust
pub struct AppState {
    pub(crate) agent_manager: Arc<AgentManager>,
    pub recipe_file_hash_map: Arc<Mutex<HashMap<String, PathBuf>>>,
    pub tunnel_manager: Arc<TunnelManager>,
    pub gateway_manager: Arc<GatewayManager>,
    pub extension_loading_tasks: ExtensionLoadingTasks,
    pub inference_runtime: Arc<InferenceRuntime>,
}
```

`AppState` 是 server 的核心容器。`AgentManager` 管理所有 session 的 Agent 實例; `GatewayManager` 處理 SSE/WebSocket 串流; `InferenceRuntime` 管理本地推理引擎。

---

## 3. Agent 核心迴圈 -- 完整資料流

### 3.1 Agent 結構

**檔案**: `crates/goose/src/agents/agent.rs` (L135-L152)

```rust
pub struct Agent {
    pub(super) provider: SharedProvider,                    // Arc<Mutex<Option<Arc<dyn Provider>>>>
    pub config: AgentConfig,
    pub extension_manager: Arc<ExtensionManager>,
    pub(super) final_output_tool: Arc<Mutex<Option<FinalOutputTool>>>,
    pub(super) frontend_tools: Mutex<HashMap<String, FrontendTool>>,
    pub(super) frontend_instructions: Mutex<Option<String>>,
    pub(super) prompt_manager: Mutex<PromptManager>,
    pub(super) confirmation_tx: mpsc::Sender<(String, PermissionConfirmation)>,
    pub(super) confirmation_rx: Mutex<mpsc::Receiver<(String, PermissionConfirmation)>>,
    pub(super) tool_result_tx: mpsc::Sender<(String, ToolResult<CallToolResult>)>,
    pub(super) tool_result_rx: ToolResultReceiver,
    pub(super) retry_manager: RetryManager,
    pub(super) tool_inspection_manager: ToolInspectionManager,
    container: Mutex<Option<Container>>,
}
```

**14 個欄位**，其中 7 個被 Mutex 包裝。確認/工具結果使用 mpsc channel (buffer=32) 在 Agent 和 UI 之間異步通訊。

### 3.2 AgentEvent 串流

```rust
#[derive(Clone, Debug)]
pub enum AgentEvent {
    Message(Message),                           // 一般消息
    McpNotification((String, ServerNotification)), // MCP 通知
    ModelChange { model: String, mode: String },   // 模型切換 (Lead-Worker)
    HistoryReplaced(Conversation),              // 壓縮後的新歷史
}
```

### 3.3 完整資料流 ASCII 圖

```
User Message
    │
    ▼
reply(user_message, session_config, cancel_token)
    │
    ├─ 處理 Elicitation 回應 (ElicitationResponse)
    ├─ 處理 Slash Commands (/compact, /clear, /recipe)
    ├─ 存儲 user_message 到 session (SQLite)
    │
    ▼
check_if_compaction_needed()
    │
    ├─ usage_ratio = current_tokens / context_limit
    ├─ 超過 80% → compact_messages()
    │     ├─ 漸進式移除 tool responses (0%, 10%, 20%, 50%, 100%)
    │     ├─ provider.complete_fast() 生成摘要
    │     ├─ 原始消息 → agent_invisible
    │     ├─ 摘要 → agent_only
    │     └─ yield HistoryReplaced(compacted)
    │
    ▼
reply_internal(conversation, session_config, session, cancel_token)
    │
    ├─ prepare_reply_context()
    │     ├─ fix_conversation() — 修復消息順序
    │     ├─ prepare_tools_and_prompt() — 收集所有 extension tools
    │     └─ apply_tool_annotations() (SmartApprove mode)
    │
    ├─ reset_retry_attempts()
    ├─ maybe_update_name() (背景 task, LLM 生成 4 字摘要)
    │
    ▼ ─────── Agent Loop ───────
    │
    │  ┌─ 檢查 cancel_token
    │  ├─ 檢查 final_output_tool (結構化輸出)
    │  ├─ 檢查 turns_taken < max_turns (預設 1000)
    │  │
    │  ├─ maybe_summarize_tool_pair() — 背景 tool pair 摘要 (停用中)
    │  ├─ inject_moim() — 注入時間+路徑+背景任務狀態
    │  │
    │  ├─ stream_response_from_provider()
    │  │     ├─ provider.stream(model_config, session_id, system, messages, tools)
    │  │     └─ 收集串流: (Option<Message>, Option<ProviderUsage>)
    │  │
    │  ├─ 如果是 Lead-Worker → yield ModelChange event
    │  ├─ update_session_metrics()
    │  │
    │  ├─ categorize_tools(response, tools)
    │  │     ├─ frontend_requests: Vec<ToolRequest>
    │  │     └─ remaining_requests: Vec<ToolRequest>
    │  │
    │  ├─ yield AgentEvent::Message(filtered_response)
    │  │
    │  ├─ 如果 num_tool_requests == 0 → 記錄文字 → continue
    │  │
    │  ├─ 建立 request_to_response_map (每個 request → Arc<Mutex<Message>>)
    │  │
    │  ├─ 處理 frontend_requests → handle_frontend_tool_request()
    │  │     └─ yield FrontendToolRequest → 等待 tool_result_rx
    │  │
    │  ├─ GooseMode::Chat → 所有工具跳過，返回計畫說明
    │  │
    │  ├─ tool_inspection_manager.inspect_tools()  ─── Inspector Pipeline ───
    │  │     ├─ SecurityInspector (最高優先級)
    │  │     ├─ PermissionInspector (中高優先級)
    │  │     └─ RepetitionInspector (低優先級)
    │  │
    │  ├─ process_inspection_results → approved / needs_approval / denied
    │  │
    │  ├─ handle_approved_and_denied_tools()
    │  │     ├─ approved → dispatch_tool_call() → tool_stream()
    │  │     └─ denied → DECLINED_RESPONSE
    │  │
    │  ├─ handle_approval_tool_requests()
    │  │     ├─ yield ActionRequired (UI 顯示確認)
    │  │     ├─ 等待 confirmation_rx
    │  │     ├─ AllowOnce / AlwaysAllow → dispatch_tool_call()
    │  │     └─ DenyOnce / AlwaysDeny → DECLINED_RESPONSE
    │  │
    │  ├─ stream::select_all(tool_futures) ─── 並行執行 ───
    │  │     ├─ ToolStreamItem::Result → 收集到 response_msg
    │  │     ├─ ToolStreamItem::Message → yield McpNotification
    │  │     └─ 定期 drain elicitation messages
    │  │
    │  ├─ 收集所有 tool_response_messages
    │  ├─ 處理 extension install 結果 → refresh tools
    │  ├─ yield 每個 tool_response
    │  │
    │  ├─ 如果觸發 recovery compaction → compact + HistoryReplaced
    │  │
    │  ├─ 注入 tool responses 到 conversation
    │  ├─ 存儲到 session
    │  │
    │  └─ handle_retry_logic() — Recipe retry 檢查
    │        ├─ execute_success_checks() — shell 命令驗證
    │        ├─ 失敗 → execute_on_failure_command()
    │        └─ 重置 conversation 到初始狀態
    │
    └─ loop → 繼續下一輪
```

### 3.4 dispatch_tool_call 細節

**檔案**: `crates/goose/src/agents/agent.rs` (L498-L590)

```rust
pub async fn dispatch_tool_call(
    &self,
    tool_call: CallToolRequestParams,
    request_id: String,
    cancellation_token: Option<CancellationToken>,
    session: &Session,
) -> (String, Result<ToolCallResult, ErrorData>) {
    // 1. 記錄 OpenTelemetry span
    tracing::Span::current().record("input", ...);

    // 2. 特殊工具快速路徑
    if tool_call.name == PLATFORM_MANAGE_SCHEDULE_TOOL_NAME { ... }
    if tool_call.name == FINAL_OUTPUT_TOOL_NAME { ... }

    // 3. Frontend 工具 → 返回錯誤 (由 UI 執行)
    if self.is_frontend_tool(&tool_call.name).await { ... }

    // 4. 普通工具 → ExtensionManager
    let result = self.extension_manager
        .dispatch_tool_call(&session.id, tool_call, ...)
        .await;

    // 5. 錯誤處理 + PostHog analytics
    result.unwrap_or_else(|e| {
        crate::posthog::emit_error("tool_execution_failed", ...);
        ToolCallResult::from(Err(error_data))
    });

    // 6. 大型回應處理
    (request_id, Ok(ToolCallResult {
        notification_stream: result.notification_stream,
        result: Box::new(result.result.map(
            large_response_handler::process_tool_response
        )),
    }))
}
```

**關鍵洞察**: 每個 dispatch 都經過 3 層包裝:
1. **Agent 層**: 特殊工具路由 + OTel tracing
2. **ExtensionManager 層**: 工具名稱解析 + MCP 路由
3. **MCP Client 層**: JSON-RPC call_tool + 通知訂閱

---

## 4. ToolStream -- Future + Notification 統一串流

### 4.1 核心設計

**檔案**: `crates/goose/src/agents/agent.rs` (L168-L200)

```rust
pub enum ToolStreamItem<T> {
    Message(ServerNotification),  // MCP 通知 (進度、日誌)
    Result(T),                    // 最終結果
}

pub type ToolStream =
    Pin<Box<dyn Stream<Item = ToolStreamItem<ToolResult<CallToolResult>>> + Send>>;

pub fn tool_stream<S, F>(rx: S, done: F) -> ToolStream
where
    S: Stream<Item = ServerNotification> + Send + Unpin + 'static,
    F: Future<Output = ToolResult<CallToolResult>> + Send + 'static,
{
    Box::pin(async_stream::stream! {
        tokio::pin!(done);
        let mut rx = rx;

        loop {
            tokio::select! {
                Some(msg) = rx.next() => {
                    yield ToolStreamItem::Message(msg);
                }
                r = &mut done => {
                    yield ToolStreamItem::Result(r);
                    break;
                }
            }
        }
    })
}
```

### 4.2 ToolCallResult 結構

**檔案**: `crates/goose/src/agents/tool_execution.rs` (L18-L30)

```rust
pub struct ToolCallResult {
    pub result: Box<dyn Future<Output = ToolResult<CallToolResult>> + Send + Unpin>,
    pub notification_stream: Option<Box<dyn Stream<Item = ServerNotification> + Send + Unpin>>,
}

impl From<ToolResult<CallToolResult>> for ToolCallResult {
    fn from(result: ToolResult<CallToolResult>) -> Self {
        Self {
            result: Box::new(futures::future::ready(result)),
            notification_stream: None,
        }
    }
}
```

### 4.3 資料流圖

```
ExtensionManager.dispatch_tool_call()
    │
    ├─ client.subscribe() → mpsc::Receiver<ServerNotification>
    ├─ client.call_tool() → impl Future<Output = ToolResult<CallToolResult>>
    │
    ▼
tool_stream(notification_rx, call_future) → ToolStream
    │
    │  ┌── tokio::select! ──┐
    │  │                     │
    │  ▼                     ▼
    │  Notification?         Result?
    │  yield Message(n)      yield Result(r)
    │  │                     │ break
    │  └──── loop ───────────┘
    │
    ▼
stream::select_all(tool_futures)  ← 多工具並行
    │
    ├─ (req_id, ToolStreamItem::Message) → yield McpNotification
    └─ (req_id, ToolStreamItem::Result) → 收集到 response_msg
```

### 4.4 設計評價

這是 Goose 最優雅的設計之一。關鍵特性:

1. **零拷貝合併**: notification stream 和 result future 不需要分別管理
2. **背壓自然**: `tokio::select!` 確保 notification 在 result 前被消費
3. **可組合**: `stream::select_all()` 讓多工具的 ToolStream 自然合併
4. **生命週期安全**: `'static` bound 確保 stream 可以跨 await point

---

## 5. Inspector Pipeline -- 三層安全檢查

### 5.1 ToolInspector Trait

**檔案**: `crates/goose/src/tool_inspection.rs` (L33-L54)

```rust
#[async_trait]
pub trait ToolInspector: Send + Sync {
    fn name(&self) -> &'static str;

    async fn inspect(
        &self,
        session_id: &str,
        tool_requests: &[ToolRequest],
        messages: &[Message],
        goose_mode: GooseMode,
    ) -> Result<Vec<InspectionResult>>;

    fn is_enabled(&self) -> bool { true }

    fn as_any(&self) -> &dyn std::any::Any;  // 向下轉型支援
}
```

### 5.2 InspectionResult 與 InspectionAction

```rust
pub struct InspectionResult {
    pub tool_request_id: String,
    pub action: InspectionAction,
    pub reason: String,
    pub confidence: f32,           // 0.0 ~ 1.0
    pub inspector_name: String,
    pub finding_id: Option<String>, // "SEC-{uuid}", "REP-001"
}

pub enum InspectionAction {
    Allow,
    Deny,
    RequireApproval(Option<String>),  // 附帶警告訊息
}
```

### 5.3 ToolInspectionManager -- 管道協調器

**檔案**: `crates/goose/src/tool_inspection.rs` (L57-L160)

```rust
pub struct ToolInspectionManager {
    inspectors: Vec<Box<dyn ToolInspector>>,
}

impl ToolInspectionManager {
    pub async fn inspect_tools(&self, ...) -> Result<Vec<InspectionResult>> {
        let mut all_results = Vec::new();
        for inspector in &self.inspectors {
            if !inspector.is_enabled() { continue; }
            match inspector.inspect(session_id, tool_requests, messages, goose_mode).await {
                Ok(results) => all_results.extend(results),
                Err(e) => {
                    tracing::error!(...);
                    // 繼續 — 單個 inspector 失敗不影響其他
                }
            }
        }
        Ok(all_results)
    }
}
```

### 5.4 三層 Inspector 的建立順序

**檔案**: `crates/goose/src/agents/agent.rs` (L257-L276)

```rust
fn create_tool_inspection_manager(
    permission_manager: Arc<PermissionManager>,
    provider: SharedProvider,
) -> ToolInspectionManager {
    let mut manager = ToolInspectionManager::new();

    // 第 1 層: SecurityInspector (最高優先級)
    manager.add_inspector(Box::new(SecurityInspector::new()));

    // 第 2 層: PermissionInspector (中高優先級)
    manager.add_inspector(Box::new(PermissionInspector::new(
        permission_manager, provider,
    )));

    // 第 3 層: RepetitionInspector (低優先級)
    manager.add_inspector(Box::new(RepetitionInspector::new(None)));

    manager
}
```

### 5.5 SecurityInspector 實作

**檔案**: `crates/goose/src/security/`

- **PromptInjectionScanner**: 正規表達式模式匹配 + 可選 ML 分類器
- **Unicode Tag 攻擊偵測**: 掃描 U+E0000-U+E007F 範圍
- **信心閾值**: 超過閾值的威脅強制用戶確認
- **OTel 指標追蹤**: 所有結果都有 `monotonic_counter.goose.prompt_injection_*`

### 5.6 RepetitionInspector 實作

**檔案**: `crates/goose/src/tool_monitor.rs` (L34-L135)

```rust
pub struct RepetitionInspector {
    max_repetitions: Option<u32>,
    last_call: Option<InternalToolCall>,
    repeat_count: u32,
    call_counts: HashMap<String, u32>,
}

impl RepetitionInspector {
    pub fn check_tool_call(&mut self, tool_call: CallToolRequestParams) -> bool {
        let internal_call = InternalToolCall::from_tool_call(&tool_call);
        // 比較工具名稱 + 完整參數 JSON
        if let Some(last) = &self.last_call {
            if last.matches(&internal_call) {
                self.repeat_count += 1;
                if self.repeat_count > self.max_repetitions.unwrap() {
                    return false;  // 超過重複上限
                }
            } else {
                self.repeat_count = 1;
            }
        }
        self.last_call = Some(internal_call);
        true
    }
}
```

**精確匹配策略**: 不只比較工具名稱，還比較完整的參數 JSON (`Value::eq`)。這代表相同工具但不同參數不算重複。

### 5.7 結果合併邏輯

**檔案**: `crates/goose/src/tool_inspection.rs` (L170-L250)

```rust
pub fn apply_inspection_results_to_permissions(
    mut permission_result: PermissionCheckResult,
    inspection_results: &[InspectionResult],
) -> PermissionCheckResult {
    for result in inspection_results {
        match result.action {
            InspectionAction::Deny => {
                // 從 approved 移除，加入 denied
                permission_result.approved.retain(|req| req.id != *request_id);
                permission_result.needs_approval.retain(|req| req.id != *request_id);
                permission_result.denied.push(request.clone());
            }
            InspectionAction::RequireApproval(_) => {
                // 從 approved 移除，加入 needs_approval
                permission_result.approved.retain(|req| req.id != *request_id);
                permission_result.needs_approval.push(request.clone());
            }
            InspectionAction::Allow => {
                // 不覆蓋其他 inspector 的決定
            }
        }
    }
    permission_result
}
```

**關鍵設計**: `Allow` 不會覆蓋其他 inspector 的 `Deny` 或 `RequireApproval`。這是一個「最嚴格優先」的合併策略。

### 5.8 Pipeline 資料流

```
tool_requests: [ToolRequest]
    │
    ▼
SecurityInspector.inspect()
    │  ├─ 掃描 tool arguments 中的 prompt injection
    │  └─ 產生 InspectionResult { finding_id: "SEC-xxx" }
    │
    ▼
PermissionInspector.inspect()
    │  ├─ 檢查 tool annotations (readOnlyHint, destructiveHint)
    │  ├─ 查詢 PermissionManager (AlwaysAllow / NeverAllow)
    │  └─ GooseMode::SmartApprove → read-only 自動允許
    │
    ▼
RepetitionInspector.inspect()
    │  ├─ 比較 (tool_name, arguments) 與上次呼叫
    │  └─ 超過 max_repetitions → Deny { finding_id: "REP-001" }
    │
    ▼
apply_inspection_results_to_permissions()
    │
    ▼
PermissionCheckResult {
    approved: Vec<ToolRequest>,      → 直接執行
    needs_approval: Vec<ToolRequest>, → 送 UI 確認
    denied: Vec<ToolRequest>,         → 返回 DECLINED_RESPONSE
}
```

---

## 6. Extension Manager -- 5 種 MCP Extension 的生命週期

### 6.1 ExtensionConfig Enum

**檔案**: `crates/goose/src/agents/extension.rs`

```rust
pub enum ExtensionConfig {
    Stdio {
        cmd: String,
        args: Vec<String>,
        envs: Envs,
        env_keys: Vec<String>,    // Keychain secret keys
        timeout: Option<u64>,
        name: String,
        description: String,
        bundled: Option<bool>,
        available_tools: Vec<String>,
    },
    StreamableHttp {              // 取代舊的 SSE
        uri: String,
        timeout: Option<u64>,
        headers: HashMap<String, String>,
        name: String,
        envs: Envs,
        env_keys: Vec<String>,
        description: String,
        bundled: Option<bool>,
        available_tools: Vec<String>,
    },
    Builtin {                     // 內建 MCP server (developer, memory)
        name: String,
        timeout: Option<u64>,
        description: String,
        display_name: Option<String>,
        bundled: Option<bool>,
        available_tools: Vec<String>,
    },
    Frontend {                    // 前端工具 (UI 直接執行)
        name: String,
        tools: Vec<Tool>,
        instructions: Option<String>,
    },
    Platform {                    // 平台工具 (analyze, summon, todo)
        name: String,
        description: String,
        display_name: Option<String>,
        bundled: Option<bool>,
        available_tools: Vec<String>,
    },
    InlinePython {                // 內嵌 Python 腳本
        name: String,
        code: String,
        description: String,
        timeout: Option<u64>,
        dependencies: Option<Vec<String>>,
    },
    Sse { .. },                   // 已棄用
}
```

### 6.2 Extension 生命週期圖

```
add_extension(config, working_dir, container, session_id)
    │
    ├─ 1. config.resolve(Config::global()) — 解析 env_keys 從 keychain
    │
    ├─ 2. 比較 config 與 resolved_config — 偵測 secret 輪換
    │     └─ 如果相同 → 跳過 (已載入)
    │
    ├─ 3. 依類型建立 MCP 客戶端:
    │     │
    │     ├─ Stdio → child_process_client()
    │     │     ├─ configure_subprocess() — 設定子行程
    │     │     ├─ TokioChildProcess::builder(command).stderr(piped).spawn()
    │     │     ├─ McpClient::connect_with_container(transport, timeout, ...)
    │     │     └─ 如果連線失敗 → 收集 stderr → ProcessExit error
    │     │
    │     ├─ StreamableHttp → create_streamable_http_client()
    │     │     ├─ reqwest::Client with custom headers
    │     │     ├─ StreamableHttpClientTransport
    │     │     ├─ McpClient::connect()
    │     │     └─ 如果 AuthRequired → oauth_flow() → 帶 auth 重連
    │     │
    │     ├─ Builtin → get_builtin_extension() / 或 Docker 內執行
    │     │     ├─ tokio::io::duplex(65536) × 2 — 雙向管道
    │     │     ├─ extension_fn(server_read, server_write) — 啟動 MCP server
    │     │     └─ McpClient::connect((client_read, client_write), ...)
    │     │
    │     ├─ Platform → PLATFORM_EXTENSIONS[name].client_factory(context)
    │     │     └─ 進程內客戶端，直接實作 McpClientTrait
    │     │
    │     └─ Frontend → 存入 frontend_tools map (不建立 MCP 連線)
    │
    ├─ 4. extension_malware_check() — 惡意軟體掃描
    │
    ├─ 5. 取得 server_info (capabilities, instructions)
    │
    ├─ 6. 存入 extensions: HashMap<String, Extension>
    │
    ├─ 7. 清除 tools_cache, 遞增 tools_cache_version
    │
    └─ 8. persist_extension_state(session_id) — 持久化到 session
```

### 6.3 環境變數安全

**檔案**: `crates/goose/src/agents/extension.rs`

```rust
impl Envs {
    const DISALLOWED_KEYS: [&'static str; 31] = [
        "PATH", "PATHEXT", "SystemRoot",       // 二進制路徑
        "LD_LIBRARY_PATH", "LD_PRELOAD",       // 動態連結器劫持
        "DYLD_INSERT_LIBRARIES",               // macOS 注入
        "PYTHONPATH", "NODE_OPTIONS",          // 語言環境劫持
        "APPINIT_DLLS", "ComSpec",             // Windows DLL/命令注入
        "HOME", "USERPROFILE",                 // 使用者目錄劫持
        "XDG_CONFIG_HOME", "XDG_DATA_HOME",    // Linux config 劫持
        "RUBYLIB", "PERL5LIB",                // Ruby/Perl 注入
        "CLASSPATH",                           // Java classpath 注入
        // ... 共 31 個
    ];
}
```

### 6.4 工具名稱前綴機制

ExtensionManager 為每個 extension 的工具添加前綴:
- `developer__shell` — developer extension 的 shell 工具
- `memory__store` — memory extension 的 store 工具

但 Platform extensions (analyze, summon) 標記為 `unprefixed_tools = true`，工具名稱不加前綴。

### 6.5 dispatch_tool_call 路由

**檔案**: `crates/goose/src/agents/extension_manager.rs` (L1386-L1416)

```rust
pub async fn dispatch_tool_call(&self, ...) -> Result<ToolCallResult> {
    let resolved = self.resolve_tool(session_id, &tool_name_str).await?;

    // 檢查工具是否在 available_tools 白名單中
    if !extension.config.is_tool_available(&resolved.actual_tool_name) {
        return Err(...);
    }

    let client = resolved.client.clone();
    let notifications_receiver = client.subscribe().await;

    // 異步 Future: 實際呼叫 MCP tool
    let result_future = async move {
        let result = client.call_tool(session_id, &actual_tool_name, arguments, ...).await;
        match result {
            Ok(result) => Ok(result),
            Err(e) => Err(ErrorData::from(e)),
        }
    };

    // 組合 notification stream + result future
    Ok(ToolCallResult {
        result: Box::new(result_future),
        notification_stream: Some(Box::new(ReceiverStream::new(notifications_receiver))),
    })
}
```

---

## 7. Recipe 系統 -- YAML + MiniJinja Template

### 7.1 Recipe 結構

**檔案**: `crates/goose/src/recipe/mod.rs` (L42-L86)

```rust
pub struct Recipe {
    pub version: String,                          // "1.0.0"
    pub title: String,
    pub description: String,
    pub instructions: Option<String>,             // system prompt
    pub prompt: Option<String>,                   // user 啟動提示
    pub extensions: Option<Vec<ExtensionConfig>>, // 需要的工具
    pub settings: Option<Settings>,               // provider/model/temperature
    pub activities: Option<Vec<String>>,           // UI 活動提示
    pub author: Option<Author>,
    pub parameters: Option<Vec<RecipeParameter>>, // 參數化輸入
    pub response: Option<Response>,               // JSON schema 結構化輸出
    pub sub_recipes: Option<Vec<SubRecipe>>,       // 子 recipe
    pub retry: Option<RetryConfig>,               // 重試設定
}
```

### 7.2 RecipeParameter 類型系統

```rust
pub enum RecipeParameterInputType {
    String,
    Number,
    Boolean,
    Date,
    File,      // 從檔案路徑匯入內容 (禁止 default 防止讀取敏感檔案)
    Select,    // 下拉選項
}

pub enum RecipeParameterRequirement {
    Required,
    Optional,
    UserPrompt,  // 互動式提示使用者輸入
}
```

### 7.3 MiniJinja 範本引擎

**檔案**: `crates/goose/src/recipe/template_recipe.rs` (L92-L115)

```rust
pub fn render_recipe_content_with_params(
    content: &str,
    params: &HashMap<String, String>,
) -> Result<String> {
    // 1. 處理空雙引號 ("" → '')
    let content_with_empty_quotes_replaced = re.replace_all(content, ": ''");

    // 2. 預處理不可解析的範本變數 → {% raw %}
    let content_with_safe_variables =
        preprocess_template_variables(&content_with_empty_quotes_replaced)?;

    // 3. 設定 MiniJinja 環境
    let env = add_template_in_env(
        &content_with_safe_variables,
        params.get("recipe_dir").cloned(),
        UndefinedBehavior::Strict,  // 未定義變數 → 錯誤
    )?;

    // 4. 渲染範本
    let template = env.get_template("recipe").unwrap();
    template.render(params)?
}
```

### 7.4 範本繼承支援

```rust
fn uses_template_inheritance(content: &str) -> bool {
    let re = Regex::new(r"\{%-?\s*(extends|include)").unwrap();
    re.is_match(content)
}
```

Recipe 支援 MiniJinja 的 `{% extends %}` 和 `{% include %}`，允許 recipe 繼承和組合。

### 7.5 SubRecipe 巢套機制

```rust
pub struct SubRecipe {
    pub name: String,
    pub path: String,                               // recipe 檔案路徑
    pub values: Option<HashMap<String, String>>,     // 參數值覆寫
    pub sequential_when_repeated: bool,              // 重複時是否順序執行
    pub description: Option<String>,
}
```

載入時自動確保必要 extension:
- 有 `developer` builtin → 自動注入 `analyze` platform
- 有 `sub_recipes` → 自動注入 `summon` platform

### 7.6 Recipe YAML 範例 (完整格式)

```yaml
version: 1.0.0
title: "Release Change Risk Check"
description: "Create a report to assess change risk"

instructions: |
  ## Step 1: Generate the heuristic report
  Run the script: {{recipe_dir}}/release_risk_report.py --version {{version}}
  ## Step 2: AI review of MEDIUM and HIGH risk PRs
  {% if test_phases == "all" or "basic" in test_phases %}
  ## Phase 1: Basic Tests
  ...
  {% endif %}

parameters:
  - key: "version"
    input_type: string
    requirement: required
    description: "release version"
  - key: "test_phases"
    input_type: select
    requirement: optional
    options: ["all", "basic", "integration"]
    default: "all"

extensions:
  - type: platform
    name: developer
  - type: stdio
    name: github
    cmd: npx
    args: ["-y", "@modelcontextprotocol/server-github"]
    env_keys: [GITHUB_TOKEN]

settings:
  goose_provider: anthropic
  goose_model: claude-sonnet-4-20250514
  temperature: 0.3
  max_turns: 50

response:
  json_schema:
    type: object
    properties:
      risk_level: { type: string, enum: [LOW, MEDIUM, HIGH] }
      summary: { type: string }
    required: [risk_level, summary]

retry:
  max_retries: 3
  checks:
    - type: shell
      command: "python check_report.py"
  on_failure: "rm -rf output/"
  timeout_seconds: 120

sub_recipes:
  - name: code_review
    path: ./code_review.yaml
    values:
      focus: security

prompt: follow the instructions to generate the final report
```

### 7.7 Recipe 安全檢查

```rust
pub fn check_for_security_warnings(&self) -> bool {
    // 偵測隱藏 Unicode tag (U+E0000-U+E007F)
    [self.instructions.as_deref(), self.prompt.as_deref()]
        .iter()
        .flatten()
        .any(|&field| contains_unicode_tags(field))
}
```

### 7.8 Recipe vs Clawtex Hand TOML 對比

| 維度 | Goose Recipe | Clawtex Hand |
|------|-------------|-------------|
| 格式 | YAML/JSON | TOML |
| 範本引擎 | MiniJinja (完整 Jinja2) | 無 |
| 參數化 | RecipeParameter (6 type + 3 requirement) | `[settings]` HashMap<String, String> |
| 條件邏輯 | `{% if %}` Jinja2 條件 | 無 |
| 繼承 | `{% extends %}` / `{% include %}` | 無 |
| 結構化輸出 | FinalOutputTool + JSON Schema | 無 |
| 子任務 | SubRecipe (扁平列表) | Multi-phase (sequential) |
| 重試 | RetryConfig + shell checks + on_failure | 無 |
| 多階段 | 無 (單一 prompt) | 有 (多 phase + condition gates) |
| 工具定義 | 引用 extension | phase.tools = ["shell", "file_write"] |
| Provider 指定 | settings.goose_provider/model | 繼承全域 |

---

## 8. Lead-Worker 雙模型策略

### 8.1 核心結構

**檔案**: `crates/goose/src/providers/lead_worker.rs` (L22-L85)

```rust
pub struct LeadWorkerProvider {
    lead_provider: Arc<dyn Provider>,      // 強力模型 (前 N 輪)
    worker_provider: Arc<dyn Provider>,    // 快速模型 (後續輪次)
    lead_turns: usize,                     // 預設 3 輪
    turn_count: Arc<Mutex<usize>>,
    failure_count: Arc<Mutex<usize>>,
    max_failures_before_fallback: usize,   // 連續失敗 2 次回退
    fallback_turns: usize,                 // 回退使用 lead 模型 2 輪
    in_fallback_mode: Arc<Mutex<bool>>,
    fallback_remaining: Arc<Mutex<usize>>,
}
```

### 8.2 狀態機

```
                     ┌─────────────────┐
                     │  LEAD PHASE     │
                     │  turn < 3       │
                     └────────┬────────┘
                              │ turn >= lead_turns
                              ▼
                     ┌─────────────────┐
                     │  WORKER PHASE   │◄────────────────┐
                     │  turn >= 3      │                  │
                     └────────┬────────┘                  │
                              │ 2 consecutive             │
                              │ task failures             │ fallback_remaining == 0
                              ▼                           │
                     ┌─────────────────┐                  │
                     │  FALLBACK MODE  │──────────────────┘
                     │  use lead for   │  (2 successful turns)
                     │  2 turns        │
                     └─────────────────┘
```

### 8.3 失敗偵測機制

**檔案**: `crates/goose/src/providers/lead_worker.rs` (L204-L293)

```rust
async fn detect_task_failures(&self, message: &Message) -> bool {
    let mut failure_indicators = 0;

    for content in &message.content {
        match content {
            MessageContent::ToolRequest(req) => {
                if req.tool_call.is_err() { failure_indicators += 1; }
            }
            MessageContent::ToolResponse(resp) => {
                if resp.tool_result.is_err() { failure_indicators += 1; }
                // 檢查輸出中的錯誤模式
                if self.contains_error_indicators(&result.content) {
                    failure_indicators += 1;
                }
            }
            MessageContent::Text(text) => {
                // 偵測使用者修正模式
                if self.contains_user_correction_patterns(&text.text) {
                    failure_indicators += 1;
                }
            }
            _ => {}
        }
    }
    failure_indicators >= 1
}
```

**工具輸出錯誤模式** (硬編碼):
```rust
fn contains_error_indicators(&self, contents: &[Content]) -> bool {
    // "error:", "failed:", "exception:", "traceback",
    // "syntax error", "permission denied", "file not found",
    // "command not found", "compilation failed", "test failed",
    // "assertion failed"
}
```

**使用者修正模式**:
```rust
fn contains_user_correction_patterns(&self, text: &str) -> bool {
    // "that's wrong", "try again", "actually, ", "fix this",
    // "this is broken", starts_with("no,"), starts_with("wrong")
}
```

### 8.4 技術失敗 vs 任務失敗

```
Technical Failure (API/LLM 錯誤):
  ├─ 不增加 failure_count
  ├─ 不增加 turn_count
  └─ 自動 retry 用 lead provider

Task Failure (工具錯誤/使用者修正):
  ├─ failure_count += 1
  ├─ turn_count += 1
  └─ 如果 failure_count >= 2 → 進入 FALLBACK MODE
```

---

## 9. Subagent Handler -- 子代理隔離與錯誤冒泡

### 9.1 SubagentRunParams

**檔案**: `crates/goose/src/agents/subagent_handler.rs` (L37-L46)

```rust
pub struct SubagentRunParams {
    pub config: AgentConfig,
    pub recipe: Recipe,
    pub task_config: TaskConfig,
    pub return_last_only: bool,           // 只返回最後一條消息
    pub session_id: String,
    pub cancellation_token: Option<CancellationToken>,
    pub on_message: Option<OnMessageCallback>,          // 每條消息回調
    pub notification_tx: Option<UnboundedSender<ServerNotification>>, // 通知管道
}
```

### 9.2 子代理執行流程

**檔案**: `crates/goose/src/agents/subagent_handler.rs` (L122-L226)

```
get_agent_messages(params)
    │
    ├─ 1. 建立新 Agent::with_config(config)
    │
    ├─ 2. agent.update_provider(task_config.provider)
    │
    ├─ 3. 逐一載入 extensions
    │     └─ 失敗 → debug log, 繼續
    │
    ├─ 4. apply_recipe_components(recipe.response) → FinalOutputTool
    │
    ├─ 5. build_subagent_prompt()
    │     ├─ 列出可用工具
    │     └─ render_template("subagent_system.md", SubagentPromptContext)
    │
    ├─ 6. agent.override_system_prompt(subagent_prompt)
    │
    ├─ 7. agent.reply(user_message, session_config, cancel_token)
    │
    ├─ 8. 消費 reply stream:
    │     ├─ AgentEvent::Message(msg) → on_message callback
    │     │                           → create_tool_notification() → notification_tx
    │     │                           → conversation.push(msg)
    │     ├─ AgentEvent::McpNotification → 忽略
    │     ├─ AgentEvent::ModelChange → 忽略
    │     ├─ AgentEvent::HistoryReplaced → 替換 conversation
    │     └─ Error → break
    │
    └─ 9. 提取結果:
          ├─ has_response_schema → final_output_tool.final_output
          └─ 否則 → extract_response_text(messages, return_last_only)
```

### 9.3 SubagentPromptContext

```rust
pub struct SubagentPromptContext {
    pub max_turns: usize,
    pub subagent_id: String,
    pub task_instructions: String,
    pub tool_count: usize,
    pub available_tools: String,  // "developer__shell, developer__edit, ..."
}
```

### 9.4 通知事件轉發

```rust
pub fn create_tool_notification(
    content: &MessageContent,
    subagent_id: &str,
) -> Option<ServerNotification> {
    if let MessageContent::ToolRequest(req) = content {
        let tool_call = req.tool_call.as_ref().ok()?;
        Some(ServerNotification::LoggingMessageNotification(
            Notification::new(
                LoggingMessageNotificationParam::new(
                    LoggingLevel::Info,
                    json!({
                        "type": "subagent_tool_request",
                        "subagent_id": subagent_id,
                        "tool_call": {
                            "name": tool_call.name,
                            "arguments": tool_call.arguments
                        }
                    }),
                ).with_logger(format!("subagent:{}", subagent_id)),
            ),
        ))
    } else { None }
}
```

### 9.5 巢狀代理防護

子代理明確**禁止**使用 `delegate` 工具 — 在 Summon 的 `handle_delegate` 中檢查 `SessionType::SubAgent`:

```rust
// 在 summon.rs 中:
if session_type == SessionType::SubAgent {
    return Err("Subagents cannot delegate to other subagents");
}
```

### 9.6 Summon 的 load vs delegate

**檔案**: `crates/goose/src/agents/platform_extensions/summon.rs`

| 操作 | 行為 | 適用場景 |
|------|------|---------|
| `load` | 將知識注入當前 context | 讀取 skill/recipe 內容、發現可用資源 |
| `delegate` (sync) | 建立子代理，阻塞等待結果 | 需要隔離執行環境的任務 |
| `delegate` (async) | 建立子代理，背景執行 | 長時間任務，透過 MOIM 監控 |

`DelegateParams`:
```rust
pub struct DelegateParams {
    pub instructions: Option<String>,     // 任務指令
    pub source: Option<String>,           // recipe/skill 名稱
    pub parameters: Option<HashMap<String, Value>>, // 參數
    pub extensions: Option<Vec<String>>,  // 額外 extensions
    pub provider: Option<String>,         // 指定 provider
    pub model: Option<String>,            // 指定 model
    pub temperature: Option<f32>,
    pub r#async: bool,                    // 同步/異步模式
}
```

---

## 10. MOIM -- 背景任務狀態注入

### 10.1 注入邏輯

**檔案**: `crates/goose/src/agents/moim.rs` (L12-L48)

```rust
pub async fn inject_moim(
    session_id: &str,
    conversation: Conversation,
    extension_manager: &ExtensionManager,
    working_dir: &Path,
) -> Conversation {
    if let Some(moim) = extension_manager.collect_moim(session_id, working_dir).await {
        let mut messages = conversation.messages().clone();
        // 插入到最後一條 Assistant 消息之前
        let idx = messages.iter()
            .rposition(|m| m.role == Role::Assistant)
            .unwrap_or(0);
        messages.insert(idx, Message::user().with_text(moim));

        // 修復合併 → 如果產生非預期問題則回退
        let (fixed, issues) = fix_conversation(Conversation::new_unvalidated(messages));
        let has_unexpected_issues = issues.iter().any(|issue| {
            !issue.contains("Merged consecutive")
        });
        if has_unexpected_issues {
            return conversation; // 回退到原始
        }
        return fixed;
    }
    conversation
}
```

### 10.2 MOIM 收集

**檔案**: `crates/goose/src/agents/extension_manager.rs` (L1617-L1650)

```rust
pub async fn collect_moim(&self, session_id: &str, working_dir: &Path) -> Option<String> {
    // 小 context 模型跳過 MOIM (< 32K)
    const MIN_CONTEXT_FOR_MOIM: usize = 32_000;
    if provider.get_model_config().context_limit() < MIN_CONTEXT_FOR_MOIM {
        return None;
    }

    // 分鐘級粒度 (避免每秒改變)
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:00").to_string();
    let mut content = format!(
        "<info-msg>\nIt is currently {}\nWorking directory: {}\n",
        timestamp, working_dir.display()
    );

    // 注入 token 使用情況
    if let (Some(total), Some(config)) = (session.total_tokens, session.model_config.as_ref()) {
        content.push_str(&format!(
            "Token usage: {} / {} ({:.0}%)\n",
            total, config.context_limit, ...
        ));
    }

    // 收集各 extension 的 MOIM 資料
    for ext in extensions {
        if let Some(moim) = ext.client.get_moim(session_id).await {
            content.push_str(&moim);
        }
    }

    content.push_str("</info-msg>");
    Some(content)
}
```

### 10.3 MOIM 輸出範例

```
<info-msg>
It is currently 2026-03-13 14:30:00
Working directory: /home/user/project
Token usage: 45000 / 200000 (22%)

[Background Task: code_review (id: abc123)]
Status: running
Duration: 2m 15s
Last activity: 30s ago
Turns completed: 5

[Background Task: test_suite (id: def456)]
Status: completed
Result: All tests passed (42/42)
</info-msg>
```

---

## 11. 上下文管理 -- 壓縮與 Token 計數

### 11.1 自動壓縮觸發

**檔案**: `crates/goose/src/context_mgmt/mod.rs` (L182-L223)

```rust
pub async fn check_if_compaction_needed(
    provider: &dyn Provider,
    conversation: &Conversation,
    threshold_override: Option<f64>,
    session: &Session,
) -> Result<bool> {
    let threshold = threshold_override.unwrap_or(0.8); // 預設 80%
    let context_limit = provider.get_model_config().context_limit();

    // 優先使用 session metadata 中的 total_tokens
    let current_tokens = match session.total_tokens {
        Some(tokens) => tokens as usize,
        None => {
            // 否則即時計算 (tiktoken-rs)
            let token_counter = create_token_counter().await?;
            messages.iter()
                .filter(|m| m.is_agent_visible())
                .map(|msg| token_counter.count_chat_tokens("", &[msg], &[]))
                .sum()
        }
    };

    let usage_ratio = current_tokens as f64 / context_limit as f64;

    // threshold <= 0 或 >= 1 → 停用自動壓縮
    if threshold <= 0.0 || threshold >= 1.0 { false }
    else { usage_ratio > threshold }
}
```

### 11.2 漸進式壓縮

**檔案**: `crates/goose/src/context_mgmt/mod.rs` (L275-L340)

```rust
async fn do_compact(provider, session_id, messages) -> Result<(Message, ProviderUsage)> {
    let agent_visible_messages: Vec<Message> = messages.iter()
        .filter(|msg| msg.is_agent_visible())
        .map(|msg| msg.agent_visible_content())
        .collect();

    // 漸進式移除 tool responses — 從中間向外
    let removal_percentages = [0, 10, 20, 50, 100];

    for &remove_percent in &removal_percentages {
        let filtered_messages = filter_tool_responses(&agent_visible_messages, remove_percent);

        let messages_text = filtered_messages.iter()
            .map(|msg| format_message_for_compacting(msg))
            .collect::<Vec<_>>()
            .join("\n");

        let system_prompt = render_template("compaction.md", &SummarizeContext { messages: messages_text })?;

        match provider.complete_fast(session_id, &system_prompt, &[user_message], &[]).await {
            Ok((response, usage)) => return Ok((response, usage)),
            Err(ProviderError::ContextLengthExceeded(_)) => {
                if attempt < removal_percentages.len() - 1 { continue; }
                else { return Err(...); }
            }
            Err(e) => return Err(e.into()),
        }
    }
}
```

### 11.3 中間外移除演算法

```rust
fn filter_tool_responses(messages: &[Message], remove_percent: u32) -> Vec<&Message> {
    let tool_indices: Vec<usize> = /* 收集所有 tool response 的索引 */;
    let num_to_remove = (tool_indices.len() * remove_percent / 100).max(1);
    let middle = tool_indices.len() / 2;

    // Middle-out: 交替從中間向左右移除
    for i in 0..num_to_remove {
        if i % 2 == 0 {
            indices_to_remove.push(tool_indices[middle - i/2 - 1]);
        } else {
            indices_to_remove.push(tool_indices[middle + i/2]);
        }
    }
    // 過濾掉被移除的消息
}
```

**設計理由**: 中間的 tool 響應通常是過渡性的，首尾的更有可能包含關鍵上下文。

### 11.4 Message Visibility 系統

```rust
pub struct MessageMetadata {
    pub agent_visible: bool,   // Agent 推論時能看到
    pub user_visible: bool,    // 使用者 UI 能看到
}

impl MessageMetadata {
    pub fn agent_only() -> Self { Self { agent_visible: true, user_visible: false } }
    pub fn invisible() -> Self { Self { agent_visible: false, user_visible: false } }
    pub fn with_agent_invisible(&self) -> Self { Self { agent_visible: false, ..*self } }
}
```

壓縮後:
- 原始消息 → `agent_visible: false, user_visible: true` (使用者可回溯)
- 摘要消息 → `agent_visible: true, user_visible: false` (agent 看摘要)
- 續接提示 → `agent_visible: true, user_visible: false`

### 11.5 續接提示

```rust
const TOOL_LOOP_CONTINUATION_TEXT: &str =
    "Your context was compacted. The previous message contains a summary...
    Continue calling tools as necessary to complete the task.";

const CONVERSATION_CONTINUATION_TEXT: &str =
    "Your context was compacted. The previous message contains a summary...
    Just continue the conversation naturally based on the summarized context.";

const MANUAL_COMPACT_CONTINUATION_TEXT: &str =
    "Your context was compacted at the user's request...
    Just continue the conversation naturally based on the summarized context.";
```

### 11.6 工具對摘要 (已停用)

```rust
const ENABLE_TOOL_PAIR_SUMMARIZATION: bool = false;
// TODO: Re-enable once tool summarization stability issues are resolved.

pub fn tool_id_to_summarize(conversation: &Conversation, cutoff: usize) -> Option<String> {
    // 超過 cutoff (預設 10) 的工具呼叫 → 找到最早的進行摘要
}

pub async fn summarize_tool_call(provider, session_id, conversation, tool_id) -> Result<Message> {
    // 使用 complete_fast 摘要單個 tool call/response pair
    // "A call to github was made to get the project status"
}
```

---

## 12. Provider 抽象與 Canonical Registry

### 12.1 Provider Trait

**檔案**: `crates/goose/src/providers/base.rs`

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn get_name(&self) -> &str;
    fn get_model_config(&self) -> ModelConfig;

    async fn stream(
        &self,
        model_config: &ModelConfig,
        session_id: &str,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError>;

    async fn complete(...) -> Result<(Message, ProviderUsage), ProviderError> {
        collect_stream(self.stream(...).await?).await
    }

    async fn complete_fast(...) -> Result<(Message, ProviderUsage), ProviderError> {
        // 先試 fast_model，失敗 fallback 到主模型
    }

    fn as_lead_worker(&self) -> Option<&dyn LeadWorkerProviderTrait> { None }
    fn permission_routing(&self) -> PermissionRouting { ... }
    fn supports_embeddings(&self) -> bool { false }
}
```

### 12.2 已支援的 Providers (30+)

| 類別 | Provider | 說明 |
|------|----------|------|
| **Cloud API** | Anthropic, OpenAI, Google, OpenRouter, xAI | 主流雲端 API |
| **Cloud Enterprise** | Azure, Bedrock, GCP Vertex, Databricks, Snowflake, SageMaker | 企業部署 |
| **Declarative** | Groq, DeepSeek, Mistral, Cerebras, Inception, Kimi, LM Studio, Moonshot, OVH | JSON 宣告式 |
| **Agent Wrapper** | ClaudeCode, Codex, ChatGPT Codex, CursorAgent, Gemini CLI, GitHub Copilot | 包裝其他 coding agents |
| **Meta** | LeadWorker, ToolShim | 組合/適配 |
| **Local** | LocalInference (llama.cpp) | 本地 GGUF 推理 |

### 12.3 Declarative Provider 機制

**檔案**: `crates/goose/src/providers/declarative/groq.json`

```json
{
  "name": "groq",
  "display_name": "Groq",
  "description": "Groq's fast inference API",
  "api_key_env": "GROQ_API_KEY",
  "base_url": "https://api.groq.com/openai/v1",
  "default_model": "llama-3.3-70b-versatile",
  "known_models": ["llama-3.3-70b-versatile", "llama-3.1-8b-instant", ...]
}
```

新增 provider 只需添加一個 JSON 檔案 — 不需要改任何 Rust 程式碼。

### 12.4 ProviderRegistry (單例)

```rust
static REGISTRY: OnceCell<RwLock<ProviderRegistry>> = OnceCell::const_new();

async fn init_registry() -> RwLock<ProviderRegistry> {
    let mut registry = ProviderRegistry::new().with_providers(|registry| {
        registry.register::<AnthropicProvider>(true);  // preferred
        registry.register::<OpenAiProvider>(true);
        registry.register::<OllamaProvider>(true);
        // ... 25+ providers
    });
    load_custom_providers_into_registry(&mut registry)?;
    RwLock::new(registry)
}
```

### 12.5 Canonical Model Registry

**檔案**: `crates/goose/src/providers/canonical/`

標準化跨 provider 的模型元數據:
- **模型名稱映射**: 各 provider 別名 → canonical ID
- **Context limit**: 精確的輸入 token 上限
- **Output limit**: 最大輸出 token
- **Tool calling**: 是否支援原生 tool calling
- **Reasoning**: 是否 reasoning model (O1/O3)
- **Modality**: text/image/audio 輸入/輸出
- **Release date**: 模型發布日期

---

## 13. Local Inference -- llama.cpp 本地推理引擎

### 13.1 InferenceRuntime 架構

**檔案**: `crates/goose/src/providers/local_inference.rs` (L50-L110)

```rust
pub struct InferenceRuntime {
    models: StdMutex<HashMap<String, ModelSlot>>,  // 模型快取
    backend: LlamaBackend,                         // llama.cpp 後端
}

// 全域 Weak reference — 共享單一 runtime
static RUNTIME: StdMutex<Weak<InferenceRuntime>> = StdMutex::new(Weak::new());

impl InferenceRuntime {
    pub fn get_or_init() -> Arc<Self> {
        let mut guard = RUNTIME.lock().expect("runtime lock poisoned");
        if let Some(runtime) = guard.upgrade() { return runtime; }

        let backend = LlamaBackend::init().unwrap();
        let runtime = Arc::new(Self {
            models: StdMutex::new(HashMap::new()),
            backend,
        });
        *guard = Arc::downgrade(&runtime);
        runtime
    }
}
```

**欄位順序重要性**: `models` 在 `backend` 之前宣告，確保 Rust 的 drop 順序先釋放所有 GPU 資源，再呼叫 `llama_backend_free()`。

### 13.2 記憶體感知的 context 計算

**檔案**: `crates/goose/src/providers/local_inference/inference_engine.rs` (L33-L81)

```rust
pub fn estimate_max_context_for_memory(
    model: &LlamaModel,
    runtime: &InferenceRuntime,
) -> Option<usize> {
    let available = available_inference_memory_bytes(runtime);
    let usable = (available as f64 * 0.5) as u64;  // 50% for KV cache

    let n_layer = model.n_layer() as u64;
    let n_head_kv = model.n_head_kv() as u64;

    // MLA 模型 (DeepSeek) 的特殊處理
    let k_per_head = model.meta_val_str(&format!("{arch}.attention.key_length"))
        .ok().and_then(|v| v.parse().ok())
        .unwrap_or(head_dim);
    let v_per_head = model.meta_val_str(&format!("{arch}.attention.value_length"))
        .ok().and_then(|v| v.parse().ok())
        .unwrap_or(head_dim);

    let bytes_per_token = (k_per_head + v_per_head) * n_head_kv * n_layer * 2;
    Some((usable / bytes_per_token) as usize)
}
```

### 13.3 推理模式

Goose 本地推理支援兩種 tool calling 模式:

1. **Native Tools** (`inference_native_tools.rs`): 模型原生支援 tool calling schema
2. **Emulated Tools** (`inference_emulated_tools.rs`): 在 prompt 中描述 tools，解析輸出

### 13.4 生成迴圈

```rust
pub fn generation_loop(
    model, ctx, settings, prompt_token_count, effective_ctx,
    mut on_piece: impl FnMut(&str) -> Result<TokenAction, ProviderError>,
) -> Result<i32, ProviderError> {
    let mut sampler = build_sampler(settings);
    let max_output = effective_ctx.saturating_sub(prompt_token_count);
    let mut decoder = encoding_rs::UTF_8.new_decoder();

    for _ in 0..max_output {
        let token = sampler.sample(ctx, -1);
        sampler.accept(token);

        if model.is_eog_token(token) { break; }

        let piece = model.token_to_piece(token, &mut decoder, true, None)?;
        if !piece.is_empty() && matches!(on_piece(&piece)?, TokenAction::Stop) {
            break;
        }

        let mut next_batch = LlamaBatch::get_one(&[token])?;
        ctx.decode(&mut next_batch)?;
    }
}
```

### 13.5 智慧模型推薦

```rust
pub fn recommend_local_model(runtime: &InferenceRuntime) -> String {
    let available_memory = available_inference_memory_bytes(runtime);

    // 從 featured models 中選最大但能放進記憶體的
    for model in models.sorted_by_size_desc() {
        if available_memory >= model.size_bytes {
            return model.id.clone();
        }
    }
    // 都放不下 → 返回最小的
    models.last().id.clone()
}
```

---

## 14. ACP -- Agent Communication Protocol

### 14.1 概述

**檔案**: `crates/goose-acp/`

ACP 是 Goose 用於跨代理通訊的協定，建立在 `sacp` SDK 之上。它讓外部代理 (如 Claude Code, Codex) 能透過標準化介面使用 Goose 的能力。

### 14.2 ACP Server 結構

**檔案**: `crates/goose-acp/src/server.rs` (核心結構)

```rust
// ACP Server 實作了完整的代理生命週期:
// - 認證 (authenticate)
// - Session 管理 (new_session, load_session, list_sessions)
// - 消息處理 (send_message → 串流回應)
// - Tool 呼叫 (call_tool)
// - 權限管理 (permission decisions)
// - 檔案系統操作 (AcpTools)
```

關鍵依賴:
```rust
use sacp::schema::{
    AgentCapabilities, AuthMethod, AuthenticateRequest, AuthenticateResponse,
    InitializeRequest, InitializeResponse,
    NewSessionRequest, NewSessionResponse, LoadSessionRequest,
    Content, ContentBlock, ContentChunk, EmbeddedResource,
    McpCapabilities, McpServer, ModelId, ModelInfo,
    SendMessageRequest, SendMessageStreamResponse,
    ToolCallParams, ToolCallResult as AcpToolCallResult,
};
```

### 14.3 ACP 傳輸層

**檔案**: `crates/goose-acp/src/transport/`

支援兩種傳輸:
- **HTTP**: RESTful API (用於遠端通訊)
- **WebSocket**: 雙向即時通訊 (用於串流回應)

### 14.4 ACP vs MCP 對比

| 維度 | MCP | ACP |
|------|-----|-----|
| 用途 | 工具擴展 | 代理通訊 |
| 方向 | Agent → Tool | Agent ↔ Agent |
| 能力 | list_tools, call_tool, resources, prompts | sessions, messages, tools, permissions |
| 串流 | ServerNotification | SendMessageStreamResponse |
| 認證 | 環境變數/OAuth | AuthMethod (API key/OAuth/Bearer) |
| SDK | rmcp | sacp |

---

## 15. 錯誤處理策略全圖

### 15.1 錯誤類型層次

```
anyhow::Error (頂層 — 大部分 Agent 邏輯)
    │
    ├── ProviderError (結構化)
    │     ├── AuthenticationError(String)
    │     ├── RateLimitExceeded(String)
    │     ├── ContextLengthExceeded(String)
    │     ├── ExecutionError(String)
    │     ├── RequestFailed(StatusCode, String)
    │     ├── InvalidResponse(String)
    │     └── InternalError(String)
    │
    ├── ExtensionError (結構化)
    │     ├── SetupError(String)
    │     ├── ConfigError(String)
    │     ├── ConnectionError(String)
    │     └── TimeoutError(String)
    │
    ├── ErrorData (MCP 標準)
    │     ├── code: ErrorCode (INTERNAL_ERROR, INVALID_PARAMS, etc.)
    │     ├── message: String
    │     └── data: Option<Value>
    │
    └── RecipeError (結構化)
          ├── MissingParams { parameters: Vec<String> }
          └── Invalid { source: anyhow::Error }
```

### 15.2 各層錯誤處理策略

| 層級 | 策略 | 實作 |
|------|------|------|
| **Provider** | 指數退避重試 + context length 特殊處理 | `providers/retry.rs` |
| **Agent Loop** | 壓縮重試 (ContextLengthExceeded → recovery compact) | `agent.rs` L1400+ |
| **Tool Dispatch** | PostHog analytics + ErrorData 轉換 | `agent.rs` dispatch_tool_call |
| **Extension** | ProcessExit (收集 stderr) + 重連 | `extension_manager.rs` |
| **Recipe** | RetryManager + shell success checks | `retry.rs` |
| **Inspector** | 單個 inspector 失敗不影響其他 | `tool_inspection.rs` |
| **MOIM** | 注入問題 → 回退到原始 conversation | `moim.rs` |

### 15.3 Recovery Compaction

當 Agent Loop 中 provider 回傳 `ContextLengthExceeded` 時:

```
ProviderError::ContextLengthExceeded
    │
    ├─ compaction_attempts += 1
    ├─ 如果 attempts >= 3 → 放棄
    │
    ├─ compact_messages() — 漸進式壓縮
    │     └─ 漸進移除 tool responses: 0% → 10% → 20% → 50% → 100%
    │
    ├─ replace_conversation(compacted)
    ├─ yield HistoryReplaced(compacted)
    │
    └─ continue → 重試 provider 呼叫
```

---

## 16. 效能特徵與瓶頸分析

### 16.1 記憶體效能

| 組件 | 策略 | 潛在瓶頸 |
|------|------|---------|
| Token Cache | DashMap (最多 10K 條) | 穩定，有上限 |
| Extension Manager | `Mutex<HashMap<String, Extension>>` | 熱路徑上的鎖競爭 |
| ToolCallResult | `Box<dyn Future + Send>` | 動態分配 |
| Conversation | `Vec<Message>` clone | 深拷貝在每輪 loop |
| SharedProvider | `Arc<Mutex<Option<Arc<dyn Provider>>>>` | 三層包裝，每次存取需 lock |

### 16.2 CPU 效能

- **tokio::select! 在 tool_stream 中**: 每個工具呼叫一個 select! poll
- **stream::select_all**: 多工具並行時 O(n) poll
- **fix_conversation**: 每輪 loop 都驗證 conversation 格式
- **inject_moim**: 每輪 loop 都收集背景狀態

### 16.3 I/O 效能

- **MCP stdio**: 每個 extension 一個子行程 + pipe
- **MCP streamable-http**: reqwest HTTP client + SSE
- **Session SQLite**: 每條消息一次寫入
- **PostHog analytics**: 非同步 HTTP (不阻塞)

### 16.4 瓶頸識別

1. **SharedProvider 三層鎖**: `Arc<Mutex<Option<Arc<dyn Provider>>>>` 每次存取需要:
   - `lock().await` (Mutex)
   - `as_ref()` (Option)
   - `Arc::clone()` (Arc)

2. **Conversation 深拷貝**: `conversation.clone()` 在每輪 loop 中用於:
   - `maybe_summarize_tool_pair()` (背景 task)
   - `inject_moim()` (MOIM 注入)
   - 壓縮檢查

3. **Sequential Inspector Execution**: inspectors 串行執行，但它們之間沒有依賴，可以並行化。

---

## 17. Clawtex-Core 差距對比與具體實作建議

### 17.1 總覽對比表

| 維度 | Goose | Clawtex-Core | 差距等級 |
|------|-------|-------------|---------|
| Agent Loop | try_stream! + AgentEvent | run_loop + String 回傳 | 高 |
| Tool 結果 | ToolStream (Future + Stream) | String | 高 |
| 安全管道 | 3 層 Inspector Pipeline | 單一 approval gate | 高 |
| Model Registry | Canonical (JSON 元數據) | 各 provider 硬編碼 | 高 |
| 子代理 | SubagentHandler + MOIM | delegate tool (無隔離) | 中 |
| Recipe/Workflow | YAML + MiniJinja + Retry | Hand TOML (multi-phase) | 互補 |
| 雙模型 | LeadWorker (turn-based) | classifier smart_routing | 中 |
| Context 壓縮 | 漸進式 + middle-out | Light/Medium/Aggressive | 低 |
| Message Visibility | agent_visible + user_visible | 無 | 中 |
| 本地推理 | llama.cpp (GGUF) | LM Studio / Ollama (外部) | 低 |
| MCP Extension 類型 | 6 種 (Stdio/HTTP/Builtin/Platform/Frontend/InlinePython) | 1 種 (Stdio) | 中 |
| 環境變數安全 | 31 key 黑名單 | 無 | 中 |
| 錯誤類型 | ProviderError/ExtensionError/RecipeError | anyhow 為主 | 中 |

### 17.2 高優先級: ToolStream 模式

**Goose 原始碼**: `agent.rs:168-200`

**Clawtex 建議**:

```rust
// src/tool_stream.rs — 新檔案

use futures::{Stream, Future};
use std::pin::Pin;
use tokio::sync::mpsc;

/// 工具執行的進度通知
pub enum ToolNotification {
    Progress { percent: u8, message: String },
    Log { level: tracing::Level, message: String },
    Warning(String),
}

/// 工具串流項目: 通知 或 最終結果
pub enum ToolStreamItem {
    Notification(ToolNotification),
    Result(Result<String, ToolError>),
}

/// 統一的工具執行串流
pub type ToolStream = Pin<Box<dyn Stream<Item = ToolStreamItem> + Send>>;

/// 合併通知 receiver 和結果 future 為統一串流
pub fn tool_stream<S, F>(notifications: S, result: F) -> ToolStream
where
    S: Stream<Item = ToolNotification> + Send + Unpin + 'static,
    F: Future<Output = Result<String, ToolError>> + Send + 'static,
{
    Box::pin(async_stream::stream! {
        tokio::pin!(result);
        let mut notifications = notifications;
        loop {
            tokio::select! {
                Some(notif) = notifications.next() => {
                    yield ToolStreamItem::Notification(notif);
                }
                r = &mut result => {
                    yield ToolStreamItem::Result(r);
                    break;
                }
            }
        }
    })
}
```

**整合到 agent_runtime.rs**:
```rust
// 替代目前的 tool.execute() -> String
let (notif_tx, notif_rx) = mpsc::channel(32);
let result_future = tool.execute_async(args, notif_tx);
let stream = tool_stream(
    tokio_stream::wrappers::ReceiverStream::new(notif_rx),
    result_future,
);

// 消費串流
pin_mut!(stream);
while let Some(item) = stream.next().await {
    match item {
        ToolStreamItem::Notification(n) => {
            telegram_bot.send_progress(chat_id, &n).await;
        }
        ToolStreamItem::Result(r) => {
            tool_result = r;
            break;
        }
    }
}
```

---

### 17.3 高優先級: Inspector Pipeline

**Goose 原始碼**: `tool_inspection.rs:33-160`

**Clawtex 建議**:

```rust
// src/inspector.rs — 新檔案

use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct InspectionResult {
    pub tool_name: String,
    pub action: InspectionAction,
    pub reason: String,
    pub confidence: f32,
    pub inspector_name: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InspectionAction {
    Allow,
    Deny,
    RequireApproval(Option<String>),
}

#[async_trait]
pub trait ToolInspector: Send + Sync {
    fn name(&self) -> &'static str;
    async fn inspect(
        &self,
        tool_name: &str,
        arguments: &serde_json::Value,
        history: &[crate::agent_runtime::Message],
    ) -> anyhow::Result<Vec<InspectionResult>>;
}

pub struct InspectorPipeline {
    inspectors: Vec<Box<dyn ToolInspector>>,
}

impl InspectorPipeline {
    pub fn new() -> Self {
        let mut pipeline = Self { inspectors: vec![] };
        // 按優先級添加
        pipeline.add(Box::new(SecurityInspector::new()));
        pipeline.add(Box::new(ApprovalInspector::new()));
        pipeline.add(Box::new(RepetitionInspector::new(5)));
        pipeline
    }

    pub fn add(&mut self, inspector: Box<dyn ToolInspector>) {
        self.inspectors.push(inspector);
    }

    pub async fn inspect_all(
        &self,
        tool_name: &str,
        arguments: &serde_json::Value,
        history: &[Message],
    ) -> anyhow::Result<InspectionAction> {
        let mut final_action = InspectionAction::Allow;

        for inspector in &self.inspectors {
            match inspector.inspect(tool_name, arguments, history).await {
                Ok(results) => {
                    for result in results {
                        // 最嚴格優先
                        match (&final_action, &result.action) {
                            (_, InspectionAction::Deny) => {
                                final_action = InspectionAction::Deny;
                            }
                            (InspectionAction::Allow, InspectionAction::RequireApproval(msg)) => {
                                final_action = InspectionAction::RequireApproval(msg.clone());
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Inspector {} failed: {}", inspector.name(), e);
                    // 繼續 — 單個失敗不阻塞
                }
            }
        }

        Ok(final_action)
    }
}
```

**整合到現有 approval.rs**:
```rust
// 在 agent_runtime.rs 的 tool 執行前:
let action = self.inspector_pipeline.inspect_all(
    &tool_name, &arguments, &self.messages
).await?;

match action {
    InspectionAction::Allow => { /* 直接執行 */ }
    InspectionAction::RequireApproval(msg) => {
        // 走現有的 Telegram approval 流程
        self.approval_gate.request_approval(chat_id, &tool_name, msg).await?;
    }
    InspectionAction::Deny => {
        return Ok("Tool execution denied by security inspector".to_string());
    }
}
```

---

### 17.4 高優先級: Canonical Model Registry

**Goose 原始碼**: `providers/canonical/`

**Clawtex 建議**:

```rust
// src/providers/model_registry.rs — 新檔案

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub aliases: Vec<String>,
    pub context_limit: usize,
    pub output_limit: usize,
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub is_reasoning: bool,
    pub cost_per_1k_input: f64,
    pub cost_per_1k_output: f64,
}

lazy_static::lazy_static! {
    static ref MODEL_REGISTRY: HashMap<String, ModelInfo> = {
        let json = include_str!("model_registry.json");
        serde_json::from_str(json).expect("valid model registry")
    };
}

pub fn get_model_info(model_name: &str) -> Option<&ModelInfo> {
    // 先精確匹配
    if let Some(info) = MODEL_REGISTRY.get(model_name) {
        return Some(info);
    }
    // 再查別名
    MODEL_REGISTRY.values().find(|info| {
        info.aliases.iter().any(|a| a == model_name)
    })
}

pub fn context_limit_for(model_name: &str) -> usize {
    get_model_info(model_name)
        .map(|info| info.context_limit)
        .unwrap_or(4096)  // 保守預設
}
```

---

### 17.5 中優先級: Lead-Worker 雙模型

**Goose 原始碼**: `providers/lead_worker.rs`

**Clawtex 建議**:

```rust
// src/providers/lead_worker.rs — 新檔案

use crate::providers::Provider;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct LeadWorkerProvider {
    lead: Arc<dyn Provider>,
    worker: Arc<dyn Provider>,
    lead_turns: usize,
    turn_count: Arc<Mutex<usize>>,
    failure_count: Arc<Mutex<usize>>,
    in_fallback: Arc<Mutex<bool>>,
}

impl LeadWorkerProvider {
    pub fn new(
        lead: Arc<dyn Provider>,
        worker: Arc<dyn Provider>,
        lead_turns: usize,
    ) -> Self {
        Self {
            lead, worker, lead_turns,
            turn_count: Arc::new(Mutex::new(0)),
            failure_count: Arc::new(Mutex::new(0)),
            in_fallback: Arc::new(Mutex::new(false)),
        }
    }

    pub async fn get_active_provider(&self) -> Arc<dyn Provider> {
        let count = *self.turn_count.lock().await;
        let fallback = *self.in_fallback.lock().await;

        if count < self.lead_turns || fallback {
            self.lead.clone()
        } else {
            self.worker.clone()
        }
    }

    pub async fn record_result(&self, success: bool) {
        let mut count = self.turn_count.lock().await;
        *count += 1;

        if !success {
            let mut failures = self.failure_count.lock().await;
            *failures += 1;
            if *failures >= 2 && !*self.in_fallback.lock().await {
                *self.in_fallback.lock().await = true;
                *failures = 0;
                tracing::warn!("Switching to lead model (fallback mode)");
            }
        } else {
            *self.failure_count.lock().await = 0;
        }
    }
}
```

**整合到 agents.toml**:
```toml
[agents.master]
provider = "lead_worker"

[agents.master.lead_worker]
lead_provider = "anthropic"
lead_model = "claude-sonnet-4-20250514"
worker_provider = "ollama"
worker_model = "qwen3-coder:latest"
lead_turns = 3
```

---

### 17.6 中優先級: Message Visibility

**Clawtex 建議**:

```rust
// 在 agent_runtime.rs 的 Message 結構中添加:

#[derive(Debug, Clone)]
pub struct MessageVisibility {
    pub agent_visible: bool,
    pub user_visible: bool,
}

impl Default for MessageVisibility {
    fn default() -> Self {
        Self { agent_visible: true, user_visible: true }
    }
}

impl Message {
    pub fn agent_only(mut self) -> Self {
        self.visibility = MessageVisibility { agent_visible: true, user_visible: false };
        self
    }

    pub fn user_only(mut self) -> Self {
        self.visibility = MessageVisibility { agent_visible: false, user_visible: true };
        self
    }
}
```

---

### 17.7 中優先級: 環境變數安全白名單

**Clawtex 建議**:

```rust
// src/tools/shell.rs 中添加:

const DISALLOWED_ENV_KEYS: &[&str] = &[
    "PATH", "PATHEXT", "SystemRoot",
    "LD_LIBRARY_PATH", "LD_PRELOAD",
    "DYLD_INSERT_LIBRARIES",
    "PYTHONPATH", "NODE_OPTIONS",
    "APPINIT_DLLS", "ComSpec",
    "HOME", "USERPROFILE",
];

fn sanitize_environment(envs: &HashMap<String, String>) -> HashMap<String, String> {
    envs.iter()
        .filter(|(key, _)| {
            let key_upper = key.to_uppercase();
            !DISALLOWED_ENV_KEYS.iter().any(|&k| k.eq_ignore_ascii_case(&key_upper))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
```

---

### 17.8 中優先級: Recipe Retry 機制

**Clawtex 建議** (適配到 Hand TOML):

```toml
# ~/.clawtex/hands/deploy/hand.toml
[retry]
max_retries = 3
timeout_seconds = 120

[[retry.checks]]
type = "shell"
command = "curl -s -o /dev/null -w '%{http_code}' https://myapp.com/health | grep 200"

[retry]
on_failure = "docker compose restart app"
```

```rust
// src/hands/runner.rs 中添加:

pub struct RetryConfig {
    pub max_retries: u32,
    pub checks: Vec<SuccessCheck>,
    pub on_failure: Option<String>,
    pub timeout_seconds: u64,
}

pub async fn run_with_retry(
    hand: &Hand,
    retry_config: &RetryConfig,
    runner: &HandRunner,
) -> Result<String> {
    for attempt in 0..=retry_config.max_retries {
        let result = runner.execute(hand).await?;

        let all_checks_pass = retry_config.checks.iter().all(|check| {
            execute_shell_check(&check.command, retry_config.timeout_seconds).await.is_ok()
        });

        if all_checks_pass {
            return Ok(result);
        }

        if attempt < retry_config.max_retries {
            if let Some(ref cmd) = retry_config.on_failure {
                execute_shell_check(cmd, retry_config.timeout_seconds).await?;
            }
            tracing::warn!("Retry attempt {} / {}", attempt + 1, retry_config.max_retries);
        }
    }
    Err(anyhow!("Max retries exceeded"))
}
```

---

### 17.9 低優先級: MOIM 狀態注入

**Clawtex 建議**:

```rust
// src/agent_runtime.rs — 在每輪 run_loop 開頭:

fn collect_status_info(&self) -> Option<String> {
    let mut parts = vec![];

    // 時間 + 工作目錄
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:00");
    parts.push(format!("<info-msg>\nTime: {}\nWorkspace: ~/.clawtex/workspace/", now));

    // Token 使用率
    if let Some(limit) = self.context_limit {
        let used = self.estimate_tokens();
        let percent = (used as f64 / limit as f64 * 100.0) as u32;
        parts.push(format!("Tokens: {}/{} ({}%)", used, limit, percent));
    }

    // Cron job 狀態
    // ... 從 core.db 讀取

    parts.push("</info-msg>".to_string());
    Some(parts.join("\n"))
}

// 在 run_loop 中:
if let Some(status) = self.collect_status_info() {
    messages.insert(last_assistant_idx, Message::system(status));
}
```

---

## 附錄 A: 關鍵檔案索引

| 功能 | 檔案路徑 (references/goose/) | 行數 |
|------|------|------|
| Agent 核心 | `crates/goose/src/agents/agent.rs` | 1500+ |
| ToolStream | `crates/goose/src/agents/agent.rs` L168-200 | 33 |
| Tool Execution | `crates/goose/src/agents/tool_execution.rs` | 173 |
| Extension Manager | `crates/goose/src/agents/extension_manager.rs` | 1700+ |
| MCP Client | `crates/goose/src/agents/mcp_client.rs` | 400+ |
| Inspector Pipeline | `crates/goose/src/tool_inspection.rs` | 303 |
| Repetition Inspector | `crates/goose/src/tool_monitor.rs` | 136 |
| Recipe 結構 | `crates/goose/src/recipe/mod.rs` | 877 |
| Template 引擎 | `crates/goose/src/recipe/template_recipe.rs` | 299 |
| Recipe Builder | `crates/goose/src/recipe/build_recipe/mod.rs` | 176 |
| Recipe 驗證 | `crates/goose/src/recipe/validate_recipe.rs` | - |
| Lead-Worker | `crates/goose/src/providers/lead_worker.rs` | 746 |
| Context 壓縮 | `crates/goose/src/context_mgmt/mod.rs` | 803 |
| Subagent Handler | `crates/goose/src/agents/subagent_handler.rs` | 342 |
| Summon Extension | `crates/goose/src/agents/platform_extensions/summon.rs` | 800+ |
| MOIM 注入 | `crates/goose/src/agents/moim.rs` | 144 |
| Provider Trait | `crates/goose/src/providers/base.rs` | 500+ |
| Provider Registry | `crates/goose/src/providers/init.rs` | 300+ |
| Canonical Models | `crates/goose/src/providers/canonical/` | 多檔 |
| Local Inference | `crates/goose/src/providers/local_inference.rs` | 500+ |
| Inference Engine | `crates/goose/src/providers/local_inference/inference_engine.rs` | 395 |
| Retry Manager | `crates/goose/src/agents/retry.rs` | 508 |
| ACP Server | `crates/goose-acp/src/server.rs` | 2000+ |
| ACP Transport | `crates/goose-acp/src/transport/` | 多檔 |
| Session Manager | `crates/goose/src/session/session_manager.rs` | - |
| Docker Container | `crates/goose/src/agents/container.rs` | - |
| Prompt Manager | `crates/goose/src/agents/prompt_manager.rs` | - |
| Large Response | `crates/goose/src/agents/large_response_handler.rs` | - |
| Malware Check | `crates/goose/src/agents/extension_malware_check.rs` | - |
| Extension Config | `crates/goose/src/agents/extension.rs` | 500+ |

---

## 附錄 B: Clawtex-Core 實作優先級矩陣

| 優先級 | 項目 | Goose 參考 | Clawtex 現狀 | 預估工時 |
|--------|------|------------|-------------|---------|
| **P0** | ToolStream (通知+結果統一) | `agent.rs:168-200` | 工具直接回傳 String | 4h |
| **P0** | Inspector Pipeline 安全 | `tool_inspection.rs` | 單一 approval gate | 6h |
| **P0** | Canonical Model Registry | `providers/canonical/` | 各 provider 硬編碼 | 3h |
| **P1** | Lead-Worker 雙模型 | `lead_worker.rs` | classifier smart_routing | 5h |
| **P1** | Message Visibility 系統 | `conversation/mod.rs` | 無 | 3h |
| **P1** | MOIM 狀態注入 | `moim.rs` | 無 (手動監控) | 2h |
| **P1** | 環境變數安全白名單 | `extension.rs:Envs` | 無 | 1h |
| **P1** | Recipe Retry 機制 | `retry.rs` | 無 | 4h |
| **P2** | 漸進式壓縮 (middle-out) | `context_mgmt` | Light/Medium/Aggressive | 3h |
| **P2** | Subagent 隔離 (巢狀防護) | `subagent_handler.rs` | delegate tool 無隔離 | 5h |
| **P2** | MiniJinja Template in Hands | `template_recipe.rs` | 無範本引擎 | 4h |
| **P2** | Docker 容器隔離 | `container.rs` | 無 | 6h |
| **P3** | Extension 惡意軟體檢查 | `extension_malware_check.rs` | 無 | 3h |
| **P3** | ACP 代理通訊 | `goose-acp/` | 無 | 8h |
| **P3** | 本地 GGUF 推理 | `local_inference/` | 依賴外部 LM Studio | 12h |
| **P3** | Declarative Provider (JSON) | `providers/declarative/` | 各 provider 手動實作 | 6h |

**總計**: P0 = 13h, P1 = 15h, P2 = 18h, P3 = 29h

---

## 附錄 C: 設計模式清單

| 模式 | Goose 實作 | 評價 |
|------|-----------|------|
| **ToolStream** | `async_stream::stream!` + `tokio::select!` | 優雅，零拷貝合併 |
| **Inspector Pipeline** | `Vec<Box<dyn ToolInspector>>`, 串行執行 | 可擴展，但可並行化 |
| **SharedProvider** | `Arc<Mutex<Option<Arc<dyn Provider>>>>` | 過度包裝 |
| **AgentEvent Stream** | `BoxStream<Result<AgentEvent>>` via `try_stream!` | 統一事件模型 |
| **Conversation Push** | 同 ID 消息自動合併 | 串流友好 |
| **Message Visibility** | `agent_visible + user_visible` 雙控 | 清晰的關注點分離 |
| **Recovery Compaction** | ContextLengthExceeded → 自動壓縮 → 重試 | 魯棒的降級策略 |
| **Middle-out Removal** | 從中間向外移除 tool responses | 保留首尾上下文 |
| **MOIM Injection** | 每輪注入時間+路徑+背景狀態 | Agent 情境感知 |
| **Lead-Worker Fallback** | Turn-based + failure detection → 自動回退 | 智慧成本控制 |
| **Declarative Providers** | JSON 宣告式 + OpenAI 相容 | 零程式碼擴展 |
| **Recipe SubRecipe** | 巢套 + 自動注入 summon extension | 組合式工作流 |
| **Environment Blacklist** | 31 key 禁止覆寫 | 全面的攻擊面覆蓋 |
| **Subagent Isolation** | 新 Agent + 獨立 extensions + SUBAGENT type | 防止遞迴 + context 隔離 |
| **Unicode Tag Detection** | U+E0000-U+E007F 掃描 | 防禦隱蔽 prompt injection |
