//! Tiered approval gate -- requires human confirmation via Telegram for risky actions.
//! Supports four tiers: Auto (no approval), Single (one human), Multi (2+ approvers),
//! and Emergency (bypass with post-facto audit).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};
use tokio::time::{timeout, Duration};
use tracing::{info, warn};

use crate::audit_log::{AuditLogger, ActionType, Outcome, RiskLevel};

// ── Approval Tiers ───────────────────────────────────────────────────────────

/// Approval tier -- determines what level of human approval is needed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApprovalTier {
    /// No approval needed -- low-risk reads, memory ops, safe queries
    Auto,
    /// One human approval via Telegram -- external sends, file writes
    Single,
    /// Requires 2+ approvals -- payments, production deploys, data exports
    Multi,
    /// Bypass with post-facto audit -- critical recovery ops
    Emergency,
}

impl std::fmt::Display for ApprovalTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApprovalTier::Auto => write!(f, "Auto"),
            ApprovalTier::Single => write!(f, "Single"),
            ApprovalTier::Multi => write!(f, "Multi"),
            ApprovalTier::Emergency => write!(f, "Emergency"),
        }
    }
}

// ── Approval Policy ──────────────────────────────────────────────────────────

/// Policy for a given approval tier -- how many approvers, timeout, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalPolicy {
    /// Which tier this policy applies to
    pub tier: ApprovalTier,
    /// Number of approvers required (0 for Auto, 1 for Single, 2+ for Multi)
    pub required_approvers: u8,
    /// Timeout in seconds for waiting for approval
    pub timeout_secs: u64,
    /// Whether to auto-deny on timeout (true) or auto-approve (false, for Emergency)
    pub auto_deny_on_timeout: bool,
}

impl ApprovalPolicy {
    /// Default policy for Auto tier
    pub fn auto() -> Self {
        Self {
            tier: ApprovalTier::Auto,
            required_approvers: 0,
            timeout_secs: 0,
            auto_deny_on_timeout: false,
        }
    }

    /// Default policy for Single tier
    pub fn single() -> Self {
        Self {
            tier: ApprovalTier::Single,
            required_approvers: 1,
            timeout_secs: 300,
            auto_deny_on_timeout: true,
        }
    }

    /// Default policy for Multi tier
    pub fn multi() -> Self {
        Self {
            tier: ApprovalTier::Multi,
            required_approvers: 2,
            timeout_secs: 600,
            auto_deny_on_timeout: true,
        }
    }

    /// Default policy for Emergency tier
    pub fn emergency() -> Self {
        Self {
            tier: ApprovalTier::Emergency,
            required_approvers: 0,
            timeout_secs: 0,
            auto_deny_on_timeout: false,
        }
    }

    /// Get the default policy for a given tier
    pub fn for_tier(tier: &ApprovalTier) -> Self {
        match tier {
            ApprovalTier::Auto => Self::auto(),
            ApprovalTier::Single => Self::single(),
            ApprovalTier::Multi => Self::multi(),
            ApprovalTier::Emergency => Self::emergency(),
        }
    }
}

// ── Tier Mapping ─────────────────────────────────────────────────────────────

/// Determine the approval tier for a tool invocation.
/// Takes the tool name and optional arguments (e.g., to detect GET vs POST for http_request).
pub fn tier_for_tool(tool_name: &str, args: Option<&Value>) -> ApprovalTier {
    match tool_name {
        // Auto tier -- safe reads, lookups, memory operations
        "file_read" | "glob" | "content_search" | "memory_recall" | "memory_store"
        | "memory_forget" | "web_search" | "vision" | "translate" | "json_transform"
        | "csv_parse" | "summarize" => ApprovalTier::Auto,

        // http_request: GET is Auto, everything else is Single
        "http_request" => {
            if let Some(a) = args {
                let method = a.get("method")
                    .and_then(|v| v.as_str())
                    .unwrap_or("GET")
                    .to_uppercase();
                if method == "GET" {
                    ApprovalTier::Auto
                } else {
                    ApprovalTier::Single
                }
            } else {
                // No args means default GET
                ApprovalTier::Auto
            }
        }

        // Single tier -- writes, external sends, code execution
        "file_write" | "file_edit" | "shell" | "email_send" | "email"
        | "slack_send" | "discord_send" | "line_send" | "whatsapp_send"
        | "blog_publish" | "twitter" | "ai_code" | "computer_use" | "browser"
        | "image_generate" | "pdf_export" | "docx_export" | "xlsx_export"
        | "skeleton_generate" | "delegate" | "delegate_to_provider"
        | "run_hand" | "cli_anything" => ApprovalTier::Single,

        // Multi tier -- payments, production deploys, data exports, SaaS scaffolding
        "stripe" | "render_deploy" | "scaffold_saas" => ApprovalTier::Multi,

        // Default: anything unknown is Single (safe default)
        _ => ApprovalTier::Single,
    }
}

/// Check if an operation name maps to the Emergency tier.
/// Emergency operations are identified by a special prefix or explicit name.
pub fn is_emergency_operation(operation: &str) -> bool {
    matches!(
        operation,
        "system_recovery" | "rollback" | "emergency_stop" | "cluster_recovery"
        | "force_restart" | "data_restore"
    ) || operation.starts_with("emergency_")
}

// ── Result & Config ──────────────────────────────────────────────────────────

/// Result of an approval request
#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalResult {
    Approved,
    Denied,
    Timeout,
}

/// Configuration for the approval gate
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApprovalConfig {
    /// Tools that require approval before execution (legacy -- kept for backward compat)
    #[serde(default = "default_approval_tools")]
    pub tools_requiring_approval: Vec<String>,
    /// Timeout in seconds for waiting for approval (default: 300 = 5 min)
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Whether approval gate is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Whether tiered approval is enabled (default: true).
    /// When true, tier_for_tool() determines approval path.
    /// When false, falls back to legacy tools_requiring_approval list.
    #[serde(default = "default_true")]
    pub tiered_enabled: bool,
    /// Custom tier policies (overrides defaults per tier)
    #[serde(default)]
    pub tier_policies: HashMap<String, TierPolicyConfig>,
}

/// Serializable tier policy config (for TOML/JSON)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TierPolicyConfig {
    pub required_approvers: Option<u8>,
    pub timeout_secs: Option<u64>,
    pub auto_deny_on_timeout: Option<bool>,
}

fn default_approval_tools() -> Vec<String> {
    vec![
        "email".to_string(),       // Sending emails
        "http_request".to_string(), // External API calls (POST/PUT/DELETE)
    ]
}

fn default_timeout() -> u64 { 300 }
fn default_true() -> bool { true }

impl Default for ApprovalConfig {
    fn default() -> Self {
        Self {
            tools_requiring_approval: default_approval_tools(),
            timeout_secs: default_timeout(),
            enabled: true,
            tiered_enabled: true,
            tier_policies: HashMap::new(),
        }
    }
}

// ── Pending Approval Types ───────────────────────────────────────────────────

/// Pending single approval request
struct PendingApproval {
    tx: oneshot::Sender<bool>,
}

/// Pending multi-approval request -- collects votes from multiple approvers
struct PendingMultiApproval {
    /// How many approvals are needed
    required: u8,
    /// Current approval count
    approvals: u8,
    /// Current denial count
    denials: u8,
    /// Sender to notify when quorum is reached
    tx: Option<oneshot::Sender<bool>>,
}

/// Async function type for sending approval notifications (e.g., via Telegram).
/// Takes (chat_id, message) and sends it. chat_id may be empty if broadcast.
pub type ApprovalNotifier = Arc<dyn Fn(String) -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

// ── ApprovalGate ─────────────────────────────────────────────────────────────

/// Approval gate -- manages pending approval requests from agents.
/// Supports tiered approval: Auto, Single, Multi, and Emergency.
///
/// When an agent tries to use a tool:
/// 1. Determine the tier via tier_for_tool()
/// 2. Auto: execute immediately
/// 3. Single: send Telegram message, wait for one Yes/No
/// 4. Multi: send Telegram message, collect required_approvers count of approvals
/// 5. Emergency: execute immediately, log audit entry for post-facto review
pub struct ApprovalGate {
    config: ApprovalConfig,
    /// Pending single approvals: approval_id -> sender
    pending: Arc<Mutex<HashMap<String, PendingApproval>>>,
    /// Pending multi approvals: approval_id -> multi-approval state
    pending_multi: Arc<Mutex<HashMap<String, PendingMultiApproval>>>,
    /// Optional notifier to send approval messages (e.g., Telegram)
    notifier: tokio::sync::RwLock<Option<ApprovalNotifier>>,
    /// Optional audit logger for recording approval decisions
    audit_logger: tokio::sync::RwLock<Option<Arc<AuditLogger>>>,
}

impl ApprovalGate {
    pub fn new(config: ApprovalConfig) -> Self {
        Self {
            config,
            pending: Arc::new(Mutex::new(HashMap::new())),
            pending_multi: Arc::new(Mutex::new(HashMap::new())),
            notifier: tokio::sync::RwLock::new(None),
            audit_logger: tokio::sync::RwLock::new(None),
        }
    }

    /// Set the audit logger for recording approval decisions.
    pub async fn set_audit_logger(&self, logger: Arc<AuditLogger>) {
        *self.audit_logger.write().await = Some(logger);
    }

    /// Set the notifier callback for sending approval request messages.
    pub async fn set_notifier(&self, notifier: ApprovalNotifier) {
        *self.notifier.write().await = Some(notifier);
    }

    /// Get the effective policy for a tier, applying any config overrides.
    pub fn policy_for_tier(&self, tier: &ApprovalTier) -> ApprovalPolicy {
        let mut policy = ApprovalPolicy::for_tier(tier);
        let tier_key = format!("{}", tier).to_lowercase();
        if let Some(override_cfg) = self.config.tier_policies.get(&tier_key) {
            if let Some(n) = override_cfg.required_approvers {
                policy.required_approvers = n;
            }
            if let Some(t) = override_cfg.timeout_secs {
                policy.timeout_secs = t;
            }
            if let Some(d) = override_cfg.auto_deny_on_timeout {
                policy.auto_deny_on_timeout = d;
            }
        }
        policy
    }

    /// Check if a tool requires approval.
    /// When tiered_enabled=true, uses tier_for_tool() logic.
    /// When tiered_enabled=false, uses the legacy tools_requiring_approval list.
    pub fn requires_approval(&self, tool_name: &str) -> bool {
        if !self.config.enabled {
            return false;
        }
        if self.config.tiered_enabled {
            let tier = tier_for_tool(tool_name, None);
            !matches!(tier, ApprovalTier::Auto)
        } else {
            self.config.tools_requiring_approval.iter().any(|t| t == tool_name)
        }
    }

    /// Determine the tier for a tool call, considering arguments.
    pub fn get_tier(&self, tool_name: &str, args: Option<&Value>) -> ApprovalTier {
        if !self.config.enabled {
            return ApprovalTier::Auto;
        }
        if self.config.tiered_enabled {
            tier_for_tool(tool_name, args)
        } else {
            // Legacy mode: check the list
            if self.config.tools_requiring_approval.iter().any(|t| t == tool_name) {
                ApprovalTier::Single
            } else {
                ApprovalTier::Auto
            }
        }
    }

    /// Check the tier and request appropriate approval.
    /// Returns immediately for Auto and Emergency tiers.
    /// Blocks for Single and Multi tiers until approval/denial/timeout.
    pub async fn check_and_request(
        &self,
        tool_name: &str,
        description: &str,
        args: Option<&Value>,
    ) -> (String, ApprovalResult) {
        let tier = self.get_tier(tool_name, args);
        let policy = self.policy_for_tier(&tier);

        match tier {
            ApprovalTier::Auto => {
                let id = self.generate_id();
                info!("Tier Auto -- auto-approved tool '{}': {}", tool_name, description);
                self.log_audit_entry(&id, tool_name, description, &tier, "auto-approved").await;
                (id, ApprovalResult::Approved)
            }
            ApprovalTier::Emergency => {
                let id = self.generate_id();
                warn!("Tier Emergency -- bypassed approval for '{}': {}", tool_name, description);
                self.log_audit_entry(&id, tool_name, description, &tier, "emergency-bypassed").await;
                // Send notification for post-facto audit
                self.send_notification(&format!(
                    "EMERGENCY BYPASS\n\n\
                     Tool: {}\n\
                     Action: {}\n\
                     ID: {}\n\n\
                     This action was executed without approval.\n\
                     Please review the audit log.",
                    tool_name, description, id
                )).await;
                (id, ApprovalResult::Approved)
            }
            ApprovalTier::Single => {
                self.request_single(tool_name, description, &policy).await
            }
            ApprovalTier::Multi => {
                self.request_multi(tool_name, description, &policy).await
            }
        }
    }

    /// Request single-approver approval.
    async fn request_single(
        &self,
        tool_name: &str,
        description: &str,
        policy: &ApprovalPolicy,
    ) -> (String, ApprovalResult) {
        let approval_id = self.generate_id();

        let (tx, rx) = oneshot::channel::<bool>();

        {
            let mut pending = self.pending.lock().await;
            pending.insert(approval_id.clone(), PendingApproval { tx });
        }

        info!("Tier Single -- approval requested: {} for tool '{}': {}", approval_id, tool_name, description);

        // Send notification
        let msg = self.format_tiered_message(&approval_id, tool_name, description, &ApprovalTier::Single, policy);
        self.send_notification(&msg).await;

        // Wait for response with timeout
        let timeout_duration = Duration::from_secs(policy.timeout_secs);
        let result = match timeout(timeout_duration, rx).await {
            Ok(Ok(true)) => {
                info!("Approval {} GRANTED", approval_id);
                ApprovalResult::Approved
            }
            Ok(Ok(false)) => {
                info!("Approval {} DENIED", approval_id);
                ApprovalResult::Denied
            }
            Ok(Err(_)) => {
                warn!("Approval {} channel dropped", approval_id);
                ApprovalResult::Timeout
            }
            Err(_) => {
                let mut pending = self.pending.lock().await;
                pending.remove(&approval_id);
                if policy.auto_deny_on_timeout {
                    warn!("Approval {} TIMED OUT (auto-denied)", approval_id);
                    ApprovalResult::Denied
                } else {
                    warn!("Approval {} TIMED OUT", approval_id);
                    ApprovalResult::Timeout
                }
            }
        };

        let result_str = match &result {
            ApprovalResult::Approved => "approved",
            ApprovalResult::Denied => "denied",
            ApprovalResult::Timeout => "timeout",
        };
        self.log_audit_entry(&approval_id, tool_name, description, &ApprovalTier::Single, result_str).await;

        (approval_id, result)
    }

    /// Request multi-approver approval.
    /// Sends notification to all admins and waits for required_approvers count of /approve responses.
    async fn request_multi(
        &self,
        tool_name: &str,
        description: &str,
        policy: &ApprovalPolicy,
    ) -> (String, ApprovalResult) {
        let approval_id = self.generate_id();

        let (tx, rx) = oneshot::channel::<bool>();

        {
            let mut pending_multi = self.pending_multi.lock().await;
            pending_multi.insert(approval_id.clone(), PendingMultiApproval {
                required: policy.required_approvers,
                approvals: 0,
                denials: 0,
                tx: Some(tx),
            });
        }

        info!(
            "Tier Multi -- approval requested: {} for tool '{}' (need {} approvers): {}",
            approval_id, tool_name, policy.required_approvers, description
        );

        // Send notification
        let msg = self.format_tiered_message(&approval_id, tool_name, description, &ApprovalTier::Multi, policy);
        self.send_notification(&msg).await;

        // Wait for quorum with timeout
        let timeout_duration = Duration::from_secs(policy.timeout_secs);
        let result = match timeout(timeout_duration, rx).await {
            Ok(Ok(true)) => {
                info!("Multi-approval {} GRANTED (quorum reached)", approval_id);
                ApprovalResult::Approved
            }
            Ok(Ok(false)) => {
                info!("Multi-approval {} DENIED", approval_id);
                ApprovalResult::Denied
            }
            Ok(Err(_)) => {
                warn!("Multi-approval {} channel dropped", approval_id);
                ApprovalResult::Timeout
            }
            Err(_) => {
                let mut pending_multi = self.pending_multi.lock().await;
                pending_multi.remove(&approval_id);
                if policy.auto_deny_on_timeout {
                    warn!("Multi-approval {} TIMED OUT (auto-denied)", approval_id);
                    ApprovalResult::Denied
                } else {
                    warn!("Multi-approval {} TIMED OUT", approval_id);
                    ApprovalResult::Timeout
                }
            }
        };

        let result_str = match &result {
            ApprovalResult::Approved => "approved",
            ApprovalResult::Denied => "denied",
            ApprovalResult::Timeout => "timeout",
        };
        self.log_audit_entry(&approval_id, tool_name, description, &ApprovalTier::Multi, result_str).await;

        (approval_id, result)
    }

    /// Legacy request method -- uses tier-aware approval path when tiered_enabled=true,
    /// otherwise uses Single tier with legacy timeout.
    /// Kept for backward compatibility with existing callers.
    pub async fn request(&self, tool_name: &str, description: &str) -> (String, ApprovalResult) {
        if self.config.tiered_enabled {
            self.check_and_request(tool_name, description, None).await
        } else {
            let policy = ApprovalPolicy {
                tier: ApprovalTier::Single,
                required_approvers: 1,
                timeout_secs: self.config.timeout_secs,
                auto_deny_on_timeout: false,
            };
            self.request_single(tool_name, description, &policy).await
        }
    }

    /// Respond to a pending approval (Single or Multi tier).
    /// For Multi tier, accumulates votes until quorum is reached.
    /// Returns true if the approval_id was found and response was recorded.
    pub async fn respond(&self, approval_id: &str, approved: bool) -> bool {
        // Check single approvals first
        {
            let mut pending = self.pending.lock().await;
            if let Some(approval) = pending.remove(approval_id) {
                let _ = approval.tx.send(approved);

                // Audit log the approval decision
                let audit = self.audit_logger.read().await;
                if let Some(ref logger) = *audit {
                    let outcome = if approved { Outcome::Success } else { Outcome::Failure };
                    let details = serde_json::json!({
                        "approved": approved,
                        "approval_id": approval_id,
                        "tier": "single",
                    });
                    let _ = logger.log_action(
                        "human",
                        ActionType::ApprovalDecision,
                        None,
                        Some(approval_id),
                        Some(details),
                        outcome,
                        None,
                        RiskLevel::Medium,
                    ).await;
                }

                return true;
            }
        }

        // Check multi approvals
        {
            let mut pending_multi = self.pending_multi.lock().await;
            if let Some(multi) = pending_multi.get_mut(approval_id) {
                if approved {
                    multi.approvals += 1;
                    info!(
                        "Multi-approval {} vote: approve ({}/{})",
                        approval_id, multi.approvals, multi.required
                    );
                    if multi.approvals >= multi.required {
                        // Quorum reached -- approve
                        if let Some(tx) = multi.tx.take() {
                            let _ = tx.send(true);
                        }
                        // Log quorum reached
                        let audit = self.audit_logger.read().await;
                        if let Some(ref logger) = *audit {
                            let details = serde_json::json!({
                                "approval_id": approval_id,
                                "tier": "multi",
                                "approvals": multi.approvals,
                                "required": multi.required,
                                "quorum_reached": true,
                            });
                            let _ = logger.log_action(
                                "human",
                                ActionType::ApprovalDecision,
                                None,
                                Some(approval_id),
                                Some(details),
                                Outcome::Success,
                                None,
                                RiskLevel::High,
                            ).await;
                        }
                        pending_multi.remove(approval_id);
                    }
                } else {
                    multi.denials += 1;
                    info!(
                        "Multi-approval {} vote: deny ({} denials)",
                        approval_id, multi.denials
                    );
                    // Any single denial kills the multi-approval
                    if let Some(tx) = multi.tx.take() {
                        let _ = tx.send(false);
                    }
                    // Log denial
                    let audit = self.audit_logger.read().await;
                    if let Some(ref logger) = *audit {
                        let details = serde_json::json!({
                            "approval_id": approval_id,
                            "tier": "multi",
                            "denials": multi.denials,
                            "denied_by_veto": true,
                        });
                        let _ = logger.log_action(
                            "human",
                            ActionType::ApprovalDecision,
                            None,
                            Some(approval_id),
                            Some(details),
                            Outcome::Failure,
                            None,
                            RiskLevel::High,
                        ).await;
                    }
                    pending_multi.remove(approval_id);
                }
                return true;
            }
        }

        false
    }

    /// Format an approval message with tier information.
    fn format_tiered_message(
        &self,
        approval_id: &str,
        tool_name: &str,
        description: &str,
        tier: &ApprovalTier,
        policy: &ApprovalPolicy,
    ) -> String {
        let tier_label = match tier {
            ApprovalTier::Single => "SINGLE APPROVAL".to_string(),
            ApprovalTier::Multi => format!("MULTI APPROVAL (need {}/{})", policy.required_approvers, policy.required_approvers),
            _ => "APPROVAL".to_string(),
        };

        format!(
            "{}\n\n\
             Tool: {}\n\
             Action: {}\n\n\
             Reply with:\n\
             /approve {} -- to allow\n\
             /deny {} -- to deny\n\n\
             (Auto-{} in {} seconds)",
            tier_label,
            tool_name, description,
            approval_id, approval_id,
            if policy.auto_deny_on_timeout { "denies" } else { "expires" },
            policy.timeout_secs
        )
    }

    /// Legacy format method (backward compat)
    pub fn format_approval_message(&self, approval_id: &str, tool_name: &str, description: &str) -> String {
        let tier = if self.config.tiered_enabled {
            tier_for_tool(tool_name, None)
        } else {
            ApprovalTier::Single
        };
        let policy = self.policy_for_tier(&tier);
        self.format_tiered_message(approval_id, tool_name, description, &tier, &policy)
    }

    /// List pending approval count (both single and multi)
    pub async fn pending_count(&self) -> usize {
        let single = self.pending.lock().await.len();
        let multi = self.pending_multi.lock().await.len();
        single + multi
    }

    /// Get all pending approval IDs (both single and multi)
    pub async fn pending_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.pending.lock().await.keys().cloned().collect();
        ids.extend(self.pending_multi.lock().await.keys().cloned());
        ids
    }

    /// Get multi-approval status for a pending request.
    /// Returns (current_approvals, required_approvals) or None if not found.
    pub async fn multi_approval_status(&self, approval_id: &str) -> Option<(u8, u8)> {
        let pending_multi = self.pending_multi.lock().await;
        pending_multi.get(approval_id).map(|m| (m.approvals, m.required))
    }

    // ── Internal helpers ──────────────────────────────────────────────

    fn generate_id(&self) -> String {
        format!("approval_{}", &uuid::Uuid::new_v4().to_string().replace('-', "")[..8])
    }

    async fn send_notification(&self, msg: &str) {
        let notifier = self.notifier.read().await;
        if let Some(ref notify_fn) = *notifier {
            notify_fn(msg.to_string()).await;
        } else {
            warn!("No notifier set for ApprovalGate -- notification not sent");
        }
    }

    async fn log_audit_entry(
        &self,
        id: &str,
        tool_name: &str,
        description: &str,
        tier: &ApprovalTier,
        result_str: &str,
    ) {
        let audit = self.audit_logger.read().await;
        if let Some(ref logger) = *audit {
            let risk = match tier {
                ApprovalTier::Auto => RiskLevel::Low,
                ApprovalTier::Single => RiskLevel::Medium,
                ApprovalTier::Multi => RiskLevel::High,
                ApprovalTier::Emergency => RiskLevel::Critical,
            };
            let outcome = match result_str {
                "auto-approved" | "approved" | "emergency-bypassed" => Outcome::Success,
                "denied" => Outcome::Failure,
                _ => Outcome::Failure,
            };
            let details = serde_json::json!({
                "tier": format!("{}", tier),
                "result": result_str,
                "description": description,
            });
            let _ = logger.log_action(
                "system",
                ActionType::ApprovalDecision,
                Some(tool_name),
                Some(id),
                Some(details),
                outcome,
                None,
                risk,
            ).await;
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Tier mapping tests ───────────────────────────────────────────

    #[test]
    fn test_tier_auto_tools() {
        assert_eq!(tier_for_tool("file_read", None), ApprovalTier::Auto);
        assert_eq!(tier_for_tool("glob", None), ApprovalTier::Auto);
        assert_eq!(tier_for_tool("content_search", None), ApprovalTier::Auto);
        assert_eq!(tier_for_tool("memory_recall", None), ApprovalTier::Auto);
        assert_eq!(tier_for_tool("web_search", None), ApprovalTier::Auto);
        assert_eq!(tier_for_tool("translate", None), ApprovalTier::Auto);
        assert_eq!(tier_for_tool("json_transform", None), ApprovalTier::Auto);
        assert_eq!(tier_for_tool("csv_parse", None), ApprovalTier::Auto);
        assert_eq!(tier_for_tool("summarize", None), ApprovalTier::Auto);
        assert_eq!(tier_for_tool("vision", None), ApprovalTier::Auto);
    }

    #[test]
    fn test_tier_http_request_get_is_auto() {
        let args = serde_json::json!({"url": "https://example.com", "method": "GET"});
        assert_eq!(tier_for_tool("http_request", Some(&args)), ApprovalTier::Auto);

        // Default (no method) is GET = Auto
        let args_no_method = serde_json::json!({"url": "https://example.com"});
        assert_eq!(tier_for_tool("http_request", Some(&args_no_method)), ApprovalTier::Auto);

        // No args at all = Auto (default GET)
        assert_eq!(tier_for_tool("http_request", None), ApprovalTier::Auto);
    }

    #[test]
    fn test_tier_http_request_post_is_single() {
        let args = serde_json::json!({"url": "https://example.com", "method": "POST"});
        assert_eq!(tier_for_tool("http_request", Some(&args)), ApprovalTier::Single);

        let args_put = serde_json::json!({"url": "https://example.com", "method": "PUT"});
        assert_eq!(tier_for_tool("http_request", Some(&args_put)), ApprovalTier::Single);

        let args_delete = serde_json::json!({"url": "https://example.com", "method": "DELETE"});
        assert_eq!(tier_for_tool("http_request", Some(&args_delete)), ApprovalTier::Single);
    }

    #[test]
    fn test_tier_single_tools() {
        assert_eq!(tier_for_tool("file_write", None), ApprovalTier::Single);
        assert_eq!(tier_for_tool("file_edit", None), ApprovalTier::Single);
        assert_eq!(tier_for_tool("shell", None), ApprovalTier::Single);
        assert_eq!(tier_for_tool("email_send", None), ApprovalTier::Single);
        assert_eq!(tier_for_tool("email", None), ApprovalTier::Single);
        assert_eq!(tier_for_tool("slack_send", None), ApprovalTier::Single);
        assert_eq!(tier_for_tool("discord_send", None), ApprovalTier::Single);
        assert_eq!(tier_for_tool("blog_publish", None), ApprovalTier::Single);
        assert_eq!(tier_for_tool("twitter", None), ApprovalTier::Single);
        assert_eq!(tier_for_tool("ai_code", None), ApprovalTier::Single);
        assert_eq!(tier_for_tool("browser", None), ApprovalTier::Single);
        assert_eq!(tier_for_tool("image_generate", None), ApprovalTier::Single);
        assert_eq!(tier_for_tool("cli_anything", None), ApprovalTier::Single);
    }

    #[test]
    fn test_tier_multi_tools() {
        assert_eq!(tier_for_tool("stripe", None), ApprovalTier::Multi);
        assert_eq!(tier_for_tool("render_deploy", None), ApprovalTier::Multi);
        assert_eq!(tier_for_tool("scaffold_saas", None), ApprovalTier::Multi);
    }

    #[test]
    fn test_tier_unknown_defaults_to_single() {
        assert_eq!(tier_for_tool("unknown_tool", None), ApprovalTier::Single);
        assert_eq!(tier_for_tool("some_new_tool", None), ApprovalTier::Single);
    }

    #[test]
    fn test_emergency_operations() {
        assert!(is_emergency_operation("system_recovery"));
        assert!(is_emergency_operation("rollback"));
        assert!(is_emergency_operation("emergency_stop"));
        assert!(is_emergency_operation("cluster_recovery"));
        assert!(is_emergency_operation("force_restart"));
        assert!(is_emergency_operation("data_restore"));
        assert!(is_emergency_operation("emergency_custom_thing"));
        assert!(!is_emergency_operation("file_read"));
        assert!(!is_emergency_operation("shell"));
        assert!(!is_emergency_operation("stripe"));
    }

    // ── ApprovalTier display ─────────────────────────────────────────

    #[test]
    fn test_tier_display() {
        assert_eq!(format!("{}", ApprovalTier::Auto), "Auto");
        assert_eq!(format!("{}", ApprovalTier::Single), "Single");
        assert_eq!(format!("{}", ApprovalTier::Multi), "Multi");
        assert_eq!(format!("{}", ApprovalTier::Emergency), "Emergency");
    }

    // ── ApprovalPolicy defaults ──────────────────────────────────────

    #[test]
    fn test_policy_defaults() {
        let auto = ApprovalPolicy::auto();
        assert_eq!(auto.required_approvers, 0);
        assert_eq!(auto.timeout_secs, 0);

        let single = ApprovalPolicy::single();
        assert_eq!(single.required_approvers, 1);
        assert_eq!(single.timeout_secs, 300);
        assert!(single.auto_deny_on_timeout);

        let multi = ApprovalPolicy::multi();
        assert_eq!(multi.required_approvers, 2);
        assert_eq!(multi.timeout_secs, 600);
        assert!(multi.auto_deny_on_timeout);

        let emergency = ApprovalPolicy::emergency();
        assert_eq!(emergency.required_approvers, 0);
        assert!(!emergency.auto_deny_on_timeout);
    }

    #[test]
    fn test_policy_for_tier() {
        let p = ApprovalPolicy::for_tier(&ApprovalTier::Auto);
        assert_eq!(p.required_approvers, 0);

        let p = ApprovalPolicy::for_tier(&ApprovalTier::Multi);
        assert_eq!(p.required_approvers, 2);
    }

    // ── ApprovalConfig defaults ──────────────────────────────────────

    #[test]
    fn test_default_config() {
        let config = ApprovalConfig::default();
        assert!(config.enabled);
        assert!(config.tiered_enabled);
        assert_eq!(config.timeout_secs, 300);
        assert!(config.tools_requiring_approval.contains(&"email".to_string()));
    }

    // ── ApprovalGate requires_approval ───────────────────────────────

    #[test]
    fn test_requires_approval_tiered() {
        let gate = ApprovalGate::new(ApprovalConfig::default());
        // Auto tier tools do NOT require approval
        assert!(!gate.requires_approval("file_read"));
        assert!(!gate.requires_approval("glob"));
        assert!(!gate.requires_approval("web_search"));
        // Single tier tools DO require approval
        assert!(gate.requires_approval("file_write"));
        assert!(gate.requires_approval("shell"));
        assert!(gate.requires_approval("email"));
        // Multi tier tools DO require approval
        assert!(gate.requires_approval("stripe"));
        assert!(gate.requires_approval("render_deploy"));
    }

    #[test]
    fn test_requires_approval_disabled() {
        let gate = ApprovalGate::new(ApprovalConfig {
            enabled: false,
            ..Default::default()
        });
        assert!(!gate.requires_approval("email"));
        assert!(!gate.requires_approval("stripe"));
    }

    #[test]
    fn test_requires_approval_legacy_mode() {
        let gate = ApprovalGate::new(ApprovalConfig {
            tiered_enabled: false,
            ..Default::default()
        });
        // Legacy: only tools in the list require approval
        assert!(gate.requires_approval("email"));
        assert!(gate.requires_approval("http_request"));
        assert!(!gate.requires_approval("shell"));
        assert!(!gate.requires_approval("stripe"));
    }

    // ── get_tier ─────────────────────────────────────────────────────

    #[test]
    fn test_get_tier_tiered() {
        let gate = ApprovalGate::new(ApprovalConfig::default());
        assert_eq!(gate.get_tier("file_read", None), ApprovalTier::Auto);
        assert_eq!(gate.get_tier("shell", None), ApprovalTier::Single);
        assert_eq!(gate.get_tier("stripe", None), ApprovalTier::Multi);
    }

    #[test]
    fn test_get_tier_with_http_args() {
        let gate = ApprovalGate::new(ApprovalConfig::default());
        let get_args = serde_json::json!({"method": "GET"});
        assert_eq!(gate.get_tier("http_request", Some(&get_args)), ApprovalTier::Auto);

        let post_args = serde_json::json!({"method": "POST"});
        assert_eq!(gate.get_tier("http_request", Some(&post_args)), ApprovalTier::Single);
    }

    #[test]
    fn test_get_tier_legacy_mode() {
        let gate = ApprovalGate::new(ApprovalConfig {
            tiered_enabled: false,
            ..Default::default()
        });
        // Legacy: email is in the list => Single
        assert_eq!(gate.get_tier("email", None), ApprovalTier::Single);
        // Legacy: shell is NOT in the list => Auto
        assert_eq!(gate.get_tier("shell", None), ApprovalTier::Auto);
    }

    #[test]
    fn test_get_tier_disabled() {
        let gate = ApprovalGate::new(ApprovalConfig {
            enabled: false,
            ..Default::default()
        });
        // Everything is Auto when disabled
        assert_eq!(gate.get_tier("stripe", None), ApprovalTier::Auto);
        assert_eq!(gate.get_tier("shell", None), ApprovalTier::Auto);
    }

    // ── Policy override from config ──────────────────────────────────

    #[test]
    fn test_policy_override() {
        let mut tier_policies = HashMap::new();
        tier_policies.insert("multi".to_string(), TierPolicyConfig {
            required_approvers: Some(3),
            timeout_secs: Some(900),
            auto_deny_on_timeout: Some(false),
        });
        let gate = ApprovalGate::new(ApprovalConfig {
            tier_policies,
            ..Default::default()
        });

        let policy = gate.policy_for_tier(&ApprovalTier::Multi);
        assert_eq!(policy.required_approvers, 3);
        assert_eq!(policy.timeout_secs, 900);
        assert!(!policy.auto_deny_on_timeout);

        // Non-overridden tiers keep defaults
        let single_policy = gate.policy_for_tier(&ApprovalTier::Single);
        assert_eq!(single_policy.required_approvers, 1);
        assert_eq!(single_policy.timeout_secs, 300);
    }

    // ── Async approval flow tests ────────────────────────────────────

    #[tokio::test]
    async fn test_auto_tier_immediate_approval() {
        let gate = ApprovalGate::new(ApprovalConfig::default());
        let (_, result) = gate.check_and_request("file_read", "Read config file", None).await;
        assert_eq!(result, ApprovalResult::Approved);
    }

    #[tokio::test]
    async fn test_auto_tier_http_get() {
        let gate = ApprovalGate::new(ApprovalConfig::default());
        let args = serde_json::json!({"url": "https://example.com", "method": "GET"});
        let (_, result) = gate.check_and_request("http_request", "Fetch data", Some(&args)).await;
        assert_eq!(result, ApprovalResult::Approved);
    }

    #[tokio::test]
    async fn test_single_approval_approved() {
        let gate = Arc::new(ApprovalGate::new(ApprovalConfig {
            tiered_enabled: true,
            ..Default::default()
        }));

        let g = gate.clone();
        let handle = tokio::spawn(async move {
            g.check_and_request("shell", "Run ls -la", None).await
        });

        // Wait for request to be registered
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Find and approve
        let ids = gate.pending_ids().await;
        assert_eq!(ids.len(), 1);
        gate.respond(&ids[0], true).await;

        let (_, result) = handle.await.unwrap();
        assert_eq!(result, ApprovalResult::Approved);
    }

    #[tokio::test]
    async fn test_single_approval_denied() {
        let gate = Arc::new(ApprovalGate::new(ApprovalConfig::default()));

        let g = gate.clone();
        let handle = tokio::spawn(async move {
            g.check_and_request("file_write", "Write to /etc/passwd", None).await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let ids = gate.pending_ids().await;
        assert_eq!(ids.len(), 1);
        gate.respond(&ids[0], false).await;

        let (_, result) = handle.await.unwrap();
        assert_eq!(result, ApprovalResult::Denied);
    }

    #[tokio::test]
    async fn test_single_approval_timeout_auto_deny() {
        // Use a very short timeout for the single tier
        let mut tier_policies = HashMap::new();
        tier_policies.insert("single".to_string(), TierPolicyConfig {
            required_approvers: None,
            timeout_secs: Some(1),
            auto_deny_on_timeout: Some(true),
        });
        let gate = ApprovalGate::new(ApprovalConfig {
            tier_policies,
            ..Default::default()
        });

        let (_, result) = gate.check_and_request("shell", "test timeout", None).await;
        assert_eq!(result, ApprovalResult::Denied);
    }

    #[tokio::test]
    async fn test_multi_approval_quorum() {
        let gate = Arc::new(ApprovalGate::new(ApprovalConfig::default()));

        let g = gate.clone();
        let handle = tokio::spawn(async move {
            g.check_and_request("stripe", "Charge $100 to customer", None).await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let ids = gate.pending_ids().await;
        assert_eq!(ids.len(), 1);

        // First approval -- not enough yet
        let found = gate.respond(&ids[0], true).await;
        assert!(found);

        // Check status: 1/2
        let status = gate.multi_approval_status(&ids[0]).await;
        assert_eq!(status, Some((1, 2)));

        // Second approval -- quorum reached
        gate.respond(&ids[0], true).await;

        let (_, result) = handle.await.unwrap();
        assert_eq!(result, ApprovalResult::Approved);
    }

    #[tokio::test]
    async fn test_multi_approval_denied_by_single_deny() {
        let gate = Arc::new(ApprovalGate::new(ApprovalConfig::default()));

        let g = gate.clone();
        let handle = tokio::spawn(async move {
            g.check_and_request("render_deploy", "Deploy to production", None).await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let ids = gate.pending_ids().await;
        assert_eq!(ids.len(), 1);

        // First vote: approve (1/2)
        gate.respond(&ids[0], true).await;

        // Second vote: deny -- immediately kills the request
        gate.respond(&ids[0], false).await;

        let (_, result) = handle.await.unwrap();
        assert_eq!(result, ApprovalResult::Denied);
    }

    #[tokio::test]
    async fn test_multi_approval_timeout() {
        let mut tier_policies = HashMap::new();
        tier_policies.insert("multi".to_string(), TierPolicyConfig {
            required_approvers: Some(2),
            timeout_secs: Some(1),
            auto_deny_on_timeout: Some(true),
        });
        let gate = ApprovalGate::new(ApprovalConfig {
            tier_policies,
            ..Default::default()
        });

        let (_, result) = gate.check_and_request("stripe", "Charge customer", None).await;
        assert_eq!(result, ApprovalResult::Denied);
    }

    // ── Pending count tests ──────────────────────────────────────────

    #[tokio::test]
    async fn test_pending_count_both_types() {
        let gate = Arc::new(ApprovalGate::new(ApprovalConfig::default()));

        // Spawn a single approval
        let g = gate.clone();
        tokio::spawn(async move {
            g.check_and_request("shell", "Run command", None).await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Spawn a multi approval
        let g2 = gate.clone();
        tokio::spawn(async move {
            g2.check_and_request("stripe", "Charge", None).await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(gate.pending_count().await, 2);
    }

    // ── Legacy compatibility tests ───────────────────────────────────

    #[tokio::test]
    async fn test_legacy_request_method() {
        let gate = Arc::new(ApprovalGate::new(ApprovalConfig {
            tiered_enabled: false,
            timeout_secs: 5,
            ..Default::default()
        }));

        let g = gate.clone();
        let handle = tokio::spawn(async move {
            g.request("email", "Send report").await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        let ids = gate.pending_ids().await;
        assert_eq!(ids.len(), 1);
        gate.respond(&ids[0], true).await;

        let (_, result) = handle.await.unwrap();
        assert_eq!(result, ApprovalResult::Approved);
    }

    #[tokio::test]
    async fn test_legacy_approval_respond_approved() {
        let gate = ApprovalGate::new(ApprovalConfig {
            tiered_enabled: false,
            timeout_secs: 5,
            ..Default::default()
        });

        let gate_clone = Arc::new(gate);
        let g = gate_clone.clone();

        // Spawn request in background
        let handle = tokio::spawn(async move {
            g.request("email", "Send email to user@example.com").await
        });

        // Wait a bit for request to be registered
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Find the pending approval and respond
        let pending = gate_clone.pending.lock().await;
        let id = pending.keys().next().cloned().unwrap();
        drop(pending);

        gate_clone.respond(&id, true).await;

        let (_, result) = handle.await.unwrap();
        assert_eq!(result, ApprovalResult::Approved);
    }

    #[tokio::test]
    async fn test_legacy_approval_respond_denied() {
        let gate = Arc::new(ApprovalGate::new(ApprovalConfig {
            tiered_enabled: false,
            timeout_secs: 5,
            ..Default::default()
        }));

        let g = gate.clone();
        let handle = tokio::spawn(async move {
            g.request("email", "Send spam").await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let pending = gate.pending.lock().await;
        let id = pending.keys().next().cloned().unwrap();
        drop(pending);

        gate.respond(&id, false).await;

        let (_, result) = handle.await.unwrap();
        assert_eq!(result, ApprovalResult::Denied);
    }

    #[tokio::test]
    async fn test_legacy_approval_timeout() {
        let gate = ApprovalGate::new(ApprovalConfig {
            tiered_enabled: false,
            timeout_secs: 1, // 1 second timeout for test
            ..Default::default()
        });

        let (_, result) = gate.request("email", "test").await;
        // Legacy uses auto_deny_on_timeout=false — timeout returns Timeout, not Denied
        assert_eq!(result, ApprovalResult::Timeout);
    }

    #[tokio::test]
    async fn test_respond_unknown_id() {
        let gate = ApprovalGate::new(ApprovalConfig::default());
        assert!(!gate.respond("nonexistent_id", true).await);
    }

    // ── Format message tests ─────────────────────────────────────────

    #[test]
    fn test_format_tiered_message_single() {
        let gate = ApprovalGate::new(ApprovalConfig::default());
        let policy = ApprovalPolicy::single();
        let msg = gate.format_tiered_message("abc123", "shell", "Run ls", &ApprovalTier::Single, &policy);
        assert!(msg.contains("SINGLE APPROVAL"));
        assert!(msg.contains("shell"));
        assert!(msg.contains("Run ls"));
        assert!(msg.contains("/approve abc123"));
        assert!(msg.contains("/deny abc123"));
        assert!(msg.contains("300 seconds"));
    }

    #[test]
    fn test_format_tiered_message_multi() {
        let gate = ApprovalGate::new(ApprovalConfig::default());
        let policy = ApprovalPolicy::multi();
        let msg = gate.format_tiered_message("xyz789", "stripe", "Charge", &ApprovalTier::Multi, &policy);
        assert!(msg.contains("MULTI APPROVAL"));
        assert!(msg.contains("need 2/2"));
        assert!(msg.contains("stripe"));
        assert!(msg.contains("600 seconds"));
    }

    #[test]
    fn test_format_legacy_message() {
        let gate = ApprovalGate::new(ApprovalConfig::default());
        let msg = gate.format_approval_message("abc123", "email", "Send report");
        assert!(msg.contains("email"));
        assert!(msg.contains("Send report"));
        assert!(msg.contains("/approve abc123"));
        assert!(msg.contains("/deny abc123"));
    }
}
