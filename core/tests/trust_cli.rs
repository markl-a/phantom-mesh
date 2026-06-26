//! End-to-end tests for `phantom trust` (the Project Trust CLI) + the doctor
//! Project layer. Spawns the real binary against a temp HOME so the trust.json
//! write path and the doctor round-trip are exercised, not just lib logic.

use std::path::Path;
use std::process::Command;

fn phantom_bin() -> &'static str {
    env!("CARGO_BIN_EXE_phantom")
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

/// `trust add` in a project dir writes trust.json; `trust show` then reports
/// it trusted; an untrusted sibling is not.
#[test]
fn trust_add_then_show_reports_trusted() {
    let home = tempfile::tempdir().unwrap();
    let proj = tempfile::tempdir().unwrap();

    let add = Command::new(phantom_bin())
        .args(["trust", "add"])
        .current_dir(proj.path())
        .env("HOME", home.path())
        .output()
        .expect("spawn add");
    assert!(add.status.success(), "add failed: {}", String::from_utf8_lossy(&add.stderr));
    assert!(home.path().join(".phantom-mesh/trust.json").is_file(), "trust.json not written");

    let show = Command::new(phantom_bin())
        .args(["trust", "show"])
        .current_dir(proj.path())
        .env("HOME", home.path())
        .output()
        .expect("spawn show");
    assert!(String::from_utf8_lossy(&show.stdout).contains("trusted"));

    // a sibling dir is still untrusted
    let other = tempfile::tempdir().unwrap();
    let show2 = Command::new(phantom_bin())
        .args(["trust", "show"])
        .current_dir(other.path())
        .env("HOME", home.path())
        .output()
        .expect("spawn show2");
    assert!(String::from_utf8_lossy(&show2.stdout).contains("untrusted"));
}

/// With enforcement off (default), `doctor --json` reports the Project layer as
/// an informational OK even in an untrusted dir (nothing is restricted).
#[test]
fn doctor_project_layer_ok_when_enforcement_off() {
    let home = tempfile::tempdir().unwrap();
    write(&home.path().join(".phantom-mesh/identity.key"), "x");
    write(
        &home.path().join(".phantom-mesh/agents.toml"),
        "[providers.groq]\ntype=\"groq\"\napi_key_env=\"GROQ_API_KEY\"\n",
    );
    let out = Command::new(phantom_bin())
        .args(["doctor", "--json"])
        .current_dir(home.path())
        .env("HOME", home.path())
        .env("GROQ_API_KEY", "sk-test")
        .output()
        .expect("spawn doctor");
    assert_eq!(out.status.code(), Some(0));
    let json = String::from_utf8_lossy(&out.stdout);
    assert!(json.contains("\"project\""), "doctor --json must include a project finding:\n{json}");
    // off → not a PROJECT_UNTRUSTED warning
    assert!(!json.contains("PROJECT_UNTRUSTED"), "off must not warn:\n{json}");
}

/// `trust add <nonexistent>` is rejected (exit 2), writes nothing.
#[test]
fn trust_add_rejects_nonexistent_dir() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(phantom_bin())
        .args(["trust", "add", "/no/such/dir/xyzzy"])
        .env("HOME", home.path())
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2));
    assert!(!home.path().join(".phantom-mesh/trust.json").is_file(), "must not write on rejection");
}
