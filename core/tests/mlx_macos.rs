//! Mac-side MLX P0 integration tests.
//!
//! Prereqs (one-time, done outside this file):
//!   - `pip install mlx-lm` (or `huggingface_hub`) → makes `hf` /
//!     `huggingface-cli` + `mlx_lm.server` available on PATH
//!   - `spectyn mlx pull` → downloads default model (~4 GB) into HF cache
//!
//! Why integration not unit: `providers::mlx` is not its own module in
//! core (MLX routes through resolver.rs `mlx-local` agent type, not a
//! dedicated provider crate). The INDEX.md `providers::mlx::tests::*`
//! paths are aspirational; these tests live at the real exec boundary.

#![cfg(target_os = "macos")]

use std::process::Command;
use std::time::Duration;

fn spectyn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_spectyn")
}

async fn serve_reachable() -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    matches!(
        client.get("http://127.0.0.1:8080/v1/models").send().await,
        Ok(r) if r.status().is_success()
    )
}

#[test]
fn pull_validates_model_id() {
    let bin = spectyn_bin();
    let output = Command::new(bin)
        .args(["mlx", "pull", ""])
        .output()
        .expect("spectyn mlx pull must spawn");
    assert!(
        !output.status.success(),
        "spectyn mlx pull '' returned exit 0, but empty model id should fail.\n\
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn mlx_pull_smoke() {
    let bin = spectyn_bin();
    let output = Command::new(bin)
        .args(["mlx", "pull"])
        .output()
        .expect("spectyn mlx pull must spawn");
    assert!(
        output.status.success(),
        "spectyn mlx pull (default model) exited {:?} — is the model \
         downloaded and `hf` on PATH?\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn serve_starts_on_localhost_8080() {
    if !serve_reachable().await {
        eprintln!(
            "SKIPPED: serve_starts_on_localhost_8080 — MLX serve not on :8080 \
             (start with `spectyn mlx serve` first)"
        );
        return;
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let resp = client
        .get("http://127.0.0.1:8080/v1/models")
        .send()
        .await
        .expect("GET /v1/models");
    assert!(
        resp.status().is_success(),
        "/v1/models returned {:?}",
        resp.status()
    );
    let j: serde_json::Value = resp.json().await.expect("json parse");
    let n = j["data"].as_array().map(|a| a.len()).unwrap_or(0);
    assert!(
        n >= 1,
        "/v1/models returned 0 entries; expected >=1 loaded model"
    );
    let id = j["data"][0]["id"].as_str().unwrap_or("");
    assert!(
        id.contains("mlx") || id.contains("Llama") || id.contains("Mistral"),
        "first model id `{}` doesn't look like an MLX-served model",
        id
    );
}

#[tokio::test]
async fn round_trip_local_model() {
    if !serve_reachable().await {
        eprintln!(
            "SKIPPED: round_trip_local_model — MLX serve not on :8080 \
             (start with `spectyn mlx serve` first)"
        );
        return;
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();
    let body = serde_json::json!({
        "model": "mlx-community/Llama-3.1-8B-Instruct-4bit",
        "messages": [{
            "role": "user",
            "content": "Reply with the single word PONG and nothing else."
        }],
        "max_tokens": 8,
        "temperature": 0.0
    });
    let resp = client
        .post("http://127.0.0.1:8080/v1/chat/completions")
        .json(&body)
        .send()
        .await
        .expect("POST /v1/chat/completions");
    assert!(
        resp.status().is_success(),
        "chat completions returned {:?}",
        resp.status()
    );
    let j: serde_json::Value = resp.json().await.expect("json parse");
    let content = j["choices"][0]["message"]["content"]
        .as_str()
        .expect("choices[0].message.content present");
    assert!(!content.trim().is_empty(), "empty completion: {}", j);
    assert!(
        content.to_uppercase().contains("PONG"),
        "completion `{}` doesn't contain PONG — model may be wrong, \
         prompt encoding may have shifted, or temperature isn't actually 0",
        content.trim()
    );
}

#[test]
fn stop_cleans_up_subprocess() {
    let bin = spectyn_bin();
    let pre = Command::new("pgrep")
        .args(["-f", "mlx_lm.server"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    let output = Command::new(bin)
        .args(["mlx", "stop"])
        .output()
        .expect("spectyn mlx stop must spawn");
    assert!(
        output.status.success(),
        "spectyn mlx stop exited {:?}.\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    if !pre {
        eprintln!(
            "SKIPPED: stop_cleans_up_subprocess — no mlx_lm.server was running before \
             stop; nothing to assert"
        );
        return;
    }
    std::thread::sleep(Duration::from_millis(500));
    let post = Command::new("pgrep")
        .args(["-f", "mlx_lm.server"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    assert!(
        !post,
        "spectyn mlx stop returned 0 but `pgrep -f mlx_lm.server` still finds \
         a live process — stop logic failed to clean up"
    );
}
