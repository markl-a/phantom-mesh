//! Provider Budget Optimizer — tracks daily/monthly API usage per provider against
//! free tier limits and recommends optimal provider selection to maximize free
//! tier utilization before falling back to local models.
//!
//! # Overview
//!
//! Each cloud LLM provider offers a free tier with varying request and token
//! limits.  `ProviderBudget` maintains real-time counters and, given a set of
//! required capabilities, returns the best available provider ordered by
//! priority.  When all cloud budgets are exhausted it recommends falling back
//! to a local model (ollama).
//!
//! # Default free-tier limits (as of 2026-03)
//!
//! | Provider     | Daily RPD | Monthly tokens | Priority |
//! |-------------|-----------|----------------|----------|
//! | gemini       | 1 500     | unlimited      | 1        |
//! | groq         | 14 400    | unlimited      | 2        |
//! | mistral      | 33 000    | 1 000 000 000  | 3        |
//! | openrouter   | 200       | unlimited      | 4        |
//! | ollama       | unlimited | unlimited      | 10       |

use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// ProviderLimit
// ---------------------------------------------------------------------------

/// Static configuration for a single provider's free tier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderLimit {
    /// Provider identifier (e.g. "gemini", "groq").
    pub provider_name: String,
    /// Maximum requests allowed per day (0 = unlimited).
    pub daily_requests: u32,
    /// Maximum tokens allowed per day (0 = unlimited).
    pub daily_tokens: u64,
    /// Maximum tokens allowed per calendar month (0 = unlimited).
    pub monthly_tokens: u64,
    /// Cost per 1 000 tokens in USD.  0.0 for free-tier providers.
    pub cost_per_1k_tokens: f64,
    /// Selection priority — lower values are tried first.
    pub priority: u8,
    /// Capabilities offered by this provider (e.g. "tool_calling", "streaming", "vision").
    pub capabilities: Vec<String>,
}

// ---------------------------------------------------------------------------
// DailyUsage
// ---------------------------------------------------------------------------

/// Mutable per-provider usage counters that reset on a daily/monthly basis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyUsage {
    /// Number of API requests made today.
    pub requests_today: u32,
    /// Total tokens consumed today.
    pub tokens_today: u64,
    /// Total tokens consumed this calendar month.
    pub tokens_this_month: u64,
    /// Timestamp of the last daily counter reset.
    pub last_reset: DateTime<Utc>,
    /// Number of API errors recorded today.
    pub errors_today: u32,
}

impl DailyUsage {
    fn new() -> Self {
        Self {
            requests_today: 0,
            tokens_today: 0,
            tokens_this_month: 0,
            last_reset: Utc::now(),
            errors_today: 0,
        }
    }

    #[allow(dead_code)]
    fn new_at(ts: DateTime<Utc>) -> Self {
        Self {
            requests_today: 0,
            tokens_today: 0,
            tokens_this_month: 0,
            last_reset: ts,
            errors_today: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// BudgetRecommendation
// ---------------------------------------------------------------------------

/// Result of asking the budget optimizer which provider to use.
#[derive(Debug, Clone, PartialEq)]
pub enum BudgetRecommendation {
    /// Use this cloud/remote provider.
    UseProvider(String),
    /// All remote APIs are exhausted — fall back to a local model.
    FallbackLocal(String),
    /// Every provider (including local) is rate-limited.  Caller should retry
    /// after the indicated number of seconds.
    RateLimited {
        retry_after_secs: u64,
    },
}

// ---------------------------------------------------------------------------
// BudgetStatus
// ---------------------------------------------------------------------------

/// Per-provider snapshot used by `budget_summary()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetStatus {
    pub provider_name: String,
    pub requests_used: u32,
    pub requests_limit: u32,
    pub tokens_used: u64,
    pub tokens_limit: u64,
    pub utilization_pct: f64,
    pub is_available: bool,
}

// ---------------------------------------------------------------------------
// ProviderBudget
// ---------------------------------------------------------------------------

/// Core budget optimizer.  Thread-safe — the inner `usage` map is behind a
/// `Mutex` so it can be shared across async tasks.
pub struct ProviderBudget {
    /// Static per-provider limits (immutable after construction).
    limits: HashMap<String, ProviderLimit>,
    /// Mutable usage counters, protected by a mutex.
    usage: Mutex<HashMap<String, DailyUsage>>,
}

impl ProviderBudget {
    // -- constructors -------------------------------------------------------

    /// Create an empty `ProviderBudget` with no provider limits registered.
    pub fn new() -> Self {
        Self {
            limits: HashMap::new(),
            usage: Mutex::new(HashMap::new()),
        }
    }

    /// Create a `ProviderBudget` pre-populated with the documented free-tier
    /// limits for the five standard providers.
    pub fn with_defaults() -> Self {
        let mut limits = HashMap::new();

        limits.insert("gemini".to_string(), ProviderLimit {
            provider_name: "gemini".to_string(),
            daily_requests: 1500,
            daily_tokens: 0,
            monthly_tokens: 0,
            cost_per_1k_tokens: 0.0,
            priority: 1,
            capabilities: vec![
                "tool_calling".to_string(),
                "streaming".to_string(),
                "vision".to_string(),
            ],
        });

        limits.insert("groq".to_string(), ProviderLimit {
            provider_name: "groq".to_string(),
            daily_requests: 14400,
            daily_tokens: 0,
            monthly_tokens: 0,
            cost_per_1k_tokens: 0.0,
            priority: 2,
            capabilities: vec![
                "tool_calling".to_string(),
                "streaming".to_string(),
            ],
        });

        limits.insert("mistral".to_string(), ProviderLimit {
            provider_name: "mistral".to_string(),
            daily_requests: 33000,
            daily_tokens: 0,
            monthly_tokens: 1_000_000_000,
            cost_per_1k_tokens: 0.0,
            priority: 3,
            capabilities: vec![
                "tool_calling".to_string(),
            ],
        });

        limits.insert("openrouter".to_string(), ProviderLimit {
            provider_name: "openrouter".to_string(),
            daily_requests: 200,
            daily_tokens: 0,
            monthly_tokens: 0,
            cost_per_1k_tokens: 0.0,
            priority: 4,
            capabilities: vec![
                "tool_calling".to_string(),
            ],
        });

        limits.insert("ollama".to_string(), ProviderLimit {
            provider_name: "ollama".to_string(),
            daily_requests: 0, // unlimited
            daily_tokens: 0,
            monthly_tokens: 0,
            cost_per_1k_tokens: 0.0,
            priority: 10,
            capabilities: vec![
                "tool_calling".to_string(),
            ],
        });

        Self {
            limits,
            usage: Mutex::new(HashMap::new()),
        }
    }

    /// Register (or overwrite) a provider limit.
    pub fn add_limit(&mut self, limit: ProviderLimit) {
        self.limits.insert(limit.provider_name.clone(), limit);
    }

    // -- recording ----------------------------------------------------------

    /// Record API usage for a provider.
    pub fn record_usage(&self, provider: &str, requests: u32, tokens: u64) {
        let mut map = self.usage.lock().unwrap();
        let entry = map.entry(provider.to_string()).or_insert_with(DailyUsage::new);
        self.maybe_reset_daily(provider, entry);
        entry.requests_today += requests;
        entry.tokens_today += tokens;
        entry.tokens_this_month += tokens;
        debug!(
            provider,
            requests,
            tokens,
            total_requests = entry.requests_today,
            "recorded provider usage"
        );
    }

    /// Record an API error for a provider.
    pub fn record_error(&self, provider: &str) {
        let mut map = self.usage.lock().unwrap();
        let entry = map.entry(provider.to_string()).or_insert_with(DailyUsage::new);
        self.maybe_reset_daily(provider, entry);
        entry.errors_today += 1;
        warn!(provider, errors = entry.errors_today, "recorded provider error");
    }

    // -- recommendation -----------------------------------------------------

    /// Pick the best available provider given a set of required capabilities.
    ///
    /// Providers are considered in priority order (lowest `priority` first).
    /// A provider is skipped if:
    /// - it lacks any of the `required_capabilities`
    /// - its daily request quota is exhausted
    /// - its daily or monthly token quota is exhausted
    ///
    /// If no remote (non-local) provider is available, the method looks for a
    /// local provider (priority >= 10) as a fallback.  If even that is
    /// unavailable, `RateLimited` is returned.
    pub fn recommend(&self, required_capabilities: &[&str]) -> BudgetRecommendation {
        let map = self.usage.lock().unwrap();

        // Collect candidates that satisfy capability requirements.
        let mut candidates: Vec<&ProviderLimit> = self
            .limits
            .values()
            .filter(|lim| {
                required_capabilities.iter().all(|cap| {
                    lim.capabilities.iter().any(|c| c == cap)
                })
            })
            .collect();

        // Sort by priority ascending (lowest = best).
        candidates.sort_by_key(|c| c.priority);

        let mut fallback_local: Option<String> = None;

        for candidate in &candidates {
            let name = &candidate.provider_name;

            // Check if the provider has remaining budget.
            let available = self.is_available_inner(candidate, map.get(name));

            if candidate.priority >= 10 {
                // Local provider — record as fallback.
                if available {
                    fallback_local = Some(name.clone());
                }
                continue;
            }

            if available {
                info!(provider = %name, "budget recommends provider");
                return BudgetRecommendation::UseProvider(name.clone());
            }
        }

        // No remote provider available — try local fallback.
        if let Some(local) = fallback_local {
            info!(provider = %local, "all APIs exhausted, falling back to local");
            return BudgetRecommendation::FallbackLocal(local);
        }

        warn!("all providers exhausted or rate-limited");
        BudgetRecommendation::RateLimited { retry_after_secs: 60 }
    }

    // -- queries ------------------------------------------------------------

    /// Return the remaining daily (requests, tokens) for a provider.
    ///
    /// A limit of 0 means unlimited — the remaining value is reported as
    /// `u32::MAX` / `u64::MAX` respectively.
    pub fn remaining_budget(&self, provider: &str) -> (u32, u64) {
        let map = self.usage.lock().unwrap();
        let limit = match self.limits.get(provider) {
            Some(l) => l,
            None => return (0, 0),
        };
        let usage = map.get(provider);

        let requests_left = if limit.daily_requests == 0 {
            u32::MAX
        } else {
            let used = usage.map(|u| u.requests_today).unwrap_or(0);
            limit.daily_requests.saturating_sub(used)
        };

        let tokens_left = if limit.daily_tokens == 0 {
            u64::MAX
        } else {
            let used = usage.map(|u| u.tokens_today).unwrap_or(0);
            limit.daily_tokens.saturating_sub(used)
        };

        (requests_left, tokens_left)
    }

    /// Return a summary of budget status for every registered provider.
    pub fn budget_summary(&self) -> Vec<BudgetStatus> {
        let map = self.usage.lock().unwrap();
        let mut out: Vec<BudgetStatus> = self
            .limits
            .values()
            .map(|lim| {
                let usage = map.get(&lim.provider_name);
                let requests_used = usage.map(|u| u.requests_today).unwrap_or(0);
                let tokens_used = usage.map(|u| u.tokens_today).unwrap_or(0);

                let utilization = self.utilization_inner(lim, usage);
                let available = self.is_available_inner(lim, usage);

                BudgetStatus {
                    provider_name: lim.provider_name.clone(),
                    requests_used,
                    requests_limit: lim.daily_requests,
                    tokens_used,
                    tokens_limit: lim.daily_tokens,
                    utilization_pct: utilization,
                    is_available: available,
                }
            })
            .collect();

        out.sort_by_key(|s| {
            self.limits
                .get(&s.provider_name)
                .map(|l| l.priority)
                .unwrap_or(u8::MAX)
        });
        out
    }

    /// Reset daily counters for all providers.  Automatically called when a
    /// date change is detected; can also be called manually.
    pub fn daily_reset(&self) {
        let mut map = self.usage.lock().unwrap();
        let now = Utc::now();
        for (name, usage) in map.iter_mut() {
            let old_month = usage.last_reset.month();
            let new_month = now.month();
            usage.requests_today = 0;
            usage.tokens_today = 0;
            usage.errors_today = 0;
            // Reset monthly counters on month change.
            if new_month != old_month {
                usage.tokens_this_month = 0;
            }
            usage.last_reset = now;
            debug!(provider = %name, "daily budget counters reset");
        }
        info!("daily budget reset complete");
    }

    /// Return the utilization percentage (0.0 – 100.0) for a single provider.
    ///
    /// The utilization is the *maximum* of the request-utilization and the
    /// token-utilization so that hitting either limit counts as exhausted.
    /// Providers with unlimited quotas (limit == 0) always report 0.0%.
    pub fn utilization_pct(&self, provider: &str) -> f64 {
        let map = self.usage.lock().unwrap();
        let limit = match self.limits.get(provider) {
            Some(l) => l,
            None => return 0.0,
        };
        self.utilization_inner(limit, map.get(provider))
    }

    /// Distribute `tasks_count` tasks across available providers proportionally
    /// to their remaining daily request budget.
    ///
    /// The result is a list of `(provider_name, allocated_tasks)` sorted by
    /// priority.  Providers with zero remaining budget are omitted.
    pub fn optimal_rotation_plan(&self, tasks_count: u32) -> Vec<(String, u32)> {
        let map = self.usage.lock().unwrap();

        // Gather providers with remaining capacity (skip unlimited-local at prio >= 10
        // unless there are no remote options).
        let mut slots: Vec<(&ProviderLimit, u32)> = Vec::new();

        for lim in self.limits.values() {
            if lim.priority >= 10 {
                continue; // local providers handled as overflow
            }
            let remaining = if lim.daily_requests == 0 {
                u32::MAX
            } else {
                let used = map.get(&lim.provider_name).map(|u| u.requests_today).unwrap_or(0);
                lim.daily_requests.saturating_sub(used)
            };
            if remaining > 0 {
                slots.push((lim, remaining));
            }
        }

        if slots.is_empty() {
            // Nothing remote — give everything to local if available.
            if let Some(local) = self.limits.values().find(|l| l.priority >= 10) {
                return vec![(local.provider_name.clone(), tasks_count)];
            }
            return vec![];
        }

        // Sort by priority.
        slots.sort_by_key(|(lim, _)| lim.priority);

        let total_remaining: u64 = slots.iter().map(|(_, r)| *r as u64).sum();
        let mut allocated: u32 = 0;
        let mut plan: Vec<(String, u32)> = Vec::new();

        for (i, (lim, remaining)) in slots.iter().enumerate() {
            let share = if i == slots.len() - 1 {
                // Last provider gets the remainder to avoid rounding loss.
                tasks_count.saturating_sub(allocated)
            } else {
                let proportion = *remaining as f64 / total_remaining as f64;
                let raw = (tasks_count as f64 * proportion).round() as u32;
                // Don't exceed what's available or the total.
                raw.min(*remaining).min(tasks_count.saturating_sub(allocated))
            };

            if share > 0 {
                plan.push((lim.provider_name.clone(), share));
                allocated += share;
            }
        }

        // If there's overflow (shouldn't normally happen), push to local.
        if allocated < tasks_count {
            if let Some(local) = self.limits.values().find(|l| l.priority >= 10) {
                plan.push((local.provider_name.clone(), tasks_count - allocated));
            }
        }

        plan
    }

    /// Whether a provider has remaining free budget.
    pub fn is_free(&self, provider: &str) -> bool {
        let map = self.usage.lock().unwrap();
        let limit = match self.limits.get(provider) {
            Some(l) => l,
            None => return false,
        };
        self.is_available_inner(limit, map.get(provider))
    }

    // -- internal helpers ---------------------------------------------------

    /// Auto-reset daily counters if the date has changed since the last reset.
    fn maybe_reset_daily(&self, provider: &str, usage: &mut DailyUsage) {
        let now = Utc::now();
        let last = usage.last_reset;
        if now.date_naive() != last.date_naive() {
            debug!(provider, "auto-resetting daily counters (date change)");
            // Reset monthly counters on month rollover.
            if now.month() != last.month() {
                usage.tokens_this_month = 0;
            }
            usage.requests_today = 0;
            usage.tokens_today = 0;
            usage.errors_today = 0;
            usage.last_reset = now;
        }
    }

    /// Inner utilization calculation (no lock acquisition).
    fn utilization_inner(&self, limit: &ProviderLimit, usage: Option<&DailyUsage>) -> f64 {
        let usage = match usage {
            Some(u) => u,
            None => return 0.0,
        };

        let req_util = if limit.daily_requests == 0 {
            0.0
        } else {
            (usage.requests_today as f64 / limit.daily_requests as f64) * 100.0
        };

        let tok_util = if limit.daily_tokens == 0 {
            0.0
        } else {
            (usage.tokens_today as f64 / limit.daily_tokens as f64) * 100.0
        };

        let monthly_util = if limit.monthly_tokens == 0 {
            0.0
        } else {
            (usage.tokens_this_month as f64 / limit.monthly_tokens as f64) * 100.0
        };

        req_util.max(tok_util).max(monthly_util)
    }

    /// Whether a provider is available (has remaining quota).
    fn is_available_inner(&self, limit: &ProviderLimit, usage: Option<&DailyUsage>) -> bool {
        let usage = match usage {
            Some(u) => u,
            None => return true, // no usage yet → fully available
        };

        // Daily request check (0 = unlimited).
        if limit.daily_requests > 0 && usage.requests_today >= limit.daily_requests {
            return false;
        }

        // Daily token check.
        if limit.daily_tokens > 0 && usage.tokens_today >= limit.daily_tokens {
            return false;
        }

        // Monthly token check.
        if limit.monthly_tokens > 0 && usage.tokens_this_month >= limit.monthly_tokens {
            return false;
        }

        true
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: build defaults and return the budget.
    fn defaults() -> ProviderBudget {
        ProviderBudget::with_defaults()
    }

    // -- 1. Default limits match documented free tiers ----------------------

    #[test]
    fn test_default_gemini_limits() {
        let b = defaults();
        let lim = b.limits.get("gemini").expect("gemini limit missing");
        assert_eq!(lim.daily_requests, 1500);
        assert_eq!(lim.daily_tokens, 0);
        assert_eq!(lim.monthly_tokens, 0);
        assert_eq!(lim.priority, 1);
        assert!(lim.capabilities.contains(&"tool_calling".to_string()));
        assert!(lim.capabilities.contains(&"streaming".to_string()));
        assert!(lim.capabilities.contains(&"vision".to_string()));
    }

    #[test]
    fn test_default_groq_limits() {
        let b = defaults();
        let lim = b.limits.get("groq").expect("groq limit missing");
        assert_eq!(lim.daily_requests, 14400);
        assert_eq!(lim.priority, 2);
        assert!(lim.capabilities.contains(&"tool_calling".to_string()));
        assert!(lim.capabilities.contains(&"streaming".to_string()));
    }

    #[test]
    fn test_default_mistral_limits() {
        let b = defaults();
        let lim = b.limits.get("mistral").expect("mistral limit missing");
        assert_eq!(lim.daily_requests, 33000);
        assert_eq!(lim.monthly_tokens, 1_000_000_000);
        assert_eq!(lim.priority, 3);
        assert!(lim.capabilities.contains(&"tool_calling".to_string()));
    }

    #[test]
    fn test_default_openrouter_limits() {
        let b = defaults();
        let lim = b.limits.get("openrouter").expect("openrouter limit missing");
        assert_eq!(lim.daily_requests, 200);
        assert_eq!(lim.priority, 4);
    }

    #[test]
    fn test_default_ollama_limits() {
        let b = defaults();
        let lim = b.limits.get("ollama").expect("ollama limit missing");
        assert_eq!(lim.daily_requests, 0); // unlimited
        assert_eq!(lim.priority, 10);
    }

    // -- 2. recommend picks highest-priority available provider -------------

    #[test]
    fn test_recommend_picks_highest_priority() {
        let b = defaults();
        let rec = b.recommend(&["tool_calling"]);
        assert_eq!(rec, BudgetRecommendation::UseProvider("gemini".to_string()));
    }

    // -- 3. recommend skips exhausted providers -----------------------------

    #[test]
    fn test_recommend_skips_exhausted_provider() {
        let b = defaults();
        // Exhaust gemini.
        b.record_usage("gemini", 1500, 0);
        let rec = b.recommend(&["tool_calling"]);
        assert_eq!(rec, BudgetRecommendation::UseProvider("groq".to_string()));
    }

    #[test]
    fn test_recommend_skips_multiple_exhausted() {
        let b = defaults();
        b.record_usage("gemini", 1500, 0);
        b.record_usage("groq", 14400, 0);
        let rec = b.recommend(&["tool_calling"]);
        assert_eq!(rec, BudgetRecommendation::UseProvider("mistral".to_string()));
    }

    // -- 4. recommend filters by capabilities --------------------------------

    #[test]
    fn test_recommend_filters_by_vision_capability() {
        let b = defaults();
        // Only gemini has "vision".
        let rec = b.recommend(&["vision"]);
        assert_eq!(rec, BudgetRecommendation::UseProvider("gemini".to_string()));
    }

    #[test]
    fn test_recommend_filters_by_streaming() {
        let b = defaults();
        // Exhaust gemini so groq (which has streaming) should be next.
        b.record_usage("gemini", 1500, 0);
        let rec = b.recommend(&["streaming"]);
        assert_eq!(rec, BudgetRecommendation::UseProvider("groq".to_string()));
    }

    // -- 5. fallback to local when all APIs exhausted -----------------------

    #[test]
    fn test_fallback_to_local_when_all_exhausted() {
        let b = defaults();
        b.record_usage("gemini", 1500, 0);
        b.record_usage("groq", 14400, 0);
        b.record_usage("mistral", 33000, 0);
        b.record_usage("openrouter", 200, 0);
        let rec = b.recommend(&["tool_calling"]);
        assert_eq!(rec, BudgetRecommendation::FallbackLocal("ollama".to_string()));
    }

    // -- 6. daily reset clears counters -------------------------------------

    #[test]
    fn test_daily_reset_clears_counters() {
        let b = defaults();
        b.record_usage("gemini", 100, 5000);
        b.record_error("gemini");
        b.daily_reset();

        let map = b.usage.lock().unwrap();
        let usage = map.get("gemini").unwrap();
        assert_eq!(usage.requests_today, 0);
        assert_eq!(usage.tokens_today, 0);
        assert_eq!(usage.errors_today, 0);
    }

    // -- 7. utilization calculation -----------------------------------------

    #[test]
    fn test_utilization_zero_when_unused() {
        let b = defaults();
        assert_eq!(b.utilization_pct("gemini"), 0.0);
    }

    #[test]
    fn test_utilization_at_50_pct() {
        let b = defaults();
        b.record_usage("gemini", 750, 0);
        let util = b.utilization_pct("gemini");
        assert!((util - 50.0).abs() < 0.01, "expected ~50%, got {}", util);
    }

    #[test]
    fn test_utilization_at_100_pct() {
        let b = defaults();
        b.record_usage("gemini", 1500, 0);
        let util = b.utilization_pct("gemini");
        assert!((util - 100.0).abs() < 0.01, "expected ~100%, got {}", util);
    }

    #[test]
    fn test_utilization_unlimited_provider() {
        let b = defaults();
        b.record_usage("ollama", 99999, 0);
        let util = b.utilization_pct("ollama");
        assert_eq!(util, 0.0, "unlimited provider should always be 0%");
    }

    // -- 8. optimal rotation distributes tasks ------------------------------

    #[test]
    fn test_optimal_rotation_all_fresh() {
        let b = defaults();
        let plan = b.optimal_rotation_plan(100);
        // Should distribute across all remote providers.
        let total: u32 = plan.iter().map(|(_, n)| n).sum();
        assert_eq!(total, 100, "plan must allocate all tasks");
        // Every provider in the plan must be a known provider.
        for (name, _) in &plan {
            assert!(b.limits.contains_key(name), "unknown provider: {}", name);
        }
    }

    #[test]
    fn test_optimal_rotation_with_exhausted() {
        let b = defaults();
        b.record_usage("gemini", 1500, 0);
        let plan = b.optimal_rotation_plan(10);
        // Gemini should not appear.
        assert!(
            !plan.iter().any(|(n, _)| n == "gemini"),
            "exhausted gemini should not be in plan"
        );
        let total: u32 = plan.iter().map(|(_, n)| n).sum();
        assert_eq!(total, 10);
    }

    #[test]
    fn test_optimal_rotation_all_remote_exhausted_falls_to_local() {
        let b = defaults();
        b.record_usage("gemini", 1500, 0);
        b.record_usage("groq", 14400, 0);
        b.record_usage("mistral", 33000, 0);
        b.record_usage("openrouter", 200, 0);
        let plan = b.optimal_rotation_plan(50);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].0, "ollama");
        assert_eq!(plan[0].1, 50);
    }

    // -- 9. record usage increments correctly --------------------------------

    #[test]
    fn test_record_usage_increments() {
        let b = defaults();
        b.record_usage("groq", 5, 1000);
        b.record_usage("groq", 3, 2000);
        let map = b.usage.lock().unwrap();
        let u = map.get("groq").unwrap();
        assert_eq!(u.requests_today, 8);
        assert_eq!(u.tokens_today, 3000);
        assert_eq!(u.tokens_this_month, 3000);
    }

    // -- 10. error tracking --------------------------------------------------

    #[test]
    fn test_record_error_increments() {
        let b = defaults();
        b.record_error("gemini");
        b.record_error("gemini");
        b.record_error("gemini");
        let map = b.usage.lock().unwrap();
        let u = map.get("gemini").unwrap();
        assert_eq!(u.errors_today, 3);
    }

    // -- 11. budget summary includes all providers ---------------------------

    #[test]
    fn test_budget_summary_includes_all_providers() {
        let b = defaults();
        let summary = b.budget_summary();
        assert_eq!(summary.len(), 5);
        let names: Vec<&str> = summary.iter().map(|s| s.provider_name.as_str()).collect();
        assert!(names.contains(&"gemini"));
        assert!(names.contains(&"groq"));
        assert!(names.contains(&"mistral"));
        assert!(names.contains(&"openrouter"));
        assert!(names.contains(&"ollama"));
    }

    #[test]
    fn test_budget_summary_reflects_usage() {
        let b = defaults();
        b.record_usage("gemini", 300, 0);
        let summary = b.budget_summary();
        let gemini = summary.iter().find(|s| s.provider_name == "gemini").unwrap();
        assert_eq!(gemini.requests_used, 300);
        assert_eq!(gemini.requests_limit, 1500);
        assert!(gemini.is_available);
        assert!((gemini.utilization_pct - 20.0).abs() < 0.01);
    }

    // -- 12. monthly token tracking ------------------------------------------

    #[test]
    fn test_monthly_token_tracking() {
        let b = defaults();
        b.record_usage("mistral", 1, 500_000_000);
        b.record_usage("mistral", 1, 500_000_000);
        // 1B tokens used — should be at the limit.
        assert!(!b.is_free("mistral"), "mistral should be exhausted at 1B monthly tokens");
    }

    #[test]
    fn test_monthly_tokens_affect_recommendation() {
        let b = defaults();
        b.record_usage("gemini", 1500, 0);
        b.record_usage("groq", 14400, 0);
        // Exhaust mistral via monthly tokens.
        b.record_usage("mistral", 0, 1_000_000_000);
        let rec = b.recommend(&["tool_calling"]);
        assert_eq!(rec, BudgetRecommendation::UseProvider("openrouter".to_string()));
    }

    // -- 13. auto-reset on date change (simulated) ---------------------------

    #[test]
    fn test_auto_reset_on_date_change() {
        let b = defaults();
        // Manually insert usage with yesterday's date.
        {
            let mut map = b.usage.lock().unwrap();
            let yesterday = Utc::now() - chrono::Duration::days(1);
            let mut u = DailyUsage::new_at(yesterday);
            u.requests_today = 999;
            u.tokens_today = 50000;
            u.errors_today = 5;
            map.insert("gemini".to_string(), u);
        }
        // Recording new usage should trigger auto-reset first.
        b.record_usage("gemini", 1, 100);
        let map = b.usage.lock().unwrap();
        let u = map.get("gemini").unwrap();
        // Should be 1 (the new usage), not 1000.
        assert_eq!(u.requests_today, 1);
        assert_eq!(u.tokens_today, 100);
        assert_eq!(u.errors_today, 0);
    }

    // -- 14. remaining budget -------------------------------------------------

    #[test]
    fn test_remaining_budget_full() {
        let b = defaults();
        let (req, _tok) = b.remaining_budget("gemini");
        assert_eq!(req, 1500);
    }

    #[test]
    fn test_remaining_budget_after_usage() {
        let b = defaults();
        b.record_usage("gemini", 500, 0);
        let (req, _tok) = b.remaining_budget("gemini");
        assert_eq!(req, 1000);
    }

    #[test]
    fn test_remaining_budget_unlimited() {
        let b = defaults();
        let (req, _tok) = b.remaining_budget("ollama");
        assert_eq!(req, u32::MAX);
    }

    #[test]
    fn test_remaining_budget_unknown_provider() {
        let b = defaults();
        let (req, tok) = b.remaining_budget("nonexistent");
        assert_eq!(req, 0);
        assert_eq!(tok, 0);
    }

    // -- 15. is_free ----------------------------------------------------------

    #[test]
    fn test_is_free_when_unused() {
        let b = defaults();
        assert!(b.is_free("gemini"));
    }

    #[test]
    fn test_is_free_when_exhausted() {
        let b = defaults();
        b.record_usage("gemini", 1500, 0);
        assert!(!b.is_free("gemini"));
    }

    #[test]
    fn test_is_free_unknown_provider() {
        let b = defaults();
        assert!(!b.is_free("fantasy_provider"));
    }

    // -- 16. empty budget (no providers registered) --------------------------

    #[test]
    fn test_empty_budget_rate_limited() {
        let b = ProviderBudget::new();
        let rec = b.recommend(&["tool_calling"]);
        assert_eq!(rec, BudgetRecommendation::RateLimited { retry_after_secs: 60 });
    }

    // -- 17. budget summary sorted by priority --------------------------------

    #[test]
    fn test_budget_summary_sorted_by_priority() {
        let b = defaults();
        let summary = b.budget_summary();
        let priorities: Vec<u8> = summary
            .iter()
            .map(|s| b.limits.get(&s.provider_name).unwrap().priority)
            .collect();
        for window in priorities.windows(2) {
            assert!(window[0] <= window[1], "summary should be sorted by priority");
        }
    }

    // -- 18. add_limit custom provider ----------------------------------------

    #[test]
    fn test_add_custom_provider() {
        let mut b = defaults();
        b.add_limit(ProviderLimit {
            provider_name: "cerebras".to_string(),
            daily_requests: 5000,
            daily_tokens: 0,
            monthly_tokens: 0,
            cost_per_1k_tokens: 0.0,
            priority: 5,
            capabilities: vec!["tool_calling".to_string()],
        });
        assert!(b.limits.contains_key("cerebras"));
        let summary = b.budget_summary();
        assert_eq!(summary.len(), 6);
    }
}
