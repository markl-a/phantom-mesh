//! Shared async HTTP client wrapper built on [`reqwest`].
//!
//! Provides [`HttpClient`], a thin convenience layer over [`reqwest::Client`]
//! that adds automatic retry-with-backoff for transient failures and JSON
//! request/response handling for typed `GET`/`POST` calls.
//!
//! # Behavior
//!
//! - **Retries**: governed by [`RetryPolicy`]. Only *transient* errors are
//!   retried — connection failures, timeouts, and `5xx` server responses.
//!   Non-transient errors (e.g. `4xx` client errors) are returned immediately
//!   without retrying.
//! - **Backoff**: delays grow exponentially from
//!   [`RetryPolicy::base_delay`] by [`RetryPolicy::backoff_factor`], capped at
//!   [`RetryPolicy::max_delay`].
//! - **Timeouts / default headers**: inherited from the underlying
//!   [`reqwest::Client`]. The default constructor uses `reqwest`'s defaults;
//!   callers needing custom timeouts or headers can extend this wrapper.
//! - **Serialization**: responses are deserialized from JSON into any
//!   [`serde::de::DeserializeOwned`] type; `POST` bodies are serialized as JSON.
//!
//! # Example
//!
//! ```ignore
//! use serde::Deserialize;
//!
//! #[derive(Deserialize)]
//! struct Item { value: i32 }
//!
//! let client = HttpClient::new();
//! let item: Item = client.get("https://example.com/api/item").await?;
//! ```

use reqwest::{Client, Error};
use serde::de::DeserializeOwned;
use std::time::Duration;
use tokio::time::sleep;

/// Configuration for retry behavior.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts (including the initial attempt).
    pub max_attempts: u32,
    /// Base delay for exponential backoff.
    pub base_delay: Duration,
    /// Maximum delay between retries.
    pub max_delay: Duration,
    /// Factor by which delay multiplies each attempt.
    pub backoff_factor: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(5),
            backoff_factor: 2.0,
        }
    }
}

/// An async HTTP client with retry capabilities.
#[derive(Debug, Clone)]
pub struct HttpClient {
    inner: Client,
    retry_policy: RetryPolicy,
}

impl HttpClient {
    /// Create a new `HttpClient` with default retry policy.
    pub fn new() -> Self {
        Self {
            inner: Client::new(),
            retry_policy: RetryPolicy::default(),
        }
    }

    /// Create a new `HttpClient` with a custom retry policy.
    pub fn with_retry_policy(retry_policy: RetryPolicy) -> Self {
        Self {
            inner: Client::new(),
            retry_policy,
        }
    }

    /// Determine if an error is transient and worth retrying.
    fn is_transient_error(err: &Error) -> bool {
        // Network errors, timeouts, and connection errors are transient.
        if err.is_timeout() {
            return true;
        }
        if err.is_connect() {
            return true;
        }
        // Status errors: retry on 5xx server errors.
        if err.is_status() {
            if let Some(status) = err.status() {
                return status.is_server_error();
            }
        }
        false
    }

    /// Perform a GET request with retry logic.
    pub async fn get<T: DeserializeOwned>(&self, url: &str) -> Result<T, Error> {
        self.request_with_retry::<T, _>(|| self.inner.get(url))
            .await
    }

    /// Perform a POST request with retry logic.
    pub async fn post<T: DeserializeOwned, B: serde::Serialize>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<T, Error> {
        self.request_with_retry::<T, _>(|| self.inner.post(url).json(body))
            .await
    }

    /// Internal helper to execute a request builder with retry.
    async fn request_with_retry<T: DeserializeOwned, F>(
        &self,
        mut request_builder: F,
    ) -> Result<T, Error>
    where
        F: FnMut() -> reqwest::RequestBuilder,
    {
        let policy = &self.retry_policy;
        let mut attempt = 0u32;
        let mut delay = policy.base_delay;

        loop {
            let response = request_builder().send().await?;
            let status = response.status();

            if status.is_success() {
                // Success! Deserialize and return.
                return response.json().await;
            }

            // Handle HTTP error status.
            let err = response.error_for_status().err().unwrap();
            if !Self::is_transient_error(&err) {
                // Return the error immediately for non-transient cases (e.g., 4xx).
                return Err(err);
            }
            // For transient 5xx, fall through to retry logic.

            // If we've exhausted retries, return the last error.
            if attempt >= policy.max_attempts - 1 {
                eprintln!(
                    "HTTP client: exhausted retries after {} attempts",
                    attempt + 1
                );
                return Err(err);
            }

            eprintln!(
                "HTTP client: attempt {} failed with status {}, retrying in {:?}",
                attempt + 1,
                status,
                delay
            );
            // Wait before retrying.
            sleep(delay).await;
            attempt += 1;
            // Exponential backoff with cap.
            delay = std::cmp::min(delay.mul_f64(policy.backoff_factor), policy.max_delay);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_get_success() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/success"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "value": 42 })))
            .expect(1)
            .mount(&mock)
            .await;

        let client = HttpClient::new();
        let resp: MyResponse = client
            .get(&format!("{}/success", mock.uri()))
            .await
            .unwrap();
        assert_eq!(resp.value, 42);
    }

    #[tokio::test]
    async fn test_get_retries_on_transient_error() {
        let mock = MockServer::start().await;
        // First two attempts return 503, third succeeds.
        Mock::given(method("GET"))
            .and(path("/flaky"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .expect(1)
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/flaky"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .expect(1)
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/flaky"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ok": true })))
            .expect(1)
            .mount(&mock)
            .await;

        let client = HttpClient::with_retry_policy(RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(10),
            ..Default::default()
        });
        let resp: FlakyResponse = client.get(&format!("{}/flaky", mock.uri())).await.unwrap();
        assert!(resp.ok);
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct MyResponse {
        value: i32,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct FlakyResponse {
        ok: bool,
    }
}
