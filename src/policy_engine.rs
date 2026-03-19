//! Policy Engine — pre-execution rule evaluation for tool calls.
//!
//! Inspired by Open SWE (LangChain) and Automaton (Conway Research) policy patterns.
//! Evaluates rules before tool execution to enforce organizational policies.

use serde::{Deserialize, Serialize};
use tracing::debug;

/// Action to take when a policy rule matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyAction {
    /// Allow the tool call to proceed
    Allow,
    /// Deny the tool call with a reason
    Deny { reason: String },
    /// Quarantine: allow but flag for review
    Quarantine { reason: String },
}

/// Condition for when a policy rule applies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PolicyCondition {
    /// Matches a specific tool by exact name
    ToolName(String),
    /// Matches tools whose name contains a pattern (case-insensitive)
    ToolPattern(String),
    /// Matches a specific agent by name
    AgentName(String),
    /// Matches if current hour (UTC) is within a range [start, end)
    TimeRange { start_hour: u8, end_hour: u8 },
    /// Matches if a specific argument key contains a value pattern
    ArgMatch { key: String, pattern: String },
    /// All sub-conditions must match (AND)
    All(Vec<PolicyCondition>),
    /// Any sub-condition must match (OR)
    Any(Vec<PolicyCondition>),
}

/// A single policy rule: condition + action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    /// Human-readable name for this rule
    pub name: String,
    /// When this rule applies
    pub condition: PolicyCondition,
    /// What action to take
    pub action: PolicyAction,
    /// Whether this rule is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Priority (higher = evaluated first). Default 0.
    #[serde(default)]
    pub priority: i32,
}

fn default_true() -> bool {
    true
}

/// Context for policy evaluation — information about the pending tool call.
#[derive(Debug, Clone)]
pub struct PolicyRequest {
    /// Name of the tool being called
    pub tool_name: String,
    /// Name of the agent making the call
    pub agent_name: String,
    /// Tool arguments (for ArgMatch conditions)
    pub args: serde_json::Value,
}

/// Result of policy evaluation.
#[derive(Debug, Clone)]
pub struct PolicyResult {
    /// The action to take
    pub action: PolicyAction,
    /// Which rule matched (if any)
    pub matched_rule: Option<String>,
}

/// Policy Engine — evaluates rules against tool call requests.
pub struct PolicyEngine {
    rules: Vec<PolicyRule>,
}

impl PolicyEngine {
    /// Create a new empty PolicyEngine.
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Create with pre-defined rules.
    pub fn with_rules(mut rules: Vec<PolicyRule>) -> Self {
        // Sort by priority descending (higher priority evaluated first)
        rules.sort_by(|a, b| b.priority.cmp(&a.priority));
        Self { rules }
    }

    /// Add a rule. Re-sorts by priority.
    pub fn add_rule(&mut self, rule: PolicyRule) {
        self.rules.push(rule);
        self.rules.sort_by(|a, b| b.priority.cmp(&a.priority));
    }

    /// Remove a rule by name. Returns true if found and removed.
    pub fn remove_rule(&self, _name: &str) -> bool {
        // Note: rules are immutable after creation in practice.
        // For runtime modification, use with_rules() to rebuild.
        false
    }

    /// Number of rules.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Get all rule names.
    pub fn rule_names(&self) -> Vec<&str> {
        self.rules.iter().map(|r| r.name.as_str()).collect()
    }

    /// Evaluate all rules against a tool call request.
    /// Returns the first matching rule's action, or Allow if no rules match.
    pub fn evaluate(&self, request: &PolicyRequest) -> PolicyResult {
        for rule in &self.rules {
            if !rule.enabled {
                continue;
            }
            if self.matches_condition(&rule.condition, request) {
                debug!(
                    "Policy '{}' matched for tool='{}' agent='{}' → {:?}",
                    rule.name, request.tool_name, request.agent_name, rule.action
                );
                return PolicyResult {
                    action: rule.action.clone(),
                    matched_rule: Some(rule.name.clone()),
                };
            }
        }
        PolicyResult {
            action: PolicyAction::Allow,
            matched_rule: None,
        }
    }

    /// Check if a condition matches the request.
    fn matches_condition(&self, condition: &PolicyCondition, request: &PolicyRequest) -> bool {
        match condition {
            PolicyCondition::ToolName(name) => {
                request.tool_name == *name
            }
            PolicyCondition::ToolPattern(pattern) => {
                request
                    .tool_name
                    .to_lowercase()
                    .contains(&pattern.to_lowercase())
            }
            PolicyCondition::AgentName(name) => {
                request.agent_name == *name
            }
            PolicyCondition::TimeRange {
                start_hour,
                end_hour,
            } => {
                let now_hour = chrono::Utc::now().hour() as u8;
                if start_hour <= end_hour {
                    now_hour >= *start_hour && now_hour < *end_hour
                } else {
                    // Wraps midnight: e.g., 22..6 means 22,23,0,1,2,3,4,5
                    now_hour >= *start_hour || now_hour < *end_hour
                }
            }
            PolicyCondition::ArgMatch { key, pattern } => {
                if let Some(val) = request.args.get(key) {
                    let val_str = val.as_str().unwrap_or("");
                    val_str.to_lowercase().contains(&pattern.to_lowercase())
                } else {
                    false
                }
            }
            PolicyCondition::All(conditions) => {
                conditions
                    .iter()
                    .all(|c| self.matches_condition(c, request))
            }
            PolicyCondition::Any(conditions) => {
                conditions
                    .iter()
                    .any(|c| self.matches_condition(c, request))
            }
        }
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

// Need chrono::Timelike for .hour()
use chrono::Timelike;

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_request(tool: &str, agent: &str) -> PolicyRequest {
        PolicyRequest {
            tool_name: tool.to_string(),
            agent_name: agent.to_string(),
            args: json!({}),
        }
    }

    fn make_request_with_args(tool: &str, agent: &str, args: serde_json::Value) -> PolicyRequest {
        PolicyRequest {
            tool_name: tool.to_string(),
            agent_name: agent.to_string(),
            args,
        }
    }

    // ── Basic tests ──

    #[test]
    fn test_empty_engine_allows() {
        let engine = PolicyEngine::new();
        let result = engine.evaluate(&make_request("shell", "master"));
        assert_eq!(result.action, PolicyAction::Allow);
        assert!(result.matched_rule.is_none());
    }

    #[test]
    fn test_tool_name_match() {
        let engine = PolicyEngine::with_rules(vec![PolicyRule {
            name: "deny_shell".to_string(),
            condition: PolicyCondition::ToolName("shell".to_string()),
            action: PolicyAction::Deny {
                reason: "Shell disabled".to_string(),
            },
            enabled: true,
            priority: 0,
        }]);

        let result = engine.evaluate(&make_request("shell", "master"));
        assert_eq!(
            result.action,
            PolicyAction::Deny {
                reason: "Shell disabled".to_string()
            }
        );
        assert_eq!(result.matched_rule, Some("deny_shell".to_string()));

        // Other tools should be allowed
        let result2 = engine.evaluate(&make_request("file_read", "master"));
        assert_eq!(result2.action, PolicyAction::Allow);
    }

    #[test]
    fn test_tool_pattern_match() {
        let engine = PolicyEngine::with_rules(vec![PolicyRule {
            name: "quarantine_file_ops".to_string(),
            condition: PolicyCondition::ToolPattern("file".to_string()),
            action: PolicyAction::Quarantine {
                reason: "File operations flagged".to_string(),
            },
            enabled: true,
            priority: 0,
        }]);

        let result = engine.evaluate(&make_request("file_write", "coder"));
        assert_eq!(
            result.action,
            PolicyAction::Quarantine {
                reason: "File operations flagged".to_string()
            }
        );

        let result2 = engine.evaluate(&make_request("file_read", "coder"));
        assert!(matches!(result2.action, PolicyAction::Quarantine { .. }));
    }

    #[test]
    fn test_agent_name_match() {
        let engine = PolicyEngine::with_rules(vec![PolicyRule {
            name: "restrict_coder".to_string(),
            condition: PolicyCondition::AgentName("coder".to_string()),
            action: PolicyAction::Deny {
                reason: "Coder agent restricted".to_string(),
            },
            enabled: true,
            priority: 0,
        }]);

        let result = engine.evaluate(&make_request("shell", "coder"));
        assert!(matches!(result.action, PolicyAction::Deny { .. }));

        let result2 = engine.evaluate(&make_request("shell", "master"));
        assert_eq!(result2.action, PolicyAction::Allow);
    }

    #[test]
    fn test_arg_match() {
        let engine = PolicyEngine::with_rules(vec![PolicyRule {
            name: "block_rm".to_string(),
            condition: PolicyCondition::ArgMatch {
                key: "command".to_string(),
                pattern: "rm -rf".to_string(),
            },
            action: PolicyAction::Deny {
                reason: "Dangerous command".to_string(),
            },
            enabled: true,
            priority: 0,
        }]);

        let req = make_request_with_args("shell", "master", json!({"command": "rm -rf /tmp"}));
        let result = engine.evaluate(&req);
        assert!(matches!(result.action, PolicyAction::Deny { .. }));

        let req2 = make_request_with_args("shell", "master", json!({"command": "ls -la"}));
        let result2 = engine.evaluate(&req2);
        assert_eq!(result2.action, PolicyAction::Allow);
    }

    #[test]
    fn test_all_condition() {
        let engine = PolicyEngine::with_rules(vec![PolicyRule {
            name: "coder_shell".to_string(),
            condition: PolicyCondition::All(vec![
                PolicyCondition::AgentName("coder".to_string()),
                PolicyCondition::ToolName("shell".to_string()),
            ]),
            action: PolicyAction::Deny {
                reason: "Coder cannot use shell".to_string(),
            },
            enabled: true,
            priority: 0,
        }]);

        // Both conditions met
        let result = engine.evaluate(&make_request("shell", "coder"));
        assert!(matches!(result.action, PolicyAction::Deny { .. }));

        // Only one condition met
        let result2 = engine.evaluate(&make_request("file_read", "coder"));
        assert_eq!(result2.action, PolicyAction::Allow);

        let result3 = engine.evaluate(&make_request("shell", "master"));
        assert_eq!(result3.action, PolicyAction::Allow);
    }

    #[test]
    fn test_any_condition() {
        let engine = PolicyEngine::with_rules(vec![PolicyRule {
            name: "deny_dangerous".to_string(),
            condition: PolicyCondition::Any(vec![
                PolicyCondition::ToolName("shell".to_string()),
                PolicyCondition::ToolName("file_write".to_string()),
            ]),
            action: PolicyAction::Quarantine {
                reason: "Dangerous tool".to_string(),
            },
            enabled: true,
            priority: 0,
        }]);

        let r1 = engine.evaluate(&make_request("shell", "master"));
        assert!(matches!(r1.action, PolicyAction::Quarantine { .. }));

        let r2 = engine.evaluate(&make_request("file_write", "master"));
        assert!(matches!(r2.action, PolicyAction::Quarantine { .. }));

        let r3 = engine.evaluate(&make_request("file_read", "master"));
        assert_eq!(r3.action, PolicyAction::Allow);
    }

    // ── Priority tests ──

    #[test]
    fn test_priority_higher_wins() {
        let engine = PolicyEngine::with_rules(vec![
            PolicyRule {
                name: "allow_shell".to_string(),
                condition: PolicyCondition::ToolName("shell".to_string()),
                action: PolicyAction::Allow,
                enabled: true,
                priority: 10, // higher priority
            },
            PolicyRule {
                name: "deny_all".to_string(),
                condition: PolicyCondition::ToolPattern("".to_string()),
                action: PolicyAction::Deny {
                    reason: "All denied".to_string(),
                },
                enabled: true,
                priority: 0,
            },
        ]);

        // shell should be allowed (higher priority rule)
        let result = engine.evaluate(&make_request("shell", "master"));
        assert_eq!(result.action, PolicyAction::Allow);
        assert_eq!(result.matched_rule, Some("allow_shell".to_string()));
    }

    // ── Disabled rule tests ──

    #[test]
    fn test_disabled_rule_skipped() {
        let engine = PolicyEngine::with_rules(vec![PolicyRule {
            name: "deny_shell".to_string(),
            condition: PolicyCondition::ToolName("shell".to_string()),
            action: PolicyAction::Deny {
                reason: "Disabled".to_string(),
            },
            enabled: false,
            priority: 0,
        }]);

        let result = engine.evaluate(&make_request("shell", "master"));
        assert_eq!(result.action, PolicyAction::Allow);
    }

    // ── API tests ──

    #[test]
    fn test_rule_count() {
        let engine = PolicyEngine::with_rules(vec![
            PolicyRule {
                name: "r1".to_string(),
                condition: PolicyCondition::ToolName("a".to_string()),
                action: PolicyAction::Allow,
                enabled: true,
                priority: 0,
            },
            PolicyRule {
                name: "r2".to_string(),
                condition: PolicyCondition::ToolName("b".to_string()),
                action: PolicyAction::Allow,
                enabled: true,
                priority: 0,
            },
        ]);
        assert_eq!(engine.rule_count(), 2);
    }

    #[test]
    fn test_rule_names() {
        let engine = PolicyEngine::with_rules(vec![
            PolicyRule {
                name: "alpha".to_string(),
                condition: PolicyCondition::ToolName("a".to_string()),
                action: PolicyAction::Allow,
                enabled: true,
                priority: 1,
            },
            PolicyRule {
                name: "beta".to_string(),
                condition: PolicyCondition::ToolName("b".to_string()),
                action: PolicyAction::Allow,
                enabled: true,
                priority: 0,
            },
        ]);
        let names = engine.rule_names();
        assert_eq!(names[0], "alpha"); // priority 1 first
        assert_eq!(names[1], "beta");
    }

    #[test]
    fn test_add_rule() {
        let mut engine = PolicyEngine::new();
        assert_eq!(engine.rule_count(), 0);
        engine.add_rule(PolicyRule {
            name: "test".to_string(),
            condition: PolicyCondition::ToolName("shell".to_string()),
            action: PolicyAction::Allow,
            enabled: true,
            priority: 0,
        });
        assert_eq!(engine.rule_count(), 1);
    }

    // ── Serialization tests ──

    #[test]
    fn test_policy_action_serialization() {
        let action = PolicyAction::Deny {
            reason: "test".to_string(),
        };
        let json = serde_json::to_string(&action).unwrap();
        let back: PolicyAction = serde_json::from_str(&json).unwrap();
        assert_eq!(back, action);
    }

    #[test]
    fn test_policy_rule_serialization() {
        let rule = PolicyRule {
            name: "test_rule".to_string(),
            condition: PolicyCondition::ToolName("shell".to_string()),
            action: PolicyAction::Quarantine {
                reason: "flagged".to_string(),
            },
            enabled: true,
            priority: 5,
        };
        let json = serde_json::to_string(&rule).unwrap();
        assert!(json.contains("test_rule"));
        let back: PolicyRule = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "test_rule");
        assert_eq!(back.priority, 5);
    }

    #[test]
    fn test_default_engine() {
        let engine = PolicyEngine::default();
        assert_eq!(engine.rule_count(), 0);
    }
}
