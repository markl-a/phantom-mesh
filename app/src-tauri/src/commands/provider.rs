use serde_json::{json, Map, Value};
use tauri::State;

use crate::commands::settings::AppConfigState;
use crate::commands::HttpClient;

fn normalize_provider_health_record(
    record: &Map<String, Value>,
    fallback_name: Option<&str>,
) -> Value {
    let name = record
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| record.get("name").and_then(Value::as_str))
        .or_else(|| record.get("display_name").and_then(Value::as_str))
        .or_else(|| record.get("provider_name").and_then(Value::as_str))
        .or(fallback_name)
        .unwrap_or("unknown");

    let is_available = record
        .get("is_available")
        .and_then(Value::as_bool)
        .or_else(|| record.get("healthy").and_then(Value::as_bool))
        .or_else(|| record.get("online").and_then(Value::as_bool))
        .or_else(|| {
            record
                .get("health")
                .and_then(Value::as_str)
                .map(|value| matches!(value, "healthy" | "online" | "ok" | "up"))
        })
        .or_else(|| {
            record
                .get("status")
                .and_then(Value::as_str)
                .map(|value| matches!(value, "healthy" | "online" | "ok" | "up"))
        })
        .unwrap_or(true);

    let raw_health = record
        .get("health")
        .and_then(Value::as_str)
        .or_else(|| record.get("status").and_then(Value::as_str))
        .unwrap_or(if is_available { "healthy" } else { "offline" });

    let health = match raw_health {
        "online" | "ok" | "up" => "healthy",
        other => other,
    };

    let mut normalized = record.clone();
    normalized.insert("id".into(), Value::String(name.to_string()));
    normalized.insert("name".into(), Value::String(name.to_string()));
    normalized.insert(
        "display_name".into(),
        Value::String(
            record
                .get("display_name")
                .and_then(Value::as_str)
                .or_else(|| record.get("name").and_then(Value::as_str))
                .or_else(|| record.get("provider_name").and_then(Value::as_str))
                .unwrap_or(name)
                .to_string(),
        ),
    );
    normalized.insert("is_available".into(), Value::Bool(is_available));
    normalized.insert("health".into(), Value::String(health.to_string()));
    normalized.insert("status".into(), Value::String(health.to_string()));
    Value::Object(normalized)
}

fn normalize_provider_health(value: Value) -> Value {
    let mut providers: Vec<Value> = Vec::new();

    match value {
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::String(name) => {
                        let display_name = name.clone();
                        let provider_id = name.clone();
                        providers.push(json!({
                            "id": provider_id,
                            "name": name,
                            "display_name": display_name,
                            "is_available": true,
                            "health": "healthy",
                            "status": "healthy",
                        }));
                    }
                    Value::Object(record) => {
                        providers.push(normalize_provider_health_record(&record, None));
                    }
                    _ => {}
                }
            }
        }
        Value::Object(obj) => {
            if let Some(Value::Array(items)) = obj.get("providers") {
                for item in items {
                    match item {
                        Value::String(name) => {
                            let display_name = name.clone();
                            let provider_id = name.clone();
                            providers.push(json!({
                                "id": provider_id,
                                "name": name,
                                "display_name": display_name,
                                "is_available": true,
                                "health": "healthy",
                                "status": "healthy",
                            }));
                        }
                        Value::Object(record) => {
                            providers.push(normalize_provider_health_record(record, None));
                        }
                        _ => {}
                    }
                }
            } else {
                for (name, item) in obj {
                    if let Value::Object(record) = item {
                        providers.push(normalize_provider_health_record(&record, Some(&name)));
                    }
                }
            }
        }
        _ => {}
    }

    json!({ "providers": providers })
}

#[tauri::command]
pub async fn get_costs(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let url = format!("{}/costs", config.hub_url);
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
pub async fn get_revenue(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let url = format!("{}/revenue", config.hub_url);
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
pub async fn get_tools(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let url = format!("{}/tools", config.hub_url);
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
pub async fn get_hands(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let url = format!("{}/hands", config.hub_url);
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
pub async fn get_provider_health(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let url = format!("{}/api/providers/health", config.hub_url);
    let resp = http
        .0
        .get(&url)
        .bearer_auth(&config.auth_key)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let payload: Value = resp.json().await.map_err(|e| e.to_string())?;
    Ok(normalize_provider_health(payload))
}
