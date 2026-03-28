//! Integration tests for PairingBundle (key_export module).

use phantom_mesh::security::{PairingBundle, PairingError};
use std::collections::HashMap;

fn sample_keys() -> HashMap<String, String> {
    let mut keys = HashMap::new();
    keys.insert("openai".to_string(), "sk-abc123".to_string());
    keys.insert("anthropic".to_string(), "sk-ant-xyz789".to_string());
    keys
}

#[test]
fn test_create_and_open() {
    let keys = sample_keys();
    let bundle = PairingBundle::create(
        "cluster-secret-42",
        "192.168.1.10:7878",
        &keys,
        "my-passphrase",
    )
    .unwrap();

    let data = bundle.open("my-passphrase").unwrap();
    assert_eq!(data.cluster_secret, "cluster-secret-42");
    assert_eq!(data.coordinator_addr, "192.168.1.10:7878");
    assert_eq!(data.api_keys.get("openai").unwrap(), "sk-abc123");
    assert_eq!(data.api_keys.get("anthropic").unwrap(), "sk-ant-xyz789");
}

#[test]
fn test_wrong_passphrase() {
    let keys = sample_keys();
    let bundle = PairingBundle::create(
        "cluster-secret",
        "192.168.1.10:7878",
        &keys,
        "correct-passphrase",
    )
    .unwrap();

    let err = bundle.open("wrong-passphrase").unwrap_err();
    assert!(matches!(err, PairingError::DecryptionFailed));
}

#[test]
fn test_expired_bundle() {
    let keys = sample_keys();
    let mut bundle = PairingBundle::create(
        "cluster-secret",
        "192.168.1.10:7878",
        &keys,
        "passphrase",
    )
    .unwrap();

    // Force expiration to the past
    bundle.expires_at = 0;

    assert!(bundle.is_expired());
    let err = bundle.open("passphrase").unwrap_err();
    assert!(matches!(err, PairingError::Expired));
}

#[test]
fn test_base64_roundtrip() {
    let keys = sample_keys();
    let bundle = PairingBundle::create(
        "cluster-secret-b64",
        "10.0.0.1:9000",
        &keys,
        "roundtrip-pass",
    )
    .unwrap();

    let encoded = bundle.to_base64().unwrap();
    let decoded = PairingBundle::from_base64(&encoded).unwrap();
    let data = decoded.open("roundtrip-pass").unwrap();

    assert_eq!(data.cluster_secret, "cluster-secret-b64");
    assert_eq!(data.coordinator_addr, "10.0.0.1:9000");
    assert_eq!(data.api_keys.get("openai").unwrap(), "sk-abc123");
    assert_eq!(data.api_keys.get("anthropic").unwrap(), "sk-ant-xyz789");
}

#[test]
fn test_multiple_keys() {
    let mut keys = HashMap::new();
    keys.insert("openai".to_string(), "sk-openai-key".to_string());
    keys.insert("anthropic".to_string(), "sk-ant-key".to_string());
    keys.insert("groq".to_string(), "gsk-groq-key".to_string());
    keys.insert("gemini".to_string(), "AIza-gemini-key".to_string());
    keys.insert("mistral".to_string(), "mist-key-12345".to_string());

    let bundle = PairingBundle::create(
        "multi-key-cluster",
        "172.16.0.1:7878",
        &keys,
        "five-key-pass",
    )
    .unwrap();

    let data = bundle.open("five-key-pass").unwrap();
    assert_eq!(data.api_keys.len(), 5);
    assert_eq!(data.api_keys.get("openai").unwrap(), "sk-openai-key");
    assert_eq!(data.api_keys.get("anthropic").unwrap(), "sk-ant-key");
    assert_eq!(data.api_keys.get("groq").unwrap(), "gsk-groq-key");
    assert_eq!(data.api_keys.get("gemini").unwrap(), "AIza-gemini-key");
    assert_eq!(data.api_keys.get("mistral").unwrap(), "mist-key-12345");
}
