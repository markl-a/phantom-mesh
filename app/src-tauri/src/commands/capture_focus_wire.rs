// Wave H1.3 — Tauri command surface for SPEC-21 focus-session capture.
//
// Wraps `spectyn_mesh::capture_focus_wire` so the H2.3 React Dashboard
// capture surface can drive the focus-session lifecycle (start → record
// interruption → complete → analyze) through Tauri's invoke channel.
//
// 2 of 4 core fns panic on Stage 4 helpers (`uuid_v7_pseudo` for start_session,
// `bump_counter_pseudo` for record_interruption, `providers_complete_pseudo`
// for analyze_focus_session). We wrap with `catch_unwind` so frontend gets a
// stable "focus.not_yet_wired" string instead of a worker crash. The fully-
// wired `complete_session` (Stage 3) is exposed verbatim.

use std::panic::{catch_unwind, AssertUnwindSafe};

use spectyn_mesh::capture_focus_wire::{
    self, AnalysisResult, FocusCaptureError, FocusSessionRequest, FocusSessionResult,
    InterruptionKind,
};

const NOT_YET_WIRED: &str =
    "focus.not_yet_wired: SPEC-21 Stage 4 deferred — core helper still unimplemented";

fn err_string(e: FocusCaptureError) -> String {
    e.to_string()
}

fn run_or_unimplemented<T>(f: impl FnOnce() -> Result<T, FocusCaptureError>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(err_string(e)),
        Err(_) => Err(NOT_YET_WIRED.to_string()),
    }
}

/// Parse the kebab-case `kind` string the front-end may send into the typed
/// `InterruptionKind` variant the wire fn expects. Matches the slugs returned
/// by `InterruptionKind::slug()` 1:1 so a UI built off the ts-rs binding
/// (`InterruptionKind.ts`) always round-trips cleanly.
///
/// The current Tauri commands take `InterruptionKind` directly via serde, but
/// this helper is exposed for any front-end path that hands in raw kebab slugs
/// (e.g. URL params, log replay). Returns `None` on unknown / typo'd slugs.
pub fn parse_interruption_kind(slug: &str) -> Option<InterruptionKind> {
    match slug {
        "user-pause" => Some(InterruptionKind::UserPause),
        "notification" => Some(InterruptionKind::Notification),
        "app-switch" => Some(InterruptionKind::AppSwitch),
        "screen-lock" => Some(InterruptionKind::ScreenLock),
        _ => None,
    }
}

// ── Focus session lifecycle — delegated to the disk-backed
// `life_node::focus_session` so the app shares ONE source of truth with the
// CLI (`spectyn focus`) and the TUI `/focus` pane. A session started in any
// surface is visible to the others (single active session, persisted to
// ~/.spectyn-mesh/focus-session.json). Command signatures are unchanged so the
// React frontend is unaffected; `session_id` is accepted for API compatibility
// but the disk model is single-active (the app only runs one timer at a time).

fn focus_home() -> Result<std::path::PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "focus.no_home_dir".to_string())
}

#[tauri::command]
pub async fn focus_start_session(req: FocusSessionRequest) -> Result<String, String> {
    use spectyn_mesh::life_node::focus_session;
    let base = focus_home()?;
    let minutes = (req.planned_duration_ms / 60_000).max(1);
    focus_session::start(&base, minutes, req.label.clone(), req.tag.clone())
        .map(|s| s.session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn focus_record_interruption(
    session_id: String,
    kind: InterruptionKind,
) -> Result<(), String> {
    use spectyn_mesh::life_node::focus_session;
    let base = focus_home()?;
    let _ = session_id; // single-active disk session; id kept for API compat
    focus_session::interrupt(&base, kind.slug())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn focus_complete_session(session_id: String) -> Result<FocusSessionResult, String> {
    use spectyn_mesh::life_node::focus_session;
    let base = focus_home()?;
    let _ = session_id;
    let r = focus_session::stop(&base).map_err(|e| e.to_string())?;
    Ok(FocusSessionResult {
        actual_duration_ms: r.actual_duration_ms,
        interruptions: r.interruptions as u16,
        completion_pct: r.completion_pct,
        // focus_session writes a templated summary to the event; the LLM
        // summary/suggestion remain a coach-review concern (kept empty here,
        // same as the previous capture_focus_wire::complete_session).
        summary: String::new(),
        suggestion: String::new(),
    })
}

#[tauri::command]
pub async fn focus_analyze_session(result: FocusSessionResult) -> Result<AnalysisResult, String> {
    run_or_unimplemented(|| capture_focus_wire::analyze_focus_session(&result))
}

/// Active focus session as seen on disk — lets the app surface a session that
/// was started in ANY surface (CLI `spectyn focus`, TUI `/focus`, or the app).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveFocus {
    pub session_id: String,
    pub started_at_ms: u64,
    pub planned_duration_ms: u64,
    pub task: Option<String>,
    pub interruptions: usize,
}

/// Read the shared disk-backed focus session (None when none is active). The
/// app's FocusPage calls this on mount so a CLI/TUI-started session shows up.
#[tauri::command]
pub async fn focus_status() -> Result<Option<ActiveFocus>, String> {
    use spectyn_mesh::life_node::focus_session;
    let base = focus_home()?;
    Ok(focus_session::status(&base).map(|s| ActiveFocus {
        session_id: s.session_id,
        started_at_ms: s.started_at_ms,
        planned_duration_ms: s.planned_duration_ms,
        task: s.task,
        interruptions: s.interruptions.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spectyn_mesh::capture_focus_wire::FocusMode;

    fn make_request() -> FocusSessionRequest {
        FocusSessionRequest {
            mode: FocusMode::DeepWork50,
            planned_duration_ms: 60_000,
            label: Some("test session".to_string()),
            tag: vec!["focus".to_string()],
        }
    }

    fn make_result() -> FocusSessionResult {
        FocusSessionResult {
            actual_duration_ms: 0,
            interruptions: 0,
            completion_pct: 0.0,
            summary: String::new(),
            suggestion: String::new(),
        }
    }

    #[tokio::test]
    async fn focus_lifecycle_delegates_to_disk_session() {
        // The focus commands now delegate to the disk-backed focus_session
        // (shared with the CLI + TUI /focus pane). Smoke-test the full
        // delegation: start → record → complete. Touches the real home like
        // other app-tauri tests (e.g. identity_status); cleans up first so a
        // stray active session from another surface doesn't fail it. The real
        // lifecycle logic is hermetically unit-tested in core focus_session.
        let _ = focus_complete_session("cleanup".to_string()).await; // end any active
        assert!(focus_status().await.unwrap().is_none(), "no active session after cleanup");
        let id = focus_start_session(make_request())
            .await
            .expect("start should succeed");
        // Disk session mints a UUIDv7 — same shape check as before (no uuid dep
        // here, so check the string shape rather than parse).
        assert_eq!(id.len(), 36, "session_id should be a UUID, got {id}");
        assert_eq!(id.as_bytes()[14], b'7', "expected UUIDv7 version nibble, got {id}");
        // focus_status reads the shared disk session back (cross-surface read).
        let active = focus_status().await.unwrap().expect("status sees the active session");
        assert_eq!(active.session_id, id, "status reports the started session");
        focus_record_interruption(id.clone(), InterruptionKind::Notification)
            .await
            .expect("record on the active session should succeed");
        let r = focus_complete_session(id)
            .await
            .expect("complete should succeed");
        assert!(r.interruptions >= 1, "interruption recorded: {}", r.interruptions);
        assert!(r.summary.is_empty(), "LLM summary deferred to coach review");
        assert!(focus_status().await.unwrap().is_none(), "no active session after complete");
    }

    #[tokio::test]
    async fn analyze_session_returns_not_yet_wired_when_providers_helper_unimplemented() {
        let err = focus_analyze_session(make_result()).await.unwrap_err();
        assert!(err.starts_with("focus.not_yet_wired"), "got {err}");
    }

    // ── Type / format invariant pins ─────────────────────────────────────
    // The four tests above pin the Tauri command surface (panic-catch +
    // typed-error pass-through). The five below pin the wire types themselves
    // from this layer so a future rename in `capture_focus_wire` (slug, error
    // Display, serde rename) breaks the Tauri surface loudly.

    #[test]
    fn parse_interruption_kind_round_trips_every_variant() {
        // Every InterruptionKind::slug() output MUST parse back to its variant.
        // Pin all four explicit slugs so a slug rename in capture_focus_wire
        // breaks this test (and the wire contract) loudly rather than silently.
        assert_eq!(
            parse_interruption_kind("user-pause"),
            Some(InterruptionKind::UserPause)
        );
        assert_eq!(
            parse_interruption_kind("notification"),
            Some(InterruptionKind::Notification)
        );
        assert_eq!(
            parse_interruption_kind("app-switch"),
            Some(InterruptionKind::AppSwitch)
        );
        assert_eq!(
            parse_interruption_kind("screen-lock"),
            Some(InterruptionKind::ScreenLock)
        );
    }

    #[test]
    fn parse_interruption_kind_round_trip_via_slug_method() {
        // Cross-pin the parse↔slug pair: serializing via slug() then parsing
        // back yields the original variant. Catches a future kind being added
        // to the enum without a matching parse arm here.
        for kind in [
            InterruptionKind::UserPause,
            InterruptionKind::Notification,
            InterruptionKind::AppSwitch,
            InterruptionKind::ScreenLock,
        ] {
            let slug = kind.slug();
            assert_eq!(
                parse_interruption_kind(slug),
                Some(kind),
                "slug `{slug}` (from {kind:?}) did not round-trip through parse_interruption_kind"
            );
        }
    }

    #[test]
    fn parse_interruption_kind_rejects_unknown_and_typos() {
        assert_eq!(parse_interruption_kind(""), None);
        assert_eq!(parse_interruption_kind("UserPause"), None); // PascalCase
        assert_eq!(parse_interruption_kind("user_pause"), None); // snake_case
        assert_eq!(parse_interruption_kind("user-pauze"), None); // typo
        assert_eq!(parse_interruption_kind("notifications"), None); // plural
    }

    #[test]
    fn err_string_carries_kebab_code_prefix() {
        // Front-end key-routes errors by splitting on the first `:` to extract
        // the `focus.<code>` prefix. Pin a few representative variants so a
        // future Display tweak on FocusCaptureError surfaces here.
        let e = FocusCaptureError::SessionAlreadyActive;
        let s = err_string(e);
        assert_eq!(s, "focus.session_already_active", "exact form: {s}");

        let e2 = FocusCaptureError::SessionNotFound {
            session_id: "abc123".to_string(),
        };
        let s2 = err_string(e2);
        assert!(s2.starts_with("focus.session_not_found:"), "prefix: {s2}");
        assert!(s2.contains("abc123"), "payload preserved: {s2}");

        let e3 = FocusCaptureError::PermissionDenied {
            detail: "mic blocked by TCC".to_string(),
        };
        let s3 = err_string(e3);
        assert!(s3.starts_with("focus.permission_denied:"), "prefix: {s3}");
        assert!(s3.contains("mic blocked"), "detail preserved: {s3}");
    }

    #[test]
    fn focus_session_request_deserializes_from_camelcase_json() {
        // Belt-and-braces: the Tauri command takes `FocusSessionRequest` by
        // value, so the invoke layer must deserialize the camelCase JSON the
        // ts-rs binding emits. Re-pin the camelCase contract here so a future
        // serde rename in capture_focus_wire surfaces against this test (the
        // wire crate also pins it, but breaking the binding here breaks the
        // Tauri surface specifically).
        let json = r#"{
            "mode": "pomodoro25",
            "plannedDurationMs": 1500000,
            "label": "draft spec",
            "tag": ["focus", "spec"]
        }"#;
        let req: FocusSessionRequest = serde_json::from_str(json).expect("parse ok");
        assert_eq!(req.planned_duration_ms, 1_500_000);
        assert_eq!(req.label.as_deref(), Some("draft spec"));
        assert_eq!(req.tag, vec!["focus".to_string(), "spec".to_string()]);
    }
}
