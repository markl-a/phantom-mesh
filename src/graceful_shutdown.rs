//! Graceful shutdown manager for the clawtex-core daemon.
//!
//! Provides coordinated shutdown with active-task tracking, RAII-based
//! task guards, configurable timeout, and OS signal handling (Ctrl+C / SIGTERM).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// ShutdownState
// ---------------------------------------------------------------------------

/// The lifecycle state of the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownState {
    /// Normal operation.
    Running,
    /// Shutdown has been requested; waiting for in-flight tasks to drain.
    ShuttingDown,
    /// All tasks finished (or timeout elapsed); process may exit.
    Terminated,
}

impl ShutdownState {
    /// Human-readable label.
    pub fn as_str(&self) -> &'static str {
        match self {
            ShutdownState::Running => "running",
            ShutdownState::ShuttingDown => "shutting_down",
            ShutdownState::Terminated => "terminated",
        }
    }
}

impl std::fmt::Display for ShutdownState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// ShutdownManager
// ---------------------------------------------------------------------------

/// Coordinates graceful shutdown across the daemon.
///
/// - Tracks the number of active tasks via an atomic counter.
/// - Uses `tokio::sync::Notify` to broadcast the shutdown signal.
/// - `register_task()` returns a [`TaskGuard`] that automatically decrements
///   the counter when dropped.
/// - `initiate_shutdown()` transitions the state, notifies all waiters, and
///   blocks until tasks drain or the timeout expires.
#[derive(Clone)]
pub struct ShutdownManager {
    shutdown_signal: Arc<Notify>,
    active_tasks: Arc<AtomicUsize>,
    shutdown_timeout: Duration,
    state: Arc<Mutex<ShutdownState>>,
    /// Notify fired every time a TaskGuard is dropped (task completes).
    task_done: Arc<Notify>,
}

impl ShutdownManager {
    /// Create a new `ShutdownManager` with the default 30-second timeout.
    pub fn new() -> Self {
        Self {
            shutdown_signal: Arc::new(Notify::new()),
            active_tasks: Arc::new(AtomicUsize::new(0)),
            shutdown_timeout: Duration::from_secs(30),
            state: Arc::new(Mutex::new(ShutdownState::Running)),
            task_done: Arc::new(Notify::new()),
        }
    }

    /// Create a `ShutdownManager` with a custom timeout.
    pub fn with_timeout(timeout: Duration) -> Self {
        let mut mgr = Self::new();
        mgr.shutdown_timeout = timeout;
        mgr
    }

    /// Register a new active task.
    ///
    /// Returns a [`TaskGuard`] whose `Drop` implementation decrements the
    /// active-task counter.  Returns `None` if the manager is already
    /// shutting down (callers should avoid starting new work).
    pub fn register_task(&self) -> Option<TaskGuard> {
        let st = self.state.lock().unwrap();
        if *st != ShutdownState::Running {
            return None;
        }
        drop(st);

        self.active_tasks.fetch_add(1, Ordering::SeqCst);
        Some(TaskGuard {
            active_tasks: Arc::clone(&self.active_tasks),
            task_done: Arc::clone(&self.task_done),
        })
    }

    /// Initiate a graceful shutdown.
    ///
    /// 1. Transitions state to `ShuttingDown`.
    /// 2. Notifies all waiters via the shutdown signal.
    /// 3. Waits until `active_tasks` reaches zero **or** the timeout elapses.
    /// 4. Transitions state to `Terminated`.
    ///
    /// Returns `true` if all tasks drained before the timeout, `false` if
    /// the timeout was reached.
    pub async fn initiate_shutdown(&self) -> bool {
        {
            let mut st = self.state.lock().unwrap();
            if *st != ShutdownState::Running {
                // Already shutting down or terminated.
                return *st == ShutdownState::Terminated;
            }
            *st = ShutdownState::ShuttingDown;
        }
        info!("Graceful shutdown initiated — waiting for active tasks to drain");
        self.shutdown_signal.notify_waiters();

        let drained = self.wait_for_drain().await;

        {
            let mut st = self.state.lock().unwrap();
            *st = ShutdownState::Terminated;
        }

        if drained {
            info!("All active tasks completed — shutdown complete");
        } else {
            warn!(
                "Shutdown timeout ({:?}) reached with {} tasks still active",
                self.shutdown_timeout,
                self.active_tasks.load(Ordering::SeqCst),
            );
        }

        drained
    }

    /// Returns `true` if the manager is in the `ShuttingDown` or `Terminated`
    /// state.
    pub fn is_shutting_down(&self) -> bool {
        let st = self.state.lock().unwrap();
        *st != ShutdownState::Running
    }

    /// Async wait until the shutdown signal is sent.
    ///
    /// Typically called by long-running background loops so they can exit
    /// cleanly when shutdown is requested.
    pub async fn wait_for_shutdown(&self) {
        if self.is_shutting_down() {
            return;
        }
        self.shutdown_signal.notified().await;
    }

    /// Number of currently active (in-flight) tasks.
    pub fn active_task_count(&self) -> usize {
        self.active_tasks.load(Ordering::SeqCst)
    }

    /// Force an immediate shutdown without waiting for tasks.
    ///
    /// Sets state to `Terminated` and notifies all waiters.
    pub fn force_shutdown(&self) {
        let remaining = self.active_tasks.load(Ordering::SeqCst);
        {
            let mut st = self.state.lock().unwrap();
            *st = ShutdownState::Terminated;
        }
        self.shutdown_signal.notify_waiters();
        warn!(
            "Force shutdown executed — {} tasks abandoned",
            remaining,
        );
    }

    /// Current shutdown state.
    pub fn state(&self) -> ShutdownState {
        let st = self.state.lock().unwrap();
        *st
    }

    /// The configured shutdown timeout.
    pub fn shutdown_timeout(&self) -> Duration {
        self.shutdown_timeout
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Poll until all active tasks complete or the timeout elapses.
    async fn wait_for_drain(&self) -> bool {
        let deadline = tokio::time::Instant::now() + self.shutdown_timeout;

        loop {
            if self.active_tasks.load(Ordering::SeqCst) == 0 {
                return true;
            }
            let remaining = deadline - tokio::time::Instant::now();
            if remaining.is_zero() {
                return false;
            }
            // Wait for either a task completion notification or the deadline.
            tokio::select! {
                _ = self.task_done.notified() => {
                    // A task finished — loop and re-check.
                }
                _ = tokio::time::sleep_until(deadline) => {
                    // Timeout expired.
                    return self.active_tasks.load(Ordering::SeqCst) == 0;
                }
            }
        }
    }
}

impl Default for ShutdownManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// TaskGuard
// ---------------------------------------------------------------------------

/// RAII guard returned by [`ShutdownManager::register_task`].
///
/// When this guard is dropped the active-task counter is decremented,
/// signalling that the task has finished.
pub struct TaskGuard {
    active_tasks: Arc<AtomicUsize>,
    task_done: Arc<Notify>,
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.active_tasks.fetch_sub(1, Ordering::SeqCst);
        self.task_done.notify_one();
    }
}

// ---------------------------------------------------------------------------
// Signal handler
// ---------------------------------------------------------------------------

/// Install OS signal handlers that trigger graceful shutdown.
///
/// On Unix: catches SIGTERM and SIGINT (Ctrl+C).
/// On Windows: catches Ctrl+C.
///
/// The handler calls `initiate_shutdown()` on the first signal and
/// `force_shutdown()` on the second.
pub fn install_signal_handler(manager: Arc<ShutdownManager>) {
    let mgr = Arc::clone(&manager);
    tokio::spawn(async move {
        let first = tokio::signal::ctrl_c().await;
        if first.is_ok() {
            info!("Received shutdown signal (Ctrl+C / SIGTERM)");
            if mgr.is_shutting_down() {
                // Second signal — force.
                mgr.force_shutdown();
            } else {
                mgr.initiate_shutdown().await;
            }
        }
    });

    // On Unix, also listen for SIGTERM separately.
    #[cfg(unix)]
    {
        let mgr2 = Arc::clone(&manager);
        tokio::spawn(async move {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("failed to install SIGTERM handler");
            sigterm.recv().await;
            info!("Received SIGTERM");
            if mgr2.is_shutting_down() {
                mgr2.force_shutdown();
            } else {
                mgr2.initiate_shutdown().await;
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- ShutdownState tests ------------------------------------------------

    #[test]
    fn test_shutdown_state_as_str() {
        assert_eq!(ShutdownState::Running.as_str(), "running");
        assert_eq!(ShutdownState::ShuttingDown.as_str(), "shutting_down");
        assert_eq!(ShutdownState::Terminated.as_str(), "terminated");
    }

    #[test]
    fn test_shutdown_state_display() {
        assert_eq!(format!("{}", ShutdownState::Running), "running");
        assert_eq!(format!("{}", ShutdownState::Terminated), "terminated");
    }

    #[test]
    fn test_shutdown_state_equality() {
        assert_eq!(ShutdownState::Running, ShutdownState::Running);
        assert_ne!(ShutdownState::Running, ShutdownState::ShuttingDown);
        assert_ne!(ShutdownState::ShuttingDown, ShutdownState::Terminated);
    }

    // -- ShutdownManager construction ---------------------------------------

    #[test]
    fn test_new_defaults() {
        let mgr = ShutdownManager::new();
        assert_eq!(mgr.state(), ShutdownState::Running);
        assert_eq!(mgr.active_task_count(), 0);
        assert!(!mgr.is_shutting_down());
        assert_eq!(mgr.shutdown_timeout(), Duration::from_secs(30));
    }

    #[test]
    fn test_with_timeout() {
        let mgr = ShutdownManager::with_timeout(Duration::from_secs(5));
        assert_eq!(mgr.shutdown_timeout(), Duration::from_secs(5));
        assert_eq!(mgr.state(), ShutdownState::Running);
    }

    #[test]
    fn test_default_trait() {
        let mgr = ShutdownManager::default();
        assert_eq!(mgr.state(), ShutdownState::Running);
        assert_eq!(mgr.shutdown_timeout(), Duration::from_secs(30));
    }

    // -- TaskGuard RAII -----------------------------------------------------

    #[test]
    fn test_register_task_increments_counter() {
        let mgr = ShutdownManager::new();
        let g1 = mgr.register_task();
        assert!(g1.is_some());
        assert_eq!(mgr.active_task_count(), 1);

        let g2 = mgr.register_task();
        assert!(g2.is_some());
        assert_eq!(mgr.active_task_count(), 2);

        drop(g1);
        assert_eq!(mgr.active_task_count(), 1);

        drop(g2);
        assert_eq!(mgr.active_task_count(), 0);
    }

    #[test]
    fn test_task_guard_drop_decrements() {
        let mgr = ShutdownManager::new();
        {
            let _g = mgr.register_task().unwrap();
            assert_eq!(mgr.active_task_count(), 1);
        }
        // Guard dropped at end of scope.
        assert_eq!(mgr.active_task_count(), 0);
    }

    #[test]
    fn test_register_task_rejected_during_shutdown() {
        let mgr = ShutdownManager::new();
        mgr.force_shutdown();
        assert!(mgr.register_task().is_none());
    }

    #[test]
    fn test_multiple_guards_scope() {
        let mgr = ShutdownManager::new();
        let guards: Vec<_> = (0..10).filter_map(|_| mgr.register_task()).collect();
        assert_eq!(mgr.active_task_count(), 10);
        drop(guards);
        assert_eq!(mgr.active_task_count(), 0);
    }

    // -- force_shutdown -----------------------------------------------------

    #[test]
    fn test_force_shutdown_sets_terminated() {
        let mgr = ShutdownManager::new();
        let _g = mgr.register_task();
        assert_eq!(mgr.active_task_count(), 1);

        mgr.force_shutdown();
        assert_eq!(mgr.state(), ShutdownState::Terminated);
        assert!(mgr.is_shutting_down());
        // Counter is NOT decremented — tasks are abandoned.
        assert_eq!(mgr.active_task_count(), 1);
    }

    // -- Clone shares state -------------------------------------------------

    #[test]
    fn test_clone_shares_state() {
        let mgr1 = ShutdownManager::new();
        let mgr2 = mgr1.clone();

        let _g = mgr1.register_task();
        assert_eq!(mgr2.active_task_count(), 1);

        mgr2.force_shutdown();
        assert!(mgr1.is_shutting_down());
    }

    // -- Async tests --------------------------------------------------------

    #[tokio::test]
    async fn test_initiate_shutdown_no_tasks() {
        let mgr = ShutdownManager::with_timeout(Duration::from_secs(1));
        let drained = mgr.initiate_shutdown().await;
        assert!(drained);
        assert_eq!(mgr.state(), ShutdownState::Terminated);
    }

    #[tokio::test]
    async fn test_initiate_shutdown_waits_for_tasks() {
        let mgr = ShutdownManager::with_timeout(Duration::from_secs(5));
        let guard = mgr.register_task().unwrap();

        let mgr_clone = mgr.clone();
        let handle = tokio::spawn(async move {
            mgr_clone.initiate_shutdown().await
        });

        // Simulate task work then complete.
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(guard);

        let drained = handle.await.unwrap();
        assert!(drained);
        assert_eq!(mgr.state(), ShutdownState::Terminated);
    }

    #[tokio::test]
    async fn test_initiate_shutdown_timeout() {
        let mgr = ShutdownManager::with_timeout(Duration::from_millis(50));
        let _guard = mgr.register_task().unwrap();

        let drained = mgr.initiate_shutdown().await;
        assert!(!drained);
        assert_eq!(mgr.state(), ShutdownState::Terminated);
        // Guard is still alive — task count is 1.
        assert_eq!(mgr.active_task_count(), 1);
    }

    #[tokio::test]
    async fn test_wait_for_shutdown_returns_immediately_when_stopped() {
        let mgr = ShutdownManager::new();
        mgr.force_shutdown();
        // Should not hang.
        mgr.wait_for_shutdown().await;
        assert!(mgr.is_shutting_down());
    }

    #[tokio::test]
    async fn test_wait_for_shutdown_wakes_on_signal() {
        let mgr = ShutdownManager::with_timeout(Duration::from_secs(1));
        let mgr_clone = mgr.clone();

        let waiter = tokio::spawn(async move {
            mgr_clone.wait_for_shutdown().await;
            true
        });

        // Give the waiter time to subscribe.
        tokio::time::sleep(Duration::from_millis(10)).await;
        mgr.initiate_shutdown().await;

        let woke = waiter.await.unwrap();
        assert!(woke);
    }

    #[tokio::test]
    async fn test_double_initiate_shutdown_is_idempotent() {
        let mgr = ShutdownManager::with_timeout(Duration::from_millis(50));
        mgr.initiate_shutdown().await;
        assert_eq!(mgr.state(), ShutdownState::Terminated);

        // Second call should return immediately.
        let result = mgr.initiate_shutdown().await;
        assert!(result); // Terminated => returns true
        assert_eq!(mgr.state(), ShutdownState::Terminated);
    }

    #[tokio::test]
    async fn test_concurrent_task_registration_and_shutdown() {
        let mgr = ShutdownManager::with_timeout(Duration::from_secs(2));
        let mut handles = Vec::new();

        // Spawn 20 tasks that each hold a guard for a short time.
        for i in 0..20 {
            let m = mgr.clone();
            handles.push(tokio::spawn(async move {
                if let Some(guard) = m.register_task() {
                    tokio::time::sleep(Duration::from_millis(10 + i * 5)).await;
                    drop(guard);
                }
            }));
        }

        // Small delay then initiate shutdown.
        tokio::time::sleep(Duration::from_millis(5)).await;
        let drained = mgr.initiate_shutdown().await;
        assert!(drained);
        assert_eq!(mgr.active_task_count(), 0);

        for h in handles {
            let _ = h.await;
        }
    }

    #[test]
    fn test_is_shutting_down_transitions() {
        let mgr = ShutdownManager::new();
        assert!(!mgr.is_shutting_down());

        // Manually set state to ShuttingDown.
        {
            let mut st = mgr.state.lock().unwrap();
            *st = ShutdownState::ShuttingDown;
        }
        assert!(mgr.is_shutting_down());

        // Transition to Terminated.
        {
            let mut st = mgr.state.lock().unwrap();
            *st = ShutdownState::Terminated;
        }
        assert!(mgr.is_shutting_down());
    }

    #[tokio::test]
    async fn test_install_signal_handler_does_not_panic() {
        // Verifies that installing the signal handler does not panic.
        // We cannot easily trigger Ctrl+C in tests, but we confirm
        // the handler spawns successfully.
        let mgr = Arc::new(ShutdownManager::new());
        install_signal_handler(Arc::clone(&mgr));

        // Manager should still be running.
        assert_eq!(mgr.state(), ShutdownState::Running);
    }
}
