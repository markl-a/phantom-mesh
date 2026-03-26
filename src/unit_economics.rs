//! Unit Economics — per-case economics tracking for Phantom Mesh hands.
//!
//! Records revenue, cost, and duration for each hand execution and provides
//! aggregated economics: margin analysis, break-even estimation, and summaries.
//! All data is held in-memory (no SQLite dependency) for fast, lock-free reads.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

// ── Data Point ──────────────────────────────────────────────────────────────────

/// A single execution data point recorded via `record_execution`.
#[derive(Debug, Clone)]
struct ExecutionRecord {
    revenue: f64,
    cost: f64,
    duration_secs: f64,
}

// ── CaseEconomics ───────────────────────────────────────────────────────────────

/// Aggregated economics for a single hand (case type).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseEconomics {
    pub case_id: String,
    pub hand_name: String,
    pub revenue_usd: f64,
    pub cost_usd: f64,
    pub margin_usd: f64,
    pub margin_pct: f64,
    pub execution_count: u32,
    pub avg_duration_secs: f64,
}

// ── EconomicsSummary ────────────────────────────────────────────────────────────

/// High-level summary across all tracked hands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomicsSummary {
    pub total_revenue_usd: f64,
    pub total_cost_usd: f64,
    pub total_margin_usd: f64,
    pub avg_margin_pct: f64,
    pub hand_count: u32,
    pub total_executions: u32,
    pub best_hand: Option<String>,
    pub worst_hand: Option<String>,
}

// ── UnitEconomics ───────────────────────────────────────────────────────────────

/// In-memory per-case economics tracker.
///
/// Thread-safe via interior `Mutex`. Record execution data points with
/// `record_execution` and query aggregated economics via `get_economics`,
/// `all_economics`, `negative_margin_hands`, `break_even_point`, and `summary`.
pub struct UnitEconomics {
    data: Mutex<HashMap<String, Vec<ExecutionRecord>>>,
}

impl UnitEconomics {
    /// Create a new, empty tracker.
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }

    /// Record a single execution data point for a hand.
    ///
    /// - `hand_name`: the hand identifier (e.g. "seo_content")
    /// - `revenue`: revenue earned (USD) from this execution
    /// - `cost`: cost incurred (USD) for this execution
    /// - `duration_secs`: wall-clock seconds the execution took
    pub fn record_execution(
        &self,
        hand_name: &str,
        revenue: f64,
        cost: f64,
        duration_secs: f64,
    ) {
        let mut map = self.data.lock().unwrap();
        map.entry(hand_name.to_string())
            .or_insert_with(Vec::new)
            .push(ExecutionRecord {
                revenue,
                cost,
                duration_secs,
            });
    }

    /// Get aggregated economics for a specific hand.
    /// Returns `None` if no executions have been recorded for `hand_name`.
    pub fn get_economics(&self, hand_name: &str) -> Option<CaseEconomics> {
        let map = self.data.lock().unwrap();
        let records = map.get(hand_name)?;
        if records.is_empty() {
            return None;
        }
        Some(Self::aggregate(hand_name, records))
    }

    /// Get aggregated economics for all tracked hands.
    pub fn all_economics(&self) -> Vec<CaseEconomics> {
        let map = self.data.lock().unwrap();
        let mut result: Vec<CaseEconomics> = map
            .iter()
            .filter(|(_, recs)| !recs.is_empty())
            .map(|(name, recs)| Self::aggregate(name, recs))
            .collect();
        // Sort by margin descending for consistent ordering
        result.sort_by(|a, b| b.margin_usd.partial_cmp(&a.margin_usd).unwrap_or(std::cmp::Ordering::Equal));
        result
    }

    /// Get hands with negative margin (cost exceeds revenue).
    pub fn negative_margin_hands(&self) -> Vec<CaseEconomics> {
        self.all_economics()
            .into_iter()
            .filter(|e| e.margin_usd < 0.0)
            .collect()
    }

    /// Estimate the number of additional executions needed to break even for a hand.
    ///
    /// Uses the **recent trend** (second half of recorded executions) to project
    /// future per-execution margin. If the hand already has a non-negative
    /// cumulative margin, returns `Some(0)`. If the recent trend margin per
    /// execution is zero or negative, returns `None` (break-even unreachable
    /// at current trend). With only one execution, the single record is used
    /// as the trend.
    pub fn break_even_point(&self, hand_name: &str) -> Option<u32> {
        let map = self.data.lock().unwrap();
        let records = map.get(hand_name)?;
        if records.is_empty() {
            return None;
        }

        let total_margin: f64 = records.iter().map(|r| r.revenue - r.cost).sum();

        // Already profitable or at break-even
        if total_margin >= 0.0 {
            return Some(0);
        }

        // Use the second half of executions as the "recent trend".
        // For a single record, use that record itself.
        let n = records.len();
        let trend_start = n / 2; // integer division: for n=1 -> 0, n=2 -> 1, n=5 -> 2
        let trend_slice = &records[trend_start..];
        let trend_count = trend_slice.len() as f64;

        let trend_revenue: f64 = trend_slice.iter().map(|r| r.revenue).sum();
        let trend_cost: f64 = trend_slice.iter().map(|r| r.cost).sum();
        let trend_margin_per_exec = (trend_revenue - trend_cost) / trend_count;

        if trend_margin_per_exec <= 0.0 {
            // Recent trend is flat or worsening — cannot break even
            return None;
        }

        // deficit / trend_margin_per_exec = additional executions needed
        let deficit = -total_margin;
        let needed = (deficit / trend_margin_per_exec).ceil() as u32;
        Some(needed)
    }

    /// Generate a high-level summary across all tracked hands.
    pub fn summary(&self) -> EconomicsSummary {
        let all = self.all_economics();

        if all.is_empty() {
            return EconomicsSummary {
                total_revenue_usd: 0.0,
                total_cost_usd: 0.0,
                total_margin_usd: 0.0,
                avg_margin_pct: 0.0,
                hand_count: 0,
                total_executions: 0,
                best_hand: None,
                worst_hand: None,
            };
        }

        let total_revenue: f64 = all.iter().map(|e| e.revenue_usd).sum();
        let total_cost: f64 = all.iter().map(|e| e.cost_usd).sum();
        let total_margin = total_revenue - total_cost;
        let avg_margin_pct = if total_revenue > 0.0 {
            (total_margin / total_revenue) * 100.0
        } else if total_cost > 0.0 {
            -100.0
        } else {
            0.0
        };
        let total_executions: u32 = all.iter().map(|e| e.execution_count).sum();

        // Best = highest margin_usd, Worst = lowest margin_usd
        // all_economics() is sorted descending by margin, so first = best, last = worst
        let best_hand = all.first().map(|e| e.hand_name.clone());
        let worst_hand = all.last().map(|e| e.hand_name.clone());

        EconomicsSummary {
            total_revenue_usd: total_revenue,
            total_cost_usd: total_cost,
            total_margin_usd: total_margin,
            avg_margin_pct,
            hand_count: all.len() as u32,
            total_executions,
            best_hand,
            worst_hand,
        }
    }

    // ── Internal ────────────────────────────────────────────────────────────────

    /// Aggregate a slice of execution records into a `CaseEconomics`.
    fn aggregate(hand_name: &str, records: &[ExecutionRecord]) -> CaseEconomics {
        let count = records.len() as u32;
        let revenue: f64 = records.iter().map(|r| r.revenue).sum();
        let cost: f64 = records.iter().map(|r| r.cost).sum();
        let margin = revenue - cost;
        let margin_pct = if revenue > 0.0 {
            (margin / revenue) * 100.0
        } else if cost > 0.0 {
            -100.0
        } else {
            0.0
        };
        let total_duration: f64 = records.iter().map(|r| r.duration_secs).sum();
        let avg_duration = total_duration / count as f64;

        CaseEconomics {
            case_id: format!("case_{}", hand_name),
            hand_name: hand_name.to_string(),
            revenue_usd: revenue,
            cost_usd: cost,
            margin_usd: margin,
            margin_pct,
            execution_count: count,
            avg_duration_secs: avg_duration,
        }
    }
}

impl Default for UnitEconomics {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_get_single_execution() {
        let ue = UnitEconomics::new();
        ue.record_execution("seo_content", 50.0, 10.0, 30.0);

        let econ = ue.get_economics("seo_content").unwrap();
        assert_eq!(econ.hand_name, "seo_content");
        assert_eq!(econ.execution_count, 1);
        assert!((econ.revenue_usd - 50.0).abs() < 0.001);
        assert!((econ.cost_usd - 10.0).abs() < 0.001);
        assert!((econ.margin_usd - 40.0).abs() < 0.001);
        assert!((econ.margin_pct - 80.0).abs() < 0.1);
        assert!((econ.avg_duration_secs - 30.0).abs() < 0.001);
    }

    #[test]
    fn test_record_multiple_executions_aggregation() {
        let ue = UnitEconomics::new();
        ue.record_execution("freelancer", 100.0, 20.0, 60.0);
        ue.record_execution("freelancer", 200.0, 30.0, 90.0);
        ue.record_execution("freelancer", 150.0, 25.0, 45.0);

        let econ = ue.get_economics("freelancer").unwrap();
        assert_eq!(econ.execution_count, 3);
        assert!((econ.revenue_usd - 450.0).abs() < 0.001);
        assert!((econ.cost_usd - 75.0).abs() < 0.001);
        assert!((econ.margin_usd - 375.0).abs() < 0.001);
        // avg duration = (60+90+45)/3 = 65.0
        assert!((econ.avg_duration_secs - 65.0).abs() < 0.001);
    }

    #[test]
    fn test_get_economics_nonexistent_hand() {
        let ue = UnitEconomics::new();
        assert!(ue.get_economics("nonexistent").is_none());
    }

    #[test]
    fn test_all_economics_empty() {
        let ue = UnitEconomics::new();
        let all = ue.all_economics();
        assert!(all.is_empty());
    }

    #[test]
    fn test_all_economics_multiple_hands() {
        let ue = UnitEconomics::new();
        ue.record_execution("seo_content", 50.0, 10.0, 30.0);
        ue.record_execution("outreach", 80.0, 15.0, 20.0);
        ue.record_execution("lead", 30.0, 5.0, 10.0);

        let all = ue.all_economics();
        assert_eq!(all.len(), 3);
        // Sorted by margin descending: outreach(65), seo_content(40), lead(25)
        assert_eq!(all[0].hand_name, "outreach");
        assert_eq!(all[1].hand_name, "seo_content");
        assert_eq!(all[2].hand_name, "lead");
    }

    #[test]
    fn test_negative_margin_hands() {
        let ue = UnitEconomics::new();
        ue.record_execution("profitable", 100.0, 20.0, 10.0);
        ue.record_execution("losing", 10.0, 50.0, 10.0);
        ue.record_execution("break_even", 30.0, 30.0, 10.0);

        let neg = ue.negative_margin_hands();
        assert_eq!(neg.len(), 1);
        assert_eq!(neg[0].hand_name, "losing");
        assert!(neg[0].margin_usd < 0.0);
    }

    #[test]
    fn test_negative_margin_hands_empty_when_all_profitable() {
        let ue = UnitEconomics::new();
        ue.record_execution("a", 100.0, 10.0, 5.0);
        ue.record_execution("b", 200.0, 50.0, 5.0);

        let neg = ue.negative_margin_hands();
        assert!(neg.is_empty());
    }

    #[test]
    fn test_break_even_already_profitable() {
        let ue = UnitEconomics::new();
        ue.record_execution("winner", 100.0, 10.0, 5.0);

        let be = ue.break_even_point("winner");
        assert_eq!(be, Some(0));
    }

    #[test]
    fn test_break_even_at_zero_margin() {
        let ue = UnitEconomics::new();
        // Exec 1: revenue=0, cost=100 (setup cost)
        // Execs 2-5: revenue=30, cost=5 each -> margin=+25 each
        // Total: revenue=120, cost=120, margin=0 -> at break-even
        ue.record_execution("growing", 0.0, 100.0, 60.0);
        ue.record_execution("growing", 30.0, 5.0, 10.0);
        ue.record_execution("growing", 30.0, 5.0, 10.0);
        ue.record_execution("growing", 30.0, 5.0, 10.0);
        ue.record_execution("growing", 30.0, 5.0, 10.0);
        // Total margin = 0 -> already at break-even
        let be = ue.break_even_point("growing");
        assert_eq!(be, Some(0));
    }

    #[test]
    fn test_break_even_needs_more_executions() {
        let ue = UnitEconomics::new();
        // Exec 1: rev=0, cost=200 (big setup)     -> margin = -200
        // Exec 2: rev=0, cost=100 (more setup)    -> margin = -100
        // Exec 3: rev=50, cost=10                  -> margin = +40
        // Exec 4: rev=50, cost=10                  -> margin = +40
        // Exec 5: rev=50, cost=10                  -> margin = +40
        // Exec 6: rev=50, cost=10                  -> margin = +40
        // Total: rev=200, cost=340, margin=-140
        // Trend (second half = execs 4-6): 3 execs, each margin=+40, avg=+40
        // Break-even: 140 / 40 = 3.5 -> ceil = 4
        ue.record_execution("startup", 0.0, 200.0, 120.0);
        ue.record_execution("startup", 0.0, 100.0, 90.0);
        ue.record_execution("startup", 50.0, 10.0, 15.0);
        ue.record_execution("startup", 50.0, 10.0, 15.0);
        ue.record_execution("startup", 50.0, 10.0, 15.0);
        ue.record_execution("startup", 50.0, 10.0, 15.0);

        let be = ue.break_even_point("startup");
        assert_eq!(be, Some(4)); // ceil(140/40) = 4
    }

    #[test]
    fn test_break_even_unreachable() {
        let ue = UnitEconomics::new();
        // Every execution loses money: revenue=5, cost=20
        ue.record_execution("money_pit", 5.0, 20.0, 10.0);
        ue.record_execution("money_pit", 5.0, 20.0, 10.0);
        ue.record_execution("money_pit", 5.0, 20.0, 10.0);
        // avg margin per exec = -15, trend is negative -> None
        let be = ue.break_even_point("money_pit");
        assert_eq!(be, None);
    }

    #[test]
    fn test_break_even_nonexistent_hand() {
        let ue = UnitEconomics::new();
        assert!(ue.break_even_point("ghost").is_none());
    }

    #[test]
    fn test_break_even_projected_with_recent_trend() {
        let ue = UnitEconomics::new();
        // Scenario: early losses, then improving trend.
        // Exec 1: rev=0, cost=100 (setup cost)     -> margin = -100
        // Exec 2: rev=0, cost=80  (more setup)     -> margin = -80
        // Exec 3: rev=40, cost=5                   -> margin = +35
        // Exec 4: rev=40, cost=5                   -> margin = +35
        // Total margin = -100 -80 +35 +35 = -110 (still in deficit)
        // Recent trend (second half = execs 3-4): avg margin = +35 per exec
        // Break-even: 110 / 35 = 3.14 -> ceil = 4 additional executions
        ue.record_execution("ramping_up", 0.0, 100.0, 60.0);
        ue.record_execution("ramping_up", 0.0, 80.0, 50.0);
        ue.record_execution("ramping_up", 40.0, 5.0, 10.0);
        ue.record_execution("ramping_up", 40.0, 5.0, 10.0);

        let be = ue.break_even_point("ramping_up");
        assert_eq!(be, Some(4)); // ceil(110/35) = 4
    }

    #[test]
    fn test_break_even_single_losing_execution() {
        let ue = UnitEconomics::new();
        // Single execution that loses money: trend = that single record
        ue.record_execution("sinking", 5.0, 20.0, 10.0);
        // Trend margin per exec = -15 -> unreachable
        assert_eq!(ue.break_even_point("sinking"), None);
    }

    #[test]
    fn test_summary_empty() {
        let ue = UnitEconomics::new();
        let s = ue.summary();
        assert_eq!(s.hand_count, 0);
        assert_eq!(s.total_executions, 0);
        assert!((s.total_revenue_usd).abs() < 0.001);
        assert!(s.best_hand.is_none());
        assert!(s.worst_hand.is_none());
    }

    #[test]
    fn test_summary_with_data() {
        let ue = UnitEconomics::new();
        ue.record_execution("seo_content", 100.0, 10.0, 30.0);
        ue.record_execution("seo_content", 50.0, 5.0, 20.0);
        ue.record_execution("outreach", 80.0, 40.0, 15.0);
        ue.record_execution("lead", 20.0, 50.0, 10.0);

        let s = ue.summary();
        assert_eq!(s.hand_count, 3);
        assert_eq!(s.total_executions, 4);
        // Total revenue: 100+50+80+20 = 250
        assert!((s.total_revenue_usd - 250.0).abs() < 0.001);
        // Total cost: 10+5+40+50 = 105
        assert!((s.total_cost_usd - 105.0).abs() < 0.001);
        // Total margin: 250-105 = 145
        assert!((s.total_margin_usd - 145.0).abs() < 0.001);
        // Best hand: seo_content (margin 135), Worst hand: lead (margin -30)
        assert_eq!(s.best_hand.as_deref(), Some("seo_content"));
        assert_eq!(s.worst_hand.as_deref(), Some("lead"));
    }

    #[test]
    fn test_case_economics_serialization() {
        let econ = CaseEconomics {
            case_id: "case_test".to_string(),
            hand_name: "test".to_string(),
            revenue_usd: 100.0,
            cost_usd: 25.0,
            margin_usd: 75.0,
            margin_pct: 75.0,
            execution_count: 5,
            avg_duration_secs: 12.5,
        };
        let json = serde_json::to_string(&econ).unwrap();
        assert!(json.contains("\"hand_name\":\"test\""));
        assert!(json.contains("\"margin_pct\":75.0"));
        let back: CaseEconomics = serde_json::from_str(&json).unwrap();
        assert_eq!(back.hand_name, "test");
        assert!((back.margin_usd - 75.0).abs() < 0.001);
    }

    #[test]
    fn test_economics_summary_serialization() {
        let s = EconomicsSummary {
            total_revenue_usd: 1000.0,
            total_cost_usd: 200.0,
            total_margin_usd: 800.0,
            avg_margin_pct: 80.0,
            hand_count: 5,
            total_executions: 50,
            best_hand: Some("top_earner".to_string()),
            worst_hand: Some("bottom_feeder".to_string()),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"best_hand\":\"top_earner\""));
        let back: EconomicsSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back.hand_count, 5);
        assert!((back.avg_margin_pct - 80.0).abs() < 0.001);
    }

    #[test]
    fn test_zero_revenue_margin_pct() {
        let ue = UnitEconomics::new();
        // Pure cost, no revenue: margin_pct should be -100%
        ue.record_execution("pure_cost", 0.0, 50.0, 10.0);
        let econ = ue.get_economics("pure_cost").unwrap();
        assert!((econ.margin_pct - (-100.0)).abs() < 0.1);
        assert!((econ.margin_usd - (-50.0)).abs() < 0.001);
    }

    #[test]
    fn test_zero_revenue_zero_cost() {
        let ue = UnitEconomics::new();
        // Free execution: 0 revenue, 0 cost
        ue.record_execution("free_run", 0.0, 0.0, 5.0);
        let econ = ue.get_economics("free_run").unwrap();
        assert!((econ.margin_pct).abs() < 0.001);
        assert!((econ.margin_usd).abs() < 0.001);
    }

    #[test]
    fn test_default_trait() {
        let ue = UnitEconomics::default();
        assert!(ue.all_economics().is_empty());
    }

    #[test]
    fn test_case_id_format() {
        let ue = UnitEconomics::new();
        ue.record_execution("market_intel", 200.0, 30.0, 45.0);
        let econ = ue.get_economics("market_intel").unwrap();
        assert_eq!(econ.case_id, "case_market_intel");
    }
}
