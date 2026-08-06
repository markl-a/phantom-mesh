// Tauri command surface for SPEC-22 habit chip / text capture.
//
// Wraps `spectyn_mesh::capture_habit_wire` so the React habit chip surfaces
// (SPEC-41 §10.3 ChipPopover / iOS widget / dashboard habit cards) can drive
// the habit lifecycle (create chip → check-in → list → streak) through Tauri's
// invoke channel. Mirrors commands/capture_focus_wire.rs.
//
// The core fns call Stage-2 `*_pseudo` helpers that are still `unimplemented!()`
// (SPEC-22 §9.2), so we wrap with catch_unwind and surface a stable
// "habit.not_yet_wired" string instead of crashing the worker. Typed validation
// errors (ChipNotFound, InvalidSlug, …) pass through verbatim.

use std::panic::{catch_unwind, AssertUnwindSafe};

use spectyn_mesh::capture_habit_wire::{
    self, HabitCaptureError, HabitCheckin, HabitDefinition, HabitStreak, HabitSummary,
};

const NOT_YET_WIRED: &str =
    "habit.not_yet_wired: SPEC-22 Stage 2 deferred — core pseudo helper still unimplemented";

fn err_string(e: HabitCaptureError) -> String {
    e.to_string()
}

fn run_or_unimplemented<T>(f: impl FnOnce() -> Result<T, HabitCaptureError>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(err_string(e)),
        Err(_) => Err(NOT_YET_WIRED.to_string()),
    }
}

#[tauri::command]
pub async fn habit_create(def: HabitDefinition) -> Result<(), String> {
    run_or_unimplemented(|| capture_habit_wire::create_habit(&def))
}

#[tauri::command]
pub async fn habit_checkin(checkin: HabitCheckin) -> Result<HabitStreak, String> {
    run_or_unimplemented(|| capture_habit_wire::record_checkin(&checkin))
}

#[tauri::command]
pub async fn habit_list() -> Result<Vec<HabitSummary>, String> {
    run_or_unimplemented(capture_habit_wire::list_habits)
}

#[tauri::command]
pub async fn habit_streak(habit_slug: String) -> Result<HabitStreak, String> {
    run_or_unimplemented(|| capture_habit_wire::compute_streak(&habit_slug))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spectyn_mesh::capture_habit_wire::{HabitCheckinSource, HabitFrequency};

    fn make_def() -> HabitDefinition {
        HabitDefinition {
            slug: "water".to_string(),
            label: "水".to_string(),
            target_frequency: HabitFrequency::Daily,
            tags: vec!["health".to_string()],
            created_at: "2026-05-28T00:00:00Z".to_string(),
        }
    }

    fn make_checkin() -> HabitCheckin {
        HabitCheckin {
            habit_slug: "water".to_string(),
            timestamp_ms: 1_716_000_000_000,
            note: None,
            source: HabitCheckinSource::Manual,
        }
    }

    // The wrapper's contract is panic-safety + a stable error prefix — NOT a
    // specific Ok/Err split, since the core `*_pseudo` helpers currently return
    // stub data (Ok) and Stage 2 will change that. So each command must simply
    // return a Result without crashing the worker, and any Err must carry the
    // `habit.` kebab-code prefix the front-end key-routes on.

    #[tokio::test]
    async fn habit_create_returns_wellformed_result() {
        if let Err(e) = habit_create(make_def()).await {
            assert!(e.starts_with("habit."), "got {e}");
        }
    }

    #[tokio::test]
    async fn habit_checkin_returns_wellformed_result() {
        if let Err(e) = habit_checkin(make_checkin()).await {
            assert!(e.starts_with("habit."), "got {e}");
        }
    }

    #[tokio::test]
    async fn habit_list_returns_wellformed_result() {
        if let Err(e) = habit_list().await {
            assert!(e.starts_with("habit."), "got {e}");
        }
    }

    #[test]
    fn err_string_carries_kebab_code_prefix() {
        let e = HabitCaptureError::ChipNotFound { slug: "water".to_string() };
        let s = err_string(e);
        assert!(s.starts_with("habit.chip_not_found:"), "got {s}");
        assert!(s.contains("water"), "payload preserved: {s}");
    }

    #[test]
    fn habit_definition_deserializes_from_camelcase_json() {
        let json = r#"{
            "slug": "coffee",
            "label": "咖啡",
            "targetFrequency": { "kind": "daily" },
            "tags": ["morning"],
            "createdAt": "2026-05-28T00:00:00Z"
        }"#;
        let def: HabitDefinition = serde_json::from_str(json).expect("parse ok");
        assert_eq!(def.slug, "coffee");
        assert_eq!(def.tags, vec!["morning".to_string()]);
    }
}
