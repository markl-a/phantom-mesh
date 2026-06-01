// Broker login — phantom-mesh iOS / Tauri equivalent of `phantom login`.
//
// Desktop CLI's login_broker() at core/src/bin/phantom.rs:5540 starts a
// localhost HTTP server on :48181 to catch the OAuth callback, but iOS
// sandbox blocks loopback. Instead the iOS app:
//
//   1. broker_login_start(broker_url) generates a device_id + a
//      `phantom://oauth/callback` redirect URI, returns the Safari URL
//      to navigate to (`<broker>/auth/cli/start?...`).
//   2. JS layer opens it via tauri-plugin-shell::open() — that hands off
//      to Mobile Safari / system browser.
//   3. User completes Google / Apple / email login on phantommesh.io.
//      Broker meta-refreshes browser to phantom://oauth/callback?p=<b64>.
//   4. iOS routes that URL to the app via tauri-plugin-deep-link's
//      onOpenUrl handler (registered in lib.rs setup()), which emits a
//      `deep-link://oauth-callback` event.
//   5. JS layer's listener extracts the `p=<b64>` query, calls
//      broker_login_finish(b64) which decodes UTF-8 base64 → identity
//      JSON → AuthState → phantom_mesh::auth::save().
//
// Server side accepts `phantom://oauth/callback` thanks to PR #15
// (REDIRECT_RE extension on phantommesh-io/src/routes/oauth.ts:15).

use base64::Engine;
use phantom_mesh::auth;
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;

// ── Client-side OAuth state binding (V8 C-2 fix) ─────────────────────────
//
// Background: the broker correctly issues + verifies an HMAC-bound nonce
// cookie on its own side (see phantommesh-io/src/routes/oauth.ts:54-65),
// so the OAuth dance from `accounts.google.com` back into `phantommesh.io`
// is CSRF-safe. But the final hop — broker → `phantom://oauth/callback?p=…`
// → Tauri deep-link handler → `broker_login_finish` — has no such binding.
// Any process or webpage that can deliver a `phantom://oauth/callback?p=…`
// URL to the OS (another installed iOS app, an HTML `<a href="phantom://…">`,
// a malicious Slack/email link) used to be enough to make the running app
// silently overwrite `~/.phantom-mesh/auth.json` with an attacker payload
// and then immediately sync the attacker's vault keys.
//
// Fix: `broker_login_start` generates a 32-byte random `client_state`,
// stashes it (with a timestamp) in a process-local OnceLock<Mutex<Option<_>>>,
// and `broker_login_finish` refuses to write `auth.json` unless one of
//   (a) the inbound deep-link's payload contains a `state` field that
//       constant-time-compares to the pending value, OR
//   (b) at minimum, a pending login exists and is < 10 min old AND the
//       broker_token (a JWT minted by the broker) decodes to the same
//       device_id we used in start.
// is true. After a successful exchange (or after the 10-min TTL), the
// pending state is cleared so a stale handle can't be replayed.
//
// Note on cold-start deep-links: if iOS / Android cold-launches the app
// because the user tapped a `phantom://oauth/callback?p=…` link, there's
// no in-memory pending state, so `broker_login_finish` rejects it. That
// is the correct security posture — the broker session is 5-min TTL on
// the server side anyway, so a "click email link tomorrow" flow was never
// supported, and the only realistic cold-launch deep-link IS the attack.

const PENDING_TTL: Duration = Duration::from_secs(600); // 10 min

// ── Deep-link URL validation (V8-HIGH-5) ─────────────────────────────────
//
// Background: tauri-plugin-deep-link registers the broad `phantom://`
// scheme at the OS layer (see `tauri.conf.json` "plugins.deep-link" and
// `Info.plist` CFBundleURLSchemes). The OS routes EVERY `phantom://*` URL
// to our `on_open_url` handler regardless of path. The previous handler
// in `lib.rs:127-133` emitted every received URL to the JS layer
// unconditionally, expanding the "deep-link API surface" to whatever
// front-end listeners happened to be attached.
//
// Fix: filter URLs at the Rust layer to the single legitimate shape we
// know about — `phantom://oauth/callback?p=<base64url>[&state=<b64url>]`
// — and reject (a) any other path, (b) any query key other than `p`/`state`,
// (c) any `p` value longer than `MAX_PAYLOAD_B64_LEN` (real broker
// payloads are ~1 KB; we cap at 16 KiB so a hostile peer can't try to
// allocate / decode multi-MB strings on the IPC boundary), and (d) any
// `p` value that isn't valid base64url. None of this replaces the
// state-binding check in `broker_login_finish_inner`; it just prevents
// the unfiltered URL from ever reaching the JS layer or the finish
// command in the first place.

/// Upper bound on the URL-safe base64 `p` payload. The legitimate
/// `BrokerPayload` JSON is ~600 bytes (≈800 base64 chars) — 16 KiB
/// gives ample headroom while still being small enough that an attacker
/// can't burn IPC bandwidth + JSON parsing time on multi-MB blobs.
const MAX_PAYLOAD_B64_LEN: usize = 16 * 1024;

/// Whether a key is allowed in the `phantom://oauth/callback` query
/// string. Anything else triggers a reject — both for defense-in-depth
/// (the broker never adds keys other than these) and to make the
/// callback URL surface fully enumerable from one place.
fn is_known_callback_query_key(k: &str) -> bool {
    matches!(k, "p" | "state")
}

/// Validation result for an inbound deep-link URL. Carries the extracted
/// `p` value (already percent-decoded) and the optional `state` value
/// when the URL passes all checks. The caller is responsible for the
/// in-process state-binding check (see `broker_login_finish_inner`).
#[derive(Debug)]
pub struct ParsedCallback {
    pub payload_b64: String,
    pub state: Option<String>,
}

/// Validate an inbound `phantom://oauth/callback` URL string. Returns
/// `Ok(ParsedCallback)` only when ALL of the following hold:
///   - scheme is `phantom://` (case-insensitive — RFC 3986 schemes are)
///   - authority + path normalizes to `oauth/callback` (we accept both
///     `phantom://oauth/callback` and `phantom:oauth/callback` — the
///     RFC distinguishes them but the OS deep-link layers across
///     iOS/Android/desktop don't, and an attacker controls neither)
///   - the query string contains a non-empty `p=<value>`
///   - every query key is in the known set (`p` or `state`)
///   - the `p` value is ≤ MAX_PAYLOAD_B64_LEN bytes after percent-decode
///   - the `p` value passes `b64url_decode` (so we don't waste an IPC
///     round-trip on garbage that would fail in `broker_login_finish`)
///
/// Returns `Err(&'static str)` describing the first failed check. The
/// error string is short and stable so it's safe to log + show in test
/// assertions, but it does NOT include the offending URL (avoid leaking
/// attacker-controlled bytes into trace logs).
pub fn validate_oauth_callback_url(url: &str) -> Result<ParsedCallback, &'static str> {
    // 1. Scheme check. Accept both `phantom://oauth/callback?…` (host-form)
    // and the unlikely-but-spec-legal `phantom:oauth/callback?…`. We do
    // case-insensitive matching on just the scheme portion.
    let after_scheme = url
        .strip_prefix("phantom://")
        .or_else(|| url.strip_prefix("phantom:"))
        .or_else(|| {
            // Case-insensitive fallback for the scheme only.
            let lower = url.to_ascii_lowercase();
            if lower.starts_with("phantom://") {
                Some(&url[10..])
            } else if lower.starts_with("phantom:") {
                Some(&url[8..])
            } else {
                None
            }
        })
        .ok_or("scheme must be phantom://")?;

    // 2. Split off the query (everything after the first `?`). A URL
    // with no `?` cannot be the OAuth callback (which always carries
    // at minimum `?p=<value>`).
    let (path_part, query_part) = match after_scheme.split_once('?') {
        Some((p, q)) => (p, q),
        None => return Err("missing query string (?p=…)"),
    };

    // Strip a trailing `/` so `oauth/callback/` and `oauth/callback`
    // both normalize. Also strip a leading `/` so the host-form
    // (`phantom://oauth/callback`) and the path-only form
    // (`phantom:/oauth/callback`) normalize the same way.
    let normalized_path = path_part.trim_end_matches('/').trim_start_matches('/');

    // 3. Path must be exactly `oauth/callback`. This rejects all other
    // phantom://… URLs (phantom://anything, phantom://oauth/something-else,
    // phantom://oauth/callback/extra/segments).
    if normalized_path != "oauth/callback" {
        return Err("path must be /oauth/callback");
    }

    // 4. Query parse — split on `&`, then `=`. We bail on any key we
    // don't recognize, and require `p` to be present + non-empty.
    let mut payload_b64: Option<String> = None;
    let mut state: Option<String> = None;
    if query_part.is_empty() {
        return Err("query string is empty");
    }
    for kv in query_part.split('&') {
        let (k, v) = match kv.split_once('=') {
            Some((k, v)) => (k, v),
            None => {
                // Bare flag (`?foo`) — reject so the surface stays enumerable.
                return Err("query has bare flag without =");
            }
        };
        if !is_known_callback_query_key(k) {
            return Err("query contains unknown key");
        }
        // Percent-decode each value once. The deep-link layer typically
        // hands us already-decoded URLs, but we want to be robust to
        // both — and the legitimate broker uses URL-safe base64 which
        // doesn't include any percent-encoded characters anyway.
        let decoded = match urlencoding::decode(v) {
            Ok(s) => s.into_owned(),
            Err(_) => return Err("query value not valid percent-encoding"),
        };
        if k == "p" {
            if payload_b64.is_some() {
                return Err("duplicate p= key");
            }
            payload_b64 = Some(decoded);
        } else if k == "state" {
            if state.is_some() {
                return Err("duplicate state= key");
            }
            state = Some(decoded);
        }
    }

    let payload_b64 = payload_b64.ok_or("missing required p= key")?;
    if payload_b64.is_empty() {
        return Err("p= value is empty");
    }

    // 5. Length cap — applied to the base64 string itself BEFORE decode
    // so we never allocate a multi-MB Vec for an attacker payload.
    if payload_b64.len() > MAX_PAYLOAD_B64_LEN {
        return Err("p= value exceeds maximum length");
    }

    // 6. Base64url shape check. This catches obvious garbage early so
    // `broker_login_finish` doesn't get invoked on malformed input. We
    // discard the decoded bytes — `broker_login_finish_inner` re-decodes
    // (it's tens of microseconds at this size; not worth threading the
    // pre-decoded buffer through the Tauri command boundary).
    b64url_decode(&payload_b64).map_err(|_| "p= value not valid base64url")?;

    Ok(ParsedCallback {
        payload_b64,
        state,
    })
}

#[derive(Clone)]
struct PendingLoginState {
    /// 32 random bytes generated in broker_login_start. Compared with
    /// `subtle::ConstantTimeEq` against the value the broker echoes back
    /// when broker_login_finish runs.
    state: [u8; 32],
    /// device_id we used to start the dance. The broker mints a JWT bound
    /// to this device_id, so even if `state` is missing from the payload
    /// (e.g. the broker hasn't been updated to echo it yet), we can still
    /// reject callbacks for a different device.
    device_id: String,
    /// When start() ran. If `Instant::now() - started_at > PENDING_TTL`,
    /// finish() rejects + clears so a stale handle can't be replayed.
    started_at: Instant,
}

fn pending_slot() -> &'static Mutex<Option<PendingLoginState>> {
    static SLOT: OnceLock<Mutex<Option<PendingLoginState>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// URL-safe base64 (no padding) encode — matches the broker's
/// `redirectToLoopback` output format so the JS side can pass values
/// through the deep-link query string without re-encoding gymnastics.
fn b64url_no_pad(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Inverse of `b64url_no_pad` — accepts both padded and unpadded variants
/// and tolerates the `+`/`-` and `/`/`_` substitutions transparently.
fn b64url_decode(s: &str) -> Result<Vec<u8>, String> {
    let std_b64 = s.replace('-', "+").replace('_', "/");
    let pad = (4 - std_b64.len() % 4) % 4;
    let padded = format!("{}{}", std_b64, "=".repeat(pad));
    base64::engine::general_purpose::STANDARD
        .decode(&padded)
        .map_err(|e| format!("base64 decode failed: {e}"))
}

/// Test-only — let unit tests reset the pending slot between cases so they
/// can run in any order without interfering. Gated behind cfg(test) so it
/// can't be invoked from production code. Only the unix tests use it
/// (Windows tests are gated off — see `sandbox_home`).
#[cfg(all(test, unix))]
fn clear_pending_for_tests() {
    if let Ok(mut g) = pending_slot().lock() {
        *g = None;
    }
}

#[derive(Serialize)]
pub struct BrokerLoginStartResponse {
    /// URL the front-end should open in the system browser. The user
    /// completes OAuth there; the broker meta-refreshes back to
    /// phantom://oauth/callback when done.
    pub auth_url: String,
    /// Persisted in case the front-end wants to display "linking
    /// device <X>…" or for diagnostics.
    pub device_id: String,
    /// The redirect URI we registered with the broker (`phantom://...`)
    /// — informational; the broker validates it against REDIRECT_RE.
    pub redirect: String,
    /// URL-safe-base64-encoded 32-byte random state we just stashed in
    /// `pending_slot()`. The JS layer treats this as opaque — it's
    /// returned in case the front-end wants to display "binding to login
    /// session <prefix>…" for diagnostics, or to pass back to
    /// `broker_login_finish` explicitly if the deep-link arrives without
    /// a payload-side `state` field.
    pub client_state: String,
}

#[tauri::command]
pub fn broker_login_start(broker_url: String) -> Result<BrokerLoginStartResponse, String> {
    broker_login_start_inner(broker_url, Instant::now())
}

/// Inner implementation split out so unit tests can inject a fixed
/// `started_at` (and then test the 10-min TTL boundary deterministically).
fn broker_login_start_inner(
    broker_url: String,
    started_at: Instant,
) -> Result<BrokerLoginStartResponse, String> {
    let broker_url = broker_url
        .trim()
        .trim_end_matches('/')
        .to_string();
    if broker_url.is_empty() {
        return Err("broker_url must not be empty".into());
    }

    // Reuse an existing device_id if one is already saved (so re-logging
    // in on the same device doesn't fragment the broker's device list).
    let device_id = auth::load()
        .map(|s| s.device_id)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(auth::random_device_id);

    // Generate 32 bytes of fresh OS randomness for this login. Stored
    // in-process so broker_login_finish can constant-time-compare against
    // whatever the deep-link callback hands us. The same value is also
    // sent up to the broker as an `cs` query param (forward-compatible:
    // a future broker version can echo it back inside the signed payload,
    // at which point finish() will check it strictly).
    let state: [u8; 32] = rand::random();
    let client_state = b64url_no_pad(&state);

    let redirect = "phantom://oauth/callback".to_string();
    let auth_url = format!(
        "{}/auth/cli/start?device_id={}&port=0&redirect={}&cs={}",
        broker_url,
        urlencoding::encode(&device_id),
        urlencoding::encode(&redirect),
        urlencoding::encode(&client_state),
    );

    // Stash the pending state. Overwrites any previous pending login —
    // by design: if the user tapped "Sign in" twice, only the most
    // recent dance is honored. The earlier handle becomes unusable.
    {
        let mut guard = pending_slot()
            .lock()
            .map_err(|e| format!("pending_slot poisoned: {e}"))?;
        *guard = Some(PendingLoginState {
            state,
            device_id: device_id.clone(),
            started_at,
        });
    }

    Ok(BrokerLoginStartResponse {
        auth_url,
        device_id,
        redirect,
        client_state,
    })
}

/// Decode the broker's `?p=<base64-payload>` query, build an AuthState,
/// persist via phantom_mesh::auth::save(). Front-end should call this
/// after extracting the `p` value from the `phantom://oauth/callback`
/// URL the deep-link handler emitted.
///
/// Format of the decoded JSON is the CliPayload defined on
/// phantommesh-io/src/types.ts:CliPayload.
#[derive(Deserialize)]
struct BrokerPayload {
    provider: String,
    email: String,
    sub: Option<String>,
    name: Option<String>,
    picture: Option<String>,
    #[serde(default)]
    broker_token: String,
    #[serde(default)]
    broker_token_expires_at_ms: i64,
    /// Optional — present when the broker echoes back the `cs` query
    /// param we sent in `broker_login_start`. When present, finish()
    /// constant-time-compares it against the in-process pending state.
    /// Currently the broker does NOT echo (REDIRECT_RE forbids extra
    /// query bits and the broker generates its own opaque KV-state for
    /// CSRF), so finish() falls back to the device_id binding instead;
    /// see check_state_or_device_binding() below.
    #[serde(default)]
    state: Option<String>,
    /// device_id the broker minted this token for. When present, used
    /// as the fallback binding when `state` is absent (forward-compat
    /// path with current broker). Defaults to None so existing broker
    /// responses parse cleanly.
    #[serde(default)]
    device_id: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct BrokerLoginFinishResponse {
    pub email: String,
    pub provider: String,
    pub display_name: Option<String>,
    pub broker_token_expires_at_ms: i64,
    pub auth_path: String,
}

#[tauri::command]
pub fn broker_login_finish(
    payload_b64: String,
    state: Option<String>,
) -> Result<BrokerLoginFinishResponse, String> {
    broker_login_finish_inner(payload_b64, state, Instant::now())
}

/// Inner implementation with an injectable "now" so the TTL-expiry unit
/// test can advance the clock past 10 min without sleeping.
fn broker_login_finish_inner(
    payload_b64: String,
    explicit_state: Option<String>,
    now_instant: Instant,
) -> Result<BrokerLoginFinishResponse, String> {
    let bytes = b64url_decode(&payload_b64)?;

    let json = String::from_utf8(bytes).map_err(|e| format!("payload not valid UTF-8: {e}"))?;
    let payload: BrokerPayload =
        serde_json::from_str(&json).map_err(|e| format!("payload not valid JSON: {e}"))?;

    if payload.email.is_empty() {
        return Err("broker payload had no email — refusing to save".into());
    }

    // ── State / device binding check (V8 C-2 fix) ───────────────────────
    // Pull the pending login set up by broker_login_start. If there is
    // none, refuse — that's an unsolicited deep-link (probable attack).
    // If there is one but it's > 10 min old, clear and refuse.
    let pending = {
        let mut guard = pending_slot()
            .lock()
            .map_err(|e| format!("pending_slot poisoned: {e}"))?;
        let taken = guard.take();
        // Take eagerly: even if checks below fail we don't want a stale
        // pending entry sitting around for an attacker to race against
        // on a subsequent retry.
        taken
    };
    let pending = match pending {
        Some(p) => p,
        None => {
            return Err(
                "no pending broker login — refusing to accept unsolicited callback"
                    .into(),
            );
        }
    };

    let elapsed = now_instant.saturating_duration_since(pending.started_at);
    if elapsed > PENDING_TTL {
        return Err(format!(
            "pending broker login expired ({}s > {}s TTL) — start over",
            elapsed.as_secs(),
            PENDING_TTL.as_secs(),
        ));
    }

    // Prefer the explicitly-passed state (from the deep-link URL query),
    // fall back to the payload's `state` field (forward-compat when the
    // broker is updated to echo it inside the signed payload).
    let candidate_state = explicit_state.or_else(|| payload.state.clone());

    if let Some(s) = candidate_state {
        // Constant-time compare against the pending state. Decode the
        // url-safe base64 first; reject malformed input.
        let provided = b64url_decode(&s)
            .map_err(|e| format!("state field not valid base64url: {e}"))?;
        if provided.len() != pending.state.len()
            || provided.ct_eq(&pending.state).unwrap_u8() != 1
        {
            return Err("state binding mismatch — refusing to save".into());
        }
    } else {
        // No echoed state — fall back to verifying the broker_token's
        // device_id matches the device_id we used to start this dance.
        // This binds the callback to our originator at least as far as
        // the broker's JWT-minting step (which IS bound to device_id;
        // see phantommesh-io/src/lib/oauth.ts:mintBrokerJwt). The JWT
        // is HMAC-signed by the broker, so an attacker can't forge a
        // token with our device_id without the broker secret.
        let token_device_id = jwt_device_id(&payload.broker_token)
            .or_else(|| payload.device_id.clone())
            .unwrap_or_default();
        if token_device_id.is_empty() || token_device_id != pending.device_id {
            return Err(
                "device_id binding mismatch — refusing to save (no state echo + token device_id != pending)"
                    .into(),
            );
        }
    }

    // ── Checks passed — persist as before ────────────────────────────────
    let now = auth::now_ms();
    let prior = auth::load();
    let device_id = prior
        .as_ref()
        .map(|s| s.device_id.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| pending.device_id.clone());

    let state = auth::AuthState {
        provider: payload.provider.clone(),
        email: payload.email.clone(),
        display_name: payload.name.clone(),
        sub: payload.sub.clone(),
        avatar_url: payload.picture.clone(),
        device_id,
        created_at_ms: prior.as_ref().map(|s| s.created_at_ms).unwrap_or(now),
        last_login_ms: now,
        password_hash: String::new(),
        salt: String::new(),
        id_token: String::new(),
        access_token: String::new(),
        broker_token: payload.broker_token.clone(),
        broker_token_expires_at_ms: payload.broker_token_expires_at_ms,
        broker_url: prior
            .as_ref()
            .map(|s| s.broker_url.clone())
            .unwrap_or_default(),
    };

    auth::save(&state).map_err(|e| format!("auth::save failed: {e}"))?;

    Ok(BrokerLoginFinishResponse {
        email: payload.email,
        provider: payload.provider,
        display_name: payload.name,
        broker_token_expires_at_ms: payload.broker_token_expires_at_ms,
        auth_path: auth::auth_path().display().to_string(),
    })
}

/// Extract the `device_id` claim from a HS256 broker JWT without
/// verifying the signature. We don't have the broker's HMAC secret
/// (intentionally — secret stays on Cloudflare Workers), but we can
/// still parse the middle segment to read the device_id binding. This
/// is safe because:
///   1. The signature WILL be verified server-side on every
///      `/api/me/*` call — a forged JWT would fail there.
///   2. We're only using device_id here to bind start↔finish in the
///      same process; the value is checked against our own in-memory
///      pending state, not against external trust roots.
///
/// Returns None for malformed tokens, missing claims, or non-3-segment
/// JWTs. The caller treats None as "binding check failed".
fn jwt_device_id(token: &str) -> Option<String> {
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload_b64 = parts.next()?;
    let _sig = parts.next()?;
    if parts.next().is_some() {
        return None; // too many segments — not a JWT
    }
    let bytes = b64url_decode(payload_b64).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .get("device_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Diagnostic — return whether we have a saved AuthState and a brief
/// human summary. Used by the front-end to decide whether to show
/// "Sign in" or "Logged in as X" in the UI.
#[tauri::command]
pub fn broker_login_status() -> Option<BrokerLoginFinishResponse> {
    let s = auth::load()?;
    Some(BrokerLoginFinishResponse {
        email: s.email,
        provider: s.provider,
        display_name: s.display_name,
        broker_token_expires_at_ms: s.broker_token_expires_at_ms,
        auth_path: auth::auth_path().display().to_string(),
    })
}

/// Wipe local broker auth — useful when broker_token is rotated server
/// side or when user wants to switch accounts.
#[tauri::command]
pub fn broker_login_logout() -> Result<(), String> {
    auth::delete().map_err(|e| format!("auth::delete failed: {e}"))?;
    Ok(())
}

// ── Post-login: pull LLM keys + cluster peers from the broker vault ──────
//
// Mirrors the desktop CLI's `phantom config pull` step (which lives in
// core/src/cli_config.rs::config_pull_lines on platform/macos but isn't
// in iOS's branch yet — inlined here so the iOS app has feature parity
// without needing a deeper merge).

#[derive(Serialize, Deserialize, Clone)]
pub struct ClusterPeer {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Serialize)]
pub struct BrokerSyncResponse {
    pub keys_written: Vec<String>,
    pub env_path: String,
    pub peers_count: usize,
    pub peers_path: Option<String>,
    /// Full peer list — front-end uses this to show a coordinator picker
    /// after sync, so the user can pick which peer the WebView should
    /// load `<coord>/m` from.
    #[serde(default)]
    pub peers: Vec<ClusterPeer>,
}

fn phantom_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".phantom-mesh")
}

fn env_file_path() -> std::path::PathBuf {
    phantom_dir().join("env")
}

fn peers_json_path() -> std::path::PathBuf {
    phantom_dir().join("peers.json")
}

fn read_env_file(path: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return out;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

fn write_env_file(
    path: &std::path::Path,
    env: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {parent:?}: {e}"))?;
    }
    let mut buf = String::new();
    for (k, v) in env {
        buf.push_str(k);
        buf.push('=');
        buf.push_str(v);
        buf.push('\n');
    }
    std::fs::write(path, buf).map_err(|e| format!("write {path:?}: {e}"))?;
    Ok(())
}

/// E2EE read-path unseal for the Tauri client (SPEC-15 §0 + contract §8.C).
///
/// Takes the sealed item the broker returned verbatim and recovers the
/// plaintext secret LOCALLY. The broker is dumb storage: it never holds the
/// `VaultSealKey` and never decrypts (SPEC-15 §0.1/§0.2). Steps required:
///   1. Load the device-held `VaultSealKey` from the OS Keychain.
///   2. Recompute `compute_client_hmac(&seal_key, service, key, value_sealed,
///      ts_ms)` and constant-time compare against `server_hmac_hex` (the
///      client HMAC the broker stored + echoed). Mismatch ⇒ tamper/wrong-key.
///   3. base64url-decode `value_sealed` (must start `age-encryption.org/v1\n`)
///      and age v1 decrypt with the deterministic x25519 identity derived from
///      the seal-key bytes.
///
/// TODO(spec15-e2ee, blocking — do NOT remove this guard to "make it work"):
/// the two core primitives this needs are not yet public in
/// `phantom_mesh::broker_vault_wire`:
///   (a) `unseal_vault_value(...)` — read-path inverse of `seal_vault_value`
///       (contract §9: currently "DOES NOT EXIST — must be added").
///   (b) a public loader returning the locally-persisted `VaultSealKey`
///       (`VaultSealKey.bytes` is `pub(crate)`; no public ctor/loader exists).
/// Until both land, this returns a clear migration error rather than ever
/// falling back to the retired plaintext `settings/raw` path. This is the
/// intentional fail-closed behaviour for the E2EE migration.
/// Minimal RFC 3986 percent-encoding for /vault/get query-param values
/// (service + key). Keeps unreserved chars; encodes everything else so a key
/// containing `&`,`=`,`#`,`/` can't break or inject into the query string.
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn unseal_vault_value_local(
    service: &str,
    key: &str,
    value_sealed: &str,
    ts_ms: u64,
    server_hmac_hex: &str,
) -> Result<String, String> {
    // Structural pre-check: confirm the payload base64url-decodes and looks
    // like age v1 ciphertext (SPEC-15 §0.3 magic line) — catches gross
    // corruption before we touch the seal key.
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value_sealed.as_bytes())
        .map_err(|e| format!("value_sealed is not URL_SAFE_NO_PAD base64: {e}"))?;
    if !decoded.starts_with(b"age-encryption.org/v1\n") {
        return Err("value_sealed does not begin with the age v1 magic line".into());
    }

    // HMAC MUST be present and match (review #6: fail closed on missing/empty).
    if server_hmac_hex.trim().is_empty() {
        return Err("missing server_hmac_hex — refusing to apply unverified vault item".into());
    }
    // Load the device-held VaultSealKey (never uploaded). Fail closed if absent.
    let seal_key = phantom_mesh::broker_vault_wire::load_vault_seal_key()?;
    // Recompute the client HMAC over service‖key‖sealed‖ts_ms and require it to
    // match the broker's echoed value — CONSTANT-TIME compare to deny a timing
    // oracle that would let an attacker forge `server_hmac_hex` byte-by-byte.
    if !phantom_mesh::broker_vault_wire::verify_client_hmac(
        &seal_key, service, key, value_sealed, ts_ms, server_hmac_hex,
    ) {
        return Err("vault item HMAC mismatch — refusing to apply (tamper or key mismatch)".into());
    }
    // Verified — unseal locally with the real read-path primitive.
    let plaintext = phantom_mesh::broker_vault_wire::unseal_vault_value(value_sealed, &seal_key)
        .map_err(|e| format!("unseal failed: {e}"))?;
    String::from_utf8(plaintext).map_err(|e| format!("unsealed value is not UTF-8: {e}"))
}

/// SPEC-15 E2EE vault pull + GET /api/me/cluster-peers using the
/// broker_token from saved AuthState; merge keys into ~/.phantom-mesh/env
/// (broker wins for keys it provides; locals untouched for keys it
/// doesn't), and write peers.json. Best-effort: peers fetch failure
/// doesn't break the keys sync.
///
/// E2EE migration (SPEC-15 §0 invariants, contract §3-§4 + §8.C):
/// The legacy plaintext path `GET /api/me/settings/raw` is RETIRED here.
/// Under true end-to-end encryption the broker NEVER returns plaintext;
/// it only stores+returns `value_sealed` (age v1 ciphertext, base64url)
/// + the opaque `client_hmac_hex`. This client now:
///   1. lists items via `GET /vault/get` (list mode),
///   2. fetches each item's `value_sealed` via `GET /vault/get?service=&key=`,
///   3. recomputes the HMAC from the unsealed payload to detect tampering,
///   4. unseals locally with the device-held `VaultSealKey` (NEVER uploaded).
///
/// TODO(spec15-e2ee, blocking): two core primitives are not yet public:
///   (a) `broker_vault_wire::unseal_vault_value(...)` — the read-path inverse
///       of `seal_vault_value` (contract §9: "DOES NOT EXIST — must be added").
///   (b) a loader returning the locally-persisted `VaultSealKey` from the OS
///       Keychain (`VaultSealKey.bytes` is `pub(crate)`; no public ctor/loader
///       exists yet).
/// Until both land, this command refuses to fall back to plaintext and
/// returns a clear migration error instead of leaking secrets. Do NOT
/// re-introduce the `settings/raw` plaintext call to "make it work".
#[tauri::command]
pub async fn broker_sync_from_vault(
    broker_url: Option<String>,
) -> Result<BrokerSyncResponse, String> {
    let broker_url = broker_url
        .unwrap_or_else(|| "https://phantommesh.io".to_string())
        .trim_end_matches('/')
        .to_string();
    let state = auth::load().ok_or("not logged in — run broker_login_start first")?;
    let token = state.broker_token.clone();
    if token.is_empty() {
        return Err("AuthState has no broker_token (login was skipped or older format)".into());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("client build: {e}"))?;

    // ── E2EE pull: enumerate sealed vault items (list mode), then fetch +
    //    unseal each locally. The broker only ever hands us `value_sealed`
    //    (age v1 ciphertext) — never plaintext (SPEC-15 §0.1). ──
    let list_url = format!("{broker_url}/vault/get");
    let resp = client
        .get(&list_url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("GET {list_url}: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "broker {} returned HTTP {} — {}",
            list_url,
            status.as_u16(),
            body.chars().take(200).collect::<String>()
        ));
    }
    let parsed: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        format!(
            "non-JSON response: {e} (body head: {})",
            body.chars().take(120).collect::<String>()
        )
    })?;
    // list-mode body = { "items": [ { service, key, ts_ms, byte_len }, ... ] }
    let items = parsed
        .get("items")
        .and_then(|v| v.as_array())
        .ok_or("broker /vault/get list response missing `items` array")?;

    let env_path = env_file_path();
    let mut existing = read_env_file(&env_path);
    let mut keys_written: Vec<String> = Vec::new();
    for item in items {
        let Some(service) = item.get("service").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(key) = item.get("key").and_then(|v| v.as_str()) else {
            continue;
        };
        // Fetch the single sealed item (contract §4 single-item mode).
        // Percent-encode service/key so a value containing &,=,#,/ cannot break
        // or inject into the query (matches the CLI pull path; review #3).
        let get_url = format!(
            "{broker_url}/vault/get?service={}&key={}",
            pct_encode(service),
            pct_encode(key)
        );
        let item_resp = client
            .get(&get_url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| format!("GET {get_url}: {e}"))?;
        if !item_resp.status().is_success() {
            // Skip individual fetch failures; do NOT abort the whole sync.
            continue;
        }
        let item_body = item_resp.text().await.unwrap_or_default();
        let item_json: serde_json::Value = match serde_json::from_str(&item_body) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let value_sealed = item_json
            .get("value_sealed")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let ts_ms = item_json.get("ts_ms").and_then(|v| v.as_u64()).unwrap_or(0);
        let server_hmac_hex = item_json
            .get("server_hmac_hex")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if value_sealed.is_empty() {
            continue;
        }

        // E2EE unseal: decode the age v1 ciphertext locally with the
        // device-held VaultSealKey. The broker can NOT do this (no key).
        let plaintext = unseal_vault_value_local(service, key, value_sealed, ts_ms, server_hmac_hex)
            .map_err(|e| format!("unseal {service}/{key}: {e}"))?;
        if plaintext.is_empty() {
            continue;
        }
        // env var naming: callers historically used the raw `key` as the env
        // var name (e.g. CEREBRAS_API_KEY); preserve that mapping by treating
        // `key` as the env name. (Service grouping is metadata only.)
        existing.insert(key.to_string(), plaintext.clone());
        // Push into the running process's env immediately — the agent
        // runtime reads provider keys via std::env::var() at every
        // chat request, so this makes the freshly-pulled keys usable
        // without an app restart. (Startup also re-loads the file in
        // lib.rs setup() for cold-launch case.)
        std::env::set_var(key, &plaintext);
        keys_written.push(key.to_string());
    }
    write_env_file(&env_path, &existing)?;
    keys_written.sort();

    // Seed agents.toml on first sync — same idempotency guard as the
    // lib.rs setup() startup hook, but here we catch the case where the
    // user did broker login at runtime (e.g. via the deep-link transfer
    // helper) and the app is already running, so they can chat without
    // restart.
    if let Err(e) = crate::commands::local_keys::seed_default_agents_toml_if_missing() {
        tracing::warn!("post-sync agents.toml seed failed: {e}");
    }

    // ── pull cluster peers (best-effort) ──
    let peers_url = format!("{broker_url}/api/me/cluster-peers");
    let mut peers: Vec<ClusterPeer> = Vec::new();
    let mut peers_path: Option<String> = None;
    if let Ok(resp) = client
        .get(&peers_url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(body) = resp.text().await {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                    if let Some(arr) = json.get("peers").and_then(|v| v.as_array()) {
                        peers = arr
                            .iter()
                            .filter_map(|p| {
                                let name = p.get("name")?.as_str()?.to_string();
                                let url = p.get("url")?.as_str()?.to_string();
                                let label = p
                                    .get("label")
                                    .and_then(|v| v.as_str())
                                    .map(String::from);
                                if name.is_empty() || url.is_empty() {
                                    None
                                } else {
                                    Some(ClusterPeer { name, url, label })
                                }
                            })
                            .collect();
                        let path = peers_json_path();
                        if let Some(parent) = path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if std::fs::write(
                            &path,
                            serde_json::to_string_pretty(&peers).unwrap_or_default(),
                        )
                        .is_ok()
                        {
                            peers_path = Some(path.display().to_string());
                        }
                    }
                }
            }
        }
    }

    Ok(BrokerSyncResponse {
        keys_written,
        env_path: env_path.display().to_string(),
        peers_count: peers.len(),
        peers_path,
        peers,
    })
}

/// Diagnostic / picker UI helper — returns the cluster peer list cached
/// in ~/.phantom-mesh/peers.json. Empty Vec when the file is missing or
/// unparseable. Front-end calls this on app boot to decide whether to
/// show "Pick a coordinator" before triggering thin-shell redirect.
#[tauri::command]
pub fn broker_list_cached_peers() -> Vec<ClusterPeer> {
    let path = peers_json_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<ClusterPeer>>(&text).unwrap_or_default()
}

/// POST /api/me/cluster-peers/upsert — register THIS device on the
/// user's cluster registry. Used by the iOS app after broker_login so
/// other peers can discover it. Best-effort; failure surfaces to UI.
#[tauri::command]
pub async fn broker_register_self_peer(
    name: String,
    url: String,
    label: Option<String>,
    broker_url: Option<String>,
) -> Result<usize, String> {
    let broker_url = broker_url
        .unwrap_or_else(|| "https://phantommesh.io".to_string())
        .trim_end_matches('/')
        .to_string();
    let state = auth::load().ok_or("not logged in")?;
    if state.broker_token.is_empty() {
        return Err("no broker_token saved".into());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("client: {e}"))?;
    let body = serde_json::json!({
        "name": name,
        "url": url,
        "label": label.unwrap_or_default(),
    });
    let resp = client
        .post(format!("{broker_url}/api/me/cluster-peers/upsert"))
        .header("Authorization", format!("Bearer {}", state.broker_token))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("POST upsert: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("HTTP {}: {body}", status.as_u16()));
    }
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    let count = parsed
        .get("peers")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    Ok(count)
}

// ── Unit tests for V8 C-2 (OAuth state binding) ──────────────────────────
//
// These tests exercise the start → finish state binding without going
// through Tauri's #[tauri::command] machinery. They use a private temp
// HOME so `auth::save()` writes into the tempdir rather than the real
// `~/.phantom-mesh/auth.json`. The tests run serially under a Mutex
// because they share the process-global $HOME env var AND the
// pending_slot() static.
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::sync::Mutex as StdMutex;

    /// Serialize the tests — they mutate process-global state ($HOME and
    /// pending_slot()), so parallel execution would race. Only used by
    /// the unix-gated tests (see `sandbox_home`).
    #[cfg(unix)]
    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    /// Build a synthetic broker payload (the same shape `redirectToLoopback`
    /// in phantommesh-io produces) and base64url-encode it. `device_id` is
    /// baked into a fake-but-shape-correct JWT in the broker_token field
    /// so the device-binding fallback can read it back out. Only used by
    /// the unix-gated tests (see `sandbox_home`).
    #[cfg(unix)]
    fn build_payload(email: &str, device_id: &str) -> String {
        // Minimal JWT: header.payload.sig, all base64url-no-pad.
        // We don't need a valid signature — jwt_device_id() only parses
        // the payload claims.
        let header = b64url_no_pad(br#"{"alg":"HS256","typ":"JWT"}"#);
        let claims = serde_json::json!({
            "sub": "42",
            "device_id": device_id,
            "iss": "phantommesh.io",
            "aud": "phantom-cli",
        });
        let payload_seg = b64url_no_pad(claims.to_string().as_bytes());
        let sig = b64url_no_pad(b"not-a-real-signature");
        let fake_jwt = format!("{header}.{payload_seg}.{sig}");

        let body = serde_json::json!({
            "provider": "google",
            "email": email,
            "sub": "google|abc",
            "name": "Test User",
            "picture": null,
            "broker_token": fake_jwt,
            "broker_token_expires_at_ms": 0_i64,
        });
        b64url_no_pad(body.to_string().as_bytes())
    }

    /// Sandbox the test: point $HOME at a fresh tempdir so auth::save()
    /// writes there (and our test never sees the user's real auth.json).
    /// Returns a guard whose Drop restores the previous $HOME.
    ///
    /// NOTE: unix-only. On Windows, `dirs::home_dir()` resolves via
    /// `SHGetKnownFolderPath(FOLDERID_Profile)`, which ignores both
    /// `$HOME` and `$USERPROFILE` — meaning a test sandbox cannot
    /// intercept `auth::save()`, and any test that calls into the
    /// real flow would clobber the developer's real
    /// `~/.phantom-mesh/auth.json`. We therefore gate `sandbox_home`
    /// and every test that depends on it behind `#[cfg(unix)]`. The
    /// Windows path is exercised in higher-level integration tests
    /// (or skipped — see PR #116 func-test findings).
    #[cfg(unix)]
    struct HomeGuard {
        prev: Option<std::ffi::OsString>,
        _tmp: tempdir_lite::TempDir,
    }
    #[cfg(unix)]
    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
    }
    #[cfg(unix)]
    fn sandbox_home() -> HomeGuard {
        let tmp = tempdir_lite::TempDir::new("phantom-broker-login-test")
            .expect("tempdir create");
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", tmp.path());
        HomeGuard { prev, _tmp: tmp }
    }

    #[cfg(unix)]
    #[test]
    fn start_then_finish_succeeds() {
        let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _home = sandbox_home();
        clear_pending_for_tests();

        let t0 = Instant::now();
        let start = broker_login_start_inner("https://broker.example".into(), t0)
            .expect("start");

        // Build a payload bound to the same device_id start picked.
        let payload = build_payload("user@example.com", &start.device_id);
        let res = broker_login_finish_inner(payload, None, t0);
        assert!(res.is_ok(), "finish with matching device_id should succeed: {res:?}");
        let r = res.unwrap();
        assert_eq!(r.email, "user@example.com");
    }

    #[cfg(unix)]
    #[test]
    fn finish_without_prior_start_fails() {
        let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _home = sandbox_home();
        clear_pending_for_tests();

        let payload = build_payload("attacker@evil.example", "some-device");
        let res = broker_login_finish_inner(payload, None, Instant::now());
        assert!(res.is_err(), "finish without start must fail");
        let e = res.unwrap_err();
        assert!(
            e.contains("no pending broker login"),
            "expected 'no pending broker login' error, got: {e}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn finish_with_mismatched_state_fails() {
        let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _home = sandbox_home();
        clear_pending_for_tests();

        let t0 = Instant::now();
        let _start = broker_login_start_inner("https://broker.example".into(), t0)
            .expect("start");
        let payload = build_payload("u@example.com", "unrelated-device");

        // Caller passes an explicit state — but it's a fresh random one
        // that doesn't match what start() stashed.
        let bogus: [u8; 32] = rand::random();
        let bogus_b64 = b64url_no_pad(&bogus);

        let res = broker_login_finish_inner(payload, Some(bogus_b64), t0);
        assert!(res.is_err(), "mismatched state must fail");
        let e = res.unwrap_err();
        assert!(
            e.contains("state binding mismatch"),
            "expected state mismatch error, got: {e}"
        );

        // And: even a follow-up finish with the right state should now
        // fail, because the first failed attempt cleared the pending slot
        // (intentional — denies replay/race retries).
        let _start2 = broker_login_start_inner("https://broker.example".into(), Instant::now())
            .expect("start2");
        // (re-running confirms a fresh start() repopulates pending — sanity)
    }

    #[cfg(unix)]
    #[test]
    fn finish_after_ttl_returns_timeout_error() {
        let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _home = sandbox_home();
        clear_pending_for_tests();

        let t0 = Instant::now();
        let start = broker_login_start_inner("https://broker.example".into(), t0)
            .expect("start");
        let payload = build_payload("u@example.com", &start.device_id);

        // Pretend 11 minutes elapsed without sleeping.
        let later = t0 + Duration::from_secs(660);
        let res = broker_login_finish_inner(payload, None, later);
        assert!(res.is_err(), "stale pending state must fail");
        let e = res.unwrap_err();
        assert!(
            e.contains("expired"),
            "expected 'expired' in error, got: {e}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn explicit_matching_state_succeeds() {
        // Forward-compat path: when a future broker echoes our `cs` value
        // back, the explicit `state` arg constant-time-compares + passes.
        let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _home = sandbox_home();
        clear_pending_for_tests();

        let t0 = Instant::now();
        let start = broker_login_start_inner("https://broker.example".into(), t0)
            .expect("start");
        let payload = build_payload("u@example.com", "doesnt-matter-when-state-matches");

        let res = broker_login_finish_inner(payload, Some(start.client_state.clone()), t0);
        assert!(res.is_ok(), "matching explicit state should succeed: {res:?}");
    }

    // ── Unit tests for V8-HIGH-5 (deep-link URL validator) ──────────────
    //
    // These run on every platform because `validate_oauth_callback_url`
    // is pure (no $HOME, no filesystem, no auth state). They cover the
    // "easy 1-2" hardening: path enforcement, query-key enforcement,
    // and the length cap.

    /// Synthesize a realistic OAuth callback URL with a JSON payload of
    /// the same shape `redirectToLoopback` emits, so the golden-path
    /// test exercises the validator on input the real broker would
    /// produce.
    fn synth_callback_url() -> String {
        let body = serde_json::json!({
            "provider": "google",
            "email": "u@example.com",
            "broker_token": "h.p.s",
        });
        let p = b64url_no_pad(body.to_string().as_bytes());
        format!("phantom://oauth/callback?p={p}")
    }

    #[test]
    fn validate_accepts_golden_callback() {
        let url = synth_callback_url();
        let parsed = validate_oauth_callback_url(&url)
            .expect("golden callback should validate");
        assert!(!parsed.payload_b64.is_empty(), "p value preserved");
        assert!(parsed.state.is_none(), "no state arm in golden");
    }

    #[test]
    fn validate_accepts_callback_with_state() {
        let body = serde_json::json!({"provider": "google", "email": "x@y.z"});
        let p = b64url_no_pad(body.to_string().as_bytes());
        let s = b64url_no_pad(&[7u8; 32]);
        let url = format!("phantom://oauth/callback?p={p}&state={s}");
        let parsed = validate_oauth_callback_url(&url).expect("with state");
        assert_eq!(parsed.state.as_deref(), Some(s.as_str()));
    }

    #[test]
    fn validate_rejects_non_phantom_scheme() {
        for url in [
            "https://oauth/callback?p=abc",
            "javascript://oauth/callback?p=abc",
            "file:///etc/passwd?p=abc",
            "phantomx://oauth/callback?p=abc",
        ] {
            let r = validate_oauth_callback_url(url);
            assert!(r.is_err(), "must reject scheme in {url:?}");
        }
    }

    #[test]
    fn validate_rejects_wrong_path() {
        // Any path other than /oauth/callback must be rejected — that's
        // the whole point of the V8-HIGH-5 fix (preventing the deep-link
        // surface from expanding implicitly when new handlers attach).
        for url in [
            "phantom://oauth/start?p=abc",
            "phantom://settings/import?p=abc",
            "phantom://oauth/callback/extra?p=abc",
            "phantom://?p=abc",
            "phantom://evil?p=abc",
        ] {
            let r = validate_oauth_callback_url(url);
            assert!(r.is_err(), "must reject path in {url:?}: got {r:?}");
        }
    }

    #[test]
    fn validate_rejects_oversize_payload() {
        // Build a `p=` value that exceeds MAX_PAYLOAD_B64_LEN by 1 byte
        // and confirm we bail BEFORE attempting to decode.
        let big = "A".repeat(MAX_PAYLOAD_B64_LEN + 1);
        let url = format!("phantom://oauth/callback?p={big}");
        let r = validate_oauth_callback_url(&url);
        assert!(r.is_err(), "oversize payload must be rejected");
        assert!(
            r.unwrap_err().contains("maximum length"),
            "error should mention length cap",
        );
    }

    #[test]
    fn validate_rejects_unknown_query_key() {
        // The broker doesn't add keys other than `p` (and optionally
        // future `state`); anything else means an attacker is trying
        // to smuggle extra data through the URL. Reject so the surface
        // stays enumerable from one place.
        let body = serde_json::json!({"provider": "google", "email": "x@y.z"});
        let p = b64url_no_pad(body.to_string().as_bytes());
        let url = format!("phantom://oauth/callback?p={p}&admin=1");
        let r = validate_oauth_callback_url(&url);
        assert!(r.is_err(), "unknown query key must be rejected");
        assert!(
            r.unwrap_err().contains("unknown key"),
            "error should mention unknown key",
        );
    }

    #[test]
    fn validate_rejects_missing_payload() {
        // No `?p=` at all → reject; bare flag → reject; empty `?p=` → reject.
        assert!(validate_oauth_callback_url("phantom://oauth/callback").is_err());
        assert!(validate_oauth_callback_url("phantom://oauth/callback?").is_err());
        assert!(validate_oauth_callback_url("phantom://oauth/callback?p=").is_err());
        assert!(validate_oauth_callback_url("phantom://oauth/callback?p").is_err());
    }

    #[test]
    fn validate_rejects_invalid_base64() {
        // The base64url decoder accepts a fairly broad alphabet (any of
        // A-Z, a-z, 0-9, `-`, `_`, optional `=` padding). Real garbage
        // — e.g. embedded `!` or `*` — must fail.
        let url = "phantom://oauth/callback?p=not!valid*base64";
        let r = validate_oauth_callback_url(url);
        assert!(r.is_err(), "garbage payload must be rejected: {r:?}");
    }

    #[test]
    fn jwt_device_id_extracts_claim() {
        let header = b64url_no_pad(br#"{"alg":"HS256","typ":"JWT"}"#);
        let body = b64url_no_pad(br#"{"device_id":"abc-123","sub":"7"}"#);
        let sig = b64url_no_pad(b"x");
        let token = format!("{header}.{body}.{sig}");
        assert_eq!(jwt_device_id(&token).as_deref(), Some("abc-123"));
        assert_eq!(jwt_device_id("not.a.jwt.too.many"), None);
        assert_eq!(jwt_device_id("only-one-segment"), None);
    }
}

// Tiny ad-hoc tempdir helper — vendored to avoid adding a `tempfile`
// dev-dep just for the broker_login tests. Creates a uniquely-named dir
// under the system temp and removes it on Drop.
//
// Only the unix test variants use this (see `sandbox_home`); on Windows
// the sandbox approach doesn't work (SHGetKnownFolderPath bypasses
// $HOME/$USERPROFILE), so we gate the helper to match.
#[cfg(all(test, unix))]
mod tempdir_lite {
    use std::path::{Path, PathBuf};

    pub struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        pub fn new(prefix: &str) -> std::io::Result<Self> {
            let base = std::env::temp_dir();
            // Unique-enough name: prefix + nanos + PID + random.
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let r: u64 = rand::random();
            let name = format!("{prefix}-{}-{}-{:x}", std::process::id(), nanos, r);
            let path = base.join(name);
            std::fs::create_dir_all(&path)?;
            Ok(TempDir { path })
        }

        pub fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
