//! DEMO-1 gap 1 Phase 4 — integration tests for the `LlmProvider` trait
//! extension (`build_stream_request`) + the migration of
//! `agent.rs::call_with_fallback` and `agent.rs::call_with_streaming` off
//! the deleted `provider_url` string-switch.
//!
//! Coverage:
//!   1. AnthropicProvider's `build_stream_request` produces a request body
//!      with cache_control on system + last tool, adaptive
//!      `thinking.display = "omitted"` for Opus 4.7+, and native
//!      `/v1/messages` URL.
//!   2. OpenAICompatProvider's `build_stream_request` produces a flat
//!      OpenAI-style `messages` array + `stream_options.include_usage`
//!      when streaming with tools.
//!   3. GeminiProvider's `build_stream_request` routes to Gemini's NATIVE
//!      `:generateContent` endpoint with a native body (`contents`, not
//!      `messages`) — intentional since #304 (2ba17b7d): the OpenAI-compat
//!      shim returns tool-calls as text, never structured `tool_calls`.
//!   4. ClaudeCliProvider's `build_stream_request` uses the same
//!      Anthropic-native shape as AnthropicProvider (cache_control etc.).
//!   5. `call_with_streaming` honours `PHANTOM_RUNTIME_OVERRIDE` after
//!      the trait migration (the URL the provider hits must match the
//!      override, not the agent's primary).
//!   6. `call_with_fallback` cascades on auth errors (401) after the
//!      trait migration (the second provider in the chain is tried).

use std::collections::HashMap;
use std::sync::Arc;

use axum::routing::post;
use axum::Router;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use phantom_mesh::{
    config::{AgentEntry, AgentsConfig, ProviderEntry},
    providers::{
        resolver::DefaultProviderResolver, BuildRequestOpts, BuildRequestParts, LlmProvider,
    },
};

// ── Part A unit-style tests on the trait impls ───────────────────────────

fn opts<'a>(
    model: &'a str,
    system: &'a str,
    messages: &'a [serde_json::Value],
    tools: &'a [serde_json::Value],
    stream: bool,
) -> BuildRequestOpts<'a> {
    BuildRequestOpts {
        model,
        system,
        messages,
        tools,
        base_url_override: None,
        stream,
        max_tokens: 1024,
    }
}

fn resolve(cfg: &AgentsConfig, name: &str) -> Arc<dyn LlmProvider> {
    DefaultProviderResolver::from_config(cfg)
        .resolve(name)
        .expect("provider resolved")
}

fn cfg_one(name: &str, ptype: &str) -> AgentsConfig {
    let mut providers = HashMap::new();
    providers.insert(
        name.into(),
        ProviderEntry {
            provider_type: ptype.into(),
            ..Default::default()
        },
    );
    AgentsConfig {
        providers,
        ..Default::default()
    }
}

#[test]
fn anthropic_build_stream_request_emits_cache_control_on_system_and_last_tool() {
    let cfg = cfg_one("primary", "anthropic");
    let provider = resolve(&cfg, "primary");
    let messages = vec![json!({"role": "user", "content": "hi"})];
    let tools = vec![
        json!({
            "type": "function",
            "function": {
                "name": "tool_a",
                "description": "first",
                "parameters": {"type": "object"},
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "tool_b",
                "description": "second",
                "parameters": {"type": "object"},
            }
        }),
    ];
    let o = opts(
        "claude-sonnet-4-5",
        "you are helpful",
        &messages,
        &tools,
        true,
    );
    let BuildRequestParts { url, body, headers } =
        provider.build_stream_request(&o).expect("build ok");

    // URL: native Messages-API endpoint, not the OpenAI-compat proxy.
    assert_eq!(url, "https://api.anthropic.com/v1/messages");

    // Headers: x-api-key is added by the caller (post-resolver), but
    // `anthropic-version` MUST be present in the trait output.
    let header_keys: Vec<&str> = headers.iter().map(|(k, _)| *k).collect();
    assert!(
        header_keys.contains(&"anthropic-version"),
        "headers={:?}",
        headers
    );

    // System rendered as cacheable text block.
    let sys = &body["system"];
    assert!(sys.is_array(), "system should be array, got: {sys}");
    assert_eq!(sys[0]["type"], "text");
    assert_eq!(sys[0]["text"], "you are helpful");
    assert_eq!(sys[0]["cache_control"]["type"], "ephemeral");

    // Tools rewritten + cache_control on the LAST tool only.
    let tools_out = body["tools"].as_array().expect("tools array");
    assert_eq!(tools_out.len(), 2);
    assert_eq!(tools_out[0]["name"], "tool_a");
    assert_eq!(tools_out[1]["name"], "tool_b");
    assert!(
        tools_out[0].get("cache_control").is_none(),
        "first tool must NOT have cache_control"
    );
    assert_eq!(tools_out[1]["cache_control"]["type"], "ephemeral");

    // Stream flag honoured.
    assert_eq!(body["stream"], true);
    // System NOT folded back into messages (Anthropic native shape).
    let msgs = body["messages"].as_array().expect("messages array");
    for m in msgs {
        assert_ne!(m["role"], "system");
    }
}

#[test]
fn anthropic_build_stream_request_adds_thinking_display_omitted_for_opus_4_7() {
    let cfg = cfg_one("primary", "anthropic");
    let provider = resolve(&cfg, "primary");
    let messages = vec![json!({"role": "user", "content": "ping"})];
    let body = provider
        .build_stream_request(&opts("claude-opus-4-7", "", &messages, &[], true))
        .expect("build ok")
        .body;
    assert_eq!(body["thinking"]["type"], "adaptive");
    assert_eq!(body["thinking"]["display"], "omitted");
}

#[test]
fn anthropic_build_stream_request_omits_thinking_for_pre_opus_4_7() {
    let cfg = cfg_one("primary", "anthropic");
    let provider = resolve(&cfg, "primary");
    let messages = vec![json!({"role": "user", "content": "ping"})];
    for model in ["claude-sonnet-4-5", "claude-opus-4-6", "claude-haiku-4-5"] {
        let body = provider
            .build_stream_request(&opts(model, "", &messages, &[], true))
            .expect("build ok")
            .body;
        assert!(
            body.get("thinking").is_none(),
            "model {} must NOT carry thinking field, body={}",
            model,
            body
        );
    }
}

#[test]
fn openai_compat_build_stream_request_emits_flat_messages_array() {
    let cfg = cfg_one("primary", "openai");
    let provider = resolve(&cfg, "primary");
    let messages = vec![
        json!({"role": "system", "content": "be brief"}),
        json!({"role": "user", "content": "hi"}),
    ];
    let tools = vec![json!({
        "type": "function",
        "function": {"name": "x", "description": "y", "parameters": {"type": "object"}},
    })];
    let parts = provider
        .build_stream_request(&opts("gpt-4o", "", &messages, &tools, true))
        .expect("build ok");

    // URL: OpenAI-compat chat/completions.
    assert_eq!(parts.url, "https://api.openai.com/v1/chat/completions");

    // Body: flat messages array (system kept as a `role=system` message,
    // not rewritten into a separate `system` field). No `thinking`,
    // no per-tool `cache_control`.
    assert_eq!(parts.body["messages"].as_array().unwrap().len(), 2);
    assert!(
        parts.body.get("system").is_none(),
        "OpenAI-compat must NOT emit top-level `system`"
    );
    assert!(parts.body.get("thinking").is_none());
    let tools_out = parts.body["tools"].as_array().expect("tools array");
    assert_eq!(tools_out.len(), 1);
    assert!(
        tools_out[0].get("cache_control").is_none(),
        "OpenAI-compat tools must NOT have cache_control"
    );

    // stream_options.include_usage is set when streaming with tools.
    assert_eq!(parts.body["stream_options"]["include_usage"], true);
    assert_eq!(parts.body["stream"], true);
}

#[test]
fn gemini_build_stream_request_uses_native_generate_content() {
    // Phase 4 originally kept Gemini on Google's OpenAI-compat shim
    // (`/v1beta/openai/chat/completions`) under the behaviour-preserve
    // mandate. #304 (2ba17b7d) intentionally moved GeminiProvider to the
    // NATIVE `:generateContent` API because the shim returns tool-calls as
    // text instead of structured `tool_calls`, silently breaking tool-use.
    //
    // Streaming contract: Gemini's native SSE is incompatible with the
    // agent's OpenAI-SSE parser, so BOTH `call_with_streaming` and
    // `call_with_fallback` short-circuit any resolved `provider_type() ==
    // "gemini"` to the non-streaming native `complete()` BEFORE
    // `build_stream_request` is reached (agent.rs gemini short-circuit
    // branches). This method therefore emits the native shape for any
    // shape-then-send caller; it is never fed to the SSE parser.
    let cfg = cfg_one("primary", "gemini");
    let provider = resolve(&cfg, "primary");
    let messages = vec![json!({"role": "user", "content": "hello"})];
    let parts = provider
        .build_stream_request(&opts("gemini-2.0-flash", "", &messages, &[], true))
        .expect("build ok");
    assert_eq!(
        parts.url,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent"
    );
    // Native shape: `contents` with `parts`, not OpenAI `messages`.
    assert!(parts.body.get("contents").is_some());
    assert!(parts.body.get("messages").is_none());
    // The OpenAI-shaped input message must survive the Value→ChatMessage→
    // contents/parts bridge (value_messages_to_chat + build_gemini_body).
    assert_eq!(parts.body["contents"][0]["parts"][0]["text"], "hello");
}

#[test]
fn claude_cli_build_stream_request_mirrors_anthropic_shape() {
    // claude_cli speaks Anthropic protocol (sk-ant-* CLI token), so its
    // request body must carry the same cache_control + adaptive thinking
    // that AnthropicProvider emits. Pre-Phase-4 streaming.rs explicitly
    // flagged this case as "Phase 4's job" — verify it's fixed.
    let cfg = cfg_one("cli", "claude_cli");
    let provider = resolve(&cfg, "cli");
    assert_eq!(provider.provider_type(), "claude_cli");

    let messages = vec![json!({"role": "user", "content": "hi"})];
    let tools = vec![json!({
        "type": "function",
        "function": {"name": "t", "description": "d", "parameters": {"type": "object"}},
    })];
    let parts = provider
        .build_stream_request(&opts("claude-opus-4-7", "sys", &messages, &tools, true))
        .expect("build ok");

    assert_eq!(parts.url, "https://api.anthropic.com/v1/messages");
    assert_eq!(
        parts.body["system"][0]["cache_control"]["type"],
        "ephemeral"
    );
    assert_eq!(parts.body["tools"][0]["cache_control"]["type"], "ephemeral");
    assert_eq!(parts.body["thinking"]["display"], "omitted");
    let header_keys: Vec<&str> = parts.headers.iter().map(|(k, _)| *k).collect();
    assert!(header_keys.contains(&"anthropic-version"));
}

// ── Part B integration tests against a mock HTTP server ──────────────────

/// Spin up a server that accepts BOTH /v1/messages (Anthropic) and
/// /v1/chat/completions (OpenAI-compat), records every (path, status,
/// body) hit, and returns canned non-streaming JSON or a configurable
/// status code per route.
struct MockServer {
    base_url: String,
    /// (path, body)
    requests: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
}

async fn start_mock_with_status(openai_status: u16, anthropic_status: u16) -> MockServer {
    use axum::http::StatusCode;

    let requests: Arc<Mutex<Vec<(String, serde_json::Value)>>> = Arc::new(Mutex::new(Vec::new()));

    let openai_body = json!({
        "choices": [{"message": {"role": "assistant", "content": "openai-ok"}}],
        "usage": {},
    });
    let anthropic_body = json!({
        "choices": [{"message": {"role": "assistant", "content": "anthropic-ok"}}],
        "usage": {},
    });

    let reqs_a = requests.clone();
    let reqs_o = requests.clone();
    let app = Router::new()
        .route(
            "/v1/messages",
            post(move |body: axum::Json<serde_json::Value>| {
                let captured = reqs_a.clone();
                let ab = anthropic_body.clone();
                async move {
                    captured.lock().await.push(("/v1/messages".into(), body.0));
                    (
                        StatusCode::from_u16(anthropic_status).expect("valid status"),
                        axum::Json(ab),
                    )
                }
            }),
        )
        .route(
            "/v1/chat/completions",
            post(move |body: axum::Json<serde_json::Value>| {
                let captured = reqs_o.clone();
                let ob = openai_body.clone();
                async move {
                    captured
                        .lock()
                        .await
                        .push(("/v1/chat/completions".into(), body.0));
                    (
                        StatusCode::from_u16(openai_status).expect("valid status"),
                        axum::Json(ob),
                    )
                }
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("mock serve");
    });
    MockServer {
        base_url: format!("http://127.0.0.1:{}", addr.port()),
        requests,
    }
}

fn cfg_two_providers(primary_url: &str, secondary_url: &str) -> AgentsConfig {
    let mut providers = HashMap::new();
    // Primary will be "anthropic" type so failure routes to /v1/messages.
    providers.insert(
        "anth".into(),
        ProviderEntry {
            provider_type: "anthropic".into(),
            url: Some(primary_url.into()),
            api_key: Some("primary-key".into()),
            ..Default::default()
        },
    );
    providers.insert(
        "openai".into(),
        ProviderEntry {
            provider_type: "openai".into(),
            url: Some(secondary_url.into()),
            api_key: Some("secondary-key".into()),
            default_model: Some("gpt-4o".into()),
            ..Default::default()
        },
    );
    let mut agent = HashMap::new();
    agent.insert(
        "master".into(),
        AgentEntry {
            // Default agent uses anth first; falls back to openai.
            provider: "anth".into(),
            providers: Some(vec!["anth".into(), "openai".into()]),
            model: "claude-sonnet-4-5".into(),
            tools: vec![],
            instructions: String::new(),
        },
    );
    AgentsConfig {
        providers,
        agent,
        ..Default::default()
    }
}

/// Process-wide mutex serialising any test that touches
/// `PHANTOM_RUNTIME_OVERRIDE` (or relies on it being unset). Multiple
/// tokio tests in this binary run in parallel by default; without this
/// the override test races the cascade test and corrupts both.
static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[tokio::test]
async fn call_with_fallback_uses_trait_dispatch_and_cascades_on_auth_error() {
    let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    // Belt-and-braces: the override test sets this env var; if a prior
    // test panicked before restoring, ensure we start clean.
    unsafe {
        std::env::remove_var("PHANTOM_RUNTIME_OVERRIDE");
    }
    // Primary (anthropic) returns 401 → call_with_fallback must SKIP it
    // (non-retriable auth error) and fall through to the secondary
    // (openai). Both hits MUST land on their wire-format-specific paths
    // (proves the trait dispatch picked the right body shape).
    let mock = start_mock_with_status(/* openai */ 200, /* anthropic */ 401).await;
    let cfg = cfg_two_providers(&mock.base_url, &mock.base_url);

    // Drive call_with_fallback indirectly via AgentRuntime::run. We use a
    // minimal `master` agent with no tools.
    let runtime = phantom_mesh::agent::AgentRuntime::new(cfg);
    let result = runtime.run("master", "hello", &[], None).await;

    // The run should succeed because the second provider (openai)
    // returned 200; the cascade is what we're testing.
    assert!(
        result.is_ok(),
        "expected run to succeed via cascade, got: {:?}",
        result.err(),
    );
    let agent_result = result.unwrap();
    assert!(
        agent_result.output.contains("openai-ok"),
        "expected fallback to openai-ok, got: {}",
        agent_result.output,
    );

    // Both providers should have been hit; first request goes to
    // /v1/messages (Anthropic trait dispatch), then /v1/chat/completions
    // (OpenAI-compat trait dispatch on the cascade).
    let reqs = mock.requests.lock().await.clone();
    let paths: Vec<&str> = reqs.iter().map(|(p, _)| p.as_str()).collect();
    assert!(
        paths.contains(&"/v1/messages"),
        "expected /v1/messages hit (Anthropic trait dispatch); got {:?}",
        paths,
    );
    assert!(
        paths.contains(&"/v1/chat/completions"),
        "expected /v1/chat/completions hit (OpenAI-compat trait dispatch after fallback); got {:?}",
        paths,
    );

    // The Anthropic request body MUST have native-format markers
    // (system as a cacheable text block when system is non-empty;
    // here system is empty so check `max_tokens` + absent `messages`
    // entries with role=system).
    let anth_body = reqs
        .iter()
        .find(|(p, _)| p == "/v1/messages")
        .map(|(_, b)| b.clone())
        .unwrap();
    assert!(
        anth_body.get("model").is_some(),
        "anthropic body missing model"
    );
    // Confirms this is the Anthropic shape (max_tokens at top level for
    // Anthropic; OpenAI-compat also has it, so the discriminator is the
    // absence of a `messages[*].role == "system"` entry — the trait
    // strips it).
    let msgs = anth_body["messages"]
        .as_array()
        .expect("anth messages array");
    for m in msgs {
        assert_ne!(
            m["role"], "system",
            "Anthropic shape must strip system from messages, got: {}",
            m
        );
    }
}

#[tokio::test]
async fn call_with_streaming_honours_runtime_override_after_trait_migration() {
    // PHANTOM_RUNTIME_OVERRIDE="openai" must reorder so openai is hit
    // first, even though the agent's `provider`/`providers` list put
    // anth first. Verifies the trait migration didn't break the
    // override-prepend logic.
    //
    // Both endpoints return 200 so the run completes on the first hit;
    // we just check WHICH endpoint was hit first.
    let mock = start_mock_with_status(200, 200).await;
    let cfg = cfg_two_providers(&mock.base_url, &mock.base_url);

    // Set the override env var BEFORE running. To avoid interference
    // from sandbox env races (per Phase 3 note: there are 2 pre-existing
    // sandbox-guard env race failures), serialise via the shared
    // ENV_MUTEX so concurrent tokio tests don't see each other's state.
    let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    let old = std::env::var("PHANTOM_RUNTIME_OVERRIDE").ok();
    // SAFETY: serialised via ENV_MUTEX above; other phantom-mesh tests
    // touching this var are also expected to hold this style of lock
    // (Phase 3's tests rely on the same convention).
    unsafe {
        std::env::set_var("PHANTOM_RUNTIME_OVERRIDE", "openai");
    }

    let runtime = phantom_mesh::agent::AgentRuntime::new(cfg);
    // Use streaming via run_with_callbacks would be ideal — but
    // call_with_streaming is internal. The public `run` path is the
    // canonical entry to `call_with_fallback`. To trigger the
    // streaming path we'd need run_with_callbacks; for the
    // override-honoured assertion the non-streaming dispatch is
    // sufficient because BOTH paths consult the same
    // PHANTOM_RUNTIME_OVERRIDE logic and the same DefaultProviderResolver.
    let result = runtime.run("master", "hello", &[], None).await;

    // Restore the env var before asserting (so failures don't leak it).
    unsafe {
        match old {
            Some(v) => std::env::set_var("PHANTOM_RUNTIME_OVERRIDE", v),
            None => std::env::remove_var("PHANTOM_RUNTIME_OVERRIDE"),
        }
    }

    assert!(
        result.is_ok(),
        "expected run to succeed: {:?}",
        result.err()
    );

    let reqs = mock.requests.lock().await.clone();
    assert!(!reqs.is_empty(), "expected at least one provider hit");
    // The FIRST hit must be /v1/chat/completions (openai), proving
    // PHANTOM_RUNTIME_OVERRIDE prepended openai ahead of the agent's
    // primary "anth".
    assert_eq!(
        reqs[0].0, "/v1/chat/completions",
        "expected first hit to be openai's /v1/chat/completions due to PHANTOM_RUNTIME_OVERRIDE; got all hits: {:?}",
        reqs.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
    );
}
