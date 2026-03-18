//! Security Hardening — secret rotation, RBAC enforcement, and security audit summaries.
//!
//! Provides three main components:
//! 1. **SecretRotator** — tracks secret ages and triggers rotation when expired
//! 2. **RbacEnforcer** — tool-level permission checks with default role policies
//! 3. **SecurityAuditor** — generates security reports with findings and severity levels

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

// ── Secret Rotation ──────────────────────────────────────────────────────────

/// Metadata about a managed secret.
#[derive(Debug, Clone)]
struct SecretEntry {
    /// When the secret was last rotated (or first created).
    last_rotated: SystemTime,
    /// The current encrypted value.
    encrypted_value: String,
}

/// Manages secret rotation schedules and re-encryption.
///
/// Tracks per-key rotation timestamps and provides helpers to check
/// whether a secret has exceeded its maximum age.
pub struct SecretRotator {
    /// key_name -> SecretEntry
    secrets: HashMap<String, SecretEntry>,
    /// Default rotation interval in days (applied when no per-key override).
    default_rotation_days: u32,
}

impl SecretRotator {
    /// Create a new rotator with the given default rotation period.
    pub fn new(default_rotation_days: u32) -> Self {
        Self {
            secrets: HashMap::new(),
            default_rotation_days,
        }
    }

    /// Register a secret with its current encrypted value and rotation timestamp.
    pub fn register_secret(&mut self, key_name: &str, encrypted_value: &str) {
        self.secrets.insert(
            key_name.to_string(),
            SecretEntry {
                last_rotated: SystemTime::now(),
                encrypted_value: encrypted_value.to_string(),
            },
        );
        debug!("SecretRotator: registered secret '{}'", key_name);
    }

    /// Register a secret with an explicit last-rotated timestamp (useful for loading state).
    pub fn register_secret_with_time(
        &mut self,
        key_name: &str,
        encrypted_value: &str,
        last_rotated: SystemTime,
    ) {
        self.secrets.insert(
            key_name.to_string(),
            SecretEntry {
                last_rotated,
                encrypted_value: encrypted_value.to_string(),
            },
        );
    }

    /// Check whether a secret needs rotation given a maximum age in days.
    pub fn check_rotation_needed(&self, key_name: &str, max_age_days: u32) -> bool {
        let Some(entry) = self.secrets.get(key_name) else {
            // Unknown secret — can't determine, treat as needing rotation.
            return true;
        };
        let max_age = Duration::from_secs(max_age_days as u64 * 86_400);
        match SystemTime::now().duration_since(entry.last_rotated) {
            Ok(elapsed) => elapsed >= max_age,
            Err(_) => false, // Clock skew — treat as fresh
        }
    }

    /// Check rotation using the default rotation period.
    pub fn needs_rotation(&self, key_name: &str) -> bool {
        self.check_rotation_needed(key_name, self.default_rotation_days)
    }

    /// Rotate a secret: generate a new random value, encrypt it with the
    /// provided encryption closure, and update the internal record.
    ///
    /// The `encrypt_fn` takes a plaintext string and returns the encrypted form.
    pub fn rotate_secret<F>(&mut self, key_name: &str, encrypt_fn: F) -> Result<String>
    where
        F: Fn(&str) -> Result<String>,
    {
        // Generate a new random plaintext secret (64 hex chars = 32 bytes of entropy).
        let mut random_bytes = [0u8; 32];
        getrandom(&mut random_bytes);
        let new_plaintext = hex::encode(random_bytes);

        let encrypted = encrypt_fn(&new_plaintext)?;

        self.secrets.insert(
            key_name.to_string(),
            SecretEntry {
                last_rotated: SystemTime::now(),
                encrypted_value: encrypted.clone(),
            },
        );

        debug!("SecretRotator: rotated secret '{}'", key_name);
        Ok(encrypted)
    }

    /// Get the current encrypted value of a secret.
    pub fn get_encrypted(&self, key_name: &str) -> Option<&str> {
        self.secrets.get(key_name).map(|e| e.encrypted_value.as_str())
    }

    /// Get the last rotation time for a secret.
    pub fn last_rotated(&self, key_name: &str) -> Option<SystemTime> {
        self.secrets.get(key_name).map(|e| e.last_rotated)
    }

    /// List all registered secret names.
    pub fn list_secrets(&self) -> Vec<String> {
        let mut names: Vec<_> = self.secrets.keys().cloned().collect();
        names.sort();
        names
    }

    /// Return the count of secrets that currently need rotation.
    pub fn expired_count(&self) -> usize {
        self.secrets
            .keys()
            .filter(|k| self.needs_rotation(k))
            .count()
    }

    /// Remove a secret from tracking.
    pub fn remove_secret(&mut self, key_name: &str) -> bool {
        self.secrets.remove(key_name).is_some()
    }
}

/// Simple cross-platform random fill (uses rand crate).
fn getrandom(buf: &mut [u8]) {
    use rand::RngCore;
    rand::thread_rng().fill_bytes(buf);
}

// ── RBAC Enforcement ─────────────────────────────────────────────────────────

/// Action categories for RBAC checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    Execute,
    Read,
    Write,
    Admin,
}

impl Action {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Execute => "execute",
            Self::Read => "read",
            Self::Write => "write",
            Self::Admin => "admin",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "execute" => Some(Self::Execute),
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }
}

/// A single permission rule: which tool + action is allowed for a role.
#[derive(Debug, Clone)]
struct PermissionRule {
    tool_pattern: String, // "*" for wildcard, or exact tool name
    action: Action,
}

/// RBAC enforcer with role-based policies and tool-level permissions.
///
/// Default roles:
/// - **admin** — all tools, all actions
/// - **operator** — tools + hands execution, read/write/execute (no admin)
/// - **viewer** — read-only access
pub struct RbacEnforcer {
    /// role -> list of permission rules
    policies: HashMap<String, Vec<PermissionRule>>,
}

impl RbacEnforcer {
    /// Create an enforcer with default policies (admin, operator, viewer).
    pub fn new() -> Self {
        let mut policies = HashMap::new();

        // Admin: all tools, all actions
        policies.insert(
            "admin".to_string(),
            vec![PermissionRule {
                tool_pattern: "*".to_string(),
                action: Action::Admin, // Admin implies all lower actions
            }],
        );

        // Operator: all tools, execute + read + write (no admin)
        policies.insert(
            "operator".to_string(),
            vec![
                PermissionRule {
                    tool_pattern: "*".to_string(),
                    action: Action::Execute,
                },
                PermissionRule {
                    tool_pattern: "*".to_string(),
                    action: Action::Read,
                },
                PermissionRule {
                    tool_pattern: "*".to_string(),
                    action: Action::Write,
                },
            ],
        );

        // Viewer: all tools, read-only
        policies.insert(
            "viewer".to_string(),
            vec![PermissionRule {
                tool_pattern: "*".to_string(),
                action: Action::Read,
            }],
        );

        Self { policies }
    }

    /// Check if a role has permission to perform an action on a tool.
    pub fn check_permission(&self, role: &str, tool: &str, action: &str) -> bool {
        let Some(action_enum) = Action::from_str(action) else {
            warn!("RbacEnforcer: unknown action '{}'", action);
            return false;
        };

        let Some(rules) = self.policies.get(role) else {
            debug!("RbacEnforcer: unknown role '{}' — denied", role);
            return false;
        };

        for rule in rules {
            let tool_matches =
                rule.tool_pattern == "*" || rule.tool_pattern == tool;
            if !tool_matches {
                continue;
            }

            // Admin action implies all other actions
            if rule.action == Action::Admin {
                return true;
            }

            if rule.action == action_enum {
                return true;
            }
        }

        false
    }

    /// Add a custom role with specific tool+action permissions.
    pub fn add_role(&mut self, role: &str, permissions: Vec<(String, Action)>) {
        let rules = permissions
            .into_iter()
            .map(|(tool_pattern, action)| PermissionRule {
                tool_pattern,
                action,
            })
            .collect();
        self.policies.insert(role.to_string(), rules);
        debug!("RbacEnforcer: added role '{}'", role);
    }

    /// Grant a specific permission to an existing role.
    pub fn grant_permission(&mut self, role: &str, tool: &str, action: Action) {
        self.policies
            .entry(role.to_string())
            .or_insert_with(Vec::new)
            .push(PermissionRule {
                tool_pattern: tool.to_string(),
                action,
            });
    }

    /// Revoke all permissions for a role on a specific tool.
    pub fn revoke_tool(&mut self, role: &str, tool: &str) {
        if let Some(rules) = self.policies.get_mut(role) {
            rules.retain(|r| r.tool_pattern != tool);
        }
    }

    /// List all roles.
    pub fn list_roles(&self) -> Vec<String> {
        let mut roles: Vec<_> = self.policies.keys().cloned().collect();
        roles.sort();
        roles
    }

    /// List all permissions for a role.
    pub fn get_permissions(&self, role: &str) -> Vec<(String, String)> {
        self.policies
            .get(role)
            .map(|rules| {
                rules
                    .iter()
                    .map(|r| (r.tool_pattern.clone(), r.action.as_str().to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Remove a role entirely.
    pub fn remove_role(&mut self, role: &str) -> bool {
        self.policies.remove(role).is_some()
    }
}

impl Default for RbacEnforcer {
    fn default() -> Self {
        Self::new()
    }
}

// ── Security Audit ───────────────────────────────────────────────────────────

/// Severity level for audit findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Info => write!(f, "info"),
            Self::Low => write!(f, "low"),
            Self::Medium => write!(f, "medium"),
            Self::High => write!(f, "high"),
            Self::Critical => write!(f, "critical"),
        }
    }
}

/// A single security finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub severity: Severity,
    pub category: String,
    pub description: String,
    pub recommendation: String,
}

/// Aggregated security report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityReport {
    pub findings: Vec<Finding>,
    pub total_findings: usize,
    pub critical_count: usize,
    pub high_count: usize,
    pub medium_count: usize,
    pub low_count: usize,
    pub info_count: usize,
    pub overall_score: u8, // 0-100, higher = more secure
}

impl SecurityReport {
    fn from_findings(findings: Vec<Finding>) -> Self {
        let total_findings = findings.len();
        let critical_count = findings.iter().filter(|f| f.severity == Severity::Critical).count();
        let high_count = findings.iter().filter(|f| f.severity == Severity::High).count();
        let medium_count = findings.iter().filter(|f| f.severity == Severity::Medium).count();
        let low_count = findings.iter().filter(|f| f.severity == Severity::Low).count();
        let info_count = findings.iter().filter(|f| f.severity == Severity::Info).count();

        // Score: start at 100, deduct per finding
        let deductions = critical_count * 25 + high_count * 15 + medium_count * 8 + low_count * 3 + info_count;
        let overall_score = 100u8.saturating_sub(deductions as u8);

        Self {
            findings,
            total_findings,
            critical_count,
            high_count,
            medium_count,
            low_count,
            info_count,
            overall_score,
        }
    }

    /// True if there are no critical or high findings.
    pub fn is_passing(&self) -> bool {
        self.critical_count == 0 && self.high_count == 0
    }
}

/// Security auditor that checks the system for common security issues.
pub struct SecurityAuditor {
    /// Known secret names and whether they are encrypted (enc2: prefix).
    secret_values: Vec<(String, String)>,
    /// Tool names and whether they have preflight checks.
    tools_with_preflight: Vec<(String, bool)>,
    /// Secret rotation tracker (optional).
    rotator: Option<SecretRotator>,
    /// API keys that are registered but potentially unused.
    registered_api_keys: Vec<String>,
}

impl SecurityAuditor {
    pub fn new() -> Self {
        Self {
            secret_values: Vec::new(),
            tools_with_preflight: Vec::new(),
            rotator: None,
            registered_api_keys: Vec::new(),
        }
    }

    /// Add a secret name + value pair for auditing. The value is checked for enc2: prefix.
    pub fn add_secret(&mut self, name: &str, value: &str) {
        self.secret_values.push((name.to_string(), value.to_string()));
    }

    /// Register a tool and whether it has a preflight check.
    pub fn add_tool(&mut self, name: &str, has_preflight: bool) {
        self.tools_with_preflight.push((name.to_string(), has_preflight));
    }

    /// Attach a SecretRotator for expired-secret checks.
    pub fn set_rotator(&mut self, rotator: SecretRotator) {
        self.rotator = Some(rotator);
    }

    /// Register an API key name as "registered" (for unused-key detection).
    pub fn add_api_key(&mut self, key_name: &str) {
        self.registered_api_keys.push(key_name.to_string());
    }

    /// Mark an API key as used (remove from the unused list).
    pub fn mark_api_key_used(&mut self, key_name: &str) {
        self.registered_api_keys.retain(|k| k != key_name);
    }

    /// Generate a full security audit report.
    pub fn audit_report(&self) -> SecurityReport {
        let mut findings = Vec::new();

        // 1. Check for unencrypted secrets
        for (name, value) in &self.secret_values {
            if !value.starts_with("enc2:") && !value.is_empty() {
                findings.push(Finding {
                    severity: Severity::Critical,
                    category: "unencrypted_secret".to_string(),
                    description: format!("Secret '{}' is stored in plaintext", name),
                    recommendation: format!(
                        "Encrypt '{}' using SecretManager.encrypt() and store with enc2: prefix",
                        name
                    ),
                });
            }
        }

        // 2. Check for expired secrets (via rotator)
        if let Some(rotator) = &self.rotator {
            for key_name in rotator.list_secrets() {
                if rotator.needs_rotation(&key_name) {
                    findings.push(Finding {
                        severity: Severity::High,
                        category: "expired_secret".to_string(),
                        description: format!(
                            "Secret '{}' has exceeded its rotation period",
                            key_name
                        ),
                        recommendation: format!(
                            "Rotate '{}' immediately using SecretRotator.rotate_secret()",
                            key_name
                        ),
                    });
                }
            }
        }

        // 3. Check for tools without preflight
        for (tool_name, has_preflight) in &self.tools_with_preflight {
            if !has_preflight {
                findings.push(Finding {
                    severity: Severity::Medium,
                    category: "missing_preflight".to_string(),
                    description: format!(
                        "Tool '{}' does not implement a preflight check",
                        tool_name
                    ),
                    recommendation: format!(
                        "Add a preflight() method to '{}' to validate inputs before execution",
                        tool_name
                    ),
                });
            }
        }

        // 4. Check for unused API keys
        for key_name in &self.registered_api_keys {
            findings.push(Finding {
                severity: Severity::Low,
                category: "unused_api_key".to_string(),
                description: format!("API key '{}' is registered but appears unused", key_name),
                recommendation: format!(
                    "Remove or rotate '{}' if it is no longer needed",
                    key_name
                ),
            });
        }

        // 5. Check for empty secrets
        for (name, value) in &self.secret_values {
            if value.is_empty() {
                findings.push(Finding {
                    severity: Severity::Medium,
                    category: "empty_secret".to_string(),
                    description: format!("Secret '{}' has an empty value", name),
                    recommendation: format!(
                        "Set a valid value for '{}' or remove it from the configuration",
                        name
                    ),
                });
            }
        }

        // Sort by severity (Critical first)
        findings.sort_by(|a, b| b.severity.cmp(&a.severity));

        SecurityReport::from_findings(findings)
    }
}

impl Default for SecurityAuditor {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── SecretRotator tests ──────────────────────────────────────────────

    #[test]
    fn test_rotator_new() {
        let rotator = SecretRotator::new(90);
        assert_eq!(rotator.default_rotation_days, 90);
        assert!(rotator.list_secrets().is_empty());
    }

    #[test]
    fn test_register_secret() {
        let mut rotator = SecretRotator::new(90);
        rotator.register_secret("api_key", "enc2:abc123");
        assert_eq!(rotator.list_secrets(), vec!["api_key"]);
        assert_eq!(rotator.get_encrypted("api_key"), Some("enc2:abc123"));
    }

    #[test]
    fn test_fresh_secret_not_expired() {
        let mut rotator = SecretRotator::new(90);
        rotator.register_secret("api_key", "enc2:abc123");
        assert!(!rotator.check_rotation_needed("api_key", 90));
        assert!(!rotator.needs_rotation("api_key"));
    }

    #[test]
    fn test_old_secret_is_expired() {
        let mut rotator = SecretRotator::new(1); // 1-day rotation
        let old_time = SystemTime::now() - Duration::from_secs(2 * 86_400); // 2 days ago
        rotator.register_secret_with_time("api_key", "enc2:abc123", old_time);
        assert!(rotator.check_rotation_needed("api_key", 1));
        assert!(rotator.needs_rotation("api_key"));
    }

    #[test]
    fn test_unknown_secret_needs_rotation() {
        let rotator = SecretRotator::new(90);
        assert!(rotator.check_rotation_needed("nonexistent", 90));
    }

    #[test]
    fn test_rotate_secret() {
        let mut rotator = SecretRotator::new(90);
        rotator.register_secret("api_key", "enc2:old_value");

        let new_encrypted = rotator
            .rotate_secret("api_key", |plaintext| {
                assert!(!plaintext.is_empty());
                assert_eq!(plaintext.len(), 64); // 32 bytes = 64 hex chars
                Ok(format!("enc2:{}", plaintext))
            })
            .unwrap();

        assert!(new_encrypted.starts_with("enc2:"));
        assert_ne!(rotator.get_encrypted("api_key"), Some("enc2:old_value"));
        assert!(!rotator.needs_rotation("api_key")); // Just rotated
    }

    #[test]
    fn test_rotate_secret_encrypt_failure() {
        let mut rotator = SecretRotator::new(90);
        rotator.register_secret("api_key", "enc2:old_value");

        let result = rotator.rotate_secret("api_key", |_| Err(anyhow!("encryption failed")));
        assert!(result.is_err());
        // Original value should remain unchanged since rotation failed
        assert_eq!(rotator.get_encrypted("api_key"), Some("enc2:old_value"));
    }

    #[test]
    fn test_last_rotated() {
        let mut rotator = SecretRotator::new(90);
        let before = SystemTime::now();
        rotator.register_secret("api_key", "enc2:abc123");
        let after = SystemTime::now();

        let last = rotator.last_rotated("api_key").unwrap();
        assert!(last >= before);
        assert!(last <= after);
    }

    #[test]
    fn test_last_rotated_unknown() {
        let rotator = SecretRotator::new(90);
        assert!(rotator.last_rotated("nonexistent").is_none());
    }

    #[test]
    fn test_remove_secret() {
        let mut rotator = SecretRotator::new(90);
        rotator.register_secret("key1", "enc2:a");
        rotator.register_secret("key2", "enc2:b");
        assert!(rotator.remove_secret("key1"));
        assert!(!rotator.remove_secret("key1")); // Already removed
        assert_eq!(rotator.list_secrets(), vec!["key2"]);
    }

    #[test]
    fn test_expired_count() {
        let mut rotator = SecretRotator::new(1); // 1-day rotation
        let old_time = SystemTime::now() - Duration::from_secs(2 * 86_400);
        rotator.register_secret("fresh", "enc2:a");
        rotator.register_secret_with_time("expired1", "enc2:b", old_time);
        rotator.register_secret_with_time("expired2", "enc2:c", old_time);
        assert_eq!(rotator.expired_count(), 2);
    }

    // ── RbacEnforcer tests ───────────────────────────────────────────────

    #[test]
    fn test_rbac_default_roles() {
        let enforcer = RbacEnforcer::new();
        let roles = enforcer.list_roles();
        assert!(roles.contains(&"admin".to_string()));
        assert!(roles.contains(&"operator".to_string()));
        assert!(roles.contains(&"viewer".to_string()));
    }

    #[test]
    fn test_admin_can_do_everything() {
        let enforcer = RbacEnforcer::new();
        assert!(enforcer.check_permission("admin", "shell", "execute"));
        assert!(enforcer.check_permission("admin", "shell", "read"));
        assert!(enforcer.check_permission("admin", "shell", "write"));
        assert!(enforcer.check_permission("admin", "shell", "admin"));
        assert!(enforcer.check_permission("admin", "file_read", "read"));
        assert!(enforcer.check_permission("admin", "twitter", "execute"));
    }

    #[test]
    fn test_operator_can_execute_and_read() {
        let enforcer = RbacEnforcer::new();
        assert!(enforcer.check_permission("operator", "shell", "execute"));
        assert!(enforcer.check_permission("operator", "shell", "read"));
        assert!(enforcer.check_permission("operator", "shell", "write"));
        assert!(!enforcer.check_permission("operator", "shell", "admin"));
    }

    #[test]
    fn test_viewer_read_only() {
        let enforcer = RbacEnforcer::new();
        assert!(enforcer.check_permission("viewer", "file_read", "read"));
        assert!(!enforcer.check_permission("viewer", "file_read", "write"));
        assert!(!enforcer.check_permission("viewer", "shell", "execute"));
        assert!(!enforcer.check_permission("viewer", "shell", "admin"));
    }

    #[test]
    fn test_unknown_role_denied() {
        let enforcer = RbacEnforcer::new();
        assert!(!enforcer.check_permission("hacker", "shell", "execute"));
    }

    #[test]
    fn test_unknown_action_denied() {
        let enforcer = RbacEnforcer::new();
        assert!(!enforcer.check_permission("admin", "shell", "destroy"));
    }

    #[test]
    fn test_add_custom_role() {
        let mut enforcer = RbacEnforcer::new();
        enforcer.add_role(
            "developer",
            vec![
                ("shell".to_string(), Action::Execute),
                ("file_read".to_string(), Action::Read),
                ("file_write".to_string(), Action::Write),
            ],
        );
        assert!(enforcer.check_permission("developer", "shell", "execute"));
        assert!(enforcer.check_permission("developer", "file_read", "read"));
        assert!(!enforcer.check_permission("developer", "twitter", "execute"));
    }

    #[test]
    fn test_grant_permission() {
        let mut enforcer = RbacEnforcer::new();
        enforcer.grant_permission("viewer", "web_search", Action::Execute);
        assert!(enforcer.check_permission("viewer", "web_search", "execute"));
        // Existing read permission still works
        assert!(enforcer.check_permission("viewer", "web_search", "read"));
    }

    #[test]
    fn test_revoke_tool() {
        let mut enforcer = RbacEnforcer::new();
        enforcer.add_role(
            "limited",
            vec![
                ("shell".to_string(), Action::Execute),
                ("file_read".to_string(), Action::Read),
            ],
        );
        enforcer.revoke_tool("limited", "shell");
        assert!(!enforcer.check_permission("limited", "shell", "execute"));
        assert!(enforcer.check_permission("limited", "file_read", "read"));
    }

    #[test]
    fn test_remove_role() {
        let mut enforcer = RbacEnforcer::new();
        enforcer.add_role("temp", vec![("*".to_string(), Action::Read)]);
        assert!(enforcer.remove_role("temp"));
        assert!(!enforcer.remove_role("temp")); // Already removed
        assert!(!enforcer.check_permission("temp", "anything", "read"));
    }

    #[test]
    fn test_get_permissions() {
        let enforcer = RbacEnforcer::new();
        let perms = enforcer.get_permissions("viewer");
        assert_eq!(perms.len(), 1);
        assert_eq!(perms[0], ("*".to_string(), "read".to_string()));
    }

    #[test]
    fn test_get_permissions_unknown_role() {
        let enforcer = RbacEnforcer::new();
        let perms = enforcer.get_permissions("nonexistent");
        assert!(perms.is_empty());
    }

    // ── SecurityAuditor tests ────────────────────────────────────────────

    #[test]
    fn test_auditor_clean_report() {
        let mut auditor = SecurityAuditor::new();
        auditor.add_secret("api_key", "enc2:encrypted_value");
        auditor.add_tool("file_read", true);

        let report = auditor.audit_report();
        assert_eq!(report.total_findings, 0);
        assert_eq!(report.overall_score, 100);
        assert!(report.is_passing());
    }

    #[test]
    fn test_auditor_unencrypted_secret() {
        let mut auditor = SecurityAuditor::new();
        auditor.add_secret("api_key", "sk-plaintext-1234567890");

        let report = auditor.audit_report();
        assert_eq!(report.critical_count, 1);
        assert_eq!(report.findings[0].category, "unencrypted_secret");
        assert_eq!(report.findings[0].severity, Severity::Critical);
        assert!(!report.is_passing());
    }

    #[test]
    fn test_auditor_empty_secret() {
        let mut auditor = SecurityAuditor::new();
        auditor.add_secret("api_key", "");

        let report = auditor.audit_report();
        assert_eq!(report.medium_count, 1);
        assert_eq!(report.findings[0].category, "empty_secret");
    }

    #[test]
    fn test_auditor_missing_preflight() {
        let mut auditor = SecurityAuditor::new();
        auditor.add_tool("shell", true);
        auditor.add_tool("custom_tool", false);

        let report = auditor.audit_report();
        assert_eq!(report.medium_count, 1);
        assert_eq!(report.findings[0].category, "missing_preflight");
    }

    #[test]
    fn test_auditor_expired_secret() {
        let mut rotator = SecretRotator::new(1); // 1-day rotation
        let old_time = SystemTime::now() - Duration::from_secs(2 * 86_400);
        rotator.register_secret_with_time("old_key", "enc2:old", old_time);

        let mut auditor = SecurityAuditor::new();
        auditor.set_rotator(rotator);

        let report = auditor.audit_report();
        assert_eq!(report.high_count, 1);
        assert_eq!(report.findings[0].category, "expired_secret");
        assert!(!report.is_passing());
    }

    #[test]
    fn test_auditor_unused_api_key() {
        let mut auditor = SecurityAuditor::new();
        auditor.add_api_key("unused_gemini_key");

        let report = auditor.audit_report();
        assert_eq!(report.low_count, 1);
        assert_eq!(report.findings[0].category, "unused_api_key");
    }

    #[test]
    fn test_auditor_mark_key_used() {
        let mut auditor = SecurityAuditor::new();
        auditor.add_api_key("gemini_key");
        auditor.mark_api_key_used("gemini_key");

        let report = auditor.audit_report();
        assert_eq!(report.total_findings, 0);
    }

    #[test]
    fn test_auditor_multiple_findings() {
        let mut rotator = SecretRotator::new(1);
        let old_time = SystemTime::now() - Duration::from_secs(2 * 86_400);
        rotator.register_secret_with_time("expired_key", "enc2:old", old_time);

        let mut auditor = SecurityAuditor::new();
        auditor.add_secret("plaintext_key", "my-secret-value");
        auditor.add_tool("custom_tool", false);
        auditor.add_api_key("unused_key");
        auditor.set_rotator(rotator);

        let report = auditor.audit_report();
        assert_eq!(report.total_findings, 4);
        assert_eq!(report.critical_count, 1); // unencrypted
        assert_eq!(report.high_count, 1); // expired
        assert_eq!(report.medium_count, 1); // missing preflight
        assert_eq!(report.low_count, 1); // unused key
        assert!(!report.is_passing());
        // Sorted by severity: critical first
        assert_eq!(report.findings[0].severity, Severity::Critical);
        assert_eq!(report.findings[1].severity, Severity::High);
    }

    #[test]
    fn test_auditor_score_calculation() {
        let mut auditor = SecurityAuditor::new();
        // Add 1 critical (25 pts) + 1 high (15 pts) = 40 deducted
        auditor.add_secret("plain", "not-encrypted");
        let mut rotator = SecretRotator::new(1);
        let old_time = SystemTime::now() - Duration::from_secs(2 * 86_400);
        rotator.register_secret_with_time("expired", "enc2:old", old_time);
        auditor.set_rotator(rotator);

        let report = auditor.audit_report();
        assert_eq!(report.overall_score, 60); // 100 - 25 - 15
    }

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Medium);
        assert!(Severity::Medium > Severity::Low);
        assert!(Severity::Low > Severity::Info);
    }

    #[test]
    fn test_severity_display() {
        assert_eq!(format!("{}", Severity::Critical), "critical");
        assert_eq!(format!("{}", Severity::High), "high");
        assert_eq!(format!("{}", Severity::Medium), "medium");
        assert_eq!(format!("{}", Severity::Low), "low");
        assert_eq!(format!("{}", Severity::Info), "info");
    }

    #[test]
    fn test_action_from_str() {
        assert_eq!(Action::from_str("execute"), Some(Action::Execute));
        assert_eq!(Action::from_str("Read"), Some(Action::Read));
        assert_eq!(Action::from_str("WRITE"), Some(Action::Write));
        assert_eq!(Action::from_str("admin"), Some(Action::Admin));
        assert_eq!(Action::from_str("unknown"), None);
    }

    #[test]
    fn test_action_as_str() {
        assert_eq!(Action::Execute.as_str(), "execute");
        assert_eq!(Action::Read.as_str(), "read");
        assert_eq!(Action::Write.as_str(), "write");
        assert_eq!(Action::Admin.as_str(), "admin");
    }

    #[test]
    fn test_report_is_passing_no_findings() {
        let report = SecurityReport::from_findings(vec![]);
        assert!(report.is_passing());
        assert_eq!(report.overall_score, 100);
    }

    #[test]
    fn test_report_is_passing_low_only() {
        let report = SecurityReport::from_findings(vec![Finding {
            severity: Severity::Low,
            category: "test".to_string(),
            description: "minor issue".to_string(),
            recommendation: "fix it".to_string(),
        }]);
        assert!(report.is_passing()); // Low findings don't fail
        assert_eq!(report.overall_score, 97); // 100 - 3
    }

    #[test]
    fn test_rbac_default_impl() {
        let enforcer = RbacEnforcer::default();
        assert!(enforcer.check_permission("admin", "shell", "execute"));
    }

    #[test]
    fn test_auditor_default_impl() {
        let auditor = SecurityAuditor::default();
        let report = auditor.audit_report();
        assert_eq!(report.total_findings, 0);
    }
}
