//! End-to-end tests for `spectyn permissions` (the Execution Permission CLI).
//! Spawns the real binary against a temp HOME so the `set` write path + the
//! `doctor` round-trip are both exercised, not just the lib logic.

use std::path::Path;
use std::process::Command;

fn spectyn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_spectyn")
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

/// `permissions list` enumerates all four profiles, exit 0.
#[test]
fn permissions_list_shows_all_profiles() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(spectyn_bin())
        .args(["permissions", "list"])
        .env("HOME", home.path())
        .output()
        .expect("spawn");
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    for slug in ["observe", "suggest", "workspace-write", "developer-full"] {
        assert!(s.contains(slug), "list must mention {slug}:\n{s}");
    }
}

/// `permissions set <slug>` writes `[permissions] profile` into the home
/// agents.toml (format-preserving — the unrelated provider block survives),
/// and a subsequent `doctor` reports the profile.
#[test]
fn permissions_set_writes_profile_and_doctor_reports_it() {
    let home = tempfile::tempdir().unwrap();
    let cfg = home.path().join(".spectyn-mesh/agents.toml");
    write(&home.path().join(".spectyn-mesh/identity.key"), "x");
    // a pre-existing config with a comment + provider block we must not clobber
    write(
        &cfg,
        "# my config\n[providers.groq]\ntype = \"groq\"\napi_key_env = \"GROQ_API_KEY\"\n",
    );

    let set = Command::new(spectyn_bin())
        .args(["permissions", "set", "workspace-write"])
        .env("HOME", home.path())
        .output()
        .expect("spawn set");
    assert!(set.status.success(), "set failed: {}", String::from_utf8_lossy(&set.stderr));

    let after = std::fs::read_to_string(&cfg).unwrap();
    assert!(after.contains("profile = \"workspace-write\""), "profile not written:\n{after}");
    assert!(after.contains("# my config"), "comment clobbered:\n{after}");
    assert!(after.contains("[providers.groq]"), "provider block clobbered:\n{after}");

    // doctor now reports the profile in its JSON state.
    let doc = Command::new(spectyn_bin())
        .args(["doctor", "--json"])
        .current_dir(home.path()) // avoid picking up a stray cwd config
        .env("HOME", home.path())
        .env("GROQ_API_KEY", "sk-test")
        .output()
        .expect("spawn doctor");
    let json = String::from_utf8_lossy(&doc.stdout);
    assert!(
        json.contains("workspace-write"),
        "doctor --json should report the profile:\n{json}"
    );
}

/// `permissions set <garbage>` is rejected with exit 2 and writes nothing.
#[test]
fn permissions_set_rejects_unknown_profile() {
    let home = tempfile::tempdir().unwrap();
    let cfg = home.path().join(".spectyn-mesh/agents.toml");
    write(&cfg, "[providers.groq]\ntype = \"groq\"\n");
    let out = Command::new(spectyn_bin())
        .args(["permissions", "set", "nonsense"])
        .env("HOME", home.path())
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "unknown profile must exit 2");
    let after = std::fs::read_to_string(&cfg).unwrap();
    assert!(!after.contains("profile"), "must not write on rejection:\n{after}");
}
