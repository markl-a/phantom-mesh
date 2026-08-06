//! CrewExecutor — the fleet Executor that runs the crew conductor (`spectyn crew`)
//! per task in the task's isolated worktree, capturing the resulting diff + a build
//! check into an ExecOutcome the fleet gate/landing consume. The crew does the
//! per-task multi-AI implement+review (codex/claude/agy + the spectyn member); the
//! FLEET gate then independently reviews the diff and lands it (the double gate).

use crate::fleet::types::{BacklogTask, ExecOutcome};
use anyhow::Result;
use std::path::Path;
use std::process::Command;

/// Pure assembly of the fleet ExecOutcome from a crew run's facts.
/// - A run that did NOT land offers NO diff (the gate must never land un-gated work)
///   and is marked build-not-ok so the conductor bumps the CHANGES round / parks it
///   rather than "landing" an empty diff.
/// - A landed run carries its captured diff; build_ok tracks the post-run build check.
pub fn assemble_outcome(crew_landed: bool, diff: String, build_rc: i32, logs: String) -> ExecOutcome {
    ExecOutcome {
        diff: if crew_landed { diff } else { String::new() },
        build_ok: crew_landed && build_rc == 0,
        logs,
    }
}

pub struct CrewExecutor {
    /// e.g. "cargo build --lib" — the build check run after the crew round (in the
    /// worktree). Empty string => skip the build check (treated as rc 0).
    pub build_cmd: String,
    pub timeout_secs: u64,
}

#[async_trait::async_trait]
impl crate::fleet::executor::Executor for CrewExecutor {
    async fn run(&self, task: &BacklogTask, worktree: &Path) -> Result<ExecOutcome> {
        // NOTE: this uses sync `std::process::Command` inside an async fn, which blocks the
        // executor's task thread for the duration of the crew round. Acceptable for v1 (the
        // current driver processes tasks sequentially); a `tokio::process`/`spawn_blocking`
        // refactor is a documented follow-up.

        // 1. Run the crew round (governed by default; default crew incl. the spectyn member
        //    when the crew.toml names it). Exits 0 on LAND, 1 on escalate.
        let crew = Command::new("spectyn")
            .arg("crew")
            .arg(&task.acceptance)
            .current_dir(worktree)
            .env("SPECTYN_CREW_TIMEOUT_SECS", self.timeout_secs.to_string())
            .output()?;
        let landed = crew.status.success();
        let mut logs = format!(
            "{}{}",
            String::from_utf8_lossy(&crew.stdout),
            String::from_utf8_lossy(&crew.stderr)
        );
        if !landed {
            // Escalated/failed crew: no diff offered, build not ok -> conductor bumps/parks.
            return Ok(assemble_outcome(false, String::new(), 1, logs));
        }
        // 2. Capture the worktree diff the fleet gate will review (incl. new files).
        let captured = crate::fleet::executor::capture_review_diff(worktree)?;
        logs.push_str(&captured.logs);
        // 3. Build check (only meaningful when the crew landed changes).
        let parts: Vec<&str> = self.build_cmd.split_whitespace().collect();
        let build_rc = if parts.is_empty() {
            0
        } else {
            Command::new(parts[0])
                .args(&parts[1..])
                .current_dir(worktree)
                .status()?
                .code()
                .unwrap_or(1)
        };
        Ok(assemble_outcome(true, captured.diff, build_rc, logs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_outcome_marks_build_ok_on_zero_and_carries_diff() {
        let ok = assemble_outcome(true, "diff --git a b".into(), 0, "transcript".into());
        assert!(ok.build_ok);
        assert_eq!(ok.diff, "diff --git a b");

        let red = assemble_outcome(true, "d".into(), 1, "t".into());
        assert!(!red.build_ok, "non-zero build rc -> build_ok false");

        // A crew that did NOT land yields an empty diff AND build_ok=false regardless.
        let noland = assemble_outcome(false, "d".into(), 0, "t".into());
        assert_eq!(noland.diff, "", "no-land -> no diff offered to the gate");
        assert!(!noland.build_ok, "no-land -> not build_ok (conductor bumps/parks)");
    }
}
