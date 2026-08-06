//! E2E for E004 — spawn `spectyn serve`, capture an event via `spectyn
//! event capture`, then verify the new on-disk `meta.json` is
//! age-encrypted (NOT plain JSON). If Gemini cooperates (no rate-limit),
//! also verify GET /api/events/:id/analysis decrypts and returns the
//! analysis.
//!
//! Gated on `GEMINI_API_KEY`. Without a key, prints SKIP and exits 0 —
//! `life_node::storage::tests::*` covers the in-process crypto path on
//! every CI run regardless.
//!
//! Test isolation note: `dirs::home_dir()` on Windows uses
//! `SHGetKnownFolderPath` and does NOT honor `USERPROFILE`/`HOME` env
//! overrides, so we can't redirect the daemon to a temp HOME on this
//! platform. Instead we diff the operator's real `~/.spectyn-mesh/events/`
//! before and after the capture, identify the test's new event dirs,
//! assert encryption on them, then clean them up so the operator's
//! events dir is restored to its pre-test state.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

fn spectyn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_spectyn")
}

/// Snapshot the set of event IDs currently in `events_dir`. Returns an
/// empty set if the dir doesn't exist (fresh install).
fn snapshot_event_ids(events_dir: &PathBuf) -> HashSet<String> {
    std::fs::read_dir(events_dir)
        .map(|rd| {
            rd.filter_map(|r| r.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test(flavor = "current_thread")]
async fn captured_event_on_disk_is_age_encrypted() {
    let Ok(gemini_key) = std::env::var("GEMINI_API_KEY") else {
        eprintln!(
            "SKIPPED: captured_event_on_disk_is_age_encrypted — GEMINI_API_KEY unset \
             (covered in-process by \
             life_node::storage::tests::encrypted_store_writes_age_format_on_disk)"
        );
        return;
    };

    let bin = spectyn_bin();

    // Resolve the operator's real ~/.spectyn-mesh — both the daemon
    // (via dirs::home_dir()) and this test must agree on the same path
    // for the dir-diff to work. We deliberately do NOT override HOME/
    // USERPROFILE because dirs::home_dir() ignores them on Windows.
    let home = dirs::home_dir().expect("home_dir");
    let spectyn_dir = home.join(".spectyn-mesh");
    let events_dir = spectyn_dir.join("events");

    // Skip if identity.key is missing — capture would 500 before
    // write_event runs, defeating the on-disk check. This is the
    // "fresh install" case; the operator's deployment runbook
    // generates identity.key.
    if !spectyn_dir.join("identity.key").exists() {
        eprintln!(
            "SKIPPED: captured_event_on_disk_is_age_encrypted — {} missing \
             (operator hasn't bootstrapped identity yet)",
            spectyn_dir.join("identity.key").display()
        );
        return;
    }

    // Snapshot the existing event dirs so we can identify the test's
    // additions afterwards.
    let before: HashSet<String> = snapshot_event_ids(&events_dir);

    // Free port: bind + immediately drop so the daemon can grab it.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    drop(listener);

    // Daemon handle: kept in scope so kill_on_drop fires at test end.
    let _child = tokio::process::Command::new(bin)
        .arg("serve")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .env("SPECTYN_NODE_NAME", "e004-e2e")
        .env("GEMINI_API_KEY", &gemini_key)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spectyn serve must spawn");

    // Wait for /healthz, up to 10 seconds.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let healthz = format!("http://127.0.0.1:{}/healthz", port);
    let mut ready = false;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if client
            .get(&healthz)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
        {
            ready = true;
            break;
        }
    }
    assert!(ready, "spectyn serve never became healthy within 10s");

    // Dispatch a text-only event through the CLI. Gemini accepts
    // text-only multimodal so we don't need a jpeg fixture.
    let coord = format!("http://127.0.0.1:{}", port);
    let capture_out = tokio::process::Command::new(bin)
        .arg("event")
        .arg("capture")
        .arg("--kind")
        .arg("note")
        .arg("--text")
        .arg("e004-encryption-e2e-smoke")
        .arg("--coord")
        .arg(&coord)
        .env("GEMINI_API_KEY", &gemini_key)
        .output()
        .await
        .expect("spectyn event capture must spawn");

    let stderr_text = String::from_utf8_lossy(&capture_out.stderr).into_owned();
    let stdout_text = String::from_utf8_lossy(&capture_out.stdout).into_owned();
    let capture_succeeded = capture_out.status.success();
    let was_rate_limited = stderr_text.contains("rate limited")
        || stderr_text.contains("429")
        || stderr_text.contains("RESOURCE_EXHAUSTED");

    // Diff: which event dirs are new since `before`?
    let after: HashSet<String> = snapshot_event_ids(&events_dir);
    let new_event_ids: Vec<String> = after.difference(&before).cloned().collect();

    // Cleanup closure — runs whether assertions pass or fail so the
    // operator's events dir is restored to its pre-test state.
    let cleanup = |ids: &[String]| {
        for id in ids {
            let _ = std::fs::remove_dir_all(events_dir.join(id));
        }
    };

    if new_event_ids.is_empty() {
        if was_rate_limited {
            eprintln!(
                "SKIPPED: captured_event_on_disk_is_age_encrypted — Gemini rate-limit \
                 hit before write_event reached disk; nothing to verify. stderr=\n{}",
                stderr_text
            );
            return;
        }
        panic!(
            "no new event dirs created.\n--- capture stderr ---\n{}\n--- capture stdout ---\n{}",
            stderr_text, stdout_text
        );
    }

    // CORE INVARIANT — every new meta.json on disk MUST be age-encrypted.
    let event_id = new_event_ids[0].clone();
    let meta_path = events_dir.join(&event_id).join("meta.json");
    let raw = match std::fs::read(&meta_path) {
        Ok(b) => b,
        Err(e) => {
            cleanup(&new_event_ids);
            panic!("read {}: {}", meta_path.display(), e);
        }
    };
    if !raw.starts_with(b"age-encryption.org/v1\n") {
        cleanup(&new_event_ids);
        panic!(
            "meta.json must be age-encrypted; first 32 bytes = {:?}",
            &raw[..raw.len().min(32)]
        );
    }
    if serde_json::from_slice::<serde_json::Value>(&raw).is_ok() {
        cleanup(&new_event_ids);
        panic!("meta.json must NOT be parseable as JSON post-encryption");
    }

    // BONUS — GET round-trip only when the full pipeline succeeded.
    // (write_analysis runs AFTER provider.analyze; on rate-limit there's
    // no analysis.json to decrypt.)
    if capture_succeeded {
        let analysis_url = format!("http://127.0.0.1:{}/api/events/{}/analysis", port, event_id);
        let resp = client
            .get(&analysis_url)
            .send()
            .await
            .expect("GET analysis");
        if !resp.status().is_success() {
            let status = resp.status();
            cleanup(&new_event_ids);
            panic!("GET {} returned {}", analysis_url, status);
        }
        let analysis: serde_json::Value = resp.json().await.expect("GET analysis JSON");
        if analysis.get("summary").and_then(|v| v.as_str()).is_none() {
            cleanup(&new_event_ids);
            panic!("analysis response missing `summary`: {}", analysis);
        }
        eprintln!(
            "✓ event {} encrypted on disk + GET decrypted analysis OK",
            event_id
        );
    } else if was_rate_limited {
        eprintln!(
            "⚠ event {} encrypted on disk OK; GET roundtrip skipped \
             (Gemini rate-limited the analyze step). Core E004 invariant verified.",
            event_id
        );
    } else {
        cleanup(&new_event_ids);
        panic!(
            "capture failed but not from rate-limit.\n--- stderr ---\n{}",
            stderr_text
        );
    }

    // Restore operator's events dir to pre-test state.
    cleanup(&new_event_ids);
}
