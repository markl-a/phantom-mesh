//! Perplexity provider adapter (api.perplexity.ai).
//!
//! OpenAI-compatible chat completions. Auth: `Authorization: Bearer <key>`.
//! Specialises in web-grounded answers via the `sonar` family. New accounts
//! get $5 of free monthly credit (verified live 2026-05-16 — pplx-api docs).
//!
//! T51 (v0.6.0 V1) push: added alongside ai21, nvidia, cohere to bring the
//! Hermes provider count from 8 → 12. Wire format is the existing OpenAI
//! streaming codepath in `streaming.rs`; this module owns only metadata,
//! default URL/model, and bearer-auth header construction.
//!
//! Gated behind Cargo feature `experimental-hermes-providers` (default OFF).

#![cfg(feature = "experimental-hermes-providers")]

use crate::config::ProviderEntry;
use reqwest::header::{HeaderName, HeaderValue, AUTHORIZATION};

/// T11: opt into the retry middleware for this provider's HTTP calls.
pub const RETRY_ENABLED: bool = true;

pub const PROVIDER_ID: &str = "perplexity";
pub const DEFAULT_BASE_URL: &str = "https://api.perplexity.ai";
/// `sonar` is Perplexity's general-purpose grounded model and the cheapest
/// entry-point on the free tier (verified docs 2026-05-16).
pub const DEFAULT_MODEL: &str = "sonar";

/// Build the chat-completions endpoint URL.
///
/// Note: Perplexity does NOT use the `/v1/` segment — its endpoint lives at
/// the bare `/chat/completions` path. We honour an explicit override that
/// already includes any path; otherwise we append `/chat/completions`.
pub fn streaming_url(provider: &ProviderEntry) -> String {
    let base = provider.url.as_deref().unwrap_or(DEFAULT_BASE_URL);
    let base = base.trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        return base.to_string();
    }
    if base.ends_with("/v1") {
        // Operator pointed us at a v1-prefixed proxy; respect it.
        return format!("{}/chat/completions", base);
    }
    format!("{}/chat/completions", base)
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
    display_name: "Perplexity",
    default_base_url: DEFAULT_BASE_URL,
    default_model: DEFAULT_MODEL,
    api_key_env: "PERPLEXITY_API_KEY",
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
    fn streaming_url_default_has_no_v1_segment() {
        // Critical: Perplexity uses bare /chat/completions, not /v1/chat/completions.
        // A regression that adds /v1/ would 404 on the real API.
        assert_eq!(
            streaming_url(&make(None)),
            "https://api.perplexity.ai/chat/completions"
        );
    }

    #[test]
    fn streaming_url_full_path_passthrough() {
        assert_eq!(
            streaming_url(&make(Some("https://api.perplexity.ai/chat/completions"))),
            "https://api.perplexity.ai/chat/completions"
        );
    }

    #[test]
    fn streaming_url_strips_trailing_slash() {
        assert_eq!(
            streaming_url(&make(Some("https://api.perplexity.ai/"))),
            "https://api.perplexity.ai/chat/completions"
        );
    }

    #[test]
    fn streaming_url_with_v1_proxy_override() {
        // If an operator points us at a v1-prefixed proxy, honour it.
        assert_eq!(
            streaming_url(&make(Some("https://proxy.example.com/v1"))),
            "https://proxy.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn auth_header_builds_bearer() {
        let (name, value) = auth_header("pplx-abc123").expect("valid");
        assert_eq!(name.as_str(), "authorization");
        assert_eq!(value.to_str().unwrap(), "Bearer pplx-abc123");
    }

    #[test]
    fn auth_header_rejects_newline_in_key() {
        let result = auth_header("pplx-bad\r\nInjected: yes");
        assert!(
            result.is_err(),
            "expected InvalidHeaderValue for CRLF in key"
        );
    }

    #[test]
    fn info_contract() {
        assert_eq!(INFO.id, "perplexity");
        assert_eq!(INFO.api_key_env, "PERPLEXITY_API_KEY");
        assert_eq!(INFO.default_base_url, "https://api.perplexity.ai");
        assert_eq!(INFO.default_model, "sonar");
    }

    #[test]
    fn retry_enabled_const_is_true() {
        assert!(
            RETRY_ENABLED,
            "T11: perplexity must opt in to retry middleware"
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
        assert!(health_check_with_retry(&provider, "k-test").await.is_ok());
    }
}
