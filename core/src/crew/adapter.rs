//! The crew integration SEAM: the `Adapter` trait every crew member is driven
//! through, its output/error types, and cross-vendor rate-limit detection.
//! Ported from the `ensemble` project (Apache-2.0) §6 crew port.
//!
//! phantom's own `GovernedCliAdapter` (a later slice) implements this trait by
//! driving codex/claude/agy/opencode through `cli_session` + `governed_run`, so
//! ensemble's conductor brain steers phantom's governed hands. The `MockAdapter`
//! here keeps the conductor's orchestration logic hermetically testable.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Mutex;
use thiserror::Error;

/// What an agent produced on one turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentOutput {
    pub agent: String,
    pub text: String,
}

/// Why an agent did NOT produce a usable answer. These are the degrade signals: the gate
/// must treat any of them as "this reviewer is unavailable", never as approval.
#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("agent flaked: {0}")]
    Flaked(String),
    #[error("agent produced empty output")]
    Empty,
    #[error("agent rate-limited / quota exhausted{0}")]
    RateLimited(RateLimitInfo),
    #[error("agent CLI not installed: {0}")]
    NotInstalled(String),
}

/// What we could recover about a rate-limit / quota-exhaustion so the operator (or a scheduler)
/// learns *why* an agent degraded and *when* it can be retried — instead of a bare "empty output".
/// Both fields are best-effort: `reason` is the offending CLI line, `retry_at` the vendor's stated
/// reset time (verbatim, e.g. "Jun 25th, 2026 5:33 AM") when the message carried one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RateLimitInfo {
    pub reason: String,
    pub retry_at: Option<String>,
}

impl std::fmt::Display for RateLimitInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.reason.is_empty() {
            write!(f, " — {}", self.reason)?;
        }
        if let Some(when) = &self.retry_at {
            write!(f, " (retry after {when})")?;
        }
        Ok(())
    }
}

/// ASCII-case-insensitive substring search returning a byte index valid in `haystack`. Vendor CLI
/// quota messages are ASCII, so this avoids the `to_lowercase()` byte-offset drift that would break
/// slicing when the message also contains multi-byte characters.
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    let (h, n) = (haystack.as_bytes(), needle.as_bytes());
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&i| {
        h[i..i + n.len()]
            .iter()
            .zip(n)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

/// Markers that identify a rate-limit / quota-exhaustion across vendors (codex, claude, opencode,
/// agy). Kept broad on purpose — "usage limit" (codex), "429"/"too many requests" (HTTP), and the
/// generic "quota"/"rate limit" cover the cases observed so far.
const QUOTA_MARKERS: &[&str] = &[
    "usage limit",
    "rate limit",
    "rate-limit",
    "ratelimit",
    "quota",
    "too many requests",
    "429",
    "insufficient_quota",
    "exceeded your",
    "over your",
];

/// Detect a rate-limit / quota signal in a CLI's output. Returns the offending line as `reason` and,
/// if the vendor stated one, the reset time as `retry_at`. `None` when no marker is present — so a
/// normal answer that never mentions quota is never misclassified.
pub fn detect_rate_limit(haystack: &str) -> Option<RateLimitInfo> {
    if !QUOTA_MARKERS.iter().any(|m| find_ci(haystack, m).is_some()) {
        return None;
    }
    let reason = haystack
        .lines()
        .find(|l| QUOTA_MARKERS.iter().any(|m| find_ci(l, m).is_some()))
        .map(|l| l.trim().to_string())
        .unwrap_or_else(|| haystack.trim().to_string());
    Some(RateLimitInfo {
        reason,
        retry_at: extract_retry_at(haystack),
    })
}

/// Pull the reset time out of a "try again at <when>" / "retry after <when>" / "resets at <when>"
/// phrase. Returns the verbatim remainder of that line (trailing period stripped), or `None`.
fn extract_retry_at(haystack: &str) -> Option<String> {
    const CUES: &[&str] = &[
        "try again at ",
        "retry after ",
        "resets at ",
        "available again at ",
    ];
    for cue in CUES {
        if let Some(pos) = find_ci(haystack, cue) {
            let rest = &haystack[pos + cue.len()..];
            let line_end = rest.find('\n').unwrap_or(rest.len());
            let val = rest[..line_end].trim().trim_end_matches('.').trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

impl AdapterError {
    /// A DISTINCT, stable process exit code per failure kind, so a conductor's shell (e.g. Claude
    /// Code's Bash tool) can branch on *why* a delegated `ensemble agent` run failed. 0=ok and
    /// 7=no-adapter-resolved are owned by main.rs and never overlap these.
    pub fn exit_code(&self) -> i32 {
        match self {
            AdapterError::Flaked(_) => 3,
            AdapterError::Empty => 4,
            AdapterError::RateLimited(_) => 5,
            AdapterError::NotInstalled(_) => 6,
        }
    }
}

/// A vendor AI CLI driven headlessly. Implementors encode the per-vendor invocation contract.
pub trait Adapter: Send + Sync {
    /// The agent's name as referenced in crew.toml (e.g. "codex", "claude").
    fn name(&self) -> &str;
    /// Run one turn: hand `prompt` to the agent with working dir `cwd`, return its reply.
    fn run(&self, prompt: &str, cwd: &Path) -> Result<AgentOutput, AdapterError>;
    /// S1b: hand the adapter a HARD-abort flag. When this flag is set DURING a `run()`, the adapter
    /// kills its child mid-turn and returns `Flaked("aborted")` instead of waiting out the turn — so
    /// `ensemble abort --hard` stops a wedged/drifting CLI immediately. Default no-op (Mock/Remote
    /// ignore it; only the real exec/PTY adapters honor it).
    fn set_abort(&self, _flag: std::sync::Arc<std::sync::atomic::AtomicBool>) {}
}

/// A scripted adapter for hermetic tests: returns successive queued responses; an exhausted
/// queue yields `AdapterError::Empty` so tests can model an agent that stops responding.
pub struct MockAdapter {
    name: String,
    responses: Mutex<VecDeque<Result<String, AdapterError>>>,
}

impl MockAdapter {
    pub fn new(name: &str, responses: Vec<Result<String, AdapterError>>) -> Self {
        Self {
            name: name.to_string(),
            responses: Mutex::new(responses.into()),
        }
    }
}

impl Adapter for MockAdapter {
    fn name(&self) -> &str {
        &self.name
    }
    fn run(&self, _prompt: &str, _cwd: &Path) -> Result<AgentOutput, AdapterError> {
        let mut q = self.responses.lock().unwrap();
        match q.pop_front() {
            Some(Ok(text)) => Ok(AgentOutput {
                agent: self.name.clone(),
                text,
            }),
            Some(Err(e)) => Err(e),
            None => Err(AdapterError::Empty),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn mock_returns_scripted_then_flakes() {
        let m = MockAdapter::new(
            "codex",
            vec![
                Ok("I implemented the change.".to_string()),
                Err(AdapterError::Empty),
            ],
        );
        assert_eq!(m.name(), "codex");
        let out = m.run("do the thing", Path::new(".")).unwrap();
        assert_eq!(out.agent, "codex");
        assert_eq!(out.text, "I implemented the change.");
        assert!(matches!(
            m.run("again", Path::new(".")),
            Err(AdapterError::Empty)
        ));
    }

    #[test]
    fn mock_exhausted_returns_empty() {
        let m = MockAdapter::new("claude", vec![]);
        assert!(matches!(m.run("x", Path::new(".")), Err(AdapterError::Empty)));
    }

    #[test]
    fn detect_rate_limit_parses_codex_usage_limit_and_reset_time() {
        // The exact line codex prints on quota exhaustion (observed 2026-06-24, M5 native run).
        let s =
            "ERROR: You've hit your usage limit. Visit https://chatgpt.com/codex/settings/usage \
                 to purchase more credits or try again at Jun 25th, 2026 5:33 AM.";
        let info = detect_rate_limit(s).expect("usage-limit line must be detected");
        assert!(info.reason.to_lowercase().contains("usage limit"));
        assert_eq!(info.retry_at.as_deref(), Some("Jun 25th, 2026 5:33 AM"));
        // and it surfaces in Display (which flows into the transcript + wire error message).
        let shown = AdapterError::RateLimited(info).to_string();
        assert!(
            shown.contains("retry after Jun 25th, 2026 5:33 AM"),
            "got {shown}"
        );
    }

    #[test]
    fn detect_rate_limit_handles_http_and_generic_phrasings() {
        assert!(detect_rate_limit("Error: 429 Too Many Requests").is_some());
        assert!(detect_rate_limit("rate limit exceeded, retry after 30s").is_some());
        let q = detect_rate_limit("You have exceeded your quota.").unwrap();
        assert_eq!(q.retry_at, None); // no reset cue present
    }

    #[test]
    fn detect_rate_limit_ignores_a_normal_answer() {
        // A real answer that never mentions quota must NOT be misread as rate-limited.
        assert!(detect_rate_limit("I implemented the change and all tests pass.").is_none());
        assert!(detect_rate_limit("").is_none());
    }

    #[test]
    fn detect_rate_limit_is_byte_safe_with_multibyte_text() {
        // Multi-byte chars before the cue must not break the byte-indexed extraction.
        let s = "額度已用完 usage limit reached — try again at 明天 5:33 AM";
        let info = detect_rate_limit(s).expect("should detect across multibyte text");
        assert_eq!(info.retry_at.as_deref(), Some("明天 5:33 AM"));
    }

    #[test]
    fn exit_code_is_total_and_distinct() {
        // Every AdapterError variant maps to a DISTINCT non-zero code so a conductor's shell can
        // branch on the failure kind. (0 = ok is owned by main.rs, not an error variant.)
        let codes = [
            AdapterError::Flaked("x".into()).exit_code(),
            AdapterError::Empty.exit_code(),
            AdapterError::RateLimited(RateLimitInfo::default()).exit_code(),
            AdapterError::NotInstalled("x".into()).exit_code(),
        ];
        assert_eq!(codes, [3, 4, 5, 6]);
        let mut sorted = codes.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len(), "exit codes must be distinct");
        assert!(codes.iter().all(|&c| c != 0 && c != 7));
    }
}
