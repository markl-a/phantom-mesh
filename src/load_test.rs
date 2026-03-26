//! Load Testing Framework — stress testing and endurance testing for Phantom Mesh hands.
//!
//! Provides `LoadTester` with two primary modes:
//! - `run_stress_test`: High-concurrency burst test with ramp-up
//! - `run_endurance_test`: Sustained steady-load test over hours
//!
//! Results are persisted to SQLite (`~/.phantom-mesh/load_tests.db`) for historical comparison.
//! Includes predefined profiles: smoke, normal, heavy.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

use crate::agent_runtime::AgentRuntime;
use crate::hands::{Hand, HandRegistry, HandRunner};
use crate::llm_router::LlmRouter;
use crate::tools::ToolRegistry;

// ── Configuration Types ─────────────────────────────────────────────────────

/// Configuration for a stress test (burst / ramp-up pattern)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StressTestConfig {
    /// Maximum number of concurrent tasks
    pub concurrent_tasks: u32,
    /// Total test duration in seconds
    pub duration_secs: u64,
    /// Ramp-up period in seconds (linearly increases concurrency from 1 to concurrent_tasks)
    pub ramp_up_secs: u64,
    /// Which hands to test (if empty, tests all available hands)
    pub target_hands: Vec<String>,
    /// Load multiplier (1.0 = normal, 2.0 = 2x, etc.)
    pub multiplier: f64,
}

impl Default for StressTestConfig {
    fn default() -> Self {
        Self {
            concurrent_tasks: 4,
            duration_secs: 300,
            ramp_up_secs: 30,
            target_hands: vec![],
            multiplier: 1.0,
        }
    }
}

/// Configuration for an endurance test (sustained load over hours)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnduranceConfig {
    /// Steady-state load: tasks submitted per minute
    pub steady_load: u32,
    /// Test duration in hours
    pub duration_hours: u64,
    /// Which hands to test (if empty, tests all available hands)
    pub target_hands: Vec<String>,
}

impl Default for EnduranceConfig {
    fn default() -> Self {
        Self {
            steady_load: 2,
            duration_hours: 1,
            target_hands: vec![],
        }
    }
}

// ── Report Types ────────────────────────────────────────────────────────────

/// Per-second metrics snapshot for timeline visualization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelinePoint {
    /// Seconds elapsed since test start
    pub elapsed_secs: u64,
    /// Number of tasks active at this second
    pub active_tasks: u32,
    /// Tasks completed during this second
    pub completed: u32,
    /// Tasks failed during this second
    pub failed: u32,
    /// Average latency of tasks completed this second (ms)
    pub avg_latency_ms: f64,
}

/// Report from a completed stress test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StressTestReport {
    /// Unique test run ID
    pub run_id: String,
    /// When the test started
    pub started_at: DateTime<Utc>,
    /// When the test ended
    pub ended_at: DateTime<Utc>,
    /// Total tasks submitted
    pub total_tasks: u64,
    /// Successfully completed tasks
    pub successful: u64,
    /// Failed tasks
    pub failed: u64,
    /// Success rate (0.0 - 1.0)
    pub success_rate: f64,
    /// Average latency in milliseconds
    pub avg_latency_ms: f64,
    /// Median (p50) latency in milliseconds
    pub p50_latency: f64,
    /// 95th percentile latency in milliseconds
    pub p95_latency: f64,
    /// 99th percentile latency in milliseconds
    pub p99_latency: f64,
    /// Error counts grouped by error type/message
    pub errors_by_type: HashMap<String, u64>,
    /// Peak concurrent tasks observed
    pub peak_concurrent: u32,
    /// Per-second timeline
    pub timeline: Vec<TimelinePoint>,
    /// The config used for this test
    pub config: StressTestConfig,
    /// Test profile name (if predefined)
    pub profile: Option<String>,
}

/// Report from a completed endurance test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnduranceReport {
    /// Unique test run ID
    pub run_id: String,
    /// When the test started
    pub started_at: DateTime<Utc>,
    /// When the test ended
    pub ended_at: DateTime<Utc>,
    /// Total tasks submitted
    pub total_tasks: u64,
    /// Successfully completed tasks
    pub successful: u64,
    /// Failed tasks
    pub failed: u64,
    /// Success rate (0.0 - 1.0)
    pub success_rate: f64,
    /// Average latency in milliseconds
    pub avg_latency_ms: f64,
    /// Median (p50) latency in milliseconds
    pub p50_latency: f64,
    /// 95th percentile latency in milliseconds
    pub p95_latency: f64,
    /// 99th percentile latency in milliseconds
    pub p99_latency: f64,
    /// Error counts grouped by error type/message
    pub errors_by_type: HashMap<String, u64>,
    /// Peak concurrent tasks observed
    pub peak_concurrent: u32,
    /// Per-minute metrics for long-running visualization
    pub timeline_minutes: Vec<TimelinePoint>,
    /// The config used for this test
    pub config: EnduranceConfig,
}

// ── Live Progress ───────────────────────────────────────────────────────────

/// Live status of a running test (queryable via GET /test/stress/status)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadTestStatus {
    /// Whether a test is currently running
    pub running: bool,
    /// Test run ID
    pub run_id: String,
    /// Test type: "stress" or "endurance"
    pub test_type: String,
    /// Elapsed seconds since test start
    pub elapsed_secs: u64,
    /// Total planned duration in seconds
    pub total_duration_secs: u64,
    /// Tasks completed so far
    pub completed: u64,
    /// Tasks failed so far
    pub failed: u64,
    /// Currently active (in-flight) tasks
    pub active: u32,
    /// Current success rate
    pub success_rate: f64,
    /// Current average latency
    pub avg_latency_ms: f64,
}

impl Default for LoadTestStatus {
    fn default() -> Self {
        Self {
            running: false,
            run_id: String::new(),
            test_type: String::new(),
            elapsed_secs: 0,
            total_duration_secs: 0,
            completed: 0,
            failed: 0,
            active: 0,
            success_rate: 0.0,
            avg_latency_ms: 0.0,
        }
    }
}

// ── Predefined Profiles ─────────────────────────────────────────────────────

/// Get a predefined test profile by name
pub fn profile(name: &str) -> Option<StressTestConfig> {
    match name {
        "smoke" => Some(StressTestConfig {
            concurrent_tasks: 2,
            duration_secs: 300,      // 5 minutes
            ramp_up_secs: 10,
            target_hands: vec![],
            multiplier: 1.0,
        }),
        "normal" => Some(StressTestConfig {
            concurrent_tasks: 8,
            duration_secs: 1800,     // 30 minutes
            ramp_up_secs: 60,
            target_hands: vec![],
            multiplier: 2.0,
        }),
        "heavy" => Some(StressTestConfig {
            concurrent_tasks: 16,
            duration_secs: 7200,     // 2 hours
            ramp_up_secs: 120,
            target_hands: vec![],
            multiplier: 3.0,
        }),
        _ => None,
    }
}

/// List available profile names
pub fn profile_names() -> Vec<&'static str> {
    vec!["smoke", "normal", "heavy"]
}

// ── Synthetic Task Generation ───────────────────────────────────────────────

/// Prompt variations to inject into hand prompts for synthetic diversity
const PROMPT_VARIATIONS: &[&str] = &[
    "Analyze the current market trends for AI-powered SaaS tools",
    "Research best practices for distributed computing in 2026",
    "Generate a report on emerging technology startups in Asia",
    "Find potential clients for automation consulting services",
    "Create content about machine learning pipeline optimization",
    "Investigate competitive landscape for developer productivity tools",
    "Summarize key findings about serverless architecture patterns",
    "Evaluate the ROI of implementing automated testing frameworks",
    "Draft a strategy for entering the European tech market",
    "Compile data on cloud infrastructure cost optimization",
    "Research sustainable AI practices and green computing",
    "Analyze customer acquisition strategies for B2B SaaS",
];

/// Generate a synthetic prompt for a given hand
fn generate_synthetic_prompt(hand: &Hand, variation_index: usize) -> String {
    let base = PROMPT_VARIATIONS[variation_index % PROMPT_VARIATIONS.len()];
    format!(
        "{} (load test variation #{} for hand '{}')",
        base, variation_index, hand.name
    )
}

// ── Internal Task Result ────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug)]
struct TaskResult {
    hand_name: String,
    success: bool,
    latency_ms: f64,
    error: Option<String>,
    completed_at: Instant,
}

// ── Shared Mutable State for Live Progress ──────────────────────────────────

#[derive(Debug, Default)]
struct LiveMetrics {
    completed: u64,
    failed: u64,
    active: u32,
    peak_concurrent: u32,
    latencies_ms: Vec<f64>,
    errors: HashMap<String, u64>,
    /// Per-second buckets: key = elapsed second, value = (completed, failed, latencies)
    second_buckets: HashMap<u64, (u32, u32, Vec<f64>)>,
}

impl LiveMetrics {
    fn record_start(&mut self) {
        self.active += 1;
        if self.active > self.peak_concurrent {
            self.peak_concurrent = self.active;
        }
    }

    fn record_completion(&mut self, result: &TaskResult, elapsed_secs: u64) {
        self.active = self.active.saturating_sub(1);
        self.latencies_ms.push(result.latency_ms);

        if result.success {
            self.completed += 1;
        } else {
            self.failed += 1;
            if let Some(ref err) = result.error {
                let key = classify_error(err);
                *self.errors.entry(key).or_insert(0) += 1;
            }
        }

        // Update per-second bucket
        let bucket = self.second_buckets.entry(elapsed_secs).or_insert((0, 0, Vec::new()));
        if result.success {
            bucket.0 += 1;
        } else {
            bucket.1 += 1;
        }
        bucket.2.push(result.latency_ms);
    }

    fn success_rate(&self) -> f64 {
        let total = self.completed + self.failed;
        if total == 0 { 0.0 } else { self.completed as f64 / total as f64 }
    }

    fn avg_latency_ms(&self) -> f64 {
        if self.latencies_ms.is_empty() {
            0.0
        } else {
            self.latencies_ms.iter().sum::<f64>() / self.latencies_ms.len() as f64
        }
    }
}

/// Classify error messages into categories for grouping
fn classify_error(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("timeout") || lower.contains("timed out") {
        "timeout".to_string()
    } else if lower.contains("rate limit") || lower.contains("429") || lower.contains("quota") {
        "rate_limited".to_string()
    } else if lower.contains("connection") || lower.contains("connect") {
        "connection_error".to_string()
    } else if lower.contains("not found") || lower.contains("404") {
        "not_found".to_string()
    } else if lower.contains("unauthorized") || lower.contains("401") || lower.contains("403") {
        "auth_error".to_string()
    } else if lower.contains("provider") {
        "provider_error".to_string()
    } else if lower.contains("budget") || lower.contains("breaker") {
        "budget_exceeded".to_string()
    } else {
        "other".to_string()
    }
}

// ── Percentile Calculator ───────────────────────────────────────────────────

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((pct / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    let idx = idx.min(sorted.len() - 1);
    sorted[idx]
}

// ── SQLite Result Store ─────────────────────────────────────────────────────

/// Store for persisting load test results to SQLite
pub struct LoadTestStore {
    db_path: String,
}

impl LoadTestStore {
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = rusqlite::Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS load_test_results (
                run_id TEXT PRIMARY KEY,
                test_type TEXT NOT NULL,
                started_at TEXT NOT NULL,
                ended_at TEXT NOT NULL,
                total_tasks INTEGER NOT NULL,
                successful INTEGER NOT NULL,
                failed INTEGER NOT NULL,
                success_rate REAL NOT NULL,
                avg_latency_ms REAL NOT NULL,
                p50_latency REAL NOT NULL,
                p95_latency REAL NOT NULL,
                p99_latency REAL NOT NULL,
                peak_concurrent INTEGER NOT NULL,
                config_json TEXT NOT NULL,
                report_json TEXT NOT NULL,
                profile TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_lt_type ON load_test_results(test_type);
            CREATE INDEX IF NOT EXISTS idx_lt_started ON load_test_results(started_at);"
        )?;
        Ok(Self { db_path: db_path.to_string() })
    }

    /// Save a stress test report
    pub fn save_stress_report(&self, report: &StressTestReport) -> Result<()> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let config_json = serde_json::to_string(&report.config)?;
        let report_json = serde_json::to_string(report)?;
        conn.execute(
            "INSERT OR REPLACE INTO load_test_results
             (run_id, test_type, started_at, ended_at, total_tasks, successful, failed,
              success_rate, avg_latency_ms, p50_latency, p95_latency, p99_latency,
              peak_concurrent, config_json, report_json, profile)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            rusqlite::params![
                report.run_id,
                "stress",
                report.started_at.to_rfc3339(),
                report.ended_at.to_rfc3339(),
                report.total_tasks,
                report.successful,
                report.failed,
                report.success_rate,
                report.avg_latency_ms,
                report.p50_latency,
                report.p95_latency,
                report.p99_latency,
                report.peak_concurrent,
                config_json,
                report_json,
                report.profile,
            ],
        )?;
        debug!("Saved stress test report: {}", report.run_id);
        Ok(())
    }

    /// Save an endurance test report
    pub fn save_endurance_report(&self, report: &EnduranceReport) -> Result<()> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let config_json = serde_json::to_string(&report.config)?;
        let report_json = serde_json::to_string(report)?;
        conn.execute(
            "INSERT OR REPLACE INTO load_test_results
             (run_id, test_type, started_at, ended_at, total_tasks, successful, failed,
              success_rate, avg_latency_ms, p50_latency, p95_latency, p99_latency,
              peak_concurrent, config_json, report_json, profile)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL)",
            rusqlite::params![
                report.run_id,
                "endurance",
                report.started_at.to_rfc3339(),
                report.ended_at.to_rfc3339(),
                report.total_tasks,
                report.successful,
                report.failed,
                report.success_rate,
                report.avg_latency_ms,
                report.p50_latency,
                report.p95_latency,
                report.p99_latency,
                report.peak_concurrent,
                config_json,
                report_json,
            ],
        )?;
        debug!("Saved endurance test report: {}", report.run_id);
        Ok(())
    }

    /// List recent test runs (newest first)
    pub fn recent(&self, limit: usize) -> Result<Vec<serde_json::Value>> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT run_id, test_type, started_at, total_tasks, successful, failed,
                    success_rate, avg_latency_ms, p95_latency, peak_concurrent, profile
             FROM load_test_results
             ORDER BY started_at DESC
             LIMIT ?1"
        )?;
        let rows = stmt.query_map([limit], |row| {
            Ok(serde_json::json!({
                "run_id": row.get::<_, String>(0)?,
                "test_type": row.get::<_, String>(1)?,
                "started_at": row.get::<_, String>(2)?,
                "total_tasks": row.get::<_, i64>(3)?,
                "successful": row.get::<_, i64>(4)?,
                "failed": row.get::<_, i64>(5)?,
                "success_rate": row.get::<_, f64>(6)?,
                "avg_latency_ms": row.get::<_, f64>(7)?,
                "p95_latency": row.get::<_, f64>(8)?,
                "peak_concurrent": row.get::<_, i64>(9)?,
                "profile": row.get::<_, Option<String>>(10)?,
            }))
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Get full report JSON by run_id
    pub fn get_report(&self, run_id: &str) -> Result<Option<serde_json::Value>> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT report_json FROM load_test_results WHERE run_id = ?1"
        )?;
        let result = stmt.query_row([run_id], |row| {
            let json_str: String = row.get(0)?;
            Ok(json_str)
        });
        match result {
            Ok(json_str) => {
                let parsed: serde_json::Value = serde_json::from_str(&json_str)?;
                Ok(Some(parsed))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

// ── Hand Executor Trait (for mocking in tests) ──────────────────────────────

/// Trait for executing hand workflows. Allows mocking in unit tests.
#[async_trait::async_trait]
pub trait HandExecutor: Send + Sync {
    async fn execute_hand(
        &self,
        hand: &Hand,
        prompt: &str,
    ) -> Result<crate::hands::HandResult>;
}

/// Real executor that uses the actual HandRunner
pub struct RealHandExecutor {
    pub runtime: Arc<AgentRuntime>,
    pub router: Arc<LlmRouter>,
    pub tool_registry: Arc<ToolRegistry>,
}

#[async_trait::async_trait]
impl HandExecutor for RealHandExecutor {
    async fn execute_hand(
        &self,
        hand: &Hand,
        prompt: &str,
    ) -> Result<crate::hands::HandResult> {
        HandRunner::run(
            hand,
            prompt,
            &self.runtime,
            &self.router,
            &self.tool_registry,
            None,  // No approval gate for load tests
        ).await
    }
}

// ── LoadTester ──────────────────────────────────────────────────────────────

/// Load testing engine for Phantom Mesh hand workflows
pub struct LoadTester {
    executor: Arc<dyn HandExecutor>,
    hands: Arc<HandRegistry>,
    store: Option<LoadTestStore>,
    status: Arc<RwLock<LoadTestStatus>>,
}

impl LoadTester {
    /// Create a new LoadTester with real hand execution
    pub fn new(
        runtime: Arc<AgentRuntime>,
        router: Arc<LlmRouter>,
        tool_registry: Arc<ToolRegistry>,
        hands: Arc<HandRegistry>,
        db_path: Option<&str>,
    ) -> Result<Self> {
        let executor = Arc::new(RealHandExecutor {
            runtime,
            router,
            tool_registry,
        });
        let store = match db_path {
            Some(path) => Some(LoadTestStore::new(path)?),
            None => None,
        };
        Ok(Self {
            executor,
            hands,
            store,
            status: Arc::new(RwLock::new(LoadTestStatus::default())),
        })
    }

    /// Create a LoadTester with a custom executor (for testing)
    pub fn with_executor(
        executor: Arc<dyn HandExecutor>,
        hands: Arc<HandRegistry>,
        store: Option<LoadTestStore>,
    ) -> Self {
        Self {
            executor,
            hands,
            store,
            status: Arc::new(RwLock::new(LoadTestStatus::default())),
        }
    }

    /// Get the current test status (for the /test/stress/status endpoint)
    pub async fn status(&self) -> LoadTestStatus {
        self.status.read().await.clone()
    }

    /// Resolve which hands to test based on config target list
    fn resolve_target_hands(&self, targets: &[String]) -> Vec<Hand> {
        if targets.is_empty() {
            // Test all available hands
            self.hands.list().into_iter().cloned().collect()
        } else {
            targets.iter()
                .filter_map(|name| self.hands.get(name).cloned())
                .collect()
        }
    }

    /// Run a stress test with the given configuration
    pub async fn run_stress_test(
        &self,
        config: StressTestConfig,
        profile_name: Option<String>,
    ) -> Result<StressTestReport> {
        let run_id = uuid::Uuid::new_v4().to_string();
        let started_at = Utc::now();
        let test_start = Instant::now();

        let hands = self.resolve_target_hands(&config.target_hands);
        if hands.is_empty() {
            return Err(anyhow!("No valid target hands found for stress test"));
        }

        info!(
            "Starting stress test '{}': {} concurrent, {}s duration, {}s ramp, {}x multiplier, {} hands",
            run_id, config.concurrent_tasks, config.duration_secs, config.ramp_up_secs,
            config.multiplier, hands.len()
        );

        // Update live status
        {
            let mut status = self.status.write().await;
            *status = LoadTestStatus {
                running: true,
                run_id: run_id.clone(),
                test_type: "stress".to_string(),
                elapsed_secs: 0,
                total_duration_secs: config.duration_secs,
                completed: 0,
                failed: 0,
                active: 0,
                success_rate: 0.0,
                avg_latency_ms: 0.0,
            };
        }

        let metrics = Arc::new(Mutex::new(LiveMetrics::default()));
        let semaphore = Arc::new(tokio::sync::Semaphore::new(1)); // Start with 1, will scale up
        let test_duration = Duration::from_secs(config.duration_secs);
        let ramp_duration = Duration::from_secs(config.ramp_up_secs);

        // Spawn the ramp-up controller that adjusts semaphore permits over time
        let sem_ramp = semaphore.clone();
        let max_concurrent = config.concurrent_tasks;
        let ramp_handle = tokio::spawn(async move {
            if ramp_duration.is_zero() {
                // No ramp-up: immediately add all permits
                sem_ramp.add_permits((max_concurrent - 1) as usize);
                return;
            }
            let steps = max_concurrent.max(1) - 1;
            if steps == 0 { return; }
            let step_interval = ramp_duration / steps;
            for _ in 0..steps {
                tokio::time::sleep(step_interval).await;
                sem_ramp.add_permits(1);
            }
        });

        // Task submission loop
        let mut task_handles = Vec::new();
        let mut variation_counter: usize = 0;

        // Calculate effective tasks per second based on multiplier
        let base_tps = config.concurrent_tasks as f64 / 10.0; // Base: fill concurrency over 10s
        let effective_tps = (base_tps * config.multiplier).max(0.1);
        let submit_interval = Duration::from_secs_f64(1.0 / effective_tps);

        loop {
            let elapsed = test_start.elapsed();
            if elapsed >= test_duration {
                break;
            }

            // Acquire semaphore permit (respects ramp-up)
            let permit = match tokio::time::timeout(
                Duration::from_millis(100),
                semaphore.clone().acquire_owned(),
            ).await {
                Ok(Ok(permit)) => permit,
                _ => {
                    // Semaphore full or timeout, just loop
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            };

            // Pick a hand (round-robin across target hands)
            let hand = hands[variation_counter % hands.len()].clone();
            let prompt = generate_synthetic_prompt(&hand, variation_counter);
            variation_counter += 1;

            let executor = self.executor.clone();
            let metrics_ref = metrics.clone();
            let status_ref = self.status.clone();
            let test_start_copy = test_start;

            // Record task start
            {
                let mut m = metrics_ref.lock().await;
                m.record_start();
            }

            // Update live status active count
            {
                let mut s = status_ref.write().await;
                s.active += 1;
            }

            let handle = tokio::spawn(async move {
                let task_start = Instant::now();
                let result = executor.execute_hand(&hand, &prompt).await;
                let latency_ms = task_start.elapsed().as_secs_f64() * 1000.0;
                let elapsed_secs = test_start_copy.elapsed().as_secs();

                let task_result = match result {
                    Ok(hr) => {
                        let is_error = hr.final_output.starts_with("Phase failed:");
                        TaskResult {
                            hand_name: hand.name.clone(),
                            success: !is_error,
                            latency_ms,
                            error: if is_error { Some(hr.final_output.clone()) } else { None },
                            completed_at: Instant::now(),
                        }
                    }
                    Err(e) => TaskResult {
                        hand_name: hand.name.clone(),
                        success: false,
                        latency_ms,
                        error: Some(e.to_string()),
                        completed_at: Instant::now(),
                    },
                };

                // Record completion
                {
                    let mut m = metrics_ref.lock().await;
                    m.record_completion(&task_result, elapsed_secs);
                }

                // Update live status
                {
                    let mut s = status_ref.write().await;
                    s.active = s.active.saturating_sub(1);
                    if task_result.success {
                        s.completed += 1;
                    } else {
                        s.failed += 1;
                    }
                    let total = s.completed + s.failed;
                    s.success_rate = if total == 0 { 0.0 } else { s.completed as f64 / total as f64 };
                    s.elapsed_secs = test_start_copy.elapsed().as_secs();
                }

                // Release permit
                drop(permit);
                task_result
            });
            task_handles.push(handle);

            // Throttle submission rate
            tokio::time::sleep(submit_interval).await;
        }

        // Wait for ramp-up to finish
        let _ = ramp_handle.await;

        // Wait for all in-flight tasks to complete (with a generous timeout)
        let wait_timeout = Duration::from_secs(config.duration_secs.max(300));
        let _ = tokio::time::timeout(wait_timeout, async {
            for handle in task_handles {
                let _ = handle.await;
            }
        }).await;

        // Build the report
        let ended_at = Utc::now();
        let final_metrics = metrics.lock().await;

        let mut sorted_latencies = final_metrics.latencies_ms.clone();
        sorted_latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        // Build timeline
        let max_sec = if final_metrics.second_buckets.is_empty() {
            0
        } else {
            *final_metrics.second_buckets.keys().max().unwrap_or(&0)
        };
        let mut timeline = Vec::new();
        for sec in 0..=max_sec {
            let (completed, failed, ref lats) = final_metrics.second_buckets
                .get(&sec)
                .cloned()
                .unwrap_or((0, 0, Vec::new()));
            let avg_lat = if lats.is_empty() {
                0.0
            } else {
                lats.iter().sum::<f64>() / lats.len() as f64
            };
            timeline.push(TimelinePoint {
                elapsed_secs: sec,
                active_tasks: 0, // Not tracked per-second
                completed,
                failed,
                avg_latency_ms: avg_lat,
            });
        }

        let report = StressTestReport {
            run_id: run_id.clone(),
            started_at,
            ended_at,
            total_tasks: final_metrics.completed + final_metrics.failed,
            successful: final_metrics.completed,
            failed: final_metrics.failed,
            success_rate: final_metrics.success_rate(),
            avg_latency_ms: final_metrics.avg_latency_ms(),
            p50_latency: percentile(&sorted_latencies, 50.0),
            p95_latency: percentile(&sorted_latencies, 95.0),
            p99_latency: percentile(&sorted_latencies, 99.0),
            errors_by_type: final_metrics.errors.clone(),
            peak_concurrent: final_metrics.peak_concurrent,
            timeline,
            config: config.clone(),
            profile: profile_name,
        };

        // Save to SQLite
        if let Some(ref store) = self.store {
            if let Err(e) = store.save_stress_report(&report) {
                warn!("Failed to save stress test report: {}", e);
            }
        }

        // Reset live status
        {
            let mut status = self.status.write().await;
            *status = LoadTestStatus::default();
        }

        info!(
            "Stress test '{}' completed: {}/{} tasks, {:.1}% success, avg {:.0}ms, p95 {:.0}ms",
            run_id, report.successful, report.total_tasks,
            report.success_rate * 100.0, report.avg_latency_ms, report.p95_latency
        );

        Ok(report)
    }

    /// Run an endurance test with sustained steady load over hours
    pub async fn run_endurance_test(&self, config: EnduranceConfig) -> Result<EnduranceReport> {
        let run_id = uuid::Uuid::new_v4().to_string();
        let started_at = Utc::now();
        let test_start = Instant::now();

        let hands = self.resolve_target_hands(&config.target_hands);
        if hands.is_empty() {
            return Err(anyhow!("No valid target hands found for endurance test"));
        }

        let total_duration = Duration::from_secs(config.duration_hours * 3600);
        let tasks_per_minute = config.steady_load;
        let submit_interval = if tasks_per_minute > 0 {
            Duration::from_secs_f64(60.0 / tasks_per_minute as f64)
        } else {
            Duration::from_secs(60)
        };

        info!(
            "Starting endurance test '{}': {} tasks/min, {}h duration, {} hands",
            run_id, tasks_per_minute, config.duration_hours, hands.len()
        );

        // Update live status
        {
            let mut status = self.status.write().await;
            *status = LoadTestStatus {
                running: true,
                run_id: run_id.clone(),
                test_type: "endurance".to_string(),
                elapsed_secs: 0,
                total_duration_secs: config.duration_hours * 3600,
                completed: 0,
                failed: 0,
                active: 0,
                success_rate: 0.0,
                avg_latency_ms: 0.0,
            };
        }

        let metrics = Arc::new(Mutex::new(LiveMetrics::default()));
        let mut task_handles = Vec::new();
        let mut variation_counter: usize = 0;
        // Use a reasonable concurrency limit for endurance
        let max_concurrent = (tasks_per_minute as usize * 5).max(4);
        let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));

        loop {
            let elapsed = test_start.elapsed();
            if elapsed >= total_duration {
                break;
            }

            // Acquire semaphore to limit max in-flight
            let permit = match tokio::time::timeout(
                Duration::from_millis(500),
                semaphore.clone().acquire_owned(),
            ).await {
                Ok(Ok(p)) => p,
                _ => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };

            let hand = hands[variation_counter % hands.len()].clone();
            let prompt = generate_synthetic_prompt(&hand, variation_counter);
            variation_counter += 1;

            let executor = self.executor.clone();
            let metrics_ref = metrics.clone();
            let status_ref = self.status.clone();
            let test_start_copy = test_start;

            {
                let mut m = metrics_ref.lock().await;
                m.record_start();
            }
            {
                let mut s = status_ref.write().await;
                s.active += 1;
            }

            let handle = tokio::spawn(async move {
                let task_start = Instant::now();
                let result = executor.execute_hand(&hand, &prompt).await;
                let latency_ms = task_start.elapsed().as_secs_f64() * 1000.0;
                let elapsed_secs = test_start_copy.elapsed().as_secs();
                // Use minute-granularity bucket for endurance (key = elapsed_minute * 60)
                let elapsed_minute_key = (elapsed_secs / 60) * 60;

                let task_result = match result {
                    Ok(hr) => {
                        let is_error = hr.final_output.starts_with("Phase failed:");
                        TaskResult {
                            hand_name: hand.name.clone(),
                            success: !is_error,
                            latency_ms,
                            error: if is_error { Some(hr.final_output.clone()) } else { None },
                            completed_at: Instant::now(),
                        }
                    }
                    Err(e) => TaskResult {
                        hand_name: hand.name.clone(),
                        success: false,
                        latency_ms,
                        error: Some(e.to_string()),
                        completed_at: Instant::now(),
                    },
                };

                {
                    let mut m = metrics_ref.lock().await;
                    m.record_completion(&task_result, elapsed_minute_key);
                }
                {
                    let mut s = status_ref.write().await;
                    s.active = s.active.saturating_sub(1);
                    if task_result.success { s.completed += 1; } else { s.failed += 1; }
                    let total = s.completed + s.failed;
                    s.success_rate = if total == 0 { 0.0 } else { s.completed as f64 / total as f64 };
                    s.elapsed_secs = test_start_copy.elapsed().as_secs();
                    let m = metrics_ref.lock().await;
                    s.avg_latency_ms = m.avg_latency_ms();
                }

                drop(permit);
                task_result
            });
            task_handles.push(handle);

            // Steady submission rate
            tokio::time::sleep(submit_interval).await;
        }

        // Wait for in-flight tasks
        let wait_timeout = Duration::from_secs(600);
        let _ = tokio::time::timeout(wait_timeout, async {
            for handle in task_handles {
                let _ = handle.await;
            }
        }).await;

        let ended_at = Utc::now();
        let final_metrics = metrics.lock().await;

        let mut sorted_latencies = final_metrics.latencies_ms.clone();
        sorted_latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        // Build per-minute timeline
        let mut minute_keys: Vec<u64> = final_metrics.second_buckets.keys().cloned().collect();
        minute_keys.sort();
        let timeline_minutes: Vec<TimelinePoint> = minute_keys.iter().map(|&minute_key| {
            let (completed, failed, ref lats) = final_metrics.second_buckets
                .get(&minute_key)
                .cloned()
                .unwrap_or((0, 0, Vec::new()));
            let avg_lat = if lats.is_empty() { 0.0 } else { lats.iter().sum::<f64>() / lats.len() as f64 };
            TimelinePoint {
                elapsed_secs: minute_key, // Actually in minutes * 60
                active_tasks: 0,
                completed,
                failed,
                avg_latency_ms: avg_lat,
            }
        }).collect();

        let report = EnduranceReport {
            run_id: run_id.clone(),
            started_at,
            ended_at,
            total_tasks: final_metrics.completed + final_metrics.failed,
            successful: final_metrics.completed,
            failed: final_metrics.failed,
            success_rate: final_metrics.success_rate(),
            avg_latency_ms: final_metrics.avg_latency_ms(),
            p50_latency: percentile(&sorted_latencies, 50.0),
            p95_latency: percentile(&sorted_latencies, 95.0),
            p99_latency: percentile(&sorted_latencies, 99.0),
            errors_by_type: final_metrics.errors.clone(),
            peak_concurrent: final_metrics.peak_concurrent,
            timeline_minutes,
            config: config.clone(),
        };

        // Save to SQLite
        if let Some(ref store) = self.store {
            if let Err(e) = store.save_endurance_report(&report) {
                warn!("Failed to save endurance test report: {}", e);
            }
        }

        // Reset live status
        {
            let mut status = self.status.write().await;
            *status = LoadTestStatus::default();
        }

        info!(
            "Endurance test '{}' completed: {}/{} tasks over {}h, {:.1}% success",
            run_id, report.successful, report.total_tasks,
            config.duration_hours, report.success_rate * 100.0
        );

        Ok(report)
    }

    /// Get the result store (if available) for querying historical results
    pub fn store(&self) -> Option<&LoadTestStore> {
        self.store.as_ref()
    }
}

// ── Unit Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hands::{Hand, Phase, HandResult, PhaseOutput};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Mock executor that returns instant results (no LLM calls)
    struct MockHandExecutor {
        /// Delay per task in milliseconds
        delay_ms: u64,
        /// Counter for total executions
        execution_count: AtomicU64,
        /// If true, every 3rd call fails
        fail_pattern: bool,
    }

    impl MockHandExecutor {
        fn new(delay_ms: u64, fail_pattern: bool) -> Self {
            Self {
                delay_ms,
                execution_count: AtomicU64::new(0),
                fail_pattern,
            }
        }
    }

    #[async_trait::async_trait]
    impl HandExecutor for MockHandExecutor {
        async fn execute_hand(
            &self,
            hand: &Hand,
            _prompt: &str,
        ) -> Result<HandResult> {
            let count = self.execution_count.fetch_add(1, Ordering::SeqCst);

            // Simulate processing time
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }

            // Fail every 3rd call if fail_pattern is enabled
            if self.fail_pattern && count % 3 == 2 {
                return Err(anyhow!("Simulated provider timeout error"));
            }

            Ok(HandResult {
                hand_name: hand.name.clone(),
                phases_completed: hand.phases.len(),
                total_phases: hand.phases.len(),
                outputs: vec![PhaseOutput {
                    phase_name: "mock_phase".to_string(),
                    output: format!("Mock output for execution #{}", count),
                    tool_calls: 0,
                    duration_secs: self.delay_ms as f64 / 1000.0,
                    skipped: false,
                    guardrail_issues: Vec::new(),
                    quality_score: None,
                    quality_retries: 0,
                }],
                final_output: format!("Mock output for execution #{}", count),
                elapsed_secs: self.delay_ms as f64 / 1000.0,
                chain_to: None,
            })
        }
    }

    fn test_hand(name: &str) -> Hand {
        Hand {
            name: name.to_string(),
            description: format!("Test hand: {}", name),
            category: "test".to_string(),
            provider: "auto".to_string(),
            model: String::new(),
            phases: vec![
                Phase {
                    name: "test_phase".to_string(),
                    system_prompt: "Test prompt".to_string(),
                    max_rounds: 1,
                    condition: None,
                    target_worker: None,
                    target_capability: None,
                    parallel_queries: Vec::new(),
                    tools: None,
                    provider: None,
                    model: None,
                    extra: HashMap::new(),
                },
            ],
            tools: None,
            output_format: "markdown".to_string(),
            schedule: None,
            settings: HashMap::new(),
            chain_to: None,
            guardrail: None,
            eval: None,
            extra: HashMap::new(),
        }
    }

    fn test_registry(hands: Vec<Hand>) -> HandRegistry {
        let mut map = HashMap::new();
        for h in hands {
            map.insert(h.name.clone(), h);
        }
        // Use HandRegistry::empty and manually insert
        // (HandRegistry fields are private, so we construct via load with a temp dir)
        let dir = tempfile::tempdir().unwrap();
        for (name, hand) in &map {
            let hand_dir = dir.path().join(name);
            std::fs::create_dir_all(&hand_dir).unwrap();
            let toml_str = toml::to_string(hand).unwrap();
            std::fs::write(hand_dir.join("hand.toml"), toml_str).unwrap();
        }
        HandRegistry::load(dir.path().to_str().unwrap()).unwrap()
    }

    // ── Profile Tests ───────────────────────────────────────────────────────

    #[test]
    fn test_profile_smoke() {
        let p = profile("smoke").unwrap();
        assert_eq!(p.concurrent_tasks, 2);
        assert_eq!(p.duration_secs, 300);
        assert_eq!(p.multiplier, 1.0);
    }

    #[test]
    fn test_profile_normal() {
        let p = profile("normal").unwrap();
        assert_eq!(p.concurrent_tasks, 8);
        assert_eq!(p.duration_secs, 1800);
        assert_eq!(p.multiplier, 2.0);
    }

    #[test]
    fn test_profile_heavy() {
        let p = profile("heavy").unwrap();
        assert_eq!(p.concurrent_tasks, 16);
        assert_eq!(p.duration_secs, 7200);
        assert_eq!(p.multiplier, 3.0);
    }

    #[test]
    fn test_profile_unknown() {
        assert!(profile("nonexistent").is_none());
    }

    #[test]
    fn test_profile_names() {
        let names = profile_names();
        assert_eq!(names, vec!["smoke", "normal", "heavy"]);
    }

    // ── Percentile Tests ────────────────────────────────────────────────────

    #[test]
    fn test_percentile_empty() {
        assert_eq!(percentile(&[], 50.0), 0.0);
    }

    #[test]
    fn test_percentile_single() {
        assert_eq!(percentile(&[100.0], 50.0), 100.0);
        assert_eq!(percentile(&[100.0], 99.0), 100.0);
    }

    #[test]
    fn test_percentile_sorted() {
        let data: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        // p50 of 1..=100: index = round(0.5 * 99) = 50, data[50] = 51
        assert_eq!(percentile(&data, 50.0), 51.0);
        // p95: index = round(0.95 * 99) = 94, data[94] = 95
        assert_eq!(percentile(&data, 95.0), 95.0);
        // p99: index = round(0.99 * 99) = 98, data[98] = 99
        assert_eq!(percentile(&data, 99.0), 99.0);
    }

    // ── Error Classification Tests ──────────────────────────────────────────

    #[test]
    fn test_classify_error_timeout() {
        assert_eq!(classify_error("Request timed out after 30s"), "timeout");
        assert_eq!(classify_error("Connection timeout"), "timeout");
    }

    #[test]
    fn test_classify_error_rate_limit() {
        assert_eq!(classify_error("429 Too Many Requests"), "rate_limited");
        assert_eq!(classify_error("Rate limit exceeded"), "rate_limited");
        assert_eq!(classify_error("Quota exhausted"), "rate_limited");
    }

    #[test]
    fn test_classify_error_connection() {
        assert_eq!(classify_error("Connection refused"), "connection_error");
    }

    #[test]
    fn test_classify_error_auth() {
        assert_eq!(classify_error("401 Unauthorized"), "auth_error");
        assert_eq!(classify_error("403 Forbidden"), "auth_error");
    }

    #[test]
    fn test_classify_error_budget() {
        assert_eq!(classify_error("Budget breaker tripped"), "budget_exceeded");
    }

    #[test]
    fn test_classify_error_other() {
        assert_eq!(classify_error("Some unknown error"), "other");
    }

    // ── Synthetic Prompt Tests ──────────────────────────────────────────────

    #[test]
    fn test_generate_synthetic_prompt() {
        let hand = test_hand("content");
        let prompt = generate_synthetic_prompt(&hand, 0);
        assert!(prompt.contains("content"));
        assert!(prompt.contains("load test variation #0"));

        // Different variation index produces different prompt
        let prompt2 = generate_synthetic_prompt(&hand, 1);
        assert_ne!(prompt, prompt2);
    }

    #[test]
    fn test_synthetic_prompt_wraps_around() {
        let hand = test_hand("test");
        let prompt_a = generate_synthetic_prompt(&hand, 0);
        let prompt_b = generate_synthetic_prompt(&hand, PROMPT_VARIATIONS.len());
        // Same base prompt but different variation number
        assert!(prompt_a.contains("variation #0"));
        assert!(prompt_b.contains(&format!("variation #{}", PROMPT_VARIATIONS.len())));
    }

    // ── LiveMetrics Tests ───────────────────────────────────────────────────

    #[test]
    fn test_live_metrics_tracking() {
        let mut metrics = LiveMetrics::default();

        // Start 3 tasks
        metrics.record_start();
        metrics.record_start();
        metrics.record_start();
        assert_eq!(metrics.active, 3);
        assert_eq!(metrics.peak_concurrent, 3);

        // Complete 2 successfully
        let success = TaskResult {
            hand_name: "test".to_string(),
            success: true,
            latency_ms: 100.0,
            error: None,
            completed_at: Instant::now(),
        };
        metrics.record_completion(&success, 1);
        metrics.record_completion(&success, 1);
        assert_eq!(metrics.active, 1);
        assert_eq!(metrics.completed, 2);
        assert_eq!(metrics.failed, 0);

        // Complete 1 with failure
        let failure = TaskResult {
            hand_name: "test".to_string(),
            success: false,
            latency_ms: 500.0,
            error: Some("Connection timeout".to_string()),
            completed_at: Instant::now(),
        };
        metrics.record_completion(&failure, 2);
        assert_eq!(metrics.active, 0);
        assert_eq!(metrics.completed, 2);
        assert_eq!(metrics.failed, 1);
        assert_eq!(metrics.peak_concurrent, 3);

        // Success rate: 2/3
        let rate = metrics.success_rate();
        assert!((rate - 0.6667).abs() < 0.01);

        // Average latency: (100 + 100 + 500) / 3 = 233.33
        let avg = metrics.avg_latency_ms();
        assert!((avg - 233.33).abs() < 1.0);

        // Error classification
        assert_eq!(*metrics.errors.get("timeout").unwrap(), 1);
    }

    #[test]
    fn test_live_metrics_empty() {
        let metrics = LiveMetrics::default();
        assert_eq!(metrics.success_rate(), 0.0);
        assert_eq!(metrics.avg_latency_ms(), 0.0);
    }

    // ── LoadTestStore Tests ─────────────────────────────────────────────────

    #[test]
    fn test_store_create() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_lt.db");
        let store = LoadTestStore::new(db_path.to_str().unwrap()).unwrap();
        let recent = store.recent(10).unwrap();
        assert!(recent.is_empty());
    }

    #[test]
    fn test_store_save_and_query_stress() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_lt.db");
        let store = LoadTestStore::new(db_path.to_str().unwrap()).unwrap();

        let report = StressTestReport {
            run_id: "test-run-1".to_string(),
            started_at: Utc::now(),
            ended_at: Utc::now(),
            total_tasks: 100,
            successful: 95,
            failed: 5,
            success_rate: 0.95,
            avg_latency_ms: 250.0,
            p50_latency: 200.0,
            p95_latency: 800.0,
            p99_latency: 1200.0,
            errors_by_type: {
                let mut m = HashMap::new();
                m.insert("timeout".to_string(), 3);
                m.insert("rate_limited".to_string(), 2);
                m
            },
            peak_concurrent: 8,
            timeline: vec![],
            config: StressTestConfig::default(),
            profile: Some("smoke".to_string()),
        };

        store.save_stress_report(&report).unwrap();

        let recent = store.recent(10).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0]["run_id"], "test-run-1");
        assert_eq!(recent[0]["total_tasks"], 100);
        assert_eq!(recent[0]["profile"], "smoke");

        // Retrieve full report
        let full = store.get_report("test-run-1").unwrap().unwrap();
        assert_eq!(full["successful"], 95);
    }

    #[test]
    fn test_store_save_and_query_endurance() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_lt.db");
        let store = LoadTestStore::new(db_path.to_str().unwrap()).unwrap();

        let report = EnduranceReport {
            run_id: "endurance-1".to_string(),
            started_at: Utc::now(),
            ended_at: Utc::now(),
            total_tasks: 500,
            successful: 490,
            failed: 10,
            success_rate: 0.98,
            avg_latency_ms: 300.0,
            p50_latency: 250.0,
            p95_latency: 900.0,
            p99_latency: 1500.0,
            errors_by_type: HashMap::new(),
            peak_concurrent: 4,
            timeline_minutes: vec![],
            config: EnduranceConfig::default(),
        };

        store.save_endurance_report(&report).unwrap();

        let recent = store.recent(10).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0]["test_type"], "endurance");
    }

    #[test]
    fn test_store_get_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_lt.db");
        let store = LoadTestStore::new(db_path.to_str().unwrap()).unwrap();
        let result = store.get_report("nonexistent").unwrap();
        assert!(result.is_none());
    }

    // ── Stress Test (mock executor) ─────────────────────────────────────────

    #[tokio::test]
    async fn test_stress_test_with_mock_all_success() {
        let executor = Arc::new(MockHandExecutor::new(10, false));
        let registry = test_registry(vec![test_hand("mock_hand")]);

        let tester = LoadTester::with_executor(
            executor.clone(),
            Arc::new(registry),
            None,
        );

        let config = StressTestConfig {
            concurrent_tasks: 2,
            duration_secs: 2,
            ramp_up_secs: 0,
            target_hands: vec!["mock_hand".to_string()],
            multiplier: 1.0,
        };

        let report = tester.run_stress_test(config, Some("test".to_string())).await.unwrap();

        assert!(report.total_tasks > 0, "Should have completed some tasks");
        assert_eq!(report.failed, 0, "No failures with all-success mock");
        assert_eq!(report.success_rate, 1.0);
        assert!(report.avg_latency_ms >= 10.0, "Latency should be at least 10ms (mock delay)");
        assert_eq!(report.profile, Some("test".to_string()));
    }

    #[tokio::test]
    async fn test_stress_test_with_failures() {
        let executor = Arc::new(MockHandExecutor::new(1, true)); // Every 3rd call fails, 1ms delay
        let registry = test_registry(vec![test_hand("fail_hand")]);

        let tester = LoadTester::with_executor(
            executor.clone(),
            Arc::new(registry),
            None,
        );

        let config = StressTestConfig {
            concurrent_tasks: 4,
            duration_secs: 3,
            ramp_up_secs: 0,
            target_hands: vec!["fail_hand".to_string()],
            multiplier: 5.0, // High multiplier to submit many tasks quickly
        };

        let report = tester.run_stress_test(config, None).await.unwrap();

        assert!(report.total_tasks >= 3, "Should have at least 3 tasks for fail pattern, got {}", report.total_tasks);
        assert!(report.failed > 0, "Should have some failures (total: {}, failed: {})", report.total_tasks, report.failed);
        assert!(report.success_rate < 1.0, "Success rate should be below 100%");
        assert!(!report.errors_by_type.is_empty(), "Should have error classifications");
        // The mock returns "timeout" errors
        assert!(report.errors_by_type.contains_key("timeout"));
    }

    #[tokio::test]
    async fn test_stress_test_no_hands() {
        let executor = Arc::new(MockHandExecutor::new(1, false));
        let registry = test_registry(vec![]);

        let tester = LoadTester::with_executor(
            executor,
            Arc::new(registry),
            None,
        );

        let config = StressTestConfig {
            concurrent_tasks: 1,
            duration_secs: 1,
            ramp_up_secs: 0,
            target_hands: vec!["nonexistent".to_string()],
            multiplier: 1.0,
        };

        let result = tester.run_stress_test(config, None).await;
        assert!(result.is_err(), "Should fail with no valid hands");
    }

    #[tokio::test]
    async fn test_status_updates_during_test() {
        let executor = Arc::new(MockHandExecutor::new(100, false));
        let registry = test_registry(vec![test_hand("status_hand")]);

        let tester = Arc::new(LoadTester::with_executor(
            executor,
            Arc::new(registry),
            None,
        ));

        // Check initial status
        let status = tester.status().await;
        assert!(!status.running);

        // Start test in background
        let tester_clone = tester.clone();
        let handle = tokio::spawn(async move {
            let config = StressTestConfig {
                concurrent_tasks: 2,
                duration_secs: 2,
                ramp_up_secs: 0,
                target_hands: vec!["status_hand".to_string()],
                multiplier: 1.0,
            };
            tester_clone.run_stress_test(config, None).await
        });

        // Give it time to start
        tokio::time::sleep(Duration::from_millis(200)).await;

        let status = tester.status().await;
        // Should be running (might have finished very quickly though)
        // This is a best-effort check for the live status mechanism
        if status.running {
            assert_eq!(status.test_type, "stress");
            assert!(!status.run_id.is_empty());
        }

        let _ = handle.await;

        // After completion, should not be running
        let status = tester.status().await;
        assert!(!status.running);
    }

    // ── Endurance Test (mock executor) ───────────────────────────────────────

    #[tokio::test]
    async fn test_endurance_test_short() {
        let executor = Arc::new(MockHandExecutor::new(5, false));
        let registry = test_registry(vec![test_hand("endurance_hand")]);

        let tester = LoadTester::with_executor(
            executor,
            Arc::new(registry),
            None,
        );

        // Very short endurance test: 1 task/min for a fraction of an hour
        // We'll fake a short duration by using 1 second via manual approach
        // Actually, the endurance config uses hours. For testing, we need to
        // do something creative. We'll set duration_hours = 0 which gives 0 duration.
        // Instead, let's test that it completes quickly when duration is 0.

        // Actually: duration_hours=1 would be 3600 seconds — too long for test.
        // But the loop checks elapsed >= total_duration. If duration_hours=0, total_duration=0,
        // the loop exits immediately. That's a valid edge case test.
        let config = EnduranceConfig {
            steady_load: 10,
            duration_hours: 0, // 0 hours = immediate exit
            target_hands: vec!["endurance_hand".to_string()],
        };

        let report = tester.run_endurance_test(config).await.unwrap();
        // With 0 duration, the loop exits immediately, so no tasks are submitted
        assert_eq!(report.total_tasks, 0);
    }

    // ── SQLite Store Integration Test ───────────────────────────────────────

    #[tokio::test]
    async fn test_stress_test_with_store() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("lt_store.db");

        let executor = Arc::new(MockHandExecutor::new(5, false));
        let registry = test_registry(vec![test_hand("stored_hand")]);
        let store = LoadTestStore::new(db_path.to_str().unwrap()).unwrap();

        let tester = LoadTester::with_executor(
            executor,
            Arc::new(registry),
            Some(store),
        );

        let config = StressTestConfig {
            concurrent_tasks: 2,
            duration_secs: 1,
            ramp_up_secs: 0,
            target_hands: vec!["stored_hand".to_string()],
            multiplier: 1.0,
        };

        let report = tester.run_stress_test(config, Some("smoke".to_string())).await.unwrap();

        // Check that the result was persisted
        let stored = tester.store().unwrap();
        let recent = stored.recent(10).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0]["run_id"].as_str().unwrap(), report.run_id);
    }

    // ── Config Defaults Test ────────────────────────────────────────────────

    #[test]
    fn test_stress_config_default() {
        let config = StressTestConfig::default();
        assert_eq!(config.concurrent_tasks, 4);
        assert_eq!(config.duration_secs, 300);
        assert_eq!(config.ramp_up_secs, 30);
        assert!(config.target_hands.is_empty());
        assert_eq!(config.multiplier, 1.0);
    }

    #[test]
    fn test_endurance_config_default() {
        let config = EnduranceConfig::default();
        assert_eq!(config.steady_load, 2);
        assert_eq!(config.duration_hours, 1);
        assert!(config.target_hands.is_empty());
    }

    // ── Serialization Tests ─────────────────────────────────────────────────

    #[test]
    fn test_stress_config_serialization() {
        let config = StressTestConfig {
            concurrent_tasks: 10,
            duration_secs: 600,
            ramp_up_secs: 60,
            target_hands: vec!["content".to_string(), "lead".to_string()],
            multiplier: 2.5,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: StressTestConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.concurrent_tasks, 10);
        assert_eq!(parsed.multiplier, 2.5);
        assert_eq!(parsed.target_hands.len(), 2);
    }

    #[test]
    fn test_stress_report_serialization() {
        let report = StressTestReport {
            run_id: "test-123".to_string(),
            started_at: Utc::now(),
            ended_at: Utc::now(),
            total_tasks: 50,
            successful: 45,
            failed: 5,
            success_rate: 0.9,
            avg_latency_ms: 200.0,
            p50_latency: 150.0,
            p95_latency: 600.0,
            p99_latency: 900.0,
            errors_by_type: HashMap::new(),
            peak_concurrent: 4,
            timeline: vec![TimelinePoint {
                elapsed_secs: 1,
                active_tasks: 2,
                completed: 3,
                failed: 0,
                avg_latency_ms: 180.0,
            }],
            config: StressTestConfig::default(),
            profile: None,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("test-123"));
        let parsed: StressTestReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.total_tasks, 50);
        assert_eq!(parsed.timeline.len(), 1);
    }

    #[test]
    fn test_load_test_status_default() {
        let status = LoadTestStatus::default();
        assert!(!status.running);
        assert!(status.run_id.is_empty());
        assert_eq!(status.completed, 0);
        assert_eq!(status.active, 0);
    }
}
