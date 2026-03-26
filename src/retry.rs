// retry.rs -- Generic retry-with-backoff utility for Phantom Mesh.
//
// Provides configurable retry logic with multiple backoff policies (Fixed,
// Exponential, Linear), jitter support, and a builder API.  The core
// `retry()` function is async and works with any fallible future.

use std::fmt;
use std::future::Future;
use std::time::{Duration, Instant};

use rand::Rng;
use tracing::{debug, warn};

// ---------------------------------------------------------------------------
// RetryPolicy
// ---------------------------------------------------------------------------

/// Backoff strategy used between retry attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPolicy {
    /// Same delay every attempt.
    Fixed,
    /// Delay multiplied by `backoff_factor` after each attempt.
    Exponential,
    /// Delay increased by `initial_delay_ms` after each attempt.
    Linear,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy::Exponential
    }
}

// ---------------------------------------------------------------------------
// RetryConfig
// ---------------------------------------------------------------------------

/// Configuration controlling how retries behave.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (does not count the initial attempt).
    pub max_retries: u32,
    /// Base delay in milliseconds before the first retry.
    pub initial_delay_ms: u64,
    /// Upper bound on the computed delay in milliseconds.
    pub max_delay_ms: u64,
    /// Multiplicative factor for Exponential backoff.
    pub backoff_factor: f64,
    /// When true, a random jitter of up to 50% of the computed delay is
    /// added (or subtracted) to avoid thundering-herd effects.
    pub jitter: bool,
    /// The backoff strategy.
    pub policy: RetryPolicy,
}

impl Default for RetryConfig {
    /// Reasonable defaults: 3 retries, 1000 ms initial, 30 000 ms max,
    /// exponential with factor 2.0 and jitter enabled.
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 1000,
            max_delay_ms: 30_000,
            backoff_factor: 2.0,
            jitter: true,
            policy: RetryPolicy::Exponential,
        }
    }
}

impl RetryConfig {
    /// Start building a new config with the default values.
    pub fn new() -> Self {
        Self::default()
    }

    // -- builder methods ----------------------------------------------------

    pub fn max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    pub fn initial_delay_ms(mut self, ms: u64) -> Self {
        self.initial_delay_ms = ms;
        self
    }

    pub fn max_delay_ms(mut self, ms: u64) -> Self {
        self.max_delay_ms = ms;
        self
    }

    pub fn backoff_factor(mut self, f: f64) -> Self {
        self.backoff_factor = f;
        self
    }

    pub fn jitter(mut self, enabled: bool) -> Self {
        self.jitter = enabled;
        self
    }

    /// Use the Fixed backoff policy.
    pub fn fixed(mut self) -> Self {
        self.policy = RetryPolicy::Fixed;
        self
    }

    /// Use the Exponential backoff policy.
    pub fn exponential(mut self) -> Self {
        self.policy = RetryPolicy::Exponential;
        self
    }

    /// Use the Linear backoff policy.
    pub fn linear(mut self) -> Self {
        self.policy = RetryPolicy::Linear;
        self
    }

    // -- predefined configs ------------------------------------------------

    /// Aggressive retry: 5 attempts, 500 ms initial, exponential, jitter on.
    pub fn aggressive() -> Self {
        Self {
            max_retries: 5,
            initial_delay_ms: 500,
            max_delay_ms: 30_000,
            backoff_factor: 2.0,
            jitter: true,
            policy: RetryPolicy::Exponential,
        }
    }

    /// Conservative retry: 3 attempts, 5000 ms initial, exponential, jitter on.
    pub fn conservative() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 5000,
            max_delay_ms: 60_000,
            backoff_factor: 2.0,
            jitter: true,
            policy: RetryPolicy::Exponential,
        }
    }

    /// Quick retry: 2 attempts, 200 ms initial, fixed, jitter off.
    pub fn quick() -> Self {
        Self {
            max_retries: 2,
            initial_delay_ms: 200,
            max_delay_ms: 1000,
            backoff_factor: 1.0,
            jitter: false,
            policy: RetryPolicy::Fixed,
        }
    }
}

// ---------------------------------------------------------------------------
// ShouldRetry trait
// ---------------------------------------------------------------------------

/// Implement this trait on your error type to control which errors are
/// retryable and which should cause immediate failure.
pub trait ShouldRetry {
    fn should_retry(&self) -> bool;
}

/// Blanket implementation: all String errors are retryable.
impl ShouldRetry for String {
    fn should_retry(&self) -> bool {
        true
    }
}

/// anyhow::Error is retryable by default.
impl ShouldRetry for anyhow::Error {
    fn should_retry(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// RetryError
// ---------------------------------------------------------------------------

/// Collects every error produced across all attempts so the caller can
/// inspect the full history.
#[derive(Debug)]
pub struct RetryError<E> {
    /// Errors from each failed attempt, in order.
    pub errors: Vec<E>,
}

impl<E: fmt::Debug> fmt::Display for RetryError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "all {} attempts failed: {:?}",
            self.errors.len(),
            self.errors
        )
    }
}

impl<E: fmt::Debug> std::error::Error for RetryError<E> {}

// ---------------------------------------------------------------------------
// RetryResult
// ---------------------------------------------------------------------------

/// Outcome of a `retry()` call, including metadata about how many attempts
/// were needed and how long the whole sequence took.
#[derive(Debug)]
pub struct RetryResult<T, E> {
    /// The final outcome -- `Ok(value)` on success, `Err(RetryError)` when
    /// all attempts have been exhausted.
    pub result: Result<T, RetryError<E>>,
    /// Total number of attempts (1 = succeeded on first try).
    pub attempts: u32,
    /// Wall-clock time from the start of the first attempt to the end of the
    /// last attempt (including sleep time between retries).
    pub total_duration: Duration,
}

impl<T, E> RetryResult<T, E> {
    /// Returns true when the operation eventually succeeded.
    pub fn is_ok(&self) -> bool {
        self.result.is_ok()
    }

    /// Returns true when all attempts failed.
    pub fn is_err(&self) -> bool {
        self.result.is_err()
    }

    /// Unwrap the inner result, panicking on failure.
    pub fn unwrap(self) -> T
    where
        E: fmt::Debug,
    {
        self.result.unwrap()
    }
}

// ---------------------------------------------------------------------------
// calculate_delay
// ---------------------------------------------------------------------------

/// Compute the sleep duration before attempt number `attempt` (0-indexed,
/// where 0 is the delay before the *first retry*, i.e. the second attempt).
pub fn calculate_delay(attempt: u32, config: &RetryConfig) -> Duration {
    let base = config.initial_delay_ms as f64;

    let raw = match config.policy {
        RetryPolicy::Fixed => base,
        RetryPolicy::Exponential => {
            base * config.backoff_factor.powi(attempt as i32)
        }
        RetryPolicy::Linear => {
            base + (base * attempt as f64)
        }
    };

    // Clamp to max_delay_ms.
    let clamped = raw.min(config.max_delay_ms as f64);

    // Apply jitter: +/- up to 50% of the clamped value.
    let final_ms = if config.jitter {
        let mut rng = rand::thread_rng();
        let jitter_range = clamped * 0.5;
        let jitter = rng.gen_range(-jitter_range..=jitter_range);
        (clamped + jitter).max(0.0)
    } else {
        clamped
    };

    Duration::from_millis(final_ms as u64)
}

// ---------------------------------------------------------------------------
// retry()
// ---------------------------------------------------------------------------

/// Retry an async operation according to `config`.
///
/// `operation` is called repeatedly until it succeeds or `max_retries` is
/// exhausted.  Between attempts the function sleeps for a duration computed
/// by `calculate_delay`.
///
/// If the error type implements `ShouldRetry`, attempts that return a
/// non-retryable error will stop the loop immediately.
///
/// # Example
/// ```ignore
/// let cfg = RetryConfig::new().max_retries(3).exponential().jitter(false);
/// let res = retry(&cfg, || async {
///     reqwest::get("https://example.com").await.map_err(|e| e.to_string())
/// }).await;
/// println!("took {} attempts", res.attempts);
/// ```
pub async fn retry<F, Fut, T, E>(config: &RetryConfig, operation: F) -> RetryResult<T, E>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: fmt::Debug + ShouldRetry,
{
    let start = Instant::now();
    let mut errors: Vec<E> = Vec::new();
    let total_attempts = config.max_retries + 1; // initial + retries

    for attempt in 0..total_attempts {
        debug!(attempt = attempt, max = total_attempts, "retry: executing attempt");

        match operation().await {
            Ok(value) => {
                if attempt > 0 {
                    debug!(attempt = attempt, "retry: succeeded after retries");
                }
                return RetryResult {
                    result: Ok(value),
                    attempts: attempt + 1,
                    total_duration: start.elapsed(),
                };
            }
            Err(e) => {
                let is_last = attempt + 1 >= total_attempts;

                if !e.should_retry() {
                    warn!(?e, "retry: error is non-retryable, stopping immediately");
                    errors.push(e);
                    return RetryResult {
                        result: Err(RetryError { errors }),
                        attempts: attempt + 1,
                        total_duration: start.elapsed(),
                    };
                }

                if is_last {
                    warn!(?e, attempt = attempt, "retry: final attempt failed");
                    errors.push(e);
                } else {
                    let delay = calculate_delay(attempt, config);
                    debug!(?e, attempt = attempt, ?delay, "retry: attempt failed, sleeping");
                    errors.push(e);
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    RetryResult {
        result: Err(RetryError { errors }),
        attempts: total_attempts,
        total_duration: start.elapsed(),
    }
}

/// Non-async convenience variant that retries a synchronous closure inside
/// a `tokio::task::spawn_blocking` compatible context.  Sleeps via
/// `std::thread::sleep`.
pub fn retry_sync<F, T, E>(config: &RetryConfig, operation: F) -> RetryResult<T, E>
where
    F: Fn() -> Result<T, E>,
    E: fmt::Debug + ShouldRetry,
{
    let start = Instant::now();
    let mut errors: Vec<E> = Vec::new();
    let total_attempts = config.max_retries + 1;

    for attempt in 0..total_attempts {
        match operation() {
            Ok(value) => {
                return RetryResult {
                    result: Ok(value),
                    attempts: attempt + 1,
                    total_duration: start.elapsed(),
                };
            }
            Err(e) => {
                let is_last = attempt + 1 >= total_attempts;

                if !e.should_retry() {
                    errors.push(e);
                    return RetryResult {
                        result: Err(RetryError { errors }),
                        attempts: attempt + 1,
                        total_duration: start.elapsed(),
                    };
                }

                if is_last {
                    errors.push(e);
                } else {
                    let delay = calculate_delay(attempt, config);
                    errors.push(e);
                    std::thread::sleep(delay);
                }
            }
        }
    }

    RetryResult {
        result: Err(RetryError { errors }),
        attempts: total_attempts,
        total_duration: start.elapsed(),
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    // -- ShouldRetry helpers ------------------------------------------------

    /// A retryable error.
    #[derive(Debug, Clone)]
    struct Retryable(String);
    impl fmt::Display for Retryable {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }
    impl ShouldRetry for Retryable {
        fn should_retry(&self) -> bool {
            true
        }
    }

    /// A non-retryable error (e.g. auth failure).
    #[derive(Debug, Clone)]
    struct NonRetryable(String);
    impl fmt::Display for NonRetryable {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }
    impl ShouldRetry for NonRetryable {
        fn should_retry(&self) -> bool {
            false
        }
    }

    /// An error that is retryable only for specific messages.
    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    struct ConditionalError {
        msg: String,
        retryable: bool,
    }
    impl ShouldRetry for ConditionalError {
        fn should_retry(&self) -> bool {
            self.retryable
        }
    }

    // -- RetryConfig tests --------------------------------------------------

    #[test]
    fn test_default_config() {
        let cfg = RetryConfig::default();
        assert_eq!(cfg.max_retries, 3);
        assert_eq!(cfg.initial_delay_ms, 1000);
        assert_eq!(cfg.max_delay_ms, 30_000);
        assert!((cfg.backoff_factor - 2.0).abs() < f64::EPSILON);
        assert!(cfg.jitter);
        assert_eq!(cfg.policy, RetryPolicy::Exponential);
    }

    #[test]
    fn test_new_equals_default() {
        let a = RetryConfig::new();
        let b = RetryConfig::default();
        assert_eq!(a.max_retries, b.max_retries);
        assert_eq!(a.initial_delay_ms, b.initial_delay_ms);
        assert_eq!(a.max_delay_ms, b.max_delay_ms);
        assert_eq!(a.policy, b.policy);
    }

    #[test]
    fn test_builder_pattern() {
        let cfg = RetryConfig::new()
            .max_retries(5)
            .initial_delay_ms(500)
            .max_delay_ms(10_000)
            .backoff_factor(3.0)
            .jitter(false)
            .exponential();

        assert_eq!(cfg.max_retries, 5);
        assert_eq!(cfg.initial_delay_ms, 500);
        assert_eq!(cfg.max_delay_ms, 10_000);
        assert!((cfg.backoff_factor - 3.0).abs() < f64::EPSILON);
        assert!(!cfg.jitter);
        assert_eq!(cfg.policy, RetryPolicy::Exponential);
    }

    #[test]
    fn test_builder_fixed_policy() {
        let cfg = RetryConfig::new().fixed();
        assert_eq!(cfg.policy, RetryPolicy::Fixed);
    }

    #[test]
    fn test_builder_linear_policy() {
        let cfg = RetryConfig::new().linear();
        assert_eq!(cfg.policy, RetryPolicy::Linear);
    }

    #[test]
    fn test_aggressive_config() {
        let cfg = RetryConfig::aggressive();
        assert_eq!(cfg.max_retries, 5);
        assert_eq!(cfg.initial_delay_ms, 500);
        assert!(cfg.jitter);
        assert_eq!(cfg.policy, RetryPolicy::Exponential);
    }

    #[test]
    fn test_conservative_config() {
        let cfg = RetryConfig::conservative();
        assert_eq!(cfg.max_retries, 3);
        assert_eq!(cfg.initial_delay_ms, 5000);
        assert_eq!(cfg.max_delay_ms, 60_000);
        assert!(cfg.jitter);
    }

    #[test]
    fn test_quick_config() {
        let cfg = RetryConfig::quick();
        assert_eq!(cfg.max_retries, 2);
        assert_eq!(cfg.initial_delay_ms, 200);
        assert_eq!(cfg.max_delay_ms, 1000);
        assert!(!cfg.jitter);
        assert_eq!(cfg.policy, RetryPolicy::Fixed);
    }

    // -- calculate_delay tests ----------------------------------------------

    #[test]
    fn test_fixed_delay() {
        let cfg = RetryConfig::new()
            .initial_delay_ms(1000)
            .fixed()
            .jitter(false);

        let d0 = calculate_delay(0, &cfg);
        let d1 = calculate_delay(1, &cfg);
        let d2 = calculate_delay(2, &cfg);

        assert_eq!(d0.as_millis(), 1000);
        assert_eq!(d1.as_millis(), 1000);
        assert_eq!(d2.as_millis(), 1000);
    }

    #[test]
    fn test_exponential_delay() {
        let cfg = RetryConfig::new()
            .initial_delay_ms(100)
            .backoff_factor(2.0)
            .max_delay_ms(100_000)
            .exponential()
            .jitter(false);

        assert_eq!(calculate_delay(0, &cfg).as_millis(), 100);  // 100 * 2^0
        assert_eq!(calculate_delay(1, &cfg).as_millis(), 200);  // 100 * 2^1
        assert_eq!(calculate_delay(2, &cfg).as_millis(), 400);  // 100 * 2^2
        assert_eq!(calculate_delay(3, &cfg).as_millis(), 800);  // 100 * 2^3
    }

    #[test]
    fn test_linear_delay() {
        let cfg = RetryConfig::new()
            .initial_delay_ms(1000)
            .max_delay_ms(100_000)
            .linear()
            .jitter(false);

        assert_eq!(calculate_delay(0, &cfg).as_millis(), 1000); // 1000 + 1000*0
        assert_eq!(calculate_delay(1, &cfg).as_millis(), 2000); // 1000 + 1000*1
        assert_eq!(calculate_delay(2, &cfg).as_millis(), 3000); // 1000 + 1000*2
    }

    #[test]
    fn test_delay_clamped_to_max() {
        let cfg = RetryConfig::new()
            .initial_delay_ms(1000)
            .backoff_factor(10.0)
            .max_delay_ms(5000)
            .exponential()
            .jitter(false);

        // 1000 * 10^2 = 100_000 but clamped to 5000
        assert_eq!(calculate_delay(2, &cfg).as_millis(), 5000);
    }

    #[test]
    fn test_jitter_stays_within_bounds() {
        let cfg = RetryConfig::new()
            .initial_delay_ms(1000)
            .max_delay_ms(100_000)
            .fixed()
            .jitter(true);

        // Run many times; jitter should keep delay between 500ms and 1500ms.
        for _ in 0..100 {
            let d = calculate_delay(0, &cfg);
            let ms = d.as_millis();
            assert!(ms >= 500, "jitter too low: {}ms", ms);
            assert!(ms <= 1500, "jitter too high: {}ms", ms);
        }
    }

    // -- retry async tests --------------------------------------------------

    #[tokio::test]
    async fn test_retry_succeeds_first_try() {
        let cfg = RetryConfig::new().max_retries(3).jitter(false);

        let result = retry(&cfg, || async {
            Ok::<_, Retryable>(42)
        }).await;

        assert!(result.is_ok());
        assert_eq!(result.attempts, 1);
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_retry_succeeds_after_failures() {
        let counter = Arc::new(AtomicU32::new(0));
        let cfg = RetryConfig::new()
            .max_retries(3)
            .initial_delay_ms(10)
            .jitter(false)
            .fixed();

        let c = counter.clone();
        let result = retry(&cfg, || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    Err(Retryable(format!("fail #{}", n)))
                } else {
                    Ok("success")
                }
            }
        }).await;

        assert!(result.is_ok());
        assert_eq!(result.attempts, 3); // failed twice, succeeded on third
        assert_eq!(result.unwrap(), "success");
    }

    #[tokio::test]
    async fn test_retry_exhausts_all_attempts() {
        let cfg = RetryConfig::new()
            .max_retries(2)
            .initial_delay_ms(10)
            .jitter(false)
            .fixed();

        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let result = retry(&cfg, || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(Retryable("always fails".into()))
            }
        }).await;

        assert!(result.is_err());
        assert_eq!(result.attempts, 3); // 1 initial + 2 retries
        assert_eq!(counter.load(Ordering::SeqCst), 3);

        let err = result.result.unwrap_err();
        assert_eq!(err.errors.len(), 3);
    }

    #[tokio::test]
    async fn test_retry_non_retryable_stops_immediately() {
        let cfg = RetryConfig::new()
            .max_retries(5)
            .initial_delay_ms(10)
            .jitter(false);

        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let result = retry(&cfg, || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(NonRetryable("auth failed".into()))
            }
        }).await;

        assert!(result.is_err());
        assert_eq!(result.attempts, 1); // stopped after first
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_conditional_error() {
        let counter = Arc::new(AtomicU32::new(0));
        let cfg = RetryConfig::new()
            .max_retries(5)
            .initial_delay_ms(10)
            .jitter(false)
            .fixed();

        let c = counter.clone();
        let result = retry(&cfg, || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    // First two: retryable
                    Err::<(), _>(ConditionalError {
                        msg: "timeout".into(),
                        retryable: true,
                    })
                } else {
                    // Third: non-retryable
                    Err(ConditionalError {
                        msg: "auth error".into(),
                        retryable: false,
                    })
                }
            }
        }).await;

        assert!(result.is_err());
        assert_eq!(result.attempts, 3); // 2 retryable + 1 non-retryable stop
        let err = result.result.unwrap_err();
        assert_eq!(err.errors.len(), 3);
        assert!(!err.errors[2].retryable);
    }

    #[tokio::test]
    async fn test_retry_zero_retries_means_single_attempt() {
        let cfg = RetryConfig::new().max_retries(0).jitter(false);

        let result = retry(&cfg, || async {
            Err::<(), _>(Retryable("fail".into()))
        }).await;

        assert!(result.is_err());
        assert_eq!(result.attempts, 1);
    }

    #[tokio::test]
    async fn test_retry_records_total_duration() {
        let cfg = RetryConfig::new()
            .max_retries(1)
            .initial_delay_ms(50)
            .jitter(false)
            .fixed();

        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let result = retry(&cfg, || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Err(Retryable("first fail".into()))
                } else {
                    Ok(())
                }
            }
        }).await;

        assert!(result.is_ok());
        // Should have waited at least 50ms for the retry delay.
        assert!(result.total_duration >= Duration::from_millis(40));
    }

    #[tokio::test]
    async fn test_retry_with_string_error() {
        let cfg = RetryConfig::new()
            .max_retries(1)
            .initial_delay_ms(10)
            .jitter(false);

        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let result = retry(&cfg, || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Err("temporary error".to_string())
                } else {
                    Ok(99)
                }
            }
        }).await;

        assert!(result.is_ok());
        assert_eq!(result.attempts, 2);
        assert_eq!(result.unwrap(), 99);
    }

    // -- retry_sync tests ---------------------------------------------------

    #[test]
    fn test_retry_sync_succeeds_first_try() {
        let cfg = RetryConfig::new().max_retries(2).jitter(false);

        let result = retry_sync(&cfg, || Ok::<_, Retryable>(10));

        assert!(result.is_ok());
        assert_eq!(result.attempts, 1);
        assert_eq!(result.unwrap(), 10);
    }

    #[test]
    fn test_retry_sync_exhausts_retries() {
        let cfg = RetryConfig::new()
            .max_retries(2)
            .initial_delay_ms(5)
            .jitter(false)
            .fixed();

        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let result = retry_sync(&cfg, || {
            c.fetch_add(1, Ordering::SeqCst);
            Err::<(), _>(Retryable("nope".into()))
        });

        assert!(result.is_err());
        assert_eq!(result.attempts, 3);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_retry_sync_non_retryable() {
        let cfg = RetryConfig::new()
            .max_retries(5)
            .initial_delay_ms(5)
            .jitter(false);

        let result = retry_sync(&cfg, || {
            Err::<(), _>(NonRetryable("permanent".into()))
        });

        assert!(result.is_err());
        assert_eq!(result.attempts, 1);
    }

    #[test]
    fn test_retry_sync_succeeds_after_failures() {
        let cfg = RetryConfig::new()
            .max_retries(4)
            .initial_delay_ms(5)
            .jitter(false)
            .fixed();

        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let result = retry_sync(&cfg, || {
            let n = c.fetch_add(1, Ordering::SeqCst);
            if n < 3 {
                Err(Retryable(format!("fail #{}", n)))
            } else {
                Ok("done")
            }
        });

        assert!(result.is_ok());
        assert_eq!(result.attempts, 4);
        assert_eq!(result.unwrap(), "done");
    }

    // -- RetryError display -------------------------------------------------

    #[test]
    fn test_retry_error_display() {
        let err: RetryError<String> = RetryError {
            errors: vec!["e1".into(), "e2".into()],
        };
        let msg = format!("{}", err);
        assert!(msg.contains("2 attempts failed"));
    }

    // -- RetryResult helpers ------------------------------------------------

    #[test]
    fn test_retry_result_is_ok_is_err() {
        let ok: RetryResult<i32, String> = RetryResult {
            result: Ok(1),
            attempts: 1,
            total_duration: Duration::ZERO,
        };
        assert!(ok.is_ok());
        assert!(!ok.is_err());

        let err: RetryResult<i32, String> = RetryResult {
            result: Err(RetryError {
                errors: vec!["x".into()],
            }),
            attempts: 1,
            total_duration: Duration::ZERO,
        };
        assert!(err.is_err());
        assert!(!err.is_ok());
    }

    // -- RetryPolicy default ------------------------------------------------

    #[test]
    fn test_retry_policy_default_is_exponential() {
        assert_eq!(RetryPolicy::default(), RetryPolicy::Exponential);
    }

    // -- Edge case: max_retries = 0 sync ------------------------------------

    #[test]
    fn test_retry_sync_zero_retries() {
        let cfg = RetryConfig::new().max_retries(0).jitter(false);

        let result = retry_sync(&cfg, || Err::<(), _>(Retryable("only one shot".into())));

        assert!(result.is_err());
        assert_eq!(result.attempts, 1);
        let err = result.result.unwrap_err();
        assert_eq!(err.errors.len(), 1);
    }

    // -- Exponential with large attempt clamped -----------------------------

    #[test]
    fn test_exponential_large_attempt_clamped() {
        let cfg = RetryConfig::new()
            .initial_delay_ms(100)
            .backoff_factor(2.0)
            .max_delay_ms(5000)
            .exponential()
            .jitter(false);

        // 100 * 2^20 = 104_857_600, but clamped to 5000
        let d = calculate_delay(20, &cfg);
        assert_eq!(d.as_millis(), 5000);
    }

    // -- Builder chaining changes policy multiple times ----------------------

    #[test]
    fn test_builder_policy_override() {
        let cfg = RetryConfig::new()
            .fixed()
            .linear()
            .exponential();
        assert_eq!(cfg.policy, RetryPolicy::Exponential);

        let cfg2 = RetryConfig::new()
            .exponential()
            .fixed();
        assert_eq!(cfg2.policy, RetryPolicy::Fixed);
    }

    // -- Verify all errors are collected in order ---------------------------

    #[tokio::test]
    async fn test_errors_collected_in_order() {
        let cfg = RetryConfig::new()
            .max_retries(3)
            .initial_delay_ms(5)
            .jitter(false)
            .fixed();

        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let result = retry(&cfg, || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(Retryable(format!("error-{}", n)))
            }
        }).await;

        let err = result.result.unwrap_err();
        assert_eq!(err.errors.len(), 4);
        assert_eq!(err.errors[0].0, "error-0");
        assert_eq!(err.errors[1].0, "error-1");
        assert_eq!(err.errors[2].0, "error-2");
        assert_eq!(err.errors[3].0, "error-3");
    }
}
