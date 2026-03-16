// Self-correction evaluate step — LLM-as-Judge pattern
//
// After an agent completes a task, optionally run an evaluation step:
//   1. Judge: LLM reviews the agent's output against the original prompt
//   2. Score: 1-5 quality rating
//   3. Retry: If score < threshold, retry with judge's feedback
//
// Inspired by OpenFang's quality gates and IronClaw's validation pipeline.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::llm_router::{ChatMessage, LlmRouter};

// ── Types ─────────────────────────────────────────────────────────────────────

/// Evaluation configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EvalConfig {
    /// Enable self-correction (default: false)
    #[serde(default)]
    pub enabled: bool,
    /// Minimum acceptable quality score (1-5, default: 3)
    #[serde(default = "default_threshold")]
    pub threshold: u8,
    /// Maximum retry attempts (default: 2)
    #[serde(default = "default_max_retries")]
    pub max_retries: u8,
    /// Provider to use for evaluation (default: same as agent)
    #[serde(default)]
    pub provider: Option<String>,
    /// Model override for evaluation
    #[serde(default)]
    pub model: Option<String>,
}

fn default_threshold() -> u8 { 3 }
fn default_max_retries() -> u8 { 2 }

impl Default for EvalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: default_threshold(),
            max_retries: default_max_retries(),
            provider: None,
            model: None,
        }
    }
}

/// Result of an evaluation
#[derive(Debug, Clone, Serialize)]
pub struct EvalResult {
    /// Quality score (1-5)
    pub score: u8,
    /// Brief reasoning from the judge
    pub reasoning: String,
    /// Specific feedback for improvement (if score < threshold)
    pub feedback: Option<String>,
    /// Whether the output passed the quality threshold
    pub passed: bool,
}

// ── Evaluation Logic ──────────────────────────────────────────────────────────

const EVAL_SYSTEM_PROMPT: &str = r#"You are a quality evaluator for AI agent outputs.
Your job is to judge whether the agent's response adequately addresses the user's request.

Score the response on a scale of 1-5:
  5 = Excellent — fully addresses the request with high quality
  4 = Good — addresses the request with minor issues
  3 = Acceptable — mostly addresses the request but has notable gaps
  2 = Poor — partially addresses the request with significant issues
  1 = Failing — does not address the request or is completely wrong

You MUST respond in EXACTLY this JSON format:
{"score": <1-5>, "reasoning": "<brief explanation>", "feedback": "<specific improvement suggestions or null>"}

Important:
- Be fair but strict. Only give 5 for truly excellent work.
- If the agent used tools correctly and got the right result, that's at least a 4.
- If the response is off-topic or clearly wrong, score 1-2.
- Keep reasoning under 100 words.
- feedback should be null if score >= 4, otherwise provide actionable suggestions."#;

/// Evaluate an agent's output using LLM-as-Judge
pub async fn evaluate(
    router: &LlmRouter,
    user_prompt: &str,
    agent_output: &str,
    config: &EvalConfig,
) -> Result<EvalResult> {
    let provider = config.provider.as_deref().unwrap_or("auto");

    let eval_prompt = format!(
        "## User's Request\n{}\n\n## Agent's Response\n{}\n\nEvaluate the agent's response. Respond with JSON only.",
        truncate(user_prompt, 1000),
        truncate(agent_output, 3000),
    );

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: EVAL_SYSTEM_PROMPT.to_string(),
            tool_calls: None,
            tool_call_id: None,
        },
        ChatMessage {
            role: "user".to_string(),
            content: eval_prompt,
            tool_calls: None,
            tool_call_id: None,
        },
    ];

    let response = router.chat_with_tools(&messages, &[], provider).await?;
    let text = response.message.content.trim().to_string();

    // Parse the JSON response
    parse_eval_response(&text, config.threshold)
}

/// Parse the LLM judge response into an EvalResult
fn parse_eval_response(text: &str, threshold: u8) -> Result<EvalResult> {
    // Try to extract JSON from the response (LLM might wrap it in markdown)
    let json_str = extract_json(text);

    let parsed: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| anyhow!("Failed to parse eval JSON: {} — raw: {}", e, text))?;

    let score = parsed
        .get("score")
        .and_then(|v| v.as_u64())
        .map(|v| v.clamp(1, 5) as u8)
        .ok_or_else(|| anyhow!("Missing 'score' in eval response"))?;

    let reasoning = parsed
        .get("reasoning")
        .and_then(|v| v.as_str())
        .unwrap_or("No reasoning provided")
        .to_string();

    let feedback = parsed
        .get("feedback")
        .and_then(|v| {
            if v.is_null() {
                None
            } else {
                v.as_str().map(String::from)
            }
        });

    Ok(EvalResult {
        score,
        reasoning,
        feedback,
        passed: score >= threshold,
    })
}

/// Extract JSON from LLM response (handles markdown code blocks)
fn extract_json(text: &str) -> String {
    // Try to find JSON block in markdown
    if let Some(start) = text.find("```json") {
        if let Some(end) = text[start + 7..].find("```") {
            return text[start + 7..start + 7 + end].trim().to_string();
        }
    }
    if let Some(start) = text.find("```") {
        if let Some(end) = text[start + 3..].find("```") {
            let inner = text[start + 3..start + 3 + end].trim();
            if inner.starts_with('{') {
                return inner.to_string();
            }
        }
    }
    // Try to find raw JSON object
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            return text[start..=end].to_string();
        }
    }
    text.to_string()
}

/// Run evaluate-and-retry loop
/// Returns (final_output, eval_results_per_attempt)
pub async fn evaluate_with_retry<F, Fut>(
    router: &LlmRouter,
    config: &EvalConfig,
    user_prompt: &str,
    mut run_agent: F,
) -> Result<(String, Vec<EvalResult>)>
where
    F: FnMut(Option<&str>) -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    let mut eval_results = Vec::new();
    let mut last_output;

    // First attempt (no feedback)
    last_output = run_agent(None).await?;

    for attempt in 0..=config.max_retries {
        let eval = evaluate(router, user_prompt, &last_output, config).await;

        match eval {
            Ok(result) => {
                info!(
                    "Eval attempt {}: score={}/5 (threshold={}) — {}",
                    attempt + 1,
                    result.score,
                    config.threshold,
                    if result.passed { "PASS" } else { "RETRY" }
                );

                let passed = result.passed;
                let feedback = result.feedback.clone();
                eval_results.push(result);

                if passed || attempt as u8 >= config.max_retries {
                    if !passed {
                        warn!(
                            "Eval: max retries ({}) reached, returning best effort",
                            config.max_retries
                        );
                    }
                    return Ok((last_output, eval_results));
                }

                // Retry with feedback
                if let Some(ref fb) = feedback {
                    debug!("Retrying with feedback: {}", fb);
                    last_output = run_agent(Some(fb)).await?;
                } else {
                    debug!("No specific feedback, retrying without modification");
                    last_output = run_agent(None).await?;
                }
            }
            Err(e) => {
                warn!("Eval failed (skipping): {}", e);
                // If evaluation itself fails, just return the output
                return Ok((last_output, eval_results));
            }
        }
    }

    Ok((last_output, eval_results))
}

fn truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_raw() {
        let input = r#"{"score": 4, "reasoning": "Good", "feedback": null}"#;
        assert_eq!(extract_json(input), input);
    }

    #[test]
    fn test_extract_json_markdown() {
        let input = "Here's my evaluation:\n```json\n{\"score\": 3, \"reasoning\": \"OK\", \"feedback\": \"Be more specific\"}\n```";
        let json = extract_json(input);
        assert!(json.starts_with('{'));
        assert!(json.contains("\"score\": 3"));
    }

    #[test]
    fn test_extract_json_with_surrounding_text() {
        let input = "Sure, here's the evaluation: {\"score\": 5, \"reasoning\": \"Excellent\", \"feedback\": null} That's my assessment.";
        let json = extract_json(input);
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));
    }

    #[test]
    fn test_parse_eval_response_pass() {
        let text = r#"{"score": 4, "reasoning": "Good response", "feedback": null}"#;
        let result = parse_eval_response(text, 3).unwrap();
        assert_eq!(result.score, 4);
        assert!(result.passed);
        assert!(result.feedback.is_none());
    }

    #[test]
    fn test_parse_eval_response_fail() {
        let text = r#"{"score": 2, "reasoning": "Off topic", "feedback": "Focus on the user's question"}"#;
        let result = parse_eval_response(text, 3).unwrap();
        assert_eq!(result.score, 2);
        assert!(!result.passed);
        assert_eq!(result.feedback.unwrap(), "Focus on the user's question");
    }

    #[test]
    fn test_parse_eval_clamp_score() {
        let text = r#"{"score": 10, "reasoning": "Amazing", "feedback": null}"#;
        let result = parse_eval_response(text, 3).unwrap();
        assert_eq!(result.score, 5); // Clamped to 5
    }

    #[test]
    fn test_parse_eval_missing_score() {
        let text = r#"{"reasoning": "No score given"}"#;
        assert!(parse_eval_response(text, 3).is_err());
    }

    #[test]
    fn test_default_config() {
        let config = EvalConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.threshold, 3);
        assert_eq!(config.max_retries, 2);
    }

    #[test]
    fn test_eval_config_from_toml() {
        let toml_str = r#"
enabled = true
threshold = 4
max_retries = 3
provider = "ollama"
"#;
        let config: EvalConfig = toml::from_str(toml_str).unwrap();
        assert!(config.enabled);
        assert_eq!(config.threshold, 4);
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.provider.unwrap(), "ollama");
    }
}
