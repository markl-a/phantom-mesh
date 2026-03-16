//! Autonomy Level — controls what actions an agent can take.
//! Inspired by ZeroClaw's AutonomyLevel system.

use serde::{Deserialize, Serialize};

/// Autonomy level determines what actions an agent is allowed to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutonomyLevel {
    /// Read-only: can observe but not act. No tool execution allowed.
    ReadOnly,
    /// Supervised: can act, but risky tools require approval.
    Supervised,
    /// Full: autonomous within policy bounds. All tools allowed.
    Full,
}

impl Default for AutonomyLevel {
    fn default() -> Self {
        Self::Full
    }
}

impl AutonomyLevel {
    /// Check if a tool is allowed at this autonomy level.
    /// Returns true if allowed, false if blocked.
    pub fn allows_tool(&self, tool_name: &str) -> bool {
        match self {
            AutonomyLevel::ReadOnly => {
                // Only read-only tools are allowed
                matches!(tool_name,
                    "file_read" | "glob_search" | "content_search" |
                    "memory_recall" | "web_search" | "vision"
                )
            }
            AutonomyLevel::Supervised => {
                // Most tools allowed, but risky ones would need approval
                // (approval gate integration is separate)
                true
            }
            AutonomyLevel::Full => true,
        }
    }

    /// Check if a tool is considered "risky" and needs approval in Supervised mode.
    pub fn needs_approval(&self, tool_name: &str) -> bool {
        match self {
            AutonomyLevel::Supervised => {
                matches!(tool_name,
                    "shell" | "file_write" | "file_edit" |
                    "email_send" | "twitter" | "blog_publish" |
                    "http_request" | "browser"
                )
            }
            _ => false,
        }
    }

    /// Parse from string, defaulting to Full for unknown values.
    pub fn from_str_or_default(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "readonly" | "read_only" | "read-only" => Self::ReadOnly,
            "supervised" => Self::Supervised,
            "full" | "autonomous" => Self::Full,
            _ => Self::Full,
        }
    }
}

impl std::fmt::Display for AutonomyLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadOnly => write!(f, "read-only"),
            Self::Supervised => write!(f, "supervised"),
            Self::Full => write!(f, "full"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_readonly_blocks_write_tools() {
        let level = AutonomyLevel::ReadOnly;
        assert!(level.allows_tool("file_read"));
        assert!(level.allows_tool("glob_search"));
        assert!(level.allows_tool("content_search"));
        assert!(level.allows_tool("memory_recall"));
        assert!(!level.allows_tool("shell"));
        assert!(!level.allows_tool("file_write"));
        assert!(!level.allows_tool("file_edit"));
        assert!(!level.allows_tool("twitter"));
        assert!(!level.allows_tool("email_send"));
    }

    #[test]
    fn test_supervised_allows_all() {
        let level = AutonomyLevel::Supervised;
        assert!(level.allows_tool("shell"));
        assert!(level.allows_tool("file_write"));
        assert!(level.allows_tool("file_read"));
    }

    #[test]
    fn test_supervised_flags_risky() {
        let level = AutonomyLevel::Supervised;
        assert!(level.needs_approval("shell"));
        assert!(level.needs_approval("file_write"));
        assert!(level.needs_approval("email_send"));
        assert!(level.needs_approval("twitter"));
        assert!(!level.needs_approval("file_read"));
        assert!(!level.needs_approval("web_search"));
    }

    #[test]
    fn test_full_allows_everything() {
        let level = AutonomyLevel::Full;
        assert!(level.allows_tool("shell"));
        assert!(level.allows_tool("twitter"));
        assert!(!level.needs_approval("shell"));
    }

    #[test]
    fn test_default_is_full() {
        assert_eq!(AutonomyLevel::default(), AutonomyLevel::Full);
    }

    #[test]
    fn test_from_str() {
        assert_eq!(AutonomyLevel::from_str_or_default("readonly"), AutonomyLevel::ReadOnly);
        assert_eq!(AutonomyLevel::from_str_or_default("read_only"), AutonomyLevel::ReadOnly);
        assert_eq!(AutonomyLevel::from_str_or_default("supervised"), AutonomyLevel::Supervised);
        assert_eq!(AutonomyLevel::from_str_or_default("full"), AutonomyLevel::Full);
        assert_eq!(AutonomyLevel::from_str_or_default("autonomous"), AutonomyLevel::Full);
        assert_eq!(AutonomyLevel::from_str_or_default("unknown"), AutonomyLevel::Full);
    }

    #[test]
    fn test_display() {
        assert_eq!(AutonomyLevel::ReadOnly.to_string(), "read-only");
        assert_eq!(AutonomyLevel::Supervised.to_string(), "supervised");
        assert_eq!(AutonomyLevel::Full.to_string(), "full");
    }

    #[test]
    fn test_serde_roundtrip() {
        let level = AutonomyLevel::Supervised;
        let json = serde_json::to_string(&level).unwrap();
        assert_eq!(json, "\"supervised\"");
        let parsed: AutonomyLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, AutonomyLevel::Supervised);
    }
}
