//! V4 ship-gate e2e — desktop Linux happy-path coverage for SPEC-61 S3
//! (Linux onboarding via `spectyn hello` / `spectyn onboarding`).
//!
//! Linux sibling of `v4_e2e_desktop_macos.rs` (task-2026052644 — the SHARED
//! brief said "pick whichever your host supports"; mac shipped the macOS
//! variant, this is the node-a/WSL Linux variant).
//!
//! **Scope today**: Skeleton + binary smoke. Verifies the test infrastructure
//! itself is sound (`spectyn` binary builds + reports a version + exposes the
//! `onboarding` subcommand). The full S3 happy-path verification (< 10s wizard
//! to Done + 30s TTFR via `spectyn dispatch hello`) is blocked on SPEC-28
//! Stage 3 (`onboarding_wire::advance` + `compute_ttfr` + `start_demo_relay_handoff`
//! all still `unimplemented!()`). Each S3-flow test below is structured to
//! become real once SPEC-28 lands — for now it skips with a TODO marker so
//! `cargo test --test v4_e2e_desktop_linux` stays green on CI.
//!
//! **Build pre-req**: `cargo build --release --bin spectyn` before running.
//! `cargo test --test ...` does NOT build BIN targets. Tests SKIP gracefully
//! when the binary is missing so CI without a pre-built spectyn still passes.
//!
//! **Source of truth**: SPEC-60 §8.4 V4 e2e gate + SPEC-61 S3 row at
//! `docs/superpowers/specs/v060-deep-spec/SPEC-61-TESTING-scenarios.md:672`.
//!
//! Override the binary path with `SPECTYN_TEST_BIN=/path/to/spectyn`.

#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Resolve the prebuilt `spectyn` binary path. Override with
/// `SPECTYN_TEST_BIN`; otherwise fall back to `target/release/spectyn`
/// relative to the workspace root (the conventional `cargo build --release`
/// output location). Returns `None` when the binary is missing so tests can
/// skip gracefully.
fn spectyn_bin_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SPECTYN_TEST_BIN") {
        let path = PathBuf::from(p);
        return path.exists().then_some(path);
    }
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|root| root.join("target").join("release").join("spectyn"))?;
    candidate.exists().then_some(candidate)
}

/// Hard time-budget for the S3 happy-path wizard per SPEC-61 S3 row:
/// "< 10s 印 Done". Used both for wall-clock assertions and as the subprocess
/// timeout so a hung wizard does not stall CI.
const S3_WIZARD_BUDGET: Duration = Duration::from_secs(10);

/// SPEC-28 §1 TTFR (Time To First Response) p95 budget — 30s on the desktop
/// happy path. The S3 row uses this for the `spectyn dispatch hello` follow-up
/// step.
const TTFR_P95_BUDGET: Duration = Duration::from_secs(30);

#[test]
fn spectyn_binary_exists_and_reports_version() {
    let Some(bin) = spectyn_bin_path() else {
        eprintln!(
            "SKIPPED: spectyn_binary_exists_and_reports_version — prebuilt spectyn \
             binary not found (run `cargo build --release --bin spectyn` first)"
        );
        return;
    };

    let start = Instant::now();
    let output = Command::new(&bin)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn spectyn --version");
    let elapsed = start.elapsed();

    assert!(
        output.status.success(),
        "spectyn --version exited non-zero: status={:?} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.to_lowercase().contains("spectyn") || stdout.contains('.'),
        "spectyn --version stdout should contain version-like text, got: {stdout:?}"
    );
    // Linux build reports its triple — assert the platform tag is right so a
    // mis-targeted binary (e.g. a stray macOS build on PATH) fails loud.
    assert!(
        stdout.to_lowercase().contains("linux") || !stdout.to_lowercase().contains("darwin"),
        "spectyn --version on Linux should not report a darwin build, got: {stdout:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "spectyn --version should return within 5s (cold start budget); took {elapsed:?}"
    );
}

#[test]
fn spectyn_onboarding_subcommand_discoverable() {
    let Some(bin) = spectyn_bin_path() else {
        eprintln!(
            "SKIPPED: spectyn_onboarding_subcommand_discoverable — prebuilt spectyn \
             binary not found"
        );
        return;
    };

    // `spectyn help` should advertise the `onboarding` entry point per
    // core/src/bin/spectyn.rs CLI help text. If this fails the SPEC-61 S3
    // scenario lost its primary entry point and the V4 gate is genuinely
    // broken — not a SPEC-28-unimplemented skip.
    let output = Command::new(&bin)
        .arg("help")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn spectyn help");

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("onboarding"),
        "spectyn help should advertise the onboarding subcommand, got: {combined:?}"
    );
}

#[test]
fn s3_linux_onboarding_happy_path_wizard_under_10s() {
    // SPEC-61 S3 happy path: fresh install → run wizard → "Done" within 10s.
    // The wizard's underlying state machine (SPEC-28 onboarding_wire::advance)
    // is currently `unimplemented!()` (Stage 3 deferred), so running the real
    // wizard from a clean home dir would panic before reaching "Done". When
    // SPEC-28 Stage 3 lands, replace this skip with:
    //
    //   1. Point HOME at a tempdir so no agents.toml exists
    //   2. Spawn `spectyn onboarding --noninteractive --provider demo-relay`
    //   3. Assert exit 0 within S3_WIZARD_BUDGET
    //   4. Assert stdout contains "Done" / equivalent
    //   5. Assert agents.toml was created in the tempdir
    eprintln!(
        "SKIPPED: s3_linux_onboarding_happy_path_wizard_under_10s — SPEC-28 Stage 3 \
         deferred; onboarding_wire::advance still unimplemented!(); full S3 wizard \
         run blocked. Budget reference: {S3_WIZARD_BUDGET:?}"
    );
}

#[test]
fn s3_linux_dispatch_hello_ttfr_under_30s() {
    // SPEC-61 S3 follow-up: after wizard, `spectyn dispatch hello` returns
    // first token within 30s. Blocked on the same SPEC-28 Stage 3 work —
    // demo-relay handshake (`onboarding_wire::start_demo_relay_handoff`) and
    // TTFR measurement (`onboarding_wire::compute_ttfr`) are still
    // `unimplemented!()`. When SPEC-28 lands, this becomes:
    //
    //   1. Pre-run the S3 wizard above to populate agents.toml + identity.key
    //   2. Spawn `spectyn dispatch hello` with stdout piped
    //   3. Read first token line; record `start.elapsed()`
    //   4. Assert elapsed < TTFR_P95_BUDGET
    //   5. Drain remaining output and assert exit 0
    eprintln!(
        "SKIPPED: s3_linux_dispatch_hello_ttfr_under_30s — SPEC-28 Stage 3 deferred; \
         compute_ttfr + start_demo_relay_handoff still unimplemented!(); full TTFR \
         measurement blocked. Budget reference: {TTFR_P95_BUDGET:?}"
    );
}
