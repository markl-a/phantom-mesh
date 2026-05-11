use std::path::PathBuf;
use crate::AppState;

#[derive(Debug, Clone, Default)]
pub struct RuntimeConfig {
    pub config_path: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
}

pub struct PhantomMeshRuntime {
    node_id: String,
    app_state: AppState,
}

impl PhantomMeshRuntime {
    pub async fn init(config: RuntimeConfig) -> anyhow::Result<Self> {
        let node_id = format!("mac-{:08x}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().subsec_nanos());

        let mut state = AppState::new();

        // Try loading config from explicit path first, then data_dir, then ~/.phantom-mesh
        let config_paths = [
            config.config_path.clone(),
            config.data_dir.as_ref().map(|d| d.join("agents.toml")),
            dirs_home().map(|h| h.join(".phantom-mesh").join("agents.toml")),
        ];

        for path in config_paths.into_iter().flatten() {
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    state.load_config_toml(&content);
                    tracing::info!("Loaded config from {}", path.display());
                    break;
                }
            }
        }

        if let Some(ref data_dir) = config.data_dir {
            let _ = std::fs::create_dir_all(data_dir);
        }

        Ok(Self { node_id, app_state: state })
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    pub fn app_state(&self) -> &AppState {
        &self.app_state
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}
