//! Phase 3: command-tree namespaces (auth / provider / project) + deprecation
//! aliases. The new grouped forms route to the existing flat handlers; the old
//! flat names still work but print a one-line hint.

use std::path::Path;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_spectyn")
}
fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}
fn home() -> tempfile::TempDir {
    let h = tempfile::tempdir().unwrap();
    write(&h.path().join(".spectyn-mesh/identity.key"), "x");
    write(
        &h.path().join(".spectyn-mesh/agents.toml"),
        "[providers.groq]\ntype=\"groq\"\napi_key_env=\"GROQ_API_KEY\"\n",
    );
    h
}

#[test]
fn namespace_with_no_verb_shows_usage_exit_2() {
    let h = home();
    let out = Command::new(bin()).arg("auth").env("HOME", h.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage: spectyn auth"));
}

#[test]
fn new_project_trust_form_works_without_deprecation_hint() {
    let h = home();
    let proj = tempfile::tempdir().unwrap();
    let out = Command::new(bin())
        .args(["project", "trust", "add"])
        .current_dir(proj.path())
        .env("HOME", h.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(h.path().join(".spectyn-mesh/trust.json").is_file(), "new form must write trust.json");
    // new form is canonical → no deprecation hint
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("old name still works"),
        "new form must not print a deprecation hint"
    );
}

#[test]
fn old_trust_form_still_works_but_warns() {
    let h = home();
    let proj = tempfile::tempdir().unwrap();
    let out = Command::new(bin())
        .args(["trust", "add"])
        .current_dir(proj.path())
        .env("HOME", h.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(h.path().join(".spectyn-mesh/trust.json").is_file());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("spectyn project trust"), "old form must hint the new name:\n{err}");
}

#[test]
fn auth_whoami_routes_to_whoami() {
    let h = home();
    let out = Command::new(bin()).args(["auth", "whoami"]).env("HOME", h.path()).output().unwrap();
    assert!(out.status.success());
    // no deprecation hint on the new form
    assert!(!String::from_utf8_lossy(&out.stderr).contains("old name still works"));
}

#[test]
fn provider_list_routes_to_providers() {
    let h = home();
    let out = Command::new(bin()).args(["provider", "list"]).env("HOME", h.path()).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("groq"), "provider list must show the provider");
}
