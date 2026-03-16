//! WorkerWatchdog — monitors cluster workers and attempts automatic recovery.
//!
//! When a worker goes offline, the watchdog can execute a recovery command
//! (typically an SSH command to restart the worker process) with configurable
//! retry limits and cooldown periods.

use std::collections::HashMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{info, warn, error};

use crate::cluster::ClusterNode;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Per-worker recovery configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryConfig {
    /// Worker name (must match `ClusterNode::name`).
    pub worker_name: String,
    /// Shell command to execute for recovery (e.g. SSH restart command).
    pub recovery_command: String,
    /// Maximum number of consecutive recovery attempts before giving up.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Minimum seconds between recovery attempts.
    #[serde(default = "default_cooldown_secs")]
    pub cooldown_secs: u64,
    /// Whether automatic recovery is enabled for this worker.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_max_retries() -> u32 {
    3
}
fn default_cooldown_secs() -> u64 {
    120
}
fn default_enabled() -> bool {
    true
}

impl RecoveryConfig {
    /// Create a new config with default retry / cooldown values.
    pub fn new(worker_name: impl Into<String>, recovery_command: impl Into<String>) -> Self {
        Self {
            worker_name: worker_name.into(),
            recovery_command: recovery_command.into(),
            max_retries: default_max_retries(),
            cooldown_secs: default_cooldown_secs(),
            enabled: default_enabled(),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

/// Tracks recovery state for a single worker.
#[derive(Debug)]
struct RecoveryState {
    /// Total recovery attempts since last full reset.
    attempts: u32,
    /// When the last recovery attempt was made.
    last_attempt: Option<Instant>,
    /// When the worker was last seen healthy after a recovery cycle.
    last_success: Option<Instant>,
    /// Number of consecutive failures (resets when worker comes online).
    consecutive_failures: u32,
}

impl Default for RecoveryState {
    fn default() -> Self {
        Self {
            attempts: 0,
            last_attempt: None,
            last_success: None,
            consecutive_failures: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Events emitted by the watchdog, serializable for logging / API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WatchdogEvent {
    WorkerDown {
        worker: String,
        since: String,
    },
    RecoveryAttempted {
        worker: String,
        attempt: u32,
        command: String,
    },
    RecoverySuccess {
        worker: String,
        attempt: u32,
    },
    RecoveryFailed {
        worker: String,
        attempt: u32,
        error: String,
    },
    MaxRetriesExceeded {
        worker: String,
    },
}

// ---------------------------------------------------------------------------
// Status snapshot (for API)
// ---------------------------------------------------------------------------

/// Serializable status for a single worker's watchdog state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchdogStatus {
    pub worker_name: String,
    pub recovery_enabled: bool,
    pub attempts: u32,
    pub max_retries: u32,
    pub last_attempt_ago_secs: Option<u64>,
    /// One of `"healthy"`, `"recovering"`, or `"exhausted"`.
    pub state: String,
}

// ---------------------------------------------------------------------------
// WorkerWatchdog
// ---------------------------------------------------------------------------

/// Maximum number of events kept in memory.
const MAX_EVENT_LOG: usize = 100;

/// Monitors cluster workers and runs recovery commands when they go offline.
pub struct WorkerWatchdog {
    /// Per-worker recovery configuration (keyed by worker name).
    configs: HashMap<String, RecoveryConfig>,
    /// Per-worker recovery state.
    states: Mutex<HashMap<String, RecoveryState>>,
    /// Rolling event log (capped at [`MAX_EVENT_LOG`]).
    event_log: Mutex<Vec<WatchdogEvent>>,
    /// HTTP client for optional health-check probes.
    #[allow(dead_code)]
    http_client: reqwest::Client,
}

impl WorkerWatchdog {
    // -- constructors -------------------------------------------------------

    /// Create an empty watchdog with no configured workers.
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
            states: Mutex::new(HashMap::new()),
            event_log: Mutex::new(Vec::new()),
            http_client: reqwest::Client::new(),
        }
    }

    /// Register a worker for monitoring.
    pub fn add_worker(&mut self, config: RecoveryConfig) {
        self.configs.insert(config.worker_name.clone(), config);
    }

    /// Create a watchdog pre-configured with the known Clawtex cluster workers.
    pub fn with_defaults() -> Self {
        let mut wd = Self::new();

        wd.add_worker(RecoveryConfig::new(
            "acer",
            r#"ssh user@192.168.1.115 "wmic process where \"name='python.exe'\" call terminate >nul 2>&1 & cd /d C:\Users\user\worker & C:\Python314\python.exe worker.py""#,
        ));

        wd.add_worker(RecoveryConfig::new(
            "m1-mac",
            r#"ssh marklight@100.87.93.58 'pkill -f worker.py; cd ~/worker && nohup /opt/homebrew/bin/python3 worker.py &'"#,
        ));

        wd
    }

    // -- core logic ---------------------------------------------------------

    /// Inspect the current cluster state and attempt recovery for offline workers.
    ///
    /// Returns all events generated during this check cycle.
    pub async fn check_and_recover(&self, workers: &[ClusterNode]) -> Vec<WatchdogEvent> {
        let mut events = Vec::new();

        // Build a lookup of current statuses.
        let status_map: HashMap<&str, &ClusterNode> =
            workers.iter().map(|w| (w.name.as_str(), w)).collect();

        for (name, config) in &self.configs {
            if !config.enabled {
                continue;
            }

            let node = status_map.get(name.as_str());
            let is_online = node.map_or(false, |n| n.status == "online");

            if is_online {
                // Worker is healthy — reset recovery state.
                let mut states = self.states.lock().await;
                if let Some(state) = states.get_mut(name.as_str()) {
                    if state.attempts > 0 {
                        let attempt = state.attempts;
                        state.attempts = 0;
                        state.consecutive_failures = 0;
                        state.last_success = Some(Instant::now());
                        let evt = WatchdogEvent::RecoverySuccess {
                            worker: name.clone(),
                            attempt,
                        };
                        info!(worker = %name, attempt, "Worker recovered successfully");
                        events.push(evt);
                    }
                }
                continue;
            }

            // Worker is offline (or unknown / not in list).
            let since = node
                .map(|n| n.last_seen.clone())
                .unwrap_or_else(|| "unknown".to_string());

            events.push(WatchdogEvent::WorkerDown {
                worker: name.clone(),
                since: since.clone(),
            });

            // Check whether we should attempt recovery.
            if !self.should_recover_inner(name, config).await {
                continue;
            }

            // Attempt recovery.
            let mut states = self.states.lock().await;
            let state = states.entry(name.clone()).or_default();
            state.attempts += 1;
            state.consecutive_failures += 1;
            state.last_attempt = Some(Instant::now());
            let attempt = state.attempts;
            drop(states); // release lock before spawning process

            info!(
                worker = %name,
                attempt,
                command = %config.recovery_command,
                "Attempting worker recovery"
            );

            events.push(WatchdogEvent::RecoveryAttempted {
                worker: name.clone(),
                attempt,
                command: config.recovery_command.clone(),
            });

            // Execute the recovery command.
            match Self::execute_recovery(&config.recovery_command).await {
                Ok(output) => {
                    info!(worker = %name, attempt, output = %output, "Recovery command completed");
                    // Note: we do NOT mark as success here. The worker must actually
                    // come back online (detected on the next check cycle) to be
                    // considered recovered.
                }
                Err(e) => {
                    let err_msg = format!("{:#}", e);
                    error!(worker = %name, attempt, error = %err_msg, "Recovery command failed");
                    events.push(WatchdogEvent::RecoveryFailed {
                        worker: name.clone(),
                        attempt,
                        error: err_msg,
                    });
                }
            }

            // Check if we've exhausted retries.
            let states = self.states.lock().await;
            if let Some(state) = states.get(name.as_str()) {
                if state.attempts >= config.max_retries {
                    warn!(worker = %name, max_retries = config.max_retries, "Max retries exceeded");
                    events.push(WatchdogEvent::MaxRetriesExceeded {
                        worker: name.clone(),
                    });
                }
            }
        }

        // Persist events to the rolling log.
        if !events.is_empty() {
            let mut log = self.event_log.lock().await;
            for evt in &events {
                log.push(evt.clone());
            }
            // Trim to the most recent MAX_EVENT_LOG entries.
            if log.len() > MAX_EVENT_LOG {
                let drain_count = log.len() - MAX_EVENT_LOG;
                log.drain(..drain_count);
            }
        }

        events
    }

    /// Execute a recovery command via the system shell.
    ///
    /// The command string is split on whitespace with the first token as
    /// the program and the rest as arguments. For SSH commands this is
    /// typically `ssh <host> "<remote command>"`.
    async fn execute_recovery(command: &str) -> anyhow::Result<String> {
        let output = if cfg!(target_os = "windows") {
            tokio::process::Command::new("cmd")
                .args(["/C", command])
                .output()
                .await?
        } else {
            tokio::process::Command::new("sh")
                .args(["-c", command])
                .output()
                .await?
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            Ok(stdout)
        } else {
            anyhow::bail!(
                "Command exited with {}: stdout={}, stderr={}",
                output.status,
                stdout.trim(),
                stderr.trim()
            )
        }
    }

    /// Check whether a recovery attempt should be made for the given worker.
    pub async fn should_recover(&self, worker_name: &str) -> bool {
        match self.configs.get(worker_name) {
            Some(config) => self.should_recover_inner(worker_name, config).await,
            None => false,
        }
    }

    /// Internal: check cooldown and retry limits.
    async fn should_recover_inner(&self, worker_name: &str, config: &RecoveryConfig) -> bool {
        if !config.enabled {
            return false;
        }

        let states = self.states.lock().await;
        let state = match states.get(worker_name) {
            Some(s) => s,
            None => return true, // no state yet — first attempt is always OK
        };

        // Exceeded max retries?
        if state.attempts >= config.max_retries {
            return false;
        }

        // Cooldown not yet elapsed?
        if let Some(last) = state.last_attempt {
            let elapsed = last.elapsed().as_secs();
            if elapsed < config.cooldown_secs {
                return false;
            }
        }

        true
    }

    // -- query methods ------------------------------------------------------

    /// Return the most recent events, up to `limit`.
    pub async fn recent_events(&self, limit: usize) -> Vec<WatchdogEvent> {
        let log = self.event_log.lock().await;
        let start = if log.len() > limit {
            log.len() - limit
        } else {
            0
        };
        log[start..].to_vec()
    }

    /// Reset recovery state for a specific worker (e.g. after manual intervention).
    pub async fn reset_worker(&self, worker_name: &str) {
        let mut states = self.states.lock().await;
        states.remove(worker_name);
        info!(worker = %worker_name, "Watchdog recovery state reset");
    }

    /// Produce a serializable status snapshot for all configured workers.
    pub async fn status_snapshot(&self) -> Vec<WatchdogStatus> {
        let states = self.states.lock().await;
        let mut out = Vec::with_capacity(self.configs.len());

        for (name, config) in &self.configs {
            let (attempts, last_attempt_ago_secs, state_label) =
                match states.get(name.as_str()) {
                    Some(s) => {
                        let ago = s.last_attempt.map(|t| t.elapsed().as_secs());
                        let label = if s.attempts >= config.max_retries {
                            "exhausted"
                        } else if s.attempts > 0 {
                            "recovering"
                        } else {
                            "healthy"
                        };
                        (s.attempts, ago, label)
                    }
                    None => (0, None, "healthy"),
                };

            out.push(WatchdogStatus {
                worker_name: name.clone(),
                recovery_enabled: config.enabled,
                attempts,
                max_retries: config.max_retries,
                last_attempt_ago_secs,
                state: state_label.to_string(),
            });
        }

        // Sort by name for deterministic output.
        out.sort_by(|a, b| a.worker_name.cmp(&b.worker_name));
        out
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Helper: build a minimal `ClusterNode` for testing.
    fn make_node(name: &str, status: &str) -> ClusterNode {
        ClusterNode {
            name: name.to_string(),
            host: "127.0.0.1".to_string(),
            port: 7878,
            status: status.to_string(),
            models: vec![],
            last_seen: "2026-03-15T00:00:00Z".to_string(),
            capabilities: vec!["tools".to_string()],
            device_type: "light".to_string(),
            cpu_load: 0.0,
        }
    }

    #[tokio::test]
    async fn test_new_watchdog() {
        let wd = WorkerWatchdog::new();
        assert!(wd.configs.is_empty());
        let snap = wd.status_snapshot().await;
        assert!(snap.is_empty());
        let events = wd.recent_events(10).await;
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_add_worker() {
        let mut wd = WorkerWatchdog::new();
        wd.add_worker(RecoveryConfig::new("test-worker", "echo hello"));
        assert_eq!(wd.configs.len(), 1);
        assert!(wd.configs.contains_key("test-worker"));

        let config = &wd.configs["test-worker"];
        assert_eq!(config.worker_name, "test-worker");
        assert_eq!(config.recovery_command, "echo hello");
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.cooldown_secs, 120);
        assert!(config.enabled);

        let snap = wd.status_snapshot().await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].worker_name, "test-worker");
        assert_eq!(snap[0].state, "healthy");
    }

    #[tokio::test]
    async fn test_should_recover_cooldown() {
        let mut wd = WorkerWatchdog::new();
        let mut config = RecoveryConfig::new("worker-a", "echo recover");
        config.cooldown_secs = 1; // 1 second for fast test
        config.max_retries = 5;
        wd.add_worker(config);

        // First attempt: should be allowed (no state).
        assert!(wd.should_recover("worker-a").await);

        // Simulate an attempt just now.
        {
            let mut states = wd.states.lock().await;
            states.insert(
                "worker-a".to_string(),
                RecoveryState {
                    attempts: 1,
                    last_attempt: Some(Instant::now()),
                    last_success: None,
                    consecutive_failures: 1,
                },
            );
        }

        // Should NOT recover — cooldown not elapsed.
        assert!(!wd.should_recover("worker-a").await);

        // Simulate that the attempt was long enough ago.
        {
            let mut states = wd.states.lock().await;
            let state = states.get_mut("worker-a").unwrap();
            // Set last_attempt to 2 seconds ago (past the 1s cooldown).
            state.last_attempt = Instant::now().checked_sub(Duration::from_secs(2));
        }

        // Now it should be allowed.
        assert!(wd.should_recover("worker-a").await);
    }

    #[tokio::test]
    async fn test_should_recover_max_retries() {
        let mut wd = WorkerWatchdog::new();
        let mut config = RecoveryConfig::new("worker-b", "echo recover");
        config.max_retries = 2;
        config.cooldown_secs = 0;
        wd.add_worker(config);

        // Set attempts to max_retries.
        {
            let mut states = wd.states.lock().await;
            states.insert(
                "worker-b".to_string(),
                RecoveryState {
                    attempts: 2,
                    last_attempt: Instant::now().checked_sub(Duration::from_secs(9999)),
                    last_success: None,
                    consecutive_failures: 2,
                },
            );
        }

        // Should NOT recover — max retries exhausted.
        assert!(!wd.should_recover("worker-b").await);

        // Unknown worker should return false.
        assert!(!wd.should_recover("nonexistent").await);
    }

    #[tokio::test]
    async fn test_default_workers() {
        let wd = WorkerWatchdog::with_defaults();
        assert_eq!(wd.configs.len(), 2);
        assert!(wd.configs.contains_key("acer"));
        assert!(wd.configs.contains_key("m1-mac"));

        let acer = &wd.configs["acer"];
        assert!(acer.recovery_command.contains("192.168.1.115"));
        assert!(acer.recovery_command.contains("python.exe"));
        assert!(acer.enabled);

        let m1 = &wd.configs["m1-mac"];
        assert!(m1.recovery_command.contains("100.87.93.58"));
        assert!(m1.recovery_command.contains("worker.py"));
        assert!(m1.enabled);

        let snap = wd.status_snapshot().await;
        assert_eq!(snap.len(), 2);
        // Sorted by name: acer < m1-mac
        assert_eq!(snap[0].worker_name, "acer");
        assert_eq!(snap[1].worker_name, "m1-mac");
    }

    #[tokio::test]
    async fn test_recovery_state_reset_on_online() {
        let mut wd = WorkerWatchdog::new();
        let mut config = RecoveryConfig::new("worker-c", "echo recover");
        config.cooldown_secs = 0;
        wd.add_worker(config);

        // Simulate a prior recovery attempt.
        {
            let mut states = wd.states.lock().await;
            states.insert(
                "worker-c".to_string(),
                RecoveryState {
                    attempts: 2,
                    last_attempt: Instant::now().checked_sub(Duration::from_secs(300)),
                    last_success: None,
                    consecutive_failures: 2,
                },
            );
        }

        // Pass in an online worker — state should reset.
        let workers = vec![make_node("worker-c", "online")];
        let events = wd.check_and_recover(&workers).await;

        // Should emit a RecoverySuccess event.
        assert!(
            events.iter().any(|e| matches!(e, WatchdogEvent::RecoverySuccess { worker, .. } if worker == "worker-c")),
            "Expected RecoverySuccess event, got: {:?}",
            events
        );

        // State should now be reset.
        let states = wd.states.lock().await;
        let state = states.get("worker-c").unwrap();
        assert_eq!(state.attempts, 0);
        assert_eq!(state.consecutive_failures, 0);
        assert!(state.last_success.is_some());
    }

    #[tokio::test]
    async fn test_event_log_limit() {
        let wd = WorkerWatchdog::new();

        // Manually push more than MAX_EVENT_LOG events.
        {
            let mut log = wd.event_log.lock().await;
            for i in 0..150 {
                log.push(WatchdogEvent::WorkerDown {
                    worker: format!("w-{}", i),
                    since: "2026-01-01T00:00:00Z".to_string(),
                });
            }
            // Simulate the trim that check_and_recover does.
            if log.len() > MAX_EVENT_LOG {
                let drain_count = log.len() - MAX_EVENT_LOG;
                log.drain(..drain_count);
            }
        }

        let log = wd.event_log.lock().await;
        assert_eq!(log.len(), MAX_EVENT_LOG);

        // The earliest remaining event should be w-50 (indices 50..149 = 100 items).
        if let WatchdogEvent::WorkerDown { worker, .. } = &log[0] {
            assert_eq!(worker, "w-50");
        } else {
            panic!("Expected WorkerDown event");
        }

        drop(log);

        // recent_events with a smaller limit.
        let recent = wd.recent_events(5).await;
        assert_eq!(recent.len(), 5);
        // Last event should be w-149.
        if let WatchdogEvent::WorkerDown { worker, .. } = &recent[4] {
            assert_eq!(worker, "w-149");
        } else {
            panic!("Expected WorkerDown event");
        }
    }

    #[tokio::test]
    async fn test_reset_worker() {
        let mut wd = WorkerWatchdog::new();
        wd.add_worker(RecoveryConfig::new("worker-d", "echo hi"));

        // Add some state.
        {
            let mut states = wd.states.lock().await;
            states.insert(
                "worker-d".to_string(),
                RecoveryState {
                    attempts: 3,
                    last_attempt: Some(Instant::now()),
                    last_success: None,
                    consecutive_failures: 3,
                },
            );
        }

        // Verify exhausted state.
        assert!(!wd.should_recover("worker-d").await);

        // Reset.
        wd.reset_worker("worker-d").await;

        // Should be recoverable again.
        assert!(wd.should_recover("worker-d").await);

        // State should be gone.
        let states = wd.states.lock().await;
        assert!(!states.contains_key("worker-d"));
    }

    #[tokio::test]
    async fn test_status_snapshot_states() {
        let mut wd = WorkerWatchdog::new();
        let mut cfg_a = RecoveryConfig::new("alpha", "echo a");
        cfg_a.max_retries = 3;
        wd.add_worker(cfg_a);

        let mut cfg_b = RecoveryConfig::new("beta", "echo b");
        cfg_b.max_retries = 2;
        wd.add_worker(cfg_b);

        // alpha: 1 attempt (recovering), beta: 2 attempts with max=2 (exhausted)
        {
            let mut states = wd.states.lock().await;
            states.insert(
                "alpha".to_string(),
                RecoveryState {
                    attempts: 1,
                    last_attempt: Some(Instant::now()),
                    last_success: None,
                    consecutive_failures: 1,
                },
            );
            states.insert(
                "beta".to_string(),
                RecoveryState {
                    attempts: 2,
                    last_attempt: Some(Instant::now()),
                    last_success: None,
                    consecutive_failures: 2,
                },
            );
        }

        let snap = wd.status_snapshot().await;
        assert_eq!(snap.len(), 2);

        let alpha_status = snap.iter().find(|s| s.worker_name == "alpha").unwrap();
        assert_eq!(alpha_status.state, "recovering");
        assert_eq!(alpha_status.attempts, 1);
        assert!(alpha_status.last_attempt_ago_secs.is_some());

        let beta_status = snap.iter().find(|s| s.worker_name == "beta").unwrap();
        assert_eq!(beta_status.state, "exhausted");
        assert_eq!(beta_status.attempts, 2);
    }

    #[tokio::test]
    async fn test_disabled_worker_skipped() {
        let mut wd = WorkerWatchdog::new();
        let mut config = RecoveryConfig::new("disabled-worker", "echo should-not-run");
        config.enabled = false;
        wd.add_worker(config);

        let workers = vec![make_node("disabled-worker", "offline")];
        let events = wd.check_and_recover(&workers).await;

        // No events should be emitted for a disabled worker.
        assert!(events.is_empty());
    }

    #[test]
    fn test_watchdog_event_serialization() {
        let event = WatchdogEvent::RecoveryAttempted {
            worker: "acer".to_string(),
            attempt: 1,
            command: "ssh test".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("RecoveryAttempted"));
        assert!(json.contains("acer"));

        // Round-trip.
        let deserialized: WatchdogEvent = serde_json::from_str(&json).unwrap();
        if let WatchdogEvent::RecoveryAttempted {
            worker, attempt, ..
        } = deserialized
        {
            assert_eq!(worker, "acer");
            assert_eq!(attempt, 1);
        } else {
            panic!("Unexpected variant after deserialization");
        }
    }
}
