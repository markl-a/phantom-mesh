// pub so integration tests under app/src-tauri/tests/ can reach the
// validators + parsers (e.g. dispatch_commands.rs).
pub mod commands;
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

// iOS-only: native URLSession bridge from native/ios_fetch.m (compiled
// by build.rs via cc crate so the symbol lives in our dylib). Used because
// tauri-plugin-http (reqwest) silently times out fetching Tailscale magic
// hostnames + private IPs from physical iOS devices.
#[cfg(target_os = "ios")]
unsafe extern "C" {
    fn spectyn_ios_fetch(
        url:          *const std::os::raw::c_char,
        method:       *const std::os::raw::c_char,
        body:         *const u8,
        body_len:     std::os::raw::c_long,
        auth_header:  *const std::os::raw::c_char,
        result_buf:   *mut u8,
        result_buf_len: *mut std::os::raw::c_long,
        status_out:   *mut std::os::raw::c_long,
        max_result_len: std::os::raw::c_long,
    );

    // native/ios_location.m — one-shot CoreLocation GPS read.
    fn spectyn_ios_location(
        lat_out: *mut f64,
        lon_out: *mut f64,
        acc_out: *mut f64,
        err_buf: *mut u8,
        err_len: *mut std::os::raw::c_long,
        max_err: std::os::raw::c_long,
    );

    // native/ios_motion.m — one-shot CoreMotion + UIDevice multi-sensor read,
    // returns a UTF-8 JSON object (battery/accel/gyro/attitude/magnetometer +
    // best-effort steps/activity).
    fn spectyn_ios_sensors(
        json_buf: *mut u8,
        json_len: *mut std::os::raw::c_long,
        max_len: std::os::raw::c_long,
    );
}

#[derive(serde::Serialize)]
struct LocationResult {
    lat: f64,
    lon: f64,
    accuracy: f64,
    error: Option<String>,
}

/// `swift_get_location` — Tauri command bridging JS → native CoreLocation.
/// Returns the device's current GPS fix (or an `error` string). iOS-only;
/// on other platforms callers should fall back to navigator.geolocation.
#[tauri::command]
async fn swift_get_location() -> Result<LocationResult, String> {
    #[cfg(not(target_os = "ios"))]
    {
        return Err("swift_get_location is iOS-only".into());
    }
    #[cfg(target_os = "ios")]
    {
        let mut lat: f64 = 0.0;
        let mut lon: f64 = 0.0;
        let mut acc: f64 = -1.0;
        const MAXE: std::os::raw::c_long = 512;
        let mut errbuf: Vec<u8> = vec![0; MAXE as usize];
        let mut errlen: std::os::raw::c_long = 0;
        // SAFETY: native/ios_location.m writes at most MAXE bytes into errbuf
        // and bounds its wait at 25s via a DispatchSemaphore.
        unsafe {
            spectyn_ios_location(
                &mut lat as *mut f64,
                &mut lon as *mut f64,
                &mut acc as *mut f64,
                errbuf.as_mut_ptr(),
                &mut errlen as *mut std::os::raw::c_long,
                MAXE,
            );
        }
        let elen = (errlen as usize).min(errbuf.len());
        let error = if elen > 0 {
            Some(String::from_utf8_lossy(&errbuf[..elen]).to_string())
        } else {
            None
        };
        Ok(LocationResult { lat, lon, accuracy: acc, error })
    }
}

/// `swift_get_sensors` — Tauri command bridging JS → native CoreMotion/UIDevice.
/// Returns a JSON string of the phone's current sensor readings (battery,
/// accel/gyro/attitude/magnetometer, plus best-effort steps/activity). iOS-only.
/// This is the "behaviour" feed the AI partner reads (NORTH-STAR §Q2).
#[tauri::command]
async fn swift_get_sensors() -> Result<String, String> {
    #[cfg(not(target_os = "ios"))]
    {
        return Err("swift_get_sensors is iOS-only".into());
    }
    #[cfg(target_os = "ios")]
    {
        const MAX: std::os::raw::c_long = 8 * 1024;
        let mut buf: Vec<u8> = vec![0; MAX as usize];
        let mut len: std::os::raw::c_long = 0;
        // SAFETY: native/ios_motion.m writes at most MAX bytes into buf and
        // bounds each sensor read with a DispatchSemaphore.
        unsafe {
            spectyn_ios_sensors(buf.as_mut_ptr(), &mut len as *mut std::os::raw::c_long, MAX);
        }
        let n = (len as usize).min(buf.len());
        Ok(String::from_utf8_lossy(&buf[..n]).to_string())
    }
}

#[derive(serde::Serialize)]
struct SwiftFetchResult {
    status: i64,
    body:   String,
}

/// `swift_cluster_fetch` — Tauri command bridging JS → Swift URLSession.
/// On non-iOS targets, returns an error explaining the fallback. On iOS,
/// invokes the @_cdecl Swift wrapper with a 64 KiB response buffer.
#[tauri::command]
async fn swift_cluster_fetch(
    url:    String,
    method: String,
    body:   String,
    auth:   String,
) -> Result<SwiftFetchResult, String> {
    #[cfg(not(target_os = "ios"))]
    {
        let _ = (url, method, body, auth);
        return Err("swift_cluster_fetch is iOS-only; use fetch() on other platforms".into());
    }
    #[cfg(target_os = "ios")]
    {
        use std::ffi::CString;
        let url_c    = CString::new(url).map_err(|e| e.to_string())?;
        let method_c = CString::new(method).map_err(|e| e.to_string())?;
        let auth_c   = CString::new(auth).map_err(|e| e.to_string())?;
        let body_bytes = body.into_bytes();

        // 64 KiB response buffer. Cluster RPC responses are JSON, well under.
        const MAX: std::os::raw::c_long = 64 * 1024;
        let mut buf:    Vec<u8> = vec![0; MAX as usize];
        let mut buf_len: std::os::raw::c_long = 0;
        let mut status:  std::os::raw::c_long = 0;

        // SAFETY: native/ios_fetch.m writes at most `max_result_len` bytes
        // into the buffer and updates result_buf_len / status_out. The
        // synchronous DispatchSemaphore inside ObjC bounds the wait at 35s.
        unsafe {
            spectyn_ios_fetch(
                url_c.as_ptr(),
                method_c.as_ptr(),
                body_bytes.as_ptr(),
                body_bytes.len() as std::os::raw::c_long,
                auth_c.as_ptr(),
                buf.as_mut_ptr(),
                &mut buf_len as *mut std::os::raw::c_long,
                &mut status as *mut std::os::raw::c_long,
                MAX,
            );
        }

        let blen = (buf_len as usize).min(buf.len());
        let body_str = String::from_utf8_lossy(&buf[..blen]).to_string();
        Ok(SwiftFetchResult { status: status as i64, body: body_str })
    }
}

#[cfg(desktop)]
use daemon::DaemonState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(desktop)]
    {
        let log_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".spectyn-mesh")
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
            tracing_appender::rolling::never(&log_dir, "spectyn-mesh.log");
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
            "spectyn-mesh mobile logger started — tmp={:?}",
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
        // tauri-plugin-opener: opens external URLs in the system browser. On iOS
        // this uses UIApplication.openURL: — the `open` crate fails in the iOS
        // sandbox, which forced open_external_url to fall back to in-webview nav,
        // so Google rejected the broker OAuth flow (disallowed_useragent). Opening
        // Safari lets the callback return via the spectyn://oauth/callback deep-link.
        .plugin(tauri_plugin_opener::init())
        // tauri-plugin-deep-link receives the OAuth-callback URL the broker
        // redirects to (`spectyn://oauth/callback?p=<base64-payload>`) and
        // routes it to on_open_url(). Required on iOS where the sandbox
        // blocks the loopback HTTP server pattern that desktop uses, and
        // also useful on macOS for the same scheme. Info.plist
        // (CFBundleURLSchemes) registers the `spectyn` scheme on iOS.
        .plugin(tauri_plugin_deep_link::init())
        // tauri-plugin-http: native reqwest-based fetch that bypasses
        // WKWebView's CORS + Mixed Content policies. Required on iOS
        // where the webview origin is https://tauri.localhost — fetching
        // any http:// URL (e.g. cluster coordinator) is mixed content
        // and silently blocked by WebKit even with ATS arbitrary loads.
        .plugin(tauri_plugin_http::init());

    #[cfg(desktop)]
    {
        let daemon_state = DaemonState::new(daemon_port);
        if let Some(ref path) = commands::settings::AppConfig::default().daemon_binary_path {
            let mut guard = daemon_state.binary_path.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(PathBuf::from(path));
        }
        builder = builder
            .manage(daemon_state)
            // tauri-plugin-updater: powers the @tauri-apps/plugin-updater JS
            // check()/downloadAndInstall() the UpdatePanel calls. Without the
            // plugin initialised here, those JS calls error ("updater not
            // found") and the in-app updater is silently dead. Desktop-only:
            // the crate dep is gated to not(android|ios) and the endpoints +
            // pubkey live in tauri.conf.json's [plugins.updater].
            .plugin(tauri_plugin_updater::Builder::new().build())
            .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.set_focus();
                }
            }))
            // SPEC-41 F2/F3 — global shortcuts. The handler emits a frontend
            // event; the React app (App.tsx) routes it to the chip quick-log /
            // focus-start surface. Cmd+Shift+H → habit chip, Cmd+Shift+F →
            // focus. (Registration happens in setup(); macOS needs Accessibility
            // permission for the OS to actually deliver the keystroke.)
            .plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(|app, shortcut, event| {
                        use tauri::Emitter as _;
                        use tauri_plugin_global_shortcut::{Code, ShortcutState};
                        if event.state == ShortcutState::Pressed {
                            match shortcut.key {
                                Code::KeyH => {
                                    let _ = app.emit("shortcut://chip", ());
                                }
                                Code::KeyF => {
                                    let _ = app.emit("shortcut://focus", ());
                                }
                                _ => {}
                            }
                        }
                    })
                    .build(),
            );
    }

    // DeepLinkExt — exposes app.deep_link() / on_open_url helpers.
    // Emitter — required for AppHandle::emit() in Tauri 2.x.
    use tauri_plugin_deep_link::DeepLinkExt as _;
    use tauri::Emitter as _;

    // E2E native-window automation (macOS WKWebView has no system WebDriver).
    // Gated behind BOTH the opt-in `e2e-webdriver` cargo feature AND
    // debug_assertions+desktop — so the in-app W3C WebDriver HTTP server (and the
    // plugin crate itself) is entirely absent from default and release builds.
    // Enable for the E2E binary with `cargo build --features e2e-webdriver`.
    #[cfg(all(feature = "e2e-webdriver", debug_assertions, desktop))]
    {
        builder = builder.plugin(tauri_plugin_webdriver_automation::init());
    }

    builder
        .setup(move |app| {
            // iOS Local Network permission trigger.
            //
            // tauri-plugin-http (reqwest) makes HTTP requests via low-level
            // sockets that DO NOT trigger iOS 14+'s local-network permission
            // flow. The result: silent timeouts when the app tries to reach
            // any private IP (Tailscale 100.x, LAN 192.168.x, etc) — iOS
            // never shows the "this app wants to use your local network"
            // dialog, and there's no toggle in Settings > Spectyn Mesh.
            //
            // Fix: kick a one-shot mDNS browse on startup. The act of
            // binding a multicast UDP socket on the local network triggers
            // iOS's permission dialog, after which reqwest is allowed.
            // Browse for our own service type so if peers ARE advertising,
            // we discover them as a bonus.
            std::thread::spawn(|| {
                use mdns_sd::ServiceDaemon;
                if let Ok(daemon) = ServiceDaemon::new() {
                    let _ = daemon.browse("_spectyn-mesh._tcp.local.");
                    // Keep the browse alive for 30s so the iOS prompt has
                    // time to surface and the user has time to tap Allow.
                    std::thread::sleep(std::time::Duration::from_secs(30));
                    let _ = daemon.shutdown();
                }
            });

            // Deep-link handler — fires when the OS hands us a spectyn://
            // URL (typically the OAuth callback the broker meta-refreshes
            // to). We hand the raw URL to the JS layer via a Tauri event
            // so the front-end can parse the `?p=<base64-payload>` query
            // and store the broker_token in the right platform vault
            // (iOS Keychain / desktop ~/.spectyn-mesh/auth.json).
            //
            // Attached inside setup() so it's wired before iOS hands us
            // the launch URL when the app cold-starts via `spectyn://...`.
            {
                let app_handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    for url in event.urls() {
                        let url_str = url.to_string();
                        // V8-HIGH-5: filter at the Rust layer so only valid
                        // spectyn://oauth/callback?p=<b64url> URLs reach the
                        // JS deep-link listener. The OS routes EVERY
                        // spectyn:// URL here (the scheme registration is
                        // path-blind), so without this filter an attacker
                        // who can route a `spectyn://anything?…` URL to the
                        // app could deliver crafted payloads to whatever
                        // front-end listeners happen to be attached. The
                        // state-binding check in broker_login_finish is the
                        // primary defense; this is defense-in-depth.

                        // Allowlist for non-OAuth deep links. Currently:
                        //   spectyn://demo-mode  → QA/demo entry point that
                        //   lets the JS layer pre-seed onboarding flags and
                        //   land directly on /settings/cluster. Carries NO
                        //   credentials, sets NO server state — purely a
                        //   navigation signal — so it's safe to forward.
                        if url_str.starts_with("spectyn://demo-mode") {
                            tracing::info!(
                                target: "spectyn-app",
                                "deep-link demo-mode accepted"
                            );
                            let _ = app_handle.emit("deep-link://demo-mode", url_str);
                            continue;
                        }

                        match commands::broker_login::validate_oauth_callback_url(&url_str) {
                            Ok(_parsed) => {
                                tracing::info!(
                                    target: "spectyn-app",
                                    "deep-link OAuth callback accepted (len={})",
                                    url_str.len(),
                                );
                                let _ = app_handle
                                    .emit("deep-link://oauth-callback", url_str);
                            }
                            Err(reason) => {
                                // Not an OAuth callback — try the generic
                                // navigation dispatcher. core::dispatch_deep_link
                                // enforces the SPEC-17 §8/§11.2 allowlist +
                                // path-traversal rejection + OAuth-token
                                // sanitization, so only well-formed spectyn://
                                // URLs survive. We forward ONLY credential-free,
                                // state-free navigation hosts (chat/settings/mesh)
                                // to the webview; oauth/demo-mode are handled
                                // above, and anything else is dropped. §13: log
                                // length + reason only, never the raw URL.
                                match spectyn_mesh::tauri_wire::dispatch_deep_link(&url_str) {
                                    Ok(route)
                                        if matches!(
                                            route.host.as_str(),
                                            "chat" | "settings" | "mesh"
                                        ) =>
                                    {
                                        tracing::info!(
                                            target: "spectyn-app",
                                            "deep-link navigate host={} (len={})",
                                            route.host,
                                            url_str.len(),
                                        );
                                        let _ = app_handle
                                            .emit("deep-link://navigate", &route);
                                    }
                                    Ok(route) => {
                                        // Parsed + allowlisted but not a
                                        // forwardable nav host (e.g. onboarding)
                                        // — drop without navigating.
                                        tracing::warn!(
                                            target: "spectyn-app",
                                            "deep-link dropped: non-nav host={} (len={})",
                                            route.host,
                                            url_str.len(),
                                        );
                                    }
                                    Err(dl_err) => {
                                        // Failed the §8/§11.2 allowlist (and
                                        // wasn't an OAuth callback). Log the
                                        // dispatcher's snake_case code only —
                                        // never the raw URL (§13 privacy). The
                                        // earlier OAuth-validation reason was
                                        // `{reason}`.
                                        tracing::warn!(
                                            target: "spectyn-app",
                                            "deep-link rejected: {} (oauth-check={}, len={})",
                                            dl_err.code,
                                            reason,
                                            url_str.len(),
                                        );
                                    }
                                }
                            }
                        }
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
                // dirs::home_dir().join(".spectyn-mesh") resolves inside the app's sandbox.
                {
                    let _ = std::env::set_var("HOME", &config_dir);
                }

                // Mobile-only: ensure a device-local EventKey so ENCRYPTED capture
                // (habit check-ins → SPEC-16 EventStore) works for a not-logged-in
                // consumer. A fresh wizard install has no broker-provisioned
                // identity.key, and the P4 fix made habit writes hard-require
                // encryption → habit.store (「寫入失敗」). Desktop has its own
                // identity flow, so this is gated to mobile. See
                // encryption_wire::ensure_local_event_key (operator-authorized
                // 2026-05-30; cross-device key reconciliation is a flagged follow-up).
                #[cfg(any(target_os = "android", target_os = "ios"))]
                {
                    let spectyn_dir = config_dir.join(".spectyn-mesh");
                    match spectyn_mesh::encryption_wire::ensure_local_event_key(&spectyn_dir) {
                        Ok(()) => tracing::info!(target: "spectyn-app", "local EventKey ready"),
                        Err(e) => tracing::warn!(
                            target: "spectyn-app",
                            "ensure_local_event_key failed: {:?}",
                            e
                        ),
                    }
                }

                // Load ~/.spectyn-mesh/env (KEY=VALUE per line, written by
                // broker_sync_from_vault) into the process env. The agent
                // runtime reads provider keys via std::env::var() so without
                // this step a freshly-logged-in iOS user would have keys on
                // disk but no LLM calls would work — the in-process providers
                // would skip every entry for "no api_key set".
                let env_file = config_dir.join(".spectyn-mesh").join("env");
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
                    config_dir.join(".spectyn-mesh").join("agents.toml"),
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

            // Start in-process SpectynMeshRuntime
            {
                let handle = app.handle().clone();
                let port = resolved_port;
                let config_path = resolved_config_path.clone();
                let data_dir = dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".spectyn-mesh");

                tauri::async_runtime::spawn(async move {
                    let rt_config = spectyn_mesh::runtime::RuntimeConfig {
                        config_path,
                        data_dir: Some(data_dir),
                        ..Default::default()
                    };

                    let started = Instant::now();
                    match RuntimeState::init(rt_config, port).await {
                        Ok(runtime_state) => {
                            tracing::info!(
                                "SpectynMeshRuntime ready in {:.2}s",
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
                                let router = spectyn_mesh::serve::router(app_state_arc).layer(cors);
                                // 0.0.0.0 on iOS binds to all interfaces incl
                                // Tailscale's utun*, but iOS app-sandbox will
                                // refuse the bind silently if entitlements
                                // don't include `com.apple.developer.networking
                                // .multipath` etc. For dev-cert IPAs this
                                // tends to "just work" on Wi-Fi/Tailscale.
                                if let Err(e) = spectyn_mesh::start_http_server(
                                    "0.0.0.0", http_port, router,
                                ).await {
                                    tracing::warn!("HTTP server bind failed: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            tracing::warn!("SpectynMeshRuntime init failed: {:#}", e);
                        }
                    }
                });
            }

            // Desktop-only: system tray
            #[cfg(desktop)]
            {
                use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
                use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
                use tauri::Emitter as _;
                // SPEC-41 S1 §10.2 — disabled header row showing app identity +
                // version (the live "N peer alive" health line is a follow-up
                // that needs dynamic menu rebuild; a static version is honest now).
                let header = MenuItem::with_id(
                    app,
                    "header",
                    format!("Spectyn Mesh v{}", env!("CARGO_PKG_VERSION")),
                    false,
                    None::<&str>,
                )?;
                let open = MenuItem::with_id(app, "open", "開啟主介面", true, None::<&str>)?;
                // SPEC-41 F4 — Life-Track quick actions in the menu bar. These
                // reuse the same shortcut://* events the global shortcuts emit,
                // so the frontend routes them identically. Unlike the global
                // shortcuts, menu clicks need no Accessibility permission.
                let focus = MenuItem::with_id(app, "focus", "開始專注 (Cmd+Shift+F)", true, None::<&str>)?;
                let chip = MenuItem::with_id(app, "chip", "記錄習慣 (Cmd+Shift+H)", true, None::<&str>)?;
                let review = MenuItem::with_id(app, "review", "今日回顧", true, None::<&str>)?;
                let sep = PredefinedMenuItem::separator(app)?;
                let settings = MenuItem::with_id(app, "settings", "開啟設定…", true, None::<&str>)?;
                // SPEC-41 S1 §10.2 "↻ Restart daemon" — replaces the previous
                // "暫停 Agent" item, which had no on_menu_event arm (dead click).
                let restart = MenuItem::with_id(app, "restart", "重新啟動精靈", true, None::<&str>)?;
                let sep2 = PredefinedMenuItem::separator(app)?;
                let quit = MenuItem::with_id(app, "quit", "結束", true, None::<&str>)?;
                let menu = Menu::with_items(
                    app,
                    &[&header, &open, &focus, &chip, &review, &sep, &settings, &restart, &sep2, &quit],
                )?;
                // Show the main window, then emit a route event the React app
                // listens for (App.tsx). Used by the focus / chip / review items.
                fn show_and_route<R: tauri::Runtime>(app: &tauri::AppHandle<R>, event: &str) {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                    let _ = app.emit(event, ());
                }
                let _tray = TrayIconBuilder::new()
                    .menu(&menu)
                    .tooltip("Spectyn Mesh")
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "open" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                        "focus" => show_and_route(app, "shortcut://focus"),
                        "chip" => show_and_route(app, "shortcut://chip"),
                        "review" => show_and_route(app, "shortcut://review"),
                        "settings" => show_and_route(app, "tray://settings"),
                        "restart" => daemon::restart_in_background(app.clone()),
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

            // SPEC-41 F2/F3 — register the global shortcuts (handler is wired on
            // the plugin in the builder). register() can fail if macOS
            // Accessibility permission isn't granted; log + continue, never crash.
            #[cfg(desktop)]
            {
                use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};
                #[cfg(target_os = "windows")]
                let mods = Modifiers::CONTROL | Modifiers::ALT;
                #[cfg(not(target_os = "windows"))]
                let mods = Modifiers::SUPER | Modifiers::SHIFT;
                #[cfg(target_os = "windows")]
                let prefix = "Ctrl+Alt";
                #[cfg(not(target_os = "windows"))]
                let prefix = "Cmd+Shift";
                for (code, key, suffix) in [
                    (Code::KeyH, "H", "habit chip"),
                    (Code::KeyF, "F", "focus start"),
                ] {
                    let label = format!("{prefix}+{key} ({suffix})");
                    if let Err(e) = app.global_shortcut().register(Shortcut::new(Some(mods), code)) {
                        eprintln!(
                            "global-shortcut: could not register {label}: {e} \
                             (grant Accessibility permission in System Settings → Privacy)"
                        );
                    }
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            swift_cluster_fetch,
            swift_get_location,
            swift_get_sensors,
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
            // F100: cluster peers + events (E002 mobile cluster screen).
            // get_cluster_peers / subscribe_cluster_events / set_this_device_label
            // route through validate_daemon_url (V8-HIGH-2 pattern) so the
            // F101 React UI never builds RPC URLs from JS-side strings.
            commands::cluster_peers::get_cluster_peers,
            commands::cluster_peers::subscribe_cluster_events,
            commands::cluster_peers::set_this_device_label,
            // F102: dispatch + token-stream channel (E002 mobile dispatch).
            // dispatch_task / cancel_dispatch / list_dispatch_providers
            // validate prompt/caps/provider/broker URL in Rust before any
            // POST goes out, and require a saved broker token (E002 sec gate).
            commands::dispatch::dispatch_task,
            commands::dispatch::cancel_dispatch,
            commands::dispatch::list_dispatch_providers,
            // F105: mobile settings extensions (E002 §"Settings screen").
            // get_broker_token_preview / rotate_broker_token /
            // get_heartbeat_interval / set_heartbeat_interval / add_cluster_peer.
            // All inputs validated in Rust against the V8-HIGH-2 allow-list
            // and write back to ~/.spectyn-mesh/agents.toml or auth.json.
            commands::mobile_settings::get_broker_token_preview,
            commands::mobile_settings::rotate_broker_token,
            commands::mobile_settings::get_heartbeat_interval,
            commands::mobile_settings::set_heartbeat_interval,
            commands::mobile_settings::add_cluster_peer,
            commands::agent::run_agent,
            commands::agent::run_hand,
            commands::agent::send_message,
            commands::agent::get_conversations,
            commands::agent::import_agents_toml,
            commands::conversation::get_conversation_history,
            commands::conversation::list_conversations,
            commands::conversation::reset_conversation,
            #[cfg(desktop)]
            daemon::start_daemon,
            #[cfg(desktop)]
            daemon::daemon_status,
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
            commands::onboarding::read_codex_token,
            commands::onboarding::detect_local_servers,
            commands::onboarding::detect_free_provider,
            commands::onboarding::finalize_onboarding_config,
            commands::onboarding::open_external_url,
            commands::onboarding_wire::onboarding_advance,
            commands::onboarding_wire::onboarding_rollback,
            commands::onboarding_wire::onboarding_compute_ttfr,
            commands::onboarding_wire::onboarding_should_fallback_to_demo_relay,
            commands::onboarding_wire::onboarding_start_demo_relay_handoff,
            commands::capture_focus_wire::focus_start_session,
            commands::capture_focus_wire::focus_record_interruption,
            commands::capture_focus_wire::focus_complete_session,
            commands::capture_focus_wire::focus_analyze_session,
            commands::capture_focus_wire::focus_status,
            commands::capture_habit_wire::habit_create,
            commands::capture_habit_wire::habit_checkin,
            commands::capture_habit_wire::habit_list,
            commands::capture_habit_wire::habit_streak,
            commands::capture_food_wire::food_analyze,
            commands::capture_food_wire::food_validate_image,
            commands::cluster_dispatch_wire::dispatch_plan,
            commands::cluster_dispatch_wire::dispatch_score_peer,
            commands::event_storage_wire::events_query,
            commands::event_detail::event_show,
            commands::event_storage_wire::events_search,
            commands::daily_review_wire::daily_review_load,
            commands::miui::miui_guide_check_should_show,
            commands::miui::miui_guide_dismiss,
            commands::miui::miui_guide_open_autostart,
            commands::miui::miui_guide_open_battery_optimization,
            commands::daily_review_wire::daily_review_generate,
            commands::identity_status::identity_status,
            commands::recall_wire::recall_search,
            commands::life_stats::life_stats,
            commands::life_stats::data_export,
            commands::life_stats::open_exports_folder,
            commands::life_stats::event_delete,
            commands::note_wire::note_capture,
            commands::partner_wire::partner_latest_reflection,
            commands::providers_wire::providers_select_provider,
            commands::providers_wire::providers_validate_config,
            commands::providers_wire::providers_complete,
            commands::providers_wire::providers_complete_streaming,
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

fn build_compat_router(state: spectyn_mesh::AppState, _port: u16) -> axum::Router {
    use axum::{routing::get, routing::post, Json, Router};
    use axum::extract::{State, Query};
    use axum::response::{IntoResponse, Redirect};
    use serde_json::{json, Value};
    use std::collections::HashMap;

    async fn health(State(state): State<spectyn_mesh::AppState>) -> Json<Value> {
        Json(json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
            "service": "spectyn-mesh",
            "mode": "library",
            "uptime_seconds": state.started_at.elapsed().as_secs(),
        }))
    }

    async fn tools_list(State(state): State<spectyn_mesh::AppState>) -> Json<Value> {
        Json(json!({ "tools": state.tool_registry.names() }))
    }

    async fn hands_list(State(state): State<spectyn_mesh::AppState>) -> Json<Value> {
        Json(json!({ "hands": state.hands.names() }))
    }

    async fn costs(State(state): State<spectyn_mesh::AppState>) -> Json<Value> {
        Json(state.cost_tracker.summary().await)
    }

    async fn revenue(State(_): State<spectyn_mesh::AppState>) -> Json<Value> {
        Json(json!({ "total_usd": 0.0 }))
    }

    async fn task_history(State(state): State<spectyn_mesh::AppState>) -> Json<Value> {
        match &state.task_queue {
            Some(q) => match q.store().list(None, None, 50).await {
                Ok(tasks) => Json(json!({ "tasks": tasks })),
                Err(_) => Json(json!({ "tasks": [] })),
            },
            None => Json(json!({ "tasks": [] })),
        }
    }

    async fn dashboard_status(State(state): State<spectyn_mesh::AppState>) -> Json<Value> {
        Json(json!({
            "tools_count": state.tool_registry.names().len(),
            "hands_count": state.hands.names().len(),
            "active_sessions": 0,
            "uptime_seconds": state.started_at.elapsed().as_secs(),
            "total_requests": 0,
        }))
    }

    async fn provider_health(State(state): State<spectyn_mesh::AppState>) -> Json<Value> {
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
        State(state): State<spectyn_mesh::AppState>,
        axum::extract::Path(name): axum::extract::Path<String>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        let prompt = body["prompt"].as_str().unwrap_or("");
        let history = state.conversations.get_history("app").await;
        match state.agent_runtime.run(&name, prompt, &history, None).await {
            Ok(result) => {
                use spectyn_mesh::providers::traits::ChatMessage;
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
        Redirect::temporary(&spectyn_mesh::oauth::google_start_url(7878))
    }

    async fn oauth_apple_start() -> impl IntoResponse {
        match spectyn_mesh::oauth::apple_start_url(7878) {
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
        match spectyn_mesh::oauth::handle_callback(&code, &state_param).await {
            Ok(url) => Redirect::temporary(&url).into_response(),
            Err(e) => axum::response::Html(format!("<html><body>{}</body></html>", e)).into_response(),
        }
    }

    async fn oauth_result() -> Json<Value> {
        match spectyn_mesh::oauth::get_result() {
            Some(Ok(id)) => Json(json!({"ok": true, "identity": id})),
            Some(Err(e)) => Json(json!({"ok": false, "error": e})),
            None => Json(json!({"ok": false, "error": "no result yet"})),
        }
    }

    async fn oauth_apple_available() -> Json<Value> {
        Json(json!({"available": spectyn_mesh::oauth::apple_available()}))
    }

    async fn debug_send_message(
        State(state): State<spectyn_mesh::AppState>,
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
            config_dir.join(".spectyn-mesh").join("agents.toml"),
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
                    match toml::from_str::<spectyn_mesh::AgentsConfig>(c) {
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
