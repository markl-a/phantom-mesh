//! Integration tests for `ClusterAuth` — Ed25519-based RPC request signing/verification.

use phantom_mesh::security::{ClusterAuth, AuthHeaders, AuthError, NodeIdentity};
use tempfile::tempdir;

/// Helper: create a ClusterAuth from a fresh identity in the given directory.
fn make_auth(secret: &str) -> (tempfile::TempDir, ClusterAuth) {
    let dir = tempdir().unwrap();
    let identity = NodeIdentity::load_or_generate(dir.path()).unwrap();
    let auth = ClusterAuth::new(identity, secret.to_string());
    (dir, auth)
}

// -----------------------------------------------------------------------
// 1. sign → verify → pass, returns correct node_id
// -----------------------------------------------------------------------

#[test]
fn test_sign_and_verify() {
    let (dir_a, auth_a) = make_auth("shared-secret");
    let (_dir_b, auth_b) = make_auth("shared-secret");

    // B needs to know A's public key to verify A's requests.
    let a_id = {
        let id = NodeIdentity::load_or_generate(dir_a.path()).unwrap();
        (id.node_id().to_string(), id.public_key_bytes())
    };
    auth_b
        .register_peer(&a_id.0, &a_id.1)
        .expect("register_peer failed");

    let body = b"dispatch task payload";
    let headers = auth_a.sign_request("POST /rpc/dispatch", body);

    let verified_id = auth_b
        .verify_request(&headers, "POST /rpc/dispatch", body)
        .expect("verify_request should succeed");

    assert_eq!(verified_id, a_id.0);
}

// -----------------------------------------------------------------------
// 2. Different cluster secrets → signature mismatch
// -----------------------------------------------------------------------

#[test]
fn test_wrong_cluster_secret() {
    let (dir_a, auth_a) = make_auth("secret-alpha");
    let (_dir_b, auth_b) = make_auth("secret-beta");

    // Register A's key with B (so B doesn't fail on "unknown node").
    let a_identity = NodeIdentity::load_or_generate(dir_a.path()).unwrap();
    auth_b
        .register_peer(a_identity.node_id(), &a_identity.public_key_bytes())
        .unwrap();

    let headers = auth_a.sign_request("GET /status", b"");
    let result = auth_b.verify_request(&headers, "GET /status", b"");

    assert!(result.is_err());
    assert!(
        matches!(result.unwrap_err(), AuthError::InvalidSignature),
        "expected InvalidSignature for mismatched cluster secrets"
    );
}

// -----------------------------------------------------------------------
// 3. Expired timestamp → TimestampExpired
// -----------------------------------------------------------------------

#[test]
fn test_expired_timestamp() {
    let (_dir, auth) = make_auth("secret");

    // Create headers with a timestamp 60 seconds in the past.
    let body = b"payload";
    let mut headers = auth.sign_request("POST /rpc/evolve", body);

    // Overwrite the timestamp to 60s ago. The signature was computed with the
    // *original* timestamp, but verification will reject on timestamp check
    // before it even gets to the signature.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    headers.timestamp = now.saturating_sub(60);

    let result = auth.verify_request(&headers, "POST /rpc/evolve", body);
    assert!(result.is_err());
    match result.unwrap_err() {
        AuthError::TimestampExpired(delta) => {
            assert!(delta >= 30, "expected delta >= 30, got {delta}");
        }
        other => panic!("expected TimestampExpired, got {other:?}"),
    }
}

// -----------------------------------------------------------------------
// 4. Unknown node → UnknownNode
// -----------------------------------------------------------------------

#[test]
fn test_unknown_node() {
    let (_dir_a, auth_a) = make_auth("secret");
    let (_dir_b, auth_b) = make_auth("secret");

    // A signs a request, but B has never registered A's key.
    let headers = auth_a.sign_request("GET /health", b"");
    let result = auth_b.verify_request(&headers, "GET /health", b"");

    assert!(result.is_err());
    assert!(
        matches!(result.unwrap_err(), AuthError::UnknownNode(_)),
        "expected UnknownNode error"
    );
}

// -----------------------------------------------------------------------
// 5. Tampered body → InvalidSignature
// -----------------------------------------------------------------------

#[test]
fn test_tampered_body() {
    let (_dir, auth) = make_auth("secret");

    let original_body = b"original payload";
    let tampered_body = b"tampered payload";

    let headers = auth.sign_request("POST /rpc/dispatch", original_body);

    // Verify with a different body — body hash won't match → bad signature.
    let result = auth.verify_request(&headers, "POST /rpc/dispatch", tampered_body);
    assert!(result.is_err());
    assert!(
        matches!(result.unwrap_err(), AuthError::InvalidSignature),
        "expected InvalidSignature for tampered body"
    );
}
