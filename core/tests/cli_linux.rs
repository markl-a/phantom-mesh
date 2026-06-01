//! Linux-side CLI integration tests for the `phantom` binary.
//!
//! Counterpart of `cli_macos.rs` + `cli_win.rs`. These exec the
//! actual built binary (via `env!("CARGO_BIN_EXE_phantom")`) and
//! assert observable behavior — exit codes, output shape, side
//! effects — rather than calling internal functions. Slower than
//! unit tests but they catch real packaging/wiring breakage on
//! Linux that unit tests miss (a binary that builds on Win but
//! refuses to bind a port on Linux, for example).
//!
//! Gated `#[cfg(target_os = "linux")]` so non-Linux CI compiles
//! this file to an empty test crate without spawning anything.

#![cfg(target_os = "linux")]

use std::process::Command;
use std::time::Duration;

fn phantom_bin() -> &'static str {
    env!("CARGO_BIN_EXE_phantom")
}

/// LIN P0 — `phantom serve` must bind a TCP port and respond 200 to
/// GET /healthz within 10 s. Uses :17878 to avoid the canonical
/// :7878 that a long-running dev/cluster instance is likely on.
#[tokio::test(flavor = "current_thread")]
async fn serve_starts_linux() {
    let bin = phantom_bin();
    let port: u16 = 17878;

    let mut child = tokio::process::Command::new(bin)
        .arg("serve")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("phantom serve must spawn");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let url = format!("http://127.0.0.1:{}/healthz", port);
    let mut got_200 = false;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() {
                got_200 = true;
                break;
            }
        }
    }

    let _ = child.kill().await;
    let _ = child.wait().await;

    assert!(
        got_200,
        "phantom serve did not respond 200 to GET {} within 10 s — \
         the daemon either failed to bind :{} or /healthz route is broken",
        url, port
    );
}

/// LIN P0 — `phantom doctor` must exit 0 on a healthy dev Linux
/// host (kernel, /proc, journal access all present). Smoke gate
/// equivalent of the macOS / Windows variants — new contributors
/// run this after install to confirm the binary works.
#[test]
fn doctor_exit_zero_linux() {
    let bin = phantom_bin();
    let output = Command::new(bin)
        .arg("doctor")
        .output()
        .expect("phantom doctor must spawn — is the bin built?");

    assert!(
        output.status.success(),
        "phantom doctor exited {:?} on a Linux dev host.\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        !combined.trim().is_empty(),
        "phantom doctor produced empty output — likely short-circuited \
         without running any checks. exit={:?}",
        output.status
    );
}

/// LIN P0 — `phantom service status` must exit cleanly (0 or 1) and
/// mention "registered" in its output. On Linux this checks the
/// systemd `--user` unit; non-installed hosts exit 1 with a clean
/// "not registered" message rather than panic.
#[test]
fn service_status_smoke_linux() {
    let bin = phantom_bin();
    let output = Command::new(bin)
        .arg("service")
        .arg("status")
        .output()
        .expect("phantom service status must spawn");

    let code = output.status.code().unwrap_or(-1);
    assert!(
        code == 0 || code == 1,
        "phantom service status exit code must be 0 or 1, got {}.\n--- stdout ---\n{}\n--- stderr ---\n{}",
        code,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("registered") || stdout.contains("phantom"),
        "phantom service status produced unexpected output: {}",
        stdout
    );
}

/// LIN P0 — `phantom --version` must print a string matching the
/// `phantom X.Y.Z (<sha> <triple> <date>)` provenance shape so that
/// `verify-binary.sh` post-install can grep it.
#[test]
fn version_provenance_linux() {
    let bin = phantom_bin();
    let output = Command::new(bin)
        .arg("--version")
        .output()
        .expect("phantom --version must spawn");

    assert!(
        output.status.success(),
        "phantom --version exit={:?}",
        output.status
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next().unwrap_or("");
    assert!(
        first_line.starts_with("phantom "),
        "first line must start with 'phantom ', got: {first_line:?}",
    );
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    assert!(
        parts.len() >= 2,
        "version line should be `phantom X.Y.Z [(hash triple date)]`, got: {first_line:?}",
    );
    assert!(
        parts[1].chars().filter(|c| *c == '.').count() >= 2,
        "version token should be semver-ish (>=2 dots), got: {:?}",
        parts[1]
    );
}

/// LIN D31 — `phantom autoevolve <garbage>` must REJECT (exit 2), not silently
/// fall through and launch the autonomous evolve loop (cargo check + spawn an
/// LLM code-modifying agent). We only assert the safe rejection + that --help
/// exits 0; the bare `autoevolve` run-loop is deliberately NOT exercised here.
#[test]
fn autoevolve_unknown_subcommand_is_rejected_linux() {
    let bin = phantom_bin();
    let home = tempfile::tempdir().expect("home tempdir");
    let run = |args: &[&str]| {
        Command::new(bin)
            .args(args)
            .env("HOME", home.path())
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap_or_else(|e| panic!("phantom {args:?} must spawn: {e}"))
    };

    for bad in [["autoevolve", "zzzz"], ["autoevolve", "--bogus"]] {
        let out = run(&bad);
        assert_eq!(
            out.status.code(),
            Some(2),
            "`phantom {bad:?}` must exit 2 (not launch the evolve loop), got {:?}\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr),
        );
        let combined =
            String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
        assert!(
            combined.contains("unknown") && !combined.contains("mode       :"),
            "must print an 'unknown argument' error and NOT the autoevolve run banner, got: {combined}"
        );
    }

    // --help must still work (exit 0).
    let help = run(&["autoevolve", "--help"]);
    assert!(help.status.success(), "autoevolve --help must exit 0");
}

/// LIN D22 — `phantom <sub> --help` must print usage and exit 0 WITHOUT
/// launching the subcommand. Before the fix, `onboarding --help` hung
/// (opened a browser), `repl`/`tui --help` launched the UI, `init --help`
/// wrote PHANTOM.md into the cwd, and `sessions --help` hit the broker.
/// Each invocation runs in an isolated HOME + cwd; we assert it terminates
/// within a hard timeout (a hang regression fails instead of blocking CI),
/// emits "usage", and leaves no PHANTOM.md behind.
#[tokio::test(flavor = "current_thread")]
async fn help_flag_never_executes_subcommand_linux() {
    let bin = phantom_bin();
    // Includes the pre-existing dangerous guards (serve binds a port, mcp
    // blocks on stdio, coordinator hubs, evolve starts a fix loop) so the whole
    // guard list is regression-covered, not just the D22 additions.
    for sub in [
        "onboarding", "tui", "repl", "init", "sessions", "hello",
        "serve", "mcp", "coordinator", "evolve",
    ] {
        for flag in ["--help", "-h"] {
            let home = tempfile::tempdir().expect("home tempdir");
            let cwd = tempfile::tempdir().expect("cwd tempdir");

            let run = tokio::process::Command::new(bin)
                .arg(sub)
                .arg(flag)
                .current_dir(cwd.path())
                .env("HOME", home.path())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .output();

            let output = tokio::time::timeout(Duration::from_secs(8), run)
                .await
                .unwrap_or_else(|_| panic!("`phantom {sub} {flag}` HUNG (>8s) — it launched the subcommand instead of printing help"))
                .unwrap_or_else(|e| panic!("`phantom {sub} {flag}` failed to spawn: {e}"));

            assert!(
                output.status.success(),
                "`phantom {sub} {flag}` must exit 0, got {:?}",
                output.status.code()
            );
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                combined.to_lowercase().contains("usage"),
                "`phantom {sub} {flag}` should print usage, got: {combined}"
            );
            assert!(
                !cwd.path().join("PHANTOM.md").exists(),
                "`phantom {sub} {flag}` must NOT write PHANTOM.md (init side-effect leak)"
            );
        }
    }
}

/// LIN D23 — invalid argument VALUES must error (exit 2), not silently fall
/// back to a default. Before the fix: `serve --port abc` bound the default
/// port, `focus --minutes abc` started a 25-min timer, `lang set zzz` saved
/// "en" and reported success, `coach review --date notadate` rendered a
/// bogus review, and `exec --jsonn` ran in human-output mode (CI footgun).
/// Each runs in an isolated HOME so it can't touch the operator's real store.
#[test]
fn invalid_arg_values_exit_2_linux() {
    let bin = phantom_bin();
    let home = tempfile::tempdir().expect("home tempdir");
    let cases: &[&[&str]] = &[
        &["serve", "--port", "abc"],
        &["serve", "--port", "99999"], // out of u16 range
        &["focus", "start", "--minutes", "abc"],
        &["focus", "start", "--minutes", "-5"],
        &["lang", "set", "zzz"],
        &["lang", "set", "zh-CN"], // Simplified not shipped
        &["coach", "review", "--date", "notadate"],
        &["exec", "--jsonn"], // typo'd flag must not be silently ignored
    ];
    for args in cases {
        let output = Command::new(bin)
            .args(*args)
            .env("HOME", home.path())
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap_or_else(|e| panic!("phantom {args:?} must spawn: {e}"));
        assert_eq!(
            output.status.code(),
            Some(2),
            "`phantom {args:?}` must exit 2 on invalid input, got {:?}\n--- stderr ---\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

/// LIN D23 — the same flags with VALID values must still succeed (no
/// over-rejection regression). `lang set` + `focus start` write into HOME.
#[test]
fn valid_arg_values_still_succeed_linux() {
    let bin = phantom_bin();
    let home = tempfile::tempdir().expect("home tempdir");
    let cases: &[&[&str]] = &[
        &["lang", "set", "en"],
        &["lang", "set", "zh-TW"],
        &["focus", "start", "--minutes", "50"],
        &["coach", "review", "--date", "2026-05-30"],
    ];
    for args in cases {
        let output = Command::new(bin)
            .args(*args)
            .env("HOME", home.path())
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap_or_else(|e| panic!("phantom {args:?} must spawn: {e}"));
        assert!(
            output.status.success(),
            "`phantom {args:?}` must succeed on valid input, got {:?}\n--- stderr ---\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

/// LIN D24 — a PRESENT-but-corrupt `identity.key` must make `phantom note`
/// REFUSE (nonzero exit) and write NO event, rather than silently downgrading
/// the private note to plaintext. (A genuinely-absent key is the separate
/// pre-encryption state where plaintext is intended — covered by unit tests.)
#[test]
fn corrupt_identity_key_refuses_plaintext_note_linux() {
    let bin = phantom_bin();
    let home = tempfile::tempdir().expect("home tempdir");
    let phantom_dir = home.path().join(".phantom-mesh");
    std::fs::create_dir_all(&phantom_dir).unwrap();
    // 5 bytes < the 16-byte minimum → present but unloadable.
    std::fs::write(phantom_dir.join("identity.key"), [0x01u8; 5]).unwrap();

    let output = Command::new(bin)
        .args(["note", "this private note must not hit disk in plaintext"])
        .env("HOME", home.path())
        .stdin(std::process::Stdio::null())
        .output()
        .expect("phantom note must spawn");

    assert!(
        !output.status.success(),
        "phantom note with a corrupt identity.key must exit nonzero, got {:?}\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );

    // The note event store must contain NO event — the capture was refused, not
    // silently written in plaintext. (events.jsonl, the argv diag, is a separate
    // concern fixed under D21 on its own branch — we only assert the event dir.)
    let events_dir = phantom_dir.join("events");
    let wrote_event = events_dir
        .read_dir()
        .map(|rd| rd.flatten().any(|e| e.path().is_file()))
        .unwrap_or(false);
    assert!(
        !wrote_event,
        "a refused note must write no event file, but {events_dir:?} has one"
    );
}

/// LIN D28 — a non-UTF8 argv byte must NOT crash the CLI. `std::env::args()`
/// `.unwrap()`s on non-UTF8 and aborted the whole process (exit 101) for any
/// stray byte; `args_os` + lossy conversion must degrade gracefully instead.
#[test]
fn non_utf8_argv_does_not_panic_linux() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let bin = phantom_bin();
    let home = tempfile::tempdir().expect("home tempdir");
    let bad = OsStr::from_bytes(&[0xff, 0xfe]); // invalid UTF-8

    // Cover the distinct argv positions the bad byte can land in:
    //  - trailing arg (args[2]) — downstream consumers,
    //  - subcommand slot (args[1]) — the `args.get(1)` dispatch match,
    //  - a flag value (args[2] after a flag) — parse_flag consumers,
    //  - the bad byte as the ONLY arg.
    let invocations: Vec<Vec<&OsStr>> = vec![
        vec![OsStr::new("whoami"), bad],
        vec![OsStr::new("doctor"), bad],
        vec![OsStr::new("recall"), bad],
        vec![bad],                                   // bad subcommand
        vec![OsStr::new("exec"), OsStr::new("--config"), bad], // bad flag value
    ];
    for argv in invocations {
        let output = Command::new(bin)
            .args(&argv)
            .env("HOME", home.path())
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap_or_else(|e| panic!("phantom {argv:?} must spawn: {e}"));
        // 101 is the Rust panic/abort exit code — the bug we're guarding against.
        assert_ne!(
            output.status.code(),
            Some(101),
            "`phantom {argv:?}` panicked (exit 101) instead of degrading gracefully\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("panicked"),
            "`phantom {argv:?}` printed a panic message"
        );
    }
}
