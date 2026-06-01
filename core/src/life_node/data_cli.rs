//! `phantom data delete --all` — wipe all Life Node events.
//!
//! Scope: removes `~/.phantom-mesh/events/` and its entire subtree. Does
//! NOT touch other phantom-mesh state (`agents.toml`, `identity.key`,
//! `sessions/`, `cluster.db`, etc.). Requires `--yes` to actually delete
//! (defensive default — running without `--yes` returns a "would delete N
//! events" error instead of touching disk).

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct DeleteSummary {
    pub events_dir: PathBuf,
    pub event_count: usize,
    pub bytes_deleted: u64,
}

/// Delete all Life Node events under `<home>/.phantom-mesh/events/`.
///
/// When `confirmed` is `false`, this counts what would be deleted and
/// returns an `Err` describing the scope — nothing is touched on disk.
/// When `true`, performs the deletion and returns the summary.
///
/// Returns an `Ok(DeleteSummary { event_count: 0, .. })` if the events
/// directory doesn't exist (idempotent on a fresh install).
pub fn run_delete_all(home: &Path, confirmed: bool) -> Result<DeleteSummary> {
    let events_dir = home.join(".phantom-mesh").join("events");
    if !events_dir.exists() {
        return Ok(DeleteSummary {
            events_dir,
            event_count: 0,
            bytes_deleted: 0,
        });
    }

    // First pass: tally event count and byte size. Walks one level deep
    // (the event UUID dirs) and sums file sizes within each.
    let mut event_count = 0usize;
    let mut bytes_deleted = 0u64;
    for entry in std::fs::read_dir(&events_dir)
        .with_context(|| format!("read_dir {}", events_dir.display()))?
    {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            event_count += 1;
            for f in std::fs::read_dir(entry.path())? {
                let f = f?;
                if f.file_type()?.is_file() {
                    bytes_deleted += f.metadata()?.len();
                }
            }
        }
    }

    if !confirmed {
        return Err(anyhow!(
            "would delete {} events ({} bytes) from {} — rerun with --yes to confirm",
            event_count,
            bytes_deleted,
            events_dir.display(),
        ));
    }

    std::fs::remove_dir_all(&events_dir)
        .with_context(|| format!("remove_dir_all {}", events_dir.display()))?;

    Ok(DeleteSummary {
        events_dir,
        event_count,
        bytes_deleted,
    })
}

/// Resolve an event id (or an unambiguous id prefix) to the full id under
/// `events_dir`. Errors if nothing matches, or if a short prefix is ambiguous
/// (so a typo can never silently target the wrong event). Shared by
/// `delete_event` and `phantom event show`.
pub fn resolve_event_id(events_dir: &Path, id_or_prefix: &str) -> Result<String> {
    let id_or_prefix = id_or_prefix.trim();
    if id_or_prefix.is_empty() {
        return Err(anyhow!("empty event id"));
    }
    if !events_dir.exists() {
        return Err(anyhow!("no events found"));
    }
    let mut matches: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(events_dir)
        .with_context(|| format!("read_dir {}", events_dir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name == id_or_prefix || name.starts_with(id_or_prefix) {
            matches.push(name);
        }
    }
    match matches.len() {
        0 => Err(anyhow!("no event matching '{}'", id_or_prefix)),
        1 => Ok(matches.into_iter().next().unwrap()),
        n => Err(anyhow!(
            "'{}' is ambiguous — {} events match; use a longer id",
            id_or_prefix,
            n
        )),
    }
}

/// Delete a single Life Node event by its id (or an unambiguous id prefix) under
/// `<home>/.phantom-mesh/events/`. Reversibility at the granular level: take back
/// one mis-captured note without nuking the whole log. Returns the resolved full
/// event id.
pub fn delete_event(home: &Path, id_or_prefix: &str) -> Result<String> {
    let events_dir = home.join(".phantom-mesh").join("events");
    let id = resolve_event_id(&events_dir, id_or_prefix)?;
    std::fs::remove_dir_all(events_dir.join(&id))
        .with_context(|| format!("remove event {}", id))?;
    Ok(id)
}

// ── `phantom data export` — portability: get your life log OUT ──────────────
//
// Counterpart to `delete`: the encryption-first / "your data is yours" ethos
// means a user must be able to take their data with them. Reuses the tested
// `recall::search_events` loader (decrypts via identity.key when present), then
// serializes chronologically (oldest-first) to JSON or Markdown.

/// Output format for `phantom data export`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Json,
    Markdown,
}

#[derive(Debug)]
pub struct ExportResult {
    pub event_count: usize,
    pub body: String,
}

/// Export Life Node events matching `filter` (empty filter = everything) from
/// `<home>/.phantom-mesh/events/`, oldest-first, as JSON or Markdown. Read-only.
pub fn run_export(
    home: &Path,
    format: ExportFormat,
    filter: &crate::life_node::recall::RecallFilter,
) -> Result<ExportResult> {
    let phantom = home.join(".phantom-mesh");
    let events_dir = phantom.join("events");
    let key = crate::life_node::key_derivation::load_event_key(&phantom.join("identity.key")).ok();
    // usize::MAX → no truncation; search_events returns newest-first, so reverse
    // for a chronological (oldest-first) export.
    let mut hits = crate::life_node::recall::search_events(&events_dir, key, filter, usize::MAX)
        .map_err(|e| anyhow!("read events {}: {}", events_dir.display(), e))?;
    hits.reverse();
    // Food (and any strict-JSON provider) stores the analysis as a JSON blob in
    // `summary`; clean it to the inner text so exports are readable, not raw JSON.
    for h in &mut hits {
        h.summary = crate::life_node::daily_review::clean_summary(&h.summary);
    }

    let body = match format {
        ExportFormat::Json => serde_json::to_string_pretty(&hits)
            .map_err(|e| anyhow!("serialize json: {}", e))?,
        ExportFormat::Markdown => {
            let mut s = String::from("# Life Node export\n\n");
            for h in &hits {
                s.push_str(&format!("- {}  [{}]  {}\n", h.timestamp, h.kind, h.summary));
            }
            s
        }
    };
    Ok(ExportResult {
        event_count: hits.len(),
        body,
    })
}

// ── `phantom data stats` / TUI `/stats` — life-log rollup ───────────────────
//
// Aggregate across the whole event store: total, by-kind breakdown, date span,
// 7-day activity. Read-only; reuses the tested recall loader.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifeStats {
    pub total: usize,
    /// `(kind, count)` sorted by count desc, then kind asc.
    pub by_kind: Vec<(String, usize)>,
    /// `YYYY-MM-DD` of the earliest / latest event (None when empty).
    pub earliest: Option<String>,
    pub latest: Option<String>,
    /// Events in the trailing 7-day window (inclusive of today).
    pub last_7d: usize,
}

/// Compute a rollup over all Life Node events under `<home>/.phantom-mesh`.
pub fn compute_stats(home: &Path) -> Result<LifeStats> {
    let phantom = home.join(".phantom-mesh");
    let events_dir = phantom.join("events");
    let key = crate::life_node::key_derivation::load_event_key(&phantom.join("identity.key")).ok();
    let hits = crate::life_node::recall::search_events(
        &events_dir,
        key,
        &crate::life_node::recall::RecallFilter::text(""),
        usize::MAX,
    )
    .map_err(|e| anyhow!("read events {}: {}", events_dir.display(), e))?;

    let cutoff_7d = (chrono::Local::now() - chrono::Duration::days(7))
        .format("%Y-%m-%d")
        .to_string();

    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut earliest: Option<String> = None;
    let mut latest: Option<String> = None;
    let mut last_7d = 0usize;
    for h in &hits {
        *counts.entry(h.kind.clone()).or_insert(0) += 1;
        // SPEC-16 T-STOR-01: timestamps are UTC; bucket by the user's LOCAL date
        // (and compare the 7-day window in local terms) so stats line up with
        // what the per-day review shows.
        let day = crate::event_storage_wire::ts_local_date(&h.timestamp);
        if !day.is_empty() {
            if earliest.as_ref().is_none_or(|e| &day < e) {
                earliest = Some(day.clone());
            }
            if latest.as_ref().is_none_or(|l| &day > l) {
                latest = Some(day.clone());
            }
        }
        // Trailing 7-day window: event's local date ≥ the YYYY-MM-DD floor.
        if day.as_str() >= cutoff_7d.as_str() {
            last_7d += 1;
        }
    }
    let mut by_kind: Vec<(String, usize)> = counts.into_iter().collect();
    by_kind.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    Ok(LifeStats {
        total: hits.len(),
        by_kind,
        earliest,
        latest,
        last_7d,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_dummy_event(home: &Path, id: &str) {
        let dir = home.join(".phantom-mesh").join("events").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("meta.json"), b"x").unwrap();
        std::fs::write(dir.join("modality_0.jpeg"), [0u8; 1024]).unwrap();
    }

    #[test]
    fn export_json_and_markdown_round_trip() {
        use crate::life_node::note_capture::capture_note;
        use crate::life_node::recall::RecallFilter;

        let tmp = TempDir::new().unwrap();
        let phantom = tmp.path().join(".phantom-mesh");
        std::fs::create_dir_all(&phantom).unwrap();
        capture_note(&phantom, "first thing", &["note".into()]).unwrap();
        capture_note(&phantom, "second thing", &["note".into()]).unwrap();

        // JSON export: parses, has both summaries.
        let json = run_export(tmp.path(), ExportFormat::Json, &RecallFilter::text("")).unwrap();
        assert_eq!(json.event_count, 2);
        let parsed: serde_json::Value = serde_json::from_str(&json.body).unwrap();
        let arr = parsed.as_array().expect("json export is an array");
        assert_eq!(arr.len(), 2);
        assert!(json.body.contains("first thing") && json.body.contains("second thing"));

        // Markdown export: a bullet per event.
        let md = run_export(tmp.path(), ExportFormat::Markdown, &RecallFilter::text("")).unwrap();
        assert_eq!(md.event_count, 2);
        assert!(md.body.contains("# Life Node export"));
        assert_eq!(md.body.matches("\n- ").count(), 2, "one bullet per event:\n{}", md.body);

        // Filter carries through: a query that matches one note exports one.
        let one = run_export(tmp.path(), ExportFormat::Json, &RecallFilter::text("first")).unwrap();
        assert_eq!(one.event_count, 1);
    }

    #[test]
    fn compute_stats_aggregates_total_and_by_kind() {
        use crate::life_node::note_capture::capture_note;
        let tmp = TempDir::new().unwrap();
        let phantom = tmp.path().join(".phantom-mesh");
        std::fs::create_dir_all(&phantom).unwrap();
        capture_note(&phantom, "a", &["note".into()]).unwrap();
        capture_note(&phantom, "b", &["note".into()]).unwrap();

        let s = compute_stats(tmp.path()).unwrap();
        assert_eq!(s.total, 2);
        // both notes project to kind=text
        assert_eq!(s.by_kind, vec![("text".to_string(), 2)]);
        assert!(s.earliest.is_some() && s.latest.is_some());
        assert_eq!(s.last_7d, 2, "fresh notes are within the 7-day window");
    }

    #[test]
    fn delete_event_removes_one_by_id_and_prefix() {
        use crate::life_node::note_capture::capture_note;
        let tmp = TempDir::new().unwrap();
        let phantom = tmp.path().join(".phantom-mesh");
        std::fs::create_dir_all(&phantom).unwrap();
        let a = capture_note(&phantom, "keep me", &["note".into()]).unwrap().event_id;
        let b = capture_note(&phantom, "delete me", &["note".into()]).unwrap().event_id;

        // delete b by an 8-char prefix → resolves to the full id, removes it
        let deleted = delete_event(tmp.path(), &b[..8]).unwrap();
        assert_eq!(deleted, b);
        assert!(!phantom.join("events").join(&b).exists(), "b removed");
        assert!(phantom.join("events").join(&a).exists(), "a kept");

        // unknown id → error, nothing else touched
        assert!(delete_event(tmp.path(), "zzzzzzzz").is_err());
        assert!(phantom.join("events").join(&a).exists(), "a still kept");

        // empty id is rejected (never prefix-matches everything)
        assert!(delete_event(tmp.path(), "").is_err());
        assert!(phantom.join("events").join(&a).exists(), "a survives empty-id guard");
    }

    #[test]
    fn compute_stats_empty_store() {
        let tmp = TempDir::new().unwrap();
        let s = compute_stats(tmp.path()).unwrap();
        assert_eq!(s.total, 0);
        assert!(s.by_kind.is_empty());
        assert!(s.earliest.is_none() && s.latest.is_none());
        assert_eq!(s.last_7d, 0);
    }

    #[test]
    fn export_empty_store_is_ok() {
        let tmp = TempDir::new().unwrap();
        let r = run_export(
            tmp.path(),
            ExportFormat::Json,
            &crate::life_node::recall::RecallFilter::text(""),
        )
        .unwrap();
        assert_eq!(r.event_count, 0);
        assert_eq!(r.body.trim(), "[]");
    }

    #[test]
    fn phantom_data_delete_all_removes_events_dir() {
        let tmp = TempDir::new().unwrap();
        write_dummy_event(tmp.path(), "evt-1");
        write_dummy_event(tmp.path(), "evt-2");

        let summary = run_delete_all(tmp.path(), true).unwrap();

        assert_eq!(summary.event_count, 2);
        assert!(summary.bytes_deleted >= 2 * 1024);
        assert!(
            !tmp.path().join(".phantom-mesh").join("events").exists(),
            "events dir must be gone after run_delete_all(confirmed=true)"
        );
    }

    #[test]
    fn dry_run_without_confirmed_does_not_delete() {
        let tmp = TempDir::new().unwrap();
        write_dummy_event(tmp.path(), "evt-1");
        let r = run_delete_all(tmp.path(), false);
        assert!(r.is_err(), "non-confirmed run must err to force --yes");
        assert!(
            tmp.path()
                .join(".phantom-mesh")
                .join("events")
                .join("evt-1")
                .exists(),
            "events dir must NOT be touched without confirmation"
        );
        // Error message should explain the gate.
        let msg = format!("{:#}", r.unwrap_err());
        assert!(
            msg.contains("--yes"),
            "error msg should mention --yes; got: {}",
            msg
        );
    }

    #[test]
    fn delete_all_does_not_touch_other_phantom_files() {
        let tmp = TempDir::new().unwrap();
        // Sibling state that MUST survive a `data delete --all`.
        let phantom_dir = tmp.path().join(".phantom-mesh");
        std::fs::create_dir_all(&phantom_dir).unwrap();
        std::fs::write(phantom_dir.join("agents.toml"), b"[core]").unwrap();
        std::fs::write(phantom_dir.join("identity.key"), [0u8; 64]).unwrap();
        std::fs::create_dir_all(phantom_dir.join("sessions")).unwrap();
        std::fs::write(phantom_dir.join("sessions").join("a.json"), b"{}").unwrap();
        write_dummy_event(tmp.path(), "evt-1");

        run_delete_all(tmp.path(), true).unwrap();

        assert!(
            phantom_dir.join("agents.toml").exists(),
            "agents.toml must survive"
        );
        assert!(
            phantom_dir.join("identity.key").exists(),
            "identity.key must survive"
        );
        assert!(
            phantom_dir.join("sessions").exists(),
            "sessions/ must survive"
        );
        assert!(
            phantom_dir.join("sessions").join("a.json").exists(),
            "session file must survive"
        );
        assert!(!phantom_dir.join("events").exists(), "events/ must be gone");
    }

    #[test]
    fn delete_all_idempotent_on_missing_events_dir() {
        let tmp = TempDir::new().unwrap();
        // No `.phantom-mesh/events/` at all.
        let s = run_delete_all(tmp.path(), true).unwrap();
        assert_eq!(s.event_count, 0);
        assert_eq!(s.bytes_deleted, 0);
    }
}
