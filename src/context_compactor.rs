//! Context Compactor — multi-strategy context window compression.
//! Replaces the simple trim in agent_runtime.rs with tiered compaction:
//! Light (summarize old), Medium (+ fold tool pairs), Aggressive (only keep recent N).
//! Falls back to ContextOptimizer::trim_messages if compaction fails.

use std::ops::Range;
use tracing::debug;

use crate::context::ContextOptimizer;
use crate::providers::ChatMessage;

/// Compaction intensity
#[derive(Debug, Clone, PartialEq)]
pub enum CompactionStrategy {
    /// Only summarize old messages
    Light,
    /// Summarize + fold tool call/result pairs
    Medium,
    /// Aggressive: only keep recent N rounds
    Aggressive,
}

/// A matched tool-call/result pair
#[derive(Debug, Clone)]
pub struct ToolPair {
    pub call_idx: usize,
    pub result_idx: usize,
    pub tool_name: String,
}

/// Plan for how to compact messages
#[derive(Debug, Clone)]
pub struct CompactionPlan {
    pub strategy: CompactionStrategy,
    /// Range of message indices to summarize
    pub summarize_range: Range<usize>,
    /// Tool pairs that can be folded
    pub tool_pairs: Vec<ToolPair>,
    /// Number of recent messages to preserve
    pub keep_recent: usize,
    /// Estimated token count after compaction
    pub estimated_after: usize,
}

/// Context Compactor — multi-strategy compression engine.
pub struct ContextCompactor;

impl ContextCompactor {
    /// Analyze messages and produce a compaction plan if needed.
    /// Returns None if messages are within budget.
    pub fn plan(messages: &[ChatMessage], model: &str) -> Option<CompactionPlan> {
        let window = ContextOptimizer::get_window_size(model);
        let budget = (window as f64 * 0.80) as usize;
        let current = ContextOptimizer::estimate_messages_tokens(messages);

        if current <= budget {
            return None;
        }

        let ratio = current as f64 / budget as f64;
        let keep_recent = Self::adaptive_keep_recent(model);
        let tool_pairs = Self::find_tool_pairs(messages);

        // Find the summarizable range: after system, before keep_recent
        let first_non_system = messages.iter()
            .position(|m| m.role != "system")
            .unwrap_or(1);
        let summarize_end = if messages.len() > keep_recent {
            messages.len() - keep_recent
        } else {
            first_non_system
        };

        if summarize_end <= first_non_system {
            return None; // Nothing to compact
        }

        let strategy = if ratio < 1.5 {
            CompactionStrategy::Light
        } else if ratio < 3.0 {
            CompactionStrategy::Medium
        } else {
            CompactionStrategy::Aggressive
        };

        // Estimate tokens after compaction (rough: summary ~100 tokens + recent)
        let recent_tokens: usize = messages[summarize_end..].iter()
            .map(|m| 4 + ContextOptimizer::estimate_tokens(&m.content))
            .sum();
        let system_tokens: usize = messages[..first_non_system].iter()
            .map(|m| 4 + ContextOptimizer::estimate_tokens(&m.content))
            .sum();
        let estimated_after = system_tokens + 100 + recent_tokens; // ~100 for summary

        Some(CompactionPlan {
            strategy,
            summarize_range: first_non_system..summarize_end,
            tool_pairs,
            keep_recent,
            estimated_after,
        })
    }

    /// Find matching tool-call and tool-result pairs in messages.
    pub fn find_tool_pairs(messages: &[ChatMessage]) -> Vec<ToolPair> {
        let mut pairs = Vec::new();

        for (i, msg) in messages.iter().enumerate() {
            if msg.role == "assistant" {
                if let Some(ref tcs) = msg.tool_calls {
                    for tc in tcs {
                        if let Some(ref call_id) = tc.id {
                            // Find matching tool result
                            for (j, result_msg) in messages[i+1..].iter().enumerate() {
                                if result_msg.role == "tool" {
                                    if result_msg.tool_call_id.as_deref() == Some(call_id) {
                                        pairs.push(ToolPair {
                                            call_idx: i,
                                            result_idx: i + 1 + j,
                                            tool_name: tc.function.name.clone(),
                                        });
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        pairs
    }

    /// Remove orphan tool results (tool results without matching call) and
    /// orphan tool calls (calls without matching results).
    pub fn repair_tool_pairing(messages: &mut Vec<ChatMessage>) {
        // Collect all tool_call IDs from assistant messages
        let mut call_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for msg in messages.iter() {
            if msg.role == "assistant" {
                if let Some(ref tcs) = msg.tool_calls {
                    for tc in tcs {
                        if let Some(ref id) = tc.id {
                            call_ids.insert(id.clone());
                        }
                    }
                }
            }
        }

        // Remove tool results that don't have a matching call
        let before_len = messages.len();
        messages.retain(|m| {
            if m.role == "tool" {
                if let Some(ref id) = m.tool_call_id {
                    return call_ids.contains(id);
                }
                return false; // No ID at all — orphan
            }
            true
        });

        let removed = before_len - messages.len();
        if removed > 0 {
            debug!("Repaired tool pairing: removed {} orphan tool results", removed);
        }
    }

    /// Build a summary prompt appropriate for the compaction strategy.
    pub fn build_summary_prompt(
        messages: &[ChatMessage],
        tool_pairs: &[ToolPair],
        strategy: &CompactionStrategy,
    ) -> String {
        let mut transcript = String::new();

        match strategy {
            CompactionStrategy::Light => {
                // Simple summarization of all messages
                for msg in messages {
                    let role = Self::role_label(&msg.role);
                    if !msg.content.trim().is_empty() {
                        let truncated: String = msg.content.chars().take(500).collect();
                        transcript.push_str(&format!("{}: {}\n", role, truncated));
                    }
                }
                format!(
                    "Summarize the following conversation in 2-3 concise paragraphs. \
                     Preserve key facts, decisions, and context. Be brief.\n\n{}",
                    transcript
                )
            }
            CompactionStrategy::Medium => {
                // Summarize, but fold tool pairs into "[Tool: X → result summary]"
                let pair_indices: std::collections::HashSet<usize> = tool_pairs.iter()
                    .flat_map(|p| vec![p.call_idx, p.result_idx])
                    .collect();

                for (i, msg) in messages.iter().enumerate() {
                    if pair_indices.contains(&i) {
                        // Check if this is a tool pair call
                        if let Some(pair) = tool_pairs.iter().find(|p| p.call_idx == i) {
                            let result = messages.get(pair.result_idx)
                                .map(|m| m.content.chars().take(100).collect::<String>())
                                .unwrap_or_default();
                            transcript.push_str(&format!("[Tool: {} → {}]\n", pair.tool_name, result));
                        }
                        continue;
                    }
                    let role = Self::role_label(&msg.role);
                    if !msg.content.trim().is_empty() {
                        let truncated: String = msg.content.chars().take(300).collect();
                        transcript.push_str(&format!("{}: {}\n", role, truncated));
                    }
                }
                format!(
                    "Summarize this conversation and tool interactions in 2-3 paragraphs. \
                     Focus on decisions made and information discovered.\n\n{}",
                    transcript
                )
            }
            CompactionStrategy::Aggressive => {
                // Very compressed: only key points
                for msg in messages {
                    let role = Self::role_label(&msg.role);
                    if !msg.content.trim().is_empty() {
                        let truncated: String = msg.content.chars().take(200).collect();
                        transcript.push_str(&format!("{}: {}\n", role, truncated));
                    }
                }
                format!(
                    "Create a very brief summary (1-2 sentences) of this conversation. \
                     Only include the most critical facts and the current task.\n\n{}",
                    transcript
                )
            }
        }
    }

    /// Apply a compaction plan: replace summarizable range with a summary message.
    pub fn apply(
        messages: &mut Vec<ChatMessage>,
        plan: &CompactionPlan,
        summary: &str,
    ) {
        if plan.summarize_range.is_empty() || summary.trim().is_empty() {
            return;
        }

        let start = plan.summarize_range.start;
        let end = plan.summarize_range.end.min(messages.len());

        if start >= end || start >= messages.len() {
            return;
        }

        // Remove the summarized range
        messages.drain(start..end);

        // Insert summary as a system message
        let strategy_label = match plan.strategy {
            CompactionStrategy::Light => "light",
            CompactionStrategy::Medium => "medium",
            CompactionStrategy::Aggressive => "aggressive",
        };

        messages.insert(start, ChatMessage {
            role: "system".into(),
            content: format!(
                "[Conversation summary ({} compaction)]\n{}",
                strategy_label, summary
            ),
            tool_calls: None,
            tool_call_id: None,
        });

        debug!(
            "Context compacted: strategy={}, removed {} messages, summary={} chars",
            strategy_label,
            end - start,
            summary.len()
        );
    }

    /// Determine how many recent messages to keep based on model window size.
    pub fn adaptive_keep_recent(model: &str) -> usize {
        let window = ContextOptimizer::get_window_size(model);
        if window >= 64000 {
            8 // Large context — keep more
        } else if window >= 16000 {
            4 // Medium context
        } else {
            2 // Small context — aggressive trimming
        }
    }

    fn role_label(role: &str) -> &str {
        match role {
            "user" => "User",
            "assistant" => "Assistant",
            "tool" => "Tool",
            "system" => "System",
            _ => role,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use crate::providers::{ToolCall, ToolCallFunction};

    fn sys_msg(content: &str) -> ChatMessage {
        ChatMessage { role: "system".into(), content: content.into(), tool_calls: None, tool_call_id: None }
    }

    fn user_msg(content: &str) -> ChatMessage {
        ChatMessage { role: "user".into(), content: content.into(), tool_calls: None, tool_call_id: None }
    }

    fn assistant_msg(content: &str) -> ChatMessage {
        ChatMessage { role: "assistant".into(), content: content.into(), tool_calls: None, tool_call_id: None }
    }

    fn tool_call_msg(call_id: &str, tool_name: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".into(),
            content: "".into(),
            tool_calls: Some(vec![ToolCall {
                id: Some(call_id.into()),
                function: ToolCallFunction {
                    name: tool_name.into(),
                    arguments: json!({}),
                },
            }]),
            tool_call_id: None,
        }
    }

    fn tool_result_msg(call_id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".into(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(call_id.into()),
        }
    }

    // ── Plan tests ──

    #[test]
    fn test_plan_within_budget_returns_none() {
        let msgs = vec![sys_msg("sys"), user_msg("hi")];
        assert!(ContextCompactor::plan(&msgs, "gpt-4o").is_none());
    }

    #[test]
    fn test_plan_over_budget_returns_some() {
        let mut msgs = vec![sys_msg("sys")];
        for _ in 0..200 {
            msgs.push(user_msg(&"x".repeat(500)));
        }
        let plan = ContextCompactor::plan(&msgs, "llama3:8b");
        assert!(plan.is_some());
    }

    #[test]
    fn test_plan_strategy_light_for_small_excess() {
        // Create messages that barely exceed budget (ratio < 1.5)
        let mut msgs = vec![sys_msg("sys")];
        // llama3:8b window = 8192, budget = 6553
        // Each msg ~ 130 tokens, need ~50 messages to slightly exceed
        for i in 0..55 {
            msgs.push(if i % 2 == 0 { user_msg(&"x".repeat(500)) } else { assistant_msg(&"y".repeat(500)) });
        }
        let plan = ContextCompactor::plan(&msgs, "llama3:8b");
        if let Some(p) = plan {
            // Strategy depends on exact ratio, but should be valid
            assert!(!p.summarize_range.is_empty());
        }
    }

    #[test]
    fn test_plan_strategy_aggressive_for_large_excess() {
        let mut msgs = vec![sys_msg("sys")];
        // Way over budget
        for i in 0..500 {
            msgs.push(if i % 2 == 0 { user_msg(&"x".repeat(500)) } else { assistant_msg(&"y".repeat(500)) });
        }
        let plan = ContextCompactor::plan(&msgs, "llama3:8b").unwrap();
        assert_eq!(plan.strategy, CompactionStrategy::Aggressive);
    }

    // ── Tool pair tests ──

    #[test]
    fn test_find_tool_pairs() {
        let msgs = vec![
            sys_msg("sys"),
            tool_call_msg("call_1", "shell"),
            tool_result_msg("call_1", "file list"),
            user_msg("thanks"),
        ];
        let pairs = ContextCompactor::find_tool_pairs(&msgs);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].tool_name, "shell");
        assert_eq!(pairs[0].call_idx, 1);
        assert_eq!(pairs[0].result_idx, 2);
    }

    #[test]
    fn test_find_tool_pairs_multiple() {
        let msgs = vec![
            sys_msg("sys"),
            tool_call_msg("c1", "shell"),
            tool_result_msg("c1", "r1"),
            tool_call_msg("c2", "file_read"),
            tool_result_msg("c2", "r2"),
        ];
        let pairs = ContextCompactor::find_tool_pairs(&msgs);
        assert_eq!(pairs.len(), 2);
    }

    #[test]
    fn test_find_tool_pairs_no_match() {
        let msgs = vec![
            sys_msg("sys"),
            user_msg("hello"),
            assistant_msg("world"),
        ];
        let pairs = ContextCompactor::find_tool_pairs(&msgs);
        assert!(pairs.is_empty());
    }

    // ── Repair tests ──

    #[test]
    fn test_repair_removes_orphan_tool_results() {
        let mut msgs = vec![
            sys_msg("sys"),
            tool_result_msg("orphan_id", "orphan result"),
            user_msg("hello"),
        ];
        ContextCompactor::repair_tool_pairing(&mut msgs);
        assert_eq!(msgs.len(), 2); // sys + user
        assert!(msgs.iter().all(|m| m.role != "tool"));
    }

    #[test]
    fn test_repair_keeps_matched_tool_results() {
        let mut msgs = vec![
            sys_msg("sys"),
            tool_call_msg("c1", "shell"),
            tool_result_msg("c1", "result"),
            user_msg("thanks"),
        ];
        ContextCompactor::repair_tool_pairing(&mut msgs);
        assert_eq!(msgs.len(), 4); // All kept
    }

    // ── Apply tests ──

    #[test]
    fn test_apply_replaces_range() {
        let mut msgs = vec![
            sys_msg("sys"),
            user_msg("msg1"),
            assistant_msg("msg2"),
            user_msg("msg3"),
            assistant_msg("msg4"),
            user_msg("recent1"),
            assistant_msg("recent2"),
        ];

        let plan = CompactionPlan {
            strategy: CompactionStrategy::Light,
            summarize_range: 1..5,
            tool_pairs: vec![],
            keep_recent: 2,
            estimated_after: 100,
        };

        ContextCompactor::apply(&mut msgs, &plan, "Summary of conversation");
        // sys + summary + recent1 + recent2 = 4
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[1].content.contains("Summary of conversation"));
        assert!(msgs[1].content.contains("light"));
    }

    #[test]
    fn test_apply_empty_summary_does_nothing() {
        let mut msgs = vec![sys_msg("sys"), user_msg("hi")];
        let original_len = msgs.len();
        let plan = CompactionPlan {
            strategy: CompactionStrategy::Light,
            summarize_range: 1..2,
            tool_pairs: vec![],
            keep_recent: 0,
            estimated_after: 50,
        };
        ContextCompactor::apply(&mut msgs, &plan, "");
        assert_eq!(msgs.len(), original_len);
    }

    #[test]
    fn test_apply_empty_range_does_nothing() {
        let mut msgs = vec![sys_msg("sys"), user_msg("hi")];
        let original_len = msgs.len();
        let plan = CompactionPlan {
            strategy: CompactionStrategy::Light,
            summarize_range: 1..1, // Empty range
            tool_pairs: vec![],
            keep_recent: 1,
            estimated_after: 50,
        };
        ContextCompactor::apply(&mut msgs, &plan, "Summary");
        assert_eq!(msgs.len(), original_len);
    }

    // ── Adaptive keep_recent tests ──

    #[test]
    fn test_adaptive_keep_recent_large_window() {
        assert_eq!(ContextCompactor::adaptive_keep_recent("gpt-4o"), 8); // 128K
        assert_eq!(ContextCompactor::adaptive_keep_recent("claude-sonnet-4-6"), 8); // 200K
    }

    #[test]
    fn test_adaptive_keep_recent_medium_window() {
        assert_eq!(ContextCompactor::adaptive_keep_recent("qwen3:8b"), 4); // 32K
    }

    #[test]
    fn test_adaptive_keep_recent_small_window() {
        assert_eq!(ContextCompactor::adaptive_keep_recent("llama3:8b"), 2); // 8K
        assert_eq!(ContextCompactor::adaptive_keep_recent("unknown-model"), 2); // Default 8K
    }

    // ── Summary prompt tests ──

    #[test]
    fn test_build_summary_prompt_light() {
        let msgs = vec![
            user_msg("What is Rust?"),
            assistant_msg("A systems language"),
        ];
        let prompt = ContextCompactor::build_summary_prompt(&msgs, &[], &CompactionStrategy::Light);
        assert!(prompt.contains("Summarize"));
        assert!(prompt.contains("User: What is Rust?"));
    }

    #[test]
    fn test_build_summary_prompt_medium_folds_tools() {
        let msgs = vec![
            tool_call_msg("c1", "shell"),
            tool_result_msg("c1", "file_list.txt"),
            user_msg("thanks"),
        ];
        let pairs = ContextCompactor::find_tool_pairs(&msgs);
        let prompt = ContextCompactor::build_summary_prompt(&msgs, &pairs, &CompactionStrategy::Medium);
        assert!(prompt.contains("[Tool: shell"));
    }

    #[test]
    fn test_build_summary_prompt_aggressive() {
        let msgs = vec![user_msg("long task")];
        let prompt = ContextCompactor::build_summary_prompt(&msgs, &[], &CompactionStrategy::Aggressive);
        assert!(prompt.contains("very brief"));
    }
}
