//! Comprehensive Health Check System
//!
//! Provides a unified health-check framework with built-in checks for databases,
//! disk space, memory usage, providers, cluster connectivity, cron scheduler,
//! rate limits, pending approvals, error rate, and cost budgets.
//!
//! Each check returns a `ComponentHealth` and the overall `SystemHealth` is
//! derived from the worst component status.
//!
//! Supports custom check registration, JSON serialization, and Prometheus text
//! exposition format.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Overall health status of a component or the entire system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    /// All subsystems operating normally.
    Healthy,
    /// One or more subsystems are impaired but the system is still functional.
    Degraded,
    /// Critical failure -- the system cannot serve requests reliably.
    Unhealthy,
}

impl HealthStatus {
    /// Numeric severity used for worst-of comparisons. Higher is worse.
    fn severity(self) -> u8 {
        match self {
            HealthStatus::Healthy => 0,
            HealthStatus::Degraded => 1,
            HealthStatus::Unhealthy => 2,
        }
    }

    /// Return the more severe of two statuses.
    pub fn worse(self, other: HealthStatus) -> HealthStatus {
        if other.severity() > self.severity() {
            other
        } else {
            self
        }
    }
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HealthStatus::Healthy => write!(f, "healthy"),
            HealthStatus::Degraded => write!(f, "degraded"),
            HealthStatus::Unhealthy => write!(f, "unhealthy"),
        }
    }
}

/// Health information for a single component / subsystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHealth {
    /// Human-readable component name (e.g. "database", "disk_space").
    pub name: String,
    /// Current health status.
    pub status: HealthStatus,
    /// Short descriptive message explaining the status.
    pub message: String,
    /// Timestamp of the last completed check.
    pub last_check: DateTime<Utc>,
    /// Latency of the check itself in milliseconds.
    pub latency_ms: u64,
    /// Arbitrary key-value metadata (e.g. free_bytes, error_count).
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl ComponentHealth {
    /// Convenience constructor for a healthy component.
    pub fn healthy(name: &str, message: &str) -> Self {
        Self {
            name: name.to_string(),
            status: HealthStatus::Healthy,
            message: message.to_string(),
            last_check: Utc::now(),
            latency_ms: 0,
            metadata: HashMap::new(),
        }
    }

    /// Convenience constructor for a degraded component.
    pub fn degraded(name: &str, message: &str) -> Self {
        Self {
            name: name.to_string(),
            status: HealthStatus::Degraded,
            message: message.to_string(),
            last_check: Utc::now(),
            latency_ms: 0,
            metadata: HashMap::new(),
        }
    }

    /// Convenience constructor for an unhealthy component.
    pub fn unhealthy(name: &str, message: &str) -> Self {
        Self {
            name: name.to_string(),
            status: HealthStatus::Unhealthy,
            message: message.to_string(),
            last_check: Utc::now(),
            latency_ms: 0,
            metadata: HashMap::new(),
        }
    }

    /// Add a metadata key-value pair (builder style).
    pub fn with_meta(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }

    /// Set latency_ms (builder style).
    pub fn with_latency(mut self, ms: u64) -> Self {
        self.latency_ms = ms;
        self
    }
}

/// Aggregated health of the entire system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemHealth {
    /// Overall status (worst of all components).
    pub status: HealthStatus,
    /// Per-component health details.
    pub components: Vec<ComponentHealth>,
    /// System uptime in seconds.
    pub uptime_secs: u64,
    /// Version string (e.g. crate version).
    pub version: String,
    /// Timestamp when the system-wide check completed.
    pub checked_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Built-in check functions
// ---------------------------------------------------------------------------

/// Check that SQLite database files under `~/.clawtex/` are accessible.
pub fn check_database(db_dir: &str) -> ComponentHealth {
    let start = Instant::now();
    let db_files = ["core.db", "costs.db", "memory.db"];
    let mut missing: Vec<String> = Vec::new();
    let mut found = 0u32;

    for name in &db_files {
        let path = format!("{}/{}", db_dir, name);
        if std::path::Path::new(&path).exists() {
            found += 1;
        } else {
            missing.push(name.to_string());
        }
    }

    let elapsed = start.elapsed().as_millis() as u64;

    if missing.is_empty() {
        ComponentHealth::healthy("database", &format!("{} databases accessible", found))
            .with_latency(elapsed)
            .with_meta("found", &found.to_string())
    } else if found > 0 {
        ComponentHealth::degraded(
            "database",
            &format!("Missing databases: {}", missing.join(", ")),
        )
        .with_latency(elapsed)
        .with_meta("found", &found.to_string())
        .with_meta("missing", &missing.join(","))
    } else {
        ComponentHealth::unhealthy("database", "No databases found")
            .with_latency(elapsed)
            .with_meta("missing", &missing.join(","))
    }
}

/// Minimum free disk space threshold in bytes (100 MB).
const MIN_FREE_DISK_BYTES: u64 = 100 * 1024 * 1024;

/// Check that the workspace directory has more than 100 MB free disk space.
pub fn check_disk_space(workspace_dir: &str) -> ComponentHealth {
    let start = Instant::now();

    // Use sysinfo to get available disk space
    let path = std::path::Path::new(workspace_dir);
    let free_bytes = disk_free_bytes(path);
    let elapsed = start.elapsed().as_millis() as u64;

    let free_mb = free_bytes / (1024 * 1024);

    if free_bytes >= MIN_FREE_DISK_BYTES * 10 {
        // > 1 GB free
        ComponentHealth::healthy("disk_space", &format!("{} MB free", free_mb))
            .with_latency(elapsed)
            .with_meta("free_bytes", &free_bytes.to_string())
            .with_meta("free_mb", &free_mb.to_string())
    } else if free_bytes >= MIN_FREE_DISK_BYTES {
        // 100 MB .. 1 GB
        ComponentHealth::degraded(
            "disk_space",
            &format!("Low disk space: {} MB free", free_mb),
        )
        .with_latency(elapsed)
        .with_meta("free_bytes", &free_bytes.to_string())
        .with_meta("free_mb", &free_mb.to_string())
    } else {
        ComponentHealth::unhealthy(
            "disk_space",
            &format!("Critical: only {} MB free (< 100 MB)", free_mb),
        )
        .with_latency(elapsed)
        .with_meta("free_bytes", &free_bytes.to_string())
        .with_meta("free_mb", &free_mb.to_string())
    }
}

/// Platform-specific helper to get free bytes for the filesystem containing `path`.
fn disk_free_bytes(path: &std::path::Path) -> u64 {
    use sysinfo::Disks;
    let disks = Disks::new_with_refreshed_list();
    // Find the disk whose mount point is a prefix of `path`
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut best_match: Option<u64> = None;
    let mut best_len = 0usize;

    for disk in disks.list() {
        let mount = disk.mount_point();
        let mount_str = mount.to_string_lossy();
        let canon_str = canonical.to_string_lossy();
        // Case-insensitive prefix match (important on Windows)
        if canon_str.to_lowercase().starts_with(&mount_str.to_lowercase())
            && mount_str.len() >= best_len
        {
            best_len = mount_str.len();
            best_match = Some(disk.available_space());
        }
    }
    best_match.unwrap_or(0)
}

/// Check process memory usage against a threshold.
pub fn check_memory_usage(threshold_bytes: u64) -> ComponentHealth {
    let start = Instant::now();

    let usage = process_memory_bytes();
    let elapsed = start.elapsed().as_millis() as u64;
    let usage_mb = usage / (1024 * 1024);
    let threshold_mb = threshold_bytes / (1024 * 1024);

    let pct = if threshold_bytes > 0 {
        (usage as f64 / threshold_bytes as f64 * 100.0) as u64
    } else {
        0
    };

    if usage < threshold_bytes * 80 / 100 {
        ComponentHealth::healthy(
            "memory_usage",
            &format!("{} MB used ({}% of {} MB limit)", usage_mb, pct, threshold_mb),
        )
        .with_latency(elapsed)
        .with_meta("usage_bytes", &usage.to_string())
        .with_meta("threshold_bytes", &threshold_bytes.to_string())
        .with_meta("pct", &pct.to_string())
    } else if usage < threshold_bytes {
        ComponentHealth::degraded(
            "memory_usage",
            &format!(
                "High memory: {} MB ({}% of {} MB limit)",
                usage_mb, pct, threshold_mb
            ),
        )
        .with_latency(elapsed)
        .with_meta("usage_bytes", &usage.to_string())
        .with_meta("threshold_bytes", &threshold_bytes.to_string())
        .with_meta("pct", &pct.to_string())
    } else {
        ComponentHealth::unhealthy(
            "memory_usage",
            &format!(
                "Memory exceeded: {} MB ({}% of {} MB limit)",
                usage_mb, pct, threshold_mb
            ),
        )
        .with_latency(elapsed)
        .with_meta("usage_bytes", &usage.to_string())
        .with_meta("threshold_bytes", &threshold_bytes.to_string())
        .with_meta("pct", &pct.to_string())
    }
}

/// Get current process memory usage via sysinfo.
fn process_memory_bytes() -> u64 {
    use sysinfo::System;
    let pid = sysinfo::get_current_pid().unwrap_or(sysinfo::Pid::from(0));
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).map(|p| p.memory()).unwrap_or(0)
}

/// Check that at least one provider is configured (config-only, no API call).
pub fn check_providers(configured_providers: &[String]) -> ComponentHealth {
    let start = Instant::now();
    let count = configured_providers.len();
    let elapsed = start.elapsed().as_millis() as u64;

    if count >= 2 {
        ComponentHealth::healthy(
            "providers",
            &format!("{} providers configured", count),
        )
        .with_latency(elapsed)
        .with_meta("count", &count.to_string())
        .with_meta("names", &configured_providers.join(","))
    } else if count == 1 {
        ComponentHealth::degraded(
            "providers",
            &format!("Only 1 provider configured: {}", configured_providers[0]),
        )
        .with_latency(elapsed)
        .with_meta("count", "1")
        .with_meta("names", &configured_providers[0])
    } else {
        ComponentHealth::unhealthy("providers", "No providers configured")
            .with_latency(elapsed)
            .with_meta("count", "0")
    }
}

/// Check cluster worker connectivity based on last heartbeat timestamps.
///
/// `workers` is a list of (name, last_heartbeat_secs_ago).
pub fn check_cluster_connectivity(workers: &[(String, u64)]) -> ComponentHealth {
    let start = Instant::now();
    let total = workers.len();
    // A heartbeat older than 120 seconds is considered stale
    let stale_threshold_secs = 120;
    let stale: Vec<String> = workers
        .iter()
        .filter(|(_, age)| *age > stale_threshold_secs)
        .map(|(name, age)| format!("{}({}s ago)", name, age))
        .collect();
    let healthy_count = total - stale.len();
    let elapsed = start.elapsed().as_millis() as u64;

    if total == 0 {
        ComponentHealth::degraded("cluster_connectivity", "No workers registered")
            .with_latency(elapsed)
            .with_meta("total", "0")
    } else if stale.is_empty() {
        ComponentHealth::healthy(
            "cluster_connectivity",
            &format!("{}/{} workers online", healthy_count, total),
        )
        .with_latency(elapsed)
        .with_meta("total", &total.to_string())
        .with_meta("healthy", &healthy_count.to_string())
    } else if healthy_count > 0 {
        ComponentHealth::degraded(
            "cluster_connectivity",
            &format!(
                "{}/{} workers online, stale: {}",
                healthy_count,
                total,
                stale.join(", ")
            ),
        )
        .with_latency(elapsed)
        .with_meta("total", &total.to_string())
        .with_meta("healthy", &healthy_count.to_string())
        .with_meta("stale", &stale.join(","))
    } else {
        ComponentHealth::unhealthy(
            "cluster_connectivity",
            &format!("All {} workers stale: {}", total, stale.join(", ")),
        )
        .with_latency(elapsed)
        .with_meta("total", &total.to_string())
        .with_meta("healthy", "0")
        .with_meta("stale", &stale.join(","))
    }
}

/// Check that cron jobs are not stalled.
///
/// `jobs` is a list of (name, status_str, secs_since_last_run). A job whose
/// last run was > `stall_threshold_secs` ago is considered stalled.
pub fn check_cron_scheduler(
    jobs: &[(String, String, Option<u64>)],
    stall_threshold_secs: u64,
) -> ComponentHealth {
    let start = Instant::now();
    let total = jobs.len();
    let active: Vec<&(String, String, Option<u64>)> = jobs
        .iter()
        .filter(|(_, status, _)| status == "active")
        .collect();
    let stalled: Vec<String> = active
        .iter()
        .filter(|(_, _, last_run)| {
            last_run.map(|t| t > stall_threshold_secs).unwrap_or(false)
        })
        .map(|(name, _, last_run)| {
            format!("{}({}s ago)", name, last_run.unwrap_or(0))
        })
        .collect();
    let elapsed = start.elapsed().as_millis() as u64;

    if total == 0 {
        ComponentHealth::healthy("cron_scheduler", "No cron jobs configured")
            .with_latency(elapsed)
    } else if stalled.is_empty() {
        ComponentHealth::healthy(
            "cron_scheduler",
            &format!(
                "{} jobs ({} active), none stalled",
                total,
                active.len()
            ),
        )
        .with_latency(elapsed)
        .with_meta("total", &total.to_string())
        .with_meta("active", &active.len().to_string())
    } else {
        ComponentHealth::degraded(
            "cron_scheduler",
            &format!("{} jobs stalled: {}", stalled.len(), stalled.join(", ")),
        )
        .with_latency(elapsed)
        .with_meta("total", &total.to_string())
        .with_meta("stalled", &stalled.join(","))
    }
}

/// Check that the system is not near its rate-limit capacity.
///
/// `current_actions` / `max_actions_per_hour` is the global rate usage.
pub fn check_rate_limits(current_actions: u32, max_actions_per_hour: u32) -> ComponentHealth {
    let start = Instant::now();
    let pct = if max_actions_per_hour > 0 {
        (current_actions as f64 / max_actions_per_hour as f64 * 100.0) as u32
    } else {
        0
    };
    let elapsed = start.elapsed().as_millis() as u64;

    if pct < 70 {
        ComponentHealth::healthy(
            "rate_limits",
            &format!(
                "{}/{} actions used ({}%)",
                current_actions, max_actions_per_hour, pct
            ),
        )
        .with_latency(elapsed)
        .with_meta("current", &current_actions.to_string())
        .with_meta("max", &max_actions_per_hour.to_string())
        .with_meta("pct", &pct.to_string())
    } else if pct < 90 {
        ComponentHealth::degraded(
            "rate_limits",
            &format!(
                "Approaching limit: {}/{} actions ({}%)",
                current_actions, max_actions_per_hour, pct
            ),
        )
        .with_latency(elapsed)
        .with_meta("current", &current_actions.to_string())
        .with_meta("max", &max_actions_per_hour.to_string())
        .with_meta("pct", &pct.to_string())
    } else {
        ComponentHealth::unhealthy(
            "rate_limits",
            &format!(
                "Near or at capacity: {}/{} actions ({}%)",
                current_actions, max_actions_per_hour, pct
            ),
        )
        .with_latency(elapsed)
        .with_meta("current", &current_actions.to_string())
        .with_meta("max", &max_actions_per_hour.to_string())
        .with_meta("pct", &pct.to_string())
    }
}

/// Check for pending approvals stuck longer than 1 hour.
///
/// `pending` is a list of (request_id, age_secs).
pub fn check_pending_approvals(pending: &[(String, u64)]) -> ComponentHealth {
    let start = Instant::now();
    let stuck_threshold_secs = 3600; // 1 hour
    let stuck: Vec<String> = pending
        .iter()
        .filter(|(_, age)| *age > stuck_threshold_secs)
        .map(|(id, age)| format!("{}({}s)", id, age))
        .collect();
    let elapsed = start.elapsed().as_millis() as u64;

    if pending.is_empty() {
        ComponentHealth::healthy("pending_approvals", "No pending approvals")
            .with_latency(elapsed)
    } else if stuck.is_empty() {
        ComponentHealth::healthy(
            "pending_approvals",
            &format!("{} pending, none stuck", pending.len()),
        )
        .with_latency(elapsed)
        .with_meta("pending", &pending.len().to_string())
    } else {
        let status = if stuck.len() >= 5 {
            HealthStatus::Unhealthy
        } else {
            HealthStatus::Degraded
        };
        let mut ch = ComponentHealth {
            name: "pending_approvals".to_string(),
            status,
            message: format!("{} approvals stuck > 1h: {}", stuck.len(), stuck.join(", ")),
            last_check: Utc::now(),
            latency_ms: elapsed,
            metadata: HashMap::new(),
        };
        ch.metadata
            .insert("pending".to_string(), pending.len().to_string());
        ch.metadata
            .insert("stuck".to_string(), stuck.len().to_string());
        ch
    }
}

/// Check that the error rate in the last hour is below 10%.
///
/// `total_requests` and `error_count` are for the most recent hour.
pub fn check_error_rate(total_requests: u64, error_count: u64) -> ComponentHealth {
    let start = Instant::now();
    let rate = if total_requests > 0 {
        (error_count as f64 / total_requests as f64 * 100.0) as u64
    } else {
        0
    };
    let elapsed = start.elapsed().as_millis() as u64;

    if total_requests == 0 {
        ComponentHealth::healthy("error_rate", "No requests in the last hour")
            .with_latency(elapsed)
            .with_meta("total_requests", "0")
            .with_meta("error_count", "0")
            .with_meta("rate_pct", "0")
    } else if rate < 5 {
        ComponentHealth::healthy(
            "error_rate",
            &format!(
                "{}/{} errors ({}%)",
                error_count, total_requests, rate
            ),
        )
        .with_latency(elapsed)
        .with_meta("total_requests", &total_requests.to_string())
        .with_meta("error_count", &error_count.to_string())
        .with_meta("rate_pct", &rate.to_string())
    } else if rate < 10 {
        ComponentHealth::degraded(
            "error_rate",
            &format!(
                "Elevated error rate: {}/{} ({}%)",
                error_count, total_requests, rate
            ),
        )
        .with_latency(elapsed)
        .with_meta("total_requests", &total_requests.to_string())
        .with_meta("error_count", &error_count.to_string())
        .with_meta("rate_pct", &rate.to_string())
    } else {
        ComponentHealth::unhealthy(
            "error_rate",
            &format!(
                "High error rate: {}/{} ({}%)",
                error_count, total_requests, rate
            ),
        )
        .with_latency(elapsed)
        .with_meta("total_requests", &total_requests.to_string())
        .with_meta("error_count", &error_count.to_string())
        .with_meta("rate_pct", &rate.to_string())
    }
}

/// Check that daily spend is within budget.
///
/// `spent_today_usd` is the total cost recorded today, `daily_budget_usd` is
/// the configured limit.
pub fn check_cost_budget(spent_today_usd: f64, daily_budget_usd: f64) -> ComponentHealth {
    let start = Instant::now();
    let pct = if daily_budget_usd > 0.0 {
        (spent_today_usd / daily_budget_usd * 100.0) as u64
    } else {
        0
    };
    let elapsed = start.elapsed().as_millis() as u64;

    if daily_budget_usd <= 0.0 {
        ComponentHealth::healthy("cost_budget", "No daily budget configured")
            .with_latency(elapsed)
            .with_meta("spent_usd", &format!("{:.4}", spent_today_usd))
    } else if pct < 80 {
        ComponentHealth::healthy(
            "cost_budget",
            &format!(
                "${:.4} / ${:.2} ({}%)",
                spent_today_usd, daily_budget_usd, pct
            ),
        )
        .with_latency(elapsed)
        .with_meta("spent_usd", &format!("{:.4}", spent_today_usd))
        .with_meta("budget_usd", &format!("{:.2}", daily_budget_usd))
        .with_meta("pct", &pct.to_string())
    } else if pct < 100 {
        ComponentHealth::degraded(
            "cost_budget",
            &format!(
                "Approaching budget: ${:.4} / ${:.2} ({}%)",
                spent_today_usd, daily_budget_usd, pct
            ),
        )
        .with_latency(elapsed)
        .with_meta("spent_usd", &format!("{:.4}", spent_today_usd))
        .with_meta("budget_usd", &format!("{:.2}", daily_budget_usd))
        .with_meta("pct", &pct.to_string())
    } else {
        ComponentHealth::unhealthy(
            "cost_budget",
            &format!(
                "Budget exceeded: ${:.4} / ${:.2} ({}%)",
                spent_today_usd, daily_budget_usd, pct
            ),
        )
        .with_latency(elapsed)
        .with_meta("spent_usd", &format!("{:.4}", spent_today_usd))
        .with_meta("budget_usd", &format!("{:.2}", daily_budget_usd))
        .with_meta("pct", &pct.to_string())
    }
}

// ---------------------------------------------------------------------------
// HealthChecker — orchestrates checks
// ---------------------------------------------------------------------------

/// Type alias for a health-check function that can be stored and invoked.
type CheckFn = Box<dyn Fn() -> ComponentHealth + Send + Sync>;

/// Central health-checker that manages a set of named health checks.
pub struct HealthChecker {
    checks: Mutex<Vec<(String, CheckFn)>>,
    start_time: Instant,
    version: String,
}

impl HealthChecker {
    /// Create a new `HealthChecker` with the given version string.
    pub fn new(version: &str) -> Self {
        Self {
            checks: Mutex::new(Vec::new()),
            start_time: Instant::now(),
            version: version.to_string(),
        }
    }

    /// Register a named health check.
    ///
    /// If a check with the same name already exists, it is replaced.
    pub fn register_check<F>(&self, name: &str, check_fn: F)
    where
        F: Fn() -> ComponentHealth + Send + Sync + 'static,
    {
        let mut checks = self.checks.lock().unwrap();
        // Remove existing check with same name (if any)
        checks.retain(|(n, _)| n != name);
        checks.push((name.to_string(), Box::new(check_fn)));
    }

    /// Run all registered checks and return the aggregated system health.
    pub fn run_all(&self) -> SystemHealth {
        let checks = self.checks.lock().unwrap();
        let mut components: Vec<ComponentHealth> = Vec::with_capacity(checks.len());
        let mut overall = HealthStatus::Healthy;

        for (name, check_fn) in checks.iter() {
            let start = Instant::now();
            let mut result = check_fn();
            let elapsed = start.elapsed().as_millis() as u64;
            // Override latency with the actual measured value (includes the
            // internal latency of the check function itself)
            if result.latency_ms == 0 {
                result.latency_ms = elapsed;
            }
            if result.name.is_empty() {
                result.name = name.clone();
            }
            overall = overall.worse(result.status);
            components.push(result);
        }

        let uptime_secs = self.start_time.elapsed().as_secs();

        SystemHealth {
            status: overall,
            components,
            uptime_secs,
            version: self.version.clone(),
            checked_at: Utc::now(),
        }
    }

    /// Run a single named check.
    ///
    /// Returns `None` if no check with that name is registered.
    pub fn run_check(&self, name: &str) -> Option<ComponentHealth> {
        let checks = self.checks.lock().unwrap();
        for (n, check_fn) in checks.iter() {
            if n == name {
                let start = Instant::now();
                let mut result = check_fn();
                let elapsed = start.elapsed().as_millis() as u64;
                if result.latency_ms == 0 {
                    result.latency_ms = elapsed;
                }
                return Some(result);
            }
        }
        None
    }

    /// Return the number of registered checks.
    pub fn check_count(&self) -> usize {
        self.checks.lock().unwrap().len()
    }

    /// Return a list of registered check names (in registration order).
    pub fn check_names(&self) -> Vec<String> {
        self.checks
            .lock()
            .unwrap()
            .iter()
            .map(|(n, _)| n.clone())
            .collect()
    }

    /// Serialize a `SystemHealth` to a `serde_json::Value`.
    pub fn to_json(health: &SystemHealth) -> Value {
        serde_json::to_value(health).unwrap_or_else(|e| {
            warn!("Failed to serialize SystemHealth: {}", e);
            serde_json::json!({"error": e.to_string()})
        })
    }

    /// Render a `SystemHealth` in Prometheus text exposition format.
    ///
    /// Emits:
    /// - `clawtex_health_status` gauge (0=healthy, 1=degraded, 2=unhealthy)
    /// - `clawtex_health_uptime_seconds` gauge
    /// - Per-component `clawtex_component_health{component="X"}` gauge
    /// - Per-component `clawtex_component_latency_ms{component="X"}` gauge
    pub fn to_prometheus(health: &SystemHealth) -> String {
        let mut out = String::new();

        // Overall status
        out.push_str("# HELP clawtex_health_status Overall system health (0=healthy, 1=degraded, 2=unhealthy)\n");
        out.push_str("# TYPE clawtex_health_status gauge\n");
        out.push_str(&format!(
            "clawtex_health_status {}\n",
            health.status.severity()
        ));

        // Uptime
        out.push_str("# HELP clawtex_health_uptime_seconds System uptime in seconds\n");
        out.push_str("# TYPE clawtex_health_uptime_seconds gauge\n");
        out.push_str(&format!(
            "clawtex_health_uptime_seconds {}\n",
            health.uptime_secs
        ));

        // Component health
        out.push_str("# HELP clawtex_component_health Per-component health (0=healthy, 1=degraded, 2=unhealthy)\n");
        out.push_str("# TYPE clawtex_component_health gauge\n");
        for comp in &health.components {
            out.push_str(&format!(
                "clawtex_component_health{{component=\"{}\"}} {}\n",
                comp.name,
                comp.status.severity()
            ));
        }

        // Component latency
        out.push_str("# HELP clawtex_component_latency_ms Health check latency in milliseconds\n");
        out.push_str("# TYPE clawtex_component_latency_ms gauge\n");
        for comp in &health.components {
            out.push_str(&format!(
                "clawtex_component_latency_ms{{component=\"{}\"}} {}\n",
                comp.name, comp.latency_ms
            ));
        }

        out
    }

    /// Current uptime in seconds.
    pub fn uptime_secs(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- HealthStatus tests --

    #[test]
    fn test_health_status_severity_ordering() {
        assert!(HealthStatus::Healthy.severity() < HealthStatus::Degraded.severity());
        assert!(HealthStatus::Degraded.severity() < HealthStatus::Unhealthy.severity());
    }

    #[test]
    fn test_health_status_worse() {
        assert_eq!(
            HealthStatus::Healthy.worse(HealthStatus::Degraded),
            HealthStatus::Degraded
        );
        assert_eq!(
            HealthStatus::Degraded.worse(HealthStatus::Healthy),
            HealthStatus::Degraded
        );
        assert_eq!(
            HealthStatus::Degraded.worse(HealthStatus::Unhealthy),
            HealthStatus::Unhealthy
        );
        assert_eq!(
            HealthStatus::Unhealthy.worse(HealthStatus::Healthy),
            HealthStatus::Unhealthy
        );
        assert_eq!(
            HealthStatus::Healthy.worse(HealthStatus::Healthy),
            HealthStatus::Healthy
        );
    }

    #[test]
    fn test_health_status_display() {
        assert_eq!(format!("{}", HealthStatus::Healthy), "healthy");
        assert_eq!(format!("{}", HealthStatus::Degraded), "degraded");
        assert_eq!(format!("{}", HealthStatus::Unhealthy), "unhealthy");
    }

    // -- ComponentHealth tests --

    #[test]
    fn test_component_health_constructors() {
        let h = ComponentHealth::healthy("test", "all good");
        assert_eq!(h.status, HealthStatus::Healthy);
        assert_eq!(h.name, "test");
        assert_eq!(h.message, "all good");

        let d = ComponentHealth::degraded("test", "partial");
        assert_eq!(d.status, HealthStatus::Degraded);

        let u = ComponentHealth::unhealthy("test", "broken");
        assert_eq!(u.status, HealthStatus::Unhealthy);
    }

    #[test]
    fn test_component_health_with_meta() {
        let ch = ComponentHealth::healthy("db", "ok")
            .with_meta("version", "3.40")
            .with_meta("tables", "5");
        assert_eq!(ch.metadata.get("version").unwrap(), "3.40");
        assert_eq!(ch.metadata.get("tables").unwrap(), "5");
    }

    #[test]
    fn test_component_health_with_latency() {
        let ch = ComponentHealth::healthy("db", "ok").with_latency(42);
        assert_eq!(ch.latency_ms, 42);
    }

    // -- check_database tests --

    #[test]
    fn test_check_database_no_dir() {
        let result = check_database("/nonexistent/path/that/should/not/exist");
        assert_eq!(result.status, HealthStatus::Unhealthy);
        assert!(result.message.contains("No databases"));
    }

    #[test]
    fn test_check_database_with_temp_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();
        // Create some DB files
        std::fs::write(tmp.path().join("core.db"), b"").unwrap();
        std::fs::write(tmp.path().join("costs.db"), b"").unwrap();
        std::fs::write(tmp.path().join("memory.db"), b"").unwrap();

        let result = check_database(dir);
        assert_eq!(result.status, HealthStatus::Healthy);
        assert!(result.message.contains("3 databases"));
    }

    #[test]
    fn test_check_database_partial() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();
        std::fs::write(tmp.path().join("core.db"), b"").unwrap();
        // costs.db and memory.db missing

        let result = check_database(dir);
        assert_eq!(result.status, HealthStatus::Degraded);
        assert!(result.message.contains("Missing"));
    }

    // -- check_disk_space tests --

    #[test]
    fn test_check_disk_space_current_dir() {
        // The current working directory should always have some disk space.
        let result = check_disk_space(".");
        // Should not be unhealthy on a normal dev machine
        assert_ne!(result.name, "");
        assert_eq!(result.name, "disk_space");
        assert!(result.metadata.contains_key("free_bytes"));
    }

    // -- check_memory_usage tests --

    #[test]
    fn test_check_memory_healthy() {
        // Set a very high threshold so the test always passes
        let result = check_memory_usage(100 * 1024 * 1024 * 1024); // 100 GB
        assert_eq!(result.status, HealthStatus::Healthy);
        assert_eq!(result.name, "memory_usage");
    }

    #[test]
    fn test_check_memory_unhealthy_low_threshold() {
        // Set threshold to 1 byte -- the process always exceeds that
        let result = check_memory_usage(1);
        assert_eq!(result.status, HealthStatus::Unhealthy);
    }

    // -- check_providers tests --

    #[test]
    fn test_check_providers_multiple() {
        let providers = vec!["ollama".to_string(), "gemini".to_string()];
        let result = check_providers(&providers);
        assert_eq!(result.status, HealthStatus::Healthy);
        assert!(result.message.contains("2 providers"));
    }

    #[test]
    fn test_check_providers_single() {
        let providers = vec!["ollama".to_string()];
        let result = check_providers(&providers);
        assert_eq!(result.status, HealthStatus::Degraded);
    }

    #[test]
    fn test_check_providers_none() {
        let providers: Vec<String> = vec![];
        let result = check_providers(&providers);
        assert_eq!(result.status, HealthStatus::Unhealthy);
    }

    // -- check_cluster_connectivity tests --

    #[test]
    fn test_cluster_all_online() {
        let workers = vec![
            ("z13".to_string(), 10u64),
            ("m1-mac".to_string(), 30u64),
        ];
        let result = check_cluster_connectivity(&workers);
        assert_eq!(result.status, HealthStatus::Healthy);
        assert!(result.message.contains("2/2"));
    }

    #[test]
    fn test_cluster_some_stale() {
        let workers = vec![
            ("z13".to_string(), 10u64),
            ("acer".to_string(), 300u64), // stale
        ];
        let result = check_cluster_connectivity(&workers);
        assert_eq!(result.status, HealthStatus::Degraded);
        assert!(result.message.contains("stale"));
    }

    #[test]
    fn test_cluster_all_stale() {
        let workers = vec![
            ("z13".to_string(), 999u64),
            ("acer".to_string(), 888u64),
        ];
        let result = check_cluster_connectivity(&workers);
        assert_eq!(result.status, HealthStatus::Unhealthy);
    }

    #[test]
    fn test_cluster_no_workers() {
        let workers: Vec<(String, u64)> = vec![];
        let result = check_cluster_connectivity(&workers);
        assert_eq!(result.status, HealthStatus::Degraded);
    }

    // -- check_cron_scheduler tests --

    #[test]
    fn test_cron_no_jobs() {
        let result = check_cron_scheduler(&[], 3600);
        assert_eq!(result.status, HealthStatus::Healthy);
    }

    #[test]
    fn test_cron_all_active_recent() {
        let jobs = vec![
            ("freelancer".to_string(), "active".to_string(), Some(60u64)),
            ("leads".to_string(), "active".to_string(), Some(120u64)),
        ];
        let result = check_cron_scheduler(&jobs, 3600);
        assert_eq!(result.status, HealthStatus::Healthy);
    }

    #[test]
    fn test_cron_stalled() {
        let jobs = vec![
            ("freelancer".to_string(), "active".to_string(), Some(7200u64)),
        ];
        let result = check_cron_scheduler(&jobs, 3600);
        assert_eq!(result.status, HealthStatus::Degraded);
        assert!(result.message.contains("stalled"));
    }

    // -- check_rate_limits tests --

    #[test]
    fn test_rate_limits_low_usage() {
        let result = check_rate_limits(10, 100);
        assert_eq!(result.status, HealthStatus::Healthy);
    }

    #[test]
    fn test_rate_limits_medium_usage() {
        let result = check_rate_limits(75, 100);
        assert_eq!(result.status, HealthStatus::Degraded);
    }

    #[test]
    fn test_rate_limits_high_usage() {
        let result = check_rate_limits(95, 100);
        assert_eq!(result.status, HealthStatus::Unhealthy);
    }

    // -- check_pending_approvals tests --

    #[test]
    fn test_no_pending_approvals() {
        let result = check_pending_approvals(&[]);
        assert_eq!(result.status, HealthStatus::Healthy);
    }

    #[test]
    fn test_pending_not_stuck() {
        let pending = vec![("req-1".to_string(), 600u64)]; // 10 min, not stuck
        let result = check_pending_approvals(&pending);
        assert_eq!(result.status, HealthStatus::Healthy);
    }

    #[test]
    fn test_pending_stuck_degraded() {
        let pending = vec![("req-1".to_string(), 7200u64)]; // 2 hours
        let result = check_pending_approvals(&pending);
        assert_eq!(result.status, HealthStatus::Degraded);
    }

    #[test]
    fn test_pending_many_stuck_unhealthy() {
        let pending: Vec<(String, u64)> = (0..6)
            .map(|i| (format!("req-{}", i), 5000u64))
            .collect();
        let result = check_pending_approvals(&pending);
        assert_eq!(result.status, HealthStatus::Unhealthy);
    }

    // -- check_error_rate tests --

    #[test]
    fn test_error_rate_none() {
        let result = check_error_rate(0, 0);
        assert_eq!(result.status, HealthStatus::Healthy);
    }

    #[test]
    fn test_error_rate_low() {
        let result = check_error_rate(100, 2);
        assert_eq!(result.status, HealthStatus::Healthy);
    }

    #[test]
    fn test_error_rate_elevated() {
        let result = check_error_rate(100, 7);
        assert_eq!(result.status, HealthStatus::Degraded);
    }

    #[test]
    fn test_error_rate_high() {
        let result = check_error_rate(100, 15);
        assert_eq!(result.status, HealthStatus::Unhealthy);
    }

    // -- check_cost_budget tests --

    #[test]
    fn test_cost_budget_no_limit() {
        let result = check_cost_budget(5.0, 0.0);
        assert_eq!(result.status, HealthStatus::Healthy);
    }

    #[test]
    fn test_cost_budget_within() {
        let result = check_cost_budget(3.0, 10.0);
        assert_eq!(result.status, HealthStatus::Healthy);
    }

    #[test]
    fn test_cost_budget_approaching() {
        let result = check_cost_budget(8.5, 10.0);
        assert_eq!(result.status, HealthStatus::Degraded);
    }

    #[test]
    fn test_cost_budget_exceeded() {
        let result = check_cost_budget(12.0, 10.0);
        assert_eq!(result.status, HealthStatus::Unhealthy);
    }

    // -- HealthChecker tests --

    #[test]
    fn test_checker_register_and_run_all() {
        let checker = HealthChecker::new("0.1.0-test");
        checker.register_check("always_healthy", || {
            ComponentHealth::healthy("always_healthy", "fine")
        });
        checker.register_check("always_degraded", || {
            ComponentHealth::degraded("always_degraded", "meh")
        });

        let health = checker.run_all();
        assert_eq!(health.status, HealthStatus::Degraded);
        assert_eq!(health.components.len(), 2);
        assert_eq!(health.version, "0.1.0-test");
    }

    #[test]
    fn test_checker_overall_worst_status() {
        let checker = HealthChecker::new("1.0.0");
        checker.register_check("ok", || ComponentHealth::healthy("ok", "fine"));
        checker.register_check("bad", || ComponentHealth::unhealthy("bad", "broken"));
        checker.register_check("mid", || ComponentHealth::degraded("mid", "meh"));

        let health = checker.run_all();
        assert_eq!(health.status, HealthStatus::Unhealthy);
    }

    #[test]
    fn test_checker_run_single_check() {
        let checker = HealthChecker::new("1.0.0");
        checker.register_check("alpha", || {
            ComponentHealth::healthy("alpha", "ok")
        });
        checker.register_check("beta", || {
            ComponentHealth::degraded("beta", "slow")
        });

        let result = checker.run_check("beta");
        assert!(result.is_some());
        assert_eq!(result.unwrap().status, HealthStatus::Degraded);

        let missing = checker.run_check("gamma");
        assert!(missing.is_none());
    }

    #[test]
    fn test_checker_check_count() {
        let checker = HealthChecker::new("1.0.0");
        assert_eq!(checker.check_count(), 0);
        checker.register_check("a", || ComponentHealth::healthy("a", "ok"));
        assert_eq!(checker.check_count(), 1);
        checker.register_check("b", || ComponentHealth::healthy("b", "ok"));
        assert_eq!(checker.check_count(), 2);
    }

    #[test]
    fn test_checker_check_names() {
        let checker = HealthChecker::new("1.0.0");
        checker.register_check("database", || ComponentHealth::healthy("database", "ok"));
        checker.register_check("disk", || ComponentHealth::healthy("disk", "ok"));
        let names = checker.check_names();
        assert_eq!(names, vec!["database", "disk"]);
    }

    #[test]
    fn test_checker_replace_existing_check() {
        let checker = HealthChecker::new("1.0.0");
        checker.register_check("test", || ComponentHealth::healthy("test", "v1"));
        checker.register_check("test", || ComponentHealth::degraded("test", "v2"));
        assert_eq!(checker.check_count(), 1);
        let result = checker.run_check("test").unwrap();
        assert_eq!(result.status, HealthStatus::Degraded);
        assert_eq!(result.message, "v2");
    }

    #[test]
    fn test_checker_to_json() {
        let checker = HealthChecker::new("0.1.0");
        checker.register_check("db", || ComponentHealth::healthy("db", "ok"));
        let health = checker.run_all();
        let json = HealthChecker::to_json(&health);
        assert_eq!(json["status"], "healthy");
        assert_eq!(json["version"], "0.1.0");
        assert!(json["components"].as_array().unwrap().len() == 1);
        assert!(json["uptime_secs"].as_u64().is_some());
    }

    #[test]
    fn test_checker_to_prometheus() {
        let checker = HealthChecker::new("0.1.0");
        checker.register_check("db", || {
            ComponentHealth::healthy("db", "ok").with_latency(5)
        });
        checker.register_check("disk", || {
            ComponentHealth::degraded("disk", "low").with_latency(12)
        });
        let health = checker.run_all();
        let prom = HealthChecker::to_prometheus(&health);

        assert!(prom.contains("clawtex_health_status 1"), "overall should be degraded (1)");
        assert!(prom.contains("clawtex_health_uptime_seconds"));
        assert!(prom.contains("clawtex_component_health{component=\"db\"} 0"));
        assert!(prom.contains("clawtex_component_health{component=\"disk\"} 1"));
        assert!(prom.contains("clawtex_component_latency_ms{component=\"db\"}"));
        assert!(prom.contains("clawtex_component_latency_ms{component=\"disk\"}"));
    }

    #[test]
    fn test_checker_all_healthy_overall() {
        let checker = HealthChecker::new("1.0.0");
        checker.register_check("a", || ComponentHealth::healthy("a", "ok"));
        checker.register_check("b", || ComponentHealth::healthy("b", "ok"));
        let health = checker.run_all();
        assert_eq!(health.status, HealthStatus::Healthy);
    }

    #[test]
    fn test_checker_empty_returns_healthy() {
        let checker = HealthChecker::new("1.0.0");
        let health = checker.run_all();
        assert_eq!(health.status, HealthStatus::Healthy);
        assert!(health.components.is_empty());
    }

    #[test]
    fn test_checker_uptime() {
        let checker = HealthChecker::new("1.0.0");
        // Uptime should be >= 0 immediately after creation
        assert!(checker.uptime_secs() < 5);
    }

    #[test]
    fn test_system_health_serialization() {
        let health = SystemHealth {
            status: HealthStatus::Degraded,
            components: vec![
                ComponentHealth::healthy("a", "ok"),
                ComponentHealth::degraded("b", "slow"),
            ],
            uptime_secs: 3600,
            version: "0.1.0".to_string(),
            checked_at: Utc::now(),
        };
        let json_str = serde_json::to_string(&health).unwrap();
        assert!(json_str.contains("\"status\":\"degraded\""));
        assert!(json_str.contains("\"version\":\"0.1.0\""));

        // Round-trip
        let parsed: SystemHealth = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.status, HealthStatus::Degraded);
        assert_eq!(parsed.components.len(), 2);
    }

    #[test]
    fn test_component_health_serialization() {
        let ch = ComponentHealth::healthy("test_comp", "everything is fine")
            .with_latency(42)
            .with_meta("key1", "val1");
        let json_str = serde_json::to_string(&ch).unwrap();
        assert!(json_str.contains("\"name\":\"test_comp\""));
        assert!(json_str.contains("\"latency_ms\":42"));
        assert!(json_str.contains("\"key1\":\"val1\""));

        let parsed: ComponentHealth = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.name, "test_comp");
        assert_eq!(parsed.latency_ms, 42);
        assert_eq!(parsed.metadata.get("key1").unwrap(), "val1");
    }

    #[test]
    fn test_prometheus_format_correctness() {
        let health = SystemHealth {
            status: HealthStatus::Healthy,
            components: vec![
                ComponentHealth::healthy("db", "ok").with_latency(1),
                ComponentHealth::healthy("disk", "ok").with_latency(2),
            ],
            uptime_secs: 120,
            version: "0.1.0".to_string(),
            checked_at: Utc::now(),
        };
        let prom = HealthChecker::to_prometheus(&health);
        // Each metric should have HELP and TYPE annotations
        assert!(prom.contains("# HELP clawtex_health_status"));
        assert!(prom.contains("# TYPE clawtex_health_status gauge"));
        assert!(prom.contains("# HELP clawtex_health_uptime_seconds"));
        assert!(prom.contains("# TYPE clawtex_health_uptime_seconds gauge"));
        assert!(prom.contains("# HELP clawtex_component_health"));
        assert!(prom.contains("# TYPE clawtex_component_health gauge"));
        assert!(prom.contains("# HELP clawtex_component_latency_ms"));
        assert!(prom.contains("# TYPE clawtex_component_latency_ms gauge"));
        // Verify values
        assert!(prom.contains("clawtex_health_status 0"));
        assert!(prom.contains("clawtex_health_uptime_seconds 120"));
    }

    #[test]
    fn test_integrated_checker_with_builtin_checks() {
        let checker = HealthChecker::new("0.1.0-test");
        // Register the built-in checks with test-appropriate parameters
        checker.register_check("providers", || {
            check_providers(&["ollama".to_string(), "gemini".to_string()])
        });
        checker.register_check("rate_limits", || check_rate_limits(5, 100));
        checker.register_check("error_rate", || check_error_rate(100, 1));
        checker.register_check("cost_budget", || check_cost_budget(2.0, 10.0));
        checker.register_check("pending_approvals", || check_pending_approvals(&[]));
        checker.register_check("cluster", || {
            check_cluster_connectivity(&[("z13".to_string(), 5)])
        });
        checker.register_check("cron", || {
            check_cron_scheduler(
                &[("test_job".to_string(), "active".to_string(), Some(60))],
                3600,
            )
        });

        let health = checker.run_all();
        assert_eq!(health.status, HealthStatus::Healthy);
        assert_eq!(health.components.len(), 7);

        // JSON output should be valid
        let json = HealthChecker::to_json(&health);
        assert_eq!(json["status"], "healthy");
        assert_eq!(json["components"].as_array().unwrap().len(), 7);

        // Prometheus output should contain all components
        let prom = HealthChecker::to_prometheus(&health);
        assert!(prom.contains("component=\"providers\""));
        assert!(prom.contains("component=\"rate_limits\""));
        assert!(prom.contains("component=\"cost_budget\""));
    }
}
