//! Shell output filtering — ANSI escape stripping and token-aware truncation.
//!
//! Shell commands can produce output that is hostile to LLM consumption:
//!
//! 1. **ANSI escape sequences** — color codes, cursor movements, terminal
//!    control characters that add noise and waste tokens.
//! 2. **Unbounded length** — a `cargo build` or `git log` can emit megabytes
//!    of text.  We keep the head and tail (most likely to be informative)
//!    and drop the middle with a short summary line.
//!
//! ## Public API
//! - [`strip_ansi`] — remove all ANSI escape sequences
//! - [`truncate_shell_output`] — head/tail truncation with a count separator
//! - [`clean_shell_output`] — strip ANSI then truncate to 16 000 chars

use once_cell::sync::Lazy;
use regex::Regex;

// ── Regex patterns ────────────────────────────────────────────────────────────

/// Standard CSI sequences: ESC [ ... <final-byte>
/// e.g. `\x1b[31m` (red), `\x1b[2J` (clear screen), `\x1b[1;33m` (bold yellow)
static ANSI_CSI: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap()
});

/// OSC sequences: ESC ] ... ST  (used for window titles, hyperlinks, etc.)
/// ST can be BEL (\x07) or ESC \ (\x1b\\)
static ANSI_OSC: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)").unwrap()
});

/// Simple two-byte sequences: ESC followed by a single letter (e.g. ESC M, ESC c)
static ANSI_SIMPLE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\x1b[a-zA-Z]").unwrap()
});

// ── Public functions ──────────────────────────────────────────────────────────

/// Remove all ANSI escape sequences from `input`.
///
/// Handles:
/// - CSI sequences (`ESC [ ... <letter>`) — colors, cursor movement, SGR
/// - OSC sequences (`ESC ] ... BEL/ST`) — window titles, hyperlinks
/// - Simple two-byte ESC sequences (`ESC <letter>`)
pub fn strip_ansi(input: &str) -> String {
    let s = ANSI_OSC.replace_all(input, "");
    let s = ANSI_CSI.replace_all(&s, "");
    let s = ANSI_SIMPLE.replace_all(&s, "");
    s.into_owned()
}

/// Truncate `output` to at most `max_chars` **characters** (not bytes).
///
/// If the output fits within `max_chars`, it is returned unchanged.
///
/// Otherwise, the first 40 % and last 40 % of `max_chars` are kept, separated
/// by a one-line summary:
/// ```text
/// \n... [truncated N lines] ...\n
/// ```
/// The split always falls on a valid UTF-8 character boundary.
pub fn truncate_shell_output(output: &str, max_chars: usize) -> String {
    // Character count is always <= byte count, so this is the cheap early exit.
    if output.chars().count() <= max_chars {
        return output.to_string();
    }

    let head_chars = max_chars * 2 / 5;  // 40 %
    let tail_chars = max_chars * 2 / 5;  // 40 %

    // Collect char indices so we can slice on char boundaries without an O(n)
    // walk twice.
    let head_end_byte = char_boundary_at(output, head_chars);
    let head = &output[..head_end_byte];

    // For the tail we need to find the byte offset of the last `tail_chars`
    // characters.
    let tail_start_byte = tail_byte_start(output, tail_chars);
    let tail = &output[tail_start_byte..];

    // Count dropped lines for the summary.
    let middle = &output[head_end_byte..tail_start_byte];
    let dropped_lines = middle.lines().count().max(1);

    format!(
        "{}\n... [truncated {} lines] ...\n{}",
        head, dropped_lines, tail
    )
}

/// Strip ANSI sequences then truncate to **16 000 characters**.
///
/// This is the convenience function used by the shell tool before returning
/// output to the agent.
pub fn clean_shell_output(output: &str) -> String {
    let stripped = strip_ansi(output);
    truncate_shell_output(&stripped, 16_000)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Return the byte index of the `n`-th character boundary (0-indexed char
/// count), clamped to `s.len()`.  Always returns a valid UTF-8 char boundary.
fn char_boundary_at(s: &str, n: usize) -> usize {
    s.char_indices()
        .nth(n)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

/// Return the byte index at which the **last** `n` characters of `s` start.
/// Always returns a valid UTF-8 char boundary.
fn tail_byte_start(s: &str, n: usize) -> usize {
    let total = s.chars().count();
    if n >= total {
        return 0;
    }
    let skip = total - n;
    char_boundary_at(s, skip)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── strip_ansi ────────────────────────────────────────────────────────

    #[test]
    fn test_strip_ansi_color_codes() {
        let input = "\x1b[31mError\x1b[0m: something went wrong";
        assert_eq!(strip_ansi(input), "Error: something went wrong");
    }

    #[test]
    fn test_strip_ansi_bold_and_underline() {
        let input = "\x1b[1mBold\x1b[0m and \x1b[4mUnderlined\x1b[0m";
        assert_eq!(strip_ansi(input), "Bold and Underlined");
    }

    #[test]
    fn test_strip_ansi_256_color() {
        // 256-color foreground: ESC[38;5;214m
        let input = "\x1b[38;5;214mOrange text\x1b[0m";
        assert_eq!(strip_ansi(input), "Orange text");
    }

    #[test]
    fn test_strip_ansi_cursor_movement() {
        // Cursor up 3 lines: ESC[3A
        let input = "line1\x1b[3Aline2";
        assert_eq!(strip_ansi(input), "line1line2");
    }

    #[test]
    fn test_strip_ansi_clear_screen() {
        // ESC[2J (clear screen) and ESC[H (cursor home)
        let input = "\x1b[2J\x1b[HHello";
        assert_eq!(strip_ansi(input), "Hello");
    }

    #[test]
    fn test_strip_ansi_osc_window_title() {
        // OSC 0 ; title BEL
        let input = "\x1b]0;My Terminal\x07Some output";
        assert_eq!(strip_ansi(input), "Some output");
    }

    #[test]
    fn test_strip_ansi_osc_with_st_terminator() {
        // OSC terminated with ESC \
        let input = "\x1b]2;title\x1b\\output";
        assert_eq!(strip_ansi(input), "output");
    }

    #[test]
    fn test_strip_ansi_simple_escape() {
        // ESC M (reverse index) and ESC c (reset)
        let input = "before\x1bMafter\x1bcend";
        assert_eq!(strip_ansi(input), "beforeafterend");
    }

    #[test]
    fn test_strip_ansi_no_escapes() {
        let input = "plain text with no escape codes";
        assert_eq!(strip_ansi(input), input);
    }

    #[test]
    fn test_strip_ansi_empty() {
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn test_strip_ansi_only_escapes() {
        let input = "\x1b[31m\x1b[0m\x1b[1m\x1b[0m";
        assert_eq!(strip_ansi(input), "");
    }

    #[test]
    fn test_strip_ansi_multiline() {
        let input = "\x1b[32m✓\x1b[0m Build succeeded\n\x1b[31m✗\x1b[0m Tests failed";
        let result = strip_ansi(input);
        assert!(result.contains("Build succeeded"));
        assert!(result.contains("Tests failed"));
        assert!(!result.contains('\x1b'));
    }

    #[test]
    fn test_strip_ansi_preserves_utf8() {
        // Multi-byte chars should survive untouched
        let input = "\x1b[33m繁體中文\x1b[0m output";
        assert_eq!(strip_ansi(input), "繁體中文 output");
    }

    // ── truncate_shell_output ─────────────────────────────────────────────

    #[test]
    fn test_truncate_short_output_unchanged() {
        let output = "hello world";
        assert_eq!(truncate_shell_output(output, 16_000), output);
    }

    #[test]
    fn test_truncate_empty_unchanged() {
        assert_eq!(truncate_shell_output("", 16_000), "");
    }

    #[test]
    fn test_truncate_exactly_at_limit() {
        // Exactly max_chars — should not truncate
        let output = "a".repeat(100);
        assert_eq!(truncate_shell_output(&output, 100), output);
    }

    #[test]
    fn test_truncate_one_over_limit() {
        // max_chars + 1 must trigger truncation
        let output = "a".repeat(101);
        let result = truncate_shell_output(&output, 100);
        assert!(result.contains("[truncated"), "Expected truncation marker, got: {}", result);
    }

    #[test]
    fn test_truncate_contains_head_and_tail() {
        // Build a string with a distinct head, large middle, and distinct tail
        let head = "HEAD_START\n".repeat(5);
        let middle = "MIDDLE_LINE\n".repeat(200);
        let tail = "TAIL_END\n".repeat(5);
        let output = format!("{}{}{}", head, middle, tail);

        let result = truncate_shell_output(&output, 200);

        assert!(result.contains("HEAD_START"), "Head should be preserved");
        assert!(result.contains("TAIL_END"), "Tail should be preserved");
        assert!(result.contains("[truncated"), "Truncation marker required");
        // The total result must be shorter than the input
        assert!(result.len() < output.len());
    }

    #[test]
    fn test_truncate_counts_lines_in_separator() {
        let lines: Vec<String> = (0..1000).map(|i| format!("line {}", i)).collect();
        let output = lines.join("\n");
        let result = truncate_shell_output(&output, 500);
        // The separator must mention that lines were truncated
        assert!(result.contains("[truncated"));
        assert!(result.contains("lines]"));
    }

    #[test]
    fn test_truncate_result_within_limit() {
        // The result must not exceed max_chars by more than the separator overhead
        let output = "x".repeat(50_000);
        let max = 16_000;
        let result = truncate_shell_output(&output, max);
        // Allow separator overhead (~50 bytes)
        assert!(
            result.chars().count() <= max + 60,
            "Result chars {} exceeds limit {} + overhead",
            result.chars().count(),
            max
        );
    }

    #[test]
    fn test_truncate_utf8_safety() {
        // 4-byte emoji repeated — char boundaries must be respected
        let emoji = "🔥";  // 4 bytes per char
        let output = emoji.repeat(10_000);
        // Should not panic on multi-byte char boundaries
        let result = truncate_shell_output(&output, 100);
        // Result must be valid UTF-8 (Rust strings always are, but slice
        // operations could panic if we hit a byte boundary mid-char)
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
    }

    #[test]
    fn test_truncate_cjk_utf8_safety() {
        // 3-byte CJK characters
        let cjk = "中文測試字符";
        let output = cjk.repeat(5_000);
        let result = truncate_shell_output(&output, 200);
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
        // Must still contain some CJK
        assert!(result.contains('中') || result.contains('['));
    }

    // ── clean_shell_output ────────────────────────────────────────────────

    #[test]
    fn test_clean_strips_then_truncates() {
        // Each colored line: "\x1b[32mok\x1b[0m line\n"
        // After ANSI strip: "ok line\n" = 8 chars.
        // 2001 repetitions → 16 008 chars, which exceeds the 16 000 char limit.
        let colored_line = "\x1b[32mok\x1b[0m line\n";
        let output = colored_line.repeat(2001);

        let result = clean_shell_output(&output);

        // No escape sequences should remain
        assert!(!result.contains('\x1b'), "ANSI escapes survived clean");
        // Must be truncated
        assert!(result.contains("[truncated"), "Expected truncation marker");
        // Must contain actual content
        assert!(result.contains("ok line"));
    }

    #[test]
    fn test_clean_short_colorized_unchanged_length() {
        let input = "\x1b[33mwarning\x1b[0m: unused variable";
        let result = clean_shell_output(input);
        assert_eq!(result, "warning: unused variable");
    }

    #[test]
    fn test_clean_empty() {
        assert_eq!(clean_shell_output(""), "");
    }

    #[test]
    fn test_clean_plain_text_short() {
        let input = "cargo test passed: 42 tests";
        assert_eq!(clean_shell_output(input), input);
    }
}
