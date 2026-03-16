# Codex CLI 深度技術分析 v2

> 分析日期：2026-03-13 (深度重寫版)
> 專案來源：`references/codex-cli/` (OpenAI Codex CLI)
> 分析目的：為 clawtex-core 提取可採用的架構模式與實作參考
> 深度：程式碼行號級引用、資料流圖、錯誤處理路徑、效能瓶頸、clawtex 差距對比

---

## 目錄

1. [專案結構與規模](#1-專案結構與規模)
2. [入口點與啟動流程](#2-入口點與啟動流程)
3. [SQ/EQ Event Queue 解耦架構（深度）](#3-sqeq-event-queue-解耦架構深度)
4. [三平台沙箱系統（深度）](#4-三平台沙箱系統深度)
5. [ApprovalStore 快取機制（深度）](#5-approvalstore-快取機制深度)
6. [Internal→External 事件映射（深度）](#6-internalexternal-事件映射深度)
7. [工具系統與並行執行](#7-工具系統與並行執行)
8. [上下文管理與自動壓縮](#8-上下文管理與自動壓縮)
9. [Provider 整合與認證](#9-provider-整合與認證)
10. [錯誤處理與韌性設計](#10-錯誤處理與韌性設計)
11. [效能分析](#11-效能分析)
12. [Clawtex 差距總覽與實作路線圖](#12-clawtex-差距總覽與實作路線圖)
13. [多代理控制系統（深度）](#13-多代理控制系統深度)
14. [CodexThread 抽象層（深度）](#14-codexthread-抽象層深度)
15. [上下文壓縮引擎（深度）](#15-上下文壓縮引擎深度)
16. [Codex 結構體與 Session 架構（深度）](#16-codex-結構體與-session-架構深度)
17. [協議與事件系統深入分析](#17-協議與事件系統深入分析)
18. [關鍵檔案索引](#18-關鍵檔案索引)

---

## 1. 專案結構與規模

### 頂層目錄

```
codex-cli/
├── codex-cli/          # TypeScript 薄包裝層 (npm 套件入口)
│   ├── bin/codex.js    # Node.js 入口 — 偵測平台後 spawn Rust 二進位
│   └── package.json    # @openai/codex
├── codex-rs/           # Rust workspace（全部核心邏輯）
│   ├── Cargo.toml      # workspace 定義，71 個 crate
│   ├── cli/            # `codex` 主二進位（TUI + 子命令路由）
│   ├── exec/           # `codex-exec` 非互動式二進位
│   ├── core/           # codex-core — 代理迴圈、工具、沙箱、上下文
│   ├── protocol/       # 協議定義（Op、Event、SandboxPolicy 等）
│   ├── app-server/     # WebSocket/stdio JSON-RPC 伺服器
│   ├── tui/            # ratatui 終端 UI
│   ├── linux-sandbox/  # Landlock + seccomp + bubblewrap
│   ├── windows-sandbox-rs/  # Windows restricted token (27 個模組)
│   ├── mcp-server/     # MCP 伺服器實作
│   ├── hooks/          # 生命週期 hook 系統
│   └── ... (共 71 個 crate)
├── sdk/                # SDK 層
└── shell-tool-mcp/     # Shell 工具 MCP 伺服器
```

### Workspace 規模與 Crate 分類

Cargo workspace 包含 **71 個 crate**，這是極度細粒度的模組化：

| 類別 | Crate 範例 | 數量 |
|------|-----------|------|
| **二進位入口** | `cli`, `exec`, `app-server` | 3 |
| **核心引擎** | `core`, `protocol` | 2 |
| **API 客戶端** | `codex-api`, `codex-client`, `backend-client` | 3 |
| **沙箱** | `linux-sandbox`, `windows-sandbox-rs`, `process-hardening` | 3 |
| **工具生態** | `mcp-server`, `rmcp-client`, `hooks`, `skills` | 4 |
| **UI** | `tui`, `ansi-escape` | 2 |
| **工具庫** | `utils/*` (stream-parser, pty, absolute-path, readiness...) | 15+ |
| **配置** | `config`, `config-loader`, `execpolicy` | 3 |
| **其他** | `network-proxy`, `otel`, `shell-command` 等 | 30+ |

**檔案路徑：** `codex-rs/Cargo.toml` (workspace 根)

---

## 2. 入口點與啟動流程

### 2.1 雙入口設計

Codex CLI 有兩個主要入口二進位：

**`codex` (互動式 TUI)**：`codex-rs/cli/src/main.rs`

```rust
// codex-rs/cli/src/main.rs
#[derive(Debug, Parser)]
#[clap(bin_name = "codex")]
struct MultitoolCli {
    #[clap(flatten)]
    pub config_overrides: CliConfigOverrides,
    #[clap(flatten)]
    interactive: TuiCli,  // 預設走 TUI
    #[clap(subcommand)]
    command: Option<SubCommand>,
}
// 子命令: exec, login, mcp, seatbelt, landlock, windows-sandbox
```

**`codex-exec` (非互動式/JSON 模式)**：`codex-rs/exec/src/main.rs`

```rust
// codex-rs/exec/src/main.rs
fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(|arg0_paths: Arg0DispatchPaths| async move {
        let top_cli = TopCli::parse();
        run_main(inner, arg0_paths).await?;
        Ok(())
    })
}
```

### 2.2 arg0 分發模式

```rust
// 一個二進位 = 多個功能。根據 argv[0] 名稱分發：
// "codex-linux-sandbox" → run_main() 走沙箱邏輯
// "codex-exec"          → run_main() 走 exec 邏輯
arg0_dispatch_or_else(|paths| async move { ... });
```

這允許交叉編譯一個二進位後通過符號連結承載多個功能，減少磁碟佔用。

### 2.3 Codex::spawn 完整初始化流程

```
codex-rs/core/src/codex.rs:379 — Codex::spawn()
│
├─ 1. 驗證 parent trace (W3C TraceContext)
├─ 2. spawn_internal()
│   ├─ 建立有界提交通道: bounded(512)
│   ├─ 建立無界事件通道: unbounded()
│   ├─ 載入 plugins (PluginsManager)
│   ├─ 載入 skills (SkillsManager)
│   ├─ 載入 exec policy (ExecPolicyManager, Starlark 引擎)
│   ├─ 解析模型 (ModelsManager::refresh)
│   ├─ 建立 MCP 連線管理器 (McpConnectionManager)
│   ├─ 建立 Session (含 SessionServices)
│   ├─ 生成 submission_loop tokio task
│   └─ 返回 CodexSpawnOk { codex, thread_id }
```

**關鍵常數**：

```rust
// codex-rs/core/src/codex.rs:373
pub(crate) const SUBMISSION_CHANNEL_CAPACITY: usize = 512;
// 事件通道是 unbounded — 防止背壓阻塞代理迴圈
```

### 2.4 TypeScript 薄包裝

`codex-cli/bin/codex.js` 純粹是 npm 分發的啟動器（~30 行）：

```javascript
// codex-cli/bin/codex.js
const child = spawn(binaryPath, process.argv.slice(2), {
    stdio: "inherit", env,
});
["SIGINT", "SIGTERM", "SIGHUP"].forEach((sig) => {
    process.on(sig, () => forwardSignal(sig));
});
```

**結論**：TypeScript 層**完全不包含**業務邏輯。所有代理邏輯、API 呼叫、沙箱管理都在 Rust 層。

> **Clawtex 實作建議**：clawtex-core 的純 Rust 架構與 Codex 一致。但 Codex 的 arg0 分發模式值得學習 — 可以讓 clawtex-core 二進位同時承載 daemon、worker、sandbox 等功能。

---

## 3. SQ/EQ Event Queue 解耦架構（深度）

### 3.1 架構概覽

Codex 的核心是 **Submission Queue / Event Queue (SQ/EQ)** 模式 — 靈感來自 Linux io_uring 的提交佇列/完成佇列設計。

```
┌─────────────┐     Submission      ┌──────────────────┐     Event        ┌──────────────┐
│   外部呼叫者  │ ──── (SQ) ────→   │  submission_loop  │ ──── (EQ) ────→ │   外部消費者   │
│  (TUI/exec)  │    bounded(512)    │    (tokio task)   │    unbounded    │  (TUI/JSONL)  │
└─────────────┘                     └──────────────────┘                  └──────────────┘
                                           ↕
                                      handlers::*
                                           ↕
                                    run_turn() 迴圈
                                    ┌──────────┐
                                    │ sample() │ ←→ Model API
                                    │ tools()  │ ←→ 工具執行
                                    │ compact()│ ←→ 上下文壓縮
                                    └──────────┘
```

### 3.2 Codex 結構體定義

```rust
// codex-rs/core/src/codex.rs:329-340
/// 高層介面。以佇列對運作：送出 submissions、接收 events。
pub struct Codex {
    pub(crate) tx_sub: Sender<Submission>,     // 提交佇列 (有界 512)
    pub(crate) rx_event: Receiver<Event>,       // 事件佇列 (無界)
    pub(crate) agent_status: watch::Receiver<AgentStatus>,  // 狀態廣播
    pub(crate) session: Arc<Session>,
    pub(crate) session_loop_termination: SessionLoopTermination,
}
```

### 3.3 Submission (SQ) 完整定義

```rust
// codex-rs/protocol/src/protocol.rs
pub struct Submission {
    pub id: String,
    pub op: Op,
    pub trace: Option<W3cTraceContext>,  // 分散式追蹤
}

pub enum Op {
    // --- 使用者互動 ---
    Interrupt,                              // 中斷當前操作
    UserInput { items: Vec<UserInput>, ... },  // 使用者訊息（同 turn）
    UserTurn { items, cwd, approval_policy, sandbox_policy, model, ... },  // 新 turn
    OverrideTurnContext { ... },             // 覆蓋 turn 上下文

    // --- 審批 ---
    ExecApproval { id, turn_id, decision },  // 命令執行審批
    PatchApproval { id, decision },          // 檔案修改審批
    NetworkPolicyApproval { ... },           // 網路策略審批
    PermissionsResponse { ... },             // 權限回應
    UserInputResponse { ... },               // 使用者輸入回應

    // --- 工具管理 ---
    DynamicToolResponse { ... },             // 動態工具回應
    McpServerRefresh { ... },                // MCP 伺服器刷新

    // --- 系統控制 ---
    Shutdown,                                // 關閉
    Compact,                                 // 手動壓縮

    // --- 即時對話 ---
    RealtimeConversationStart { ... },       // 即時對話開始
    RealtimeConversationAudio { ... },       // 音訊資料
    RealtimeConversationText { ... },        // 文字資料
    RealtimeConversationClose,               // 即時對話結束

    // --- 回饋 ---
    FeedbackTag { ... },                     // 回饋標記

    // 共計 20+ 個變體
}
```

### 3.4 submission_loop 核心迴圈

```rust
// codex-rs/core/src/codex.rs (概念化重建，非直接引用)
async fn submission_loop(
    sess: Arc<Session>,
    config: Arc<Config>,
    rx_sub: Receiver<Submission>,
) {
    while let Ok(sub) = rx_sub.recv().await {
        // 設定 W3C trace context (分散式追蹤)
        if let Some(trace) = &sub.trace {
            set_parent_from_w3c_trace_context(&span, trace);
        }

        match sub.op.clone() {
            Op::Interrupt => handlers::interrupt(&sess).await,
            Op::UserInput { .. } | Op::UserTurn { .. } => {
                handlers::user_input_or_turn(&sess, sub.id, sub.op).await;
            }
            Op::ExecApproval { id, turn_id, decision } => {
                handlers::exec_approval(&sess, id, turn_id, decision).await;
            }
            Op::Compact => handlers::compact(&sess).await,
            Op::Shutdown => break,
            // ... 20+ 個 Op 變體的分派
        }
    }
}
```

### 3.5 run_turn — 代理主迴圈

```rust
// codex-rs/core/src/codex.rs (概念化)
pub(crate) async fn run_turn(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    input: Vec<UserInput>,
    prewarmed_client_session: Option<ModelClientSession>,
    cancellation_token: CancellationToken,
) -> Option<String> {
    // 1. 發送 TurnStarted 事件
    sess.send_event(&turn_context, EventMsg::TurnStarted(...)).await;

    // 2. 預壓縮（如果上下文接近限制）
    run_pre_sampling_compact(&sess, &turn_context).await;

    // 3. 解析技能、插件、MCP 工具
    let mcp_tools = sess.services.mcp_connection_manager.list_tools().await;
    let skill_items = build_skill_injections(&sess, &turn_context).await;

    // 4. 主迴圈：采樣 → 工具執行 → 再次采樣
    loop {
        let sampling_request_input = sess.clone_history().await
            .for_prompt(&turn_context.model_info.input_modalities);

        match run_sampling_request(&sess, &turn_context, &sampling_request_input).await {
            Ok(result) => {
                if result.needs_follow_up {
                    // 有工具呼叫，執行工具後繼續迴圈
                    if token_limit_reached {
                        run_auto_compact(&sess, &turn_context).await;
                    }
                    continue;
                }
                // 沒有 follow-up，turn 結束
                break;
            }
            Err(err) => {
                // 錯誤處理：重試或終止
                match classify_error(&err) {
                    ErrorClass::Retryable => {
                        backoff::exponential_backoff(attempt).await;
                        continue;
                    }
                    ErrorClass::Fatal => {
                        sess.send_event(EventMsg::Error(...)).await;
                        break;
                    }
                }
            }
        }
    }
}
```

### 3.6 SQ/EQ 資料流完整圖

```
外部 (TUI/exec/app-server)
    │
    ▼ Op::UserTurn { items, model, sandbox_policy, ... }
┌─────────────────────────────────────────────────────────────┐
│                   submission_loop                            │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐   │
│  │  handlers::user_input_or_turn()                       │   │
│  │    ├─ 建立 TurnContext (model_info, sandbox, cwd)     │   │
│  │    ├─ 建立 CancellationToken (child_token)            │   │
│  │    └─ tokio::spawn(run_turn(...))                     │   │
│  │                                                        │   │
│  │  run_turn():                                           │   │
│  │    ├─ ① EventMsg::TurnStarted →→→→→→→→→→→→→→→→→→→→→→  │   │
│  │    ├─ ② run_pre_sampling_compact()                    │   │
│  │    ├─ ③ build_skill_injections()                      │   │
│  │    ├─ ④ loop {                                         │   │
│  │    │     ├─ clone_history().for_prompt()               │   │
│  │    │     ├─ run_sampling_request() ───→ Model API      │   │
│  │    │     │     ├─ EventMsg::AgentMessage →→→→→→→→→→→→  │   │
│  │    │     │     ├─ EventMsg::AgentReasoning →→→→→→→→→→  │   │
│  │    │     │     └─ tool_calls[]                         │   │
│  │    │     ├─ ToolCallRuntime::handle_tool_call() ×N     │   │
│  │    │     │     ├─ EventMsg::ExecCommandBegin →→→→→→→→  │   │
│  │    │     │     ├─ 沙箱轉換 + 執行                      │   │
│  │    │     │     ├─ EventMsg::ExecCommandEnd →→→→→→→→→→  │   │
│  │    │     │     └─ 審批流程 (如需要)                     │   │
│  │    │     │           ├─ EventMsg::ExecApprovalReq →→→  │   │
│  │    │     │           └─ 等待 Op::ExecApproval ←←←←←←  │   │
│  │    │     ├─ auto_compact (如 token 超限)               │   │
│  │    │     └─ needs_follow_up? → continue/break          │   │
│  │    │   }                                               │   │
│  │    └─ ⑤ EventMsg::TurnComplete →→→→→→→→→→→→→→→→→→→→→  │   │
│  └──────────────────────────────────────────────────────┘   │
│                          ↓↓↓ Event 通道 (unbounded) ↓↓↓     │
└─────────────────────────────────────────────────────────────┘
                           │
                           ▼
                    外部事件消費者
              ┌─────────────────────┐
              │ EventProcessorWith  │
              │ JsonOutput          │  → 8 種 ThreadEvent (JSONL)
              │ (50+ EventMsg →     │
              │  8 ThreadEvent)     │
              └─────────────────────┘
```

### 3.7 CancellationToken 階層

```rust
// Codex 使用 tokio-util 的 CancellationToken 形成樹狀取消
Session CancellationToken
  └─ Turn CancellationToken (child_token)
       └─ Tool CancellationToken (child_token)
            └─ Exec CancellationToken (child_token)

// 取消父級會連鎖取消所有子級
// 取消子級不影響父級
```

### 3.8 watch Channel 狀態廣播

```rust
// codex-rs/core/src/codex.rs
let (agent_status_tx, agent_status_rx) = watch::channel(AgentStatus::PendingInit);
// 多個觀察者可以隨時讀取最新狀態，不需要緩衝
// AgentStatus: PendingInit → Ready → Working → Error → Shutdown
```

### 3.9 W3C Trace Context 傳播

```rust
pub struct Submission {
    pub trace: Option<W3cTraceContext>,
}
pub struct W3cTraceContext {
    pub traceparent: Option<String>,  // trace-id + span-id
    pub tracestate: Option<String>,   // 供應商擴展
}
// 每個 Submission 可攜帶 trace context，跨非同步邊界傳播
```

> **Clawtex 實作建議**：
> 1. clawtex-core 的 `agent_runtime.rs` 應引入 SQ/EQ 分離。目前 clawtex 直接在 telegram handler 中呼叫 provider，沒有佇列解耦。
> 2. 建議引入 `enum AgentOp` (UserMessage, ToolResult, Interrupt, Shutdown) + `enum AgentEvent` (Message, ToolCall, Error, TurnComplete)。
> 3. 有界提交通道 (512) + 無界事件通道是好的預設 — 防止外部壓力但不阻塞代理。
> 4. CancellationToken 樹應替換 clawtex 現有的 `AtomicBool` e-stop 機制。
> 5. W3C TraceContext 可用於 clawtex 的分散式 cluster 追蹤。

---

## 4. 三平台沙箱系統（深度）

### 4.1 架構總覽

Codex 的沙箱是**三層疊加**設計：

```
層次 1: SandboxPolicy (協議層)     — 宣告「想要」什麼保護
層次 2: SandboxManager (核心層)    — 選擇平台實作 + 轉換命令
層次 3: 平台沙箱 (系統層)          — 實際的 OS 級隔離
         ├── macOS: Seatbelt (sandbox-exec profiles)
         ├── Linux: Landlock + seccomp + bubblewrap
         └── Windows: Restricted Token + ACL + Firewall + DPAPI
```

### 4.2 層次 1 — SandboxPolicy

```rust
// codex-rs/protocol/src/protocol.rs
pub enum SandboxPolicy {
    DangerFullAccess,                        // 完全無限制
    ReadOnly { access, network_access },     // 唯讀
    ExternalSandbox { network_access },      // 外部沙箱
    WorkspaceWrite {                         // 工作區可寫（最常用）
        writable_roots: Vec<PathBuf>,
        network_access: NetworkAccess,
    },
}
```

### 4.3 層次 2 — SandboxManager

```rust
// codex-rs/core/src/sandboxing/mod.rs:52-78
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
    pub expiration: ExecExpiration,
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<PermissionProfile>,
    pub justification: Option<String>,  // 用於審計日誌
}

pub struct ExecRequest {
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
    pub network: Option<NetworkProxy>,      // 網路代理
    pub expiration: ExecExpiration,
    pub sandbox: SandboxType,               // 選定的沙箱類型
    pub windows_sandbox_level: WindowsSandboxLevel,
    pub sandbox_permissions: SandboxPermissions,
    pub sandbox_policy: SandboxPolicy,
    pub file_system_sandbox_policy: FileSystemSandboxPolicy,
    pub network_sandbox_policy: NetworkSandboxPolicy,
    pub justification: Option<String>,
    pub arg0: Option<String>,
}
```

**沙箱選擇邏輯**：

```rust
// codex-rs/core/src/exec.rs:88-101
fn select_process_exec_tool_sandbox_type(
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    network_sandbox_policy: NetworkSandboxPolicy,
    windows_sandbox_level: WindowsSandboxLevel,
    enforce_managed_network: bool,
) -> SandboxType {
    SandboxManager::new().select_initial(
        file_system_sandbox_policy,
        network_sandbox_policy,
        SandboxablePreference::Auto,  // Auto / Require / Forbid
        windows_sandbox_level,
        enforce_managed_network,
    )
}
```

### 4.4 層次 3a — Linux 沙箱（最完整）

**Crate**：`codex-rs/linux-sandbox/`

```rust
// codex-rs/linux-sandbox/src/lib.rs:1-25
//! Linux sandbox helper entry point.
//!
//! On Linux, `codex-linux-sandbox` applies:
//! - in-process restrictions (`no_new_privs` + seccomp), and
//! - bubblewrap for filesystem isolation.
mod bwrap;           // bubblewrap 封裝
mod landlock;        // Landlock LSM 規則
mod linux_run_main;  // 主入口
mod proxy_routing;   // 網路代理路由
mod vendored_bwrap;  // 內建的 bwrap 二進位
```

**三重保護堆疊**：

```
┌──────────────────────────────────────────┐
│ 1. PR_SET_NO_NEW_PRIVS                   │  ← 禁止 setuid 提權
│    prctl(PR_SET_NO_NEW_PRIVS, 1)         │
├──────────────────────────────────────────┤
│ 2. seccomp BPF                           │  ← 系統呼叫白名單
│    限制可用的 syscalls                     │
├──────────────────────────────────────────┤
│ 3. Landlock LSM                          │  ← 檔案系統存取控制
│    per-path 讀/寫/執行/刪除權限           │
├──────────────────────────────────────────┤
│ 4. bubblewrap (bwrap)                    │  ← 命名空間隔離
│    mount namespace + PID namespace        │
│    + network namespace (可選)             │
└──────────────────────────────────────────┘
```

**Landlock 規則產生**：

```rust
// codex-rs/linux-sandbox/src/landlock.rs (概念化)
// 根據 FileSystemSandboxPolicy 生成 Landlock 規則：
// - ReadOnlyEntries: 只允許讀
// - WritableEntries: 允許讀寫
// - 自動加入: /usr, /lib, /etc (只讀)
// - cwd 和 writable_roots: 讀寫
```

**bubblewrap 封裝**：

```rust
// codex-rs/linux-sandbox/src/bwrap.rs
// 產生 bwrap 命令列參數：
// --ro-bind /usr /usr       → 只讀掛載系統目錄
// --bind <cwd> <cwd>        → 可讀寫掛載工作目錄
// --dev /dev                → 設備檔案
// --proc /proc              → 程序資訊
// --tmpfs /tmp              → 臨時目錄
// --unshare-pid             → PID 隔離
// --die-with-parent         → 父程序死亡時自動清理
```

### 4.5 層次 3b — macOS Seatbelt

```rust
// codex-rs/core/src/seatbelt.rs
pub const MACOS_PATH_TO_SEATBELT_EXECUTABLE: &str = "/usr/bin/sandbox-exec";

// 產生 sandbox-exec profile (.sb 格式)：
// (version 1)
// (deny default)                   → 預設拒絕
// (allow file-read* (subpath "/usr"))  → 允許讀系統目錄
// (allow file-write* (subpath "<cwd>")) → 允許寫工作區
// (allow network*)                 → 網路（根據策略）
```

**macOS 權限交集/合併**：

```rust
// codex-rs/core/src/sandboxing/macos_permissions.rs
fn intersect_macos_seatbelt_profile_extensions(
    a: Option<&MacOsSeatbeltProfileExtensions>,
    b: &PermissionProfile,
) -> MacOsSeatbeltProfileExtensions {
    // 取兩者的交集，確保安全
}

fn merge_macos_seatbelt_profile_extensions(
    base: Option<&MacOsSeatbeltProfileExtensions>,
    additional: Option<&MacOsSeatbeltProfileExtensions>,
) -> Option<MacOsSeatbeltProfileExtensions> {
    // 合併額外權限
}
```

### 4.6 層次 3c — Windows 沙箱（最複雜）

**Crate**：`codex-rs/windows-sandbox-rs/` — **27 個模組**

```rust
// codex-rs/windows-sandbox-rs/src/lib.rs:1-24
windows_modules!(
    acl,                    // NTFS ACL 操作
    allow,                  // 允許規則
    audit,                  // 審計掃描 (world-writable 偵測)
    cap,                    // Capability SID (沙箱能力)
    dpapi,                  // Windows DPAPI 加密/解密
    env,                    // 環境變數消毒
    helper_materialization, // 輔助程式實體化
    hide_users,             // 隱藏使用者設定檔
    identity,               // 沙箱使用者身份
    logging,                // 沙箱日誌
    path_normalization,     // 路徑正規化
    policy,                 // 策略解析
    process,                // 程序管理
    token,                  // Restricted Token
    winutil,                // Windows 工具函式
    workspace_acl,          // 工作區 ACL
);
// + setup_orchestrator, elevated_impl, setup_error, sandbox_users, ...
```

**Windows 沙箱分級**：

```
WindowsSandboxLevel::Off         → 不啟用沙箱
WindowsSandboxLevel::ReadOnly    → Restricted Token + 唯讀 ACL
WindowsSandboxLevel::Standard    → Restricted Token + 工作區可寫 ACL
WindowsSandboxLevel::Strict      → 獨立使用者帳號 + Firewall 規則 + DPAPI
```

**Restricted Token 流程**：

```
1. token.rs         → CreateRestrictedToken() 移除危險 SID
2. acl.rs           → 設定 NTFS DACL (Deny Write ACE)
3. workspace_acl.rs → 工作區目錄授權
4. firewall.rs      → 設定 Windows Firewall 規則
5. dpapi.rs         → 使用 DPAPI 保護敏感資料
6. hide_users.rs    → 隱藏沙箱使用者帳號
7. audit.rs         → 掃描 world-writable 路徑，加入 Deny ACE
```

**沙箱轉換完整流程**：

```
CommandSpec (高階命令描述)
    │
    ▼ SandboxManager::transform()
ExecRequest (包含平台特定的命令前綴)
    │
    ├─ macOS: ["sandbox-exec", "-f", "<profile>.sb", "--", "bash", "-c", ...]
    ├─ Linux: ["codex-linux-sandbox", "--landlock-rules", ..., "--", "bash", "-c", ...]
    └─ Windows: [restricted token 啟動, "cmd.exe", "/C", ...]
    │
    ▼ execute_exec_request()
tokio::process::Command::new(...)
    .spawn() → Child process
```

### 4.7 網路代理

```rust
// codex-network-proxy crate
// 支援管理式網路存取：
// 1. 啟動本地 HTTP/HTTPS 代理
// 2. 設定 HTTP_PROXY/HTTPS_PROXY 環境變數
// 3. 攔截並審計所有網路請求
// 4. 根據 NetworkSandboxPolicy 允許/拒絕
```

### 4.8 ExecExpiration 與超時

```rust
// codex-rs/core/src/exec.rs:43-146
pub const DEFAULT_EXEC_COMMAND_TIMEOUT_MS: u64 = 10_000;  // 10 秒
pub const IO_DRAIN_TIMEOUT_MS: u64 = 2_000;  // I/O 清理超時 2 秒

pub enum ExecExpiration {
    Timeout(Duration),           // 明確超時
    DefaultTimeout,              // 預設 10 秒
    Cancellation(CancellationToken),  // 取消令牌
}

// I/O 限制常數：
const READ_CHUNK_SIZE: usize = 8192;
const AGGREGATE_BUFFER_INITIAL_CAPACITY: usize = 8 * 1024;
const EXEC_OUTPUT_MAX_BYTES: usize = DEFAULT_OUTPUT_BYTES_CAP;
pub(crate) const MAX_EXEC_OUTPUT_DELTAS_PER_CALL: usize = 10_000;
```

**錯誤處理**：

```rust
const SIGKILL_CODE: i32 = 9;
const TIMEOUT_CODE: i32 = 64;
const EXIT_CODE_SIGNAL_BASE: i32 = 128;
const EXEC_TIMEOUT_EXIT_CODE: i32 = 124;  // 與 GNU timeout 一致
```

> **Clawtex 實作建議**：
> 1. clawtex-core 目前只有基本的工作區路徑隔離 (`SecurityConfig.workspace`)。應引入平台感知沙箱。
> 2. **最小可行方案 (Windows)**：在 shell tool 中使用 Restricted Token。參考 `windows-sandbox-rs/src/token.rs` 的 `CreateRestrictedToken`。
> 3. **Linux 方案**：引入 Landlock (kernel 5.13+) 做檔案系統 ACL，不需要 root。
> 4. 網路代理是重要安全層 — clawtex 的 shell tool 目前無網路控制。
> 5. `ExecExpiration` 的三種模式（固定超時、預設超時、取消令牌）比 clawtex 當前的 `Duration` 更靈活。

---

## 5. ApprovalStore 快取機制（深度）

### 5.1 ApprovalStore 結構

```rust
// codex-rs/core/src/tools/sandboxing.rs:35-58
#[derive(Clone, Default, Debug)]
pub(crate) struct ApprovalStore {
    // 使用序列化的鍵做泛型快取
    map: HashMap<String, ReviewDecision>,
}

impl ApprovalStore {
    pub fn get<K>(&self, key: &K) -> Option<ReviewDecision>
    where K: Serialize {
        let s = serde_json::to_string(key).ok()?;
        self.map.get(&s).cloned()
    }

    pub fn put<K>(&mut self, key: K, value: ReviewDecision)
    where K: Serialize {
        if let Ok(s) = serde_json::to_string(&key) {
            self.map.insert(s, value);
        }
    }
}
```

**設計要點**：
- 使用 `serde_json::to_string()` 將任意 `Serialize` 類型序列化為字串鍵
- 這允許不同工具使用不同的鍵類型（命令字串、檔案路徑、工具名稱等）
- `HashMap<String, ReviewDecision>` 提供 O(1) 查找

### 5.2 with_cached_approval 函數

```rust
// codex-rs/core/src/tools/sandboxing.rs:60-112
pub(crate) async fn with_cached_approval<K, F, Fut>(
    services: &SessionServices,
    tool_name: &str,    // 用於遙測
    keys: Vec<K>,       // 多個鍵（apply_patch 可修改多檔案）
    fetch: F,           // 實際詢問使用者的閉包
) -> ReviewDecision
where
    K: Serialize,
    F: FnOnce() -> Fut,
    Fut: Future<Output = ReviewDecision>,
{
    // 防禦性：空鍵直接詢問
    if keys.is_empty() {
        return fetch().await;
    }

    // 步驟 1: 檢查所有鍵是否都已在快取中被批准
    let already_approved = {
        let store = services.tool_approvals.lock().await;
        keys.iter()
            .all(|key| matches!(store.get(key), Some(ReviewDecision::ApprovedForSession)))
    };

    // 步驟 2: 如果全部已批准，跳過詢問
    if already_approved {
        return ReviewDecision::ApprovedForSession;
    }

    // 步驟 3: 執行實際的審批詢問
    let decision = fetch().await;

    // 步驟 4: 記錄遙測
    services.session_telemetry.counter(
        "codex.approval.requested", 1,
        &[("tool", tool_name), ("approved", decision.to_opaque_string())],
    );

    // 步驟 5: 如果批准，儲存每個鍵到快取
    if matches!(decision, ReviewDecision::ApprovedForSession) {
        let mut store = services.tool_approvals.lock().await;
        for key in keys {
            store.put(key, ReviewDecision::ApprovedForSession);
        }
    }

    decision
}
```

### 5.3 審批流程完整圖

```
模型請求執行工具
    │
    ▼
ExecApprovalRequirement 判定
    ├─ Skip { bypass_sandbox: bool }        → 直接執行
    ├─ NeedsApproval { reason }             → 進入審批流程
    └─ Forbidden { reason }                 → 拒絕
         │
         ▼ (NeedsApproval 路徑)
with_cached_approval(services, "shell", keys, || async { ... })
    │
    ├─ 快取命中 (全部 key 已 ApprovedForSession) → 跳過詢問
    │
    ├─ 快取未命中 → fetch()
    │   │
    │   ├─ 發送 EventMsg::ExecApprovalRequest → 外部 (TUI/Telegram)
    │   ├─ 等待 Op::ExecApproval { decision }
    │   │
    │   └─ ReviewDecision:
    │       ├─ Approved          → 單次批准，不快取
    │       ├─ ApprovedForSession → 快取到 ApprovalStore
    │       ├─ Denied            → 拒絕
    │       └─ DeniedForSession  → 拒絕（可快取）
    │
    └─ 返回 ReviewDecision
```

### 5.4 ExecApprovalRequirement 三態

```rust
// codex-rs/core/src/tools/sandboxing.rs:124-143
pub(crate) enum ExecApprovalRequirement {
    /// 不需要審批
    Skip {
        bypass_sandbox: bool,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
    },
    /// 需要審批
    NeedsApproval {
        reason: Option<String>,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
    },
    /// 禁止執行
    Forbidden { reason: String },
}
```

**ExecPolicyAmendment**：當使用者批准某個命令時，系統可以提議一個 Starlark 規則修正案，自動在未來跳過相同類型的命令。這是漸進式信任建立。

### 5.5 AskForApproval 策略

```rust
// codex-rs/protocol/src/protocol.rs
pub enum AskForApproval {
    UnlessTrusted,   // 除非指令被 exec policy 信任
    OnFailure,       // 沙箱失敗時才詢問（已棄用）
    OnRequest,       // 模型決定何時詢問（預設）
    Reject(RejectConfig),  // 自動拒絕
    Never,           // 從不詢問（--yolo 模式）
}
```

### 5.6 安全命令分類

```rust
// codex-rs/shell-command/ crate
pub fn is_safe_command(cmd: &str) -> bool;
    // ls, cat, head, tail, wc, grep, find, echo, pwd, ...
pub fn is_dangerous_command(cmd: &str) -> bool;
    // rm -rf, chmod 777, dd, mkfs, curl | bash, ...
pub fn parse_command(cmd: &str) -> ParsedCommand;
    // 解析命令為結構化表示
```

> **Clawtex 實作建議**：
> 1. clawtex 的 `approval.rs` (Telegram 人在環路) 應加入 `ApprovalStore` 快取。目前每次 shell 執行都要等 Telegram 回應。
> 2. 實作 `HashMap<String, ReviewDecision>` 的會話級快取。鍵建議用 `format!("{tool}:{canonicalized_command}")` 做正規化。
> 3. 引入 `ExecApprovalRequirement` 三態（Skip/NeedsApproval/Forbidden），替代現有的 bool 型審批。
> 4. 參考 Codex 的 `is_safe_command()` 白名單，為 clawtex 的 shell allowlist 添加自動免審批。
> 5. `proposed_execpolicy_amendment` 的漸進式信任概念可應用於 clawtex 的 `agents.toml` — 使用者批准後自動更新配置。

---

## 6. Internal→External 事件映射（深度）

### 6.1 問題背景

Codex 內部有 **50+ 種 EventMsg**（用於代理迴圈的各個階段），但外部 API 只暴露 **8 種 ThreadEvent**。這需要一個穩定的映射層。

### 6.2 內部事件 EventMsg（50+ 種）

```rust
// codex-rs/protocol/src/protocol.rs (概念化列舉)
pub enum EventMsg {
    // --- 工作階段 ---
    SessionConfigured(SessionConfiguredEvent),
    ThreadNameUpdated(ThreadNameUpdatedEvent),

    // --- 代理回應 ---
    AgentMessage(AgentMessageEvent),
    AgentMessageContentDelta(AgentMessageContentDeltaEvent),
    AgentReasoning(AgentReasoningEvent),
    AgentReasoningSectionBreak(AgentReasoningSectionBreakEvent),
    ReasoningContentDelta(ReasoningContentDeltaEvent),
    ReasoningRawContentDelta(ReasoningRawContentDeltaEvent),

    // --- 命令執行 ---
    ExecCommandBegin(ExecCommandBeginEvent),
    ExecCommandEnd(ExecCommandEndEvent),
    ExecCommandOutputDelta(ExecCommandOutputDeltaEvent),
    TerminalInteraction(TerminalInteractionEvent),

    // --- 檔案修改 ---
    PatchApplyBegin(PatchApplyBeginEvent),
    PatchApplyEnd(PatchApplyEndEvent),

    // --- MCP ---
    McpToolCallBegin(McpToolCallBeginEvent),
    McpToolCallEnd(McpToolCallEndEvent),

    // --- 協作 ---
    CollabAgentSpawnBegin(CollabAgentSpawnBeginEvent),
    CollabAgentSpawnEnd(CollabAgentSpawnEndEvent),
    CollabAgentInteractionBegin(CollabAgentInteractionBeginEvent),
    CollabAgentInteractionEnd(CollabAgentInteractionEndEvent),
    CollabWaitingBegin(CollabWaitingBeginEvent),
    CollabWaitingEnd(CollabWaitingEndEvent),
    CollabCloseBegin(CollabCloseBeginEvent),
    CollabCloseEnd(CollabCloseEndEvent),

    // --- 搜尋 ---
    WebSearchBegin(WebSearchBeginEvent),
    WebSearchEnd(WebSearchEndEvent),

    // --- 計畫 ---
    PlanUpdate(PlanUpdateEvent),
    PlanDelta(PlanDeltaEvent),

    // --- Turn 生命週期 ---
    TurnStarted(TurnStartedEvent),
    TurnComplete(TurnCompleteEvent),
    TurnAborted(TurnAbortedEvent),

    // --- 項目 ---
    ItemStarted(ItemStartedEvent),
    ItemCompleted(ItemCompletedEvent),
    RawResponseItem(RawResponseItemEvent),

    // --- 審批 ---
    ExecApprovalRequest(ExecApprovalRequestEvent),
    PatchApprovalRequest(ApplyPatchApprovalRequestEvent),
    RequestPermissions(RequestPermissionsEvent),
    RequestUserInput(RequestUserInputEvent),

    // --- 狀態 ---
    TokenCount(TokenCountEvent),
    TurnDiff(TurnDiffEvent),
    ModelReroute(ModelRerouteEvent),
    DeprecationNotice(DeprecationNoticeEvent),

    // --- 錯誤 ---
    Error(ErrorEvent),
    Warning(WarningEvent),
    StreamError(StreamErrorEvent),

    // --- 背景 ---
    BackgroundEvent(BackgroundEventEvent),

    // ... 共 50+ 種
}
```

### 6.3 外部事件 ThreadEvent（8 種）

```rust
// codex-rs/exec/src/exec_events.rs:9-37
pub enum ThreadEvent {
    #[serde(rename = "thread.started")]
    ThreadStarted(ThreadStartedEvent),

    #[serde(rename = "turn.started")]
    TurnStarted(TurnStartedEvent),

    #[serde(rename = "turn.completed")]
    TurnCompleted(TurnCompletedEvent),

    #[serde(rename = "turn.failed")]
    TurnFailed(TurnFailedEvent),

    #[serde(rename = "item.started")]
    ItemStarted(ItemStartedEvent),

    #[serde(rename = "item.updated")]
    ItemUpdated(ItemUpdatedEvent),

    #[serde(rename = "item.completed")]
    ItemCompleted(ItemCompletedEvent),

    #[serde(rename = "error")]
    Error(ThreadErrorEvent),
}
```

### 6.4 ThreadItemDetails（9 種語義類型）

```rust
// codex-rs/exec/src/exec_events.rs:100-128
pub enum ThreadItemDetails {
    AgentMessage(AgentMessageItem),        // 文字回應
    Reasoning(ReasoningItem),              // 推理摘要
    CommandExecution(CommandExecutionItem), // 命令執行
    FileChange(FileChangeItem),            // 檔案變更
    McpToolCall(McpToolCallItem),          // MCP 工具
    CollabToolCall(CollabToolCallItem),     // 協作工具
    WebSearch(WebSearchItem),              // 網頁搜尋
    TodoList(TodoListItem),                // 待辦清單
    Error(ErrorItem),                      // 錯誤
}
```

### 6.5 EventProcessorWithJsonOutput 映射表

```rust
// codex-rs/exec/src/event_processor_with_jsonl_output.rs:119-191
impl EventProcessorWithJsonOutput {
    pub fn collect_thread_events(&mut self, event: &protocol::Event) -> Vec<ThreadEvent> {
        match &event.msg {
            // --- 一對一映射 ---
            SessionConfigured(ev) → [ThreadStarted]
            TurnStarted(ev) → [TurnStarted]
            Error(ev) → [Error]
            StreamError(ev) → [Error]
            Warning(ev) → [ItemCompleted(Error)]

            // --- 聚合映射（多個內部事件 → 一個外部 Item）---
            ExecCommandBegin(ev) → [ItemStarted(CommandExecution)]
              // 追蹤到 running_commands HashMap
            ExecCommandOutputDelta(ev) → [ItemUpdated(CommandExecution)]
              // 聚合到 running_commands[call_id].aggregated_output
            ExecCommandEnd(ev) → [ItemCompleted(CommandExecution)]
              // 從 running_commands 移除，計算 exit_code

            // --- Begin/End 配對 ---
            McpToolCallBegin → [ItemStarted(McpToolCall)]
            McpToolCallEnd → [ItemCompleted(McpToolCall)]

            PatchApplyBegin → 存入 running_patch_applies
            PatchApplyEnd → [ItemCompleted(FileChange)]

            WebSearchBegin → 存入 running_web_search_calls
            WebSearchEnd → [ItemStarted + ItemCompleted(WebSearch)]

            // --- 串流聚合 ---
            AgentMessage(ev) → [ItemStarted(AgentMessage)]
              // 文字增量累積
            AgentReasoning(ev) → [ItemStarted(Reasoning)]

            // --- 計畫管理 ---
            PlanUpdate(ev) → [ItemStarted/ItemUpdated/ItemCompleted(TodoList)]

            // --- Collab 複合映射 ---
            CollabAgentSpawnBegin → [ItemStarted(CollabToolCall)]
            CollabAgentSpawnEnd → 更新狀態
            CollabAgentInteractionBegin → [ItemUpdated(CollabToolCall)]
            CollabAgentInteractionEnd → 更新狀態
            CollabWaitingBegin → [ItemUpdated(CollabToolCall)]
            CollabWaitingEnd → 更新狀態
            CollabCloseBegin → [ItemUpdated(CollabToolCall)]
            CollabCloseEnd → [ItemCompleted(CollabToolCall)]

            // --- 無對應映射（靜默丟棄）---
            ThreadNameUpdated → []
            TokenCount → [] (僅更新內部 last_total_token_usage)
            _ → []
        }
    }
}
```

### 6.6 狀態追蹤 HashMap

```rust
// codex-rs/exec/src/event_processor_with_jsonl_output.rs:59-73
pub struct EventProcessorWithJsonOutput {
    running_commands: HashMap<String, RunningCommand>,      // 進行中的命令
    running_patch_applies: HashMap<String, PatchApplyBeginEvent>,  // 進行中的 patch
    running_todo_list: Option<RunningTodoList>,              // 當前 turn 的 TodoList
    last_total_token_usage: Option<TokenUsage>,              // 累積 token 用量
    running_mcp_tool_calls: HashMap<String, RunningMcpToolCall>,  // 進行中的 MCP 呼叫
    running_collab_tool_calls: HashMap<String, RunningCollabToolCall>,  // 協作呼叫
    running_web_search_calls: HashMap<String, String>,       // 搜尋呼叫
    last_critical_error: Option<ThreadErrorEvent>,           // 最後一個錯誤
    next_event_id: AtomicU64,                                // 自增 ID 產生器
}
```

### 6.7 映射設計原則

1. **穩定的公開 API**：外部只有 8 種事件 + 9 種 ItemDetails，不會因內部重構而改變
2. **有狀態映射**：Begin/End 配對需要 HashMap 追蹤中間狀態
3. **聚合模式**：多個 OutputDelta 聚合為一個 CommandExecution 的最終輸出
4. **靜默丟棄**：不相關的內部事件（如 TokenCount, ThreadNameUpdated）不產生外部事件
5. **自增 ID**：使用 AtomicU64 產生穩定的外部 item_id

### 6.8 典型 JSONL 輸出序列

```jsonl
{"type":"thread.started","thread_id":"abc-123"}
{"type":"turn.started"}
{"type":"item.started","item":{"id":"item_0","type":"agent_message","text":""}}
{"type":"item.updated","item":{"id":"item_0","type":"agent_message","text":"Let me "}}
{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"Let me check..."}}
{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"ls -la","status":"in_progress"}}
{"type":"item.updated","item":{"id":"item_1","type":"command_execution","command":"ls -la","status":"in_progress","output":"..."}}
{"type":"item.completed","item":{"id":"item_1","type":"command_execution","command":"ls -la","status":"completed","exit_code":0,"output":"total 42..."}}
{"type":"turn.completed","usage":{"input_tokens":1234,"cached_input_tokens":500,"output_tokens":200}}
```

> **Clawtex 實作建議**：
> 1. clawtex-core 的 `agent_events` 應建立類似的雙層事件系統：內部 `AgentEventMsg` (精細) → 外部 `AgentStreamEvent` (穩定)。
> 2. 目前 clawtex 的 SSE 端點直接暴露內部結構。引入映射層可讓 HTTP API 和 Telegram 介面各自消費不同的外部事件集。
> 3. 有狀態映射器 (`HashMap<String, RunningCommand>`) 是處理 Begin/End 配對的正確模式。
> 4. `AtomicU64` 自增 ID 比 UUID 更輕量，適合高頻事件。

---

## 7. 工具系統與並行執行

### 7.1 ToolRouter

```rust
// codex-rs/core/src/tools/router.rs
pub struct ToolRouter {
    registry: ToolRegistry,
    specs: Vec<ConfiguredToolSpec>,
}

impl ToolRouter {
    pub fn from_config(config: &ToolsConfig, params: ToolRouterParams<'_>) -> Self;
    pub fn specs(&self) -> Vec<ToolSpec>;
    pub async fn build_tool_call(
        session: &Session, item: ResponseItem,
    ) -> Result<Option<ToolCall>, FunctionCallError>;
    pub fn tool_supports_parallel(&self, tool_name: &str) -> bool;
    pub async fn dispatch_tool_call(
        session: Arc<Session>, turn: Arc<TurnContext>,
        tracker: SharedTurnDiffTracker, call: ToolCall, source: ToolCallSource,
    ) -> Result<ResponseInputItem, FunctionCallError>;
}
```

### 7.2 並行執行引擎 — ToolCallRuntime

```rust
// codex-rs/core/src/tools/parallel.rs:24-31
pub(crate) struct ToolCallRuntime {
    router: Arc<ToolRouter>,
    session: Arc<Session>,
    turn_context: Arc<TurnContext>,
    tracker: SharedTurnDiffTracker,
    parallel_execution: Arc<RwLock<()>>,  // 讀寫鎖控制並行度
}
```

**並行控制策略**：

```rust
// codex-rs/core/src/tools/parallel.rs:80-98
// 使用 RwLock 精巧地控制並行：
// - 支持並行的工具（如 grep, ls）：取 read lock → 可同時多個
// - 不支持並行的工具（如 shell, apply_patch）：取 write lock → 獨佔
let _guard = if supports_parallel {
    Either::Left(lock.read().await)   // 共享讀鎖
} else {
    Either::Right(lock.write().await) // 獨佔寫鎖
};
```

**取消處理**：

```rust
// codex-rs/core/src/tools/parallel.rs:72-99
tokio::select! {
    _ = cancellation_token.cancelled() => {
        // 中斷時產生 abort 回應
        Ok(Self::aborted_response(&call, secs))
    },
    res = async {
        let _guard = if supports_parallel { ... } else { ... };
        router.dispatch_tool_call(session, turn, tracker, call, source).await
    } => res,
}
```

**中斷回應格式化**（按工具類型不同）：

```rust
// codex-rs/core/src/tools/parallel.rs:116-150
fn aborted_response(call: &ToolCall, secs: f32) -> ResponseInputItem {
    match &call.payload {
        ToolPayload::Custom { .. } => CustomToolCallOutput { ... },
        ToolPayload::ToolSearch { .. } => ToolSearchOutput { tools: Vec::new() },
        ToolPayload::Mcp { .. } => McpToolCallOutput { ... },
        _ => FunctionCallOutput { ... },
    }
}

fn abort_message(call: &ToolCall, secs: f32) -> String {
    match call.tool_name.as_str() {
        "shell" | "container.exec" | "local_shell" | "unified_exec" => {
            format!("Wall time: {secs:.1} seconds\naborted by user")
        }
        _ => format!("Tool call aborted after {secs:.1}s")
    }
}
```

### 7.3 工具分類

| 工具 | Handler 檔案 | 支援並行 |
|------|-------------|---------|
| shell / unified_exec | `handlers/shell.rs`, `handlers/unified_exec.rs` | No |
| apply_patch | `handlers/apply_patch.rs` | No |
| read_file | `handlers/read_file.rs` | Yes |
| grep_files | `handlers/grep_files.rs` | Yes |
| list_dir | `handlers/list_dir.rs` | Yes |
| mcp | `handlers/mcp.rs` | Yes |
| code_mode (JS REPL) | `handlers/code_mode.rs` | No |
| view_image | `handlers/view_image.rs` | Yes |
| tool_search | `handlers/tool_search.rs` | Yes |
| tool_suggest | `handlers/tool_suggest.rs` | Yes |
| multi_agents | `handlers/multi_agents.rs` | No |
| plan | `handlers/plan.rs` | Yes |
| request_user_input | `handlers/request_user_input.rs` | No |
| request_permissions | `handlers/request_permissions.rs` | No |
| artifacts | `handlers/artifacts.rs` | Yes |

> **Clawtex 實作建議**：
> 1. clawtex-core 的工具目前是循序的。引入 `RwLock<()>` 並行控制是最小改動方案。
> 2. 標記 clawtex 的 24 工具中哪些是唯讀（file_read, glob, content_search, memory_recall）→ 可並行。
> 3. 副作用工具（shell, file_write, file_edit）→ 獨佔鎖。
> 4. `AbortOnDropHandle` 確保取消時工具 task 自動清理，防止孤兒 task。

---

## 8. 上下文管理與自動壓縮

### 8.1 ContextManager

```rust
// codex-rs/core/src/context_manager/history.rs
pub(crate) struct ContextManager {
    items: Vec<ResponseItem>,
    token_info: Option<TokenUsageInfo>,
    reference_context_item: Option<TurnContextItem>,
}

impl ContextManager {
    pub(crate) fn record_items<I>(&mut self, items: I, policy: TruncationPolicy);
    pub(crate) fn for_prompt(mut self, modalities: &[InputModality]) -> Vec<ResponseItem> {
        self.normalize_history(modalities);
        self.items.retain(|item| !matches!(item, ResponseItem::GhostSnapshot { .. }));
        self.items
    }
}
```

### 8.2 自動壓縮

```rust
// codex-rs/core/src/compact.rs
pub const SUMMARIZATION_PROMPT: &str = include_str!("../templates/compact/prompt.md");

pub(crate) async fn run_inline_auto_compact_task(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    initial_context_injection: InitialContextInjection,
) -> CodexResult<()> {
    // 使用模型壓縮歷史
}

pub enum InitialContextInjection {
    BeforeLastUserMessage,  // mid-turn 壓縮
    DoNotInject,            // pre-turn / manual 壓縮
}
```

### 8.3 遠端壓縮

```rust
// codex-rs/core/src/compact_remote.rs
pub(crate) async fn run_inline_remote_auto_compact_task(...) -> CodexResult<()> {
    // OpenAI 提供者使用遠端壓縮 API
    // 本地模型使用 run_inline_auto_compact_task
}
```

### 8.4 截斷策略

```rust
// codex-rs/core/src/truncate.rs
pub(crate) enum TruncationPolicy {
    // 智慧截斷大型工具輸出
    // 保留開頭和結尾，截斷中間
}

// exec.rs 中的截斷常數
const EXEC_OUTPUT_MAX_BYTES: usize = DEFAULT_OUTPUT_BYTES_CAP;
```

> **Clawtex 實作建議**：
> 1. clawtex 的 `context_compactor.rs` 可借鏡 Codex 的分層壓縮策略（pre-turn vs mid-turn）。
> 2. 引入 `InitialContextInjection` 概念，讓壓縮後的摘要在正確的位置注入。
> 3. 遠端壓縮是一個好概念 — 讓 cloud provider 做壓縮比本地 LLM 更高效。

---

## 9. Provider 整合與認證

### 9.1 認證系統

```rust
// codex-rs/core/src/auth.rs
pub enum CodexAuth {
    ApiKey(ApiKeyAuth),              // OPENAI_API_KEY
    Chatgpt(ChatgptAuth),            // ChatGPT OAuth
    ChatgptAuthTokens(ChatgptAuthTokens),
}

pub enum AuthMode {
    ApiKey,    // API Key → api.openai.com
    Chatgpt,   // ChatGPT → chatgpt.com
}
```

### 9.2 模型客戶端

```rust
// codex-rs/core/src/client.rs
// ModelClient: session 範圍
// ModelClientSession: 每個 turn，快取 WebSocket
```

支援的 API：
- **Responses API** (主要)
- **WebSocket** (即時連線 + prewarm)
- **SSE** (串流回應)

### 9.3 OSS Provider

```rust
// codex-rs/ollama/ + codex-rs/lmstudio/
pub const OLLAMA_OSS_PROVIDER_ID: &str = "ollama";
pub const LMSTUDIO_OSS_PROVIDER_ID: &str = "lmstudio";
```

---

## 10. 錯誤處理與韌性設計

### 10.1 錯誤分類

```rust
// codex-rs/core/src/error.rs
pub enum CodexErr {
    Fatal(String),     // 不可恢復
    Sandbox(SandboxErr), // 沙箱錯誤
    // + 其他變體
}

pub enum SandboxErr {
    MissingExecutable,
    TransformFailed(SandboxTransformError),
    ExecFailed(std::io::Error),
}
```

### 10.2 指數退避

```rust
// codex-rs/core/src/util.rs
pub(crate) async fn backoff::exponential_backoff(attempt: u32) {
    // 基本延遲 * 2^attempt + jitter
}
```

### 10.3 I/O 清理超時

```rust
// codex-rs/core/src/exec.rs:67-73
// 命令執行後等待 stdout/stderr 清理
// 防止孫程序繼承 fd 導致永久掛起
pub const IO_DRAIN_TIMEOUT_MS: u64 = 2_000;
```

### 10.4 輸出限制

```rust
const EXEC_OUTPUT_MAX_BYTES: usize = DEFAULT_OUTPUT_BYTES_CAP;
pub(crate) const MAX_EXEC_OUTPUT_DELTAS_PER_CALL: usize = 10_000;
// 防止單一命令 OOM
```

> **Clawtex 實作建議**：
> 1. clawtex 的 shell tool 應加入 `IO_DRAIN_TIMEOUT_MS` — 防止孫程序掛起。
> 2. `MAX_EXEC_OUTPUT_DELTAS_PER_CALL` 限制串流事件數是好的防禦 — clawtex 應引入類似機制。
> 3. Codex 的 `EXEC_OUTPUT_MAX_BYTES` 防止 OOM — clawtex 的 shell tool 需要加入輸出上限。

---

## 11. 效能分析

### 11.1 通道容量設計

| 通道 | 類型 | 容量 | 原因 |
|------|------|------|------|
| Submission (SQ) | bounded | 512 | 防止外部過載代理 |
| Event (EQ) | unbounded | 無限 | 不阻塞代理迴圈 |
| app-server | bounded | 128 | WebSocket 背壓 |

### 11.2 並行工具執行

使用 `RwLock<()>` 的讀寫分離提供了優秀的並行效能：
- N 個唯讀工具可同時執行
- 寫入工具獨佔但不阻塞後續唯讀工具

### 11.3 WebSocket Prewarm

```rust
// ModelClientSession 每個 turn 建立，支援 WebSocket prewarm
// 在 turn 開始前就建立連線，減少首 token 延遲
```

### 11.4 Atomic ID 產生

```rust
next_event_id: AtomicU64
// O(1) lock-free ID 產生，比 UUID 快 100x
```

---

## 12. Clawtex 差距總覽與實作路線圖

### 12.1 差距矩陣

| 功能 | Codex CLI | Clawtex | 差距程度 | 優先級 |
|------|-----------|---------|---------|--------|
| SQ/EQ 解耦 | 完整實作 | 直接呼叫 | **高** | P0 |
| 三平台沙箱 | Linux+macOS+Windows | 基本路徑隔離 | **高** | P1 |
| ApprovalStore 快取 | HashMap 快取 | 每次 Telegram 詢問 | **中** | P0 |
| Internal→External 映射 | 50+→8 映射層 | 直接暴露 | **中** | P1 |
| 並行工具執行 | RwLock 策略 | 循序 | **中** | P1 |
| 上下文壓縮 | 分層壓縮+遠端 | 基本壓縮 | **低** | P2 |
| CancellationToken 樹 | 三層取消 | AtomicBool e-stop | **中** | P1 |
| W3C TraceContext | 完整 | 無 | **低** | P3 |
| 命令安全分類 | is_safe/dangerous | 白名單 | **低** | P2 |
| WebSocket Prewarm | 支援 | 無 | **低** | P3 |

### 12.2 建議實作順序

```
Sprint 1 (1 週): 基礎架構
  ├── P0: ApprovalStore 快取 → src/approval.rs
  ├── P0: SQ/EQ 通道分離 → src/agent_runtime.rs
  └── P1: CancellationToken 替換 e-stop

Sprint 2 (1 週): 效能與安全
  ├── P1: 並行工具執行 (RwLock)
  ├── P1: Internal→External 事件映射
  └── P1: Windows Restricted Token (基本)

Sprint 3 (1 週): 進階
  ├── P2: 上下文分層壓縮
  ├── P2: 命令安全分類
  └── P3: W3C TraceContext
```

---

## 13. 多代理控制系統（深度）

Codex CLI 的多代理系統是整個架構中最精密的子系統之一，包含代理生成、守衛限制、暱稱管理、歷史分叉四大機制。

### 13.1 AgentControl — 控制平面

```rust
// codex-rs/core/src/agent/control.rs:62-75
/// Control-plane handle for multi-agent operations.
/// `AgentControl` is held by each session (via `SessionServices`). It provides capability to
/// spawn new agents and the inter-agent communication layer.
/// An `AgentControl` instance is shared per "user session" which means the same `AgentControl`
/// is used for every sub-agent spawned by Codex.
#[derive(Clone, Default)]
pub(crate) struct AgentControl {
    /// Weak handle back to the global thread registry/state.
    /// This is `Weak` to avoid reference cycles and shadow persistence of the form
    /// `ThreadManagerState -> CodexThread -> Session -> SessionServices -> ThreadManagerState`.
    manager: Weak<ThreadManagerState>,
    state: Arc<Guards>,
}
```

關鍵設計：
- 使用 `Weak<ThreadManagerState>` 避免參考循環 — 防止記憶體洩漏
- `Arc<Guards>` 在所有子代理間共享 — 保證全域限制
- 每個「使用者 session」共用一個 `AgentControl`

### 13.2 Guards — 資源守衛

```rust
// codex-rs/core/src/agent/guards.rs:14-24
/// This structure is used to add some limits on the multi-agent capabilities for Codex.
/// In the current implementation, it limits:
/// * Total number of sub-agents (i.e. threads) per user session
#[derive(Default)]
pub(crate) struct Guards {
    active_agents: Mutex<ActiveAgents>,
    total_count: AtomicUsize,
}

#[derive(Default)]
struct ActiveAgents {
    threads_set: HashSet<ThreadId>,
    thread_agent_nicknames: HashMap<ThreadId, String>,
    used_agent_nicknames: HashSet<String>,
    nickname_reset_count: usize,
}
```

**守衛機制的核心邏輯**：

```
spawn_agent()
  │
  ├── reserve_spawn_slot(max_threads)  ─→ 如果超出限制：CodexErr::AgentLimitReached
  │     └── AtomicUsize::fetch_add(1, AcqRel)  ← 原子操作，無鎖
  │
  ├── reserve_agent_nickname(candidates)
  │     ├── 過濾已使用的暱稱
  │     ├── 如果耗盡：清空 used_nicknames + 增加 reset_count
  │     │     └── 下一輪暱稱格式："Alice the 2nd", "Bob the 3rd"
  │     └── OTel 指標: codex.multi_agent.nickname_pool_reset
  │
  └── fork_thread_with_source() 或 spawn_new_thread_with_source()
```

### 13.3 歷史分叉（Fork）機制

```rust
// codex-rs/core/src/agent/control.rs:134-182
// 分叉代理的歷史繼承流程
if let Some(call_id) = options.fork_parent_spawn_call_id.as_ref() {
    // 1. 確保父代理的 rollout 已持久化
    parent_thread.codex.session.ensure_rollout_materialized().await;
    parent_thread.codex.session.flush_rollout().await;

    // 2. 取得父代理的完整歷史
    let forked_rollout_items = RolloutRecorder::get_rollout_history(&rollout_path)
        .await?.get_rollout_items();

    // 3. 附加分叉標記
    forked_rollout_items.push(RolloutItem::ResponseItem(
        ResponseItem::FunctionCallOutput {
            call_id: call_id.clone(),
            output: FunctionCallOutputPayload::from_text(
                FORKED_SPAWN_AGENT_OUTPUT_MESSAGE.to_string()
            ),
        },
    ));

    // 4. 以 Forked 模式初始化
    let initial_history = InitialHistory::Forked(forked_rollout_items);
    state.fork_thread_with_source(config, initial_history, ...).await?
}
```

**Fork vs Spawn 比較**：

| 特性 | Fork | Spawn |
|------|------|-------|
| 歷史繼承 | 完整複製父代理歷史 | 空白起始 |
| 用途 | 分支探索、平行嘗試 | 獨立子任務 |
| rollout | 需要先 flush 父代理 | 無前置操作 |
| JSONL 持久化 | 包含父代理所有歷史 | 僅包含自身 |

### 13.4 深度控制

```rust
// codex-rs/core/src/agent/guards.rs:53-67
fn session_depth(session_source: &SessionSource) -> i32 {
    match session_source {
        SessionSource::SubAgent(SubAgentSource::ThreadSpawn { depth, .. }) => *depth,
        SessionSource::SubAgent(_) => 0,
        _ => 0,
    }
}

pub(crate) fn next_thread_spawn_depth(session_source: &SessionSource) -> i32 {
    session_depth(session_source).saturating_add(1)  // saturating 防止 i32 溢出
}

pub(crate) fn exceeds_thread_spawn_depth_limit(depth: i32, max_depth: i32) -> bool {
    depth > max_depth
}
```

**多層防禦**：
1. `max_threads` — 限制總代理數量
2. `max_depth` — 限制代理嵌套深度（防止遞歸爆炸）
3. `saturating_add` — 防止整數溢出（Rust 安全慣例）
4. 暱稱耗盡重置 — 優雅降級而非崩潰

> **Clawtex 實作建議**：
> 1. clawtex 的 `delegate` 工具目前沒有深度限制 — 應引入 `next_spawn_depth` + `max_depth` 檢查。
> 2. `Weak<T>` 模式避免參考循環 — clawtex 的 `cluster_hub.rs` 應使用類似模式。
> 3. 暱稱系統可用於 clawtex 的 Telegram 多代理對話 — 讓使用者區分不同子代理的回覆。
> 4. Fork 歷史繼承可用於 clawtex 的「繼續上次對話」功能 — 不需重新載入完整 context。

---

## 14. CodexThread 抽象層（深度）

### 14.1 Thread 作為對話容器

```rust
// codex-rs/core/src/codex_thread.rs:30-49
#[derive(Clone, Debug)]
pub struct ThreadConfigSnapshot {
    pub model: String,
    pub model_provider_id: String,
    pub service_tier: Option<ServiceTier>,
    pub approval_policy: AskForApproval,
    pub sandbox_policy: SandboxPolicy,
    pub cwd: PathBuf,
    pub ephemeral: bool,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub personality: Option<Personality>,
    pub session_source: SessionSource,
}

pub struct CodexThread {
    pub(crate) codex: Codex,
    rollout_path: Option<PathBuf>,
    out_of_band_elicitation_count: Mutex<u64>,
    _watch_registration: WatchRegistration,
}
```

`ThreadConfigSnapshot` 的設計是一個「不可變快照」模式 — 在任何時間點取得完整的 thread 設定，不需要持有鎖。

### 14.2 帶外引出（Out-of-Band Elicitation）

```rust
// codex-rs/core/src/codex_thread.rs:164-197
pub async fn increment_out_of_band_elicitation_count(&self) -> CodexResult<u64> {
    let mut guard = self.out_of_band_elicitation_count.lock().await;
    let was_zero = *guard == 0;
    *guard = guard.checked_add(1).ok_or_else(|| {
        CodexErr::Fatal("out-of-band elicitation count overflowed".to_string())
    })?;
    if was_zero {
        self.codex.session.set_out_of_band_elicitation_pause_state(true);
    }
    Ok(*guard)
}

pub async fn decrement_out_of_band_elicitation_count(&self) -> CodexResult<u64> {
    let mut guard = self.out_of_band_elicitation_count.lock().await;
    if *guard == 0 {
        return Err(CodexErr::InvalidRequest(
            "out-of-band elicitation count is already zero".to_string(),
        ));
    }
    *guard -= 1;
    let now_zero = *guard == 0;
    if now_zero {
        self.codex.session.set_out_of_band_elicitation_pause_state(false);
    }
    Ok(*guard)
}
```

**帶外引出機制的用途**：
- 外部系統（如 UI、MCP client）需要在代理回合中途注入訊息
- 計數器 > 0 時暫停代理處理，直到所有帶外引出完成
- `checked_add` 防止 u64 溢出（極端情況下的安全防護）

### 14.3 消息注入（不建立新回合）

```rust
// codex-rs/core/src/codex_thread.rs:122-146
/// Records a user-role session-prefix message without creating a new user turn boundary.
pub(crate) async fn inject_user_message_without_turn(&self, message: String) {
    let pending_item = ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputText { text: message }],
    };
    let pending_items = vec![pending_item];
    let Err(items_without_active_turn) = self
        .codex.session.inject_response_items(pending_items).await
    else {
        return;
    };
    // 如果沒有活躍回合，建立一個預設回合來記錄
    let turn_context = self.codex.session.new_default_turn().await;
    let items: Vec<ResponseItem> = items_without_active_turn
        .into_iter().map(ResponseItem::from).collect();
    self.codex.session
        .record_conversation_items(turn_context.as_ref(), &items).await;
}
```

**關鍵設計**：
- 優先嘗試注入到活躍回合
- 如果失敗（沒有活躍回合），建立新的「預設回合」來記錄
- 使用 `Err` 回傳未注入的項目 — 優雅的失敗回退模式

> **Clawtex 實作建議**：
> 1. clawtex 的 `agent_runtime.rs` 缺乏 `ThreadConfigSnapshot` — 導致讀取設定時需要鎖。
> 2. 帶外引出機制可用於 clawtex 的 Telegram 人工介入 — 暫停代理直到使用者回覆。
> 3. `inject_user_message_without_turn` 可用於 clawtex 的系統通知 — 如 cron 觸發的背景資訊注入。

---

## 15. 上下文壓縮引擎（深度）

### 15.1 壓縮策略分層

```rust
// codex-rs/core/src/compact.rs:36-48
/// Controls whether compaction replacement history must include initial context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InitialContextInjection {
    BeforeLastUserMessage,  // mid-turn: 壓縮後在最後使用者訊息前注入初始上下文
    DoNotInject,            // pre-turn: 壓縮後清空 reference_context_item，
                            //           下一個正常回合會完整重新注入
}
```

**兩種壓縮時機**：

| 時機 | 觸發條件 | 初始上下文處理 | 函式 |
|------|---------|--------------|------|
| Pre-turn | 使用者手動觸發 | DoNotInject（下次回合重新注入） | `run_compact_task` |
| Mid-turn | Token 超出限制 | BeforeLastUserMessage（立即注入） | `run_inline_auto_compact_task` |

### 15.2 壓縮重試與降級

```rust
// codex-rs/core/src/compact.rs:116-189
loop {
    let attempt_result = drain_to_completed(
        &sess, turn_context.as_ref(), &mut client_session,
        turn_metadata_header.as_deref(), &prompt,
    ).await;

    match attempt_result {
        Ok(()) => {
            if truncated_count > 0 {
                sess.notify_background_event(
                    turn_context.as_ref(),
                    format!("Trimmed {truncated_count} older thread item(s) \
                             before compacting so the prompt fits the model context window."),
                ).await;
            }
            break;
        }
        Err(CodexErr::ContextWindowExceeded) => {
            if turn_input_len > 1 {
                // 從頭部移除最舊的歷史項目（保留快取前綴）
                history.remove_first_item();
                truncated_count += 1;
                retries = 0;  // 重置重試計數器！
                continue;
            }
            // 即使只剩一個項目仍然超出，發送錯誤
            sess.set_total_tokens_full(turn_context.as_ref()).await;
            return Err(e);
        }
        Err(e) => {
            if retries < max_retries {
                retries += 1;
                let delay = backoff(retries);
                tokio::time::sleep(delay).await;
                continue;
            }
            return Err(e);
        }
    }
}
```

**壓縮降級策略流程圖**：

```
嘗試壓縮
  │
  ├── 成功 → 產生摘要 → 替換歷史
  │
  ├── ContextWindowExceeded
  │     ├── 歷史 > 1 項 → 移除最舊項目 → 重試（重置計數器）
  │     └── 歷史 = 1 項 → 發送 Error → 設置 total_tokens_full
  │
  └── 其他錯誤
        ├── retries < max_retries → 指數退避 → 重試
        └── 超出重試次數 → 發送 Error
```

### 15.3 壓縮摘要構建

```rust
// codex-rs/core/src/compact.rs:191-197
let history_snapshot = sess.clone_history().await;
let history_items = history_snapshot.raw_items();
let summary_suffix = get_last_assistant_message_from_turn(history_items)
    .unwrap_or_default();
let summary_text = format!("{SUMMARY_PREFIX}\n{summary_suffix}");
let user_messages = collect_user_messages(history_items);
let mut new_history = build_compacted_history(Vec::new(), &user_messages, &summary_text);
```

**壓縮後歷史的結構**：

```
[壓縮前]                           [壓縮後]
├── user: "建立 API server"         ├── summary: "使用者要求建立 API server，
├── assistant: "好的，我來..."       │            已完成路由定義、middleware..."
├── tool_call: shell("npm init")    ├── user: "加入驗證"     ← 保留使用者訊息
├── tool_result: ...                └── user: "修復 bug #42" ← 保留使用者訊息
├── assistant: "已完成..."
├── user: "加入驗證"
├── assistant: "..."
├── tool_call: ...
├── tool_result: ...
├── user: "修復 bug #42"
```

### 15.4 摘要提示詞

```rust
// codex-rs/core/src/compact.rs:31
pub const SUMMARIZATION_PROMPT: &str = include_str!("../templates/compact/prompt.md");
pub const SUMMARY_PREFIX: &str = include_str!("../templates/compact/summary_prefix.md");
const COMPACT_USER_MESSAGE_MAX_TOKENS: usize = 20_000;
```

使用 `include_str!` 在編譯時嵌入模板 — 零運行時 I/O 成本。

### 15.5 遠端壓縮（Remote Compact）

```rust
// codex-rs/core/src/compact.rs:50-52
pub(crate) fn should_use_remote_compact_task(provider: &ModelProviderInfo) -> bool {
    provider.is_openai()  // 僅 OpenAI 支援遠端壓縮
}
```

遠端壓縮利用 OpenAI 的伺服器端壓縮能力，減少本地計算開銷。

> **Clawtex 實作建議**：
> 1. clawtex 的 `context_optimizer.rs` 應引入 `InitialContextInjection` 分層 — 區分 mid-turn 和 pre-turn 壓縮。
> 2. `ContextWindowExceeded` 的逐步刪除策略（移除最舊項目→重試→重置計數器）優於 clawtex 目前的一次性截斷。
> 3. `COMPACT_USER_MESSAGE_MAX_TOKENS = 20_000` — 壓縮提示自身也有 token 限制，clawtex 應引入類似安全閥。
> 4. `include_str!` 嵌入模板 — clawtex 目前用 `format!` 硬編碼壓縮提示，應改為外部模板。
> 5. 保留使用者訊息（不壓縮）是重要的設計 — 確保壓縮後仍能追蹤使用者意圖鏈。

---

## 16. Codex 結構體與 Session 架構（深度）

### 16.1 Codex 核心結構

```rust
// codex-rs/core/src/codex.rs:329-342
/// The high-level interface to the Codex system.
/// It operates as a queue pair where you send submissions and receive events.
pub struct Codex {
    pub(crate) tx_sub: Sender<Submission>,
    pub(crate) rx_event: Receiver<Event>,
    pub(crate) agent_status: watch::Receiver<AgentStatus>,
    pub(crate) session: Arc<Session>,
    pub(crate) session_loop_termination: SessionLoopTermination,
}

pub(crate) type SessionLoopTermination = Shared<BoxFuture<'static, ()>>;
```

### 16.2 CodexSpawnArgs — 依賴注入

```rust
// codex-rs/core/src/codex.rs:354-370
pub(crate) struct CodexSpawnArgs {
    pub(crate) config: Config,
    pub(crate) auth_manager: Arc<AuthManager>,
    pub(crate) models_manager: Arc<ModelsManager>,
    pub(crate) skills_manager: Arc<SkillsManager>,
    pub(crate) plugins_manager: Arc<PluginsManager>,
    pub(crate) mcp_manager: Arc<McpManager>,
    pub(crate) file_watcher: Arc<FileWatcher>,
    pub(crate) conversation_history: InitialHistory,
    pub(crate) session_source: SessionSource,
    pub(crate) agent_control: AgentControl,
    pub(crate) dynamic_tools: Vec<DynamicToolSpec>,
    pub(crate) persist_extended_history: bool,
    pub(crate) metrics_service_name: Option<String>,
    pub(crate) inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
    pub(crate) parent_trace: Option<W3cTraceContext>,
}
```

**依賴注入圖**：

```
CodexSpawnArgs
├── Config ─────────────── 全域設定
├── Arc<AuthManager> ───── 認證（API key 管理）
├── Arc<ModelsManager> ─── 模型清單與能力
├── Arc<SkillsManager> ─── 技能載入與注入
├── Arc<PluginsManager> ── 外掛管理
├── Arc<McpManager> ────── MCP 連線管理
├── Arc<FileWatcher> ───── 檔案系統監控（inotify/FSEvents/ReadDirectoryChanges）
├── InitialHistory ─────── 初始歷史（Blank | Forked | Continued）
├── SessionSource ──────── 來源標記（User | SubAgent | API）
├── AgentControl ───────── 多代理控制平面
├── Vec<DynamicToolSpec> ── 動態工具（MCP 提供的額外工具）
├── Option<ShellSnapshot>── 繼承的 shell 環境
└── Option<W3cTraceContext>── 分散式追蹤上下文
```

### 16.3 通道容量常數

```rust
// codex-rs/core/src/codex.rs:372-374
pub(crate) const INITIAL_SUBMIT_ID: &str = "";
pub(crate) const SUBMISSION_CHANNEL_CAPACITY: usize = 512;
```

SQ 容量 512 的設計理由：
1. 足夠緩衝快速連續的使用者操作（如粘貼多行指令）
2. 不至於消耗過多記憶體（每個 `Submission` ≈ 1-10KB）
3. 配合 `bounded` channel 提供背壓 — 阻止外部以更快速度灌入

### 16.4 W3C TraceContext 整合

```rust
// codex-rs/core/src/codex.rs:378-399
impl Codex {
    pub(crate) async fn spawn(args: CodexSpawnArgs) -> CodexResult<CodexSpawnOk> {
        let parent_trace = match args.parent_trace {
            Some(trace) => {
                if codex_otel::context_from_w3c_trace_context(&trace).is_some() {
                    Some(trace)
                } else {
                    warn!("ignoring invalid thread spawn trace carrier");
                    None  // 優雅降級：無效的 trace 不阻止代理啟動
                }
            }
            None => None,
        };
        let thread_spawn_span = info_span!("thread_spawn", otel.name = "thread_spawn");
        if let Some(trace) = parent_trace.as_ref() {
            let _ = set_parent_from_w3c_trace_context(&thread_spawn_span, trace);
        }
        Self::spawn_internal(CodexSpawnArgs { parent_trace, ..args })
            .instrument(thread_spawn_span)
    }
}
```

**分散式追蹤的三層設計**：
1. **Thread 層**：`thread_spawn_span` — 追蹤代理生命週期
2. **Turn 層**：每個回合有獨立 span（在 session loop 內）
3. **Tool 層**：每個工具執行有子 span

> **Clawtex 實作建議**：
> 1. clawtex 缺乏結構化的依賴注入 — `agent_runtime.rs` 中的建構函式應改為類似 `CodexSpawnArgs` 的結構體。
> 2. `SessionLoopTermination = Shared<BoxFuture>` 允許多個呼叫者等待關閉 — clawtex 的 `daemon` 應使用類似模式。
> 3. W3C TraceContext 的優雅降級（無效 trace 不阻止啟動）— clawtex 應學習此模式用於 cluster_hub。
> 4. `FileWatcher` 支援跨平台檔案監控 — clawtex 可利用 `notify` crate 實現 workspace 檔案變更偵測。
> 5. `ShellSnapshot` 繼承 — 子代理繼承父代理的 shell 環境，clawtex 的 `delegate` 工具應支援此功能。

---

## 17. 協議與事件系統深入分析

### 17.1 事件類型完整清單

Codex CLI 的內部事件系統包含 50+ 種事件類型。以下是核心分類：

```
EventMsg 類型分布:

代理狀態 (5 種):
  ├── SessionConfigured    — session 初始化完成
  ├── TurnStarted          — 新回合開始
  ├── TurnCompleted        — 回合完成
  ├── AgentStatusChanged   — 代理狀態轉變
  └── SessionCompleted     — session 結束

內容串流 (6 種):
  ├── AgentMessageContentDelta   — 主內容增量
  ├── ReasoningContentDelta      — 推理過程增量
  ├── ReasoningRawContentDelta   — 原始推理數據
  ├── ReasoningSectionBreak      — 推理段落分隔
  ├── PlanDelta                  — 計畫增量
  └── StreamError                — 串流錯誤

工具執行 (8 種):
  ├── ExecApprovalRequest        — 命令審批請求
  ├── ApplyPatchApprovalRequest  — patch 審批請求
  ├── ToolStart                  — 工具開始
  ├── ToolDelta                  — 工具輸出增量
  ├── ToolEnd                    — 工具結束
  ├── TurnDiff                   — 檔案變更差異
  ├── NetworkApprovalRequest     — 網路存取審批
  └── BackgroundEvent            — 背景事件通知

資源管理 (4 種):
  ├── TokenCount                 — token 使用量
  ├── Compacted                  — 壓縮完成
  ├── Warning                    — 警告
  └── Error                      — 錯誤

模型管理 (3 種):
  ├── ModelReroute               — 模型重新路由
  ├── DeprecationNotice          — 棄用通知
  └── RateLimitSnapshot          — 限速快照

使用者互動 (2 種):
  ├── RequestUserInput           — 請求使用者輸入
  └── SkillError                 — 技能錯誤
```

### 17.2 ThreadEvent 外部映射（8 種）

```rust
// codex-rs/exec/src/exec_events.rs
pub enum ThreadEvent {
    MessageCreated { .. },         // 所有內容型事件 → 統一為「訊息建立」
    MessageDelta { .. },           // 串流增量
    MessageCompleted { .. },       // 訊息完成
    ThreadRunStarted { .. },       // 回合開始
    ThreadRunCompleted { .. },     // 回合結束
    ThreadRunCancelled { .. },     // 回合取消
    ThreadRunFailed { .. },        // 回合失敗
    Usage { .. },                  // 使用量彙總
}
```

**映射壓縮比**：50+ 內部事件 → 8 外部事件，壓縮比 **6.25:1**

這個壓縮比的目的是：
1. 外部消費者（TUI、SDK、第三方整合）不需了解內部實作細節
2. 版本穩定性 — 內部事件可自由增減，不影響外部 API
3. 簡化序列化 — 8 種類型的 JSON 模式遠比 50+ 種容易維護

### 17.3 事件映射的效能影響

```
50+ EventMsg variants
    │
    ├── 每個 variant 可能附帶：
    │     ├── String clone（clone 成本）
    │     ├── Vec<TurnItem> clone
    │     └── TokenUsage 計算
    │
    └── 映射到 8 ThreadEvent
          │
          ├── serde_json::to_string()  ← JSON 序列化
          │     └── 成本：~1-50μs per event
          │
          └── JSONL 寫入
                └── 成本：~0.1-1μs per event（buffered I/O）
```

> **Clawtex 實作建議**：
> 1. clawtex 的 Telegram 介面直接消費 agent_runtime 事件 — 應建立類似的外部事件層。
> 2. 8 種外部事件 vs clawtex 的直接暴露 — clawtex 應定義 `TelegramEvent` 枚舉作為中間層。
> 3. 事件壓縮比 6.25:1 是良好的抽象界線 — clawtex 的 HTTP API 也應使用壓縮後的事件。

---

## 18. 關鍵檔案索引

| 功能 | 檔案路徑 | 說明 |
|------|----------|------|
| Rust workspace 根 | `codex-rs/Cargo.toml` | 71 crate 的 workspace 定義 |
| TUI 入口 | `codex-rs/cli/src/main.rs` | ratatui 終端 UI |
| exec 入口 | `codex-rs/exec/src/main.rs` | 非互動式執行 |
| exec 核心 | `codex-rs/exec/src/lib.rs` | 指令列模式核心 |
| **代理迴圈** | `codex-rs/core/src/codex.rs` | SQ/EQ、Codex struct |
| **CodexThread** | `codex-rs/core/src/codex_thread.rs` | 對話容器、帶外引出 |
| **多代理控制** | `codex-rs/core/src/agent/control.rs` | AgentControl、歷史分叉 |
| **代理守衛** | `codex-rs/core/src/agent/guards.rs` | 限制代理數、暱稱管理 |
| **代理角色** | `codex-rs/core/src/agent/role.rs` | 角色定義與解析 |
| **協議定義** | `codex-rs/protocol/src/protocol.rs` | Op、Event、SandboxPolicy |
| **沙箱管理** | `codex-rs/core/src/sandboxing/mod.rs` | CommandSpec、ExecRequest |
| **審批快取** | `codex-rs/core/src/tools/sandboxing.rs` | ApprovalStore、with_cached_approval |
| **工具並行** | `codex-rs/core/src/tools/parallel.rs` | RwLock、AbortOnDropHandle |
| **事件映射** | `codex-rs/exec/src/event_processor_with_jsonl_output.rs` | 50+→8 事件轉換 |
| JSONL 事件定義 | `codex-rs/exec/src/exec_events.rs` | ThreadEvent(8 種) |
| 事件類型映射 | `codex-rs/core/src/event_mapping.rs` | ResponseItem 解析 |
| 命令執行引擎 | `codex-rs/core/src/exec.rs` | SandboxType、ExecParams |
| 工具路由 | `codex-rs/core/src/tools/router.rs` | 工具分發 |
| 工具註冊 | `codex-rs/core/src/tools/registry.rs` | 工具登記 |
| Shell handler | `codex-rs/core/src/tools/handlers/shell.rs` | Shell 命令執行 |
| MCP handler | `codex-rs/core/src/tools/handlers/mcp.rs` | MCP 工具橋接 |
| 上下文管理 | `codex-rs/core/src/context_manager/history.rs` | 歷史記錄、截斷 |
| **自動壓縮** | `codex-rs/core/src/compact.rs` | 分層壓縮、重試降級 |
| 遠端壓縮 | `codex-rs/core/src/compact_remote.rs` | OpenAI 伺服器端壓縮 |
| 認證系統 | `codex-rs/core/src/auth.rs` | API key 管理 |
| 認證儲存 | `codex-rs/core/src/auth/storage.rs` | Token 持久化 |
| 模型客戶端 | `codex-rs/core/src/client.rs` | HTTP/WebSocket 客戶端 |
| Linux 沙箱 lib | `codex-rs/linux-sandbox/src/lib.rs` | bwrap + landlock + proxy |
| Linux Landlock | `codex-rs/linux-sandbox/src/landlock.rs` | 檔案系統存取控制 |
| Linux bubblewrap | `codex-rs/linux-sandbox/src/bwrap.rs` | 容器化隔離 |
| Windows 沙箱 lib | `codex-rs/windows-sandbox-rs/src/lib.rs` | 27 模組統一入口 |
| Windows Token | `codex-rs/windows-sandbox-rs/src/token.rs` | Restricted Token |
| Windows ACL | `codex-rs/windows-sandbox-rs/src/acl.rs` | 存取控制列表 |
| Windows Firewall | `codex-rs/windows-sandbox-rs/src/firewall.rs` | WFP 防火牆 |
| Windows DPAPI | `codex-rs/windows-sandbox-rs/src/dpapi.rs` | 資料保護 |
| macOS Seatbelt | `codex-rs/core/src/seatbelt.rs` | sandbox-exec 包裝 |
| macOS 權限 | `codex-rs/core/src/sandboxing/macos_permissions.rs` | 權限管理 |
| 設定載入 | `codex-rs/core/src/config/mod.rs` | 分層設定 |
| 設定類型 | `codex-rs/core/src/config/types.rs` | Config struct |
| 設定服務 | `codex-rs/core/src/config/service.rs` | 設定服務層 |
| 設定權限 | `codex-rs/core/src/config/permissions.rs` | 權限定義 |
| Exec Policy | `codex-rs/execpolicy/` | Starlark 引擎 |
| 網路代理 | `codex-rs/network-proxy/` | 網路流量代理 |
| Shell 命令分析 | `codex-rs/shell-command/` | 命令解析與分類 |
| app-server | `codex-rs/app-server/src/lib.rs` | WebSocket/stdio JSON-RPC |
| TS 入口 | `codex-cli/bin/codex.js` | Node.js 啟動器 |
| 分析客戶端 | `codex-rs/core/src/analytics_client.rs` | 遙測數據收集 |
| 技能管理 | `codex-rs/core/src/skills/` | 技能載入與注入 |
| 外掛管理 | `codex-rs/core/src/plugins/` | 外掛系統 |
| Rollout 記錄 | `codex-rs/core/src/rollout/` | 對話持久化 |
| 狀態資料庫 | `codex-rs/core/src/state_db/` | 狀態持久化 |
