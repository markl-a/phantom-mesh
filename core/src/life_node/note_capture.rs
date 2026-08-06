//! Synchronous free-text note capture into the Life Node event store.
//!
//! Unlike `spectyn event capture` / `spectyn food` (which POST to the local
//! `spectyn serve` daemon for multimodal analysis), a plain text note needs no
//! image/audio and no LLM round-trip. So this writes **directly** to the same
//! `~/.spectyn-mesh/events` store the `/review` pane reads — the path focus
//! events already use. No daemon dependency; instant; offline-friendly.
//!
//! P4: honors `<spectyn_dir>/identity.key` via `event_key_for_write` — when a
//! usable key is present the event is age-encrypted at rest (SPEC-13); when no
//! key exists the note is written plaintext (the intended pre-encryption state,
//! which `/identity` + `doctor` already surface). A PRESENT-but-corrupt key is
//! NOT silently downgraded to plaintext (D24): the capture is refused with an
//! error instead. The returned `encrypted` flag lets the caller tell the user
//! which, so a note's privacy state is never misrepresented.

use std::path::Path;

use crate::life_node::key_derivation::event_key_for_write;
use crate::life_node::multimodal::{AnalysisResult, Modality};
use crate::life_node::storage::EventStore;

/// Outcome of a note capture: the new event id + whether it was encrypted at
/// rest (true when an `identity.key`-derived event key was available).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteCaptured {
    pub event_id: String,
    pub encrypted: bool,
}

/// Source-node label, from `$SPECTYN_NODE` / `$HOSTNAME`, else `"local"`.
/// Mirrors the focus-session capture path.
fn source_node() -> String {
    std::env::var("SPECTYN_NODE")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "local".to_string())
}

/// Capture `text` as a `kind="note"` Life Node event under
/// `<spectyn_dir>/events`. `spectyn_dir` is the `.spectyn-mesh` directory
/// (so the real call passes `~/.spectyn-mesh`). Returns the new event id +
/// encryption state.
pub fn capture_note(
    spectyn_dir: &Path,
    text: &str,
    tags: &[String],
) -> std::io::Result<NoteCaptured> {
    let events_dir = spectyn_dir.join("events");
    // D24: distinguish "no identity.key" (plaintext is the intended
    // pre-encryption state) from "key present but corrupt" — the latter must
    // NOT silently downgrade a private note to plaintext on disk.
    let key = event_key_for_write(&spectyn_dir.join("identity.key")).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("identity.key present but unloadable — refusing to write a plaintext note: {e}"),
        )
    })?;
    let encrypted = key.is_some();
    let store = match key {
        Some(k) => EventStore::with_key(&events_dir, k),
        None => EventStore::new(&events_dir),
    };
    let sn = source_node();
    let meta = store.write_event("note", &[Modality::Text(text.to_string())], tags, &sn)?;
    // Write a sibling analysis so the note surfaces in `/review` — the daily
    // loader skips events without one. There's no LLM here: the "analysis" is
    // just the note text echoed back (model_id marks it as locally captured).
    store.write_analysis(
        &meta.event_id,
        &AnalysisResult {
            summary: text.to_string(),
            goal_impact: None,
            suggestion: None,
            confidence: None,
            raw_response: serde_json::json!({}),
            model_id: "local-note".to_string(),
            latency_ms: 0,
            cost_usd: None,
        },
    )?;
    Ok(NoteCaptured {
        event_id: meta.event_id,
        encrypted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_note_writes_plaintext_event_when_no_key() {
        let tmp = tempfile::tempdir().unwrap();
        let spectyn = tmp.path().join(".spectyn-mesh");
        std::fs::create_dir_all(&spectyn).unwrap();

        let out = capture_note(&spectyn, "call the dentist", &["note".to_string()]).unwrap();
        assert!(!out.event_id.is_empty(), "event id returned");
        assert!(!out.encrypted, "no identity.key → plaintext");

        // The event dir + meta.json must exist with kind=note.
        let meta_path = spectyn
            .join("events")
            .join(&out.event_id)
            .join("meta.json");
        assert!(meta_path.exists(), "meta.json written: {}", meta_path.display());
        let meta = std::fs::read_to_string(&meta_path).unwrap();
        assert!(meta.contains("\"note\""), "kind=note in meta: {meta}");
    }

    #[test]
    fn capture_note_round_trips_via_daily_review() {
        let tmp = tempfile::tempdir().unwrap();
        let spectyn = tmp.path().join(".spectyn-mesh");
        std::fs::create_dir_all(&spectyn).unwrap();

        capture_note(&spectyn, "shipped the note feature", &["note".to_string()]).unwrap();

        // Today's events (plaintext, no key) load back through the same path
        // the /review pane uses.
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let pairs = crate::life_node::daily_review::load_events_for_date(
            &spectyn.join("events"),
            &today,
            None,
        )
        .unwrap_or_default();
        assert_eq!(pairs.len(), 1, "the captured note loads back for today");
        // On-disk kind "note" projects to the EventKind::Text wire variant
        // (catch-all per storage::project_to_wire); the analysis carries the text.
        assert_eq!(pairs[0].0.kind, crate::rpc_wire::EventKind::Text);
        assert_eq!(pairs[0].1.summary, "shipped the note feature");
    }
}
