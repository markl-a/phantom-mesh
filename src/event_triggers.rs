//! Event-driven reminder triggers — condition-based firing with cooldown.
//!
//! Each `TriggerCondition` evaluates against the SQLite database.
//! `EventTriggerManager` bootstraps default triggers from `UserProfile.alert_thresholds`
//! and is invoked from the cron tick loop.

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use crate::cron::JobAction;
use crate::user_profile::UserProfile;

// ── TriggerCondition ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerCondition {
    DeadlineApproaching { days_before: u32 },
    StreakBroken { missed_days: u32 },
    BudgetExceeded { percent: f64 },
    TaskFailureStreak { count: u32 },
    UserIdle { days: u32 },
}

impl TriggerCondition {
    /// Evaluate the condition against the database.
    pub fn evaluate(&self, conn: &Connection) -> Result<bool> {
        match self {
            TriggerCondition::DeadlineApproaching { days_before } => {
                let threshold = Utc::now() + chrono::Duration::days(*days_before as i64);
                let threshold_str = threshold.format("%Y-%m-%dT%H:%M:%S").to_string();
                let now_str = Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM (SELECT 1 FROM goals WHERE status = 'active' AND target_date IS NOT NULL AND target_date <= ?1 AND target_date > ?2)",
                    params![threshold_str, now_str],
                    |row| row.get(0),
                ).unwrap_or(0);
                Ok(count > 0)
            }
            TriggerCondition::TaskFailureStreak { count } => {
                let total_recent: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM (SELECT status FROM task_queue ORDER BY created_at DESC LIMIT ?1)",
                    params![count],
                    |row| row.get(0),
                ).unwrap_or(0);
                if total_recent < *count as i64 {
                    return Ok(false);
                }
                let recent_failures: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM (SELECT status FROM task_queue ORDER BY created_at DESC LIMIT ?1) WHERE status = 'failed'",
                    params![count],
                    |row| row.get(0),
                ).unwrap_or(0);
                Ok(recent_failures >= *count as i64)
            }
            TriggerCondition::StreakBroken { .. } => Ok(false), // requires recurring_tasks table
            TriggerCondition::BudgetExceeded { .. } => Ok(false), // requires cost_tracker integration
            TriggerCondition::UserIdle { .. } => Ok(false), // requires last interaction timestamp
        }
    }
}

// ── EventTrigger ──────────────────────────────────────────────────────

pub struct EventTrigger {
    pub id: String,
    pub condition: TriggerCondition,
    pub action: JobAction,
    pub cooldown_secs: u64,
    pub last_fired: Option<DateTime<Utc>>,
    pub enabled: bool,
    pub last_evaluated: Option<Instant>,
    pub check_interval_secs: u64,
}

impl EventTrigger {
    /// Check if this trigger should fire (enabled + cooldown expired).
    pub fn should_fire(&self) -> bool {
        if !self.enabled {
            return false;
        }
        if let Some(last) = self.last_fired {
            let elapsed = (Utc::now() - last).num_seconds().max(0) as u64;
            elapsed >= self.cooldown_secs
        } else {
            true
        }
    }

    /// Check if enough time has passed since last evaluation.
    pub fn should_evaluate(&self) -> bool {
        if !self.enabled {
            return false;
        }
        if let Some(last) = self.last_evaluated {
            last.elapsed().as_secs() >= self.check_interval_secs
        } else {
            true
        }
    }
}

// ── EventTriggerManager ───────────────────────────────────────────────

pub struct EventTriggerManager {
    pub triggers: Vec<EventTrigger>,
    pub profile: Arc<RwLock<UserProfile>>,
}

impl EventTriggerManager {
    pub fn new(triggers: Vec<EventTrigger>, profile: Arc<RwLock<UserProfile>>) -> Self {
        Self { triggers, profile }
    }

    /// Create the event_triggers table if it does not exist.
    pub fn create_table(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS event_triggers (
                id TEXT PRIMARY KEY,
                condition_json TEXT NOT NULL,
                action_json TEXT NOT NULL,
                cooldown_secs INTEGER NOT NULL,
                last_fired TEXT,
                enabled INTEGER NOT NULL DEFAULT 1
            );"
        )?;
        Ok(())
    }

    /// Bootstrap 5 default triggers from UserProfile thresholds.
    pub fn bootstrap_defaults(conn: &Connection, profile: &UserProfile) -> Result<()> {
        let defaults: Vec<(&str, TriggerCondition, u64, u64)> = vec![
            ("deadline_approaching", TriggerCondition::DeadlineApproaching { days_before: profile.alert_thresholds.deadline_warn_days }, 86400, 1800),
            ("streak_broken", TriggerCondition::StreakBroken { missed_days: profile.alert_thresholds.streak_break_days }, 43200, 1800),
            ("budget_exceeded", TriggerCondition::BudgetExceeded { percent: profile.alert_thresholds.budget_warn_percent }, 3600, 300),
            ("task_failure_streak", TriggerCondition::TaskFailureStreak { count: profile.alert_thresholds.task_failure_count }, 1800, 30),
            ("user_idle", TriggerCondition::UserIdle { days: profile.alert_thresholds.idle_days }, 259200, 21600),
        ];

        for (id, condition, cooldown, _interval) in defaults {
            let condition_json = serde_json::to_string(&condition)?;
            let action = JobAction::Notify {
                chat_id: "default".to_string(),
                message: format!("Event trigger: {}", id),
            };
            let action_json = serde_json::to_string(&action)?;
            conn.execute(
                "INSERT OR IGNORE INTO event_triggers (id, condition_json, action_json, cooldown_secs, enabled) VALUES (?1, ?2, ?3, ?4, 1)",
                params![id, condition_json, action_json, cooldown as i64],
            )?;
        }
        Ok(())
    }

    /// Load triggers from SQLite.
    pub fn load_triggers(conn: &Connection) -> Result<Vec<EventTrigger>> {
        let mut stmt = conn.prepare(
            "SELECT id, condition_json, action_json, cooldown_secs, last_fired, enabled FROM event_triggers"
        )?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let condition_json: String = row.get(1)?;
            let action_json: String = row.get(2)?;
            let cooldown_secs: i64 = row.get(3)?;
            let last_fired: Option<String> = row.get(4)?;
            let enabled: bool = row.get(5)?;
            Ok((id, condition_json, action_json, cooldown_secs as u64, last_fired, enabled))
        })?.collect::<std::result::Result<Vec<_>, _>>()?;

        let mut result = Vec::new();
        for (id, cond_json, act_json, cooldown, last_fired_str, enabled) in rows {
            let condition: TriggerCondition = serde_json::from_str(&cond_json)?;
            let action: JobAction = serde_json::from_str(&act_json)?;
            let last_fired = last_fired_str
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc)));

            result.push(EventTrigger {
                id,
                condition,
                action,
                cooldown_secs: cooldown,
                last_fired,
                enabled,
                last_evaluated: None,
                check_interval_secs: 30,
            });
        }
        Ok(result)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS goals (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                target_date TEXT,
                status TEXT DEFAULT 'active'
            );
            CREATE TABLE IF NOT EXISTS task_queue (
                id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
        ").unwrap();
        EventTriggerManager::create_table(&conn).unwrap();
        conn
    }

    #[test]
    fn test_deadline_approaching_true() {
        let conn = test_db();
        let deadline = (Utc::now() + chrono::Duration::days(2)).format("%Y-%m-%dT%H:%M:%S").to_string();
        conn.execute(
            "INSERT INTO goals (id, title, target_date, status) VALUES ('g1', 'Test Goal', ?1, 'active')",
            params![deadline],
        ).unwrap();

        let condition = TriggerCondition::DeadlineApproaching { days_before: 3 };
        assert!(condition.evaluate(&conn).unwrap());
    }

    #[test]
    fn test_deadline_approaching_false() {
        let conn = test_db();
        let deadline = (Utc::now() + chrono::Duration::days(10)).format("%Y-%m-%dT%H:%M:%S").to_string();
        conn.execute(
            "INSERT INTO goals (id, title, target_date, status) VALUES ('g1', 'Far Goal', ?1, 'active')",
            params![deadline],
        ).unwrap();

        let condition = TriggerCondition::DeadlineApproaching { days_before: 3 };
        assert!(!condition.evaluate(&conn).unwrap());
    }

    #[test]
    fn test_task_failure_streak_true() {
        let conn = test_db();
        let now = Utc::now().to_rfc3339();
        for i in 0..5 {
            conn.execute(
                "INSERT INTO task_queue (id, status, created_at) VALUES (?1, 'failed', ?2)",
                params![format!("t{}", i), now],
            ).unwrap();
        }

        let condition = TriggerCondition::TaskFailureStreak { count: 3 };
        assert!(condition.evaluate(&conn).unwrap());
    }

    #[test]
    fn test_task_failure_streak_false() {
        let conn = test_db();
        let now = Utc::now().to_rfc3339();
        conn.execute("INSERT INTO task_queue (id, status, created_at) VALUES ('t1', 'failed', ?1)", params![now]).unwrap();
        conn.execute("INSERT INTO task_queue (id, status, created_at) VALUES ('t2', 'failed', ?1)", params![now]).unwrap();

        let condition = TriggerCondition::TaskFailureStreak { count: 3 };
        assert!(!condition.evaluate(&conn).unwrap());
    }

    #[test]
    fn test_cooldown_prevents_refire() {
        let trigger = EventTrigger {
            id: "test".to_string(),
            condition: TriggerCondition::TaskFailureStreak { count: 1 },
            action: JobAction::Notify { chat_id: "123".to_string(), message: "alert".to_string() },
            cooldown_secs: 3600,
            last_fired: Some(Utc::now()),
            enabled: true,
            last_evaluated: None,
            check_interval_secs: 30,
        };
        assert!(!trigger.should_fire());
    }

    #[test]
    fn test_cooldown_expired_allows_fire() {
        let trigger = EventTrigger {
            id: "test".to_string(),
            condition: TriggerCondition::TaskFailureStreak { count: 1 },
            action: JobAction::Notify { chat_id: "123".to_string(), message: "alert".to_string() },
            cooldown_secs: 0,
            last_fired: Some(Utc::now() - chrono::Duration::seconds(10)),
            enabled: true,
            last_evaluated: None,
            check_interval_secs: 30,
        };
        assert!(trigger.should_fire());
    }

    #[test]
    fn test_disabled_trigger_never_fires() {
        let trigger = EventTrigger {
            id: "test".to_string(),
            condition: TriggerCondition::TaskFailureStreak { count: 1 },
            action: JobAction::Notify { chat_id: "123".to_string(), message: "alert".to_string() },
            cooldown_secs: 0,
            last_fired: None,
            enabled: false,
            last_evaluated: None,
            check_interval_secs: 30,
        };
        assert!(!trigger.should_fire());
    }

    #[test]
    fn test_frequency_control_skips_early() {
        let trigger = EventTrigger {
            id: "test".to_string(),
            condition: TriggerCondition::DeadlineApproaching { days_before: 3 },
            action: JobAction::Notify { chat_id: "123".to_string(), message: "deadline".to_string() },
            cooldown_secs: 86400,
            last_fired: None,
            enabled: true,
            last_evaluated: Some(Instant::now()), // Just evaluated
            check_interval_secs: 1800,
        };
        assert!(!trigger.should_evaluate());
    }

    #[test]
    fn test_bootstrap_creates_defaults() {
        let conn = test_db();
        let profile = UserProfile::default();
        EventTriggerManager::bootstrap_defaults(&conn, &profile).unwrap();

        let count: i64 = conn.query_row("SELECT COUNT(*) FROM event_triggers", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 5);
    }

    #[test]
    fn test_sqlite_persistence_roundtrip() {
        let conn = test_db();
        let profile = UserProfile::default();
        EventTriggerManager::bootstrap_defaults(&conn, &profile).unwrap();

        let triggers = EventTriggerManager::load_triggers(&conn).unwrap();
        assert_eq!(triggers.len(), 5);
        assert!(triggers.iter().all(|t| t.enabled));
    }

    #[test]
    fn test_trigger_evaluate_and_fire_cycle() {
        let conn = test_db();
        let deadline = (Utc::now() + chrono::Duration::days(2)).format("%Y-%m-%dT%H:%M:%S").to_string();
        conn.execute(
            "INSERT INTO goals (id, title, target_date, status) VALUES ('g1', 'Ship v0.1', ?1, 'active')",
            params![deadline],
        ).unwrap();

        let mut trigger = EventTrigger {
            id: "deadline_test".to_string(),
            condition: TriggerCondition::DeadlineApproaching { days_before: 3 },
            action: JobAction::Notify { chat_id: "123".to_string(), message: "deadline".to_string() },
            cooldown_secs: 86400,
            last_fired: None,
            enabled: true,
            last_evaluated: None,
            check_interval_secs: 0,
        };

        assert!(trigger.should_evaluate());
        assert!(trigger.should_fire());
        assert!(trigger.condition.evaluate(&conn).unwrap());

        trigger.last_fired = Some(Utc::now());
        trigger.last_evaluated = Some(Instant::now());

        assert!(!trigger.should_fire());
    }
}
