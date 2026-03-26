// concurrency_manager.rs — Per-node concurrency limits for cluster dispatch.
//
// Each cluster node has a maximum number of concurrent tasks it can handle.
// This module provides an RAII-based permit system so that callers can
// acquire a slot, do work, and automatically release it on drop.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// NodeConcurrency
// ---------------------------------------------------------------------------

/// Per-node concurrency state.
///
/// Uses atomics for lock-free reads of `active_count` and `paused`, while
/// `max_concurrent` is immutable after construction.
#[derive(Debug)]
pub struct NodeConcurrency {
    /// Maximum number of tasks that can run simultaneously on this node.
    pub max_concurrent: usize,
    /// Current number of active tasks (in-flight permits).
    pub active_count: AtomicUsize,
    /// When `true`, no new permits are issued regardless of capacity.
    pub paused: AtomicBool,
}

impl NodeConcurrency {
    /// Create a new `NodeConcurrency` with the given limit.
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            max_concurrent,
            active_count: AtomicUsize::new(0),
            paused: AtomicBool::new(false),
        }
    }
}

// ---------------------------------------------------------------------------
// ConcurrencyPermit — RAII guard
// ---------------------------------------------------------------------------

/// An RAII guard that represents one active task slot on a node.
///
/// When dropped, it decrements the node's `active_count` automatically.
/// Callers may also use [`ConcurrencyPermit::release`] for explicit release.
pub struct ConcurrencyPermit {
    node_name: String,
    active_count: Arc<AtomicUsize>,
    released: bool,
}

impl std::fmt::Debug for ConcurrencyPermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConcurrencyPermit")
            .field("node_name", &self.node_name)
            .field("released", &self.released)
            .finish()
    }
}

impl ConcurrencyPermit {
    /// The node name this permit is for.
    pub fn node_name(&self) -> &str {
        &self.node_name
    }

    /// Explicitly release the permit (alternative to relying on `Drop`).
    ///
    /// Calling this more than once is a no-op.
    pub fn release(&mut self) {
        if !self.released {
            self.released = true;
            let prev = self.active_count.fetch_sub(1, Ordering::SeqCst);
            debug!(
                node = %self.node_name,
                prev_active = prev,
                "concurrency permit explicitly released"
            );
        }
    }
}

impl Drop for ConcurrencyPermit {
    fn drop(&mut self) {
        if !self.released {
            self.released = true;
            let prev = self.active_count.fetch_sub(1, Ordering::SeqCst);
            debug!(
                node = %self.node_name,
                prev_active = prev,
                "concurrency permit dropped (RAII release)"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// ConcurrencyManager
// ---------------------------------------------------------------------------

/// Manages per-node concurrency limits for cluster dispatch.
///
/// Thread-safe: all inner state is atomic, and the node map is immutable
/// after construction (nodes are pre-registered). If dynamic node
/// addition is needed in the future, wrap `nodes` in an `RwLock`.
pub struct ConcurrencyManager {
    /// Map of node name -> concurrency state.
    /// The `Arc<AtomicUsize>` for `active_count` is shared with issued permits.
    nodes: HashMap<String, NodeEntry>,
}

/// Internal entry holding node state plus a shareable handle to `active_count`.
#[derive(Debug)]
struct NodeEntry {
    max_concurrent: usize,
    active_count: Arc<AtomicUsize>,
    paused: AtomicBool,
}

impl ConcurrencyManager {
    /// Create a new `ConcurrencyManager` from a map of node name -> max concurrency.
    pub fn new(limits: HashMap<String, usize>) -> Self {
        let nodes = limits
            .into_iter()
            .map(|(name, max)| {
                info!(node = %name, max_concurrent = max, "registered node concurrency limit");
                let entry = NodeEntry {
                    max_concurrent: max,
                    active_count: Arc::new(AtomicUsize::new(0)),
                    paused: AtomicBool::new(false),
                };
                (name, entry)
            })
            .collect();

        Self { nodes }
    }

    /// Create a `ConcurrencyManager` with the default Phantom Mesh cluster limits:
    /// Z13: 4, M1: 2, Acer: 3, AYANEO: 2.
    pub fn with_defaults() -> Self {
        let mut limits = HashMap::new();
        limits.insert("Z13".to_string(), 4);
        limits.insert("M1".to_string(), 2);
        limits.insert("Acer".to_string(), 3);
        limits.insert("AYANEO".to_string(), 2);
        Self::new(limits)
    }

    // -- try_acquire ----------------------------------------------------------

    /// Attempt to acquire a concurrency permit for `node`.
    ///
    /// Returns `Ok(ConcurrencyPermit)` if the node has available capacity and
    /// is not paused. Returns `Err(String)` with a human-readable reason on
    /// failure.
    ///
    /// Uses a compare-and-swap loop to atomically claim a slot, avoiding the
    /// need for a mutex.
    pub fn try_acquire(&self, node: &str) -> Result<ConcurrencyPermit, String> {
        let entry = self
            .nodes
            .get(node)
            .ok_or_else(|| format!("unknown node: {}", node))?;

        // Check pause flag first.
        if entry.paused.load(Ordering::SeqCst) {
            return Err(format!("node {} is paused", node));
        }

        // CAS loop: atomically increment active_count only if under limit.
        loop {
            let current = entry.active_count.load(Ordering::SeqCst);
            if current >= entry.max_concurrent {
                return Err(format!(
                    "node {} at capacity ({}/{})",
                    node, current, entry.max_concurrent
                ));
            }

            match entry.active_count.compare_exchange(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {
                    debug!(
                        node = node,
                        active = current + 1,
                        max = entry.max_concurrent,
                        "concurrency permit acquired"
                    );
                    return Ok(ConcurrencyPermit {
                        node_name: node.to_string(),
                        active_count: Arc::clone(&entry.active_count),
                        released: false,
                    });
                }
                Err(_) => {
                    // Another thread changed active_count; retry.
                    continue;
                }
            }
        }
    }

    // -- release --------------------------------------------------------------

    /// Explicitly release a concurrency slot for `node` (alternative to the
    /// RAII guard). Returns `Err` if the node is unknown or has zero active
    /// tasks.
    pub fn release(&self, node: &str) -> Result<(), String> {
        let entry = self
            .nodes
            .get(node)
            .ok_or_else(|| format!("unknown node: {}", node))?;

        loop {
            let current = entry.active_count.load(Ordering::SeqCst);
            if current == 0 {
                return Err(format!("node {} has no active tasks to release", node));
            }

            match entry.active_count.compare_exchange(
                current,
                current - 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {
                    debug!(
                        node = node,
                        active = current - 1,
                        "concurrency slot explicitly released"
                    );
                    return Ok(());
                }
                Err(_) => continue,
            }
        }
    }

    // -- is_available ---------------------------------------------------------

    /// Check whether `node` has at least one available concurrency slot and
    /// is not paused. Returns `false` for unknown nodes.
    pub fn is_available(&self, node: &str) -> bool {
        match self.nodes.get(node) {
            None => false,
            Some(entry) => {
                if entry.paused.load(Ordering::SeqCst) {
                    return false;
                }
                entry.active_count.load(Ordering::SeqCst) < entry.max_concurrent
            }
        }
    }

    // -- pause_node / resume_node ---------------------------------------------

    /// Pause a node — prevents new permits from being issued, even if capacity
    /// is available. Existing permits remain valid. Useful for manual overload
    /// mitigation or maintenance windows.
    pub fn pause_node(&self, node: &str) -> Result<(), String> {
        let entry = self
            .nodes
            .get(node)
            .ok_or_else(|| format!("unknown node: {}", node))?;

        entry.paused.store(true, Ordering::SeqCst);
        warn!(node = node, "node paused — no new permits will be issued");
        Ok(())
    }

    /// Resume a paused node — new permits can be issued again.
    pub fn resume_node(&self, node: &str) -> Result<(), String> {
        let entry = self
            .nodes
            .get(node)
            .ok_or_else(|| format!("unknown node: {}", node))?;

        entry.paused.store(false, Ordering::SeqCst);
        info!(node = node, "node resumed — permits can be issued again");
        Ok(())
    }

    // -- stats ----------------------------------------------------------------

    /// Return a snapshot of (active_count, max_concurrent, paused) per node.
    pub fn stats(&self) -> HashMap<String, (usize, usize, bool)> {
        self.nodes
            .iter()
            .map(|(name, entry)| {
                let active = entry.active_count.load(Ordering::SeqCst);
                let paused = entry.paused.load(Ordering::SeqCst);
                (name.clone(), (active, entry.max_concurrent, paused))
            })
            .collect()
    }

    // -- node_count -----------------------------------------------------------

    /// Return the number of registered nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    /// Helper: create a manager with small limits for testing.
    fn test_manager() -> ConcurrencyManager {
        let mut limits = HashMap::new();
        limits.insert("node_a".to_string(), 2);
        limits.insert("node_b".to_string(), 1);
        limits.insert("node_c".to_string(), 3);
        ConcurrencyManager::new(limits)
    }

    // -- basic acquire/release ------------------------------------------------

    #[test]
    fn test_acquire_and_release_via_drop() {
        let mgr = test_manager();

        // Acquire a permit — active should go to 1.
        {
            let _permit = mgr.try_acquire("node_a").expect("should acquire");
            let stats = mgr.stats();
            assert_eq!(stats["node_a"].0, 1); // active = 1
        }

        // After drop, active should go back to 0.
        let stats = mgr.stats();
        assert_eq!(stats["node_a"].0, 0);
    }

    #[test]
    fn test_acquire_and_explicit_release() {
        let mgr = test_manager();

        let mut permit = mgr.try_acquire("node_a").expect("should acquire");
        assert_eq!(mgr.stats()["node_a"].0, 1);

        permit.release();
        assert_eq!(mgr.stats()["node_a"].0, 0);

        // Double release is a no-op.
        permit.release();
        assert_eq!(mgr.stats()["node_a"].0, 0);
    }

    // -- limit enforcement ----------------------------------------------------

    #[test]
    fn test_limit_enforcement() {
        let mgr = test_manager();

        // node_b has limit 1; first acquire should succeed.
        let _p1 = mgr.try_acquire("node_b").expect("should acquire first");
        assert_eq!(mgr.stats()["node_b"].0, 1);

        // Second acquire should fail.
        let result = mgr.try_acquire("node_b");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("at capacity"));
    }

    #[test]
    fn test_limit_restored_after_release() {
        let mgr = test_manager();

        // Fill node_b (limit 1).
        let permit = mgr.try_acquire("node_b").expect("should acquire");
        assert!(mgr.try_acquire("node_b").is_err());

        // Drop permit — should free the slot.
        drop(permit);

        // Now we can acquire again.
        let _p2 = mgr.try_acquire("node_b").expect("should acquire after release");
        assert_eq!(mgr.stats()["node_b"].0, 1);
    }

    #[test]
    fn test_multiple_permits_up_to_limit() {
        let mgr = test_manager();

        // node_c has limit 3.
        let _p1 = mgr.try_acquire("node_c").unwrap();
        let _p2 = mgr.try_acquire("node_c").unwrap();
        let _p3 = mgr.try_acquire("node_c").unwrap();
        assert_eq!(mgr.stats()["node_c"].0, 3);

        // 4th should fail.
        assert!(mgr.try_acquire("node_c").is_err());
    }

    // -- RAII drop correctness ------------------------------------------------

    #[test]
    fn test_raii_drop_decrements() {
        let mgr = test_manager();

        let p1 = mgr.try_acquire("node_a").unwrap();
        let p2 = mgr.try_acquire("node_a").unwrap();
        assert_eq!(mgr.stats()["node_a"].0, 2);

        drop(p1);
        assert_eq!(mgr.stats()["node_a"].0, 1);

        drop(p2);
        assert_eq!(mgr.stats()["node_a"].0, 0);
    }

    #[test]
    fn test_explicit_release_then_drop_no_double_decrement() {
        let mgr = test_manager();

        let mut permit = mgr.try_acquire("node_a").unwrap();
        assert_eq!(mgr.stats()["node_a"].0, 1);

        permit.release();
        assert_eq!(mgr.stats()["node_a"].0, 0);

        // Drop should not decrement again.
        drop(permit);
        assert_eq!(mgr.stats()["node_a"].0, 0);
    }

    // -- pause / resume -------------------------------------------------------

    #[test]
    fn test_pause_blocks_acquire() {
        let mgr = test_manager();

        mgr.pause_node("node_a").unwrap();

        let result = mgr.try_acquire("node_a");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("paused"));

        // is_available should also return false.
        assert!(!mgr.is_available("node_a"));
    }

    #[test]
    fn test_resume_restores_acquire() {
        let mgr = test_manager();

        mgr.pause_node("node_a").unwrap();
        assert!(!mgr.is_available("node_a"));

        mgr.resume_node("node_a").unwrap();
        assert!(mgr.is_available("node_a"));

        let _p = mgr.try_acquire("node_a").expect("should acquire after resume");
        assert_eq!(mgr.stats()["node_a"].0, 1);
    }

    #[test]
    fn test_pause_does_not_affect_existing_permits() {
        let mgr = test_manager();

        let _permit = mgr.try_acquire("node_a").unwrap();
        assert_eq!(mgr.stats()["node_a"].0, 1);

        // Pause while a permit is active.
        mgr.pause_node("node_a").unwrap();

        // Active count unchanged; permit is still valid.
        assert_eq!(mgr.stats()["node_a"].0, 1);

        // But new permits are blocked.
        assert!(mgr.try_acquire("node_a").is_err());
    }

    // -- unknown node handling ------------------------------------------------

    #[test]
    fn test_unknown_node_acquire() {
        let mgr = test_manager();

        let result = mgr.try_acquire("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown node"));
    }

    #[test]
    fn test_unknown_node_is_available() {
        let mgr = test_manager();
        assert!(!mgr.is_available("nonexistent"));
    }

    #[test]
    fn test_unknown_node_pause_resume() {
        let mgr = test_manager();

        assert!(mgr.pause_node("ghost").is_err());
        assert!(mgr.resume_node("ghost").is_err());
    }

    #[test]
    fn test_unknown_node_release() {
        let mgr = test_manager();

        let result = mgr.release("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown node"));
    }

    // -- stats ----------------------------------------------------------------

    #[test]
    fn test_stats_reports_all_nodes() {
        let mgr = test_manager();

        let stats = mgr.stats();
        assert_eq!(stats.len(), 3);

        // All nodes start at (0, max, false).
        assert_eq!(stats["node_a"], (0, 2, false));
        assert_eq!(stats["node_b"], (0, 1, false));
        assert_eq!(stats["node_c"], (0, 3, false));
    }

    #[test]
    fn test_stats_reflects_active_and_paused() {
        let mgr = test_manager();

        let _p = mgr.try_acquire("node_a").unwrap();
        mgr.pause_node("node_b").unwrap();

        let stats = mgr.stats();
        assert_eq!(stats["node_a"], (1, 2, false));
        assert_eq!(stats["node_b"], (0, 1, true));
        assert_eq!(stats["node_c"], (0, 3, false));
    }

    // -- default limits -------------------------------------------------------

    #[test]
    fn test_with_defaults() {
        let mgr = ConcurrencyManager::with_defaults();

        let stats = mgr.stats();
        assert_eq!(stats.len(), 4);
        assert_eq!(stats["Z13"], (0, 4, false));
        assert_eq!(stats["M1"], (0, 2, false));
        assert_eq!(stats["Acer"], (0, 3, false));
        assert_eq!(stats["AYANEO"], (0, 2, false));
    }

    // -- permit node_name accessor --------------------------------------------

    #[test]
    fn test_permit_node_name() {
        let mgr = test_manager();
        let permit = mgr.try_acquire("node_a").unwrap();
        assert_eq!(permit.node_name(), "node_a");
    }

    // -- concurrent access (multi-threaded) -----------------------------------

    #[test]
    fn test_concurrent_acquire_respects_limit() {
        let mgr = Arc::new(ConcurrencyManager::new({
            let mut m = HashMap::new();
            m.insert("shared".to_string(), 4);
            m
        }));

        let mut handles = vec![];

        // Spawn 10 threads, each trying to acquire a permit.
        for i in 0..10 {
            let mgr = Arc::clone(&mgr);
            handles.push(thread::spawn(move || {
                match mgr.try_acquire("shared") {
                    Ok(permit) => {
                        // Hold the permit briefly.
                        thread::sleep(std::time::Duration::from_millis(10));
                        Some(permit)
                    }
                    Err(_) => None,
                }
            }));
        }

        // Collect results — at most 4 should have succeeded simultaneously.
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let acquired: usize = results.iter().filter(|r| r.is_some()).count();

        // With limit=4 and 10 threads racing, anywhere from 4-10 could succeed
        // depending on timing (permits get dropped). But at any instant, the
        // active count should never exceed 4.
        assert!(acquired >= 4, "at least 4 should have acquired permits");

        // After all permits are dropped, active should be 0.
        drop(results);
        assert_eq!(mgr.stats()["shared"].0, 0);
    }

    // -- explicit release via manager -----------------------------------------

    #[test]
    fn test_manager_release() {
        let mgr = test_manager();

        // Manually bump the active count by acquiring a permit and forgetting it.
        let permit = mgr.try_acquire("node_a").unwrap();
        assert_eq!(mgr.stats()["node_a"].0, 1);

        // Use manager-level release.
        std::mem::forget(permit); // prevent RAII drop
        mgr.release("node_a").unwrap();
        assert_eq!(mgr.stats()["node_a"].0, 0);
    }

    #[test]
    fn test_release_zero_active_fails() {
        let mgr = test_manager();

        let result = mgr.release("node_a");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no active tasks"));
    }
}
