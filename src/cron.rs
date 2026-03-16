// Cron/Scheduling system — inspired by ZeroClaw's cron module + OpenCrust's heartbeat
// Supports: cron expressions, one-shot (at), fixed interval (every)
// Persisted in SQLite, executed by background tokio task

use anyhow::Result;
use chrono::{DateTime, Utc, Duration as ChronoDuration, Timelike, Datelike};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

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
    db_path: String,
}

impl CronStore {
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = rusqlite::Connection::open(db_path)?;
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
        Ok(Self { db_path: db_path.to_string() })
    }

    pub fn save_job(&self, job: &CronJob) -> Result<()> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        conn.execute(
            "INSERT OR REPLACE INTO cron_jobs
             (id, name, schedule_json, action_json, status, next_run, last_run, last_result, run_count, max_runs, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                job.id,
                job.name,
                serde_json::to_string(&job.schedule)?,
                serde_json::to_string(&job.action)?,
                serde_json::to_string(&job.status)?,
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
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT id, name, schedule_json, action_json, status, next_run, last_run, last_result, run_count, max_runs, created_at
             FROM cron_jobs WHERE status = '\"active\"'"
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
                status: serde_json::from_str(&status_str).unwrap_or(JobStatus::Active),
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
        let conn = rusqlite::Connection::open(&self.db_path)?;
        conn.execute(
            "UPDATE cron_jobs SET last_run = ?1, last_result = ?2, next_run = ?3, status = ?4, run_count = run_count + 1
             WHERE id = ?5",
            rusqlite::params![
                Utc::now().to_rfc3339(),
                result,
                next_run.map(|t| t.to_rfc3339()),
                serde_json::to_string(&new_status)?,
                job_id,
            ],
        )?;
        Ok(())
    }

    pub fn delete_job(&self, job_id: &str) -> Result<bool> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let rows = conn.execute("DELETE FROM cron_jobs WHERE id = ?1", [job_id])?;
        Ok(rows > 0)
    }

    pub fn list_all(&self) -> Result<Vec<CronJob>> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
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
                status: serde_json::from_str(&status_str).unwrap_or(JobStatus::Active),
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

// ── Scheduler (background runner) ────────────────────────────────────────────

/// Callback type for executing job actions
pub type JobExecutor = Arc<dyn Fn(JobAction) -> tokio::task::JoinHandle<String> + Send + Sync>;

/// The scheduler runs in the background, checking for due jobs every 30 seconds
pub struct Scheduler {
    store: Arc<CronStore>,
    jobs: RwLock<Vec<CronJob>>,
}

impl Scheduler {
    pub fn new(store: Arc<CronStore>) -> Result<Self> {
        let jobs = store.load_active_jobs()?;
        info!("Scheduler loaded {} active jobs", jobs.len());
        Ok(Self {
            store,
            jobs: RwLock::new(jobs),
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

    /// Main scheduler loop — call this in a tokio::spawn
    pub async fn run(&self, executor: JobExecutor) {
        info!("Scheduler started");
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));

        loop {
            interval.tick().await;
            let now = Utc::now();

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
        let dir = std::env::temp_dir().join("clawtex_test_cron");
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
        let dir = std::env::temp_dir().join("clawtex_test_cron_del");
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
        let dir = std::env::temp_dir().join("clawtex_test_cron_hand");
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
}
