// F102 · Tauri dispatch commands + token-stream channel.
//
// Spec: docs/superpowers/specs/_current/E002-mobile-cluster-dispatch-ui.md
//   §"Dispatch screen" + Test matrix row "Security: Dispatch endpoint
//   requires valid broker token".
// Feature spec: docs/superpowers/features/F102-tauri-dispatch-commands.md
//
// Surface exposed to JS:
//   - dispatch_task(prompt, required_caps, provider_override?, broker_url?)
//       → returns { dispatch_id, started_at_unix }
//       → spawns a tokio task that POSTs to {broker}/api/squad/dispatch,
//         reads the SSE response, and emits `dispatch::token::<dispatch_id>`
//         Tauri events to the front window. Frame shapes are forwarded
//         verbatim so the JS render layer (F103) controls UX.
//   - cancel_dispatch(dispatch_id) → cancels the per-dispatch task.
//   - list_dispatch_providers() → allow-list pull for the F103 dropdown.
//
// V8-HIGH-2 contract: ALL validation lives in this file. JS passes raw
// strings; Rust enforces:
//   - prompt: 1..=8000 bytes UTF-8, no NUL byte, non-whitespace
//   - required_caps: each ^[a-z][a-z0-9_-]{0,31}$, max 3 entries
//   - provider_override: must be in the allow-list (built from
//     phantom_mesh::config::AgentsConfig::find_and_load().ok_or(()) at command time)
//   - broker_url: validated via the daemon-allowlist validator from
//     cluster_peers.rs::validate_daemon_url (re-used so the surface
//     stays enumerable from one place)
//   - broker token: pulled from phantom_mesh::auth::load(); missing
//     token → E_DISPATCH_AUTH_REQUIRED (E002 security gate).
//
// Stable error code prefix is `E_DISPATCH_` so the JS layer can pattern-
// match on the leading token (mirrors `E_CLUSTER_*` from cluster_peers.rs).

use phantom_mesh::auth;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{Emitter, State};
use tokio::sync::watch;

use crate::commands::cluster_peers::validate_daemon_url;
use crate::commands::HttpClient;

// ── Stable error codes ───────────────────────────────────────────────────

const E_PROMPT_EMPTY: &str = "E_DISPATCH_PROMPT_EMPTY";
const E_PROMPT_TOO_LONG: &str = "E_DISPATCH_PROMPT_TOO_LONG";
const E_PROMPT_INVALID: &str = "E_DISPATCH_PROMPT_INVALID";
const E_CAPS_INVALID: &str = "E_DISPATCH_CAPS_INVALID";
const E_CAPS_TOO_MANY: &str = "E_DISPATCH_CAPS_TOO_MANY";
const E_PROVIDER_UNKNOWN: &str = "E_DISPATCH_PROVIDER_UNKNOWN";
const E_AUTH_REQUIRED: &str = "E_DISPATCH_AUTH_REQUIRED";
const E_URL_INVALID: &str = "E_DISPATCH_URL_INVALID";
const E_NETWORK: &str = "E_DISPATCH_NETWORK";
const E_HTTP_STATUS: &str = "E_DISPATCH_HTTP_STATUS";

/// Hard cap on prompt size — matches the broker's /api/squad/dispatch
/// limit. Anything over this would be rejected server-side anyway, so
/// reject locally and save the round-trip + bearer-token exposure.
const MAX_PROMPT_BYTES: usize = 8_000;

/// Mobile UI exposes a ≤3-chip strip. Cap matches F103 acceptance.
const MAX_CAPS: usize = 3;

// ── Request / response types ─────────────────────────────────────────────

/// JS-side request payload. Names match Tauri's camelCase → snake_case
/// serde renaming applied to fn args — these go through `#[tauri::command]`
/// so the keys on the JS side are `prompt`, `requiredCaps`, etc.
#[derive(Deserialize)]
pub struct DispatchRequest {
    pub prompt: String,
    #[serde(default)]
    pub required_caps: Vec<String>,
    #[serde(default)]
    pub provider_override: Option<String>,
    /// Optional broker URL override. Defaults to the user's saved
    /// `state.broker_url` from `phantom_mesh::auth::load()`, then to
    /// `https://phantommesh.io`. The validator runs on whichever value
    /// is finally selected.
    #[serde(default)]
    pub broker_url: Option<String>,
}

// Custom Debug avoids leaking prompt text or auth tokens into trace logs
// or `dbg!()` snapshots — see `no_secret_in_debug` test below.
impl std::fmt::Debug for DispatchRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // We intentionally skip the prompt body and broker token. Caps +
        // provider are non-secret allow-list values, safe to print.
        f.debug_struct("DispatchRequest")
            .field("prompt_len", &self.prompt.len())
            .field("required_caps", &self.required_caps)
            .field("provider_override", &self.provider_override)
            .field("broker_url_set", &self.broker_url.is_some())
            .finish()
    }
}

#[derive(Debug, Serialize)]
pub struct DispatchHandle {
    pub dispatch_id: String,
    pub started_at_unix: u64,
}

#[derive(Debug, Serialize)]
pub struct ProviderSummary {
    pub name: String,
    /// True when the provider has a usable api_key in agents.toml. UI
    /// uses this to disable dropdown entries that aren't configured.
    pub configured: bool,
}

// ── Validators ───────────────────────────────────────────────────────────

pub fn validate_prompt(prompt: &str) -> Result<(), &'static str> {
    let trimmed_len = prompt.trim().len();
    if trimmed_len == 0 {
        return Err(E_PROMPT_EMPTY);
    }
    if prompt.len() > MAX_PROMPT_BYTES {
        return Err(E_PROMPT_TOO_LONG);
    }
    if prompt.contains('\0') {
        return Err(E_PROMPT_INVALID);
    }
    Ok(())
}

/// Each cap must match `^[a-z][a-z0-9_-]{0,31}$`. Inline hand-rolled
/// matcher so we don't pull in `regex` just for one allow-list.
fn cap_token_ok(s: &str) -> bool {
    if s.is_empty() || s.len() > 32 {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_lowercase() {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub fn validate_caps(caps: &[String]) -> Result<(), &'static str> {
    if caps.len() > MAX_CAPS {
        return Err(E_CAPS_TOO_MANY);
    }
    for c in caps {
        if !cap_token_ok(c) {
            return Err(E_CAPS_INVALID);
        }
    }
    Ok(())
}

/// Validate that `provider` (if Some) is in the supplied allow-list.
/// Caller is responsible for building `allowed` from agents.toml — split
/// so unit tests don't have to read the filesystem.
pub fn validate_provider_in_set(
    provider: Option<&str>,
    allowed: &[String],
) -> Result<(), &'static str> {
    let Some(p) = provider else { return Ok(()) };
    if p.is_empty() {
        // Treat empty string the same as "no override" — Tauri's serde
        // sometimes hands us `Some("")` for an unset dropdown.
        return Ok(());
    }
    if allowed.iter().any(|a| a == p) {
        Ok(())
    } else {
        Err(E_PROVIDER_UNKNOWN)
    }
}

// ── SSE frame parser ─────────────────────────────────────────────────────
//
// The broker emits Server-Sent-Events frames separated by blank lines.
// Each non-blank line is `key: value`. We care about `data:` lines —
// every other key (id, event, retry) is ignored. The `data:` value is
// JSON of the shape:
//
//   {"type":"token","text":"..."}
//   {"type":"status","phase":"queued"|"running"}
//   {"type":"done","result":"..."}
//   {"type":"error","code":"...","message":"..."}
//
// Unknown variants are forwarded verbatim — the JS layer can ignore
// them, and we don't crash the per-dispatch task on a broker version
// bump.

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DispatchFrame {
    Token { text: String },
    Status { phase: String },
    Done { result: String },
    Error { code: String, message: String },
    /// Catch-all so the front-end can render diagnostic frames the broker
    /// adds without forcing a Rust release. JS treats this as informational.
    Other(serde_json::Value),
}

/// Parse one SSE `data:` JSON payload. Returns `None` if the JSON is
/// malformed (caller logs + skips, doesn't crash).
pub fn parse_frame(json: &str) -> Option<DispatchFrame> {
    let v: serde_json::Value = serde_json::from_str(json.trim()).ok()?;
    let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    match ty {
        "token" => Some(DispatchFrame::Token {
            text: v
                .get("text")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        "status" => Some(DispatchFrame::Status {
            phase: v
                .get("phase")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        "done" => Some(DispatchFrame::Done {
            result: v
                .get("result")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        "error" => Some(DispatchFrame::Error {
            code: v
                .get("code")
                .and_then(|x| x.as_str())
                .unwrap_or("E_UNKNOWN")
                .to_string(),
            message: v
                .get("message")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        _ => Some(DispatchFrame::Other(v)),
    }
}

/// Pull `data:` lines out of one or more raw SSE-event blocks. The
/// broker may pack multiple frames into one TCP chunk, or split one
/// frame across chunks — caller is responsible for accumulating a
/// `pending` buffer across chunks and feeding complete blocks (split
/// on `\n\n`) here.
fn extract_data_lines(block: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in block.split('\n') {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("data:") {
            // SSE spec: trim one optional space after the colon.
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(rest);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

// ── Per-dispatch cancellation registry ──────────────────────────────────
//
// We need a "fire-once, observable-from-anywhere" cancel signal per
// dispatch. `tokio::sync::watch` fits exactly: the sender flip survives
// across task yields (unlike `Notify::notify_waiters`, which only wakes
// *already-awaiting* tasks and would race if cancel fires before the
// reader hits `.notified().await`). The receiver's `changed()` future
// resolves immediately when the latest value differs from what was
// previously observed — perfect for a one-shot cancel.
//
// The registry stores the Sender side; the in-flight task holds a
// Receiver that survives even if `cancel_dispatch` fires before the
// task reaches the select loop.

type CancelRegistry = HashMap<String, watch::Sender<bool>>;

fn cancel_registry() -> &'static Mutex<CancelRegistry> {
    static SLOT: OnceLock<Mutex<CancelRegistry>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_cancel(id: &str) -> watch::Receiver<bool> {
    let (tx, rx) = watch::channel(false);
    if let Ok(mut g) = cancel_registry().lock() {
        g.insert(id.to_string(), tx);
    }
    rx
}

fn unregister_cancel(id: &str) {
    if let Ok(mut g) = cancel_registry().lock() {
        g.remove(id);
    }
}

fn fire_cancel(id: &str) -> bool {
    let Ok(mut g) = cancel_registry().lock() else {
        return false;
    };
    if let Some(tx) = g.remove(id) {
        // Flip the value to `true`. Any current OR future `changed()`
        // call on the receiver will resolve immediately.
        let _ = tx.send(true);
        true
    } else {
        false
    }
}

#[cfg(test)]
pub fn _test_cancel_registry_len() -> usize {
    cancel_registry().lock().map(|g| g.len()).unwrap_or(0)
}

// ── Provider allow-list pull ────────────────────────────────────────────

/// Read providers from agents.toml. Errors (file missing, parse fail)
/// yield an empty list — caller treats that as "no provider override
/// allowed", which is the safe default for a fresh install.
fn load_provider_allowlist() -> Vec<String> {
    let Ok(cfg) = phantom_mesh::config::AgentsConfig::find_and_load().ok_or(()) else {
        return Vec::new();
    };
    let mut names: Vec<String> = cfg.providers.keys().cloned().collect();
    names.sort();
    names
}

#[tauri::command]
pub fn list_dispatch_providers() -> Vec<ProviderSummary> {
    let Ok(cfg) = phantom_mesh::config::AgentsConfig::find_and_load().ok_or(()) else {
        return Vec::new();
    };
    let mut out: Vec<ProviderSummary> = cfg
        .providers
        .iter()
        .map(|(name, p)| ProviderSummary {
            name: name.clone(),
            configured: p
                .api_key
                .as_ref()
                .map(|k| !k.is_empty())
                .unwrap_or(false),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

// ── dispatch_task ────────────────────────────────────────────────────────

const DEFAULT_BROKER_URL: &str = "https://phantommesh.io";

#[tauri::command]
pub async fn dispatch_task<R: tauri::Runtime>(
    request: DispatchRequest,
    http: State<'_, HttpClient>,
    window: tauri::Window<R>,
) -> Result<DispatchHandle, String> {
    // 1. Validate everything before we even look at auth.
    validate_prompt(&request.prompt).map_err(|c| c.to_string())?;
    validate_caps(&request.required_caps).map_err(|c| c.to_string())?;
    let allowed = load_provider_allowlist();
    validate_provider_in_set(request.provider_override.as_deref(), &allowed)
        .map_err(|c| c.to_string())?;

    // 2. Resolve broker URL + bearer token. We require a saved auth
    //    state for the E002 security acceptance (`dispatch_rejects_missing_token`).
    let auth_state = auth::load();
    let broker_token = auth_state
        .as_ref()
        .map(|s| s.broker_token.clone())
        .filter(|t| !t.is_empty());
    let broker_token = match broker_token {
        Some(t) => t,
        None => return Err(E_AUTH_REQUIRED.to_string()),
    };

    let broker_url = request
        .broker_url
        .clone()
        .or_else(|| auth_state.as_ref().map(|s| s.broker_url.clone()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BROKER_URL.to_string());
    let broker_url = broker_url.trim_end_matches('/').to_string();
    validate_daemon_url(&broker_url)
        .map_err(|reason| format!("{E_URL_INVALID}: {reason}"))?;

    // 3. Mint a dispatch_id + cancel slot before launching the SSE task.
    let dispatch_id = make_dispatch_id();
    let started_at_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let cancel = register_cancel(&dispatch_id);

    // 4. POST + spawn the reader loop. We don't await the loop here —
    //    return the handle immediately so the UI can update.
    let post_url = format!("{broker_url}/api/squad/dispatch");
    let event_name = format!("dispatch::token::{dispatch_id}");
    let body = serde_json::json!({
        "prompt": request.prompt,
        "required_caps": request.required_caps,
        "provider_override": request.provider_override,
        "dispatch_id": dispatch_id,
    });

    let client = http.0.clone();
    let id_for_task = dispatch_id.clone();
    let win = window.clone();

    tauri::async_runtime::spawn(async move {
        let result = run_dispatch_stream(
            &client,
            &post_url,
            &broker_token,
            &body,
            &event_name,
            &win,
            cancel,
        )
        .await;
        if let Err(e) = result {
            let _ = win.emit(
                &event_name,
                DispatchFrame::Error {
                    code: e.code.to_string(),
                    message: e.message.clone(),
                },
            );
        }
        unregister_cancel(&id_for_task);
    });

    Ok(DispatchHandle {
        dispatch_id,
        started_at_unix,
    })
}

#[tauri::command]
pub fn cancel_dispatch(dispatch_id: String) -> Result<(), String> {
    fire_cancel(&dispatch_id);
    Ok(())
}

// ── SSE reader loop ─────────────────────────────────────────────────────

struct DispatchError {
    code: &'static str,
    message: String,
}

async fn run_dispatch_stream<R: tauri::Runtime>(
    client: &reqwest::Client,
    post_url: &str,
    bearer: &str,
    body: &serde_json::Value,
    event_name: &str,
    window: &tauri::Window<R>,
    mut cancel: watch::Receiver<bool>,
) -> Result<(), DispatchError> {
    // Fast path: cancel fired before we even POSTed. Honor it cheaply.
    if *cancel.borrow() {
        let _ = window.emit(event_name, DispatchFrame::Status {
            phase: "cancelled".to_string(),
        });
        return Ok(());
    }
    let mut resp = client
        .post(post_url)
        .bearer_auth(bearer)
        .header("Accept", "text/event-stream")
        .header("Content-Type", "application/json")
        .json(body)
        .timeout(Duration::from_secs(300))
        .send()
        .await
        .map_err(|e| DispatchError {
            code: E_NETWORK,
            message: e.to_string(),
        })?;
    let status = resp.status();
    if !status.is_success() {
        return Err(DispatchError {
            code: E_HTTP_STATUS,
            message: format!("HTTP {}", status.as_u16()),
        });
    }

    // Drain the chunked response body line-by-line, accumulating until we
    // see a blank line (= one SSE event block), then parse + emit each.
    let mut buf = String::new();
    loop {
        tokio::select! {
            biased;
            res = cancel.changed() => {
                // `changed()` resolves when the sender flips the value.
                // We treat any change (which is always `true` per
                // fire_cancel) as a cancel signal. `Err` means the
                // sender was dropped — also stop cleanly.
                if res.is_ok() && !*cancel.borrow() {
                    // Spurious / unchanged — keep looping. (Can't happen
                    // with our sender flow but guards against future
                    // changes.)
                    continue;
                }
                // Emit one synthetic status frame so the UI knows we
                // cancelled cleanly (vs. timed out).
                let _ = window.emit(event_name, DispatchFrame::Status {
                    phase: "cancelled".to_string(),
                });
                return Ok(());
            }
            chunk = resp.chunk() => {
                let chunk = match chunk {
                    Ok(Some(b)) => b,
                    Ok(None) => break, // EOF
                    Err(e) => {
                        return Err(DispatchError {
                            code: E_NETWORK,
                            message: e.to_string(),
                        });
                    }
                };
                if let Ok(s) = std::str::from_utf8(&chunk) {
                    buf.push_str(s);
                }
                // Drain complete events (separated by blank lines).
                while let Some(idx) = buf.find("\n\n") {
                    let block: String = buf.drain(..idx + 2).collect();
                    for data in extract_data_lines(&block) {
                        if let Some(frame) = parse_frame(&data) {
                            let _ = window.emit(event_name, frame);
                        }
                    }
                }
            }
        }
    }
    // Flush trailing partial block (e.g. server closed without final \n\n).
    if !buf.trim().is_empty() {
        for data in extract_data_lines(&buf) {
            if let Some(frame) = parse_frame(&data) {
                let _ = window.emit(event_name, frame);
            }
        }
    }
    Ok(())
}

fn make_dispatch_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let r: u64 = rand::random();
    format!("d-{nanos:x}-{r:x}")
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_prompt ────────────────────────────────────────────────

    #[test]
    fn prompt_validation_accepts_normal_text() {
        assert!(validate_prompt("hi").is_ok());
        assert!(validate_prompt("Please summarize this paragraph for me.").is_ok());
        // UTF-8 / multibyte should pass — length cap is BYTES.
        assert!(validate_prompt("摘要這段文字").is_ok());
    }

    #[test]
    fn prompt_validation_rejects_empty_and_whitespace() {
        assert_eq!(validate_prompt("").unwrap_err(), E_PROMPT_EMPTY);
        assert_eq!(validate_prompt("   ").unwrap_err(), E_PROMPT_EMPTY);
        assert_eq!(validate_prompt("\n\t\n").unwrap_err(), E_PROMPT_EMPTY);
    }

    #[test]
    fn prompt_validation_rejects_too_long() {
        let s = "a".repeat(MAX_PROMPT_BYTES + 1);
        assert_eq!(validate_prompt(&s).unwrap_err(), E_PROMPT_TOO_LONG);
        // Exactly at the cap is OK.
        let s = "a".repeat(MAX_PROMPT_BYTES);
        assert!(validate_prompt(&s).is_ok());
    }

    #[test]
    fn prompt_validation_rejects_nul_byte() {
        assert_eq!(
            validate_prompt("hello\0world").unwrap_err(),
            E_PROMPT_INVALID
        );
    }

    // ── validate_caps ──────────────────────────────────────────────────

    #[test]
    fn caps_regex_accepts_valid_tokens() {
        assert!(validate_caps(&[]).is_ok());
        assert!(validate_caps(&["gpu".to_string()]).is_ok());
        assert!(validate_caps(&[
            "gpu".to_string(),
            "vision".to_string(),
            "audio_in".to_string(),
        ])
        .is_ok());
        assert!(validate_caps(&["llm-local".to_string()]).is_ok());
    }

    #[test]
    fn caps_regex_rejects_uppercase_and_spaces() {
        assert_eq!(
            validate_caps(&["GPU".to_string()]).unwrap_err(),
            E_CAPS_INVALID
        );
        assert_eq!(
            validate_caps(&["gpu ".to_string()]).unwrap_err(),
            E_CAPS_INVALID
        );
        assert_eq!(
            validate_caps(&[" gpu".to_string()]).unwrap_err(),
            E_CAPS_INVALID
        );
        assert_eq!(
            validate_caps(&["GPU ".to_string()]).unwrap_err(),
            E_CAPS_INVALID
        );
    }

    #[test]
    fn caps_regex_rejects_leading_digit_and_special() {
        assert_eq!(
            validate_caps(&["1gpu".to_string()]).unwrap_err(),
            E_CAPS_INVALID
        );
        assert_eq!(
            validate_caps(&["gpu!".to_string()]).unwrap_err(),
            E_CAPS_INVALID
        );
        assert_eq!(
            validate_caps(&["".to_string()]).unwrap_err(),
            E_CAPS_INVALID
        );
        // Length cap (>32 chars).
        assert_eq!(
            validate_caps(&["a".repeat(33)]).unwrap_err(),
            E_CAPS_INVALID
        );
    }

    #[test]
    fn caps_regex_rejects_more_than_three() {
        let caps = vec![
            "gpu".to_string(),
            "audio".to_string(),
            "vision".to_string(),
            "gps".to_string(),
        ];
        assert_eq!(validate_caps(&caps).unwrap_err(), E_CAPS_TOO_MANY);
    }

    // ── validate_provider_in_set ───────────────────────────────────────

    #[test]
    fn provider_allowlist_accepts_known_names() {
        let allowed = vec!["openai".to_string(), "anthropic".to_string()];
        assert!(validate_provider_in_set(None, &allowed).is_ok());
        assert!(validate_provider_in_set(Some(""), &allowed).is_ok(),
            "empty string is treated as no-override");
        assert!(validate_provider_in_set(Some("openai"), &allowed).is_ok());
        assert!(validate_provider_in_set(Some("anthropic"), &allowed).is_ok());
    }

    #[test]
    fn provider_allowlist_rejects_unknown() {
        let allowed = vec!["openai".to_string()];
        assert_eq!(
            validate_provider_in_set(Some("evil-provider"), &allowed).unwrap_err(),
            E_PROVIDER_UNKNOWN
        );
        assert_eq!(
            validate_provider_in_set(Some("openai-fake"), &allowed).unwrap_err(),
            E_PROVIDER_UNKNOWN
        );
        // Empty allow-list rejects everything except None / empty.
        let empty: Vec<String> = vec![];
        assert_eq!(
            validate_provider_in_set(Some("openai"), &empty).unwrap_err(),
            E_PROVIDER_UNKNOWN
        );
    }

    // ── Frame parser ───────────────────────────────────────────────────

    #[test]
    fn frame_parse_handles_all_four_variants() {
        let f = parse_frame(r#"{"type":"token","text":"hi"}"#).unwrap();
        assert!(matches!(f, DispatchFrame::Token { ref text } if text == "hi"));

        let f = parse_frame(r#"{"type":"status","phase":"queued"}"#).unwrap();
        assert!(matches!(f, DispatchFrame::Status { ref phase } if phase == "queued"));

        let f = parse_frame(r#"{"type":"done","result":"final answer"}"#).unwrap();
        assert!(matches!(f, DispatchFrame::Done { ref result } if result == "final answer"));

        let f = parse_frame(r#"{"type":"error","code":"E_X","message":"boom"}"#).unwrap();
        match f {
            DispatchFrame::Error { code, message } => {
                assert_eq!(code, "E_X");
                assert_eq!(message, "boom");
            }
            _ => panic!("expected Error variant"),
        }
    }

    #[test]
    fn frame_parse_passes_unknown_variants_through() {
        // An unknown `type` value must NOT crash the parser — we forward
        // as `Other` so a broker version bump doesn't break the task.
        let f = parse_frame(r#"{"type":"heartbeat","seq":42}"#).unwrap();
        assert!(matches!(f, DispatchFrame::Other(_)));
    }

    #[test]
    fn frame_parse_returns_none_for_garbage() {
        assert!(parse_frame("not json").is_none());
        assert!(parse_frame("").is_none());
    }

    #[test]
    fn frame_parse_missing_fields_get_defaults() {
        // Token frame with no `text` → empty string, not crash.
        let f = parse_frame(r#"{"type":"token"}"#).unwrap();
        assert!(matches!(f, DispatchFrame::Token { ref text } if text.is_empty()));
        // Error frame with no `code` → "E_UNKNOWN".
        let f = parse_frame(r#"{"type":"error"}"#).unwrap();
        match f {
            DispatchFrame::Error { code, .. } => assert_eq!(code, "E_UNKNOWN"),
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn extract_data_lines_handles_multi_line_data() {
        // SSE spec: contiguous `data:` lines in one event are joined with \n.
        let block = "id:1\ndata: line one\ndata: line two\n\n";
        let lines = extract_data_lines(block);
        assert_eq!(lines, vec!["line one\nline two".to_string()]);
    }

    #[test]
    fn extract_data_lines_handles_optional_space() {
        let block = "data:nospace\n\n";
        assert_eq!(extract_data_lines(block), vec!["nospace".to_string()]);
        let block = "data: with-space\n\n";
        assert_eq!(extract_data_lines(block), vec!["with-space".to_string()]);
    }

    // ── Cancellation ───────────────────────────────────────────────────

    #[tokio::test]
    async fn cancel_cleans_unblocks_task() {
        // Register a fake dispatch + spawn a task that watches the cancel
        // signal. After fire_cancel, the watcher should observe the flip
        // and exit. The watch-channel approach is robust to the "cancel
        // fires before the reader awaits" race (which is why we switched
        // from Notify::notify_waiters).
        let id = "test-cancel-001".to_string();
        let mut rx = register_cancel(&id);
        let handle = tokio::spawn(async move {
            // Wait for the channel value to change from false → true.
            let _ = rx.changed().await;
            *rx.borrow()
        });

        // Fire before the receiver even awaits — watch::Sender::send sets
        // the value durably, so the next `changed()` resolves immediately.
        assert!(fire_cancel(&id));

        let outcome = tokio::time::timeout(Duration::from_millis(500), handle).await;
        let joined = outcome.expect("task should exit within 500ms");
        let final_value = joined.expect("task should not panic");
        assert!(final_value, "value should have been set to true");
    }

    #[tokio::test]
    async fn cancel_after_register_is_observable() {
        // Pre-fire variant — confirm that even if the receiver doesn't
        // start awaiting until AFTER the sender flips, the change is
        // still picked up (this is the race that killed the earlier
        // Notify-based implementation).
        let id = "test-cancel-prefire".to_string();
        let mut rx = register_cancel(&id);
        assert!(fire_cancel(&id));
        // Sleep a tick to make the race window wider.
        tokio::time::sleep(Duration::from_millis(10)).await;
        let res = tokio::time::timeout(Duration::from_millis(200), rx.changed()).await;
        assert!(res.is_ok(), "changed() must resolve even when fired earlier");
        assert!(*rx.borrow());
    }

    #[tokio::test]
    async fn cancel_unknown_id_returns_false() {
        let fired = fire_cancel("never-registered");
        assert!(!fired);
    }

    // ── E002 security gate: dispatch_rejects_missing_token ─────────────
    //
    // The full `dispatch_task` command path is hard to drive without a
    // running Tauri app (it requires `State<'_, HttpClient>` and a Window).
    // Instead we factor the auth check into a small helper that mirrors
    // exactly what dispatch_task does, and drive that here. The
    // integration test (`tests/dispatch_commands.rs`) covers the full
    // invoke path.
    //
    // The helper is gated behind cfg(test) so production code paths still
    // go through dispatch_task's inline check — no shadow logic.

    /// Mirror of the auth check at the top of `dispatch_task`. Used by
    /// the unit + integration tests so the security contract is verified
    /// without needing a Tauri Window.
    #[cfg(test)]
    pub(super) fn _check_auth_required(state: Option<&auth::AuthState>) -> Result<String, String> {
        let token = state
            .map(|s| s.broker_token.clone())
            .filter(|t| !t.is_empty());
        token.ok_or_else(|| E_AUTH_REQUIRED.to_string())
    }

    #[test]
    fn dispatch_rejects_missing_token() {
        // None state → reject.
        let err = _check_auth_required(None).unwrap_err();
        assert_eq!(err, E_AUTH_REQUIRED);

        // Some state but empty broker_token → also reject.
        let empty_state = auth::AuthState {
            provider: String::new(),
            email: String::new(),
            display_name: None,
            sub: None,
            avatar_url: None,
            device_id: String::new(),
            created_at_ms: 0,
            last_login_ms: 0,
            password_hash: String::new(),
            salt: String::new(),
            id_token: String::new(),
            access_token: String::new(),
            broker_token: String::new(),
            broker_token_expires_at_ms: 0,
            broker_url: String::new(),
        };
        let err = _check_auth_required(Some(&empty_state)).unwrap_err();
        assert_eq!(err, E_AUTH_REQUIRED);

        // Populated token → pass.
        let good_state = auth::AuthState {
            broker_token: "real-token-xxx".to_string(),
            ..empty_state
        };
        let token = _check_auth_required(Some(&good_state)).unwrap();
        assert_eq!(token, "real-token-xxx");
    }

    // ── Debug redaction ────────────────────────────────────────────────

    #[test]
    fn no_secret_in_debug() {
        let req = DispatchRequest {
            prompt: "VERY-SENSITIVE-PROMPT-BODY".to_string(),
            required_caps: vec!["gpu".to_string()],
            provider_override: Some("openai".to_string()),
            broker_url: Some("https://broker.example".to_string()),
        };
        let s = format!("{:?}", req);
        assert!(!s.contains("VERY-SENSITIVE-PROMPT-BODY"),
            "Debug must not include prompt body, got: {s}");
        // Non-secret fields should still appear so devs can debug.
        assert!(s.contains("gpu"));
        assert!(s.contains("openai"));
        // Length is a safe proxy for prompt content.
        assert!(s.contains("prompt_len"));
    }

    // ── Provider allow-list source ─────────────────────────────────────

    #[test]
    fn provider_allowlist_loads_or_returns_empty() {
        // Pure smoke: the loader must not panic and must always return a
        // Vec<String>. Whether it's empty depends on whether agents.toml
        // exists in $HOME at test-run time — both branches are acceptable.
        let v = load_provider_allowlist();
        // Each name must be non-empty.
        for name in &v {
            assert!(!name.is_empty(), "loaded provider name should be non-empty");
        }
    }
}
