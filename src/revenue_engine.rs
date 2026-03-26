//! Revenue Engine — 持續營利自動化引擎
//! 監控營收/成本, 計算 ROI, 自動調整排程, 觸發告警, 生成儀表板
//!
//! 核心職責:
//! 1. ROI 計算: 每條路線的投資報酬率
//! 2. 優化迴圈: 每日 22:00 分析並調整排程
//! 3. 週結算: 利潤分配 (60% 擴展 / 20% API / 20% 工具)
//! 4. 失敗恢復: 連續零營收觸發診斷
//! 5. 儀表板: Telegram + HTTP 數據展示

use anyhow::Result;
use chrono::{DateTime, Utc, Duration as ChronoDuration};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info};

use crate::cost_tracker::CostTracker;
use crate::revenue_tracker::{RevenueTracker, ALL_ROUTES};

// ── Route-to-Hand Mapping ─────────────────────────────────────────────────────

/// Returns the list of Hands associated with a revenue route.
pub fn route_hands(route: &str) -> Vec<&'static str> {
    match route {
        "A:freelance_dev"        => vec!["freelancer"],
        "B:saas_products"        => vec!["product_spec", "code_gen", "saas_deploy"],
        "C:content_monetization" => vec!["seo_content", "content"],
        "D:consulting"           => vec!["market_intel", "lead", "outreach"],
        "E:api_services"         => vec!["code_gen", "saas_deploy"],
        "F:affiliate_marketing"  => vec!["seo_content", "content"],
        "G:digital_products"     => vec!["product_spec", "content"],
        "H:automation_services"  => vec!["lead", "outreach", "market_intel"],
        "I:data_services"        => vec!["market_intel"],
        "J:training_education"   => vec!["content", "seo_content"],
        _ => vec![],
    }
}

/// Total number of hand slots across all routes (for proportional cost allocation).
fn total_hand_slots() -> f64 {
    ALL_ROUTES.iter()
        .map(|r| route_hands(r).len())
        .sum::<usize>() as f64
}

// ── Alert System ──────────────────────────────────────────────────────────────

/// Alert severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertLevel {
    Info,
    Warning,
    Critical,
    Emergency,
}

/// A single alert produced by the engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub level: AlertLevel,
    pub route: Option<String>,
    pub message: String,
    pub timestamp: DateTime<Utc>,
    pub suggested_action: Option<String>,
}

// ── ROI Data ──────────────────────────────────────────────────────────────────

/// Per-route ROI analysis result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteROI {
    pub route: String,
    pub revenue_7d: f64,
    pub cost_7d: f64,
    /// (revenue - cost) / cost. `f64::INFINITY` when cost == 0 but revenue > 0.
    pub roi: f64,
    pub daily_avg_revenue: f64,
    pub zero_revenue_days: u32,
    pub trend: TrendDirection,
}

/// Revenue trend direction over a 7-day window
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrendDirection {
    Rising,
    Stable,
    Falling,
    Inactive,
}

// ── Budget ────────────────────────────────────────────────────────────────────

/// Current budget allocation state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetState {
    /// Expansion fund in TWD
    pub expansion_fund_twd: f64,
    /// Accumulated API budget in USD
    pub api_budget_usd: f64,
    /// Accumulated tools budget in USD
    pub tools_budget_usd: f64,
    /// Per-day API spending limit
    pub daily_api_limit_usd: f64,
    /// Hard daily cap (never exceeded)
    pub daily_hard_limit_usd: f64,
    /// When the last weekly settlement ran
    pub last_settlement: DateTime<Utc>,
}

impl Default for BudgetState {
    fn default() -> Self {
        Self {
            expansion_fund_twd: 0.0,
            api_budget_usd: 0.0,
            tools_budget_usd: 0.0,
            daily_api_limit_usd: 5.0,
            daily_hard_limit_usd: 20.0,
            last_settlement: Utc::now(),
        }
    }
}

// ── Optimization Decisions ────────────────────────────────────────────────────

/// The output of one optimization cycle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationDecision {
    pub timestamp: DateTime<Utc>,
    pub route_adjustments: Vec<RouteAdjustment>,
    pub provider_switches: Vec<ProviderSwitch>,
    pub alerts: Vec<Alert>,
    pub budget_update: Option<BudgetState>,
    pub summary: String,
}

/// An adjustment to be applied to a revenue route's scheduling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteAdjustment {
    pub route: String,
    pub action: AdjustmentAction,
    pub reason: String,
}

/// Types of schedule adjustments
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdjustmentAction {
    /// Increase execution frequency by multiplier (e.g. 1.5x)
    IncreaseFrequency { multiplier: f64 },
    /// Decrease execution frequency by multiplier (e.g. 0.75x)
    DecreaseFrequency { multiplier: f64 },
    /// Pause all cron jobs for this route
    Pause,
    /// Resume paused cron jobs
    Resume,
    /// Reduce to minimum frequency (once per week)
    MinimumFrequency,
    /// Trigger diagnostic analysis
    Diagnose,
}

/// A provider switch recommendation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSwitch {
    pub from_provider: String,
    pub to_provider: String,
    pub reason: String,
}

// ── Dashboard ─────────────────────────────────────────────────────────────────

/// Full dashboard data for rendering (Telegram or HTTP)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardData {
    pub generated_at: DateTime<Utc>,
    // Today
    pub today_revenue: f64,
    pub today_cost: f64,
    pub today_net: f64,
    pub today_transactions: u32,
    pub today_llm_calls: u32,
    // This week
    pub week_revenue: f64,
    pub week_cost: f64,
    pub week_net: f64,
    // Route rankings
    pub route_rankings: Vec<RouteROI>,
    // Daily trend (date, revenue)
    pub daily_trend: Vec<(String, f64)>,
    // Budget
    pub budget: BudgetState,
    // Schedule entries
    pub tomorrow_schedule: Vec<ScheduleEntry>,
    // Active alerts
    pub active_alerts: Vec<Alert>,
}

/// A single schedule entry for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleEntry {
    /// Display time in TWD timezone (e.g. "05:00")
    pub time_twd: String,
    pub hand_name: String,
    pub description: String,
}

// ── Engine Configuration ──────────────────────────────────────────────────────

/// Configuration for the revenue engine (loaded from agents.toml)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevenueEngineConfig {
    /// Default daily API budget in USD
    #[serde(default = "default_daily_limit")]
    pub daily_api_limit: f64,
    /// Hard daily cap
    #[serde(default = "default_hard_limit")]
    pub daily_hard_limit: f64,
    /// USD to TWD exchange rate
    #[serde(default = "default_usd_to_twd")]
    pub usd_to_twd: f64,
    /// Expansion fund thresholds in TWD
    #[serde(default = "default_threshold_1")]
    pub expansion_threshold_1: f64,
    #[serde(default = "default_threshold_2")]
    pub expansion_threshold_2: f64,
    /// Profit allocation percentages (should sum to 100)
    #[serde(default = "default_expansion_pct")]
    pub profit_expansion_pct: u32,
    #[serde(default = "default_api_pct")]
    pub profit_api_pct: u32,
    #[serde(default = "default_tools_pct")]
    pub profit_tools_pct: u32,
    /// Alert: consecutive zero-revenue days
    #[serde(default = "default_zero_days")]
    pub zero_revenue_alert_days: u32,
    /// Alert: revenue drop percentage thresholds
    #[serde(default = "default_warning_pct")]
    pub revenue_drop_warning_pct: f64,
    #[serde(default = "default_critical_pct")]
    pub revenue_drop_critical_pct: f64,
    /// Max frequency multiplier
    #[serde(default = "default_max_freq")]
    pub max_frequency_multiplier: f64,
}

fn default_daily_limit() -> f64 { 5.0 }
fn default_hard_limit() -> f64 { 20.0 }
fn default_usd_to_twd() -> f64 { 32.0 }
fn default_threshold_1() -> f64 { 15_000.0 }
fn default_threshold_2() -> f64 { 30_000.0 }
fn default_expansion_pct() -> u32 { 60 }
fn default_api_pct() -> u32 { 20 }
fn default_tools_pct() -> u32 { 20 }
fn default_zero_days() -> u32 { 3 }
fn default_warning_pct() -> f64 { 25.0 }
fn default_critical_pct() -> f64 { 50.0 }
fn default_max_freq() -> f64 { 2.0 }

impl Default for RevenueEngineConfig {
    fn default() -> Self {
        Self {
            daily_api_limit: default_daily_limit(),
            daily_hard_limit: default_hard_limit(),
            usd_to_twd: default_usd_to_twd(),
            expansion_threshold_1: default_threshold_1(),
            expansion_threshold_2: default_threshold_2(),
            profit_expansion_pct: default_expansion_pct(),
            profit_api_pct: default_api_pct(),
            profit_tools_pct: default_tools_pct(),
            zero_revenue_alert_days: default_zero_days(),
            revenue_drop_warning_pct: default_warning_pct(),
            revenue_drop_critical_pct: default_critical_pct(),
            max_frequency_multiplier: default_max_freq(),
        }
    }
}

// ── RevenueEngine ─────────────────────────────────────────────────────────────

/// The core revenue automation engine.
pub struct RevenueEngine {
    revenue_tracker: Arc<RevenueTracker>,
    cost_tracker: Arc<CostTracker>,
    config: RevenueEngineConfig,
    budget: Arc<tokio::sync::RwLock<BudgetState>>,
    alerts: Arc<tokio::sync::RwLock<Vec<Alert>>>,
    /// Maps route -> list of cron job IDs for schedule management
    route_job_ids: Arc<tokio::sync::RwLock<HashMap<String, Vec<String>>>>,
}

impl RevenueEngine {
    /// Create a new engine with default configuration.
    pub fn new(
        revenue_tracker: Arc<RevenueTracker>,
        cost_tracker: Arc<CostTracker>,
    ) -> Self {
        let config = RevenueEngineConfig::default();
        let budget = BudgetState {
            daily_api_limit_usd: config.daily_api_limit,
            daily_hard_limit_usd: config.daily_hard_limit,
            ..Default::default()
        };
        Self {
            revenue_tracker,
            cost_tracker,
            config,
            budget: Arc::new(tokio::sync::RwLock::new(budget)),
            alerts: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            route_job_ids: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    /// Create with explicit configuration.
    pub fn with_config(
        revenue_tracker: Arc<RevenueTracker>,
        cost_tracker: Arc<CostTracker>,
        config: RevenueEngineConfig,
    ) -> Self {
        let budget = BudgetState {
            daily_api_limit_usd: config.daily_api_limit,
            daily_hard_limit_usd: config.daily_hard_limit,
            ..Default::default()
        };
        Self {
            revenue_tracker,
            cost_tracker,
            config,
            budget: Arc::new(tokio::sync::RwLock::new(budget)),
            alerts: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            route_job_ids: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    // ── ROI Calculation ───────────────────────────────────────────────────────

    /// Calculate 7-day ROI for all routes (A-J). Returns sorted by ROI descending.
    pub fn calculate_all_roi(&self) -> Result<Vec<RouteROI>> {
        let revenue_by_route = self.revenue_tracker.by_route(7)?;
        let cost_by_day = self.cost_tracker.by_day(7)?;
        let revenue_by_day = self.revenue_tracker.by_day(7)?;

        let total_cost_7d: f64 = cost_by_day.iter().map(|s| s.total_cost_usd).sum();
        let total_days = revenue_by_day.len().max(1) as f64;
        let total_slots = total_hand_slots().max(1.0);

        let mut results = Vec::new();

        for route_const in ALL_ROUTES {
            let route = route_const.to_string();
            let rev_summary = revenue_by_route.iter().find(|s| s.group == route);
            let revenue_7d = rev_summary.map(|s| s.total_usd).unwrap_or(0.0);

            // Proportional cost allocation based on number of hands for this route
            let hand_count = route_hands(&route).len().max(1) as f64;
            let cost_7d = total_cost_7d * (hand_count / total_slots);

            let roi = if cost_7d > 0.001 {
                (revenue_7d - cost_7d) / cost_7d
            } else if revenue_7d > 0.0 {
                f64::INFINITY
            } else {
                0.0
            };

            let zero_days = self.count_zero_revenue_days_for_route(&route, 30)?;
            let trend = self.calculate_trend_for_route(&route)?;

            results.push(RouteROI {
                route,
                revenue_7d,
                cost_7d,
                roi,
                daily_avg_revenue: revenue_7d / total_days,
                zero_revenue_days: zero_days,
                trend,
            });
        }

        // Sort by ROI descending (Infinity first, then finite, then zero, then negative)
        results.sort_by(|a, b| {
            b.roi.partial_cmp(&a.roi).unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(results)
    }

    /// Count consecutive zero-revenue days for a specific route, looking back N days.
    fn count_zero_revenue_days_for_route(&self, _route: &str, lookback: u32) -> Result<u32> {
        // Check each day going backwards from today
        let now = Utc::now();
        let mut consecutive = 0u32;

        for day_offset in 0..lookback {
            let date = (now - ChronoDuration::days(day_offset as i64))
                .format("%Y-%m-%d")
                .to_string();
            let summary = self.revenue_tracker.summary_for_date(&date)?;
            // Note: summary_for_date returns aggregate across all routes.
            // For per-route zero-day tracking, we'd need a route-filtered query.
            // For now, we use the global summary as an approximation.
            // TODO: Add RevenueTracker::summary_for_date_and_route() for precise tracking.
            if summary.total_usd < 0.01 {
                consecutive += 1;
            } else {
                break;
            }
        }

        Ok(consecutive)
    }

    /// Calculate trend direction for a route by comparing recent 7 days to prior 7 days.
    fn calculate_trend_for_route(&self, _route: &str) -> Result<TrendDirection> {
        let recent = self.revenue_tracker.by_day(7)?;
        let all_14 = self.revenue_tracker.by_day(14)?;

        let recent_sum: f64 = recent.iter().map(|s| s.total_usd).sum();
        let total_14: f64 = all_14.iter().map(|s| s.total_usd).sum();
        let prev_sum = total_14 - recent_sum;

        if recent_sum < 0.01 && prev_sum < 0.01 {
            return Ok(TrendDirection::Inactive);
        }

        if prev_sum < 0.01 {
            return Ok(if recent_sum > 0.0 {
                TrendDirection::Rising
            } else {
                TrendDirection::Inactive
            });
        }

        let change = (recent_sum - prev_sum) / prev_sum;
        Ok(match change {
            c if c > 0.10 => TrendDirection::Rising,
            c if c < -0.10 => TrendDirection::Falling,
            _ => TrendDirection::Stable,
        })
    }

    // ── Optimization Loop ─────────────────────────────────────────────────────

    /// Run the daily optimization loop. Called by the self_optimize cron job (22:00 TWD).
    /// Analyzes ROI data, produces adjustments, alerts, and provider-switch recommendations.
    pub async fn run_optimization_loop(&self) -> Result<OptimizationDecision> {
        info!("RevenueEngine: starting daily optimization loop");

        let roi_data = self.calculate_all_roi()?;
        let today_cost = self.cost_tracker.today_total()?;
        let budget = self.budget.read().await.clone();

        // If there's no data at all, return empty decision
        let has_any_data = roi_data.iter().any(|r| r.revenue_7d > 0.0 || r.cost_7d > 0.001);
        if !has_any_data {
            info!("RevenueEngine: no revenue or cost data, skipping optimization");
            return Ok(OptimizationDecision {
                route_adjustments: Vec::new(),
                provider_switches: Vec::new(),
                alerts: Vec::new(),
                budget_update: None,
                summary: "No data available for optimization.".into(),
                timestamp: Utc::now(),
            });
        }

        let mut adjustments = Vec::new();
        let mut alerts = Vec::new();
        let mut provider_switches = Vec::new();

        // ---- 1. Boost top-performing routes ----
        let top_routes: Vec<&RouteROI> = roi_data.iter()
            .filter(|r| r.roi > 0.0 && r.revenue_7d > 0.0)
            .collect();

        if let Some(best) = top_routes.first() {
            if best.roi > 1.0 {
                let multiplier = (1.5_f64).min(self.config.max_frequency_multiplier);
                adjustments.push(RouteAdjustment {
                    route: best.route.clone(),
                    action: AdjustmentAction::IncreaseFrequency { multiplier },
                    reason: format!(
                        "Highest ROI: {:.0}%, revenue ${:.2}/7d",
                        best.roi * 100.0, best.revenue_7d
                    ),
                });
                info!(
                    "RevenueEngine: boosting route {} (ROI={:.0}%)",
                    best.route, best.roi * 100.0
                );
            }
        }

        // Slightly reduce lower-performing routes (only if they have some revenue)
        for roi in top_routes.iter().skip(2) {
            if roi.revenue_7d > 0.0 {
                adjustments.push(RouteAdjustment {
                    route: roi.route.clone(),
                    action: AdjustmentAction::DecreaseFrequency { multiplier: 0.75 },
                    reason: format!(
                        "Lower ROI: {:.0}%, reducing frequency to save costs",
                        roi.roi * 100.0
                    ),
                });
            }
        }

        // ---- 2. Diagnose dead routes (consecutive zero-revenue days) ----
        for roi in &roi_data {
            if roi.zero_revenue_days >= self.config.zero_revenue_alert_days {
                adjustments.push(RouteAdjustment {
                    route: roi.route.clone(),
                    action: AdjustmentAction::Diagnose,
                    reason: format!(
                        "{} consecutive zero-revenue days",
                        roi.zero_revenue_days
                    ),
                });
                alerts.push(Alert {
                    level: AlertLevel::Critical,
                    route: Some(roi.route.clone()),
                    message: format!(
                        "Route {} has had 0 revenue for {} consecutive days. Diagnostic triggered.",
                        roi.route, roi.zero_revenue_days
                    ),
                    timestamp: Utc::now(),
                    suggested_action: Some(
                        "Check hand execution logs. Consider adjusting strategy or prompt.".into()
                    ),
                });
            }
        }

        // ---- 3. Pause bleeding routes (heavily negative ROI) ----
        for roi in &roi_data {
            if roi.roi < -1.0 && roi.cost_7d > 1.0 {
                adjustments.push(RouteAdjustment {
                    route: roi.route.clone(),
                    action: AdjustmentAction::Pause,
                    reason: format!(
                        "ROI {:.0}%, loss ${:.2} over 7 days",
                        roi.roi * 100.0, roi.cost_7d - roi.revenue_7d
                    ),
                });
                alerts.push(Alert {
                    level: AlertLevel::Critical,
                    route: Some(roi.route.clone()),
                    message: format!(
                        "Route {} paused: ROI={:.0}%, 7-day loss=${:.2}. Use /approve to resume.",
                        roi.route, roi.roi * 100.0, roi.cost_7d - roi.revenue_7d
                    ),
                    timestamp: Utc::now(),
                    suggested_action: Some("/approve to resume or /deny to keep paused".into()),
                });
            }
        }

        // ---- 4. Cost control: switch providers if near budget ----
        let cost_ratio = if budget.daily_api_limit_usd > 0.0 {
            today_cost.total_cost_usd / budget.daily_api_limit_usd
        } else {
            0.0
        };

        if cost_ratio > 0.8 {
            provider_switches.push(ProviderSwitch {
                from_provider: "anthropic".into(),
                to_provider: "lmstudio".into(),
                reason: format!(
                    "Daily cost ${:.2} is {:.0}% of budget ${:.2}. Switching to free provider.",
                    today_cost.total_cost_usd,
                    cost_ratio * 100.0,
                    budget.daily_api_limit_usd
                ),
            });
            alerts.push(Alert {
                level: AlertLevel::Warning,
                route: None,
                message: format!(
                    "API cost ${:.2} near daily limit ${:.2}. Switching to free providers.",
                    today_cost.total_cost_usd, budget.daily_api_limit_usd
                ),
                timestamp: Utc::now(),
                suggested_action: None,
            });
        } else if cost_ratio < 0.3 && budget.daily_api_limit_usd > 2.0 {
            // Under-spending: can upgrade quality
            debug!(
                "RevenueEngine: under budget ({:.0}%), can use higher-quality models",
                cost_ratio * 100.0
            );
        }

        // ---- 5. Overall trend alert ----
        if let Some(trend_alert) = self.check_overall_trend()? {
            alerts.push(trend_alert);
        }

        // Build summary
        let summary = format!(
            "Optimization complete: {} route adjustments, {} alerts, {} provider switches",
            adjustments.len(), alerts.len(), provider_switches.len()
        );
        info!("RevenueEngine: {}", summary);

        // Store alerts (keep most recent 100)
        {
            let mut alert_store = self.alerts.write().await;
            alert_store.extend(alerts.clone());
            if alert_store.len() > 100 {
                let drain = alert_store.len() - 100;
                alert_store.drain(0..drain);
            }
        }

        Ok(OptimizationDecision {
            timestamp: Utc::now(),
            route_adjustments: adjustments,
            provider_switches,
            alerts,
            budget_update: None,
            summary,
        })
    }

    /// Check overall revenue trend (7-day vs prior 7-day).
    fn check_overall_trend(&self) -> Result<Option<Alert>> {
        let recent = self.revenue_tracker.by_day(7)?;
        let all_14 = self.revenue_tracker.by_day(14)?;

        let recent_sum: f64 = recent.iter().map(|s| s.total_usd).sum();
        let total_14: f64 = all_14.iter().map(|s| s.total_usd).sum();
        let prev_sum = total_14 - recent_sum;

        if prev_sum < 0.01 {
            return Ok(None); // Not enough history
        }

        let change_pct = (recent_sum - prev_sum) / prev_sum * 100.0;

        if change_pct < -self.config.revenue_drop_critical_pct {
            Ok(Some(Alert {
                level: AlertLevel::Critical,
                route: None,
                message: format!(
                    "Overall revenue dropped {:.0}% (this week ${:.2} vs last week ${:.2}). Emergency adjustment needed.",
                    change_pct.abs(), recent_sum, prev_sum
                ),
                timestamp: Utc::now(),
                suggested_action: Some(
                    "Pause low-ROI routes, boost high-ROI routes, check for external factors.".into()
                ),
            }))
        } else if change_pct < -self.config.revenue_drop_warning_pct {
            Ok(Some(Alert {
                level: AlertLevel::Warning,
                route: None,
                message: format!(
                    "Overall revenue declined {:.0}% (this week ${:.2} vs last week ${:.2}).",
                    change_pct.abs(), recent_sum, prev_sum
                ),
                timestamp: Utc::now(),
                suggested_action: Some("Monitor trend. If persistent, consider strategy shift.".into()),
            }))
        } else {
            Ok(None)
        }
    }

    // ── Weekly Settlement ─────────────────────────────────────────────────────

    /// Run weekly profit settlement. Allocates profits into expansion/API/tools budgets.
    /// Called every Sunday 23:00 UTC.
    pub async fn weekly_settlement(&self) -> Result<String> {
        let revenue_days = self.revenue_tracker.by_day(7)?;
        let cost_days = self.cost_tracker.by_day(7)?;

        let weekly_revenue: f64 = revenue_days.iter().map(|s| s.total_usd).sum();
        let weekly_cost: f64 = cost_days.iter().map(|s| s.total_cost_usd).sum();
        let net_profit = weekly_revenue - weekly_cost;

        let mut budget = self.budget.write().await;
        let mut report = format!(
            "=== Weekly Settlement ===\n\
             Revenue: ${:.2}\n\
             Cost:    ${:.2}\n\
             Profit:  ${:.2}\n\n",
            weekly_revenue, weekly_cost, net_profit
        );

        if net_profit > 0.0 {
            let expansion_pct = self.config.profit_expansion_pct as f64 / 100.0;
            let api_pct = self.config.profit_api_pct as f64 / 100.0;
            let tools_pct = self.config.profit_tools_pct as f64 / 100.0;

            let expansion = net_profit * expansion_pct * self.config.usd_to_twd;
            let api = net_profit * api_pct;
            let tools = net_profit * tools_pct;

            budget.expansion_fund_twd += expansion;
            budget.api_budget_usd += api;
            budget.tools_budget_usd += tools;

            // Adjust daily API limit (spread over 7 days, cap at 1.5x current)
            let proposed_daily = budget.api_budget_usd / 7.0;
            let max_daily = budget.daily_api_limit_usd * 1.5;
            if proposed_daily > budget.daily_api_limit_usd && proposed_daily <= max_daily {
                budget.daily_api_limit_usd = proposed_daily;
            }

            budget.last_settlement = Utc::now();

            report += &format!(
                "Allocation:\n\
                 - Expansion fund: +NT${:.0} (total NT${:.0})\n\
                 - API budget:     +${:.2} (total ${:.2})\n\
                 - Tools budget:   +${:.2} (total ${:.2})\n\
                 - Daily API limit: ${:.2}\n",
                expansion, budget.expansion_fund_twd,
                api, budget.api_budget_usd,
                tools, budget.tools_budget_usd,
                budget.daily_api_limit_usd,
            );

            // Expansion threshold checks
            if budget.expansion_fund_twd >= self.config.expansion_threshold_2 {
                report += &format!(
                    "\n[EXPANSION] Fund reached NT${:.0} (>= NT${:.0}). Consider 2nd machine!\n",
                    budget.expansion_fund_twd, self.config.expansion_threshold_2
                );
            } else if budget.expansion_fund_twd >= self.config.expansion_threshold_1 {
                report += &format!(
                    "\n[EXPANSION] Fund reached NT${:.0} (>= NT${:.0}). Consider 1st machine!\n",
                    budget.expansion_fund_twd, self.config.expansion_threshold_1
                );
            }
        } else {
            report += "Net profit is negative. No allocation this week.\n";
            report += "Recommendation: reduce paid API usage, increase free provider share.\n";
        }

        info!("RevenueEngine: weekly settlement done. Net profit: ${:.2}", net_profit);
        Ok(report)
    }

    // ── Dashboard ─────────────────────────────────────────────────────────────

    /// Generate full dashboard data.
    pub async fn generate_dashboard(&self) -> Result<DashboardData> {
        let today_rev = self.revenue_tracker.today_total()?;
        let today_cost = self.cost_tracker.today_total()?;
        let week_rev_days = self.revenue_tracker.by_day(7)?;
        let week_cost_days = self.cost_tracker.by_day(7)?;
        let roi_data = self.calculate_all_roi()?;
        let budget = self.budget.read().await.clone();
        let active_alerts = self.alerts.read().await.clone();

        let week_revenue: f64 = week_rev_days.iter().map(|s| s.total_usd).sum();
        let week_cost: f64 = week_cost_days.iter().map(|s| s.total_cost_usd).sum();

        let daily_trend: Vec<(String, f64)> = week_rev_days.iter()
            .map(|s| (s.group.clone(), s.total_usd))
            .collect();

        let tomorrow_schedule = default_schedule_entries();

        Ok(DashboardData {
            generated_at: Utc::now(),
            today_revenue: today_rev.total_usd,
            today_cost: today_cost.total_cost_usd,
            today_net: today_rev.total_usd - today_cost.total_cost_usd,
            today_transactions: today_rev.count,
            today_llm_calls: today_cost.call_count,
            week_revenue,
            week_cost,
            week_net: week_revenue - week_cost,
            route_rankings: roi_data,
            daily_trend,
            budget,
            tomorrow_schedule,
            active_alerts: active_alerts.into_iter()
                .filter(|a| a.level == AlertLevel::Critical || a.level == AlertLevel::Warning)
                .rev()
                .take(10)
                .collect(),
        })
    }

    /// Format dashboard data as a Telegram-friendly text message.
    pub async fn format_dashboard_telegram(&self) -> Result<String> {
        let data = self.generate_dashboard().await?;

        let today_roi = if data.today_cost > 0.001 {
            format!("{:.0}%", ((data.today_revenue - data.today_cost) / data.today_cost) * 100.0)
        } else if data.today_revenue > 0.0 {
            "inf".to_string()
        } else {
            "N/A".to_string()
        };

        let mut text = format!(
            "===== PHANTOM_MESH DASHBOARD =====\n\
             {}\n\n\
             --- Today ---\n\
             Revenue: ${:.2} ({} txns)\n\
             Cost:    ${:.2} ({} LLM calls)\n\
             Net:     ${:.2}\n\
             ROI:     {}\n\n\
             --- This Week ---\n\
             Revenue: ${:.2}\n\
             Cost:    ${:.2}\n\
             Net:     ${:.2}\n\n\
             --- Route Rankings (7d) ---\n",
            Utc::now().format("%Y-%m-%d %H:%M UTC"),
            data.today_revenue, data.today_transactions,
            data.today_cost, data.today_llm_calls,
            data.today_net,
            today_roi,
            data.week_revenue,
            data.week_cost,
            data.week_net,
        );

        // Route rankings (top 5)
        for (i, roi) in data.route_rankings.iter().enumerate().take(5) {
            let roi_str = if roi.roi.is_infinite() {
                "inf".to_string()
            } else {
                format!("{:.0}%", roi.roi * 100.0)
            };
            let trend_icon = match roi.trend {
                TrendDirection::Rising => "^",
                TrendDirection::Falling => "v",
                TrendDirection::Stable => "=",
                TrendDirection::Inactive => "-",
            };
            text += &format!(
                "{}. {} ${:.2}  ROI:{} {}\n",
                i + 1, roi.route, roi.revenue_7d, roi_str, trend_icon
            );
        }

        // Trend chart
        text += "\n--- Trend (7d) ---\n";
        let max_rev = data.daily_trend.iter()
            .map(|(_, v)| *v)
            .fold(0.0f64, f64::max)
            .max(1.0);
        for (date, rev) in &data.daily_trend {
            let bar_len = ((*rev / max_rev) * 20.0) as usize;
            let bar: String = std::iter::repeat('#').take(bar_len).collect();
            let short_date = if date.len() >= 5 { &date[5..] } else { date };
            text += &format!("{}: ${:.2} {}\n", short_date, rev, bar);
        }

        // Budget
        let budget_pct = if data.budget.daily_api_limit_usd > 0.001 {
            (data.today_cost / data.budget.daily_api_limit_usd * 100.0) as u32
        } else {
            0
        };
        text += &format!(
            "\n--- Budget ---\n\
             API:     ${:.2} / ${:.2} ({}%)\n\
             Fund:    NT${:.0} / NT${:.0}\n\
             Tools:   ${:.2}\n",
            data.today_cost, data.budget.daily_api_limit_usd, budget_pct,
            data.budget.expansion_fund_twd, self.config.expansion_threshold_1,
            data.budget.tools_budget_usd,
        );

        // Schedule
        text += "\n--- Schedule ---\n";
        for entry in &data.tomorrow_schedule {
            text += &format!("{} {}\n", entry.time_twd, entry.hand_name);
        }

        // Alerts
        if !data.active_alerts.is_empty() {
            text += "\n--- Alerts ---\n";
            for alert in &data.active_alerts {
                let level = match alert.level {
                    AlertLevel::Critical => "[!]",
                    AlertLevel::Warning  => "[W]",
                    AlertLevel::Emergency => "[E]",
                    _ => "[i]",
                };
                text += &format!("{} {}\n", level, alert.message);
            }
        }

        text += "============================";
        Ok(text)
    }

    // ── Budget Accessors ──────────────────────────────────────────────────────

    /// Get current budget state.
    pub async fn get_budget(&self) -> BudgetState {
        self.budget.read().await.clone()
    }

    /// Update budget state (manual override or from persistence).
    pub async fn update_budget(&self, budget: BudgetState) {
        *self.budget.write().await = budget;
    }

    /// Get recent alerts (most recent first).
    pub async fn recent_alerts(&self, limit: usize) -> Vec<Alert> {
        let alerts = self.alerts.read().await;
        alerts.iter().rev().take(limit).cloned().collect()
    }

    /// Add an alert manually.
    pub async fn push_alert(&self, alert: Alert) {
        let mut alerts = self.alerts.write().await;
        alerts.push(alert);
        if alerts.len() > 100 {
            let drain = alerts.len() - 100;
            alerts.drain(0..drain);
        }
    }

    /// Get configuration reference.
    pub fn config(&self) -> &RevenueEngineConfig {
        &self.config
    }

    /// Register route-to-cron-job mapping.
    pub async fn register_route_job(&self, route: &str, job_id: &str) {
        let mut map = self.route_job_ids.write().await;
        map.entry(route.to_string())
            .or_insert_with(Vec::new)
            .push(job_id.to_string());
    }
}

// ── Default Schedule ──────────────────────────────────────────────────────────

/// Build the default daily schedule entries (for display in dashboard).
pub fn default_schedule_entries() -> Vec<ScheduleEntry> {
    vec![
        ScheduleEntry {
            time_twd: "05:00".into(),
            hand_name: "market_intel".into(),
            description: "Scan market opportunities".into(),
        },
        ScheduleEntry {
            time_twd: "06:00".into(),
            hand_name: "lead".into(),
            description: "Find potential clients".into(),
        },
        ScheduleEntry {
            time_twd: "07:00".into(),
            hand_name: "freelancer".into(),
            description: "Search job opportunities".into(),
        },
        ScheduleEntry {
            time_twd: "08:00".into(),
            hand_name: "seo_content".into(),
            description: "Generate SEO articles".into(),
        },
        ScheduleEntry {
            time_twd: "09:00".into(),
            hand_name: "content".into(),
            description: "Social media content + publish".into(),
        },
        ScheduleEntry {
            time_twd: "10:00".into(),
            hand_name: "outreach".into(),
            description: "Cold email outreach".into(),
        },
        ScheduleEntry {
            time_twd: "12:00".into(),
            hand_name: "trading_analysis".into(),
            description: "Market analysis report".into(),
        },
        ScheduleEntry {
            time_twd: "14:00".into(),
            hand_name: "roi_midcheck".into(),
            description: "Mid-day ROI check".into(),
        },
        ScheduleEntry {
            time_twd: "18:00".into(),
            hand_name: "auto_report".into(),
            description: "Daily operations report".into(),
        },
        ScheduleEntry {
            time_twd: "20:00".into(),
            hand_name: "customer_service".into(),
            description: "Reply to customer inquiries".into(),
        },
        ScheduleEntry {
            time_twd: "22:00".into(),
            hand_name: "self_optimize".into(),
            description: "Analyze performance + adjust schedule".into(),
        },
        ScheduleEntry {
            time_twd: "03:00".into(),
            hand_name: "cluster_evolve".into(),
            description: "Distributed cluster self-evolution".into(),
        },
    ]
}

/// Default cron schedules for the revenue pipeline (UTC times).
/// Returns (name, cron_expr, hand_name, default_input).
pub fn default_cron_schedules() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    vec![
        ("daily_market_intel", "0 21 * * *", "market_intel",
         "Scan AI/SaaS/automation market opportunities, focus on Taiwan and SEA markets"),
        ("daily_lead_gen", "0 22 * * *", "lead",
         "Find companies needing AI automation, LLM integration. Focus: SMBs"),
        ("daily_freelancer", "0 23 * * *", "freelancer",
         "Search Upwork/Fiverr for AI automation, LLM integration, Rust development jobs"),
        ("daily_seo_content", "0 0 * * *", "seo_content",
         "Generate SEO articles about AI automation. Keywords: AI agent, LLM automation, Rust AI"),
        ("daily_content", "0 1 * * *", "content",
         "Generate social media content: AI tool recommendations, automation tips, tech insights"),
        ("daily_outreach", "0 2 * * *", "outreach",
         "Send personalized cold emails based on today's lead results"),
        ("daily_trading", "0 4 * * *", "trading_analysis",
         "Analyze AI stock/crypto trends. Focus: NVIDIA, AMD, AI ETFs"),
        ("daily_report", "0 10 * * *", "auto_report",
         "Generate daily operations report: revenue, costs, ROI, hand execution results"),
        ("daily_customer", "0 12 * * *", "customer_service",
         "Reply to all customer inquiries today. Be polite, professional, guide to conversion"),
        ("daily_optimize", "0 14 * * *", "self_optimize",
         "Analyze today's revenue/cost data, calculate per-route ROI, adjust tomorrow's schedule"),
        ("daily_cluster_evolve", "0 19 * * *", "cluster_evolve",
         "Distributed self-evolution: analyze cluster metrics, dispatch AI improvements, integrate and deploy"),
        // Weekly settlement (Sunday 23:00 UTC = Monday 07:00 TWD)
        ("weekly_settlement", "0 23 * * 0", "auto_report",
         "Weekly settlement: calculate net profit, allocate budgets, check expansion thresholds"),
    ]
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::revenue_tracker::{RevenueRecord, RevenueStatus, ROUTE_A, ROUTE_B, ROUTE_C};
    use crate::cost_tracker::CostRecord;

    fn temp_db(prefix: &str, suffix: &str) -> (String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join("phantom_mesh_test_engine");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join(format!("{}_{}.db", prefix, suffix));
        let _ = std::fs::remove_file(&db_path);
        (db_path.to_str().unwrap().to_string(), db_path)
    }

    fn make_revenue(route: &str, amount: f64) -> RevenueRecord {
        RevenueRecord {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            route: route.to_string(),
            source: "test".to_string(),
            client_name: "TestClient".to_string(),
            amount_usd: amount,
            currency: "USD".to_string(),
            status: RevenueStatus::Confirmed,
            notes: None,
            invoice_id: None,
        }
    }

    fn make_cost(agent: &str, provider: &str, cost: f64) -> CostRecord {
        CostRecord {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            agent: agent.to_string(),
            provider: provider.to_string(),
            model: "test-model".to_string(),
            tokens_in: 500,
            tokens_out: 500,
            total_tokens: 1000,
            node_id: Some("local".to_string()),
            api_estimated_cost_usd: cost,
            hardware_estimated_cost_usd: 0.0,
            estimated_cost_usd: cost,
            duration_secs: 1.0,
            context: None,
        }
    }

    #[test]
    fn test_route_hands_mapping() {
        assert_eq!(route_hands("A:freelance_dev"), vec!["freelancer"]);
        assert_eq!(route_hands("C:content_monetization"), vec!["seo_content", "content"]);
        assert_eq!(route_hands("D:consulting"), vec!["market_intel", "lead", "outreach"]);
        assert!(route_hands("unknown").is_empty());
    }

    #[test]
    fn test_total_hand_slots() {
        let slots = total_hand_slots();
        // Sum of all route hand counts
        assert!(slots > 10.0);
    }

    #[test]
    fn test_calculate_all_roi_empty() {
        let (rev_db, rev_path) = temp_db("roi_empty", "rev");
        let (cost_db, cost_path) = temp_db("roi_empty", "cost");

        let rev = Arc::new(RevenueTracker::new(&rev_db).unwrap());
        let cost = Arc::new(CostTracker::new(&cost_db).unwrap());
        let engine = RevenueEngine::new(rev, cost);

        let results = engine.calculate_all_roi().unwrap();
        assert_eq!(results.len(), 10); // All 10 routes
        for r in &results {
            assert_eq!(r.revenue_7d, 0.0);
            assert_eq!(r.roi, 0.0); // No revenue, no cost
        }

        let _ = std::fs::remove_file(&rev_path);
        let _ = std::fs::remove_file(&cost_path);
    }

    #[test]
    fn test_calculate_all_roi_with_data() {
        let (rev_db, rev_path) = temp_db("roi_data", "rev");
        let (cost_db, cost_path) = temp_db("roi_data", "cost");

        let rev = Arc::new(RevenueTracker::new(&rev_db).unwrap());
        let cost = Arc::new(CostTracker::new(&cost_db).unwrap());

        // Record some revenue
        rev.record(&make_revenue(ROUTE_A, 500.0)).unwrap();
        rev.record(&make_revenue(ROUTE_C, 200.0)).unwrap();

        // Record some costs
        cost.record(&make_cost("master", "ollama", 0.0)).unwrap(); // free
        cost.record(&make_cost("master", "anthropic", 2.0)).unwrap();

        let engine = RevenueEngine::new(Arc::clone(&rev), Arc::clone(&cost));
        let results = engine.calculate_all_roi().unwrap();

        // Route A should have the highest revenue
        let route_a = results.iter().find(|r| r.route == ROUTE_A).unwrap();
        assert_eq!(route_a.revenue_7d, 500.0);
        assert!(route_a.roi > 0.0);

        // Route C should also have revenue
        let route_c = results.iter().find(|r| r.route == ROUTE_C).unwrap();
        assert_eq!(route_c.revenue_7d, 200.0);

        // Route B should have zero revenue
        let route_b = results.iter().find(|r| r.route == ROUTE_B).unwrap();
        assert_eq!(route_b.revenue_7d, 0.0);

        let _ = std::fs::remove_file(&rev_path);
        let _ = std::fs::remove_file(&cost_path);
    }

    #[test]
    fn test_calculate_roi_infinite_when_free() {
        let (rev_db, rev_path) = temp_db("roi_free", "rev");
        let (cost_db, cost_path) = temp_db("roi_free", "cost");

        let rev = Arc::new(RevenueTracker::new(&rev_db).unwrap());
        let cost = Arc::new(CostTracker::new(&cost_db).unwrap());

        rev.record(&make_revenue(ROUTE_A, 100.0)).unwrap();
        // No costs at all

        let engine = RevenueEngine::new(Arc::clone(&rev), Arc::clone(&cost));
        let results = engine.calculate_all_roi().unwrap();

        let route_a = results.iter().find(|r| r.route == ROUTE_A).unwrap();
        assert!(route_a.roi.is_infinite(), "ROI should be infinite when cost is 0 and revenue > 0");

        let _ = std::fs::remove_file(&rev_path);
        let _ = std::fs::remove_file(&cost_path);
    }

    #[test]
    fn test_trend_inactive() {
        let (rev_db, rev_path) = temp_db("trend_inact", "rev");
        let (cost_db, cost_path) = temp_db("trend_inact", "cost");

        let rev = Arc::new(RevenueTracker::new(&rev_db).unwrap());
        let cost = Arc::new(CostTracker::new(&cost_db).unwrap());
        let engine = RevenueEngine::new(rev, cost);

        let trend = engine.calculate_trend_for_route("A:freelance_dev").unwrap();
        assert_eq!(trend, TrendDirection::Inactive);

        let _ = std::fs::remove_file(&rev_path);
        let _ = std::fs::remove_file(&cost_path);
    }

    #[tokio::test]
    async fn test_optimization_loop_empty() {
        let (rev_db, rev_path) = temp_db("opt_empty", "rev");
        let (cost_db, cost_path) = temp_db("opt_empty", "cost");

        let rev = Arc::new(RevenueTracker::new(&rev_db).unwrap());
        let cost = Arc::new(CostTracker::new(&cost_db).unwrap());
        let engine = RevenueEngine::new(rev, cost);

        let decision = engine.run_optimization_loop().await.unwrap();
        assert!(decision.route_adjustments.is_empty(), "No adjustments when no data");
        assert!(decision.provider_switches.is_empty());
        assert!(decision.alerts.is_empty());

        let _ = std::fs::remove_file(&rev_path);
        let _ = std::fs::remove_file(&cost_path);
    }

    #[tokio::test]
    async fn test_optimization_loop_with_revenue() {
        let (rev_db, rev_path) = temp_db("opt_rev", "rev");
        let (cost_db, cost_path) = temp_db("opt_rev", "cost");

        let rev = Arc::new(RevenueTracker::new(&rev_db).unwrap());
        let cost = Arc::new(CostTracker::new(&cost_db).unwrap());

        rev.record(&make_revenue(ROUTE_A, 500.0)).unwrap();
        cost.record(&make_cost("master", "ollama", 0.0)).unwrap();

        let engine = RevenueEngine::new(Arc::clone(&rev), Arc::clone(&cost));
        let decision = engine.run_optimization_loop().await.unwrap();

        // Should recommend boosting route A (infinite ROI)
        let boost = decision.route_adjustments.iter()
            .find(|a| a.route == ROUTE_A);
        assert!(boost.is_some(), "Should boost top-performing route A");

        let _ = std::fs::remove_file(&rev_path);
        let _ = std::fs::remove_file(&cost_path);
    }

    #[tokio::test]
    async fn test_optimization_cost_alert() {
        let (rev_db, rev_path) = temp_db("opt_cost", "rev");
        let (cost_db, cost_path) = temp_db("opt_cost", "cost");

        let rev = Arc::new(RevenueTracker::new(&rev_db).unwrap());
        let cost = Arc::new(CostTracker::new(&cost_db).unwrap());

        // Expensive costs today
        let c = make_cost("master", "anthropic", 4.5);
        cost.record(&c).unwrap();

        let engine = RevenueEngine::new(Arc::clone(&rev), Arc::clone(&cost));
        let decision = engine.run_optimization_loop().await.unwrap();

        // Should trigger cost alert (4.5 > 5.0 * 0.8 = 4.0)
        assert!(!decision.provider_switches.is_empty(), "Should switch provider when near budget");

        let _ = std::fs::remove_file(&rev_path);
        let _ = std::fs::remove_file(&cost_path);
    }

    #[tokio::test]
    async fn test_weekly_settlement_profit() {
        let (rev_db, rev_path) = temp_db("settle_profit", "rev");
        let (cost_db, cost_path) = temp_db("settle_profit", "cost");

        let rev = Arc::new(RevenueTracker::new(&rev_db).unwrap());
        let cost = Arc::new(CostTracker::new(&cost_db).unwrap());

        rev.record(&make_revenue(ROUTE_A, 100.0)).unwrap();
        cost.record(&make_cost("master", "ollama", 0.0)).unwrap();

        let engine = RevenueEngine::new(Arc::clone(&rev), Arc::clone(&cost));
        let report = engine.weekly_settlement().await.unwrap();

        assert!(report.contains("$100.00"), "Report should show revenue");
        assert!(report.contains("Expansion fund"), "Report should show fund allocation");

        let budget = engine.get_budget().await;
        assert!(budget.expansion_fund_twd > 0.0, "Expansion fund should increase");
        assert!(budget.api_budget_usd > 0.0, "API budget should increase");

        let _ = std::fs::remove_file(&rev_path);
        let _ = std::fs::remove_file(&cost_path);
    }

    #[tokio::test]
    async fn test_weekly_settlement_no_profit() {
        let (rev_db, rev_path) = temp_db("settle_noprofit", "rev");
        let (cost_db, cost_path) = temp_db("settle_noprofit", "cost");

        let rev = Arc::new(RevenueTracker::new(&rev_db).unwrap());
        let cost = Arc::new(CostTracker::new(&cost_db).unwrap());

        cost.record(&make_cost("master", "anthropic", 10.0)).unwrap();
        // No revenue

        let engine = RevenueEngine::new(Arc::clone(&rev), Arc::clone(&cost));
        let report = engine.weekly_settlement().await.unwrap();

        assert!(report.contains("negative"), "Report should note negative profit");

        let budget = engine.get_budget().await;
        assert_eq!(budget.expansion_fund_twd, 0.0, "No allocation when unprofitable");

        let _ = std::fs::remove_file(&rev_path);
        let _ = std::fs::remove_file(&cost_path);
    }

    #[tokio::test]
    async fn test_dashboard_generation() {
        let (rev_db, rev_path) = temp_db("dash", "rev");
        let (cost_db, cost_path) = temp_db("dash", "cost");

        let rev = Arc::new(RevenueTracker::new(&rev_db).unwrap());
        let cost = Arc::new(CostTracker::new(&cost_db).unwrap());

        rev.record(&make_revenue(ROUTE_A, 250.0)).unwrap();
        cost.record(&make_cost("master", "ollama", 0.0)).unwrap();

        let engine = RevenueEngine::new(Arc::clone(&rev), Arc::clone(&cost));
        let data = engine.generate_dashboard().await.unwrap();

        assert_eq!(data.today_revenue, 250.0);
        assert_eq!(data.today_transactions, 1);
        assert_eq!(data.route_rankings.len(), 10);
        assert!(!data.tomorrow_schedule.is_empty());

        let _ = std::fs::remove_file(&rev_path);
        let _ = std::fs::remove_file(&cost_path);
    }

    #[tokio::test]
    async fn test_dashboard_telegram_format() {
        let (rev_db, rev_path) = temp_db("dash_tg", "rev");
        let (cost_db, cost_path) = temp_db("dash_tg", "cost");

        let rev = Arc::new(RevenueTracker::new(&rev_db).unwrap());
        let cost = Arc::new(CostTracker::new(&cost_db).unwrap());

        rev.record(&make_revenue(ROUTE_C, 120.0)).unwrap();

        let engine = RevenueEngine::new(Arc::clone(&rev), Arc::clone(&cost));
        let text = engine.format_dashboard_telegram().await.unwrap();

        assert!(text.contains("PHANTOM_MESH DASHBOARD"));
        assert!(text.contains("$120.00"));
        assert!(text.contains("Route Rankings"));
        assert!(text.contains("Schedule"));

        let _ = std::fs::remove_file(&rev_path);
        let _ = std::fs::remove_file(&cost_path);
    }

    #[tokio::test]
    async fn test_alerts_management() {
        let (rev_db, rev_path) = temp_db("alerts", "rev");
        let (cost_db, cost_path) = temp_db("alerts", "cost");

        let rev = Arc::new(RevenueTracker::new(&rev_db).unwrap());
        let cost = Arc::new(CostTracker::new(&cost_db).unwrap());

        let engine = RevenueEngine::new(rev, cost);

        // Push some alerts
        engine.push_alert(Alert {
            level: AlertLevel::Warning,
            route: Some("A:freelance_dev".into()),
            message: "Test warning".into(),
            timestamp: Utc::now(),
            suggested_action: None,
        }).await;

        engine.push_alert(Alert {
            level: AlertLevel::Critical,
            route: None,
            message: "Test critical".into(),
            timestamp: Utc::now(),
            suggested_action: Some("Do something".into()),
        }).await;

        let recent = engine.recent_alerts(10).await;
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].message, "Test critical"); // Most recent first

        let _ = std::fs::remove_file(&rev_path);
        let _ = std::fs::remove_file(&cost_path);
    }

    #[tokio::test]
    async fn test_budget_update() {
        let (rev_db, rev_path) = temp_db("budget", "rev");
        let (cost_db, cost_path) = temp_db("budget", "cost");

        let rev = Arc::new(RevenueTracker::new(&rev_db).unwrap());
        let cost = Arc::new(CostTracker::new(&cost_db).unwrap());

        let engine = RevenueEngine::new(rev, cost);

        let mut budget = engine.get_budget().await;
        assert_eq!(budget.expansion_fund_twd, 0.0);

        budget.expansion_fund_twd = 10_000.0;
        budget.api_budget_usd = 50.0;
        engine.update_budget(budget).await;

        let updated = engine.get_budget().await;
        assert_eq!(updated.expansion_fund_twd, 10_000.0);
        assert_eq!(updated.api_budget_usd, 50.0);

        let _ = std::fs::remove_file(&rev_path);
        let _ = std::fs::remove_file(&cost_path);
    }

    #[test]
    fn test_default_config() {
        let config = RevenueEngineConfig::default();
        assert_eq!(config.daily_api_limit, 5.0);
        assert_eq!(config.daily_hard_limit, 20.0);
        assert_eq!(config.usd_to_twd, 32.0);
        assert_eq!(config.profit_expansion_pct, 60);
        assert_eq!(config.profit_api_pct, 20);
        assert_eq!(config.profit_tools_pct, 20);
        assert_eq!(config.zero_revenue_alert_days, 3);
    }

    #[test]
    fn test_default_cron_schedules() {
        let schedules = default_cron_schedules();
        assert!(schedules.len() >= 10, "Should have at least 10 scheduled jobs");

        // Verify all have valid hand names
        let hand_names: Vec<&str> = schedules.iter().map(|s| s.2).collect();
        assert!(hand_names.contains(&"freelancer"));
        assert!(hand_names.contains(&"market_intel"));
        assert!(hand_names.contains(&"self_optimize"));
    }

    #[test]
    fn test_default_schedule_entries() {
        let entries = default_schedule_entries();
        assert!(entries.len() >= 10);
        assert_eq!(entries[0].time_twd, "05:00");
        assert_eq!(entries[0].hand_name, "market_intel");
    }

    #[test]
    fn test_alert_level_serialization() {
        let alert = Alert {
            level: AlertLevel::Critical,
            route: Some("A:freelance_dev".into()),
            message: "Test".into(),
            timestamp: Utc::now(),
            suggested_action: None,
        };
        let json = serde_json::to_string(&alert).unwrap();
        assert!(json.contains("\"critical\""));

        let parsed: Alert = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.level, AlertLevel::Critical);
    }

    #[test]
    fn test_adjustment_action_serialization() {
        let action = AdjustmentAction::IncreaseFrequency { multiplier: 1.5 };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("increase_frequency"));
        assert!(json.contains("1.5"));

        let parsed: AdjustmentAction = serde_json::from_str(&json).unwrap();
        match parsed {
            AdjustmentAction::IncreaseFrequency { multiplier } => {
                assert!((multiplier - 1.5).abs() < 0.01);
            }
            _ => panic!("Expected IncreaseFrequency"),
        }
    }

    #[test]
    fn test_budget_state_default() {
        let budget = BudgetState::default();
        assert_eq!(budget.expansion_fund_twd, 0.0);
        assert_eq!(budget.daily_api_limit_usd, 5.0);
        assert_eq!(budget.daily_hard_limit_usd, 20.0);
    }

    #[tokio::test]
    async fn test_register_route_job() {
        let (rev_db, rev_path) = temp_db("route_job", "rev");
        let (cost_db, cost_path) = temp_db("route_job", "cost");

        let rev = Arc::new(RevenueTracker::new(&rev_db).unwrap());
        let cost = Arc::new(CostTracker::new(&cost_db).unwrap());
        let engine = RevenueEngine::new(rev, cost);

        engine.register_route_job("A:freelance_dev", "job-001").await;
        engine.register_route_job("A:freelance_dev", "job-002").await;

        let map = engine.route_job_ids.read().await;
        let jobs = map.get("A:freelance_dev").unwrap();
        assert_eq!(jobs.len(), 2);

        let _ = std::fs::remove_file(&rev_path);
        let _ = std::fs::remove_file(&cost_path);
    }

    #[test]
    fn test_optimization_decision_serialization() {
        let decision = OptimizationDecision {
            timestamp: Utc::now(),
            route_adjustments: vec![RouteAdjustment {
                route: "A:freelance_dev".into(),
                action: AdjustmentAction::IncreaseFrequency { multiplier: 1.5 },
                reason: "High ROI".into(),
            }],
            provider_switches: vec![],
            alerts: vec![],
            budget_update: None,
            summary: "test".into(),
        };
        let json = serde_json::to_string(&decision).unwrap();
        assert!(json.contains("A:freelance_dev"));
        assert!(json.contains("increase_frequency"));
    }

    #[test]
    fn test_dashboard_data_serialization() {
        let data = DashboardData {
            generated_at: Utc::now(),
            today_revenue: 100.0,
            today_cost: 5.0,
            today_net: 95.0,
            today_transactions: 3,
            today_llm_calls: 25,
            week_revenue: 500.0,
            week_cost: 20.0,
            week_net: 480.0,
            route_rankings: vec![],
            daily_trend: vec![("2026-03-05".into(), 100.0)],
            budget: BudgetState::default(),
            tomorrow_schedule: default_schedule_entries(),
            active_alerts: vec![],
        };
        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("\"today_revenue\":100.0"));
        assert!(json.contains("2026-03-05"));
    }
}
