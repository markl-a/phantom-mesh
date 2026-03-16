//! Approval gate — requires human confirmation via Telegram for risky actions.
//! Sends a message to the user and waits for their Yes/No response.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};
use tokio::time::{timeout, Duration};
use tracing::{info, warn};

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
    /// Tools that require approval before execution
    #[serde(default = "default_approval_tools")]
    pub tools_requiring_approval: Vec<String>,
    /// Timeout in seconds for waiting for approval (default: 300 = 5 min)
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Whether approval gate is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
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
        }
    }
}

/// Pending approval request
struct PendingApproval {
    tx: oneshot::Sender<bool>,
}

/// Async function type for sending approval notifications (e.g., via Telegram).
/// Takes (chat_id, message) and sends it. chat_id may be empty if broadcast.
pub type ApprovalNotifier = Arc<dyn Fn(String) -> Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

/// Approval gate — manages pending approval requests from agents.
/// When an agent tries to use a tool that requires approval, it:
/// 1. Sends a message to the user via Telegram
/// 2. Waits for the user to respond (Yes/No)
/// 3. Returns Approved/Denied/Timeout
pub struct ApprovalGate {
    config: ApprovalConfig,
    /// Pending approvals: approval_id -> sender
    pending: Arc<Mutex<HashMap<String, PendingApproval>>>,
    /// Optional notifier to send approval messages (e.g., Telegram)
    notifier: tokio::sync::RwLock<Option<ApprovalNotifier>>,
}

impl ApprovalGate {
    pub fn new(config: ApprovalConfig) -> Self {
        Self {
            config,
            pending: Arc::new(Mutex::new(HashMap::new())),
            notifier: tokio::sync::RwLock::new(None),
        }
    }

    /// Set the notifier callback for sending approval request messages.
    pub async fn set_notifier(&self, notifier: ApprovalNotifier) {
        *self.notifier.write().await = Some(notifier);
    }

    /// Check if a tool requires approval
    pub fn requires_approval(&self, tool_name: &str) -> bool {
        if !self.config.enabled {
            return false;
        }
        self.config.tools_requiring_approval.iter().any(|t| t == tool_name)
    }

    /// Request approval and wait for response.
    /// Returns the approval ID that should be sent to the user.
    /// Call `respond()` when the user replies.
    pub async fn request(&self, tool_name: &str, description: &str) -> (String, ApprovalResult) {
        let approval_id = format!("approval_{}", uuid::Uuid::new_v4().to_string().replace('-', "")[..8].to_string());

        let (tx, rx) = oneshot::channel::<bool>();

        {
            let mut pending = self.pending.lock().await;
            pending.insert(approval_id.clone(), PendingApproval { tx });
        }

        info!("Approval requested: {} for tool '{}': {}", approval_id, tool_name, description);

        // Send notification via Telegram (or other channel) if notifier is set
        {
            let notifier = self.notifier.read().await;
            if let Some(ref notify_fn) = *notifier {
                let msg = self.format_approval_message(&approval_id, tool_name, description);
                notify_fn(msg).await;
            } else {
                warn!("No notifier set for ApprovalGate — user won't see approval request {}", approval_id);
            }
        }

        // Wait for response with timeout
        let timeout_duration = Duration::from_secs(self.config.timeout_secs);
        match timeout(timeout_duration, rx).await {
            Ok(Ok(true)) => {
                info!("Approval {} GRANTED", approval_id);
                (approval_id, ApprovalResult::Approved)
            }
            Ok(Ok(false)) => {
                info!("Approval {} DENIED", approval_id);
                (approval_id, ApprovalResult::Denied)
            }
            Ok(Err(_)) => {
                warn!("Approval {} channel dropped", approval_id);
                (approval_id, ApprovalResult::Timeout)
            }
            Err(_) => {
                // Timeout — clean up
                let mut pending = self.pending.lock().await;
                pending.remove(&approval_id);
                warn!("Approval {} TIMED OUT", approval_id);
                (approval_id, ApprovalResult::Timeout)
            }
        }
    }

    /// Respond to a pending approval
    pub async fn respond(&self, approval_id: &str, approved: bool) -> bool {
        let mut pending = self.pending.lock().await;
        if let Some(approval) = pending.remove(approval_id) {
            let _ = approval.tx.send(approved);
            true
        } else {
            false
        }
    }

    /// Get the formatted approval message for Telegram
    pub fn format_approval_message(&self, approval_id: &str, tool_name: &str, description: &str) -> String {
        format!(
            "Approval Required\n\n\
             Tool: {}\n\
             Action: {}\n\n\
             Reply with:\n\
             /approve {} — to allow\n\
             /deny {} — to deny\n\n\
             (Auto-denies in {} seconds)",
            tool_name, description, approval_id, approval_id, self.config.timeout_secs
        )
    }

    /// List pending approval IDs
    pub async fn pending_count(&self) -> usize {
        self.pending.lock().await.len()
    }

    /// Get all pending approval IDs
    pub async fn pending_ids(&self) -> Vec<String> {
        self.pending.lock().await.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ApprovalConfig::default();
        assert!(config.enabled);
        assert_eq!(config.timeout_secs, 300);
        assert!(config.tools_requiring_approval.contains(&"email".to_string()));
    }

    #[test]
    fn test_requires_approval() {
        let gate = ApprovalGate::new(ApprovalConfig::default());
        assert!(gate.requires_approval("email"));
        assert!(gate.requires_approval("http_request"));
        assert!(!gate.requires_approval("shell"));
        assert!(!gate.requires_approval("file_read"));
    }

    #[test]
    fn test_requires_approval_disabled() {
        let gate = ApprovalGate::new(ApprovalConfig {
            enabled: false,
            ..Default::default()
        });
        assert!(!gate.requires_approval("email"));
    }

    #[tokio::test]
    async fn test_approval_respond_approved() {
        let gate = ApprovalGate::new(ApprovalConfig {
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
    async fn test_approval_respond_denied() {
        let gate = Arc::new(ApprovalGate::new(ApprovalConfig {
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
    async fn test_approval_timeout() {
        let gate = ApprovalGate::new(ApprovalConfig {
            timeout_secs: 1, // 1 second timeout for test
            ..Default::default()
        });

        let (_, result) = gate.request("email", "test").await;
        assert_eq!(result, ApprovalResult::Timeout);
    }

    #[test]
    fn test_format_message() {
        let gate = ApprovalGate::new(ApprovalConfig::default());
        let msg = gate.format_approval_message("abc123", "email", "Send report");
        assert!(msg.contains("email"));
        assert!(msg.contains("Send report"));
        assert!(msg.contains("/approve abc123"));
        assert!(msg.contains("/deny abc123"));
    }
}
