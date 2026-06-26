//! The automated test gate (firewall A): run a project's real test command in the worktree; GREEN
//! (exit 0) unlocks the AI reviewers, RED bounces the traceback back to the implementer. A test that
//! can't even be RUN is treated as RED (fail-closed — never let an un-runnable suite look green).
//! Ported from the `ensemble` project (Apache-2.0) §6 crew port.
//!
//! No per-command timeout here: `Command::output()` spawns, waits, and drains both pipes
//! concurrently (the std idiom — no pipe-buffer deadlock). A robust hard timeout needs process-tree
//! termination (Unix process-group / Windows job object) to avoid a killed shell's grandchildren
//! keeping the pipes open; that is a documented follow-up. The gate-level `max_task_secs` (firewall
//! B) bounds the overall task at round boundaries.

use std::path::Path;
use std::process::Command;

pub struct TestOutcome {
    pub passed: bool,
    pub output: String, // excerpted combined stdout+stderr (the tail — failures live at the end)
}

#[cfg(windows)]
fn shell(command: &str) -> Command {
    let mut c = Command::new("cmd");
    c.arg("/C").arg(command);
    c
}
#[cfg(not(windows))]
fn shell(command: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(command);
    c
}

/// Run `command` in `worktree`. GREEN on exit 0; RED on a non-zero exit OR a spawn/run failure
/// (fail-closed — an un-runnable suite must never look green).
pub fn run_tests(worktree: &Path, command: &str) -> TestOutcome {
    let out = shell(command)
        .current_dir(worktree)
        .stdin(std::process::Stdio::null())
        .output(); // spawns + waits + drains stdout/stderr concurrently (no pipe deadlock)
    match out {
        Ok(o) => {
            let mut combined = String::from_utf8_lossy(&o.stdout).into_owned();
            combined.push_str(&String::from_utf8_lossy(&o.stderr));
            TestOutcome {
                passed: o.status.success(),
                output: tail(&combined, 2000),
            }
        }
        Err(e) => TestOutcome {
            passed: false,
            output: format!("could not run test command `{command}`: {e}"),
        },
    }
}

/// Keep the LAST `max` chars (test failures/tracebacks are at the end of the output).
fn tail(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        s.to_string()
    } else {
        let skip = n - max;
        format!("…{}", s.chars().skip(skip).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn green_on_exit_zero() {
        let dir = tempfile::tempdir().unwrap();
        assert!(run_tests(dir.path(), "exit 0").passed);
    }

    #[test]
    fn red_on_exit_one() {
        // `exit 1` is non-zero in both `sh` and `cmd` → cross-platform RED.
        let dir = tempfile::tempdir().unwrap();
        assert!(!run_tests(dir.path(), "exit 1").passed);
    }

    #[cfg(unix)]
    #[test]
    fn red_captures_stderr_output() {
        let dir = tempfile::tempdir().unwrap();
        let t = run_tests(dir.path(), "echo boom 1>&2; exit 1");
        assert!(!t.passed);
        assert!(t.output.contains("boom"));
    }

    #[cfg(unix)]
    #[test]
    fn large_output_does_not_deadlock() {
        // ~200KB of output (well over the pipe buffer) that exits 0 — `output()` drains concurrently
        // so this can't deadlock on a full pipe.
        let dir = tempfile::tempdir().unwrap();
        let t = run_tests(
            dir.path(),
            "for i in $(seq 1 8000); do echo xxxxxxxxxxxxxxxxxxxxxxxx; done; exit 0",
        );
        assert!(t.passed);
    }
}
