use crate::providers::ChatMessage;

/// Known model context window sizes (in tokens)
const MODEL_WINDOWS: &[(&str, usize)] = &[
    // Ollama / local models
    ("qwen3:8b", 32768),
    ("qwen3:32b", 32768),
    ("qwen3.5:27b", 32768),
    ("llama3:8b", 8192),
    ("llama3:70b", 8192),
    ("llama3.1:8b", 131072),
    ("llama3.1:70b", 131072),
    ("mistral:7b", 32768),
    ("mixtral:8x7b", 32768),
    ("phi-4-mini", 16384),
    ("deepseek-r1:14b", 65536),
    ("deepseek-r1:32b", 65536),
    ("gemma2:9b", 8192),
    ("codellama:13b", 16384),
    // Cloud models
    ("claude-opus-4-6", 200000),
    ("claude-sonnet-4-6", 200000),
    ("claude-haiku-4-5-20251001", 200000),
    ("gpt-4o", 128000),
    ("gpt-4o-mini", 128000),
    ("gpt-4-turbo", 128000),
    ("gpt-3.5-turbo", 16384),
    ("o1", 200000),
    ("o1-mini", 128000),
    // Cloud free/cheap providers
    ("deepseek-chat", 65536),
    ("llama-3.3-70b-versatile", 131072),
    ("llama-3.3-70b", 131072),
    ("meta-llama/Llama-3.3-70B-Instruct-Turbo", 131072),
    ("meta-llama/llama-3.3-70b-instruct", 131072),
];

/// Default context window if model is unknown
const DEFAULT_WINDOW: usize = 8192;

/// Fraction of the context window to target (leave room for the response)
const TARGET_FILL_RATIO: f64 = 0.80;

/// Context window management: token estimation and message trimming.
pub struct ContextOptimizer;

impl ContextOptimizer {
    /// Estimate token count for a string (chars / 4 heuristic).
    /// This is a rough estimate — actual tokenization varies per model.
    pub fn estimate_tokens(text: &str) -> usize {
        // chars/4 is a reasonable heuristic for English + code;
        // CJK characters are roughly 1 token each, so slightly underestimates for CJK
        (text.len() + 3) / 4
    }

    /// Estimate total tokens for a message array
    pub fn estimate_messages_tokens(messages: &[ChatMessage]) -> usize {
        messages.iter().map(|m| {
            // Each message has ~4 tokens of overhead (role, formatting)
            let overhead = 4;
            let content_tokens = Self::estimate_tokens(&m.content);
            let tool_tokens = m.tool_calls.as_ref().map(|tcs| {
                tcs.iter().map(|tc| {
                    Self::estimate_tokens(&tc.function.name)
                        + Self::estimate_tokens(&tc.function.arguments.to_string())
                        + 4 // tool call overhead
                }).sum::<usize>()
            }).unwrap_or(0);
            overhead + content_tokens + tool_tokens
        }).sum()
    }

    /// Get the context window size for a model name.
    /// Returns default (8192) for unknown models.
    pub fn get_window_size(model: &str) -> usize {
        // Try exact match first
        for &(name, size) in MODEL_WINDOWS {
            if model == name {
                return size;
            }
        }
        // Try prefix match (e.g., "qwen3:8b-q4" matches "qwen3:8b")
        for &(name, size) in MODEL_WINDOWS {
            if model.starts_with(name) {
                return size;
            }
        }
        // Try contains match for cloud models (e.g., "claude-sonnet-4-6-20260301")
        for &(name, size) in MODEL_WINDOWS {
            if model.contains(name) || name.contains(model) {
                return size;
            }
        }
        DEFAULT_WINDOW
    }

    /// Check if messages exceed the context budget for a model.
    pub fn needs_compaction(messages: &[ChatMessage], model: &str) -> bool {
        let window = Self::get_window_size(model);
        let budget = (window as f64 * TARGET_FILL_RATIO) as usize;
        Self::estimate_messages_tokens(messages) > budget
    }

    /// Compact old messages into a summary, preserving recent context.
    /// Returns the messages that should be summarized (the "evictable" middle).
    /// The caller is responsible for getting the summary from the LLM and calling
    /// `apply_compaction()` with the result.
    pub fn messages_to_compact(messages: &[ChatMessage], model: &str, keep_recent: usize) -> Vec<ChatMessage> {
        let window = Self::get_window_size(model);
        let budget = (window as f64 * TARGET_FILL_RATIO) as usize;
        let current = Self::estimate_messages_tokens(messages);

        if current <= budget || messages.len() <= 2 + keep_recent {
            return vec![];
        }

        // System message(s) at the start + last `keep_recent` messages are protected
        let first_non_system = messages.iter().position(|m| m.role != "system").unwrap_or(1);
        let evict_end = if messages.len() > keep_recent {
            messages.len() - keep_recent
        } else {
            first_non_system
        };

        if evict_end <= first_non_system {
            return vec![];
        }

        messages[first_non_system..evict_end].to_vec()
    }

    /// Apply compaction: replace evictable messages with a summary message.
    /// `keep_recent` must match the value used in `messages_to_compact`.
    pub fn apply_compaction(messages: &mut Vec<ChatMessage>, summary: &str, model: &str, keep_recent: usize) {
        let first_non_system = messages.iter().position(|m| m.role != "system").unwrap_or(1);
        let evict_end = if messages.len() > keep_recent {
            messages.len() - keep_recent
        } else {
            return; // Nothing to compact
        };

        if evict_end <= first_non_system {
            return;
        }

        // Remove the evictable range
        messages.drain(first_non_system..evict_end);

        // Insert summary as a system message right after the original system prompt
        messages.insert(first_non_system, ChatMessage {
            role: "system".to_string(),
            content: format!("[Conversation summary of earlier messages]\n{}", summary),
            tool_calls: None,
            tool_call_id: None,
        });

        // If still over budget, fall back to hard trim
        let _ = model; // used for budget check
        Self::trim_messages(messages, model);
    }

    /// Build a prompt for the LLM to summarize old messages.
    pub fn build_summary_prompt(messages: &[ChatMessage]) -> String {
        let mut transcript = String::new();
        for msg in messages {
            let role = match msg.role.as_str() {
                "user" => "User",
                "assistant" => "Assistant",
                "tool" => "Tool result",
                _ => &msg.role,
            };
            if !msg.content.trim().is_empty() {
                transcript.push_str(&format!("{}: {}\n", role, msg.content.chars().take(500).collect::<String>()));
            }
        }
        format!(
            "Summarize the following conversation in 2-3 concise paragraphs. \
             Preserve key facts, decisions, and context. Be brief.\n\n{}",
            transcript
        )
    }

    /// Trim messages to fit within the model's context window.
    /// Always preserves: first system message + last user message.
    /// Removes oldest non-system messages when over budget.
    pub fn trim_messages(messages: &mut Vec<ChatMessage>, model: &str) {
        let window = Self::get_window_size(model);
        let budget = (window as f64 * TARGET_FILL_RATIO) as usize;

        let current = Self::estimate_messages_tokens(messages);
        if current <= budget {
            return; // Already within budget
        }

        // Strategy: keep system messages (index 0 typically) and remove
        // oldest non-system messages until within budget.
        // Always keep the last message (the user's current prompt).

        if messages.len() <= 2 {
            return; // Can't trim further (system + user)
        }

        // Find system messages and the last message
        let last_idx = messages.len() - 1;
        let mut protected: std::collections::HashSet<usize> = std::collections::HashSet::new();

        // Protect system messages
        for (i, m) in messages.iter().enumerate() {
            if m.role == "system" {
                protected.insert(i);
            }
        }
        // Protect the last message
        protected.insert(last_idx);

        // Remove from oldest non-protected until within budget
        let mut to_remove: Vec<usize> = Vec::new();
        for i in 0..messages.len() {
            if protected.contains(&i) {
                continue;
            }
            to_remove.push(i);
            // Check if removing these brings us under budget
            let remaining: Vec<&ChatMessage> = messages.iter().enumerate()
                .filter(|(idx, _)| !to_remove.contains(idx))
                .map(|(_, m)| m)
                .collect();
            let remaining_tokens: usize = remaining.iter().map(|m| {
                4 + Self::estimate_tokens(&m.content)
                    + m.tool_calls.as_ref().map(|tcs| {
                        tcs.iter().map(|tc| {
                            Self::estimate_tokens(&tc.function.name)
                                + Self::estimate_tokens(&tc.function.arguments.to_string())
                                + 4
                        }).sum::<usize>()
                    }).unwrap_or(0)
            }).sum();
            if remaining_tokens <= budget {
                break;
            }
        }

        // Remove in reverse order to preserve indices
        for i in to_remove.into_iter().rev() {
            messages.remove(i);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use crate::providers::{ToolCall, ToolCallFunction};

    #[test]
    fn test_estimate_tokens_empty() {
        assert_eq!(ContextOptimizer::estimate_tokens(""), 0);
    }

    #[test]
    fn test_estimate_tokens_basic() {
        // "hello" is 5 chars, (5+3)/4 = 2 tokens
        assert_eq!(ContextOptimizer::estimate_tokens("hello"), 2);
        // 100 chars ~ 25 tokens
        let text = "a".repeat(100);
        assert_eq!(ContextOptimizer::estimate_tokens(&text), 25);
    }

    #[test]
    fn test_estimate_messages_tokens() {
        let msgs = vec![
            ChatMessage { role: "user".into(), content: "hello world".into(), tool_calls: None, tool_call_id: None },
        ];
        let tokens = ContextOptimizer::estimate_messages_tokens(&msgs);
        // 4 overhead + (11+3)/4 = 4 + 3 = 7
        assert!(tokens > 0);
    }

    #[test]
    fn test_get_window_size_known() {
        assert_eq!(ContextOptimizer::get_window_size("qwen3:8b"), 32768);
        assert_eq!(ContextOptimizer::get_window_size("gpt-4o"), 128000);
        assert_eq!(ContextOptimizer::get_window_size("claude-sonnet-4-6"), 200000);
    }

    #[test]
    fn test_get_window_size_unknown() {
        assert_eq!(ContextOptimizer::get_window_size("some-unknown-model"), DEFAULT_WINDOW);
    }

    #[test]
    fn test_get_window_size_prefix_match() {
        // "qwen3:8b-q4_0" should match "qwen3:8b"
        assert_eq!(ContextOptimizer::get_window_size("qwen3:8b-q4_0"), 32768);
    }

    #[test]
    fn test_trim_messages_within_budget() {
        let mut msgs = vec![
            ChatMessage { role: "system".into(), content: "sys".into(), tool_calls: None, tool_call_id: None },
            ChatMessage { role: "user".into(), content: "hi".into(), tool_calls: None, tool_call_id: None },
        ];
        let original_len = msgs.len();
        ContextOptimizer::trim_messages(&mut msgs, "gpt-4o"); // huge window
        assert_eq!(msgs.len(), original_len);
    }

    #[test]
    fn test_trim_messages_over_budget() {
        // Create many messages that exceed a small model's window
        let mut msgs = vec![
            ChatMessage { role: "system".into(), content: "system prompt".into(), tool_calls: None, tool_call_id: None },
        ];
        // Add 100 large messages (~130 tokens each = ~13,000 total, budget is ~6,553)
        for i in 0..100 {
            msgs.push(ChatMessage {
                role: if i % 2 == 0 { "user".into() } else { "assistant".into() },
                content: "x".repeat(500),
                tool_calls: None,
                tool_call_id: None,
            });
        }
        let original_len = msgs.len();
        ContextOptimizer::trim_messages(&mut msgs, "llama3:8b"); // 8192 window
        assert!(msgs.len() < original_len, "Should have trimmed messages");
        // System message should be preserved
        assert_eq!(msgs[0].role, "system");
        // Last message should be preserved
        assert_eq!(msgs.last().unwrap().content, "x".repeat(500));
    }

    #[test]
    fn test_trim_preserves_system_and_last() {
        let mut msgs = vec![
            ChatMessage { role: "system".into(), content: "a".repeat(1000), tool_calls: None, tool_call_id: None },
            ChatMessage { role: "user".into(), content: "b".repeat(1000), tool_calls: None, tool_call_id: None },
            ChatMessage { role: "assistant".into(), content: "c".repeat(1000), tool_calls: None, tool_call_id: None },
            ChatMessage { role: "user".into(), content: "d".repeat(1000), tool_calls: None, tool_call_id: None },
        ];
        // With a very small window, middle messages get dropped
        ContextOptimizer::trim_messages(&mut msgs, "some-tiny-model");
        // System should still be first
        assert_eq!(msgs[0].role, "system");
        // Last user message should still be last
        assert_eq!(msgs.last().unwrap().role, "user");
        assert_eq!(msgs.last().unwrap().content, "d".repeat(1000));
    }

    #[test]
    fn test_estimate_with_tool_calls() {
        let msgs = vec![
            ChatMessage {
                role: "assistant".into(),
                content: "".into(),
                tool_calls: Some(vec![ToolCall {
                    id: Some("1".into()),
                    function: ToolCallFunction {
                        name: "shell".into(),
                        arguments: json!({"command": "ls -la"}),
                    },
                }]),
                tool_call_id: None,
            },
        ];
        let tokens = ContextOptimizer::estimate_messages_tokens(&msgs);
        assert!(tokens > 4); // overhead + tool call tokens
    }

    #[test]
    fn test_default_window() {
        assert_eq!(DEFAULT_WINDOW, 8192);
    }

    #[test]
    fn test_target_fill_ratio() {
        assert!((TARGET_FILL_RATIO - 0.80).abs() < f64::EPSILON);
    }

    // ── Compaction tests ─────────────────────────────────────────────────────

    #[test]
    fn test_needs_compaction_under_budget() {
        let msgs = vec![
            ChatMessage { role: "system".into(), content: "sys".into(), tool_calls: None, tool_call_id: None },
            ChatMessage { role: "user".into(), content: "hi".into(), tool_calls: None, tool_call_id: None },
        ];
        assert!(!ContextOptimizer::needs_compaction(&msgs, "gpt-4o"));
    }

    #[test]
    fn test_needs_compaction_over_budget() {
        let mut msgs = vec![
            ChatMessage { role: "system".into(), content: "sys".into(), tool_calls: None, tool_call_id: None },
        ];
        for _ in 0..200 {
            msgs.push(ChatMessage {
                role: "user".into(), content: "x".repeat(500), tool_calls: None, tool_call_id: None,
            });
        }
        assert!(ContextOptimizer::needs_compaction(&msgs, "llama3:8b"));
    }

    #[test]
    fn test_messages_to_compact_returns_middle() {
        let mut msgs = vec![
            ChatMessage { role: "system".into(), content: "sys prompt".into(), tool_calls: None, tool_call_id: None },
        ];
        // Add 100 large messages to exceed llama3:8b's budget (~6553 tokens)
        for i in 0..100 {
            msgs.push(ChatMessage {
                role: if i % 2 == 0 { "user".into() } else { "assistant".into() },
                content: "x".repeat(500),
                tool_calls: None,
                tool_call_id: None,
            });
        }
        let to_compact = ContextOptimizer::messages_to_compact(&msgs, "llama3:8b", 4);
        // Should return messages between system and last 4
        assert!(!to_compact.is_empty());
        // Should not include system or last 4
        assert!(to_compact.len() <= msgs.len() - 5); // 1 system + 4 recent
    }

    #[test]
    fn test_messages_to_compact_short_conversation() {
        let msgs = vec![
            ChatMessage { role: "system".into(), content: "sys".into(), tool_calls: None, tool_call_id: None },
            ChatMessage { role: "user".into(), content: "hi".into(), tool_calls: None, tool_call_id: None },
        ];
        let to_compact = ContextOptimizer::messages_to_compact(&msgs, "gpt-4o", 4);
        assert!(to_compact.is_empty()); // Too short to compact
    }

    #[test]
    fn test_apply_compaction() {
        let mut msgs = vec![
            ChatMessage { role: "system".into(), content: "sys".into(), tool_calls: None, tool_call_id: None },
        ];
        for i in 0..10 {
            msgs.push(ChatMessage {
                role: if i % 2 == 0 { "user".into() } else { "assistant".into() },
                content: format!("msg_{}", i),
                tool_calls: None,
                tool_call_id: None,
            });
        }
        let original_len = msgs.len(); // 11
        ContextOptimizer::apply_compaction(&mut msgs, "Summary of conversation", "gpt-4o", 4);
        // Should be: system + summary + last 4 = 6
        assert!(msgs.len() < original_len);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[0].content, "sys");
        assert!(msgs[1].content.contains("[Conversation summary"));
        assert!(msgs[1].content.contains("Summary of conversation"));
    }

    #[test]
    fn test_build_summary_prompt() {
        let msgs = vec![
            ChatMessage { role: "user".into(), content: "What is Rust?".into(), tool_calls: None, tool_call_id: None },
            ChatMessage { role: "assistant".into(), content: "Rust is a systems programming language.".into(), tool_calls: None, tool_call_id: None },
        ];
        let prompt = ContextOptimizer::build_summary_prompt(&msgs);
        assert!(prompt.contains("Summarize"));
        assert!(prompt.contains("User: What is Rust?"));
        assert!(prompt.contains("Assistant: Rust is a systems"));
    }
}
