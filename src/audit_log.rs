//! Audit Log — records all significant actions for security auditing and compliance.
//! SQLite-backed, persisted to ~/.phantom-mesh/audit.db.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Categories of auditable actions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActionType {
    ToolExecution,
    FileWrite,
    ShellCommand,
    ExternalSend,
    ApprovalDecision,
    ConfigChange,
    DataExport,
    Login,
}

impl ActionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ToolExecution => "ToolExecution",
            Self::FileWrite => "FileWrite",
            Self::ShellCommand => "ShellCommand",
            Self::ExternalSend => "ExternalSend",
            Self::ApprovalDecision => "ApprovalDecision",
            Self::ConfigChange => "ConfigChange",
            Self::DataExport => "DataExport",
            Self::Login => "Login",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "ToolExecution" => Some(Self::ToolExecution),
            "FileWrite" => Some(Self::FileWrite),
            "ShellCommand" => Some(Self::ShellCommand),
            "ExternalSend" => Some(Self::ExternalSend),
            "ApprovalDecision" => Some(Self::ApprovalDecision),
            "ConfigChange" => Some(Self::ConfigChange),
            "DataExport" => Some(Self::DataExport),
            "Login" => Some(Self::Login),
            _ => None,
        }
    }
}

impl std::fmt::Display for ActionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Risk level for audited actions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Outcome of an audited action
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Failure,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "success" => Some(Self::Success),
            "failure" => Some(Self::Failure),
            _ => None,
        }
    }
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A single audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub agent_name: String,
    pub action_type: ActionType,
    pub tool_name: Option<String>,
    pub target: Option<String>,
    pub details: Option<Value>,
    pub outcome: Outcome,
    pub session_id: Option<String>,
    pub risk_level: RiskLevel,
}

/// Filters for querying the audit log
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditFilter {
    pub agent: Option<String>,
    pub action_type: Option<ActionType>,
    pub risk_level: Option<RiskLevel>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub tool_name: Option<String>,
    pub outcome: Option<Outcome>,
    pub limit: Option<usize>,
}

// ── Risk Classification ───────────────────────────────────────────────────────

/// Determine risk level for a tool execution based on tool name.
pub fn risk_level_for_tool(tool_name: &str) -> RiskLevel {
    match tool_name {
        // Critical: direct system access or external sends with side effects
        "shell" | "computer_use" => RiskLevel::High,

        // Medium: file mutations, external communication
        "file_write" | "file_edit" | "scaffold_saas" | "render_deploy" | "stripe" => {
            RiskLevel::Medium
        }
        "email" | "email_send" | "twitter" | "blog_publish" | "slack_send" | "discord_send"
        | "line_send" | "whatsapp_send" => RiskLevel::Medium,

        // Low: read-only or local processing
        "file_read" | "web_search" | "http_request" | "glob_search" | "content_search"
        | "memory_store" | "memory_recall" | "memory_forget" | "vision" | "browser"
        | "translate" | "json_transform" | "csv_parse" | "summarize" | "pdf_export"
        | "docx_export" | "xlsx_export" | "image_generate" | "skeleton_generate"
        | "ai_code" | "cli_anything" | "delegate" | "delegate_to_provider" | "run_hand" => {
            RiskLevel::Low
        }

        // Default: unknown tools get medium risk
        _ => RiskLevel::Medium,
    }
}

// ── AuditLogger ───────────────────────────────────────────────────────────────

/// Audit logger with SQLite persistence.
/// Thread-safe: stores db_path and opens connections per-call
/// (same pattern as CostTracker and KnowledgeCapturer).
pub struct AuditLogger {
    db_path: String,
}

impl AuditLogger {
    /// Create a new AuditLogger, initializing the SQLite schema.
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = rusqlite::Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS audit_log (
                id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                agent_name TEXT NOT NULL,
                action_type TEXT NOT NULL,
                tool_name TEXT,
                target TEXT,
                details TEXT,
                outcome TEXT NOT NULL,
                session_id TEXT,
                risk_level TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_log(timestamp);
            CREATE INDEX IF NOT EXISTS idx_audit_agent ON audit_log(agent_name);
            CREATE INDEX IF NOT EXISTS idx_audit_action_type ON audit_log(action_type);
            CREATE INDEX IF NOT EXISTS idx_audit_risk_level ON audit_log(risk_level);
            CREATE INDEX IF NOT EXISTS idx_audit_tool ON audit_log(tool_name);",
        )?;
        Ok(Self {
            db_path: db_path.to_string(),
        })
    }

    /// Log an action to the audit trail.
    pub async fn log_action(
        &self,
        agent_name: &str,
        action_type: ActionType,
        tool_name: Option<&str>,
        target: Option<&str>,
        details: Option<Value>,
        outcome: Outcome,
        session_id: Option<&str>,
        risk_level: RiskLevel,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let timestamp = Utc::now();
        let details_str = details.as_ref().map(|d| d.to_string());

        let db_path = self.db_path.clone();
        let id_clone = id.clone();
        let agent = agent_name.to_string();
        let action = action_type.as_str().to_string();
        let tool = tool_name.map(|s| s.to_string());
        let tgt = target.map(|s| s.to_string());
        let out = outcome.as_str().to_string();
        let sess = session_id.map(|s| s.to_string());
        let risk = risk_level.as_str().to_string();

        // Run SQLite insert on blocking thread pool to avoid blocking async runtime
        tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(&db_path)?;
            conn.execute(
                "INSERT INTO audit_log (id, timestamp, agent_name, action_type, tool_name, target, details, outcome, session_id, risk_level)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    id_clone,
                    timestamp.to_rfc3339(),
                    agent,
                    action,
                    tool,
                    tgt,
                    details_str,
                    out,
                    sess,
                    risk,
                ],
            )?;
            Ok::<(), anyhow::Error>(())
        })
        .await??;

        debug!(
            "Audit logged: {} {} {} [{}] -> {}",
            agent_name,
            action_type,
            tool_name.unwrap_or("-"),
            risk_level,
            outcome,
        );

        Ok(id)
    }

    /// Query audit entries with optional filters.
    pub async fn query_audit(&self, filter: &AuditFilter) -> Result<Vec<AuditEntry>> {
        let db_path = self.db_path.clone();
        let filter = filter.clone();

        tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(&db_path)?;
            let mut conditions: Vec<String> = Vec::new();
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(ref agent) = filter.agent {
                conditions.push(format!("agent_name = ?{}", params.len() + 1));
                params.push(Box::new(agent.clone()));
            }
            if let Some(ref at) = filter.action_type {
                conditions.push(format!("action_type = ?{}", params.len() + 1));
                params.push(Box::new(at.as_str().to_string()));
            }
            if let Some(ref rl) = filter.risk_level {
                conditions.push(format!("risk_level = ?{}", params.len() + 1));
                params.push(Box::new(rl.as_str().to_string()));
            }
            if let Some(ref tool) = filter.tool_name {
                conditions.push(format!("tool_name = ?{}", params.len() + 1));
                params.push(Box::new(tool.clone()));
            }
            if let Some(ref out) = filter.outcome {
                conditions.push(format!("outcome = ?{}", params.len() + 1));
                params.push(Box::new(out.as_str().to_string()));
            }
            if let Some(ref start) = filter.start_time {
                conditions.push(format!("timestamp >= ?{}", params.len() + 1));
                params.push(Box::new(start.to_rfc3339()));
            }
            if let Some(ref end) = filter.end_time {
                conditions.push(format!("timestamp <= ?{}", params.len() + 1));
                params.push(Box::new(end.to_rfc3339()));
            }

            let where_clause = if conditions.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", conditions.join(" AND "))
            };

            let limit = filter.limit.unwrap_or(100);
            let query = format!(
                "SELECT id, timestamp, agent_name, action_type, tool_name, target, details, outcome, session_id, risk_level
                 FROM audit_log {} ORDER BY timestamp DESC LIMIT {}",
                where_clause, limit
            );

            let mut stmt = conn.prepare(&query)?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            let entries = stmt
                .query_map(param_refs.as_slice(), |row| {
                    let ts_str: String = row.get(1)?;
                    let details_str: Option<String> = row.get(6)?;
                    let action_str: String = row.get(3)?;
                    let outcome_str: String = row.get(7)?;
                    let risk_str: String = row.get(9)?;
                    Ok(AuditEntry {
                        id: row.get(0)?,
                        timestamp: DateTime::parse_from_rfc3339(&ts_str)
                            .map(|d| d.with_timezone(&Utc))
                            .unwrap_or_else(|_| Utc::now()),
                        agent_name: row.get(2)?,
                        action_type: ActionType::from_str(&action_str)
                            .unwrap_or(ActionType::ToolExecution),
                        tool_name: row.get(4)?,
                        target: row.get(5)?,
                        details: details_str
                            .and_then(|s| serde_json::from_str(&s).ok()),
                        outcome: Outcome::from_str(&outcome_str).unwrap_or(Outcome::Failure),
                        session_id: row.get(8)?,
                        risk_level: RiskLevel::from_str(&risk_str).unwrap_or(RiskLevel::Medium),
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();
            Ok(entries)
        })
        .await?
    }

    /// Get the most recent audit entries.
    pub async fn get_recent(&self, limit: usize) -> Result<Vec<AuditEntry>> {
        self.query_audit(&AuditFilter {
            limit: Some(limit),
            ..Default::default()
        })
        .await
    }

    /// Count total audit entries.
    pub async fn count(&self) -> Result<u64> {
        let db_path = self.db_path.clone();
        tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(&db_path)?;
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM audit_log", [], |row| row.get(0))?;
            Ok(count as u64)
        })
        .await?
    }

    /// Count entries by risk level (for health dashboard).
    pub async fn count_by_risk(&self) -> Result<Vec<(String, u64)>> {
        let db_path = self.db_path.clone();
        tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(&db_path)?;
            let mut stmt = conn.prepare(
                "SELECT risk_level, COUNT(*) FROM audit_log GROUP BY risk_level ORDER BY COUNT(*) DESC",
            )?;
            let results = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
                })?
                .filter_map(|r| r.ok())
                .collect();
            Ok(results)
        })
        .await?
    }

    /// Persist all in-memory audit_log entries to a separate SQLite database
    /// at `db_path`, writing into the `audit_entries` table.
    /// The target table is auto-created if it does not exist.
    pub async fn persist_to_db(&self, target_db_path: &str) -> Result<usize> {
        let source_db = self.db_path.clone();
        let target_db = target_db_path.to_string();

        tokio::task::spawn_blocking(move || {
            // Read all entries from the source audit_log table
            let src_conn = rusqlite::Connection::open(&source_db)?;
            let mut stmt = src_conn.prepare(
                "SELECT id, timestamp, agent_name, action_type, tool_name, target, details, outcome, session_id, risk_level
                 FROM audit_log ORDER BY timestamp ASC",
            )?;
            let entries: Vec<(String, String, String, String, Option<String>, Option<String>, Option<String>, String, Option<String>, String)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?,
                        row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?,
                        row.get(8)?, row.get(9)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();

            // Open target DB and ensure audit_entries table exists
            let tgt_conn = rusqlite::Connection::open(&target_db)?;
            ensure_audit_entries_table(&tgt_conn)?;

            let mut count = 0usize;
            for (id, ts, agent, action, tool, target, details, outcome, session, risk) in &entries {
                tgt_conn.execute(
                    "INSERT OR REPLACE INTO audit_entries (id, timestamp, agent_name, action_type, tool_name, target, details, outcome, session_id, risk_level)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![id, ts, agent, action, tool, target, details, outcome, session, risk],
                )?;
                count += 1;
            }
            Ok(count)
        })
        .await?
    }
}

// ── Standalone persistence helpers ────────────────────────────────────────────

/// Ensure the `audit_entries` table exists in the given connection.
/// Called automatically by `persist_to_db` and `load_from_db`.
fn ensure_audit_entries_table(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS audit_entries (
            id TEXT PRIMARY KEY,
            timestamp TEXT NOT NULL,
            agent_name TEXT NOT NULL,
            action_type TEXT NOT NULL,
            tool_name TEXT,
            target TEXT,
            details TEXT,
            outcome TEXT NOT NULL,
            session_id TEXT,
            risk_level TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_ae_timestamp ON audit_entries(timestamp);
        CREATE INDEX IF NOT EXISTS idx_ae_agent ON audit_entries(agent_name);
        CREATE INDEX IF NOT EXISTS idx_ae_action_type ON audit_entries(action_type);
        CREATE INDEX IF NOT EXISTS idx_ae_risk_level ON audit_entries(risk_level);
        CREATE INDEX IF NOT EXISTS idx_ae_tool ON audit_entries(tool_name);",
    )?;
    Ok(())
}

/// Parse a single row from the `audit_entries` table into an `AuditEntry`.
fn row_to_audit_entry(row: &rusqlite::Row) -> rusqlite::Result<AuditEntry> {
    let ts_str: String = row.get(1)?;
    let details_str: Option<String> = row.get(6)?;
    let action_str: String = row.get(3)?;
    let outcome_str: String = row.get(7)?;
    let risk_str: String = row.get(9)?;
    Ok(AuditEntry {
        id: row.get(0)?,
        timestamp: DateTime::parse_from_rfc3339(&ts_str)
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        agent_name: row.get(2)?,
        action_type: ActionType::from_str(&action_str).unwrap_or(ActionType::ToolExecution),
        tool_name: row.get(4)?,
        target: row.get(5)?,
        details: details_str.and_then(|s| serde_json::from_str(&s).ok()),
        outcome: Outcome::from_str(&outcome_str).unwrap_or(Outcome::Failure),
        session_id: row.get(8)?,
        risk_level: RiskLevel::from_str(&risk_str).unwrap_or(RiskLevel::Medium),
    })
}

/// Load audit entries from the `audit_entries` table in the given SQLite DB,
/// applying the provided filters. The table is auto-created if missing.
pub fn load_from_db(db_path: &str, filter: &AuditFilter) -> Result<Vec<AuditEntry>> {
    let conn = rusqlite::Connection::open(db_path)?;
    ensure_audit_entries_table(&conn)?;

    let mut conditions: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(ref agent) = filter.agent {
        conditions.push(format!("agent_name = ?{}", params.len() + 1));
        params.push(Box::new(agent.clone()));
    }
    if let Some(ref at) = filter.action_type {
        conditions.push(format!("action_type = ?{}", params.len() + 1));
        params.push(Box::new(at.as_str().to_string()));
    }
    if let Some(ref rl) = filter.risk_level {
        conditions.push(format!("risk_level = ?{}", params.len() + 1));
        params.push(Box::new(rl.as_str().to_string()));
    }
    if let Some(ref tool) = filter.tool_name {
        conditions.push(format!("tool_name = ?{}", params.len() + 1));
        params.push(Box::new(tool.clone()));
    }
    if let Some(ref out) = filter.outcome {
        conditions.push(format!("outcome = ?{}", params.len() + 1));
        params.push(Box::new(out.as_str().to_string()));
    }
    if let Some(ref start) = filter.start_time {
        conditions.push(format!("timestamp >= ?{}", params.len() + 1));
        params.push(Box::new(start.to_rfc3339()));
    }
    if let Some(ref end) = filter.end_time {
        conditions.push(format!("timestamp <= ?{}", params.len() + 1));
        params.push(Box::new(end.to_rfc3339()));
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let limit = filter.limit.unwrap_or(1000);
    let query = format!(
        "SELECT id, timestamp, agent_name, action_type, tool_name, target, details, outcome, session_id, risk_level
         FROM audit_entries {} ORDER BY timestamp DESC LIMIT {}",
        where_clause, limit
    );

    let mut stmt = conn.prepare(&query)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let entries = stmt
        .query_map(param_refs.as_slice(), row_to_audit_entry)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(entries)
}

/// Delete audit entries older than `max_days` from the `audit_entries` table.
/// Returns the number of deleted rows.
pub fn retention_cleanup(db_path: &str, max_days: u32) -> Result<usize> {
    let conn = rusqlite::Connection::open(db_path)?;
    ensure_audit_entries_table(&conn)?;

    let cutoff = Utc::now() - Duration::days(max_days as i64);
    let cutoff_str = cutoff.to_rfc3339();

    let deleted = conn.execute(
        "DELETE FROM audit_entries WHERE timestamp < ?1",
        rusqlite::params![cutoff_str],
    )?;

    debug!("Retention cleanup: deleted {} entries older than {} days", deleted, max_days);
    Ok(deleted)
}

/// Export a slice of `AuditEntry` values as a CSV string.
/// Columns: id, timestamp, agent_name, action_type, tool_name, target, details, outcome, session_id, risk_level
pub fn export_csv(entries: &[AuditEntry]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());

    // Header
    let _ = wtr.write_record([
        "id",
        "timestamp",
        "agent_name",
        "action_type",
        "tool_name",
        "target",
        "details",
        "outcome",
        "session_id",
        "risk_level",
    ]);

    for entry in entries {
        let _ = wtr.write_record([
            &entry.id,
            &entry.timestamp.to_rfc3339(),
            &entry.agent_name,
            entry.action_type.as_str(),
            entry.tool_name.as_deref().unwrap_or(""),
            entry.target.as_deref().unwrap_or(""),
            &entry.details.as_ref().map(|d| d.to_string()).unwrap_or_default(),
            entry.outcome.as_str(),
            entry.session_id.as_deref().unwrap_or(""),
            entry.risk_level.as_str(),
        ]);
    }

    let _ = wtr.flush();
    String::from_utf8(wtr.into_inner().unwrap_or_default()).unwrap_or_default()
}

/// Export a slice of `AuditEntry` values as a JSON string (pretty-printed array).
pub fn export_json(entries: &[AuditEntry]) -> String {
    serde_json::to_string_pretty(entries).unwrap_or_else(|_| "[]".to_string())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> (String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join("phantom_mesh_test_audit");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join(format!("{}.db", name));
        let _ = std::fs::remove_file(&db_path);
        (db_path.to_str().unwrap().to_string(), db_path)
    }

    #[tokio::test]
    async fn test_audit_log_create_and_count() {
        let (db_str, db_path) = temp_db("create_count");
        let logger = AuditLogger::new(&db_str).unwrap();
        assert_eq!(logger.count().await.unwrap(), 0);

        logger
            .log_action(
                "master",
                ActionType::ToolExecution,
                Some("shell"),
                Some("git status"),
                None,
                Outcome::Success,
                None,
                RiskLevel::High,
            )
            .await
            .unwrap();

        assert_eq!(logger.count().await.unwrap(), 1);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_audit_log_multiple_entries() {
        let (db_str, db_path) = temp_db("multiple");
        let logger = AuditLogger::new(&db_str).unwrap();

        logger
            .log_action(
                "master",
                ActionType::ToolExecution,
                Some("shell"),
                Some("ls"),
                None,
                Outcome::Success,
                None,
                RiskLevel::High,
            )
            .await
            .unwrap();

        logger
            .log_action(
                "coder",
                ActionType::FileWrite,
                Some("file_write"),
                Some("/workspace/test.py"),
                Some(serde_json::json!({"size": 1024})),
                Outcome::Success,
                Some("session-abc"),
                RiskLevel::Medium,
            )
            .await
            .unwrap();

        logger
            .log_action(
                "master",
                ActionType::ExternalSend,
                Some("email"),
                Some("user@example.com"),
                None,
                Outcome::Failure,
                None,
                RiskLevel::Medium,
            )
            .await
            .unwrap();

        assert_eq!(logger.count().await.unwrap(), 3);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_audit_get_recent() {
        let (db_str, db_path) = temp_db("recent");
        let logger = AuditLogger::new(&db_str).unwrap();

        for i in 0..5 {
            logger
                .log_action(
                    "master",
                    ActionType::ToolExecution,
                    Some(&format!("tool_{}", i)),
                    None,
                    None,
                    Outcome::Success,
                    None,
                    RiskLevel::Low,
                )
                .await
                .unwrap();
        }

        let recent = logger.get_recent(3).await.unwrap();
        assert_eq!(recent.len(), 3);
        // Most recent first
        assert!(recent[0].timestamp >= recent[1].timestamp);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_audit_query_by_agent() {
        let (db_str, db_path) = temp_db("query_agent");
        let logger = AuditLogger::new(&db_str).unwrap();

        logger
            .log_action("master", ActionType::ToolExecution, Some("shell"), None, None, Outcome::Success, None, RiskLevel::High)
            .await
            .unwrap();
        logger
            .log_action("coder", ActionType::ToolExecution, Some("file_read"), None, None, Outcome::Success, None, RiskLevel::Low)
            .await
            .unwrap();
        logger
            .log_action("master", ActionType::ShellCommand, Some("shell"), Some("npm test"), None, Outcome::Success, None, RiskLevel::High)
            .await
            .unwrap();

        let filter = AuditFilter {
            agent: Some("master".to_string()),
            ..Default::default()
        };
        let results = logger.query_audit(&filter).await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|e| e.agent_name == "master"));
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_audit_query_by_action_type() {
        let (db_str, db_path) = temp_db("query_action");
        let logger = AuditLogger::new(&db_str).unwrap();

        logger
            .log_action("master", ActionType::ToolExecution, Some("shell"), None, None, Outcome::Success, None, RiskLevel::High)
            .await
            .unwrap();
        logger
            .log_action("master", ActionType::ApprovalDecision, None, Some("approval_abc"), None, Outcome::Success, None, RiskLevel::Medium)
            .await
            .unwrap();

        let filter = AuditFilter {
            action_type: Some(ActionType::ApprovalDecision),
            ..Default::default()
        };
        let results = logger.query_audit(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].action_type, ActionType::ApprovalDecision);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_audit_query_by_risk_level() {
        let (db_str, db_path) = temp_db("query_risk");
        let logger = AuditLogger::new(&db_str).unwrap();

        logger
            .log_action("master", ActionType::ToolExecution, Some("shell"), None, None, Outcome::Success, None, RiskLevel::High)
            .await
            .unwrap();
        logger
            .log_action("master", ActionType::ToolExecution, Some("web_search"), None, None, Outcome::Success, None, RiskLevel::Low)
            .await
            .unwrap();
        logger
            .log_action("master", ActionType::ToolExecution, Some("file_write"), None, None, Outcome::Success, None, RiskLevel::Medium)
            .await
            .unwrap();

        let filter = AuditFilter {
            risk_level: Some(RiskLevel::High),
            ..Default::default()
        };
        let results = logger.query_audit(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].risk_level, RiskLevel::High);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_audit_query_by_tool_name() {
        let (db_str, db_path) = temp_db("query_tool");
        let logger = AuditLogger::new(&db_str).unwrap();

        logger
            .log_action("master", ActionType::ToolExecution, Some("shell"), None, None, Outcome::Success, None, RiskLevel::High)
            .await
            .unwrap();
        logger
            .log_action("master", ActionType::ToolExecution, Some("file_read"), None, None, Outcome::Success, None, RiskLevel::Low)
            .await
            .unwrap();

        let filter = AuditFilter {
            tool_name: Some("shell".to_string()),
            ..Default::default()
        };
        let results = logger.query_audit(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tool_name, Some("shell".to_string()));
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_audit_query_by_outcome() {
        let (db_str, db_path) = temp_db("query_outcome");
        let logger = AuditLogger::new(&db_str).unwrap();

        logger
            .log_action("master", ActionType::ToolExecution, Some("shell"), None, None, Outcome::Success, None, RiskLevel::High)
            .await
            .unwrap();
        logger
            .log_action("master", ActionType::ToolExecution, Some("shell"), None, None, Outcome::Failure, None, RiskLevel::High)
            .await
            .unwrap();

        let filter = AuditFilter {
            outcome: Some(Outcome::Failure),
            ..Default::default()
        };
        let results = logger.query_audit(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].outcome, Outcome::Failure);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_audit_query_combined_filters() {
        let (db_str, db_path) = temp_db("query_combined");
        let logger = AuditLogger::new(&db_str).unwrap();

        logger
            .log_action("master", ActionType::ToolExecution, Some("shell"), None, None, Outcome::Success, None, RiskLevel::High)
            .await
            .unwrap();
        logger
            .log_action("master", ActionType::ToolExecution, Some("file_read"), None, None, Outcome::Success, None, RiskLevel::Low)
            .await
            .unwrap();
        logger
            .log_action("coder", ActionType::ToolExecution, Some("shell"), None, None, Outcome::Failure, None, RiskLevel::High)
            .await
            .unwrap();

        let filter = AuditFilter {
            agent: Some("master".to_string()),
            risk_level: Some(RiskLevel::High),
            ..Default::default()
        };
        let results = logger.query_audit(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].agent_name, "master");
        assert_eq!(results[0].tool_name, Some("shell".to_string()));
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_audit_query_empty_result() {
        let (db_str, db_path) = temp_db("query_empty");
        let logger = AuditLogger::new(&db_str).unwrap();

        let filter = AuditFilter {
            agent: Some("nonexistent".to_string()),
            ..Default::default()
        };
        let results = logger.query_audit(&filter).await.unwrap();
        assert!(results.is_empty());
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_audit_details_json_roundtrip() {
        let (db_str, db_path) = temp_db("details_json");
        let logger = AuditLogger::new(&db_str).unwrap();

        let details = serde_json::json!({
            "command": "git status",
            "exit_code": 0,
            "output_len": 256
        });

        logger
            .log_action(
                "master",
                ActionType::ShellCommand,
                Some("shell"),
                Some("git status"),
                Some(details.clone()),
                Outcome::Success,
                Some("sess-123"),
                RiskLevel::High,
            )
            .await
            .unwrap();

        let entries = logger.get_recent(1).await.unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.agent_name, "master");
        assert_eq!(entry.action_type, ActionType::ShellCommand);
        assert_eq!(entry.tool_name, Some("shell".to_string()));
        assert_eq!(entry.target, Some("git status".to_string()));
        assert_eq!(entry.session_id, Some("sess-123".to_string()));
        assert_eq!(entry.risk_level, RiskLevel::High);
        assert_eq!(entry.outcome, Outcome::Success);

        // Verify JSON details round-tripped correctly
        let stored_details = entry.details.as_ref().unwrap();
        assert_eq!(stored_details["command"], "git status");
        assert_eq!(stored_details["exit_code"], 0);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_audit_count_by_risk() {
        let (db_str, db_path) = temp_db("count_risk");
        let logger = AuditLogger::new(&db_str).unwrap();

        // 2 high, 1 low, 1 medium
        logger.log_action("m", ActionType::ShellCommand, Some("shell"), None, None, Outcome::Success, None, RiskLevel::High).await.unwrap();
        logger.log_action("m", ActionType::ShellCommand, Some("shell"), None, None, Outcome::Success, None, RiskLevel::High).await.unwrap();
        logger.log_action("m", ActionType::ToolExecution, Some("web_search"), None, None, Outcome::Success, None, RiskLevel::Low).await.unwrap();
        logger.log_action("m", ActionType::FileWrite, Some("file_write"), None, None, Outcome::Success, None, RiskLevel::Medium).await.unwrap();

        let by_risk = logger.count_by_risk().await.unwrap();
        assert_eq!(by_risk.len(), 3);
        // Find high count
        let high = by_risk.iter().find(|(r, _)| r == "high").unwrap();
        assert_eq!(high.1, 2);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_risk_level_for_tool() {
        assert_eq!(risk_level_for_tool("shell"), RiskLevel::High);
        assert_eq!(risk_level_for_tool("computer_use"), RiskLevel::High);
        assert_eq!(risk_level_for_tool("file_write"), RiskLevel::Medium);
        assert_eq!(risk_level_for_tool("email"), RiskLevel::Medium);
        assert_eq!(risk_level_for_tool("file_read"), RiskLevel::Low);
        assert_eq!(risk_level_for_tool("web_search"), RiskLevel::Low);
        assert_eq!(risk_level_for_tool("unknown_new_tool"), RiskLevel::Medium);
    }

    #[test]
    fn test_action_type_roundtrip() {
        let types = vec![
            ActionType::ToolExecution,
            ActionType::FileWrite,
            ActionType::ShellCommand,
            ActionType::ExternalSend,
            ActionType::ApprovalDecision,
            ActionType::ConfigChange,
            ActionType::DataExport,
            ActionType::Login,
        ];
        for at in types {
            let s = at.as_str();
            let back = ActionType::from_str(s).unwrap();
            assert_eq!(at, back);
        }
    }

    #[test]
    fn test_risk_level_roundtrip() {
        let levels = vec![
            RiskLevel::Low,
            RiskLevel::Medium,
            RiskLevel::High,
            RiskLevel::Critical,
        ];
        for rl in levels {
            let s = rl.as_str();
            let back = RiskLevel::from_str(s).unwrap();
            assert_eq!(rl, back);
        }
    }

    #[test]
    fn test_outcome_roundtrip() {
        assert_eq!(Outcome::from_str("success"), Some(Outcome::Success));
        assert_eq!(Outcome::from_str("failure"), Some(Outcome::Failure));
        assert_eq!(Outcome::from_str("invalid"), None);
    }

    #[test]
    fn test_risk_level_ordering() {
        assert!(RiskLevel::Low < RiskLevel::Medium);
        assert!(RiskLevel::Medium < RiskLevel::High);
        assert!(RiskLevel::High < RiskLevel::Critical);
    }

    #[test]
    fn test_audit_entry_serialize() {
        let entry = AuditEntry {
            id: "test-id".to_string(),
            timestamp: Utc::now(),
            agent_name: "master".to_string(),
            action_type: ActionType::ToolExecution,
            tool_name: Some("shell".to_string()),
            target: Some("ls".to_string()),
            details: Some(serde_json::json!({"key": "value"})),
            outcome: Outcome::Success,
            session_id: Some("sess-1".to_string()),
            risk_level: RiskLevel::High,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("master"));
        assert!(json.contains("ToolExecution"));
        assert!(json.contains("High"));
        let back: AuditEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.agent_name, "master");
        assert_eq!(back.action_type, ActionType::ToolExecution);
    }

    #[test]
    fn test_audit_filter_default() {
        let filter = AuditFilter::default();
        assert!(filter.agent.is_none());
        assert!(filter.action_type.is_none());
        assert!(filter.risk_level.is_none());
        assert!(filter.start_time.is_none());
        assert!(filter.end_time.is_none());
        assert!(filter.limit.is_none());
    }

    #[tokio::test]
    async fn test_audit_query_with_limit() {
        let (db_str, db_path) = temp_db("query_limit");
        let logger = AuditLogger::new(&db_str).unwrap();

        for i in 0..10 {
            logger
                .log_action(
                    "master",
                    ActionType::ToolExecution,
                    Some(&format!("tool_{}", i)),
                    None,
                    None,
                    Outcome::Success,
                    None,
                    RiskLevel::Low,
                )
                .await
                .unwrap();
        }

        let filter = AuditFilter {
            limit: Some(5),
            ..Default::default()
        };
        let results = logger.query_audit(&filter).await.unwrap();
        assert_eq!(results.len(), 5);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_audit_no_filters_returns_all() {
        let (db_str, db_path) = temp_db("no_filters");
        let logger = AuditLogger::new(&db_str).unwrap();

        logger.log_action("a", ActionType::ToolExecution, None, None, None, Outcome::Success, None, RiskLevel::Low).await.unwrap();
        logger.log_action("b", ActionType::FileWrite, None, None, None, Outcome::Failure, None, RiskLevel::Medium).await.unwrap();

        let filter = AuditFilter::default();
        let results = logger.query_audit(&filter).await.unwrap();
        assert_eq!(results.len(), 2);
        let _ = std::fs::remove_file(&db_path);
    }

    // ── persist_to_db / load_from_db / retention_cleanup / export tests ───────

    fn tempfile_db() -> tempfile::NamedTempFile {
        tempfile::Builder::new()
            .suffix(".db")
            .tempfile()
            .unwrap()
    }

    #[tokio::test]
    async fn test_persist_to_db_basic() {
        let src = tempfile_db();
        let tgt = tempfile_db();
        let logger = AuditLogger::new(src.path().to_str().unwrap()).unwrap();

        logger.log_action("agent1", ActionType::ToolExecution, Some("shell"), Some("/bin/ls"), None, Outcome::Success, None, RiskLevel::High).await.unwrap();
        logger.log_action("agent2", ActionType::FileWrite, Some("file_write"), Some("/tmp/out"), None, Outcome::Success, Some("s1"), RiskLevel::Medium).await.unwrap();

        let count = logger.persist_to_db(tgt.path().to_str().unwrap()).await.unwrap();
        assert_eq!(count, 2);

        // Verify via load_from_db
        let entries = load_from_db(tgt.path().to_str().unwrap(), &AuditFilter::default()).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn test_persist_to_db_creates_table() {
        let src = tempfile_db();
        let tgt = tempfile_db();
        let logger = AuditLogger::new(src.path().to_str().unwrap()).unwrap();

        logger.log_action("a", ActionType::Login, None, None, None, Outcome::Success, None, RiskLevel::Low).await.unwrap();
        let _ = logger.persist_to_db(tgt.path().to_str().unwrap()).await.unwrap();

        // The audit_entries table should now exist in the target DB
        let conn = rusqlite::Connection::open(tgt.path()).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM audit_entries", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_persist_to_db_upsert_idempotent() {
        let src = tempfile_db();
        let tgt = tempfile_db();
        let logger = AuditLogger::new(src.path().to_str().unwrap()).unwrap();

        logger.log_action("a", ActionType::ToolExecution, Some("shell"), None, None, Outcome::Success, None, RiskLevel::Low).await.unwrap();

        // Persist twice — should not duplicate thanks to INSERT OR REPLACE
        let _ = logger.persist_to_db(tgt.path().to_str().unwrap()).await.unwrap();
        let _ = logger.persist_to_db(tgt.path().to_str().unwrap()).await.unwrap();

        let entries = load_from_db(tgt.path().to_str().unwrap(), &AuditFilter::default()).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_load_from_db_empty() {
        let db = tempfile_db();
        let entries = load_from_db(db.path().to_str().unwrap(), &AuditFilter::default()).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_load_from_db_with_agent_filter() {
        let db = tempfile_db();
        let conn = rusqlite::Connection::open(db.path()).unwrap();
        ensure_audit_entries_table(&conn).unwrap();

        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO audit_entries (id, timestamp, agent_name, action_type, outcome, risk_level)
             VALUES ('id1', ?1, 'alpha', 'ToolExecution', 'success', 'low')",
            rusqlite::params![now],
        ).unwrap();
        conn.execute(
            "INSERT INTO audit_entries (id, timestamp, agent_name, action_type, outcome, risk_level)
             VALUES ('id2', ?1, 'beta', 'FileWrite', 'failure', 'high')",
            rusqlite::params![now],
        ).unwrap();
        drop(conn);

        let filter = AuditFilter {
            agent: Some("alpha".to_string()),
            ..Default::default()
        };
        let entries = load_from_db(db.path().to_str().unwrap(), &filter).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_name, "alpha");
    }

    #[test]
    fn test_load_from_db_with_risk_filter() {
        let db = tempfile_db();
        let conn = rusqlite::Connection::open(db.path()).unwrap();
        ensure_audit_entries_table(&conn).unwrap();

        let now = Utc::now().to_rfc3339();
        for (id, risk) in &[("r1", "low"), ("r2", "high"), ("r3", "high")] {
            conn.execute(
                "INSERT INTO audit_entries (id, timestamp, agent_name, action_type, outcome, risk_level)
                 VALUES (?1, ?2, 'test', 'ToolExecution', 'success', ?3)",
                rusqlite::params![id, now, risk],
            ).unwrap();
        }
        drop(conn);

        let filter = AuditFilter {
            risk_level: Some(RiskLevel::High),
            ..Default::default()
        };
        let entries = load_from_db(db.path().to_str().unwrap(), &filter).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.risk_level == RiskLevel::High));
    }

    #[test]
    fn test_load_from_db_with_limit() {
        let db = tempfile_db();
        let conn = rusqlite::Connection::open(db.path()).unwrap();
        ensure_audit_entries_table(&conn).unwrap();

        let now = Utc::now().to_rfc3339();
        for i in 0..10 {
            conn.execute(
                "INSERT INTO audit_entries (id, timestamp, agent_name, action_type, outcome, risk_level)
                 VALUES (?1, ?2, 'a', 'ToolExecution', 'success', 'low')",
                rusqlite::params![format!("lim-{}", i), now],
            ).unwrap();
        }
        drop(conn);

        let filter = AuditFilter {
            limit: Some(3),
            ..Default::default()
        };
        let entries = load_from_db(db.path().to_str().unwrap(), &filter).unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_retention_cleanup_deletes_old() {
        let db = tempfile_db();
        let conn = rusqlite::Connection::open(db.path()).unwrap();
        ensure_audit_entries_table(&conn).unwrap();

        let old_ts = (Utc::now() - Duration::days(60)).to_rfc3339();
        let recent_ts = Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO audit_entries (id, timestamp, agent_name, action_type, outcome, risk_level)
             VALUES ('old1', ?1, 'a', 'Login', 'success', 'low')",
            rusqlite::params![old_ts],
        ).unwrap();
        conn.execute(
            "INSERT INTO audit_entries (id, timestamp, agent_name, action_type, outcome, risk_level)
             VALUES ('old2', ?1, 'a', 'Login', 'success', 'low')",
            rusqlite::params![old_ts],
        ).unwrap();
        conn.execute(
            "INSERT INTO audit_entries (id, timestamp, agent_name, action_type, outcome, risk_level)
             VALUES ('new1', ?1, 'a', 'Login', 'success', 'low')",
            rusqlite::params![recent_ts],
        ).unwrap();
        drop(conn);

        let deleted = retention_cleanup(db.path().to_str().unwrap(), 30).unwrap();
        assert_eq!(deleted, 2);

        // Only the recent entry should remain
        let remaining = load_from_db(db.path().to_str().unwrap(), &AuditFilter::default()).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "new1");
    }

    #[test]
    fn test_retention_cleanup_no_op_when_all_recent() {
        let db = tempfile_db();
        let conn = rusqlite::Connection::open(db.path()).unwrap();
        ensure_audit_entries_table(&conn).unwrap();

        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO audit_entries (id, timestamp, agent_name, action_type, outcome, risk_level)
             VALUES ('keep1', ?1, 'a', 'Login', 'success', 'low')",
            rusqlite::params![now],
        ).unwrap();
        drop(conn);

        let deleted = retention_cleanup(db.path().to_str().unwrap(), 7).unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn test_retention_cleanup_empty_table() {
        let db = tempfile_db();
        let deleted = retention_cleanup(db.path().to_str().unwrap(), 1).unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn test_export_csv_basic() {
        let entries = vec![
            AuditEntry {
                id: "csv-1".to_string(),
                timestamp: Utc::now(),
                agent_name: "master".to_string(),
                action_type: ActionType::ShellCommand,
                tool_name: Some("shell".to_string()),
                target: Some("git log".to_string()),
                details: None,
                outcome: Outcome::Success,
                session_id: None,
                risk_level: RiskLevel::High,
            },
            AuditEntry {
                id: "csv-2".to_string(),
                timestamp: Utc::now(),
                agent_name: "coder".to_string(),
                action_type: ActionType::FileWrite,
                tool_name: Some("file_write".to_string()),
                target: Some("/tmp/x.txt".to_string()),
                details: Some(serde_json::json!({"size": 42})),
                outcome: Outcome::Failure,
                session_id: Some("sess-abc".to_string()),
                risk_level: RiskLevel::Medium,
            },
        ];

        let csv_output = export_csv(&entries);
        // Verify header
        assert!(csv_output.starts_with("id,timestamp,agent_name,action_type,tool_name,target,details,outcome,session_id,risk_level"));
        // Verify data rows exist
        assert!(csv_output.contains("csv-1"));
        assert!(csv_output.contains("csv-2"));
        assert!(csv_output.contains("master"));
        assert!(csv_output.contains("coder"));
        assert!(csv_output.contains("ShellCommand"));
        assert!(csv_output.contains("FileWrite"));
        assert!(csv_output.contains("success"));
        assert!(csv_output.contains("failure"));

        // Parse as CSV to verify structure
        let mut rdr = csv::Reader::from_reader(csv_output.as_bytes());
        let records: Vec<_> = rdr.records().filter_map(|r| r.ok()).collect();
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn test_export_csv_empty() {
        let csv_output = export_csv(&[]);
        // Should still have header
        assert!(csv_output.contains("id,timestamp"));
        let mut rdr = csv::Reader::from_reader(csv_output.as_bytes());
        let records: Vec<_> = rdr.records().filter_map(|r| r.ok()).collect();
        assert_eq!(records.len(), 0);
    }

    #[test]
    fn test_export_json_basic() {
        let entries = vec![
            AuditEntry {
                id: "json-1".to_string(),
                timestamp: Utc::now(),
                agent_name: "tester".to_string(),
                action_type: ActionType::Login,
                tool_name: None,
                target: None,
                details: Some(serde_json::json!({"ip": "127.0.0.1"})),
                outcome: Outcome::Success,
                session_id: Some("sess-xyz".to_string()),
                risk_level: RiskLevel::Low,
            },
        ];

        let json_output = export_json(&entries);
        // Should be valid JSON array
        let parsed: Vec<AuditEntry> = serde_json::from_str(&json_output).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "json-1");
        assert_eq!(parsed[0].agent_name, "tester");
        assert_eq!(parsed[0].action_type, ActionType::Login);
        assert_eq!(parsed[0].risk_level, RiskLevel::Low);
        assert_eq!(parsed[0].details.as_ref().unwrap()["ip"], "127.0.0.1");
    }

    #[test]
    fn test_export_json_empty() {
        let json_output = export_json(&[]);
        let parsed: Vec<AuditEntry> = serde_json::from_str(&json_output).unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn test_export_json_multiple_entries() {
        let entries = vec![
            AuditEntry {
                id: "j1".to_string(),
                timestamp: Utc::now(),
                agent_name: "a".to_string(),
                action_type: ActionType::DataExport,
                tool_name: None,
                target: None,
                details: None,
                outcome: Outcome::Success,
                session_id: None,
                risk_level: RiskLevel::Medium,
            },
            AuditEntry {
                id: "j2".to_string(),
                timestamp: Utc::now(),
                agent_name: "b".to_string(),
                action_type: ActionType::ConfigChange,
                tool_name: Some("config_tool".to_string()),
                target: Some("agents.toml".to_string()),
                details: None,
                outcome: Outcome::Failure,
                session_id: None,
                risk_level: RiskLevel::Critical,
            },
        ];

        let json_output = export_json(&entries);
        let parsed: Vec<AuditEntry> = serde_json::from_str(&json_output).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, "j1");
        assert_eq!(parsed[1].id, "j2");
        assert_eq!(parsed[1].risk_level, RiskLevel::Critical);
    }

    #[tokio::test]
    async fn test_persist_and_load_roundtrip_with_details() {
        let src = tempfile_db();
        let tgt = tempfile_db();
        let logger = AuditLogger::new(src.path().to_str().unwrap()).unwrap();

        let details = serde_json::json!({"cmd": "deploy", "version": "2.1.0"});
        logger.log_action(
            "deployer",
            ActionType::ShellCommand,
            Some("shell"),
            Some("deploy.sh"),
            Some(details.clone()),
            Outcome::Success,
            Some("deploy-session"),
            RiskLevel::Critical,
        ).await.unwrap();

        logger.persist_to_db(tgt.path().to_str().unwrap()).await.unwrap();

        let entries = load_from_db(tgt.path().to_str().unwrap(), &AuditFilter::default()).unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.agent_name, "deployer");
        assert_eq!(e.action_type, ActionType::ShellCommand);
        assert_eq!(e.tool_name, Some("shell".to_string()));
        assert_eq!(e.target, Some("deploy.sh".to_string()));
        assert_eq!(e.outcome, Outcome::Success);
        assert_eq!(e.session_id, Some("deploy-session".to_string()));
        assert_eq!(e.risk_level, RiskLevel::Critical);
        let d = e.details.as_ref().unwrap();
        assert_eq!(d["cmd"], "deploy");
        assert_eq!(d["version"], "2.1.0");
    }
}
