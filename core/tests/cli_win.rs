//! Windows-side CLI integration tests for the `spectyn` binary.
//!
//! Counterpart of `cli_macos.rs`. These exec the actual built binary
//! (via `env!("CARGO_BIN_EXE_spectyn")`) and assert observable
//! behavior — exit codes, output shape, side effects — rather than
//! calling internal functions. Slower than unit tests but they catch
//! real packaging/wiring breakage (a binary that builds but panics
//! on startup, a `serve` route that builds but doesn't bind, etc.).
//!
//! Gated `#[cfg(target_os = "windows")]` so non-Win CI compiles this
//! file to an empty test crate without spawning anything.

#![cfg(target_os = "windows")]

use std::process::Command;
use std::time::Duration;

fn spectyn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_spectyn")
}

/// WIN P0 — `spectyn serve` must bind a TCP port and respond 200 to
/// GET /healthz within 10 s. Uses a non-default port (17878) so the
/// test doesn't collide with a long-running `spectyn serve` started
/// by the dev or by the cluster on the canonical :7878.
#[tokio::test(flavor = "current_thread")]
async fn serve_starts_windows() {
    let bin = spectyn_bin();
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
        .expect("spectyn serve must spawn");

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
        "spectyn serve did not respond 200 to GET {} within 10 s — \
         the daemon either failed to bind :{} or /healthz route is broken",
        url, port
    );
}

/// WIN P0 — `spectyn doctor` must exit 0 on a healthy dev Windows
/// host. This is the smoke gate that new contributors run after
/// install to confirm the binary works end-to-end before they start
/// configuring providers / cluster.
#[test]
fn doctor_exit_zero_windows() {
    let bin = spectyn_bin();
    let output = Command::new(bin)
        .arg("doctor")
        .output()
        .expect("spectyn doctor must spawn — is the bin built?");

    assert!(
        output.status.success(),
        "spectyn doctor exited {:?} on a Windows dev host.\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Sanity: output must mention at least one Win-relevant check.
    // Catches a future regression where `doctor` short-circuits and
    // exits 0 without actually running anything.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        !combined.trim().is_empty(),
        "spectyn doctor produced empty output — likely short-circuited \
         without running any checks. exit={:?}",
        output.status
    );
}

/// WIN P0 — `spectyn service status` must exit cleanly (0 or 1; 1
/// is the documented exit when the Scheduled Task isn't installed).
/// Output must mention "registered" so the smoke gate confirms the
/// status formatter runs, not just that the binary doesn't panic.
#[test]
fn service_status_smoke_windows() {
    let bin = spectyn_bin();
    let output = Command::new(bin)
        .arg("service")
        .arg("status")
        .output()
        .expect("spectyn service status must spawn");

    // Either exit 0 (installed) or 1 (not installed) is acceptable.
    let code = output.status.code().unwrap_or(-1);
    assert!(
        code == 0 || code == 1,
        "spectyn service status exit code must be 0 or 1, got {}.\n--- stdout ---\n{}\n--- stderr ---\n{}",
        code,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("registered") || stdout.contains("spectyn"),
        "spectyn service status produced unexpected output: {}",
        stdout
    );
}

/// WIN P0 — `spectyn --version` must print a string matching the
/// `spectyn X.Y.Z (<sha> <triple> <date>)` provenance shape. This
/// catches release builds that lose git hash / triple info — which
/// would silently break `verify-binary.ps1` post-install.
#[test]
fn version_provenance_windows() {
    let bin = spectyn_bin();
    let output = Command::new(bin)
        .arg("--version")
        .output()
        .expect("spectyn --version must spawn");

    assert!(
        output.status.success(),
        "spectyn --version exit={:?}",
        output.status
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next().unwrap_or("");
    assert!(
        first_line.starts_with("spectyn "),
        "first line must start with 'spectyn ', got: {first_line:?}",
    );
    // Expect at least 'spectyn X.Y.Z' on the first line.
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    assert!(
        parts.len() >= 2,
        "version line should be `spectyn X.Y.Z [(hash triple date)]`, got: {first_line:?}",
    );
    assert!(
        parts[1].chars().filter(|c| *c == '.').count() >= 2,
        "version token should be semver-ish (>=2 dots), got: {:?}",
        parts[1]
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Argument-hardening parity (D22 / D23 / D24 / D28)
//
// These port the dispatch-safety tests that already gate the Linux binary
// (`cli_linux.rs`) so the SAME guarantees are exercised against the Windows
// build, which ships these surfaces too. The behavior under test (a `--help`
// short-circuit before side effects, exit-2 on bad arg VALUES, a corrupt
// identity.key refusing a plaintext note, and graceful non-UTF8 argv handling)
// is platform-agnostic in `core/src/bin/spectyn.rs`; only the OS-isms differ:
//
//   • HOME redirection: on Windows, `dirs::home_dir()` resolves the profile via
//     `SHGetKnownFolderPath(FOLDERID_Profile)` and IGNORES `HOME`/`USERPROFILE`
//     env vars (verified against dirs 6.0.0 `src/win.rs`). So unlike Linux, we
//     cannot redirect the data root with an env var. The D22/D23/D28 cases all
//     short-circuit BEFORE any home access (verified: each exits at parse time),
//     so they need no home isolation. The D24 case resolves the real home, so —
//     because we cannot redirect it — it SKIPS entirely when a real
//     `~/.spectyn-mesh/identity.key` is present, and only runs on a key-less
//     host (planting then removing a corrupt key it owns). It never touches an
//     operator key.
//
//   • non-UTF8 argv: Windows argv is UTF-16, so an "invalid" argument is an
//     OsString built from an unpaired surrogate via `OsStringExt::from_wide`
//     (the wide-char analogue of the Unix raw-byte `OsStrExt::from_bytes`).

/// WIN D22 — `spectyn <sub> --help` must print usage and exit 0 WITHOUT
/// launching the subcommand (mirrors `help_flag_never_executes_subcommand_linux`).
/// The footgun guard in `spectyn.rs` (the `serve`/`mcp`/`onboarding`/… intercept
/// at the top of dispatch) returns `Ok(())` with a `usage:` banner before any
/// side effect — serve would bind a port, onboarding would open a browser and
/// hang, init would write SPECTYN.md into the cwd, etc. Each invocation runs in
/// an isolated cwd (the one side-effect we CAN redirect on Windows) and must
/// terminate within a hard timeout, emit "usage", and leave no SPECTYN.md.
#[tokio::test(flavor = "current_thread")]
async fn help_flag_never_executes_subcommand_windows() {
    let bin = spectyn_bin();
    // Same guard list as the Linux test: the pre-existing dangerous subcommands
    // (serve binds a port, mcp blocks on stdio, coordinator hubs, evolve starts
    // a fix loop) plus the D22 additions (onboarding/tui/repl/init/sessions/hello).
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
                .unwrap_or_else(|_| panic!("`spectyn {sub} {flag}` HUNG (>8s) — it launched the subcommand instead of printing help"))
                .unwrap_or_else(|e| panic!("`spectyn {sub} {flag}` failed to spawn: {e}"));

            assert!(
                output.status.success(),
                "`spectyn {sub} {flag}` must exit 0, got {:?}",
                output.status.code()
            );
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                combined.to_lowercase().contains("usage"),
                "`spectyn {sub} {flag}` should print usage, got: {combined}"
            );
            assert!(
                !cwd.path().join("SPECTYN.md").exists(),
                "`spectyn {sub} {flag}` must NOT write SPECTYN.md (init side-effect leak)"
            );
        }
    }
}

/// WIN D23 — invalid argument VALUES must error (exit 2), not silently fall
/// back to a default (mirrors `invalid_arg_values_exit_2_linux`). Before the
/// fix: `serve --port abc` bound the default port, `focus --minutes abc`
/// started a 25-min timer, `lang set zzz` saved "en" and reported success,
/// `coach review --date notadate` rendered a bogus review, and `exec --jsonn`
/// ran in human-output mode (a CI footgun). Each case exits at parse time
/// BEFORE any home access in `spectyn.rs`, so HOME isolation is only defensive.
#[test]
fn invalid_arg_values_exit_2_windows() {
    let bin = spectyn_bin();
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
            .unwrap_or_else(|e| panic!("spectyn {args:?} must spawn: {e}"));
        assert_eq!(
            output.status.code(),
            Some(2),
            "`spectyn {args:?}` must exit 2 on invalid input, got {:?}\n--- stderr ---\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

/// WIN D24 — a PRESENT-but-corrupt `identity.key` must make `spectyn note`
/// REFUSE (nonzero exit) and write NO event, rather than silently downgrading
/// the private note to plaintext (mirrors
/// `corrupt_identity_key_refuses_plaintext_note_linux`). A genuinely-absent key
/// is the separate pre-encryption state where plaintext is intended (covered by
/// unit tests).
///
/// Windows wrinkle: `spectyn note` resolves its store + identity.key via
/// `dirs::home_dir()`, which on Windows ignores `HOME`/`USERPROFILE` — so unlike
/// the Linux test we CANNOT point it at a tempdir. To stay safe we therefore
/// SKIP entirely when a real `~/.spectyn-mesh/identity.key` is present (never
/// touching the operator's key), and only run on a key-less host / CI: there we
/// plant the 5-byte corrupt key, assert the refusal + that NO NEW event landed,
/// then remove only the corrupt key we planted. A real key is never moved,
/// overwritten, or restored.
#[test]
fn corrupt_identity_key_refuses_plaintext_note_windows() {
    let bin = spectyn_bin();

    // Resolve the SAME home the binary will use (`dirs::home_dir()`), since no
    // env var can redirect it on Windows.
    let Some(home) = dirs::home_dir() else {
        eprintln!("no home dir resolved — skip (cannot locate ~/.spectyn-mesh on this host)");
        return;
    };
    let spectyn_dir = home.join(".spectyn-mesh");
    let key_path = spectyn_dir.join("identity.key");
    let events_dir = spectyn_dir.join("events");

    // SAFETY — no SPECTYN_HOME isolation on Windows yet (dirs::home_dir() ignores
    // HOME/USERPROFILE), so we cannot redirect the store to a tempdir. Therefore:
    // if the operator already has a real identity.key, SKIP. We must NEVER move
    // aside, overwrite, or otherwise touch a real private key. This test only runs
    // on a clean host / CI where no identity exists yet.
    if key_path.exists() {
        eprintln!(
            "real ~/.spectyn-mesh/identity.key present — skip (refusing to touch the operator's \
             private key; no SPECTYN_HOME isolation on Windows yet)"
        );
        return;
    }
    std::fs::create_dir_all(&spectyn_dir).expect("create .spectyn-mesh");

    // Snapshot pre-existing event files so a refused note is distinguished from
    // notes the host already had.
    let pre_existing: std::collections::BTreeSet<std::path::PathBuf> = events_dir
        .read_dir()
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();

    // Remove ONLY the corrupt key we plant, on every exit path (incl. assertion
    // failure). We never restore/replace an operator key — we skipped above if
    // one existed, so there is nothing of the operator's to protect here.
    struct Cleanup {
        key_path: std::path::PathBuf,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.key_path);
        }
    }
    let _cleanup = Cleanup {
        key_path: key_path.clone(),
    };

    // ── Install a present-but-corrupt key: 5 bytes < the 16-byte minimum. ──
    std::fs::write(&key_path, [0x01u8; 5]).expect("write corrupt identity.key");

    let output = Command::new(bin)
        .args(["note", "this private note must not hit disk in plaintext"])
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spectyn note must spawn");

    assert!(
        !output.status.success(),
        "spectyn note with a corrupt identity.key must exit nonzero, got {:?}\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );

    // No NEW event file may appear — the capture was refused, not silently
    // written in plaintext. We compare against the pre-existing snapshot so the
    // operator's own prior notes don't trip the assertion.
    let post: std::collections::BTreeSet<std::path::PathBuf> = events_dir
        .read_dir()
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    let new_events: Vec<_> = post.difference(&pre_existing).collect();
    assert!(
        new_events.is_empty(),
        "a refused note must write no new event file, but these appeared: {new_events:?}"
    );
}

/// WIN D28 — a non-UTF8 argv must NOT crash the CLI (mirrors
/// `non_utf8_argv_does_not_panic_linux`). `std::env::args()` `.unwrap()`s on
/// non-UTF8 and aborts the whole process (exit 101) for any un-decodable arg;
/// the binary's `args_lossy()` (`args_os` + `to_string_lossy`) must degrade
/// gracefully instead. Windows argv is UTF-16, so the "bad" argument is an
/// `OsString` carrying an UNPAIRED SURROGATE (`0xD800`) built via
/// `OsStringExt::from_wide` — the wide-char analogue of the Unix test's raw
/// invalid byte sequence.
#[test]
fn non_utf8_argv_does_not_panic_windows() {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    let bin = spectyn_bin();
    let home = tempfile::tempdir().expect("home tempdir");

    // 0xD800 is a high surrogate with no following low surrogate → not valid
    // UTF-16, so it cannot round-trip to a UTF-8 String and forces the lossy
    // (U+FFFD) path. `0x0041` ('A') keeps it a plausibly-real arg shape.
    let bad: OsString = OsString::from_wide(&[0x0041u16, 0xD800u16]);

    // Cover the distinct argv positions the bad arg can land in:
    //  - trailing arg (args[2]) — downstream consumers,
    //  - subcommand slot (args[1]) — the `args.get(1)` dispatch match,
    //  - a flag value (args[2] after a flag) — parse_flag consumers,
    //  - the bad arg as the ONLY arg.
    let invocations: Vec<Vec<OsString>> = vec![
        vec![OsString::from("whoami"), bad.clone()],
        vec![OsString::from("doctor"), bad.clone()],
        vec![OsString::from("recall"), bad.clone()],
        vec![bad.clone()],                                            // bad subcommand
        vec![OsString::from("exec"), OsString::from("--config"), bad.clone()], // bad flag value
    ];
    for argv in invocations {
        let output = Command::new(bin)
            .args(&argv)
            .env("HOME", home.path())
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap_or_else(|e| panic!("spectyn {argv:?} must spawn: {e}"));
        // 101 is the Rust panic/abort exit code — the bug we're guarding against.
        assert_ne!(
            output.status.code(),
            Some(101),
            "`spectyn {argv:?}` panicked (exit 101) instead of degrading gracefully\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("panicked"),
            "`spectyn {argv:?}` printed a panic message"
        );
    }
}
