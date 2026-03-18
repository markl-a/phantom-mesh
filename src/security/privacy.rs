//! Privacy Guard — regex-based sensitivity classifier for privacy-aware provider routing.
//!
//! Routes messages to local or cloud providers based on detected data sensitivity:
//! - Critical (API keys, SSN, credit cards) → local provider only
//! - Sensitive (email, phone, PII) → prefer local
//! - Internal (general business) → configurable (e.g. fireworks)
//! - Public (general knowledge) → fastest cloud provider

use regex::Regex;
use serde::Deserialize;
use std::fmt;
use tracing::debug;

use crate::providers::ChatMessage;

// ── PrivacyTier ─────────────────────────────────────────────────────────────

/// Sensitivity level for a message or conversation.
/// Higher ordinal = more sensitive. Classification takes the max across all messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrivacyTier {
    Public   = 0,
    Internal = 1,
    Sensitive = 2,
    Critical = 3,
}

impl fmt::Display for PrivacyTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Public   => write!(f, "public"),
            Self::Internal => write!(f, "internal"),
            Self::Sensitive => write!(f, "sensitive"),
            Self::Critical => write!(f, "critical"),
        }
    }
}

impl PrivacyTier {
    fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "critical" => Self::Critical,
            "sensitive" => Self::Sensitive,
            "internal" => Self::Internal,
            "public" => Self::Public,
            _ => Self::Internal,
        }
    }
}

// ── PrivacyConfig ───────────────────────────────────────────────────────────

/// Configuration for the Privacy Guard, parsed from `[privacy]` in agents.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct PrivacyConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_local_provider")]
    pub critical_provider: String,
    #[serde(default = "default_local_provider")]
    pub sensitive_provider: String,
    #[serde(default = "default_internal_provider")]
    pub internal_provider: String,
    #[serde(default = "default_public_provider")]
    pub public_provider: String,
    #[serde(default = "default_tier_str")]
    pub default_tier: String,
    #[serde(default)]
    pub critical_keywords: Vec<String>,
    #[serde(default)]
    pub sensitive_keywords: Vec<String>,
}

fn default_local_provider() -> String { "ollama".to_string() }
fn default_internal_provider() -> String { "fireworks".to_string() }
fn default_public_provider() -> String { "groq".to_string() }
fn default_tier_str() -> String { "internal".to_string() }

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            critical_provider: default_local_provider(),
            sensitive_provider: default_local_provider(),
            internal_provider: default_internal_provider(),
            public_provider: default_public_provider(),
            default_tier: default_tier_str(),
            critical_keywords: Vec::new(),
            sensitive_keywords: Vec::new(),
        }
    }
}

// ── PrivacyGuard ────────────────────────────────────────────────────────────

/// Regex-based privacy classifier. Zero LLM calls — pure pattern matching.
pub struct PrivacyGuard {
    config: PrivacyConfig,
    critical_patterns: Vec<Regex>,
    sensitive_patterns: Vec<Regex>,
    custom_critical_patterns: Vec<Regex>,
    custom_sensitive_patterns: Vec<Regex>,
    default_tier: PrivacyTier,
}

impl PrivacyGuard {
    pub fn new(config: PrivacyConfig) -> Self {
        let default_tier = PrivacyTier::from_str_loose(&config.default_tier);

        // Built-in critical patterns (must stay local)
        let critical_patterns = vec![
            // SSN (US)
            Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap(),
            // Credit card (13-19 digits, optionally separated by spaces/dashes)
            Regex::new(r"\b(?:\d[ -]*?){13,19}\b").unwrap(),
            // API key patterns
            Regex::new(r"(?i)(?:api_key|secret_key|private_key|access_token|api[-_]?secret)\s*[:=]\s*\S+").unwrap(),
            // AWS access key
            Regex::new(r"\bAKIA[0-9A-Z]{16}\b").unwrap(),
            // Password in context
            Regex::new(r"(?i)(?:password|passwd|pwd)\s*[:=]\s*\S+").unwrap(),
            // PEM private key block
            Regex::new(r"-----BEGIN (?:RSA |EC |DSA )?PRIVATE KEY-----").unwrap(),
            // JWT token (3 base64 segments separated by dots)
            Regex::new(r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b").unwrap(),
            // Generic secret/token with value
            Regex::new(r#"(?i)(?:secret|token|credential)\s*[:=]\s*['"]?[A-Za-z0-9_\-/+=]{16,}"#).unwrap(),
        ];

        // Built-in sensitive patterns (PII — prefer local)
        let sensitive_patterns = vec![
            // Email address
            Regex::new(r"\b[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}\b").unwrap(),
            // Phone (international formats)
            Regex::new(r"(?:\+\d{1,3}[-.\s]?)?\(?\d{2,4}\)?[-.\s]?\d{3,4}[-.\s]?\d{3,4}\b").unwrap(),
            // Street address patterns
            Regex::new(r"(?i)\b\d+\s+\w+\s+(?:street|st|avenue|ave|road|rd|boulevard|blvd|drive|dr|lane|ln|court|ct)\b").unwrap(),
            // Taiwan national ID
            Regex::new(r"\b[A-Z]\d{9}\b").unwrap(),
            // IPv4 address
            Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").unwrap(),
            // Date of birth patterns
            Regex::new(r"(?i)(?:birth|dob|born)\s*[:=]?\s*\d{4}[-/]\d{2}[-/]\d{2}").unwrap(),
        ];

        // Custom patterns from config
        let custom_critical_patterns: Vec<Regex> = config.critical_keywords.iter()
            .filter_map(|kw| {
                Regex::new(&format!(r"(?i){}", regex::escape(kw)))
                    .ok()
            })
            .collect();

        let custom_sensitive_patterns: Vec<Regex> = config.sensitive_keywords.iter()
            .filter_map(|kw| {
                Regex::new(&format!(r"(?i){}", regex::escape(kw)))
                    .ok()
            })
            .collect();

        Self {
            config,
            critical_patterns,
            sensitive_patterns,
            custom_critical_patterns,
            custom_sensitive_patterns,
            default_tier,
        }
    }

    /// Classify a set of messages by scanning user and tool messages for sensitive data.
    /// Returns the highest (most restrictive) tier found, or the configured default.
    pub fn classify(&self, messages: &[ChatMessage]) -> PrivacyTier {
        let mut max_tier = self.default_tier;

        for msg in messages {
            // Only scan user and tool messages — skip system/assistant
            if msg.role != "user" && msg.role != "tool" {
                continue;
            }

            let tier = self.classify_text(&msg.content);
            if tier > max_tier {
                max_tier = tier;
                // Short-circuit: can't go higher than Critical
                if max_tier == PrivacyTier::Critical {
                    return PrivacyTier::Critical;
                }
            }
        }

        max_tier
    }

    /// Classify a single text string.
    fn classify_text(&self, text: &str) -> PrivacyTier {
        if text.is_empty() {
            return PrivacyTier::Public;
        }

        // Check critical patterns first (highest priority)
        for pat in &self.critical_patterns {
            if pat.is_match(text) {
                debug!("PrivacyGuard: critical pattern matched: {}", pat.as_str());
                return PrivacyTier::Critical;
            }
        }
        for pat in &self.custom_critical_patterns {
            if pat.is_match(text) {
                debug!("PrivacyGuard: custom critical keyword matched: {}", pat.as_str());
                return PrivacyTier::Critical;
            }
        }

        // Check sensitive patterns
        for pat in &self.sensitive_patterns {
            if pat.is_match(text) {
                debug!("PrivacyGuard: sensitive pattern matched: {}", pat.as_str());
                return PrivacyTier::Sensitive;
            }
        }
        for pat in &self.custom_sensitive_patterns {
            if pat.is_match(text) {
                debug!("PrivacyGuard: custom sensitive keyword matched: {}", pat.as_str());
                return PrivacyTier::Sensitive;
            }
        }

        PrivacyTier::Public
    }

    /// Resolve a privacy tier to its configured provider name.
    pub fn resolve_provider(&self, tier: PrivacyTier) -> &str {
        match tier {
            PrivacyTier::Critical  => &self.config.critical_provider,
            PrivacyTier::Sensitive => &self.config.sensitive_provider,
            PrivacyTier::Internal  => &self.config.internal_provider,
            PrivacyTier::Public    => &self.config.public_provider,
        }
    }

    /// Classify messages and return the resolved provider name and tier.
    pub fn classify_and_route(&self, messages: &[ChatMessage]) -> (String, PrivacyTier) {
        let tier = self.classify(messages);
        let provider = self.resolve_provider(tier).to_string();
        (provider, tier)
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_guard() -> PrivacyGuard {
        PrivacyGuard::new(PrivacyConfig::default())
    }

    fn msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    // ── Critical pattern tests ──────────────────────────────────────────

    #[test]
    fn test_ssn_detected_as_critical() {
        let guard = default_guard();
        let msgs = [msg("user", "My SSN is 123-45-6789")];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Critical);
    }

    #[test]
    fn test_credit_card_detected_as_critical() {
        let guard = default_guard();
        let msgs = [msg("user", "Card number: 4111 1111 1111 1111")];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Critical);
    }

    #[test]
    fn test_api_key_detected_as_critical() {
        let guard = default_guard();
        let msgs = [msg("user", "my api_key = sk-abc123def456")];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Critical);
    }

    #[test]
    fn test_aws_key_detected_as_critical() {
        let guard = default_guard();
        let msgs = [msg("user", "AWS key: AKIAIOSFODNN7EXAMPLE")];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Critical);
    }

    #[test]
    fn test_password_detected_as_critical() {
        let guard = default_guard();
        let msgs = [msg("user", "password = hunter2")];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Critical);
    }

    #[test]
    fn test_pem_key_detected_as_critical() {
        let guard = default_guard();
        let msgs = [msg("user", "-----BEGIN RSA PRIVATE KEY-----\nMIIE...")];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Critical);
    }

    #[test]
    fn test_jwt_detected_as_critical() {
        let guard = default_guard();
        let token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let msgs = [msg("user", &format!("Token: {}", token))];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Critical);
    }

    #[test]
    fn test_secret_token_detected_as_critical() {
        let guard = default_guard();
        let msgs = [msg("user", "secret = 'abcdefghijklmnopqrstuvwxyz1234567890'")];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Critical);
    }

    // ── Sensitive pattern tests ─────────────────────────────────────────

    #[test]
    fn test_email_detected_as_sensitive() {
        let guard = default_guard();
        let msgs = [msg("user", "Contact me at john@example.com")];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Sensitive);
    }

    #[test]
    fn test_phone_detected_as_sensitive() {
        let guard = default_guard();
        let msgs = [msg("user", "Call me at +1-555-123-4567")];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Sensitive);
    }

    #[test]
    fn test_address_detected_as_sensitive() {
        let guard = default_guard();
        let msgs = [msg("user", "I live at 123 Main Street")];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Sensitive);
    }

    #[test]
    fn test_taiwan_id_detected_as_sensitive() {
        let guard = default_guard();
        let msgs = [msg("user", "My ID is A123456789")];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Sensitive);
    }

    #[test]
    fn test_ip_address_detected_as_sensitive() {
        let guard = default_guard();
        let msgs = [msg("user", "Server is at 10.0.1.2")];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Sensitive);
    }

    // ── Public / default tests ──────────────────────────────────────────

    #[test]
    fn test_public_message() {
        let guard = default_guard();
        let msgs = [msg("user", "What is the weather today?")];
        // Default tier is Internal, but text itself is Public.
        // Since default_tier is Internal and classify_text returns Public,
        // max(Internal, Public) = Internal
        assert_eq!(guard.classify(&msgs), PrivacyTier::Internal);
    }

    #[test]
    fn test_empty_messages() {
        let guard = default_guard();
        let msgs: Vec<ChatMessage> = vec![];
        // No messages → default tier
        assert_eq!(guard.classify(&msgs), PrivacyTier::Internal);
    }

    #[test]
    fn test_empty_content() {
        let guard = default_guard();
        let msgs = [msg("user", "")];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Internal);
    }

    #[test]
    fn test_public_tier_config() {
        let config = PrivacyConfig {
            default_tier: "public".to_string(),
            ..Default::default()
        };
        let guard = PrivacyGuard::new(config);
        let msgs = [msg("user", "Hello world")];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Public);
    }

    // ── Multi-message / max tier tests ──────────────────────────────────

    #[test]
    fn test_multi_message_takes_highest_tier() {
        let guard = default_guard();
        let msgs = [
            msg("user", "What is the weather?"),      // Public
            msg("user", "Contact john@example.com"),   // Sensitive
        ];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Sensitive);
    }

    #[test]
    fn test_critical_overrides_sensitive() {
        let guard = default_guard();
        let msgs = [
            msg("user", "Email: john@example.com"),           // Sensitive
            msg("user", "password = supersecret123"),          // Critical
        ];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Critical);
    }

    #[test]
    fn test_system_and_assistant_messages_skipped() {
        let guard = default_guard();
        let msgs = [
            msg("system", "password = admin123"),   // Should be skipped
            msg("assistant", "api_key = sk-test"),  // Should be skipped
            msg("user", "Hello world"),             // Public
        ];
        // Only user message scanned → Public, default is Internal → Internal
        assert_eq!(guard.classify(&msgs), PrivacyTier::Internal);
    }

    #[test]
    fn test_tool_message_scanned() {
        let guard = default_guard();
        let msgs = [
            msg("user", "Read my config file"),
            msg("tool", "password = hunter2"),  // Tool result with secret
        ];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Critical);
    }

    // ── Custom keyword tests ────────────────────────────────────────────

    #[test]
    fn test_custom_critical_keyword() {
        let config = PrivacyConfig {
            critical_keywords: vec!["PROJECT_ALPHA".to_string()],
            ..Default::default()
        };
        let guard = PrivacyGuard::new(config);
        let msgs = [msg("user", "Details about PROJECT_ALPHA launch")];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Critical);
    }

    #[test]
    fn test_custom_sensitive_keyword() {
        let config = PrivacyConfig {
            sensitive_keywords: vec!["salary".to_string()],
            ..Default::default()
        };
        let guard = PrivacyGuard::new(config);
        let msgs = [msg("user", "What is the average salary?")];
        assert_eq!(guard.classify(&msgs), PrivacyTier::Sensitive);
    }

    // ── Provider routing tests ──────────────────────────────────────────

    #[test]
    fn test_resolve_provider_critical() {
        let guard = default_guard();
        assert_eq!(guard.resolve_provider(PrivacyTier::Critical), "ollama");
    }

    #[test]
    fn test_resolve_provider_sensitive() {
        let guard = default_guard();
        assert_eq!(guard.resolve_provider(PrivacyTier::Sensitive), "ollama");
    }

    #[test]
    fn test_resolve_provider_internal() {
        let guard = default_guard();
        assert_eq!(guard.resolve_provider(PrivacyTier::Internal), "fireworks");
    }

    #[test]
    fn test_resolve_provider_public() {
        let guard = default_guard();
        assert_eq!(guard.resolve_provider(PrivacyTier::Public), "groq");
    }

    #[test]
    fn test_classify_and_route_returns_both() {
        let guard = default_guard();
        let msgs = [msg("user", "my api_key = sk-test123")];
        let (provider, tier) = guard.classify_and_route(&msgs);
        assert_eq!(tier, PrivacyTier::Critical);
        assert_eq!(provider, "ollama");
    }

    #[test]
    fn test_custom_provider_mapping() {
        let config = PrivacyConfig {
            critical_provider: "lmstudio".to_string(),
            public_provider: "anthropic".to_string(),
            ..Default::default()
        };
        let guard = PrivacyGuard::new(config);
        assert_eq!(guard.resolve_provider(PrivacyTier::Critical), "lmstudio");
        assert_eq!(guard.resolve_provider(PrivacyTier::Public), "anthropic");
    }

    // ── Config serde test ───────────────────────────────────────────────

    #[test]
    fn test_config_deserialize() {
        let toml_str = r#"
            enabled = true
            critical_provider = "ollama"
            sensitive_provider = "ollama"
            internal_provider = "fireworks"
            public_provider = "groq"
            default_tier = "internal"
            critical_keywords = ["TOP_SECRET"]
            sensitive_keywords = ["salary", "medical"]
        "#;
        let config: PrivacyConfig = toml::from_str(toml_str).unwrap();
        assert!(config.enabled);
        assert_eq!(config.critical_provider, "ollama");
        assert_eq!(config.internal_provider, "fireworks");
        assert_eq!(config.critical_keywords, vec!["TOP_SECRET"]);
        assert_eq!(config.sensitive_keywords, vec!["salary", "medical"]);
    }

    #[test]
    fn test_config_deserialize_minimal() {
        let toml_str = r#"
            enabled = true
        "#;
        let config: PrivacyConfig = toml::from_str(toml_str).unwrap();
        assert!(config.enabled);
        assert_eq!(config.critical_provider, "ollama");
        assert_eq!(config.public_provider, "groq");
        assert!(config.critical_keywords.is_empty());
    }

    // ── Tier ordering test ──────────────────────────────────────────────

    #[test]
    fn test_tier_ordering() {
        assert!(PrivacyTier::Critical > PrivacyTier::Sensitive);
        assert!(PrivacyTier::Sensitive > PrivacyTier::Internal);
        assert!(PrivacyTier::Internal > PrivacyTier::Public);
    }

    #[test]
    fn test_tier_display() {
        assert_eq!(format!("{}", PrivacyTier::Critical), "critical");
        assert_eq!(format!("{}", PrivacyTier::Public), "public");
    }
}
