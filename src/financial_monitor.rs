//! Financial Red-Line Monitoring — tracks 7 key financial indicators and raises alerts
//! when thresholds are breached. Designed for real-time cost/revenue health checks
//! across the Clawtex cluster.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Severity level for a financial alert.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AlertLevel {
    Info,
    Warn,
    Critical,
    Emergency,
}

impl std::fmt::Display for AlertLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertLevel::Info => write!(f, "INFO"),
            AlertLevel::Warn => write!(f, "WARN"),
            AlertLevel::Critical => write!(f, "CRITICAL"),
            AlertLevel::Emergency => write!(f, "EMERGENCY"),
        }
    }
}

/// A single financial alert produced when an indicator breaches its threshold.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinancialAlert {
    pub level: AlertLevel,
    pub indicator_name: String,
    pub current_value: f64,
    pub threshold: f64,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

/// Snapshot of all financial values needed for a full evaluation pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinancialSnapshot {
    /// Today's total spending so far (USD)
    pub daily_spend: f64,
    /// Configured daily spending limit (USD)
    pub daily_limit: f64,
    /// API costs for the current period (USD)
    pub api_cost: f64,
    /// Revenue for the current period (USD)
    pub revenue: f64,
    /// Previous period revenue for trend comparison (USD)
    pub previous_revenue: f64,
    /// Total project cost (USD)
    pub project_cost: f64,
    /// Current cash balance (USD)
    pub cash_balance: f64,
    /// Monthly burn rate (USD)
    pub monthly_burn: f64,
    /// Current period cost for spike detection (USD)
    pub current_period_cost: f64,
    /// Rolling average cost for comparison (USD)
    pub average_cost: f64,
    /// Budget used so far (USD)
    pub budget_used: f64,
    /// Total budget (USD)
    pub budget_total: f64,
}

/// Financial red-line monitor. Stateless evaluator — feed it a snapshot and it
/// returns any alerts that fire.
#[derive(Debug, Clone)]
pub struct FinancialMonitor {
    /// Percentage of daily limit that triggers a warning (default 0.80)
    pub daily_spend_warn_pct: f64,
    /// API-to-revenue ratio threshold (default 0.25)
    pub api_revenue_ratio_limit: f64,
    /// Minimum acceptable project margin (default 0.30)
    pub min_project_margin: f64,
    /// Minimum cash runway in months (default 2.0)
    pub min_runway_months: f64,
    /// Cost spike multiplier over average (default 2.0)
    pub cost_spike_multiplier: f64,
    /// Revenue decline percentage threshold (default 0.20)
    pub revenue_decline_pct: f64,
    /// Budget utilization warning threshold (default 0.90)
    pub budget_utilization_warn_pct: f64,
}

impl Default for FinancialMonitor {
    fn default() -> Self {
        Self {
            daily_spend_warn_pct: 0.80,
            api_revenue_ratio_limit: 0.25,
            min_project_margin: 0.30,
            min_runway_months: 2.0,
            cost_spike_multiplier: 2.0,
            revenue_decline_pct: 0.20,
            budget_utilization_warn_pct: 0.90,
        }
    }
}

impl FinancialMonitor {
    /// Create a monitor with default thresholds.
    pub fn new() -> Self {
        Self::default()
    }

    // ---------------------------------------------------------------
    // Individual indicator checks
    // ---------------------------------------------------------------

    /// Check daily spend against a configured limit.
    /// - >=100% of limit  -> Emergency
    /// - >=95% of limit   -> Critical
    /// - >=80% of limit   -> Warn
    /// - otherwise         -> None
    pub fn check_daily_spend(&self, costs: f64, limit: f64) -> Option<FinancialAlert> {
        if limit <= 0.0 {
            return None;
        }
        let ratio = costs / limit;
        if ratio >= 1.0 {
            Some(self.make_alert(
                AlertLevel::Emergency,
                "daily_spend",
                costs,
                limit,
                format!(
                    "Daily spend ${:.4} has EXCEEDED limit ${:.2} ({:.1}%)",
                    costs,
                    limit,
                    ratio * 100.0,
                ),
            ))
        } else if ratio >= 0.95 {
            Some(self.make_alert(
                AlertLevel::Critical,
                "daily_spend",
                costs,
                limit,
                format!(
                    "Daily spend ${:.4} is at {:.1}% of ${:.2} limit — approaching cap",
                    costs,
                    ratio * 100.0,
                    limit,
                ),
            ))
        } else if ratio >= self.daily_spend_warn_pct {
            Some(self.make_alert(
                AlertLevel::Warn,
                "daily_spend",
                costs,
                limit,
                format!(
                    "Daily spend ${:.4} is at {:.1}% of ${:.2} limit",
                    costs,
                    ratio * 100.0,
                    limit,
                ),
            ))
        } else {
            None
        }
    }

    /// Check API cost to revenue ratio.
    /// - ratio >50%  -> Critical
    /// - ratio >25%  -> Warn
    /// - otherwise   -> None
    ///
    /// If revenue is zero (or negative) and api_cost > 0, that is Emergency.
    pub fn check_api_revenue_ratio(&self, api_cost: f64, revenue: f64) -> Option<FinancialAlert> {
        if api_cost <= 0.0 {
            return None;
        }
        if revenue <= 0.0 {
            return Some(self.make_alert(
                AlertLevel::Emergency,
                "api_revenue_ratio",
                api_cost,
                0.0,
                format!(
                    "API cost ${:.4} with zero/negative revenue — infinite cost ratio",
                    api_cost,
                ),
            ));
        }
        let ratio = api_cost / revenue;
        if ratio > 0.50 {
            Some(self.make_alert(
                AlertLevel::Critical,
                "api_revenue_ratio",
                ratio,
                self.api_revenue_ratio_limit,
                format!(
                    "API cost/revenue ratio {:.1}% exceeds 50% — costs dominating revenue",
                    ratio * 100.0,
                ),
            ))
        } else if ratio > self.api_revenue_ratio_limit {
            Some(self.make_alert(
                AlertLevel::Warn,
                "api_revenue_ratio",
                ratio,
                self.api_revenue_ratio_limit,
                format!(
                    "API cost/revenue ratio {:.1}% exceeds {:.0}% threshold",
                    ratio * 100.0,
                    self.api_revenue_ratio_limit * 100.0,
                ),
            ))
        } else {
            None
        }
    }

    /// Check project profit margin.
    /// - margin <10%  -> Critical
    /// - margin <30%  -> Warn
    /// - otherwise    -> None
    ///
    /// Margin = (revenue - cost) / revenue. If revenue <= 0, returns Emergency when cost > 0.
    pub fn check_project_margin(&self, revenue: f64, cost: f64) -> Option<FinancialAlert> {
        if revenue <= 0.0 {
            if cost > 0.0 {
                return Some(self.make_alert(
                    AlertLevel::Emergency,
                    "project_margin",
                    -1.0,
                    self.min_project_margin,
                    format!(
                        "Negative/zero revenue (${:.2}) with cost ${:.4} — margin undefined",
                        revenue, cost,
                    ),
                ));
            }
            return None;
        }
        let margin = (revenue - cost) / revenue;
        if margin < 0.10 {
            Some(self.make_alert(
                AlertLevel::Critical,
                "project_margin",
                margin,
                self.min_project_margin,
                format!(
                    "Project margin {:.1}% is critically low (< 10%)",
                    margin * 100.0,
                ),
            ))
        } else if margin < self.min_project_margin {
            Some(self.make_alert(
                AlertLevel::Warn,
                "project_margin",
                margin,
                self.min_project_margin,
                format!(
                    "Project margin {:.1}% is below {:.0}% minimum",
                    margin * 100.0,
                    self.min_project_margin * 100.0,
                ),
            ))
        } else {
            None
        }
    }

    /// Check cash runway (balance / monthly_burn).
    /// - <1 month  -> Emergency
    /// - <2 months -> Critical
    /// - <3 months -> Warn
    /// - otherwise -> None
    ///
    /// If monthly_burn <= 0, no alert (no burn = infinite runway).
    pub fn check_cash_runway(&self, balance: f64, monthly_burn: f64) -> Option<FinancialAlert> {
        if monthly_burn <= 0.0 {
            return None;
        }
        let runway_months = balance / monthly_burn;
        if runway_months < 1.0 {
            Some(self.make_alert(
                AlertLevel::Emergency,
                "cash_runway",
                runway_months,
                self.min_runway_months,
                format!(
                    "Cash runway {:.1} months — LESS THAN 1 MONTH remaining (${:.2} / ${:.2}/mo)",
                    runway_months, balance, monthly_burn,
                ),
            ))
        } else if runway_months < self.min_runway_months {
            Some(self.make_alert(
                AlertLevel::Critical,
                "cash_runway",
                runway_months,
                self.min_runway_months,
                format!(
                    "Cash runway {:.1} months is below {:.0}-month minimum (${:.2} / ${:.2}/mo)",
                    runway_months, self.min_runway_months, balance, monthly_burn,
                ),
            ))
        } else if runway_months < 3.0 {
            Some(self.make_alert(
                AlertLevel::Warn,
                "cash_runway",
                runway_months,
                self.min_runway_months,
                format!(
                    "Cash runway {:.1} months — consider extending (${:.2} / ${:.2}/mo)",
                    runway_months, balance, monthly_burn,
                ),
            ))
        } else {
            None
        }
    }

    /// Check for cost spikes vs rolling average.
    /// - >3x average -> Emergency
    /// - >2x average -> Critical
    /// - >1.5x average -> Warn
    /// - otherwise   -> None
    ///
    /// If avg <= 0, no meaningful comparison — skip.
    pub fn check_cost_spike(&self, current: f64, avg: f64) -> Option<FinancialAlert> {
        if avg <= 0.0 || current <= 0.0 {
            return None;
        }
        let ratio = current / avg;
        if ratio > 3.0 {
            Some(self.make_alert(
                AlertLevel::Emergency,
                "cost_spike",
                current,
                avg * self.cost_spike_multiplier,
                format!(
                    "Cost spike: ${:.4} is {:.1}x the average ${:.4} — extreme anomaly",
                    current, ratio, avg,
                ),
            ))
        } else if ratio > self.cost_spike_multiplier {
            Some(self.make_alert(
                AlertLevel::Critical,
                "cost_spike",
                current,
                avg * self.cost_spike_multiplier,
                format!(
                    "Cost spike: ${:.4} is {:.1}x the average ${:.4}",
                    current, ratio, avg,
                ),
            ))
        } else if ratio > 1.5 {
            Some(self.make_alert(
                AlertLevel::Warn,
                "cost_spike",
                current,
                avg * self.cost_spike_multiplier,
                format!(
                    "Elevated costs: ${:.4} is {:.1}x the average ${:.4}",
                    current, ratio, avg,
                ),
            ))
        } else {
            None
        }
    }

    /// Check for revenue decline vs previous period.
    /// - >50% drop  -> Emergency
    /// - >20% drop  -> Critical
    /// - >10% drop  -> Warn
    /// - otherwise  -> None
    ///
    /// If previous <= 0, no meaningful comparison — skip.
    pub fn check_revenue_decline(&self, current: f64, previous: f64) -> Option<FinancialAlert> {
        if previous <= 0.0 {
            return None;
        }
        if current >= previous {
            return None;
        }
        let decline_pct = (previous - current) / previous;
        if decline_pct > 0.50 {
            Some(self.make_alert(
                AlertLevel::Emergency,
                "revenue_decline",
                decline_pct,
                self.revenue_decline_pct,
                format!(
                    "Revenue crashed {:.1}% — ${:.2} vs previous ${:.2}",
                    decline_pct * 100.0, current, previous,
                ),
            ))
        } else if decline_pct > self.revenue_decline_pct {
            Some(self.make_alert(
                AlertLevel::Critical,
                "revenue_decline",
                decline_pct,
                self.revenue_decline_pct,
                format!(
                    "Revenue declined {:.1}% — ${:.2} vs previous ${:.2}",
                    decline_pct * 100.0, current, previous,
                ),
            ))
        } else if decline_pct > 0.10 {
            Some(self.make_alert(
                AlertLevel::Warn,
                "revenue_decline",
                decline_pct,
                self.revenue_decline_pct,
                format!(
                    "Revenue dipped {:.1}% — ${:.2} vs previous ${:.2}",
                    decline_pct * 100.0, current, previous,
                ),
            ))
        } else {
            None
        }
    }

    /// Check budget utilization.
    /// - >=100% -> Emergency
    /// - >=95%  -> Critical
    /// - >=90%  -> Warn
    /// - otherwise -> None
    pub fn check_budget_utilization(&self, used: f64, total: f64) -> Option<FinancialAlert> {
        if total <= 0.0 {
            return None;
        }
        let ratio = used / total;
        if ratio >= 1.0 {
            Some(self.make_alert(
                AlertLevel::Emergency,
                "budget_utilization",
                used,
                total,
                format!(
                    "Budget EXCEEDED: ${:.4} used of ${:.2} total ({:.1}%)",
                    used, total, ratio * 100.0,
                ),
            ))
        } else if ratio >= 0.95 {
            Some(self.make_alert(
                AlertLevel::Critical,
                "budget_utilization",
                used,
                total,
                format!(
                    "Budget nearly exhausted: ${:.4} of ${:.2} ({:.1}%)",
                    used, total, ratio * 100.0,
                ),
            ))
        } else if ratio >= self.budget_utilization_warn_pct {
            Some(self.make_alert(
                AlertLevel::Warn,
                "budget_utilization",
                used,
                total,
                format!(
                    "Budget utilization {:.1}%: ${:.4} of ${:.2}",
                    ratio * 100.0, used, total,
                ),
            ))
        } else {
            None
        }
    }

    // ---------------------------------------------------------------
    // Aggregate evaluation
    // ---------------------------------------------------------------

    /// Run all 7 indicator checks against a financial snapshot.
    /// Returns all alerts that fired, sorted by severity (highest first).
    pub fn evaluate_all(&self, snapshot: &FinancialSnapshot) -> Vec<FinancialAlert> {
        let mut alerts = Vec::new();

        if let Some(a) = self.check_daily_spend(snapshot.daily_spend, snapshot.daily_limit) {
            alerts.push(a);
        }
        if let Some(a) = self.check_api_revenue_ratio(snapshot.api_cost, snapshot.revenue) {
            alerts.push(a);
        }
        if let Some(a) = self.check_project_margin(snapshot.revenue, snapshot.project_cost) {
            alerts.push(a);
        }
        if let Some(a) = self.check_cash_runway(snapshot.cash_balance, snapshot.monthly_burn) {
            alerts.push(a);
        }
        if let Some(a) = self.check_cost_spike(snapshot.current_period_cost, snapshot.average_cost) {
            alerts.push(a);
        }
        if let Some(a) = self.check_revenue_decline(snapshot.revenue, snapshot.previous_revenue) {
            alerts.push(a);
        }
        if let Some(a) = self.check_budget_utilization(snapshot.budget_used, snapshot.budget_total) {
            alerts.push(a);
        }

        // Sort by severity descending (Emergency first)
        alerts.sort_by(|a, b| b.level.cmp(&a.level));
        alerts
    }

    /// Returns true if any alert is Critical or Emergency level.
    pub fn has_critical_alerts(alerts: &[FinancialAlert]) -> bool {
        alerts.iter().any(|a| a.level >= AlertLevel::Critical)
    }

    // ---------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------

    fn make_alert(
        &self,
        level: AlertLevel,
        indicator_name: &str,
        current_value: f64,
        threshold: f64,
        message: String,
    ) -> FinancialAlert {
        FinancialAlert {
            level,
            indicator_name: indicator_name.to_string(),
            current_value,
            threshold,
            message,
            timestamp: Utc::now(),
        }
    }
}

// ===================================================================
// Tests
// ===================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor() -> FinancialMonitor {
        FinancialMonitor::new()
    }

    // ---------------------------------------------------------------
    // 1. check_daily_spend
    // ---------------------------------------------------------------

    #[test]
    fn daily_spend_under_threshold_no_alert() {
        let m = monitor();
        assert!(m.check_daily_spend(5.0, 10.0).is_none()); // 50%
    }

    #[test]
    fn daily_spend_at_warn_level() {
        let m = monitor();
        let alert = m.check_daily_spend(8.5, 10.0).unwrap(); // 85%
        assert_eq!(alert.level, AlertLevel::Warn);
        assert_eq!(alert.indicator_name, "daily_spend");
    }

    #[test]
    fn daily_spend_at_critical_level() {
        let m = monitor();
        let alert = m.check_daily_spend(9.6, 10.0).unwrap(); // 96%
        assert_eq!(alert.level, AlertLevel::Critical);
    }

    #[test]
    fn daily_spend_exceeded_emergency() {
        let m = monitor();
        let alert = m.check_daily_spend(12.0, 10.0).unwrap(); // 120%
        assert_eq!(alert.level, AlertLevel::Emergency);
    }

    #[test]
    fn daily_spend_zero_limit_no_alert() {
        let m = monitor();
        assert!(m.check_daily_spend(5.0, 0.0).is_none());
    }

    // ---------------------------------------------------------------
    // 2. check_api_revenue_ratio
    // ---------------------------------------------------------------

    #[test]
    fn api_revenue_ratio_healthy() {
        let m = monitor();
        assert!(m.check_api_revenue_ratio(10.0, 100.0).is_none()); // 10%
    }

    #[test]
    fn api_revenue_ratio_warn() {
        let m = monitor();
        let alert = m.check_api_revenue_ratio(30.0, 100.0).unwrap(); // 30%
        assert_eq!(alert.level, AlertLevel::Warn);
    }

    #[test]
    fn api_revenue_ratio_critical() {
        let m = monitor();
        let alert = m.check_api_revenue_ratio(60.0, 100.0).unwrap(); // 60%
        assert_eq!(alert.level, AlertLevel::Critical);
    }

    #[test]
    fn api_revenue_ratio_zero_revenue_emergency() {
        let m = monitor();
        let alert = m.check_api_revenue_ratio(5.0, 0.0).unwrap();
        assert_eq!(alert.level, AlertLevel::Emergency);
    }

    #[test]
    fn api_revenue_ratio_zero_cost_no_alert() {
        let m = monitor();
        assert!(m.check_api_revenue_ratio(0.0, 100.0).is_none());
    }

    // ---------------------------------------------------------------
    // 3. check_project_margin
    // ---------------------------------------------------------------

    #[test]
    fn project_margin_healthy() {
        let m = monitor();
        assert!(m.check_project_margin(100.0, 50.0).is_none()); // 50% margin
    }

    #[test]
    fn project_margin_warn() {
        let m = monitor();
        let alert = m.check_project_margin(100.0, 75.0).unwrap(); // 25% margin
        assert_eq!(alert.level, AlertLevel::Warn);
    }

    #[test]
    fn project_margin_critical() {
        let m = monitor();
        let alert = m.check_project_margin(100.0, 95.0).unwrap(); // 5% margin
        assert_eq!(alert.level, AlertLevel::Critical);
    }

    #[test]
    fn project_margin_zero_revenue_with_cost_emergency() {
        let m = monitor();
        let alert = m.check_project_margin(0.0, 50.0).unwrap();
        assert_eq!(alert.level, AlertLevel::Emergency);
    }

    // ---------------------------------------------------------------
    // 4. check_cash_runway
    // ---------------------------------------------------------------

    #[test]
    fn cash_runway_healthy() {
        let m = monitor();
        assert!(m.check_cash_runway(10000.0, 1000.0).is_none()); // 10 months
    }

    #[test]
    fn cash_runway_warn() {
        let m = monitor();
        let alert = m.check_cash_runway(2500.0, 1000.0).unwrap(); // 2.5 months
        assert_eq!(alert.level, AlertLevel::Warn);
    }

    #[test]
    fn cash_runway_critical() {
        let m = monitor();
        let alert = m.check_cash_runway(1500.0, 1000.0).unwrap(); // 1.5 months
        assert_eq!(alert.level, AlertLevel::Critical);
    }

    #[test]
    fn cash_runway_emergency() {
        let m = monitor();
        let alert = m.check_cash_runway(500.0, 1000.0).unwrap(); // 0.5 months
        assert_eq!(alert.level, AlertLevel::Emergency);
    }

    #[test]
    fn cash_runway_zero_burn_no_alert() {
        let m = monitor();
        assert!(m.check_cash_runway(100.0, 0.0).is_none());
    }

    // ---------------------------------------------------------------
    // 5. check_cost_spike
    // ---------------------------------------------------------------

    #[test]
    fn cost_spike_normal() {
        let m = monitor();
        assert!(m.check_cost_spike(10.0, 10.0).is_none()); // 1x
    }

    #[test]
    fn cost_spike_warn() {
        let m = monitor();
        let alert = m.check_cost_spike(18.0, 10.0).unwrap(); // 1.8x
        assert_eq!(alert.level, AlertLevel::Warn);
    }

    #[test]
    fn cost_spike_critical() {
        let m = monitor();
        let alert = m.check_cost_spike(25.0, 10.0).unwrap(); // 2.5x
        assert_eq!(alert.level, AlertLevel::Critical);
    }

    #[test]
    fn cost_spike_emergency() {
        let m = monitor();
        let alert = m.check_cost_spike(35.0, 10.0).unwrap(); // 3.5x
        assert_eq!(alert.level, AlertLevel::Emergency);
    }

    #[test]
    fn cost_spike_zero_avg_no_alert() {
        let m = monitor();
        assert!(m.check_cost_spike(10.0, 0.0).is_none());
    }

    // ---------------------------------------------------------------
    // 6. check_revenue_decline
    // ---------------------------------------------------------------

    #[test]
    fn revenue_decline_growth_no_alert() {
        let m = monitor();
        assert!(m.check_revenue_decline(120.0, 100.0).is_none()); // grew
    }

    #[test]
    fn revenue_decline_small_dip_no_alert() {
        let m = monitor();
        assert!(m.check_revenue_decline(95.0, 100.0).is_none()); // 5% — under 10%
    }

    #[test]
    fn revenue_decline_warn() {
        let m = monitor();
        let alert = m.check_revenue_decline(85.0, 100.0).unwrap(); // 15%
        assert_eq!(alert.level, AlertLevel::Warn);
    }

    #[test]
    fn revenue_decline_critical() {
        let m = monitor();
        let alert = m.check_revenue_decline(70.0, 100.0).unwrap(); // 30%
        assert_eq!(alert.level, AlertLevel::Critical);
    }

    #[test]
    fn revenue_decline_emergency() {
        let m = monitor();
        let alert = m.check_revenue_decline(40.0, 100.0).unwrap(); // 60%
        assert_eq!(alert.level, AlertLevel::Emergency);
    }

    #[test]
    fn revenue_decline_zero_previous_no_alert() {
        let m = monitor();
        assert!(m.check_revenue_decline(50.0, 0.0).is_none());
    }

    // ---------------------------------------------------------------
    // 7. check_budget_utilization
    // ---------------------------------------------------------------

    #[test]
    fn budget_utilization_healthy() {
        let m = monitor();
        assert!(m.check_budget_utilization(70.0, 100.0).is_none()); // 70%
    }

    #[test]
    fn budget_utilization_warn() {
        let m = monitor();
        let alert = m.check_budget_utilization(92.0, 100.0).unwrap(); // 92%
        assert_eq!(alert.level, AlertLevel::Warn);
    }

    #[test]
    fn budget_utilization_critical() {
        let m = monitor();
        let alert = m.check_budget_utilization(96.0, 100.0).unwrap(); // 96%
        assert_eq!(alert.level, AlertLevel::Critical);
    }

    #[test]
    fn budget_utilization_exceeded() {
        let m = monitor();
        let alert = m.check_budget_utilization(110.0, 100.0).unwrap(); // 110%
        assert_eq!(alert.level, AlertLevel::Emergency);
    }

    #[test]
    fn budget_utilization_zero_total_no_alert() {
        let m = monitor();
        assert!(m.check_budget_utilization(50.0, 0.0).is_none());
    }

    // ---------------------------------------------------------------
    // evaluate_all integration
    // ---------------------------------------------------------------

    #[test]
    fn evaluate_all_healthy_snapshot_no_alerts() {
        let m = monitor();
        let snap = FinancialSnapshot {
            daily_spend: 2.0,
            daily_limit: 10.0,
            api_cost: 5.0,
            revenue: 100.0,
            previous_revenue: 90.0,
            project_cost: 30.0,
            cash_balance: 50000.0,
            monthly_burn: 1000.0,
            current_period_cost: 10.0,
            average_cost: 10.0,
            budget_used: 50.0,
            budget_total: 100.0,
        };
        let alerts = m.evaluate_all(&snap);
        assert!(alerts.is_empty(), "Expected no alerts, got: {:?}", alerts);
    }

    #[test]
    fn evaluate_all_multiple_alerts_sorted_by_severity() {
        let m = monitor();
        let snap = FinancialSnapshot {
            daily_spend: 11.0,       // Emergency: exceeded limit
            daily_limit: 10.0,
            api_cost: 60.0,          // Critical: 60% ratio
            revenue: 100.0,
            previous_revenue: 100.0,
            project_cost: 80.0,      // Warn: 20% margin
            cash_balance: 50000.0,
            monthly_burn: 1000.0,
            current_period_cost: 10.0,
            average_cost: 10.0,
            budget_used: 50.0,
            budget_total: 100.0,
        };
        let alerts = m.evaluate_all(&snap);
        assert!(alerts.len() >= 3, "Expected >=3 alerts, got {}", alerts.len());
        // First alert should be highest severity
        assert_eq!(alerts[0].level, AlertLevel::Emergency);
    }

    #[test]
    fn evaluate_all_worst_case_all_fire() {
        let m = monitor();
        let snap = FinancialSnapshot {
            daily_spend: 15.0,         // Emergency
            daily_limit: 10.0,
            api_cost: 80.0,            // Critical
            revenue: 100.0,
            previous_revenue: 300.0,   // Emergency: 67% decline
            project_cost: 95.0,        // Critical: 5% margin
            cash_balance: 500.0,       // Emergency: 0.5 months
            monthly_burn: 1000.0,
            current_period_cost: 40.0, // Emergency: 4x
            average_cost: 10.0,
            budget_used: 110.0,        // Emergency
            budget_total: 100.0,
        };
        let alerts = m.evaluate_all(&snap);
        assert_eq!(alerts.len(), 7, "Expected all 7 indicators to fire, got {}", alerts.len());
    }

    #[test]
    fn has_critical_alerts_detects_critical() {
        let m = monitor();
        let alerts = vec![m.make_alert(
            AlertLevel::Critical,
            "test",
            1.0,
            1.0,
            "test".to_string(),
        )];
        assert!(FinancialMonitor::has_critical_alerts(&alerts));
    }

    #[test]
    fn has_critical_alerts_false_for_warn_only() {
        let m = monitor();
        let alerts = vec![m.make_alert(
            AlertLevel::Warn,
            "test",
            1.0,
            1.0,
            "test".to_string(),
        )];
        assert!(!FinancialMonitor::has_critical_alerts(&alerts));
    }

    // ---------------------------------------------------------------
    // Edge cases
    // ---------------------------------------------------------------

    #[test]
    fn edge_exact_boundary_daily_spend() {
        let m = monitor();
        // Exactly 80% should trigger Warn
        let alert = m.check_daily_spend(8.0, 10.0).unwrap();
        assert_eq!(alert.level, AlertLevel::Warn);
    }

    #[test]
    fn edge_negative_values_handled() {
        let m = monitor();
        // Negative costs should not trigger (nonsensical input)
        assert!(m.check_cost_spike(-5.0, 10.0).is_none());
        assert!(m.check_cash_runway(-100.0, 1000.0).is_some()); // negative balance = immediate emergency
    }

    #[test]
    fn alert_level_ordering() {
        assert!(AlertLevel::Emergency > AlertLevel::Critical);
        assert!(AlertLevel::Critical > AlertLevel::Warn);
        assert!(AlertLevel::Warn > AlertLevel::Info);
    }

    #[test]
    fn alert_level_display() {
        assert_eq!(format!("{}", AlertLevel::Info), "INFO");
        assert_eq!(format!("{}", AlertLevel::Warn), "WARN");
        assert_eq!(format!("{}", AlertLevel::Critical), "CRITICAL");
        assert_eq!(format!("{}", AlertLevel::Emergency), "EMERGENCY");
    }
}
