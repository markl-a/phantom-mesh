// rate_limiter_v2.rs -- Advanced rate limiter with token bucket + sliding window.
//
// Provides per-tool, per-agent, per-provider, and global rate limiting with
// configurable burst capacity and sliding-window counters. Combines the two
// classic algorithms to achieve both burst tolerance (token bucket) and
// accurate long-term rate control (sliding window).

use serde::Deserialize;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

// ---------------------------------------------------------------------------
// RateLimitV2Config
// ---------------------------------------------------------------------------

/// TOML-level configuration for the v2 rate limiter.
///
/// Example:
/// ```toml
/// [rate_limit]
/// global_rps = 100
/// per_tool_rps = 20
/// per_agent_rps = 50
/// burst_multiplier = 2.0
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitV2Config {
    /// Maximum sustained requests per second globally.
    #[serde(default = "default_global_rps")]
    pub global_rps: f64,

    /// Maximum sustained requests per second per tool.
    #[serde(default = "default_per_tool_rps")]
    pub per_tool_rps: f64,

    /// Maximum sustained requests per second per agent.
    #[serde(default = "default_per_agent_rps")]
    pub per_agent_rps: f64,

    /// Maximum sustained requests per second per provider.
    #[serde(default = "default_per_provider_rps")]
    pub per_provider_rps: f64,

    /// Multiplier for burst capacity over sustained rate.
    /// A value of 2.0 means the bucket can hold 2x the per-second rate.
    #[serde(default = "default_burst_multiplier")]
    pub burst_multiplier: f64,

    /// Sliding window size in seconds (for long-term enforcement).
    #[serde(default = "default_window_secs")]
    pub window_secs: u64,
}

fn default_global_rps() -> f64 { 100.0 }
fn default_per_tool_rps() -> f64 { 20.0 }
fn default_per_agent_rps() -> f64 { 50.0 }
fn default_per_provider_rps() -> f64 { 30.0 }
fn default_burst_multiplier() -> f64 { 2.0 }
fn default_window_secs() -> u64 { 60 }

impl Default for RateLimitV2Config {
    fn default() -> Self {
        Self {
            global_rps: default_global_rps(),
            per_tool_rps: default_per_tool_rps(),
            per_agent_rps: default_per_agent_rps(),
            per_provider_rps: default_per_provider_rps(),
            burst_multiplier: default_burst_multiplier(),
            window_secs: default_window_secs(),
        }
    }
}

// ---------------------------------------------------------------------------
// TokenBucket
// ---------------------------------------------------------------------------

/// A classic token-bucket rate limiter.
///
/// Tokens are added at `refill_rate` tokens/second up to `capacity`.
/// Each request consumes `n` tokens; if insufficient tokens are available
/// the request is denied.
#[derive(Debug, Clone)]
pub struct TokenBucket {
    /// Maximum number of tokens the bucket can hold.
    pub capacity: f64,
    /// Current number of available tokens.
    tokens: f64,
    /// Rate at which tokens are added (tokens per second).
    pub refill_rate: f64,
    /// When we last performed a refill calculation.
    last_refill: Instant,
}

impl TokenBucket {
    /// Create a new token bucket.  Starts full.
    pub fn new(capacity: f64, refill_rate: f64) -> Self {
        Self {
            capacity,
            tokens: capacity,
            refill_rate,
            last_refill: Instant::now(),
        }
    }

    /// Create a token bucket with a specific starting instant (for testing).
    pub fn new_at(capacity: f64, refill_rate: f64, now: Instant) -> Self {
        Self {
            capacity,
            tokens: capacity,
            refill_rate,
            last_refill: now,
        }
    }

    /// Refill tokens based on elapsed time.
    fn refill(&mut self) {
        self.refill_at(Instant::now());
    }

    /// Refill tokens using an explicit timestamp (for deterministic testing).
    fn refill_at(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.last_refill);
        let added = elapsed.as_secs_f64() * self.refill_rate;
        if added > 0.0 {
            self.tokens = (self.tokens + added).min(self.capacity);
            self.last_refill = now;
        }
    }

    /// Try to consume `n` tokens.  Returns `true` if successful.
    pub fn try_acquire(&mut self, n: u32) -> bool {
        self.try_acquire_at(n, Instant::now())
    }

    /// Try to consume `n` tokens at a specific instant.
    pub fn try_acquire_at(&mut self, n: u32, now: Instant) -> bool {
        self.refill_at(now);
        let needed = n as f64;
        if self.tokens >= needed {
            self.tokens -= needed;
            true
        } else {
            false
        }
    }

    /// Number of tokens currently available (after refill).
    pub fn tokens_available(&mut self) -> f64 {
        self.refill();
        self.tokens
    }

    /// Number of tokens available at a specific instant.
    pub fn tokens_available_at(&mut self, now: Instant) -> f64 {
        self.refill_at(now);
        self.tokens
    }

    /// How long until `n` tokens will be available.
    /// Returns `Duration::ZERO` if tokens are already available.
    pub fn time_until_available(&mut self, n: u32) -> Duration {
        self.time_until_available_at(n, Instant::now())
    }

    /// How long until `n` tokens will be available, relative to a specific instant.
    pub fn time_until_available_at(&mut self, n: u32, now: Instant) -> Duration {
        self.refill_at(now);
        let needed = n as f64;
        if self.tokens >= needed {
            Duration::ZERO
        } else {
            let deficit = needed - self.tokens;
            if self.refill_rate <= 0.0 {
                // Infinite wait -- cap at 1 hour.
                Duration::from_secs(3600)
            } else {
                Duration::from_secs_f64(deficit / self.refill_rate)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SlidingWindowCounter
// ---------------------------------------------------------------------------

/// A sliding-window counter that tracks events within a fixed time window.
///
/// Older events are evicted on each operation, giving an accurate count
/// over the trailing `window_size` duration.
#[derive(Debug, Clone)]
pub struct SlidingWindowCounter {
    /// Size of the sliding window.
    pub window_size: Duration,
    /// Maximum number of events allowed within the window.
    pub max_count: usize,
    /// Timestamps of recorded events (front = oldest).
    timestamps: VecDeque<Instant>,
}

impl SlidingWindowCounter {
    /// Create a new sliding-window counter.
    pub fn new(window_size: Duration, max_count: usize) -> Self {
        Self {
            window_size,
            max_count,
            timestamps: VecDeque::new(),
        }
    }

    /// Remove timestamps older than the window boundary.
    fn evict(&mut self, now: Instant) {
        // Use checked_sub to avoid Instant underflow on Windows.
        let cutoff = match now.checked_sub(self.window_size) {
            Some(c) => c,
            None => return, // Window extends before process start; keep everything.
        };
        while let Some(&front) = self.timestamps.front() {
            if front < cutoff {
                self.timestamps.pop_front();
            } else {
                break;
            }
        }
    }

    /// Record an event.  Returns `true` if the event is allowed (under limit).
    /// Returns `false` if recording would exceed `max_count`.
    pub fn record(&mut self) -> bool {
        self.record_at(Instant::now())
    }

    /// Record an event at a specific instant.
    pub fn record_at(&mut self, now: Instant) -> bool {
        self.evict(now);
        if self.timestamps.len() >= self.max_count {
            false
        } else {
            self.timestamps.push_back(now);
            true
        }
    }

    /// Current event count within the window.
    pub fn count(&mut self) -> usize {
        self.count_at(Instant::now())
    }

    /// Current event count at a specific instant.
    pub fn count_at(&mut self, now: Instant) -> usize {
        self.evict(now);
        self.timestamps.len()
    }

    /// Reset the counter, clearing all recorded timestamps.
    pub fn reset(&mut self) {
        self.timestamps.clear();
    }
}

// ---------------------------------------------------------------------------
// RateLimitDecision
// ---------------------------------------------------------------------------

/// The outcome of a rate-limit check.
#[derive(Debug, Clone)]
pub enum RateLimitDecision {
    /// The request is allowed to proceed immediately.
    Allowed,
    /// The request is denied outright.
    Denied {
        /// How long the caller should wait before retrying.
        retry_after: Duration,
        /// Human-readable reason for the denial.
        reason: String,
    },
    /// The request may proceed, but the caller should insert a delay
    /// to stay within sustainable rates.
    Throttled {
        /// Suggested delay before proceeding.
        delay: Duration,
    },
}

impl RateLimitDecision {
    /// Returns true if the decision allows the request (either Allowed or Throttled).
    pub fn is_allowed(&self) -> bool {
        matches!(self, RateLimitDecision::Allowed | RateLimitDecision::Throttled { .. })
    }
}

// ---------------------------------------------------------------------------
// RateLimitStats
// ---------------------------------------------------------------------------

/// Usage statistics for a rate-limiter category.
#[derive(Debug, Clone, Default)]
pub struct RateLimitStats {
    /// Total requests allowed.
    pub requests_allowed: u64,
    /// Total requests denied.
    pub requests_denied: u64,
    /// Current usage as a percentage (0.0 - 100.0).
    pub current_usage_pct: f64,
}

// ---------------------------------------------------------------------------
// LimitEntry (internal, per-key)
// ---------------------------------------------------------------------------

/// Combined bucket + window for a single key (tool name, agent name, etc.).
#[derive(Debug)]
struct LimitEntry {
    bucket: TokenBucket,
    window: SlidingWindowCounter,
    stats: RateLimitStats,
}

impl LimitEntry {
    fn new(rps: f64, burst_multiplier: f64, window_size: Duration) -> Self {
        let capacity = rps * burst_multiplier;
        let max_window_count = (rps * window_size.as_secs_f64()).ceil() as usize;
        Self {
            bucket: TokenBucket::new(capacity, rps),
            window: SlidingWindowCounter::new(window_size, max_window_count),
            stats: RateLimitStats::default(),
        }
    }

    fn new_at(rps: f64, burst_multiplier: f64, window_size: Duration, now: Instant) -> Self {
        let capacity = rps * burst_multiplier;
        let max_window_count = (rps * window_size.as_secs_f64()).ceil() as usize;
        Self {
            bucket: TokenBucket::new_at(capacity, rps, now),
            window: SlidingWindowCounter::new(window_size, max_window_count),
            stats: RateLimitStats::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// RateLimiterV2
// ---------------------------------------------------------------------------

/// Advanced rate limiter combining token-bucket and sliding-window algorithms.
///
/// Maintains independent limiters for:
/// - Each tool (keyed by tool name)
/// - Each agent (keyed by agent name)
/// - Each provider (keyed by provider name)
/// - A single global limiter
///
/// A request must pass **all four** checks to be allowed.
pub struct RateLimiterV2 {
    config: RateLimitV2Config,
    tool_limits: Mutex<HashMap<String, LimitEntry>>,
    agent_limits: Mutex<HashMap<String, LimitEntry>>,
    provider_limits: Mutex<HashMap<String, LimitEntry>>,
    global: Mutex<LimitEntry>,
}

impl std::fmt::Debug for RateLimiterV2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimiterV2")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl RateLimiterV2 {
    /// Create a new rate limiter from the given config.
    pub fn new(config: RateLimitV2Config) -> Self {
        let window = Duration::from_secs(config.window_secs);
        let global = LimitEntry::new(
            config.global_rps,
            config.burst_multiplier,
            window,
        );
        Self {
            config,
            tool_limits: Mutex::new(HashMap::new()),
            agent_limits: Mutex::new(HashMap::new()),
            provider_limits: Mutex::new(HashMap::new()),
            global: Mutex::new(global),
        }
    }

    /// Create with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(RateLimitV2Config::default())
    }

    /// Check whether a request from `agent` using `tool` via `provider` should
    /// be allowed, denied, or throttled.
    ///
    /// If allowed, tokens are consumed and the sliding window is updated.
    pub fn check(&self, agent: &str, tool: &str, provider: &str) -> RateLimitDecision {
        self.check_at(agent, tool, provider, Instant::now())
    }

    /// Check with an explicit timestamp (for deterministic testing).
    pub fn check_at(
        &self,
        agent: &str,
        tool: &str,
        provider: &str,
        now: Instant,
    ) -> RateLimitDecision {
        let window = Duration::from_secs(self.config.window_secs);

        // 1. Global check
        {
            let mut global = self.global.lock().unwrap();
            if !global.bucket.try_acquire_at(1, now) {
                let retry = global.bucket.time_until_available_at(1, now);
                global.stats.requests_denied += 1;
                warn!(
                    agent = agent,
                    tool = tool,
                    provider = provider,
                    "rate_limiter_v2: denied by global limit"
                );
                return RateLimitDecision::Denied {
                    retry_after: retry,
                    reason: "global rate limit exceeded".into(),
                };
            }
            if !global.window.record_at(now) {
                // Bucket allowed but window says no -- refund the token.
                global.bucket.tokens += 1.0;
                global.stats.requests_denied += 1;
                return RateLimitDecision::Denied {
                    retry_after: Duration::from_secs(1),
                    reason: "global sliding window limit exceeded".into(),
                };
            }
            global.stats.requests_allowed += 1;
            let cap = global.bucket.capacity;
            let avail = global.bucket.tokens;
            global.stats.current_usage_pct = ((cap - avail) / cap * 100.0).clamp(0.0, 100.0);
        }

        // 2. Per-tool check
        {
            let mut tools = self.tool_limits.lock().unwrap();
            let entry = tools
                .entry(tool.to_string())
                .or_insert_with(|| {
                    LimitEntry::new_at(
                        self.config.per_tool_rps,
                        self.config.burst_multiplier,
                        window,
                        now,
                    )
                });
            if !entry.bucket.try_acquire_at(1, now) {
                let retry = entry.bucket.time_until_available_at(1, now);
                entry.stats.requests_denied += 1;
                warn!(tool = tool, "rate_limiter_v2: denied by per-tool limit");
                // Undo global counters.
                self.undo_global(now);
                return RateLimitDecision::Denied {
                    retry_after: retry,
                    reason: format!("per-tool rate limit exceeded for '{}'", tool),
                };
            }
            if !entry.window.record_at(now) {
                entry.bucket.tokens += 1.0;
                entry.stats.requests_denied += 1;
                self.undo_global(now);
                return RateLimitDecision::Denied {
                    retry_after: Duration::from_secs(1),
                    reason: format!("per-tool sliding window limit exceeded for '{}'", tool),
                };
            }
            entry.stats.requests_allowed += 1;
            let cap = entry.bucket.capacity;
            let avail = entry.bucket.tokens;
            entry.stats.current_usage_pct = ((cap - avail) / cap * 100.0).clamp(0.0, 100.0);
        }

        // 3. Per-agent check
        {
            let mut agents = self.agent_limits.lock().unwrap();
            let entry = agents
                .entry(agent.to_string())
                .or_insert_with(|| {
                    LimitEntry::new_at(
                        self.config.per_agent_rps,
                        self.config.burst_multiplier,
                        window,
                        now,
                    )
                });
            if !entry.bucket.try_acquire_at(1, now) {
                let retry = entry.bucket.time_until_available_at(1, now);
                entry.stats.requests_denied += 1;
                warn!(agent = agent, "rate_limiter_v2: denied by per-agent limit");
                self.undo_global(now);
                self.undo_tool(tool, now);
                return RateLimitDecision::Denied {
                    retry_after: retry,
                    reason: format!("per-agent rate limit exceeded for '{}'", agent),
                };
            }
            if !entry.window.record_at(now) {
                entry.bucket.tokens += 1.0;
                entry.stats.requests_denied += 1;
                self.undo_global(now);
                self.undo_tool(tool, now);
                return RateLimitDecision::Denied {
                    retry_after: Duration::from_secs(1),
                    reason: format!("per-agent sliding window limit exceeded for '{}'", agent),
                };
            }
            entry.stats.requests_allowed += 1;
            let cap = entry.bucket.capacity;
            let avail = entry.bucket.tokens;
            entry.stats.current_usage_pct = ((cap - avail) / cap * 100.0).clamp(0.0, 100.0);
        }

        // 4. Per-provider check
        {
            let mut providers = self.provider_limits.lock().unwrap();
            let entry = providers
                .entry(provider.to_string())
                .or_insert_with(|| {
                    LimitEntry::new_at(
                        self.config.per_provider_rps,
                        self.config.burst_multiplier,
                        window,
                        now,
                    )
                });
            if !entry.bucket.try_acquire_at(1, now) {
                let retry = entry.bucket.time_until_available_at(1, now);
                entry.stats.requests_denied += 1;
                warn!(provider = provider, "rate_limiter_v2: denied by per-provider limit");
                self.undo_global(now);
                self.undo_tool(tool, now);
                self.undo_agent(agent, now);
                return RateLimitDecision::Denied {
                    retry_after: retry,
                    reason: format!("per-provider rate limit exceeded for '{}'", provider),
                };
            }
            if !entry.window.record_at(now) {
                entry.bucket.tokens += 1.0;
                entry.stats.requests_denied += 1;
                self.undo_global(now);
                self.undo_tool(tool, now);
                self.undo_agent(agent, now);
                return RateLimitDecision::Denied {
                    retry_after: Duration::from_secs(1),
                    reason: format!("per-provider sliding window limit exceeded for '{}'", provider),
                };
            }
            entry.stats.requests_allowed += 1;
            let cap = entry.bucket.capacity;
            let avail = entry.bucket.tokens;
            entry.stats.current_usage_pct = ((cap - avail) / cap * 100.0).clamp(0.0, 100.0);
        }

        // 5. Determine if we should suggest throttling.
        //    If global usage > 80%, suggest a small delay.
        let global_usage = {
            let global = self.global.lock().unwrap();
            global.stats.current_usage_pct
        };

        if global_usage > 80.0 {
            let delay_ms = ((global_usage - 80.0) / 20.0 * 100.0) as u64; // 0-100ms
            debug!(
                agent = agent,
                tool = tool,
                usage_pct = global_usage,
                "rate_limiter_v2: throttling suggested"
            );
            RateLimitDecision::Throttled {
                delay: Duration::from_millis(delay_ms.max(5)),
            }
        } else {
            RateLimitDecision::Allowed
        }
    }

    // -- Undo helpers (best-effort, for consistency on partial check failure) --

    fn undo_global(&self, _now: Instant) {
        let mut global = self.global.lock().unwrap();
        global.bucket.tokens = (global.bucket.tokens + 1.0).min(global.bucket.capacity);
        if global.stats.requests_allowed > 0 {
            global.stats.requests_allowed -= 1;
        }
        global.stats.requests_denied += 1;
        global.window.timestamps.pop_back();
    }

    fn undo_tool(&self, tool: &str, _now: Instant) {
        let mut tools = self.tool_limits.lock().unwrap();
        if let Some(entry) = tools.get_mut(tool) {
            entry.bucket.tokens = (entry.bucket.tokens + 1.0).min(entry.bucket.capacity);
            if entry.stats.requests_allowed > 0 {
                entry.stats.requests_allowed -= 1;
            }
            entry.window.timestamps.pop_back();
        }
    }

    fn undo_agent(&self, agent: &str, _now: Instant) {
        let mut agents = self.agent_limits.lock().unwrap();
        if let Some(entry) = agents.get_mut(agent) {
            entry.bucket.tokens = (entry.bucket.tokens + 1.0).min(entry.bucket.capacity);
            if entry.stats.requests_allowed > 0 {
                entry.stats.requests_allowed -= 1;
            }
            entry.window.timestamps.pop_back();
        }
    }

    // -- Stats --

    /// Retrieve stats for a specific tool.
    pub fn tool_stats(&self, tool: &str) -> Option<RateLimitStats> {
        let tools = self.tool_limits.lock().unwrap();
        tools.get(tool).map(|e| e.stats.clone())
    }

    /// Retrieve stats for a specific agent.
    pub fn agent_stats(&self, agent: &str) -> Option<RateLimitStats> {
        let agents = self.agent_limits.lock().unwrap();
        agents.get(agent).map(|e| e.stats.clone())
    }

    /// Retrieve stats for a specific provider.
    pub fn provider_stats(&self, provider: &str) -> Option<RateLimitStats> {
        let providers = self.provider_limits.lock().unwrap();
        providers.get(provider).map(|e| e.stats.clone())
    }

    /// Retrieve global stats.
    pub fn global_stats(&self) -> RateLimitStats {
        let global = self.global.lock().unwrap();
        global.stats.clone()
    }

    /// Reset all limiters (useful for config hot-reload).
    pub fn reset(&self) {
        let window = Duration::from_secs(self.config.window_secs);
        {
            let mut global = self.global.lock().unwrap();
            *global = LimitEntry::new(self.config.global_rps, self.config.burst_multiplier, window);
        }
        self.tool_limits.lock().unwrap().clear();
        self.agent_limits.lock().unwrap().clear();
        self.provider_limits.lock().unwrap().clear();
    }

    /// Return the underlying config.
    pub fn config(&self) -> &RateLimitV2Config {
        &self.config
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    // -- TokenBucket tests --------------------------------------------------

    #[test]
    fn test_token_bucket_new_starts_full() {
        let mut bucket = TokenBucket::new(10.0, 1.0);
        assert!((bucket.tokens_available() - 10.0).abs() < 0.1);
    }

    #[test]
    fn test_token_bucket_acquire_single() {
        let now = Instant::now();
        let mut bucket = TokenBucket::new_at(5.0, 1.0, now);
        assert!(bucket.try_acquire_at(1, now));
        assert!((bucket.tokens_available_at(now) - 4.0).abs() < 0.01);
    }

    #[test]
    fn test_token_bucket_acquire_multiple() {
        let now = Instant::now();
        let mut bucket = TokenBucket::new_at(5.0, 1.0, now);
        assert!(bucket.try_acquire_at(3, now));
        assert!((bucket.tokens_available_at(now) - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_token_bucket_acquire_exceeds_capacity() {
        let now = Instant::now();
        let mut bucket = TokenBucket::new_at(5.0, 1.0, now);
        assert!(!bucket.try_acquire_at(6, now));
        // Tokens should remain unchanged on failure.
        assert!((bucket.tokens_available_at(now) - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_token_bucket_refill_over_time() {
        let now = Instant::now();
        let mut bucket = TokenBucket::new_at(10.0, 2.0, now);
        // Drain to 0.
        assert!(bucket.try_acquire_at(10, now));
        assert!((bucket.tokens_available_at(now) - 0.0).abs() < 0.01);
        // After 3 seconds at 2 tokens/sec, should have 6 tokens.
        let later = now + Duration::from_secs(3);
        assert!((bucket.tokens_available_at(later) - 6.0).abs() < 0.1);
    }

    #[test]
    fn test_token_bucket_refill_caps_at_capacity() {
        let now = Instant::now();
        let mut bucket = TokenBucket::new_at(10.0, 5.0, now);
        // Drain 2 tokens.
        assert!(bucket.try_acquire_at(2, now));
        // After 10 seconds, should be back to 10 (not 10 + leftover).
        let later = now + Duration::from_secs(10);
        assert!((bucket.tokens_available_at(later) - 10.0).abs() < 0.01);
    }

    #[test]
    fn test_token_bucket_time_until_available() {
        let now = Instant::now();
        let mut bucket = TokenBucket::new_at(10.0, 2.0, now);
        // Drain all tokens.
        assert!(bucket.try_acquire_at(10, now));
        // Need 4 tokens at 2/sec = 2 seconds.
        let wait = bucket.time_until_available_at(4, now);
        assert!((wait.as_secs_f64() - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_token_bucket_time_until_available_already_available() {
        let now = Instant::now();
        let mut bucket = TokenBucket::new_at(10.0, 2.0, now);
        let wait = bucket.time_until_available_at(5, now);
        assert_eq!(wait, Duration::ZERO);
    }

    #[test]
    fn test_token_bucket_zero_refill_rate() {
        let now = Instant::now();
        let mut bucket = TokenBucket::new_at(5.0, 0.0, now);
        assert!(bucket.try_acquire_at(5, now));
        // With zero refill, time_until_available should return large duration.
        let wait = bucket.time_until_available_at(1, now);
        assert!(wait >= Duration::from_secs(3600));
    }

    // -- SlidingWindowCounter tests -----------------------------------------

    #[test]
    fn test_sliding_window_basic() {
        let now = Instant::now();
        let mut sw = SlidingWindowCounter::new(Duration::from_secs(10), 5);
        assert!(sw.record_at(now));
        assert_eq!(sw.count_at(now), 1);
    }

    #[test]
    fn test_sliding_window_max_count() {
        let now = Instant::now();
        let mut sw = SlidingWindowCounter::new(Duration::from_secs(10), 3);
        assert!(sw.record_at(now));
        assert!(sw.record_at(now));
        assert!(sw.record_at(now));
        // 4th should be denied.
        assert!(!sw.record_at(now));
        assert_eq!(sw.count_at(now), 3);
    }

    #[test]
    fn test_sliding_window_eviction() {
        let now = Instant::now();
        let mut sw = SlidingWindowCounter::new(Duration::from_secs(5), 3);
        assert!(sw.record_at(now));
        assert!(sw.record_at(now + Duration::from_secs(1)));
        assert!(sw.record_at(now + Duration::from_secs(2)));
        // Full. But after 6 seconds the first entry is evicted.
        let later = now + Duration::from_secs(6);
        assert_eq!(sw.count_at(later), 2); // second two are still in window
        assert!(sw.record_at(later)); // should be allowed now
    }

    #[test]
    fn test_sliding_window_reset() {
        let now = Instant::now();
        let mut sw = SlidingWindowCounter::new(Duration::from_secs(10), 3);
        sw.record_at(now);
        sw.record_at(now);
        assert_eq!(sw.count_at(now), 2);
        sw.reset();
        assert_eq!(sw.count_at(now), 0);
    }

    #[test]
    fn test_sliding_window_all_evicted() {
        let now = Instant::now();
        let mut sw = SlidingWindowCounter::new(Duration::from_secs(2), 10);
        for _ in 0..10 {
            sw.record_at(now);
        }
        assert_eq!(sw.count_at(now), 10);
        // All should be evicted after the window passes.
        let later = now + Duration::from_secs(3);
        assert_eq!(sw.count_at(later), 0);
    }

    // -- RateLimitDecision tests --------------------------------------------

    #[test]
    fn test_decision_is_allowed() {
        assert!(RateLimitDecision::Allowed.is_allowed());
        assert!(RateLimitDecision::Throttled { delay: Duration::from_millis(10) }.is_allowed());
        assert!(!RateLimitDecision::Denied {
            retry_after: Duration::from_secs(1),
            reason: "test".into(),
        }.is_allowed());
    }

    // -- RateLimitV2Config tests --------------------------------------------

    #[test]
    fn test_config_defaults() {
        let config = RateLimitV2Config::default();
        assert!((config.global_rps - 100.0).abs() < f64::EPSILON);
        assert!((config.per_tool_rps - 20.0).abs() < f64::EPSILON);
        assert!((config.per_agent_rps - 50.0).abs() < f64::EPSILON);
        assert!((config.burst_multiplier - 2.0).abs() < f64::EPSILON);
        assert_eq!(config.window_secs, 60);
    }

    #[test]
    fn test_config_from_toml() {
        let toml_str = r#"
            global_rps = 200
            per_tool_rps = 40
            per_agent_rps = 80
            per_provider_rps = 60
            burst_multiplier = 3.0
            window_secs = 120
        "#;
        let config: RateLimitV2Config = toml::from_str(toml_str).unwrap();
        assert!((config.global_rps - 200.0).abs() < f64::EPSILON);
        assert!((config.per_tool_rps - 40.0).abs() < f64::EPSILON);
        assert!((config.per_agent_rps - 80.0).abs() < f64::EPSILON);
        assert!((config.per_provider_rps - 60.0).abs() < f64::EPSILON);
        assert!((config.burst_multiplier - 3.0).abs() < f64::EPSILON);
        assert_eq!(config.window_secs, 120);
    }

    #[test]
    fn test_config_partial_toml_uses_defaults() {
        let toml_str = r#"
            global_rps = 500
        "#;
        let config: RateLimitV2Config = toml::from_str(toml_str).unwrap();
        assert!((config.global_rps - 500.0).abs() < f64::EPSILON);
        assert!((config.per_tool_rps - 20.0).abs() < f64::EPSILON); // default
        assert!((config.burst_multiplier - 2.0).abs() < f64::EPSILON); // default
    }

    // -- RateLimiterV2 integrated tests -------------------------------------

    #[test]
    fn test_limiter_allows_normal_request() {
        let limiter = RateLimiterV2::with_defaults();
        let decision = limiter.check("agent1", "shell", "ollama");
        assert!(decision.is_allowed());
    }

    #[test]
    fn test_limiter_per_tool_limit_exhaustion() {
        let config = RateLimitV2Config {
            global_rps: 1000.0,
            per_tool_rps: 2.0,
            per_agent_rps: 1000.0,
            per_provider_rps: 1000.0,
            burst_multiplier: 1.0, // No burst headroom.
            window_secs: 60,
        };
        let limiter = RateLimiterV2::new(config);
        let now = Instant::now();
        // First 2 should pass (bucket capacity = 2.0 * 1.0 = 2).
        assert!(limiter.check_at("a", "my_tool", "p", now).is_allowed());
        assert!(limiter.check_at("a", "my_tool", "p", now).is_allowed());
        // Third should be denied.
        let decision = limiter.check_at("a", "my_tool", "p", now);
        assert!(!decision.is_allowed());
        if let RateLimitDecision::Denied { reason, .. } = &decision {
            assert!(reason.contains("my_tool"));
        } else {
            panic!("expected Denied");
        }
    }

    #[test]
    fn test_limiter_per_agent_limit_exhaustion() {
        let config = RateLimitV2Config {
            global_rps: 1000.0,
            per_tool_rps: 1000.0,
            per_agent_rps: 3.0,
            per_provider_rps: 1000.0,
            burst_multiplier: 1.0,
            window_secs: 60,
        };
        let limiter = RateLimiterV2::new(config);
        let now = Instant::now();
        // Use different tools so per-tool is not the bottleneck.
        assert!(limiter.check_at("bot", "t1", "p", now).is_allowed());
        assert!(limiter.check_at("bot", "t2", "p", now).is_allowed());
        assert!(limiter.check_at("bot", "t3", "p", now).is_allowed());
        let decision = limiter.check_at("bot", "t4", "p", now);
        assert!(!decision.is_allowed());
        if let RateLimitDecision::Denied { reason, .. } = &decision {
            assert!(reason.contains("bot"));
        } else {
            panic!("expected Denied for agent");
        }
    }

    #[test]
    fn test_limiter_per_provider_limit_exhaustion() {
        let config = RateLimitV2Config {
            global_rps: 1000.0,
            per_tool_rps: 1000.0,
            per_agent_rps: 1000.0,
            per_provider_rps: 2.0,
            burst_multiplier: 1.0,
            window_secs: 60,
        };
        let limiter = RateLimiterV2::new(config);
        let now = Instant::now();
        assert!(limiter.check_at("a1", "t1", "gemini", now).is_allowed());
        assert!(limiter.check_at("a2", "t2", "gemini", now).is_allowed());
        let decision = limiter.check_at("a3", "t3", "gemini", now);
        assert!(!decision.is_allowed());
        if let RateLimitDecision::Denied { reason, .. } = &decision {
            assert!(reason.contains("gemini"));
        } else {
            panic!("expected Denied for provider");
        }
    }

    #[test]
    fn test_limiter_global_limit_exhaustion() {
        let config = RateLimitV2Config {
            global_rps: 2.0,
            per_tool_rps: 1000.0,
            per_agent_rps: 1000.0,
            per_provider_rps: 1000.0,
            burst_multiplier: 1.0,
            window_secs: 60,
        };
        let limiter = RateLimiterV2::new(config);
        let now = Instant::now();
        assert!(limiter.check_at("a", "t1", "p", now).is_allowed());
        assert!(limiter.check_at("a", "t2", "p", now).is_allowed());
        let decision = limiter.check_at("a", "t3", "p", now);
        assert!(!decision.is_allowed());
        if let RateLimitDecision::Denied { reason, .. } = &decision {
            assert!(reason.contains("global"));
        } else {
            panic!("expected Denied for global");
        }
    }

    #[test]
    fn test_limiter_burst_allows_more_than_base() {
        let config = RateLimitV2Config {
            global_rps: 1000.0,
            per_tool_rps: 5.0,
            per_agent_rps: 1000.0,
            per_provider_rps: 1000.0,
            burst_multiplier: 3.0, // capacity = 15
            window_secs: 60,
        };
        let limiter = RateLimiterV2::new(config);
        let now = Instant::now();
        // With burst_multiplier=3, capacity = 5*3 = 15 tokens.
        for i in 0..15 {
            let d = limiter.check_at("a", "tool", "p", now);
            assert!(d.is_allowed(), "request {} should be allowed", i);
        }
        // 16th should be denied.
        let decision = limiter.check_at("a", "tool", "p", now);
        assert!(!decision.is_allowed());
    }

    #[test]
    fn test_limiter_recovery_after_wait() {
        let config = RateLimitV2Config {
            global_rps: 1000.0,
            per_tool_rps: 2.0,
            per_agent_rps: 1000.0,
            per_provider_rps: 1000.0,
            burst_multiplier: 1.0,
            window_secs: 60,
        };
        let limiter = RateLimiterV2::new(config);
        let now = Instant::now();
        // Exhaust.
        assert!(limiter.check_at("a", "t", "p", now).is_allowed());
        assert!(limiter.check_at("a", "t", "p", now).is_allowed());
        assert!(!limiter.check_at("a", "t", "p", now).is_allowed());
        // After 1 second, 2 more tokens should be refilled (rate=2/s).
        let later = now + Duration::from_secs(1);
        assert!(limiter.check_at("a", "t", "p", later).is_allowed());
    }

    #[test]
    fn test_limiter_different_tools_independent() {
        let config = RateLimitV2Config {
            global_rps: 1000.0,
            per_tool_rps: 2.0,
            per_agent_rps: 1000.0,
            per_provider_rps: 1000.0,
            burst_multiplier: 1.0,
            window_secs: 60,
        };
        let limiter = RateLimiterV2::new(config);
        let now = Instant::now();
        // Exhaust tool_a.
        assert!(limiter.check_at("a", "tool_a", "p", now).is_allowed());
        assert!(limiter.check_at("a", "tool_a", "p", now).is_allowed());
        assert!(!limiter.check_at("a", "tool_a", "p", now).is_allowed());
        // tool_b should still be available.
        assert!(limiter.check_at("a", "tool_b", "p", now).is_allowed());
    }

    #[test]
    fn test_limiter_different_agents_independent() {
        let config = RateLimitV2Config {
            global_rps: 1000.0,
            per_tool_rps: 1000.0,
            per_agent_rps: 1.0,
            per_provider_rps: 1000.0,
            burst_multiplier: 1.0,
            window_secs: 60,
        };
        let limiter = RateLimiterV2::new(config);
        let now = Instant::now();
        assert!(limiter.check_at("alice", "t", "p", now).is_allowed());
        assert!(!limiter.check_at("alice", "t", "p", now).is_allowed());
        // bob should be independent.
        assert!(limiter.check_at("bob", "t", "p", now).is_allowed());
    }

    #[test]
    fn test_limiter_stats_tracking() {
        let config = RateLimitV2Config {
            global_rps: 1000.0,
            per_tool_rps: 2.0,
            per_agent_rps: 1000.0,
            per_provider_rps: 1000.0,
            burst_multiplier: 1.0,
            window_secs: 60,
        };
        let limiter = RateLimiterV2::new(config);
        let now = Instant::now();
        limiter.check_at("a", "shell", "p", now);
        limiter.check_at("a", "shell", "p", now);
        limiter.check_at("a", "shell", "p", now); // denied

        let stats = limiter.tool_stats("shell").unwrap();
        assert_eq!(stats.requests_allowed, 2);
        assert_eq!(stats.requests_denied, 1);

        let global = limiter.global_stats();
        assert_eq!(global.requests_allowed, 2);
    }

    #[test]
    fn test_limiter_reset() {
        let config = RateLimitV2Config {
            global_rps: 1000.0,
            per_tool_rps: 1.0,
            per_agent_rps: 1000.0,
            per_provider_rps: 1000.0,
            burst_multiplier: 1.0,
            window_secs: 60,
        };
        let limiter = RateLimiterV2::new(config);
        let now = Instant::now();
        assert!(limiter.check_at("a", "t", "p", now).is_allowed());
        assert!(!limiter.check_at("a", "t", "p", now).is_allowed());
        limiter.reset();
        // After reset, should be allowed again.
        assert!(limiter.check_at("a", "t", "p", now).is_allowed());
    }

    #[test]
    fn test_limiter_throttle_at_high_usage() {
        // Set global capacity very low so usage hits 80%+ after one request.
        let config = RateLimitV2Config {
            global_rps: 5.0,
            per_tool_rps: 1000.0,
            per_agent_rps: 1000.0,
            per_provider_rps: 1000.0,
            burst_multiplier: 1.0, // capacity = 5
            window_secs: 60,
        };
        let limiter = RateLimiterV2::new(config);
        let now = Instant::now();
        // Use 5 tokens: 4 allowed (80%) + 1 more to trigger throttle.
        // Actually, 4 out of 5 = 80%, so the 5th call should see >80% usage.
        for _ in 0..4 {
            limiter.check_at("a", "t", "p", now);
        }
        // After consuming 4/5 = 80% usage.  The 5th request, if allowed, will
        // see usage > 80%.
        let decision = limiter.check_at("a", "t5", "p5", now);
        match decision {
            RateLimitDecision::Throttled { delay } => {
                assert!(delay.as_millis() >= 5);
            }
            RateLimitDecision::Allowed => {
                // Marginal case at exactly 80% is also acceptable.
            }
            RateLimitDecision::Denied { .. } => {
                panic!("should not be denied at 5/5 bucket with different keys");
            }
        }
    }

    #[test]
    fn test_limiter_sliding_window_enforcement() {
        // Use a very short window with small max_count to trigger window denial.
        let config = RateLimitV2Config {
            global_rps: 1000.0,
            per_tool_rps: 1000.0,
            per_agent_rps: 1000.0,
            per_provider_rps: 1000.0,
            burst_multiplier: 100.0, // huge burst so bucket is never the bottleneck
            window_secs: 1,          // 1-second window
        };
        let limiter = RateLimiterV2::new(config);
        let now = Instant::now();
        // Global window: max_count = ceil(1000 * 1) = 1000.
        // Per-tool window: ceil(1000 * 1) = 1000.
        // With burst_multiplier=100, bucket capacity = 100000.
        // We need to exhaust the sliding window. Let us fire 1000 requests.
        let mut allowed = 0;
        for _ in 0..1001 {
            if limiter.check_at("a", "t", "p", now).is_allowed() {
                allowed += 1;
            }
        }
        // Should allow exactly 1000 (window limit) and deny the 1001st.
        assert_eq!(allowed, 1000);
    }

    #[test]
    fn test_limiter_retry_after_nonzero() {
        let config = RateLimitV2Config {
            global_rps: 1.0,
            per_tool_rps: 1000.0,
            per_agent_rps: 1000.0,
            per_provider_rps: 1000.0,
            burst_multiplier: 1.0,
            window_secs: 60,
        };
        let limiter = RateLimiterV2::new(config);
        let now = Instant::now();
        assert!(limiter.check_at("a", "t", "p", now).is_allowed());
        match limiter.check_at("a", "t", "p", now) {
            RateLimitDecision::Denied { retry_after, .. } => {
                // Should be approximately 1 second (1 token at 1/s).
                assert!(retry_after.as_secs_f64() > 0.5);
                assert!(retry_after.as_secs_f64() < 1.5);
            }
            other => panic!("expected Denied, got {:?}", other),
        }
    }

    #[test]
    fn test_limiter_stats_none_for_unknown_key() {
        let limiter = RateLimiterV2::with_defaults();
        assert!(limiter.tool_stats("nonexistent").is_none());
        assert!(limiter.agent_stats("nobody").is_none());
        assert!(limiter.provider_stats("unknown").is_none());
    }

    #[test]
    fn test_limiter_config_accessor() {
        let config = RateLimitV2Config {
            global_rps: 42.0,
            ..Default::default()
        };
        let limiter = RateLimiterV2::new(config);
        assert!((limiter.config().global_rps - 42.0).abs() < f64::EPSILON);
    }
}
