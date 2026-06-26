//! Hermetic (Unix-gated) contract test for `phantom exec --json`.
//!
//! ## The gap this closes
//!
//! `phantom exec --json` emits one `AgentEvent` JSON per line on stdout — the
//! machine-consumable stream that **cluster RPC dispatch and CI log parsers
//! rely on**. Any drift in that schema (the serde `tag = "type"` discriminator
//! or a renamed variant) silently breaks every downstream consumer.
//!
//! Until now the ONLY coverage of that contract was `cli_macos.rs ::
//! exec_json_stream_macos`, which (a) is `#[cfg(target_os = "macos")]` so it
//! never compiles on Windows/Linux, and (b) early-returns (skips) unless a real
//! provider key (`OPENCODE_API_KEY` / `ANTHROPIC_API_KEY` / `GROQ_API_KEY`) is
//! present in the environment. So on the actual CI runners — Windows + Linux,
//! no provider keys — the exec JSONL schema had **zero** enforced coverage.
//!
//! ## How this test is hermetic
//!
//! It points an OpenAI-compatible provider's base URL at a local **wiremock**
//! server that returns a canned SSE completion, then execs the real built
//! binary (`env!("CARGO_BIN_EXE_phantom")`) with
//! `phantom exec --json --config <tmp>/agents.toml "<prompt>"`. No network, no
//! real API key — the `[providers.mock].url` in the temp `agents.toml` is the
//! documented test seam (`ProviderEntry::url`, alias `base_url`) that the
//! streaming path (`AgentRuntime::call_with_streaming` →
//! `OpenAICompatProvider::build_stream_request`, honouring `base_url_override`)
//! sends to. The inline `api_key = "test-key"` satisfies the key gate without
//! any env var, so the run never SKIPS the way the macOS smoke test does.
//!
//! ## What it asserts
//!
//!   1. `phantom exec --json` exits 0.
//!   2. Every non-empty stdout line parses as JSON (the `--json` contract).
//!   3. At least one line deserialises into the real [`AgentEvent`] shape — the
//!      serde-tagged enum (`{"type":"token", ...}` / `{"type":"done", ...}`) —
//!      and we see a recognised tag. The terminal `Done` event always fires in
//!      `run_with_callbacks`, and the canned SSE chunk additionally yields a
//!      `Token`, so both a content event and the completion event are proven.
//!
//! ## Isolation
//!
//! Runs under a temp data root. `HOME` + `USERPROFILE` (so `ConversationStore`
//! and home-based config resolution land in the temp dir on every OS) and
//! `PHANTOM_HOME` are all pointed at it, and `--config` pins the mocked
//! provider explicitly — so the test never reads or writes the developer's real
//! `~/.phantom-mesh`. Network env that could perturb routing
//! (`PHANTOM_LOCAL_FIRST`, `PHANTOM_RUNTIME_OVERRIDE`) is cleared for the child.

// PLATFORM GATE: the exec child's diag::init() / auto_load_env() resolve the data
// root via bare dirs::home_dir(), which IGNORES HOME/USERPROFILE/PHANTOM_HOME on
// Windows — so on a Windows runner the child would best-effort write events.jsonl
// into the developer's REAL ~/.phantom-mesh. Gate to Unix (where the HOME redirect
// is honoured and the isolation above actually holds) until a PHANTOM_HOME-aware
// resolver lands; Linux CI already covers this platform-agnostic schema contract.
#![cfg(unix)]

use std::process::Command;

use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn phantom_bin() -> &'static str {
    env!("CARGO_BIN_EXE_phantom")
}

/// A deserialise-only mirror of `phantom_mesh::agent::AgentEvent`'s wire shape.
///
/// The real enum is `Serialize`-only and `pub(crate)`-adjacent for some
/// variants, so we can't deserialise into it directly from an integration
/// test. Instead we model the exact serde contract it emits — internally
/// tagged on `type`, `snake_case` variant names — and assert the stdout lines
/// fit it. If the producer ever renames the tag key or a variant, this
/// `#[serde(deny_unknown_fields)]`-free but tag-strict decode stops matching
/// and the test fails, which is the whole point.
// `#[allow(dead_code)]`: the payload fields exist purely to prove they
// DESERIALISE (the schema contract) — most aren't read after decode, which the
// dead-code lint would flag. Beyond the usual noise, rustc 1.95.0 has an ICE
// (annotate_snippets "slice index starts at N but ends at N-1") while RENDERING
// that specific "field is never read" diagnostic for a tagged enum, so allowing
// the lint here is also load-bearing: it stops the compiler from crashing on a
// warning it can't print. The decode itself is the assertion.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
enum WireAgentEvent {
    Token { content: String },
    Thinking { content: String },
    ToolStart { name: String, args_preview: String },
    ToolDone { name: String, output_preview: String },
    Done { output: String, cost_usd: f64, elapsed_secs: f64 },
    Notice { message: String },
}

impl WireAgentEvent {
    fn tag(&self) -> &'static str {
        match self {
            WireAgentEvent::Token { .. } => "token",
            WireAgentEvent::Thinking { .. } => "thinking",
            WireAgentEvent::ToolStart { .. } => "tool_start",
            WireAgentEvent::ToolDone { .. } => "tool_done",
            WireAgentEvent::Done { .. } => "done",
            WireAgentEvent::Notice { .. } => "notice",
        }
    }
}

fn unique_home() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "phantom-exec-jsonl-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// One OpenAI-style SSE chunk carrying a text token, then the `[DONE]`
/// sentinel — exactly what `AgentRuntime::call_with_streaming`'s SSE parser
/// consumes (`data: {…}\n` frames; `choices[0].delta.content` → `Token`).
fn openai_sse_body(token: &str) -> String {
    let chunk = serde_json::json!({
        "id": "chatcmpl-mock",
        "object": "chat.completion.chunk",
        "choices": [{
            "index": 0,
            "delta": { "role": "assistant", "content": token },
            "finish_reason": serde_json::Value::Null
        }]
    });
    let done = serde_json::json!({
        "id": "chatcmpl-mock",
        "object": "chat.completion.chunk",
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": "stop"
        }]
    });
    format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n", chunk, done)
}

#[test]
fn exec_json_stream_hermetic() {
    // ── 1. stand up the mock provider on a short-lived multi-thread runtime ──
    // wiremock needs a live tokio runtime to serve. We start the server, hold
    // the runtime + server for the whole test, and shell out to the binary
    // (its own process) while the server stays reachable.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("multi-thread tokio runtime");

    let server = rt.block_on(async {
        let s = MockServer::start().await;
        // Match any POST (the OpenAI-compat path posts to
        // `<url>/v1/chat/completions`). Return the canned SSE stream.
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(openai_sse_body("PONG")),
            )
            .mount(&s)
            .await;
        s
    });
    let mock_uri = server.uri();

    // ── 2. seed a temp data root with a minimal agents.toml ──────────────────
    // `type = "openai"` → OpenAICompatProvider, whose streaming request honours
    // `base_url_override` (the provider's `url`). We deliberately do NOT name
    // the provider `gemini` / give it `type = "gemini"`, because the agent
    // short-circuits gemini to a non-streaming native path that would bypass
    // the SSE parser this test is asserting on.
    let home = unique_home();
    let pm = home.join(".phantom-mesh");
    std::fs::create_dir_all(&pm).expect("create temp .phantom-mesh");
    let agents_toml = format!(
        r#"
[agent.master]
provider = "mock"
model = "mock-model"

[providers.mock]
type = "openai"
url = "{uri}"
api_key = "test-key"
default_model = "mock-model"
"#,
        uri = mock_uri
    );
    let config_path = pm.join("agents.toml");
    std::fs::write(&config_path, agents_toml).expect("write temp agents.toml");

    // ── 3. exec the real binary in --json mode against the mock provider ─────
    // Run the blocking child on the runtime's blocking pool so the reactor
    // thread serving wiremock stays free while the child streams from it.
    let bin = phantom_bin();
    let home_s = home.to_string_lossy().to_string();
    let config_s = config_path.to_string_lossy().to_string();
    let output = rt.block_on(async move {
        tokio::task::spawn_blocking(move || {
            Command::new(bin)
                .args([
                    "exec",
                    "--json",
                    "--config",
                    &config_s,
                    "Reply with the literal word PONG and nothing else.",
                ])
                // Redirect every home-resolution seam at the temp dir so the
                // child never touches the real ~/.phantom-mesh. ConversationStore
                // reads HOME → USERPROFILE → dirs::home_dir(); cover the first two
                // (the third is platform fallback only).
                .env("HOME", &home_s)
                .env("USERPROFILE", &home_s)
                .env("PHANTOM_HOME", &home_s)
                // Neutralise routing-perturbing env that could reorder/override
                // the provider chain out from under the mock.
                .env_remove("PHANTOM_LOCAL_FIRST")
                .env_remove("PHANTOM_RUNTIME_OVERRIDE")
                .output()
                .expect("phantom exec --json must spawn")
        })
        .await
        .expect("join exec spawn_blocking")
    });

    // Server + runtime are no longer needed once the child has returned.
    drop(server);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // ── 4a. exit code contract ───────────────────────────────────────────────
    assert!(
        output.status.success(),
        "phantom exec --json exited {:?} (expected 0).\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        stdout,
        stderr,
    );

    // ── 4b. there must be at least one stdout line ───────────────────────────
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        !lines.is_empty(),
        "phantom exec --json produced no stdout lines — stream never started?\n\
         stderr:\n{}",
        stderr,
    );

    // ── 4c. every line is JSON; ≥1 decodes into the AgentEvent contract ──────
    let mut parsed_json = 0usize;
    let mut decoded_event = 0usize;
    let mut seen_tags: Vec<String> = Vec::new();
    for line in &lines {
        // Plain-JSON gate (the `--json` promise: machine-readable, one obj/line).
        let value: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|e| {
            panic!(
                "non-JSON line in --json stream: `{line}` (error: {e})\n\
                 full stdout:\n{stdout}"
            )
        });
        parsed_json += 1;

        // Schema gate: the line must fit the real AgentEvent serde shape
        // (internally tagged on `type`). A line that is valid JSON but does
        // NOT carry a recognised `type` tag means the contract drifted.
        if let Ok(ev) = serde_json::from_value::<WireAgentEvent>(value) {
            decoded_event += 1;
            seen_tags.push(ev.tag().to_string());
        }
    }

    assert!(
        parsed_json > 0,
        "no JSON lines parsed out of {} non-empty stdout lines",
        lines.len()
    );
    assert!(
        decoded_event > 0,
        "no stdout line decoded into the AgentEvent schema — the `type`-tagged \
         contract that cluster RPC + CI parsers depend on may have drifted.\n\
         lines were:\n{}",
        lines.join("\n"),
    );

    // The terminal `Done` event always fires in `run_with_callbacks`; the canned
    // SSE chunk additionally yields a `Token`. Require the completion tag at
    // minimum so a regression that stops emitting `Done` is caught.
    assert!(
        seen_tags.iter().any(|t| t == "done"),
        "expected a `done` AgentEvent terminating the --json stream; saw tags: {:?}\n\
         stderr:\n{}",
        seen_tags,
        stderr,
    );
}
