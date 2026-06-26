//! `skill_diff` — produce a unified diff of two strings.
//!
//! Pure Rust LCS implementation. No external diff crate; uses only
//! `serde_json`, `async_trait`, and the standard library.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{SkillTool, ToolError, ToolResult};

pub struct Diff;

#[async_trait]
impl SkillTool for Diff {
    fn name(&self) -> &'static str {
        "skill_diff"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "skill_diff",
                "description": "Compute a unified diff between two strings. \
                    Optional `from_label`/`to_label` set the `---`/`+++` header names \
                    (default 'a'/'b'). Context is fixed at 3 lines.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "from":       {"type": "string"},
                        "to":         {"type": "string"},
                        "from_label": {"type": "string"},
                        "to_label":   {"type": "string"}
                    },
                    "required": ["from", "to"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let from = args
            .get("from")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("from required".into()))?;
        let to = args
            .get("to")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("to required".into()))?;
        let from_label = args
            .get("from_label")
            .and_then(|v| v.as_str())
            .unwrap_or("a");
        let to_label = args.get("to_label").and_then(|v| v.as_str()).unwrap_or("b");

        let a: Vec<&str> = from.split('\n').collect();
        let b: Vec<&str> = to.split('\n').collect();
        let ops = lcs_diff(&a, &b);
        let diff = render_unified(&a, &b, &ops, from_label, to_label);
        Ok(json!({ "diff": diff, "changed": !ops.iter().all(|o| matches!(o, Op::Eq(_, _))) }))
    }
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Eq(usize, usize),
    Del(usize),
    Ins(usize),
}

/// Classic LCS DP. Inputs are line vectors; returns a sequence of ops
/// in order from start to end.
fn lcs_diff(a: &[&str], b: &[&str]) -> Vec<Op> {
    let n = a.len();
    let m = b.len();
    // dp[i][j] = length of LCS of a[..i] and b[..j].
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in 0..n {
        for j in 0..m {
            dp[i + 1][j + 1] = if a[i] == b[j] {
                dp[i][j] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    // Backtrack.
    let mut ops = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        if a[i - 1] == b[j - 1] {
            ops.push(Op::Eq(i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] >= dp[i][j - 1] {
            ops.push(Op::Del(i - 1));
            i -= 1;
        } else {
            ops.push(Op::Ins(j - 1));
            j -= 1;
        }
    }
    while i > 0 {
        ops.push(Op::Del(i - 1));
        i -= 1;
    }
    while j > 0 {
        ops.push(Op::Ins(j - 1));
        j -= 1;
    }
    ops.reverse();
    ops
}

/// Render ops as a unified diff with 3 lines of context. Returns
/// empty string if the inputs are equal.
fn render_unified(a: &[&str], b: &[&str], ops: &[Op], from_label: &str, to_label: &str) -> String {
    if ops.iter().all(|o| matches!(o, Op::Eq(_, _))) {
        return String::new();
    }
    const CTX: usize = 3;
    // Find hunks: runs containing at least one change, padded with up to CTX equal lines.
    let mut change_idx: Vec<usize> = Vec::new();
    for (k, op) in ops.iter().enumerate() {
        if !matches!(op, Op::Eq(_, _)) {
            change_idx.push(k);
        }
    }
    // Group nearby change indices into hunks: gap > 2*CTX starts a new hunk.
    let mut hunks: Vec<(usize, usize)> = Vec::new(); // inclusive op-index ranges
    for &c in &change_idx {
        if let Some(last) = hunks.last_mut() {
            if c <= last.1 + 2 * CTX {
                last.1 = c;
                continue;
            }
        }
        hunks.push((c, c));
    }
    // Pad each hunk by CTX lines on each side, clamped.
    for h in hunks.iter_mut() {
        h.0 = h.0.saturating_sub(CTX);
        h.1 = (h.1 + CTX).min(ops.len() - 1);
    }
    let mut out = String::new();
    out.push_str(&format!("--- {}\n+++ {}\n", from_label, to_label));
    for (lo, hi) in hunks {
        // Count old/new lines + start positions.
        let mut a_start = 0;
        let mut b_start = 0;
        let mut found_start = false;
        let mut a_count = 0;
        let mut b_count = 0;
        for op in &ops[lo..=hi] {
            match *op {
                Op::Eq(ai, bi) => {
                    if !found_start {
                        a_start = ai + 1;
                        b_start = bi + 1;
                        found_start = true;
                    }
                    a_count += 1;
                    b_count += 1;
                }
                Op::Del(ai) => {
                    if !found_start {
                        a_start = ai + 1;
                        b_start = ops[lo..]
                            .iter()
                            .find_map(|o| {
                                if let Op::Ins(bi) | Op::Eq(_, bi) = o {
                                    Some(*bi + 1)
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(1);
                        found_start = true;
                    }
                    a_count += 1;
                }
                Op::Ins(bi) => {
                    if !found_start {
                        b_start = bi + 1;
                        a_start = ops[lo..]
                            .iter()
                            .find_map(|o| {
                                if let Op::Del(ai) | Op::Eq(ai, _) = o {
                                    Some(*ai + 1)
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(1);
                        found_start = true;
                    }
                    b_count += 1;
                }
            }
        }
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            a_start, a_count, b_start, b_count
        ));
        for op in &ops[lo..=hi] {
            match *op {
                Op::Eq(ai, _) => {
                    out.push(' ');
                    out.push_str(a[ai]);
                    out.push('\n');
                }
                Op::Del(ai) => {
                    out.push('-');
                    out.push_str(a[ai]);
                    out.push('\n');
                }
                Op::Ins(bi) => {
                    out.push('+');
                    out.push_str(b[bi]);
                    out.push('\n');
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn identical_inputs_produce_empty_diff() {
        let tool = Diff;
        let r = tool
            .call(&json!({"from": "a\nb\nc", "to": "a\nb\nc"}))
            .await
            .unwrap();
        assert_eq!(r["diff"], "");
        assert_eq!(r["changed"], false);
    }

    #[tokio::test]
    async fn single_line_change_renders_hunk_header() {
        let tool = Diff;
        let r = tool
            .call(&json!({"from": "a\nb\nc", "to": "a\nB\nc"}))
            .await
            .unwrap();
        let diff = r["diff"].as_str().unwrap();
        assert!(diff.starts_with("--- a\n+++ b\n"), "diff = {:?}", diff);
        assert!(diff.contains("@@"), "diff = {:?}", diff);
        assert!(diff.contains("-b\n"), "diff = {:?}", diff);
        assert!(diff.contains("+B\n"), "diff = {:?}", diff);
        assert_eq!(r["changed"], true);
    }

    #[tokio::test]
    async fn custom_labels_show_in_header() {
        let tool = Diff;
        let r = tool
            .call(&json!({
                "from": "x", "to": "y",
                "from_label": "old.txt", "to_label": "new.txt"
            }))
            .await
            .unwrap();
        let diff = r["diff"].as_str().unwrap();
        assert!(diff.starts_with("--- old.txt\n+++ new.txt\n"));
    }
}
