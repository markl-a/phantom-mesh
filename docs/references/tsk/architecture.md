# TSK 架構掃描報告

**掃描日期**: 2026-03-13
**專案**: tsk-tsk (Task Sandbox Kit)
**語言**: Rust
**核心概念**: AI Agent 沙箱 + 容器 + Git 工作流

---

## 1. 專案概覽

TSK 是一個企業級的代理沙箱框架，用於在隔離的 Docker/Podman 容器中安全執行 AI 代理任務。主要特性：

- **沙箱隔離**：每個任務在獨立容器運行，檔案系統隔離、網路隔離
- **分層鏡像**：基礎層 (Base) → 堆棧層 (Stack: Go/Python/Rust) → 專案層 → 代理層
- **代理支持**：Claude、Codex 等 AI 編碼代理
- **Git 工作流**：自動創建分支、提交、回傳結果
- **服務器模式**：並行執行多個任務，支援任務鏈
- **網路代理**：Squid 代理強制執行域名白名單
- **TUI 儀表板**：實時監控任務進度

**典型用場**：
- AI 代理自動開發（生成代碼、修復 Bug、寫文檔）
- 企業級代碼審查自動化
- 多代理平行開發
- 沙箱化的工具執行

---

## 2. 目錄結構

```
tsk-tsk/
├── Cargo.toml (應用程序清單)
├── justfile (開發命令)
├── README.md (使用者文檔)
├── CLAUDE.md (開發指南)
├── src/
│   ├── main.rs (CLI 主入口)
│   ├── agent/                     # AI 代理適配層
│   │   ├── mod.rs
│   │   ├── claude.rs             # Claude 代理
│   │   ├── claude/
│   │   │   └── claude_log_processor.rs
│   │   ├── codex.rs              # Codex 代理
│   │   ├── codex/
│   │   │   └── codex_log_processor.rs
│   │   ├── integ.rs              # 集成代理
│   │   ├── no_op.rs              # 無操作代理 (測試)
│   │   ├── provider.rs           # 代理工廠
│   │   ├── task_logger.rs        # 任務日誌記錄器
│   │   ├── log_processor.rs      # 日誌處理介面
│   │   ├── log_line.rs           # 日誌行解析
│   │   └── task_result.rs        # 任務結果
│   ├── commands/                  # CLI 子命令
│   │   ├── mod.rs
│   │   ├── run.rs               # `tsk run` (同步執行)
│   │   ├── shell.rs             # `tsk shell` (互動)
│   │   ├── add.rs               # `tsk add` (隊列)
│   │   ├── list.rs              # `tsk list` (列表)
│   │   ├── cancel.rs            # `tsk cancel` (取消)
│   │   ├── delete.rs            # `tsk delete` (刪除)
│   │   ├── clean.rs             # `tsk clean` (清理)
│   │   ├── retry.rs             # `tsk retry` (重試)
│   │   ├── task_args.rs         # 任務參數解析
│   │   ├── docker/
│   │   │   ├── mod.rs
│   │   │   └── build.rs         # `tsk docker build`
│   │   ├── server/
│   │   │   ├── mod.rs
│   │   │   ├── start.rs         # `tsk server start`
│   │   │   └── stop.rs          # `tsk server stop`
│   │   └── template/
│   │       ├── mod.rs
│   │       ├── list.rs          # `tsk template list`
│   │       ├── show.rs          # `tsk template show`
│   │       └── edit.rs          # `tsk template edit`
│   ├── context/                  # 依賴注入 + 配置
│   │   ├── mod.rs               # AppContext (DI 容器)
│   │   ├── tsk_env.rs           # 目錄路徑
│   │   ├── tsk_config.rs        # 配置加載和解析
│   │   ├── task_storage.rs      # SQLite 任務存儲
│   │   ├── docker_client.rs     # Docker 抽象介面
│   │   ├── terminal.rs          # 終端實用程序
│   │   └── notifications.rs     # 通知 (notify-rust)
│   ├── docker/                   # Docker 容器管理
│   │   ├── mod.rs               # DockerManager (主協調器)
│   │   ├── composer.rs          # Dockerfile 動態生成
│   │   ├── image_manager.rs     # 鏡像構建 (分層)
│   │   ├── proxy_manager.rs     # Squid 代理生命週期
│   │   ├── template_engine.rs   # Handlebars 模板引擎
│   │   ├── build_lock_manager.rs# 鏡像構建鎖
│   │   ├── layers.rs            # 鏡像層管理
│   │   └── seccomp_dind.json    # seccomp 配置
│   ├── server/                   # 伺服器模式 (後臺守護程序)
│   │   ├── mod.rs               # TskServer (主伺服器)
│   │   ├── scheduler.rs         # TaskScheduler (任務調度)
│   │   ├── worker_pool.rs       # WorkerPool (並行工作者)
│   │   └── lifecycle.rs         # 伺服器生命週期
│   ├── tui/                      # 終端使用者介面
│   │   ├── mod.rs
│   │   ├── app.rs              # TUI 應用狀態
│   │   ├── run.rs              # TUI 運行迴圈
│   │   ├── ui.rs               # UI 繪製
│   │   ├── events.rs           # 事件通道
│   │   └── input.rs            # 鍵盤/滑鼠輸入
│   ├── git.rs                    # Git 命令包裝器
│   ├── git_sync.rs              # Git 並發同步
│   ├── git_operations.rs        # 低層次 Git 操作
│   ├── repo_utils.rs            # 儲存庫工具函數
│   ├── task.rs                  # Task struct + 狀態
│   ├── task_manager.rs          # 任務管理邏輯
│   ├── task_builder.rs          # Task 構建者
│   ├── task_runner.rs           # 任務執行協調
│   ├── file_system.rs           # 檔案系統抽象
│   ├── repository.rs            # 自動檢測 (堆棧、專案)
│   ├── display.rs               # 格式化輸出
│   ├── stdin_utils.rs           # stdin 讀取
│   ├── utils.rs                 # 通用工具函數
│   ├── assets/                  # 嵌入資源
│   │   ├── mod.rs
│   │   ├── embedded.rs          # rust-embed 整合
│   │   ├── frontmatter.rs       # YAML frontmatter 解析
│   │   └── utils.rs
│   └── test_utils/              # 測試工具
│       ├── mod.rs
│       ├── docker_clients.rs    # Mock Docker
│       ├── git_test_helpers.rs
│       ├── git_test_utils.rs
│       └── git_test_utils.rs
├── dockerfiles/                  # Dockerfile 層
│   ├── base/
│   │   └── default.dockerfile   # OS + 基礎工具
│   ├── stack/
│   │   ├── default.dockerfile   # 最小堆棧
│   │   ├── go.dockerfile        # Go 工具鏈
│   │   ├── python.dockerfile    # Python 環境
│   │   ├── rust.dockerfile      # Rust 工具鏈
│   │   ├── node.dockerfile      # Node.js
│   │   ├── java.dockerfile      # Java/Maven
│   │   └── lua.dockerfile       # Lua
│   ├── agent/                   # 代理層
│   │   ├── claude.dockerfile
│   │   └── codex.dockerfile
│   └── tsk-proxy/
│       ├── proxy.dockerfile
│       └── squid.conf           # 代理配置
├── templates/                    # 任務模板
│   ├── feat.md                  # 功能開發
│   ├── fix.md                   # Bug 修復
│   ├── refactor.md              # 重構
│   ├── doc.md                   # 文檔
│   ├── test.md                  # 測試
│   └── ...
├── .tsk/                         # 專案級配置
│   ├── tsk.toml                 # 專案配置
│   └── templates/               # 自訂模板
├── tests/
│   └── integration/
│       └── projects/            # 堆棧層集成測試
│           ├── rust/
│           ├── python/
│           ├── go/
│           └── ...
├── skills/                       # Claude Code Skills
└── documentation/               # 文檔
    ├── docker-builds.md
    ├── network-isolation.md
    ├── skill-marketplace.md
    └── ...
```

---

## 3. 核心 Trait / Struct

### 3.1 任務 (Task)

```rust
pub enum TaskStatus {
    Queued,      // 等待執行
    Running,     // 執行中
    Complete,    // 成功完成
    Failed,      // 執行失敗
    Cancelled,   // 被取消
}

pub struct Task {
    pub id: String,                    // YYYY-MM-DD-HHMM-{type}-{name}
    pub repo_root: PathBuf,
    pub name: String,
    pub task_type: String,             // "feat", "fix", "doc", etc.
    pub instructions_file: String,
    pub agent: String,                 // "claude", "codex"
    pub status: TaskStatus,
    pub created_at: DateTime<Local>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub branch_name: String,           // tsk/{type}/{name}/{id}
    pub error_message: Option<String>,
    pub source_commit: String,
    pub source_branch: Option<String>,
    pub stack: String,                 // "rust", "python", "go"
    pub project: String,               // 專案名稱
    pub copied_repo_path: Option<PathBuf>,  // 任務的儲存庫副本
    pub is_interactive: bool,
    pub parent_ids: Vec<String>,       // 鏈任務的父代
    pub resolved_config: Option<String>, // 快照配置 (JSON)
}
```

### 3.2 應用上下文 (AppContext - DI 容器)

```rust
pub struct AppContext {
    tsk_env: TskEnv,
    tsk_config: TskConfig,
    task_storage: Arc<TaskStorage>,
    // 其他共享資源
}

// TskEnv: 目錄路徑
pub struct TskEnv {
    pub data_dir: PathBuf,      // ~/.local/share/tsk
    pub config_dir: PathBuf,    // ~/.config/tsk
    pub tasks_db: PathBuf,      // ~/.local/share/tsk/tasks.db
}

// TskConfig: 配置結構
pub struct TskConfig {
    pub container_engine: ContainerEngine,
    pub defaults: ConfigDefaults,
    pub projects: HashMap<String, ProjectConfig>,
    pub stack_config: HashMap<String, StackConfig>,
    pub agent_config: HashMap<String, AgentConfig>,
}
```

### 3.3 Docker 管理器 (DockerManager)

```rust
pub struct DockerManager {
    ctx: AppContext,
    client: Arc<dyn DockerClient>,
    proxy_manager: ProxyManager,
    event_sender: Option<ServerEventSender>,
}

// 主要方法：
// - async fn run_task(task, agent, log_processor) -> Result<()>
// - async fn build_image(stack, agent, project) -> Result<String>
// - 容器生命週期：create → configure → run → collect_logs → cleanup
```

### 3.4 鏡像管理器 (DockerImageManager)

```rust
pub struct DockerImageManager {
    // 構建分層鏡像：
    // base (OS + Git + Git-LFS)
    //   ↓
    // stack (語言工具鏈)
    //   ↓
    // project (專案特定依賴)
    //   ↓
    // agent (Claude 或 Codex)

    // 方法：
    // async fn build_or_get_image(...) -> Result<String>
}
```

### 3.5 代理適配層 (Agent Trait)

```rust
pub trait Agent: Send + Sync {
    async fn validate(&self) -> Result<()>;
    async fn warmup(&self) -> Result<()>;
    async fn build_command(&self, instructions: &str) -> Result<String>;
    async fn version(&self) -> Result<String>;
}

// 實現：
pub struct ClaudeAgent { /* ... */ }
pub struct CodexAgent { /* ... */ }
pub struct NoOpAgent { /* ... */ }  // 測試
```

### 3.6 伺服器 (TskServer)

```rust
pub struct TskServer {
    app_context: Arc<AppContext>,
    docker_client: Arc<dyn DockerClient>,
    quit_signal: Arc<tokio::sync::Notify>,
    scheduler: Arc<Mutex<TaskScheduler>>,
    scheduler_handle: Mutex<Option<JoinHandle<()>>>,
    submitted_tasks: Arc<Mutex<HashSet<String>>>,
    running_flag: Arc<Mutex<bool>>,
    lifecycle: ServerLifecycle,
    workers: u32,
    event_sender: Option<ServerEventSender>,
}

// 方法：
// async fn run() -> Result<()>  // 啟動伺服器
// async fn graceful_shutdown()   // 優雅關閉
// async fn submit_task(task)    // 提交任務
```

### 3.7 任務調度器 (TaskScheduler)

```rust
pub struct TaskScheduler {
    // 責任：
    // 1. 從 TaskStorage 取出 Queued 任務
    // 2. 檢查依賴 (parent_ids)
    // 3. 通過 WorkerPool 分派給工作者
    // 4. 監視執行狀態
    // 5. 發出 ServerEvent (TUI)
    // 6. 自動清理舊任務 (7 天)

    // 方法：
    // async fn schedule_tasks() -> Result<()>
    // async fn on_task_complete(task_id) -> Result<()>
}
```

### 3.8 工作者池 (WorkerPool)

```rust
pub struct WorkerPool {
    // 並行執行 N 個任務 (N = --workers 標誌)
    // 每個工作者：
    // 1. 取出任務
    // 2. 創建 DockerManager
    // 3. 運行任務
    // 4. 更新狀態

    workers: Vec<Worker>,
    task_channel: mpsc::UnboundedReceiver<TaskId>,
}
```

### 3.9 TUI (終端使用者介面)

```rust
pub struct TuiApp {
    tasks: Vec<Task>,  // 任務列表
    selected_task_idx: usize,
    log_lines: VecDeque<String>,
    scroll_offset: usize,
    focus: UiFocus,  // TaskList | LogViewer
}

// 事件：
// ServerEvent::TaskStatusChanged
// ServerEvent::LogLineAdded
// ServerEvent::TaskCompleted

// 鍵盤控制：
// Up/Down (j/k) - 導航
// Left/Right (h/l) - 焦點切換
// c - 取消任務
// d - 刪除任務
// q - 退出
```

### 3.10 任務存儲 (TaskStorage)

```rust
pub struct TaskStorage {
    // SQLite 資料庫：tasks.db
    // 表：
    // - tasks (id, repo_root, name, status, ...)
    // - cost_records (可選，用於成本追蹤)

    // 方法：
    // async fn create_task(task) -> Result<()>
    // async fn update_task_status(id, status) -> Result<()>
    // async fn get_task(id) -> Result<Task>
    // async fn list_tasks(status) -> Result<Vec<Task>>
    // async fn delete_task(id) -> Result<()>
}
```

---

## 4. 啟動流程

### 4.1 CLI 啟動 (main.rs)

```
main()
  ↓
Cli::parse() (Clap)
  ↓
AppContext::builder()
  .with_container_engine(...)
  .build()
  ↓
匹配命令類型
  ├─ Run      → RunCommand::execute() [同步]
  ├─ Shell    → ShellCommand::execute() [互動]
  ├─ Add      → AddCommand::execute() [隊列]
  ├─ List     → ListCommand::execute() [查詢]
  ├─ Server   → ServerStartCommand::execute() [後臺]
  ├─ Docker   → DockerBuildCommand::execute() [構建]
  └─ Template → TemplateListCommand::execute() [範本]
```

### 4.2 `tsk run` 啟動流程

```
RunCommand::execute(ctx)
  ↓
TaskBuilder::new(ctx)
  .with_args(name, type, prompt, ...)
  .build_task()  // 創建 Task 結構體
  ↓
DockerManager::new(ctx, docker_client)
  ↓
DockerImageManager::build_or_get_image(stack, agent, project)
  ├─ 檢查鏡像快取
  ├─ 動態生成 Dockerfile (Composer)
  ├─ docker build (分層)
  └─ 標籤鏡像
  ↓
TaskRunner::run(task, docker_manager, agent, log_processor)
  ├─ 複製儲存庫 (GitSyncManager + overlay)
  ├─ 建立 Git 分支
  ├─ 建立代理容器
  ├─ 建立代理容器
  ├─ 啟動 Squid 代理容器
  ├─ 執行代理命令 (claude/codex)
  ├─ 收集日誌 (stream 至 stdout)
  ├─ 監視完成
  ├─ 提交/推送 Git 分支
  └─ 清理
  ↓
返回結果 (成功/失敗)
```

### 4.3 `tsk add` 啟動流程

```
AddCommand::execute(ctx)
  ↓
TaskBuilder::new(ctx)
  .with_args(...)
  .build_task()
  ↓
TaskStorage::create_task(task)
  ├─ 快照配置到 resolved_config
  ├─ 插入 SQLite 資料庫
  └─ 返回任務 ID
  ↓
打印任務 ID + 提示
```

### 4.4 `tsk server start` 啟動流程

```
ServerStartCommand::execute(ctx, workers)
  ↓
TskServer::with_workers(ctx, docker_client, workers)
  ├─ 創建 TaskScheduler
  ├─ 創建 WorkerPool
  ├─ 如果是互動終端 → TUI 模式
  └─ 否則 → 純文本模式
  ↓
TskServer::run()
  ├─ 啟動 TaskScheduler (背景任務)
  ├─ TaskScheduler 迴圈：
  │   ├─ 查詢 Queued/Waiting 任務
  │   ├─ 檢查依賴
  │   ├─ 分派給 WorkerPool
  │   └─ 發出 ServerEvent
  ├─ TUI 或 文本 主迴圈：
  │   ├─ 接收事件 (ServerEvent)
  │   ├─ 更新狀態
  │   ├─ 重繪 TUI 或 打印
  │   └─ 處理鍵盤輸入 (q 退出)
  └─ 優雅關閉
```

### 4.5 任務執行生命週期

```
Task 狀態機：
Queued (隊列中)
  ↓
[Server 檢查依賴]
  ├─ 如果有 parent_id 且 parent 未完成 → Waiting
  │   (等待 parent 完成)
  └─ 否則 → Running
  ↓
Running (執行中)
  ├─ 複製儲存庫
  ├─ 建立分支
  ├─ 啟動容器
  ├─ 運行代理
  ├─ 收集結果
  ├─ 提交/推送
  └─ [成功/失敗]
  ↓
Complete 或 Failed
  (或 Cancelled 如果被中斷)
```

---

## 5. 資料流 ASCII 圖

### 5.1 整體架構

```
┌──────────────────────────────────────────────────────┐
│                   User / CLI                         │
│  tsk run | tsk add | tsk shell | tsk server start   │
└─────────────────────┬────────────────────────────────┘
                      │
                      ↓
        ┌─────────────────────────────┐
        │    AppContext (DI)          │
        │  ├─ TskEnv (paths)         │
        │  ├─ TskConfig (settings)   │
        │  └─ TaskStorage (DB)       │
        └─────────────────────────────┘
                      │
         ┌────────────┼────────────┐
         ↓            ↓            ↓
    ┌─────────┐ ┌──────────┐ ┌──────────┐
    │Run Cmd  │ │Server    │ │Docker Cmd│
    │(Sync)   │ │(Daemon)  │ │(Build)   │
    └────┬────┘ └────┬─────┘ └────┬─────┘
         │           │            │
         ↓           ↓            ↓
  ┌──────────────────────────────────────┐
  │    Task/Docker/Git Management       │
  │  ├─ TaskBuilder → Task              │
  │  ├─ TaskRunner → Execution          │
  │  ├─ DockerManager → Containers      │
  │  ├─ GitSyncManager → Repo Copies    │
  │  └─ ProxyManager → Network Isolation│
  └──────────────────────────────────────┘
         │
         ├─────────────┬──────────┐
         ↓             ↓          ↓
    ┌────────┐    ┌─────────┐  ┌─────────┐
    │ Docker │    │   Git   │  │Squid    │
    │ Engine │    │  Repo   │  │Proxy    │
    │        │    │ Copy    │  │         │
    └────────┘    └─────────┘  └─────────┘
         │             │           │
         └─────────────┴───────────┘
                 ↓
          [Container Execution]
```

### 5.2 `tsk run` 詳細流程

```
User Input
  ↓
CLI Parse (Clap)
  │
  ├─ name: "my-feature"
  ├─ type: "feat"
  ├─ prompt: "Add new API endpoint"
  ├─ agent: "claude"
  ├─ stack: "rust" (or auto-detect)
  └─ project: "my-service" (or use dir name)
  ↓
TaskBuilder
  ├─ 生成 ID: 2026-03-13-1423-feat-my-feature
  ├─ 建立 instructions.md
  ├─ 快照配置
  └─ 創建 Task struct
  ↓
DockerManager::run_task()
  ├─ DockerImageManager::build_or_get_image()
  │   ├─ 讀取 Dockerfile 嵌入資源
  │   ├─ 使用 Composer 動態生成
  │   ├─ docker build (分層)
  │   └─ 返回鏡像 ID
  │
  ├─ GitSyncManager::clone_with_overlay()
  │   ├─ git clone (淺複製)
  │   ├─ 應用工作目錄 overlay
  │   └─ 返回副本路徑
  │
  ├─ ProxyManager::start_proxy()
  │   ├─ 生成 squid.conf (白名單)
  │   ├─ docker run tsk-proxy
  │   └─ 返回代理主機
  │
  ├─ 建立任務容器 (docker create)
  │   ├─ 環境變數 (TSK_PROXY_HOST, etc.)
  │   ├─ 卷掛載 (repo, output)
  │   ├─ 網路 (代理容器)
  │   └─ seccomp 設定檔 (安全)
  │
  ├─ docker start → 執行代理命令
  │   └─ /bin/bash -c "claude --agent-args ..."
  │
  ├─ 流式收集日誌
  │   ├─ docker logs --follow
  │   └─ 解析結構化 JSON 行
  │
  ├─ 監視完成 (docker wait)
  │   └─ 捕捉 exit code
  │
  ├─ GitSyncManager::commit_and_push()
  │   ├─ git add -A
  │   ├─ git commit (自動訊息)
  │   └─ git push origin tsk/feat/my-feature/ID
  │
  └─ 清理容器 + 代理
  ↓
成功 / 失敗
```

### 5.3 伺服器模式 (並行執行)

```
┌─────────────────────────────────────┐
│   tsk server start --workers 4     │
└────────────┬────────────────────────┘
             ↓
      ┌──────────────────┐
      │  TskServer       │
      │  (workers = 4)   │
      └────────┬─────────┘
               │
    ┌──────────┼──────────┐
    ↓          ↓          ↓
┌────────┐┌────────┐┌────────┐
│Worker 1│ │Worker 2│ │Worker 3│
│        │ │        │ │        │
└────┬───┘ └────┬───┘ └────┬───┘
     │          │          │
┌────┴──────────┴──────────┴────┐
│    TaskScheduler (async)       │
│  查詢 Queued/Waiting 任務      │
│  檢查 parent_ids (鏈)          │
│  分派給 WorkerPool             │
│  發出 ServerEvent              │
└────┬──────────────────────────┘
     │
     ├─ 儲存庫副本 (per task)
     ├─ 鏡像構建快取 (shared)
     └─ 代理容器 (isolated)

[TUI 儀表板]
┌─────────────────┬──────────────┐
│ Task List       │  Log Viewer  │
│ - Running (1)   │  [Task Log]  │
│ - Queued (2)    │              │
│ - Complete (5)  │              │
└─────────────────┴──────────────┘
```

### 5.4 任務鏈 (Task Chaining)

```
tsk add -t feat -n setup --prompt "Setup DB" > task-1
tsk add -t feat -n migrate --parent task-1 --prompt "Run migrations"
tsk add -t test -n validate --parent task-2 --prompt "Run tests"

狀態流：

Task-1 (setup)
├─ Status: Queued → Running → Complete
│
Task-2 (migrate)
├─ parent_ids: [task-1]
├─ Status: Queued → Waiting → Running → Complete
│  (等待 task-1 完成後才轉 Running)
│
Task-3 (validate)
├─ parent_ids: [task-2]
├─ Status: Queued → Waiting → Running → Complete
│  (等待 task-2 完成後才轉 Running)
```

---

## 6. 子系統清單

### P0 (核心 - 必須運作)

| 子系統 | 模組位置 | 責任 | 依賴 |
|------|---------|------|------|
| **Task Model** | src/task.rs | 任務定義 + 狀態機 | chrono, serde |
| **AppContext** | src/context/mod.rs | DI 容器 + 配置 | TskEnv, TskConfig, TaskStorage |
| **TaskStorage** | src/context/task_storage.rs | SQLite 任務資料庫 | rusqlite |
| **DockerManager** | src/docker/mod.rs | 容器生命週期 | bollard, DockerImageManager |
| **DockerImageManager** | src/docker/image_manager.rs | 分層鏡像構建 | Composer, docker CLI |
| **GitSyncManager** | src/git_sync.rs | 儲存庫複製 + 同步 | git2, GitOperations |
| **TaskRunner** | src/task_runner.rs | 任務執行協調 | DockerManager, Agent |
| **Agent Trait** | src/agent/mod.rs | 代理抽象 | 無 |
| **ClaudeAgent** | src/agent/claude.rs | Claude 代理實現 | 無 |
| **CodexAgent** | src/agent/codex.rs | Codex 代理實現 | 無 |
| **ProxyManager** | src/docker/proxy_manager.rs | Squid 代理容器 | docker, Composer |

### P1 (重要 - 功能完整性)

| 子系統 | 模組位置 | 責任 | 依賴 |
|------|---------|------|------|
| **TskServer** | src/server/mod.rs | 伺服器主迴圈 | TaskScheduler, WorkerPool |
| **TaskScheduler** | src/server/scheduler.rs | 任務調度 + 依賴解析 | TaskStorage, WorkerPool |
| **WorkerPool** | src/server/worker_pool.rs | 並行工作者 | tokio, TaskRunner |
| **TskConfig** | src/context/tsk_config.rs | TOML 配置解析 | toml, serde |
| **Composer** | src/docker/composer.rs | Dockerfile 動態生成 | Handlebars, template_engine |
| **ProxyManager** | src/docker/proxy_manager.rs | Squid 代理配置 | docker |
| **TUI** | src/tui/ | 終端儀表板 | ratatui, crossterm |
| **GitOperations** | src/git_operations.rs | 低層 git 命令 | git CLI |
| **TaskLogger** | src/agent/task_logger.rs | 結構化日誌 | 無 |
| **LogProcessor** | src/agent/log_processor.rs | 日誌解析 | serde_json |

### P2 (可選 - 示例/工具)

| 子系統 | 模組位置 | 責任 | 依賴 |
|------|---------|------|------|
| **Commands** | src/commands/ | CLI 子命令 | 無 |
| **TaskBuilder** | src/task_builder.rs | 任務構造 | 無 |
| **Repository** | src/repository.rs | 自動檢測 (stack, project) | 無 |
| **FileSystem** | src/file_system.rs | FS 抽象 | 無 |
| **Templates** | templates/ + assets/ | 任務範本 + YAML frontmatter | 無 |
| **Assets** | src/assets/ | 嵌入 Dockerfile + 配置 | rust-embed |
| **Notifications** | src/context/notifications.rs | 系統通知 | notify-rust |
| **TestUtils** | src/test_utils/ | 測試助手 | 無 |

---

## 7. 技術棧

- **主語言**：Rust 2024 Edition (MSRV 可能較舊)
- **異步運行時**：Tokio (full features)
- **容器**：Bollard (Docker SDK)
- **Git**：git2 (libgit2 綁定)
- **CLI**：Clap (derive)
- **Web**：無 (純 CLI)
- **TUI**：Ratatui + Crossterm
- **配置**：TOML + Serde
- **範本**：Handlebars + rust-embed
- **資料庫**：Rusqlite (SQLite)
- **JSON**：Serde JSON
- **日誌**：Tracing (optional)
- **錯誤**：Anyhow + ThisError
- **通知**：notify-rust (可選)
- **時間**：Chrono

---

## 8. 關鍵設計模式

### 8.1 DI 容器模式
`AppContext` 集中管理依賴：配置、儲存、客戶端。

### 8.2 Builder 模式
`TaskBuilder`、`AppContext::builder()` 提供流暢的構建 API。

### 8.3 策略模式
代理 (Claude, Codex, NoOp) 實現 Agent trait，可互換。

### 8.4 模板方法
`TaskRunner::run()` 定義執行流程，`Agent` 子類自訂細節。

### 8.5 狀態機
Task 狀態 (Queued → Running → Complete/Failed) 由調度器驅動。

### 8.6 配置快照
任務創建時快照配置到 `resolved_config` (JSON)，避免配置漂移。

### 8.7 分層鏡像
Docker 鏡像分 4 層 (Base → Stack → Project → Agent)，重用快取。

### 8.8 代理白名單
Squid 代理強制執行域名白名單，網路隔離。

---

## 9. 擴展點

1. **新代理**：實現 Agent trait (validate, warmup, build_command, version)
2. **新堆棧**：添加 `dockerfiles/stack/` Dockerfile
3. **新命令**：實現 Command trait + 在 main.rs 匹配
4. **新配置選項**：擴展 TskConfig 結構體
5. **自訂範本**：添加 `.tsk/templates/` 範本文件
6. **事件監聽**：訂閱 ServerEvent 通道

---

## 10. 性能特性

- **並行執行**：WorkerPool 支援 N 個並行任務
- **鏡像快取**：分層構建重用基礎層
- **增量 Git**：淺複製 (--depth=1) 加快儲存庫複製
- **流式日誌**：即時流式輸出，無緩衝延遲
- **自動清理**：舊任務自動清理 (7 天)

---

## 11. 安全特性

- **容器隔離**：每個任務獨立容器，檔案系統隔離
- **網路隔離**：Squid 代理強制域名白名單
- **能力下降**：Docker seccomp 設定檔限制系統呼叫
- **唯讀掛載**：配置資源以只讀方式掛載
- **無根容器**：容器內運行 `agent` 使用者 (非 root)

---

## 12. 已知限制

- 無 Windows 原生支援 (需 WSL2 + Docker Desktop)
- 任務鏈父代失敗時，子代自動標記失敗 (無重試)
- Squid 代理配置目前不支援模式匹配 (FQDN 精確比對)
- TUI 儀表板不支援 Windows 原生終端 (需要 Windows Terminal)

---

**文檔版本**: 1.0
**最後更新**: 2026-03-13
