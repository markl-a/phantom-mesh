//! Governor — automates policy lifecycle (Draft → Canary → Active → RolledBack).
//!
//! The Governor periodically inspects policies in Canary status, evaluates their
//! accumulated canary results, and either promotes them to Active, rejects them,
//! or leaves them alone if insufficient data has been collected.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use serde::Serialize;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::optimizer_store::{OptimizerStore, PolicyStatus};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Tuning knobs for the governor's decision-making.
#[derive(Debug, Clone, Serialize)]
pub struct GovernorConfig {
    /// Minimum number of canary results required before making a promotion
    /// or rejection decision.
    pub canary_min_runs: u32,
    /// Fraction of successful canary results required to promote (0.0–1.0).
    pub canary_success_threshold: f64,
    /// If the average quality score drops by more than this percentage relative
    /// to a perfect 1.0, the canary is rolled back / rejected.
    pub rollback_quality_drop_pct: f64,
    /// Maximum age (in days) of stale policy versions to consider for GC.
    pub gc_max_age_days: u32,
    /// Interval (in seconds) between background governor loop iterations.
    pub check_interval_secs: u64,
}

impl Default for GovernorConfig {
    fn default() -> Self {
        Self {
            canary_min_runs: 5,
            canary_success_threshold: 0.8,
            rollback_quality_drop_pct: 20.0,
            gc_max_age_days: 30,
            check_interval_secs: 3600,
        }
    }
}

// ---------------------------------------------------------------------------
// Canary result tracking
// ---------------------------------------------------------------------------

/// A single canary observation recorded via `record_canary_result`.
#[derive(Debug, Clone, Serialize)]
pub struct CanaryResult {
    pub success: bool,
    pub quality_score: f64,
}

// ---------------------------------------------------------------------------
// GovernorAction
// ---------------------------------------------------------------------------

/// Describes what the governor decided to do (or not do) for a given policy.
#[derive(Debug, Clone, Serialize)]
pub enum GovernorAction {
    Promoted {
        policy_id: String,
        from_version: i64,
        to_status: String,
    },
    RolledBack {
        policy_id: String,
        reason: String,
    },
    NoAction {
        policy_id: String,
        reason: String,
    },
}

// ---------------------------------------------------------------------------
// Governor
// ---------------------------------------------------------------------------

/// The Governor watches Canary-status policies, accumulates runtime canary
/// results, and automatically promotes or rejects them once enough evidence has
/// been gathered.
pub struct Governor {
    pub store: Arc<OptimizerStore>,
    pub config: GovernorConfig,
    /// In-memory tracking of canary results keyed by policy_id.
    /// We use `std::sync::Mutex` (not tokio) because the lock is held only
    /// briefly for HashMap inserts / reads.
    canary_results: Mutex<HashMap<String, Vec<CanaryResult>>>,
}

impl Governor {
    /// Create a new Governor with the given store and config.
    pub fn new(store: Arc<OptimizerStore>, config: GovernorConfig) -> Self {
        Self {
            store,
            config,
            canary_results: Mutex::new(HashMap::new()),
        }
    }

    /// Record a canary observation for a policy.  These results are stored
    /// in-memory and consulted by `check_and_promote`.
    pub fn record_canary_result(&self, policy_id: &str, success: bool, quality_score: f64) {
        let mut map = self.canary_results.lock().unwrap();
        map.entry(policy_id.to_string())
            .or_default()
            .push(CanaryResult {
                success,
                quality_score,
            });
        debug!(
            "Governor: recorded canary result for {}: success={} quality={:.3}",
            policy_id, success, quality_score
        );
    }

    /// Inspect all Canary-status policies and decide whether to promote,
    /// reject, or take no action on each one.
    pub async fn check_and_promote(&self) -> Result<Vec<GovernorAction>> {
        let canary_policies = self.store.list_policies_by_status(PolicyStatus::Canary)?;
        let mut actions = Vec::new();

        for policy in &canary_policies {
            let policy_id = &policy.policy_id;
            let results = {
                let map = self.canary_results.lock().unwrap();
                map.get(policy_id).cloned().unwrap_or_default()
            };

            let total = results.len() as u32;

            // Not enough data yet — skip.
            if total < self.config.canary_min_runs {
                let reason = format!(
                    "insufficient canary runs: {} of {} required",
                    total, self.config.canary_min_runs
                );
                debug!("Governor: {} — {}", policy_id, reason);
                actions.push(GovernorAction::NoAction {
                    policy_id: policy_id.clone(),
                    reason,
                });
                continue;
            }

            let success_count = results.iter().filter(|r| r.success).count() as f64;
            let success_rate = success_count / total as f64;
            let avg_quality: f64 =
                results.iter().map(|r| r.quality_score).sum::<f64>() / total as f64;

            // Check for unacceptable quality drop.
            let quality_drop_pct = (1.0 - avg_quality) * 100.0;

            if success_rate >= self.config.canary_success_threshold
                && quality_drop_pct <= self.config.rollback_quality_drop_pct
            {
                // Promote to Active.
                info!(
                    "Governor: promoting {} (v{}) to Active — success_rate={:.2}, avg_quality={:.3}",
                    policy_id, policy.version, success_rate, avg_quality
                );
                let promoted = self.store.promote_policy(
                    policy_id,
                    policy.version,
                    PolicyStatus::Active,
                )?;
                // Record the optimization run for audit trail.
                let _ = self.store.record_optimization_run(
                    "governor_promote",
                    policy_id,
                    &format!("{} canary runs", total),
                    Some(&policy.policy_ref),
                    Some(&promoted.policy_ref),
                    "promoted",
                    &format!(
                        "Canary promoted to Active: success_rate={:.2}, avg_quality={:.3}",
                        success_rate, avg_quality
                    ),
                );
                // Clear canary results now that the policy has been promoted.
                {
                    let mut map = self.canary_results.lock().unwrap();
                    map.remove(policy_id);
                }
                actions.push(GovernorAction::Promoted {
                    policy_id: policy_id.clone(),
                    from_version: policy.version,
                    to_status: "active".to_string(),
                });
            } else {
                // Reject (failure rate too high or quality drop too large).
                let reason = format!(
                    "canary failed: success_rate={:.2} (threshold={:.2}), avg_quality={:.3} (max_drop={:.1}%)",
                    success_rate,
                    self.config.canary_success_threshold,
                    avg_quality,
                    self.config.rollback_quality_drop_pct,
                );
                warn!("Governor: rejecting {} — {}", policy_id, reason);
                let _rejected = self.store.promote_policy(
                    policy_id,
                    policy.version,
                    PolicyStatus::Rejected,
                )?;
                let _ = self.store.record_optimization_run(
                    "governor_reject",
                    policy_id,
                    &format!("{} canary runs", total),
                    Some(&policy.policy_ref),
                    None,
                    "rejected",
                    &reason,
                );
                // Clear canary results.
                {
                    let mut map = self.canary_results.lock().unwrap();
                    map.remove(policy_id);
                }
                actions.push(GovernorAction::RolledBack {
                    policy_id: policy_id.clone(),
                    reason,
                });
            }
        }

        Ok(actions)
    }

    /// Spawn a background tokio loop that calls `check_and_promote` at the
    /// configured interval.  Returns a `JoinHandle` that the caller can
    /// `.abort()` to stop the loop.
    pub fn spawn_loop(self: Arc<Self>) -> JoinHandle<()> {
        let interval_secs = self.config.check_interval_secs;
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                interval.tick().await;
                match self.check_and_promote().await {
                    Ok(actions) => {
                        if !actions.is_empty() {
                            info!("Governor loop: {} actions taken", actions.len());
                            for action in &actions {
                                debug!("  {:?}", action);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Governor loop error: {}", e);
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimizer_store::{OptimizerStore, PolicyStatus, PolicyType};
    use uuid::Uuid;

    /// Create an isolated temp DB path using a UUID to avoid collisions.
    fn temp_db(name: &str) -> String {
        let unique = Uuid::new_v4();
        let dir =
            std::env::temp_dir().join(format!("phantom_mesh_governor_{}_{}", name, unique));
        let _ = std::fs::create_dir_all(&dir);
        dir.join("governor.db").to_string_lossy().to_string()
    }

    /// Helper: set up a store with a single Canary-status policy and return
    /// the (store, policy_id, version) triple.
    async fn setup_canary_policy(db: &str) -> (Arc<OptimizerStore>, String, i64) {
        let store = Arc::new(OptimizerStore::new(db).await.unwrap());
        let policy_id = format!("test.policy.{}", Uuid::new_v4());

        // v1: Active baseline
        store
            .insert_policy_version(
                &policy_id,
                PolicyType::Prompt,
                1,
                r#"{"baseline": true}"#,
                PolicyStatus::Active,
                Some(chrono::Utc::now().to_rfc3339()),
                None,
            )
            .unwrap();

        // v2: Canary candidate
        store
            .insert_policy_version(
                &policy_id,
                PolicyType::Prompt,
                2,
                r#"{"candidate": true}"#,
                PolicyStatus::Canary,
                None,
                None,
            )
            .unwrap();

        (store, policy_id, 2)
    }

    #[tokio::test]
    async fn test_promote_canary_to_active() {
        let db = temp_db("promote");
        let (store, policy_id, _version) = setup_canary_policy(&db).await;

        let config = GovernorConfig {
            canary_min_runs: 3,
            canary_success_threshold: 0.7,
            rollback_quality_drop_pct: 30.0,
            ..GovernorConfig::default()
        };
        let governor = Governor::new(Arc::clone(&store), config);

        // Record 4 successful canary results with high quality.
        for _ in 0..4 {
            governor.record_canary_result(&policy_id, true, 0.95);
        }

        let actions = governor.check_and_promote().await.unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            GovernorAction::Promoted {
                policy_id: pid,
                from_version,
                to_status,
            } => {
                assert_eq!(pid, &policy_id);
                assert_eq!(*from_version, 2);
                assert_eq!(to_status, "active");
            }
            other => panic!("expected Promoted, got {:?}", other),
        }

        // Verify the new version is Active in the store.
        let latest = store.latest_policy(&policy_id).unwrap().unwrap();
        assert_eq!(latest.version, 3);
        assert_eq!(latest.status, PolicyStatus::Active);

        // Canary results should have been cleared.
        let map = governor.canary_results.lock().unwrap();
        assert!(!map.contains_key(&policy_id));
    }

    #[tokio::test]
    async fn test_reject_failing_canary() {
        let db = temp_db("reject");
        let (store, policy_id, _version) = setup_canary_policy(&db).await;

        let config = GovernorConfig {
            canary_min_runs: 5,
            canary_success_threshold: 0.8,
            rollback_quality_drop_pct: 20.0,
            ..GovernorConfig::default()
        };
        let governor = Governor::new(Arc::clone(&store), config);

        // Record 5 results: 2 successes and 3 failures (40% success rate — below 80% threshold).
        governor.record_canary_result(&policy_id, true, 0.9);
        governor.record_canary_result(&policy_id, true, 0.85);
        governor.record_canary_result(&policy_id, false, 0.3);
        governor.record_canary_result(&policy_id, false, 0.2);
        governor.record_canary_result(&policy_id, false, 0.1);

        let actions = governor.check_and_promote().await.unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            GovernorAction::RolledBack { policy_id: pid, reason } => {
                assert_eq!(pid, &policy_id);
                assert!(reason.contains("canary failed"));
                assert!(reason.contains("success_rate=0.40"));
            }
            other => panic!("expected RolledBack, got {:?}", other),
        }

        // Verify the new version is Rejected in the store.
        let latest = store.latest_policy(&policy_id).unwrap().unwrap();
        assert_eq!(latest.version, 3);
        assert_eq!(latest.status, PolicyStatus::Rejected);

        // Canary results should have been cleared.
        let map = governor.canary_results.lock().unwrap();
        assert!(!map.contains_key(&policy_id));
    }

    #[tokio::test]
    async fn test_no_action_insufficient_runs() {
        let db = temp_db("no_action");
        let (store, policy_id, _version) = setup_canary_policy(&db).await;

        let config = GovernorConfig {
            canary_min_runs: 10,
            canary_success_threshold: 0.8,
            rollback_quality_drop_pct: 20.0,
            ..GovernorConfig::default()
        };
        let governor = Governor::new(Arc::clone(&store), config);

        // Record only 3 results — well below the 10 required.
        governor.record_canary_result(&policy_id, true, 0.95);
        governor.record_canary_result(&policy_id, true, 0.90);
        governor.record_canary_result(&policy_id, true, 0.88);

        let actions = governor.check_and_promote().await.unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            GovernorAction::NoAction { policy_id: pid, reason } => {
                assert_eq!(pid, &policy_id);
                assert!(reason.contains("insufficient canary runs"));
                assert!(reason.contains("3 of 10"));
            }
            other => panic!("expected NoAction, got {:?}", other),
        }

        // The policy version should remain unchanged (still v2 Canary).
        let latest = store.latest_policy(&policy_id).unwrap().unwrap();
        assert_eq!(latest.version, 2);
        assert_eq!(latest.status, PolicyStatus::Canary);

        // Canary results should still be present (not cleared).
        let map = governor.canary_results.lock().unwrap();
        assert_eq!(map.get(&policy_id).unwrap().len(), 3);
    }

    #[tokio::test]
    async fn test_no_canary_policies_yields_empty() {
        let db = temp_db("empty");
        let store = Arc::new(OptimizerStore::new(&db).await.unwrap());
        let governor = Governor::new(store, GovernorConfig::default());

        let actions = governor.check_and_promote().await.unwrap();
        assert!(actions.is_empty());
    }

    #[tokio::test]
    async fn test_quality_drop_causes_rejection() {
        let db = temp_db("quality_drop");
        let (store, policy_id, _version) = setup_canary_policy(&db).await;

        let config = GovernorConfig {
            canary_min_runs: 3,
            canary_success_threshold: 0.5, // low threshold — success rate is fine
            rollback_quality_drop_pct: 10.0, // but quality drop limit is tight
            ..GovernorConfig::default()
        };
        let governor = Governor::new(Arc::clone(&store), config);

        // All succeed, but quality is poor (avg ~0.5 → 50% drop from 1.0,
        // which exceeds the 10% max drop).
        governor.record_canary_result(&policy_id, true, 0.5);
        governor.record_canary_result(&policy_id, true, 0.5);
        governor.record_canary_result(&policy_id, true, 0.5);

        let actions = governor.check_and_promote().await.unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            GovernorAction::RolledBack { policy_id: pid, .. } => {
                assert_eq!(pid, &policy_id);
            }
            other => panic!("expected RolledBack due to quality drop, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_multiple_canary_policies() {
        let db = temp_db("multi");
        let store = Arc::new(OptimizerStore::new(&db).await.unwrap());

        let pid_a = format!("policy.a.{}", Uuid::new_v4());
        let pid_b = format!("policy.b.{}", Uuid::new_v4());

        // Policy A: v1 Active, v2 Canary
        store
            .insert_policy_version(&pid_a, PolicyType::Routing, 1, r#"{}"#, PolicyStatus::Active, Some(chrono::Utc::now().to_rfc3339()), None)
            .unwrap();
        store
            .insert_policy_version(&pid_a, PolicyType::Routing, 2, r#"{}"#, PolicyStatus::Canary, None, None)
            .unwrap();

        // Policy B: v1 Active, v2 Canary
        store
            .insert_policy_version(&pid_b, PolicyType::Workflow, 1, r#"{}"#, PolicyStatus::Active, Some(chrono::Utc::now().to_rfc3339()), None)
            .unwrap();
        store
            .insert_policy_version(&pid_b, PolicyType::Workflow, 2, r#"{}"#, PolicyStatus::Canary, None, None)
            .unwrap();

        let config = GovernorConfig {
            canary_min_runs: 2,
            canary_success_threshold: 0.8,
            rollback_quality_drop_pct: 20.0,
            ..GovernorConfig::default()
        };
        let governor = Governor::new(Arc::clone(&store), config);

        // A: passes
        governor.record_canary_result(&pid_a, true, 0.95);
        governor.record_canary_result(&pid_a, true, 0.90);

        // B: fails
        governor.record_canary_result(&pid_b, false, 0.1);
        governor.record_canary_result(&pid_b, false, 0.2);

        let actions = governor.check_and_promote().await.unwrap();
        assert_eq!(actions.len(), 2);

        let promoted_count = actions
            .iter()
            .filter(|a| matches!(a, GovernorAction::Promoted { .. }))
            .count();
        let rolled_back_count = actions
            .iter()
            .filter(|a| matches!(a, GovernorAction::RolledBack { .. }))
            .count();
        assert_eq!(promoted_count, 1);
        assert_eq!(rolled_back_count, 1);
    }
}
