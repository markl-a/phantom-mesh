// Tauri command surface for the Daily Review reader (SPEC-41 macOS screen #3,
// BIG-GOAL P2 / Life Track). Wraps the real, read-only, offline backend
// `phantom_mesh::daily_review_wire::load_daily_review` so the React screen can
// show today's (or any date's) Life Node summary.
//
// `date` defaults to the local-today ISO date when omitted/empty. The home dir
// is resolved via `dirs::home_dir()`. catch_unwind guards against any
// unexpected panic deep in the events/decrypt path so a bad event folder can
// never crash the worker — it surfaces as a stable error string instead.

use std::panic::{catch_unwind, AssertUnwindSafe};

use phantom_mesh::daily_review_wire::{load_daily_review, DailyReviewView};

const NOT_YET_WIRED: &str =
    "daily_review.unavailable: could not read the Life Node events directory";

#[tauri::command]
pub async fn daily_review_load(date: Option<String>) -> Result<DailyReviewView, String> {
    let home = dirs::home_dir().ok_or_else(|| "daily_review.no_home_dir".to_string())?;
    let date_iso = match date {
        Some(d) if !d.trim().is_empty() => d,
        _ => chrono::Local::now().format("%Y-%m-%d").to_string(),
    };
    match catch_unwind(AssertUnwindSafe(|| load_daily_review(&home, &date_iso))) {
        Ok(v) => Ok(v),
        Err(_) => Err(NOT_YET_WIRED.to_string()),
    }
}

/// Generate a full coach review for `date` — the aggregate PLUS the Gemini
/// "Tomorrow's one action" pass (graceful no-key footer), persisted to
/// ~/.phantom-mesh/reviews/{date}.md when `save`. App counterpart of `phantom
/// coach review [--save]`; shares `daily_review::run_coach_review` with the CLI.
///
/// When the day's events are locked (age-encrypted + no identity key) we can't
/// read them, so we short-circuit to the read-only view (which carries the
/// unlock prompt) instead of producing an empty review.
#[tauri::command]
pub async fn daily_review_generate(
    date: Option<String>,
    save: Option<bool>,
) -> Result<DailyReviewView, String> {
    let home = dirs::home_dir().ok_or_else(|| "daily_review.no_home_dir".to_string())?;
    let date_iso = match date {
        Some(d) if !d.trim().is_empty() => d,
        _ => chrono::Local::now().format("%Y-%m-%d").to_string(),
    };

    let base = load_daily_review(&home, &date_iso);
    if base.locked {
        return Ok(base);
    }

    let review =
        phantom_mesh::life_node::daily_review::run_coach_review(&home, &date_iso, save.unwrap_or(false))
            .await
            .map_err(|e| format!("daily_review.generate_failed: {e}"))?;
    let flagged = phantom_mesh::life_node::coach_prompts::lint::check(&review.markdown).is_err();
    Ok(DailyReviewView {
        date: date_iso,
        markdown: review.markdown,
        event_count: review.event_count,
        locked: false,
        flagged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn load_returns_wellformed_view_for_explicit_date() {
        // Real home dir; with or without events this must return a view, never
        // panic. (locked when no identity.key — that's a valid view.)
        let v = daily_review_load(Some("2026-01-01".to_string()))
            .await
            .expect("daily_review_load should not error");
        assert_eq!(v.date, "2026-01-01");
        assert!(v.markdown.contains("Daily review"));
    }

    #[tokio::test]
    async fn empty_date_defaults_to_today() {
        let v = daily_review_load(Some("   ".to_string()))
            .await
            .expect("ok");
        // today's ISO date is 10 chars YYYY-MM-DD
        assert_eq!(v.date.len(), 10, "defaulted date should be ISO YYYY-MM-DD");
    }
}
