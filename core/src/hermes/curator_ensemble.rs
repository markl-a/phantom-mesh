//! T28: Hermes Curator V2 — multi-judge ensemble.
//!
//! See docs/superpowers/plans/2026-05-15-track-t28-curator-v2.md.
//!
//! Builds on V1 (`curator.rs`) which scores via a single Anthropic call.
//! V2 sends the same `EvolveCheckpoint` to N independent judges concurrently,
//! aggregates verdicts via median + population stddev, and flags >2σ
//! disagreement (or <2 succeeded judges) as `NeedsHumanReview`.
//!
//! All V1 APIs remain available; V2 is additive.
//!
//! Concurrency primitive: `tokio::task::JoinSet` (already available via the
//! workspace's tokio = "1" full-features dep). No new external crates.

#![cfg(feature = "experimental-hermes-curator")]

use std::fmt;

use crate::evolve_checkpoint::{AgreementClass, EnsembleVerdict, EvolveCheckpoint, JudgeVerdict};
use crate::hermes::curator::{build_judge_user_prompt, parse_judge_reply_strict, RUBRIC_VERSION};

// ─── Errors + trait ──────────────────────────────────────────────────────

/// Strongly-typed judge failure. Constructed by adapters; consumed by
/// `EnsembleCurator::judge_ensemble` to keep a per-judge result vec.
#[derive(Debug)]
pub enum JudgeError {
    Http(String),
    Schema(String),
    Timeout,
}

impl fmt::Display for JudgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JudgeError::Http(m) => write!(f, "http: {}", m),
            JudgeError::Schema(m) => write!(f, "schema: {}", m),
            JudgeError::Timeout => write!(f, "timeout"),
        }
    }
}

impl std::error::Error for JudgeError {}

/// Provider-agnostic judge. One impl per provider family: Anthropic vs
/// OpenAI-compat (mistral / xai / together / fireworks).
#[async_trait::async_trait]
pub trait JudgeProvider: Send + Sync {
    /// Identifier baked into the JudgeVerdict's `model` field. Must be
    /// unique enough to grep for: e.g. "claude-haiku-4-5-20251001"
    /// or "mistral-small-latest@mistral".
    fn model_id(&self) -> String;

    /// Issue the round-trip and return (score, rationale). Implementations
    /// must call `parse_judge_reply_strict` on the assistant text and
    /// return `JudgeError::Schema` on any deviation from the JSON schema.
    async fn score(&self, user_prompt: &str) -> Result<(u8, String), JudgeError>;
}

// ─── Aggregation pure-fns ────────────────────────────────────────────────

/// Sorted median of u8 scores in [0..=10]. Even-count returns the mean of the
/// two middles as f32 (which the caller rounds to u8 via `round_half_up`).
/// **Panics** on empty input — caller (`aggregate`) MUST guard.
pub fn median_score(scores: &[u8]) -> f32 {
    assert!(!scores.is_empty(), "median_score: empty input");
    let mut sorted: Vec<u8> = scores.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2] as f32
    } else {
        (sorted[n / 2 - 1] as f32 + sorted[n / 2] as f32) / 2.0
    }
}

/// Population (not sample) stddev of u8 scores. Returns 0.0 for empty or
/// single-element input — those are degenerate cases that `aggregate` flags
/// separately via `judges_succeeded < 2 ⇒ NeedsHumanReview`.
pub fn population_stddev(scores: &[u8]) -> f32 {
    if scores.len() < 2 {
        return 0.0;
    }
    let mean: f32 = scores.iter().map(|&s| s as f32).sum::<f32>() / scores.len() as f32;
    let var: f32 = scores
        .iter()
        .map(|&s| {
            let d = s as f32 - mean;
            d * d
        })
        .sum::<f32>()
        / scores.len() as f32;
    var.sqrt()
}

/// Even-count median lands on a 0.5; round half-up to a u8 score.
fn round_half_up(x: f32) -> u8 {
    (x + 0.5).floor().clamp(0.0, 10.0) as u8
}

/// Aggregate per-judge verdicts into an EnsembleVerdict. Pure function.
///
/// Contract:
/// - `succeeded` may be empty (all judges failed); the result still has a
///   valid shape but score=0 and agreement=NeedsHumanReview.
/// - `attempted` is the total judge count the caller dispatched (success +
///   failure), used to populate `judges_attempted` for diagnostic surfacing.
/// - `judged_at_ms` is the caller's wall-clock at the time the ensemble
///   completed (typically `now_ms()` after the last judge returns).
pub fn aggregate(
    succeeded: Vec<JudgeVerdict>,
    attempted: u8,
    judged_at_ms: i64,
) -> EnsembleVerdict {
    let scores: Vec<u8> = succeeded.iter().map(|v| v.score).collect();

    let (median_f, stddev_f, score_u8) = if scores.is_empty() {
        (0.0_f32, 0.0_f32, 0u8)
    } else {
        let m = median_score(&scores);
        let s = population_stddev(&scores);
        (m, s, round_half_up(m))
    };

    let agreement = if (succeeded.len() as u8) < 2 {
        AgreementClass::NeedsHumanReview
    } else if stddev_f > 2.0 {
        AgreementClass::NeedsHumanReview
    } else if stddev_f == 0.0 {
        AgreementClass::Unanimous
    } else {
        AgreementClass::Consensus
    };

    let aggregated = JudgeVerdict {
        score: score_u8,
        rubric_version: RUBRIC_VERSION.to_string(),
        model: format!("ensemble:{}", attempted),
        rationale: format!(
            "ensemble of {} judges ({} succeeded); see individual[*]",
            attempted,
            succeeded.len()
        ),
        judged_at_ms,
    };

    let succeeded_count = succeeded.len() as u8;
    EnsembleVerdict {
        aggregated,
        individual: succeeded,
        agreement,
        score_median: median_f,
        score_stddev: stddev_f,
        judges_attempted: attempted,
        judges_succeeded: succeeded_count,
    }
}

// ─── Adapters: AnthropicJudge + OpenAICompatJudge ────────────────────────

/// V2 Anthropic-shaped judge. Same wire format as V1 Curator (Messages API
/// at `/v1/messages` with `x-api-key` header).
pub struct AnthropicJudge {
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    pub timeout_secs: u64,
}

#[async_trait::async_trait]
impl JudgeProvider for AnthropicJudge {
    fn model_id(&self) -> String {
        self.model.clone()
    }

    async fn score(&self, user_prompt: &str) -> Result<(u8, String), JudgeError> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 256,
            "messages": [{"role": "user", "content": user_prompt}]
        });
        let url = format!("{}/v1/messages", self.api_base.trim_end_matches('/'));
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(self.timeout_secs))
            .build()
            .map_err(|e| JudgeError::Http(format!("client build: {}", e)))?;

        let resp = client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    JudgeError::Timeout
                } else {
                    JudgeError::Http(format!("send: {}", e))
                }
            })?;

        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(JudgeError::Http(format!(
                "status {}: {}",
                status,
                &txt[..txt.len().min(200)]
            )));
        }

        let value: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| JudgeError::Http(format!("decode: {}", e)))?;
        let text = value
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| {
                arr.iter().find_map(|b| {
                    let t = b.get("type")?.as_str()?;
                    if t == "text" {
                        b.get("text")?.as_str()
                    } else {
                        None
                    }
                })
            })
            .ok_or_else(|| JudgeError::Schema("no text block in reply".into()))?
            .to_string();

        parse_judge_reply_strict(&text).map_err(JudgeError::Schema)
    }
}

/// V2 OpenAI-compatible judge — drives mistral / xai / together / fireworks.
/// Uses `/v1/chat/completions` + `Authorization: Bearer <key>`.
///
/// `model_id()` returns `"<model>@<provider_id>"` so verdicts coming from
/// `mistral-small-latest` on mistral vs `mistral-small-latest` on a hypothetical
/// other host stay distinguishable in the persisted JudgeVerdict.
pub struct OpenAICompatJudge {
    pub provider_id: String,
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    pub timeout_secs: u64,
}

#[async_trait::async_trait]
impl JudgeProvider for OpenAICompatJudge {
    fn model_id(&self) -> String {
        format!("{}@{}", self.model, self.provider_id)
    }

    async fn score(&self, user_prompt: &str) -> Result<(u8, String), JudgeError> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 256,
            "messages": [{"role": "user", "content": user_prompt}]
        });
        let base = self.api_base.trim_end_matches('/');
        // Normalise to /v1/chat/completions regardless of whether the
        // caller passed a bare host, an /v1, or the final path.
        let url = if base.ends_with("/chat/completions") {
            base.to_string()
        } else if base.ends_with("/v1") {
            format!("{}/chat/completions", base)
        } else {
            format!("{}/v1/chat/completions", base)
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(self.timeout_secs))
            .build()
            .map_err(|e| JudgeError::Http(format!("client build: {}", e)))?;

        let resp = client
            .post(&url)
            .header("authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    JudgeError::Timeout
                } else {
                    JudgeError::Http(format!("send: {}", e))
                }
            })?;

        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(JudgeError::Http(format!(
                "status {}: {}",
                status,
                &txt[..txt.len().min(200)]
            )));
        }

        let value: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| JudgeError::Http(format!("decode: {}", e)))?;
        let text = value
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JudgeError::Schema("no choices[0].message.content".into()))?
            .to_string();

        parse_judge_reply_strict(&text).map_err(JudgeError::Schema)
    }
}

// ─── Ensemble orchestrator ───────────────────────────────────────────────

/// V2 orchestrator. Construct with a `Vec<Arc<dyn JudgeProvider>>` of length
/// N (typically 3). `judge_ensemble` dispatches all judges concurrently via
/// `tokio::task::JoinSet`, collects results in dispatch order, aggregates,
/// and writes the EnsembleVerdict to the checkpoint via
/// `record_ensemble_verdict`.
pub struct EnsembleCurator {
    pub judges: Vec<std::sync::Arc<dyn JudgeProvider>>,
}

pub struct EnsembleOutcome {
    pub verdict: EnsembleVerdict,
    /// One entry per attempted judge, in dispatch order. Successful entries
    /// hold the JudgeVerdict produced; failures hold the JudgeError that
    /// excluded them from the median.
    pub per_judge_results: Vec<Result<JudgeVerdict, JudgeError>>,
}

impl EnsembleCurator {
    /// Score `ckpt` via every configured judge concurrently, aggregate, and
    /// persist via `record_ensemble_verdict`. Caller is responsible for
    /// calling `ckpt.save()` afterwards.
    ///
    /// This method does NOT return an error: a partial or total judge
    /// failure still produces an EnsembleVerdict (with agreement =
    /// NeedsHumanReview when <2 succeeded). The per-judge errors are
    /// returned in `EnsembleOutcome::per_judge_results` for caller logging.
    pub async fn judge_ensemble(&self, ckpt: &mut EvolveCheckpoint) -> EnsembleOutcome {
        use std::time::{SystemTime, UNIX_EPOCH};
        let prompt = std::sync::Arc::new(build_judge_user_prompt(ckpt));
        let now_ms = || {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)
        };

        let mut set = tokio::task::JoinSet::new();
        for (idx, judge) in self.judges.iter().enumerate() {
            let j = judge.clone();
            let p = prompt.clone();
            set.spawn(async move {
                let started = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let res = j.score(&p).await;
                (idx, j.model_id(), started, res)
            });
        }

        // Collect results back into dispatch order.
        let mut slots: Vec<Option<Result<JudgeVerdict, JudgeError>>> =
            (0..self.judges.len()).map(|_| None).collect();
        while let Some(joined) = set.join_next().await {
            let (idx, model_id, started, res) = joined.expect("judge task panicked");
            slots[idx] = Some(match res {
                Ok((score, rationale)) => Ok(JudgeVerdict {
                    score,
                    rubric_version: RUBRIC_VERSION.to_string(),
                    model: model_id,
                    rationale,
                    judged_at_ms: started,
                }),
                Err(e) => Err(e),
            });
        }
        let results: Vec<Result<JudgeVerdict, JudgeError>> = slots
            .into_iter()
            .map(|s| s.expect("all slots filled"))
            .collect();

        let succeeded: Vec<JudgeVerdict> = results
            .iter()
            .filter_map(|r| r.as_ref().ok().cloned())
            .collect();
        let verdict = aggregate(succeeded, self.judges.len() as u8, now_ms());

        ckpt.record_ensemble_verdict(verdict.clone());
        EnsembleOutcome {
            verdict,
            per_judge_results: results,
        }
    }
}

// ─── tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ─── median_score / population_stddev pure-fn tests ──────────────────

    #[test]
    fn median_odd_count_picks_middle() {
        assert_eq!(median_score(&[3, 5, 8]), 5.0);
        assert_eq!(median_score(&[10, 0, 5]), 5.0); // unsorted input ok
    }

    #[test]
    fn median_even_count_averages_two_middles() {
        assert_eq!(median_score(&[4, 6]), 5.0);
        assert_eq!(median_score(&[2, 4, 6, 8]), 5.0);
    }

    #[test]
    fn median_single_element_returns_that_element() {
        assert_eq!(median_score(&[7]), 7.0);
    }

    #[test]
    fn median_empty_panics_caller_must_guard() {
        let result = std::panic::catch_unwind(|| median_score(&[]));
        assert!(
            result.is_err(),
            "median_score([]) must panic — caller must guard"
        );
    }

    #[test]
    fn population_stddev_all_equal_is_zero() {
        let s = population_stddev(&[7, 7, 7]);
        assert!(s.abs() < 1e-6, "expected 0.0, got {}", s);
    }

    #[test]
    fn population_stddev_known_values() {
        // [2, 4, 6]: mean=4, deviations=(-2,0,2), variance=8/3, stddev≈1.633
        let s = population_stddev(&[2, 4, 6]);
        assert!((s - 1.6329932).abs() < 1e-4, "got {}", s);
    }

    #[test]
    fn population_stddev_high_spread_exceeds_2() {
        // [0, 5, 10]: mean=5, variance=50/3≈16.67, stddev≈4.08
        let s = population_stddev(&[0, 5, 10]);
        assert!(s > 2.0, "stddev {} should exceed 2σ trigger", s);
    }

    // ─── aggregate() tests ───────────────────────────────────────────────

    fn mk_verdict(score: u8, model: &str) -> JudgeVerdict {
        JudgeVerdict {
            score,
            rubric_version: RUBRIC_VERSION.to_string(),
            model: model.to_string(),
            rationale: format!("rationale from {}", model),
            judged_at_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn aggregate_three_unanimous_judges() {
        let v = vec![mk_verdict(8, "a"), mk_verdict(8, "b"), mk_verdict(8, "c")];
        let agg = aggregate(v.clone(), 3, 1_700_000_000_000);
        assert_eq!(agg.aggregated.score, 8);
        assert_eq!(agg.aggregated.model, "ensemble:3");
        assert_eq!(agg.score_median, 8.0);
        assert!(agg.score_stddev.abs() < 1e-6);
        assert_eq!(agg.agreement, AgreementClass::Unanimous);
        assert_eq!(agg.judges_succeeded, 3);
        assert_eq!(agg.judges_attempted, 3);
        assert_eq!(agg.individual.len(), 3);
    }

    #[test]
    fn aggregate_three_judges_consensus_within_2sigma() {
        // [7, 8, 9]: median=8, stddev≈0.816 → Consensus (in (0, 2.0])
        let v = vec![mk_verdict(7, "a"), mk_verdict(8, "b"), mk_verdict(9, "c")];
        let agg = aggregate(v, 3, 1);
        assert_eq!(agg.aggregated.score, 8);
        assert_eq!(agg.agreement, AgreementClass::Consensus);
        assert!(agg.score_stddev > 0.0 && agg.score_stddev <= 2.0);
    }

    #[test]
    fn aggregate_three_judges_disagreement_flagged_needs_human_review() {
        // [1, 5, 10]: stddev≈3.68 > 2.0 → NeedsHumanReview
        let v = vec![mk_verdict(1, "a"), mk_verdict(5, "b"), mk_verdict(10, "c")];
        let agg = aggregate(v, 3, 1);
        assert_eq!(agg.agreement, AgreementClass::NeedsHumanReview);
        // Median still computed:
        assert_eq!(agg.aggregated.score, 5);
    }

    #[test]
    fn aggregate_partial_failure_one_of_three_succeeded_flagged() {
        // Only 1 of 3 attempted succeeded — fewer than 2 ⇒ NeedsHumanReview.
        let v = vec![mk_verdict(8, "a")];
        let agg = aggregate(v, 3, 1);
        assert_eq!(agg.agreement, AgreementClass::NeedsHumanReview);
        assert_eq!(agg.judges_succeeded, 1);
        assert_eq!(agg.judges_attempted, 3);
        assert_eq!(agg.aggregated.score, 8);
    }

    #[test]
    fn aggregate_two_of_three_with_low_spread_is_consensus_not_needs_review() {
        // 2 succeeded judges, identical scores → Unanimous (still >= 2 succeeded).
        let v = vec![mk_verdict(7, "a"), mk_verdict(7, "b")];
        let agg = aggregate(v, 3, 1);
        assert_eq!(agg.judges_succeeded, 2);
        assert_eq!(agg.agreement, AgreementClass::Unanimous);
    }

    #[test]
    fn aggregate_zero_succeeded_marks_needs_human_review_with_zero_score() {
        let agg = aggregate(vec![], 3, 1);
        assert_eq!(agg.agreement, AgreementClass::NeedsHumanReview);
        assert_eq!(agg.judges_succeeded, 0);
        assert_eq!(agg.aggregated.score, 0);
    }

    // ─── AnthropicJudge wiremock tests ───────────────────────────────────

    fn anthropic_text_response(text: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "msg_test", "type": "message", "role": "assistant",
            "model": "claude-haiku-4-5-20251001",
            "content": [{"type": "text", "text": text}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 100, "output_tokens": 20}
        })
    }

    #[tokio::test]
    async fn anthropic_judge_round_trip_returns_score_and_rationale() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "k"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(anthropic_text_response(
                    r#"{"score": 8, "rationale": "good"}"#,
                )),
            )
            .mount(&server)
            .await;

        let judge = AnthropicJudge {
            api_base: server.uri(),
            api_key: "k".into(),
            model: "claude-haiku-4-5-20251001".into(),
            timeout_secs: 5,
        };
        let (score, rationale) = judge.score("dummy prompt").await.expect("ok");
        assert_eq!(score, 8);
        assert_eq!(rationale, "good");
        assert!(judge.model_id().contains("claude-haiku"));
    }

    #[tokio::test]
    async fn anthropic_judge_returns_schema_error_on_prose_reply() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(anthropic_text_response("just prose, no json here")),
            )
            .mount(&server)
            .await;

        let judge = AnthropicJudge {
            api_base: server.uri(),
            api_key: "k".into(),
            model: "claude-haiku-4-5-20251001".into(),
            timeout_secs: 5,
        };
        let err = judge.score("x").await.unwrap_err();
        assert!(
            matches!(err, JudgeError::Schema(_)),
            "expected schema error, got {:?}",
            err
        );
    }

    // ─── OpenAICompatJudge wiremock tests ────────────────────────────────

    fn openai_chat_response(text: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "cmpl-test", "object": "chat.completion", "created": 0,
            "model": "mistral-small-latest",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": text},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120}
        })
    }

    #[tokio::test]
    async fn openai_compat_judge_extracts_score_from_chat_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(openai_chat_response(r#"{"score": 6, "rationale": "ok"}"#)),
            )
            .mount(&server)
            .await;

        let judge = OpenAICompatJudge {
            provider_id: "mistral".into(),
            api_base: server.uri(),
            api_key: "test-key".into(),
            model: "mistral-small-latest".into(),
            timeout_secs: 5,
        };
        let (score, rationale) = judge.score("dummy").await.expect("ok");
        assert_eq!(score, 6);
        assert_eq!(rationale, "ok");
        assert_eq!(judge.model_id(), "mistral-small-latest@mistral");
    }

    #[tokio::test]
    async fn openai_compat_judge_http_500_returns_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal"))
            .mount(&server)
            .await;

        let judge = OpenAICompatJudge {
            provider_id: "xai".into(),
            api_base: server.uri(),
            api_key: "k".into(),
            model: "grok-4".into(),
            timeout_secs: 5,
        };
        let err = judge.score("x").await.unwrap_err();
        assert!(
            matches!(err, JudgeError::Http(_)),
            "expected Http error, got {:?}",
            err
        );
    }

    // ─── EnsembleCurator orchestration tests ─────────────────────────────

    #[tokio::test]
    async fn ensemble_three_anthropic_unanimous_persists_to_checkpoint() {
        let s1 = MockServer::start().await;
        let s2 = MockServer::start().await;
        let s3 = MockServer::start().await;
        for s in [&s1, &s2, &s3] {
            Mock::given(method("POST"))
                .and(path("/v1/messages"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(anthropic_text_response(
                        r#"{"score": 8, "rationale": "all good"}"#,
                    )),
                )
                .mount(s)
                .await;
        }
        let curator = EnsembleCurator {
            judges: vec![
                std::sync::Arc::new(AnthropicJudge {
                    api_base: s1.uri(),
                    api_key: "k1".into(),
                    model: "claude-haiku-4-5-20251001".into(),
                    timeout_secs: 5,
                }),
                std::sync::Arc::new(AnthropicJudge {
                    api_base: s2.uri(),
                    api_key: "k2".into(),
                    model: "claude-haiku-4-5-20251001".into(),
                    timeout_secs: 5,
                }),
                std::sync::Arc::new(AnthropicJudge {
                    api_base: s3.uri(),
                    api_key: "k3".into(),
                    model: "claude-haiku-4-5-20251001".into(),
                    timeout_secs: 5,
                }),
            ],
        };
        let mut ck = EvolveCheckpoint::new("test", "check", "n");
        let outcome = curator.judge_ensemble(&mut ck).await;
        let e = ck
            .judge_ensemble
            .as_ref()
            .expect("ensemble verdict persisted");
        assert_eq!(e.aggregated.score, 8);
        assert_eq!(e.agreement, AgreementClass::Unanimous);
        assert_eq!(e.judges_succeeded, 3);
        assert_eq!(
            outcome
                .per_judge_results
                .iter()
                .filter(|r| r.is_ok())
                .count(),
            3
        );
    }

    #[tokio::test]
    async fn ensemble_three_judges_disagreement_marks_needs_human_review() {
        let s1 = MockServer::start().await;
        let s2 = MockServer::start().await;
        let s3 = MockServer::start().await;
        let payloads = ["1", "5", "10"]; // wildly different
        for (s, p) in [&s1, &s2, &s3].into_iter().zip(payloads) {
            let body = format!(r#"{{"score": {}, "rationale": "x"}}"#, p);
            Mock::given(method("POST"))
                .and(path("/v1/messages"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(anthropic_text_response(&body)),
                )
                .mount(s)
                .await;
        }
        let curator = EnsembleCurator {
            judges: vec![
                std::sync::Arc::new(AnthropicJudge {
                    api_base: s1.uri(),
                    api_key: "k".into(),
                    model: "claude-haiku-4-5-20251001".into(),
                    timeout_secs: 5,
                }),
                std::sync::Arc::new(AnthropicJudge {
                    api_base: s2.uri(),
                    api_key: "k".into(),
                    model: "claude-haiku-4-5-20251001".into(),
                    timeout_secs: 5,
                }),
                std::sync::Arc::new(AnthropicJudge {
                    api_base: s3.uri(),
                    api_key: "k".into(),
                    model: "claude-haiku-4-5-20251001".into(),
                    timeout_secs: 5,
                }),
            ],
        };
        let mut ck = EvolveCheckpoint::new("t", "check", "n");
        curator.judge_ensemble(&mut ck).await;
        let e = ck.judge_ensemble.as_ref().unwrap();
        assert_eq!(e.agreement, AgreementClass::NeedsHumanReview);
        assert_eq!(e.judges_succeeded, 3);
        assert!(e.score_stddev > 2.0);
    }

    #[tokio::test]
    async fn ensemble_schema_violation_in_one_judge_does_not_taint_others() {
        let s1 = MockServer::start().await;
        let s2 = MockServer::start().await;
        let s3 = MockServer::start().await;
        // s2 returns a non-JSON reply → schema rejection.
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(anthropic_text_response(
                    r#"{"score": 7, "rationale": "ok"}"#,
                )),
            )
            .mount(&s1)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(anthropic_text_response(
                    "the model wrote prose instead of json",
                )),
            )
            .mount(&s2)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(anthropic_text_response(
                    r#"{"score": 8, "rationale": "ok"}"#,
                )),
            )
            .mount(&s3)
            .await;

        let curator = EnsembleCurator {
            judges: vec![
                std::sync::Arc::new(AnthropicJudge {
                    api_base: s1.uri(),
                    api_key: "k".into(),
                    model: "claude-haiku-4-5-20251001".into(),
                    timeout_secs: 5,
                }),
                std::sync::Arc::new(AnthropicJudge {
                    api_base: s2.uri(),
                    api_key: "k".into(),
                    model: "claude-haiku-4-5-20251001".into(),
                    timeout_secs: 5,
                }),
                std::sync::Arc::new(AnthropicJudge {
                    api_base: s3.uri(),
                    api_key: "k".into(),
                    model: "claude-haiku-4-5-20251001".into(),
                    timeout_secs: 5,
                }),
            ],
        };
        let mut ck = EvolveCheckpoint::new("t", "check", "n");
        let outcome = curator.judge_ensemble(&mut ck).await;
        let e = ck.judge_ensemble.as_ref().unwrap();
        assert_eq!(e.judges_attempted, 3);
        assert_eq!(e.judges_succeeded, 2);
        // 2 succeeded with low spread (7,8) → stddev > 0 → Consensus.
        assert_eq!(e.agreement, AgreementClass::Consensus);
        let schema_failures = outcome
            .per_judge_results
            .iter()
            .filter(|r| matches!(r, Err(JudgeError::Schema(_))))
            .count();
        assert_eq!(schema_failures, 1);
    }

    #[tokio::test]
    async fn ensemble_all_fail_marks_needs_human_review_score_zero() {
        let s1 = MockServer::start().await;
        let s2 = MockServer::start().await;
        let s3 = MockServer::start().await;
        for s in [&s1, &s2, &s3] {
            Mock::given(method("POST"))
                .and(path("/v1/messages"))
                .respond_with(ResponseTemplate::new(500).set_body_string("nope"))
                .mount(s)
                .await;
        }
        let curator = EnsembleCurator {
            judges: vec![
                std::sync::Arc::new(AnthropicJudge {
                    api_base: s1.uri(),
                    api_key: "k".into(),
                    model: "claude-haiku-4-5-20251001".into(),
                    timeout_secs: 5,
                }),
                std::sync::Arc::new(AnthropicJudge {
                    api_base: s2.uri(),
                    api_key: "k".into(),
                    model: "claude-haiku-4-5-20251001".into(),
                    timeout_secs: 5,
                }),
                std::sync::Arc::new(AnthropicJudge {
                    api_base: s3.uri(),
                    api_key: "k".into(),
                    model: "claude-haiku-4-5-20251001".into(),
                    timeout_secs: 5,
                }),
            ],
        };
        let mut ck = EvolveCheckpoint::new("t", "check", "n");
        curator.judge_ensemble(&mut ck).await;
        let e = ck.judge_ensemble.as_ref().unwrap();
        assert_eq!(e.judges_attempted, 3);
        assert_eq!(e.judges_succeeded, 0);
        assert_eq!(e.aggregated.score, 0);
        assert_eq!(e.agreement, AgreementClass::NeedsHumanReview);
    }

    #[tokio::test]
    async fn ensemble_dispatches_judges_concurrently_not_sequentially() {
        use std::time::Instant;
        // Each mock delays response by 600ms. Sequential 3-judge would be ~1.8s,
        // concurrent should complete in roughly one 600ms slot (+ overhead).
        let s1 = MockServer::start().await;
        let s2 = MockServer::start().await;
        let s3 = MockServer::start().await;
        for s in [&s1, &s2, &s3] {
            Mock::given(method("POST"))
                .and(path("/v1/messages"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_delay(std::time::Duration::from_millis(600))
                        .set_body_json(anthropic_text_response(
                            r#"{"score": 5, "rationale": "x"}"#,
                        )),
                )
                .mount(s)
                .await;
        }
        let curator = EnsembleCurator {
            judges: vec![
                std::sync::Arc::new(AnthropicJudge {
                    api_base: s1.uri(),
                    api_key: "k".into(),
                    model: "claude-haiku-4-5-20251001".into(),
                    timeout_secs: 5,
                }),
                std::sync::Arc::new(AnthropicJudge {
                    api_base: s2.uri(),
                    api_key: "k".into(),
                    model: "claude-haiku-4-5-20251001".into(),
                    timeout_secs: 5,
                }),
                std::sync::Arc::new(AnthropicJudge {
                    api_base: s3.uri(),
                    api_key: "k".into(),
                    model: "claude-haiku-4-5-20251001".into(),
                    timeout_secs: 5,
                }),
            ],
        };
        let mut ck = EvolveCheckpoint::new("t", "check", "n");
        let t0 = Instant::now();
        curator.judge_ensemble(&mut ck).await;
        let elapsed = t0.elapsed();
        // Generous bound: 1.2s allows for slow CI; sequential would be >=1.8s.
        assert!(
            elapsed.as_millis() < 1200,
            "expected concurrent dispatch, took {} ms",
            elapsed.as_millis()
        );
    }
}
