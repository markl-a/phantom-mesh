//! AI21 Labs provider adapter (api.ai21.com/studio/v1).
//!
//! OpenAI-compatible chat completions. Auth: `Authorization: Bearer <key>`.
//! Hosts the Jamba family (hybrid SSM-Transformer with 256k context).
//! AI21 Studio offers ~$10 of free credit on signup (docs 2026-05-16).
//!
//! T51 (v0.6.0 V1) push: added alongside perplexity, nvidia, cohere.
//!
//! Gated behind Cargo feature `experimental-extra-providers` (default OFF).

#![cfg(feature = "experimental-extra-providers")]

use crate::config::ProviderEntry;
use reqwest::header::{HeaderName, HeaderValue, AUTHORIZATION};

/// T11: opt into the retry middleware for this provider's HTTP calls.
pub const RETRY_ENABLED: bool = true;

pub const PROVIDER_ID: &str = "ai21";
/// AI21 nests its OpenAI-compat endpoint under `/studio/v1`, not `/v1`.
/// We bake `/studio` into the base URL so the same `streaming_url` helper
/// pattern as the other adapters works with one trailing-segment append.
pub const DEFAULT_BASE_URL: &str = "https://api.ai21.com/studio";
/// `jamba-1.5-large` is AI21's flagship 256k-context model. Listed first in
/// their model card 2026-05-16; smaller `jamba-1.5-mini` is also valid here.
pub const DEFAULT_MODEL: &str = "jamba-1.5-large";

pub fn streaming_url(provider: &ProviderEntry) -> String {
    let base = provider.url.as_deref().unwrap_or(DEFAULT_BASE_URL);
    let base = base.trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        return base.to_string();
    }
    if base.ends_with("/v1") {
        return format!("{}/chat/completions", base);
    }
    // Otherwise (bare host, custom path, or default base baked with /studio) —
    // append /v1/chat/completions.
    format!("{}/v1/chat/completions", base)
}

pub fn auth_header(
    api_key: &str,
) -> Result<(HeaderName, HeaderValue), reqwest::header::InvalidHeaderValue> {
    let value = HeaderValue::from_str(&format!("Bearer {}", api_key))?;
    Ok((AUTHORIZATION, value))
}

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
    display_name: "AI21 Labs",
    default_base_url: DEFAULT_BASE_URL,
    default_model: DEFAULT_MODEL,
    api_key_env: "AI21_API_KEY",
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
    fn streaming_url_default_uses_studio_path() {
        // Critical: AI21 nests under /studio/v1 — losing /studio would 404.
        assert_eq!(
            streaming_url(&make(None)),
            "https://api.ai21.com/studio/v1/chat/completions"
        );
    }

    #[test]
    fn streaming_url_with_explicit_v1() {
        assert_eq!(
            streaming_url(&make(Some("https://api.ai21.com/studio/v1"))),
            "https://api.ai21.com/studio/v1/chat/completions"
        );
    }

    #[test]
    fn streaming_url_full_path_passthrough() {
        assert_eq!(
            streaming_url(&make(Some(
                "https://api.ai21.com/studio/v1/chat/completions"
            ))),
            "https://api.ai21.com/studio/v1/chat/completions"
        );
    }

    #[test]
    fn auth_header_builds_bearer() {
        let (name, value) = auth_header("ai21-test-key").expect("valid");
        assert_eq!(name.as_str(), "authorization");
        assert_eq!(value.to_str().unwrap(), "Bearer ai21-test-key");
    }

    #[test]
    fn auth_header_rejects_newline_in_key() {
        assert!(auth_header("k\r\nbad").is_err(), "CRLF must be rejected");
    }

    #[test]
    fn info_contract() {
        assert_eq!(INFO.id, "ai21");
        assert_eq!(INFO.api_key_env, "AI21_API_KEY");
        assert!(
            INFO.default_base_url.contains("/studio"),
            "studio path must be baked into default base"
        );
        assert!(INFO.default_model.starts_with("jamba"));
    }

    #[test]
    fn retry_enabled_const_is_true() {
        assert!(RETRY_ENABLED, "T11: ai21 must opt in to retry middleware");
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
        assert!(health_check_with_retry(&provider, "k-test").await.is_ok());
    }
}
