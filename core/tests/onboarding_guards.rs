//! Regression guards for `phantom onboarding` so it never HANGS on a headless
//! box trying to open a browser / spin up the onboarding web server.
//!
//! Two guards, both checked BEFORE `run_web_onboarding` is reached:
//!   (a) `phantom onboarding -h` / `--help` → usage to stdout + exit 0, fast.
//!   (b) when stdin is NOT a terminal (piped / closed), print actionable
//!       guidance and exit 2 — DON'T launch the web onboarding (which binds a
//!       port + opens a browser and blocks forever in CI).
//!
//! These spawn the REAL binary (CARGO_BIN_EXE_phantom). The non-TTY case is
//! reproduced by giving the child a piped (empty) stdin: `Stdio::piped()` +
//! immediate drop ⇒ EOF on a non-terminal fd, exactly the headless shape.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn phantom_bin() -> &'static str {
    env!("CARGO_BIN_EXE_phantom")
}

/// `phantom onboarding --help` must short-circuit to usage + exit 0, fast, and
/// MUST NOT fall through into the web-onboarding (which would block).
#[test]
fn onboarding_help_exits_0_with_usage_fast() {
    for flag in ["--help", "-h"] {
        let start = Instant::now();
        let output = Command::new(phantom_bin())
            .args(["onboarding", flag])
            .stdin(Stdio::null())
            .output()
            .expect("phantom onboarding --help must spawn");
        let elapsed = start.elapsed();

        assert_eq!(
            output.status.code(),
            Some(0),
            "`onboarding {flag}` must exit 0, not {:?}",
            output.status.code()
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "`onboarding {flag}` must be near-instant, took {:?}",
            elapsed
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.to_lowercase().contains("usage")
                || stdout.to_lowercase().contains("onboarding"),
            "`onboarding {flag}` stdout must contain usage:\n{stdout}"
        );
    }
}

/// `phantom onboarding` with a non-terminal stdin (piped/empty) must NOT launch
/// the web onboarding. It must print actionable guidance and exit 2, quickly,
/// and bind NO onboarding port (7878..=7888) while doing so.
#[test]
fn onboarding_non_tty_exits_2_binds_no_port() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();

    // Snapshot which onboarding ports are already held by SOMETHING ELSE in
    // this environment (e.g. a running `phantom serve`). We only care that the
    // guarded onboarding process doesn't bind a NEW one, so we exclude these
    // pre-existing listeners from the post-run "must be free" assertion.
    let pre_busy: Vec<u16> = (7878..=7888u16)
        .filter(|p| std::net::TcpListener::bind(("127.0.0.1", *p)).is_err())
        .collect();

    let start = Instant::now();
    let mut child = Command::new(phantom_bin())
        .arg("onboarding")
        .current_dir(cwd.path())
        .env("HOME", home.path())
        // Piped stdin that we drop immediately ⇒ a closed, non-terminal fd:
        // exactly the headless / CI shape that used to hang on the browser.
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("phantom onboarding must spawn");

    // Drop the child's stdin handle → EOF, no terminal.
    drop(child.stdin.take());

    // It must finish FAST. If the guard is missing it would hang on the web
    // server; bound the wait and fail loudly rather than hang the test.
    let deadline = start + Duration::from_secs(2);
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_status) => break,
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "`phantom onboarding` (non-TTY) did not exit within 2s — \
                         it is launching the web onboarding instead of guarding"
                    );
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    let output = child.wait_with_output().expect("wait_with_output");
    assert_eq!(
        output.status.code(),
        Some(2),
        "non-TTY `onboarding` must exit 2 (guidance), not {:?}",
        output.status.code()
    );

    // It must have printed actionable guidance (on stderr per the bare-phantom
    // guard convention).
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.to_lowercase().contains("terminal")
            || stderr.to_lowercase().contains("browser")
            || stderr.to_lowercase().contains("onboarding"),
        "non-TTY `onboarding` must print actionable guidance:\n{stderr}"
    );

    // And it must NOT have bound an onboarding port: if the guard worked, every
    // port in the onboarding range (minus any that were ALREADY held by an
    // unrelated process before we spawned) is free for us to bind now. A port
    // that was free pre-run but is busy post-run = the guard launched the web
    // server, which is exactly the regression we forbid.
    for p in 7878..=7888u16 {
        if pre_busy.contains(&p) {
            continue;
        }
        let bound = std::net::TcpListener::bind(("127.0.0.1", p));
        assert!(
            bound.is_ok(),
            "onboarding port {p} was free before but is busy now — the guard \
             must NOT have started the web server: {:?}",
            bound.err()
        );
    }
}
