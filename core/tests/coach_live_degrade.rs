//! DRIFT-fix gate: the LIVE coach review path (`daily_review::run_coach_review`,
//! the one the CLI `spectyn coach review` + the SPEC-23 scheduler daemon
//! actually fire) must carry the SPEC-23 §8 Degraded state machine that
//! `coach_wire::run_daily_review` already has + is tested for.
//!
//! Before this fix the live path, on a no-keys / Ollama-off run, produced a
//! silent stats-only markdown footer (`## Tomorrow's one action\n\n(skipped:
//! …)`) with NO machine-readable status and NO persisted retryable row — the UI
//! / scheduler could not tell "completed" from "degraded, retry later".
//!
//! This drives the REAL live `run_coach_review` with:
//!   - no GEMINI_API_KEY / GROQ_API_KEY in env, and
//!   - Ollama disabled (OLLAMA_DISABLE=1),
//! so the provider fallback chain is empty → the review degrades. It then
//! asserts:
//!   1. `result.status == ReviewStatus::Degraded` (the unified wire status),
//!   2. `result.next_action` is empty (no fabricated action), and
//!   3. a persisted, retryable `coach_review` row exists for the date in the
//!      EventStore (queryable from history — the same row the wire path writes).
//!
//! Fully offline + hermetic: HOME is redirected to a unique tempdir and a
//! deterministic EventKey is installed from a fixed seed so the degraded review
//! can be age-encrypted + persisted (persist happens on the degraded path too).

use spectyn_mesh::coach_wire::ReviewStatus;
use spectyn_mesh::event_storage_wire::{query_events, EventStoreQuery};
use spectyn_mesh::life_node::daily_review::run_coach_review;

mod harness {
    use std::sync::Mutex;

    // Serialise tests that mutate process-global HOME / env / EventKey so they
    // don't race (cargo runs integration tests on threads in one binary).
    pub static ENV_LOCK: Mutex<()> = Mutex::new(());

    pub fn unique_home() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "spectyn-coach-live-degrade-{}-{}",
            std::process::id(),
            nanos()
        ))
    }

    fn nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    /// Install a deterministic per-process EventKey from a fixed seed so the
    /// degraded review can be age-encrypted + persisted. Populates the cache
    /// `lookup_or_derive_event_key` reads first — no on-disk identity.key needed.
    pub fn set_event_key() {
        spectyn_mesh::encryption_wire::install_event_key_from_seed(&[9u8; 32])
            .expect("install test EventKey from seed");
    }
}

#[tokio::test]
async fn coach_live_review_no_providers_degrades_and_persists_retryable_row() {
    let _guard = harness::ENV_LOCK.lock().unwrap();

    let home = harness::unique_home();
    let pm = home.join(".spectyn-mesh");
    std::fs::create_dir_all(pm.join("events")).expect("create .spectyn-mesh/events");

    // Redirect HOME (and USERPROFILE for Windows) so the EventStore root +
    // reviews dir land in the tempdir, not the real ~/.spectyn-mesh.
    let saved_home = std::env::var("HOME").ok();
    let saved_userprofile = std::env::var("USERPROFILE").ok();
    let saved_spectyn_home = std::env::var("SPECTYN_HOME").ok();
    let saved_gemini = std::env::var("GEMINI_API_KEY").ok();
    let saved_groq = std::env::var("GROQ_API_KEY").ok();
    let saved_ollama = std::env::var("OLLAMA_DISABLE").ok();

    std::env::set_var("HOME", &home);
    std::env::set_var("USERPROFILE", &home);
    std::env::set_var("SPECTYN_HOME", &pm);
    // No provider keys + Ollama off → the live provider fallback chain is EMPTY,
    // which is exactly the "no-keys / Ollama unreachable" degraded trigger.
    std::env::remove_var("GEMINI_API_KEY");
    std::env::remove_var("GROQ_API_KEY");
    std::env::set_var("OLLAMA_DISABLE", "1");
    // EventKey present so the degraded review row can be persisted.
    harness::set_event_key();

    let date = "2026-05-31";

    // Drive the REAL live path — same fn the CLI + scheduler fire. save=false,
    // no partner deps (the no-LLM test path).
    let result = run_coach_review(&home, date, false, None)
        .await
        .expect("degraded live review must return Ok, not propagate a hard error");

    // 1. The unified SPEC-23 §8 status must be Degraded — no provider answered.
    assert_eq!(
        result.status,
        ReviewStatus::Degraded,
        "no provider keys + Ollama off → live review status must be Degraded"
    );

    // 2. No fabricated action.
    assert!(
        result.next_action.is_empty(),
        "degraded live review must not fabricate a next_action, got {:?}",
        result.next_action
    );

    // The stats-only brief must still be present (the always-available artefact).
    assert!(
        result.markdown.contains("# Daily review — 2026-05-31"),
        "the stats-only brief must still be produced; got:\n{}",
        result.markdown
    );

    // 3. A persisted, retryable `coach_review` row exists for the date — the
    //    same EventStore-tagged row the wire path writes, queryable from
    //    history. The persisted row is stamped with `now()` as its timestamp but
    //    carries a `date:<YYYY-MM-DD>` tag for the reviewed date, so we filter on
    //    the `coach_review` tag and then assert the per-date marker is present
    //    (a date_iso filter would compare the now()-timestamp, not the tag).
    let rows = query_events(&EventStoreQuery {
        date_iso: None,
        kind: None,
        tag: Some("coach_review".to_string()),
        limit: None,
        offset: None,
    })
    .expect("query coach_review rows");
    let date_tag = format!("date:{date}");
    assert!(
        rows.iter()
            .any(|r| r.meta.tags.iter().any(|t| t == &date_tag)),
        "a degraded live review must persist a retryable coach_review row tagged \
         {date_tag} (rows found: {:?})",
        rows.iter().map(|r| &r.meta.tags).collect::<Vec<_>>()
    );

    // Restore env so a failed assert above can't leak state into sibling tests.
    let restore = |k: &str, v: Option<String>| match v {
        Some(v) => std::env::set_var(k, v),
        None => std::env::remove_var(k),
    };
    restore("HOME", saved_home);
    restore("USERPROFILE", saved_userprofile);
    restore("SPECTYN_HOME", saved_spectyn_home);
    restore("GEMINI_API_KEY", saved_gemini);
    restore("GROQ_API_KEY", saved_groq);
    restore("OLLAMA_DISABLE", saved_ollama);

    let _ = std::fs::remove_dir_all(&home);
}
