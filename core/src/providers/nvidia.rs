//! NVIDIA NIM (Inference Microservices) provider adapter
//! (integrate.api.nvidia.com/v1).
//!
//! OpenAI-compatible chat completions. Auth: `Authorization: Bearer <key>`.
//! NVIDIA's hosted catalog of open-weight models (Llama, Mixtral, NeMo,
//! plus their own Nemotron tunes). Free developer accounts get 1000 credits
//! (~1000 short requests) per month (docs 2026-05-16).
//!
//! T51 (v0.6.0 V1) push: added alongside perplexity, ai21, cohere.
//!
//! Gated behind Cargo feature `experimental-hermes-providers` (default OFF).

#![cfg(feature = "experimental-hermes-providers")]

use crate::config::ProviderEntry;
use reqwest::header::{HeaderName, HeaderValue, AUTHORIZATION};

/// T11: opt into the retry middleware for this provider's HTTP calls.
pub const RETRY_ENABLED: bool = true;

pub const PROVIDER_ID: &str = "nvidia";
pub const DEFAULT_BASE_URL: &str = "https://integrate.api.nvidia.com";
/// `meta/llama-3.3-70b-instruct` is NVIDIA's Llama 3.3 hosting; the
/// `<vendor>/<model>` slash form is NVIDIA's catalog convention. Verified
/// in build.nvidia.com model browser 2026-05-16.
pub const DEFAULT_MODEL: &str = "meta/llama-3.3-70b-instruct";

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
    display_name: "NVIDIA NIM",
    default_base_url: DEFAULT_BASE_URL,
    default_model: DEFAULT_MODEL,
    // doctor in core/src/bin/phantom.rs already references this exact env name
    // ("NVIDIA_NIM_API_KEY", "NVIDIA NIM", "nvidia") — keep them in sync if
    // either side moves.
    api_key_env: "NVIDIA_NIM_API_KEY",
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
            "https://integrate.api.nvidia.com/v1/chat/completions"
        );
    }

    #[test]
    fn streaming_url_with_explicit_v1() {
        assert_eq!(
            streaming_url(&make(Some("https://integrate.api.nvidia.com/v1"))),
            "https://integrate.api.nvidia.com/v1/chat/completions"
        );
    }

    #[test]
    fn streaming_url_full_path_passthrough() {
        assert_eq!(
            streaming_url(&make(Some(
                "https://integrate.api.nvidia.com/v1/chat/completions"
            ))),
            "https://integrate.api.nvidia.com/v1/chat/completions"
        );
    }

    #[test]
    fn auth_header_builds_bearer() {
        let (name, value) = auth_header("nvapi-abcXYZ").expect("valid");
        assert_eq!(name.as_str(), "authorization");
        assert_eq!(value.to_str().unwrap(), "Bearer nvapi-abcXYZ");
    }

    #[test]
    fn auth_header_rejects_newline_in_key() {
        assert!(
            auth_header("nvapi\r\nbad").is_err(),
            "CRLF must be rejected"
        );
    }

    #[test]
    fn info_contract() {
        assert_eq!(INFO.id, "nvidia");
        assert_eq!(INFO.api_key_env, "NVIDIA_NIM_API_KEY");
        // NVIDIA model names use the `vendor/model` slash form; a regression
        // would silently switch to a HuggingFace-style id and 404 in catalog.
        assert!(INFO.default_model.contains('/'));
    }

    #[test]
    fn retry_enabled_const_is_true() {
        assert!(RETRY_ENABLED, "T11: nvidia must opt in to retry middleware");
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
