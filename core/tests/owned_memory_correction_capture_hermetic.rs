//! Hermetic round-trip (apex ②, OWNED MEMORY — capture-after-correction,
//! Option A): the slice-1 loop closes in one process — a denied tool is captured
//! into the hand-off queue (`capture_correction`), drained to the SPEC-25
//! `skills` table (`drain_corrections_to_store`), and recalled back via the REAL
//! public `recall_skills` FTS5 path — against a temp SQLite DB, NO network, NO
//! live model. The recalled skill's steps must name the denied tool, proving the
//! correction's content survives the store→recall trip. Modeled on
//! `owned_memory_loop_e2e.rs`.

use phantom_mesh::coach_wire::RecallPolicy;
use phantom_mesh::skill_wire::{
    capture_correction, drain_corrections_to_store, recall_skills,
};

#[test]
fn capture_drain_recall_round_trips_the_correction() {
    // Only test in this binary ⇒ the process-global PHANTOM_DB_PATH /
    // PHANTOM_OWNED_MEMORY mutations race nothing, and the process-global
    // hand-off queue starts empty (no sibling test enqueued into it).
    let db = tempfile::NamedTempFile::new().expect("temp DB file");
    let saved_db = std::env::var_os("PHANTOM_DB_PATH");
    let saved_om = std::env::var_os("PHANTOM_OWNED_MEMORY");
    std::env::set_var("PHANTOM_DB_PATH", db.path());
    std::env::remove_var("PHANTOM_OWNED_MEMORY"); // default ON

    let outcome = (|| -> Result<(usize, bool), String> {
        // (capture) the operator denies a `shell` force-push.
        capture_correction("force push to main branch", "shell", "protected branch");
        // (drain → store) Option A closes the loop in-process.
        let stored = drain_corrections_to_store().map_err(|e| format!("drain: {e:?}"))?;
        // (recall) the captured correction comes back via the FTS5 path.
        let recall = recall_skills("force push to main branch", RecallPolicy::default())
            .map_err(|e| format!("recall must not error: {e:?}"))?;
        let steps_name_tool = recall
            .skills
            .iter()
            .any(|s| s.steps.iter().any(|st| st.contains("shell")));
        Ok((stored, steps_name_tool))
    })();

    // Restore env BEFORE asserting.
    match saved_db {
        Some(v) => std::env::set_var("PHANTOM_DB_PATH", v),
        None => std::env::remove_var("PHANTOM_DB_PATH"),
    }
    match saved_om {
        Some(v) => std::env::set_var("PHANTOM_OWNED_MEMORY", v),
        None => std::env::remove_var("PHANTOM_OWNED_MEMORY"),
    }

    let (stored, steps_name_tool) = outcome.expect("capture→drain→recall round-trip");
    assert_eq!(stored, 1, "exactly one captured correction must be stored");
    assert!(
        steps_name_tool,
        "the recalled skill's steps must name the denied tool (`shell`)"
    );
}
