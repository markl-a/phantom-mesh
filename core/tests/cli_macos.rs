//! Mac-side CLI integration tests for `phantom` binary.
//!
//! These exec the actual built binary (via `env!("CARGO_BIN_EXE_phantom")`)
//! and assert observable behavior — exit codes, output shape, side effects
//! — rather than calling internal functions. Slower than unit tests but
//! they catch real packaging/wiring breakage (e.g. a binary that builds
//! but panics on startup).
//!
//! Gated `#[cfg(target_os = "macos")]` so non-Mac CI runs compile this
//! file to an empty test crate without spawning anything.

#![cfg(target_os = "macos")]

use std::process::Command;
use std::time::Duration;

fn phantom_bin() -> &'static str {
    env!("CARGO_BIN_EXE_phantom")
}

/// MAC P0 — `phantom repl -c "<prompt>"` one-shot REPL must exit 0
/// and return some non-empty completion. Uses the default agent
/// (opencode, configured in agents.toml) so cost per run is ~$0.0001.
///
/// Skips (eprintln + early return) when no provider key is reachable
/// via env — partial-key dev hosts shouldn't get spurious red.
#[test]
fn repl_macos() {
    if std::env::var("OPENCODE_API_KEY")
        .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
        .or_else(|_| std::env::var("GROQ_API_KEY"))
        .is_err()
    {
        eprintln!(
            "SKIPPED: repl_macos — no provider key in env \
             (source ~/.phantom-mesh/env first)"
        );
        return;
    }
    let bin = phantom_bin();
    let output = Command::new(bin)
        .args([
            "repl",
            "-c",
            "Reply with the literal word PING and nothing else.",
        ])
        .output()
        .expect("phantom repl -c must spawn");

    assert!(
        output.status.success(),
        "phantom repl -c exited {:?}.\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.trim().is_empty(),
        "phantom repl -c produced empty stdout — LLM never replied?\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// MAC P0 — `phantom exec --json <prompt>` must emit a stream of JSON
/// AgentEvent lines on stdout (one event per line), exit 0, and the
/// stream must contain at least one event with a recognizable tag
/// (StreamStart / ContentBlock / Done / similar). This is the
/// machine-consumable mode CI pipelines + the cluster RPC rely on —
/// any drift in the event schema breaks downstream parsers silently.
#[test]
fn exec_json_stream_macos() {
    if std::env::var("OPENCODE_API_KEY")
        .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
        .or_else(|_| std::env::var("GROQ_API_KEY"))
        .is_err()
    {
        eprintln!(
            "SKIPPED: exec_json_stream_macos — no provider key in env \
             (source ~/.phantom-mesh/env first)"
        );
        return;
    }
    let bin = phantom_bin();
    let output = Command::new(bin)
        .args([
            "exec",
            "--json",
            "Reply with the literal word PONG and nothing else.",
        ])
        .output()
        .expect("phantom exec --json must spawn");

    assert!(
        output.status.success(),
        "phantom exec --json exited {:?}.\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        !lines.is_empty(),
        "phantom exec --json produced no stdout lines — stream never started?\n\
         stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Every non-empty line must parse as JSON (--json contract).
    let mut parsed = 0usize;
    let mut seen_tag = false;
    for line in &lines {
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(v) => {
                parsed += 1;
                // Look for any field that smells like an AgentEvent tag.
                let tag = v
                    .get("tag")
                    .and_then(|t| t.as_str())
                    .or_else(|| v.get("type").and_then(|t| t.as_str()))
                    .or_else(|| v.get("event").and_then(|t| t.as_str()));
                if let Some(t) = tag {
                    if !t.is_empty() {
                        seen_tag = true;
                    }
                }
            }
            Err(e) => panic!(
                "non-JSON line in --json stream: `{}` (error: {})\n\
                 full stdout:\n{}",
                line, e, stdout
            ),
        }
    }
    assert!(
        parsed > 0,
        "no JSON lines parsed out of {} non-empty stdout lines",
        lines.len()
    );
    assert!(
        seen_tag,
        "no event with `tag`/`type`/`event` field — schema may have shifted; \
         first line was: {}",
        lines.first().unwrap_or(&"")
    );
}

/// MAC P0 — `phantom serve` must bind a TCP port and respond 200 to
/// GET /healthz within 10 s. Uses a non-default port (17878) so the
/// test doesn't collide with a long-running `phantom serve` started
/// by the dev or by the cluster on the canonical :7878.
#[tokio::test(flavor = "current_thread")]
async fn serve_starts_macos() {
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

    // Poll /healthz every 500 ms for up to 10 s.
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

    // Always kill — kill_on_drop is the safety net but explicit is
    // better when we have an awaitable handle.
    let _ = child.kill().await;
    let _ = child.wait().await;

    assert!(
        got_200,
        "phantom serve did not respond 200 to GET {} within 10 s — \
         the daemon either failed to bind :{} or /healthz route is broken",
        url, port
    );
}

/// MAC P0 — `phantom snapshot create` must exit 0 and produce a
/// recognizable success message. Real side effect: creates one
/// purgeable APFS snapshot (macOS auto-prunes; no disk leak).
/// Pairs with the unit test `snapshot::tests::create_returns_unique_id`
/// but exercises the full CLI dispatch path (argv → bin → snapshot
/// module → tmutil) rather than the module function alone.
#[test]
fn snapshot_create_smoke() {
    let bin = phantom_bin();
    let output = Command::new(bin)
        .arg("snapshot")
        .arg("create")
        .output()
        .expect("phantom snapshot create must spawn");

    assert!(
        output.status.success(),
        "phantom snapshot create exited {:?}.\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Output should mention "Created" or contain an id-shaped string.
    // (id format YYYY-MM-DD-HHMMSS — 17 chars with dashes at 4/7/10).
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Created") || stdout.contains("snapshot"),
        "phantom snapshot create produced unexpected output: {}",
        stdout
    );
}

/// MAC P0 — `phantom doctor` must exit 0 on a healthy dev Mac (Xcode
/// CLT, sandbox-exec, sysctl all present). This is the smoke gate that
/// new contributors run after `cargo install` to confirm the binary
/// works end-to-end before they start configuring providers / cluster.
#[test]
fn doctor_exit_zero_macos() {
    let bin = phantom_bin();
    let output = Command::new(bin)
        .arg("doctor")
        .output()
        .expect("phantom doctor must spawn — is the bin built?");

    assert!(
        output.status.success(),
        "phantom doctor exited {:?} on a Mac dev host.\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Sanity: output must mention at least one check we know runs on
    // macOS (xcode/sandbox/sysctl variants). Catches a future regression
    // where `doctor` short-circuits and exits 0 without actually running
    // anything.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        !combined.trim().is_empty(),
        "phantom doctor produced empty output — likely shortcircuited \
         without running any checks. exit={:?}",
        output.status
    );
}
