//! Integration tests for `NodeIdentity` — Ed25519 keypair management.

use phantom_mesh::security::NodeIdentity;
use tempfile::tempdir;

/// Generate a new identity, sign "hello", and verify the signature.
#[test]
fn test_generate_and_sign_verify() {
    let dir = tempdir().unwrap();
    let identity = NodeIdentity::load_or_generate(dir.path()).unwrap();

    let message = b"hello";
    let signature = identity.sign(message);

    // Signature should be exactly 64 bytes (Ed25519).
    assert_eq!(signature.len(), 64);

    // Verify with our own public key — should pass.
    let pk = identity.public_key_bytes();
    assert!(NodeIdentity::verify(&pk, message, &signature));
}

/// Generate, save to disk, then load — must produce the same `node_id`.
#[test]
fn test_load_existing_key() {
    let dir = tempdir().unwrap();

    let id1 = NodeIdentity::load_or_generate(dir.path()).unwrap();
    let node_id_1 = id1.node_id.clone();
    let pk1 = id1.public_key_bytes();

    // Drop and reload — identity.key already exists on disk.
    drop(id1);
    let id2 = NodeIdentity::load_or_generate(dir.path()).unwrap();

    assert_eq!(id2.node_id, node_id_1, "node_id should survive a reload");
    assert_eq!(id2.public_key_bytes(), pk1, "public key should survive a reload");

    // Sign + verify across load boundary.
    let sig = id2.sign(b"persistence check");
    assert!(NodeIdentity::verify(&pk1, b"persistence check", &sig));
}

/// Sign with key A, try to verify with key B — must fail.
#[test]
fn test_verify_wrong_key() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();

    let id_a = NodeIdentity::load_or_generate(dir_a.path()).unwrap();
    let id_b = NodeIdentity::load_or_generate(dir_b.path()).unwrap();

    let message = b"signed by A";
    let sig_a = id_a.sign(message);

    // Verify with B's public key — should fail.
    let pk_b = id_b.public_key_bytes();
    assert!(
        !NodeIdentity::verify(&pk_b, message, &sig_a),
        "verification with wrong key must fail"
    );
}

/// Sign "hello", try to verify "world" — must fail.
#[test]
fn test_verify_tampered_message() {
    let dir = tempdir().unwrap();
    let identity = NodeIdentity::load_or_generate(dir.path()).unwrap();

    let sig = identity.sign(b"hello");
    let pk = identity.public_key_bytes();

    assert!(
        !NodeIdentity::verify(&pk, b"world", &sig),
        "verification with tampered message must fail"
    );
}

/// The same keypair should always produce the same `node_id`.
#[test]
fn test_node_id_is_deterministic() {
    let dir = tempdir().unwrap();

    let id1 = NodeIdentity::load_or_generate(dir.path()).unwrap();
    let nid1 = id1.node_id.clone();
    drop(id1);

    // Reload multiple times — must always get the same node_id.
    for _ in 0..5 {
        let id = NodeIdentity::load_or_generate(dir.path()).unwrap();
        assert_eq!(id.node_id, nid1, "node_id must be deterministic");
    }

    // node_id should be 16 hex characters.
    assert_eq!(nid1.len(), 16);
    assert!(nid1.chars().all(|c| c.is_ascii_hexdigit()));
}
