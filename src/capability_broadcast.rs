//! Capability broadcast — injects a compact list of available tools and hands
//! into the agent's system prompt at session start.
//!
//! This lets the LLM know which capabilities are available without requiring
//! the agent to ask or guess. The injected text is intentionally compact
//! (under 500 tokens) to minimise context overhead.

use crate::tools::ToolSpec;

/// Build a compact capability summary string.
///
/// Format (two lines):
/// ```text
/// Available tools: shell, file_read, file_write, ... (N total)
/// Available hands: content, freelancer, seo_content, ... (M total)
/// ```
///
/// Both lists are sorted alphabetically. The names list is truncated at a
/// configurable character budget so the final string stays well under 500
/// tokens even when there are many tools/hands.
pub fn build_capability_prompt(tools: &[ToolSpec], hands: &[String]) -> String {
    let mut tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    tool_names.sort_unstable();

    let mut hand_names: Vec<&str> = hands.iter().map(|h| h.as_str()).collect();
    hand_names.sort_unstable();

    let tool_line = format_names_line("Available tools", &tool_names);
    let hand_line = format_names_line("Available hands", &hand_names);

    format!("{}\n{}", tool_line, hand_line)
}

/// Format a single line: "Available <label>: a, b, c, ... (N total)"
///
/// The name list is trimmed at roughly 300 characters so each line stays
/// short. The total count is always accurate regardless of truncation.
fn format_names_line(label: &str, names: &[&str]) -> String {
    let total = names.len();
    if total == 0 {
        return format!("{}: (none)", label);
    }

    // Budget: ~300 chars for the comma-separated names before we add "(N total)".
    const NAMES_BUDGET: usize = 300;

    let mut buf = String::new();
    let mut truncated = false;
    for (i, name) in names.iter().enumerate() {
        let sep = if i == 0 { "" } else { ", " };
        let candidate = format!("{}{}", sep, name);
        if buf.len() + candidate.len() > NAMES_BUDGET {
            truncated = true;
            break;
        }
        buf.push_str(&candidate);
    }

    if truncated {
        format!("{}: {}, ... ({} total)", label, buf, total)
    } else {
        format!("{}: {} ({} total)", label, buf, total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_spec(name: &str) -> ToolSpec {
        ToolSpec {
            name: name.to_string(),
            description: format!("Description of {}", name),
            parameters: json!({"type": "object", "properties": {}}),
        }
    }

    #[test]
    fn test_empty_inputs() {
        let result = build_capability_prompt(&[], &[]);
        assert!(result.contains("Available tools: (none)"), "got: {}", result);
        assert!(result.contains("Available hands: (none)"), "got: {}", result);
    }

    #[test]
    fn test_format_lists_tools_and_hands() {
        let tools = vec![make_spec("shell"), make_spec("file_read"), make_spec("web_search")];
        let hands = vec!["content".to_string(), "freelancer".to_string(), "seo_content".to_string()];

        let result = build_capability_prompt(&tools, &hands);

        // Both labels must appear
        assert!(result.contains("Available tools:"), "got: {}", result);
        assert!(result.contains("Available hands:"), "got: {}", result);

        // All tool names must appear (sorted)
        assert!(result.contains("file_read"), "got: {}", result);
        assert!(result.contains("shell"), "got: {}", result);
        assert!(result.contains("web_search"), "got: {}", result);

        // All hand names must appear
        assert!(result.contains("content"), "got: {}", result);
        assert!(result.contains("freelancer"), "got: {}", result);
        assert!(result.contains("seo_content"), "got: {}", result);
    }

    #[test]
    fn test_tool_count_matches() {
        let names = ["alpha", "beta", "gamma", "delta"];
        let tools: Vec<ToolSpec> = names.iter().map(|n| make_spec(n)).collect();
        let result = build_capability_prompt(&tools, &[]);

        // The total count in the line should match the number of tools supplied.
        let expected_count = format!("({} total)", names.len());
        assert!(
            result.contains(&expected_count),
            "Expected '{}' in:\n{}",
            expected_count,
            result
        );
    }

    #[test]
    fn test_hand_count_matches() {
        let hands: Vec<String> = (0..7).map(|i| format!("hand_{}", i)).collect();
        let result = build_capability_prompt(&[], &hands);

        let expected_count = format!("({} total)", hands.len());
        assert!(
            result.contains(&expected_count),
            "Expected '{}' in:\n{}",
            expected_count,
            result
        );
    }

    #[test]
    fn test_names_are_sorted() {
        // Provide unsorted names — the output should be alphabetical.
        let tools = vec![make_spec("zzz_last"), make_spec("aaa_first"), make_spec("mmm_mid")];
        let result = build_capability_prompt(&tools, &[]);

        let tools_line = result.lines().next().unwrap_or("");
        let pos_aaa = tools_line.find("aaa_first").unwrap_or(usize::MAX);
        let pos_mmm = tools_line.find("mmm_mid").unwrap_or(usize::MAX);
        let pos_zzz = tools_line.find("zzz_last").unwrap_or(usize::MAX);

        assert!(pos_aaa < pos_mmm, "aaa_first should appear before mmm_mid");
        assert!(pos_mmm < pos_zzz, "mmm_mid should appear before zzz_last");
    }

    #[test]
    fn test_output_is_compact_under_500_tokens() {
        // Generate a realistic worst-case: 55 tools and 30 hands.
        let tools: Vec<ToolSpec> = (0..55)
            .map(|i| make_spec(&format!("tool_with_longer_name_{:02}", i)))
            .collect();
        let hands: Vec<String> = (0..30)
            .map(|i| format!("hand_with_longer_name_{:02}", i))
            .collect();

        let result = build_capability_prompt(&tools, &hands);

        // Rough token estimate: 1 token ≈ 4 chars.
        // Keep the prompt well under 500 tokens → ~2000 chars.
        assert!(
            result.len() < 2000,
            "Capability prompt is too long ({} chars): {}",
            result.len(),
            result
        );
    }

    #[test]
    fn test_truncated_still_shows_count() {
        // Even when the name list is truncated, the total count should still be visible.
        let tools: Vec<ToolSpec> = (0..60)
            .map(|i| make_spec(&format!("tool_name_quite_long_{:02}", i)))
            .collect();
        let result = build_capability_prompt(&tools, &[]);
        let tools_line = result.lines().next().unwrap_or("");

        // Must contain the real total even when "..." is present.
        assert!(
            tools_line.contains("(60 total)"),
            "Expected '(60 total)' in: {}",
            tools_line
        );
        assert!(tools_line.contains("..."), "Expected truncation marker in: {}", tools_line);
    }

    #[test]
    fn test_two_line_format() {
        let tools = vec![make_spec("shell")];
        let hands = vec!["content".to_string()];
        let result = build_capability_prompt(&tools, &hands);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 2, "Expected exactly 2 lines, got:\n{}", result);
        assert!(lines[0].starts_with("Available tools:"), "Line 0: {}", lines[0]);
        assert!(lines[1].starts_with("Available hands:"), "Line 1: {}", lines[1]);
    }
}
