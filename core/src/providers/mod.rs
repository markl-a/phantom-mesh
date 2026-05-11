pub mod claude_cli;
pub mod credential_scanner;
pub mod traits;

use serde::{Deserialize, Serialize};

use crate::config::ProviderEntry;

// ── Legacy discovery type ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredProviderInfo {
    pub name: String,
    pub provider_type: String,
    pub source: String,
}

// ── Model alias resolution ────────────────────────────────────────────────

/// Resolve a friendly model alias to the canonical model ID.
///
/// If `model` is not a recognized alias it is returned unchanged, so callers
/// can always pass this function's output directly to an API without any
/// conditional logic.
///
/// # Examples
///
/// ```
/// use phantom_mesh::providers::resolve_model_alias;
/// assert_eq!(resolve_model_alias("sonnet"), "claude-sonnet-4-5-20251022");
/// assert_eq!(resolve_model_alias("my-custom-model"), "my-custom-model");
/// ```
pub fn resolve_model_alias(model: &str) -> &str {
    match model {
        "sonnet"    => "claude-sonnet-4-5-20251022",
        "opus"      => "claude-opus-4-5",
        "haiku"     => "claude-haiku-4-5-20251001",
        "gpt4"      => "gpt-4o",
        "gpt4mini"  => "gpt-4o-mini",
        "gemini"    => "gemini-2.0-flash",
        other       => other,
    }
}

// ── Provider display name ─────────────────────────────────────────────────

/// Return a human-friendly display string for a provider.
///
/// The name is derived from `provider_type` and, where possible, the
/// canonical `base_url` so that the UI can show meaningful labels without
/// requiring a separate name field in the config.
///
/// # Examples
///
/// * `provider_type = "anthropic"` → `"Claude (Anthropic)"`
/// * `provider_type = "openai"`    → `"GPT-4o (OpenAI)"`
/// * `provider_type = "gemini"`    → `"Gemini (Google)"`
/// * `provider_type = "groq"`      → `"Groq"`
/// * `provider_type = "opencode"`  → `"OpenCode"`
/// * anything else                 → the raw `provider_type` string, title-cased
pub fn display_name(provider: &ProviderEntry) -> String {
    // Also inspect the base URL as a fallback when provider_type is generic.
    let url = provider.url.as_deref().unwrap_or("");

    match provider.provider_type.as_str() {
        "anthropic" => "Claude (Anthropic)".into(),
        "openai"    => "GPT-4o (OpenAI)".into(),
        "gemini"    => "Gemini (Google)".into(),
        "groq"      => "Groq".into(),
        "opencode"  => "OpenCode".into(),
        _ => {
            // Fall back to URL-based detection for unknown types.
            if url.contains("anthropic.com") {
                return "Claude (Anthropic)".into();
            }
            if url.contains("openai.com") {
                return "GPT-4o (OpenAI)".into();
            }
            if url.contains("googleapis.com") || url.contains("gemini") {
                return "Gemini (Google)".into();
            }
            if url.contains("groq.com") {
                return "Groq".into();
            }
            if url.contains("openrouter.ai") {
                return "OpenRouter".into();
            }
            // Generic fallback: title-case the provider_type string.
            let t = &provider.provider_type;
            if t.is_empty() {
                "Unknown Provider".into()
            } else {
                let mut chars = t.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            }
        }
    }
}

// ── Provider health check ─────────────────────────────────────────────────

/// Verify that a provider is reachable by sending a minimal API request.
///
/// For Anthropic providers a tiny `messages` request is made.  For all other
/// providers the OpenAI-compatible `/chat/completions` endpoint is used.
///
/// Returns `true` if the provider responds with HTTP 200 within 5 seconds.
/// Returns `false` on timeout, network error, or any non-200 status.
pub async fn health_check(provider: &ProviderEntry, client: &reqwest::Client) -> bool {
    use std::time::Duration;

    let api_key = provider.api_key.clone()
        .or_else(|| {
            provider.api_key_env.as_ref()
                .and_then(|env| std::env::var(env).ok())
        });

    let Some(key) = api_key.filter(|k| !k.is_empty()) else {
        tracing::debug!("health_check: no API key configured for provider");
        return false;
    };

    let is_anthropic = provider.provider_type == "anthropic"
        || provider.url.as_deref().unwrap_or("").contains("anthropic.com");

    let model = provider.default_model.as_deref()
        .unwrap_or(if is_anthropic { "claude-haiku-4-5-20251001" } else { "gpt-4o-mini" });

    let timeout = Duration::from_secs(5);

    if is_anthropic {
        let url = {
            let base = provider.url.as_deref().unwrap_or("https://api.anthropic.com");
            let base = base.trim_end_matches('/');
            // Normalise: strip any trailing path and append the messages endpoint.
            let base = base
                .trim_end_matches("/v1/messages")
                .trim_end_matches("/v1/chat/completions")
                .trim_end_matches('/');
            format!("{}/v1/messages", base)
        };

        let body = serde_json::json!({
            "model": model,
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "ping"}]
        });

        let result = tokio::time::timeout(timeout, async {
            client
                .post(&url)
                .header("x-api-key", &key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
        })
        .await;

        match result {
            Ok(Ok(resp)) => resp.status().is_success(),
            Ok(Err(e)) => {
                tracing::debug!("health_check Anthropic request error: {}", e);
                false
            }
            Err(_) => {
                tracing::debug!("health_check Anthropic timed out");
                false
            }
        }
    } else {
        // OpenAI-compatible endpoint.
        let url = {
            let base = provider.url.as_deref()
                .unwrap_or("https://api.openai.com");
            let base = base.trim_end_matches('/');
            if base.ends_with("/chat/completions") {
                base.to_string()
            } else if base.ends_with("/v1") {
                format!("{}/chat/completions", base)
            } else {
                format!("{}/v1/chat/completions", base)
            }
        };

        let body = serde_json::json!({
            "model": model,
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "ping"}]
        });

        let result = tokio::time::timeout(timeout, async {
            client
                .post(&url)
                .header("Authorization", format!("Bearer {}", key))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
        })
        .await;

        match result {
            Ok(Ok(resp)) => resp.status().is_success(),
            Ok(Err(e)) => {
                tracing::debug!("health_check OpenAI-compat request error: {}", e);
                false
            }
            Err(_) => {
                tracing::debug!("health_check OpenAI-compat timed out");
                false
            }
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── resolve_model_alias ───────────────────────────────────────────────

    #[test]
    fn alias_sonnet() {
        assert_eq!(resolve_model_alias("sonnet"), "claude-sonnet-4-5-20251022");
    }

    #[test]
    fn alias_opus() {
        assert_eq!(resolve_model_alias("opus"), "claude-opus-4-5");
    }

    #[test]
    fn alias_haiku() {
        assert_eq!(resolve_model_alias("haiku"), "claude-haiku-4-5-20251001");
    }

    #[test]
    fn alias_gpt4() {
        assert_eq!(resolve_model_alias("gpt4"), "gpt-4o");
    }

    #[test]
    fn alias_gpt4mini() {
        assert_eq!(resolve_model_alias("gpt4mini"), "gpt-4o-mini");
    }

    #[test]
    fn alias_gemini() {
        assert_eq!(resolve_model_alias("gemini"), "gemini-2.0-flash");
    }

    #[test]
    fn alias_passthrough() {
        assert_eq!(resolve_model_alias("my-custom-model-v1"), "my-custom-model-v1");
        assert_eq!(resolve_model_alias("gpt-4-turbo"), "gpt-4-turbo");
    }

    // ── display_name ──────────────────────────────────────────────────────

    fn make_provider(provider_type: &str, url: Option<&str>) -> ProviderEntry {
        ProviderEntry {
            provider_type: provider_type.into(),
            url: url.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn display_anthropic() {
        let p = make_provider("anthropic", None);
        assert_eq!(display_name(&p), "Claude (Anthropic)");
    }

    #[test]
    fn display_openai() {
        let p = make_provider("openai", None);
        assert_eq!(display_name(&p), "GPT-4o (OpenAI)");
    }

    #[test]
    fn display_gemini() {
        let p = make_provider("gemini", None);
        assert_eq!(display_name(&p), "Gemini (Google)");
    }

    #[test]
    fn display_groq() {
        let p = make_provider("groq", None);
        assert_eq!(display_name(&p), "Groq");
    }

    #[test]
    fn display_url_fallback_anthropic() {
        let p = make_provider("custom", Some("https://api.anthropic.com/v1/messages"));
        assert_eq!(display_name(&p), "Claude (Anthropic)");
    }

    #[test]
    fn display_url_fallback_openrouter() {
        let p = make_provider("custom", Some("https://openrouter.ai/api/v1"));
        assert_eq!(display_name(&p), "OpenRouter");
    }

    #[test]
    fn display_title_case_unknown() {
        let p = make_provider("mistral", None);
        assert_eq!(display_name(&p), "Mistral");
    }

    #[test]
    fn display_empty_type() {
        let p = make_provider("", None);
        assert_eq!(display_name(&p), "Unknown Provider");
    }

    // ── classify_error ────────────────────────────────────────────────────

    use crate::providers::traits::{classify_error, ProviderError};

    #[test]
    fn classify_rate_limit() {
        assert_eq!(classify_error(429, "Too Many Requests"), ProviderError::RateLimit);
    }

    #[test]
    fn classify_auth_401() {
        assert_eq!(classify_error(401, "Unauthorized"), ProviderError::AuthError);
    }

    #[test]
    fn classify_auth_403() {
        assert_eq!(classify_error(403, "Forbidden"), ProviderError::AuthError);
    }

    #[test]
    fn classify_model_not_found() {
        assert_eq!(
            classify_error(404, r#"{"error":{"message":"model not found"}}"#),
            ProviderError::ModelNotFound
        );
    }

    #[test]
    fn classify_generic_404() {
        assert!(matches!(classify_error(404, "page not found"), ProviderError::Unknown(_)));
    }

    #[test]
    fn classify_context_too_long() {
        assert_eq!(
            classify_error(400, "context length exceeded the limit"),
            ProviderError::ContextTooLong
        );
    }

    #[test]
    fn classify_network_error() {
        assert_eq!(classify_error(0, ""), ProviderError::NetworkError);
    }

    #[test]
    fn classify_unknown_500() {
        assert!(matches!(classify_error(500, "internal server error"), ProviderError::Unknown(_)));
    }
}
