use phantom_mesh::tools::shell;
use serde_json::json;

// ── 1. Basic command executes successfully ────────────────────────────────

#[tokio::test]
async fn test_shell_echo() {
    let result = shell::run(&json!({"command": "echo hello"})).await;
    assert!(
        result.contains("hello"),
        "expected 'hello' in output, got: {result:?}"
    );
    assert!(
        result.contains("[exit code: 0]"),
        "expected '[exit code: 0]' in output, got: {result:?}"
    );
}

// ── 2. Exit code is captured in output ───────────────────────────────────

#[tokio::test]
async fn test_shell_exit_code_nonzero() {
    // `false` is a POSIX command guaranteed to exit 1
    let result = shell::run(&json!({"command": "false"})).await;
    assert!(
        result.contains("[exit code: 1]"),
        "expected '[exit code: 1]' in output, got: {result:?}"
    );
}

#[tokio::test]
async fn test_shell_exit_code_zero() {
    let result = shell::run(&json!({"command": "true"})).await;
    assert!(
        result.contains("[exit code: 0]"),
        "expected '[exit code: 0]' in output, got: {result:?}"
    );
}

// ── 3. stderr is captured ────────────────────────────────────────────────

#[tokio::test]
async fn test_shell_stderr_captured() {
    // sh -c writes "err" to stderr, nothing to stdout
    let result = shell::run(&json!({"command": "sh -c 'echo err >&2'"})).await;
    assert!(
        result.contains("STDERR:"),
        "expected 'STDERR:' prefix in output, got: {result:?}"
    );
    assert!(
        result.contains("err"),
        "expected stderr text 'err' in output, got: {result:?}"
    );
}

// ── 4. Blocklist rejects dangerous commands ──────────────────────────────

#[tokio::test]
async fn test_blocked_rm_rf_root() {
    let result = shell::run(&json!({"command": "rm -rf /"})).await;
    assert!(
        result.starts_with("Error: blocked"),
        "expected block error, got: {result:?}"
    );
}

#[tokio::test]
async fn test_blocked_rm_rf_home() {
    let result = shell::run(&json!({"command": "rm -rf ~"})).await;
    assert!(
        result.starts_with("Error: blocked"),
        "expected block error, got: {result:?}"
    );
}

#[tokio::test]
async fn test_blocked_fork_bomb() {
    let result = shell::run(&json!({"command": ":(){:|:&};:"})).await;
    assert!(
        result.starts_with("Error: blocked"),
        "expected block error for fork bomb, got: {result:?}"
    );
}

#[tokio::test]
async fn test_blocked_curl_pipe_sh() {
    // The blocklist pattern is the literal substring "curl | sh".
    // Use a command that contains that exact substring without requiring network access.
    let result = shell::run(&json!({"command": "echo curl | sh"})).await;
    assert!(
        result.starts_with("Error: blocked"),
        "expected block error for 'curl | sh' pattern, got: {result:?}"
    );
}

#[tokio::test]
async fn test_blocked_curl_pipe_sh_nospace() {
    // Blocklist pattern "curl|sh" — embed it literally in an echo command.
    let result = shell::run(&json!({"command": "echo curl|sh"})).await;
    assert!(
        result.starts_with("Error: blocked"),
        "expected block error for 'curl|sh' pattern (no spaces), got: {result:?}"
    );
}

#[tokio::test]
async fn test_blocked_wget_pipe_sh() {
    // Blocklist pattern "wget -O- | sh" — embed literally without needing wget installed.
    let result = shell::run(&json!({"command": "echo wget -O- | sh"})).await;
    assert!(
        result.starts_with("Error: blocked"),
        "expected block error for 'wget -O- | sh' pattern, got: {result:?}"
    );
}

#[tokio::test]
async fn test_blocked_sudo_rm() {
    let result = shell::run(&json!({"command": "sudo rm /etc/hosts"})).await;
    assert!(
        result.starts_with("Error: blocked"),
        "expected block error for sudo rm, got: {result:?}"
    );
}

#[tokio::test]
async fn test_blocked_mkfs() {
    let result = shell::run(&json!({"command": "mkfs /dev/sda1"})).await;
    assert!(
        result.starts_with("Error: blocked"),
        "expected block error for mkfs, got: {result:?}"
    );
}

#[tokio::test]
async fn test_blocked_dd_devzero() {
    let result = shell::run(&json!({"command": "dd if=/dev/zero of=/dev/sda"})).await;
    assert!(
        result.starts_with("Error: blocked"),
        "expected block error for dd if=/dev/zero of=/dev/, got: {result:?}"
    );
}

// ── 5. Compound commands work ─────────────────────────────────────────────

#[tokio::test]
async fn test_compound_command_and_and() {
    let result = shell::run(&json!({"command": "echo a && echo b"})).await;
    assert!(
        result.contains('a'),
        "expected 'a' in compound output, got: {result:?}"
    );
    assert!(
        result.contains('b'),
        "expected 'b' in compound output, got: {result:?}"
    );
}

#[tokio::test]
async fn test_compound_command_semicolon() {
    let result = shell::run(&json!({"command": "echo first; echo second"})).await;
    assert!(
        result.contains("first"),
        "expected 'first' in output, got: {result:?}"
    );
    assert!(
        result.contains("second"),
        "expected 'second' in output, got: {result:?}"
    );
}

// ── 6. Timeout is respected ───────────────────────────────────────────────

#[tokio::test]
async fn test_timeout_respected() {
    let result = shell::run(&json!({"command": "sleep 10", "timeout_secs": 1})).await;
    assert!(
        result.contains("timed out"),
        "expected timeout message, got: {result:?}"
    );
    // Implementation emits [exit code: -1] on timeout
    assert!(
        result.contains("[exit code: -1]"),
        "expected '[exit code: -1]' in timeout output, got: {result:?}"
    );
}

// ── 7. Missing command argument ───────────────────────────────────────────

#[tokio::test]
async fn test_missing_command_key() {
    // No "command" key at all
    let result = shell::run(&json!({})).await;
    assert!(
        result.starts_with("Error: missing"),
        "expected missing-argument error, got: {result:?}"
    );
}

#[tokio::test]
async fn test_missing_command_null() {
    // "command" key present but null
    let result = shell::run(&json!({"command": null})).await;
    assert!(
        result.starts_with("Error: missing"),
        "expected missing-argument error for null command, got: {result:?}"
    );
}

// ── 8. Output truncation ──────────────────────────────────────────────────

#[tokio::test]
async fn test_output_truncated() {
    // Generate >20 000 chars with a single command (no pipes/operators so it uses
    // the direct-spawn path that calls truncate()).  python3 is always available on macOS.
    let result = shell::run(&json!({"command": "python3 -c \"print('a' * 25000)\""})).await;
    assert!(
        result.contains("truncated") || result.contains("omitted"),
        "expected truncation notice in long output, got first 200 chars: {:?}",
        &result[..result.len().min(200)]
    );
}

// ── Bonus: empty command string ───────────────────────────────────────────

#[tokio::test]
async fn test_empty_command_string() {
    // An empty string parses to an empty argv vector
    let result = shell::run(&json!({"command": ""})).await;
    assert!(
        result.starts_with("Error:"),
        "expected an error for an empty command, got: {result:?}"
    );
}

// ── Security: bash -c bypass prevention ──────────────────────────────────

#[tokio::test]
async fn test_blocked_bash_c_rm() {
    let result = shell::run(&json!({"command": "bash -c 'rm -rf /'"})).await;
    assert!(
        result.starts_with("Error:"),
        "bash -c rm should be blocked: {result:?}"
    );
}

#[tokio::test]
async fn test_blocked_sh_c_sudo_rm() {
    let result = shell::run(&json!({"command": "sh -c 'sudo rm -rf /tmp'"})).await;
    assert!(
        result.starts_with("Error:"),
        "sh -c sudo rm should be blocked: {result:?}"
    );
}

#[tokio::test]
async fn test_bash_c_safe_allowed() {
    let result = shell::run(&json!({"command": "bash -c 'echo hello'"})).await;
    assert!(
        result.contains("hello"),
        "safe bash -c should be allowed: {result:?}"
    );
}

// ── Security: shell quoting bypass prevention ─────────────────────────────

#[tokio::test]
async fn test_blocked_quoted_rm_rf() {
    // Quoting shouldn't bypass blocklist — post-parse check catches this
    let result = shell::run(&json!({"command": "rm '-rf' '/'"})).await;
    assert!(
        result.starts_with("Error:"),
        "quoted rm -rf / should be blocked: {result:?}"
    );
}

// ── Bonus: stdout and stderr both present ─────────────────────────────────

#[tokio::test]
async fn test_stdout_and_stderr_combined() {
    // Use python3 to write to both stdout and stderr in a single command,
    // avoiding semicolons/&& that would trigger the compound-command path.
    // Write the script to a temp file so the command string contains no
    // semicolons or "&&" — otherwise the compound-command splitter shreds it.
    use std::io::Write as _;
    let mut f = tempfile::NamedTempFile::new().expect("tempfile");
    f.write_all(b"import sys\nprint('out')\nprint('err', file=sys.stderr)\n")
        .expect("write script");
    let path = f.path().to_str().unwrap().to_owned();
    let result = shell::run(&json!({"command": format!("python3 {path}")})).await;
    assert!(
        result.contains("out"),
        "expected stdout in output, got: {result:?}"
    );
    assert!(
        result.contains("STDERR:"),
        "expected STDERR: prefix, got: {result:?}"
    );
    assert!(
        result.contains("err"),
        "expected stderr text, got: {result:?}"
    );
}
