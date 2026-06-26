//! Real-path integration test for the `phantom nodes` CLI (mvp/node-manifest).
//!
//! Spawns the actually-built binary (`env!("CARGO_BIN_EXE_phantom")`) against a
//! temp `$HOME` plus a `PHANTOM_HOME` data root so it reads the same
//! `~/.phantom-mesh/peers.json`
//! roster a real user hits — no mocks. Asserts:
//!
//!   * `nodes caps --json` is a well-formed array carrying the local node,
//!   * `nodes inspect local --json` emits a NodeManifest with os/arch/version,
//!   * `nodes inspect <peer>` renders the seeded peer's caps + url as a table,
//!   * `nodes inspect <unknown>` exits nonzero with a clean error (no panic),
//!   * human-table output is ASCII (CP950-safe).
//!
//! ## Isolation / platform gate
//!
//! `HOME` + `USERPROFILE` point at a unique temp dir and `PHANTOM_HOME` points
//! at that temp dir's `.phantom-mesh` data root, so the child never touches the
//! developer's real `~/.phantom-mesh`. Gated to `unix` to mirror
//! `task_cli_roundtrip.rs`: on Windows a bare `dirs::home_dir()` fallback can
//! still win in some child contexts; the platform-agnostic manifest/CLI logic
//! is fully covered on the Linux CI runner.

#![cfg(unix)]

use std::process::Command;

fn phantom_bin() -> &'static str {
    env!("CARGO_BIN_EXE_phantom")
}

fn unique_home() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "phantom-nodes-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// Run `phantom <args...>` under the temp `$HOME`, returning the captured output.
fn run(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    let home_s = home.to_string_lossy().to_string();
    let data_root_s = data_root(home).to_string_lossy().to_string();
    Command::new(phantom_bin())
        .args(args)
        .env("HOME", &home_s)
        .env("USERPROFILE", &home_s)
        .env("PHANTOM_HOME", &data_root_s)
        // Pin a deterministic local node name so resolution is predictable.
        .env("PHANTOM_NODE_NAME", "test-local")
        .env_remove("PHANTOM_LOCAL_FIRST")
        .env_remove("PHANTOM_RUNTIME_OVERRIDE")
        .output()
        .expect("phantom must spawn")
}

fn data_root(home: &std::path::Path) -> std::path::PathBuf {
    home.join(".phantom-mesh")
}

/// Seed a minimal peers.json roster with one peer.
fn seed_home() -> std::path::PathBuf {
    let home = unique_home();
    let dir = data_root(&home);
    std::fs::create_dir_all(&dir).expect("seed temp home");
    let peers = r#"[
        {"name":"peer-x","url":"http://10.0.0.5:7878","capabilities":["rust","cargo"]}
    ]"#;
    std::fs::write(dir.join("peers.json"), peers).expect("seed peers.json");
    home
}

#[test]
fn nodes_caps_json_includes_local() {
    let home = seed_home();

    let out = run(&home, &["nodes", "caps", "--json"]);
    assert!(
        out.status.success(),
        "nodes caps --json exited {:?}\nstderr:{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let rows: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("nodes caps --json must emit a JSON array");
    let arr = rows.as_array().expect("caps json is an array");
    assert!(
        arr.iter().any(|r| r["is_local"] == serde_json::json!(true)
            && r["node_id"] == serde_json::json!("test-local")),
        "caps json must carry the local node 'test-local', got: {}",
        rows
    );
    // The seeded peer surfaces with its caps.
    assert!(
        arr.iter()
            .any(|r| r["node_id"] == serde_json::json!("peer-x")
                && r["capabilities"] == serde_json::json!(["rust", "cargo"])),
        "caps json must carry peer-x with [rust,cargo], got: {}",
        rows
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn nodes_inspect_local_json_has_os_arch_version() {
    let home = seed_home();

    let out = run(&home, &["nodes", "inspect", "local", "--json"]);
    assert!(
        out.status.success(),
        "nodes inspect local --json exited {:?}\nstderr:{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let m: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("inspect --json must emit one JSON object");
    assert_eq!(m["node_id"], serde_json::json!("test-local"));
    assert_eq!(m["is_local"], serde_json::json!(true));
    assert!(m["os"].is_string(), "local manifest must carry os: {}", m);
    assert!(
        m["arch"].is_string(),
        "local manifest must carry arch: {}",
        m
    );
    assert!(
        m["version"].is_string(),
        "local manifest must carry version: {}",
        m
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn nodes_inspect_peer_table_renders_caps_and_url() {
    let home = seed_home();

    let out = run(&home, &["nodes", "inspect", "peer-x"]);
    assert!(
        out.status.success(),
        "nodes inspect peer-x exited {:?}\nstderr:{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.is_ascii(),
        "table output must be ASCII-safe for CP950, got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("peer-x"),
        "table must name the node: {}",
        stdout
    );
    assert!(
        stdout.contains("http://10.0.0.5:7878"),
        "table must show the peer base_url: {}",
        stdout
    );
    assert!(
        stdout.contains("rust") && stdout.contains("cargo"),
        "table must show the peer caps: {}",
        stdout
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn nodes_inspect_unknown_fails_cleanly() {
    let home = seed_home();

    let out = run(&home, &["nodes", "inspect", "no-such-node"]);
    assert!(
        !out.status.success(),
        "inspecting an unknown node must exit nonzero"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown node"),
        "expected a clean 'unknown node' error, got: {:?}",
        stderr
    );

    let _ = std::fs::remove_dir_all(&home);
}
