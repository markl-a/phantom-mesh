use serde_json::Value;
use tauri::State;

use crate::commands::settings::AppConfigState;
use crate::commands::HttpClient;

#[tauri::command]
pub async fn get_network_discovery(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let url = format!("{}/networking/discovered", config.hub_url);
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
pub async fn get_network_routes(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let url = format!("{}/networking/routes", config.hub_url);
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
pub async fn get_network_status(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let url = format!("{}/networking/status", config.hub_url);
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
