//! Runtime bootstrap and event-loop wiring for a phantom-mesh node.
//!
//! This module owns the top-level process lifecycle: it builds the shared
//! [`AppState`], discovers and loads the node's `agents.toml` configuration,
//! and assigns a per-process node identifier. Everything else in the crate
//! reaches into the runtime through [`PhantomMeshRuntime`] to obtain the
//! initialized [`AppState`] and the node's identity.
//!
//! # Initialization flow
//!
//! 1. [`PhantomMeshRuntime::init`] derives a `node_id` and constructs a fresh
//!    [`AppState`].
//! 2. Configuration is resolved by probing, in order: an explicit
//!    `config_path`, then `<data_dir>/agents.toml`, then
//!    `~/.phantom-mesh/agents.toml`. The first existing, readable file wins
//!    and is loaded into the shared state; remaining candidates are skipped.
//! 3. The configured `data_dir` (if any) is created on disk so later
//!    components can persist state.
//!
//! The returned runtime is then consumed by higher layers (server, CLI, GUI)
//! which drive their own async event loops against the shared state.

use crate::providers::credential_scanner::home_dir_lenient;
use crate::AppState;
use std::path::PathBuf;

/// Inputs that steer how the runtime locates and loads its configuration.
///
/// All fields are optional; when absent the runtime falls back to the
/// well-known `~/.phantom-mesh/agents.toml` location.
#[derive(Debug, Clone, Default)]
pub struct RuntimeConfig {
    /// Explicit path to an `agents.toml` file. Takes precedence over every
    /// other config source when set and present on disk.
    pub config_path: Option<PathBuf>,
    /// Data directory used both as a secondary config source
    /// (`<data_dir>/agents.toml`) and as the location created on disk for
    /// persisting runtime state.
    pub data_dir: Option<PathBuf>,
}

/// Top-level handle to an initialized phantom-mesh node.
///
/// Holds the per-process node identifier and the shared [`AppState`] that the
/// rest of the crate operates on. Construct one with
/// [`PhantomMeshRuntime::init`].
pub struct PhantomMeshRuntime {
    node_id: String,
    app_state: AppState,
}

impl PhantomMeshRuntime {
    /// Initialize the runtime: derive a node identifier, build a fresh
    /// [`AppState`], load configuration from the first available source, and
    /// ensure the data directory exists.
    ///
    /// # Errors
    ///
    /// Returns an error if runtime construction fails. Config and data-dir
    /// I/O failures are tolerated (the node starts with defaults).
    pub async fn init(config: RuntimeConfig) -> anyhow::Result<Self> {
        let node_id = format!(
            "mac-{:08x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        );

        let mut state = AppState::new();

        // Try loading config from explicit path first, then data_dir, then ~/.phantom-mesh
        let config_paths = [
            config.config_path.clone(),
            config.data_dir.as_ref().map(|d| d.join("agents.toml")),
            home_dir_lenient().map(|h| h.join(".phantom-mesh").join("agents.toml")),
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

        Ok(Self {
            node_id,
            app_state: state,
        })
    }

    /// The per-process node identifier assigned at [`init`](Self::init) time.
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Borrow the shared application state owned by this runtime.
    pub fn app_state(&self) -> &AppState {
        &self.app_state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Restores the saved env var value on drop (panic-safe cleanup).
    struct VarGuard(&'static str, Option<String>);
    impl VarGuard {
        fn save(key: &'static str) -> Self {
            Self(key, std::env::var(key).ok())
        }
    }
    impl Drop for VarGuard {
        fn drop(&mut self) {
            match &self.1 {
                Some(v) => std::env::set_var(self.0, v),
                None => std::env::remove_var(self.0),
            }
        }
    }

    /// Runtime's `agents.toml` home resolution honours `HOME` when it is set,
    /// preserving the Unix `$HOME`-redirect isolation tests rely on.
    #[test]
    fn home_resolution_prefers_home_env() {
        let _g = crate::env_lock::acquire();
        let _saved = VarGuard::save("HOME");
        std::env::set_var("HOME", "/tmp/phantom-runtime-home-test");
        assert_eq!(
            home_dir_lenient(),
            Some(PathBuf::from("/tmp/phantom-runtime-home-test")),
        );
    }

    /// With `HOME` unset (the Windows reality), runtime home resolution must
    /// still yield a directory via the `dirs::home_dir()` fallback rather than
    /// `None` — the defect the bare `std::env::var("HOME")` version exhibited.
    #[test]
    fn home_resolution_falls_back_when_home_unset() {
        let _g = crate::env_lock::acquire();
        let _saved = VarGuard::save("HOME");
        std::env::remove_var("HOME");
        assert_eq!(home_dir_lenient(), dirs::home_dir());
    }
}
