use serde_json::Value;
use tauri::State;

use crate::commands::settings::AppConfigState;
use crate::commands::HttpClient;

#[tauri::command]
pub async fn get_memory_observations(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    query: Option<String>,
    limit: Option<u32>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let url = format!("{}/memory/observations", config.hub_url);

    let mut query_params: Vec<(&str, String)> = Vec::new();
    if let Some(ref q) = query {
        query_params.push(("query", q.clone()));
    }
    if let Some(l) = limit {
        query_params.push(("limit", l.to_string()));
    }

    let resp = http
        .0
        .get(&url)
        .bearer_auth(&config.auth_key)
        .query(&query_params)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_memory_stats(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let url = format!("{}/memory/observations/stats", config.hub_url);
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

#[tauri::command]
pub async fn search_memory(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    query: String,
) -> Result<Value, String> {
    let config = config.read().clone();
    let url = format!("{}/memory/observations", config.hub_url);
    let resp = http
        .0
        .get(&url)
        .bearer_auth(&config.auth_key)
        .query(&[("query", &query)])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}
