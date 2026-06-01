//! Text-search across captured Life Node events — backs the TUI `/recall`.
//!
//! `/review` is date-scoped; this is content-scoped: a case-insensitive
//! substring match over every event's analysis `summary` + `tags`, newest
//! first. Reads the **file** store (`~/.phantom-mesh/events/<id>/`) that
//! `/note`, focus, and `/review` populate — NOT the separate SPEC-16
//! `events.sqlite` FTS5 path (wire scaffolding, not wired to this data).
//!
//! P4: honors the event key — encrypted events decrypt transparently when a
//! key is supplied; without it they can't be read and are skipped (you can't
//! search what you can't decrypt — same as `/review`). Events lacking an
//! `analysis.json` are skipped, matching the daily-review loader.

use std::path::Path;

use crate::life_node::key_derivation::EventKey;
use crate::life_node::storage::EventStore;
use crate::rpc_wire::EventKind;

/// One search hit, projected from an event's meta + analysis.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct EventHit {
    pub event_id: String,
    /// ISO-8601 timestamp from the event meta.
    pub timestamp: String,
    /// Short kind label: food / focus / habit / text.
    pub kind: String,
    /// The analysis summary (the searchable, human-facing text).
    pub summary: String,
}

/// Short lowercase label for an `EventKind`.
fn kind_str(k: &EventKind) -> &'static str {
    match k {
        EventKind::Food => "food",
        EventKind::Focus => "focus",
        EventKind::Habit => "habit",
        EventKind::Text => "text",
    }
}

/// Filters for an event search. `query` is the case-insensitive substring
/// (empty = match all); `kind` restricts to one event kind (food/focus/habit/
/// text); `since` keeps events on/after a `YYYY-MM-DD` date (inclusive).
#[derive(Debug, Clone, Default)]
pub struct RecallFilter<'a> {
    pub query: &'a str,
    pub kind: Option<&'a str>,
    pub since: Option<&'a str>,
}

impl<'a> RecallFilter<'a> {
    /// Convenience: a text-only filter (no kind/since), matching the original
    /// `search_events(.., query, ..)` behavior.
    pub fn text(query: &'a str) -> Self {
        RecallFilter {
            query,
            kind: None,
            since: None,
        }
    }
}

/// Search events matching `filter`, newest-first, capped at `limit`. Matches a
/// case-insensitive substring over `summary` + `tags`, optionally restricted to
/// a `kind` and/or `since` date. Pure over the on-disk store + supplied key.
pub fn search_events(
    events_dir: &Path,
    key: Option<EventKey>,
    filter: &RecallFilter,
    limit: usize,
) -> std::io::Result<Vec<EventHit>> {
    if !events_dir.exists() {
        return Ok(Vec::new());
    }
    let store = match key {
        Some(k) => EventStore::with_key(events_dir, k),
        None => EventStore::new(events_dir),
    };
    // Split the query into whitespace-separated terms and require ALL of them to
    // appear (in any order) — implicit-AND, the same default FTS5 applies to a
    // multi-token MATCH. So "coffee morning" finds events mentioning both words,
    // not only the literal substring "coffee morning". Single-term queries are
    // unchanged (still a case-insensitive substring match); an empty query still
    // lists everything.
    let terms: Vec<String> = filter
        .query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let kind_want = filter.kind.map(|k| k.trim().to_lowercase());
    let since = filter.since.map(|s| s.trim());
    let mut hits: Vec<EventHit> = Vec::new();
    for entry in std::fs::read_dir(events_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        let Ok(meta) = store.read_meta(&id) else {
            continue;
        };
        let Ok(analysis) = store.read_analysis(&id) else {
            continue;
        };
        let kind = kind_str(&meta.kind);
        // kind filter
        if let Some(want) = &kind_want {
            if kind != want {
                continue;
            }
        }
        // since filter — ISO-8601 timestamps compare lexicographically against
        // a `YYYY-MM-DD` floor. SPEC-16 T-STOR-01: timestamps are UTC, so compare
        // the event's LOCAL date (not the raw UTC string) against the floor —
        // otherwise an event late on the floor day (local) but already next-day
        // in UTC would be wrongly excluded.
        if let Some(floor) = since {
            if crate::event_storage_wire::ts_local_date(&meta.timestamp).as_str() < floor {
                continue;
            }
        }
        let hay = format!("{} {}", analysis.summary, meta.tags.join(" ")).to_lowercase();
        if terms.iter().all(|t| hay.contains(t)) {
            hits.push(EventHit {
                event_id: meta.event_id,
                timestamp: meta.timestamp,
                kind: kind.to_string(),
                summary: analysis.summary,
            });
        }
    }
    // Newest first, by absolute instant — lexical sort isn't chronological once
    // the store mixes legacy local-offset and new UTC timestamps. T-STOR-01.
    hits.sort_by(|a, b| {
        crate::event_storage_wire::ts_epoch_ms(&b.timestamp)
            .cmp(&crate::event_storage_wire::ts_epoch_ms(&a.timestamp))
    });
    hits.truncate(limit);
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::life_node::note_capture::capture_note;

    #[test]
    fn search_finds_by_substring_case_insensitive_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let phantom = tmp.path().join(".phantom-mesh");
        std::fs::create_dir_all(&phantom).unwrap();
        let events = phantom.join("events");

        capture_note(&phantom, "remember to call the Dentist", &["note".into()]).unwrap();
        capture_note(&phantom, "buy milk and eggs", &["note".into()]).unwrap();

        // case-insensitive substring match
        let hits = search_events(&events, None, &RecallFilter::text("dentist"), 15).unwrap();
        assert_eq!(hits.len(), 1, "only the dentist note matches");
        assert!(hits[0].summary.contains("Dentist"));
        assert_eq!(hits[0].kind, "text");

        // empty query → everything (recent listing), newest-first
        let all = search_events(&events, None, &RecallFilter::text(""), 15).unwrap();
        assert_eq!(all.len(), 2);
        assert!(
            all[0].timestamp >= all[1].timestamp,
            "newest first: {:?}",
            all.iter().map(|h| &h.timestamp).collect::<Vec<_>>()
        );

        // no match → empty
        assert!(search_events(&events, None, &RecallFilter::text("zzznope"), 15)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn search_multi_term_requires_all_terms_any_order() {
        let tmp = tempfile::tempdir().unwrap();
        let phantom = tmp.path().join(".phantom-mesh");
        std::fs::create_dir_all(&phantom).unwrap();
        let events = phantom.join("events");

        capture_note(&phantom, "morning coffee with the team", &["note".into()]).unwrap();
        capture_note(&phantom, "afternoon coffee run", &["note".into()]).unwrap();
        capture_note(&phantom, "morning standup", &["note".into()]).unwrap();

        // Both terms must appear → only the note with BOTH "coffee" + "morning".
        let hits = search_events(&events, None, &RecallFilter::text("coffee morning"), 15).unwrap();
        assert_eq!(hits.len(), 1, "AND semantics: only the both-terms note");
        assert!(hits[0].summary.contains("morning coffee"));

        // Order-independent — same single hit.
        let rev = search_events(&events, None, &RecallFilter::text("morning coffee"), 15).unwrap();
        assert_eq!(rev.len(), 1);
        assert_eq!(rev[0].event_id, hits[0].event_id);

        // A term present in NO event → zero hits (AND, not OR).
        let none = search_events(&events, None, &RecallFilter::text("coffee zzznope"), 15).unwrap();
        assert!(none.is_empty(), "missing term excludes via AND");

        // Extra interior whitespace is ignored (split_whitespace).
        let spaced = search_events(&events, None, &RecallFilter::text("  coffee   morning  "), 15)
            .unwrap();
        assert_eq!(spaced.len(), 1);
    }

    #[test]
    fn search_multi_term_spans_summary_and_tags() {
        let tmp = tempfile::tempdir().unwrap();
        let phantom = tmp.path().join(".phantom-mesh");
        std::fs::create_dir_all(&phantom).unwrap();
        // One term in the summary, the other only in a tag → still matches (the
        // haystack is summary + tags joined).
        capture_note(&phantom, "quarterly planning", &["work".into()]).unwrap();
        let hits = search_events(&phantom.join("events"), None, &RecallFilter::text("planning work"), 15)
            .unwrap();
        assert_eq!(hits.len(), 1, "terms may match across summary and tags");
    }

    #[test]
    fn search_matches_on_tags_too() {
        let tmp = tempfile::tempdir().unwrap();
        let phantom = tmp.path().join(".phantom-mesh");
        std::fs::create_dir_all(&phantom).unwrap();
        capture_note(&phantom, "standup notes", &["work".into(), "meeting".into()]).unwrap();
        let hits = search_events(&phantom.join("events"), None, &RecallFilter::text("meeting"), 15)
            .unwrap();
        assert_eq!(hits.len(), 1, "tag match counts");
    }

    #[test]
    fn kind_filter_restricts_to_one_kind() {
        let tmp = tempfile::tempdir().unwrap();
        let phantom = tmp.path().join(".phantom-mesh");
        std::fs::create_dir_all(&phantom).unwrap();
        let events = phantom.join("events");
        // Two text notes (capture_note writes kind="note" → projects to Text).
        capture_note(&phantom, "alpha", &["note".into()]).unwrap();
        capture_note(&phantom, "beta", &["note".into()]).unwrap();

        let text_hits = search_events(
            &events,
            None,
            &RecallFilter { query: "", kind: Some("text"), since: None },
            15,
        )
        .unwrap();
        assert_eq!(text_hits.len(), 2, "both notes are kind=text");

        let food_hits = search_events(
            &events,
            None,
            &RecallFilter { query: "", kind: Some("food"), since: None },
            15,
        )
        .unwrap();
        assert!(food_hits.is_empty(), "no food events captured");
    }

    #[test]
    fn since_filter_keeps_on_or_after_date() {
        let tmp = tempfile::tempdir().unwrap();
        let phantom = tmp.path().join(".phantom-mesh");
        std::fs::create_dir_all(&phantom).unwrap();
        let events = phantom.join("events");
        capture_note(&phantom, "today note", &["note".into()]).unwrap();

        // A far-future floor excludes today's note; the epoch floor keeps it.
        let none = search_events(
            &events,
            None,
            &RecallFilter { query: "", kind: None, since: Some("2999-01-01") },
            15,
        )
        .unwrap();
        assert!(none.is_empty(), "future since-floor excludes everything");

        let all = search_events(
            &events,
            None,
            &RecallFilter { query: "", kind: None, since: Some("1970-01-01") },
            15,
        )
        .unwrap();
        assert_eq!(all.len(), 1, "epoch floor keeps the note");
    }

    #[test]
    fn missing_events_dir_is_empty_not_error() {
        let tmp = tempfile::tempdir().unwrap();
        let hits = search_events(&tmp.path().join("nope"), None, &RecallFilter::text("x"), 5).unwrap();
        assert!(hits.is_empty());
    }
}
