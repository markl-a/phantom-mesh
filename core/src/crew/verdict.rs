//! A reviewer's verdict on a task, and the parser that turns an agent's raw
//! reply into one. Ported from the `ensemble` project (Apache-2.0) into this
//! AGPL crate as the first slice of the crew choreography port (ECOSYSTEM
//! master-plan §6); the orchestration logic is unchanged.

/// A reviewer's decision on a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Approve,
    /// Changes requested; the String is the message routed back to the implementer.
    Changes(String),
}

/// Parse an agent's reply into a verdict. Convention: a line `VERDICT: LGTM|APPROVE` approves;
/// `VERDICT: CHANGES: <msg>` requests changes. Anything without an explicit approving VERDICT
/// line is treated as changes-requested — an unparseable or ambiguous review must NEVER land.
pub fn parse_verdict(text: &str) -> Verdict {
    // Scan every line that MENTIONS "verdict" (case-insensitive) — tolerating markdown prefixes a
    // real reviewer adds, e.g. "## Review verdict: ✅ Approve" or "**VERDICT: LGTM**". Classify by
    // the verdict TOKEN that follows the marker, keeping the LAST such line as authoritative. A
    // reply with no verdict line is conservatively changes-requested.
    //
    // The classification reads only the payload AFTER the `verdict` marker and matches the
    // token at its head — NOT a substring scan of the whole line. Two real failure modes this
    // avoids (caught porting from ensemble): a CHANGES message that itself contains the word
    // "approve" (`VERDICT: CHANGES: do not approve until tests pass`) must NOT land; and a
    // negative token (`DISAPPROVE` / `NOT APPROVED`) must fall through to conservative changes,
    // never matched as an approval by a bare `contains("approve")`.
    let mut result: Option<Verdict> = None;
    for line in text.lines() {
        let low = line.to_ascii_lowercase();
        let Some(vidx) = low.find("verdict") else {
            continue;
        };
        let marker_end = vidx + "verdict".len();
        // Strip the leading separators/markdown the reviewer put between `verdict` and the token
        // (`:`, whitespace, `✅`, `*`, …) so we can match the token at the head of the payload.
        // Lowercasing ASCII preserves byte offsets, so `low` indices are valid in `line` too.
        let token_low = low[marker_end..].trim_start_matches(|c: char| !c.is_ascii_alphanumeric());
        if token_low.starts_with("changes") {
            // Message after "changes" (+ optional ':') in the ORIGINAL-case line.
            let after_start = line.len() - token_low.len() + "changes".len();
            let after = line[after_start..]
                .trim_start_matches(|c: char| c == ':' || c.is_whitespace());
            result = Some(Verdict::Changes(after.to_string()));
        } else if token_low.starts_with("lgtm") || token_low.starts_with("approve") {
            result = Some(Verdict::Approve);
        } else {
            // Unknown / negative token (disapprove, not approved, blank) → conservative changes.
            result = Some(Verdict::Changes(format!(
                "unrecognized verdict line: {}",
                line.trim()
            )));
        }
    }
    result.unwrap_or_else(|| {
        Verdict::Changes("no explicit VERDICT line; treating as changes-requested".to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_approve_and_changes_conservatively() {
        assert_eq!(parse_verdict("looks good\nVERDICT: LGTM"), Verdict::Approve);
        assert_eq!(parse_verdict("VERDICT: APPROVE"), Verdict::Approve);
        assert_eq!(
            parse_verdict("issues...\nVERDICT: CHANGES: fix the off-by-one"),
            Verdict::Changes("fix the off-by-one".to_string())
        );
        // No marker at all ⇒ conservative: NOT an approval (an unparseable review can't land).
        assert_eq!(
            parse_verdict("I think it is fine"),
            Verdict::Changes("no explicit VERDICT line; treating as changes-requested".to_string())
        );
    }

    #[test]
    fn changes_message_mentioning_approve_does_not_land() {
        // A CHANGES verdict whose MESSAGE contains the word "approve" must stay Changes — the
        // token after the marker is CHANGES, not an approval. (A bare contains("approve") scan
        // of the whole line would wrongly land this.)
        assert_eq!(
            parse_verdict("VERDICT: CHANGES: do not approve until tests pass"),
            Verdict::Changes("do not approve until tests pass".to_string())
        );
    }

    #[test]
    fn negative_tokens_fall_through_to_conservative_changes() {
        // "disapprove" / "not approved" embed the substring "approve" but are NOT approvals;
        // they must fall through to conservative changes-requested, never land.
        assert!(matches!(
            parse_verdict("VERDICT: DISAPPROVE"),
            Verdict::Changes(_)
        ));
        assert!(matches!(
            parse_verdict("VERDICT: NOT APPROVED"),
            Verdict::Changes(_)
        ));
    }

    #[test]
    fn markdown_and_emoji_prefixed_approve_still_lands() {
        // A real reviewer's markdown/emoji around the token must not block a genuine approval.
        assert_eq!(parse_verdict("## Review verdict: ✅ Approve"), Verdict::Approve);
        assert_eq!(parse_verdict("**VERDICT: LGTM**"), Verdict::Approve);
        // A "changes" word BEFORE the marker is ignored — the token after the marker wins.
        assert_eq!(parse_verdict("no changes needed. VERDICT: LGTM"), Verdict::Approve);
    }
}
