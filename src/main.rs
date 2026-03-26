use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, Json},
    routing::{get, post},
    Router,
};
use clap::{Parser, Subcommand};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

use clawtex_core::{
    AiCodeConfig, AgentRuntime, ApprovalGate, Channel, ChannelMessage, ChatMessage, ClusterRegistry,
    ClusterHub, ClusterWorker, ClusterConfig, WorkerConfig, TaskResultPayload,
    ComputerUseConfig, ConversationStore, CostTracker, CostSummary, CronStore,
    EmailConfig, ImapConfig, EStop, EvalConfig, GatewayState, HandRegistry, HandRunner, JobAction, LlmRouter,
    MemoryCategory, MemoryConfig, MemoryStore, PhaseOutput, PrivacyConfig, PrivacyGuard,
    ProviderCircuitBreaker, BreakerConfig,
    RevenueTracker, RevenueSummary,
    RecoveryConfig, WorkerWatchdog,
    Schedule, Scheduler, SearchConfig,
    SecretManager, SecurityConfig, SkillRegistry, TaskQueue, TelegramChannel, TelegramConfig,
    ToolRegistry, TrajectoryLogger, TwitterConfig, BlogConfig, TrustLevel,
    SlackConfig, DiscordConfig, LineConfig, WhatsAppConfig,
    AuditLogger, AuditFilter,
    ConsistencyTester,
    WorkerOnboarder, OnboardConfig,
    LoadTester, StressTestConfig,
    ServiceTierManager, ServiceTier,
    AutoDiagnoser,
    TenantManager,
    OrderWorkflow,
    CustomerHealthManager, ChurnDetector,
    PreemptionManager, NodeScorer, NodeMetrics,
    ObservationalMemory,
    OpsReporter,
    FinancialMonitor, FinancialSnapshot,
    UnitEconomics,
    OptimizerStore, PolicyType,
    PowerEconomics, NodePowerProfile,
    ProviderPricingStore, ProviderPriceRule,
    DeployManifest,
    StripeWebhook, WebhookAction,
};
use clawtex_core::telegram_i18n::{TelegramI18n, LangCommand, parse_lang_command, detect_locale, supported_locales};
use clawtex_core::user_profile::UserProfile;
use clawtex_core::plugin_bus::PluginBus;
use clawtex_core::health_check::HealthCheckPlugin;
use clawtex_core::trajectory::TrajectoryPlugin;
use clawtex_core::circuit_breaker::CircuitBreakerPlugin;

// ── CLI Args ───────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "clawtex-core",
    version,
    about = "Clawtex LLM Cluster Core — lightweight daemon for LLM routing, task queue, and agent coordination"
)]
struct Args {
    /// Host to bind to (0.0.0.0 for all interfaces, needed for cluster workers)
    #[arg(long, default_value = "0.0.0.0")]
    host: String,

    /// Port to listen on
    #[arg(long, default_value_t = 7878)]
    port: u16,

    /// Config file path (default: ~/.clawtex/agents.toml)
    #[arg(long)]
    config: Option<String>,

    /// SQLite database path (default: ~/.clawtex/core.db)
    #[arg(long)]
    db: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Start the daemon (default if no subcommand given)
    Daemon {
        /// Vault password (insecure: visible in process listing, use --vault-password-stdin)
        #[arg(long)]
        vault_password: Option<String>,

        /// Read vault password from stdin instead of CLI argument
        #[arg(long)]
        vault_password_stdin: bool,
    },
    /// Run a single prompt and exit
    Run {
        /// The prompt to execute
        prompt: String,
        /// Agent to use
        #[arg(long, default_value = "master")]
        agent: String,
    },
    /// Start interactive REPL mode
    Interactive {
        /// Agent to use
        #[arg(long, default_value = "master")]
        agent: String,
    },
    /// Display current configuration
    Config,
    /// Show status of providers, tools, and MCP servers
    Status,
    /// Encrypt a secret value for use in config
    EncryptSecret {
        /// The value to encrypt
        value: String,
    },
    /// Start in worker mode — register with hub and accept tool dispatch
    Worker {
        /// Hub URL (e.g., http://100.x.x.x:7878)
        #[arg(long)]
        hub: String,
        /// Worker name (auto-detected from hostname if not set)
        #[arg(long)]
        name: Option<String>,
        /// Port to listen on for incoming tool dispatch
        #[arg(long, default_value_t = 7879)]
        port: u16,
        /// Device type: "full" or "light"
        #[arg(long, default_value = "full")]
        device_type: String,
    },
}

// ── Config ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
struct CoreConfig {
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    hub_api_key: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
struct AppConfig {
    #[serde(default)]
    core: Option<CoreConfig>,
    #[serde(default)]
    telegram: Option<TelegramConfig>,
    #[serde(default)]
    security: Option<SecurityConfig>,
    #[serde(default)]
    search: Option<SearchConfig>,
    #[serde(default)]
    ai_code: Option<AiCodeConfig>,
    #[serde(default)]
    computer_use: Option<ComputerUseConfig>,
    #[serde(default)]
    memory: Option<MemoryConfig>,
    #[serde(default)]
    eval: Option<EvalConfig>,
    #[serde(default)]
    email: Option<EmailConfig>,
    #[serde(default)]
    imap: Option<ImapConfig>,
    #[serde(default)]
    twitter: Option<TwitterConfig>,
    #[serde(default)]
    blog: Option<BlogConfig>,
    #[serde(default)]
    stripe: Option<StripeConfig>,
    #[serde(default)]
    render: Option<RenderConfig>,
    #[serde(default)]
    cluster: Option<ClusterConfig>,
    #[serde(default)]
    privacy: Option<PrivacyConfig>,
    #[serde(default)]
    slack: Option<SlackConfig>,
    #[serde(default)]
    discord: Option<DiscordConfig>,
    #[serde(default)]
    line: Option<LineConfig>,
    #[serde(default)]
    whatsapp: Option<WhatsAppConfig>,
    #[serde(default)]
    image_generate: Option<ImageGenerateAppConfig>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct ImageGenerateAppConfig {
    #[serde(default)]
    gemini_api_key: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct StripeConfig {
    #[serde(default)]
    secret_key: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct RenderConfig {
    #[serde(default)]
    api_key: String,
}

fn default_load_factor() -> f64 {
    1.0
}

#[derive(Debug, Deserialize)]
struct PowerEstimateRequest {
    node_id: String,
    duration_secs: f64,
    #[serde(default = "default_load_factor")]
    load_factor: f64,
}

#[derive(Debug, Deserialize)]
struct PowerProfitabilityRequest {
    node_id: String,
    expected_revenue_per_hour_usd: f64,
    #[serde(default)]
    api_cost_per_hour_usd: f64,
    #[serde(default = "default_load_factor")]
    load_factor: f64,
}

#[derive(Debug, Deserialize)]
struct PowerProfileUpsertRequest {
    idle_watts: f64,
    active_watts: f64,
    electricity_usd_per_kwh: f64,
    #[serde(default)]
    depreciation_usd_per_hour: f64,
    #[serde(default)]
    cooling_usd_per_hour: f64,
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PricingRuleUpsertRequest {
    provider: String,
    model_pattern: String,
    input_usd_per_1m_tokens: f64,
    output_usd_per_1m_tokens: f64,
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PricingEstimateRequest {
    provider: String,
    model: String,
    tokens_in: u32,
    tokens_out: u32,
}

// ── App State ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    llm_router: Arc<LlmRouter>,
    task_queue: Arc<TaskQueue>,
    agent_runtime: Arc<AgentRuntime>,
    cluster: Arc<ClusterRegistry>,
    tool_registry: Arc<ToolRegistry>,
    conversations: Arc<ConversationStore>,
    memory_store: Option<Arc<MemoryStore>>,
    skill_registry: Arc<SkillRegistry>,
    eval_config: EvalConfig,
    estop: Arc<EStop>,
    hands: Arc<HandRegistry>,
    approval_gate: Arc<ApprovalGate>,
    scheduler: Option<Arc<Scheduler>>,
    cost_tracker: Option<Arc<CostTracker>>,
    revenue_tracker: Option<Arc<RevenueTracker>>,
    cluster_hub: Option<Arc<ClusterHub>>,
    hub_api_key: Option<String>,
    dashboard_token: String,
    public_url: Option<String>,
    metrics_registry: Arc<clawtex_core::MetricsRegistry>,
    audit_logger: Option<Arc<AuditLogger>>,
    load_tester: Option<Arc<LoadTester>>,
    worker_onboarder: Option<Arc<WorkerOnboarder>>,
    service_tier: Option<Arc<ServiceTierManager>>,
    optimizer_store: Option<Arc<OptimizerStore>>,
    auto_diagnoser: Option<Arc<AutoDiagnoser>>,
    tenant_manager: Option<Arc<TenantManager>>,
    order_workflow: Option<Arc<OrderWorkflow>>,
    customer_health: Option<Arc<CustomerHealthManager>>,
    churn_detector: Option<Arc<ChurnDetector>>,
    observational_memory: Option<Arc<ObservationalMemory>>,
    preemption_manager: Option<Arc<PreemptionManager>>,
    node_scorer: Option<Arc<NodeScorer>>,
    power_economics: Option<Arc<PowerEconomics>>,
    provider_pricing: Option<Arc<ProviderPricingStore>>,
    financial_monitor: Option<Arc<FinancialMonitor>>,
    unit_economics: Option<Arc<UnitEconomics>>,
    telegram_i18n: Arc<tokio::sync::RwLock<TelegramI18n>>,
    /// Shared secret for inter-node cluster authentication.
    /// When set, cluster endpoints (register, heartbeat, poll, result) require
    /// `Authorization: Bearer <secret>`. When `None`, auth is disabled (open cluster).
    cluster_secret: Option<String>,
    started_at: Instant,
    // Efficiency engine subsystems
    roi_gate: Option<Arc<clawtex_core::roi_gate::RoiGate>>,
    governor: Option<Arc<clawtex_core::governor::Governor>>,
    pipeline_orchestrator: Option<Arc<tokio::sync::RwLock<clawtex_core::pipeline::PipelineOrchestrator>>>,
    feedback_loop_config: Option<clawtex_core::feedback_loop::FeedbackLoopConfig>,
    roi_scheduler: Option<Arc<clawtex_core::roi_scheduler::RoiScheduler>>,
    route_manager: Option<Arc<clawtex_core::networking::RouteManager>>,
    goals_store: Option<Arc<clawtex_core::goals::GoalsStore>>,
    user_profile: Arc<RwLock<UserProfile>>,
    /// Event trigger manager — shared with cron tick loop and Telegram /alerts handler.
    trigger_manager: Option<Arc<std::sync::Mutex<clawtex_core::event_triggers::EventTriggerManager>>>,
    /// Background networking task handles (for shutdown cleanup).
    #[allow(dead_code)]
    networking_tasks: Arc<tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

// ── Auth Middleware ────────────────────────────────────────────────────────────

async fn auth_middleware(
    State(state): State<AppState>,
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    // /health, /dashboard, /dashboard/v2, /api/dashboard/*, /api/stripe/webhook are always public
    let path = req.uri().path();
    if path == "/health"
        || path == "/dashboard"
        || path == "/dashboard/v2"
        || path.starts_with("/api/dashboard/")
        || path == "/api/stripe/webhook"
        || path.starts_with("/cluster/")
    {
        return Ok(next.run(req).await);
    }
    // No key configured = auth disabled
    let key = match &state.hub_api_key {
        Some(k) => k,
        None => return Ok(next.run(req).await),
    };
    // Check Authorization: Bearer <key>
    let auth = req.headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match auth {
        Some(token) if token == key => Ok(next.run(req).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

// ── Handlers ───────────────────────────────────────────────────────────────────

async fn health(State(_state): State<AppState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "service": "clawtex-core"
    }))
}

async fn route_llm(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    if state.estop.is_stopped() {
        return Ok(Json(json!({"error": "E-Stop active"})));
    }

    let prompt = body
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let provider = body
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("auto");

    match state.llm_router.route(prompt, provider).await {
        Ok(response) => Ok(Json(json!({ "response": response, "provider": provider }))),
        Err(e) => {
            tracing::error!("LLM routing failed: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn task_add(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let title = body
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let prompt = body
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or(title);

    match state.task_queue.add(title, prompt).await {
        Ok(task_id) => Ok(Json(json!({ "task_id": task_id, "status": "pending" }))),
        Err(e) => {
            tracing::error!("Task add failed: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn task_run(
    State(state): State<AppState>,
    axum::extract::Path(task_id): axum::extract::Path<String>,
) -> Result<Json<Value>, StatusCode> {
    if state.estop.is_stopped() {
        return Ok(Json(json!({"error": "E-Stop active"})));
    }
    match state.task_queue.run(&task_id, &state.llm_router).await {
        Ok(result) => Ok(Json(json!({ "task_id": task_id, "result": result, "status": "done" }))),
        Err(e) => {
            tracing::error!("Task run failed: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn task_history(State(state): State<AppState>) -> Result<Json<Value>, StatusCode> {
    match state.task_queue.history(20).await {
        Ok(tasks) => Ok(Json(json!({ "tasks": tasks }))),
        Err(e) => {
            tracing::error!("Task history failed: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn agent_run(
    State(state): State<AppState>,
    axum::extract::Path(agent_name): axum::extract::Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    // E-Stop guard
    if state.estop.is_stopped() {
        return Ok(Json(json!({"error": "E-Stop active — all agent operations halted"})));
    }

    let prompt = body
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or(StatusCode::BAD_REQUEST)?;

    // Inject user profile context
    let profile_ctx = {
        let profile = state.user_profile.read().unwrap_or_else(|p| p.into_inner());
        profile.system_prompt_context()
    };

    // Inject goals context for the master agent
    let goals_ctx = if agent_name == "master" {
        state.goals_store.as_ref()
            .and_then(|gs| clawtex_core::goals_push::goals_context(gs).ok())
            .filter(|s| !s.is_empty())
    } else {
        None
    };

    // Combine profile + goals context
    let mut extra = profile_ctx;
    if let Some(goals) = &goals_ctx {
        extra.push_str("\n\n");
        extra.push_str(goals);
    }
    let extra_ref = if extra.is_empty() { None } else { Some(extra.as_str()) };

    // HTTP API calls don't have conversation history (stateless)
    match state
        .agent_runtime
        .run(&agent_name, prompt, &[], &state.llm_router, &state.tool_registry, extra_ref)
        .await
    {
        Ok(result) => Ok(Json(json!({
            "agent": agent_name,
            "result": result.output,
            "tool_calls": result.tool_calls_made,
            "elapsed": result.elapsed_secs
        }))),
        Err(e) => {
            tracing::error!("Agent run failed: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Validate the `Authorization: Bearer <token>` header for cluster endpoints.
/// Returns `Ok(())` if auth passes (or is disabled), `Err(StatusCode::UNAUTHORIZED)` otherwise.
fn validate_cluster_auth(state: &AppState, headers: &axum::http::HeaderMap) -> Result<(), StatusCode> {
    let secret = match &state.cluster_secret {
        Some(s) => s,
        None => return Ok(()), // Auth disabled — allow all requests
    };
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match auth_header {
        Some(token) if token == secret => Ok(()),
        _ => {
            warn!("Cluster auth failed: missing or invalid Authorization header");
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

async fn cluster_status(State(state): State<AppState>) -> Json<Value> {
    let nodes = state.cluster.status().await;
    Json(json!({ "nodes": nodes }))
}

/// POST /cluster/register — worker self-registration
async fn cluster_register(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    validate_cluster_auth(&state, &headers)?;
    let name = body.get("name").and_then(|v| v.as_str()).ok_or(StatusCode::BAD_REQUEST)?;
    let host = body.get("host").and_then(|v| v.as_str()).ok_or(StatusCode::BAD_REQUEST)?;
    let port = body.get("port").and_then(|v| v.as_u64()).unwrap_or(7879) as u16;
    let capabilities: Vec<String> = body.get("capabilities")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_else(|| vec!["tools".to_string()]);
    let device_type = body.get("device_type").and_then(|v| v.as_str()).unwrap_or("full");

    match state.cluster.register_full(name, host, port, &capabilities, device_type).await {
        Ok(()) => {
            info!("Worker '{}' registered: {}:{} ({}, caps: {:?})", name, host, port, device_type, capabilities);
            Ok(Json(json!({"status": "registered", "name": name})))
        }
        Err(e) => {
            error!("Worker registration failed: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// POST /cluster/heartbeat — worker heartbeat with cpu_load
async fn cluster_heartbeat(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    validate_cluster_auth(&state, &headers)?;
    let name = body.get("name").and_then(|v| v.as_str()).ok_or(StatusCode::BAD_REQUEST)?;
    let cpu_load = body.get("cpu_load").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;

    match state.cluster.heartbeat(name, cpu_load).await {
        Ok(()) => Ok(Json(json!({"status": "ok"}))),
        Err(e) => {
            warn!("Heartbeat from unknown node '{}': {}", name, e);
            Err(StatusCode::NOT_FOUND)
        }
    }
}

/// GET /cluster/workers — list online workers
async fn cluster_workers(State(state): State<AppState>) -> Json<Value> {
    let workers = state.cluster.online_workers().await;
    Json(json!({ "workers": workers, "count": workers.len() }))
}

/// POST /cluster/dispatch — dispatch a tool to a worker.
/// Body: { "tool": "shell", "input": {...} }
/// Optional targeting: { "worker": "acer" } or { "capability": "ios_build" }
/// Priority: worker > capability > auto-routing
async fn cluster_dispatch(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    if state.estop.is_stopped() {
        return Ok(Json(json!({"error": "E-Stop active"})));
    }
    let tool = body.get("tool").and_then(|v| v.as_str()).ok_or(StatusCode::BAD_REQUEST)?;
    let input = body.get("input").cloned().unwrap_or(json!({}));
    let target_worker = body.get("worker").and_then(|v| v.as_str());
    let target_capability = body.get("capability").and_then(|v| v.as_str());

    let hub = state.cluster_hub.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let result = if let Some(worker_name) = target_worker {
        // Targeted dispatch — specific worker
        info!("Targeted dispatch: {} → worker '{}'", tool, worker_name);
        hub.dispatch_to_worker(worker_name, tool, input).await
    } else if let Some(capability) = target_capability {
        // Capability dispatch — best worker with capability
        info!("Capability dispatch: {} → cap '{}'", tool, capability);
        hub.dispatch_by_capability(capability, tool, input).await
    } else {
        // Auto-routing (original behavior)
        hub.dispatch_tool(tool, input).await
    };

    match result {
        Ok(result) => Ok(Json(result)),
        Err(e) => {
            warn!("Dispatch failed for tool '{}': {}", tool, e);
            Ok(Json(json!({"error": e.to_string()})))
        }
    }
}

/// GET /metrics — Prometheus text exposition format
async fn prometheus_metrics(State(state): State<AppState>) -> (StatusCode, [(axum::http::header::HeaderName, &'static str); 1], String) {
    // Update uptime gauge
    let uptime = state.started_at.elapsed().as_secs();
    state.metrics_registry.gauge_set("clawtex_uptime_seconds", uptime);

    // Update worker count gauge
    let worker_count = state.cluster.online_workers().await.len() as u64;
    state.metrics_registry.gauge_set("clawtex_workers_online", worker_count);

    // Update tool count gauge
    state.metrics_registry.gauge_set("clawtex_tools_registered", state.tool_registry.names().len() as u64);

    let body = state.metrics_registry.render_prometheus();
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

/// GET /metrics/health — JSON health summary
async fn metrics_health(State(state): State<AppState>) -> Json<Value> {
    let uptime = state.started_at.elapsed().as_secs();
    state.metrics_registry.gauge_set("clawtex_uptime_seconds", uptime);
    let worker_count = state.cluster.online_workers().await.len() as u64;
    state.metrics_registry.gauge_set("clawtex_workers_online", worker_count);
    state.metrics_registry.gauge_set("clawtex_tools_registered", state.tool_registry.names().len() as u64);

    Json(state.metrics_registry.render_health_json())
}

/// GET /cluster/metrics — cluster performance data
async fn cluster_metrics(State(state): State<AppState>) -> Json<Value> {
    let hub = match state.cluster_hub.as_ref() {
        Some(h) => h,
        None => return Json(json!({"error": "cluster hub not initialized"})),
    };
    Json(hub.metrics.snapshot().await)
}

/// GET /cluster/metrics/:worker — per-worker stats
async fn cluster_metrics_worker(
    State(state): State<AppState>,
    Path(worker): Path<String>,
) -> Json<Value> {
    let hub = match state.cluster_hub.as_ref() {
        Some(h) => h,
        None => return Json(json!({"error": "cluster hub not initialized"})),
    };
    match hub.metrics.worker_stats(&worker).await {
        Some(stats) => Json(json!(stats)),
        None => Json(json!({"error": format!("No stats for worker '{}'", worker)})),
    }
}

/// GET /cluster/poll?worker=<name> — mobile worker polls for next task (also acts as heartbeat)
async fn cluster_poll(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Result<Json<Value>, StatusCode> {
    validate_cluster_auth(&state, &headers)?;
    let worker_name = params.get("worker").ok_or(StatusCode::BAD_REQUEST)?;

    let hub = state.cluster_hub.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match hub.poll_task(worker_name).await {
        Some(task) => {
            // Check if this is an agent task (tool == "__agent_task__")
            if task.tool == "__agent_task__" {
                debug!("Mobile worker '{}' picked up agent task '{}'", worker_name, task.task_id);
                // The input contains the full AgentTask struct serialized
                let goal = task.input.get("goal").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let max_iterations = task.input.get("max_iterations").and_then(|v| v.as_u64()).unwrap_or(8) as u32;
                let available_tools: Vec<String> = task.input.get("available_tools")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                Ok(Json(json!({
                    "status": "agent_task",
                    "task_id": task.task_id,
                    "goal": goal,
                    "max_iterations": max_iterations,
                    "available_tools": available_tools,
                })))
            } else {
                debug!("Mobile worker '{}' picked up task '{}'", worker_name, task.task_id);
                Ok(Json(json!({
                    "status": "task",
                    "task_id": task.task_id,
                    "tool": task.tool,
                    "input": task.input,
                })))
            }
        }
        None => Ok(Json(json!({ "status": "idle" }))),
    }
}

/// POST /cluster/result — mobile worker submits completed task result
async fn cluster_result(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    validate_cluster_auth(&state, &headers)?;
    let task_id = body.get("task_id").and_then(|v| v.as_str()).ok_or(StatusCode::BAD_REQUEST)?;
    let success = body.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
    let output = body.get("output").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let worker = body.get("worker").and_then(|v| v.as_str()).map(|s| s.to_string());

    let hub = state.cluster_hub.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let payload = TaskResultPayload {
        task_id: task_id.to_string(),
        success,
        output,
        worker,
    };

    match hub.submit_result(payload).await {
        Ok(()) => {
            info!("Mobile task '{}' result submitted", task_id);
            Ok(Json(json!({"status": "accepted"})))
        }
        Err(e) => {
            warn!("Failed to submit result for task '{}': {}", task_id, e);
            Ok(Json(json!({"status": "error", "message": e.to_string()})))
        }
    }
}

/// POST /cluster/onboard — start onboarding a new worker
async fn cluster_onboard(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let onboarder = state.worker_onboarder.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let config: OnboardConfig = serde_json::from_value(body)
        .map_err(|e| {
            warn!("Invalid onboard config: {}", e);
            StatusCode::BAD_REQUEST
        })?;

    info!("Onboarding worker '{}' (type: {})", config.worker_name, config.worker_type);

    match onboarder.onboard_worker(config).await {
        Ok(result) => Ok(Json(serde_json::to_value(result).unwrap_or(json!({"error": "serialization failed"})))),
        Err(e) => {
            error!("Onboarding failed: {}", e);
            Ok(Json(json!({"error": e.to_string()})))
        }
    }
}

/// GET /cluster/onboard/status/:worker — check onboarding status for a worker
async fn cluster_onboard_status(
    State(state): State<AppState>,
    Path(worker): Path<String>,
) -> Json<Value> {
    let onboarder = match state.worker_onboarder.as_ref() {
        Some(o) => o,
        None => return Json(json!({"error": "onboarder not initialized"})),
    };

    match onboarder.get_status(&worker).await {
        Some(status) => Json(serde_json::to_value(status).unwrap_or(json!({"error": "serialization failed"}))),
        None => {
            // No active onboarding — check if worker exists via verify
            match onboarder.verify_worker(&worker).await {
                Ok(health) => Json(json!({
                    "worker_name": worker,
                    "state": if health.registered { "registered" } else { "unknown" },
                    "health": serde_json::to_value(health).unwrap_or(json!({})),
                })),
                Err(e) => Json(json!({
                    "worker_name": worker,
                    "state": "unknown",
                    "error": e.to_string(),
                })),
            }
        }
    }
}

/// GET /cluster/onboard/verify/:worker — verify worker health
async fn cluster_onboard_verify(
    State(state): State<AppState>,
    Path(worker): Path<String>,
) -> Json<Value> {
    let onboarder = match state.worker_onboarder.as_ref() {
        Some(o) => o,
        None => return Json(json!({"error": "onboarder not initialized"})),
    };

    match onboarder.verify_worker(&worker).await {
        Ok(health) => Json(serde_json::to_value(health).unwrap_or(json!({"error": "serialization failed"}))),
        Err(e) => Json(json!({"error": e.to_string()})),
    }
}

/// POST /cluster/onboard/mobile — generate mobile worker join link
async fn cluster_onboard_mobile(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let worker_name = body.get("worker_name").and_then(|v| v.as_str()).ok_or(StatusCode::BAD_REQUEST)?;
    let hub_url = body.get("hub_url").and_then(|v| v.as_str()).ok_or(StatusCode::BAD_REQUEST)?;
    let auth_token = body.get("auth_token").and_then(|v| v.as_str());

    let link = clawtex_core::WorkerOnboarder::generate_mobile_link(hub_url, auth_token, worker_name);

    // Also pre-register in the registry
    let capabilities = body.get("capabilities")
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
        .unwrap_or_else(|| vec!["web_search".to_string(), "http_request".to_string()]);

    let _ = state.cluster.register_full(
        worker_name,
        "0.0.0.0",
        0,
        &capabilities,
        "mobile",
    ).await;

    Ok(Json(json!({
        "worker_name": worker_name,
        "deep_link": link,
        "instructions": "Open this link on the mobile device, or scan the QR code in the app"
    })))
}

/// POST /cluster/consistency-test -- run cross-device consistency tests.
async fn cluster_consistency_test(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    if state.estop.is_stopped() {
        return Ok(Json(json!({"error": "E-Stop active"})));
    }
    let hub = state.cluster_hub.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let home = dirs_home();
    let db_path = format!("{}/.clawtex/consistency.db", home);
    let tester = ConsistencyTester::new(&db_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let threshold = body.get("threshold").and_then(|v| v.as_f64()).unwrap_or(0.90);
    let tester = tester.with_threshold(threshold);
    let use_predefined = body.get("predefined").and_then(|v| v.as_bool()).unwrap_or(false);
    let workers: Vec<String> = if let Some(arr) = body.get("workers").and_then(|v| v.as_array()) {
        arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
    } else {
        hub.registry.online_workers().await.iter().map(|w| w.name.clone()).collect()
    };
    if workers.len() < 2 {
        return Ok(Json(json!({"error": "Need at least 2 workers", "online_workers": workers.len()})));
    }
    if use_predefined {
        let summary = tester.run_predefined_suite(workers, hub).await;
        return Ok(Json(json!({"status": "complete", "total_prompts": summary.total_prompts, "passed": summary.passed, "failed": summary.failed, "avg_similarity": summary.avg_similarity, "reports": summary.reports})));
    }
    let prompts: Vec<String> = body.get("prompts").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    if prompts.is_empty() {
        return Ok(Json(json!({"error": "Provide 'prompts' array or set 'predefined': true"})));
    }
    let reports = tester.run_batch(prompts, workers, hub).await;
    let passed = reports.iter().filter(|r| r.pass).count();
    let total = reports.len();
    let avg_sim = if total > 0 { reports.iter().map(|r| r.avg_similarity).sum::<f64>() / total as f64 } else { 0.0 };
    Ok(Json(json!({"status": "complete", "total_prompts": total, "passed": passed, "failed": total - passed, "avg_similarity": avg_sim, "reports": reports})))
}

/// GET /cluster/consistency-history -- view historical consistency test results
async fn cluster_consistency_history(
    State(_state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Json<Value> {
    let home = dirs_home();
    let db_path = format!("{}/.clawtex/consistency.db", home);
    let tester = match ConsistencyTester::new(&db_path) {
        Ok(t) => t,
        Err(e) => return Json(json!({"error": format!("Failed to open DB: {}", e)})),
    };
    let limit = params.get("limit").and_then(|v| v.parse::<usize>().ok()).unwrap_or(20);
    match tester.history_summary() {
        Ok(summary) => {
            let recent = tester.recent_reports(limit).unwrap_or_default();
            Json(json!({"summary": summary, "recent_reports": recent}))
        }
        Err(e) => Json(json!({"error": e.to_string()})),
    }
}

async fn preemption_pending(State(state): State<AppState>) -> Json<Value> { let mgr = match state.preemption_manager.as_ref() { Some(m) => m, None => return Json(json!({"error": "preemption manager not initialized"})) }; match mgr.pending_restorations() { Ok(records) => Json(json!({"pending": records, "count": records.len()})), Err(e) => Json(json!({"error": e.to_string()})) } }
async fn preemption_history(State(state): State<AppState>, axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>) -> Json<Value> { let mgr = match state.preemption_manager.as_ref() { Some(m) => m, None => return Json(json!({"error": "preemption manager not initialized"})) }; let limit = params.get("limit").and_then(|v| v.parse::<i64>().ok()).unwrap_or(50); match mgr.history(limit) { Ok(records) => Json(json!({"history": records, "count": records.len()})), Err(e) => Json(json!({"error": e.to_string()})) } }
async fn cluster_scores(State(state): State<AppState>) -> Json<Value> { let scorer = match state.node_scorer.as_ref() { Some(s) => s, None => return Json(json!({"error": "node scorer not initialized"})) }; let rankings = scorer.get_rankings(); let nodes: Vec<Value> = rankings.iter().map(|(id, score)| { json!({"node_id": id, "stability": score.stability, "speed": score.speed, "cost_efficiency": score.cost_efficiency, "quality": score.quality, "overall": score.overall, "grade": format!("{}", score.grade)}) }).collect(); Json(json!({"rankings": nodes, "count": nodes.len()})) }
async fn cluster_score_node(State(state): State<AppState>, Path(node_id): Path<String>) -> Json<Value> { let scorer = match state.node_scorer.as_ref() { Some(s) => s, None => return Json(json!({"error": "node scorer not initialized"})) }; match scorer.get_node_details(&node_id) { Some((metrics, score)) => Json(json!({"node_id": node_id, "metrics": metrics, "score": score})), None => Json(json!({"error": format!("No data for node '{}'", node_id)})) } }
async fn cluster_score_update(State(state): State<AppState>, Path(node_id): Path<String>, Json(body): Json<Value>) -> Result<Json<Value>, StatusCode> { let scorer = match state.node_scorer.as_ref() { Some(s) => s, None => return Err(StatusCode::SERVICE_UNAVAILABLE) }; let metrics = NodeMetrics { success_count: body.get("success_count").and_then(|v| v.as_u64()).unwrap_or(0), failure_count: body.get("failure_count").and_then(|v| v.as_u64()).unwrap_or(0), avg_latency_ms: body.get("avg_latency_ms").and_then(|v| v.as_f64()).unwrap_or(0.0), total_cost: body.get("total_cost").and_then(|v| v.as_f64()).unwrap_or(0.0), quality_score: body.get("quality_score").and_then(|v| v.as_f64()).unwrap_or(0.0) }; match scorer.update_metrics(&node_id, metrics) { Ok(score) => Ok(Json(json!({"node_id": node_id, "score": score, "status": "updated"}))), Err(e) => { warn!("Failed to update node score for '{}': {}", node_id, e); Err(StatusCode::INTERNAL_SERVER_ERROR) } } }
async fn power_nodes(State(state): State<AppState>) -> Json<Value> {
    let power = match state.power_economics.as_ref() {
        Some(p) => p,
        None => return Json(json!({"error": "power economics not initialized"})),
    };

    let cluster_nodes = state.cluster.status().await;
    let profiles = match power.list_profiles() {
        Ok(p) => p,
        Err(e) => return Json(json!({"error": e.to_string()})),
    };

    let nodes: Vec<Value> = profiles
        .iter()
        .map(|profile| {
            let live = cluster_nodes.iter().find(|n| n.name == profile.node_id);
            let live_load = live.map(|n| n.cpu_load as f64).unwrap_or(1.0);
            let live_hourly = power.estimate_hourly_cost(&profile.node_id, live_load).ok();
            let full_load = power.estimate_hourly_cost(&profile.node_id, 1.0).ok();
            json!({
                "profile": profile,
                "cluster": live.map(|n| json!({
                    "status": n.status,
                    "cpu_load": n.cpu_load,
                    "device_type": n.device_type,
                    "host": n.host,
                    "port": n.port,
                })),
                "live_hourly_cost": live_hourly,
                "full_load_hourly_cost": full_load,
            })
        })
        .collect();

    Json(json!({"nodes": nodes, "count": nodes.len()}))
}

async fn power_node_detail(State(state): State<AppState>, Path(node_id): Path<String>) -> Json<Value> {
    let power = match state.power_economics.as_ref() {
        Some(p) => p,
        None => return Json(json!({"error": "power economics not initialized"})),
    };

    let profile = match power.get_profile(&node_id) {
        Ok(Some(p)) => p,
        Ok(None) => return Json(json!({"error": format!("No power profile for node '{}'", node_id)})),
        Err(e) => return Json(json!({"error": e.to_string()})),
    };

    let cluster_node = state.cluster.get_node(&node_id).await;
    let live_load = cluster_node.as_ref().map(|n| n.cpu_load as f64).unwrap_or(1.0);
    let idle_hourly = power.estimate_hourly_cost(&node_id, 0.0).ok();
    let mid_hourly = power.estimate_hourly_cost(&node_id, 0.5).ok();
    let full_hourly = power.estimate_hourly_cost(&node_id, 1.0).ok();
    let live_hourly = power.estimate_hourly_cost(&node_id, live_load).ok();

    Json(json!({
        "profile": profile,
        "cluster": cluster_node,
        "hourly_idle": idle_hourly,
        "hourly_mid": mid_hourly,
        "hourly_full": full_hourly,
        "hourly_live": live_hourly,
    }))
}

async fn power_node_upsert(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    Json(body): Json<PowerProfileUpsertRequest>,
) -> Result<Json<Value>, StatusCode> {
    let power = state.power_economics.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let profile = NodePowerProfile {
        node_id: node_id.clone(),
        idle_watts: body.idle_watts,
        active_watts: body.active_watts,
        electricity_usd_per_kwh: body.electricity_usd_per_kwh,
        depreciation_usd_per_hour: body.depreciation_usd_per_hour,
        cooling_usd_per_hour: body.cooling_usd_per_hour,
        notes: body.notes.clone(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };

    power.upsert_profile(&profile).map_err(|e| {
        warn!("Failed to upsert power profile for '{}': {}", node_id, e);
        StatusCode::BAD_REQUEST
    })?;

    let stored = power
        .get_profile(&node_id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({"status": "updated", "profile": stored})))
}

async fn power_estimate(
    State(state): State<AppState>,
    Json(body): Json<PowerEstimateRequest>,
) -> Result<Json<Value>, StatusCode> {
    let power = state.power_economics.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match power.estimate_run_cost(&body.node_id, body.duration_secs, body.load_factor) {
        Ok(estimate) => Ok(Json(json!(estimate))),
        Err(e) => {
            warn!("Power estimate failed for '{}': {}", body.node_id, e);
            Err(StatusCode::BAD_REQUEST)
        }
    }
}

async fn power_profitability(
    State(state): State<AppState>,
    Json(body): Json<PowerProfitabilityRequest>,
) -> Result<Json<Value>, StatusCode> {
    let power = state.power_economics.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match power.assess_profitability(
        &body.node_id,
        body.expected_revenue_per_hour_usd,
        body.api_cost_per_hour_usd,
        body.load_factor,
    ) {
        Ok(assessment) => Ok(Json(json!(assessment))),
        Err(e) => {
            warn!("Power profitability failed for '{}': {}", body.node_id, e);
            Err(StatusCode::BAD_REQUEST)
        }
    }
}

async fn pricing_rules(State(state): State<AppState>) -> Json<Value> {
    let pricing = match state.provider_pricing.as_ref() {
        Some(p) => p,
        None => return Json(json!({"error": "provider pricing not initialized"})),
    };
    match pricing.list_rules() {
        Ok(rules) => Json(json!({"rules": rules, "count": rules.len()})),
        Err(e) => Json(json!({"error": e.to_string()})),
    }
}

async fn pricing_rules_provider(
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> Json<Value> {
    let pricing = match state.provider_pricing.as_ref() {
        Some(p) => p,
        None => return Json(json!({"error": "provider pricing not initialized"})),
    };
    match pricing.list_rules_for_provider(Some(&provider)) {
        Ok(rules) => Json(json!({"provider": provider, "rules": rules, "count": rules.len()})),
        Err(e) => Json(json!({"error": e.to_string()})),
    }
}

async fn pricing_rule_upsert(
    State(state): State<AppState>,
    Json(body): Json<PricingRuleUpsertRequest>,
) -> Result<Json<Value>, StatusCode> {
    let pricing = state.provider_pricing.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let rule = ProviderPriceRule {
        provider: body.provider,
        model_pattern: body.model_pattern,
        input_usd_per_1m_tokens: body.input_usd_per_1m_tokens,
        output_usd_per_1m_tokens: body.output_usd_per_1m_tokens,
        notes: body.notes,
        updated_at: chrono::Utc::now().to_rfc3339(),
    };

    pricing.upsert_rule(&rule).map_err(|e| {
        warn!(
            "Failed to upsert provider price rule for {}:{}: {}",
            rule.provider, rule.model_pattern, e
        );
        StatusCode::BAD_REQUEST
    })?;

    let stored = pricing
        .get_rule(&rule.provider, &rule.model_pattern)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({"status": "updated", "rule": stored})))
}

async fn pricing_estimate(
    State(state): State<AppState>,
    Json(body): Json<PricingEstimateRequest>,
) -> Result<Json<Value>, StatusCode> {
    let pricing = state.provider_pricing.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match pricing.estimate_cost(&body.provider, &body.model, body.tokens_in, body.tokens_out) {
        Ok(estimate) => Ok(Json(json!(estimate))),
        Err(e) => {
            warn!(
                "Pricing estimate failed for {}:{}: {}",
                body.provider, body.model, e
            );
            Err(StatusCode::BAD_REQUEST)
        }
    }
}

async fn tools_list(State(state): State<AppState>) -> Json<Value> {
    let specs = state.tool_registry.specs();
    let tools: Vec<Value> = specs
        .iter()
        .map(|s| json!({"name": s.name, "description": s.description}))
        .collect();
    Json(json!({ "tools": tools }))
}

async fn hands_list(State(state): State<AppState>) -> Json<Value> {
    let hands: Vec<Value> = state.hands.list().iter().map(|h| {
        json!({
            "name": h.name,
            "description": h.description,
            "category": h.category,
            "phases": h.phases.len(),
            "tools": h.tools,
        })
    }).collect();
    Json(json!({ "hands": hands, "count": hands.len() }))
}

async fn hand_run(
    State(state): State<AppState>,
    Path(hand_name): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    // E-Stop guard
    if state.estop.is_stopped() {
        return Ok(Json(json!({"error": "E-Stop active — all agent operations halted"})));
    }

    let user_input = body.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
    if user_input.is_empty() {
        return Ok(Json(json!({"error": "Missing 'prompt' field"})));
    }

    let hand = match state.hands.get(&hand_name) {
        Some(h) => h.clone(),
        None => {
            let available = state.hands.names().join(", ");
            return Ok(Json(json!({
                "error": format!("Unknown hand '{}'. Available: {}", hand_name, available)
            })));
        }
    };

    match HandRunner::run(&hand, user_input, &state.agent_runtime, &state.llm_router, &state.tool_registry, Some(&state.approval_gate)).await {
        Ok(result) => {
            let mut response = json!({
                "hand": result.hand_name,
                "phases_completed": result.phases_completed,
                "total_phases": result.total_phases,
                "final_output": result.final_output,
                "elapsed_secs": result.elapsed_secs,
                "phase_details": result.outputs.iter().map(|o| json!({
                    "phase": o.phase_name,
                    "tool_calls": o.tool_calls,
                    "output_length": o.output.len(),
                })).collect::<Vec<_>>(),
            });

            // Execute chained hand if configured
            if let Some(ref next_hand_name) = result.chain_to {
                if let Some(next_hand) = state.hands.get(next_hand_name) {
                    let next_hand = next_hand.clone();
                    let chain_input = format!(
                        "Previous hand '{}' output:\n\n{}\n\nOriginal request: {}",
                        result.hand_name, result.final_output, user_input
                    );
                    match HandRunner::run(&next_hand, &chain_input, &state.agent_runtime, &state.llm_router, &state.tool_registry, Some(&state.approval_gate)).await {
                        Ok(chain_result) => {
                            response["chained_hand"] = json!({
                                "hand": chain_result.hand_name,
                                "phases_completed": chain_result.phases_completed,
                                "total_phases": chain_result.total_phases,
                                "final_output": chain_result.final_output,
                                "elapsed_secs": chain_result.elapsed_secs,
                            });
                        }
                        Err(e) => {
                            response["chain_error"] = json!(format!("{}", e));
                        }
                    }
                }
            }

            Ok(Json(response))
        }
        Err(e) => {
            tracing::error!("Hand '{}' failed: {}", hand_name, e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn workspace_files(State(state): State<AppState>) -> Json<Value> {
    let ws_dir = state.tool_registry.workspace_dir().to_string();
    let ws_path = std::path::Path::new(&ws_dir);

    if !ws_path.exists() {
        return Json(json!({ "files": [], "count": 0, "workspace": ws_dir }));
    }

    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(ws_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = path.is_dir();
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let modified = entry.metadata().ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            files.push(json!({
                "name": name,
                "is_dir": is_dir,
                "size": size,
                "modified_epoch": modified,
            }));
        }
    }

    let count = files.len();
    Json(json!({ "files": files, "count": count, "workspace": ws_dir }))
}

async fn costs_summary(State(state): State<AppState>) -> Json<Value> {
    let ct = match &state.cost_tracker {
        Some(ct) => ct,
        None => return Json(json!({ "error": "Cost tracker not available" })),
    };
    let today = ct.today_total().unwrap_or(CostSummary {
        group: "today".into(),
        total_tokens: 0,
        api_cost_usd: 0.0,
        hardware_cost_usd: 0.0,
        total_cost_usd: 0.0,
        call_count: 0,
    });
    let by_provider = ct.by_provider(7).unwrap_or_default();
    let by_agent = ct.by_agent(7).unwrap_or_default();
    let by_day = ct.by_day(7).unwrap_or_default();
    Json(json!({
        "today": {
            "tokens": today.total_tokens,
            "api_cost_usd": today.api_cost_usd,
            "hardware_cost_usd": today.hardware_cost_usd,
            "cost_usd": today.total_cost_usd,
            "calls": today.call_count
        },
        "by_provider_7d": by_provider,
        "by_agent_7d": by_agent,
        "by_day_7d": by_day,
    }))
}

async fn revenue_summary(State(state): State<AppState>) -> Json<Value> {
    let rt = match &state.revenue_tracker {
        Some(rt) => rt,
        None => return Json(json!({ "error": "Revenue tracker not available" })),
    };
    let today = rt.today_total().unwrap_or(RevenueSummary { group: "today".into(), total_usd: 0.0, count: 0 });
    let by_route = rt.by_route(30).unwrap_or_default();
    let by_source = rt.by_source(30).unwrap_or_default();
    let by_day = rt.by_day(30).unwrap_or_default();
    Json(json!({
        "today": { "total_usd": today.total_usd, "count": today.count },
        "by_route_30d": by_route,
        "by_source_30d": by_source,
        "by_day_30d": by_day,
    }))
}

async fn optimizer_policies(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Json<Value> {
    let store = match &state.optimizer_store {
        Some(store) => store,
        None => return Json(json!({ "error": "Optimizer store not available" })),
    };
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50);
    match store.list_policies(limit) {
        Ok(policies) => Json(json!({ "policies": policies, "count": policies.len() })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

async fn optimizer_runs(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Json<Value> {
    let store = match &state.optimizer_store {
        Some(store) => store,
        None => return Json(json!({ "error": "Optimizer store not available" })),
    };
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50);
    match store.list_runs(limit) {
        Ok(runs) => Json(json!({ "runs": runs, "count": runs.len() })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

// ── Operations Report Endpoints ──────────────────────────────────────────────

/// GET /reports/daily -- generate and return daily ops report
async fn report_daily(State(state): State<AppState>) -> Json<Value> {
    let reporter = OpsReporter::new(5.0);
    let report = reporter
        .generate_daily_report(
            state.cost_tracker.as_deref(),
            Some(&*state.cluster),
            Some(&*state.task_queue),
        )
        .await;
    Json(serde_json::to_value(&report).unwrap_or(json!({"error": "serialization failed"})))
}

/// GET /reports/weekly -- generate and return weekly ops report
async fn report_weekly(State(state): State<AppState>) -> Json<Value> {
    let reporter = OpsReporter::new(5.0);
    let report = reporter
        .generate_weekly_report(
            state.cost_tracker.as_deref(),
            Some(&*state.cluster),
            Some(&*state.task_queue),
        )
        .await;
    Json(serde_json::to_value(&report).unwrap_or(json!({"error": "serialization failed"})))
}

/// POST /reports/send -- generate daily report and format for Telegram
async fn report_send(State(state): State<AppState>) -> Json<Value> {
    let reporter = OpsReporter::new(5.0);
    let report = reporter
        .generate_daily_report(
            state.cost_tracker.as_deref(),
            Some(&*state.cluster),
            Some(&*state.task_queue),
        )
        .await;
    let telegram_text = OpsReporter::format_telegram(&report);
    Json(json!({
        "report": report,
        "telegram_text": telegram_text,
        "status": "generated"
    }))
}

// ── Service Tier Endpoints ───────────────────────────────────────────────────

/// GET /tier/:agent -- get agent's tier info and limits
async fn tier_get(
    State(state): State<AppState>,
    Path(agent): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let mgr = state.service_tier.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let tier = mgr.get_tier(&agent);
    let limits = mgr.get_limits(tier);
    let usage = mgr.get_usage(&agent);
    Ok(Json(json!({
        "agent": agent,
        "tier": tier,
        "limits": {
            "max_tasks_per_day": limits.max_tasks_per_day,
            "max_storage_bytes": limits.max_storage_bytes,
            "priority_boost": limits.priority_boost,
            "max_concurrent_agents": limits.max_concurrent_agents,
        },
        "usage": {
            "tasks_today": usage.tasks_today,
            "tasks_limit": usage.tasks_limit,
            "storage_used": usage.storage_used,
            "storage_limit": usage.storage_limit,
        }
    })))
}

/// PUT /tier/:agent — set agent's tier
async fn tier_set(
    State(state): State<AppState>,
    Path(agent): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let mgr = state.service_tier.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let tier_str = body.get("tier").and_then(|v| v.as_str()).ok_or(StatusCode::BAD_REQUEST)?;
    let tier = ServiceTier::from_str_loose(tier_str).ok_or(StatusCode::BAD_REQUEST)?;
    mgr.set_tier(&agent, tier).map_err(|e| {
        warn!("Failed to set tier for '{}': {}", agent, e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(json!({
        "agent": agent,
        "tier": tier,
        "status": "updated"
    })))
}

/// GET /tier/:agent/usage — get agent's current usage stats
async fn tier_usage(
    State(state): State<AppState>,
    Path(agent): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let mgr = state.service_tier.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let usage = mgr.get_usage(&agent);
    Ok(Json(json!(usage)))
}


// ── Tenant Endpoints ──────────────────────────────────────────────────────────

async fn tenant_create(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let mgr = state.tenant_manager.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let name = body.get("name").and_then(|v| v.as_str()).ok_or(StatusCode::BAD_REQUEST)?;
    let tier_str = body.get("tier").and_then(|v| v.as_str()).unwrap_or("lite");
    let tier = ServiceTier::from_str_loose(tier_str).ok_or(StatusCode::BAD_REQUEST)?;
    match mgr.create_tenant(name, tier) {
        Ok(tenant) => Ok(Json(json!(tenant))),
        Err(e) => {
            tracing::error!("Failed to create tenant: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn tenant_list(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let mgr = state.tenant_manager.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let tenants = mgr.list_tenants();
    Ok(Json(json!({ "tenants": tenants, "count": tenants.len() })))
}

async fn tenant_get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let mgr = state.tenant_manager.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match mgr.get_tenant(&id) {
        Some(tenant) => Ok(Json(json!(tenant))),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn tenant_update_tier(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let mgr = state.tenant_manager.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let tier_str = body.get("tier").and_then(|v| v.as_str()).ok_or(StatusCode::BAD_REQUEST)?;
    let tier = ServiceTier::from_str_loose(tier_str).ok_or(StatusCode::BAD_REQUEST)?;
    mgr.update_tier(&id, tier).map_err(|e| {
        tracing::error!("Failed to update tenant tier: {}", e);
        StatusCode::NOT_FOUND
    })?;
    Ok(Json(json!({ "status": "ok", "id": id, "tier": tier.to_string() })))
}

async fn tenant_deactivate(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let mgr = state.tenant_manager.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    mgr.deactivate_tenant(&id).map_err(|e| {
        tracing::error!("Failed to deactivate tenant: {}", e);
        StatusCode::NOT_FOUND
    })?;
    Ok(Json(json!({ "status": "ok", "id": id, "message": "Tenant deactivated" })))
}

async fn tenant_validate(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Result<Json<Value>, StatusCode> {
    let mgr = state.tenant_manager.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let key = params.get("key").ok_or(StatusCode::BAD_REQUEST)?;
    match mgr.validate_api_key(key) {
        Some(tenant) => Ok(Json(json!({ "valid": true, "tenant": tenant }))),
        None => Ok(Json(json!({ "valid": false }))),
    }
}

async fn estop_activate(State(state): State<AppState>) -> Json<Value> {
    state.estop.stop();
    Json(json!({ "status": "stopped", "message": "Emergency stop activated" }))
}

async fn estop_reset(State(state): State<AppState>) -> Json<Value> {
    state.estop.reset();
    Json(json!({ "status": "running", "message": "Emergency stop deactivated" }))
}

async fn estop_status(State(state): State<AppState>) -> Json<Value> {
    let stopped = state.estop.is_stopped();
    Json(json!({
        "stopped": stopped,
        "status": if stopped { "stopped" } else { "running" }
    }))
}

// ── Load Testing Endpoints ──────────────────────────────────────────
async fn test_stress_start(State(state): State<AppState>, Json(body): Json<Value>) -> Result<Json<Value>, StatusCode> {
    let tester = match &state.load_tester { Some(t) => t.clone(), None => return Ok(Json(json!({"error":"Load tester not available"}))) };
    let st = tester.status().await;
    if st.running { return Ok(Json(json!({"error":"Test already running","run_id":st.run_id}))); }
    let (cfg, pn) = if let Some(ps) = body.get("profile").and_then(|v| v.as_str()) {
        match clawtex_core::load_test::profile(ps) { Some(c) => (c, Some(ps.to_string())), None => return Ok(Json(json!({"error":format!("Unknown profile '{}'", ps)}))) }
    } else { match serde_json::from_value::<StressTestConfig>(body.clone()) { Ok(c) => (c, None), Err(e) => return Ok(Json(json!({"error":format!("Invalid config: {}", e)}))) } };
    let cs = cfg.clone(); let pc = pn.clone();
    tokio::spawn(async move { let _ = tester.run_stress_test(cfg, pc).await; });
    Ok(Json(json!({"status":"started","config":cs,"profile":pn})))
}
async fn test_stress_status(State(state): State<AppState>) -> Json<Value> {
    match &state.load_tester { Some(t) => Json(serde_json::to_value(&t.status().await).unwrap_or(json!({}))), None => Json(json!({"error":"not available"})) }
}
async fn test_stress_history(State(state): State<AppState>, axum::extract::Query(p): axum::extract::Query<HashMap<String, String>>) -> Json<Value> {
    let t = match &state.load_tester { Some(t) => t, None => return Json(json!({"error":"not available"})) };
    let lim = p.get("limit").and_then(|v| v.parse::<usize>().ok()).unwrap_or(20);
    match t.store() { Some(s) => match s.recent(lim) { Ok(r) => Json(json!({"results":r,"count":r.len()})), Err(e) => Json(json!({"error":e.to_string()})) }, None => Json(json!({"error":"no store"})) }
}
async fn test_stress_report(State(state): State<AppState>, Path(rid): Path<String>) -> Json<Value> {
    let t = match &state.load_tester { Some(t) => t, None => return Json(json!({"error":"not available"})) };
    match t.store() { Some(s) => match s.get_report(&rid) { Ok(Some(r)) => Json(r), Ok(None) => Json(json!({"error":"not found"})), Err(e) => Json(json!({"error":e.to_string()})) }, None => Json(json!({"error":"no store"})) }
}
async fn test_profiles() -> Json<Value> {
    let p: Vec<Value> = clawtex_core::load_test::profile_names().iter().filter_map(|n| clawtex_core::load_test::profile(n).map(|c| json!({"name":n,"concurrent_tasks":c.concurrent_tasks,"duration_secs":c.duration_secs,"multiplier":c.multiplier}))).collect();
    Json(json!({"profiles":p}))
}

/// GET /audit — query audit log with optional filters
/// Query params: agent, action_type, risk_level, tool, outcome, limit
async fn audit_query(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Json<Value> {
    let logger = match &state.audit_logger {
        Some(l) => l,
        None => return Json(json!({"error": "Audit logger not available"})),
    };

    let filter = AuditFilter {
        agent: params.get("agent").cloned(),
        action_type: params.get("action_type")
            .and_then(|s| clawtex_core::ActionType::from_str(s)),
        risk_level: params.get("risk_level")
            .and_then(|s| clawtex_core::RiskLevel::from_str(s)),
        tool_name: params.get("tool").cloned(),
        outcome: params.get("outcome")
            .and_then(|s| clawtex_core::Outcome::from_str(s)),
        start_time: None,
        end_time: None,
        limit: params.get("limit").and_then(|s| s.parse().ok()),
    };

    match logger.query_audit(&filter).await {
        Ok(entries) => {
            let count = entries.len();
            Json(json!({
                "entries": entries,
                "count": count,
            }))
        }
        Err(e) => Json(json!({"error": format!("Audit query failed: {}", e)})),
    }
}

async fn dashboard(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Result<Html<String>, StatusCode> {
    // Token authentication
    let token = params.get("token").map(|s| s.as_str()).unwrap_or("");
    if token != state.dashboard_token {
        return Err(StatusCode::FORBIDDEN);
    }

    let tasks = state.task_queue.history(50).await.unwrap_or_default();
    let tools = state.tool_registry.names();
    let agents = state.agent_runtime.list_agents();

    // Quick Ollama health check
    let ollama_status = match state.llm_router.route("ping", "ollama").await {
        Ok(_) => "connected",
        Err(_) => "disconnected",
    };

    let uptime_secs = state.started_at.elapsed().as_secs();
    let active_chats = state.conversations.active_count().await;

    Ok(clawtex_core::dashboard::render(
        &tasks, &tools, &agents, ollama_status, &state.dashboard_token,
        uptime_secs, active_chats,
    ))
}

// ── Telegram message handler ───────────────────────────────────────────────────

async fn handle_telegram_messages(
    mut rx: mpsc::Receiver<ChannelMessage>,
    telegram: Arc<TelegramChannel>,
    state: AppState,
    last_chat_id: Arc<tokio::sync::RwLock<Option<String>>>,
) {
    info!("Telegram message handler started");

    while let Some(msg) = rx.recv().await {
        let telegram = telegram.clone();
        let state = state.clone();
        // Track the latest chat_id for approval notifications
        {
            *last_chat_id.write().await = Some(msg.chat_id.clone());
        }

        tokio::spawn(async move {
            let chat_id = msg.chat_id.clone();
            let text = msg.text.trim().to_string();

            let title = if text.len() > 40 {
                format!(
                    "{}...",
                    &text[..text.char_indices().nth(40).map(|(i, _)| i).unwrap_or(text.len())]
                )
            } else {
                text.clone()
            };

            info!("Processing message from @{}: {}", msg.sender, title);

            // Handle commands
            if text == "/clear" || text == "/reset" {
                state.conversations.clear(&chat_id).await;
                let _ = telegram.send(&chat_id, "Conversation cleared.").await;
                return;
            }

            // ── Goals inline keyboard callbacks ──────────────────────
            if let Some(mood_str) = text.strip_prefix("/goals_mood ") {
                if let Ok(mood) = mood_str.trim().parse::<i32>() {
                    if let Some(ref gs) = state.goals_store {
                        // Record check-in for all active goals
                        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
                        let goals = gs.list_goals(Some(clawtex_core::goals::GoalStatus::Active)).unwrap_or_default();
                        let mood_emojis = ["", "\u{1f622}", "\u{1f614}", "\u{1f610}", "\u{1f60a}", "\u{1f929}"];
                        let mood_labels = ["", "糟糕", "不太好", "普通", "不錯", "超棒"];
                        for g in &goals {
                            let ci = clawtex_core::goals::CheckIn {
                                id: format!("ci-{}-{}", g.id, today),
                                goal_id: g.id.clone(),
                                date: today.clone(),
                                mood,
                                note: None,
                                ai_feedback: None,
                            };
                            let _ = gs.add_check_in(&ci);
                        }
                        let emoji = mood_emojis.get(mood as usize).unwrap_or(&"");
                        let label = mood_labels.get(mood as usize).unwrap_or(&"");
                        let reply = format!(
                            "{} 已記錄今日心情：{} ({})\n\n為 {} 個目標記錄了 check-in。晚安！",
                            emoji, label, mood, goals.len()
                        );
                        let _ = telegram.send(&chat_id, &reply).await;
                    } else {
                        let _ = telegram.send(&chat_id, "目標系統尚未初始化").await;
                    }
                }
                return;
            }

            if let Some(task_id) = text.strip_prefix("/goals_task ") {
                let task_id = task_id.trim();
                if let Some(ref gs) = state.goals_store {
                    match gs.complete_recurring_task(task_id) {
                        Ok(Some(streak)) => {
                            let streak_msg = if streak > 1 {
                                format!(" \u{1f525} 連續 {} 天！", streak)
                            } else {
                                String::new()
                            };
                            let _ = telegram.send(&chat_id, &format!(
                                "\u{2705} 任務完成！{}",
                                streak_msg
                            )).await;
                        }
                        Ok(None) => {
                            let _ = telegram.send(&chat_id, "找不到這個任務").await;
                        }
                        Err(e) => {
                            let _ = telegram.send(&chat_id, &format!("完成任務失敗：{}", e)).await;
                        }
                    }
                } else {
                    let _ = telegram.send(&chat_id, "目標系統尚未初始化").await;
                }
                return;
            }

            // ── Mood number reply (1-5) — quick check-in shortcut ──────
            if text.len() == 1 {
                if let Ok(mood) = text.parse::<i32>() {
                    if (1..=5).contains(&mood) {
                        // Check if there's a recent evening check-in push (within last hour)
                        // Simple heuristic: if any active goals exist, treat bare 1-5 as mood
                        if let Some(ref gs) = state.goals_store {
                            let goals = gs.list_goals(Some(clawtex_core::goals::GoalStatus::Active)).unwrap_or_default();
                            if !goals.is_empty() {
                                let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
                                let mood_emojis = ["", "\u{1f622}", "\u{1f614}", "\u{1f610}", "\u{1f60a}", "\u{1f929}"];
                                for g in &goals {
                                    let ci = clawtex_core::goals::CheckIn {
                                        id: format!("ci-{}-{}", g.id, today),
                                        goal_id: g.id.clone(),
                                        date: today.clone(),
                                        mood,
                                        note: None,
                                        ai_feedback: None,
                                    };
                                    let _ = gs.add_check_in(&ci);
                                }
                                let emoji = mood_emojis.get(mood as usize).unwrap_or(&"");
                                let _ = telegram.send(&chat_id, &format!(
                                    "{} 已記錄心情 {}/5，晚安！",
                                    emoji, mood
                                )).await;
                                return;
                            }
                        }
                    }
                }
            }

            if text == "/goals" {
                if let Some(ref gs) = state.goals_store {
                    let ctx = clawtex_core::goals_push::goals_context(gs).unwrap_or_default();
                    if ctx.is_empty() {
                        let _ = telegram.send(&chat_id, "目前沒有進行中的目標。\n\n告訴我你的目標，我會幫你建立追蹤計畫！").await;
                    } else {
                        // Also include today's task status
                        let briefing = clawtex_core::goals_push::morning_briefing(gs).unwrap_or_default();
                        if briefing.is_empty() {
                            let _ = telegram.send(&chat_id, &ctx).await;
                        } else {
                            let _ = telegram.send(&chat_id, &briefing).await;
                        }
                    }
                } else {
                    let _ = telegram.send(&chat_id, "目標系統尚未初始化").await;
                }
                return;
            }

            if text == "/history" {
                let count = state.conversations.message_count(&chat_id).await;
                let reply = format!("Current conversation: {} messages ({} turns)", count, count / 2);
                let _ = telegram.send(&chat_id, &reply).await;
                return;
            }

            // ── /profile command — view/modify user profile ────────────
            if text == "/profile" || text.starts_with("/profile ") {
                if text.starts_with("/profile set ") {
                    // /profile set <field> <value>
                    let parts: Vec<&str> = text.splitn(4, ' ').collect();
                    if parts.len() < 4 {
                        let _ = telegram.send(&chat_id, "Usage: /profile set <field> <value>\nFields: timezone, locale, display_name, butler_name, proactivity").await;
                        return;
                    }
                    let field = parts[2];
                    let value = parts[3];

                    // Scope the write lock so it is dropped before any .await
                    let reply = {
                        let mut profile = state.user_profile.write().unwrap_or_else(|p| p.into_inner());
                        let result: Result<String, String> = match field {
                            "timezone" => {
                                if value.parse::<chrono_tz::Tz>().is_err() {
                                    Err("Invalid timezone. Use IANA format, e.g. America/New_York".to_string())
                                } else {
                                    profile.timezone = value.to_string();
                                    Ok(format!("Timezone set to {}", value))
                                }
                            }
                            "locale" => {
                                profile.locale = value.to_string();
                                Ok(format!("Locale set to {}", value))
                            }
                            "display_name" | "name" => {
                                profile.display_name = value.to_string();
                                Ok(format!("Display name set to {}", value))
                            }
                            "butler_name" => {
                                profile.persona.name = value.to_string();
                                Ok(format!("Butler name set to {}", value))
                            }
                            "proactivity" => {
                                match serde_json::from_str::<clawtex_core::user_profile::ProactivityLevel>(
                                    &format!("\"{}\"", value),
                                ) {
                                    Ok(level) => {
                                        profile.persona.proactivity = level;
                                        Ok(format!("Proactivity set to {}", value))
                                    }
                                    Err(_) => Err("Invalid proactivity. Use: passive, moderate, active, autonomous".to_string()),
                                }
                            }
                            _ => Err(format!(
                                "Unknown field: {}. Available: timezone, locale, display_name, butler_name, proactivity",
                                field
                            )),
                        };

                        // On success, persist to SQLite
                        if result.is_ok() {
                            let db_path = format!("{}/.clawtex/core.db", dirs_home());
                            if let Ok(conn) = rusqlite::Connection::open(&db_path) {
                                let _ = profile.save(&conn);
                            }
                        }

                        match result {
                            Ok(msg) => format!("\u{2705} {}", msg),
                            Err(msg) => format!("\u{274c} {}", msg),
                        }
                    }; // write lock dropped here

                    let _ = telegram.send(&chat_id, &reply).await;
                } else {
                    // Display current profile — scope the read lock before .await
                    let reply = {
                        let profile = state.user_profile.read().unwrap_or_else(|p| p.into_inner());
                        let proactivity_str = serde_json::to_string(&profile.persona.proactivity)
                            .unwrap_or_default()
                            .trim_matches('"')
                            .to_string();
                        format!(
                            "\u{1f464} Profile\n\
                             Name: {}\n\
                             Locale: {}\n\
                             Timezone: {}\n\
                             Butler: {} ({})\n\
                             Proactivity: {}\n\n\
                             Use: /profile set <field> <value>\n\
                             Fields: timezone, locale, display_name, butler_name, proactivity",
                            profile.display_name,
                            profile.locale,
                            profile.timezone,
                            profile.persona.name,
                            profile.persona.style,
                            proactivity_str,
                        )
                    }; // read lock dropped here

                    let _ = telegram.send(&chat_id, &reply).await;
                }
                return;
            }

            // ── /alerts command — manage event trigger alerts ──────────
            if text == "/alerts" || text.starts_with("/alerts ") {
                if let Some(ref trigger_mgr) = state.trigger_manager {
                    let parts: Vec<&str> = text.split_whitespace().collect();

                    if parts.len() >= 3 && (parts[1] == "enable" || parts[1] == "disable") {
                        let trigger_id = parts[2].to_string();
                        let enable = parts[1] == "enable";

                        // Scope the mutex guard so it is dropped before .await
                        let reply = {
                            let mut mgr = trigger_mgr.lock().unwrap_or_else(|p| p.into_inner());
                            if let Some(trigger) = mgr.triggers.iter_mut().find(|t| t.id == trigger_id) {
                                trigger.enabled = enable;
                                // Persist to SQLite
                                let db_path = format!("{}/.clawtex/core.db", dirs_home());
                                if let Ok(conn) = rusqlite::Connection::open(&db_path) {
                                    let _ = conn.execute(
                                        "UPDATE event_triggers SET enabled = ?1 WHERE id = ?2",
                                        rusqlite::params![enable as i32, &trigger_id],
                                    );
                                }
                                let status = if enable { "enabled" } else { "disabled" };
                                format!(
                                    "{} Trigger '{}' {}",
                                    if enable { "\u{2705}" } else { "\u{274c}" },
                                    trigger_id,
                                    status
                                )
                            } else {
                                format!("Unknown trigger ID: {}", trigger_id)
                            }
                        }; // mutex guard dropped here

                        let _ = telegram.send(&chat_id, &reply).await;
                    } else {
                        // List all triggers — scope the guard before .await
                        let reply = {
                            let mgr = trigger_mgr.lock().unwrap_or_else(|p| p.into_inner());
                            let mut out = "\u{1f514} Event Triggers:\n\n".to_string();
                            for trigger in &mgr.triggers {
                                let status = if trigger.enabled { "\u{2705}" } else { "\u{274c}" };
                                let last = trigger.last_fired
                                    .map(|t| t.format("%m-%d %H:%M").to_string())
                                    .unwrap_or_else(|| "never".to_string());
                                out.push_str(&format!(
                                    "{} {} \u{2014} cooldown: {}s, last fired: {}\n",
                                    status, trigger.id, trigger.cooldown_secs, last
                                ));
                            }
                            if mgr.triggers.is_empty() {
                                out.push_str("No triggers configured.\n");
                            }
                            out.push_str("\nUse /alerts enable|disable <id> to toggle.");
                            out
                        }; // mutex guard dropped here

                        let _ = telegram.send(&chat_id, &reply).await;
                    }
                } else {
                    let _ = telegram.send(&chat_id, "Event triggers not configured.").await;
                }
                return;
            }

            if text == "/help" || text == "/start" {
                let reply = "\
Clawtex Bot Commands:

/help — Show this help
/lang — List available languages
/lang <locale> — Switch bot language (en, zh-TW, zh-CN, ja, ko)
/goals — View active goals and today's tasks
/status — System status (uptime, LLM, tasks)
/tools — List available tools
/hands — List available workflow hands
/hand <name> <input> — Run a hand workflow
/product <idea> — Full SaaS pipeline (spec → code → deploy → Stripe)
/sot <topic> — Parallel content generation (Skeleton-of-Thought)
/cron list — List scheduled jobs
/cron add <schedule> <action> — Schedule a job
/cron remove <id> — Remove a scheduled job
/costs — Token usage and cost summary
/revenue — Revenue tracking summary
/setup — API key setup status (Stripe, Render)
/setup stripe <key> — Set Stripe secret key
/setup render <key> — Set Render API key
/pipeline — Show full pipeline readiness
/crm — Outreach pipeline status
/dashboard — Open web dashboard
/profile — View your profile
/profile set <field> <value> — Update profile field
/alerts — List event trigger alerts
/alerts enable|disable <id> — Toggle an alert trigger
/history — Conversation message count
/clear — Clear conversation memory
/estop — Emergency stop all agents
/resume — Resume after e-stop

Any other message will be processed by the AI agent.";
                // Prepend localized welcome greeting for /start
                if text == "/start" {
                    let chat_id_num = chat_id.parse::<i64>().unwrap_or(0);
                    let ti = state.telegram_i18n.read().await;
                    let welcome = ti.translate(chat_id_num, "welcome");
                    let full_reply = format!("{}\n\n{}", welcome, reply);
                    let _ = telegram.send(&chat_id, &full_reply).await;
                } else {
                    let _ = telegram.send(&chat_id, reply).await;
                }
                return;
            }

            // ── /lang command — per-chat locale switching ──
            match parse_lang_command(&text) {
                LangCommand::Switch(locale) => {
                    let chat_id_num = chat_id.parse::<i64>().unwrap_or(0);
                    let mut ti = state.telegram_i18n.write().await;
                    match ti.set_locale(chat_id_num, &locale) {
                        Ok(()) => {
                            let welcome = ti.translate(chat_id_num, "welcome");
                            let _ = telegram.send(&chat_id, &format!(
                                "Language set to '{}'. {}", locale, welcome
                            )).await;
                        }
                        Err(e) => {
                            let _ = telegram.send(&chat_id, &e).await;
                        }
                    }
                    return;
                }
                LangCommand::List => {
                    let locales = supported_locales();
                    let chat_id_num = chat_id.parse::<i64>().unwrap_or(0);
                    let ti = state.telegram_i18n.read().await;
                    let current = ti.get_locale(chat_id_num);
                    let reply = format!(
                        "Available languages: {}\nCurrent: {}\n\nUsage: /lang <locale>",
                        locales.join(", "), current
                    );
                    let _ = telegram.send(&chat_id, &reply).await;
                    return;
                }
                LangCommand::NotACommand => { /* fall through to other commands */ }
            }

            // ── /setup command — API key configuration ──
            if text == "/setup" || text.starts_with("/setup ") {
                let args = text.strip_prefix("/setup").unwrap_or("").trim();
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_else(|_| ".".to_string());
                let config_path = format!("{}/.clawtex/agents.toml", home);

                if args.is_empty() {
                    // Show setup status
                    let has_stripe = state.tool_registry.names().contains(&"stripe".to_string());
                    let has_render = state.tool_registry.names().contains(&"render_deploy".to_string());
                    let reply = format!(
                        "🔧 Setup Status:\n\n\
                        Stripe: {}\n\
                        Render: {}\n\n\
                        To configure:\n\
                        /setup stripe sk_test_... (from dashboard.stripe.com/test/apikeys)\n\
                        /setup render rnd_... (from dashboard.render.com/settings#api-keys)\n\n\
                        Both have free tiers. After setting keys, restart the daemon to activate tools.",
                        if has_stripe { "✅ Active" } else { "❌ Not configured" },
                        if has_render { "✅ Active" } else { "❌ Not configured" },
                    );
                    let _ = telegram.send(&chat_id, &reply).await;
                    return;
                }

                // Parse: /setup stripe <key> or /setup render <key>
                let parts: Vec<&str> = args.splitn(2, ' ').collect();
                if parts.len() != 2 || parts[1].is_empty() {
                    let _ = telegram.send(&chat_id, "Usage: /setup stripe <key> or /setup render <key>").await;
                    return;
                }

                let (service, key) = (parts[0], parts[1].trim());
                let (toml_section, toml_key) = match service {
                    "stripe" => ("stripe", "secret_key"),
                    "render" => ("render", "api_key"),
                    _ => {
                        let _ = telegram.send(&chat_id, "Unknown service. Use: /setup stripe <key> or /setup render <key>").await;
                        return;
                    }
                };

                // Update agents.toml
                match std::fs::read_to_string(&config_path) {
                    Ok(content) => {
                        // Replace the empty key with the new value
                        let old_pattern = format!("[{}]\n{} = \"\"", toml_section, toml_key);
                        let new_pattern = format!("[{}]\n{} = \"{}\"", toml_section, toml_key, key);
                        let updated = if content.contains(&old_pattern) {
                            content.replace(&old_pattern, &new_pattern)
                        } else {
                            // Section exists but key has a value — replace it
                            let re_pattern = format!(r#"(?m)(\[{}\]\n{} = )"[^"]*""#, toml_section, toml_key);
                            if let Ok(re) = regex::Regex::new(&re_pattern) {
                                re.replace(&content, format!("${{1}}\"{}\"", key).as_str()).to_string()
                            } else {
                                content
                            }
                        };
                        match std::fs::write(&config_path, &updated) {
                            Ok(_) => {
                                let _ = telegram.send(&chat_id, &format!(
                                    "✅ {} key saved to agents.toml.\n\nRestart the daemon to activate the {} tool:\nkill + cargo run --release",
                                    service, service
                                )).await;
                            }
                            Err(e) => {
                                let _ = telegram.send(&chat_id, &format!("❌ Failed to write config: {}", e)).await;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = telegram.send(&chat_id, &format!("❌ Failed to read config: {}", e)).await;
                    }
                }
                return;
            }

            // ── /pipeline — Show full pipeline readiness ──
            if text == "/pipeline" {
                let tools = state.tool_registry.names();
                let has_stripe = tools.contains(&"stripe".to_string());
                let has_render = tools.contains(&"render_deploy".to_string());
                let has_sot = tools.contains(&"skeleton_generate".to_string());
                let llm_ok = state.llm_router.route("ping", "auto").await.is_ok();

                let hands_list = state.hands.list();
                let hand_names: Vec<&str> = hands_list.iter().map(|h| h.name.as_str()).collect();

                let check = |ok: bool| if ok { "✅" } else { "❌" };

                let reply = format!(
                    "📊 Pipeline Readiness\n\n\
                    === Core ===\n\
                    {} LLM Provider (LM Studio / Ollama)\n\
                    {} Tool Registry ({} tools)\n\
                    {} Hand Registry ({} hands)\n\n\
                    === Market Research ===\n\
                    {} market_intel hand\n\
                    {} researcher hand\n\
                    {} web_search tool\n\n\
                    === Product Development ===\n\
                    {} product_spec hand (market → spec)\n\
                    {} code_gen hand (spec → code)\n\
                    {} ai_code tool\n\n\
                    === Deployment ===\n\
                    {} Stripe ({}) \n\
                    {} Render ({})\n\
                    {} saas_deploy hand\n\n\
                    === Content & Outreach ===\n\
                    {} skeleton_generate (SoT parallel)\n\
                    {} seo_content hand\n\
                    {} blog_publish tool\n\
                    {} twitter tool\n\
                    {} email tool\n\
                    {} outreach hand\n\n\
                    === Revenue Tracking ===\n\
                    {} cost_tracker\n\
                    {} revenue_tracker\n\n\
                    Pipeline command: /product <idea>\n\
                    Missing keys? /setup to configure",
                    check(llm_ok),
                    check(tools.len() > 10), tools.len(),
                    check(hands_list.len() >= 13), hands_list.len(),
                    check(hand_names.contains(&"market_intel")),
                    check(hand_names.contains(&"researcher")),
                    check(tools.contains(&"web_search".to_string())),
                    check(hand_names.contains(&"product_spec")),
                    check(hand_names.contains(&"code_gen")),
                    check(tools.contains(&"ai_code".to_string())),
                    check(has_stripe), if has_stripe { "configured" } else { "run /setup stripe <key>" },
                    check(has_render), if has_render { "configured" } else { "run /setup render <key>" },
                    check(hand_names.contains(&"saas_deploy")),
                    check(has_sot),
                    check(hand_names.contains(&"seo_content")),
                    check(tools.contains(&"blog_publish".to_string())),
                    check(tools.contains(&"twitter".to_string())),
                    check(tools.contains(&"email".to_string())),
                    check(hand_names.contains(&"outreach")),
                    check(state.cost_tracker.is_some()),
                    check(state.revenue_tracker.is_some()),
                );
                let _ = telegram.send(&chat_id, &reply).await;
                return;
            }

            if text == "/status" {
                let uptime_secs = state.started_at.elapsed().as_secs();
                let hours = uptime_secs / 3600;
                let mins = (uptime_secs % 3600) / 60;
                let task_count = state.task_queue.history(100).await.map(|t| t.len()).unwrap_or(0);
                let conv_count = state.conversations.message_count(&chat_id).await;
                let tools = state.tool_registry.names();
                let agents = state.agent_runtime.list_agents();

                let llm_status = match state.llm_router.route("ping", "auto").await {
                    Ok(_) => "online",
                    Err(_) => "offline",
                };

                // Rate limit stats
                let rl_stats = state.tool_registry.rate_limit_stats();
                let global_calls = rl_stats.get("global").copied().unwrap_or(0);

                let reply = format!(
                    "Clawtex Status\n\n\
                     Uptime: {}h {}m\n\
                     LLM: {}\n\
                     Tools: {} ({})\n\
                     Agents: {} ({})\n\
                     Tasks: {}\n\
                     Your messages: {}\n\
                     Tool calls (1hr): {}",
                    hours, mins,
                    llm_status,
                    tools.len(), tools.join(", "),
                    agents.len(), agents.join(", "),
                    task_count,
                    conv_count,
                    global_calls,
                );
                let _ = telegram.send(&chat_id, &reply).await;
                return;
            }

            if text == "/tools" {
                let specs = state.tool_registry.specs();
                let mut reply = String::from("Available Tools:\n");
                for spec in &specs {
                    reply.push_str(&format!("\n• {} — {}", spec.name, spec.description));
                }
                let _ = telegram.send(&chat_id, &reply).await;
                return;
            }

            if text == "/hands" {
                let hands_list = state.hands.list();
                if hands_list.is_empty() {
                    let _ = telegram.send(&chat_id, "No hands available. Add TOML files to ~/.clawtex/hands/").await;
                } else {
                    let mut reply = String::from("Available Hands:\n");
                    for hand in &hands_list {
                        reply.push_str(&format!(
                            "\n• {} — {} ({} phases)\n  Usage: /hand {} <your request>",
                            hand.name, hand.description, hand.phases.len(), hand.name
                        ));
                    }
                    let _ = telegram.send(&chat_id, &reply).await;
                }
                return;
            }

            // ── /product command — Full SaaS product pipeline ──
            if text.starts_with("/product ") {
                let idea = text[9..].trim();
                if idea.is_empty() {
                    let _ = telegram.send(&chat_id, "Usage: /product <product idea>\nExample: /product AI-powered text summarizer API").await;
                    return;
                }

                // Pre-flight checks
                let tools = state.tool_registry.names();
                let has_stripe = tools.contains(&"stripe".to_string());
                let has_render = tools.contains(&"render_deploy".to_string());
                let llm_ok = state.llm_router.route("ping", "auto").await.is_ok();

                if !llm_ok {
                    let _ = telegram.send(&chat_id, "❌ LLM provider is offline. Start LM Studio or Ollama first.").await;
                    return;
                }

                // Determine which hands to run based on available tools
                let mut pipeline_hands: Vec<&str> = vec!["product_spec", "code_gen"];
                let mut deploy_note = String::new();
                if has_stripe && has_render {
                    pipeline_hands.push("saas_deploy");
                } else {
                    let mut missing = Vec::new();
                    if !has_stripe { missing.push("Stripe (/setup stripe <key>)"); }
                    if !has_render { missing.push("Render (/setup render <key>)"); }
                    deploy_note = format!(
                        "\n⚠️ Skipping deploy phase — missing: {}\nRun /setup to configure, then /hand saas_deploy to deploy later.",
                        missing.join(", ")
                    );
                }

                let pipeline_desc = pipeline_hands.iter()
                    .map(|h| *h)
                    .collect::<Vec<_>>()
                    .join(" → ");

                let _ = telegram.send(&chat_id, &format!(
                    "🚀 Starting SaaS product pipeline:\n\
                     Idea: {}\n\
                     Pipeline: {}\n\
                     Est. time: {}{}\n\n\
                     Progress updates per phase.",
                    &idea[..idea.len().min(100)],
                    pipeline_desc,
                    if pipeline_hands.len() == 3 { "15-30 min" } else { "5-15 min" },
                    deploy_note
                )).await;

                let mut pipeline_input = idea.to_string();
                let mut all_ok = true;
                let mut completed_hands: Vec<String> = Vec::new();

                for hand_name in &pipeline_hands {
                    if let Some(hand) = state.hands.get(*hand_name) {
                        let hand = hand.clone();
                        let total_phases = hand.phases.len();
                        let _ = telegram.send(&chat_id, &format!(
                            "⏳ [{}/{}] Running '{}' ({} phases)...",
                            completed_hands.len() + 1, pipeline_hands.len(),
                            hand_name, total_phases
                        )).await;

                        match HandRunner::run(
                            &hand, &pipeline_input,
                            &state.agent_runtime, &state.llm_router, &state.tool_registry,
                            Some(&state.approval_gate),
                        ).await {
                            Ok(result) => {
                                let preview = if result.final_output.len() > 1500 {
                                    format!("{}...\n[truncated]", &result.final_output[..result.final_output
                                        .char_indices().nth(1500).map(|(i,_)| i).unwrap_or(result.final_output.len())])
                                } else {
                                    result.final_output.clone()
                                };
                                let _ = telegram.send(&chat_id, &format!(
                                    "✅ '{}' done ({}/{} phases, {:.1}s)\n\n{}",
                                    hand_name, result.phases_completed, result.total_phases,
                                    result.elapsed_secs, preview
                                )).await;
                                completed_hands.push(hand_name.to_string());
                                // Pass output to next hand
                                pipeline_input = format!(
                                    "Previous hand '{}' output:\n\n{}\n\nOriginal idea: {}",
                                    hand_name, result.final_output, idea
                                );
                            }
                            Err(e) => {
                                let _ = telegram.send(&chat_id, &format!(
                                    "❌ '{}' failed: {}\n\nCompleted: {}\nUse /hand {} <input> to retry.",
                                    hand_name, e,
                                    if completed_hands.is_empty() { "none".into() } else { completed_hands.join(", ") },
                                    hand_name
                                )).await;
                                all_ok = false;
                                break;
                            }
                        }
                    } else {
                        let _ = telegram.send(&chat_id, &format!(
                            "⚠️ Hand '{}' not found. Check ~/.clawtex/hands/", hand_name
                        )).await;
                        all_ok = false;
                        break;
                    }
                }

                if all_ok {
                    let summary = if pipeline_hands.len() == 3 {
                        format!(
                            "🎉 SaaS pipeline complete!\n\
                             Product: {}\n\n\
                             Output files in workspace:\n\
                             📋 openapi.yaml — API specification\n\
                             💰 pricing.json — Pricing strategy\n\
                             🏗️ architecture.md — Tech architecture\n\
                             📦 Project code — Ready to deploy\n\
                             🌐 Live on Render + Stripe payment\n\n\
                             /revenue to track income",
                            idea
                        )
                    } else {
                        format!(
                            "✅ Product spec + code generation complete!\n\
                             Product: {}\n\n\
                             Output files in workspace:\n\
                             📋 openapi.yaml — API specification\n\
                             💰 pricing.json — Pricing strategy\n\
                             🏗️ architecture.md — Tech architecture\n\
                             📦 Project code — Ready to deploy\n\n\
                             Next: /setup stripe + /setup render, then:\n\
                             /hand saas_deploy \"deploy the generated project\"",
                            idea
                        )
                    };
                    let _ = telegram.send(&chat_id, &summary).await;
                }
                return;
            }

            // ── /sot command — Skeleton-of-Thought parallel generation ──
            if text.starts_with("/sot ") {
                let topic = text[5..].trim();
                if topic.is_empty() {
                    let _ = telegram.send(&chat_id, "Usage: /sot <topic>\nExample: /sot Write a guide to Rust async programming").await;
                    return;
                }

                // Check which providers are alive
                let config = clawtex_core::SkeletonConfig::default();
                let mut alive_list = Vec::new();
                for p in &config.expansion_providers {
                    if state.llm_router.has_provider(p) && state.llm_router.is_alive(p).await {
                        alive_list.push(p.as_str());
                    }
                }
                let alive_str = if alive_list.is_empty() { "auto".to_string() } else { alive_list.join(", ") };

                let _ = telegram.send(&chat_id, &format!(
                    "SoT: Starting parallel generation\n\
                     Topic: {}\n\
                     Alive providers: {}\n\
                     Step 1: Generating skeleton outline...",
                    &topic[..topic.len().min(80)], alive_str
                )).await;

                let runner = clawtex_core::SkeletonRunner::new(
                    state.llm_router.clone(), config,
                );

                match runner.generate(topic).await {
                    Ok(result) => {
                        let summary = format!(
                            "SoT complete: {}/{} sections\n\
                             Providers: {}\n\n{}",
                            result.successful_sections,
                            result.total_sections,
                            result.providers_used.join(", "),
                            if result.merged_output.len() > 3800 {
                                format!("{}...\n[truncated — full output: {} chars]",
                                    &result.merged_output[..result.merged_output
                                        .char_indices().nth(3800).map(|(i,_)| i)
                                        .unwrap_or(result.merged_output.len())],
                                    result.merged_output.len())
                            } else {
                                result.merged_output
                            }
                        );
                        let _ = telegram.send(&chat_id, &summary).await;
                    }
                    Err(e) => {
                        let _ = telegram.send(&chat_id, &format!("SoT failed: {}", e)).await;
                    }
                }
                return;
            }

            if text.starts_with("/hand ") {
                let parts: Vec<&str> = text[6..].splitn(2, ' ').collect();
                let hand_name = parts[0];
                let user_input = if parts.len() > 1 { parts[1] } else { "" };

                if let Some(hand) = state.hands.get(hand_name) {
                    let hand = hand.clone();
                    let total_phases = hand.phases.len();
                    let phase_names: Vec<String> = hand.phases.iter().map(|p| p.name.clone()).collect();
                    let starting_msg = {
                        let chat_id_num = chat_id.parse::<i64>().unwrap_or(0);
                        let ti = state.telegram_i18n.read().await;
                        ti.translate(chat_id_num, "hand.starting")
                    };
                    let _ = telegram.send(&chat_id, &format!(
                        "{} '{}' ({} phases)\nPhases: {}",
                        starting_msg, hand.name, total_phases, phase_names.join(" → ")
                    )).await;

                    // Run phases one by one with progress reporting
                    let start = std::time::Instant::now();
                    let mut outputs: Vec<PhaseOutput> = Vec::new();
                    let mut context = HandRunner::prepare_context(&hand, user_input);
                    let mut all_ok = true;

                    for i in 0..total_phases {
                        let phase_name = &hand.phases[i].name;
                        let _ = telegram.send(&chat_id, &format!(
                            "⏳ Phase {}/{}: {} ...",
                            i + 1, total_phases, phase_name
                        )).await;

                        match HandRunner::run_single_phase(
                            &hand, i, user_input, &context, &outputs,
                            &state.agent_runtime, &state.llm_router, &state.tool_registry,
                        ).await {
                            Ok((output, new_context)) => {
                                let preview = if output.output.len() > 300 {
                                    format!("{}...", &output.output[..output.output
                                        .char_indices().nth(300).map(|(idx,_)| idx).unwrap_or(output.output.len())])
                                } else {
                                    output.output.clone()
                                };
                                let phase_done_msg = {
                                    let chat_id_num = chat_id.parse::<i64>().unwrap_or(0);
                                    let ti = state.telegram_i18n.read().await;
                                    ti.translate(chat_id_num, "hand.phase_complete")
                                };
                                let _ = telegram.send(&chat_id, &format!(
                                    "Phase {}/{}: {} — {} ({} tool calls)\n\n{}",
                                    i + 1, total_phases, phase_name, phase_done_msg, output.tool_calls, preview
                                )).await;
                                outputs.push(output);
                                context = new_context;
                            }
                            Err(e) => {
                                let _ = telegram.send(&chat_id, &format!(
                                    "❌ Phase {}/{}: {} failed: {}",
                                    i + 1, total_phases, phase_name, e
                                )).await;
                                all_ok = false;
                                break;
                            }
                        }
                    }

                    let elapsed = start.elapsed().as_secs_f64();
                    let final_output = outputs.last()
                        .map(|o| o.output.clone())
                        .unwrap_or_else(|| "No output".to_string());

                    let complete_msg = {
                        let chat_id_num = chat_id.parse::<i64>().unwrap_or(0);
                        let ti = state.telegram_i18n.read().await;
                        ti.translate(chat_id_num, "hand.complete")
                    };
                    let status = if all_ok { &complete_msg } else { "partially completed" };
                    let summary = format!(
                        "Hand '{}' — {} ({}/{} phases, {:.1}s)\n\n{}",
                        hand.name, status,
                        outputs.len(), total_phases, elapsed,
                        if final_output.len() > 3500 {
                            format!("{}...\n[truncated]", &final_output[..final_output
                                .char_indices().nth(3500).map(|(idx,_)| idx).unwrap_or(final_output.len())])
                        } else {
                            final_output.clone()
                        }
                    );
                    let _ = telegram.send(&chat_id, &summary).await;

                    // Hand chaining: if chain_to is set, auto-start the next hand
                    if all_ok {
                        if let Some(ref next_hand_name) = hand.chain_to {
                            if let Some(next_hand) = state.hands.get(next_hand_name) {
                                let next_hand = next_hand.clone();
                                let chain_input = format!(
                                    "Previous hand '{}' output:\n\n{}\n\nOriginal request: {}",
                                    hand.name, final_output, user_input
                                );
                                let _ = telegram.send(&chat_id, &format!(
                                    "🔗 Chaining to hand '{}' ({} phases)...",
                                    next_hand.name, next_hand.phases.len()
                                )).await;

                                // Run chained hand
                                match HandRunner::run(
                                    &next_hand, &chain_input,
                                    &state.agent_runtime, &state.llm_router, &state.tool_registry,
                                    Some(&state.approval_gate),
                                ).await {
                                    Ok(chain_result) => {
                                        let chain_summary = format!(
                                            "🔗 Chained hand '{}' completed ({}/{} phases, {:.1}s)\n\n{}",
                                            chain_result.hand_name,
                                            chain_result.phases_completed,
                                            chain_result.total_phases,
                                            chain_result.elapsed_secs,
                                            if chain_result.final_output.len() > 3000 {
                                                format!("{}...\n[truncated]",
                                                    &chain_result.final_output[..chain_result.final_output
                                                        .char_indices().nth(3000).map(|(idx,_)| idx)
                                                        .unwrap_or(chain_result.final_output.len())])
                                            } else {
                                                chain_result.final_output
                                            }
                                        );
                                        let _ = telegram.send(&chat_id, &chain_summary).await;
                                    }
                                    Err(e) => {
                                        let _ = telegram.send(&chat_id, &format!(
                                            "🔗 Chained hand '{}' failed: {}", next_hand_name, e
                                        )).await;
                                    }
                                }
                            } else {
                                let _ = telegram.send(&chat_id, &format!(
                                    "⚠️ chain_to '{}' not found in registry", next_hand_name
                                )).await;
                            }
                        }
                    }
                } else {
                    let available = state.hands.names().join(", ");
                    let _ = telegram.send(&chat_id, &format!(
                        "Unknown hand '{}'. Available: {}\nUsage: /hand <name> <your request>",
                        hand_name, if available.is_empty() { "(none)".to_string() } else { available }
                    )).await;
                }
                return;
            }

            // Handle approval responses (supports Single and Multi tier)
            if text.starts_with("/approve ") {
                let id = &text[9..];
                // Check multi-approval status before responding (to show progress)
                let was_multi = state.approval_gate.multi_approval_status(id).await;
                if state.approval_gate.respond(id, true).await {
                    if let Some((prev_approvals, required)) = was_multi {
                        let new_count = prev_approvals + 1;
                        if new_count >= required {
                            let _ = telegram.send(&chat_id, &format!(
                                "Approved ({}/{}). Quorum reached.", new_count, required
                            )).await;
                        } else {
                            let _ = telegram.send(&chat_id, &format!(
                                "Vote recorded ({}/{}). Waiting for more approvals...", new_count, required
                            )).await;
                        }
                    } else {
                        let _ = telegram.send(&chat_id, "Approved.").await;
                    }
                } else {
                    let _ = telegram.send(&chat_id, "No pending approval with that ID.").await;
                }
                return;
            }
            if text.starts_with("/deny ") {
                let id = &text[6..];
                if state.approval_gate.respond(id, false).await {
                    let _ = telegram.send(&chat_id, "Denied.").await;
                } else {
                    let _ = telegram.send(&chat_id, "No pending approval with that ID.").await;
                }
                return;
            }

            // ── /cron commands ──────────────────────────────────────
            if text == "/cron" || text == "/cron list" {
                if let Some(ref sched) = state.scheduler {
                    let jobs = sched.list_jobs().await;
                    if jobs.is_empty() {
                        let _ = telegram.send(&chat_id, "No scheduled jobs.").await;
                    } else {
                        let mut reply = format!("Scheduled Jobs ({}):\n", jobs.len());
                        for job in &jobs {
                            let action_desc = match &job.action {
                                clawtex_core::JobAction::Shell { command } => format!("shell: {}", &command[..command.len().min(40)]),
                                clawtex_core::JobAction::Agent { agent, prompt } => format!("agent:{} \"{}\"", agent, &prompt[..prompt.len().min(30)]),
                                clawtex_core::JobAction::Hand { hand_name, input } => format!("hand:{} \"{}\"", hand_name, &input[..input.len().min(30)]),
                                clawtex_core::JobAction::Notify { chat_id, message } => format!("notify:{} \"{}\"", chat_id, &message[..message.len().min(30)]),
                            };
                            let sched_desc = match &job.schedule {
                                clawtex_core::Schedule::Cron { expr } => format!("cron:{}", expr),
                                clawtex_core::Schedule::At { at } => format!("at:{}", at.format("%Y-%m-%d %H:%M")),
                                clawtex_core::Schedule::Every { interval_secs } => format!("every:{}s", interval_secs),
                            };
                            reply.push_str(&format!(
                                "\n• {} [{}]\n  {} | {} | runs:{}\n  id: {}",
                                job.name, sched_desc, action_desc,
                                format!("{:?}", job.status).to_lowercase(),
                                job.run_count, &job.id[..8]
                            ));
                        }
                        let _ = telegram.send(&chat_id, &reply).await;
                    }
                } else {
                    let _ = telegram.send(&chat_id, "Scheduler not available.").await;
                }
                return;
            }

            if text.starts_with("/cron add ") {
                // Format: /cron add "0 9 * * *" hand:freelancer "AI automation jobs"
                // Or:     /cron add every:3600 shell "echo hello"
                // Or:     /cron add "*/30 * * * *" agent:master "check tasks"
                let rest = text.strip_prefix("/cron add ").unwrap().trim();
                let parsed = parse_cron_add_command(rest);
                match parsed {
                    Some((schedule, action, name)) => {
                        if let Some(ref sched) = state.scheduler {
                            match sched.add_job(&name, schedule, action, None).await {
                                Ok(id) => {
                                    let _ = telegram.send(&chat_id, &format!("Job '{}' created (id: {})", name, &id[..8])).await;
                                }
                                Err(e) => {
                                    let _ = telegram.send(&chat_id, &format!("Failed to create job: {}", e)).await;
                                }
                            }
                        } else {
                            let _ = telegram.send(&chat_id, "Scheduler not available.").await;
                        }
                    }
                    None => {
                        let _ = telegram.send(&chat_id, "Usage: /cron add <schedule> <action> [name]\n\nSchedule: \"0 9 * * *\" | every:3600\nAction: hand:<name> \"input\" | agent:<name> \"prompt\" | shell \"command\"").await;
                    }
                }
                return;
            }

            if text.starts_with("/cron remove ") || text.starts_with("/cron delete ") {
                let id_prefix = text.split_whitespace().nth(2).unwrap_or("");
                if let Some(ref sched) = state.scheduler {
                    // Find job by id prefix
                    let jobs = sched.list_jobs().await;
                    let matching: Vec<_> = jobs.iter().filter(|j| j.id.starts_with(id_prefix)).collect();
                    match matching.len() {
                        0 => { let _ = telegram.send(&chat_id, &format!("No job matching id '{}'", id_prefix)).await; }
                        1 => {
                            let job_id = matching[0].id.clone();
                            let job_name = matching[0].name.clone();
                            match sched.delete_job(&job_id).await {
                                Ok(true) => { let _ = telegram.send(&chat_id, &format!("Deleted job '{}' ({})", job_name, &job_id[..8])).await; }
                                Ok(false) => { let _ = telegram.send(&chat_id, "Job not found.").await; }
                                Err(e) => { let _ = telegram.send(&chat_id, &format!("Error: {}", e)).await; }
                            }
                        }
                        n => { let _ = telegram.send(&chat_id, &format!("{} jobs match '{}', be more specific.", n, id_prefix)).await; }
                    }
                } else {
                    let _ = telegram.send(&chat_id, "Scheduler not available.").await;
                }
                return;
            }

            // ── /costs command ──────────────────────────────────────
            if text == "/costs" {
                if let Some(ref ct) = state.cost_tracker {
                    let mut reply = String::from("Cost Summary:\n");
                    match ct.today_total() {
                        Ok(today) => {
                            reply.push_str(&format!(
                                "\nToday: {} tokens, ${:.4} total (${:.4} API + ${:.4} hardware), {} calls",
                                today.total_tokens,
                                today.total_cost_usd,
                                today.api_cost_usd,
                                today.hardware_cost_usd,
                                today.call_count
                            ));
                        }
                        Err(e) => reply.push_str(&format!("\nToday: error — {}", e)),
                    }
                    if let Ok(by_prov) = ct.by_provider(7) {
                        if !by_prov.is_empty() {
                            reply.push_str("\n\n7-Day by Provider:");
                            for s in &by_prov {
                                reply.push_str(&format!("\n  {} — {}tok, ${:.4}, {}calls", s.group, s.total_tokens, s.total_cost_usd, s.call_count));
                            }
                        }
                    }
                    if let Ok(by_day) = ct.by_day(7) {
                        if !by_day.is_empty() {
                            reply.push_str("\n\nLast 7 Days:");
                            for s in &by_day {
                                reply.push_str(&format!("\n  {} — {}tok, ${:.4}", s.group, s.total_tokens, s.total_cost_usd));
                            }
                        }
                    }
                    let _ = telegram.send(&chat_id, &reply).await;
                } else {
                    let _ = telegram.send(&chat_id, "Cost tracker not available.").await;
                }
                return;
            }

            // ── /revenue command ────────────────────────────────────
            if text == "/revenue" {
                if let Some(ref rt) = state.revenue_tracker {
                    let mut reply = String::from("Revenue Summary:\n");
                    match rt.today_total() {
                        Ok(today) => {
                            reply.push_str(&format!("\nToday: ${:.2} ({} transactions)", today.total_usd, today.count));
                        }
                        Err(e) => reply.push_str(&format!("\nToday: error — {}", e)),
                    }
                    if let Ok(by_route) = rt.by_route(30) {
                        if !by_route.is_empty() {
                            reply.push_str("\n\n30-Day by Route:");
                            for s in &by_route {
                                reply.push_str(&format!("\n  {} — ${:.2} ({}x)", s.group, s.total_usd, s.count));
                            }
                        }
                    }
                    if let Ok(by_source) = rt.by_source(30) {
                        if !by_source.is_empty() {
                            reply.push_str("\n\n30-Day by Source:");
                            for s in &by_source {
                                reply.push_str(&format!("\n  {} — ${:.2} ({}x)", s.group, s.total_usd, s.count));
                            }
                        }
                    }
                    if let Ok(by_day) = rt.by_day(7) {
                        if !by_day.is_empty() {
                            reply.push_str("\n\nLast 7 Days:");
                            for s in &by_day {
                                reply.push_str(&format!("\n  {} — ${:.2}", s.group, s.total_usd));
                            }
                        }
                    }
                    let _ = telegram.send(&chat_id, &reply).await;
                } else {
                    let _ = telegram.send(&chat_id, "Revenue tracker not available.").await;
                }
                return;
            }

            // ── /crm command — outreach pipeline from memory ─────
            if text == "/crm" {
                if let Some(ref ms) = state.memory_store {
                    // Search for all outreach_ keys in memory via keyword recall
                    match ms.recall("outreach_", 50, None).await {
                        Ok(entries) => {
                            let crm_entries: Vec<_> = entries.iter().filter(|e| e.key.starts_with("outreach_")).collect();
                            if crm_entries.is_empty() {
                                let _ = telegram.send(&chat_id, "No outreach records found. Run the outreach hand first.").await;
                            } else {
                                let mut reply = format!("CRM Pipeline ({} contacts):\n", crm_entries.len());
                                for entry in &crm_entries {
                                    let val_preview = if entry.content.len() > 100 { &entry.content[..100] } else { &entry.content };
                                    reply.push_str(&format!("\n• {} → {}", entry.key, val_preview));
                                }
                                let _ = telegram.send(&chat_id, &reply).await;
                            }
                        }
                        Err(e) => {
                            let _ = telegram.send(&chat_id, &format!("CRM error: {}", e)).await;
                        }
                    }
                } else {
                    let _ = telegram.send(&chat_id, "Memory store not available.").await;
                }
                return;
            }

            if text == "/estop" {
                state.estop.stop();
                let _ = telegram.send(&chat_id, "E-STOP ACTIVATED. All agent operations halted.\nUse /resume to deactivate.").await;
                return;
            }

            if text == "/resume" {
                state.estop.reset();
                let _ = telegram.send(&chat_id, "E-Stop deactivated. Normal operation resumed.").await;
                return;
            }

            // Check E-Stop before processing
            if state.estop.is_stopped() {
                let _ = telegram.send(&chat_id, "E-Stop is active. Send /resume to deactivate.").await;
                return;
            }

            if text == "/dashboard" {
                let reply = if let Some(ref url) = state.public_url {
                    format!(
                        "{}/dashboard?token={}",
                        url, state.dashboard_token
                    )
                } else {
                    format!(
                        "http://localhost:7878/dashboard?token={}\n\n(ngrok not detected — use ngrok URL if accessing remotely)",
                        state.dashboard_token
                    )
                };
                let _ = telegram.send(&chat_id, &reply).await;
                return;
            }

            // ── Auto-detect locale from message text ─────────────────
            // Only auto-detect if the chat has no explicit locale override
            // (i.e. the user hasn't used /lang yet for this chat).
            {
                let chat_id_num = chat_id.parse::<i64>().unwrap_or(0);
                let needs_detection = {
                    let ti = state.telegram_i18n.read().await;
                    !ti.has_override(chat_id_num)
                };
                if needs_detection {
                    if let Some(detected) = detect_locale(&text) {
                        let mut ti_w = state.telegram_i18n.write().await;
                        if !ti_w.has_override(chat_id_num) {
                            if ti_w.set_locale(chat_id_num, &detected).is_ok() {
                                debug!("Auto-detected locale '{}' for chat {}", detected, chat_id);
                            }
                        }
                    }
                }
            }

            // Show "typing..." indicator
            let _typing = telegram.keep_typing(chat_id.clone());

            // Record task in TaskQueue (so Dashboard shows it)
            let task_id = match state.task_queue.add(&title, &text).await {
                Ok(id) => Some(id),
                Err(e) => {
                    warn!("Failed to record task: {}", e);
                    None
                }
            };

            // Load conversation history
            let history = state.conversations.get_history(&chat_id).await;
            let history_len = history.len();

            // ── Memory recall ─────────────────────────────────────────
            let memory_ctx = if let Some(ref mem) = state.memory_store {
                match mem.recall(&text, 5, Some(&chat_id)).await {
                    Ok(memories) if !memories.is_empty() => {
                        debug!("Recalled {} memories for chat {}", memories.len(), chat_id);
                        MemoryStore::format_context(&memories)
                    }
                    _ => String::new(),
                }
            } else {
                String::new()
            };

            // ── Skill selection ───────────────────────────────────────
            let selected_skills = state.skill_registry.select_for_prompt(&text, "master", 6000);
            let skills_ctx = SkillRegistry::format_context(&selected_skills);

            // ── User profile context ─────────────────────────────────
            let profile_ctx = {
                let profile = state.user_profile.read().unwrap_or_else(|p| p.into_inner());
                profile.system_prompt_context()
            };

            // ── Goals context injection ──────────────────────────────
            let goals_ctx = if let Some(ref gs) = state.goals_store {
                clawtex_core::goals_push::goals_context(gs).unwrap_or_default()
            } else {
                String::new()
            };

            // Combine extra context
            let extra_context = [profile_ctx.as_str(), memory_ctx.as_str(), skills_ctx.as_str(), goals_ctx.as_str()]
                .iter()
                .filter(|s| !s.is_empty())
                .copied()
                .collect::<Vec<&str>>()
                .join("\n\n");
            let extra = if extra_context.is_empty() { None } else { Some(extra_context.as_str()) };

            // ── Agent run (with progress reporting) ────────────────────
            let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<String>(16);
            let progress_tg = telegram.clone();
            let progress_chat_id = chat_id.clone();
            let progress_task = tokio::spawn(async move {
                while let Some(msg) = progress_rx.recv().await {
                    let _ = progress_tg.send(&progress_chat_id, &msg).await;
                }
            });

            match state
                .agent_runtime
                .run_with_progress("master", &text, &history, &state.llm_router, &state.tool_registry, extra, progress_tx)
                .await
            {
                Ok(result) => {
                    let mut final_output = result.output.clone();
                    let mut eval_info = String::new();

                    // ── Self-correction evaluate ──────────────────────
                    if state.eval_config.enabled {
                        match clawtex_core::evaluate::evaluate(
                            &state.llm_router, &text, &final_output, &state.eval_config,
                        ).await {
                            Ok(eval_result) => {
                                eval_info = format!(", eval {}/5", eval_result.score);
                                if !eval_result.passed {
                                    if let Some(ref feedback) = eval_result.feedback {
                                        // Retry with feedback injected
                                        let retry_prompt = format!(
                                            "{}\n\n[Previous attempt was rated {}/5. Feedback: {}]",
                                            text, eval_result.score, feedback
                                        );
                                        if let Ok(retry) = state.agent_runtime.run(
                                            "master", &retry_prompt, &history,
                                            &state.llm_router, &state.tool_registry, extra,
                                        ).await {
                                            final_output = retry.output;
                                            eval_info = format!(", eval {}/5→retry", eval_result.score);
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                debug!("Eval skipped: {}", e);
                            }
                        }
                    }

                    // Update task status → Done
                    if let Some(ref tid) = task_id {
                        let result_preview = if final_output.len() > 500 {
                            format!("{}...", &final_output[..final_output
                                .char_indices().nth(500).map(|(i,_)| i).unwrap_or(final_output.len())])
                        } else {
                            final_output.clone()
                        };
                        let _ = state.task_queue.set_status(
                            tid,
                            clawtex_core::task_queue::TaskStatus::Done,
                            Some(&result_preview),
                            Some("master"),
                        );
                    }

                    // Store this turn in conversation memory
                    let user_msg = ChatMessage {
                        role: "user".to_string(),
                        content: text.clone(),
                        tool_calls: None,
                        tool_call_id: None,
                    };
                    let assistant_msg = ChatMessage {
                        role: "assistant".to_string(),
                        content: final_output.clone(),
                        tool_calls: None,
                        tool_call_id: None,
                    };
                    state.conversations.append(&chat_id, user_msg, assistant_msg).await;

                    // ── Store in semantic memory ──────────────────────
                    if let Some(ref mem) = state.memory_store {
                        let text_preview: String = text.chars().take(200).collect();
                        let output_preview: String = final_output.chars().take(300).collect();
                        let summary = format!("User: {}\nAssistant: {}",
                            text_preview, output_preview,
                        );
                        let _ = mem.store(
                            &format!("turn_{}", chrono::Utc::now().timestamp()),
                            &summary,
                            MemoryCategory::Conversation,
                            Some(&chat_id),
                        ).await;
                    }

                    // Format response
                    let response = if result.tool_calls_made > 0 || history_len > 0 || !eval_info.is_empty() {
                        let mut footer_parts = Vec::new();
                        if result.tool_calls_made > 0 {
                            footer_parts.push(format!("{} tools", result.tool_calls_made));
                        }
                        if history_len > 0 {
                            footer_parts.push(format!("{} ctx msgs", history_len));
                        }
                        footer_parts.push(format!("{:.1}s{}", result.elapsed_secs, eval_info));
                        format!("{}\n\n[{}]", final_output, footer_parts.join(", "))
                    } else {
                        final_output
                    };

                    if let Err(e) = telegram.send(&chat_id, &response).await {
                        error!("Failed to send response: {}", e);
                    }
                }
                Err(e) => {
                    // Update task status → Failed
                    if let Some(ref tid) = task_id {
                        let _ = state.task_queue.set_status(
                            tid,
                            clawtex_core::task_queue::TaskStatus::Failed,
                            Some(&e.to_string()),
                            None,
                        );
                    }

                    error!("Agent processing failed: {}", e);
                    let _ = telegram
                        .send(&chat_id, &format!("Error: {}", e))
                        .await;
                }
            }
            // Stop progress reporter
            progress_task.abort();
        });
    }
}

// ── Main ───────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let home = dirs_home();

    // Setup logging: console + daily rotating log file
    let log_dir = format!("{}/.clawtex/logs", home);
    let _ = std::fs::create_dir_all(&log_dir);
    let file_appender = tracing_appender::rolling::daily(&log_dir, "clawtex-core.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    use tracing_subscriber::fmt::writer::MakeWriterExt;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("clawtex_core=info".parse()?))
        .with_writer(std::io::stderr.and(non_blocking))
        .init();
    let config_path = args
        .config
        .unwrap_or_else(|| format!("{}/.clawtex/agents.toml", home));
    let db_path = args
        .db
        .unwrap_or_else(|| format!("{}/.clawtex/core.db", home));

    info!("clawtex-core v{} starting", env!("CARGO_PKG_VERSION"));
    info!("config: {}", config_path);
    info!("db:     {}", db_path);

    // Load app config (with automatic enc2: secret decryption)
    let app_config: AppConfig = if std::path::Path::new(&config_path).exists() {
        let content = std::fs::read_to_string(&config_path)?;

        // Decrypt enc2: values in-place before TOML parsing
        let clawtex_dir = format!(
            "{}/.clawtex",
            std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string())
        );
        let content = match SecretManager::new(&clawtex_dir) {
            Ok(sm) => {
                let mut decrypted = content.clone();
                // Find and decrypt enc2: values in the raw TOML string
                while let Some(start) = decrypted.find("enc2:") {
                    // Find the end of the enc2: value (next quote or end of line)
                    let value_start = start;
                    let rest = &decrypted[start..];
                    let end = rest.find('"')
                        .or_else(|| rest.find('\''))
                        .or_else(|| rest.find('\n'))
                        .unwrap_or(rest.len());
                    let enc_value = &decrypted[value_start..value_start + end].to_string();
                    match sm.decrypt(enc_value) {
                        Ok(plain) => {
                            decrypted = format!(
                                "{}{}{}",
                                &decrypted[..value_start],
                                plain,
                                &decrypted[value_start + end..]
                            );
                            debug!("Decrypted config secret (len={})", plain.len());
                        }
                        Err(e) => {
                            warn!("Failed to decrypt config value: {}", e);
                            break;
                        }
                    }
                }
                decrypted
            }
            Err(e) => {
                warn!("SecretManager init failed ({}), config secrets will not be decrypted", e);
                content
            }
        };

        toml::from_str(&content).unwrap_or_default()
    } else {
        warn!("Config not found at {}, using defaults", config_path);
        AppConfig::default()
    };

    // ── Handle early-exit subcommands ──
    let _vault_pw: Option<String>;
    match args.command {
        Some(Command::EncryptSecret { value }) => {
            let secret_dir = format!(
                "{}/.clawtex",
                std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_else(|_| ".".to_string())
            );
            let sm = SecretManager::new(&secret_dir)
                .expect("Failed to initialize SecretManager");
            match sm.encrypt(&value) {
                Ok(encrypted) => {
                    println!("{}", encrypted);
                    return Ok(());
                }
                Err(e) => {
                    eprintln!("Encryption failed: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Some(Command::Worker { hub, name, port, device_type }) => {
        info!("Starting in WORKER mode");
        let security = app_config.security.unwrap_or_default();
        let search_config = app_config.search.unwrap_or_default();
        let tool_registry = Arc::new(ToolRegistry::new_with_search(security, search_config));

        let node_name = name.unwrap_or_else(|| {
            std::env::var("COMPUTERNAME")
                .or_else(|_| std::env::var("HOSTNAME"))
                .unwrap_or_else(|_| format!("worker-{}", port))
        });

        let capabilities = match device_type.as_str() {
            "light" => vec!["web_search".to_string(), "http_request".to_string(), "email_send".to_string()],
            _ => vec!["tools".to_string(), "llm".to_string()],
        };

        // Resolve cluster secret for worker auth (config or CLUSTER_SECRET env var)
        let worker_cluster_secret = {
            let config_val = app_config.cluster.as_ref().and_then(|c| c.cluster_secret.as_deref());
            clawtex_core::cluster_worker::resolve_cluster_secret(config_val)
        };
        if worker_cluster_secret.is_some() {
            info!("Cluster secret found — worker will authenticate with hub");
        }

        let config = WorkerConfig {
            hub_url: hub,
            node_name: node_name.clone(),
            capabilities,
            device_type,
            port,
            cluster_secret: worker_cluster_secret,
        };

        info!("Worker '{}' connecting to hub at {}", node_name, config.hub_url);
        let worker = ClusterWorker::new(config, tool_registry);
        return worker.start_server().await;
        }
        Some(Command::Daemon { vault_password, vault_password_stdin }) => {
            _vault_pw = if vault_password_stdin {
                use std::io::BufRead;
                let stdin = std::io::stdin();
                let mut line = String::new();
                stdin.lock().read_line(&mut line)
                    .map_err(|e| anyhow::anyhow!("Failed to read vault password from stdin: {}", e))?;
                Some(line.trim().to_string())
            } else {
                vault_password
            };
        }
        _ => {
            // None (default to daemon) or other subcommands — no vault password fields available
            _vault_pw = None;
        }
    }

    // Init security config (from TOML or defaults)
    let security = app_config.security.unwrap_or_default();
    info!("workspace: {}", security.workspace_dir);

    // Init components
    let mut llm_router = LlmRouter::new(&config_path)?;

    // Provider rotation engine — rate-limit-aware provider switching
    let provider_names = llm_router.provider_names();
    let rotation_config = clawtex_core::RotationConfig {
        base_cooldown_secs: 60,
        max_cooldown_secs: 600,
        backoff_multiplier: 2.0,
        priority_order: provider_names.clone(),
    };
    let rotation = Arc::new(clawtex_core::ProviderRotation::new(rotation_config));
    llm_router.set_rotation(rotation.clone());
    info!("Provider rotation engine initialized ({} providers)", provider_names.len());

    // --- Plugin Bus (Phase 1: partial migration) ---
    let mut plugin_bus = PluginBus::new();

    // Phase 1: Infrastructure
    plugin_bus.register(1, Arc::new(HealthCheckPlugin::new()))?;

    // Phase 2: Data layer
    let traj_db = format!("{}/.clawtex/trajectories.db", home);
    plugin_bus.register(2, Arc::new(TrajectoryPlugin::new(&traj_db)))?;

    // Phase 4: Engine
    plugin_bus.register(4, Arc::new(CircuitBreakerPlugin::new(
        clawtex_core::circuit_breaker::BreakerConfig::default()
    )))?;

    let clawtex_dir = std::path::PathBuf::from(format!("{}/.clawtex", home));

    // Initialize all plugins — retrieve services from AppContext on success,
    // fall back to manual construction on failure.
    let (circuit_breaker, trajectory_logger): (Arc<ProviderCircuitBreaker>, Option<Arc<TrajectoryLogger>>);

    if let Err(e) = plugin_bus.init_all().await {
        tracing::error!("[PluginBus] Init failed: {} — falling back to manual construction", e);

        // Fallback: construct services manually
        circuit_breaker = Arc::new(ProviderCircuitBreaker::new(BreakerConfig::default()));
        llm_router.set_circuit_breaker(circuit_breaker.clone());
        info!("CircuitBreaker (fallback) attached to LlmRouter");

        trajectory_logger = match TrajectoryLogger::new(
            clawtex_dir.join("trajectories.db").to_str().unwrap_or("trajectories.db"),
        ) {
            Ok(tl) => {
                let tl = Arc::new(tl);
                llm_router.set_trajectory_logger(tl.clone());
                info!("TrajectoryLogger (fallback) initialized and wired to LlmRouter");
                Some(tl)
            }
            Err(e) => {
                warn!("Failed to initialize TrajectoryLogger (fallback): {}", e);
                None
            }
        };
    } else {
        tracing::info!(
            "[PluginBus] {} modules initialized: {:?}",
            plugin_bus.initialized_ids().len(),
            plugin_bus.initialized_ids()
        );

        // 從 AppContext 取得 PluginBus 管理的服務，避免重複建立實例
        let app_context = plugin_bus.context().clone();

        circuit_breaker = app_context.get::<ProviderCircuitBreaker>()
            .unwrap_or_else(|| {
                warn!("CircuitBreaker not found in AppContext, creating fallback");
                Arc::new(ProviderCircuitBreaker::new(BreakerConfig::default()))
            });
        llm_router.set_circuit_breaker(circuit_breaker.clone());
        info!("CircuitBreaker from PluginBus wired to LlmRouter");

        trajectory_logger = app_context.get::<TrajectoryLogger>();
        if let Some(ref tl) = trajectory_logger {
            llm_router.set_trajectory_logger(tl.clone());
            info!("TrajectoryLogger from PluginBus wired to LlmRouter");
        } else {
            warn!("TrajectoryLogger not found in AppContext");
        }
    };

    // Wrap plugin_bus 以便在 graceful shutdown 時使用
    let plugin_bus = Arc::new(tokio::sync::Mutex::new(plugin_bus));

    let llm_router = Arc::new(llm_router);
    let task_queue = Arc::new(TaskQueue::new(&db_path).await?);
    let mut agent_runtime = AgentRuntime::new(&config_path)?;
    let execution_node_id = std::env::var("CLAWTEX_NODE_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "local".to_string());
    agent_runtime.set_execution_node_id(execution_node_id.clone());
    info!("AgentRuntime execution node set to '{}'", execution_node_id);
    if let Some(ref tl) = trajectory_logger {
        agent_runtime.set_trajectory_logger(tl.clone());
        info!("TrajectoryLogger wired to AgentRuntime");
    }
    // Wire cost tracker into agent runtime (initialized below, set before Arc wrap)
    let cost_db_path = format!("{}/.clawtex/costs.db", home);
    let cost_tracker: Option<Arc<CostTracker>> = match CostTracker::new(&cost_db_path) {
        Ok(ct) => {
            let ct = Arc::new(ct);
            agent_runtime.set_cost_tracker(ct.clone());
            info!("Cost tracker initialized and wired to agent runtime");
            Some(ct)
        }
        Err(e) => {
            warn!("Cost tracker failed to init: {}", e);
            None
        }
    };
    let pricing_db_path = format!("{}/.clawtex/pricing.db", home);
    let provider_pricing: Option<Arc<ProviderPricingStore>> = match ProviderPricingStore::new(&pricing_db_path) {
        Ok(store) => {
            let store = Arc::new(store);
            agent_runtime.set_provider_pricing(store.clone());
            info!("Provider pricing initialized and wired to AgentRuntime: {}", pricing_db_path);
            Some(store)
        }
        Err(e) => {
            warn!("Provider pricing failed to init: {}", e);
            None
        }
    };
    let power_db_path = format!("{}/.clawtex/power.db", home);
    let power_economics: Option<Arc<PowerEconomics>> = match PowerEconomics::new(&power_db_path) {
        Ok(pe) => {
            let pe = Arc::new(pe);
            agent_runtime.set_power_economics(pe.clone());
            info!("Power economics wired to AgentRuntime on node '{}'", execution_node_id);
            info!("Power economics initialized: {}", power_db_path);
            Some(pe)
        }
        Err(e) => {
            warn!("Power economics failed to init: {}", e);
            None
        }
    };
    // Revenue tracker
    let revenue_db_path = format!("{}/.clawtex/revenue.db", home);
    let revenue_tracker: Option<Arc<RevenueTracker>> = match RevenueTracker::new(&revenue_db_path) {
        Ok(rt) => {
            info!("Revenue tracker initialized");
            Some(Arc::new(rt))
        }
        Err(e) => {
            warn!("Revenue tracker failed to init: {}", e);
            None
        }
    };
    // Goals store
    let goals_db_path = format!("{}/.clawtex/goals.db", home);
    let goals_store: Option<Arc<clawtex_core::goals::GoalsStore>> = match clawtex_core::goals::GoalsStore::new(&goals_db_path) {
        Ok(gs) => {
            info!("Goals store initialized: {}", goals_db_path);
            Some(Arc::new(gs))
        }
        Err(e) => {
            warn!("Goals store failed to init: {}", e);
            None
        }
    };
    let cluster = Arc::new(ClusterRegistry::new(&db_path).await?);
    // Start CPU monitor for accurate cluster scheduling (even on hub)
    clawtex_core::cluster_worker::start_cpu_monitor();
    // Wire cluster hub into agent runtime for distributed tool dispatch
    let cluster_hub = Arc::new(ClusterHub::new(cluster.clone()));
    agent_runtime.set_cluster_hub(cluster_hub.clone());
    // Wire privacy guard if enabled in config
    if let Some(pc) = app_config.privacy {
        if pc.enabled {
            agent_runtime.set_privacy_guard(PrivacyGuard::new(pc));
            info!("Privacy Guard enabled — provider routing by sensitivity tier");
        }
    }
    // Wire budget breaker for fast-path budget checking (5 minute cooldown)
    let budget_breaker = Arc::new(clawtex_core::BudgetBreaker::new(300));
    agent_runtime.set_budget_breaker(budget_breaker.clone());
    info!("Budget breaker wired (300s cooldown)");

    // Wire injection guard for prompt safety
    let injection_guard = Arc::new(clawtex_core::InjectionGuard::new());
    agent_runtime.set_injection_guard(injection_guard.clone());
    info!("Injection guard wired (8 patterns)");

    // ── Service Tier Manager ─────────────────────────────────────────
    let tier_db_path = format!("{}/.clawtex/tiers.db", home);
    let service_tier: Option<Arc<ServiceTierManager>> = match ServiceTierManager::new(&tier_db_path) {
        Ok(stm) => {
            let stm = Arc::new(stm);
            agent_runtime.set_service_tier(stm.clone());
            info!("Service tier manager initialized (db: {})", tier_db_path);
            Some(stm)
        }
        Err(e) => {
            warn!("Service tier manager failed to init: {}", e);
            None
        }
    };

    // ── Tenant Manager ────────────────────────────────────────────────
    let tenant_db_path = format!("{}/.clawtex/tenants.db", home);
    let tenant_base_dir = format!("{}/.clawtex/tenants", home);
    let tenant_manager: Option<Arc<TenantManager>> = match TenantManager::new(&tenant_db_path, &tenant_base_dir) {
        Ok(tm) => {
            let tm = Arc::new(tm);
            info!("Tenant manager initialized (db: {}, base: {})", tenant_db_path, tenant_base_dir);
            Some(tm)
        }
        Err(e) => {
            warn!("Tenant manager failed to init: {}", e);
            None
        }
    };

    // ── Order Workflow ─────────────────────────────────────────────
    let orders_db_path = format!("{}/.clawtex/orders.db", home);
    let order_workflow: Option<Arc<OrderWorkflow>> = match OrderWorkflow::new(&orders_db_path) {
        Ok(ow) => {
            let ow = Arc::new(ow);
            info!("Order workflow initialized (db: {})", orders_db_path);
            Some(ow)
        }
        Err(e) => {
            warn!("Order workflow failed to init: {}", e);
            None
        }
    };

    let agent_runtime = Arc::new(agent_runtime);
    // Load search API config
    let search_config = app_config.search.unwrap_or_default();
    if !search_config.serper_api_key.is_empty() {
        info!("Search backend: Serper (primary) + Tavily (fallback) + Google News RSS");
    } else {
        info!("Search backend: Google News RSS only (no API keys configured)");
    }

    let security_for_aicode = security.clone();
    let mut tool_registry = ToolRegistry::new_with_search(security, search_config);

    // Register ai_code tool (external AI CLI integration)
    let ai_code_config = app_config.ai_code.unwrap_or_default();
    tool_registry.register(Box::new(clawtex_core::tools::ai_code::AiCodeTool::new(
        ai_code_config,
        security_for_aicode,
    )));

    // Register computer_use tool (GUI automation via Claude Computer Use API)
    if let Some(cu_config) = app_config.computer_use {
        if cu_config.enabled {
            info!("computer_use: enabled (sandbox: {})", cu_config.sandbox);
            tool_registry.register(Box::new(
                clawtex_core::tools::computer_use::ComputerUseTool::new(cu_config),
            ));
        } else {
            info!("computer_use: disabled in config");
        }
    }

    // Register delegate tools (needs Arcs, so done after component init)
    let subagent_tools = Arc::new(ToolRegistry::new(SecurityConfig::default())); // subagent gets base tools only (no delegate to prevent loops)
    tool_registry.register_delegate_tools(
        agent_runtime.clone(),
        llm_router.clone(),
        subagent_tools,
    );

    let conversations = Arc::new(ConversationStore::new(&db_path).await?);

    info!("Conversation memory enabled (max 10 msgs/chat, SQLite persistence)");

    // ── Semantic Memory ─────────────────────────────────────────────
    let memory_config = app_config.memory.unwrap_or_default();
    let memory_db = format!("{}/.clawtex/memory.db", home);
    let memory_store = match MemoryStore::from_config(memory_config.clone(), &memory_db).await {
        Ok(store) => {
            info!("Semantic memory: {} backend enabled", store.backend_name());
            Some(Arc::new(store))
        }
        Err(e) => {
            warn!("Semantic memory disabled: {}", e);
            None
        }
    };

    // Register memory tools (need Arc<MemoryStore>)
    if let Some(ref mem) = memory_store {
        tool_registry.register(Box::new(clawtex_core::tools::memory_tools::MemoryStoreTool::new(mem.clone())));
        tool_registry.register(Box::new(clawtex_core::tools::memory_tools::MemoryRecallTool::new(mem.clone())));
        tool_registry.register(Box::new(clawtex_core::tools::memory_tools::MemoryForgetTool::new(mem.clone())));
        info!("Memory tools registered: memory_store, memory_recall, memory_forget");
    }

    // Register vision tool (uses Gemini/Groq free API for image analysis)
    let gemini_key = std::env::var("GEMINI_API_KEY").ok();
    let groq_key = std::env::var("GROQ_API_KEY").ok();
    if gemini_key.is_some() || groq_key.is_some() {
        tool_registry.register(Box::new(clawtex_core::tools::vision::VisionTool::new(
            gemini_key.clone(), groq_key.clone(),
        )));
        info!("Vision tool registered (Gemini={}, Groq={})",
            if gemini_key.is_some() { "yes" } else { "no" },
            if groq_key.is_some() { "yes" } else { "no" });
    }

    // Register email tool (SMTP send — requires approval gate)
    if let Some(email_config) = app_config.email {
        if !email_config.username.is_empty() {
            tool_registry.register(Box::new(clawtex_core::tools::email::EmailTool::new(email_config)));
            info!("Email tool registered (SMTP configured)");
        } else {
            info!("Email tool: SMTP username not set, skipping");
        }
    }

    // Register email receive tool (IMAP read — uses Python imaplib subprocess)
    {
        let imap_config = app_config.imap.unwrap_or_default();
        tool_registry.register(Box::new(clawtex_core::tools::email_receive::EmailReceiveTool::new(imap_config.clone())));
        if imap_config.is_configured() {
            info!("Email receive tool registered (IMAP configured: {})", imap_config.host);
        } else {
            info!("Email receive tool registered (IMAP config pending — will use env vars or args at runtime)");
        }
    }

    // Register slack tool (Slack Incoming Webhook)
    if let Some(slack_config) = app_config.slack {
        if !slack_config.webhook_url.is_empty() {
            tool_registry.register(Box::new(clawtex_core::tools::slack::SlackTool::new(slack_config)));
            info!("Slack tool registered");
        } else {
            info!("Slack tool: webhook_url not set, skipping");
        }
    }

    // Register discord tool (Discord Webhook)
    if let Some(discord_config) = app_config.discord {
        if !discord_config.webhook_url.is_empty() {
            tool_registry.register(Box::new(clawtex_core::tools::discord::DiscordTool::new(discord_config)));
            info!("Discord tool registered");
        } else {
            info!("Discord tool: webhook_url not set, skipping");
        }
    }

    // Register LINE Notify tool
    if let Some(line_config) = app_config.line {
        if !line_config.notify_token.is_empty() {
            tool_registry.register(Box::new(clawtex_core::tools::line_notify::LineTool::new(line_config)));
            info!("LINE Notify tool registered");
        } else {
            info!("LINE Notify tool: notify_token not set, skipping");
        }
    }

    // Register WhatsApp tool (Business Cloud API)
    if let Some(whatsapp_config) = app_config.whatsapp {
        if !whatsapp_config.phone_number_id.is_empty() {
            tool_registry.register(Box::new(clawtex_core::tools::whatsapp::WhatsAppTool::new(whatsapp_config)));
            info!("WhatsApp tool registered");
        } else {
            info!("WhatsApp tool: phone_number_id not set, skipping");
        }
    }

    // Register twitter tool (tweet posting via API + Playwright browser)
    if let Some(twitter_config) = app_config.twitter {
        if !twitter_config.consumer_key.is_empty() {
            tool_registry.register(Box::new(clawtex_core::tools::twitter::TwitterTool::new(twitter_config)));
            info!("Twitter tool registered (API + browser posting)");
        } else {
            info!("Twitter tool: consumer_key not set, skipping");
        }
    }

    // Register blog_publish tool (MDX + index.ts + git push → Vercel)
    if let Some(blog_config) = app_config.blog {
        if !blog_config.repo_path.is_empty() {
            tool_registry.register(Box::new(clawtex_core::tools::blog_publish::BlogPublishTool::new(blog_config)));
            info!("Blog publish tool registered");
        } else {
            info!("Blog publish tool: repo_path not set, skipping");
        }
    }

    // Register pdf_export tool
    {
        let ws_dir = tool_registry.workspace_dir().to_string();
        tool_registry.register(Box::new(clawtex_core::tools::pdf_export::PdfExportTool::new(&ws_dir)));
        info!("PDF export tool registered");
    }

    // Register skeleton_generate tool (SoT parallel content generation)
    tool_registry.register(Box::new(clawtex_core::tools::skeleton_generate::SkeletonGenerateTool::new(
        llm_router.clone(),
    )));
    info!("Skeleton-of-Thought (SoT) tool registered");

    // Register scaffold_saas tool (SaaS project template scaffolding)
    tool_registry.register(Box::new(clawtex_core::tools::scaffold_saas::ScaffoldSaasTool::new(&home)));
    info!("scaffold_saas tool registered");

    // Register cli_anything tool (CLI-Anything integration for controlling desktop software)
    tool_registry.register(Box::new(clawtex_core::tools::cli_anything::CliAnythingTool::new()));
    info!("cli_anything tool registered");

    // Register utility tools (no external API keys needed)
    tool_registry.register(Box::new(clawtex_core::tools::translate::TranslateTool::new()));
    info!("translate tool registered");
    tool_registry.register(Box::new(clawtex_core::tools::json_transform::JsonTransformTool::new()));
    info!("json_transform tool registered");
    tool_registry.register(Box::new(clawtex_core::tools::csv_parse::CsvParseTool::new()));
    info!("csv_parse tool registered");
    tool_registry.register(Box::new(clawtex_core::tools::summarize::SummarizeTool::new()));
    info!("summarize tool registered");
    tool_registry.register(Box::new(clawtex_core::tools::docx_export::DocxExportTool::new()));
    info!("docx_export tool registered");
    tool_registry.register(Box::new(clawtex_core::tools::xlsx_export::XlsxExportTool::new()));
    info!("xlsx_export tool registered");

    // Register image_generate tool (config-gated: requires gemini_api_key)
    if let Some(img_config) = app_config.image_generate {
        if !img_config.gemini_api_key.is_empty() {
            tool_registry.register(Box::new(clawtex_core::tools::image_generate::ImageGenerateTool::new(
                clawtex_core::tools::image_generate::ImageGenerateConfig {
                    gemini_api_key: img_config.gemini_api_key,
                }
            )));
            info!("image_generate tool registered");
        }
    }

    // Register TTS (text-to-speech) tool — always available (edge-tts is free, elevenlabs needs API key)
    tool_registry.register(Box::new(clawtex_core::tools::tts::TtsTool::new()));
    info!("tts tool registered");

    // Register video_compose tool — always available (requires ffmpeg in PATH)
    tool_registry.register(Box::new(clawtex_core::tools::video_compose::VideoComposeTool::new()));
    info!("video_compose tool registered");

    // Register youtube_upload tool — always available (requires YOUTUBE_OAUTH_TOKEN or YOUTUBE_API_KEY)
    tool_registry.register(Box::new(clawtex_core::tools::youtube_upload::YouTubeUploadTool::new()));
    info!("youtube_upload tool registered");

    // Register stripe tool (payment integration) — config first, env var fallback
    let stripe_key = app_config.stripe.as_ref()
        .map(|c| c.secret_key.clone())
        .filter(|k| !k.is_empty())
        .or_else(|| std::env::var("STRIPE_SECRET_KEY").ok().filter(|k| !k.is_empty()));
    if let Some(key) = stripe_key {
        tool_registry.register(Box::new(clawtex_core::tools::stripe::StripeTool::new(key)));
        info!("Stripe tool registered");
    } else {
        info!("Stripe tool: no key in [stripe] config or STRIPE_SECRET_KEY env, skipping");
    }

    // Register render_deploy tool (cloud deployment) — config first, env var fallback
    let render_key = app_config.render.as_ref()
        .map(|c| c.api_key.clone())
        .filter(|k| !k.is_empty())
        .or_else(|| std::env::var("RENDER_API_KEY").ok().filter(|k| !k.is_empty()));
    if let Some(key) = render_key {
        tool_registry.register(Box::new(clawtex_core::tools::render_deploy::RenderDeployTool::new(key)));
        info!("Render deploy tool registered");
    } else {
        info!("Render deploy tool: no key in [render] config or RENDER_API_KEY env, skipping");
    }

    // ── Hands Registry (early, needed for run_hand tool) ───────────
    let hands_dir = format!("{}/.clawtex/hands", home);
    let _ = std::fs::create_dir_all(&hands_dir);
    let hands = Arc::new(HandRegistry::load(&hands_dir).unwrap_or_else(|e| {
        warn!("Hands loading failed: {}", e);
        HandRegistry::empty()
    }));
    let hand_names = hands.names();
    if !hand_names.is_empty() {
        info!("Hands loaded: {}", hand_names.join(", "));
    }

    // Register run_hand tool (natural language → Hand workflow)
    let tool_registry_arc_for_hand = Arc::new(ToolRegistry::new(SecurityConfig::default()));
    tool_registry.register(Box::new(clawtex_core::tools::run_hand::RunHandTool::new(
        agent_runtime.clone(),
        llm_router.clone(),
        tool_registry_arc_for_hand,
        hands.clone(),
    )));
    info!("run_hand tool registered ({} hands available)", hands.names().len());

    // ── Audit Logger ─────────────────────────────────────────────────
    let audit_db_path = format!("{}/.clawtex/audit.db", home);
    let audit_logger: Option<Arc<AuditLogger>> = match AuditLogger::new(&audit_db_path) {
        Ok(al) => {
            let al = Arc::new(al);
            tool_registry.set_audit_logger(al.clone());
            info!("Audit logger initialized and wired to tool registry");
            Some(al)
        }
        Err(e) => {
            warn!("Audit logger failed to init: {}", e);
            None
        }
    };

    let tool_registry = Arc::new(tool_registry);

    // ── Skills Registry ─────────────────────────────────────────────
    let skills_dir = format!("{}/.clawtex/skills", home);
    let installed_dir = format!("{}/.clawtex/installed_skills", home);
    let _ = std::fs::create_dir_all(&skills_dir);
    let skill_registry = Arc::new(SkillRegistry::load(&[
        (&skills_dir, TrustLevel::Trusted),
        (&installed_dir, TrustLevel::Installed),
    ]).unwrap_or_else(|e| {
        warn!("Skills loading failed: {}", e);
        SkillRegistry::load(&[]).unwrap()
    }));

    // ── Eval Config ─────────────────────────────────────────────────
    let eval_config = app_config.eval.unwrap_or_default();
    if eval_config.enabled {
        info!("Self-correction: enabled (threshold={}/5, max_retries={})", eval_config.threshold, eval_config.max_retries);
    }

    // ── Startup Self-Test ─────────────────────────────────────────────
    info!("Running startup checks...");
    {
        // Check workspace dir
        let ws = &tool_registry.workspace_dir();
        if std::path::Path::new(ws).exists() {
            info!("  [OK] workspace: {}", ws);
        } else {
            warn!("  [!!] workspace dir missing, creating: {}", ws);
            let _ = std::fs::create_dir_all(ws);
        }

        // Check master agent exists
        if agent_runtime.get_config("master").is_some() {
            info!("  [OK] master agent configured");
        } else {
            warn!("  [!!] no 'master' agent in config — Telegram will fail");
        }

        // Check LLM provider connectivity
        match llm_router.route("ping", "auto").await {
            Ok(_) => info!("  [OK] LLM provider reachable"),
            Err(e) => warn!("  [!!] LLM provider unreachable: {}", e),
        }
    }
    info!("Startup checks complete.");

    // Initialize E-Stop
    let estop = Arc::new(EStop::new());

    // ── Approval Gate ──────────────────────────────────────────────
    let approval_gate = Arc::new(ApprovalGate::new(Default::default()));
    // Wire audit logger into approval gate for logging approval decisions
    if let Some(ref al) = audit_logger {
        approval_gate.set_audit_logger(al.clone()).await;
        info!("Audit logger wired to approval gate");
    }

    // Generate dashboard access token
    let dashboard_token = uuid::Uuid::new_v4().to_string().replace("-", "")[..16].to_string();
    info!("Dashboard token generated (use /dashboard in Telegram to get URL)");

    // Hub API key — from config or auto-generated
    let hub_api_key: Option<String> = app_config.core
        .as_ref()
        .and_then(|c| c.hub_api_key.clone())
        .and_then(|k| if k.is_empty() { None } else { Some(k) })
        .or_else(|| {
            let key = uuid::Uuid::new_v4().to_string().replace("-", "");
            info!("Hub API key auto-generated: {}...", &key[..16]);
            Some(key)
        });
    if let Some(ref key) = hub_api_key {
        info!("Hub API key: {} (use Authorization: Bearer <key>)", key);
    }

    // Cluster secret — optional token for worker ↔ hub authentication.
    // Read from [cluster] config section or CLUSTER_SECRET env var.
    let cluster_secret: Option<String> = {
        let config_val = app_config.cluster.as_ref().and_then(|c| c.cluster_secret.as_deref());
        clawtex_core::cluster_worker::resolve_cluster_secret(config_val)
    };
    if cluster_secret.is_some() {
        info!("Cluster secret configured — worker authentication enabled on cluster endpoints");
    } else {
        info!("No CLUSTER_SECRET set — cluster endpoints accept unauthenticated requests");
    }

    // Auto-detect ngrok public URL
    let public_url = detect_ngrok_url().await;
    if let Some(ref url) = public_url {
        info!("Detected ngrok tunnel: {}", url);
    }

    // ── Cron Scheduler ─────────────────────────────────────────────
    let cron_store = Arc::new(CronStore::new(&db_path)?);
    let scheduler = Arc::new(Scheduler::new(cron_store)?);

    // ── Load or create user profile ──────────────────────────────────
    let user_profile = {
        let profile_conn = rusqlite::Connection::open(&db_path)?;
        UserProfile::create_table(&profile_conn)?;
        let profile = UserProfile::load(&profile_conn)?
            .unwrap_or_else(|| {
                let default = UserProfile::default();
                let _ = default.save(&profile_conn);
                default
            });
        info!("User profile loaded: {} ({})", profile.display_name, profile.timezone);
        Arc::new(RwLock::new(profile))
    };

    // ── Bootstrap event triggers ─────────────────────────────────────
    let trigger_manager: Option<Arc<std::sync::Mutex<clawtex_core::event_triggers::EventTriggerManager>>> = {
        use clawtex_core::event_triggers::EventTriggerManager;
        match rusqlite::Connection::open(&db_path) {
            Ok(conn) => {
                if let Err(e) = EventTriggerManager::create_table(&conn) {
                    warn!("Failed to create event_triggers table: {}", e);
                    None
                } else {
                    // Bootstrap defaults if table is empty
                    let trigger_count: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM event_triggers", [], |r| r.get(0)
                    ).unwrap_or(0);
                    if trigger_count == 0 {
                        let profile = user_profile.read().unwrap_or_else(|p| p.into_inner());
                        if let Err(e) = EventTriggerManager::bootstrap_defaults(&conn, &profile) {
                            warn!("Failed to bootstrap event trigger defaults: {}", e);
                        } else {
                            info!("Bootstrapped 5 default event triggers");
                        }
                    }
                    match EventTriggerManager::load_triggers(&conn) {
                        Ok(triggers) => {
                            info!("Loaded {} event trigger(s) from DB", triggers.len());
                            Some(Arc::new(std::sync::Mutex::new(
                                EventTriggerManager::new(triggers, user_profile.clone())
                            )))
                        }
                        Err(e) => {
                            warn!("Failed to load event triggers: {}", e);
                            None
                        }
                    }
                }
            }
            Err(e) => {
                warn!("Failed to open DB for event triggers: {}", e);
                None
            }
        }
    };

    // Register default cron jobs if no jobs exist yet
    {
        let existing = scheduler.list_jobs().await;
        if existing.is_empty() {
            info!("No cron jobs found — registering default schedules");
            // Daily freelancer search at 9:00 AM
            if let Err(e) = scheduler.add_job(
                "daily-freelancer",
                Schedule::Cron { expr: "0 9 * * *".to_string() },
                JobAction::Hand {
                    hand_name: "freelancer".to_string(),
                    input: "AI automation, web development, and data analysis jobs".to_string(),
                },
                None,
            ).await {
                warn!("Failed to register default freelancer cron: {}", e);
            }
            // Weekly lead generation on Mondays at 10:00 AM
            if let Err(e) = scheduler.add_job(
                "weekly-leads",
                Schedule::Cron { expr: "0 10 * * 1".to_string() },
                JobAction::Hand {
                    hand_name: "lead".to_string(),
                    input: "SaaS companies needing AI automation in healthcare and fintech".to_string(),
                },
                None,
            ).await {
                warn!("Failed to register default lead cron: {}", e);
            }
            // Bi-weekly SEO content on Tuesday and Thursday at 11:00 AM
            if let Err(e) = scheduler.add_job(
                "biweekly-seo-content",
                Schedule::Cron { expr: "0 11 * * 2,4".to_string() },
                JobAction::Hand {
                    hand_name: "seo_content".to_string(),
                    input: "AI tools reviews, comparisons, and tutorials for developers".to_string(),
                },
                None,
            ).await {
                warn!("Failed to register default seo_content cron: {}", e);
            }
            // Daily content creation at 8:00 AM
            if let Err(e) = scheduler.add_job(
                "daily-content",
                Schedule::Cron { expr: "0 8 * * *".to_string() },
                JobAction::Hand {
                    hand_name: "content".to_string(),
                    input: "AI automation trends, developer productivity, and tech insights".to_string(),
                },
                None,
            ).await {
                warn!("Failed to register default content cron: {}", e);
            }
            // Cluster health check every 4 hours
            if let Err(e) = scheduler.add_job(
                "cluster-health",
                Schedule::Cron { expr: "0 */4 * * *".to_string() },
                JobAction::Hand {
                    hand_name: "cluster_health".to_string(),
                    input: "Run health check on all cluster nodes".to_string(),
                },
                None,
            ).await {
                warn!("Failed to register cluster_health cron: {}", e);
            }
            // Self-optimize weekly on Sunday at 3:00 AM
            if let Err(e) = scheduler.add_job(
                "weekly-self-optimize",
                Schedule::Cron { expr: "0 3 * * 0".to_string() },
                JobAction::Hand {
                    hand_name: "self_optimize".to_string(),
                    input: "Weekly cluster optimization based on health data".to_string(),
                },
                None,
            ).await {
                warn!("Failed to register self_optimize cron: {}", e);
            }
            info!("Default cron jobs registered: freelancer (daily 9AM), leads (Mon 10AM), seo_content (Tue/Thu 11AM), content (daily 8AM), cluster_health (every 4h), self_optimize (Sun 3AM)");
        } else {
            info!("{} existing cron jobs loaded", existing.len());
        }

        // Register goals push jobs (idempotent — skip if already exist)
        let existing_names: std::collections::HashSet<String> = scheduler.list_jobs().await.iter().map(|j| j.name.clone()).collect();
        if !existing_names.contains("goals-morning-briefing") {
            if let Err(e) = scheduler.add_job(
                "goals-morning-briefing",
                Schedule::Cron { expr: "0 8 * * *".to_string() },
                JobAction::Notify {
                    chat_id: "auto".to_string(),
                    message: "__GOALS_MORNING__".to_string(),
                },
                None,
            ).await {
                warn!("Failed to register goals morning briefing: {}", e);
            } else {
                info!("Registered goals-morning-briefing cron (daily 8AM)");
            }
        }
        if !existing_names.contains("goals-evening-checkin") {
            if let Err(e) = scheduler.add_job(
                "goals-evening-checkin",
                Schedule::Cron { expr: "0 21 * * *".to_string() },
                JobAction::Notify {
                    chat_id: "auto".to_string(),
                    message: "__GOALS_EVENING__".to_string(),
                },
                None,
            ).await {
                warn!("Failed to register goals evening check-in: {}", e);
            } else {
                info!("Registered goals-evening-checkin cron (daily 9PM)");
            }
        }
        if !existing_names.contains("goals-weekly-report") {
            if let Err(e) = scheduler.add_job(
                "goals-weekly-report",
                Schedule::Cron { expr: "0 20 * * 0".to_string() },
                JobAction::Notify {
                    chat_id: "auto".to_string(),
                    message: "__GOALS_WEEKLY__".to_string(),
                },
                None,
            ).await {
                warn!("Failed to register goals weekly report: {}", e);
            } else {
                info!("Registered goals-weekly-report cron (Sunday 8PM)");
            }
        }
    }

    // Reuse the ClusterHub created earlier for agent_runtime dispatch

    // Initialize load tester
    let load_test_db_path = format!("{}/.clawtex/load_tests.db", dirs_home());
    let load_tester: Option<Arc<LoadTester>> = match LoadTester::new(
        agent_runtime.clone(),
        llm_router.clone(),
        tool_registry.clone(),
        hands.clone(),
        Some(&load_test_db_path),
    ) {
        Ok(lt) => {
            info!("Load tester initialized (db: {})", load_test_db_path);
            Some(Arc::new(lt))
        }
        Err(e) => {
            warn!("Load tester failed to init: {}", e);
            None
        }
    };

    // ── Optimizer Policy Store ───────────────────────────────────────
    let optimizer_db_path = clawtex_dir.join("optimizer.db");
    let optimizer_store: Option<Arc<OptimizerStore>> = match OptimizerStore::new(
        optimizer_db_path.to_str().unwrap_or("optimizer.db"),
    ) {
        Ok(store) => {
            let store = Arc::new(store);
            let baselines = [
                ("prompt.default", PolicyType::Prompt, r#"{"hands":{},"agents":{}}"#),
                ("routing.default", PolicyType::Routing, r#"{"preferred_nodes":{},"fallback_nodes":{},"tool_overrides":{}}"#),
                ("workflow.default", PolicyType::Workflow, r#"{"hands":{},"phase_order_overrides":{},"playbooks":{}}"#),
                ("runtime_tuning.default", PolicyType::RuntimeTuning, r#"{"timeouts":{"agent_secs":120,"shell_default_secs":30},"retry":{},"budget":{}}"#),
            ];
            for (policy_id, policy_type, content) in baselines {
                if let Err(e) = store.ensure_baseline_policy(policy_id, policy_type, content) {
                    warn!("Failed to bootstrap baseline policy '{}': {}", policy_id, e);
                }
            }
            info!("Optimizer store initialized (db: {:?})", optimizer_db_path);
            Some(store)
        }
        Err(e) => {
            warn!("Optimizer store failed to init: {}", e);
            None
        }
    };

    // ── Worker Onboarder ────────────────────────────────────────────
    let worker_onboarder = Arc::new(WorkerOnboarder::new(cluster.clone()));
    info!("Worker onboarder initialized");

    // ── Auto Diagnoser ──────────────────────────────────────────────
    let diagnosis_db_path = format!("{}/.clawtex/diagnosis.db", home);
    let auto_diagnoser: Option<Arc<AutoDiagnoser>> = match AutoDiagnoser::new(&diagnosis_db_path) {
        Ok(ad) => {
            info!("Auto-diagnosis engine initialized ({} known patterns, db: {})",
                  ad.get_common_issues().len(), diagnosis_db_path);
            Some(Arc::new(ad))
        }
        Err(e) => {
            warn!("Auto-diagnosis engine failed to init: {}", e);
            None
        }
    };

    // ── Customer Health & Churn Detection ──────────────────────────
    let customer_health_db_path = format!("{}/.clawtex/customer_health.db", home);
    let (customer_health, churn_detector) = match CustomerHealthManager::new(&customer_health_db_path) {
        Ok(mgr) => {
            let detector = ChurnDetector::new(&customer_health_db_path).ok().map(Arc::new);
            info!("Customer health manager initialized (db: {})", customer_health_db_path);
            (Some(Arc::new(mgr)), detector)
        }
        Err(e) => {
            warn!("Customer health manager failed to init: {}", e);
            (None, None)
        }
    };

    // ── Observational Memory ─────────────────────────────────────────
    let obs_db_path = format!("{}/.clawtex/observations.db", home);
    let observational_memory: Option<Arc<ObservationalMemory>> = match ObservationalMemory::new(&obs_db_path) {
        Ok(om) => {
            info!("Observational memory initialized (db: {})", obs_db_path);
            Some(Arc::new(om))
        }
        Err(e) => {
            warn!("Observational memory disabled: {}", e);
            None
        }
    };

    // ── Task Preemption Manager ─────────────────────────────────────
    let preemption_db_path = format!("{}/.clawtex/core.db", home);
    let preemption_manager: Option<Arc<PreemptionManager>> = match PreemptionManager::new(&preemption_db_path) {
        Ok(pm) => {
            info!("Task preemption manager initialized (db: {})", preemption_db_path);
            Some(Arc::new(pm))
        }
        Err(e) => {
            warn!("Task preemption manager failed to init: {}", e);
            None
        }
    };

    // ── Node Capability Scorer ──────────────────────────────────────
    let scoring_db_path = format!("{}/.clawtex/core.db", home);
    let node_scorer: Option<Arc<NodeScorer>> = match NodeScorer::new(&scoring_db_path) {
        Ok(ns) => {
            info!("Node capability scorer initialized (db: {})", scoring_db_path);
            Some(Arc::new(ns))
        }
        Err(e) => {
            warn!("Node capability scorer failed to init: {}", e);
            None
        }
    };

    let mut state = AppState {
        llm_router,
        task_queue,
        agent_runtime,
        cluster,
        tool_registry,
        conversations,
        memory_store,
        skill_registry,
        eval_config,
        estop,
        hands,
        approval_gate,
        scheduler: Some(scheduler.clone()),
        cost_tracker,
        revenue_tracker,
        cluster_hub: Some(cluster_hub.clone()),
        hub_api_key,
        dashboard_token,
        public_url,
        metrics_registry: Arc::new(clawtex_core::metrics::default_metrics()),
        audit_logger,
        load_tester,
        worker_onboarder: Some(worker_onboarder),
        service_tier,
        optimizer_store,
        auto_diagnoser,
        tenant_manager,
        order_workflow,
        customer_health,
        churn_detector,
        observational_memory,
        preemption_manager,
        node_scorer,
        power_economics,
        provider_pricing,
        financial_monitor: Some(Arc::new(FinancialMonitor::default())),
        unit_economics: Some(Arc::new(UnitEconomics::new())),
        telegram_i18n: Arc::new(tokio::sync::RwLock::new(TelegramI18n::new())),
        cluster_secret,
        started_at: Instant::now(),
        // Efficiency engine — initialized below after AppState is built
        roi_gate: None,
        governor: None,
        pipeline_orchestrator: None,
        feedback_loop_config: None,
        roi_scheduler: None,
        route_manager: None,
        goals_store,
        user_profile,
        trigger_manager,
        networking_tasks: Arc::new(tokio::sync::Mutex::new(Vec::new())),
    };

    // ── Networking: mDNS Discovery + Route Manager ─────────────────
    {
        use clawtex_core::networking::mdns::MdnsDiscovery;
        use clawtex_core::networking::iroh_transport::IrohTransport;
        use clawtex_core::networking::{RouteManager, ServiceDiscovery};

        let node_name = std::env::var("CLAWTEX_NODE_NAME")
            .unwrap_or_else(|_| "clawtex-hub".to_string());
        let mdns = Arc::new(MdnsDiscovery::new(node_name.clone(), args.port, vec!["hub".into()]));
        match ServiceDiscovery::start(mdns.as_ref()).await {
            Ok(()) => info!("mDNS discovery started for '{}' on port {}", node_name, args.port),
            Err(e) => warn!("mDNS discovery failed to start: {} (networking degraded)", e),
        }

        // Layer 2: Iroh/QUIC transport (stub — ready for real iroh crate when available)
        let iroh = Arc::new(IrohTransport::new());
        info!("Iroh transport registered (stub mode — QUIC mesh not yet active)");

        let mut rm = RouteManager::default_ttl();
        rm.add_discovery(mdns.clone());
        rm.add_transport(iroh);

        // Sync discovered nodes to ClusterRegistry periodically
        let rm = Arc::new(rm);
        let rm_clone = Arc::clone(&rm);
        let cluster_for_mdns = state.cluster.clone();
        let sync_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
            loop {
                interval.tick().await;
                let routes = rm_clone.all_routes().await;
                for route in &routes {
                    // Auto-register discovered nodes into ClusterRegistry
                    // Use url::Url parser for correct IPv6/scheme handling
                    if let Ok(parsed) = url::Url::parse(&route.url) {
                        let host = parsed.host_str().unwrap_or("127.0.0.1").to_string();
                        let port = parsed.port().unwrap_or(7878);
                        if let Err(e) = cluster_for_mdns.register(&route.node_name, &host, port).await {
                            tracing::debug!("Auto-register {} failed: {}", route.node_name, e);
                        }
                    }
                }
            }
        });

        let refresh_handle = rm.spawn_refresh_loop(std::time::Duration::from_secs(30));
        {
            let mut tasks = state.networking_tasks.lock().await;
            tasks.push(sync_handle);
            tasks.push(refresh_handle);
        }
        state.route_manager = Some(rm);
    }
    {
        let hub = cluster_hub.clone();
        tokio::spawn(async move { hub.staleness_loop().await });
    }

    // Shared Telegram refs — used by both Telegram handler and cron executor
    let shared_telegram: Arc<tokio::sync::RwLock<Option<Arc<TelegramChannel>>>> =
        Arc::new(tokio::sync::RwLock::new(None));
    let shared_last_chat_id: Arc<tokio::sync::RwLock<Option<String>>> =
        Arc::new(tokio::sync::RwLock::new(None));

    // Start Telegram channel if configured
    if let Some(tg_config) = app_config.telegram {
        if tg_config.bot_token.is_empty() || tg_config.bot_token.starts_with("YOUR_") {
            warn!("Telegram bot_token not set, skipping Telegram");
        } else {
            let telegram = Arc::new(TelegramChannel::new(tg_config));
            *shared_telegram.write().await = Some(telegram.clone());
            let (tx, rx) = mpsc::channel::<ChannelMessage>(100);

            let tg_listen = telegram.clone();
            tokio::spawn(async move {
                if let Err(e) = tg_listen.listen(tx).await {
                    error!("Telegram listener error: {}", e);
                }
            });

            // Wire up approval gate notifier — sends approval requests to Telegram.
            // Uses a shared last_chat_id that gets updated from incoming messages.
            let last_chat_id = shared_last_chat_id.clone();
            {
                let tg_for_approval = telegram.clone();
                let last_id = last_chat_id.clone();
                let notifier: clawtex_core::ApprovalNotifier = Arc::new(move |msg: String| {
                    let tg = tg_for_approval.clone();
                    let last_id = last_id.clone();
                    Box::pin(async move {
                        let chat_id = last_id.read().await.clone();
                        if let Some(cid) = chat_id {
                            if let Err(e) = tg.send(&cid, &msg).await {
                                warn!("Failed to send approval notification to Telegram: {}", e);
                            }
                        } else {
                            warn!("No chat_id available for approval notification — send a message to the bot first");
                        }
                    })
                });
                state.approval_gate.set_notifier(notifier).await;
                info!("Approval gate notifier wired to Telegram");
            }

            let tg_handler = telegram.clone();
            let handler_state = state.clone();
            tokio::spawn(async move {
                handle_telegram_messages(rx, tg_handler, handler_state, last_chat_id).await;
            });

            info!("Telegram channel enabled");
        }
    } else {
        warn!("No [telegram] section in config, Telegram disabled");
    }

    // ── Start Cron Scheduler Background Loop ─────────────────────
    {
        let scheduler = scheduler.clone();
        let executor_state = state.clone();
        let tg_for_cron = shared_telegram.clone();
        let chat_id_for_cron = shared_last_chat_id.clone();
        let trigger_manager_for_cron = state.trigger_manager.clone();
        let db_path_for_cron = db_path.clone();
        tokio::spawn(async move {
            let executor: clawtex_core::cron::JobExecutor = Arc::new(move |action| {
                let s = executor_state.clone();
                let tg_ref = tg_for_cron.clone();
                let chat_ref = chat_id_for_cron.clone();
                tokio::spawn(async move {
                    match action {
                        clawtex_core::JobAction::Shell { command } => {
                            match s.tool_registry.execute_tool("shell", serde_json::json!({"command": command})).await {
                                Ok(r) => r.output,
                                Err(e) => format!("Shell error: {}", e),
                            }
                        }
                        clawtex_core::JobAction::Agent { agent, prompt } => {
                            let history = vec![];
                            match s.agent_runtime.run(&agent, &prompt, &history, &s.llm_router, &s.tool_registry, None).await {
                                Ok(r) => r.output,
                                Err(e) => format!("Agent error: {}", e),
                            }
                        }
                        clawtex_core::JobAction::Notify { chat_id, message } => {
                            // Handle goals push magic messages
                            let is_goals_push = message == "__GOALS_MORNING__" || message == "__GOALS_EVENING__" || message == "__GOALS_WEEKLY__";
                            let goals_keyboard: Option<clawtex_core::telegram_menu::InlineKeyboard> = if is_goals_push {
                                if message == "__GOALS_EVENING__" {
                                    Some(clawtex_core::telegram_menu::goals_mood_selector())
                                } else if message == "__GOALS_MORNING__" {
                                    // Build task buttons for incomplete tasks
                                    if let Some(ref gs) = s.goals_store {
                                        if let Ok(tasks) = gs.get_today_tasks() {
                                            let pending: Vec<(&str, &str)> = tasks.iter()
                                                .filter(|t| !t.completed_today)
                                                .map(|t| (t.task.id.as_str(), t.task.title.as_str()))
                                                .collect();
                                            if !pending.is_empty() {
                                                Some(clawtex_core::telegram_menu::goals_task_buttons(&pending))
                                            } else { None }
                                        } else { None }
                                    } else { None }
                                } else { None }
                            } else { None };

                            let actual_message = if is_goals_push {
                                if let Some(ref gs) = s.goals_store {
                                    let result = if message == "__GOALS_MORNING__" {
                                        clawtex_core::goals_push::morning_briefing(gs)
                                    } else if message == "__GOALS_WEEKLY__" {
                                        clawtex_core::goals_push::weekly_report(gs)
                                    } else {
                                        clawtex_core::goals_push::evening_checkin(gs)
                                    };
                                    match result {
                                        Ok(msg) if msg.is_empty() => {
                                            return "Goals push skipped: no active goals".to_string();
                                        }
                                        Ok(msg) => msg,
                                        Err(e) => {
                                            warn!("Goals push generation error: {}", e);
                                            return format!("Goals push error: {}", e);
                                        }
                                    }
                                } else {
                                    return "Goals push skipped: no goals store".to_string();
                                }
                            } else {
                                message
                            };

                            // Send via Telegram if available
                            let tg_guard = tg_ref.read().await;
                            if let Some(ref tg) = *tg_guard {
                                let target = if chat_id.is_empty() || chat_id == "auto" {
                                    chat_ref.read().await.clone()
                                } else {
                                    Some(chat_id.clone())
                                };
                                if let Some(cid) = target {
                                    let send_result = if let Some(ref kb) = goals_keyboard {
                                        tg.send_message_with_keyboard(&cid, &actual_message, kb).await.map(|_| ())
                                    } else {
                                        tg.send(&cid, &actual_message).await
                                    };
                                    match send_result {
                                        Ok(()) => {
                                            info!("Cron notify sent to Telegram [{}]", cid);
                                            format!("Notified via Telegram: {}", &actual_message[..actual_message.len().min(80)])
                                        }
                                        Err(e) => {
                                            warn!("Cron notify Telegram error: {}", e);
                                            format!("Telegram send failed: {}", e)
                                        }
                                    }
                                } else {
                                    info!("Cron notify (no chat_id): {}", &actual_message[..actual_message.len().min(80)]);
                                    format!("Notified (no Telegram chat): {}", &actual_message[..actual_message.len().min(80)])
                                }
                            } else {
                                info!("Cron notify (no Telegram): {}", &actual_message[..actual_message.len().min(80)]);
                                format!("Notified (Telegram disabled): {}", &actual_message[..actual_message.len().min(80)])
                            }
                        }
                        clawtex_core::JobAction::Hand { hand_name, input } => {
                            if let Some(hand) = s.hands.get(&hand_name) {
                                info!("Cron executing hand '{}' with input: {}", hand_name, &input[..input.len().min(80)]);
                                match HandRunner::run(hand, &input, &s.agent_runtime, &s.llm_router, &s.tool_registry, Some(&s.approval_gate)).await {
                                    Ok(result) => {
                                        let summary = format!(
                                            "Hand '{}' completed: {}/{} phases in {:.1}s",
                                            result.hand_name, result.phases_completed, result.total_phases, result.elapsed_secs
                                        );
                                        info!("{}", summary);
                                        // If chained, run next hand
                                        if let Some(ref next_hand_name) = result.chain_to {
                                            if let Some(next_hand) = s.hands.get(next_hand_name) {
                                                info!("Cron chaining to hand '{}'", next_hand_name);
                                                match HandRunner::run(next_hand, &result.final_output, &s.agent_runtime, &s.llm_router, &s.tool_registry, Some(&s.approval_gate)).await {
                                                    Ok(chained) => format!("{}\n→ Chained '{}': {}/{} phases in {:.1}s", summary, chained.hand_name, chained.phases_completed, chained.total_phases, chained.elapsed_secs),
                                                    Err(e) => format!("{}\n→ Chain '{}' failed: {}", summary, next_hand_name, e),
                                                }
                                            } else {
                                                format!("{}\n→ Chain target '{}' not found", summary, next_hand_name)
                                            }
                                        } else {
                                            summary
                                        }
                                    }
                                    Err(e) => format!("Hand '{}' error: {}", hand_name, e),
                                }
                            } else {
                                format!("Hand '{}' not found in registry", hand_name)
                            }
                        }
                    }
                })
            });
            scheduler.run_with_triggers(executor, trigger_manager_for_cron, db_path_for_cron, None, None).await;
        });
        info!("Cron scheduler started (event triggers enabled)");
    }

    // Periodic cleanup of stale conversations (every 30 min)
    let cleanup_convos = state.conversations.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1800));
        loop {
            interval.tick().await;
            cleanup_convos.cleanup_stale().await;
        }
    });

    // 24/7 Watchdog — monitors system health every hour
    let watchdog_estop = state.estop.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            // Check if e-stop is engaged
            if watchdog_estop.is_stopped() {
                tracing::warn!("Watchdog: e-stop is engaged — system paused");
                continue;
            }
            // Log uptime heartbeat
            tracing::info!("Watchdog: system alive — uptime check OK");
        }
    });

    // Worker Watchdog — monitors workers, auto-restarts via SSH
    let mut watchdog = WorkerWatchdog::with_defaults();
    // Also add ayaneo
    watchdog.add_worker(RecoveryConfig::new(
        "ayaneo",
        r#"ssh worker@10.0.1.4 "wmic process where \"name='python.exe'\" call terminate >nul 2>&1 & cd /d /home/user/worker & python worker.py""#,
    ));
    let watchdog = Arc::new(tokio::sync::Mutex::new(watchdog));
    info!("WorkerWatchdog initialized with {} workers", 3);

    // Build gateway state for streaming + trajectory + health endpoints
    let gateway_state = GatewayState {
        agent_runtime: state.agent_runtime.clone(),
        llm_router: state.llm_router.clone(),
        tool_registry: state.tool_registry.clone(),
        estop: state.estop.clone(),
        trajectory_logger: trajectory_logger.clone(),
        circuit_breaker: Some(circuit_breaker.clone()),
        watchdog: Some(watchdog.clone()),
        agent_think_rate: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
    };

    // Spawn watchdog monitoring loop
    let wd_for_loop = watchdog.clone();
    let hub_for_wd = state.cluster_hub.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Some(hub) = &hub_for_wd {
                let workers = hub.registry.status().await;
                let wd = wd_for_loop.lock().await;
                let events = wd.check_and_recover(&workers).await;
                for event in &events {
                    info!("Watchdog event: {:?}", event);
                }
            }
        }
    });
    info!("Watchdog monitoring loop started (60s interval)");

    // ── Customer Health Endpoint Handlers ──────────────────────────

    async fn customers_health_list(State(state): State<AppState>) -> Json<Value> {
        let mgr = match &state.customer_health {
            Some(m) => m,
            None => return Json(json!({ "error": "Customer health manager not available" })),
        };
        let all = mgr.list_all().unwrap_or_default();
        let avg = mgr.average_health().unwrap_or(0.0);
        Json(json!({ "customers": all, "count": all.len(), "average_health": avg }))
    }

    async fn customers_health_get(
        State(state): State<AppState>, Path(id): Path<String>,
    ) -> Result<Json<Value>, StatusCode> {
        let mgr = state.customer_health.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        match mgr.get_health(&id) {
            Ok(Some(h)) => Ok(Json(json!(h))),
            Ok(None) => Err(StatusCode::NOT_FOUND),
            Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    async fn customers_health_update(
        State(state): State<AppState>, Path(id): Path<String>, Json(body): Json<Value>,
    ) -> Result<Json<Value>, StatusCode> {
        let mgr = state.customer_health.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown");
        let eff = body.get("efficiency").and_then(|v| v.as_f64()).ok_or(StatusCode::BAD_REQUEST)?;
        let qual = body.get("quality").and_then(|v| v.as_f64()).ok_or(StatusCode::BAD_REQUEST)?;
        let spd = body.get("speed").and_then(|v| v.as_f64()).ok_or(StatusCode::BAD_REQUEST)?;
        let sat = body.get("satisfaction").and_then(|v| v.as_f64()).ok_or(StatusCode::BAD_REQUEST)?;
        mgr.update_scores(&id, name, eff, qual, spd, sat)
            .map(|h| Json(json!(h)))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    }

    async fn customers_at_risk(State(state): State<AppState>) -> Json<Value> {
        let mgr = match &state.customer_health {
            Some(m) => m,
            None => return Json(json!({ "error": "Customer health manager not available" })),
        };
        let at_risk = mgr.get_at_risk().unwrap_or_default();
        Json(json!({ "at_risk": at_risk, "count": at_risk.len() }))
    }

    async fn customers_churn_alerts(State(state): State<AppState>) -> Json<Value> {
        let detector = match &state.churn_detector {
            Some(d) => d,
            None => return Json(json!({ "error": "Churn detector not available" })),
        };
        let alerts = detector.get_all_active_alerts().unwrap_or_default();
        let summary = detector.churn_summary().unwrap_or(clawtex_core::ChurnSummary {
            total_active: 0, low: 0, medium: 0, high: 0, critical: 0,
        });
        Json(json!({ "alerts": alerts, "summary": summary }))
    }

    async fn customers_record_activity(
        State(state): State<AppState>, Path(id): Path<String>,
    ) -> Result<Json<Value>, StatusCode> {
        let detector = state.churn_detector.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        detector.record_activity(&id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        Ok(Json(json!({ "status": "ok", "customer_id": id })))
    }

    // ── Revenue Dashboard API ─────────────────────────────────────────────────
    async fn api_revenue_dashboard() -> Result<Json<Value>, StatusCode> {
        let home = dirs::home_dir().ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        let revenue_db = home.join(".clawtex").join("revenue.db");
        let cost_db = home.join(".clawtex").join("costs.db");
        let revenue_path = revenue_db.to_str().ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        let cost_path = cost_db.to_str().ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

        match clawtex_core::web_dashboard::build_revenue_dashboard(revenue_path, cost_path) {
            Ok(data) => {
                let json_val = serde_json::to_value(&data).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                Ok(Json(json_val))
            }
            Err(e) => {
                error!("Revenue dashboard build failed: {}", e);
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }

    // ── Deploy Manifest API ───────────────────────────────────────────────────
    async fn api_deploy_manifest(State(state): State<AppState>) -> Json<Value> {
        let manifest = DeployManifest::generate()
            .with_hands(state.hands.list().iter().map(|h| h.name.clone()).collect())
            .with_tools(state.tool_registry.names())
            .with_providers(state.llm_router.provider_names());

        match serde_json::from_str::<Value>(&manifest.to_json()) {
            Ok(val) => Json(val),
            Err(_) => Json(json!({"error": "Failed to serialize deploy manifest"})),
        }
    }

    // ── Provider Health API ───────────────────────────────────────────────────
    async fn api_providers_health(State(state): State<AppState>) -> Json<Value> {
        let summary = state.llm_router.inner().health_summary();
        match serde_json::to_value(&summary) {
            Ok(val) => Json(val),
            Err(_) => Json(json!([])),
        }
    }

    // ── Stripe Webhook API ────────────────────────────────────────────────────
    async fn api_stripe_webhook(
        State(state): State<AppState>,
        headers: axum::http::HeaderMap,
        body: axum::body::Bytes,
    ) -> Result<Json<Value>, StatusCode> {
        let signature = headers
            .get("Stripe-Signature")
            .and_then(|v| v.to_str().ok())
            .ok_or(StatusCode::BAD_REQUEST)?;

        let payload = std::str::from_utf8(&body).map_err(|_| StatusCode::BAD_REQUEST)?;

        // Use webhook secret from env or default empty (will fail verification gracefully)
        let webhook_secret = std::env::var("STRIPE_WEBHOOK_SECRET").unwrap_or_default();
        if webhook_secret.is_empty() {
            warn!("STRIPE_WEBHOOK_SECRET not set, rejecting webhook");
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }

        let webhook = StripeWebhook::new(&webhook_secret);
        let action = webhook.process(payload, signature);

        match action {
            WebhookAction::RecordRevenue { amount_usd, client, description } => {
                info!("Stripe webhook: recording ${:.2} from {} — {}", amount_usd, client, description);
                // Record revenue if tracker is available
                if let Some(ref tracker) = state.revenue_tracker {
                    let record = clawtex_core::revenue_tracker::RevenueRecord {
                        id: uuid::Uuid::new_v4().to_string(),
                        timestamp: chrono::Utc::now(),
                        route: "stripe".to_string(),
                        source: "stripe_webhook".to_string(),
                        client_name: client.clone(),
                        amount_usd,
                        currency: "USD".to_string(),
                        status: clawtex_core::revenue_tracker::RevenueStatus::Confirmed,
                        notes: Some(description.clone()),
                        invoice_id: None,
                    };
                    if let Err(e) = tracker.record(&record) {
                        error!("Failed to record Stripe revenue: {}", e);
                    }
                }
                Ok(Json(json!({
                    "status": "recorded",
                    "amount_usd": amount_usd,
                    "client": client,
                    "description": description
                })))
            }
            WebhookAction::Ignore => {
                Ok(Json(json!({ "status": "ignored" })))
            }
            WebhookAction::Error(msg) => {
                warn!("Stripe webhook error: {}", msg);
                Ok(Json(json!({ "status": "error", "message": msg })))
            }
        }
    }

    // ── Financial Status API ──────────────────────────────────────────────────
    async fn api_financial_status(State(state): State<AppState>) -> Result<Json<Value>, StatusCode> {
        let monitor = state.financial_monitor.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

        // Build a snapshot from available data (defaults/zeros for unavailable fields)
        let snapshot = FinancialSnapshot {
            daily_spend: 0.0,
            daily_limit: 10.0,
            api_cost: 0.0,
            revenue: 0.0,
            previous_revenue: 0.0,
            project_cost: 0.0,
            cash_balance: 0.0,
            monthly_burn: 0.0,
            current_period_cost: 0.0,
            average_cost: 0.0,
            budget_used: 0.0,
            budget_total: 100.0,
        };

        let alerts = monitor.evaluate_all(&snapshot);
        match serde_json::to_value(&alerts) {
            Ok(val) => Ok(Json(json!({
                "alerts": val,
                "alert_count": alerts.len(),
                "has_critical": FinancialMonitor::has_critical_alerts(&alerts)
            }))),
            Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    // ── Economics Summary API ─────────────────────────────────────────────────
    // ── Efficiency Engine Endpoints ─────────────────────────────────────────

    async fn api_governor_status(State(state): State<AppState>) -> Result<Json<Value>, StatusCode> {
        let store = state.optimizer_store.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        let canary = store.list_policies_by_status(clawtex_core::optimizer_store::PolicyStatus::Canary).unwrap_or_default();
        let active = store.list_policies_by_status(clawtex_core::optimizer_store::PolicyStatus::Active).unwrap_or_default();
        let draft = store.list_policies_by_status(clawtex_core::optimizer_store::PolicyStatus::Draft).unwrap_or_default();
        Ok(Json(json!({
            "canary_policies": canary.len(),
            "active_policies": active.len(),
            "draft_policies": draft.len(),
            "canary_details": canary.iter().map(|p| json!({
                "policy_id": p.policy_id,
                "version": p.version,
                "created_at": p.created_at,
            })).collect::<Vec<_>>(),
        })))
    }

    async fn api_roi_gate_status(State(state): State<AppState>) -> Result<Json<Value>, StatusCode> {
        if let Some(ref gate) = state.roi_gate {
            let spend = gate.current_spend();
            let config = &gate.config();
            Ok(Json(json!({
                "daily_spend_usd": spend,
                "daily_budget_usd": config.daily_budget_usd,
                "remaining_usd": config.daily_budget_usd - spend,
                "min_roi_threshold": config.min_roi_threshold,
                "exempt_hands": config.exempt_hands,
            })))
        } else {
            Ok(Json(json!({ "status": "not_initialized" })))
        }
    }

    async fn api_pipeline_list(State(state): State<AppState>) -> Result<Json<Value>, StatusCode> {
        if let Some(ref orch) = state.pipeline_orchestrator {
            let orch = orch.read().await;
            let pipelines: Vec<_> = orch.list_pipelines().iter().map(|p| json!({
                "name": p.name,
                "description": p.description,
                "steps": p.steps.len(),
                "step_details": p.steps.iter().map(|s| json!({
                    "hand_name": s.hand_name,
                    "optional": s.optional,
                    "has_condition": s.condition.is_some(),
                })).collect::<Vec<_>>(),
            })).collect();
            Ok(Json(json!({ "pipelines": pipelines })))
        } else {
            Ok(Json(json!({ "status": "not_initialized" })))
        }
    }

    async fn api_pipeline_run(
        State(state): State<AppState>,
        Json(body): Json<Value>,
    ) -> Result<Json<Value>, StatusCode> {
        let pipeline_name = body["pipeline"].as_str().unwrap_or("").to_string();
        let input = body["input"].as_str().unwrap_or("").to_string();
        if pipeline_name.is_empty() || input.is_empty() {
            return Ok(Json(json!({ "error": "pipeline and input fields required" })));
        }
        let orch = state.pipeline_orchestrator.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        let orch = orch.read().await;
        if orch.get_pipeline(&pipeline_name).is_none() {
            return Ok(Json(json!({ "error": format!("Pipeline '{}' not found", pipeline_name) })));
        }
        // Pipeline execution would be async — return acknowledgment
        Ok(Json(json!({
            "status": "accepted",
            "pipeline": pipeline_name,
            "input": input,
            "message": "Pipeline queued for execution. Check /api/feedback/report for results.",
        })))
    }

    async fn api_feedback_report(State(state): State<AppState>) -> Result<Json<Value>, StatusCode> {
        let config = state.feedback_loop_config.as_ref();
        let roi = state.roi_scheduler.as_ref();

        let mut result = json!({
            "feedback_loop": if config.is_some() { "active" } else { "not_initialized" },
        });

        if let Some(cfg) = config {
            result["config"] = json!({
                "min_trajectories": cfg.min_trajectories,
                "interval_secs": cfg.interval_secs,
                "target_hands": cfg.target_hands,
            });
        }

        if let Some(roi) = roi {
            result["roi_scheduler"] = json!({
                "status": "active",
            });
        }

        Ok(Json(result))
    }

    async fn api_economics_summary(State(state): State<AppState>) -> Result<Json<Value>, StatusCode> {
        let economics = state.unit_economics.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        let summary = economics.summary();
        match serde_json::to_value(&summary) {
            Ok(val) => Ok(Json(val)),
            Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    let host = args.host;
    let port = args.port;

    let auth_state = state.clone();
    let app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(prometheus_metrics))
        .route("/metrics/health", get(metrics_health))
        .route("/llm/route", post(route_llm))
        .route("/task", post(task_add))
        .route("/task/:id/run", post(task_run))
        .route("/task/history", get(task_history))
        .route("/agent/:name/run", post(agent_run))
        .route("/cluster/status", get(cluster_status))
        .route("/cluster/register", post(cluster_register))
        .route("/cluster/heartbeat", post(cluster_heartbeat))
        .route("/cluster/workers", get(cluster_workers))
        .route("/cluster/dispatch", post(cluster_dispatch))
        .route("/cluster/metrics", get(cluster_metrics))
        .route("/cluster/metrics/:worker", get(cluster_metrics_worker))
        .route("/cluster/poll", get(cluster_poll))
        .route("/cluster/result", post(cluster_result))
        .route("/cluster/onboard", post(cluster_onboard))
        .route("/cluster/onboard/status/:worker", get(cluster_onboard_status))
        .route("/cluster/onboard/verify/:worker", get(cluster_onboard_verify))
        .route("/cluster/onboard/mobile", post(cluster_onboard_mobile))
        .route("/cluster/consistency-test", post(cluster_consistency_test))
        .route("/cluster/consistency-history", get(cluster_consistency_history))
        .route("/cluster/preemption/pending", get(preemption_pending))
        .route("/cluster/preemption/history", get(preemption_history))
        .route("/cluster/scores", get(cluster_scores))
        .route("/cluster/scores/:node", get(cluster_score_node).post(cluster_score_update))
        .route("/power/nodes", get(power_nodes))
        .route("/power/nodes/:node_id", get(power_node_detail).post(power_node_upsert))
        .route("/power/estimate", post(power_estimate))
        .route("/power/profitability", post(power_profitability))
        .route("/pricing/rules", get(pricing_rules).post(pricing_rule_upsert))
        .route("/pricing/rules/:provider", get(pricing_rules_provider))
        .route("/pricing/estimate", post(pricing_estimate))
        .route("/tools", get(tools_list))
        .route("/hands", get(hands_list))
        .route("/hand/:name/run", post(hand_run))
        .route("/workspace/files", get(workspace_files))
        .route("/costs", get(costs_summary))
        .route("/revenue", get(revenue_summary))
        .route("/optimizer/policies", get(optimizer_policies))
        .route("/optimizer/runs", get(optimizer_runs))
        // Revenue, deploy, provider health, Stripe, financial, economics API endpoints
        .route("/api/revenue/dashboard", get(api_revenue_dashboard))
        .route("/api/deploy/manifest", get(api_deploy_manifest))
        .route("/api/providers/health", get(api_providers_health))
        .route("/api/stripe/webhook", post(api_stripe_webhook))
        .route("/api/financial/status", get(api_financial_status))
        .route("/api/economics/summary", get(api_economics_summary))
        // Operations report endpoints
        .route("/reports/daily", get(report_daily))
        .route("/reports/weekly", get(report_weekly))
        .route("/reports/send", post(report_send))
        .route("/tier/:agent", get(tier_get))
        .route("/tier/:agent", axum::routing::put(tier_set))
        .route("/tier/:agent/usage", get(tier_usage))
        .route("/audit", get(audit_query))
        // Tenant management endpoints
        .route("/tenants", post(tenant_create))
        .route("/tenants", get(tenant_list))
        .route("/tenants/validate", get(tenant_validate))
        .route("/tenants/:id", get(tenant_get))
        .route("/tenants/:id/tier", axum::routing::put(tenant_update_tier))
        .route("/tenants/:id", axum::routing::delete(tenant_deactivate))
        // Load testing endpoints
        .route("/test/stress", post(test_stress_start))
        .route("/test/stress/status", get(test_stress_status))
        .route("/test/stress/history", get(test_stress_history))
        .route("/test/stress/report/:run_id", get(test_stress_report))
        .route("/test/profiles", get(test_profiles))
        // Auto-diagnosis endpoints
        .route("/diagnose", post(diagnose_error_handler))
        .route("/diagnose/recent", get(diagnose_recent_handler))
        .route("/diagnose/stats", get(diagnose_stats_handler))
        .route("/diagnose/known-issues", get(diagnose_known_issues_handler))
        .route("/diagnose/:error_id", get(diagnose_get_handler))
        .route("/dashboard", get(dashboard))
        // Observational memory endpoints
        .route("/memory/observe", post(memory_observe))
        .route("/memory/observations", get(memory_observations))
        .route("/memory/observations/recent", get(memory_observations_recent))
        .route("/memory/observations/stats", get(memory_observations_stats))
        // Customer health & churn detection endpoints
        .route("/customers/health", get(customers_health_list))
        .route("/customers/health/:id", get(customers_health_get))
        .route("/customers/health/:id", axum::routing::put(customers_health_update))
        .route("/customers/at-risk", get(customers_at_risk))
        .route("/customers/churn-alerts", get(customers_churn_alerts))
        .route("/customers/:id/activity", post(customers_record_activity))
        // Order workflow endpoints
        .route("/orders", post(orders_create))
        .route("/orders", get(orders_list))
        .route("/orders/pipeline", get(orders_pipeline))
        .route("/orders/overdue", get(orders_overdue))
        .route("/orders/:id", get(orders_get))
        .route("/orders/:id/status", axum::routing::put(orders_transition))
        .route("/orders/:id/note", post(orders_add_note))
        // E-Stop endpoints
        .route("/estop", post(estop_activate))
        .route("/estop", axum::routing::delete(estop_reset))
        .route("/estop", get(estop_status))
        // Efficiency engine endpoints
        .route("/api/governor/status", get(api_governor_status))
        .route("/api/roi-gate/status", get(api_roi_gate_status))
        .route("/api/pipeline/list", get(api_pipeline_list))
        .route("/api/pipeline/run", post(api_pipeline_run))
        .route("/api/feedback/report", get(api_feedback_report))
        // ── Networking API ──────────────────────────────────────────
        .route("/networking/discovered", get(networking_discovered))
        .route("/networking/routes", get(networking_routes))
        .route("/networking/status", get(networking_status))
        // ── Goals API ──────────────────────────────────────────────
        .route("/goals", get(goals_list).post(goals_create))
        .route("/goals/today", get(goals_today))
        .route("/goals/summary", get(goals_active_summary))
        .route("/goals/push/preview", get(goals_push_preview))
        .route("/goals/:id", get(goals_get).put(goals_update).delete(goals_delete))
        .route("/goals/:id/progress", get(goals_progress))
        .route("/goals/:id/milestones", get(goals_milestones_list).post(goals_milestone_add))
        .route("/goals/:id/milestones/:ms_id/toggle", post(goals_milestone_toggle))
        .route("/goals/:id/recurring", get(goals_recurring_list).post(goals_recurring_add))
        .route("/goals/:id/recurring/:task_id/complete", post(goals_recurring_complete))
        .route("/goals/weekly-summary", get(goals_weekly_summary))
        .route("/goals/mood", get(goals_global_mood))
        .route("/goals/:id/checkins", get(goals_checkins_list).post(goals_checkin_add))
        .route("/goals/:id/mood-trend", get(goals_mood_trend))
        .with_state(state.clone())
        // Gateway streaming endpoints (separate state)
        .route("/stream/agent/:name", get(clawtex_core::gateway::sse_agent))
        .route("/ws/agent/:name", get(clawtex_core::gateway::ws_agent))
        .route("/agent/think", post(clawtex_core::gateway::agent_think))
        .route("/trajectories", get(clawtex_core::gateway::get_trajectories))
        .route("/trajectories/stats", get(clawtex_core::gateway::get_trajectory_stats))
        .route("/cluster/health", get(clawtex_core::gateway::get_cluster_health))
        .with_state(gateway_state)
        // Web dashboard — embedded single-page UI + JSON API
        .merge(clawtex_core::dashboard_routes(clawtex_core::DashboardState {
            tool_registry: state.tool_registry.clone(),
            hands: state.hands.clone(),
            conversations: state.conversations.clone(),
            cluster: state.cluster.clone(),
            cluster_hub: state.cluster_hub.clone(),
            cost_tracker: state.cost_tracker.clone(),
            agent_runtime: state.agent_runtime.clone(),
            started_at: state.started_at,
        }))
        // Hub Bearer token auth — exempts /health and /dashboard
        .layer(axum::middleware::from_fn_with_state(auth_state, auth_middleware));

    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr).await?;
    info!("Listening on http://{}", addr);

    let plugin_bus_shutdown = plugin_bus.clone();

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            info!("Received Ctrl+C, shutting down gracefully...");
            let mut bus = plugin_bus_shutdown.lock().await;
            if let Err(e) = bus.shutdown_all().await {
                tracing::error!("[PluginBus] Shutdown errors: {}", e);
            }
        })
        .await?;
    info!("Daemon stopped.");
    Ok(())
}

// ── Auto-Diagnosis Handlers ──────────────────────────────────────────────────

/// POST /diagnose — submit an error for auto-diagnosis
async fn diagnose_error_handler(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let diagnoser = state.auto_diagnoser.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let error_message = body.get("error_message")
        .and_then(|v| v.as_str())
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_string();
    let agent_name = body.get("agent_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let tool_name = body.get("tool_name").and_then(|v| v.as_str()).map(String::from);
    let hand_name = body.get("hand_name").and_then(|v| v.as_str()).map(String::from);
    let phase = body.get("phase").and_then(|v| v.as_u64()).map(|p| p as u32);
    let stack_trace = body.get("stack_trace").and_then(|v| v.as_str()).map(String::from);
    let recent_logs: Vec<String> = body.get("recent_logs")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let context = clawtex_core::ErrorContext {
        error_message,
        tool_name,
        hand_name,
        phase,
        agent_name,
        timestamp: chrono::Utc::now(),
        stack_trace,
        recent_logs,
    };

    match diagnoser.diagnose_error(&context) {
        Ok(report) => Ok(Json(json!(report))),
        Err(e) => {
            error!("Auto-diagnosis failed: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// GET /diagnose/:error_id — retrieve a stored diagnosis by ID
async fn diagnose_get_handler(
    State(state): State<AppState>,
    Path(error_id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let diagnoser = state.auto_diagnoser.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    match diagnoser.get_diagnosis(&error_id) {
        Ok(Some(report)) => Ok(Json(json!(report))),
        Ok(None) => Ok(Json(json!({"error": format!("Diagnosis '{}' not found", error_id)}))),
        Err(e) => {
            error!("Diagnosis lookup failed: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// GET /diagnose/recent — list recent diagnoses
async fn diagnose_recent_handler(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Result<Json<Value>, StatusCode> {
    let diagnoser = state.auto_diagnoser.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let limit = params.get("limit").and_then(|v| v.parse::<usize>().ok()).unwrap_or(20);

    match diagnoser.list_recent(limit) {
        Ok(reports) => Ok(Json(json!({
            "diagnoses": reports,
            "count": reports.len(),
        }))),
        Err(e) => {
            error!("Diagnosis list failed: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// GET /diagnose/stats — diagnosis statistics
async fn diagnose_stats_handler(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let diagnoser = state.auto_diagnoser.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let total = diagnoser.count().unwrap_or(0);
    let by_category = diagnoser.stats_by_category().unwrap_or_default();
    let known_count = diagnoser.get_common_issues().len();

    Ok(Json(json!({
        "total_diagnoses": total,
        "known_patterns": known_count,
        "by_category": by_category.iter().map(|(c, n)| json!({"category": c, "count": n})).collect::<Vec<_>>(),
    })))
}

/// GET /diagnose/known-issues — list all known issue patterns
async fn diagnose_known_issues_handler(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let diagnoser = state.auto_diagnoser.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let issues = diagnoser.get_common_issues();
    Ok(Json(json!({
        "known_issues": issues,
        "count": issues.len(),
    })))
}

// ── Observational Memory Endpoints ─────────────────────────────────────────

/// POST /memory/observe — compress conversation messages into an observation
async fn memory_observe(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let om = state.observational_memory.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let session_id = body.get("session_id")
        .and_then(|v| v.as_str())
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_string();

    let messages_val = body.get("messages")
        .and_then(|v| v.as_array())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let messages: Vec<clawtex_core::ConversationMessage> = messages_val
        .iter()
        .filter_map(|m| {
            let role = m.get("role")?.as_str()?.to_string();
            let content = m.get("content")?.as_str()?.to_string();
            let timestamp = m.get("timestamp").and_then(|t| t.as_str()).map(String::from);
            Some(clawtex_core::ConversationMessage { role, content, timestamp })
        })
        .collect();

    if messages.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    match om.observe(&session_id, &messages) {
        Ok(obs) => Ok(Json(json!(obs))),
        Err(e) => {
            error!("Observational memory observe failed: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// GET /memory/observations?query=X&limit=10 — search observations by keyword
async fn memory_observations(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Result<Json<Value>, StatusCode> {
    let om = state.observational_memory.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let query = params.get("query").map(|s| s.as_str()).unwrap_or("");
    let limit: usize = params.get("limit")
        .and_then(|l| l.parse().ok())
        .unwrap_or(10);

    match om.recall(query, limit) {
        Ok(observations) => Ok(Json(json!({
            "observations": observations,
            "count": observations.len(),
        }))),
        Err(e) => {
            error!("Observational memory recall failed: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// GET /memory/observations/recent?limit=5 — most recent observations
async fn memory_observations_recent(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Result<Json<Value>, StatusCode> {
    let om = state.observational_memory.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let limit: usize = params.get("limit")
        .and_then(|l| l.parse().ok())
        .unwrap_or(5);

    match om.recall_recent(limit) {
        Ok(observations) => Ok(Json(json!({
            "observations": observations,
            "count": observations.len(),
        }))),
        Err(e) => {
            error!("Observational memory recall_recent failed: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// GET /memory/observations/stats — observation statistics
async fn memory_observations_stats(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let om = state.observational_memory.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let count = om.count().unwrap_or(0);
    let tokens_saved = om.total_tokens_saved().unwrap_or(0);
    let avg_compression = om.avg_compression_ratio().unwrap_or(0.0);

    Ok(Json(json!({
        "count": count,
        "total_tokens_saved": tokens_saved,
        "avg_compression_ratio": avg_compression,
    })))
}

// ── Order Workflow Handlers ──────────────────────────────────────────────────

async fn orders_create(State(state): State<AppState>, Json(body): Json<Value>) -> Result<Json<Value>, StatusCode> {
    let wf = state.order_workflow.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let name = body.get("customer_name").and_then(|v| v.as_str()).ok_or(StatusCode::BAD_REQUEST)?;
    let email = body.get("customer_email").and_then(|v| v.as_str()).unwrap_or("");
    let tier = body.get("service_tier").and_then(|v| v.as_str()).unwrap_or("standard");
    match wf.create_order(name, email, tier) {
        Ok(order) => {
            if let Some(amount) = body.get("amount_usd").and_then(|v| v.as_f64()) { let _ = wf.set_amount(&order.id, amount); }
            if let Some(agent) = body.get("assigned_agent").and_then(|v| v.as_str()) { let _ = wf.assign_agent(&order.id, agent); }
            let refreshed = wf.get_order(&order.id).unwrap_or(Some(order));
            Ok(Json(json!({ "status": "created", "order": refreshed })))
        }
        Err(e) => { error!("Order create failed: {}", e); Err(StatusCode::INTERNAL_SERVER_ERROR) }
    }
}

async fn orders_list(State(state): State<AppState>, axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>) -> Result<Json<Value>, StatusCode> {
    let wf = state.order_workflow.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let orders = if let Some(status_str) = params.get("status") {
        let status = clawtex_core::OrderStatus::from_str_loose(status_str).ok_or(StatusCode::BAD_REQUEST)?;
        wf.list_by_status(status).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    } else {
        wf.list_all().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };
    Ok(Json(json!({ "orders": orders, "count": orders.len() })))
}

async fn orders_get(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<Value>, StatusCode> {
    let wf = state.order_workflow.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match wf.get_order(&id) {
        Ok(Some(order)) => Ok(Json(json!({ "order": order }))),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn orders_transition(State(state): State<AppState>, Path(id): Path<String>, Json(body): Json<Value>) -> Result<Json<Value>, StatusCode> {
    let wf = state.order_workflow.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let status_str = body.get("status").and_then(|v| v.as_str()).ok_or(StatusCode::BAD_REQUEST)?;
    let new_status = clawtex_core::OrderStatus::from_str_loose(status_str).ok_or(StatusCode::BAD_REQUEST)?;
    match wf.transition(&id, new_status) {
        Ok(order) => Ok(Json(json!({ "status": "transitioned", "order": order }))),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") { Err(StatusCode::NOT_FOUND) }
            else if msg.contains("Invalid transition") { Ok(Json(json!({ "error": msg }))) }
            else { Err(StatusCode::INTERNAL_SERVER_ERROR) }
        }
    }
}

async fn orders_pipeline(State(state): State<AppState>) -> Result<Json<Value>, StatusCode> {
    let wf = state.order_workflow.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match wf.pipeline_summary() {
        Ok(summary) => Ok(Json(json!(summary))),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn orders_overdue(State(state): State<AppState>, axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>) -> Result<Json<Value>, StatusCode> {
    let wf = state.order_workflow.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let sla = params.get("sla_hours").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    match wf.overdue_orders(sla) {
        Ok(orders) => Ok(Json(json!({ "overdue": orders, "count": orders.len() }))),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn orders_add_note(State(state): State<AppState>, Path(id): Path<String>, Json(body): Json<Value>) -> Result<Json<Value>, StatusCode> {
    let wf = state.order_workflow.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let note = body.get("note").and_then(|v| v.as_str()).ok_or(StatusCode::BAD_REQUEST)?;
    match wf.add_note(&id, note) {
        Ok(()) => Ok(Json(json!({ "status": "note_added" }))),
        Err(e) => { if e.to_string().contains("not found") { Err(StatusCode::NOT_FOUND) } else { Err(StatusCode::INTERNAL_SERVER_ERROR) } }
    }
}

fn dirs_home() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string())
}

/// Parse a /cron add command string into (Schedule, JobAction, name)
/// Formats:
///   "0 9 * * *" hand:freelancer "AI automation jobs"
///   every:3600 hand:lead "Find SaaS leads"
///   "*/30 * * * *" agent:master "check tasks"
///   "0 9 * * 1-5" shell "echo weekday"
fn parse_cron_add_command(input: &str) -> Option<(clawtex_core::Schedule, clawtex_core::JobAction, String)> {
    let input = input.trim();

    // Parse schedule: either "cron expr" in quotes or every:N
    let (schedule, rest) = if input.starts_with('"') {
        // Quoted cron expression
        let end_quote = input[1..].find('"')? + 1;
        let expr = &input[1..end_quote];
        let schedule = clawtex_core::Schedule::Cron { expr: expr.to_string() };
        (schedule, input[end_quote + 1..].trim())
    } else if input.starts_with("every:") {
        let space_idx = input.find(' ')?;
        let secs: u64 = input[6..space_idx].parse().ok()?;
        let schedule = clawtex_core::Schedule::Every { interval_secs: secs };
        (schedule, input[space_idx + 1..].trim())
    } else {
        return None;
    };

    // Parse action: hand:name, agent:name, or shell followed by quoted string
    let (action, _rest) = if rest.starts_with("hand:") {
        let after_prefix = &rest[5..];
        let space_idx = after_prefix.find(' ').unwrap_or(after_prefix.len());
        let hand_name = &after_prefix[..space_idx];
        let input_text = extract_quoted_or_rest(&after_prefix[space_idx..]);
        (clawtex_core::JobAction::Hand { hand_name: hand_name.to_string(), input: input_text.clone() }, input_text)
    } else if rest.starts_with("agent:") {
        let after_prefix = &rest[6..];
        let space_idx = after_prefix.find(' ').unwrap_or(after_prefix.len());
        let agent_name = &after_prefix[..space_idx];
        let prompt = extract_quoted_or_rest(&after_prefix[space_idx..]);
        (clawtex_core::JobAction::Agent { agent: agent_name.to_string(), prompt: prompt.clone() }, prompt)
    } else if rest.starts_with("shell ") {
        let cmd = extract_quoted_or_rest(&rest[5..]);
        (clawtex_core::JobAction::Shell { command: cmd.clone() }, cmd)
    } else {
        return None;
    };

    // Generate a name from the action
    let name = match &action {
        clawtex_core::JobAction::Hand { hand_name, .. } => format!("cron-hand-{}", hand_name),
        clawtex_core::JobAction::Agent { agent, .. } => format!("cron-agent-{}", agent),
        clawtex_core::JobAction::Shell { .. } => "cron-shell".to_string(),
        clawtex_core::JobAction::Notify { .. } => "cron-notify".to_string(),
    };

    Some((schedule, action, name))
}

/// Extract a quoted string or the remaining text
fn extract_quoted_or_rest(input: &str) -> String {
    let input = input.trim();
    if input.starts_with('"') {
        if let Some(end) = input[1..].find('"') {
            return input[1..end + 1].to_string();
        }
    }
    input.trim_matches('"').to_string()
}

// ── Networking API Handlers ────────────────────────────────────────────────

async fn networking_discovered(State(state): State<AppState>) -> Json<Value> {
    if let Some(ref rm) = state.route_manager {
        let routes = rm.all_routes().await;
        let nodes: Vec<Value> = routes
            .iter()
            .map(|r| {
                json!({
                    "name": r.node_name,
                    "url": r.url,
                    "layer": r.layer,
                    "latency_ms": r.latency_ms,
                    "cached": r.cached,
                })
            })
            .collect();
        Json(json!({ "nodes": nodes }))
    } else {
        Json(json!({ "nodes": [] }))
    }
}

async fn networking_routes(State(state): State<AppState>) -> Json<Value> {
    if let Some(ref rm) = state.route_manager {
        let routes = rm.all_routes().await;
        Json(json!({ "routes": routes }))
    } else {
        Json(json!({ "routes": [] }))
    }
}

async fn networking_status(State(state): State<AppState>) -> Json<Value> {
    if let Some(ref rm) = state.route_manager {
        let routes = rm.all_routes().await;
        let layer_counts: HashMap<String, usize> = routes.iter().fold(HashMap::new(), |mut acc, r| {
            // Use serde lowercase name for stable API contract
            let key = serde_json::to_value(&r.layer)
                .ok()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| format!("{}", r.layer));
            *acc.entry(key).or_insert(0) += 1;
            acc
        });
        Json(json!({
            "enabled": true,
            "discovery_backends": rm.discovery_count(),
            "transport_backends": rm.transport_count(),
            "known_routes": routes.len(),
            "layers": layer_counts,
        }))
    } else {
        Json(json!({
            "enabled": false,
            "discovery_backends": 0,
            "transport_backends": 0,
            "known_routes": 0,
            "layers": {},
        }))
    }
}

// ── Goals API Handlers ─────────────────────────────────────────────────────

async fn goals_list(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    let status_filter = params.get("status").and_then(|s| {
        Some(clawtex_core::goals::GoalStatus::from_str(s))
    });
    match store.list_goals(status_filter) {
        Ok(goals) => Json(json!({ "goals": goals })),
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_create(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Ok(Json(json!({ "error": "Goals store not available" }))),
    };
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let goal = clawtex_core::goals::Goal {
        id: id.clone(),
        title: body["title"].as_str().unwrap_or("").to_string(),
        category: body["category"].as_str().unwrap_or("").to_string(),
        description: body["description"].as_str().map(|s| s.to_string()),
        target_date: body["target_date"].as_str().map(|s| s.to_string()),
        status: clawtex_core::goals::GoalStatus::Active,
        context: body["context"].as_str().map(|s| s.to_string()),
        created_at: now.clone(),
        updated_at: now,
    };
    match store.create_goal(&goal) {
        Ok(()) => Ok(Json(json!({ "id": id, "goal": goal }))),
        Err(e) => Ok(Json(json!({ "error": format!("{}", e) }))),
    }
}

async fn goals_get(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    match store.get_goal(&id) {
        Ok(Some(goal)) => Json(json!({ "goal": goal })),
        Ok(None) => Json(json!({ "error": "Goal not found" })),
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_update(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    let status = body["status"].as_str().map(|s| clawtex_core::goals::GoalStatus::from_str(s));
    match store.update_goal(
        &id,
        body["title"].as_str(),
        status,
        body["description"].as_str(),
        body["context"].as_str(),
    ) {
        Ok(true) => Json(json!({ "updated": true })),
        Ok(false) => Json(json!({ "error": "Goal not found" })),
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_delete(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    match store.delete_goal(&id) {
        Ok(true) => Json(json!({ "deleted": true })),
        Ok(false) => Json(json!({ "error": "Goal not found" })),
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_progress(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    match store.get_goal_progress(&id) {
        Ok(Some(p)) => Json(json!({ "progress": p })),
        Ok(None) => Json(json!({ "error": "Goal not found" })),
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_today(State(state): State<AppState>) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    match store.get_today_tasks() {
        Ok(tasks) => Json(json!({ "tasks": tasks })),
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_active_summary(State(state): State<AppState>) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    match store.active_goals_summary() {
        Ok(summaries) => Json(json!({ "summaries": summaries })),
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_milestones_list(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    match store.list_milestones(&id) {
        Ok(ms) => Json(json!({ "milestones": ms })),
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_milestone_add(
    State(state): State<AppState>,
    axum::extract::Path(goal_id): axum::extract::Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    let id = uuid::Uuid::new_v4().to_string();
    let ms = clawtex_core::goals::Milestone {
        id: id.clone(),
        goal_id,
        title: body["title"].as_str().unwrap_or("").to_string(),
        due_date: body["due_date"].as_str().map(|s| s.to_string()),
        status: "pending".to_string(),
        sort_order: body["sort_order"].as_i64().unwrap_or(0) as i32,
        completed_at: None,
    };
    match store.add_milestone(&ms) {
        Ok(()) => Json(json!({ "id": id, "milestone": ms })),
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_milestone_toggle(
    State(state): State<AppState>,
    axum::extract::Path((_goal_id, ms_id)): axum::extract::Path<(String, String)>,
) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    match store.toggle_milestone(&ms_id) {
        Ok(Some(ms)) => Json(json!({ "milestone": ms })),
        Ok(None) => Json(json!({ "error": "Milestone not found" })),
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_recurring_list(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    match store.list_recurring_tasks(&id) {
        Ok(tasks) => Json(json!({ "tasks": tasks })),
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_recurring_add(
    State(state): State<AppState>,
    axum::extract::Path(goal_id): axum::extract::Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    let id = uuid::Uuid::new_v4().to_string();
    let task = clawtex_core::goals::RecurringTask {
        id: id.clone(),
        goal_id,
        title: body["title"].as_str().unwrap_or("").to_string(),
        cron_expr: body["cron_expr"].as_str().unwrap_or("0 9 * * *").to_string(),
        last_completed: None,
        streak_count: 0,
        enabled: true,
    };
    match store.add_recurring_task(&task) {
        Ok(()) => Json(json!({ "id": id, "task": task })),
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_recurring_complete(
    State(state): State<AppState>,
    axum::extract::Path((_goal_id, task_id)): axum::extract::Path<(String, String)>,
) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    match store.complete_recurring_task(&task_id) {
        Ok(Some(streak)) => Json(json!({ "completed": true, "streak": streak })),
        Ok(None) => Json(json!({ "error": "Task not found" })),
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_checkin_add(
    State(state): State<AppState>,
    axum::extract::Path(goal_id): axum::extract::Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    let id = uuid::Uuid::new_v4().to_string();
    let ci = clawtex_core::goals::CheckIn {
        id: id.clone(),
        goal_id,
        date: body["date"].as_str().unwrap_or(&chrono::Utc::now().format("%Y-%m-%d").to_string()).to_string(),
        mood: body["mood"].as_i64().unwrap_or(3) as i32,
        note: body["note"].as_str().map(|s| s.to_string()),
        ai_feedback: body["ai_feedback"].as_str().map(|s| s.to_string()),
    };
    match store.add_check_in(&ci) {
        Ok(()) => Json(json!({ "id": id, "check_in": ci })),
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_push_preview(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    let kind = params.get("type").map(|s| s.as_str()).unwrap_or("morning");
    let result = match kind {
        "evening" => clawtex_core::goals_push::evening_checkin(store),
        "weekly" => clawtex_core::goals_push::weekly_report(store),
        _ => clawtex_core::goals_push::morning_briefing(store),
    };
    match result {
        Ok(msg) if msg.is_empty() => Json(json!({ "message": null, "reason": "no active goals" })),
        Ok(msg) => Json(json!({ "type": kind, "message": msg })),
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_checkins_list(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    let limit = params.get("limit").and_then(|l| l.parse().ok()).unwrap_or(20);
    match store.list_check_ins(&id, limit) {
        Ok(cis) => Json(json!({ "check_ins": cis })),
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_mood_trend(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    let days = params.get("days").and_then(|d| d.parse().ok()).unwrap_or(30);
    match store.mood_trend(&id, days) {
        Ok(trend) => {
            let points: Vec<Value> = trend.iter().map(|(d, m)| json!({ "date": d, "mood": m })).collect();
            Json(json!({ "trend": points }))
        }
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_weekly_summary(
    State(state): State<AppState>,
) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    match store.weekly_summary() {
        Ok(ws) => Json(json!({ "summary": ws })),
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

async fn goals_global_mood(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Json<Value> {
    let store = match &state.goals_store {
        Some(s) => s,
        None => return Json(json!({ "error": "Goals store not available" })),
    };
    let days = params.get("days").and_then(|d| d.parse().ok()).unwrap_or(30);
    match store.global_mood_trend(days) {
        Ok(trend) => {
            let points: Vec<Value> = trend.iter().map(|(d, m)| json!({ "date": d, "avg_mood": m })).collect();
            Json(json!({ "trend": points }))
        }
        Err(e) => Json(json!({ "error": format!("{}", e) })),
    }
}

/// Detect ngrok public URL by querying its local API
async fn detect_ngrok_url() -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;
    let resp = client
        .get("http://127.0.0.1:4040/api/tunnels")
        .send()
        .await
        .ok()?;
    let json: serde_json::Value = resp.json().await.ok()?;
    json.pointer("/tunnels/0/public_url")
        .and_then(|v| v.as_str())
        .map(String::from)
}
