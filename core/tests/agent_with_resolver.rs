//! DEMO-1 gap 1 Phase 5 — integration tests for
//! `AgentRuntime::with_resolver` (the injection-point builder) and the
//! mini DEMO-3 swap-per-request flow.
//!
//! Coverage:
//!   1. `with_resolver_overrides_default` — installing a `MockResolver` via
//!      `with_resolver` makes `active_resolver().resolve_by_name(...)`
//!      return the mock-supplied provider instead of the default
//!      string-switch passthrough.
//!   2. `without_with_resolver_uses_default` — regression: when no
//!      override is installed, `active_resolver()` falls through to a
//!      fresh `DefaultProviderResolver::from_config(&self.config)`
//!      identical to Phase 4. The dispatch is byte-for-byte unchanged.
//!   3. `swappable_resolver_routes_by_prompt_prefix` — mini DEMO-3 in
//!      test form: one runtime, one resolver, three prompts, three
//!      different mock providers picked. Belt-and-braces alongside the
//!      runnable `examples/demo3_swap_provider_per_request.rs`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use phantom_mesh::agent::AgentRuntime;
use phantom_mesh::config::{AgentsConfig, ProviderEntry};
use phantom_mesh::providers::traits::{ChatMessage, ProviderError};
use phantom_mesh::providers::LlmProvider;
use phantom_mesh::streaming::ResolveProvider;

// ── Test fixtures ────────────────────────────────────────────────────────

/// Minimal `LlmProvider` that only matters for its `provider_type()`. The
/// `stream` / `complete` methods are unused — Phase 5's injection point is
/// observed via the trait identity returned from the resolver.
struct StaticProvider {
    type_id: &'static str,
}

#[async_trait]
impl LlmProvider for StaticProvider {
    async fn stream(
        &self,
        _api_key: &str,
        _model: &str,
        _messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> Result<reqwest::Response, ProviderError> {
        Err(ProviderError::Unknown("StaticProvider stub".into()))
    }
    async fn complete(
        &self,
        _api_key: &str,
        _model: &str,
        _messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> Result<(ChatMessage, serde_json::Value), ProviderError> {
        Err(ProviderError::Unknown("StaticProvider stub".into()))
    }
    fn provider_type(&self) -> &'static str {
        self.type_id
    }
}

/// A `ResolveProvider` that always hands back the same provider and counts
/// every `resolve_by_name` call.
struct CountingResolver {
    provider: Arc<dyn LlmProvider>,
    calls: Arc<AtomicUsize>,
}

impl ResolveProvider for CountingResolver {
    fn resolve_by_name(&self, _name: &str) -> Option<Arc<dyn LlmProvider>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Some(self.provider.clone())
    }
}

/// Build an `AgentsConfig` with a single openai provider entry so the
/// default resolver path returns `provider_type() == "openai"`.
fn cfg_with_openai() -> AgentsConfig {
    let mut providers = HashMap::new();
    providers.insert(
        "primary".into(),
        ProviderEntry {
            provider_type: "openai".into(),
            ..Default::default()
        },
    );
    AgentsConfig {
        providers,
        ..Default::default()
    }
}

// ── Test 1: with_resolver overrides default ──────────────────────────────

#[test]
fn with_resolver_overrides_default() {
    // Default config would resolve `primary` to an openai provider. The
    // override resolves it to a custom "demo-mock" provider instead. The
    // assertion is on the trait identity returned — proves the override
    // is what `agent.rs` will consult, not the default string-switch.
    let cfg = cfg_with_openai();
    let calls = Arc::new(AtomicUsize::new(0));
    let resolver = Arc::new(CountingResolver {
        provider: Arc::new(StaticProvider {
            type_id: "demo-mock",
        }),
        calls: calls.clone(),
    });

    let runtime = AgentRuntime::new(cfg).with_resolver(resolver.clone());
    let active = runtime.active_resolver();
    let chosen = active
        .resolve_by_name("primary")
        .expect("override resolver returns Some");

    assert_eq!(chosen.provider_type(), "demo-mock");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "override resolver must be hit"
    );
}

// ── Test 2: without with_resolver, default behaviour preserved ───────────

#[test]
fn without_with_resolver_uses_default() {
    // No `.with_resolver(...)` call → `active_resolver()` must build a
    // fresh `DefaultProviderResolver` and resolve `primary` to the
    // openai-typed provider exactly as Phase 4 did. Regression canary.
    let cfg = cfg_with_openai();
    let runtime = AgentRuntime::new(cfg);
    let active = runtime.active_resolver();
    let chosen = active
        .resolve_by_name("primary")
        .expect("default resolver returns Some for configured provider");

    assert_eq!(chosen.provider_type(), "openai");
}

// ── Test 3: mini DEMO-3 — swappable resolver routes by prompt prefix ─────

/// Mirrors the structure of `examples/demo3_swap_provider_per_request.rs`
/// but as a test: one runtime, one resolver, three prompts, three different
/// providers — verified via the trait identity each time.
struct PromptRoutedResolver {
    anthropic: Arc<dyn LlmProvider>,
    openai: Arc<dyn LlmProvider>,
    gemini: Arc<dyn LlmProvider>,
    next_prompt: Mutex<Option<String>>,
}

impl ResolveProvider for PromptRoutedResolver {
    fn resolve_by_name(&self, _name: &str) -> Option<Arc<dyn LlmProvider>> {
        let prompt = self
            .next_prompt
            .lock()
            .expect("next_prompt lock")
            .clone()
            .unwrap_or_default();
        let picked: Arc<dyn LlmProvider> = if prompt.starts_with("claude:") {
            self.anthropic.clone()
        } else if prompt.starts_with("gpt:") {
            self.openai.clone()
        } else if prompt.starts_with("gemini:") {
            self.gemini.clone()
        } else {
            // Fall back to anthropic — same default the example uses.
            self.anthropic.clone()
        };
        Some(picked)
    }
}

#[test]
fn swappable_resolver_routes_by_prompt_prefix() {
    let resolver = Arc::new(PromptRoutedResolver {
        anthropic: Arc::new(StaticProvider {
            type_id: "anthropic",
        }),
        openai: Arc::new(StaticProvider { type_id: "openai" }),
        gemini: Arc::new(StaticProvider { type_id: "gemini" }),
        next_prompt: Mutex::new(None),
    });

    let runtime = AgentRuntime::new(AgentsConfig::default()).with_resolver(resolver.clone());

    let cases = [
        ("claude: hello", "anthropic"),
        ("gpt: hello", "openai"),
        ("gemini: hello", "gemini"),
    ];

    for (prompt, expected_type) in cases {
        *resolver.next_prompt.lock().expect("lock") = Some(prompt.into());
        let active = runtime.active_resolver();
        let provider = active.resolve_by_name("any-config-name").expect("Some");
        assert_eq!(
            provider.provider_type(),
            expected_type,
            "prompt {:?} should route to {:?}, got {:?}",
            prompt,
            expected_type,
            provider.provider_type()
        );
    }
}
