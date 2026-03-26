//! SubscriptionPacer — daily quota management for subscription-tier providers.
//!
//! Spreads a fixed monthly token quota evenly across remaining days so that
//! subscription capacity isn't exhausted early in the billing cycle.
//!
//! # Example
//!
//! A ChatGPT Plus plan with 500K tokens/month and 20 days remaining gives
//! `daily_allowance() = 500_000 / 20 = 25_000` tokens per day.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::debug;

// ---------------------------------------------------------------------------
// SubscriptionPacer
// ---------------------------------------------------------------------------

/// Manages daily token budgets for a subscription-tier provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionPacer {
    /// Provider name (for logging)
    pub provider_name: String,
    /// Total token quota for the billing cycle
    pub total_quota: u64,
    /// Tokens already used in this billing cycle
    pub used_quota: u64,
    /// When the billing cycle resets (UTC)
    pub reset_at: DateTime<Utc>,
    /// Tokens used today
    pub used_today: u64,
}

impl SubscriptionPacer {
    /// Create a new pacer for a subscription provider.
    pub fn new(
        provider_name: &str,
        total_quota: u64,
        reset_at: DateTime<Utc>,
    ) -> Self {
        Self {
            provider_name: provider_name.to_string(),
            total_quota,
            used_quota: 0,
            reset_at,
            used_today: 0,
        }
    }

    /// How many days remain until the billing cycle resets.
    /// Returns at least 1 to avoid division by zero.
    pub fn days_remaining(&self) -> u64 {
        self.days_remaining_at(Utc::now())
    }

    /// (Testable) How many days remain as of `now`.
    fn days_remaining_at(&self, now: DateTime<Utc>) -> u64 {
        let diff = self.reset_at.signed_duration_since(now);
        let days = diff.num_days();
        if days < 1 { 1 } else { days as u64 }
    }

    /// Remaining tokens in this billing cycle.
    pub fn remaining(&self) -> u64 {
        self.total_quota.saturating_sub(self.used_quota)
    }

    /// How many tokens can be used today: `remaining / days_left`.
    pub fn daily_allowance(&self) -> u64 {
        self.daily_allowance_at(Utc::now())
    }

    /// (Testable) Daily allowance as of `now`.
    fn daily_allowance_at(&self, now: DateTime<Utc>) -> u64 {
        let remaining = self.remaining();
        if remaining == 0 {
            return 0;
        }
        let days = self.days_remaining_at(now);
        // Use ceiling division to avoid blocking when remaining < days
        (remaining + days - 1) / days
    }

    /// Whether usage today is still within the daily allowance.
    pub fn can_use_today(&self) -> bool {
        self.can_use_today_at(Utc::now())
    }

    /// (Testable) Check daily allowance as of `now`.
    fn can_use_today_at(&self, now: DateTime<Utc>) -> bool {
        self.used_today < self.daily_allowance_at(now)
    }

    /// Record token usage. Updates both daily and cycle counters.
    pub fn record_usage(&mut self, tokens: u64) {
        self.used_today = self.used_today.saturating_add(tokens);
        self.used_quota = self.used_quota.saturating_add(tokens);
        debug!(
            "SubscriptionPacer [{}]: used {} tokens (today={}, cycle={})",
            self.provider_name, tokens, self.used_today, self.used_quota
        );
    }

    /// Whether the daily allowance has been fully exhausted.
    pub fn is_exhausted(&self) -> bool {
        self.is_exhausted_at(Utc::now())
    }

    /// (Testable) Check exhaustion as of `now`.
    fn is_exhausted_at(&self, now: DateTime<Utc>) -> bool {
        self.used_today >= self.daily_allowance_at(now)
    }

    /// Reset daily counter (call at start of each day).
    pub fn reset_daily(&mut self) {
        self.used_today = 0;
        debug!("SubscriptionPacer [{}]: daily counter reset", self.provider_name);
    }

    /// Reset the entire billing cycle (call when `reset_at` is reached).
    pub fn reset_cycle(&mut self, new_reset_at: DateTime<Utc>) {
        self.used_quota = 0;
        self.used_today = 0;
        self.reset_at = new_reset_at;
        debug!("SubscriptionPacer [{}]: billing cycle reset", self.provider_name);
    }

    /// Utilization percentage (0.0 – 1.0) for the billing cycle.
    pub fn utilization(&self) -> f64 {
        if self.total_quota == 0 {
            return 0.0;
        }
        self.used_quota as f64 / self.total_quota as f64
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn future(days: i64) -> DateTime<Utc> {
        Utc::now() + Duration::days(days)
    }

    #[test]
    fn normal_daily_allowance() {
        let now = Utc::now();
        let pacer = SubscriptionPacer::new("chatgpt-plus", 500_000, now + Duration::days(20));
        // 500_000 / 20 = 25_000 (use deterministic `now` to avoid clock drift)
        let allowance = pacer.daily_allowance_at(now);
        assert_eq!(allowance, 25_000);
    }

    #[test]
    fn last_day_gets_all_remaining() {
        let now = Utc::now();
        let reset = now + Duration::hours(12); // less than 1 day
        let pacer = SubscriptionPacer::new("test", 10_000, reset);
        // days_remaining clamps to 1, so allowance = 10_000
        let allowance = pacer.daily_allowance_at(now);
        assert_eq!(allowance, 10_000);
    }

    #[test]
    fn already_exceeded_blocks_usage() {
        let now = Utc::now();
        let mut pacer = SubscriptionPacer::new("test", 100_000, now + Duration::days(10));
        // daily_allowance = 100_000 / 10 = 10_000
        pacer.record_usage(10_000);
        assert!(pacer.is_exhausted_at(now));
        assert!(!pacer.can_use_today_at(now));
    }

    #[test]
    fn partial_usage_allows_more() {
        let now = Utc::now();
        let mut pacer = SubscriptionPacer::new("test", 100_000, now + Duration::days(10));
        pacer.record_usage(5_000);
        assert!(!pacer.is_exhausted_at(now));
        assert!(pacer.can_use_today_at(now));
    }

    #[test]
    fn quota_fully_used_blocks_everything() {
        let now = Utc::now();
        let mut pacer = SubscriptionPacer::new("test", 1_000, now + Duration::days(10));
        pacer.record_usage(1_000);
        assert_eq!(pacer.remaining(), 0);
        assert_eq!(pacer.daily_allowance_at(now), 0);
        assert!(!pacer.can_use_today_at(now));
    }

    #[test]
    fn fresh_reset_restores_quota() {
        let now = Utc::now();
        let mut pacer = SubscriptionPacer::new("test", 50_000, now + Duration::days(5));
        pacer.record_usage(50_000);
        assert_eq!(pacer.remaining(), 0);

        pacer.reset_cycle(now + Duration::days(30));
        assert_eq!(pacer.remaining(), 50_000);
        assert_eq!(pacer.used_today, 0);
        assert_eq!(pacer.used_quota, 0);
    }

    #[test]
    fn reset_daily_only_clears_today() {
        let mut pacer = SubscriptionPacer::new("test", 100_000, future(10));
        pacer.record_usage(5_000);
        assert_eq!(pacer.used_today, 5_000);
        assert_eq!(pacer.used_quota, 5_000);

        pacer.reset_daily();
        assert_eq!(pacer.used_today, 0);
        assert_eq!(pacer.used_quota, 5_000); // cycle usage preserved
    }

    #[test]
    fn utilization_percentage() {
        let mut pacer = SubscriptionPacer::new("test", 100_000, future(10));
        assert_eq!(pacer.utilization(), 0.0);

        pacer.record_usage(25_000);
        assert!((pacer.utilization() - 0.25).abs() < f64::EPSILON);

        pacer.record_usage(75_000);
        assert!((pacer.utilization() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn zero_total_quota_utilization() {
        let pacer = SubscriptionPacer::new("test", 0, future(10));
        assert_eq!(pacer.utilization(), 0.0);
        assert_eq!(pacer.daily_allowance(), 0);
    }

    #[test]
    fn past_reset_date_clamps_to_one_day() {
        let now = Utc::now();
        let past = now - Duration::days(5);
        let pacer = SubscriptionPacer::new("test", 10_000, past);
        // Reset already passed → days_remaining = 1 → allowance = 10_000
        assert_eq!(pacer.days_remaining_at(now), 1);
        assert_eq!(pacer.daily_allowance_at(now), 10_000);
    }

    #[test]
    fn small_remaining_still_allows_usage() {
        let now = Utc::now();
        let mut pacer = SubscriptionPacer::new("test", 100, now + Duration::days(200));
        // remaining=100, days=200, floor division gives 0, ceiling gives 1
        assert_eq!(pacer.daily_allowance_at(now), 1);
        assert!(pacer.can_use_today_at(now));
    }
}
