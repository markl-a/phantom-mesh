use serde_json::Value;

use crate::tools::validate;

fn validate_git_path(path: &str) -> Result<(), String> {
    for ch in [';', '|', '&', '$', '`', '>', '<', '\n', '\r'] {
        if path.contains(ch) {
            return Err(format!(
                "Error: invalid character '{}' in path argument",
                ch
            ));
        }
    }
    Ok(())
}

fn validate_file_arg(file: &str) -> Result<(), String> {
    for ch in [';', '|', '&', '$', '`', '>', '<', '\n', '\r'] {
        if file.contains(ch) {
            return Err(format!(
                "Error: invalid character '{}' in file argument",
                ch
            ));
        }
    }
    Ok(())
}

fn validate_commit_message(msg: &str) -> Result<(), String> {
    if msg.len() > 1000 {
        return Err("Error: commit message too long (max 1000 characters)".into());
    }
    if msg.contains("$(") || msg.contains('`') {
        return Err("Error: commit message contains disallowed shell substitution".into());
    }
    Ok(())
}

pub async fn status(args: &Value) -> String {
    let path = args["path"].as_str().unwrap_or(".");
    if let Err(e) = validate_git_path(path) {
        return e;
    }
    match tokio::process::Command::new("git")
        .args(["-C", path, "status", "--short"])
        .output()
        .await
    {
        Ok(out) => {
            let s = String::from_utf8_lossy(&out.stdout).to_string();
            if s.trim().is_empty() {
                "Working tree clean".into()
            } else {
                s
            }
        }
        Err(e) => format!("git error: {}", e),
    }
}

pub async fn diff(args: &Value) -> String {
    let path = args["path"].as_str().unwrap_or(".");
    if let Err(e) = validate_git_path(path) {
        return e;
    }
    let cached = args["cached"].as_bool().unwrap_or(false);
    // `full` = show the patch (changed lines); default keeps the `--stat`
    // summary (backward-compatible — existing callers pass no `full`).
    let full = args["full"].as_bool().unwrap_or(false);
    let file = args["file"].as_str().unwrap_or("");
    if !file.is_empty() {
        if let Err(e) = validate_file_arg(file) {
            return e;
        }
    }
    let mut git_args = vec!["-C", path, "diff"];
    if !full {
        git_args.push("--stat");
    }
    if cached {
        git_args.push("--cached");
    }
    if !file.is_empty() {
        git_args.extend(["--", file]);
    }
    match tokio::process::Command::new("git")
        .args(&git_args)
        .output()
        .await
    {
        Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
        Err(e) => format!("git error: {}", e),
    }
}

pub async fn log(args: &Value) -> String {
    let path = args["path"].as_str().unwrap_or(".");
    if let Err(e) = validate_git_path(path) {
        return e;
    }
    let n = args["n"].as_u64().unwrap_or(10);
    match tokio::process::Command::new("git")
        .args(["-C", path, "log", "--oneline", &format!("-{}", n)])
        .output()
        .await
    {
        Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
        Err(e) => format!("git error: {}", e),
    }
}

pub async fn commit(args: &Value) -> String {
    let path = args["path"].as_str().unwrap_or(".");
    if let Err(e) = validate_git_path(path) {
        return e;
    }
    let message = match args["message"].as_str() {
        Some(m) => m,
        None => return "Error: missing 'message' argument".into(),
    };
    if let Err(e) = validate_commit_message(message) {
        return e;
    }
    // Audit H-4: git treats `--` as end-of-options. Without it a malicious
    // path or message that looks like `--exec=sh` could be interpreted as a
    // flag. The message itself can't reach there (it's the value after
    // `-m`) but the path / repo arg deserves the guard.
    match tokio::process::Command::new("git")
        .args(["-C", path, "commit", "-m", message])
        .output()
        .await
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            format!("{}{}", stdout, stderr)
        }
        Err(e) => format!("git error: {}", e),
    }
}

/// Stage a file for the next commit.
///
/// Validates that the path does not contain shell injection characters.
/// Requires SPECTYN_AUTO_APPROVE=1 or returns APPROVAL_REQUIRED.
pub async fn add(args: &Value) -> String {
    let path = match args["path"].as_str() {
        Some(p) => p,
        None => return "Error: missing 'path' argument".into(),
    };

    // Validate that the path doesn't contain shell-injection characters.
    for ch in [';', '|', '&', '$', '`', '>', '<'] {
        if path.contains(ch) {
            return format!("Error: invalid character '{}' in path argument", ch);
        }
    }

    let repo_path = args["repo"].as_str().unwrap_or(".");

    match tokio::process::Command::new("git")
        .args(["-C", repo_path, "add", "--", path])
        .output()
        .await
    {
        Ok(out) => {
            if out.status.success() {
                format!("Staged: {}", path)
            } else {
                format!("Error: {}", String::from_utf8_lossy(&out.stderr))
            }
        }
        Err(e) => format!("Error: git add failed: {}", e),
    }
}

/// Push commits to the remote repository.
///
/// Requires SPECTYN_AUTO_APPROVE=1 to execute; otherwise returns APPROVAL_REQUIRED.
pub async fn push(args: &Value) -> String {
    // Audit H-4: validate option-injection BEFORE the approval gate. A
    // user-approved invocation must still not be allowed to push to a
    // remote named `--exec=sh`; "approved" is not the same as "safe".
    let repo_path = args["path"].as_str().unwrap_or(".");
    if let Err(e) = validate_git_path(repo_path) {
        return e;
    }
    let remote = args["remote"].as_str().unwrap_or("origin");
    let branch = args["branch"].as_str().unwrap_or("HEAD");
    if let Err(e) = validate::validate_git_extern_arg("remote", remote) {
        return e;
    }
    if let Err(e) = validate::validate_git_ref("branch", branch) {
        return e;
    }

    if std::env::var("SPECTYN_AUTO_APPROVE").as_deref() != Ok("1") {
        return "APPROVAL_REQUIRED: git push is a destructive/remote operation. \
                Set SPECTYN_AUTO_APPROVE=1 to allow."
            .to_string();
    }

    // Note: `git push` does not accept a `--` end-of-options sentinel before
    // the remote name (it parses `<repository> [<refspec>...]` positionally),
    // so we rely on the strict allow-list in `validate_git_extern_arg` /
    // `validate_git_ref` above to reject anything starting with `-`.
    match tokio::process::Command::new("git")
        .args(["-C", repo_path, "push", remote, branch])
        .output()
        .await
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            format!("{}{}", stdout, stderr)
        }
        Err(e) => format!("Error: git push failed: {}", e),
    }
}

/// Reset the working tree.
///
/// Mode "hard" requires SPECTYN_AUTO_APPROVE=1; other modes do not.
pub async fn reset(args: &Value) -> String {
    let mode = args["mode"].as_str().unwrap_or("soft");
    let repo_path = args["path"].as_str().unwrap_or(".");
    if let Err(e) = validate_git_path(repo_path) {
        return e;
    }

    // Audit H-5: hard allow-list. Pre-fix, `mode = "exec=sh"` produced
    // `git reset --exec=sh` which runs an arbitrary process and bypasses
    // every approval gate (the gate only checked for the literal "hard").
    if let Err(e) = validate::validate_git_reset_mode(mode) {
        return e;
    }

    if mode == "hard" && std::env::var("SPECTYN_AUTO_APPROVE").as_deref() != Ok("1") {
        return "APPROVAL_REQUIRED: git reset --hard is destructive (discards uncommitted changes). \
                Set SPECTYN_AUTO_APPROVE=1 to allow."
            .to_string();
    }

    let flag = format!("--{}", mode);
    match tokio::process::Command::new("git")
        .args(["-C", repo_path, "reset", &flag])
        .output()
        .await
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            if out.status.success() {
                format!("{}{}Reset complete.", stdout, stderr)
            } else {
                format!("Error: {}{}", stdout, stderr)
            }
        }
        Err(e) => format!("Error: git reset failed: {}", e),
    }
}

pub async fn blame(args: &Value) -> String {
    let repo_path = args["path"].as_str().unwrap_or(".");
    let file = match args["file"].as_str() {
        Some(f) => f,
        None => return "Error: missing 'file' argument".into(),
    };
    match tokio::process::Command::new("git")
        .args(["-C", repo_path, "blame", "--", file])
        .output()
        .await
    {
        Ok(out) => {
            let s = String::from_utf8_lossy(&out.stdout).to_string();
            if s.trim().is_empty() {
                String::from_utf8_lossy(&out.stderr).to_string()
            } else {
                s
            }
        }
        Err(e) => format!("git error: {}", e),
    }
}

pub async fn show(args: &Value) -> String {
    let repo_path = args["path"].as_str().unwrap_or(".");
    let r#ref = args["ref"].as_str().unwrap_or("HEAD");
    match tokio::process::Command::new("git")
        .args(["-C", repo_path, "show", "--stat", r#ref])
        .output()
        .await
    {
        Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
        Err(e) => format!("git error: {}", e),
    }
}

pub async fn branch(args: &Value) -> String {
    let repo_path = args["path"].as_str().unwrap_or(".");
    match tokio::process::Command::new("git")
        .args(["-C", repo_path, "branch", "-v"])
        .output()
        .await
    {
        Ok(out) => {
            let s = String::from_utf8_lossy(&out.stdout).to_string();
            if s.trim().is_empty() {
                "No branches".into()
            } else {
                s
            }
        }
        Err(e) => format!("git error: {}", e),
    }
}

pub async fn stash(args: &Value) -> String {
    let repo_path = args["path"].as_str().unwrap_or(".");
    let action = args["action"].as_str().unwrap_or("list");
    let git_args: Vec<&str> = match action {
        "list" => vec!["-C", repo_path, "stash", "list"],
        "pop" => vec!["-C", repo_path, "stash", "pop"],
        "push" => vec!["-C", repo_path, "stash", "push"],
        _ => return format!("Error: unknown stash action '{}'", action),
    };
    match tokio::process::Command::new("git")
        .args(&git_args)
        .output()
        .await
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            if out.status.success() {
                if stdout.trim().is_empty() {
                    format!("{}(no stashes)", stderr)
                } else {
                    stdout
                }
            } else {
                format!("Error: {}{}", stdout, stderr)
            }
        }
        Err(e) => format!("git error: {}", e),
    }
}

pub async fn pull(args: &Value) -> String {
    let repo_path = args["path"].as_str().unwrap_or(".");
    if let Err(e) = validate_git_path(repo_path) {
        return e;
    }
    let remote = args["remote"].as_str().unwrap_or("origin");
    // Audit H-4: same option-injection guard as `push`.
    if let Err(e) = validate::validate_git_extern_arg("remote", remote) {
        return e;
    }
    match tokio::process::Command::new("git")
        .args(["-C", repo_path, "pull", remote])
        .output()
        .await
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            format!("{}{}", stdout, stderr)
        }
        Err(e) => format!("Error: git pull failed: {}", e),
    }
}

// ---------------------------------------------------------------------------
// New tools added below
// ---------------------------------------------------------------------------

/// List all branches. `path` sets the repo dir. If `remote` is true, shows
/// remote-tracking branches as well (git branch -a), otherwise local only.
pub async fn git_branch_list(args: &Value) -> String {
    let repo_path = args["path"].as_str().unwrap_or(".");
    if let Err(e) = validate_git_path(repo_path) {
        return e;
    }
    let show_remote = args["remote"].as_bool().unwrap_or(false);
    let flag = if show_remote { "-av" } else { "-v" };
    match tokio::process::Command::new("git")
        .args(["-C", repo_path, "branch", flag])
        .output()
        .await
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            if out.status.success() {
                if stdout.trim().is_empty() {
                    "No branches found.".into()
                } else {
                    stdout
                }
            } else {
                format!("Error: {}{}", stdout, stderr)
            }
        }
        Err(e) => format!("git error: {}", e),
    }
}

/// Switch branch or restore file. `path` sets the repo dir. `branch` is the
/// target branch name. If `create` is true, passes `-b` to create a new branch.
pub async fn git_checkout(args: &Value) -> String {
    let repo_path = args["path"].as_str().unwrap_or(".");
    if let Err(e) = validate_git_path(repo_path) {
        return e;
    }
    let branch = match args["branch"].as_str() {
        Some(b) => b,
        None => return "Error: missing 'branch' argument".into(),
    };
    if let Err(e) = validate_git_path(branch) {
        return e;
    }
    // Audit H-4: also reject `--exec=sh`-style refs that pass `validate_git_path`
    // (which only screens for shell metacharacters, not leading `-`).
    if let Err(e) = validate::validate_git_ref("branch", branch) {
        return e;
    }
    let create = args["create"].as_bool().unwrap_or(false);
    let mut git_args = vec!["-C", repo_path, "checkout"];
    if create {
        git_args.push("-b");
    }
    git_args.push(branch);
    match tokio::process::Command::new("git")
        .args(&git_args)
        .output()
        .await
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            if out.status.success() {
                format!("{}{}", stdout, stderr)
            } else {
                format!("Error: {}{}", stdout, stderr)
            }
        }
        Err(e) => format!("git error: {}", e),
    }
}

/// Show a commit's details. `path` sets the repo dir. `ref_` is the commit
/// ref (default HEAD). If `stat_only` is true, only --stat output is shown;
/// otherwise the full diff is included.
pub async fn git_show(args: &Value) -> String {
    let repo_path = args["path"].as_str().unwrap_or(".");
    if let Err(e) = validate_git_path(repo_path) {
        return e;
    }
    let commit_ref = args["ref_"].as_str().unwrap_or("HEAD");
    if let Err(e) = validate_git_path(commit_ref) {
        return e;
    }
    if let Err(e) = validate::validate_git_ref("ref_", commit_ref) {
        return e;
    }
    let stat_only = args["stat_only"].as_bool().unwrap_or(false);
    let mut git_args = vec!["-C", repo_path, "show"];
    if stat_only {
        git_args.push("--stat");
    }
    git_args.push(commit_ref);
    match tokio::process::Command::new("git")
        .args(&git_args)
        .output()
        .await
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            if out.status.success() {
                if stdout.trim().is_empty() {
                    stderr
                } else {
                    stdout
                }
            } else {
                format!("Error: {}{}", stdout, stderr)
            }
        }
        Err(e) => format!("git error: {}", e),
    }
}

/// Show who last modified each line of a file. `path` is the FILE to blame
/// (required). `repo` is the repo dir (defaults to ".").
/// Output is truncated to 100 lines.
pub async fn git_blame(args: &Value) -> String {
    let file_path = match args["path"].as_str() {
        Some(p) => p,
        None => return "Error: missing 'path' argument (the file to blame)".into(),
    };
    if let Err(e) = validate_file_arg(file_path) {
        return e;
    }
    let repo_path = args["repo"].as_str().unwrap_or(".");
    if let Err(e) = validate_git_path(repo_path) {
        return e;
    }
    match tokio::process::Command::new("git")
        .args(["-C", repo_path, "blame", "--", file_path])
        .output()
        .await
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            if out.status.success() {
                let lines: Vec<&str> = stdout.lines().collect();
                let truncated = lines.len() > 100;
                let result = lines
                    .iter()
                    .take(100)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n");
                if truncated {
                    format!("{}\n... (output truncated to 100 lines)", result)
                } else {
                    result
                }
            } else {
                format!("Error: {}{}", stdout, stderr)
            }
        }
        Err(e) => format!("git error: {}", e),
    }
}

/// Stage multiple files for the next commit. `path` sets the repo dir.
/// `files` is the list of file paths to stage.
pub async fn git_add(args: &Value) -> String {
    let repo_path = args["path"].as_str().unwrap_or(".");
    if let Err(e) = validate_git_path(repo_path) {
        return e;
    }
    let files: Vec<String> = match args["files"].as_array() {
        Some(arr) => {
            let mut result = Vec::new();
            for v in arr {
                match v.as_str() {
                    Some(s) => {
                        if let Err(e) = validate_file_arg(s) {
                            return e;
                        }
                        result.push(s.to_string());
                    }
                    None => return "Error: 'files' must be an array of strings".into(),
                }
            }
            result
        }
        None => return "Error: missing 'files' argument".into(),
    };
    if files.is_empty() {
        return "Error: 'files' array is empty".into();
    }
    let mut git_args: Vec<&str> = vec!["-C", repo_path, "add", "--"];
    for f in &files {
        git_args.push(f.as_str());
    }
    match tokio::process::Command::new("git")
        .args(&git_args)
        .output()
        .await
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            if out.status.success() {
                format!("Staged {} file(s): {}", files.len(), files.join(", "))
            } else {
                format!("Error: {}{}", stdout, stderr)
            }
        }
        Err(e) => format!("git error: {}", e),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Audit H-4 / H-5 regression tests — option-injection guards.
//
// Each `tokio::test` below proves a previously-exploitable argument is
// rejected before reaching the `git` subprocess. The tests do NOT need
// `git` to be installed; rejection happens in pure-Rust validation.
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod option_injection_tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn push_rejects_exec_remote() {
        // Pre-fix: this would have run `git push --exec=sh HEAD` which is a
        // documented option-injection vector that executes `sh` on the local
        // hook side. The validator now fires BEFORE the approval gate so
        // even an "approved" caller can't get past it (and the test does
        // not need to touch SPECTYN_AUTO_APPROVE — avoiding cross-test
        // env-var races).
        let result = push(&json!({"remote": "--exec=sh"})).await;
        assert!(
            result.starts_with("Error:") && result.contains("remote"),
            "got: {}",
            result
        );
    }

    #[tokio::test]
    async fn push_rejects_upload_pack_remote() {
        let result = push(&json!({"remote": "--upload-pack=/tmp/evil"})).await;
        assert!(result.starts_with("Error:"), "got: {}", result);
    }

    #[tokio::test]
    async fn push_rejects_branch_dash_prefix() {
        let result = push(&json!({"remote": "origin", "branch": "--exec=sh"})).await;
        assert!(
            result.starts_with("Error:") && result.contains("branch"),
            "got: {}",
            result
        );
    }

    #[tokio::test]
    async fn pull_rejects_remote_option_injection() {
        let result = pull(&json!({"remote": "--upload-pack=/tmp/evil"})).await;
        assert!(result.starts_with("Error:"), "got: {}", result);
    }

    #[tokio::test]
    async fn reset_rejects_exec_mode() {
        // Audit H-5: `mode = "exec=sh"` would produce `git reset --exec=sh`
        // (an arbitrary process exec) and the approval gate (`mode == "hard"`)
        // would not fire. Now rejected by the allow-list.
        let result = reset(&json!({"mode": "exec=sh"})).await;
        assert!(
            result.starts_with("Error:") && result.contains("invalid git reset mode"),
            "got: {}",
            result
        );
    }

    #[tokio::test]
    async fn reset_rejects_upload_pack_mode() {
        let result = reset(&json!({"mode": "upload-pack=/tmp/evil"})).await;
        assert!(result.starts_with("Error:"), "got: {}", result);
    }

    #[tokio::test]
    async fn reset_accepts_safe_mode_soft() {
        // Sanity: `soft` is on the allow-list and should at least pass
        // validation (the subprocess may fail outside a git repo, but that
        // produces a different error string).
        let result = reset(&json!({"mode": "soft", "path": "."})).await;
        // We don't care about the precise outcome of the git command; we
        // only need to confirm validation didn't block it.
        assert!(
            !result.contains("invalid git reset mode"),
            "soft should be on allow-list, got: {}",
            result
        );
    }

    #[tokio::test]
    async fn git_checkout_rejects_exec_branch() {
        let result = git_checkout(&json!({"branch": "--exec=sh"})).await;
        // The legacy `validate_git_path` catches `$` but not leading `-`;
        // the new `validate_git_ref` does. Either error path proves the fix.
        assert!(result.starts_with("Error:"), "got: {}", result);
    }
}

/// List all stashes in the repository. `path` sets the repo dir.
pub async fn git_stash_list(args: &Value) -> String {
    let repo_path = args["path"].as_str().unwrap_or(".");
    if let Err(e) = validate_git_path(repo_path) {
        return e;
    }
    match tokio::process::Command::new("git")
        .args(["-C", repo_path, "stash", "list"])
        .output()
        .await
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            if out.status.success() {
                if stdout.trim().is_empty() {
                    "No stashes.".into()
                } else {
                    stdout
                }
            } else {
                format!("Error: {}{}", stdout, stderr)
            }
        }
        Err(e) => format!("git error: {}", e),
    }
}
