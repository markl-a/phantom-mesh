//! StreamingThinkFilter — strips `<think>...</think>` blocks from LLM output.
//!
//! Qwen, DeepSeek, and other reasoning models emit `<think>` blocks containing
//! chain-of-thought reasoning. These are useful for debugging but should be
//! stripped from user-facing output and tool call parsing.
//!
//! Supports both:
//! - **Batch mode**: `strip_think_tags()` for complete response strings
//! - **Streaming mode**: `ThinkFilter` struct for incremental chunk processing
//!
//! Reference: OpenFang `ThinkFilter` (stateful streaming filter)

use once_cell::sync::Lazy;
use regex::Regex;

/// Regex that matches `<think>...</think>` blocks (including multiline, lazy)
static THINK_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?si)<think>.*?</think>").unwrap()
});

/// Regex for unclosed `<think>` at end of string (streaming edge case)
static THINK_OPEN_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?si)<think>[^<]*$").unwrap()
});

// ── Batch mode ──────────────────────────────────────────────────────────

/// Strip all `<think>...</think>` blocks from a complete string.
/// Also trims leading/trailing whitespace left behind.
pub fn strip_think_tags(text: &str) -> String {
    let result = THINK_RE.replace_all(text, "");
    // Clean up multiple newlines left behind
    let cleaned = result
        .lines()
        .filter(|line| !line.trim().is_empty() || true) // keep structure
        .collect::<Vec<_>>()
        .join("\n");
    // Trim leading newlines that were left after removing a leading <think> block
    cleaned.trim_start_matches('\n').to_string()
}

// ── Streaming mode ──────────────────────────────────────────────────────

/// Stateful filter for streaming chunks.
/// Buffers content inside `<think>` tags and only emits non-think content.
#[derive(Debug)]
pub struct ThinkFilter {
    /// Are we currently inside a `<think>` block?
    inside_think: bool,
    /// Buffer for partial tag detection at chunk boundaries
    buffer: String,
}

impl ThinkFilter {
    pub fn new() -> Self {
        Self {
            inside_think: false,
            buffer: String::new(),
        }
    }

    /// Process a streaming chunk. Returns the filtered output (may be empty
    /// if the chunk is entirely inside a think block).
    pub fn process_chunk(&mut self, chunk: &str) -> String {
        self.buffer.push_str(chunk);
        let mut output = String::new();

        loop {
            if self.inside_think {
                // Look for closing </think>
                if let Some(end_pos) = self.buffer.find("</think>") {
                    // Skip everything up to and including </think>
                    self.buffer = self.buffer[end_pos + 8..].to_string();
                    self.inside_think = false;
                    // Continue processing remaining buffer
                } else {
                    // Still inside think block, consume entire buffer
                    // But keep last 8 chars in case "</think>" spans chunks
                    if self.buffer.len() > 8 {
                        self.buffer = self.buffer[self.buffer.len() - 8..].to_string();
                    }
                    break;
                }
            } else {
                // Look for opening <think>
                if let Some(start_pos) = self.buffer.find("<think>") {
                    // Emit everything before <think>
                    output.push_str(&self.buffer[..start_pos]);
                    self.buffer = self.buffer[start_pos + 7..].to_string();
                    self.inside_think = true;
                    // Continue processing remaining buffer
                } else {
                    // No <think> found — emit buffer but keep last 7 chars
                    // in case "<think>" spans chunks
                    if self.buffer.len() > 7 {
                        let emit_end = self.buffer.len() - 7;
                        output.push_str(&self.buffer[..emit_end]);
                        self.buffer = self.buffer[emit_end..].to_string();
                    }
                    break;
                }
            }
        }

        output
    }

    /// Flush any remaining buffer content (call at end of stream).
    pub fn flush(&mut self) -> String {
        let remaining = std::mem::take(&mut self.buffer);
        if self.inside_think {
            // Unclosed think block — discard
            String::new()
        } else {
            remaining
        }
    }
}

impl Default for ThinkFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Batch mode tests ────────────────────────────────────────────────

    #[test]
    fn test_strip_simple() {
        let input = "<think>Let me reason about this...</think>The answer is 42.";
        assert_eq!(strip_think_tags(input), "The answer is 42.");
    }

    #[test]
    fn test_strip_multiline() {
        let input = "<think>\nStep 1: Consider...\nStep 2: Therefore...\n</think>\nHere is the result.";
        let result = strip_think_tags(input);
        assert!(result.contains("Here is the result."));
        assert!(!result.contains("Step 1"));
    }

    #[test]
    fn test_strip_multiple_blocks() {
        let input = "<think>first</think>Hello <think>second</think>World";
        assert_eq!(strip_think_tags(input), "Hello World");
    }

    #[test]
    fn test_strip_no_think_tags() {
        let input = "Just normal text without any tags.";
        assert_eq!(strip_think_tags(input), input);
    }

    #[test]
    fn test_strip_empty() {
        assert_eq!(strip_think_tags(""), "");
    }

    #[test]
    fn test_strip_only_think() {
        let input = "<think>All reasoning, no output</think>";
        let result = strip_think_tags(input);
        assert!(result.trim().is_empty());
    }

    #[test]
    fn test_strip_case_insensitive() {
        let input = "<THINK>reasoning</THINK>output";
        assert_eq!(strip_think_tags(input), "output");
    }

    #[test]
    fn test_strip_nested_angle_brackets() {
        let input = "<think>if x < 10 && y > 5 then...</think>Result here";
        // The regex is lazy, so it should handle < inside think blocks
        let result = strip_think_tags(input);
        assert!(result.contains("Result here"));
    }

    // ── Streaming mode tests ────────────────────────────────────────────

    #[test]
    fn test_stream_simple() {
        let mut filter = ThinkFilter::new();
        let out = filter.process_chunk("<think>reasoning</think>Hello");
        let flushed = filter.flush();
        assert_eq!(format!("{}{}", out, flushed), "Hello");
    }

    #[test]
    fn test_stream_split_across_chunks() {
        let mut filter = ThinkFilter::new();
        let o1 = filter.process_chunk("<think>reas");
        let o2 = filter.process_chunk("oning</think>He");
        let o3 = filter.process_chunk("llo World");
        let flushed = filter.flush();
        let result = format!("{}{}{}{}", o1, o2, o3, flushed);
        assert!(result.contains("Hello World"), "Got: {}", result);
        assert!(!result.contains("reasoning"), "Got: {}", result);
    }

    #[test]
    fn test_stream_no_think() {
        let mut filter = ThinkFilter::new();
        let o1 = filter.process_chunk("Hello ");
        let o2 = filter.process_chunk("World");
        let flushed = filter.flush();
        let result = format!("{}{}{}", o1, o2, flushed);
        assert_eq!(result, "Hello World");
    }

    #[test]
    fn test_stream_tag_split_at_boundary() {
        let mut filter = ThinkFilter::new();
        // "<think>" split across two chunks
        let o1 = filter.process_chunk("Hello <thi");
        let o2 = filter.process_chunk("nk>secret</think> World");
        let flushed = filter.flush();
        let result = format!("{}{}{}", o1, o2, flushed);
        assert!(result.contains("Hello"), "Got: {}", result);
        assert!(result.contains("World"), "Got: {}", result);
        assert!(!result.contains("secret"), "Got: {}", result);
    }

    #[test]
    fn test_stream_unclosed_think() {
        let mut filter = ThinkFilter::new();
        let o1 = filter.process_chunk("Hello <think>unclosed reasoning");
        let flushed = filter.flush();
        let result = format!("{}{}", o1, flushed);
        // Unclosed think block should be discarded
        assert!(result.contains("Hello"), "Got: {}", result);
        assert!(!result.contains("unclosed"), "Got: {}", result);
    }

    #[test]
    fn test_stream_multiple_blocks() {
        let mut filter = ThinkFilter::new();
        let o1 = filter.process_chunk("<think>a</think>X<think>b</think>Y");
        let flushed = filter.flush();
        let result = format!("{}{}", o1, flushed);
        assert!(result.contains("X"), "Got: {}", result);
        assert!(result.contains("Y"), "Got: {}", result);
        assert!(!result.contains("a"), "Got: {}", result);
        assert!(!result.contains("b"), "Got: {}", result);
    }
}
