//! Goals Push — generates morning briefing & evening check-in messages from GoalsStore.

use crate::goals::GoalsStore;
use anyhow::Result;

/// Generate a morning briefing message listing today's tasks and active goal progress.
pub fn morning_briefing(store: &GoalsStore) -> Result<String> {
    let today_tasks = store.get_today_tasks()?;
    let summaries = store.active_goals_summary()?;

    if summaries.is_empty() {
        return Ok("".to_string()); // No active goals — skip briefing
    }

    let mut msg = String::from("☀️ 早安 Master！以下是今日任務簡報：\n\n");

    // Active goals overview
    msg.push_str("📊 目標進度：\n");
    for s in &summaries {
        let bar = progress_bar(s.percentage);
        let streak_str = if s.current_streak > 0 {
            format!(" 🔥{}", s.current_streak)
        } else {
            String::new()
        };
        let days_str = match s.days_remaining {
            Some(d) if d > 0 => format!(" ({}天)", d),
            Some(0) => " (今天到期!)".to_string(),
            Some(d) if d < 0 => format!(" (已逾期{}天)", -d),
            _ => String::new(),
        };
        msg.push_str(&format!(
            "• {} {}{:.0}%{}{}\n",
            s.goal.title, bar, s.percentage, streak_str, days_str
        ));
    }

    // Today's tasks
    if !today_tasks.is_empty() {
        msg.push_str("\n✅ 今日待完成：\n");
        for tt in &today_tasks {
            let icon = if tt.completed_today { "✓" } else { "○" };
            let streak = if tt.task.streak_count > 0 {
                format!(" (連續{}天)", tt.task.streak_count)
            } else {
                String::new()
            };
            msg.push_str(&format!(
                "{} {}{} — {}\n",
                icon, tt.task.title, streak, tt.goal_title
            ));
        }

        let done = today_tasks.iter().filter(|t| t.completed_today).count();
        let total = today_tasks.len();
        if done > 0 {
            msg.push_str(&format!("\n已完成 {}/{} 🎉\n", done, total));
        }
    } else {
        msg.push_str("\n今天沒有排定的任務，休息也很重要！\n");
    }

    msg.push_str("\n💬 回覆任何訊息即可開始對話");

    Ok(msg)
}

/// Generate an evening check-in message summarizing today's progress.
pub fn evening_checkin(store: &GoalsStore) -> Result<String> {
    let today_tasks = store.get_today_tasks()?;
    let summaries = store.active_goals_summary()?;

    if summaries.is_empty() {
        return Ok("".to_string());
    }

    let total = today_tasks.len();
    let done = today_tasks.iter().filter(|t| t.completed_today).count();

    let mut msg = String::from("🌙 晚安 Master！來回顧一下今天的進度：\n\n");

    if total > 0 {
        let pct = if total > 0 { (done as f64 / total as f64) * 100.0 } else { 0.0 };
        msg.push_str(&format!("📋 今日任務：{}/{} 完成 ({:.0}%)\n", done, total, pct));

        // List incomplete tasks
        let incomplete: Vec<_> = today_tasks.iter().filter(|t| !t.completed_today).collect();
        if !incomplete.is_empty() {
            msg.push_str("\n未完成：\n");
            for t in &incomplete {
                msg.push_str(&format!("  ○ {} ({})\n", t.task.title, t.goal_title));
            }
        }

        // List completed
        let completed: Vec<_> = today_tasks.iter().filter(|t| t.completed_today).collect();
        if !completed.is_empty() {
            msg.push_str("\n已完成：\n");
            for t in &completed {
                let streak = if t.task.streak_count > 0 {
                    format!(" 🔥{}", t.task.streak_count)
                } else {
                    String::new()
                };
                msg.push_str(&format!("  ✓ {}{}\n", t.task.title, streak));
            }
        }
    }

    // Best streaks
    let max_streak = summaries.iter().map(|s| s.current_streak).max().unwrap_or(0);
    if max_streak >= 3 {
        msg.push_str(&format!("\n🔥 最長連續記錄：{} 天！繼續保持！\n", max_streak));
    }

    msg.push_str("\n你今天過得如何？回覆 1-5 分 (1=糟糕 5=超棒) 記錄心情");

    Ok(msg)
}

/// Generate a compact context block for system prompt injection.
/// This gives the LLM awareness of the user's active goals, milestones, and daily tasks.
pub fn goals_context(store: &GoalsStore) -> Result<String> {
    let summaries = store.active_goals_summary()?;
    if summaries.is_empty() {
        return Ok(String::new());
    }

    let today_tasks = store.get_today_tasks()?;

    let mut ctx = String::from("\n\n[User's Active Goals]\n");
    for s in &summaries {
        ctx.push_str(&format!(
            "- {} ({}): {:.0}% complete, {}/{} milestones",
            s.goal.title, s.goal.category, s.percentage, s.milestones_done, s.milestones_total,
        ));
        if s.current_streak > 0 {
            ctx.push_str(&format!(", streak {}d", s.current_streak));
        }
        if let Some(d) = s.days_remaining {
            if d > 0 {
                ctx.push_str(&format!(", {} days left", d));
            } else if d == 0 {
                ctx.push_str(", due today");
            } else {
                ctx.push_str(&format!(", {} days overdue", -d));
            }
        }
        ctx.push('\n');
    }

    if !today_tasks.is_empty() {
        ctx.push_str("Today's tasks: ");
        let done = today_tasks.iter().filter(|t| t.completed_today).count();
        ctx.push_str(&format!("{}/{} done. ", done, today_tasks.len()));
        let pending: Vec<&str> = today_tasks.iter()
            .filter(|t| !t.completed_today)
            .map(|t| t.task.title.as_str())
            .collect();
        if !pending.is_empty() {
            ctx.push_str(&format!("Pending: {}", pending.join(", ")));
        }
        ctx.push('\n');
    }

    ctx.push_str("When the user's message relates to their goals, reference this context to provide relevant, personalized advice. Help them track progress and stay motivated.\n");

    Ok(ctx)
}

/// Generate a weekly report message summarizing the past 7 days.
pub fn weekly_report(store: &GoalsStore) -> Result<String> {
    let summaries = store.active_goals_summary()?;
    if summaries.is_empty() {
        return Ok(String::new());
    }

    let ws = store.weekly_summary()?;

    let mut msg = String::from("📊 Weekly Report — 本週回顧\n\n");

    // Overall stats
    msg.push_str(&format!("📅 {} ~ {}\n\n", ws.week_start, ws.week_end));

    // Task completion
    if ws.total_tasks > 0 {
        let bar = progress_bar(ws.completion_rate);
        msg.push_str(&format!(
            "✅ 任務完成率：{} {:.0}% ({}/{})\n",
            bar, ws.completion_rate, ws.completed_tasks, ws.total_tasks
        ));
    }

    // Mood average
    if let Some(mood) = ws.avg_mood {
        let emoji = match mood.round() as i32 {
            5 => "🤩",
            4 => "😊",
            3 => "😐",
            2 => "😔",
            _ => "😢",
        };
        msg.push_str(&format!("💭 平均心情：{} {:.1}/5\n", emoji, mood));
    }

    // Best streak
    if ws.best_streak > 0 {
        msg.push_str(&format!("🔥 最佳連續記錄：{} 天\n", ws.best_streak));
    }

    // Milestones completed
    if ws.milestones_completed > 0 {
        msg.push_str(&format!("🏆 本週達成里程碑：{} 個\n", ws.milestones_completed));
    }

    // Per-goal breakdown
    msg.push_str("\n📈 各目標進度：\n");
    for s in &summaries {
        let bar = progress_bar(s.percentage);
        let streak_str = if s.current_streak > 0 {
            format!(" 🔥{}", s.current_streak)
        } else {
            String::new()
        };
        msg.push_str(&format!(
            "• {} {} {:.0}%{}\n",
            s.goal.title, bar, s.percentage, streak_str
        ));
    }

    // Mood trend (last 7 check-ins)
    let mood_trend = store.global_mood_trend(7)?;
    if mood_trend.len() >= 2 {
        let first = mood_trend.last().map(|m| m.1).unwrap_or(0.0);
        let last = mood_trend.first().map(|m| m.1).unwrap_or(0.0);
        let diff = last - first;
        let trend = if diff > 0.3 { "📈 上升" }
                    else if diff < -0.3 { "📉 下降" }
                    else { "➡️ 穩定" };
        msg.push_str(&format!("\n心情趨勢：{}\n", trend));
    }

    msg.push_str("\n繼續加油！每一小步都是進步 💪");

    Ok(msg)
}

fn progress_bar(pct: f64) -> String {
    let filled = (pct / 10.0).round() as usize;
    let empty = 10usize.saturating_sub(filled);
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::{Goal, GoalStatus, RecurringTask};

    #[test]
    fn test_morning_briefing_empty() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let store = GoalsStore::new(db.to_str().unwrap()).unwrap();
        let msg = morning_briefing(&store).unwrap();
        assert!(msg.is_empty(), "No goals = no briefing");
    }

    #[test]
    fn test_morning_briefing_with_goals() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let store = GoalsStore::new(db.to_str().unwrap()).unwrap();

        let now = chrono::Utc::now().to_rfc3339();
        store.create_goal(&Goal {
            id: "g1".into(),
            title: "考上台大".into(),
            category: "學業".into(),
            description: None,
            target_date: Some("2027-07-01".into()),
            status: GoalStatus::Active,
            context: None,
            created_at: now.clone(),
            updated_at: now,
        }).unwrap();

        store.add_recurring_task(&RecurringTask {
            id: "t1".into(),
            goal_id: "g1".into(),
            title: "每日數學練習".into(),
            cron_expr: "0 9 * * *".into(),
            last_completed: None,
            streak_count: 5,
            enabled: true,
        }).unwrap();

        let msg = morning_briefing(&store).unwrap();
        assert!(msg.contains("考上台大"), "Should mention goal title");
        assert!(msg.contains("每日數學練習"), "Should list task");
        assert!(msg.contains("早安"), "Should have greeting");
    }

    #[test]
    fn test_evening_checkin_with_data() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let store = GoalsStore::new(db.to_str().unwrap()).unwrap();

        let now = chrono::Utc::now().to_rfc3339();
        store.create_goal(&Goal {
            id: "g1".into(),
            title: "學好英文".into(),
            category: "學業".into(),
            description: None,
            target_date: None,
            status: GoalStatus::Active,
            context: None,
            created_at: now.clone(),
            updated_at: now,
        }).unwrap();

        store.add_recurring_task(&RecurringTask {
            id: "t1".into(),
            goal_id: "g1".into(),
            title: "背單字".into(),
            cron_expr: "0 8 * * *".into(),
            last_completed: None,
            streak_count: 0,
            enabled: true,
        }).unwrap();

        let msg = evening_checkin(&store).unwrap();
        assert!(msg.contains("晚安"), "Should have evening greeting");
        assert!(msg.contains("背單字"), "Should mention task");
    }

    #[test]
    fn test_progress_bar() {
        assert_eq!(progress_bar(0.0), "[░░░░░░░░░░]");
        assert_eq!(progress_bar(50.0), "[█████░░░░░]");
        assert_eq!(progress_bar(100.0), "[██████████]");
        assert_eq!(progress_bar(25.0), "[███░░░░░░░]");
    }

    #[test]
    fn test_weekly_report_empty() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let store = GoalsStore::new(db.to_str().unwrap()).unwrap();
        let msg = weekly_report(&store).unwrap();
        assert!(msg.is_empty(), "No goals = no report");
    }

    #[test]
    fn test_weekly_report_with_data() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let store = GoalsStore::new(db.to_str().unwrap()).unwrap();

        let now = chrono::Utc::now().to_rfc3339();
        store.create_goal(&crate::goals::Goal {
            id: "g1".into(),
            title: "學日文".into(),
            category: "語言".into(),
            description: None,
            target_date: Some("2027-01-01".into()),
            status: GoalStatus::Active,
            context: None,
            created_at: now.clone(),
            updated_at: now,
        }).unwrap();

        store.add_recurring_task(&RecurringTask {
            id: "t1".into(),
            goal_id: "g1".into(),
            title: "每日日文聽力".into(),
            cron_expr: "0 9 * * *".into(),
            last_completed: None,
            streak_count: 7,
            enabled: true,
        }).unwrap();

        let msg = weekly_report(&store).unwrap();
        assert!(msg.contains("Weekly Report"), "Should have title");
        assert!(msg.contains("學日文"), "Should mention goal");
        assert!(msg.contains("🔥"), "Should show streak");
    }
}
