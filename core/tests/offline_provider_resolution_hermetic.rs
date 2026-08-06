//! Platform-agnostic, hermetic offline-first assertions (SYS-B): provider resolution + empty-scan->stub fallback work with no network, no subprocess, no live model.

use spectyn_mesh::config::{AgentsConfig, ProviderEntry};
use spectyn_mesh::providers::resolver::DefaultProviderResolver;

#[cfg(feature = "offline-stub-model")]
use spectyn_mesh::onboarding_config::write_onboarding_config;

#[test]
fn offline_provider_resolution_succeeds_without_network() {
    let mut cfg = AgentsConfig::with_defaults();
    cfg.providers.clear();
    cfg.providers.insert(
        "local-ollama".into(),
        ProviderEntry {
            provider_type: "ollama".into(),
            url: Some("http://127.0.0.1:11434/v1".into()),
            ..Default::default()
        },
    );
    cfg.providers.insert(
        "primary".into(),
        ProviderEntry {
            provider_type: "openai".into(),
            url: Some("http://127.0.0.1:1234/v1".into()),
            ..Default::default()
        },
    );

    let r = DefaultProviderResolver::from_config(&cfg);

    // Resolution is synchronous construction only; it performs no network call,
    // so it works with zero connectivity (SYS-B offline-first).
    assert!(r.resolve("local-ollama").is_some());
    assert!(r.resolve("primary").is_some());
    assert!(r.resolve("nonexistent").is_none());
}

#[cfg(feature = "offline-stub-model")]
#[test]
fn empty_scan_falls_back_to_resolvable_stub() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let cfg_path = dir.path().join("agents.toml");

    write_onboarding_config(&cfg_path, &[], None, None).expect("write onboarding config");
    let written = std::fs::read_to_string(&cfg_path).expect("read onboarding config");

    assert!(written.contains("type = \"stub\""));
    assert!(written.contains("url = \"stub://offline\""));

    let mut cfg = AgentsConfig::with_defaults();
    cfg.providers.clear();
    cfg.providers.insert(
        "local-stub".into(),
        ProviderEntry {
            provider_type: "stub".into(),
            url: Some("stub://offline".into()),
            ..Default::default()
        },
    );

    assert!(DefaultProviderResolver::from_config(&cfg)
        .resolve("local-stub")
        .is_some());
}
