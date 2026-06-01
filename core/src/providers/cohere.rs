//! Cohere provider adapter (api.cohere.com/v1).
//!
//! Cohere's chat API is **NOT** OpenAI-compatible — it uses its own request
//! shape (`message`/`chat_history` rather than `messages: [...]`) and accepts
//! either `Authorization: Bearer <key>` or `X-API-Key: <key>` for auth. We
//! use `X-API-Key` here per Cohere's recommended header in their auth docs
//! (verified at docs.cohere.com 2026-05-16).
//!
//! Trial keys are free and rate-limited (no credit card required).
//!
//! T51 (v0.6.0 V1) push: this is the only one of the four new adapters whose
//! wire format diverges from OpenAI. The other three (perplexity, ai21,
//! nvidia) reuse the OpenAI streaming codepath — Cohere needs its own
//! adapter when streaming is wired in V2. For V1 we ship metadata + a
//! retry-aware health-check ping that posts a minimal Cohere-shaped body.
//!
//! Gated behind Cargo feature `experimental-hermes-providers` (default OFF).

#![cfg(feature = "experimental-hermes-providers")]

use crate::config::ProviderEntry;
use reqwest::header::{HeaderName, HeaderValue};

/// T11: opt into the retry middleware for this provider's HTTP calls.
pub const RETRY_ENABLED: bool = true;

pub const PROVIDER_ID: &str = "cohere";
pub const DEFAULT_BASE_URL: &str = "https://api.cohere.com";
/// `command-a-03-2025` is Cohere's most capable chat model as of May 2026
/// (verified docs.cohere.com 2026-05-16). Trial keys can call it.
pub const DEFAULT_MODEL: &str = "command-a-03-2025";

/// Cohere's chat header name. Lower-case form is what `reqwest` will send,
/// but Cohere is case-insensitive.
pub const AUTH_HEADER_NAME: &str = "x-api-key";

/// Build the chat endpoint URL for Cohere.
///
/// Cohere's chat path is `/v1/chat` (NOT `/v1/chat/completions` — that is
/// the OpenAI-compat convention used by every other Hermes provider). A
/// regression that appended `/completions` would 404 on the real API.
///
/// Honours operator overrides:
/// * full URL ending in `/v1/chat` or `/chat` — use as-is
/// * URL ending in `/v1` — append `/chat`
/// * otherwise — append `/v1/chat`
pub fn streaming_url(provider: &ProviderEntry) -> String {
    let base = provider.url.as_deref().unwrap_or(DEFAULT_BASE_URL);
    let base = base.trim_end_matches('/');
    if base.ends_with("/chat") {
        return base.to_string();
    }
    if base.ends_with("/v1") {
        return format!("{}/chat", base);
    }
    format!("{}/v1/chat", base)
}

/// Build the `X-API-Key: <key>` header.
///
/// Returns an error if the key contains characters invalid for a header value
/// (e.g. CR/LF — guards against header injection). Note: unlike the OpenAI
/// adapters, the value is the raw key — no `Bearer ` prefix.
pub fn auth_header(
    api_key: &str,
) -> Result<(HeaderName, HeaderValue), reqwest::header::InvalidHeaderValue> {
    let name = HeaderName::from_static(AUTH_HEADER_NAME);
    let value = HeaderValue::from_str(api_key)?;
    Ok((name, value))
}

/// Health-check this provider through the T11 retry middleware.
///
/// Posts a minimal Cohere-shape `chat` ping. Cohere's body shape differs
/// from OpenAI: a single `message` string (no `messages: [...]` array),
/// and `max_tokens` controls the cap. The response shape is also different
/// (no `choices`), but we only check the HTTP status — body parsing happens
/// upstream when the V2 streaming codepath wires in.
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
        "message": "ping",
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
    let _ = outcome.response;
    Ok(())
}

pub struct ProviderInfo {
    pub id: &'static str,
    pub display_name: &'static str,
    pub default_base_url: &'static str,
    pub default_model: &'static str,
    pub api_key_env: &'static str,
}

pub const INFO: ProviderInfo = ProviderInfo {
    id: PROVIDER_ID,
    display_name: "Cohere",
    default_base_url: DEFAULT_BASE_URL,
    default_model: DEFAULT_MODEL,
    api_key_env: "COHERE_API_KEY",
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
    fn streaming_url_default_uses_v1_chat_not_completions() {
        // Critical: Cohere is /v1/chat, NOT /v1/chat/completions. A regression
        // that copied the OpenAI helper would 404 on the real API.
        assert_eq!(streaming_url(&make(None)), "https://api.cohere.com/v1/chat");
    }

    #[test]
    fn streaming_url_full_path_passthrough() {
        assert_eq!(
            streaming_url(&make(Some("https://api.cohere.com/v1/chat"))),
            "https://api.cohere.com/v1/chat"
        );
    }

    #[test]
    fn streaming_url_with_v1_override() {
        assert_eq!(
            streaming_url(&make(Some("https://proxy.example.com/v1"))),
            "https://proxy.example.com/v1/chat"
        );
    }

    #[test]
    fn streaming_url_strips_trailing_slash() {
        assert_eq!(
            streaming_url(&make(Some("https://api.cohere.com/"))),
            "https://api.cohere.com/v1/chat"
        );
    }

    #[test]
    fn auth_header_uses_x_api_key_not_bearer() {
        // Critical regression guard: Cohere uses X-API-Key, not Authorization
        // Bearer. The other 11 Hermes adapters use Bearer; copy/paste from one
        // of those would break Cohere auth.
        let (name, value) = auth_header("co-test-key-123").expect("valid");
        assert_eq!(name.as_str(), "x-api-key");
        assert_eq!(value.to_str().unwrap(), "co-test-key-123");
        // And specifically: no "Bearer " prefix.
        assert!(!value.to_str().unwrap().starts_with("Bearer "));
    }

    #[test]
    fn auth_header_rejects_newline_in_key() {
        // Headers must not contain CR/LF — HTTP smuggling defense, same as
        // the bearer-form adapters.
        let result = auth_header("co-bad\r\nInjected: yes");
        assert!(
            result.is_err(),
            "expected InvalidHeaderValue for CRLF in key"
        );
    }

    #[test]
    fn info_contract() {
        assert_eq!(INFO.id, "cohere");
        assert_eq!(INFO.api_key_env, "COHERE_API_KEY");
        assert_eq!(INFO.default_base_url, "https://api.cohere.com");
        assert_eq!(INFO.default_model, "command-a-03-2025");
    }

    #[test]
    fn retry_enabled_const_is_true() {
        assert!(RETRY_ENABLED, "T11: cohere must opt in to retry middleware");
    }

    #[tokio::test]
    async fn health_check_with_retry_passes_on_200() {
        use wiremock::matchers::{header, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock = MockServer::start().await;
        // Verify the mock receives x-api-key (not Authorization). This pins
        // the auth-header contract end-to-end through health_check_with_retry.
        Mock::given(method("POST"))
            .and(header("x-api-key", "k-test"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&mock)
            .await;
        let provider = ProviderEntry {
            provider_type: PROVIDER_ID.into(),
            url: Some(mock.uri()),
            ..Default::default()
        };
        let res = health_check_with_retry(&provider, "k-test").await;
        assert!(res.is_ok(), "expected Ok, got {:?}", res);
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
