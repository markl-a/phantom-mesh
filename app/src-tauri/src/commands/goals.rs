use serde_json::Value;
use tauri::State;

use crate::commands::settings::AppConfigState;
use crate::commands::HttpClient;

fn api_url(config: &crate::commands::settings::AppConfig, path: &str) -> String {
    format!("{}{}", config.hub_url, path)
}

#[tauri::command]
pub async fn goals_list(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    status: Option<String>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let mut url = api_url(&config, "/goals");
    if let Some(s) = status {
        url = format!("{}?status={}", url, s);
    }
    let resp = http.0.get(&url).bearer_auth(&config.auth_key)
        .send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_create(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    data: Value,
) -> Result<Value, String> {
    let config = config.read().clone();
    let resp = http.0.post(&api_url(&config, "/goals")).bearer_auth(&config.auth_key)
        .json(&data).send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_get(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    id: String,
) -> Result<Value, String> {
    let config = config.read().clone();
    let resp = http.0.get(&api_url(&config, &format!("/goals/{}", id))).bearer_auth(&config.auth_key)
        .send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_update(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    id: String,
    data: Value,
) -> Result<Value, String> {
    let config = config.read().clone();
    let resp = http.0.put(&api_url(&config, &format!("/goals/{}", id))).bearer_auth(&config.auth_key)
        .json(&data).send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_delete(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    id: String,
) -> Result<Value, String> {
    let config = config.read().clone();
    let resp = http.0.delete(&api_url(&config, &format!("/goals/{}", id))).bearer_auth(&config.auth_key)
        .send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_progress(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    id: String,
) -> Result<Value, String> {
    let config = config.read().clone();
    let resp = http.0.get(&api_url(&config, &format!("/goals/{}/progress", id))).bearer_auth(&config.auth_key)
        .send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_today(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let resp = http.0.get(&api_url(&config, "/goals/today")).bearer_auth(&config.auth_key)
        .send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_summary(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let resp = http.0.get(&api_url(&config, "/goals/summary")).bearer_auth(&config.auth_key)
        .send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_milestones(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    id: String,
) -> Result<Value, String> {
    let config = config.read().clone();
    let resp = http.0.get(&api_url(&config, &format!("/goals/{}/milestones", id))).bearer_auth(&config.auth_key)
        .send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_milestone_add(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    goal_id: String,
    data: Value,
) -> Result<Value, String> {
    let config = config.read().clone();
    let resp = http.0.post(&api_url(&config, &format!("/goals/{}/milestones", goal_id))).bearer_auth(&config.auth_key)
        .json(&data).send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_milestone_toggle(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    goal_id: String,
    milestone_id: String,
) -> Result<Value, String> {
    let config = config.read().clone();
    let resp = http.0.post(&api_url(&config, &format!("/goals/{}/milestones/{}/toggle", goal_id, milestone_id)))
        .bearer_auth(&config.auth_key)
        .send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_recurring_tasks(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    id: String,
) -> Result<Value, String> {
    let config = config.read().clone();
    let resp = http.0.get(&api_url(&config, &format!("/goals/{}/recurring", id))).bearer_auth(&config.auth_key)
        .send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_recurring_add(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    goal_id: String,
    data: Value,
) -> Result<Value, String> {
    let config = config.read().clone();
    let resp = http.0.post(&api_url(&config, &format!("/goals/{}/recurring", goal_id))).bearer_auth(&config.auth_key)
        .json(&data).send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_recurring_complete(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    goal_id: String,
    task_id: String,
) -> Result<Value, String> {
    let config = config.read().clone();
    let resp = http.0.post(&api_url(&config, &format!("/goals/{}/recurring/{}/complete", goal_id, task_id)))
        .bearer_auth(&config.auth_key)
        .send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_checkin_add(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    goal_id: String,
    data: Value,
) -> Result<Value, String> {
    let config = config.read().clone();
    let resp = http.0.post(&api_url(&config, &format!("/goals/{}/checkins", goal_id))).bearer_auth(&config.auth_key)
        .json(&data).send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_checkins(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    id: String,
    limit: Option<i32>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let lim = limit.unwrap_or(20);
    let resp = http.0.get(&api_url(&config, &format!("/goals/{}/checkins?limit={}", id, lim)))
        .bearer_auth(&config.auth_key)
        .send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_mood_trend(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    id: String,
    days: Option<i32>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let d = days.unwrap_or(30);
    let resp = http.0.get(&api_url(&config, &format!("/goals/{}/mood-trend?days={}", id, d)))
        .bearer_auth(&config.auth_key)
        .send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_weekly_summary(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let resp = http.0.get(&api_url(&config, "/goals/weekly-summary"))
        .bearer_auth(&config.auth_key)
        .send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn goals_global_mood(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    days: Option<i32>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let d = days.unwrap_or(30);
    let resp = http.0.get(&api_url(&config, &format!("/goals/mood?days={}", d)))
        .bearer_auth(&config.auth_key)
        .send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}
