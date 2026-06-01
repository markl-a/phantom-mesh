//! `MultimodalProvider` trait and supporting types. Per spec
//! `2026-05-19-life-node-pivot.md` §6.

use serde::{Deserialize, Serialize};

/// One modality slot in an `AnalysisInput`. Limited to a closed set;
/// new modalities (video, PDF) added as new variants — not via trait
/// objects — so providers `match` exhaustively at compile time.
#[derive(Debug, Clone)]
pub enum Modality {
    /// Raw image bytes (jpeg / png / webp). `mime` follows IANA spec.
    Image { bytes: Vec<u8>, mime: String },
    /// Raw audio bytes (wav / mp3 / ogg). `mime` per IANA.
    Audio { bytes: Vec<u8>, mime: String },
    /// Plain UTF-8 text.
    Text(String),
}

/// Provider's expected output format. JSON-mode forces structured
/// output where the provider supports it (Gemini `response_mime_type`,
/// Claude tool-use, GPT structured outputs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseFormat {
    Json,
    Markdown,
    PlainText,
}

/// Typed provider errors. `RateLimit` lets dispatcher do typed
/// failover; the generic `Provider(msg)` is the catch-all for
/// vendor-specific errors not worth modelling.
#[derive(thiserror::Error, Debug)]
pub enum ProviderError {
    #[error("auth: {0}")]
    Auth(String),
    #[error("rate limited, retry in {retry_after_ms:?}ms")]
    RateLimit { retry_after_ms: Option<u64> },
    #[error("modality unsupported: {0}")]
    Modality(String),
    #[error("network: {0}")]
    Network(#[from] reqwest::Error),
    #[error("parse: {0}")]
    Parse(String),
    #[error("provider: {0}")]
    Provider(String),
}

/// Input to `MultimodalProvider::analyze`. Owns all modality bytes
/// so caller can drop file handles before the async call.
#[derive(Clone)]
pub struct AnalysisInput {
    pub modalities: Vec<Modality>,
    pub system_prompt: Option<String>,
    pub user_prompt: String,
    pub max_output_tokens: Option<u32>,
    pub response_format: ResponseFormat,
    /// Provider-specific JSON schema for structured outputs. Gemini
    /// reads this as `generationConfig.response_schema`. Ignored by
    /// providers that don't support schema-constrained output.
    pub response_schema: Option<serde_json::Value>,
}

/// Output from `analyze`. The semantic fields (`summary`, `goal_impact`,
/// `suggestion`, `confidence`) are extracted by the provider impl from
/// the LLM response — they may be missing if the response didn't
/// follow the requested shape. `raw_response` always contains the full
/// provider JSON for downstream debugging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub summary: String,
    pub goal_impact: Option<String>,
    pub suggestion: Option<String>,
    pub confidence: Option<f32>,
    pub raw_response: serde_json::Value,
    pub model_id: String,
    pub latency_ms: u64,
    pub cost_usd: Option<f32>,
}

/// Static metadata about a provider — used by the future dispatcher
/// (out of v0.6.0 scope, but the shape lands here so adding more
/// providers is purely additive).
#[derive(Debug, Clone, Copy)]
pub struct ProviderCapabilities {
    pub supports_image: bool,
    pub supports_audio: bool,
    pub supports_video: bool,
    pub max_image_count: u32,
    pub max_audio_secs: u32,
    pub max_total_bytes: u64,
}

/// Provider-agnostic multimodal analysis trait. Impl #0 lives in
/// `providers::gemini`; future impls (Claude, GPT-4o, ollama, MLX)
/// drop in without changing callers.
#[async_trait::async_trait]
pub trait MultimodalProvider: Send + Sync {
    async fn analyze(&self, input: AnalysisInput) -> Result<AnalysisResult, ProviderError>;
    fn capabilities(&self) -> ProviderCapabilities;
    fn model_id(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modality_image_holds_bytes_and_mime() {
        let m = Modality::Image {
            bytes: vec![0xff, 0xd8, 0xff],
            mime: "image/jpeg".into(),
        };
        match m {
            Modality::Image { bytes, mime } => {
                assert_eq!(bytes, vec![0xff, 0xd8, 0xff]);
                assert_eq!(mime, "image/jpeg");
            }
            _ => panic!("expected Image variant"),
        }
    }

    #[test]
    fn modality_audio_holds_bytes_and_mime() {
        let m = Modality::Audio {
            bytes: vec![1, 2, 3],
            mime: "audio/wav".into(),
        };
        if let Modality::Audio { bytes, mime } = m {
            assert_eq!(bytes.len(), 3);
            assert_eq!(mime, "audio/wav");
        } else {
            panic!("expected Audio variant");
        }
    }

    #[test]
    fn modality_text_holds_string() {
        let m = Modality::Text("lunch".into());
        if let Modality::Text(s) = m {
            assert_eq!(s, "lunch");
        } else {
            panic!("expected Text variant");
        }
    }

    #[test]
    fn response_format_equality() {
        assert_eq!(ResponseFormat::Json, ResponseFormat::Json);
        assert_ne!(ResponseFormat::Json, ResponseFormat::Markdown);
    }

    #[test]
    fn provider_error_displays_with_context() {
        let e = ProviderError::Auth("missing api key".into());
        assert!(e.to_string().contains("missing api key"));

        let e2 = ProviderError::RateLimit {
            retry_after_ms: Some(1500),
        };
        assert!(e2.to_string().contains("1500"));

        let e3 = ProviderError::Modality("video not supported".into());
        assert!(e3.to_string().contains("video"));
    }

    #[test]
    fn analysis_input_can_be_constructed_with_three_modalities() {
        let input = AnalysisInput {
            modalities: vec![
                Modality::Image {
                    bytes: vec![1, 2, 3],
                    mime: "image/jpeg".into(),
                },
                Modality::Audio {
                    bytes: vec![4, 5, 6],
                    mime: "audio/wav".into(),
                },
                Modality::Text("describe this meal".into()),
            ],
            system_prompt: Some("you are a coach".into()),
            user_prompt: "analyze for fat loss".into(),
            max_output_tokens: Some(512),
            response_format: ResponseFormat::Json,
            response_schema: None,
        };
        assert_eq!(input.modalities.len(), 3);
        assert_eq!(input.max_output_tokens, Some(512));
    }

    #[test]
    fn analysis_result_round_trips_through_json() {
        let r = AnalysisResult {
            summary: "looks heavy on carbs".into(),
            goal_impact: Some("slight setback for fat loss".into()),
            suggestion: Some("add a serving of vegetables next meal".into()),
            confidence: Some(0.7),
            raw_response: serde_json::json!({"candidates": []}),
            model_id: "gemini-2.0-flash-exp".into(),
            latency_ms: 1234,
            cost_usd: Some(0.001),
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: AnalysisResult = serde_json::from_str(&s).unwrap();
        assert_eq!(back.summary, r.summary);
        assert_eq!(back.confidence, r.confidence);
        assert_eq!(back.latency_ms, r.latency_ms);
    }

    #[test]
    fn capabilities_struct_works() {
        let c = ProviderCapabilities {
            supports_image: true,
            supports_audio: true,
            supports_video: false,
            max_image_count: 16,
            max_audio_secs: 60,
            max_total_bytes: 20 * 1024 * 1024,
        };
        assert!(c.supports_image);
        assert!(!c.supports_video);
        assert_eq!(c.max_audio_secs, 60);
    }

    /// Test-only impl that echoes its input as a `summary` — proves the
    /// trait shape compiles and is `Send + Sync`. Real impls live in
    /// `providers/`.
    struct EchoProvider;

    #[async_trait::async_trait]
    impl MultimodalProvider for EchoProvider {
        async fn analyze(&self, input: AnalysisInput) -> Result<AnalysisResult, ProviderError> {
            Ok(AnalysisResult {
                summary: format!("echo: {}", input.user_prompt),
                goal_impact: None,
                suggestion: None,
                confidence: None,
                raw_response: serde_json::json!({}),
                model_id: "echo".into(),
                latency_ms: 0,
                cost_usd: None,
            })
        }
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                supports_image: true,
                supports_audio: true,
                supports_video: false,
                max_image_count: 1,
                max_audio_secs: 10,
                max_total_bytes: 1024,
            }
        }
        fn model_id(&self) -> &str {
            "echo"
        }
    }

    #[tokio::test]
    async fn trait_compiles_and_echo_provider_returns_summary() {
        let p = EchoProvider;
        let r = p
            .analyze(AnalysisInput {
                modalities: vec![Modality::Text("hi".into())],
                system_prompt: None,
                user_prompt: "what is this".into(),
                max_output_tokens: None,
                response_format: ResponseFormat::PlainText,
                response_schema: None,
            })
            .await
            .unwrap();
        assert_eq!(r.summary, "echo: what is this");
        assert_eq!(p.model_id(), "echo");

        // dispatchability — trait object compiles
        let _b: Box<dyn MultimodalProvider> = Box::new(EchoProvider);
    }

    #[tokio::test]
    async fn trait_round_trip_with_image_only() {
        let p = EchoProvider;
        let r = p
            .analyze(AnalysisInput {
                modalities: vec![Modality::Image {
                    bytes: vec![1, 2, 3],
                    mime: "image/jpeg".into(),
                }],
                system_prompt: None,
                user_prompt: "what is in this image".into(),
                max_output_tokens: None,
                response_format: ResponseFormat::PlainText,
                response_schema: None,
            })
            .await
            .unwrap();
        assert!(r.summary.contains("what is in this image"));
    }

    #[tokio::test]
    async fn trait_round_trip_with_audio_only() {
        let p = EchoProvider;
        let r = p
            .analyze(AnalysisInput {
                modalities: vec![Modality::Audio {
                    bytes: vec![1, 2, 3, 4],
                    mime: "audio/wav".into(),
                }],
                system_prompt: None,
                user_prompt: "transcribe this audio".into(),
                max_output_tokens: None,
                response_format: ResponseFormat::PlainText,
                response_schema: None,
            })
            .await
            .unwrap();
        assert!(r.summary.contains("transcribe this audio"));
    }

    #[tokio::test]
    async fn trait_round_trip_with_image_plus_audio() {
        let p = EchoProvider;
        let r = p
            .analyze(AnalysisInput {
                modalities: vec![
                    Modality::Image {
                        bytes: vec![1, 2],
                        mime: "image/png".into(),
                    },
                    Modality::Audio {
                        bytes: vec![3, 4],
                        mime: "audio/mp3".into(),
                    },
                    Modality::Text("with context".into()),
                ],
                system_prompt: None,
                user_prompt: "describe both".into(),
                max_output_tokens: None,
                response_format: ResponseFormat::PlainText,
                response_schema: None,
            })
            .await
            .unwrap();
        assert!(r.summary.contains("describe both"));
    }
}
