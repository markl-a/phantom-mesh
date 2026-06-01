//! Example: experimental-hermes-providers (H4).
//!
//! Builds a default `ProviderEntry` for each of the 4 new adapters and
//! verifies (a) the URL builder lands on the correct endpoint and
//! (b) `auth_header` builds a bearer header AND rejects CRLF in the key.
//!
//! Run:
//!   CARGO_TARGET_DIR=D:/tmp/hermes-docs-target \
//!     cargo run -p phantom-mesh \
//!       --example experimental_hermes_providers_example \
//!       --features experimental-hermes-providers
//!
//! Expected last line: `experimental-hermes-providers OK`. Exit code 0.

use phantom_mesh::config::ProviderEntry;
use phantom_mesh::providers::{
    ai21, cohere, fireworks, mistral, nvidia, perplexity, together, xai,
};

fn check_provider(id: &str, expected_url: &str, url_built: String) {
    assert_eq!(url_built, expected_url, "{id} URL mismatch");
    println!("  {id:<10} -> {url_built}");
}

fn make(id: &str) -> ProviderEntry {
    ProviderEntry {
        provider_type: id.into(),
        ..Default::default()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("[1] default streaming URLs:");
    check_provider(
        mistral::PROVIDER_ID,
        "https://api.mistral.ai/v1/chat/completions",
        mistral::streaming_url(&make(mistral::PROVIDER_ID)),
    );
    check_provider(
        xai::PROVIDER_ID,
        "https://api.x.ai/v1/chat/completions",
        xai::streaming_url(&make(xai::PROVIDER_ID)),
    );
    check_provider(
        together::PROVIDER_ID,
        "https://api.together.xyz/v1/chat/completions",
        together::streaming_url(&make(together::PROVIDER_ID)),
    );
    // Fireworks has the unusual /inference/v1 path.
    check_provider(
        fireworks::PROVIDER_ID,
        "https://api.fireworks.ai/inference/v1/chat/completions",
        fireworks::streaming_url(&make(fireworks::PROVIDER_ID)),
    );
    // ── T51 (2026-05-16): v0.6.0 V1 — 4 more provider URL checks ─────────
    // Perplexity: bare /chat/completions (no /v1/ segment).
    check_provider(
        perplexity::PROVIDER_ID,
        "https://api.perplexity.ai/chat/completions",
        perplexity::streaming_url(&make(perplexity::PROVIDER_ID)),
    );
    // AI21: nests under /studio/v1.
    check_provider(
        ai21::PROVIDER_ID,
        "https://api.ai21.com/studio/v1/chat/completions",
        ai21::streaming_url(&make(ai21::PROVIDER_ID)),
    );
    // NVIDIA NIM: integrate-prefixed host with the standard /v1 path.
    check_provider(
        nvidia::PROVIDER_ID,
        "https://integrate.api.nvidia.com/v1/chat/completions",
        nvidia::streaming_url(&make(nvidia::PROVIDER_ID)),
    );
    // Cohere: /v1/chat — NOT /chat/completions (Cohere is not OpenAI-compat).
    check_provider(
        cohere::PROVIDER_ID,
        "https://api.cohere.com/v1/chat",
        cohere::streaming_url(&make(cohere::PROVIDER_ID)),
    );

    println!("[2] auth_header builds Bearer + rejects CRLF:");
    let (_, mistral_v) = mistral::auth_header("sk-good")?;
    let (_, xai_v) = xai::auth_header("sk-good")?;
    let (_, together_v) = together::auth_header("sk-good")?;
    let (_, fireworks_v) = fireworks::auth_header("sk-good")?;
    let (_, perplexity_v) = perplexity::auth_header("sk-good")?;
    let (_, ai21_v) = ai21::auth_header("sk-good")?;
    let (_, nvidia_v) = nvidia::auth_header("sk-good")?;
    for (name, value) in [
        ("mistral", mistral_v),
        ("xai", xai_v),
        ("together", together_v),
        ("fireworks", fireworks_v),
        ("perplexity", perplexity_v),
        ("ai21", ai21_v),
        ("nvidia", nvidia_v),
    ] {
        assert_eq!(value.to_str()?, "Bearer sk-good");
        println!("  {name:<11} ok");
    }
    // Cohere is the lone outlier: X-API-Key, no Bearer prefix.
    let (cohere_name, cohere_v) = cohere::auth_header("sk-good")?;
    assert_eq!(cohere_name.as_str(), "x-api-key");
    assert_eq!(cohere_v.to_str()?, "sk-good");
    println!("  cohere      ok (x-api-key, no Bearer)");
    assert!(
        mistral::auth_header("sk-bad\r\nInjected: yes").is_err(),
        "CRLF must be rejected"
    );
    assert!(
        cohere::auth_header("sk-bad\r\nInjected: yes").is_err(),
        "CRLF must be rejected (cohere)"
    );
    println!("  CRLF rejection OK");

    println!("[3] INFO blocks (each provider module owns its own ProviderInfo type):");
    // Each module defines its own `ProviderInfo` (same field names, distinct types),
    // so we touch them individually rather than collecting into an array.
    assert!(mistral::INFO.default_base_url.starts_with("https://"));
    println!(
        "  {} (env={}, model={})",
        mistral::INFO.id,
        mistral::INFO.api_key_env,
        mistral::INFO.default_model
    );
    assert!(xai::INFO.default_base_url.starts_with("https://"));
    println!(
        "  {} (env={}, model={})",
        xai::INFO.id,
        xai::INFO.api_key_env,
        xai::INFO.default_model
    );
    assert!(together::INFO.default_base_url.starts_with("https://"));
    println!(
        "  {} (env={}, model={})",
        together::INFO.id,
        together::INFO.api_key_env,
        together::INFO.default_model
    );
    assert!(fireworks::INFO.default_base_url.starts_with("https://"));
    println!(
        "  {} (env={}, model={})",
        fireworks::INFO.id,
        fireworks::INFO.api_key_env,
        fireworks::INFO.default_model
    );
    assert!(perplexity::INFO.default_base_url.starts_with("https://"));
    println!(
        "  {} (env={}, model={})",
        perplexity::INFO.id,
        perplexity::INFO.api_key_env,
        perplexity::INFO.default_model
    );
    assert!(ai21::INFO.default_base_url.starts_with("https://"));
    println!(
        "  {} (env={}, model={})",
        ai21::INFO.id,
        ai21::INFO.api_key_env,
        ai21::INFO.default_model
    );
    assert!(nvidia::INFO.default_base_url.starts_with("https://"));
    println!(
        "  {} (env={}, model={})",
        nvidia::INFO.id,
        nvidia::INFO.api_key_env,
        nvidia::INFO.default_model
    );
    assert!(cohere::INFO.default_base_url.starts_with("https://"));
    println!(
        "  {} (env={}, model={})",
        cohere::INFO.id,
        cohere::INFO.api_key_env,
        cohere::INFO.default_model
    );

    println!("experimental-hermes-providers OK");
    Ok(())
}
