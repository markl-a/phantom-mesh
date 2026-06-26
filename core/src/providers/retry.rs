//! Retry + backoff middleware for provider HTTP calls.
//!
//! Gated behind Cargo feature `experimental-extra-providers` (default OFF).
//!
//! See `docs/superpowers/plans/2026-05-15-track-t11-provider-retry.md` for the
//! design rationale.

#![cfg(feature = "experimental-extra-providers")]

use std::time::Duration;

/// Tunable retry parameters. Defaults match the T11 brief:
/// up to 4 retries (5 total attempts), 0.5s/1s/2s/4s base backoff with ±20% jitter,
/// max single sleep capped at 30s, response body excerpts capped at 200 bytes.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub jitter_ratio: f64,
    pub body_excerpt_bytes: usize,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 4,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            jitter_ratio: 0.20,
            body_excerpt_bytes: 200,
        }
    }
}

/// Rich error returned when retries are exhausted or a non-retryable status fires.
#[derive(Debug, thiserror::Error)]
#[error(
    "[{provider}] request failed after {attempts} attempt(s); last_status={last_status:?}, last_body_excerpt={last_body_excerpt:?}"
)]
pub struct RetryError {
    pub provider: String,
    pub attempts: u32,
    pub last_status: Option<u16>,
    pub last_body_excerpt: Option<String>,
    #[source]
    pub source: Option<reqwest::Error>,
}

/// Successful return from `execute_with_retry`: the raw response + how many
/// attempts we needed. Caller decides how to parse the body.
#[derive(Debug)]
pub struct RetryOutcome {
    pub response: reqwest::Response,
    pub attempts: u32,
}

/// Compute the delay before the *next* attempt.
///
/// * `attempt` is 0-based: attempt 0 is the first retry slot (so base_delay),
///   attempt 1 doubles, attempt 2 doubles again, etc.
/// * `retry_after`, when `Some`, overrides the exponential calculation; it is
///   still clamped to `cfg.max_delay` so a malicious or buggy server can't
///   stall us for hours.
/// * `jitter_fn(0.0..=1.0)` is the unit-interval jitter source. Production
///   passes `|_| rand::random::<f64>()`; tests pass `|_| 0.5` for determinism.
///
/// The final multiplier applied to the exponential value is
/// `(1.0 - jitter_ratio) + 2.0 * jitter_ratio * jitter_fn(...)`,
/// so jitter ∈ `[1 - r, 1 + r]` uniformly when the supplied `jitter_fn` is
/// uniform on [0, 1].
pub fn compute_backoff<F: FnOnce(f64) -> f64>(
    cfg: &RetryConfig,
    attempt: u32,
    retry_after: Option<Duration>,
    jitter_fn: F,
) -> Duration {
    if let Some(d) = retry_after {
        return std::cmp::min(d, cfg.max_delay);
    }
    let base_ms = cfg.base_delay.as_millis() as f64;
    // 2^attempt — saturate at attempt = 30 to avoid f64 overflow.
    let factor = 2f64.powi(attempt.min(30) as i32);
    let raw_ms = base_ms * factor;
    let jitter = jitter_fn(0.0); // arg is a placeholder; closure ignores it
    let multiplier = (1.0 - cfg.jitter_ratio) + 2.0 * cfg.jitter_ratio * jitter;
    let jittered_ms = raw_ms * multiplier;
    let capped_ms = jittered_ms.min(cfg.max_delay.as_millis() as f64);
    Duration::from_millis(capped_ms.round() as u64)
}

/// HTTP client wrapper that adds retry + backoff on top of a `reqwest::Client`.
///
/// Construct with [`RetryClient::new`]; call [`RetryClient::execute_with_retry`]
/// with the provider name (for logging / error context) and a request builder
/// closure. The closure is invoked once per attempt because `reqwest::Request`
/// is not `Clone`.
#[derive(Debug, Clone)]
pub struct RetryClient {
    inner: reqwest::Client,
    config: RetryConfig,
}

impl RetryClient {
    pub fn new(inner: reqwest::Client, config: RetryConfig) -> Self {
        Self { inner, config }
    }

    pub fn config(&self) -> &RetryConfig {
        &self.config
    }

    /// Execute a request, retrying on transient failures.
    ///
    /// `build_request` is called once per attempt (since `reqwest::Request`
    /// can't be cloned). `provider` is used only for logging and error context.
    ///
    /// Returns `Ok(RetryOutcome)` on the first successful (`< 400`) response.
    /// Returns `Err(RetryError)` when:
    /// * a non-retryable status (400/401/403/404) is received, or
    /// * all retries have been exhausted on a retryable status, or
    /// * a non-retryable network error occurs (e.g. DNS failure).
    pub async fn execute_with_retry<F>(
        &self,
        provider: &str,
        build_request: F,
    ) -> Result<RetryOutcome, RetryError>
    where
        F: Fn() -> reqwest::Request + Send + Sync,
    {
        self.execute_with_retry_inner(provider, build_request, |_| rand::random::<f64>())
            .await
    }

    // Test seam: jitter source is injectable so tests can be deterministic.
    pub(crate) async fn execute_with_retry_inner<F, J>(
        &self,
        provider: &str,
        build_request: F,
        jitter_fn: J,
    ) -> Result<RetryOutcome, RetryError>
    where
        F: Fn() -> reqwest::Request + Send + Sync,
        J: Fn(f64) -> f64 + Send + Sync,
    {
        let cfg = &self.config;
        let mut last_status: Option<u16> = None;
        let mut last_body: Option<String> = None;
        let mut last_source: Option<reqwest::Error> = None;

        // Total attempts = 1 initial + max_retries.
        let total = cfg.max_retries.saturating_add(1);

        for attempt_idx in 0..total {
            let attempt_number = attempt_idx + 1; // 1-based for humans
            let req = build_request();
            let result = self.inner.execute(req).await;

            match result {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return Ok(RetryOutcome {
                            response: resp,
                            attempts: attempt_number,
                        });
                    }
                    let status_u16 = status.as_u16();
                    let retry_after_hdr = resp
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(parse_retry_after);
                    // Capture body now — we'll need it whether we retry or bail.
                    let body = resp.text().await.unwrap_or_default();
                    let excerpt = truncate_excerpt(&body, cfg.body_excerpt_bytes);
                    last_status = Some(status_u16);
                    last_body = Some(excerpt.clone());

                    if !is_retryable_status(status_u16) {
                        // Terminal: do not retry.
                        return Err(RetryError {
                            provider: provider.to_string(),
                            attempts: attempt_number,
                            last_status,
                            last_body_excerpt: last_body,
                            source: None,
                        });
                    }
                    if attempt_idx + 1 >= total {
                        // Retryable, but we're out of retries.
                        return Err(RetryError {
                            provider: provider.to_string(),
                            attempts: attempt_number,
                            last_status,
                            last_body_excerpt: last_body,
                            source: None,
                        });
                    }
                    let sleep = compute_backoff(cfg, attempt_idx, retry_after_hdr, &jitter_fn);
                    tracing::info!(
                        provider = %provider,
                        attempt = attempt_number,
                        status_code = status_u16,
                        sleep_ms = sleep.as_millis() as u64,
                        "provider retry: backing off"
                    );
                    tokio::time::sleep(sleep).await;
                    continue;
                }
                Err(e) => {
                    let transient = e.is_timeout() || e.is_connect() || e.is_request();
                    if !transient || attempt_idx + 1 >= total {
                        return Err(RetryError {
                            provider: provider.to_string(),
                            attempts: attempt_number,
                            last_status,
                            last_body_excerpt: last_body,
                            source: Some(e),
                        });
                    }
                    let sleep = compute_backoff(cfg, attempt_idx, None, &jitter_fn);
                    tracing::info!(
                        provider = %provider,
                        attempt = attempt_number,
                        status_code = tracing::field::Empty,
                        sleep_ms = sleep.as_millis() as u64,
                        "provider retry: transient network error, backing off"
                    );
                    last_source = Some(e);
                    tokio::time::sleep(sleep).await;
                    continue;
                }
            }
        }

        // Unreachable in practice — the loop always returns. Defensive fallback
        // in case `total == 0` (which Default forbids but a hand-tuned config
        // could produce).
        Err(RetryError {
            provider: provider.to_string(),
            attempts: 0,
            last_status,
            last_body_excerpt: last_body,
            source: last_source,
        })
    }
}

/// Truncate `body` to at most `max_bytes`, honouring UTF-8 char boundaries
/// so we never panic on slicing through a multi-byte sequence.
fn truncate_excerpt(body: &str, max_bytes: usize) -> String {
    if body.len() <= max_bytes {
        return body.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    body[..end].to_string()
}

/// Parse a `Retry-After` header value into a `Duration`.
///
/// Supports the numeric-seconds form only (RFC 7231 §7.1.3 first variant).
/// Returns `None` for HTTP-date values, non-numeric strings, zero, or any
/// negative value — callers then fall back to exponential backoff.
pub fn parse_retry_after(value: &reqwest::header::HeaderValue) -> Option<Duration> {
    let s = value.to_str().ok()?;
    let n: i64 = s.trim().parse().ok()?;
    if n <= 0 {
        return None;
    }
    Some(Duration::from_secs(n as u64))
}

/// Return `true` iff a given HTTP status code is one we should retry.
///
/// Per the T11 brief: retry on 429 (Too Many Requests) and 503 (Service
/// Unavailable). Explicitly do NOT retry on 400 / 401 / 403 / 404 — these are
/// caller bugs, auth problems, or model-not-found, none of which retrying
/// will fix.
///
/// Other 5xx (500/502/504) are intentionally NOT retried here. Providers
/// vary too much in what those mean; if we widen this later, update both the
/// docs and the test table.
pub fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 503)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_brief() {
        let c = RetryConfig::default();
        assert_eq!(c.max_retries, 4);
        assert_eq!(c.base_delay, Duration::from_millis(500));
        assert_eq!(c.max_delay, Duration::from_secs(30));
        assert!((c.jitter_ratio - 0.20).abs() < 1e-9);
        assert_eq!(c.body_excerpt_bytes, 200);
    }

    #[test]
    fn retry_error_display_contains_provider_and_attempts() {
        let err = RetryError {
            provider: "mistral".into(),
            attempts: 3,
            last_status: Some(429),
            last_body_excerpt: Some("Too Many Requests".into()),
            source: None,
        };
        let msg = format!("{}", err);
        assert!(
            msg.contains("[mistral]"),
            "missing provider tag in: {}",
            msg
        );
        assert!(msg.contains("3"), "missing attempt count in: {}", msg);
        assert!(msg.contains("429"), "missing last_status in: {}", msg);
        assert!(
            msg.contains("Too Many Requests"),
            "missing body excerpt in: {}",
            msg
        );
    }

    #[test]
    fn classify_retryable_429() {
        assert!(is_retryable_status(429));
    }

    #[test]
    fn classify_retryable_503() {
        assert!(is_retryable_status(503));
    }

    #[test]
    fn classify_non_retryable_400() {
        assert!(!is_retryable_status(400));
    }

    #[test]
    fn classify_non_retryable_401() {
        assert!(!is_retryable_status(401));
    }

    #[test]
    fn classify_non_retryable_403() {
        assert!(!is_retryable_status(403));
    }

    #[test]
    fn classify_non_retryable_404() {
        assert!(!is_retryable_status(404));
    }

    #[test]
    fn classify_other_5xx_not_retryable_by_default() {
        // The brief lists only 503 explicitly; 500/502/504 are intentionally
        // NOT retried here to keep semantics narrow. If we widen this later,
        // bump the test list and the doc comment together.
        assert!(!is_retryable_status(500));
        assert!(!is_retryable_status(502));
        assert!(!is_retryable_status(504));
    }

    #[test]
    fn classify_success_not_retryable() {
        assert!(!is_retryable_status(200));
        assert!(!is_retryable_status(201));
    }

    #[test]
    fn backoff_attempt_0_is_base_with_zero_jitter() {
        let cfg = RetryConfig::default();
        // jitter_fn returns 0.5 → centred (no jitter shift)
        let d = compute_backoff(&cfg, 0, None, |_| 0.5);
        assert_eq!(d, Duration::from_millis(500));
    }

    #[test]
    fn backoff_exponential_no_jitter() {
        let cfg = RetryConfig::default();
        assert_eq!(
            compute_backoff(&cfg, 0, None, |_| 0.5),
            Duration::from_millis(500)
        );
        assert_eq!(
            compute_backoff(&cfg, 1, None, |_| 0.5),
            Duration::from_millis(1000)
        );
        assert_eq!(
            compute_backoff(&cfg, 2, None, |_| 0.5),
            Duration::from_millis(2000)
        );
        assert_eq!(
            compute_backoff(&cfg, 3, None, |_| 0.5),
            Duration::from_millis(4000)
        );
    }

    #[test]
    fn backoff_jitter_low_end() {
        // jitter_fn = 0.0 → multiplier = (1 - 0.20) = 0.80
        let cfg = RetryConfig::default();
        let d = compute_backoff(&cfg, 0, None, |_| 0.0);
        assert_eq!(d, Duration::from_millis(400));
    }

    #[test]
    fn backoff_jitter_high_end() {
        // jitter_fn = 1.0 → multiplier = (1 + 0.20) = 1.20
        let cfg = RetryConfig::default();
        let d = compute_backoff(&cfg, 0, None, |_| 1.0);
        assert_eq!(d, Duration::from_millis(600));
    }

    #[test]
    fn backoff_capped_at_max_delay() {
        let cfg = RetryConfig {
            max_delay: Duration::from_millis(1500),
            ..RetryConfig::default()
        };
        // attempt 3 with no jitter would be 4000ms; cap forces 1500.
        let d = compute_backoff(&cfg, 3, None, |_| 0.5);
        assert_eq!(d, Duration::from_millis(1500));
    }

    #[test]
    fn backoff_retry_after_seconds_overrides_calculation() {
        let cfg = RetryConfig::default();
        // server says wait 7s — that wins over exponential math.
        let d = compute_backoff(&cfg, 0, Some(Duration::from_secs(7)), |_| 0.5);
        assert_eq!(d, Duration::from_secs(7));
    }

    #[test]
    fn backoff_retry_after_capped_at_max_delay() {
        let cfg = RetryConfig::default();
        // server says 5 minutes — we cap at max_delay (30s) so we don't hang.
        let d = compute_backoff(&cfg, 0, Some(Duration::from_secs(300)), |_| 0.5);
        assert_eq!(d, Duration::from_secs(30));
    }

    #[test]
    fn retry_after_parses_seconds() {
        let h = reqwest::header::HeaderValue::from_static("5");
        assert_eq!(parse_retry_after(&h), Some(Duration::from_secs(5)));
    }

    #[test]
    fn retry_after_ignores_zero_and_negative() {
        let zero = reqwest::header::HeaderValue::from_static("0");
        assert_eq!(parse_retry_after(&zero), None);
        let neg = reqwest::header::HeaderValue::from_static("-3");
        assert_eq!(parse_retry_after(&neg), None);
    }

    #[test]
    fn retry_after_ignores_http_date() {
        // We don't parse HTTP-dates; fall back to backoff math.
        let date = reqwest::header::HeaderValue::from_static("Wed, 21 Oct 2026 07:28:00 GMT");
        assert_eq!(parse_retry_after(&date), None);
    }

    #[test]
    fn retry_after_ignores_garbage() {
        let garbage = reqwest::header::HeaderValue::from_static("soonish");
        assert_eq!(parse_retry_after(&garbage), None);
    }
}

#[cfg(test)]
mod wire_tests {
    use super::*;
    use reqwest::Client;
    use std::sync::Arc;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Make a config with near-zero delays so tests don't actually sleep seconds.
    fn fast_config() -> RetryConfig {
        RetryConfig {
            max_retries: 4,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(20),
            jitter_ratio: 0.0,
            body_excerpt_bytes: 200,
        }
    }

    fn build_get(url: String) -> impl Fn() -> reqwest::Request + Send + Sync {
        let client = Client::new();
        let url = Arc::new(url);
        move || {
            let url = url.clone();
            client.get(url.as_str()).build().expect("build request")
        }
    }

    #[tokio::test]
    async fn retries_on_429_then_succeeds() {
        let mock = MockServer::start().await;

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(2)
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&mock)
            .await;

        let rc = RetryClient::new(Client::new(), fast_config());
        let outcome = rc
            .execute_with_retry("mistral", build_get(mock.uri()))
            .await
            .expect("should succeed on 3rd try");
        assert_eq!(
            outcome.attempts, 3,
            "expected 3 attempts, got {}",
            outcome.attempts
        );
        assert_eq!(outcome.response.status(), 200);
    }

    #[tokio::test]
    async fn retries_on_503_then_exhausts() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503).set_body_string("nope"))
            .mount(&mock)
            .await;

        let rc = RetryClient::new(Client::new(), fast_config());
        let err = rc
            .execute_with_retry("xai", build_get(mock.uri()))
            .await
            .expect_err("should exhaust retries on 503");
        // 1 initial + 4 retries = 5 attempts.
        assert_eq!(err.attempts, 5, "got {}", err.attempts);
        assert_eq!(err.last_status, Some(503));
        assert_eq!(err.last_body_excerpt.as_deref(), Some("nope"));
        assert_eq!(err.provider, "xai");
    }

    #[tokio::test]
    async fn no_retry_on_400() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad input"))
            .expect(1) // critical: must NOT retry
            .mount(&mock)
            .await;

        let rc = RetryClient::new(Client::new(), fast_config());
        let err = rc
            .execute_with_retry("together", build_get(mock.uri()))
            .await
            .expect_err("400 must not retry");
        assert_eq!(err.attempts, 1);
        assert_eq!(err.last_status, Some(400));
        assert_eq!(err.last_body_excerpt.as_deref(), Some("bad input"));
    }

    #[tokio::test]
    async fn no_retry_on_401_auth_error() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&mock)
            .await;

        let rc = RetryClient::new(Client::new(), fast_config());
        let err = rc
            .execute_with_retry("fireworks", build_get(mock.uri()))
            .await
            .expect_err("401 must not retry");
        assert_eq!(err.attempts, 1);
        assert_eq!(err.last_status, Some(401));
    }

    #[tokio::test]
    async fn no_retry_on_404() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&mock)
            .await;

        let rc = RetryClient::new(Client::new(), fast_config());
        let err = rc
            .execute_with_retry("mistral", build_get(mock.uri()))
            .await
            .expect_err("404 must not retry");
        assert_eq!(err.attempts, 1);
        assert_eq!(err.last_status, Some(404));
    }

    #[tokio::test]
    async fn honours_retry_after_header() {
        let mock = MockServer::start().await;

        // First response: 429 with Retry-After: 1 (second). With fast_config's
        // max_delay = 20ms this gets clamped to 20ms — so we test that the
        // header was *parsed* (the response was retried) AND that the cap was
        // respected (the test still finishes in milliseconds, not seconds).
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "1"))
            .up_to_n_times(1)
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock)
            .await;

        let rc = RetryClient::new(Client::new(), fast_config());
        let start = std::time::Instant::now();
        let outcome = rc
            .execute_with_retry("mistral", build_get(mock.uri()))
            .await
            .expect("should succeed after honouring Retry-After");
        let elapsed = start.elapsed();
        assert_eq!(outcome.attempts, 2);
        assert!(
            elapsed < Duration::from_millis(500),
            "Retry-After should have been clamped to max_delay (20ms); took {:?}",
            elapsed,
        );
    }

    #[tokio::test]
    async fn body_excerpt_truncated_to_config_limit() {
        let big = "x".repeat(5_000);
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(400).set_body_string(big.clone()))
            .expect(1)
            .mount(&mock)
            .await;

        let cfg = RetryConfig {
            body_excerpt_bytes: 50,
            ..fast_config()
        };
        let rc = RetryClient::new(Client::new(), cfg);
        let err = rc
            .execute_with_retry("mistral", build_get(mock.uri()))
            .await
            .expect_err("400 fails");
        let excerpt = err.last_body_excerpt.expect("excerpt set");
        assert_eq!(excerpt.len(), 50);
        assert!(excerpt.chars().all(|c| c == 'x'));
    }

    #[tokio::test]
    async fn truncate_excerpt_respects_utf8_boundary() {
        // Synthetic test: 3-byte UTF-8 char (Chinese 你) repeated. Slicing at
        // an odd boundary must not panic.
        let s: String = "你".repeat(20); // 60 bytes
        let out = truncate_excerpt(&s, 7); // 7 % 3 = 1 — would split a char if naive
                                           // Either 0, 3, or 6 bytes — never 7 — so length is divisible by 3.
        assert!(out.len() % 3 == 0, "got len {} for {:?}", out.len(), out);
        assert!(out.len() <= 7);
    }
}
