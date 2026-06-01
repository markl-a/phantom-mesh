//! Groq text-only fallback for life_node event analysis.
//!
//! Used when Gemini is rate-limited. Text-only (no image/audio/video).
//! Reads `GROQ_API_KEY` from env; falls back gracefully when unset.

use crate::life_node::multimodal::{
    AnalysisInput, AnalysisResult, Modality, MultimodalProvider, ProviderCapabilities,
    ProviderError, ResponseFormat,
};
use serde_json::{json, Value};

const DEFAULT_MODEL: &str = "llama-3.1-8b-instant";
const API_BASE: &str = "https://api.groq.com/openai/v1/chat/completions";

pub struct GroqTextProvider {
    api_key: String,
    model: String,
    http: reqwest::Client,
}

impl GroqTextProvider {
    pub fn new(api_key: impl Into<String>, model: Option<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.unwrap_or_else(|| DEFAULT_MODEL.into()),
            http: reqwest::Client::new(),
        }
    }

    pub fn from_env() -> Result<Self, ProviderError> {
        let key = std::env::var("GROQ_API_KEY")
            .map_err(|_| ProviderError::Auth("GROQ_API_KEY not set".into()))?;
        if key.is_empty() {
            return Err(ProviderError::Auth("GROQ_API_KEY is empty".into()));
        }
        Ok(Self::new(key, None))
    }
}

#[async_trait::async_trait]
impl MultimodalProvider for GroqTextProvider {
    async fn analyze(&self, input: AnalysisInput) -> Result<AnalysisResult, ProviderError> {
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
            "temperature": 0.3
        });

        let t0 = std::time::Instant::now();
        let resp = self
            .http
            .post(API_BASE)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if status.as_u16() == 429 {
            return Err(ProviderError::RateLimit {
                retry_after_ms: None,
            });
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
            cost_usd: None,
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
