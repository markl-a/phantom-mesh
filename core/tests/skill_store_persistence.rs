//! P0-2 Task 3 — `skills` store durability across a simulated spectyn restart.
//!
//! The STORE half of the apex ②owned-memory loop must be DURABLE: a skill
//! stored in one "process" must survive a restart. Because
//! `skill_wire::store_skill` opens a fresh `Connection` per call and lets it
//! drop (no in-memory `:memory:` handle, no shared `Arc<Mutex<Connection>>`),
//! every write is already flushed to the on-disk SQLite file. A "restart" is
//! therefore: write the skill, keep NO handle, then re-open the same on-disk DB
//! through a brand-new connection / a fresh `recall_skills` call and confirm the
//! skill is still found.
//!
//! Gated `#![cfg(feature = "experimental-memory")]` so
//! `cargo test --no-default-features` compiles this file empty and skips it
//! cleanly — the same convention `skill_rpc_skills.rs` uses (integration tests
//! are auto-discovered; there is no `[[test]]` stanza for them).

#![cfg(feature = "experimental-memory")]

use spectyn_mesh::coach_wire::RecallPolicy;
use spectyn_mesh::skill_wire::{recall_skills, store_skill, Skill};

// `spectyn_mesh::env_lock` is `#[cfg(test)]`-gated, so it is NOT reachable from
// an integration test (which links the non-test lib build). Serialize on a
// file-local mutex instead — SPECTYN_DB_PATH is a process-global, and this is
// the only test in this binary that touches it, so a file-local lock is
// sufficient to keep the env mutation hermetic.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Store a skill, drop every handle, then "restart" by recalling it through a
/// fresh connection AND reading the row count through a raw `rusqlite` handle.
/// Both must see the skill — proving the write reached the on-disk file.
#[test]
fn stored_skill_survives_a_simulated_restart() {
    // Serialize on the file-local env mutex (SPECTYN_DB_PATH is a process-global)
    // so this never races a sibling test in this binary that touches the env.
    let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    // A tempDIR (not a NamedTempFile) so the on-disk path outlives any single
    // connection — exactly the "DB file persists across a restart" condition.
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("skills.db");
    let saved = std::env::var_os("SPECTYN_DB_PATH");
    std::env::set_var("SPECTYN_DB_PATH", &db_path);

    let skill = Skill {
        id: "sk-restart-1".into(),
        name: "deploy the staging cluster".into(),
        trigger_pattern: "deploy staging".into(),
        steps: vec!["ssh staging".into(), "run deploy.sh".into()],
        examples: vec![],
        version: 1,
        quality_score: 0.8,
        last_applied_at: 0,
        source_event_count: 3,
    };

    // Wrap the DB-touching body so SPECTYN_DB_PATH is restored BEFORE asserting,
    // even on an early failure path.
    let outcome = (|| -> Result<(bool, i64), String> {
        // Phase 1 — store via the PUBLIC surface, keeping no handle afterwards.
        // The durability contract is about on-disk survival of the write, not
        // queue mechanics (Task 1 covers the hand-off queue → drain path).
        store_skill(&skill).map_err(|e| format!("store: {e:?}"))?;

        // Phase 2 — "restart": no handle from Phase 1 is alive. `recall_skills`
        // opens its OWN fresh connection from SPECTYN_DB_PATH.
        let res = recall_skills("staging", RecallPolicy::default())
            .map_err(|e| format!("recall: {e:?}"))?;
        let found = res.skills.iter().any(|s| s.id == "sk-restart-1");

        // Also confirm via a raw connection that the row physically persisted.
        let conn = rusqlite::Connection::open(&db_path).map_err(|e| format!("open: {e}"))?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM skills WHERE id = ?1",
                rusqlite::params!["sk-restart-1"],
                |r| r.get(0),
            )
            .map_err(|e| format!("count: {e}"))?;
        Ok((found, count))
    })();

    // Restore env BEFORE asserting so a failure can't leak the override.
    match saved {
        Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
        None => std::env::remove_var("SPECTYN_DB_PATH"),
    }

    let (found, count) = outcome.expect("store + restart-recall round-trip");
    assert!(
        found,
        "skill must be FTS5-recallable after a simulated restart (fresh connection)"
    );
    assert_eq!(count, 1, "the skills row must physically persist on disk");
}
