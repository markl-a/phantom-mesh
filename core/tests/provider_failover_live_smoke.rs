//! P0-5 — OPT-IN live smoke. NOT run in CI (every test is `#[ignore]` AND
//! gated on `PHANTOM_LIVE_SMOKE=1`). Run manually:
//!   PHANTOM_LIVE_SMOKE=1 GROQ_API_KEY=sk-... \
//!     cargo test -p phantom-mesh --test provider_failover_live_smoke -- --ignored --nocapture
//!
//! Confirms a real upstream's HTTP status maps through `classify_error` →
//! `classify_failure` to the expected failover decision. Hermetic unit tests
//! own correctness; this only catches drift in a live endpoint's error shape.

use phantom_mesh::providers::circuit_breaker::{classify_failure, FailureKind};
use phantom_mesh::providers::traits::classify_error;

fn smoke_enabled() -> bool {
    std::env::var("PHANTOM_LIVE_SMOKE").map(|v| v == "1").unwrap_or(false)
}

#[tokio::test]
#[ignore = "opt-in live smoke; set PHANTOM_LIVE_SMOKE=1 + GROQ_API_KEY"]
async fn live_groq_bad_key_is_classified_failover() {
    if !smoke_enabled() {
        eprintln!("skipped: PHANTOM_LIVE_SMOKE != 1");
        return;
    }
    // Deliberately send a bogus key → expect 401 → AuthError → Failover.
    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .header("Authorization", "Bearer sk-definitely-invalid-key")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": "llama-3.1-8b-instant",
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "ping"}]
        }))
        .send()
        .await
        .expect("network reachable");
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    let err = classify_error(status, &body);
    let decision = classify_failure(&err);
    eprintln!("live groq bad-key: HTTP {status} → {err:?} → {decision:?}");
    assert_eq!(
        decision,
        FailureKind::Failover,
        "a 401 from a bad key must be a Failover decision (don't retry/open breaker)"
    );
}
