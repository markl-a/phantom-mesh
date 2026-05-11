pub mod agent;
pub mod cli_config;
pub mod models_cache;
pub mod platform;

/// Shared mutex for unit tests that mutate process-global env vars
/// (HOME / USERPROFILE / etc.). `std::env::set_var` is process-global on
/// all platforms — without serialization, parallel tests racing on these
/// vars are flaky. Any test that calls `set_var("HOME", …)` should hold
/// `env_lock::acquire()` for the duration of its work.
#[cfg(test)]
pub mod env_lock {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn cell() -> &'static Mutex<()> {
        static M: OnceLock<Mutex<()>> = OnceLock::new();
        M.get_or_init(|| Mutex::new(()))
    }

    /// Block until no other test holds the lock. Recovers from poisoning
    /// because a panicking test shouldn't permanently break the suite.
    pub fn acquire() -> MutexGuard<'static, ()> {
        cell().lock().unwrap_or_else(|p| p.into_inner())
    }
}
pub mod channels;
pub mod runtime;
pub mod config;
pub mod context;
pub mod cost;
pub mod diff_render;
pub mod goals_push;
pub mod diag;
pub mod evolve_checkpoint;
pub mod evolve_goals;
pub mod extensions;
pub mod hardware;
pub mod approval;
pub mod identity;
pub mod interrupt;
pub mod permission;
pub mod projects;
pub mod keys;
pub mod mcp;
pub mod mcp_client;
pub mod mesh;
pub mod multimodal;
pub mod notifications;
pub mod oauth;
pub mod project_context;
pub mod providers;
pub mod recipe;
pub mod sandbox;
pub mod scaffold;
pub mod serve;
pub mod session;
pub mod streaming;
pub mod tasks;
pub mod tools;
pub mod workspace;
pub mod http_client;
pub mod tui;
pub mod auth;

#[cfg(target_os = "macos")]
pub mod snapshot;

pub use agent::{AgentEvent, AgentResult, AgentRuntime};
pub use config::{AgentsConfig, AgentEntry, CoreConfig, ProviderEntry, ToolsConfig, WorkspaceConfig};
pub use cost::CostTracker;
pub use notifications::NotificationDispatcher;
pub use session::ConversationStore;
pub use tasks::{TaskQueue, TaskRecord, TaskStatus, TaskStore};
pub use workspace::{Workspace, WorkspaceId, WorkspaceRegistry, WorkspaceResolver};

use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use std::time::Instant;

// ── Wire-protocol version ─────────────────────────────────────────────────
//
// Bumped whenever a peer-facing RPC schema changes in a backward-
// incompatible way (field removed, type changed, semantics shifted). Add-
// only changes do NOT bump. Every peer-facing RPC response carries this
// value at the top level so a mismatched peer can refuse with a clear
// error rather than silently `serde::de::Error` on a missing field.
//
// See docs/MULTI-DEVICE-COORDINATION.md Rule 5 for the bump policy.
pub const WIRE_VERSION: u32 = 1;

/// Short git SHA the binary was built from. Falls back to "unknown" if
/// the build script didn't set PHANTOM_GIT_HASH (e.g. cargo install from
/// crates.io). Used in /rpc/ping so peers can detect they're running
/// different builds even on the same semver.
pub const fn core_sha() -> &'static str {
    match option_env!("PHANTOM_GIT_HASH") {
        Some(h) => h,
        None => "unknown",
    }
}

// ── AppState ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub started_at: Instant,
    pub conversations: ConversationStore,
    pub cost_tracker: CostTracker,
    pub agent_runtime: AgentRuntime,
    pub llm_router: LLMRouter,
    pub tool_registry: ToolRegistry,
    pub hands: HandRegistry,
    pub user_profile: Arc<RwLock<UserProfile>>,
    pub goals_store: Option<GoalsStore>,
    pub cluster_manager: mesh::ClusterManager,
    pub telegram_config: Option<TelegramConfig>,
    pub workspace_resolver: Option<WorkspaceResolver>,
    pub task_queue: Option<TaskQueue>,
    pub notifier: Option<NotificationDispatcher>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            conversations: ConversationStore::default(),
            cost_tracker: CostTracker::new(),
            agent_runtime: AgentRuntime::default(),
            llm_router: LLMRouter::default(),
            tool_registry: ToolRegistry::default(),
            hands: HandRegistry::default(),
            user_profile: Arc::new(RwLock::new(UserProfile::default())),
            goals_store: Some(GoalsStore::default()),
            cluster_manager: mesh::ClusterManager::default(),
            telegram_config: None,
            workspace_resolver: None,
            task_queue: None,
            notifier: None,
        }
    }

    pub fn load_config_toml(&mut self, content: &str) {
        if let Ok(mut config) = toml::from_str::<AgentsConfig>(content) {
            // Resolve ${ENV_VAR} in provider strings before anything
            // (validation, key checks, AgentRuntime build) reads them.
            config.resolve_env_vars();
            let providers: Vec<ProviderHealthSummary> = config.providers.iter().map(|(name, entry)| {
                ProviderHealthSummary {
                    provider_name: name.clone(),
                    is_available: entry.api_key.is_some() || entry.api_key_env.is_some(),
                    circuit_state: "closed".into(),
                    rotation_status: "active".into(),
                    request_count: 0,
                    avg_latency_ms: 0.0,
                    last_error: None,
                }
            }).collect();
            self.llm_router = LLMRouter { inner: Arc::new(LLMRouterInner { providers }) };

            if let Some(master) = config.agent.get("master") {
                if !master.tools.is_empty() {
                    self.tool_registry = ToolRegistry { tools: Arc::new(master.tools.clone()) };
                }
            }

            self.telegram_config = config.telegram.clone();
            self.cluster_manager = mesh::ClusterManager::new(config.cluster.clone());
            self.agent_runtime = AgentRuntime::new(config);
        }
    }

    pub fn app_state(&self) -> &Self { self }
}

impl Default for AppState {
    fn default() -> Self { Self::new() }
}

// ── ToolRegistry ──────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ToolRegistry {
    tools: Arc<Vec<String>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self { tools: Arc::new(vec![
            "shell".into(), "file_read".into(), "file_write".into(),
            "file_edit".into(), "content_search".into(), "glob_search".into(),
            "web_search".into(), "git_status".into(), "git_diff".into(),
            "git_log".into(), "git_commit".into(),
        ]) }
    }
}

impl ToolRegistry {
    pub fn names(&self) -> Vec<String> { self.tools.as_ref().clone() }
}

// ── HandRegistry ──────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct HandRegistry {
    hands: Arc<Vec<String>>,
}

impl Default for HandRegistry {
    fn default() -> Self {
        Self { hands: Arc::new(vec!["master".into(), "coder".into(), "researcher".into()]) }
    }
}

impl HandRegistry {
    pub fn names(&self) -> Vec<String> { self.hands.as_ref().clone() }
}

// ── LLMRouter ────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct LLMRouter {
    inner: Arc<LLMRouterInner>,
}

impl LLMRouter {
    pub fn inner(&self) -> &LLMRouterInner { &self.inner }
}

#[derive(Default)]
pub struct LLMRouterInner {
    providers: Vec<ProviderHealthSummary>,
}

impl LLMRouterInner {
    pub fn health_summary(&self) -> Vec<ProviderHealthSummary> {
        if self.providers.is_empty() {
            vec![ProviderHealthSummary {
                provider_name: "none".into(),
                is_available: false,
                circuit_state: "open".into(),
                rotation_status: "standby".into(),
                request_count: 0,
                avg_latency_ms: 0.0,
                last_error: Some("no providers configured".into()),
            }]
        } else {
            self.providers.clone()
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderHealthSummary {
    pub provider_name: String,
    pub is_available: bool,
    pub circuit_state: String,
    pub rotation_status: String,
    pub request_count: u64,
    pub avg_latency_ms: f64,
    pub last_error: Option<String>,
}

// ── UserProfile ───────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct UserProfile {
    pub name: Option<String>,
}

impl UserProfile {
    pub fn system_prompt_context(&self) -> String {
        match &self.name {
            Some(name) => format!("User: {}", name),
            None => String::new(),
        }
    }
}

// ── GoalsStore ────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct GoalsStore {
    pub inner: Arc<std::sync::Mutex<Vec<String>>>,
}

// ── TelegramConfig ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramConfig {
    pub bot_token_env: String,
    #[serde(default)]
    pub allowed_users: Vec<i64>,
    #[serde(default = "default_telegram_agent")]
    pub agent: String,
    /// Chat id that outbound notifications are sent to. Defaults to the first
    /// entry in `allowed_users` if unset.
    #[serde(default)]
    pub notification_chat_id: Option<i64>,
}

fn default_telegram_agent() -> String { "master".into() }

// ── HTTP Server ───────────────────────────────────────────────────────────

pub async fn start_http_server(
    host: &str,
    port: u16,
    router: axum::Router,
) -> anyhow::Result<()> {
    let addr: std::net::SocketAddr = format!("{}:{}", host, port).parse()?;
    // SO_REUSEADDR + retry. Two failure modes this handles:
    //   1. Previous serve was force-killed (e.g. by cluster upgrade
    //      trampoline's taskkill). Its socket lingers in TIME_WAIT for
    //      30-120s. Without REUSEADDR a fresh bind on the same port
    //      gets EADDRINUSE the whole time.
    //   2. Two phantom serves briefly coexist mid-rollover. REUSEADDR
    //      lets the new one bind even if the old hasn't fully cleaned
    //      up its listening socket yet.
    // The retry loop catches the rarer "literally another process is
    // listening on this port right now" case (e.g. another serve that
    // wasn't killed — wait a few seconds then give up clearly).
    let mut last_err: Option<std::io::Error> = None;
    let listener = {
        let mut listener: Option<tokio::net::TcpListener> = None;
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(15) {
            let socket = match addr {
                std::net::SocketAddr::V4(_) => tokio::net::TcpSocket::new_v4(),
                std::net::SocketAddr::V6(_) => tokio::net::TcpSocket::new_v6(),
            }?;
            // set_reuseaddr does the right thing on both unix
            // (SO_REUSEADDR) and Windows (SO_REUSEADDR — note
            // Windows semantics differ subtly from Linux but the
            // "rebind a TIME_WAIT port" use case works on both).
            socket.set_reuseaddr(true)?;
            match socket.bind(addr).and_then(|()| socket.listen(1024)) {
                Ok(l) => { listener = Some(l); break; }
                Err(e) => {
                    let in_use = e.kind() == std::io::ErrorKind::AddrInUse
                        || e.raw_os_error() == Some(10048); // WSAEADDRINUSE on Windows
                    last_err = Some(e);
                    if !in_use { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        }
        listener.ok_or_else(|| anyhow::anyhow!(
            "bind {} failed after 15s: {}",
            addr,
            last_err.map(|e| e.to_string()).unwrap_or_default()
        ))?
    };
    axum::serve(listener, router).await?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appstate_default_tools() {
        let state = AppState::new();
        assert!(state.tool_registry.names().contains(&"shell".to_string()));
        assert!(state.tool_registry.names().contains(&"file_read".to_string()));
        // Verify that the version string is not empty
        assert!(!env!("CARGO_PKG_VERSION").is_empty());
    }

    #[test]
    fn wire_version_is_set_and_core_sha_resolves() {
        // Sanity: the constant must be > 0 (0 is reserved for "unset" /
        // pre-versioning wire) and core_sha is non-empty even on builds
        // without git metadata.
        assert!(WIRE_VERSION >= 1, "WIRE_VERSION should start at 1");
        let sha = core_sha();
        assert!(!sha.is_empty(), "core_sha() must be non-empty (got '{sha}')");
    }

    #[tokio::test]
    async fn conversation_store_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConversationStore::new_with_dir(dir.path().to_path_buf());
        use providers::traits::ChatMessage;
        let user = ChatMessage { role: "user".into(), content: "hello".into(), tool_calls: None };
        let asst = ChatMessage { role: "assistant".into(), content: "hi".into(), tool_calls: None };
        store.append("test", user, asst).await;
        let history = store.get_history("test").await;
        assert_eq!(history.len(), 2);
    }
}
