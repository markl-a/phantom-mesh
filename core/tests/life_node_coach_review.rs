//! Integration smoke for E003 `phantom coach review`.
//!
//! Seeds a fake `~/.phantom-mesh/events/` with one event's meta.json +
//! analysis.json, runs the CLI with HOME redirected to a tempdir, and
//! asserts the resulting Markdown contains a reference to the analysis
//! summary. Covers E003 acceptance criterion #1 (`exit 0 + non-empty MD`)
//! and #5 (`Markdown contains a reference to that event's analysis`).
//!
//! Skips if the `phantom` binary isn't built — `cargo test --test ...`
//! does NOT build the BIN target, so dev boxes need `cargo build --bin
//! phantom` first; CI builds in advance.

use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn phantom_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PHANTOM_TEST_BIN") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    // env!() exposes whatever target-dir cargo is using for this run.
    let p = PathBuf::from(env!("CARGO_BIN_EXE_phantom"));
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

#[test]
fn coach_review_outputs_markdown_referencing_seeded_event() {
    let Some(bin) = phantom_binary() else {
        eprintln!("SKIP: phantom binary not found — build with `cargo build --bin phantom`");
        return;
    };

    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path();
    let events_dir = home.join(".phantom-mesh/events");
    let event_dir = events_dir.join("evt-test-001");
    fs::create_dir_all(&event_dir).unwrap();

    // Seed meta.json — timestamp 2026-05-22 so the `--date` arg matches.
    let meta = json!({
        "event_id":       "evt-test-001",
        "kind":           "food_log",
        "timestamp":      "2026-05-22T12:30:00Z",
        "source_node":    "test-mac",
        "goal_tags":      ["fat_loss"],
        "modality_files": [],
        "user_text":      "Caesar salad with grilled chicken"
    });
    fs::write(
        event_dir.join("meta.json"),
        serde_json::to_vec_pretty(&meta).unwrap(),
    )
    .unwrap();

    // Seed analysis.json — the summary string is what the review must
    // surface back in its Markdown body.
    let analysis = json!({
        "summary":      "Caesar salad with grilled chicken — within fat-loss targets, ~520 kcal estimate.",
        "goal_impact":  null,
        "suggestion":   null,
        "confidence":   0.85,
        "raw_response": {},
        "model_id":     "test-model",
        "latency_ms":   150,
        "cost_usd":     null
    });
    fs::write(
        event_dir.join("analysis.json"),
        serde_json::to_vec_pretty(&analysis).unwrap(),
    )
    .unwrap();

    // Run `phantom coach review --date 2026-05-22` with HOME redirected.
    let out = Command::new(&bin)
        .env("HOME", home)
        .env("USERPROFILE", home) // Windows
        .args(["coach", "review", "--date", "2026-05-22"])
        .output()
        .expect("spawn phantom");
    assert!(
        out.status.success(),
        "exit non-zero: status={:?} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );

    let stdout = String::from_utf8(out.stdout).expect("utf-8 stdout");
    assert!(
        stdout.starts_with("# Daily review — 2026-05-22"),
        "missing heading; got: {}",
        stdout
    );
    assert!(
        stdout.contains("**Events captured:** 1"),
        "missing event count line; got: {}",
        stdout
    );
    assert!(
        stdout.contains("## fat_loss"),
        "missing goal-tag section; got: {}",
        stdout
    );
    assert!(
        stdout.contains("Caesar salad"),
        "missing reference to seeded analysis summary; got: {}",
        stdout
    );
}

#[test]
fn coach_review_save_flag_writes_reviews_dir() {
    let Some(bin) = phantom_binary() else {
        eprintln!("SKIP: phantom binary not found");
        return;
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path();

    // Empty events dir is fine — the brief still writes (with "no events" stub).
    fs::create_dir_all(home.join(".phantom-mesh/events")).unwrap();

    let out = Command::new(&bin)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .args(["coach", "review", "--date", "2026-05-22", "--save"])
        .output()
        .expect("spawn phantom");
    assert!(
        out.status.success(),
        "exit non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let saved = home.join(".phantom-mesh/reviews/2026-05-22.md");
    assert!(saved.exists(), "--save did not write {:?}", saved);
    // No identity.key → plaintext path
    let body = fs::read_to_string(&saved).unwrap();
    assert!(body.contains("# Daily review — 2026-05-22"));
}

/// E003 acceptance #6: when an identity.key exists, `--save` writes
/// age-encrypted bytes (covers the E004 substrate hook).
#[test]
fn coach_review_save_encrypts_when_identity_key_present() {
    let Some(bin) = phantom_binary() else {
        eprintln!("SKIP: phantom binary not found");
        return;
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path();
    fs::create_dir_all(home.join(".phantom-mesh/events")).unwrap();
    // Seed an identity.key (32+ bytes of test data is enough for HKDF).
    let identity_bytes = vec![0xA5u8; 64];
    fs::write(home.join(".phantom-mesh/identity.key"), &identity_bytes).unwrap();

    let out = Command::new(&bin)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .args(["coach", "review", "--date", "2026-05-22", "--save"])
        .output()
        .expect("spawn phantom");
    assert!(
        out.status.success(),
        "exit non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let saved = home.join(".phantom-mesh/reviews/2026-05-22.md");
    assert!(saved.exists(), "--save did not write {:?}", saved);
    let bytes = fs::read(&saved).unwrap();

    // age v1 binary magic is "age-encryption.org/v1\n" but the actual on-disk
    // form starts with `age-encryption.org/v1` ASCII. The crypto module
    // exposes `looks_like_age` for this exact check.
    assert!(
        phantom_mesh::life_node::crypto::looks_like_age(&bytes),
        "saved file is not age-encrypted (first 32 bytes: {:?})",
        &bytes[..bytes.len().min(32)],
    );
    // Stderr should say "age-encrypted"
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("age-encrypted"),
        "stderr should mention encryption; got: {}",
        stderr
    );
}
