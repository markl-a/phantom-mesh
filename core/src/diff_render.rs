//! Unified-diff renderer for previewing file edits before they are applied.
//!
//! Used by the `/perm diff` permission gate in the REPL to show a Codex-style
//! patch preview when the agent calls `file_edit`. The implementation is a
//! simple LCS-based line diff (good enough for small hunks; we typically
//! diff a single short replacement region, not whole files).
//!
//! Output is colored with raw ANSI escape codes:
//!   * `--- a/<path>`   → red
//!   * `+++ b/<path>`   → green
//!   * `@@ -a,b +c,d @@`→ purple/magenta
//!   * `- removed`      → red
//!   * `+ added`        → green
//!   * ` context`       → dim

const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const MAGENTA: &str = "\x1b[35m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

/// How many context lines to include around each hunk.
const CONTEXT: usize = 3;

#[derive(Debug, Clone, PartialEq)]
enum Op {
    Equal(usize, usize), // (old_idx, new_idx)
    Del(usize),          // old_idx
    Ins(usize),          // new_idx
}

/// Compute a line-level edit script via LCS dynamic programming.
/// Suitable for small inputs (<~2000 lines); O(n*m) memory.
fn lcs_diff(old: &[&str], new: &[&str]) -> Vec<Op> {
    let n = old.len();
    let m = new.len();
    // dp[i][j] = LCS length of old[..i] and new[..j]
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in 0..n {
        for j in 0..m {
            dp[i + 1][j + 1] = if old[i] == new[j] {
                dp[i][j] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    // Backtrack
    let mut ops = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old[i - 1] == new[j - 1] {
            ops.push(Op::Equal(i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            ops.push(Op::Ins(j - 1));
            j -= 1;
        } else if i > 0 {
            ops.push(Op::Del(i - 1));
            i -= 1;
        } else {
            break;
        }
    }
    ops.reverse();
    ops
}

/// Render a colored unified diff between `old_text` and `new_text` for `path`.
///
/// `path` is shown verbatim in the `--- a/` / `+++ b/` headers (no canonicalising).
pub fn render_unified_diff(path: &str, old_text: &str, new_text: &str) -> String {
    let old_lines: Vec<&str> = old_text.split('\n').collect();
    let new_lines: Vec<&str> = new_text.split('\n').collect();

    // Trim trailing empty line that comes from trailing '\n' in file content,
    // so the diff doesn't include a spectyn blank "line" after the real content.
    let trim_trailing = |v: &mut Vec<&str>| {
        if v.last().map(|s| s.is_empty()).unwrap_or(false) && v.len() > 1 {
            v.pop();
        }
    };
    let mut old_lines = old_lines;
    let mut new_lines = new_lines;
    trim_trailing(&mut old_lines);
    trim_trailing(&mut new_lines);

    let ops = lcs_diff(&old_lines, &new_lines);

    // Group ops into hunks: each hunk = a contiguous block of changes plus
    // CONTEXT equal lines on each side.
    let n_ops = ops.len();
    let mut hunks: Vec<(usize, usize)> = Vec::new(); // [start_op, end_op_exclusive]
    let mut i = 0;
    while i < n_ops {
        if matches!(ops[i], Op::Del(_) | Op::Ins(_)) {
            let mut start = i;
            // Walk back up to CONTEXT equal lines.
            let mut back = 0;
            while start > 0 && back < CONTEXT && matches!(ops[start - 1], Op::Equal(..)) {
                start -= 1;
                back += 1;
            }
            // Find end: continue past changes; allow up to 2*CONTEXT consecutive
            // equal lines to bridge nearby hunks.
            let mut j = i + 1;
            let mut equal_run = 0usize;
            while j < n_ops {
                match ops[j] {
                    Op::Equal(..) => {
                        equal_run += 1;
                        if equal_run > 2 * CONTEXT {
                            break;
                        }
                    }
                    _ => equal_run = 0,
                }
                j += 1;
            }
            // Trim trailing equals down to CONTEXT.
            let mut end = j;
            let mut tail_equal = 0;
            while end > start && matches!(ops[end - 1], Op::Equal(..)) && tail_equal < CONTEXT {
                tail_equal += 1;
                end -= 1;
            }
            // If we trimmed further than CONTEXT (because the run was longer),
            // the loop above already stopped at exactly CONTEXT — good.
            // But we may have stopped too early; pad back up to CONTEXT trailing equals.
            let mut want = CONTEXT.saturating_sub(tail_equal);
            while want > 0 && end < n_ops && matches!(ops[end], Op::Equal(..)) {
                end += 1;
                want -= 1;
            }
            hunks.push((start, end));
            i = end;
        } else {
            i += 1;
        }
    }

    if hunks.is_empty() {
        return format!(
            "{}--- a/{}{}\n{}+++ b/{}{}\n{}(no changes){}\n",
            RED, path, RESET, GREEN, path, RESET, DIM, RESET
        );
    }

    let mut out = String::new();
    out.push_str(&format!("{}--- a/{}{}\n", RED, path, RESET));
    out.push_str(&format!("{}+++ b/{}{}\n", GREEN, path, RESET));

    for (start, end) in hunks {
        // Compute @@ -old_start,old_count +new_start,new_count @@
        let mut old_start = 0usize;
        let mut new_start = 0usize;
        let mut old_count = 0usize;
        let mut new_count = 0usize;
        let mut got_start = false;
        for op in &ops[start..end] {
            match *op {
                Op::Equal(o, n) => {
                    if !got_start {
                        old_start = o;
                        new_start = n;
                        got_start = true;
                    }
                    old_count += 1;
                    new_count += 1;
                }
                Op::Del(o) => {
                    if !got_start {
                        old_start = o;
                        // new_start defaults to first change position; approximate.
                        got_start = true;
                    }
                    old_count += 1;
                }
                Op::Ins(n) => {
                    if !got_start {
                        new_start = n;
                        got_start = true;
                    }
                    new_count += 1;
                }
            }
        }
        out.push_str(&format!(
            "{}@@ -{},{} +{},{} @@{}\n",
            MAGENTA,
            old_start + 1,
            old_count,
            new_start + 1,
            new_count,
            RESET,
        ));
        for op in &ops[start..end] {
            match *op {
                Op::Equal(o, _) => {
                    out.push_str(&format!("{} {}{}\n", DIM, old_lines[o], RESET));
                }
                Op::Del(o) => {
                    out.push_str(&format!("{}- {}{}\n", RED, old_lines[o], RESET));
                }
                Op::Ins(n) => {
                    out.push_str(&format!("{}+ {}{}\n", GREEN, new_lines[n], RESET));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_ansi(s: &str) -> String {
        // Crude ANSI stripper for assertions.
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                // skip until 'm'
                for c2 in chars.by_ref() {
                    if c2 == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn unified_diff_single_line_change() {
        let old = "line one\nline two\nline three\n";
        let new = "line one\nLINE TWO ALTERED\nline three\n";
        let rendered = render_unified_diff("/tmp/diff-test.txt", old, new);
        let plain = strip_ansi(&rendered);
        assert!(
            plain.contains("--- a/tmp/diff-test.txt") || plain.contains("--- a//tmp/diff-test.txt")
        );
        assert!(plain.contains("+++ b/"));
        assert!(plain.contains("- line two"));
        assert!(plain.contains("+ LINE TWO ALTERED"));
        assert!(plain.contains(" line one"));
        assert!(plain.contains(" line three"));
        // Hunk header present
        assert!(plain.contains("@@ "));
    }

    #[test]
    fn unified_diff_no_changes() {
        let same = "a\nb\nc\n";
        let rendered = render_unified_diff("x.txt", same, same);
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("(no changes)"));
    }

    #[test]
    fn unified_diff_pure_addition() {
        let old = "a\n";
        let new = "a\nb\n";
        let rendered = render_unified_diff("x.txt", old, new);
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("+ b"));
    }

    #[test]
    fn unified_diff_empty_to_empty() {
        // Two empty inputs produce no changes (and must not panic).
        let rendered = render_unified_diff("empty.txt", "", "");
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("(no changes)"));
        // Headers are still emitted in the no-change branch.
        assert!(plain.contains("--- a/empty.txt"));
        assert!(plain.contains("+++ b/empty.txt"));
    }

    #[test]
    fn unified_diff_empty_to_content() {
        // Adding content to an empty file should surface the new line(s).
        let rendered = render_unified_diff("new.txt", "", "hello world\n");
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("+ hello world"));
        assert!(plain.contains("@@ "));
    }

    #[test]
    fn unified_diff_content_to_empty() {
        // Removing all content should surface the removed line(s).
        let rendered = render_unified_diff("gone.txt", "hello world\n", "");
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("- hello world"));
        assert!(plain.contains("@@ "));
    }

    #[test]
    fn unified_diff_single_line_no_trailing_newline() {
        // A single-line change with no trailing newline on either side.
        let rendered = render_unified_diff("one.txt", "before", "after");
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("- before"));
        assert!(plain.contains("+ after"));
        assert!(plain.contains("@@ "));
    }

    #[test]
    fn unified_diff_unicode_content() {
        // Non-ASCII content (CJK + emoji) must round-trip through the renderer
        // without panicking or corrupting the changed line.
        let old = "第一行\n第二行\n第三行\n";
        let new = "第一行\n第二行已修改 🚀\n第三行\n";
        let rendered = render_unified_diff("文件.txt", old, new);
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("--- a/文件.txt"));
        assert!(plain.contains("- 第二行"));
        assert!(plain.contains("+ 第二行已修改 🚀"));
        // Unchanged context lines preserved verbatim.
        assert!(plain.contains(" 第一行"));
        assert!(plain.contains(" 第三行"));
    }

    #[test]
    fn unified_diff_unicode_pure_addition() {
        // Inserting a unicode-only line into existing unicode content.
        let old = "α\nγ\n";
        let new = "α\nβ\nγ\n";
        let rendered = render_unified_diff("greek.txt", old, new);
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("+ β"));
    }
}
