//! Goals module — persistent goal tracking with milestones, recurring tasks, and check-ins.

use anyhow::Result;
use chrono::{Utc, NaiveDate};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

// ── Status Enums ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Completed,
    Paused,
    Abandoned,
}

impl GoalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Paused => "paused",
            Self::Abandoned => "abandoned",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "completed" => Self::Completed,
            "paused" => Self::Paused,
            "abandoned" => Self::Abandoned,
            _ => Self::Active,
        }
    }
}

// ── Data Types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub title: String,
    pub category: String,
    pub description: Option<String>,
    pub target_date: Option<String>,
    pub status: GoalStatus,
    pub context: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub id: String,
    pub goal_id: String,
    pub title: String,
    pub due_date: Option<String>,
    pub status: String, // "pending" | "done"
    pub sort_order: i32,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecurringTask {
    pub id: String,
    pub goal_id: String,
    pub title: String,
    pub cron_expr: String,
    pub last_completed: Option<String>,
    pub streak_count: i32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckIn {
    pub id: String,
    pub goal_id: String,
    pub date: String,
    pub mood: i32,
    pub note: Option<String>,
    pub ai_feedback: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalProgress {
    pub goal: Goal,
    pub milestones_total: i32,
    pub milestones_done: i32,
    pub percentage: f64,
    pub current_streak: i32,
    pub days_remaining: Option<i64>,
    pub recent_check_ins: Vec<CheckIn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodayTask {
    pub task: RecurringTask,
    pub goal_title: String,
    pub completed_today: bool,
}

/// Daily completion snapshot for progress charts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyStats {
    pub date: String,
    pub tasks_total: i32,
    pub tasks_done: i32,
    pub avg_mood: Option<f64>,
}

/// Weekly summary data for reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeeklySummary {
    pub week_start: String,
    pub week_end: String,
    pub total_tasks: i32,
    pub completed_tasks: i32,
    pub completion_rate: f64,
    pub avg_mood: Option<f64>,
    pub best_streak: i32,
    pub milestones_completed: i32,
}

// ── GoalsStore ─────────────────────────────────────────────────────────────

pub struct GoalsStore {
    db_path: String,
}

impl GoalsStore {
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = rusqlite::Connection::open(db_path)?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS goals (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                category TEXT NOT NULL DEFAULT '',
                description TEXT,
                target_date TEXT,
                status TEXT NOT NULL DEFAULT 'active',
                context TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_goals_status ON goals(status);

            CREATE TABLE IF NOT EXISTS milestones (
                id TEXT PRIMARY KEY,
                goal_id TEXT NOT NULL,
                title TEXT NOT NULL,
                due_date TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                sort_order INTEGER NOT NULL DEFAULT 0,
                completed_at TEXT,
                FOREIGN KEY (goal_id) REFERENCES goals(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_milestones_goal ON milestones(goal_id);

            CREATE TABLE IF NOT EXISTS recurring_tasks (
                id TEXT PRIMARY KEY,
                goal_id TEXT NOT NULL,
                title TEXT NOT NULL,
                cron_expr TEXT NOT NULL DEFAULT '0 9 * * *',
                last_completed TEXT,
                streak_count INTEGER NOT NULL DEFAULT 0,
                enabled INTEGER NOT NULL DEFAULT 1,
                FOREIGN KEY (goal_id) REFERENCES goals(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_tasks_goal ON recurring_tasks(goal_id);

            CREATE TABLE IF NOT EXISTS check_ins (
                id TEXT PRIMARY KEY,
                goal_id TEXT NOT NULL,
                date TEXT NOT NULL,
                mood INTEGER NOT NULL DEFAULT 3,
                note TEXT,
                ai_feedback TEXT,
                FOREIGN KEY (goal_id) REFERENCES goals(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_checkins_goal ON check_ins(goal_id);
            CREATE INDEX IF NOT EXISTS idx_checkins_date ON check_ins(date);
        ")?;

        // Enable foreign keys
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        info!("GoalsStore initialized: {}", db_path);
        Ok(Self { db_path: db_path.to_string() })
    }

    fn conn(&self) -> Result<rusqlite::Connection> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(conn)
    }

    // ── Goal CRUD ──────────────────────────────────────────────────────

    pub fn create_goal(&self, goal: &Goal) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO goals (id, title, category, description, target_date, status, context, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                goal.id, goal.title, goal.category, goal.description,
                goal.target_date, goal.status.as_str(), goal.context,
                goal.created_at, goal.updated_at,
            ],
        )?;
        debug!("Goal created: {} ({})", goal.title, goal.id);
        Ok(())
    }

    pub fn get_goal(&self, id: &str) -> Result<Option<Goal>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, category, description, target_date, status, context, created_at, updated_at
             FROM goals WHERE id = ?1"
        )?;
        let goal = stmt.query_row([id], |row| {
            Ok(Goal {
                id: row.get(0)?,
                title: row.get(1)?,
                category: row.get(2)?,
                description: row.get(3)?,
                target_date: row.get(4)?,
                status: GoalStatus::from_str(&row.get::<_, String>(5)?),
                context: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        });
        match goal {
            Ok(g) => Ok(Some(g)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list_goals(&self, status: Option<GoalStatus>) -> Result<Vec<Goal>> {
        let conn = self.conn()?;
        let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match status {
            Some(s) => (
                "SELECT id, title, category, description, target_date, status, context, created_at, updated_at
                 FROM goals WHERE status = ?1 ORDER BY created_at DESC",
                vec![Box::new(s.as_str().to_string())],
            ),
            None => (
                "SELECT id, title, category, description, target_date, status, context, created_at, updated_at
                 FROM goals ORDER BY created_at DESC",
                vec![],
            ),
        };
        let mut stmt = conn.prepare(sql)?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let goals = stmt.query_map(params_ref.as_slice(), |row| {
            Ok(Goal {
                id: row.get(0)?,
                title: row.get(1)?,
                category: row.get(2)?,
                description: row.get(3)?,
                target_date: row.get(4)?,
                status: GoalStatus::from_str(&row.get::<_, String>(5)?),
                context: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })?.filter_map(|r| r.ok()).collect();
        Ok(goals)
    }

    pub fn update_goal(&self, id: &str, title: Option<&str>, status: Option<GoalStatus>, description: Option<&str>, context: Option<&str>) -> Result<bool> {
        let conn = self.conn()?;
        let now = Utc::now().to_rfc3339();
        let mut sets = vec!["updated_at = ?1".to_string()];
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now)];
        let mut idx = 2;

        if let Some(t) = title {
            sets.push(format!("title = ?{}", idx));
            params.push(Box::new(t.to_string()));
            idx += 1;
        }
        if let Some(s) = status {
            sets.push(format!("status = ?{}", idx));
            params.push(Box::new(s.as_str().to_string()));
            idx += 1;
        }
        if let Some(d) = description {
            sets.push(format!("description = ?{}", idx));
            params.push(Box::new(d.to_string()));
            idx += 1;
        }
        if let Some(c) = context {
            sets.push(format!("context = ?{}", idx));
            params.push(Box::new(c.to_string()));
            idx += 1;
        }

        let sql = format!("UPDATE goals SET {} WHERE id = ?{}", sets.join(", "), idx);
        params.push(Box::new(id.to_string()));
        let params_ref: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let rows = conn.execute(&sql, params_ref.as_slice())?;
        Ok(rows > 0)
    }

    pub fn delete_goal(&self, id: &str) -> Result<bool> {
        let conn = self.conn()?;
        let rows = conn.execute("DELETE FROM goals WHERE id = ?1", [id])?;
        Ok(rows > 0)
    }

    // ── Milestone CRUD ─────────────────────────────────────────────────

    pub fn add_milestone(&self, ms: &Milestone) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO milestones (id, goal_id, title, due_date, status, sort_order, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![ms.id, ms.goal_id, ms.title, ms.due_date, ms.status, ms.sort_order, ms.completed_at],
        )?;
        Ok(())
    }

    pub fn list_milestones(&self, goal_id: &str) -> Result<Vec<Milestone>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, goal_id, title, due_date, status, sort_order, completed_at
             FROM milestones WHERE goal_id = ?1 ORDER BY sort_order"
        )?;
        let ms = stmt.query_map([goal_id], |row| {
            Ok(Milestone {
                id: row.get(0)?, goal_id: row.get(1)?, title: row.get(2)?,
                due_date: row.get(3)?, status: row.get(4)?, sort_order: row.get(5)?,
                completed_at: row.get(6)?,
            })
        })?.filter_map(|r| r.ok()).collect();
        Ok(ms)
    }

    pub fn toggle_milestone(&self, id: &str) -> Result<Option<Milestone>> {
        let conn = self.conn()?;
        let current: Option<String> = conn.query_row(
            "SELECT status FROM milestones WHERE id = ?1", [id],
            |row| row.get(0),
        ).ok();
        let Some(current_status) = current else { return Ok(None) };
        let (new_status, completed_at): (&str, Option<String>) = if current_status == "done" {
            ("pending", None)
        } else {
            ("done", Some(Utc::now().to_rfc3339()))
        };
        conn.execute(
            "UPDATE milestones SET status = ?1, completed_at = ?2 WHERE id = ?3",
            rusqlite::params![new_status, completed_at, id],
        )?;
        // Return updated
        let mut stmt = conn.prepare(
            "SELECT id, goal_id, title, due_date, status, sort_order, completed_at FROM milestones WHERE id = ?1"
        )?;
        let ms = stmt.query_row([id], |row| {
            Ok(Milestone {
                id: row.get(0)?, goal_id: row.get(1)?, title: row.get(2)?,
                due_date: row.get(3)?, status: row.get(4)?, sort_order: row.get(5)?,
                completed_at: row.get(6)?,
            })
        }).ok();
        Ok(ms)
    }

    // ── Recurring Tasks ────────────────────────────────────────────────

    pub fn add_recurring_task(&self, task: &RecurringTask) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO recurring_tasks (id, goal_id, title, cron_expr, last_completed, streak_count, enabled)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![task.id, task.goal_id, task.title, task.cron_expr, task.last_completed, task.streak_count, task.enabled],
        )?;
        Ok(())
    }

    pub fn list_recurring_tasks(&self, goal_id: &str) -> Result<Vec<RecurringTask>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, goal_id, title, cron_expr, last_completed, streak_count, enabled
             FROM recurring_tasks WHERE goal_id = ?1 ORDER BY title"
        )?;
        let tasks = stmt.query_map([goal_id], |row| {
            Ok(RecurringTask {
                id: row.get(0)?, goal_id: row.get(1)?, title: row.get(2)?,
                cron_expr: row.get(3)?, last_completed: row.get(4)?, streak_count: row.get(5)?,
                enabled: row.get(6)?,
            })
        })?.filter_map(|r| r.ok()).collect();
        Ok(tasks)
    }

    pub fn complete_recurring_task(&self, task_id: &str) -> Result<Option<i32>> {
        let conn = self.conn()?;
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let current: Option<(Option<String>, i32)> = conn.query_row(
            "SELECT last_completed, streak_count FROM recurring_tasks WHERE id = ?1", [task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).ok();
        let Some((last, streak)) = current else { return Ok(None) };

        // Calculate new streak
        let new_streak = if let Some(ref last_date) = last {
            let last_d = NaiveDate::parse_from_str(last_date, "%Y-%m-%d").unwrap_or_default();
            let today_d = NaiveDate::parse_from_str(&today, "%Y-%m-%d").unwrap_or_default();
            let diff = (today_d - last_d).num_days();
            if diff <= 1 { streak + 1 } else { 1 }
        } else {
            1
        };

        conn.execute(
            "UPDATE recurring_tasks SET last_completed = ?1, streak_count = ?2 WHERE id = ?3",
            rusqlite::params![today, new_streak, task_id],
        )?;
        Ok(Some(new_streak))
    }

    pub fn get_today_tasks(&self) -> Result<Vec<TodayTask>> {
        let conn = self.conn()?;
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let mut stmt = conn.prepare(
            "SELECT rt.id, rt.goal_id, rt.title, rt.cron_expr, rt.last_completed, rt.streak_count, rt.enabled, g.title
             FROM recurring_tasks rt JOIN goals g ON rt.goal_id = g.id
             WHERE rt.enabled = 1 AND g.status = 'active'
             ORDER BY g.title, rt.title"
        )?;
        let tasks = stmt.query_map([], |row| {
            let last_completed: Option<String> = row.get(4)?;
            let completed_today = last_completed.as_deref() == Some(today.as_str());
            Ok(TodayTask {
                task: RecurringTask {
                    id: row.get(0)?, goal_id: row.get(1)?, title: row.get(2)?,
                    cron_expr: row.get(3)?, last_completed, streak_count: row.get(5)?,
                    enabled: row.get(6)?,
                },
                goal_title: row.get(7)?,
                completed_today,
            })
        })?.filter_map(|r| r.ok()).collect();
        Ok(tasks)
    }

    // ── Check-ins ──────────────────────────────────────────────────────

    pub fn add_check_in(&self, ci: &CheckIn) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO check_ins (id, goal_id, date, mood, note, ai_feedback)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![ci.id, ci.goal_id, ci.date, ci.mood, ci.note, ci.ai_feedback],
        )?;
        Ok(())
    }

    pub fn list_check_ins(&self, goal_id: &str, limit: i32) -> Result<Vec<CheckIn>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, goal_id, date, mood, note, ai_feedback
             FROM check_ins WHERE goal_id = ?1 ORDER BY date DESC LIMIT ?2"
        )?;
        let cis = stmt.query_map(rusqlite::params![goal_id, limit], |row| {
            Ok(CheckIn {
                id: row.get(0)?, goal_id: row.get(1)?, date: row.get(2)?,
                mood: row.get(3)?, note: row.get(4)?, ai_feedback: row.get(5)?,
            })
        })?.filter_map(|r| r.ok()).collect();
        Ok(cis)
    }

    // ── Progress ───────────────────────────────────────────────────────

    pub fn get_goal_progress(&self, goal_id: &str) -> Result<Option<GoalProgress>> {
        let goal = match self.get_goal(goal_id)? {
            Some(g) => g,
            None => return Ok(None),
        };
        let milestones = self.list_milestones(goal_id)?;
        let ms_total = milestones.len() as i32;
        let ms_done = milestones.iter().filter(|m| m.status == "done").count() as i32;
        let percentage = if ms_total > 0 { (ms_done as f64 / ms_total as f64) * 100.0 } else { 0.0 };

        // Best streak across recurring tasks
        let tasks = self.list_recurring_tasks(goal_id)?;
        let current_streak = tasks.iter().map(|t| t.streak_count).max().unwrap_or(0);

        // Days remaining
        let days_remaining = goal.target_date.as_ref().and_then(|td| {
            NaiveDate::parse_from_str(td, "%Y-%m-%d").ok().map(|target| {
                let today = Utc::now().date_naive();
                (target - today).num_days()
            })
        });

        let recent_check_ins = self.list_check_ins(goal_id, 5)?;

        Ok(Some(GoalProgress {
            goal, milestones_total: ms_total, milestones_done: ms_done,
            percentage, current_streak, days_remaining, recent_check_ins,
        }))
    }

    /// Get a summary for all active goals (for Chat system prompt injection)
    pub fn active_goals_summary(&self) -> Result<Vec<GoalProgress>> {
        let goals = self.list_goals(Some(GoalStatus::Active))?;
        let mut summaries = Vec::new();
        for g in goals {
            if let Some(progress) = self.get_goal_progress(&g.id)? {
                summaries.push(progress);
            }
        }
        Ok(summaries)
    }

    // ── Progress History ────────────────────────────────────────────────

    /// Get mood trend for a goal over the last N days.
    pub fn mood_trend(&self, goal_id: &str, days: i32) -> Result<Vec<(String, i32)>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT date, mood FROM check_ins
             WHERE goal_id = ?1
             ORDER BY date DESC LIMIT ?2"
        )?;
        let rows: Vec<(String, i32)> = stmt.query_map(
            rusqlite::params![goal_id, days],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?.filter_map(|r| r.ok()).collect();
        Ok(rows)
    }

    /// Get mood trend across ALL goals for overall history.
    pub fn global_mood_trend(&self, days: i32) -> Result<Vec<(String, f64)>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT date, AVG(mood) as avg_mood FROM check_ins
             GROUP BY date ORDER BY date DESC LIMIT ?1"
        )?;
        let rows: Vec<(String, f64)> = stmt.query_map(
            rusqlite::params![days],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?.filter_map(|r| r.ok()).collect();
        Ok(rows)
    }

    /// Compute weekly summary for active goals (for weekly report).
    pub fn weekly_summary(&self) -> Result<WeeklySummary> {
        let today = Utc::now().date_naive();
        let week_start = today - chrono::Duration::days(6);
        let week_start_str = week_start.format("%Y-%m-%d").to_string();
        let week_end_str = today.format("%Y-%m-%d").to_string();

        let conn = self.conn()?;

        // Count recurring task completions this week
        let completed_tasks: i32 = conn.query_row(
            "SELECT COUNT(*) FROM recurring_tasks rt
             JOIN goals g ON rt.goal_id = g.id
             WHERE g.status = 'active' AND rt.enabled = 1
             AND rt.last_completed >= ?1",
            rusqlite::params![week_start_str],
            |row| row.get(0),
        ).unwrap_or(0);

        let total_tasks: i32 = conn.query_row(
            "SELECT COUNT(*) FROM recurring_tasks rt
             JOIN goals g ON rt.goal_id = g.id
             WHERE g.status = 'active' AND rt.enabled = 1",
            rusqlite::params![],
            |row| row.get(0),
        ).unwrap_or(0);

        // 7 days x total_tasks gives total possible completions
        let total_possible = total_tasks * 7;
        let completion_rate = if total_possible > 0 {
            (completed_tasks as f64 / total_possible as f64) * 100.0
        } else {
            0.0
        };

        // Average mood this week
        let avg_mood: Option<f64> = conn.query_row(
            "SELECT AVG(mood) FROM check_ins WHERE date >= ?1",
            rusqlite::params![week_start_str],
            |row| row.get(0),
        ).unwrap_or(None);

        // Best streak across active goals
        let best_streak: i32 = conn.query_row(
            "SELECT COALESCE(MAX(rt.streak_count), 0) FROM recurring_tasks rt
             JOIN goals g ON rt.goal_id = g.id
             WHERE g.status = 'active'",
            [],
            |row| row.get(0),
        ).unwrap_or(0);

        // Milestones completed this week
        let milestones_completed: i32 = conn.query_row(
            "SELECT COUNT(*) FROM milestones WHERE status = 'done' AND completed_at >= ?1",
            rusqlite::params![week_start_str],
            |row| row.get(0),
        ).unwrap_or(0);

        Ok(WeeklySummary {
            week_start: week_start_str,
            week_end: week_end_str,
            total_tasks: total_possible,
            completed_tasks,
            completion_rate,
            avg_mood,
            best_streak,
            milestones_completed,
        })
    }

    /// All check-ins across all goals, limited.
    pub fn all_check_ins(&self, limit: i32) -> Result<Vec<CheckIn>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, goal_id, date, mood, note, ai_feedback
             FROM check_ins ORDER BY date DESC LIMIT ?1"
        )?;
        let cis = stmt.query_map(rusqlite::params![limit], |row| {
            Ok(CheckIn {
                id: row.get(0)?, goal_id: row.get(1)?, date: row.get(2)?,
                mood: row.get(3)?, note: row.get(4)?, ai_feedback: row.get(5)?,
            })
        })?.filter_map(|r| r.ok()).collect();
        Ok(cis)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (GoalsStore, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test_goals.db");
        let store = GoalsStore::new(path.to_str().unwrap()).unwrap();
        (store, dir)
    }

    #[test]
    fn test_goal_crud() {
        let (store, _dir) = temp_store();
        let now = Utc::now().to_rfc3339();
        let goal = Goal {
            id: "g1".into(), title: "Pass exam".into(), category: "study".into(),
            description: Some("Get into NTU".into()), target_date: Some("2026-09-01".into()),
            status: GoalStatus::Active, context: None,
            created_at: now.clone(), updated_at: now,
        };
        store.create_goal(&goal).unwrap();

        let fetched = store.get_goal("g1").unwrap().unwrap();
        assert_eq!(fetched.title, "Pass exam");

        store.update_goal("g1", Some("Pass NTU exam"), None, None, None).unwrap();
        let updated = store.get_goal("g1").unwrap().unwrap();
        assert_eq!(updated.title, "Pass NTU exam");

        let all = store.list_goals(Some(GoalStatus::Active)).unwrap();
        assert_eq!(all.len(), 1);

        store.delete_goal("g1").unwrap();
        assert!(store.get_goal("g1").unwrap().is_none());
    }

    #[test]
    fn test_milestones() {
        let (store, _dir) = temp_store();
        let now = Utc::now().to_rfc3339();
        store.create_goal(&Goal {
            id: "g1".into(), title: "Test".into(), category: "test".into(),
            description: None, target_date: None, status: GoalStatus::Active,
            context: None, created_at: now.clone(), updated_at: now,
        }).unwrap();

        store.add_milestone(&Milestone {
            id: "m1".into(), goal_id: "g1".into(), title: "Step 1".into(),
            due_date: None, status: "pending".into(), sort_order: 0, completed_at: None,
        }).unwrap();

        let ms = store.list_milestones("g1").unwrap();
        assert_eq!(ms.len(), 1);

        let toggled = store.toggle_milestone("m1").unwrap().unwrap();
        assert_eq!(toggled.status, "done");
        assert!(toggled.completed_at.is_some());

        let toggled2 = store.toggle_milestone("m1").unwrap().unwrap();
        assert_eq!(toggled2.status, "pending");
    }

    #[test]
    fn test_recurring_tasks_and_streak() {
        let (store, _dir) = temp_store();
        let now = Utc::now().to_rfc3339();
        store.create_goal(&Goal {
            id: "g1".into(), title: "Test".into(), category: "test".into(),
            description: None, target_date: None, status: GoalStatus::Active,
            context: None, created_at: now.clone(), updated_at: now,
        }).unwrap();

        store.add_recurring_task(&RecurringTask {
            id: "t1".into(), goal_id: "g1".into(), title: "Study math".into(),
            cron_expr: "0 9 * * *".into(), last_completed: None, streak_count: 0, enabled: true,
        }).unwrap();

        let streak = store.complete_recurring_task("t1").unwrap().unwrap();
        assert_eq!(streak, 1);

        let tasks = store.get_today_tasks().unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].completed_today);
    }

    #[test]
    fn test_check_ins() {
        let (store, _dir) = temp_store();
        let now = Utc::now().to_rfc3339();
        store.create_goal(&Goal {
            id: "g1".into(), title: "Test".into(), category: "test".into(),
            description: None, target_date: None, status: GoalStatus::Active,
            context: None, created_at: now.clone(), updated_at: now,
        }).unwrap();

        store.add_check_in(&CheckIn {
            id: "c1".into(), goal_id: "g1".into(), date: "2026-03-25".into(),
            mood: 4, note: Some("Good day".into()), ai_feedback: None,
        }).unwrap();

        let cis = store.list_check_ins("g1", 10).unwrap();
        assert_eq!(cis.len(), 1);
        assert_eq!(cis[0].mood, 4);
    }

    #[test]
    fn test_goal_progress() {
        let (store, _dir) = temp_store();
        let now = Utc::now().to_rfc3339();
        store.create_goal(&Goal {
            id: "g1".into(), title: "Test".into(), category: "test".into(),
            description: None, target_date: Some("2026-12-31".into()),
            status: GoalStatus::Active, context: None,
            created_at: now.clone(), updated_at: now,
        }).unwrap();

        store.add_milestone(&Milestone {
            id: "m1".into(), goal_id: "g1".into(), title: "Step 1".into(),
            due_date: None, status: "done".into(), sort_order: 0, completed_at: Some(Utc::now().to_rfc3339()),
        }).unwrap();
        store.add_milestone(&Milestone {
            id: "m2".into(), goal_id: "g1".into(), title: "Step 2".into(),
            due_date: None, status: "pending".into(), sort_order: 1, completed_at: None,
        }).unwrap();

        let progress = store.get_goal_progress("g1").unwrap().unwrap();
        assert_eq!(progress.milestones_total, 2);
        assert_eq!(progress.milestones_done, 1);
        assert!((progress.percentage - 50.0).abs() < 0.1);
        assert!(progress.days_remaining.unwrap() > 0);
    }

    #[test]
    fn test_mood_trend() {
        let (store, _dir) = temp_store();
        let now = Utc::now().to_rfc3339();
        store.create_goal(&Goal {
            id: "g1".into(), title: "Test".into(), category: "test".into(),
            description: None, target_date: None, status: GoalStatus::Active,
            context: None, created_at: now.clone(), updated_at: now,
        }).unwrap();

        for i in 1..=5 {
            store.add_check_in(&CheckIn {
                id: format!("c{}", i), goal_id: "g1".into(),
                date: format!("2026-03-{:02}", 20 + i),
                mood: i, note: None, ai_feedback: None,
            }).unwrap();
        }

        let trend = store.mood_trend("g1", 10).unwrap();
        assert_eq!(trend.len(), 5);
        // Most recent first
        assert_eq!(trend[0].1, 5);
    }

    #[test]
    fn test_weekly_summary() {
        let (store, _dir) = temp_store();
        let now = Utc::now().to_rfc3339();
        store.create_goal(&Goal {
            id: "g1".into(), title: "Test".into(), category: "test".into(),
            description: None, target_date: None, status: GoalStatus::Active,
            context: None, created_at: now.clone(), updated_at: now,
        }).unwrap();

        store.add_recurring_task(&RecurringTask {
            id: "t1".into(), goal_id: "g1".into(), title: "Daily study".into(),
            cron_expr: "0 9 * * *".into(), last_completed: None,
            streak_count: 3, enabled: true,
        }).unwrap();

        let ws = store.weekly_summary().unwrap();
        assert_eq!(ws.total_tasks, 7); // 1 task x 7 days
        assert_eq!(ws.best_streak, 3);
    }

    #[test]
    fn test_global_mood_trend() {
        let (store, _dir) = temp_store();
        let now = Utc::now().to_rfc3339();
        store.create_goal(&Goal {
            id: "g1".into(), title: "A".into(), category: "test".into(),
            description: None, target_date: None, status: GoalStatus::Active,
            context: None, created_at: now.clone(), updated_at: now.clone(),
        }).unwrap();
        store.create_goal(&Goal {
            id: "g2".into(), title: "B".into(), category: "test".into(),
            description: None, target_date: None, status: GoalStatus::Active,
            context: None, created_at: now.clone(), updated_at: now,
        }).unwrap();

        store.add_check_in(&CheckIn {
            id: "c1".into(), goal_id: "g1".into(), date: "2026-03-25".into(),
            mood: 4, note: None, ai_feedback: None,
        }).unwrap();
        store.add_check_in(&CheckIn {
            id: "c2".into(), goal_id: "g2".into(), date: "2026-03-25".into(),
            mood: 2, note: None, ai_feedback: None,
        }).unwrap();

        let trend = store.global_mood_trend(30).unwrap();
        assert_eq!(trend.len(), 1);
        assert!((trend[0].1 - 3.0).abs() < 0.01); // avg of 4 and 2
    }
}
