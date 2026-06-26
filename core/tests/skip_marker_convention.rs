//! Meta-test: prerequisite-gated integration tests must announce skips
//! with one canonical, greppable marker.
//!
//! Rust's libtest has no native "skipped" outcome — tests that early-return
//! when a key/binary/server is absent still count as PASS. That is only
//! honest if the log says so, in one uniform shape:
//!
//!     eprintln!("SKIPPED: <test fn> — <missing prerequisite>");
//!
//! (Run with `cargo test -- --nocapture` to see the markers on passing
//! runs; libtest captures eprintln! output of passing tests by default.)
//!
//! This test scans `core/tests/*.rs` and fails on legacy ad-hoc markers
//! (string literals starting with the bare upper/lower skip word and a
//! colon) and on canonical lines that drop the `<test fn> — <reason>`
//! tail, so new skip gates can't silently drift back to unreadable or
//! ungreppable forms.

use std::path::PathBuf;

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

#[test]
fn skip_markers_use_canonical_skipped_prefix() {
    // Needles assembled at runtime so this file's own source never matches.
    let legacy_upper = format!("\"{}:", "SKIP");
    let legacy_lower = format!("\"{}:", "skip");
    let canonical = format!("\"{}: ", "SKIPPED");

    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(tests_dir()).expect("read core/tests") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for (i, line) in src.lines().enumerate() {
            if line.contains(&legacy_upper) || line.contains(&legacy_lower) {
                offenders.push(format!(
                    "{name}:{}: legacy skip marker — use `SKIPPED: <test> — <reason>`: {}",
                    i + 1,
                    line.trim()
                ));
            }
            // Canonical lines must carry the `<test fn> — <reason>` tail on
            // the same literal fragment so a bare `SKIPPED:` can't sneak in.
            if line.contains(&canonical) && !line.contains(" — ") {
                offenders.push(format!(
                    "{name}:{}: SKIPPED marker missing `<test> — <reason>` tail: {}",
                    i + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "non-canonical skip markers in core/tests (want `eprintln!(\"{}: <test> — <reason>\")`):\n{}",
        "SKIPPED",
        offenders.join("\n")
    );
}
