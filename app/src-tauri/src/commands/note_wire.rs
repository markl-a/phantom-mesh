// Tauri command for quick text-note capture — app counterpart of the TUI
// `/note` + CLI `phantom note` (BIG-GOAL P2 Life Track). Writes a kind="note"
// Life Node event (+ sibling analysis) via life_node::note_capture::capture_note
// to the shared event store, so notes appear in the timeline / review / recall
// across all surfaces. Encrypted at rest when an identity key is present.

use phantom_mesh::life_node::note_capture::capture_note;

/// Capture `text` as a Life Node note event; returns the new event id.
#[tauri::command]
pub async fn note_capture(text: String, tags: Option<Vec<String>>) -> Result<String, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("note.empty: text is required".to_string());
    }
    let home = dirs::home_dir().ok_or_else(|| "note.no_home_dir".to_string())?;
    let phantom = home.join(".phantom-mesh");
    let tags = tags.unwrap_or_default();
    let captured = capture_note(&phantom, t, &tags).map_err(|e| format!("note.failed: {e}"))?;
    Ok(captured.event_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn note_capture_rejects_empty_and_accepts_text() {
        // Empty → error, never writes.
        assert!(note_capture("   ".to_string(), None).await.is_err());
        // Real text → returns a non-empty event id (writes to the real store).
        let id = note_capture("comprehensive note-capture test".to_string(), None)
            .await
            .expect("note capture ok");
        assert!(!id.is_empty(), "event id returned");
    }
}
