//! B9 / T90 — per-channel token-bucket rate limiter for the
//! remote-control surface (BIG-GOAL §P3).
//!
//! Each remote (Telegram / Slack / WhatsApp) has its own upstream quota;
//! a chatty operator on one channel must not be able to spend every other
//! channel's budget. This bucket sits in front of every
//! [`Channel::send_message`] so an over-eager remote gets a polite
//! `RateLimited` *before* we burn a TCP round-trip on a guaranteed-429.
//!
//! Channels (`Telegram`, `Slack`, `WhatsApp`) all live behind upstream API
//! quotas; the upstream's own 429 responses are *slow* (they cost a TCP
//! round-trip and count against connection budgets), so we keep a local
//! token bucket per channel and reject ahead of the network call. Every
//! [`Channel::send_message`] impl is expected to call
//! [`PerChannelLimiter::check`] before performing any HTTP I/O. A throttled
//! send returns [`ChannelError::RateLimited`] with a `retry_after_sec` hint
//! computed from the bucket's refill rate.
//!
//! # Design
//!
//! - [`TokenBucket`] is a classic refill-on-read bucket: `try_acquire` first
//!   credits any tokens that have accumulated since `last`, caps them at
//!   `capacity`, then debits one token if any remain.
//! - [`PerChannelLimiter`] holds `DashMap<String, ArcSwap<Mutex<TokenBucket>>>`
//!   so reads of the hot path (existing channels) never block other
//!   channels, and the `ArcSwap` layer leaves room for a future "reconfigure
//!   refill rate at runtime" path without touching the mutex protocol.
//! - All public types are `Send + Sync` so a single `Arc<PerChannelLimiter>`
//!   can be cloned into every channel adapter at startup.
//!
//! # Defaults
//!
//! `PerChannelLimiter::new(default_rate)` is used when a channel asks for
//! itself before being explicitly configured. Per-channel rates are taken
//! from upstream documentation:
//!
//! - **Telegram**: 30 msg/sec per bot ([Bots FAQ](https://core.telegram.org/bots/faq#broadcasting-to-users))
//! - **Slack**: 1 msg/sec Tier 1 ([rate limits](https://api.slack.com/apis/rate-limits))
//! - **WhatsApp**: 80 msg/sec Cloud API default ([throughput](https://developers.facebook.com/docs/whatsapp/cloud-api/overview))
//!
//! See [`defaults`] for the public constants.
//!
//! # Feature gating
//!
//! Compiled only with `--features experimental-remote-control`. Default
//! `cargo build` produces a byte-identical baseline binary.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use dashmap::DashMap;

use super::channel_trait::ChannelError;

/// Canonical per-channel defaults sourced from upstream documentation.
/// Exposed so call sites stay self-documenting rather than sprinkling
/// magic numbers across each `Channel` impl.
pub mod defaults {
    /// Telegram Bot API: 30 messages/sec per bot.
    pub const TELEGRAM_RATE: u32 = 30;
    /// Slack Web API Tier 1: ~1 request/sec.
    pub const SLACK_RATE: u32 = 1;
    /// WhatsApp Cloud API default: 80 messages/sec.
    pub const WHATSAPP_RATE: u32 = 80;
}

/// Local error surface for the limiter. Promoted to
/// [`ChannelError::RateLimited`] at the call site so channel impls can
/// `?`-propagate without reshaping the error.
#[derive(Debug, thiserror::Error)]
pub enum RateLimitError {
    /// No tokens left; caller should wait `retry_after_sec` before retrying.
    #[error("rate-limited on {channel}; retry in {retry_after_sec:.3}s")]
    Throttled {
        /// Static-string channel name — matches the `&'static str` shape used
        /// throughout `ChannelError` so promotion is allocation-free.
        channel: &'static str,
        /// Wall-clock seconds until a single token is available again.
        retry_after_sec: f64,
    },
}

impl RateLimitError {
    /// Convert into the channel-level error variant. Kept as a method so
    /// call sites stay one line: `self.limiter.check(name).map_err(|e| e.into_channel())?;`.
    pub fn into_channel(self) -> ChannelError {
        match self {
            RateLimitError::Throttled {
                channel,
                retry_after_sec,
            } => ChannelError::RateLimited {
                channel,
                retry_after_sec,
            },
        }
    }
}

/// A simple refill-on-read token bucket.
///
/// `capacity` tokens are added to the bucket at a rate of `refill_per_sec`,
/// capped at `capacity`. `try_acquire` first credits any accumulated tokens,
/// then attempts to consume one.
#[derive(Debug)]
pub struct TokenBucket {
    capacity: u32,
    refill_per_sec: f64,
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    /// Build a bucket that starts full (`capacity` tokens already credited).
    /// Panics if `capacity` is zero or `refill_per_sec` is non-positive — both
    /// would create an unusable bucket and reflect a programmer error.
    pub fn new(capacity: u32, refill_per_sec: f64) -> Self {
        assert!(capacity > 0, "TokenBucket capacity must be > 0");
        assert!(
            refill_per_sec.is_finite() && refill_per_sec > 0.0,
            "TokenBucket refill_per_sec must be finite and > 0"
        );
        Self {
            capacity,
            refill_per_sec,
            tokens: capacity as f64,
            last: Instant::now(),
        }
    }

    /// Returns `true` if a token was successfully consumed.
    /// Returns `false` when the bucket is empty (caller should back off).
    pub fn try_acquire(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Wall-clock seconds until the bucket can satisfy one more request.
    /// Returns `0.0` when at least one token is already available.
    pub fn retry_after_sec(&mut self) -> f64 {
        self.refill();
        if self.tokens >= 1.0 {
            0.0
        } else {
            // tokens < 1 → need (1 - tokens) more, at refill_per_sec/sec.
            (1.0 - self.tokens) / self.refill_per_sec
        }
    }

    /// Current bucket level (post-refill). Test helper.
    #[cfg(test)]
    pub fn current_tokens(&mut self) -> f64 {
        self.refill();
        self.tokens
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last);
        // Saturating duration → never negative; safe to cast.
        let new_tokens = elapsed.as_secs_f64() * self.refill_per_sec;
        if new_tokens > 0.0 {
            self.tokens = (self.tokens + new_tokens).min(self.capacity as f64);
            self.last = now;
        }
    }
}

/// Concurrent per-channel limiter. Cheap to clone via `Arc`.
///
/// The map is keyed by the same short channel name returned by
/// [`super::channel_trait::Channel::name`] (e.g. `"telegram"`). Unknown
/// channels are auto-registered at `default_rate` on first `check`.
pub struct PerChannelLimiter {
    /// Per-channel buckets. `ArcSwap` lets the limiter (future work) swap an
    /// entire bucket — e.g. to change the refill rate at runtime — without
    /// blocking concurrent `check` calls on that channel's mutex.
    buckets: DashMap<String, ArcSwap<Mutex<TokenBucket>>>,
    /// Fallback rate applied when `check` sees a channel for the first time
    /// (capacity == rate, 1-second refill window).
    default_rate: u32,
}

impl PerChannelLimiter {
    /// Build an empty limiter. New channel keys are lazily inserted at
    /// `default_rate` tokens/sec when `check` first sees them.
    pub fn new(default_rate: u32) -> Self {
        assert!(default_rate > 0, "default_rate must be > 0");
        Self {
            buckets: DashMap::new(),
            default_rate,
        }
    }

    /// Pre-register a channel with explicit `capacity` and `refill_per_sec`.
    /// Overwrites any existing bucket for `channel`.
    pub fn configure(&self, channel: &str, capacity: u32, refill_per_sec: f64) {
        let bucket = ArcSwap::from_pointee(Mutex::new(TokenBucket::new(capacity, refill_per_sec)));
        self.buckets.insert(channel.to_string(), bucket);
    }

    /// Build a limiter pre-populated with the remote-control production defaults
    /// (Telegram / Slack / WhatsApp). `default_rate` still applies to any
    /// channel name not listed here.
    pub fn with_remote_defaults(default_rate: u32) -> Self {
        let limiter = Self::new(default_rate);
        limiter.configure(
            "telegram",
            defaults::TELEGRAM_RATE,
            defaults::TELEGRAM_RATE as f64,
        );
        limiter.configure("slack", defaults::SLACK_RATE, defaults::SLACK_RATE as f64);
        limiter.configure(
            "whatsapp",
            defaults::WHATSAPP_RATE,
            defaults::WHATSAPP_RATE as f64,
        );
        limiter
    }

    /// Attempt to consume one token from `channel`'s bucket. Returns
    /// `Err(RateLimitError::Throttled { … })` when the bucket is empty.
    ///
    /// The `channel` argument is matched verbatim against bucket keys; for
    /// the production wire-in, `Channel::name()` is what should be passed.
    ///
    /// `static_channel_name` carries the same value but as `&'static str` so
    /// the resulting error stays allocation-free (matches the existing
    /// `ChannelError` shape). A helper takes care of the common case where
    /// these are the same string slice — see [`Self::check`].
    pub fn check_with_static_name(
        &self,
        channel: &str,
        static_channel_name: &'static str,
    ) -> Result<(), RateLimitError> {
        // Fast path: bucket already exists.
        if let Some(entry) = self.buckets.get(channel) {
            return Self::try_consume(&entry, static_channel_name);
        }
        // Slow path: insert at default rate, then retry. `entry()` keeps the
        // insert atomic — concurrent first-time callers race only on the
        // initial bucket build, not on the subsequent consume.
        let entry = self.buckets.entry(channel.to_string()).or_insert_with(|| {
            ArcSwap::from_pointee(Mutex::new(TokenBucket::new(
                self.default_rate,
                self.default_rate as f64,
            )))
        });
        Self::try_consume(&entry, static_channel_name)
    }

    /// Convenience wrapper for the common case where the caller's channel
    /// name is itself a `&'static str` (which it almost always is — see
    /// [`super::channel_trait::ChannelError`]).
    pub fn check(&self, static_channel_name: &'static str) -> Result<(), RateLimitError> {
        self.check_with_static_name(static_channel_name, static_channel_name)
    }

    fn try_consume(
        bucket_swap: &ArcSwap<Mutex<TokenBucket>>,
        channel: &'static str,
    ) -> Result<(), RateLimitError> {
        let guard = bucket_swap.load();
        let mut bucket = guard.lock().unwrap_or_else(|p| p.into_inner());
        if bucket.try_acquire() {
            Ok(())
        } else {
            Err(RateLimitError::Throttled {
                channel,
                retry_after_sec: bucket.retry_after_sec(),
            })
        }
    }
}

impl Default for PerChannelLimiter {
    /// Empty limiter with a generous 100 req/sec default. Production code
    /// should prefer [`PerChannelLimiter::with_remote_defaults`] or call
    /// [`configure`](Self::configure) explicitly.
    fn default() -> Self {
        Self::new(100)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;

    // ── TokenBucket ──────────────────────────────────────────────────────

    #[test]
    fn bucket_starts_full() {
        let mut b = TokenBucket::new(5, 5.0);
        assert!((b.current_tokens() - 5.0).abs() < 1e-9);
    }

    #[test]
    fn bucket_drains_then_blocks() {
        let mut b = TokenBucket::new(3, 1.0);
        assert!(b.try_acquire());
        assert!(b.try_acquire());
        assert!(b.try_acquire());
        assert!(!b.try_acquire(), "4th acquire should fail");
        let retry = b.retry_after_sec();
        assert!(
            retry > 0.0 && retry <= 1.0,
            "retry_after must be in (0, 1] s, got {retry}"
        );
    }

    #[test]
    fn bucket_refills_after_one_second() {
        // capacity = 10, refill = 10/sec → after 1s the bucket is fully credited.
        let mut b = TokenBucket::new(10, 10.0);
        for _ in 0..10 {
            assert!(b.try_acquire());
        }
        assert!(!b.try_acquire());
        thread::sleep(Duration::from_millis(1050));
        // Allow a small fudge; refill is wall-clock based.
        let tokens = b.current_tokens();
        assert!(
            (tokens - 10.0).abs() < 0.5,
            "bucket should be ~full after 1s, got {tokens}"
        );
        assert!(b.try_acquire());
    }

    #[test]
    fn bucket_refill_is_capped_at_capacity() {
        let mut b = TokenBucket::new(2, 100.0);
        // Drain.
        assert!(b.try_acquire());
        assert!(b.try_acquire());
        // Sleep way longer than needed to refill — must cap at capacity.
        thread::sleep(Duration::from_millis(200));
        let tokens = b.current_tokens();
        assert!(tokens <= 2.0, "tokens {tokens} exceeded capacity 2");
        assert!(
            tokens >= 1.5,
            "tokens {tokens} unexpectedly low after long sleep"
        );
    }

    #[test]
    fn retry_after_zero_when_tokens_available() {
        let mut b = TokenBucket::new(5, 5.0);
        assert_eq!(b.retry_after_sec(), 0.0);
    }

    // ── PerChannelLimiter ────────────────────────────────────────────────

    #[test]
    fn limiter_first_call_uses_default_rate() {
        let lim = PerChannelLimiter::new(2);
        assert!(lim.check("unknown").is_ok());
        assert!(lim.check("unknown").is_ok());
        let err = lim.check("unknown").unwrap_err();
        match err {
            RateLimitError::Throttled {
                channel,
                retry_after_sec,
            } => {
                assert_eq!(channel, "unknown");
                assert!(retry_after_sec > 0.0);
            }
        }
    }

    #[test]
    fn limiter_isolates_channels() {
        let lim = PerChannelLimiter::new(1);
        assert!(lim.check("a").is_ok());
        assert!(
            lim.check("a").is_err(),
            "a should be throttled after 1 token"
        );
        assert!(lim.check("b").is_ok(), "b's bucket must be independent");
    }

    #[test]
    fn limiter_into_channel_error_preserves_fields() {
        let lim = PerChannelLimiter::new(1);
        assert!(lim.check("telegram").is_ok());
        let err = lim.check("telegram").unwrap_err().into_channel();
        match err {
            ChannelError::RateLimited {
                channel,
                retry_after_sec,
            } => {
                assert_eq!(channel, "telegram");
                assert!(retry_after_sec > 0.0);
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[test]
    fn limiter_with_remote_defaults_seeds_known_channels() {
        let lim = PerChannelLimiter::with_remote_defaults(50);
        // Slack is 1 msg/sec → after one success the next is throttled.
        assert!(lim.check("slack").is_ok());
        assert!(lim.check("slack").is_err());
        // Telegram is 30 msg/sec → at least the first two should pass.
        assert!(lim.check("telegram").is_ok());
        assert!(lim.check("telegram").is_ok());
        // WhatsApp is 80 msg/sec — same.
        assert!(lim.check("whatsapp").is_ok());
    }

    #[test]
    fn limiter_configure_overwrites_existing_bucket() {
        let lim = PerChannelLimiter::new(100);
        // Prime with default.
        assert!(lim.check("custom").is_ok());
        // Overwrite to capacity = 1, then verify only 1 token is available.
        lim.configure("custom", 1, 1.0);
        assert!(lim.check("custom").is_ok());
        assert!(lim.check("custom").is_err());
    }

    /// Spec test #1 — 100 concurrent try_acquire calls; exactly N == capacity succeed.
    #[test]
    fn limiter_concurrent_acquires_exactly_capacity_succeed() {
        const CAPACITY: u32 = 17; // Pick something non-round to catch off-by-one.
        const THREADS: usize = 100;
        let lim = Arc::new(PerChannelLimiter::new(CAPACITY));
        let barrier = Arc::new(Barrier::new(THREADS));
        let success = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::with_capacity(THREADS);
        for _ in 0..THREADS {
            let lim = Arc::clone(&lim);
            let barrier = Arc::clone(&barrier);
            let success = Arc::clone(&success);
            handles.push(thread::spawn(move || {
                barrier.wait();
                if lim.check("concurrent").is_ok() {
                    success.fetch_add(1, Ordering::SeqCst);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let got = success.load(Ordering::SeqCst);
        // Refill is CAPACITY tokens/sec → over the few ms of contention the
        // bucket may credit one extra token. Accept the canonical value or
        // canonical+1; never accept fewer.
        assert!(
            got == CAPACITY as usize || got == CAPACITY as usize + 1,
            "expected ~{} successes, got {got}",
            CAPACITY
        );
    }

    /// Spec test #2 — after 1 second of idle the bucket is fully refilled.
    #[test]
    fn limiter_refills_to_full_capacity_after_one_second() {
        const CAPACITY: u32 = 5;
        let lim = PerChannelLimiter::new(CAPACITY);
        for _ in 0..CAPACITY {
            assert!(lim.check("refill").is_ok());
        }
        assert!(lim.check("refill").is_err());
        thread::sleep(Duration::from_millis(1100));
        // All CAPACITY tokens must be back.
        for i in 0..CAPACITY {
            assert!(
                lim.check("refill").is_ok(),
                "token {i} should be available after refill"
            );
        }
        assert!(lim.check("refill").is_err());
    }

    /// Spec test #3 — DashMap path is thread-safe: concurrent inserts of
    /// distinct channel keys must not lose updates or panic.
    #[test]
    fn limiter_dashmap_first_insert_is_thread_safe() {
        const THREADS: usize = 64;
        let lim = Arc::new(PerChannelLimiter::new(1));
        let barrier = Arc::new(Barrier::new(THREADS));
        let mut handles = Vec::with_capacity(THREADS);
        for i in 0..THREADS {
            let lim = Arc::clone(&lim);
            let barrier = Arc::clone(&barrier);
            // Leak a static-lifetime channel name per thread; in production
            // channel names are `&'static str` literals so this matches the
            // real usage shape.
            let name: &'static str = Box::leak(format!("ch-{i}").into_boxed_str());
            handles.push(thread::spawn(move || {
                barrier.wait();
                lim.check(name)
            }));
        }
        for h in handles {
            let res = h.join().unwrap();
            // Each thread targets a *unique* channel with capacity 1, so
            // every single check must succeed — no insert race, no lost
            // bucket, no premature throttle.
            assert!(
                res.is_ok(),
                "first acquire on a brand-new channel should succeed"
            );
        }
    }

    /// Same channel under contention from many threads — DashMap shard
    /// locking + Mutex<TokenBucket> must produce a stable success count
    /// equal to capacity (modulo the 1-token refill tolerance).
    #[test]
    fn limiter_dashmap_shared_channel_thread_safe() {
        const CAPACITY: u32 = 10;
        const THREADS: usize = 64;
        let lim = Arc::new(PerChannelLimiter::new(CAPACITY));
        let barrier = Arc::new(Barrier::new(THREADS));
        let success = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::with_capacity(THREADS);
        for _ in 0..THREADS {
            let lim = Arc::clone(&lim);
            let barrier = Arc::clone(&barrier);
            let success = Arc::clone(&success);
            handles.push(thread::spawn(move || {
                barrier.wait();
                if lim.check("shared").is_ok() {
                    success.fetch_add(1, Ordering::SeqCst);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let got = success.load(Ordering::SeqCst);
        assert!(
            got >= CAPACITY as usize && got <= CAPACITY as usize + 2,
            "expected {}..={} successes, got {got}",
            CAPACITY,
            CAPACITY + 2
        );
    }
}
