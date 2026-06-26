//! Double-gate: ≥2 distinct AI LGTM, flake-excluded, with backup substitution.
use crate::fleet::types::Verdict;
use anyhow::Result;
use async_trait::async_trait;

const QUORUM: usize = 2;

#[async_trait]
pub trait Reviewer: Send + Sync {
    /// Ask one tool to review; return its raw stdout (may be empty on flake).
    async fn ask(&self, tool: &str, prompt: &str) -> Result<String>;
}

/// Parse a single reviewer's output into a Verdict (last VERDICT line wins).
pub fn parse_verdict(text: &str) -> Verdict {
    for line in text.lines().rev() {
        let l = line.trim();
        let up = l.to_uppercase();
        if let Some(rest) = up.strip_prefix("VERDICT:") {
            let rest = rest.trim();
            if rest.starts_with("LGTM") {
                return Verdict::Lgtm;
            }
            if rest.starts_with("CHANGES") {
                let note = l
                    .splitn(2, "CHANGES")
                    .nth(1)
                    .unwrap_or("")
                    .trim_start_matches([':', ' '])
                    .to_string();
                return Verdict::Changes(vec![if note.is_empty() { l.to_string() } else { note }]);
            }
        }
    }
    Verdict::Inconclusive
}

/// Run the double-gate. `primary` are the first-choice distinct AIs; `backups` fill in
/// (in order) for any primary that flakes (Inconclusive). CHANGES from anyone blocks.
pub async fn review(
    r: &dyn Reviewer,
    diff: &str,
    intent: &str,
    primary: &[&str],
    backups: &[&str],
) -> Verdict {
    let prompt = format!(
        "Review this change for correctness and scope. Intent: {intent}\n\n```diff\n{diff}\n```\n\
         End your reply with exactly one line: `VERDICT: LGTM` or `VERDICT: CHANGES: <reason>`."
    );

    let mut lgtm = 0usize;
    let mut used: Vec<String> = Vec::new();
    let mut queue: Vec<String> = primary.iter().map(|s| s.to_string()).collect();
    let mut backup_iter = backups.iter();

    while let Some(tool) = queue.pop() {
        if used.contains(&tool) {
            continue;
        }
        used.push(tool.clone());
        let verdict = match r.ask(&tool, &prompt).await {
            Ok(out) => parse_verdict(&out),
            Err(_) => Verdict::Inconclusive,
        };
        match verdict {
            Verdict::Changes(n) => return Verdict::Changes(n), // any CHANGES blocks
            Verdict::Lgtm => {
                lgtm += 1;
                if lgtm >= QUORUM {
                    return Verdict::Lgtm;
                }
            }
            Verdict::Inconclusive => {
                if let Some(b) = backup_iter.next() {
                    queue.push(b.to_string());
                }
            }
        }
    }
    if lgtm >= QUORUM {
        Verdict::Lgtm
    } else {
        Verdict::Inconclusive
    }
}

/// Map a reviewer process result to the text the gate parses. A nonzero exit
/// (timeout / quota / crash) yields empty -> Inconclusive, never a parseable LGTM.
fn reviewer_output(success: bool, stdout: &str) -> String {
    if success {
        stdout.to_string()
    } else {
        String::new()
    }
}

/// Real reviewer: shells to `.claude/skills/local-ai/ask.sh <tool> "<prompt>"`.
pub struct AskShReviewer {
    pub ask_sh: std::path::PathBuf,
}

#[async_trait]
impl Reviewer for AskShReviewer {
    async fn ask(&self, tool: &str, prompt: &str) -> Result<String> {
        let out = tokio::process::Command::new("bash")
            .arg(&self.ask_sh)
            .arg(tool)
            .arg(prompt)
            .output()
            .await?;
        Ok(reviewer_output(
            out.status.success(),
            &String::from_utf8_lossy(&out.stdout),
        ))
    }
}

#[cfg(test)]
pub struct MockReviewer {
    scripted: std::collections::HashMap<String, String>,
}
#[cfg(test)]
impl MockReviewer {
    pub fn new(scripted: std::collections::HashMap<String, String>) -> Self {
        Self { scripted }
    }
}
#[cfg(test)]
#[async_trait]
impl Reviewer for MockReviewer {
    async fn ask(&self, tool: &str, _prompt: &str) -> Result<String> {
        Ok(self.scripted.get(tool).cloned().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::types::Verdict;
    use std::collections::HashMap;

    #[test]
    fn parses_verdict_lines() {
        assert_eq!(parse_verdict("blah\nVERDICT: LGTM"), Verdict::Lgtm);
        match parse_verdict("VERDICT: CHANGES: fix the off-by-one") {
            Verdict::Changes(n) => assert!(n[0].contains("off-by-one")),
            o => panic!("{o:?}"),
        }
        assert_eq!(
            parse_verdict("garbage with no verdict line"),
            Verdict::Inconclusive
        );
        assert_eq!(parse_verdict(""), Verdict::Inconclusive);
    }

    #[test]
    fn nonzero_exit_reviewer_is_inconclusive_not_lgtm() {
        // even if a flaked process printed an LGTM-looking line, a nonzero exit -> empty -> Inconclusive
        assert_eq!(
            parse_verdict(&reviewer_output(false, "VERDICT: LGTM")),
            Verdict::Inconclusive
        );
        assert_eq!(
            parse_verdict(&reviewer_output(true, "VERDICT: LGTM")),
            Verdict::Lgtm
        );
    }

    #[tokio::test]
    async fn two_distinct_lgtm_passes() {
        let mut scripted = HashMap::new();
        scripted.insert("codex".to_string(), "VERDICT: LGTM".to_string());
        scripted.insert("claude".to_string(), "VERDICT: LGTM".to_string());
        let r = MockReviewer::new(scripted);
        let v = review(
            &r,
            "diff",
            "intent",
            &["codex", "claude"],
            &["opencode", "agy"],
        )
        .await;
        assert_eq!(v, Verdict::Lgtm);
    }

    #[tokio::test]
    async fn flake_substitutes_a_backup_to_fill_quorum() {
        let mut scripted = HashMap::new();
        scripted.insert("codex".to_string(), "VERDICT: LGTM".to_string());
        scripted.insert("claude".to_string(), "".to_string()); // flake (inconclusive)
        scripted.insert("opencode".to_string(), "VERDICT: LGTM".to_string()); // backup fills
        let r = MockReviewer::new(scripted);
        let v = review(
            &r,
            "diff",
            "intent",
            &["codex", "claude"],
            &["opencode", "agy"],
        )
        .await;
        assert_eq!(
            v,
            Verdict::Lgtm,
            "backup opencode substitutes for flaked claude"
        );
    }

    #[tokio::test]
    async fn changes_from_any_reviewer_blocks() {
        let mut scripted = HashMap::new();
        scripted.insert("codex".to_string(), "VERDICT: LGTM".to_string());
        scripted.insert("claude".to_string(), "VERDICT: CHANGES: nope".to_string());
        let r = MockReviewer::new(scripted);
        let v = review(
            &r,
            "diff",
            "intent",
            &["codex", "claude"],
            &["opencode", "agy"],
        )
        .await;
        matches!(v, Verdict::Changes(_))
            .then_some(())
            .expect("CHANGES must block");
    }

    #[tokio::test]
    async fn unfillable_quorum_is_inconclusive() {
        let scripted = HashMap::new(); // everyone flakes
        let r = MockReviewer::new(scripted);
        let v = review(
            &r,
            "diff",
            "intent",
            &["codex", "claude"],
            &["opencode", "agy"],
        )
        .await;
        assert_eq!(v, Verdict::Inconclusive);
    }
}
