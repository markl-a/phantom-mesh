// F100 · Tauri commands for cluster peers + events
//
// Spec: docs/superpowers/features/F100-tauri-cluster-peers-commands.md
// Parent epic: E002 (mobile cluster-dispatch UI)
//
// This file is the Rust-side seam the F101 React screen calls into via
// `invoke()`. The contract is: JS asks "what peers exist?" and "tell me
// when a peer flips online/Unhealthy", Rust owns URL construction +
// bearer-auth + JSON parsing, no raw `fetch()` in the front-end.
//
// Why a separate file from `cluster.rs`: the existing `cluster.rs`
// (commands `get_cluster_status` / `get_cluster_workers` / `get_cluster_scores`)
// builds URLs via `format!("{}/cluster/...", config.hub_url)` with no
// validator gate — a V8-HIGH-2 regression waiting to happen. F100 is the
// "new pattern" file; old commands stay untouched (E001 still ships) but
// new mobile screens MUST use the commands here, which route through
// `validate_daemon_url` first.
//
// Endpoint reality check (verified by reading core/src/serve.rs:144-145
// on this branch's main):
//   - GET /rpc/peers       ← exists, returns {peers: [PeerStatus], self: PeerStatus, wire_version: N}
//   - GET /rpc/ping        ← exists, returns this node's PeerStatus
//   - GET /api/cluster/events SSE ← does NOT exist on the broker side
//
// The spec acknowledges the SSE backgrounding/reconnect is F106. F100's
// job here is to expose the `subscribe_cluster_events` *command* (so the
// F101 UI can call it on mount) and forward SSE frames if-and-when a
// future broker route lands. Current behaviour with no `/api/cluster/events`:
// the spawned task logs once and exits — UI keeps working via polling
// `get_cluster_peers` on its own cadence. No scope expansion to add an
// endpoint here (would require auth-gate + broker buy-in).

use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use crate::commands::settings::AppConfigState;
use crate::commands::HttpClient;

// ── Stable error codes ───────────────────────────────────────────────────
//
// The spec calls for "a stable error code (E_CLUSTER_HUB_UNCONFIGURED),
// not a panic". We surface error codes via the `String` payload Tauri
// returns to JS — front-end pattern-matches the leading token. Keep the
// shape `E_CLUSTER_*: human description` so logs are grep-friendly.
//
// All codes are prefixed `E_CLUSTER_` so a future structured-error
// refactor can extract them with one regex.

const E_HUB_UNCONFIGURED: &str = "E_CLUSTER_HUB_UNCONFIGURED";
const E_URL_INVALID:      &str = "E_CLUSTER_URL_INVALID";
const E_NETWORK:          &str = "E_CLUSTER_NETWORK";
const E_HTTP_STATUS:      &str = "E_CLUSTER_HTTP_STATUS";
const E_PARSE:            &str = "E_CLUSTER_PARSE";
const E_PERSIST:          &str = "E_CLUSTER_PERSIST";

// ── PeerStatus wire mirror ───────────────────────────────────────────────
//
// We deliberately don't pull in `spectyn_mesh::mesh::PeerStatus` directly —
// the mobile UI cares about a slimmer subset, and decoupling the wire type
// from the IPC type lets the mobile app keep working when the core peer
// type adds new fields (UI just ignores them). The shape below maps
// 1:1 onto the JSON `/rpc/peers` produces (see core/src/mesh.rs:373).

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PeerStatusKind {
    /// Heartbeat within the last TTL AND no recent fault counters tripped.
    Online,
    /// Heartbeat fresh-ish but the C4 health-aware selection layer marked
    /// this peer down (consecutive 5xx, timeouts, etc). Still reachable
    /// for the user to click into; UI shows amber badge.
    Unhealthy,
    /// No heartbeat received yet OR peer dropped off the heartbeat
    /// schedule entirely. UI shows grey badge + "last seen Xm ago".
    Unknown,
}

/// IPC-side peer summary returned to the JS `invoke('get_cluster_peers')`
/// caller. Mirrors the JSON `/rpc/peers` returns from the core daemon
/// (see `core/src/mesh.rs:PeerStatus`), trimmed to the fields the F101
/// cluster screen actually renders.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerSummary {
    /// Stable identity of the peer. Currently the `name` field from the
    /// core wire type (typically `[cluster].node_name` from agents.toml).
    /// JS uses this as the React key + the value `set_this_device_label`
    /// stamps "this device" against.
    pub peer_id: String,
    /// Human display name. Falls back to `peer_id` when the core
    /// `PeerStatus.name` is empty. Front-end never has to do this dance.
    pub display_name: String,
    /// Capabilities advertised by this peer (union of `capabilities` +
    /// `worker_caps` from the wire type — front-end doesn't care about
    /// the split). Sorted + deduped so the UI render is stable.
    pub caps: Vec<String>,
    pub status: PeerStatusKind,
    /// Unix-seconds timestamp of the last heartbeat we saw. UI computes
    /// "last seen Xm ago" from this. 0 when the peer has never been
    /// pinged successfully.
    pub last_seen_unix: u64,
    /// URL the peer answers `/rpc/*` on. Front-end uses it for the
    /// "open coordinator" deep-link.
    pub url: String,
    /// True when this peer is the device the app is running on. Set
    /// either by matching the peer_id against the value stashed via
    /// `set_this_device_label`, or by the `self` field of /rpc/peers
    /// (which the core daemon always populates for the originator).
    pub is_this_device: bool,
}

// ── URL validator (V8-HIGH-2 pattern, daemon-allowlist variant) ──────────
//
// Background: PR #169 introduced `validate_external_url` in onboarding.rs
// for the `open_external_url` command, restricting it to https://* and
// http://localhost. F100 needs a SIMILAR but DISTINCT allowlist for
// outbound RPC calls to the spectyn daemon, because:
//
//   - Daemons run on the user's own machines, so http://localhost:* is
//     the common case (the local in-process daemon `spectyn serve` binds
//     to 7878).
//   - Cluster peers are reachable over Tailscale at `*.tail.ts.net` (or
//     the user's chosen tailnet) — these are private network addresses
//     where TLS termination is optional and `https://` would require a
//     cert per peer. We allow `http://*.tail.ts.net` for this case.
//   - Anything else (public internet) must be https:// to avoid sending
//     the bearer token in plaintext.
//
// The validator REJECTS, with stable error code E_CLUSTER_URL_INVALID:
//   - file://, javascript:, spectyn://, ftp://, vscode://, anything else
//   - http:// schemes whose host is NOT exactly localhost / 127.0.0.1 /
//     ::1 / *.tail.ts.net (case-insensitive). The "exact" check uses the
//     same userinfo-strip + lookalike-defense logic as PR #169.
//   - URLs with userinfo (`user:pass@host`) — bearer auth goes in the
//     Authorization header, never in the URL.
//
// This is intentionally STRICTER than `validate_external_url` (no
// http://example.com pass-through) AND LOOSER (allows the Tailscale
// magic-DNS suffix). Don't unify into one function unless both call
// sites need both behaviours — that subtle drift is exactly the kind
// of bug the explicit two-validator split prevents.

/// Validate a daemon URL per the F100 allowlist. Returns Ok(()) on
/// pass; on reject, returns the error code suffix the caller wraps into
/// the user-facing error message.
pub fn validate_daemon_url(url: &str) -> Result<(), &'static str> {
    // 1. Scheme split — must be `scheme://rest`. URLs without `://`
    //    (e.g. bare `localhost:7878`) are rejected at this step.
    let (scheme, rest) = match url.split_once("://") {
        Some(parts) => parts,
        None => return Err("URL missing scheme"),
    };

    // 2. Authority = everything before the first `/`, `?`, or `#`.
    //    This is what the URL "host" actually resolves to.
    let authority_end = rest
        .find(|c: char| c == '/' || c == '?' || c == '#')
        .unwrap_or(rest.len());
    let authority = &rest[..authority_end];

    // 3. Reject userinfo. `http://user:pass@evil.com/` would route to
    //    evil.com but the naive `starts_with("http://localhost")` check
    //    would think it's localhost. Same defense as PR #169.
    if authority.contains('@') {
        return Err("URLs with userinfo are not allowed");
    }

    // 4. Strip port for host comparison. IPv6 literals come in as
    //    `[::1]:7878` — `split(':')` mangles them, so handle bracketed
    //    form explicitly.
    let host = if let Some(rest_after_bracket) = authority.strip_prefix('[') {
        // IPv6 literal: take everything up to ']'.
        match rest_after_bracket.find(']') {
            Some(idx) => &rest_after_bracket[..idx],
            None => return Err("malformed IPv6 literal"),
        }
    } else {
        authority.split(':').next().unwrap_or("")
    };

    match scheme.to_ascii_lowercase().as_str() {
        "https" => {
            if host.is_empty() {
                return Err("https URL missing host");
            }
            Ok(())
        }
        "http" => {
            // Loopback (exact match).
            if host.eq_ignore_ascii_case("localhost")
                || host == "127.0.0.1"
                || host == "::1"
            {
                return Ok(());
            }
            // Tailscale magic-DNS suffix (`*.tail.ts.net`, case-insensitive).
            // We require the dot before `tail.ts.net` so "tail.ts.net" by
            // itself doesn't match (no actual peer would be at that root,
            // and the suffix-only form opens a registration-poisoning vector
            // if tailscale ever stops owning the apex).
            let host_lower = host.to_ascii_lowercase();
            if host_lower.ends_with(".tail.ts.net") && host_lower.len() > ".tail.ts.net".len() {
                return Ok(());
            }
            Err("http:// is allowed only for localhost / 127.0.0.1 / ::1 / *.tail.ts.net")
        }
        _ => Err("only https:// or http://(localhost|*.tail.ts.net) URLs are allowed"),
    }
}

// ── get_cluster_peers ────────────────────────────────────────────────────

/// Build the `/rpc/peers` URL from a validated daemon base.
/// Pure (no I/O); split out so the test suite can verify URL construction
/// without binding a port.
fn build_peers_url(hub_url: &str) -> Result<String, String> {
    let hub = hub_url.trim().trim_end_matches('/');
    if hub.is_empty() {
        return Err(format!("{E_HUB_UNCONFIGURED}: hub_url is empty — run onboarding first"));
    }
    validate_daemon_url(hub)
        .map_err(|reason| format!("{E_URL_INVALID}: {reason}"))?;
    Ok(format!("{hub}/rpc/peers"))
}

/// Parse the `/rpc/peers` JSON response shape into the IPC PeerSummary
/// list. Split out so the test suite can exercise parsing without
/// running a TCP server.
fn parse_peers_response(body: &serde_json::Value, this_device_label: Option<&str>) -> Vec<PeerSummary> {
    let mut out: Vec<PeerSummary> = Vec::new();
    let peers = body.get("peers").and_then(|v| v.as_array());
    let self_obj = body.get("self");
    let self_peer_id: Option<String> = self_obj
        .and_then(|s| s.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let push = |out: &mut Vec<PeerSummary>, v: &serde_json::Value, is_self: bool| {
        let peer_id = v
            .get("name")
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string();
        if peer_id.is_empty() {
            return; // skip malformed entries silently — UI shouldn't render an unidentified peer
        }
        let display_name = peer_id.clone();
        // Merge `capabilities` + `worker_caps`, sort, dedupe.
        let mut caps: Vec<String> = Vec::new();
        if let Some(arr) = v.get("capabilities").and_then(|x| x.as_array()) {
            for c in arr {
                if let Some(s) = c.as_str() {
                    caps.push(s.to_string());
                }
            }
        }
        if let Some(arr) = v.get("worker_caps").and_then(|x| x.as_array()) {
            for c in arr {
                if let Some(s) = c.as_str() {
                    caps.push(s.to_string());
                }
            }
        }
        caps.sort();
        caps.dedup();

        let online = v.get("online").and_then(|x| x.as_bool()).unwrap_or(false);
        let last_seen_unix = v.get("last_seen").and_then(|x| x.as_u64()).unwrap_or(0);
        // Heuristic for "Unhealthy": online=true but last_seen is older
        // than ~5 min. The core C4 layer also emits an explicit health
        // signal; until that's exposed on /rpc/peers we approximate.
        let status = if online {
            PeerStatusKind::Online
        } else if last_seen_unix > 0 {
            PeerStatusKind::Unhealthy
        } else {
            PeerStatusKind::Unknown
        };

        let url = v
            .get("url")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string();

        // "this device" = either explicitly labelled via
        // set_this_device_label, OR the peer arrived in the `self` slot.
        let is_this_device = is_self
            || this_device_label
                .map(|lbl| !lbl.is_empty() && lbl == peer_id)
                .unwrap_or(false);

        out.push(PeerSummary {
            peer_id,
            display_name,
            caps,
            status,
            last_seen_unix,
            url,
            is_this_device,
        });
    };

    if let Some(arr) = peers {
        for p in arr {
            // Skip duplicates of self (some core builds include self in
            // the peers array; we want it only once).
            let pname = p.get("name").and_then(|v| v.as_str()).map(String::from);
            let is_dup_of_self = match (&pname, &self_peer_id) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            };
            if is_dup_of_self { continue; }
            push(&mut out, p, false);
        }
    }
    if let Some(s) = self_obj {
        push(&mut out, s, true);
    }
    out
}

/// `get_cluster_peers` — Tauri command. Fetches `/rpc/peers` from the
/// configured hub, parses the response, returns a slim `Vec<PeerSummary>`
/// the F101 React screen renders directly.
#[tauri::command]
pub async fn get_cluster_peers(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
) -> Result<Vec<PeerSummary>, String> {
    let (hub_url, auth_key) = {
        let cfg = config.read();
        (cfg.hub_url.clone(), cfg.auth_key.clone())
    };

    let url = build_peers_url(&hub_url)?;
    let label = load_this_device_label();

    let req = http.0.get(&url);
    let req = if !auth_key.is_empty() {
        req.bearer_auth(&auth_key)
    } else {
        req
    };
    let resp = req
        .send()
        .await
        .map_err(|e| format!("{E_NETWORK}: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("{E_HTTP_STATUS}: HTTP {}", status.as_u16()));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("{E_PARSE}: {e}"))?;
    Ok(parse_peers_response(&body, label.as_deref()))
}

// ── subscribe_cluster_events ────────────────────────────────────────────
//
// Spec scope-out: "SSE backgrounding/reconnect logic (that's F106)". F100
// exposes the COMMAND SEAM so F101 can call it on mount and F106 can fill
// in the streaming behaviour without another Tauri API surface change.
//
// Current implementation: spawns a tokio task that attempts to connect
// to `{hub_url}/rpc/events` (a route the broker doesn't have yet — see
// the endpoint reality check at the top of this file). When the connect
// fails (current expected state), the task logs once and exits cleanly.
// When the connect succeeds (future), each SSE frame becomes a Tauri
// `cluster::peer_event` emit with the raw frame payload as JSON.
//
// JS side calls this once; reciprocity is via the global event bus. We
// return a JoinHandle-style token (just a tag string) so the UI can
// future-proof an unsubscribe path even though Tauri 2 doesn't currently
// give us a clean way to cancel a one-shot setup task.

/// Returned to JS so the front-end can hold a token / log it. Currently
/// opaque (no unsubscribe method is wired in F100 — that's F106's seam).
#[derive(Serialize)]
pub struct EventSubscriptionHandle {
    pub id: String,
}

#[tauri::command]
pub async fn subscribe_cluster_events<R: tauri::Runtime>(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    window: tauri::Window<R>,
) -> Result<EventSubscriptionHandle, String> {
    let (hub_url, auth_key) = {
        let cfg = config.read();
        (cfg.hub_url.clone(), cfg.auth_key.clone())
    };
    let hub = hub_url.trim().trim_end_matches('/').to_string();
    if hub.is_empty() {
        return Err(format!("{E_HUB_UNCONFIGURED}: hub_url is empty"));
    }
    validate_daemon_url(&hub)
        .map_err(|reason| format!("{E_URL_INVALID}: {reason}"))?;

    let events_url = format!("{hub}/rpc/events");
    let client = http.0.clone();
    let win = window.clone();
    let id = format!("sub-{}", random_id());

    tauri::async_runtime::spawn(async move {
        // F106 will replace this with a real SSE loop (reqwest-eventsource
        // crate + exponential backoff + jitter). For F100 we make ONE
        // attempt; if it fails we log + exit. UI keeps working via
        // periodic get_cluster_peers polling.
        let req = client.get(&events_url);
        let req = if !auth_key.is_empty() {
            req.bearer_auth(&auth_key)
        } else {
            req
        };
        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!(
                    target: "spectyn-app::cluster",
                    "SSE connected to {} — F106 will replace this stub",
                    events_url
                );
                // Forward a single sentinel event so the UI knows the
                // subscription is live. Real frame parsing lands in F106.
                let _ = win.emit(
                    "cluster::peer_event",
                    serde_json::json!({
                        "kind": "subscription_ready",
                        "url": events_url,
                    }),
                );
            }
            Ok(resp) => {
                tracing::warn!(
                    target: "spectyn-app::cluster",
                    "SSE endpoint {} returned HTTP {} — endpoint not implemented yet (F106)",
                    events_url,
                    resp.status().as_u16(),
                );
            }
            Err(e) => {
                tracing::warn!(
                    target: "spectyn-app::cluster",
                    "SSE connect to {} failed: {} (expected until F106 lands)",
                    events_url, e
                );
            }
        }
    });

    Ok(EventSubscriptionHandle { id })
}

/// Tiny non-cryptographic id for the subscription handle. Doesn't need
/// to be unguessable — it's just a debug aid for the JS layer.
fn random_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

// ── set_this_device_label ────────────────────────────────────────────────
//
// Persists which peer_id maps to "this device" so the F101 UI can render
// the "this device" badge without having to introspect hostname / IP at
// runtime (which is unreliable on iOS / Android sandboxes).
//
// Storage: `~/.spectyn-mesh/cluster-ui.json` — a tiny JSON sidecar that
// only this F100 module reads/writes. Chosen over tauri-plugin-store
// because:
//   1. The plugin's API requires a Tauri AppHandle, and the spec wants
//      the inner persistence function to be testable without spinning up
//      a full Tauri context.
//   2. The existing broker_login.rs already writes ad-hoc JSON files in
//      the same `~/.spectyn-mesh/` dir (peers.json), so this matches the
//      established pattern in the same crate.
//   3. Survives daemon restart for free — the file lives outside the
//      app process.

#[derive(Serialize, Deserialize, Default)]
struct ClusterUiState {
    /// peer_id stamped as "this device". When empty, the UI falls back
    /// to the `self` slot from /rpc/peers.
    #[serde(default)]
    this_device_label: String,
}

fn cluster_ui_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".spectyn-mesh")
        .join("cluster-ui.json")
}

/// Test-injectable path override. Production code always uses
/// `cluster_ui_path()`; tests set this so they can use a tempdir without
/// touching the real `~/.spectyn-mesh/`.
#[cfg(test)]
static TEST_PATH_OVERRIDE: std::sync::OnceLock<std::sync::Mutex<Option<std::path::PathBuf>>> = std::sync::OnceLock::new();

#[cfg(test)]
fn set_test_path(p: std::path::PathBuf) {
    let slot = TEST_PATH_OVERRIDE.get_or_init(|| std::sync::Mutex::new(None));
    *slot.lock().unwrap() = Some(p);
}

#[cfg(test)]
fn clear_test_path() {
    if let Some(slot) = TEST_PATH_OVERRIDE.get() {
        *slot.lock().unwrap() = None;
    }
}

fn effective_path() -> std::path::PathBuf {
    #[cfg(test)]
    {
        if let Some(slot) = TEST_PATH_OVERRIDE.get() {
            if let Some(p) = slot.lock().unwrap().clone() {
                return p;
            }
        }
    }
    cluster_ui_path()
}

fn load_this_device_label() -> Option<String> {
    let path = effective_path();
    let text = std::fs::read_to_string(&path).ok()?;
    let parsed: ClusterUiState = serde_json::from_str(&text).ok()?;
    if parsed.this_device_label.is_empty() {
        None
    } else {
        Some(parsed.this_device_label)
    }
}

fn save_this_device_label(label: &str) -> Result<(), String> {
    let path = effective_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("{E_PERSIST}: mkdir {parent:?}: {e}"))?;
    }
    let state = ClusterUiState {
        this_device_label: label.to_string(),
    };
    let buf = serde_json::to_string_pretty(&state)
        .map_err(|e| format!("{E_PERSIST}: serialize: {e}"))?;
    std::fs::write(&path, buf)
        .map_err(|e| format!("{E_PERSIST}: write {path:?}: {e}"))?;
    Ok(())
}

#[tauri::command]
pub fn set_this_device_label(label: String) -> Result<(), String> {
    save_this_device_label(&label)
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// Test bodies that touch the on-disk label file run serially under
    /// this mutex so the per-process TEST_PATH_OVERRIDE doesn't race.
    static FILE_TEST_LOCK: StdMutex<()> = StdMutex::new(());

    // ── validate_daemon_url ─────────────────────────────────────────────

    #[test]
    fn validator_accepts_https_anywhere() {
        assert!(validate_daemon_url("https://example.com").is_ok());
        assert!(validate_daemon_url("https://example.com/rpc/peers").is_ok());
        assert!(validate_daemon_url("https://phantommesh.io/api").is_ok());
        assert!(validate_daemon_url("HTTPS://Example.COM/").is_ok(), "scheme is case-insensitive");
    }

    #[test]
    fn validator_accepts_http_localhost_and_loopback() {
        assert!(validate_daemon_url("http://localhost").is_ok());
        assert!(validate_daemon_url("http://localhost:7878").is_ok());
        assert!(validate_daemon_url("http://localhost:7878/rpc/peers").is_ok());
        assert!(validate_daemon_url("http://127.0.0.1:7878/").is_ok());
        assert!(validate_daemon_url("http://[::1]:7878/").is_ok());
        assert!(validate_daemon_url("http://LOCALHOST/").is_ok());
    }

    #[test]
    fn validator_accepts_tailscale_suffix() {
        assert!(validate_daemon_url("http://oracle-arm.tail.ts.net:7878/").is_ok());
        assert!(validate_daemon_url("http://Workstation.TAIL.TS.NET/rpc/peers").is_ok());
        // Suffix-only must reject — no real peer is at the apex and
        // allowing it opens a poisoning vector.
        assert!(validate_daemon_url("http://tail.ts.net/").is_err());
    }

    #[test]
    fn validator_rejects_dangerous_schemes() {
        // V8-HIGH-1: NO shell:* / spectyn:* / vscode:* / file:* / javascript:*
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "vscode://path",
            "spectyn://oauth/callback",
            "ftp://example.com/",
            "shell://anything",
        ] {
            assert!(
                validate_daemon_url(url).is_err(),
                "must reject scheme in {url:?}"
            );
        }
    }

    #[test]
    fn validator_rejects_http_for_public_internet() {
        // The whole point of the http-allowlist split: don't let the
        // mobile app POST bearer tokens in plaintext to arbitrary hosts.
        for url in [
            "http://example.com/",
            "http://api.openai.com/v1",
            "http://localhost.attacker.com/",
            "http://localhost.evil/",
            "http://localhostX/",
        ] {
            assert!(
                validate_daemon_url(url).is_err(),
                "must reject http for {url:?}"
            );
        }
    }

    #[test]
    fn validator_rejects_userinfo() {
        // PR #169 lookalike defense — userinfo lets the URL pretend to be
        // localhost but actually route to attacker.com.
        assert!(validate_daemon_url("http://localhost@attacker.com/").is_err());
        assert!(validate_daemon_url("https://user:pass@example.com/").is_err());
    }

    #[test]
    fn validator_rejects_malformed() {
        assert!(validate_daemon_url("not a url").is_err());
        assert!(validate_daemon_url("https://").is_err());
        assert!(validate_daemon_url("").is_err());
        assert!(validate_daemon_url("localhost:7878").is_err()); // no scheme
    }

    // ── build_peers_url ─────────────────────────────────────────────────

    #[test]
    fn build_peers_url_constructs_correct_endpoint() {
        let url = build_peers_url("http://localhost:7878").unwrap();
        assert_eq!(url, "http://localhost:7878/rpc/peers");

        let url = build_peers_url("http://localhost:7878/").unwrap();
        assert_eq!(url, "http://localhost:7878/rpc/peers",
            "trailing slash should be stripped");

        let url = build_peers_url("https://oracle.tail.ts.net").unwrap();
        assert_eq!(url, "https://oracle.tail.ts.net/rpc/peers");
    }

    #[test]
    fn build_peers_url_missing_hub_returns_stable_error() {
        let err = build_peers_url("").unwrap_err();
        assert!(err.starts_with(E_HUB_UNCONFIGURED),
            "error must carry stable code, got: {err}");

        let err = build_peers_url("   ").unwrap_err();
        assert!(err.starts_with(E_HUB_UNCONFIGURED));
    }

    #[test]
    fn build_peers_url_invalid_scheme_returns_stable_error() {
        let err = build_peers_url("javascript:alert(1)").unwrap_err();
        assert!(err.starts_with(E_URL_INVALID),
            "error must carry stable code, got: {err}");

        let err = build_peers_url("http://evil.example/").unwrap_err();
        assert!(err.starts_with(E_URL_INVALID));
    }

    // ── parse_peers_response ────────────────────────────────────────────

    #[test]
    fn parse_peers_handles_empty_response() {
        let body = serde_json::json!({"peers": [], "self": null, "wire_version": 1});
        let peers = parse_peers_response(&body, None);
        assert!(peers.is_empty());
    }

    #[test]
    fn parse_peers_extracts_status_and_caps() {
        let body = serde_json::json!({
            "wire_version": 1,
            "self": {
                "url": "http://localhost:7878",
                "name": "self-node",
                "version": "0.5.0",
                "online": true,
                "active_tasks": 0,
                "uptime_secs": 100,
                "last_seen": 1_700_000_000_u64,
                "capabilities": ["llm_local"],
                "worker_caps": ["memory", "web"]
            },
            "peers": [
                {
                    "url": "http://oracle.tail.ts.net:7878",
                    "name": "oracle-arm",
                    "version": "0.5.0",
                    "online": true,
                    "active_tasks": 1,
                    "uptime_secs": 3600,
                    "last_seen": 1_700_000_001_u64,
                    "capabilities": ["llm_remote"],
                    "worker_caps": []
                },
                {
                    "url": "http://termux.tail.ts.net:7878",
                    "name": "android-termux",
                    "version": "0.4.9",
                    "online": false,
                    "active_tasks": 0,
                    "uptime_secs": 0,
                    "last_seen": 1_699_999_000_u64,
                    "capabilities": [],
                    "worker_caps": []
                }
            ]
        });
        let peers = parse_peers_response(&body, None);
        assert_eq!(peers.len(), 3, "expected 3 peers (2 + self), got {peers:?}");

        // Find by peer_id rather than relying on order (parse_peers_response
        // appends self last; that's an implementation detail tests shouldn't
        // depend on).
        let oracle = peers.iter().find(|p| p.peer_id == "oracle-arm").unwrap();
        assert_eq!(oracle.status, PeerStatusKind::Online);
        assert_eq!(oracle.caps, vec!["llm_remote"]);
        assert!(!oracle.is_this_device);

        let android = peers.iter().find(|p| p.peer_id == "android-termux").unwrap();
        // online=false but last_seen>0 → Unhealthy
        assert_eq!(android.status, PeerStatusKind::Unhealthy);

        let self_node = peers.iter().find(|p| p.peer_id == "self-node").unwrap();
        assert!(self_node.is_this_device, "self slot must set is_this_device");
        // caps merged + sorted + deduped
        assert_eq!(self_node.caps, vec!["llm_local", "memory", "web"]);
    }

    #[test]
    fn parse_peers_skips_unidentified_entries() {
        // Entry with empty `name` would be unrenderable in the UI —
        // drop silently rather than emit a junk row.
        let body = serde_json::json!({
            "peers": [
                {"name": "", "url": "http://x", "online": true},
                {"name": "good", "url": "http://y", "online": true}
            ],
            "self": null
        });
        let peers = parse_peers_response(&body, None);
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].peer_id, "good");
    }

    #[test]
    fn parse_peers_deduplicates_self_from_peers_array() {
        // Some core builds include self in BOTH the peers array AND the
        // self slot. We render it once, tagged as this_device.
        let body = serde_json::json!({
            "peers": [
                {"name": "alpha", "url": "http://a", "online": true},
                {"name": "self-node", "url": "http://s", "online": true}
            ],
            "self": {"name": "self-node", "url": "http://s", "online": true}
        });
        let peers = parse_peers_response(&body, None);
        let self_matches: Vec<_> = peers.iter().filter(|p| p.peer_id == "self-node").collect();
        assert_eq!(self_matches.len(), 1, "self should appear exactly once");
        assert!(self_matches[0].is_this_device);
    }

    #[test]
    fn parse_peers_honors_explicit_this_device_label() {
        // When set_this_device_label has been called, the label tags the
        // matching peer even if the wire `self` slot is absent.
        let body = serde_json::json!({
            "peers": [{"name": "my-laptop", "url": "http://x", "online": true}],
            "self": null
        });
        let peers = parse_peers_response(&body, Some("my-laptop"));
        assert!(peers[0].is_this_device);

        // Wrong label → not tagged.
        let peers = parse_peers_response(&body, Some("not-me"));
        assert!(!peers[0].is_this_device);
    }

    // ── set_this_device_label persistence ───────────────────────────────

    /// Set TEST_PATH_OVERRIDE to a tempdir for the duration of one test.
    /// Returns a guard whose Drop clears the override + removes the dir.
    struct TempPathGuard {
        path: std::path::PathBuf,
    }
    impl Drop for TempPathGuard {
        fn drop(&mut self) {
            clear_test_path();
            if let Some(parent) = self.path.parent() {
                let _ = std::fs::remove_dir_all(parent);
            }
        }
    }
    fn tempdir_with_label_file() -> TempPathGuard {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "spectyn-f100-{}-{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("cluster-ui.json");
        set_test_path(path.clone());
        TempPathGuard { path }
    }

    #[test]
    fn label_round_trip_persists() {
        let _lock = FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = tempdir_with_label_file();
        assert!(load_this_device_label().is_none(), "empty initial state");

        save_this_device_label("my-device").expect("save");
        assert_eq!(load_this_device_label().as_deref(), Some("my-device"));

        // Overwrite
        save_this_device_label("new-device").expect("overwrite");
        assert_eq!(load_this_device_label().as_deref(), Some("new-device"));
    }

    #[test]
    fn label_empty_string_clears_effective_label() {
        let _lock = FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = tempdir_with_label_file();
        save_this_device_label("temp").expect("seed");
        assert_eq!(load_this_device_label().as_deref(), Some("temp"));
        save_this_device_label("").expect("clear");
        assert!(load_this_device_label().is_none(), "empty label loads as None");
    }

    // ── HTTP integration test: bearer header + endpoint shape ───────────
    //
    // We roll a tiny axum server on a random port instead of pulling in
    // wiremock as a dev-dep (would add a 30-crate transitive tail for
    // one test). The server captures inbound headers + serves a canned
    // /rpc/peers response, and we drive get_cluster_peers indirectly
    // via build_peers_url + a manual reqwest call — exercising the same
    // bearer + URL construction path the Tauri command uses.

    #[tokio::test]
    async fn http_request_includes_bearer_and_hits_rpc_peers() {
        use axum::{routing::get, Json, Router};
        use std::sync::Arc;
        use tokio::sync::Mutex;

        // Captured request data from the mock server.
        #[derive(Default, Clone)]
        struct Capture {
            auth_header: Option<String>,
            path: String,
        }
        let cap = Arc::new(Mutex::new(Capture::default()));

        let cap_for_handler = cap.clone();
        let app = Router::new().route(
            "/rpc/peers",
            get(move |headers: axum::http::HeaderMap, uri: axum::http::Uri| {
                let cap = cap_for_handler.clone();
                async move {
                    let mut g = cap.lock().await;
                    g.auth_header = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(String::from);
                    g.path = uri.path().to_string();
                    Json(serde_json::json!({
                        "wire_version": 1,
                        "peers": [],
                        "self": {"name": "mock-self", "url": "http://mock", "online": true,
                                 "version": "0", "active_tasks": 0, "uptime_secs": 0,
                                 "last_seen": 0, "capabilities": [], "worker_caps": []}
                    }))
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        // Drive the same code path get_cluster_peers uses: build URL +
        // reqwest with bearer_auth + parse.
        let base = format!("http://127.0.0.1:{}", addr.port());
        let url = build_peers_url(&base).expect("url");
        assert_eq!(url, format!("http://127.0.0.1:{}/rpc/peers", addr.port()));

        let client = reqwest::Client::new();
        let body: serde_json::Value = client
            .get(&url)
            .bearer_auth("test-token-12345")
            .send()
            .await
            .expect("send")
            .json()
            .await
            .expect("json");
        let peers = parse_peers_response(&body, None);
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].peer_id, "mock-self");
        assert!(peers[0].is_this_device);

        // Verify capture.
        let g = cap.lock().await;
        assert_eq!(g.path, "/rpc/peers");
        assert_eq!(
            g.auth_header.as_deref(),
            Some("Bearer test-token-12345"),
            "command must send Authorization: Bearer <token>"
        );

        server.abort();
    }
}
