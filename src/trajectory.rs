//! Trajectory Logger — records every agent run's complete trajectory to SQLite.
//! Used by the self-evolution system (review_agents / self_evolve hands) to analyze
//! provider quality, worker efficiency, and identify optimization opportunities.
//!
//! DB: `~/.phantom-mesh/trajectories.db`

use std::sync::{Arc, Mutex};

use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tracing::debug;

// ---------------------------------------------------------------------------
// Data structs
// ---------------------------------------------------------------------------

/// A single trajectory entry capturing a complete agent run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryEntry {
    pub id: String,
    pub session_id: Option<String>,
    pub agent_name: String,
    pub hand_name: Option<String>,
    pub phase_name: Option<String>,
    pub provider: String,
    pub model: String,
    /// Prompt text (truncated to 2000 chars on insert).
    pub prompt: String,
    /// Model output text (truncated to 5000 chars on insert).
    pub output: String,
    pub tool_calls: usize,
    pub tool_names: Vec<String>,
    pub total_tokens: u32,
    pub duration_secs: f64,
    pub estimated_cost_usd: f64,
    /// Quality score from L2 LLM-as-Judge (1-5).
    pub quality_score: Option<u8>,
    /// Issues flagged by L1 guardrail.
    pub guardrail_issues: Vec<String>,
    pub success: bool,
    pub error_message: Option<String>,
    pub worker_name: Option<String>,
    pub worker_latency_ms: Option<u64>,
    pub created_at: String,
    /// Date portion only (YYYY-MM-DD), used for indexed queries and cleanup.
    pub date_key: String,
}

/// Aggregated quality statistics grouped by provider + model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityStats {
    pub provider: String,
    pub model: String,
    pub total_runs: u64,
    pub avg_quality: f64,
    pub avg_cost: f64,
    pub avg_duration: f64,
    pub success_rate: f64,
}

/// Worker efficiency statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerEfficiency {
    pub worker_name: String,
    pub total_tasks: u64,
    pub success_rate: f64,
    pub avg_latency_ms: f64,
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Truncate a string to at most `max_chars` characters, respecting UTF-8 char
/// boundaries. If truncated, no ellipsis is appended — callers can add one if
/// desired.
pub fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars).collect()
    }
}

// ---------------------------------------------------------------------------
// TrajectoryLogger
// ---------------------------------------------------------------------------

/// Persistent trajectory logger backed by SQLite.
///
/// Thread-safe: wraps `rusqlite::Connection` in `Arc<Mutex<>>` so it can be
/// shared across async tasks / threads.
pub struct TrajectoryLogger {
    conn: Arc<Mutex<Connection>>,
}

impl TrajectoryLogger {
    /// Open (or create) the trajectories database at `db_path` and ensure the
    /// schema exists.
    pub async fn new(db_path: &str) -> Result<Self> {
        let path = db_path.to_string();
        let conn = tokio::task::spawn_blocking(move || -> Result<Connection> {
            let conn = Connection::open(&path)?;
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS trajectories (
                    id                TEXT PRIMARY KEY,
                    session_id        TEXT,
                    agent_name        TEXT NOT NULL,
                    hand_name         TEXT,
                    phase_name        TEXT,
                    provider          TEXT NOT NULL,
                    model             TEXT NOT NULL,
                    prompt            TEXT NOT NULL,
                    output            TEXT NOT NULL,
                    tool_calls        INTEGER NOT NULL DEFAULT 0,
                    tool_names        TEXT NOT NULL DEFAULT '[]',
                    total_tokens      INTEGER NOT NULL DEFAULT 0,
                    duration_secs     REAL NOT NULL DEFAULT 0.0,
                    estimated_cost_usd REAL NOT NULL DEFAULT 0.0,
                    quality_score     INTEGER,
                    guardrail_issues  TEXT NOT NULL DEFAULT '[]',
                    success           INTEGER NOT NULL DEFAULT 1,
                    error_message     TEXT,
                    worker_name       TEXT,
                    worker_latency_ms INTEGER,
                    created_at        TEXT NOT NULL,
                    date_key          TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_traj_date      ON trajectories(date_key);
                CREATE INDEX IF NOT EXISTS idx_traj_agent     ON trajectories(agent_name);
                CREATE INDEX IF NOT EXISTS idx_traj_hand      ON trajectories(hand_name);
                CREATE INDEX IF NOT EXISTS idx_traj_provider  ON trajectories(provider);
                CREATE INDEX IF NOT EXISTS idx_traj_worker    ON trajectories(worker_name);
                CREATE INDEX IF NOT EXISTS idx_traj_created   ON trajectories(created_at);",
            )?;

            Ok(conn)
        }).await.map_err(|e| anyhow::anyhow!("spawn_blocking join error: {}", e))??;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Insert a trajectory entry. Prompt and output are automatically truncated
    /// to 2000 and 5000 chars respectively.
    pub fn log_run(&self, entry: &TrajectoryEntry) -> Result<()> {
        let prompt = truncate_str(&entry.prompt, 2000);
        let output = truncate_str(&entry.output, 5000);
        let tool_names_json = serde_json::to_string(&entry.tool_names)?;
        let guardrail_json = serde_json::to_string(&entry.guardrail_issues)?;

        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        conn.execute(
            "INSERT INTO trajectories (
                id, session_id, agent_name, hand_name, phase_name,
                provider, model, prompt, output,
                tool_calls, tool_names, total_tokens, duration_secs, estimated_cost_usd,
                quality_score, guardrail_issues, success, error_message,
                worker_name, worker_latency_ms,
                created_at, date_key
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9,
                ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18,
                ?19, ?20,
                ?21, ?22
            )",
            params![
                entry.id,
                entry.session_id,
                entry.agent_name,
                entry.hand_name,
                entry.phase_name,
                entry.provider,
                entry.model,
                prompt,
                output,
                entry.tool_calls as i64,
                tool_names_json,
                entry.total_tokens,
                entry.duration_secs,
                entry.estimated_cost_usd,
                entry.quality_score.map(|v| v as i32),
                guardrail_json,
                entry.success as i32,
                entry.error_message,
                entry.worker_name,
                entry.worker_latency_ms.map(|v| v as i64),
                entry.created_at,
                entry.date_key,
            ],
        )?;
        debug!(
            "Trajectory logged: agent={} provider={}:{} tokens={} cost=${:.6} success={}",
            entry.agent_name, entry.provider, entry.model,
            entry.total_tokens, entry.estimated_cost_usd, entry.success,
        );
        Ok(())
    }

    /// Query the most recent entries within the last `days` days, up to `limit`.
    pub fn recent(&self, days: u32, limit: usize) -> Result<Vec<TrajectoryEntry>> {
        let cutoff = (Utc::now() - chrono::Duration::days(days as i64))
            .format("%Y-%m-%d")
            .to_string();
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, agent_name, hand_name, phase_name,
                    provider, model, prompt, output,
                    tool_calls, tool_names, total_tokens, duration_secs, estimated_cost_usd,
                    quality_score, guardrail_issues, success, error_message,
                    worker_name, worker_latency_ms,
                    created_at, date_key
             FROM trajectories
             WHERE date_key >= ?1
             ORDER BY created_at DESC
             LIMIT ?2",
        )?;
        let entries = stmt
            .query_map(params![cutoff, limit as i64], row_to_entry)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    /// Query entries for a specific hand, most recent first.
    pub fn by_hand(&self, hand_name: &str, limit: usize) -> Result<Vec<TrajectoryEntry>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, agent_name, hand_name, phase_name,
                    provider, model, prompt, output,
                    tool_calls, tool_names, total_tokens, duration_secs, estimated_cost_usd,
                    quality_score, guardrail_issues, success, error_message,
                    worker_name, worker_latency_ms,
                    created_at, date_key
             FROM trajectories
             WHERE hand_name = ?1
             ORDER BY created_at DESC
             LIMIT ?2",
        )?;
        let entries = stmt
            .query_map(params![hand_name, limit as i64], row_to_entry)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    /// Query entries for a specific hand + phase.
    pub fn by_hand_phase(
        &self,
        hand_name: &str,
        phase_name: &str,
        limit: usize,
    ) -> Result<Vec<TrajectoryEntry>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, agent_name, hand_name, phase_name,
                    provider, model, prompt, output,
                    tool_calls, tool_names, total_tokens, duration_secs, estimated_cost_usd,
                    quality_score, guardrail_issues, success, error_message,
                    worker_name, worker_latency_ms,
                    created_at, date_key
             FROM trajectories
             WHERE hand_name = ?1 AND phase_name = ?2
             ORDER BY created_at DESC
             LIMIT ?3",
        )?;
        let entries = stmt
            .query_map(params![hand_name, phase_name, limit as i64], row_to_entry)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    /// Aggregate quality statistics grouped by (provider, model).
    /// Only considers entries that have a non-NULL quality_score.
    pub fn quality_stats(&self) -> Result<Vec<QualityStats>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT provider, model,
                    COUNT(*) as total_runs,
                    AVG(quality_score) as avg_quality,
                    AVG(estimated_cost_usd) as avg_cost,
                    AVG(duration_secs) as avg_duration,
                    (SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END) * 1.0 / COUNT(*)) as success_rate
             FROM trajectories
             WHERE quality_score IS NOT NULL
             GROUP BY provider, model
             ORDER BY avg_quality DESC",
        )?;
        let stats = stmt
            .query_map([], |row| {
                Ok(QualityStats {
                    provider: row.get(0)?,
                    model: row.get(1)?,
                    total_runs: row.get::<_, i64>(2)? as u64,
                    avg_quality: row.get(3)?,
                    avg_cost: row.get(4)?,
                    avg_duration: row.get(5)?,
                    success_rate: row.get(6)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(stats)
    }

    /// Worker efficiency statistics grouped by worker_name.
    /// Only considers entries with a non-NULL worker_name.
    pub fn worker_stats(&self) -> Result<Vec<WorkerEfficiency>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT worker_name,
                    COUNT(*) as total_tasks,
                    (SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END) * 1.0 / COUNT(*)) as success_rate,
                    AVG(worker_latency_ms) as avg_latency_ms
             FROM trajectories
             WHERE worker_name IS NOT NULL
             GROUP BY worker_name
             ORDER BY total_tasks DESC",
        )?;
        let stats = stmt
            .query_map([], |row| {
                Ok(WorkerEfficiency {
                    worker_name: row.get(0)?,
                    total_tasks: row.get::<_, i64>(1)? as u64,
                    success_rate: row.get(2)?,
                    avg_latency_ms: row.get::<_, f64>(3)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(stats)
    }

    /// List distinct hand names that have trajectory data.
    pub fn list_hand_names(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT hand_name FROM trajectories WHERE hand_name IS NOT NULL ORDER BY hand_name",
        )?;
        let names: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(names)
    }

    /// Count trajectory entries for a specific hand.
    pub fn count_for_hand(&self, hand_name: &str) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM trajectories WHERE hand_name = ?1",
            params![hand_name],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Delete trajectory entries older than `days` days.
    pub fn cleanup_old(&self, days: u32) -> Result<usize> {
        let cutoff = (Utc::now() - chrono::Duration::days(days as i64))
            .format("%Y-%m-%d")
            .to_string();
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        let deleted = conn.execute(
            "DELETE FROM trajectories WHERE date_key < ?1",
            params![cutoff],
        )?;
        debug!("Trajectory cleanup: deleted {} entries older than {} days", deleted, days);
        Ok(deleted)
    }
}

// ---------------------------------------------------------------------------
// Row mapping helper
// ---------------------------------------------------------------------------

/// Map a SQLite row to a `TrajectoryEntry`. Column order must match the SELECT
/// used in every query method above.
fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<TrajectoryEntry> {
    let tool_names_json: String = row.get(10)?;
    let guardrail_json: String = row.get(15)?;

    let tool_names: Vec<String> =
        serde_json::from_str(&tool_names_json).unwrap_or_default();
    let guardrail_issues: Vec<String> =
        serde_json::from_str(&guardrail_json).unwrap_or_default();

    let quality_raw: Option<i32> = row.get(14)?;
    let latency_raw: Option<i64> = row.get(19)?;

    Ok(TrajectoryEntry {
        id: row.get(0)?,
        session_id: row.get(1)?,
        agent_name: row.get(2)?,
        hand_name: row.get(3)?,
        phase_name: row.get(4)?,
        provider: row.get(5)?,
        model: row.get(6)?,
        prompt: row.get(7)?,
        output: row.get(8)?,
        tool_calls: row.get::<_, i64>(9)? as usize,
        tool_names,
        total_tokens: row.get::<_, i64>(11)? as u32,
        duration_secs: row.get(12)?,
        estimated_cost_usd: row.get(13)?,
        quality_score: quality_raw.map(|v| v as u8),
        guardrail_issues,
        success: row.get::<_, i32>(16)? != 0,
        error_message: row.get(17)?,
        worker_name: row.get(18)?,
        worker_latency_ms: latency_raw.map(|v| v as u64),
        created_at: row.get(20)?,
        date_key: row.get(21)?,
    })
}

// ---------------------------------------------------------------------------
// PluginModule adapter
// ---------------------------------------------------------------------------

use crate::app_context::AppContext;
use crate::health_check::HealthStatus;
use crate::plugin_bus::PluginModule;
use async_trait::async_trait;

/// Wraps TrajectoryLogger as a PluginModule.
///
/// On init, opens the SQLite database and registers `Arc<TrajectoryLogger>`
/// in AppContext for other modules (FeedbackLoop, Governor, etc.).
pub struct TrajectoryPlugin {
    db_path: String,
    logger: std::sync::RwLock<Option<Arc<TrajectoryLogger>>>,
}

impl TrajectoryPlugin {
    pub fn new(db_path: &str) -> Self {
        Self {
            db_path: db_path.to_string(),
            logger: std::sync::RwLock::new(None),
        }
    }
}

#[async_trait]
impl PluginModule for TrajectoryPlugin {
    fn id(&self) -> &str { "trajectory-logger" }
    fn version(&self) -> &str { env!("CARGO_PKG_VERSION") }
    fn capabilities(&self) -> Vec<String> {
        vec!["trajectory-logging".into(), "quality-analysis".into()]
    }
    async fn init(&self, ctx: &AppContext) -> anyhow::Result<()> {
        let logger = Arc::new(TrajectoryLogger::new(&self.db_path).await?);
        ctx.register(logger.clone());
        *self.logger.write().expect("lock poisoned") = Some(logger);
        Ok(())
    }
    async fn shutdown(&self) -> anyhow::Result<()> {
        *self.logger.write().expect("lock poisoned") = None;
        Ok(())
    }
    fn health(&self) -> HealthStatus {
        match self.logger.read().expect("lock poisoned").as_ref() {
            Some(_) => HealthStatus::Healthy,
            None => HealthStatus::Unhealthy,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a temp DB path, removing any stale file.
    fn temp_db(name: &str) -> (String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join("phantom_mesh_test_trajectory");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join(format!("{}.db", name));
        let _ = std::fs::remove_file(&db_path);
        (db_path.to_str().unwrap().to_string(), db_path)
    }

    /// Build a minimal valid entry for testing.
    fn sample_entry(agent: &str, provider: &str, model: &str) -> TrajectoryEntry {
        let now = Utc::now();
        TrajectoryEntry {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: None,
            agent_name: agent.to_string(),
            hand_name: None,
            phase_name: None,
            provider: provider.to_string(),
            model: model.to_string(),
            prompt: "test prompt".to_string(),
            output: "test output".to_string(),
            tool_calls: 0,
            tool_names: vec![],
            total_tokens: 100,
            duration_secs: 1.0,
            estimated_cost_usd: 0.001,
            quality_score: None,
            guardrail_issues: vec![],
            success: true,
            error_message: None,
            worker_name: None,
            worker_latency_ms: None,
            created_at: now.to_rfc3339(),
            date_key: now.format("%Y-%m-%d").to_string(),
        }
    }

    #[tokio::test]
    async fn test_create_and_log() {
        let (db_str, db_path) = temp_db("create_and_log");
        let logger = TrajectoryLogger::new(&db_str).await.unwrap();

        let entry = sample_entry("master", "ollama", "qwen3:8b");
        logger.log_run(&entry).unwrap();

        // Verify it was persisted
        let results = logger.recent(1, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].agent_name, "master");
        assert_eq!(results[0].provider, "ollama");
        assert_eq!(results[0].model, "qwen3:8b");

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_recent_query() {
        let (db_str, db_path) = temp_db("recent_query");
        let logger = TrajectoryLogger::new(&db_str).await.unwrap();

        // Insert 5 entries
        for i in 0..5 {
            let mut entry = sample_entry("agent", "ollama", "qwen3:8b");
            entry.total_tokens = (i + 1) * 100;
            logger.log_run(&entry).unwrap();
        }

        // Limit to 3
        let results = logger.recent(1, 3).unwrap();
        assert_eq!(results.len(), 3);

        // All 5
        let results = logger.recent(1, 100).unwrap();
        assert_eq!(results.len(), 5);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_by_hand_query() {
        let (db_str, db_path) = temp_db("by_hand");
        let logger = TrajectoryLogger::new(&db_str).await.unwrap();

        // Two entries for "seo_content", one for "outreach"
        let mut e1 = sample_entry("master", "gemini", "flash");
        e1.hand_name = Some("seo_content".to_string());
        logger.log_run(&e1).unwrap();

        let mut e2 = sample_entry("master", "gemini", "flash");
        e2.hand_name = Some("seo_content".to_string());
        logger.log_run(&e2).unwrap();

        let mut e3 = sample_entry("master", "ollama", "qwen3:8b");
        e3.hand_name = Some("outreach".to_string());
        logger.log_run(&e3).unwrap();

        let seo = logger.by_hand("seo_content", 100).unwrap();
        assert_eq!(seo.len(), 2);

        let outreach = logger.by_hand("outreach", 100).unwrap();
        assert_eq!(outreach.len(), 1);

        let missing = logger.by_hand("nonexistent", 100).unwrap();
        assert_eq!(missing.len(), 0);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_quality_stats() {
        let (db_str, db_path) = temp_db("quality_stats");
        let logger = TrajectoryLogger::new(&db_str).await.unwrap();

        // Provider A: 2 runs, quality 4 and 5, one failure
        let mut e1 = sample_entry("agent", "anthropic", "claude-sonnet");
        e1.quality_score = Some(4);
        e1.estimated_cost_usd = 0.01;
        e1.duration_secs = 2.0;
        e1.success = true;
        logger.log_run(&e1).unwrap();

        let mut e2 = sample_entry("agent", "anthropic", "claude-sonnet");
        e2.quality_score = Some(5);
        e2.estimated_cost_usd = 0.02;
        e2.duration_secs = 3.0;
        e2.success = false;
        logger.log_run(&e2).unwrap();

        // Provider B: 1 run, quality 3
        let mut e3 = sample_entry("agent", "ollama", "qwen3:8b");
        e3.quality_score = Some(3);
        e3.estimated_cost_usd = 0.0;
        e3.duration_secs = 1.0;
        e3.success = true;
        logger.log_run(&e3).unwrap();

        // Entry without quality score (should be excluded)
        let e4 = sample_entry("agent", "groq", "llama");
        logger.log_run(&e4).unwrap();

        let stats = logger.quality_stats().unwrap();
        assert_eq!(stats.len(), 2); // anthropic + ollama (groq excluded)

        // Anthropic should have avg quality 4.5
        let anth = stats.iter().find(|s| s.provider == "anthropic").unwrap();
        assert_eq!(anth.total_runs, 2);
        assert!((anth.avg_quality - 4.5).abs() < 0.01);
        assert!((anth.success_rate - 0.5).abs() < 0.01);

        // Ollama should have avg quality 3.0
        let oll = stats.iter().find(|s| s.provider == "ollama").unwrap();
        assert_eq!(oll.total_runs, 1);
        assert!((oll.avg_quality - 3.0).abs() < 0.01);
        assert!((oll.success_rate - 1.0).abs() < 0.01);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_worker_stats() {
        let (db_str, db_path) = temp_db("worker_stats");
        let logger = TrajectoryLogger::new(&db_str).await.unwrap();

        // Worker "acer": 3 tasks, 2 success, 1 failure
        for i in 0..3 {
            let mut e = sample_entry("agent", "ollama", "qwen3:8b");
            e.worker_name = Some("acer".to_string());
            e.worker_latency_ms = Some(100 + i * 50);
            e.success = i < 2; // first two succeed
            logger.log_run(&e).unwrap();
        }

        // Worker "m1-mac": 1 task, success
        let mut e = sample_entry("agent", "ollama", "qwen3:8b");
        e.worker_name = Some("m1-mac".to_string());
        e.worker_latency_ms = Some(200);
        e.success = true;
        logger.log_run(&e).unwrap();

        // Entry without worker (should be excluded)
        let e_no_worker = sample_entry("agent", "ollama", "qwen3:8b");
        logger.log_run(&e_no_worker).unwrap();

        let stats = logger.worker_stats().unwrap();
        assert_eq!(stats.len(), 2);

        let acer = stats.iter().find(|s| s.worker_name == "acer").unwrap();
        assert_eq!(acer.total_tasks, 3);
        // 2 out of 3 succeed = 0.6667
        assert!((acer.success_rate - 2.0 / 3.0).abs() < 0.01);
        // avg latency = (100 + 150 + 200) / 3 = 150
        assert!((acer.avg_latency_ms - 150.0).abs() < 1.0);

        let m1 = stats.iter().find(|s| s.worker_name == "m1-mac").unwrap();
        assert_eq!(m1.total_tasks, 1);
        assert!((m1.success_rate - 1.0).abs() < 0.01);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_cleanup_old() {
        let (db_str, db_path) = temp_db("cleanup_old");
        let logger = TrajectoryLogger::new(&db_str).await.unwrap();

        // Insert an entry dated "today"
        let mut today_entry = sample_entry("agent", "ollama", "qwen3:8b");
        today_entry.date_key = Utc::now().format("%Y-%m-%d").to_string();
        logger.log_run(&today_entry).unwrap();

        // Insert an entry dated 60 days ago
        let old_date = (Utc::now() - chrono::Duration::days(60))
            .format("%Y-%m-%d")
            .to_string();
        let mut old_entry = sample_entry("agent", "ollama", "qwen3:8b");
        old_entry.date_key = old_date;
        logger.log_run(&old_entry).unwrap();

        // Before cleanup: 2 entries
        let all = logger.recent(365, 100).unwrap();
        assert_eq!(all.len(), 2);

        // Cleanup entries older than 30 days
        let deleted = logger.cleanup_old(30).unwrap();
        assert_eq!(deleted, 1);

        // After cleanup: only today's entry remains
        let remaining = logger.recent(365, 100).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].agent_name, "agent");

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_truncate_str() {
        // ASCII within limit
        assert_eq!(truncate_str("hello", 10), "hello");

        // ASCII at limit
        assert_eq!(truncate_str("hello", 5), "hello");

        // ASCII over limit
        assert_eq!(truncate_str("hello world", 5), "hello");

        // Empty string
        assert_eq!(truncate_str("", 5), "");

        // Unicode: each CJK char is 1 char (but multi-byte)
        let cjk = "你好世界測試";
        assert_eq!(truncate_str(cjk, 3), "你好世");

        // Emoji (multi-byte, but single char)
        let emoji = "😀😁😂🤣😃";
        assert_eq!(truncate_str(emoji, 2), "😀😁");

        // Zero limit
        assert_eq!(truncate_str("hello", 0), "");
    }

    #[tokio::test]
    async fn test_entry_with_all_fields() {
        let (db_str, db_path) = temp_db("all_fields");
        let logger = TrajectoryLogger::new(&db_str).await.unwrap();

        let now = Utc::now();
        let entry = TrajectoryEntry {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: Some("sess-123".to_string()),
            agent_name: "master".to_string(),
            hand_name: Some("seo_content".to_string()),
            phase_name: Some("research".to_string()),
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4".to_string(),
            prompt: "Write a blog post about Rust async".to_string(),
            output: "Here is the blog post...".to_string(),
            tool_calls: 3,
            tool_names: vec![
                "web_search".to_string(),
                "file_write".to_string(),
                "memory_store".to_string(),
            ],
            total_tokens: 4500,
            duration_secs: 12.5,
            estimated_cost_usd: 0.045,
            quality_score: Some(4),
            guardrail_issues: vec!["minor_pii_detected".to_string()],
            success: true,
            error_message: None,
            worker_name: Some("acer".to_string()),
            worker_latency_ms: Some(250),
            created_at: now.to_rfc3339(),
            date_key: now.format("%Y-%m-%d").to_string(),
        };

        logger.log_run(&entry).unwrap();

        let results = logger.recent(1, 10).unwrap();
        assert_eq!(results.len(), 1);

        let r = &results[0];
        assert_eq!(r.session_id.as_deref(), Some("sess-123"));
        assert_eq!(r.agent_name, "master");
        assert_eq!(r.hand_name.as_deref(), Some("seo_content"));
        assert_eq!(r.phase_name.as_deref(), Some("research"));
        assert_eq!(r.provider, "anthropic");
        assert_eq!(r.model, "claude-sonnet-4");
        assert_eq!(r.tool_calls, 3);
        assert_eq!(r.tool_names, vec!["web_search", "file_write", "memory_store"]);
        assert_eq!(r.total_tokens, 4500);
        assert!((r.duration_secs - 12.5).abs() < 0.01);
        assert!((r.estimated_cost_usd - 0.045).abs() < 0.0001);
        assert_eq!(r.quality_score, Some(4));
        assert_eq!(r.guardrail_issues, vec!["minor_pii_detected"]);
        assert!(r.success);
        assert!(r.error_message.is_none());
        assert_eq!(r.worker_name.as_deref(), Some("acer"));
        assert_eq!(r.worker_latency_ms, Some(250));

        // Also test by_hand_phase
        let by_phase = logger.by_hand_phase("seo_content", "research", 10).unwrap();
        assert_eq!(by_phase.len(), 1);
        assert_eq!(by_phase[0].phase_name.as_deref(), Some("research"));

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_trajectory_plugin_lifecycle() {
        use crate::app_context::AppContext;
        use crate::plugin_bus::PluginModule;

        let dir = std::env::temp_dir().join(format!("traj-plugin-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("traj.db");

        let plugin = TrajectoryPlugin::new(db_path.to_str().unwrap());
        let ctx = AppContext::new();

        assert_eq!(plugin.id(), "trajectory-logger");
        assert_eq!(plugin.health(), HealthStatus::Unhealthy); // Not initialized yet

        plugin.init(&ctx).await.unwrap();
        let logger = ctx.get::<TrajectoryLogger>().unwrap();
        assert_eq!(logger.count_for_hand("nonexistent").unwrap(), 0);
        assert_eq!(plugin.health(), HealthStatus::Healthy);

        plugin.shutdown().await.unwrap();
        assert_eq!(plugin.health(), HealthStatus::Unhealthy);
    }
}
