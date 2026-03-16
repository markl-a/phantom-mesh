//! Cost Tracker — records token usage and estimates costs per agent/provider/model run.
//! Persisted to SQLite. Queryable by day, agent, provider.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::debug;

/// A single cost record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostRecord {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub agent: String,
    pub provider: String,
    pub model: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub total_tokens: u32,
    pub estimated_cost_usd: f64,
    pub duration_secs: f64,
    /// Optional context: hand name, phase, cron job, etc.
    pub context: Option<String>,
}

/// Summary of costs grouped by a dimension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostSummary {
    pub group: String,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
    pub call_count: u32,
}

/// Cost tracker with SQLite persistence
pub struct CostTracker {
    db_path: String,
}

impl CostTracker {
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = rusqlite::Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cost_records (
                id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                agent TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                tokens_in INTEGER NOT NULL DEFAULT 0,
                tokens_out INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                estimated_cost_usd REAL NOT NULL DEFAULT 0.0,
                duration_secs REAL NOT NULL DEFAULT 0.0,
                context TEXT,
                date_key TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_cost_date ON cost_records(date_key);
            CREATE INDEX IF NOT EXISTS idx_cost_agent ON cost_records(agent);
            CREATE INDEX IF NOT EXISTS idx_cost_provider ON cost_records(provider);"
        )?;
        Ok(Self { db_path: db_path.to_string() })
    }

    /// Record a new cost entry
    pub fn record(&self, record: &CostRecord) -> Result<()> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let date_key = record.timestamp.format("%Y-%m-%d").to_string();
        conn.execute(
            "INSERT INTO cost_records (id, timestamp, agent, provider, model, tokens_in, tokens_out, total_tokens, estimated_cost_usd, duration_secs, context, date_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                record.id,
                record.timestamp.to_rfc3339(),
                record.agent,
                record.provider,
                record.model,
                record.tokens_in,
                record.tokens_out,
                record.total_tokens,
                record.estimated_cost_usd,
                record.duration_secs,
                record.context,
                date_key,
            ],
        )?;
        debug!("Cost recorded: {} tokens, ${:.6} ({}:{})", record.total_tokens, record.estimated_cost_usd, record.provider, record.model);
        Ok(())
    }

    /// Check if an agent has exceeded its daily budget.
    /// Returns Ok(()) if within budget, Err if over.
    /// If `daily_limit_usd` is 0.0 or negative, no limit is enforced.
    pub fn check_budget(&self, agent: &str, daily_limit_usd: f64) -> Result<()> {
        if daily_limit_usd <= 0.0 {
            return Ok(()); // No limit configured
        }
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT COALESCE(SUM(estimated_cost_usd), 0.0)
             FROM cost_records WHERE date_key = ?1 AND agent = ?2"
        )?;
        let spent: f64 = stmt.query_row(rusqlite::params![today, agent], |row| row.get(0))?;
        if spent >= daily_limit_usd {
            anyhow::bail!(
                "Budget exceeded for agent '{}': ${:.4} spent today (limit: ${:.2})",
                agent, spent, daily_limit_usd
            );
        }
        debug!("Budget check OK for '{}': ${:.4} / ${:.2}", agent, spent, daily_limit_usd);
        Ok(())
    }

    /// Get total costs for today
    pub fn today_total(&self) -> Result<CostSummary> {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        self.summary_for_date(&today)
    }

    /// Get total costs for a specific date
    pub fn summary_for_date(&self, date: &str) -> Result<CostSummary> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT COALESCE(SUM(total_tokens), 0), COALESCE(SUM(estimated_cost_usd), 0.0), COUNT(*)
             FROM cost_records WHERE date_key = ?1"
        )?;
        let (tokens, cost, count) = stmt.query_row([date], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?, row.get::<_, u32>(2)?))
        })?;
        Ok(CostSummary {
            group: date.to_string(),
            total_tokens: tokens as u64,
            total_cost_usd: cost,
            call_count: count,
        })
    }

    /// Get costs grouped by agent
    pub fn by_agent(&self, days: u32) -> Result<Vec<CostSummary>> {
        let cutoff = (Utc::now() - chrono::Duration::days(days as i64)).format("%Y-%m-%d").to_string();
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT agent, SUM(total_tokens), SUM(estimated_cost_usd), COUNT(*)
             FROM cost_records WHERE date_key >= ?1
             GROUP BY agent ORDER BY SUM(estimated_cost_usd) DESC"
        )?;
        let summaries = stmt.query_map([&cutoff], |row| {
            Ok(CostSummary {
                group: row.get(0)?,
                total_tokens: row.get::<_, i64>(1)? as u64,
                total_cost_usd: row.get(2)?,
                call_count: row.get(3)?,
            })
        })?.filter_map(|r| r.ok()).collect();
        Ok(summaries)
    }

    /// Get costs grouped by provider
    pub fn by_provider(&self, days: u32) -> Result<Vec<CostSummary>> {
        let cutoff = (Utc::now() - chrono::Duration::days(days as i64)).format("%Y-%m-%d").to_string();
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT provider, SUM(total_tokens), SUM(estimated_cost_usd), COUNT(*)
             FROM cost_records WHERE date_key >= ?1
             GROUP BY provider ORDER BY SUM(estimated_cost_usd) DESC"
        )?;
        let summaries = stmt.query_map([&cutoff], |row| {
            Ok(CostSummary {
                group: row.get(0)?,
                total_tokens: row.get::<_, i64>(1)? as u64,
                total_cost_usd: row.get(2)?,
                call_count: row.get(3)?,
            })
        })?.filter_map(|r| r.ok()).collect();
        Ok(summaries)
    }

    /// Get costs grouped by day (last N days)
    pub fn by_day(&self, days: u32) -> Result<Vec<CostSummary>> {
        let cutoff = (Utc::now() - chrono::Duration::days(days as i64)).format("%Y-%m-%d").to_string();
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT date_key, SUM(total_tokens), SUM(estimated_cost_usd), COUNT(*)
             FROM cost_records WHERE date_key >= ?1
             GROUP BY date_key ORDER BY date_key DESC"
        )?;
        let summaries = stmt.query_map([&cutoff], |row| {
            Ok(CostSummary {
                group: row.get(0)?,
                total_tokens: row.get::<_, i64>(1)? as u64,
                total_cost_usd: row.get(2)?,
                call_count: row.get(3)?,
            })
        })?.filter_map(|r| r.ok()).collect();
        Ok(summaries)
    }

    /// Get all records for a date range (for export)
    pub fn records_between(&self, start: &str, end: &str) -> Result<Vec<CostRecord>> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT id, timestamp, agent, provider, model, tokens_in, tokens_out, total_tokens, estimated_cost_usd, duration_secs, context
             FROM cost_records WHERE date_key >= ?1 AND date_key <= ?2
             ORDER BY timestamp DESC"
        )?;
        let records = stmt.query_map(rusqlite::params![start, end], |row| {
            let ts_str: String = row.get(1)?;
            Ok(CostRecord {
                id: row.get(0)?,
                timestamp: DateTime::parse_from_rfc3339(&ts_str)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                agent: row.get(2)?,
                provider: row.get(3)?,
                model: row.get(4)?,
                tokens_in: row.get(5)?,
                tokens_out: row.get(6)?,
                total_tokens: row.get(7)?,
                estimated_cost_usd: row.get(8)?,
                duration_secs: row.get(9)?,
                context: row.get(10)?,
            })
        })?.filter_map(|r| r.ok()).collect();
        Ok(records)
    }
}

/// Estimate cost in USD based on provider/model and token counts.
/// Uses approximate public pricing (free-tier models = $0).
pub fn estimate_cost(provider: &str, model: &str, tokens_in: u32, tokens_out: u32) -> f64 {
    // Per 1M tokens pricing (approximate)
    let (input_per_m, output_per_m) = match provider {
        "anthropic" => match model {
            m if m.contains("opus") => (15.0, 75.0),
            m if m.contains("sonnet") => (3.0, 15.0),
            m if m.contains("haiku") => (0.25, 1.25),
            _ => (3.0, 15.0),
        },
        "openai" => match model {
            m if m.contains("gpt-4o") => (2.5, 10.0),
            m if m.contains("gpt-4") => (10.0, 30.0),
            m if m.contains("gpt-3.5") => (0.5, 1.5),
            m if m.contains("o1") => (15.0, 60.0),
            _ => (2.5, 10.0),
        },
        "groq" => (0.0, 0.0), // Free tier
        "gemini" => (0.0, 0.0), // Free tier
        "deepseek" => (0.14, 0.28), // Per 1M tokens
        "cerebras" => (0.0, 0.0), // Free tier
        "together" => (0.88, 0.88),
        "openrouter" => (0.4, 0.4),
        "ollama" | "lmstudio" | "lemonade" => (0.0, 0.0), // Local
        _ => (0.0, 0.0),
    };
    let cost = (tokens_in as f64 * input_per_m + tokens_out as f64 * output_per_m) / 1_000_000.0;
    cost
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> (String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join("clawtex_test_cost");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join(format!("{}.db", name));
        let _ = std::fs::remove_file(&db_path);
        (db_path.to_str().unwrap().to_string(), db_path)
    }

    fn sample_record(agent: &str, provider: &str, model: &str, tokens: u32) -> CostRecord {
        CostRecord {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            agent: agent.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            tokens_in: tokens / 2,
            tokens_out: tokens / 2,
            total_tokens: tokens,
            estimated_cost_usd: estimate_cost(provider, model, tokens / 2, tokens / 2),
            duration_secs: 1.5,
            context: None,
        }
    }

    #[test]
    fn test_cost_tracker_record_and_query() {
        let (db_str, db_path) = temp_db("record_query");
        let tracker = CostTracker::new(&db_str).unwrap();

        tracker.record(&sample_record("master", "ollama", "qwen3:8b", 1000)).unwrap();
        tracker.record(&sample_record("master", "gemini", "gemini-2.5-flash-lite", 2000)).unwrap();

        let today = tracker.today_total().unwrap();
        assert_eq!(today.call_count, 2);
        assert_eq!(today.total_tokens, 3000);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_cost_tracker_by_agent() {
        let (db_str, db_path) = temp_db("by_agent");
        let tracker = CostTracker::new(&db_str).unwrap();

        tracker.record(&sample_record("master", "ollama", "qwen:8b", 1000)).unwrap();
        tracker.record(&sample_record("master", "ollama", "qwen:8b", 500)).unwrap();
        tracker.record(&sample_record("researcher", "gemini", "flash", 2000)).unwrap();

        let by_agent = tracker.by_agent(7).unwrap();
        assert_eq!(by_agent.len(), 2);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_cost_tracker_by_provider() {
        let (db_str, db_path) = temp_db("by_provider");
        let tracker = CostTracker::new(&db_str).unwrap();

        tracker.record(&sample_record("master", "ollama", "qwen:8b", 1000)).unwrap();
        tracker.record(&sample_record("master", "anthropic", "claude-sonnet", 500)).unwrap();

        let by_prov = tracker.by_provider(7).unwrap();
        assert_eq!(by_prov.len(), 2);
        // Anthropic should cost more
        let anthropic = by_prov.iter().find(|s| s.group == "anthropic").unwrap();
        assert!(anthropic.total_cost_usd > 0.0);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_estimate_cost_free() {
        assert_eq!(estimate_cost("ollama", "qwen:8b", 1000, 1000), 0.0);
        assert_eq!(estimate_cost("gemini", "flash", 1000, 1000), 0.0);
        assert_eq!(estimate_cost("groq", "llama", 1000, 1000), 0.0);
    }

    #[test]
    fn test_estimate_cost_paid() {
        let cost = estimate_cost("anthropic", "claude-sonnet-4", 1000, 500);
        // 1000 * 3.0 / 1M + 500 * 15.0 / 1M = 0.003 + 0.0075 = 0.0105
        assert!((cost - 0.0105).abs() < 0.0001);
    }

    #[test]
    fn test_cost_tracker_empty() {
        let (db_str, db_path) = temp_db("empty");
        let tracker = CostTracker::new(&db_str).unwrap();

        let today = tracker.today_total().unwrap();
        assert_eq!(today.call_count, 0);
        assert_eq!(today.total_tokens, 0);
        assert_eq!(today.total_cost_usd, 0.0);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_check_budget_within_limit() {
        let (db_str, db_path) = temp_db("budget_ok");
        let tracker = CostTracker::new(&db_str).unwrap();
        // Record a small cost (ollama = free)
        tracker.record(&sample_record("master", "ollama", "qwen:8b", 1000)).unwrap();
        // Budget check should pass
        assert!(tracker.check_budget("master", 10.0).is_ok());
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_check_budget_exceeded() {
        let (db_str, db_path) = temp_db("budget_exceeded");
        let tracker = CostTracker::new(&db_str).unwrap();
        // Record expensive Anthropic usage
        let mut rec = sample_record("master", "anthropic", "claude-opus", 100_000);
        rec.estimated_cost_usd = 5.0; // Force $5 spent
        tracker.record(&rec).unwrap();
        // Budget of $1 should be exceeded
        assert!(tracker.check_budget("master", 1.0).is_err());
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_check_budget_no_limit() {
        let (db_str, db_path) = temp_db("budget_no_limit");
        let tracker = CostTracker::new(&db_str).unwrap();
        let mut rec = sample_record("master", "anthropic", "claude-opus", 100_000);
        rec.estimated_cost_usd = 999.0;
        tracker.record(&rec).unwrap();
        // 0.0 means no limit
        assert!(tracker.check_budget("master", 0.0).is_ok());
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_check_budget_per_agent() {
        let (db_str, db_path) = temp_db("budget_per_agent");
        let tracker = CostTracker::new(&db_str).unwrap();
        // Master spends $5
        let mut rec = sample_record("master", "anthropic", "claude-opus", 100_000);
        rec.estimated_cost_usd = 5.0;
        tracker.record(&rec).unwrap();
        // Coder has spent nothing — should pass
        assert!(tracker.check_budget("coder", 1.0).is_ok());
        // Master should fail
        assert!(tracker.check_budget("master", 1.0).is_err());
        let _ = std::fs::remove_file(&db_path);
    }
}