//! DEMO-1 gap 1 Phase 3 — integration tests for the `LlmProvider` trait
//! migration of `streaming.rs`.
//!
//! Verifies:
//!   1. The migrated `streaming.rs::stream_one_round` path actually consults
//!      the injected `ResolveProvider` (recorded via a `MockProvider`).
//!   2. Swapping the resolver between calls changes which provider is
//!      dispatched to — proving the trait indirection is the dispatch
//!      point, not a hard-coded string-switch.
//!
//! These tests intentionally target the `_with_resolver` variant (public for
//! exactly this purpose) so they can install a `MockResolver` without
//! relying on `DefaultProviderResolver`'s string-switch passthrough.
//!
//! ## Why the test uses a real SSE mock server
//!
//! The trait's `LlmProvider::stream` builds its own request body, but
//! `streaming.rs` builds its body in `build_request_body` (so it can apply
//! the Anthropic-specific `cache_control` + adaptive `thinking` fields).
//! For Phase 3, the trait is consulted for **provider-type identification**
//! (which drives `is_anthropic` and therefore the body shape and the URL),
//! but the actual HTTP request is sent by streaming.rs's existing client.
//!
//! So to verify the trait was consulted, we:
//!   * point the provider entry at a local axum mock SSE server,
//!   * have the resolver return a `MockProvider` whose `provider_type()`
//!     returns either `"anthropic"` or `"openai"`,
//!   * check what URL path the mock server received (Anthropic →
//!     `/v1/messages`, OpenAI → `/v1/chat/completions`).
//!
//! Different wire format = trait was honoured. Same wire format across
//! swaps = trait was ignored (test failure).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use axum::routing::post;
use axum::Router;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use spectyn_mesh::{
    config::{AgentEntry, AgentsConfig, ProviderEntry},
    providers::{
        traits::{ChatMessage, ProviderError},
        LlmProvider,
    },
    streaming::{stream_agent_full_with_resolver, ResolveProvider, StreamEvent, StreamResult},
};

const NO_HISTORY: &[ChatMessage] = &[];

// ── MockProvider + MockResolver ──────────────────────────────────────────

/// A `LlmProvider` that records every call to `provider_type()` and returns
/// a configurable type id. The `stream` / `complete` methods are unused by
/// the streaming.rs migration path (Phase 3 only consults `provider_type`)
/// but are stubbed to satisfy the trait.
struct MockProvider {
    type_id: &'static str,
    provider_type_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl LlmProvider for MockProvider {
    async fn stream(
        &self,
        _api_key: &str,
        _model: &str,
        _messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> Result<reqwest::Response, ProviderError> {
        Err(ProviderError::Unknown(
            "MockProvider::stream not implemented".into(),
        ))
    }

    async fn complete(
        &self,
        _api_key: &str,
        _model: &str,
        _messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> Result<(ChatMessage, serde_json::Value), ProviderError> {
        Err(ProviderError::Unknown(
            "MockProvider::complete not implemented".into(),
        ))
    }

    fn provider_type(&self) -> &'static str {
        self.provider_type_calls.fetch_add(1, Ordering::SeqCst);
        self.type_id
    }
}

/// A `ResolveProvider` that always returns the same canned `MockProvider`
/// instance and records every `resolve_by_name` call.
struct MockResolver {
    provider: Arc<dyn LlmProvider>,
    resolve_calls: Arc<AtomicUsize>,
    last_name: Arc<Mutex<Option<String>>>,
}

impl ResolveProvider for MockResolver {
    fn resolve_by_name(&self, name: &str) -> Option<Arc<dyn LlmProvider>> {
        self.resolve_calls.fetch_add(1, Ordering::SeqCst);
        // Spawn-blocking-safe: tokio::sync::Mutex's blocking_lock would
        // panic in async context. Use try_lock with a spin fallback since
        // contention here is effectively never (single producer in test).
        if let Ok(mut guard) = self.last_name.try_lock() {
            *guard = Some(name.to_string());
        }
        Some(self.provider.clone())
    }
}

// ── SSE mock server (records the request path) ──────────────────────────

/// Spin up a mock HTTP server that:
///   * accepts BOTH `/v1/messages` (Anthropic) and `/v1/chat/completions`
///     (OpenAI-compat) on the same port,
///   * records the path of the last incoming POST into the supplied
///     `Arc<Mutex<Option<String>>>`,
///   * returns a minimal valid SSE body matching the wire format of the
///     hit path so `stream_one_round` finishes cleanly.
async fn start_dual_format_sse_mock(captured_path: Arc<Mutex<Option<String>>>) -> String {
    use axum::http::StatusCode;

    let anthropic_sse = "event: message_start\n\
        data: {\"type\":\"message_start\",\"message\":{\"id\":\"x\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n\
        event: content_block_delta\n\
        data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n\
        event: message_stop\n\
        data: {\"type\":\"message_stop\"}\n\n";
    let openai_sse =
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"hi\"}}]}\n\n\
        data: [DONE]\n\n";

    let anthropic_path = captured_path.clone();
    let openai_path = captured_path.clone();

    let app = Router::new()
        .route(
            "/v1/messages",
            post(move || {
                let captured = anthropic_path.clone();
                async move {
                    *captured.lock().await = Some("/v1/messages".into());
                    axum::response::Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", "text/event-stream")
                        .body(axum::body::Body::from(anthropic_sse))
                        .expect("build anthropic SSE response")
                }
            }),
        )
        .route(
            "/v1/chat/completions",
            post(move || {
                let captured = openai_path.clone();
                async move {
                    *captured.lock().await = Some("/v1/chat/completions".into());
                    axum::response::Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", "text/event-stream")
                        .body(axum::body::Body::from(openai_sse))
                        .expect("build openai SSE response")
                }
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind dual-format SSE mock");
    let addr = listener.local_addr().expect("local addr");

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("dual-format SSE mock error");
    });

    format!("http://127.0.0.1:{}", addr.port())
}

/// Build a config with a single provider whose URL points at our mock.
/// We register it under `provider_type: "openai_compat"` so the legacy
/// `is_anthropic` string-compare in streaming.rs would say "openai-compat";
/// the trait migration is what flips it to Anthropic when the MockProvider
/// reports `provider_type = "anthropic"`.
fn cfg_pointing_at(base_url: &str) -> AgentsConfig {
    let mut providers = std::collections::HashMap::new();
    providers.insert(
        "mock".into(),
        ProviderEntry {
            // Deliberately NOT "anthropic" — proves the trait override wins
            // over the legacy string-compare against ProviderEntry.provider_type.
            provider_type: "openai_compat".into(),
            url: Some(base_url.to_string()),
            api_key: Some("test-key".into()),
            api_key_env: None,
            default_model: None,
            tier: None,
        },
    );
    let mut agent = std::collections::HashMap::new();
    agent.insert(
        "master".into(),
        AgentEntry {
            provider: "mock".into(),
            providers: None,
            model: "mock-model".into(),
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

// ── Test 1: trait-mocked stream + counter proves the trait was used ─────

#[tokio::test]
async fn stream_response_uses_provided_provider_via_trait() {
    let captured_path: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let base_url = start_dual_format_sse_mock(captured_path.clone()).await;
    let config = cfg_pointing_at(&base_url);

    // MockProvider claims to be "anthropic". The provider entry's
    // provider_type is "openai_compat" — so if the legacy string-compare
    // were still in effect, the request would go to /v1/chat/completions.
    // The trait migration MUST route it to /v1/messages.
    let pt_calls = Arc::new(AtomicUsize::new(0));
    let resolve_calls = Arc::new(AtomicUsize::new(0));
    let mock_provider = Arc::new(MockProvider {
        type_id: "anthropic",
        provider_type_calls: pt_calls.clone(),
    });
    let resolver = Arc::new(MockResolver {
        provider: mock_provider as Arc<dyn LlmProvider>,
        resolve_calls: resolve_calls.clone(),
        last_name: Arc::new(Mutex::new(None)),
    });

    let mut tokens: Vec<String> = Vec::new();
    let tokens_collector = std::sync::Mutex::new(Vec::<String>::new());

    let result: anyhow::Result<StreamResult> = stream_agent_full_with_resolver(
        &config,
        "master",
        "ping",
        NO_HISTORY,
        None,
        None,
        resolver as Arc<dyn ResolveProvider>,
        |event: StreamEvent| {
            if let StreamEvent::Token { content } = event {
                tokens_collector.lock().unwrap().push(content);
            }
        },
    )
    .await;

    tokens.extend(tokens_collector.into_inner().unwrap());
    assert!(
        result.is_ok(),
        "streaming should succeed: {:?}",
        result.err()
    );

    // Trait was consulted at least once (for the dispatch decision).
    assert!(
        resolve_calls.load(Ordering::SeqCst) >= 1,
        "MockResolver::resolve_by_name should be called at least once, got {}",
        resolve_calls.load(Ordering::SeqCst),
    );
    assert!(
        pt_calls.load(Ordering::SeqCst) >= 1,
        "MockProvider::provider_type should be called at least once, got {}",
        pt_calls.load(Ordering::SeqCst),
    );

    // The wire format that was actually used MUST match what the trait
    // claimed (anthropic → /v1/messages), NOT what the legacy string
    // compare would have picked (openai_compat → /v1/chat/completions).
    let path = captured_path.lock().await.clone();
    assert_eq!(
        path.as_deref(),
        Some("/v1/messages"),
        "trait-reported provider_type='anthropic' should select Anthropic wire format; instead the request went to {:?}",
        path,
    );

    // Sanity: we did receive at least the "hi" token from the mock SSE.
    assert!(
        tokens.iter().any(|t| t == "hi"),
        "expected token 'hi' from canned SSE, got tokens={:?}",
        tokens,
    );
}

// ── Test 2: swap the resolver between calls; dispatch changes ───────────

#[tokio::test]
async fn resolver_swap_mid_session_changes_provider() {
    let captured_path: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let base_url = start_dual_format_sse_mock(captured_path.clone()).await;
    let config = cfg_pointing_at(&base_url);

    // First call: resolver returns a MockProvider claiming "openai".
    let pt_calls_a = Arc::new(AtomicUsize::new(0));
    let mock_openai = Arc::new(MockProvider {
        type_id: "openai",
        provider_type_calls: pt_calls_a.clone(),
    });
    let resolver_a = Arc::new(MockResolver {
        provider: mock_openai as Arc<dyn LlmProvider>,
        resolve_calls: Arc::new(AtomicUsize::new(0)),
        last_name: Arc::new(Mutex::new(None)),
    });

    let _ = stream_agent_full_with_resolver(
        &config,
        "master",
        "ping-1",
        NO_HISTORY,
        None,
        None,
        resolver_a as Arc<dyn ResolveProvider>,
        |_| {},
    )
    .await
    .expect("first streaming call should succeed");

    let path_a = captured_path.lock().await.clone();
    assert_eq!(
        path_a.as_deref(),
        Some("/v1/chat/completions"),
        "with resolver_a (provider_type='openai'), expected /v1/chat/completions; got {:?}",
        path_a,
    );

    // Reset the captured path and swap to resolver_b returning "anthropic".
    *captured_path.lock().await = None;

    let pt_calls_b = Arc::new(AtomicUsize::new(0));
    let mock_anthropic = Arc::new(MockProvider {
        type_id: "anthropic",
        provider_type_calls: pt_calls_b.clone(),
    });
    let resolver_b = Arc::new(MockResolver {
        provider: mock_anthropic as Arc<dyn LlmProvider>,
        resolve_calls: Arc::new(AtomicUsize::new(0)),
        last_name: Arc::new(Mutex::new(None)),
    });

    let _ = stream_agent_full_with_resolver(
        &config,
        "master",
        "ping-2",
        NO_HISTORY,
        None,
        None,
        resolver_b as Arc<dyn ResolveProvider>,
        |_| {},
    )
    .await
    .expect("second streaming call should succeed");

    let path_b = captured_path.lock().await.clone();
    assert_eq!(
        path_b.as_deref(),
        Some("/v1/messages"),
        "after swap to resolver_b (provider_type='anthropic'), expected /v1/messages; got {:?}",
        path_b,
    );

    // Both resolvers were consulted in their respective sessions.
    assert!(pt_calls_a.load(Ordering::SeqCst) >= 1);
    assert!(pt_calls_b.load(Ordering::SeqCst) >= 1);

    // Silence dead-code on the JSON import used only for clarity in helpers above.
    let _ = json!({});
}
