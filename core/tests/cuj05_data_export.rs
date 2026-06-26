// CUJ-05 · data-export subset — integration test for `phantom data export`.
//
// Covers MAC-CUJ05-EXP-001 / MAC-CUJ05-EXP-002: the real demo command
// `phantom data export` must emit the local Life Node store to stdout in two
// portable formats:
//
//   EXP-001 (`--format json`): stdout parses as a JSON array (serde_json).
//       An empty store is allowed to emit `[]` — that is still a valid array.
//   EXP-002 (`--format md`):  stdout carries the "# Life Node export" header
//       so the markdown dump is self-describing even when empty.
//
// HARNESS NOTES:
//   - Reuses the `phantom_bin()` locate-or-skip pattern from
//     `cuj05_backup_export.rs`: `cargo test --test ...` does NOT build the bin
//     target, so a fresh tree with no prior `cargo build` would otherwise fail
//     spuriously. We skip (eprintln + return) instead.
//   - Each case runs against an isolated $HOME temp dir with an empty
//     ~/.phantom-mesh/, so the test is hermetic and order-independent — it
//     never touches the developer's real Life Node store and needs no
//     EventKey (export reads whatever events exist; here that is none).

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Build the `phantom` CLI bin path. Mirrors `cuj05_backup_export.rs`:
/// honour `PHANTOM_TEST_BIN`, then try target-triple paths before the generic
/// `target/{release,debug}/phantom`, and skip the test if none exist.
fn phantom_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PHANTOM_TEST_BIN") {
        return Some(PathBuf::from(p));
    }
    let candidates = [
        "target/aarch64-apple-darwin/release/phantom",
        "target/aarch64-apple-darwin/debug/phantom",
        "target/release/phantom",
        "target/debug/phantom",
    ];
    for rel in candidates {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Spawn `phantom data export <args...>` with an isolated, empty $HOME and
/// return (exit_success, stdout, stderr). Returns None if the bin is unbuilt.
fn run_data_export(args: &[&str]) -> Option<(bool, String, String)> {
    let bin = phantom_bin()?;
    let home_dir = TempDir::new().expect("tempdir for HOME");
    // Plant an empty ~/.phantom-mesh/ — the "no events" path EXP-001/002 guard.
    fs::create_dir_all(home_dir.path().join(".phantom-mesh")).expect("mkdir .phantom-mesh");

    let mut cmd = Command::new(&bin);
    cmd.env("HOME", home_dir.path()).args(["data", "export"]);
    cmd.args(args);
    let output = cmd.output().expect("spawn phantom data export");
    Some((
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

#[test]
fn cuj05_data_export_json_emits_parseable_array() {
    let (ok, stdout, stderr) = match run_data_export(&["--format", "json"]) {
        Some(r) => r,
        None => {
            eprintln!(
                "SKIPPED: cuj05_data_export_json_emits_parseable_array — no built \
                 phantom bin found (run `cargo build --release --bin phantom`)"
            );
            return;
        }
    };

    assert!(
        ok,
        "`phantom data export --format json` should exit 0; stderr: {stderr}"
    );

    // MAC-CUJ05-EXP-001: stdout must be a JSON ARRAY. An empty store → `[]`,
    // which is acceptable. Parse into a Vec<Value> so a JSON object or a bare
    // scalar would fail the test (the export contract is an array of events).
    let parsed: Vec<serde_json::Value> = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("stdout did not parse as a JSON array: {e}\n--- stdout ---\n{stdout}")
    });
    // No further shape assertion: an empty array is a valid (no-events) export.
    let _ = parsed;
}

#[test]
fn cuj05_data_export_md_has_life_node_header() {
    let (ok, stdout, stderr) = match run_data_export(&["--format", "md"]) {
        Some(r) => r,
        None => {
            eprintln!(
                "SKIPPED: cuj05_data_export_md_has_life_node_header — no built phantom \
                 bin found (run `cargo build --release --bin phantom`)"
            );
            return;
        }
    };

    assert!(
        ok,
        "`phantom data export --format md` should exit 0; stderr: {stderr}"
    );

    // MAC-CUJ05-EXP-002: the markdown dump leads with the Life Node header so
    // even an empty export is self-describing.
    assert!(
        stdout.contains("# Life Node export"),
        "markdown export should contain the Life Node export header; got:\n{stdout}"
    );
}
