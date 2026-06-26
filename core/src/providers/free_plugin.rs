//! Free-tier LLM provider plugin — default-on in onboarding.
//!
//! A curated registry of genuinely-free, **no-credit-card**, OpenAI-compatible
//! cloud LLM tiers. This is the "free API plugin": when a brand-new user has no
//! subscription CLI (claude / codex / gemini) and no local server (Ollama),
//! onboarding falls back to a free cloud provider listed here so the partner can
//! answer minute-one — a paid key is never required.
//!
//! ## Honest contract (no false-green)
//!
//! These tiers still need a *free* API key (no credit card, ~1-minute signup).
//! The plugin makes that as close to zero-config as is honestly possible:
//!
//! 1. **Auto-detect** a key already in the environment → zero further input.
//!    The detection env-var names match exactly what the live chat path reads
//!    (`agent.rs` → `provider.api_key_env`, and the wire layer's
//!    `PHANTOM_MESH_<UPPER>_API_KEY` override).
//! 2. **Otherwise guide** a 1-tap "get a free key" step, defaulting to the
//!    recommended provider (Groq — the proven free workhorse on this project's
//!    headless dev fleet: fast, no credit card).
//!
//! **No secret is ever written by the config writer** — only the *name* of the
//! env var (`api_key_env`), matching the no-keys-in-file design the rest of
//! onboarding uses. A key the user explicitly pastes is persisted via the
//! dedicated, properly-escaped `keys::set_api_key` path, never string-built here.
//!
//! 中文: 免費 LLM 供應商外掛 — onboarding 預設啟動。精選一組「真免費、免信用卡、
//! OpenAI 相容」的雲端 tier。新手沒訂閱、沒本機 Ollama 時，預設落到這裡的免費雲端
//! 供應商,讓夥伴第一分鐘就能回話、永不需付費金鑰。誠實合約:仍需免費金鑰,但①已在
//! 環境變數的金鑰自動偵測=零設定;②否則引導 1 分鐘取得(預設 Groq)。設定檔只寫
//! 環境變數「名稱」(api_key_env)不寫秘密;使用者貼上的金鑰走 keys::set_api_key。

/// One free-tier cloud provider in the curated registry. Every field is a
/// `&'static str` so the whole registry is a compile-time constant — zero
/// allocation, zero new dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreeProvider {
    /// agents.toml block name + stable slug (e.g. `"groq"`). Doubles as the
    /// `PHANTOM_MESH_<UPPER>_API_KEY` stem for the wire-layer env override.
    pub slug: &'static str,
    /// Human label for the picker (e.g. `"Groq"`).
    pub display: &'static str,
    /// agents.toml `type` — MUST be a type `resolver.rs::build_provider`
    /// handles (it routes anything non-special through the OpenAI-compat
    /// client using the explicit `url`, and special-cases `gemini`).
    pub provider_type: &'static str,
    /// OpenAI-compatible base URL written verbatim as the block `url` so
    /// routing is deterministic (never relies on a type→url default guess).
    pub base_url: &'static str,
    /// The env var the live chat path sources the key from — written as the
    /// block's `api_key_env` line (the NAME, never the value).
    pub api_key_env: &'static str,
    /// A resolvable default model on the free tier (the runtime model resolver
    /// needs a `default_model` or the provider is skipped — see
    /// `write_onboarding_config`).
    pub default_model: &'static str,
    /// Where a user gets a free key in ~1 minute (surfaced in the guided step).
    pub get_key_url: &'static str,
    /// True iff signing up for the free tier needs NO credit card.
    pub no_credit_card: bool,
}

/// The curated registry, **best-first**. Groq leads: the proven free workhorse
/// on this project (the headless dev fleet ran on Groq), fastest no-credit-card
/// tier, fully wired end-to-end (keys.rs meta + probe + resolver). The runtime
/// (`resolver.rs::build_provider`) routes groq / cerebras / openrouter through
/// the OpenAI-compat client and special-cases gemini — all four are safe.
pub const FREE_PROVIDERS: &[FreeProvider] = &[
    FreeProvider {
        slug: "groq",
        display: "Groq",
        provider_type: "groq",
        base_url: "https://api.groq.com/openai/v1",
        api_key_env: "GROQ_API_KEY",
        default_model: "llama-3.3-70b-versatile",
        get_key_url: "https://console.groq.com/keys",
        no_credit_card: true,
    },
    FreeProvider {
        slug: "cerebras",
        display: "Cerebras",
        provider_type: "cerebras",
        base_url: "https://api.cerebras.ai/v1",
        api_key_env: "CEREBRAS_API_KEY",
        // Verified against a live free key's /models (2026-06-14): Cerebras's
        // free tier serves gpt-oss-120b (not any llama tag — an earlier
        // llama-3.3-70b guess 404'd in the real-path E2E).
        default_model: "gpt-oss-120b",
        get_key_url: "https://cloud.cerebras.ai",
        no_credit_card: true,
    },
    FreeProvider {
        slug: "openrouter",
        display: "OpenRouter (free models)",
        provider_type: "openrouter",
        base_url: "https://openrouter.ai/api/v1",
        api_key_env: "OPENROUTER_API_KEY",
        default_model: "meta-llama/llama-3.3-70b-instruct:free",
        get_key_url: "https://openrouter.ai/keys",
        no_credit_card: true,
    },
    FreeProvider {
        slug: "gemini",
        display: "Google Gemini (free)",
        provider_type: "gemini",
        base_url: "https://generativelanguage.googleapis.com/v1beta",
        api_key_env: "GEMINI_API_KEY",
        default_model: "gemini-2.5-flash",
        get_key_url: "https://aistudio.google.com/apikey",
        no_credit_card: true,
    },
];

/// The recommended default free provider to surface in the guided "get a key"
/// step when none is auto-detected. Groq — fastest no-credit-card free tier.
pub fn default_free_provider() -> &'static FreeProvider {
    // SAFETY: FREE_PROVIDERS is a non-empty compile-time const (asserted by the
    // `registry_is_non_empty` test), so [0] never panics.
    &FREE_PROVIDERS[0]
}

/// Look up a free provider by its slug.
pub fn free_provider_by_slug(slug: &str) -> Option<&'static FreeProvider> {
    FREE_PROVIDERS.iter().find(|p| p.slug == slug)
}

/// The env-var names, in resolution order, that hold this provider's key. Mirrors
/// the live key-resolution path exactly: the wire layer checks
/// `PHANTOM_MESH_<UPPER>_API_KEY` first (test/CLI override), then the canonical
/// `<...>_API_KEY` (`agent.rs` sources this via the block's `api_key_env`).
pub fn env_var_candidates(p: &FreeProvider) -> [String; 2] {
    let upper = p.slug.to_ascii_uppercase();
    [
        format!("PHANTOM_MESH_{}_API_KEY", upper),
        p.api_key_env.to_string(),
    ]
}

/// Pure core of env detection: the first registry provider (priority order) for
/// which `present(env_var_name)` returns true for ANY of its env-var candidates.
/// Pure — takes the lookup as a closure so unit tests never touch process env.
pub fn first_with_key_present(present: impl Fn(&str) -> bool) -> Option<&'static FreeProvider> {
    FREE_PROVIDERS
        .iter()
        .find(|p| env_var_candidates(p).iter().any(|name| present(name)))
}

/// Auto-detect a free provider whose key is ALREADY in the environment — the
/// zero-config default. Impure wrapper over [`first_with_key_present`]; reads
/// process env, no network.
pub fn detect_free_from_env() -> Option<&'static FreeProvider> {
    first_with_key_present(|name| {
        std::env::var(name)
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_non_empty() {
        assert!(!FREE_PROVIDERS.is_empty(), "free provider registry must not be empty");
    }

    #[test]
    fn registry_entries_are_well_formed() {
        // Every entry must be runtime-resolvable: a non-empty slug/type, an
        // HTTPS base URL, a `<...>_API_KEY` env name, a default model, and a
        // get-key URL. A malformed entry would write a dead provider block.
        for p in FREE_PROVIDERS {
            assert!(!p.slug.is_empty(), "slug empty: {p:?}");
            assert!(!p.display.is_empty(), "display empty: {p:?}");
            assert!(!p.provider_type.is_empty(), "type empty: {p:?}");
            assert!(
                p.base_url.starts_with("https://"),
                "base_url must be https: {p:?}"
            );
            assert!(
                p.api_key_env.ends_with("_API_KEY"),
                "api_key_env must look like an env var: {p:?}"
            );
            assert!(!p.default_model.is_empty(), "default_model empty: {p:?}");
            assert!(
                p.get_key_url.starts_with("https://"),
                "get_key_url must be https: {p:?}"
            );
            // The MVP registry only lists no-credit-card tiers — that is the
            // whole point ("一般使用者預設就能用").
            assert!(p.no_credit_card, "registry is no-credit-card only: {p:?}");
        }
    }

    #[test]
    fn provider_types_are_runtime_supported() {
        // resolver.rs::build_provider special-cases `gemini` and routes
        // everything else through the OpenAI-compat client. Guard against a
        // typo'd type that would silently misroute.
        const SUPPORTED: &[&str] = &["groq", "cerebras", "openrouter", "gemini", "openai_compat"];
        for p in FREE_PROVIDERS {
            assert!(
                SUPPORTED.contains(&p.provider_type),
                "provider_type '{}' is not a resolver-supported type",
                p.provider_type
            );
        }
    }

    #[test]
    fn slugs_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for p in FREE_PROVIDERS {
            assert!(seen.insert(p.slug), "duplicate slug: {}", p.slug);
        }
    }

    #[test]
    fn default_free_provider_is_groq() {
        let d = default_free_provider();
        assert_eq!(d.slug, "groq");
        assert!(d.no_credit_card);
    }

    #[test]
    fn lookup_by_slug_roundtrips() {
        assert_eq!(free_provider_by_slug("groq").map(|p| p.slug), Some("groq"));
        assert_eq!(free_provider_by_slug("cerebras").map(|p| p.slug), Some("cerebras"));
        assert!(free_provider_by_slug("does-not-exist").is_none());
    }

    #[test]
    fn env_var_candidates_match_runtime_resolution() {
        // Must match what `agent.rs` (api_key_env) + the wire layer
        // (PHANTOM_MESH_<UPPER>_API_KEY) actually read, or detection lies.
        let groq = free_provider_by_slug("groq").unwrap();
        assert_eq!(
            env_var_candidates(groq),
            ["PHANTOM_MESH_GROQ_API_KEY".to_string(), "GROQ_API_KEY".to_string()]
        );
    }

    #[test]
    fn detection_respects_priority_order() {
        // Only an OpenRouter key present → pick OpenRouter.
        let only_openrouter = first_with_key_present(|n| n == "OPENROUTER_API_KEY");
        assert_eq!(only_openrouter.map(|p| p.slug), Some("openrouter"));

        // Both Groq and OpenRouter keys present → Groq wins (higher priority).
        let both = first_with_key_present(|n| n == "GROQ_API_KEY" || n == "OPENROUTER_API_KEY");
        assert_eq!(both.map(|p| p.slug), Some("groq"));

        // The PHANTOM_MESH_ override name is also honoured.
        let via_override = first_with_key_present(|n| n == "PHANTOM_MESH_CEREBRAS_API_KEY");
        assert_eq!(via_override.map(|p| p.slug), Some("cerebras"));

        // Nothing present → no provider.
        assert!(first_with_key_present(|_| false).is_none());
    }
}
