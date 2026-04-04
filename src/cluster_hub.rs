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
use crate::concurrency_manager::{ConcurrencyManager, ConcurrencyPermit};
use crate::task_taxonomy::TaskTaxonomy;

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

/// Consolidated task state — single lock for all task routing data.
struct TaskState {
    /// Per-worker queue of pending tasks (for polling/mobile workers)
    pending_tasks: HashMap<String, VecDeque<PendingTask>>,
    /// Map of task_id → oneshot sender, for results that arrive via POST /cluster/result
    inflight_results: HashMap<String, tokio::sync::oneshot::Sender<Value>>,
    /// Number of in-flight tasks per worker (for load-aware routing)
    inflight_counts: HashMap<String, u32>,
    /// Shared task pool for mobile workers (any mobile worker can pick up)
    shared_mobile_pool: VecDeque<PendingTask>,
    /// Per-worker queue of agent tasks (higher-level autonomous tasks)
    pending_agent_tasks: HashMap<String, VecDeque<PendingAgentTask>>,
    /// Shared agent task pool for mobile workers
    shared_agent_pool: VecDeque<PendingAgentTask>,
}

impl TaskState {
    fn new() -> Self {
        Self {
            pending_tasks: HashMap::new(),
            inflight_results: HashMap::new(),
            inflight_counts: HashMap::new(),
            shared_mobile_pool: VecDeque::new(),
            pending_agent_tasks: HashMap::new(),
            shared_agent_pool: VecDeque::new(),
        }
    }
}

/// ClusterHub — central coordinator that dispatches tasks to workers
pub struct ClusterHub {
    pub registry: Arc<ClusterRegistry>,
    pub metrics: Arc<ClusterMetrics>,
    http_client: reqwest::Client,
    /// Consolidated task state — single lock for all task routing data
    task_state: Mutex<TaskState>,
    /// Idempotency log: key → timestamp (for dedup within TTL, separate lifecycle)
    dispatch_log: Mutex<HashMap<String, Instant>>,
    /// Optional task taxonomy for category-aware routing
    taxonomy: Option<Arc<TaskTaxonomy>>,
    /// Optional concurrency manager for per-node admission control
    concurrency: Option<Arc<ConcurrencyManager>>,
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
            task_state: Mutex::new(TaskState::new()),
            dispatch_log: Mutex::new(HashMap::new()),
            taxonomy: None,
            concurrency: None,
        }
    }

    /// Set the task taxonomy for category-aware dispatch routing.
    pub fn set_taxonomy(&mut self, taxonomy: Arc<TaskTaxonomy>) {
        self.taxonomy = Some(taxonomy);
    }

    /// Set the concurrency manager for per-node admission control.
    pub fn set_concurrency(&mut self, concurrency: Arc<ConcurrencyManager>) {
        self.concurrency = Some(concurrency);
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

    /// Get effective loads for multiple workers in a single lock acquisition.
    /// Returns a Vec of f32 loads in the same order as the input workers.
    async fn effective_loads(&self, workers: &[ClusterNode]) -> Vec<f32> {
        let state = self.task_state.lock().await;
        workers.iter().map(|w| {
            let inflight = state.inflight_counts.get(&w.name).copied().unwrap_or(0);
            w.cpu_load + (inflight as f32 * 0.15)
        }).collect()
    }

    /// Increment inflight count for a worker.
    async fn inc_inflight(&self, worker_name: &str) {
        let mut state = self.task_state.lock().await;
        *state.inflight_counts.entry(worker_name.to_string()).or_insert(0) += 1;
    }

    /// Decrement inflight count for a worker.
    async fn dec_inflight(&self, worker_name: &str) {
        let mut state = self.task_state.lock().await;
        if let Some(count) = state.inflight_counts.get_mut(worker_name) {
            *count = count.saturating_sub(1);
        }
    }

    /// Reorder workers by taxonomy preference: preferred first, then fallback,
    /// then any remaining candidates (preserving load-based ordering within each group).
    fn reorder_by_taxonomy(
        candidates: &[ClusterNode],
        preferred: &[String],
        fallback: &[String],
    ) -> Vec<ClusterNode> {
        let mut preferred_workers: Vec<ClusterNode> = Vec::new();
        let mut fallback_workers: Vec<ClusterNode> = Vec::new();
        let mut rest: Vec<ClusterNode> = Vec::new();

        for w in candidates {
            if preferred.iter().any(|p| p == &w.name) {
                preferred_workers.push(w.clone());
            } else if fallback.iter().any(|f| f == &w.name) {
                fallback_workers.push(w.clone());
            } else {
                rest.push(w.clone());
            }
        }

        // Preserve the preferred_nodes ordering: iterate preferred names in order
        let mut ordered_preferred: Vec<ClusterNode> = Vec::new();
        for pname in preferred {
            if let Some(w) = preferred_workers.iter().find(|w| &w.name == pname) {
                ordered_preferred.push(w.clone());
            }
        }

        let mut ordered_fallback: Vec<ClusterNode> = Vec::new();
        for fname in fallback {
            if let Some(w) = fallback_workers.iter().find(|w| &w.name == fname) {
                ordered_fallback.push(w.clone());
            }
        }

        let mut result = ordered_preferred;
        result.extend(ordered_fallback);
        result.extend(rest);
        result
    }

    /// Try to acquire a concurrency permit for a worker. Returns the permit if
    /// successful, or None if the node is at capacity / unknown.
    fn try_concurrency_permit(&self, worker_name: &str) -> Option<ConcurrencyPermit> {
        if let Some(ref mgr) = self.concurrency {
            match mgr.try_acquire(worker_name) {
                Ok(permit) => Some(permit),
                Err(reason) => {
                    debug!(worker = worker_name, reason = %reason, "concurrency permit denied");
                    None
                }
            }
        } else {
            // No concurrency manager — always allowed (backward compat).
            None
        }
    }

    /// Dispatch a tool call to the best available worker.
    /// For mobile workers (polling mode), enqueues the task and waits for the result.
    /// For push workers, sends an HTTP POST directly.
    ///
    /// When a `TaskTaxonomy` is set, workers are prioritized by the taxonomy
    /// profile's `preferred_nodes` / `fallback_nodes`.
    ///
    /// When a `ConcurrencyManager` is set, each candidate worker must have an
    /// available concurrency slot before dispatch. The RAII permit is held for
    /// the duration of the dispatch and auto-released on completion.
    pub async fn dispatch_tool(&self, tool_name: &str, input: Value) -> Result<Value> {
        self.dispatch_tool_ext(tool_name, input, None).await
    }

    /// Extended dispatch that accepts an optional hand_name for taxonomy classification.
    pub async fn dispatch_tool_ext(&self, tool_name: &str, input: Value, hand_name: Option<&str>) -> Result<Value> {
        let routing = self.tool_routing(tool_name);
        if routing == ToolRouting::Local {
            return Err(anyhow!("Tool '{}' is local-only", tool_name));
        }

        // Gather candidate workers based on routing type.
        let workers = self.registry.online_workers().await;
        let candidates: Vec<ClusterNode> = match routing {
            ToolRouting::Local => unreachable!(),
            ToolRouting::MobileOnly => workers.into_iter().filter(|w| w.device_type == "mobile").collect(),
            ToolRouting::AnyWorker => workers,
            ToolRouting::FullWorkerOnly => workers.into_iter().filter(|n| n.capabilities.iter().any(|c| c == "tools")).collect(),
        };

        if candidates.is_empty() {
            let msg = match routing {
                ToolRouting::MobileOnly => format!("No mobile workers available for tool '{}'", tool_name),
                _ => format!("No online workers available for tool '{}'", tool_name),
            };
            self.metrics.record_failure("none", &msg).await;
            return Err(anyhow!(msg));
        }

        // Reorder candidates by taxonomy if available.
        let ordered = if let Some(ref tax) = self.taxonomy {
            let (_cat, profile) = tax.classify_and_profile(tool_name, hand_name);
            debug!(
                tool = tool_name,
                hand = ?hand_name,
                category = %_cat,
                preferred = ?profile.preferred_nodes,
                fallback = ?profile.fallback_nodes,
                "taxonomy classification for dispatch"
            );
            Self::reorder_by_taxonomy(&candidates, &profile.preferred_nodes, &profile.fallback_nodes)
        } else {
            // No taxonomy — sort by effective load (backward compat).
            let sorted = candidates.clone();
            // Batch read: single lock for all workers' effective loads.
            let eff_loads = self.effective_loads(&sorted).await;
            let mut indexed: Vec<(usize, f32)> = eff_loads.into_iter().enumerate().collect();
            indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            let reordered: Vec<ClusterNode> = indexed.iter().map(|(i, _)| sorted[*i].clone()).collect();
            reordered
        };

        // Try each candidate in order, checking concurrency limits.
        if self.concurrency.is_some() {
            // With concurrency manager: walk candidates, acquire permit, dispatch.
            for worker in &ordered {
                if let Some(permit) = self.try_concurrency_permit(&worker.name) {
                    debug!(worker = %worker.name, "concurrency permit acquired, dispatching");
                    self.inc_inflight(&worker.name).await;
                    let result = if worker.device_type == "mobile" {
                        self.dispatch_to_mobile(worker, tool_name, input.clone(), default_priority(), None).await
                    } else {
                        self.execute_on_worker(worker, tool_name, input.clone()).await
                    };
                    self.dec_inflight(&worker.name).await;
                    // Permit is dropped here (RAII release).
                    drop(permit);
                    return result;
                }
                // This worker is at capacity, try next.
                debug!(worker = %worker.name, "skipping (at capacity), trying next");
            }
            // All workers at capacity.
            let msg = format!(
                "All workers at concurrency capacity for tool '{}' ({} candidates checked)",
                tool_name,
                ordered.len()
            );
            self.metrics.record_failure("none", &msg).await;
            return Err(anyhow!(msg));
        } else {
            // No concurrency manager — use first candidate (best by taxonomy or load).
            let worker = &ordered[0];
            self.inc_inflight(&worker.name).await;
            let result = if worker.device_type == "mobile" {
                self.dispatch_to_mobile(worker, tool_name, input, default_priority(), None).await
            } else {
                self.execute_on_worker(worker, tool_name, input).await
            };
            self.dec_inflight(&worker.name).await;
            return result;
        }
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
                // Batch read: single lock for all workers' effective loads.
                let eff_loads = self.effective_loads(&filtered).await;
                let best_idx = eff_loads.iter().enumerate()
                    .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                filtered[best_idx].clone()
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
            let mut state = self.task_state.lock().await;
            state.pending_tasks.entry(worker.name.clone()).or_default().push_back(pending);
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
                let mut state = self.task_state.lock().await;
                if let Some(queue) = state.pending_tasks.get_mut(&worker.name) {
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
            let mut state = self.task_state.lock().await;
            state.shared_mobile_pool.push_back(pending);
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
                let mut state = self.task_state.lock().await;
                state.shared_mobile_pool.retain(|t| t.id != task_id);
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
            let mut state = self.task_state.lock().await;
            state.pending_agent_tasks.entry(worker_name.to_string()).or_default().push_back(pending);
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
                let mut state = self.task_state.lock().await;
                if let Some(queue) = state.pending_agent_tasks.get_mut(worker_name) {
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
            let mut state = self.task_state.lock().await;
            state.shared_agent_pool.push_back(pending);
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
                let mut state = self.task_state.lock().await;
                state.shared_agent_pool.retain(|t| t.task.task_id != task_id);
                Err(anyhow!(msg))
            }
        }
    }

    /// Called by GET /cluster/poll — returns the next pending task for a worker, or None.
    /// Priority: agent_task > per-worker tool task > shared agent pool > shared mobile pool.
    /// Also acts as a heartbeat for the worker.
    ///
    /// Single lock acquisition — all task queues and inflight tracking are in TaskState.
    pub async fn poll_task(&self, worker_name: &str) -> Option<PollTaskResponse> {
        // Update heartbeat (treat poll as heartbeat for mobile workers)
        let _ = self.registry.heartbeat(worker_name, 0.0).await;

        let mut state = self.task_state.lock().await;

        // Priority 1: check per-worker agent task queue (highest-priority first)
        if let Some(queue) = state.pending_agent_tasks.get_mut(worker_name) {
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
                state.inflight_results.insert(task_id, agent_pending.result_tx);
                return Some(response);
            }
        }

        // Priority 2: check per-worker tool task queue (highest-priority first)
        if let Some(queue) = state.pending_tasks.get_mut(worker_name) {
            if let Some(idx) = Self::best_priority_task_idx(queue) {
                let task = queue.remove(idx).unwrap();
                let priority = task.priority;
                let response = PollTaskResponse {
                    task_id: task.id.clone(),
                    tool: task.tool.clone(),
                    input: task.input.clone(),
                    priority,
                };
                state.inflight_results.insert(task.id, task.result_tx);
                return Some(response);
            }
        }

        // Priority 3: shared agent pool (highest-priority first)
        if let Some(idx) = Self::best_priority_agent_idx(&state.shared_agent_pool) {
            let agent_pending = state.shared_agent_pool.remove(idx).unwrap();
            let task_id = agent_pending.task.task_id.clone();
            let priority = agent_pending.priority;
            let response = PollTaskResponse {
                task_id: task_id.clone(),
                tool: "__agent_task__".to_string(),
                input: serde_json::to_value(&agent_pending.task).unwrap_or_default(),
                priority,
            };
            state.inflight_results.insert(task_id, agent_pending.result_tx);
            return Some(response);
        }

        // Priority 4: shared mobile pool (highest-priority first)
        if let Some(idx) = Self::best_priority_task_idx(&state.shared_mobile_pool) {
            let task = state.shared_mobile_pool.remove(idx).unwrap();
            let priority = task.priority;
            let response = PollTaskResponse {
                task_id: task.id.clone(),
                tool: task.tool.clone(),
                input: task.input.clone(),
                priority,
            };
            state.inflight_results.insert(task.id, task.result_tx);
            return Some(response);
        }

        None
    }

    /// Called by POST /cluster/result — worker submits the completed task result.
    pub async fn submit_result(&self, payload: TaskResultPayload) -> Result<()> {
        let mut state = self.task_state.lock().await;
        if let Some(tx) = state.inflight_results.remove(&payload.task_id) {
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

        // Single lock for all task state cleanup
        {
            let mut state = self.task_state.lock().await;

            // Clean per-worker task queues
            for (_worker, queue) in state.pending_tasks.iter_mut() {
                queue.retain(|task| task.created_at > cutoff);
            }

            // Clean shared mobile pool
            state.shared_mobile_pool.retain(|task| task.created_at > cutoff);

            // Clean agent task queues (use longer cutoff: 10 min)
            let agent_cutoff = Instant::now().checked_sub(std::time::Duration::from_secs(max_age_secs * 4));
            if let Some(agent_cutoff) = agent_cutoff {
                for (_worker, queue) in state.pending_agent_tasks.iter_mut() {
                    queue.retain(|t| t.created_at > agent_cutoff);
                }
                state.shared_agent_pool.retain(|t| t.created_at > agent_cutoff);
            }

            if !state.inflight_results.is_empty() {
                debug!("Inflight tasks: {}", state.inflight_results.len());
            }
        }

        // Clean expired idempotency keys (separate lock — different lifecycle)
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
            let mut state = hub.task_state.lock().await;
            state.pending_tasks.entry("android1".to_string()).or_default().push_back(PendingTask {
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
            let mut state = hub.task_state.lock().await;
            state.pending_tasks.entry("rog6".to_string()).or_default().push_back(PendingTask {
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
            let mut state = hub.task_state.lock().await;
            state.pending_agent_tasks.entry("rog6".to_string()).or_default().push_back(PendingAgentTask {
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
            let mut state = hub.task_state.lock().await;
            state.shared_agent_pool.push_back(PendingAgentTask {
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
            hub.task_state.lock().await
                .pending_tasks.entry("w1".to_string()).or_default()
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
            hub.task_state.lock().await
                .pending_tasks.entry("w1".to_string()).or_default()
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

    // ── TaskTaxonomy + ConcurrencyManager Integration Tests ──────────

    #[test]
    fn test_reorder_by_taxonomy_preferred_first() {
        // Build mock ClusterNodes
        let make_node = |name: &str| ClusterNode {
            name: name.to_string(),
            host: "0.0.0.0".to_string(),
            port: 7878,
            status: "online".to_string(),
            models: vec![],
            last_seen: String::new(),
            capabilities: vec!["tools".into()],
            device_type: "full".to_string(),
            cpu_load: 0.1,
        };

        let candidates = vec![
            make_node("Acer"),
            make_node("Z13"),
            make_node("M1Mac"),
            make_node("AYANEO"),
        ];

        let preferred = vec!["Z13".to_string()];
        let fallback = vec!["M1Mac".to_string(), "AYANEO".to_string()];

        let ordered = ClusterHub::reorder_by_taxonomy(&candidates, &preferred, &fallback);

        assert_eq!(ordered.len(), 4);
        assert_eq!(ordered[0].name, "Z13", "preferred node should be first");
        assert_eq!(ordered[1].name, "M1Mac", "first fallback node should be second");
        assert_eq!(ordered[2].name, "AYANEO", "second fallback node should be third");
        assert_eq!(ordered[3].name, "Acer", "remaining node should be last");
    }

    #[test]
    fn test_reorder_by_taxonomy_preferred_unavailable_uses_fallback() {
        let make_node = |name: &str| ClusterNode {
            name: name.to_string(),
            host: "0.0.0.0".to_string(),
            port: 7878,
            status: "online".to_string(),
            models: vec![],
            last_seen: String::new(),
            capabilities: vec!["tools".into()],
            device_type: "full".to_string(),
            cpu_load: 0.1,
        };

        // Z13 (preferred) is NOT in the candidate list — it is offline.
        let candidates = vec![
            make_node("Acer"),
            make_node("AYANEO"),
        ];

        let preferred = vec!["Z13".to_string()];
        let fallback = vec!["AYANEO".to_string(), "Acer".to_string()];

        let ordered = ClusterHub::reorder_by_taxonomy(&candidates, &preferred, &fallback);

        assert_eq!(ordered.len(), 2);
        // Z13 is absent, so fallback nodes come first in their declared order.
        assert_eq!(ordered[0].name, "AYANEO", "first fallback should lead when preferred absent");
        assert_eq!(ordered[1].name, "Acer", "second fallback follows");
    }

    #[tokio::test]
    async fn test_dispatch_uses_taxonomy_preferred_nodes() {
        // Register two workers: Z13 (preferred for Code) and Acer
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        registry.register_full("Z13", "127.0.0.1", 7878, &["tools".into()], "full").await.unwrap();
        registry.register_full("Acer", "127.0.0.2", 7881, &["tools".into()], "full").await.unwrap();

        let mut hub = ClusterHub::new(registry);
        hub.set_taxonomy(Arc::new(TaskTaxonomy::new()));

        // "shell" is a Code tool → taxonomy prefers Z13.
        // dispatch_tool will try to HTTP POST to Z13 which won't be reachable,
        // but we can verify the attempt goes to Z13 by checking the error message.
        let result = hub.dispatch_tool("shell", json!({"cmd": "echo hi"})).await;
        assert!(result.is_err());
        // The error should mention Z13 (the preferred node for Code tasks).
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Z13"),
            "Expected dispatch attempt to Z13 (preferred for Code), got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_dispatch_taxonomy_fallback_when_preferred_unavailable() {
        // Only register Acer — Z13 (preferred for Code) is NOT registered/online.
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        registry.register_full("Acer", "127.0.0.2", 7881, &["tools".into()], "full").await.unwrap();

        let mut hub = ClusterHub::new(registry);
        hub.set_taxonomy(Arc::new(TaskTaxonomy::new()));

        // "shell" is Code → preferred=Z13 (absent), fallback=M1Mac,AYANEO,Acer
        let result = hub.dispatch_tool("shell", json!({"cmd": "echo hi"})).await;
        assert!(result.is_err());
        // Should attempt Acer since it's the only online worker (and in fallback list).
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Acer"),
            "Expected fallback dispatch to Acer, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_concurrency_blocks_overloaded_node() {
        use crate::concurrency_manager::ConcurrencyManager;

        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        registry.register_full("Z13", "127.0.0.1", 7878, &["tools".into()], "full").await.unwrap();

        // Create concurrency manager with limit=1 for Z13.
        let mut limits = HashMap::new();
        limits.insert("Z13".to_string(), 1);
        let concurrency = Arc::new(ConcurrencyManager::new(limits));

        // Pre-acquire the only slot so dispatch finds Z13 at capacity.
        let _permit = concurrency.try_acquire("Z13").unwrap();

        let mut hub = ClusterHub::new(registry);
        hub.set_concurrency(concurrency);

        let result = hub.dispatch_tool("shell", json!({"cmd": "echo hi"})).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("concurrency capacity") || err_msg.contains("at capacity"),
            "Expected concurrency capacity error, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_dispatch_succeeds_after_permit_released() {
        use crate::concurrency_manager::ConcurrencyManager;

        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        registry.register_full("Z13", "127.0.0.1", 7878, &["tools".into()], "full").await.unwrap();

        let mut limits = HashMap::new();
        limits.insert("Z13".to_string(), 1);
        let concurrency = Arc::new(ConcurrencyManager::new(limits));

        // Acquire and then release the permit.
        {
            let _permit = concurrency.try_acquire("Z13").unwrap();
            // permit drops here — slot is freed.
        }

        let mut hub = ClusterHub::new(registry);
        hub.set_concurrency(concurrency);

        // Now dispatch should acquire the permit and attempt to reach Z13.
        // It will fail with a network error (not a concurrency error), proving
        // that the concurrency gate was passed.
        let result = hub.dispatch_tool("shell", json!({"cmd": "echo hi"})).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        // The error should be a network/HTTP error, NOT a concurrency capacity error.
        assert!(
            !err_msg.contains("concurrency capacity"),
            "Should NOT be a concurrency error after permit released, got: {}",
            err_msg
        );
        assert!(
            err_msg.contains("unreachable") || err_msg.contains("Z13"),
            "Expected network dispatch error to Z13, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_concurrency_skips_to_next_worker() {
        use crate::concurrency_manager::ConcurrencyManager;

        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        registry.register_full("Z13", "127.0.0.1", 7878, &["tools".into()], "full").await.unwrap();
        registry.register_full("Acer", "127.0.0.2", 7881, &["tools".into()], "full").await.unwrap();

        let mut limits = HashMap::new();
        limits.insert("Z13".to_string(), 1);
        limits.insert("Acer".to_string(), 2);
        let concurrency = Arc::new(ConcurrencyManager::new(limits));

        // Saturate Z13 so it is at capacity.
        let _z13_permit = concurrency.try_acquire("Z13").unwrap();

        let mut hub = ClusterHub::new(registry);
        hub.set_taxonomy(Arc::new(TaskTaxonomy::new()));
        hub.set_concurrency(concurrency);

        // "shell" is Code → taxonomy prefers Z13. But Z13 is at capacity, so
        // dispatch should skip to the next available worker (Acer, in fallback).
        let result = hub.dispatch_tool("shell", json!({"cmd": "echo hi"})).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        // Should attempt Acer (not Z13) and get a network error.
        assert!(
            err_msg.contains("Acer"),
            "Expected dispatch to skip Z13 and try Acer, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_backward_compat_no_taxonomy_no_concurrency() {
        // When neither taxonomy nor concurrency is set, dispatch_tool should
        // behave exactly as before (select by load, no admission control).
        let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
        registry.register_full("W1", "127.0.0.1", 9001, &["tools".into()], "full").await.unwrap();

        let hub = ClusterHub::new(registry);
        // No set_taxonomy / set_concurrency — both default to None.

        let result = hub.dispatch_tool("shell", json!({"cmd": "echo hi"})).await;
        // Should fail with network error (worker not actually running), not with
        // a taxonomy or concurrency error.
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("unreachable") || err_msg.contains("W1"),
            "Expected plain network dispatch error, got: {}",
            err_msg
        );
        assert!(
            !err_msg.contains("taxonomy") && !err_msg.contains("concurrency"),
            "Should not mention taxonomy/concurrency when not configured, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_set_taxonomy_and_concurrency() {
        let registry = Arc::new(futures_util::FutureExt::now_or_never(
            ClusterRegistry::new(":memory:")
        ).unwrap().unwrap());
        let mut hub = ClusterHub::new(registry);

        assert!(hub.taxonomy.is_none());
        assert!(hub.concurrency.is_none());

        hub.set_taxonomy(Arc::new(TaskTaxonomy::new()));
        hub.set_concurrency(Arc::new(ConcurrencyManager::with_defaults()));

        assert!(hub.taxonomy.is_some());
        assert!(hub.concurrency.is_some());
    }
}
