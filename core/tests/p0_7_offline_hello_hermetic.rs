//! P0-7 S4 — the apex 30-second 'hello' works OFFLINE with no cloud.
//!
//! Two hermetic proofs, both with HTTP(S)_PROXY black-holed so a stray outbound
//! call fails fast instead of silently hitting the real internet:
//!   1. wiremock-local-model: `spectyn exec` against an OpenAI-compat provider
//!      whose base_url points at a localhost wiremock — proves the real local
//!      model code path answers under the 30s budget, no cloud. (Independent of
//!      S2; same seam as cli_exec_jsonl_schema_hermetic.rs.)
//!   2. (feature offline-stub-model) stub-model: `spectyn exec` with NO provider
//!      url at all, only the built-in stub — proves the truly-zero-install path
//!      answers offline.
//!
//! Unix-gated for the same reason as cli_exec_jsonl_schema_hermetic.rs: the exec
//! child resolves its data root via bare dirs::home_dir(), which ignores the
//! HOME/USERPROFILE/SPECTYN_HOME redirect on Windows, so the isolation only
//! holds on Unix. On Windows this compiles to an empty binary and passes
//! trivially; it runs for real on WSL / Linux CI.
#![cfg(unix)]

use std::process::Command;
use std::time::{Duration, Instant};

use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const HELLO_BUDGET: Duration = Duration::from_secs(30);

fn spectyn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_spectyn")
}

fn temp_root(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "spectyn-p07-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// One OpenAI-style SSE token chunk then the `[DONE]` sentinel — exactly what
/// `AgentRuntime::call_with_streaming`'s SSE parser consumes.
fn openai_sse(token: &str) -> String {
    let c = serde_json::json!({
        "choices": [{
            "index": 0,
            "delta": {"role": "assistant", "content": token},
            "finish_reason": serde_json::Value::Null
        }]
    });
    let d = serde_json::json!({
        "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]
    });
    format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n", c, d)
}

#[test]
fn offline_hello_local_model_under_budget() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("multi-thread tokio runtime");
    let server = rt.block_on(async {
        let s = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(openai_sse("PONG")),
            )
            .mount(&s)
            .await;
        s
    });
    let uri = server.uri();

    let root = temp_root("hello");
    let pm = root.join(".spectyn-mesh");
    std::fs::create_dir_all(&pm).unwrap();
    // Local-model config: type=openai (OpenAICompatProvider) honouring base_url.
    // An inline api_key satisfies the key gate WITHOUT any env var (so it never
    // SKIPs the provider).
    let config_path = pm.join("agents.toml");
    std::fs::write(
        &config_path,
        format!(
            "[providers.localmodel]\ntype = \"openai\"\nurl = \"{uri}\"\napi_key = \"test\"\ndefault_model = \"local\"\n\n[agent.master]\nprovider = \"localmodel\"\nproviders = [\"localmodel\"]\n"
        ),
    )
    .unwrap();

    let bin = spectyn_bin();
    let root_s = root.to_string_lossy().to_string();
    let config_s = config_path.to_string_lossy().to_string();
    let start = Instant::now();
    let out = rt.block_on(async move {
        tokio::task::spawn_blocking(move || {
            Command::new(bin)
                .args(["exec", "say hello", "--config", &config_s])
                .env("HOME", &root_s)
                .env("USERPROFILE", &root_s)
                .env("SPECTYN_HOME", &root_s)
                // Black-hole any CLOUD (non-loopback) HTTP so a stray outbound
                // call fails fast — but EXEMPT loopback (NO_PROXY) so the local
                // wiremock model is reachable. Without the exemption ALL_PROXY
                // would route the localhost request through the dead proxy too.
                .env("HTTP_PROXY", "http://127.0.0.1:1")
                .env("HTTPS_PROXY", "http://127.0.0.1:1")
                .env("ALL_PROXY", "http://127.0.0.1:1")
                .env("NO_PROXY", "127.0.0.1,localhost")
                .env("no_proxy", "127.0.0.1,localhost")
                .env_remove("SPECTYN_LOCAL_FIRST")
                .env_remove("SPECTYN_RUNTIME_OVERRIDE")
                .output()
                .expect("run spectyn exec")
        })
        .await
        .expect("join exec spawn_blocking")
    });
    let elapsed = start.elapsed();
    drop(server);

    assert!(
        out.status.success(),
        "offline exec must succeed; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("PONG"),
        "must stream the local model reply; stdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        elapsed <= HELLO_BUDGET,
        "offline hello {:?} exceeded 30s budget",
        elapsed
    );
    let _ = std::fs::remove_dir_all(&root);
    drop(rt);
}

#[cfg(feature = "offline-stub-model")]
#[test]
fn offline_hello_stub_model_nothing_installed() {
    // No provider url at all — rely on the built-in stub. Proves the truly
    // zero-install path answers offline.
    let root = temp_root("stub");
    let pm = root.join(".spectyn-mesh");
    std::fs::create_dir_all(&pm).unwrap();
    std::fs::write(
        pm.join("agents.toml"),
        "[providers.local-stub]\ntype = \"stub\"\nurl = \"stub://offline\"\ndefault_model = \"stub-echo\"\n\n[agent.master]\nprovider = \"local-stub\"\nproviders = [\"local-stub\"]\n",
    )
    .unwrap();
    let config_s = pm.join("agents.toml").to_string_lossy().to_string();
    let root_s = root.to_string_lossy().to_string();
    let start = Instant::now();
    let out = Command::new(spectyn_bin())
        .args(["exec", "say hello", "--config", &config_s])
        .env("HOME", &root_s)
        .env("USERPROFILE", &root_s)
        .env("SPECTYN_HOME", &root_s)
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .env_remove("SPECTYN_LOCAL_FIRST")
        .env_remove("SPECTYN_RUNTIME_OVERRIDE")
        .output()
        .expect("run spectyn exec");
    assert!(
        out.status.success(),
        "stub exec must succeed offline; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("spectyn offline (stub)"),
        "must render the built-in stub reply; stdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(start.elapsed() <= HELLO_BUDGET);
    let _ = std::fs::remove_dir_all(&root);
}
