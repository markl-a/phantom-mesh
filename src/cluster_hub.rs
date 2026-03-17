//! ClusterHub — task distribution and worker orchestration.
//! Routes tool calls to the best available worker based on capability and load.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::cluster::{ClusterNode, ClusterRegistry};

/// TTL for idempotency keys (5 minutes)
const IDEMPOTENCY_TTL_SECS: u64 = 300;

/// Default task priority (mid-range)
fn default_priority() -> u8 { 100 }

/// Tools that are network-bound — can run on light workers
const NETWORK_TOOLS: &[&str] = &[
    "web_search", "http_request", "email_send",
];

/// Mobile-specific tools (sensors, local LLM, JS execution)
const MOBILE_TOOLS: &[&str] = &[
    "sensor_gps", "sensor_camera", "sensor_accel", "sensor_audio",
    "local_llm", "js_exec",
];

/// Tools that must stay on the hub (local filesystem, memory)
const LOCAL_ONLY_TOOLS: &[&str] = &[
    "file_write", "file_edit", "file_read",
    "memory_store", "memory_recall", "memory_forget",
    "glob_search", "content_search",
];

/// Per-worker performance stats
#[derive(Debug, Clone, Serialize)]
pub struct WorkerStats {
    pub tasks_completed: u64,
    pub tasks_failed: u64,
    pub avg_latency_ms: u64,
    pub last_error: Option<String>,
}

impl Default for WorkerStats {
    fn default() -> Self {
        Self {
            tasks_completed: 0,
            tasks_failed: 0,
            avg_latency_ms: 0,
            last_error: None,
        }
    }
}

/// Cluster-wide metrics
pub struct ClusterMetrics {
    pub dispatch_count: AtomicU64,
    pub dispatch_failures: AtomicU64,
    pub avg_response_ms: AtomicU64,
    pub per_worker_stats: Mutex<HashMap<String, WorkerStats>>,
}

impl ClusterMetrics {
    pub fn new() -> Self {
        Self {
            dispatch_count: AtomicU64::new(0),
            dispatch_failures: AtomicU64::new(0),
            avg_response_ms: AtomicU64::new(0),
            per_worker_stats: Mutex::new(HashMap::new()),
        }
    }

    /// Record a successful dispatch
    pub async fn record_success(&self, worker_name: &str, latency_ms: u64) {
        self.dispatch_count.fetch_add(1, Ordering::Relaxed);

        // Update rolling average
        let count = self.dispatch_count.load(Ordering::Relaxed);
        let prev_avg = self.avg_response_ms.load(Ordering::Relaxed);
        let new_avg = if count <= 1 {
            latency_ms
        } else {
            (prev_avg * (count - 1) + latency_ms) / count
        };
        self.avg_response_ms.store(new_avg, Ordering::Relaxed);

        // Update per-worker stats
        let mut stats = self.per_worker_stats.lock().await;
        let ws = stats.entry(worker_name.to_string()).or_default();
        ws.tasks_completed += 1;
        let total = ws.tasks_completed + ws.tasks_failed;
        ws.avg_latency_ms = (ws.avg_latency_ms * (total - 1) + latency_ms) / total;
    }

    /// Record a failed dispatch
    pub async fn record_failure(&self, worker_name: &str, error: &str) {
        self.dispatch_failures.fetch_add(1, Ordering::Relaxed);
        self.dispatch_count.fetch_add(1, Ordering::Relaxed);

        let mut stats = self.per_worker_stats.lock().await;
        let ws = stats.entry(worker_name.to_string()).or_default();
        ws.tasks_failed += 1;
        ws.last_error = Some(error.to_string());
    }

    /// Get a serializable snapshot of metrics
    pub async fn snapshot(&self) -> Value {
        let stats = self.per_worker_stats.lock().await;
        json!({
            "dispatch_count": self.dispatch_count.load(Ordering::Relaxed),
            "dispatch_failures": self.dispatch_failures.load(Ordering::Relaxed),
            "avg_response_ms": self.avg_response_ms.load(Ordering::Relaxed),
            "per_worker": stats.clone(),
        })
    }

    /// Get stats for a specific worker
    pub async fn worker_stats(&self, worker_name: &str) -> Option<WorkerStats> {
        let stats = self.per_worker_stats.lock().await;
        stats.get(worker_name).cloned()
    }
}

/// Response from a worker tool execution
#[derive(Debug, Deserialize)]
struct WorkerToolResponse {
    success: bool,
    output: String,
}

/// A pending task waiting for a polling (mobile) worker to pick up
pub struct PendingTask {
    pub id: String,
    pub tool: String,
    pub input: Value,
    pub priority: u8,  // 0=highest, 255=lowest, default 100
    pub idempotency_key: Option<String>,
    pub result_tx: tokio::sync::oneshot::Sender<Value>,
    pub created_at: Instant,
}

/// Serialized view of a pending task (sent to polling workers)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollTaskResponse {
    pub task_id: String,
    pub tool: String,
    pub input: Value,
    #[serde(default = "default_priority")]
    pub priority: u8,
}

/// Result submitted by a polling worker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResultPayload {
    pub task_id: String,
    pub success: bool,
    pub output: String,
    pub worker: Option<String>,
}

/// An agent-level task — the worker becomes an autonomous agent to achieve a goal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    pub task_id: String,
    pub goal: String,
    pub max_iterations: u32,
    pub available_tools: Vec<String>,
}

/// A pending agent task waiting for a polling worker
pub struct PendingAgentTask {
    pub task: AgentTask,
    pub priority: u8,  // 0=highest, 255=lowest, default 100
    pub result_tx: tokio::sync::oneshot::Sender<Value>,
    pub created_at: Instant,
}

/// ClusterHub — central coordinator that dispatches tasks to workers
pub struct ClusterHub {
    pub registry: Arc<ClusterRegistry>,
    pub metrics: Arc<ClusterMetrics>,
    http_client: reqwest::Client,
    /// Per-worker queue of pending tasks (for polling/mobile workers)
    pending_tasks: Mutex<HashMap<String, VecDeque<PendingTask>>>,
    /// Map of task_id → oneshot sender, for results that arrive via POST /cluster/result
    inflight_results: Mutex<HashMap<String, tokio::sync::oneshot::Sender<Value>>>,
    /// Number of in-flight tasks per worker (for load-aware routing)
    inflight_counts: Mutex<HashMap<String, u32>>,
    /// Shared task pool for mobile workers (any mobile worker can pick up)
    shared_mobile_pool: Mutex<VecDeque<PendingTask>>,
    /// Per-worker queue of agent tasks (higher-level autonomous tasks)
    pending_agent_tasks: Mutex<HashMap<String, VecDeque<PendingAgentTask>>>,
    /// Shared agent task pool for mobile workers
    shared_agent_pool: Mutex<VecDeque<PendingAgentTask>>,
    /// Idempotency log: key → timestamp (for dedup within TTL)
    dispatch_log: Mutex<HashMap<String, Instant>>,
}

impl ClusterHub {
    pub fn new(registry: Arc<ClusterRegistry>) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_default();

        Self {
            registry,
            metrics: Arc::new(ClusterMetrics::new()),
            http_client,
            pending_tasks: Mutex::new(HashMap::new()),
            inflight_results: Mutex::new(HashMap::new()),
            inflight_counts: Mutex::new(HashMap::new()),
            shared_mobile_pool: Mutex::new(VecDeque::new()),
            pending_agent_tasks: Mutex::new(HashMap::new()),
            shared_agent_pool: Mutex::new(VecDeque::new()),
            dispatch_log: Mutex::new(HashMap::new()),
        }
    }

    /// Check idempotency key — returns error if duplicate within TTL
    async fn check_idempotency(&self, key: &str) -> Result<()> {
        let mut log = self.dispatch_log.lock().await;
        if let Some(ts) = log.get(key) {
            let age = ts.elapsed().as_secs();
            if age < IDEMPOTENCY_TTL_SECS {
                return Err(anyhow!("Duplicate dispatch: idempotency key '{}' seen {}s ago (TTL {}s)", key, age, IDEMPOTENCY_TTL_SECS));
            }
        }
        log.insert(key.to_string(), Instant::now());
        Ok(())
    }

    /// Find the index of the highest-priority task (lowest u8) in a VecDeque
    fn best_priority_task_idx(queue: &VecDeque<PendingTask>) -> Option<usize> {
        if queue.is_empty() { return None; }
        queue.iter().enumerate()
            .min_by_key(|(_, t)| t.priority)
            .map(|(i, _)| i)
    }

    /// Find the index of the highest-priority agent task in a VecDeque
    fn best_priority_agent_idx(queue: &VecDeque<PendingAgentTask>) -> Option<usize> {
        if queue.is_empty() { return None; }
        queue.iter().enumerate()
            .min_by_key(|(_, t)| t.priority)
            .map(|(i, _)| i)
    }

    /// Determine how a tool should be routed
    pub fn tool_routing(&self, tool_name: &str) -> ToolRouting {
        if LOCAL_ONLY_TOOLS.contains(&tool_name) {
            ToolRouting::Local
        } else if MOBILE_TOOLS.contains(&tool_name) {
            ToolRouting::MobileOnly
        } else if NETWORK_TOOLS.contains(&tool_name) {
            ToolRouting::AnyWorker
        } else {
            ToolRouting::FullWorkerOnly
        }
    }

    /// Check if a tool should be dispatched to a remote worker
    pub fn should_dispatch(&self, tool_name: &str) -> bool {
        !matches!(self.tool_routing(tool_name), ToolRouting::Local)
    }

    /// Get the effective load for a worker (cpu_load + inflight penalty).
    async fn effective_load(&self, worker: &ClusterNode) -> f32 {
        let counts = self.inflight_counts.lock().await;
        let inflight = counts.get(&worker.name).copied().unwrap_or(0);
        worker.cpu_load + (inflight as f32 * 0.15)
    }

    /// Increment inflight count for a worker.
    async fn inc_inflight(&self, worker_name: &str) {
        let mut counts = self.inflight_counts.lock().await;
        *counts.entry(worker_name.to_string()).or_insert(0) += 1;
    }

    /// Decrement inflight count for a worker.
    async fn dec_inflight(&self, worker_name: &str) {
        let mut counts = self.inflight_counts.lock().await;
        if let Some(count) = counts.get_mut(worker_name) {
            *count = count.saturating_sub(1);
        }
    }

    /// Dispatch a tool call to the best available worker.
    /// For mobile workers (polling mode), enqueues the task and waits for the result.
    /// For push workers, sends an HTTP POST directly.
    pub async fn dispatch_tool(&self, tool_name: &str, input: Value) -> Result<Value> {
        let routing = self.tool_routing(tool_name);
        let worker = match routing {
            ToolRouting::Local => return Err(anyhow!("Tool '{}' is local-only", tool_name)),
            ToolRouting::MobileOnly => {
                // Only mobile workers can handle sensor/local_llm/js_exec tools
                let workers = self.registry.online_workers().await;
                let mobile_workers: Vec<_> = workers.into_iter()
                    .filter(|w| w.device_type == "mobile")
                    .collect();
                if mobile_workers.is_empty() {
                    let msg = format!("No mobile workers available for tool '{}'", tool_name);
                    self.metrics.record_failure("none", &msg).await;
                    return Err(anyhow!(msg));
                }
                // Pick by effective load (cpu + inflight penalty)
                let mut best = mobile_workers[0].clone();
                let mut best_load = self.effective_load(&best).await;
                for w in &mobile_workers[1..] {
                    let eff = self.effective_load(w).await;
                    if eff < best_load {
                        best = w.clone();
                        best_load = eff;
                    }
                }
                best
            }
            ToolRouting::AnyWorker => {
                // Any online worker (including light and mobile workers)
                let workers = self.registry.online_workers().await;
                if workers.is_empty() {
                    let msg = format!("No online workers available for tool '{}'", tool_name);
                    self.metrics.record_failure("none", &msg).await;
                    return Err(anyhow!(msg));
                }
                // Pick by effective load (cpu + inflight penalty)
                let mut best = workers[0].clone();
                let mut best_load = self.effective_load(&best).await;
                for w in &workers[1..] {
                    let eff = self.effective_load(w).await;
                    if eff < best_load {
                        best = w.clone();
                        best_load = eff;
                    }
                }
                best
            }
            ToolRouting::FullWorkerOnly => {
                // Only full workers — use effective load for selection
                let workers = self.registry.online_workers().await;
                let full_workers: Vec<_> = workers.into_iter()
                    .filter(|n| n.capabilities.iter().any(|c| c == "tools"))
                    .collect();
                if full_workers.is_empty() {
                    let msg = format!("No full workers available for tool '{}'", tool_name);
                    self.metrics.record_failure("none", &msg).await;
                    return Err(anyhow!(msg));
                }
                let mut best = full_workers[0].clone();
                let mut best_load = self.effective_load(&best).await;
                for w in &full_workers[1..] {
                    let eff = self.effective_load(w).await;
                    if eff < best_load {
                        best = w.clone();
                        best_load = eff;
                    }
                }
                best
            }
        };

        // Mobile workers use polling mode — enqueue task and wait for result
        self.inc_inflight(&worker.name).await;
        let result = if worker.device_type == "mobile" {
            self.dispatch_to_mobile(&worker, tool_name, input, default_priority(), None).await
        } else {
            self.execute_on_worker(&worker, tool_name, input).await
        };
        self.dec_inflight(&worker.name).await;
        result
    }

    /// Dispatch a tool with explicit priority (0=highest, 255=lowest).
    pub async fn dispatch_tool_with_priority(&self, tool_name: &str, input: Value, priority: u8) -> Result<Value> {
        self.dispatch_tool_inner(tool_name, input, priority, None).await
    }

    /// Dispatch a tool with idempotency key (prevents duplicate dispatch within 5 min).
    pub async fn dispatch_tool_idempotent(&self, tool_name: &str, input: Value, idempotency_key: String) -> Result<Value> {
        self.dispatch_tool_inner(tool_name, input, default_priority(), Some(idempotency_key)).await
    }

    /// Internal dispatch with priority + idempotency support.
    async fn dispatch_tool_inner(&self, tool_name: &str, input: Value, priority: u8, idempotency_key: Option<String>) -> Result<Value> {
        // Check idempotency before anything else
        if let Some(ref key) = idempotency_key {
            self.check_idempotency(key).await?;
        }

        let routing = self.tool_routing(tool_name);
        let worker = match routing {
            ToolRouting::Local => return Err(anyhow!("Tool '{}' is local-only", tool_name)),
            ToolRouting::MobileOnly | ToolRouting::AnyWorker | ToolRouting::FullWorkerOnly => {
                let workers = self.registry.online_workers().await;
                let filtered: Vec<_> = match routing {
                    ToolRouting::MobileOnly => workers.into_iter().filter(|w| w.device_type == "mobile").collect(),
                    ToolRouting::AnyWorker => workers,
                    ToolRouting::FullWorkerOnly => workers.into_iter().filter(|n| n.capabilities.iter().any(|c| c == "tools")).collect(),
                    _ => unreachable!(),
                };
                if filtered.is_empty() {
                    let msg = format!("No workers available for tool '{}' (routing: {:?})", tool_name, routing);
                    self.metrics.record_failure("none", &msg).await;
                    return Err(anyhow!(msg));
                }
                let mut best = filtered[0].clone();
                let mut best_load = self.effective_load(&best).await;
                for w in &filtered[1..] {
                    let eff = self.effective_load(w).await;
                    if eff < best_load {
                        best = w.clone();
                        best_load = eff;
                    }
                }
                best
            }
        };

        self.inc_inflight(&worker.name).await;
        let result = if worker.device_type == "mobile" {
            self.dispatch_to_mobile(&worker, tool_name, input, priority, None).await
        } else {
            self.execute_on_worker(&worker, tool_name, input).await
        };
        self.dec_inflight(&worker.name).await;
        result
    }

    /// Dispatch a tool to a specific worker by name.
    /// Bypasses capability/routing checks — caller decides targeting.
    pub async fn dispatch_to_worker(&self, worker_name: &str, tool_name: &str, input: Value) -> Result<Value> {
        let worker = self.registry.get_node(worker_name).await
            .ok_or_else(|| anyhow!("Worker '{}' not found", worker_name))?;
        if worker.status != "online" {
            return Err(anyhow!("Worker '{}' is offline", worker_name));
        }

        info!("Targeted dispatch: tool '{}' → worker '{}'", tool_name, worker_name);

        self.inc_inflight(worker_name).await;
        let result = if worker.device_type == "mobile" {
            self.dispatch_to_mobile(&worker, tool_name, input, default_priority(), None).await
        } else {
            self.execute_on_worker(&worker, tool_name, input).await
        };
        self.dec_inflight(worker_name).await;
        result
    }

    /// Dispatch a tool to the best worker that has a given capability.
    /// Finds least-loaded online worker with the capability, regardless of tool routing rules.
    pub async fn dispatch_by_capability(&self, capability: &str, tool_name: &str, input: Value) -> Result<Value> {
        let worker = self.registry.best_worker_for(capability).await
            .ok_or_else(|| anyhow!("No online worker with capability '{}'", capability))?;

        info!("Capability dispatch: tool '{}' → worker '{}' (cap: {})", tool_name, worker.name, capability);

        self.inc_inflight(&worker.name).await;
        let result = if worker.device_type == "mobile" {
            self.dispatch_to_mobile(&worker, tool_name, input, default_priority(), None).await
        } else {
            self.execute_on_worker(&worker, tool_name, input).await
        };
        self.dec_inflight(&worker.name).await;
        result
    }

    /// Enqueue a task for a mobile (polling) worker and wait for the result via oneshot channel.
    async fn dispatch_to_mobile(&self, worker: &ClusterNode, tool_name: &str, input: Value, priority: u8, idempotency_key: Option<String>) -> Result<Value> {
        // Check idempotency
        if let Some(ref key) = idempotency_key {
            self.check_idempotency(key).await?;
        }

        let task_id = format!("mt-{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("0000"));
        let (tx, rx) = tokio::sync::oneshot::channel();

        debug!("Enqueuing task '{}' (tool: {}, priority: {}) for mobile worker '{}'", task_id, tool_name, priority, worker.name);

        let pending = PendingTask {
            id: task_id.clone(),
            tool: tool_name.to_string(),
            input: input.clone(),
            priority,
            idempotency_key,
            result_tx: tx,
            created_at: Instant::now(),
        };

        // Enqueue for this specific worker
        {
            let mut queues = self.pending_tasks.lock().await;
            queues.entry(worker.name.clone()).or_default().push_back(pending);
        }

        // Wait for result with timeout (120s)
        let start = Instant::now();
        match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
            Ok(Ok(result)) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                self.metrics.record_success(&worker.name, latency_ms).await;
                info!("Mobile task '{}' (tool: {}) completed on '{}' in {}ms", task_id, tool_name, worker.name, latency_ms);
                Ok(result)
            }
            Ok(Err(_)) => {
                let msg = format!("Mobile task '{}' channel dropped (worker disconnected?)", task_id);
                self.metrics.record_failure(&worker.name, &msg).await;
                Err(anyhow!(msg))
            }
            Err(_) => {
                // Timeout — remove from inflight
                let msg = format!("Mobile task '{}' timed out after 120s", task_id);
                self.metrics.record_failure(&worker.name, &msg).await;
                // Clean up the pending task if it wasn't picked up
                let mut queues = self.pending_tasks.lock().await;
                if let Some(queue) = queues.get_mut(&worker.name) {
                    queue.retain(|t| t.id != task_id);
                }
                Err(anyhow!(msg))
            }
        }
    }

    /// Dispatch a tool to the shared mobile pool — any mobile worker can pick it up.
    /// Returns when some mobile worker completes the task via poll+submit.
    pub async fn dispatch_to_mobile_pool(&self, tool_name: &str, input: Value) -> Result<Value> {
        self.dispatch_to_mobile_pool_with_priority(tool_name, input, default_priority()).await
    }

    /// Dispatch to mobile pool with explicit priority.
    pub async fn dispatch_to_mobile_pool_with_priority(&self, tool_name: &str, input: Value, priority: u8) -> Result<Value> {
        let task_id = format!("mp-{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("0000"));
        let (tx, rx) = tokio::sync::oneshot::channel();

        debug!("Enqueuing task '{}' (tool: {}, priority: {}) to shared mobile pool", task_id, tool_name, priority);

        let pending = PendingTask {
            id: task_id.clone(),
            tool: tool_name.to_string(),
            input: input.clone(),
            priority,
            idempotency_key: None,
            result_tx: tx,
            created_at: Instant::now(),
        };

        {
            let mut pool = self.shared_mobile_pool.lock().await;
            pool.push_back(pending);
        }

        let start = Instant::now();
        match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
            Ok(Ok(result)) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                let worker_name = result["worker"].as_str().unwrap_or("mobile-pool");
                self.metrics.record_success(worker_name, latency_ms).await;
                info!("Mobile pool task '{}' (tool: {}) completed in {}ms", task_id, tool_name, latency_ms);
                Ok(result)
            }
            Ok(Err(_)) => {
                let msg = format!("Mobile pool task '{}' channel dropped", task_id);
                self.metrics.record_failure("mobile-pool", &msg).await;
                Err(anyhow!(msg))
            }
            Err(_) => {
                let msg = format!("Mobile pool task '{}' timed out after 120s", task_id);
                self.metrics.record_failure("mobile-pool", &msg).await;
                let mut pool = self.shared_mobile_pool.lock().await;
                pool.retain(|t| t.id != task_id);
                Err(anyhow!(msg))
            }
        }
    }

    /// Batch dispatch — send multiple inputs for the same tool across different workers.
    /// Round-robins across available workers for maximum parallelism.
    pub async fn dispatch_batch(&self, tool_name: &str, inputs: Vec<Value>) -> Vec<Result<Value>> {
        let routing = self.tool_routing(tool_name);
        let workers = match routing {
            ToolRouting::Local => {
                return inputs.iter().map(|_| Err(anyhow!("Tool '{}' is local-only", tool_name))).collect();
            }
            ToolRouting::MobileOnly => {
                let all = self.registry.online_workers().await;
                all.into_iter().filter(|w| w.device_type == "mobile").collect::<Vec<_>>()
            }
            ToolRouting::AnyWorker => {
                self.registry.online_workers().await
            }
            ToolRouting::FullWorkerOnly => {
                let all = self.registry.online_workers().await;
                all.into_iter().filter(|n| n.capabilities.iter().any(|c| c == "tools")).collect::<Vec<_>>()
            }
        };

        if workers.is_empty() {
            return inputs.iter().map(|_| Err(anyhow!("No workers available for tool '{}'", tool_name))).collect();
        }

        info!("Batch dispatch: {} tasks for tool '{}' across {} workers", inputs.len(), tool_name, workers.len());

        let mut handles = Vec::with_capacity(inputs.len());
        for (i, input) in inputs.into_iter().enumerate() {
            let worker = workers[i % workers.len()].clone();
            let tool = tool_name.to_string();
            // We need Arc<Self> — build futures that share the hub reference
            // Since we can't easily get Arc<Self> here, dispatch via the existing methods
            let worker_name = worker.name.clone();
            let _hub_registry = self.registry.clone();
            let client = self.http_client.clone();
            let metrics = self.metrics.clone();

            handles.push(tokio::spawn(async move {
                let url = format!("http://{}:{}/worker/execute", worker.host, worker.port);
                let payload = json!({ "tool": tool, "input": input });
                let start = std::time::Instant::now();

                let response = client
                    .post(&url)
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|e| anyhow!("Worker '{}' unreachable: {}", worker_name, e))?;

                let latency_ms = start.elapsed().as_millis() as u64;

                if !response.status().is_success() {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    let msg = format!("Worker '{}' returned {}: {}", worker_name, status, body);
                    metrics.record_failure(&worker_name, &msg).await;
                    return Err(anyhow!(msg));
                }

                let result: WorkerToolResponse = response.json().await
                    .map_err(|e| anyhow!("Failed to parse worker response: {}", e))?;

                metrics.record_success(&worker_name, latency_ms).await;

                Ok(json!({
                    "success": result.success,
                    "output": result.output,
                    "worker": worker_name,
                    "latency_ms": latency_ms,
                }))
            }));
        }

        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            match handle.await {
                Ok(r) => results.push(r),
                Err(e) => results.push(Err(anyhow!("Task join error: {}", e))),
            }
        }
        results
    }

    /// Dispatch an agent-level task to a specific worker.
    /// The worker will autonomously plan + execute using /agent/think callback.
    /// Returns when the worker completes (or times out at 600s for agent tasks).
    pub async fn dispatch_agent_task(
        &self,
        worker_name: &str,
        goal: String,
        max_iterations: u32,
        available_tools: Vec<String>,
    ) -> Result<Value> {
        let task_id = format!("ag-{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("0000"));
        let (tx, rx) = tokio::sync::oneshot::channel();

        info!("Dispatching agent task '{}' to worker '{}': goal='{}'",
            task_id, worker_name, truncate_str(&goal, 80));

        let agent_task = AgentTask {
            task_id: task_id.clone(),
            goal,
            max_iterations,
            available_tools,
        };

        let pending = PendingAgentTask {
            task: agent_task,
            priority: default_priority(),
            result_tx: tx,
            created_at: Instant::now(),
        };

        {
            let mut queues = self.pending_agent_tasks.lock().await;
            queues.entry(worker_name.to_string()).or_default().push_back(pending);
        }

        // Agent tasks get longer timeout (600s = 10 min)
        let start = Instant::now();
        match tokio::time::timeout(std::time::Duration::from_secs(600), rx).await {
            Ok(Ok(result)) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                self.metrics.record_success(worker_name, latency_ms).await;
                info!("Agent task '{}' completed on '{}' in {}ms", task_id, worker_name, latency_ms);
                Ok(result)
            }
            Ok(Err(_)) => {
                let msg = format!("Agent task '{}' channel dropped", task_id);
                self.metrics.record_failure(worker_name, &msg).await;
                Err(anyhow!(msg))
            }
            Err(_) => {
                let msg = format!("Agent task '{}' timed out after 600s", task_id);
                self.metrics.record_failure(worker_name, &msg).await;
                let mut queues = self.pending_agent_tasks.lock().await;
                if let Some(queue) = queues.get_mut(worker_name) {
                    queue.retain(|t| t.task.task_id != task_id);
                }
                Err(anyhow!(msg))
            }
        }
    }

    /// Dispatch an agent task to the shared mobile pool — any mobile worker can pick it up.
    pub async fn dispatch_agent_task_to_pool(
        &self,
        goal: String,
        max_iterations: u32,
        available_tools: Vec<String>,
    ) -> Result<Value> {
        let task_id = format!("ap-{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("0000"));
        let (tx, rx) = tokio::sync::oneshot::channel();

        info!("Dispatching agent task '{}' to shared pool: goal='{}'", task_id, truncate_str(&goal, 80));

        let agent_task = AgentTask {
            task_id: task_id.clone(),
            goal,
            max_iterations,
            available_tools,
        };

        let pending = PendingAgentTask {
            task: agent_task,
            priority: default_priority(),
            result_tx: tx,
            created_at: Instant::now(),
        };

        {
            let mut pool = self.shared_agent_pool.lock().await;
            pool.push_back(pending);
        }

        let start = Instant::now();
        match tokio::time::timeout(std::time::Duration::from_secs(600), rx).await {
            Ok(Ok(result)) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                let worker_name = result["worker"].as_str().unwrap_or("agent-pool");
                self.metrics.record_success(worker_name, latency_ms).await;
                info!("Agent pool task '{}' completed in {}ms", task_id, latency_ms);
                Ok(result)
            }
            Ok(Err(_)) => {
                let msg = format!("Agent pool task '{}' channel dropped", task_id);
                self.metrics.record_failure("agent-pool", &msg).await;
                Err(anyhow!(msg))
            }
            Err(_) => {
                let msg = format!("Agent pool task '{}' timed out after 600s", task_id);
                self.metrics.record_failure("agent-pool", &msg).await;
                let mut pool = self.shared_agent_pool.lock().await;
                pool.retain(|t| t.task.task_id != task_id);
                Err(anyhow!(msg))
            }
        }
    }

    /// Called by GET /cluster/poll — returns the next pending task for a worker, or None.
    /// Priority: agent_task > per-worker tool task > shared agent pool > shared mobile pool.
    /// Also acts as a heartbeat for the worker.
    pub async fn poll_task(&self, worker_name: &str) -> Option<PollTaskResponse> {
        // Update heartbeat (treat poll as heartbeat for mobile workers)
        let _ = self.registry.heartbeat(worker_name, 0.0).await;

        // Priority 1: check per-worker agent task queue (highest-priority first)
        {
            let mut queues = self.pending_agent_tasks.lock().await;
            if let Some(queue) = queues.get_mut(worker_name) {
                if let Some(idx) = Self::best_priority_agent_idx(queue) {
                    let agent_pending = queue.remove(idx).unwrap();
                    let task_id = agent_pending.task.task_id.clone();
                    let priority = agent_pending.priority;
                    let response = PollTaskResponse {
                        task_id: task_id.clone(),
                        tool: "__agent_task__".to_string(),
                        input: serde_json::to_value(&agent_pending.task).unwrap_or_default(),
                        priority,
                    };
                    let mut inflight = self.inflight_results.lock().await;
                    inflight.insert(task_id, agent_pending.result_tx);
                    return Some(response);
                }
            }
        }

        // Priority 2: check per-worker tool task queue (highest-priority first)
        {
            let mut queues = self.pending_tasks.lock().await;
            if let Some(queue) = queues.get_mut(worker_name) {
                if let Some(idx) = Self::best_priority_task_idx(queue) {
                    let task = queue.remove(idx).unwrap();
                    let priority = task.priority;
                    let response = PollTaskResponse {
                        task_id: task.id.clone(),
                        tool: task.tool.clone(),
                        input: task.input.clone(),
                        priority,
                    };
                    let mut inflight = self.inflight_results.lock().await;
                    inflight.insert(task.id, task.result_tx);
                    return Some(response);
                }
            }
        }

        // Priority 3: shared agent pool (highest-priority first)
        {
            let mut pool = self.shared_agent_pool.lock().await;
            if let Some(idx) = Self::best_priority_agent_idx(&pool) {
                let agent_pending = pool.remove(idx).unwrap();
                let task_id = agent_pending.task.task_id.clone();
                let priority = agent_pending.priority;
                let response = PollTaskResponse {
                    task_id: task_id.clone(),
                    tool: "__agent_task__".to_string(),
                    input: serde_json::to_value(&agent_pending.task).unwrap_or_default(),
                    priority,
                };
                let mut inflight = self.inflight_results.lock().await;
                inflight.insert(task_id, agent_pending.result_tx);
                return Some(response);
            }
        }

        // Priority 4: shared mobile pool (highest-priority first)
        {
            let mut pool = self.shared_mobile_pool.lock().await;
            if let Some(idx) = Self::best_priority_task_idx(&pool) {
                let task = pool.remove(idx).unwrap();
                let priority = task.priority;
                let response = PollTaskResponse {
                    task_id: task.id.clone(),
                    tool: task.tool.clone(),
                    input: task.input.clone(),
                    priority,
                };
                let mut inflight = self.inflight_results.lock().await;
                inflight.insert(task.id, task.result_tx);
                return Some(response);
            }
        }

        None
    }

    /// Called by POST /cluster/result — worker submits the completed task result.
    pub async fn submit_result(&self, payload: TaskResultPayload) -> Result<()> {
        let mut inflight = self.inflight_results.lock().await;
        if let Some(tx) = inflight.remove(&payload.task_id) {
            let worker_name = payload.worker.clone().unwrap_or_else(|| "unknown".to_string());
            let result = json!({
                "success": payload.success,
                "output": payload.output,
                "worker": worker_name,
            });
            tx.send(result).map_err(|_| anyhow!("Result receiver already dropped for task '{}'", payload.task_id))?;
            Ok(())
        } else {
            Err(anyhow!("No inflight task found with id '{}'", payload.task_id))
        }
    }

    /// Clean up expired pending tasks (call periodically from staleness_loop)
    pub async fn cleanup_expired_tasks(&self, max_age_secs: u64) {
        let cutoff = Instant::now().checked_sub(std::time::Duration::from_secs(max_age_secs));
        let cutoff = match cutoff {
            Some(c) => c,
            None => return,
        };

        let mut queues = self.pending_tasks.lock().await;
        for (_worker, queue) in queues.iter_mut() {
            queue.retain(|task| task.created_at > cutoff);
        }

        // Clean shared mobile pool
        {
            let mut pool = self.shared_mobile_pool.lock().await;
            pool.retain(|task| task.created_at > cutoff);
        }

        // Clean agent task queues (use longer cutoff: 10 min)
        let agent_cutoff = Instant::now().checked_sub(std::time::Duration::from_secs(max_age_secs * 4));
        if let Some(agent_cutoff) = agent_cutoff {
            let mut agent_queues = self.pending_agent_tasks.lock().await;
            for (_worker, queue) in agent_queues.iter_mut() {
                queue.retain(|t| t.created_at > agent_cutoff);
            }
            let mut agent_pool = self.shared_agent_pool.lock().await;
            agent_pool.retain(|t| t.created_at > agent_cutoff);
        }

        let mut inflight = self.inflight_results.lock().await;
        if !inflight.is_empty() {
            debug!("Inflight tasks: {}", inflight.len());
        }
        let _ = inflight;

        // Clean expired idempotency keys
        if let Some(idem_cutoff) = Instant::now().checked_sub(std::time::Duration::from_secs(IDEMPOTENCY_TTL_SECS)) {
            let mut log = self.dispatch_log.lock().await;
            log.retain(|_, ts| *ts > idem_cutoff);
        }
    }

    /// Execute a tool on a specific worker node
    async fn execute_on_worker(&self, worker: &ClusterNode, tool_name: &str, input: Value) -> Result<Value> {
        let url = format!("http://{}:{}/worker/execute", worker.host, worker.port);
        let payload = json!({
            "tool": tool_name,
            "input": input,
        });

        debug!("Dispatching tool '{}' to worker '{}' at {}", tool_name, worker.name, url);
        let start = std::time::Instant::now();

        let response = self.http_client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                let err_msg = format!("Worker '{}' unreachable: {}", worker.name, e);
                warn!("{}", err_msg);
                anyhow!(err_msg)
            })?;

        let latency_ms = start.elapsed().as_millis() as u64;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let err_msg = format!("Worker '{}' returned {}: {}", worker.name, status, body);
            self.metrics.record_failure(&worker.name, &err_msg).await;
            return Err(anyhow!(err_msg));
        }

        let result: WorkerToolResponse = response.json().await
            .map_err(|e| anyhow!("Failed to parse worker response: {}", e))?;

        self.metrics.record_success(&worker.name, latency_ms).await;
        info!("Tool '{}' completed on worker '{}' in {}ms", tool_name, worker.name, latency_ms);

        Ok(json!({
            "success": result.success,
            "output": result.output,
            "worker": worker.name,
            "latency_ms": latency_ms,
        }))
    }

    /// Broadcast a health check to all workers concurrently.
    /// Returns a list of (worker_name, is_healthy) pairs.
    pub async fn broadcast_health_check(&self) -> Vec<(String, bool)> {
        let workers = self.registry.status().await;
        let mut handles = Vec::new();

        for worker in workers.iter().filter(|w| w.name != "local") {
            let url = format!("http://{}:{}/health", worker.host, worker.port);
            let name = worker.name.clone();
            let client = self.http_client.clone();

            handles.push(tokio::spawn(async move {
                let ok = match client.get(&url).send().await {
                    Ok(resp) => resp.status().is_success(),
                    Err(_) => false,
                };
                (name, ok)
            }));
        }

        let mut results = Vec::new();
        for handle in handles {
            if let Ok(result) = handle.await {
                results.push(result);
            }
        }
        results
    }

    /// Background loop that marks stale workers as offline and cleans up expired tasks.
    /// Call this as a tokio::spawn task.
    pub async fn staleness_loop(self: Arc<Self>) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            self.registry.mark_offline_stale(90).await; // 90s timeout (3 missed heartbeats at 15s each + buffer)
            self.cleanup_expired_tasks(150).await; // 2.5 min — slightly longer than dispatch timeout
        }
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}...", &s[..max]) }
}

/// How a tool should be routed in the cluster
#[derive(Debug, Clone, PartialEq)]
pub enum ToolRouting {
    /// Must execute on the hub (local filesystem/memory)
    Local,
    /// Can execute on any online worker (including light and mobile workers)
    AnyWorker,
    /// Must execute on a full worker (not light workers)
    FullWorkerOnly,
    /// Must execute on a mobile worker (sensors, local_llm, js_exec)
    MobileOnly,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_routing_local() {
        let registry = Arc::new(futures_util::FutureExt::now_or_never(
            ClusterRegistry::new(":memory:")
        ).unwrap().unwrap());
        let hub = ClusterHub::new(registry);

        assert_eq!(hub.tool_routing("file_write"), ToolRouting::Local);
        assert_eq!(hub.tool_routing("file_edit"), ToolRouting::Local);
        assert_eq!(hub.tool_routing("file_read"), ToolRouting::Local);
        assert_eq!(hub.tool_routing("memory_store"), ToolRouting::Local);
        assert_eq!(hub.tool_routing("memory_recall"), ToolRouting::Local);
        assert_eq!(hub.tool_routing("glob_search"), ToolRouting::Local);
        assert_eq!(hub.tool_routing("content_search"), ToolRouting::Local);
    }

    #[test]
    fn test_tool_routing_network() {
        let registry = Arc::new(futures_util::FutureExt::now_or_never(
            ClusterRegistry::new(":memory:")
        ).unwrap().unwrap());
        let hub = ClusterHub::new(registry);

        assert_eq!(hub.tool_routing("web_search"), ToolRouting::AnyWorker);
        assert_eq!(hub.tool_routing("http_request"), ToolRouting::AnyWorker);
        assert_eq!(hub.tool_routing("email_send"), ToolRouting::AnyWorker);
    }

    #[test]
    fn test_tool_routing_full_worker() {
        let registry = Arc::new(futures_util::FutureExt::now_or_never(
            ClusterRegistry::new(":memory:")
        ).unwrap().unwrap());
        let hub = ClusterHub::new(registry);

        assert_eq!(hub.tool_routing("shell"), ToolRouting::FullWorkerOnly);
        assert_eq!(hub.tool_routing("ai_code"), ToolRouting::FullWorkerOnly);
        assert_eq!(hub.tool_routing("skeleton_generate"), ToolRouting::FullWorkerOnly);
        assert_eq!(hub.tool_routing("browser"), ToolRouting::FullWorkerOnly);
    }

    #[test]
    fn test_tool_routing_mobile() {
        let registry = Arc::new(futures_util::FutureExt::now_or_never(
            ClusterRegistry::new(":memory:")
        ).unwrap().unwrap());
        let hub = ClusterHub::new(registry);

        assert_eq!(hub.tool_routing("sensor_gps"), ToolRouting::MobileOnly);
        assert_eq!(hub.tool_routing("sensor_camera"), ToolRouting::MobileOnly);
        assert_eq!(hub.tool_routing("sensor_accel"), ToolRouting::MobileOnly);
        assert_eq!(hub.tool_routing("sensor_audio"), ToolRouting::MobileOnly);
        assert_eq!(hub.tool_routing("local_llm"), ToolRouting::MobileOnly);
        assert_eq!(hub.tool_routing("js_exec"), ToolRouting::MobileOnly);
    }

    #[test]
    fn test_should_dispatch() {
        let registry = Arc::new(futures_util::FutureExt::now_or_never(
            ClusterRegistry::new(":memory:")
        ).unwrap().unwrap());
        let hub = ClusterHub::new(registry);

        assert!(!hub.should_dispatch("file_write"));
        assert!(!hub.should_dispatch("memory_store"));
        assert!(hub.should_dispatch("web_search"));
        assert!(hub.should_dispatch("shell"));
        assert!(hub.should_dispatch("sensor_gps"));
        assert!(hub.should_dispatch("local_llm"));
    }

    #[tokio::test]
    async fn test_dispatch_no_workers() {
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        let hub = ClusterHub::new(registry);

        let result = hub.dispatch_tool("web_search", json!({"query": "test"})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No online workers"));
    }

    #[tokio::test]
    async fn test_dispatch_local_only_rejected() {
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        let hub = ClusterHub::new(registry);

        let result = hub.dispatch_tool("file_write", json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("local-only"));
    }

    #[tokio::test]
    async fn test_metrics_snapshot() {
        let metrics = ClusterMetrics::new();
        metrics.record_success("w1", 100).await;
        metrics.record_success("w1", 200).await;
        metrics.record_failure("w2", "timeout").await;

        let snap = metrics.snapshot().await;
        assert_eq!(snap["dispatch_count"], 3);
        assert_eq!(snap["dispatch_failures"], 1);
    }

    #[tokio::test]
    async fn test_metrics_worker_stats() {
        let metrics = ClusterMetrics::new();
        metrics.record_success("w1", 100).await;
        metrics.record_success("w1", 300).await;

        let stats = metrics.worker_stats("w1").await.unwrap();
        assert_eq!(stats.tasks_completed, 2);
        assert_eq!(stats.tasks_failed, 0);
        assert_eq!(stats.avg_latency_ms, 200); // (100 + 300) / 2
    }

    #[tokio::test]
    async fn test_dispatch_to_worker_not_found() {
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        let hub = ClusterHub::new(registry);

        let result = hub.dispatch_to_worker("nonexistent", "shell", json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_dispatch_to_worker_offline() {
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        registry.register_full("acer", "100.0.0.4", 7881, &["tools".into()], "light").await.unwrap();
        // Force offline
        {
            let conn = registry.conn.lock().unwrap();
            conn.execute("UPDATE cluster_nodes SET status = 'offline' WHERE name = 'acer'", []).unwrap();
        }
        let hub = ClusterHub::new(registry);

        let result = hub.dispatch_to_worker("acer", "shell", json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("offline"));
    }

    #[tokio::test]
    async fn test_dispatch_by_capability_not_found() {
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        let hub = ClusterHub::new(registry);

        let result = hub.dispatch_by_capability("ios_build", "shell", json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No online worker"));
    }

    #[tokio::test]
    async fn test_broadcast_health_no_workers() {
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        let hub = ClusterHub::new(registry);

        let results = hub.broadcast_health_check().await;
        assert!(results.is_empty()); // only 'local' exists, excluded
    }

    #[tokio::test]
    async fn test_poll_task_empty() {
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        let hub = ClusterHub::new(registry);

        let result = hub.poll_task("android1").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_poll_and_submit_result() {
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        registry.register_full(
            "android1", "0.0.0.0", 0,
            &["web_search".to_string(), "http_request".to_string()],
            "mobile",
        ).await.unwrap();
        let hub = Arc::new(ClusterHub::new(registry));

        // Manually enqueue a task for android1
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut queues = hub.pending_tasks.lock().await;
            queues.entry("android1".to_string()).or_default().push_back(PendingTask {
                id: "mt-test1".to_string(),
                tool: "web_search".to_string(),
                input: json!({"query": "test"}),
                priority: default_priority(),
                idempotency_key: None,
                result_tx: tx,
                created_at: Instant::now(),
            });
        }

        // Poll should return the task
        let polled = hub.poll_task("android1").await;
        assert!(polled.is_some());
        let task = polled.unwrap();
        assert_eq!(task.task_id, "mt-test1");
        assert_eq!(task.tool, "web_search");

        // Submit result
        hub.submit_result(TaskResultPayload {
            task_id: "mt-test1".to_string(),
            success: true,
            output: "search results here".to_string(),
            worker: Some("android1".to_string()),
        }).await.unwrap();

        // The oneshot should have received the result
        let result = rx.await.unwrap();
        assert_eq!(result["success"], true);
        assert_eq!(result["output"], "search results here");
    }

    #[tokio::test]
    async fn test_submit_result_unknown_task() {
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        let hub = ClusterHub::new(registry);

        let result = hub.submit_result(TaskResultPayload {
            task_id: "nonexistent".to_string(),
            success: true,
            output: "test".to_string(),
            worker: None,
        }).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No inflight task"));
    }

    #[tokio::test]
    async fn test_dispatch_mobile_no_workers() {
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        let hub = ClusterHub::new(registry);

        let result = hub.dispatch_tool("sensor_gps", json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No mobile workers"));
    }

    #[tokio::test]
    async fn test_agent_task_poll_priority() {
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        registry.register_full(
            "rog6", "0.0.0.0", 0,
            &["web_search".to_string()],
            "mobile",
        ).await.unwrap();
        let hub = Arc::new(ClusterHub::new(registry));

        // Enqueue a regular tool task
        let (tx1, _rx1) = tokio::sync::oneshot::channel();
        {
            let mut queues = hub.pending_tasks.lock().await;
            queues.entry("rog6".to_string()).or_default().push_back(PendingTask {
                id: "mt-tool1".to_string(),
                tool: "web_search".to_string(),
                input: json!({"query": "test"}),
                priority: default_priority(),
                idempotency_key: None,
                result_tx: tx1,
                created_at: Instant::now(),
            });
        }

        // Enqueue an agent task
        let (tx2, _rx2) = tokio::sync::oneshot::channel();
        {
            let mut queues = hub.pending_agent_tasks.lock().await;
            queues.entry("rog6".to_string()).or_default().push_back(PendingAgentTask {
                task: AgentTask {
                    task_id: "ag-agent1".to_string(),
                    goal: "Research pricing".to_string(),
                    max_iterations: 5,
                    available_tools: vec!["web_search".to_string()],
                },
                priority: default_priority(),
                result_tx: tx2,
                created_at: Instant::now(),
            });
        }

        // Poll should return agent_task first (higher priority)
        let polled = hub.poll_task("rog6").await;
        assert!(polled.is_some());
        let task = polled.unwrap();
        assert_eq!(task.tool, "__agent_task__");
        assert_eq!(task.task_id, "ag-agent1");

        // Next poll should return the regular tool task
        let polled2 = hub.poll_task("rog6").await;
        assert!(polled2.is_some());
        assert_eq!(polled2.unwrap().tool, "web_search");
    }

    #[tokio::test]
    async fn test_agent_task_shared_pool() {
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        registry.register_full(
            "ipad", "0.0.0.0", 0,
            &["web_search".to_string()],
            "mobile",
        ).await.unwrap();
        let hub = Arc::new(ClusterHub::new(registry));

        // Enqueue to shared agent pool
        let (tx, _rx) = tokio::sync::oneshot::channel();
        {
            let mut pool = hub.shared_agent_pool.lock().await;
            pool.push_back(PendingAgentTask {
                task: AgentTask {
                    task_id: "ap-shared1".to_string(),
                    goal: "Search trends".to_string(),
                    max_iterations: 3,
                    available_tools: vec!["web_search".to_string()],
                },
                priority: default_priority(),
                result_tx: tx,
                created_at: Instant::now(),
            });
        }

        // Any mobile worker can pick it up
        let polled = hub.poll_task("ipad").await;
        assert!(polled.is_some());
        let task = polled.unwrap();
        assert_eq!(task.tool, "__agent_task__");
        assert_eq!(task.task_id, "ap-shared1");

        // Submit result
        hub.submit_result(TaskResultPayload {
            task_id: "ap-shared1".to_string(),
            success: true,
            output: "Trends: AI agents growing".to_string(),
            worker: Some("ipad".to_string()),
        }).await.unwrap();
    }

    #[test]
    fn test_agent_task_serialize() {
        let task = AgentTask {
            task_id: "ag-123".to_string(),
            goal: "Research pricing".to_string(),
            max_iterations: 8,
            available_tools: vec!["web_search".to_string(), "http_request".to_string()],
        };
        let json = serde_json::to_value(&task).unwrap();
        assert_eq!(json["task_id"], "ag-123");
        assert_eq!(json["max_iterations"], 8);
        assert_eq!(json["available_tools"].as_array().unwrap().len(), 2);
    }

    // ── SLA Priority Tests ──────────────────────────────────────────

    #[tokio::test]
    async fn test_priority_ordering() {
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        registry.register_full("w1", "0.0.0.0", 0, &["web_search".into()], "mobile").await.unwrap();
        let hub = Arc::new(ClusterHub::new(registry));

        // Enqueue 3 tasks with different priorities
        for (id, prio) in [("t-low", 200u8), ("t-high", 10u8), ("t-mid", 100u8)] {
            let (tx, _rx) = tokio::sync::oneshot::channel();
            hub.pending_tasks.lock().await
                .entry("w1".to_string()).or_default()
                .push_back(PendingTask {
                    id: id.to_string(),
                    tool: "web_search".to_string(),
                    input: json!({}),
                    priority: prio,
                    idempotency_key: None,
                    result_tx: tx,
                    created_at: Instant::now(),
                });
        }

        // Poll should return highest priority (lowest number) first
        let p1 = hub.poll_task("w1").await.unwrap();
        assert_eq!(p1.task_id, "t-high");
        assert_eq!(p1.priority, 10);

        let p2 = hub.poll_task("w1").await.unwrap();
        assert_eq!(p2.task_id, "t-mid");
        assert_eq!(p2.priority, 100);

        let p3 = hub.poll_task("w1").await.unwrap();
        assert_eq!(p3.task_id, "t-low");
        assert_eq!(p3.priority, 200);
    }

    #[tokio::test]
    async fn test_priority_fifo_within_same() {
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        registry.register_full("w1", "0.0.0.0", 0, &["web_search".into()], "mobile").await.unwrap();
        let hub = Arc::new(ClusterHub::new(registry));

        // Enqueue 3 tasks with same priority — should be FIFO
        for id in ["t-first", "t-second", "t-third"] {
            let (tx, _rx) = tokio::sync::oneshot::channel();
            hub.pending_tasks.lock().await
                .entry("w1".to_string()).or_default()
                .push_back(PendingTask {
                    id: id.to_string(),
                    tool: "web_search".to_string(),
                    input: json!({}),
                    priority: 50,
                    idempotency_key: None,
                    result_tx: tx,
                    created_at: Instant::now(),
                });
        }

        // min_by_key returns first match when equal → FIFO preserved
        let p1 = hub.poll_task("w1").await.unwrap();
        assert_eq!(p1.task_id, "t-first");
        let p2 = hub.poll_task("w1").await.unwrap();
        assert_eq!(p2.task_id, "t-second");
        let p3 = hub.poll_task("w1").await.unwrap();
        assert_eq!(p3.task_id, "t-third");
    }

    // ── Idempotency Tests ───────────────────────────────────────────

    #[tokio::test]
    async fn test_idempotency_dedup() {
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        let hub = ClusterHub::new(registry);

        // First dispatch with key should succeed
        hub.check_idempotency("key-abc").await.unwrap();

        // Second dispatch with same key should fail
        let result = hub.check_idempotency("key-abc").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Duplicate dispatch"));
    }

    #[tokio::test]
    async fn test_idempotency_different_keys() {
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        let hub = ClusterHub::new(registry);

        hub.check_idempotency("key-1").await.unwrap();
        hub.check_idempotency("key-2").await.unwrap();
        hub.check_idempotency("key-3").await.unwrap();
        // All different keys → all succeed
    }

    #[tokio::test]
    async fn test_idempotency_cleanup() {
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        let hub = ClusterHub::new(registry);

        // Insert a key with old timestamp
        {
            let mut log = hub.dispatch_log.lock().await;
            let old_time = Instant::now().checked_sub(std::time::Duration::from_secs(600)).unwrap();
            log.insert("old-key".to_string(), old_time);
            log.insert("new-key".to_string(), Instant::now());
        }

        hub.cleanup_expired_tasks(150).await;

        let log = hub.dispatch_log.lock().await;
        assert!(!log.contains_key("old-key")); // cleaned up
        assert!(log.contains_key("new-key")); // still valid
    }

    #[test]
    fn test_default_priority() {
        assert_eq!(default_priority(), 100);
    }

    #[test]
    fn test_poll_response_includes_priority() {
        let resp = PollTaskResponse {
            task_id: "t-1".to_string(),
            tool: "web_search".to_string(),
            input: json!({}),
            priority: 10,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["priority"], 10);
    }

    #[test]
    fn test_poll_response_default_priority() {
        let json_str = r#"{"task_id":"t-1","tool":"test","input":{}}"#;
        let resp: PollTaskResponse = serde_json::from_str(json_str).unwrap();
        assert_eq!(resp.priority, 100); // default
    }
}
