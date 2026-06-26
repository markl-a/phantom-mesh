//! Hermetic proof that skill application prepends recall context without rewriting the user's prompt.

use phantom_mesh::skill_wire::{apply_skill_to_prompt, Skill};

#[test]
fn apply_skill_to_prompt_preserves_prompt_under_repeated_application() {
    let skills = vec![Skill {
        id: "sk-apply-1".into(),
        name: "deploy the staging cluster".into(),
        trigger_pattern: "deploy staging".into(),
        steps: vec!["ssh staging".into()],
        examples: vec![],
        version: 1,
        quality_score: 0.8,
        last_applied_at: 0,
        source_event_count: 3,
    }];
    let prompt = "USER_TASK_SENTINEL_7f3a: refactor the auth module";

    let once = apply_skill_to_prompt(prompt, &skills);
    let twice = apply_skill_to_prompt(&once, &skills);

    assert!(
        once.ends_with(prompt),
        "apply must preserve the original prompt verbatim as a suffix"
    );
    assert!(
        twice.ends_with(&once),
        "re-applying must preserve the prior prompt content as a suffix"
    );
    assert!(
        twice.ends_with(prompt),
        "re-applying must preserve the original prompt verbatim as a suffix"
    );
    assert_eq!(
        twice.matches("USER_TASK_SENTINEL_7f3a").count(),
        1,
        "the user prompt sentinel must not be duplicated or corrupted"
    );
    assert!(
        once.contains("deploy the staging cluster"),
        "apply must inject recalled skill content"
    );
    assert_eq!(
        apply_skill_to_prompt(prompt, &skills),
        apply_skill_to_prompt(prompt, &skills),
        "apply must be deterministic for identical inputs"
    );
}
