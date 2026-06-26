//! Mistral AI provider adapter (api.mistral.ai/v1).
//!
//! OpenAI-compatible chat completions API. Auth via `Authorization: Bearer <key>`.
//! Free tier (la Plateforme): rate-limited but no credit card required.
//!
//! Gated behind Cargo feature `experimental-extra-providers` (default OFF).

#![cfg(feature = "experimental-extra-providers")]

use crate::config::ProviderEntry;
use reqwest::header::{HeaderName, HeaderValue, AUTHORIZATION};

/// T11: opt into the retry middleware for this provider's HTTP calls.
/// Set to `false` to keep raw `reqwest` semantics. See
/// `core/src/providers/retry.rs` for the contract.
pub const RETRY_ENABLED: bool = true;

/// Stable identifier used in `ProviderEntry.provider_type` for routing.
pub const PROVIDER_ID: &str = "mistral";

/// Default base URL (no trailing slash, no path).
pub const DEFAULT_BASE_URL: &str = "https://api.mistral.ai";

/// Default model when none configured. `mistral-small-latest` is on the free tier.
pub const DEFAULT_MODEL: &str = "mistral-small-latest";

/// Build the chat-completions endpoint URL for a configured provider.
///
/// Honours an explicit `provider.url` if the operator overrode it; otherwise
/// uses [`DEFAULT_BASE_URL`]. Always returns a URL ending in
/// `/v1/chat/completions` so the existing streaming.rs OpenAI codepath can use it.
pub fn streaming_url(provider: &ProviderEntry) -> String {
    let base = provider.url.as_deref().unwrap_or(DEFAULT_BASE_URL);
    let base = base.trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        return base.to_string();
    }
    if base.ends_with("/v1") {
        return format!("{}/chat/completions", base);
    }
    format!("{}/v1/chat/completions", base)
}

/// Build the `Authorization: Bearer <key>` header.
///
/// Returns an error if the key contains characters invalid for a header value
/// (e.g. CR/LF — guards against header injection).
pub fn auth_header(
    api_key: &str,
) -> Result<(HeaderName, HeaderValue), reqwest::header::InvalidHeaderValue> {
    let value = HeaderValue::from_str(&format!("Bearer {}", api_key))?;
    Ok((AUTHORIZATION, value))
}

/// Health-check this provider through the T11 retry middleware.
///
/// Posts a minimal `chat/completions` ping to the provider's streaming URL,
/// returning `Ok(())` on any 2xx and `Err(RetryError)` on terminal failure or
/// exhausted retries. Used by `providers::mod::health_check` when
/// `RETRY_ENABLED` is true.
pub async fn health_check_with_retry(
    provider: &ProviderEntry,
    api_key: &str,
) -> Result<(), crate::providers::retry::RetryError> {
    use crate::providers::retry::{RetryClient, RetryConfig};

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| crate::providers::retry::RetryError {
            provider: PROVIDER_ID.to_string(),
            attempts: 0,
            last_status: None,
            last_body_excerpt: None,
            source: Some(e),
        })?;
    let url = streaming_url(provider);
    let model = provider
        .default_model
        .as_deref()
        .unwrap_or(DEFAULT_MODEL)
        .to_string();
    let key = api_key.to_string();
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1,
        "messages": [{"role": "user", "content": "ping"}],
    });

    let rc = RetryClient::new(client.clone(), RetryConfig::default());
    let outcome = rc
        .execute_with_retry(PROVIDER_ID, || {
            let (name, value) = auth_header(&key).expect("validated upstream");
            client
                .post(&url)
                .header(name, value)
                .header("content-type", "application/json")
                .json(&body)
                .build()
                .expect("request builds")
        })
        .await?;
    let _ = outcome.response; // success status already verified by RetryClient
    Ok(())
}

/// Static metadata for registration in providers::mod.
pub struct ProviderInfo {
    pub id: &'static str,
    pub display_name: &'static str,
    pub default_base_url: &'static str,
    pub default_model: &'static str,
    pub api_key_env: &'static str,
}

pub const INFO: ProviderInfo = ProviderInfo {
    id: PROVIDER_ID,
    display_name: "Mistral AI",
    default_base_url: DEFAULT_BASE_URL,
    default_model: DEFAULT_MODEL,
    api_key_env: "MISTRAL_API_KEY",
};

#[cfg(test)]
mod tests {
    use super::*;

    fn make(url: Option<&str>) -> ProviderEntry {
        ProviderEntry {
            provider_type: PROVIDER_ID.into(),
            url: url.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn streaming_url_default() {
        assert_eq!(
            streaming_url(&make(None)),
            "https://api.mistral.ai/v1/chat/completions"
        );
    }

    #[test]
    fn streaming_url_honours_explicit_v1() {
        assert_eq!(
            streaming_url(&make(Some("https://proxy.example.com/v1"))),
            "https://proxy.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn streaming_url_honours_full_path() {
        assert_eq!(
            streaming_url(&make(Some("https://proxy.example.com/v1/chat/completions"))),
            "https://proxy.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn streaming_url_strips_trailing_slash() {
        assert_eq!(
            streaming_url(&make(Some("https://api.mistral.ai/"))),
            "https://api.mistral.ai/v1/chat/completions"
        );
    }

    #[test]
    fn auth_header_builds_bearer() {
        let (name, value) = auth_header("sk-test-key-123").expect("valid key");
        assert_eq!(name.as_str(), "authorization");
        assert_eq!(value.to_str().unwrap(), "Bearer sk-test-key-123");
    }

    #[test]
    fn auth_header_rejects_newline_in_key() {
        // Headers must not contain CR/LF — this is HTTP smuggling defense.
        let result = auth_header("sk-bad\r\nInjected: yes");
        assert!(
            result.is_err(),
            "expected InvalidHeaderValue for CRLF in key"
        );
    }

    #[test]
    fn info_contract() {
        assert_eq!(INFO.id, "mistral");
        assert_eq!(INFO.api_key_env, "MISTRAL_API_KEY");
        assert!(INFO.default_base_url.starts_with("https://"));
        assert!(!INFO.default_model.is_empty());
    }

    #[test]
    fn retry_enabled_const_is_true() {
        assert!(
            RETRY_ENABLED,
            "T11: mistral must opt in to retry middleware"
        );
    }

    #[tokio::test]
    async fn health_check_with_retry_passes_on_200() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&mock)
            .await;

        let provider = ProviderEntry {
            provider_type: PROVIDER_ID.into(),
            url: Some(mock.uri()),
            ..Default::default()
        };
        let ok = health_check_with_retry(&provider, "k-test").await;
        assert!(ok.is_ok(), "expected Ok, got {:?}", ok);
    }

    #[tokio::test]
    async fn health_check_with_retry_retries_429_then_succeeds() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&mock)
            .await;

        let provider = ProviderEntry {
            provider_type: PROVIDER_ID.into(),
            url: Some(mock.uri()),
            ..Default::default()
        };
        let res = health_check_with_retry(&provider, "k-test").await;
        assert!(res.is_ok(), "expected Ok after retry, got {:?}", res);
    }
}
