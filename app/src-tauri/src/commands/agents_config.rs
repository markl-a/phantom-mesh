use serde_json::Value;

/// Read agents.toml and return it serialized as JSON
#[tauri::command]
pub async fn read_agents_toml() -> Result<Value, String> {
    let config = phantom_mesh::config::read_agents_toml()
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&config).map_err(|e| e.to_string())
}

/// Write agents.toml from a JSON representation of AgentsConfig
#[tauri::command]
pub async fn write_agents_toml(content: String) -> Result<(), String> {
    let config: phantom_mesh::AgentsConfig = serde_json::from_str(&content)
        .map_err(|e| e.to_string())?;
    phantom_mesh::write_agents_toml(&config)
        .map_err(|e| e.to_string())
}

/// Get list of configured providers with their status
#[tauri::command]
pub async fn get_providers() -> Result<Vec<Value>, String> {
    let config = phantom_mesh::config::read_agents_toml()
        .map_err(|e| e.to_string())?;
    let summaries = phantom_mesh::list_providers(&config);
    Ok(summaries.into_iter()
        .map(|s| serde_json::to_value(s).unwrap_or_default())
        .collect())
}

/// Set the API key for a provider directly in agents.toml
#[tauri::command]
pub async fn set_provider_api_key(provider_name: String, api_key: String) -> Result<(), String> {
    let mut config = phantom_mesh::config::read_agents_toml()
        .map_err(|e| e.to_string())?;

    if let Some(provider) = config.providers.get_mut(&provider_name) {
        provider.api_key = Some(api_key);
    } else {
        return Err(format!("Provider '{}' not found", provider_name));
    }

    phantom_mesh::write_agents_toml(&config).map_err(|e| e.to_string())
}
