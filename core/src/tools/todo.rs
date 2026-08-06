use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Todo,
    InProgress,
    Done,
}

impl TaskStatus {
    fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Todo => "todo",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::Done => "done",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "todo" => Some(TaskStatus::Todo),
            "in_progress" => Some(TaskStatus::InProgress),
            "done" => Some(TaskStatus::Done),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: u32,
    pub description: String,
    pub status: TaskStatus,
    pub created_at: String,
}

fn tasks_path(session: &str) -> std::path::PathBuf {
    crate::cli_config::spectyn_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("tasks")
        .join(format!("{}.json", session))
}

fn resolve_session(args: &Value) -> String {
    args["session"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("default")
        .to_string()
}

async fn load_tasks(session: &str) -> Vec<Task> {
    let path = tasks_path(session);
    if let Ok(data) = tokio::fs::read_to_string(&path).await {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Vec::new()
    }
}

async fn save_tasks(session: &str, tasks: &[Task]) {
    let path = tasks_path(session);
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let _ = tokio::fs::write(
        &path,
        serde_json::to_string_pretty(tasks).unwrap_or_default(),
    )
    .await;
}

fn now_iso() -> String {
    // Use a simple approach compatible with std: format seconds since epoch as ISO-ish string.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Convert to a basic ISO 8601 timestamp (UTC)
    let s = secs;
    let (y, mo, d, h, mi, sec) = epoch_to_datetime(s);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, mi, sec)
}

fn epoch_to_datetime(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let sec = secs % 60;
    let mins = secs / 60;
    let min = mins % 60;
    let hours = mins / 60;
    let hour = hours % 24;
    let days = hours / 24;

    // Gregorian calendar calculation from day count (days since 1970-01-01)
    let mut y = 1970u64;
    let mut rem = days;
    loop {
        let dy = if is_leap(y) { 366 } else { 365 };
        if rem < dy {
            break;
        }
        rem -= dy;
        y += 1;
    }
    let months = if is_leap(y) {
        [31u64, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31u64, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut mo = 1u64;
    for &dm in &months {
        if rem < dm {
            break;
        }
        rem -= dm;
        mo += 1;
    }
    let d = rem + 1;
    (y, mo, d, hour, min, sec)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// task_add — create a new task with auto-incremented ID, status=Todo
pub async fn add(args: &Value) -> String {
    let description = match args["description"].as_str() {
        Some(d) if !d.is_empty() => d.to_string(),
        _ => return "Error: missing 'description' argument".to_string(),
    };
    let session = resolve_session(args);
    let mut tasks = load_tasks(&session).await;

    let next_id = tasks.iter().map(|t| t.id).max().unwrap_or(0) + 1;
    let task = Task {
        id: next_id,
        description: description.clone(),
        status: TaskStatus::Todo,
        created_at: now_iso(),
    };
    tasks.push(task);
    save_tasks(&session, &tasks).await;
    format!("Added task #{}: {}", next_id, description)
}

/// task_update — update the status of a task by id
pub async fn update(args: &Value) -> String {
    let id = match args["id"].as_u64() {
        Some(i) => i as u32,
        None => return "Error: missing or invalid 'id' argument".to_string(),
    };
    let status_str = match args["status"].as_str() {
        Some(s) => s,
        None => return "Error: missing 'status' argument".to_string(),
    };
    let new_status = match TaskStatus::from_str(status_str) {
        Some(s) => s,
        None => {
            return format!(
                "Error: invalid status '{}'. Use todo|in_progress|done",
                status_str
            )
        }
    };
    let session = resolve_session(args);
    let mut tasks = load_tasks(&session).await;

    match tasks.iter_mut().find(|t| t.id == id) {
        Some(task) => {
            task.status = new_status.clone();
            save_tasks(&session, &tasks).await;
            format!("Task #{} marked as {}", id, new_status.as_str())
        }
        None => format!("Error: task #{} not found", id),
    }
}

/// task_list — list tasks, optionally filtered by status
pub async fn list(args: &Value) -> String {
    let session = resolve_session(args);
    let status_filter = args["status_filter"].as_str();
    let tasks = load_tasks(&session).await;

    let filtered: Vec<&Task> = tasks
        .iter()
        .filter(|t| {
            if let Some(f) = status_filter {
                t.status.as_str() == f
            } else {
                true
            }
        })
        .collect();

    if filtered.is_empty() {
        return "No tasks found.".to_string();
    }

    let total = filtered.len();
    let done_count = filtered
        .iter()
        .filter(|t| t.status == TaskStatus::Done)
        .count();

    let mut lines = vec![format!("Tasks ({} total, {} done):", total, done_count)];
    for task in &filtered {
        lines.push(format!(
            "  #{} [{}] {}",
            task.id,
            task.status.as_str(),
            task.description
        ));
    }
    lines.join("\n")
}

/// task_clear — remove tasks (all or done-only) for a session
pub async fn clear(args: &Value) -> String {
    let session = resolve_session(args);
    let done_only = args["done_only"].as_bool().unwrap_or(false);
    let mut tasks = load_tasks(&session).await;

    let before = tasks.len();
    if done_only {
        tasks.retain(|t| t.status != TaskStatus::Done);
    } else {
        tasks.clear();
    }
    let removed = before - tasks.len();
    save_tasks(&session, &tasks).await;
    format!(
        "Cleared {} task{}",
        removed,
        if removed == 1 { "" } else { "s" }
    )
}
