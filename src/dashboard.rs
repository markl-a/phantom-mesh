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
