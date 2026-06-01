//! Shape-1 scanner: detects assistant replies that assert file/script
//! creation while the round's tool_calls list is empty.

use serde_json::Value;

/// One pattern that, when matched in the reply, demands that this round
/// included at least one tool_start whose name is in `required_tools`.
#[derive(Debug, Clone)]
pub struct ClaimRule {
    pub id: &'static str,
    pub pattern: &'static str,
    pub required_tools: &'static [&'static str],
}

/// A single claim in the reply that lacks corroborating tool activity.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UnbackedClaim {
    pub rule_id: &'static str,
    pub matched_text: String,
    pub byte_offset: usize,
    pub explanation: String,
}

/// V1 rule set — Shape 1 only.
///
/// `claim_file_written`: matches assertion of file creation/writing
/// (success markers ✅/完成/成功/created/wrote) anywhere in the reply.
/// Requires at least one tool_start named `file_write` OR `file_edit`
/// OR `shell` to be present in this round's tool_calls.
pub const CLAIM_SIGNATURES: &[ClaimRule] = &[ClaimRule {
    id: "claim_file_written",
    // Conservative pattern: a success marker within 40 chars of a
    // file/path/script noun. Avoids the FP where the agent says
    // "✅ I understand" with no file claim. Multilingual: English
    // verbs + Traditional Chinese verbs.
    pattern: r"(?i)(✅|完成|成功|created|wrote|written|saved)\b.{0,40}\b(file|script|程式|檔案|文件|腳本)",
    required_tools: &["file_write", "file_edit", "shell"],
}];

/// Scan `reply` against all V1 rules. Returns one `UnbackedClaim` per
/// rule that fires without corroborating evidence in `tool_calls`.
///
/// `tool_calls` is the round's `all_tool_calls` slice as built by
/// `agent.rs::run_inner` — a `Vec<serde_json::Value>` where each value
/// has shape `{"tool": "<name>", "args": <object>}`.
/// `tool_results` is the round's tool result strings (currently unused
/// by V1 rules but kept in the signature so V2 rules can grow without
/// changing the public API).
pub fn scan(reply: &str, tool_calls: &[Value], _tool_results: &[String]) -> Vec<UnbackedClaim> {
    let called: std::collections::HashSet<&str> = tool_calls
        .iter()
        .filter_map(|tc| tc.get("tool").and_then(|t| t.as_str()))
        .collect();

    let mut out = Vec::new();
    for rule in CLAIM_SIGNATURES {
        let re = match regex::Regex::new(rule.pattern) {
            Ok(re) => re,
            Err(e) => {
                tracing::warn!(
                    rule_id = rule.id,
                    pattern = rule.pattern,
                    "hallucination rule regex compile failed: {}",
                    e
                );
                continue; // never panic — bad rule is a non-event
            }
        };
        let Some(m) = re.find(reply) else { continue };

        let any_required = rule.required_tools.iter().any(|t| called.contains(t));
        if any_required {
            continue;
        }
        out.push(UnbackedClaim {
            rule_id: rule.id,
            matched_text: m.as_str().to_string(),
            byte_offset: m.start(),
            explanation: format!(
                "assistant asserted '{}' but this round called none of: {:?}",
                m.as_str(),
                rule.required_tools,
            ),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shape1_fires_when_claim_present_and_no_tools_called() {
        let reply = "✅ 完成！我已建立 file 範例程式於 D:/foo.py";
        let tool_calls: Vec<Value> = vec![]; // zero tool activity
        let tool_results: Vec<String> = vec![];

        let claims = scan(reply, &tool_calls, &tool_results);

        assert_eq!(
            claims.len(),
            1,
            "expected exactly one unbacked claim, got: {:?}",
            claims
        );
        assert_eq!(claims[0].rule_id, "claim_file_written");
        assert!(
            claims[0].matched_text.contains("完成"),
            "matched_text was {:?}",
            claims[0].matched_text
        );
    }

    #[test]
    fn shape1_silent_when_claim_present_but_tool_called() {
        let reply = "✅ 完成！我已建立 file 範例程式於 D:/foo.py";
        let tool_calls: Vec<Value> = vec![json!({
            "tool": "file_write",
            "args": {"path": "D:/foo.py", "content": "print('x')"}
        })];
        let tool_results: Vec<String> = vec!["wrote 12 bytes".into()];

        let claims = scan(reply, &tool_calls, &tool_results);

        assert!(
            claims.is_empty(),
            "expected no warning when file_write was called, got: {:?}",
            claims
        );
    }

    #[test]
    fn shape1_does_not_fire_on_generic_completion_phrase() {
        // "completed" without a file/script noun nearby should NOT trigger.
        // FP guard: the agent often says "✅ Got it" or "completed your
        // question" without claiming a side effect.
        let reply = "✅ Got it. I understand your question and will help.";
        let tool_calls: Vec<Value> = vec![];
        let tool_results: Vec<String> = vec![];

        let claims = scan(reply, &tool_calls, &tool_results);

        assert!(
            claims.is_empty(),
            "FP: scanner fired on bare ✅ acknowledgement, got: {:?}",
            claims
        );
    }

    #[test]
    fn shape1_handles_empty_reply_without_panic() {
        let claims = scan("", &[], &[]);
        assert!(claims.is_empty());
    }

    #[test]
    fn shape1_handles_empty_reply_with_tool_calls() {
        // Edge case: model emitted tool calls but no natural-language reply
        // (common mid-loop turn). Scanner must produce no claims and not panic.
        let tool_calls: Vec<Value> = vec![json!({"tool": "file_read", "args": {}})];
        let claims = scan("", &tool_calls, &[]);
        assert!(claims.is_empty());
    }
}
