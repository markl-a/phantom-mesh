# Clawtex 8-Node Cluster Task Scheduler 設計文件

> 日期：2026-03-05
> 作者：分散式系統架構師
> 狀態：設計稿

---

## 目錄

1. [現有架構分析](#1-現有架構分析)
2. [核心資料結構](#2-核心資料結構)
3. [排程演算法](#3-排程演算法)
4. [負載均衡策略](#4-負載均衡策略)
5. [任務類型路由](#5-任務類型路由)
6. [佇列管理](#6-佇列管理)
7. [節點通訊協定](#7-節點通訊協定)
8. [與現有模組整合](#8-與現有模組整合)
9. [Rust 實作建議](#9-rust-實作建議)
10. [測試策略](#10-測試策略)

---

## 1. 現有架構分析

### 1.1 目前的單機執行路徑

```
Telegram/HTTP → AgentRuntime::run_with_config()
                 └─ for round in 0..MAX_TOOL_ROUNDS
                      ├─ router.chat_with_tools() → ProviderRouter → Provider::chat()
                      ├─ dispatcher::parse_tool_calls()
                      └─ tool_registry.execute_tool() [join_all 並行]

Cron/Scheduler → JobExecutor(JobAction)
                  ├─ Agent { agent, prompt } → AgentRuntime::run()
                  └─ Hand { hand_name, input } → HandRunner::run()

Hands → HandRunner::run()
         └─ for phase in 0..phases.len()
              └─ run_single_phase() → AgentRuntime::run_with_config()

SoT  → SkeletonRunner::generate()
         ├─ generate_skeleton()           [單一 provider]
         └─ expand_parallel()              [tokio::spawn round-robin]
              └─ router.chat_with_tools()  [各 section 獨立]
```

### 1.2 可並行化的切入點

| 執行路徑 | 目前 | 集群後 |
|----------|------|--------|
| AgentRuntime 單輪 LLM 呼叫 | 單機 provider | 可路由到遠端節點 |
| 同一輪的多 tool calls | `join_all` 本地並行 | 可分散到多節點 |
| Hand 各 phase | 串行（有依賴） | 串行但每 phase 可選節點 |
| SoT expand_parallel | `tokio::spawn` 本地 | 天然適合分散到 8 節點 |
| Cron 同時觸發多 job | 單機串行 | 各 job 分發到不同節點 |
| 多 user 同時 chat | 共用 provider | 分流到不同節點 |

### 1.3 現有可複用的基礎設施

- **`cluster.rs`**: `ClusterRegistry` — SQLite 節點註冊表（name, host, port, status, models）
- **`task_queue.rs`**: `TaskQueue` — SQLite 任務持久化（無優先級、無節點映射）
- **`cron.rs`**: `Scheduler` — 背景排程器（30 秒輪詢）
- **`estop.rs`**: `EStop` + `Heartbeat` — 緊急停止 + 心跳偵測
- **`providers/reliable.rs`**: `ReliableProvider` — 斷路器 + 指數退避 + 故障轉移鏈
- **`providers/router.rs`**: `ProviderRouter` — hint-based 路由 + auto fallback
- **`metrics.rs`**: `MetricsRegistry` — 原子計數器/直方圖（Prometheus 格式）
- **`cost_tracker.rs`**: `CostTracker` — 每次 LLM 呼叫的成本記錄

---

## 2. 核心資料結構

### 2.1 ID 類型

```rust
use std::fmt;
use serde::{Deserialize, Serialize};

/// 節點唯一識別（對應 ClusterRegistry.name）
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct NodeId(pub String);

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 任務唯一識別
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskId(pub String);

impl TaskId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}
```

### 2.2 任務優先級

```rust
/// 5 級優先級，數值越小越優先
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Priority {
    /// 緊急：E-Stop 恢復後的清理、安全相關
    Critical = 0,
    /// 高：即時 Chat 回應（使用者正在等待）
    High = 1,
    /// 正常：Hand phase 執行、一般 agent 任務
    Normal = 2,
    /// 低：Cron 排程的批次任務
    Low = 3,
    /// 背景：SoT 擴展、預載模型、報表生成
    Background = 4,
}

impl Default for Priority {
    fn default() -> Self {
        Priority::Normal
    }
}
```

### 2.3 任務類型

```rust
/// 任務類型 — 決定路由策略和資源需求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskType {
    /// 即時對話 — 低延遲優先
    Chat {
        agent_name: String,
        user_id: String,
    },
    /// Hand 工作流的單一 phase
    HandPhase {
        hand_name: String,
        phase_index: usize,
        total_phases: usize,
        /// 用於親和性：同一 execution_id 的 phase 偏好同節點
        execution_id: String,
    },
    /// SoT 並行擴展的單一 section
    SoTExpansion {
        topic: String,
        section_index: usize,
        total_sections: usize,
        /// 父任務 ID，用於收集結果
        parent_task_id: TaskId,
    },
    /// 背景批次（cron 觸發、報表、爬蟲等）
    BackgroundBatch {
        job_name: String,
    },
    /// 程式碼生成（需要大模型或 Claude/Gemini）
    Coding {
        project_context: String,
    },
    /// 工具執行（可能需要特定節點的資源）
    ToolExecution {
        tool_name: String,
        /// 需要瀏覽器的工具（browser, twitter）只能在有 Playwright 的節點
        requires_browser: bool,
        /// 需要 GPU 的工具（vision）
        requires_gpu: bool,
    },
}
```

### 2.4 任務約束

```rust
/// 任務對執行環境的約束條件
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskConstraints {
    /// 需要的最小 VRAM (GB)，0 = 無限制
    pub min_vram_gb: u32,
    /// 需要的模型名稱（如果已知）
    pub required_model: Option<String>,
    /// 偏好的 provider（如 "ollama", "lmstudio", "anthropic"）
    pub preferred_provider: Option<String>,
    /// 是否需要瀏覽器環境（Playwright）
    pub requires_browser: bool,
    /// 是否需要 GPU
    pub requires_gpu: bool,
    /// 是否需要 NPU（XDNA 加速）
    pub requires_npu: bool,
    /// 親和性節點（偏好在此節點執行，但不強制）
    pub affinity_node: Option<NodeId>,
    /// 反親和性（避免在此節點執行）
    pub anti_affinity_nodes: Vec<NodeId>,
    /// 最大可接受延遲 (ms)，0 = 不限
    pub max_latency_ms: u64,
    /// 預估所需 token 數（用於容量規劃）
    pub estimated_tokens: u32,
}
```

### 2.5 任務結構

```rust
/// 完整的排程任務
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: TaskId,
    pub task_type: TaskType,
    pub priority: Priority,
    pub constraints: TaskConstraints,
    pub status: ScheduledTaskStatus,

    // ── 執行上下文 ────────────────────────────────────
    /// Agent 配置（序列化的 AgentConfig）
    pub agent_config: Option<String>,
    /// 提示詞
    pub prompt: String,
    /// 對話歷史（序列化的 Vec<ChatMessage>）
    pub history_json: Option<String>,
    /// 額外上下文
    pub extra_context: Option<String>,

    // ── 元數據 ────────────────────────────────────────
    /// 提交時間
    pub submitted_at: chrono::DateTime<chrono::Utc>,
    /// 開始執行時間
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 完成時間
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 指派的節點
    pub assigned_node: Option<NodeId>,
    /// 重試次數
    pub retry_count: u32,
    /// 最大重試次數
    pub max_retries: u32,
    /// 超時 (秒)
    pub timeout_secs: u64,
    /// 被搶佔的次數
    pub preemption_count: u32,
    /// 回調通道（oneshot sender 的 token，用於返回結果）
    #[serde(skip)]
    pub result_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduledTaskStatus {
    /// 在佇列中等待
    Queued,
    /// 已指派節點，等待執行
    Assigned,
    /// 正在執行
    Running,
    /// 完成
    Completed,
    /// 失敗
    Failed,
    /// 被搶佔（移回佇列）
    Preempted,
    /// 超時
    TimedOut,
    /// 在死信佇列中
    DeadLettered,
    /// 被取消
    Cancelled,
}
```

### 2.6 節點狀態

```rust
/// 單個節點的即時狀態
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeState {
    pub node_id: NodeId,
    pub host: String,
    pub port: u16,
    pub status: NodeStatus,

    // ── 硬體能力 ──────────────────────────────────────
    pub capabilities: NodeCapabilities,

    // ── 即時負載 ──────────────────────────────────────
    pub current_load: NodeLoad,

    // ── 已載入的模型 ──────────────────────────────────
    pub loaded_models: Vec<LoadedModel>,

    // ── 連線品質 ──────────────────────────────────────
    pub last_heartbeat: chrono::DateTime<chrono::Utc>,
    pub avg_latency_ms: f64,
    pub last_latency_ms: u64,

    // ── 統計 ──────────────────────────────────────────
    pub tasks_completed: u64,
    pub tasks_failed: u64,
    pub total_tokens_processed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    Online,
    Offline,
    Draining,   // 不接新任務，等待執行中的完成
    Overloaded, // 暫時超載
    Maintenance, // 維護模式
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCapabilities {
    /// CPU 核心數
    pub cpu_cores: u32,
    /// 總 RAM (GB)
    pub ram_gb: u32,
    /// GPU VRAM (GB)，0 = 無 GPU
    pub vram_gb: u32,
    /// 是否有 NPU
    pub has_npu: bool,
    /// NPU TOPS
    pub npu_tops: u32,
    /// 是否安裝了 Playwright
    pub has_browser: bool,
    /// 支援的 provider 類型
    pub available_providers: Vec<String>,
    /// 節點權重（用於 Weighted Round Robin，1-100）
    pub weight: u32,
    /// 最大並行任務數
    pub max_concurrent_tasks: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeLoad {
    /// 目前正在執行的任務數
    pub running_tasks: u32,
    /// 佇列中等待的任務數
    pub queued_tasks: u32,
    /// CPU 使用率 (0.0 - 1.0)
    pub cpu_usage: f64,
    /// RAM 使用率 (0.0 - 1.0)
    pub ram_usage: f64,
    /// VRAM 使用率 (0.0 - 1.0)
    pub vram_usage: f64,
    /// 正在使用的 VRAM (GB)
    pub vram_used_gb: f64,
    /// 可用 VRAM (GB)
    pub vram_available_gb: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadedModel {
    pub name: String,
    pub size_gb: f64,
    pub provider: String,
    /// 上次使用時間（用於 LRU 驅逐）
    pub last_used: chrono::DateTime<chrono::Utc>,
}
```

### 2.7 任務排程器主結構

```rust
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock, Notify, mpsc, oneshot};

/// 8-Node Cluster Task Scheduler
pub struct TaskScheduler {
    // ── 佇列層 ────────────────────────────────────────
    /// 主優先佇列（BinaryHeap + 自訂 Ord）
    queue: Arc<Mutex<BinaryHeap<PrioritizedTask>>>,
    /// 死信佇列（失敗超過 max_retries 的任務）
    dead_letter_queue: Arc<Mutex<Vec<ScheduledTask>>>,
    /// 等待結果的 oneshot channels
    result_channels: Arc<Mutex<HashMap<TaskId, oneshot::Sender<TaskResult>>>>,

    // ── 節點層 ────────────────────────────────────────
    /// 所有節點的即時狀態
    nodes: Arc<RwLock<HashMap<NodeId, NodeState>>>,
    /// 任務到節點的映射
    assignments: Arc<RwLock<HashMap<TaskId, NodeId>>>,
    /// 節點到其執行中任務的反向映射
    node_tasks: Arc<RwLock<HashMap<NodeId, Vec<TaskId>>>>,

    // ── 配置 ──────────────────────────────────────────
    config: SchedulerConfig,

    // ── 信號 ──────────────────────────────────────────
    /// 新任務到達通知
    notify_new_task: Arc<Notify>,
    /// 任務完成通知
    notify_task_done: Arc<Notify>,
    /// 關閉信號
    shutdown: Arc<tokio::sync::watch::Sender<bool>>,

    // ── 共用資源 ──────────────────────────────────────
    estop: Arc<EStop>,
    cost_tracker: Option<Arc<CostTracker>>,
    metrics: Arc<MetricsRegistry>,
}

#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// 主佇列最大容量
    pub max_queue_size: usize,             // default: 10000
    /// 死信佇列最大容量
    pub max_dlq_size: usize,               // default: 1000
    /// 排程策略
    pub strategy: SchedulingStrategy,      // default: Hybrid
    /// 任務預設超時 (秒)
    pub default_timeout_secs: u64,         // default: 600
    /// 任務預設最大重試次數
    pub default_max_retries: u32,          // default: 2
    /// 節點心跳超時 (秒)
    pub heartbeat_timeout_secs: u64,       // default: 30
    /// 排程循環間隔 (ms)
    pub schedule_interval_ms: u64,         // default: 100
    /// 是否啟用搶佔
    pub enable_preemption: bool,           // default: true
    /// 搶佔的優先級差值門檻（只有差 >= 2 級才搶佔）
    pub preemption_priority_gap: u32,      // default: 2
    /// Z13 本機節點 ID
    pub local_node_id: NodeId,
    /// 背壓啟動水位（佇列使用率 > 此值開始 reject Low/Background）
    pub backpressure_threshold: f64,       // default: 0.8
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulingStrategy {
    /// 加權輪詢
    WeightedRoundRobin,
    /// 最少連線
    LeastConnections,
    /// 模型親和性優先
    ModelAffinity,
    /// 混合策略（推薦）
    Hybrid,
}
```

### 2.8 優先佇列的排序包裝

```rust
/// 包裝 ScheduledTask 以實作 BinaryHeap 的排序
/// BinaryHeap 是 max-heap，我們需要 min-heap（優先級數值小的先出）
#[derive(Debug)]
struct PrioritizedTask {
    task: ScheduledTask,
    /// 有效優先級 = 基礎優先級 - aging_bonus
    effective_priority: i32,
    /// 入隊時間（用於 aging 和 FIFO 打破平手）
    enqueued_at: std::time::Instant,
}

impl Ord for PrioritizedTask {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // 1. effective_priority 小的優先（反轉比較）
        other.effective_priority.cmp(&self.effective_priority)
            // 2. 相同優先級時，先入隊的優先（FIFO）
            .then_with(|| other.enqueued_at.cmp(&self.enqueued_at))
    }
}

impl PartialOrd for PrioritizedTask {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for PrioritizedTask {}

impl PartialEq for PrioritizedTask {
    fn eq(&self, other: &Self) -> bool {
        self.task.id == other.task.id
    }
}
```

---

## 3. 排程演算法

### 3.1 混合排程演算法（Hybrid Scoring）

混合策略是推薦的預設策略。每次從佇列取出任務時，為所有可用節點計算一個**綜合分數**，選擇分數最高的節點。

```
Score(node, task) = w1 * ModelAffinityScore
                  + w2 * LoadScore
                  + w3 * HandAffinityScore
                  + w4 * LatencyScore
                  + w5 * CapacityScore
                  - penalty_if_z13

其中預設權重：
  w1 = 0.35  （模型親和性最重要，避免冷啟動）
  w2 = 0.25  （負載均衡）
  w3 = 0.15  （Hand 親和性）
  w4 = 0.15  （網路延遲）
  w5 = 0.10  （剩餘容量）
```

### 3.2 各項分數的計算

```rust
impl TaskScheduler {
    /// 為一個 (task, node) 組合計算排程分數
    /// 返回 0.0 - 1.0 之間的分數，越高越適合
    fn score_node_for_task(
        &self,
        task: &ScheduledTask,
        node: &NodeState,
        config: &SchedulerConfig,
    ) -> f64 {
        // ── 硬約束檢查（不通過直接返回 -1.0）─────────────
        if node.status != NodeStatus::Online {
            return -1.0;
        }
        if task.constraints.requires_browser && !node.capabilities.has_browser {
            return -1.0;
        }
        if task.constraints.requires_gpu && node.capabilities.vram_gb == 0 {
            return -1.0;
        }
        if task.constraints.requires_npu && !node.capabilities.has_npu {
            return -1.0;
        }
        if task.constraints.min_vram_gb > 0
            && (node.current_load.vram_available_gb as u32) < task.constraints.min_vram_gb
        {
            return -1.0;
        }
        if node.current_load.running_tasks >= node.capabilities.max_concurrent_tasks {
            return -1.0;
        }
        if task.constraints.anti_affinity_nodes.contains(&node.node_id) {
            return -1.0;
        }
        if task.constraints.max_latency_ms > 0
            && node.last_latency_ms > task.constraints.max_latency_ms
        {
            return -1.0;
        }

        let mut score = 0.0;

        // ── S1: 模型親和性 (0.0 - 1.0) ─────────────────
        let model_score = if let Some(ref required_model) = task.constraints.required_model {
            if node.loaded_models.iter().any(|m| m.name == *required_model) {
                1.0  // 模型已載入，零冷啟動
            } else if node.capabilities.available_providers.iter()
                .any(|p| Some(p) == task.constraints.preferred_provider.as_ref())
            {
                0.4  // provider 可用但模型需載入
            } else {
                0.0  // 完全不匹配
            }
        } else {
            0.5  // 無特定模型需求
        };
        score += 0.35 * model_score;

        // ── S2: 負載分數 (0.0 - 1.0，越閒越高) ─────────
        let utilization = node.current_load.running_tasks as f64
            / node.capabilities.max_concurrent_tasks.max(1) as f64;
        let load_score = 1.0 - utilization;
        score += 0.25 * load_score;

        // ── S3: Hand 親和性 (0.0 - 1.0) ─────────────────
        let hand_score = match &task.task_type {
            TaskType::HandPhase { execution_id, .. } => {
                // 檢查同一 execution_id 的其他 phase 是否在此節點
                if task.constraints.affinity_node.as_ref() == Some(&node.node_id) {
                    1.0
                } else {
                    0.0
                }
            }
            _ => 0.5, // 非 Hand 任務，中性分數
        };
        score += 0.15 * hand_score;

        // ── S4: 延遲分數 (0.0 - 1.0，越低越好) ──────────
        let latency_score = if node.avg_latency_ms < 5.0 {
            1.0  // 本地節點
        } else if node.avg_latency_ms < 20.0 {
            0.8  // 區網節點
        } else if node.avg_latency_ms < 100.0 {
            0.5  // 近端
        } else {
            0.2  // 遠端
        };
        score += 0.15 * latency_score;

        // ── S5: 容量分數 (0.0 - 1.0) ───────────────────
        let remaining_slots = node.capabilities.max_concurrent_tasks
            .saturating_sub(node.current_load.running_tasks);
        let capacity_score = remaining_slots as f64
            / node.capabilities.max_concurrent_tasks.max(1) as f64;
        score += 0.10 * capacity_score;

        // ── Z13 均衡懲罰 ────────────────────────────────
        // 避免所有任務都擠在 Z13：如果 Z13 負載 > 50% 且有其他節點空閒
        if node.node_id == config.local_node_id && utilization > 0.5 {
            score -= 0.05;
        }

        // ── 親和性加分 ──────────────────────────────────
        if task.constraints.affinity_node.as_ref() == Some(&node.node_id) {
            score += 0.10;
        }

        score.clamp(0.0, 1.0)
    }
}
```

### 3.3 Aging 防飢餓機制

```rust
impl TaskScheduler {
    /// 每隔 AGING_INTERVAL 對佇列中的任務提升有效優先級
    /// 避免低優先級任務永遠無法執行
    const AGING_INTERVAL_SECS: u64 = 30;
    const AGING_STEP: i32 = 1;       // 每次提升 1 級
    const MIN_EFFECTIVE_PRIORITY: i32 = 0; // 不會比 Critical 更高

    async fn aging_loop(&self) {
        let mut interval = tokio::time::interval(
            std::time::Duration::from_secs(Self::AGING_INTERVAL_SECS)
        );
        loop {
            interval.tick().await;
            let mut queue = self.queue.lock().await;

            // BinaryHeap 不支援原地修改，需要 drain + rebuild
            let mut tasks: Vec<PrioritizedTask> = queue.drain().collect();
            let now = std::time::Instant::now();

            for task in &mut tasks {
                let wait_secs = now.duration_since(task.enqueued_at).as_secs();
                // 每等待 AGING_INTERVAL_SECS 秒，降低 effective_priority 1 點
                let aging_bonus = (wait_secs / Self::AGING_INTERVAL_SECS) as i32 * Self::AGING_STEP;
                task.effective_priority = (task.task.priority as i32 - aging_bonus)
                    .max(Self::MIN_EFFECTIVE_PRIORITY);
            }

            *queue = tasks.into_iter().collect();
        }
    }
}
```

### 3.4 搶佔機制

```rust
impl TaskScheduler {
    /// 嘗試搶佔：當高優先級任務無可用節點時，搶佔低優先級任務
    async fn try_preempt(
        &self,
        task: &ScheduledTask,
    ) -> Option<(NodeId, TaskId)> {
        if !self.config.enable_preemption {
            return None;
        }

        let assignments = self.assignments.read().await;
        let nodes = self.nodes.read().await;
        let node_tasks = self.node_tasks.read().await;

        // 找出所有正在執行的任務中，優先級比新任務低 >= preemption_priority_gap 的
        let mut candidates: Vec<(NodeId, TaskId, Priority)> = Vec::new();

        for (task_id, node_id) in assignments.iter() {
            if let Some(node) = nodes.get(node_id) {
                // 只搶佔 Online 節點上的任務
                if node.status != NodeStatus::Online {
                    continue;
                }
                // 找到這個任務的優先級（需要從 running_tasks 裡查）
                // 這裡簡化：假設我們維護了一個 running_tasks map
                // 實際上需要一個 Arc<RwLock<HashMap<TaskId, ScheduledTask>>>
            }
        }

        // 演算法：
        // 1. 收集所有 running_task 的 (node_id, task_id, priority)
        // 2. 篩選 priority差值 >= preemption_priority_gap
        // 3. 按 priority 由大到小排序（搶最低優先級的）
        // 4. 返回第一個可搶佔的
        // 5. 發送 preempt 指令給節點 → 節點中斷執行 → 任務移回佇列

        // 偽碼：
        // for (node_id, task_id, running_priority) in candidates.sorted_desc_by_priority() {
        //     if task.priority as i32 + gap <= running_priority as i32 {
        //         return Some((node_id, task_id));
        //     }
        // }
        None
    }
}
```

### 3.5 主排程迴圈虛擬碼

```
async fn schedule_loop(self: Arc<Self>) {
    loop {
        // 等待新任務通知或定時觸發
        tokio::select! {
            _ = self.notify_new_task.notified() => {},
            _ = tokio::time::sleep(Duration::from_millis(config.schedule_interval_ms)) => {},
            _ = shutdown_rx.changed() => break,
        }

        // E-Stop 檢查
        if self.estop.is_stopped() {
            continue;
        }

        // 取出佇列頂部任務（不移除，先看看）
        let task = {
            let queue = self.queue.lock().await;
            queue.peek().cloned()
        };

        if let Some(prioritized_task) = task {
            let task = &prioritized_task.task;

            // 取得所有 Online 節點的快照
            let nodes = self.nodes.read().await;
            let online_nodes: Vec<&NodeState> = nodes.values()
                .filter(|n| n.status == NodeStatus::Online)
                .collect();

            if online_nodes.is_empty() {
                // 無可用節點，等待
                metrics.increment("scheduler_no_nodes_available");
                continue;
            }

            // 計算每個節點的分數
            let mut scored: Vec<(&NodeState, f64)> = online_nodes.iter()
                .map(|n| (*n, self.score_node_for_task(task, n, &self.config)))
                .filter(|(_, score)| *score >= 0.0)  // 過濾掉硬約束不通過的
                .collect();

            // 按分數降序排列
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            if let Some((best_node, score)) = scored.first() {
                // 正式從佇列移除
                let mut queue = self.queue.lock().await;
                let task = queue.pop().unwrap().task;

                // 指派給最佳節點
                self.assign_task(task, best_node.node_id.clone()).await;

                metrics.observe("scheduler_assignment_score", score * 100.0);
            } else {
                // 沒有合適節點 → 嘗試搶佔
                if let Some((preempt_node, preempt_task)) = self.try_preempt(task).await {
                    self.preempt_and_reassign(preempt_node, preempt_task, task.clone()).await;
                } else {
                    // 真的無法排程，任務留在佇列中
                    metrics.increment("scheduler_no_suitable_node");
                }
            }
        }
    }
}
```

---

## 4. 負載均衡策略

### 4.1 策略比較

| 策略 | 優點 | 缺點 | 適用場景 |
|------|------|------|----------|
| **Weighted Round Robin** | 簡單、公平、可預測 | 不感知模型載入 | 均質節點、SoT 擴展 |
| **Least Connections** | 自適應、避免熱點 | 可能頻繁切換模型 | 混合負載 |
| **Model Affinity** | 避免冷啟動、最佳 TTFT | 可能造成負載不均 | 模型載入成本高時 |
| **Hybrid (推薦)** | 綜合最優 | 計算開銷略高 | 生產環境 |

### 4.2 按任務類型的策略映射

```rust
impl TaskScheduler {
    /// 根據任務類型決定排程策略的權重調整
    fn weights_for_task_type(task_type: &TaskType) -> ScoringWeights {
        match task_type {
            TaskType::Chat { .. } => ScoringWeights {
                model_affinity: 0.30,
                load: 0.20,
                hand_affinity: 0.00,
                latency: 0.40,    // Chat 最重視延遲
                capacity: 0.10,
            },
            TaskType::HandPhase { .. } => ScoringWeights {
                model_affinity: 0.25,
                load: 0.20,
                hand_affinity: 0.35, // Hand 最重視親和性
                latency: 0.10,
                capacity: 0.10,
            },
            TaskType::SoTExpansion { .. } => ScoringWeights {
                model_affinity: 0.20,
                load: 0.40,       // SoT 最重視負載均衡（round-robin 效果）
                hand_affinity: 0.00,
                latency: 0.10,
                capacity: 0.30,
            },
            TaskType::BackgroundBatch { .. } => ScoringWeights {
                model_affinity: 0.15,
                load: 0.45,       // 背景任務路由到最閒節點
                hand_affinity: 0.00,
                latency: 0.05,
                capacity: 0.35,
            },
            TaskType::Coding { .. } => ScoringWeights {
                model_affinity: 0.50,  // 需要特定大模型
                load: 0.20,
                hand_affinity: 0.00,
                latency: 0.15,
                capacity: 0.15,
            },
            TaskType::ToolExecution { .. } => ScoringWeights {
                model_affinity: 0.00,  // 工具執行不需要 LLM
                load: 0.35,
                hand_affinity: 0.00,
                latency: 0.30,
                capacity: 0.35,
            },
        }
    }
}

#[derive(Debug, Clone)]
struct ScoringWeights {
    model_affinity: f64,
    load: f64,
    hand_affinity: f64,
    latency: f64,
    capacity: f64,
}
```

### 4.3 Z13 保護機制

Z13 是主控節點（Telegram bot, HTTP gateway, Scheduler 都在這裡），需要確保不被批次任務佔滿。

```rust
impl TaskScheduler {
    /// Z13 保護：限制背景任務在 Z13 上的並行度
    const Z13_BACKGROUND_LIMIT: u32 = 2;

    fn z13_guard(
        &self,
        task: &ScheduledTask,
        node: &NodeState,
    ) -> bool {
        if node.node_id != self.config.local_node_id {
            return true; // 非 Z13，不限制
        }

        match task.priority {
            Priority::Critical | Priority::High => true,  // 高優先級不限
            Priority::Normal => node.current_load.running_tasks < 4,
            Priority::Low | Priority::Background => {
                // 計算目前 Z13 上的背景任務數
                // 如果 > Z13_BACKGROUND_LIMIT，拒絕
                node.current_load.running_tasks < Self::Z13_BACKGROUND_LIMIT
            }
        }
    }
}
```

---

## 5. 任務類型路由

### 5.1 路由決策樹

```
收到任務
  │
  ├─ Chat（即時）
  │    ├─ Z13 本地 provider 可用且負載 < 50%？ → Z13 執行
  │    ├─ 有節點已載入所需模型？ → 該節點執行
  │    └─ 否則 → 最低延遲的空閒節點
  │
  ├─ Hand Phase
  │    ├─ 同 execution_id 的上一個 phase 在哪個節點？
  │    │    ├─ 該節點可用且有容量？ → 同節點（減少 context 傳輸）
  │    │    └─ 不可用 → Hybrid 評分最高的節點
  │    ├─ provider_hint 指定了特定 provider？ → 有該 provider 的節點
  │    └─ 否則 → Hybrid 評分
  │
  ├─ SoT Expansion
  │    ├─ N 個 section，M 個可用節點
  │    ├─ Round-robin: section[i] → nodes[i % M]
  │    └─ 跳過已超載或 circuit-breaker-open 的節點
  │
  ├─ Background Batch
  │    ├─ 排到最閒的節點（Least Connections）
  │    ├─ 如果所有節點負載 > 80% → 佇列中等待
  │    └─ 避免 Z13（除非只有 Z13 在線）
  │
  └─ Coding
       ├─ 需要 Claude API？ → 有 Anthropic API key 的節點
       ├─ 需要 Gemini CLI？ → 有 Gemini 設定的節點
       └─ 否則 → 有最大 VRAM 的節點（大模型）
```

### 5.2 Rust 路由實作

```rust
impl TaskScheduler {
    /// 高階路由：根據任務類型選擇初始候選節點集
    fn candidate_nodes<'a>(
        &self,
        task: &ScheduledTask,
        all_nodes: &'a HashMap<NodeId, NodeState>,
    ) -> Vec<&'a NodeState> {
        let online: Vec<&NodeState> = all_nodes.values()
            .filter(|n| n.status == NodeStatus::Online)
            .collect();

        match &task.task_type {
            TaskType::Chat { .. } => {
                // 優先 Z13 和低延遲節點
                let mut candidates = online.clone();
                candidates.sort_by(|a, b| {
                    a.avg_latency_ms.partial_cmp(&b.avg_latency_ms)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                candidates
            }

            TaskType::HandPhase { execution_id, .. } => {
                // 親和性優先
                if let Some(ref affinity) = task.constraints.affinity_node {
                    if let Some(node) = all_nodes.get(affinity) {
                        if node.status == NodeStatus::Online
                            && node.current_load.running_tasks < node.capabilities.max_concurrent_tasks
                        {
                            return vec![node]; // 強親和性
                        }
                    }
                }
                online
            }

            TaskType::SoTExpansion { section_index, .. } => {
                // Round-robin：按 section_index 排序節點
                let mut candidates = online.clone();
                // 旋轉列表使得 section_index 對應不同起始位置
                if !candidates.is_empty() {
                    let rotate = section_index % candidates.len();
                    candidates.rotate_left(rotate);
                }
                candidates
            }

            TaskType::BackgroundBatch { .. } => {
                // 排除 Z13（除非只有 Z13）
                let non_z13: Vec<&NodeState> = online.iter()
                    .filter(|n| n.node_id != self.config.local_node_id)
                    .copied()
                    .collect();
                if non_z13.is_empty() { online } else { non_z13 }
            }

            TaskType::Coding { .. } => {
                // 按 VRAM 降序（大模型需要更多 VRAM）
                let mut candidates = online.clone();
                candidates.sort_by(|a, b| {
                    b.capabilities.vram_gb.cmp(&a.capabilities.vram_gb)
                });
                candidates
            }

            TaskType::ToolExecution { requires_browser, requires_gpu, .. } => {
                let mut candidates = online.clone();
                if *requires_browser {
                    candidates.retain(|n| n.capabilities.has_browser);
                }
                if *requires_gpu {
                    candidates.retain(|n| n.capabilities.vram_gb > 0);
                }
                candidates
            }
        }
    }
}
```

---

## 6. 佇列管理

### 6.1 佇列滿時的處理：分級背壓

```rust
impl TaskScheduler {
    /// 提交任務到排程器
    pub async fn submit(
        &self,
        task: ScheduledTask,
    ) -> Result<TaskId, SubmitError> {
        // E-Stop 檢查
        if self.estop.is_stopped() {
            return Err(SubmitError::EStopActive);
        }

        let queue = self.queue.lock().await;
        let queue_len = queue.len();
        let capacity = self.config.max_queue_size;
        let utilization = queue_len as f64 / capacity as f64;
        drop(queue);

        // ── 分級背壓策略 ─────────────────────────────
        // Level 0 (< 60%): 接受所有任務
        // Level 1 (60%-80%): 拒絕 Background
        // Level 2 (80%-95%): 拒絕 Low + Background
        // Level 3 (> 95%): 只接受 Critical + High
        match task.priority {
            Priority::Critical => {}, // 永遠接受
            Priority::High => {
                if utilization > 0.95 {
                    return Err(SubmitError::QueueFull { current: queue_len, max: capacity });
                }
            },
            Priority::Normal => {
                if utilization > 0.95 {
                    return Err(SubmitError::BackPressure { level: 3, priority: task.priority });
                }
            },
            Priority::Low => {
                if utilization > 0.80 {
                    return Err(SubmitError::BackPressure { level: 2, priority: task.priority });
                }
            },
            Priority::Background => {
                if utilization > 0.60 {
                    return Err(SubmitError::BackPressure { level: 1, priority: task.priority });
                }
            },
        }

        let task_id = task.id.clone();

        // 入隊
        let mut queue = self.queue.lock().await;
        queue.push(PrioritizedTask {
            effective_priority: task.priority as i32,
            enqueued_at: std::time::Instant::now(),
            task,
        });

        // 通知排程迴圈
        self.notify_new_task.notify_one();

        self.metrics.increment("scheduler_tasks_submitted");
        Ok(task_id)
    }
}

#[derive(Debug)]
pub enum SubmitError {
    EStopActive,
    QueueFull { current: usize, max: usize },
    BackPressure { level: u32, priority: Priority },
}
```

### 6.2 任務超時處理

```rust
impl TaskScheduler {
    /// 超時監控迴圈
    async fn timeout_monitor_loop(&self) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));

        loop {
            interval.tick().await;

            let now = chrono::Utc::now();
            let assignments = self.assignments.read().await;

            // 檢查所有正在執行的任務
            let mut timed_out = Vec::new();

            for (task_id, node_id) in assignments.iter() {
                // 需要一個 running_tasks map 來查找 started_at 和 timeout_secs
                // 偽碼：
                // if let Some(task) = running_tasks.get(task_id) {
                //     if let Some(started) = task.started_at {
                //         let elapsed = (now - started).num_seconds() as u64;
                //         if elapsed > task.timeout_secs {
                //             timed_out.push((task_id.clone(), node_id.clone()));
                //         }
                //     }
                // }
            }

            for (task_id, node_id) in timed_out {
                warn!("Task {} on node {} timed out", task_id, node_id);

                // 1. 發送取消指令給節點
                // 2. 如果 retry_count < max_retries → 移回佇列重試
                // 3. 否則 → 移入死信佇列
                self.handle_task_failure(task_id, node_id, "timeout").await;

                self.metrics.increment("scheduler_tasks_timed_out");
            }
        }
    }
}
```

### 6.3 重試策略

```rust
impl TaskScheduler {
    /// 處理任務失敗（超時、節點故障、執行錯誤）
    async fn handle_task_failure(
        &self,
        task_id: TaskId,
        failed_node: NodeId,
        reason: &str,
    ) {
        // 從 assignments 移除
        self.assignments.write().await.remove(&task_id);

        // 取得任務（需要一個 all_tasks map）
        // let mut task = running_tasks.remove(&task_id).unwrap();
        // 以下為偽碼：

        // task.retry_count += 1;
        // task.status = ScheduledTaskStatus::Queued;

        // if task.retry_count > task.max_retries {
        //     // 移入死信佇列
        //     task.status = ScheduledTaskStatus::DeadLettered;
        //     self.dead_letter_queue.lock().await.push(task);
        //     metrics.increment("scheduler_tasks_dead_lettered");
        //     warn!("Task {} moved to DLQ after {} retries (reason: {})",
        //         task_id, task.max_retries, reason);
        //     return;
        // }

        // // 指數退避：第 N 次重試等待 2^N 秒（但不超過 60 秒）
        // let backoff = Duration::from_secs(
        //     2u64.pow(task.retry_count).min(60)
        // );

        // // 加入反親和性：避免再次在失敗的節點上執行
        // task.constraints.anti_affinity_nodes.push(failed_node);

        // // 延遲後重新入隊
        // tokio::spawn(async move {
        //     tokio::time::sleep(backoff).await;
        //     self.queue.lock().await.push(PrioritizedTask { ... });
        //     self.notify_new_task.notify_one();
        // });

        // info!("Task {} retry {} (backoff: {:?}, reason: {})",
        //     task_id, task.retry_count, backoff, reason);
    }
}
```

### 6.4 死信佇列（DLQ）管理

```rust
impl TaskScheduler {
    /// 獲取死信佇列中的任務
    pub async fn list_dead_letters(&self) -> Vec<ScheduledTask> {
        self.dead_letter_queue.lock().await.clone()
    }

    /// 重新提交死信佇列中的任務（手動重試）
    pub async fn retry_dead_letter(&self, task_id: &TaskId) -> Result<(), SubmitError> {
        let mut dlq = self.dead_letter_queue.lock().await;
        if let Some(pos) = dlq.iter().position(|t| t.id == *task_id) {
            let mut task = dlq.remove(pos);
            task.retry_count = 0;
            task.max_retries = self.config.default_max_retries;
            task.status = ScheduledTaskStatus::Queued;
            task.constraints.anti_affinity_nodes.clear();
            drop(dlq);
            self.submit(task).await?;
            Ok(())
        } else {
            Err(SubmitError::QueueFull { current: 0, max: 0 }) // TaskNotFound
        }
    }

    /// 清除死信佇列
    pub async fn purge_dead_letters(&self) -> usize {
        let mut dlq = self.dead_letter_queue.lock().await;
        let count = dlq.len();
        dlq.clear();
        count
    }

    /// DLQ 容量限制：當 DLQ 滿時，丟棄最舊的
    async fn dlq_evict_if_full(&self) {
        let mut dlq = self.dead_letter_queue.lock().await;
        while dlq.len() > self.config.max_dlq_size {
            let evicted = dlq.remove(0);
            warn!("DLQ full, evicting oldest task: {}", evicted.id.0);
        }
    }
}
```

---

## 7. 節點通訊協定

### 7.1 通訊方式選擇

| 方案 | 延遲 | 實作複雜度 | 可靠性 |
|------|------|-----------|--------|
| HTTP/REST | ~5ms | 低 | 高（現有 axum 框架） |
| gRPC | ~2ms | 中（需 tonic） | 高 |
| WebSocket | ~1ms | 中（已有 axum ws） | 中（長連線管理） |
| NATS/Redis PubSub | ~1ms | 中（外部依賴） | 高 |

**推薦：HTTP/REST + WebSocket 混合**
- 任務分發和結果回報用 HTTP（冪等、可重試）
- 心跳和即時通知用 WebSocket（低延遲、低開銷）

### 7.2 節點 API 端點

每個 clawtex-core 節點（包括 Z13）都暴露以下 HTTP API：

```
# ── 健康檢查 ──────────────────────────
GET  /cluster/health           → { status, load, loaded_models }
GET  /cluster/capabilities     → { cpu_cores, ram_gb, vram_gb, ... }

# ── 任務執行 ──────────────────────────
POST /cluster/task/execute     → 接收並執行任務（同步等待結果）
POST /cluster/task/cancel      → 取消正在執行的任務
GET  /cluster/task/:id/status  → 查詢任務狀態

# ── 模型管理 ──────────────────────────
GET  /cluster/models           → 列出已載入的模型
POST /cluster/models/preload   → 預載模型（非同步）
POST /cluster/models/evict     → 驅逐模型

# ── 心跳 WebSocket ────────────────────
WS   /cluster/ws               → 雙向心跳 + 即時負載報告
```

### 7.3 心跳協定

```rust
/// 節點每 5 秒發送心跳到 Scheduler（Z13）
#[derive(Debug, Serialize, Deserialize)]
struct HeartbeatMessage {
    node_id: NodeId,
    timestamp: chrono::DateTime<chrono::Utc>,
    load: NodeLoad,
    loaded_models: Vec<String>,
    status: NodeStatus,
}

/// Scheduler 端心跳管理
impl TaskScheduler {
    async fn heartbeat_receiver_loop(&self) {
        // 每 heartbeat_timeout_secs 檢查一次
        let mut interval = tokio::time::interval(
            std::time::Duration::from_secs(self.config.heartbeat_timeout_secs / 2)
        );

        loop {
            interval.tick().await;
            let now = chrono::Utc::now();
            let mut nodes = self.nodes.write().await;

            for (_, node) in nodes.iter_mut() {
                let elapsed = (now - node.last_heartbeat).num_seconds() as u64;
                if elapsed > self.config.heartbeat_timeout_secs {
                    if node.status == NodeStatus::Online {
                        warn!("Node {} missed heartbeat ({}s), marking Offline",
                            node.node_id, elapsed);
                        node.status = NodeStatus::Offline;

                        // 重新排程該節點上的所有任務
                        self.reschedule_node_tasks(&node.node_id).await;
                    }
                }
            }
        }
    }

    /// 節點故障時，將其任務移回佇列
    async fn reschedule_node_tasks(&self, node_id: &NodeId) {
        let task_ids: Vec<TaskId> = {
            let nt = self.node_tasks.read().await;
            nt.get(node_id).cloned().unwrap_or_default()
        };

        for task_id in task_ids {
            self.handle_task_failure(task_id, node_id.clone(), "node_offline").await;
        }
    }
}
```

### 7.4 任務結果傳遞

```rust
/// 任務執行結果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: TaskId,
    pub status: ScheduledTaskStatus,
    /// agent_runtime 的輸出
    pub output: Option<String>,
    /// 錯誤訊息
    pub error: Option<String>,
    /// token 使用量
    pub tokens_used: u32,
    /// 使用的 provider
    pub provider_used: String,
    /// 使用的 model
    pub model_used: String,
    /// 執行時間 (秒)
    pub duration_secs: f64,
    /// 執行該任務的節點
    pub executed_on: NodeId,
}

impl TaskScheduler {
    /// 接收任務完成回報
    async fn handle_task_result(&self, result: TaskResult) {
        let task_id = result.task_id.clone();

        // 更新指派表
        self.assignments.write().await.remove(&task_id);

        // 更新節點任務列表
        if let Some(tasks) = self.node_tasks.write().await.get_mut(&result.executed_on) {
            tasks.retain(|id| id != &task_id);
        }

        // 更新節點統計
        if let Some(node) = self.nodes.write().await.get_mut(&result.executed_on) {
            if result.status == ScheduledTaskStatus::Completed {
                node.tasks_completed += 1;
            } else {
                node.tasks_failed += 1;
            }
            node.total_tokens_processed += result.tokens_used as u64;
            node.current_load.running_tasks = node.current_load.running_tasks.saturating_sub(1);
        }

        // 記錄成本
        if let Some(ref ct) = self.cost_tracker {
            // ... record cost
        }

        // 通知等待者
        if let Some(sender) = self.result_channels.lock().await.remove(&task_id) {
            let _ = sender.send(result.clone());
        }

        // 觸發 metrics
        self.metrics.increment("scheduler_tasks_completed");
        self.metrics.observe("scheduler_task_duration_ms", result.duration_secs * 1000.0);

        // 通知排程迴圈（有新的容量了）
        self.notify_task_done.notify_one();
    }
}
```

---

## 8. 與現有模組整合

### 8.1 AgentRuntime 整合

**目前**：`AgentRuntime::run_with_config()` 直接呼叫 `router.chat_with_tools()`

**集群後**：在 `chat_with_tools` 之前插入 `TaskScheduler`

```rust
// 方案 A：透明代理（推薦）
// 建立一個 ClusterProvider 實作 Provider trait，
// 內部將 chat() 呼叫封裝為 Task 提交到 TaskScheduler

pub struct ClusterProvider {
    scheduler: Arc<TaskScheduler>,
    local_node_id: NodeId,
}

#[async_trait]
impl Provider for ClusterProvider {
    fn name(&self) -> &str { "cluster" }
    fn default_model(&self) -> &str { "auto" }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<ChatResponse> {
        let task = ScheduledTask {
            id: TaskId::new(),
            task_type: TaskType::Chat {
                agent_name: "dynamic".into(),
                user_id: "system".into(),
            },
            priority: Priority::Normal,
            constraints: TaskConstraints {
                required_model: if model.is_empty() { None } else { Some(model.to_string()) },
                ..Default::default()
            },
            prompt: serde_json::to_string(messages)?,
            // ... other fields
            ..Default::default()
        };

        let (tx, rx) = oneshot::channel();
        self.scheduler.submit_with_channel(task, tx).await?;

        // 等待結果
        let result = rx.await??;
        // 解序列化為 ChatResponse
        // ...
        todo!()
    }

    async fn is_alive(&self) -> bool {
        // 只要 scheduler 有任何 online 節點就是 alive
        !self.scheduler.nodes.read().await.is_empty()
    }
}
```

```rust
// 方案 B：在 AgentRuntime 層面整合（侵入性較大）
// 修改 run_with_config() 來判斷是否應該分發到遠端節點

impl AgentRuntime {
    pub async fn run_with_config(
        &self,
        // ... existing params
        scheduler: Option<&TaskScheduler>, // 新增
    ) -> Result<AgentResult> {
        // 如果有 scheduler 且不在本地節點，封裝為 Task 提交
        if let Some(sched) = scheduler {
            if sched.should_remote_execute(agent_name, provider) {
                return sched.submit_agent_task(agent_name, config, prompt, ...).await;
            }
        }

        // 否則走原有本地路徑
        // ... existing code
    }
}
```

**推薦方案 A**：最小侵入性，透過 Provider 抽象層插入集群路由。

### 8.2 HandRunner 整合

```rust
impl HandRunner {
    /// 集群版：每個 phase 可在不同節點執行
    pub async fn run_clustered(
        hand: &Hand,
        user_input: &str,
        scheduler: &TaskScheduler,
        runtime: &AgentRuntime,
        router: &LlmRouter,
        tool_registry: &ToolRegistry,
    ) -> Result<HandResult> {
        let execution_id = uuid::Uuid::new_v4().to_string();
        let mut outputs = Vec::new();
        let mut context = Self::prepare_context(hand, user_input);
        let mut last_node: Option<NodeId> = None;

        for (i, phase) in hand.phases.iter().enumerate() {
            let task = ScheduledTask {
                id: TaskId::new(),
                task_type: TaskType::HandPhase {
                    hand_name: hand.name.clone(),
                    phase_index: i,
                    total_phases: hand.phases.len(),
                    execution_id: execution_id.clone(),
                },
                priority: Priority::Normal,
                constraints: TaskConstraints {
                    // 親和性：偏好上一個 phase 的節點
                    affinity_node: last_node.clone(),
                    preferred_provider: if hand.provider != "auto" {
                        Some(hand.provider.clone())
                    } else {
                        None
                    },
                    ..Default::default()
                },
                prompt: context.clone(),
                // ...
                ..Default::default()
            };

            let (tx, rx) = oneshot::channel();
            scheduler.submit_with_channel(task, tx).await?;

            let result = rx.await??;
            last_node = Some(result.executed_on.clone());

            outputs.push(PhaseOutput {
                phase_name: phase.name.clone(),
                output: result.output.clone().unwrap_or_default(),
                tool_calls: 0, // 需要從結果中提取
                skipped: false,
            });

            context = result.output.unwrap_or_default();
        }

        Ok(HandResult {
            hand_name: hand.name.clone(),
            phases_completed: outputs.len(),
            total_phases: hand.phases.len(),
            outputs,
            final_output: context,
            elapsed_secs: 0.0, // 需要計算
            chain_to: hand.chain_to.clone(),
        })
    }
}
```

### 8.3 SoT 整合

```rust
impl SkeletonRunner {
    /// 集群版：section 分發到不同節點
    pub async fn expand_parallel_clustered(
        &self,
        topic: &str,
        sections: &[SkeletonSection],
        scheduler: &TaskScheduler,
        parent_task_id: &TaskId,
    ) -> Vec<SectionResult> {
        let mut receivers = Vec::new();

        for section in sections {
            let task = ScheduledTask {
                id: TaskId::new(),
                task_type: TaskType::SoTExpansion {
                    topic: topic.to_string(),
                    section_index: section.index,
                    total_sections: sections.len(),
                    parent_task_id: parent_task_id.clone(),
                },
                priority: Priority::Background,
                constraints: TaskConstraints::default(),
                prompt: format!(
                    "Expand section {}: {} --- {}",
                    section.index, section.title, section.description
                ),
                timeout_secs: self.config.section_timeout_secs,
                ..Default::default()
            };

            let (tx, rx) = oneshot::channel();
            let _ = scheduler.submit_with_channel(task, tx).await;
            receivers.push((section.clone(), rx));
        }

        // 並行等待所有結果
        let mut results = Vec::new();
        for (section, rx) in receivers {
            match rx.await {
                Ok(Ok(result)) => {
                    results.push(SectionResult {
                        index: section.index,
                        title: section.title,
                        content: result.output.unwrap_or_default(),
                        provider: result.provider_used,
                        success: result.status == ScheduledTaskStatus::Completed,
                    });
                }
                _ => {
                    results.push(SectionResult {
                        index: section.index,
                        title: section.title,
                        content: "Error: task failed or timed out".into(),
                        provider: "unknown".into(),
                        success: false,
                    });
                }
            }
        }

        results
    }
}
```

### 8.4 Cron 整合

```rust
// 修改 JobExecutor 閉包，將 Agent/Hand 任務透過 Scheduler 分發
let scheduler = Arc::clone(&scheduler);
let executor: JobExecutor = Arc::new(move |action| {
    let sched = scheduler.clone();
    tokio::spawn(async move {
        match action {
            JobAction::Agent { agent, prompt } => {
                let task = ScheduledTask {
                    task_type: TaskType::BackgroundBatch {
                        job_name: format!("cron_agent_{}", agent),
                    },
                    priority: Priority::Low,
                    prompt,
                    ..Default::default()
                };
                match sched.submit(task).await {
                    Ok(id) => format!("Submitted to cluster: {}", id.0),
                    Err(e) => format!("Submit failed: {:?}", e),
                }
            }
            JobAction::Hand { hand_name, input } => {
                // 類似處理
                "hand submitted".to_string()
            }
            JobAction::Shell { command } => {
                // Shell 任務仍在本地執行
                // ...
                "shell done".to_string()
            }
            _ => "unknown action".to_string(),
        }
    })
});
```

### 8.5 Telegram 命令

新增以下 Telegram 命令：

```
/cluster            — 顯示集群狀態（所有節點 + 負載）
/cluster nodes      — 列出所有節點
/cluster tasks      — 列出排程中的任務
/cluster dlq        — 檢視死信佇列
/cluster drain <n>  — 將節點 N 設為 Draining
/cluster resume <n> — 恢復節點 N
/cluster preload <node> <model> — 預載模型到節點
```

### 8.6 HTTP API

```
GET  /cluster/status          → 集群總覽（節點、佇列深度、DLQ）
GET  /cluster/nodes           → 節點列表 + 負載
GET  /cluster/queue           → 當前佇列快照
GET  /cluster/dlq             → 死信佇列
POST /cluster/task            → 提交任務
DELETE /cluster/task/:id      → 取消任務
POST /cluster/dlq/:id/retry  → 重試死信任務
DELETE /cluster/dlq           → 清空死信佇列
POST /cluster/node/:id/drain → Drain 節點
POST /cluster/node/:id/resume → 恢復節點
```

---

## 9. Rust 實作建議

### 9.1 Crate 選擇

| 用途 | Crate | 理由 |
|------|-------|------|
| 非同步運行時 | `tokio` (已用) | 全專案統一 |
| HTTP Client/Server | `reqwest` + `axum` (已用) | 節點間通訊複用現有框架 |
| WebSocket | `axum::extract::ws` (已用) | 心跳通道 |
| 序列化 | `serde` + `serde_json` (已用) | 任務序列化 |
| UUID | `uuid` (已用) | TaskId 生成 |
| 時間 | `chrono` (已用) | 時間戳 |
| SQLite | `rusqlite` (已用) | 任務持久化 |
| 優先佇列 | `std::collections::BinaryHeap` | 標準庫，無外部依賴 |
| 並行原語 | `tokio::sync::{Mutex, RwLock, Notify, mpsc, oneshot, watch}` | 全 async 友好 |
| Metrics | 自建 `MetricsRegistry` (已有) | 零外部依賴 |

**不需要新增外部 crate**，全部使用現有依賴。

### 9.2 資料結構選擇

| 結構 | 用途 | 理由 |
|------|------|------|
| `BinaryHeap<PrioritizedTask>` | 主佇列 | O(log n) push/pop，天然支援優先級 |
| `HashMap<NodeId, NodeState>` + `RwLock` | 節點狀態 | O(1) 查詢，讀多寫少用 RwLock |
| `HashMap<TaskId, NodeId>` + `RwLock` | 任務指派 | O(1) 雙向查詢 |
| `HashMap<NodeId, Vec<TaskId>>` + `RwLock` | 反向映射 | 節點故障時快速找到受影響任務 |
| `Vec<ScheduledTask>` + `Mutex` | 死信佇列 | 低頻操作，簡單就好 |
| `HashMap<TaskId, oneshot::Sender>` + `Mutex` | 結果通道 | 一次性使用，生命周期清晰 |
| `watch::Sender<bool>` | 關閉信號 | 廣播給所有 receiver |

### 9.3 併發控制策略

```
                   ┌─────────────────────────────┐
                   │     TaskScheduler (主結構)     │
                   └─────────────────────────────┘
                              │
        ┌─────────────────────┼─────────────────────┐
        │                     │                     │
   schedule_loop         aging_loop         timeout_monitor
   (100ms interval)    (30s interval)      (10s interval)
        │                     │                     │
        ▼                     ▼                     ▼
   queue: Mutex         queue: Mutex         assignments: RwLock
   nodes: RwLock        (drain+rebuild)      node_tasks: RwLock
   assignments: RwLock
   node_tasks: RwLock

   ┌────────────────┐   ┌────────────────┐
   │  submit() 入口  │   │  result 回報    │
   │  (多 caller)    │   │  (多節點並行)   │
   └────────────────┘   └────────────────┘
          │                      │
          ▼                      ▼
     queue: Mutex          assignments: RwLock
     notify: Notify        result_channels: Mutex
```

**關鍵原則**：
1. `Mutex` 用於需要獨占寫入的短臨界區（queue, dlq, result_channels）
2. `RwLock` 用於讀多寫少的長期狀態（nodes, assignments, node_tasks）
3. `Notify` 用於生產者-消費者通知（比 channel 更輕量）
4. `watch` 用於廣播信號（shutdown）
5. 所有鎖的持有時間盡量短，避免在持鎖期間做 async 操作

### 9.4 持久化策略

```rust
/// 任務持久化到 SQLite（擴展現有 task_queue.rs）
impl TaskScheduler {
    /// 持久化：重要任務入隊時寫入 SQLite
    /// 只持久化 Normal 及以上的任務（Background 丟了就丟了）
    async fn persist_task(&self, task: &ScheduledTask) {
        if task.priority == Priority::Background {
            return; // 背景任務不持久化
        }
        // INSERT INTO scheduled_tasks (id, type_json, priority, constraints_json,
        //     prompt, status, submitted_at, ...)
        // ON CONFLICT(id) DO UPDATE SET status = ...
    }

    /// 啟動時從 SQLite 恢復未完成的任務
    async fn recover_from_persistence(&self) {
        // SELECT * FROM scheduled_tasks WHERE status IN ('queued', 'assigned', 'running')
        // 全部設為 Queued 重新排程
    }
}
```

### 9.5 模組檔案規劃

```
src/
  scheduler/
    mod.rs           — pub use, TaskScheduler 主結構
    types.rs         — TaskId, NodeId, Priority, TaskType, TaskConstraints, etc.
    queue.rs         — PrioritizedTask, BinaryHeap 包裝, aging
    scoring.rs       — score_node_for_task(), ScoringWeights
    routing.rs       — candidate_nodes(), 各 TaskType 路由邏輯
    node_manager.rs  — NodeState, heartbeat, health check
    preemption.rs    — try_preempt(), preempt_and_reassign()
    backpressure.rs  — submit(), 分級背壓
    retry.rs         — handle_task_failure(), DLQ
    persistence.rs   — SQLite 持久化和恢復
    cluster_api.rs   — axum handlers (HTTP + WS)
    cluster_provider.rs — ClusterProvider impl Provider
```

---

## 10. 測試策略

### 10.1 單元測試

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // ── 優先佇列排序 ──────────────────────────
    #[test]
    fn test_priority_ordering_critical_first() {
        // Critical 任務永遠先出
    }

    #[test]
    fn test_fifo_within_same_priority() {
        // 同優先級先入先出
    }

    #[test]
    fn test_aging_promotes_low_priority() {
        // 等待足夠久的 Low 任務會升級
    }

    // ── 節點評分 ──────────────────────────────
    #[test]
    fn test_score_model_loaded_node_wins() {
        // 已載入模型的節點得分最高
    }

    #[test]
    fn test_score_hard_constraint_rejects() {
        // 不滿足硬約束的節點返回 -1.0
    }

    #[test]
    fn test_score_z13_penalty_when_loaded() {
        // Z13 負載 > 50% 時有懲罰分
    }

    // ── 背壓 ──────────────────────────────────
    #[test]
    fn test_backpressure_rejects_background_at_60_percent() {
        // 佇列 60% 滿時拒絕 Background
    }

    #[test]
    fn test_critical_never_rejected() {
        // Critical 任務永不被背壓拒絕
    }

    // ── 路由 ──────────────────────────────────
    #[test]
    fn test_sot_round_robin_distribution() {
        // N 個 section 均勻分配到 M 個節點
    }

    #[test]
    fn test_hand_phase_affinity() {
        // 同一 execution_id 的 phase 偏好同節點
    }

    #[test]
    fn test_background_avoids_z13() {
        // 背景任務預設不去 Z13
    }

    // ── 重試和 DLQ ────────────────────────────
    #[test]
    fn test_retry_adds_anti_affinity() {
        // 重試時加入失敗節點的反親和性
    }

    #[test]
    fn test_max_retries_sends_to_dlq() {
        // 超過最大重試次數進入死信佇列
    }

    #[test]
    fn test_dlq_eviction_when_full() {
        // DLQ 滿時驅逐最舊的
    }
}
```

### 10.2 整合測試

```rust
#[tokio::test]
async fn test_submit_and_receive_result() {
    // 1. 建立 scheduler (2 mock nodes)
    // 2. 提交一個 Chat 任務
    // 3. 模擬節點完成
    // 4. 驗證結果通過 oneshot channel 返回
}

#[tokio::test]
async fn test_node_failure_reschedules_tasks() {
    // 1. 建立 scheduler (3 nodes)
    // 2. 提交 5 個任務，分配到不同節點
    // 3. 模擬 node-2 離線
    // 4. 驗證 node-2 的任務被重新排程
}

#[tokio::test]
async fn test_preemption_flow() {
    // 1. 填滿所有節點（Background 任務）
    // 2. 提交 Critical 任務
    // 3. 驗證 Background 被搶佔
    // 4. 驗證 Background 任務移回佇列
}

#[tokio::test]
async fn test_hand_phases_execute_sequentially() {
    // 1. 提交一個 4-phase Hand
    // 2. 驗證 phase 按序執行
    // 3. 驗證後續 phase 在同節點（親和性）
}

#[tokio::test]
async fn test_sot_parallel_across_nodes() {
    // 1. 提交 8 section 的 SoT 到 4 節點
    // 2. 驗證每個節點收到 2 個 section
    // 3. 驗證結果按 index 正確合併
}
```

---

## 附錄 A：8 節點集群的硬體假設

| 節點 | 角色 | CPU | RAM | GPU/NPU | 特殊能力 |
|------|------|-----|-----|---------|---------|
| Z13 (local) | 主控 + 執行 | Ryzen AI MAX+ 395 16C | 64GB | RDNA 3.5 96GB + XDNA 50T | Playwright, 全工具 |
| node-1 | 執行 | 待定 | 待定 | 待定 | Ollama |
| node-2 | 執行 | 待定 | 待定 | 待定 | Ollama |
| node-3 | 執行 | 待定 | 待定 | 待定 | Ollama |
| node-4 | 執行 | 待定 | 待定 | 待定 | LM Studio |
| node-5 | 執行 | 待定 | 待定 | 待定 | LM Studio |
| node-6 | 雲端 | - | - | - | Anthropic API |
| node-7 | 雲端 | - | - | - | Gemini/Groq API |

> Z13 的 `weight` 設為 100，本地節點永遠是 `max_concurrent_tasks` 最高的。
> 雲端節點沒有 VRAM/NPU，但 `max_latency_ms` 可能較高。

## 附錄 B：指標清單 (Prometheus)

```
# 佇列
clawtex_scheduler_queue_size{priority}         gauge
clawtex_scheduler_dlq_size                     gauge
clawtex_scheduler_tasks_submitted_total        counter
clawtex_scheduler_tasks_completed_total        counter
clawtex_scheduler_tasks_failed_total           counter
clawtex_scheduler_tasks_timed_out_total        counter
clawtex_scheduler_tasks_dead_lettered_total    counter
clawtex_scheduler_tasks_preempted_total        counter

# 延遲
clawtex_scheduler_task_duration_ms             histogram
clawtex_scheduler_queue_wait_ms                histogram
clawtex_scheduler_assignment_score             histogram

# 節點
clawtex_scheduler_node_running_tasks{node_id}  gauge
clawtex_scheduler_node_status{node_id}         gauge (1=online, 0=offline)
clawtex_scheduler_node_latency_ms{node_id}     gauge

# 背壓
clawtex_scheduler_backpressure_rejects_total{level}  counter
clawtex_scheduler_no_suitable_node_total             counter
```

## 附錄 C：配置範例 (agents.toml)

```toml
[scheduler]
strategy = "hybrid"
max_queue_size = 10000
max_dlq_size = 1000
default_timeout_secs = 600
default_max_retries = 2
heartbeat_timeout_secs = 30
schedule_interval_ms = 100
enable_preemption = true
preemption_priority_gap = 2
backpressure_threshold = 0.8
local_node_id = "z13"

[[cluster_nodes]]
name = "z13"
host = "127.0.0.1"
port = 7878
weight = 100
max_concurrent_tasks = 8
has_browser = true
has_npu = true
vram_gb = 96
available_providers = ["ollama", "lmstudio", "lemonade"]

[[cluster_nodes]]
name = "node-1"
host = "10.0.2.1"
port = 7878
weight = 60
max_concurrent_tasks = 4
has_browser = false
has_npu = false
vram_gb = 24
available_providers = ["ollama"]

[[cluster_nodes]]
name = "cloud-anthropic"
host = "api.anthropic.com"
port = 443
weight = 40
max_concurrent_tasks = 10
has_browser = false
has_npu = false
vram_gb = 0
available_providers = ["anthropic"]
```

---

## 總結

本設計的核心理念是**最小侵入性**：
1. 透過 `ClusterProvider` 實作 `Provider` trait，插入到現有 `ProviderRouter` 中
2. 不修改 `AgentRuntime::run_with_config()` 的核心邏輯
3. `HandRunner` 和 `SkeletonRunner` 各提供一個 `_clustered()` 變體
4. 所有新程式碼集中在 `src/scheduler/` 模組中
5. 完全使用現有 crate（tokio, axum, rusqlite, serde），零新依賴
6. 向下相容：scheduler 為 `Option`，不啟用時走原有本地路徑
