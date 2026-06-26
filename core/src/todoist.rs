//! Todoist REST client — the partner's "goal model" + DO-action backend.
//!
//! Two consumers, one client so token resolution and the REST surface live in a
//! single place:
//!   - DO-actions: the `todoist_add_task` / `todoist_list_tasks` built-in tools
//!     (see `tools::todoist`) so a "把這個記到 Todoist" message actually creates
//!     a task (NORTH-STAR 使用①: 記東西).
//!   - Goal model: [`crate::partner::daily_reflection`] (via
//!     [`crate::partner::fetch_goal_model`]) injects the user's real open tasks +
//!     projects into the daily alignment reflection, replacing the old
//!     `<unknown>` placeholder (使用⑥ #8).
//!
//! Auth is a Todoist API token (Settings → Integrations → Developer). It is
//! NEVER hardcoded: resolved from `[tools] todoist_api_token` in agents.toml or,
//! failing that, the `TODOIST_API_TOKEN` env var. When no token is configured,
//! the goal-model fetch degrades gracefully (the reflection falls back to the
//! "goals not connected" line) and the tools return a clear setup hint.
//!
//! Uses the Todoist API v1 (`https://api.todoist.com/api/v1`) — the legacy
//! `rest/v2` surface is deprecated and now returns HTTP 410 Gone. The list
//! endpoints return a paginated `{"results": [...], "next_cursor": ...}`
//! envelope; we consume the first page (full `next_cursor` pagination is a
//! TODO). Reuses the crate's existing `reqwest` (rustls) stack — no new dep.

use std::time::Duration;

use serde::Deserialize;

const REST_BASE_DEFAULT: &str = "https://api.todoist.com/api/v1";
const TIMEOUT_SECS: u64 = 20;

/// The REST API base URL. Defaults to the real Todoist endpoint; overridable via
/// `TODOIST_API_BASE` so integration tests can point at a local mock server
/// (mirrors the `base_url` override providers already support). The trailing
/// slash, if any, is trimmed so `{base}/tasks` is always well-formed.
fn rest_base() -> String {
    match std::env::var("TODOIST_API_BASE") {
        Ok(b) if !b.trim().is_empty() => b.trim().trim_end_matches('/').to_string(),
        _ => REST_BASE_DEFAULT.to_string(),
    }
}

/// Resolve the Todoist API token: config first (`[tools] todoist_api_token`),
/// then the `TODOIST_API_TOKEN` env var. Returns `None` when neither is set, so
/// callers can degrade gracefully instead of erroring.
pub fn resolve_token(config: Option<&crate::config::ToolsConfig>) -> Option<String> {
    if let Some(cfg) = config {
        if let Some(t) = cfg.todoist_api_token.as_deref() {
            if !t.trim().is_empty() {
                return Some(t.trim().to_string());
            }
        }
    }
    match std::env::var("TODOIST_API_TOKEN") {
        Ok(t) if !t.trim().is_empty() => Some(t.trim().to_string()),
        _ => None,
    }
}

/// A Todoist task, narrowed to the fields the partner cares about.
#[derive(Debug, Clone, Deserialize)]
pub struct Task {
    pub id: String,
    pub content: String,
    #[serde(default)]
    pub priority: u8,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub due: Option<Due>,
    /// v1 renamed v2's `is_completed` to `checked`; accept both so the
    /// field (and every caller) keeps the `is_completed` name.
    #[serde(default, alias = "checked")]
    pub is_completed: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Due {
    #[serde(default)]
    pub string: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
}

/// A Todoist project, narrowed to the fields the partner cares about.
#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
}

/// API v1 list envelope: `{ "results": [...], "next_cursor": ... }`.
/// v1 wraps list responses (v2 returned a bare array). `next_cursor` is
/// captured for future pagination; we currently read only the first page.
#[derive(Debug, Clone, Deserialize)]
struct Paginated<T> {
    #[serde(default = "Vec::new")]
    results: Vec<T>,
    #[serde(default)]
    #[allow(dead_code)]
    next_cursor: Option<String>,
}

fn client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .use_rustls_tls()
        .user_agent("phantom-mesh-partner/0.6")
        .build()
}

/// List active (incomplete) tasks. `filter` is an optional Todoist filter query
/// (e.g. `"today | overdue"`); when `None`, all active tasks are returned.
pub async fn list_tasks(token: &str, filter: Option<&str>) -> anyhow::Result<Vec<Task>> {
    let c = client()?;
    let base = rest_base();
    let mut req = c.get(format!("{base}/tasks")).bearer_auth(token);
    if let Some(f) = filter {
        if !f.trim().is_empty() {
            req = req.query(&[("filter", f)]);
        }
    }
    let resp = req.send().await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("Todoist GET /tasks failed: HTTP {} {}", status.as_u16(), body);
    }
    let page: Paginated<Task> = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("parse /tasks response: {e}: {body}"))?;
    // TODO: follow `page.next_cursor` to fetch remaining pages.
    Ok(page.results)
}

/// List the user's projects.
pub async fn list_projects(token: &str) -> anyhow::Result<Vec<Project>> {
    let c = client()?;
    let base = rest_base();
    let resp = c
        .get(format!("{base}/projects"))
        .bearer_auth(token)
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!(
            "Todoist GET /projects failed: HTTP {} {}",
            status.as_u16(),
            body
        );
    }
    let page: Paginated<Project> = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("parse /projects response: {e}: {body}"))?;
    // TODO: follow `page.next_cursor` to fetch remaining pages.
    Ok(page.results)
}

/// Create a task. `content` is required; `due_string` (natural language, e.g.
/// "tomorrow 9am"), `priority` (Todoist 1..=4, where 4 is highest), and
/// `project_id` are optional. Returns the created [`Task`].
pub async fn add_task(
    token: &str,
    content: &str,
    due_string: Option<&str>,
    priority: Option<u8>,
    project_id: Option<&str>,
) -> anyhow::Result<Task> {
    let c = client()?;
    let mut body = serde_json::Map::new();
    body.insert("content".into(), serde_json::Value::String(content.to_string()));
    if let Some(d) = due_string {
        if !d.trim().is_empty() {
            body.insert("due_string".into(), serde_json::Value::String(d.to_string()));
        }
    }
    if let Some(p) = priority {
        // Todoist priority is 1..=4; clamp so a bad input never 400s.
        let p = p.clamp(1, 4);
        body.insert("priority".into(), serde_json::Value::from(p));
    }
    if let Some(pid) = project_id {
        if !pid.trim().is_empty() {
            body.insert("project_id".into(), serde_json::Value::String(pid.to_string()));
        }
    }

    let base = rest_base();
    let resp = c
        .post(format!("{base}/tasks"))
        .bearer_auth(token)
        .json(&serde_json::Value::Object(body))
        .send()
        .await?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("Todoist POST /tasks failed: HTTP {} {}", status.as_u16(), text);
    }
    // API v1 POST /tasks returns the created task object directly (no
    // `results` envelope), so deserialize it as a single Task.
    let task: Task = serde_json::from_str(&text)
        .map_err(|e| anyhow::anyhow!("parse created task: {e}: {text}"))?;
    Ok(task)
}

/// Complete (close) a task by id — the partner's bounded "mark it done" DO-action.
///
/// Hits Todoist API v1 `POST /tasks/{id}/close`, which moves the task to the
/// completed state and returns `204 No Content` on success. This is the ONLY
/// write this function can perform: it cannot create, delete, or mutate
/// arbitrary fields, and it cannot run any command — it just closes one task the
/// caller names by id. `task_id` is validated so a malformed id can never escape
/// the `/tasks/{id}/close` shape into another endpoint.
pub async fn complete_task(token: &str, task_id: &str) -> anyhow::Result<()> {
    let task_id = task_id.trim();
    if task_id.is_empty() {
        anyhow::bail!("task_id must not be empty");
    }
    // A Todoist task id is a short alphanumeric string. Reject anything with
    // path separators or other URL-significant characters so the id cannot
    // redirect the request to a different path/endpoint.
    if !task_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        anyhow::bail!("task_id contains invalid characters: {task_id:?}");
    }
    let c = client()?;
    let base = rest_base();
    let resp = c
        .post(format!("{base}/tasks/{task_id}/close"))
        .bearer_auth(token)
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "Todoist POST /tasks/{}/close failed: HTTP {} {}",
            task_id,
            status.as_u16(),
            body
        );
    }
    Ok(())
}

/// Build a compact, human-readable "goal model" block for the daily reflection
/// prompt from the user's open tasks + projects. Highest-priority and
/// soonest-due tasks first; capped at `max_tasks` lines so the prompt stays
/// small. Returns `None` when there is nothing actionable to show.
pub fn format_goal_model(tasks: &[Task], projects: &[Project], max_tasks: usize) -> Option<String> {
    use std::collections::HashMap;
    let proj_by_id: HashMap<&str, &str> = projects
        .iter()
        .map(|p| (p.id.as_str(), p.name.as_str()))
        .collect();

    let mut open: Vec<&Task> = tasks.iter().filter(|t| !t.is_completed).collect();
    if open.is_empty() {
        return None;
    }
    // Todoist priority: 4 = highest. Sort priority desc, then by due date asc
    // (tasks with a due date ahead of undated ones), then content for stability.
    open.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| due_sort_key(a).cmp(&due_sort_key(b)))
            .then_with(|| a.content.cmp(&b.content))
    });

    let mut lines: Vec<String> = Vec::new();
    for t in open.iter().take(max_tasks) {
        let mut line = format!("- {}", t.content.trim());
        if let Some(due) = t.due.as_ref().and_then(|d| d.string.clone().or(d.date.clone())) {
            if !due.is_empty() {
                line.push_str(&format!(" (due {due})"));
            }
        }
        if let Some(name) = t
            .project_id
            .as_deref()
            .and_then(|pid| proj_by_id.get(pid))
        {
            line.push_str(&format!(" [{name}]"));
        }
        lines.push(line);
    }

    let extra = open.len().saturating_sub(lines.len());
    let mut block = format!(
        "my Todoist goals — {} open task{} ({} project{}):",
        open.len(),
        if open.len() == 1 { "" } else { "s" },
        projects.len(),
        if projects.len() == 1 { "" } else { "s" },
    );
    for l in &lines {
        block.push('\n');
        block.push_str(l);
    }
    if extra > 0 {
        block.push_str(&format!("\n- (and {extra} more)"));
    }
    Some(block)
}

/// Sort key for a task's due date: dated tasks (by ISO date / string) sort
/// before undated ones (which get a far-future sentinel).
fn due_sort_key(t: &Task) -> String {
    t.due
        .as_ref()
        .and_then(|d| d.date.clone().or_else(|| d.string.clone()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "~".to_string()) // '~' > any ascii digit → undated last
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, content: &str, priority: u8, due: Option<&str>) -> Task {
        Task {
            id: id.into(),
            content: content.into(),
            priority,
            project_id: None,
            due: due.map(|d| Due {
                string: Some(d.into()),
                date: Some(d.into()),
            }),
            is_completed: false,
        }
    }

    #[test]
    fn format_goal_model_orders_by_priority_then_due() {
        let tasks = vec![
            task("1", "low prio no due", 1, None),
            task("2", "high prio", 4, Some("tomorrow")),
            task("3", "high prio earlier", 4, Some("2026-06-01")),
        ];
        let projects = vec![Project {
            id: "p1".into(),
            name: "Work".into(),
        }];
        let block = format_goal_model(&tasks, &projects, 10).expect("non-empty");
        // Header reflects the real counts.
        assert!(block.contains("3 open tasks"), "header counts tasks: {block}");
        // p4 tasks come before the p1 task.
        let hi = block.find("high prio").unwrap();
        let lo = block.find("low prio").unwrap();
        assert!(hi < lo, "higher priority listed first:\n{block}");
        // Due strings are surfaced.
        assert!(block.contains("(due tomorrow)") || block.contains("(due 2026-06-01)"));
    }

    #[test]
    fn format_goal_model_caps_and_reports_extra() {
        let tasks: Vec<Task> = (0..5)
            .map(|i| task(&i.to_string(), &format!("task {i}"), 1, None))
            .collect();
        let block = format_goal_model(&tasks, &[], 2).expect("non-empty");
        assert!(block.contains("and 3 more"), "caps at max_tasks: {block}");
    }

    #[test]
    fn format_goal_model_empty_is_none() {
        assert!(format_goal_model(&[], &[], 10).is_none());
        // Completed tasks don't count as open goals.
        let done = Task {
            id: "x".into(),
            content: "done".into(),
            priority: 1,
            project_id: None,
            due: None,
            is_completed: true,
        };
        assert!(format_goal_model(&[done], &[], 10).is_none());
    }

    #[test]
    fn parses_v1_results_envelope_into_tasks() {
        // Real-shaped API v1 GET /tasks payload: a `results` array of v1
        // tasks using `checked` (not v2's `is_completed`).
        let body = r#"{
            "results": [
                {
                    "id": "7001",
                    "content": "buy milk",
                    "project_id": "p9",
                    "priority": 4,
                    "checked": false,
                    "due": {"string": "tomorrow", "date": "2026-06-07"}
                },
                {
                    "id": "7002",
                    "content": "done thing",
                    "priority": 1,
                    "checked": true
                }
            ],
            "next_cursor": "abc123"
        }"#;
        // Mirror list_tasks' parse step (the network call can't run in a unit
        // test, so we assert on the deserialization that defines the contract).
        let page: Paginated<Task> =
            serde_json::from_str(body).expect("v1 envelope parses");
        let tasks = page.results;
        assert_eq!(tasks.len(), 2, "both results parsed");
        assert_eq!(page.next_cursor.as_deref(), Some("abc123"));

        let t0 = &tasks[0];
        assert_eq!(t0.id, "7001");
        assert_eq!(t0.content, "buy milk");
        assert_eq!(t0.priority, 4, "v1 priority maps unchanged (4=highest)");
        assert_eq!(t0.project_id.as_deref(), Some("p9"));
        assert!(!t0.is_completed, "checked:false -> is_completed:false");
        assert_eq!(
            t0.due.as_ref().and_then(|d| d.date.as_deref()),
            Some("2026-06-07")
        );

        // `checked: true` must map onto the existing is_completed field.
        assert!(tasks[1].is_completed, "checked:true -> is_completed:true");
    }

    #[test]
    fn resolve_token_prefers_config_over_env() {
        let _g = crate::env_lock::acquire();
        std::env::set_var("TODOIST_API_TOKEN", "from-env");
        let cfg = crate::config::ToolsConfig {
            todoist_api_token: Some("from-config".to_string()),
            ..Default::default()
        };
        assert_eq!(resolve_token(Some(&cfg)).as_deref(), Some("from-config"));
        // No config token → env fallback.
        assert_eq!(resolve_token(None).as_deref(), Some("from-env"));
        std::env::remove_var("TODOIST_API_TOKEN");
        assert_eq!(resolve_token(None), None);
    }

    #[tokio::test]
    async fn complete_task_rejects_empty_and_unsafe_ids() {
        // Empty id never makes a network call — it errors before building a URL.
        let err = complete_task("tok", "   ")
            .await
            .expect_err("empty id is rejected");
        assert!(
            err.to_string().contains("must not be empty"),
            "empty id error: {err}"
        );

        // Path-traversal / endpoint-escape attempts are rejected before any
        // request is made, so a malformed id can never reach a different path.
        for bad in ["123/../projects", "../foo", "1 2", "a?b", "x#y", "@home"] {
            let err = complete_task("tok", bad)
                .await
                .expect_err("unsafe id is rejected");
            assert!(
                err.to_string().contains("invalid characters"),
                "unsafe id {bad:?} error: {err}"
            );
        }
    }
}
