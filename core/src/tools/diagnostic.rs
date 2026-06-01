use serde_json::Value;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

const TIMEOUT_SECS: u64 = 120;

// ── Shared helper ──────────────────────────────────────────────────────────

/// Run a command, capturing combined stdout+stderr, with a 120s timeout.
async fn run_cmd(
    program: &str,
    args: &[&str],
    cwd: Option<&str>,
) -> Result<(String, bool), String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn '{}': {}", program, e))?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let collect: tokio::task::JoinHandle<(Vec<u8>, Vec<u8>)> = tokio::spawn(async move {
        let mut out = Vec::new();
        let mut err = Vec::new();
        if let Some(mut p) = stdout_pipe {
            let _ = p.read_to_end(&mut out).await;
        }
        if let Some(mut p) = stderr_pipe {
            let _ = p.read_to_end(&mut err).await;
        }
        (out, err)
    });

    match tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), child.wait()).await {
        Ok(Ok(status)) => {
            let (out, err) = collect.await.unwrap_or_default();
            let mut combined = String::from_utf8_lossy(&out).into_owned();
            let stderr_str = String::from_utf8_lossy(&err).into_owned();
            if !stderr_str.is_empty() {
                if !combined.is_empty() {
                    combined.push('\n');
                }
                combined.push_str(&stderr_str);
            }
            Ok((combined, status.success()))
        }
        Ok(Err(e)) => Err(format!("Process error: {}", e)),
        Err(_) => {
            collect.abort();
            Err(format!("Command timed out after {}s", TIMEOUT_SECS))
        }
    }
}

// ── 1. cargo_check ─────────────────────────────────────────────────────────

/// Tool name: "cargo_check"
/// Params: path (manifest dir, default "."), package (optional)
pub async fn cargo_check(args: &Value) -> String {
    let path = args["path"].as_str().unwrap_or(".");
    let package = args["package"].as_str();

    let mut cmd_args: Vec<&str> = vec!["check", "--message-format=short"];
    let pkg_flag;
    if let Some(pkg) = package {
        pkg_flag = pkg.to_string();
        cmd_args.push("--package");
        cmd_args.push(&pkg_flag);
    }

    let (output, success) = match run_cmd("cargo", &cmd_args, Some(path)).await {
        Ok(r) => r,
        Err(e) => {
            // Retry without --message-format=short in case cargo version doesn't support it
            let mut fallback_args: Vec<&str> = vec!["check"];
            if let Some(pkg) = package {
                fallback_args.push("--package");
                fallback_args.push(pkg);
            }
            match run_cmd("cargo", &fallback_args, Some(path)).await {
                Ok(r) => r,
                Err(_) => return format!("Error running cargo check: {}", e),
            }
        }
    };

    if success {
        // Count warnings
        let warning_count = output
            .lines()
            .filter(|l| l.contains("warning:") || l.contains("warning["))
            .count();
        format!("✓ cargo check passed ({} warnings)", warning_count)
    } else {
        // Extract error lines
        let errors: Vec<&str> = output
            .lines()
            .filter(|l| l.contains("error") || l.contains("warning"))
            .collect();

        if errors.is_empty() {
            crate::tools::truncate(format!("cargo check failed:\n{}", output), 5000)
        } else {
            let summary = errors.join("\n");
            crate::tools::truncate(format!("cargo check failed:\n{}", summary), 5000)
        }
    }
}

// ── 2. cargo_test ──────────────────────────────────────────────────────────

/// Tool name: "cargo_test"
/// Params: path, filter (test name filter), package
pub async fn cargo_test(args: &Value) -> String {
    let path = args["path"].as_str().unwrap_or(".");
    let filter = args["filter"].as_str();
    let package = args["package"].as_str();

    let mut cmd_args: Vec<String> = vec!["test".to_string()];

    if let Some(pkg) = package {
        cmd_args.push("--package".to_string());
        cmd_args.push(pkg.to_string());
    }

    if let Some(f) = filter {
        cmd_args.push(f.to_string());
    }

    cmd_args.push("--".to_string());
    cmd_args.push("--nocapture".to_string());

    let args_refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();

    let (output, success) = match run_cmd("cargo", &args_refs, Some(path)).await {
        Ok(r) => r,
        Err(e) => return format!("Error running cargo test: {}", e),
    };

    // Parse test results
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for line in output.lines() {
        if line.starts_with("test ") && line.ends_with(" ... ok") {
            passed += 1;
        } else if line.starts_with("test ") && line.ends_with(" ... FAILED") {
            failed += 1;
            failures.push(line.to_string());
        } else if line.contains("test result:") {
            // e.g. "test result: ok. 5 passed; 0 failed; ..."
            // already summarized below
        }
    }

    let summary = if success {
        format!("{} passed, {} failed", passed, failed)
    } else {
        let failure_detail = if failures.is_empty() {
            output.clone()
        } else {
            failures.join("\n")
        };
        format!(
            "{} passed, {} failed\nFailed tests:\n{}",
            passed, failed, failure_detail
        )
    };

    crate::tools::truncate(summary, 3000)
}

// ── 3. tsc_check ──────────────────────────────────────────────────────────

/// Tool name: "tsc_check"
/// Params: path (project dir), config (tsconfig path)
pub async fn tsc_check(args: &Value) -> String {
    let path = args["path"].as_str().unwrap_or(".");
    let config = args["config"].as_str();

    // Try `tsc` first, fall back to `npx tsc`
    let tsc_available = run_cmd("tsc", &["--version"], None)
        .await
        .map(|(_, ok)| ok)
        .unwrap_or(false);

    let program = if tsc_available { "tsc" } else { "npx" };

    let mut cmd_args: Vec<String> = if tsc_available {
        vec!["--noEmit".to_string()]
    } else {
        vec!["tsc".to_string(), "--noEmit".to_string()]
    };

    if let Some(cfg) = config {
        cmd_args.push("--project".to_string());
        cmd_args.push(cfg.to_string());
    }

    let args_refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();

    let (output, success) = match run_cmd(program, &args_refs, Some(path)).await {
        Ok(r) => r,
        Err(e) => return format!("Error running tsc: {}", e),
    };

    if success && output.trim().is_empty() {
        "✓ TypeScript check passed".to_string()
    } else if success {
        format!("✓ TypeScript check passed\n{}", output.trim())
    } else {
        let errors: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();

        if errors.is_empty() {
            "TypeScript check failed (no output)".to_string()
        } else {
            crate::tools::truncate(format!("TypeScript errors:\n{}", errors.join("\n")), 5000)
        }
    }
}

// ── 4. run_tests ──────────────────────────────────────────────────────────

/// Tool name: "run_tests"
/// Params: command (required), path (optional working dir)
pub async fn run_tests(args: &Value) -> String {
    let command = match args["command"].as_str() {
        Some(c) if !c.is_empty() => c,
        _ => return "Error: missing 'command' argument".to_string(),
    };
    let path = args["path"].as_str().unwrap_or(".");

    // Split into program + args using shell-like splitting
    let parts: Vec<&str> = command.splitn(2, ' ').collect();
    let program = parts[0];
    let rest: Vec<&str> = if parts.len() > 1 {
        parts[1].split_whitespace().collect()
    } else {
        vec![]
    };

    let (output, success) = match run_cmd(program, &rest, Some(path)).await {
        Ok(r) => r,
        Err(e) => return format!("Error running '{}': {}", command, e),
    };

    let status_line = if success {
        "Tests completed successfully."
    } else {
        "Tests finished with failures."
    };

    let body = if output.trim().is_empty() {
        "(no output)".to_string()
    } else {
        output
    };

    crate::tools::truncate(format!("{}\n{}", status_line, body), 5000)
}
