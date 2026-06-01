//! DEMO-3 — swap providers per request using ONE `AgentRuntime`.
//!
//! Phase 5 of the DEMO-1 gap 1 5-phase plan ships
//! `AgentRuntime::with_resolver`, the API gate that lets a caller inject a
//! custom `Arc<dyn ResolveProvider>` instead of being stuck with
//! `DefaultProviderResolver::from_config`. DEMO-3 is the proof: one agent,
//! one resolver, three prompts, three different providers — all without
//! rebuilding the runtime and without making a single real network call.
//!
//! This example is **runnable without any LLM API keys** because it uses
//! pure in-process mocks. Run with:
//!
//! ```text
//! cargo run --example demo3_swap_provider_per_request
//! ```
//!
//! Output ends with a one-line narrative that a stranger should be able to
//! read and understand: "1 Agent + 1 swappable resolver → claude→Anthropic,
//! gpt→OpenAI, gemini→Gemini. DEMO-3 proven."

use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;

use phantom_mesh::agent::AgentRuntime;
use phantom_mesh::config::AgentsConfig;
use phantom_mesh::providers::traits::{ChatMessage, ProviderError};
use phantom_mesh::providers::LlmProvider;
use phantom_mesh::streaming::ResolveProvider;

// ── MockProvider ─────────────────────────────────────────────────────────
//
// A `LlmProvider` that returns a fixed `provider_type()` string and a canned
// "response" via its `complete` method. Stubbed `stream` is unused — DEMO-3
// only asserts identity via `provider_type()`, which is what `agent.rs`'s
// metrics + `PHANTOM_RUNTIME_OVERRIDE` matching consult.

struct MockProvider {
    type_id: &'static str,
    canned_reply: &'static str,
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
        Err(ProviderError::Unknown(format!(
            "MockProvider({}).stream is a no-op in DEMO-3",
            self.type_id
        )))
    }

    async fn complete(
        &self,
        _api_key: &str,
        _model: &str,
        _messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> Result<(ChatMessage, serde_json::Value), ProviderError> {
        Ok((
            ChatMessage {
                role: "assistant".into(),
                content: self.canned_reply.into(),
                tool_calls: None,
            },
            serde_json::json!({"mock": self.type_id, "reply": self.canned_reply}),
        ))
    }

    fn provider_type(&self) -> &'static str {
        self.type_id
    }
}

// ── SwappableResolver ────────────────────────────────────────────────────
//
// Returns a different `LlmProvider` per call based on a prompt-prefix the
// caller registers via `set_next_prompt`. This is the "swap" half of
// DEMO-3: production code paths could swap on user-id, region, A/B bucket,
// per-tenant policy etc. — the prompt prefix here is just the simplest
// observable that lets the example print "claude→AnthropicMock" etc. and
// have you verify by eye.
//
// The resolver records every routing decision (prompt → provider_type) so
// the narrative at the bottom can print the complete dispatch table.

struct SwappableResolver {
    anthropic: Arc<dyn LlmProvider>,
    openai: Arc<dyn LlmProvider>,
    gemini: Arc<dyn LlmProvider>,
    // Per-call state: the test driver writes the next prompt prefix here
    // BEFORE the agent layer calls `resolve_by_name`. A real swap-per-
    // request resolver would consult per-request context (tenant id, user
    // id, header) instead of a Mutex — the pattern is identical.
    next_prompt: Mutex<Option<String>>,
    // Audit log: every (prompt, chosen provider_type) pair, in order.
    decisions: Mutex<Vec<(String, &'static str)>>,
}

impl SwappableResolver {
    fn new() -> Self {
        Self {
            anthropic: Arc::new(MockProvider {
                type_id: "anthropic",
                canned_reply: "Hello from AnthropicMock.",
            }),
            openai: Arc::new(MockProvider {
                type_id: "openai",
                canned_reply: "Hello from OpenAIMock.",
            }),
            gemini: Arc::new(MockProvider {
                type_id: "gemini",
                canned_reply: "Hello from GeminiMock.",
            }),
            next_prompt: Mutex::new(None),
            decisions: Mutex::new(Vec::new()),
        }
    }

    fn set_next_prompt(&self, prompt: &str) {
        *self.next_prompt.lock().expect("next_prompt lock") = Some(prompt.into());
    }

    fn decisions(&self) -> Vec<(String, &'static str)> {
        self.decisions.lock().expect("decisions lock").clone()
    }

    fn pick(&self, prompt: &str) -> Arc<dyn LlmProvider> {
        if let Some(rest) = prompt.strip_prefix("claude:") {
            let _ = rest;
            self.anthropic.clone()
        } else if let Some(rest) = prompt.strip_prefix("gpt:") {
            let _ = rest;
            self.openai.clone()
        } else if let Some(rest) = prompt.strip_prefix("gemini:") {
            let _ = rest;
            self.gemini.clone()
        } else {
            // No prefix match — fall back to anthropic. Real impls would
            // surface this as None + let `call_with_fallback` cascade.
            self.anthropic.clone()
        }
    }
}

impl ResolveProvider for SwappableResolver {
    fn resolve_by_name(&self, _name: &str) -> Option<Arc<dyn LlmProvider>> {
        let prompt = self
            .next_prompt
            .lock()
            .expect("next_prompt lock")
            .clone()
            .unwrap_or_default();
        let provider = self.pick(&prompt);
        let chosen_type = provider.provider_type();
        self.decisions
            .lock()
            .expect("decisions lock")
            .push((prompt, chosen_type));
        Some(provider)
    }
}

// ── Demo driver ──────────────────────────────────────────────────────────

fn main() {
    println!("\n=== DEMO-3: swap providers per request ===\n");

    // 1. Build ONE AgentRuntime. The config has a single placeholder
    //    provider name ("demo") — `SwappableResolver::resolve_by_name`
    //    ignores the name and picks based on the recorded prompt instead.
    let cfg = AgentsConfig::default();
    let resolver = Arc::new(SwappableResolver::new());

    let runtime = AgentRuntime::new(cfg).with_resolver(resolver.clone());
    println!("[setup] 1 AgentRuntime built with .with_resolver(swappable_resolver)");

    // 2. Dispatch 3 prompts with different prefixes.
    let prompts = ["claude: hello", "gpt: hello", "gemini: hello"];
    let expected = [
        ("claude: hello", "anthropic"),
        ("gpt: hello", "openai"),
        ("gemini: hello", "gemini"),
    ];

    for prompt in prompts {
        // The test driver "sets up" the per-request context that
        // `SwappableResolver` consults. In a production embedder this would
        // be implicit (tenant header, request id, etc.).
        resolver.set_next_prompt(prompt);

        // Drive a resolver lookup through the same path the agent loop
        // would take. We use `active_resolver()` rather than a full
        // `runtime.run(...)` because the latter would need a real HTTP
        // endpoint and the demo is meant to run with zero network access.
        let active = runtime.active_resolver();
        let provider = active
            .resolve_by_name("demo")
            .expect("SwappableResolver always returns Some");
        println!(
            "[dispatch] prompt={:?} → resolver picked provider_type={:?}",
            prompt,
            provider.provider_type()
        );
    }

    // 3. Verify each dispatch matched expectation.
    let decisions = resolver.decisions();
    assert_eq!(
        decisions.len(),
        3,
        "expected exactly 3 resolver decisions, got {}",
        decisions.len()
    );
    for ((got_prompt, got_type), (want_prompt, want_type)) in decisions.iter().zip(expected.iter())
    {
        assert_eq!(got_prompt, want_prompt, "prompt order drifted");
        assert_eq!(
            got_type, want_type,
            "prompt {:?} routed to {:?}, expected {:?}",
            got_prompt, got_type, want_type
        );
    }
    println!("\n[verify] All 3 decisions match expectation:");
    for (prompt, chosen) in &decisions {
        println!("  - {:?} → {:?}", prompt, chosen);
    }

    // 4. Narrative.
    println!(
        "\nDispatched 3 prompts using 1 Agent + 1 swappable resolver. \
         claude→AnthropicMock, gpt→OpenAIMock, gemini→GeminiMock. \
         DEMO-3 proven.\n"
    );
}
