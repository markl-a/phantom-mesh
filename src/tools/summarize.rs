//! Text summarization tool — extractive summarization (no external API needed).

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

use super::{Tool, ToolResult};

pub struct SummarizeTool;

impl SummarizeTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl Tool for SummarizeTool {
    fn name(&self) -> &str { "summarize" }

    fn description(&self) -> &str {
        "Summarize text using extractive summarization. No external API needed."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "Text to summarize" },
                "max_sentences": { "type": "integer", "description": "Max sentences in summary (default 3)" },
                "style": { "type": "string", "description": "'extractive' (default) or 'bullets'" }
            },
            "required": ["text"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let text = args["text"].as_str().unwrap_or("").trim();
        let max_sentences = args["max_sentences"].as_u64().unwrap_or(3) as usize;
        let style = args["style"].as_str().unwrap_or("extractive").trim();

        if text.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing required parameter: text".into() });
        }

        // Split into sentences
        let sentences = split_sentences(text);
        if sentences.is_empty() {
            return Ok(ToolResult { success: false, output: "No sentences found in text".into() });
        }

        // If text is already short enough, return as-is
        if sentences.len() <= max_sentences {
            let output = if style == "bullets" {
                sentences.iter().map(|s| format!("- {}", s)).collect::<Vec<_>>().join("\n")
            } else {
                sentences.join(" ")
            };
            return Ok(ToolResult { success: true, output });
        }

        // Score sentences
        let word_freq = build_word_frequency(&sentences);
        let mut scored: Vec<(usize, f64)> = sentences.iter()
            .enumerate()
            .map(|(i, s)| (i, score_sentence(s, &word_freq, i, sentences.len())))
            .collect();

        // Sort by score (descending) and pick top N
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let mut selected: Vec<usize> = scored.iter()
            .take(max_sentences)
            .map(|(i, _)| *i)
            .collect();

        // Sort by original position to maintain flow
        selected.sort();

        let summary_sentences: Vec<&str> = selected.iter()
            .map(|&i| sentences[i].as_str())
            .collect();

        let output = if style == "bullets" {
            summary_sentences.iter().map(|s| format!("- {}", s)).collect::<Vec<_>>().join("\n")
        } else {
            summary_sentences.join(" ")
        };

        Ok(ToolResult { success: true, output })
    }
}

/// Split text into sentences
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        current.push(ch);
        if (ch == '.' || ch == '!' || ch == '?') && current.trim().len() > 10 {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                sentences.push(trimmed);
            }
            current.clear();
        }
    }

    // Don't forget remaining text
    let remaining = current.trim().to_string();
    if !remaining.is_empty() && remaining.len() > 5 {
        sentences.push(remaining);
    }

    sentences
}

/// Build word frequency map (lowercased, excluding stop words)
fn build_word_frequency(sentences: &[String]) -> HashMap<String, usize> {
    let stop_words = [
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being",
        "have", "has", "had", "do", "does", "did", "will", "would", "could",
        "should", "may", "might", "can", "shall", "to", "of", "in", "for",
        "on", "with", "at", "by", "from", "as", "into", "through", "during",
        "before", "after", "above", "below", "between", "out", "off", "over",
        "under", "again", "further", "then", "once", "and", "but", "or", "nor",
        "not", "so", "yet", "both", "either", "neither", "each", "every",
        "all", "any", "few", "more", "most", "other", "some", "such",
        "no", "only", "own", "same", "than", "too", "very", "just",
        "it", "its", "this", "that", "these", "those", "i", "you", "he",
        "she", "we", "they", "me", "him", "her", "us", "them", "my", "your",
        "his", "our", "their", "what", "which", "who", "whom", "how",
    ];

    let mut freq = HashMap::new();
    for sentence in sentences {
        for word in sentence.split_whitespace() {
            let word = word.to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>();
            if word.len() > 2 && !stop_words.contains(&word.as_str()) {
                *freq.entry(word).or_insert(0) += 1;
            }
        }
    }
    freq
}

/// Score a sentence based on word frequency, position, and length
fn score_sentence(sentence: &str, word_freq: &HashMap<String, usize>, position: usize, total: usize) -> f64 {
    let words: Vec<String> = sentence.split_whitespace()
        .map(|w| w.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect())
        .filter(|w: &String| w.len() > 2)
        .collect();

    if words.is_empty() { return 0.0; }

    // Word frequency score
    let freq_score: f64 = words.iter()
        .map(|w| *word_freq.get(w).unwrap_or(&0) as f64)
        .sum::<f64>() / words.len() as f64;

    // Position bonus (first and last sentences get bonus)
    let position_score = if position == 0 {
        1.5
    } else if position == total - 1 {
        1.2
    } else {
        1.0
    };

    // Length penalty (too short or too long sentences are less useful)
    let len = words.len();
    let length_score = if len < 5 {
        0.7
    } else if len > 30 {
        0.8
    } else {
        1.0
    };

    freq_score * position_score * length_score
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_TEXT: &str = "Artificial intelligence is transforming the world. \
Machine learning algorithms are becoming more sophisticated. \
Deep learning has achieved remarkable results in image recognition. \
Natural language processing enables computers to understand human language. \
The future of AI holds tremendous potential for society.";

    #[test]
    fn test_name() {
        assert_eq!(SummarizeTool::new().name(), "summarize");
    }

    #[test]
    fn test_schema() {
        let tool = SummarizeTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["text"].is_object());
        assert_eq!(schema["required"][0], "text");
    }

    #[tokio::test]
    async fn test_short_text() {
        let tool = SummarizeTool::new();
        let result = tool.execute(json!({"text": "Hello world. This is short."})).await.unwrap();
        assert!(result.success);
        // Short text returned as-is
    }

    #[tokio::test]
    async fn test_long_text() {
        let tool = SummarizeTool::new();
        let result = tool.execute(json!({"text": TEST_TEXT, "max_sentences": 2})).await.unwrap();
        assert!(result.success);
        // Should be shorter than original
        assert!(result.output.len() < TEST_TEXT.len());
    }

    #[tokio::test]
    async fn test_bullets_style() {
        let tool = SummarizeTool::new();
        let result = tool.execute(json!({"text": TEST_TEXT, "max_sentences": 2, "style": "bullets"})).await.unwrap();
        assert!(result.success);
        assert!(result.output.starts_with("- "));
        assert!(result.output.contains("\n- "));
    }

    #[tokio::test]
    async fn test_empty_text() {
        let tool = SummarizeTool::new();
        let result = tool.execute(json!({"text": ""})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_custom_max() {
        let tool = SummarizeTool::new();
        let result = tool.execute(json!({"text": TEST_TEXT, "max_sentences": 1})).await.unwrap();
        assert!(result.success);
        // Should contain only one sentence
        let periods = result.output.matches('.').count();
        assert!(periods <= 2); // at most 1-2 periods
    }

    #[test]
    fn test_split_sentences() {
        let sentences = split_sentences("Hello world. How are you today? I am fine!");
        assert_eq!(sentences.len(), 3);
    }

    #[test]
    fn test_description() {
        let tool = SummarizeTool::new();
        assert!(tool.description().contains("Summarize"));
    }
}
