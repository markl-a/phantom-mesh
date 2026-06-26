use std::path::PathBuf;

use axum::http::{HeaderValue, Method};
use axum::{
    extract::{Query, State},
    response::{IntoResponse, Redirect},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use tower_http::cors::CorsLayer;

use phantom_mesh::channels::telegram::TelegramBot;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();

    // Install the process-wide tool gate (HOME permission profile + project
    // trust) BEFORE any agent / HTTP / daemon surface starts. Non-interactive /
    // fail-closed — this daemon has no terminal to prompt. Without this the
    // `phantom-mesh` daemon would run tools ungated even when HOME policy denies.
    phantom_mesh::tool_gate::install(false);

    // D28: lossily decode argv (args_os) so a stray non-UTF8 byte degrades to
    // U+FFFD instead of panicking the daemon (`std::env::args()` unwraps).
    let args: Vec<String> = std::env::args_os()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    let mut host = "0.0.0.0".to_string();
    let mut port: u16 = 7878;
    let mut config_path: Option<PathBuf> = None;
    let mut session_id: String = "daemon".to_string();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--host" => {
                i += 1;
                host = args.get(i).cloned().unwrap_or(host);
            }
            "--port" => {
                i += 1;
                port = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(port);
            }
            "--config" => {
                i += 1;
                config_path = args.get(i).map(PathBuf::from);
            }
            "--session" => {
                i += 1;
                session_id = args.get(i).cloned().unwrap_or(session_id);
            }
            "daemon" => {}
            _ => {}
        }
        i += 1;
    }

    std::env::set_var("PHANTOM_SESSION", &session_id);

    let config_path = config_path.unwrap_or_else(|| {
        phantom_mesh::cli_config::phantom_data_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("agents.toml")
    });

    let mut app_state = phantom_mesh::AppState::new();

    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        app_state.load_config_toml(&content);
        tracing::info!("Loaded config from {}", config_path.display());
    } else {
        tracing::warn!("No config found at {}", config_path.display());
    }

    // P9: initialise workspace registry + resolver, and materialise the current cwd.
    match phantom_mesh::WorkspaceRegistry::open_default() {
        Ok(registry) => {
            let resolver = phantom_mesh::WorkspaceResolver::new(registry);
            if let Ok(cwd) = std::env::current_dir() {
                match resolver.resolve_or_create(&cwd).await {
                    Ok(ws) => tracing::info!(
                        workspace_id = %ws.id,
                        root = %ws.root.display(),
                        "workspace resolved"
                    ),
                    Err(e) => tracing::warn!("workspace resolve failed: {}", e),
                }
            }
            app_state.workspace_resolver = Some(resolver);
        }
        Err(e) => tracing::warn!("workspace registry unavailable: {}", e),
    }

    // P5: initialise persistent task queue and recover crash-interrupted tasks.
    match phantom_mesh::TaskStore::open_default() {
        Ok(store) => {
            let queue = phantom_mesh::TaskQueue::new(store);
            match queue.mark_interrupted().await {
                Ok(0) => {}
                Ok(n) => tracing::info!("marked {} interrupted task(s) as failed", n),
                Err(e) => tracing::warn!("interrupted-task sweep failed: {}", e),
            }
            app_state.task_queue = Some(queue);
            tracing::info!("task queue initialised");
        }
        Err(e) => tracing::warn!("task queue unavailable: {}", e),
    }

    // P15: initialise notification dispatcher.
    // OS channel is only available on non-Android platforms (requires dbus / NSNotificationCenter).
    let notifier = phantom_mesh::NotificationDispatcher::new();
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    notifier
        .add_channel(std::sync::Arc::new(
            phantom_mesh::notifications::channels::OsChannel,
        ))
        .await;
    std::sync::Arc::new(notifier.clone()).spawn_flush_loop();
    app_state.notifier = Some(notifier);
    tracing::info!("notification dispatcher initialised");

    let cors = CorsLayer::new()
        .allow_origin("http://localhost:5173".parse::<HeaderValue>().unwrap())
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(tower_http::cors::Any);

    // Start Telegram bot if configured
    if let Some(tg_cfg) = &app_state.telegram_config {
        let bot_token_env = tg_cfg.bot_token_env.clone();
        let allowed_users = tg_cfg.allowed_users.clone();
        let agent_name = tg_cfg.agent.clone();
        let notify_chat_id = tg_cfg
            .notification_chat_id
            .or_else(|| allowed_users.first().copied());
        let runtime = app_state.agent_runtime.clone();
        let conversations = app_state.conversations.clone();
        let _llm_router = app_state.llm_router.clone();
        let _tool_registry = app_state.tool_registry.clone();
        let cost_tracker = app_state.cost_tracker.clone();

        if let Ok(token) = std::env::var(&bot_token_env) {
            if !token.is_empty() {
                if allowed_users.is_empty() {
                    tracing::warn!("Telegram bot: no allowed_users configured — ALL users can interact with this bot");
                }
                tracing::info!("Starting Telegram bot...");
                let bot = std::sync::Arc::new(TelegramBot::new(token, allowed_users));

                // Attach Telegram as a notification channel if we have a chat to send to.
                if let (Some(chat_id), Some(notifier)) =
                    (notify_chat_id, app_state.notifier.clone())
                {
                    let ch = std::sync::Arc::new(
                        phantom_mesh::notifications::channels::TelegramChannel::new(
                            bot.clone(),
                            chat_id,
                        ),
                    );
                    notifier.add_channel(ch).await;
                    tracing::info!(
                        "Telegram notification channel attached (chat_id={})",
                        chat_id
                    );
                }

                let bot_poll = bot.clone();
                tokio::spawn(async move {
                    let mut offset: i64 = 0;
                    loop {
                        match bot_poll.poll_updates(offset).await {
                            Ok(updates) => {
                                for (chat_id, user_id, text, update_id) in updates {
                                    if !bot_poll.is_user_allowed(user_id) {
                                        offset = offset.max(update_id + 1);
                                        continue;
                                    }
                                    let history =
                                        conversations.get_history(&format!("tg:{}", chat_id)).await;
                                    let extra =
                                        format!("You are responding via Telegram. Be concise.");
                                    let result = runtime
                                        .run_tracked(
                                            &agent_name,
                                            &text,
                                            &history,
                                            Some(&extra),
                                            &cost_tracker,
                                        )
                                        .await;
                                    let reply = match result {
                                        Ok(r) => r.output,
                                        Err(e) => format!("Error: {}", e),
                                    };
                                    if let Err(e) = bot_poll.send_message(chat_id, &reply).await {
                                        tracing::warn!(chat_id = chat_id, "Telegram send_message failed (user sees no reply): {}", e);
                                    }
                                    offset = offset.max(update_id + 1);
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Telegram poll error: {}", e);
                                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                            }
                        }
                    }
                });
            } else {
                tracing::warn!("Telegram bot_token_env '{}' is empty", bot_token_env);
            }
        }
    }

    let router = build_router(app_state, cors);

    tracing::info!("Phantom Mesh daemon listening on {}:{}", host, port);
    phantom_mesh::start_http_server(&host, port, router).await
}

fn build_router(state: phantom_mesh::AppState, cors: CorsLayer) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/version", get(api_version))
        .route("/api/dashboard/status", get(dashboard_status))
        .route("/api/providers/health", get(provider_health))
        .route("/cluster/status", get(cluster_status))
        .route("/cluster/workers", get(cluster_workers))
        .route("/cluster/scores", get(cluster_scores))
        .route("/costs", get(costs))
        .route("/tools", get(tools_list))
        .route("/hands", get(hands_list))
        .route("/task/history", get(task_history))
        .route("/memory/observations", get(memory_observations))
        .route("/memory/observations/stats", get(memory_stats))
        .route("/audit", get(audit_log))
        .route("/agent/:name/run", post(agent_run))
        .route("/agent/:name/run-async", post(agent_run_async))
        .route("/conversations/history", get(conversation_history))
        .route("/conversations/list", get(conversations_list))
        .route(
            "/conversations/:chat_id/history",
            get(conversation_history_by_id),
        )
        .route("/conversations/:chat_id/reset", post(conversation_reset))
        .route("/scan/hardware", get(scan_hardware))
        .route("/scan/credentials", get(scan_credentials))
        .route("/oauth/google", get(oauth_google_start))
        .route("/oauth/apple", get(oauth_apple_start))
        .route("/oauth/apple/available", get(oauth_apple_available))
        .route("/oauth/callback", get(oauth_callback))
        .route("/callback", get(oauth_callback))
        .route("/oauth/result", get(oauth_result))
        // ── Cluster RPC endpoints ──────────────────────────────────────────
        .route("/rpc/ping", post(rpc_ping))
        .route("/rpc/peers", get(rpc_peers))
        .route("/rpc/task/assign", post(rpc_task_assign))
        .route("/rpc/task/status/:job_id", get(rpc_task_status))
        // ── Workspaces (P9) ───────────────────────────────────────────────
        .route("/workspaces", get(workspaces_list))
        .route("/workspaces/current", get(workspaces_current))
        .route("/workspaces/:id/name", post(workspaces_rename))
        .route("/workspaces/:id/tags", post(workspaces_add_tag))
        // ── Tasks (P5) ────────────────────────────────────────────────────
        .route("/tasks", get(tasks_list))
        .route("/tasks/:id", get(tasks_get))
        .route("/tasks/:id/stream", get(tasks_stream))
        .route("/tasks/:id/cancel", post(tasks_cancel))
        .route("/tasks/:id/resume", post(tasks_resume))
        .with_state(state)
        .layer(cors)
}

// ── Handlers ───────────────────────────────────────────────────────────────

async fn health(State(state): State<phantom_mesh::AppState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "service": "phantom-mesh",
        "mode": "daemon",
        "uptime_seconds": state.started_at.elapsed().as_secs(),
    }))
}

async fn api_version() -> Json<Value> {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "build_date": env!("CARGO_PKG_VERSION"),
        "commit": option_env!("GIT_COMMIT_HASH").unwrap_or("unknown"),
    }))
}

async fn dashboard_status(State(state): State<phantom_mesh::AppState>) -> Json<Value> {
    let cluster_nodes = state.cluster_manager.status().await.len();
    Json(json!({
        "tools_count": state.tool_registry.names().len(),
        "hands_count": state.hands.names().len(),
        "cluster_nodes": cluster_nodes,
        "uptime_seconds": state.started_at.elapsed().as_secs(),
    }))
}

async fn provider_health(State(state): State<phantom_mesh::AppState>) -> Json<Value> {
    Json(json!({ "providers": state.llm_router.inner().health_summary() }))
}

async fn cluster_status(State(state): State<phantom_mesh::AppState>) -> Json<Value> {
    let nodes = state.cluster_manager.status().await;
    Json(json!({
        "node_count": nodes.len(),
        "nodes": nodes.iter().map(|n| json!({
            "name": n.name, "url": n.url,
            "online": n.online, "last_seen": n.last_seen,
        })).collect::<Vec<_>>(),
    }))
}

async fn cluster_workers() -> Json<Value> {
    Json(json!({ "workers": [] }))
}
async fn cluster_scores() -> Json<Value> {
    Json(json!({ "scores": [] }))
}
async fn costs(State(state): State<phantom_mesh::AppState>) -> Json<Value> {
    Json(state.cost_tracker.summary().await)
}
async fn task_history() -> Json<Value> {
    Json(json!({ "tasks": [] }))
}
async fn memory_observations() -> Json<Value> {
    Json(json!({ "observations": [] }))
}
async fn memory_stats() -> Json<Value> {
    Json(json!({ "total_observations": 0 }))
}
async fn audit_log() -> Json<Value> {
    Json(json!({ "entries": [] }))
}

async fn tools_list(State(state): State<phantom_mesh::AppState>) -> Json<Value> {
    Json(json!({ "tools": state.tool_registry.names() }))
}

async fn hands_list(State(state): State<phantom_mesh::AppState>) -> Json<Value> {
    Json(json!({ "hands": state.hands.names() }))
}

// ── Conversations ──────────────────────────────────────────────────────────

async fn conversation_history(State(state): State<phantom_mesh::AppState>) -> Json<Value> {
    let history = state.conversations.get_history("daemon").await;
    let messages: Vec<Value> = history
        .iter()
        .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
        .collect();
    Json(json!({ "messages": messages }))
}

async fn conversations_list(_state: State<phantom_mesh::AppState>) -> Json<Value> {
    // Enumerate .jsonl files on disk — each file is one conversation
    let dir = phantom_mesh::cli_config::phantom_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("conversations");
    let ids: Vec<String> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let name = e.file_name().into_string().ok()?;
                    name.strip_suffix(".jsonl").map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    Json(json!({ "conversations": ids }))
}

async fn conversation_history_by_id(
    State(state): State<phantom_mesh::AppState>,
    axum::extract::Path(chat_id): axum::extract::Path<String>,
) -> Json<Value> {
    let history = state.conversations.get_history(&chat_id).await;
    let messages: Vec<Value> = history
        .iter()
        .map(|m| json!({ "role": m.role, "content": m.content }))
        .collect();
    Json(json!({ "chat_id": chat_id, "messages": messages }))
}

async fn conversation_reset(
    State(state): State<phantom_mesh::AppState>,
    axum::extract::Path(chat_id): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // T7b T13-N1 follow-up: HMAC gate.
    if let Err((code, json)) =
        phantom_mesh::auth_gate::require_cluster_auth(&state.cluster_manager, &headers, &body)
    {
        return (code, json).into_response();
    }
    // Delete the conversation file directly
    let path = phantom_mesh::cli_config::phantom_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("conversations")
        .join(format!(
            "{}.jsonl",
            chat_id.replace(
                |c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != ':',
                "_"
            )
        ));
    let deleted = path.exists();
    let _ = std::fs::remove_file(&path);
    // In-memory cache will be stale until the process restarts; disk is source of truth on reload.
    Json(json!({ "chat_id": chat_id, "reset": deleted })).into_response()
}

// ── Hardware Scan ──────────────────────────────────────────────────────────

async fn scan_hardware() -> Json<Value> {
    let result = phantom_mesh::hardware::scan().await;
    Json(serde_json::to_value(result).unwrap_or_default())
}

async fn scan_credentials() -> Json<Value> {
    let creds = phantom_mesh::providers::credential_scanner::scan_all().await;
    let infos: Vec<_> = creds.iter().map(|c| c.to_frontend_info()).collect();
    Json(serde_json::to_value(infos).unwrap_or_default())
}

// ── OAuth Handlers ─────────────────────────────────────────────────────────

async fn oauth_google_start() -> impl IntoResponse {
    let url = phantom_mesh::oauth::google_start_url(7878);
    Redirect::temporary(&url)
}

async fn oauth_apple_start() -> impl IntoResponse {
    match phantom_mesh::oauth::apple_start_url(7878) {
        Ok(url) => Redirect::temporary(&url).into_response(),
        Err(e) => axum::response::Html(format!(
            "<html><body style='background:#1a1a2e;color:#fff;font-family:system-ui;padding:40px'>\
             <h2>Apple 登入尚未設定</h2><p>{}</p>\
             <p style='color:#888;margin-top:20px'>需要建立 <code>~/.phantom-mesh/apple-auth.json</code>：</p>\
             <pre style='background:#111;padding:16px;border-radius:8px;font-size:13px'>{}</pre>\
             <p style='color:#888'>然後重啟 daemon。</p>\
             <a href='http://localhost:5173' style='color:#6c63ff'>← 返回</a></body></html>",
            e,
            r#"{
  "client_id": "ai.phantommesh.auth",
  "team_id": "YOUR_TEAM_ID",
  "key_id": "YOUR_KEY_ID",
  "p8_path": "/path/to/AuthKey.p8"
}"#
        )).into_response(),
    }
}

async fn oauth_apple_available() -> Json<Value> {
    Json(json!({ "available": phantom_mesh::oauth::apple_available() }))
}

async fn oauth_callback(Query(params): Query<HashMap<String, String>>) -> impl IntoResponse {
    let code = params.get("code").cloned().unwrap_or_default();
    let state = params.get("state").cloned().unwrap_or_default();
    let error = params.get("error").cloned().unwrap_or_default();

    if !error.is_empty() {
        return axum::response::Html(format!(
            "<html><body style='background:#1a1a2e;color:#fff;font-family:system-ui;padding:40px'>\
             <h2>登入失敗</h2><p>{}</p>\
             <a href='http://localhost:5173' style='color:#6c63ff'>← 返回</a></body></html>",
            error
        ))
        .into_response();
    }

    match phantom_mesh::oauth::handle_callback(&code, &state).await {
        Ok(redirect_url) => Redirect::temporary(&redirect_url).into_response(),
        Err(e) => axum::response::Html(format!(
            "<html><body style='background:#1a1a2e;color:#fff;font-family:system-ui;padding:40px'>\
             <h2>登入失敗</h2><p>{}</p>\
             <a href='http://localhost:5173' style='color:#6c63ff'>← 返回</a></body></html>",
            e
        ))
        .into_response(),
    }
}

async fn oauth_result() -> Json<Value> {
    match phantom_mesh::oauth::get_result() {
        Some(Ok(identity)) => Json(json!({ "ok": true, "identity": identity })),
        Some(Err(e)) => Json(json!({ "ok": false, "error": e })),
        None => Json(json!({ "ok": false, "error": "no result yet" })),
    }
}

// ── Agent Handler ──────────────────────────────────────────────────────────

async fn agent_run(
    State(state): State<phantom_mesh::AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // T7b T13-N1 CRITICAL: HMAC gate on daemon /agent/:name/run.
    if let Err((code, json)) =
        phantom_mesh::auth_gate::require_cluster_auth(&state.cluster_manager, &headers, &body)
    {
        return (code, json).into_response();
    }
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("malformed body: {e}") })),
            )
                .into_response();
        }
    };
    let prompt = parsed["prompt"].as_str().unwrap_or("").to_string();
    let default_session = std::env::var("PHANTOM_SESSION").unwrap_or_else(|_| "daemon".into());
    let chat_id = parsed["chat_id"]
        .as_str()
        .unwrap_or(&default_session)
        .to_string();

    let (task_id, workspace_id) = create_agent_task(&state, &name, &prompt).await;
    let run_outcome = run_and_finalize_agent_task(&state, &name, &prompt, &chat_id, task_id).await;

    match run_outcome {
        Ok(result) => Json(json!({
            "agent": name,
            "output": result.output,
            "tool_calls": result.tool_calls_made,
            "turns": result.turns,
            "cost_usd": result.cost_delta_usd,
            "elapsed": result.elapsed_secs,
            "task_id": task_id.map(|u| u.to_string()),
            "workspace_id": workspace_id,
        }))
        .into_response(),
        Err(err_str) => Json(json!({
            "error": err_str,
            "task_id": task_id.map(|u| u.to_string()),
        }))
        .into_response(),
    }
}

/// POST /agent/:name/run-async — background variant of /agent/:name/run.
///
/// Creates the TaskRecord synchronously, spawns the agent loop in a detached
/// tokio task, and returns 202 immediately with the task_id. Clients use
/// `GET /tasks/:id` to poll or `GET /tasks/:id/stream` for SSE live updates.
async fn agent_run_async(
    State(state): State<phantom_mesh::AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // T7b T13-N1 CRITICAL: HMAC gate on daemon /agent/:name/run-async.
    if let Err((code, json)) =
        phantom_mesh::auth_gate::require_cluster_auth(&state.cluster_manager, &headers, &body)
    {
        return (code, json).into_response();
    }
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("malformed body: {e}") })),
            )
                .into_response();
        }
    };
    let prompt = parsed["prompt"].as_str().unwrap_or("").to_string();
    let default_session = std::env::var("PHANTOM_SESSION").unwrap_or_else(|_| "daemon".into());
    let chat_id = parsed["chat_id"]
        .as_str()
        .unwrap_or(&default_session)
        .to_string();

    let (task_id, workspace_id) = create_agent_task(&state, &name, &prompt).await;

    let Some(tid) = task_id else {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "task queue unavailable" })),
        )
            .into_response();
    };

    let spawn_state = state.clone();
    let spawn_name = name.clone();
    tokio::spawn(async move {
        let _ =
            run_and_finalize_agent_task(&spawn_state, &spawn_name, &prompt, &chat_id, Some(tid))
                .await;
    });

    (
        axum::http::StatusCode::ACCEPTED,
        Json(json!({
            "task_id": tid.to_string(),
            "workspace_id": workspace_id,
            "stream_url": format!("/tasks/{}/stream", tid),
        })),
    )
        .into_response()
}

/// Create a TaskRecord (Pending → Running) scoped to the current workspace.
/// Returns the task_id (None if task queue unavailable) plus the resolved
/// workspace_id.
async fn create_agent_task(
    state: &phantom_mesh::AppState,
    agent_name: &str,
    prompt: &str,
) -> (Option<uuid::Uuid>, String) {
    let workspace_id = match (&state.workspace_resolver, std::env::current_dir()) {
        (Some(resolver), Ok(cwd)) => resolver
            .resolve_or_create(&cwd)
            .await
            .ok()
            .map(|ws| ws.id.0)
            .unwrap_or_else(|| "default".into()),
        _ => "default".into(),
    };

    let task_id = if let Some(queue) = &state.task_queue {
        match queue.create(&workspace_id, agent_name, prompt).await {
            Ok(t) => {
                if let Err(e) = queue
                    .transition(t.task_id, pm_types::TaskStatus::Running, None)
                    .await
                {
                    tracing::warn!(task_id = %t.task_id, "task transition to Running failed: {}", e);
                }
                Some(t.task_id)
            }
            Err(e) => {
                tracing::warn!("task create failed: {}", e);
                None
            }
        }
    } else {
        None
    };

    (task_id, workspace_id)
}

/// Execute the agent loop, persist conversation, and finalize task state.
/// Returns the AgentResult on success or an error string on failure — the
/// TaskRecord is already transitioned to Completed/Failed before return.
async fn run_and_finalize_agent_task(
    state: &phantom_mesh::AppState,
    agent_name: &str,
    prompt: &str,
    chat_id: &str,
    task_id: Option<uuid::Uuid>,
) -> Result<phantom_mesh::AgentResult, String> {
    let history = state.conversations.get_history(chat_id).await;

    // Open a SessionWriter when we have a tracked task so each turn lands in
    // JSONL (claw-code pattern). Failure to open is non-fatal.
    let writer = match (&state.task_queue, task_id) {
        (Some(queue), Some(tid)) => match queue.get(tid).await {
            Ok(Some(record)) => phantom_mesh::tasks::SessionWriter::open(&record.workspace_id, tid)
                .await
                .map_err(|e| tracing::warn!("session writer open failed: {}", e))
                .ok(),
            _ => None,
        },
        _ => None,
    };

    let run_res = if let Some(w) = writer.as_ref() {
        state
            .agent_runtime
            .run_tracked_with_session(agent_name, prompt, &history, None, &state.cost_tracker, w)
            .await
    } else {
        state
            .agent_runtime
            .run_tracked(agent_name, prompt, &history, None, &state.cost_tracker)
            .await
    };

    match run_res {
        Ok(result) => {
            use phantom_mesh::providers::traits::ChatMessage;
            let user_msg = ChatMessage {
                role: "user".into(),
                content: prompt.to_string(),
                tool_calls: None,
            };
            let asst_msg = ChatMessage {
                role: "assistant".into(),
                content: result.output.clone(),
                tool_calls: None,
            };
            state
                .conversations
                .append(chat_id, user_msg, asst_msg)
                .await;

            if let (Some(queue), Some(tid)) = (&state.task_queue, task_id) {
                if let Err(e) = queue
                    .record_progress(tid, result.turns, result.cost_delta_usd)
                    .await
                {
                    tracing::warn!(task_id = %tid, "task record_progress failed: {}", e);
                }
                if let Err(e) = queue
                    .transition(tid, pm_types::TaskStatus::Completed, None)
                    .await
                {
                    tracing::warn!(task_id = %tid, "task transition to Completed failed: {}", e);
                }
            }
            notify_task_transition(
                state,
                task_id,
                agent_name,
                prompt,
                pm_types::TaskStatus::Completed,
                Some(&result.output),
            )
            .await;
            Ok(result)
        }
        Err(e) => {
            let err_str = e.to_string();
            if let (Some(queue), Some(tid)) = (&state.task_queue, task_id) {
                if let Err(qe) = queue
                    .transition(tid, pm_types::TaskStatus::Failed, Some(&err_str))
                    .await
                {
                    tracing::warn!(task_id = %tid, "task transition to Failed failed: {}", qe);
                }
            }
            notify_task_transition(
                state,
                task_id,
                agent_name,
                prompt,
                pm_types::TaskStatus::Failed,
                Some(&err_str),
            )
            .await;
            Err(err_str)
        }
    }
}

/// Emit a task-state notification. Silently skips if dispatcher or task_id is
/// absent; the dedupe cache protects against repeated sends.
async fn notify_task_transition(
    state: &phantom_mesh::AppState,
    task_id: Option<uuid::Uuid>,
    agent_name: &str,
    prompt: &str,
    status: pm_types::TaskStatus,
    detail: Option<&str>,
) {
    let Some(notifier) = &state.notifier else {
        return;
    };
    let Some(tid) = task_id else {
        return;
    };
    let workspace_id = state
        .task_queue
        .as_ref()
        .and_then(|_q| std::env::current_dir().ok())
        .and_then(|cwd| state.workspace_resolver.as_ref().map(|r| (r.clone(), cwd)))
        .map(|(r, cwd)| async move {
            r.resolve_or_create(&cwd)
                .await
                .map(|ws| ws.id.0)
                .unwrap_or_else(|_| "default".into())
        });
    let workspace_id = if let Some(fut) = workspace_id {
        fut.await
    } else {
        "default".into()
    };
    let title = match status {
        pm_types::TaskStatus::Completed => format!("✅ {} 完成", agent_name),
        pm_types::TaskStatus::Failed => format!("❌ {} 失敗", agent_name),
        pm_types::TaskStatus::Cancelled => format!("⏹ {} 已取消", agent_name),
        _ => format!("{} 狀態更新", agent_name),
    };
    let body_src = detail.unwrap_or("");
    let preview: String = prompt.chars().take(60).collect();
    let body = if body_src.is_empty() {
        format!("任務：{}", preview)
    } else {
        let detail_preview: String = body_src.chars().take(200).collect();
        format!("任務：{}\n\n{}", preview, detail_preview)
    };
    let n = pm_types::Notification::task_update(tid, workspace_id, status, title, body);
    notifier.notify(n).await;
}

// ── Cluster RPC Handlers ───────────────────────────────────────────────────

/// POST /rpc/ping — return this node's own PeerInfo (used by other nodes to ping us).
async fn rpc_ping(State(state): State<phantom_mesh::AppState>) -> Json<Value> {
    let node_name = state
        .cluster_manager
        .config
        .node_name
        .as_deref()
        .unwrap_or("phantom-mesh-node");
    Json(json!({
        "name": node_name,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": state.started_at.elapsed().as_secs(),
        "active_tasks": 0,
        "online": true,
    }))
}

/// GET /rpc/peers — list all configured peers with their latest status.
async fn rpc_peers(State(state): State<phantom_mesh::AppState>) -> Json<Value> {
    let peers = state.cluster_manager.status().await;
    Json(json!({
        "peers": peers.iter().map(|p| json!({
            "name": p.name,
            "host": p.url,  // rename url -> host for frontend compatibility
            "online": p.online,
            "active_tasks": p.active_tasks,
        })).collect::<Vec<_>>(),
        "self": {
            "name": state.cluster_manager.config.node_name.as_deref().unwrap_or("local"),
            "version": env!("CARGO_PKG_VERSION"),
        }
    }))
}

/// POST /rpc/task/assign — accept a task from another node and run it asynchronously.
///
/// Requires `X-Cluster-Auth: <SHA256(secret+body)>` header.
/// Body: `{ "agent": "master", "prompt": "..." }`
/// Returns 202 Accepted with `{ "job_id": "..." }` immediately.
async fn rpc_task_assign(
    State(state): State<phantom_mesh::AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl axum::response::IntoResponse {
    // Verify cluster auth
    let auth_token = headers
        .get("x-cluster-auth")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !state.cluster_manager.verify_auth(auth_token, &body) {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid or missing X-Cluster-Auth" })),
        )
            .into_response();
    }

    // Parse request body
    let req: phantom_mesh::mesh::TaskAssignRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("invalid request body: {}", e) })),
            )
                .into_response()
        }
    };

    // Create a persistent TaskRecord; the task_id doubles as the external job_id.
    let Some(queue) = state.task_queue.clone() else {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "task queue unavailable" })),
        )
            .into_response();
    };

    // ── T5 + C1: server-side capability enforcement (caps-aware) ──────────────
    // DRIFT fix: the hardened serve.rs handler (rpc_task_assign) enforces
    // `required_caps` against this worker's `worker_caps`; THIS shipped daemon
    // copy did not — a buggy or malicious orchestrator holding cluster_secret
    // could POST a task this worker can't satisfy and we'd spawn it anyway. Port
    // serve.rs verbatim: compute the effective enforce mode, then either Allow,
    // LogAndAllow (soft — log + continue, default), Reject (strict — 409, no
    // spawn), or ForwardTo (C1 — route to a capable peer when the env gate is on
    // and one exists). Reuses the shared `mesh::enforce_required_caps*` helpers
    // so client- and server-side cap matching stay identical.
    let local_caps = &state.cluster_manager.config.worker_caps;
    let mode = state.cluster_manager.config.effective_enforce_mode();
    let peers_snapshot = state.cluster_manager.peer_infos().await;
    let decision = if state.cluster_manager.config.node_name.is_none()
        && phantom_mesh::mesh::forward_on_caps_mismatch_enabled()
    {
        tracing::warn!(
            target: "phantom::dispatch::forward",
            "PHANTOM_FORWARD_ON_CAPS_MISMATCH=1 but node_name is unset; \
             refusing to forward (would emit a malformed chain). \
             Add [cluster].node_name to agents.toml."
        );
        phantom_mesh::mesh::enforce_required_caps(local_caps, &req.required_caps, mode)
    } else {
        phantom_mesh::mesh::enforce_required_caps_with_forwarding(
            local_caps,
            &req.required_caps,
            mode,
            &peers_snapshot,
        )
    };

    match decision {
        phantom_mesh::mesh::CapsDecision::Allow => { /* fall through to local run */ }
        phantom_mesh::mesh::CapsDecision::LogAndAllow { missing } => {
            tracing::warn!(
                target: "phantom::dispatch",
                ?missing,
                local = ?local_caps,
                required = ?req.required_caps,
                "capability_mismatch (soft mode): accepting task this worker may not be able to satisfy"
            );
        }
        phantom_mesh::mesh::CapsDecision::Reject { missing } => {
            // C1: distinguish "no peer would satisfy" from a plain mismatch.
            if phantom_mesh::mesh::forward_on_caps_mismatch_enabled() {
                let inventory: Vec<Value> = peers_snapshot
                    .iter()
                    .filter(|p| p.online)
                    .map(|p| json!({ "url": p.url, "capabilities": p.capabilities }))
                    .collect();
                return (
                    axum::http::StatusCode::CONFLICT,
                    Json(json!({
                        "error":           "no_peer_satisfies_caps",
                        "error_code":      "no_peer_satisfies_caps",
                        "required":        req.required_caps,
                        "local":           local_caps,
                        "missing":         missing,
                        "available_peers": inventory,
                    })),
                )
                    .into_response();
            }
            return (
                axum::http::StatusCode::CONFLICT,
                Json(json!({
                    "error":      "capability_mismatch",
                    "error_code": "capability_mismatch",
                    "required":   req.required_caps,
                    "local":      local_caps,
                    "missing":    missing,
                })),
            )
                .into_response();
        }
        phantom_mesh::mesh::CapsDecision::ForwardTo { peer, missing: _ } => {
            // C1 happy path: a downstream peer satisfies. HMAC-re-sign happens
            // inside `forward_task_to_capable_peer`.
            let my_node_name = state
                .cluster_manager
                .config
                .node_name
                .clone()
                .unwrap_or_default();
            let target_name = peer.name.clone();
            let target_url = peer.url.clone();
            match state
                .cluster_manager
                .forward_task_to_capable_peer(&req, &peer, &my_node_name)
                .await
            {
                Ok(remote_job_id) => {
                    return (
                        axum::http::StatusCode::ACCEPTED,
                        Json(json!({
                            "job_id":         remote_job_id,
                            "dispatched_to":  target_name,
                            "dispatched_url": target_url,
                            "forwarded":      true,
                        })),
                    )
                        .into_response();
                }
                Err(e) => {
                    tracing::warn!(
                        target: "phantom::dispatch::forward",
                        peer = %target_name,
                        url = %target_url,
                        error = %e,
                        "forward attempt failed; surfacing structured error to caller"
                    );
                    let (status, code) = match &e {
                        phantom_mesh::mesh::DispatchError::ForwardRejected { status, .. } => (
                            axum::http::StatusCode::from_u16(*status)
                                .unwrap_or(axum::http::StatusCode::BAD_GATEWAY),
                            "forward_rejected",
                        ),
                        phantom_mesh::mesh::DispatchError::HMACMismatch { .. } => {
                            (axum::http::StatusCode::BAD_GATEWAY, "forward_hmac_mismatch")
                        }
                        _ => (axum::http::StatusCode::BAD_GATEWAY, "forward_failed"),
                    };
                    return (
                        status,
                        Json(json!({
                            "error":       "forward_failed",
                            "error_code":  code,
                            "target_peer": target_name,
                            "detail":      e.to_string(),
                        })),
                    )
                        .into_response();
                }
            }
        }
    }

    // ── Best-effort, process-local at-most-once dedup (DISPATCH-MESH-DURABILITY
    //    §3.0) ────────────────────────────────────────────────────────────────
    // DRIFT fix: the hardened serve.rs handler already deduped re-sent assigns,
    // but THIS shipped daemon copy did not — a coordinator re-posting on its own
    // poll timeout, a forwarded retry carrying the same `idempotency_key`, or a
    // plain double-POST spawned the agent (and inserted a TaskQueue row) a second
    // time. Mirror serve.rs verbatim: mint the candidate `job_id` UP FRONT,
    // record it in the file-backed ledger (core/src/idempotency.rs) alongside the
    // dedup key, and short-circuit a duplicate with 200 + the ORIGINAL job_id so
    // the caller polls the same job (a job_id-less success is treated as a
    // DispatchError by `mesh::assign_task_to_peer*`). Best-effort: the ledger is
    // serialized by a process Mutex and fails open on FS errors — it collapses
    // the common retry storms but is not exactly-once.
    let idem_key = phantom_mesh::idempotency::task_assign_idem_key(
        req.idempotency_key.as_deref(),
        &req.agent,
        &req.prompt,
    );
    let job_id = uuid::Uuid::new_v4().to_string();
    let (decision, stored_job_id) =
        phantom_mesh::idempotency::check_and_record_value_default(
            &idem_key,
            "task_assign",
            Some(&job_id),
        );
    if let phantom_mesh::idempotency::Decision::Duplicate { first_seen } = decision {
        // STRICT at-most-once: a Duplicate ALWAYS returns and NEVER falls through
        // to spawn. Hand back the ORIGINAL job_id so the caller polls the same
        // job; 200 (not 202) distinguishes "already handled" from "new job
        // accepted". A rare legacy value-less ledger row yields a null job_id +
        // note rather than a re-spawn — at-most-once wins over availability.
        let original = stored_job_id
            .map(Value::from)
            .unwrap_or(Value::Null);
        let mut payload = json!({
            "job_id": original,
            "deduped": true,
            "first_seen": first_seen,
        });
        if payload["job_id"].is_null() {
            payload["note"] = json!(
                "deduped: original job_id unrecoverable (legacy value-less ledger entry); not re-spawning to preserve at-most-once"
            );
        }
        return (axum::http::StatusCode::OK, Json(payload)).into_response();
    }

    // First sighting: persist the durable row with the job_id minted above (so a
    // deduped retry stays resolvable by /rpc/task/status), then spawn.
    let workspace_id = match (&state.workspace_resolver, std::env::current_dir()) {
        (Some(resolver), Ok(cwd)) => resolver
            .resolve_or_create(&cwd)
            .await
            .ok()
            .map(|ws| ws.id.0)
            .unwrap_or_else(|| "default".into()),
        _ => "default".into(),
    };
    // `job_id` was minted via `Uuid::new_v4()` above, so the parse is infallible
    // here; the `?`-style fallback keeps the handler total rather than panicking.
    let Ok(task_uuid) = uuid::Uuid::parse_str(&job_id) else {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "failed to mint job id" })),
        )
            .into_response();
    };
    let task = match queue
        .create_with_id(task_uuid, &workspace_id, &req.agent, &req.prompt)
        .await
    {
        Ok(t) => t,
        Err(e) => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    if let Err(e) = queue
        .transition(task.task_id, pm_types::TaskStatus::Running, None)
        .await
    {
        tracing::warn!(task_id = %task.task_id, "task queue transition to Running failed: {}", e);
    }

    // Spawn the agent run in the background so the HTTP thread is not blocked.
    let runtime = state.agent_runtime.clone();
    let _llm_router = state.llm_router.clone();
    let _tool_registry = state.tool_registry.clone();
    let conversations = state.conversations.clone();
    let cost_tracker = state.cost_tracker.clone();
    let task_id = task.task_id;
    tokio::spawn(async move {
        let history = conversations.get_history("rpc").await;
        let result = runtime
            .run_tracked(&req.agent, &req.prompt, &history, None, &cost_tracker)
            .await;
        match result {
            Ok(r) => {
                if let Err(e) = queue
                    .record_progress(task_id, r.turns, r.cost_delta_usd)
                    .await
                {
                    tracing::warn!(%task_id, "task queue record_progress failed: {}", e);
                }
                if let Err(e) = queue
                    .transition(task_id, pm_types::TaskStatus::Completed, None)
                    .await
                {
                    tracing::warn!(%task_id, "task queue transition to Completed failed: {}", e);
                }
            }
            Err(e) => {
                let err = e.to_string();
                if let Err(te) = queue
                    .transition(task_id, pm_types::TaskStatus::Failed, Some(&err))
                    .await
                {
                    tracing::warn!(%task_id, "task queue transition to Failed failed: {}", te);
                }
            }
        }
    });

    (
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "job_id": job_id })),
    )
        .into_response()
}

/// GET /rpc/task/status/:job_id — poll the status of an async task.
///
/// job_id is a TaskRecord.task_id (UUID string). Returns the legacy shape
/// {id, status, output, error} for backwards compatibility with peer nodes.
async fn rpc_task_status(
    State(state): State<phantom_mesh::AppState>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    let Some(queue) = state.task_queue else {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "task queue unavailable" })),
        )
            .into_response();
    };
    let Ok(uuid) = uuid::Uuid::parse_str(&job_id) else {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({ "error": "job not found" })),
        )
            .into_response();
    };
    match queue.get(uuid).await {
        Ok(Some(t)) => {
            let legacy_status = match t.status {
                pm_types::TaskStatus::Completed => "done",
                pm_types::TaskStatus::Failed => "error",
                pm_types::TaskStatus::Cancelled => "cancelled",
                _ => "running",
            };
            Json(json!({
                "id": job_id,
                "status": legacy_status,
                "output": null,
                "error": t.error,
                "turns": t.turns,
                "cost_usd": t.cost_usd,
            }))
            .into_response()
        }
        Ok(None) => (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({ "error": "job not found" })),
        )
            .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ── Workspaces (P9) ────────────────────────────────────────────────────────

/// GET /workspaces — list all known workspaces, newest-used first.
async fn workspaces_list(
    State(state): State<phantom_mesh::AppState>,
) -> impl axum::response::IntoResponse {
    let Some(resolver) = state.workspace_resolver else {
        return Json(json!({ "workspaces": [] })).into_response();
    };
    match resolver.list().await {
        Ok(list) => Json(json!({ "workspaces": list })).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /workspaces/current — resolve (or create) the workspace for the daemon's cwd.
async fn workspaces_current(
    State(state): State<phantom_mesh::AppState>,
) -> impl axum::response::IntoResponse {
    let Some(resolver) = state.workspace_resolver else {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "workspace resolver unavailable" })),
        )
            .into_response();
    };
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };
    match resolver.resolve_or_create(&cwd).await {
        Ok(ws) => Json(json!(ws)).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct NameReq {
    name: Option<String>,
}

async fn workspaces_rename(
    State(state): State<phantom_mesh::AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // T7b T13-N1 follow-up: HMAC gate.
    if let Err((code, json)) =
        phantom_mesh::auth_gate::require_cluster_auth(&state.cluster_manager, &headers, &body)
    {
        return (code, json).into_response();
    }
    let parsed: NameReq = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("malformed body: {e}") })),
            )
                .into_response();
        }
    };
    let Some(resolver) = state.workspace_resolver else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let ws_id = pm_types::WorkspaceId(id);
    match resolver.registry().rename(&ws_id, parsed.name).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct TagReq {
    tag: String,
}

async fn workspaces_add_tag(
    State(state): State<phantom_mesh::AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // T7b T13-N1 follow-up: HMAC gate.
    if let Err((code, json)) =
        phantom_mesh::auth_gate::require_cluster_auth(&state.cluster_manager, &headers, &body)
    {
        return (code, json).into_response();
    }
    let parsed: TagReq = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("malformed body: {e}") })),
            )
                .into_response();
        }
    };
    let Some(resolver) = state.workspace_resolver else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let ws_id = pm_types::WorkspaceId(id);
    match resolver.registry().add_tag(&ws_id, &parsed.tag).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ── Tasks (P5) ─────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct TaskListQuery {
    workspace_id: Option<String>,
    status: Option<String>,
    limit: Option<usize>,
}

async fn tasks_list(
    State(state): State<phantom_mesh::AppState>,
    Query(q): Query<TaskListQuery>,
) -> impl axum::response::IntoResponse {
    let Some(queue) = state.task_queue else {
        return Json(json!({ "tasks": [] })).into_response();
    };
    let status_filter = q.status.as_deref().and_then(pm_types::TaskStatus::from_str);
    let limit = q.limit.unwrap_or(50);
    match queue
        .list(q.workspace_id.as_deref(), status_filter, limit)
        .await
    {
        Ok(tasks) => Json(json!({ "tasks": tasks })).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn tasks_get(
    State(state): State<phantom_mesh::AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    let Some(queue) = state.task_queue else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Ok(uuid) = uuid::Uuid::parse_str(&id) else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid uuid" })),
        )
            .into_response();
    };
    match queue.get(uuid).await {
        Ok(Some(t)) => Json(json!(t)).into_response(),
        Ok(None) => (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({ "error": "task not found" })),
        )
            .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /tasks/:id/stream — Server-Sent Events stream of task state changes.
///
/// Poll-based (500 ms) — emits an `update` event each time status / turns /
/// cost / error differs from the last snapshot. Closes once the task reaches
/// a terminal state (Completed / Failed / Cancelled).
async fn tasks_stream(
    State(state): State<phantom_mesh::AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use futures::stream;

    let Some(queue) = state.task_queue else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Ok(uuid) = uuid::Uuid::parse_str(&id) else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid uuid" })),
        )
            .into_response();
    };

    // Unfold-based poll loop. State carries the previous snapshot so we only
    // emit when something changed.
    let stream = stream::unfold(
        (
            queue,
            uuid,
            None::<(String, u32, f64, Option<String>)>,
            false,
        ),
        |(queue, uuid, last_key, done)| async move {
            if done {
                return None;
            }
            // Poll current record.
            let record = queue.get(uuid).await.ok().flatten();
            let Some(t) = record else {
                let ev = Event::default()
                    .event("error")
                    .data(r#"{"error":"task not found"}"#);
                return Some((
                    Ok::<_, std::convert::Infallible>(ev),
                    (queue, uuid, last_key, true),
                ));
            };

            let is_terminal = t.status.is_terminal();
            let key = (
                t.status.as_str().to_string(),
                t.turns,
                t.cost_usd,
                t.error.clone(),
            );
            let changed = last_key.as_ref() != Some(&key);

            // First iteration always emits so consumer gets an initial snapshot.
            if changed || last_key.is_none() {
                let payload = serde_json::json!({
                    "task_id": t.task_id,
                    "status": t.status,
                    "turns": t.turns,
                    "cost_usd": t.cost_usd,
                    "error": t.error,
                    "finished_at": t.finished_at,
                });
                let ev = Event::default().event("update").data(payload.to_string());
                let next_done = is_terminal;
                return Some((Ok(ev), (queue, uuid, Some(key), next_done)));
            }

            // No change yet — sleep and loop.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            // Emit a comment to keep the connection alive through proxies.
            let ev = Event::default().comment("heartbeat");
            Some((Ok(ev), (queue, uuid, last_key, false)))
        },
    );

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// POST /tasks/:id/resume — fork a new task that inherits the prior task's
/// session JSONL (with Session Repair applied) as its conversational history.
/// Returns 202 + the new task_id; the agent run happens in the background.
async fn tasks_resume(
    State(state): State<phantom_mesh::AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // T7b T13-N1 follow-up: HMAC gate.
    if let Err((code, json)) =
        phantom_mesh::auth_gate::require_cluster_auth(&state.cluster_manager, &headers, &body)
    {
        return (code, json).into_response();
    }
    let Some(queue) = state.task_queue.clone() else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Ok(parent_uuid) = uuid::Uuid::parse_str(&id) else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid uuid" })),
        )
            .into_response();
    };
    let parent = match queue.get(parent_uuid).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                Json(json!({ "error": "task not found" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    // Replay + repair the prior session. Empty session is allowed (fresh fork).
    let repaired =
        match phantom_mesh::tasks::load_and_repair(&parent.workspace_id, parent_uuid).await {
            Ok(r) => r,
            Err(e) => {
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": format!("session load: {}", e) })),
                )
                    .into_response();
            }
        };

    // Materialise the repaired session as a ChatMessage history that the agent
    // loop can consume. Tool calls / results are flattened to text turns
    // (sufficient for re-context; the agent will issue fresh tool calls).
    use phantom_mesh::providers::traits::ChatMessage;
    let mut history: Vec<ChatMessage> = Vec::new();
    let mut original_prompt: Option<String> = None;
    for entry in &repaired.entries {
        match entry {
            pm_types::SessionEntry::User { content, .. } => {
                if original_prompt.is_none() {
                    original_prompt = Some(content.clone());
                }
                history.push(ChatMessage {
                    role: "user".into(),
                    content: content.clone(),
                    tool_calls: None,
                });
            }
            pm_types::SessionEntry::Assistant { content, .. } => {
                history.push(ChatMessage {
                    role: "assistant".into(),
                    content: content.clone(),
                    tool_calls: None,
                });
            }
            pm_types::SessionEntry::ToolCall { name, args, .. } => {
                history.push(ChatMessage {
                    role: "assistant".into(),
                    content: format!("[tool_call] {}({})", name, args),
                    tool_calls: None,
                });
            }
            pm_types::SessionEntry::ToolResult {
                output, synthetic, ..
            } => {
                let prefix = if *synthetic {
                    "[tool_result, synthetic]"
                } else {
                    "[tool_result]"
                };
                history.push(ChatMessage {
                    role: "tool".into(),
                    content: format!("{} {}", prefix, output),
                    tool_calls: None,
                });
            }
            pm_types::SessionEntry::System { content, .. } => {
                history.push(ChatMessage {
                    role: "system".into(),
                    content: content.clone(),
                    tool_calls: None,
                });
            }
        }
    }

    let prompt = original_prompt.unwrap_or_else(|| parent.prompt.clone());
    let agent_name = parent.agent_name.clone();
    let chat_id = format!("resume:{}", parent_uuid);

    // Create a child task scoped to the same workspace. We bypass
    // create_agent_task so we can set parent_task_id correctly.
    let mut child = pm_types::TaskRecord::new(
        parent.workspace_id.clone(),
        agent_name.clone(),
        prompt.clone(),
    );
    child.parent_task_id = Some(parent_uuid);
    if let Err(e) = queue.store().insert(&child).await {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("create child: {}", e) })),
        )
            .into_response();
    }
    if let Err(e) = queue
        .transition(child.task_id, pm_types::TaskStatus::Running, None)
        .await
    {
        tracing::warn!(task_id = %child.task_id, "child task transition to Running failed: {}", e);
    }

    let child_id = child.task_id;
    let spawn_state = state.clone();
    let spawn_history = history.clone();
    tokio::spawn(async move {
        let result = match (
            &spawn_state.task_queue,
            spawn_state.workspace_resolver.is_some(),
        ) {
            (Some(queue), _) => match queue.get(child_id).await {
                Ok(Some(record)) => {
                    let writer =
                        phantom_mesh::tasks::SessionWriter::open(&record.workspace_id, child_id)
                            .await
                            .map_err(|e| tracing::warn!("session writer: {}", e))
                            .ok();
                    if let Some(w) = writer.as_ref() {
                        spawn_state
                            .agent_runtime
                            .run_tracked_with_session(
                                &agent_name,
                                &prompt,
                                &spawn_history,
                                None,
                                &spawn_state.cost_tracker,
                                w,
                            )
                            .await
                    } else {
                        spawn_state
                            .agent_runtime
                            .run_tracked(
                                &agent_name,
                                &prompt,
                                &spawn_history,
                                None,
                                &spawn_state.cost_tracker,
                            )
                            .await
                    }
                }
                _ => Err(anyhow::anyhow!("child task vanished")),
            },
            _ => Err(anyhow::anyhow!("task queue unavailable")),
        };

        if let Some(queue) = &spawn_state.task_queue {
            match result {
                Ok(r) => {
                    if let Err(e) = queue
                        .record_progress(child_id, r.turns, r.cost_delta_usd)
                        .await
                    {
                        tracing::warn!(%child_id, "child task record_progress failed: {}", e);
                    }
                    if let Err(e) = queue
                        .transition(child_id, pm_types::TaskStatus::Completed, None)
                        .await
                    {
                        tracing::warn!(%child_id, "child task transition to Completed failed: {}", e);
                    }
                    notify_task_transition(
                        &spawn_state,
                        Some(child_id),
                        &agent_name,
                        &prompt,
                        pm_types::TaskStatus::Completed,
                        Some(&r.output),
                    )
                    .await;
                    spawn_state
                        .conversations
                        .append(
                            &chat_id,
                            ChatMessage {
                                role: "user".into(),
                                content: prompt.clone(),
                                tool_calls: None,
                            },
                            ChatMessage {
                                role: "assistant".into(),
                                content: r.output,
                                tool_calls: None,
                            },
                        )
                        .await;
                }
                Err(e) => {
                    let err = e.to_string();
                    if let Err(te) = queue
                        .transition(child_id, pm_types::TaskStatus::Failed, Some(&err))
                        .await
                    {
                        tracing::warn!(%child_id, "child task transition to Failed failed: {}", te);
                    }
                    notify_task_transition(
                        &spawn_state,
                        Some(child_id),
                        &agent_name,
                        &prompt,
                        pm_types::TaskStatus::Failed,
                        Some(&err),
                    )
                    .await;
                }
            }
        }
    });

    (
        axum::http::StatusCode::ACCEPTED,
        Json(json!({
            "task_id": child_id.to_string(),
            "parent_task_id": parent_uuid.to_string(),
            "workspace_id": parent.workspace_id,
            "history_entries": repaired.entries.len(),
            "synthetic_repairs": repaired.repaired_count,
            "stream_url": format!("/tasks/{}/stream", child_id),
        })),
    )
        .into_response()
}

async fn tasks_cancel(
    State(state): State<phantom_mesh::AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // T7b T13-N1 follow-up: HMAC gate.
    if let Err((code, json)) =
        phantom_mesh::auth_gate::require_cluster_auth(&state.cluster_manager, &headers, &body)
    {
        return (code, json).into_response();
    }
    let Some(queue) = state.task_queue.clone() else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Ok(uuid) = uuid::Uuid::parse_str(&id) else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid uuid" })),
        )
            .into_response();
    };
    match queue
        .transition(uuid, pm_types::TaskStatus::Cancelled, None)
        .await
    {
        Ok(t) => {
            notify_task_transition(
                &state,
                Some(uuid),
                &t.agent_name,
                &t.prompt,
                pm_types::TaskStatus::Cancelled,
                None,
            )
            .await;
            Json(json!(t)).into_response()
        }
        Err(e) => (
            axum::http::StatusCode::CONFLICT,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use phantom_mesh::mesh::{ClusterConfig, ClusterManager};
    use phantom_mesh::{AppState, TaskQueue, TaskStore};
    use tower::ServiceExt; // for `oneshot`

    const TEST_CLUSTER_SECRET: &str = "test-secret";

    /// Serializes env-touching tests in THIS binary's test module. The library
    /// crate's `env_lock` is `#[cfg(test)]`-only and not visible when compiling
    /// the binary's tests, so we keep a local guard. Recovers from poisoning so
    /// a panicking test can't permanently wedge the suite.
    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        static M: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        M.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    /// Point the at-most-once ledger at a throwaway file for one test,
    /// restoring the prior env on drop. The handler records a dedup key in the
    /// (default, PERSISTENT) ledger keyed by `agent\nprompt`, so without
    /// isolation an identical body collides across tests AND across runs within
    /// the 24h TTL. The caller must already hold the crate env lock.
    struct IdemStoreGuard {
        _tmp: tempfile::TempDir,
        prev: Option<String>,
    }
    impl IdemStoreGuard {
        fn new() -> Self {
            let tmp = tempfile::TempDir::new().expect("tempdir");
            let prev = std::env::var("PHANTOM_IDEMPOTENCY_STORE").ok();
            std::env::set_var(
                "PHANTOM_IDEMPOTENCY_STORE",
                tmp.path().join("idempotency.jsonl"),
            );
            Self { _tmp: tmp, prev }
        }
    }
    impl Drop for IdemStoreGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("PHANTOM_IDEMPOTENCY_STORE", v),
                None => std::env::remove_var("PHANTOM_IDEMPOTENCY_STORE"),
            }
        }
    }

    fn permissive_cors() -> CorsLayer {
        CorsLayer::new()
            .allow_origin(tower_http::cors::Any)
            .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
            .allow_headers(tower_http::cors::Any)
    }

    /// Build the production daemon router (`main.rs::build_router`) backed by an
    /// in-temp TaskStore + the test cluster secret, so `/rpc/task/assign`'s HMAC
    /// gate passes. Returns (router-builder closure, queue handle) — the queue is
    /// shared (same SQLite file) so the test can count rows independently of the
    /// router (oneshot consumes the router, so we rebuild it per request).
    fn make_state_with_queue(db_path: std::path::PathBuf) -> AppState {
        make_state_with_caps(db_path, Vec::new())
    }

    /// Same as [`make_state_with_queue`] but advertises a fixed set of
    /// `worker_caps`. An empty vec means "full worker" (accept anything),
    /// matching `enforce_required_caps`'s Rule 1.
    fn make_state_with_caps(db_path: std::path::PathBuf, worker_caps: Vec<String>) -> AppState {
        let cfg = ClusterConfig {
            node_name: Some("test".into()),
            cluster_secret: Some(TEST_CLUSTER_SECRET.into()),
            worker_caps,
            ..ClusterConfig::default()
        };
        let mut state = AppState::new();
        state.cluster_manager = ClusterManager::new(cfg);
        let store = TaskStore::open_at(db_path).expect("open task store");
        state.task_queue = Some(TaskQueue::new(store));
        state
    }

    /// Build a signed POST /rpc/task/assign whose `X-Cluster-Auth` is the
    /// HMAC-SHA256 of the body keyed by [`TEST_CLUSTER_SECRET`].
    fn assign_request(body: &Value) -> Request<Body> {
        let body_str = body.to_string();
        let signing_cfg = ClusterConfig {
            cluster_secret: Some(TEST_CLUSTER_SECRET.into()),
            ..ClusterConfig::default()
        };
        let token = ClusterManager::new(signing_cfg).make_auth_token(&body_str);
        Request::builder()
            .method("POST")
            .uri("/rpc/task/assign")
            .header("content-type", "application/json")
            .header("X-Cluster-Auth", token)
            .body(Body::from(body_str))
            .expect("build request")
    }

    async fn body_json(resp: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .expect("read body");
        serde_json::from_slice(&bytes).expect("parse json body")
    }

    /// DRIFT regression guard (DISPATCH-MESH-DURABILITY §3.0): the SHIPPED daemon
    /// router (`main.rs`, not `serve.rs`) must dedup a re-sent /rpc/task/assign.
    /// A duplicate POST of the SAME body must (a) return 200 with `deduped:true`
    /// and the ORIGINAL job_id, and (b) NOT create a second TaskQueue row.
    #[tokio::test]
    async fn duplicate_assign_dedups_and_spawns_no_second_task_row() {
        let _g = env_guard(); // serialize env-touching tests
        let _idem = IdemStoreGuard::new(); // isolate the dedup ledger

        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("phantom.db");

        let body = json!({ "agent": "master", "prompt": "main.rs dedup probe" });

        // First assign: accepted (202), mints a job_id, inserts exactly one row.
        let resp1 = build_router(make_state_with_queue(db_path.clone()), permissive_cors())
            .oneshot(assign_request(&body))
            .await
            .expect("first /rpc/task/assign");
        assert_eq!(
            resp1.status(),
            StatusCode::ACCEPTED,
            "first assign must be accepted (202)"
        );
        let body1 = body_json(resp1).await;
        let first_job_id = body1["job_id"]
            .as_str()
            .expect("first response must carry a job_id")
            .to_string();

        // Count rows after the first assign via an independent queue over the SAME db.
        let count_after_first = {
            let q = TaskQueue::new(TaskStore::open_at(db_path.clone()).unwrap());
            q.list(None, None, 1000).await.unwrap().len()
        };
        assert_eq!(
            count_after_first, 1,
            "first assign must create exactly one TaskQueue row"
        );

        // Duplicate assign (identical body → identical derived dedup key).
        let resp2 = build_router(make_state_with_queue(db_path.clone()), permissive_cors())
            .oneshot(assign_request(&body))
            .await
            .expect("duplicate /rpc/task/assign");
        assert_eq!(
            resp2.status(),
            StatusCode::OK,
            "duplicate must be deduped (200, not a fresh 202)"
        );
        let body2 = body_json(resp2).await;
        assert_eq!(
            body2["deduped"],
            json!(true),
            "duplicate must carry deduped:true, got {body2}"
        );
        assert_eq!(
            body2["job_id"].as_str(),
            Some(first_job_id.as_str()),
            "duplicate MUST return the ORIGINAL job_id, got {body2}"
        );

        // Exactly ONE row must exist — the duplicate spawned no second task.
        let count_after_dup = {
            let q = TaskQueue::new(TaskStore::open_at(db_path.clone()).unwrap());
            q.list(None, None, 1000).await.unwrap().len()
        };
        assert_eq!(
            count_after_dup, 1,
            "duplicate must NOT create a second TaskQueue row"
        );
    }

    /// DRIFT regression guard: the SHIPPED daemon router (`main.rs`) must enforce
    /// `required_caps` in STRICT mode exactly like the hardened `serve.rs`
    /// handler. A worker advertising caps=[memory] that is handed a task
    /// requiring [shell] must (a) reject with 409, (b) name the missing cap, and
    /// (c) NOT spawn — no TaskQueue row may be created.
    #[tokio::test]
    async fn strict_mode_rejects_missing_caps_with_409_and_no_spawn() {
        let _g = env_guard(); // serialize env-touching tests
        let _idem = IdemStoreGuard::new(); // isolate the dedup ledger

        // STRICT enforcement; restore on scope exit.
        let prev_enforce = std::env::var("PHANTOM_ENFORCE_REQUIRED_CAPS").ok();
        std::env::set_var("PHANTOM_ENFORCE_REQUIRED_CAPS", "strict");
        // Ensure forwarding gate is OFF so the decision is a plain Reject.
        let prev_fwd = std::env::var("PHANTOM_FORWARD_ON_CAPS_MISMATCH").ok();
        std::env::remove_var("PHANTOM_FORWARD_ON_CAPS_MISMATCH");

        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("phantom.db");

        // Worker advertises caps=[memory]; request requires [shell] → mismatch.
        let body = json!({
            "agent": "master",
            "prompt": "caps probe",
            "required_caps": ["shell"],
        });

        let resp = build_router(
            make_state_with_caps(db_path.clone(), vec!["memory".into()]),
            permissive_cors(),
        )
        .oneshot(assign_request(&body))
        .await
        .expect("/rpc/task/assign");

        assert_eq!(
            resp.status(),
            StatusCode::CONFLICT,
            "strict-mode capability mismatch must return 409"
        );
        let body_v = body_json(resp).await;
        let missing: Vec<String> = body_v["missing"]
            .as_array()
            .expect("body must carry a `missing` array")
            .iter()
            .map(|m| m.as_str().unwrap_or("").to_string())
            .collect();
        assert!(
            missing.iter().any(|m| m == "shell"),
            "missing must name [shell], got {body_v}"
        );

        // NO spawn: the TaskQueue must be empty.
        let count = {
            let q = TaskQueue::new(TaskStore::open_at(db_path.clone()).unwrap());
            q.list(None, None, 1000).await.unwrap().len()
        };
        assert_eq!(
            count, 0,
            "strict-mode rejection must NOT create any TaskQueue row"
        );

        // Restore env.
        match prev_enforce {
            Some(v) => std::env::set_var("PHANTOM_ENFORCE_REQUIRED_CAPS", v),
            None => std::env::remove_var("PHANTOM_ENFORCE_REQUIRED_CAPS"),
        }
        if let Some(v) = prev_fwd {
            std::env::set_var("PHANTOM_FORWARD_ON_CAPS_MISMATCH", v);
        }
    }
}
