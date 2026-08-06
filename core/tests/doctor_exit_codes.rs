//! Regression guard for `spectyn doctor`'s PROCESS EXIT CODE.
//!
//! An earlier review found the wiring broken: `run_doctor` printed its report
//! but returned `Ok(())` regardless of severity, so a genuinely broken machine
//! exited 0. The lib unit tests cover `diagnose()`/`exit_code()` in isolation,
//! but only an end-to-end spawn of the REAL binary proves `run_doctor` actually
//! calls `process::exit` — i.e. catches a refactor that drops it.
//!
//! Cross-platform: the exit code comes from the (OS-agnostic) state machine,
//! and `HOME` controls the home `diagnose()` reads (via `resolve_home_dir`,
//! which prefers `$HOME` on every platform), so these run everywhere.
//!
//! Conventions asserted (see the doc block over the `doctor` dispatch in
//! `src/bin/spectyn.rs`):
//!   • `spectyn doctor`        → 2 on a genuine Fail, 0 on healthy-or-warnings
//!   • `spectyn doctor --json` → ALWAYS 0 (health is in the JSON `state.worst`)

use std::path::Path;
use std::process::Command;

fn spectyn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_spectyn")
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

/// A keyed `groq` provider + a permission rule — i.e. a fully usable config.
const USABLE_CONFIG: &str = "[providers.groq]\n\
     type = \"groq\"\n\
     api_key_env = \"GROQ_API_KEY\"\n\
     default_model = \"llama-3.3-70b-versatile\"\n\n\
     [permissions]\nallow = [\"file_read\"]\n";

/// No identity + no config = genuinely broken → exit 2.
#[test]
fn doctor_exits_2_on_broken_machine() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let status = Command::new(spectyn_bin())
        .arg("doctor")
        .current_dir(cwd.path())
        .env("HOME", home.path())
        .env_remove("GROQ_API_KEY")
        .status()
        .expect("spectyn doctor must spawn");
    assert_eq!(
        status.code(),
        Some(2),
        "a machine with no identity + no config must exit 2, not {:?}",
        status.code()
    );
}

/// Identity + a provider whose `api_key_env` is actually set → exit 0.
#[test]
fn doctor_exits_0_when_configured_and_key_present() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    write(&home.path().join(".spectyn-mesh/identity.key"), "x");
    write(&home.path().join(".spectyn-mesh/agents.toml"), USABLE_CONFIG);
    let status = Command::new(spectyn_bin())
        .arg("doctor")
        .current_dir(cwd.path())
        .env("HOME", home.path())
        .env("GROQ_API_KEY", "sk-test")
        .status()
        .expect("spectyn doctor must spawn");
    assert_eq!(
        status.code(),
        Some(0),
        "configured + key present must exit 0, not {:?}",
        status.code()
    );
}

/// An advisory warning (provider key NAMED but not exported) is not a hard
/// failure — `scripts/validate-mac.sh`'s contract relies on this → exit 0.
#[test]
fn doctor_exits_0_on_warnings_only() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    write(&home.path().join(".spectyn-mesh/identity.key"), "x");
    write(&home.path().join(".spectyn-mesh/agents.toml"), USABLE_CONFIG);
    let status = Command::new(spectyn_bin())
        .arg("doctor")
        .current_dir(cwd.path())
        .env("HOME", home.path())
        .env_remove("GROQ_API_KEY") // named but unset → PROVIDER_KEY_MISSING warn
        .status()
        .expect("spectyn doctor must spawn");
    assert_eq!(
        status.code(),
        Some(0),
        "warnings are advisory → exit 0, not {:?}",
        status.code()
    );
}

/// `doctor --json` always exits 0 — even on a broken machine — because the
/// health lives in the JSON `state.worst` field. verify-binary.sh / selftest
/// gate on "valid JSON ⇒ exit 0", so this MUST stay 0.
#[test]
fn doctor_json_always_exits_0_even_when_broken() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let output = Command::new(spectyn_bin())
        .args(["doctor", "--json"])
        .current_dir(cwd.path())
        .env("HOME", home.path())
        .output()
        .expect("spectyn doctor --json must spawn");
    assert_eq!(
        output.status.code(),
        Some(0),
        "--json must exit 0 regardless of health, not {:?}",
        output.status.code()
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim_start().starts_with('{'),
        "--json must emit a JSON object:\n{stdout}"
    );
}
