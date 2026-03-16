//! RBAC (Role-Based Access Control) — per-user role management.
//! Inspired by ZeroClaw's RoleRegistry with owner/admin/operator/viewer.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::debug;

/// User role with associated permissions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Full control: all tools, all agents, config changes
    Owner,
    /// Administrative: all tools, all agents, no config changes
    Admin,
    /// Operational: approved tools only, assigned agents only
    Operator,
    /// View-only: read-only tools, no actions
    Viewer,
}

impl Default for Role {
    fn default() -> Self {
        Self::Viewer // Safe default: least privilege
    }
}

impl Role {
    /// Get the list of tool categories this role can use
    pub fn allowed_tool_categories(&self) -> &'static [&'static str] {
        match self {
            Role::Owner | Role::Admin => &["read", "write", "execute", "network", "social", "admin"],
            Role::Operator => &["read", "write", "execute", "network"],
            Role::Viewer => &["read"],
        }
    }

    /// Check if this role can use a specific tool
    pub fn can_use_tool(&self, tool_name: &str) -> bool {
        match self {
            Role::Owner | Role::Admin => true,
            Role::Operator => {
                // Operators can use most tools except social/admin
                !matches!(tool_name,
                    "twitter" | "blog_publish" | "email_send"
                )
            }
            Role::Viewer => {
                // Viewers can only read
                matches!(tool_name,
                    "file_read" | "glob_search" | "content_search" |
                    "memory_recall" | "web_search" | "vision"
                )
            }
        }
    }

    /// Check if this role can manage agents
    pub fn can_manage_agents(&self) -> bool {
        matches!(self, Role::Owner | Role::Admin)
    }

    /// Check if this role can modify configuration
    pub fn can_modify_config(&self) -> bool {
        matches!(self, Role::Owner)
    }

    /// Check if this role can use the e-stop
    pub fn can_estop(&self) -> bool {
        !matches!(self, Role::Viewer)
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Owner => write!(f, "owner"),
            Self::Admin => write!(f, "admin"),
            Self::Operator => write!(f, "operator"),
            Self::Viewer => write!(f, "viewer"),
        }
    }
}

/// Registry mapping user IDs to roles
pub struct RoleRegistry {
    /// user_id -> Role mapping
    roles: HashMap<String, Role>,
    /// The owner's user ID (always has Owner role)
    owner_id: Option<String>,
}

impl RoleRegistry {
    pub fn new() -> Self {
        Self {
            roles: HashMap::new(),
            owner_id: None,
        }
    }

    /// Create with an initial owner
    pub fn with_owner(owner_id: &str) -> Self {
        let mut registry = Self::new();
        registry.set_owner(owner_id);
        registry
    }

    /// Set the owner (only one owner allowed)
    pub fn set_owner(&mut self, user_id: &str) {
        self.owner_id = Some(user_id.to_string());
        self.roles.insert(user_id.to_string(), Role::Owner);
        debug!("Owner set: {}", user_id);
    }

    /// Assign a role to a user
    pub fn set_role(&mut self, user_id: &str, role: Role) {
        // Can't reassign the owner
        if self.owner_id.as_deref() == Some(user_id) && role != Role::Owner {
            return;
        }
        self.roles.insert(user_id.to_string(), role);
        debug!("Role assigned: {} -> {}", user_id, role);
    }

    /// Get a user's role (defaults to Viewer for unknown users)
    pub fn get_role(&self, user_id: &str) -> Role {
        self.roles.get(user_id).copied().unwrap_or(Role::Viewer)
    }

    /// Check if a user can use a tool
    pub fn can_use_tool(&self, user_id: &str, tool_name: &str) -> bool {
        self.get_role(user_id).can_use_tool(tool_name)
    }

    /// List all users and their roles
    pub fn list_roles(&self) -> Vec<(String, Role)> {
        let mut list: Vec<_> = self.roles.iter()
            .map(|(id, role)| (id.clone(), *role))
            .collect();
        list.sort_by(|a, b| a.0.cmp(&b.0));
        list
    }

    /// Remove a user's role (they'll default to Viewer)
    pub fn remove_role(&mut self, user_id: &str) {
        if self.owner_id.as_deref() == Some(user_id) {
            return; // Can't remove owner
        }
        self.roles.remove(user_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_role_default_is_viewer() {
        assert_eq!(Role::default(), Role::Viewer);
    }

    #[test]
    fn test_owner_can_do_everything() {
        let role = Role::Owner;
        assert!(role.can_use_tool("shell"));
        assert!(role.can_use_tool("twitter"));
        assert!(role.can_use_tool("file_read"));
        assert!(role.can_manage_agents());
        assert!(role.can_modify_config());
        assert!(role.can_estop());
    }

    #[test]
    fn test_admin_cannot_modify_config() {
        let role = Role::Admin;
        assert!(role.can_use_tool("shell"));
        assert!(role.can_manage_agents());
        assert!(!role.can_modify_config());
    }

    #[test]
    fn test_operator_limited_tools() {
        let role = Role::Operator;
        assert!(role.can_use_tool("shell"));
        assert!(role.can_use_tool("file_read"));
        assert!(!role.can_use_tool("twitter"));
        assert!(!role.can_use_tool("email_send"));
        assert!(!role.can_manage_agents());
    }

    #[test]
    fn test_viewer_read_only() {
        let role = Role::Viewer;
        assert!(role.can_use_tool("file_read"));
        assert!(role.can_use_tool("web_search"));
        assert!(!role.can_use_tool("shell"));
        assert!(!role.can_use_tool("file_write"));
        assert!(!role.can_manage_agents());
        assert!(!role.can_modify_config());
        assert!(!role.can_estop());
    }

    #[test]
    fn test_registry_with_owner() {
        let registry = RoleRegistry::with_owner("user123");
        assert_eq!(registry.get_role("user123"), Role::Owner);
        assert_eq!(registry.get_role("unknown"), Role::Viewer); // default
    }

    #[test]
    fn test_registry_set_role() {
        let mut registry = RoleRegistry::with_owner("owner1");
        registry.set_role("user2", Role::Admin);
        registry.set_role("user3", Role::Operator);
        assert_eq!(registry.get_role("user2"), Role::Admin);
        assert_eq!(registry.get_role("user3"), Role::Operator);
    }

    #[test]
    fn test_registry_cannot_demote_owner() {
        let mut registry = RoleRegistry::with_owner("owner1");
        registry.set_role("owner1", Role::Viewer); // Should be ignored
        assert_eq!(registry.get_role("owner1"), Role::Owner);
    }

    #[test]
    fn test_registry_cannot_remove_owner() {
        let mut registry = RoleRegistry::with_owner("owner1");
        registry.remove_role("owner1"); // Should be ignored
        assert_eq!(registry.get_role("owner1"), Role::Owner);
    }

    #[test]
    fn test_registry_remove_non_owner() {
        let mut registry = RoleRegistry::with_owner("owner1");
        registry.set_role("user2", Role::Admin);
        registry.remove_role("user2");
        assert_eq!(registry.get_role("user2"), Role::Viewer); // back to default
    }

    #[test]
    fn test_registry_can_use_tool() {
        let mut registry = RoleRegistry::with_owner("owner1");
        registry.set_role("viewer1", Role::Viewer);
        assert!(registry.can_use_tool("owner1", "shell"));
        assert!(!registry.can_use_tool("viewer1", "shell"));
        assert!(registry.can_use_tool("viewer1", "file_read"));
    }

    #[test]
    fn test_list_roles() {
        let mut registry = RoleRegistry::with_owner("owner1");
        registry.set_role("admin1", Role::Admin);
        let list = registry.list_roles();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_role_serde_roundtrip() {
        let role = Role::Operator;
        let json = serde_json::to_string(&role).unwrap();
        assert_eq!(json, "\"operator\"");
        let parsed: Role = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, Role::Operator);
    }

    #[test]
    fn test_role_display() {
        assert_eq!(Role::Owner.to_string(), "owner");
        assert_eq!(Role::Admin.to_string(), "admin");
        assert_eq!(Role::Operator.to_string(), "operator");
        assert_eq!(Role::Viewer.to_string(), "viewer");
    }
}
