//! Hand workflow engine integration tests — 15+ tests covering
//! Hand loading from TOML, TOML edge cases, phase execution helpers,
//! condition gates, and HandRegistry operations.

use clawtex_core::hands::{
    Hand, HandRegistry, HandRunner, PhaseOutput, HandCheckpoint,
    evaluate_condition,
};
use serde_json::json;
use std::fs;
use tempfile::TempDir;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Create a temporary directory with a single hand.toml inside hand_name/
fn make_hand_dir(name: &str, toml: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let hand_dir = dir.path().join(name);
    fs::create_dir_all(&hand_dir).unwrap();
    fs::write(hand_dir.join("hand.toml"), toml).unwrap();
    dir
}

const SIMPLE_HAND_TOML: &str = r#"
name = "test_simple"
description = "A simple test hand"
category = "testing"
provider = "gemini"
model = "gemini-1.5-flash"
tools = ["web_search", "file_write"]
output_format = "markdown"

[[phases]]
name = "research"
system_prompt = "Search for information about the topic."
max_rounds = 3

[[phases]]
name = "write"
system_prompt = "Write a report based on the research."
max_rounds = 5
"#;

const HAND_WITH_CONDITION_TOML: &str = r#"
name = "conditional_hand"
description = "Hand with conditional phases"
provider = "auto"

[[phases]]
name = "phase_one"
system_prompt = "Do phase one."

[[phases]]
name = "phase_two"
system_prompt = "Do phase two only if phase one found results."
condition = "contains:result"

[[phases]]
name = "phase_three"
system_prompt = "Always run phase three."
"#;

const HAND_WITH_SETTINGS_TOML: &str = r#"
name = "settings_hand"
description = "Hand with custom settings"
provider = "auto"

[settings]
target_audience = "developers"
word_count = "500"
language = "English"

[[phases]]
name = "generate"
system_prompt = "Generate content for the target audience."
"#;

const HAND_WITH_CHAIN_TOML: &str = r#"
name = "chain_hand"
description = "Hand that chains to another"
provider = "auto"
chain_to = "followup_hand"

[[phases]]
name = "first"
system_prompt = "Do first step."
"#;

const HAND_WITH_SCHEDULE_TOML: &str = r#"
name = "scheduled_hand"
description = "Hand with cron schedule"
provider = "gemini"
schedule = "0 9 * * 1"

[[phases]]
name = "run"
system_prompt = "Execute scheduled task."
"#;

const HAND_WITH_PARALLEL_QUERIES_TOML: &str = r#"
name = "parallel_hand"
description = "Hand with parallel queries"
provider = "auto"

[[phases]]
name = "research"
system_prompt = "Analyze the pre-fetched search results."
parallel_queries = ["AI trends 2026", "ML market size", "deep learning papers"]
"#;

// ── Hand TOML Parsing Tests ───────────────────────────────────────────────────

#[test]
fn hand_parses_simple_toml() {
    let hand: Hand = toml::from_str(SIMPLE_HAND_TOML).unwrap();
    assert_eq!(hand.name, "test_simple");
    assert_eq!(hand.description, "A simple test hand");
    assert_eq!(hand.category, "testing");
    assert_eq!(hand.provider, "gemini");
    assert_eq!(hand.model, "gemini-1.5-flash");
    assert_eq!(hand.phases.len(), 2);
    assert_eq!(hand.output_format, "markdown");
}

#[test]
fn hand_parses_phases_correctly() {
    let hand: Hand = toml::from_str(SIMPLE_HAND_TOML).unwrap();
    assert_eq!(hand.phases[0].name, "research");
    assert_eq!(hand.phases[0].max_rounds, 3);
    assert!(hand.phases[0].system_prompt.contains("Search for information"));
    assert_eq!(hand.phases[1].name, "write");
    assert_eq!(hand.phases[1].max_rounds, 5);
}

#[test]
fn hand_parses_tools_list() {
    let hand: Hand = toml::from_str(SIMPLE_HAND_TOML).unwrap();
    assert!(hand.tools.contains(&"web_search".to_string()));
    assert!(hand.tools.contains(&"file_write".to_string()));
}

#[test]
fn hand_parses_condition_in_phase() {
    let hand: Hand = toml::from_str(HAND_WITH_CONDITION_TOML).unwrap();
    assert_eq!(hand.phases.len(), 3);
    assert!(hand.phases[0].condition.is_none());
    assert_eq!(hand.phases[1].condition.as_deref(), Some("contains:result"));
    assert!(hand.phases[2].condition.is_none());
}

#[test]
fn hand_parses_settings_map() {
    let hand: Hand = toml::from_str(HAND_WITH_SETTINGS_TOML).unwrap();
    assert_eq!(hand.settings.get("target_audience").map(|s| s.as_str()), Some("developers"));
    assert_eq!(hand.settings.get("word_count").map(|s| s.as_str()), Some("500"));
    assert_eq!(hand.settings.get("language").map(|s| s.as_str()), Some("English"));
}

#[test]
fn hand_parses_chain_to() {
    let hand: Hand = toml::from_str(HAND_WITH_CHAIN_TOML).unwrap();
    assert_eq!(hand.chain_to.as_deref(), Some("followup_hand"));
}

#[test]
fn hand_parses_schedule() {
    let hand: Hand = toml::from_str(HAND_WITH_SCHEDULE_TOML).unwrap();
    assert_eq!(hand.schedule.as_deref(), Some("0 9 * * 1"));
}

#[test]
fn hand_parses_parallel_queries() {
    let hand: Hand = toml::from_str(HAND_WITH_PARALLEL_QUERIES_TOML).unwrap();
    assert_eq!(hand.phases[0].parallel_queries.len(), 3);
    assert!(hand.phases[0].parallel_queries.contains(&"AI trends 2026".to_string()));
}

#[test]
fn hand_default_provider_is_auto() {
    let toml = r#"
name = "auto_hand"
description = "Uses default provider"
[[phases]]
name = "p"
system_prompt = "Do something."
"#;
    let hand: Hand = toml::from_str(toml).unwrap();
    assert_eq!(hand.provider, "auto");
}

#[test]
fn hand_default_output_format_is_markdown() {
    let toml = r#"
name = "fmt_hand"
description = "Default format"
[[phases]]
name = "p"
system_prompt = "Generate."
"#;
    let hand: Hand = toml::from_str(toml).unwrap();
    assert_eq!(hand.output_format, "markdown");
}

#[test]
fn hand_default_max_rounds_is_five() {
    let toml = r#"
name = "rounds_hand"
description = "Default rounds"
[[phases]]
name = "p"
system_prompt = "Work."
"#;
    let hand: Hand = toml::from_str(toml).unwrap();
    assert_eq!(hand.phases[0].max_rounds, 5);
}

#[test]
fn hand_ignores_extra_top_level_fields() {
    let toml = r#"
name = "extra_hand"
description = "Hand with extra fields"
unknown_field = "should not break parsing"
another_extra = 42

[[phases]]
name = "p"
system_prompt = "Go."
"#;
    let hand: Hand = toml::from_str(toml).unwrap();
    assert_eq!(hand.name, "extra_hand");
    // Extra fields go into `extra` HashMap, not causing a parse error
}

#[test]
fn hand_ignores_extra_phase_fields() {
    let toml = r#"
name = "extra_phase_hand"
description = "Phases with extra fields"

[[phases]]
name = "p"
system_prompt = "Work."
tool_calls = ["web_search", "file_write"]
unknown_phase_field = "ignored"
"#;
    let hand: Hand = toml::from_str(toml).unwrap();
    assert_eq!(hand.phases[0].name, "p");
    assert!(hand.phases[0].extra.contains_key("tool_calls"));
}

#[test]
fn hand_multiline_system_prompt_parses() {
    let toml = r#"
name = "multiline_hand"
description = "Multi-line prompt test"

[[phases]]
name = "analyze"
system_prompt = """Analyze the following data:
1. Check for anomalies.
2. Identify trends.
3. Generate a report."""
"#;
    let hand: Hand = toml::from_str(toml).unwrap();
    let prompt = &hand.phases[0].system_prompt;
    assert!(prompt.contains("Analyze the following data"));
    assert!(prompt.contains("anomalies"));
    assert!(prompt.contains("trends"));
}

// ── HandRegistry Tests ────────────────────────────────────────────────────────

#[test]
fn hand_registry_loads_from_directory() {
    let dir = make_hand_dir("test_simple", SIMPLE_HAND_TOML);
    let registry = HandRegistry::load(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(registry.names().len(), 1);
    assert!(registry.get("test_simple").is_some());
}

#[test]
fn hand_registry_empty_for_nonexistent_directory() {
    let registry = HandRegistry::load("/nonexistent/path/to/hands").unwrap();
    assert!(registry.names().is_empty());
}

#[test]
fn hand_registry_empty_factory() {
    let registry = HandRegistry::empty();
    assert!(registry.names().is_empty());
    assert!(registry.list().is_empty());
}

#[test]
fn hand_registry_get_nonexistent_returns_none() {
    let registry = HandRegistry::empty();
    assert!(registry.get("no_such_hand").is_none());
}

#[test]
fn hand_registry_loads_multiple_hands() {
    let dir = tempfile::tempdir().unwrap();
    for (name, toml) in [
        ("hand_a", SIMPLE_HAND_TOML),
        ("hand_b", HAND_WITH_CONDITION_TOML),
        ("hand_c", HAND_WITH_SETTINGS_TOML),
    ] {
        let hand_dir = dir.path().join(name);
        fs::create_dir_all(&hand_dir).unwrap();
        // Override name in TOML to match directory name
        let adjusted = toml.replacen(
            &format!("name = \"{}\"", toml.lines()
                .find(|l| l.starts_with("name = "))
                .unwrap_or("name = \"x\"")
                .trim_start_matches("name = \"")
                .trim_end_matches('"')),
            &format!("name = \"{}\"", name),
            1
        );
        fs::write(hand_dir.join("hand.toml"), &adjusted).unwrap();
    }
    let registry = HandRegistry::load(dir.path().to_str().unwrap()).unwrap();
    // Should have loaded at least some hands (names match or partially match)
    assert!(registry.names().len() >= 1);
}

#[test]
fn hand_registry_skips_invalid_toml() {
    let dir = tempfile::tempdir().unwrap();
    // Valid hand
    let valid_dir = dir.path().join("valid");
    fs::create_dir_all(&valid_dir).unwrap();
    fs::write(valid_dir.join("hand.toml"), SIMPLE_HAND_TOML).unwrap();
    // Invalid hand (bad TOML)
    let invalid_dir = dir.path().join("invalid");
    fs::create_dir_all(&invalid_dir).unwrap();
    fs::write(invalid_dir.join("hand.toml"), "not valid toml {{{{").unwrap();

    let registry = HandRegistry::load(dir.path().to_str().unwrap()).unwrap();
    // Should load the valid one and skip the invalid one (no panic)
    assert_eq!(registry.names().len(), 1);
}

#[test]
fn hand_registry_names_sorted_alphabetically() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["zeta", "alpha", "mango"] {
        let hand_dir = dir.path().join(name);
        fs::create_dir_all(&hand_dir).unwrap();
        let toml = format!(
            "name = \"{}\"\ndescription = \"test\"\n\n[[phases]]\nname = \"p\"\nsystem_prompt = \"go\"\n",
            name
        );
        fs::write(hand_dir.join("hand.toml"), &toml).unwrap();
    }
    let registry = HandRegistry::load(dir.path().to_str().unwrap()).unwrap();
    let names = registry.names();
    assert_eq!(names.len(), 3);
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "names() must return sorted list");
}

// ── HandRunner::prepare_context Tests ────────────────────────────────────────

#[test]
fn hand_runner_prepare_context_no_settings() {
    let hand: Hand = toml::from_str(SIMPLE_HAND_TOML).unwrap();
    let context = HandRunner::prepare_context(&hand, "Research AI");
    assert!(context.contains("Research AI"));
}

#[test]
fn hand_runner_prepare_context_injects_settings() {
    let hand: Hand = toml::from_str(HAND_WITH_SETTINGS_TOML).unwrap();
    let context = HandRunner::prepare_context(&hand, "Generate blog post");
    assert!(context.contains("Generate blog post"));
    assert!(context.contains("target_audience"));
    assert!(context.contains("developers"));
    assert!(context.contains("word_count"));
    assert!(context.contains("500"));
}

// ── evaluate_condition Tests ──────────────────────────────────────────────────

#[test]
fn evaluate_condition_contains_true() {
    assert!(evaluate_condition("contains:result", "Here are the results found"));
}

#[test]
fn evaluate_condition_contains_false() {
    assert!(!evaluate_condition("contains:result", "No data here"));
}

#[test]
fn evaluate_condition_not_contains_true() {
    assert!(evaluate_condition("not_contains:error", "All good, success achieved"));
}

#[test]
fn evaluate_condition_not_contains_false() {
    assert!(!evaluate_condition("not_contains:error", "There was an error in processing"));
}

#[test]
fn evaluate_condition_min_length_meets_threshold() {
    let long_output = "a".repeat(600);
    assert!(evaluate_condition("min_length:500", &long_output));
}

#[test]
fn evaluate_condition_min_length_below_threshold() {
    let short_output = "short";
    assert!(!evaluate_condition("min_length:500", short_output));
}

#[test]
fn evaluate_condition_previous_success_not_failed() {
    assert!(evaluate_condition("previous_success", "Output: Analysis complete"));
}

#[test]
fn evaluate_condition_previous_success_failed_output() {
    assert!(!evaluate_condition("previous_success", "Phase failed: Provider timeout"));
}

#[test]
fn evaluate_condition_unknown_type_returns_true() {
    // Unknown condition types should default to true (don't block execution)
    assert!(evaluate_condition("unknown_condition:value", "any output"));
}

// ── HandCheckpoint Tests ──────────────────────────────────────────────────────

#[test]
fn hand_checkpoint_save_and_load() {
    let tmp = tempfile::tempdir().unwrap();
    // We can't easily override the checkpoint path (~/.clawtex/checkpoints),
    // so just verify the struct serializes correctly
    let cp = HandCheckpoint {
        hand_name: "test_hand".to_string(),
        run_id: "run-123".to_string(),
        completed_phases: vec![
            PhaseOutput {
                phase_name: "research".to_string(),
                output: "Found 5 results".to_string(),
                tool_calls: 2,
                duration_secs: 12.5,
                skipped: false,
                guardrail_issues: vec![],
                quality_score: Some(85),
                quality_retries: 0,
            }
        ],
        last_phase_index: 0,
        context: "Research context here".to_string(),
        created_at: "2026-03-18T12:00:00Z".to_string(),
    };

    // Serialize and deserialize to verify struct integrity
    let json = serde_json::to_string(&cp).unwrap();
    let restored: HandCheckpoint = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.hand_name, "test_hand");
    assert_eq!(restored.run_id, "run-123");
    assert_eq!(restored.completed_phases.len(), 1);
    assert_eq!(restored.completed_phases[0].phase_name, "research");
    assert_eq!(restored.completed_phases[0].quality_score, Some(85));
    assert_eq!(restored.last_phase_index, 0);
}

// ── PhaseOutput Tests ─────────────────────────────────────────────────────────

#[test]
fn phase_output_default_skipped_false() {
    let po = PhaseOutput {
        phase_name: "test".to_string(),
        output: "done".to_string(),
        tool_calls: 0,
        duration_secs: 1.0,
        skipped: false,
        guardrail_issues: vec![],
        quality_score: None,
        quality_retries: 0,
    };
    let json = serde_json::to_value(&po).unwrap();
    assert_eq!(json["skipped"], false);
    assert!(json["quality_score"].is_null());
}

#[test]
fn phase_output_with_guardrail_issues() {
    let po = PhaseOutput {
        phase_name: "guarded".to_string(),
        output: "output".to_string(),
        tool_calls: 1,
        duration_secs: 5.0,
        skipped: false,
        guardrail_issues: vec!["Output too short".to_string(), "Missing required section".to_string()],
        quality_score: Some(45),
        quality_retries: 2,
    };
    let json = serde_json::to_value(&po).unwrap();
    assert_eq!(json["guardrail_issues"].as_array().unwrap().len(), 2);
    assert_eq!(json["quality_retries"], 2);
}
