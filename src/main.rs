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
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

use clawtex_core::{
    AiCodeConfig, AgentRuntime, ApprovalGate, Channel, ChannelMessage, ChatMessage, ClusterRegistry,
    ClusterHub, ClusterWorker, ClusterConfig, WorkerConfig, TaskResultPayload,
    ComputerUseConfig, ConversationStore, CostTracker, CostSummary, CronStore,
    EmailConfig, EStop, EvalConfig, GatewayState, HandRegistry, HandRunner, JobAction, LlmRouter,
    MemoryCategory, MemoryConfig, MemoryStore, PhaseOutput, PrivacyConfig, PrivacyGuard,
    ProviderCircuitBreaker, BreakerConfig,
    RevenueTracker, RevenueSummary,
    RecoveryConfig, WorkerWatchdog,
    Schedule, Scheduler, SearchConfig,
    SecretManager, SecurityConfig, SkillRegistry, TaskQueue, TelegramChannel, TelegramConfig,
    ToolRegistry, TrajectoryLogger, TwitterConfig, BlogConfig, TrustLevel,
    SlackConfig, DiscordConfig, LineConfig, WhatsAppConfig,
};

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
    Daemon,
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
    started_at: Instant,
}

// ── Auth Middleware ────────────────────────────────────────────────────────────

async fn auth_middleware(
    State(state): State<AppState>,
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    // /health and /dashboard are always public (dashboard has its own query-param auth)
    let path = req.uri().path();
    if path == "/health" || path == "/dashboard" {
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

    // HTTP API calls don't have conversation history (stateless)
    match state
        .agent_runtime
        .run(&agent_name, prompt, &[], &state.llm_router, &state.tool_registry, None)
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

async fn cluster_status(State(state): State<AppState>) -> Json<Value> {
    let nodes = state.cluster.status().await;
    Json(json!({ "nodes": nodes }))
}

/// POST /cluster/register — worker self-registration
async fn cluster_register(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
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
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
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
    let worker_count = state.cluster.all_workers().len() as u64;
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
    let worker_count = state.cluster.all_workers().len() as u64;
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
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Result<Json<Value>, StatusCode> {
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
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
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
    let today = ct.today_total().unwrap_or(CostSummary { group: "today".into(), total_tokens: 0, total_cost_usd: 0.0, call_count: 0 });
    let by_provider = ct.by_provider(7).unwrap_or_default();
    let by_agent = ct.by_agent(7).unwrap_or_default();
    let by_day = ct.by_day(7).unwrap_or_default();
    Json(json!({
        "today": { "tokens": today.total_tokens, "cost_usd": today.total_cost_usd, "calls": today.call_count },
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

            if text == "/history" {
                let count = state.conversations.message_count(&chat_id).await;
                let reply = format!("Current conversation: {} messages ({} turns)", count, count / 2);
                let _ = telegram.send(&chat_id, &reply).await;
                return;
            }

            if text == "/help" || text == "/start" {
                let reply = "\
Clawtex Bot Commands:

/help — Show this help
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
/history — Conversation message count
/clear — Clear conversation memory
/estop — Emergency stop all agents
/resume — Resume after e-stop

Any other message will be processed by the AI agent.";
                let _ = telegram.send(&chat_id, reply).await;
                return;
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
                    let _ = telegram.send(&chat_id, &format!(
                        "Running hand '{}' ({} phases)...\nPhases: {}\nThis may take a while.",
                        hand.name, total_phases, phase_names.join(" → ")
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
                                let _ = telegram.send(&chat_id, &format!(
                                    "✅ Phase {}/{}: {} done ({} tool calls)\n\n{}",
                                    i + 1, total_phases, phase_name, output.tool_calls, preview
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

                    let status = if all_ok { "completed" } else { "partially completed" };
                    let summary = format!(
                        "Hand '{}' {} ({}/{} phases, {:.1}s)\n\n{}",
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

            // Handle approval responses
            if text.starts_with("/approve ") {
                let id = &text[9..];
                if state.approval_gate.respond(id, true).await {
                    let _ = telegram.send(&chat_id, "Approved.").await;
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
                            reply.push_str(&format!("\nToday: {} tokens, ${:.4}, {} calls", today.total_tokens, today.total_cost_usd, today.call_count));
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

            // Combine extra context
            let extra_context = format!("{}{}", memory_ctx, skills_ctx);
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

        let config = WorkerConfig {
            hub_url: hub,
            node_name: node_name.clone(),
            capabilities,
            device_type,
            port,
        };

        info!("Worker '{}' connecting to hub at {}", node_name, config.hub_url);
        let worker = ClusterWorker::new(config, tool_registry);
        return worker.start_server().await;
        }
        _ => {} // Daemon, Run, Interactive, Config, Status — handled below
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

    // Circuit Breaker — auto-trips failing providers (attached before Arc wrap)
    let circuit_breaker = Arc::new(ProviderCircuitBreaker::new(BreakerConfig::default()));
    llm_router.set_circuit_breaker(circuit_breaker.clone());
    info!("CircuitBreaker attached to LlmRouter");

    let llm_router = Arc::new(llm_router);
    let task_queue = Arc::new(TaskQueue::new(&db_path).await?);
    let mut agent_runtime = AgentRuntime::new(&config_path)?;
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

    // Register delegate tool (needs Arcs, so done after component init)
    let subagent_tools = Arc::new(ToolRegistry::new(SecurityConfig::default())); // subagent gets base tools only (no delegate to prevent loops)
    tool_registry.register(Box::new(clawtex_core::tools::delegate::DelegateTool::new(
        agent_runtime.clone(),
        llm_router.clone(),
        subagent_tools.clone(),
    )));

    // Register delegate_to_provider tool (dynamic provider routing for multi-agent coordination)
    tool_registry.register(Box::new(clawtex_core::tools::delegate_to_provider::DelegateToProviderTool::new(
        agent_runtime.clone(),
        llm_router.clone(),
        subagent_tools,
    )));

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

    // Auto-detect ngrok public URL
    let public_url = detect_ngrok_url().await;
    if let Some(ref url) = public_url {
        info!("Detected ngrok tunnel: {}", url);
    }

    // ── Cron Scheduler ─────────────────────────────────────────────
    let cron_store = Arc::new(CronStore::new(&db_path)?);
    let scheduler = Arc::new(Scheduler::new(cron_store)?);

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
    }

    // Reuse the ClusterHub created earlier for agent_runtime dispatch

    let state = AppState {
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
        started_at: Instant::now(),
    };
    {
        let hub = cluster_hub.clone();
        tokio::spawn(async move { hub.staleness_loop().await });
    }

    // Start Telegram channel if configured
    if let Some(tg_config) = app_config.telegram {
        if tg_config.bot_token.is_empty() || tg_config.bot_token.starts_with("YOUR_") {
            warn!("Telegram bot_token not set, skipping Telegram");
        } else {
            let telegram = Arc::new(TelegramChannel::new(tg_config));
            let (tx, rx) = mpsc::channel::<ChannelMessage>(100);

            let tg_listen = telegram.clone();
            tokio::spawn(async move {
                if let Err(e) = tg_listen.listen(tx).await {
                    error!("Telegram listener error: {}", e);
                }
            });

            // Wire up approval gate notifier — sends approval requests to Telegram.
            // Uses a shared last_chat_id that gets updated from incoming messages.
            let last_chat_id: Arc<tokio::sync::RwLock<Option<String>>> = Arc::new(tokio::sync::RwLock::new(None));
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
        tokio::spawn(async move {
            let executor: clawtex_core::cron::JobExecutor = Arc::new(move |action| {
                let s = executor_state.clone();
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
                            // TODO: send via Telegram when channel ref is available
                            info!("Cron notify [{}]: {}", chat_id, message);
                            format!("Notified: {}", message)
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
            scheduler.run(executor).await;
        });
        info!("Cron scheduler started");
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

    // ── Self-Evolution Modules ───────────────────────────────────────────────
    let clawtex_dir = dirs::home_dir()
        .map(|h| h.join(".clawtex"))
        .unwrap_or_else(|| std::path::PathBuf::from(".clawtex"));

    // Trajectory Logger — records every agent run for analysis
    let trajectory_logger = match TrajectoryLogger::new(
        clawtex_dir.join("trajectories.db").to_str().unwrap_or("trajectories.db")
    ) {
        Ok(tl) => {
            info!("TrajectoryLogger initialized: {:?}", clawtex_dir.join("trajectories.db"));
            Some(Arc::new(tl))
        }
        Err(e) => {
            warn!("Failed to initialize TrajectoryLogger: {}", e);
            None
        }
    };

    // Worker Watchdog — monitors workers, auto-restarts via SSH
    let mut watchdog = WorkerWatchdog::with_defaults();
    // Also add ayaneo
    watchdog.add_worker(RecoveryConfig::new(
        "ayaneo",
        r#"ssh m4932@192.168.1.117 "wmic process where \"name='python.exe'\" call terminate >nul 2>&1 & cd /d C:\Users\m4932\worker & python worker.py""#,
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
        .route("/tools", get(tools_list))
        .route("/hands", get(hands_list))
        .route("/hand/:name/run", post(hand_run))
        .route("/workspace/files", get(workspace_files))
        .route("/costs", get(costs_summary))
        .route("/revenue", get(revenue_summary))
        .route("/dashboard", get(dashboard))
        // E-Stop endpoints
        .route("/estop", post(estop_activate))
        .route("/estop", axum::routing::delete(estop_reset))
        .route("/estop", get(estop_status))
        .with_state(state)
        // Gateway streaming endpoints (separate state)
        .route("/stream/agent/:name", get(clawtex_core::gateway::sse_agent))
        .route("/ws/agent/:name", get(clawtex_core::gateway::ws_agent))
        .route("/agent/think", post(clawtex_core::gateway::agent_think))
        .route("/trajectories", get(clawtex_core::gateway::get_trajectories))
        .route("/trajectories/stats", get(clawtex_core::gateway::get_trajectory_stats))
        .route("/cluster/health", get(clawtex_core::gateway::get_cluster_health))
        .with_state(gateway_state)
        // Hub Bearer token auth — exempts /health and /dashboard
        .layer(axum::middleware::from_fn_with_state(auth_state, auth_middleware));

    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr).await?;
    info!("Listening on http://{}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            info!("Received Ctrl+C, shutting down gracefully...");
        })
        .await?;
    info!("Daemon stopped.");
    Ok(())
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
