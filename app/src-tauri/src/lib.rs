mod commands;
#[cfg(desktop)]
mod daemon;
mod runtime_state;
#[cfg(desktop)]
mod updater;

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static DEBUG_CONFIG_INFO: OnceLock<Mutex<String>> = OnceLock::new();

fn set_debug_info(s: String) {
    let m = DEBUG_CONFIG_INFO.get_or_init(|| Mutex::new(String::new()));
    if let Ok(mut g) = m.lock() { *g = s; }
}
fn get_debug_info() -> String {
    DEBUG_CONFIG_INFO.get()
        .and_then(|m| m.lock().ok())
        .map(|g| g.clone())
        .unwrap_or_else(|| "<not set>".into())
}

use commands::settings::AppConfigState;
use runtime_state::RuntimeState;
use tauri::Manager;

#[cfg(desktop)]
use daemon::DaemonState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(desktop)]
    {
        let log_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".phantom-mesh")
            .join("logs");
        let _ = std::fs::create_dir_all(&log_dir);
        let file_appender = tracing_appender::rolling::daily(&log_dir, "app.log");
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::fmt()
            .with_writer(non_blocking)
            .with_ansi(false)
            .init();
    }
    #[cfg(mobile)]
    {
        // Mobile: also write to a file in the sandbox so host can pull
        // it via `xcrun devicectl device copy from`. Stderr still goes
        // to iOS unified log (visible via Console.app over wifi).
        let log_dir = std::env::var("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"));
        let _ = std::fs::create_dir_all(&log_dir);
        let file_appender =
            tracing_appender::rolling::never(&log_dir, "phantom-mesh.log");
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
        // Leak guard so the worker thread keeps draining for app lifetime.
        // Without this the file would never flush.
        Box::leak(Box::new(_guard));
        tracing_subscriber::fmt()
            .with_writer(non_blocking)
            .with_ansi(false)
            .init();
        // One INFO line so the file is non-empty even if nothing else logs.
        tracing::info!(
            "phantom-mesh mobile logger started — tmp={:?}",
            log_dir.display()
        );
    }

    let default_config = commands::settings::AppConfig::default();
    let daemon_port = default_config.daemon_port;

    let mut builder = tauri::Builder::default()
        .manage(AppConfigState::new(default_config))
        .manage(commands::HttpClient::default())
        .manage(commands::supabase::SupabaseState::default())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        // tauri-plugin-deep-link receives the OAuth-callback URL the broker
        // redirects to (`phantom://oauth/callback?p=<base64-payload>`) and
        // routes it to on_open_url(). Required on iOS where the sandbox
        // blocks the loopback HTTP server pattern that desktop uses, and
        // also useful on macOS for the same scheme. Info.plist
        // (CFBundleURLSchemes) registers the `phantom` scheme on iOS.
        .plugin(tauri_plugin_deep_link::init());

    #[cfg(desktop)]
    {
        let daemon_state = DaemonState::new(daemon_port);
        if let Some(ref path) = commands::settings::AppConfig::default().daemon_binary_path {
            let mut guard = daemon_state.binary_path.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(PathBuf::from(path));
        }
        builder = builder
            .manage(daemon_state)
            .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.set_focus();
                }
            }));
    }

    // DeepLinkExt — exposes app.deep_link() / on_open_url helpers.
    // Emitter — required for AppHandle::emit() in Tauri 2.x.
    use tauri_plugin_deep_link::DeepLinkExt as _;
    use tauri::Emitter as _;

    builder
        .setup(move |app| {
            // Deep-link handler — fires when the OS hands us a phantom://
            // URL (typically the OAuth callback the broker meta-refreshes
            // to). We hand the raw URL to the JS layer via a Tauri event
            // so the front-end can parse the `?p=<base64-payload>` query
            // and store the broker_token in the right platform vault
            // (iOS Keychain / desktop ~/.phantom-mesh/auth.json).
            //
            // Attached inside setup() so it's wired before iOS hands us
            // the launch URL when the app cold-starts via `phantom://...`.
            {
                let app_handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    for url in event.urls() {
                        let url_str = url.to_string();
                        tracing::info!(target: "phantom-app", "deep-link received: {}", url_str);
                        let _ = app_handle.emit("deep-link://oauth-callback", url_str);
                    }
                });
            }

            let mut resolved_port = daemon_port;
            let mut resolved_config_path: Option<PathBuf> = None;

            let app_config_dir_result = app.path().app_config_dir();
            set_debug_info(format!(
                "app_config_dir={:?}",
                app_config_dir_result.as_ref().map(|p| p.display().to_string())
            ));
            if let Ok(config_dir) = app_config_dir_result {
                // On Android, app_config_dir() returns the package dir (e.g. /data/user/0/PACKAGE),
                // NOT the files/ subdirectory. Set HOME to the package dir so that
                // dirs::home_dir().join(".phantom-mesh") resolves inside the app's sandbox.
                {
                    let _ = std::env::set_var("HOME", &config_dir);
                }

                // Load ~/.phantom-mesh/env (KEY=VALUE per line, written by
                // broker_sync_from_vault) into the process env. The agent
                // runtime reads provider keys via std::env::var() so without
                // this step a freshly-logged-in iOS user would have keys on
                // disk but no LLM calls would work — the in-process providers
                // would skip every entry for "no api_key set".
                let env_file = config_dir.join(".phantom-mesh").join("env");
                if let Ok(text) = std::fs::read_to_string(&env_file) {
                    let mut loaded = 0usize;
                    for line in text.lines() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') { continue; }
                        if let Some((k, v)) = line.split_once('=') {
                            let k = k.trim();
                            let v = v.trim();
                            if !k.is_empty() && !v.is_empty() {
                                std::env::set_var(k, v);
                                loaded += 1;
                            }
                        }
                    }
                    tracing::info!(
                        "Loaded {} env vars from {} into process env",
                        loaded, env_file.display()
                    );
                }

                // Seed a default agents.toml if one's missing. Without
                // this, the agent runtime can't dispatch chat to any
                // provider — it has the env vars but no [providers.*]
                // block telling it "OPENAI_API_KEY → use this URL with
                // this model". Seeding only happens on first launch
                // (file-exists guard); user edits survive forever.
                match commands::local_keys::seed_default_agents_toml_if_missing() {
                    Ok(true) => tracing::info!("seeded default agents.toml at first-launch"),
                    Ok(false) => tracing::info!("agents.toml already present, no seed"),
                    Err(e) => tracing::warn!("agents.toml seed failed: {e}"),
                }

                // Search for agents.toml in the package dir and its common subdirectories.
                let candidates = [
                    config_dir.join("files").join("agents.toml"),
                    config_dir.join(".phantom-mesh").join("agents.toml"),
                    config_dir.join("agents.toml"),
                    config_dir.join("files").join("config").join("agents.toml"),
                ];
                let toml_path = candidates.iter()
                    .find(|p| !p.as_os_str().is_empty() && p.exists())
                    .cloned()
                    .unwrap_or_else(|| config_dir.join("agents.toml"));
                if toml_path.exists() {
                    tracing::info!("Found agents.toml at: {}", toml_path.display());
                    resolved_config_path = Some(toml_path.clone());
                    set_debug_info(format!(
                        "app_config_dir={}, resolved={}",
                        config_dir.display(),
                        toml_path.display()
                    ));
                } else {
                    tracing::warn!("agents.toml not found; checked: {:?}", candidates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>());
                    set_debug_info(format!(
                        "app_config_dir={}, resolved=NONE, checked={:?}",
                        config_dir.display(),
                        candidates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()
                    ));
                }
                if let Ok(content) = std::fs::read_to_string(&toml_path) {
                    if let Ok(parsed) = content.parse::<toml::Table>() {
                        let config_state = app.state::<AppConfigState>();
                        let mut cfg = config_state.write();
                        if let Some(core) = parsed.get("core").and_then(|v| v.as_table()) {
                            if let Some(key) = core.get("hub_api_key").and_then(|v| v.as_str()) {
                                if !key.is_empty() {
                                    cfg.auth_key = key.to_string();
                                }
                            }
                            if let Some(port) = core.get("port").and_then(|v| v.as_integer()) {
                                resolved_port = port as u16;
                                cfg.daemon_port = resolved_port;
                                cfg.hub_url = format!("http://localhost:{}", resolved_port);
                            }
                        }
                    }
                }
            }

            // Start in-process PhantomMeshRuntime
            {
                let handle = app.handle().clone();
                let port = resolved_port;
                let config_path = resolved_config_path.clone();
                let data_dir = dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".phantom-mesh");

                tauri::async_runtime::spawn(async move {
                    let rt_config = phantom_mesh::runtime::RuntimeConfig {
                        config_path,
                        data_dir: Some(data_dir),
                        ..Default::default()
                    };

                    let started = Instant::now();
                    match RuntimeState::init(rt_config, port).await {
                        Ok(runtime_state) => {
                            tracing::info!(
                                "PhantomMeshRuntime ready in {:.2}s",
                                started.elapsed().as_secs_f64()
                            );
                            let app_state = runtime_state.runtime.app_state().clone();
                            let http_port = runtime_state.port;
                            handle.manage(runtime_state);

                            // HTTP compat server lets peers hit this node's
                            // /rpc/* endpoints. On iOS this is gated behind a
                            // user toggle (UI button → sets a runtime flag) so
                            // the device only listens when the foreground app
                            // is active. v1.5 G8 sandbox-worker dispatch uses
                            // this listener; cluster-mode CLIENT dispatch
                            // (dispatchToCluster.ts) goes outbound and doesn't
                            // need it.
                            tokio::spawn(async move {
                                use axum::http::Method;
                                use tower_http::cors::CorsLayer;
                                let cors = CorsLayer::new()
                                    .allow_origin(tower_http::cors::Any)
                                    .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                                    .allow_headers(tower_http::cors::Any);
                                // Use core's full cluster router (`/healthz`,
                                // `/rpc/ping`, `/rpc/task/assign`, /api/...).
                                // This is what makes the node addressable as
                                // a mesh peer / sandbox worker. Don't merge
                                // build_compat_router on top — the two routers
                                // both define `/api/dashboard/status` +
                                // `/api/providers/health`, and Router::merge
                                // panics on duplicate routes. The Tauri
                                // frontend talks to Rust via tauri::invoke,
                                // not HTTP, so the compat router is unused on
                                // mobile anyway.
                                let _ = http_port; // referenced only for log
                                let app_state_arc = std::sync::Arc::new(app_state);
                                let router = phantom_mesh::serve::router(app_state_arc).layer(cors);
                                // 0.0.0.0 on iOS binds to all interfaces incl
                                // Tailscale's utun*, but iOS app-sandbox will
                                // refuse the bind silently if entitlements
                                // don't include `com.apple.developer.networking
                                // .multipath` etc. For dev-cert IPAs this
                                // tends to "just work" on Wi-Fi/Tailscale.
                                if let Err(e) = phantom_mesh::start_http_server(
                                    "0.0.0.0", http_port, router,
                                ).await {
                                    tracing::warn!("HTTP server bind failed: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            tracing::warn!("PhantomMeshRuntime init failed: {:#}", e);
                        }
                    }
                });
            }

            // Desktop-only: system tray
            #[cfg(desktop)]
            {
                use tauri::menu::{Menu, MenuItem};
                use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
                let open = MenuItem::with_id(app, "open", "開啟主介面", true, None::<&str>)?;
                let pause = MenuItem::with_id(app, "pause", "暫停 Agent", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "結束", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&open, &pause, &quit])?;
                let _tray = TrayIconBuilder::new()
                    .menu(&menu)
                    .tooltip("Phantom Mesh")
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "open" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                        "quit" => {
                            app.state::<DaemonState>().kill();
                            app.exit(0);
                        }
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            if let Some(w) = tray.app_handle().get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    })
                    .build(app)?;
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::broker_login::broker_login_start,
            commands::broker_login::broker_login_finish,
            commands::broker_login::broker_login_status,
            commands::broker_login::broker_login_logout,
            commands::broker_login::broker_sync_from_vault,
            commands::broker_login::broker_register_self_peer,
            commands::broker_login::broker_list_cached_peers,
            commands::local_keys::list_provider_keys,
            commands::local_keys::set_provider_key,
            commands::local_keys::set_provider_keys_bulk,
            commands::local_keys::agents_toml_status,
            commands::local_keys::reseed_agents_toml,
            commands::health::get_health,
            commands::health::get_dashboard_status,
            commands::cluster::get_cluster_status,
            commands::cluster::get_cluster_workers,
            commands::cluster::get_cluster_scores,
            commands::agent::run_agent,
            commands::agent::run_hand,
            commands::agent::send_message,
            commands::agent::get_conversations,
            commands::agent::import_agents_toml,
            commands::provider::get_costs,
            commands::provider::get_revenue,
            commands::provider::get_tools,
            commands::provider::get_hands,
            commands::provider::get_provider_health,
            commands::settings::get_config,
            commands::settings::set_config,
            commands::tasks::get_task_history,
            commands::security::get_audit_log,
            commands::memory::get_memory_observations,
            commands::memory::get_memory_stats,
            commands::memory::search_memory,
            commands::health::get_estop_status,
            commands::networking::get_network_discovery,
            commands::networking::get_network_routes,
            commands::networking::get_network_status,
            commands::onboarding::scan_hardware,
            commands::onboarding::test_ollama,
            commands::onboarding::validate_api_key,
            commands::onboarding::write_config,
            #[cfg(desktop)]
            commands::onboarding::launch_daemon,
            commands::onboarding::generate_qr_data,
            commands::onboarding::get_local_ip,
            commands::onboarding::scan_credentials,
            commands::onboarding::read_copilot_token,
            commands::onboarding::read_gcloud_adc,
            commands::onboarding::read_claude_cli_token,
            commands::onboarding::open_external_url,
            commands::oauth::oauth_sign_in,
            commands::supabase::supabase_sign_in,
            commands::supabase::supabase_get_session,
            commands::supabase::supabase_log_usage,
            commands::supabase::supabase_backup_config,
            commands::supabase::supabase_restore_config,
            commands::supabase::supabase_sign_out,
            commands::goals::goals_list,
            commands::goals::goals_create,
            commands::goals::goals_get,
            commands::goals::goals_update,
            commands::goals::goals_delete,
            commands::goals::goals_progress,
            commands::goals::goals_today,
            commands::goals::goals_summary,
            commands::goals::goals_milestones,
            commands::goals::goals_milestone_add,
            commands::goals::goals_milestone_toggle,
            commands::goals::goals_recurring_tasks,
            commands::goals::goals_recurring_add,
            commands::goals::goals_recurring_complete,
            commands::goals::goals_checkin_add,
            commands::goals::goals_checkins,
            commands::goals::goals_mood_trend,
            commands::goals::goals_weekly_summary,
            commands::goals::goals_global_mood,
            commands::browser::browser_navigate,
            commands::browser::browser_screenshot,
            commands::browser::browser_snapshot,
            commands::browser::browser_status,
            commands::browser::browser_close,
            commands::pages::list_pages,
            commands::pages::load_page,
            commands::pages::save_page,
            commands::pages::delete_page,
            commands::pages::page_db_get,
            commands::pages::page_db_set,
            commands::pages::page_db_query,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn build_compat_router(state: phantom_mesh::AppState, _port: u16) -> axum::Router {
    use axum::{routing::get, routing::post, Json, Router};
    use axum::extract::{State, Query};
    use axum::response::{IntoResponse, Redirect};
    use serde_json::{json, Value};
    use std::collections::HashMap;

    async fn health(State(state): State<phantom_mesh::AppState>) -> Json<Value> {
        Json(json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
            "service": "phantom-mesh",
            "mode": "library",
            "uptime_seconds": state.started_at.elapsed().as_secs(),
        }))
    }

    async fn tools_list(State(state): State<phantom_mesh::AppState>) -> Json<Value> {
        Json(json!({ "tools": state.tool_registry.names() }))
    }

    async fn hands_list(State(state): State<phantom_mesh::AppState>) -> Json<Value> {
        Json(json!({ "hands": state.hands.names() }))
    }

    async fn costs(State(state): State<phantom_mesh::AppState>) -> Json<Value> {
        Json(state.cost_tracker.summary().await)
    }

    async fn revenue(State(_): State<phantom_mesh::AppState>) -> Json<Value> {
        Json(json!({ "total_usd": 0.0 }))
    }

    async fn task_history(State(state): State<phantom_mesh::AppState>) -> Json<Value> {
        match &state.task_queue {
            Some(q) => match q.store().list(None, None, 50).await {
                Ok(tasks) => Json(json!({ "tasks": tasks })),
                Err(_) => Json(json!({ "tasks": [] })),
            },
            None => Json(json!({ "tasks": [] })),
        }
    }

    async fn dashboard_status(State(state): State<phantom_mesh::AppState>) -> Json<Value> {
        Json(json!({
            "tools_count": state.tool_registry.names().len(),
            "hands_count": state.hands.names().len(),
            "active_sessions": 0,
            "uptime_seconds": state.started_at.elapsed().as_secs(),
            "total_requests": 0,
        }))
    }

    async fn provider_health(State(state): State<phantom_mesh::AppState>) -> Json<Value> {
        let providers = state.llm_router.inner().health_summary().into_iter().map(|s| {
            let health = if s.is_available { "healthy" } else { "offline" };
            json!({
                "id": s.provider_name.clone(),
                "name": s.provider_name.clone(),
                "display_name": s.provider_name,
                "is_available": s.is_available,
                "health": health,
                "status": health,
            })
        }).collect::<Vec<_>>();
        Json(json!({ "providers": providers }))
    }

    async fn agent_run(
        State(state): State<phantom_mesh::AppState>,
        axum::extract::Path(name): axum::extract::Path<String>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        let prompt = body["prompt"].as_str().unwrap_or("");
        let history = state.conversations.get_history("app").await;
        match state.agent_runtime.run(&name, prompt, &history, None).await {
            Ok(result) => {
                use phantom_mesh::providers::traits::ChatMessage;
                state.conversations.append("app",
                    ChatMessage { role: "user".into(), content: prompt.to_string(), tool_calls: None },
                    ChatMessage { role: "assistant".into(), content: result.output.clone(), tool_calls: None },
                ).await;
                Json(json!({ "agent": name, "output": result.output, "elapsed": result.elapsed_secs }))
            }
            Err(e) => Json(json!({ "error": e.to_string() })),
        }
    }

    async fn oauth_google_start() -> impl IntoResponse {
        Redirect::temporary(&phantom_mesh::oauth::google_start_url(7878))
    }

    async fn oauth_apple_start() -> impl IntoResponse {
        match phantom_mesh::oauth::apple_start_url(7878) {
            Ok(url) => Redirect::temporary(&url).into_response(),
            Err(e) => axum::response::Html(format!("<html><body>{}</body></html>", e)).into_response(),
        }
    }

    async fn oauth_callback(Query(params): Query<HashMap<String, String>>) -> impl IntoResponse {
        let code = params.get("code").cloned().unwrap_or_default();
        let state_param = params.get("state").cloned().unwrap_or_default();
        let error = params.get("error").cloned().unwrap_or_default();
        if !error.is_empty() {
            return axum::response::Html(format!("<html><body>{}</body></html>", error)).into_response();
        }
        match phantom_mesh::oauth::handle_callback(&code, &state_param).await {
            Ok(url) => Redirect::temporary(&url).into_response(),
            Err(e) => axum::response::Html(format!("<html><body>{}</body></html>", e)).into_response(),
        }
    }

    async fn oauth_result() -> Json<Value> {
        match phantom_mesh::oauth::get_result() {
            Some(Ok(id)) => Json(json!({"ok": true, "identity": id})),
            Some(Err(e)) => Json(json!({"ok": false, "error": e})),
            None => Json(json!({"ok": false, "error": "no result yet"})),
        }
    }

    async fn oauth_apple_available() -> Json<Value> {
        Json(json!({"available": phantom_mesh::oauth::apple_available()}))
    }

    async fn debug_send_message(
        State(state): State<phantom_mesh::AppState>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        let prompt = body["prompt"].as_str().unwrap_or("hello").to_string();
        let agent_name = body["agent"].as_str().unwrap_or("master").to_string();
        let chat_id = "app-default";
        let history = state.conversations.get_history(chat_id).await;
        let res = state
            .agent_runtime
            .run_tracked(&agent_name, &prompt, &history, None, &state.cost_tracker)
            .await;
        match res {
            Ok(r) => Json(json!({
                "ok": true,
                "agent": agent_name,
                "output": r.output,
                "elapsed": r.elapsed_secs,
                "tool_calls": r.tool_calls_made,
            })),
            Err(e) => Json(json!({
                "ok": false,
                "error": format!("{:#}", e),
            })),
        }
    }

    async fn debug_config() -> Json<Value> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "<unset>".into());
        let setup_debug = get_debug_info();
        // Derive paths from the same config_dir startup uses (HOME is set to
        // app_config_dir at boot). Hard-coded paths previously pointed at
        // /data/user/0/<pkg>/files/* which doesn't include the root
        // agents.toml that write_config actually writes to, so the debug
        // endpoint reported "not found" even when the file existed.
        let config_dir = std::path::PathBuf::from(&home);
        let check_paths: Vec<std::path::PathBuf> = vec![
            config_dir.join("files").join("agents.toml"),
            config_dir.join(".phantom-mesh").join("agents.toml"),
            config_dir.join("agents.toml"),
            config_dir.join("files").join("config").join("agents.toml"),
        ];
        let mut parse_result: Option<String> = None;
        let path_info: Vec<Value> = check_paths.iter().map(|p| {
            let path_str = p.display().to_string();
            let exists = p.exists();
            let content = if exists { std::fs::read_to_string(p).ok() } else { None };
            if let Some(ref c) = content {
                if parse_result.is_none() {
                    match toml::from_str::<phantom_mesh::AgentsConfig>(c) {
                        Ok(cfg) => {
                            let provider_names: Vec<String> = cfg.providers.keys().cloned().collect();
                            parse_result = Some(format!("OK: providers={:?}", provider_names));
                        }
                        Err(e) => {
                            parse_result = Some(format!("ERR: {}", e));
                        }
                    }
                }
            }
            json!({"path": path_str, "exists": exists, "preview": content.map(|s| s.chars().take(200).collect::<String>())})
        }).collect();
        Json(json!({
            "home_env": home,
            "setup_debug": setup_debug,
            "paths_checked": path_info,
            "parse_result": parse_result,
        }))
    }

    Router::new()
        .route("/health", get(health))
        .route("/tools", get(tools_list))
        .route("/hands", get(hands_list))
        .route("/costs", get(costs))
        .route("/revenue", get(revenue))
        .route("/task/history", get(task_history))
        .route("/api/dashboard/status", get(dashboard_status))
        .route("/api/providers/health", get(provider_health))
        .route("/agent/:name/run", post(agent_run))
        .route("/oauth/google", get(oauth_google_start))
        .route("/oauth/apple", get(oauth_apple_start))
        .route("/oauth/apple/available", get(oauth_apple_available))
        .route("/oauth/callback", get(oauth_callback))
        .route("/callback", get(oauth_callback))
        .route("/oauth/result", get(oauth_result))
        .route("/debug/config", get(debug_config))
        .route("/debug/send_message", post(debug_send_message))
        .with_state(state)
}
