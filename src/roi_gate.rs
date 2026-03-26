//! ROI Gate — pre-execution profitability checks before hands run.
//!
//! Evaluates whether a hand should be allowed to execute based on its ROI
//! history, daily budget constraints, scheduler recommendations, and
//! unit economics. User-triggered and exempt hands always pass.

use crate::roi_scheduler::{FrequencyTier, RoiScheduler};
use crate::unit_economics::UnitEconomics;
use serde::Serialize;
use std::sync::{Arc, Mutex};

// ── Configuration ───────────────────────────────────────────────────────────

/// Configuration for the ROI gate's decision thresholds.
#[derive(Debug, Clone)]
pub struct RoiGateConfig {
    /// Whether first-time (unknown) hands are allowed through the gate.
    pub allow_unknown_hands: bool,
    /// Maximum consecutive failures before a hand is denied.
    pub max_consecutive_failures: u32,
    /// Maximum daily spend (USD) across all hands before denying new runs.
    pub daily_budget_usd: f64,
    /// Minimum ROI threshold; hands below this get an AllowWithWarning.
    pub min_roi_threshold: f64,
    /// Hand names that always pass the gate regardless of metrics.
    pub exempt_hands: Vec<String>,
}

impl Default for RoiGateConfig {
    fn default() -> Self {
        Self {
            allow_unknown_hands: true,
            max_consecutive_failures: 5,
            daily_budget_usd: 5.0,
            min_roi_threshold: 0.5,
            exempt_hands: vec![
                "cluster-health".to_string(),
                "self-optimize".to_string(),
            ],
        }
    }
}

// ── Gate Decision ───────────────────────────────────────────────────────────

/// The result of an ROI gate check for a hand.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum GateDecision {
    /// Hand is allowed to run.
    Allow { reason: String },
    /// Hand is denied from running.
    Deny { reason: String },
    /// Hand is allowed but with a warning about suboptimal ROI.
    AllowWithWarning { reason: String, warning: String },
}

impl GateDecision {
    /// Returns `true` if the decision permits execution (Allow or AllowWithWarning).
    pub fn is_allowed(&self) -> bool {
        matches!(self, GateDecision::Allow { .. } | GateDecision::AllowWithWarning { .. })
    }
}

// ── ROI Gate ────────────────────────────────────────────────────────────────

/// Pre-execution profitability gate that checks ROI, budget, and scheduler
/// recommendations before allowing a hand to run.
pub struct RoiGate {
    roi_scheduler: Arc<RoiScheduler>,
    unit_economics: Arc<UnitEconomics>,
    config: RoiGateConfig,
    daily_spend: Mutex<f64>,
    daily_spend_date: Mutex<chrono::NaiveDate>,
}

impl RoiGate {
    /// Create a new ROI gate with the given dependencies and configuration.
    pub fn new(
        roi_scheduler: Arc<RoiScheduler>,
        unit_economics: Arc<UnitEconomics>,
        config: RoiGateConfig,
    ) -> Self {
        Self {
            roi_scheduler,
            unit_economics,
            config,
            daily_spend: Mutex::new(0.0),
            daily_spend_date: Mutex::new(chrono::Utc::now().date_naive()),
        }
    }

    /// Check whether a hand should be allowed to execute.
    ///
    /// Decision priority (first match wins):
    /// 1. User-triggered executions always pass.
    /// 2. Exempt hands always pass.
    /// 3. Paused frequency tier → Deny.
    /// 4. Scheduler says should_run_now() false → Deny.
    /// 5. Daily budget exceeded → Deny.
    /// 6. Low ROI (below min_roi_threshold) → AllowWithWarning.
    /// 7. Unknown hand + allow_unknown_hands → Allow.
    /// 8. Otherwise → Allow.
    pub fn check(&self, hand_name: &str, is_user_triggered: bool) -> GateDecision {
        // 1. User-triggered always passes
        if is_user_triggered {
            return GateDecision::Allow {
                reason: "User-triggered execution always allowed".to_string(),
            };
        }

        // 2. Exempt hands always pass
        if self.config.exempt_hands.iter().any(|e| e == hand_name) {
            return GateDecision::Allow {
                reason: format!("Hand '{}' is exempt from ROI gate", hand_name),
            };
        }

        // Reset daily spend if the date has changed
        self.reset_if_new_day();

        // 3. Check frequency tier — Paused means deny
        let tier = self.roi_scheduler.get_recommendation(hand_name);
        if tier == FrequencyTier::Paused {
            return GateDecision::Deny {
                reason: format!(
                    "Hand '{}' is Paused by ROI scheduler (negative ROI)",
                    hand_name
                ),
            };
        }

        // 4. Scheduler timing check
        if !self.roi_scheduler.should_run_now(hand_name) {
            return GateDecision::Deny {
                reason: format!(
                    "Hand '{}' ran too recently for its {:?} tier",
                    hand_name, tier
                ),
            };
        }

        // 5. Daily budget check
        let current_spend = self.current_spend();
        if current_spend >= self.config.daily_budget_usd {
            return GateDecision::Deny {
                reason: format!(
                    "Daily budget exhausted: ${:.2} spent of ${:.2} limit",
                    current_spend, self.config.daily_budget_usd
                ),
            };
        }

        // 6. Check unit economics for low ROI warning
        if let Some(economics) = self.unit_economics.get_economics(hand_name) {
            let roi = if economics.cost_usd > 0.0 {
                (economics.revenue_usd - economics.cost_usd) / economics.cost_usd
            } else {
                f64::INFINITY
            };

            if roi < self.config.min_roi_threshold && !roi.is_infinite() {
                return GateDecision::AllowWithWarning {
                    reason: format!("Hand '{}' allowed but ROI is below threshold", hand_name),
                    warning: format!(
                        "ROI is {:.1}% (threshold: {:.1}%); margin ${:.2} over {} executions",
                        roi * 100.0,
                        self.config.min_roi_threshold * 100.0,
                        economics.margin_usd,
                        economics.execution_count,
                    ),
                };
            }

            return GateDecision::Allow {
                reason: format!(
                    "Hand '{}' has healthy ROI ({:.1}%) — allowed",
                    hand_name,
                    roi * 100.0
                ),
            };
        }

        // 7. Unknown hand
        if self.config.allow_unknown_hands {
            return GateDecision::Allow {
                reason: format!(
                    "Hand '{}' is unknown (first run) — allowed by policy",
                    hand_name
                ),
            };
        }

        GateDecision::Deny {
            reason: format!(
                "Hand '{}' is unknown and allow_unknown_hands is disabled",
                hand_name
            ),
        }
    }

    /// Record spend against the daily budget.
    pub fn record_spend(&self, cost_usd: f64) {
        self.reset_if_new_day();
        let mut spend = self.daily_spend.lock().unwrap();
        *spend += cost_usd;
    }

    /// Return the current daily spend so far.
    pub fn current_spend(&self) -> f64 {
        let spend = self.daily_spend.lock().unwrap();
        *spend
    }

    /// Reset daily spend to zero if the calendar date has changed.
    /// Get a reference to the current config.
    pub fn config(&self) -> &RoiGateConfig {
        &self.config
    }

    pub fn reset_if_new_day(&self) {
        let today = chrono::Utc::now().date_naive();
        let mut date = self.daily_spend_date.lock().unwrap();
        if *date != today {
            *date = today;
            let mut spend = self.daily_spend.lock().unwrap();
            *spend = 0.0;
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a gate with default config and fresh scheduler/economics.
    fn make_gate() -> RoiGate {
        let scheduler = Arc::new(RoiScheduler::new());
        let economics = Arc::new(UnitEconomics::new());
        let config = RoiGateConfig::default();
        RoiGate::new(scheduler, economics, config)
    }

    /// Helper: build a gate with shared scheduler and economics so callers can
    /// pre-populate data.
    fn make_gate_with(
        scheduler: Arc<RoiScheduler>,
        economics: Arc<UnitEconomics>,
        config: RoiGateConfig,
    ) -> RoiGate {
        RoiGate::new(scheduler, economics, config)
    }

    // ── 1. User-triggered always allowed ────────────────────────────────────

    #[test]
    fn test_user_triggered_always_allowed() {
        let gate = make_gate();
        // Even a completely unknown hand should be allowed when user-triggered
        let decision = gate.check("totally-unknown-hand", true);
        assert!(decision.is_allowed());
        match decision {
            GateDecision::Allow { reason } => {
                assert!(reason.contains("User-triggered"));
            }
            _ => panic!("Expected Allow for user-triggered"),
        }

        // Even a hand that would normally be paused should pass when user-triggered
        let scheduler = Arc::new(RoiScheduler::new());
        // Feed enough negative-ROI executions to get Paused
        for _ in 0..5 {
            scheduler.record_execution("bad-hand", 0.1, 2.0, true);
        }
        assert_eq!(scheduler.get_recommendation("bad-hand"), FrequencyTier::Paused);

        let economics = Arc::new(UnitEconomics::new());
        let gate = make_gate_with(scheduler, economics, RoiGateConfig::default());
        let decision = gate.check("bad-hand", true);
        assert!(decision.is_allowed());
    }

    // ── 2. Exempt hand allowed ──────────────────────────────────────────────

    #[test]
    fn test_exempt_hand_allowed() {
        let gate = make_gate();

        // Default exempt hands: "cluster-health" and "self-optimize"
        let decision = gate.check("cluster-health", false);
        assert!(decision.is_allowed());
        match decision {
            GateDecision::Allow { reason } => {
                assert!(reason.contains("exempt"));
            }
            _ => panic!("Expected Allow for exempt hand"),
        }

        let decision = gate.check("self-optimize", false);
        assert!(decision.is_allowed());
    }

    // ── 3. Budget exhausted → denied ────────────────────────────────────────

    #[test]
    fn test_budget_exhausted_denied() {
        let gate = make_gate();

        // Spend the entire daily budget
        gate.record_spend(5.0);
        assert!((gate.current_spend() - 5.0).abs() < f64::EPSILON);

        // Now a non-exempt, non-user-triggered hand should be denied
        let decision = gate.check("some-hand", false);
        match decision {
            GateDecision::Deny { reason } => {
                assert!(reason.contains("budget"));
            }
            _ => panic!("Expected Deny when budget exhausted, got {:?}", decision),
        }
    }

    // ── 4. Paused hand → denied ─────────────────────────────────────────────

    #[test]
    fn test_paused_hand_denied() {
        let scheduler = Arc::new(RoiScheduler::new());
        // ROI = (0.5 - 10) / 10 = -0.95 → below pause threshold (-0.5)
        for _ in 0..5 {
            scheduler.record_execution("money-pit", 0.1, 2.0, true);
        }
        assert_eq!(
            scheduler.get_recommendation("money-pit"),
            FrequencyTier::Paused
        );

        let economics = Arc::new(UnitEconomics::new());
        let gate = make_gate_with(scheduler, economics, RoiGateConfig::default());

        let decision = gate.check("money-pit", false);
        match decision {
            GateDecision::Deny { reason } => {
                assert!(reason.contains("Paused"));
            }
            _ => panic!("Expected Deny for paused hand, got {:?}", decision),
        }
    }

    // ── 5. Unknown hand allowed by default ──────────────────────────────────

    #[test]
    fn test_unknown_hand_allowed_by_default() {
        let gate = make_gate();

        let decision = gate.check("brand-new-hand", false);
        assert!(decision.is_allowed());
        match decision {
            GateDecision::Allow { reason } => {
                assert!(reason.contains("unknown") || reason.contains("first run"));
            }
            _ => panic!("Expected Allow for unknown hand, got {:?}", decision),
        }
    }

    // ── 6. Unknown hand denied when policy disables it ──────────────────────

    #[test]
    fn test_unknown_hand_denied_when_disabled() {
        let scheduler = Arc::new(RoiScheduler::new());
        let economics = Arc::new(UnitEconomics::new());
        let config = RoiGateConfig {
            allow_unknown_hands: false,
            ..Default::default()
        };
        let gate = make_gate_with(scheduler, economics, config);

        let decision = gate.check("never-seen", false);
        match decision {
            GateDecision::Deny { reason } => {
                assert!(reason.contains("unknown"));
            }
            _ => panic!("Expected Deny for unknown hand with policy disabled"),
        }
    }

    // ── 7. Low ROI → AllowWithWarning ───────────────────────────────────────

    #[test]
    fn test_low_roi_allow_with_warning() {
        let scheduler = Arc::new(RoiScheduler::new());
        let economics = Arc::new(UnitEconomics::new());

        // Record economics with low but positive ROI (below 0.5 threshold)
        // revenue=12, cost=10 → ROI = (12-10)/10 = 0.2 → below min_roi_threshold (0.5)
        for _ in 0..5 {
            economics.record_execution("low-roi-hand", 2.4, 2.0, 10.0);
        }
        // Also record in scheduler so it has metrics (enough runs, not paused)
        // ROI = 0.2 → Conservative tier (between -0.5 and 0.5)
        for _ in 0..5 {
            scheduler.record_execution("low-roi-hand", 2.4, 2.0, true);
        }

        let gate = make_gate_with(scheduler, economics, RoiGateConfig::default());

        // The scheduler will say should_run_now() is false because it just ran,
        // so we need to test with a fresh scheduler that has never been run.
        // Instead, build a scheduler with the right data but set the hand
        // to have last run long enough ago. Since we can't manipulate time easily,
        // we'll use a hand that the scheduler considers Experimental (< 5 runs)
        // and then feed economics directly.

        let scheduler2 = Arc::new(RoiScheduler::new());
        let economics2 = Arc::new(UnitEconomics::new());
        // Only 3 scheduler runs → Experimental tier, should_run_now returns true
        // because elapsed > 168h for unknown/first time
        // Actually for unknown hand should_run_now returns true
        for _ in 0..3 {
            economics2.record_execution("low-roi-hand", 2.4, 2.0, 10.0);
        }

        let gate2 = make_gate_with(scheduler2, economics2, RoiGateConfig::default());
        let decision = gate2.check("low-roi-hand", false);
        match decision {
            GateDecision::AllowWithWarning { reason, warning } => {
                assert!(reason.contains("ROI"));
                assert!(warning.contains("threshold"));
            }
            _ => panic!("Expected AllowWithWarning for low-ROI hand, got {:?}", decision),
        }
    }

    // ── 8. Healthy ROI → Allow ──────────────────────────────────────────────

    #[test]
    fn test_healthy_roi_allowed() {
        let scheduler = Arc::new(RoiScheduler::new());
        let economics = Arc::new(UnitEconomics::new());

        // Good ROI: revenue=10, cost=2 → ROI = (10-2)/2 = 4.0 → well above threshold
        for _ in 0..3 {
            economics.record_execution("star-hand", 10.0, 2.0, 5.0);
        }

        let gate = make_gate_with(scheduler, economics, RoiGateConfig::default());
        let decision = gate.check("star-hand", false);
        match decision {
            GateDecision::Allow { reason } => {
                assert!(reason.contains("healthy ROI") || reason.contains("allowed"));
            }
            _ => panic!("Expected Allow for healthy-ROI hand, got {:?}", decision),
        }
    }

    // ── 9. record_spend and current_spend ───────────────────────────────────

    #[test]
    fn test_record_and_current_spend() {
        let gate = make_gate();

        assert!((gate.current_spend() - 0.0).abs() < f64::EPSILON);

        gate.record_spend(1.50);
        assert!((gate.current_spend() - 1.50).abs() < f64::EPSILON);

        gate.record_spend(0.75);
        assert!((gate.current_spend() - 2.25).abs() < f64::EPSILON);
    }

    // ── 10. GateDecision::is_allowed ────────────────────────────────────────

    #[test]
    fn test_gate_decision_is_allowed() {
        let allow = GateDecision::Allow {
            reason: "ok".to_string(),
        };
        assert!(allow.is_allowed());

        let warn = GateDecision::AllowWithWarning {
            reason: "ok".to_string(),
            warning: "heads up".to_string(),
        };
        assert!(warn.is_allowed());

        let deny = GateDecision::Deny {
            reason: "no".to_string(),
        };
        assert!(!deny.is_allowed());
    }

    // ── 11. Config defaults ─────────────────────────────────────────────────

    #[test]
    fn test_config_defaults() {
        let cfg = RoiGateConfig::default();
        assert!(cfg.allow_unknown_hands);
        assert_eq!(cfg.max_consecutive_failures, 5);
        assert!((cfg.daily_budget_usd - 5.0).abs() < f64::EPSILON);
        assert!((cfg.min_roi_threshold - 0.5).abs() < f64::EPSILON);
        assert_eq!(cfg.exempt_hands.len(), 2);
        assert!(cfg.exempt_hands.contains(&"cluster-health".to_string()));
        assert!(cfg.exempt_hands.contains(&"self-optimize".to_string()));
    }

    // ── 12. GateDecision serialization ──────────────────────────────────────

    #[test]
    fn test_gate_decision_serialization() {
        let allow = GateDecision::Allow {
            reason: "test".to_string(),
        };
        let json = serde_json::to_string(&allow).unwrap();
        assert!(json.contains("\"decision\":\"allow\""));
        assert!(json.contains("\"reason\":\"test\""));

        let deny = GateDecision::Deny {
            reason: "nope".to_string(),
        };
        let json = serde_json::to_string(&deny).unwrap();
        assert!(json.contains("\"decision\":\"deny\""));

        let warn = GateDecision::AllowWithWarning {
            reason: "ok".to_string(),
            warning: "careful".to_string(),
        };
        let json = serde_json::to_string(&warn).unwrap();
        assert!(json.contains("\"decision\":\"allow_with_warning\""));
        assert!(json.contains("\"warning\":\"careful\""));
    }
}
