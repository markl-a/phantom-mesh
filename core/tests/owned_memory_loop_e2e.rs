//! Hermetic e2e (apex #1, OWNED MEMORY): the capture->store->recall->apply loop
//! round-trips through the REAL **public** skill_wire API — `store_skill`,
//! `recall_skills`, `apply_skill_to_prompt` — against a temp SQLite DB, with NO
//! network and NO live model. This proves the production recall surface: in
//! production there is no `ort`/MiniLM embedder wired, so `recall_skills` degrades
//! to the FTS5 keyword leg (the embedding leg `Err`s -> FTS5-only). A captured
//! skill must be stored, recalled TOP-1 for its own trigger, NOT recalled for an
//! unrelated query, and rendered into the prompt — while the original prompt is
//! preserved. CI-visible (NOT `#[ignore]`).
//!
//! The embedding/semantic leg (FixtureEmbedder via the private `set_test_embedder`
//! hook -> HybridUnion top-1) is exercised by the in-crate unit test
//! `skill_wire::tests::six_step_owned_memory_loop_round_trips_semantically`; it
//! cannot be reached from an integration crate because the hook is `fn`-private.
//! This file is the complementary public-API / production-path proof.

use spectyn_mesh::coach_wire::RecallPolicy;
use spectyn_mesh::skill_wire::{apply_skill_to_prompt, recall_skills, store_skill, Skill};

#[test]
fn owned_memory_loop_store_recall_apply_round_trips() {
    // This is the ONLY test in its own integration test binary, so the
    // process-global SPECTYN_DB_PATH mutation below races nothing (no sibling
    // thread in this process touches it). The in-crate `env_lock` mutex used by
    // the unit tests is `#[cfg(test)]`-only and intentionally not reachable here.
    let db = tempfile::NamedTempFile::new().expect("temp DB file");
    let saved = std::env::var_os("SPECTYN_DB_PATH");
    std::env::set_var("SPECTYN_DB_PATH", db.path());

    // (capture -> extract) The owned skill the operator's activity produced. Its
    // trigger/name both carry the tokens "deploy" + "staging".
    let target = Skill {
        id: "sk-omloop-deploy".into(),
        name: "deploy the staging cluster".into(),
        trigger_pattern: "deploy staging".into(),
        steps: vec!["ssh staging".into(), "run deploy.sh".into()],
        examples: vec![],
        version: 1,
        quality_score: 0.8,
        last_applied_at: 0,
        source_event_count: 3,
    };

    // A content-unrelated skill: it shares NO query token with "deploy staging",
    // so a relevance-respecting recall must NOT surface it (proves recall is
    // discriminating, not "return everything in the table").
    let distractor = Skill {
        id: "sk-omloop-rotate".into(),
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
        // (store) Both skills go through the REAL public persistence path (writes
        // the canonical row + the FTS5 mirror).
        store_skill(&target).map_err(|e| format!("store target: {e:?}"))?;
        store_skill(&distractor).map_err(|e| format!("store distractor: {e:?}"))?;

        // (recall) The captured trigger recalls the owned skill. recall_skills must
        // never error over storage state — it degrades, it does not panic.
        let recall = recall_skills("deploy staging", RecallPolicy::default())
            .map_err(|e| format!("recall must not error: {e:?}"))?;
        let top_id = recall.skills.first().map(|s| s.id.clone());
        let distractor_present = recall.skills.iter().any(|s| s.id == "sk-omloop-rotate");

        // (apply) Render the recalled skills into the prompt; the owned skill's
        // name must appear AND the original prompt must be preserved verbatim.
        let applied = apply_skill_to_prompt("<task/>", &recall.skills);

        // A query that shares NO token with either stored skill recalls nothing —
        // the loop does not leak unrelated memory into the prompt.
        let unrelated = recall_skills("compile the kernel", RecallPolicy::default())
            .map_err(|e| format!("unrelated recall must not error: {e:?}"))?;

        Ok(Outcome {
            top_id,
            distractor_present,
            applied_contains_skill: applied.contains("deploy the staging cluster"),
            applied_preserves_prompt: applied.contains("<task/>"),
            unrelated_recalls_target: unrelated.skills.iter().any(|s| s.id == "sk-omloop-deploy"),
        })
    })();

    // Restore the prior SPECTYN_DB_PATH BEFORE asserting, so a panicking assert
    // never leaks our temp path into the process for any later-linked test.
    match saved {
        Some(value) => std::env::set_var("SPECTYN_DB_PATH", value),
        None => std::env::remove_var("SPECTYN_DB_PATH"),
    }

    let o = outcome.expect("owned-memory loop store->recall->apply round-trip");
    assert_eq!(
        o.top_id.as_deref(),
        Some("sk-omloop-deploy"),
        "the captured skill must be recalled TOP-1 for its own trigger"
    );
    assert!(
        !o.distractor_present,
        "an unrelated skill must NOT be recalled for the captured trigger"
    );
    assert!(
        o.applied_contains_skill,
        "applied prompt must include the recalled target skill name"
    );
    assert!(
        o.applied_preserves_prompt,
        "applied prompt must preserve the original prompt verbatim"
    );
    assert!(
        !o.unrelated_recalls_target,
        "a content-unrelated query must NOT recall the captured skill"
    );
}

struct Outcome {
    top_id: Option<String>,
    distractor_present: bool,
    applied_contains_skill: bool,
    applied_preserves_prompt: bool,
    unrelated_recalls_target: bool,
}
