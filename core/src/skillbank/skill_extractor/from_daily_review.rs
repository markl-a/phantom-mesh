//! Extract skill candidates from a daily review markdown brief
//! (the output of `spectyn coach review --date YYYY-MM-DD --save`).
//!
//! Contract with E003's `daily_review::aggregate`:
//!   - `# Daily review — YYYY-MM-DD` heading
//!   - `**Events captured:** N` line
//!   - one `## <goal_tag> (count)` section per distinct tag
//!   - bullet lines `- **<kind>** (<timestamp>): <analysis summary>`
//!
//! One `SkillCandidate` is emitted per goal-tag section (skipping
//! `untagged` — that's noise, not a skill).
//!
//! This file is the input-side of the skillbank 6-step loop
//! (judge → extract → store → recall → apply → measure). The judge
//! (Curator V2 ensemble) lives in `skillbank::curator`; this file is the
//! extract step.

use super::{Provenance, SkillCandidate};

/// Parse a daily-review markdown brief and return one candidate per
/// non-`untagged` goal-tag section. `date` should match the brief
/// heading; pass it explicitly rather than re-parsing the heading so
/// callers can keep canonicalisation in one place.
pub fn extract_from_review_markdown(content: &str, date: &str) -> Vec<SkillCandidate> {
    let mut candidates = Vec::new();
    let mut current_section: Option<SectionState> = None;

    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            // Flush any previous section before starting a new one.
            if let Some(s) = current_section.take() {
                if let Some(c) = s.into_candidate(date) {
                    candidates.push(c);
                }
            }
            // Parse `<goal_tag> (count)` — count is informational, kept for
            // provenance. Anything that doesn't match the shape is ignored.
            let (tag, count) = split_tag_count(rest);
            if tag == "untagged" {
                current_section = None; // skip noise
                continue;
            }
            current_section = Some(SectionState::new(tag.to_string(), count));
        } else if let Some(state) = current_section.as_mut() {
            // Body line of the current section.
            if line.starts_with("- ") {
                state.absorb_bullet(line);
            } else if !line.is_empty() {
                state.body.push_str(line);
                state.body.push('\n');
            }
        }
    }
    // Flush the final section.
    if let Some(s) = current_section {
        if let Some(c) = s.into_candidate(date) {
            candidates.push(c);
        }
    }
    candidates
}

/// Parse `"fat_loss (3)"` → `("fat_loss", 3)`. If the count isn't in
/// the expected form, default to 0 — the body still has bullets we can
/// count later if needed.
fn split_tag_count(rest: &str) -> (&str, usize) {
    if let Some((tag, suffix)) = rest.rsplit_once(" (") {
        if let Some(n_str) = suffix.strip_suffix(')') {
            if let Ok(n) = n_str.parse::<usize>() {
                return (tag.trim(), n);
            }
        }
    }
    (rest.trim(), 0)
}

/// In-flight section accumulator. Owns the lines we'll fold into a
/// SkillCandidate when we hit the next `## ` heading (or EOF).
struct SectionState {
    goal_tag: String,
    declared_count: usize, // count from the heading
    bullets: Vec<String>,
    body: String,
    first_summary: Option<String>,
}

impl SectionState {
    fn new(goal_tag: String, declared_count: usize) -> Self {
        Self {
            goal_tag,
            declared_count,
            bullets: Vec::new(),
            body: String::new(),
            first_summary: None,
        }
    }

    /// Absorb a bullet of the form `- **<kind>** (<timestamp>): <summary>`.
    /// Records the first summary string for provenance.
    fn absorb_bullet(&mut self, line: &str) {
        self.bullets.push(line.to_string());
        if self.first_summary.is_none() {
            if let Some(idx) = line.find("): ") {
                let summary = line[idx + 3..].trim().to_string();
                if !summary.is_empty() && summary != "(no summary)" {
                    self.first_summary = Some(summary);
                }
            }
        }
    }

    fn into_candidate(self, date: &str) -> Option<SkillCandidate> {
        if self.bullets.is_empty() {
            return None;
        }
        // Use bullets count when heading count was malformed/0; otherwise honor
        // what the brief declared (defends against a future change where bullets
        // are split across multiple lines).
        let event_count = if self.declared_count > 0 {
            self.declared_count
        } else {
            self.bullets.len()
        };
        let title = format!("{} — {}", self.goal_tag, date);
        let body = self.bullets.join("\n");
        Some(SkillCandidate {
            title,
            body,
            goal_tag: self.goal_tag,
            provenance: Provenance {
                source_kind: "daily_review",
                source_date: date.to_string(),
                event_count,
                analysis_quote: self.first_summary,
            },
            confidence: None, // Curator V2 ensemble assigns this — out of scope here
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_REVIEW: &str = "\
# Daily review — 2026-05-22

**Events captured:** 5

## fat_loss (3)
- **food_log** (2026-05-22T08:30:00Z): Two slices of toast with butter — moderate carb load.
- **food_log** (2026-05-22T12:30:00Z): Caesar salad and grilled chicken — within targets.
- **food_log** (2026-05-22T20:00:00Z): Small portion of pasta, late-ish dinner.

## focus (1)
- **focus_session** (2026-05-22T10:00:00Z): 50 minutes deep work, no distractions.

## habit (2)
- **food_log** (2026-05-22T12:30:00Z): Caesar salad and grilled chicken — within targets.
- **habit_check** (2026-05-22T18:00:00Z): Evening walk 30 min, weather mild.
";

    #[test]
    fn extracts_one_candidate_per_goal_tag_section() {
        let cands = extract_from_review_markdown(SAMPLE_REVIEW, "2026-05-22");
        assert_eq!(
            cands.len(),
            3,
            "expected one candidate per non-untagged section"
        );
        let tags: Vec<&str> = cands.iter().map(|c| c.goal_tag.as_str()).collect();
        assert!(tags.contains(&"fat_loss"));
        assert!(tags.contains(&"focus"));
        assert!(tags.contains(&"habit"));
    }

    #[test]
    fn candidate_title_contains_tag_and_date() {
        let cands = extract_from_review_markdown(SAMPLE_REVIEW, "2026-05-22");
        for c in &cands {
            assert!(c.title.contains(&c.goal_tag));
            assert!(c.title.contains("2026-05-22"));
        }
    }

    #[test]
    fn provenance_records_event_count_and_first_summary() {
        let cands = extract_from_review_markdown(SAMPLE_REVIEW, "2026-05-22");
        let fat = cands.iter().find(|c| c.goal_tag == "fat_loss").unwrap();
        assert_eq!(fat.provenance.source_kind, "daily_review");
        assert_eq!(fat.provenance.source_date, "2026-05-22");
        assert_eq!(
            fat.provenance.event_count, 3,
            "should honour the count from the heading"
        );
        let quote = fat
            .provenance
            .analysis_quote
            .as_ref()
            .expect("first summary present");
        assert!(
            quote.contains("toast"),
            "first summary should be the first bullet's: {}",
            quote
        );
    }

    #[test]
    fn ignores_untagged_section() {
        let with_untagged = format!(
            "{}\n## untagged (1)\n- **misc** (2026-05-22T09:00:00Z): just a thing\n",
            SAMPLE_REVIEW
        );
        let cands = extract_from_review_markdown(&with_untagged, "2026-05-22");
        let tags: Vec<&str> = cands.iter().map(|c| c.goal_tag.as_str()).collect();
        assert!(
            !tags.contains(&"untagged"),
            "untagged is noise, not a skill"
        );
        assert_eq!(cands.len(), 3); // unchanged from sample
    }

    #[test]
    fn empty_review_yields_no_candidates() {
        let empty =
            "# Daily review — 2026-05-22\n\n**Events captured:** 0\n\n(no events for this date)\n";
        let cands = extract_from_review_markdown(empty, "2026-05-22");
        assert!(cands.is_empty());
    }

    #[test]
    fn confidence_is_none_for_unjudged_candidates() {
        let cands = extract_from_review_markdown(SAMPLE_REVIEW, "2026-05-22");
        for c in &cands {
            assert!(
                c.confidence.is_none(),
                "Curator V2 assigns confidence; extractor does not"
            );
        }
    }
}
