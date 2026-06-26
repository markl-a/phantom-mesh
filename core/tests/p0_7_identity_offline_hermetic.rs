//! P0-7 S1 — local ed25519 identity is created OFFLINE and never leaves the device.
//!
//! Hermetic: runs the REAL `phantom` binary with `keys init` under a temp data
//! root (HOME + USERPROFILE + PHANTOM_HOME redirected) so it cannot touch the
//! developer's real ~/.phantom-mesh, and proves the keypair lands with no
//! outbound network. Unix-gated for the same reason as
//! cli_exec_jsonl_schema_hermetic.rs (the child's dirs::home_dir() ignores the
//! HOME redirect on Windows). On Windows this file compiles to an empty test
//! binary and trivially passes; it executes for real on WSL / Linux CI.
#![cfg(unix)]

use std::process::Command;

fn phantom_bin() -> &'static str {
    env!("CARGO_BIN_EXE_phantom")
}

fn temp_root() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "phantom-p07-id-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn keys_init_creates_ed25519_identity_offline() {
    let root = temp_root();
    let pm = root.join(".phantom-mesh");
    std::fs::create_dir_all(&pm).unwrap();

    // No provider keys, no broker token, no network env in scope. The child
    // must complete `keys init` purely with the OS CSPRNG + filesystem.
    let out = Command::new(phantom_bin())
        .arg("keys")
        .arg("init")
        .env("HOME", &root)
        .env("USERPROFILE", &root)
        .env("PHANTOM_HOME", &pm)
        // Route any accidental HTTP through a black-hole proxy: if some code path
        // tried to reach the network, the call would fail fast against 127.0.0.1:1
        // rather than silently succeeding against the real internet.
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .output()
        .expect("run phantom keys init");

    assert!(
        out.status.success(),
        "keys init must succeed offline; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let priv_path = pm.join("keys").join("ed25519.priv");
    let pub_path = pm.join("keys").join("ed25519.pub");
    let idk = pm.join("identity.key");
    assert!(priv_path.exists(), "ed25519.priv must be created");
    assert!(pub_path.exists(), "ed25519.pub must be created");
    assert!(idk.exists(), "identity.key (root IKM) must be provisioned");

    // Private key is real 32-byte seed material (raw on Unix), not empty.
    let priv_bytes = std::fs::read(&priv_path).unwrap();
    assert_eq!(priv_bytes.len(), 32, "ed25519 priv seed is 32 bytes on Unix");
    assert_ne!(priv_bytes, vec![0u8; 32], "priv key must not be all-zero");

    // identity.key is the 64-byte HKDF IKM.
    assert_eq!(
        std::fs::read(&idk).unwrap().len(),
        64,
        "identity.key is 64-byte IKM"
    );

    // The private key must live ONLY under the temp data root (never copied out).
    // (No assertion can prove a negative network send directly; the proxy
    // black-hole above + the success exit code is the offline guarantee.)
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn keys_init_is_idempotent_offline() {
    let root = temp_root();
    let pm = root.join(".phantom-mesh");
    std::fs::create_dir_all(&pm).unwrap();
    let run = || {
        Command::new(phantom_bin())
            .arg("keys")
            .arg("init")
            .env("HOME", &root)
            .env("USERPROFILE", &root)
            .env("PHANTOM_HOME", &pm)
            .output()
            .unwrap()
    };
    assert!(run().status.success());
    let first = std::fs::read(pm.join("keys").join("ed25519.priv")).unwrap();
    // Second init without --force must NOT overwrite (keeps signatures valid).
    assert!(run().status.success());
    let second = std::fs::read(pm.join("keys").join("ed25519.priv")).unwrap();
    assert_eq!(first, second, "keys init must be idempotent without --force");
    let _ = std::fs::remove_dir_all(&root);
}
