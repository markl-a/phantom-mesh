//! Todoist built-in tools — the partner's DO-actions for tasks.
//!
//! `todoist_add_task` lets a "把這個記到 Todoist / add a task" message actually
//! create a Todoist task (NORTH-STAR 使用①: 記東西, low-risk write). `todoist_list_tasks`
//! lets the agent read what's on the user's plate before acting. Both delegate
//! to the shared [`crate::todoist`] REST client, which resolves the API token
//! from `[tools] todoist_api_token` (agents.toml) or the `TODOIST_API_TOKEN`
//! env var — never hardcoded.

use serde_json::Value;

use crate::config::ToolsConfig;

const NO_TOKEN_HINT: &str = "ERROR: no Todoist token configured. Set `[tools] todoist_api_token = \"…\"` \
    in ~/.spectyn-mesh/agents.toml or the TODOIST_API_TOKEN env var \
    (get one at Todoist → Settings → Integrations → Developer).";

/// Create a Todoist task. Args: `content` (required), optional `due_string`
/// (natural language, e.g. "tomorrow 9am"), `priority` (1..=4, 4 = highest),
/// `project_id`.
pub async fn add_task(args: &Value, config: &ToolsConfig) -> String {
    let content = match args.get("content").and_then(Value::as_str) {
        Some(c) if !c.trim().is_empty() => c.trim(),
        _ => return "ERROR: missing required parameter 'content'".to_string(),
    };
    let token = match crate::todoist::resolve_token(Some(config)) {
        Some(t) => t,
        None => return NO_TOKEN_HINT.to_string(),
    };
    let due = args.get("due_string").and_then(Value::as_str);
    let priority = args
        .get("priority")
        .and_then(Value::as_u64)
        .map(|p| p as u8);
    let project_id = args.get("project_id").and_then(Value::as_str);

    match crate::todoist::add_task(&token, content, due, priority, project_id).await {
        Ok(task) => {
            let due_note = task
                .due
                .as_ref()
                .and_then(|d| d.string.clone().or(d.date.clone()))
                .map(|d| format!(" (due {d})"))
                .unwrap_or_default();
            format!("Added Todoist task #{}: \"{}\"{}", task.id, task.content, due_note)
        }
        Err(e) => format!("ERROR: failed to add Todoist task: {e}"),
    }
}

/// List active Todoist tasks. Optional `filter` is a Todoist filter query
/// (e.g. "today | overdue", "p1"). Returns up to 50 tasks, highest priority first.
pub async fn list_tasks(args: &Value, config: &ToolsConfig) -> String {
    let token = match crate::todoist::resolve_token(Some(config)) {
        Some(t) => t,
        None => return NO_TOKEN_HINT.to_string(),
    };
    let filter = args.get("filter").and_then(Value::as_str);

    match crate::todoist::list_tasks(&token, filter).await {
        Ok(tasks) if tasks.is_empty() => "No matching Todoist tasks.".to_string(),
        Ok(mut tasks) => {
            tasks.sort_by_key(|t| std::cmp::Reverse(t.priority));
            let mut out = format!("{} Todoist task(s):", tasks.len().min(50));
            for t in tasks.iter().take(50) {
                let due = t
                    .due
                    .as_ref()
                    .and_then(|d| d.string.clone().or(d.date.clone()))
                    .map(|d| format!(" (due {d})"))
                    .unwrap_or_default();
                out.push_str(&format!("\n- [p{}] {}{}  #{}", t.priority, t.content, due, t.id));
            }
            out
        }
        Err(e) => format!("ERROR: failed to list Todoist tasks: {e}"),
    }
}

/// Complete (mark done) a Todoist task. Args: `task_id` (required) — the id of
/// the task to close, e.g. one returned by `todoist_list_tasks`. This is a
/// bounded DO-action: it can only close the one task named, never run a command
/// or touch anything else (NORTH-STAR 反應半: 用 code 把事辦完 — safe write).
pub async fn complete_task(args: &Value, config: &ToolsConfig) -> String {
    let task_id = match args.get("task_id").and_then(Value::as_str) {
        Some(id) if !id.trim().is_empty() => id.trim(),
        _ => return "ERROR: missing required parameter 'task_id'".to_string(),
    };
    let token = match crate::todoist::resolve_token(Some(config)) {
        Some(t) => t,
        None => return NO_TOKEN_HINT.to_string(),
    };

    match crate::todoist::complete_task(&token, task_id).await {
        Ok(()) => format!("Completed Todoist task #{task_id}."),
        Err(e) => format!("ERROR: failed to complete Todoist task #{task_id}: {e}"),
    }
}
