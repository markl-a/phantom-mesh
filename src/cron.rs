// Cron/Scheduling system — inspired by ZeroClaw's cron module + OpenCrust's heartbeat
// Supports: cron expressions, one-shot (at), fixed interval (every)
// Persisted in SQLite, executed by background tokio task

use anyhow::Result;
use chrono::{DateTime, Utc, Duration as ChronoDuration, Timelike, Datelike};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::cost_tracker::CostTracker;
use crate::event_triggers::EventTriggerManager;
use crate::financial_monitor::{AlertLevel, FinancialAlert, FinancialMonitor, FinancialSnapshot};
use crate::revenue_tracker::RevenueTracker;

// ── Schedule Types ────────────────────────────────────────────────────────────

/// Schedule kind for a cron job
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Schedule {
    /// Standard 5-field cron expression: "min hour day month weekday"
    Cron { expr: String },
    /// One-shot at a specific UTC timestamp
    At { at: DateTime<Utc> },
    /// Fixed interval in seconds
    Every { interval_secs: u64 },
}

/// Job type: run a shell command or send a prompt to an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JobAction {
    /// Execute a shell command
    Shell { command: String },
    /// Send a prompt to a named agent
    Agent { agent: String, prompt: String },
    /// Send a message to a Telegram chat
    Notify { chat_id: String, message: String },
    /// Run a Hand workflow (multi-phase agent pipeline)
    Hand { hand_name: String, input: String },
}

/// Status of a cron job
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Active,
    Paused,
    Completed,
    Failed,
}

impl JobStatus {
    /// Convert to a plain string suitable for SQL storage (no JSON quotes).
    pub fn as_sql_str(&self) -> &'static str {
        match self {
            JobStatus::Active => "active",
            JobStatus::Paused => "paused",
            JobStatus::Completed => "completed",
            JobStatus::Failed => "failed",
        }
    }

    /// Parse from a plain SQL string. Falls back to Active for unrecognized values.
    pub fn from_sql_str(s: &str) -> Self {
        match s {
            "active" => JobStatus::Active,
            "paused" => JobStatus::Paused,
            "completed" => JobStatus::Completed,
            "failed" => JobStatus::Failed,
            // Backwards compat: handle old JSON-serialized values like "\"active\""
            s if s.starts_with('"') && s.ends_with('"') => {
                Self::from_sql_str(&s[1..s.len()-1])
            }
            _ => JobStatus::Active,
        }
    }
}

/// A scheduled job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub schedule: Schedule,
    pub action: JobAction,
    pub status: JobStatus,
    pub next_run: Option<DateTime<Utc>>,
    pub last_run: Option<DateTime<Utc>>,
    pub last_result: Option<String>,
    pub run_count: u32,
    pub max_runs: Option<u32>,  // None = unlimited
    pub created_at: DateTime<Utc>,
}

// ── Cron Expression Parser (minimal, 5-field) ────────────────────────────────

/// Parse a 5-field cron expression and compute the next run time after `after`
/// Fields: minute(0-59) hour(0-23) day(1-31) month(1-12) weekday(0-6, 0=Sun)
pub fn next_cron_run(expr: &str, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return None;
    }

    let minutes = parse_cron_field(fields[0], 0, 59)?;
    let hours = parse_cron_field(fields[1], 0, 23)?;
    let days = parse_cron_field(fields[2], 1, 31)?;
    let months = parse_cron_field(fields[3], 1, 12)?;
    let weekdays = parse_cron_field(fields[4], 0, 6)?;

    // Search forward from `after` + 1 minute, up to 1 year ahead
    let mut candidate = after + ChronoDuration::minutes(1);
    // Zero out seconds
    candidate = candidate.with_second(0).unwrap_or(candidate);

    let max_time = after + ChronoDuration::days(366);

    while candidate < max_time {
        let month = candidate.month();
        let day = candidate.day();
        let hour = candidate.hour();
        let minute = candidate.minute();
        let weekday = candidate.weekday().num_days_from_sunday(); // 0=Sun

        if months.contains(&(month as u8))
            && days.contains(&(day as u8))
            && hours.contains(&(hour as u8))
            && minutes.contains(&(minute as u8))
            && weekdays.contains(&(weekday as u8))
        {
            return Some(candidate);
        }

        candidate = candidate + ChronoDuration::minutes(1);
    }

    None // No match within a year
}

/// Parse a single cron field (e.g., "*/5", "1,15", "0-23", "*")
fn parse_cron_field(field: &str, min: u8, max: u8) -> Option<Vec<u8>> {
    if field == "*" {
        return Some((min..=max).collect());
    }

    // Handle */step
    if let Some(step_str) = field.strip_prefix("*/") {
        let step: u8 = step_str.parse().ok()?;
        if step == 0 { return None; }
        return Some((min..=max).step_by(step as usize).collect());
    }

    // Handle comma-separated values and ranges
    let mut values = Vec::new();
    for part in field.split(',') {
        if let Some((start_str, end_str)) = part.split_once('-') {
            let start: u8 = start_str.parse().ok()?;
            let end: u8 = end_str.parse().ok()?;
            if start > max || end > max { return None; }
            values.extend(start..=end);
        } else {
            let val: u8 = part.parse().ok()?;
            if val > max { return None; }
            values.push(val);
        }
    }

    if values.is_empty() { None } else { Some(values) }
}

// ── CronStore (SQLite persistence) ───────────────────────────────────────────

pub struct CronStore {
    #[allow(dead_code)]
    db_path: String,
    conn: std::sync::Mutex<rusqlite::Connection>,
}

impl CronStore {
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = rusqlite::Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cron_jobs (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                schedule_json TEXT NOT NULL,
                action_json TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                next_run TEXT,
                last_run TEXT,
                last_result TEXT,
                run_count INTEGER NOT NULL DEFAULT 0,
                max_runs INTEGER,
                created_at TEXT NOT NULL
            );"
        )?;
        Ok(Self { db_path: db_path.to_string(), conn: std::sync::Mutex::new(conn) })
    }

    pub fn save_job(&self, job: &CronJob) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR REPLACE INTO cron_jobs
             (id, name, schedule_json, action_json, status, next_run, last_run, last_result, run_count, max_runs, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                job.id,
                job.name,
                serde_json::to_string(&job.schedule)?,
                serde_json::to_string(&job.action)?,
                job.status.as_sql_str(),
                job.next_run.map(|t| t.to_rfc3339()),
                job.last_run.map(|t| t.to_rfc3339()),
                job.last_result,
                job.run_count,
                job.max_runs,
                job.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn load_active_jobs(&self) -> Result<Vec<CronJob>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, name, schedule_json, action_json, status, next_run, last_run, last_result, run_count, max_runs, created_at
             FROM cron_jobs WHERE status = 'active'"
        )?;

        let jobs = stmt.query_map([], |row| {
            let schedule_str: String = row.get(2)?;
            let action_str: String = row.get(3)?;
            let status_str: String = row.get(4)?;
            let next_run_str: Option<String> = row.get(5)?;
            let last_run_str: Option<String> = row.get(6)?;
            let created_str: String = row.get(10)?;

            Ok(CronJob {
                id: row.get(0)?,
                name: row.get(1)?,
                schedule: serde_json::from_str(&schedule_str).unwrap_or(Schedule::Every { interval_secs: 3600 }),
                action: serde_json::from_str(&action_str).unwrap_or(JobAction::Shell { command: "echo error".to_string() }),
                status: JobStatus::from_sql_str(&status_str),
                next_run: next_run_str.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc))),
                last_run: last_run_str.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc))),
                last_result: row.get(7)?,
                run_count: row.get::<_, u32>(8)?,
                max_runs: row.get(9)?,
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        })?.filter_map(|r| r.ok()).collect();

        Ok(jobs)
    }

    pub fn update_after_run(&self, job_id: &str, result: &str, next_run: Option<DateTime<Utc>>, new_status: JobStatus) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE cron_jobs SET last_run = ?1, last_result = ?2, next_run = ?3, status = ?4, run_count = run_count + 1
             WHERE id = ?5",
            rusqlite::params![
                Utc::now().to_rfc3339(),
                result,
                next_run.map(|t| t.to_rfc3339()),
                new_status.as_sql_str(),
                job_id,
            ],
        )?;
        Ok(())
    }

    pub fn delete_job(&self, job_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let rows = conn.execute("DELETE FROM cron_jobs WHERE id = ?1", [job_id])?;
        Ok(rows > 0)
    }

    pub fn list_all(&self) -> Result<Vec<CronJob>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, name, schedule_json, action_json, status, next_run, last_run, last_result, run_count, max_runs, created_at
             FROM cron_jobs ORDER BY created_at DESC"
        )?;

        let jobs = stmt.query_map([], |row| {
            let schedule_str: String = row.get(2)?;
            let action_str: String = row.get(3)?;
            let status_str: String = row.get(4)?;
            let next_run_str: Option<String> = row.get(5)?;
            let last_run_str: Option<String> = row.get(6)?;
            let created_str: String = row.get(10)?;

            Ok(CronJob {
                id: row.get(0)?,
                name: row.get(1)?,
                schedule: serde_json::from_str(&schedule_str).unwrap_or(Schedule::Every { interval_secs: 3600 }),
                action: serde_json::from_str(&action_str).unwrap_or(JobAction::Shell { command: "echo error".to_string() }),
                status: JobStatus::from_sql_str(&status_str),
                next_run: next_run_str.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc))),
                last_run: last_run_str.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc))),
                last_result: row.get(7)?,
                run_count: row.get::<_, u32>(8)?,
                max_runs: row.get(9)?,
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        })?.filter_map(|r| r.ok()).collect();

        Ok(jobs)
    }
}

// ── Financial Health Check ─────────────────────────────────────────────────

/// Default daily spending limit (USD) used when no config is available.
const DEFAULT_DAILY_LIMIT: f64 = 10.0;

/// Default interval between financial health checks (seconds). 1 hour.
pub const DEFAULT_FINANCIAL_CHECK_INTERVAL_SECS: u64 = 3600;

/// Run a periodic financial health check using data from cost_tracker and
/// optionally revenue_tracker. Builds a `FinancialSnapshot`, evaluates all
/// 7 indicators, logs each alert at the appropriate tracing level, and returns
/// the alert vector.
pub fn run_financial_check(
    cost_tracker: &CostTracker,
    revenue_tracker: Option<&RevenueTracker>,
) -> Vec<FinancialAlert> {
    // --- Gather daily spend from cost tracker ---
    let daily_spend = match cost_tracker.today_total() {
        Ok(summary) => summary.total_cost_usd,
        Err(e) => {
            warn!("Financial check: failed to read today's cost: {}", e);
            0.0
        }
    };

    // --- Gather revenue (today) from revenue tracker if available ---
    let revenue = match revenue_tracker {
        Some(rt) => match rt.today_total() {
            Ok(summary) => summary.total_usd,
            Err(e) => {
                warn!("Financial check: failed to read today's revenue: {}", e);
                0.0
            }
        },
        None => 0.0,
    };

    // Build snapshot with available data; fields we cannot determine use
    // safe defaults that will not trigger false positives.
    let snapshot = FinancialSnapshot {
        daily_spend,
        daily_limit: DEFAULT_DAILY_LIMIT,
        api_cost: daily_spend,           // approximate: daily API cost ~ daily spend
        revenue,
        previous_revenue: 0.0,           // no historical comparison available here
        project_cost: daily_spend,
        cash_balance: 0.0,               // unknown — will not fire (monthly_burn=0)
        monthly_burn: 0.0,               // unknown — skip runway check
        current_period_cost: daily_spend,
        average_cost: 0.0,               // unknown — skip spike check
        budget_used: daily_spend,
        budget_total: DEFAULT_DAILY_LIMIT,
    };

    let monitor = FinancialMonitor::default();
    let alerts = monitor.evaluate_all(&snapshot);

    // Log each alert at the appropriate tracing level
    for alert in &alerts {
        match alert.level {
            AlertLevel::Emergency | AlertLevel::Critical => {
                error!(
                    indicator = %alert.indicator_name,
                    level = %alert.level,
                    value = alert.current_value,
                    threshold = alert.threshold,
                    "[Financial] {}",
                    alert.message
                );
            }
            AlertLevel::Warn => {
                warn!(
                    indicator = %alert.indicator_name,
                    level = %alert.level,
                    value = alert.current_value,
                    threshold = alert.threshold,
                    "[Financial] {}",
                    alert.message
                );
            }
            AlertLevel::Info => {
                info!(
                    indicator = %alert.indicator_name,
                    level = %alert.level,
                    "[Financial] {}",
                    alert.message
                );
            }
        }
    }

    if alerts.is_empty() {
        debug!("Financial health check passed — no alerts (daily_spend=${:.4})", daily_spend);
    } else {
        info!(
            "Financial health check: {} alert(s) raised (daily_spend=${:.4})",
            alerts.len(),
            daily_spend
        );
    }

    alerts
}

// ── Scheduler (background runner) ────────────────────────────────────────────

/// Callback type for executing job actions
pub type JobExecutor = Arc<dyn Fn(JobAction) -> tokio::task::JoinHandle<String> + Send + Sync>;

/// The scheduler runs in the background, checking for due jobs every 30 seconds.
/// Optionally runs a periodic financial health check alongside cron jobs.
pub struct Scheduler {
    store: Arc<CronStore>,
    jobs: RwLock<Vec<CronJob>>,
    /// Interval in seconds between financial health checks. 0 = disabled.
    pub financial_check_interval_secs: u64,
}

impl Scheduler {
    pub fn new(store: Arc<CronStore>) -> Result<Self> {
        let jobs = store.load_active_jobs()?;
        info!("Scheduler loaded {} active jobs", jobs.len());
        Ok(Self {
            store,
            jobs: RwLock::new(jobs),
            financial_check_interval_secs: DEFAULT_FINANCIAL_CHECK_INTERVAL_SECS,
        })
    }

    /// Add a new job
    pub async fn add_job(&self, name: &str, schedule: Schedule, action: JobAction, max_runs: Option<u32>) -> Result<String> {
        let now = Utc::now();
        let next_run = compute_next_run(&schedule, &now);

        let job = CronJob {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            schedule,
            action,
            status: JobStatus::Active,
            next_run,
            last_run: None,
            last_result: None,
            run_count: 0,
            max_runs,
            created_at: now,
        };

        self.store.save_job(&job)?;
        let id = job.id.clone();
        self.jobs.write().await.push(job);
        info!("Added cron job '{}' (id={})", name, id);
        Ok(id)
    }

    /// Pause a job
    pub async fn pause_job(&self, job_id: &str) -> Result<bool> {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == job_id) {
            job.status = JobStatus::Paused;
            self.store.save_job(job)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Resume a paused job
    pub async fn resume_job(&self, job_id: &str) -> Result<bool> {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == job_id && j.status == JobStatus::Paused) {
            job.status = JobStatus::Active;
            job.next_run = compute_next_run(&job.schedule, &Utc::now());
            self.store.save_job(job)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Delete a job
    pub async fn delete_job(&self, job_id: &str) -> Result<bool> {
        let mut jobs = self.jobs.write().await;
        let len_before = jobs.len();
        jobs.retain(|j| j.id != job_id);
        if jobs.len() < len_before {
            self.store.delete_job(job_id)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// List all jobs
    pub async fn list_jobs(&self) -> Vec<CronJob> {
        self.jobs.read().await.clone()
    }

    /// Evaluate all due jobs once (single tick). Returns names of triggered jobs.
    /// Used by test harnesses to drive the scheduler without the infinite loop.
    pub async fn tick_now(&self, executor: &JobExecutor) -> Vec<String> {
        let now = chrono::Utc::now();
        let mut triggered = Vec::new();

        // Collect due jobs
        let due_jobs: Vec<CronJob> = {
            let jobs = self.jobs.read().await;
            jobs.iter()
                .filter(|j| {
                    j.status == JobStatus::Active
                        && j.next_run.map(|nr| nr <= now).unwrap_or(false)
                })
                .cloned()
                .collect()
        };

        for job in due_jobs {
            let action = job.action.clone();
            let handle = executor(action);
            let result = match handle.await {
                Ok(output) => output,
                Err(e) => format!("Job execution error: {}", e),
            };

            let next_run = compute_next_run(&job.schedule, &now);
            let new_run_count = job.run_count + 1;
            let new_status = if matches!(job.schedule, Schedule::At { .. }) {
                JobStatus::Completed
            } else if job.max_runs.map(|m| new_run_count >= m).unwrap_or(false) {
                JobStatus::Completed
            } else if next_run.is_none() {
                JobStatus::Completed
            } else {
                JobStatus::Active
            };

            if let Err(e) = self.store.update_after_run(&job.id, &result, next_run, new_status) {
                tracing::error!("Failed to update cron job '{}': {}", job.id, e);
            }

            {
                let mut jobs = self.jobs.write().await;
                if let Some(j) = jobs.iter_mut().find(|j| j.id == job.id) {
                    j.last_run = Some(now);
                    j.last_result = Some(result);
                    j.next_run = next_run;
                    j.run_count = new_run_count;
                    j.status = new_status;
                }
            }

            triggered.push(job.name.clone());
        }

        triggered
    }

    /// Main scheduler loop — call this in a tokio::spawn.
    /// This is the backward-compatible version without financial checks.
    pub async fn run(&self, executor: JobExecutor) {
        self.run_with_financial_check(executor, None, None).await;
    }

    /// Main scheduler loop with optional event trigger evaluation.
    /// Evaluates event triggers every tick alongside regular cron jobs.
    /// If `trigger_manager` is None, trigger evaluation is skipped.
    pub async fn run_with_triggers(
        &self,
        executor: JobExecutor,
        trigger_manager: Option<Arc<std::sync::Mutex<EventTriggerManager>>>,
        db_path: String,
        cost_tracker: Option<Arc<CostTracker>>,
        revenue_tracker: Option<Arc<RevenueTracker>>,
    ) {
        info!("Scheduler started (with event trigger support)");
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        let mut last_financial_check = Instant::now();
        let financial_enabled = self.financial_check_interval_secs > 0 && cost_tracker.is_some();
        if financial_enabled {
            info!(
                "Financial health check enabled (interval={}s)",
                self.financial_check_interval_secs
            );
        }

        loop {
            interval.tick().await;
            let now = Utc::now();

            // ── Financial health check ──────────────────────────────────
            if financial_enabled {
                let elapsed = last_financial_check.elapsed().as_secs();
                if elapsed >= self.financial_check_interval_secs {
                    if let Some(ref ct) = cost_tracker {
                        let rt_ref = revenue_tracker.as_deref();
                        run_financial_check(ct, rt_ref);
                    }
                    last_financial_check = Instant::now();
                }
            }

            // ── Event trigger evaluation ────────────────────────────────
            if let Some(ref tm) = trigger_manager {
                if let Ok(conn) = rusqlite::Connection::open(&db_path) {
                    let mut mgr = tm.lock().unwrap_or_else(|p| p.into_inner());
                    for trigger in &mut mgr.triggers {
                        if !trigger.should_evaluate() || !trigger.should_fire() {
                            continue;
                        }
                        trigger.last_evaluated = Some(Instant::now());
                        if let Ok(true) = trigger.condition.evaluate(&conn) {
                            trigger.last_fired = Some(Utc::now());
                            // Log trigger firing — actual action execution added in Task 13
                            tracing::info!(trigger_id = %trigger.id, "Event trigger fired");
                        }
                    }
                }
            }

            // Collect due jobs
            let due_jobs: Vec<CronJob> = {
                let jobs = self.jobs.read().await;
                jobs.iter()
                    .filter(|j| {
                        j.status == JobStatus::Active
                            && j.next_run.map(|nr| nr <= now).unwrap_or(false)
                    })
                    .cloned()
                    .collect()
            };

            for job in due_jobs {
                debug!("Executing cron job '{}' (id={})", job.name, job.id);

                let action = job.action.clone();
                let handle = executor(action);

                let result = match handle.await {
                    Ok(output) => output,
                    Err(e) => format!("Job execution error: {}", e),
                };

                let next_run = compute_next_run(&job.schedule, &now);
                let new_run_count = job.run_count + 1;

                let new_status = if matches!(job.schedule, Schedule::At { .. }) {
                    JobStatus::Completed
                } else if job.max_runs.map(|m| new_run_count >= m).unwrap_or(false) {
                    JobStatus::Completed
                } else if next_run.is_none() {
                    JobStatus::Completed
                } else {
                    JobStatus::Active
                };

                if let Err(e) = self.store.update_after_run(&job.id, &result, next_run, new_status) {
                    error!("Failed to update cron job '{}': {}", job.id, e);
                }

                let mut jobs = self.jobs.write().await;
                if let Some(j) = jobs.iter_mut().find(|j| j.id == job.id) {
                    j.last_run = Some(now);
                    j.last_result = Some(result.clone());
                    j.next_run = next_run;
                    j.run_count = new_run_count;
                    j.status = new_status;
                }

                info!("Cron job '{}' completed (status={:?}, next={:?})", job.name, new_status, next_run);
            }

            // Clean up completed jobs from in-memory list
            {
                let mut jobs = self.jobs.write().await;
                jobs.retain(|j| j.status == JobStatus::Active || j.status == JobStatus::Paused);
            }
        }
    }

    /// Main scheduler loop with optional periodic financial health checks.
    /// If `cost_tracker` is None, financial checks are skipped.
    pub async fn run_with_financial_check(
        &self,
        executor: JobExecutor,
        cost_tracker: Option<Arc<CostTracker>>,
        revenue_tracker: Option<Arc<RevenueTracker>>,
    ) {
        info!("Scheduler started");
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        let mut last_financial_check = Instant::now();
        // Run the first financial check soon after startup (after first tick)
        let financial_enabled = self.financial_check_interval_secs > 0 && cost_tracker.is_some();
        if financial_enabled {
            info!(
                "Financial health check enabled (interval={}s)",
                self.financial_check_interval_secs
            );
        }

        loop {
            interval.tick().await;
            let now = Utc::now();

            // ── Financial health check ──────────────────────────────────
            if financial_enabled {
                let elapsed = last_financial_check.elapsed().as_secs();
                if elapsed >= self.financial_check_interval_secs {
                    if let Some(ref ct) = cost_tracker {
                        let rt_ref = revenue_tracker.as_deref();
                        run_financial_check(ct, rt_ref);
                    }
                    last_financial_check = Instant::now();
                }
            }

            // Collect due jobs
            let due_jobs: Vec<CronJob> = {
                let jobs = self.jobs.read().await;
                jobs.iter()
                    .filter(|j| {
                        j.status == JobStatus::Active
                            && j.next_run.map(|nr| nr <= now).unwrap_or(false)
                    })
                    .cloned()
                    .collect()
            };

            for job in due_jobs {
                debug!("Executing cron job '{}' (id={})", job.name, job.id);

                // Execute the action
                let action = job.action.clone();
                let handle = executor(action);

                let result = match handle.await {
                    Ok(output) => output,
                    Err(e) => format!("Job execution error: {}", e),
                };

                // Compute next run
                let next_run = compute_next_run(&job.schedule, &now);
                let new_run_count = job.run_count + 1;

                // Check if job should complete
                let new_status = if matches!(job.schedule, Schedule::At { .. }) {
                    // One-shot jobs complete after running
                    JobStatus::Completed
                } else if job.max_runs.map(|m| new_run_count >= m).unwrap_or(false) {
                    // Hit max runs
                    JobStatus::Completed
                } else if next_run.is_none() {
                    // No next run possible
                    JobStatus::Completed
                } else {
                    JobStatus::Active
                };

                // Update store
                if let Err(e) = self.store.update_after_run(&job.id, &result, next_run, new_status) {
                    error!("Failed to update cron job '{}': {}", job.id, e);
                }

                // Update in-memory
                let mut jobs = self.jobs.write().await;
                if let Some(j) = jobs.iter_mut().find(|j| j.id == job.id) {
                    j.last_run = Some(now);
                    j.last_result = Some(result.clone());
                    j.next_run = next_run;
                    j.run_count = new_run_count;
                    j.status = new_status;
                }

                info!("Cron job '{}' completed (status={:?}, next={:?})", job.name, new_status, next_run);
            }

            // Clean up completed jobs from in-memory list
            {
                let mut jobs = self.jobs.write().await;
                jobs.retain(|j| j.status == JobStatus::Active || j.status == JobStatus::Paused);
            }
        }
    }
}

/// Compute the next run time for a schedule
fn compute_next_run(schedule: &Schedule, after: &DateTime<Utc>) -> Option<DateTime<Utc>> {
    match schedule {
        Schedule::Cron { expr } => next_cron_run(expr, *after),
        Schedule::At { at } => {
            if at > after { Some(*at) } else { None }
        }
        Schedule::Every { interval_secs } => {
            Some(*after + ChronoDuration::seconds(*interval_secs as i64))
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cron_field_star() {
        let vals = parse_cron_field("*", 0, 5).unwrap();
        assert_eq!(vals, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_parse_cron_field_step() {
        let vals = parse_cron_field("*/15", 0, 59).unwrap();
        assert_eq!(vals, vec![0, 15, 30, 45]);
    }

    #[test]
    fn test_parse_cron_field_range() {
        let vals = parse_cron_field("1-5", 0, 31).unwrap();
        assert_eq!(vals, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_parse_cron_field_list() {
        let vals = parse_cron_field("1,15,30", 0, 59).unwrap();
        assert_eq!(vals, vec![1, 15, 30]);
    }

    #[test]
    fn test_parse_cron_field_invalid() {
        assert!(parse_cron_field("*/0", 0, 59).is_none());
        assert!(parse_cron_field("60", 0, 59).is_none());
    }

    #[test]
    fn test_next_cron_run_every_hour() {
        // "0 * * * *" = at minute 0 of every hour
        let now = chrono::DateTime::parse_from_rfc3339("2026-03-02T14:30:00Z")
            .unwrap().with_timezone(&Utc);
        let next = next_cron_run("0 * * * *", now).unwrap();
        assert_eq!(next.hour(), 15);
        assert_eq!(next.minute(), 0);
    }

    #[test]
    fn test_next_cron_run_specific_time() {
        // "30 9 * * 1-5" = 9:30 on weekdays
        let now = chrono::DateTime::parse_from_rfc3339("2026-03-02T10:00:00Z")
            .unwrap().with_timezone(&Utc); // Monday
        let next = next_cron_run("30 9 * * 1-5", now).unwrap();
        // Next weekday 9:30 — should be Tuesday March 3
        assert_eq!(next.hour(), 9);
        assert_eq!(next.minute(), 30);
        assert_eq!(next.day(), 3);
    }

    #[test]
    fn test_next_cron_run_invalid_expr() {
        let now = Utc::now();
        assert!(next_cron_run("bad expr", now).is_none());
        assert!(next_cron_run("* * *", now).is_none()); // only 3 fields
    }

    #[test]
    fn test_compute_next_run_at_future() {
        let future = Utc::now() + ChronoDuration::hours(1);
        let next = compute_next_run(&Schedule::At { at: future }, &Utc::now());
        assert!(next.is_some());
    }

    #[test]
    fn test_compute_next_run_at_past() {
        let past = Utc::now() - ChronoDuration::hours(1);
        let next = compute_next_run(&Schedule::At { at: past }, &Utc::now());
        assert!(next.is_none());
    }

    #[test]
    fn test_compute_next_run_every() {
        let now = Utc::now();
        let next = compute_next_run(&Schedule::Every { interval_secs: 300 }, &now).unwrap();
        let diff = (next - now).num_seconds();
        assert_eq!(diff, 300);
    }

    #[test]
    fn test_cron_store_roundtrip() {
        let dir = std::env::temp_dir().join("phantom_mesh_test_cron");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join("test_cron.db");
        let _ = std::fs::remove_file(&db_path);

        let store = CronStore::new(db_path.to_str().unwrap()).unwrap();

        let job = CronJob {
            id: "test-1".to_string(),
            name: "Test Job".to_string(),
            schedule: Schedule::Every { interval_secs: 60 },
            action: JobAction::Shell { command: "echo hello".to_string() },
            status: JobStatus::Active,
            next_run: Some(Utc::now() + ChronoDuration::minutes(1)),
            last_run: None,
            last_result: None,
            run_count: 0,
            max_runs: None,
            created_at: Utc::now(),
        };

        store.save_job(&job).unwrap();
        let loaded = store.load_active_jobs().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "test-1");
        assert_eq!(loaded[0].name, "Test Job");

        // Clean up
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_cron_store_delete() {
        let dir = std::env::temp_dir().join("phantom_mesh_test_cron_del");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join("test_cron_del.db");
        let _ = std::fs::remove_file(&db_path);

        let store = CronStore::new(db_path.to_str().unwrap()).unwrap();

        let job = CronJob {
            id: "del-1".to_string(),
            name: "Delete Me".to_string(),
            schedule: Schedule::At { at: Utc::now() + ChronoDuration::hours(1) },
            action: JobAction::Notify { chat_id: "123".to_string(), message: "hi".to_string() },
            status: JobStatus::Active,
            next_run: Some(Utc::now() + ChronoDuration::hours(1)),
            last_run: None,
            last_result: None,
            run_count: 0,
            max_runs: Some(1),
            created_at: Utc::now(),
        };

        store.save_job(&job).unwrap();
        assert!(store.delete_job("del-1").unwrap());
        assert!(!store.delete_job("nonexistent").unwrap());
        assert_eq!(store.load_active_jobs().unwrap().len(), 0);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_job_action_hand_serialization() {
        let action = JobAction::Hand {
            hand_name: "freelancer".to_string(),
            input: "AI automation jobs on Upwork".to_string(),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("\"type\":\"hand\""));
        assert!(json.contains("freelancer"));

        let parsed: JobAction = serde_json::from_str(&json).unwrap();
        match parsed {
            JobAction::Hand { hand_name, input } => {
                assert_eq!(hand_name, "freelancer");
                assert_eq!(input, "AI automation jobs on Upwork");
            }
            _ => panic!("Expected Hand variant"),
        }
    }

    #[test]
    fn test_cron_store_hand_job() {
        let dir = std::env::temp_dir().join("phantom_mesh_test_cron_hand");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join("test_cron_hand.db");
        let _ = std::fs::remove_file(&db_path);

        let store = CronStore::new(db_path.to_str().unwrap()).unwrap();

        let job = CronJob {
            id: "hand-1".to_string(),
            name: "Daily Freelancer".to_string(),
            schedule: Schedule::Cron { expr: "0 9 * * *".to_string() },
            action: JobAction::Hand {
                hand_name: "freelancer".to_string(),
                input: "AI automation jobs".to_string(),
            },
            status: JobStatus::Active,
            next_run: Some(Utc::now() + ChronoDuration::hours(1)),
            last_run: None,
            last_result: None,
            run_count: 0,
            max_runs: None,
            created_at: Utc::now(),
        };

        store.save_job(&job).unwrap();
        let loaded = store.load_active_jobs().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "Daily Freelancer");
        match &loaded[0].action {
            JobAction::Hand { hand_name, input } => {
                assert_eq!(hand_name, "freelancer");
                assert_eq!(input, "AI automation jobs");
            }
            _ => panic!("Expected Hand action"),
        }

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_job_action_all_variants_serialize() {
        // Ensure all 4 variants roundtrip
        let actions = vec![
            JobAction::Shell { command: "echo hi".to_string() },
            JobAction::Agent { agent: "master".to_string(), prompt: "hello".to_string() },
            JobAction::Notify { chat_id: "123".to_string(), message: "test".to_string() },
            JobAction::Hand { hand_name: "lead".to_string(), input: "SaaS companies".to_string() },
        ];
        for action in actions {
            let json = serde_json::to_string(&action).unwrap();
            let parsed: JobAction = serde_json::from_str(&json).unwrap();
            assert_eq!(
                serde_json::to_string(&parsed).unwrap(),
                json
            );
        }
    }

    // ── Financial health check tests ──────────────────────────────────────────

    /// Helper: create a temporary CostTracker backed by an in-memory-style temp DB.
    fn temp_cost_tracker(name: &str) -> (CostTracker, std::path::PathBuf) {
        let dir = std::env::temp_dir().join("phantom_mesh_test_cron_fin");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join(format!("{}.db", name));
        let _ = std::fs::remove_file(&db_path);
        let tracker = CostTracker::new(db_path.to_str().unwrap()).unwrap();
        (tracker, db_path)
    }

    #[test]
    fn test_financial_check_healthy_no_alerts() {
        // With zero spend (empty DB) and default $10 limit, no alerts should fire
        // for daily_spend or budget_utilization (both 0%).
        // api_revenue_ratio: api_cost=0 → skip. project_margin: revenue=0,cost=0 → skip.
        // cash_runway: burn=0 → skip. cost_spike: avg=0 → skip. revenue_decline: prev=0 → skip.
        let (tracker, db_path) = temp_cost_tracker("fin_healthy");
        let alerts = run_financial_check(&tracker, None);
        assert!(
            alerts.is_empty(),
            "Expected no alerts for empty tracker, got {} alert(s): {:?}",
            alerts.len(),
            alerts.iter().map(|a| &a.message).collect::<Vec<_>>()
        );
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_financial_check_high_spend_triggers_alerts() {
        // Record high spending that exceeds the default $10 daily limit
        let (tracker, db_path) = temp_cost_tracker("fin_high_spend");
        let record = crate::cost_tracker::CostRecord {
            id: "fin-test-1".to_string(),
            timestamp: Utc::now(),
            agent: "master".to_string(),
            provider: "anthropic".to_string(),
            model: "claude-opus".to_string(),
            tokens_in: 50_000,
            tokens_out: 50_000,
            total_tokens: 100_000,
            node_id: Some("local".to_string()),
            api_estimated_cost_usd: 12.0,
            hardware_estimated_cost_usd: 0.0,
            estimated_cost_usd: 12.0, // $12 > $10 limit → Emergency
            duration_secs: 5.0,
            context: None,
        };
        tracker.record(&record).unwrap();

        let alerts = run_financial_check(&tracker, None);
        // Should have at least a daily_spend alert (Emergency: exceeded limit)
        // and a budget_utilization alert (Emergency: exceeded budget)
        assert!(
            !alerts.is_empty(),
            "Expected alerts for $12 spend on $10 limit, got none"
        );

        // Check that we have an Emergency-level daily_spend alert
        let daily_alert = alerts.iter().find(|a| a.indicator_name == "daily_spend");
        assert!(daily_alert.is_some(), "Expected a daily_spend alert");
        assert_eq!(
            daily_alert.unwrap().level,
            AlertLevel::Emergency,
            "daily_spend with $12/$10 should be Emergency"
        );

        // Check that we have an Emergency-level budget_utilization alert
        let budget_alert = alerts.iter().find(|a| a.indicator_name == "budget_utilization");
        assert!(budget_alert.is_some(), "Expected a budget_utilization alert");
        assert_eq!(
            budget_alert.unwrap().level,
            AlertLevel::Emergency,
            "budget $12/$10 should be Emergency"
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_financial_check_warn_level_spend() {
        // Record spending at 85% of limit ($8.50 / $10) → Warn level
        let (tracker, db_path) = temp_cost_tracker("fin_warn_spend");
        let record = crate::cost_tracker::CostRecord {
            id: "fin-test-2".to_string(),
            timestamp: Utc::now(),
            agent: "master".to_string(),
            provider: "anthropic".to_string(),
            model: "claude-sonnet".to_string(),
            tokens_in: 10_000,
            tokens_out: 10_000,
            total_tokens: 20_000,
            node_id: Some("local".to_string()),
            api_estimated_cost_usd: 8.5,
            hardware_estimated_cost_usd: 0.0,
            estimated_cost_usd: 8.5, // 85% of $10 → Warn
            duration_secs: 2.0,
            context: None,
        };
        tracker.record(&record).unwrap();

        let alerts = run_financial_check(&tracker, None);
        assert!(!alerts.is_empty(), "Expected alerts for $8.50 on $10 limit");

        let daily_alert = alerts.iter().find(|a| a.indicator_name == "daily_spend");
        assert!(daily_alert.is_some(), "Expected a daily_spend alert");
        assert_eq!(
            daily_alert.unwrap().level,
            AlertLevel::Warn,
            "daily_spend $8.50/$10 (85%) should be Warn"
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_financial_check_alert_levels_are_correct() {
        // Verify the logging dispatch logic by checking returned alert levels
        // at different spend amounts against the default $10 limit.
        let test_cases: Vec<(f64, Option<AlertLevel>)> = vec![
            (5.0, None),                              // 50% → no alert
            (8.0, Some(AlertLevel::Warn)),             // 80% → Warn
            (9.6, Some(AlertLevel::Critical)),         // 96% → Critical
            (11.0, Some(AlertLevel::Emergency)),       // 110% → Emergency
        ];

        for (spend, expected_level) in test_cases {
            let db_name = format!("fin_levels_{}", (spend * 10.0) as u32);
            let (tracker, db_path) = temp_cost_tracker(&db_name);

            if spend > 0.0 {
                let record = crate::cost_tracker::CostRecord {
                    id: format!("fin-level-{}", (spend * 10.0) as u32),
                    timestamp: Utc::now(),
                    agent: "master".to_string(),
                    provider: "anthropic".to_string(),
                    model: "claude-sonnet".to_string(),
                    tokens_in: 1000,
                    tokens_out: 1000,
                    total_tokens: 2000,
                    node_id: Some("local".to_string()),
                    api_estimated_cost_usd: spend,
                    hardware_estimated_cost_usd: 0.0,
                    estimated_cost_usd: spend,
                    duration_secs: 1.0,
                    context: None,
                };
                tracker.record(&record).unwrap();
            }

            let alerts = run_financial_check(&tracker, None);
            let daily_alert = alerts.iter().find(|a| a.indicator_name == "daily_spend");

            match expected_level {
                None => {
                    assert!(
                        daily_alert.is_none(),
                        "Expected no daily_spend alert for ${:.1}, but got one",
                        spend
                    );
                }
                Some(ref level) => {
                    assert!(
                        daily_alert.is_some(),
                        "Expected daily_spend alert for ${:.1}, but got none",
                        spend
                    );
                    assert_eq!(
                        &daily_alert.unwrap().level,
                        level,
                        "Wrong alert level for ${:.1} spend",
                        spend
                    );
                }
            }

            let _ = std::fs::remove_file(&db_path);
        }
    }

    #[tokio::test]
    async fn test_tick_now_fires_due_jobs() {
        let dir = std::env::temp_dir().join("phantom_mesh_test_tick_now");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join("cron.db");
        let store = Arc::new(CronStore::new(db_path.to_str().unwrap()).unwrap());
        let scheduler = Scheduler::new(store).unwrap();

        // Add a job that's due immediately (Every 1 second)
        scheduler.add_job(
            "test-job",
            Schedule::Every { interval_secs: 1 },
            JobAction::Shell { command: "echo hello".to_string() },
            None,
        ).await.unwrap();

        // Wait a moment so the job becomes due
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

        let call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cc = call_count.clone();
        let executor: JobExecutor = Arc::new(move |_action| {
            cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::spawn(async { "ok".to_string() })
        });

        let triggered = scheduler.tick_now(&executor).await;
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered[0], "test-job");
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_tick_now_skips_paused_jobs() {
        let dir = std::env::temp_dir().join("phantom_mesh_test_tick_paused");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join("cron.db");
        let store = Arc::new(CronStore::new(db_path.to_str().unwrap()).unwrap());
        let scheduler = Scheduler::new(store).unwrap();

        let id = scheduler.add_job(
            "paused-job",
            Schedule::Every { interval_secs: 1 },
            JobAction::Shell { command: "echo hi".to_string() },
            None,
        ).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        scheduler.pause_job(&id).await.unwrap();

        let executor: JobExecutor = Arc::new(|_| tokio::spawn(async { "ok".to_string() }));
        let triggered = scheduler.tick_now(&executor).await;
        assert!(triggered.is_empty(), "Paused jobs should not fire");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
