// Tauri command surface for SPEC-16 event storage (read/query side).
//
// Exposes the real query/search fns from `spectyn_mesh::event_storage_wire`:
//   - events_query(query)        → Vec<EventRecord>  (metadata only; bodies stay encrypted)
//   - events_search(query, limit) → Vec<String>      (FTS5 → matching event ids)
//
// query_events reads only the plaintext meta.json sidecars (no keystore /
// decryption), so it's safe to surface for a "life timeline" browser. catch_unwind
// guards the one remaining Stage-2 stub path; EventStoreError has Display.

use std::panic::{catch_unwind, AssertUnwindSafe};

use spectyn_mesh::event_storage_wire::{
    self, EventRecord, EventStoreError, EventStoreQuery,
};

const NOT_YET_WIRED: &str = "events.not_yet_wired: event-storage helper unavailable";

fn run<T>(f: impl FnOnce() -> Result<T, EventStoreError>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(NOT_YET_WIRED.to_string()),
    }
}

#[tauri::command]
pub async fn events_query(query: EventStoreQuery) -> Result<Vec<EventRecord>, String> {
    run(|| event_storage_wire::query_events(&query))
}

#[tauri::command]
pub async fn events_search(query: String, limit: usize) -> Result<Vec<String>, String> {
    run(|| event_storage_wire::search_fts5(&query, limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn events_query_returns_wellformed_result() {
        // Empty/missing store → Ok([]) or a typed EventStoreError; never a panic.
        let q: EventStoreQuery = serde_json::from_str(
            r#"{"dateIso":null,"kind":null,"tag":null,"limit":20,"offset":null}"#,
        )
        .expect("parse query");
        match events_query(q).await {
            Ok(_) => {}
            Err(e) => assert!(!e.is_empty(), "error string should be non-empty"),
        }
    }

    #[tokio::test]
    async fn events_search_returns_wellformed_result() {
        match events_search("water".to_string(), 10).await {
            Ok(ids) => assert!(ids.len() <= 10),
            Err(e) => assert!(!e.is_empty()),
        }
    }

    #[test]
    fn event_store_query_deserializes_from_camelcase() {
        let q: EventStoreQuery = serde_json::from_str(
            r#"{"dateIso":"2026-05-28","kind":null,"tag":"fat_loss","limit":50,"offset":0}"#,
        )
        .expect("parse");
        assert_eq!(q.limit, Some(50));
    }
}
