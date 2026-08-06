//! Phase E §V7 — accessibility (a11y) skeleton via axe-core + puppeteer.
//!
//! task-2026052647. SPEC-60 V7 ship-gate calls for an automated WCAG/a11y scan
//! of the desktop SPA (single-page app). The real scan needs a JS runtime
//! (node), a headless browser (puppeteer + chromium), and a built SPA to load
//! — none of which are guaranteed on every CI runner or dev host. So this file
//! is a **skip-gated skeleton**, mirroring `v4_e2e_desktop_linux.rs`: it stays
//! green by skipping when the toolchain/build is absent, and becomes a real
//! gate once the a11y harness lands.
//!
//! **Why a Rust test driving JS**: keeps the a11y gate in the same `cargo test`
//! ship-gate surface as the rest of V-series. The Rust test is a thin launcher
//! that shells out to a node harness (`app/scripts/a11y-axe.mjs`, added when
//! this becomes real) and asserts on its JSON verdict.
//!
//! **Enablement plan** (when V7 a11y goes real — see each test's TODO):
//!   1. Add `app/scripts/a11y-axe.mjs`: launch puppeteer, load the built SPA
//!      (`file://.../app/dist/index.html` or a `vite preview` URL), inject
//!      `axe-core`, run `axe.run()`, print `{ violations: [...] }` JSON.
//!   2. `cd app && pnpm install && pnpm build` to produce `app/dist`.
//!   3. Flip the skip-gates below to assert `critical + serious == 0`.
//!
//! Run: `cargo test --test v7_a11y_axe_core` (skips cleanly without the harness).
//! Override the app dir with `SPECTYN_APP_DIR`.

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Resolve the SPA app directory (`<workspace>/app`). Override with
/// `SPECTYN_APP_DIR`. Returns `None` when it can't be located.
fn app_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SPECTYN_APP_DIR") {
        let path = PathBuf::from(p);
        return path.is_dir().then_some(path);
    }
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|root| root.join("app"))?;
    candidate.is_dir().then_some(candidate)
}

/// True when a JS runtime is on PATH (the a11y harness needs node).
fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn v7_a11y_toolchain_probe_is_sound() {
    // Always-on guard: the probe helpers themselves must not panic, regardless
    // of host capability. This keeps the skeleton's plumbing honest so that
    // when the harness lands, only the gate logic (not the probes) is new.
    let _ = node_available();
    match app_dir() {
        Some(d) => println!("v7-a11y: app dir = {}", d.display()),
        None => eprintln!("v7-a11y: app dir not found (set SPECTYN_APP_DIR)"),
    }
}

#[test]
fn v7_a11y_axe_scan_skeleton_pending_harness() {
    // SPEC-60 V7: the built SPA must have zero critical/serious axe-core
    // violations. Skip-gated until the harness + build exist.
    //
    // NO-FAKING: this test is renamed away from "..._scan_no_critical_violations"
    // because, until enablement step 3 lands the real axe assert, a green here
    // does NOT verify a11y — it only verifies the SKELETON precondition (the
    // harness is not yet wired). The legitimate skip-gates below stay skips, but
    // the "all prerequisites present yet no assert" fall-through now FAILS loudly
    // (it can only be reached once someone adds the harness + build, at which
    // point they MUST also wire the assert) instead of silently passing.
    let Some(app) = app_dir() else {
        eprintln!(
            "SKIPPED: v7_a11y_axe_scan_no_critical_violations — app dir not found \
             (set SPECTYN_APP_DIR)"
        );
        return;
    };
    if !node_available() {
        eprintln!(
            "SKIPPED: v7_a11y_axe_scan_no_critical_violations — node not on PATH \
             (a11y scan needs a JS runtime)"
        );
        return;
    }
    let harness = app.join("scripts").join("a11y-axe.mjs");
    if !harness.exists() {
        eprintln!(
            "SKIPPED: v7_a11y_axe_scan_no_critical_violations — a11y harness not \
             present yet ({}); V7 a11y is skeleton-only, see file header enablement \
             plan.",
            harness.display()
        );
        return;
    }
    let build = app.join("dist").join("index.html");
    if !build.exists() {
        eprintln!(
            "SKIPPED: v7_a11y_axe_scan_no_critical_violations — SPA build not found \
             ({}); run `cd app && pnpm build` first.",
            build.display()
        );
        return;
    }

    // TODO (enablement step 3): run the harness and assert no critical/serious
    // violations. Shape, once a11y-axe.mjs prints `{ "violations": [ { "impact":
    // "critical"|"serious"|... } ] }`:
    //
    //   let out = Command::new("node").arg(&harness).arg(&build)
    //       .output().expect("run a11y-axe");
    //   let report: serde_json::Value =
    //       serde_json::from_slice(&out.stdout).expect("a11y json");
    //   let bad = report["violations"].as_array().unwrap().iter()
    //       .filter(|v| matches!(v["impact"].as_str(), Some("critical" | "serious")))
    //       .count();
    //   assert_eq!(bad, 0, "axe-core found {bad} critical/serious a11y violations");
    // All prerequisites are present (harness + build exist + node on PATH) — so
    // there is no honest reason to skip. Reaching here means the a11y gate is
    // wired-up except for the assert itself, which would be a silent fake-green.
    // Fail loudly to force enablement step 3 to land the real axe assertion.
    panic!(
        "v7 a11y prerequisites are all present (harness={} build={}) but the \
         axe-core assert is still a TODO — wire enablement step 3 (run the harness \
         and assert zero critical/serious violations) instead of passing vacuously.",
        harness.display(),
        build.display()
    );
}
