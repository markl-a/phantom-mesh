// Tauri command for the Dashboard's life-log stats card — app counterpart of
// the TUI `/stats` + CLI `phantom data stats` (BIG-GOAL P2 Life Track).
// Wraps life_node::data_cli::compute_stats over the shared event store.
// Read-only; encrypted events count only when the key is present.

use serde::Serialize;

use phantom_mesh::life_node::data_cli::{compute_stats, delete_event, run_export, ExportFormat};
use phantom_mesh::life_node::recall::{RecallFilter, RecallMode};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KindCount {
    pub kind: String,
    pub count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifeStatsView {
    pub total: usize,
    pub by_kind: Vec<KindCount>,
    pub earliest: Option<String>,
    pub latest: Option<String>,
    pub last_7d: usize,
}

/// Roll up all Life Node events: total · date span · last-7d · by-kind.
#[tauri::command]
pub async fn life_stats() -> Result<LifeStatsView, String> {
    let home = dirs::home_dir().ok_or_else(|| "life_stats.no_home_dir".to_string())?;
    let s = compute_stats(&home).map_err(|e| format!("life_stats.failed: {e}"))?;
    Ok(LifeStatsView {
        total: s.total,
        by_kind: s.by_kind.into_iter().map(|(kind, count)| KindCount { kind, count }).collect(),
        earliest: s.earliest,
        latest: s.latest,
        last_7d: s.last_7d,
    })
}

/// Export Life Node events to `~/.phantom-mesh/exports/life-export-<ts>.<ext>`
/// (JSON or Markdown) and return the written path + event count. App counterpart
/// of `phantom data export`. Optional `kind` (food/focus/habit/text) and `since`
/// (YYYY-MM-DD) narrow the export. Writes to a fixed dir (no save-dialog plugin).
#[tauri::command]
pub async fn data_export(
    format: String,
    kind: Option<String>,
    since: Option<String>,
) -> Result<String, String> {
    let home = dirs::home_dir().ok_or_else(|| "data_export.no_home_dir".to_string())?;
    let (fmt, ext) = match format.as_str() {
        "markdown" | "md" => (ExportFormat::Markdown, "md"),
        _ => (ExportFormat::Json, "json"),
    };
    // Empty/blank filter fields → no constraint.
    let kind = kind.filter(|k| !k.trim().is_empty());
    let since = since.filter(|s| !s.trim().is_empty());
    let filter = RecallFilter {
        query: "",
        kind: kind.as_deref(),
        since: since.as_deref(),
        // Export lists everything offline — keep it keyword-only (matches the
        // CLI export path; never depends on or calls the embedder).
        mode: RecallMode::Keyword,
    };
    let res = run_export(&home, fmt, &filter).map_err(|e| format!("data_export.failed: {e}"))?;
    let dir = home.join(".phantom-mesh").join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| format!("data_export.mkdir: {e}"))?;
    let suffix = kind.as_deref().map(|k| format!("-{k}")).unwrap_or_default();
    let path = dir.join(format!(
        "life-export-{}{}.{}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        suffix,
        ext
    ));
    std::fs::write(&path, res.body).map_err(|e| format!("data_export.write: {e}"))?;
    Ok(format!("{} · {} events", path.display(), res.event_count))
}

/// Delete a single Life Node event by id (or unambiguous prefix). Permanently
/// removes the event directory from the store — BIG-GOAL reversibility: your
/// data is yours to remove, one entry at a time. App counterpart of `phantom
/// data delete <event-id>`. Returns the full id of the deleted event; errors on
/// empty / no-match / ambiguous-prefix so the UI can surface why nothing went.
#[tauri::command]
pub async fn event_delete(event_id: String) -> Result<String, String> {
    let home = dirs::home_dir().ok_or_else(|| "event_delete.no_home_dir".to_string())?;
    delete_event(&home, &event_id).map_err(|e| format!("event_delete.failed: {e}"))
}

/// Reveal the exports folder in the OS file manager (macOS Finder / Windows
/// Explorer / Linux xdg-open). Best-effort; returns the folder path.
#[tauri::command]
pub async fn open_exports_folder() -> Result<String, String> {
    let home = dirs::home_dir().ok_or_else(|| "open_exports.no_home_dir".to_string())?;
    let dir = home.join(".phantom-mesh").join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| format!("open_exports.mkdir: {e}"))?;
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(&dir)
        .spawn()
        .map_err(|e| format!("open_exports.spawn: {e}"))?;
    Ok(dir.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn data_export_writes_file_and_reports_path() {
        let out = data_export("json".to_string(), None, None).await.expect("export ok");
        assert!(out.contains("life-export-") && out.contains(".json"), "got {out}");
        assert!(out.contains("events"), "reports count: {out}");
    }

    #[tokio::test]
    async fn data_export_filtered_by_kind_names_the_file() {
        // A kind filter narrows the export + tags the filename. Never panics.
        let out = data_export("markdown".to_string(), Some("focus".to_string()), None)
            .await
            .expect("filtered export ok");
        assert!(out.contains("-focus.md"), "kind-tagged filename: {out}");
    }

    #[tokio::test]
    async fn event_delete_rejects_empty_and_missing() {
        // Empty id → error, never touches the store.
        assert!(event_delete("".to_string()).await.is_err());
        // A 32-char id that cannot match any real event → error (non-destructive).
        assert!(event_delete("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz".to_string())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn life_stats_returns_wellformed() {
        let s = life_stats().await.expect("life_stats ok");
        // Each event has exactly one kind, so the by-kind counts sum to total.
        let sum_kinds: usize = s.by_kind.iter().map(|k| k.count).sum();
        assert_eq!(sum_kinds, s.total, "by_kind counts sum to total");
        assert!(s.last_7d <= s.total, "last_7d within total");
    }
}
