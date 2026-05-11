use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use crate::commands::settings::AppConfigState;
use crate::commands::HttpClient;

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub uptime_seconds: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DashboardStatus {
    pub tools_count: usize,
    pub hands_count: usize,
    pub active_sessions: usize,
    pub cluster_nodes: usize,
    pub uptime_seconds: u64,
    pub total_requests: u64,
}

#[tauri::command]
pub async fn get_health(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
) -> Result<HealthResponse, String> {
    let config = config.read().clone();
    let url = format!("{}/health", config.hub_url);
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
pub async fn get_dashboard_status(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
) -> Result<DashboardStatus, String> {
    let config = config.read().clone();
    let url = format!("{}/api/dashboard/status", config.hub_url);
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
pub async fn get_estop_status(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let url = format!("{}/estop", config.hub_url);
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
