//! Xcode toolchain wrappers for phantom — `xcrun simctl` first, with room
//! for `xcodebuild` and `swift package` in follow-up commits.
//!
//! Goal: bring real Apple developer workflows under phantom subagent control
//! — boot a simulator, install an app, list runtimes, etc.
//!
//! Gated `#[cfg(target_os = "macos")]`. Returns a clear error if Xcode
//! command-line tools are not installed.

use serde_json::Value;
use tokio::process::Command;

const ALLOWED_ACTIONS: &[&str] = &[
    "list",
    "boot",
    "shutdown",
    "shutdown_all",
    "erase",
    "erase_all",
    "install",
    "uninstall",
    "launch",
    "terminate",
    "openurl",
    "io",          // screenshot / video record (we expose just screenshot below)
    "screenshot",  // alias for `simctl io booted screenshot <path>`
];

/// `xcode_simctl({action, args})`
///
/// Args (JSON):
/// - `action` (required): one of {list, boot, shutdown, shutdown_all,
///   erase, erase_all, install, uninstall, launch, terminate, openurl,
///   screenshot}.
/// - `args` (optional, array of strings): forwarded after the action verb.
///   For `screenshot`, if no path is given we default to
///   `/tmp/phantom-sim-<timestamp>.png`.
/// - `device` (optional, string): defaults to `booted` for verbs that need
///   a target device.
pub async fn simctl(args: &Value) -> String {
    let action = match args.get("action").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return format!(
                "[xcode_simctl] missing 'action'. allowed: {}",
                ALLOWED_ACTIONS.join(", ")
            )
        }
    };

    if !ALLOWED_ACTIONS.contains(&action.as_str()) {
        return format!(
            "[xcode_simctl] unknown action '{}'. allowed: {}",
            action,
            ALLOWED_ACTIONS.join(", ")
        );
    }

    // Pre-flight: is Xcode CLT installed?
    if Command::new("xcrun")
        .arg("--find")
        .arg("simctl")
        .output()
        .await
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        return "[xcode_simctl] xcrun simctl not found. \
            Install Xcode command-line tools with: xcode-select --install"
            .to_string();
    }

    let device = args
        .get("device")
        .and_then(|v| v.as_str())
        .unwrap_or("booted")
        .to_string();

    let mut argv: Vec<String> = vec!["simctl".to_string()];

    match action.as_str() {
        "list" => {
            // Optional sub-domain: runtimes / devices / devicetypes / pairs
            if let Some(sub) = args.get("args").and_then(|v| v.as_array()) {
                argv.extend(sub.iter().filter_map(|x| x.as_str().map(String::from)));
            } else {
                argv.push("list".into());
            }
        }
        "shutdown_all" => {
            argv.push("shutdown".into());
            argv.push("all".into());
        }
        "erase_all" => {
            argv.push("erase".into());
            argv.push("all".into());
        }
        "screenshot" => {
            argv.push("io".into());
            argv.push(device.clone());
            argv.push("screenshot".into());
            // Output path: arg-supplied or auto.
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| {
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    format!("/tmp/phantom-sim-{}.png", ts)
                });
            argv.push(path);
        }
        // Verbs that take a device: boot/shutdown/erase/install/uninstall/launch/terminate/openurl
        _ => {
            argv.push(action.clone());
            // Attach the device id (or "booted") if the user didn't already
            // pass a UUID through args.
            argv.push(device.clone());
            if let Some(extra) = args.get("args").and_then(|v| v.as_array()) {
                argv.extend(extra.iter().filter_map(|x| x.as_str().map(String::from)));
            }
        }
    }

    let mut cmd = Command::new("xcrun");
    for a in &argv[0..1] {
        cmd.arg(a);
    }
    for a in &argv[1..] {
        cmd.arg(a);
    }

    let out = match cmd.output().await {
        Ok(o) => o,
        Err(e) => return format!("[xcode_simctl] could not run xcrun: {}", e),
    };

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    if !out.status.success() {
        return format!(
            "[xcode_simctl] action='{}' failed (exit {})\n--- stdout ---\n{}\n--- stderr ---\n{}",
            action,
            out.status.code().unwrap_or(-1),
            stdout.trim(),
            stderr.trim()
        );
    }

    // Truncate huge outputs from `list` to keep context manageable.
    let body = stdout.to_string();
    if body.len() > 32_000 {
        let mut s = body[..16_000].to_string();
        s.push_str("\n\n[... truncated ...]\n\n");
        s.push_str(&body[body.len() - 16_000..]);
        return s;
    }
    if body.is_empty() {
        format!("[xcode_simctl] action='{}' OK (no output)", action)
    } else {
        body
    }
}
