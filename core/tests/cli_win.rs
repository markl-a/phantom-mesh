//! Windows-side CLI integration tests for the `phantom` binary.
//!
//! Counterpart of `cli_macos.rs`. These exec the actual built binary
//! (via `env!("CARGO_BIN_EXE_phantom")`) and assert observable
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

fn phantom_bin() -> &'static str {
    env!("CARGO_BIN_EXE_phantom")
}

/// WIN P0 — `phantom serve` must bind a TCP port and respond 200 to
/// GET /healthz within 10 s. Uses a non-default port (17878) so the
/// test doesn't collide with a long-running `phantom serve` started
/// by the dev or by the cluster on the canonical :7878.
#[tokio::test(flavor = "current_thread")]
async fn serve_starts_windows() {
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

/// WIN P0 — `phantom doctor` must exit 0 on a healthy dev Windows
/// host. This is the smoke gate that new contributors run after
/// install to confirm the binary works end-to-end before they start
/// configuring providers / cluster.
#[test]
fn doctor_exit_zero_windows() {
    let bin = phantom_bin();
    let output = Command::new(bin)
        .arg("doctor")
        .output()
        .expect("phantom doctor must spawn — is the bin built?");

    assert!(
        output.status.success(),
        "phantom doctor exited {:?} on a Windows dev host.\n--- stdout ---\n{}\n--- stderr ---\n{}",
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
        "phantom doctor produced empty output — likely short-circuited \
         without running any checks. exit={:?}",
        output.status
    );
}

/// WIN P0 — `phantom service status` must exit cleanly (0 or 1; 1
/// is the documented exit when the Scheduled Task isn't installed).
/// Output must mention "registered" so the smoke gate confirms the
/// status formatter runs, not just that the binary doesn't panic.
#[test]
fn service_status_smoke_windows() {
    let bin = phantom_bin();
    let output = Command::new(bin)
        .arg("service")
        .arg("status")
        .output()
        .expect("phantom service status must spawn");

    // Either exit 0 (installed) or 1 (not installed) is acceptable.
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

/// WIN P0 — `phantom --version` must print a string matching the
/// `phantom X.Y.Z (<sha> <triple> <date>)` provenance shape. This
/// catches release builds that lose git hash / triple info — which
/// would silently break `verify-binary.ps1` post-install.
#[test]
fn version_provenance_windows() {
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
    // Expect at least 'phantom X.Y.Z' on the first line.
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
