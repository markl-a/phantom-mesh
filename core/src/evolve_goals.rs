//! `EVOLVE-GOALS.md` parser + advancer.
//!
//! Lets autoevolve work from a human-curated checklist instead of just
//! "fix the next failing test". The format is intentionally Markdown so a
//! human can edit it in any editor without needing the CLI.
//!
//! Format:
//!
//! ```text
//! # Evolve Goals
//!
//! ## Pending
//! - [ ] Add cluster_secret rotation endpoint
//! - [ ] Make `phantom evolve list` accept `--json`
//!
//! ## Done
//! - [x] (2026-04-30 sha=abc1234) Phase 2 mesh handoff
//! ```
//!
//! Sections after `## Pending` and `## Done` are H2 headings. Lines that
//! don't match `- [ ] ...` / `- [x] ...` are preserved verbatim — the file
//! is round-trip safe so humans can leave notes between goals.

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalSection {
    Pending,
    Done,
    /// Anything outside the two known H2 sections — preamble, custom
    /// sections, free-form notes — gets passed through unchanged.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalLine {
    /// 0-based line index in the source file.
    pub idx: usize,
    /// Verbatim line contents (without trailing `\n`).
    pub raw: String,
    /// `Some(text)` if this is a `- [ ] text` checkbox; None otherwise.
    /// Keeps `Other` lines unparsed.
    pub checkbox: Option<Checkbox>,
    pub section: GoalSection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkbox {
    pub checked: bool,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct GoalsFile {
    pub path: PathBuf,
    pub lines: Vec<GoalLine>,
}

impl GoalsFile {
    /// Load and parse a goals file. Missing file returns an empty document
    /// rather than an error so a fresh repo can start with `phantom evolve
    /// goals add ...` and the file is created on first save.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(anyhow!("read {}: {}", path.display(), e)),
        };
        Ok(Self::parse(path, &text))
    }

    fn parse(path: PathBuf, text: &str) -> Self {
        let mut section = GoalSection::Other;
        let lines = text
            .lines()
            .enumerate()
            .map(|(idx, raw)| {
                let trimmed = raw.trim();
                // H2 transitions. We only treat exact "## Pending" / "## Done"
                // as section markers; "## Pending (later)" stays in Other.
                if trimmed.eq_ignore_ascii_case("## pending") {
                    section = GoalSection::Pending;
                } else if trimmed.eq_ignore_ascii_case("## done") {
                    section = GoalSection::Done;
                } else if trimmed.starts_with("# ") || trimmed.starts_with("## ") {
                    section = GoalSection::Other;
                }

                let checkbox = parse_checkbox(raw);
                GoalLine {
                    idx,
                    raw: raw.to_string(),
                    checkbox,
                    section: section.clone(),
                }
            })
            .collect();
        Self { path, lines }
    }

    /// First pending unchecked goal. None when the queue is drained.
    pub fn next_pending(&self) -> Option<&GoalLine> {
        self.lines.iter().find(|l| {
            l.section == GoalSection::Pending
                && l.checkbox.as_ref().map(|c| !c.checked).unwrap_or(false)
        })
    }

    pub fn pending_count(&self) -> usize {
        self.lines.iter().filter(|l| {
            l.section == GoalSection::Pending
                && l.checkbox.as_ref().map(|c| !c.checked).unwrap_or(false)
        }).count()
    }

    pub fn done_count(&self) -> usize {
        self.lines.iter().filter(|l| {
            l.section == GoalSection::Done
                && l.checkbox.as_ref().map(|c| c.checked).unwrap_or(false)
        }).count()
    }

    /// Mark the pending goal at `line_idx` as done. Moves it to the `## Done`
    /// section with `(YYYY-MM-DD sha=…) ` prefix so `git log` and the file
    /// agree on what shipped when. Returns the marked goal text for callers
    /// that want to embed it in commit messages.
    ///
    /// `commit_sha` is short-form (7 chars) for readability — pass `"pending"`
    /// when called pre-commit and rewrite afterward.
    pub fn mark_done(
        &mut self,
        line_idx: usize,
        date_yyyymmdd: &str,
        commit_sha: &str,
    ) -> Result<String> {
        let pos = self
            .lines
            .iter()
            .position(|l| l.idx == line_idx)
            .ok_or_else(|| anyhow!("line idx {} not found", line_idx))?;

        let original_text = match &self.lines[pos].checkbox {
            Some(cb) if !cb.checked => cb.text.clone(),
            Some(_) => return Err(anyhow!("line {} already checked", line_idx)),
            None => return Err(anyhow!("line {} is not a checkbox", line_idx)),
        };

        // Remove from Pending section, insert at the top of Done section.
        self.lines.remove(pos);
        let done_anchor = self
            .lines
            .iter()
            .position(|l| l.raw.trim().eq_ignore_ascii_case("## done"));
        let new_line_raw = format!(
            "- [x] ({} sha={}) {}",
            date_yyyymmdd, commit_sha, original_text
        );
        let new_line = GoalLine {
            idx: usize::MAX, // recomputed by renumber
            raw: new_line_raw.clone(),
            checkbox: Some(Checkbox {
                checked: true,
                text: format!("({} sha={}) {}", date_yyyymmdd, commit_sha, original_text),
            }),
            section: GoalSection::Done,
        };

        match done_anchor {
            Some(i) => self.lines.insert(i + 1, new_line),
            None => {
                // No ## Done section — append one at end.
                self.lines.push(GoalLine {
                    idx: usize::MAX,
                    raw: String::new(),
                    checkbox: None,
                    section: GoalSection::Other,
                });
                self.lines.push(GoalLine {
                    idx: usize::MAX,
                    raw: "## Done".into(),
                    checkbox: None,
                    section: GoalSection::Done,
                });
                self.lines.push(new_line);
            }
        }
        self.renumber();
        Ok(original_text)
    }

    fn renumber(&mut self) {
        for (i, l) in self.lines.iter_mut().enumerate() {
            l.idx = i;
        }
    }

    pub fn to_text(&self) -> String {
        let mut s = self
            .lines
            .iter()
            .map(|l| l.raw.clone())
            .collect::<Vec<_>>()
            .join("\n");
        // Keep trailing newline if the original had one — easier on tools
        // like `wc -l` and most editors.
        if !s.ends_with('\n') {
            s.push('\n');
        }
        s
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(&self.path, self.to_text())?;
        Ok(())
    }

    /// Append a new unchecked goal under `## Pending`. Creates the section
    /// if it doesn't exist yet. The caller is responsible for calling `save()`
    /// after adding one or more goals.
    pub fn add_pending(&mut self, text: &str) {
        let new_line_raw = format!("- [ ] {}", text);
        let new_line = GoalLine {
            idx: usize::MAX,
            raw: new_line_raw.clone(),
            checkbox: Some(Checkbox {
                checked: false,
                text: text.to_string(),
            }),
            section: GoalSection::Pending,
        };

        // Insert after the last line that belongs to Pending.
        let anchor = self.lines.iter().rposition(|l| l.section == GoalSection::Pending);
        match anchor {
            Some(i) => self.lines.insert(i + 1, new_line),
            None => {
                // No ## Pending section — create one at end of file.
                self.lines.push(GoalLine {
                    idx: usize::MAX,
                    raw: String::new(),
                    checkbox: None,
                    section: GoalSection::Other,
                });
                self.lines.push(GoalLine {
                    idx: usize::MAX,
                    raw: "## Pending".into(),
                    checkbox: None,
                    section: GoalSection::Pending,
                });
                self.lines.push(new_line);
            }
        }
        self.renumber();
    }

    /// Collect all pending checkbox texts into a `Vec<String>`.
    pub fn pending_goals(&self) -> Vec<String> {
        self.lines
            .iter()
            .filter(|l| {
                l.section == GoalSection::Pending
                    && l.checkbox.as_ref().map(|c| !c.checked).unwrap_or(false)
            })
            .map(|l| l.checkbox.as_ref().unwrap().text.clone())
            .collect()
    }

    /// Collect all done checkbox texts into a `Vec<String>`.
    pub fn done_goals(&self) -> Vec<String> {
        self.lines
            .iter()
            .filter(|l| {
                l.section == GoalSection::Done
                    && l.checkbox.as_ref().map(|c| c.checked).unwrap_or(false)
            })
            .map(|l| l.checkbox.as_ref().unwrap().text.clone())
            .collect()
    }
}

/// Parse `- [ ] text` / `- [x] text` (case-insensitive on x) with optional
/// leading whitespace. Returns None for any other line shape so non-goal
/// content survives a round trip unchanged.
fn parse_checkbox(raw: &str) -> Option<Checkbox> {
    let s = raw.trim_start();
    if !s.starts_with("- [") {
        return None;
    }
    let bytes = s.as_bytes();
    if bytes.len() < 5 || bytes[4] != b']' {
        return None;
    }
    let marker = bytes[3];
    let checked = matches!(marker, b'x' | b'X');
    if !checked && marker != b' ' {
        return None;
    }
    // Skip past `- [.] ` (6 chars including the space).
    let rest = s.get(5..)?.trim_start();
    Some(Checkbox {
        checked,
        text: rest.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# Evolve Goals

Notes about the project go here. They survive a round trip.

## Pending
- [ ] First goal: do thing X
- [ ] Second goal: do thing Y

## Done
- [x] (2026-04-29 sha=abc1234) Phase 1 checkpoint module
";

    #[test]
    fn parses_pending_and_done_sections() {
        let g = GoalsFile::parse(PathBuf::from("test.md"), SAMPLE);
        assert_eq!(g.pending_count(), 2);
        assert_eq!(g.done_count(), 1);
    }

    #[test]
    fn next_pending_returns_first_unchecked() {
        let g = GoalsFile::parse(PathBuf::from("test.md"), SAMPLE);
        let next = g.next_pending().expect("should have a pending");
        assert_eq!(
            next.checkbox.as_ref().unwrap().text,
            "First goal: do thing X"
        );
    }

    #[test]
    fn mark_done_moves_line_into_done_section() {
        let mut g = GoalsFile::parse(PathBuf::from("test.md"), SAMPLE);
        let line_idx = g.next_pending().unwrap().idx;
        let text = g.mark_done(line_idx, "2026-04-30", "deadbee").unwrap();
        assert_eq!(text, "First goal: do thing X");
        assert_eq!(g.pending_count(), 1);
        assert_eq!(g.done_count(), 2);
        // The newly checked entry is the first under Done.
        let done_first = g
            .lines
            .iter()
            .filter(|l| l.section == GoalSection::Done && l.checkbox.is_some())
            .next()
            .unwrap();
        assert!(done_first.raw.contains("(2026-04-30 sha=deadbee) First goal"));
    }

    #[test]
    fn round_trip_preserves_non_goal_content() {
        let g = GoalsFile::parse(PathBuf::from("test.md"), SAMPLE);
        let out = g.to_text();
        assert!(out.contains("Notes about the project go here."));
        assert!(out.contains("- [ ] First goal: do thing X"));
        assert!(out.contains("- [x] (2026-04-29 sha=abc1234) Phase 1"));
    }

    #[test]
    fn marking_already_checked_line_errors() {
        let mut g = GoalsFile::parse(PathBuf::from("test.md"), SAMPLE);
        let done_idx = g
            .lines
            .iter()
            .find(|l| {
                l.section == GoalSection::Done
                    && l.checkbox.as_ref().map(|c| c.checked).unwrap_or(false)
            })
            .unwrap()
            .idx;
        let err = g.mark_done(done_idx, "2026-04-30", "x").unwrap_err();
        assert!(err.to_string().contains("already checked"));
    }

    #[test]
    fn missing_file_loads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("does-not-exist.md");
        let g = GoalsFile::load(&p).unwrap();
        assert_eq!(g.pending_count(), 0);
        assert_eq!(g.done_count(), 0);
        assert!(g.next_pending().is_none());
    }

    #[test]
    fn save_then_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("goals.md");
        let mut g = GoalsFile::parse(p.clone(), SAMPLE);
        g.path = p.clone();
        let line_idx = g.next_pending().unwrap().idx;
        g.mark_done(line_idx, "2026-04-30", "feedb33").unwrap();
        g.save().unwrap();
        let g2 = GoalsFile::load(&p).unwrap();
        assert_eq!(g2.pending_count(), 1);
        assert_eq!(g2.done_count(), 2);
    }

    #[test]
    fn checkbox_parser_ignores_non_checkbox_lines() {
        assert!(parse_checkbox("plain text").is_none());
        assert!(parse_checkbox("- not a checkbox").is_none());
        assert!(parse_checkbox("- [y] wrong marker").is_none());
        assert!(parse_checkbox("  - [ ] indented").is_some());
        assert!(parse_checkbox("- [X] capital X").unwrap().checked);
    }

    #[test]
    fn nested_h2_outside_known_sections_is_other() {
        let src = "## Custom\n- [ ] not-pending\n## Pending\n- [ ] real-pending\n";
        let g = GoalsFile::parse(PathBuf::from("t.md"), src);
        assert_eq!(g.pending_count(), 1);
        let p = g.next_pending().unwrap();
        assert_eq!(p.checkbox.as_ref().unwrap().text, "real-pending");
    }

    #[test]
    fn add_pending_appends_under_existing_pending_section() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("goals.md");
        let mut g = GoalsFile::parse(p.clone(), SAMPLE);
        g.path = p.clone();
        assert_eq!(g.pending_count(), 2);

        g.add_pending("Third goal: do thing Z");
        g.save().unwrap();

        let g2 = GoalsFile::load(&p).unwrap();
        assert_eq!(g2.pending_count(), 3);
        assert_eq!(g2.next_pending().unwrap().checkbox.as_ref().unwrap().text,
                   "First goal: do thing X");
        // The new entry is last in Pending.
        let all: Vec<_> = g2.lines.iter()
            .filter(|l| l.section == GoalSection::Pending && l.checkbox.is_some())
            .collect();
        assert_eq!(all.last().unwrap().checkbox.as_ref().unwrap().text,
                   "Third goal: do thing Z");
    }

    #[test]
    fn add_pending_creates_pending_section_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("goals.md");
        let mut g = GoalsFile::parse(p.clone(), "No sections here.\n");
        g.path = p.clone();
        assert_eq!(g.pending_count(), 0);

        g.add_pending("First goal ever");
        g.save().unwrap();

        let g2 = GoalsFile::load(&p).unwrap();
        assert_eq!(g2.pending_count(), 1);
        assert_eq!(g2.next_pending().unwrap().checkbox.as_ref().unwrap().text,
                   "First goal ever");
    }

    #[test]
    fn pending_goals_and_done_goals_return_texts() {
        let g = GoalsFile::parse(PathBuf::from("t.md"), SAMPLE);
        assert_eq!(g.pending_goals(), &["First goal: do thing X", "Second goal: do thing Y"]);
        // The done entry includes the timestamp/sha prefix added at parse time.
        assert_eq!(g.done_goals(), &["(2026-04-29 sha=abc1234) Phase 1 checkpoint module"]);
    }

    #[test]
    fn save_add_load_round_trip() {
        // Verifies: load → add → save → load → next_pending matches the new entry.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("goals.md");
        let mut g = GoalsFile::load(&p).unwrap(); // empty file
        g.path = p.clone();
        g.add_pending("Add JSON output to goals list");
        g.save().unwrap();

        let g2 = GoalsFile::load(&p).unwrap();
        assert_eq!(g2.pending_count(), 1);
        assert_eq!(g2.next_pending().unwrap().checkbox.as_ref().unwrap().text,
                   "Add JSON output to goals list");
    }

    #[test]
    fn list_json_flag_produces_json_struct() {
        // Test that pending_goals/done_goals produce data that serialises
        // to the shape `{"pending":[...],"done":[...]}` — the binary's --json
        // flag uses the same method under the hood.
        let g = GoalsFile::parse(PathBuf::from("t.md"), SAMPLE);
        let json = format!("{{\"pending\":{},\"done\":{}}}",
                           serde_json::to_string(&g.pending_goals()).unwrap(),
                           serde_json::to_string(&g.done_goals()).unwrap());
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["pending"].as_array().unwrap().len(), 2);
        assert_eq!(parsed["done"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["pending"][0], "First goal: do thing X");
    }
}
