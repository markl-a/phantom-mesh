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
        let home = std::env::var("HOME").ok().or_else(|| std::env::var("USERPROFILE").ok()).or_else(|| dirs::home_dir().map(|p| p.to_string_lossy().into_owned())).unwrap_or_else(|| ".".to_string());
        PathBuf::from(home)
            .join(".phantom-mesh")
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
    let home = std::env::var("HOME").ok().or_else(|| std::env::var("USERPROFILE").ok()).or_else(|| dirs::home_dir().map(|p| p.to_string_lossy().into_owned())).unwrap_or_else(|| ".".to_string());
    let dir = std::path::PathBuf::from(home)
        .join(".phantom-mesh")
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
    let home = std::env::var("HOME").ok().or_else(|| std::env::var("USERPROFILE").ok()).or_else(|| dirs::home_dir().map(|p| p.to_string_lossy().into_owned())).unwrap_or_else(|| ".".to_string());
    let path = std::path::PathBuf::from(home)
        .join(".phantom-mesh")
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
    let workspace_id = match (&state.workspace_resolver, std::env::current_dir()) {
        (Some(resolver), Ok(cwd)) => resolver
            .resolve_or_create(&cwd)
            .await
            .ok()
            .map(|ws| ws.id.0)
            .unwrap_or_else(|| "default".into()),
        _ => "default".into(),
    };
    let task = match queue.create(&workspace_id, &req.agent, &req.prompt).await {
        Ok(t) => t,
        Err(e) => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let job_id = task.task_id.to_string();
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
