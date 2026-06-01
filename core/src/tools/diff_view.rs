use serde_json::Value;

use crate::tools::file::safe_path;

const MAX_OUTPUT: usize = 5000;

// ── Myers diff ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Op {
    Equal(usize, usize), // (idx_a, idx_b)
    Delete(usize),       // idx_a
    Insert(usize),       // idx_b
}

/// Compute a sequence of edit operations between two line slices using a
/// simplified O(ND) Myers algorithm.
fn myers_diff<'a>(a: &'a [&'a str], b: &'a [&'a str]) -> Vec<Op> {
    let n = a.len();
    let m = b.len();
    let max = n + m;

    // v[k + max] = furthest x reached along diagonal k
    let mut v: Vec<i64> = vec![-1; 2 * max + 2];
    v[max + 1] = 0;

    // trace[d] = snapshot of v after d-th round
    let mut trace: Vec<Vec<i64>> = Vec::new();

    'outer: for d in 0..=(max as i64) {
        trace.push(v.clone());
        let k_min = -d;
        let k_max = d;
        let mut k = k_min;
        while k <= k_max {
            let ki = (k + max as i64) as usize;
            let mut x: i64 = if k == -d || (k != d && v[ki - 1] < v[ki + 1]) {
                v[ki + 1]
            } else {
                v[ki - 1] + 1
            };
            let mut y = x - k;
            // snake
            while x < n as i64 && y < m as i64 && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[ki] = x;
            if x >= n as i64 && y >= m as i64 {
                break 'outer;
            }
            k += 2;
        }
    }

    // Backtrack through trace to reconstruct ops
    let mut ops: Vec<Op> = Vec::new();
    let mut x = n as i64;
    let mut y = m as i64;

    for (d, snapshot) in trace.iter().enumerate().rev() {
        let d = d as i64;
        let k = x - y;
        let ki = (k + max as i64) as usize;

        let prev_k = if k == -d || (k != d && snapshot[ki - 1] < snapshot[ki + 1]) {
            k + 1
        } else {
            k - 1
        };
        let prev_ki = (prev_k + max as i64) as usize;
        let prev_x = snapshot[prev_ki];
        let prev_y = prev_x - prev_k;

        // Walk back along the snake
        let mut cx = x;
        let mut cy = y;
        while cx > prev_x + 1 && cy > prev_y + 1 {
            cx -= 1;
            cy -= 1;
            ops.push(Op::Equal(cx as usize, cy as usize));
        }

        if d > 0 {
            // Myers V-graph: k = x - y.
            //   Going DOWN  (delete) increases x, leaves y → new_k = old_k - 1
            //   Going RIGHT (insert) leaves x, increases y → new_k = old_k + 1
            //
            // So prev_k == k - 1 means the last move was a DELETE (down) and
            // the deleted character is a[prev_x]. The previous version had
            // these labels swapped, which produced indices off-the-end of `a`
            // for inputs like "hello" vs "hellx" → panic at format!("-{}", a[*ai]).
            if prev_k == k - 1 {
                ops.push(Op::Delete(prev_x as usize));
            } else {
                ops.push(Op::Insert(prev_y as usize));
            }
        }

        x = prev_x;
        y = prev_y;
    }

    ops.reverse();
    ops
}

// ── Hunk formatting ──────────────────────────────────────────────────────────

struct Hunk {
    a_start: usize, // 1-based
    b_start: usize,
    lines: Vec<String>,
    a_count: usize,
    b_count: usize,
}

fn build_hunks(ops: &[Op], a: &[&str], b: &[&str], ctx: usize) -> Vec<Hunk> {
    // Classify each op position
    #[derive(Clone)]
    enum Kind {
        Eq(usize, usize),
        Del(usize),
        Ins(usize),
    }

    let flat: Vec<Kind> = ops
        .iter()
        .map(|op| match *op {
            Op::Equal(ai, bi) => Kind::Eq(ai, bi),
            Op::Delete(ai) => Kind::Del(ai),
            Op::Insert(bi) => Kind::Ins(bi),
        })
        .collect();

    // Find indices of changed ops
    let changed: Vec<usize> = flat
        .iter()
        .enumerate()
        .filter(|(_, k)| !matches!(k, Kind::Eq(_, _)))
        .map(|(i, _)| i)
        .collect();

    if changed.is_empty() {
        return vec![];
    }

    // Group changed indices into ranges with ctx expansion
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut start = changed[0].saturating_sub(ctx);
    let mut end = (changed[0] + ctx + 1).min(flat.len());

    for &ci in &changed[1..] {
        let lo = ci.saturating_sub(ctx);
        let hi = (ci + ctx + 1).min(flat.len());
        if lo <= end {
            end = hi;
        } else {
            ranges.push((start, end));
            start = lo;
            end = hi;
        }
    }
    ranges.push((start, end));

    // Build hunks
    let mut hunks = Vec::new();
    for (range_start, range_end) in ranges {
        let mut lines: Vec<String> = Vec::new();
        let mut a_count = 0usize;
        let mut b_count = 0usize;
        let mut a_start = 0usize;
        let mut b_start = 0usize;
        let mut first = true;

        for kind in &flat[range_start..range_end] {
            // Bounds-check every Kind index. Without this, an off-by-one
            // from the diff producer (a Kind::Del(ai) where ai == a.len())
            // panics with "index out of bounds" — see crash logs from
            // 2026-04-30 09:00–09:03 (3 instances within one minute,
            // implying a hot path). Defend in depth: skip out-of-range
            // entries silently, since they signal a producer-side bug
            // we can fix later but must not crash MCP `tools/call` over.
            match kind {
                Kind::Eq(ai, bi) => {
                    if *ai >= a.len() || *bi >= b.len() {
                        continue;
                    }
                    if first {
                        a_start = ai + 1;
                        b_start = bi + 1;
                        first = false;
                    }
                    lines.push(format!(" {}", a[*ai]));
                    a_count += 1;
                    b_count += 1;
                }
                Kind::Del(ai) => {
                    if *ai >= a.len() {
                        continue;
                    }
                    if first {
                        a_start = ai + 1;
                        b_start = 1; // placeholder, will be fixed below
                        first = false;
                    }
                    lines.push(format!("-{}", a[*ai]));
                    a_count += 1;
                }
                Kind::Ins(bi) => {
                    if *bi >= b.len() {
                        continue;
                    }
                    if first {
                        a_start = 1;
                        b_start = bi + 1;
                        first = false;
                    }
                    lines.push(format!("+{}", b[*bi]));
                    b_count += 1;
                }
            }
        }

        // Fix b_start for pure-delete hunks and a_start for pure-insert hunks
        if b_start == 1 {
            // try to find first b index in range
            for kind in &flat[range_start..range_end] {
                if let Kind::Eq(_, bi) = kind {
                    b_start = bi + 1;
                    break;
                }
            }
        }
        if a_start == 1 {
            for kind in &flat[range_start..range_end] {
                if let Kind::Eq(ai, _) = kind {
                    a_start = ai + 1;
                    break;
                }
            }
        }

        hunks.push(Hunk {
            a_start,
            b_start,
            lines,
            a_count,
            b_count,
        });
    }

    hunks
}

fn format_diff(label_a: &str, label_b: &str, a: &[&str], b: &[&str], ctx: usize) -> String {
    let ops = myers_diff(a, b);
    let hunks = build_hunks(&ops, a, b, ctx);

    if hunks.is_empty() {
        return "Files are identical".to_string();
    }

    let mut out = format!("--- {}\n+++ {}\n", label_a, label_b);
    for h in &hunks {
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            h.a_start, h.a_count, h.b_start, h.b_count
        ));
        for line in &h.lines {
            out.push_str(line);
            out.push('\n');
        }
    }

    if out.len() > MAX_OUTPUT {
        out.truncate(MAX_OUTPUT);
        out.push_str("\n[... diff truncated ...]");
    }
    out
}

// ── Public tool functions ────────────────────────────────────────────────────

pub async fn diff_files(args: &Value) -> String {
    let path_a_raw = match args.get("path_a").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => return "Error: missing required param 'path_a'".to_string(),
    };
    let path_b_raw = match args.get("path_b").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => return "Error: missing required param 'path_b'".to_string(),
    };
    let ctx = args
        .get("context_lines")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(3);

    // [T7f] Workspace-boundary check (PR #75 audit H-6). Before this fix
    // a model could exfiltrate `/etc/passwd` or `~/.ssh/id_rsa` simply by
    // diffing the sensitive file against any in-workspace file: the
    // returned hunk lists the sensitive file's contents as `+` lines.
    let path_a = match safe_path(&path_a_raw) {
        Ok(p) => p,
        Err(e) => return format!("Error: invalid path_a: {}", e),
    };
    let path_b = match safe_path(&path_b_raw) {
        Ok(p) => p,
        Err(e) => return format!("Error: invalid path_b: {}", e),
    };

    let content_a = match tokio::fs::read_to_string(&path_a).await {
        Ok(s) => s,
        Err(e) => return format!("Error reading '{}': {}", path_a.display(), e),
    };
    let content_b = match tokio::fs::read_to_string(&path_b).await {
        Ok(s) => s,
        Err(e) => return format!("Error reading '{}': {}", path_b.display(), e),
    };

    let lines_a: Vec<&str> = content_a.lines().collect();
    let lines_b: Vec<&str> = content_b.lines().collect();

    format_diff(
        &format!("a/{}", path_a.display()),
        &format!("b/{}", path_b.display()),
        &lines_a,
        &lines_b,
        ctx,
    )
}

pub async fn diff_strings(args: &Value) -> String {
    let a = match args.get("a").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return "Error: missing required param 'a'".to_string(),
    };
    let b = match args.get("b").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return "Error: missing required param 'b'".to_string(),
    };
    let label_a = args
        .get("label_a")
        .and_then(|v| v.as_str())
        .unwrap_or("a")
        .to_string();
    let label_b = args
        .get("label_b")
        .and_then(|v| v.as_str())
        .unwrap_or("b")
        .to_string();
    let ctx = args
        .get("context_lines")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(3);

    let lines_a: Vec<&str> = a.lines().collect();
    let lines_b: Vec<&str> = b.lines().collect();

    format_diff(&label_a, &label_b, &lines_a, &lines_b, ctx)
}
