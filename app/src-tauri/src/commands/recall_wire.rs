// Tauri command for the app's Recall (content search) surface — the app
// counterpart of the TUI `/recall` + CLI `phantom recall` (BIG-GOAL P2 Life
// Track). Searches the SAME file event store (~/.phantom-mesh/events) via
// life_node::recall::search_events, so all three surfaces find the same events.
// Read-only + offline; encrypted events decrypt only with the key (skipped
// without one — never surfaces ciphertext).

use serde::Serialize;

use phantom_mesh::life_node::key_derivation::load_event_key;
use phantom_mesh::life_node::recall::{search_events, RecallFilter, RecallMode};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecallHitView {
    pub event_id: String,
    pub timestamp: String,
    pub kind: String,
    pub summary: String,
}

/// Search past Life Node events by content. `query` empty → recent events.
/// `kind`/`since` optional filters; `limit` defaults to 50.
#[tauri::command]
pub async fn recall_search(
    query: String,
    kind: Option<String>,
    since: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<RecallHitView>, String> {
    let home = dirs::home_dir().ok_or_else(|| "recall.no_home_dir".to_string())?;
    let phantom = home.join(".phantom-mesh");
    let key = load_event_key(&phantom.join("identity.key")).ok();
    let filter = RecallFilter {
        query: query.trim(),
        kind: kind.as_deref(),
        since: since.as_deref(),
        // Desktop recall stays lexical/offline (no embedder dependency); the
        // CLI `recall --mode` exposes semantic/hybrid explicitly.
        mode: RecallMode::Keyword,
    };
    let hits = search_events(&phantom.join("events"), key, &filter, limit.unwrap_or(50))
        .map_err(|e| format!("recall.failed: {e:?}"))?;
    Ok(hits
        .into_iter()
        .map(|h| RecallHitView {
            event_id: h.event_id,
            timestamp: h.timestamp,
            kind: h.kind,
            summary: h.summary,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn recall_search_returns_wellformed() {
        // Real home; with or without events this must return Ok (never panic).
        let hits = recall_search("zzz_no_match_expected".to_string(), None, None, Some(5))
            .await
            .expect("recall_search ok");
        assert!(hits.len() <= 5);
    }
}
