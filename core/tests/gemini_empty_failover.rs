//! Regression test for the Gemini native function-calling empty-result
//! short-circuit (track `gemini-empty-failover`).
//!
//! Bug: the Gemini native short-circuits in `agent.rs`
//! (`call_with_streaming` ~1497 and `call_with_fallback` ~2092) called
//! `p.complete(...)` and on `Ok` ALWAYS returned `Ok((synthetic, model))`
//! — even when the Gemini 200 response carried NO candidates / a safety
//! block. `parse_gemini_response` yields `content == ""` + no `tool_calls`
//! for those, so `run_inner` aborted the WHOLE turn with "agent produced no
//! output and made no tool calls" instead of failing over to the next
//! configured provider.
//!
//! Fix mirrors the SSE path's guard: when the synthetic has empty content
//! AND no tool_calls, record the provider failure and `continue 'providers`
//! to the next chain entry.
//!
//! These tests inject (via `with_resolver`) a first gemini-typed provider
//! that returns the empty-candidates synthetic, with a second gemini-typed
//! provider that answers. Both go through the native short-circuit (no
//! HTTP), so the assertion is purely on the failover behaviour: the run
//! must end with the FALLBACK's answer, never abort.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use phantom_mesh::agent::AgentRuntime;
use phantom_mesh::config::{AgentEntry, AgentsConfig, ProviderEntry};
use phantom_mesh::providers::traits::{ChatMessage, ProviderError};
use phantom_mesh::providers::LlmProvider;
use phantom_mesh::streaming::ResolveProvider;

const PRIMARY: &str = "gemini_primary";
const FALLBACK: &str = "gemini_fallback";
const FALLBACK_ANSWER: &str = "fallback gemini answered";

/// A `gemini`-typed `LlmProvider` whose `complete` returns a fixed synthetic.
/// `provider_type() == "gemini"` so the agent loop routes it through the
/// native short-circuit (the code path under test), bypassing all HTTP.
struct FixedGeminiProvider {
    /// The exact `(ChatMessage, synthetic_json)` pair `complete` returns —
    /// shaped identically to `parse_gemini_response`'s output.
    result: (ChatMessage, serde_json::Value),
}

#[async_trait]
impl LlmProvider for FixedGeminiProvider {
    async fn stream(
        &self,
        _api_key: &str,
        _model: &str,
        _messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> Result<reqwest::Response, ProviderError> {
        // Never used: the gemini short-circuit calls `complete`, not `stream`.
        Err(ProviderError::Unknown("stream() unused in this test".into()))
    }

    async fn complete(
        &self,
        _api_key: &str,
        _model: &str,
        _messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> Result<(ChatMessage, serde_json::Value), ProviderError> {
        Ok(self.result.clone())
    }

    fn provider_type(&self) -> &'static str {
        "gemini"
    }
}

/// What `parse_gemini_response` produces for a Gemini 200 with no candidates
/// (or a safety block): `content == ""`, `tool_calls == null`.
fn empty_candidates_synthetic() -> (ChatMessage, serde_json::Value) {
    let synthetic = serde_json::json!({
        "choices": [{
            "message": { "role": "assistant", "content": "", "tool_calls": serde_json::Value::Null },
            "finish_reason": "SAFETY",
        }],
        "usage": { "prompt_tokens": 0, "completion_tokens": 0 },
        "model": "gemini-2.0-flash",
    });
    let chat = ChatMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: None,
    };
    (chat, synthetic)
}

/// A normal, non-empty Gemini answer (text content, no tool calls).
fn answering_synthetic() -> (ChatMessage, serde_json::Value) {
    let synthetic = serde_json::json!({
        "choices": [{
            "message": { "role": "assistant", "content": FALLBACK_ANSWER, "tool_calls": serde_json::Value::Null },
            "finish_reason": "stop",
        }],
        "usage": { "prompt_tokens": 3, "completion_tokens": 4 },
        "model": "gemini-2.0-flash",
    });
    let chat = ChatMessage {
        role: "assistant".into(),
        content: FALLBACK_ANSWER.into(),
        tool_calls: None,
    };
    (chat, synthetic)
}

/// Routes `PRIMARY` → empty provider, `FALLBACK` → answering provider.
struct FailoverResolver {
    primary: Arc<dyn LlmProvider>,
    fallback: Arc<dyn LlmProvider>,
}

impl ResolveProvider for FailoverResolver {
    fn resolve_by_name(&self, name: &str) -> Option<Arc<dyn LlmProvider>> {
        match name {
            PRIMARY => Some(self.primary.clone()),
            FALLBACK => Some(self.fallback.clone()),
            _ => None,
        }
    }
}

/// Config with two gemini providers and an agent whose priority list tries
/// `PRIMARY` then `FALLBACK`. Each provider has an inline api_key (so the key
/// gate passes) and a default_model (so the model gate passes).
fn cfg_two_gemini() -> AgentsConfig {
    let gemini_entry = || ProviderEntry {
        provider_type: "gemini".into(),
        api_key: Some("test-key".into()),
        default_model: Some("gemini-2.0-flash".into()),
        ..Default::default()
    };

    let mut providers = HashMap::new();
    providers.insert(PRIMARY.into(), gemini_entry());
    providers.insert(FALLBACK.into(), gemini_entry());

    let mut agent = HashMap::new();
    agent.insert(
        "master".into(),
        AgentEntry {
            provider: PRIMARY.into(),
            providers: Some(vec![PRIMARY.into(), FALLBACK.into()]),
            model: "gemini-2.0-flash".into(),
            tools: Vec::new(),
            instructions: "you are a test agent".into(),
        },
    );

    AgentsConfig {
        providers,
        agent,
        ..Default::default()
    }
}

fn make_resolver() -> Arc<FailoverResolver> {
    Arc::new(FailoverResolver {
        primary: Arc::new(FixedGeminiProvider {
            result: empty_candidates_synthetic(),
        }),
        fallback: Arc::new(FixedGeminiProvider {
            result: answering_synthetic(),
        }),
    })
}

// ── Non-streaming path (`call_with_fallback`, via `run`) ─────────────────────

#[tokio::test]
async fn gemini_empty_candidates_fails_over_non_streaming() {
    let runtime = AgentRuntime::new(cfg_two_gemini()).with_resolver(make_resolver());

    let result = runtime
        .run("master", "hello", &[], None)
        .await
        .expect("run must succeed by failing over to the fallback, not abort");

    assert_eq!(
        result.output, FALLBACK_ANSWER,
        "expected the fallback gemini provider's answer, got {:?}",
        result.output
    );
}

// ── Streaming path (`call_with_streaming`, via `run_with_callbacks`) ─────────

#[tokio::test]
async fn gemini_empty_candidates_fails_over_streaming() {
    let runtime = AgentRuntime::new(cfg_two_gemini()).with_resolver(make_resolver());
    let cost = phantom_mesh::cost::CostTracker::new();

    let result = runtime
        .run_with_callbacks("master", "hello", &[], None, &cost, |_ev| {})
        .await
        .expect("streaming run must fail over to the fallback, not abort");

    assert_eq!(
        result.output, FALLBACK_ANSWER,
        "expected the fallback gemini provider's answer, got {:?}",
        result.output
    );
}

// ── Negative control: BOTH gemini providers empty → genuine abort ────────────

#[tokio::test]
async fn both_gemini_empty_still_aborts() {
    // When NO provider produces output, the run must still error (the guard
    // must not mask a genuine all-empty failure as a fake success).
    let resolver = Arc::new(FailoverResolver {
        primary: Arc::new(FixedGeminiProvider {
            result: empty_candidates_synthetic(),
        }),
        fallback: Arc::new(FixedGeminiProvider {
            result: empty_candidates_synthetic(),
        }),
    });
    let runtime = AgentRuntime::new(cfg_two_gemini()).with_resolver(resolver);

    let outcome = runtime.run("master", "hello", &[], None).await;
    let err = match outcome {
        Ok(r) => panic!(
            "all-empty chain must surface an error, not a blank success (output={:?})",
            r.output
        ),
        Err(e) => e,
    };
    let msg = err.to_string();
    // The exhausted-chain error must carry the per-provider "empty response"
    // detail (NOT just the generic "All providers failed" header — the old
    // `contains("provider")` fallback could never fail), and it must list
    // BOTH chain entries — i.e. the guard really did `continue 'providers`
    // past the first empty result instead of aborting the loop.
    assert!(
        msg.contains("empty response"),
        "error should carry the per-provider empty-response detail, got: {msg}"
    );
    assert!(
        msg.contains(PRIMARY) && msg.contains(FALLBACK),
        "error should list both attempted providers (failover must advance past the first), got: {msg}"
    );
}
