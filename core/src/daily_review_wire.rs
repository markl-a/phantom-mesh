// Wire layer for the Daily Review reader — the desktop/app surface of the
// Life Node 每日回顧 (daily review). Mirrors the TUI `/review` pane design in
// docs/superpowers/design/tui-daily-review.md, but for the Tauri app
// (SPEC-41 macOS screen #3 "coach review reader"). Pillar: P2 multimodal
// understanding, serving the Life Track.
//
// Read-only + offline by construction: it reuses the SAME backend
// (`life_node::daily_review`) that `spectyn coach review` uses —
// `load_events_for_date` + `aggregate` — and deliberately does NOT run the
// network "tomorrow's action" pass (that stays on the CLI / a future opt-in).
// Never errors hard: missing events dir → empty; missing identity.key →
// locked (encrypted-at-rest, key not loaded), never surfaces ciphertext.

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::life_node::coach_prompts::lint;
use crate::life_node::daily_review::{aggregate, load_events_for_date};
use crate::life_node::key_derivation::load_event_key;

/// View-model the app renders. `markdown` is the single source of truth (the
/// app parses it for display); the booleans pick which of the 3 states to show.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/daily_review/")]
#[serde(rename_all = "camelCase")]
pub struct DailyReviewView {
    /// ISO date (YYYY-MM-DD) this review covers.
    pub date: String,
    /// Aggregate Markdown brief (`# Daily review — …`, `## tag (n)`, bullets).
    pub markdown: String,
    /// Number of Life Node events found for `date`.
    pub event_count: usize,
    /// identity.key absent → events can't be decrypted (locked state).
    pub locked: bool,
    /// Shame-free lint flagged the aggregate → app shows a neutral banner
    /// ("some entries were flagged — showing raw log") instead of hiding it.
    pub flagged: bool,
}

/// Build the daily-review view for `date_iso` from `<base>/.spectyn-mesh`.
/// `base` is the home directory, injected so tests can point at a tempdir.
///
/// State resolution mirrors the TUI `/review` pane (fixed by node-a in 7321bbbd):
/// events are written PLAINTEXT when there is no identity.key, so we must NOT
/// assume "no key → locked". Always load (plaintext reads without a key;
/// age-encrypted events decrypt only with one); report `locked` only when there
/// are no readable events AND a genuinely age-encrypted event exists that we
/// have no key for — otherwise it's just empty.
pub fn load_daily_review(base: &Path, date_iso: &str) -> DailyReviewView {
    let spectyn = crate::cli_config::spectyn_dir_under(base);
    let events_dir = spectyn.join("events");
    let identity_path = spectyn.join("identity.key");

    let key = load_event_key(&identity_path).ok();
    let has_key = key.is_some();

    let pairs = load_events_for_date(&events_dir, date_iso, key).unwrap_or_default();
    let event_count = pairs.len();
    let markdown = aggregate(date_iso, &pairs);
    // Best-effort shame-free rail: the backend's lint is non-blocking, so we
    // run it here and let the app surface a neutral banner on a hit.
    let flagged = lint::check(&markdown).is_err();
    // Locked only when nothing was readable AND an event is genuinely encrypted
    // with a key we don't have — never falsely lock plaintext events.
    let locked = event_count == 0 && !has_key && events_dir_has_encrypted(&events_dir);

    DailyReviewView {
        date: date_iso.to_string(),
        markdown,
        event_count,
        locked,
        flagged,
    }
}

/// True if any event under `events_dir` has an age-encrypted `meta.json` (the
/// age v1 magic), i.e. written WITH a key and unreadable without one. Lets us
/// tell a genuine Locked state apart from a plaintext Empty. Mirrors the TUI's
/// `events_dir_has_encrypted` and reuses the shared age-magic check.
fn events_dir_has_encrypted(events_dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(events_dir) else {
        return false;
    };
    for entry in rd.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(entry.path().join("meta.json")) {
            if crate::life_node::crypto::looks_like_age(&bytes) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_key_no_events_is_empty_not_locked() {
        let tmp = tempfile::tempdir().unwrap();
        // No identity.key AND no events → empty, NOT locked (plaintext events
        // would be readable without a key; there just aren't any).
        let v = load_daily_review(tmp.path(), "2026-05-28");
        assert!(!v.locked, "no key + no events must be empty, not locked");
        assert_eq!(v.event_count, 0);
        assert_eq!(v.date, "2026-05-28");
        assert!(v.markdown.contains("Daily review"), "still a well-formed brief");
        assert!(!v.flagged, "empty aggregate is shame-free");
    }

    #[test]
    fn no_key_with_encrypted_event_is_locked() {
        let tmp = tempfile::tempdir().unwrap();
        // An age-encrypted event on disk + no key → genuinely Locked.
        let ev = tmp.path().join(".spectyn-mesh").join("events").join("evt-enc");
        std::fs::create_dir_all(&ev).unwrap();
        std::fs::write(ev.join("meta.json"), b"age-encryption.org/v1\n<ciphertext>").unwrap();
        let v = load_daily_review(tmp.path(), "2026-05-28");
        assert!(v.locked, "encrypted event + no key → locked");
        assert_eq!(v.event_count, 0);
    }

    #[test]
    fn no_key_with_plaintext_event_is_not_locked() {
        let tmp = tempfile::tempdir().unwrap();
        // A plaintext meta.json (no age magic) → must NOT falsely lock.
        let ev = tmp.path().join(".spectyn-mesh").join("events").join("evt-plain");
        std::fs::create_dir_all(&ev).unwrap();
        std::fs::write(ev.join("meta.json"), b"{\"event_id\":\"x\"}").unwrap();
        let v = load_daily_review(tmp.path(), "2026-05-28");
        assert!(!v.locked, "plaintext event must not be reported as locked");
    }
}
