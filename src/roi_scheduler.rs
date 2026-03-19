//! ROI-Based Dynamic Scheduler — automatically adjusts cron scheduling frequency
//! for revenue hands based on their ROI performance.
//!
//! High-ROI hands run more often, low-ROI hands run less often or get paused.
//! Tracks per-hand metrics including revenue, cost, success rate, trend, and
//! recommended scheduling frequency tier.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

// ── Trend & Frequency Types ─────────────────────────────────────────────────

/// Direction a hand's ROI is moving over recent executions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trend {
    /// ROI improving >10% compared to prior window
    Improving,
    /// ROI within +/-10% of prior window
    Stable,
    /// ROI declining >10% compared to prior window
    Declining,
    /// Fewer than `min_executions_before_adjust` runs — not enough data
    New,
}

impl std::fmt::Display for Trend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Trend::Improving => write!(f, "Improving"),
            Trend::Stable => write!(f, "Stable"),
            Trend::Declining => write!(f, "Declining"),
            Trend::New => write!(f, "New"),
        }
    }
}

/// Scheduling frequency tier derived from ROI performance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrequencyTier {
    /// Every 2 hours — top performers
    Aggressive,
    /// Every 6 hours — healthy ROI
    Normal,
    /// Once daily — marginal ROI
    Conservative,
    /// Once weekly — still gathering data
    Experimental,
    /// Disabled — losing money
    Paused,
}

impl std::fmt::Display for FrequencyTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrequencyTier::Aggressive => write!(f, "Aggressive (every 2h)"),
            FrequencyTier::Normal => write!(f, "Normal (every 6h)"),
            FrequencyTier::Conservative => write!(f, "Conservative (daily)"),
            FrequencyTier::Experimental => write!(f, "Experimental (weekly)"),
            FrequencyTier::Paused => write!(f, "Paused"),
        }
    }
}

// ── Execution Record ────────────────────────────────────────────────────────

/// A single execution record for rolling-window calculations.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExecutionRecord {
    pub revenue: f64,
    pub cost: f64,
    pub success: bool,
    pub timestamp: DateTime<Utc>,
}

// ── Hand Metrics ────────────────────────────────────────────────────────────

/// Per-hand performance metrics tracked by the scheduler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandMetrics {
    pub hand_name: String,
    pub total_revenue: f64,
    pub total_cost: f64,
    pub execution_count: u32,
    pub success_count: u32,
    /// Revenue from the most recent execution
    pub last_revenue: f64,
    /// (revenue - cost) / cost. `f64::INFINITY` when cost == 0 and revenue > 0.
    pub roi: f64,
    /// Trend direction based on rolling windows
    pub trend: Trend,
    /// Recommended scheduling frequency
    pub recommended_frequency: FrequencyTier,
    /// Timestamp of the most recent execution
    pub last_execution: Option<DateTime<Utc>>,
    /// Rolling window of recent executions for trend calculation
    #[serde(skip)]
    history: Vec<ExecutionRecord>,
}

impl HandMetrics {
    fn new(name: &str) -> Self {
        Self {
            hand_name: name.to_string(),
            total_revenue: 0.0,
            total_cost: 0.0,
            execution_count: 0,
            success_count: 0,
            last_revenue: 0.0,
            roi: 0.0,
            trend: Trend::New,
            recommended_frequency: FrequencyTier::Experimental,
            last_execution: None,
            history: Vec::new(),
        }
    }
}

// ── Configuration ───────────────────────────────────────────────────────────

/// Configuration for ROI-based scheduling thresholds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoiConfig {
    /// Minimum executions before adjusting frequency (default: 5)
    pub min_executions_before_adjust: u32,
    /// ROI above this → Aggressive tier (default: 2.0 = 200%)
    pub aggressive_roi_threshold: f64,
    /// ROI below this → Conservative tier (default: 0.5 = 50%)
    pub conservative_roi_threshold: f64,
    /// ROI below this → Paused (default: -0.5 = losing 50%)
    pub pause_roi_threshold: f64,
    /// Max runs in Experimental before promoting/demoting (default: 10)
    pub experimental_max_runs: u32,
}

impl Default for RoiConfig {
    fn default() -> Self {
        Self {
            min_executions_before_adjust: 5,
            aggressive_roi_threshold: 2.0,
            conservative_roi_threshold: 0.5,
            pause_roi_threshold: -0.5,
            experimental_max_runs: 10,
        }
    }
}

// ── ROI Scheduler ───────────────────────────────────────────────────────────

/// Dynamic scheduler that adjusts hand execution frequency based on ROI.
pub struct RoiScheduler {
    /// Per-hand performance metrics, keyed by hand name
    hand_metrics: Mutex<HashMap<String, HandMetrics>>,
    /// Scheduling configuration / thresholds
    config: RoiConfig,
}

impl RoiScheduler {
    /// Create a new scheduler with default configuration.
    pub fn new() -> Self {
        Self {
            hand_metrics: Mutex::new(HashMap::new()),
            config: RoiConfig::default(),
        }
    }

    /// Create a scheduler with custom configuration.
    pub fn with_config(config: RoiConfig) -> Self {
        Self {
            hand_metrics: Mutex::new(HashMap::new()),
            config,
        }
    }

    /// Record the result of a hand execution.
    pub fn record_execution(&self, hand_name: &str, revenue: f64, cost: f64, success: bool) {
        let mut map = self.hand_metrics.lock().unwrap();
        let metrics = map
            .entry(hand_name.to_string())
            .or_insert_with(|| HandMetrics::new(hand_name));

        metrics.total_revenue += revenue;
        metrics.total_cost += cost;
        metrics.execution_count += 1;
        if success {
            metrics.success_count += 1;
        }
        metrics.last_revenue = revenue;
        metrics.last_execution = Some(Utc::now());

        metrics.history.push(ExecutionRecord {
            revenue,
            cost,
            success,
            timestamp: Utc::now(),
        });

        // Keep only last 20 records for trend calculation
        if metrics.history.len() > 20 {
            let drain_count = metrics.history.len() - 20;
            metrics.history.drain(..drain_count);
        }

        // Recalculate derived fields in-place
        Self::recalculate_metrics(metrics, &self.config);
    }

    /// Recalculate ROI, trend, and recommended frequency for a hand.
    pub fn update_metrics(&self, hand_name: &str) {
        let mut map = self.hand_metrics.lock().unwrap();
        if let Some(metrics) = map.get_mut(hand_name) {
            Self::recalculate_metrics(metrics, &self.config);
        }
    }

    /// Internal: recalculate derived metrics fields.
    fn recalculate_metrics(metrics: &mut HandMetrics, config: &RoiConfig) {
        // ── ROI ──
        if metrics.total_cost > 0.0 {
            metrics.roi = (metrics.total_revenue - metrics.total_cost) / metrics.total_cost;
        } else if metrics.total_revenue > 0.0 {
            metrics.roi = f64::INFINITY;
        } else {
            metrics.roi = 0.0;
        }

        // ── Trend ──
        metrics.trend = Self::calculate_trend(&metrics.history, config.min_executions_before_adjust);

        // ── Frequency recommendation ──
        metrics.recommended_frequency = Self::calculate_frequency(metrics, config);
    }

    /// Calculate trend by comparing ROI of recent 3 executions vs prior 3.
    fn calculate_trend(history: &[ExecutionRecord], min_executions: u32) -> Trend {
        if (history.len() as u32) < min_executions {
            return Trend::New;
        }

        if history.len() < 6 {
            // Not enough data for a meaningful comparison — call it Stable
            return Trend::Stable;
        }

        let len = history.len();
        let recent_window = &history[len - 3..];
        let prior_window = &history[len - 6..len - 3];

        let window_roi = |records: &[ExecutionRecord]| -> f64 {
            let rev: f64 = records.iter().map(|r| r.revenue).sum();
            let cost: f64 = records.iter().map(|r| r.cost).sum();
            if cost > 0.0 {
                (rev - cost) / cost
            } else if rev > 0.0 {
                f64::INFINITY
            } else {
                0.0
            }
        };

        let recent_roi = window_roi(recent_window);
        let prior_roi = window_roi(prior_window);

        // Handle infinite ROIs
        if recent_roi.is_infinite() && prior_roi.is_infinite() {
            return Trend::Stable;
        }
        if recent_roi.is_infinite() {
            return Trend::Improving;
        }
        if prior_roi.is_infinite() {
            return Trend::Declining;
        }

        // Avoid division by zero in relative change
        if prior_roi.abs() < f64::EPSILON {
            if recent_roi > 0.1 {
                return Trend::Improving;
            } else if recent_roi < -0.1 {
                return Trend::Declining;
            } else {
                return Trend::Stable;
            }
        }

        let change = (recent_roi - prior_roi) / prior_roi.abs();

        if change > 0.10 {
            Trend::Improving
        } else if change < -0.10 {
            Trend::Declining
        } else {
            Trend::Stable
        }
    }

    /// Calculate the recommended frequency tier based on metrics and config.
    fn calculate_frequency(metrics: &HandMetrics, config: &RoiConfig) -> FrequencyTier {
        // Still in experimental phase?
        if metrics.execution_count < config.min_executions_before_adjust {
            return FrequencyTier::Experimental;
        }

        let roi = metrics.roi;

        if roi.is_infinite() || roi >= config.aggressive_roi_threshold {
            FrequencyTier::Aggressive
        } else if roi >= config.conservative_roi_threshold {
            FrequencyTier::Normal
        } else if roi >= config.pause_roi_threshold {
            FrequencyTier::Conservative
        } else {
            FrequencyTier::Paused
        }
    }

    /// Get the scheduling recommendation for a specific hand.
    pub fn get_recommendation(&self, hand_name: &str) -> FrequencyTier {
        let map = self.hand_metrics.lock().unwrap();
        map.get(hand_name)
            .map(|m| m.recommended_frequency)
            .unwrap_or(FrequencyTier::Experimental)
    }

    /// Get all hand recommendations sorted by ROI descending.
    /// Returns `(hand_name, frequency_tier, roi)` tuples.
    pub fn get_all_recommendations(&self) -> Vec<(String, FrequencyTier, f64)> {
        let map = self.hand_metrics.lock().unwrap();
        let mut result: Vec<(String, FrequencyTier, f64)> = map
            .values()
            .map(|m| (m.hand_name.clone(), m.recommended_frequency, m.roi))
            .collect();

        // Sort by ROI descending. Infinite ROI sorts first. NaN sorts last.
        result.sort_by(|a, b| {
            b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal)
        });
        result
    }

    /// Determine whether a hand should run now based on its frequency tier
    /// and the time since its last execution.
    pub fn should_run_now(&self, hand_name: &str) -> bool {
        let map = self.hand_metrics.lock().unwrap();
        let metrics = match map.get(hand_name) {
            Some(m) => m,
            None => return true, // Unknown hand → allow first run
        };

        if metrics.recommended_frequency == FrequencyTier::Paused {
            return false;
        }

        let last = match metrics.last_execution {
            Some(ts) => ts,
            None => return true, // Never run → should run
        };

        let elapsed = Utc::now().signed_duration_since(last);
        let required_hours = match metrics.recommended_frequency {
            FrequencyTier::Aggressive => 2,
            FrequencyTier::Normal => 6,
            FrequencyTier::Conservative => 24,
            FrequencyTier::Experimental => 168, // 7 days
            FrequencyTier::Paused => return false,
        };

        elapsed.num_hours() >= required_hours
    }

    /// Return the top N hands by ROI, sorted descending.
    pub fn top_performers(&self, n: usize) -> Vec<HandMetrics> {
        let map = self.hand_metrics.lock().unwrap();
        let mut hands: Vec<HandMetrics> = map.values().cloned().collect();
        hands.sort_by(|a, b| {
            b.roi.partial_cmp(&a.roi).unwrap_or(std::cmp::Ordering::Equal)
        });
        hands.truncate(n);
        hands
    }

    /// Return all hands with negative ROI.
    pub fn underperformers(&self) -> Vec<HandMetrics> {
        let map = self.hand_metrics.lock().unwrap();
        let mut hands: Vec<HandMetrics> = map
            .values()
            .filter(|m| m.roi < 0.0 && m.execution_count >= self.config.min_executions_before_adjust)
            .cloned()
            .collect();
        hands.sort_by(|a, b| {
            a.roi.partial_cmp(&b.roi).unwrap_or(std::cmp::Ordering::Equal)
        });
        hands
    }

    /// Return hand names that have fewer than `min_executions_before_adjust`
    /// runs and still need more data.
    pub fn experiment_candidates(&self) -> Vec<String> {
        let map = self.hand_metrics.lock().unwrap();
        let mut candidates: Vec<String> = map
            .values()
            .filter(|m| m.execution_count < self.config.min_executions_before_adjust)
            .map(|m| m.hand_name.clone())
            .collect();
        candidates.sort();
        candidates
    }

    /// Convert a frequency tier to a cron expression string.
    pub fn to_cron_schedule(tier: FrequencyTier) -> String {
        match tier {
            FrequencyTier::Aggressive => "0 */2 * * *".to_string(),    // every 2 hours
            FrequencyTier::Normal => "0 */6 * * *".to_string(),         // every 6 hours
            FrequencyTier::Conservative => "0 8 * * *".to_string(),     // daily at 08:00
            FrequencyTier::Experimental => "0 10 * * 0".to_string(),    // Sunday 10:00
            FrequencyTier::Paused => "".to_string(),                    // no schedule
        }
    }

    /// Generate a human-readable report of all hands and their scheduling status.
    pub fn generate_schedule_report(&self) -> String {
        let map = self.hand_metrics.lock().unwrap();

        if map.is_empty() {
            return "ROI Scheduler Report\n====================\nNo hands tracked yet.\n"
                .to_string();
        }

        let mut hands: Vec<&HandMetrics> = map.values().collect();
        hands.sort_by(|a, b| {
            b.roi.partial_cmp(&a.roi).unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut report = String::new();
        report.push_str("ROI Scheduler Report\n");
        report.push_str("====================\n\n");
        report.push_str(&format!("Total hands tracked: {}\n", hands.len()));
        report.push_str(&format!(
            "Generated: {}\n\n",
            Utc::now().format("%Y-%m-%d %H:%M UTC")
        ));

        // Summary counts per tier
        let mut tier_counts: HashMap<&str, usize> = HashMap::new();
        for h in &hands {
            let key = match h.recommended_frequency {
                FrequencyTier::Aggressive => "Aggressive",
                FrequencyTier::Normal => "Normal",
                FrequencyTier::Conservative => "Conservative",
                FrequencyTier::Experimental => "Experimental",
                FrequencyTier::Paused => "Paused",
            };
            *tier_counts.entry(key).or_insert(0) += 1;
        }
        report.push_str("Tier Distribution:\n");
        for tier in &["Aggressive", "Normal", "Conservative", "Experimental", "Paused"] {
            let count = tier_counts.get(tier).unwrap_or(&0);
            report.push_str(&format!("  {}: {}\n", tier, count));
        }
        report.push('\n');

        // Per-hand details
        report.push_str("Hand Details (sorted by ROI descending):\n");
        report.push_str(&format!(
            "{:<25} {:>10} {:>10} {:>8} {:>8} {:>12} {:>10} {}\n",
            "Hand", "Revenue", "Cost", "ROI", "Runs", "Success%", "Trend", "Tier"
        ));
        report.push_str(&"-".repeat(100));
        report.push('\n');

        for h in &hands {
            let roi_str = if h.roi.is_infinite() {
                "INF".to_string()
            } else {
                format!("{:.1}%", h.roi * 100.0)
            };
            let success_pct = if h.execution_count > 0 {
                format!(
                    "{:.0}%",
                    (h.success_count as f64 / h.execution_count as f64) * 100.0
                )
            } else {
                "N/A".to_string()
            };
            let cron = Self::to_cron_schedule(h.recommended_frequency);
            let cron_display = if cron.is_empty() {
                "(disabled)".to_string()
            } else {
                cron
            };

            report.push_str(&format!(
                "{:<25} {:>10.2} {:>10.2} {:>8} {:>8} {:>12} {:>10} {} [{}]\n",
                h.hand_name,
                h.total_revenue,
                h.total_cost,
                roi_str,
                h.execution_count,
                success_pct,
                h.trend,
                h.recommended_frequency,
                cron_display,
            ));
        }

        report
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helper ──

    fn scheduler_with_defaults() -> RoiScheduler {
        RoiScheduler::new()
    }

    fn scheduler_min_execs(n: u32) -> RoiScheduler {
        RoiScheduler::with_config(RoiConfig {
            min_executions_before_adjust: n,
            ..Default::default()
        })
    }

    /// Feed `n` identical executions into the scheduler.
    fn feed_n(scheduler: &RoiScheduler, hand: &str, n: u32, revenue: f64, cost: f64, success: bool) {
        for _ in 0..n {
            scheduler.record_execution(hand, revenue, cost, success);
        }
    }

    // ── 1. Record and update metrics ──

    #[test]
    fn test_record_execution_basic() {
        let s = scheduler_with_defaults();
        s.record_execution("seo_content", 10.0, 2.0, true);

        let map = s.hand_metrics.lock().unwrap();
        let m = map.get("seo_content").unwrap();
        assert_eq!(m.execution_count, 1);
        assert_eq!(m.success_count, 1);
        assert!((m.total_revenue - 10.0).abs() < f64::EPSILON);
        assert!((m.total_cost - 2.0).abs() < f64::EPSILON);
        assert!((m.last_revenue - 10.0).abs() < f64::EPSILON);
        assert!(m.last_execution.is_some());
    }

    #[test]
    fn test_record_multiple_executions() {
        let s = scheduler_with_defaults();
        s.record_execution("content", 5.0, 1.0, true);
        s.record_execution("content", 8.0, 1.5, true);
        s.record_execution("content", 0.0, 1.0, false);

        let map = s.hand_metrics.lock().unwrap();
        let m = map.get("content").unwrap();
        assert_eq!(m.execution_count, 3);
        assert_eq!(m.success_count, 2);
        assert!((m.total_revenue - 13.0).abs() < f64::EPSILON);
        assert!((m.total_cost - 3.5).abs() < f64::EPSILON);
        assert!((m.last_revenue - 0.0).abs() < f64::EPSILON);
    }

    // ── 2. ROI calculation ──

    #[test]
    fn test_roi_positive() {
        let s = scheduler_with_defaults();
        // revenue=30, cost=10 → ROI = (30-10)/10 = 2.0
        feed_n(&s, "lead", 5, 6.0, 2.0, true);

        let map = s.hand_metrics.lock().unwrap();
        let m = map.get("lead").unwrap();
        assert!((m.roi - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_roi_negative() {
        let s = scheduler_with_defaults();
        // revenue=5, cost=20 → ROI = (5-20)/20 = -0.75
        feed_n(&s, "outreach", 5, 1.0, 4.0, true);

        let map = s.hand_metrics.lock().unwrap();
        let m = map.get("outreach").unwrap();
        assert!((m.roi - (-0.75)).abs() < 0.01);
    }

    #[test]
    fn test_roi_zero_cost() {
        let s = scheduler_with_defaults();
        // Free provider, some revenue → infinite ROI
        s.record_execution("free_hand", 10.0, 0.0, true);

        let map = s.hand_metrics.lock().unwrap();
        let m = map.get("free_hand").unwrap();
        assert!(m.roi.is_infinite());
    }

    #[test]
    fn test_roi_zero_everything() {
        let s = scheduler_with_defaults();
        s.record_execution("empty_hand", 0.0, 0.0, true);

        let map = s.hand_metrics.lock().unwrap();
        let m = map.get("empty_hand").unwrap();
        assert!((m.roi - 0.0).abs() < f64::EPSILON);
    }

    // ── 3. Trend detection ──

    #[test]
    fn test_trend_new() {
        let s = scheduler_with_defaults(); // min_executions = 5
        s.record_execution("new_hand", 5.0, 1.0, true);

        let map = s.hand_metrics.lock().unwrap();
        assert_eq!(map.get("new_hand").unwrap().trend, Trend::New);
    }

    #[test]
    fn test_trend_improving() {
        let s = scheduler_min_execs(3);
        // Prior window: low revenue
        s.record_execution("improver", 1.0, 1.0, true);
        s.record_execution("improver", 1.0, 1.0, true);
        s.record_execution("improver", 1.0, 1.0, true);
        // Recent window: much higher revenue
        s.record_execution("improver", 10.0, 1.0, true);
        s.record_execution("improver", 10.0, 1.0, true);
        s.record_execution("improver", 10.0, 1.0, true);

        let map = s.hand_metrics.lock().unwrap();
        assert_eq!(map.get("improver").unwrap().trend, Trend::Improving);
    }

    #[test]
    fn test_trend_declining() {
        let s = scheduler_min_execs(3);
        // Prior window: high revenue
        s.record_execution("decliner", 10.0, 1.0, true);
        s.record_execution("decliner", 10.0, 1.0, true);
        s.record_execution("decliner", 10.0, 1.0, true);
        // Recent window: low revenue
        s.record_execution("decliner", 1.0, 1.0, true);
        s.record_execution("decliner", 1.0, 1.0, true);
        s.record_execution("decliner", 1.0, 1.0, true);

        let map = s.hand_metrics.lock().unwrap();
        assert_eq!(map.get("decliner").unwrap().trend, Trend::Declining);
    }

    #[test]
    fn test_trend_stable() {
        let s = scheduler_min_execs(3);
        // Both windows same revenue/cost
        for _ in 0..6 {
            s.record_execution("stable_hand", 5.0, 1.0, true);
        }

        let map = s.hand_metrics.lock().unwrap();
        assert_eq!(map.get("stable_hand").unwrap().trend, Trend::Stable);
    }

    // ── 4. Frequency tier recommendations ──

    #[test]
    fn test_tier_aggressive() {
        let s = scheduler_with_defaults();
        // ROI = (50-10)/10 = 4.0 → well above aggressive_roi_threshold (2.0)
        feed_n(&s, "star", 5, 10.0, 2.0, true);

        assert_eq!(s.get_recommendation("star"), FrequencyTier::Aggressive);
    }

    #[test]
    fn test_tier_normal() {
        let s = scheduler_with_defaults();
        // ROI = (15-10)/10 = 0.5 → at conservative threshold, but Normal is [0.5, 2.0)
        // Actually 0.5 is the conservative boundary. Let's aim for ROI = 1.0
        // revenue=20, cost=10 → ROI = 1.0
        feed_n(&s, "decent", 5, 4.0, 2.0, true);

        assert_eq!(s.get_recommendation("decent"), FrequencyTier::Normal);
    }

    #[test]
    fn test_tier_conservative() {
        let s = scheduler_with_defaults();
        // ROI = (6-5)/5 = 0.2 → between pause_roi_threshold (-0.5) and conservative (0.5)
        feed_n(&s, "marginal", 5, 1.2, 1.0, true);

        assert_eq!(s.get_recommendation("marginal"), FrequencyTier::Conservative);
    }

    #[test]
    fn test_tier_experimental() {
        let s = scheduler_with_defaults();
        // Only 2 executions, min is 5 → Experimental
        s.record_execution("newbie", 100.0, 1.0, true);
        s.record_execution("newbie", 100.0, 1.0, true);

        assert_eq!(s.get_recommendation("newbie"), FrequencyTier::Experimental);
    }

    #[test]
    fn test_tier_paused() {
        let s = scheduler_with_defaults();
        // ROI = (1-10)/10 = -0.9 → below pause_roi_threshold (-0.5)
        feed_n(&s, "loser", 5, 0.2, 2.0, true);

        assert_eq!(s.get_recommendation("loser"), FrequencyTier::Paused);
    }

    #[test]
    fn test_tier_infinite_roi() {
        let s = scheduler_with_defaults();
        // Zero cost, positive revenue → infinite ROI → Aggressive
        feed_n(&s, "freebie", 5, 5.0, 0.0, true);

        assert_eq!(s.get_recommendation("freebie"), FrequencyTier::Aggressive);
    }

    // ── 5. should_run_now ──

    #[test]
    fn test_should_run_unknown_hand() {
        let s = scheduler_with_defaults();
        // Unknown hand → always allow
        assert!(s.should_run_now("never_seen"));
    }

    #[test]
    fn test_should_run_paused_hand() {
        let s = scheduler_with_defaults();
        feed_n(&s, "paused", 5, 0.1, 2.0, true);
        assert_eq!(s.get_recommendation("paused"), FrequencyTier::Paused);
        assert!(!s.should_run_now("paused"));
    }

    #[test]
    fn test_should_run_just_executed() {
        let s = scheduler_with_defaults();
        // Record execution → last_execution = now → should NOT run again immediately
        // for Normal tier (6h interval)
        feed_n(&s, "recent", 5, 4.0, 2.0, true); // ROI=1.0 → Normal
        assert_eq!(s.get_recommendation("recent"), FrequencyTier::Normal);
        // Just executed → elapsed ~0h < 6h → should not run
        assert!(!s.should_run_now("recent"));
    }

    // ── 6. top_performers and underperformers ──

    #[test]
    fn test_top_performers() {
        let s = scheduler_with_defaults();
        feed_n(&s, "high_roi", 5, 20.0, 1.0, true);
        feed_n(&s, "med_roi", 5, 4.0, 2.0, true);
        feed_n(&s, "low_roi", 5, 0.5, 2.0, true);

        let top = s.top_performers(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].hand_name, "high_roi");
        assert_eq!(top[1].hand_name, "med_roi");
    }

    #[test]
    fn test_underperformers() {
        let s = scheduler_with_defaults();
        feed_n(&s, "profitable", 5, 10.0, 1.0, true);
        feed_n(&s, "money_pit", 5, 0.5, 5.0, true);
        feed_n(&s, "barely_negative", 5, 0.9, 1.0, true);

        let under = s.underperformers();
        assert!(!under.is_empty());
        // money_pit: ROI = (2.5 - 25)/25 = -0.9
        assert!(under.iter().any(|m| m.hand_name == "money_pit"));
        // profitable should NOT be here
        assert!(!under.iter().any(|m| m.hand_name == "profitable"));
    }

    // ── 7. experiment_candidates ──

    #[test]
    fn test_experiment_candidates() {
        let s = scheduler_with_defaults(); // min_executions = 5
        s.record_execution("newbie_a", 1.0, 0.5, true);
        s.record_execution("newbie_b", 2.0, 0.5, true);
        feed_n(&s, "veteran", 5, 5.0, 1.0, true);

        let candidates = s.experiment_candidates();
        assert!(candidates.contains(&"newbie_a".to_string()));
        assert!(candidates.contains(&"newbie_b".to_string()));
        assert!(!candidates.contains(&"veteran".to_string()));
    }

    // ── 8. cron schedule generation ──

    #[test]
    fn test_cron_schedule_generation() {
        assert_eq!(RoiScheduler::to_cron_schedule(FrequencyTier::Aggressive), "0 */2 * * *");
        assert_eq!(RoiScheduler::to_cron_schedule(FrequencyTier::Normal), "0 */6 * * *");
        assert_eq!(RoiScheduler::to_cron_schedule(FrequencyTier::Conservative), "0 8 * * *");
        assert_eq!(RoiScheduler::to_cron_schedule(FrequencyTier::Experimental), "0 10 * * 0");
        assert_eq!(RoiScheduler::to_cron_schedule(FrequencyTier::Paused), "");
    }

    // ── 9. Realistic scale: 48 hands ──

    #[test]
    fn test_48_hands_scale() {
        let s = scheduler_with_defaults();
        let hand_names: Vec<String> = (0..48).map(|i| format!("hand_{:02}", i)).collect();

        for (i, name) in hand_names.iter().enumerate() {
            let revenue = (i as f64 + 1.0) * 2.0;
            let cost = 3.0;
            feed_n(&s, name, 6, revenue, cost, true);
        }

        // All 48 should be tracked
        let all = s.get_all_recommendations();
        assert_eq!(all.len(), 48);

        // First should be highest ROI
        assert_eq!(all[0].0, "hand_47");
        // Last should be lowest ROI
        assert_eq!(all[47].0, "hand_00");

        // Top 5 should work
        let top5 = s.top_performers(5);
        assert_eq!(top5.len(), 5);

        // Report should contain all 48 hands
        let report = s.generate_schedule_report();
        assert!(report.contains("Total hands tracked: 48"));
        for name in &hand_names {
            assert!(report.contains(name.as_str()));
        }
    }

    // ── 10. Config defaults ──

    #[test]
    fn test_config_defaults() {
        let cfg = RoiConfig::default();
        assert_eq!(cfg.min_executions_before_adjust, 5);
        assert!((cfg.aggressive_roi_threshold - 2.0).abs() < f64::EPSILON);
        assert!((cfg.conservative_roi_threshold - 0.5).abs() < f64::EPSILON);
        assert!((cfg.pause_roi_threshold - (-0.5)).abs() < f64::EPSILON);
        assert_eq!(cfg.experimental_max_runs, 10);
    }

    #[test]
    fn test_custom_config() {
        let cfg = RoiConfig {
            min_executions_before_adjust: 3,
            aggressive_roi_threshold: 1.0,
            conservative_roi_threshold: 0.2,
            pause_roi_threshold: -0.3,
            experimental_max_runs: 5,
        };
        let s = RoiScheduler::with_config(cfg);
        // With lower aggressive threshold, ROI=1.0 should be Aggressive
        feed_n(&s, "custom", 3, 4.0, 2.0, true); // ROI = (12-6)/6 = 1.0
        assert_eq!(s.get_recommendation("custom"), FrequencyTier::Aggressive);
    }

    // ── 11. get_all_recommendations sorting ──

    #[test]
    fn test_all_recommendations_sorted() {
        let s = scheduler_with_defaults();
        feed_n(&s, "low", 5, 1.0, 2.0, true);   // ROI = -0.5
        feed_n(&s, "mid", 5, 3.0, 2.0, true);   // ROI = 0.5
        feed_n(&s, "high", 5, 10.0, 2.0, true);  // ROI = 4.0

        let recs = s.get_all_recommendations();
        assert_eq!(recs.len(), 3);
        assert_eq!(recs[0].0, "high");
        assert_eq!(recs[1].0, "mid");
        assert_eq!(recs[2].0, "low");
    }

    // ── 12. update_metrics explicit call ──

    #[test]
    fn test_update_metrics_explicit() {
        let s = scheduler_with_defaults();
        feed_n(&s, "explicit", 5, 5.0, 1.0, true);

        // Manually tweak total_revenue inside the lock, then call update_metrics
        {
            let mut map = s.hand_metrics.lock().unwrap();
            let m = map.get_mut("explicit").unwrap();
            m.total_revenue = 100.0; // artificially boost revenue
        }
        s.update_metrics("explicit");

        let map = s.hand_metrics.lock().unwrap();
        let m = map.get("explicit").unwrap();
        // ROI = (100 - 5) / 5 = 19.0
        assert!((m.roi - 19.0).abs() < 0.01);
    }

    // ── 13. Generate schedule report ──

    #[test]
    fn test_generate_report_empty() {
        let s = scheduler_with_defaults();
        let report = s.generate_schedule_report();
        assert!(report.contains("No hands tracked yet"));
    }

    #[test]
    fn test_generate_report_with_data() {
        let s = scheduler_with_defaults();
        feed_n(&s, "seo_content", 6, 8.0, 1.0, true);
        feed_n(&s, "outreach", 6, 0.5, 2.0, true);

        let report = s.generate_schedule_report();
        assert!(report.contains("ROI Scheduler Report"));
        assert!(report.contains("seo_content"));
        assert!(report.contains("outreach"));
        assert!(report.contains("Total hands tracked: 2"));
        assert!(report.contains("Tier Distribution:"));
    }

    // ── 14. History pruning ──

    #[test]
    fn test_history_pruning_at_20() {
        let s = scheduler_with_defaults();
        // Feed 30 executions — should keep only last 20
        feed_n(&s, "pruned", 30, 5.0, 1.0, true);

        let map = s.hand_metrics.lock().unwrap();
        let m = map.get("pruned").unwrap();
        assert_eq!(m.execution_count, 30); // counter tracks all
        assert_eq!(m.history.len(), 20);   // window is bounded
    }

    // ── 15. Mixed success/failure ──

    #[test]
    fn test_mixed_success_failure() {
        let s = scheduler_with_defaults();
        s.record_execution("mixed", 10.0, 2.0, true);
        s.record_execution("mixed", 0.0, 2.0, false);
        s.record_execution("mixed", 5.0, 2.0, true);
        s.record_execution("mixed", 0.0, 2.0, false);
        s.record_execution("mixed", 8.0, 2.0, true);

        let map = s.hand_metrics.lock().unwrap();
        let m = map.get("mixed").unwrap();
        assert_eq!(m.execution_count, 5);
        assert_eq!(m.success_count, 3);
        assert!((m.total_revenue - 23.0).abs() < f64::EPSILON);
        assert!((m.total_cost - 10.0).abs() < f64::EPSILON);
        // ROI = (23 - 10) / 10 = 1.3
        assert!((m.roi - 1.3).abs() < 0.01);
    }

    // ── 16. Recommendation for unknown hand ──

    #[test]
    fn test_recommendation_unknown_hand() {
        let s = scheduler_with_defaults();
        assert_eq!(s.get_recommendation("does_not_exist"), FrequencyTier::Experimental);
    }

    // ── 17. FrequencyTier and Trend Display ──

    #[test]
    fn test_display_traits() {
        assert_eq!(format!("{}", FrequencyTier::Aggressive), "Aggressive (every 2h)");
        assert_eq!(format!("{}", FrequencyTier::Paused), "Paused");
        assert_eq!(format!("{}", Trend::Improving), "Improving");
        assert_eq!(format!("{}", Trend::New), "New");
    }
}
