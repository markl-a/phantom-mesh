//! First-run `agents.toml` writer — the **single** source of truth shared by
//! every onboarding surface (CLI `run_first_time_onboarding`, and the desktop /
//! mobile GUI via the `finalize_onboarding_config` Tauri command).
//!
//! Lifting this out of the `phantom` bin (where it used to live, bin-private) is
//! step 1 of the unified onboarding design (`docs/DESIGN-ONBOARDING.md` §8.1):
//! one engine, three thin shells. Before this, the CLI wrote `agents.toml` here
//! while the streamlined GUI wrote nothing — the two flows diverged. Now both
//! call this exact writer so the generated config is identical everywhere.
//!
//! Invariant: **no secret is ever written** — only the env-var NAME
//! (`api_key_env`), exactly the seam `agent.rs` sources the key from. A key the
//! user pastes is persisted by the caller via `keys::set_api_key` (properly
//! escaped), never string-built here.
//!
//! 中文: 首啟 `agents.toml` 寫入器 —— onboarding 三介面(CLI / 桌面 / 手機)共用的
//! 唯一真相。原本藏在 phantom bin 裡(bin-private),抽到 lib 是統一設計的第一步:
//! 一個引擎、三層薄殼。設定檔只寫 env 變數「名稱」,不寫秘密。

use anyhow::Result;

use crate::providers::free_plugin::FreeProvider;
use crate::providers::local_servers::LocalServer;

/// Canonical first-run `default_model` for an onboarding provider block.
///
/// The block *name* doubles as the provider *type* for subscription blocks
/// (`claude_cli` / `codex_oauth` / `gemini_oauth`); the Ollama fallback block is
/// named `local-ollama` but typed `ollama`, so callers pass `"ollama"` here. The
/// strings below are exactly the values the rest of the codebase treats as the
/// canonical default for each type — see:
///   - `claude_cli`   → `claude-sonnet-4-5-20251022` (config.rs `default_model()`
///     and the `[providers.anthropic].default_model` representative fixture; the
///     `claude_cli` type streams native Anthropic `/v1/messages`).
///   - `codex_oauth`  → `gpt-5.5` (providers/codex_oauth.rs: the model used when
///     the caller passes an empty model). NOTE: the ChatGPT-account Codex backend
///     gates by model — `gpt-5.x-codex` ids return HTTP 400 "not supported when
///     using Codex with a ChatGPT account". `gpt-5.5` is what the official `codex`
///     CLI defaults to for a ChatGPT login and is verified working here.
///   - `gemini_oauth` → `gemini-2.5-flash` (providers/gemini_oauth.rs: ditto).
///   - `ollama`       → a model the local server advertises if known, else
///     `llama3.1:8b` (the OpenAI-`/models`-shape example in
///     providers/local_servers.rs — a near-universal small Ollama tag).
///
/// `detected` is the local server's first advertised model (Ollama only). No
/// secret/key material is ever produced here.
pub fn onboarding_default_model(provider_type: &str, detected: Option<&str>) -> String {
    match provider_type {
        "claude_cli" | "claude_agent" => "claude-sonnet-4-5-20251022".to_string(),
        "codex_oauth" => "gpt-5.5".to_string(),
        "gemini_oauth" => "gemini-2.5-flash".to_string(),
        "ollama" | "local-ollama" | "lmstudio" | "lemonade" => detected
            .filter(|m| !m.is_empty())
            .unwrap_or("llama3.1:8b")
            .to_string(),
        // Unknown/forward-compat block: fall back to the project default so the
        // resolver still finds a model rather than skipping the provider.
        _ => "claude-sonnet-4-5-20251022".to_string(),
    }
}

/// Build + write the first-time `agents.toml` for the onboarding flow.
///
/// `ordered` is the chosen subscription-provider block names in priority order
/// (each one of `claude_cli` / `codex_oauth` / `gemini_oauth`). Ollama is
/// appended LAST as the always-on local fallback (block name `local-ollama`,
/// type `ollama`). No `api_key` lines are ever written — the runtime sources
/// subscription tokens live (see agent.rs) and local servers need no key.
///
/// `free` is the optional free-tier cloud provider plugin (Groq / Cerebras /
/// OpenRouter / Gemini — see `providers::free_plugin`). When present it is
/// written as a `[providers.<slug>]` block carrying `type` / `url` /
/// `api_key_env` / `default_model` (NEVER the key itself — only the env-var
/// NAME, exactly the seam `agent.rs` sources the key from), and inserted into
/// the failover chain AFTER subscriptions but BEFORE `local-ollama` (a real
/// free cloud key beats a maybe-not-running local server). This is the
/// "default-on free API plugin": a no-subscription user with a free key gets a
/// working provider as their primary minute-one; a subscription user gets it as
/// a cloud fallback.
///
/// `[agent.master]` gets `provider = <first>` plus an explicit `providers = [..]`
/// failover list (config.rs priority mechanism) = `ordered`, then the optional
/// `free` provider, then `local-ollama`.
///
/// Every `[providers.*]` block also gets an explicit `default_model`. Without it
/// the runtime model resolver (agent.rs ~1348 / ~2000: per-entry model →
/// `agent.model` → `provider.default_model`) finds nothing — the generated file
/// carries no `provider:model` entry and no `[agent.master].model` — so it skips
/// every provider with "no model — entry isn't `provider:model` and provider has
/// no default_model" and `phantom exec` fails for the brand-new user. Baking a
/// resolvable default per provider type fixes the first-run round-trip.
pub fn write_onboarding_config(
    cfg_path: &std::path::Path,
    ordered: &[&str],
    ollama: Option<&LocalServer>,
    free: Option<&FreeProvider>,
) -> Result<String> {
    use std::io::Write;

    let mut toml = String::new();
    toml.push_str("# phantom-mesh agents.toml — generated by first-time setup\n");
    toml.push_str("# Subscription tokens are sourced live from the upstream CLIs; no keys here.\n\n");
    toml.push_str("[core]\n");
    toml.push_str("host = \"127.0.0.1\"\n");
    toml.push_str("port = 7878\n\n");

    // One [providers.NAME] block per chosen subscription (type only, no key) plus
    // a resolvable default_model so the runtime model resolver can pick a model.
    for name in ordered {
        toml.push_str(&format!("[providers.{}]\n", name));
        toml.push_str(&format!("type = \"{}\"\n", name));
        toml.push_str(&format!(
            "default_model = \"{}\"\n\n",
            onboarding_default_model(name, None)
        ));
    }

    // Free-tier cloud plugin block (Groq / Cerebras / OpenRouter / Gemini),
    // when a free key is available (detected in env or pasted during the guided
    // step). Carries an explicit `url` (deterministic routing) + `api_key_env`
    // (the env-var NAME the live chat path reads — NOT the key) + a resolvable
    // `default_model`. A pasted key, if any, is persisted separately by the
    // caller via the escaping `keys::set_api_key` path — never string-built here,
    // so this writer stays secret-free.
    if let Some(fp) = free {
        toml.push_str(&format!("[providers.{}]\n", fp.slug));
        toml.push_str(&format!("type = \"{}\"\n", fp.provider_type));
        toml.push_str(&format!("url = \"{}\"\n", fp.base_url));
        toml.push_str(&format!("api_key_env = \"{}\"\n", fp.api_key_env));
        toml.push_str(&format!("default_model = \"{}\"\n\n", fp.default_model));
    }

    // Always write the local Ollama fallback last. Type-only on SECRETS (no API
    // key — Ollama needs none), but it MUST carry the localhost url: the resolver
    // has no built-in Ollama default and otherwise falls through unknown
    // OpenAI-compatible types to OpenRouter, so a key-less `type="ollama"` block
    // would silently hit OpenRouter instead of local Ollama (review: codex/opencode).
    // The url is config, not a secret.
    toml.push_str("[providers.local-ollama]\n");
    toml.push_str("type = \"ollama\"\n");
    toml.push_str("url = \"http://127.0.0.1:11434/v1\"\n");
    // A default_model so the runtime resolver ALWAYS resolves a model for the
    // ollama fallback. Without it an ollama-only first-run config (the chain is
    // just ["local-ollama"]) resolves NO model and the first chat fails — the
    // "first-run all-providers-failed-no-model" bug. Prefer a model the local
    // server already advertises; fall back to a sane Ollama tag otherwise.
    toml.push_str(&format!(
        "default_model = \"{}\"\n\n",
        onboarding_default_model(
            "local-ollama",
            ollama.and_then(|o| o.models.first().map(|s| s.as_str()))
        )
    ));

    // P0-7 SYS-B (opt-in): when NOTHING else was chosen (no subscription, no
    // free cloud key), the only tail is local-ollama — which fails the first
    // chat unless Ollama is actually running. With the offline-stub-model
    // feature on, append a built-in always-available stub tail so a zero-config
    // offline desktop never dead-ends (SPEC-03 §8). Block written AFTER ollama
    // so a real running Ollama still wins; the stub is the last resort. Guarded
    // by the feature → the default binary writes a byte-identical config.
    #[cfg(feature = "offline-stub-model")]
    let append_stub = ordered.is_empty() && free.is_none();
    #[cfg(feature = "offline-stub-model")]
    if append_stub {
        let (name, ptype, url) = crate::providers::local_servers::STUB_PROVIDER;
        toml.push_str(&format!("[providers.{}]\n", name));
        toml.push_str(&format!("type = \"{}\"\n", ptype));
        toml.push_str(&format!("url = \"{}\"\n", url));
        toml.push_str(&format!(
            "default_model = \"{}\"\n\n",
            crate::providers::local_servers::STUB_DEFAULT_MODEL
        ));
    }

    // Build the failover priority list: chosen subs first, then the optional
    // free cloud provider (a real free key beats a maybe-offline local server),
    // then local-ollama as the always-on local last resort.
    let mut chain: Vec<String> = ordered.iter().map(|s| s.to_string()).collect();
    if let Some(fp) = free {
        chain.push(fp.slug.to_string());
    }
    chain.push("local-ollama".to_string());
    // The always-usable offline stub is the final last-resort entry (opt-in).
    #[cfg(feature = "offline-stub-model")]
    if append_stub {
        chain.push(crate::providers::local_servers::STUB_PROVIDER.0.to_string());
    }

    toml.push_str("[agent.master]\n");
    if let Some(first) = chain.first() {
        toml.push_str(&format!("provider = \"{}\"\n", first));
    }
    let list = chain
        .iter()
        .map(|p| format!("\"{}\"", p))
        .collect::<Vec<_>>()
        .join(", ");
    toml.push_str(&format!("providers = [{}]\n", list));
    toml.push_str("instructions = \"You are phantom, a helpful AI agent.\"\n");

    let mut f = std::fs::File::create(cfg_path)?;
    f.write_all(toml.as_bytes())?;
    Ok(chain
        .first()
        .cloned()
        .unwrap_or_else(|| "local-ollama".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_to_tmp(ordered: &[&str]) -> String {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("agents.toml");
        write_onboarding_config(&path, ordered, None, None).unwrap();
        std::fs::read_to_string(&path).unwrap()
    }

    #[test]
    fn always_writes_local_ollama_loopback_tail() {
        // Invariant unchanged across the feature: the local-ollama fallback is
        // always present and loopback-only.
        let cfg = write_to_tmp(&[]);
        assert!(cfg.contains("[providers.local-ollama]"));
        assert!(cfg.contains("url = \"http://127.0.0.1:11434/v1\""));
    }

    #[cfg(feature = "offline-stub-model")]
    #[test]
    fn zero_config_appends_offline_stub_tail() {
        // No subscription, no free key, feature ON: the always-usable stub is
        // written AFTER local-ollama and added as the last failover entry.
        let cfg = write_to_tmp(&[]);
        assert!(
            cfg.contains("[providers.local-stub]"),
            "zero-config offline config must carry the stub tail:\n{cfg}"
        );
        assert!(cfg.contains("type = \"stub\""));
        assert!(cfg.contains("url = \"stub://offline\""));
        assert!(cfg.contains("default_model = \"stub-echo\""));
        // Last resort: ollama block precedes the stub block.
        let ollama_at = cfg.find("[providers.local-ollama]").unwrap();
        let stub_at = cfg.find("[providers.local-stub]").unwrap();
        assert!(ollama_at < stub_at, "stub must be the LAST tail (after ollama)");
        // And it is in the failover chain.
        assert!(cfg.contains("\"local-stub\""), "stub must be in the providers chain");
    }

    #[cfg(feature = "offline-stub-model")]
    #[test]
    fn stub_not_appended_when_a_subscription_was_chosen() {
        // A user with a real provider does NOT get the stub — it is strictly the
        // nothing-else-available last resort.
        let cfg = write_to_tmp(&["claude_cli"]);
        assert!(
            !cfg.contains("[providers.local-stub]"),
            "stub must NOT be written when a subscription is configured:\n{cfg}"
        );
    }

    #[cfg(not(feature = "offline-stub-model"))]
    #[test]
    fn no_stub_block_in_default_build() {
        // No-regression: the default binary never writes a stub block.
        let cfg = write_to_tmp(&[]);
        assert!(!cfg.contains("local-stub"), "default build must not emit a stub block");
        assert!(!cfg.contains("stub://offline"));
    }
}
