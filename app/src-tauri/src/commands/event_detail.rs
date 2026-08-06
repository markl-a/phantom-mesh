// Tauri command for the timeline event-detail view — app counterpart of the CLI
// `spectyn event show <id>` / TUI `/event <id>`. Reads ONE Life Node event's
// metadata + LLM analysis (summary / suggestion / goal-impact) via the same
// real, key-aware `EventStore` the daily review uses, so encrypted events
// decrypt transparently when an identity key is present. Analysis is optional
// (most events ship without one); the summary is cleaned of food's strict-JSON
// blob via daily_review::clean_summary so it reads as prose.

use serde::Serialize;

use spectyn_mesh::life_node::daily_review::clean_summary;
use spectyn_mesh::life_node::storage::EventStore;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDetailView {
    pub event_id: String,
    pub timestamp: String,
    pub kind: String,
    pub tags: Vec<String>,
    /// Present only when the event has a sibling analysis.json (and it decrypts).
    pub summary: Option<String>,
    pub suggestion: Option<String>,
    pub goal_impact: Option<String>,
    pub confidence: Option<f32>,
    pub model_id: Option<String>,
}

/// Load one event's full detail by id. Errors when the id is empty or no event
/// dir matches; a present-but-undecryptable analysis degrades to `summary: null`
/// rather than failing the whole read (the metadata is still useful).
#[tauri::command]
pub async fn event_show(event_id: String) -> Result<EventDetailView, String> {
    let id = event_id.trim();
    if id.is_empty() {
        return Err("event_show.empty: event id is required".to_string());
    }
    let home = dirs::home_dir().ok_or_else(|| "event_show.no_home_dir".to_string())?;
    let events_dir = home.join(".spectyn-mesh").join("events");
    let store = EventStore::with_identity_file(
        &events_dir,
        &home.join(".spectyn-mesh").join("identity.key"),
    );

    let meta = store
        .read_meta(id)
        .map_err(|e| format!("event_show.not_found: {e}"))?;
    let kind = serde_json::to_string(&meta.kind)
        .unwrap_or_else(|_| "\"unknown\"".to_string())
        .trim_matches('"')
        .to_string();

    // analysis.json is optional + may be locked without the key → degrade to None.
    let analysis = store.read_analysis(id).ok();
    Ok(EventDetailView {
        event_id: meta.event_id,
        timestamp: meta.timestamp,
        kind,
        tags: meta.tags,
        summary: analysis.as_ref().map(|a| clean_summary(&a.summary)),
        suggestion: analysis
            .as_ref()
            .and_then(|a| a.suggestion.as_deref())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        goal_impact: analysis
            .as_ref()
            .and_then(|a| a.goal_impact.as_deref())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        confidence: analysis.as_ref().and_then(|a| a.confidence),
        model_id: analysis
            .as_ref()
            .map(|a| a.model_id.trim().to_string())
            .filter(|s| !s.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn event_show_rejects_empty_and_missing() {
        assert!(event_show("".to_string()).await.is_err());
        assert!(event_show("zzzzzzzz-nonexistent-event-id".to_string())
            .await
            .is_err());
    }
}
