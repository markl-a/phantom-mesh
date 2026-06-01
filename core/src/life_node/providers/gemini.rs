//! Gemini 2.5 Flash impl of `MultimodalProvider`. Native multimodal:
//! image + audio + text in a single `generateContent` call.
//!
//! API ref: https://ai.google.dev/api/generate-content

use crate::life_node::multimodal::{
    AnalysisInput, AnalysisResult, Modality, MultimodalProvider, ProviderCapabilities,
    ProviderError, ResponseFormat,
};

const DEFAULT_MODEL: &str = "gemini-2.5-flash";
const API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";

pub struct GeminiMultimodalProvider {
    api_key: String,
    model: String,
    base: String, // overridable for tests (wiremock)
    http: reqwest::Client,
}

impl GeminiMultimodalProvider {
    /// Construct directly from an API key. `model` defaults to
    /// `gemini-2.5-flash`. Use `from_env` for production.
    pub fn new(api_key: impl Into<String>, model: Option<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.unwrap_or_else(|| DEFAULT_MODEL.into()),
            base: API_BASE.into(),
            http: reqwest::Client::new(),
        }
    }

    /// Load from `GEMINI_API_KEY` env var. Returns `ProviderError::Auth`
    /// if unset or empty.
    pub fn from_env() -> Result<Self, ProviderError> {
        let key = std::env::var("GEMINI_API_KEY")
            .map_err(|_| ProviderError::Auth("GEMINI_API_KEY not set".into()))?;
        if key.is_empty() {
            return Err(ProviderError::Auth("GEMINI_API_KEY is empty".into()));
        }
        Ok(Self::new(key, None))
    }

    /// Override the API base — for tests against wiremock.
    #[cfg(test)]
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = base.into();
        self
    }
}

/// Build the Gemini `generationConfig` for an analysis call.
///
/// IMPORTANT — disables thinking (`thinkingConfig.thinkingBudget = 0`): the
/// default model `gemini-2.5-flash` is a *thinking* model (thinking on by
/// default), and on the Gemini API `maxOutputTokens` caps thinking **and**
/// output COMBINED. For a non-trivial prompt the reasoning tokens consumed the
/// whole budget, leaving ~10 for the JSON body → `finishReason=MAX_TOKENS` and a
/// truncated `{"summary": "The…` fragment stored as the event summary (the real
/// content lost). These calls are extraction/summarization, not reasoning-heavy,
/// so we disable thinking and let the full `maxOutputTokens` go to the answer.
/// (Ref: https://ai.google.dev/gemini-api/docs/thinking — set budget 0 to off.)
fn build_generation_config(
    input: &AnalysisInput,
) -> serde_json::Map<String, serde_json::Value> {
    let mut gen_config = serde_json::Map::new();
    if let Some(n) = input.max_output_tokens {
        gen_config.insert("maxOutputTokens".into(), serde_json::json!(n));
    }
    // Disable thinking so maxOutputTokens is not starved (see fn doc).
    gen_config.insert(
        "thinkingConfig".into(),
        serde_json::json!({ "thinkingBudget": 0 }),
    );
    if matches!(input.response_format, ResponseFormat::Json) {
        gen_config.insert(
            "response_mime_type".into(),
            serde_json::json!("application/json"),
        );
        if let Some(schema) = &input.response_schema {
            gen_config.insert("response_schema".into(), schema.clone());
        }
    }
    gen_config
}

#[async_trait::async_trait]
impl MultimodalProvider for GeminiMultimodalProvider {
    async fn analyze(&self, input: AnalysisInput) -> Result<AnalysisResult, ProviderError> {
        use base64::Engine as _;
        let start = std::time::Instant::now();

        // Build Gemini "contents" array — every modality is a part.
        let mut parts: Vec<serde_json::Value> = Vec::with_capacity(input.modalities.len() + 1);
        if let Some(sp) = &input.system_prompt {
            parts.push(serde_json::json!({"text": format!("[SYSTEM] {}\n", sp)}));
        }
        for m in &input.modalities {
            match m {
                Modality::Image { bytes, mime } | Modality::Audio { bytes, mime } => {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                    parts.push(serde_json::json!({
                        "inline_data": { "mime_type": mime, "data": b64 }
                    }));
                }
                Modality::Text(t) => {
                    parts.push(serde_json::json!({"text": t}));
                }
            }
        }
        parts.push(serde_json::json!({"text": input.user_prompt}));

        let gen_config = build_generation_config(&input);

        let body = serde_json::json!({
            "contents":          [{ "parts": parts }],
            "generationConfig":  gen_config,
        });

        // Send the API key in the `x-goog-api-key` header, NOT the URL query.
        // reqwest embeds the full request URL (query included) in transport-error
        // Display, and URLs also flow to logs / OTEL spans / proxies — a key in
        // `?key=` leaks through all of them. The header keeps it out of the URL.
        // (groq.rs uses bearer_auth for the same reason.)
        let url = format!("{}/models/{}:generateContent", self.base, self.model);
        let resp = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .header("x-goog-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        match status.as_u16() {
            200 => {}
            401 | 403 => {
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Auth(format!("HTTP {}: {}", status, txt)));
            }
            429 => {
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
            _ => {
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Provider(format!("HTTP {}: {}", status, txt)));
            }
        }

        let raw: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Parse(format!("response JSON: {}", e)))?;

        let text = raw["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .ok_or_else(|| {
                ProviderError::Parse("missing candidates[0].content.parts[0].text".into())
            })?;

        // Best-effort structured-output parse. If the model returned JSON
        // matching our shape, lift fields; otherwise fall back to using
        // the whole text as `summary`.
        let (summary, goal_impact, suggestion, confidence) =
            if matches!(input.response_format, ResponseFormat::Json) {
                if let Ok(j) = serde_json::from_str::<serde_json::Value>(text) {
                    (
                        j["summary"].as_str().unwrap_or(text).to_string(),
                        j["goal_impact"].as_str().map(|s| s.to_string()),
                        j["suggestion"].as_str().map(|s| s.to_string()),
                        j["confidence"].as_f64().map(|f| f as f32),
                    )
                } else {
                    (text.to_string(), None, None, None)
                }
            } else {
                (text.to_string(), None, None, None)
            };

        Ok(AnalysisResult {
            summary,
            goal_impact,
            suggestion,
            confidence,
            raw_response: raw,
            model_id: self.model.clone(),
            latency_ms: start.elapsed().as_millis() as u64,
            cost_usd: None, // populated in a future task once token-cost table lands
        })
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_image: true,
            supports_audio: true,
            supports_video: false, // Gemini 2.0 Flash supports video too, but
            // out of v0.6.0 scope; flip when adding F104 video tests
            max_image_count: 16,
            max_audio_secs: 60,
            max_total_bytes: 20 * 1024 * 1024,
        }
    }

    fn model_id(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: gemini-2.5-flash is a thinking model and maxOutputTokens caps
    // thinking+output combined, so without thinkingBudget=0 the JSON summary was
    // truncated (finishReason=MAX_TOKENS). Guard that we always disable thinking
    // and still keep the token cap + structured-output fields.
    #[test]
    fn generation_config_disables_thinking_to_prevent_summary_truncation() {
        let input = AnalysisInput {
            modalities: vec![Modality::Text("a meal photo".into())],
            system_prompt: None,
            user_prompt: "summarize".into(),
            max_output_tokens: Some(512),
            response_format: ResponseFormat::Json,
            response_schema: Some(serde_json::json!({"type": "object"})),
        };
        let gc = build_generation_config(&input);
        assert_eq!(
            gc.get("thinkingConfig"),
            Some(&serde_json::json!({ "thinkingBudget": 0 })),
            "thinking must be disabled so maxOutputTokens feeds the answer, not reasoning"
        );
        assert_eq!(gc.get("maxOutputTokens"), Some(&serde_json::json!(512)));
        assert_eq!(
            gc.get("response_mime_type"),
            Some(&serde_json::json!("application/json"))
        );
        assert!(gc.contains_key("response_schema"));
    }

    #[test]
    fn generation_config_disables_thinking_even_for_plaintext() {
        let input = AnalysisInput {
            modalities: vec![Modality::Text("x".into())],
            system_prompt: None,
            user_prompt: "y".into(),
            max_output_tokens: None,
            response_format: ResponseFormat::PlainText,
            response_schema: None,
        };
        let gc = build_generation_config(&input);
        assert_eq!(
            gc.get("thinkingConfig"),
            Some(&serde_json::json!({ "thinkingBudget": 0 }))
        );
        // No cap requested → key absent; no JSON fields for plaintext.
        assert!(!gc.contains_key("maxOutputTokens"));
        assert!(!gc.contains_key("response_mime_type"));
    }

    #[test]
    fn from_env_fails_when_unset() {
        // GEMINI_API_KEY is process-global; serialize against api_key_from_env
        // (and any other env-touching test) via the crate env mutex.
        let _g = crate::env_lock::acquire();
        // Save + clear env to avoid leakage across tests
        let saved = std::env::var("GEMINI_API_KEY").ok();
        std::env::remove_var("GEMINI_API_KEY");

        let r = GeminiMultimodalProvider::from_env();
        assert!(matches!(r, Err(ProviderError::Auth(_))));
        if let Err(ProviderError::Auth(msg)) = r {
            assert!(msg.contains("GEMINI_API_KEY"));
        }

        // restore
        if let Some(v) = saved {
            std::env::set_var("GEMINI_API_KEY", v);
        }
    }

    #[test]
    fn api_key_from_env() {
        let _g = crate::env_lock::acquire();
        std::env::set_var("GEMINI_API_KEY", "fake-key");
        let r = GeminiMultimodalProvider::from_env();
        assert!(r.is_ok(), "expected Ok, got {:?}", r.err());
        let p = r.unwrap();
        assert_eq!(p.api_key, "fake-key");
        assert_eq!(p.model, DEFAULT_MODEL);
        std::env::remove_var("GEMINI_API_KEY");
    }

    #[test]
    fn capabilities_advertises_image_and_audio() {
        let p = GeminiMultimodalProvider::new("k", None);
        let c = p.capabilities();
        assert!(c.supports_image);
        assert!(c.supports_audio);
        assert!(!c.supports_video);
        assert!(c.max_total_bytes >= 1024 * 1024);
    }

    #[test]
    fn model_id_defaults_then_overrides() {
        let p = GeminiMultimodalProvider::new("k", None);
        assert_eq!(p.model_id(), DEFAULT_MODEL);
        let p2 = GeminiMultimodalProvider::new("k", Some("gemini-1.5-pro".into()));
        assert_eq!(p2.model_id(), "gemini-1.5-pro");
    }

    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn analyze_with_image_returns_summary_from_mock_gemini() {
        let mock = MockServer::start().await;

        // Gemini's response shape — minimal but realistic
        let body = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "text": r#"{"summary":"a plate of fried rice","goal_impact":"moderate carbs","suggestion":"add vegetables","confidence":0.75}"#
                    }]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 50,
                "candidatesTokenCount": 30,
                "totalTokenCount": 80
            }
        });

        Mock::given(method("POST"))
            .and(path_regex(r":generateContent$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&mock)
            .await;

        let p = GeminiMultimodalProvider::new("fake-key", None).with_base(mock.uri());
        let input = AnalysisInput {
            modalities: vec![
                Modality::Image {
                    bytes: vec![0xff, 0xd8, 0xff],
                    mime: "image/jpeg".into(),
                },
                Modality::Text("what's in this photo".into()),
            ],
            system_prompt: Some("you are a coach".into()),
            user_prompt: "analyze the meal".into(),
            max_output_tokens: Some(256),
            response_format: ResponseFormat::Json,
            response_schema: None,
        };

        let r = p.analyze(input).await.expect("mock should respond 200");
        assert_eq!(r.summary, "a plate of fried rice");
        assert_eq!(r.goal_impact.as_deref(), Some("moderate carbs"));
        assert_eq!(r.suggestion.as_deref(), Some("add vegetables"));
        assert_eq!(r.confidence, Some(0.75));
        assert_eq!(r.model_id, DEFAULT_MODEL);
        assert!(
            r.latency_ms < 5000,
            "mock should be fast: {}ms",
            r.latency_ms
        );
    }

    #[tokio::test]
    async fn rate_limit_returns_typed_error() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r":generateContent$"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&mock)
            .await;

        let p = GeminiMultimodalProvider::new("k", None).with_base(mock.uri());
        let r = p
            .analyze(AnalysisInput {
                modalities: vec![Modality::Text("hi".into())],
                system_prompt: None,
                user_prompt: "x".into(),
                max_output_tokens: None,
                response_format: ResponseFormat::PlainText,
                response_schema: None,
            })
            .await;

        assert!(
            matches!(r, Err(ProviderError::RateLimit { .. })),
            "expected RateLimit, got {:?}",
            r.map_err(|e| e.to_string())
        );
    }

    #[tokio::test]
    async fn send_error_never_leaks_api_key() {
        // The API key must never surface in an error string (which flows to
        // logs / OTEL spans / proxy traces). Point at an unroutable address so
        // `.send()` fails at the transport layer; reqwest embeds the request
        // URL in that error's Display, so a key carried in the URL query would
        // leak. Regression guard for sending the key via the `x-goog-api-key`
        // header instead (groq.rs already uses bearer_auth for the same reason).
        let secret = "fake-leak-canary-not-a-real-key-0000";
        let p = GeminiMultimodalProvider::new(secret, None).with_base("http://127.0.0.1:1");
        let err = p
            .analyze(AnalysisInput {
                modalities: vec![Modality::Text("hi".into())],
                system_prompt: None,
                user_prompt: "x".into(),
                max_output_tokens: None,
                response_format: ResponseFormat::PlainText,
                response_schema: None,
            })
            .await
            .expect_err("connection to 127.0.0.1:1 must fail");
        let msg = err.to_string();
        assert!(
            !msg.contains(secret),
            "API key leaked in provider error: {msg}"
        );
    }

    #[tokio::test]
    async fn analyze_returns_auth_on_401() {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r":generateContent$"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock)
            .await;

        let p = GeminiMultimodalProvider::new("k", None).with_base(mock.uri());
        let r = p
            .analyze(AnalysisInput {
                modalities: vec![Modality::Text("x".into())],
                system_prompt: None,
                user_prompt: "x".into(),
                max_output_tokens: None,
                response_format: ResponseFormat::PlainText,
                response_schema: None,
            })
            .await;

        assert!(matches!(r, Err(ProviderError::Auth(_))));
    }
}
