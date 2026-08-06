//! Hermetic proof (apex ②, OWNED MEMORY — recall-before-run): the
//! `owned_memory_system_block` helper that `run_inner` (agent.rs) injects into
//! the system prompt actually surfaces a stored skill for the user's latest
//! message, rendered as a `<recalled_skills>` block, against a temp SQLite DB
//! with NO network and NO live model. This is the production-path proof that the
//! always-compiled recall hook turns on a plain `cargo build` (no feature flags,
//! no `AgentRuntime.skill_runtime`). Modeled on `owned_memory_loop_e2e.rs`.
//!
//! NOTE: we assert against `owned_memory_system_block(prompt)` directly rather
//! than driving the full async `run_inner` — `run_inner` requires a live
//! provider attempt to reach a result, which a hermetic test cannot stand up.
//! The wiring at agent.rs is a verbatim `push_str` of this helper's output onto
//! `system`, so proving the helper renders the block proves the injected system
//! prompt carries it.

use spectyn_mesh::skill_wire::{owned_memory_system_block, store_skill, Skill};

#[test]
fn run_inner_recall_block_surfaces_stored_skill() {
    // Only test in this binary ⇒ the process-global SPECTYN_DB_PATH mutation
    // races nothing (the in-crate env_lock is #[cfg(test)]-only, unreachable
    // from an integration crate).
    let db = tempfile::NamedTempFile::new().expect("temp DB file");
    let saved_db = std::env::var_os("SPECTYN_DB_PATH");
    let saved_om = std::env::var_os("SPECTYN_OWNED_MEMORY");
    std::env::set_var("SPECTYN_DB_PATH", db.path());
    std::env::remove_var("SPECTYN_OWNED_MEMORY"); // default ON

    let deploy = Skill {
        id: "sk-inject-deploy".into(),
        name: "deploy the staging cluster".into(),
        trigger_pattern: "deploy staging".into(),
        steps: vec!["ssh staging".into(), "run deploy.sh".into()],
        examples: vec![],
        version: 1,
        quality_score: 0.9,
        last_applied_at: 0,
        source_event_count: 3,
    };

    let outcome = (|| -> Result<(bool, bool), String> {
        store_skill(&deploy).map_err(|e| format!("store deploy skill: {e:?}"))?;
        // The same call agent.rs::run_inner makes, with `prompt` = the user's
        // latest message.
        let block = owned_memory_system_block("please deploy staging");
        Ok((
            block.contains("<recalled_skills>"),
            block.contains("deploy the staging cluster"),
        ))
    })();

    // Restore env BEFORE asserting so a panicking assert can't leak the override.
    match saved_db {
        Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
        None => std::env::remove_var("SPECTYN_DB_PATH"),
    }
    match saved_om {
        Some(v) => std::env::set_var("SPECTYN_OWNED_MEMORY", v),
        None => std::env::remove_var("SPECTYN_OWNED_MEMORY"),
    }

    let (has_wrapper, has_skill) = outcome.expect("recall block render round-trip");
    assert!(
        has_wrapper,
        "injected system block must carry the <recalled_skills> wrapper"
    );
    assert!(
        has_skill,
        "injected system block must contain the stored deploy skill"
    );
}
