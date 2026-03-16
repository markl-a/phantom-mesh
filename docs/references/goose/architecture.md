# Goose 架構掃描

## 1. 專案概覽

**Goose** 是一個本地化、開源的 AI 代理框架，專門為自動化工程任務而設計。本質上是一個端對端的智能編程助手系統，可以從零開始構建完整項目、執行代碼、調試失敗、協調工作流程、與外部 API 交互——全自動。

**核心特性：**
- **多 LLM 支持**：Anthropic Claude、OpenAI、Google Gemini、Azure、Bedrock、Ollama 等 25+ 提供商
- **MCP 服務集成**：通過 Model Context Protocol 擴展工具能力
- **會話管理**：持久化跨越多個輪次的對話上下文
- **權限系統**：細粒度控制代理工具訪問（ACP - Agent Capability Protocol）
- **多介面**：CLI、桌面應用（Tauri）、HTTP 伺服器（Axum）
- **工作流編排**：Recipe 系統支持複雜的多步驟任務鏈

版本：1.27.0 | 授權：Apache-2.0 | 主倉庫：github.com/block/goose

---

## 2. 目錄結構

```
goose/
├── crates/                          # Rust 工作區
│   ├── goose/                       # 核心庫 (lib.rs)
│   │   └── src/
│   │       ├── agents/              # 代理核心邏輯
│   │       ├── providers/           # LLM 提供商實現 (25+)
│   │       ├── conversation/        # 對話管理
│   │       ├── acp/                 # 代理能力協議
│   │       ├── config/              # 配置管理
│   │       ├── session/             # 會話持久化
│   │       ├── permission/          # 權限框架
│   │       ├── execution/           # 執行層
│   │       ├── recipe/              # Recipe 工作流
│   │       └── ... (其他 16+ 模塊)
│   ├── goose-server/                # HTTP 伺服器 (Axum)
│   │   └── src/
│   │       ├── routes/              # REST API 端點
│   │       ├── commands/            # CLI 命令實現
│   │       └── configuration.rs     # 伺服器配置
│   ├── goose-cli/                   # CLI 客戶端
│   │   └── src/
│   │       ├── session/             # 互動式會話
│   │       ├── commands/            # CLI 命令
│   │       ├── recipes/             # Recipe 管理
│   │       └── scenario_tests/      # 集成測試
│   ├── goose-acp/                   # ACP 伺服器實現
│   │   └── src/
│   │       ├── server.rs            # ACP 協議伺服器
│   │       ├── transport/           # HTTP/WebSocket 傳輸
│   │       └── custom_requests.rs   # 自訂請求處理
│   ├── goose-mcp/                   # MCP 客戶端/伺服器
│   │   └── src/
│   │       ├── mcp_server_runner.rs # MCP 伺服器啟動
│   │       ├── computercontroller/  # 計算機控制工具
│   │       └── memory/              # MCP 記憶體服務
│   ├── goose-acp-macros/            # ACP 過程宏
│   └── goose-test/                  # 集成測試套件
├── ui/
│   ├── desktop/                     # Tauri 桌面應用
│   └── text/                        # 終端 UI
└── documentation/                   # 文檔與示例
```

**關鍵檔案：**
- `Cargo.toml` - 工作區配置，定義 25+ 依賴和樹解析器
- `rust-toolchain.toml` - Rust 版本指定
- `clippy.toml` - 代碼品質檢查規則

---

## 3. 核心 Trait/Struct

### 3.1 代理核心（Agent）

```rust
// crates/goose/src/agents/agent.rs
pub struct Agent {
    provider: Arc<dyn Provider>,           // LLM 提供商（可動態切換）
    extension_manager: ExtensionManager,   // MCP 工具管理
    session_config: SessionConfig,         // 會話配置
    permission_manager: PermissionManager, // 權限控制
}

pub trait Provider: Send + Sync {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse>;
    async fn stream_chat(&self, req: ChatRequest) -> Result<ChatStream>;
    // 支持多模態、工具呼叫、流式輸出
}

pub struct AgentConfig {
    model: String,                         // 模型標識
    system_prompt: String,
    temperature: f32,
    max_tokens: usize,
    tool_use_enabled: bool,
}
```

### 3.2 會話管理（Conversation）

```rust
// crates/goose/src/conversation/mod.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation(Vec<Message>);

pub struct Message {
    pub role: Role,              // User / Assistant / System
    pub content: Vec<MessageContent>,
    pub id: Option<String>,
    pub metadata: Option<MessageMetadata>,
}

pub enum MessageContent {
    Text(TextMessage),
    ToolCall(ToolCall),
    ToolResult(ToolResult),
    Image(ImageContent),
    // ... 其他類型
}
```

### 3.3 提供商抽象（Provider Registry）

```rust
// crates/goose/src/providers/mod.rs
pub mod anthropic;       // Claude API
pub mod openai;          // OpenAI API
pub mod google;          // Google Vertex AI
pub mod ollama;          // 本地推論
pub mod azure;           // Azure OpenAI
pub mod gemini_cli;      // Gemini CLI
pub mod bedrock;         // AWS Bedrock
pub mod codex;           // 自訂代理
pub mod local_inference; // 本地推論引擎
// ... 共 25+ 提供商
```

### 3.4 權限系統（ACP - Agent Capability Protocol）

```rust
// crates/goose/src/acp/mod.rs
pub struct PermissionManager {
    decisions: Arc<Mutex<HashMap<String, PermissionDecision>>>,
}

pub enum PermissionDecision {
    Approved,
    Denied,
    RequiresApproval,
}

// crates/goose-acp/src/server.rs
pub struct GooseAcpAgent {
    sessions: Arc<Mutex<HashMap<String, GooseAcpSession>>>,
    permission_manager: Arc<PermissionManager>,
}
```

### 3.5 擴展/工具管理（Extension Manager）

```rust
// crates/goose/src/agents/extension_manager.rs
pub struct ExtensionManager {
    extensions: Vec<Extension>,
    mcp_clients: HashMap<String, McpClient>,
}

pub enum ExtensionConfig {
    Stdio {
        name: String,
        cmd: String,
        args: Vec<String>,
        envs: Envs,
    },
    Http {
        url: String,
    },
}
```

### 3.6 Recipe 工作流引擎

```rust
// crates/goose/src/recipe/mod.rs
pub struct Recipe {
    name: String,
    phases: Vec<Phase>,
    conditions: Vec<Condition>,
    resources: HashMap<String, String>,
}

pub struct Phase {
    name: String,
    tasks: Vec<Task>,
    success_condition: Option<String>,
}
```

---

## 4. 啟動流程

### 4.1 伺服器模式（goose-server）

```
┌─────────────────────────────────────────────────────────┐
│ main.rs - 解析 CLI 參數                                  │
├─────────────────────────────────────────────────────────┤
│ Commands::Agent                                         │
│  → commands::agent::run()                               │
└─────────────┬───────────────────────────────────────────┘
              │
              ▼
┌─────────────────────────────────────────────────────────┐
│ configuration.rs - 載入 config.yaml                      │
│ • 提供商設定                                              │
│ • MCP 伺服器定義                                          │
│ • 權限規則                                                │
└─────────────┬───────────────────────────────────────────┘
              │
              ▼
┌─────────────────────────────────────────────────────────┐
│ Axum 路由設定 (routes/)                                  │
│ • POST /chat/{session_id}     - 發送消息                 │
│ • GET  /session/{session_id}  - 獲取會話                 │
│ • POST /permission/request    - 權限確認                 │
│ • WebSocket /subscribe        - 實時更新                 │
└─────────────┬───────────────────────────────────────────┘
              │
              ▼
┌─────────────────────────────────────────────────────────┐
│ 監聽 localhost:7878 (可配置)                             │
│ 支援 TLS、代理、認證                                      │
└─────────────────────────────────────────────────────────┘
```

### 4.2 CLI 模式（goose-cli）

```
┌──────────────────────────────────────────────────────────┐
│ main.rs                                                  │
│ → goose_cli::logging::setup_logging()                    │
│ → cli() 命令解析 (Clap)                                  │
└─────────────┬────────────────────────────────────────────┘
              │
              ├─► session  - 互動式聊天會話
              ├─► recipe   - 執行 Recipe
              ├─► project  - 項目管理
              ├─► gateway  - 網關配置
              ├─► schedule - 排程任務
              └─► info     - 系統信息
              │
              ▼
┌──────────────────────────────────────────────────────────┐
│ session/mod.rs - 互動式會話引擎                           │
│ • input()      - 讀用戶輸入                              │
│ • completion() - 調用 Agent                              │
│ • output()     - 流式輸出结果                             │
│ • export()     - 保存會話記錄                             │
└──────────────────────────────────────────────────────────┘
```

### 4.3 ACP 伺服器模式（goose-server acp）

```
┌──────────────────────────────────────────────────────────┐
│ goose-server mcp auto-visualiser/computer-controller     │
├──────────────────────────────────────────────────────────┤
│ 使用 goose-mcp::mcp_server_runner::serve()               │
│ 實現 MCP 協議伺服器                                       │
│ JSON-RPC 2.0 over stdio                                  │
└──────────────────────────────────────────────────────────┘
```

---

## 5. 資料流 ASCII 圖

### 5.1 標準聊天流程

```
用戶輸入
   │
   ▼
┌──────────────────────────────────────┐
│ Session::new_message(prompt)         │
└──────────────────┬───────────────────┘
                   │
                   ▼
          ┌─────────────────┐
          │ Conversation    │
          │ .push(message)  │ ◄─── 消息歷史
          └────────┬────────┘
                   │
                   ▼
        ┌──────────────────────┐
        │ Agent.chat()         │
        │ (Provider + Tools)   │
        └──────────┬───────────┘
                   │
                   ├─────────────────┐
                   │                 │
                   ▼                 ▼
          ┌──────────────┐   ┌──────────────────┐
          │ 生成回應      │   │ 工具呼叫決策      │
          │ (Text)       │   │ (Tool Calls)     │
          └──────────────┘   └────────┬─────────┘
                                      │
                                      ▼
                          ┌───────────────────────┐
                          │ ExtensionManager      │
                          │ .execute_tool(...)    │
                          └───────────┬───────────┘
                                      │
                                      ├──► MCP 伺服器
                                      ├──► Shell 命令
                                      ├──► 文件操作
                                      └──► HTTP 請求
                                      │
                          ┌───────────▼───────────┐
                          │ ToolResult            │
                          │ 返回工具執行結果      │
                          └───────────────────────┘
                                      │
                   ┌──────────────────┘
                   │
                   ▼
        ┌──────────────────────┐
        │ 迴圈 (if needed)     │
        │ 或 Final Response    │
        └──────────┬───────────┘
                   │
                   ▼
          ┌─────────────────┐
          │ 流式輸出到用戶  │
          │ 更新 UI         │
          └─────────────────┘
```

### 5.2 權限確認流程

```
Tool 呼叫
   │
   ▼
┌─────────────────────────────────┐
│ PermissionManager::check()       │
└──────────────┬──────────────────┘
               │
        ┌──────┴──────┐
        │             │
        ▼             ▼
    Approved     Need Approval
        │             │
        │             ▼
        │      ┌──────────────────┐
        │      │ 發送權限確認      │
        │      │ (Telegram/UI)    │
        │      └────────┬─────────┘
        │               │
        │        ┌──────┴──────┐
        │        │             │
        │        ▼             ▼
        │    User Approves  User Denies
        │        │             │
        └────────┴──────┬──────┘
                       │
                       ▼
              ┌─────────────────┐
              │ 執行或拒絕       │
              │ 工具呼叫        │
              └─────────────────┘
```

### 5.3 Recipe 執行流程

```
用戶請求 Recipe (JSON/YAML)
   │
   ▼
┌─────────────────────────────┐
│ Recipe Parser               │
│ (phases + conditions)        │
└──────────────┬──────────────┘
               │
               ▼
        ┌─────────────────┐
        │ Phase 1         │ ◄──── 並行或順序
        │ Task 1, 2, 3    │
        └────────┬────────┘
                 │
                 ▼
         ┌──────────────────┐
         │ Check Condition  │
         └────────┬─────────┘
                  │
          ┌───────┴────────┐
          ▼                ▼
        Pass            Fail
        │                 │
        ▼                 ▼
    Phase 2          Error Handler
    ...              ...
```

---

## 6. 子系統清單 (Priority 等級)

### P0 - 核心（必須）

| 子系統 | 檔案位置 | 功能 | 狀態 |
|--------|---------|------|------|
| **Agent Core** | `agents/agent.rs` | 代理主邏輯、工具執行循環 | ✅ 穩定 |
| **Provider Factory** | `providers/init.rs`, `provider_registry.rs` | LLM 動態載入與切換 | ✅ 穩定 |
| **Conversation** | `conversation/mod.rs` | 消息歷史管理、驗證 | ✅ 穩定 |
| **ACP Protocol** | `acp/mod.rs`, `goose-acp/src/server.rs` | 權限與能力協議 | ✅ 穩定 |
| **Session Manager** | `session/mod.rs` | 會話持久化、狀態管理 | ✅ 穩定 |
| **Extension Manager** | `agents/extension_manager.rs` | MCP 伺服器整合 | ✅ 穩定 |
| **Axum HTTP Server** | `goose-server/src/routes/` | REST API 端點 | ✅ 穩定 |

### P1 - 重要（功能特性）

| 子系統 | 檔案位置 | 功能 | 狀態 |
|--------|---------|------|------|
| **Recipe Engine** | `recipe/mod.rs` | 工作流編排與執行 | ✅ 穩定 |
| **MCP Client** | `agents/mcp_client.rs` | MCP 協議客戶端 | ✅ 穩定 |
| **Tool Inspection** | `tool_inspection.rs` | Schema 分析與驗證 | ✅ 穩定 |
| **Permission UI** | `goose-server/routes/action_required.rs` | 權限確認介面 | ✅ 穩定 |
| **Token Counting** | `token_counter.rs` | 代碼化計數（Tiktoken） | ✅ 穩定 |
| **OAuth Handler** | `oauth/mod.rs` | 第三方認證 | ✅ 穩定 |
| **Execution Layer** | `execution/mod.rs` | 命令執行、沙箱 | ✅ 穩定 |

### P2 - 增強（可選/實驗）

| 子系統 | 檔案位置 | 功能 | 狀態 |
|--------|---------|------|------|
| **Config Management** | `config/mod.rs` | YAML/TOML 配置解析 | ✅ 穩定 |
| **Local Inference** | `providers/local_inference.rs` | 本地模型推論 | ⚠️ 實驗 |
| **Dictation** | `dictation.rs` | 語音轉文本 | ⚠️ 實驗 |
| **Scheduler** | `scheduler/mod.rs` | 任務排程（Cron） | ⚠️ 實驗 |
| **Context Management** | `context_mgmt.rs` | 上下文壓縮與檢索 | ⚠️ 實驗 |
| **Posthog Analytics** | `posthog.rs` | 使用者分析跟蹤 | ⚠️ 實驗 |
| **OTEL Instrumentation** | `otel/mod.rs` | OpenTelemetry 監控 | ⚠️ 實驗 |
| **Desktop UI** | `ui/desktop/` | Tauri 應用程序 | ✅ 穩定 |
| **Gateway** | `gateway/mod.rs` | API 網關、路由 | ⚠️ 實驗 |

---

## 7. 技術棧總結

### 語言與框架
- **語言**：Rust 1.70+ (Edition 2021)
- **異步運行時**：Tokio 1.49+
- **HTTP 框架**：Axum 0.8 (Hyper 底層)
- **CLI 框架**：Clap 4
- **序列化**：Serde + Serde JSON + YAML

### 關鍵依賴
- **MCP 協議**：rmcp 1.2.0 (Model Context Protocol)
- **ACP 協議**：sacp 10.1.0 (Agent Capability Protocol)
- **LLM 套件**：Tiktoken (代碼化)、OAuth2
- **代碼分析**：Tree-sitter (8 語言支持)
- **可視化**：OpenTelemetry + OTLP
- **測試**：WireMock、Serial Test、Test Case

### 提供商支持 (25+)
Anthropic、OpenAI、Google Gemini、Azure、AWS Bedrock、Ollama、
LiteLLM、OpenRouter、Venice、XAI、Databricks、Snowflake、
GitHub Copilot、Cursor、ChatGPT Codex、本地推論

---

## 8. 關鍵特性對比 (vs Clawtex)

| 特性 | Goose | Clawtex |
|------|-------|---------|
| **提供商數量** | 25+ | 6+ |
| **協議** | ACP + MCP | HTTP + MCP |
| **會話管理** | ✅ 完整持久化 | ⚠️ 基本 |
| **權限系統** | ✅ 細粒度 ACP | ⚠️ 粗粒度 |
| **工作流引擎** | ✅ Recipe (YAML/JSON) | ⚠️ Hands (TOML) |
| **工具數量** | 12+ 內置 | 24+ (含聚合) |
| **桌面應用** | ✅ Tauri | ❌ CLI only |
| **本地推論** | ✅ 支持 | ⚠️ 限制 |
| **代碼分析** | ✅ Tree-sitter (8 種) | ✅ 自訂 |

---

## 9. 快速啟動參考

```bash
# 編譯整個工作區
cargo build --release

# 啟動 HTTP 伺服器
cargo run --bin goose-server -- agent

# 啟動 CLI
cargo run --bin goose-cli

# 驗證配置
cargo run --bin goose-server -- validate-extensions /path/to/extensions.json

# 執行測試套件
cargo test --workspace
```

## 10. 設計模式與最佳實踐

1. **Trait-based Provider**：所有 LLM 實現統一 `Provider` trait
2. **Session Isolation**：每個會話獨立的代理實例與權限上下文
3. **Async-first**：全堆棧異步 (Tokio)，支持流式輸出
4. **Protocol Layering**：ACP (權限) → MCP (工具) → Provider (推論)
5. **Tool Schema Validation**：自動 JSON Schema 檢查
6. **Error Propagation**：使用 `anyhow::Result` + `thiserror`

---

**文檔生成日期**：2026-03-13
**資訊來源**：Goose 源碼掃描 (v1.27.0)
**掃描範圍**：7 個核心 Crate、37 個模塊、500+ 檔案

