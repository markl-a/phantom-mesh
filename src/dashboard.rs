// Dashboard — embedded HTML Kanban-style task board + system status
// No React/Vue — pure HTML + CSS, rendered server-side

use axum::response::Html;

use crate::task_queue::{Task, TaskStatus};

/// Render the full dashboard HTML page
pub fn render(
    tasks: &[Task],
    tools: &[String],
    agents: &[String],
    ollama_status: &str,
    token: &str,
    uptime_secs: u64,
    active_chats: usize,
) -> Html<String> {
    let pending: Vec<&Task> = tasks.iter().filter(|t| t.status == TaskStatus::Pending).collect();
    let running: Vec<&Task> = tasks.iter().filter(|t| t.status == TaskStatus::Running).collect();
    let done: Vec<&Task> = tasks.iter().filter(|t| t.status == TaskStatus::Done).collect();
    let failed: Vec<&Task> = tasks.iter().filter(|t| t.status == TaskStatus::Failed).collect();

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="zh-TW">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Clawtex Dashboard</title>
<style>
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
    background: #0d1117;
    color: #c9d1d9;
    min-height: 100vh;
}}
.header {{
    background: #161b22;
    border-bottom: 1px solid #30363d;
    padding: 16px 24px;
    display: flex;
    justify-content: space-between;
    align-items: center;
}}
.header h1 {{
    font-size: 20px;
    color: #58a6ff;
}}
.header .status {{
    display: flex;
    gap: 16px;
    font-size: 13px;
}}
.header .status .dot {{
    width: 8px;
    height: 8px;
    border-radius: 50%;
    display: inline-block;
    margin-right: 4px;
}}
.dot-green {{ background: #3fb950; }}
.dot-yellow {{ background: #d29922; }}
.dot-red {{ background: #f85149; }}
.info-bar {{
    background: #161b22;
    border-bottom: 1px solid #30363d;
    padding: 10px 24px;
    display: flex;
    gap: 24px;
    font-size: 12px;
    color: #8b949e;
    flex-wrap: wrap;
}}
.info-bar span {{ white-space: nowrap; }}
.board {{
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 12px;
    padding: 16px;
    max-width: 1400px;
    margin: 0 auto;
}}
@media (max-width: 768px) {{
    .board {{ grid-template-columns: 1fr 1fr; }}
}}
@media (max-width: 480px) {{
    .board {{ grid-template-columns: 1fr; }}
}}
.column {{
    background: #161b22;
    border: 1px solid #30363d;
    border-radius: 8px;
    min-height: 200px;
}}
.column-header {{
    padding: 12px 16px;
    border-bottom: 1px solid #30363d;
    font-weight: 600;
    font-size: 14px;
    display: flex;
    justify-content: space-between;
    align-items: center;
}}
.column-header .count {{
    background: #30363d;
    color: #8b949e;
    padding: 2px 8px;
    border-radius: 10px;
    font-size: 12px;
}}
.col-pending .column-header {{ color: #d29922; }}
.col-running .column-header {{ color: #58a6ff; }}
.col-done .column-header {{ color: #3fb950; }}
.col-failed .column-header {{ color: #f85149; }}
.column-body {{
    padding: 8px;
}}
.card {{
    background: #0d1117;
    border: 1px solid #30363d;
    border-radius: 6px;
    padding: 10px 12px;
    margin-bottom: 8px;
    font-size: 13px;
}}
.card:hover {{
    border-color: #58a6ff;
}}
.card-title {{
    font-weight: 500;
    margin-bottom: 4px;
    word-break: break-word;
}}
.card-meta {{
    color: #8b949e;
    font-size: 11px;
    display: flex;
    justify-content: space-between;
}}
.card-result {{
    color: #8b949e;
    font-size: 11px;
    margin-top: 6px;
    max-height: 60px;
    overflow: hidden;
    word-break: break-word;
}}
.refresh {{
    color: #8b949e;
    font-size: 12px;
    text-align: center;
    padding: 12px;
}}
.refresh a {{
    color: #58a6ff;
    text-decoration: none;
}}
</style>
</head>
<body>
<div class="header">
    <h1>Clawtex Dashboard</h1>
    <div class="status">
        <span><span class="dot dot-green"></span>Daemon online</span>
        <span><span class="dot {ollama_dot}"></span>Ollama: {ollama_status}</span>
        <span>{total_tasks} tasks</span>
    </div>
</div>
<div class="info-bar">
    <span>Uptime: {uptime}</span>
    <span>Chats: {active_chats}</span>
    <span>Tools: {tools_list}</span>
    <span>Agents: {agents_list}</span>
    <span>v{version}</span>
</div>
<div class="board">
    <div class="column col-pending">
        <div class="column-header">Pending <span class="count">{pending_count}</span></div>
        <div class="column-body">{pending_cards}</div>
    </div>
    <div class="column col-running">
        <div class="column-header">Running <span class="count">{running_count}</span></div>
        <div class="column-body">{running_cards}</div>
    </div>
    <div class="column col-done">
        <div class="column-header">Done <span class="count">{done_count}</span></div>
        <div class="column-body">{done_cards}</div>
    </div>
    <div class="column col-failed">
        <div class="column-header">Failed <span class="count">{failed_count}</span></div>
        <div class="column-body">{failed_cards}</div>
    </div>
</div>
<div class="refresh">
    Auto-refresh in 15s &middot; <a href="/dashboard?token={token}">Refresh now</a>
</div>
<script>setTimeout(() => location.reload(), 15000);</script>
</body>
</html>"#,
        ollama_dot = if ollama_status == "connected" { "dot-green" } else { "dot-red" },
        ollama_status = ollama_status,
        total_tasks = tasks.len(),
        uptime = format_uptime(uptime_secs),
        active_chats = active_chats,
        tools_list = tools.join(", "),
        agents_list = agents.join(", "),
        version = env!("CARGO_PKG_VERSION"),
        pending_count = pending.len(),
        running_count = running.len(),
        done_count = done.len(),
        failed_count = failed.len(),
        pending_cards = render_cards(&pending),
        running_cards = render_cards(&running),
        done_cards = render_cards(&done),
        failed_cards = render_cards(&failed),
        token = token,
    );

    Html(html)
}

fn render_cards(tasks: &[&Task]) -> String {
    if tasks.is_empty() {
        return r#"<div style="color:#484f58;font-size:12px;padding:8px;text-align:center;">No tasks</div>"#.to_string();
    }

    tasks
        .iter()
        .map(|t| {
            let title_display = html_escape(&t.title);
            let time = &t.created_at[..t.created_at.len().min(19)]; // trim to YYYY-MM-DDTHH:MM:SS
            let id_short = &t.task_id[..t.task_id.len().min(8)];

            let result_html = if let Some(ref result) = t.result {
                let preview = if result.chars().count() > 120 {
                    let end = result.char_indices().nth(120).map(|(i, _)| i).unwrap_or(result.len());
                    format!("{}...", html_escape(&result[..end]))
                } else {
                    html_escape(result)
                };
                format!(r#"<div class="card-result">{}</div>"#, preview)
            } else {
                String::new()
            };

            format!(
                r#"<div class="card">
<div class="card-title">{title}</div>
<div class="card-meta"><span>{id}</span><span>{time}</span></div>
{result}
</div>"#,
                title = title_display,
                id = id_short,
                time = time,
                result = result_html,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_uptime(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if h > 0 {
        format!("{}h {}m", h, m)
    } else {
        format!("{}m", m)
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_queue::{Task, TaskStatus, TaskPriority};

    fn make_task(id: &str, title: &str, status: TaskStatus, result: Option<&str>) -> Task {
        Task {
            task_id: id.to_string(),
            title: title.to_string(),
            prompt: "test prompt".to_string(),
            status,
            result: result.map(|s| s.to_string()),
            strategy_used: None,
            feedback_score: None,
            priority: TaskPriority::Normal,
            idempotency_key: None,
            created_at: "2026-03-17T10:00:00+00:00".to_string(),
            updated_at: "2026-03-17T10:00:00+00:00".to_string(),
        }
    }

    #[test]
    fn test_render_empty_tasks() {
        let html = render(
            &[],
            &["shell".to_string(), "web_search".to_string()],
            &["master".to_string()],
            "connected",
            "test-token",
            3600,
            2,
        );
        let body = html.0;
        assert!(body.contains("Clawtex Dashboard"));
        assert!(body.contains("0 tasks"));
        assert!(body.contains("No tasks")); // empty columns show "No tasks"
    }

    #[test]
    fn test_render_with_tasks_in_all_columns() {
        let tasks = vec![
            make_task("aaaa1111-0000-0000-0000-000000000000", "Pending task", TaskStatus::Pending, None),
            make_task("bbbb2222-0000-0000-0000-000000000000", "Running task", TaskStatus::Running, None),
            make_task("cccc3333-0000-0000-0000-000000000000", "Done task", TaskStatus::Done, Some("result ok")),
            make_task("dddd4444-0000-0000-0000-000000000000", "Failed task", TaskStatus::Failed, Some("error: timeout")),
        ];
        let html = render(
            &tasks,
            &["shell".to_string()],
            &["master".to_string()],
            "connected",
            "tok",
            7200,
            5,
        );
        let body = html.0;
        assert!(body.contains("4 tasks"));
        assert!(body.contains("Pending task"));
        assert!(body.contains("Running task"));
        assert!(body.contains("Done task"));
        assert!(body.contains("Failed task"));
        assert!(body.contains("result ok"));
        assert!(body.contains("error: timeout"));
    }

    #[test]
    fn test_render_ollama_connected_green_dot() {
        let html = render(&[], &[], &[], "connected", "t", 0, 0);
        let body = html.0;
        assert!(body.contains("dot-green"));
        assert!(body.contains("Ollama: connected"));
    }

    #[test]
    fn test_render_ollama_disconnected_red_dot() {
        let html = render(&[], &[], &[], "disconnected", "t", 0, 0);
        let body = html.0;
        assert!(body.contains("dot-red"));
        assert!(body.contains("Ollama: disconnected"));
    }

    #[test]
    fn test_render_uptime_hours() {
        let html = render(&[], &[], &[], "connected", "t", 7260, 0);
        let body = html.0;
        assert!(body.contains("2h 1m"));
    }

    #[test]
    fn test_render_uptime_minutes_only() {
        let html = render(&[], &[], &[], "connected", "t", 300, 0);
        let body = html.0;
        assert!(body.contains("5m"));
    }

    #[test]
    fn test_render_tools_list() {
        let html = render(
            &[],
            &["shell".to_string(), "file_read".to_string(), "web_search".to_string()],
            &[],
            "connected",
            "t",
            0,
            0,
        );
        let body = html.0;
        assert!(body.contains("shell, file_read, web_search"));
    }

    #[test]
    fn test_render_agents_list() {
        let html = render(
            &[],
            &[],
            &["master".to_string(), "coder".to_string()],
            "connected",
            "t",
            0,
            0,
        );
        let body = html.0;
        assert!(body.contains("master, coder"));
    }

    #[test]
    fn test_render_refresh_link_with_token() {
        let html = render(&[], &[], &[], "connected", "my-secret-token", 0, 0);
        let body = html.0;
        assert!(body.contains("/dashboard?token=my-secret-token"));
    }

    #[test]
    fn test_render_active_chats() {
        let html = render(&[], &[], &[], "connected", "t", 0, 42);
        let body = html.0;
        assert!(body.contains("Chats: 42"));
    }

    #[test]
    fn test_format_uptime_zero() {
        assert_eq!(format_uptime(0), "0m");
    }

    #[test]
    fn test_format_uptime_hours_and_minutes() {
        assert_eq!(format_uptime(3661), "1h 1m");
    }

    #[test]
    fn test_format_uptime_exact_hour() {
        assert_eq!(format_uptime(7200), "2h 0m");
    }

    #[test]
    fn test_html_escape_special_chars() {
        assert_eq!(html_escape("<script>alert('xss')</script>"), "&lt;script&gt;alert('xss')&lt;/script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape("\"quoted\""), "&quot;quoted&quot;");
    }

    #[test]
    fn test_html_escape_no_change() {
        assert_eq!(html_escape("hello world"), "hello world");
    }

    #[test]
    fn test_render_cards_empty() {
        let cards = render_cards(&[]);
        assert!(cards.contains("No tasks"));
    }

    #[test]
    fn test_render_cards_with_task() {
        let task = make_task("abcd1234-5678-9012-3456-789012345678", "My Task", TaskStatus::Done, Some("Done!"));
        let cards = render_cards(&[&task]);
        assert!(cards.contains("My Task"));
        assert!(cards.contains("abcd1234"));
        assert!(cards.contains("Done!"));
    }

    #[test]
    fn test_render_cards_html_escapes_title() {
        let task = make_task("abcd1234-5678-9012-3456-789012345678", "<b>Bold</b>", TaskStatus::Pending, None);
        let cards = render_cards(&[&task]);
        assert!(cards.contains("&lt;b&gt;Bold&lt;/b&gt;"));
        assert!(!cards.contains("<b>Bold</b>"));
    }

    #[test]
    fn test_render_cards_long_result_truncated() {
        let long_result = "x".repeat(200);
        let task = make_task("abcd1234-5678-9012-3456-789012345678", "Long", TaskStatus::Done, Some(&long_result));
        let cards = render_cards(&[&task]);
        assert!(cards.contains("..."));
        // The full 200-char result should not appear
        assert!(!cards.contains(&long_result));
    }

    #[test]
    fn test_render_column_counts() {
        let tasks = vec![
            make_task("a1111111-0000-0000-0000-000000000000", "P1", TaskStatus::Pending, None),
            make_task("a2222222-0000-0000-0000-000000000000", "P2", TaskStatus::Pending, None),
            make_task("b1111111-0000-0000-0000-000000000000", "R1", TaskStatus::Running, None),
            make_task("c1111111-0000-0000-0000-000000000000", "D1", TaskStatus::Done, None),
            make_task("c2222222-0000-0000-0000-000000000000", "D2", TaskStatus::Done, None),
            make_task("c3333333-0000-0000-0000-000000000000", "D3", TaskStatus::Done, None),
        ];
        let html = render(&tasks, &[], &[], "connected", "t", 0, 0);
        let body = html.0;
        // Check column header counts
        assert!(body.contains(r#"Pending <span class="count">2</span>"#));
        assert!(body.contains(r#"Running <span class="count">1</span>"#));
        assert!(body.contains(r#"Done <span class="count">3</span>"#));
        assert!(body.contains(r#"Failed <span class="count">0</span>"#));
    }
}
