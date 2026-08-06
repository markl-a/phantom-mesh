//! Disk-backed focus session for the `spectyn focus` CLI (SPEC-21, Life Track).
//!
//! `capture_focus_wire` keeps the active session in a process-global in-memory
//! table — fine for the long-lived desktop app, but a CLI runs `start` and
//! `stop` as SEPARATE processes, so the in-memory session would vanish between
//! them. This module persists the single active session to
//! `<home>/.spectyn-mesh/focus-session.json` (single-active invariant) so the
//! terminal flow survives across invocations.
//!
//! On `stop` the finished session is written as a Life Node focus event
//! (kind = "focus", with a summary) so it shows up in `spectyn coach review`
//! and the desktop Daily Review screen — closing the capture→review loop.
//! `base` is the home directory, injected so tests use a tempdir.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::life_node::crypto::{decrypt, encrypt, looks_like_age};
use crate::life_node::key_derivation::{event_key_for_write, load_event_key, EventKey};
use crate::life_node::multimodal::{AnalysisResult, Modality};
use crate::life_node::storage::EventStore;

#[derive(Debug, thiserror::Error)]
pub enum FocusSessionError {
    #[error("a focus session is already active (run `spectyn focus stop` first)")]
    AlreadyActive,
    #[error("no active focus session")]
    NotActive,
    #[error("focus session io: {0}")]
    Io(String),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Interruption {
    pub at_ms: u64,
    pub note: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DiskFocusSession {
    pub session_id: String,
    pub started_at_ms: u64,
    pub planned_duration_ms: u64,
    pub task: Option<String>,
    pub tags: Vec<String>,
    pub interruptions: Vec<Interruption>,
}

#[derive(Debug, Clone)]
pub struct FocusStopResult {
    pub actual_duration_ms: u64,
    pub planned_duration_ms: u64,
    pub completion_pct: f32,
    pub interruptions: usize,
    pub task: Option<String>,
    /// event_id of the Life Node focus event written on stop. Always `Some`
    /// when `stop` returns `Ok` — if the event write is refused (e.g. a corrupt
    /// identity.key), `stop` returns `Err` and the session file is left intact
    /// for retry rather than the event being dropped.
    pub event_id: Option<String>,
}

/// Clock-injectable epoch-millis reading — the hermetic core. Tests pass a
/// `MockClock` to pin `started_at_ms` / interruption `at_ms` deterministically.
fn now_ms_on(clock: &dyn crate::clock::Clock) -> u64 {
    clock.now_ms()
}

/// Production wall-clock reading. Delegates to [`SystemClock`], which reproduces
/// the previous `SystemTime::now() - UNIX_EPOCH` behavior byte-for-byte, so the
/// disk session timestamps are identical to before this refactor.
fn now_ms() -> u64 {
    now_ms_on(&crate::clock::SystemClock)
}

fn spectyn_dir(base: &Path) -> PathBuf {
    crate::cli_config::spectyn_dir_under(base)
}

fn session_path(base: &Path) -> PathBuf {
    spectyn_dir(base).join("focus-session.json")
}

/// The age `EventKey` for sealing the active-session file, if `identity.key`
/// exists. `None` → plaintext fallback (tests / pre-encryption machines), same
/// policy as `EventStore::new` vs `with_key`.
fn session_key(base: &Path) -> Option<EventKey> {
    load_event_key(&spectyn_dir(base).join("identity.key")).ok()
}

/// The active session, or `None` when no session file exists / is unreadable.
///
/// The on-disk session file is age-encrypted when an `identity.key` is present
/// (SPEC-13 / P4 "only you can read"): the live `spectyn focus` session carries
/// `task` + per-interruption `note` free text, which must not sit in plaintext
/// on disk for the whole session. Reads transparently handle both the encrypted
/// form (age magic → decrypt) and a legacy plaintext file (migration).
pub fn status(base: &Path) -> Option<DiskFocusSession> {
    let raw = std::fs::read(session_path(base)).ok()?;
    let plain = if looks_like_age(&raw) {
        // Encrypted on disk → need the key to read it.
        decrypt(&raw, &session_key(base)?).ok()?
    } else {
        raw // legacy plaintext (written before this fix, or no identity.key)
    };
    serde_json::from_slice(&plain).ok()
}

/// Start a session. Refuses if one is already active (single-active invariant).
pub fn start(
    base: &Path,
    planned_minutes: u64,
    task: Option<String>,
    tags: Vec<String>,
) -> Result<DiskFocusSession, FocusSessionError> {
    if status(base).is_some() {
        return Err(FocusSessionError::AlreadyActive);
    }
    std::fs::create_dir_all(spectyn_dir(base)).map_err(|e| FocusSessionError::Io(e.to_string()))?;
    let session = DiskFocusSession {
        session_id: uuid::Uuid::now_v7().to_string(),
        started_at_ms: now_ms(),
        planned_duration_ms: planned_minutes.saturating_mul(60_000),
        task,
        tags: if tags.is_empty() { vec!["focus".to_string()] } else { tags },
        interruptions: Vec::new(),
    };
    write_session(base, &session)?;
    Ok(session)
}

/// Record an interruption on the active session; returns the new total.
pub fn interrupt(base: &Path, note: &str) -> Result<usize, FocusSessionError> {
    let mut session = status(base).ok_or(FocusSessionError::NotActive)?;
    session.interruptions.push(Interruption {
        at_ms: now_ms(),
        note: note.to_string(),
    });
    let n = session.interruptions.len();
    write_session(base, &session)?;
    Ok(n)
}

/// Stop the active session: compute the result, write a Life Node focus event
/// (so it appears in coach review), then remove the session file.
pub fn stop(base: &Path) -> Result<FocusStopResult, FocusSessionError> {
    let session = status(base).ok_or(FocusSessionError::NotActive)?;
    let actual_duration_ms = now_ms().saturating_sub(session.started_at_ms);
    let completion_pct = if session.planned_duration_ms == 0 {
        0.0
    } else {
        ((actual_duration_ms as f64 / session.planned_duration_ms as f64) * 100.0).min(100.0) as f32
    };
    // Persist the focus event FIRST; only delete the live session file once it
    // is durably written. If the write is refused — D24: identity.key present
    // but corrupt — surface the error and KEEP the session file so the user can
    // retry `spectyn focus stop` after repairing the key. Deleting it here (the
    // old behavior) would lose the session's task + interruption notes outright,
    // which is worse than the silent-plaintext bug D24 set out to fix.
    let event_id = Some(write_focus_event(base, &session, actual_duration_ms)?);
    let _ = std::fs::remove_file(session_path(base));
    Ok(FocusStopResult {
        actual_duration_ms,
        planned_duration_ms: session.planned_duration_ms,
        completion_pct,
        interruptions: session.interruptions.len(),
        task: session.task,
        event_id,
    })
}

fn write_session(base: &Path, session: &DiskFocusSession) -> Result<(), FocusSessionError> {
    let bytes = serde_json::to_vec_pretty(session).map_err(|e| FocusSessionError::Io(e.to_string()))?;
    // Seal the active-session file with the same age key as the completed focus
    // event (SPEC-13 / P4): never leave the user's `task`/interruption notes in
    // plaintext on disk. No identity.key → plaintext fallback (matches EventStore).
    // D24: a PRESENT-but-corrupt key must refuse, not silently write plaintext.
    let key = event_key_for_write(&spectyn_dir(base).join("identity.key")).map_err(|e| {
        FocusSessionError::Io(format!(
            "identity.key present but unloadable — refusing to write a plaintext session: {e}"
        ))
    })?;
    let payload = match key {
        Some(k) => encrypt(&bytes, &k).map_err(|e| FocusSessionError::Io(format!("encrypt: {}", e)))?,
        None => bytes,
    };
    std::fs::write(session_path(base), payload).map_err(|e| FocusSessionError::Io(e.to_string()))
}

/// Write the finished session as a Life Node focus event (meta + analysis).
fn write_focus_event(
    base: &Path,
    session: &DiskFocusSession,
    actual_duration_ms: u64,
) -> Result<String, FocusSessionError> {
    let events_dir = spectyn_dir(base).join("events");
    // D24: refuse to write a plaintext focus event when identity.key is present
    // but corrupt (vs. genuinely absent → plaintext is the pre-encryption state).
    let key = event_key_for_write(&spectyn_dir(base).join("identity.key")).map_err(|e| {
        FocusSessionError::Io(format!(
            "identity.key present but unloadable — refusing to write a plaintext focus event: {e}"
        ))
    })?;
    let store = match key {
        Some(k) => EventStore::with_key(&events_dir, k),
        None => EventStore::new(&events_dir),
    };
    let mins = actual_duration_ms / 60_000;
    let secs = (actual_duration_ms % 60_000) / 1000;
    let task = session.task.as_deref().unwrap_or("deep work");
    let summary = format!(
        "{mins}m {secs}s focus session on \"{task}\", {} interruption(s).",
        session.interruptions.len()
    );
    let source_node = std::env::var("SPECTYN_NODE")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "local".to_string());
    let meta = store
        .write_event("focus", &[Modality::Text(summary.clone())], &session.tags, &source_node)
        .map_err(|e| FocusSessionError::Io(e.to_string()))?;
    store
        .write_analysis(
            &meta.event_id,
            &AnalysisResult {
                summary,
                goal_impact: None,
                suggestion: None,
                confidence: None,
                raw_response: serde_json::json!({}),
                model_id: "local-focus-timer".to_string(),
                latency_ms: 0,
                cost_usd: None,
            },
        )
        .map_err(|e| FocusSessionError::Io(e.to_string()))?;
    Ok(meta.event_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_ms_on_reads_the_injected_clock() {
        use crate::clock::{MockClock, SystemClock};
        // Injected mock clock → now_ms_on returns exactly the pinned instant.
        let mock = MockClock::new(1_700_000_000_123);
        assert_eq!(super::now_ms_on(&mock), 1_700_000_000_123);
        mock.advance_ms(1_000);
        assert_eq!(super::now_ms_on(&mock), 1_700_000_001_123);
        // The legacy free fn still reads the real wall clock (production path
        // unchanged): it must equal SystemClock::now_ms within a small skew.
        let sys = SystemClock;
        let a = super::now_ms();
        let b = crate::clock::Clock::now_ms(&sys);
        assert!(a.abs_diff(b) < 5_000, "legacy now_ms must still read the real clock");
    }

    #[test]
    fn start_status_interrupt_stop_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        // No session yet.
        assert!(status(base).is_none());
        // Start.
        let s = start(base, 25, Some("write spec".into()), vec![]).unwrap();
        assert_eq!(s.tags, vec!["focus".to_string()]);
        assert!(status(base).is_some(), "session persisted to disk");
        // Double-start refused.
        assert!(matches!(start(base, 25, None, vec![]), Err(FocusSessionError::AlreadyActive)));
        // Interrupt twice.
        assert_eq!(interrupt(base, "slack ping").unwrap(), 1);
        assert_eq!(interrupt(base, "coffee").unwrap(), 2);
        // Stop → result + event written + session file gone.
        let r = stop(base).unwrap();
        assert_eq!(r.interruptions, 2);
        assert!(r.event_id.is_some(), "focus event written to store");
        assert!(status(base).is_none(), "session file removed after stop");
        // The focus event is on disk under events/ with meta + analysis.
        let events = base.join(".spectyn-mesh").join("events");
        let id = r.event_id.unwrap();
        assert!(events.join(&id).join("meta.json").exists());
        assert!(events.join(&id).join("analysis.json").exists());
        // Stop with no active session errors.
        assert!(matches!(stop(base), Err(FocusSessionError::NotActive)));
    }

    #[test]
    fn active_session_file_is_age_encrypted_when_identity_key_present() {
        // SPEC-13 / P4: with an identity.key, the live active-session file must
        // be age-encrypted on disk — the user's task + interruption notes must
        // not sit in plaintext for the whole session.
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        std::fs::create_dir_all(spectyn_dir(base)).unwrap();
        std::fs::write(spectyn_dir(base).join("identity.key"), [0x42u8; 64]).unwrap();

        start(base, 25, Some("secret merger plan".into()), vec![]).unwrap();
        interrupt(base, "private note about Alice").unwrap();

        // On-disk file is age ciphertext, not the plaintext JSON.
        let raw = std::fs::read(session_path(base)).unwrap();
        assert!(looks_like_age(&raw), "active session file must be age-encrypted");
        assert!(
            serde_json::from_slice::<DiskFocusSession>(&raw).is_err(),
            "ciphertext must not parse as plaintext session JSON"
        );
        let needle = b"secret merger plan";
        assert!(
            !raw.windows(needle.len()).any(|w| w == needle),
            "the task text must not appear in plaintext on disk"
        );

        // …yet status() transparently decrypts + round-trips the data.
        let s = status(base).expect("encrypted session must read back");
        assert_eq!(s.task.as_deref(), Some("secret merger plan"));
        assert_eq!(s.interruptions.len(), 1);
        assert_eq!(s.interruptions[0].note, "private note about Alice");
    }

    #[test]
    fn legacy_plaintext_session_still_reads() {
        // Migration: a session file written before this fix (plaintext, no key)
        // must still be readable by status().
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        // No identity.key → start() writes plaintext (fallback).
        start(base, 10, Some("legacy task".into()), vec![]).unwrap();
        let raw = std::fs::read(session_path(base)).unwrap();
        assert!(!looks_like_age(&raw), "no key → plaintext fallback");
        assert_eq!(status(base).unwrap().task.as_deref(), Some("legacy task"));
    }
}
