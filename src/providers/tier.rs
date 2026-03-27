//! Four-Tier Provider Routing — prioritizes providers by cost tier and local LLM speed.
//!
//! # Tiers (ascending cost)
//!
//! 1. **Local** — Ollama, llama.cpp, etc. (zero cost, variable latency)
//! 2. **FreeApi** — Free-tier cloud APIs (Gemini, Groq free plans)
//! 3. **Subscription** — Fixed-cost plans (ChatGPT Plus, Claude Pro)
//! 4. **PayAsYouGo** — Per-token billing (OpenAI API, Anthropic API)
//!
//! # Routing modes (based on local LLM latency)
//!
//! - **Fast** (<500ms): Local → Free → Subscription → PayAsYouGo
//! - **Medium** (500ms–3s): Free ↔ Local → Subscription → PayAsYouGo
//! - **Slow** (>3s): Free → Local → Subscription → PayAsYouGo

use serde::{Deserialize, Serialize};
use tracing::debug;

// ---------------------------------------------------------------------------
// LocalSpeed
// ---------------------------------------------------------------------------

/// Classification of local LLM inference speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalSpeed {
    /// < 500ms response time
    Fast,
    /// 500ms – 3000ms
    Medium,
    /// > 3000ms
    Slow,
    /// No local LLM available or not yet probed
    Unknown,
}

impl LocalSpeed {
    /// Classify a latency measurement (in milliseconds) into a speed category.
    pub fn from_latency_ms(ms: u64) -> Self {
        match ms {
            0..=499 => LocalSpeed::Fast,
            500..=3000 => LocalSpeed::Medium,
            _ => LocalSpeed::Slow,
        }
    }
}

// ---------------------------------------------------------------------------
// ProviderTier
// ---------------------------------------------------------------------------

/// Cost tier for a provider — lower number = cheaper / preferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ProviderTier {
    /// Local inference (Ollama, llama.cpp) — zero cost
    #[serde(rename = "local")]
    Local = 1,
    /// Free-tier cloud APIs (Gemini free, Groq free)
    #[serde(rename = "free", alias = "FreeApi")]
    FreeApi = 2,
    /// Subscription plans (ChatGPT Plus, Claude Pro)
    #[serde(rename = "subscription")]
    Subscription = 3,
    /// Pay-as-you-go API billing
    #[serde(rename = "payg", alias = "PayAsYouGo")]
    PayAsYouGo = 4,
}

// ---------------------------------------------------------------------------
// TieredProvider
// ---------------------------------------------------------------------------

fn default_available() -> bool {
    true
}

/// A provider registered with the tier router.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TieredProvider {
    /// Provider name (matches ProviderRouter key, e.g. "ollama", "gemini")
    pub provider_name: String,
    /// Which cost tier this provider belongs to
    pub tier: ProviderTier,
    /// Priority within the same tier (lower = preferred). Allows user customization.
    pub priority_score: u32,
    /// Whether this provider is currently available (circuit breaker not tripped)
    #[serde(skip, default = "default_available")]
    pub available: bool,
}

// ---------------------------------------------------------------------------
// TierRouter
// ---------------------------------------------------------------------------

/// Routes requests to providers based on cost tier and local LLM speed.
pub struct TierRouter {
    providers: Vec<TieredProvider>,
}

impl TierRouter {
    /// Create a new tier router with the given providers.
    pub fn new(mut providers: Vec<TieredProvider>) -> Self {
        // Sort by tier (ascending cost), then by priority_score within tier
        providers.sort_by(|a, b| {
            a.tier.cmp(&b.tier).then(a.priority_score.cmp(&b.priority_score))
        });
        Self { providers }
    }

    /// Register a new provider. Re-sorts the list.
    pub fn register(&mut self, provider: TieredProvider) {
        self.providers.push(provider);
        self.providers.sort_by(|a, b| {
            a.tier.cmp(&b.tier).then(a.priority_score.cmp(&b.priority_score))
        });
    }

    /// Mark a provider as unavailable (e.g. circuit breaker tripped).
    pub fn set_available(&mut self, provider_name: &str, available: bool) {
        for p in &mut self.providers {
            if p.provider_name == provider_name {
                p.available = available;
            }
        }
    }

    /// Mark all providers with the given names as unavailable.
    pub fn set_tripped(&mut self, tripped_names: &[String]) {
        let tripped_set: std::collections::HashSet<&String> = tripped_names.iter().collect();
        for p in &mut self.providers {
            if tripped_set.contains(&p.provider_name) {
                p.available = false;
            }
        }
    }

    /// Get the ordered list of provider names based on local speed.
    /// Skips providers marked as unavailable.
    pub fn best_providers(&self, local_speed: LocalSpeed) -> Vec<String> {
        let tier_order = Self::tier_order(local_speed);

        let mut result = Vec::new();
        for tier in &tier_order {
            let mut tier_providers: Vec<&TieredProvider> = self
                .providers
                .iter()
                .filter(|p| p.tier == *tier && p.available)
                .collect();
            tier_providers.sort_by_key(|p| p.priority_score);
            for p in tier_providers {
                result.push(p.provider_name.clone());
            }
        }

        debug!(
            "TierRouter: local_speed={:?}, order={:?}",
            local_speed, result
        );
        result
    }

    /// Determine tier ordering based on local LLM speed.
    fn tier_order(local_speed: LocalSpeed) -> Vec<ProviderTier> {
        match local_speed {
            LocalSpeed::Fast => vec![
                ProviderTier::Local,
                ProviderTier::FreeApi,
                ProviderTier::Subscription,
                ProviderTier::PayAsYouGo,
            ],
            LocalSpeed::Medium => vec![
                ProviderTier::FreeApi,
                ProviderTier::Local,
                ProviderTier::Subscription,
                ProviderTier::PayAsYouGo,
            ],
            LocalSpeed::Slow | LocalSpeed::Unknown => vec![
                ProviderTier::FreeApi,
                ProviderTier::Local,
                ProviderTier::Subscription,
                ProviderTier::PayAsYouGo,
            ],
        }
    }

    /// Get all registered providers (for diagnostics).
    pub fn all_providers(&self) -> &[TieredProvider] {
        &self.providers
    }

    /// Get providers in a specific tier.
    pub fn providers_in_tier(&self, tier: ProviderTier) -> Vec<&TieredProvider> {
        self.providers.iter().filter(|p| p.tier == tier).collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_router() -> TierRouter {
        TierRouter::new(vec![
            TieredProvider {
                provider_name: "ollama".into(),
                tier: ProviderTier::Local,
                priority_score: 0,
                available: true,
            },
            TieredProvider {
                provider_name: "gemini-free".into(),
                tier: ProviderTier::FreeApi,
                priority_score: 0,
                available: true,
            },
            TieredProvider {
                provider_name: "groq-free".into(),
                tier: ProviderTier::FreeApi,
                priority_score: 1,
                available: true,
            },
            TieredProvider {
                provider_name: "chatgpt-plus".into(),
                tier: ProviderTier::Subscription,
                priority_score: 0,
                available: true,
            },
            TieredProvider {
                provider_name: "openai-api".into(),
                tier: ProviderTier::PayAsYouGo,
                priority_score: 0,
                available: true,
            },
            TieredProvider {
                provider_name: "anthropic-api".into(),
                tier: ProviderTier::PayAsYouGo,
                priority_score: 1,
                available: true,
            },
        ])
    }

    #[test]
    fn fast_local_prefers_local_first() {
        let router = make_router();
        let order = router.best_providers(LocalSpeed::Fast);
        assert_eq!(order[0], "ollama");
        assert_eq!(order[1], "gemini-free");
        assert_eq!(order[2], "groq-free");
        assert_eq!(order[3], "chatgpt-plus");
        assert_eq!(order[4], "openai-api");
        assert_eq!(order[5], "anthropic-api");
    }

    #[test]
    fn medium_local_prefers_free_first() {
        let router = make_router();
        let order = router.best_providers(LocalSpeed::Medium);
        assert_eq!(order[0], "gemini-free");
        assert_eq!(order[1], "groq-free");
        assert_eq!(order[2], "ollama");
        assert_eq!(order[3], "chatgpt-plus");
    }

    #[test]
    fn slow_local_prefers_free_first() {
        let router = make_router();
        let order = router.best_providers(LocalSpeed::Slow);
        assert_eq!(order[0], "gemini-free");
        assert_eq!(order[1], "groq-free");
        assert_eq!(order[2], "ollama");
    }

    #[test]
    fn unknown_speed_same_as_slow() {
        let router = make_router();
        let slow = router.best_providers(LocalSpeed::Slow);
        let unknown = router.best_providers(LocalSpeed::Unknown);
        assert_eq!(slow, unknown);
    }

    #[test]
    fn circuit_breaker_skips_tripped() {
        let mut router = make_router();
        router.set_available("gemini-free", false);
        router.set_available("ollama", false);

        let order = router.best_providers(LocalSpeed::Fast);
        assert!(!order.contains(&"ollama".to_string()));
        assert!(!order.contains(&"gemini-free".to_string()));
        assert_eq!(order[0], "groq-free");
        assert_eq!(order[1], "chatgpt-plus");
    }

    #[test]
    fn set_tripped_batch() {
        let mut router = make_router();
        router.set_tripped(&["ollama".into(), "openai-api".into()]);

        let order = router.best_providers(LocalSpeed::Fast);
        assert!(!order.contains(&"ollama".to_string()));
        assert!(!order.contains(&"openai-api".to_string()));
        assert!(order.contains(&"gemini-free".to_string()));
    }

    #[test]
    fn empty_router_returns_empty() {
        let router = TierRouter::new(vec![]);
        let order = router.best_providers(LocalSpeed::Fast);
        assert!(order.is_empty());
    }

    #[test]
    fn all_tripped_returns_empty() {
        let mut router = make_router();
        for p in router.providers.iter_mut() {
            p.available = false;
        }
        let order = router.best_providers(LocalSpeed::Fast);
        assert!(order.is_empty());
    }

    #[test]
    fn priority_within_tier() {
        let router = TierRouter::new(vec![
            TieredProvider {
                provider_name: "groq-free".into(),
                tier: ProviderTier::FreeApi,
                priority_score: 10,
                available: true,
            },
            TieredProvider {
                provider_name: "gemini-free".into(),
                tier: ProviderTier::FreeApi,
                priority_score: 1,
                available: true,
            },
        ]);
        let order = router.best_providers(LocalSpeed::Slow);
        assert_eq!(order[0], "gemini-free"); // lower priority_score = preferred
        assert_eq!(order[1], "groq-free");
    }

    #[test]
    fn register_adds_and_sorts() {
        let mut router = TierRouter::new(vec![TieredProvider {
            provider_name: "ollama".into(),
            tier: ProviderTier::Local,
            priority_score: 0,
            available: true,
        }]);

        router.register(TieredProvider {
            provider_name: "gemini".into(),
            tier: ProviderTier::FreeApi,
            priority_score: 0,
            available: true,
        });

        assert_eq!(router.all_providers().len(), 2);
        let order = router.best_providers(LocalSpeed::Fast);
        assert_eq!(order[0], "ollama");
        assert_eq!(order[1], "gemini");
    }

    #[test]
    fn providers_in_tier_filters_correctly() {
        let router = make_router();
        let free = router.providers_in_tier(ProviderTier::FreeApi);
        assert_eq!(free.len(), 2);
        assert!(free.iter().all(|p| p.tier == ProviderTier::FreeApi));

        let payg = router.providers_in_tier(ProviderTier::PayAsYouGo);
        assert_eq!(payg.len(), 2);
    }

    #[test]
    fn local_speed_classification() {
        assert_eq!(LocalSpeed::from_latency_ms(0), LocalSpeed::Fast);
        assert_eq!(LocalSpeed::from_latency_ms(499), LocalSpeed::Fast);
        assert_eq!(LocalSpeed::from_latency_ms(500), LocalSpeed::Medium);
        assert_eq!(LocalSpeed::from_latency_ms(3000), LocalSpeed::Medium);
        assert_eq!(LocalSpeed::from_latency_ms(3001), LocalSpeed::Slow);
        assert_eq!(LocalSpeed::from_latency_ms(10000), LocalSpeed::Slow);
    }
}
