//! Service Tier Enforcement — defines Lite/Pro/Team tiers with tool access,
//! rate limits, storage quotas, and priority boosts.
//! Persisted to SQLite. Checked on tool execution and agent message processing.

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use tracing::{debug, info};

// ── Tier Enum ───────────────────────────────────────────────────────────────────

/// Service tier levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceTier {
    Lite,
    Pro,
    Team,
}

impl fmt::Display for ServiceTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServiceTier::Lite => write!(f, "lite"),
            ServiceTier::Pro => write!(f, "pro"),
            ServiceTier::Team => write!(f, "team"),
        }
    }
}

impl ServiceTier {
    /// Parse from string (case-insensitive)
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "lite" => Some(ServiceTier::Lite),
            "pro" => Some(ServiceTier::Pro),
            "team" => Some(ServiceTier::Team),
            _ => None,
        }
    }
}

// ── Tier Denied Error ───────────────────────────────────────────────────────────

/// Error returned when a tier check fails
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierDenied {
    pub agent: String,
    pub tier: ServiceTier,
    pub reason: String,
}

impl fmt::Display for TierDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tier '{}' denied for agent '{}': {}", self.tier, self.agent, self.reason)
    }
}

impl std::error::Error for TierDenied {}

// ── Allowed Set ─────────────────────────────────────────────────────────────────

/// Represents either "all" or a specific list of allowed items
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AllowedSet {
    All,
    List(Vec<String>),
}

impl AllowedSet {
    pub fn contains(&self, item: &str) -> bool {
        match self {
            AllowedSet::All => true,
            AllowedSet::List(list) => list.iter().any(|s| s == item),
        }
    }

    pub fn is_all(&self) -> bool {
        matches!(self, AllowedSet::All)
    }
}

// ── Tier Limits ─────────────────────────────────────────────────────────────────

/// Per-tier limits and permissions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierLimits {
    pub max_tasks_per_day: u32,
    pub allowed_tools: AllowedSet,
    pub allowed_models: AllowedSet,
    pub max_storage_bytes: u64,
    /// Priority boost (0 = no boost, higher = more priority, max 255)
    pub priority_boost: u8,
    /// Max concurrent agents (0 = unlimited)
    pub max_concurrent_agents: u32,
}

// ── Tier Config ─────────────────────────────────────────────────────────────────

/// Full tier configuration: tier + its limits
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierConfig {
    pub tier: ServiceTier,
    pub limits: TierLimits,
}

/// Build the default limits for each tier
pub fn default_tier_config(tier: ServiceTier) -> TierConfig {
    match tier {
        ServiceTier::Lite => TierConfig {
            tier,
            limits: TierLimits {
                max_tasks_per_day: 10_000,
                // Self-hosted single-user: allow all tools on Lite tier
                // (original restriction was for multi-tenant SaaS use)
                allowed_tools: AllowedSet::All,
                allowed_models: AllowedSet::All,
                max_storage_bytes: 10_737_418_240, // 10 GB
                priority_boost: 0,
                max_concurrent_agents: 10,
            },
        },
        ServiceTier::Pro => TierConfig {
            tier,
            limits: TierLimits {
                max_tasks_per_day: 500,
                allowed_tools: AllowedSet::List(
                    // All tools except scaffold_saas and render_deploy
                    vec![
                        "shell", "file_read", "file_write", "file_edit",
                        "web_search", "http_request", "glob_search", "content_search",
                        "memory_store", "memory_recall", "memory_forget",
                        "delegate", "delegate_to_provider", "ai_code",
                        "computer_use", "browser", "vision",
                        "email_send", "twitter", "blog_publish",
                        "pdf_export", "docx_export", "xlsx_export",
                        "skeleton_generate", "stripe",
                        "slack_send", "discord_send", "line_send", "whatsapp_send",
                        "translate", "json_transform", "csv_parse", "summarize",
                        "image_generate", "run_hand", "cli_anything",
                    ]
                    .into_iter()
                    .map(String::from)
                    .collect(),
                ),
                allowed_models: AllowedSet::List(vec![
                    "llama3.2:1b".to_string(),
                    "qwen3:8b".to_string(),
                    "gemma2:2b".to_string(),
                    "gpt-4o".to_string(),
                    "gpt-5.4".to_string(),
                    "claude-sonnet-4-20250514".to_string(),
                    "gemini-2.5-pro".to_string(),
                ]),
                max_storage_bytes: 10_737_418_240, // 10 GB
                priority_boost: 5,
                max_concurrent_agents: 5,
            },
        },
        ServiceTier::Team => TierConfig {
            tier,
            limits: TierLimits {
                max_tasks_per_day: 0, // 0 = unlimited
                allowed_tools: AllowedSet::All,
                allowed_models: AllowedSet::All,
                max_storage_bytes: 0, // 0 = unlimited
                priority_boost: 10,
                max_concurrent_agents: 0, // 0 = unlimited
            },
        },
    }
}

// ── Tier Usage ──────────────────────────────────────────────────────────────────

/// Current usage stats for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierUsage {
    pub agent: String,
    pub tier: ServiceTier,
    pub tasks_today: u32,
    pub tasks_limit: u32,
    pub storage_used: u64,
    pub storage_limit: u64,
}

// ── Service Tier Manager ────────────────────────────────────────────────────────

/// Manages tier assignments and enforces limits.
/// Stores tier assignments and daily task counts in SQLite.
pub struct ServiceTierManager {
    conn: Mutex<Connection>,
    /// Cached tier configs (precomputed for each tier)
    configs: HashMap<ServiceTier, TierConfig>,
}

impl ServiceTierManager {
    /// Create a new manager backed by the given SQLite path.
    /// Creates tables if they don't exist.
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS agent_tiers (
                agent TEXT PRIMARY KEY,
                tier TEXT NOT NULL DEFAULT 'lite',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS tier_usage (
                agent TEXT NOT NULL,
                date_key TEXT NOT NULL,
                task_count INTEGER NOT NULL DEFAULT 0,
                storage_bytes INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (agent, date_key)
            );"
        )?;

        let mut configs = HashMap::new();
        configs.insert(ServiceTier::Lite, default_tier_config(ServiceTier::Lite));
        configs.insert(ServiceTier::Pro, default_tier_config(ServiceTier::Pro));
        configs.insert(ServiceTier::Team, default_tier_config(ServiceTier::Team));

        Ok(Self {
            conn: Mutex::new(conn),
            configs,
        })
    }

    /// Get the tier for an agent. Defaults to Lite if not set.
    pub fn get_tier(&self, agent: &str) -> ServiceTier {
        let conn = self.conn.lock().unwrap();
        let result: Option<String> = conn
            .query_row(
                "SELECT tier FROM agent_tiers WHERE agent = ?1",
                params![agent],
                |row| row.get(0),
            )
            .ok();
        match result {
            Some(ref s) => ServiceTier::from_str_loose(s).unwrap_or(ServiceTier::Lite),
            None => ServiceTier::Lite,
        }
    }

    /// Set the tier for an agent (upsert).
    pub fn set_tier(&self, agent: &str, tier: ServiceTier) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO agent_tiers (agent, tier, updated_at) VALUES (?1, ?2, datetime('now'))
             ON CONFLICT(agent) DO UPDATE SET tier = ?2, updated_at = datetime('now')",
            params![agent, tier.to_string()],
        )?;
        info!("Set tier for agent '{}' to '{}'", agent, tier);
        Ok(())
    }

    /// Get the limits for a given tier.
    pub fn get_limits(&self, tier: ServiceTier) -> &TierLimits {
        &self.configs[&tier].limits
    }

    /// Get the full config for a tier.
    pub fn get_config(&self, tier: ServiceTier) -> &TierConfig {
        &self.configs[&tier]
    }

    /// Check if an agent is allowed to use a specific tool.
    /// Returns Ok(()) if allowed, Err(TierDenied) if not.
    pub fn check_access(&self, agent: &str, tool: &str) -> std::result::Result<(), TierDenied> {
        let tier = self.get_tier(agent);
        let limits = self.get_limits(tier);

        if !limits.allowed_tools.contains(tool) {
            return Err(TierDenied {
                agent: agent.to_string(),
                tier,
                reason: format!(
                    "Tool '{}' is not available on the {} tier. Upgrade to access this tool.",
                    tool, tier
                ),
            });
        }

        Ok(())
    }

    /// Check if an agent is allowed to use a specific model.
    pub fn check_model_access(&self, agent: &str, model: &str) -> std::result::Result<(), TierDenied> {
        let tier = self.get_tier(agent);
        let limits = self.get_limits(tier);

        if !limits.allowed_models.contains(model) {
            return Err(TierDenied {
                agent: agent.to_string(),
                tier,
                reason: format!(
                    "Model '{}' is not available on the {} tier. Upgrade to access this model.",
                    model, tier
                ),
            });
        }

        Ok(())
    }

    /// Check rate limit: has the agent exceeded its daily task quota?
    /// Returns Ok(()) if within limits, Err(TierDenied) if exceeded.
    pub fn check_rate_limit(&self, agent: &str) -> std::result::Result<(), TierDenied> {
        let tier = self.get_tier(agent);
        let limits = self.get_limits(tier);

        // Unlimited tasks (Team tier with 0 = unlimited)
        if limits.max_tasks_per_day == 0 {
            return Ok(());
        }

        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let conn = self.conn.lock().unwrap();
        let count: u32 = conn
            .query_row(
                "SELECT COALESCE(task_count, 0) FROM tier_usage WHERE agent = ?1 AND date_key = ?2",
                params![agent, today],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if count >= limits.max_tasks_per_day {
            return Err(TierDenied {
                agent: agent.to_string(),
                tier,
                reason: format!(
                    "Daily task limit exceeded: {}/{} tasks. Resets at midnight UTC.",
                    count, limits.max_tasks_per_day
                ),
            });
        }

        Ok(())
    }

    /// Increment the daily task counter for an agent.
    /// Should be called after a successful task execution.
    pub fn record_task(&self, agent: &str) -> Result<()> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO tier_usage (agent, date_key, task_count, storage_bytes)
             VALUES (?1, ?2, 1, 0)
             ON CONFLICT(agent, date_key) DO UPDATE SET task_count = task_count + 1",
            params![agent, today],
        )?;
        debug!("Recorded task for agent '{}' (date: {})", agent, today);
        Ok(())
    }

    /// Update storage usage for an agent.
    pub fn update_storage(&self, agent: &str, bytes: u64) -> Result<()> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO tier_usage (agent, date_key, task_count, storage_bytes)
             VALUES (?1, ?2, 0, ?3)
             ON CONFLICT(agent, date_key) DO UPDATE SET storage_bytes = ?3",
            params![agent, today, bytes as i64],
        )?;
        Ok(())
    }

    /// Get current usage stats for an agent.
    pub fn get_usage(&self, agent: &str) -> TierUsage {
        let tier = self.get_tier(agent);
        let limits = self.get_limits(tier);
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

        let conn = self.conn.lock().unwrap();
        let (tasks_today, storage_used) = conn
            .query_row(
                "SELECT COALESCE(task_count, 0), COALESCE(storage_bytes, 0) FROM tier_usage WHERE agent = ?1 AND date_key = ?2",
                params![agent, today],
                |row| Ok((row.get::<_, u32>(0)?, row.get::<_, u64>(1)?)),
            )
            .unwrap_or((0, 0));

        TierUsage {
            agent: agent.to_string(),
            tier,
            tasks_today,
            tasks_limit: limits.max_tasks_per_day,
            storage_used,
            storage_limit: limits.max_storage_bytes,
        }
    }

    /// Get priority boost for an agent (used in task queue ordering).
    pub fn priority_boost(&self, agent: &str) -> u8 {
        let tier = self.get_tier(agent);
        self.get_limits(tier).priority_boost
    }

    /// List all agents with their tier assignments.
    pub fn list_agents(&self) -> Vec<(String, ServiceTier)> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT agent, tier FROM agent_tiers ORDER BY agent")
            .unwrap();
        stmt.query_map([], |row| {
            let agent: String = row.get(0)?;
            let tier_str: String = row.get(1)?;
            Ok((agent, ServiceTier::from_str_loose(&tier_str).unwrap_or(ServiceTier::Lite)))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> String {
        let path = std::env::temp_dir().join(format!("clawtex_tier_test_{}.db", uuid::Uuid::new_v4()));
        path.to_string_lossy().to_string()
    }

    #[test]
    fn test_default_tier_is_lite() {
        let db = temp_db();
        let mgr = ServiceTierManager::new(&db).unwrap();
        assert_eq!(mgr.get_tier("some_agent"), ServiceTier::Lite);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn test_set_and_get_tier() {
        let db = temp_db();
        let mgr = ServiceTierManager::new(&db).unwrap();
        mgr.set_tier("agent_a", ServiceTier::Pro).unwrap();
        assert_eq!(mgr.get_tier("agent_a"), ServiceTier::Pro);
        mgr.set_tier("agent_a", ServiceTier::Team).unwrap();
        assert_eq!(mgr.get_tier("agent_a"), ServiceTier::Team);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn test_lite_tool_access() {
        let db = temp_db();
        let mgr = ServiceTierManager::new(&db).unwrap();
        // Lite tier: all tools allowed (self-hosted single-user mode)
        assert!(mgr.check_access("agent", "web_search").is_ok());
        assert!(mgr.check_access("agent", "shell").is_ok());
        assert!(mgr.check_access("agent", "scaffold_saas").is_ok());
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn test_pro_tool_access() {
        let db = temp_db();
        let mgr = ServiceTierManager::new(&db).unwrap();
        mgr.set_tier("agent", ServiceTier::Pro).unwrap();
        // Pro tier: shell allowed
        assert!(mgr.check_access("agent", "shell").is_ok());
        // Pro tier: scaffold_saas NOT allowed
        assert!(mgr.check_access("agent", "scaffold_saas").is_err());
        // Pro tier: render_deploy NOT allowed
        assert!(mgr.check_access("agent", "render_deploy").is_err());
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn test_team_all_tools() {
        let db = temp_db();
        let mgr = ServiceTierManager::new(&db).unwrap();
        mgr.set_tier("agent", ServiceTier::Team).unwrap();
        // Team tier: everything allowed
        assert!(mgr.check_access("agent", "scaffold_saas").is_ok());
        assert!(mgr.check_access("agent", "render_deploy").is_ok());
        assert!(mgr.check_access("agent", "shell").is_ok());
        assert!(mgr.check_access("agent", "any_future_tool").is_ok());
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn test_rate_limit_lite() {
        let db = temp_db();
        let mgr = ServiceTierManager::new(&db).unwrap();
        // Record 10_000 tasks (Lite limit)
        for _ in 0..10_000 {
            mgr.record_task("agent").unwrap();
        }
        // 10_001st should be denied
        let result = mgr.check_rate_limit("agent");
        assert!(result.is_err());
        let denied = result.unwrap_err();
        assert!(denied.reason.contains("10000"));
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn test_rate_limit_team_unlimited() {
        let db = temp_db();
        let mgr = ServiceTierManager::new(&db).unwrap();
        mgr.set_tier("agent", ServiceTier::Team).unwrap();
        // Team tier: unlimited tasks
        for _ in 0..200 {
            mgr.record_task("agent").unwrap();
        }
        assert!(mgr.check_rate_limit("agent").is_ok());
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn test_get_usage() {
        let db = temp_db();
        let mgr = ServiceTierManager::new(&db).unwrap();
        mgr.set_tier("agent", ServiceTier::Pro).unwrap();
        mgr.record_task("agent").unwrap();
        mgr.record_task("agent").unwrap();
        mgr.record_task("agent").unwrap();
        let usage = mgr.get_usage("agent");
        assert_eq!(usage.tier, ServiceTier::Pro);
        assert_eq!(usage.tasks_today, 3);
        assert_eq!(usage.tasks_limit, 500);
        assert_eq!(usage.storage_limit, 10_737_418_240);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn test_priority_boost() {
        let db = temp_db();
        let mgr = ServiceTierManager::new(&db).unwrap();
        assert_eq!(mgr.priority_boost("agent"), 0); // Lite default
        mgr.set_tier("agent", ServiceTier::Pro).unwrap();
        assert_eq!(mgr.priority_boost("agent"), 5);
        mgr.set_tier("agent", ServiceTier::Team).unwrap();
        assert_eq!(mgr.priority_boost("agent"), 10);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn test_model_access() {
        let db = temp_db();
        let mgr = ServiceTierManager::new(&db).unwrap();
        // Lite: all models allowed (self-hosted single-user mode)
        assert!(mgr.check_model_access("agent", "qwen3:8b").is_ok());
        assert!(mgr.check_model_access("agent", "gpt-4o").is_ok());
        // Pro: all models
        mgr.set_tier("agent", ServiceTier::Pro).unwrap();
        assert!(mgr.check_model_access("agent", "gpt-4o").is_ok());
        // Team: all models
        mgr.set_tier("agent", ServiceTier::Team).unwrap();
        assert!(mgr.check_model_access("agent", "any-model-xyz").is_ok());
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn test_list_agents() {
        let db = temp_db();
        let mgr = ServiceTierManager::new(&db).unwrap();
        mgr.set_tier("alice", ServiceTier::Pro).unwrap();
        mgr.set_tier("bob", ServiceTier::Team).unwrap();
        mgr.set_tier("charlie", ServiceTier::Lite).unwrap();
        let agents = mgr.list_agents();
        assert_eq!(agents.len(), 3);
        assert!(agents.iter().any(|(name, tier)| name == "alice" && *tier == ServiceTier::Pro));
        assert!(agents.iter().any(|(name, tier)| name == "bob" && *tier == ServiceTier::Team));
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn test_tier_display() {
        assert_eq!(ServiceTier::Lite.to_string(), "lite");
        assert_eq!(ServiceTier::Pro.to_string(), "pro");
        assert_eq!(ServiceTier::Team.to_string(), "team");
    }

    #[test]
    fn test_tier_from_str() {
        assert_eq!(ServiceTier::from_str_loose("lite"), Some(ServiceTier::Lite));
        assert_eq!(ServiceTier::from_str_loose("PRO"), Some(ServiceTier::Pro));
        assert_eq!(ServiceTier::from_str_loose("Team"), Some(ServiceTier::Team));
        assert_eq!(ServiceTier::from_str_loose("invalid"), None);
    }

    #[test]
    fn test_allowed_set_all() {
        let set = AllowedSet::All;
        assert!(set.contains("anything"));
        assert!(set.contains("shell"));
        assert!(set.is_all());
    }

    #[test]
    fn test_allowed_set_list() {
        let set = AllowedSet::List(vec!["a".to_string(), "b".to_string()]);
        assert!(set.contains("a"));
        assert!(set.contains("b"));
        assert!(!set.contains("c"));
        assert!(!set.is_all());
    }

    #[test]
    fn test_tier_denied_display() {
        let denied = TierDenied {
            agent: "test".to_string(),
            tier: ServiceTier::Lite,
            reason: "Tool 'shell' not available".to_string(),
        };
        let s = denied.to_string();
        assert!(s.contains("lite"));
        assert!(s.contains("test"));
        assert!(s.contains("shell"));
    }

    #[test]
    fn test_default_tier_configs() {
        let lite = default_tier_config(ServiceTier::Lite);
        assert_eq!(lite.limits.max_tasks_per_day, 10_000);
        assert_eq!(lite.limits.max_storage_bytes, 10_737_418_240);
        assert_eq!(lite.limits.priority_boost, 0);
        assert_eq!(lite.limits.max_concurrent_agents, 10);

        let pro = default_tier_config(ServiceTier::Pro);
        assert_eq!(pro.limits.max_tasks_per_day, 500);
        assert_eq!(pro.limits.max_storage_bytes, 10_737_418_240);
        assert_eq!(pro.limits.priority_boost, 5);
        assert_eq!(pro.limits.max_concurrent_agents, 5);
        assert!(!pro.limits.allowed_tools.contains("scaffold_saas"));
        assert!(!pro.limits.allowed_tools.contains("render_deploy"));

        let team = default_tier_config(ServiceTier::Team);
        assert_eq!(team.limits.max_tasks_per_day, 0); // unlimited
        assert_eq!(team.limits.max_storage_bytes, 0); // unlimited
        assert_eq!(team.limits.priority_boost, 10);
        assert_eq!(team.limits.max_concurrent_agents, 0); // unlimited
        assert!(team.limits.allowed_tools.is_all());
        assert!(team.limits.allowed_models.is_all());
    }

    #[test]
    fn test_storage_tracking() {
        let db = temp_db();
        let mgr = ServiceTierManager::new(&db).unwrap();
        mgr.update_storage("agent", 5000).unwrap();
        let usage = mgr.get_usage("agent");
        assert_eq!(usage.storage_used, 5000);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn test_set_tier_upsert() {
        let db = temp_db();
        let mgr = ServiceTierManager::new(&db).unwrap();
        mgr.set_tier("agent", ServiceTier::Lite).unwrap();
        mgr.set_tier("agent", ServiceTier::Pro).unwrap();
        mgr.set_tier("agent", ServiceTier::Team).unwrap();
        // Should not have duplicate rows — just one row with Team
        assert_eq!(mgr.get_tier("agent"), ServiceTier::Team);
        let agents = mgr.list_agents();
        assert_eq!(agents.len(), 1);
        let _ = std::fs::remove_file(&db);
    }
}
