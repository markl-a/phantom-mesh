//! PhantomAgentAdapter — phantom's OWN agent as a crew member (dogfooding). Drives
//! `phantom exec "<prompt>"` in the worktree, so phantom's tools + owned-memory +
//! learning loop participate as implementer/reviewer alongside codex/claude/agy.
//! Unlike GovernedCliAdapter, this is NOT wrapped in the crew governor: `phantom
//! exec` self-governs (its own agent loop owns tool-gating/owned-memory), so the
//! crew uses it directly.

use crate::crew::{detect_rate_limit, AdapterError, AgentOutput};
use std::path::Path;
use std::process::Command;

/// Classify `phantom exec` output into the crew result. Shares the precedence rules
/// with the cli_session fold: a quota/rate-limit line -> RateLimited; empty -> Empty;
/// otherwise the text is the agent's reply.
pub fn fold_phantom_output(agent: &str, output: &str) -> Result<AgentOutput, AdapterError> {
    if let Some(info) = detect_rate_limit(output) {
        return Err(AdapterError::RateLimited(info));
    }
    if output.trim().is_empty() {
        return Err(AdapterError::Empty);
    }
    Ok(AgentOutput {
        agent: agent.to_string(),
        text: output.to_string(),
    })
}

/// Decide the crew result from a full `phantom exec` run (exit status + both streams).
/// Precedence, so a benign stderr diagnostic can't discard a real answer:
/// 1. A successful run with a non-empty stdout answer IS the reply — only stdout is
///    classified, so an unrelated stderr line mentioning "429"/"quota" cannot turn a
///    good review into a false RateLimited (which would drop phantom's vote).
/// 2. Otherwise (empty stdout or non-zero exit) the signal is in stderr: scan both
///    streams so a real quota/rate-limit line surfaces as RateLimited, not a bare Empty.
pub fn classify_phantom_run(
    agent: &str,
    success: bool,
    stdout: &str,
    stderr: &str,
) -> Result<AgentOutput, AdapterError> {
    if success && !stdout.trim().is_empty() {
        // A rate-limit explicitly on the answer stream is still honored; stderr noise isn't.
        return fold_phantom_output(agent, stdout);
    }
    fold_phantom_output(agent, &format!("{stdout}\n{stderr}"))
}

pub struct PhantomAgentAdapter {
    name: String,
    /// Advisory only: `phantom exec` self-governs its own wall-clock; retained so a
    /// future flag (e.g. an external kill timer) can use it. NOT passed to phantom.
    timeout_secs: u64,
}

impl PhantomAgentAdapter {
    pub fn new(name: impl Into<String>, timeout_secs: u64) -> Self {
        Self {
            name: name.into(),
            timeout_secs,
        }
    }
}

impl crate::crew::Adapter for PhantomAgentAdapter {
    fn name(&self) -> &str {
        &self.name
    }
    fn run(&self, prompt: &str, cwd: &Path) -> Result<AgentOutput, AdapterError> {
        let _ = self.timeout_secs; // see field doc: phantom exec self-governs; not passed.
        let out = Command::new("phantom")
            .arg("exec")
            .arg(prompt)
            .current_dir(cwd)
            .output()
            .map_err(|e| AdapterError::NotInstalled(format!("phantom exec: {e}")))?;
        let text = String::from_utf8_lossy(&out.stdout);
        let err = String::from_utf8_lossy(&out.stderr);
        classify_phantom_run(&self.name, out.status.success(), &text, &err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crew::AdapterError;

    #[test]
    fn fold_phantom_stdout_into_reply_or_empty() {
        let ok = fold_phantom_output("phantom", "VERDICT: LGTM\n").expect("text");
        assert_eq!(ok.agent, "phantom");
        assert_eq!(ok.text.trim(), "VERDICT: LGTM");
        assert!(matches!(
            fold_phantom_output("phantom", "   "),
            Err(AdapterError::Empty)
        ));
        assert!(matches!(
            fold_phantom_output("phantom", "error: rate limit exceeded"),
            Err(AdapterError::RateLimited(_))
        ));
    }

    #[test]
    fn classify_prefers_stdout_answer_over_benign_stderr_noise() {
        // A good answer on stdout (exit 0) must not be discarded because stderr
        // happens to mention a quota-ish token like "429" (M1: false RateLimited).
        let ok = classify_phantom_run(
            "phantom",
            true,
            "VERDICT: LGTM\n",
            "warn: upstream returned 429 earlier; retried ok\n",
        )
        .expect("good stdout answer wins over stderr noise");
        assert_eq!(ok.text.trim(), "VERDICT: LGTM");
    }

    #[test]
    fn classify_surfaces_rate_limit_when_run_failed() {
        // Empty stdout + a real quota line on stderr + non-zero exit -> RateLimited,
        // not a bare Empty (so the operator learns WHY phantom degraded).
        assert!(matches!(
            classify_phantom_run("phantom", false, "", "You have exceeded your quota.\n"),
            Err(AdapterError::RateLimited(_))
        ));
        // Nothing on either stream -> Empty.
        assert!(matches!(
            classify_phantom_run("phantom", false, "", ""),
            Err(AdapterError::Empty)
        ));
        // A throttle printed on the answer stream itself is still honored.
        assert!(matches!(
            classify_phantom_run("phantom", true, "rate limit exceeded", ""),
            Err(AdapterError::RateLimited(_))
        ));
    }
}
