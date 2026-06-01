//! Together AI provider adapter (api.together.xyz/v1).
//!
//! OpenAI-compatible chat completions. Auth: `Authorization: Bearer <key>`.
//! Hosts open-weight models (Llama, Mixtral, Qwen, etc.). $25 free credit on signup.
//!
//! Gated behind Cargo feature `experimental-hermes-providers` (default OFF).

#![cfg(feature = "experimental-hermes-providers")]

use crate::config::ProviderEntry;
use reqwest::header::{HeaderName, HeaderValue, AUTHORIZATION};

/// T11: opt into the retry middleware for this provider's HTTP calls.
pub const RETRY_ENABLED: bool = true;

pub const PROVIDER_ID: &str = "together";
pub const DEFAULT_BASE_URL: &str = "https://api.together.xyz";
pub const DEFAULT_MODEL: &str = "meta-llama/Llama-3.3-70B-Instruct-Turbo";

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
    display_name: "Together AI",
    default_base_url: DEFAULT_BASE_URL,
    default_model: DEFAULT_MODEL,
    api_key_env: "TOGETHER_API_KEY",
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
            "https://api.together.xyz/v1/chat/completions"
        );
    }

    #[test]
    fn streaming_url_with_explicit_full_path() {
        assert_eq!(
            streaming_url(&make(Some("https://api.together.xyz/v1/chat/completions"))),
            "https://api.together.xyz/v1/chat/completions"
        );
    }

    #[test]
    fn auth_header_builds_bearer() {
        let (_, value) = auth_header("together-key-xyz").expect("valid");
        assert_eq!(value.to_str().unwrap(), "Bearer together-key-xyz");
    }

    #[test]
    fn info_contract() {
        assert_eq!(INFO.id, "together");
        assert_eq!(INFO.api_key_env, "TOGETHER_API_KEY");
        // Together's flagship model name uses a vendor/model slash form — make
        // sure we keep that intact (regression: someone might title-case it).
        assert!(INFO.default_model.contains('/'));
    }

    #[test]
    fn retry_enabled_const_is_true() {
        assert!(
            RETRY_ENABLED,
            "T11: together must opt in to retry middleware"
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
