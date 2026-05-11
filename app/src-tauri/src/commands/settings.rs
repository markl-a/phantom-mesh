use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub hub_url: String,
    pub auth_key: String,
    pub agent_name: String,
    pub auto_start: bool,
    /// Port for the managed phantom-mesh daemon (default 7878)
    pub daemon_port: u16,
    /// Optional explicit path to the phantom-mesh binary.
    /// When `None`, auto-detection is used.
    pub daemon_binary_path: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hub_url: "http://localhost:7878".to_string(),
            auth_key: String::new(),
            agent_name: "master".to_string(),
            auto_start: false,  // Don't auto-start before onboarding completes
            daemon_port: 7878,
            daemon_binary_path: None,
        }
    }
}

/// Thread-safe wrapper so auth_key (and other fields) can be updated at runtime
/// (e.g. after onboarding sets the daemon auth key).
pub struct AppConfigState(pub std::sync::RwLock<AppConfig>);

impl AppConfigState {
    pub fn new(config: AppConfig) -> Self {
        Self(std::sync::RwLock::new(config))
    }

    pub fn read(&self) -> std::sync::RwLockReadGuard<'_, AppConfig> {
        self.0.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn write(&self) -> std::sync::RwLockWriteGuard<'_, AppConfig> {
        self.0.write().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.hub_url, "http://localhost:7878");
        assert_eq!(config.daemon_port, 7878);
        assert!(!config.auto_start);
        assert!(config.daemon_binary_path.is_none());
        assert_eq!(config.agent_name, "master");
    }

    #[test]
    fn test_config_serialization_roundtrip() {
        let config = AppConfig {
            hub_url: "http://192.168.1.10:8080".to_string(),
            auth_key: "secret123".to_string(),
            agent_name: "worker-1".to_string(),
            auto_start: false,
            daemon_port: 9090,
            daemon_binary_path: Some("/opt/phantom-mesh".to_string()),
        };
        let json = serde_json::to_string(&config).unwrap();
        let restored: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.hub_url, config.hub_url);
        assert_eq!(restored.daemon_port, 9090);
        assert_eq!(restored.daemon_binary_path.unwrap(), "/opt/phantom-mesh");
    }

    #[test]
    fn test_config_state_read_write() {
        let state = AppConfigState::new(AppConfig::default());
        assert_eq!(state.read().auth_key, "");
        state.write().auth_key = "new_key".to_string();
        assert_eq!(state.read().auth_key, "new_key");
    }
}

#[tauri::command]
pub async fn get_config(
    config: tauri::State<'_, AppConfigState>,
) -> Result<AppConfig, String> {
    Ok(config.read().clone())
}

#[tauri::command]
pub async fn set_config(
    state: tauri::State<'_, AppConfigState>,
    config: AppConfig,
) -> Result<(), String> {
    tracing::info!("Config updated: hub_url={}", config.hub_url);
    *state.write() = config;
    Ok(())
}
