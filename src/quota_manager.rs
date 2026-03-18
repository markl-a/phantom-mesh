/// Quota-aware preemptive context compaction manager.
///
/// `QuotaManager` tracks model token limits and provides headroom checks so
/// that callers can compact a conversation *before* submitting it to an LLM,
/// rather than discovering a context-length error at call time.
///
/// # Integration with `context.rs`
/// `QuotaManager` is intentionally decoupled from `ContextOptimizer`.
/// The typical usage pattern is:
///
/// ```text
/// let status = QuotaManager::new().check_headroom(model, current_tokens);
/// match status {
///     QuotaStatus::NeedsCompaction => {
///         let target = QuotaManager::new().recommended_target(model);
///         // call ContextOptimizer::trim_messages / apply_compaction to reach target
///     }
///     _ => {} // Ok or Warning — proceed
/// }
/// ```

use std::collections::HashMap;

/// Result of a headroom check.
#[derive(Debug, Clone, PartialEq)]
pub enum QuotaStatus {
    /// Usage is below 80% of the model limit — no action needed.
    Ok,
    /// Usage is between 80% and 90% — approaching the limit (`pct` is the
    /// current fill percentage as a value in 0..=100).
    Warning(u8),
    /// Usage is at or above 90% — compaction is strongly recommended before
    /// sending the next LLM call.
    NeedsCompaction,
}

/// Manager for per-model token quota limits and headroom checks.
///
/// Construct once and reuse (it is cheaply cloneable) or use the static
/// helper methods which build a default instance internally.
#[derive(Debug, Clone)]
pub struct QuotaManager {
    /// Map of model name → maximum context window in tokens.
    limits: HashMap<String, usize>,
}

impl QuotaManager {
    /// Build a `QuotaManager` pre-populated with well-known model limits.
    ///
    /// The defaults deliberately match common API docs / provider specs:
    ///
    /// | Model key    | Max tokens |
    /// |-------------|-----------|
    /// | gpt-4       | 128 000   |
    /// | gpt-3.5     | 16 000    |
    /// | gemini-pro  | 1 000 000 |
    /// | llama       | 8 000     |
    /// | claude      | 200 000   |
    /// | *(default)* | 8 000     |
    pub fn new() -> Self {
        let mut limits = HashMap::new();

        // OpenAI GPT family
        limits.insert("gpt-4".to_string(), 128_000);
        limits.insert("gpt-4o".to_string(), 128_000);
        limits.insert("gpt-4o-mini".to_string(), 128_000);
        limits.insert("gpt-4-turbo".to_string(), 128_000);
        limits.insert("gpt-3.5".to_string(), 16_000);
        limits.insert("gpt-3.5-turbo".to_string(), 16_000);

        // Anthropic Claude family
        limits.insert("claude".to_string(), 200_000);
        limits.insert("claude-opus-4-6".to_string(), 200_000);
        limits.insert("claude-sonnet-4-6".to_string(), 200_000);
        limits.insert("claude-haiku".to_string(), 200_000);

        // Google Gemini family
        limits.insert("gemini-pro".to_string(), 1_000_000);
        limits.insert("gemini".to_string(), 1_000_000);
        limits.insert("gemini-flash".to_string(), 1_000_000);

        // Meta LLaMA / local models
        limits.insert("llama".to_string(), 8_000);
        limits.insert("llama3".to_string(), 8_000);
        limits.insert("llama3.1".to_string(), 131_072);
        limits.insert("llama3.3".to_string(), 131_072);

        // Miscellaneous local
        limits.insert("mistral".to_string(), 32_768);
        limits.insert("mixtral".to_string(), 32_768);
        limits.insert("qwen3".to_string(), 32_768);
        limits.insert("deepseek".to_string(), 65_536);
        limits.insert("phi".to_string(), 16_384);

        Self { limits }
    }

    /// Look up the maximum token limit for `model`.
    ///
    /// Resolution order:
    /// 1. Exact match.
    /// 2. Any registered key that is a prefix of `model`
    ///    (e.g., `"gpt-4"` matches `"gpt-4-turbo-preview"`).
    /// 3. Any registered key contained in `model`
    ///    (e.g., `"claude"` matches `"claude-sonnet-4-6"`).
    /// 4. Default fallback: **8 000** tokens.
    pub fn max_tokens(&self, model: &str) -> usize {
        // 1. Exact match
        if let Some(&v) = self.limits.get(model) {
            return v;
        }
        // 2. Prefix: a registered key is a prefix of the requested model name
        for (key, &val) in &self.limits {
            if model.starts_with(key.as_str()) {
                return val;
            }
        }
        // 3. Substring: a registered key is contained anywhere in the name
        for (key, &val) in &self.limits {
            if model.contains(key.as_str()) {
                return val;
            }
        }
        // 4. Default
        8_000
    }

    /// Rough token estimation: one token ≈ four characters.
    ///
    /// This mirrors the heuristic used in `context.rs` so that callers get
    /// consistent numbers without duplicating the formula.
    pub fn estimate_tokens(text: &str) -> usize {
        (text.len() + 3) / 4
    }

    /// Check how much headroom remains for `model` given `current_tokens`.
    ///
    /// Thresholds (applied to the model's max window):
    /// - **< 80%** → `QuotaStatus::Ok`
    /// - **80% – 89%** → `QuotaStatus::Warning(pct)` where `pct` is 0–100
    /// - **≥ 90%** → `QuotaStatus::NeedsCompaction`
    pub fn check_headroom(&self, model: &str, current_tokens: usize) -> QuotaStatus {
        let max = self.max_tokens(model);
        if max == 0 {
            return QuotaStatus::NeedsCompaction;
        }

        // Compute fill percentage (0..=100, rounded down)
        let pct = (current_tokens * 100) / max;

        if pct >= 90 {
            QuotaStatus::NeedsCompaction
        } else if pct >= 80 {
            QuotaStatus::Warning(pct as u8)
        } else {
            QuotaStatus::Ok
        }
    }

    /// Recommended token target after compaction: **60% of the model's max**.
    ///
    /// Targeting 60% leaves a comfortable margin for the model's response
    /// tokens and upcoming turns.
    pub fn recommended_target(&self, model: &str) -> usize {
        let max = self.max_tokens(model);
        (max as f64 * 0.60) as usize
    }
}

impl Default for QuotaManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Token estimation ─────────────────────────────────────────────────────

    #[test]
    fn test_estimate_tokens_empty() {
        assert_eq!(QuotaManager::estimate_tokens(""), 0);
    }

    #[test]
    fn test_estimate_tokens_short() {
        // "hello" = 5 chars → (5+3)/4 = 2
        assert_eq!(QuotaManager::estimate_tokens("hello"), 2);
    }

    #[test]
    fn test_estimate_tokens_exact_multiple() {
        // 8 chars → (8+3)/4 = 2 (integer division)
        assert_eq!(QuotaManager::estimate_tokens("abcdefgh"), 2);
    }

    #[test]
    fn test_estimate_tokens_100_chars() {
        let text = "a".repeat(100);
        assert_eq!(QuotaManager::estimate_tokens(&text), 25);
    }

    #[test]
    fn test_estimate_tokens_400_chars() {
        let text = "x".repeat(400);
        assert_eq!(QuotaManager::estimate_tokens(&text), 100);
    }

    // ── max_tokens / model resolution ────────────────────────────────────────

    #[test]
    fn test_max_tokens_exact_gpt4() {
        let qm = QuotaManager::new();
        assert_eq!(qm.max_tokens("gpt-4"), 128_000);
    }

    #[test]
    fn test_max_tokens_exact_gpt35() {
        let qm = QuotaManager::new();
        assert_eq!(qm.max_tokens("gpt-3.5"), 16_000);
    }

    #[test]
    fn test_max_tokens_exact_gemini_pro() {
        let qm = QuotaManager::new();
        assert_eq!(qm.max_tokens("gemini-pro"), 1_000_000);
    }

    #[test]
    fn test_max_tokens_exact_llama() {
        let qm = QuotaManager::new();
        assert_eq!(qm.max_tokens("llama"), 8_000);
    }

    #[test]
    fn test_max_tokens_exact_claude() {
        let qm = QuotaManager::new();
        assert_eq!(qm.max_tokens("claude"), 200_000);
    }

    #[test]
    fn test_max_tokens_default_fallback() {
        let qm = QuotaManager::new();
        assert_eq!(qm.max_tokens("some-unknown-model-xyz"), 8_000);
    }

    #[test]
    fn test_max_tokens_prefix_match_gpt4_variant() {
        // "gpt-4o" is a registered key, but "gpt-4-preview" hits prefix "gpt-4"
        let qm = QuotaManager::new();
        // "gpt-4-turbo-preview" should match the registered "gpt-4" prefix
        assert_eq!(qm.max_tokens("gpt-4-turbo-preview"), 128_000);
    }

    #[test]
    fn test_max_tokens_substring_match_claude() {
        let qm = QuotaManager::new();
        // "claude-3-opus-20240229" contains "claude"
        assert_eq!(qm.max_tokens("claude-3-opus-20240229"), 200_000);
    }

    #[test]
    fn test_max_tokens_substring_match_gemini() {
        let qm = QuotaManager::new();
        assert_eq!(qm.max_tokens("gemini-1.5-pro"), 1_000_000);
    }

    // ── check_headroom ───────────────────────────────────────────────────────

    #[test]
    fn test_headroom_ok_well_below_limit() {
        let qm = QuotaManager::new();
        // gpt-4 max = 128_000; 10_000 / 128_000 ≈ 7% → Ok
        assert_eq!(qm.check_headroom("gpt-4", 10_000), QuotaStatus::Ok);
    }

    #[test]
    fn test_headroom_ok_at_79_percent() {
        let qm = QuotaManager::new();
        let max = qm.max_tokens("gpt-4");
        let tokens = (max * 79) / 100; // 79%
        assert_eq!(qm.check_headroom("gpt-4", tokens), QuotaStatus::Ok);
    }

    #[test]
    fn test_headroom_warning_at_80_percent() {
        let qm = QuotaManager::new();
        let max = qm.max_tokens("gpt-4"); // 128_000
        // Exactly 80%
        let tokens = (max * 80) / 100; // 102_400
        let status = qm.check_headroom("gpt-4", tokens);
        assert!(
            matches!(status, QuotaStatus::Warning(80)),
            "expected Warning(80), got {:?}",
            status
        );
    }

    #[test]
    fn test_headroom_warning_at_85_percent() {
        let qm = QuotaManager::new();
        let max = qm.max_tokens("gpt-4");
        let tokens = (max * 85) / 100;
        let status = qm.check_headroom("gpt-4", tokens);
        assert!(
            matches!(status, QuotaStatus::Warning(85)),
            "expected Warning(85), got {:?}",
            status
        );
    }

    #[test]
    fn test_headroom_needs_compaction_at_90_percent() {
        let qm = QuotaManager::new();
        let max = qm.max_tokens("gpt-4");
        let tokens = (max * 90) / 100;
        assert_eq!(
            qm.check_headroom("gpt-4", tokens),
            QuotaStatus::NeedsCompaction
        );
    }

    #[test]
    fn test_headroom_needs_compaction_at_95_percent() {
        let qm = QuotaManager::new();
        let max = qm.max_tokens("gpt-4");
        let tokens = (max * 95) / 100;
        assert_eq!(
            qm.check_headroom("gpt-4", tokens),
            QuotaStatus::NeedsCompaction
        );
    }

    #[test]
    fn test_headroom_needs_compaction_over_100_percent() {
        let qm = QuotaManager::new();
        // More tokens than the limit
        assert_eq!(
            qm.check_headroom("gpt-4", 200_000),
            QuotaStatus::NeedsCompaction
        );
    }

    #[test]
    fn test_headroom_llama_small_window() {
        let qm = QuotaManager::new();
        // llama max = 8_000; 7_500 / 8_000 = 93% → NeedsCompaction
        assert_eq!(
            qm.check_headroom("llama", 7_500),
            QuotaStatus::NeedsCompaction
        );
    }

    #[test]
    fn test_headroom_llama_warning_zone() {
        let qm = QuotaManager::new();
        // llama max = 8_000; 6_500 / 8_000 = 81% → Warning
        let status = qm.check_headroom("llama", 6_500);
        assert!(
            matches!(status, QuotaStatus::Warning(_)),
            "expected Warning, got {:?}",
            status
        );
    }

    #[test]
    fn test_headroom_gemini_pro_large_window() {
        let qm = QuotaManager::new();
        // gemini-pro max = 1_000_000; even 500_000 tokens is just 50% → Ok
        assert_eq!(
            qm.check_headroom("gemini-pro", 500_000),
            QuotaStatus::Ok
        );
    }

    #[test]
    fn test_headroom_claude_ok() {
        let qm = QuotaManager::new();
        // claude max = 200_000; 100_000 = 50% → Ok
        assert_eq!(qm.check_headroom("claude", 100_000), QuotaStatus::Ok);
    }

    #[test]
    fn test_headroom_default_fallback_model() {
        let qm = QuotaManager::new();
        // Unknown model → default 8_000; 7_500 / 8_000 = 93% → NeedsCompaction
        assert_eq!(
            qm.check_headroom("unknown-model-abc", 7_500),
            QuotaStatus::NeedsCompaction
        );
    }

    #[test]
    fn test_headroom_zero_tokens() {
        let qm = QuotaManager::new();
        assert_eq!(qm.check_headroom("gpt-4", 0), QuotaStatus::Ok);
    }

    // ── recommended_target ───────────────────────────────────────────────────

    #[test]
    fn test_recommended_target_gpt4() {
        let qm = QuotaManager::new();
        // 60% of 128_000 = 76_800
        assert_eq!(qm.recommended_target("gpt-4"), 76_800);
    }

    #[test]
    fn test_recommended_target_gpt35() {
        let qm = QuotaManager::new();
        // 60% of 16_000 = 9_600
        assert_eq!(qm.recommended_target("gpt-3.5"), 9_600);
    }

    #[test]
    fn test_recommended_target_claude() {
        let qm = QuotaManager::new();
        // 60% of 200_000 = 120_000
        assert_eq!(qm.recommended_target("claude"), 120_000);
    }

    #[test]
    fn test_recommended_target_gemini_pro() {
        let qm = QuotaManager::new();
        // 60% of 1_000_000 = 600_000
        assert_eq!(qm.recommended_target("gemini-pro"), 600_000);
    }

    #[test]
    fn test_recommended_target_llama() {
        let qm = QuotaManager::new();
        // 60% of 8_000 = 4_800
        assert_eq!(qm.recommended_target("llama"), 4_800);
    }

    #[test]
    fn test_recommended_target_default() {
        let qm = QuotaManager::new();
        // Unknown model → 60% of 8_000 = 4_800
        assert_eq!(qm.recommended_target("mystery-model"), 4_800);
    }

    // ── Default impl ─────────────────────────────────────────────────────────

    #[test]
    fn test_default_is_equivalent_to_new() {
        let qm1 = QuotaManager::new();
        let qm2 = QuotaManager::default();
        assert_eq!(qm1.max_tokens("gpt-4"), qm2.max_tokens("gpt-4"));
        assert_eq!(qm1.max_tokens("claude"), qm2.max_tokens("claude"));
        assert_eq!(qm1.max_tokens("unknown"), qm2.max_tokens("unknown"));
    }

    // ── End-to-end integration scenario ──────────────────────────────────────

    #[test]
    fn test_e2e_quota_workflow() {
        let qm = QuotaManager::new();
        let model = "gpt-4";

        // A long prompt that is ~92% of gpt-4's limit
        let max = qm.max_tokens(model); // 128_000
        let heavy_tokens = (max * 92) / 100;

        // Should require compaction
        assert_eq!(
            qm.check_headroom(model, heavy_tokens),
            QuotaStatus::NeedsCompaction
        );

        // After compaction we aim for the recommended target
        let target = qm.recommended_target(model); // 76_800
        // The target should be well below the warning threshold
        assert_eq!(qm.check_headroom(model, target), QuotaStatus::Ok);
    }
}
