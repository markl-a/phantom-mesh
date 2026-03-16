# tsk 專案深度技術分析

> 分析日期: 2026-03-13 (v2 — 深度翻倍)
> 專案版本: tsk-ai v0.10.3 (Rust 2024 Edition)
> 倉庫位址: https://github.com/dtormoen/tsk-tsk
> 原始碼路徑: `LLM-Cluster-Project/references/tsk/`

---

## 目錄

1. [專案結構](#1-專案結構)
2. [進入點與啟動流程](#2-進入點與啟動流程)
3. [WorkerPool — Semaphore 並行控制](#3-workerpool--semaphore-並行控制)
4. [TaskScheduler — 排程核心深度解析](#4-taskscheduler--排程核心深度解析)
5. [Config Snapshot 系統](#5-config-snapshot-系統)
6. [DAG Task Chaining](#6-dag-task-chaining)
7. [TaskBuilder — 任務建構流水線](#7-taskbuilder--任務建構流水線)
8. [Agent 整合與日誌處理](#8-agent-整合與日誌處理)
9. [Docker 容器執行](#9-docker-容器執行)
10. [TaskStorage — SQLite 持久化層](#10-taskstorage--sqlite-持久化層)
11. [錯誤處理深度分析](#11-錯誤處理深度分析)
12. [效能特徵分析](#12-效能特徵分析)
13. [Clawtex 差距對比與 Rust 實作建議](#13-clawtex-差距對比與-rust-實作建議)

---

## 1. 專案結構

### 1.1 目錄樹

```
tsk/
├── src/
│   ├── main.rs                    # CLI 進入點 (clap derive)
│   ├── task.rs                    # Task struct + TaskStatus enum (370 行)
│   ├── task_builder.rs            # TaskBuilder 流式 API (500+ 行)
│   ├── task_manager.rs            # 任務 CRUD + 清理 + BFS 後代搜尋
│   ├── task_runner.rs             # 任務在容器中的執行引擎 (526 行)
│   ├── display.rs                 # 終端顯示格式化
│   ├── file_system.rs             # 非同步檔案系統操作 (copy_dir)
│   ├── git.rs                     # RepoManager: 倉庫複製/分支/提交
│   ├── git_operations.rs          # 低階 git CLI 操作
│   ├── git_sync.rs                # GitSyncManager: tokio::Mutex + flock 雙鎖
│   ├── repo_utils.rs              # 倉庫根目錄搜尋
│   ├── repository.rs              # Stack/Project 自動偵測
│   ├── stdin_utils.rs             # stdin 管道輸入處理
│   ├── utils.rs                   # 通用工具函式
│   ├── build.rs                   # 編譯時資源嵌入
│   ├── agent/
│   │   ├── mod.rs                 # Agent trait 定義 (91 行)
│   │   ├── provider.rs            # AgentProvider: 工廠模式
│   │   ├── claude.rs              # ClaudeAgent 實作
│   │   ├── claude/
│   │   │   └── claude_log_processor.rs  # Claude JSON stream 解析
│   │   ├── codex.rs               # CodexAgent 實作
│   │   ├── codex/
│   │   │   └── codex_log_processor.rs   # Codex JSONL 解析
│   │   ├── integ.rs               # IntegAgent (整合測試用)
│   │   ├── no_op.rs               # NoOpAgent (空操作)
│   │   ├── no_op_log_processor.rs # NoOp 日誌處理器
│   │   ├── log_line.rs            # LogLine 結構化日誌枚舉
│   │   ├── log_processor.rs       # LogProcessor trait
│   │   ├── task_logger.rs         # TaskLogger: 檔案+stdout 雙通道寫入
│   │   └── task_result.rs         # TaskResult 結構
│   ├── commands/
│   │   ├── mod.rs                 # Command trait
│   │   ├── run.rs                 # tsk run (同步即時執行)
│   │   ├── shell.rs               # tsk shell (互動式沙盒)
│   │   ├── add.rs                 # tsk add (佇列排程)
│   │   ├── list.rs                # tsk list
│   │   ├── cancel.rs              # tsk cancel
│   │   ├── clean.rs               # tsk clean
│   │   ├── delete.rs              # tsk delete
│   │   ├── retry.rs               # tsk retry
│   │   ├── task_args.rs           # 共用 CLI 參數解析
│   │   ├── docker/mod.rs          # tsk docker build
│   │   │   └── build.rs
│   │   ├── server/
│   │   │   ├── mod.rs             # server 子命令路由
│   │   │   ├── start.rs           # tsk server start
│   │   │   └── stop.rs            # tsk server stop
│   │   └── template/
│   │       ├── mod.rs             # template 子命令路由
│   │       ├── list.rs            # tsk template list
│   │       ├── show.rs            # tsk template show
│   │       └── edit.rs            # tsk template edit
│   ├── context/
│   │   ├── mod.rs                 # AppContext: DI 容器 (234 行)
│   │   ├── tsk_env.rs             # TskEnv: 路徑管理 (XDG 規範)
│   │   ├── tsk_config.rs          # TskConfig: TOML 分層配置 (850+ 行)
│   │   ├── task_storage.rs        # TaskStorage: SQLite 後端 (570+ 行)
│   │   ├── docker_client.rs       # DockerClient trait + 實作
│   │   ├── notifications.rs       # 系統通知 (notify-rust)
│   │   └── terminal.rs            # 終端標題管理
│   ├── docker/
│   │   ├── mod.rs                 # DockerManager: 容器執行
│   │   ├── image_manager.rs       # DockerImageManager: 映像建構
│   │   ├── composer.rs            # Dockerfile 組合器
│   │   ├── layers.rs              # 分層建構邏輯 (base→stack→project→agent)
│   │   ├── template_engine.rs     # Handlebars 模板引擎
│   │   ├── proxy_manager.rs       # Squid 代理容器管理
│   │   └── build_lock_manager.rs  # 建構鎖管理 (flock)
│   ├── server/
│   │   ├── mod.rs                 # TskServer 主結構
│   │   ├── scheduler.rs           # TaskScheduler: 排程核心 (600+ 行)
│   │   ├── worker_pool.rs         # WorkerPool: Semaphore 並行執行池 (487 行)
│   │   └── lifecycle.rs           # PID 檔案/進程管理
│   ├── tui/
│   │   ├── mod.rs                 # TUI 模組入口
│   │   ├── app.rs                 # ratatui 應用狀態
│   │   ├── events.rs              # ServerEvent 事件枚舉
│   │   ├── input.rs               # 鍵盤/滑鼠輸入處理
│   │   ├── run.rs                 # TUI 事件迴圈
│   │   └── ui.rs                  # 介面渲染
│   ├── assets/
│   │   ├── mod.rs                 # 模板/資源管理
│   │   ├── embedded.rs            # rust-embed 嵌入資源
│   │   ├── frontmatter.rs         # YAML frontmatter 解析
│   │   └── utils.rs               # 模板工具函式
│   └── test_utils/
│       ├── mod.rs                 # 測試輔助
│       ├── docker_clients.rs      # Mock Docker clients
│       ├── git_test_helpers.rs    # Git 測試輔助
│       └── git_test_utils.rs      # TestGitRepository
├── templates/                     # 內建任務模板 (feat, fix, refactor, plan, doc, ack, shell)
├── dockerfiles/                   # Docker 映像定義 (agent/stack/project/tsk-proxy)
├── skills/                        # Claude Code 技能市集 (tsk-add, tsk-config, tsk-help)
└── Cargo.toml
```

### 1.2 關鍵依賴

| 依賴 | 用途 | 設計重要性 |
|------|------|-----------|
| `clap` (derive) | CLI 參數解析 | Command pattern 入口 |
| `bollard` | Docker API 客戶端 | 容器生命週期管理 |
| `tokio` (full) | 非同步執行時 | Semaphore, JoinSet, Mutex |
| `rusqlite` (bundled) | 任務持久化 (SQLite) | StdMutex + blocking_task |
| `git2` | libgit2 綁定 (部分 git 操作) | 倉庫狀態查詢 |
| `ratatui` + `crossterm` | TUI 面板 | ServerEvent 可視化 |
| `serde` / `serde_json` | 序列化/反序列化 | Config snapshot |
| `handlebars` | Dockerfile 模板渲染 | 分層映像建構 |
| `rust-embed` | 內建資源嵌入 | 模板 + Dockerfile |
| `tar` | tar 歸檔操作 | 檔案複製至容器 |
| `nanoid` | 唯一任務 ID | 8 字元，排除 `-` |
| `notify-rust` | 系統通知 | 任務完成通知 |
| `chrono` | 時間處理 | Task 生命週期時間戳 |

---

## 2. 進入點與啟動流程

### 2.1 CLI 介面

**檔案**: `src/main.rs`

```rust
#[derive(Parser)]
#[command(name = "tsk")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Run { ... },           // 即時同步執行
    Shell { ... },         // 互動式沙盒
    Add { ... },           // 佇列任務
    Server(ServerArgs),    // server start/stop
    List,                  // 列出所有任務
    Cancel { ... },        // 取消任務
    Clean,                 // 清理已完成任務
    Delete { ... },        // 刪除任務
    Retry { ... },         // 重試任務
    Docker(DockerArgs),    // docker build
    Template(TemplateArgs), // template list/show/edit
}
```

### 2.2 啟動流程資料流圖

```
main()
  │
  ├─ Cli::parse()           // clap 解析命令列參數
  │
  ├─ AppContext::builder()   // 建構 DI 容器
  │   ├─ with_container_engine(...)
  │   ├─ .build()
  │   │   ├─ TskEnv::new()        // XDG 路徑
  │   │   ├─ TskConfig::load()    // TOML 分層配置
  │   │   ├─ TaskStorage::new()   // SQLite 初始化
  │   │   ├─ GitSyncManager::new() // 雙鎖初始化
  │   │   └─ NotificationClient
  │   │
  │   └─ #[cfg(test)]:
  │       ├─ TempDir 自動創建
  │       ├─ 記憶體 SQLite
  │       └─ interactive = false
  │
  ├─ warn_deprecated_dockerfiles()
  │
  ├─ match cli.command ──▶ Box<dyn Command>  // 命令模式分發
  │
  └─ command.execute(&app_context)
         │
         ├─ Run:   TaskBuilder → DockerManager → 即時執行
         ├─ Add:   TaskBuilder → TaskStorage::add_task()
         ├─ Shell: TaskBuilder → interactive container
         └─ Server: TskServer → TaskScheduler → WorkerPool
```

### 2.3 Command trait

**檔案**: `src/commands/mod.rs`

```rust
#[async_trait]
pub trait Command: Send + Sync {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>>;
}
```

所有子命令實作此 trait，透過 `Box<dyn Command>` 實現多態分發。每個命令獨立封裝自己的邏輯，且易於測試。

### 2.4 AppContext: 依賴注入容器

**檔案**: `src/context/mod.rs` (234 行)

```rust
#[derive(Clone)]
pub struct AppContext {
    git_sync_manager: Arc<GitSyncManager>,
    interactive: bool,
    notification_client: Arc<NotificationClient>,
    task_storage: Arc<TaskStorage>,
    terminal_operations: Arc<terminal::TerminalOperations>,
    tsk_config: Arc<TskConfig>,
    tsk_env: Arc<TskEnv>,
}
```

**關鍵設計決策**:
1. **Docker client 不在 AppContext 中**: 只有需要容器操作的命令才建構 Docker client。`add`、`list` 等純資料操作不需要 Docker daemon。
2. **Builder 模式**: 測試時自動建立臨時目錄和 mock 組件
3. **所有欄位使用 `Arc`**: 支援跨執行緒安全共享
4. **`interactive` 標記**: 控制 TUI 模式 vs 純文字輸出

```rust
// 生產模式
let ctx = AppContext::builder()
    .with_container_engine(Some(ContainerEngine::Docker))
    .build();

// 測試模式 (自動使用 TempDir 和 mock)
let ctx = AppContext::builder().build();
```

---

## 3. WorkerPool — Semaphore 並行控制

### 3.1 核心結構

**檔案**: `src/server/worker_pool.rs` 第 52-64 行

```rust
pub struct WorkerPool<T: AsyncJob> {
    workers: usize,                                      // 最大並行數
    semaphore: Arc<Semaphore>,                           // 並行度控制
    active_jobs: Arc<Mutex<JoinSet<Result<JobResult, JobError>>>>,  // 活動任務追蹤
    shutting_down: Arc<Mutex<bool>>,                     // 關閉標記
    _phantom: std::marker::PhantomData<T>,               // 型別標記
}
```

### 3.2 AsyncJob Trait

**檔案**: `worker_pool.rs` 第 37-43 行

```rust
pub trait AsyncJob: Send + 'static {
    fn execute(self) -> impl std::future::Future<Output = Result<JobResult, JobError>> + Send;
    fn job_id(&self) -> String;
}
```

**注意**: 使用 `impl Future` 語法而非 `#[async_trait]` — 這是 Rust 2024 edition 的新特性，避免了 `async_trait` 的 box 開銷。`self` 是 `self`（非 `&self`），意味著 job 在執行後被消費（move 語義）。

### 3.3 非阻塞提交 (try_submit)

**檔案**: `worker_pool.rs` 第 96-123 行

```rust
pub async fn try_submit(&self, job: T) -> Result<Option<JobHandle>, JobError> {
    // 檢查是否正在關閉
    if *self.shutting_down.lock().await {
        return Err(JobError::from("Worker pool is shutting down".to_string()));
    }

    // 嘗試獲取 permit（非阻塞）
    let permit = match self.semaphore.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return Ok(None),  // 沒有空閒 worker → 回傳 None
    };

    let job_id = job.job_id();

    // 將 job spawn 到 JoinSet
    let mut jobs = self.active_jobs.lock().await;
    jobs.spawn(async move {
        let _permit = permit;  // 持有 permit 直到任務完成
        job.execute().await    // 執行任務
    });
    drop(jobs);  // 明確釋放鎖

    Ok(Some(JobHandle { job_id }))
}
```

**Semaphore 的精妙用法**:
1. `try_acquire_owned()` — 非阻塞獲取，失敗時返回 `None` 而非等待
2. `OwnedSemaphorePermit` — owned 版本的 permit，可以 move 進 async block
3. `let _permit = permit` — permit 的 Drop impl 會自動釋放 semaphore slot
4. 效果：worker 數量 = semaphore slots，一個 worker 完成 → permit drop → slot 釋放 → 新 job 可提交

### 3.4 輪詢完成 (poll_completed)

**檔案**: `worker_pool.rs` 第 128-141 行

```rust
pub async fn poll_completed(&self) -> Vec<Result<JobResult, JobError>> {
    let mut results = Vec::new();
    let mut jobs = self.active_jobs.lock().await;

    // 非阻塞收集已完成的任務
    while let Some(result) = jobs.try_join_next() {
        match result {
            Ok(job_result) => results.push(job_result),
            Err(e) => results.push(Err(JobError::from(
                format!("Job panicked: {}", e)
            ))),
        }
    }
    results
}
```

**`try_join_next()` vs `join_next().await`**:
- `try_join_next()` — 非阻塞，如果沒有已完成的 job 立即返回 `None`
- `join_next().await` — 阻塞等待下一個 job 完成（用於 `shutdown()`）
- 這使排程器的主迴圈保持響應性 — 不會被卡在等待某個 job 完成

### 3.5 優雅關閉 (shutdown)

**檔案**: `worker_pool.rs` 第 146-162 行

```rust
pub async fn shutdown(&self) -> Result<Vec<Result<JobResult, JobError>>, JobError> {
    // 標記為正在關閉
    *self.shutting_down.lock().await = true;

    // 等待所有活動 job 完成
    let mut results = Vec::new();
    let mut jobs = self.active_jobs.lock().await;
    while let Some(result) = jobs.join_next().await {
        match result {
            Ok(job_result) => results.push(job_result),
            Err(e) => results.push(Err(JobError::from(format!("Job panicked: {}", e)))),
        }
    }
    Ok(results)
}
```

**關閉語義**:
1. 設定 `shutting_down = true` → 新的 `try_submit()` 立即回傳 `Err`
2. `join_next().await` — 阻塞等待所有正在執行的 job 完成
3. 不會 cancel 正在執行的 job — 讓它們自然完成

### 3.6 Worker 計數器

**檔案**: `worker_pool.rs` 第 78-91 行

```rust
pub fn total_workers(&self) -> usize { self.workers }
pub fn active_workers(&self) -> usize { self.workers - self.semaphore.available_permits() }
pub fn available_workers(&self) -> usize { self.semaphore.available_permits() }
```

**精妙之處**: 利用 Semaphore 的 `available_permits()` 計算活躍 worker 數 — 不需要額外的計數器。`total - available = active`。

### 3.7 測試覆蓋

**檔案**: `worker_pool.rs` 第 165-487 行

6 個測試涵蓋：
- `test_worker_pool_basic_execution` — 基本提交和完成
- `test_worker_pool_concurrent_execution` — 4 個 job / 2 workers
- `test_worker_pool_concurrency_limit` — 驗證 3 job / 2 workers 耗時 >=150ms
- `test_worker_pool_error_handling` — job 失敗時的錯誤傳播
- `test_worker_pool_try_submit` — 池滿時返回 None
- `test_worker_pool_shutdown` — 關閉後拒絕新提交
- `test_worker_pool_worker_counts` — 計數器準確性

> **Clawtex 實作建議**: 直接移植 `WorkerPool` 到 clawtex-core 的 `src/cluster_hub.rs`。WorkerPool 是一個純粹的並行控制元件，不依賴 tsk 的其他模組。僅需修改 `AsyncJob` trait 的方法簽名適配 clawtex 的 Hand/Cluster 任務模型。

```rust
// clawtex 建議的 WorkerPool 應用
pub struct ClusterWorkerPool {
    pool: WorkerPool<HandJob>,
}

pub struct HandJob {
    hand_name: String,
    prompt: String,
    config_snapshot: HandConfig,
}

impl AsyncJob for HandJob {
    async fn execute(self) -> Result<JobResult, JobError> {
        let runner = HandRunner::new(self.config_snapshot);
        match runner.run(&self.prompt).await {
            Ok(result) => Ok(JobResult {
                job_id: self.hand_name,
                success: true,
                message: Some(result),
            }),
            Err(e) => Err(JobError::from(e.to_string())),
        }
    }

    fn job_id(&self) -> String { self.hand_name.clone() }
}
```

---

## 4. TaskScheduler — 排程核心深度解析

### 4.1 結構定義

**檔案**: `src/server/scheduler.rs`

```rust
pub struct TaskScheduler {
    app_context: Arc<AppContext>,
    docker_manager: DockerManager,
    worker_pool: WorkerPool<TaskJob>,
    submitted_tasks: Arc<Mutex<HashSet<String>>>,   // 防止重複排程
    warmup_wait_until: Arc<Mutex<Option<Instant>>>, // 預熱失敗退避
    event_sender: Option<ServerEventSender>,
    last_oauth_check: Arc<Mutex<Option<Instant>>>,  // OAuth 檢查節流
}
```

### 4.2 排程主迴圈

**檔案**: `scheduler.rs` 排程核心邏輯

```
loop {
    ┌─────────────────────────────────────────────┐
    │  1. poll_completed()                        │
    │     收集已完成的 worker 結果                  │
    │     處理每個結果 (process_result)             │
    └───────────┬─────────────────────────────────┘
                │
    ┌───────────▼─────────────────────────────────┐
    │  2. process_results()                       │
    │     ├─ 成功 → emit TaskCompleted event      │
    │     ├─ 失敗 → emit TaskFailed event         │
    │     │          cascade_to_child_tasks()      │
    │     └─ 從 submitted_tasks 移除              │
    └───────────┬─────────────────────────────────┘
                │
    ┌───────────▼─────────────────────────────────┐
    │  3. check_oauth_token()                     │
    │     每 60 秒檢查一次 Claude OAuth 是否有效   │
    │     過期 → 設定 warmup_wait_until            │
    └───────────┬─────────────────────────────────┘
                │
    ┌───────────▼─────────────────────────────────┐
    │  4. auto_clean_tasks()                      │
    │     清理 7 天前完成的任務                     │
    │     (可配置 auto_clean_age_days)             │
    └───────────┬─────────────────────────────────┘
                │
    ┌───────────▼─────────────────────────────────┐
    │  5. select_schedulable_tasks()              │
    │     篩選狀態為 Queued 的任務                 │
    │     排除已在 submitted_tasks 中的            │
    │     排除 warmup_wait 期間的                  │
    └───────────┬─────────────────────────────────┘
                │
    ┌───────────▼─────────────────────────────────┐
    │  6. for each schedulable task:              │
    │     ├─ check_parent_status()                │
    │     │   ├─ Ready(parent) → prepare_child    │
    │     │   ├─ Waiting → skip                   │
    │     │   ├─ Failed → cascade fail            │
    │     │   ├─ Cancelled → cascade cancel       │
    │     │   └─ NotFound → mark failed           │
    │     │                                       │
    │     ├─ prepare_child_task()                  │
    │     │   複製父任務的倉庫到子任務路徑          │
    │     │   更新 source_commit                   │
    │     │   繼承父任務的 resolved_config          │
    │     │                                       │
    │     └─ worker_pool.try_submit(TaskJob)      │
    │         加入 submitted_tasks                 │
    └───────────┬─────────────────────────────────┘
                │
    ┌───────────▼─────────────────────────────────┐
    │  7. update_terminal_title()                 │
    │     格式: "tsk: N running, M queued"        │
    └───────────┬─────────────────────────────────┘
                │
                ▼
         sleep(1s) → 回到步驟 1
```

### 4.3 ParentStatus 枚舉

**檔案**: `scheduler.rs` 第 20-31 行

```rust
#[derive(Debug)]
enum ParentStatus {
    Ready(Box<Task>),    // 父任務完成，含完整 Task 用於倉庫準備
    Waiting,             // 父任務仍在排隊或執行中
    Failed(String),      // 父任務失敗，含錯誤訊息
    Cancelled,           // 父任務被取消
    NotFound(String),    // 父任務在儲存中找不到
}
```

### 4.4 級聯失敗機制

```rust
async fn cascade_to_child_tasks(&self, parent_id: &str, status: TaskStatus) {
    let all_tasks = self.task_storage.list_tasks().await.unwrap_or_default();
    let descendant_ids = TaskManager::find_descendant_tasks(&all_tasks, parent_id);

    for task_id in descendant_ids {
        match status {
            TaskStatus::Failed => {
                let msg = format!("Parent task {} failed", parent_id);
                self.task_storage.mark_failed(&task_id, &msg).await;
            },
            TaskStatus::Cancelled => {
                self.task_storage.mark_cancelled(&task_id).await;
            },
            _ => {}
        }
    }
}
```

**BFS 後代搜尋** (`task_manager.rs`):

```rust
pub fn find_descendant_tasks(all_tasks: &[Task], parent_id: &str) -> Vec<String> {
    let mut descendants = Vec::new();
    let mut queue = VecDeque::new();
    queue.push_back(parent_id.to_string());

    while let Some(current_id) = queue.pop_front() {
        for task in all_tasks {
            if task.parent_ids.contains(&current_id) {
                descendants.push(task.id.clone());
                queue.push_back(task.id.clone());
            }
        }
    }
    descendants
}
```

### 4.5 OAuth Token 預檢

```rust
async fn check_oauth_token(&self) {
    // 節流：每 60 秒只檢查一次
    if let Some(last_check) = *self.last_oauth_check.lock().await {
        if last_check.elapsed() < OAUTH_RETRY_INTERVAL {
            return;
        }
    }

    match check_oauth_token_validity().await {
        OAuthTokenStatus::Valid => { /* OK */ },
        OAuthTokenStatus::Expired => {
            // 設定退避等待
            *self.warmup_wait_until.lock().await =
                Some(Instant::now() + OAUTH_RETRY_INTERVAL);
        },
        OAuthTokenStatus::Unknown => { /* 忽略 */ },
    }

    *self.last_oauth_check.lock().await = Some(Instant::now());
}
```

### 4.6 重複排程防護

```rust
// submitted_tasks HashSet 防止同一任務被多次提交
if self.submitted_tasks.lock().await.contains(&task.id) {
    continue;  // 跳過已提交的任務
}

// 提交成功後加入 set
self.submitted_tasks.lock().await.insert(task.id.clone());

// 完成後移除
self.submitted_tasks.lock().await.remove(&task.id);
```

> **Clawtex 實作建議**: clawtex 的 cron 排程和 Hands 執行可以借鑑 TaskScheduler 的模式：(1) 防重複的 `submitted_tasks` HashSet，(2) OAuth/API key 預檢機制，(3) 級聯失敗/取消。特別是 cron job 可能在上一次執行未完成時又觸發新一次，`submitted_tasks` 模式可以防止這種情況。

```rust
// clawtex 建議的 cron 排程器
pub struct CronScheduler {
    running_jobs: Arc<Mutex<HashSet<String>>>,  // 正在執行的 cron job 名稱
    worker_pool: WorkerPool<CronJob>,
}

impl CronScheduler {
    pub async fn tick(&self) {
        let due_jobs = self.get_due_cron_jobs().await;
        for job in due_jobs {
            // 防重複
            if self.running_jobs.lock().await.contains(&job.name) {
                tracing::warn!("Cron job {} still running, skipping", job.name);
                continue;
            }
            if let Some(_handle) = self.worker_pool.try_submit(job.clone()).await? {
                self.running_jobs.lock().await.insert(job.name.clone());
            }
        }
    }
}
```

---

## 5. Config Snapshot 系統

### 5.1 設計動機

tsk 在任務建立時將配置序列化為 JSON 快照，存入 Task 結構的 `resolved_config` 欄位。執行時使用快照而非即時讀取配置。

**檔案**: `src/task.rs` 第 116-121 行

```rust
pub struct Task {
    // ...
    /// Serialized JSON of the fully-resolved ResolvedConfig at task creation time.
    /// Used at execution time instead of re-resolving from config files.
    /// None for tasks created before this feature (falls back to live resolution).
    #[serde(default)]
    pub resolved_config: Option<String>,
}
```

### 5.2 TskConfig 分層解析

**檔案**: `src/context/tsk_config.rs`

```rust
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TskConfig {
    pub container_engine: ContainerEngine,
    pub server: ServerConfig,
    pub defaults: SharedConfig,                           // 全域預設
    pub project: HashMap<String, SharedConfig>,            // 專案覆蓋
}
```

**分層配置解析** (`resolve_config` 方法):

```
解析優先級（後者覆蓋前者）:
1. 內建預設值 (built-in defaults)
2. 使用者 [defaults] 段落 (~/.config/tsk/tsk.toml)
3. 專案 .tsk/tsk.toml
4. 使用者 [project.<name>] 段落
5. CLI 參數
```

```rust
pub fn resolve_config(&self, project_name: &str,
    project_config: Option<&SharedConfig>) -> ResolvedConfig
{
    // 1. 從 defaults 開始
    let mut resolved = ResolvedConfig::from(&self.defaults);

    // 2. 合併專案級配置
    if let Some(proj_config) = project_config {
        resolved.merge(proj_config);
    }

    // 3. 合併使用者的專案覆蓋
    if let Some(user_project) = self.project.get(project_name) {
        resolved.merge(user_project);
    }

    resolved
}
```

**合併語義**:
- **純量值** (agent, stack, memory_gb): first-set 優先級
- **列表值** (volumes, env, host_ports): 跨層合併，高優先級勝出衝突
- **Map 值** (stack_config, agent_config): 合併所有名稱，同名高優先級替換

### 5.3 快照化流程

```
TaskBuilder.build()
  │
  ├─ resolve_config(project_name, project_config)
  │   └─ ResolvedConfig { agent, stack, memory_gb, cpu, ... }
  │
  ├─ serde_json::to_string(&resolved_config)
  │   └─ JSON 字串
  │
  └─ Task { resolved_config: Some(json_string), ... }
```

### 5.4 子任務配置繼承

**檔案**: `scheduler.rs` — `prepare_child_task()` 邏輯

```rust
// 子任務繼承父任務的 resolved_config
if let Some(parent_config) = &parent_task.resolved_config {
    child_task.resolved_config = Some(parent_config.clone());
}
```

**安全性考量**: 防止代理在執行中修改配置來放鬆安全性限制。子任務使用父任務建立時的快照，而非當前的配置狀態。

> **Clawtex 實作建議**: clawtex 的 Hands 系統應在 Hand 開始執行時快照配置。

```rust
// hands/runner.rs 中新增
pub struct HandExecutionContext {
    hand_config_snapshot: HandConfig,          // hand.toml 快照
    agents_config_snapshot: AgentsConfig,      // agents.toml 快照
    start_time: Instant,
    phase_outcomes: HashMap<String, String>,
}

impl HandRunner {
    pub async fn run_with_snapshot(&self, prompt: &str) -> Result<String> {
        // 在開始時快照配置
        let snapshot = HandExecutionContext {
            hand_config_snapshot: self.hand_config.clone(),
            agents_config_snapshot: self.agents_config.clone(),
            start_time: Instant::now(),
            phase_outcomes: HashMap::new(),
        };

        // 使用快照而非即時配置
        for phase in &snapshot.hand_config_snapshot.phases {
            let result = self.execute_phase_with_context(phase, &snapshot).await?;
            snapshot.phase_outcomes.insert(phase.name.clone(), result);
        }

        Ok(snapshot.phase_outcomes.values().last().cloned().unwrap_or_default())
    }
}
```

---

## 6. DAG Task Chaining

### 6.1 parent_ids 設計

**檔案**: `src/task.rs` 第 101-109 行

```rust
pub struct Task {
    /// Parent task IDs that this task is chained to.
    #[serde(
        default,
        rename = "parent_id",
        deserialize_with = "deserialize_parent_id",    // 向後相容 JSON
        serialize_with = "serialize_parent_id"
    )]
    pub parent_ids: Vec<String>,
}
```

**向後相容**: 原始 JSON 格式使用 `parent_id: Option<String>`，遷移到 SQLite 後使用 `parent_ids: Vec<String>` 支援多父任務。自訂的 serde 序列化/反序列化確保舊格式能正確讀取。

### 6.2 任務鏈執行流程

```
使用者:
  tsk add -t feat -n add-api -p "Add REST API"           → Task A (Queued)
  tsk add -t feat -n add-tests -p "Add tests" --parent A  → Task B (Queued)
  tsk add -t feat -n deploy -p "Deploy" --parent B         → Task C (Queued)

排程器:
  ┌──────────────────────────────────────────────────────┐
  │ Iteration 1:                                         │
  │   Task A: parent_ids=[] → Ready → submit to pool     │
  │   Task B: parent_ids=[A] → check_parent_status(A)    │
  │     → A is Running → Waiting → skip                  │
  │   Task C: parent_ids=[B] → check_parent_status(B)    │
  │     → B is Queued → Waiting → skip                   │
  │                                                      │
  │ Iteration N (A completes):                           │
  │   Task B: parent_ids=[A] → check_parent_status(A)    │
  │     → A is Complete → Ready(A)                        │
  │     → prepare_child_task(B, from=A's repo)            │
  │       ├─ copy A's completed repo → B's task dir       │
  │       ├─ update B.source_commit = A's HEAD            │
  │       └─ B.resolved_config = A.resolved_config        │
  │     → submit B to pool                                │
  │                                                      │
  │ Iteration M (B completes):                           │
  │   Task C: similar to above                            │
  │                                                      │
  │ 級聯失敗 (if A fails):                               │
  │   cascade_to_child_tasks(A, Failed)                   │
  │     → B marked Failed ("Parent task A failed")        │
  │     → C marked Failed (BFS finds C as descendant of A)│
  └──────────────────────────────────────────────────────┘
```

### 6.3 prepare_child_task 深度分析

```rust
async fn prepare_child_task(
    &self,
    child_task: &Task,
    parent_task: &Task,
) -> Result<(), String>
{
    let parent_repo = parent_task.copied_repo_path.as_ref()
        .ok_or("Parent has no repo")?;

    let child_repo = self.tsk_env.task_dir(&child_task.id).join("repo");

    // 1. 複製父任務的完成倉庫（含所有變更）
    file_system::copy_dir(parent_repo, &child_repo).await
        .map_err(|e| format!("Failed to copy repo: {e}"))?;

    // 2. 取得父任務的最新 commit
    let parent_head = git_operations::get_head_commit(parent_repo)?;

    // 3. 更新 SQLite 中子任務的資訊
    self.task_storage.prepare_child_task(
        &child_task.id,
        &child_repo,
        &parent_head,
        parent_task.source_branch.as_deref(),
        parent_task.resolved_config.as_deref(),
    ).await?;

    Ok(())
}
```

> **Clawtex 實作建議**: clawtex 的 `chain_to` 機制可以增強為類似 tsk 的模式——子 Hand 繼承父 Hand 的工作區狀態和配置快照。

```rust
// hands/runner.rs 中增強 chain_to
pub async fn chain_to_hand(
    &self,
    child_hand_name: &str,
    parent_context: &HandExecutionContext,
) -> Result<String> {
    let child_config = load_hand_config(child_hand_name)?;

    // 繼承父 Hand 的配置快照
    let child_context = HandExecutionContext {
        hand_config_snapshot: child_config,
        agents_config_snapshot: parent_context.agents_config_snapshot.clone(),
        start_time: Instant::now(),
        // 繼承父 Hand 的所有 phase 結果
        phase_outcomes: parent_context.phase_outcomes.clone(),
    };

    let child_runner = HandRunner::with_context(child_context);
    child_runner.run(&format!(
        "繼續執行。前序結果: {:?}",
        parent_context.phase_outcomes
    )).await
}
```

---

## 7. TaskBuilder — 任務建構流水線

### 7.1 Builder 結構

**檔案**: `src/task_builder.rs`

```rust
pub struct TaskBuilder {
    repo_root: Option<PathBuf>,
    name: Option<String>,
    task_type: Option<String>,
    prompt: Option<String>,
    prompt_file_path: Option<PathBuf>,
    existing_instructions_file: Option<PathBuf>,
    edit: bool,
    agent: Option<String>,
    stack: Option<String>,
    project: Option<String>,
    copied_repo_path: Option<PathBuf>,
    is_interactive: bool,
    parent_id: Option<String>,
    network_isolation: bool,
    dind: Option<bool>,
    repo_copy_source: Option<PathBuf>,
}
```

### 7.2 建構流水線

```
TaskBuilder::new()
  │
  ├─ .repo_root(path)         // 設定倉庫根目錄
  ├─ .name("my-feature")      // 人類可讀名稱
  ├─ .task_type("feat")       // 任務類型
  ├─ .prompt("Add login")     // 使用者提示
  ├─ .agent("claude")         // AI 代理
  │
  └─ .build(ctx)              // 非同步建構
       │
       ├─ 1. 解析倉庫根目錄
       │     find_repo_root()
       │
       ├─ 2. 自動偵測技術棧
       │     detect_stack(repo_root)
       │     ├─ Cargo.toml → "rust"
       │     ├─ package.json → "node"
       │     ├─ go.mod → "go"
       │     ├─ requirements.txt → "python"
       │     └─ 其他 → "default"
       │
       ├─ 3. 產生唯一 ID
       │     nanoid!(8, &TASK_ID_ALPHABET)
       │     // 63 字元集（排除 `-`）
       │
       ├─ 4. 找到並處理模板
       │     find_template(task_type)
       │     ├─ .tsk/templates/{type}.md     (專案級)
       │     ├─ ~/.config/tsk/templates/{type}.md (使用者級)
       │     └─ templates/{type}.md          (內建 rust-embed)
       │
       │     strip_frontmatter(template)
       │     template.replace("{{PROMPT}}", prompt)
       │
       ├─ 5. 寫入 instructions.md
       │     task_dir/instructions.md
       │
       ├─ 6. 複製倉庫至隔離目錄
       │     file_system::copy_dir(repo_root, task_dir/repo)
       │     git_operations::create_branch(branch_name)
       │
       ├─ 7. 快照化配置
       │     tsk_config.resolve_config(project, project_config)
       │     serde_json::to_string(&resolved_config)
       │
       └─ 8. 建立 Task struct
             Task::new(id, repo_root, name, task_type, ...)
```

### 7.3 Task ID 生成

```rust
const TASK_ID_ALPHABET: [char; 63] = [
    'A'..'Z', 'a'..'z', '0'..'9', '_'
];
// 排除 '-' 防止被誤認為 CLI 參數
```

### 7.4 from_existing (重試支援)

```rust
pub fn from_existing(existing_task: &Task) -> Self {
    Self {
        repo_root: Some(existing_task.repo_root.clone()),
        name: Some(existing_task.name.clone()),
        task_type: Some(existing_task.task_type.clone()),
        agent: Some(existing_task.agent.clone()),
        existing_instructions_file: Some(PathBuf::from(&existing_task.instructions_file)),
        // ... 複製其他欄位
        ..Default::default()
    }
}
```

---

## 8. Agent 整合與日誌處理

### 8.1 Agent Trait

**檔案**: `src/agent/mod.rs`

```rust
#[async_trait]
pub trait Agent: Send + Sync {
    // 必要方法
    fn build_command(&self, instruction_path: &str, is_interactive: bool) -> Vec<String>;
    fn volumes(&self) -> Vec<(String, String, String)>;
    fn environment(&self) -> Vec<(String, String)>;
    fn create_log_processor(&self, task: Option<&Task>) -> Box<dyn LogProcessor>;
    fn name(&self) -> &str;

    // 可選方法（有預設實作）
    async fn validate(&self) -> Result<(), String> { Ok(()) }
    async fn warmup(&self) -> Result<(), String> { Ok(()) }
    fn version(&self) -> String { "unknown".to_string() }
    fn files_to_copy(&self) -> Vec<(Vec<u8>, String)> { vec![] }
}
```

**Trait 設計分析**:
- `build_command` — 產生容器內執行的 shell 命令（不同代理有不同 CLI）
- `volumes` — 掛載卷（如 `~/.claude`）
- `environment` — 環境變數注入
- `create_log_processor` — 工廠方法，每個代理有自己的輸出格式解析器
- `validate` — 預先驗證（如檢查是否已登入）
- `warmup` — 預熱步驟（如 OAuth token 刷新）
- `version` — 版本追蹤，變更時觸發映像重建
- `files_to_copy` — tar 歸檔方式複製檔案到容器

### 8.2 LogLine 結構化日誌

**檔案**: `src/agent/log_line.rs`

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LogLine {
    Message {
        level: Level,           // Info / Success / Warning / Error
        tags: Vec<String>,      // 如 ["tsk"], ["opus-4"]
        tool: Option<String>,   // 如 "Bash", "Read"
        message: String,
    },
    Todo {
        tags: Vec<String>,
        items: Vec<TodoItem>,   // 結構化待辦清單
    },
    Summary {
        success: bool,
        message: String,
        cost_usd: Option<f64>,
        duration_ms: Option<u64>,
        num_turns: Option<u64>,
    },
}
```

**三種日誌類型**:
1. **Message**: 通用日誌行（工具呼叫、結果、錯誤）
2. **Todo**: 結構化待辦清單更新（追蹤 Claude 的 TodoWrite 操作）
3. **Summary**: 最終任務摘要（成功/失敗、成本、時間、輪數）

### 8.3 LogProcessor Trait

**檔案**: `src/agent/log_processor.rs`

```rust
#[async_trait]
pub trait LogProcessor: Send {
    fn process_line(&mut self, line: &str) -> Option<LogLine>;
    fn get_final_result(&self) -> Option<&TaskResult>;
}
```

**每個代理有專屬 LogProcessor**:
- `ClaudeLogProcessor` — 解析 `--output-format stream-json`
- `CodexLogProcessor` — 解析 JSONL (thread.started, turn.completed...)
- `NoOpLogProcessor` — 直接傳遞文字

### 8.4 Claude LogProcessor 深度分析

**檔案**: `src/agent/claude/claude_log_processor.rs`

```rust
// Claude Code 輸出的事件類型:
struct ClaudeMessage {
    message_type: String,      // "system", "assistant", "result"
    subtype: Option<String>,   // "tool_use", "tool_result", "text", "progress"
    message: Option<MessageContent>,
    cost_usd: Option<f64>,
    is_error: Option<bool>,
    duration_ms: Option<u64>,
    num_turns: Option<u64>,
    result: Option<String>,
}

// 處理邏輯:
// 1. system → 提取模型名稱作為 tag
// 2. assistant + tool_use → LogLine::Message { tool: "Bash" }
// 3. assistant + text → LogLine::Message { message: "..." }
// 4. assistant + progress + task_progress → LogLine::Todo { items }
// 5. result → LogLine::Summary { cost_usd, duration_ms }
```

**TodoWrite 進度追蹤**: Claude Code 透過 `subtype: "progress"` + `content_type: "task_progress"` 發送結構化的待辦清單更新。tsk 解析為 `LogLine::Todo`，在 TUI 中以 checkbox 格式即時顯示代理的工作進度。

### 8.5 TaskLogger 雙通道

**檔案**: `src/agent/task_logger.rs`

```rust
pub struct TaskLogger {
    file: Mutex<Option<File>>,
    suppress_stdout: bool,
}

impl TaskLogger {
    pub fn log(&self, line: LogLine) {
        // 1. 寫入檔案（JSON 格式）
        if let Some(file) = self.file.lock().unwrap().as_mut() {
            writeln!(file, "{}", serde_json::to_string(&line).unwrap());
        }

        // 2. 寫入 stdout/stderr（純文字格式）
        if !self.suppress_stdout {
            match &line {
                LogLine::Message { level: Level::Warning | Level::Error, .. } =>
                    eprintln!("{}", format_log_line(&line)),
                _ =>
                    println!("{}", format_log_line(&line)),
            }
        }
    }
}
```

**`suppress_stdout`**: 在 TUI 模式下為 `true`，避免直接 stdout 輸出干擾 ratatui 介面。TUI 透過 `ServerEvent` channel 接收事件。

> **Clawtex 實作建議**: clawtex 的 provider 系統可以引入類似的結構化日誌層，統一不同 provider 的輸出格式。

```rust
// clawtex 建議的結構化日誌
#[derive(Serialize, Deserialize)]
pub enum AgentLogLine {
    ToolCall {
        tool_name: String,
        args: serde_json::Value,
        session_id: String,
    },
    ToolResult {
        tool_name: String,
        result: String,
        duration_ms: u64,
    },
    LlmResponse {
        model: String,
        content: String,
        tokens_used: u32,
        cost_usd: Option<f64>,
    },
    PhaseComplete {
        hand_name: String,
        phase_name: String,
        success: bool,
    },
}

// 整合到 Telegram 輸出
pub fn format_for_telegram(line: &AgentLogLine) -> String {
    match line {
        AgentLogLine::ToolCall { tool_name, .. } =>
            format!(">>> 使用工具: {}", tool_name),
        AgentLogLine::PhaseComplete { phase_name, success, .. } =>
            format!("{} Phase {} {}", if *success { "v" } else { "x" }, phase_name,
                if *success { "完成" } else { "失敗" }),
        _ => String::new(),
    }
}
```

---

## 9. Docker 容器執行

### 9.1 分層映像建構

```
┌──────────────────────────────────────────┐
│  Agent Layer (claude/codex/no-op)         │
│  ├─ npm install -g @anthropic-ai/claude   │
│  └─ 或 codex 安裝                        │
├──────────────────────────────────────────┤
│  Project Layer (專案特定)                 │
│  └─ 可選的自訂 setup 指令                 │
├──────────────────────────────────────────┤
│  Stack Layer (rust/python/node/go...)     │
│  ├─ 語言工具鏈安裝                       │
│  └─ stack_config.setup 自訂              │
├──────────────────────────────────────────┤
│  Base Layer (default)                     │
│  ├─ Ubuntu + git + git-lfs               │
│  └─ agent 使用者 (非 root)               │
└──────────────────────────────────────────┘
```

### 9.2 TaskRunner 執行流程

**檔案**: `src/task_runner.rs` 第 169-325 行

```rust
async fn run_in_container(&self, task: &Task)
    -> Result<TaskExecutionResult, TaskExecutionError>
{
    // 1. 取得代理
    let agent = AgentProvider::get_agent(&task.agent, self.ctx.tsk_env())?;

    // 2. 驗證代理
    agent.validate().await?;

    // 3. 確認倉庫路徑
    let repo_path = task.copied_repo_path.as_ref()
        .ok_or("Task has no copied repository")?;

    // 4. 建立 TaskLogger
    let task_logger = TaskLogger::new(
        self.ctx.tsk_env().open_agent_log(&task.id)?,
        self.event_sender.is_some()  // TUI 模式時 suppress stdout
    );

    // 5. 解析配置快照
    let resolved_config = resolve_config_from_task(task, &self.ctx, &self.event_sender);

    // 6. 確保 Docker 映像存在
    let docker_image_tag = task_image_manager.ensure_image(&EnsureImageOptions {
        stack: &task.stack,
        agent: &task.agent,
        project: Some(&task.project),
        build_root: Some(repo_path.as_path()),
        force_rebuild: true,  // 總是重建以取得最新變更
        logger: &task_logger,
        resolved_config: Some(&resolved_config),
    }).await?;

    // 7. 代理預熱
    agent.warmup().await.map_err(|e| TaskExecutionError {
        message: format!("Agent warmup failed: {e}"),
        is_warmup_failure: true,  // 特殊標記 → 排程器會退避重試
    })?;

    // 8. 執行容器
    let (_output, task_result) = self.docker_manager
        .run_task_container(&docker_image_tag, task, agent.as_ref())
        .await?;

    // 9. 提交 git 變更
    self.repo_manager.commit_changes(repo_path, &commit_message).await?;

    // 10. 拉取變更到原始倉庫
    self.repo_manager.fetch_changes(
        repo_path, &branch_name, &task.repo_root,
        &task.source_commit, task.source_branch.as_deref(),
        resolved_config.git_town,
    ).await?;

    // 11. 回傳結果
    if task_result.success {
        Ok(TaskExecutionResult { branch_name, message: task_result.message })
    } else {
        Err(TaskExecutionError { message: task_result.message, is_warmup_failure: false })
    }
}
```

### 9.3 ClaudeAgent 命令建構

**檔案**: `src/agent/claude.rs`

```rust
fn build_command(&self, instruction_path: &str, is_interactive: bool) -> Vec<String> {
    if is_interactive {
        // 互動模式: 顯示指令後啟動 bash
        vec!["sh", "-c", format!(
            "sleep 0.5; echo '=== Task Instructions ==='; \
             cat /instructions/{filename}; \
             echo '=== Starting Interactive Claude Code Session ==='; \
             exec /bin/bash"
        )]
    } else {
        // 自動模式: 管道輸入到 claude CLI
        vec!["sh", "-c", format!(
            "cat /instructions/{filename} | claude -p --verbose \
             --output-format stream-json --dangerously-skip-permissions \
             2>&1 | tee /output/claude-log.txt"
        )]
    }
}
```

**參數說明**:
- `-p` — 管道模式（從 stdin 讀取）
- `--verbose` — 詳細輸出
- `--output-format stream-json` — JSON 串流（LogProcessor 解析用）
- `--dangerously-skip-permissions` — YOLO 模式（跳過所有確認）
- `tee /output/claude-log.txt` — 同時輸出到 stdout 和日誌檔

---

## 10. TaskStorage — SQLite 持久化層

### 10.1 結構定義

**檔案**: `src/context/task_storage.rs`

```rust
pub struct TaskStorage {
    conn: Arc<StdMutex<Connection>>,  // 使用 std::sync::Mutex 而非 tokio::Mutex
}
```

**為什麼用 `StdMutex`**: `rusqlite::Connection` 不是 `Send`，不能直接在 async 上下文中使用。用 `StdMutex` 包裹後，透過 `tokio::task::spawn_blocking()` 在阻塞任務中存取。

### 10.2 Schema

```sql
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    repo_root TEXT NOT NULL,
    name TEXT NOT NULL,
    task_type TEXT NOT NULL,
    instructions_file TEXT NOT NULL,
    agent TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'QUEUED',
    created_at TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT,
    branch_name TEXT NOT NULL,
    error_message TEXT,
    source_commit TEXT NOT NULL,
    source_branch TEXT,
    stack TEXT NOT NULL DEFAULT 'default',
    project TEXT NOT NULL DEFAULT 'default',
    copied_repo_path TEXT,
    is_interactive INTEGER NOT NULL DEFAULT 0,
    parent_ids TEXT NOT NULL DEFAULT '[]',     -- JSON 陣列
    network_isolation INTEGER NOT NULL DEFAULT 1,
    dind INTEGER NOT NULL DEFAULT 0,
    resolved_config TEXT                       -- JSON 快照
)
```

### 10.3 非同步包裝模式

```rust
pub async fn get_task(&self, task_id: &str) -> Result<Option<Task>, Box<dyn Error + Send + Sync>> {
    let conn = self.conn.clone();
    let task_id = task_id.to_string();

    tokio::task::spawn_blocking(move || {
        let conn = conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM tasks WHERE id = ?1")?;
        let mut rows = stmt.query_map(params![task_id], row_to_task)?;
        Ok(rows.next().transpose()?)
    })
    .await?
}
```

### 10.4 狀態轉換方法

```rust
pub async fn mark_running(&self, task_id: &str) -> Result<...> {
    // UPDATE tasks SET status='RUNNING', started_at=NOW() WHERE id=?
}

pub async fn mark_complete(&self, task_id: &str, branch_name: &str) -> Result<...> {
    // UPDATE tasks SET status='COMPLETE', completed_at=NOW(), branch_name=? WHERE id=?
}

pub async fn mark_failed(&self, task_id: &str, error_message: &str) -> Result<...> {
    // UPDATE tasks SET status='FAILED', completed_at=NOW(), error_message=? WHERE id=?
}

pub async fn mark_cancelled(&self, task_id: &str) -> Result<...> {
    // UPDATE tasks SET status='CANCELLED', completed_at=NOW() WHERE id=?
}

pub async fn prepare_child_task(&self, task_id: &str, path: &Path,
    commit: &str, branch: Option<&str>, config: Option<&str>) -> Result<...>
{
    // UPDATE tasks SET copied_repo_path=?, source_commit=?,
    //   source_branch=?, resolved_config=? WHERE id=?
}
```

> **Clawtex 實作建議**: clawtex 已有 `core.db` (sessions) 和 `costs.db` (cost_records)。建議增加 `hands_executions` 表追蹤 Hand 的執行歷史，支援 resume 和 retry。

```sql
-- hands_executions table
CREATE TABLE IF NOT EXISTS hands_executions (
    id TEXT PRIMARY KEY,
    hand_name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'RUNNING',
    prompt TEXT NOT NULL,
    config_snapshot TEXT,              -- JSON 快照
    phase_outcomes TEXT DEFAULT '{}',  -- JSON: phase_name → result
    current_phase TEXT,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    error_message TEXT,
    parent_execution_id TEXT,          -- chain_to 父 Hand
    session_id TEXT,
    cost_usd REAL DEFAULT 0.0
);
```

```rust
// hands/storage.rs
pub struct HandExecutionStorage {
    conn: Arc<StdMutex<Connection>>,
}

impl HandExecutionStorage {
    pub async fn create_execution(&self, hand_name: &str, prompt: &str,
        config_snapshot: &str) -> Result<String>
    {
        let id = format!("hand-{}", uuid::Uuid::new_v4());
        // INSERT INTO hands_executions (id, hand_name, status, prompt, config_snapshot, started_at)
        // VALUES (?, ?, 'RUNNING', ?, ?, datetime('now'))
        Ok(id)
    }

    pub async fn update_phase(&self, execution_id: &str,
        phase_name: &str, phase_result: &str) -> Result<()>
    {
        // 更新 phase_outcomes JSON 和 current_phase
    }

    pub async fn mark_complete(&self, execution_id: &str) -> Result<()> {
        // UPDATE ... SET status='COMPLETE', completed_at=datetime('now')
    }

    pub async fn resume_execution(&self, execution_id: &str) -> Result<HandExecutionContext> {
        // 從資料庫載入快照和已完成的 phase 結果
        // 從中斷的 phase 繼續
    }
}
```

---

## 11. 錯誤處理深度分析

### 11.1 TaskExecutionError

**檔案**: `src/task_runner.rs` 第 23-35 行

```rust
#[derive(Debug)]
pub struct TaskExecutionError {
    pub message: String,
    pub is_warmup_failure: bool,
}
```

`is_warmup_failure` 標記使排程器可以採取不同策略:
- **一般失敗**: 立即標記任務為 Failed
- **預熱失敗**: 設定退避時間（60 秒），稍後重試（如 OAuth token 過期）

### 11.2 分層錯誤處理

```
TaskRunner.run_with_lifecycle()
  │
  ├──▶ run_in_container()
  │       │
  │       ├──▶ Agent.validate() 失敗
  │       │    → TaskExecutionError { is_warmup: false }
  │       │
  │       ├──▶ DockerImageManager.ensure_image() 失敗
  │       │    → LogLine::tsk_error() + TaskExecutionError
  │       │
  │       ├──▶ Agent.warmup() 失敗
  │       │    → TaskExecutionError { is_warmup_failure: true }
  │       │    → 排程器設定 warmup_wait_until
  │       │
  │       ├──▶ DockerManager.run_task_container() 失敗
  │       │    → TaskExecutionError { is_warmup: false }
  │       │
  │       └──▶ TaskResult.success == false
  │            → TaskExecutionError (代理報告失敗)
  │
  ├── 成功: mark_complete() + notify_task_complete()
  │
  └── 失敗: mark_failed() + notify_task_complete(false)
```

### 11.3 優雅關機

```rust
pub async fn graceful_shutdown(&self) {
    // 1. 停止排程器
    *self.running_flag.lock().await = false;

    // 2. Kill 所有執行中的容器
    for id in &task_ids {
        docker_client.kill_container(&format!("tsk-{id}")).await;
    }

    // 3. 等待排程器任務完成 (5秒超時)
    timeout(Duration::from_secs(5), scheduler_handle).await;

    // 4. 標記未完成的任務為 Cancelled
    for task_id in &task_ids {
        if task.status == TaskStatus::Running {
            storage.mark_cancelled(task_id).await;
        }
    }

    // 5. 停止代理容器
    proxy_manager.force_stop_proxy().await;

    // 6. 清理 PID 檔案
    lifecycle.cleanup();
}
```

### 11.4 Ctrl+C 處理

**檔案**: `src/commands/run.rs`

```rust
tokio::spawn(async move {
    let mut sigterm = signal(SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = sigterm.recv() => {},
    }
    let _ = cancel_storage.mark_cancelled(&task_id).await;
    let _ = cancel_client.kill_container(&container_name).await;
});
```

---

## 12. 效能特徵分析

### 12.1 Semaphore 效能

- `try_acquire_owned()` — O(1) 操作，原子 CAS
- `available_permits()` — O(1) 讀取
- permit drop — O(1) 原子釋放
- 無 contention 時零開銷

### 12.2 SQLite 瓶頸

- `StdMutex<Connection>` — 每次 DB 操作獲取全域鎖
- `spawn_blocking` — 每次 DB 操作消耗一個 tokio 阻塞執行緒
- 改進方案: 使用 WAL 模式 + 連接池

### 12.3 檔案複製開銷

- `copy_dir()` — 非同步遞歸複製整個倉庫
- 大型倉庫（GB 級）會導致顯著延遲
- git 物件重複佔用磁碟空間
- 改進方案: 使用 `--reference` 或硬連結

### 12.4 Docker 映像建構

- `force_rebuild: true` — 每次任務都重建映像
- 分層快取可以緩解，但仍有開銷
- `BuildLockManager` — flock 防止並行建構

---

## 13. Clawtex 差距對比與 Rust 實作建議

### 13.1 WorkerPool vs Tokio 裸 spawn

**tsk**: `WorkerPool<T>` — Semaphore + JoinSet + shutdown 語義
**clawtex**: `tokio::spawn` — 無並行度控制

```rust
// clawtex 的 WorkerPool 整合點:
// 1. ClusterHub dispatch → WorkerPool
// 2. Hands 並行 phase → WorkerPool
// 3. Cron 排程 → WorkerPool (防重複)

// src/cluster_hub.rs
pub struct ClusterHub {
    worker_pool: WorkerPool<ClusterTask>,
    // ...
}

impl ClusterHub {
    pub async fn dispatch(&self, task: ClusterTask) -> Result<()> {
        match self.worker_pool.try_submit(task).await? {
            Some(handle) => {
                tracing::info!("Task {} dispatched", handle.job_id);
                Ok(())
            },
            None => {
                tracing::warn!("All workers busy, task queued");
                // 加入等待佇列
                self.pending_queue.push(task);
                Ok(())
            }
        }
    }
}
```

> **Clawtex 實作建議**: 從 tsk 的 `worker_pool.rs` 移植 `WorkerPool`（487 行，含完整測試）。這是一個自包含的模組，只依賴 `tokio::sync::{Semaphore, Mutex}` 和 `tokio::task::JoinSet`。修改 `AsyncJob` trait 為 clawtex 的任務模型即可。

---

### 13.2 Config Snapshot vs 即時讀取

**tsk**: 任務建立時快照 `resolved_config`，執行時使用快照
**clawtex**: Hands 執行時即時讀取 `hand.toml` 和 `agents.toml`

> **Clawtex 實作建議**: 在 `HandRunner::run()` 開始時快照配置到 `HandExecutionContext`，所有 phase 使用快照中的配置。`chain_to` 的子 Hand 繼承父 Hand 的快照。將快照 JSON 存入 `hands_executions` 表支援 resume。

---

### 13.3 SQLite 持久化 vs 記憶體狀態

**tsk**: 所有任務狀態持久化到 SQLite，支援 resume/retry/list
**clawtex**: Hand 執行狀態在記憶體中，daemon 重啟丟失

```rust
// clawtex 建議的 Hand 持久化
// 在 src/hands/storage.rs 中新增

pub async fn save_phase_checkpoint(
    db: &Connection,
    execution_id: &str,
    phase_name: &str,
    result: &str,
) -> Result<()> {
    db.execute(
        "UPDATE hands_executions SET current_phase = ?1, \
         phase_outcomes = json_set(phase_outcomes, '$.' || ?1, ?2) \
         WHERE id = ?3",
        params![phase_name, result, execution_id],
    )?;
    Ok(())
}

pub async fn resume_from_checkpoint(
    db: &Connection,
    execution_id: &str,
) -> Result<(String, HashMap<String, String>)> {
    let row = db.query_row(
        "SELECT current_phase, phase_outcomes FROM hands_executions WHERE id = ?1",
        params![execution_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;
    let phase_outcomes: HashMap<String, String> = serde_json::from_str(&row.1)?;
    Ok((row.0, phase_outcomes))
}
```

> **Clawtex 實作建議**: 使用現有的 `core.db` 新增 `hands_executions` 表。每個 phase 完成後寫入 checkpoint。daemon 重啟時可以從 checkpoint 恢復。與 Telegram Bot 的 `/hands` 命令整合，顯示歷史執行記錄。

---

### 13.4 Task Chaining vs chain_to

**tsk**: `parent_ids: Vec<String>` — 支援 DAG（多父任務），級聯失敗
**clawtex**: `chain_to: String` — 線性鏈

```rust
// clawtex 建議增強 chain_to 為 DAG
// hand.toml 中:
// [settings]
// chain_from = ["research_hand", "data_gather_hand"]
// chain_to = "publish_hand"

// hands/runner.rs
pub async fn run_hand_with_dependencies(
    &self,
    hand_name: &str,
    prompt: &str,
    parent_executions: &[String],  // 父 Hand 的 execution_id
) -> Result<String> {
    // 1. 等待所有父 Hand 完成
    for parent_id in parent_executions {
        loop {
            let status = self.storage.get_execution_status(parent_id).await?;
            match status.as_str() {
                "COMPLETE" => break,
                "FAILED" | "CANCELLED" => {
                    // 級聯失敗
                    return Err(anyhow!("Parent hand {} failed", parent_id));
                },
                _ => tokio::time::sleep(Duration::from_secs(2)).await,
            }
        }
    }

    // 2. 合併父 Hand 的結果作為上下文
    let mut context = HashMap::new();
    for parent_id in parent_executions {
        let outcomes = self.storage.get_phase_outcomes(parent_id).await?;
        context.extend(outcomes);
    }

    // 3. 執行當前 Hand
    self.run_with_context(hand_name, prompt, &context).await
}
```

> **Clawtex 實作建議**: 增強 `chain_to` 為雙向（`chain_from` + `chain_to`），支援多對一和一對多的 Hand 依賴。結合 `hands_executions` 表實現級聯失敗和依賴等待。

---

### 13.5 結構化日誌 vs 原始文字

**tsk**: `LogLine` enum (Message/Todo/Summary) + `TaskLogger` 雙通道
**clawtex**: 原始 LLM 回應文字 → Telegram

> **Clawtex 實作建議**: 引入 `AgentLogLine` enum 統一不同 provider 的輸出格式。在 `agent_runtime.rs` 的 tool-calling loop 中，每次工具呼叫和 LLM 回應都生成結構化日誌。透過 Telegram 格式化輸出。

---

### 13.6 總體差距矩陣

| 特性 | tsk | Clawtex | 差距 | 優先級 |
|------|-----|---------|------|--------|
| WorkerPool | Semaphore + JoinSet | 無並行控制 | **大** | 高 |
| Config Snapshot | 任務建立時快照 | 即時讀取 | **大** | 高 |
| SQLite 持久化 | 完整任務生命週期 | 記憶體狀態 | **大** | 高 |
| Task Chaining | DAG (parent_ids) | 線性 chain_to | 中 | 中 |
| 結構化日誌 | LogLine enum | 原始文字 | 中 | 中 |
| 級聯失敗 | BFS 後代搜尋 | 無 | 中 | 中 |
| 優雅關機 | kill容器+cancel任務 | E-Stop | 不同策略 | 低 |
| Docker 沙盒 | 完全隔離 | 同進程 workspace | 設計差異 | — |
| 模板系統 | 3 層優先級 | 內嵌 TOML | 小 | 低 |
| OAuth 預檢 | 排程前檢查 | 無 | 小 | 低 |
| 重複排程防護 | submitted_tasks Set | 無 | 小 | 高 |
| TUI 面板 | ratatui + ServerEvent | Telegram Bot | 不同入口 | — |
| 代理版本追蹤 | version() → rebuild | 無 | 小 | 低 |

---

## 附錄：關鍵程式碼路徑索引

| 功能 | 檔案 | 行範圍/重點 |
|------|------|------------|
| WorkerPool 核心 | `server/worker_pool.rs` | 52-163 |
| AsyncJob trait | `server/worker_pool.rs` | 37-43 |
| try_submit (Semaphore) | `server/worker_pool.rs` | 96-123 |
| poll_completed | `server/worker_pool.rs` | 128-141 |
| shutdown | `server/worker_pool.rs` | 146-162 |
| WorkerPool 測試 | `server/worker_pool.rs` | 165-487 |
| TaskScheduler 結構 | `server/scheduler.rs` | 全檔 |
| ParentStatus enum | `server/scheduler.rs` | 20-31 |
| 排程主迴圈 | `server/scheduler.rs` | start() 方法 |
| prepare_child_task | `server/scheduler.rs` | 子任務準備邏輯 |
| cascade_to_child_tasks | `server/scheduler.rs` | 級聯失敗 |
| Task struct | `task.rs` | 56-121 |
| TaskStatus enum | `task.rs` | 36-53 |
| parent_ids serde | `task.rs` | 16-33 |
| TaskBuilder | `task_builder.rs` | 全檔 |
| TASK_ID_ALPHABET | `task_builder.rs` | 17-22 |
| TaskRunner | `task_runner.rs` | 45-325 |
| run_in_container | `task_runner.rs` | 169-325 |
| TaskExecutionError | `task_runner.rs` | 23-35 |
| Agent trait | `agent/mod.rs` | 全檔 |
| AgentProvider | `agent/provider.rs` | 全檔 |
| ClaudeAgent | `agent/claude.rs` | 全檔 |
| LogLine enum | `agent/log_line.rs` | 全檔 |
| LogProcessor trait | `agent/log_processor.rs` | 全檔 |
| ClaudeLogProcessor | `agent/claude/claude_log_processor.rs` | 全檔 |
| TaskLogger | `agent/task_logger.rs` | 全檔 |
| AppContext | `context/mod.rs` | 全檔 |
| TskConfig | `context/tsk_config.rs` | 全檔 |
| TaskStorage | `context/task_storage.rs` | 全檔 |
| Command trait | `commands/mod.rs` | 全檔 |
| ServerEvent | `tui/events.rs` | 全檔 |
| TskServer | `server/mod.rs` | 全檔 |
| graceful_shutdown | `server/mod.rs` | shutdown 方法 |
