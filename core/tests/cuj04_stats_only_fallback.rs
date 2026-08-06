//! CUJ-04 #142 — the coach daily review must DEGRADE to a stats-only outcome
//! when the entire provider fallback chain fails, rather than propagating the
//! error and blocking the review.
//!
//! Covers MAC-CUJ04-OFF-002 / MAC-CUJ04-LLM-001 / MAC-CUJ02-PROV-006:
//! "all providers down → the review still completes (status = Degraded), exits
//! Ok, the aggregated stats brief is preserved, and no next_action is
//! fabricated."
//!
//! Fully offline + hermetic: it points the agents.toml fallback chain at a
//! single provider whose API key is absent, so the provider call fails at
//! `resolve_api_key` BEFORE any HTTP client is built. That surfaces as
//! `ProviderError::FallbackExhausted` → `CoachError::LlmAllProvidersFailed` →
//! the engine's degraded branch.

use spectyn_mesh::coach_wire::{run_daily_review, DailyReviewRequest, ReviewStatus};

mod harness {
    use std::sync::Mutex;

    // Serialise tests that mutate the process-global HOME + EventKey so they
    // don't race (cargo runs integration tests on threads in one binary).
    pub static ENV_LOCK: Mutex<()> = Mutex::new(());

    pub fn unique_home() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("spectyn-cuj04-{}-{}", std::process::id(), nanos()))
    }

    fn nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    /// Install a deterministic per-process EventKey from a fixed seed so the
    /// degraded review can still be age-encrypted + persisted (persist happens
    /// on BOTH the happy and the degraded path). Populates the cache that
    /// `lookup_or_derive_event_key` reads first — no on-disk identity.key needed.
    pub fn set_event_key() {
        spectyn_mesh::encryption_wire::install_event_key_from_seed(&[7u8; 32])
            .expect("install test EventKey from seed");
    }
}

#[test]
fn cuj04_all_providers_failed_degrades_to_stats_only() {
    let _guard = harness::ENV_LOCK.lock().unwrap();

    let home = harness::unique_home();
    let pm = home.join(".spectyn-mesh");
    std::fs::create_dir_all(&pm).expect("create .spectyn-mesh");
    std::fs::write(
        pm.join("agents.toml"),
        r#"
[routing]
fallback_chain = ["groq"]

[providers.groq]
default_model = "llama-3.1-8b-instant"
"#,
    )
    .expect("write agents.toml");

    std::env::set_var("HOME", &home);
    // Guarantee the key is absent → the provider call fails offline.
    std::env::remove_var("GROQ_API_KEY");
    std::env::remove_var("SPECTYN_MESH_GROQ_API_KEY");
    // EventKey present so the degraded review can be persisted.
    harness::set_event_key();

    // A date with no captured events → aggregate yields the placeholder brief;
    // the LLM step then exhausts the (keyless) provider chain.
    let req = DailyReviewRequest {
        date: "2026-05-30".to_string(),
        recall_k: 5,
    };

    let outcome = run_daily_review(&req, None)
        .expect("degraded review must return Ok, not propagate the provider failure");

    assert_eq!(
        outcome.status,
        ReviewStatus::Degraded,
        "all providers failed → status must be Degraded"
    );
    assert!(
        outcome.next_action.is_empty(),
        "degraded review must not fabricate a next_action, got {:?}",
        outcome.next_action
    );
    assert!(
        outcome.model_used.is_empty(),
        "degraded review reports no model_used, got {:?}",
        outcome.model_used
    );
    assert!(outcome.cost_usd.is_none(), "degraded review reports no cost");
    assert!(
        !outcome.events_summary.is_empty(),
        "the stats-only brief must still be present"
    );
    assert!(
        !outcome.takeaways.is_empty(),
        "aggregator takeaways must be present even when the LLM is down"
    );
    assert!(
        !outcome.event_id.is_empty(),
        "degraded review is still persisted (retryable from history)"
    );

    // Best-effort cleanup of the temp HOME.
    let _ = std::fs::remove_dir_all(&home);
}
