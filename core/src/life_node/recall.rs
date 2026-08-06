//! Text-search across captured Life Node events — backs the TUI `/recall`.
//!
//! `/review` is date-scoped; this is content-scoped: a case-insensitive
//! substring match over every event's analysis `summary` + `tags`, newest
//! first. Reads the **file** store (`~/.spectyn-mesh/events/<id>/`) that
//! `/note`, focus, and `/review` populate, AND merges hits from the SPEC-16
//! `events.sqlite` FTS5 index that the wire capture path (food/habit via
//! `event_storage_wire::write_event` + `index_fts5`) populates — those events
//! carry no `analysis.json` and a non-loader meta schema, so the file walk
//! alone would miss them. Deduped by `event_id`, both under the same filters.
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
    /// Hybrid relevance score in `[0,1]` — a blend of lexical (FTS5/substring)
    /// presence and semantic cosine similarity. `#[serde(default)]` keeps the
    /// `--json` surface backward-compatible: older consumers that don't read it
    /// still parse, and a deserialized hit without the field defaults to 0.
    /// Higher = more relevant. In `keyword` mode this stays a lexical-only
    /// proxy; in `semantic`/`hybrid` it carries the cosine contribution.
    #[serde(default)]
    pub relevance: f32,
}

/// Short lowercase label for an `EventKind`.
fn kind_str(k: &EventKind) -> &'static str {
    match k {
        EventKind::Food => "food",
        EventKind::Focus => "focus",
        EventKind::Habit => "habit",
        EventKind::Dispatch => "dispatch",
        EventKind::Text => "text",
    }
}

/// Which retrieval legs to run. `Keyword` = lexical only (the pre-existing
/// FTS5 BM25 + file-store substring behaviour). `Semantic` = embedding cosine
/// only. `Hybrid` (default) = union of both, deduped, blended score. When the
/// embedder is unavailable, `Semantic`/`Hybrid` gracefully degrade to lexical.
///
/// 中文: 召回模式。Keyword=純關鍵字;Semantic=純語意向量;Hybrid(預設)=兩者
/// 聯集去重 + 混合分數。embedder 不在時 Semantic/Hybrid 自動降級為關鍵字。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecallMode {
    Keyword,
    Semantic,
    #[default]
    Hybrid,
}

impl RecallMode {
    /// Parse a `--mode` CLI value; unknown / empty → `Hybrid` (the default).
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "keyword" | "kw" | "lexical" => RecallMode::Keyword,
            "semantic" | "sem" | "vector" => RecallMode::Semantic,
            _ => RecallMode::Hybrid,
        }
    }
}

/// Filters for an event search. `query` is the case-insensitive substring
/// (empty = match all); `kind` restricts to one event kind (food/focus/habit/
/// text); `since` keeps events on/after a `YYYY-MM-DD` date (inclusive);
/// `mode` selects the keyword/semantic/hybrid retrieval legs.
#[derive(Debug, Clone, Default)]
pub struct RecallFilter<'a> {
    pub query: &'a str,
    pub kind: Option<&'a str>,
    pub since: Option<&'a str>,
    pub mode: RecallMode,
}

impl<'a> RecallFilter<'a> {
    /// Convenience: a text-only filter (no kind/since, keyword mode), matching
    /// the original `search_events(.., query, ..)` behavior — a pure lexical,
    /// embedder-independent search. Callers that want semantic/hybrid recall
    /// construct `RecallFilter { mode, .. }` explicitly.
    pub fn text(query: &'a str) -> Self {
        RecallFilter {
            query,
            kind: None,
            since: None,
            mode: RecallMode::Keyword,
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
    let mode = filter.mode;
    // The FTS5/semantic db is the sibling of the file store: `<base>/events.sqlite`
    // next to `<base>/events`. Derive it from the SAME base as `events_dir` so a
    // caller passing a temp `events_dir` isolates these reads too (hermeticity).
    let fts_db = match events_dir.parent() {
        Some(base) => base.join("events.sqlite"),
        None => crate::event_storage_wire::default_events_sqlite_path(),
    };

    // A small local helper to apply the shared kind/since gate to a recovered
    // meta. Returns the lowercase kind label if it passes, else None.
    let passes_filters = |kind: &str, timestamp: &str| -> bool {
        if let Some(want) = &kind_want {
            if kind != want {
                return false;
            }
        }
        if let Some(floor) = since {
            if crate::event_storage_wire::ts_local_date(timestamp).as_str() < floor {
                return false;
            }
        }
        true
    };

    let mut hits: Vec<EventHit> = Vec::new();
    // ── Lexical legs (file-store substring + FTS5 BM25). Extracted into a
    //    closure so it can run EITHER up-front (Keyword/Hybrid) OR as a graceful
    //    fallback for Semantic when the embedder is unavailable — in that case
    //    the semantic leg returns nothing, and rather than answer `--semantic`
    //    with an empty result while Ollama is down we degrade to keyword hits
    //    (the `RecallMode` doc's "Semantic/Hybrid gracefully degrade to lexical"
    //    contract). For Keyword/Hybrid this is byte-identical to before.
    let collect_lexical = |hits: &mut Vec<EventHit>| -> std::io::Result<()> {
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
            // since filter — ISO-8601 timestamps compare lexicographically against
            // a `YYYY-MM-DD` floor. SPEC-16 T-STOR-01: timestamps are UTC, so
            // compare the event's LOCAL date (not the raw UTC string) against the
            // floor — otherwise an event late on the floor day (local) but already
            // next-day in UTC would be wrongly excluded.
            if !passes_filters(kind, &meta.timestamp) {
                continue;
            }
            let hay = format!("{} {}", analysis.summary, meta.tags.join(" ")).to_lowercase();
            if terms.iter().all(|t| hay.contains(t)) {
                hits.push(EventHit {
                    event_id: meta.event_id,
                    timestamp: meta.timestamp,
                    kind: kind.to_string(),
                    summary: analysis.summary,
                    // Lexical hit: a present-term proxy. The semantic merge below
                    // can lift this if the same event also scores by cosine.
                    relevance: 0.5,
                });
            }
        }
        // ── Also pull in the SPEC-16 wire/FTS5 store (events.sqlite) ───────────
        //
        // Food/habit captures that go through `event_storage_wire::write_event`
        // write a PLAINTEXT `meta.json` + index their PII-scrubbed `summary` into
        // the FTS5 index — but write NO `analysis.json`, so the file-store walk
        // above skips them. Merge the FTS5 hits here with the SAME kind/since
        // filters, deduped against the file-store hits.
        let already: std::collections::HashSet<String> =
            hits.iter().map(|h| h.event_id.clone()).collect();
        if let Ok(fts_hits) =
            crate::event_storage_wire::search_fts5_hits_at(&fts_db, filter.query, limit.max(1))
        {
            for fh in fts_hits {
                if already.contains(&fh.event_id) {
                    continue;
                }
                // The wire meta is plaintext (no EventKey needed); recover kind +
                // timestamp. Skip if absent / not this store's schema.
                let Some(meta) =
                    crate::event_storage_wire::read_meta_only_at(events_dir, &fh.event_id)
                else {
                    continue;
                };
                let kind = kind_str(&meta.kind);
                if !passes_filters(kind, &meta.timestamp) {
                    continue;
                }
                hits.push(EventHit {
                    event_id: meta.event_id,
                    timestamp: meta.timestamp,
                    kind: kind.to_string(),
                    summary: fh.content,
                    relevance: 0.5,
                });
            }
        }
        Ok(())
    };

    if mode != RecallMode::Semantic {
        collect_lexical(&mut hits)?;
    }

    // ── Semantic leg (embedding cosine) — run for Semantic and Hybrid. ─────────
    //
    // Embed the query, brute-force top-k over `events_emb`, then materialise each
    // hit's kind/timestamp/summary from whichever store holds it (file store via
    // the EventKey, else the plaintext wire meta). This is what lets a query in
    // DIFFERENT words ("async runtime crash") surface a note that said "tokio
    // runtime panic". Best-effort: if the embedder is unavailable, `semantic_topk_at`
    // returns an empty Vec → recall silently degrades to the lexical hits above.
    if mode != RecallMode::Keyword && !filter.query.trim().is_empty() {
        // Pull a generous candidate window (cosine ranks everything; we filter
        // by kind/since after, then truncate to `limit` at the end).
        let topk = crate::event_storage_wire::semantic_topk_at(&fts_db, filter.query, limit.max(1).saturating_mul(4))
            .unwrap_or_default();
        let mut by_id: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for (idx, h) in hits.iter().enumerate() {
            by_id.insert(h.event_id.clone(), idx);
        }
        for (event_id, cosine) in topk {
            // Map cosine ([-1,1]) into a [0,1] relevance; clamp negatives to 0.
            let sem_rel = cosine.clamp(0.0, 1.0);
            if let Some(&idx) = by_id.get(&event_id) {
                // Already surfaced lexically — blend: lift its relevance toward
                // the cosine score (a doc found by BOTH legs is the strongest).
                let lex = hits[idx].relevance;
                hits[idx].relevance = (lex + sem_rel).min(1.0).max(sem_rel);
                continue;
            }
            // New, semantic-only hit. Materialise it from whichever store has it.
            let Some((timestamp, kind, summary)) = materialise_hit(&store, events_dir, &event_id)
            else {
                continue;
            };
            if !passes_filters(&kind, &timestamp) {
                continue;
            }
            let new_idx = hits.len();
            hits.push(EventHit {
                event_id: event_id.clone(),
                timestamp,
                kind,
                summary,
                relevance: sem_rel,
            });
            by_id.insert(event_id, new_idx);
        }
    }

    // Graceful degradation for pure Semantic mode: when the embedder is
    // unavailable/empty the semantic leg above gathered nothing, which would
    // make `--semantic` silently return zero results while Ollama is down. Fall
    // back to the lexical legs so recall still answers with FTS5/file-store
    // keyword hits (honoring the `RecallMode` doc's "Semantic degrades to
    // lexical" contract) — never an empty surprise, never a panic.
    if mode == RecallMode::Semantic && hits.is_empty() {
        collect_lexical(&mut hits)?;
    }

    // Ordering:
    //  • Keyword mode keeps the historical "newest first" contract (relevance is
    //    a flat lexical proxy there, so timestamp is the meaningful axis).
    //  • Semantic / Hybrid order by relevance DESC (the whole point — most
    //    semantically related first), with newest-first as the tiebreak.
    // Nanosecond precision keeps the tiebreak deterministic (ms truncation tied
    // same-millisecond events, flaking the order). T-STOR-01.
    if mode == RecallMode::Keyword {
        hits.sort_by(|a, b| {
            crate::event_storage_wire::ts_epoch_nanos(&b.timestamp)
                .cmp(&crate::event_storage_wire::ts_epoch_nanos(&a.timestamp))
        });
    } else {
        hits.sort_by(|a, b| {
            b.relevance
                .total_cmp(&a.relevance)
                .then_with(|| {
                    crate::event_storage_wire::ts_epoch_nanos(&b.timestamp)
                        .cmp(&crate::event_storage_wire::ts_epoch_nanos(&a.timestamp))
                })
        });
    }
    hits.truncate(limit);
    Ok(hits)
}

/// Materialise a semantic-only hit (located by event_id from the `events_emb`
/// cosine ranking) into `(timestamp, kind, summary)`. Tries the file store first
/// (decrypting `meta.json` + `analysis.json` via the EventKey, where `/note` /
/// focus events live), then falls back to the plaintext wire `meta.json` +
/// FTS5-indexed content (where food/habit captures live). Returns `None` if the
/// event can't be read from either store (e.g. key locked) — the hit is dropped,
/// matching recall's "can't search what you can't decrypt" contract.
fn materialise_hit(
    store: &EventStore,
    events_dir: &Path,
    event_id: &str,
) -> Option<(String, String, String)> {
    // File store (note/focus): decrypt meta + analysis.
    if let (Ok(meta), Ok(analysis)) = (store.read_meta(event_id), store.read_analysis(event_id)) {
        let kind = kind_str(&meta.kind).to_string();
        return Some((meta.timestamp, kind, analysis.summary));
    }
    // Wire store (food/habit): plaintext meta + FTS5 content for the summary.
    let meta = crate::event_storage_wire::read_meta_only_at(events_dir, event_id)?;
    let kind = kind_str(&meta.kind).to_string();
    // Recover the indexed summary from FTS5 (the wire store has no analysis.json).
    let fts_db = match events_dir.parent() {
        Some(base) => base.join("events.sqlite"),
        None => crate::event_storage_wire::default_events_sqlite_path(),
    };
    let summary = crate::event_storage_wire::search_fts5_hits_at(&fts_db, "", 1000)
        .ok()
        .and_then(|hits| {
            hits.into_iter()
                .find(|h| h.event_id == event_id)
                .map(|h| h.content)
        })
        .unwrap_or_default();
    Some((meta.timestamp, kind, summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::life_node::note_capture::capture_note;

    #[test]
    fn search_finds_by_substring_case_insensitive_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let spectyn = tmp.path().join(".spectyn-mesh");
        std::fs::create_dir_all(&spectyn).unwrap();
        let events = spectyn.join("events");

        capture_note(&spectyn, "remember to call the Dentist", &["note".into()]).unwrap();
        capture_note(&spectyn, "buy milk and eggs", &["note".into()]).unwrap();

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
        let spectyn = tmp.path().join(".spectyn-mesh");
        std::fs::create_dir_all(&spectyn).unwrap();
        let events = spectyn.join("events");

        capture_note(&spectyn, "morning coffee with the team", &["note".into()]).unwrap();
        capture_note(&spectyn, "afternoon coffee run", &["note".into()]).unwrap();
        capture_note(&spectyn, "morning standup", &["note".into()]).unwrap();

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
        let spectyn = tmp.path().join(".spectyn-mesh");
        std::fs::create_dir_all(&spectyn).unwrap();
        // One term in the summary, the other only in a tag → still matches (the
        // haystack is summary + tags joined).
        capture_note(&spectyn, "quarterly planning", &["work".into()]).unwrap();
        let hits = search_events(&spectyn.join("events"), None, &RecallFilter::text("planning work"), 15)
            .unwrap();
        assert_eq!(hits.len(), 1, "terms may match across summary and tags");
    }

    #[test]
    fn search_matches_on_tags_too() {
        let tmp = tempfile::tempdir().unwrap();
        let spectyn = tmp.path().join(".spectyn-mesh");
        std::fs::create_dir_all(&spectyn).unwrap();
        capture_note(&spectyn, "standup notes", &["work".into(), "meeting".into()]).unwrap();
        let hits = search_events(&spectyn.join("events"), None, &RecallFilter::text("meeting"), 15)
            .unwrap();
        assert_eq!(hits.len(), 1, "tag match counts");
    }

    #[test]
    fn kind_filter_restricts_to_one_kind() {
        let tmp = tempfile::tempdir().unwrap();
        let spectyn = tmp.path().join(".spectyn-mesh");
        std::fs::create_dir_all(&spectyn).unwrap();
        let events = spectyn.join("events");
        // Two text notes (capture_note writes kind="note" → projects to Text).
        capture_note(&spectyn, "alpha", &["note".into()]).unwrap();
        capture_note(&spectyn, "beta", &["note".into()]).unwrap();

        let text_hits = search_events(
            &events,
            None,
            &RecallFilter { query: "", kind: Some("text"), since: None, mode: RecallMode::default() },
            15,
        )
        .unwrap();
        assert_eq!(text_hits.len(), 2, "both notes are kind=text");

        let food_hits = search_events(
            &events,
            None,
            &RecallFilter { query: "", kind: Some("food"), since: None, mode: RecallMode::default() },
            15,
        )
        .unwrap();
        assert!(food_hits.is_empty(), "no food events captured");
    }

    #[test]
    fn kind_filter_surfaces_dispatch_events() {
        use crate::life_node::multimodal::{AnalysisResult, Modality};
        use crate::life_node::storage::EventStore;
        let tmp = tempfile::tempdir().unwrap();
        let spectyn = tmp.path().join(".spectyn-mesh");
        std::fs::create_dir_all(&spectyn).unwrap();
        let events = spectyn.join("events");

        // One ordinary note (kind=note→text) plus a cross-node dispatch event
        // written the way `persist_dispatch_event` writes them (kind "dispatch"
        // + an analysis.json so recall's file-walk doesn't skip it).
        capture_note(&spectyn, "ordinary note", &["note".into()]).unwrap();
        let store = EventStore::new(&events);
        let m = store
            .write_event("dispatch", &[Modality::Text("ran echo on peer".into())], &[], "n")
            .unwrap();
        store
            .write_analysis(
                &m.event_id,
                &AnalysisResult {
                    summary: "dispatch — 1 ok, 0 failed, 12ms".into(),
                    goal_impact: None,
                    suggestion: None,
                    confidence: None,
                    raw_response: serde_json::Value::Null,
                    model_id: "dispatch".into(),
                    latency_ms: 12,
                    cost_usd: None,
                },
            )
            .unwrap();

        // `spectyn recall --kind dispatch` surfaces the dispatch event…
        let disp = search_events(
            &events,
            None,
            &RecallFilter { query: "", kind: Some("dispatch"), since: None, mode: RecallMode::default() },
            15,
        )
        .unwrap();
        assert_eq!(disp.len(), 1, "dispatch event is recall-visible under --kind dispatch");
        assert_eq!(disp[0].kind, "dispatch");

        // …and the dispatch event is NOT mistaken for a text event.
        let txt = search_events(
            &events,
            None,
            &RecallFilter { query: "", kind: Some("text"), since: None, mode: RecallMode::default() },
            15,
        )
        .unwrap();
        assert_eq!(txt.len(), 1, "only the ordinary note is kind=text, not the dispatch");
    }

    #[test]
    fn since_filter_keeps_on_or_after_date() {
        let tmp = tempfile::tempdir().unwrap();
        let spectyn = tmp.path().join(".spectyn-mesh");
        std::fs::create_dir_all(&spectyn).unwrap();
        let events = spectyn.join("events");
        capture_note(&spectyn, "today note", &["note".into()]).unwrap();

        // A far-future floor excludes today's note; the epoch floor keeps it.
        let none = search_events(
            &events,
            None,
            &RecallFilter { query: "", kind: None, since: Some("2999-01-01"), mode: RecallMode::default() },
            15,
        )
        .unwrap();
        assert!(none.is_empty(), "future since-floor excludes everything");

        let all = search_events(
            &events,
            None,
            &RecallFilter { query: "", kind: None, since: Some("1970-01-01"), mode: RecallMode::default() },
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

    // ── Cross-chain end-to-end: habit capture → recall (FTS5 merge branch) ────
    //
    // `capture_note` writes the file-store (`events/<id>/` with analysis.json),
    // which the file-walk above covers. But the SPEC-16 *wire* capture path
    // (`spectyn habit` → `capture_habit_wire::record_checkin`) writes a
    // PLAINTEXT `meta.json` in the wire `EventMeta` shape + indexes a PII-scrubbed
    // summary into the `events.sqlite` FTS5 index, and NO `analysis.json`. Those
    // events are invisible to the file walk and only surface through the FTS5
    // merge branch (the `search_fts5_hits` + `read_meta_only` block above). These
    // tests drive a REAL habit check-in end-to-end and assert it is recallable —
    // the read side (`recall`) finding what the write side (`habit`) indexed.
    //
    // `$HOME`-mutating + per-process EventKey cache, so `#[ignore]`d like the
    // other env-dependent integration tests (run via `--ignored`). Serialised by
    // a local mutex so two of these never race the shared `$HOME` / key cache.

    use crate::capture_habit_wire::{
        create_habit, record_checkin, HabitCheckin, HabitCheckinSource, HabitDefinition,
        HabitFrequency,
    };

    static RECALL_HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// RAII guard: isolate `$HOME` to a tempdir, install a deterministic
    /// per-process EventKey (OSS-safe fixed seed) so the encrypted EventStore
    /// round-trips, and restore both on drop. Holds `RECALL_HOME_LOCK` for the
    /// guard's lifetime so the exclusive window spans the whole test body
    /// (`$HOME` + the EventKey cache are global process state).
    struct WireEnvGuard {
        prev_home: Option<std::ffi::OsString>,
        _tmp: tempfile::TempDir,
        _lock: std::sync::MutexGuard<'static, ()>,
        // Crate-wide env lock so these $HOME-mutating tests also serialise
        // against the rest of the suite (capture_food_wire / event_storage_wire
        // / …), not just sibling recall tests. Declared last so it drops AFTER
        // `drop()` restores HOME and after the local `_lock`: HOME is put back
        // while env_lock is still held, then env_lock releases.
        _env: std::sync::MutexGuard<'static, ()>,
    }
    impl Drop for WireEnvGuard {
        fn drop(&mut self) {
            crate::encryption_wire::clear_event_key_cache();
            match &self.prev_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
    }
    fn isolate_wire_env() -> WireEnvGuard {
        // Acquire the crate-wide env lock FIRST (consistent ordering with every
        // other file, which only takes env_lock) so there is no lock-order
        // inversion, then the module-local RECALL_HOME_LOCK.
        let env = crate::env_lock::acquire();
        let lock = RECALL_HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", tmp.path());
        crate::encryption_wire::install_event_key_from_seed(&[0x42u8; 32])
            .expect("install test EventKey");
        WireEnvGuard {
            prev_home,
            _tmp: tmp,
            _lock: lock,
            _env: env,
        }
    }

    /// End-to-end: register a habit + record a check-in through the real wire
    /// chain, then `spectyn recall` (search_events) must surface it via the FTS5
    /// merge branch. The indexed summary is `"habit <slug> <source>"`, so a
    /// search for the slug finds the check-in even though it has no analysis.json
    /// and is invisible to the file-store walk.
    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn habit_checkin_is_recallable_via_fts5_merge() {
        let _env = isolate_wire_env();
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap();
        let events = home.join(".spectyn-mesh/events");

        create_habit(&HabitDefinition {
            slug: "water".to_string(),
            label: "喝水".to_string(),
            target_frequency: HabitFrequency::Daily,
            tags: vec![],
            created_at: "2026-05-25T00:00:00Z".to_string(),
        })
        .expect("create habit");
        record_checkin(&HabitCheckin {
            habit_slug: "water".to_string(),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            note: None,
            source: HabitCheckinSource::Manual,
        })
        .expect("record check-in");

        // The slug appears in the FTS5-indexed summary ("habit water manual").
        let hits = search_events(&events, None, &RecallFilter::text("water"), 15)
            .expect("recall search");
        assert_eq!(hits.len(), 1, "the habit check-in must be recallable: {hits:?}");
        assert_eq!(hits[0].kind, "habit", "wire meta.json carries kind=Habit");
        assert!(
            hits[0].summary.contains("water"),
            "FTS5 summary surfaces the slug: {}",
            hits[0].summary
        );

        // A keyword in no event → zero hits (proves we are not matching all rows).
        let none = search_events(&events, None, &RecallFilter::text("zzznope"), 15)
            .expect("recall search");
        assert!(none.is_empty(), "unrelated keyword finds nothing: {none:?}");
    }

    /// End-to-end: the `kind=habit` recall filter restricts to wire-store habit
    /// events. A captured note (file store, kind=text) must be EXCLUDED when the
    /// caller asks for `--kind habit`, while the habit check-in is included — the
    /// merge branch applies the same kind filter the file walk does.
    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn recall_kind_habit_filter_excludes_notes_includes_checkins() {
        let _env = isolate_wire_env();
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap();
        let spectyn = home.join(".spectyn-mesh");
        let events = spectyn.join("events");

        // A plain note (file store, projects to kind=text).
        capture_note(&spectyn, "buy more water filters", &["note".into()]).expect("note");
        // A real habit check-in (wire store, kind=habit).
        create_habit(&HabitDefinition {
            slug: "water".to_string(),
            label: "喝水".to_string(),
            target_frequency: HabitFrequency::Daily,
            tags: vec![],
            created_at: "2026-05-25T00:00:00Z".to_string(),
        })
        .expect("create habit");
        record_checkin(&HabitCheckin {
            habit_slug: "water".to_string(),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            note: None,
            source: HabitCheckinSource::Manual,
        })
        .expect("record check-in");

        // Unfiltered "water" matches BOTH the note (summary) and the check-in.
        let both = search_events(&events, None, &RecallFilter::text("water"), 15).expect("all");
        assert_eq!(both.len(), 2, "both the note and the check-in mention water: {both:?}");

        // kind=habit restricts to the wire check-in only.
        let habit_only = search_events(
            &events,
            None,
            &RecallFilter { query: "water", kind: Some("habit"), since: None, mode: RecallMode::default() },
            15,
        )
        .expect("kind=habit");
        assert_eq!(habit_only.len(), 1, "kind=habit keeps only the check-in: {habit_only:?}");
        assert_eq!(habit_only[0].kind, "habit");
    }

    // ─── Hermetic semantic-recall proof (stub embedder, NO network) ──────────
    //
    // The production embedder is local Ollama; these tests inject a deterministic
    // stub via `event_storage_wire::{set_test_embedder, clear_test_embedder}` so
    // the REAL recall entry (`search_events`) exercises the embed→cosine-top-k→
    // merge path with NO Ollama running. They prove (a) semantic recall returns
    // cosine-similarity order through the real entry, (b) it degrades to FTS5
    // keyword hits (no panic) when the embedder is unavailable, and (c) keyword
    // mode ignores the embedder entirely. Fully hermetic: tmp dirs + an explicit
    // db path (no $HOME, no `~`), so they run in the DEFAULT test pass (not
    // `#[ignore]`d like the env-mutating wire integration tests above).

    /// Deterministic stub: maps a marker substring in the text to an explicit
    /// fixed-dim vector so cosine ordering is fully controlled. The query (no
    /// marker) is `[1,0,0]`; events rank near > mid > far against it.
    struct OrderStub;
    impl crate::embeddings::EmbeddingProvider for OrderStub {
        fn model_id(&self) -> &str {
            "order-stub"
        }
        fn dim(&self) -> usize {
            3
        }
        fn embed(
            &self,
            texts: &[String],
        ) -> Result<Vec<Vec<f32>>, crate::embeddings::EmbedError> {
            Ok(texts
                .iter()
                .map(|t| {
                    let t = t.to_lowercase();
                    if t.contains("zznear") {
                        vec![1.0, 0.05, 0.0] // cosine ~0.999 with the query → #1
                    } else if t.contains("zzmid") {
                        vec![0.6, 0.8, 0.0] // cosine 0.6 → #2
                    } else if t.contains("zzfar") {
                        vec![0.0, 1.0, 0.0] // cosine 0.0 → #3
                    } else {
                        vec![1.0, 0.0, 0.0] // the query vector
                    }
                })
                .collect())
        }
    }

    /// Stub that always reports the backend unavailable (Ollama down) — drives
    /// the graceful-degradation path.
    struct DownStub;
    impl crate::embeddings::EmbeddingProvider for DownStub {
        fn model_id(&self) -> &str {
            "down-stub"
        }
        fn dim(&self) -> usize {
            3
        }
        fn embed(
            &self,
            _texts: &[String],
        ) -> Result<Vec<Vec<f32>>, crate::embeddings::EmbedError> {
            Err(crate::embeddings::EmbedError::Unavailable(
                "test: embedder down".into(),
            ))
        }
    }

    /// RAII: clear the thread-local test embedder on drop so a panic can never
    /// leak it into a sibling test sharing the runner thread.
    struct EmbedderGuard;
    impl Drop for EmbedderGuard {
        fn drop(&mut self) {
            crate::event_storage_wire::clear_test_embedder();
        }
    }

    #[test]
    fn semantic_recall_ranks_by_cosine_through_real_recall_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let spectyn = tmp.path().join(".spectyn-mesh");
        std::fs::create_dir_all(&spectyn).unwrap();
        let events = spectyn.join("events");
        let sqlite = spectyn.join("events.sqlite");

        // Capture near→mid→far so the NEWEST event ("far") is the LEAST similar:
        // a naive newest-first ordering would put "far" FIRST — the exact OPPOSITE
        // of the cosine order — so this test passes ONLY if cosine drives the rank
        // (relevances are all distinct, so the timestamp tiebreak is never hit).
        let near = capture_note(&spectyn, "alpha zznear tokens", &["note".into()]).unwrap();
        let mid = capture_note(&spectyn, "beta zzmid tokens", &["note".into()]).unwrap();
        let far = capture_note(&spectyn, "gamma zzfar tokens", &["note".into()]).unwrap();

        let _g = EmbedderGuard;
        crate::event_storage_wire::set_test_embedder(Box::new(OrderStub));
        // Embed each note's summary via the REAL capture-time store fn (now using
        // the injected stub) — proves the store side too, not just recall.
        for (note, text) in [
            (&near, "alpha zznear tokens"),
            (&mid, "beta zzmid tokens"),
            (&far, "gamma zzfar tokens"),
        ] {
            let stored = crate::event_storage_wire::embed_and_store_at(
                &sqlite,
                &note.event_id,
                text,
            )
            .expect("embed_and_store_at");
            assert!(stored, "stub embedder stores a vector for {}", note.event_id);
        }

        // Semantic recall through the REAL entry. Query carries no marker → query
        // vector [1,0,0]; expected cosine order: near > mid > far.
        let hits = search_events(
            &events,
            None,
            &RecallFilter {
                query: "zzquery",
                kind: None,
                since: None,
                mode: RecallMode::Semantic,
            },
            10,
        )
        .unwrap();

        assert_eq!(hits.len(), 3, "all three semantic hits surface: {hits:?}");
        assert_eq!(hits[0].event_id, near.event_id, "highest cosine first");
        assert_eq!(hits[1].event_id, mid.event_id, "middle cosine second");
        assert_eq!(hits[2].event_id, far.event_id, "lowest cosine last");
        assert!(
            hits[0].relevance >= hits[1].relevance && hits[1].relevance >= hits[2].relevance,
            "relevance is monotonically non-increasing (cosine order): {:?}",
            hits.iter().map(|h| h.relevance).collect::<Vec<_>>()
        );
    }

    #[test]
    fn semantic_recall_degrades_to_fts5_when_embedder_unavailable() {
        let tmp = tempfile::tempdir().unwrap();
        let spectyn = tmp.path().join(".spectyn-mesh");
        std::fs::create_dir_all(&spectyn).unwrap();
        let events = spectyn.join("events");

        capture_note(&spectyn, "tokio runtime panic in scheduler", &["note".into()]).unwrap();
        capture_note(&spectyn, "bought groceries today", &["note".into()]).unwrap();

        let _g = EmbedderGuard;
        // Embedder reports unavailable → the semantic leg yields nothing.
        crate::event_storage_wire::set_test_embedder(Box::new(DownStub));

        // Pure Semantic mode must NOT return empty or panic — it falls back to
        // the lexical (file-store) keyword leg and still finds the matching note.
        let hits = search_events(
            &events,
            None,
            &RecallFilter {
                query: "tokio",
                kind: None,
                since: None,
                mode: RecallMode::Semantic,
            },
            10,
        )
        .unwrap();
        assert_eq!(
            hits.len(),
            1,
            "FTS5/keyword fallback returns the matching note: {hits:?}"
        );
        assert!(
            hits[0].summary.contains("tokio"),
            "fallback hit is the tokio note: {}",
            hits[0].summary
        );

        // An unrelated keyword still finds nothing (the fallback is not matching
        // all rows — it is the real lexical leg).
        let none = search_events(
            &events,
            None,
            &RecallFilter {
                query: "zzznope",
                kind: None,
                since: None,
                mode: RecallMode::Semantic,
            },
            10,
        )
        .unwrap();
        assert!(none.is_empty(), "no false positives in the fallback: {none:?}");
    }

    #[test]
    fn keyword_mode_ignores_the_embedder() {
        let tmp = tempfile::tempdir().unwrap();
        let spectyn = tmp.path().join(".spectyn-mesh");
        std::fs::create_dir_all(&spectyn).unwrap();
        let events = spectyn.join("events");
        let sqlite = spectyn.join("events.sqlite");

        let n = capture_note(&spectyn, "alpha zznear tokens", &["note".into()]).unwrap();

        let _g = EmbedderGuard;
        crate::event_storage_wire::set_test_embedder(Box::new(OrderStub));
        crate::event_storage_wire::embed_and_store_at(&sqlite, &n.event_id, "alpha zznear tokens")
            .unwrap();

        // Keyword mode: even with an embedder installed + a stored vector, a
        // query that matches ONLY semantically (no shared keyword) returns
        // nothing — the keyword path is the pre-existing lexical behavior.
        let semantic_only = search_events(
            &events,
            None,
            &RecallFilter {
                query: "zzquery",
                kind: None,
                since: None,
                mode: RecallMode::Keyword,
            },
            10,
        )
        .unwrap();
        assert!(
            semantic_only.is_empty(),
            "keyword mode does not use embeddings: {semantic_only:?}"
        );

        // The SAME query in Semantic mode DOES surface it — proving the
        // difference is the mode, not the fixture.
        let semantic = search_events(
            &events,
            None,
            &RecallFilter {
                query: "zzquery",
                kind: None,
                since: None,
                mode: RecallMode::Semantic,
            },
            10,
        )
        .unwrap();
        assert_eq!(semantic.len(), 1, "semantic mode surfaces the vector-matched note");
        assert_eq!(semantic[0].event_id, n.event_id);

        // And a literal keyword query still works in keyword mode (unchanged).
        let kw = search_events(&events, None, &RecallFilter::text("zznear"), 10).unwrap();
        assert_eq!(kw.len(), 1, "literal keyword still matches in keyword mode");
    }
}
