use phantom_mesh::coach_wire::RecallPolicy;
use phantom_mesh::skill_wire::{recall_skills, store_skill, Skill};

/// apex #2 (OWNED MEMORY) recall is relevance-RANKING, not just presence: a skill
/// whose name+trigger match the query densely outranks one that merely mentions
/// the query term once. Proven through the REAL public skill_wire API on the FTS5
/// production leg (no embedder wired in production). NOTE: this asserts the *rank
/// order* of results (a guaranteed property), not the raw `scores` vector, which
/// is a quality-boosted composite and is not contractually parallel-descending.
#[test]
fn recall_ranks_more_relevant_skill_above_less_relevant() {
    let db = tempfile::NamedTempFile::new().expect("temp DB file");
    let saved = std::env::var_os("PHANTOM_DB_PATH");
    std::env::set_var("PHANTOM_DB_PATH", db.path());

    let less = Skill {
        id: "sk-rank-less".into(),
        name: "kubernetes".into(),
        trigger_pattern: "weekly infrastructure status report meeting agenda notes".into(),
        steps: vec![],
        examples: vec![],
        version: 1,
        quality_score: 0.5,
        last_applied_at: 0,
        source_event_count: 1,
    };

    let more = Skill {
        id: "sk-rank-more".into(),
        name: "kubernetes rollout".into(),
        trigger_pattern: "kubernetes rollout restart".into(),
        steps: vec![],
        examples: vec![],
        version: 1,
        quality_score: 0.9,
        last_applied_at: 0,
        source_event_count: 2,
    };

    let outcome = (|| -> Result<Outcome, String> {
        store_skill(&less).map_err(|e| format!("store less: {e:?}"))?;
        store_skill(&more).map_err(|e| format!("store more: {e:?}"))?;

        let result = recall_skills("kubernetes", RecallPolicy::default())
            .map_err(|e| format!("recall must not error: {e:?}"))?;

        let more_idx = result.skills.iter().position(|s| s.id == "sk-rank-more");
        let less_idx = result.skills.iter().position(|s| s.id == "sk-rank-less");
        let first_id = result.skills.first().map(|s| s.id.clone());

        Ok(Outcome {
            more_idx,
            less_idx,
            first_id,
        })
    })();

    match saved {
        Some(value) => std::env::set_var("PHANTOM_DB_PATH", value),
        None => std::env::remove_var("PHANTOM_DB_PATH"),
    }

    let outcome = outcome.expect("recall ranking through public skill_wire API");
    assert!(
        outcome.more_idx.is_some(),
        "more-relevant skill must be present in recall results"
    );
    assert!(
        outcome.less_idx.is_some(),
        "less-relevant skill must be present in recall results"
    );

    let more_idx = outcome.more_idx.expect("checked more skill presence");
    let less_idx = outcome.less_idx.expect("checked less skill presence");

    assert!(
        more_idx < less_idx,
        "more-relevant skill must rank above less-relevant skill"
    );
    assert_eq!(
        outcome.first_id.as_deref(),
        Some("sk-rank-more"),
        "more-relevant skill must be the first recall result"
    );
}

struct Outcome {
    more_idx: Option<usize>,
    less_idx: Option<usize>,
    first_id: Option<String>,
}
