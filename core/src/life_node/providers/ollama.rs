//! Local Ollama text-only fallback for life_node coach tasks.
//!
//! This is the FINAL link in the tomorrow-action fallback chain (after
//! Gemini → Groq): a locally-hosted model that costs nothing and is not
//! subject to a free-tier 429, so the coach's "Tomorrow's one action" still
//! lands when the hosted providers are rate-limited. Text-only (no
//! image/audio/video) — it is only used for the text brief → action pass.
//!
//! Endpoint: Ollama's OpenAI-compatible chat completions
//! (`/v1/chat/completions`), so the request/response shape mirrors `groq.rs`
//! and the same error mapping applies. A local Ollama needs no auth; a
//! reverse-proxied / remote one may, so `OLLAMA_API_KEY` (when set) is sent as
//! a bearer token. `OLLAMA_HOST` overrides the base URL (default
//! `http://localhost:11434`).
//!
//! Error mapping mirrors the other providers so the chain's typed failover
//! works identically: 429 → RateLimit, transport → Network, other non-2xx →
//! Provider, bad JSON → Parse.

use crate::life_node::multimodal::{
    AnalysisInput, AnalysisResult, Modality, MultimodalProvider, ProviderCapabilities,
    ProviderError, ResponseFormat,
};
use serde_json::{json, Value};

const DEFAULT_MODEL: &str = "llama3.2";
const DEFAULT_HOST: &str = "http://localhost:11434";

pub struct OllamaTextProvider {
    /// Optional bearer token. Empty for a bare local Ollama (no auth).
    api_key: Option<String>,
    model: String,
    base: String, // host root, e.g. http://localhost:11434 — overridable for tests
    http: reqwest::Client,
}

impl OllamaTextProvider {
    /// Construct directly. `model` defaults to `llama3.2`; `base` defaults to
    /// `http://localhost:11434`. Use `from_env_or_default` for production.
    pub fn new(
        api_key: Option<String>,
        model: Option<String>,
        base: Option<String>,
    ) -> Self {
        Self {
            api_key: api_key.filter(|k| !k.is_empty()),
            model: model.unwrap_or_else(|| DEFAULT_MODEL.into()),
            base: base.unwrap_or_else(|| DEFAULT_HOST.into()),
            http: reqwest::Client::new(),
        }
    }

    /// Build a local-Ollama fallback. Unlike Gemini/Groq this does NOT require
    /// an API key (a bare local Ollama is unauthenticated), so it is enabled by
    /// default and only skipped when explicitly disabled via
    /// `OLLAMA_DISABLE=1`. Honours `OLLAMA_API_KEY` (bearer, optional),
    /// `OLLAMA_HOST` (base URL), and `OLLAMA_MODEL` (model id) env overrides.
    ///
    /// Returns `ProviderError::Auth` only when explicitly disabled, so the
    /// chain-construction `if let Ok(..)` cleanly skips it — keeping the
    /// graceful-degradation contract: an unreachable localhost simply surfaces
    /// as a per-call Network/Provider error that the chain falls through.
    pub fn from_env_or_default() -> Result<Self, ProviderError> {
        if std::env::var("OLLAMA_DISABLE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
        {
            return Err(ProviderError::Auth("OLLAMA_DISABLE set".into()));
        }
        let api_key = std::env::var("OLLAMA_API_KEY").ok();
        let model = std::env::var("OLLAMA_MODEL").ok();
        let base = std::env::var("OLLAMA_HOST").ok();
        Ok(Self::new(api_key, model, base))
    }

    /// Override the base URL — for tests against wiremock.
    #[cfg(test)]
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = base.into();
        self
    }
}

#[async_trait::async_trait]
impl MultimodalProvider for OllamaTextProvider {
    async fn analyze(&self, input: AnalysisInput) -> Result<AnalysisResult, ProviderError> {
        // Text-only: collapse any text modalities (non-text are ignored — this
        // provider only serves the coach brief → action pass).
        let text: String = input
            .modalities
            .iter()
            .filter_map(|m| {
                if let Modality::Text(t) = m {
                    Some(t.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let system = input.system_prompt.unwrap_or_else(|| {
            "You are a helpful life coach. Be specific, positive, and shame-free.".into()
        });

        let schema_hint = match &input.response_format {
            ResponseFormat::Json => {
                " Respond with valid JSON only: {\"summary\": \"...\", \"goal_impact\": \"...\", \"suggestion\": \"...\", \"confidence\": 0.8}"
            }
            ResponseFormat::PlainText | ResponseFormat::Markdown => "",
        };

        let user_msg = format!("{}{}\n\n{}", input.user_prompt, schema_hint, text);

        let body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user",   "content": user_msg}
            ],
            "max_tokens": input.max_output_tokens.unwrap_or(512),
            "temperature": 0.3,
            "stream": false
        });

        let url = format!("{}/v1/chat/completions", self.base.trim_end_matches('/'));
        let t0 = std::time::Instant::now();
        let mut req = self.http.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await?;

        let status = resp.status();
        if status.as_u16() == 429 {
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(|s| s * 1000);
            return Err(ProviderError::RateLimit {
                retry_after_ms: retry_after,
            });
        }
        if status.as_u16() == 401 || status.as_u16() == 403 {
            let msg = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Auth(format!(
                "HTTP {}: {}",
                status,
                &msg[..msg.len().min(200)]
            )));
        }
        if !status.is_success() {
            let msg = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Provider(format!(
                "HTTP {}: {}",
                status,
                &msg[..msg.len().min(200)]
            )));
        }

        let latency_ms = t0.elapsed().as_millis() as u64;
        let raw: Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Parse(e.to_string()))?;
        let content = raw["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();

        let summary = if matches!(input.response_format, ResponseFormat::Json) {
            serde_json::from_str::<Value>(&content)
                .ok()
                .and_then(|v| v["summary"].as_str().map(String::from))
                .unwrap_or_else(|| content.clone())
        } else {
            content.clone()
        };

        Ok(AnalysisResult {
            summary,
            goal_impact: serde_json::from_str::<Value>(&content)
                .ok()
                .and_then(|v| v["goal_impact"].as_str().map(String::from)),
            suggestion: serde_json::from_str::<Value>(&content)
                .ok()
                .and_then(|v| v["suggestion"].as_str().map(String::from)),
            confidence: serde_json::from_str::<Value>(&content)
                .ok()
                .and_then(|v| v["confidence"].as_f64().map(|f| f as f32)),
            raw_response: raw,
            model_id: self.model.clone(),
            latency_ms,
            cost_usd: Some(0.0), // local model — free
        })
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_image: false,
            supports_audio: false,
            supports_video: false,
            max_image_count: 0,
            max_audio_secs: 0,
            max_total_bytes: 0,
        }
    }

    fn model_id(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn text_input(prompt: &str) -> AnalysisInput {
        AnalysisInput {
            modalities: vec![Modality::Text(prompt.into())],
            system_prompt: None,
            user_prompt: "x".into(),
            max_output_tokens: Some(64),
            response_format: ResponseFormat::PlainText,
            response_schema: None,
        }
    }

    #[tokio::test]
    async fn analyze_returns_summary_from_mock_ollama() {
        let mock = MockServer::start().await;
        let body = json!({
            "choices": [{
                "message": {"role": "assistant", "content": "walk 10 min tomorrow"}
            }]
        });
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&mock)
            .await;

        let p = OllamaTextProvider::new(None, None, None).with_base(mock.uri());
        let r = p.analyze(text_input("brief")).await.expect("mock 200");
        assert_eq!(r.summary, "walk 10 min tomorrow");
        assert_eq!(r.model_id, DEFAULT_MODEL);
    }

    #[tokio::test]
    async fn rate_limit_429_maps_to_rate_limit_error() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&mock)
            .await;

        let p = OllamaTextProvider::new(None, None, None).with_base(mock.uri());
        let r = p.analyze(text_input("brief")).await;
        assert!(
            matches!(r, Err(ProviderError::RateLimit { .. })),
            "expected RateLimit, got {:?}",
            r.map_err(|e| e.to_string())
        );
    }

    #[tokio::test]
    async fn server_error_maps_to_provider_error() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&mock)
            .await;

        let p = OllamaTextProvider::new(None, None, None).with_base(mock.uri());
        let r = p.analyze(text_input("brief")).await;
        assert!(
            matches!(r, Err(ProviderError::Provider(_))),
            "expected Provider, got {:?}",
            r.map_err(|e| e.to_string())
        );
    }

    #[tokio::test]
    async fn unreachable_host_maps_to_network_error() {
        // No server: connection to 127.0.0.1:1 fails at transport → Network,
        // which the chain falls through gracefully.
        let p = OllamaTextProvider::new(None, None, None).with_base("http://127.0.0.1:1");
        let r = p.analyze(text_input("brief")).await;
        assert!(
            matches!(r, Err(ProviderError::Network(_))),
            "expected Network, got {:?}",
            r.map_err(|e| e.to_string())
        );
    }

    #[test]
    fn from_env_or_default_is_enabled_without_key() {
        // All four OLLAMA_* vars are process-global; serialize on the env mutex
        // and clear them so the defaults are observed regardless of the host's
        // real Ollama config (this dev box actually exports OLLAMA_HOST).
        let _g = crate::env_lock::acquire();
        let saved_disable = std::env::var("OLLAMA_DISABLE").ok();
        let saved_key = std::env::var("OLLAMA_API_KEY").ok();
        let saved_host = std::env::var("OLLAMA_HOST").ok();
        let saved_model = std::env::var("OLLAMA_MODEL").ok();
        std::env::remove_var("OLLAMA_DISABLE");
        std::env::remove_var("OLLAMA_API_KEY");
        std::env::remove_var("OLLAMA_HOST");
        std::env::remove_var("OLLAMA_MODEL");

        let r = OllamaTextProvider::from_env_or_default();
        assert!(r.is_ok(), "local Ollama enabled by default (no key needed)");
        let p = r.unwrap();
        assert!(p.api_key.is_none(), "no bearer when OLLAMA_API_KEY unset");
        assert_eq!(p.base, DEFAULT_HOST);
        assert_eq!(p.model, DEFAULT_MODEL);

        let restore = |k: &str, v: Option<String>| match v {
            Some(v) => std::env::set_var(k, v),
            None => std::env::remove_var(k),
        };
        restore("OLLAMA_DISABLE", saved_disable);
        restore("OLLAMA_API_KEY", saved_key);
        restore("OLLAMA_HOST", saved_host);
        restore("OLLAMA_MODEL", saved_model);
    }

    #[test]
    fn from_env_or_default_reads_host_and_model_overrides() {
        let _g = crate::env_lock::acquire();
        let saved_disable = std::env::var("OLLAMA_DISABLE").ok();
        let saved_host = std::env::var("OLLAMA_HOST").ok();
        let saved_model = std::env::var("OLLAMA_MODEL").ok();
        std::env::remove_var("OLLAMA_DISABLE");
        std::env::set_var("OLLAMA_HOST", "http://10.0.0.5:11434");
        std::env::set_var("OLLAMA_MODEL", "qwen2.5");

        let p = OllamaTextProvider::from_env_or_default().expect("enabled");
        assert_eq!(p.base, "http://10.0.0.5:11434");
        assert_eq!(p.model, "qwen2.5");

        let restore = |k: &str, v: Option<String>| match v {
            Some(v) => std::env::set_var(k, v),
            None => std::env::remove_var(k),
        };
        restore("OLLAMA_DISABLE", saved_disable);
        restore("OLLAMA_HOST", saved_host);
        restore("OLLAMA_MODEL", saved_model);
    }

    #[test]
    fn from_env_or_default_respects_disable_flag() {
        let _g = crate::env_lock::acquire();
        let saved = std::env::var("OLLAMA_DISABLE").ok();
        std::env::set_var("OLLAMA_DISABLE", "1");

        let r = OllamaTextProvider::from_env_or_default();
        assert!(
            matches!(r, Err(ProviderError::Auth(_))),
            "OLLAMA_DISABLE=1 → skip provider"
        );

        match saved {
            Some(v) => std::env::set_var("OLLAMA_DISABLE", v),
            None => std::env::remove_var("OLLAMA_DISABLE"),
        }
    }
}
