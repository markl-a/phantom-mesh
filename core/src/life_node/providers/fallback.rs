//! Vision-preserving provider fallback chain (SPEC-20 capture-food).
//!
//! The pre-fix food path tried Gemini first and, on a rate-limit, fell back to
//! a TEXT-ONLY provider (Groq) — silently discarding the meal photo and
//! analysing only the caption. This module fails over correctly: when the input
//! carries image pixels, providers that don't natively accept images are
//! SKIPPED rather than handed a request whose photo they'd drop.
//!
//! 中文: 保留影像的 provider 後援鏈 — 輸入含圖片時,跳過「不支援影像」的
//! provider,絕不把照片靜默降級成純文字分析。

use crate::life_node::multimodal::{
    AnalysisInput, AnalysisResult, Modality, MultimodalProvider, ProviderError,
};

/// True when the input carries a non-text modality (image) that a provider must
/// natively accept to analyse faithfully.
fn input_has_image(input: &AnalysisInput) -> bool {
    input
        .modalities
        .iter()
        .any(|m| matches!(m, Modality::Image { .. }))
}

/// Run `input` through `providers` in order, returning the first success.
///
/// Failover policy:
/// - **Image inputs skip text-only providers** (`capabilities().supports_image
///   == false`) — never degrade a photo to a caption-only analysis.
/// - Transient/soft errors (`RateLimit` / `Network` / `Provider`) advance to the
///   next eligible provider.
/// - Deterministic errors (`Auth` / `Parse` / `Modality`) fail fast — another
///   provider won't fix a bad key or a malformed response, and surfacing them
///   beats masking a real bug.
///
/// If every eligible provider failed, returns the last soft error. If the input
/// needed an image but no provider in the chain was image-capable, returns a
/// `Modality` error (so the caller reports "no vision provider" instead of
/// silently analysing text).
pub async fn try_vision_chain(
    input: AnalysisInput,
    providers: &[Box<dyn MultimodalProvider>],
) -> Result<AnalysisResult, ProviderError> {
    let needs_image = input_has_image(&input);
    let mut last_err: Option<ProviderError> = None;
    let mut skipped_text_only = 0usize;

    for p in providers {
        if needs_image && !p.capabilities().supports_image {
            skipped_text_only += 1;
            continue;
        }
        match p.analyze(input.clone()).await {
            Ok(a) => return Ok(a),
            Err(e @ ProviderError::RateLimit { .. })
            | Err(e @ ProviderError::Network(_))
            | Err(e @ ProviderError::Provider(_)) => {
                last_err = Some(e);
                continue;
            }
            // Auth / Parse / Modality: deterministic — don't mask by retrying.
            Err(e) => return Err(e),
        }
    }

    Err(last_err.unwrap_or_else(|| {
        ProviderError::Modality(format!(
            "no image-capable provider available ({} text-only skipped) for an image input",
            skipped_text_only
        ))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::life_node::multimodal::{ProviderCapabilities, ResponseFormat};
    use async_trait::async_trait;

    /// Configurable mock provider. `result` is what `analyze` returns;
    /// `supports_image` drives capability filtering. `seen_image` records
    /// whether the provider was actually handed an image modality, so a test can
    /// prove the photo survived to the winning provider.
    struct MockProvider {
        id: &'static str,
        supports_image: bool,
        outcome: MockOutcome,
        seen_image: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    enum MockOutcome {
        Ok,
        RateLimit,
        Auth,
    }

    #[async_trait]
    impl MultimodalProvider for MockProvider {
        async fn analyze(&self, input: AnalysisInput) -> Result<AnalysisResult, ProviderError> {
            if input
                .modalities
                .iter()
                .any(|m| matches!(m, Modality::Image { .. }))
            {
                self.seen_image
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            match self.outcome {
                MockOutcome::Ok => Ok(AnalysisResult {
                    summary: format!("ok from {}", self.id),
                    goal_impact: None,
                    suggestion: None,
                    confidence: None,
                    raw_response: serde_json::json!({}),
                    model_id: self.id.to_string(),
                    latency_ms: 1,
                    cost_usd: None,
                }),
                MockOutcome::RateLimit => Err(ProviderError::RateLimit {
                    retry_after_ms: Some(1000),
                }),
                MockOutcome::Auth => Err(ProviderError::Auth("bad key".into())),
            }
        }
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                supports_image: self.supports_image,
                supports_audio: false,
                supports_video: false,
                max_image_count: 1,
                max_audio_secs: 0,
                max_total_bytes: 10_000_000,
            }
        }
        fn model_id(&self) -> &str {
            self.id
        }
    }

    fn image_input() -> AnalysisInput {
        AnalysisInput {
            modalities: vec![Modality::Image {
                bytes: vec![0xff, 0xd8, 0xff],
                mime: "image/jpeg".into(),
            }],
            system_prompt: None,
            user_prompt: "meal".into(),
            max_output_tokens: None,
            response_format: ResponseFormat::Json,
            response_schema: None,
        }
    }
    fn text_input() -> AnalysisInput {
        AnalysisInput {
            modalities: vec![Modality::Text("salad".into())],
            system_prompt: None,
            user_prompt: "meal".into(),
            max_output_tokens: None,
            response_format: ResponseFormat::Json,
            response_schema: None,
        }
    }
    fn flag() -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))
    }

    #[tokio::test]
    async fn rate_limited_vision_provider_fails_over_keeping_the_image() {
        // Gemini-like (image) rate-limited → second image-capable provider wins,
        // and the winning provider MUST have received the image (not a stripped
        // text-only request). This is the core regression guard.
        let seen = flag();
        let chain: Vec<Box<dyn MultimodalProvider>> = vec![
            Box::new(MockProvider {
                id: "vision-1",
                supports_image: true,
                outcome: MockOutcome::RateLimit,
                seen_image: flag(),
            }),
            Box::new(MockProvider {
                id: "vision-2",
                supports_image: true,
                outcome: MockOutcome::Ok,
                seen_image: seen.clone(),
            }),
        ];
        let out = try_vision_chain(image_input(), &chain).await.unwrap();
        assert_eq!(out.model_id, "vision-2", "failover should reach the 2nd vision provider");
        assert!(
            seen.load(std::sync::atomic::Ordering::SeqCst),
            "the winning provider must have received the image (photo not dropped)"
        );
    }

    #[tokio::test]
    async fn image_input_skips_text_only_provider_instead_of_dropping_photo() {
        // Image input + a rate-limited vision provider + a text-only provider:
        // the text-only one must be SKIPPED (not silently analyse text), so the
        // chain reports a Modality error rather than a bogus caption analysis.
        let text_seen = flag();
        let chain: Vec<Box<dyn MultimodalProvider>> = vec![
            Box::new(MockProvider {
                id: "vision",
                supports_image: true,
                outcome: MockOutcome::RateLimit,
                seen_image: flag(),
            }),
            Box::new(MockProvider {
                id: "text-only",
                supports_image: false,
                outcome: MockOutcome::Ok,
                seen_image: text_seen.clone(),
            }),
        ];
        let err = try_vision_chain(image_input(), &chain).await.unwrap_err();
        // We surfaced the last soft error (the vision rate-limit) rather than
        // using the text-only provider.
        assert!(matches!(err, ProviderError::RateLimit { .. }));
        assert!(
            !text_seen.load(std::sync::atomic::Ordering::SeqCst),
            "text-only provider must never be handed an image input"
        );
    }

    #[tokio::test]
    async fn no_image_capable_provider_for_image_yields_modality_error() {
        // Only text-only providers, but the input has a photo → Modality error,
        // never a text analysis of the caption.
        let chain: Vec<Box<dyn MultimodalProvider>> = vec![Box::new(MockProvider {
            id: "text-only",
            supports_image: false,
            outcome: MockOutcome::Ok,
            seen_image: flag(),
        })];
        let err = try_vision_chain(image_input(), &chain).await.unwrap_err();
        assert!(matches!(err, ProviderError::Modality(_)));
    }

    #[tokio::test]
    async fn text_input_may_use_text_only_provider() {
        // No image → a text-only provider is perfectly eligible.
        let chain: Vec<Box<dyn MultimodalProvider>> = vec![Box::new(MockProvider {
            id: "text-only",
            supports_image: false,
            outcome: MockOutcome::Ok,
            seen_image: flag(),
        })];
        let out = try_vision_chain(text_input(), &chain).await.unwrap();
        assert_eq!(out.model_id, "text-only");
    }

    #[tokio::test]
    async fn auth_error_fails_fast_without_trying_next() {
        // Deterministic Auth error must not be masked by failing over.
        let later_seen = flag();
        let chain: Vec<Box<dyn MultimodalProvider>> = vec![
            Box::new(MockProvider {
                id: "vision-1",
                supports_image: true,
                outcome: MockOutcome::Auth,
                seen_image: flag(),
            }),
            Box::new(MockProvider {
                id: "vision-2",
                supports_image: true,
                outcome: MockOutcome::Ok,
                seen_image: later_seen.clone(),
            }),
        ];
        let err = try_vision_chain(image_input(), &chain).await.unwrap_err();
        assert!(matches!(err, ProviderError::Auth(_)));
        assert!(
            !later_seen.load(std::sync::atomic::Ordering::SeqCst),
            "auth error should fail fast, not try the next provider"
        );
    }
}
