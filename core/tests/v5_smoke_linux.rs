//! V5 cross-OS smoke — Linux entry (SPEC-60 §V5; task-2026052731).
//!
//! SPEC-60's V5 ship gate is the 5-platform "does it basically run?" smoke
//! matrix (Mac · Windows · Linux · iOS · Android, one Rust codebase —
//! BIG-GOAL line 50). This file is the **Linux** slice: a minimal, fast,
//! NO-NETWORK / NO-LLM / NO-daemon set of checks that the built `phantom`
//! binary runs and honors its basic CLI contract. It complements
//! `cli_linux.rs` (which exercises deeper Linux-specific behavior like
//! `serve` binding a port); this file is the universal smoke that the V5
//! gate runs identically on every OS.
//!
//! Uses `env!("CARGO_BIN_EXE_phantom")` so `cargo test` builds + locates the
//! real binary. Gated `#[cfg(target_os = "linux")]` so other OSes compile it
//! to an empty crate.

#![cfg(target_os = "linux")]

use std::process::{Command, Stdio};

fn phantom() -> Command {
    Command::new(env!("CARGO_BIN_EXE_phantom"))
}

/// V5/LIN — `--version` runs (exit 0) and prints provenance: the product name,
/// a semver, and the linux target triple. (Verifies the build embedded its
/// git/arch/date stamps — a binary missing those is a broken release artifact.)
#[test]
fn v5_version_has_linux_provenance() {
    let out = phantom().arg("--version").output().expect("spawn --version");
    assert!(out.status.success(), "--version exit: {:?}", out.status);
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(s.contains("phantom"), "missing product name: {s}");
    assert!(s.contains("linux"), "missing linux target triple: {s}");
    // provenance form: `phantom <ver> (<hash>, linux-…, built …)`
    assert!(s.contains('('), "missing provenance parens: {s}");
}

/// V5/LIN — `--version --short` prints a bare semver (`X.Y.Z[-pre]`), no
/// decoration. This is the form release tooling parses.
#[test]
fn v5_version_short_is_bare_semver() {
    let out = phantom()
        .args(["--version", "--short"])
        .output()
        .expect("spawn --version --short");
    assert!(out.status.success());
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let core = v.split('-').next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    assert_eq!(parts.len(), 3, "expected X.Y.Z, got {v:?}");
    assert!(
        parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())),
        "non-numeric semver core: {v:?}"
    );
    assert!(!v.contains('('), "short form should have no provenance: {v:?}");
}

/// V5/LIN — `phantom doctor` runs to completion (exit 0; it's a best-effort
/// diagnostic that never hard-fails) and emits a non-empty report. No daemon,
/// network, or provider key required.
#[test]
fn v5_doctor_runs_offline() {
    let out = phantom().arg("doctor").output().expect("spawn doctor");
    assert!(out.status.success(), "doctor exit: {:?}", out.status);
    let n = out.stdout.len() + out.stderr.len();
    assert!(n > 0, "doctor produced no output");
}

/// V5/LIN — `phantom selftest --list` runs (exit 0) without executing the
/// suite — confirms the self-test harness is wired and discoverable.
#[test]
fn v5_selftest_list_runs() {
    let out = phantom()
        .args(["selftest", "--list"])
        .output()
        .expect("spawn selftest --list");
    assert!(out.status.success(), "selftest --list exit: {:?}", out.status);
}

/// V5/LIN — headless contract: `phantom exec` with no prompt arg and a
/// non-TTY, empty stdin exits 2 (usage error), rather than hanging or
/// dispatching an empty prompt. (Pairs with the exec stdin-pipe path used by
/// CI/automation.)
#[test]
fn v5_exec_empty_stdin_is_usage_error() {
    let out = phantom()
        .arg("exec")
        .stdin(Stdio::null())
        .output()
        .expect("spawn exec");
    assert_eq!(
        out.status.code(),
        Some(2),
        "exec with empty stdin should exit 2 (usage); got {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}
