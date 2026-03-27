//! Feedback Loop — connects trajectory analysis → prompt optimization → policy store → scheduler.
//!
//! This is the "brain" that makes the system self-improving by analyzing execution
//! data and generating better prompts/routing policies automatically.

use crate::governor::Governor;
use crate::optimizer_store::{OptimizerStore, PolicyStatus, PolicyType};
use crate::roi_scheduler::RoiScheduler;
use crate::trajectory::TrajectoryLogger;
use serde::Serialize;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FeedbackLoopConfig {
    /// Minimum trajectory entries before attempting optimization.
    pub min_trajectories: usize,
    /// How often to run the feedback cycle (seconds).
    pub interval_secs: u64,
    /// Hands to optimize (empty = all hands with trajectory data).
    pub target_hands: Vec<String>,
}

impl Default for FeedbackLoopConfig {
    fn default() -> Self {
        Self {
            min_trajectories: 10,
            interval_secs: 21600, // 6 hours
            target_hands: vec![],
        }
    }
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct FeedbackCycleReport {
    pub hands_analyzed: u32,
    pub optimizations_attempted: u32,
    pub new_policies_created: u32,
    pub frequency_adjustments: u32,
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// FeedbackLoop
// ---------------------------------------------------------------------------

pub struct FeedbackLoop {
    trajectory: Arc<TrajectoryLogger>,
    store: Arc<OptimizerStore>,
    governor: Arc<Governor>,
    roi_scheduler: Arc<RoiScheduler>,
    config: FeedbackLoopConfig,
}

impl FeedbackLoop {
    pub fn new(
        trajectory: Arc<TrajectoryLogger>,
        store: Arc<OptimizerStore>,
        governor: Arc<Governor>,
        roi_scheduler: Arc<RoiScheduler>,
        config: FeedbackLoopConfig,
    ) -> Self {
        Self {
            trajectory,
            store,
            governor,
            roi_scheduler,
            config,
        }
    }

    /// Run one cycle of the feedback loop.
    ///
    /// 1. List hands with trajectory data
    /// 2. For each hand with enough data, analyze quality trends
    /// 3. If quality is declining, create a Draft policy suggesting investigation
    /// 4. Run governor promotion check
    /// 5. Log frequency recommendations from ROI scheduler
    pub async fn run_cycle(&self) -> FeedbackCycleReport {
        let mut report = FeedbackCycleReport {
            hands_analyzed: 0,
            optimizations_attempted: 0,
            new_policies_created: 0,
            frequency_adjustments: 0,
            errors: vec![],
        };

        // Step 1: Get hands to analyze
        let hands_to_analyze = if self.config.target_hands.is_empty() {
            match self.trajectory.list_hand_names() {
                Ok(names) => names,
                Err(e) => {
                    report
                        .errors
                        .push(format!("Failed to list hands: {}", e));
                    return report;
                }
            }
        } else {
            self.config.target_hands.clone()
        };

        for hand_name in &hands_to_analyze {
            report.hands_analyzed += 1;

            // Step 2: Check if enough trajectories exist
            let trajectory_count = match self.trajectory.count_for_hand(hand_name) {
                Ok(n) => n,
                Err(e) => {
                    report
                        .errors
                        .push(format!("{}: trajectory count failed: {}", hand_name, e));
                    continue;
                }
            };

            if trajectory_count < self.config.min_trajectories {
                tracing::debug!(
                    "[FeedbackLoop] {}: only {} trajectories (need {}), skipping",
                    hand_name,
                    trajectory_count,
                    self.config.min_trajectories
                );
                continue;
            }

            // Step 3: Analyze recent trajectories for quality trends
            report.optimizations_attempted += 1;
            let policy_id = format!("prompt-{}", hand_name);

            match self.trajectory.by_hand(hand_name, 50) {
                Ok(entries) => {
                    if entries.is_empty() {
                        continue;
                    }

                    // Calculate average quality from entries that have scores
                    let scored: Vec<_> = entries
                        .iter()
                        .filter_map(|e| e.quality_score)
                        .collect();

                    if scored.len() >= 5 {
                        let avg_quality =
                            scored.iter().map(|&s| s as f64).sum::<f64>() / scored.len() as f64;
                        let success_rate = entries.iter().filter(|e| e.success).count() as f64
                            / entries.len() as f64;

                        // If quality is low, create a Draft policy flagging the issue
                        if avg_quality < 3.0 || success_rate < 0.6 {
                            let content = serde_json::json!({
                                "analysis": "low_quality_detected",
                                "avg_quality": avg_quality,
                                "success_rate": success_rate,
                                "sample_size": scored.len(),
                                "suggestion": "Review prompt and provider for this hand",
                                "source": "feedback_loop",
                            });

                            let version = chrono::Utc::now().timestamp();
                            match self.store.insert_policy_version(
                                &policy_id,
                                PolicyType::Prompt,
                                version,
                                &content.to_string(),
                                PolicyStatus::Draft,
                                None,
                                None,
                            ) {
                                Ok(_) => {
                                    report.new_policies_created += 1;
                                    tracing::info!(
                                        "[FeedbackLoop] Created Draft policy for {}: avg_quality={:.1}, success_rate={:.0}%",
                                        hand_name, avg_quality, success_rate * 100.0
                                    );
                                }
                                Err(e) => {
                                    report
                                        .errors
                                        .push(format!("{}: policy insert failed: {}", hand_name, e));
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    report
                        .errors
                        .push(format!("{}: trajectory query failed: {}", hand_name, e));
                }
            }

            // Step 5: Log ROI frequency recommendation
            let recommendation = self.roi_scheduler.get_recommendation(hand_name);
            tracing::info!(
                "[FeedbackLoop] {}: frequency recommendation = {:?}",
                hand_name,
                recommendation
            );
            report.frequency_adjustments += 1;
        }

        // Step 6: Run governor promotion check
        match self.governor.check_and_promote().await {
            Ok(actions) => {
                for action in &actions {
                    tracing::info!("[FeedbackLoop] Governor action: {:?}", action);
                }
            }
            Err(e) => {
                report
                    .errors
                    .push(format!("Governor check failed: {}", e));
            }
        }

        report
    }

    /// Spawn the background feedback loop.
    pub fn spawn_loop(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        let interval = self.config.interval_secs;
        tokio::spawn(async move {
            loop {
                tracing::info!("[FeedbackLoop] Starting optimization cycle...");
                let report = self.run_cycle().await;
                tracing::info!(
                    "[FeedbackLoop] Cycle complete: analyzed={}, optimized={}, new_policies={}, errors={}",
                    report.hands_analyzed,
                    report.optimizations_attempted,
                    report.new_policies_created,
                    report.errors.len()
                );
                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            }
        })
    }

    /// Get the current config (for API endpoint).
    pub fn config(&self) -> &FeedbackLoopConfig {
        &self.config
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = FeedbackLoopConfig::default();
        assert_eq!(config.min_trajectories, 10);
        assert_eq!(config.interval_secs, 21600);
        assert!(config.target_hands.is_empty());
    }

    #[test]
    fn test_report_serializable() {
        let report = FeedbackCycleReport {
            hands_analyzed: 5,
            optimizations_attempted: 3,
            new_policies_created: 1,
            frequency_adjustments: 2,
            errors: vec!["test error".to_string()],
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("hands_analyzed"));
        assert!(json.contains("test error"));
    }
}
