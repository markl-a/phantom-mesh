//! Hermetic restart e2e (apex #2, OWNED MEMORY): proves the full
//! capture->store->recall->apply loop crosses a simulated process boundary on
//! the DEFAULT production FTS5 path. This is not a duplicate of
//! `owned_memory_loop_e2e.rs`, which proves the loop without a restart, nor
//! `skill_store_persistence.rs`, which is feature-gated behind
//! `experimental-memory`, uses raw rusqlite for a COUNT check, only
//! checks presence of one skill, and never calls apply.

use spectyn_mesh::coach_wire::RecallPolicy;
use spectyn_mesh::skill_wire::{apply_skill_to_prompt, recall_skills, store_skill, Skill};

#[test]
fn owned_memory_loop_survives_process_restart() {
    // This is the ONLY test in its own integration test binary, so the
    // process-global SPECTYN_DB_PATH mutation below races nothing.
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("skills.db");
    let saved = std::env::var_os("SPECTYN_DB_PATH");
    std::env::set_var("SPECTYN_DB_PATH", &db_path);

    let target = Skill {
        id: "sk-restart-deploy".into(),
        name: "deploy the staging cluster".into(),
        trigger_pattern: "deploy staging".into(),
        steps: vec!["ssh staging".into(), "run deploy.sh".into()],
        examples: vec![],
        version: 1,
        quality_score: 0.8,
        last_applied_at: 0,
        source_event_count: 3,
    };

    let distractor = Skill {
        id: "sk-restart-rotate".into(),
        name: "rotate the vault secrets".into(),
        trigger_pattern: "rotate secrets".into(),
        steps: vec![],
        examples: vec![],
        version: 1,
        quality_score: 0.5,
        last_applied_at: 0,
        source_event_count: 2,
    };

    let outcome = (|| -> Result<Outcome, String> {
        // PHASE 1 ("process 1"): each public store call opens and drops its own
        // connection, so no handle is alive when this phase ends.
        store_skill(&target).map_err(|e| format!("store target: {e:?}"))?;
        store_skill(&distractor).map_err(|e| format!("store distractor: {e:?}"))?;

        // PHASE 2 ("process 2 / after restart"): recall opens a brand-new
        // connection from SPECTYN_DB_PATH, then apply renders what survived.
        let recall = recall_skills("deploy staging", RecallPolicy::default())
            .map_err(|e| format!("recall: {e:?}"))?;
        let top_id = recall.skills.first().map(|s| s.id.clone());
        let distractor_present = recall.skills.iter().any(|s| s.id == "sk-restart-rotate");
        let applied = apply_skill_to_prompt("<task/>", &recall.skills);

        Ok(Outcome {
            top_id,
            distractor_present,
            applied_contains_skill: applied.contains("deploy the staging cluster"),
            applied_preserves_prompt: applied.contains("<task/>"),
        })
    })();

    match saved {
        Some(value) => std::env::set_var("SPECTYN_DB_PATH", value),
        None => std::env::remove_var("SPECTYN_DB_PATH"),
    }

    let o = outcome.expect("owned-memory loop survives process restart");
    assert_eq!(
        o.top_id.as_deref(),
        Some("sk-restart-deploy"),
        "the owned skill is recalled TOP-1 after restart via a fresh connection"
    );
    assert!(
        !o.distractor_present,
        "the distractor must not be recalled for the target trigger"
    );
    assert!(
        o.applied_contains_skill,
        "apply must render the recalled skill post-restart"
    );
    assert!(
        o.applied_preserves_prompt,
        "apply must preserve the original prompt across the whole loop"
    );
}

struct Outcome {
    top_id: Option<String>,
    distractor_present: bool,
    applied_contains_skill: bool,
    applied_preserves_prompt: bool,
}
