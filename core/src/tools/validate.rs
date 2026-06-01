//! Shared input-validation helpers used by `shell.rs`, `bash_bg.rs`, `git.rs`,
//! and `search.rs` to defend against the [T7d] audit findings:
//!
//! - **C-1** (CRITICAL) — `shell.rs` blocklist bypass via Unicode lookalikes /
//!   multi-space / native-shell routing.
//! - **H-4 / H-5** (HIGH) — git option-injection through `remote` / `branch` /
//!   `--exec=…` and `git reset --<arbitrary>`.
//! - **M-8** (HIGH-impact MEDIUM) — `rg` / `grep` option-injection via a
//!   user-supplied pattern that begins with `-` (`--pre=…` runs a preprocessor).
//!
//! Goals:
//!
//! 1. Centralise the blocklist + normalisation logic so `shell::run`,
//!    `shell::run_via_native_shell`, `shell::run_compound`, `shell::run_bg`,
//!    and `bash_bg::run_background` all enforce the **same** policy. Before
//!    this module, only `shell::run` did the substring check; the other
//!    paths silently bypassed the gate.
//! 2. Defang the obvious Unicode bypasses called out in the audit
//!    (`rm\u{2010}rf /` Unicode hyphen, `rm\u{00A0}-rf` non-breaking space,
//!    `rm  -rf  /` multi-space, `rm\t-rf\t/` tab). We don't pull in a full
//!    NFKC normaliser — instead we map the specific whitespace + dash
//!    classes the blocklist needs to recognise into ASCII before matching,
//!    and reject control characters / zero-width joiners outright (no
//!    legitimate single-tool call needs them).
//! 3. Provide allow-list / deny-list helpers for git remote-name + reset-mode
//!    + grep pattern arguments, all of which would otherwise reach `git` /
//!    `rg` as positional flags via the lone `-` prefix trick.

/// Strict allow-list for `git reset --<mode>`. Anything else (including
/// `exec=sh`, `pre=…`, `upload-pack=…`) is rejected outright — `git reset
/// --exec=sh` is a documented option-injection vector.
pub const GIT_RESET_MODES: &[&str] = &["soft", "mixed", "hard", "keep", "merge"];

/// Maximum length of a single command string before we refuse to even
/// normalise it. A 1 MiB command line is a sign of either a model loop or
/// an injection attempt; legitimate single tool calls fit in 16 KiB.
const MAX_CMD_LEN: usize = 65_536;

/// Reject bytes/chars that no legitimate single-tool call needs:
/// - C0 control chars (`\x00`-`\x1F`) **except** `\t` and `\n`/`\r` which we
///   handle separately during normalisation.
/// - C1 control chars (`\x7F`-`\x9F`).
/// - Zero-width joiners and bidi-override characters (`\u{200B}`-`\u{200F}`,
///   `\u{202A}`-`\u{202E}`, `\u{2060}`-`\u{206F}`, `\u{FEFF}`).
///
/// `\n` and `\r` are also rejected — multi-line commands are a known evasion
/// vector for substring blocklists (`rm \<newline>-rf /`) and no single
/// `shell` tool call should be carrying script text. Callers who genuinely
/// need a multi-line script should write it to a file via `file_write` and
/// invoke `bash <path>` instead.
pub fn reject_dangerous_chars(cmd: &str) -> Result<(), String> {
    if cmd.len() > MAX_CMD_LEN {
        return Err(format!(
            "Error: command too long ({} bytes, max {})",
            cmd.len(),
            MAX_CMD_LEN
        ));
    }
    for ch in cmd.chars() {
        let code = ch as u32;
        // C0 controls except tab.
        if code < 0x20 && code != 0x09 {
            return Err(format!(
                "Error: command contains disallowed control character (U+{:04X})",
                code
            ));
        }
        // C1 controls.
        if (0x7F..=0x9F).contains(&code) {
            return Err(format!(
                "Error: command contains disallowed control character (U+{:04X})",
                code
            ));
        }
        // Zero-width / bidi-override / BOM.
        if matches!(
            code,
            0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0xFEFF
        ) {
            return Err(format!(
                "Error: command contains disallowed zero-width / bidi character (U+{:04X})",
                code
            ));
        }
    }
    Ok(())
}

/// Normalise a command string so substring blocklist checks see a canonical
/// form regardless of which Unicode lookalike or whitespace class the caller
/// used. Specifically:
///
/// - Map every Unicode whitespace + Unicode dash variant called out in the
///   audit to the ASCII equivalent (`-` for dashes, ` ` for spaces).
/// - Collapse runs of whitespace to single spaces so `rm  -rf  /`
///   (double-space) matches `rm -rf /`.
/// - Lowercase ASCII (so SQL keywords like `DROP TABLE` match `drop table`).
///
/// **Returns the normalised string for matching only.** The original command
/// is still what gets executed — this is purely the form the blocklist
/// inspects.
pub fn normalise_for_blocklist(cmd: &str) -> String {
    let mut out = String::with_capacity(cmd.len());
    let mut last_was_space = false;

    for ch in cmd.chars() {
        let mapped = match ch {
            // Whitespace family — every variant collapses to ASCII space.
            ' '
            | '\t'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}' => ' ',
            // Dash / hyphen family — every variant collapses to ASCII '-'.
            '\u{2010}'..='\u{2015}' | '\u{2212}' | '\u{FE58}' | '\u{FE63}' | '\u{FF0D}' => '-',
            // ASCII-uppercase → lowercase for case-insensitive blocklist.
            c if c.is_ascii_uppercase() => c.to_ascii_lowercase(),
            other => other,
        };

        if mapped == ' ' {
            if last_was_space {
                continue;
            }
            last_was_space = true;
        } else {
            last_was_space = false;
        }
        out.push(mapped);
    }

    out.trim().to_string()
}

/// Returns `Some(pattern)` if `cmd` matches a deny-listed shell pattern
/// after normalisation. Patterns are themselves expected to be in
/// already-normalised form (lowercase, single-space-separated).
pub fn match_blocklist<'a>(cmd: &str, patterns: &'a [&'a str]) -> Option<&'a str> {
    let normalised = normalise_for_blocklist(cmd);
    for pat in patterns {
        if normalised.contains(pat) {
            return Some(pat);
        }
    }
    None
}

/// Validate a git "external" argument (remote name, branch name, refspec)
/// against option-injection. Per audit H-4/H-5, an unvalidated `remote`
/// like `--upload-pack=/tmp/evil` or `--exec=sh` is interpreted by git as
/// a flag and gives the model arbitrary process execution.
///
/// Rules:
/// - Must not be empty.
/// - Must not start with `-` (option-injection canary).
/// - Must not contain shell metacharacters (`;|&$\`<>` plus `\n`, `\r`).
/// - Must not contain whitespace other than nothing (a remote name with
///   spaces is never legitimate).
pub fn validate_git_extern_arg(name: &'static str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("Error: git {} argument is empty", name));
    }
    if value.starts_with('-') {
        return Err(format!(
            "Error: git {} argument may not start with '-' (option-injection guard): {:?}",
            name, value
        ));
    }
    for ch in value.chars() {
        if matches!(ch, ';' | '|' | '&' | '$' | '`' | '<' | '>' | '\n' | '\r') {
            return Err(format!(
                "Error: git {} argument contains disallowed character {:?}",
                name, ch
            ));
        }
        if ch.is_whitespace() {
            return Err(format!(
                "Error: git {} argument may not contain whitespace",
                name
            ));
        }
    }
    Ok(())
}

/// Validate a git refspec (branch, tag, commit ref). Like
/// [`validate_git_extern_arg`] but also allows the standard refspec
/// characters like `/`, `.`, `@`, `~`, `^`, `:`. Still rejects leading `-`.
pub fn validate_git_ref(name: &'static str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("Error: git {} argument is empty", name));
    }
    if value.starts_with('-') {
        return Err(format!(
            "Error: git {} argument may not start with '-' (option-injection guard): {:?}",
            name, value
        ));
    }
    for ch in value.chars() {
        if matches!(ch, ';' | '|' | '&' | '$' | '`' | '<' | '>' | '\n' | '\r') {
            return Err(format!(
                "Error: git {} argument contains disallowed character {:?}",
                name, ch
            ));
        }
        if ch.is_whitespace() {
            return Err(format!(
                "Error: git {} argument may not contain whitespace",
                name
            ));
        }
    }
    Ok(())
}

/// Validate that `mode` is one of the safe `git reset --<mode>` values.
/// Per audit H-5, `mode = "exec=sh"` becomes `git reset --exec=sh`, a valid
/// option that runs an arbitrary process — only the literal allow-list is safe.
pub fn validate_git_reset_mode(mode: &str) -> Result<(), String> {
    if !GIT_RESET_MODES.contains(&mode) {
        return Err(format!(
            "Error: invalid git reset mode {:?}; allowed: {:?}",
            mode, GIT_RESET_MODES
        ));
    }
    Ok(())
}

/// Validate a `rg` / `grep` pattern argument against option-injection
/// (audit M-8). `rg` accepts options anywhere on the command line, so a
/// pattern like `--pre=/bin/sh` becomes a flag that specifies a preprocessor
/// program.
///
/// Mitigation: callers MUST insert a literal `--` separator before the
/// pattern in the argv list. This helper additionally rejects patterns that
/// start with `-` to give a clear error message rather than silently
/// truncating the search at the `--` separator.
pub fn validate_search_pattern(pattern: &str) -> Result<(), String> {
    if pattern.is_empty() {
        return Err("Error: search pattern is empty".into());
    }
    if pattern.starts_with('-') {
        return Err(
            "Error: search pattern may not start with '-' (option-injection guard); \
             escape with backslash if you really need to match a literal hyphen"
                .into(),
        );
    }
    for ch in pattern.chars() {
        let code = ch as u32;
        if code < 0x20 && code != 0x09 {
            return Err(format!(
                "Error: search pattern contains disallowed control character (U+{:04X})",
                code
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── reject_dangerous_chars ────────────────────────────────────────────

    #[test]
    fn rejects_newline() {
        let err = reject_dangerous_chars("rm \n-rf /").unwrap_err();
        assert!(err.contains("control character"), "got: {}", err);
    }

    #[test]
    fn rejects_carriage_return() {
        assert!(reject_dangerous_chars("rm \r-rf /").is_err());
    }

    #[test]
    fn rejects_zero_width_joiner() {
        assert!(reject_dangerous_chars("rm\u{200B} -rf /").is_err());
    }

    #[test]
    fn allows_tab() {
        // Tab is legitimate (some tool args contain it). Normalisation maps it
        // to space for blocklist checks; we don't reject it here.
        assert!(reject_dangerous_chars("echo a\tb").is_ok());
    }

    #[test]
    fn allows_normal_ascii() {
        assert!(reject_dangerous_chars("ls -la /tmp").is_ok());
    }

    // ── normalise_for_blocklist ───────────────────────────────────────────

    #[test]
    fn normalise_collapses_double_space() {
        assert_eq!(normalise_for_blocklist("rm  -rf  /"), "rm -rf /");
    }

    #[test]
    fn normalise_collapses_tab() {
        assert_eq!(normalise_for_blocklist("rm\t-rf\t/"), "rm -rf /");
    }

    #[test]
    fn normalise_unicode_hyphen() {
        // U+2010 hyphen, U+2212 minus, U+FF0D fullwidth hyphen-minus all → '-'.
        assert_eq!(normalise_for_blocklist("rm \u{2010}rf /"), "rm -rf /");
        assert_eq!(normalise_for_blocklist("rm \u{2212}rf /"), "rm -rf /");
        assert_eq!(normalise_for_blocklist("rm \u{FF0D}rf /"), "rm -rf /");
    }

    #[test]
    fn normalise_non_breaking_space() {
        assert_eq!(normalise_for_blocklist("rm\u{00A0}-rf /"), "rm -rf /");
    }

    #[test]
    fn normalise_uppercase() {
        assert_eq!(
            normalise_for_blocklist("DROP TABLE Users"),
            "drop table users"
        );
    }

    // ── match_blocklist ───────────────────────────────────────────────────

    #[test]
    fn match_blocklist_double_space_rm_rf() {
        // Pre-fix: literal `cmd.contains("rm -rf /")` missed the double space.
        let pat = match_blocklist("rm  -rf  /", &["rm -rf /"]);
        assert_eq!(pat, Some("rm -rf /"));
    }

    #[test]
    fn match_blocklist_unicode_hyphen() {
        let pat = match_blocklist("rm \u{2010}rf /", &["rm -rf /"]);
        assert_eq!(pat, Some("rm -rf /"));
    }

    #[test]
    fn match_blocklist_clean_command() {
        let pat = match_blocklist("ls -la /tmp", &["rm -rf /"]);
        assert!(pat.is_none());
    }

    // ── validate_git_extern_arg ───────────────────────────────────────────

    #[test]
    fn git_extern_rejects_dash_prefix() {
        // The classic `--upload-pack=/tmp/evil` and `--exec=sh` vectors.
        assert!(validate_git_extern_arg("remote", "--upload-pack=/tmp/evil").is_err());
        assert!(validate_git_extern_arg("remote", "--exec=sh").is_err());
        assert!(validate_git_extern_arg("remote", "-c").is_err());
    }

    #[test]
    fn git_extern_rejects_shell_metas() {
        assert!(validate_git_extern_arg("remote", "origin;rm -rf /").is_err());
        assert!(validate_git_extern_arg("remote", "$(whoami)").is_err());
    }

    #[test]
    fn git_extern_accepts_legit_names() {
        assert!(validate_git_extern_arg("remote", "origin").is_ok());
        assert!(validate_git_extern_arg("remote", "upstream").is_ok());
        assert!(validate_git_extern_arg("branch", "feat/foo-bar").is_ok());
    }

    #[test]
    fn git_extern_rejects_empty() {
        assert!(validate_git_extern_arg("remote", "").is_err());
    }

    // ── validate_git_ref ──────────────────────────────────────────────────

    #[test]
    fn git_ref_accepts_common_forms() {
        assert!(validate_git_ref("ref", "HEAD").is_ok());
        assert!(validate_git_ref("ref", "main").is_ok());
        assert!(validate_git_ref("ref", "v1.2.3").is_ok());
        assert!(validate_git_ref("ref", "refs/heads/main").is_ok());
        assert!(validate_git_ref("ref", "abc123def").is_ok());
        assert!(validate_git_ref("ref", "HEAD~1").is_ok());
        assert!(validate_git_ref("ref", "main^").is_ok());
    }

    #[test]
    fn git_ref_rejects_dash_prefix() {
        assert!(validate_git_ref("ref", "--exec=sh").is_err());
    }

    // ── validate_git_reset_mode ───────────────────────────────────────────

    #[test]
    fn reset_mode_accepts_safe_modes() {
        for m in GIT_RESET_MODES {
            assert!(
                validate_git_reset_mode(m).is_ok(),
                "rejected safe mode: {}",
                m
            );
        }
    }

    #[test]
    fn reset_mode_rejects_exec_injection() {
        // The canonical attack from audit H-5: mode = "exec=sh" → git reset --exec=sh.
        assert!(validate_git_reset_mode("exec=sh").is_err());
        assert!(validate_git_reset_mode("upload-pack=/tmp/evil").is_err());
        assert!(validate_git_reset_mode("").is_err());
        assert!(validate_git_reset_mode("HARD").is_err()); // case-sensitive on purpose
    }

    // ── validate_search_pattern ───────────────────────────────────────────

    #[test]
    fn search_pattern_rejects_pre_flag() {
        // Audit M-8 — pattern="--pre=/bin/sh" makes rg execute /bin/sh as a
        // preprocessor. Even with `--` separator, we surface a clear error.
        assert!(validate_search_pattern("--pre=/bin/sh").is_err());
        assert!(validate_search_pattern("-e").is_err());
        assert!(validate_search_pattern("-f").is_err());
    }

    #[test]
    fn search_pattern_accepts_normal_regex() {
        assert!(validate_search_pattern(r"fn \w+").is_ok());
        assert!(validate_search_pattern("TODO").is_ok());
        assert!(validate_search_pattern(r"\bclass\b").is_ok());
    }

    #[test]
    fn search_pattern_rejects_empty() {
        assert!(validate_search_pattern("").is_err());
    }
}
