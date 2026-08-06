//! CUJ-02 · MAC-CUJ02-FOOD-001 / MAC-CUJ02-FOC-001 — hermetic food / focus
//! capture integration tests.
//!
//! Until now both cases only had (a) `#[ignore]`/env-dependent unit tests in
//! `capture_food_wire` / `capture_focus_wire` and (b) the GEMINI_API_KEY-gated
//! e2e in `core/tests/life_node_capture_e2e.rs`, which SKIPs on CI. This file
//! adds always-on coverage with zero real network and zero developer-state
//! pollution, following the cuj04 harness pattern (temp HOME + seeded
//! agents.toml + deterministic EventKey + `SPECTYN_MESH_<SLUG>_BASE_URL` →
//! wiremock).
//!
//! ## What is covered (the deepest layer that IS hermetic)
//!
//!   • FOOD-001: `capture_food_wire::record_food` — the full library pipeline
//!     `analyze_food` (real HTTP to a wiremock Gemini, image inlined as
//!     `inlineData`) → `write_food_event` (age-encrypted SPEC-16 EventStore)
//!     → read BACK via the public `read_event` / `query_events` + a direct
//!     `decrypt_raw_age_blob` round-trip, plus the P4 plaintext-PII boundary.
//!   • FOC-001: the `spectyn focus` CLI engine
//!     `life_node::focus_session::{start, interrupt, stop}` with an injected
//!     temp base dir — `stop` persists a `kind=focus` Life Node event whose
//!     `meta.json` / `analysis.json` are age-encrypted at rest (identity.key
//!     present) and read back decrypted through `EventStore`.
//!
//! ## What REMAINS un-hermetic (documented gap, not covered here)
//!
//!   • The `spectyn food` / `spectyn event capture` CLI surface POSTs
//!     multipart to a live `spectyn serve` daemon (`life_node::capture::run` →
//!     `/api/events`); serve-side capture is embargoed (parallel lineage) and
//!     needs a built bin, so the daemon hop stays covered by the key-gated
//!     `life_node_capture_e2e.rs` only. The CLI→daemon hop itself has a
//!     wiremock unit test in `life_node::capture::tests`.
//!   • FOC-001's "audio blob 加密" clause: the CLI focus path records no audio
//!     (text summary event only) — ASR/audio capture has no hermetic seam yet.
//!
//! HOME redirection only works on unix (`dirs::home_dir()` ignores `$HOME` on
//! Windows and would pollute the real `~/.spectyn-mesh`), hence the file gate.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use spectyn_mesh::capture_food_wire::{record_food, FoodCaptureRequest, FOOD_LOG_KIND};
use spectyn_mesh::event_storage_wire::{query_events, read_event, EventStoreQuery};
use spectyn_mesh::rpc_wire::EventKind;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Serialise tests that mutate process-global state (HOME, provider env vars,
/// the per-process EventKey cache) — cargo runs `#[test]`s on threads within
/// one binary. The focus test takes it too: it shares this process with the
/// food test and ordering its fs work behind the env mutation keeps the file
/// trivially race-free.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

fn unique_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "spectyn-cuj02cap-{}-{}-{}",
        tag,
        std::process::id(),
        nanos()
    ))
}

/// Recursively scan every file under `root` for `needle`. Returns the paths
/// of files that contain it — used for the P4 "no plaintext PII at rest"
/// boundary assertion.
fn files_containing(root: &Path, needle: &[u8]) -> Vec<PathBuf> {
    let mut hits = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(bytes) = std::fs::read(&p) {
                if bytes.windows(needle.len()).any(|w| w == needle) {
                    hits.push(p);
                }
            }
        }
    }
    hits
}

/// Gemini `generateContent` 200 body whose single text part is the model's
/// strict-JSON `FoodAnalysisResult` reply (camelCase per the wire shape).
fn gemini_food_ok() -> serde_json::Value {
    let analysis_json = serde_json::json!({
        "summary": "Salmon bento, about 650 kcal — balanced lunch.",
        "macroEstimate": {
            "calories": 650,
            "proteinG": 38,
            "carbsG": 70,
            "fatG": 18,
            "fiberG": 5
        },
        "fatLossScore": 0.74,
        "suggestion": "add a side of greens at dinner",
        "confidence": 0.9
    })
    .to_string();
    serde_json::json!({
        "candidates": [{ "content": { "parts": [{ "text": analysis_json }] } }],
        "usageMetadata": { "promptTokenCount": 12, "candidatesTokenCount": 9 }
    })
}

/// MAC-CUJ02-FOOD-001 — happy path: `record_food` analyses the meal via the
/// (mocked) Gemini provider and persists an age-encrypted Food event that
/// reads back through the public EventStore API.
#[test]
fn cuj02_food_001_record_food_persists_encrypted_event_readable_back() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    // One long-lived MULTI-THREAD runtime hosts the MockServer AND drives the
    // sync `record_food` via `spawn_blocking`, so `providers_wire::
    // block_on_async` takes the `block_in_place` branch on a live runtime
    // (same runtime model as cuj04_coach_review_llm.rs — a throwaway
    // current_thread runtime would be torn down mid-request).
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("multi-thread tokio runtime");

    let server = rt.block_on(async {
        let s = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(gemini_food_ok()))
            .mount(&s)
            .await;
        s
    });

    // ── Isolated HOME + seeded agents.toml + deterministic EventKey ────────
    let home = unique_dir("food");
    let pm = home.join(".spectyn-mesh");
    std::fs::create_dir_all(&pm).expect("create .spectyn-mesh");
    std::fs::write(
        pm.join("agents.toml"),
        r#"
[routing]
fallback_chain = ["gemini"]

[providers.gemini]
default_model = "gemini-2.5-flash"
"#,
    )
    .expect("write agents.toml");
    std::env::set_var("HOME", &home);
    std::env::set_var("SPECTYN_MESH_GEMINI_API_KEY", "test-key");
    std::env::set_var("SPECTYN_MESH_GEMINI_BASE_URL", server.uri());
    spectyn_mesh::encryption_wire::install_event_key_from_seed(&[7u8; 32])
        .expect("install deterministic test EventKey");

    // Source image: bytes are read + base64-inlined, never decoded, so a
    // 4-byte JPEG marker pair is enough for the ≤10 MB read-side guard.
    let img = home.join("lunch.jpg");
    std::fs::write(&img, [0xFFu8, 0xD8, 0xFF, 0xD9]).expect("write test jpg");

    // The note is PII (SPEC-20 §13 / P4): it must only ever exist inside the
    // age-encrypted body. ASCII marker so the at-rest scan is byte-exact.
    let note = "PII-NOTE-CUJ02-FOOD-7d3f salmon bento";
    let req = FoodCaptureRequest {
        text: Some(note.to_string()),
        image_path: Some(img.to_string_lossy().into_owned()),
        kind: FOOD_LOG_KIND.to_string(),
        tag: vec!["fat_loss".to_string()],
        timestamp_ms: 1716557400000, // 2024-05-24T13:30:00Z
    };

    let event_id = rt
        .block_on(async move {
            tokio::task::spawn_blocking(move || record_food(&req))
                .await
                .expect("record_food spawn_blocking join")
        })
        .expect("FOOD-001 happy path: record_food returns Ok(event_id)");
    assert!(!event_id.is_empty(), "event_id assigned");

    // ── The LLM was actually consulted, with the image inlined ─────────────
    let requests = rt
        .block_on(async { server.received_requests().await })
        .expect("mock server records requests");
    assert_eq!(requests.len(), 1, "exactly one provider call for one capture");
    let body = String::from_utf8_lossy(&requests[0].body);
    assert!(
        body.contains("inlineData"),
        "the Gemini request must carry the image as an inlineData part (SPEC-20 T-FOOD-02)"
    );
    // FAKE-GREEN GUARD (SPEC-20 NO-FAKING): assert the ACTUAL image pixels reached
    // the wire, not just the key name. The 4 source bytes [FF D8 FF D9] base64
    // (STANDARD) encode to exactly "/9j/2Q==" — an empty/stubbed data_b64, or the
    // old "send the filename" behaviour, would still emit "inlineData" but NOT
    // these bytes. This is what proves image-in is pixels, not a path.
    assert!(
        body.contains("/9j/2Q=="),
        "the request must inline the real image pixels (base64 of the JPEG bytes), not a filename/empty: {body}"
    );
    assert!(
        body.contains("image/jpeg"),
        "the inlined image part must carry the inferred image/jpeg MIME type: {body}"
    );

    // ── Read back through the PUBLIC read paths ─────────────────────────────
    let rec = read_event(&event_id).expect("read_event decrypts + returns the record");
    assert!(matches!(rec.meta.kind, EventKind::Food), "kind=Food");
    assert!(
        rec.meta.tags.iter().any(|t| t == "food") && rec.meta.tags.iter().any(|t| t == "fat_loss"),
        "plaintext meta carries the food + request tags, got {:?}",
        rec.meta.tags
    );

    let rows = query_events(&EventStoreQuery {
        date_iso: None,
        kind: Some(EventKind::Food),
        tag: Some("food".to_string()),
        limit: None,
        offset: None,
    })
    .expect("query_events kind=Food tag=food");
    assert!(
        rows.iter().any(|r| r.meta.event_id == event_id),
        "the captured meal must be queryable back by kind+tag"
    );

    // ── At-rest shape: body is age v1 ciphertext that decrypts to the meal ─
    let body_path = pm.join("events").join(&event_id).join("body.age");
    let raw = std::fs::read(&body_path).expect("body.age exists");
    assert!(
        raw.starts_with(b"age-encryption.org/v1"),
        "body.age must be an age v1 blob (SPEC-13)"
    );
    let plain = spectyn_mesh::encryption_wire::decrypt_raw_age_blob(&raw)
        .expect("body decrypts under the installed EventKey");
    let v: serde_json::Value = serde_json::from_slice(&plain).expect("decrypted body is JSON");
    assert_eq!(v["note"], note, "user note round-trips through encryption");
    assert_eq!(v["summary"], "Salmon bento, about 650 kcal — balanced lunch.");
    assert_eq!(v["macro_estimate"]["calories"], 650, "macros preserved");

    // ── P4 boundary: the PII note exists NOWHERE in plaintext at rest ──────
    let leaks = files_containing(&pm, note.as_bytes());
    assert!(
        leaks.is_empty(),
        "PII note must never touch disk in plaintext, leaked in {:?}",
        leaks
    );

    std::env::remove_var("SPECTYN_MESH_GEMINI_BASE_URL");
    std::env::remove_var("SPECTYN_MESH_GEMINI_API_KEY");
    let _ = std::fs::remove_dir_all(&home);
}

/// MAC-CUJ02-FOC-001 — happy path: the `spectyn focus` CLI engine persists a
/// `kind=focus` event on `stop`, encrypted at rest, readable back decrypted.
#[test]
fn cuj02_foc_001_focus_stop_persists_encrypted_event_readable_back() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    use spectyn_mesh::life_node::focus_session;
    use spectyn_mesh::life_node::storage::EventStore;

    // Injected base dir (the CLI passes the user's home) — fully hermetic.
    let base = unique_dir("focus");
    let pm = base.join(".spectyn-mesh");
    std::fs::create_dir_all(&pm).expect("create .spectyn-mesh");
    // identity.key present → both the live session file and the completed
    // event must be age-encrypted at rest (SPEC-13 / P4).
    std::fs::write(pm.join("identity.key"), [0x42u8; 64]).expect("seed identity.key");

    let task = "FOC-001-PII-MARKER deep work on merger plan";
    focus_session::start(&base, 1, Some(task.to_string()), vec![]).expect("focus start");
    assert!(focus_session::status(&base).is_some(), "session persisted");
    focus_session::interrupt(&base, "slack ping").expect("record interruption");

    let result = focus_session::stop(&base).expect("focus stop");
    assert_eq!(result.interruptions, 1);
    let event_id = result.event_id.expect("stop persists a Life Node focus event");
    assert!(
        focus_session::status(&base).is_none(),
        "session file removed after stop"
    );

    // ── At-rest shape: meta + analysis are age v1 ciphertext ────────────────
    let event_dir = pm.join("events").join(&event_id);
    for file in ["meta.json", "analysis.json"] {
        let raw = std::fs::read(event_dir.join(file)).expect(file);
        assert!(
            raw.starts_with(b"age-encryption.org/v1"),
            "{} must be age-encrypted when identity.key is present",
            file
        );
    }

    // ── Read back DECRYPTED through the EventStore ──────────────────────────
    let store = EventStore::with_identity_file(pm.join("events"), &pm.join("identity.key"));
    let meta = store.read_meta(&event_id).expect("meta reads back decrypted");
    assert!(matches!(meta.kind, EventKind::Focus), "kind=Focus");
    assert_eq!(meta.event_id, event_id);

    let analysis = store
        .read_analysis(&event_id)
        .expect("analysis reads back decrypted");
    assert!(
        analysis.summary.contains("1 interruption"),
        "summary carries the session metrics, got {:?}",
        analysis.summary
    );
    assert!(
        analysis.summary.contains(task),
        "summary names the task, got {:?}",
        analysis.summary
    );

    // ── P4 boundary: the task text exists NOWHERE in plaintext at rest ─────
    let leaks = files_containing(&pm, task.as_bytes());
    assert!(
        leaks.is_empty(),
        "focus task (PII) must never touch disk in plaintext, leaked in {:?}",
        leaks
    );

    let _ = std::fs::remove_dir_all(&base);
}
