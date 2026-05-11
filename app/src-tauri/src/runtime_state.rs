//! Tauri managed state wrapping PhantomMeshRuntime.
//!
//! Replaces the sidecar daemon approach: the full PhantomMesh agent runtime
//! runs in-process, and a lightweight HTTP server provides backward
//! compatibility for commands that still proxy through localhost HTTP.

use std::sync::Arc;

use phantom_mesh::runtime::{PhantomMeshRuntime, RuntimeConfig};

/// Tauri managed state that owns the in-process PhantomMesh runtime.
pub struct RuntimeState {
    pub runtime: Arc<PhantomMeshRuntime>,
    pub port: u16,
}

impl RuntimeState {
    /// Initialize the PhantomMesh runtime and return the managed state.
    ///
    /// `config_path` — optional explicit path to `agents.toml`.
    /// `port` — the HTTP port for backward-compatible API access.
    pub async fn init(config: RuntimeConfig, port: u16) -> anyhow::Result<Self> {
        let runtime = PhantomMeshRuntime::init(config).await?;
        Ok(Self {
            runtime: Arc::new(runtime),
            port,
        })
    }
}
