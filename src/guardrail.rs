//! L1 Guardrail — zero-cost format validation for agent/hand outputs.
//!
//! Pure Rust checks (no LLM calls) that catch ~30% of garbage output:
//! - Minimum/maximum length
//! - Required sections (e.g., "## Introduction")
//! - Forbidden patterns (simplified Chinese, repeated paragraphs, etc.)
//!
//! Used at Hand phase transitions and agent output before delivery.
//!
//! Reference: CrewAI guardrail system (function-based validators)

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Result of a guardrail check
#[derive(Debug, Clone, Serialize)]
pub enum GuardrailResult {
    /// Output passed all checks
    Pass,
    /// Output failed one or more checks
    Fail {
        issues: Vec<String>,
        /// Suggested action: "retry" (fixable) or "reject" (unfixable)
        action: String,
    },
}

impl GuardrailResult {
    pub fn is_pass(&self) -> bool {
        matches!(self, GuardrailResult::Pass)
    }
}

/// Guardrail configuration — can be defined per-hand or per-phase
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GuardrailConfig {
    /// Minimum output length in characters (0 = no minimum)
    #[serde(default)]
    pub min_length: usize,
    /// Maximum output length in characters (0 = no maximum)
    #[serde(default)]
    pub max_length: usize,
    /// Sections that must appear in the output (e.g., ["## 結論", "## Summary"])
    #[serde(default)]
    pub required_sections: Vec<String>,
    /// Regex patterns that must NOT appear in the output
    #[serde(default)]
    pub forbidden_patterns: Vec<String>,
    /// Check for simplified Chinese characters (common Qwen issue)
    #[serde(default)]
    pub reject_simplified_chinese: bool,
    /// Check for excessive repetition (same paragraph repeated 3+ times)
    #[serde(default = "default_true")]
    pub reject_repetition: bool,
    /// Check for empty/placeholder output ("I don't know", "I cannot help")
    #[serde(default = "default_true")]
    pub reject_placeholder: bool,
}

fn default_true() -> bool { true }

impl Default for GuardrailConfig {
    fn default() -> Self {
        Self {
            min_length: 0,
            max_length: 0,
            required_sections: Vec::new(),
            forbidden_patterns: Vec::new(),
            reject_simplified_chinese: false,
            reject_repetition: true,
            reject_placeholder: true,
        }
    }
}

/// Validate output against guardrail config
pub fn validate(config: &GuardrailConfig, output: &str) -> GuardrailResult {
    let mut issues = Vec::new();

    // 1. Length checks
    if config.min_length > 0 && output.len() < config.min_length {
        issues.push(format!(
            "輸出太短: {} 字元 (最少 {})",
            output.len(), config.min_length
        ));
    }
    if config.max_length > 0 && output.len() > config.max_length {
        issues.push(format!(
            "輸出太長: {} 字元 (最多 {})",
            output.len(), config.max_length
        ));
    }

    // 2. Required sections
    for section in &config.required_sections {
        if !output.contains(section.as_str()) {
            issues.push(format!("缺少必要段落: {}", section));
        }
    }

    // 3. Forbidden patterns
    for pattern_str in &config.forbidden_patterns {
        if let Ok(re) = Regex::new(pattern_str) {
            if re.is_match(output) {
                issues.push(format!("包含禁止模式: {}", pattern_str));
            }
        }
    }

    // 4. Simplified Chinese detection
    if config.reject_simplified_chinese {
        let simplified_chars = detect_simplified_chinese(output);
        if !simplified_chars.is_empty() {
            issues.push(format!(
                "偵測到簡體中文字元: {} (應使用繁體中文)",
                simplified_chars.iter().take(5).collect::<String>()
            ));
        }
    }

    // 5. Repetition detection
    if config.reject_repetition {
        if let Some(repeated) = detect_repetition(output) {
            issues.push(format!("偵測到重複段落: \"{}...\"", &repeated[..repeated.len().min(50)]));
        }
    }

    // 6. Placeholder detection
    if config.reject_placeholder && is_placeholder(output) {
        issues.push("輸出是佔位符/拒絕回答 (placeholder response)".to_string());
    }

    if issues.is_empty() {
        GuardrailResult::Pass
    } else {
        let action = if issues.iter().any(|i| i.contains("佔位符") || i.contains("太短")) {
            "retry".to_string()
        } else {
            "retry".to_string() // most issues are retryable
        };
        GuardrailResult::Fail { issues, action }
    }
}

/// Quick validate with sensible defaults (no config needed)
pub fn quick_validate(output: &str) -> GuardrailResult {
    validate(&GuardrailConfig {
        min_length: 20,
        reject_repetition: true,
        reject_placeholder: true,
        ..Default::default()
    }, output)
}

/// Detect common simplified Chinese characters that should be traditional
fn detect_simplified_chinese(text: &str) -> Vec<char> {
    // Common simplified→traditional pairs where simplified is clearly wrong
    let simplified_only: &[char] = &[
        '与', '为', '书', '买', '产', '亲', '从', '众', '优', '会',
        '传', '伤', '体', '佣', '侠', '俭', '备', '复', '头', '夹',
        '奋', '妇', '学', '实', '对', '导', '专', '将', '尽', '层',
        '属', '带', '帮', '庆', '应', '开', '张', '当', '录', '归',
        '总', '据', '损', '换', '护', '报', '担', '择', '拥', '拦',
        '挡', '挤', '挥', '损', '摄', '数', '断', '无', '时', '显',
        '晓', '权', '条', '来', '杂', '构', '标', '样', '树', '业',
        '极', '档', '检', '欢', '残', '毕', '气', '汇', '没', '沟',
        '注', '济', '浏', '测', '准', '漏', '点', '热', '爱', '独',
        '环', '现', '理', '电', '画', '确', '码', '积', '称', '类',
        '紧', '纪', '纯', '纲', '练', '组', '织', '给', '统', '继',
        '绩', '续', '网', '罗', '联', '脑', '节', '范', '补', '观',
        '规', '觉', '计', '订', '认', '让', '记', '讨', '议', '设',
        '证', '评', '试', '话', '询', '课', '调', '谁', '论', '讲',
        '许', '诉', '语', '说', '请', '读', '质', '负', '费', '资',
        '购', '赢', '达', '运', '还', '进', '连', '选', '适', '通',
        '邮', '钱', '铁', '门', '间', '问', '阅', '阳', '队', '阶',
        '际', '险', '难', '集', '页', '顾', '须', '领', '题', '风',
        '饭', '馆', '验', '鱼', '鸡',
    ];

    text.chars()
        .filter(|c| simplified_only.contains(c))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect()
}

/// Detect if the same paragraph (50+ chars) appears 3+ times
fn detect_repetition(text: &str) -> Option<String> {
    let paragraphs: Vec<&str> = text.split("\n\n")
        .map(|p| p.trim())
        .filter(|p| p.len() >= 50)
        .collect();

    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for p in &paragraphs {
        *counts.entry(p).or_insert(0) += 1;
    }

    counts.into_iter()
        .find(|(_, count)| *count >= 3)
        .map(|(text, _)| text.to_string())
}

/// Detect placeholder/refusal responses
fn is_placeholder(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 10 {
        return true;
    }
    let lower = trimmed.to_lowercase();
    let placeholders = [
        "i don't know",
        "i cannot help",
        "i'm not sure",
        "as an ai",
        "i apologize",
        "i'm sorry, but i",
        "抱歉，我無法",
        "我不確定",
        "作為一個AI",
        "作为一个AI",
    ];
    placeholders.iter().any(|p| lower.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pass_valid_output() {
        let config = GuardrailConfig {
            min_length: 10,
            ..Default::default()
        };
        let result = validate(&config, "This is a perfectly valid output with enough content.");
        assert!(result.is_pass());
    }

    #[test]
    fn test_fail_too_short() {
        let config = GuardrailConfig {
            min_length: 100,
            ..Default::default()
        };
        let result = validate(&config, "Too short.");
        assert!(!result.is_pass());
        if let GuardrailResult::Fail { issues, .. } = result {
            assert!(issues[0].contains("太短"));
        }
    }

    #[test]
    fn test_fail_too_long() {
        let config = GuardrailConfig {
            max_length: 10,
            ..Default::default()
        };
        let result = validate(&config, "This output is way too long for the limit.");
        assert!(!result.is_pass());
    }

    #[test]
    fn test_fail_missing_section() {
        let config = GuardrailConfig {
            required_sections: vec!["## Conclusion".to_string()],
            ..Default::default()
        };
        let result = validate(&config, "## Introduction\nSome content here.\n## Analysis\nMore content.");
        assert!(!result.is_pass());
        if let GuardrailResult::Fail { issues, .. } = result {
            assert!(issues[0].contains("Conclusion"));
        }
    }

    #[test]
    fn test_pass_with_required_section() {
        let config = GuardrailConfig {
            required_sections: vec!["## Conclusion".to_string()],
            ..Default::default()
        };
        let result = validate(&config, "## Introduction\nContent.\n## Conclusion\nFinal thoughts.");
        assert!(result.is_pass());
    }

    #[test]
    fn test_fail_forbidden_pattern() {
        let config = GuardrailConfig {
            forbidden_patterns: vec!["TODO".to_string()],
            ..Default::default()
        };
        let result = validate(&config, "Here is the output. TODO: finish this later.");
        assert!(!result.is_pass());
    }

    #[test]
    fn test_detect_simplified_chinese() {
        let chars = detect_simplified_chinese("这是简体中文的测试");
        assert!(!chars.is_empty()); // Should detect simplified chars
    }

    #[test]
    fn test_no_simplified_in_traditional() {
        let chars = detect_simplified_chinese("這是繁體中文的測試");
        assert!(chars.is_empty()); // Traditional should pass
    }

    #[test]
    fn test_reject_simplified_chinese() {
        let config = GuardrailConfig {
            reject_simplified_chinese: true,
            ..Default::default()
        };
        let result = validate(&config, "这是一个简体中文的输出结果。");
        assert!(!result.is_pass());
    }

    #[test]
    fn test_detect_repetition() {
        let text = "A normal paragraph.\n\n\
            This is a repeated paragraph that is long enough to be detected by the filter mechanism.\n\n\
            Some other content.\n\n\
            This is a repeated paragraph that is long enough to be detected by the filter mechanism.\n\n\
            More content.\n\n\
            This is a repeated paragraph that is long enough to be detected by the filter mechanism.";
        assert!(detect_repetition(text).is_some());
    }

    #[test]
    fn test_no_repetition() {
        let text = "Paragraph one.\n\nParagraph two.\n\nParagraph three.";
        assert!(detect_repetition(text).is_none());
    }

    #[test]
    fn test_placeholder_detection() {
        assert!(is_placeholder("I don't know"));
        assert!(is_placeholder("I'm sorry, but I cannot help with that."));
        assert!(is_placeholder("抱歉，我無法回答這個問題。"));
        assert!(is_placeholder("")); // empty = placeholder
        assert!(is_placeholder("hi")); // too short
    }

    #[test]
    fn test_not_placeholder() {
        assert!(!is_placeholder("Here is a detailed analysis of the market trends for 2026."));
    }

    #[test]
    fn test_quick_validate_pass() {
        let result = quick_validate("Here is a detailed analysis of the market trends for 2026, covering multiple sectors.");
        assert!(result.is_pass());
    }

    #[test]
    fn test_quick_validate_fail_placeholder() {
        let result = quick_validate("I don't know the answer to that.");
        assert!(!result.is_pass());
    }

    #[test]
    fn test_default_config() {
        let config = GuardrailConfig::default();
        assert_eq!(config.min_length, 0);
        assert_eq!(config.max_length, 0);
        assert!(config.required_sections.is_empty());
        assert!(config.reject_repetition);
        assert!(config.reject_placeholder);
    }
}
