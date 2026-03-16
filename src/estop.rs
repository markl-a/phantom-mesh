//! Emergency stop (E-Stop) mechanism.
//! Uses an AtomicBool to signal all agent loops and tool executions to halt.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Global emergency stop flag.
/// Check `is_stopped()` before each agent round and tool execution.
#[derive(Clone)]
pub struct EStop {
    stopped: Arc<AtomicBool>,
}

impl EStop {
    pub fn new() -> Self {
        Self {
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Activate emergency stop
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        warn!("E-STOP ACTIVATED — all agent operations will halt");
    }

    /// Deactivate emergency stop (resume normal operation)
    pub fn reset(&self) {
        self.stopped.store(false, Ordering::SeqCst);
        info!("E-STOP reset — resuming normal operation");
    }

    /// Check if emergency stop is active
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// Return Err if stopped, for use in agent loops
    pub fn check(&self) -> Result<(), EStopError> {
        if self.is_stopped() {
            Err(EStopError)
        } else {
            Ok(())
        }
    }
}

impl Default for EStop {
    fn default() -> Self {
        Self::new()
    }
}

/// Error returned when an operation is halted by E-Stop
#[derive(Debug, Clone)]
pub struct EStopError;

impl std::fmt::Display for EStopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Operation halted: emergency stop is active")
    }
}

impl std::error::Error for EStopError {}

/// Safety heartbeat — tracks liveness of running agents.
/// Agents call `beat()` each round; external monitors call `stale_agents()`
/// to detect agents that haven't reported within a timeout.
#[derive(Clone)]
pub struct Heartbeat {
    inner: Arc<RwLock<HashMap<String, Instant>>>,
    /// Maximum time without a heartbeat before an agent is considered stale
    pub timeout: Duration,
}

impl Heartbeat {
    pub fn new(timeout: Duration) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            timeout,
        }
    }

    /// Record a heartbeat for the given agent
    pub fn beat(&self, agent_name: &str) {
        let mut map = self.inner.write().unwrap();
        map.insert(agent_name.to_string(), Instant::now());
    }

    /// Remove an agent from heartbeat tracking (e.g., when it finishes)
    pub fn remove(&self, agent_name: &str) {
        let mut map = self.inner.write().unwrap();
        map.remove(agent_name);
    }

    /// Return the list of agents that haven't sent a heartbeat within the timeout
    pub fn stale_agents(&self) -> Vec<String> {
        let map = self.inner.read().unwrap();
        let now = Instant::now();
        map.iter()
            .filter(|(_, last)| now.duration_since(**last) > self.timeout)
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Check if a specific agent is stale
    pub fn is_stale(&self, agent_name: &str) -> bool {
        let map = self.inner.read().unwrap();
        match map.get(agent_name) {
            Some(last) => Instant::now().duration_since(*last) > self.timeout,
            None => false, // unknown agent is not considered stale
        }
    }

    /// Number of tracked agents
    pub fn tracked_count(&self) -> usize {
        let map = self.inner.read().unwrap();
        map.len()
    }
}

impl Default for Heartbeat {
    fn default() -> Self {
        Self::new(Duration::from_secs(120)) // 2 minute default timeout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state() {
        let estop = EStop::new();
        assert!(!estop.is_stopped());
        assert!(estop.check().is_ok());
    }

    #[test]
    fn test_stop() {
        let estop = EStop::new();
        estop.stop();
        assert!(estop.is_stopped());
        assert!(estop.check().is_err());
    }

    #[test]
    fn test_reset() {
        let estop = EStop::new();
        estop.stop();
        assert!(estop.is_stopped());
        estop.reset();
        assert!(!estop.is_stopped());
        assert!(estop.check().is_ok());
    }

    #[test]
    fn test_clone_shares_state() {
        let estop1 = EStop::new();
        let estop2 = estop1.clone();

        estop1.stop();
        assert!(estop2.is_stopped());

        estop2.reset();
        assert!(!estop1.is_stopped());
    }

    #[test]
    fn test_thread_safety() {
        let estop = EStop::new();
        let estop2 = estop.clone();

        let handle = std::thread::spawn(move || {
            estop2.stop();
        });

        handle.join().unwrap();
        assert!(estop.is_stopped());
    }

    #[test]
    fn test_estop_error_display() {
        let err = EStopError;
        assert!(err.to_string().contains("emergency stop"));
    }

    #[test]
    fn test_default() {
        let estop = EStop::default();
        assert!(!estop.is_stopped());
    }

    #[test]
    fn test_heartbeat_basic() {
        let hb = Heartbeat::new(Duration::from_secs(60));
        assert_eq!(hb.tracked_count(), 0);
        hb.beat("agent-1");
        assert_eq!(hb.tracked_count(), 1);
        assert!(!hb.is_stale("agent-1"));
        assert!(hb.stale_agents().is_empty());
    }

    #[test]
    fn test_heartbeat_stale_detection() {
        // Use a very short timeout so we can test staleness
        let hb = Heartbeat::new(Duration::from_millis(10));
        hb.beat("agent-slow");
        std::thread::sleep(Duration::from_millis(20));
        assert!(hb.is_stale("agent-slow"));
        let stale = hb.stale_agents();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0], "agent-slow");
    }

    #[test]
    fn test_heartbeat_remove() {
        let hb = Heartbeat::new(Duration::from_secs(60));
        hb.beat("agent-done");
        assert_eq!(hb.tracked_count(), 1);
        hb.remove("agent-done");
        assert_eq!(hb.tracked_count(), 0);
    }

    #[test]
    fn test_heartbeat_refresh() {
        let hb = Heartbeat::new(Duration::from_millis(50));
        hb.beat("agent-active");
        std::thread::sleep(Duration::from_millis(30));
        hb.beat("agent-active"); // refresh
        std::thread::sleep(Duration::from_millis(30));
        // 60ms total but last beat was 30ms ago, timeout is 50ms → not stale
        assert!(!hb.is_stale("agent-active"));
    }

    #[test]
    fn test_heartbeat_unknown_agent_not_stale() {
        let hb = Heartbeat::new(Duration::from_millis(1));
        assert!(!hb.is_stale("nonexistent"));
    }

    #[test]
    fn test_heartbeat_clone_shares_state() {
        let hb1 = Heartbeat::new(Duration::from_secs(60));
        let hb2 = hb1.clone();
        hb1.beat("agent-x");
        assert_eq!(hb2.tracked_count(), 1);
    }
}
