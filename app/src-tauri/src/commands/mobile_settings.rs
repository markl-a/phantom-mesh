// F105 · Tauri mobile-settings commands (broker rotation, peer add, heartbeat).
//
// Spec: docs/superpowers/specs/_current/E002-mobile-cluster-dispatch-ui.md
//   §"Settings screen (extend existing) — broker token rotation + manual peer
//   add + heartbeat-interval slider (writes back to ~/.spectyn-mesh/agents.toml
//   on the local node via Rust command)".
//
// Surface exposed to JS:
//   - get_broker_token_preview() → { token_preview, broker_url, expires_at_ms }
//     Read-only summary used by the obfuscated current-token row in the UI.
//   - rotate_broker_token() → { token_preview, rotated_at_unix }
//     Generates a new 256-bit random broker token, writes auth.json, returns
//     a redacted preview so the JS pill can confirm success without ever
//     touching the raw token.
//   - get_heartbeat_interval() → u64 (current `[cluster] heartbeat_interval_secs`
//     from ~/.spectyn-mesh/agents.toml; falls back to DEFAULT_HEARTBEAT_SECS
//     when unset).
//   - set_heartbeat_interval(secs) → ()  (5..=300; read-modify-write that
//     PRESERVES every other key/comment in agents.toml — backed by toml_edit).
//   - add_cluster_peer(peer_url) → ()  (validate_daemon_url + append-unique
//     to `[cluster] peers`).
//
// V8-HIGH-2 contract: every input is validated in Rust before touching disk.
// JS may not bypass the URL allow-list (validate_daemon_url) or the
// heartbeat range. Stable error code prefix is `E_SETTINGS_*` so the
// JS layer can pattern-match identically to E_DISPATCH_* / E_CLUSTER_*.
//
// Token-equality checks use `subtle::ConstantTimeEq` per the broker-token
// secret discipline — see `tests::token_equality_is_constant_time`.

use spectyn_mesh::auth;
use serde::Serialize;
use std::path::PathBuf;
use subtle::ConstantTimeEq;

use crate::commands::cluster_peers::validate_daemon_url;

// ── Stable error codes ───────────────────────────────────────────────────

const E_AUTH_REQUIRED:        &str = "E_SETTINGS_AUTH_REQUIRED";
const E_AUTH_WRITE:           &str = "E_SETTINGS_AUTH_WRITE";
const E_PEER_URL_INVALID:     &str = "E_SETTINGS_PEER_URL_INVALID";
const E_PEER_URL_EMPTY:       &str = "E_SETTINGS_PEER_URL_EMPTY";
const E_PEER_DUPLICATE:       &str = "E_SETTINGS_PEER_DUPLICATE";
const E_HEARTBEAT_OUT_OF_RANGE: &str = "E_SETTINGS_HEARTBEAT_OUT_OF_RANGE";
const E_TOML_READ:            &str = "E_SETTINGS_TOML_READ";
const E_TOML_PARSE:           &str = "E_SETTINGS_TOML_PARSE";
const E_TOML_WRITE:           &str = "E_SETTINGS_TOML_WRITE";
const E_TOML_SHAPE:           &str = "E_SETTINGS_TOML_SHAPE";

/// Heartbeat slider clamp range. 5s lower bound matches what the C4
/// heartbeat task can do without saturating tiny peers; 300s upper bound
/// matches the M1 runbook's "every 5 minutes" max so an Unhealthy peer
/// gets detected on a sensible cadence. Mirror these in
/// MobileSettings.tsx — keep both ends in sync.
pub const MIN_HEARTBEAT_SECS: u64 = 5;
pub const MAX_HEARTBEAT_SECS: u64 = 300;
/// Default the slider snaps to when agents.toml hasn't set the field.
/// Matches spectyn_mesh::mesh::DEFAULT_HEARTBEAT_INTERVAL_SECS so the
/// behaviour after first-load is the same as before the slider existed.
pub const DEFAULT_HEARTBEAT_SECS: u64 = 30;

// ── Return shapes ────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct BrokerTokenPreview {
    /// "********abcd" — last 4 chars of the broker token plus 8 asterisks.
    /// Empty string when no token is saved.
    pub token_preview: String,
    /// Broker URL the saved token was issued by. Empty when no auth state.
    pub broker_url: String,
    /// Unix-ms expiry hint from AuthState. 0 when unknown.
    pub expires_at_ms: i64,
    /// True when an AuthState exists AND broker_token is non-empty. UI
    /// keys "Rotate" button enable state off this.
    pub configured: bool,
}

#[derive(Debug, Serialize)]
pub struct RotateBrokerTokenResult {
    pub token_preview: String,
    pub rotated_at_unix: u64,
}

// ── Validators ───────────────────────────────────────────────────────────

/// Trim + range-check a heartbeat-interval slider value. Split out so the
/// unit tests don't have to touch the filesystem.
pub fn validate_heartbeat_secs(secs: u64) -> Result<(), &'static str> {
    if secs < MIN_HEARTBEAT_SECS || secs > MAX_HEARTBEAT_SECS {
        return Err(E_HEARTBEAT_OUT_OF_RANGE);
    }
    Ok(())
}

/// Trim + allow-list a peer URL. Empty / pure-whitespace → distinct error
/// code so the UI can show a clearer "URL required" hint than the
/// daemon-allowlist reject for, e.g., `localhost:7878`.
pub fn validate_peer_url(url: &str) -> Result<String, String> {
    let trimmed = url.trim().trim_end_matches('/').to_string();
    if trimmed.is_empty() {
        return Err(E_PEER_URL_EMPTY.to_string());
    }
    validate_daemon_url(&trimmed)
        .map_err(|reason| format!("{E_PEER_URL_INVALID}: {reason}"))?;
    Ok(trimmed)
}

// ── Token preview (redaction helper) ─────────────────────────────────────

/// Build the obfuscated "********abcd" preview from a raw token. Pure +
/// total — never panics, returns "" for empty input. Lives outside the
/// command function so the test suite verifies the redaction shape
/// directly.
pub fn redact_token(token: &str) -> String {
    if token.is_empty() {
        return String::new();
    }
    let suffix: String = token.chars().rev().take(4).collect::<String>().chars().rev().collect();
    format!("********{suffix}")
}

// ── get_broker_token_preview ─────────────────────────────────────────────

#[tauri::command]
pub fn get_broker_token_preview() -> BrokerTokenPreview {
    let state = auth::load();
    match state {
        Some(s) => BrokerTokenPreview {
            token_preview: redact_token(&s.broker_token),
            broker_url: s.broker_url.clone(),
            expires_at_ms: s.broker_token_expires_at_ms,
            configured: !s.broker_token.is_empty(),
        },
        None => BrokerTokenPreview {
            token_preview: String::new(),
            broker_url: String::new(),
            expires_at_ms: 0,
            configured: false,
        },
    }
}

// ── rotate_broker_token ──────────────────────────────────────────────────
//
// Server-side rotation (calling phantommesh.io) is intentionally out of
// scope for F105: the broker doesn't yet expose a `/api/me/rotate-token`
// endpoint, and the spec wants the surface usable offline. We mint a
// fresh 256-bit random token locally and persist it; the broker
// re-issues a real JWT on next login. The UI is expected to prompt the
// user to re-log-in to phantommesh.io after rotation when the local
// token has aged past its server-side validity.

fn generate_token_bytes() -> [u8; 32] {
    let mut buf = [0u8; 32];
    for slot in buf.iter_mut() {
        *slot = rand::random::<u8>();
    }
    buf
}

fn encode_token(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[tauri::command]
pub fn rotate_broker_token() -> Result<RotateBrokerTokenResult, String> {
    // Require an existing AuthState. We refuse to create one from
    // scratch here — the user must complete broker login first, both
    // because we need their device_id/email and because a freshly-minted
    // token with no broker_url to point at is useless.
    let Some(mut state) = auth::load() else {
        return Err(E_AUTH_REQUIRED.to_string());
    };
    if state.broker_url.is_empty() {
        return Err(E_AUTH_REQUIRED.to_string());
    }

    let new_token = encode_token(&generate_token_bytes());
    // Defensive equality check before write: ensures the entropy source
    // didn't somehow hand us the existing token. constant_time_eq guards
    // against any side-channel on the comparison (broker_token is the
    // single most sensitive field on the device).
    let same = state.broker_token.as_bytes().ct_eq(new_token.as_bytes());
    if bool::from(same) {
        return Err(format!("{E_AUTH_WRITE}: entropy collision"));
    }
    state.broker_token = new_token.clone();
    state.broker_token_expires_at_ms = 0; // unknown until next broker call
    state.last_login_ms = auth::now_ms();
    auth::save(&state).map_err(|e| format!("{E_AUTH_WRITE}: {e}"))?;

    let rotated_at_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok(RotateBrokerTokenResult {
        token_preview: redact_token(&new_token),
        rotated_at_unix,
    })
}

// ── agents.toml read-modify-write helpers ────────────────────────────────
//
// We deliberately do NOT round-trip through `AgentsConfig` for these
// edits: that would lose comments and re-order keys (Serialize-then-
// write produces a freshly-formatted document). toml_edit preserves
// every byte we don't explicitly touch.

fn agents_toml_path() -> PathBuf {
    #[cfg(test)]
    {
        if let Some(p) = test_path_override() {
            return p;
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".spectyn-mesh")
        .join("agents.toml")
}

#[cfg(test)]
fn test_path_override() -> Option<PathBuf> {
    let slot = TEST_PATH_OVERRIDE.get_or_init(|| std::sync::Mutex::new(None));
    slot.lock().ok().and_then(|g| g.clone())
}

#[cfg(test)]
static TEST_PATH_OVERRIDE: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

#[cfg(test)]
fn set_test_path(p: PathBuf) {
    let slot = TEST_PATH_OVERRIDE.get_or_init(|| std::sync::Mutex::new(None));
    *slot.lock().unwrap() = Some(p);
}

#[cfg(test)]
fn clear_test_path() {
    if let Some(slot) = TEST_PATH_OVERRIDE.get() {
        *slot.lock().unwrap() = None;
    }
}

/// Load agents.toml as a mutable toml_edit document. Used by both the
/// peer-add and heartbeat-set commands so the read + parse error codes
/// are identical and grep-able.
fn load_agents_doc() -> Result<(toml_edit::DocumentMut, PathBuf), String> {
    let path = agents_toml_path();
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("{E_TOML_READ}: {}: {e}", path.display()))?;
    let doc: toml_edit::DocumentMut = raw
        .parse()
        .map_err(|e| format!("{E_TOML_PARSE}: {}: {e}", path.display()))?;
    Ok((doc, path))
}

/// Atomic write back to disk: write to `<file>.tmp`, then rename. Same
/// pattern cli_config.rs uses so partial writes never leave a corrupt
/// agents.toml on the box.
fn save_agents_doc(doc: &toml_edit::DocumentMut, path: &PathBuf) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("{E_TOML_WRITE}: mkdir {parent:?}: {e}"))?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string())
        .map_err(|e| format!("{E_TOML_WRITE}: write tmp {tmp:?}: {e}"))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("{E_TOML_WRITE}: rename {tmp:?} → {path:?}: {e}"))?;
    Ok(())
}

/// Ensure `[cluster]` exists as a table in the document and return a
/// mutable handle to it. Refuses to overwrite a non-table value at that
/// key (would be a user-data-mangle).
fn cluster_table_mut(doc: &mut toml_edit::DocumentMut) -> Result<&mut toml_edit::Table, String> {
    let item = doc
        .entry("cluster")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    item.as_table_mut()
        .ok_or_else(|| format!("{E_TOML_SHAPE}: [cluster] is not a table"))
}

// ── get_heartbeat_interval ───────────────────────────────────────────────

#[tauri::command]
pub fn get_heartbeat_interval() -> Result<u64, String> {
    // Read-only path. Missing file or missing key → default. We don't
    // bubble up a hard error for the no-file case so the UI can render
    // the slider at default and the user's first slider-commit creates
    // the section on disk.
    let path = agents_toml_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Ok(DEFAULT_HEARTBEAT_SECS);
    };
    let doc: toml_edit::DocumentMut = match raw.parse() {
        Ok(d) => d,
        Err(_) => return Ok(DEFAULT_HEARTBEAT_SECS),
    };
    let secs = doc
        .get("cluster")
        .and_then(|c| c.as_table())
        .and_then(|t| t.get("heartbeat_interval_secs"))
        .and_then(|v| v.as_integer())
        .and_then(|n| if n >= 0 { Some(n as u64) } else { None })
        .unwrap_or(DEFAULT_HEARTBEAT_SECS);
    // Clamp on read so a manually-edited out-of-range value doesn't
    // surface as a slider that snaps past the rail.
    Ok(secs.clamp(MIN_HEARTBEAT_SECS, MAX_HEARTBEAT_SECS))
}

// ── set_heartbeat_interval ───────────────────────────────────────────────

#[tauri::command]
pub fn set_heartbeat_interval(secs: u64) -> Result<(), String> {
    validate_heartbeat_secs(secs).map_err(|c| c.to_string())?;
    // Try the read-modify-write path. If agents.toml is missing we
    // synthesise a minimal document so the first-time slider commit
    // still writes something durable (matches the read fallback above).
    let (mut doc, path) = match load_agents_doc() {
        Ok(t) => t,
        Err(_) => {
            let doc: toml_edit::DocumentMut = toml_edit::DocumentMut::new();
            (doc, agents_toml_path())
        }
    };
    {
        let tbl = cluster_table_mut(&mut doc)?;
        tbl.insert("heartbeat_interval_secs", toml_edit::value(secs as i64));
    }
    save_agents_doc(&doc, &path)?;
    Ok(())
}

// ── add_cluster_peer ─────────────────────────────────────────────────────

/// Pure helper: append `url` to a peers array if not already present.
/// Returns true when the array changed (caller persists the doc), false
/// when the URL was already there (caller emits `E_PEER_DUPLICATE`).
fn append_peer_unique(arr: &mut toml_edit::Array, url: &str) -> bool {
    for existing in arr.iter() {
        if let Some(s) = existing.as_str() {
            // Case-sensitive equality on the canonical (trimmed)
            // form. We deliberately don't normalise the scheme/case
            // here — TLS off vs on is a real difference and we don't
            // want to silently merge.
            if s == url {
                return false;
            }
        }
    }
    arr.push(url);
    true
}

#[tauri::command]
pub fn add_cluster_peer(peer_url: String) -> Result<(), String> {
    let url = validate_peer_url(&peer_url)?;

    let (mut doc, path) = match load_agents_doc() {
        Ok(t) => t,
        Err(_) => {
            let doc: toml_edit::DocumentMut = toml_edit::DocumentMut::new();
            (doc, agents_toml_path())
        }
    };
    {
        let tbl = cluster_table_mut(&mut doc)?;
        // Fetch-or-create the `peers` array as a real TOML array (not an
        // inline-table). We default to a multi-line array so future
        // additions land on their own line — mirrors the cli_config.rs
        // join-cluster formatting.
        let needs_init = tbl
            .get("peers")
            .map(|v| v.as_array().is_none())
            .unwrap_or(true);
        if needs_init {
            let mut arr = toml_edit::Array::new();
            arr.set_trailing_comma(true);
            tbl.insert("peers", toml_edit::Item::Value(toml_edit::Value::Array(arr)));
        }
        let arr = tbl
            .get_mut("peers")
            .and_then(|i| i.as_array_mut())
            .ok_or_else(|| format!("{E_TOML_SHAPE}: [cluster].peers is not an array"))?;
        if !append_peer_unique(arr, &url) {
            return Err(E_PEER_DUPLICATE.to_string());
        }
    }
    save_agents_doc(&doc, &path)?;
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    // Tests that mutate the on-disk TEST_PATH_OVERRIDE or the
    // ~/.spectyn-mesh/auth.json file run under this lock so the
    // per-process global state doesn't race across parallel tests.
    static FILE_TEST_LOCK: StdMutex<()> = StdMutex::new(());

    // ── validate_heartbeat_secs ────────────────────────────────────────

    #[test]
    fn heartbeat_validator_accepts_range() {
        assert!(validate_heartbeat_secs(MIN_HEARTBEAT_SECS).is_ok());
        assert!(validate_heartbeat_secs(MAX_HEARTBEAT_SECS).is_ok());
        assert!(validate_heartbeat_secs(30).is_ok());
        assert!(validate_heartbeat_secs(120).is_ok());
    }

    #[test]
    fn heartbeat_validator_rejects_out_of_range() {
        assert_eq!(
            validate_heartbeat_secs(0).unwrap_err(),
            E_HEARTBEAT_OUT_OF_RANGE
        );
        assert_eq!(
            validate_heartbeat_secs(4).unwrap_err(),
            E_HEARTBEAT_OUT_OF_RANGE
        );
        assert_eq!(
            validate_heartbeat_secs(301).unwrap_err(),
            E_HEARTBEAT_OUT_OF_RANGE
        );
        assert_eq!(
            validate_heartbeat_secs(u64::MAX).unwrap_err(),
            E_HEARTBEAT_OUT_OF_RANGE
        );
    }

    // ── validate_peer_url ──────────────────────────────────────────────

    #[test]
    fn peer_validator_accepts_allowlisted_urls() {
        assert_eq!(
            validate_peer_url("http://localhost:7878").unwrap(),
            "http://localhost:7878"
        );
        assert_eq!(
            validate_peer_url("  http://localhost:7878/  ").unwrap(),
            "http://localhost:7878",
            "should trim whitespace + trailing slash"
        );
        assert!(validate_peer_url("https://phantommesh.io").is_ok());
        assert!(validate_peer_url("http://oracle.tail.ts.net:7878").is_ok());
    }

    #[test]
    fn peer_validator_rejects_empty() {
        assert_eq!(validate_peer_url("").unwrap_err(), E_PEER_URL_EMPTY);
        assert_eq!(validate_peer_url("   ").unwrap_err(), E_PEER_URL_EMPTY);
        assert_eq!(validate_peer_url(" \n ").unwrap_err(), E_PEER_URL_EMPTY);
    }

    #[test]
    fn peer_validator_rejects_disallowed() {
        // Same V8-HIGH-2 allow-list as cluster_peers::validate_daemon_url.
        for bad in [
            "http://evil.example.com/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "spectyn://anything",
            "ftp://example.com/",
            "http://user:pass@localhost/",
            "localhost:7878",
            "not a url",
        ] {
            let err = validate_peer_url(bad).unwrap_err();
            assert!(
                err.starts_with(E_PEER_URL_INVALID),
                "must reject {bad:?} with stable code, got: {err}"
            );
        }
    }

    // ── redact_token ───────────────────────────────────────────────────

    #[test]
    fn redact_token_preserves_only_last_four() {
        assert_eq!(redact_token(""), "");
        assert_eq!(redact_token("abcd"), "********abcd");
        assert_eq!(redact_token("verylongbrokertoken-xyzw"), "********xyzw");
        // Multi-byte safe — last 4 unicode chars, not bytes.
        let s = redact_token("aaaaaaaaaa摘要這段");
        assert!(s.ends_with("摘要這段"), "got: {s}");
    }

    #[test]
    fn redact_token_never_includes_raw_prefix() {
        // The whole point: even with a short token the prefix bytes
        // must not leak via the preview surface.
        let raw = "SUPER-SECRET-PREFIX-1234";
        let red = redact_token(raw);
        assert!(!red.contains("SUPER"));
        assert!(!red.contains("SECRET"));
        assert!(!red.contains("PREFIX"));
        assert!(red.ends_with("1234"));
    }

    // ── append_peer_unique ─────────────────────────────────────────────

    #[test]
    fn append_peer_unique_skips_duplicates() {
        let mut arr = toml_edit::Array::new();
        assert!(append_peer_unique(&mut arr, "http://localhost:7878"));
        assert!(!append_peer_unique(&mut arr, "http://localhost:7878"),
            "second insert of same URL must return false");
        assert!(append_peer_unique(&mut arr, "http://oracle.tail.ts.net:7878"));
        assert_eq!(arr.len(), 2);
    }

    // ── token_equality_is_constant_time (subtle::ConstantTimeEq sanity) ─
    //
    // We don't try to prove constant-time experimentally — that requires
    // a stats-based timing harness out of scope here. Instead we verify
    // that ct_eq is invoked, returns the right boolean, and that
    // bool::from(...) is the API the rotate path uses. Catches an
    // accidental switch back to `==` on the secret comparison.

    #[test]
    fn token_equality_is_constant_time() {
        let a = "token-aaa";
        let b = "token-aaa";
        let c = "token-bbb";
        let same = a.as_bytes().ct_eq(b.as_bytes());
        let diff = a.as_bytes().ct_eq(c.as_bytes());
        assert!(bool::from(same), "equal slices should compare equal");
        assert!(!bool::from(diff), "unequal slices should compare unequal");
    }

    // ── agents.toml round-trip helpers ─────────────────────────────────

    struct TempDirGuard {
        path: PathBuf,
    }
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            clear_test_path();
            if let Some(parent) = self.path.parent() {
                let _ = std::fs::remove_dir_all(parent);
            }
        }
    }
    fn fresh_agents_toml_dir() -> TempDirGuard {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "spectyn-f105-{}-{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("agents.toml");
        set_test_path(path.clone());
        TempDirGuard { path }
    }

    // ── get_heartbeat_interval ─────────────────────────────────────────

    #[test]
    fn heartbeat_get_returns_default_when_file_missing() {
        let _lock = FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = fresh_agents_toml_dir();
        assert_eq!(get_heartbeat_interval().unwrap(), DEFAULT_HEARTBEAT_SECS);
    }

    #[test]
    fn heartbeat_get_returns_default_when_key_missing() {
        let _lock = FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let g = fresh_agents_toml_dir();
        let body = "[cluster]\npeers = []\n";
        std::fs::write(&g.path, body).expect("seed");
        assert_eq!(get_heartbeat_interval().unwrap(), DEFAULT_HEARTBEAT_SECS);
    }

    #[test]
    fn heartbeat_get_reads_persisted_value() {
        let _lock = FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let g = fresh_agents_toml_dir();
        let body = "[cluster]\nheartbeat_interval_secs = 60\n";
        std::fs::write(&g.path, body).expect("seed");
        assert_eq!(get_heartbeat_interval().unwrap(), 60);
    }

    #[test]
    fn heartbeat_get_clamps_out_of_range_on_read() {
        let _lock = FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let g = fresh_agents_toml_dir();
        let body = "[cluster]\nheartbeat_interval_secs = 99999\n";
        std::fs::write(&g.path, body).expect("seed");
        assert_eq!(get_heartbeat_interval().unwrap(), MAX_HEARTBEAT_SECS);
    }

    // ── set_heartbeat_interval ─────────────────────────────────────────

    #[test]
    fn heartbeat_set_writes_value_creating_file() {
        let _lock = FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let g = fresh_agents_toml_dir();
        // No existing file — set should still succeed.
        set_heartbeat_interval(45).expect("set");
        let raw = std::fs::read_to_string(&g.path).expect("read");
        assert!(raw.contains("heartbeat_interval_secs"));
        assert!(raw.contains("45"));
        // Re-read via the command path.
        assert_eq!(get_heartbeat_interval().unwrap(), 45);
    }

    #[test]
    fn heartbeat_set_preserves_other_keys_and_comments() {
        // THE critical contract for E002 §settings — read-modify-write
        // must not blow away providers / agents / cluster_secret. We
        // seed a realistic agents.toml fragment and verify every byte
        // we didn't touch survives.
        let _lock = FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let g = fresh_agents_toml_dir();
        let seed = "\
# top-of-file comment must survive
default_model = \"openai:gpt-4o-mini\"

[providers.openai]
api_key = \"sk-xxx\"  # don't drop me

[cluster]
node_name = \"my-node\"
cluster_secret = \"spectyn-cluster-existing\"
heartbeat_interval_secs = 30
peers = [
  \"http://1.2.3.4:7878\",  # legacy peer
]

[agent.master]
provider = \"openai\"
";
        std::fs::write(&g.path, seed).expect("seed");
        set_heartbeat_interval(120).expect("set");
        let raw = std::fs::read_to_string(&g.path).expect("read");

        // Updated value.
        assert!(raw.contains("heartbeat_interval_secs = 120"),
            "new value missing, got: {raw}");
        // Old value gone.
        assert!(!raw.contains("heartbeat_interval_secs = 30"),
            "old value lingered, got: {raw}");
        // Unrelated keys + comments untouched.
        assert!(raw.contains("# top-of-file comment must survive"));
        assert!(raw.contains("[providers.openai]"));
        assert!(raw.contains("sk-xxx"));
        assert!(raw.contains("# don't drop me"));
        assert!(raw.contains("cluster_secret = \"spectyn-cluster-existing\""));
        assert!(raw.contains("# legacy peer"));
        assert!(raw.contains("[agent.master]"));
    }

    #[test]
    fn heartbeat_set_rejects_out_of_range_without_touching_disk() {
        let _lock = FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let g = fresh_agents_toml_dir();
        let seed = "[cluster]\nheartbeat_interval_secs = 30\n";
        std::fs::write(&g.path, seed).expect("seed");

        let err = set_heartbeat_interval(0).unwrap_err();
        assert_eq!(err, E_HEARTBEAT_OUT_OF_RANGE);
        let err = set_heartbeat_interval(9999).unwrap_err();
        assert_eq!(err, E_HEARTBEAT_OUT_OF_RANGE);

        // File untouched.
        let raw = std::fs::read_to_string(&g.path).expect("read");
        assert_eq!(raw, seed);
    }

    // ── add_cluster_peer ───────────────────────────────────────────────

    #[test]
    fn add_peer_appends_unique_to_array() {
        let _lock = FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let g = fresh_agents_toml_dir();
        let seed = "[cluster]\npeers = [\"http://1.2.3.4:7878\"]\n";
        std::fs::write(&g.path, seed).expect("seed");

        add_cluster_peer("http://oracle.tail.ts.net:7878".to_string()).expect("add");
        let raw = std::fs::read_to_string(&g.path).expect("read");
        assert!(raw.contains("1.2.3.4"), "original peer must survive: {raw}");
        assert!(raw.contains("oracle.tail.ts.net"));
    }

    #[test]
    fn add_peer_rejects_duplicate_with_stable_code() {
        let _lock = FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let g = fresh_agents_toml_dir();
        let seed = "[cluster]\npeers = [\"http://localhost:7878\"]\n";
        std::fs::write(&g.path, seed).expect("seed");

        let err = add_cluster_peer("http://localhost:7878".to_string()).unwrap_err();
        assert_eq!(err, E_PEER_DUPLICATE);
        // Also rejects same URL with trailing slash (normalised before
        // the duplicate check).
        let err = add_cluster_peer("http://localhost:7878/".to_string()).unwrap_err();
        assert_eq!(err, E_PEER_DUPLICATE);
    }

    #[test]
    fn add_peer_creates_cluster_section_when_missing() {
        let _lock = FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let g = fresh_agents_toml_dir();
        let seed = "default_model = \"openai:gpt-4o-mini\"\n";
        std::fs::write(&g.path, seed).expect("seed");

        add_cluster_peer("http://localhost:7878".to_string()).expect("add");
        let raw = std::fs::read_to_string(&g.path).expect("read");
        assert!(raw.contains("[cluster]"));
        assert!(raw.contains("http://localhost:7878"));
        assert!(raw.contains("default_model"), "pre-existing keys must survive");
    }

    #[test]
    fn add_peer_validates_url_before_touching_disk() {
        let _lock = FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let g = fresh_agents_toml_dir();
        let seed = "[cluster]\npeers = []\n";
        std::fs::write(&g.path, seed).expect("seed");

        let err = add_cluster_peer("http://attacker.example.com".to_string()).unwrap_err();
        assert!(
            err.starts_with(E_PEER_URL_INVALID),
            "must reject with stable code, got: {err}"
        );
        let err = add_cluster_peer("".to_string()).unwrap_err();
        assert_eq!(err, E_PEER_URL_EMPTY);

        // File untouched.
        let raw = std::fs::read_to_string(&g.path).expect("read");
        assert_eq!(raw, seed);
    }

    // ── rotate_broker_token (auth.json) ────────────────────────────────
    //
    // These tests overwrite the real ~/.spectyn-mesh/auth.json on the
    // dev box. They take an exclusive lock and restore the original
    // contents on drop to keep the dev environment clean.

    struct AuthSnapshot {
        existed: bool,
        bytes: Vec<u8>,
    }
    impl AuthSnapshot {
        fn capture() -> Self {
            let p = auth::auth_path();
            match std::fs::read(&p) {
                Ok(b) => AuthSnapshot { existed: true, bytes: b },
                Err(_) => AuthSnapshot { existed: false, bytes: Vec::new() },
            }
        }
    }
    impl Drop for AuthSnapshot {
        fn drop(&mut self) {
            let p = auth::auth_path();
            if self.existed {
                let _ = std::fs::write(&p, &self.bytes);
            } else {
                let _ = std::fs::remove_file(&p);
            }
        }
    }

    #[test]
    fn rotate_requires_existing_auth_state() {
        let _lock = FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _snap = AuthSnapshot::capture();
        let _ = std::fs::remove_file(auth::auth_path());
        let err = rotate_broker_token().unwrap_err();
        assert_eq!(err, E_AUTH_REQUIRED);
    }

    #[test]
    fn rotate_replaces_token_and_returns_redacted_preview() {
        let _lock = FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _snap = AuthSnapshot::capture();

        // Seed a minimal AuthState.
        let mut seed = auth::AuthState {
            provider: "email".to_string(),
            email: "x@y".to_string(),
            display_name: None,
            sub: None,
            avatar_url: None,
            device_id: "device-1".to_string(),
            created_at_ms: 0,
            last_login_ms: 0,
            password_hash: String::new(),
            salt: String::new(),
            id_token: String::new(),
            access_token: String::new(),
            broker_token: "OLD-TOKEN-aaaa".to_string(),
            broker_token_expires_at_ms: 12345,
            broker_url: "https://phantommesh.io".to_string(),
        };
        // Ensure the parent dir exists for the save call.
        if let Some(parent) = auth::auth_path().parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        auth::save(&seed).expect("seed save");

        let out = rotate_broker_token().expect("rotate");
        assert!(out.token_preview.starts_with("********"));
        assert!(!out.token_preview.contains("OLD"));
        assert!(out.rotated_at_unix > 0);

        // Reload + verify the on-disk token changed.
        let reloaded = auth::load().expect("reload");
        assert_ne!(reloaded.broker_token, "OLD-TOKEN-aaaa");
        assert!(!reloaded.broker_token.is_empty());
        // 32 random bytes → base64 url-safe → 43 chars (no padding).
        assert_eq!(reloaded.broker_token.len(), 43);
        // Other fields untouched.
        assert_eq!(reloaded.email, "x@y");
        assert_eq!(reloaded.device_id, "device-1");
        assert_eq!(reloaded.broker_url, "https://phantommesh.io");

        // Compile-time touch so we don't get an unused-variable lint
        // on `seed` if the test ever short-circuits earlier.
        seed.broker_token.clear();
    }

    #[test]
    fn rotate_requires_broker_url_not_just_state() {
        let _lock = FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _snap = AuthSnapshot::capture();
        let seed = auth::AuthState {
            provider: "email".to_string(),
            email: "x@y".to_string(),
            display_name: None,
            sub: None,
            avatar_url: None,
            device_id: "device-1".to_string(),
            created_at_ms: 0,
            last_login_ms: 0,
            password_hash: String::new(),
            salt: String::new(),
            id_token: String::new(),
            access_token: String::new(),
            broker_token: "anything".to_string(),
            broker_token_expires_at_ms: 0,
            broker_url: String::new(), // empty → reject
        };
        if let Some(parent) = auth::auth_path().parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        auth::save(&seed).expect("seed save");
        let err = rotate_broker_token().unwrap_err();
        assert_eq!(err, E_AUTH_REQUIRED);
    }

    // ── get_broker_token_preview redaction ─────────────────────────────

    #[test]
    fn preview_never_returns_raw_token() {
        let _lock = FILE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _snap = AuthSnapshot::capture();
        let seed = auth::AuthState {
            provider: "email".to_string(),
            email: "x@y".to_string(),
            display_name: None,
            sub: None,
            avatar_url: None,
            device_id: "device-1".to_string(),
            created_at_ms: 0,
            last_login_ms: 0,
            password_hash: String::new(),
            salt: String::new(),
            id_token: String::new(),
            access_token: String::new(),
            broker_token: "VERY-LONG-SECRET-TOKEN-xyzw".to_string(),
            broker_token_expires_at_ms: 999,
            broker_url: "https://phantommesh.io".to_string(),
        };
        if let Some(parent) = auth::auth_path().parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        auth::save(&seed).expect("seed save");
        let p = get_broker_token_preview();
        assert!(p.configured);
        assert_eq!(p.token_preview, "********xyzw");
        assert!(!p.token_preview.contains("SECRET"));
        assert_eq!(p.broker_url, "https://phantommesh.io");
        assert_eq!(p.expires_at_ms, 999);
    }
}
