//! Nudge cooldown ledger — the "don't re-nag" half of the sense→learn→nudge
//! loop (capability ③, spec `desktop-nudge`/N3).
//!
//! When `spectyn coach review --notify` finds a goal behind target, it fires a
//! real desktop banner (see [`crate::life_node::daily_review`]). Without a
//! memory of what we already nudged, re-running the review (or the SPEC-23
//! scheduler firing again) would re-pop the SAME banner and train the user to
//! ignore us. This ledger records each fired nudge so a per-tag cooldown can
//! suppress a repeat within the cooldown window.
//!
//! Storage is an append-only JSON-lines ledger at `~/.spectyn-mesh/nudges.jsonl`,
//! mirroring the `goals.jsonl` / `partner-signals.jsonl` convention in
//! [`crate::life_node::goals`] / [`crate::partner`]: no DB to provision, survives
//! a crash mid-write (the worst case is one truncated trailing line, which the
//! reader skips), and is trivially greppable.
//!
//! The testable core ([`should_nudge_in`] / [`record_nudge_in`]) is BOTH
//! path-injectable (caller passes the `.spectyn-mesh` dir) AND clock-injectable
//! (caller passes `now_secs`) so the hermetic test is fully deterministic — the
//! core never reads the wall clock or the real home dir. The thin home-dir
//! wrappers ([`should_nudge`] / [`record_nudge`]) resolve `~/.spectyn-mesh` and
//! the real `SystemTime::now()` for production.

use std::fs::OpenOptions;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Nudge cooldown window in seconds (30 minutes). A nudge for a given tag is
/// suppressed if an earlier nudge for the same tag was recorded within this many
/// seconds of "now".
pub const NUDGE_COOLDOWN_SECS: u64 = 1800;

/// Ledger file name under the `.spectyn-mesh` home dir.
const NUDGES_FILE: &str = "nudges.jsonl";

/// One recorded nudge: which goal `tag` we nudged about, and WHEN (unix seconds).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NudgeRecord {
    pub tag: String,
    pub ts_secs: u64,
}

/// Path of the nudges JSONL ledger: `~/.spectyn-mesh/nudges.jsonl`. Resolved
/// from `$HOME` (via `dirs::home_dir`) so a temp `$HOME` would isolate it; the
/// hermetic test drives the path-injectable cores directly and never touches the
/// real home dir. Mirrors `goals::goals_jsonl_path`.
pub fn nudges_jsonl_path() -> PathBuf {
    crate::cli_config::spectyn_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(NUDGES_FILE)
}

/// Read every nudge record from the ledger inside `dir`. A missing file yields
/// an empty Vec (nothing nudged yet). Truncated / malformed trailing lines (a
/// crash mid-write) are skipped, not fatal — same forgiving read as the goals
/// ledger.
fn read_records_in(dir: &Path) -> Vec<NudgeRecord> {
    let path = dir.join(NUDGES_FILE);
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let reader = std::io::BufReader::new(file);
    let mut out = Vec::new();
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(rec) = serde_json::from_str::<NudgeRecord>(trimmed) {
            out.push(rec);
        }
    }
    out
}

/// Path- and clock-injectable core: return `true` unless a nudge for `tag` was
/// recorded within the last [`NUDGE_COOLDOWN_SECS`] seconds of `now_secs`.
///
/// `now_secs` is passed in (NOT read from `SystemTime`) so the cooldown logic is
/// deterministically testable. The most-recent matching record decides: if any
/// record for `tag` lies within `[now_secs - cooldown, now_secs]`, the nudge is
/// on cooldown → returns `false`. A future-dated record (clock skew) is treated
/// as "within cooldown" too (still inside the window from now's perspective),
/// which fails safe toward NOT re-nagging.
pub fn should_nudge_in(dir: &Path, tag: &str, now_secs: u64) -> bool {
    let cutoff = now_secs.saturating_sub(NUDGE_COOLDOWN_SECS);
    let on_cooldown = read_records_in(dir)
        .into_iter()
        .filter(|r| r.tag == tag)
        .any(|r| r.ts_secs >= cutoff);
    !on_cooldown
}

/// Path-injectable core: append one nudge record for `tag` at `now_secs` to the
/// ledger inside `dir` (creating `dir` and any parents). One record per line,
/// serialized as a JSON object. Append-only — a fresh nudge adds a line, it does
/// not rewrite the file.
pub fn record_nudge_in(dir: &Path, tag: &str, now_secs: u64) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(NUDGES_FILE);
    let rec = NudgeRecord {
        tag: tag.to_string(),
        ts_secs: now_secs,
    };
    let line = serde_json::to_string(&rec)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(f, "{}", line)?;
    Ok(())
}

/// Thin home-dir wrapper over [`should_nudge_in`], resolving `~/.spectyn-mesh`
/// and the real wall clock. Used by production callers (the coach review notify
/// path). On a system with no resolvable home dir the cores still work against
/// `./.spectyn-mesh`.
pub fn should_nudge(tag: &str, now_secs: u64) -> bool {
    let dir = nudges_jsonl_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    should_nudge_in(&dir, tag, now_secs)
}

/// Thin home-dir wrapper over [`record_nudge_in`].
pub fn record_nudge(tag: &str, now_secs: u64) -> std::io::Result<()> {
    let dir = nudges_jsonl_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    record_nudge_in(&dir, tag, now_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// HERMETIC (real-path, no mock): a real tempdir holds a real `nudges.jsonl`.
    /// Record a "focus" nudge at t=1000, then:
    ///   - at t=1000+600 (10 min, inside the 30-min cooldown) → must NOT re-nudge.
    ///   - at t=1000+1860 (31 min, past the cooldown) → must re-nudge.
    /// `now_secs` is injected so the assertion is deterministic with no sleeps.
    #[test]
    fn cooldown_suppresses_within_window_and_lifts_after() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".spectyn-mesh");

        // Fresh ledger: nothing recorded yet → free to nudge.
        assert!(
            should_nudge_in(&dir, "focus", 1000),
            "no prior nudge → should be allowed to nudge"
        );

        // Record a real nudge at t=1000 to a real file on disk.
        record_nudge_in(&dir, "focus", 1000).expect("record nudge");
        assert!(
            dir.join("nudges.jsonl").exists(),
            "ledger file must exist on disk after record"
        );

        // 10 minutes later (1000 + 600 = 1600): inside the 30-min cooldown.
        assert!(
            !should_nudge_in(&dir, "focus", 1000 + 600),
            "10 min after a nudge is inside the 30-min cooldown → must NOT re-nudge"
        );

        // 31 minutes later (1000 + 1860 = 2860): past the cooldown.
        assert!(
            should_nudge_in(&dir, "focus", 1000 + 1860),
            "31 min after a nudge is past the 30-min cooldown → must re-nudge"
        );
    }

    /// The cooldown is PER-TAG: a recorded "focus" nudge must not silence a
    /// different goal ("reading") that is also behind in the same review.
    #[test]
    fn cooldown_is_per_tag() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".spectyn-mesh");
        record_nudge_in(&dir, "focus", 1000).expect("record focus nudge");
        assert!(
            !should_nudge_in(&dir, "focus", 1000 + 600),
            "focus on cooldown"
        );
        assert!(
            should_nudge_in(&dir, "reading", 1000 + 600),
            "a different tag is unaffected by focus's cooldown"
        );
    }
}
