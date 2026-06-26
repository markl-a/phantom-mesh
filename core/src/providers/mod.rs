pub mod claude_agent;
pub mod claude_cli;
pub mod cli_session_provider;
pub mod codex_cli;
pub mod codex_oauth;
pub mod credential_scanner;
pub mod gemini_cli;
pub mod gemini_oauth;
pub mod local_servers;
pub mod openai_oauth;
// Free-tier cloud LLM plugin — default-on in onboarding so a brand-new user
// with no subscription + no local Ollama still gets a working (free, no-credit-
// card) provider minute-one. See `free_plugin.rs`.
pub mod free_plugin;
pub mod traits;

// ── P0-5 (2026-06-17): deterministic circuit breaker + failover decision.
// State machine (Closed→Open→HalfOpen) over the existing ProviderError
// catalog, driven by an injected crate::clock::Clock so cooldown/half-open
// transitions are unit-testable without real time. Rewires the breaker
// functions in providers_wire.rs (provider_alive / record_provider_*).
pub mod circuit_breaker;
pub use circuit_breaker::{
    classify_failure, BreakerConfig, BreakerState, CircuitBreaker, FailureKind,
};

// ── DEMO-1 gap 1 (2026-05-17): LlmProvider trait + DefaultProviderResolver.
// Phase 1 introduces the trait surface (no call-site changes); Phase 2 adds
// the resolver + 4 passthrough impls. See
// `docs/superpowers/specs/2026-05-17-demo1-gap1-llmprovider-design.md`.
pub mod llm_provider;
pub mod resolver;
pub use llm_provider::{BuildRequestOpts, BuildRequestParts, LlmProvider};
pub use resolver::DefaultProviderResolver;

// ── H4 weekend push (2026-05-15): experimental new provider adapters ─────
// All four are OpenAI-compat chat-completions; modules own only metadata,
// default URLs/models, and bearer-auth header construction. Wire format
// is the existing OpenAI streaming codepath in `streaming.rs`.
#[cfg(feature = "experimental-extra-providers")]
pub mod fireworks;
#[cfg(feature = "experimental-extra-providers")]
pub mod mistral;
#[cfg(feature = "experimental-extra-providers")]
pub mod together;
#[cfg(feature = "experimental-extra-providers")]
pub mod xai;

// ── T51 (2026-05-16): v0.6.0 V1 push — 4 more provider adapters ──────────
// Bring the extra OpenAI-compat provider count from 8 → 12. Same feature gate so
// default `cargo build` stays byte-identical. Three are OpenAI-compat;
// `cohere` is the lone outlier (own request shape + X-API-Key header).
#[cfg(feature = "experimental-extra-providers")]
pub mod ai21;
#[cfg(feature = "experimental-extra-providers")]
pub mod cohere;
#[cfg(feature = "experimental-extra-providers")]
pub mod nvidia;
#[cfg(feature = "experimental-extra-providers")]
pub mod perplexity;

// T11 (2026-05-15): retry + backoff middleware for the H4 providers.
// Same feature gate so default `cargo build` stays byte-identical.
#[cfg(feature = "experimental-extra-providers")]
pub mod retry;

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
        "sonnet" => "claude-sonnet-4-5-20251022",
        "opus" => "claude-opus-4-5",
        "haiku" => "claude-haiku-4-5-20251001",
        "gpt4" => "gpt-4o",
        "gpt4mini" => "gpt-4o-mini",
        "gemini" => "gemini-2.0-flash",
        other => other,
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
        "openai" => "GPT-4o (OpenAI)".into(),
        "gemini" => "Gemini (Google)".into(),
        "groq" => "Groq".into(),
        "opencode" => "OpenCode".into(),
        // ── H4 weekend push (2026-05-15): friendly labels for new adapters.
        // These match arms are unconditional (no feature gate) — they're cheap
        // string literals and harmless when the modules aren't compiled in.
        "mistral" => "Mistral AI".into(),
        "xai" => "xAI Grok".into(),
        "together" => "Together AI".into(),
        "fireworks" => "Fireworks AI".into(),
        // ── T51 (2026-05-16): v0.6.0 V1 — 4 more friendly labels ─────────
        // Same shape as the H4 push above: cheap string literals, no feature
        // gate, harmless when the modules aren't compiled in.
        "perplexity" => "Perplexity".into(),
        "ai21" => "AI21 Labs".into(),
        "nvidia" => "NVIDIA NIM".into(),
        "cohere" => "Cohere".into(),
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
            if url.contains("mistral.ai") {
                return "Mistral AI".into();
            }
            if url.contains("api.x.ai") {
                return "xAI Grok".into();
            }
            if url.contains("together.xyz") {
                return "Together AI".into();
            }
            if url.contains("fireworks.ai") {
                return "Fireworks AI".into();
            }
            if url.contains("groq.com") {
                return "Groq".into();
            }
            if url.contains("openrouter.ai") {
                return "OpenRouter".into();
            }
            // ── T51 (2026-05-16): URL-fallback labels for the v0.6.0 V1
            // additions. Match on canonical hostnames documented at the
            // provider; subdomains like `integrate.api.nvidia.com` still
            // contain the substring `nvidia.com`, so the same `.contains`
            // check works for the integrate-prefixed NVIDIA endpoint.
            if url.contains("perplexity.ai") {
                return "Perplexity".into();
            }
            if url.contains("ai21.com") {
                return "AI21 Labs".into();
            }
            if url.contains("nvidia.com") {
                return "NVIDIA NIM".into();
            }
            if url.contains("cohere.com") {
                return "Cohere".into();
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

    let api_key = provider.api_key.clone().or_else(|| {
        provider
            .api_key_env
            .as_ref()
            .and_then(|env| std::env::var(env).ok())
    });

    let Some(key) = api_key.filter(|k| !k.is_empty()) else {
        tracing::debug!("health_check: no API key configured for provider");
        return false;
    };

    // T11 (2026-05-15): per-provider retry-enabled dispatch. Each H4 module
    // owns its own RETRY_ENABLED const + health_check_with_retry helper so
    // we don't centralise routing logic — adding a 5th retry-enabled
    // provider is one match-arm + that provider's own commit.
    #[cfg(feature = "experimental-extra-providers")]
    {
        if mistral::RETRY_ENABLED && provider.provider_type == mistral::PROVIDER_ID {
            return mistral::health_check_with_retry(provider, &key)
                .await
                .is_ok();
        }
        if xai::RETRY_ENABLED && provider.provider_type == xai::PROVIDER_ID {
            return xai::health_check_with_retry(provider, &key).await.is_ok();
        }
        if together::RETRY_ENABLED && provider.provider_type == together::PROVIDER_ID {
            return together::health_check_with_retry(provider, &key)
                .await
                .is_ok();
        }
        if fireworks::RETRY_ENABLED && provider.provider_type == fireworks::PROVIDER_ID {
            return fireworks::health_check_with_retry(provider, &key)
                .await
                .is_ok();
        }
        // ── T51 (2026-05-16): v0.6.0 V1 dispatch ─────────────────────────
        if perplexity::RETRY_ENABLED && provider.provider_type == perplexity::PROVIDER_ID {
            return perplexity::health_check_with_retry(provider, &key)
                .await
                .is_ok();
        }
        if ai21::RETRY_ENABLED && provider.provider_type == ai21::PROVIDER_ID {
            return ai21::health_check_with_retry(provider, &key).await.is_ok();
        }
        if nvidia::RETRY_ENABLED && provider.provider_type == nvidia::PROVIDER_ID {
            return nvidia::health_check_with_retry(provider, &key)
                .await
                .is_ok();
        }
        if cohere::RETRY_ENABLED && provider.provider_type == cohere::PROVIDER_ID {
            return cohere::health_check_with_retry(provider, &key)
                .await
                .is_ok();
        }
    }

    let is_anthropic = provider.provider_type == "anthropic"
        || provider
            .url
            .as_deref()
            .unwrap_or("")
            .contains("anthropic.com");

    let model = provider
        .default_model
        .as_deref()
        .unwrap_or(if is_anthropic {
            "claude-haiku-4-5-20251001"
        } else {
            "gpt-4o-mini"
        });

    let timeout = Duration::from_secs(5);

    if is_anthropic {
        let url = {
            let base = provider
                .url
                .as_deref()
                .unwrap_or("https://api.anthropic.com");
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
            let base = provider.url.as_deref().unwrap_or("https://api.openai.com");
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
        assert_eq!(
            resolve_model_alias("my-custom-model-v1"),
            "my-custom-model-v1"
        );
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
        // H4 (2026-05-15): "mistral" is now a known provider returning
        // "Mistral AI"; use a different unknown string to test the
        // title-case fallback path.
        let p = make_provider("anthropicxyz", None);
        assert_eq!(display_name(&p), "Anthropicxyz");
    }

    #[test]
    fn display_empty_type() {
        let p = make_provider("", None);
        assert_eq!(display_name(&p), "Unknown Provider");
    }

    // ── H4 weekend push: display_name for the 4 new providers ────────────

    #[test]
    fn display_mistral() {
        let p = make_provider("mistral", None);
        assert_eq!(display_name(&p), "Mistral AI");
    }

    #[test]
    fn display_xai() {
        let p = make_provider("xai", None);
        assert_eq!(display_name(&p), "xAI Grok");
    }

    #[test]
    fn display_together() {
        let p = make_provider("together", None);
        assert_eq!(display_name(&p), "Together AI");
    }

    #[test]
    fn display_fireworks() {
        let p = make_provider("fireworks", None);
        assert_eq!(display_name(&p), "Fireworks AI");
    }

    #[test]
    fn display_url_fallback_mistral() {
        let p = make_provider("custom", Some("https://api.mistral.ai/v1/chat/completions"));
        assert_eq!(display_name(&p), "Mistral AI");
    }

    #[test]
    fn display_url_fallback_xai() {
        let p = make_provider("custom", Some("https://api.x.ai/v1/chat/completions"));
        assert_eq!(display_name(&p), "xAI Grok");
    }

    #[test]
    fn display_url_fallback_together() {
        let p = make_provider(
            "custom",
            Some("https://api.together.xyz/v1/chat/completions"),
        );
        assert_eq!(display_name(&p), "Together AI");
    }

    #[test]
    fn display_url_fallback_fireworks() {
        let p = make_provider(
            "custom",
            Some("https://api.fireworks.ai/inference/v1/chat/completions"),
        );
        assert_eq!(display_name(&p), "Fireworks AI");
    }

    // ── T51 (2026-05-16): display_name for the 4 new providers ───────────

    #[test]
    fn display_perplexity() {
        let p = make_provider("perplexity", None);
        assert_eq!(display_name(&p), "Perplexity");
    }

    #[test]
    fn display_ai21() {
        let p = make_provider("ai21", None);
        assert_eq!(display_name(&p), "AI21 Labs");
    }

    #[test]
    fn display_nvidia() {
        let p = make_provider("nvidia", None);
        assert_eq!(display_name(&p), "NVIDIA NIM");
    }

    #[test]
    fn display_cohere() {
        let p = make_provider("cohere", None);
        assert_eq!(display_name(&p), "Cohere");
    }

    #[test]
    fn display_url_fallback_perplexity() {
        let p = make_provider("custom", Some("https://api.perplexity.ai/chat/completions"));
        assert_eq!(display_name(&p), "Perplexity");
    }

    #[test]
    fn display_url_fallback_ai21() {
        let p = make_provider(
            "custom",
            Some("https://api.ai21.com/studio/v1/chat/completions"),
        );
        assert_eq!(display_name(&p), "AI21 Labs");
    }

    #[test]
    fn display_url_fallback_nvidia() {
        // NVIDIA's hosted endpoint lives under integrate.api.nvidia.com — the
        // substring `nvidia.com` still matches, which is what we want.
        let p = make_provider(
            "custom",
            Some("https://integrate.api.nvidia.com/v1/chat/completions"),
        );
        assert_eq!(display_name(&p), "NVIDIA NIM");
    }

    #[test]
    fn display_url_fallback_cohere() {
        let p = make_provider("custom", Some("https://api.cohere.com/v1/chat"));
        assert_eq!(display_name(&p), "Cohere");
    }

    /// V1 P0 — pins the public list of 12 first-party provider types
    /// that the dispatcher must understand at startup. Asserts each
    /// type resolves to its branded label (NOT the type-name title-case
    /// fallback that the catch-all arm emits). A regression that
    /// dropped one of the H4 (Mistral/xAI/Together/Fireworks) or T51
    /// (Perplexity/AI21/NVIDIA/Cohere) labels would fail this test
    /// before the silent rename hit prod.
    #[test]
    fn all_12_providers_register_at_startup() {
        let canonical: &[(&str, &str)] = &[
            ("anthropic", "Claude (Anthropic)"),
            ("openai", "GPT-4o (OpenAI)"),
            ("gemini", "Gemini (Google)"),
            ("groq", "Groq"),
            ("mistral", "Mistral AI"),
            ("xai", "xAI Grok"),
            ("together", "Together AI"),
            ("fireworks", "Fireworks AI"),
            ("perplexity", "Perplexity"),
            ("ai21", "AI21 Labs"),
            ("nvidia", "NVIDIA NIM"),
            ("cohere", "Cohere"),
        ];
        assert_eq!(
            canonical.len(),
            12,
            "V1 commits to exactly 12 first-party providers",
        );

        for (ty, want_label) in canonical {
            let p = make_provider(ty, None);
            assert_eq!(
                display_name(&p),
                *want_label,
                "provider {ty} must register with its branded label, not a title-case fallback",
            );
        }
    }

    // ── classify_error ────────────────────────────────────────────────────

    use crate::providers::traits::{classify_error, ProviderError};

    #[test]
    fn classify_rate_limit() {
        assert_eq!(
            classify_error(429, "Too Many Requests"),
            ProviderError::RateLimit
        );
    }

    #[test]
    fn classify_auth_401() {
        assert_eq!(
            classify_error(401, "Unauthorized"),
            ProviderError::AuthError
        );
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
        assert!(matches!(
            classify_error(404, "page not found"),
            ProviderError::Unknown(_)
        ));
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
        assert!(matches!(
            classify_error(500, "internal server error"),
            ProviderError::Unknown(_)
        ));
    }

    #[cfg(feature = "experimental-extra-providers")]
    #[tokio::test]
    async fn health_check_dispatches_to_mistral_retry() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // First call: 429. Second call: 200. If retry is wired, this returns true.
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

        let mut provider = ProviderEntry::default();
        provider.provider_type = "mistral".into();
        provider.url = Some(mock.uri());
        provider.api_key = Some("k-test".into());

        let client = reqwest::Client::new();
        assert!(
            health_check(&provider, &client).await,
            "health_check should retry through mistral::health_check_with_retry"
        );
    }
}
