//! Shared cluster-auth gate.
//!
//! Lifted out of `core/src/serve.rs` (T7) so both routers — the `phantom
//! serve` UI router (`serve.rs`) and the daemon router (`main.rs::build_router`
//! used by `phantom-mesh daemon`) — can share one HMAC check.
//!
//! Behaviour mirrors T7's original `require_cluster_auth` exactly:
//!
//! 1. If `cluster_secret` is configured, the body must carry an
//!    `X-Cluster-Auth` HMAC-SHA256 that matches.
//! 2. If `cluster_secret` is empty / missing, the call is REFUSED
//!    (`403 Forbidden`) with a migration hint — UNLESS the operator has
//!    set `PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET=1` to opt into legacy
//!    insecure behaviour for one migration release.
//!
//! The error JSON is bare (no wire_version wrapper). Callers in `serve.rs`
//! that want the wrapper apply it themselves after the `?`.

use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde_json::{json, Value};
use std::net::SocketAddr;

/// Like [`require_cluster_auth`], but additionally exempts requests that
/// originate from the loopback interface (`127.0.0.0/8` / `::1`).
///
/// Used ONLY for local-UI endpoints (e.g. `/api/chat`) so the same-host
/// dashboard / desktop app can call its own daemon without the cluster HMAC —
/// while REMOTE peers stay fully gated (they fall through to
/// `require_cluster_auth`). Genuine cluster RPC endpoints (`/rpc/message`,
/// `/rpc/task/assign`) intentionally do NOT use this and keep the strict gate.
///
/// Loopback is decided from the real peer socket address (axum `ConnectInfo`),
/// which a remote attacker cannot spoof — unlike the client-controlled `Host`
/// header. Implements SPEC-46 I3 ("local plane must not require cluster auth")
/// without weakening remote auth.
pub fn require_cluster_auth_local_ui(
    cm: &crate::mesh::ClusterManager,
    peer: SocketAddr,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(), (StatusCode, Json<Value>)> {
    if peer.ip().is_loopback() {
        return Ok(());
    }
    require_cluster_auth(cm, headers, body)
}

/// Verify a cluster-auth request. See module docs for behaviour.
pub fn require_cluster_auth(
    cm: &crate::mesh::ClusterManager,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(), (StatusCode, Json<Value>)> {
    let secret_configured = cm
        .config
        .cluster_secret
        .as_deref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);

    if !secret_configured {
        if std::env::var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
        {
            return Ok(());
        }
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "refused: cluster_secret not configured on this node — \
                         set [cluster].cluster_secret in agents.toml, or set \
                         PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET=1 to restore the \
                         legacy (insecure) behaviour for one migration release"
            })),
        ));
    }

    let token = headers
        .get("X-Cluster-Auth")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !cm.verify_auth(token, body) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": "unauthorized — bad or missing X-Cluster-Auth"
            })),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{ClusterConfig, ClusterManager};
    use std::sync::MutexGuard;

    /// Serialise env-mutating tests. Delegates to the crate-wide env mutex so
    /// PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET tests here serialize against the
    /// serve.rs tests that mutate the same var (a per-file mutex let them race).
    fn env_guard() -> MutexGuard<'static, ()> {
        crate::env_lock::acquire()
    }

    fn cm_with_secret(secret: &str) -> ClusterManager {
        let mut cfg = ClusterConfig::default();
        cfg.cluster_secret = if secret.is_empty() {
            None
        } else {
            Some(secret.into())
        };
        ClusterManager::new(cfg)
    }

    #[test]
    fn accepts_valid_token() {
        let _g = env_guard();
        std::env::remove_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET");
        let cm = cm_with_secret("topsecret");
        let body = b"{\"hello\":\"world\"}";
        let token = cm.make_auth_token(std::str::from_utf8(body).unwrap());
        let mut h = HeaderMap::new();
        h.insert("X-Cluster-Auth", token.parse().unwrap());
        assert!(require_cluster_auth(&cm, &h, body).is_ok());
    }

    #[test]
    fn rejects_bad_token() {
        let _g = env_guard();
        std::env::remove_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET");
        let cm = cm_with_secret("topsecret");
        let body = b"{\"hello\":\"world\"}";
        let mut h = HeaderMap::new();
        h.insert("X-Cluster-Auth", "deadbeef".parse().unwrap());
        let err = require_cluster_auth(&cm, &h, body).unwrap_err();
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn fails_closed_when_secret_empty() {
        let _g = env_guard();
        std::env::remove_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET");
        let cm = cm_with_secret("");
        let body = b"{}";
        let h = HeaderMap::new();
        let err = require_cluster_auth(&cm, &h, body).unwrap_err();
        assert_eq!(err.0, StatusCode::FORBIDDEN);
        let body_str = serde_json::to_string(&err.1 .0).unwrap();
        assert!(
            body_str.contains("cluster_secret not configured"),
            "missing migration hint in error body: {body_str}"
        );
        assert!(
            body_str.contains("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET"),
            "missing override hint in error body: {body_str}"
        );
    }

    #[test]
    fn local_ui_loopback_exempt_without_token() {
        // SPEC-46 I3: a same-host (loopback) caller to a local-UI endpoint is
        // exempt from the cluster HMAC even with NO X-Cluster-Auth header.
        let _g = env_guard();
        std::env::remove_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET");
        let cm = cm_with_secret("topsecret");
        let body = b"{\"message\":\"hi\"}";
        let h = HeaderMap::new(); // no token at all
        let v4: std::net::SocketAddr = "127.0.0.1:54321".parse().unwrap();
        assert!(require_cluster_auth_local_ui(&cm, v4, &h, body).is_ok());
        let v6: std::net::SocketAddr = "[::1]:54321".parse().unwrap();
        assert!(require_cluster_auth_local_ui(&cm, v6, &h, body).is_ok());
    }

    #[test]
    fn local_ui_remote_still_gated() {
        // A non-loopback peer must STILL need a valid token — no remote hole.
        let _g = env_guard();
        std::env::remove_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET");
        let cm = cm_with_secret("topsecret");
        let body = b"{\"message\":\"hi\"}";
        let h = HeaderMap::new(); // no token
        let remote: std::net::SocketAddr = "100.64.1.2:443".parse().unwrap();
        let err = require_cluster_auth_local_ui(&cm, remote, &h, body).unwrap_err();
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn local_ui_remote_with_valid_token_ok() {
        // A remote peer with a correct HMAC still works (delegates to the gate).
        let _g = env_guard();
        std::env::remove_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET");
        let cm = cm_with_secret("topsecret");
        let body = b"{\"message\":\"hi\"}";
        let token = cm.make_auth_token(std::str::from_utf8(body).unwrap());
        let mut h = HeaderMap::new();
        h.insert("X-Cluster-Auth", token.parse().unwrap());
        let remote: std::net::SocketAddr = "100.64.1.2:443".parse().unwrap();
        assert!(require_cluster_auth_local_ui(&cm, remote, &h, body).is_ok());
    }

    #[test]
    fn env_override_skips_check() {
        let _g = env_guard();
        std::env::set_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET", "1");
        let cm = cm_with_secret("");
        let body = b"{}";
        let h = HeaderMap::new();
        let result = require_cluster_auth(&cm, &h, body);
        std::env::remove_var("PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET");
        assert!(
            result.is_ok(),
            "override should permit empty-secret call: {result:?}"
        );
    }
}
