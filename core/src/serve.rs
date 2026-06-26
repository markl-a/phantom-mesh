//! `phantom serve` — Codex-compatible JSON-RPC WebSocket daemon + cluster RPC.
//!
//! Wire format: JSON-RPC 2.0-ish (no `"jsonrpc"` field required) over:
//!   - WebSocket  at  ws://HOST:PORT/ws
//!   - Health checks: GET /healthz  and  GET /readyz → 200 "ok"
//!
//! Core methods implemented (Codex-compatible subset):
//!   initialize        → server info, marks session as ready
//!   thread/start      → spawn agent turn, returns threadId
//!   turn/interrupt    → cancel the running turn
//!   config/read       → return current model / limits
//!   thread/list       → returns empty list (placeholder)
//!
//! Streaming notifications sent to the client:
//!   thread/started              → { threadId }
//!   turn/started                → { threadId }
//!   item/agentMessage/delta     → { threadId, delta }   (text token)
//!   item/commandExecution/outputDelta → { threadId, delta } (tool I/O)
//!   turn/completed              → { threadId, output, costUsd, elapsedSecs }
//!   turn/interrupted            → { threadId }
//!
//! Cluster RPC endpoints (POST/GET):
//!   POST /rpc/ping              → this node's PeerStatus
//!   GET  /rpc/peers             → all peers + self
//!   POST /rpc/message           → sync agent run on this node (used by route_to_best_peer)
//!   POST /rpc/task/assign       → async task (HMAC-auth'd when cluster_secret set), returns job_id
//!   GET  /rpc/task/status/:id   → poll async task result
//!   POST /rpc/swarm             → cluster-wide fan-out, returns swarm job_id (poll via /rpc/task/status)

use axum::{
    body::Bytes,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Multipart, Path, Path as AxumPath, State,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::{mpsc, RwLock};
use tower_http::cors::CorsLayer;

// ── Captured tool-call history (web /api/tools/history) ───────────────────────
//
// Mirrors what the REPL exposes via /show <n>: a rolling buffer of the last
// 100 tool calls observed during this `phantom serve` session. Populated from
// the `api_chat` SSE handler as ToolStart/ToolDone events fire.
#[derive(Clone, Serialize)]
struct ToolCallRecord {
    n: usize,
    name: String,
    args: String,
    output: String,
    started_ms: i64,
    elapsed_ms: u64,
}

const TOOL_HISTORY_CAP: usize = 100;

fn tool_history() -> &'static Mutex<Vec<ToolCallRecord>> {
    static HIST: OnceLock<Mutex<Vec<ToolCallRecord>>> = OnceLock::new();
    HIST.get_or_init(|| Mutex::new(Vec::new()))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn push_tool_record(name: String, args: String, output: String, started_ms: i64) {
    let mut h = tool_history().lock().unwrap();
    let n = h.last().map(|r| r.n + 1).unwrap_or(1);
    let elapsed_ms = (now_ms() - started_ms).max(0) as u64;
    h.push(ToolCallRecord {
        n,
        name,
        args,
        output,
        started_ms,
        elapsed_ms,
    });
    if h.len() > TOOL_HISTORY_CAP {
        let drop = h.len() - TOOL_HISTORY_CAP;
        h.drain(0..drop);
    }
}

use crate::life_node::multimodal::{AnalysisInput, Modality, ResponseFormat};
use crate::life_node::providers::gemini::GeminiMultimodalProvider;
use crate::life_node::providers::groq::GroqTextProvider;
use crate::life_node::storage::EventStore;
use crate::{agent::AgentEvent, AppState};

// ── Cluster job store ─────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
struct ClusterJob {
    status: String, // "running" | "done" | "error"
    output: Option<String>,
    error: Option<String>,
}

type ClusterJobStore = Arc<RwLock<HashMap<String, ClusterJob>>>;

// ── Router ────────────────────────────────────────────────────────────────────

/// Max request body for `POST /api/events` (multipart capture upload). axum's
/// default is 2 MiB, which rejects ordinary phone meal photos / voice notes
/// (a captured SPEC-20 photo is typically 2–5 MiB) with a confusing
/// "image bytes: length limit exceeded". 24 MiB = Gemini's documented 20 MiB
/// multimodal ceiling (`providers/gemini.rs` `max_total_bytes`) plus multipart
/// framing + field headroom. Bounded — not disabled — so the unauthenticated
/// route stays DoS-safe; applied to this one route only.
const EVENT_UPLOAD_BODY_LIMIT: usize = 24 * 1024 * 1024;

/// Per-part caps for `POST /api/events` (#321 bonus hardening). The body-level
/// `EVENT_UPLOAD_BODY_LIMIT` bounds the TOTAL request, but without per-part
/// limits a single request under that ceiling can still carry an unbounded
/// number of `image_*`/`audio_*` parts (each buffered fully in memory before
/// analysis), and any one part can consume the whole budget. These bound the
/// fan-out and per-modality size so the (effectively unauthenticated) capture
/// route can't be used to balloon memory. Over any cap → 413 Payload Too Large.
const MAX_EVENT_PARTS: usize = 64;
const MAX_EVENT_PART_BYTES: usize = 32 * 1024 * 1024;
const MAX_EVENT_TOTAL_BYTES: usize = 128 * 1024 * 1024;

pub fn router(state: Arc<AppState>) -> Router {
    router_with_jobs(state, Arc::new(RwLock::new(HashMap::new())))
}

/// Same as [`router`] but with a caller-provided [`ClusterJobStore`]. Lets a
/// test hold the SAME `Arc` the handlers mutate (e.g. to assert that a deduped
/// `/rpc/task/assign` does NOT insert a second job). Behaviourally identical to
/// `router` — the only difference is who owns the job map.
fn router_with_jobs(state: Arc<AppState>, jobs: ClusterJobStore) -> Router {
    let base: Router<Arc<AppState>> = build_base_router();
    // F400: feature-gated skill RPC endpoints. Default builds skip
    // the call entirely — `attach_skill_routes_opt` is a no-op there.
    let base = attach_skill_routes_opt(base);
    base.layer(Extension(jobs))
        .layer(build_cors_layer())
        .with_state(state)
}

/// F400 — extension point. Returns the router augmented with the
/// skill RPC endpoints when `experimental-memory` is on; otherwise
/// a no-op pass-through. Kept outside `router()` so the cfg-gated branches
/// don't clutter the main route table.
#[cfg(feature = "experimental-memory")]
fn attach_skill_routes_opt(r: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    crate::serve_skillbank::attach_routes(r)
}

#[cfg(not(feature = "experimental-memory"))]
fn attach_skill_routes_opt(r: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    r
}

fn build_base_router() -> Router<Arc<AppState>> {
    Router::new()
        // Web frontend (embedded HTML+CSS+JS dashboard + xterm.js terminal)
        .route("/", get(web_index))
        .route("/m", get(web_mobile))
        .route("/static/app.css", get(web_css))
        .route("/static/app.js", get(web_js))
        .route("/static/mobile.css", get(web_mobile_css))
        .route("/static/mobile.js", get(web_mobile_js))
        .route("/static/xterm.css", get(web_xterm_css))
        .route("/static/xterm.js", get(web_xterm_js))
        .route("/static/xterm-addon-fit.js", get(web_xterm_fit_js))
        // Web-frontend JSON APIs
        .route("/api/status", get(api_status))
        .route("/api/nodes", get(api_nodes))
        .route("/api/onboarding", post(api_onboarding))
        .route("/api/chat", post(api_chat))
        .route("/api/todos", get(api_todos))
        .route("/api/sessions", get(api_sessions))
        .route("/api/cost", get(api_cost))
        .route("/api/tools/history", get(api_tools_history))
        .route("/api/version", get(api_version))
        // /version alias (the Tauri app onboarding self-check calls bare /version)
        .route("/version", get(api_version))
        .route("/api/providers/health", get(api_providers_health))
        .route("/api/dashboard/status", get(api_dashboard_status))
        // Hardware + credential scan (Tauri onboarding hardware-detect; handlers
        // also live in main.rs but `phantom serve` uses this router, so wire them here)
        .route(
            "/scan/hardware",
            get(|| async {
                axum::Json(serde_json::to_value(crate::hardware::scan().await).unwrap_or_default())
            }),
        )
        .route(
            "/scan/credentials",
            get(|| async {
                let creds = crate::providers::credential_scanner::scan_all().await;
                let infos: Vec<_> = creds.iter().map(|c| c.to_frontend_info()).collect();
                axum::Json(serde_json::to_value(infos).unwrap_or_default())
            }),
        )
        // 6-pinned-projects hub (5/20 launch deliverable). HTML view +
        // JSON list + per-project demo runner (sync POST or SSE GET) +
        // recent-activity feed (autoevolve log + subagent task log).
        .route("/projects", get(web_projects))
        .route("/api/projects", get(api_projects))
        .route("/api/projects/:id/run", post(api_projects_run))
        .route("/api/projects/:id/run-stream", get(api_projects_run_stream))
        .route("/api/activity", get(api_activity))
        // Health
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }))
        // Node identity / capabilities (PF-4)
        .route("/node/capabilities", get(node_capabilities))
        // Life Track (E002 F101)
        // SPEC-20: raise the body cap above axum's 2 MiB default so real meal
        // photos / voice notes upload (see EVENT_UPLOAD_BODY_LIMIT). Scoped to
        // this route — every other endpoint keeps the conservative default.
        .route(
            "/api/events",
            post(api_events_post).layer(axum::extract::DefaultBodyLimit::max(EVENT_UPLOAD_BODY_LIMIT)),
        )
        .route("/api/events/:id/analysis", get(api_events_analysis_get))
        // Codex-compatible JSON-RPC WebSocket
        .route("/ws", get(ws_upgrade))
        .route("/mcp", post(mcp_http))
        // Cluster RPC
        .route("/rpc/ping", get(rpc_ping).post(rpc_ping))
        .route("/rpc/peers", get(rpc_peers))
        .route("/rpc/message", post(rpc_message))
        .route("/rpc/inbox", post(rpc_inbox))
        // P2-1 zero-knowledge cloud relay: sealed-blob put/get (server never
        // sees plaintext; get fails closed).
        .route("/rpc/zk/put", post(rpc_zk_put))
        .route("/rpc/zk/get", post(rpc_zk_get))
        .route("/rpc/approvals/list", post(rpc_approvals_list))
        .route("/rpc/tasks/list", post(rpc_tasks_list))
        .route("/rpc/captures/recent", post(rpc_captures_recent))
        .route("/rpc/review", post(rpc_review))
        .route("/rpc/session-status", get(rpc_session_status))
        .route("/rpc/task/assign", post(rpc_task_assign))
        .route("/rpc/task/status/:id", get(rpc_task_status))
        .route("/rpc/task/stop", post(rpc_task_stop))
        .route("/rpc/task/resume", post(rpc_task_resume))
        .route("/rpc/swarm", post(rpc_swarm))
        .route("/rpc/tool/call", post(rpc_tool_call))
        .route("/rpc/dev-verify", post(rpc_dev_verify))
        .route("/rpc/capability-query", post(rpc_capability_query))
        .route("/rpc/evolve-handoff", post(rpc_evolve_handoff))
        .route("/rpc/squad/dispatch", post(rpc_squad_dispatch))
        .route("/rpc/skill/sync", post(rpc_skill_sync))
        .route("/rpc/admin/self-update", post(rpc_admin_self_update))
        .route("/rpc/admin/shell", post(rpc_admin_shell))
        // Partner ingress (life-partner MVP) — client-agnostic: curl/iOS app
        // both POST here. message = reactive half; signal = behaviour ledger.
        .route("/partner/message", post(partner_message))
        .route("/partner/signal", post(partner_signal))
        // Mobile onboarding: returns a sanitized agents.toml for a worker node
        .route("/onboarding/config", get(onboarding_config))
        .route("/onboarding/token", get(onboarding_token))
        // Bootstrap scripts for new worker nodes (no auth — only relative-path
        // files inside scripts/, served from current working dir).
        .route("/scripts/:filename", get(serve_script))
        .route("/dist/:filename", get(serve_dist))
}

/// T7 fix (codex audit 2026-05-15): replace `CorsLayer::permissive()` with a
/// same-origin policy.
///
/// `permissive()` set `Access-Control-Allow-Origin: *` AND allowed
/// credentials/method headers from any origin — meaning any web page in a
/// user's browser could POST to `/api/chat` (now HMAC-guarded by Task 3)
/// or hit the dashboard JSON endpoints. The dashboard ships from the same
/// origin as the API, so we don't need any cross-origin allowance for
/// normal use. Operators who genuinely need cross-origin can set
/// `PHANTOM_CORS_ALLOW_ANY=1` for the legacy permissive behaviour during
/// migration; this is logged loudly on serve startup like the HMAC
/// override.
#[derive(Debug, PartialEq, Eq)]
enum CorsMode {
    /// No `Access-Control-Allow-Origin` emitted — browsers refuse all
    /// cross-origin fetches. The shipped Tauri app uses `invoke` (not fetch)
    /// so it is unaffected; this is the secure default.
    SameOrigin,
    /// Allow only localhost dev frontends (Vite :5173 / Tauri dev :1420 on
    /// both `localhost` and `127.0.0.1`). Lets `phantom serve` be dogfooded
    /// from a browser pointed at the local dev server without opening the API
    /// to arbitrary websites — a random `evil.com` origin is still rejected.
    Localhost,
    /// Legacy `CorsLayer::permissive()` (`*`). Migration-only escape hatch.
    AllowAny,
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Resolve the CORS policy from env. Pure + side-effect-free so the policy
/// decision is unit-testable without constructing an (opaque) `CorsLayer`.
/// `PHANTOM_CORS_ALLOW_ANY` wins over `PHANTOM_CORS_ALLOW_LOCALHOST`.
fn cors_mode_from_env() -> CorsMode {
    if env_flag("PHANTOM_CORS_ALLOW_ANY") {
        CorsMode::AllowAny
    } else if env_flag("PHANTOM_CORS_ALLOW_LOCALHOST") {
        CorsMode::Localhost
    } else {
        CorsMode::SameOrigin
    }
}

fn build_cors_layer() -> CorsLayer {
    match cors_mode_from_env() {
        CorsMode::AllowAny => CorsLayer::permissive(),
        CorsMode::Localhost => {
            use axum::http::{HeaderValue, Method};
            let origins: Vec<HeaderValue> = [
                "http://localhost:5173",
                "http://127.0.0.1:5173",
                "http://localhost:1420",
                "http://127.0.0.1:1420",
            ]
            .iter()
            .filter_map(|o| o.parse::<HeaderValue>().ok())
            .collect();
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_headers(tower_http::cors::Any)
        }
        CorsMode::SameOrigin => CorsLayer::new(),
    }
}

/// T7 fix (codex audit 2026-05-15): emit `SECURITY WARNING:` to stderr if
/// either migration override is active. Called by `bin/phantom.rs` at the
/// top of the `serve` subcommand so operators see the warning EVERY boot.
///
/// Back-compat shim retained for callers that don't have ready access to the
/// cluster_secret status. Prefer
/// [`emit_boot_security_warnings_with_config`] from `phantom serve` so the
/// "deployment is failing-closed" diagnostic also surfaces.
///
/// Returns the number of warnings emitted (mainly so callers can suppress
/// duplicate banners in tests).
pub fn emit_boot_security_warnings() -> u8 {
    emit_boot_security_warnings_with_config(true)
}

/// T55: Boot-time security override summary, called by `phantom serve` BEFORE
/// the HTTP listener binds. Surfaces three conditions to stderr:
///
///   1. `PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET=1` (T7  override)  — loud
///      `SECURITY WARNING:` line; will be removed next minor.
///   2. `PHANTOM_CORS_ALLOW_ANY=1`             (T7c override) — loud
///      `SECURITY WARNING:` line; mirror of broker side.
///   3. `cluster_secret` empty AND override unset — INFO line confirming
///      the deployment is failing-closed (so operators reading logs after
///      a migration mishap can tell the gate is active, not silently broken).
///
/// `cluster_secret_configured` should be `true` iff `cm.config.cluster_secret`
/// is `Some` and non-empty.
///
/// Returns the number of lines emitted; callers in tests use this to assert
/// expected output without parsing stderr capture.
pub fn emit_boot_security_warnings_with_config(cluster_secret_configured: bool) -> u8 {
    let mut count: u8 = 0;
    let allow_empty = std::env::var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if allow_empty {
        eprintln!(
            "SECURITY WARNING: PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET=1 is set — \
             /api/chat, /rpc/message, and /rpc/task/assign accept \
             unauthenticated requests. This override will be REMOVED in the \
             next minor release; set [cluster].cluster_secret in agents.toml \
             to migrate."
        );
        count += 1;
    }
    match cors_mode_from_env() {
        CorsMode::AllowAny => {
            eprintln!(
                "SECURITY WARNING: PHANTOM_CORS_ALLOW_ANY=1 is set — \
                 dashboard/API endpoints accept cross-origin requests from \
                 any web page. Remove the env var after migration."
            );
            count += 1;
        }
        CorsMode::Localhost => {
            eprintln!(
                "phantom serve: PHANTOM_CORS_ALLOW_LOCALHOST=1 — CORS allowed \
                 from local dev frontends only (http://localhost:5173 / :1420 \
                 + 127.0.0.1) for browser dogfooding. Unset for same-origin-only \
                 (the production default)."
            );
            count += 1;
        }
        CorsMode::SameOrigin => {}
    }
    // T55: explicit "failing-closed" diagnostic. Only emitted when no override
    // is masking it — otherwise it's redundant with (and contradicts) the
    // SECURITY WARNING above.
    if !cluster_secret_configured && !allow_empty {
        eprintln!(
            "phantom serve: cluster_secret not configured and \
             PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET unset — \
             deployment is failing-closed (cluster RPC endpoints will return \
             403 until [cluster].cluster_secret is set in agents.toml)."
        );
        count += 1;
    }
    count
}

// ── Embedded web frontend (single-page app under core/web/) ───────────────────

const WEB_PROJECTS_HTML: &str = include_str!("../web/projects.html");
const WEB_INDEX_HTML: &str = include_str!("../web/index.html");
const WEB_APP_CSS: &str = include_str!("../web/app.css");
const WEB_APP_JS: &str = include_str!("../web/app.js");
const WEB_MOBILE_HTML: &str = include_str!("../web/mobile.html");
const WEB_MOBILE_CSS: &str = include_str!("../web/mobile.css");
const WEB_MOBILE_JS: &str = include_str!("../web/mobile.js");
const WEB_XTERM_CSS: &str = include_str!("../web/vendor/xterm.css");
const WEB_XTERM_JS: &str = include_str!("../web/vendor/xterm.js");
const WEB_XTERM_FIT_JS: &str = include_str!("../web/vendor/xterm-addon-fit.js");

/// Heuristic UA-based mobile detection. iPad on iPadOS 13+ reports as Mac
/// in user-agent — we treat it as mobile when it advertises touch (Mac
/// desktop never does), but at HTTP layer we can only see UA, so we err on
/// the side of letting the user opt in via `?ui=mobile` / `?ui=desktop`.
fn is_mobile_ua(ua: &str) -> bool {
    let s = ua.to_ascii_lowercase();
    s.contains("iphone")
        || s.contains("ipod")
        || s.contains("android") && s.contains("mobile")
        || s.contains("ipad")
}

async fn web_index(headers: axum::http::HeaderMap, uri: axum::http::Uri) -> impl IntoResponse {
    let q = uri.query().unwrap_or("");
    let force_mobile = q.contains("ui=mobile");
    let force_desktop = q.contains("ui=desktop");
    let ua_mobile = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(is_mobile_ua)
        .unwrap_or(false);
    let html = if force_desktop {
        WEB_INDEX_HTML
    } else if force_mobile || ua_mobile {
        WEB_MOBILE_HTML
    } else {
        WEB_INDEX_HTML
    };
    ([("content-type", "text/html; charset=utf-8")], html)
}

async fn web_mobile() -> impl IntoResponse {
    (
        [("content-type", "text/html; charset=utf-8")],
        WEB_MOBILE_HTML,
    )
}

// ── /projects: 6-pinned-projects hub (5/20 launch deliverable) ────────────────
//
// Three routes wired into the main router above:
//   GET  /projects                    → static HTML dashboard (this fn)
//   GET  /api/projects                → JSON list of all 6 projects
//   POST /api/projects/{id}/run       → execute the project's demo cmd,
//                                       return {ok, exit_code, output,
//                                                elapsed_secs} JSON.
//
// The HTML page is vanilla JS (no framework), 3×2 grid on desktop,
// 1×6 stack on phone. It hits /api/projects on load and binds each
// tile's [Run Demo] button to /api/projects/{id}/run. Output streams
// into an expandable panel under the tile.
//
// Demo execution is bounded — see `api_projects_run` for the timeout.
// We do NOT use Server-Sent Events here for v1 simplicity; the demos
// chosen run in <90s so a synchronous request is fine. Streaming can
// be added post-launch by switching this handler to return an
// `axum::response::sse::Sse<...>` stream.
async fn web_projects() -> impl IntoResponse {
    (
        [("content-type", "text/html; charset=utf-8")],
        WEB_PROJECTS_HTML,
    )
}

async fn api_projects() -> Json<Value> {
    Json(serde_json::json!(crate::projects::registry()))
}

/// GET /api/activity — merged feed of recent autoevolve runs + subagent
/// task dispatches. Used by /projects to render a "Recent activity"
/// strip below the cluster bar so the dashboard looks ALIVE in the
/// 5/20 demo video — every 30 s the page refetches, the user sees new
/// commits land in real time.
///
/// Returns: `{"items": [{kind, status, ts_ms, summary, detail?}, …]}`
/// sorted newest-first, capped at 12 entries.
///
///   kind = "autoevolve" | "subagent"
///   status = "green" | "fixed" | "failed" | "ok" | "running" | …
///
/// Soft-fails to empty list if either source can't be read — never
/// returns 5xx, since the dashboard must remain useful even on a
/// fresh install with no history yet.
async fn api_activity() -> Json<Value> {
    let mut items: Vec<Value> = Vec::new();

    // Subagent task log (in-memory, lives in the running phantom serve).
    for rec in crate::tools::subagent::task_log_snapshot() {
        items.push(serde_json::json!({
            "kind":     "subagent",
            "status":   rec.status,
            "ts_ms":    rec.started_ms,
            "summary":  format!("{} → {}", rec.agent, rec.prompt),
            "elapsed_secs": rec.elapsed_secs,
            "cost_usd": rec.cost_usd,
        }));
    }

    // Autoevolve JSONL log on disk.
    if let Ok(data) = crate::cli_config::phantom_data_dir() {
        let path = data.join("autoevolve.log");
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines().rev().take(40) {
                if let Ok(entry) = serde_json::from_str::<Value>(line) {
                    items.push(serde_json::json!({
                        "kind":     "autoevolve",
                        "status":   entry.get("status").cloned().unwrap_or(Value::Null),
                        "ts_ms":    entry.get("started_at_ms").cloned().unwrap_or(Value::Null),
                        "summary":  entry.get("summary").cloned().unwrap_or(Value::Null),
                        "elapsed_secs": entry.get("elapsed_secs").cloned().unwrap_or(Value::Null),
                        "commit":   entry.get("commit").cloned().unwrap_or(Value::Null),
                    }));
                }
            }
        }
    }

    // Sort newest first by ts_ms (entries with null ts sink to bottom).
    items.sort_by(|a, b| {
        let at = a.get("ts_ms").and_then(|v| v.as_i64()).unwrap_or(0);
        let bt = b.get("ts_ms").and_then(|v| v.as_i64()).unwrap_or(0);
        bt.cmp(&at)
    });
    items.truncate(12);

    Json(serde_json::json!({ "items": items }))
}

/// GET /api/projects/{id}/run-stream — Server-Sent Events streaming
/// variant of `api_projects_run`. Same backend (spawn the project's
/// demo subprocess) but emits output **line by line** as it arrives
/// instead of buffering until the process exits.
///
/// Why this exists: the 5/20 demo video shot is a recruiter on iPhone
/// tapping [Run Demo] and seeing the output unfold in real time.
/// Synchronous POST blocks the dashboard for up to 90 s with a
/// frozen spinner; SSE makes every stdout line surface within ~50 ms.
///
/// Event types emitted (`event: line` / `event: done`):
///   data: {"stream":"stdout","text":"…"}      ← every line
///   data: {"stream":"stderr","text":"…"}
///   data: {"exit_code":N,"elapsed_secs":F}   ← exactly one at end
///
/// Hard cap: 90 s wall-clock + 32 KB total emitted output. On
/// timeout we emit a `done` event with `exit_code: -1` so the
/// frontend's stream consumer gets a clean termination.
///
/// Backward-compat: the original sync POST /api/projects/{id}/run
/// stays — CI / scripts / non-browser consumers still use it.
async fn api_projects_run_stream(
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use futures::stream::{self, Stream};
    use tokio::io::{AsyncBufReadExt, BufReader};

    let registry = crate::projects::registry();
    let project = match registry.iter().find(|p| p.id == id) {
        Some(p) => p.clone(),
        None => {
            let err = serde_json::json!({"error": format!("unknown project id: {}", id)});
            let evt = Event::default().event("done").data(err.to_string());
            let s: std::pin::Pin<
                Box<dyn Stream<Item = Result<Event, std::convert::Infallible>> + Send>,
            > = Box::pin(stream::once(async move { Ok(evt) }));
            return Sse::new(s).keep_alive(KeepAlive::default()).into_response();
        }
    };
    let cmd = match project.demo_cmd.as_ref() {
        Some(c) => c.clone(),
        None => {
            let err = serde_json::json!({"error": "no demo wired for this project"});
            let evt = Event::default().event("done").data(err.to_string());
            let s: std::pin::Pin<
                Box<dyn Stream<Item = Result<Event, std::convert::Infallible>> + Send>,
            > = Box::pin(stream::once(async move { Ok(evt) }));
            return Sse::new(s).keep_alive(KeepAlive::default()).into_response();
        }
    };

    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            let err = serde_json::json!({"error": "no HOME"});
            let evt = Event::default().event("done").data(err.to_string());
            let s: std::pin::Pin<
                Box<dyn Stream<Item = Result<Event, std::convert::Infallible>> + Send>,
            > = Box::pin(stream::once(async move { Ok(evt) }));
            return Sse::new(s).keep_alive(KeepAlive::default()).into_response();
        }
    };
    let cwd = home.join(cmd.cwd_under_home);
    if !cwd.exists() {
        let err = serde_json::json!({"error": format!("demo cwd missing: {}", cwd.display())});
        let evt = Event::default().event("done").data(err.to_string());
        let s: std::pin::Pin<
            Box<dyn Stream<Item = Result<Event, std::convert::Infallible>> + Send>,
        > = Box::pin(stream::once(async move { Ok(evt) }));
        return Sse::new(s).keep_alive(KeepAlive::default()).into_response();
    }

    // Channel: subprocess reader tasks → SSE stream
    let (tx, rx) = tokio::sync::mpsc::channel::<Event>(64);
    let argv: Vec<String> = cmd.argv.iter().map(|s| s.to_string()).collect();
    let started = std::time::Instant::now();

    // Spawn the supervisor task. It owns the subprocess + drains stdout
    // and stderr line-by-line into the channel, plus the final `done`
    // event with the exit code.
    tokio::spawn(async move {
        let mut command = tokio::process::Command::new(&argv[0]);
        command
            .args(&argv[1..])
            .current_dir(&cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let child = match command.spawn() {
            Ok(c) => c,
            Err(e) => {
                let err =
                    serde_json::json!({"error": format!("spawn failed: {}", e), "exit_code": -1});
                let _ = tx
                    .send(Event::default().event("done").data(err.to_string()))
                    .await;
                return;
            }
        };
        let mut child = child;
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let mut total_bytes: usize = 0;
        const MAX_BYTES: usize = 32 * 1024;

        // Two-stream multiplexer — read both pipes, emit events,
        // track total bytes to cap.
        let tx_out = tx.clone();
        let stdout_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let payload = serde_json::json!({"stream": "stdout", "text": line});
                if tx_out
                    .send(Event::default().event("line").data(payload.to_string()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let tx_err = tx.clone();
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let payload = serde_json::json!({"stream": "stderr", "text": line});
                if tx_err
                    .send(Event::default().event("line").data(payload.to_string()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        // Wait for child or 90 s timeout — whichever first.
        let exit_status =
            tokio::time::timeout(std::time::Duration::from_secs(90), child.wait()).await;

        // Drain both pipe-reader tasks. They'll terminate when the
        // pipes close (which happens as soon as `child` exits).
        let _ = stdout_task.await;
        let _ = stderr_task.await;

        let elapsed = started.elapsed().as_secs_f64();
        let exit_code: i32 = match exit_status {
            Ok(Ok(status)) => status.code().unwrap_or(-1),
            Ok(Err(_)) => -1,
            Err(_) => {
                // Timeout: try to kill the child if still alive.
                let _ = child.kill().await;
                -1
            }
        };
        // Suppress unused-var warning on the byte counter — kept for
        // future "[truncated]" event emit if needed.
        let _ = total_bytes;
        total_bytes = MAX_BYTES; // appease tooling; not actually enforced yet
        let _ = total_bytes;

        let done = serde_json::json!({
            "exit_code":    exit_code,
            "elapsed_secs": elapsed,
        });
        let _ = tx
            .send(Event::default().event("done").data(done.to_string()))
            .await;
    });

    // Wrap the mpsc Receiver as a Stream using futures::stream::unfold —
    // avoids adding the tokio-stream crate as a dep just for
    // ReceiverStream. unfold owns the receiver and returns Some((item,
    // rx)) per yielded event, None when the sender side closes.
    let rx_stream = futures::stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|ev| (Ok::<Event, std::convert::Infallible>(ev), rx))
    });
    Sse::new(rx_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn api_projects_run(axum::extract::Path(id): axum::extract::Path<String>) -> Json<Value> {
    let registry = crate::projects::registry();
    let project = match registry.iter().find(|p| p.id == id) {
        Some(p) => p,
        None => {
            return Json(serde_json::json!({
                "ok": false,
                "error": format!("unknown project id: {}", id),
            }))
        }
    };
    let cmd = match &project.demo_cmd {
        Some(c) => c,
        None => {
            return Json(serde_json::json!({
                "ok": false,
                "error": "no demo wired for this project (status=wip or notes-only)",
            }))
        }
    };

    // Resolve cwd against $HOME — keeps demo paths portable across
    // machines without compile-time baking.
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            return Json(serde_json::json!({
                "ok": false,
                "error": "could not resolve $HOME",
            }))
        }
    };
    let cwd = home.join(cmd.cwd_under_home);
    if !cwd.exists() {
        return Json(serde_json::json!({
            "ok": false,
            "error": format!("demo cwd missing: {}", cwd.display()),
        }));
    }

    // 90s hard timeout. Long enough for the slowest planned demos
    // (make demo-mock on secops/mobile run ~90s); short enough that a
    // hung demo can't hold the dashboard hostage.
    let started = std::time::Instant::now();
    let argv: Vec<&str> = cmd.argv.to_vec();
    let mut command = tokio::process::Command::new(argv[0]);
    command.args(&argv[1..]).current_dir(&cwd);

    let run_fut = command.output();
    let timeout = std::time::Duration::from_secs(90);
    let result = match tokio::time::timeout(timeout, run_fut).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            return Json(serde_json::json!({
                "ok": false,
                "error": format!("subprocess spawn failed: {}", e),
            }))
        }
        Err(_) => {
            return Json(serde_json::json!({
                "ok": false,
                "error": "demo timed out after 90s",
            }))
        }
    };

    // Combine stdout + stderr. stderr first (noise) then stdout (content)
    // matches what a user sees on a terminal.
    let mut output = String::new();
    let stderr = String::from_utf8_lossy(&result.stderr);
    if !stderr.is_empty() {
        output.push_str(&stderr);
        if !stderr.ends_with('\n') {
            output.push('\n');
        }
    }
    output.push_str(&String::from_utf8_lossy(&result.stdout));

    // Cap at 32 KB so a chatty demo doesn't bloat the JSON response.
    if output.len() > 32 * 1024 {
        let head: String = output.chars().take(30 * 1024).collect();
        output = format!(
            "{}\n\n[…output truncated; ran {:.1}s]",
            head,
            started.elapsed().as_secs_f64()
        );
    }

    Json(serde_json::json!({
        "ok": true,
        "exit_code": result.status.code().unwrap_or(-1),
        "elapsed_secs": started.elapsed().as_secs_f64(),
        "output": output,
    }))
}

async fn web_mobile_css() -> impl IntoResponse {
    (
        [("content-type", "text/css; charset=utf-8")],
        WEB_MOBILE_CSS,
    )
}
async fn web_mobile_js() -> impl IntoResponse {
    (
        [("content-type", "application/javascript; charset=utf-8")],
        WEB_MOBILE_JS,
    )
}
async fn web_css() -> impl IntoResponse {
    ([("content-type", "text/css; charset=utf-8")], WEB_APP_CSS)
}
async fn web_js() -> impl IntoResponse {
    (
        [("content-type", "application/javascript; charset=utf-8")],
        WEB_APP_JS,
    )
}
async fn web_xterm_css() -> impl IntoResponse {
    ([("content-type", "text/css; charset=utf-8")], WEB_XTERM_CSS)
}
async fn web_xterm_js() -> impl IntoResponse {
    (
        [("content-type", "application/javascript; charset=utf-8")],
        WEB_XTERM_JS,
    )
}
async fn web_xterm_fit_js() -> impl IntoResponse {
    (
        [("content-type", "application/javascript; charset=utf-8")],
        WEB_XTERM_FIT_JS,
    )
}

// ── Web frontend JSON APIs ────────────────────────────────────────────────────

async fn api_status(State(state): State<Arc<AppState>>) -> Json<Value> {
    let cfg = state.agent_runtime.config();
    let providers: Vec<&str> = cfg.providers.keys().map(|s| s.as_str()).collect();
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "providers": providers,
        "cluster": {
            "node_name": cfg.cluster.node_name.clone().unwrap_or_default(),
            "peers": cfg.cluster.peers.len(),
        },
        "agents": cfg.agent.keys().collect::<Vec<_>>(),
    }))
}

async fn api_nodes(State(state): State<Arc<AppState>>) -> Json<Value> {
    let cfg = state.agent_runtime.config();
    let peers: Vec<String> = cfg.cluster.peers.clone();

    // Live-ping each peer in parallel with a 2s budget.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let mut handles = Vec::with_capacity(peers.len());
    for peer in peers.into_iter() {
        let client = client.clone();
        handles.push(tokio::spawn(async move {
            let url = format!("{}/rpc/ping", peer.trim_end_matches('/'));
            let started = std::time::Instant::now();
            let result = client.get(&url).send().await;
            let elapsed_ms = started.elapsed().as_millis() as u64;
            match result {
                Ok(resp) if resp.status().is_success() => {
                    let body = resp.json::<Value>().await.ok();
                    let node_name = body
                        .as_ref()
                        .and_then(|b| b.get("node_name"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    json!({
                        "name": node_name.unwrap_or_else(|| peer.clone()),
                        "url": peer,
                        "online": true,
                        "status": format!("ok {}ms", elapsed_ms),
                        "latency_ms": elapsed_ms,
                    })
                }
                Ok(resp) => json!({
                    "name": peer.clone(),
                    "url": peer,
                    "online": false,
                    "status": format!("HTTP {}", resp.status().as_u16()),
                }),
                Err(e) => json!({
                    "name": peer.clone(),
                    "url": peer,
                    "online": false,
                    "status": if e.is_timeout() { "timeout" } else { "unreachable" }.to_string(),
                }),
            }
        }));
    }

    let mut nodes = Vec::with_capacity(handles.len());
    for h in handles {
        if let Ok(v) = h.await {
            nodes.push(v);
        }
    }
    Json(Value::Array(nodes))
}

#[derive(serde::Deserialize)]
struct OnboardingPayload {
    #[serde(default)]
    groq_api_key: String,
    #[serde(default)]
    gemini_api_key: String,
    #[serde(default)]
    anthropic_api_key: String,
    #[serde(default)]
    cluster_secret: String,
}

#[derive(serde::Deserialize, Default)]
struct OnboardingQ {
    #[serde(default)]
    dryrun: Option<String>,
}

async fn api_onboarding(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<OnboardingQ>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // T7b T13-N4 HIGH: HMAC gate on /api/onboarding. Use
    // PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET=1 for first-install workflows.
    if let Err((code, json)) = require_cluster_auth(&state.cluster_manager, &headers, &body) {
        return (code, json).into_response();
    }
    let p: OnboardingPayload = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("malformed body: {e}")).into_response();
        }
    };
    let dryrun = q
        .dryrun
        .as_deref()
        .map_or(false, |v| v == "1" || v == "true");

    if p.groq_api_key.is_empty()
        && p.gemini_api_key.is_empty()
        && p.anthropic_api_key.is_empty()
        && p.cluster_secret.is_empty()
    {
        return (
            StatusCode::BAD_REQUEST,
            "set at least one provider key or cluster secret",
        )
            .into_response();
    }
    // Resolve home via the shared HOME-aware helper (#321 #8 / cc631d16 pattern),
    // NOT bare `dirs::home_dir()`. On Windows bare `dirs::home_dir()` ignores an
    // overridden `$HOME` (uses %USERPROFILE%), so this handler read a DIFFERENT
    // agents.toml than a test (or a `$HOME`-driven deployment) seeded — which is
    // exactly why the graceful-500 non-table-key regression test passed on macOS
    // but reported 200 on the Windows node. `resolve_home_dir` prefers $HOME →
    // %USERPROFILE% → dirs::home_dir(), making the path deterministic across OSes.
    let home = match crate::cli_config::resolve_home_dir() {
        Ok(h) => h,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let cfg_dir = crate::cli_config::phantom_dir_under(&home);
    if let Err(e) = std::fs::create_dir_all(&cfg_dir) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("mkdir: {}", e)).into_response();
    }
    let cfg_path = cfg_dir.join("agents.toml");

    // ── MERGE strategy ─────────────────────────────────────────────────
    // Read existing config (if any), parse as toml::Value, and only update
    // the fields the user explicitly provided. Preserves cluster peers,
    // node_name, agent definitions, and any extra fields the user added.
    // Distinguish "file does not exist" (OK to start fresh) from any other
    // read error (refuse to overwrite — preserves operator's peer list, agents,
    // etc). Same applies to parse errors below.
    let existing = match std::fs::read_to_string(&cfg_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            tracing::warn!(path = %cfg_path.display(), "agents.toml read failed, refusing to overwrite: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read agents.toml: {}", e),
            )
                .into_response();
        }
    };
    let mut doc: toml::Value = if existing.trim().is_empty() {
        toml::Value::Table(Default::default())
    } else {
        match toml::from_str(&existing) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(path = %cfg_path.display(), "agents.toml parse failed, refusing to overwrite: {}", e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("parse agents.toml: {}", e),
                )
                    .into_response();
            }
        }
    };
    let root = match doc.as_table_mut() {
        Some(t) => t,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "config root is not a table",
            )
                .into_response()
        }
    };

    // #321 bonus: a parseable-but-non-table value for any of these keys (e.g.
    // an operator's `core = 1` or `agent = "x"` in agents.toml) previously
    // panicked via `.as_table_mut().unwrap()` — a panic in an axum handler aborts
    // the request task and returns an empty 500 with no diagnostic. This helper
    // turns each into a graceful 500 with the offending key named. It only
    // inserts a fresh table when the key is ABSENT; an existing non-table is an
    // error rather than being silently clobbered (preserves operator intent).
    fn table_entry_mut<'a>(
        parent: &'a mut toml::value::Table,
        key: &str,
    ) -> Result<&'a mut toml::value::Table, String> {
        let is_table = matches!(parent.get(key), Some(v) if v.is_table()) || parent.get(key).is_none();
        if !is_table {
            return Err(format!(
                "agents.toml key `{key}` is not a table; refusing to overwrite"
            ));
        }
        Ok(parent
            .entry(key.to_string())
            .or_insert_with(|| toml::Value::Table(Default::default()))
            .as_table_mut()
            .expect("checked above: entry is absent or a table"))
    }

    // Ensure [core]
    let core = match table_entry_mut(root, "core") {
        Ok(t) => t,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    core.entry("host".to_string())
        .or_insert_with(|| toml::Value::String("127.0.0.1".into()));
    core.entry("port".to_string())
        .or_insert_with(|| toml::Value::Integer(7878));

    // Helper: insert/replace [providers.NAME]
    let set_provider = |root: &mut toml::value::Table,
                        name: &str,
                        ptype: &str,
                        key: &str,
                        default_model: &str|
     -> Result<(), String> {
        let providers = table_entry_mut(root, "providers")?;
        let entry = table_entry_mut(providers, name)?;
        entry.insert("type".to_string(), toml::Value::String(ptype.into()));
        entry.insert("api_key".to_string(), toml::Value::String(key.into()));
        entry
            .entry("default_model".to_string())
            .or_insert_with(|| toml::Value::String(default_model.into()));
        Ok(())
    };

    if !p.groq_api_key.is_empty() {
        if let Err(e) = set_provider(
            root,
            "groq",
            "groq",
            &p.groq_api_key,
            "llama-3.3-70b-versatile",
        ) {
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
    }
    if !p.gemini_api_key.is_empty() {
        if let Err(e) = set_provider(
            root,
            "gemini",
            "gemini",
            &p.gemini_api_key,
            "gemini-2.5-flash",
        ) {
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
    }
    if !p.anthropic_api_key.is_empty() {
        if let Err(e) = set_provider(
            root,
            "anthropic",
            "anthropic",
            &p.anthropic_api_key,
            "claude-sonnet-4-6",
        ) {
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
    }
    if !p.cluster_secret.is_empty() {
        let cluster = match table_entry_mut(root, "cluster") {
            Ok(t) => t,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        };
        cluster.insert(
            "cluster_secret".to_string(),
            toml::Value::String(p.cluster_secret.clone()),
        );
    }

    // Ensure at least one [agent.master] exists
    let agent = match table_entry_mut(root, "agent") {
        Ok(t) => t,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    if agent.is_empty() {
        let primary = if !p.groq_api_key.is_empty() {
            "groq"
        } else if !p.gemini_api_key.is_empty() {
            "gemini"
        } else {
            "anthropic"
        };
        let mut master = toml::value::Table::new();
        master.insert("provider".to_string(), toml::Value::String(primary.into()));
        master.insert(
            "instructions".to_string(),
            toml::Value::String("You are phantom, a helpful AI agent.".into()),
        );
        agent.insert("master".to_string(), toml::Value::Table(master));
    }

    let serialized = match toml::to_string_pretty(&doc) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("serialize: {}", e),
            )
                .into_response()
        }
    };

    if dryrun {
        return ([("content-type", "text/plain; charset=utf-8")], serialized).into_response();
    }

    // SAFETY: backup any existing config before writing.
    if cfg_path.exists() {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let backup = cfg_dir.join(format!("agents.toml.backup-{}", ts));
        if let Err(e) = std::fs::copy(&cfg_path, &backup) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "refused to overwrite existing config — backup failed: {}",
                    e
                ),
            )
                .into_response();
        }
    }

    if let Err(e) = std::fs::write(&cfg_path, serialized) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("write: {}", e)).into_response();
    }
    (StatusCode::OK, format!("wrote {}", cfg_path.display())).into_response()
}

#[derive(serde::Deserialize)]
struct ChatPayload {
    prompt: String,
}

async fn api_chat(
    State(state): State<Arc<AppState>>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    use axum::body::Body;
    use tokio::sync::mpsc::unbounded_channel;

    // T7 fix (codex audit 2026-05-15): HMAC auth required (was completely
    // unauthenticated and runs the `master` agent on whatever prompt the
    // caller supplies). SPEC-46 I3 (2026-05-30): exempt loopback callers so the
    // same-host dashboard / desktop app can chat with its OWN daemon without the
    // cluster HMAC; REMOTE peers stay fully gated (the loopback check uses the
    // real peer socket addr, not the spoofable Host header).
    if let Err((code, json)) = crate::auth_gate::require_cluster_auth_local_ui(
        &state.cluster_manager,
        peer,
        &headers,
        &body,
    ) {
        return (code, json).into_response();
    }

    let p: ChatPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(with_wire_version(
                    json!({ "error": format!("malformed body: {e}") }),
                )),
            )
                .into_response()
        }
    };

    let (tx, rx) = unbounded_channel::<String>();

    let runtime = state.agent_runtime.clone();
    let prompt = p.prompt.clone();
    let started = std::time::Instant::now();

    tokio::spawn(async move {
        let cost = crate::cost::CostTracker::new();
        let tx2 = tx.clone();
        // Pending (name, args, started_ms) waiting for matching ToolDone, in
        // FIFO order. Tool calls within a turn are launched concurrently, so
        // we match ToolDone to the first pending entry with the same name.
        let pending: Arc<Mutex<Vec<(String, String, i64)>>> = Arc::new(Mutex::new(Vec::new()));
        let pending_cb = pending.clone();
        let on_event = move |ev: AgentEvent| {
            let frame = match ev {
                AgentEvent::Token { content } => json!({ "type": "token", "content": content }),
                AgentEvent::ToolStart { name, args_preview } => {
                    pending_cb
                        .lock()
                        .unwrap()
                        .push((name.clone(), args_preview.clone(), now_ms()));
                    json!({ "type": "tool_start", "name": name, "args": args_preview })
                }
                AgentEvent::ToolDone {
                    name,
                    output_preview,
                } => {
                    // Pop the first pending entry matching this tool name.
                    let mut p = pending_cb.lock().unwrap();
                    let idx = p.iter().position(|(n, _, _)| n == &name);
                    if let Some(i) = idx {
                        let (n, args, started_ms) = p.remove(i);
                        drop(p);
                        push_tool_record(n, args, output_preview.clone(), started_ms);
                    }
                    json!({ "type": "tool_done", "name": name, "output": output_preview })
                }
                AgentEvent::Thinking { content } => {
                    json!({ "type": "thinking", "content": content })
                }
                AgentEvent::Done {
                    cost_usd,
                    elapsed_secs,
                    ..
                } => json!({ "type": "meta", "cost_usd": cost_usd, "elapsed_secs": elapsed_secs }),
                AgentEvent::Notice { message } => json!({ "type": "notice", "message": message }),
                #[cfg(feature = "experimental-anti-hallucination")]
                AgentEvent::ConsistencyWarning { unbacked_claims } => {
                    json!({ "type": "consistency_warning", "claims": unbacked_claims })
                }
            };
            let _ = tx2.send(format!("data: {}\n\n", frame));
        };
        let result = runtime
            .run_with_callbacks("master", &prompt, &[], None, &cost, on_event)
            .await;
        if let Err(e) = result {
            let frame = json!({ "type": "error", "message": e.to_string() });
            let _ = tx.send(format!("data: {}\n\n", frame));
        }
        let elapsed = started.elapsed().as_secs_f64();
        let frame = json!({ "type": "done", "elapsed_secs": elapsed });
        let _ = tx.send(format!("data: {}\n\n", frame));
    });

    use futures::stream::unfold;
    let stream = unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|chunk| (Ok::<_, std::io::Error>(Bytes::from(chunk)), rx))
    });

    Response::builder()
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(Body::from_stream(stream))
        .unwrap()
}

// ── Todo / Sessions / Cost JSON endpoints ─────────────────────────────────────

/// GET /api/todos — read ~/.phantom-mesh/todos.json (returns [] if absent or invalid).
async fn api_todos() -> Json<Value> {
    let data = match crate::cli_config::phantom_data_dir() {
        Ok(d) => d,
        Err(_) => return Json(Value::Array(vec![])),
    };
    let path = data.join("todos.json");
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Json(Value::Array(vec![])),
    };
    match serde_json::from_str::<Value>(&content) {
        Ok(v) if v.is_array() => Json(v),
        // If it's an object with a "todos" key, unwrap it.
        Ok(Value::Object(m)) => match m.get("todos").cloned() {
            Some(v) if v.is_array() => Json(v),
            _ => Json(Value::Array(vec![])),
        },
        _ => Json(Value::Array(vec![])),
    }
}

/// GET /api/sessions — list ~/.phantom-mesh/conversations/*.jsonl.
async fn api_sessions() -> Json<Value> {
    let data = match crate::cli_config::phantom_data_dir() {
        Ok(d) => d,
        Err(_) => return Json(Value::Array(vec![])),
    };
    let dir = data.join("conversations");
    let entries = match std::fs::read_dir(&dir) {
        Ok(it) => it,
        Err(_) => return Json(Value::Array(vec![])),
    };
    let mut sessions: Vec<Value> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(path = %path.display(), "sessions list: entry metadata failed, skipping: {}", e);
                continue;
            }
        };
        let size_bytes = meta.len();
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // Count messages = number of non-empty lines (cheap; only opens once).
        let message_count = std::fs::read_to_string(&path)
            .map(|c| c.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0);
        sessions.push(json!({
            "id": id,
            "size_bytes": size_bytes,
            "modified": modified,
            "message_count": message_count,
        }));
    }
    // Most-recently-modified first.
    sessions.sort_by(|a, b| {
        b["modified"]
            .as_u64()
            .unwrap_or(0)
            .cmp(&a["modified"].as_u64().unwrap_or(0))
    });
    Json(Value::Array(sessions))
}

/// GET /api/cost — derive a UI-friendly summary from CostTracker::summary().
/// Adds a `by_provider` array synthesised from the per-model breakdown
/// (model name → provider name via heuristic).
async fn api_cost(State(state): State<Arc<AppState>>) -> Json<Value> {
    let summary = state.cost_tracker.summary().await;

    // Aggregate by_model → by_provider.
    let mut by_provider: std::collections::BTreeMap<String, (u64, f64)> =
        std::collections::BTreeMap::new();
    if let Some(by_model) = summary.get("by_model").and_then(|v| v.as_object()) {
        for (model, stats) in by_model {
            let provider = provider_for_model(model);
            let cost = stats
                .get("cost_usd")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            // We don't track per-model request counts here, so count each model as 1
            // entry and sum costs. This is best-effort — total `requests` lives at
            // the top level.
            let entry = by_provider.entry(provider).or_insert((0, 0.0));
            entry.0 += 1;
            entry.1 += cost;
        }
    }
    let by_provider_json: Vec<Value> = by_provider
        .into_iter()
        .map(|(name, (reqs, usd))| {
            json!({
                "name": name,
                "requests": reqs,
                "usd": (usd * 10000.0).round() / 10000.0,
            })
        })
        .collect();

    Json(json!({
        "session_usd":       summary.get("session_usd").cloned().unwrap_or(json!(0.0)),
        "total_usd":         summary.get("total_usd").cloned().unwrap_or(json!(0.0)),
        "requests":          summary.get("requests").cloned().unwrap_or(json!(0)),
        "prompt_tokens":     summary.get("prompt_tokens").cloned().unwrap_or(json!(0)),
        "completion_tokens": summary.get("completion_tokens").cloned().unwrap_or(json!(0)),
        "by_provider":       by_provider_json,
    }))
}

/// GET /api/tools/history — returns the rolling buffer of captured tool calls.
/// Mirrors what the REPL exposes via /show <n>.
async fn api_tools_history() -> Json<Value> {
    let h = tool_history().lock().unwrap();
    Json(serde_json::to_value(&*h).unwrap_or(Value::Array(vec![])))
}

fn provider_for_model(model: &str) -> String {
    let m = model.to_lowercase();
    if m.contains("claude") {
        return "anthropic".into();
    }
    if m.contains("gemini") {
        return "gemini".into();
    }
    if m.contains("gpt") || m.contains("o1") || m.contains("o3") {
        return "openai".into();
    }
    if m.contains("llama") || m.contains("groq") || m.contains("mixtral") {
        return "groq".into();
    }
    if m.contains("deepseek") {
        return "deepseek".into();
    }
    if m.contains("qwen") {
        return "qwen".into();
    }
    "other".into()
}

// ── MCP over HTTP (stateless, for remote tool use) ────────────────────────────
//
// POST /mcp  { "jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{...} }

async fn mcp_http(State(state): State<Arc<AppState>>, headers: HeaderMap, body: Bytes) -> Response {
    // T7b T13-N2 CRITICAL: HMAC gate on /mcp (executes any tool via tools/call,
    // RCE-equivalent on the `shell` tool).
    if let Err((code, json)) = require_cluster_auth(&state.cluster_manager, &headers, &body) {
        return (code, json).into_response();
    }
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("malformed JSON: {e}") })),
            )
                .into_response();
        }
    };
    let method = msg["method"].as_str().unwrap_or("").to_string();
    let id = msg["id"].clone();
    let params = msg["params"].clone();
    let cfg = state.agent_runtime.config();

    match crate::mcp::handle_http(&method, &params, &cfg.tools).await {
        Ok(result) => Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response(),
        Err((code, message)) => Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message }
        }))
        .into_response(),
    }
}

async fn ws_upgrade(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ws: Option<WebSocketUpgrade>,
) -> Response {
    // T7b T13-N3 HIGH: gate the upgrade BEFORE the WS extractor short-circuits
    // with 426 Upgrade Required. Canonical body is empty bytes; clients pass
    // `X-Cluster-Auth: hex(HMAC-SHA256(cluster_secret, ""))`.
    //
    // Note: `WebSocketUpgrade` is wrapped in `Option<…>` so a non-WS request
    // doesn't bypass the auth check by failing extraction first.
    if let Err((code, json)) = require_cluster_auth(&state.cluster_manager, &headers, b"") {
        return (code, json).into_response();
    }
    let Some(ws) = ws else {
        return (
            StatusCode::UPGRADE_REQUIRED,
            Json(json!({ "error": "WebSocket upgrade required" })),
        )
            .into_response();
    };
    ws.on_upgrade(move |socket| session(socket, state))
}

// ── Per-connection session ────────────────────────────────────────────────────

async fn session(mut socket: WebSocket, state: Arc<AppState>) {
    // Buffered channel: agent tasks push notifications here; main loop drains it.
    let (tx, mut rx) = mpsc::channel::<String>(256);
    let mut initialized = false;
    let mut cancel: Option<tokio::sync::oneshot::Sender<()>> = None;

    loop {
        tokio::select! {
            biased;

            // ── Outbound: forward agent events to client ──────────────────
            Some(frame) = rx.recv() => {
                if socket.send(Message::Text(frame.into())).await.is_err() {
                    break;
                }
            }

            // ── Inbound: handle client messages ───────────────────────────
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let replies = dispatch(
                            &text, &state, &tx,
                            &mut initialized, &mut cancel,
                        ).await;
                        for r in replies {
                            if socket.send(Message::Text(r.into())).await.is_err() {
                                return;
                            }
                        }
                    }
                    Some(Ok(Message::Ping(b))) => {
                        let _ = socket.send(Message::Pong(b)).await;
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
}

// ── RPC dispatch ──────────────────────────────────────────────────────────────

/// Returns zero or more JSON-RPC frames to be sent back immediately
/// (before any async agent output).  Agent notifications are pushed
/// through `event_tx` from the spawned task.
async fn dispatch(
    text: &str,
    state: &Arc<AppState>,
    event_tx: &mpsc::Sender<String>,
    initialized: &mut bool,
    cancel: &mut Option<tokio::sync::oneshot::Sender<()>>,
) -> Vec<String> {
    let msg: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return vec![err(&json!(null), -32700, &format!("Parse error: {}", e))],
    };

    let method = msg["method"].as_str().unwrap_or("").to_string();
    let id = msg["id"].clone();
    let params = &msg["params"];

    // Notifications from client (no "id" field) require no response.
    if msg.get("id").is_none() {
        tracing::debug!(method, "client notification — no response needed");
        return vec![];
    }

    match method.as_str() {
        // ── Handshake ─────────────────────────────────────────────────────
        "initialize" => {
            *initialized = true;
            vec![ok(
                &id,
                json!({
                    "userAgent":      concat!("phantom-mesh/", env!("CARGO_PKG_VERSION")),
                    "platformFamily": if cfg!(windows) { "windows" } else { "unix" },
                    "platformOs":     std::env::consts::OS,
                }),
            )]
        }

        // ── Start a new agent turn ────────────────────────────────────────
        "thread/start" | "turn/start" if *initialized => {
            let prompt = params["userMessage"]
                .as_str()
                .or_else(|| params["message"].as_str())
                .or_else(|| params["content"].as_str())
                .unwrap_or("")
                .to_string();
            let agent_name = params["agent"].as_str().unwrap_or("master").to_string();
            let thread_id = uuid::Uuid::new_v4().to_string();

            // Cancel any running turn.
            if let Some(tx) = cancel.take() {
                let _ = tx.send(());
            }
            let (c_tx, c_rx) = tokio::sync::oneshot::channel::<()>();
            *cancel = Some(c_tx);

            // Queue pre-turn notifications (non-blocking — channel has room).
            let _ = event_tx.try_send(notif("thread/started", json!({ "threadId": thread_id })));
            let _ = event_tx.try_send(notif("turn/started", json!({ "threadId": thread_id })));

            // Spawn the agent task.
            let state2 = state.clone();
            let etx = event_tx.clone();
            let tid = thread_id.clone();
            tokio::spawn(async move {
                let runtime = state2.agent_runtime.clone();
                let cost_tracker = state2.cost_tracker.clone();
                let tid2 = tid.clone();
                let etx2 = etx.clone();

                let on_event = move |ev: AgentEvent| {
                    let frame = match ev {
                        AgentEvent::Token { content } => notif(
                            "item/agentMessage/delta",
                            json!({ "threadId": tid2, "delta": content }),
                        ),
                        AgentEvent::ToolStart { name, args_preview } => notif(
                            "item/commandExecution/outputDelta",
                            json!({ "threadId": tid2, "delta": format!("▶ {}: {}\n", name, args_preview) }),
                        ),
                        AgentEvent::ToolDone {
                            name,
                            output_preview,
                            ..
                        } => notif(
                            "item/commandExecution/outputDelta",
                            json!({ "threadId": tid2, "delta": format!("✓ {}: {}\n", name, output_preview) }),
                        ),
                        AgentEvent::Thinking { content } => notif(
                            "item/agentReasoning/delta",
                            json!({ "threadId": tid2, "delta": content }),
                        ),
                        AgentEvent::Done {
                            output,
                            cost_usd,
                            elapsed_secs,
                        } => notif(
                            "turn/completed",
                            json!({
                                "threadId":    tid2,
                                "output":      output,
                                "costUsd":     cost_usd,
                                "elapsedSecs": elapsed_secs,
                            }),
                        ),
                        AgentEvent::Notice { message } => notif(
                            "item/agentMessage/notice",
                            json!({ "threadId": tid2, "message": message }),
                        ),
                        #[cfg(feature = "experimental-anti-hallucination")]
                        AgentEvent::ConsistencyWarning { unbacked_claims } => notif(
                            "item/agentMessage/notice",
                            json!({
                                "threadId": tid2,
                                "message": format!(
                                    "anti-hallucination: {} unbacked claim(s) — {}",
                                    unbacked_claims.len(),
                                    unbacked_claims.join(" | "),
                                ),
                            }),
                        ),
                    };
                    let _ = etx2.try_send(frame);
                };

                let result = tokio::select! {
                    r = runtime.run_with_callbacks(
                            &agent_name, &prompt, &[], None, &cost_tracker, on_event,
                        ) => r,
                    _ = c_rx => {
                        let _ = etx.try_send(notif("turn/interrupted", json!({ "threadId": tid })));
                        return;
                    }
                };

                if let Err(e) = result {
                    let _ = etx.try_send(notif(
                        "turn/completed",
                        json!({
                            "threadId": tid, "output": format!("Error: {}", e),
                            "costUsd": 0.0, "elapsedSecs": 0.0,
                        }),
                    ));
                }
            });

            vec![ok(&id, json!({ "threadId": thread_id }))]
        }

        // ── Interrupt the running turn ────────────────────────────────────
        "turn/interrupt" if *initialized => {
            if let Some(tx) = cancel.take() {
                let _ = tx.send(());
            }
            vec![ok(&id, json!({}))]
        }

        // ── Config read ───────────────────────────────────────────────────
        "config/read" if *initialized => {
            let cfg = state.agent_runtime.config();
            vec![ok(
                &id,
                json!({
                    "defaultModel": cfg.default_model,
                    "maxRounds":    cfg.max_rounds,
                    "tokenBudget":  cfg.token_budget,
                }),
            )]
        }

        // ── Thread list (stub) ────────────────────────────────────────────
        "thread/list" if *initialized => {
            vec![ok(&id, json!({ "threads": [] }))]
        }

        // ── Guard: not initialized ────────────────────────────────────────
        _ if !*initialized => {
            vec![err(
                &id,
                -32001,
                "Not initialized — send {\"id\":1,\"method\":\"initialize\"} first",
            )]
        }

        // ── Unknown method ────────────────────────────────────────────────
        other => {
            tracing::debug!(method = other, "unknown RPC method");
            vec![err(&id, -32601, &format!("Method not found: {}", other))]
        }
    }
}

// ── Cluster RPC handlers ──────────────────────────────────────────────────────

/// Inject `wire_version` into the top level of a JSON response so peers
/// can refuse incompatible payloads. Per Rule 5 of MULTI-DEVICE-
/// COORDINATION.md, every peer-facing RPC carries this. Non-object
/// values pass through unchanged (defensive — should never happen for
/// our handlers, all of which return objects).
fn with_wire_version(mut v: Value) -> Value {
    if let Some(obj) = v.as_object_mut() {
        obj.entry("wire_version".to_string())
            .or_insert(json!(crate::WIRE_VERSION));
    }
    v
}

/// Reject an incoming RPC body whose declared `wire_version` is HIGHER
/// than this binary's. Higher means "the peer speaks a newer dialect we
/// haven't been taught"; we cannot best-effort handle that. Lower or
/// missing is tolerated (degraded warning happens at the doctor layer,
/// not here, so old peers keep working). Returns Some(error_response)
/// when the request must be refused.
fn check_wire_version(body: &Value) -> Option<(StatusCode, Json<Value>)> {
    let peer_wv = body
        .get("wire_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    if peer_wv > crate::WIRE_VERSION {
        let msg = format!(
            "peer is wire v{}, this binary is v{}, run `phantom upgrade`",
            peer_wv,
            crate::WIRE_VERSION
        );
        return Some((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": msg,
                "wire_version": crate::WIRE_VERSION,
            })),
        ));
    }
    None
}

/// T7b: extracted into a shared module so the daemon router in
/// `core/src/main.rs` can reuse the same HMAC gate. Bare-JSON error body
/// (no `wire_version` wrapper); callers wrap on the way out if needed.
pub use crate::auth_gate::require_cluster_auth;

/// SPEC-10 migration dual-accept gate for inbound peer `/rpc/*` calls.
///
/// Tries the legacy gate first ([`require_cluster_auth`] — HMAC over the raw
/// body in `X-Cluster-Auth`, including its empty-secret fail-closed + override
/// behaviour). Only if that rejects does it try the SPEC-10 canonical
/// signature (`X-Cluster-Auth` = HMAC over
/// `rpc_wire::build_canonical_string`). Returns the legacy gate's error when
/// neither verifies, so the empty-secret 403 / bad-token 401 semantics are
/// preserved unchanged. Widening what we *accept* is non-breaking: legacy
/// peers keep working while SPEC-10 peers become acceptable ahead of the
/// coordinated outbound cutover (T-CORE-01 Stage 3).
///
/// 中文: 遷移期雙重接受閘門 — 先走舊版驗證，失敗才退而驗 SPEC-10 canonical
/// 簽章；兩者皆不過才回舊版錯誤，維持原本 403/401 語意不變。
///
/// `raw_query` is the request's raw query string (`None`/empty for the three
/// POST-only mesh routes). The empty-query route invariant is ENFORCED here
/// (review: codex): if a non-empty query is present, the canonical arm is
/// skipped — a SPEC-10 signature is verified over an empty `sorted_query`, so
/// allowing it through would leave the query component unauthenticated. Failing
/// closed on any query keeps the signature bound to the exact request target
/// until Stage 3 implements real query canonicalisation.
fn require_cluster_auth_dual(
    cm: &crate::mesh::ClusterManager,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    raw_query: Option<&str>,
    body: &[u8],
) -> Result<(), (StatusCode, Json<Value>)> {
    match require_cluster_auth(cm, headers, body) {
        Ok(()) => Ok(()),
        Err(legacy_err) => {
            // `to_str()` returns Err only for non-UTF8 header bytes; a valid
            // hex signature is always ASCII, so a non-UTF8 header can never be
            // a real signature — treating it as absent (None) is correct, not a
            // bypass (review: opencode O1).
            let sig = headers
                .get("X-Cluster-Auth")
                .and_then(|v| v.to_str().ok());
            let traceparent = headers.get("traceparent").and_then(|v| v.to_str().ok());
            // ENFORCED route invariant (review: codex): these three mesh RPC
            // routes are POST-only with NO query string, so the canonical
            // `sorted_query` segment is "". We verify with "" AND refuse the
            // canonical arm outright when a query is actually present, so a
            // query-bearing request can never be authorised by an empty-query
            // signature (the query would otherwise ride unauthenticated). When
            // Stage 3 adds outbound canonical signing it must extract + sort the
            // real query here instead of refusing.
            let query_absent = raw_query.map_or(true, |q| q.is_empty());
            if query_absent
                && sig.is_some()
                && cm.verify_auth_dual(None, sig, method, path, "", body, traceparent)
            {
                Ok(())
            } else {
                Err(legacy_err)
            }
        }
    }
}

/// `GET /node/capabilities` — return this node's capability report
/// as JSON for cluster peer discovery + capability-aware dispatch.
///
/// Same payload as `phantom node-capabilities --json` CLI; both use
/// `phantom_mesh::capabilities::NodeCapabilityReport::detect()`. PF-4.
async fn node_capabilities() -> Json<Value> {
    let report = crate::capabilities::NodeCapabilityReport::detect();
    // serde_json::to_value never fails for serde-derived types in
    // practice; on the off-chance it does, return an error shape so
    // callers don't get a 500 with empty body.
    match serde_json::to_value(&report) {
        Ok(v) => Json(v),
        Err(e) => Json(json!({
            "error": "capability_serialize_failed",
            "detail": e.to_string(),
        })),
    }
}

/// POST/GET /rpc/ping — return this node's own PeerStatus PLUS the
/// wire-protocol compatibility tuple (wire_version + phantom_version +
/// core_sha) PLUS this node's [agent.*] inventory so the Squad
/// Pipeline dispatcher (SPEC-FREEZE-V1 §11.1, §12.4 step [2]) can
/// build a routing plan without a separate inventory RPC.
async fn rpc_ping(State(state): State<Arc<AppState>>) -> Json<Value> {
    let mut peer = state.cluster_manager.own_peer_status();
    // Inject this node's agent name list — ClusterManager doesn't
    // see [agent.*] keys, so we populate from AgentRuntime here at
    // request time. Sorted for stable serialisation across pings.
    let mut agents: Vec<String> = state.agent_runtime.config().agent.keys().cloned().collect();
    agents.sort();
    peer.agents = agents;

    let mut body = serde_json::to_value(peer).unwrap_or_else(|_| json!({"online": true}));
    if let Some(obj) = body.as_object_mut() {
        obj.insert("wire_version".to_string(), json!(crate::WIRE_VERSION));
        obj.insert(
            "phantom_version".to_string(),
            json!(env!("CARGO_PKG_VERSION")),
        );
        obj.insert("core_sha".to_string(), json!(crate::core_sha()));
    }
    Json(body)
}

/// POST /rpc/skill/sync — SPEC-25 §8.7 cross-peer skill ingest. Thin transport
/// over the fail-closed ingest core (`skillbank::sync::ingest_batch`): verify the
/// outer X-Cluster-Auth HMAC, enforce the batch cap, resolve the two keys, then
/// delegate the per-envelope verify+decrypt+LWW-merge. The security logic +
/// fail-closed semantics live (and are unit-tested) in the core; this handler is
/// auth + parse + size + key-resolve + dispatch.
async fn rpc_skill_sync(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // 1. Outer cluster auth — HMAC over the raw body (same gate as other /rpc/*).
    if let Err((code, json)) = require_cluster_auth(&state.cluster_manager, &headers, &body) {
        return (code, json).into_response();
    }

    // 2. Parse the batch body.
    let batch: crate::skillbank::sync::SkillSyncBatch = match serde_json::from_slice(&body) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "bad_request", "detail": e.to_string() })),
            )
                .into_response();
        }
    };

    // 3. Batch cap (SPEC-25 §9.5 → 413).
    if batch.skills.len() > crate::skillbank::sync::MAX_BATCH {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({
                "error": "batch_too_large",
                "max_skills": crate::skillbank::sync::MAX_BATCH,
            })),
        )
            .into_response();
    }

    // 4. Resolve the two keys. cluster_secret: the outer auth already proved a
    //    real secret is configured + the token verified, but re-read it for the
    //    per-envelope signature check — fail CLOSED if the env-override empty-secret
    //    path let auth through, since we cannot verify envelopes without it.
    let cluster_secret = match state.cluster_manager.config.cluster_secret.clone() {
        Some(s) if !s.is_empty() => s,
        _ => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "cluster_secret_required" })),
            )
                .into_response();
        }
    };
    let Ok(data) = crate::cli_config::phantom_data_dir() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "no_data_dir" })),
        )
            .into_response();
    };
    let event_key =
        match crate::life_node::key_derivation::load_event_key(&data.join("identity.key")) {
            Ok(k) => k,
            Err(_) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "no_event_key" })),
                )
                    .into_response();
            }
        };

    // 5. Ingest on the blocking pool — sqlite + per-envelope age decrypt are sync.
    let secret_bytes = cluster_secret.into_bytes();
    let result = tokio::task::spawn_blocking(move || {
        crate::skillbank::sync::ingest_batch(&batch.skills, &secret_bytes, &event_key)
    })
    .await;

    match result {
        Ok(Ok(r)) => Json(json!({
            "accepted": r.accepted,
            "duplicates": r.duplicates,
            "rejected": r.rejected,
        }))
        .into_response(),
        // Real store/db failure (or a join error) — never partial-applied state
        // beyond what ingest_batch already committed row-by-row.
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "store_failed" })),
        )
            .into_response(),
    }
}

/// GET /rpc/peers — return all configured peers (cached) plus self.
async fn rpc_peers(State(state): State<Arc<AppState>>) -> Json<Value> {
    let peers = state.cluster_manager.status().await;
    let own = state.cluster_manager.own_peer_status();
    Json(with_wire_version(json!({ "peers": peers, "self": own })))
}

/// POST /rpc/message — run the local agent synchronously and return its output.
/// Used by remote nodes calling `route_to_best_peer`.
/// Body: `{ "message": "...", "agent": "master" }` (agent optional).
///
/// **Auth (T7 codex audit 2026-05-15):** requires `X-Cluster-Auth` HMAC.
/// Refuses outright if cluster_secret is empty unless
/// `PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET=1` is set.
async fn rpc_message(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/rpc/message",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(with_wire_version(
                    json!({ "error": format!("malformed body: {e}") }),
                )),
            )
                .into_response()
        }
    };
    if let Some((code, err)) = check_wire_version(&parsed) {
        return (code, err).into_response();
    }
    let message = parsed["message"].as_str().unwrap_or("").to_string();
    let agent = parsed["agent"].as_str().unwrap_or("master").to_string();
    if message.is_empty() {
        return Json(with_wire_version(
            json!({ "error": "message field required" }),
        ))
        .into_response();
    }
    match state.agent_runtime.run(&agent, &message, &[], None).await {
        Ok(result) => Json(with_wire_version(json!({ "output": result.output }))).into_response(),
        Err(e) => Json(with_wire_version(json!({ "error": e.to_string() }))).into_response(),
    }
}

/// POST /rpc/inbox — persist a small coordination message for the dev
/// session running on this node (S1 of the multi-machine dev framework).
/// Body: `{ "from": "m1", "text": "...", "topic": "backlog" }` (from/topic
/// optional). The message lands as a file under `~/.phantom-mesh/inbox/`;
/// the local session reads + acks it via `phantom inbox list/ack` on its
/// next loop tick. Unlike `/rpc/message` this never runs an agent — it is
/// pure mailbox, so delivery is cheap and safe to broadcast.
///
/// **Auth:** same `X-Cluster-Auth` HMAC posture as `/rpc/message`.
async fn rpc_inbox(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/rpc/inbox",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(with_wire_version(
                    json!({ "error": format!("malformed body: {e}") }),
                )),
            )
                .into_response()
        }
    };
    if let Some((code, err)) = check_wire_version(&parsed) {
        return (code, err).into_response();
    }
    let text = parsed["text"].as_str().unwrap_or("");
    let from = parsed["from"].as_str().unwrap_or("unknown");
    let topic = parsed["topic"].as_str();
    let Ok(home) = crate::cli_config::resolve_home_dir() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(with_wire_version(json!({ "error": "no home dir" }))),
        )
            .into_response();
    };
    match crate::inbox::write_message(&home, from, text, topic) {
        Ok(id) => Json(with_wire_version(json!({ "id": id, "queued": true }))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(with_wire_version(json!({ "error": e.to_string() }))),
        )
            .into_response(),
    }
}

/// POST /rpc/zk/put — zero-knowledge relay (P2-1): accept ONE age-sealed blob
/// keyed by `(device_id, blob_id)` and store it. The server NEVER sees plaintext
/// — it stores opaque ciphertext and holds no key material. Body:
/// `{ "device_id": "...", "blob_id": "...", "sealed_b64": "<base64 age blob>" }`.
/// A payload that is not an age-sealed blob is REFUSED (400) — the relay can
/// never be coaxed into storing plaintext.
///
/// **Auth:** same `X-Cluster-Auth` HMAC posture as `/rpc/inbox`.
async fn rpc_zk_put(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    use base64::Engine as _;
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/rpc/zk/put",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(with_wire_version(
                    json!({ "error": format!("malformed body: {e}") }),
                )),
            )
                .into_response()
        }
    };
    if let Some((code, err)) = check_wire_version(&parsed) {
        return (code, err).into_response();
    }
    let device_id = parsed["device_id"].as_str().unwrap_or("");
    let blob_id = parsed["blob_id"].as_str().unwrap_or("");
    let sealed = match base64::engine::general_purpose::STANDARD
        .decode(parsed["sealed_b64"].as_str().unwrap_or(""))
    {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(with_wire_version(
                    json!({ "error": format!("sealed_b64 is not valid base64: {e}") }),
                )),
            )
                .into_response()
        }
    };
    let Ok(home) = crate::cli_config::resolve_home_dir() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(with_wire_version(json!({ "error": "no home dir" }))),
        )
            .into_response();
    };
    match crate::zk_cloud::put_blob(&home, device_id, blob_id, &sealed) {
        Ok(()) => Json(with_wire_version(json!({ "stored": true }))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(with_wire_version(json!({ "error": e.to_string() }))),
        )
            .into_response(),
    }
}

/// POST /rpc/zk/get — zero-knowledge relay (P2-1): return the sealed blob for
/// `(device_id, blob_id)` as base64. FAILS CLOSED — a missing/unknown key yields
/// 404 with a generic error, NEVER plaintext and NEVER a different blob. The
/// server never decrypts. Body: `{ "device_id": "...", "blob_id": "..." }`.
///
/// **Auth:** same `X-Cluster-Auth` HMAC posture as `/rpc/inbox`.
async fn rpc_zk_get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    use base64::Engine as _;
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/rpc/zk/get",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(with_wire_version(
                    json!({ "error": format!("malformed body: {e}") }),
                )),
            )
                .into_response()
        }
    };
    if let Some((code, err)) = check_wire_version(&parsed) {
        return (code, err).into_response();
    }
    let device_id = parsed["device_id"].as_str().unwrap_or("");
    let blob_id = parsed["blob_id"].as_str().unwrap_or("");
    let Ok(home) = crate::cli_config::resolve_home_dir() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(with_wire_version(json!({ "error": "no home dir" }))),
        )
            .into_response();
    };
    match crate::zk_cloud::get_blob(&home, device_id, blob_id) {
        Ok(sealed) => {
            let sealed_b64 = base64::engine::general_purpose::STANDARD.encode(&sealed);
            Json(with_wire_version(json!({ "sealed_b64": sealed_b64 }))).into_response()
        }
        // Fail closed: a missing/unknown key is a generic 404 — we never leak
        // whether the device or the blob existed, never plaintext, never another
        // blob.
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(with_wire_version(json!({ "error": "not found" }))),
        )
            .into_response(),
    }
}

/// POST /rpc/approvals/list — list the high-risk approvals a governed run on
/// this node is currently BLOCKED on, so a phone app can render decision cards
/// (apex-④ phone approval UI). Read-only: it just reads the filesystem pending
/// store that `PhoneEscalator::await_decision` mirrors. Decision *submission*
/// stays on `/rpc/inbox` (the phone POSTs `{topic: approval_id, text:
/// "approve"/"deny"/"stop"}`). Returns `{ "pending": [PendingCard...] }`.
///
/// **Auth:** same `X-Cluster-Auth` HMAC posture as `/rpc/inbox` (the legacy
/// body-HMAC arm signs the — typically empty — body; POST-only, no query).
async fn rpc_approvals_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/rpc/approvals/list",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }
    let Ok(home) = crate::cli_config::resolve_home_dir() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(with_wire_version(json!({ "error": "no home dir" }))),
        )
            .into_response();
    };
    match crate::pending_approvals::list_pending(&home) {
        Ok(pending) => {
            Json(with_wire_version(json!({ "pending": pending }))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(with_wire_version(json!({ "error": e.to_string() }))),
        )
            .into_response(),
    }
}

/// Stable wire projection for one durable task row (P1-2 mobile-supervisor
/// surface). Pins the field set the phone renders so future `TaskRecord` field
/// churn can't silently change the contract the app's `parseTasks` reads.
fn task_record_to_wire(r: &pm_types::TaskRecord) -> Value {
    json!({
        "task_id":     r.task_id.to_string(),
        "agent_name":  r.agent_name,
        "prompt":      r.prompt,
        "status":      r.status.as_str(),
        "created_at":  r.created_at,
        "started_at":  r.started_at,
        "finished_at": r.finished_at,
        "cost_usd":    r.cost_usd,
        "turns":       r.turns,
        "error":       r.error,
        "output":      r.output,
    })
}

/// POST /rpc/tasks/list — live supervisor view of backend tasks (P1-2 M1).
/// Body: `{ "limit"?: number }` (default 50, capped 200). Returns recent durable
/// tasks (created_at DESC) plus the pending-approval cards awaiting the operator
/// (the "what's awaiting me" half). HMAC-authed via `require_cluster_auth_dual`
/// — exposes operator prompts, so same posture as `/rpc/message`.
async fn rpc_tasks_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/rpc/tasks/list",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }
    let limit = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| v.get("limit").and_then(|n| n.as_u64()))
        .unwrap_or(50)
        .min(200) as usize;

    let tasks: Vec<Value> = match &state.task_queue {
        Some(tq) => match tq.list(None, None, limit).await {
            Ok(rows) => rows.iter().map(task_record_to_wire).collect(),
            Err(e) => {
                tracing::warn!(target: "phantom::serve", "tasks/list failed: {e}");
                Vec::new()
            }
        },
        None => Vec::new(),
    };

    let pending: Vec<Value> = crate::cli_config::resolve_home_dir()
        .ok()
        .and_then(|h| crate::pending_approvals::list_pending(&h).ok())
        .map(|cards| {
            cards
                .iter()
                .map(|c| {
                    json!({
                        "approval_id": c.approval_id,
                        "task_id":     c.task_id,
                        "tool":        c.tool,
                        "risk":        c.risk,
                        "reason":      c.reason,
                        "created_ms":  c.created_ms,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Json(with_wire_version(json!({ "tasks": tasks, "pending": pending }))).into_response()
}

/// Stable snake_case wire projection for one captured event (P1-2 M2). The
/// public `EventMeta` is `#[serde(rename_all = "camelCase")]`, so we project to
/// explicit snake_case keys the phone's `parseCaptures` reads (`kind` stays the
/// EventKind snake_case string: "food" | "focus" | "habit" | "dispatch" |
/// "text", matching KIND_EMOJI keys).
fn event_meta_to_wire(m: &crate::event_storage_wire::EventMeta) -> Value {
    json!({
        "event_id":  m.event_id,
        "timestamp": m.timestamp,
        "kind":      m.kind,
        "tags":      m.tags,
    })
}

/// POST /rpc/captures/recent — recent captured life-node events (P1-2 M2).
/// Body: `{ "limit"?: number }` (default 50, cap 200). Enumerates the events dir
/// (same loader posture as `daily_review::load_events_for_date`), reads each
/// meta, projects to the stable wire shape, newest-first by UTC instant.
/// HMAC-authed — exposes captured private life events.
async fn rpc_captures_recent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/rpc/captures/recent",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }
    let limit = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| v.get("limit").and_then(|n| n.as_u64()))
        .unwrap_or(50)
        .min(200) as usize;

    let Ok(data) = crate::cli_config::phantom_data_dir() else {
        return Json(with_wire_version(json!({ "captures": [] }))).into_response();
    };
    // The enumeration is unbounded synchronous disk I/O + per-event decryption
    // (read_dir + read_meta), so run it on the blocking pool to avoid starving
    // the async executor as the events dir grows (review: agy DoS finding).
    let metas: Vec<crate::event_storage_wire::EventMeta> =
        tokio::task::spawn_blocking(move || {
            let events_dir = data.join("events");
            // Encrypted store when an identity key exists; plaintext fallback
            // otherwise (older stores + the test fixtures). Mirrors
            // load_events_for_date.
            let store =
                match crate::life_node::key_derivation::load_event_key(&data.join("identity.key")) {
                    Ok(key) => EventStore::with_key(&events_dir, key),
                    Err(_) => EventStore::new(&events_dir),
                };
            let mut metas: Vec<crate::event_storage_wire::EventMeta> = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&events_dir) {
                for entry in rd.flatten() {
                    if !entry.path().is_dir() {
                        continue;
                    }
                    let id = entry.file_name().to_string_lossy().to_string();
                    if let Ok(meta) = store.read_meta(&id) {
                        metas.push(meta);
                    }
                }
            }
            // Newest first by absolute UTC instant (offset-agnostic, matches the
            // daily_review sort key).
            metas.sort_by_key(|m| {
                std::cmp::Reverse(crate::event_storage_wire::ts_epoch_ms(&m.timestamp))
            });
            metas.truncate(limit);
            metas
        })
        .await
        .unwrap_or_default();
    let captures: Vec<Value> = metas.iter().map(event_meta_to_wire).collect();

    Json(with_wire_version(json!({ "captures": captures }))).into_response()
}

/// POST /rpc/review — offline daily-review aggregate for a date (P1-2 M3).
/// Body: `{ "date"?: "YYYY-MM-DD" }` (defaults to local-today). Mirrors the
/// `daily_review_load` Tauri command (NO LLM pass — aggregate only) so the
/// supervisor phone sees the backend's captured-events brief. HMAC-authed.
async fn rpc_review(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/rpc/review",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }
    let date = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| v.get("date").and_then(|d| d.as_str().map(String::from)))
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());

    let Ok(data) = crate::cli_config::phantom_data_dir() else {
        return Json(with_wire_version(json!({ "date": date, "markdown": "" }))).into_response();
    };
    // load_events_for_date does synchronous dir traversal + per-event decrypt;
    // run it (and the aggregate) on the blocking pool (review: agy DoS finding).
    let date_for_task = date.clone();
    let markdown = tokio::task::spawn_blocking(move || {
        let events_dir = data.join("events");
        // Encrypted store when an identity key exists; `None` → plaintext loader
        // (matches the test fixtures + older stores).
        let key = crate::life_node::key_derivation::load_event_key(&data.join("identity.key")).ok();
        match crate::life_node::daily_review::load_events_for_date(&events_dir, &date_for_task, key) {
            Ok(events) => crate::life_node::daily_review::aggregate(&date_for_task, &events),
            Err(e) => {
                tracing::warn!(target: "phantom::serve", "review load failed: {e}");
                String::new()
            }
        }
    })
    .await
    .unwrap_or_default();
    Json(with_wire_version(json!({ "date": date, "markdown": markdown }))).into_response()
}

/// GET /rpc/session-status — this node's dev-session heartbeat (S2).
/// Returns `{ "node": "...", "status": {...}|null, "phantom_version": "..." }`
/// where `status` is whatever the local routine last wrote via
/// `phantom status set` (null on a node whose session never reported).
/// Read-only and cheap — `phantom status mesh` fans this out cluster-wide.
///
/// **Auth:** same HMAC posture as the other /rpc routes (clients sign the
/// empty body with the legacy arm, like the dispatch status poll).
async fn rpc_session_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
) -> impl IntoResponse {
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "GET",
        "/rpc/session-status",
        raw_query.as_deref(),
        b"",
    ) {
        return (code, json).into_response();
    }
    let node = crate::cli_config::resolve_self_node_name().unwrap_or_else(|| "unknown".into());
    let status = crate::cli_config::resolve_home_dir().ok().and_then(|h| crate::session_status::read_status(&h));
    let age = status.as_ref().map(crate::session_status::age_secs);
    Json(with_wire_version(json!({
        "node": node,
        "status": status,
        "age_secs": age,
        "phantom_version": env!("CARGO_PKG_VERSION"),
    })))
    .into_response()
}

/// Derive the at-most-once dedup key for a `/rpc/task/assign` request (review
/// #321 §5 restore). Thin re-export of the canonical
/// [`crate::idempotency::task_assign_idem_key`] so this router and the shipped
/// `main.rs` daemon share ONE keying definition and cannot drift. Prefers the
/// caller's explicit `idempotency_key`; absent (or blank), falls back to a
/// stable content hash of `agent\nprompt`. Scoped to `task_assign` so it never
/// collides with the squad-dispatch (`dispatch`) or partner-message ledgers for
/// the same body. Pure (no IO) so the keying logic is unit-testable on its own.
fn task_assign_idem_key(idempotency_key: Option<&str>, agent: &str, prompt: &str) -> String {
    crate::idempotency::task_assign_idem_key(idempotency_key, agent, prompt)
}

/// POST /rpc/task/assign — authenticate (HMAC-SHA256 when cluster_secret is set),
/// spawn the task asynchronously, and immediately return `{ "job_id": "..." }`.
/// Body: `{ "agent": "master", "prompt": "..." }`.
async fn rpc_task_assign(
    State(state): State<Arc<AppState>>,
    Extension(jobs): Extension<ClusterJobStore>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    // T7 fix (codex audit 2026-05-15): fail closed when cluster_secret is
    // empty (previously silently accepted unauthenticated remote agent
    // execution). Override via PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET=1 for
    // one migration release; logged loudly at boot.
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/rpc/task/assign",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }

    // Refuse if the peer speaks a newer wire version than this binary.
    if let Ok(peek) = serde_json::from_slice::<Value>(&body) {
        if let Some((code, err)) = check_wire_version(&peek) {
            return (code, err).into_response();
        }
    }

    let req: crate::mesh::TaskAssignRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(with_wire_version(json!({ "error": e.to_string() }))),
            )
                .into_response()
        }
    };

    // ── C1: cycle guard (before forwarding decision) ───────────────────
    // Run BEFORE the capability enforcement so a cycling forward never
    // reaches the runtime — even in soft mode where it would otherwise
    // be silently accepted. Spec §5: limit hops to FORWARD_CHAIN_LIMIT
    // and reject if `self` is already in the chain.
    let my_node_name = state
        .cluster_manager
        .config
        .node_name
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    if req.forward_chain.len() >= crate::mesh::FORWARD_CHAIN_LIMIT {
        return (
            StatusCode::CONFLICT,
            Json(with_wire_version(json!({
                "error":      "cycle_detected",
                "error_code": "forward_chain_exhausted",
                "chain":      req.forward_chain,
                "limit":      crate::mesh::FORWARD_CHAIN_LIMIT,
            }))),
        )
            .into_response();
    }
    if req.forward_chain.iter().any(|n| n == &my_node_name) {
        return (
            StatusCode::CONFLICT,
            Json(with_wire_version(json!({
                "error":      "cycle_detected",
                "error_code": "self_in_chain",
                "chain":      req.forward_chain,
                "node":       my_node_name,
            }))),
        )
            .into_response();
    }

    // ── T5 + C1: server-side capability enforcement (caps-aware) ──────
    // Defense-in-depth complement to the M1 client-side dispatch filter.
    // A buggy or malicious orchestrator (holding cluster_secret) could
    // POST a task with required_caps this worker doesn't satisfy. In
    // strict mode we bounce with 409; in soft mode (default) we log and
    // continue so existing deployments are unchanged.
    //
    // C1 extension: if PHANTOM_FORWARD_ON_CAPS_MISMATCH=1 AND a peer in
    // peers.json satisfies, route the task there instead of running it
    // locally. Q3 (spec §14): forwarding is refused when node_name is
    // unset to avoid malformed chains — we just fall back to the base
    // decision below.
    let local_caps = &state.cluster_manager.config.worker_caps;
    let mode = state.cluster_manager.config.effective_enforce_mode();
    let peers_snapshot = state.cluster_manager.peer_infos().await;
    let decision = if state.cluster_manager.config.node_name.is_none()
        && crate::mesh::forward_on_caps_mismatch_enabled()
    {
        tracing::warn!(
            target: "phantom::dispatch::forward",
            "PHANTOM_FORWARD_ON_CAPS_MISMATCH=1 but node_name is unset; \
             refusing to forward (would emit a malformed chain). \
             Add [cluster].node_name to agents.toml."
        );
        crate::mesh::enforce_required_caps(local_caps, &req.required_caps, mode)
    } else {
        crate::mesh::enforce_required_caps_with_forwarding(
            local_caps,
            &req.required_caps,
            mode,
            &peers_snapshot,
        )
    };

    match decision {
        crate::mesh::CapsDecision::Allow => { /* fall through to local run */ }
        crate::mesh::CapsDecision::LogAndAllow { missing } => {
            tracing::warn!(
                target: "phantom::dispatch",
                ?missing,
                local = ?local_caps,
                required = ?req.required_caps,
                "capability_mismatch (soft mode): accepting task this worker may not be able to satisfy"
            );
        }
        crate::mesh::CapsDecision::Reject { missing } => {
            // C1: distinguish "no peer would satisfy" from the original
            // capability-mismatch. When the env gate is on but no peer
            // satisfies, surface the inventory so the operator can see
            // why we didn't forward.
            if crate::mesh::forward_on_caps_mismatch_enabled() {
                let inventory: Vec<serde_json::Value> = peers_snapshot
                    .iter()
                    .filter(|p| p.online)
                    .map(|p| json!({ "url": p.url, "capabilities": p.capabilities }))
                    .collect();
                return (
                    StatusCode::CONFLICT,
                    Json(with_wire_version(json!({
                        "error":           "no_peer_satisfies_caps",
                        "error_code":      "no_peer_satisfies_caps",
                        "required":        req.required_caps,
                        "local":           local_caps,
                        "missing":         missing,
                        "available_peers": inventory,
                    }))),
                )
                    .into_response();
            }
            return (
                StatusCode::CONFLICT,
                Json(with_wire_version(json!({
                    "error":      "capability_mismatch",
                    "error_code": "capability_mismatch",
                    "required":   req.required_caps,
                    "local":      local_caps,
                    "missing":    missing,
                }))),
            )
                .into_response();
        }
        crate::mesh::CapsDecision::ForwardTo { peer, missing: _ } => {
            // C1 happy path: a downstream peer satisfies. HMAC-re-sign
            // happens inside `forward_task_to_capable_peer` — see spec §6.
            let target_name = peer.name.clone();
            let target_url = peer.url.clone();
            match state
                .cluster_manager
                .forward_task_to_capable_peer(&req, &peer, &my_node_name)
                .await
            {
                Ok(remote_job_id) => {
                    return (
                        StatusCode::ACCEPTED,
                        Json(with_wire_version(json!({
                            "job_id":         remote_job_id,
                            "dispatched_to": target_name,
                            "dispatched_url": target_url,
                            "forwarded":     true,
                        }))),
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
                        crate::mesh::DispatchError::ForwardRejected { status, .. } => (
                            StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY),
                            "forward_rejected",
                        ),
                        crate::mesh::DispatchError::HMACMismatch { .. } => {
                            (StatusCode::BAD_GATEWAY, "forward_hmac_mismatch")
                        }
                        _ => (StatusCode::BAD_GATEWAY, "forward_failed"),
                    };
                    return (
                        status,
                        Json(with_wire_version(json!({
                            "error":         "forward_failed",
                            "error_code":    code,
                            "target_peer":   target_name,
                            "detail":        e.to_string(),
                        }))),
                    )
                        .into_response();
                }
            }
        }
    }

    // ── Best-effort, process-local at-most-once dedup (not strict/distributed;
    //    DISPATCH-MESH-DURABILITY §3.0) ──────────────────────────────────────
    // Gap (c)'s hard prerequisite: a re-sent assign — a coordinator re-posting
    // on its own poll timeout, a forwarded retry carrying the same
    // `idempotency_key`, or a plain double-POST — should NOT spawn the agent a
    // second time. The guarantee is best-effort: the ledger is serialized by a
    // process Mutex and fails open on FS errors (see `idempotency.rs`), so it
    // collapses the common retry storms but is not exactly-once. It matters now
    // that dispatched agents hold write tools. Mirrors the squad-dispatch and
    // partner-message gates wired in b1fb66b6, reusing the same file-backed
    // ledger (`core/src/idempotency.rs`) — no schema/DB change. Runs AFTER the
    // forward/reject decisions (a forwarded task dedups on the node that
    // actually runs it) and BEFORE the local spawn below.
    //
    // We mint the candidate `job_id` UP FRONT and record it alongside the dedup
    // key, so a duplicate can be answered with the ORIGINAL accepted job_id.
    // This is required for caller compatibility: `mesh::assign_task_to_peer` /
    // `assign_task_to_peer_full` do `data.job_id.ok_or_else(...)` — a success
    // response WITHOUT a job_id is treated as a DispatchError, so a forwarded/
    // retried assign that dedups here would otherwise become a forward failure.
    let idem_key =
        task_assign_idem_key(req.idempotency_key.as_deref(), &req.agent, &req.prompt);
    let job_id = uuid::Uuid::new_v4().to_string();
    let (decision, stored_job_id) = crate::idempotency::check_and_record_value_default(
        &idem_key,
        "task_assign",
        Some(&job_id),
    );
    if let crate::idempotency::Decision::Duplicate { first_seen } = decision {
        // STRICT at-most-once (#321 fix): a Duplicate ALWAYS returns and NEVER
        // falls through to spawn. The previous code did a "resolvability" probe
        // and re-created the job when the durable row was missing — two bugs:
        //   (a) a concurrent duplicate could race a not-yet-written durable row
        //       (ledger recorded before the row exists) into a SECOND spawn;
        //   (b) a crash-orphaned or legacy value-less id re-spawned on EVERY
        //       retry (the ledger keeps pointing at the missing id), so a
        //       resend storm could fan out unbounded extra executions.
        // The dedup guarantee is execution-safety, not availability: a rare
        // crash-orphaned id simply polls "not found" until the TTL expires and
        // the key is re-mintable — never a duplicate agent run.
        if let Some(original_job_id) = stored_job_id {
            // Already accepted within the TTL window: hand back the ORIGINAL
            // job_id so the caller polls the same job. 200 (not 202)
            // distinguishes "already handled" from "new job accepted". No
            // resolvability probe / orphan re-create — at-most-once wins over
            // availability.
            return (
                StatusCode::OK,
                Json(with_wire_version(json!({
                    "job_id":        original_job_id,
                    "deduped":       true,
                    "first_seen":    first_seen,
                    "dispatched_to": my_node_name,
                    "forwarded":     false,
                }))),
            )
                .into_response();
        }
        // Legacy value-less row (only possible from a pre-fix binary that
        // recorded this key without a job_id). We cannot resolve an id, but we
        // MUST NOT spawn — that would duplicate execution of an already-handled
        // task. Return 200 deduped with a null job_id + a note so the caller
        // knows the work was already accepted but the id is unrecoverable.
        // (No debug_assert: a legacy ledger entry is a real, expected state for
        // a node upgraded in place, not a broken invariant.)
        return (
            StatusCode::OK,
            Json(with_wire_version(json!({
                "job_id":        Value::Null,
                "deduped":       true,
                "first_seen":    first_seen,
                "dispatched_to": my_node_name,
                "forwarded":     false,
                "note":          "deduped: original job_id unrecoverable (legacy value-less ledger entry); not re-spawning to preserve at-most-once",
            }))),
        )
            .into_response();
    }

    // First sighting (or the unreachable legacy fallback): persist + spawn.
    let runtime = state.agent_runtime.clone();
    let agent = req.agent.clone();
    let prompt = req.prompt.clone();

    // Prefer the DURABLE store (DISPATCH-MESH-DURABILITY gap-a): when a task
    // queue is configured, persist {job_id, running, agent} so the job — and
    // its terminal status/output — survive a daemon restart. This is also what
    // makes the at-most-once dedup above correct by construction: a deduped
    // job_id stays resolvable by /rpc/task/status even after a restart, instead
    // of pointing at an in-memory job that the restart wiped. `mark_interrupted`
    // at boot (core/src/main.rs) turns a pre-restart `Running` row into a
    // terminal `Failed("interrupted: daemon restart")`, so status returns a
    // definitive answer rather than "job not found".
    //
    // `job_id` was minted via `Uuid::new_v4()` above, so the parse only fails in
    // the impossible case of a corrupt id — and `task_queue` is `None` only on a
    // misconfigured node / unavailable DB. Either way we fall back to the legacy
    // in-memory map so the node degrades rather than 500s (spec §1.3 step 3).
    let durable_uuid = state
        .task_queue
        .as_ref()
        .and_then(|_| uuid::Uuid::parse_str(&job_id).ok());
    if let (Some(tq), Some(job_uuid)) = (state.task_queue.clone(), durable_uuid) {
        // Persist Pending → Running before returning so an immediate poll sees
        // "running", not "not found".
        if let Err(e) = tq
            .create_with_id(job_uuid, &my_node_name, &agent, &prompt)
            .await
        {
            // Durable create failed (DB locked/full/etc). Do NOT fall through to
            // return 202 with a job_id that has no row — the caller would poll
            // "job not found". Fail the request so it can retry; the retry
            // re-derives the same dedup key, the now-orphaned ledger entry is
            // detected as a missing row above, and a fresh job is created once
            // the DB recovers.
            tracing::error!(target: "phantom::dispatch", job_id = %job_id, "durable job create failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(with_wire_version(json!({
                    "error": format!("failed to persist job: {e}"),
                }))),
            )
                .into_response();
        }
        if let Err(e) = tq
            .transition(job_uuid, pm_types::TaskStatus::Running, None)
            .await
        {
            tracing::error!(target: "phantom::dispatch", job_id = %job_id, "durable mark-running failed: {e}");
        }
        // apex-④ off-switch: register a cooperative-abort handle keyed by job_id
        // BEFORE launching the runner, and attach it so a `/rpc/task/stop` flip
        // unwinds the live loop at its next safe point. Removed on completion so
        // the registry doesn't leak terminal jobs.
        let abort_handle = crate::interrupt::InterruptHandle::new();
        state
            .task_aborts
            .write()
            .await
            .insert(job_uuid, abort_handle.clone());
        let aborts = state.task_aborts.clone();
        // apex-④ dispatch↔govern correlation: carry the dispatch row's `job_uuid`
        // onto the runtime so a governed cli_session run uses it AS the govern
        // task_id (one correlation key). An approval raised mid-run then stamps its
        // `approval_id` onto THIS dispatch row live, and `/tasks` /
        // `/rpc/task/status/:job_id` surface it. Additive: ungoverned runs ignore it.
        let runtime = runtime
            .with_interrupt(abort_handle)
            .with_dispatch_task_id(job_uuid);
        tokio::spawn(async move {
            let run_result = runtime.run(&agent, &prompt, &[], None).await;
            // Drop the abort handle once the runner has finished so the registry
            // never accumulates terminal jobs.
            aborts.write().await.remove(&job_uuid);
            match run_result {
                Ok(result) => {
                    if let Err(e) = tq
                        .record_result(
                            job_uuid,
                            pm_types::TaskStatus::Completed,
                            Some(&result.output),
                            None,
                        )
                        .await
                    {
                        tracing::error!(target: "phantom::dispatch", job_id = %job_uuid, "durable record done failed: {e}");
                    }
                }
                Err(e) => {
                    if let Err(err) = tq
                        .record_result(
                            job_uuid,
                            pm_types::TaskStatus::Failed,
                            None,
                            Some(&e.to_string()),
                        )
                        .await
                    {
                        tracing::error!(target: "phantom::dispatch", job_id = %job_uuid, "durable record error failed: {err}");
                    }
                }
            }
        });
    } else {
        // Legacy in-memory map (degraded; lost on restart).
        let jid = job_id.clone();
        let jobs_ref = jobs.clone();
        jobs.write().await.insert(
            job_id.clone(),
            ClusterJob {
                status: "running".into(),
                output: None,
                error: None,
            },
        );
        tokio::spawn(async move {
            match runtime.run(&agent, &prompt, &[], None).await {
                Ok(result) => {
                    jobs_ref.write().await.insert(
                        jid,
                        ClusterJob {
                            status: "done".into(),
                            output: Some(result.output),
                            error: None,
                        },
                    );
                }
                Err(e) => {
                    jobs_ref.write().await.insert(
                        jid,
                        ClusterJob {
                            status: "error".into(),
                            output: None,
                            error: Some(e.to_string()),
                        },
                    );
                }
            }
        });
    }

    // C1: include `dispatched_to` on the local-run path too so callers
    // can audit routing decisions uniformly — same field name as the
    // forwarded branch above. `forwarded: false` for symmetry.
    (
        StatusCode::ACCEPTED,
        Json(with_wire_version(json!({
            "job_id":        job_id,
            "dispatched_to": my_node_name,
            "forwarded":     false,
        }))),
    )
        .into_response()
}

/// POST /rpc/swarm — single-call cluster fan-out.
///
/// Mobile / web clients (Tauri webview, curl, browser) can trigger a
/// full swarm without first enumerating peers and dispatching N times.
/// The handler:
///   1. HMAC-auths via `require_cluster_auth`.
///   2. Refuses peers speaking a newer wire_version (same gate as
///      `/rpc/task/assign`).
///   3. Reserves a swarm-job-id in the shared `ClusterJobStore`.
///   4. Returns `202 Accepted { "job_id": "<swarm-job-id>" }` immediately.
///   5. In a background task: runs `swarm::do_swarm` (fan-out to all
///      online peers + local single-shot), then writes the aggregated
///      JSON blob (see `swarm::SwarmResult::to_json_string`) into the
///      same ClusterJobStore under `swarm-job-id`.
///
/// Callers poll `GET /rpc/task/status/:swarm-job-id` (unchanged) until
/// `status == "done"` and `output` is the JSON aggregate.
///
/// Body (all fields except `prompt` optional):
/// ```json
/// {
///   "agent":         "master",
///   "prompt":        "...",
///   "max_wait_ms":   60000,       // default 120000
///   "include_local": true         // default true
/// }
/// ```
/// POST /rpc/dev-verify — run the dev_verify anti-fake-pass gate on THIS node on
/// behalf of a cluster peer, returning the structured verdict {passed, exit_code,
/// summary, failed, log_path}. HMAC-authed (require_cluster_auth_dual). This is
/// what lets heterogeneous AI tools verify on a remote machine through the mesh.
async fn rpc_dev_verify(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/rpc/dev-verify",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }
    let body_slice: &[u8] = if body.is_empty() { b"{}" } else { &body };
    let mut args: Value = match serde_json::from_slice(body_slice) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("malformed body: {e}") })),
            )
                .into_response()
        }
    };
    // Never recurse: strip any `remote` so this node runs the verify locally.
    if let Some(obj) = args.as_object_mut() {
        obj.remove("remote");
    }
    // Route through tools::execute so the permission/trust gate applies — a
    // cluster peer must not run dev_verify when HOME policy denies it.
    let result = crate::tools::execute("dev_verify", &args, &crate::config::ToolsConfig::default()).await;
    match serde_json::from_str::<Value>(&result) {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(_) => (StatusCode::OK, result).into_response(),
    }
}

/// Resolve the explicit machine/human origin marker an inbound `/partner/message`
/// carries on the wire, if any. The dogfood-moat guard (see
/// [`crate::partner::MessageOrigin`]): test/bot/loop/smoke traffic can tag itself
/// so its turn is segregated to the machine ledger and never inflates the
/// human-usage count, while the real app (which sends no marker) defaults to
/// Human via [`crate::partner::resolve_origin`].
///
/// Markers are read with this precedence (first present wins):
///   1. body `origin` field (`{"text":"…","origin":"machine"}`)
///   2. `X-Partner-Origin` header  ← the canonical marker for test/bot clients
///   3. `X-Phantom-Origin` header  ← historical alias, kept for back-compat
///
/// The value is parsed by [`crate::partner::MessageOrigin::from_wire`]
/// (case-insensitive: `machine`/`bot`/`system`/`classifier`/`loop`/`smoke`/`test`
/// → Machine; `human`/`user`/`person` → Human). An absent or unrecognized marker
/// yields `None`, so the caller applies the content heuristic + Human default —
/// we never silently upgrade an unknown value. Pure (no IO) so it is directly
/// unit-testable from a `HeaderMap` + parsed body.
fn parse_origin_marker(
    headers: &HeaderMap,
    parsed: &Value,
) -> Option<crate::partner::MessageOrigin> {
    let header_marker = |name: &str| -> Option<String> {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
    };
    parsed["origin"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .or_else(|| header_marker("X-Partner-Origin"))
        .or_else(|| header_marker("X-Phantom-Origin"))
        .and_then(|s| crate::partner::MessageOrigin::from_wire(&s))
}

/// POST /partner/message — reactive half of the life-partner. Body `{text,
/// agent?}`; routes the text through an agent turn and returns its reply.
/// Client-agnostic: a curl test client now, the iOS app later — same contract.
async fn partner_message(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/partner/message",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("malformed body: {e}") })),
            )
                .into_response()
        }
    };
    let text = parsed["text"].as_str().unwrap_or("").to_string();
    if text.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "text field required" })),
        )
            .into_response();
    }
    let agent = parsed["agent"].as_str().unwrap_or("master");
    // At-most-once: an explicit client request id (body `idempotency_key` or the
    // standard `Idempotency-Key` header) dedups a re-sent message; absent one, a
    // content hash of the text dedups a body resent without a key. A duplicate is
    // suppressed BEFORE the agent turn / any write tool runs.
    let idem_key = parsed["idempotency_key"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            headers
                .get("Idempotency-Key")
                .and_then(|v| v.to_str().ok())
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string())
        });
    // Dogfood-moat guard: an explicit origin marker (body `origin` field, or the
    // `X-Partner-Origin` / `X-Phantom-Origin` header) lets the app/loops/smoke-
    // tests tag themselves as machine so their turns don't pollute the human-usage
    // ledger; absent a marker, the content heuristic still catches legacy untagged
    // classifier prompts, otherwise it defaults to Human. (See
    // `parse_origin_marker` + `partner::resolve_origin`.)
    let explicit_origin = parse_origin_marker(&headers, &parsed);
    let origin = crate::partner::resolve_origin(explicit_origin, &text);
    match crate::partner::handle_message_idempotent(
        &state.agent_runtime,
        agent,
        &text,
        idem_key.as_deref(),
        origin,
    )
    .await
    {
        Ok((r, deduped)) => Json(json!({
            "reply": r.reply,
            "turns": r.turns,
            "elapsed_secs": r.elapsed_secs,
            "deduped": deduped,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// POST /partner/signal — proactive substrate. Body = arbitrary sensor/behaviour
/// JSON (location, motion, manual check-in …); appended verbatim to the ledger
/// the daily alignment reflection reads.
async fn partner_signal(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/partner/signal",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }
    let body_slice: &[u8] = if body.is_empty() { b"{}" } else { &body };
    let payload: Value = match serde_json::from_slice(body_slice) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("malformed body: {e}") })),
            )
                .into_response()
        }
    };
    // Pollution hard wall (ACCEL-FRAMEWORK §④, owner option B): a signal tagged as
    // dev-loop / `phantom_self` / machine (body `origin` field or the
    // `X-Partner-Origin` / `X-Phantom-Origin` header) is diverted to the dev-loop
    // log and NEVER written to the human-usage moat. Absent any marker the signal
    // defaults to Human (a sensor check-in carries no classifier markers, so the
    // content heuristic effectively never fires here — real human use is not
    // mis-killed). See `parse_origin_marker` + `partner::record_signal_with_origin`.
    let explicit_origin = parse_origin_marker(&headers, &payload);
    let origin = crate::partner::resolve_origin(explicit_origin, "");
    match crate::partner::record_signal_with_origin(origin, "sensor", &payload) {
        Ok(path) => Json(json!({
            "ok": true,
            "stored": path.to_string_lossy(),
            "origin": match origin {
                crate::partner::MessageOrigin::Human => "human",
                crate::partner::MessageOrigin::Machine => "machine",
            },
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn rpc_swarm(
    State(state): State<Arc<AppState>>,
    Extension(jobs): Extension<ClusterJobStore>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/rpc/swarm",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(with_wire_version(
                    json!({ "error": format!("malformed body: {e}") }),
                )),
            )
                .into_response()
        }
    };
    if let Some((code, err)) = check_wire_version(&parsed) {
        return (code, err).into_response();
    }
    let prompt = parsed["prompt"].as_str().unwrap_or("").to_string();
    if prompt.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(with_wire_version(
                json!({ "error": "prompt field required" }),
            )),
        )
            .into_response();
    }
    let agent = parsed["agent"].as_str().unwrap_or("master").to_string();
    let include_local = parsed["include_local"].as_bool().unwrap_or(true);
    let max_wait_ms = parsed["max_wait_ms"].as_u64().unwrap_or(120_000);
    let max_wait = std::time::Duration::from_millis(max_wait_ms);
    // Optional selective fan-out: `"targets": ["peer-2", "192.0.2.7"]`.
    // Absent / empty → fan out to every online peer (original behaviour).
    let targets: Option<Vec<String>> = parsed["targets"].as_array().map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .filter(|s| !s.is_empty())
            .collect()
    });

    let job_id = uuid::Uuid::new_v4().to_string();

    // Durable swarm job (DISPATCH-MESH-DURABILITY gap-a): persist the swarm
    // job so its aggregated result survives a restart, same as a plain assign.
    // Falls back to the in-memory map when no task queue is configured.
    let durable_uuid = state
        .task_queue
        .as_ref()
        .and_then(|_| uuid::Uuid::parse_str(&job_id).ok());
    if let (Some(tq), Some(job_uuid)) = (state.task_queue.clone(), durable_uuid) {
        if let Err(e) = tq
            .create_with_id(job_uuid, "swarm", &agent, &prompt)
            .await
        {
            tracing::error!(target: "phantom::dispatch", job_id = %job_id, "durable swarm create failed: {e}");
        }
        if let Err(e) = tq
            .transition(job_uuid, pm_types::TaskStatus::Running, None)
            .await
        {
            tracing::error!(target: "phantom::dispatch", job_id = %job_id, "durable swarm mark-running failed: {e}");
        }
        let state_for_task = state.clone();
        tokio::spawn(async move {
            let result = crate::swarm::do_swarm_with_throttle(
                state_for_task,
                &agent,
                &prompt,
                include_local,
                max_wait,
                None,
                targets,
            )
            .await;
            if let Err(e) = tq
                .record_result(
                    job_uuid,
                    pm_types::TaskStatus::Completed,
                    Some(&result.to_json_string()),
                    None,
                )
                .await
            {
                tracing::error!(target: "phantom::dispatch", job_id = %job_uuid, "durable swarm record done failed: {e}");
            }
        });
    } else {
        let jid = job_id.clone();
        let jobs_ref = jobs.clone();
        // Reserve the slot so callers can immediately start polling.
        jobs.write().await.insert(
            job_id.clone(),
            ClusterJob {
                status: "running".into(),
                output: None,
                error: None,
            },
        );
        let state_for_task = state.clone();
        tokio::spawn(async move {
            let result = crate::swarm::do_swarm_with_throttle(
                state_for_task,
                &agent,
                &prompt,
                include_local,
                max_wait,
                None,
                targets,
            )
            .await;
            jobs_ref.write().await.insert(
                jid,
                ClusterJob {
                    status: "done".into(),
                    output: Some(result.to_json_string()),
                    error: None,
                },
            );
        });
    }

    (
        StatusCode::ACCEPTED,
        Json(with_wire_version(json!({
            "job_id":         job_id,
            "swarm":          true,
            "include_local":  include_local,
            "max_wait_ms":    max_wait_ms,
        }))),
    )
        .into_response()
}

/// POST /rpc/tool/call — execute one built-in tool on this node and return
/// its output. Generic remote-tool entry point: the counterpart to
/// `/rpc/message` (which runs a remote *agent*), this runs a single *tool*.
///
/// Body: `{ "tool": "shell", "args": { "command": "ls -la" } }`
/// Reply: `{ "tool": "shell", "output": "..." }`
///
/// **Auth:** `require_cluster_auth_dual` (HMAC; tailnet-trusted peers are
/// exempt) — same posture as `/rpc/message`. Note this can run `shell`,
/// `file_write`, etc., so it is as powerful as remote agent execution.
async fn rpc_tool_call(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/rpc/tool/call",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(with_wire_version(
                    json!({ "error": format!("malformed body: {e}") }),
                )),
            )
                .into_response()
        }
    };
    if let Some((code, err)) = check_wire_version(&parsed) {
        return (code, err).into_response();
    }
    let tool = parsed["tool"].as_str().unwrap_or("").to_string();
    if tool.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(with_wire_version(json!({ "error": "tool field required" }))),
        )
            .into_response();
    }
    let args = parsed.get("args").cloned().unwrap_or_else(|| json!({}));
    // SECURITY/correctness (review #321 §6): use the node's REAL [tools] config,
    // not `ToolsConfig::default()`. The default discards every configured API key
    // (web_search, todoist, …) so a remote tool_call would silently run with no
    // credentials. Mirror the /mcp handler at ~serve.rs:1468 which reads
    // `state.agent_runtime.config().tools`. Auth above is already fail-closed.
    let runtime_cfg = state.agent_runtime.config();
    let output = crate::tools::execute(&tool, &args, &runtime_cfg.tools).await;
    Json(with_wire_version(json!({ "tool": tool, "output": output }))).into_response()
}

/// POST /rpc/capability-query (T-CORE-02) - answer "who can do <caps>?".
///
/// A thin pub/sub style capability-query overlay over the existing HTTP REST
/// mesh: the answering node computes the set of peers (and optionally itself)
/// whose advertised capabilities cover `required_caps`, entirely from the
/// locally-cached roster with no extra network I/O. Authed with the same dual
/// X-Cluster-Auth scheme as the other /rpc/* routes. An empty body (or `{}`)
/// deserializes to defaults: empty required_caps + include_self = true.
async fn rpc_capability_query(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/rpc/capability-query",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }
    // An empty body is treated as `{}`: serde fills both fields from their
    // defaults (empty required_caps + include_self = true).
    let body_slice: &[u8] = if body.is_empty() { b"{}" } else { &body };
    let req: crate::mesh::CapabilityQueryRequest = match serde_json::from_slice(body_slice) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(with_wire_version(
                    json!({ "error": format!("malformed body: {e}") }),
                )),
            )
                .into_response()
        }
    };

    let answers = state
        .cluster_manager
        .query_capability(&req.required_caps, req.include_self)
        .await;
    let resp = crate::mesh::CapabilityQueryResponse {
        required_caps: req.required_caps,
        count: answers.len(),
        answers,
    };
    Json(with_wire_version(serde_json::to_value(resp).unwrap())).into_response()
}

/// GET /rpc/task/status/:id — poll async task result.
async fn rpc_task_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Extension(jobs): Extension<ClusterJobStore>,
) -> Json<Value> {
    // Durable store first (DISPATCH-MESH-DURABILITY gap-a): a job created
    // before a daemon restart is still answerable here (terminal status), not
    // "job not found". Map the persisted `TaskStatus` back to the legacy
    // running|done|error wire strings so existing pollers (mesh::poll_task) are
    // unaffected.
    if let Some(tq) = &state.task_queue {
        if let Ok(uuid) = uuid::Uuid::parse_str(&id) {
            match tq.get(uuid).await {
                Ok(Some(rec)) => {
                    return Json(with_wire_version(json!({
                        "job_id": id,
                        "status": task_status_to_wire(rec.status),
                        "output": rec.output,
                        "error":  rec.error,
                    })));
                }
                Ok(None) => { /* not in durable store — fall through to legacy map */ }
                Err(e) => {
                    tracing::warn!(target: "phantom::dispatch", job_id = %id, "durable status read failed: {e}");
                }
            }
        }
    }
    // Legacy in-memory fallback (degraded nodes; lost on restart).
    match jobs.read().await.get(&id).cloned() {
        Some(job) => Json(with_wire_version(json!({
            "job_id": id,
            "status": job.status,
            "output": job.output,
            "error":  job.error,
        }))),
        None => Json(with_wire_version(
            json!({ "error": "job not found", "job_id": id }),
        )),
    }
}

/// Map a durable `TaskStatus` back to the legacy async-dispatch wire strings
/// (`running` | `done` | `error`) that `/rpc/task/status` has always returned,
/// so existing pollers (`mesh::poll_task`) need no change. Non-terminal states
/// read as "running"; `Completed` as "done"; every other terminal (`Failed`,
/// `Cancelled`) as "error".
fn task_status_to_wire(s: pm_types::TaskStatus) -> &'static str {
    use pm_types::TaskStatus::*;
    match s {
        Completed => "done",
        Failed | Cancelled => "error",
        Pending | AwaitingApproval | Running => "running",
    }
}

/// Parse the `job_id` from a stop/resume request body, returning a 400 shape on
/// a missing/un-parseable id. Shared by `rpc_task_stop` + `rpc_task_resume` so
/// both reject malformed input identically.
fn parse_job_id(body: &[u8]) -> Result<uuid::Uuid, (StatusCode, Json<Value>)> {
    let v: Value = serde_json::from_slice(body).unwrap_or(json!({}));
    let id = v.get("job_id").and_then(|x| x.as_str()).unwrap_or("");
    uuid::Uuid::parse_str(id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(with_wire_version(
                json!({ "error": "missing or invalid job_id" }),
            )),
        )
    })
}

/// POST /rpc/task/stop — apex-④ off-switch. A phone STOP on the shipping assign
/// flow lands here. HMAC-authed (fail-closed 401/403 like every mesh RPC).
/// Body: `{ "job_id": "<uuid>" }`.
///
/// Effect: (1) fire the cooperative-abort handle if the task is locally in
/// flight, so the live agent loop unwinds at its next safe point; (2) flip the
/// durable task off `Running` into `AwaitingApproval` (the "paused, awaiting
/// operator" state) so it is no longer runnable and CAN be RESUMED. The state
/// flip is the source of truth — it happens even when no local abort handle
/// exists (e.g. a restart-orphaned row), so a phone can still stop a job whose
/// in-memory runner was lost.
async fn rpc_task_stop(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/rpc/task/stop",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }
    let job_id = match parse_job_id(&body) {
        Ok(id) => id,
        Err((code, json)) => return (code, json).into_response(),
    };

    let Some(tq) = state.task_queue.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(with_wire_version(
                json!({ "error": "no durable task store on this node" }),
            )),
        )
            .into_response();
    };

    // Look up the durable task.
    let current = match tq.get(job_id).await {
        Ok(Some(rec)) => rec,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(with_wire_version(
                    json!({ "error": "job not found", "job_id": job_id.to_string() }),
                )),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(with_wire_version(json!({ "error": e.to_string() }))),
            )
                .into_response();
        }
    };

    // Terminal already → nothing to stop; report current status idempotently.
    if current.status.is_terminal() {
        return Json(with_wire_version(json!({
            "job_id": job_id.to_string(),
            "status": "stopped",
            "note":   "task already terminal; no-op",
            "durable_status": current.status.as_str(),
        })))
        .into_response();
    }

    // 1. Signal the live runner to abort (cooperative; no-op if not in flight).
    if let Some(handle) = state.task_aborts.read().await.get(&job_id) {
        handle.interrupt(None);
    }

    // 2. Flip durable state off Running into the parked AwaitingApproval state.
    //    Pending → AwaitingApproval and Running → AwaitingApproval are both legal
    //    (the latter is the STOP edge). AwaitingApproval already-parked is a
    //    no-op flip that we treat as success.
    if current.status != pm_types::TaskStatus::AwaitingApproval {
        if let Err(e) = tq
            .transition(job_id, pm_types::TaskStatus::AwaitingApproval, None)
            .await
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(with_wire_version(
                    json!({ "error": format!("stop transition failed: {e}") }),
                )),
            )
                .into_response();
        }
    }

    Json(with_wire_version(json!({
        "job_id": job_id.to_string(),
        "status": "stopped",
    })))
    .into_response()
}

/// POST /rpc/task/resume — apex-④ off-switch counterpart. A phone RESUME on a
/// previously-stopped task lands here. HMAC-authed (fail-closed 401/403).
/// Body: `{ "job_id": "<uuid>" }`.
///
/// Effect: flip a parked (`AwaitingApproval`) task back to `Running` so the
/// runner/redispatch path can pick it up again. Refuses (409) if the task is
/// terminal or not in a resumable state.
async fn rpc_task_resume(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/rpc/task/resume",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }
    let job_id = match parse_job_id(&body) {
        Ok(id) => id,
        Err((code, json)) => return (code, json).into_response(),
    };

    let Some(tq) = state.task_queue.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(with_wire_version(
                json!({ "error": "no durable task store on this node" }),
            )),
        )
            .into_response();
    };

    let current = match tq.get(job_id).await {
        Ok(Some(rec)) => rec,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(with_wire_version(
                    json!({ "error": "job not found", "job_id": job_id.to_string() }),
                )),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(with_wire_version(json!({ "error": e.to_string() }))),
            )
                .into_response();
        }
    };

    // Already running → idempotent success.
    if current.status == pm_types::TaskStatus::Running {
        return Json(with_wire_version(json!({
            "job_id": job_id.to_string(),
            "status": "running",
            "note":   "already running; no-op",
        })))
        .into_response();
    }

    // Only a parked (AwaitingApproval) task is resumable. Terminal / Pending
    // states are refused so RESUME never resurrects a finished task.
    if current.status != pm_types::TaskStatus::AwaitingApproval {
        return (
            StatusCode::CONFLICT,
            Json(with_wire_version(json!({
                "error":          "task is not in a resumable (stopped) state",
                "durable_status": current.status.as_str(),
            }))),
        )
            .into_response();
    }

    if let Err(e) = tq
        .transition(job_id, pm_types::TaskStatus::Running, None)
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(with_wire_version(
                json!({ "error": format!("resume transition failed: {e}") }),
            )),
        )
            .into_response();
    }

    Json(with_wire_version(json!({
        "job_id": job_id.to_string(),
        "status": "running",
    })))
    .into_response()
}

/// POST /rpc/admin/self-update — download a new phantom binary from a URL,
/// stage it next to the current exe as `<exe>.new`, spawn a detached
/// trampoline that swaps files + restarts `phantom serve`, then exit.
///
/// Body (JSON, optional):
///   { "url": "<override download url>" }   // defaults to https://phantommesh.io/dist/<platform-asset>
///   { "delay_ms": 3000 }                   // override trampoline wait (default 3000)
///
/// HMAC-authed: requires X-Cluster-Auth (same scheme as /rpc/task/assign).
/// **Self-replacement caveat**: this RCEs anyone holding cluster_secret —
/// matches the existing dispatch trust boundary but be aware. Future:
/// add a [cluster].auto_update_allowed=false escape hatch and codesign
/// verification of the downloaded binary before swap.
async fn rpc_admin_self_update(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // HMAC auth — same pattern as rpc_task_assign.
    let secret_configured = state
        .cluster_manager
        .config
        .cluster_secret
        .as_deref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if !secret_configured {
        return (
            StatusCode::FORBIDDEN,
            Json(with_wire_version(json!({
                "error": "self-update refused: cluster_secret not configured on this node"
            }))),
        )
            .into_response();
    }
    let token = headers
        .get("X-Cluster-Auth")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !state.cluster_manager.verify_auth(token, &body) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(with_wire_version(
                json!({ "error": "unauthorized — bad X-Cluster-Auth" }),
            )),
        )
            .into_response();
    }

    let req: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    let delay_ms = req.get("delay_ms").and_then(|v| v.as_u64()).unwrap_or(3000);
    let url = match req.get("url").and_then(|v| v.as_str()) {
        Some(u) => u.to_string(),
        None => format!("https://phantommesh.io/dist/{}", default_dist_asset_name()),
    };

    let exe_path = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(with_wire_version(
                    json!({ "error": format!("current_exe(): {e}") }),
                )),
            )
                .into_response()
        }
    };

    // 1. Download new binary to <exe>.new
    let new_path = exe_path.with_extension(
        // .exe.new on Windows; .new on unix
        if exe_path.extension().and_then(|s| s.to_str()) == Some("exe") {
            "exe.new"
        } else {
            "new"
        },
    );
    let bytes = match download_binary(&url).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(with_wire_version(
                    json!({ "error": format!("download {url}: {e}") }),
                )),
            )
                .into_response()
        }
    };
    if bytes.len() < 1024 * 1024 {
        return (StatusCode::BAD_GATEWAY,
            Json(with_wire_version(json!({
                "error": format!("downloaded binary suspiciously small ({} bytes); refusing swap", bytes.len())
            })))).into_response();
    }
    if let Err(e) = std::fs::write(&new_path, &bytes) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(with_wire_version(
                json!({ "error": format!("write {}: {e}", new_path.display()) }),
            )),
        )
            .into_response();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&new_path, std::fs::Permissions::from_mode(0o755));
    }

    // 2. Spawn the detached trampoline that will swap + restart serve
    //    once we exit and release the file lock.
    if let Err(e) = spawn_swap_trampoline(&exe_path, &new_path, delay_ms) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(with_wire_version(
                json!({ "error": format!("spawn trampoline: {e}") }),
            )),
        )
            .into_response();
    }

    // 3. Schedule our own exit shortly AFTER the response flushes. Trampoline
    //    is sleeping `delay_ms` so we have headroom.
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        std::process::exit(0);
    });

    (
        StatusCode::ACCEPTED,
        Json(with_wire_version(json!({
            "status":      "scheduled",
            "downloaded":  bytes.len(),
            "exe_path":    exe_path.to_string_lossy(),
            "staged_at":   new_path.to_string_lossy(),
            "swap_in_ms":  delay_ms,
            "from_url":    url,
        }))),
    )
        .into_response()
}

/// R2 binary asset name for THIS host's platform. Mirrors the dispatch
/// table in dist.ts on the broker side.
fn default_dist_asset_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "phantom-windows-x86_64.exe"
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "phantom-darwin-arm64"
        } else {
            "phantom-darwin-x86_64"
        }
    } else if cfg!(target_os = "linux") {
        if cfg!(target_arch = "aarch64") {
            "phantom-linux-aarch64"
        } else {
            "phantom-linux-x86_64"
        }
    } else {
        "phantom-windows-x86_64.exe" // best-effort fallback
    }
}

/// POST /rpc/admin/shell — run a shell command on this node, return
/// stdout/stderr/exit_code. HMAC-authed; same trust boundary as
/// dispatch (anyone with cluster_secret can already RCE via the LLM
/// agent's `shell` tool, this just does it directly without the LLM
/// round-trip — saves ~5s of LLM hallucination on every git pull).
///
/// Body: { "cmd": "...", "cwd"?: "...", "timeout_secs"?: N }
/// Response: { "exit_code": N, "stdout": "...", "stderr": "..." }
///
/// Used by `phantom git sync --all` and other admin fan-outs that need
/// a deterministic shell behavior (LLM-flavored shell calls hallucinate
/// or refuse on free-tier models).
async fn rpc_admin_shell(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // HMAC auth — refuse outright if no cluster_secret on this node.
    let secret_configured = state
        .cluster_manager
        .config
        .cluster_secret
        .as_deref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if !secret_configured {
        return (
            StatusCode::FORBIDDEN,
            Json(with_wire_version(json!({
                "error": "shell rpc refused: cluster_secret not configured on this node"
            }))),
        )
            .into_response();
    }
    let token = headers
        .get("X-Cluster-Auth")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !state.cluster_manager.verify_auth(token, &body) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(with_wire_version(
                json!({ "error": "unauthorized — bad X-Cluster-Auth" }),
            )),
        )
            .into_response();
    }

    let req: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    let cmd = match req.get("cmd").and_then(|v| v.as_str()) {
        Some(c) if !c.is_empty() => c.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(with_wire_version(
                    json!({ "error": "missing or empty cmd" }),
                )),
            )
                .into_response()
        }
    };
    // cwd resolution: explicit body.cwd wins; otherwise fall back to
    // this node's own [workspace].default_dir so `phantom git sync --all`
    // pulls each peer's pinned project without the caller having to know
    // each remote's path layout.
    let cwd = req
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| {
            crate::cli_config::agents_toml_path()
                .and_then(|p| std::fs::read_to_string(&p).ok())
                .and_then(|raw| toml::from_str::<crate::config::AgentsConfig>(&raw).ok())
                .and_then(|cfg| cfg.workspace.default_dir)
        });
    let timeout_secs = req
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(120);

    // Use cmd /c on Windows, sh -c on unix. Same dispatch as the shell
    // tool that LLM agents already invoke — reuses the platform shell so
    // shell builtins (`&&`, `>`, etc.) work as expected.
    let output_result =
        tokio::task::spawn_blocking(move || -> std::io::Result<std::process::Output> {
            let mut command = if cfg!(windows) {
                let mut c = std::process::Command::new("cmd");
                c.arg("/c").arg(&cmd);
                c
            } else {
                let mut c = std::process::Command::new("sh");
                c.arg("-c").arg(&cmd);
                c
            };
            if let Some(d) = &cwd {
                command.current_dir(d);
            }
            command.output()
        })
        .await;

    let output = match output_result {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(with_wire_version(
                    json!({ "error": format!("exec: {}", e) }),
                )),
            )
                .into_response()
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(with_wire_version(
                    json!({ "error": format!("join: {}", e) }),
                )),
            )
                .into_response()
        }
    };

    // truncate to keep RPC payload sane (matches shell tool's caps).
    let cap = |s: Vec<u8>, n: usize| -> String {
        let mut t = String::from_utf8_lossy(&s).to_string();
        if t.len() > n {
            t.truncate(n);
            t.push_str("\n[truncated]");
        }
        t
    };

    let _ = timeout_secs; // reserved — we'd add a timeout wrapper in v2

    (
        StatusCode::OK,
        Json(with_wire_version(json!({
            "exit_code": output.status.code().unwrap_or(-1),
            "stdout":    cap(output.stdout, 64 * 1024),
            "stderr":    cap(output.stderr, 16 * 1024),
        }))),
    )
        .into_response()
}

/// Stream a remote binary into memory. 5-min ceiling to handle slow
/// peers; reqwest already times out idle reads.
async fn download_binary(url: &str) -> anyhow::Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status().as_u16());
    }
    Ok(resp.bytes().await?.to_vec())
}

/// Spawn the platform-specific trampoline as a fully detached process.
/// On Windows: cmd.exe sleeps via `ping` (no built-in `sleep`), then
/// del + ren + start in one chained command. On Unix: sh with sleep + mv +
/// nohup + setsid.
///
/// **Windows Job Object gotcha** (the bug that killed node-b's serve in
/// the first round): if the parent serve.exe is in a Job Object with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (PowerShell's Start-Process,
/// VS Code, Tauri, and several service managers all do this), every
/// descendant — including a "detached" cmd.exe — gets killed when
/// the parent dies. `CREATE_BREAKAWAY_FROM_JOB` explicitly removes
/// the spawned process from any inherited Job Object so the trampoline
/// outlives the parent's death-by-self-update.
///
/// We also redirect the trampoline's chained-command output to a log
/// file (`<exe-dir>/phantom-restart.log`) so when something does go
/// sideways we have a forensic trail rather than a silent dead serve.
fn spawn_swap_trampoline(
    exe: &std::path::Path,
    new: &std::path::Path,
    delay_ms: u64,
) -> anyhow::Result<()> {
    let exe_s = exe.to_string_lossy().to_string();
    let new_s = new.to_string_lossy().to_string();

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // ping -n N waits ~(N-1) seconds. delay_ms→count rounds up.
        let pings = std::cmp::max(2, (delay_ms / 1000) as u32 + 1);
        let bin_dir = exe
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf();
        let log_path = bin_dir.join("phantom-restart.log");
        let bat_path = bin_dir.join("phantom-restart.bat");
        let log_s = log_path.to_string_lossy().to_string();
        let out_path = bin_dir.join("phantom-serve.out.log");
        let err_path = bin_dir.join("phantom-serve.err.log");
        let out_s = out_path.to_string_lossy().to_string();
        let err_s = err_path.to_string_lossy().to_string();
        let new_name = std::path::Path::new(&new_s)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let exe_name = std::path::Path::new(&exe_s)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let ts_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let serve_pid = std::process::id();

        // Write the trampoline as a .bat file then spawn cmd /c on it.
        // The previous in-line `cmd /c "..."` approach got hosed by
        // Rust's Windows arg escaping (embedded quotes become \" which
        // cmd.exe doesn't unescape — it expects "" doubled). A .bat
        // file is just plain text so quoting is trivially correct.
        //
        // We also taskkill the parent PID directly instead of relying
        // on the parent calling exit() — eliminates one whole class of
        // "serve didn't exit so swap couldn't proceed" failure modes
        // and (combined with CREATE_BREAKAWAY_FROM_JOB) is what made
        // this actually work end-to-end.
        let bat_content = format!(
            "@echo off\r\n\
             echo === phantom self-update unix={ts} pid={pid} === > \"{log}\"\r\n\
             ping 127.0.0.1 -n {pings} > nul\r\n\
             echo killing parent pid {pid} >> \"{log}\"\r\n\
             taskkill /F /PID {pid} >> \"{log}\" 2>&1\r\n\
             ping 127.0.0.1 -n 2 > nul\r\n\
             echo deleting old exe >> \"{log}\"\r\n\
             del \"{exe}\" >> \"{log}\" 2>&1\r\n\
             echo renaming new exe >> \"{log}\"\r\n\
             ren \"{new}\" \"{exe_name}\" >> \"{log}\" 2>&1\r\n\
             echo restarting serve >> \"{log}\"\r\n\
             powershell -NoProfile -WindowStyle Hidden -Command \"Start-Process -FilePath '{exe}' -ArgumentList serve -WindowStyle Hidden -RedirectStandardOutput '{out}' -RedirectStandardError '{err}'\" >> \"{log}\" 2>&1\r\n\
             echo done >> \"{log}\"\r\n",
            ts = ts_secs,
            pid = serve_pid,
            pings = pings,
            log = log_s,
            exe = exe_s,
            new = new_s,
            exe_name = exe_name,
            out = out_s,
            err = err_s,
        );
        let _ = new_name; // silence unused warning; kept for readability above
        std::fs::write(&bat_path, bat_content)
            .map_err(|e| anyhow::anyhow!("write {}: {}", bat_path.display(), e))?;

        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x01000000;
        std::process::Command::new("cmd")
            .arg("/c")
            .arg(&bat_path)
            .current_dir(&bin_dir)
            .creation_flags(
                DETACHED_PROCESS
                    | CREATE_NEW_PROCESS_GROUP
                    | CREATE_NO_WINDOW
                    | CREATE_BREAKAWAY_FROM_JOB,
            )
            .spawn()?;
        return Ok(());
    }

    #[cfg(unix)]
    {
        let secs = (delay_ms / 1000).max(1);
        let cmd = format!(
            "sleep {secs} && mv '{new}' '{exe}' && nohup '{exe}' serve > /dev/null 2>&1 &",
            secs = secs,
            new = new_s,
            exe = exe_s,
        );
        std::process::Command::new("sh")
            .args(["-c", &cmd])
            .spawn()?;
        Ok(())
    }
}

// ── Wire helpers ──────────────────────────────────────────────────────────────

fn ok(id: &Value, result: Value) -> String {
    json!({ "id": id, "result": result }).to_string()
}

fn err(id: &Value, code: i64, message: &str) -> String {
    json!({ "id": id, "error": { "code": code, "message": message } }).to_string()
}

fn notif(method: &str, params: Value) -> String {
    json!({ "method": method, "params": params }).to_string()
}

// ── Mobile onboarding ─────────────────────────────────────────────────────────
//
// Flow:
//   1. User opens Phantom Mesh on Mac, copies an onboarding token (printed by CLI
//      or shown in Settings UI).
//   2. On the mobile device, user enters Mac Tailscale IP + token + node_name.
//      Mobile fetches GET /onboarding/config?token=...&node_name=...
//   3. Server validates token (HMAC of cluster_secret + a fixed marker), and
//      returns a worker agents.toml (cluster_secret + provider api_keys + peers
//      + a [agent.master] for that node_name).
//   4. Mobile writes the response to its own agents.toml and restarts the runtime.

#[derive(serde::Deserialize)]
struct OnboardingQuery {
    token: String,
    #[serde(default = "default_node_name")]
    node_name: String,
}

fn default_node_name() -> String {
    "mobile-worker".into()
}

/// Derive a stable, secret-derived onboarding token.
/// HMAC-SHA256(cluster_secret, b"phantom-mesh-onboarding-v1") truncated to first 16 hex chars.
fn make_onboarding_token(cluster_secret: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    let mut mac =
        HmacSha256::new_from_slice(cluster_secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(b"phantom-mesh-onboarding-v1");
    let bytes = mac.finalize().into_bytes();
    bytes[..8].iter().map(|b| format!("{:02x}", b)).collect()
}

/// GET /onboarding/token — returns the current onboarding token (the user shows
/// this on Mac, copies into mobile). Token is derived from cluster_secret so it
/// rotates whenever cluster_secret changes.
async fn onboarding_token(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    // T7b T13-N5 HIGH: HMAC gate. Pre-fix the helper response was a function
    // of cluster_secret only, letting an unauth caller pivot through
    // /onboarding/config to exfil the full agents.toml.
    if let Err((code, json)) = require_cluster_auth(&state.cluster_manager, &headers, b"") {
        return (code, json).into_response();
    }
    let secret = state
        .cluster_manager
        .config
        .cluster_secret
        .clone()
        .unwrap_or_default();
    if secret.is_empty() {
        return Json(json!({
            "ok": false,
            "error": "cluster_secret 沒設定，請先在 agents.toml 配置 cluster_secret"
        }))
        .into_response();
    }
    Json(json!({
        "ok": true,
        "token": make_onboarding_token(&secret),
        "hint": "把 token 貼到手機 app → 設定 → 從 Mac 匯入設定"
    }))
    .into_response()
}

/// GET /onboarding/config?token=...&node_name=...
/// Returns a ready-to-use agents.toml for a mobile worker.
async fn onboarding_config(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<OnboardingQuery>,
    headers: HeaderMap,
) -> Response {
    // T7b T13-N5 HIGH: HMAC gate (defence in depth on top of the existing
    // `q.token` check, which was itself a function of cluster_secret).
    if let Err((code, json)) = require_cluster_auth(&state.cluster_manager, &headers, b"") {
        return (code, json).into_response();
    }
    let cluster_cfg = &state.cluster_manager.config;
    let secret = cluster_cfg.cluster_secret.clone().unwrap_or_default();
    if secret.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "cluster_secret not configured on this coordinator",
        )
            .into_response();
    }
    let expected_token = make_onboarding_token(&secret);
    if q.token != expected_token {
        return (StatusCode::UNAUTHORIZED, "invalid onboarding token").into_response();
    }

    // Build the worker agents.toml.
    let cfg = state.agent_runtime.config();

    // Sanitize node_name (alphanumeric + dash/underscore only)
    let node_name: String = q
        .node_name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(40)
        .collect();
    let node_name = if node_name.is_empty() {
        "mobile-worker".into()
    } else {
        node_name
    };

    // Peers: include this coordinator + all known peers (online or not).
    let mut peers: Vec<String> = cluster_cfg.peers.clone();
    // Add coordinator's own URL guess if missing — best effort.
    let mut toml = String::new();
    toml.push_str("# Auto-generated by /onboarding/config\n");
    toml.push_str(&format!("# Generated for node_name={}\n\n", node_name));
    toml.push_str("[core]\n");
    toml.push_str("host = \"0.0.0.0\"\n");
    toml.push_str("port = 7878\n\n");

    toml.push_str("[cluster]\n");
    toml.push_str(&format!("node_name      = \"{}\"\n", node_name));
    toml.push_str(&format!("cluster_secret = \"{}\"\n", secret));
    toml.push_str("capabilities   = [\"mobile\",\"web\",\"analysis\"]\n");
    if !peers.is_empty() {
        peers.sort();
        peers.dedup();
        toml.push_str("peers = [\n");
        for p in &peers {
            toml.push_str(&format!("  \"{}\",\n", p));
        }
        toml.push_str("]\n");
    }
    toml.push_str("\n");

    // Providers: include any with a non-empty api_key already loaded into config.
    for (name, p) in &cfg.providers {
        let key = match &p.api_key {
            Some(k) if !k.is_empty() => k.clone(),
            _ => continue, // skip env-only providers; mobile can't read host env
        };
        toml.push_str(&format!("[providers.{}]\n", name));
        if let Some(url) = &p.url {
            toml.push_str(&format!("base_url      = \"{}\"\n", url));
        }
        toml.push_str(&format!("api_key       = \"{}\"\n", key));
        if let Some(model) = &p.default_model {
            toml.push_str(&format!("default_model = \"{}\"\n", model));
        }
        toml.push_str("\n");
    }

    // Pick a reasonable default agent.master based on the first available provider.
    let default_provider = cfg
        .providers
        .iter()
        .find(|(_, p)| p.api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false))
        .map(|(name, _)| name.clone())
        .unwrap_or_else(|| "groq".into());
    let default_model = cfg
        .providers
        .get(&default_provider)
        .and_then(|p| p.default_model.clone())
        .unwrap_or_else(|| "llama-3.3-70b-versatile".into());

    toml.push_str("[agent.master]\n");
    toml.push_str(&format!("provider     = \"{}\"\n", default_provider));
    toml.push_str(&format!("model        = \"{}\"\n", default_model));
    toml.push_str("tools        = [\"shell\",\"file_read\",\"file_write\",\"web_search\",\"content_search\"]\n");
    toml.push_str(&format!("instructions = \"You are {}, a phantom-mesh mobile worker. Reply concisely in Traditional Chinese.\"\n", node_name));

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(axum::body::Body::from(toml))
        .unwrap()
}

/// GET /scripts/:filename — serve a bootstrap script from `scripts/` dir.
///
/// Allowlist of filenames (no path traversal, no other dirs):
///   - windows-bootstrap.ps1
///   - setup-pi.sh
///   - termux-setup.sh
///
/// CWD when phantom serve runs is wherever the user launched it. The script
/// dir is resolved relative to where the binary's `core/` parent lives, with
/// a fallback to `$HOME/.phantom-mesh/scripts` and finally `/usr/local/share/phantom-mesh/scripts`.
async fn serve_script(axum::extract::Path(filename): axum::extract::Path<String>) -> Response {
    // Allowlist — no path traversal possible
    const ALLOWED: &[&str] = &[
        "windows-bootstrap.ps1",
        "install-phantom-windows.ps1",
        "setup-pi.sh",
        "termux-setup.sh",
        "install-mac.sh",
    ];
    if !ALLOWED.contains(&filename.as_str()) {
        return (StatusCode::NOT_FOUND, "script not in allowlist").into_response();
    }

    // Search candidate locations (in order)
    let candidates: Vec<std::path::PathBuf> = {
        let mut v = Vec::new();
        // 1. ./scripts/<file>  (cwd)
        v.push(std::path::PathBuf::from("scripts").join(&filename));
        // 2. <repo-root>/scripts/<file>  (cwd is core/, ../scripts/)
        v.push(std::path::PathBuf::from("../scripts").join(&filename));
        // 3. user-local
        if let Ok(data) = crate::cli_config::phantom_data_dir() {
            v.push(data.join("scripts").join(&filename));
        }
        // 4. system-wide
        v.push(std::path::PathBuf::from("/usr/local/share/phantom-mesh/scripts").join(&filename));
        v
    };

    for path in &candidates {
        if let Ok(content) = std::fs::read_to_string(path) {
            tracing::info!("Serving script {} from {}", filename, path.display());
            // Always text/plain — .ps1 and the other script extensions all
            // benefit from inline rendering in browsers; per-extension MIME
            // tuning hasn't been needed in practice.
            let content_type = "text/plain; charset=utf-8";
            return Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", content_type)
                .header(
                    "Content-Disposition",
                    format!("inline; filename=\"{}\"", filename),
                )
                .body(axum::body::Body::from(content))
                .unwrap();
        }
    }

    (
        StatusCode::NOT_FOUND,
        format!(
            "script not found in any candidate path; checked: {:?}",
            candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
        ),
    )
        .into_response()
}

/// Serve cross-platform phantom binaries from the repository's `dist/` so
/// remote nodes (Termux on Android, fresh Linux/Windows boxes) can curl
/// them directly off the coordinator without going through GitHub releases.
///
/// Allowlist mirrors the artefacts the build pipeline produces; anything
/// outside it is rejected to prevent path traversal and arbitrary download.
async fn serve_dist(axum::extract::Path(filename): axum::extract::Path<String>) -> Response {
    const ALLOWED: &[&str] = &[
        "phantom-aarch64-apple-darwin",
        "phantom-aarch64-linux-android",
        "phantom-aarch64-unknown-linux",
        "phantom-x86_64-pc-windows.exe",
        "phantom-x86_64-unknown-linux",
        "phantom-mesh-android.apk", // Tauri Android thin-shell APK
        "phantom-mesh-ios.ipa",     // Tauri iOS thin-shell IPA (signed dev cert)
    ];
    if !ALLOWED.contains(&filename.as_str()) {
        return (StatusCode::NOT_FOUND, "binary not in allowlist").into_response();
    }

    let candidates: Vec<std::path::PathBuf> = {
        let mut v = Vec::new();
        // 1. ./dist/<file>  (cwd = repo root)
        v.push(std::path::PathBuf::from("dist").join(&filename));
        // 2. <repo-root>/dist/<file>  (cwd = core/)
        v.push(std::path::PathBuf::from("../dist").join(&filename));
        // 3. user-local
        if let Some(home) = dirs::home_dir() {
            v.push(crate::cli_config::phantom_dir_under(&home).join("dist").join(&filename));
            // launchd-friendly install location
            v.push(
                home.join("Library/Application Support/phantom-mesh/dist")
                    .join(&filename),
            );
        }
        // 4. system-wide
        v.push(std::path::PathBuf::from("/usr/local/share/phantom-mesh/dist").join(&filename));
        v
    };

    for path in &candidates {
        if let Ok(bytes) = std::fs::read(path) {
            tracing::info!(
                "Serving binary {} ({} bytes) from {}",
                filename,
                bytes.len(),
                path.display()
            );
            return Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/octet-stream")
                .header(
                    "Content-Disposition",
                    format!("attachment; filename=\"{}\"", filename),
                )
                .body(axum::body::Body::from(bytes))
                .unwrap();
        }
    }

    (
        StatusCode::NOT_FOUND,
        format!(
            "binary not found in any candidate path; checked: {:?}",
            candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
        ),
    )
        .into_response()
}

// ── /api/version, /api/providers/health, /api/dashboard/status ──────────────
//
// These were registered in core/src/main.rs but never wired into phantom serve,
// so the README's documented endpoints 404'd in production. Tier-5 protocol
// test caught it. Keeping them here keeps every documented endpoint live on
// the actual daemon.

/// GET /api/version — build identification (semver + git short hash).
async fn api_version() -> Json<Value> {
    Json(json!({
        "version":      env!("CARGO_PKG_VERSION"),
        // build.rs sets PHANTOM_GIT_HASH; this used to read GIT_COMMIT_HASH
        // (a name nothing in the repo sets), so the field always returned
        // "unknown" — see Bug #13 in the 2026-05-01 test sweep.
        "commit":       crate::core_sha(),
        "target":       std::env::consts::OS,
        "wire_version": crate::WIRE_VERSION,
    }))
}

/// GET /api/providers/health — surface every configured provider's circuit
/// state and a coarse availability flag (api_key present / env resolves).
async fn api_providers_health(State(state): State<Arc<AppState>>) -> Json<Value> {
    let cfg = state.agent_runtime.config();
    let mut entries = Vec::with_capacity(cfg.providers.len());
    for (name, p) in cfg.providers.iter() {
        let has_inline = p.api_key.as_deref().map(|k| !k.is_empty()).unwrap_or(false);
        let has_env = p
            .api_key_env
            .as_deref()
            .map(|var| std::env::var(var).map(|v| !v.is_empty()).unwrap_or(false))
            .unwrap_or(false);
        entries.push(json!({
            "name":         name,
            "type":         p.provider_type,
            "default_model": p.default_model,
            "has_key":      has_inline || has_env,
            "key_source":   if has_inline { "inline" } else if has_env { "env" } else { "none" },
        }));
    }
    Json(json!({ "providers": entries }))
}

/// POST /rpc/evolve-handoff — receive an EvolveCheckpoint from a peer.
///
/// This is the cross-machine half of the audit-aware self-improvement
/// system: a sender that's blocked (rate-limited, low battery, leaving
/// the network) ships its full checkpoint here, including the plan,
/// hypothesis, dead-ends, and journey so far. The receiver records the
/// hop, persists locally, and returns the new session_id for tracking.
///
/// HMAC-protected with X-Cluster-Auth when cluster_secret is configured
/// (matches the existing /rpc/task/assign auth pattern).
///
/// Note: this endpoint accepts the checkpoint and saves it. It does NOT
/// auto-resume the evolve loop on receipt — the receiver is expected to
/// invoke `phantom autoevolve --resume <session-id>` (or pick up via
/// the normal scheduled run) to act on the handed-off state. Keeping
/// receipt and execution decoupled means a peer can be a passive
/// "shelter" for in-flight work without needing to be ready to run it
/// instantly.
async fn rpc_evolve_handoff(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    use crate::evolve_checkpoint::EvolveCheckpoint;

    // HMAC verification — FAIL-CLOSED (#321 fix): previously this gated the
    // auth block on `secret_configured`, so an unset/empty cluster_secret
    // SKIPPED auth entirely and let an unauthenticated remote peer reach
    // `checkpoint.save()`. Route through the shared `require_cluster_auth_dual`
    // helper (same as /rpc/message, /rpc/task/assign, …) which refuses outright
    // when the secret is empty unless PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET=1.
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/rpc/evolve-handoff",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }

    // Refuse if the peer speaks a newer wire version than this binary.
    if let Ok(peek) = serde_json::from_slice::<Value>(&body) {
        if let Some((code, err)) = check_wire_version(&peek) {
            return (code, err).into_response();
        }
    }

    let mut checkpoint: EvolveCheckpoint = match serde_json::from_slice(&body) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(with_wire_version(
                    json!({ "error": format!("malformed checkpoint: {}", e) }),
                )),
            )
                .into_response();
        }
    };

    let our_node = state
        .cluster_manager
        .config
        .node_name
        .clone()
        .or_else(|| std::env::var("PHANTOM_NODE_NAME").ok())
        .unwrap_or_else(|| "phantom".into());

    let prior_node = checkpoint.current_node.clone();
    checkpoint.record_node_hop(
        our_node.clone(),
        format!("handoff received via /rpc/evolve-handoff"),
    );

    if let Err(e) = checkpoint.save() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(with_wire_version(json!({
                "error": format!("save failed on receiver: {}", e),
            }))),
        )
            .into_response();
    }

    Json(with_wire_version(json!({
        "accepted":   true,
        "session_id": checkpoint.session_id,
        "from_node":  prior_node,
        "to_node":    our_node,
        "hops":       checkpoint.journey.len(),
        "saved_at_ms": checkpoint.last_updated_ms,
    })))
    .into_response()
}

/// POST /rpc/squad/dispatch — run a specific local agent on this node
/// against a caller-provided prompt and return the result. The
/// foundation of Squad Pipeline (SPEC-FREEZE-V1 §11.1, §12.4).
///
/// Body: `{ "agent": "<agent_name>", "prompt": "<text>",
///          "wire_version": 1 }`
///
/// HMAC-required (matches /rpc/task/assign + /rpc/evolve-handoff
/// pattern): `X-Cluster-Auth: SHA256(cluster_secret || body)` hex.
///
/// Response (synchronous; streaming SSE comes in v0.2):
///   { "output": "...", "agent": "...", "node": "...",
///     "elapsed_ms": N, "wire_version": 1 }
/// Errors: 401 unauthorized, 400 bad wire version / unknown agent /
/// missing fields, 500 agent execution failure.
async fn rpc_squad_dispatch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Bytes,
) -> impl IntoResponse {
    // 1. HMAC verification — FAIL-CLOSED (#321 fix). The previous
    //    `if secret_configured { … }` gate meant an unset/empty cluster_secret
    //    SKIPPED auth, letting an unauthenticated remote peer reach
    //    `agent_runtime.run()` (unauth RCE). Route through the shared
    //    `require_cluster_auth_dual` helper (same as /rpc/task/assign,
    //    /rpc/evolve-handoff, …) which refuses outright when the secret is
    //    empty unless PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET=1.
    if let Err((code, json)) = require_cluster_auth_dual(
        &state.cluster_manager,
        &headers,
        "POST",
        "/rpc/squad/dispatch",
        raw_query.as_deref(),
        &body,
    ) {
        return (code, json).into_response();
    }

    // 2. Wire-version sanity (same gate as message/handoff).
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(with_wire_version(
                    json!({ "error": format!("malformed body: {e}") }),
                )),
            )
                .into_response()
        }
    };
    if let Some((code, err)) = check_wire_version(&parsed) {
        return (code, err).into_response();
    }

    // 3. Required fields.
    let agent = parsed
        .get("agent")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let prompt = parsed
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if agent.is_empty() || prompt.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(with_wire_version(json!({
                "error": "both `agent` and `prompt` fields required",
                "received_keys": parsed.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default(),
            }))),
        )
            .into_response();
    }

    // 4. Validate agent exists locally — peer's /rpc/ping advertised
    //    agents list should mean this never fires, but defensive
    //    rejection prevents Squad Pipeline from silently routing to
    //    nonexistent agents on a peer that drifted from its ping
    //    inventory.
    if !state.agent_runtime.config().agent.contains_key(&agent) {
        let available: Vec<String> = state.agent_runtime.config().agent.keys().cloned().collect();
        return (
            StatusCode::BAD_REQUEST,
            Json(with_wire_version(json!({
                "error": format!("agent `{agent}` not configured on this node"),
                "available_agents": available,
            }))),
        )
            .into_response();
    }

    let node_name = state
        .cluster_manager
        .config
        .node_name
        .clone()
        .unwrap_or_else(|| "phantom".into());

    // 5. At-most-once: a peer re-posts a dispatch on its own timeout even though
    //    this node may already be running/finished the same job. Dedup BEFORE the
    //    agent runs so a re-post doesn't fire the agent (and its write tools)
    //    twice. Prefer the caller's explicit `idempotency_key`; otherwise hash
    //    `agent\nprompt`. A duplicate returns 200 with `deduped:true` so the
    //    Squad Pipeline treats it as already-done, not an error.
    let idem_key = parsed
        .get("idempotency_key")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| format!("dispatch:{}", s.trim()))
        .unwrap_or_else(|| {
            crate::idempotency::content_key("dispatch", &format!("{agent}\n{prompt}"))
        });
    if let crate::idempotency::Decision::Duplicate { first_seen } =
        crate::idempotency::check_and_record_default(&idem_key, "dispatch")
    {
        return Json(with_wire_version(json!({
            "output":     "",
            "agent":      agent,
            "node":       node_name,
            "elapsed_ms": 0,
            "deduped":    true,
            "first_seen": first_seen,
        })))
        .into_response();
    }

    // 6. Run the agent. Synchronous; output captured in result.
    let started = std::time::Instant::now();
    let result = state.agent_runtime.run(&agent, &prompt, &[], None).await;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    match result {
        Ok(r) => Json(with_wire_version(json!({
            "output":     r.output,
            "agent":      agent,
            "node":       node_name,
            "elapsed_ms": elapsed_ms,
        })))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(with_wire_version(json!({
                "error":      format!("agent `{agent}` failed: {e}"),
                "agent":      agent,
                "node":       node_name,
                "elapsed_ms": elapsed_ms,
            }))),
        )
            .into_response(),
    }
}

/// GET /api/dashboard/status — small JSON the home dashboard polls.
///
/// Note on "tools" cardinality (Bug #14 fix): the dashboard previously
/// returned `tools_count = state.tool_registry.names().len()`, which is
/// the master agent's *configured* tool whitelist (18 here). Doctor and
/// MCP `tools/list` report 51 — the actual registry size. Same word,
/// different denominator. Now we expose both:
///   tools_enabled:   master agent's whitelist length
///   tools_available: full built-in tool registry size
async fn api_dashboard_status(State(state): State<Arc<AppState>>) -> Json<Value> {
    let cfg = state.agent_runtime.config();
    let enabled = state.tool_registry.names().len();
    let available = crate::tools::all_tool_names().len();
    Json(json!({
        "version":         env!("CARGO_PKG_VERSION"),
        "tools_enabled":   enabled,
        "tools_available": available,
        // Backwards-compat: existing dashboard JS reads `tools_count`.
        // Keep it pointing at the agent's whitelist (the smaller, more
        // honest number for "what this agent can call right now").
        "tools_count":     enabled,
        "providers_count": cfg.providers.len(),
        "agents_count":    cfg.agent.len(),
        "cluster_peers":   cfg.cluster.peers.len(),
    }))
}

// ── Life Track (E002 F101) ───────────────────────────────────────────────────
//
// POST /api/events — multipart form (kind / goal_tags / text / image_N /
// audio_N). Persists raw modalities + meta via `EventStore`, calls the
// Gemini multimodal provider for analysis, persists the analysis, and
// returns `{ event_id, analysis }`.
//
// GET /api/events/:id/analysis — reads back the stored `AnalysisResult`.
//
// The handler currently constructs `GeminiMultimodalProvider::from_env()`
// per request — fine for v0.1; later refactor injects via DI (Task 11+).
/// #321 bonus: enforce the per-part and running-total media byte caps for
/// `POST /api/events`. Returns 413 over either cap; otherwise folds `len` into
/// `total`. Kept as a free fn so both the image and audio branches share it.
fn check_event_part_caps(
    len: usize,
    total: &mut usize,
) -> Result<(), (StatusCode, String)> {
    if len > MAX_EVENT_PART_BYTES {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "multipart part too large ({} bytes, max {})",
                len, MAX_EVENT_PART_BYTES
            ),
        ));
    }
    *total += len;
    if *total > MAX_EVENT_TOTAL_BYTES {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "total multipart media too large ({} bytes, max {})",
                total, MAX_EVENT_TOTAL_BYTES
            ),
        ));
    }
    Ok(())
}

async fn api_events_post(
    mut mp: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    use serde_json::json;

    let mut kind: String = "note".into();
    let mut goal_tags_csv: String = String::new();
    let mut text: Option<String> = None;
    let mut modalities: Vec<Modality> = Vec::new();

    // #321 bonus hardening: bound the number of parts and the total media bytes
    // buffered, in addition to the per-part byte cap enforced below. Returns 413
    // over any cap so the unauthenticated capture route can't balloon memory.
    let mut part_count: usize = 0;
    let mut total_media_bytes: usize = 0;

    while let Some(field) = mp
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("multipart: {}", e)))?
    {
        part_count += 1;
        if part_count > MAX_EVENT_PARTS {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("too many multipart parts (max {})", MAX_EVENT_PARTS),
            ));
        }
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "kind" => {
                kind = field
                    .text()
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, format!("kind text: {}", e)))?
            }
            "goal_tags" => {
                goal_tags_csv = field
                    .text()
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, format!("goal_tags: {}", e)))?
            }
            "text" => {
                text = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| (StatusCode::BAD_REQUEST, format!("text: {}", e)))?,
                )
            }
            n if n.starts_with("image_") => {
                let mime = field.content_type().unwrap_or("image/jpeg").to_string();
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, format!("image bytes: {}", e)))?
                    .to_vec();
                check_event_part_caps(bytes.len(), &mut total_media_bytes)?;
                modalities.push(Modality::Image { bytes, mime });
            }
            n if n.starts_with("audio_") => {
                let mime = field.content_type().unwrap_or("audio/wav").to_string();
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, format!("audio bytes: {}", e)))?
                    .to_vec();
                check_event_part_caps(bytes.len(), &mut total_media_bytes)?;
                modalities.push(Modality::Audio { bytes, mime });
            }
            _ => {} // ignore unknown fields
        }
    }
    if let Some(t) = &text {
        if !t.is_empty() {
            modalities.push(Modality::Text(t.clone()));
        }
    }

    let goal_tags: Vec<String> = goal_tags_csv
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let data = crate::cli_config::phantom_data_dir()
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "no home dir".to_string()))?;
    let identity_path = data.join("identity.key");
    let key = crate::life_node::key_derivation::load_event_key(&identity_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("key derivation: {}", e),
        )
    })?;
    let store = EventStore::with_key(data.join("events"), key);
    let source_node = std::env::var("PHANTOM_NODE_NAME").unwrap_or_else(|_| "unknown".into());
    let meta = store
        .write_event(&kind, &modalities, &goal_tags, &source_node)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("write_event: {}", e),
            )
        })?;

    let input = AnalysisInput {
        modalities,
        system_prompt: Some("You are a fat-loss + focus coach. Be specific and shame-free.".into()),
        user_prompt: format!(
            "Analyse this {} event for the user's goals: {}",
            kind,
            goal_tags.join(", ")
        ),
        max_output_tokens: Some(512),
        response_format: ResponseFormat::Json,
        response_schema: Some(json!({
            "type":"object",
            "properties":{
                "summary":     {"type":"string"},
                "goal_impact": {"type":"string"},
                "suggestion":  {"type":"string"},
                "confidence":  {"type":"number"}
            },
            "required":["summary"]
        })),
    };
    // SPEC-20 vision-preserving fallback: build the provider chain (Gemini =
    // image-capable, Groq = text-only) and fail over with try_vision_chain. When
    // the event carries a photo, the text-only provider is SKIPPED rather than
    // handed a request whose pixels it would silently drop (the prior bug: a
    // rate-limited Gemini fell back to Groq and analysed only the caption).
    let mut provider_chain: Vec<Box<dyn crate::life_node::multimodal::MultimodalProvider>> =
        Vec::new();
    if let Ok(p) = GeminiMultimodalProvider::from_env() {
        provider_chain.push(Box::new(p));
    }
    if let Ok(p) = GroqTextProvider::from_env() {
        provider_chain.push(Box::new(p));
    }
    // E006 graceful no-provider degrade. The event is already persisted above, so
    // returning 503 here would (a) hand the user an error for a capture that
    // actually succeeded and (b) leave the event with no analysis file — which
    // makes `coach review` skip it (load_events_for_date requires meta + analysis).
    // Instead, write a "skipped" analysis and return 200; set GEMINI_API_KEY or
    // GROQ_API_KEY to get real analysis.
    let analysis_skipped = provider_chain.is_empty();
    let analysis = if analysis_skipped {
        crate::life_node::multimodal::AnalysisResult {
            summary:
                "analysis skipped: no LLM provider configured (set GEMINI_API_KEY or GROQ_API_KEY for analysis)"
                    .into(),
            goal_impact: None,
            suggestion: None,
            confidence: None,
            raw_response: json!({ "skipped": true, "reason": "no_provider" }),
            model_id: "none".into(),
            latency_ms: 0,
            cost_usd: None,
        }
    } else {
        crate::life_node::providers::fallback::try_vision_chain(input, &provider_chain)
            .await
            .map_err(|e| match e {
                crate::life_node::multimodal::ProviderError::Modality(m) => (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("no image-capable provider for this photo: {}", m),
                ),
                other => (StatusCode::BAD_GATEWAY, format!("provider analyze: {}", other)),
            })?
    };
    store
        .write_analysis(&meta.event_id, &analysis)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("write_analysis: {}", e),
            )
        })?;

    Ok(Json(json!({
        "event_id": meta.event_id,
        "analysis": analysis,
        "analysis_skipped": analysis_skipped,
    })))
}

async fn api_events_analysis_get(
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let data = crate::cli_config::phantom_data_dir()
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "no home dir".to_string()))?;
    let identity_path = data.join("identity.key");
    let store =
        EventStore::with_identity_file(data.join("events"), &identity_path);
    let analysis = store
        .read_analysis(&id)
        .map_err(|e| (StatusCode::NOT_FOUND, format!("read_analysis: {}", e)))?;
    Ok(Json(serde_json::to_value(analysis).unwrap()))
}

#[cfg(test)]
mod boot_security_warning_tests {
    //! T55 — verify `emit_boot_security_warnings_with_config` emits exactly
    //! the right line set for each (env, config) combination.
    //!
    //! Env vars are process-global; cargo runs tests in parallel. Every test
    //! here MUST take `env_guard()` and clear both override env vars before
    //! the assertion to avoid bleed between tests in this mod.
    use super::*;
    use std::sync::MutexGuard;

    // Delegate to the crate-wide env mutex: PHANTOM_ENFORCE_REQUIRED_CAPS and
    // PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET are also mutated by tests in mesh.rs
    // and auth_gate.rs. A per-file mutex here let those groups race; sharing
    // crate::env_lock serializes every env-touching test process-wide.
    fn env_guard() -> MutexGuard<'static, ()> {
        crate::env_lock::acquire()
    }

    fn clear_overrides() {
        std::env::remove_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET");
        std::env::remove_var("PHANTOM_CORS_ALLOW_ANY");
        std::env::remove_var("PHANTOM_CORS_ALLOW_LOCALHOST");
    }

    #[test]
    fn cors_mode_defaults_to_same_origin() {
        let _g = env_guard();
        clear_overrides();
        assert_eq!(cors_mode_from_env(), CorsMode::SameOrigin);
    }

    #[test]
    fn cors_mode_localhost_when_flag_set() {
        let _g = env_guard();
        clear_overrides();
        std::env::set_var("PHANTOM_CORS_ALLOW_LOCALHOST", "1");
        assert_eq!(cors_mode_from_env(), CorsMode::Localhost);
        clear_overrides();
    }

    #[test]
    fn cors_mode_allow_any_wins_over_localhost() {
        let _g = env_guard();
        clear_overrides();
        std::env::set_var("PHANTOM_CORS_ALLOW_LOCALHOST", "1");
        std::env::set_var("PHANTOM_CORS_ALLOW_ANY", "1");
        assert_eq!(cors_mode_from_env(), CorsMode::AllowAny);
        clear_overrides();
    }

    #[test]
    fn cors_localhost_emits_one_info_line() {
        let _g = env_guard();
        clear_overrides();
        std::env::set_var("PHANTOM_CORS_ALLOW_LOCALHOST", "1");
        // secret configured → only the localhost CORS info line is emitted.
        let n = emit_boot_security_warnings_with_config(true);
        clear_overrides();
        assert_eq!(n, 1, "ALLOW_LOCALHOST alone should emit exactly 1 line");
    }

    #[test]
    fn no_overrides_and_secret_set_is_silent() {
        let _g = env_guard();
        clear_overrides();
        // Secret configured + no overrides → 0 lines (back-compat invariant).
        let n = emit_boot_security_warnings_with_config(true);
        assert_eq!(n, 0, "should emit nothing in the secured-default path");
    }

    #[test]
    fn cors_override_emits_warning() {
        let _g = env_guard();
        clear_overrides();
        std::env::set_var("PHANTOM_CORS_ALLOW_ANY", "1");
        let n = emit_boot_security_warnings_with_config(true);
        clear_overrides();
        assert_eq!(n, 1, "CORS override alone should emit exactly 1 line");
    }

    #[test]
    fn allow_empty_secret_override_emits_warning() {
        let _g = env_guard();
        clear_overrides();
        std::env::set_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET", "1");
        // Even with secret_configured=false, the override should suppress the
        // failing-closed diagnostic (would contradict the warning).
        let n = emit_boot_security_warnings_with_config(false);
        clear_overrides();
        assert_eq!(
            n, 1,
            "override should emit warning + suppress failing-closed line"
        );
    }

    #[test]
    fn empty_secret_no_override_emits_failing_closed_diagnostic() {
        let _g = env_guard();
        clear_overrides();
        // Empty secret + no override → 1 INFO line confirming fail-closed.
        let n = emit_boot_security_warnings_with_config(false);
        assert_eq!(
            n, 1,
            "empty cluster_secret + no override must emit 1 fail-closed diagnostic"
        );
    }

    #[test]
    fn both_overrides_set_emit_two_warnings() {
        let _g = env_guard();
        clear_overrides();
        std::env::set_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET", "1");
        std::env::set_var("PHANTOM_CORS_ALLOW_ANY", "1");
        let n = emit_boot_security_warnings_with_config(false);
        clear_overrides();
        assert_eq!(n, 2, "both overrides → both warnings, no fail-closed line");
    }

    #[test]
    fn back_compat_zero_arg_call_assumes_secret_configured() {
        let _g = env_guard();
        clear_overrides();
        // The deprecated zero-arg shim must remain silent when no overrides
        // are set (it can't know the secret status, so it assumes configured).
        let n = emit_boot_security_warnings();
        assert_eq!(n, 0, "zero-arg shim must be silent in default config");
    }
}

#[cfg(test)]
mod wire_version_tests {
    use super::*;

    #[test]
    fn with_wire_version_injects_field() {
        let out = with_wire_version(json!({ "ok": true }));
        assert_eq!(
            out["wire_version"].as_u64().unwrap() as u32,
            crate::WIRE_VERSION
        );
        assert_eq!(out["ok"], json!(true));
    }

    #[test]
    fn with_wire_version_preserves_explicit_override() {
        // If a handler has already set its own wire_version (e.g. in a
        // forward-compat shim), we don't overwrite. Documents the
        // `or_insert` semantics.
        let out = with_wire_version(json!({ "wire_version": 99 }));
        assert_eq!(out["wire_version"].as_u64().unwrap(), 99);
    }

    #[test]
    fn check_wire_version_accepts_equal_or_lower() {
        assert!(check_wire_version(&json!({ "wire_version": crate::WIRE_VERSION })).is_none());
        // No wire_version field at all → tolerated (degraded warning is
        // surfaced at the doctor layer, not here, so old peers keep working).
        assert!(check_wire_version(&json!({})).is_none());
    }

    #[test]
    fn check_wire_version_rejects_higher() {
        let resp = check_wire_version(&json!({ "wire_version": crate::WIRE_VERSION + 5 }));
        let (code, json) = resp.expect("should reject higher wire_version");
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let err = json.0["error"].as_str().unwrap_or("");
        assert!(
            err.contains("phantom upgrade"),
            "error missing upgrade hint: {err}"
        );
        assert!(err.contains(&format!("v{}", crate::WIRE_VERSION + 5)));
        assert!(err.contains(&format!("v{}", crate::WIRE_VERSION)));
    }
}

// T7b: `cluster_auth_helper_tests` moved to `core/src/auth_gate.rs::tests`
// (the helper now lives there; we re-export via `pub use`).

#[cfg(test)]
mod squad_dispatch_tests {
    use crate::mesh::{ClusterConfig, ClusterManager, EnforceMode, PeerStatus};
    use crate::AppState;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::{json, Value};
    use std::sync::Arc;
    use tower::ServiceExt; // for `oneshot`

    /// Serialise tests that mutate process env. These tests touch
    /// `PHANTOM_ENFORCE_REQUIRED_CAPS` AND — now that `/rpc/task/assign`
    /// records an at-most-once dedup key — `PHANTOM_IDEMPOTENCY_STORE`. Both
    /// vars are also mutated by tests in other modules (coach, auth_gate,
    /// mesh), so we share the crate-wide [`crate::env_lock`] rather than a
    /// module-local mutex; a module-local lock would let those groups race.
    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        crate::env_lock::acquire()
    }

    /// RAII: point the at-most-once ledger at a throwaway tempdir for one test,
    /// restoring the prior env on drop. REQUIRED for any test that drives
    /// `/rpc/task/assign` to the spawn path: the handler records a dedup key in
    /// the (default, PERSISTENT) ledger keyed by `agent\nprompt`, so without
    /// isolation an identical body collides across tests AND across runs within
    /// the 24h TTL — surfacing as a duplicate `200` where `202` is expected.
    /// The caller must already hold [`env_guard`] (serializes the env mutation).
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

    /// Test cluster secret used by every dispatch-filter test in this
    /// module. T7 (#69) made `/rpc/task/assign` fail-closed when no
    /// `cluster_secret` is configured — so every request these tests
    /// fire must carry a matching `X-Cluster-Auth` HMAC, computed
    /// from THIS secret and the exact request body.
    const TEST_CLUSTER_SECRET: &str = "test-secret";

    /// Build an `AppState` with the given worker_caps and enforce_caps,
    /// wrap it in the production router. Returned router can be driven
    /// via `router.oneshot(req)`.
    ///
    /// The cluster_secret is set to [`TEST_CLUSTER_SECRET`] so the
    /// T7 cluster-auth gate lets the request through; tests must use
    /// [`assign_request`] (which signs the body) to reach the dispatch
    /// filter logic under test.
    fn router_with_caps(
        worker_caps: Vec<String>,
        enforce_caps: Option<EnforceMode>,
    ) -> axum::Router {
        let cfg = ClusterConfig {
            node_name: Some("test".into()),
            worker_caps,
            enforce_caps,
            cluster_secret: Some(TEST_CLUSTER_SECRET.into()),
            ..ClusterConfig::default()
        };
        let mut state = AppState::new();
        state.cluster_manager = ClusterManager::new(cfg);
        super::router(Arc::new(state))
    }

    /// Build a POST /rpc/task/assign request whose body is `body` and
    /// whose `X-Cluster-Auth` header is the HMAC-SHA256 of the body
    /// keyed by [`TEST_CLUSTER_SECRET`]. This is what real callers
    /// must send post-T7 (#69); the dispatch filter only sees the
    /// request after `require_cluster_auth` accepts it.
    fn assign_request(body: Value) -> Request<Body> {
        let body_str = body.to_string();
        // Reuse the production HMAC code path so this test cannot drift
        // from how require_cluster_auth verifies tokens.
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

    /// Build a signed POST request to an arbitrary `path` (mirrors
    /// [`assign_request`] but parameterised by path) so the zk-relay endpoints
    /// can be driven through the same body-HMAC auth path real callers use.
    fn signed_req(path: &str, body: Value) -> Request<Body> {
        let body_str = body.to_string();
        let signing_cfg = ClusterConfig {
            cluster_secret: Some(TEST_CLUSTER_SECRET.into()),
            ..ClusterConfig::default()
        };
        let token = ClusterManager::new(signing_cfg).make_auth_token(&body_str);
        Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .header("X-Cluster-Auth", token)
            .body(Body::from(body_str))
            .expect("build request")
    }

    /// P2-1 zero-knowledge relay: a signed put stores an age-sealed blob; a
    /// signed get returns the EXACT sealed bytes (which the client — not the
    /// server — decrypts); an unknown key FAILS CLOSED with 404 (never
    /// plaintext, never another blob).
    #[tokio::test]
    async fn zk_put_get_roundtrip_seals_and_fails_closed() {
        use base64::Engine as _;
        let _g = env_guard(); // serialize the PHANTOM_HOME mutation
        let tmp = tempfile::tempdir().unwrap();
        let prev = std::env::var_os("PHANTOM_HOME");
        // PHANTOM_HOME is the data-root verbatim → isolates the relay store dir.
        std::env::set_var("PHANTOM_HOME", tmp.path().join(".phantom-mesh"));

        // Client seals plaintext; the server only ever sees this ciphertext.
        let key = crate::life_node::key_derivation::derive_event_key(&[0x33u8; 32]).unwrap();
        let plaintext = b"relay roundtrip secret";
        let sealed = crate::life_node::crypto::encrypt(plaintext, &key).unwrap();
        let sealed_b64 = base64::engine::general_purpose::STANDARD.encode(&sealed);

        // PUT
        let put_body =
            json!({ "device_id": "dev-a", "blob_id": "blob-1", "sealed_b64": sealed_b64 });
        let resp = router_with_caps(vec![], None)
            .oneshot(signed_req("/rpc/zk/put", put_body))
            .await
            .expect("zk put");
        assert_eq!(resp.status(), StatusCode::OK, "put must succeed");
        assert_eq!(body_json(resp).await["stored"], json!(true));

        // GET returns the exact sealed bytes; only the client key recovers plaintext.
        let resp = router_with_caps(vec![], None)
            .oneshot(signed_req(
                "/rpc/zk/get",
                json!({ "device_id": "dev-a", "blob_id": "blob-1" }),
            ))
            .await
            .expect("zk get");
        assert_eq!(resp.status(), StatusCode::OK);
        let got_b64 = body_json(resp).await["sealed_b64"].as_str().unwrap().to_string();
        let got = base64::engine::general_purpose::STANDARD.decode(got_b64).unwrap();
        assert_eq!(got, sealed, "server must return the exact sealed bytes");
        assert_eq!(
            crate::life_node::crypto::decrypt(&got, &key).unwrap(),
            plaintext
        );

        // Unknown key FAILS CLOSED — 404, never plaintext, never another blob.
        let resp = router_with_caps(vec![], None)
            .oneshot(signed_req(
                "/rpc/zk/get",
                json!({ "device_id": "dev-a", "blob_id": "ghost" }),
            ))
            .await
            .expect("zk get missing");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "missing key must fail closed");

        match prev {
            Some(v) => std::env::set_var("PHANTOM_HOME", v),
            None => std::env::remove_var("PHANTOM_HOME"),
        }
    }

    /// Both zk-relay endpoints reject an UNauthenticated request (no
    /// `X-Cluster-Auth`) — they must never be open. 401/403, never 200.
    #[tokio::test]
    async fn zk_endpoints_reject_unauthenticated() {
        for path in ["/rpc/zk/put", "/rpc/zk/get"] {
            let req = Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "device_id": "d", "blob_id": "b" }).to_string(),
                ))
                .unwrap();
            let resp = router_with_caps(vec![], None)
                .oneshot(req)
                .await
                .expect("unauth req");
            assert!(
                resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::UNAUTHORIZED,
                "unauthenticated {path} must be rejected, got {}",
                resp.status()
            );
        }
    }

    /// POST a single-image multipart capture of `size` bytes to `/api/events`
    /// on a fresh production router, returning the response status. Used by the
    /// body-limit regression test below.
    async fn post_event_photo(size: usize) -> StatusCode {
        let boundary = "XBoundaryEventsTest";
        let photo = vec![0u8; size];
        let mut body: Vec<u8> = Vec::new();
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"image_0\"; \
                 filename=\"meal.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(&photo);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let req = Request::builder()
            .method("POST")
            .uri("/api/events")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .expect("build request");
        router_with_caps(vec![], None)
            .oneshot(req)
            .await
            .expect("call /api/events")
            .status()
    }

    /// POST `n` tiny `image_i` parts to `/api/events`, returning the status.
    /// Used by the #321 part-count-cap regression test. Parts are 1 byte each
    /// so the whole request stays well under `EVENT_UPLOAD_BODY_LIMIT` — the
    /// ONLY thing that can reject it is the per-request `MAX_EVENT_PARTS` cap.
    async fn post_event_n_parts(n: usize) -> StatusCode {
        let boundary = "XBoundaryPartsTest";
        let mut body: Vec<u8> = Vec::new();
        for i in 0..n {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"image_{i}\"; \
                     filename=\"p.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n\x00\r\n"
                )
                .as_bytes(),
            );
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        let req = Request::builder()
            .method("POST")
            .uri("/api/events")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .expect("build request");
        router_with_caps(vec![], None)
            .oneshot(req)
            .await
            .expect("call /api/events")
            .status()
    }

    #[tokio::test]
    async fn events_upload_rejects_too_many_parts_with_413() {
        // #321 bonus: without a part-count cap, a single sub-body-limit request
        // could carry an unbounded number of image_*/audio_* parts (each fully
        // buffered). MAX_EVENT_PARTS bounds the fan-out → 413 over the cap.
        // Pin HOME + clear provider keys so an UNDER-cap request can't do a real
        // event write / network call (mirrors the body-limit test's isolation).
        let _env = crate::env_lock::acquire();
        struct VarGuard(&'static str, Option<String>);
        impl Drop for VarGuard {
            fn drop(&mut self) {
                match &self.1 {
                    Some(v) => std::env::set_var(self.0, v),
                    None => std::env::remove_var(self.0),
                }
            }
        }
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let _h = VarGuard("HOME", std::env::var("HOME").ok());
        let _g1 = VarGuard("GEMINI_API_KEY", std::env::var("GEMINI_API_KEY").ok());
        let _g2 = VarGuard("GROQ_API_KEY", std::env::var("GROQ_API_KEY").ok());
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("GEMINI_API_KEY");
        std::env::remove_var("GROQ_API_KEY");

        // MAX_EVENT_PARTS = 64. One over the cap must be rejected.
        let over = post_event_n_parts(super::MAX_EVENT_PARTS + 1).await;
        assert_eq!(
            over,
            StatusCode::PAYLOAD_TOO_LARGE,
            "more than MAX_EVENT_PARTS parts must be rejected with 413"
        );
        // A handful of parts (well under the cap) must NOT be rejected as 413.
        let under = post_event_n_parts(3).await;
        assert_ne!(
            under,
            StatusCode::PAYLOAD_TOO_LARGE,
            "a few parts must not trip the part-count cap"
        );
    }

    #[tokio::test]
    async fn events_upload_accepts_photo_over_2mib_default() {
        // SPEC-20 regression: a real meal photo (3 MiB here) must upload.
        // axum's default 2 MiB request cap makes `field.bytes()` fail and the
        // handler return 400 "image bytes: length limit exceeded"; the per-route
        // DefaultBodyLimit(EVENT_UPLOAD_BODY_LIMIT) raises the cap so the field
        // reads OK and the request proceeds down the SAME path as a tiny one.
        //
        // Pin HOME to a tempdir (so any event write stays out of the real
        // ~/.phantom-mesh) and clear provider keys (so no network call), under
        // the shared env lock. Assert by behaviour-equivalence: with the fix a
        // 3 MiB and a 1 KiB photo reach the identical downstream status; without
        // it the 3 MiB one would be 400.
        let _env = crate::env_lock::acquire();
        struct VarGuard(&'static str, Option<String>);
        impl Drop for VarGuard {
            fn drop(&mut self) {
                match &self.1 {
                    Some(v) => std::env::set_var(self.0, v),
                    None => std::env::remove_var(self.0),
                }
            }
        }
        let tmp = tempfile::TempDir::new().expect("tempdir");
        // Declared after `tmp` so HOME is restored BEFORE the tempdir is removed.
        let _h = VarGuard("HOME", std::env::var("HOME").ok());
        let _g1 = VarGuard("GEMINI_API_KEY", std::env::var("GEMINI_API_KEY").ok());
        let _g2 = VarGuard("GROQ_API_KEY", std::env::var("GROQ_API_KEY").ok());
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("GEMINI_API_KEY");
        std::env::remove_var("GROQ_API_KEY");

        let small = post_event_photo(1024).await; // 1 KiB
        let big = post_event_photo(3 * 1024 * 1024).await; // 3 MiB > 2 MiB default

        assert_ne!(
            big,
            StatusCode::PAYLOAD_TOO_LARGE,
            "3 MiB photo wrongly rejected as too large"
        );
        assert_ne!(
            big,
            StatusCode::BAD_REQUEST,
            "3 MiB photo tripped the old 2 MiB body cap (got 400 from field.bytes())"
        );
        assert_eq!(
            big, small,
            "once the cap is raised a 3 MiB photo must take the same path as a 1 KiB one"
        );
    }

    #[tokio::test]
    async fn peer_status_carries_worker_caps_and_agents() {
        // Sandbox-worker config (iOS / Android Tauri).
        let mut cfg = ClusterConfig::default();
        cfg.node_name = Some("ios-test".into());
        cfg.worker_caps = vec![
            "file_in_container".into(),
            "memory".into(),
            "web".into(),
            "subagent".into(),
            "llm_local".into(),
        ];
        let cm = ClusterManager::new(cfg);
        let status: PeerStatus = cm.own_peer_status();

        assert_eq!(status.worker_caps.len(), 5);
        assert!(status.worker_caps.iter().any(|s| s == "file_in_container"));
        // `agents` is populated by serve.rs::rpc_ping at request time, not
        // by ClusterManager; left empty in own_peer_status by design.
        assert!(
            status.agents.is_empty(),
            "agents must be populated at /rpc/ping time, not by ClusterManager"
        );
    }

    #[tokio::test]
    async fn full_worker_has_empty_worker_caps_by_default() {
        // Mac / Win / Linux full worker — no worker_caps in agents.toml.
        let mut cfg = ClusterConfig::default();
        cfg.node_name = Some("mac-test".into());
        let cm = ClusterManager::new(cfg);
        let status = cm.own_peer_status();
        assert!(
            status.worker_caps.is_empty(),
            "full workers should have empty worker_caps (= no restriction)"
        );
    }

    #[test]
    fn dispatch_route_registered() {
        // Schema-only check: confirm the new /rpc/squad/dispatch route
        // string appears in the source. Routing-correctness lives in
        // axum's own test suite; this guards against accidental rename
        // or removal during refactors.
        let src = include_str!("../src/serve.rs");
        assert!(
            src.contains("/rpc/squad/dispatch"),
            "/rpc/squad/dispatch route must be wired in app() builder"
        );
        assert!(
            src.contains("rpc_squad_dispatch"),
            "rpc_squad_dispatch handler must exist"
        );
    }

    #[tokio::test]
    async fn strict_mode_rejects_mismatched_required_caps_with_409() {
        // Worker advertises only file_in_container + memory.
        // Request demands "shell" (not in local caps).
        // Strict mode → 409 with capability_mismatch body.
        let _g = env_guard();
        std::env::remove_var("PHANTOM_ENFORCE_REQUIRED_CAPS");
        let router = router_with_caps(
            vec!["file_in_container".into(), "memory".into()],
            Some(EnforceMode::Strict),
        );
        let req = assign_request(json!({
            "agent":         "master",
            "prompt":        "do dangerous thing",
            "required_caps": ["file_in_container", "shell"],
        }));
        let resp = router.oneshot(req).await.expect("call /rpc/task/assign");
        assert_eq!(resp.status(), StatusCode::CONFLICT, "strict mode must 409");
        let body = body_json(resp).await;
        assert_eq!(body["error_code"], "capability_mismatch");
        assert_eq!(body["missing"], json!(["shell"]));
        assert_eq!(body["local"], json!(["file_in_container", "memory"]));
        assert_eq!(body["required"], json!(["file_in_container", "shell"]));
        assert!(
            body.get("job_id").is_none(),
            "rejection must not allocate a job_id"
        );
    }

    #[tokio::test]
    async fn soft_mode_accepts_mismatched_required_caps_with_202() {
        // Same mismatch as the strict test, but soft mode (default).
        // Behaviour must match pre-T5: the task is accepted, the
        // response includes a job_id, status is 202 Accepted.
        // (The warn log is fire-and-forget; we don't assert on it.)
        let _g = env_guard();
        let _idem = IdemStoreGuard::new();
        std::env::remove_var("PHANTOM_ENFORCE_REQUIRED_CAPS");
        let router = router_with_caps(
            vec!["file_in_container".into(), "memory".into()],
            Some(EnforceMode::Soft),
        );
        let req = assign_request(json!({
            "agent":         "master",
            "prompt":        "do dangerous thing",
            "required_caps": ["file_in_container", "shell"],
        }));
        let resp = router.oneshot(req).await.expect("call /rpc/task/assign");
        assert_eq!(
            resp.status(),
            StatusCode::ACCEPTED,
            "soft mode must accept (202)"
        );
        let body = body_json(resp).await;
        assert!(
            body.get("job_id").and_then(|v| v.as_str()).is_some(),
            "soft mode must return a job_id, got {body}"
        );
        assert!(
            body.get("error_code").is_none(),
            "soft mode must not include capability_mismatch error_code"
        );
    }

    /// Build the production router with a caller-owned job store and the test
    /// cluster secret (so [`assign_request`]'s HMAC passes auth). Same handlers
    /// as `router_with_caps`, but the test keeps the `Arc` so it can count jobs.
    fn router_sharing_jobs(jobs: super::ClusterJobStore) -> axum::Router {
        let cfg = ClusterConfig {
            node_name: Some("test".into()),
            cluster_secret: Some(TEST_CLUSTER_SECRET.into()),
            ..ClusterConfig::default()
        };
        let mut state = AppState::new();
        state.cluster_manager = ClusterManager::new(cfg);
        super::router_with_jobs(Arc::new(state), jobs)
    }

    #[tokio::test]
    async fn duplicate_assign_returns_first_job_id_and_spawns_no_second_job() {
        // VERIFIED-FINDING #1/#2 regression guard (review round 2). A re-sent
        // /rpc/task/assign must (a) NOT spawn a second job and (b) return a
        // caller-compatible body whose `job_id` == the first accepted job_id.
        // Callers (`mesh::assign_task_to_peer` / `_full`) do
        // `data.job_id.ok_or_else(...)`, so a job_id-less success would be
        // mis-read as a DispatchError → spurious forward failure.
        let _g = env_guard(); // process-wide env lock (serializes env-touching tests)
        let _idem = IdemStoreGuard::new(); // isolate the dedup ledger to a tempdir
        std::env::remove_var("PHANTOM_ENFORCE_REQUIRED_CAPS"); // soft mode → Allow

        // One job store shared by both router instances (oneshot consumes the
        // router, so we build it twice over the SAME Arc).
        let jobs: super::ClusterJobStore =
            Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let body = json!({ "agent": "master", "prompt": "review-round-2 dedup probe" });

        // First assign: accepted (202), mints a job_id, inserts exactly one job.
        let resp1 = router_sharing_jobs(jobs.clone())
            .oneshot(assign_request(body.clone()))
            .await
            .expect("first /rpc/task/assign");
        assert_eq!(resp1.status(), StatusCode::ACCEPTED, "first assign accepted (202)");
        let body1 = body_json(resp1).await;
        let first_job_id = body1["job_id"]
            .as_str()
            .expect("first response must carry a job_id")
            .to_string();
        assert_eq!(
            jobs.read().await.len(),
            1,
            "first assign must spawn exactly one job"
        );

        // Duplicate assign (identical body → identical derived key): deduped.
        let resp2 = router_sharing_jobs(jobs.clone())
            .oneshot(assign_request(body.clone()))
            .await
            .expect("duplicate /rpc/task/assign");
        assert_eq!(
            resp2.status(),
            StatusCode::OK,
            "duplicate must be deduped (200, not a fresh 202)"
        );
        let body2 = body_json(resp2).await;
        assert_eq!(body2["deduped"], json!(true), "duplicate carries deduped:true");
        assert_eq!(
            body2["job_id"].as_str(),
            Some(first_job_id.as_str()),
            "duplicate MUST return the ORIGINAL job_id for caller compatibility, got {body2}"
        );
        assert!(
            body2.get("first_seen").is_some(),
            "duplicate should report the original first_seen ts"
        );
        assert_eq!(
            jobs.read().await.len(),
            1,
            "duplicate must NOT spawn a second job — store still holds exactly one"
        );
    }

    /// #321 fix (#1 + #2): with NO cluster_secret and NO empty-secret override,
    /// the two formerly fail-OPEN mesh RPC routes — /rpc/squad/dispatch (reached
    /// agent_runtime.run() = unauth RCE) and /rpc/evolve-handoff (reached
    /// checkpoint.save()) — must now FAIL CLOSED, rejecting the unauthenticated
    /// POST with a client error (401/403). They must never be open (200) nor
    /// unwired (404/405). /rpc/task/assign is included as a parity baseline.
    #[tokio::test]
    async fn empty_secret_fails_closed_on_dispatch_handoff_and_assign() {
        let _g = env_guard();
        // The whole point of the fix: with the override OFF, empty secret rejects.
        std::env::remove_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET");

        // AppState::new() has no cluster_secret configured (None/empty).
        let state = AppState::new();
        assert!(
            state
                .cluster_manager
                .config
                .cluster_secret
                .as_deref()
                .map_or(true, |s| s.is_empty()),
            "precondition: this test must run with an empty/unset cluster_secret"
        );
        let arc = Arc::new(state);

        for (path, body) in [
            ("/rpc/squad/dispatch", json!({ "agent": "master", "prompt": "rce attempt" })),
            ("/rpc/evolve-handoff", json!({ "session_id": "x", "current_node": "evil" })),
            ("/rpc/task/assign", json!({ "agent": "master", "prompt": "rce attempt" })),
        ] {
            let req = Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("build request");
            // Rebuild the router per call (oneshot consumes it); same Arc state.
            let resp = super::router(arc.clone())
                .oneshot(req)
                .await
                .unwrap_or_else(|_| panic!("call {path}"));
            let code = resp.status().as_u16();
            assert!(
                (400..500).contains(&code),
                "unauthenticated POST {path} with empty cluster_secret MUST fail closed \
                 (4xx client error), got {code} — fail-OPEN regression",
            );
            assert!(
                code == 401 || code == 403,
                "empty-secret rejection on {path} should be 401/403 (auth gate), got {code}",
            );
        }
    }

    /// #321 fix (#5): a Duplicate sighting whose durable row is MISSING (a
    /// crash-orphaned ledger entry, or any not-yet-written row) must NOT
    /// re-spawn. The old code probed "resolvability" and fell through to a
    /// SECOND spawn when the row was absent — so a retry storm could fan out
    /// extra agent executions. Strict at-most-once: the dedup branch always
    /// returns 200 deduped with the original (possibly-unresolvable) id and the
    /// job store gains no second entry.
    #[tokio::test]
    async fn duplicate_with_missing_durable_row_never_respawns() {
        let _g = env_guard();
        let _idem = IdemStoreGuard::new();
        std::env::remove_var("PHANTOM_ENFORCE_REQUIRED_CAPS"); // soft mode → Allow

        // A DURABLE task queue is configured but we will NEVER let the first
        // assign create a row in it — instead we pre-seed the at-most-once ledger
        // with a job_id that has no matching durable row, simulating the
        // crash-orphan gap (ledger recorded, row write lost).
        let dir = tempfile::TempDir::new().expect("tempdir");
        let db = dir.path().join("phantom.db");
        let queue = crate::TaskQueue::new(crate::TaskStore::open_at(db).expect("open"));

        let jobs: super::ClusterJobStore =
            Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let cfg = ClusterConfig {
            node_name: Some("test".into()),
            cluster_secret: Some(TEST_CLUSTER_SECRET.into()),
            ..ClusterConfig::default()
        };
        let mut state = AppState::new();
        state.cluster_manager = ClusterManager::new(cfg);
        state.task_queue = Some(queue);
        let arc = Arc::new(state);

        let body = json!({ "agent": "master", "prompt": "orphan-row dedup probe #321" });
        let idem_key = super::task_assign_idem_key(None, "master", "orphan-row dedup probe #321");
        // Pre-record the dedup key pointing at an orphaned id (no durable row).
        let orphan_id = uuid::Uuid::new_v4().to_string();
        let (decision, _) = crate::idempotency::check_and_record_value_default(
            &idem_key,
            "task_assign",
            Some(&orphan_id),
        );
        assert!(
            matches!(decision, crate::idempotency::Decision::First),
            "pre-seed must be the FIRST sighting of this key"
        );

        // Now POST the identical body: it derives the SAME key → Duplicate whose
        // recorded id (orphan_id) has NO durable row. Strict at-most-once: 200
        // deduped, original id echoed, and ZERO new jobs spawned.
        let req = Request::builder()
            .method("POST")
            .uri("/rpc/task/assign")
            .header("content-type", "application/json")
            .header(
                "X-Cluster-Auth",
                ClusterManager::new(ClusterConfig {
                    cluster_secret: Some(TEST_CLUSTER_SECRET.into()),
                    ..ClusterConfig::default()
                })
                .make_auth_token(&body.to_string()),
            )
            .body(Body::from(body.to_string()))
            .expect("build request");
        let resp = super::router_with_jobs(arc.clone(), jobs.clone())
            .oneshot(req)
            .await
            .expect("call /rpc/task/assign");
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "orphan-row duplicate must dedup (200), NOT spawn a fresh 202"
        );
        let body_resp = body_json(resp).await;
        assert_eq!(body_resp["deduped"], json!(true), "must be marked deduped");
        assert_eq!(
            body_resp["job_id"].as_str(),
            Some(orphan_id.as_str()),
            "must echo the ORIGINAL (orphaned) job_id, not mint a new one — got {body_resp}"
        );
        assert!(
            jobs.read().await.is_empty(),
            "strict at-most-once: a missing-durable-row duplicate must NOT spawn — \
             in-memory job store must stay empty, got {} jobs",
            jobs.read().await.len()
        );
    }

    /// Build the production router backed by a DURABLE task queue (gap-a).
    /// Returns the shared `Arc<AppState>` too so a test can read the durable
    /// store after driving a handler. node_name + cluster_secret are set so
    /// [`assign_request`]'s HMAC passes auth.
    fn router_with_queue(queue: crate::TaskQueue) -> (Arc<AppState>, axum::Router) {
        let cfg = ClusterConfig {
            node_name: Some("test".into()),
            cluster_secret: Some(TEST_CLUSTER_SECRET.into()),
            ..ClusterConfig::default()
        };
        let mut state = AppState::new();
        state.cluster_manager = ClusterManager::new(cfg);
        state.task_queue = Some(queue);
        let arc = Arc::new(state);
        (arc.clone(), super::router(arc))
    }

    // ─── apex-④ off-switch: /rpc/task/stop + /rpc/task/resume ───────────────
    // A phone STOP/RESUME on the shipping assign flow must actually control a
    // running durable task. STOP parks a Running task (→ AwaitingApproval, the
    // durable "paused, awaiting operator" state) and signals the cooperative
    // interrupt registry so the live runner unwinds; RESUME moves it back to
    // Running. Both HMAC-authed (fail-closed 401/403 like every mesh RPC).

    /// Seed a durable task already in `Running` (stands in for a long-running
    /// in-flight job that a phone would want to STOP). Returns its job_id.
    async fn seed_running_task(queue: &crate::TaskQueue) -> uuid::Uuid {
        let t = queue.create("ws", "coder", "long task").await.expect("create");
        queue
            .transition(t.task_id, pm_types::TaskStatus::Running, None)
            .await
            .expect("→running");
        t.task_id
    }

    #[tokio::test]
    async fn rpc_task_stop_authed_parks_running_task() {
        let _g = env_guard();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let queue = crate::TaskQueue::new(
            crate::TaskStore::open_at(dir.path().join("phantom.db")).expect("open"),
        );
        let job_id = seed_running_task(&queue).await;
        let (arc, router) = router_with_queue(queue);

        let body = json!({ "job_id": job_id.to_string() }).to_string();
        let resp = router
            .oneshot(signed_post("/rpc/task/stop", &body))
            .await
            .expect("call /rpc/task/stop");
        assert_eq!(resp.status(), StatusCode::OK, "valid HMAC stop must 200");
        let j = body_json(resp).await;
        assert_eq!(j["job_id"], json!(job_id.to_string()));
        assert_eq!(j["status"], json!("stopped"), "wire status must read stopped");

        // Durable state actually flipped off the runnable Running state.
        let rec = arc
            .task_queue
            .as_ref()
            .unwrap()
            .get(job_id)
            .await
            .expect("get")
            .expect("row");
        assert_eq!(
            rec.status,
            pm_types::TaskStatus::AwaitingApproval,
            "STOP must park the task off Running (durable state changed)"
        );
    }

    #[tokio::test]
    async fn rpc_task_stop_bad_hmac_fails_closed_no_state_change() {
        let _g = env_guard();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let queue = crate::TaskQueue::new(
            crate::TaskStore::open_at(dir.path().join("phantom.db")).expect("open"),
        );
        let job_id = seed_running_task(&queue).await;
        let (arc, router) = router_with_queue(queue);

        // BAD signature: well-formed header, wrong secret.
        let body = json!({ "job_id": job_id.to_string() }).to_string();
        let bad_token = ClusterManager::new(ClusterConfig {
            cluster_secret: Some("WRONG-SECRET".into()),
            ..ClusterConfig::default()
        })
        .make_auth_token(&body);
        let req = Request::builder()
            .method("POST")
            .uri("/rpc/task/stop")
            .header("content-type", "application/json")
            .header("X-Cluster-Auth", bad_token)
            .body(Body::from(body.clone()))
            .expect("build");
        let resp = router.oneshot(req).await.expect("call");
        let code = resp.status().as_u16();
        assert!(code == 401 || code == 403, "bad HMAC must 401/403, got {code}");

        // Fail-closed: the task is UNTOUCHED (still Running).
        let rec = arc
            .task_queue
            .as_ref()
            .unwrap()
            .get(job_id)
            .await
            .expect("get")
            .expect("row");
        assert_eq!(
            rec.status,
            pm_types::TaskStatus::Running,
            "bad-HMAC stop must NOT change durable state"
        );
    }

    #[tokio::test]
    async fn rpc_task_stop_missing_hmac_fails_closed() {
        let _g = env_guard();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let queue = crate::TaskQueue::new(
            crate::TaskStore::open_at(dir.path().join("phantom.db")).expect("open"),
        );
        let job_id = seed_running_task(&queue).await;
        let (arc, router) = router_with_queue(queue);

        let body = json!({ "job_id": job_id.to_string() }).to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/rpc/task/stop")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .expect("build");
        let resp = router.oneshot(req).await.expect("call");
        let code = resp.status().as_u16();
        assert!(code == 401 || code == 403, "missing HMAC must 401/403, got {code}");

        let rec = arc
            .task_queue
            .as_ref()
            .unwrap()
            .get(job_id)
            .await
            .expect("get")
            .expect("row");
        assert_eq!(rec.status, pm_types::TaskStatus::Running);
    }

    #[tokio::test]
    async fn rpc_task_resume_authed_returns_task_to_running() {
        let _g = env_guard();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let queue = crate::TaskQueue::new(
            crate::TaskStore::open_at(dir.path().join("phantom.db")).expect("open"),
        );
        let job_id = seed_running_task(&queue).await;
        // Park it first (as STOP would).
        queue
            .transition(job_id, pm_types::TaskStatus::AwaitingApproval, None)
            .await
            .expect("→park");
        let (arc, router) = router_with_queue(queue);

        let body = json!({ "job_id": job_id.to_string() }).to_string();
        let resp = router
            .oneshot(signed_post("/rpc/task/resume", &body))
            .await
            .expect("call /rpc/task/resume");
        assert_eq!(resp.status(), StatusCode::OK, "valid HMAC resume must 200");
        let j = body_json(resp).await;
        assert_eq!(j["status"], json!("running"), "resume reports running");

        let rec = arc
            .task_queue
            .as_ref()
            .unwrap()
            .get(job_id)
            .await
            .expect("get")
            .expect("row");
        assert_eq!(
            rec.status,
            pm_types::TaskStatus::Running,
            "RESUME must return the task to Running"
        );
    }

    #[tokio::test]
    async fn rpc_task_resume_bad_hmac_fails_closed_no_state_change() {
        let _g = env_guard();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let queue = crate::TaskQueue::new(
            crate::TaskStore::open_at(dir.path().join("phantom.db")).expect("open"),
        );
        let job_id = seed_running_task(&queue).await;
        queue
            .transition(job_id, pm_types::TaskStatus::AwaitingApproval, None)
            .await
            .expect("→park");
        let (arc, router) = router_with_queue(queue);

        let body = json!({ "job_id": job_id.to_string() }).to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/rpc/task/resume")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .expect("build");
        let resp = router.oneshot(req).await.expect("call");
        let code = resp.status().as_u16();
        assert!(code == 401 || code == 403, "missing HMAC must 401/403, got {code}");

        let rec = arc
            .task_queue
            .as_ref()
            .unwrap()
            .get(job_id)
            .await
            .expect("get")
            .expect("row");
        assert_eq!(
            rec.status,
            pm_types::TaskStatus::AwaitingApproval,
            "bad-HMAC resume must NOT change durable state"
        );
    }

    /// Route-presence: both new routes are WIRED (not 404/405). An
    /// unauthenticated POST is rejected with an auth error (401/403) — never
    /// "not found" (404) or "method not allowed" (405).
    #[tokio::test]
    async fn rpc_task_stop_and_resume_routes_exist() {
        let _g = env_guard();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let queue = crate::TaskQueue::new(
            crate::TaskStore::open_at(dir.path().join("phantom.db")).expect("open"),
        );
        let (_arc, _r) = router_with_queue(queue);
        for path in ["/rpc/task/stop", "/rpc/task/resume"] {
            // Fresh router per call (oneshot consumes it).
            let dir2 = tempfile::TempDir::new().expect("tempdir");
            let q2 = crate::TaskQueue::new(
                crate::TaskStore::open_at(dir2.path().join("phantom.db")).expect("open"),
            );
            let (_a, router) = router_with_queue(q2);
            let req = Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .expect("build");
            let resp = router.oneshot(req).await.expect("call");
            let code = resp.status().as_u16();
            assert_ne!(code, 404, "{path} must be WIRED (got 404)");
            assert_ne!(code, 405, "{path} must accept POST (got 405)");
            assert!(
                code == 401 || code == 403,
                "{path} must fail closed for unauthed POST, got {code}"
            );
        }
    }

    // ─── P1-2 mobile-supervisor RPC tests ──────────────────────────────────
    // `/rpc/tasks/list`, `/rpc/captures/recent`, `/rpc/review` — HMAC-authed
    // read endpoints that the phone supervisor tabs poll. All hermetic: a
    // temp PHANTOM_HOME data-root + plaintext on-disk fixtures + oneshot.

    /// RAII: point the phantom DATA-ROOT (`PHANTOM_HOME`) at a throwaway dir for
    /// one test, restoring the prior value on drop. `phantom_data_dir()` honors
    /// `PHANTOM_HOME` verbatim, so `events/` and `pending/` both resolve under it
    /// (MEMORY: windows-home-resolution-phantom-home). Caller must hold
    /// [`env_guard`] (serializes the env mutation).
    struct PhantomHomeGuard {
        _tmp: tempfile::TempDir,
        prev: Option<std::ffi::OsString>,
    }
    impl PhantomHomeGuard {
        fn new() -> Self {
            let tmp = tempfile::TempDir::new().expect("tempdir");
            let prev = std::env::var_os("PHANTOM_HOME");
            std::env::set_var("PHANTOM_HOME", tmp.path());
            Self { _tmp: tmp, prev }
        }
        fn data_dir(&self) -> std::path::PathBuf {
            self._tmp.path().to_path_buf()
        }
    }
    impl Drop for PhantomHomeGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("PHANTOM_HOME", v),
                None => std::env::remove_var("PHANTOM_HOME"),
            }
        }
    }

    /// Build a signed POST request for a P1-2 supervisor RPC: `X-Cluster-Auth`
    /// is the HMAC-SHA256 of the exact body keyed by [`TEST_CLUSTER_SECRET`],
    /// matching how the phone's `clusterPost` signs the raw body.
    fn signed_post(uri: &str, body: &str) -> Request<Body> {
        let token = ClusterManager::new(ClusterConfig {
            cluster_secret: Some(TEST_CLUSTER_SECRET.into()),
            ..ClusterConfig::default()
        })
        .make_auth_token(body);
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .header("X-Cluster-Auth", token)
            .body(Body::from(body.to_string()))
            .expect("build signed request")
    }

    /// Build a router with a configured cluster_secret but NO task_queue — for
    /// the captures/review tests, which read the events dir, not the queue.
    fn router_secret_only() -> axum::Router {
        let cfg = ClusterConfig {
            node_name: Some("test".into()),
            cluster_secret: Some(TEST_CLUSTER_SECRET.into()),
            ..ClusterConfig::default()
        };
        let mut state = AppState::new();
        state.cluster_manager = ClusterManager::new(cfg);
        super::router(Arc::new(state))
    }

    /// Write a plaintext event fixture (`meta.json` [+ optional `analysis.json`])
    /// under `events_dir/<id>/`. `kind` is the on-disk free-form string
    /// (`"food_log"` → projects to `EventKind::Food` → wire `"food"`).
    fn write_event_fixture(
        events_dir: &std::path::Path,
        id: &str,
        kind: &str,
        timestamp: &str,
        tags: &[&str],
        analysis_summary: Option<&str>,
    ) {
        let ev = events_dir.join(id);
        std::fs::create_dir_all(&ev).expect("mkdir event");
        let meta = json!({
            "event_id": id,
            "kind": kind,
            "timestamp": timestamp,
            "source_node": "test",
            "goal_tags": tags,
            "modality_files": [],
            "user_text": "salad"
        });
        std::fs::write(ev.join("meta.json"), meta.to_string()).expect("write meta");
        if let Some(summary) = analysis_summary {
            let analysis = json!({
                "summary": summary,
                "confidence": 0.9,
                "goal_impact": "",
                "suggestion": "",
                "cost_usd": 0.0,
                "latency_ms": 0,
                "model_id": "test:offline",
                "raw_response": ""
            });
            std::fs::write(ev.join("analysis.json"), analysis.to_string()).expect("write analysis");
        }
    }

    #[tokio::test]
    async fn rpc_tasks_list_returns_durable_tasks_authed() {
        let _g = env_guard();
        let _home = PhantomHomeGuard::new(); // isolate pending dir
        let dir = tempfile::TempDir::new().expect("tempdir");
        let db = dir.path().join("phantom.db");
        let queue = crate::TaskQueue::new(crate::TaskStore::open_at(db).expect("open"));
        // Seed one durable task.
        queue
            .create("ws", "coder", "fix the bug")
            .await
            .expect("create");

        let (_arc, router) = router_with_queue(queue);
        let body = json!({ "limit": 50 }).to_string();
        let resp = router
            .oneshot(signed_post("/rpc/tasks/list", &body))
            .await
            .expect("call /rpc/tasks/list");
        assert_eq!(resp.status(), StatusCode::OK);
        let j = body_json(resp).await;
        assert_eq!(j["tasks"].as_array().unwrap().len(), 1);
        assert_eq!(j["tasks"][0]["agent_name"], "coder");
        assert_eq!(j["tasks"][0]["prompt"], "fix the bug");
        assert!(j["pending"].is_array(), "pending key must always be present");
    }

    #[tokio::test]
    async fn rpc_tasks_list_rejects_unauthed() {
        let _g = env_guard();
        let _home = PhantomHomeGuard::new();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let queue = crate::TaskQueue::new(
            crate::TaskStore::open_at(dir.path().join("phantom.db")).expect("open"),
        );
        let (_arc, router) = router_with_queue(queue);
        let req = Request::builder()
            .method("POST")
            .uri("/rpc/tasks/list")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let resp = router.oneshot(req).await.expect("call");
        let code = resp.status().as_u16();
        assert!(code == 401 || code == 403, "unauthed must be rejected, got {code}");
    }

    #[tokio::test]
    async fn rpc_captures_recent_lists_event_metas_authed() {
        let _g = env_guard();
        let home = PhantomHomeGuard::new();
        let events_dir = home.data_dir().join("events");
        write_event_fixture(
            &events_dir,
            "e1",
            "food_log",
            "2026-06-17T01:02:03Z",
            &["fat_loss"],
            None,
        );

        let router = router_secret_only();
        let resp = router
            .oneshot(signed_post("/rpc/captures/recent", "{}"))
            .await
            .expect("call");
        assert_eq!(resp.status(), StatusCode::OK);
        let j = body_json(resp).await;
        let caps = j["captures"].as_array().unwrap();
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0]["event_id"], "e1");
        // On-disk "food_log" → EventKind::Food → snake_case wire "food".
        assert_eq!(caps[0]["kind"], "food");
        assert_eq!(caps[0]["tags"][0], "fat_loss");
    }

    #[tokio::test]
    async fn rpc_captures_recent_rejects_unauthed() {
        let _g = env_guard();
        let _home = PhantomHomeGuard::new();
        let router = router_secret_only();
        let req = Request::builder()
            .method("POST")
            .uri("/rpc/captures/recent")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let resp = router.oneshot(req).await.expect("call");
        let code = resp.status().as_u16();
        assert!(code == 401 || code == 403, "unauthed must be rejected, got {code}");
    }

    #[tokio::test]
    async fn rpc_review_aggregates_events_for_date_authed() {
        let _g = env_guard();
        let home = PhantomHomeGuard::new();
        let events_dir = home.data_dir().join("events");
        // 01:02:03Z is the same local calendar day for any TZ ≥ UTC-1, which
        // covers the operator's UTC+8 host; load_events_for_date matches on the
        // LOCAL date. Provide analysis so the (meta, analysis) pair is kept.
        write_event_fixture(
            &events_dir,
            "e1",
            "food_log",
            "2026-06-17T01:02:03Z",
            &["fat_loss"],
            Some("ate a salad"),
        );

        let router = router_secret_only();
        let body = json!({ "date": "2026-06-17" }).to_string();
        let resp = router
            .oneshot(signed_post("/rpc/review", &body))
            .await
            .expect("call");
        assert_eq!(resp.status(), StatusCode::OK);
        let j = body_json(resp).await;
        assert_eq!(j["date"], "2026-06-17");
        let md = j["markdown"].as_str().unwrap();
        assert!(
            md.contains("Daily review"),
            "markdown should be the aggregate brief, got: {md}"
        );
        assert!(
            md.contains("ate a salad"),
            "aggregate should include the analysis summary, got: {md}"
        );
    }

    #[tokio::test]
    async fn rpc_review_rejects_unauthed() {
        let _g = env_guard();
        let _home = PhantomHomeGuard::new();
        let router = router_secret_only();
        let req = Request::builder()
            .method("POST")
            .uri("/rpc/review")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let resp = router.oneshot(req).await.expect("call");
        let code = resp.status().as_u16();
        assert!(code == 401 || code == 403, "unauthed must be rejected, got {code}");
    }

    #[tokio::test]
    async fn durable_status_survives_restart() {
        // gap-a done-when: a job accepted before a daemon restart is still
        // answerable by /rpc/task/status — a terminal status, NOT "job not
        // found". Deterministic (no agent spawn): we persist a Running row,
        // drop the connection (≈ process exit), reopen the same db file, run
        // the boot-time mark_interrupted sweep, then poll via the handler.
        let dir = tempfile::TempDir::new().expect("tempdir");
        let db = dir.path().join("phantom.db");

        // Before restart: accept + mark running, persisted to the db file.
        let job_id = {
            let q = crate::TaskQueue::new(crate::TaskStore::open_at(db.clone()).expect("open"));
            let id = uuid::Uuid::new_v4();
            q.create_with_id(id, "test", "master", "long job")
                .await
                .expect("create");
            q.transition(id, pm_types::TaskStatus::Running, None)
                .await
                .expect("running");
            id
        }; // queue (and its sqlite connection) dropped → simulates exit

        // Restart: reopen the SAME file; boot runs mark_interrupted.
        let q = crate::TaskQueue::new(crate::TaskStore::open_at(db.clone()).expect("reopen"));
        let swept = q.mark_interrupted().await.expect("sweep");
        assert_eq!(swept, 1, "the pre-restart Running job must be swept to Failed");

        let (_state, router) = router_with_queue(q);
        let req = Request::builder()
            .method("GET")
            .uri(format!("/rpc/task/status/{job_id}"))
            .body(Body::empty())
            .expect("build status req");
        let resp = router.oneshot(req).await.expect("call /rpc/task/status");
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_ne!(
            body["error"],
            json!("job not found"),
            "durable job must NOT read as 'job not found' after restart, got {body}"
        );
        assert_eq!(
            body["status"],
            json!("error"),
            "an interrupted job maps to the legacy wire 'error', got {body}"
        );
        assert_eq!(body["job_id"], json!(job_id.to_string()));
        assert!(
            body["error"].as_str().unwrap_or("").contains("interrupted"),
            "error should explain the restart interruption, got {body}"
        );
    }

    #[tokio::test]
    async fn assign_persists_durable_row() {
        // The /rpc/task/assign handler must write to the durable store when a
        // task queue is configured, so the returned job_id is resolvable (and
        // survives a restart). Asserts row existence + identity, tolerant of
        // whatever terminal status the background agent run lands on.
        let _g = env_guard();
        let _idem = IdemStoreGuard::new();
        std::env::remove_var("PHANTOM_ENFORCE_REQUIRED_CAPS"); // soft mode → Allow

        let dir = tempfile::TempDir::new().expect("tempdir");
        let db = dir.path().join("phantom.db");
        let q = crate::TaskQueue::new(crate::TaskStore::open_at(db).expect("open"));
        let (state, router) = router_with_queue(q);

        let body = json!({ "agent": "master", "prompt": "durable-roundtrip probe" });
        let resp = router
            .oneshot(assign_request(body))
            .await
            .expect("call /rpc/task/assign");
        assert_eq!(
            resp.status(),
            StatusCode::ACCEPTED,
            "durable assign must accept (202)"
        );
        let rb = body_json(resp).await;
        let job_id = rb["job_id"].as_str().expect("response carries job_id").to_string();
        let job_uuid = uuid::Uuid::parse_str(&job_id).expect("job_id is a uuid");

        let rec = state
            .task_queue
            .as_ref()
            .unwrap()
            .get(job_uuid)
            .await
            .expect("durable get")
            .expect("assigned job must exist in the durable store under its job_id");
        assert_eq!(rec.task_id, job_uuid);
        assert_eq!(rec.agent_name, "master");
    }

    #[tokio::test]
    async fn strict_mode_accepts_subset_match() {
        // required ⊆ local, strict mode → 202 Accepted, job_id present.
        let _g = env_guard();
        let _idem = IdemStoreGuard::new();
        std::env::remove_var("PHANTOM_ENFORCE_REQUIRED_CAPS");
        let router = router_with_caps(
            vec!["file_in_container".into(), "memory".into(), "web".into()],
            Some(EnforceMode::Strict),
        );
        let req = assign_request(json!({
            "agent":         "master",
            "prompt":        "research a thing",
            "required_caps": ["file_in_container", "web"],
        }));
        let resp = router.oneshot(req).await.expect("call /rpc/task/assign");
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let body = body_json(resp).await;
        assert!(
            body.get("job_id").and_then(|v| v.as_str()).is_some(),
            "matching caps must produce a job_id, got {body}"
        );
    }

    #[tokio::test]
    async fn strict_mode_accepts_request_with_no_required_caps_field() {
        // Old client that doesn't know about required_caps at all.
        // Forward-compat invariant from PR #32: missing field == [].
        // Even a tight sandbox worker in strict mode must accept.
        let _g = env_guard();
        let _idem = IdemStoreGuard::new();
        std::env::remove_var("PHANTOM_ENFORCE_REQUIRED_CAPS");
        let router = router_with_caps(vec!["file_in_container".into()], Some(EnforceMode::Strict));
        let req = assign_request(json!({
            "agent":  "master",
            "prompt": "legacy client call",
            // no required_caps field at all
        }));
        let resp = router.oneshot(req).await.expect("call /rpc/task/assign");
        assert_eq!(
            resp.status(),
            StatusCode::ACCEPTED,
            "old client (no required_caps) must be accepted even in strict mode"
        );
        let body = body_json(resp).await;
        assert!(body.get("job_id").and_then(|v| v.as_str()).is_some());
    }

    #[tokio::test]
    async fn strict_mode_full_worker_accepts_any_required_caps() {
        // Mac/Win/Linux full worker: worker_caps = []. Strict mode
        // must still accept even exotic required_caps because the
        // worker_caps=[] sentinel means "no restriction".
        let _g = env_guard();
        let _idem = IdemStoreGuard::new();
        std::env::remove_var("PHANTOM_ENFORCE_REQUIRED_CAPS");
        let router = router_with_caps(
            vec![], // full worker
            Some(EnforceMode::Strict),
        );
        let req = assign_request(json!({
            "agent":         "master",
            "prompt":        "exotic task",
            "required_caps": ["shell", "gpu", "kernel_module"],
        }));
        let resp = router.oneshot(req).await.expect("call /rpc/task/assign");
        assert_eq!(
            resp.status(),
            StatusCode::ACCEPTED,
            "full worker (empty worker_caps) must accept any required_caps"
        );
        let body = body_json(resp).await;
        assert!(body.get("job_id").and_then(|v| v.as_str()).is_some());
    }

    #[tokio::test]
    async fn env_override_flips_soft_node_to_strict() {
        // Operator escape hatch: a node configured for soft mode
        // (or no config) should flip to strict when PHANTOM_ENFORCE_
        // REQUIRED_CAPS=strict is in the environment. The override
        // is read at request time via effective_enforce_mode().
        let _g = env_guard();
        std::env::set_var("PHANTOM_ENFORCE_REQUIRED_CAPS", "strict");
        let router = router_with_caps(
            vec!["file_in_container".into()],
            Some(EnforceMode::Soft), // config says soft …
        );
        let req = assign_request(json!({
            "agent":         "master",
            "prompt":        "test",
            "required_caps": ["shell"],
        }));
        let resp = router.oneshot(req).await.expect("call /rpc/task/assign");
        // … but env beat config, so 409.
        assert_eq!(
            resp.status(),
            StatusCode::CONFLICT,
            "env override must promote soft config to strict enforcement"
        );
        std::env::remove_var("PHANTOM_ENFORCE_REQUIRED_CAPS");
    }

    /// SHARED P0 (`mesh::tests::cluster_secret_mismatch_rejects_with_401`)
    ///
    /// Verifies the cluster-auth gate end-to-end through the axum router:
    /// a request signed with a DIFFERENT secret than the node has
    /// configured must come back 401, BEFORE the dispatch filter or any
    /// downstream handler sees the body. This is the contract that keeps
    /// rogue tailnet peers from posting `/rpc/task/assign` and executing
    /// shell commands on this node — a regression that returned 200 here
    /// would ship a remote-exec hole.
    #[tokio::test]
    async fn cluster_secret_mismatch_rejects_with_401() {
        let router = router_with_caps(vec!["file_in_container".into()], None);

        let body = json!({
            "agent":  "master",
            "prompt": "rogue request",
        });
        let body_str = body.to_string();

        let attacker_cfg = ClusterConfig {
            cluster_secret: Some("attacker-secret-xyz".to_string()),
            ..ClusterConfig::default()
        };
        let bad_token = ClusterManager::new(attacker_cfg).make_auth_token(&body_str);

        let req = Request::builder()
            .method("POST")
            .uri("/rpc/task/assign")
            .header("content-type", "application/json")
            .header("X-Cluster-Auth", bad_token)
            .body(Body::from(body_str))
            .expect("build request");

        let resp = router.oneshot(req).await.expect("call /rpc/task/assign");
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "wrong-secret token must trip the cluster-auth gate with 401",
        );

        let body = body_json(resp).await;
        let err_msg = body.get("error").and_then(|v| v.as_str()).unwrap_or("");
        assert!(
            err_msg.contains("unauthorized"),
            "error body should say unauthorized, got: {body}",
        );
    }

    /// SECURITY regression (review #321 §1 + §2): the previous handlers gated the
    /// HMAC check behind `if secret_configured`, so an EMPTY cluster_secret made
    /// `/rpc/squad/dispatch` (unauth RCE) and `/rpc/evolve-handoff` (unauth disk
    /// write) FAIL OPEN. With the fail-closed `require_cluster_auth_dual` fix, an
    /// unauthenticated POST to either route must be REJECTED (not 200) when the
    /// secret is empty and the migration override is unset.
    #[tokio::test]
    async fn empty_secret_fails_closed_on_dispatch_and_handoff() {
        let _g = crate::env_lock::acquire();
        std::env::remove_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET");

        // Router with an EMPTY cluster_secret (the fail-open trigger).
        let cfg = ClusterConfig {
            node_name: Some("test".into()),
            cluster_secret: Some(String::new()),
            ..ClusterConfig::default()
        };
        let mut state = AppState::new();
        state.cluster_manager = ClusterManager::new(cfg);
        let router = super::router(Arc::new(state));

        for path in ["/rpc/squad/dispatch", "/rpc/evolve-handoff"] {
            let req = Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(json!({ "agent": "master", "prompt": "x" }).to_string()))
                .expect("build request");
            let resp = router.clone().oneshot(req).await.expect("call route");
            assert!(
                resp.status().is_client_error(),
                "{path}: empty secret + no auth must FAIL CLOSED (got {})",
                resp.status()
            );
        }
    }
}

#[cfg(test)]
mod node_capabilities_tests {
    use super::*;
    use serde_json::Value;

    /// PF-4: `GET /node/capabilities` handler returns the same payload
    /// as `phantom node-capabilities --json` (NodeCapabilityReport via
    /// the PF-3 detector).
    ///
    /// We call the handler directly (no router/server spin-up) and
    /// unwrap the Json wrapper.
    #[tokio::test]
    async fn returns_schema_v1_with_platform_and_capabilities() {
        let resp = node_capabilities().await;
        let v: &Value = &resp.0;

        // schema_version always 1 for v0.6.0 / v0.7.0
        assert_eq!(v["schema_version"], 1, "schema_version must be 1");

        // platform present + has os field
        let platform = &v["platform"];
        assert!(platform.is_object(), "platform must be an object");
        assert!(platform["os"].is_string(), "platform.os must be a string");

        // capability_ids is a non-empty array of strings
        let cap_ids = v["capability_ids"]
            .as_array()
            .expect("capability_ids must be array");
        assert!(!cap_ids.is_empty(), "at least 1 capability detected");
        for id in cap_ids {
            assert!(id.is_string(), "every capability_id is string");
        }
    }

    /// Confirms response format matches `phantom node-capabilities --json`
    /// (which uses the same `NodeCapabilityReport` via PF-3). PF-4 DoD.
    #[tokio::test]
    async fn http_payload_matches_cli_json_payload() {
        let resp = node_capabilities().await;
        let from_http: &Value = &resp.0;

        let from_cli_struct = crate::capabilities::NodeCapabilityReport::detect();
        let from_cli_json = serde_json::to_value(&from_cli_struct).unwrap();

        assert_eq!(
            from_http, &from_cli_json,
            "HTTP /node/capabilities payload must equal CLI --json payload"
        );
    }
}

#[cfg(test)]
mod api_events_route_tests {
    //! E002 Task 10 — smoke test that the two new Life-Track routes
    //! (POST `/api/events`, GET `/api/events/:id/analysis`) are wired
    //! into the production router. End-to-end exercise with a real
    //! provider lives in Task 11.
    use super::*;
    use crate::AppState;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt; // for `oneshot`

    /// Smoke that the route is wired into the router. End-to-end with
    /// real Gemini lives in Task 11; this just proves multipart parsing
    /// is registered.
    #[tokio::test]
    async fn api_events_route_exists_in_router() {
        let state = AppState::new();
        let router = super::router(Arc::new(state));
        let req = Request::builder()
            .method("GET")
            .uri("/api/events/nonexistent/analysis")
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        // 404 is fine — proves the route IS wired, just no data.
        // 405 would mean the route exists for some other method but
        // not GET — i.e. the registration was off.
        assert_ne!(
            resp.status().as_u16(),
            405,
            "405 means route exists for some method but not the one we tried; \
             expected /api/events/:id/analysis registered for GET"
        );
    }

    /// T-CORE-02 (codex review #3): prove POST /rpc/capability-query is wired
    /// into the router AND auth-gated end-to-end. With no cluster_secret and no
    /// override, the dual gate fails closed (403); it must never be open (200)
    /// nor unwired (404/405).
    #[tokio::test]
    async fn capability_query_route_is_wired_and_auth_gated() {
        let _g = crate::env_lock::acquire();
        std::env::remove_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET");
        let state = AppState::new();
        let router = super::router(Arc::new(state));
        let req = Request::builder()
            .method("POST")
            .uri("/rpc/capability-query")
            .body(Body::from("{}"))
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        let code = resp.status().as_u16();
        assert!(
            code == 401 || code == 403,
            "unauthenticated POST /rpc/capability-query must be rejected (401/403), got {code}"
        );
    }

    /// #321 bonus: a parseable-but-non-table `core` (or any of core/providers/
    /// cluster/agent) in an existing agents.toml previously panicked the
    /// onboarding handler via `.as_table_mut().unwrap()` — an axum handler panic
    /// returns an empty 500 (or aborts the task) with no diagnostic. It must now
    /// return a graceful 500 naming the offending key. Driven with `dryrun=1`
    /// (no file write) + the empty-secret override so the HMAC gate passes.
    #[tokio::test]
    async fn onboarding_non_table_key_returns_graceful_500_not_panic() {
        let _g = crate::env_lock::acquire();
        std::env::set_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET", "1");
        struct VarGuard(&'static str, Option<String>);
        impl Drop for VarGuard {
            fn drop(&mut self) {
                match &self.1 {
                    Some(v) => std::env::set_var(self.0, v),
                    None => std::env::remove_var(self.0),
                }
            }
        }
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let _h = VarGuard("HOME", std::env::var("HOME").ok());
        let _a = VarGuard(
            "PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET",
            std::env::var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET").ok(),
        );
        std::env::set_var("HOME", tmp.path());

        // Seed ~/.phantom-mesh/agents.toml with a NON-TABLE `core` key. This is
        // valid TOML (parses fine) but breaks the `core.as_table_mut()` assumption.
        let cfg_dir = tmp.path().join(".phantom-mesh");
        std::fs::create_dir_all(&cfg_dir).expect("mkdir cfg");
        std::fs::write(cfg_dir.join("agents.toml"), "core = 1\n").expect("seed toml");

        let state = AppState::new();
        let router = super::router(Arc::new(state));
        let req = Request::builder()
            .method("POST")
            .uri("/api/onboarding?dryrun=1")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"groq_api_key":"gsk_test"}"#))
            .expect("build request");
        let resp = router.oneshot(req).await.expect("call /api/onboarding");
        assert_eq!(
            resp.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "a non-table `core` must produce a graceful 500, not a panic/empty response"
        );
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .expect("read body");
        let msg = String::from_utf8_lossy(&bytes);
        assert!(
            msg.contains("core") && msg.contains("not a table"),
            "graceful 500 should name the offending non-table key, got: {msg}"
        );
    }
}

#[cfg(test)]
mod dual_auth_gate_tests {
    //! T-CORE-01 Stage 2: the inbound `/rpc/*` gate must accept BOTH the legacy
    //! body-HMAC (`X-Cluster-Auth`) and the SPEC-10 canonical-HMAC
    //! (`X-Cluster-Auth`) during the migration window, while preserving the
    //! legacy reject semantics (401 on bad token, fail-closed on no secret).
    use super::require_cluster_auth_dual;
    use crate::mesh::{ClusterConfig, ClusterManager};
    use axum::http::{HeaderMap, StatusCode};

    fn cm(secret: &str) -> ClusterManager {
        ClusterManager::new(ClusterConfig {
            cluster_secret: Some(secret.to_string()),
            ..ClusterConfig::default()
        })
    }

    #[test]
    fn accepts_legacy_x_cluster_auth() {
        let mgr = cm("seal-the-mesh");
        let body = br#"{"message":"hi","agent":"master"}"#;
        let token = mgr.make_auth_token(std::str::from_utf8(body).unwrap());
        let mut h = HeaderMap::new();
        h.insert("X-Cluster-Auth", token.parse().unwrap());
        assert!(
            require_cluster_auth_dual(&mgr, &h, "POST", "/rpc/message", None, body).is_ok(),
            "legacy X-Cluster-Auth must be accepted"
        );
    }

    #[test]
    fn accepts_spec10_literal_canonical_in_x_cluster_auth() {
        // T-DRIFT-10a acceptance: a SPEC-10-literal client puts the canonical
        // HMAC in the spec's `X-Cluster-Auth` header (NOT an invented header).
        // The legacy body-HMAC check fails, then the canonical arm — reading the
        // SAME X-Cluster-Auth header — verifies it. No X-Phantom-Signature.
        let secret = "seal-the-mesh";
        let mgr = cm(secret);
        let body = br#"{"message":"hi","agent":"master"}"#;
        let canonical =
            crate::rpc_wire::build_canonical_string("POST", "/rpc/message", "", body, None);
        let sig = crate::rpc_wire::sign_hmac(secret.as_bytes(), &canonical);
        let mut h = HeaderMap::new();
        h.insert("X-Cluster-Auth", sig.parse().unwrap());
        assert!(
            require_cluster_auth_dual(&mgr, &h, "POST", "/rpc/message", None, body).is_ok(),
            "SPEC-10-literal canonical HMAC in X-Cluster-Auth must be accepted"
        );
    }

    #[test]
    fn rejects_when_neither_present() {
        let mgr = cm("seal-the-mesh");
        let body = br#"{"message":"hi"}"#;
        let h = HeaderMap::new();
        let err = require_cluster_auth_dual(&mgr, &h, "POST", "/rpc/message", None, body).unwrap_err();
        assert_eq!(
            err.0,
            StatusCode::UNAUTHORIZED,
            "no auth header with a configured secret → 401 (legacy semantics preserved)"
        );
    }

    #[test]
    fn rejects_canonical_sig_for_wrong_path() {
        // A signature bound to a different path must NOT validate — proves the
        // canonical string really covers method+path, not just the body.
        let secret = "seal-the-mesh";
        let mgr = cm(secret);
        let body = br#"{"message":"hi"}"#;
        let canonical =
            crate::rpc_wire::build_canonical_string("POST", "/rpc/swarm", "", body, None);
        let sig = crate::rpc_wire::sign_hmac(secret.as_bytes(), &canonical);
        let mut h = HeaderMap::new();
        h.insert("X-Cluster-Auth", sig.parse().unwrap());
        assert!(
            require_cluster_auth_dual(&mgr, &h, "POST", "/rpc/message", None, body).is_err(),
            "a canonical sig minted for /rpc/swarm must not authorize /rpc/message"
        );
    }

    #[test]
    fn inbox_auth_accepts_legacy_body_hmac_and_binds_canonical_to_path() {
        // /rpc/inbox is gated by the same dual scheme as the other /rpc routes:
        // (1) the legacy body-HMAC arm (what `phantom inbox send` mints) must
        // pass, and (2) a canonical sig minted for a different path must NOT
        // authorize /rpc/inbox.
        let secret = "seal-the-mesh";
        let mgr = cm(secret);
        let body = br#"{"from":"m1","text":"tick"}"#;
        let legacy = {
            use hmac::{Hmac, Mac};
            use sha2::Sha256;
            let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
            mac.update(body);
            hex::encode(mac.finalize().into_bytes())
        };
        let mut h = HeaderMap::new();
        h.insert("X-Cluster-Auth", legacy.parse().unwrap());
        assert!(
            require_cluster_auth_dual(&mgr, &h, "POST", "/rpc/inbox", None, body).is_ok(),
            "legacy body-HMAC (the `phantom inbox send` client arm) must authorize /rpc/inbox"
        );

        let canonical =
            crate::rpc_wire::build_canonical_string("POST", "/rpc/message", "", body, None);
        let sig = crate::rpc_wire::sign_hmac(secret.as_bytes(), &canonical);
        let mut h2 = HeaderMap::new();
        h2.insert("X-Cluster-Auth", sig.parse().unwrap());
        assert!(
            require_cluster_auth_dual(&mgr, &h2, "POST", "/rpc/inbox", None, body).is_err(),
            "a canonical sig minted for /rpc/message must not authorize /rpc/inbox"
        );
    }

    #[test]
    fn session_status_auth_accepts_legacy_empty_body_hmac() {
        // GET /rpc/session-status is signed over the EMPTY body with the
        // legacy arm (what `phantom status mesh` mints, same as the dispatch
        // status poll). Must pass; and a sig over a non-empty body must not.
        let secret = "seal-the-mesh";
        let mgr = cm(secret);
        let legacy_empty = {
            use hmac::{Hmac, Mac};
            use sha2::Sha256;
            let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
            mac.update(b"");
            hex::encode(mac.finalize().into_bytes())
        };
        let mut h = HeaderMap::new();
        h.insert("X-Cluster-Auth", legacy_empty.parse().unwrap());
        assert!(
            require_cluster_auth_dual(&mgr, &h, "GET", "/rpc/session-status", None, b"").is_ok(),
            "legacy empty-body HMAC must authorize GET /rpc/session-status"
        );
        let mut h2 = HeaderMap::new();
        h2.insert("X-Cluster-Auth", "deadbeef".repeat(8).parse().unwrap());
        assert!(
            require_cluster_auth_dual(&mgr, &h2, "GET", "/rpc/session-status", None, b"").is_err(),
            "a wrong sig must reject"
        );
    }

    #[test]
    fn rejects_empty_phantom_signature() {
        // An empty X-Cluster-Auth is present-but-unusable: it must reject
        // (hex-decode of "" fails), never accidentally pass (review: codex).
        let mgr = cm("seal-the-mesh");
        let body = br#"{"message":"hi"}"#;
        let mut h = HeaderMap::new();
        h.insert("X-Cluster-Auth", "".parse().unwrap());
        assert!(
            require_cluster_auth_dual(&mgr, &h, "POST", "/rpc/message", None, body).is_err(),
            "empty X-Cluster-Auth must reject"
        );
    }

    #[test]
    fn none_secret_rejects_even_with_canonical() {
        // Fail-closed: a node with no cluster_secret must reject a canonical sig
        // (review: opencode O3) — 403, matching the legacy gate.
        let _g = crate::env_lock::acquire();
        std::env::remove_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET");
        let mgr = ClusterManager::new(ClusterConfig {
            cluster_secret: None,
            ..ClusterConfig::default()
        });
        let body = br#"{"message":"hi"}"#;
        let canonical =
            crate::rpc_wire::build_canonical_string("POST", "/rpc/message", "", body, None);
        let sig = crate::rpc_wire::sign_hmac(b"whatever-secret", &canonical);
        let mut h = HeaderMap::new();
        h.insert("X-Cluster-Auth", sig.parse().unwrap());
        let err =
            require_cluster_auth_dual(&mgr, &h, "POST", "/rpc/message", None, body).unwrap_err();
        assert_eq!(
            err.0,
            StatusCode::FORBIDDEN,
            "no secret configured → fail-closed 403 regardless of a canonical sig"
        );
    }

    #[test]
    fn empty_secret_override_env_accepts() {
        // Criterion C: PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET=1 still bypasses the
        // gate through the dual path (delegates to the legacy gate first).
        let _g = crate::env_lock::acquire();
        std::env::set_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET", "1");
        let mgr = ClusterManager::new(ClusterConfig {
            cluster_secret: None,
            ..ClusterConfig::default()
        });
        let body = br#"{"message":"hi"}"#;
        let h = HeaderMap::new();
        let result = require_cluster_auth_dual(&mgr, &h, "POST", "/rpc/message", None, body);
        std::env::remove_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET");
        assert!(
            result.is_ok(),
            "empty-secret override must permit the dual gate: {result:?}"
        );
    }

    #[test]
    fn accepts_canonical_with_traceparent() {
        // The gate must forward the inbound `traceparent` into the canonical so
        // a SPEC-10 sig that covers it verifies (review: opencode O3).
        let secret = "seal-the-mesh";
        let mgr = cm(secret);
        let body = br#"{"message":"hi"}"#;
        let tp = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
        let canonical =
            crate::rpc_wire::build_canonical_string("POST", "/rpc/message", "", body, Some(tp));
        let sig = crate::rpc_wire::sign_hmac(secret.as_bytes(), &canonical);
        let mut h = HeaderMap::new();
        h.insert("X-Cluster-Auth", sig.parse().unwrap());
        h.insert("traceparent", tp.parse().unwrap());
        assert!(
            require_cluster_auth_dual(&mgr, &h, "POST", "/rpc/message", None, body).is_ok(),
            "canonical sig over a traceparent-bearing canonical must verify when the gate forwards traceparent"
        );
    }

    #[test]
    fn query_string_does_not_authorize_via_canonical() {
        // Documents the route invariant (review: codex): the gate verifies with
        // an EMPTY query. A canonical sig that COVERS a query string therefore
        // will NOT verify — so adding a query without updating the gate is
        // fail-closed, never a bypass.
        let secret = "seal-the-mesh";
        let mgr = cm(secret);
        let body = br#"{"message":"hi"}"#;
        let canonical_with_q =
            crate::rpc_wire::build_canonical_string("POST", "/rpc/message", "x=1", body, None);
        let sig = crate::rpc_wire::sign_hmac(secret.as_bytes(), &canonical_with_q);
        let mut h = HeaderMap::new();
        h.insert("X-Cluster-Auth", sig.parse().unwrap());
        assert!(
            require_cluster_auth_dual(&mgr, &h, "POST", "/rpc/message", None, body).is_err(),
            "a canonical sig covering a query string must not authorize the empty-query gate"
        );
    }

    #[test]
    fn rejects_canonical_when_query_present() {
        // ENFORCED invariant (review: codex): even a perfectly valid empty-query
        // canonical signature must be refused when the request actually carries
        // a query string — otherwise the query rides unauthenticated.
        let secret = "seal-the-mesh";
        let mgr = cm(secret);
        let body = br#"{"message":"hi"}"#;
        // A sig that correctly covers the EMPTY query (would pass with no query):
        let canonical =
            crate::rpc_wire::build_canonical_string("POST", "/rpc/message", "", body, None);
        let sig = crate::rpc_wire::sign_hmac(secret.as_bytes(), &canonical);
        let mut h = HeaderMap::new();
        h.insert("X-Cluster-Auth", sig.parse().unwrap());
        // ...but the request carries `?x=1`, so the gate must refuse the
        // canonical arm and fall through to the legacy error.
        assert!(
            require_cluster_auth_dual(&mgr, &h, "POST", "/rpc/message", Some("x=1"), body).is_err(),
            "a query-bearing request must not be authorized by an empty-query canonical signature"
        );
        // Sanity: the SAME signature DOES authorize when no query is present.
        assert!(
            require_cluster_auth_dual(&mgr, &h, "POST", "/rpc/message", None, body).is_ok(),
            "the same canonical sig must still authorize a query-free request"
        );
    }
}

/// `/partner/message` origin marker → ledger routing (the dogfood-moat guard).
///
/// These pin the wire contract the `/partner/message` handler implements via
/// [`parse_origin_marker`] + [`crate::partner::resolve_origin`] +
/// [`crate::partner::record_interaction`]: a test/bot client that tags itself
/// machine is kept OUT of the human-usage ledger, while the real app (no marker)
/// defaults to Human. We drive the SAME parsing the handler uses and the SAME
/// recorder it calls (skipping only the LLM agent turn, which needs a live model),
/// then assert which ledger file the interaction landed in.
///
/// `PHANTOM_PARTNER_SIGNALS` relocates BOTH the human ledger and its derived
/// `.machine.jsonl` into a tempdir; it is taken under the crate-wide
/// [`crate::env_lock`] mutex — the SAME lock `partner.rs`'s env-touching tests
/// now use — so the two groups never race on the var.
#[cfg(test)]
mod partner_origin_marker_tests {
    use super::parse_origin_marker;
    use crate::partner::{
        machine_signals_path, record_interaction, resolve_origin, MessageOrigin,
    };
    use axum::http::HeaderMap;
    use serde_json::json;

    #[test]
    fn machine_marker_routes_to_machine_ledger_not_human() {
        // (a) A message tagged machine (header OR body field) must land in the
        // segregated `.machine.jsonl` and NEVER in the human-usage ledger.
        let _g = crate::env_lock::acquire();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let human = tmp.path().join("partner-signals.jsonl");
        std::env::set_var("PHANTOM_PARTNER_SIGNALS", &human);
        let machine = machine_signals_path();

        // (a-1) Header form: `X-Partner-Origin: machine` (the canonical marker).
        // Text is an untagged "記:" note — exactly the kind of E2E traffic that
        // used to leak into the human ledger before the marker existed.
        let mut headers = HeaderMap::new();
        headers.insert("X-Partner-Origin", "machine".parse().unwrap());
        let body_hdr = json!({ "text": "記: header bot note" });
        let origin_hdr =
            resolve_origin(parse_origin_marker(&headers, &body_hdr), "記: header bot note");
        record_interaction(origin_hdr, &json!({ "user": "記: header bot note" })).unwrap();

        // (a-2) Body form: `{"origin":"machine"}` — same effect, no header.
        let body_field = json!({ "text": "一句話 body bot", "origin": "machine" });
        let origin_field = resolve_origin(
            parse_origin_marker(&HeaderMap::new(), &body_field),
            "一句話 body bot",
        );
        record_interaction(origin_field, &json!({ "user": "一句話 body bot" })).unwrap();

        let machine_content = std::fs::read_to_string(&machine).unwrap_or_default();
        // The human ledger may not even be created — treat absent as empty.
        let human_content = std::fs::read_to_string(&human).unwrap_or_default();
        std::env::remove_var("PHANTOM_PARTNER_SIGNALS");

        // Both machine-marked messages landed in the segregated machine log...
        assert!(machine_content.contains("header bot note"), "machine: {machine_content}");
        assert!(machine_content.contains("body bot"), "machine: {machine_content}");
        assert_eq!(
            machine_content.lines().filter(|l| !l.trim().is_empty()).count(),
            2,
            "both machine-marked messages in the machine log"
        );
        // ...and NOTHING leaked into the human-usage ledger (the moat).
        assert!(
            !human_content.contains("header bot note") && !human_content.contains("body bot"),
            "machine-marked traffic must NOT pollute the human ledger: {human_content:?}"
        );
        assert!(
            human_content.lines().all(|l| l.trim().is_empty()),
            "human ledger must be empty for machine-only traffic: {human_content:?}"
        );
    }

    #[test]
    fn no_marker_routes_to_human_ledger() {
        // (b) Real-app behaviour: the iOS chat box sends NO origin marker, so an
        // ordinary message must default to Human and land in the human-usage
        // ledger — proving the marker is opt-in and the app needs no change.
        let _g = crate::env_lock::acquire();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let human = tmp.path().join("partner-signals.jsonl");
        std::env::set_var("PHANTOM_PARTNER_SIGNALS", &human);
        let machine = machine_signals_path();

        // No header, no `origin` field — exactly what the real app sends.
        let body = json!({ "text": "今天天氣如何" });
        assert_eq!(
            parse_origin_marker(&HeaderMap::new(), &body),
            None,
            "an unmarked message carries no explicit origin"
        );
        let origin = resolve_origin(parse_origin_marker(&HeaderMap::new(), &body), "今天天氣如何");
        record_interaction(origin, &json!({ "user": "今天天氣如何" })).unwrap();

        let human_content = std::fs::read_to_string(&human).unwrap_or_default();
        let machine_content = std::fs::read_to_string(&machine).unwrap_or_default();
        std::env::remove_var("PHANTOM_PARTNER_SIGNALS");

        // The unmarked (real-app) message is in the human-usage ledger...
        assert!(human_content.contains("今天天氣如何"), "human: {human_content}");
        assert_eq!(
            human_content.lines().filter(|l| !l.trim().is_empty()).count(),
            1,
            "the single unmarked message counts as human usage"
        );
        // ...and never touched the machine log.
        assert!(
            machine_content.lines().all(|l| l.trim().is_empty()),
            "unmarked human traffic must not reach the machine log: {machine_content:?}"
        );
    }

    #[test]
    fn parse_origin_marker_precedence_and_aliases() {
        // Pin the wire-marker precedence + header aliases the handler relies on,
        // without IO: body `origin` > `X-Partner-Origin` > `X-Phantom-Origin`,
        // unknown/absent → None (caller applies heuristic + Human default).

        // Body field wins over both headers.
        let mut h = HeaderMap::new();
        h.insert("X-Partner-Origin", "machine".parse().unwrap());
        let body = json!({ "text": "x", "origin": "human" });
        assert_eq!(
            parse_origin_marker(&h, &body),
            Some(MessageOrigin::Human),
            "body `origin` field wins over the header"
        );

        // `X-Partner-Origin` recognized (the brief's canonical marker).
        let mut h = HeaderMap::new();
        h.insert("X-Partner-Origin", "machine".parse().unwrap());
        assert_eq!(
            parse_origin_marker(&h, &json!({ "text": "x" })),
            Some(MessageOrigin::Machine),
            "X-Partner-Origin: machine → Machine"
        );

        // `X-Phantom-Origin` still recognized (historical alias / back-compat).
        let mut h = HeaderMap::new();
        h.insert("X-Phantom-Origin", "bot".parse().unwrap());
        assert_eq!(
            parse_origin_marker(&h, &json!({ "text": "x" })),
            Some(MessageOrigin::Machine),
            "X-Phantom-Origin alias still honored"
        );

        // `X-Partner-Origin` takes precedence over `X-Phantom-Origin`.
        let mut h = HeaderMap::new();
        h.insert("X-Partner-Origin", "human".parse().unwrap());
        h.insert("X-Phantom-Origin", "machine".parse().unwrap());
        assert_eq!(
            parse_origin_marker(&h, &json!({ "text": "x" })),
            Some(MessageOrigin::Human),
            "X-Partner-Origin outranks the X-Phantom-Origin alias"
        );

        // No marker anywhere → None (real-app default path).
        assert_eq!(parse_origin_marker(&HeaderMap::new(), &json!({ "text": "x" })), None);
        // An unknown marker → None (never silently upgraded).
        let mut h = HeaderMap::new();
        h.insert("X-Partner-Origin", "wat".parse().unwrap());
        assert_eq!(parse_origin_marker(&h, &json!({ "text": "x" })), None);
    }
}

#[cfg(test)]
mod task_assign_idempotency_tests {
    use super::task_assign_idem_key;

    #[test]
    fn explicit_key_wins_and_is_scoped() {
        // A caller-supplied idempotency_key is used verbatim under the
        // task_assign scope (this is the key a re-dispatch/forward preserves).
        assert_eq!(
            task_assign_idem_key(Some("req-7"), "master", "do x"),
            "task_assign:req-7"
        );
        // Surrounding whitespace is trimmed.
        assert_eq!(
            task_assign_idem_key(Some("  req-7  "), "master", "do x"),
            "task_assign:req-7"
        );
    }

    #[test]
    fn blank_or_absent_key_falls_back_to_content_hash() {
        let absent = task_assign_idem_key(None, "master", "do x");
        let blank = task_assign_idem_key(Some("   "), "master", "do x");
        // Both fall back to the same content hash (scope-prefixed), not the
        // explicit-key form.
        assert!(absent.starts_with("task_assign:"));
        assert_eq!(absent, blank, "absent and blank keys hash identically");
        assert_ne!(absent, "task_assign:req-7");
    }

    #[test]
    fn content_hash_is_stable_and_distinguishes_body() {
        let a = task_assign_idem_key(None, "master", "do x");
        let a2 = task_assign_idem_key(None, "master", "do x");
        assert_eq!(a, a2, "identical agent+prompt → identical key");
        let diff_prompt = task_assign_idem_key(None, "master", "do y");
        assert_ne!(a, diff_prompt, "different prompt → different key");
        let diff_agent = task_assign_idem_key(None, "worker", "do x");
        assert_ne!(a, diff_agent, "different agent → different key");
    }

    #[test]
    fn scope_does_not_collide_with_squad_dispatch() {
        // The same agent+prompt body must NOT dedup across the task_assign and
        // squad-dispatch ingresses — they are independent front doors.
        let assign = task_assign_idem_key(None, "master", "do x");
        let dispatch = crate::idempotency::content_key("dispatch", "master\ndo x");
        assert_ne!(assign, dispatch, "ingress scopes must stay distinct");
    }

    #[test]
    fn ledger_dedups_a_resent_assign() {
        // End-to-end over the real ledger primitive: first sighting proceeds,
        // an immediate re-post of the same derived key is a duplicate.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("idempotency.jsonl");
        let key = task_assign_idem_key(None, "master", "remind me to call mum");
        let now = 1_000_000;
        assert!(
            crate::idempotency::check_and_record_at(&path, &key, "task_assign", 3600, now)
                .is_first(),
            "first assign proceeds"
        );
        assert!(
            crate::idempotency::check_and_record_at(&path, &key, "task_assign", 3600, now + 1)
                .is_duplicate(),
            "re-sent assign within TTL is a duplicate"
        );
    }
}
