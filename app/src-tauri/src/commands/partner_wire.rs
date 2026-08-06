// Tauri command surface for the life-partner brain (NORTH-STAR Q2, partner MVP).
//
// `partner_latest_reflection` reads the most recent `kind:"reflection"` record
// out of the human-usage ledger (`~/.spectyn-mesh/partner-signals.jsonl`, the
// same file `core/src/partner.rs::daily_reflection` appends to once a day via
// the coach daemon). It's read-only and offline — no LLM / network — so the
// desktop "每日對齊反思" panel can surface the last reflection the proactive
// half produced. Returns `None` when the ledger is missing or holds no
// reflection record yet (a fresh install before the first 21:00 coach run).

use std::fs;

use spectyn_mesh::partner::signals_path;
use serde::Serialize;
use serde_json::Value;

/// The latest daily alignment reflection, as surfaced to the desktop panel.
#[derive(Serialize)]
pub struct LatestReflection {
    /// The reflection text the partner produced.
    pub text: String,
    /// The deterministic one-line day summary the reflection was built from.
    pub summary: String,
    /// Unix-seconds timestamp of the ledger record (when it was written).
    pub ts: u64,
}

/// Return the most recent reflection record from the partner ledger, or `None`
/// if the ledger is absent / has no reflection yet. Never errors on a missing
/// file — an empty ledger is a normal pre-first-run state, surfaced as `None`.
#[tauri::command]
pub async fn partner_latest_reflection() -> Result<Option<LatestReflection>, String> {
    let path = signals_path();
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        // No ledger yet (fresh install / coach hasn't run) → no reflection.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("partner.read_failed: {e}")),
    };

    // Scan from the end so we surface the most recent reflection cheaply.
    for line in content.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let rec: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue, // skip a malformed line rather than fail the read
        };
        if rec.get("kind").and_then(Value::as_str) != Some("reflection") {
            continue;
        }
        let payload = rec.get("payload").cloned().unwrap_or(Value::Null);
        let text = payload
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if text.trim().is_empty() {
            continue;
        }
        let summary = payload
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let ts = rec.get("ts").and_then(Value::as_u64).unwrap_or(0);
        return Ok(Some(LatestReflection { text, summary, ts }));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// `signals_path` honours `SPECTYN_PARTNER_SIGNALS`; point it at a temp file
    /// so the test reads a known ledger and never touches the real one.
    #[tokio::test]
    async fn reads_latest_reflection_and_skips_other_kinds() {
        let dir = std::env::temp_dir().join(format!("spectyn-reflect-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("partner-signals.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, r#"{{"ts":100,"kind":"interaction","payload":{{"user":"hi"}}}}"#).unwrap();
        writeln!(
            f,
            r#"{{"ts":200,"kind":"reflection","payload":{{"text":"old","summary":"s1"}}}}"#
        )
        .unwrap();
        writeln!(f, r#"{{"ts":300,"kind":"sensor","payload":{{"lat":1}}}}"#).unwrap();
        writeln!(
            f,
            r#"{{"ts":400,"kind":"reflection","payload":{{"text":"newest","summary":"s2"}}}}"#
        )
        .unwrap();
        drop(f);

        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &path);
        let got = partner_latest_reflection().await.unwrap().unwrap();
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");

        assert_eq!(got.text, "newest", "scans from the end for the latest");
        assert_eq!(got.summary, "s2");
        assert_eq!(got.ts, 400);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn missing_ledger_is_none_not_error() {
        let path = std::env::temp_dir().join(format!("spectyn-reflect-absent-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        std::env::set_var("SPECTYN_PARTNER_SIGNALS", &path);
        let got = partner_latest_reflection().await.unwrap();
        std::env::remove_var("SPECTYN_PARTNER_SIGNALS");
        assert!(got.is_none(), "absent ledger → Ok(None), never an error");
    }
}
