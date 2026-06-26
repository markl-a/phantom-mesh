//! T28 black-box integration tests for skill Curator V2.
//!
//! Exercises EnsembleCurator through its public crate API to lock the
//! contract against future refactors. Covers the 4 mandated scenarios:
//!   1. 3-judge unanimity persists to checkpoint
//!   2. 3-judge disagreement (>2σ) flags NeedsHumanReview
//!   3. Schema-violation rejection in one judge does not taint others
//!   4. Partial failure (1 of 3 fails) graceful aggregation

#![cfg(feature = "experimental-curator")]

use phantom_mesh::evolve_checkpoint::{AgreementClass, EvolveCheckpoint};
use phantom_mesh::skillbank::curator_ensemble::{
    AnthropicJudge, EnsembleCurator, JudgeError, JudgeProvider,
};
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn anthropic_text(text: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "x", "type": "message", "role": "assistant",
        "model": "claude-haiku-4-5-20251001",
        "content": [{"type": "text", "text": text}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 1, "output_tokens": 1}
    })
}

fn mk_anthropic(uri: String) -> Arc<dyn JudgeProvider> {
    Arc::new(AnthropicJudge {
        api_base: uri,
        api_key: "k".into(),
        model: "claude-haiku-4-5-20251001".into(),
        timeout_secs: 5,
    })
}

#[tokio::test]
async fn integration_three_judge_unanimity_persists_to_checkpoint() {
    let s1 = MockServer::start().await;
    let s2 = MockServer::start().await;
    let s3 = MockServer::start().await;
    for s in [&s1, &s2, &s3] {
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(anthropic_text(r#"{"score": 9, "rationale": "all good"}"#)),
            )
            .mount(s)
            .await;
    }
    let curator = EnsembleCurator {
        judges: vec![
            mk_anthropic(s1.uri()),
            mk_anthropic(s2.uri()),
            mk_anthropic(s3.uri()),
        ],
    };
    let mut ck = EvolveCheckpoint::new("t", "check", "n");
    curator.judge_ensemble(&mut ck).await;
    let e = ck.judge_ensemble.expect("persisted");
    assert_eq!(e.aggregated.score, 9);
    assert_eq!(e.agreement, AgreementClass::Unanimous);
}

#[tokio::test]
async fn integration_three_judge_disagreement_flags_review() {
    let s1 = MockServer::start().await;
    let s2 = MockServer::start().await;
    let s3 = MockServer::start().await;
    for (s, score) in [(&s1, "1"), (&s2, "5"), (&s3, "10")] {
        let body = format!(r#"{{"score": {}, "rationale": "x"}}"#, score);
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_text(&body)))
            .mount(s)
            .await;
    }
    let curator = EnsembleCurator {
        judges: vec![
            mk_anthropic(s1.uri()),
            mk_anthropic(s2.uri()),
            mk_anthropic(s3.uri()),
        ],
    };
    let mut ck = EvolveCheckpoint::new("t", "check", "n");
    curator.judge_ensemble(&mut ck).await;
    let e = ck.judge_ensemble.expect("persisted");
    assert_eq!(e.agreement, AgreementClass::NeedsHumanReview);
    assert!(e.score_stddev > 2.0);
}

#[tokio::test]
async fn integration_schema_violation_rejected_does_not_taint_others() {
    let s1 = MockServer::start().await;
    let s2 = MockServer::start().await;
    let s3 = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(anthropic_text(r#"{"score": 7, "rationale": "ok"}"#)),
        )
        .mount(&s1)
        .await;
    // s2 sends an unknown extra field → strict parser rejects.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_text(
            r#"{"score": 8, "rationale": "ok", "verdict_color": "green"}"#,
        )))
        .mount(&s2)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(anthropic_text(r#"{"score": 7, "rationale": "ok"}"#)),
        )
        .mount(&s3)
        .await;

    let curator = EnsembleCurator {
        judges: vec![
            mk_anthropic(s1.uri()),
            mk_anthropic(s2.uri()),
            mk_anthropic(s3.uri()),
        ],
    };
    let mut ck = EvolveCheckpoint::new("t", "check", "n");
    let outcome = curator.judge_ensemble(&mut ck).await;
    let e = ck.judge_ensemble.expect("persisted");
    assert_eq!(e.judges_attempted, 3);
    assert_eq!(e.judges_succeeded, 2);
    let schema_fails = outcome
        .per_judge_results
        .iter()
        .filter(|r| matches!(r, Err(JudgeError::Schema(_))))
        .count();
    assert_eq!(schema_fails, 1);
}

#[tokio::test]
async fn integration_partial_failure_one_of_three_fails_gracefully() {
    let s1 = MockServer::start().await;
    let s2 = MockServer::start().await;
    let s3 = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(anthropic_text(r#"{"score": 7, "rationale": "ok"}"#)),
        )
        .mount(&s1)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(500).set_body_string("provider explosion"))
        .mount(&s2)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(anthropic_text(r#"{"score": 8, "rationale": "ok"}"#)),
        )
        .mount(&s3)
        .await;

    let curator = EnsembleCurator {
        judges: vec![
            mk_anthropic(s1.uri()),
            mk_anthropic(s2.uri()),
            mk_anthropic(s3.uri()),
        ],
    };
    let mut ck = EvolveCheckpoint::new("t", "check", "n");
    let outcome = curator.judge_ensemble(&mut ck).await;
    let e = ck.judge_ensemble.expect("persisted");
    assert_eq!(e.judges_attempted, 3);
    assert_eq!(e.judges_succeeded, 2);
    // 2 succeeded with [7, 8] → low spread → Consensus.
    assert_eq!(e.agreement, AgreementClass::Consensus);
    let http_fails = outcome
        .per_judge_results
        .iter()
        .filter(|r| matches!(r, Err(JudgeError::Http(_))))
        .count();
    assert_eq!(http_fails, 1);
}
