pub mod agent;
pub mod capabilities;
pub mod cli_config;
pub mod clock;
pub mod cold_launch;
pub mod node_manifest;
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
pub mod cli_session;
// `remote_control` is exposed when ANY of the remote-control sub-features (or the
// umbrella) is enabled. Without this, enabling just
// `experimental-remote-control-telegram` or `experimental-remote-control-slack` would
// compile `remote_control/mod.rs` (via the inner sub-feature gates) but leave the
// module path invisible to dependents like `bin/phantom.rs`. The B3/T84
// webhook-secret validator (remote_control::webhook_auth) is reachable via the
// standalone telegram sub-feature; the B5/T86 real Slack adapter
// (remote_control::slack) is reachable via the standalone slack sub-feature.
// The inner `pub mod telegram/whatsapp/slack/persona` declarations each
// carry their own `#[cfg]`, so this widening adds zero extra compile units
// to a default build. Fixed by [B2/T83] (2026-05-16), extended by [B5/T86].
pub mod approval;
pub mod auth;
pub mod auth_gate;
pub mod config;
pub mod context;
pub mod crew;
pub mod contract_gate;
pub mod cost;
pub mod diag;
pub mod diff_render;
pub mod i18n;
pub mod evolve_checkpoint;
pub mod evolve_goals;
pub mod execution_contract;
pub mod extensions;
pub mod external_agent;
pub mod fleet;
pub mod goals_push;
pub mod governed_run;
#[cfg(feature = "experimental-anti-hallucination")]
pub mod hallucination;
pub mod hardware;
pub mod skillbank;
pub mod http_client;
pub mod identity;
pub mod idempotency;
pub mod inbox;
pub mod interrupt;
pub mod session_status;
pub mod keys;
pub mod life_node;
pub mod mcp;
pub mod mcp_client;
pub mod mesh;
pub mod multimodal;
pub mod notifications;
pub mod oauth;
#[cfg(any(
    feature = "experimental-remote-control",
    feature = "experimental-remote-control-telegram",
    feature = "experimental-remote-control-whatsapp",
    feature = "experimental-remote-control-slack",
))]
pub mod remote_control;
pub mod pending_approvals;
pub mod permission;
pub mod permission_profiles;
pub mod process_sandbox;
pub mod project_trust;
pub mod tool_gate;
pub mod partner;
pub mod project_context;
pub mod projects;
pub mod providers;
pub mod recipe;
pub mod redact;
pub mod runtime;
pub mod sandbox;
pub mod scaffold;
pub mod serve;
#[cfg(feature = "experimental-memory")]
pub mod serve_skillbank;
pub mod service;
pub mod session;
pub mod streaming;
pub mod swarm;
pub mod tasks;
pub mod todoist;
pub mod tools;
pub mod tracing;
pub mod tui;
pub mod util;
pub mod vault;
pub mod worker_installer;
pub mod workspace;
// P2-1 §minimal-v1 zero-knowledge cloud relay store (sealed-blob in, fail-closed out).
pub mod zk_cloud;
// SPEC-10 §7 wire types (Stage 2: pseudocode HMAC stubs + ts-rs).
pub mod rpc_wire;
// SPEC-12 §7 identity-keypair wire types (Stage 1: types + ts-rs + stubs).
pub mod identity_wire;
// SPEC-13 §7 encryption (age v1) wire types (Stage 1).
pub mod encryption_wire;
// SPEC-16 §7 event-storage wire types (Stage 1).
pub mod event_storage_wire;
// Semantic memory/recall engine — local-first embedding layer (the moat).
pub mod embeddings;
// SPEC-23 / SPEC-41 #3 — Daily Review reader wire (app surface of /review).
pub mod daily_review_wire;
// SPEC-11 §7 mDNS discovery wire types (Stage 1).
pub mod mdns_wire;
// SPEC-14 §7 LLM providers wire types (Stage 1).
pub mod providers_wire;
// SPEC-17 §7+§9 Tauri bridge wire types (Stage 1).
pub mod tauri_wire;
// SPEC-20 §7 capture-food wire types (Stage 1).
pub mod capture_food_wire;
// SPEC-21 §7 capture-focus wire types (Stage 1).
pub mod capture_focus_wire;
// SPEC-22 §7 capture-habit wire types (Stage 1).
pub mod capture_habit_wire;
// SPEC-23 §7 coach-engine wire types (Stage 1).
pub mod coach_wire;
// SPEC-15 §7 broker-vault-sync wire types (Stage 1).
pub mod broker_vault_wire;
// SPEC-24 §7 coach-delivery wire types (Stage 1).
pub mod coach_delivery_wire;
// SPEC-25 §7 skill-extraction wire types (Stage 1).
pub mod skill_wire;
// SPEC-26 §7 cluster-dispatch wire types (Stage 1).
pub mod cluster_dispatch_wire;
// SPEC-27 §7 smart-task-decompose wire types (Stage 1).
pub mod smart_decompose_wire;
// SPEC-28 §7 onboarding (30s-hello FSM) wire types (Stage 1).
pub mod onboarding_wire;
// First-run agents.toml writer — single source of truth shared by the CLI +
// GUI onboarding surfaces (unified onboarding design §8.1).
pub mod onboarding_config;
// System-state diagnostics — the shared state-machine behind `status`/`doctor`
// (4-layer onboarding model: identity / provider / project-trust / permission).
pub mod diagnostics;
// SPEC-29 §7 release-pipeline wire types (Stage 1).
pub mod release_pipeline_wire;
// SPEC-60 §7 release-evidence ship-gate collector (P2-2): gate-map → resolve →
// run → classify → ShipGateReport, with the load-bearing honesty contract.
pub mod test_report;
// SPEC-61 §7 S1..S40 scenario catalog (P2-2): CSV parse + meta-validators.
pub mod scenarios;

#[cfg(target_os = "macos")]
pub mod snapshot;

pub use agent::{AgentEvent, AgentResult, AgentRuntime};
pub use config::{
    AgentEntry, AgentsConfig, CoreConfig, ProviderEntry, ToolsConfig, WorkspaceConfig,
};
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
    /// apex-④ off-switch: cooperative-abort handles keyed by job_id. The
    /// `/rpc/task/assign` spawn path registers a fresh [`interrupt::InterruptHandle`]
    /// here before launching the runner; `/rpc/task/stop` looks it up and fires
    /// it so the live agent loop unwinds at its next safe point. Empty handle =
    /// the task isn't locally in flight (or finished) — STOP still flips durable
    /// state regardless, so a restart-orphaned row is still controllable.
    pub task_aborts:
        Arc<tokio::sync::RwLock<std::collections::HashMap<uuid::Uuid, interrupt::InterruptHandle>>>,
    pub notifier: Option<NotificationDispatcher>,
    /// F400 — skill bank, exposed by the `/api/skills*`
    /// endpoints in `serve_skillbank`. Field is feature-gated (and `Option`)
    /// so the default cargo build and existing deployments (which never
    /// open a skill DB) carry no observable change.
    #[cfg(feature = "experimental-memory")]
    pub skill_memory: Option<crate::skillbank::memory::SkillMemory>,
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
            task_aborts: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            notifier: None,
            #[cfg(feature = "experimental-memory")]
            skill_memory: None,
        }
    }

    pub fn load_config_toml(&mut self, content: &str) {
        if let Ok(mut config) = toml::from_str::<AgentsConfig>(content) {
            // Resolve ${ENV_VAR} in provider strings before anything
            // (validation, key checks, AgentRuntime build) reads them.
            config.resolve_env_vars();
            let providers: Vec<ProviderHealthSummary> = config
                .providers
                .iter()
                .map(|(name, entry)| ProviderHealthSummary {
                    provider_name: name.clone(),
                    is_available: entry.api_key.is_some() || entry.api_key_env.is_some(),
                    circuit_state: "closed".into(),
                    rotation_status: "active".into(),
                    request_count: 0,
                    avg_latency_ms: 0.0,
                    last_error: None,
                })
                .collect();
            self.llm_router = LLMRouter {
                inner: Arc::new(LLMRouterInner { providers }),
            };

            if let Some(master) = config.agent.get("master") {
                if !master.tools.is_empty() {
                    self.tool_registry = ToolRegistry {
                        tools: Arc::new(master.tools.clone()),
                    };
                }
            }

            self.telegram_config = config.telegram.clone();
            self.cluster_manager = mesh::ClusterManager::new(config.cluster.clone());
            self.agent_runtime = AgentRuntime::new(config);
        }
    }

    pub fn app_state(&self) -> &Self {
        self
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ── ToolRegistry ──────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ToolRegistry {
    tools: Arc<Vec<String>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self {
            tools: Arc::new(vec![
                "shell".into(),
                "file_read".into(),
                "file_write".into(),
                "file_edit".into(),
                "content_search".into(),
                "glob_search".into(),
                "web_search".into(),
                "git_status".into(),
                "git_diff".into(),
                "git_log".into(),
                "git_commit".into(),
            ]),
        }
    }
}

impl ToolRegistry {
    pub fn names(&self) -> Vec<String> {
        self.tools.as_ref().clone()
    }
}

// ── HandRegistry ──────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct HandRegistry {
    hands: Arc<Vec<String>>,
}

impl Default for HandRegistry {
    fn default() -> Self {
        Self {
            hands: Arc::new(vec!["master".into(), "coder".into(), "researcher".into()]),
        }
    }
}

impl HandRegistry {
    pub fn names(&self) -> Vec<String> {
        self.hands.as_ref().clone()
    }
}

// ── LLMRouter ────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct LLMRouter {
    inner: Arc<LLMRouterInner>,
}

impl LLMRouter {
    pub fn inner(&self) -> &LLMRouterInner {
        &self.inner
    }
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

fn default_telegram_agent() -> String {
    "master".into()
}

// ── HTTP Server ───────────────────────────────────────────────────────────

/// Bind a `tokio::net::TcpListener` on `host:port` with SO_REUSEADDR + a 15s
/// retry loop. Returns the bound listener on success, or a `"bind … failed after
/// 15s"` error if the port could not be claimed in time.
///
/// SPLIT from the old monolithic `start_http_server` so a caller can confirm
/// THIS process actually owns the port BEFORE arming side-effects (e.g. the
/// active-app sampler): a bind failure `?`-returns here, before anything that
/// would otherwise observe a stranger's HTTP service on the same port. The
/// non-capture callers go through the unchanged `start_http_server` wrapper.
///
/// SO_REUSEADDR + retry. Two failure modes this handles:
///   1. Previous serve was force-killed (e.g. by cluster upgrade
///      trampoline's taskkill). Its socket lingers in TIME_WAIT for
///      30-120s. Without REUSEADDR a fresh bind on the same port
///      gets EADDRINUSE the whole time.
///   2. Two phantom serves briefly coexist mid-rollover. REUSEADDR
///      lets the new one bind even if the old hasn't fully cleaned
///      up its listening socket yet.
/// The retry loop catches the rarer "literally another process is
/// listening on this port right now" case (e.g. another serve that
/// wasn't killed — wait a few seconds then give up clearly).
pub async fn bind_http_listener(host: &str, port: u16) -> anyhow::Result<tokio::net::TcpListener> {
    let addr: std::net::SocketAddr = format!("{}:{}", host, port).parse()?;
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
                Ok(l) => {
                    listener = Some(l);
                    break;
                }
                Err(e) => {
                    let in_use = e.kind() == std::io::ErrorKind::AddrInUse
                        || e.raw_os_error() == Some(10048); // WSAEADDRINUSE on Windows
                    last_err = Some(e);
                    if !in_use {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        }
        listener.ok_or_else(|| {
            anyhow::anyhow!(
                "bind {} failed after 15s: {}",
                addr,
                last_err.map(|e| e.to_string()).unwrap_or_default()
            )
        })?
    };
    Ok(listener)
}

/// Serve an already-bound listener with the given router until the connection
/// loop ends (or errors). SPLIT from `start_http_server` so the capture serve
/// path can interleave arming the sampler between bind and serve.
///
/// `into_make_service_with_connect_info` exposes the peer socket address to
/// handlers via `axum::extract::ConnectInfo<SocketAddr>` (used by `/api/chat`
/// for the SPEC-46 I3 loopback exemption). Backward-compatible: handlers that
/// do not extract ConnectInfo are unaffected.
pub async fn serve_http(
    listener: tokio::net::TcpListener,
    router: axum::Router,
) -> anyhow::Result<()> {
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}

/// Bind + serve in one call. Thin wrapper over `bind_http_listener` +
/// `serve_http` preserving the ORIGINAL signature and behaviour, so the
/// non-capture callers (`main.rs`, the non-capture serve path in `bin/phantom.rs`)
/// stay unchanged and identical.
pub async fn start_http_server(host: &str, port: u16, router: axum::Router) -> anyhow::Result<()> {
    let listener = bind_http_listener(host, port).await?;
    serve_http(listener, router).await
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appstate_default_tools() {
        let state = AppState::new();
        assert!(state.tool_registry.names().contains(&"shell".to_string()));
        assert!(state
            .tool_registry
            .names()
            .contains(&"file_read".to_string()));
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
        assert!(
            !sha.is_empty(),
            "core_sha() must be non-empty (got '{sha}')"
        );
    }

    #[tokio::test]
    async fn conversation_store_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConversationStore::new_with_dir(dir.path().to_path_buf());
        use providers::traits::ChatMessage;
        let user = ChatMessage {
            role: "user".into(),
            content: "hello".into(),
            tool_calls: None,
        };
        let asst = ChatMessage {
            role: "assistant".into(),
            content: "hi".into(),
            tool_calls: None,
        };
        store.append("test", user, asst).await;
        let history = store.get_history("test").await;
        assert_eq!(history.len(), 2);
    }
}
