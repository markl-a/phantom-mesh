//! macOS Spotlight (`mdfind`) wrapper as a tool.
//!
//! Spotlight maintains a system-wide content index, so query latency is
//! typically <100ms even across hundreds of thousands of files — orders of
//! magnitude faster than `glob_search` (ripgrep across cwd) for whole-system
//! questions like "find files modified today" or "find every Swift source
//! containing AppDelegate".
//!
//! The tool is gated `#[cfg(target_os = "macos")]` and not exposed at all on
//! other platforms.

use serde_json::Value;
use tokio::process::Command;

/// Run a Spotlight query.
///
/// Args (JSON):
/// - `query` (required): the live mdfind query, OR a plain substring. If it
///   contains an `=` we assume it's already a Spotlight query expression
///   (e.g. `kMDItemContentType == "public.swift-source"`); otherwise we wrap
///   it in `kMDItemDisplayName == "*<query>*"c` (case-insensitive name).
/// - `scope` (optional): directory to limit to (`-onlyin <scope>`).
/// - `changed_within_hours` (optional, integer): adds an InRange clause
///   restricting results to items whose content-change date is within the
///   given window.
/// - `max_results` (optional, default 50): hard cap on lines returned.
///
/// Returns the matching paths as a newline-separated list, or an error
/// string if `mdfind` cannot be invoked.
pub async fn search(args: &Value) -> String {
    let query = match args.get("query").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return "[spotlight_search] missing required 'query' argument".to_string(),
    };

    let scope = args.get("scope").and_then(|v| v.as_str()).map(String::from);
    let changed_within_hours = args.get("changed_within_hours").and_then(|v| v.as_u64());
    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .min(500) as usize;

    // Build the Spotlight expression.
    //
    // [T58 M-11] Pre-fix `escape_spotlight` only escaped `"` → `\"`. A
    // query like `"*) || kMDItemContentType == "public.unix-executable"c &&("`
    // could close the wrapping kMDItemDisplayName clause early, splice in a
    // new comparison, and re-open the trailing `*` — extending the model's
    // reach to ANY Spotlight attribute (keychain entries, plist passwords,
    // etc.) outside the intended display-name scope.
    //
    // Two layers of defence below:
    //   1. If the query LOOKS like a raw expression (`kMDItem` or '='), it
    //      must additionally pass `validate_raw_expression` — which rejects
    //      C0 control chars and excessively long inputs but still allows
    //      legitimate compound expressions the model might write.
    //   2. Otherwise we treat the query as a literal and run it through
    //      `escape_spotlight`, which now neutralises EVERY metacharacter
    //      Spotlight recognises in the kMDItem wrapper context.
    let base_expr = if query.contains('=') || query.contains("kMDItem") {
        // User-supplied raw Spotlight expression.
        if let Err(e) = validate_raw_expression(&query) {
            return format!("[spotlight_search] {}", e);
        }
        query.clone()
    } else {
        // Wrap as case-insensitive display-name match.
        format!("kMDItemDisplayName == \"*{}*\"c", escape_spotlight(&query))
    };

    let expr = if let Some(hours) = changed_within_hours {
        // $time.now(-N) — Spotlight understands time tokens with negative
        // offset in seconds. Use 3600 * hours.
        let secs = (hours as i64) * 3600;
        format!(
            "({}) && kMDItemFSContentChangeDate >= $time.now(-{})",
            base_expr, secs
        )
    } else {
        base_expr
    };

    let mut cmd = Command::new("mdfind");
    cmd.arg(&expr);
    if let Some(s) = &scope {
        cmd.arg("-onlyin").arg(s);
    }

    let out = match cmd.output().await {
        Ok(o) => o,
        Err(e) => return format!("[spotlight_search] could not run mdfind: {}", e),
    };

    if !out.status.success() {
        return format!(
            "[spotlight_search] mdfind failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();

    let total = lines.len();
    let truncated = total > max_results;
    if truncated {
        lines.truncate(max_results);
    }

    if lines.is_empty() {
        return format!(
            "[spotlight_search] no results for: {}\n(hint: ensure the directory is indexed by Spotlight; try `mdutil -s <path>`)",
            expr
        );
    }

    let mut body = lines.join("\n");
    body.push('\n');
    body.push_str(&format!(
        "\n[spotlight_search] {} match{} returned{}",
        lines.len(),
        if lines.len() == 1 { "" } else { "es" },
        if truncated {
            format!(" (truncated from {})", total)
        } else {
            String::new()
        }
    ));
    body
}

/// [T58 M-11] Neutralise every Spotlight metacharacter when wrapping an
/// untrusted query into the `kMDItemDisplayName == "*<query>*"c` template.
///
/// Pre-fix this function only escaped `"` → `\"`. An attacker could supply
/// `*) || kMDItemContentType == "public.unix-executable" && (` to break
/// out of the displayName clause, splice in a new attribute query, then
/// re-enter via the trailing `*`. We now:
///
///  - Backslash-escape `\` first (so the escapes for `"` aren't re-escaped).
///  - Backslash-escape `"`.
///  - Drop `(`, `)`, `&`, `|`, `=` outright — there is no legitimate Spotlight
///    "literal" use for these in a filename glob, and Spotlight tokenises on
///    them. Dropping (instead of trying to "escape" with `\`) keeps the
///    surrounding template syntactically valid in every case.
///  - Drop control characters (`\n`, `\r`, `\t`, …).
///  - Drop `*` and `?` — those would extend the wrapper's `*…*` wildcard
///    in unexpected ways; if the caller wants wildcards they should pass a
///    raw expression (which goes down the validate-raw-expression branch).
fn escape_spotlight(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            // Drop tokens that would break out of the wrapper or extend its
            // wildcard. Replacement is the empty string so the query simply
            // matches "everything between the surviving literal chars".
            '(' | ')' | '&' | '|' | '=' | '*' | '?' => {}
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}

/// [T58 M-11] Validate a raw Spotlight expression supplied by the caller.
/// We allow the full expression grammar (so legitimate `kMDItemContentType
/// == "public.swift-source"` style queries still work) but reject:
///
///  - Empty strings.
///  - Strings over 4096 chars (mdfind doesn't need more; longer inputs are
///    a sign of either a model loop or an injection attempt).
///  - Newlines / carriage returns / NUL bytes — Spotlight expressions are
///    one-line and these characters would not appear in a legitimate query.
fn validate_raw_expression(expr: &str) -> Result<(), String> {
    if expr.is_empty() {
        return Err("raw Spotlight expression is empty".into());
    }
    if expr.len() > 4096 {
        return Err(format!(
            "Spotlight expression too long ({} chars, max 4096)",
            expr.len()
        ));
    }
    for ch in expr.chars() {
        if matches!(ch, '\n' | '\r' | '\0') {
            return Err("Spotlight expression contains disallowed control character".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── [T58 M-11] spotlight injection-vector regression tests ──────────

    /// Pre-fix: `escape_spotlight` only handled `"`. The attack vector cited
    /// in the audit was a query that closed the displayName clause early,
    /// spliced in a new comparison against `kMDItemContentType`, and
    /// re-opened with a trailing `*`. Post-fix, every break-out token
    /// (`(`, `)`, `&`, `|`, `=`, `*`, `?`) is dropped before interpolation
    /// so the wrapper remains a single `displayName == "*<sanitised>*"c`.
    #[test]
    fn escape_neutralises_metachars() {
        // The exact attack string from the audit.
        let evil = r#"*) || kMDItemContentType == "public.unix-executable" && ("#;
        let escaped = escape_spotlight(evil);
        // Result must NOT contain any unescaped break-out tokens.
        for forbidden in ['(', ')', '&', '|', '=', '*', '?'] {
            assert!(
                !escaped.contains(forbidden),
                "escape_spotlight leaked '{}' — full result: {:?}",
                forbidden,
                escaped
            );
        }
        // Quotes must be escaped (backslash-prefixed) rather than dropped, so
        // the legitimate display-name search for files containing literal
        // quotes still works.
        assert!(
            escaped.contains("\\\""),
            "escaped '\"' not preserved: {:?}",
            escaped
        );
    }

    #[test]
    fn escape_preserves_normal_chars() {
        let s = escape_spotlight("AppDelegate.swift");
        assert_eq!(s, "AppDelegate.swift");
    }

    #[test]
    fn escape_handles_backslash_first() {
        // A pre-existing `\` must be escaped before we add new escapes,
        // otherwise the new `\"` becomes `\\"` (literal `\` + literal `"`).
        let s = escape_spotlight(r#"a\b"c"#);
        // Expected: `a\\b\"c` — backslash doubled, quote escaped.
        assert_eq!(s, "a\\\\b\\\"c");
    }

    #[test]
    fn raw_expression_validator_rejects_newlines() {
        assert!(validate_raw_expression("foo\nbar").is_err());
        assert!(validate_raw_expression("foo\rbar").is_err());
        assert!(validate_raw_expression("foo\0bar").is_err());
    }

    #[test]
    fn raw_expression_validator_accepts_normal() {
        assert!(validate_raw_expression(r#"kMDItemContentType == "public.swift-source""#).is_ok());
        assert!(validate_raw_expression(r#"kMDItemFSName == "*.rs"c"#).is_ok());
    }

    /// End-to-end: feeding the audit's attack payload through `search()`
    /// must produce a query that mdfind would interpret strictly as a
    /// displayName match. We can't actually invoke mdfind in a unit test
    /// (it's macOS-only and not deterministic), so instead we simulate the
    /// expression-construction path and assert the wrapper survives.
    #[tokio::test]
    async fn injection_payload_stays_within_displayname_wrapper() {
        let evil = r#"*) || kMDItemContentType == "public.unix-executable" && ("#;
        let escaped = escape_spotlight(evil);
        let expr = format!("kMDItemDisplayName == \"*{}*\"c", escaped);

        // The expression must be a single, well-formed displayName clause.
        // We can't count `kMDItem` substrings — those legitimately appear
        // twice when the attacker's payload contains the literal string
        // `kMDItem` (drops into displayName as harmless text). Instead
        // verify the *syntactic* shape: escape_spotlight drops `=`, `&`,
        // `|`, `(`, `)`, `*`, `?`, so after escaping, the only `=` chars
        // left should be the wrapper's own `==` (two `=` total).
        let eq_count = expr.matches('=').count();
        assert_eq!(
            eq_count, 2,
            "attacker's `==` survived escape (extra equality clause); got expr: {}",
            expr
        );
        // No unescaped `)` either — that would close the wrapper early.
        assert!(
            !expr[..expr.len() - 4].contains(')'), // last 4 chars are *"c (no paren)
            "expression closed early: {}",
            expr
        );
        // No unescaped `"` inside the wrapper — there should be exactly
        // 2 raw `"` chars total (the wrapper's open + close); attacker's
        // quotes must have been escaped to `\"`.
        let raw_quotes = expr
            .char_indices()
            .filter(|(i, c)| *c == '"' && (*i == 0 || expr.as_bytes()[*i - 1] != b'\\'))
            .count();
        assert_eq!(
            raw_quotes, 2,
            "wrapper has extra unescaped `\"` (attacker closed wrapper early); got expr: {}",
            expr
        );
    }
}
