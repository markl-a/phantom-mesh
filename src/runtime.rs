//! Library-level runtime initializer for PhantomMesh.
//!
//! [`PhantomMeshRuntime`] wraps [`AppState`] initialization so that Tauri
//! (and other library consumers) can start a full agent runtime without
//! running the CLI daemon.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tokio::sync::RwLock as TokioRwLock;

use crate::agent_runtime::AgentRuntime;
use crate::app_state::AppState;
use crate::approval::{ApprovalGate, ApprovalConfig};
use crate::cluster::ClusterRegistry;
use crate::conversation::ConversationStore;
use crate::estop::EStop;
use crate::evaluate::EvalConfig;
use crate::hands::HandRegistry;
use crate::llm_router::LlmRouter;
use crate::metrics::MetricsRegistry;
use crate::providers::ProviderRouter;
use crate::security::{SecretManager, NodeIdentity};
use crate::skills::SkillRegistry;
use crate::task_queue::TaskQueue;
use crate::telegram_i18n::TelegramI18n;
use crate::tools::{ToolRegistry, SecurityConfig};
use crate::user_profile::UserProfile;

// ── Configuration ──────────────────────────────────────────────────────────────

/// Configuration for [`PhantomMeshRuntime`].
///
/// All fields are optional — sensible defaults are derived from the
/// platform's home directory (`~/.phantom-mesh/`).
#[derive(Debug, Clone, Default)]
pub struct RuntimeConfig {
    /// Path to `agents.toml`.  Defaults to `<data_dir>/agents.toml`.
    pub config_path: Option<PathBuf>,
    /// Path to the main SQLite database.  Defaults to `<data_dir>/core.db`.
    pub db_path: Option<PathBuf>,
    /// Root data directory.  Defaults to `~/.phantom-mesh`.
    pub data_dir: Option<PathBuf>,
}

impl RuntimeConfig {
    /// Resolve `data_dir` — uses the explicit value, or `~/.phantom-mesh`.
    fn resolve_data_dir(&self) -> PathBuf {
        if let Some(ref d) = self.data_dir {
            d.clone()
        } else {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".phantom-mesh")
        }
    }

    fn resolve_config_path(&self) -> PathBuf {
        self.config_path
            .clone()
            .unwrap_or_else(|| self.resolve_data_dir().join("agents.toml"))
    }

    fn resolve_db_path(&self) -> PathBuf {
        self.db_path
            .clone()
            .unwrap_or_else(|| self.resolve_data_dir().join("core.db"))
    }
}

// ── Runtime ────────────────────────────────────────────────────────────────────

/// Top-level runtime that owns a fully-initialized [`AppState`].
///
/// Library consumers (e.g. Tauri) create one of these at startup and
/// hand its `AppState` to whatever HTTP / IPC layer they use.
pub struct PhantomMeshRuntime {
    state: AppState,
    identity: NodeIdentity,
}

impl PhantomMeshRuntime {
    /// Initialize all services and return a ready-to-use runtime.
    ///
    /// ```rust,no_run
    /// # #[tokio::main] async fn main() -> anyhow::Result<()> {
    /// use phantom_mesh::runtime::{PhantomMeshRuntime, RuntimeConfig};
    /// let rt = PhantomMeshRuntime::init(RuntimeConfig::default()).await?;
    /// let state = rt.app_state();
    /// # Ok(())
    /// # }
    /// ```
    pub async fn init(config: RuntimeConfig) -> Result<Self> {
        let data_dir = config.resolve_data_dir();
        std::fs::create_dir_all(&data_dir)?;

        // --- Node Identity (Ed25519 keypair) ---
        let identity = NodeIdentity::load_or_generate(&data_dir)?;
        tracing::info!("Node ID: {}", identity.node_id);

        let config_path = config.resolve_config_path();
        let db_path = config.resolve_db_path();

        // --- LLM Router ---
        let llm_router = if config_path.exists() {
            Arc::new(LlmRouter::new(config_path.to_str().unwrap_or("agents.toml"))?)
        } else {
            // No config file — start with an empty router (providers can be
            // registered later through the mutable inner handle).
            Arc::new(LlmRouter::from_router(ProviderRouter::empty()))
        };

        // --- Agent Runtime ---
        let agent_runtime = Arc::new(
            AgentRuntime::new(config_path.to_str().unwrap_or("agents.toml"))?,
        );

        // --- Tool Registry ---
        let workspace_dir = data_dir.join("workspace");
        std::fs::create_dir_all(&workspace_dir)?;

        let security = SecurityConfig {
            workspace_dir: workspace_dir.to_string_lossy().to_string(),
            ..Default::default()
        };
        let tool_registry = Arc::new(ToolRegistry::new(security));

        // --- Async data stores ---
        let db_str = db_path.to_str().unwrap_or("core.db");
        let task_queue = Arc::new(TaskQueue::new(db_str).await?);
        let cluster = Arc::new(ClusterRegistry::new(
            data_dir.join("cluster.db").to_str().unwrap_or(":memory:"),
        ).await?);
        let conversations = Arc::new(ConversationStore::new(
            data_dir.join("conversations.db").to_str().unwrap_or("conversations.db"),
        ).await?);

        // --- Simple sync components ---
        let skill_registry = Arc::new(SkillRegistry::new());
        let hands = Arc::new(HandRegistry::default());
        let metrics_registry = Arc::new(MetricsRegistry::new());
        let estop = Arc::new(EStop::new());
        let approval_gate = Arc::new(ApprovalGate::new(ApprovalConfig::default()));
        let telegram_i18n = Arc::new(TokioRwLock::new(TelegramI18n::new()));
        let user_profile = Arc::new(std::sync::RwLock::new(UserProfile::default()));
        let dashboard_token = uuid::Uuid::new_v4().to_string();

        let state = AppState {
            llm_router,
            task_queue,
            agent_runtime,
            cluster,
            tool_registry,
            conversations,
            memory_store: None,
            skill_registry,
            eval_config: EvalConfig::default(),
            estop,
            hands,
            approval_gate,
            scheduler: None,
            cost_tracker: None,
            revenue_tracker: None,
            cluster_hub: None,
            hub_api_key: None,
            dashboard_token,
            public_url: None,
            metrics_registry,
            audit_logger: None,
            load_tester: None,
            worker_onboarder: None,
            service_tier: None,
            optimizer_store: None,
            auto_diagnoser: None,
            tenant_manager: None,
            order_workflow: None,
            customer_health: None,
            churn_detector: None,
            observational_memory: None,
            preemption_manager: None,
            node_scorer: None,
            power_economics: None,
            provider_pricing: None,
            financial_monitor: None,
            unit_economics: None,
            telegram_i18n,
            cluster_secret: None,
            started_at: Instant::now(),
            roi_gate: None,
            governor: None,
            pipeline_orchestrator: None,
            feedback_loop_config: None,
            roi_scheduler: None,
            route_manager: None,
            goals_store: None,
            user_profile,
            trigger_manager: None,
            networking_tasks: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        };

        Ok(Self { state, identity })
    }

    /// Access this node's Ed25519 identity.
    pub fn identity(&self) -> &NodeIdentity {
        &self.identity
    }

    /// Get the short node ID (first 16 hex chars of public key).
    pub fn node_id(&self) -> &str {
        &self.identity.node_id
    }

    /// Borrow the fully-initialized [`AppState`].
    pub fn app_state(&self) -> &AppState {
        &self.state
    }

    /// Quick access to the agent runtime.
    pub fn agent_runtime(&self) -> &Arc<AgentRuntime> {
        &self.state.agent_runtime
    }
}

// ── HTTP Server ─────────────────────────────────────────────────────────────────

/// Start the HTTP API server (for CLI daemon mode).
///
/// Tauri doesn't need this — it calls Rust functions directly via
/// [`PhantomMeshRuntime`].  This is a standalone helper that wraps the
/// axum serve boilerplate with graceful Ctrl-C shutdown.
pub async fn start_http_server(
    host: &str,
    port: u16,
    router: axum::Router,
) -> anyhow::Result<()> {
    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening on http://{}", addr);
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("Shutting down...");
        })
        .await?;
    Ok(())
}

// ── Config secret decryption helper ────────────────────────────────────────────

/// Replace `enc2:…` encrypted tokens in a TOML config string with their
/// plaintext values.  Used to decrypt `agents.toml` before parsing.
///
/// Each `enc2:<hex>` token is terminated by the first `"`, `'`, or newline
/// character (or end-of-string).  If decryption fails for any token the
/// function stops replacing and returns what it has so far.
pub fn decrypt_config_secrets(sm: &SecretManager, content: &str) -> String {
    let mut result = content.to_string();
    while let Some(start) = result.find("enc2:") {
        let rest = &result[start..];
        let end = rest
            .find('"')
            .or_else(|| rest.find('\''))
            .or_else(|| rest.find('\n'))
            .unwrap_or(rest.len());
        let enc_value = result[start..start + end].to_string();
        match sm.decrypt(&enc_value) {
            Ok(plain) => {
                result = format!(
                    "{}{}{}",
                    &result[..start],
                    plain,
                    &result[start + end..],
                );
            }
            Err(_) => break,
        }
    }
    result
}

// ── Unit tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_config_defaults_resolve() {
        let cfg = RuntimeConfig::default();
        let data = cfg.resolve_data_dir();
        assert!(data.ends_with(".phantom-mesh"));
        assert!(cfg.resolve_config_path().ends_with("agents.toml"));
        assert!(cfg.resolve_db_path().ends_with("core.db"));
    }

    #[test]
    fn runtime_config_explicit_overrides() {
        let cfg = RuntimeConfig {
            config_path: Some(PathBuf::from("/tmp/my/agents.toml")),
            db_path: Some(PathBuf::from("/tmp/my/core.db")),
            data_dir: Some(PathBuf::from("/tmp/my")),
        };
        assert_eq!(cfg.resolve_config_path(), PathBuf::from("/tmp/my/agents.toml"));
        assert_eq!(cfg.resolve_db_path(), PathBuf::from("/tmp/my/core.db"));
        assert_eq!(cfg.resolve_data_dir(), PathBuf::from("/tmp/my"));
    }

    #[test]
    fn decrypt_noop_when_no_enc_tokens() {
        // Without a SecretManager instance we cannot test real decryption,
        // but we can verify that content without `enc2:` passes through
        // unchanged.
        let content = "api_key = \"sk-plain-key\"\nmodel = \"gpt-4\"";
        // Create a SecretManager in a temp dir
        let tmp = tempfile::TempDir::new().unwrap();
        let sm = SecretManager::new(tmp.path().to_str().unwrap()).unwrap();
        let out = decrypt_config_secrets(&sm, content);
        assert_eq!(out, content);
    }

    #[test]
    fn decrypt_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sm = SecretManager::new(tmp.path().to_str().unwrap()).unwrap();
        let secret = "my-super-secret-key";
        let encrypted = sm.encrypt(secret).unwrap();
        let config = format!("api_key = \"{}\"\n", encrypted);
        let decrypted = decrypt_config_secrets(&sm, &config);
        assert_eq!(decrypted, format!("api_key = \"{}\"\n", secret));
    }

    #[tokio::test]
    async fn runtime_init_with_temp_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cfg = RuntimeConfig {
            data_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let rt = PhantomMeshRuntime::init(cfg).await.unwrap();
        // Verify we got a valid state
        assert!(!rt.app_state().dashboard_token.is_empty());
        assert!(rt.app_state().started_at.elapsed().as_secs() < 5);
    }
}
