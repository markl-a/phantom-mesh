//! bench_anthropic_cache_hit_vs_miss
//!
//! Goal: produce a baseline timing for the F1 (PR #31) prompt-cache codepath
//! by running `stream_agent_full` against a wiremock that returns canned
//! Anthropic SSE frames. Two scenarios:
//!
//!   * miss: usage block has `cache_creation_input_tokens > 0`, `cache_read = 0`
//!   * hit : usage block has `cache_read_input_tokens > 0`, `cache_creation = 0`
//!
//! What this proves:
//!   * the cache-token parsing path (`streaming::process_anthropic_event`)
//!     does not regress in wall-time across releases
//!   * `StreamResult.cache_read_input_tokens` is populated > 0 on the
//!     hit-scenario response (asserted at setup time, not by Criterion)
//!
//! What this does NOT prove:
//!   * any savings from the *real* Anthropic API — only Anthropic-side
//!     telemetry can confirm that. See docs/perf/2026-05-15-baselines.md
//!     for the suggested manual capture procedure for real-API numbers.

use std::sync::OnceLock;

use axum::{routing::post, Router};
use criterion::{criterion_group, criterion_main, Criterion};
use tokio::net::TcpListener;
use tokio::runtime::Runtime;

use spectyn_mesh::{
    config::{AgentEntry, AgentsConfig, ProviderEntry},
    providers::traits::ChatMessage,
    streaming::{stream_agent_full, StreamEvent, StreamResult},
};

#[path = "common/mod.rs"]
mod common;

// Anthropic SSE bodies that simulate a cache MISS and a cache HIT. The
// message_delta frame's `usage` block is where Anthropic reports the
// cache_* counts — `process_anthropic_event` reads from there.
const SSE_MISS: &str = "\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":100,\"output_tokens\":0,\"cache_creation_input_tokens\":100,\"cache_read_input_tokens\":0}}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":1,\"cache_creation_input_tokens\":100,\"cache_read_input_tokens\":0}}\n\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\n";

const SSE_HIT: &str = "\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"m2\",\"usage\":{\"input_tokens\":10,\"output_tokens\":0,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":100}}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":1,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":100}}\n\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\n";

/// Spin up a one-route mock Anthropic SSE server. Returns a base URL like
/// `http://127.0.0.1:<port>`. The route is `/v1/messages` (Anthropic native).
async fn start_anthropic_mock(sse_body: &'static str) -> String {
    use axum::http::StatusCode;
    let app = Router::new().route(
        "/v1/messages",
        post(move || async move {
            axum::response::Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/event-stream")
                .body(axum::body::Body::from(sse_body))
                .expect("build mock SSE response")
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind mock");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock");
    });
    format!("http://127.0.0.1:{}", addr.port())
}

/// Build an Anthropic-typed AgentsConfig pointing at `base_url`. Provider
/// type MUST be `"anthropic"` because that flag is what activates the
/// cache-control breakpoint logic in `build_request_body`.
fn anthropic_config(base_url: &str) -> AgentsConfig {
    let mut providers = std::collections::HashMap::new();
    providers.insert(
        "mock-anthropic".to_string(),
        ProviderEntry {
            provider_type: "anthropic".to_string(),
            url: Some(format!("{}/v1/messages", base_url)),
            api_key: Some("test-key".to_string()),
            api_key_env: None,
            default_model: None,
            tier: None,
        },
    );
    let mut agent = std::collections::HashMap::new();
    agent.insert(
        "master".to_string(),
        AgentEntry {
            provider: "mock-anthropic".to_string(),
            providers: None,
            model: "claude-sonnet-4-5-20251022".to_string(),
            tools: vec![],
            instructions: "you are a test agent".to_string(),
        },
    );
    AgentsConfig {
        providers,
        agent,
        ..Default::default()
    }
}

/// Cache the mock servers across iterations — spinning up axum per-iter
/// would dominate the timing.
struct MockServers {
    miss_url: String,
    hit_url: String,
}

fn mock_servers(rt: &Runtime) -> &'static MockServers {
    static CELL: OnceLock<MockServers> = OnceLock::new();
    CELL.get_or_init(|| {
        let miss_url = rt.block_on(start_anthropic_mock(SSE_MISS));
        let hit_url = rt.block_on(start_anthropic_mock(SSE_HIT));
        MockServers { miss_url, hit_url }
    })
}

/// Run one full streaming round against `base_url` and return the result.
async fn run_round(base_url: &str) -> StreamResult {
    let cfg = anthropic_config(base_url);
    stream_agent_full(
        &cfg,
        "master",
        "hello",
        &[] as &[ChatMessage],
        None,
        None,
        |_e: StreamEvent| {},
    )
    .await
    .expect("stream_agent_full mock should succeed")
}

fn bench_anthropic_cache_hit_vs_miss(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let servers = mock_servers(&rt);

    // ── Setup-time assertions (NOT in the timed loop) ────────────────
    // Confirm the cache-miss SSE actually surfaces cache_creation_input_tokens
    // and the cache-hit SSE actually surfaces cache_read_input_tokens.
    let miss = rt.block_on(run_round(&servers.miss_url));
    assert!(
        miss.cache_creation_input_tokens > 0,
        "MISS scenario must produce cache_creation_input_tokens > 0, got {miss:?}"
    );
    assert_eq!(
        miss.cache_read_input_tokens, 0,
        "MISS must have read=0, got {miss:?}"
    );

    let hit = rt.block_on(run_round(&servers.hit_url));
    assert!(
        hit.cache_read_input_tokens > 0,
        "HIT scenario must produce cache_read_input_tokens > 0, got {hit:?}"
    );
    assert_eq!(
        hit.cache_creation_input_tokens, 0,
        "HIT must have creation=0, got {hit:?}"
    );

    // ── Timed benches ────────────────────────────────────────────────
    let mut g = c.benchmark_group("anthropic_cache_hit_vs_miss");
    g.sample_size(30);

    g.bench_function("miss", |b| {
        b.to_async(&rt).iter(|| run_round(&servers.miss_url));
    });
    g.bench_function("hit", |b| {
        b.to_async(&rt).iter(|| run_round(&servers.hit_url));
    });
    g.finish();
}

criterion_group! {
    name = anthropic;
    config = common::standard_criterion();
    targets = bench_anthropic_cache_hit_vs_miss
}
criterion_main!(anthropic);
