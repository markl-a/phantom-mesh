use serde_json::Value;
use tauri::State;

use crate::commands::settings::AppConfigState;
use crate::commands::HttpClient;

#[tauri::command]
pub async fn get_task_history(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
) -> Result<Value, String> {
    let config = config.read().clone();
    // `/tasks` is the real persisted task queue (core/src/main.rs `tasks_list`,
    // backed by the `tasks` SQLite table). The older `/task/history` route is a
    // hardcoded legacy stub that always returns `{"tasks": []}`, so proxying it
    // rendered the dashboard TasksPanel permanently empty. Rows are raw
    // `pm_types::TaskRecord`s; the panel maps them to its display shape.
    let url = format!("{}/tasks?limit=50", config.hub_url);
    let resp = http
        .0
        .get(&url)
        .bearer_auth(&config.auth_key)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}
