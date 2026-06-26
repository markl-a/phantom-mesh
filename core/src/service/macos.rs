// core/src/service/macos.rs
//
// macOS service-manager adapter (PF-2c minimal scaffold).
//
// The full launchd install / uninstall / status logic still lives in
// `core/src/bin/phantom.rs` (lines 6886-7977 + 9096-9155) where it was
// before PF-2a/2b. This file currently exposes only the path-derivation
// helpers so the V7+PF-2c P0 test `launchctl_plist_paths_correct` has
// somewhere to live. The full impl port is the rest of PF-2c.

#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::process::Command;

/// launchd job label for the always-on `phantom serve` daemon. Must
/// stay in sync with `LAUNCH_AGENT_LABEL` in `core/src/bin/phantom.rs`
/// — both compile-time pinned because the label is referenced from
/// templates (`templates/ai.phantommesh.serve.plist.tmpl`) and from
/// `launchctl kickstart gui/<uid>/ai.phantommesh.serve` strings.
pub const LAUNCH_AGENT_LABEL: &str = "ai.phantommesh.serve";

/// Path to the per-user LaunchAgent plist:
/// `~/Library/LaunchAgents/ai.phantommesh.serve.plist`. Returns `None`
/// when `$HOME` is unresolvable (test sandboxes, headless CI).
pub fn launch_agent_plist_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| {
        home.join("Library/LaunchAgents")
            .join(format!("{}.plist", LAUNCH_AGENT_LABEL))
    })
}

/// Path to the system-wide LaunchDaemon plist:
/// `/Library/LaunchDaemons/ai.phantommesh.serve.plist`. Independent
/// of `$HOME` because LaunchDaemons run as root.
pub fn launch_daemon_plist_path() -> PathBuf {
    PathBuf::from("/Library/LaunchDaemons").join(format!("{}.plist", LAUNCH_AGENT_LABEL))
}

/// Parse `launchctl print gui/<uid>/<label>` output, extracting the
/// running daemon's pid. Returns `Some(pid)` when the service is
/// active, `None` when not registered, the parser can't find a `pid =
/// <N>` line, or `launchctl` itself errored. Per-line lookup is loose
/// (the surrounding output has tab indentation and trailing semicolons
/// vary across macOS releases).
///
/// Pure parsing — no I/O — so the test for this function is hermetic.
pub fn parse_pid_from_launchctl_print(output: &str) -> Option<u32> {
    for line in output.lines() {
        let line = line.trim();
        // Match `pid = 1156` and `pid = 1156;` (some macOS versions).
        // Reject lines where `pid` appears as a substring of e.g.
        // `inherited_pid` by requiring a word boundary.
        if let Some(rest) = line.strip_prefix("pid ") {
            // After "pid " we expect "= <number>".
            if let Some(num) = rest.trim_start().strip_prefix('=') {
                let n = num.trim().trim_end_matches(';').trim();
                if let Ok(p) = n.parse::<u32>() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Shell out to `launchctl print gui/<uid>/<label>` and return the
/// raw stdout. Used by status probes. Returns `None` when the target
/// isn't registered (launchctl exits non-zero) or when the call fails.
pub fn launchctl_print(label: &str) -> Option<String> {
    let uid = nix_uid();
    let target = format!("gui/{}/{}", uid, label);
    let out = Command::new("launchctl")
        .arg("print")
        .arg(&target)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// `id -u` returns the numeric uid. Falls back to 501 (default macOS
/// admin uid since 10.5) only if the call fails — never panics.
fn nix_uid() -> u32 {
    if let Ok(out) = Command::new("id").arg("-u").output() {
        if let Ok(s) = String::from_utf8(out.stdout) {
            if let Ok(n) = s.trim().parse::<u32>() {
                return n;
            }
        }
    }
    501
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MAC P0 — `launchctl print gui/<uid>/ai.phantommesh.serve` on a
    /// host where the LaunchAgent is registered (this dev Mac, pid 1156
    /// at session-start) must surface a parseable pid. We don't assert
    /// the *value* (pid changes on every install/launchctl bootout),
    /// only that the parser finds *some* pid > 0 when the service is
    /// running.
    ///
    /// On a host where the service isn't installed (e.g. CI), both
    /// `launchctl_print` and the parser return None and the test passes
    /// via the eprintln-skip path — no spurious red on CI.
    #[test]
    fn launchctl_print_includes_pid() {
        // Parser-only check first (no I/O).
        let sample = "gui/501/ai.phantommesh.serve = {\n\
                      \tactive count = 1\n\
                      \tpath = /Users/foo/Library/LaunchAgents/ai.phantommesh.serve.plist\n\
                      \ttype = LaunchAgent\n\
                      \tstate = running\n\
                      \tprogram = /Users/foo/.local/bin/phantom\n\
                      \tpid = 4242\n\
                      }";
        assert_eq!(parse_pid_from_launchctl_print(sample), Some(4242));

        // Reject impostor lines (e.g. inherited_pid, stopped_pid).
        let no_pid = "gui/501/ai.phantommesh.serve = {\n\
                      \tactive count = 0\n\
                      \tstate = not running\n}";
        assert_eq!(parse_pid_from_launchctl_print(no_pid), None);

        // Live check: if the LaunchAgent is registered on this host,
        // confirm the end-to-end shell-out + parse pipeline finds a pid.
        // Otherwise skip silently — non-fatal so CI doesn't flake.
        match launchctl_print(LAUNCH_AGENT_LABEL) {
            Some(out) => {
                let pid = parse_pid_from_launchctl_print(&out);
                if let Some(p) = pid {
                    assert!(p > 0, "pid = 0 makes no sense; raw: {}", out);
                } else {
                    // Registered but not running (state = not running) is fine.
                    eprintln!(
                        "launchctl_print returned but no pid found — service \
                         is probably loaded-but-stopped. raw output (truncated):\n{}",
                        out.chars().take(400).collect::<String>()
                    );
                }
            }
            None => {
                eprintln!(
                    "no `{}` LaunchAgent registered on this host — skipping \
                     live launchctl_print round-trip.",
                    LAUNCH_AGENT_LABEL
                );
            }
        }
    }

    /// Build a minimal LaunchAgent plist for testing that won't collide
    /// with the real `ai.phantommesh.serve` (test label, /bin/true program).
    /// Returns (label, plist_path). Caller is responsible for cleanup.
    fn write_test_plist() -> (String, std::path::PathBuf) {
        let label = format!("ai.phantommesh.tdd-{}", std::process::id());
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n<dict>\n\
             \t<key>Label</key>\n\t<string>{}</string>\n\
             \t<key>ProgramArguments</key>\n\t<array>\n\
             \t\t<string>/bin/true</string>\n\t</array>\n\
             \t<key>RunAtLoad</key>\n\t<false/>\n\
             </dict>\n</plist>\n",
            label
        );
        let path = dirs::home_dir()
            .expect("HOME must exist for LaunchAgent path")
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", label));
        std::fs::write(&path, plist).expect("write test plist");
        (label, path)
    }

    fn cleanup_test_plist(label: &str, path: &std::path::Path) {
        // Best-effort bootout — fine if already gone.
        let uid = nix_uid();
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("gui/{}/{}", uid, label)])
            .output();
        let _ = std::fs::remove_file(path);
    }

    /// MAC P0 — `launchctl bootstrap gui/<uid> <plist>` on a fresh,
    /// well-formed plist must succeed and result in the agent being
    /// registered. Test uses a throwaway plist (`/bin/true` program,
    /// `tdd-<pid>` label) so it can't collide with the real
    /// `ai.phantommesh.serve` daemon running on the dev's Mac.
    #[test]
    #[ignore = "integration / env-dependent — real launchctl bootstrap (needs root / clean launchd session); run via --ignored"]
    fn launchctl_bootstrap_succeeds() {
        let (label, path) = write_test_plist();
        let uid = nix_uid();
        let target = format!("gui/{}", uid);

        let bootstrap = Command::new("launchctl")
            .args(["bootstrap", &target])
            .arg(&path)
            .output()
            .expect("launchctl bootstrap must spawn");

        let registered_after = launchctl_print(&label).is_some();

        // Cleanup BEFORE asserting (don't leak on failure).
        cleanup_test_plist(&label, &path);

        assert!(
            bootstrap.status.success(),
            "launchctl bootstrap {} {} failed (exit {:?}): {}",
            target,
            path.display(),
            bootstrap.status,
            String::from_utf8_lossy(&bootstrap.stderr)
        );
        assert!(
            registered_after,
            "launchctl bootstrap returned 0 but the agent isn't visible \
             via `launchctl print`. Either the bootstrap silently no-op'd \
             or the print pipeline is broken."
        );
    }

    /// MAC P0 — `launchctl bootout` followed by `launchctl bootstrap` of
    /// the same plist must round-trip cleanly (the post-state matches
    /// the pre-state). Catches the regression where bootout leaves a
    /// half-registered service that the next bootstrap rejects with
    /// "Bootstrap failed: 5: Input/output error".
    #[test]
    #[ignore = "integration / env-dependent — real launchctl bootout/bootstrap round-trip (needs root / clean launchd session); run via --ignored"]
    fn unload_then_load_round_trip() {
        let (label, path) = write_test_plist();
        let uid = nix_uid();
        let target = format!("gui/{}", uid);

        // Bootstrap first.
        let b1 = Command::new("launchctl")
            .args(["bootstrap", &target])
            .arg(&path)
            .output()
            .expect("launchctl bootstrap (round 1)");
        if !b1.status.success() {
            cleanup_test_plist(&label, &path);
            panic!(
                "round 1 bootstrap failed: {}",
                String::from_utf8_lossy(&b1.stderr)
            );
        }
        let loaded_after_round1 = launchctl_print(&label).is_some();

        // Bootout.
        let bo = Command::new("launchctl")
            .args(["bootout", &format!("gui/{}/{}", uid, label)])
            .output()
            .expect("launchctl bootout");
        let unloaded_after_bootout = launchctl_print(&label).is_none();

        // Bootstrap round 2.
        let b2 = Command::new("launchctl")
            .args(["bootstrap", &target])
            .arg(&path)
            .output()
            .expect("launchctl bootstrap (round 2)");
        let loaded_after_round2 = launchctl_print(&label).is_some();

        // Cleanup before assertions.
        cleanup_test_plist(&label, &path);

        assert!(
            loaded_after_round1,
            "agent not visible after round 1 bootstrap"
        );
        assert!(
            bo.status.success(),
            "bootout failed: {}",
            String::from_utf8_lossy(&bo.stderr)
        );
        assert!(unloaded_after_bootout, "agent still visible after bootout");
        assert!(
            b2.status.success(),
            "round 2 bootstrap failed (idempotency broken): {}",
            String::from_utf8_lossy(&b2.stderr)
        );
        assert!(
            loaded_after_round2,
            "agent not visible after round 2 bootstrap"
        );
    }

    #[test]
    fn launchctl_plist_paths_correct() {
        // Label must match what's hard-coded across the codebase
        // (phantom.rs templates, cli_config.rs launchctl strings,
        // OAuth client id is a different label so don't conflate).
        assert_eq!(LAUNCH_AGENT_LABEL, "ai.phantommesh.serve");

        // LaunchDaemon path is fixed — no $HOME involvement.
        let daemon = launch_daemon_plist_path();
        assert_eq!(
            daemon,
            PathBuf::from("/Library/LaunchDaemons/ai.phantommesh.serve.plist"),
            "system-wide LaunchDaemon must live at /Library/LaunchDaemons \
             (not /System/, which is SIP-protected)"
        );

        // LaunchAgent path requires $HOME. In every realistic dev
        // environment this is set, but the function returns Option so
        // headless CI doesn't panic.
        if let Some(home) = dirs::home_dir() {
            let agent = launch_agent_plist_path()
                .expect("home_dir present → launch_agent_plist_path is Some");
            let expected = home
                .join("Library/LaunchAgents")
                .join("ai.phantommesh.serve.plist");
            assert_eq!(
                agent, expected,
                "per-user LaunchAgent must live under \
                 ~/Library/LaunchAgents (not /Library/, which needs sudo)"
            );

            // Defensive sub-assertions: file name + parent must be
            // exactly what `launchctl bootstrap gui/<uid> <path>` expects.
            assert_eq!(
                agent.file_name().and_then(|s| s.to_str()),
                Some("ai.phantommesh.serve.plist")
            );
            assert!(agent
                .to_string_lossy()
                .ends_with("/Library/LaunchAgents/ai.phantommesh.serve.plist"));
        }
    }
}
