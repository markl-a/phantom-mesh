//! Budget Downgrade — L1/L2/L3 automatic model downgrade when cost budget approaches limits.
//!
//! When an agent's spending approaches its daily budget, this system automatically
//! downgrades to cheaper models to preserve budget:
//!
//! - **L1** (default 80% budget): Switch from expensive model to a cheaper cloud model
//! - **L2** (default 90% budget): Switch to a local model (ollama/lmstudio)
//! - **L3** (default 95% budget): Halt execution and notify via Telegram
//!
//! Integrates with `CostTracker` and `BudgetBreaker` from `cost_tracker.rs`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use tracing::{debug, info, warn};

/// Model cost tier classification
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelTier {
    /// Expensive cloud models (gpt-4, claude-3-opus, o1, gpt-4o)
    Expensive,
    /// Medium-cost cloud models (gemini-flash, groq, claude-haiku, gpt-3.5)
    Medium,
    /// Free/local models (ollama, lmstudio)
    Cheap,
}

/// Action to take based on budget usage level
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DowngradeAction {
    /// Budget is fine, no action needed
    NoAction,
    /// L1: Switch to a cheaper cloud model
    SwitchModel {
        from_model: String,
        to_model: String,
        to_provider: String,
        reason: String,
    },
    /// L2: Switch to a local model
    SwitchToLocal {
        from_model: String,
        to_provider: String,
        to_model: String,
        reason: String,
    },
    /// L3: Halt execution entirely
    HaltExecution {
        reason: String,
        spent: f64,
        budget: f64,
    },
}

/// Configurable thresholds for budget downgrade tiers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DowngradeThresholds {
    /// L1 threshold: percentage of budget to trigger model switch (default 0.80)
    pub l1_pct: f64,
    /// L2 threshold: percentage of budget to trigger local switch (default 0.90)
    pub l2_pct: f64,
    /// L3 threshold: percentage of budget to trigger halt (default 0.95)
    pub l3_pct: f64,
}

impl Default for DowngradeThresholds {
    fn default() -> Self {
        Self {
            l1_pct: 0.80,
            l2_pct: 0.90,
            l3_pct: 0.95,
        }
    }
}

impl DowngradeThresholds {
    /// Create custom thresholds with validation
    pub fn new(l1: f64, l2: f64, l3: f64) -> Result<Self, String> {
        if l1 <= 0.0 || l1 >= 1.0 {
            return Err(format!("L1 threshold must be between 0 and 1, got {}", l1));
        }
        if l2 <= 0.0 || l2 >= 1.0 {
            return Err(format!("L2 threshold must be between 0 and 1, got {}", l2));
        }
        if l3 <= 0.0 || l3 >= 1.0 {
            return Err(format!("L3 threshold must be between 0 and 1, got {}", l3));
        }
        if l1 >= l2 {
            return Err(format!("L1 ({}) must be less than L2 ({})", l1, l2));
        }
        if l2 >= l3 {
            return Err(format!("L2 ({}) must be less than L3 ({})", l2, l3));
        }
        Ok(Self {
            l1_pct: l1,
            l2_pct: l2,
            l3_pct: l3,
        })
    }
}

/// Model mapping entry: what to downgrade to for each tier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMapping {
    pub provider: String,
    pub model: String,
}

/// BudgetDowngrader — checks agent cost vs budget and recommends model downgrades.
///
/// Stateless per-check (reads current cost externally), but tracks which agents
/// have been downgraded to avoid log spam.
pub struct BudgetDowngrader {
    thresholds: DowngradeThresholds,
    /// Model tier classification: model_name_pattern -> ModelTier
    model_tiers: HashMap<String, ModelTier>,
    /// Medium-tier fallback models (L1 downgrade targets)
    medium_models: Vec<ModelMapping>,
    /// Cheap/local fallback models (L2 downgrade targets)
    local_models: Vec<ModelMapping>,
    /// Tracks current downgrade level per agent to avoid repeated log noise
    active_downgrades: Mutex<HashMap<String, u8>>,
}

impl BudgetDowngrader {
    /// Create a new BudgetDowngrader with default model tiers and thresholds
    pub fn new() -> Self {
        Self::with_thresholds(DowngradeThresholds::default())
    }

    /// Create with custom thresholds
    pub fn with_thresholds(thresholds: DowngradeThresholds) -> Self {
        let mut model_tiers = HashMap::new();

        // Expensive models
        for pattern in &[
            "gpt-4", "gpt-4o", "gpt-4-turbo", "gpt-5",
            "claude-3-opus", "claude-opus", "claude-sonnet",
            "claude-3-sonnet", "claude-3.5-sonnet", "claude-4",
            "o1", "o1-preview", "o1-mini", "o3",
        ] {
            model_tiers.insert(pattern.to_string(), ModelTier::Expensive);
        }

        // Medium models
        for pattern in &[
            "gemini-flash", "gemini-2.5-flash", "gemini-2.0-flash",
            "gemini-pro", "gemini-1.5-pro",
            "groq", "llama", "mixtral", "mistral",
            "claude-haiku", "claude-3-haiku",
            "gpt-3.5", "gpt-3.5-turbo",
            "deepseek", "qwen",
        ] {
            model_tiers.insert(pattern.to_string(), ModelTier::Medium);
        }

        // Cheap/local models
        for pattern in &[
            "ollama", "lmstudio", "lemonade", "npu",
            "phi", "tinyllama", "llama3.2:1b", "llama3.2:3b",
        ] {
            model_tiers.insert(pattern.to_string(), ModelTier::Cheap);
        }

        let medium_models = vec![
            ModelMapping { provider: "gemini".to_string(), model: "gemini-2.5-flash-lite".to_string() },
            ModelMapping { provider: "groq".to_string(), model: "llama-3.3-70b-versatile".to_string() },
            ModelMapping { provider: "cerebras".to_string(), model: "llama-3.3-70b".to_string() },
        ];

        let local_models = vec![
            ModelMapping { provider: "ollama".to_string(), model: "llama3.2:1b".to_string() },
            ModelMapping { provider: "lmstudio".to_string(), model: "default".to_string() },
        ];

        Self {
            thresholds,
            model_tiers,
            medium_models,
            local_models,
            active_downgrades: Mutex::new(HashMap::new()),
        }
    }

    /// Classify a model string into a cost tier by checking against known patterns
    pub fn classify_model(&self, model: &str) -> ModelTier {
        let lower = model.to_lowercase();
        // Check for exact matches first, then substring matches
        // Sort by longest pattern first so "gpt-4o" matches before "gpt-4"
        let mut patterns: Vec<_> = self.model_tiers.iter().collect();
        patterns.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

        for (pattern, tier) in patterns {
            if lower.contains(&pattern.to_lowercase()) {
                return tier.clone();
            }
        }
        // Unknown models default to Medium (conservative)
        ModelTier::Medium
    }

    /// Core check: given current cost and budget, determine what action to take.
    ///
    /// Returns `DowngradeAction` indicating whether to switch models or halt.
    pub fn check_and_downgrade(
        &self,
        agent: &str,
        current_cost: f64,
        budget: f64,
        current_model: &str,
    ) -> DowngradeAction {
        // No budget = no downgrade
        if budget <= 0.0 {
            return DowngradeAction::NoAction;
        }

        let usage_pct = current_cost / budget;

        // L3: >= 95% — halt execution
        if usage_pct >= self.thresholds.l3_pct {
            let level = 3u8;
            self.update_downgrade_level(agent, level);
            warn!(
                "L3 budget halt for agent '{}': ${:.4} / ${:.2} ({:.1}%)",
                agent, current_cost, budget, usage_pct * 100.0
            );
            return DowngradeAction::HaltExecution {
                reason: format!(
                    "L3 budget halt: agent '{}' has used {:.1}% of ${:.2} budget (${:.4} spent). Execution suspended.",
                    agent, usage_pct * 100.0, budget, current_cost
                ),
                spent: current_cost,
                budget,
            };
        }

        // L2: >= 90% — switch to local model
        if usage_pct >= self.thresholds.l2_pct {
            let current_tier = self.classify_model(current_model);

            // Already on a cheap/local model? No action needed.
            if current_tier == ModelTier::Cheap {
                debug!(
                    "Agent '{}' already on cheap model '{}' at L2 ({:.1}%)",
                    agent, current_model, usage_pct * 100.0
                );
                return DowngradeAction::NoAction;
            }

            let level = 2u8;
            self.update_downgrade_level(agent, level);

            if let Some(local) = self.local_models.first() {
                info!(
                    "L2 budget downgrade for agent '{}': {} -> {}:{} ({:.1}% of budget)",
                    agent, current_model, local.provider, local.model, usage_pct * 100.0
                );
                return DowngradeAction::SwitchToLocal {
                    from_model: current_model.to_string(),
                    to_provider: local.provider.clone(),
                    to_model: local.model.clone(),
                    reason: format!(
                        "L2 budget downgrade: {:.1}% of ${:.2} budget used. Switching to local model.",
                        usage_pct * 100.0, budget
                    ),
                };
            }

            // No local models configured — halt as fallback
            return DowngradeAction::HaltExecution {
                reason: format!(
                    "L2 budget threshold reached ({:.1}%) but no local models configured.",
                    usage_pct * 100.0
                ),
                spent: current_cost,
                budget,
            };
        }

        // L1: >= 80% — switch to cheaper cloud model
        if usage_pct >= self.thresholds.l1_pct {
            let current_tier = self.classify_model(current_model);

            // Already on medium or cheap? No further downgrade at L1.
            if current_tier == ModelTier::Medium || current_tier == ModelTier::Cheap {
                debug!(
                    "Agent '{}' already on {}-tier model '{}' at L1 ({:.1}%)",
                    agent, if current_tier == ModelTier::Medium { "medium" } else { "cheap" },
                    current_model, usage_pct * 100.0
                );
                return DowngradeAction::NoAction;
            }

            let level = 1u8;
            self.update_downgrade_level(agent, level);

            if let Some(medium) = self.medium_models.first() {
                info!(
                    "L1 budget downgrade for agent '{}': {} -> {}:{} ({:.1}% of budget)",
                    agent, current_model, medium.provider, medium.model, usage_pct * 100.0
                );
                return DowngradeAction::SwitchModel {
                    from_model: current_model.to_string(),
                    to_model: medium.model.clone(),
                    to_provider: medium.provider.clone(),
                    reason: format!(
                        "L1 budget downgrade: {:.1}% of ${:.2} budget used. Switching to cheaper model.",
                        usage_pct * 100.0, budget
                    ),
                };
            }

            // No medium models available — try local
            if let Some(local) = self.local_models.first() {
                return DowngradeAction::SwitchToLocal {
                    from_model: current_model.to_string(),
                    to_provider: local.provider.clone(),
                    to_model: local.model.clone(),
                    reason: format!(
                        "L1 budget downgrade: no medium models available, switching to local.",
                    ),
                };
            }
        }

        // Below all thresholds — no action
        DowngradeAction::NoAction
    }

    /// Update the tracked downgrade level for an agent
    fn update_downgrade_level(&self, agent: &str, level: u8) {
        let mut map = self.active_downgrades.lock().unwrap();
        let prev = map.get(agent).copied().unwrap_or(0);
        if level != prev {
            debug!("Agent '{}' downgrade level changed: L{} -> L{}", agent, prev, level);
        }
        map.insert(agent.to_string(), level);
    }

    /// Get the current downgrade level for an agent (0 = none, 1-3 = L1-L3)
    pub fn current_level(&self, agent: &str) -> u8 {
        let map = self.active_downgrades.lock().unwrap();
        map.get(agent).copied().unwrap_or(0)
    }

    /// Reset downgrade tracking for an agent (e.g., new day / budget reset)
    pub fn reset_agent(&self, agent: &str) {
        let mut map = self.active_downgrades.lock().unwrap();
        if map.remove(agent).is_some() {
            info!("Budget downgrade reset for agent '{}'", agent);
        }
    }

    /// Reset all agent downgrade tracking
    pub fn reset_all(&self) {
        let mut map = self.active_downgrades.lock().unwrap();
        let count = map.len();
        map.clear();
        if count > 0 {
            info!("Budget downgrade reset for {} agents", count);
        }
    }

    /// Get list of agents with active downgrades
    pub fn downgraded_agents(&self) -> Vec<(String, u8)> {
        let map = self.active_downgrades.lock().unwrap();
        map.iter().map(|(k, v)| (k.clone(), *v)).collect()
    }

    /// Add a custom model tier classification
    pub fn add_model_tier(&mut self, pattern: &str, tier: ModelTier) {
        self.model_tiers.insert(pattern.to_string(), tier);
    }

    /// Set custom medium-tier fallback models
    pub fn set_medium_models(&mut self, models: Vec<ModelMapping>) {
        self.medium_models = models;
    }

    /// Set custom local/cheap fallback models
    pub fn set_local_models(&mut self, models: Vec<ModelMapping>) {
        self.local_models = models;
    }

    /// Get the configured thresholds
    pub fn thresholds(&self) -> &DowngradeThresholds {
        &self.thresholds
    }

    /// Get the medium-tier fallback models
    pub fn medium_models(&self) -> &[ModelMapping] {
        &self.medium_models
    }

    /// Get the local fallback models
    pub fn local_models(&self) -> &[ModelMapping] {
        &self.local_models
    }

    /// Compute usage percentage
    pub fn usage_pct(current_cost: f64, budget: f64) -> f64 {
        if budget <= 0.0 {
            return 0.0;
        }
        (current_cost / budget) * 100.0
    }
}

impl Default for BudgetDowngrader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_downgrader() -> BudgetDowngrader {
        BudgetDowngrader::new()
    }

    // ===== Model Tier Classification =====

    #[test]
    fn test_classify_expensive_models() {
        let d = make_downgrader();
        assert_eq!(d.classify_model("gpt-4"), ModelTier::Expensive);
        assert_eq!(d.classify_model("gpt-4o"), ModelTier::Expensive);
        assert_eq!(d.classify_model("gpt-4-turbo"), ModelTier::Expensive);
        assert_eq!(d.classify_model("claude-3-opus"), ModelTier::Expensive);
        assert_eq!(d.classify_model("claude-sonnet-4"), ModelTier::Expensive);
        assert_eq!(d.classify_model("o1-preview"), ModelTier::Expensive);
    }

    #[test]
    fn test_classify_medium_models() {
        let d = make_downgrader();
        assert_eq!(d.classify_model("gemini-flash"), ModelTier::Medium);
        assert_eq!(d.classify_model("gemini-2.5-flash-lite"), ModelTier::Medium);
        assert_eq!(d.classify_model("llama-3.3-70b-versatile"), ModelTier::Medium);
        assert_eq!(d.classify_model("mixtral-8x7b"), ModelTier::Medium);
        assert_eq!(d.classify_model("gpt-3.5-turbo"), ModelTier::Medium);
        assert_eq!(d.classify_model("claude-3-haiku"), ModelTier::Medium);
        assert_eq!(d.classify_model("deepseek-coder"), ModelTier::Medium);
        assert_eq!(d.classify_model("qwen3-coder-next"), ModelTier::Medium);
    }

    #[test]
    fn test_classify_cheap_models() {
        let d = make_downgrader();
        assert_eq!(d.classify_model("ollama/qwen:8b"), ModelTier::Cheap);
        assert_eq!(d.classify_model("lmstudio-default"), ModelTier::Cheap);
        assert_eq!(d.classify_model("phi-3-mini"), ModelTier::Cheap);
        assert_eq!(d.classify_model("tinyllama-1.1b"), ModelTier::Cheap);
    }

    #[test]
    fn test_classify_unknown_model_defaults_to_medium() {
        let d = make_downgrader();
        assert_eq!(d.classify_model("some-unknown-model-v99"), ModelTier::Medium);
    }

    #[test]
    fn test_classify_case_insensitive() {
        let d = make_downgrader();
        assert_eq!(d.classify_model("GPT-4"), ModelTier::Expensive);
        assert_eq!(d.classify_model("Gemini-Flash"), ModelTier::Medium);
        assert_eq!(d.classify_model("OLLAMA/model"), ModelTier::Cheap);
    }

    // ===== L1 Downgrade (80% budget) =====

    #[test]
    fn test_l1_downgrade_expensive_to_medium() {
        let d = make_downgrader();
        let action = d.check_and_downgrade("master", 8.5, 10.0, "gpt-4");
        match action {
            DowngradeAction::SwitchModel { from_model, to_model, to_provider, .. } => {
                assert_eq!(from_model, "gpt-4");
                assert_eq!(to_provider, "gemini");
                assert!(to_model.contains("gemini"));
            }
            _ => panic!("Expected SwitchModel at L1, got {:?}", action),
        }
    }

    #[test]
    fn test_l1_no_action_if_already_medium() {
        let d = make_downgrader();
        // At 85% budget with a medium model — should not downgrade further at L1
        let action = d.check_and_downgrade("master", 8.5, 10.0, "gemini-flash");
        assert_eq!(action, DowngradeAction::NoAction);
    }

    #[test]
    fn test_l1_no_action_if_already_cheap() {
        let d = make_downgrader();
        // At 85% budget with a cheap model — no downgrade needed
        let action = d.check_and_downgrade("master", 8.5, 10.0, "ollama/qwen:8b");
        assert_eq!(action, DowngradeAction::NoAction);
    }

    // ===== L2 Downgrade (90% budget) =====

    #[test]
    fn test_l2_downgrade_expensive_to_local() {
        let d = make_downgrader();
        let action = d.check_and_downgrade("master", 9.2, 10.0, "gpt-4");
        match action {
            DowngradeAction::SwitchToLocal { from_model, to_provider, to_model, .. } => {
                assert_eq!(from_model, "gpt-4");
                assert_eq!(to_provider, "ollama");
                assert!(to_model.contains("llama"));
            }
            _ => panic!("Expected SwitchToLocal at L2, got {:?}", action),
        }
    }

    #[test]
    fn test_l2_downgrade_medium_to_local() {
        let d = make_downgrader();
        // Medium model at 92% budget should downgrade to local
        let action = d.check_and_downgrade("master", 9.2, 10.0, "gemini-flash");
        match action {
            DowngradeAction::SwitchToLocal { from_model, to_provider, .. } => {
                assert_eq!(from_model, "gemini-flash");
                assert_eq!(to_provider, "ollama");
            }
            _ => panic!("Expected SwitchToLocal at L2 for medium model, got {:?}", action),
        }
    }

    #[test]
    fn test_l2_no_action_if_already_local() {
        let d = make_downgrader();
        // Already on a cheap model at 92% — no further downgrade at L2
        let action = d.check_and_downgrade("master", 9.2, 10.0, "ollama/llama3:8b");
        assert_eq!(action, DowngradeAction::NoAction);
    }

    // ===== L3 Halt (95% budget) =====

    #[test]
    fn test_l3_halt_execution() {
        let d = make_downgrader();
        let action = d.check_and_downgrade("master", 9.6, 10.0, "gpt-4");
        match action {
            DowngradeAction::HaltExecution { spent, budget, reason } => {
                assert!((spent - 9.6).abs() < 0.01);
                assert!((budget - 10.0).abs() < 0.01);
                assert!(reason.contains("L3"));
                assert!(reason.contains("96.0%"));
            }
            _ => panic!("Expected HaltExecution at L3, got {:?}", action),
        }
    }

    #[test]
    fn test_l3_halt_even_on_cheap_model() {
        let d = make_downgrader();
        // Even local models get halted at L3
        let action = d.check_and_downgrade("master", 9.8, 10.0, "ollama/qwen:8b");
        match &action {
            DowngradeAction::HaltExecution { reason, .. } => {
                assert!(reason.contains("L3"));
            }
            _ => panic!("Expected HaltExecution at L3 even for cheap model, got {:?}", action),
        }
    }

    #[test]
    fn test_l3_halt_at_100_percent() {
        let d = make_downgrader();
        let action = d.check_and_downgrade("master", 10.0, 10.0, "gpt-4");
        match &action {
            DowngradeAction::HaltExecution { spent, budget, .. } => {
                assert!((spent - 10.0).abs() < 0.01);
                assert!((budget - 10.0).abs() < 0.01);
            }
            _ => panic!("Expected HaltExecution at 100%, got {:?}", action),
        }
    }

    #[test]
    fn test_l3_halt_over_budget() {
        let d = make_downgrader();
        // Even over 100% should halt
        let action = d.check_and_downgrade("master", 15.0, 10.0, "gpt-4");
        match &action {
            DowngradeAction::HaltExecution { .. } => {}
            _ => panic!("Expected HaltExecution over budget, got {:?}", action),
        }
    }

    // ===== No Action (below thresholds) =====

    #[test]
    fn test_no_action_below_l1() {
        let d = make_downgrader();
        let action = d.check_and_downgrade("master", 5.0, 10.0, "gpt-4");
        assert_eq!(action, DowngradeAction::NoAction);
    }

    #[test]
    fn test_no_action_zero_cost() {
        let d = make_downgrader();
        let action = d.check_and_downgrade("master", 0.0, 10.0, "gpt-4");
        assert_eq!(action, DowngradeAction::NoAction);
    }

    #[test]
    fn test_no_action_no_budget_set() {
        let d = make_downgrader();
        // Budget of 0.0 means unlimited — never downgrade
        let action = d.check_and_downgrade("master", 999.0, 0.0, "gpt-4");
        assert_eq!(action, DowngradeAction::NoAction);
    }

    #[test]
    fn test_no_action_negative_budget() {
        let d = make_downgrader();
        let action = d.check_and_downgrade("master", 5.0, -1.0, "gpt-4");
        assert_eq!(action, DowngradeAction::NoAction);
    }

    // ===== Custom Thresholds =====

    #[test]
    fn test_custom_thresholds() {
        let thresholds = DowngradeThresholds::new(0.50, 0.70, 0.85).unwrap();
        let d = BudgetDowngrader::with_thresholds(thresholds);

        // At 55% with custom L1=50% threshold — should trigger L1
        let action = d.check_and_downgrade("master", 5.5, 10.0, "gpt-4");
        match &action {
            DowngradeAction::SwitchModel { .. } => {}
            _ => panic!("Expected SwitchModel at custom L1 50%, got {:?}", action),
        }

        // At 75% with custom L2=70% — should trigger L2
        let action = d.check_and_downgrade("master", 7.5, 10.0, "gpt-4");
        match &action {
            DowngradeAction::SwitchToLocal { .. } => {}
            _ => panic!("Expected SwitchToLocal at custom L2 70%, got {:?}", action),
        }

        // At 90% with custom L3=85% — should trigger L3
        let action = d.check_and_downgrade("master", 9.0, 10.0, "gpt-4");
        match &action {
            DowngradeAction::HaltExecution { .. } => {}
            _ => panic!("Expected HaltExecution at custom L3 85%, got {:?}", action),
        }
    }

    #[test]
    fn test_invalid_thresholds_ordering() {
        // L1 > L2 should fail
        assert!(DowngradeThresholds::new(0.90, 0.80, 0.95).is_err());
        // L2 > L3 should fail
        assert!(DowngradeThresholds::new(0.50, 0.95, 0.90).is_err());
        // Out of range
        assert!(DowngradeThresholds::new(0.0, 0.80, 0.95).is_err());
        assert!(DowngradeThresholds::new(0.50, 1.0, 0.95).is_err());
    }

    #[test]
    fn test_valid_thresholds() {
        assert!(DowngradeThresholds::new(0.60, 0.80, 0.95).is_ok());
        assert!(DowngradeThresholds::new(0.01, 0.02, 0.99).is_ok());
    }

    // ===== Agent Tracking =====

    #[test]
    fn test_current_level_default_zero() {
        let d = make_downgrader();
        assert_eq!(d.current_level("master"), 0);
    }

    #[test]
    fn test_current_level_tracks_downgrades() {
        let d = make_downgrader();
        // Trigger L1
        d.check_and_downgrade("master", 8.5, 10.0, "gpt-4");
        assert_eq!(d.current_level("master"), 1);
        // Trigger L2
        d.check_and_downgrade("master", 9.2, 10.0, "gpt-4");
        assert_eq!(d.current_level("master"), 2);
        // Trigger L3
        d.check_and_downgrade("master", 9.6, 10.0, "gpt-4");
        assert_eq!(d.current_level("master"), 3);
    }

    #[test]
    fn test_reset_agent() {
        let d = make_downgrader();
        d.check_and_downgrade("master", 9.6, 10.0, "gpt-4"); // L3
        assert_eq!(d.current_level("master"), 3);
        d.reset_agent("master");
        assert_eq!(d.current_level("master"), 0);
    }

    #[test]
    fn test_reset_all() {
        let d = make_downgrader();
        d.check_and_downgrade("master", 9.6, 10.0, "gpt-4");
        d.check_and_downgrade("coder", 8.5, 10.0, "gpt-4");
        assert_eq!(d.downgraded_agents().len(), 2);
        d.reset_all();
        assert_eq!(d.downgraded_agents().len(), 0);
    }

    #[test]
    fn test_downgraded_agents_list() {
        let d = make_downgrader();
        d.check_and_downgrade("master", 8.5, 10.0, "gpt-4");
        d.check_and_downgrade("coder", 9.6, 10.0, "claude-3-opus");
        let agents = d.downgraded_agents();
        assert_eq!(agents.len(), 2);
        let master = agents.iter().find(|(name, _)| name == "master");
        let coder = agents.iter().find(|(name, _)| name == "coder");
        assert_eq!(master.unwrap().1, 1);
        assert_eq!(coder.unwrap().1, 3);
    }

    // ===== Edge Cases =====

    #[test]
    fn test_exact_l1_boundary() {
        let d = make_downgrader();
        // Exactly 80% — should trigger L1
        let action = d.check_and_downgrade("master", 8.0, 10.0, "gpt-4");
        match &action {
            DowngradeAction::SwitchModel { .. } => {}
            _ => panic!("Expected SwitchModel at exactly 80%, got {:?}", action),
        }
    }

    #[test]
    fn test_just_below_l1_boundary() {
        let d = make_downgrader();
        // 79.99% — should NOT trigger
        let action = d.check_and_downgrade("master", 7.999, 10.0, "gpt-4");
        assert_eq!(action, DowngradeAction::NoAction);
    }

    #[test]
    fn test_exact_l2_boundary() {
        let d = make_downgrader();
        // Exactly 90% — should trigger L2
        let action = d.check_and_downgrade("master", 9.0, 10.0, "gpt-4");
        match &action {
            DowngradeAction::SwitchToLocal { .. } => {}
            _ => panic!("Expected SwitchToLocal at exactly 90%, got {:?}", action),
        }
    }

    #[test]
    fn test_exact_l3_boundary() {
        let d = make_downgrader();
        // Exactly 95% — should trigger L3
        let action = d.check_and_downgrade("master", 9.5, 10.0, "gpt-4");
        match &action {
            DowngradeAction::HaltExecution { .. } => {}
            _ => panic!("Expected HaltExecution at exactly 95%, got {:?}", action),
        }
    }

    #[test]
    fn test_very_small_budget() {
        let d = make_downgrader();
        // $0.01 budget, $0.009 spent (90% — hits L1 first since 90% >= 80%)
        let action = d.check_and_downgrade("micro", 0.009, 0.01, "gpt-4");
        // L1 triggers SwitchModel to medium; L2 would need already-medium model
        match &action {
            DowngradeAction::SwitchModel { .. } | DowngradeAction::SwitchToLocal { .. } => {}
            _ => panic!("Expected downgrade for micro budget, got {:?}", action),
        }
    }

    #[test]
    fn test_large_budget() {
        let d = make_downgrader();
        // $1000 budget, $500 spent (50% — no action)
        let action = d.check_and_downgrade("enterprise", 500.0, 1000.0, "gpt-4");
        assert_eq!(action, DowngradeAction::NoAction);
    }

    // ===== Custom Model Config =====

    #[test]
    fn test_custom_model_tier() {
        let mut d = make_downgrader();
        d.add_model_tier("my-custom-expensive", ModelTier::Expensive);
        assert_eq!(d.classify_model("my-custom-expensive-v2"), ModelTier::Expensive);
    }

    #[test]
    fn test_custom_medium_models() {
        let mut d = make_downgrader();
        d.set_medium_models(vec![
            ModelMapping { provider: "openrouter".to_string(), model: "llama-free".to_string() },
        ]);
        let action = d.check_and_downgrade("master", 8.5, 10.0, "gpt-4");
        match &action {
            DowngradeAction::SwitchModel { to_provider, to_model, .. } => {
                assert_eq!(to_provider, "openrouter");
                assert_eq!(to_model, "llama-free");
            }
            _ => panic!("Expected SwitchModel with custom medium, got {:?}", action),
        }
    }

    #[test]
    fn test_l2_with_no_local_models_halts() {
        let mut d = make_downgrader();
        d.set_local_models(vec![]); // No local models available
        let action = d.check_and_downgrade("master", 9.2, 10.0, "gpt-4");
        match &action {
            DowngradeAction::HaltExecution { reason, .. } => {
                assert!(reason.contains("no local models"));
            }
            _ => panic!("Expected HaltExecution with no local models, got {:?}", action),
        }
    }

    // ===== Utility =====

    #[test]
    fn test_usage_pct() {
        assert!((BudgetDowngrader::usage_pct(8.0, 10.0) - 80.0).abs() < 0.01);
        assert!((BudgetDowngrader::usage_pct(0.0, 10.0) - 0.0).abs() < 0.01);
        assert!((BudgetDowngrader::usage_pct(10.0, 10.0) - 100.0).abs() < 0.01);
        assert!((BudgetDowngrader::usage_pct(5.0, 0.0) - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_default_thresholds() {
        let d = make_downgrader();
        let t = d.thresholds();
        assert!((t.l1_pct - 0.80).abs() < 0.001);
        assert!((t.l2_pct - 0.90).abs() < 0.001);
        assert!((t.l3_pct - 0.95).abs() < 0.001);
    }

    #[test]
    fn test_multi_agent_independent_tracking() {
        let d = make_downgrader();
        // Master at L1, coder at L3 — independent
        d.check_and_downgrade("master", 8.5, 10.0, "gpt-4");
        d.check_and_downgrade("coder", 9.6, 10.0, "gpt-4");

        assert_eq!(d.current_level("master"), 1);
        assert_eq!(d.current_level("coder"), 3);

        // Reset master, coder should remain
        d.reset_agent("master");
        assert_eq!(d.current_level("master"), 0);
        assert_eq!(d.current_level("coder"), 3);
    }

    #[test]
    fn test_default_impl() {
        // Verify Default trait impl works
        let d = BudgetDowngrader::default();
        assert_eq!(d.current_level("any"), 0);
    }

    #[test]
    fn test_medium_and_local_model_accessors() {
        let d = make_downgrader();
        assert!(!d.medium_models().is_empty());
        assert!(!d.local_models().is_empty());
        assert_eq!(d.medium_models()[0].provider, "gemini");
        assert_eq!(d.local_models()[0].provider, "ollama");
    }

    #[test]
    fn test_downgrade_action_serialization() {
        let action = DowngradeAction::SwitchModel {
            from_model: "gpt-4".to_string(),
            to_model: "gemini-flash".to_string(),
            to_provider: "gemini".to_string(),
            reason: "L1 downgrade".to_string(),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("SwitchModel"));
        assert!(json.contains("gpt-4"));
        let back: DowngradeAction = serde_json::from_str(&json).unwrap();
        assert_eq!(back, action);
    }

    #[test]
    fn test_halt_action_serialization() {
        let action = DowngradeAction::HaltExecution {
            reason: "L3 budget halt".to_string(),
            spent: 9.8,
            budget: 10.0,
        };
        let json = serde_json::to_string(&action).unwrap();
        let back: DowngradeAction = serde_json::from_str(&json).unwrap();
        assert_eq!(back, action);
    }

    #[test]
    fn test_model_tier_serialization() {
        let tier = ModelTier::Expensive;
        let json = serde_json::to_string(&tier).unwrap();
        assert_eq!(json, "\"Expensive\"");
        let back: ModelTier = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ModelTier::Expensive);
    }
}
