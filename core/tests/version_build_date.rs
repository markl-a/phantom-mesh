//! Cross-platform regression guard for `phantom --version`'s build-date stamp.
//!
//! The build date is injected by `core/build.rs` into the compile-time env var
//! `PHANTOM_BUILD_DATE`, which the `--version` printer in `src/bin/phantom.rs`
//! renders as `... built <DATE>`. The original build.rs shelled out to the unix
//! `date -u` binary, which does not exist on Windows, so the stamp degraded to
//! `?` / `unknown` on Windows builds. This spawns the REAL binary and asserts
//! the stamp is a well-formed `YYYY-MM-DD` and never a sentinel.
//!
//! Cross-platform: it spawns `CARGO_BIN_EXE_phantom` and inspects stdout only —
//! no shelling out, no platform-specific assumptions.

use std::process::Command;

fn phantom_bin() -> &'static str {
    env!("CARGO_BIN_EXE_phantom")
}

/// Tiny dependency-free `\d{4}-\d{2}-\d{2}` matcher: scan for any window of
/// `DIGIT DIGIT DIGIT DIGIT - DIGIT DIGIT - DIGIT DIGIT`.
fn contains_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    let d = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    let dash = |i: usize| b.get(i) == Some(&b'-');
    (0..b.len()).any(|i| {
        d(i) && d(i + 1) && d(i + 2) && d(i + 3)
            && dash(i + 4)
            && d(i + 5) && d(i + 6)
            && dash(i + 7)
            && d(i + 8) && d(i + 9)
    })
}

#[test]
fn version_build_date_is_iso_and_never_sentinel() {
    let out = Command::new(phantom_bin())
        .arg("--version")
        .output()
        .expect("failed to spawn phantom --version");

    assert!(
        out.status.success(),
        "phantom --version exited non-zero: {:?}",
        out.status
    );

    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        contains_iso_date(&stdout),
        "`phantom --version` stdout has no YYYY-MM-DD build date: {stdout:?}"
    );

    // The build date must never degrade to a sentinel — the whole point of the
    // fix is that the stamp is always a real date on every platform.
    assert!(
        !stdout.contains("built ?"),
        "build date degraded to '?': {stdout:?}"
    );
    assert!(
        !stdout.contains("unknown"),
        "build date degraded to 'unknown': {stdout:?}"
    );
}

/// Self-check: the matcher accepts real ISO dates and rejects sentinels, so a
/// future refactor can't accidentally make the assertion vacuously pass.
#[test]
fn iso_date_matcher_sanity() {
    assert!(contains_iso_date("phantom 0.6.0 (abc, windows-x86_64, built 2026-06-24)"));
    assert!(contains_iso_date("2026-01-01"));
    assert!(!contains_iso_date("phantom 0.6.0 (abc, windows-x86_64, built ?)"));
    assert!(!contains_iso_date("phantom 0.6.0 (abc, windows-x86_64, built unknown)"));
    assert!(!contains_iso_date("2026-6-24")); // not zero-padded → not matched
}
