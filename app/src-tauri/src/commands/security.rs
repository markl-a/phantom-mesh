use serde_json::Value;
use tauri::State;

use crate::commands::settings::AppConfigState;
use crate::commands::HttpClient;

#[tauri::command]
pub async fn get_audit_log(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    risk_level: Option<String>,
    limit: Option<u32>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let url = format!("{}/audit", config.hub_url);

    let mut query_params: Vec<(&str, String)> = Vec::new();
    if let Some(ref rl) = risk_level {
        query_params.push(("risk_level", rl.clone()));
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
