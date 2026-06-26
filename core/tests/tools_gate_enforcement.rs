//! Proves the centralized tool gate ACTUALLY ENFORCES on a non-interactive
//! surface — not just that the helpers compile. `phantom tool <name> --args`
//! calls `tools::execute` directly (no LLM needed), which is the single gate
//! chokepoint, so these exercise the real end-to-end path that `exec` / `serve`
//! / subagents also flow through.
//!
//! Covers the review's must-fix findings:
//!   - bypass: a profile/trust set in HOME is enforced on headless tool calls
//!   - escalation: a cwd/agents.toml CANNOT weaken the HOME security policy
//!   - fail-closed + escape hatch (PHANTOM_TRUST_ALL)

use std::path::Path;
use std::process::Command;

fn phantom_bin() -> &'static str {
    env!("CARGO_BIN_EXE_phantom")
}
fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

/// Run `phantom tool <tool> --args <json>` with HOME + cwd set; return stdout+stderr.
fn run_tool(home: &Path, cwd: &Path, extra_env: &[(&str, &str)], tool: &str, args_json: &str) -> String {
    let mut cmd = Command::new(phantom_bin());
    cmd.args(["tool", tool, "--args", args_json])
        .current_dir(cwd)
        .env("HOME", home)
        .env_remove("PHANTOM_TRUST_ALL");
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("phantom tool must spawn");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn home_with(profile_or_trust: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    write(&home.path().join(".phantom-mesh/identity.key"), "x");
    write(
        &home.path().join(".phantom-mesh/agents.toml"),
        &format!("[providers.groq]\ntype=\"groq\"\napi_key_env=\"GROQ_API_KEY\"\n\n{profile_or_trust}"),
    );
    home
}

#[test]
fn observe_profile_denies_headless_file_write() {
    let home = home_with("[permissions]\nprofile = \"observe\"\n");
    let cwd = tempfile::tempdir().unwrap();
    let target = cwd.path().join("x.txt");
    let out = run_tool(
        home.path(),
        cwd.path(),
        &[],
        "file_write",
        &format!("{{\"path\":\"{}\",\"content\":\"hi\"}}", target.display()),
    );
    assert!(out.contains("[denied]"), "observe must deny headless file_write:\n{out}");
    assert!(!target.exists(), "denied write must not create the file");
}

#[test]
fn observe_profile_allows_headless_file_read() {
    let home = home_with("[permissions]\nprofile = \"observe\"\n");
    let cwd = tempfile::tempdir().unwrap();
    let cfg = home.path().join(".phantom-mesh/agents.toml");
    let out = run_tool(
        home.path(),
        cwd.path(),
        &[],
        "file_read",
        &format!("{{\"path\":\"{}\"}}", cfg.display()),
    );
    assert!(!out.contains("[denied]"), "observe must allow file_read:\n{out}");
}

#[test]
fn trust_all_escape_hatch_bypasses_the_gate() {
    let home = home_with("[permissions]\nprofile = \"observe\"\n");
    let cwd = tempfile::tempdir().unwrap();
    let target = cwd.path().join("y.txt");
    let out = run_tool(
        home.path(),
        cwd.path(),
        &[("PHANTOM_TRUST_ALL", "1")],
        "file_write",
        &format!("{{\"path\":\"{}\",\"content\":\"hi\"}}", target.display()),
    );
    assert!(!out.contains("[denied]"), "PHANTOM_TRUST_ALL must bypass:\n{out}");
    assert!(target.exists(), "escape hatch must let the write through");
}

#[test]
fn cwd_config_cannot_weaken_home_security_policy() {
    // HOME says observe (read-only); a malicious cwd/agents.toml says
    // developer-full. The HOME policy must win → still denied.
    let home = home_with("[permissions]\nprofile = \"observe\"\n");
    let cwd = tempfile::tempdir().unwrap();
    write(&cwd.path().join("agents.toml"), "[permissions]\nprofile = \"developer-full\"\n");
    let target = cwd.path().join("z.txt");
    let out = run_tool(
        home.path(),
        cwd.path(),
        &[],
        "file_write",
        &format!("{{\"path\":\"{}\",\"content\":\"hi\"}}", target.display()),
    );
    assert!(out.contains("[denied]"), "cwd config must NOT weaken home policy:\n{out}");
    assert!(!target.exists());
}

#[test]
fn suggest_profile_fail_closed_denies_write_on_headless_surface() {
    // suggest → file_write is an Ask. On a non-interactive surface (phantom tool)
    // there's no one to prompt, so it must FAIL-CLOSED (Deny), never auto-allow.
    let home = home_with("[permissions]\nprofile = \"suggest\"\n");
    let cwd = tempfile::tempdir().unwrap();
    let target = cwd.path().join("s.txt");
    let out = run_tool(
        home.path(),
        cwd.path(),
        &[],
        "file_write",
        &format!("{{\"path\":\"{}\",\"content\":\"hi\"}}", target.display()),
    );
    assert!(out.contains("[denied]"), "suggest Ask must fail-closed headless:\n{out}");
    assert!(!target.exists());
    // but suggest allows reads
    let cfg = home.path().join(".phantom-mesh/agents.toml");
    let read = run_tool(home.path(), cwd.path(), &[], "file_read", &format!("{{\"path\":\"{}\"}}", cfg.display()));
    assert!(!read.contains("[denied]"), "suggest must allow reads:\n{read}");
}

#[test]
fn malformed_home_config_fails_closed_not_open() {
    // A typo in the HOME security config must DENY (fail-closed), not silently
    // drop to allow-all.
    let home = tempfile::tempdir().unwrap();
    write(&home.path().join(".phantom-mesh/identity.key"), "x");
    write(&home.path().join(".phantom-mesh/agents.toml"), "this is { not valid toml");
    let cwd = tempfile::tempdir().unwrap();
    let cfg = home.path().join(".phantom-mesh/agents.toml");
    let out = run_tool(
        home.path(),
        cwd.path(),
        &[],
        "file_read",
        &format!("{{\"path\":\"{}\"}}", cfg.display()),
    );
    assert!(out.contains("[denied]"), "malformed home config must fail closed:\n{out}");
}

#[test]
fn phantom_perm_deny_works_even_with_no_profile_configured() {
    // The gate is now installed even for a default (no-profile, trust-off) config,
    // so PHANTOM_PERM=deny / plan-mode work on every surface — previously the gate
    // was skipped for default configs, making those controls silently inert
    // (the TUI fail-open).
    let home = tempfile::tempdir().unwrap();
    write(&home.path().join(".phantom-mesh/identity.key"), "x");
    write(
        &home.path().join(".phantom-mesh/agents.toml"),
        "[providers.groq]\ntype=\"groq\"\napi_key_env=\"GROQ_API_KEY\"\n", // NO [permissions], NO [trust]
    );
    let cwd = tempfile::tempdir().unwrap();
    let cfg = home.path().join(".phantom-mesh/agents.toml");
    let out = run_tool(
        home.path(),
        cwd.path(),
        &[("PHANTOM_PERM", "deny")],
        "file_read",
        &format!("{{\"path\":\"{}\"}}", cfg.display()),
    );
    assert!(out.contains("[denied]"), "PHANTOM_PERM=deny must work without a profile:\n{out}");
}

#[test]
fn trust_all_does_not_override_phantom_perm_deny() {
    // PHANTOM_PERM=deny (most restrictive) wins even with PHANTOM_TRUST_ALL=1.
    let home = home_with("[permissions]\nprofile = \"developer-full\"\n");
    let cwd = tempfile::tempdir().unwrap();
    let cfg = home.path().join(".phantom-mesh/agents.toml");
    let out = run_tool(
        home.path(),
        cwd.path(),
        &[("PHANTOM_TRUST_ALL", "1"), ("PHANTOM_PERM", "deny")],
        "file_read",
        &format!("{{\"path\":\"{}\"}}", cfg.display()),
    );
    assert!(out.contains("[denied]"), "PHANTOM_PERM=deny must beat TRUST_ALL:\n{out}");
}

#[test]
fn phantom_perm_deny_is_a_hard_deny_even_for_reads() {
    // PHANTOM_PERM=deny is a global hard override — even a read tool is denied,
    // independent of any profile.
    let home = home_with("[permissions]\nprofile = \"developer-full\"\n");
    let cwd = tempfile::tempdir().unwrap();
    let cfg = home.path().join(".phantom-mesh/agents.toml");
    let out = run_tool(
        home.path(),
        cwd.path(),
        &[("PHANTOM_PERM", "deny")],
        "file_read",
        &format!("{{\"path\":\"{}\"}}", cfg.display()),
    );
    assert!(out.contains("[denied]"), "PHANTOM_PERM=deny must hard-deny even reads:\n{out}");
}

#[test]
fn trust_observe_denies_in_untrusted_dir_allows_when_trusted() {
    // No permission profile; trust enforcement=observe does the gating.
    let home = home_with("[trust]\nenforcement = \"observe\"\n");
    let cwd = tempfile::tempdir().unwrap();
    let target = cwd.path().join("t.txt");
    let argjson = format!("{{\"path\":\"{}\",\"content\":\"hi\"}}", target.display());

    // untrusted → denied
    let out = run_tool(home.path(), cwd.path(), &[], "file_write", &argjson);
    assert!(out.contains("[denied]"), "untrusted dir under observe-trust must deny write:\n{out}");

    // trust the dir, then it's allowed
    let trusted = Command::new(phantom_bin())
        .args(["trust", "add"])
        .current_dir(cwd.path())
        .env("HOME", home.path())
        .status()
        .expect("trust add");
    assert!(trusted.success());
    let out2 = run_tool(home.path(), cwd.path(), &[], "file_write", &argjson);
    assert!(!out2.contains("[denied]"), "trusted dir must allow write:\n{out2}");
    assert!(target.exists());
}
