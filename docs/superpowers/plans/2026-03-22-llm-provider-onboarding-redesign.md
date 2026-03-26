# LLM Provider Onboarding Redesign — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Overhaul desktop onboarding to auto-detect credentials, support subscription services (Copilot, Claude CLI), add 6 new providers, and split into Discovery + Manual two-step flow.

**Architecture:** Rust backend adds auth_profile.rs (unified auth model), credential_scanner.rs (parallel scanning), and 4 new provider files (copilot, claude_cli, azure_openai, bedrock). Frontend splits StepProviders into StepProviderDiscovery (auto-detect) + StepProviderManual (API keys), extending the wizard from 5 to 6 steps.

**Tech Stack:** Rust (phantom-mesh), Tauri v2 (phantom-mesh-desktop), React + TypeScript, reqwest, tokio, serde, dirs

**Spec:** `phantom-mesh/docs/superpowers/specs/2026-03-22-llm-provider-onboarding-redesign.md`

**Build note:** This repo is on an exFAT drive. Use `CARGO_TARGET_DIR=target_onboarding` to avoid file lock conflicts with other cargo processes.

---

## File Structure

### New Files (phantom-mesh)

| File | Responsibility |
|------|---------------|
| `src/providers/auth_profile.rs` | `CredentialType`, `AuthProfile`, `ProfileUsageStats`, `FailoverReason`, `ProviderTier` serde |
| `src/providers/credential_scanner.rs` | `scan_all()`, per-source scanners, `DiscoveredCredential`, `DiscoveredProviderInfo` |
| `src/providers/copilot.rs` | `CopilotTokenManager`, `CopilotAwareProvider` wrapping `OpenAiCompatProvider` |
| `src/providers/claude_cli.rs` | `ClaudeCliTokenManager`, reads `~/.claude/` auth files |
| `src/providers/azure_openai.rs` | `AzureOpenAiProvider` with Azure-specific URL/header patterns |
| `src/providers/bedrock.rs` | `BedrockProvider` behind `bedrock` feature flag |

### New Files (phantom-mesh-desktop)

| File | Responsibility |
|------|---------------|
| `src/components/onboarding/StepProviderDiscovery.tsx` | Auto-detect results + one-click login UI |

### Modified Files

| File | Change |
|------|--------|
| `phantom-mesh/src/providers/mod.rs` | Add 6 `pub mod` declarations + re-exports |
| `phantom-mesh/src/providers/tier.rs` | Add serde rename/alias to `ProviderTier` |
| `phantom-mesh/Cargo.toml` | Add `which = "7"`, `aws-sdk-bedrockruntime`, `aws-config` (optional) |
| `phantom-mesh-desktop/src-tauri/Cargo.toml` | Add `phantom-mesh = { path = "../../phantom-mesh" }`, `dirs = "5"` |
| `phantom-mesh-desktop/src/components/onboarding/types.ts` | Add new interfaces, update `OnboardingData`, extend `WizardStep` |
| `phantom-mesh-desktop/src/components/onboarding/useWizardState.ts` | New `INITIAL_DATA`, `goTo()`, providers→manualProviders rename |
| `phantom-mesh-desktop/src/components/onboarding/StepProviders.tsx` | Rename to `StepProviderManual.tsx`, add providers + Azure/Bedrock |
| `phantom-mesh-desktop/src/components/onboarding/OnboardingWizard.tsx` | 6-step layout, insert StepProviderDiscovery |
| `phantom-mesh-desktop/src/components/onboarding/StepComplete.tsx` | Merge discovered + manual providers into write_config |
| `phantom-mesh-desktop/src-tauri/src/commands/onboarding.rs` | New commands + updated `write_config` |
| `phantom-mesh-desktop/src-tauri/src/commands/mod.rs` | Register new commands |
| `phantom-mesh-desktop/src-tauri/src/main.rs` | Register new commands in handler |

---

## Task 1: auth_profile.rs — Unified Auth Model + FailoverReason

Foundation types that everything else depends on.

**Files:**
- Modify: `phantom-mesh/src/providers/tier.rs`
- Create: `phantom-mesh/src/providers/auth_profile.rs`
- Modify: `phantom-mesh/src/providers/mod.rs`

**Reference:** Read `phantom-mesh/src/providers/reliable.rs` for `ErrorClass` enum (variants: `NonRetryable`, `RateLimited`, `Transient`). Read `phantom-mesh/src/providers/tier.rs` for existing `ProviderTier` enum (derives include `Hash`; variants: `Local = 1`, `FreeApi = 2`, `Subscription = 3`, `PayAsYouGo = 4`). Read `phantom-mesh/src/providers/subscription_pacer.rs` for `DateTime<Utc>` usage pattern.

- [ ] **Step 1: Update tier.rs — add serde rename/alias to ProviderTier**

This must happen first so auth_profile.rs can re-export the serde-aware enum.

In `phantom-mesh/src/providers/tier.rs`, update the `ProviderTier` derive + variants. **Keep `Hash`** (it's used as a HashMap key in TierRouter):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ProviderTier {
    #[serde(rename = "local")]
    Local = 1,
    #[serde(rename = "free", alias = "FreeApi")]
    FreeApi = 2,
    #[serde(rename = "subscription")]
    Subscription = 3,
    #[serde(rename = "payg", alias = "PayAsYouGo")]
    PayAsYouGo = 4,
}
```

- [ ] **Step 2: Verify tier.rs compiles and existing tests pass**

```bash
cd phantom-mesh && CARGO_TARGET_DIR=target_onboarding cargo test --lib providers::tier -- --nocapture 2>&1 | tail -20
```
Expected: all 12 existing tier tests pass.

- [ ] **Step 3: Write tests for auth_profile.rs**

```rust
// phantom-mesh/src/providers/auth_profile.rs — at bottom

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::reliable::ErrorClass;

    #[test]
    fn failover_auth_is_non_retryable() {
        assert!(matches!(
            FailoverReason::Auth.to_error_class(),
            ErrorClass::NonRetryable
        ));
    }

    #[test]
    fn failover_billing_is_non_retryable() {
        assert!(matches!(
            FailoverReason::Billing.to_error_class(),
            ErrorClass::NonRetryable
        ));
    }

    #[test]
    fn failover_rate_limit_is_rate_limited() {
        assert!(matches!(
            FailoverReason::RateLimit.to_error_class(),
            ErrorClass::RateLimited
        ));
    }

    #[test]
    fn failover_overload_is_transient() {
        assert!(matches!(
            FailoverReason::Overload.to_error_class(),
            ErrorClass::Transient
        ));
    }

    #[test]
    fn failover_context_overflow_is_transient() {
        assert!(matches!(
            FailoverReason::ContextOverflow.to_error_class(),
            ErrorClass::Transient
        ));
    }

    #[test]
    fn failover_timeout_is_transient() {
        assert!(matches!(
            FailoverReason::Timeout.to_error_class(),
            ErrorClass::Transient
        ));
    }

    #[test]
    fn profile_usage_stats_default() {
        let stats = ProfileUsageStats::default();
        assert_eq!(stats.success_count, 0);
        assert_eq!(stats.failure_count, 0);
        assert!(stats.last_used.is_none());
        assert!(stats.cooldown_until.is_none());
    }

    #[test]
    fn provider_tier_serde_json_roundtrip() {
        let tier = ProviderTier::PayAsYouGo;
        let json = serde_json::to_string(&tier).unwrap();
        assert_eq!(json, "\"payg\"");
        let back: ProviderTier = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, ProviderTier::PayAsYouGo));
    }

    #[test]
    fn provider_tier_serde_alias() {
        // Old format should still deserialize
        let back: ProviderTier = serde_json::from_str("\"FreeApi\"").unwrap();
        assert!(matches!(back, ProviderTier::FreeApi));
    }
}
```

- [ ] **Step 4: Implement auth_profile.rs**

`auth_profile.rs` re-exports `ProviderTier` from `tier.rs` — **no duplicate enum**.

```rust
// phantom-mesh/src/providers/auth_profile.rs

use chrono::{DateTime, Utc};
use std::path::PathBuf;

use super::reliable::ErrorClass;
pub use super::tier::ProviderTier;

// ── Credential Types ───────────────────────────────────────

#[derive(Debug, Clone)]
pub enum CredentialType {
    /// Standard API key (OpenAI, Anthropic, Groq, etc.)
    ApiKey { key: String },
    /// OAuth token with optional refresh (Codex, Copilot, gcloud)
    OAuth {
        access_token: String,
        refresh_token: Option<String>,
        expires_at: Option<DateTime<Utc>>,
    },
    /// Token read from a local file (Claude CLI, Copilot hosts.json)
    TokenFile {
        token: String,
        source_path: PathBuf,
    },
}

// ── Auth Profile ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AuthProfile {
    pub provider_name: String,
    pub credential: CredentialType,
    pub tier: ProviderTier,
    pub usage_stats: ProfileUsageStats,
}

#[derive(Debug, Clone, Default)]
pub struct ProfileUsageStats {
    pub last_used: Option<DateTime<Utc>>,
    pub success_count: u64,
    pub failure_count: u64,
    pub last_error: Option<String>,
    pub cooldown_until: Option<DateTime<Utc>>,
}

// ── Failover Reason ────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum FailoverReason {
    Auth,
    Billing,
    RateLimit,
    Overload,
    ContextOverflow,
    Timeout,
}

impl FailoverReason {
    /// Convert to existing ErrorClass for retry logic compatibility
    pub fn to_error_class(&self) -> ErrorClass {
        match self {
            Self::Auth | Self::Billing => ErrorClass::NonRetryable,
            Self::RateLimit => ErrorClass::RateLimited,
            Self::Overload | Self::ContextOverflow | Self::Timeout => ErrorClass::Transient,
        }
    }
}
```

- [ ] **Step 5: Add `pub mod auth_profile;` to mod.rs**

In `phantom-mesh/src/providers/mod.rs`, add after other `pub mod` declarations:
```rust
pub mod auth_profile;
```

And add re-export:
```rust
pub use auth_profile::{CredentialType, AuthProfile, ProfileUsageStats, FailoverReason};
```

- [ ] **Step 6: Run tests to verify they pass**

```bash
cd phantom-mesh && CARGO_TARGET_DIR=target_onboarding cargo test --lib providers::auth_profile -- --nocapture 2>&1 | tail -20
```
Expected: all 9 tests pass.

- [ ] **Step 7: Commit**

```bash
git add phantom-mesh/src/providers/auth_profile.rs phantom-mesh/src/providers/mod.rs phantom-mesh/src/providers/tier.rs
git commit -m "feat: add auth_profile.rs with CredentialType, FailoverReason, ProviderTier serde"
```

---

## Task 2: credential_scanner.rs — Parallel Credential Scanner

**Files:**
- Create: `phantom-mesh/src/providers/credential_scanner.rs`
- Modify: `phantom-mesh/src/providers/mod.rs`

**Prerequisite:** Add `which = "7"` to `phantom-mesh/Cargo.toml` `[dependencies]` (needed by `scan_opencode()`).

**Reference:** Read `phantom-mesh/src/providers/codex.rs` for `read_auth_file()` pattern (JSON parsing of `~/.codex/auth.json`). Read `phantom-mesh/src/providers/auth_profile.rs` for `CredentialType`, `ProviderTier`.

- [ ] **Step 1: Write tests for credential scanner data structures and env var scanning**

```rust
// phantom-mesh/src/providers/credential_scanner.rs — at bottom

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovered_credential_to_frontend_info_strips_secrets() {
        let cred = DiscoveredCredential {
            provider_name: "openai".to_string(),
            provider_type: "openai".to_string(),
            source: CredentialSource::EnvVar("OPENAI_API_KEY".to_string()),
            credential: CredentialType::ApiKey {
                key: "sk-secret-key-12345".to_string(),
            },
            tier: ProviderTier::PayAsYouGo,
            display_label: "OpenAI".to_string(),
            available_models: vec![],
        };

        let info = cred.to_frontend_info();
        assert_eq!(info.name, "openai");
        assert_eq!(info.provider_type, "openai");
        assert_eq!(info.source, "env_var");
        assert_eq!(info.tier, "payg");
        assert_eq!(info.display_label, "OpenAI");
        // Verify no secret key in the info struct fields
        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains("sk-secret-key-12345"));
    }

    #[test]
    fn discovered_credential_to_frontend_info_token_file() {
        let cred = DiscoveredCredential {
            provider_name: "copilot".to_string(),
            provider_type: "copilot".to_string(),
            source: CredentialSource::TokenFile("/home/user/.config/github-copilot/hosts.json".into()),
            credential: CredentialType::TokenFile {
                token: "gho_secret".to_string(),
                source_path: "/home/user/.config/github-copilot/hosts.json".into(),
            },
            tier: ProviderTier::Subscription,
            display_label: "GitHub Copilot".to_string(),
            available_models: vec!["gpt-4o".to_string()],
        };

        let info = cred.to_frontend_info();
        assert_eq!(info.source, "token_file");
        assert_eq!(info.tier, "subscription");
        assert_eq!(info.models, vec!["gpt-4o"]);
    }

    #[test]
    fn scan_env_vars_detects_set_keys() {
        // Set a test env var
        std::env::set_var("PHANTOM_MESH_TEST_OPENAI_API_KEY", "sk-test-123");

        let results = scan_single_env_var(
            "PHANTOM_MESH_TEST_OPENAI_API_KEY",
            "test_openai",
            "openai",
            "Test OpenAI",
        );

        assert!(results.is_some());
        let cred = results.unwrap();
        assert_eq!(cred.provider_name, "test_openai");
        assert!(matches!(cred.tier, ProviderTier::PayAsYouGo));

        std::env::remove_var("PHANTOM_MESH_TEST_OPENAI_API_KEY");
    }

    #[test]
    fn scan_env_vars_skips_unset_keys() {
        std::env::remove_var("PHANTOM_MESH_TEST_MISSING_KEY");
        let results = scan_single_env_var(
            "PHANTOM_MESH_TEST_MISSING_KEY",
            "missing",
            "openai_compat",
            "Missing",
        );
        assert!(results.is_none());
    }

    #[test]
    fn scan_env_vars_skips_empty_keys() {
        std::env::set_var("PHANTOM_MESH_TEST_EMPTY_KEY", "");
        let results = scan_single_env_var(
            "PHANTOM_MESH_TEST_EMPTY_KEY",
            "empty",
            "openai_compat",
            "Empty",
        );
        assert!(results.is_none());
        std::env::remove_var("PHANTOM_MESH_TEST_EMPTY_KEY");
    }

    #[test]
    fn credential_source_display() {
        assert_eq!(
            CredentialSource::EnvVar("KEY".to_string()).as_str(),
            "env_var"
        );
        assert_eq!(
            CredentialSource::TokenFile("/path".into()).as_str(),
            "token_file"
        );
        assert_eq!(
            CredentialSource::LocalProbe("http://localhost".to_string()).as_str(),
            "local_probe"
        );
        assert_eq!(
            CredentialSource::CliTool("opencode".to_string()).as_str(),
            "cli_tool"
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd phantom-mesh && CARGO_TARGET_DIR=target_onboarding cargo test --lib providers::credential_scanner 2>&1 | head -30
```
Expected: compilation error — module doesn't exist.

- [ ] **Step 3: Implement credential_scanner.rs**

```rust
// phantom-mesh/src/providers/credential_scanner.rs

use serde::Serialize;
use std::path::PathBuf;
use tracing::warn;

use super::auth_profile::{CredentialType, ProviderTier};

// ── Data Structures ────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum CredentialSource {
    TokenFile(PathBuf),
    EnvVar(String),
    LocalProbe(String),
    CliTool(String),
}

impl CredentialSource {
    pub fn as_str(&self) -> &str {
        match self {
            Self::TokenFile(_) => "token_file",
            Self::EnvVar(_) => "env_var",
            Self::LocalProbe(_) => "local_probe",
            Self::CliTool(_) => "cli_tool",
        }
    }
}

/// Internal — contains secrets, never sent to frontend
#[derive(Debug, Clone)]
pub struct DiscoveredCredential {
    pub provider_name: String,
    pub provider_type: String,
    pub source: CredentialSource,
    pub credential: CredentialType,
    pub tier: ProviderTier,
    pub display_label: String,
    pub available_models: Vec<String>,
}

/// Frontend-safe — no secrets
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredProviderInfo {
    pub name: String,
    pub provider_type: String,
    pub source: String,
    pub tier: String,
    pub display_label: String,
    pub models: Vec<String>,
}

impl DiscoveredCredential {
    pub fn to_frontend_info(&self) -> DiscoveredProviderInfo {
        let tier_str = match self.tier {
            ProviderTier::Local => "local",
            ProviderTier::FreeApi => "free",
            ProviderTier::Subscription => "subscription",
            ProviderTier::PayAsYouGo => "payg",
        };
        DiscoveredProviderInfo {
            name: self.provider_name.clone(),
            provider_type: self.provider_type.clone(),
            source: self.source.as_str().to_string(),
            tier: tier_str.to_string(),
            display_label: self.display_label.clone(),
            models: self.available_models.clone(),
        }
    }
}

// ── Scan Engine ────────────────────────────────────────────

pub async fn scan_all() -> Vec<DiscoveredCredential> {
    let (ollama, codex, copilot, claude, gcloud, opencode, env_vars, aws) = tokio::join!(
        scan_ollama(),
        scan_codex(),
        scan_copilot(),
        scan_claude_cli(),
        scan_gcloud(),
        scan_opencode(),
        scan_env_vars(),
        scan_aws(),
    );
    let mut results = Vec::new();
    results.extend(ollama);
    results.extend(codex);
    results.extend(copilot);
    results.extend(claude);
    results.extend(gcloud);
    results.extend(opencode);
    results.extend(env_vars);
    results.extend(aws);
    results
}

// ── Per-Source Scanners ────────────────────────────────────

async fn scan_ollama() -> Vec<DiscoveredCredential> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    match client.get("http://localhost:11434/api/tags").send().await {
        Ok(resp) if resp.status().is_success() => {
            let models = if let Ok(body) = resp.json::<serde_json::Value>().await {
                body["models"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| m["name"].as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                vec![]
            };
            vec![DiscoveredCredential {
                provider_name: "ollama".to_string(),
                provider_type: "ollama".to_string(),
                source: CredentialSource::LocalProbe("http://localhost:11434".to_string()),
                credential: CredentialType::ApiKey {
                    key: String::new(),
                },
                tier: ProviderTier::Local,
                display_label: "Ollama (本地)".to_string(),
                available_models: models,
            }]
        }
        _ => vec![],
    }
}

async fn scan_codex() -> Vec<DiscoveredCredential> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return vec![],
    };

    let paths = [
        home.join(".codex").join("auth.json"),
        home.join(".codex-cli").join("auth.json"),
    ];

    for path in &paths {
        if let Ok(content) = tokio::fs::read_to_string(path).await {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                // Nested format: { tokens: { access_token: "..." } }
                // Flat format: { access_token: "..." }
                let token = json["tokens"]["access_token"]
                    .as_str()
                    .or_else(|| json["access_token"].as_str());

                if let Some(tok) = token {
                    if !tok.is_empty() {
                        return vec![DiscoveredCredential {
                            provider_name: "codex".to_string(),
                            provider_type: "codex".to_string(),
                            source: CredentialSource::TokenFile(path.clone()),
                            credential: CredentialType::TokenFile {
                                token: tok.to_string(),
                                source_path: path.clone(),
                            },
                            tier: ProviderTier::Subscription,
                            display_label: "Codex (ChatGPT Plus)".to_string(),
                            available_models: vec![],
                        }];
                    }
                }
            }
        }
    }
    vec![]
}

async fn scan_copilot() -> Vec<DiscoveredCredential> {
    let paths = copilot_token_paths();

    for path in &paths {
        if let Ok(content) = tokio::fs::read_to_string(path).await {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                // hosts.json: { "github.com": { "oauth_token": "gho_xxx", "user": "..." } }
                // apps.json: similar structure
                if let Some(obj) = json.as_object() {
                    for (_host, val) in obj {
                        if let Some(token) = val["oauth_token"].as_str() {
                            if !token.is_empty() {
                                return vec![DiscoveredCredential {
                                    provider_name: "copilot".to_string(),
                                    provider_type: "copilot".to_string(),
                                    source: CredentialSource::TokenFile(path.clone()),
                                    credential: CredentialType::TokenFile {
                                        token: token.to_string(),
                                        source_path: path.clone(),
                                    },
                                    tier: ProviderTier::Subscription,
                                    display_label: "GitHub Copilot".to_string(),
                                    available_models: vec![],
                                }];
                            }
                        }
                    }
                }
            }
        }
    }
    vec![]
}

pub fn copilot_token_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // Linux/macOS
    if let Some(config) = dirs::config_dir() {
        paths.push(config.join("github-copilot").join("hosts.json"));
        paths.push(config.join("github-copilot").join("apps.json"));
    }

    // Windows: %LOCALAPPDATA%
    #[cfg(target_os = "windows")]
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let dir = PathBuf::from(local_app_data).join("github-copilot");
        paths.push(dir.join("hosts.json"));
        paths.push(dir.join("apps.json"));
    }

    paths
}

async fn scan_claude_cli() -> Vec<DiscoveredCredential> {
    // TODO: update if Claude CLI auth format changes
    let paths = claude_cli_paths();

    for path in &paths {
        if let Ok(content) = tokio::fs::read_to_string(path).await {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                // Try multiple known schemas
                let token = json["sessionKey"]
                    .as_str()
                    .or_else(|| json["token"].as_str())
                    .or_else(|| json["access_token"].as_str())
                    .or_else(|| json["apiKey"].as_str());

                if let Some(tok) = token {
                    if !tok.is_empty() {
                        tracing::debug!("Claude CLI token found at: {}", path.display());
                        return vec![DiscoveredCredential {
                            provider_name: "claude_cli".to_string(),
                            provider_type: "claude_cli".to_string(),
                            source: CredentialSource::TokenFile(path.clone()),
                            credential: CredentialType::TokenFile {
                                token: tok.to_string(),
                                source_path: path.clone(),
                            },
                            tier: ProviderTier::Subscription,
                            display_label: "Claude CLI".to_string(),
                            available_models: vec![],
                        }];
                    }
                }
            }
        }
    }
    vec![]
}

pub fn claude_cli_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(home) = dirs::home_dir() {
        let claude_dir = home.join(".claude");
        paths.push(claude_dir.join(".credentials.json"));
        paths.push(claude_dir.join("credentials.json"));
        paths.push(claude_dir.join("auth.json"));
    }

    // Windows: %APPDATA%\claude\
    if let Some(config) = dirs::config_dir() {
        let claude_dir = config.join("claude");
        paths.push(claude_dir.join(".credentials.json"));
        paths.push(claude_dir.join("credentials.json"));
        paths.push(claude_dir.join("auth.json"));
    }

    paths
}

async fn scan_gcloud() -> Vec<DiscoveredCredential> {
    let adc_path = if let Some(config) = dirs::config_dir() {
        config
            .join("gcloud")
            .join("application_default_credentials.json")
    } else {
        return vec![];
    };

    if let Ok(content) = tokio::fs::read_to_string(&adc_path).await {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if json["client_id"].as_str().is_some() || json["type"].as_str().is_some() {
                return vec![DiscoveredCredential {
                    provider_name: "gemini".to_string(),
                    provider_type: "gemini".to_string(),
                    source: CredentialSource::TokenFile(adc_path),
                    credential: CredentialType::OAuth {
                        access_token: String::new(), // Resolved at runtime by gcloud SDK
                        refresh_token: json["refresh_token"]
                            .as_str()
                            .map(String::from),
                        expires_at: None,
                    },
                    tier: ProviderTier::FreeApi,
                    display_label: "Google Gemini (gcloud ADC)".to_string(),
                    available_models: vec![],
                }];
            }
        }
    }
    vec![]
}

async fn scan_opencode() -> Vec<DiscoveredCredential> {
    let found = tokio::task::spawn_blocking(|| which::which("opencode").is_ok())
        .await
        .unwrap_or(false);

    if found {
        vec![DiscoveredCredential {
            provider_name: "opencode".to_string(),
            provider_type: "opencode".to_string(),
            source: CredentialSource::CliTool("opencode".to_string()),
            credential: CredentialType::ApiKey {
                key: String::new(),
            },
            tier: ProviderTier::Subscription,
            display_label: "OpenCode".to_string(),
            available_models: vec![],
        }]
    } else {
        vec![]
    }
}

// Async for interface consistency with scan_all() tokio::join! — no await points needed
async fn scan_env_vars() -> Vec<DiscoveredCredential> {
    let checks = [
        ("OPENAI_API_KEY", "openai", "openai", "OpenAI"),
        ("ANTHROPIC_API_KEY", "anthropic", "anthropic", "Anthropic"),
        ("GEMINI_API_KEY", "gemini", "gemini", "Gemini"),
        ("GROQ_API_KEY", "groq", "groq", "Groq"),
        ("OPENROUTER_API_KEY", "openrouter", "openai_compat", "OpenRouter"),
        ("DEEPSEEK_API_KEY", "deepseek", "openai_compat", "DeepSeek"),
        ("MISTRAL_API_KEY", "mistral", "openai_compat", "Mistral"),
        ("XAI_API_KEY", "xai", "openai_compat", "xAI (Grok)"),
    ];

    checks
        .iter()
        .filter_map(|(env_key, name, ptype, label)| {
            scan_single_env_var(env_key, name, ptype, label)
        })
        .collect()
}

fn scan_single_env_var(
    env_key: &str,
    name: &str,
    provider_type: &str,
    label: &str,
) -> Option<DiscoveredCredential> {
    match std::env::var(env_key) {
        Ok(val) if !val.is_empty() => Some(DiscoveredCredential {
            provider_name: name.to_string(),
            provider_type: provider_type.to_string(),
            source: CredentialSource::EnvVar(env_key.to_string()),
            credential: CredentialType::ApiKey { key: val },
            tier: ProviderTier::PayAsYouGo,
            display_label: label.to_string(),
            available_models: vec![],
        }),
        _ => None,
    }
}

async fn scan_aws() -> Vec<DiscoveredCredential> {
    // Check env var first
    if std::env::var("AWS_ACCESS_KEY_ID").map(|v| !v.is_empty()).unwrap_or(false) {
        return vec![DiscoveredCredential {
            provider_name: "bedrock".to_string(),
            provider_type: "bedrock".to_string(),
            source: CredentialSource::EnvVar("AWS_ACCESS_KEY_ID".to_string()),
            credential: CredentialType::ApiKey { key: String::new() },
            tier: ProviderTier::PayAsYouGo,
            display_label: "AWS Bedrock".to_string(),
            available_models: vec![],
        }];
    }

    // Check ~/.aws/credentials
    if let Some(home) = dirs::home_dir() {
        let aws_creds = home.join(".aws").join("credentials");
        if aws_creds.exists() {
            return vec![DiscoveredCredential {
                provider_name: "bedrock".to_string(),
                provider_type: "bedrock".to_string(),
                source: CredentialSource::TokenFile(aws_creds),
                credential: CredentialType::ApiKey { key: String::new() },
                tier: ProviderTier::PayAsYouGo,
                display_label: "AWS Bedrock".to_string(),
                available_models: vec![],
            }];
        }
    }

    vec![]
}
```

- [ ] **Step 4: Add `pub mod credential_scanner;` to mod.rs**

In `phantom-mesh/src/providers/mod.rs`, add:
```rust
pub mod credential_scanner;
```

And re-exports:
```rust
pub use credential_scanner::{DiscoveredCredential, DiscoveredProviderInfo, CredentialSource};
```

- [ ] **Step 5: Run tests**

```bash
cd phantom-mesh && CARGO_TARGET_DIR=target_onboarding cargo test --lib providers::credential_scanner -- --nocapture 2>&1 | tail -20
```
Expected: all 5 tests pass.

- [ ] **Step 6: Commit**

```bash
git add phantom-mesh/src/providers/credential_scanner.rs phantom-mesh/src/providers/mod.rs phantom-mesh/Cargo.toml
git commit -m "feat: add credential_scanner.rs with parallel scanning for 8 sources"
```

---

## Task 3: copilot.rs — GitHub Copilot Token Manager + Provider

**Files:**
- Create: `phantom-mesh/src/providers/copilot.rs`
- Modify: `phantom-mesh/src/providers/mod.rs`

**Note on Provider trait:** `CopilotAwareProvider` uses standalone methods (`chat()`, `stream_chat()`, `is_alive()`) that delegate to `OpenAiCompatProvider` with token injection. The formal `impl Provider for CopilotAwareProvider` (matching the trait in `traits.rs`) is deferred — it will be wired in a follow-up task when integrating with the `ProviderRouter`. The standalone methods have the same signatures and can be trivially wrapped.

**Reference:** Read `phantom-mesh/src/providers/codex.rs` for `CodexTokenManager` pattern (token refresh, Mutex, expiry buffer). Read `phantom-mesh/src/providers/openai_compat.rs` for `OpenAiCompatProvider` constructor and `chat_with_token()`.

- [ ] **Step 1: Write tests for CopilotTokenManager**

```rust
// phantom-mesh/src/providers/copilot.rs — at bottom

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copilot_api_token_expired() {
        let token = CopilotApiToken {
            token: "test".to_string(),
            expires_at: Utc::now() - chrono::Duration::minutes(5),
        };
        assert!(token.is_expired());
    }

    #[test]
    fn copilot_api_token_not_expired() {
        let token = CopilotApiToken {
            token: "test".to_string(),
            expires_at: Utc::now() + chrono::Duration::minutes(30),
        };
        assert!(!token.is_expired());
    }

    #[test]
    fn copilot_api_token_near_expiry() {
        // Within 2-minute buffer
        let token = CopilotApiToken {
            token: "test".to_string(),
            expires_at: Utc::now() + chrono::Duration::seconds(60),
        };
        assert!(token.is_expired()); // Should be "expired" due to buffer
    }

    #[test]
    fn parse_copilot_hosts_json() {
        let json = r#"{
            "github.com": {
                "oauth_token": "gho_test123",
                "user": "testuser"
            }
        }"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let token = extract_oauth_token_from_hosts(&parsed);
        assert_eq!(token, Some("gho_test123".to_string()));
    }

    #[test]
    fn parse_copilot_apps_json() {
        let json = r#"{
            "github.com": {
                "oauth_token": "ghu_apps_token",
                "user": "testuser"
            }
        }"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let token = extract_oauth_token_from_hosts(&parsed);
        assert_eq!(token, Some("ghu_apps_token".to_string()));
    }

    #[test]
    fn copilot_token_paths_not_empty() {
        let paths = super::super::credential_scanner::copilot_token_paths();
        assert!(!paths.is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd phantom-mesh && CARGO_TARGET_DIR=target_onboarding cargo test --lib providers::copilot 2>&1 | head -20
```

- [ ] **Step 3: Implement copilot.rs**

```rust
// phantom-mesh/src/providers/copilot.rs

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use super::openai_compat::OpenAiCompatProvider;

const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const COPILOT_BASE_URL: &str = "https://api.githubcopilot.com";
const TOKEN_EXPIRY_BUFFER_SECS: i64 = 120; // 2 minutes

// ── Token Types ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CopilotApiToken {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

impl CopilotApiToken {
    pub fn is_expired(&self) -> bool {
        Utc::now() + chrono::Duration::seconds(TOKEN_EXPIRY_BUFFER_SECS) >= self.expires_at
    }
}

#[derive(Debug, Deserialize)]
struct TokenExchangeResponse {
    token: String,
    expires_at: i64,
}

// ── Token Manager ──────────────────────────────────────────

pub struct CopilotTokenManager {
    oauth_token: Mutex<Option<String>>,
    api_token: Mutex<Option<CopilotApiToken>>,
    token_file_paths: Vec<PathBuf>,
    client: Client,
}

impl CopilotTokenManager {
    pub fn new(token_file_paths: Vec<PathBuf>) -> Self {
        Self {
            oauth_token: Mutex::new(None),
            api_token: Mutex::new(None),
            token_file_paths,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn with_oauth_token(token: String) -> Self {
        Self {
            oauth_token: Mutex::new(Some(token)),
            api_token: Mutex::new(None),
            token_file_paths: vec![],
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Get a valid API token, refreshing if needed
    pub async fn get_token(&self) -> Result<String> {
        // Check cached API token
        {
            let cached = self.api_token.lock().await;
            if let Some(ref token) = *cached {
                if !token.is_expired() {
                    return Ok(token.token.clone());
                }
            }
        }

        // Need to exchange OAuth token for API token
        let oauth_token = self.get_oauth_token().await?;
        let api_token = self.exchange_token(&oauth_token).await?;

        let result = api_token.token.clone();
        *self.api_token.lock().await = Some(api_token);
        Ok(result)
    }

    async fn get_oauth_token(&self) -> Result<String> {
        // Check cached
        {
            let cached = self.oauth_token.lock().await;
            if let Some(ref token) = *cached {
                return Ok(token.clone());
            }
        }

        // Try reading from files
        for path in &self.token_file_paths {
            if let Ok(content) = tokio::fs::read_to_string(path).await {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(token) = extract_oauth_token_from_hosts(&json) {
                        debug!("Copilot OAuth token found at: {}", path.display());
                        *self.oauth_token.lock().await = Some(token.clone());
                        return Ok(token);
                    }
                }
            }
        }

        Err(anyhow!("No GitHub Copilot OAuth token found"))
    }

    async fn exchange_token(&self, oauth_token: &str) -> Result<CopilotApiToken> {
        // GitHub Copilot token exchange uses GET with token auth
        let resp = self
            .client
            .get(COPILOT_TOKEN_URL)
            .header("Authorization", format!("token {}", oauth_token))
            .header("User-Agent", "phantom-mesh")
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| anyhow!("Copilot token exchange failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Copilot token exchange HTTP {}: {}",
                status,
                body
            ));
        }

        let exchange: TokenExchangeResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse Copilot token response: {}", e))?;

        Ok(CopilotApiToken {
            token: exchange.token,
            expires_at: DateTime::from_timestamp(exchange.expires_at, 0)
                .unwrap_or_else(Utc::now),
        })
    }

    pub async fn invalidate(&self) {
        *self.api_token.lock().await = None;
    }
}

pub fn extract_oauth_token_from_hosts(json: &serde_json::Value) -> Option<String> {
    // hosts.json / apps.json: { "github.com": { "oauth_token": "gho_xxx" } }
    if let Some(obj) = json.as_object() {
        for (_host, val) in obj {
            if let Some(token) = val["oauth_token"].as_str() {
                if !token.is_empty() {
                    return Some(token.to_string());
                }
            }
        }
    }
    None
}

// ── Provider ───────────────────────────────────────────────

pub struct CopilotAwareProvider {
    inner: OpenAiCompatProvider,
    token_manager: Arc<CopilotTokenManager>,
}

impl CopilotAwareProvider {
    pub fn new(token_manager: Arc<CopilotTokenManager>) -> Self {
        Self {
            inner: OpenAiCompatProvider::new(
                "copilot".to_string(),
                COPILOT_BASE_URL.to_string(),
                "gpt-4o".to_string(),
                None, // Token injected per-call
            ),
            token_manager,
        }
    }

    pub async fn chat(
        &self,
        messages: &[crate::providers::ChatMessage],
        tools: &[serde_json::Value],
        model: &str,
    ) -> Result<crate::providers::ChatResponse> {
        let token = self.token_manager.get_token().await?;
        self.inner.chat_with_token(messages, tools, model, &token).await
    }

    pub async fn stream_chat(
        &self,
        messages: &[crate::providers::ChatMessage],
        tools: &[serde_json::Value],
        model: &str,
    ) -> Result<std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<crate::providers::StreamChunk>> + Send>>>
    {
        let token = self.token_manager.get_token().await?;
        self.inner
            .stream_chat_with_token(messages, tools, model, &token)
            .await
    }

    pub fn name(&self) -> &str {
        "copilot"
    }

    pub fn default_model(&self) -> &str {
        "gpt-4o"
    }

    pub async fn is_alive(&self) -> bool {
        self.token_manager.get_token().await.is_ok()
    }
}
```

- [ ] **Step 4: Register in mod.rs**

```rust
pub mod copilot;
pub use copilot::{CopilotTokenManager, CopilotAwareProvider, CopilotApiToken};
```

- [ ] **Step 5: Run tests**

```bash
cd phantom-mesh && CARGO_TARGET_DIR=target_onboarding cargo test --lib providers::copilot -- --nocapture 2>&1 | tail -20
```
Expected: all 6 tests pass.

- [ ] **Step 6: Commit**

```bash
git add phantom-mesh/src/providers/copilot.rs phantom-mesh/src/providers/mod.rs
git commit -m "feat: add copilot.rs — GitHub Copilot token manager + OpenAI-compat provider"
```

---

## Task 4: claude_cli.rs — Claude CLI Token Reader

**Files:**
- Create: `phantom-mesh/src/providers/claude_cli.rs`
- Modify: `phantom-mesh/src/providers/mod.rs`

- [ ] **Step 1: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_session_key_format() {
        let json = r#"{ "sessionKey": "sk-ant-session-test123" }"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let token = extract_claude_token(&parsed);
        assert_eq!(token, Some("sk-ant-session-test123".to_string()));
    }

    #[test]
    fn parse_token_format() {
        let json = r#"{ "token": "clt_test456" }"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let token = extract_claude_token(&parsed);
        assert_eq!(token, Some("clt_test456".to_string()));
    }

    #[test]
    fn parse_access_token_format() {
        let json = r#"{ "access_token": "acc_test789" }"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let token = extract_claude_token(&parsed);
        assert_eq!(token, Some("acc_test789".to_string()));
    }

    #[test]
    fn empty_token_returns_none() {
        let json = r#"{ "sessionKey": "" }"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let token = extract_claude_token(&parsed);
        assert!(token.is_none());
    }

    #[test]
    fn no_known_fields_returns_none() {
        let json = r#"{ "unknown_field": "value" }"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let token = extract_claude_token(&parsed);
        assert!(token.is_none());
    }
}
```

- [ ] **Step 2: Implement claude_cli.rs**

```rust
// phantom-mesh/src/providers/claude_cli.rs

use anyhow::{anyhow, Result};
use std::path::PathBuf;
use tokio::sync::Mutex;
use tracing::debug;

// ── Token Extraction ───────────────────────────────────────

/// Try multiple known JSON field names for Claude CLI auth files
// TODO: update if Claude CLI auth format changes
pub fn extract_claude_token(json: &serde_json::Value) -> Option<String> {
    for key in &["sessionKey", "token", "access_token", "apiKey"] {
        if let Some(val) = json[key].as_str() {
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

// ── Token Manager ──────────────────────────────────────────

#[derive(Debug)]
pub struct ClaudeCliCredential {
    pub token: String,
    pub source_path: PathBuf,
}

pub struct ClaudeCliTokenManager {
    credential: Mutex<Option<ClaudeCliCredential>>,
    auth_file_paths: Vec<PathBuf>,
}

impl ClaudeCliTokenManager {
    pub fn new(auth_file_paths: Vec<PathBuf>) -> Self {
        Self {
            credential: Mutex::new(None),
            auth_file_paths,
        }
    }

    pub async fn get_token(&self) -> Result<String> {
        // Check cached
        {
            let cached = self.credential.lock().await;
            if let Some(ref cred) = *cached {
                return Ok(cred.token.clone());
            }
        }

        // Scan files
        for path in &self.auth_file_paths {
            if let Ok(content) = tokio::fs::read_to_string(path).await {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(token) = extract_claude_token(&json) {
                        debug!("Claude CLI token found at: {}", path.display());
                        let cred = ClaudeCliCredential {
                            token: token.clone(),
                            source_path: path.clone(),
                        };
                        *self.credential.lock().await = Some(cred);
                        return Ok(token);
                    }
                }
            }
        }

        Err(anyhow!("No Claude CLI token found"))
    }

    pub async fn invalidate(&self) {
        *self.credential.lock().await = None;
    }
}
```

- [ ] **Step 3: Register in mod.rs, run tests, commit**

```bash
cd phantom-mesh && CARGO_TARGET_DIR=target_onboarding cargo test --lib providers::claude_cli -- --nocapture 2>&1 | tail -15
```

```bash
git add phantom-mesh/src/providers/claude_cli.rs phantom-mesh/src/providers/mod.rs
git commit -m "feat: add claude_cli.rs — Claude CLI token reader with multi-schema support"
```

---

## Task 5: azure_openai.rs — Azure OpenAI Provider

**Files:**
- Create: `phantom-mesh/src/providers/azure_openai.rs`
- Modify: `phantom-mesh/src/providers/mod.rs`

**Note on Provider trait:** Same as copilot.rs — standalone methods now, formal `impl Provider` deferred to router integration task.

**Reference:** Read `phantom-mesh/src/providers/openai_compat.rs` for request/response format (same JSON body, different URL pattern and auth header).

- [ ] **Step 1: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn azure_url_construction() {
        let url = build_azure_url(
            "https://mydeployment.openai.azure.com",
            "gpt-4o",
            "2024-02-01",
        );
        assert_eq!(
            url,
            "https://mydeployment.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version=2024-02-01"
        );
    }

    #[test]
    fn azure_url_strips_trailing_slash() {
        let url = build_azure_url(
            "https://mydeployment.openai.azure.com/",
            "gpt-4o",
            "2024-02-01",
        );
        assert_eq!(
            url,
            "https://mydeployment.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version=2024-02-01"
        );
    }

    #[test]
    fn azure_provider_name() {
        let provider = AzureOpenAiProvider::new(
            "https://test.openai.azure.com".to_string(),
            "test-key".to_string(),
            "2024-02-01".to_string(),
            "gpt-4o".to_string(),
        );
        assert_eq!(provider.name(), "azure_openai");
    }

    #[test]
    fn azure_default_model() {
        let provider = AzureOpenAiProvider::new(
            "https://test.openai.azure.com".to_string(),
            "test-key".to_string(),
            "2024-02-01".to_string(),
            "gpt-4o".to_string(),
        );
        assert_eq!(provider.default_model(), "gpt-4o");
    }
}
```

- [ ] **Step 2: Implement azure_openai.rs**

```rust
// phantom-mesh/src/providers/azure_openai.rs

use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::Value;
use tracing::debug;

use super::{ChatMessage, ChatResponse, StreamChunk};

pub fn build_azure_url(endpoint: &str, model: &str, api_version: &str) -> String {
    let base = endpoint.trim_end_matches('/');
    format!(
        "{}/openai/deployments/{}/chat/completions?api-version={}",
        base, model, api_version
    )
}

pub struct AzureOpenAiProvider {
    endpoint: String,
    api_key: String,
    api_version: String,
    default_model: String,
    client: Client,
}

impl AzureOpenAiProvider {
    pub fn new(
        endpoint: String,
        api_key: String,
        api_version: String,
        default_model: String,
    ) -> Self {
        Self {
            endpoint,
            api_key,
            api_version,
            default_model,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .unwrap_or_default(),
        }
    }

    fn resolve_model<'a>(&'a self, model: &'a str) -> &'a str {
        if model.is_empty() {
            &self.default_model
        } else {
            model
        }
    }

    pub async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<ChatResponse> {
        let model = self.resolve_model(model);
        let url = build_azure_url(&self.endpoint, model, &self.api_version);

        let mut body = serde_json::json!({
            "messages": messages,
            "max_tokens": 4096,
        });
        // Note: model is NOT in the body for Azure — it's in the URL path
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools.to_vec());
        }

        let resp = self
            .client
            .post(&url)
            .header("api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Azure OpenAI request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Azure OpenAI HTTP {}: {}", status, text));
        }

        let response: ChatResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("Azure OpenAI parse error: {}", e))?;

        Ok(response)
    }

    pub fn name(&self) -> &str {
        "azure_openai"
    }

    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    pub async fn is_alive(&self) -> bool {
        // Azure doesn't have a simple /models endpoint — check with a lightweight call
        let url = format!(
            "{}/openai/deployments?api-version={}",
            self.endpoint.trim_end_matches('/'),
            self.api_version
        );
        self.client
            .get(&url)
            .header("api-key", &self.api_key)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}
```

- [ ] **Step 3: Register in mod.rs, run tests, commit**

```bash
cd phantom-mesh && CARGO_TARGET_DIR=target_onboarding cargo test --lib providers::azure_openai -- --nocapture 2>&1 | tail -15
```

```bash
git add phantom-mesh/src/providers/azure_openai.rs phantom-mesh/src/providers/mod.rs
git commit -m "feat: add azure_openai.rs — Azure OpenAI provider with deployment URL pattern"
```

---

## Task 6: bedrock.rs — AWS Bedrock Provider (Feature-Gated)

**Files:**
- Create: `phantom-mesh/src/providers/bedrock.rs`
- Modify: `phantom-mesh/src/providers/mod.rs`
- Modify: `phantom-mesh/Cargo.toml`

- [ ] **Step 1: Add dependencies to Cargo.toml**

Add to `[features]`:
```toml
bedrock = ["dep:aws-sdk-bedrockruntime", "dep:aws-config"]
```

Add to `[dependencies]`:
```toml
aws-sdk-bedrockruntime = { version = "1", optional = true }
aws-config = { version = "1", optional = true }
```

- [ ] **Step 2: Write tests (non-AWS-SDK unit tests only)**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bedrock_model_id_format() {
        assert_eq!(
            normalize_bedrock_model("claude-3-sonnet"),
            "anthropic.claude-3-sonnet-20240229-v1:0"
        );
    }

    #[test]
    fn bedrock_model_id_already_qualified() {
        assert_eq!(
            normalize_bedrock_model("anthropic.claude-3-sonnet-20240229-v1:0"),
            "anthropic.claude-3-sonnet-20240229-v1:0"
        );
    }

    #[test]
    fn bedrock_provider_name() {
        assert_eq!(BedrockProvider::provider_name(), "bedrock");
    }
}
```

- [ ] **Step 3: Implement bedrock.rs**

```rust
// phantom-mesh/src/providers/bedrock.rs

//! AWS Bedrock provider — behind `bedrock` feature flag.
//! Uses the Converse API, not OpenAI-compat.

use std::collections::HashMap;

/// Known Bedrock model ID mappings for short names
fn bedrock_model_map() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert(
        "claude-3-sonnet",
        "anthropic.claude-3-sonnet-20240229-v1:0",
    );
    m.insert(
        "claude-3-haiku",
        "anthropic.claude-3-haiku-20240307-v1:0",
    );
    m.insert(
        "claude-3-opus",
        "anthropic.claude-3-opus-20240229-v1:0",
    );
    m.insert(
        "claude-3.5-sonnet",
        "anthropic.claude-3-5-sonnet-20241022-v2:0",
    );
    m
}

pub fn normalize_bedrock_model(model: &str) -> String {
    if model.contains('.') {
        // Already qualified (e.g., "anthropic.claude-3-sonnet-...")
        return model.to_string();
    }
    bedrock_model_map()
        .get(model)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("anthropic.{}", model))
}

pub struct BedrockProvider {
    pub region: String,
}

impl BedrockProvider {
    pub fn new(region: String) -> Self {
        Self { region }
    }

    pub fn provider_name() -> &'static str {
        "bedrock"
    }
}

// Full AWS SDK integration requires the `bedrock` feature flag.
// The Converse API implementation is added when building with:
//   cargo build --features bedrock
#[cfg(feature = "bedrock")]
mod sdk_impl {
    use super::*;
    use anyhow::Result;

    impl BedrockProvider {
        pub async fn init_client(&self) -> Result<aws_sdk_bedrockruntime::Client> {
            let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
                .region(aws_config::Region::new(self.region.clone()))
                .load()
                .await;
            Ok(aws_sdk_bedrockruntime::Client::new(&config))
        }
    }
}
```

- [ ] **Step 4: Register in mod.rs**

Always compile `bedrock.rs` (the feature gate is on the SDK import inside, not the module):
```rust
pub mod bedrock;
```

- [ ] **Step 5: Run tests (no bedrock feature needed for unit tests)**

```bash
cd phantom-mesh && CARGO_TARGET_DIR=target_onboarding cargo test --lib providers::bedrock -- --nocapture 2>&1 | tail -15
```

- [ ] **Step 6: Commit**

```bash
git add phantom-mesh/src/providers/bedrock.rs phantom-mesh/src/providers/mod.rs phantom-mesh/Cargo.toml
git commit -m "feat: add bedrock.rs — AWS Bedrock provider with feature-gated SDK"
```

---

## Task 7: Frontend Types + State Updates

**Files:**
- Modify: `phantom-mesh-desktop/src/components/onboarding/types.ts`
- Modify: `phantom-mesh-desktop/src/components/onboarding/useWizardState.ts`

**Reference:** Read these files first to understand existing structure.

- [ ] **Step 1: Update types.ts**

Add new interfaces and update existing ones:

```typescript
// Add after existing interfaces

export interface DiscoveredProvider {
  name: string;
  providerType: string;
  source: 'token_file' | 'env_var' | 'local_probe' | 'cli_tool';
  enabled: boolean;
  tier: 'local' | 'free' | 'subscription' | 'payg';
  models: string[];
  displayLabel: string;
}

export interface DiscoveredProviderEntry {
  name: string;
  provider_type: string;
  tier: string;
  token_source: string;
  base_url: string | null;
  env_key_name: string | null;
}

export interface ManualProviderEntry {
  name: string;
  provider_type: string;
  api_key: string;
  tier: string;
  base_url: string | null;
  endpoint: string | null;
  region: string | null;
}

export interface CopilotTokenStatus {
  found: boolean;
  user: string | null;
}

export interface GcloudAdcStatus {
  found: boolean;
  project: string | null;
}

export interface ClaudeCliStatus {
  found: boolean;
}
```

Update existing `ProviderConfig`:
```typescript
export interface ProviderConfig {
  name: string;
  apiKey: string;
  providerType: string;
  validated: boolean;
  models: string[];
  baseUrl?: string;    // NEW: for Azure
  endpoint?: string;   // NEW: for Azure
  region?: string;     // NEW: for Bedrock
}
```

Update `OnboardingData`:
```typescript
export interface OnboardingData {
  hardwareScan: HardwareScanResult | null;
  identity: UserIdentity | null;
  vaultPin: string;
  discoveredProviders: DiscoveredProvider[];  // NEW
  manualProviders: ProviderConfig[];          // RENAMED from providers
  clusterEnabled: boolean;
  clusterNodes: string[];
  telegramToken: string;
  qrPayload: QrPayload | null;
  ollamaEndpoint: string;
  ollamaEnabled: boolean;
}
```

Update `PersistedWizardState`:
```typescript
export interface PersistedWizardState {
  currentStep: number;
  ollamaEnabled: boolean;
  ollamaEndpoint: string;
  providerNames: string[];
  discoveredProviderNames?: string[];  // NEW
  clusterEnabled: boolean;
  clusterNodes: string[];
  telegramConfigured: boolean;
  identityEmail?: string;
  identityProvider?: string;
}
```

Update `WizardStep`:
```typescript
export type WizardStep = 0 | 1 | 2 | 3 | 4 | 5;
```

- [ ] **Step 2: Update useWizardState.ts**

Update `INITIAL_DATA`:
```typescript
const INITIAL_DATA: OnboardingData = {
  hardwareScan: null,
  identity: null,
  vaultPin: '',
  discoveredProviders: [],
  manualProviders: [],
  clusterEnabled: false,
  clusterNodes: [],
  telegramToken: '',
  qrPayload: null,
  ollamaEndpoint: 'http://localhost:11434',
  ollamaEnabled: false,
};
```

Update `goNext` max to 5.

Add `goTo` function:
```typescript
const goTo = (step: WizardStep) => {
  setCurrentStep(step);
};
```

Rename all references from `data.providers` to `data.manualProviders`.

Update persistence effect to include `discoveredProviderNames`:
```typescript
discoveredProviderNames: data.discoveredProviders
  .filter(p => p.enabled)
  .map(p => p.name),
```

Crash recovery for discovered providers: `loadPersistedState()` reads `discoveredProviderNames` from localStorage. The actual async re-scan happens in `StepProviderDiscovery` on mount — it calls `scan_credentials` and pre-checks providers whose names are in the persisted list. `loadPersistedState()` itself does NOT invoke async commands.

Return `goTo` from the hook alongside `goNext`, `goBack`.

- [ ] **Step 3: Verify TypeScript compiles**

```bash
cd phantom-mesh-desktop && npx tsc --noEmit 2>&1 | head -30
```

Expect type errors from components still using `data.providers` — that's expected and will be fixed in Tasks 9-11.

- [ ] **Step 4: Commit**

```bash
git add phantom-mesh-desktop/src/components/onboarding/types.ts phantom-mesh-desktop/src/components/onboarding/useWizardState.ts
git commit -m "feat: update onboarding types for 6-step wizard with discovered + manual providers"
```

---

## Task 8: Tauri Backend — New Commands + Updated write_config

**Files:**
- Modify: `phantom-mesh-desktop/src-tauri/Cargo.toml`
- Modify: `phantom-mesh-desktop/src-tauri/src/commands/onboarding.rs`
- Modify: `phantom-mesh-desktop/src-tauri/src/commands/mod.rs`
- Modify: `phantom-mesh-desktop/src-tauri/src/main.rs`

**Prerequisite:** Add dependencies to `phantom-mesh-desktop/src-tauri/Cargo.toml`:
```toml
[dependencies]
phantom-mesh = { path = "../../phantom-mesh" }
dirs = "5"
```

**Reference:** Read `phantom-mesh-desktop/src-tauri/src/commands/onboarding.rs` for existing command patterns. Read `phantom-mesh/src/providers/credential_scanner.rs` for `scan_all()` and `DiscoveredProviderInfo`.

- [ ] **Step 1: Add new response types + Tauri commands to onboarding.rs**

Add after existing response types:

```rust
#[derive(Debug, Serialize)]
pub struct CopilotTokenStatus {
    pub found: bool,
    pub user: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GcloudAdcStatus {
    pub found: bool,
    pub project: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ClaudeCliStatus {
    pub found: bool,
}
```

Add new commands:

```rust
#[tauri::command]
pub async fn scan_credentials() -> Result<Vec<phantom_mesh::providers::DiscoveredProviderInfo>, String> {
    let discovered = phantom_mesh::providers::credential_scanner::scan_all().await;
    Ok(discovered.iter().map(|d| d.to_frontend_info()).collect())
}

#[tauri::command]
pub async fn read_copilot_token() -> Result<CopilotTokenStatus, String> {
    let paths = phantom_mesh::providers::credential_scanner::copilot_token_paths();
    for path in &paths {
        if let Ok(content) = tokio::fs::read_to_string(path).await {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(obj) = json.as_object() {
                    for (_host, val) in obj {
                        if val["oauth_token"].as_str().is_some() {
                            let user = val["user"].as_str().map(String::from);
                            return Ok(CopilotTokenStatus { found: true, user });
                        }
                    }
                }
            }
        }
    }
    Ok(CopilotTokenStatus { found: false, user: None })
}

#[tauri::command]
pub async fn read_gcloud_adc() -> Result<GcloudAdcStatus, String> {
    let adc_path = dirs::config_dir()
        .map(|c| c.join("gcloud").join("application_default_credentials.json"));
    if let Some(path) = adc_path {
        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                let project = json["quota_project_id"].as_str().map(String::from);
                return Ok(GcloudAdcStatus { found: true, project });
            }
        }
    }
    Ok(GcloudAdcStatus { found: false, project: None })
}

#[tauri::command]
pub async fn read_claude_cli_token() -> Result<ClaudeCliStatus, String> {
    let paths = phantom_mesh::providers::credential_scanner::claude_cli_paths();
    for path in &paths {
        if let Ok(content) = tokio::fs::read_to_string(path).await {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if phantom_mesh::providers::claude_cli::extract_claude_token(&json).is_some() {
                    return Ok(ClaudeCliStatus { found: true });
                }
            }
        }
    }
    Ok(ClaudeCliStatus { found: false })
}
```

- [ ] **Step 2: Update OnboardingConfig + write_config for new provider types**

Replace existing `OnboardingConfig` and `ProviderEntry`:

```rust
#[derive(Debug, Deserialize)]
pub struct OnboardingConfig {
    pub port: u16,
    pub discovered_providers: Vec<DiscoveredProviderEntry>,
    pub manual_providers: Vec<ManualProviderEntry>,
    pub ollama_endpoint: Option<String>,
    pub default_agent_provider: String,
    pub default_agent_model: String,
    pub auth_key: String,
    pub telegram_token: Option<String>,
    pub identity_provider: Option<String>,
    pub identity_sub: Option<String>,
    pub identity_email: Option<String>,
    pub is_primary: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct DiscoveredProviderEntry {
    pub name: String,
    pub provider_type: String,
    pub tier: String,
    pub token_source: String,
    pub base_url: Option<String>,
    pub env_key_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ManualProviderEntry {
    pub name: String,
    pub provider_type: String,
    pub api_key: String,
    pub tier: String,
    pub base_url: Option<String>,
    pub endpoint: Option<String>,
    pub region: Option<String>,
}
```

Replace the `write_config` function body. Full implementation:

```rust
#[tauri::command]
pub async fn write_config(
    app: tauri::AppHandle,
    data: OnboardingConfig,
) -> Result<(), String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e: tauri::Error| e.to_string())?;
    std::fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;

    let toml_path = config_dir.join("agents.toml");
    if toml_path.exists() {
        let backup = config_dir.join("agents.toml.bak");
        std::fs::copy(&toml_path, &backup).ok();
    }

    let mut toml = format!(
        "[core]\nhost = \"0.0.0.0\"\nport = {}\n\n",
        data.port
    );

    // Ollama (handled separately)
    if let Some(ref endpoint) = data.ollama_endpoint {
        toml.push_str(&format!(
            "[providers.ollama]\ntype = \"ollama\"\nurl = \"{}\"\ntier = \"local\"\n\n",
            endpoint
        ));
    }

    // Discovered providers (token_source = "auto" or "env")
    for p in &data.discovered_providers {
        toml.push_str(&format!("[providers.{}]\ntype = \"{}\"\ntier = \"{}\"\n",
            p.name, p.provider_type, p.tier));
        if p.token_source == "auto" {
            toml.push_str(&format!("token_source = \"auto\"\n"));
        } else if let Some(ref env_key) = p.env_key_name {
            toml.push_str(&format!("api_key_env = \"{}\"\n", env_key));
        }
        if let Some(ref base_url) = p.base_url {
            toml.push_str(&format!("base_url = \"{}\"\n", base_url));
        }
        toml.push('\n');
    }

    // Manual providers (API key in .env)
    for p in &data.manual_providers {
        let env_key = format!("{}_API_KEY", p.name.to_uppercase());
        toml.push_str(&format!("[providers.{}]\ntype = \"{}\"\ntier = \"{}\"\napi_key_env = \"{}\"\n",
            p.name, p.provider_type, p.tier, env_key));
        if let Some(ref base_url) = p.base_url {
            toml.push_str(&format!("base_url = \"{}\"\n", base_url));
        }
        // Azure-specific
        if let Some(ref endpoint) = p.endpoint {
            toml.push_str(&format!("endpoint = \"{}\"\n", endpoint));
            toml.push_str("api_version = \"2024-02-01\"\n");
        }
        // Bedrock-specific
        if let Some(ref region) = p.region {
            toml.push_str(&format!("region = \"{}\"\n", region));
        }
        toml.push('\n');
    }

    // Default agent
    toml.push_str(&format!(
        "[agent.master]\nprovider = \"{}\"\nmodel = \"{}\"\ntools = [\"web_search\", \"http_request\"]\ninstructions = \"You are a helpful AI assistant.\"\n\n",
        data.default_agent_provider, data.default_agent_model
    ));

    // Auth
    toml.push_str(&format!("[auth]\nbearer_token = \"{}\"\n", data.auth_key));

    // Identity
    if let (Some(ref provider), Some(ref sub), Some(ref email)) =
        (&data.identity_provider, &data.identity_sub, &data.identity_email)
    {
        if !provider.is_empty() {
            toml.push_str(&format!(
                "\n[identity]\nprovider = \"{}\"\nsub = \"{}\"\nemail = \"{}\"\n",
                provider, sub, email
            ));
        }
    }

    // Sync
    if data.is_primary.unwrap_or(false) {
        toml.push_str("\n[sync]\nis_primary = true\n");
    }

    std::fs::write(&toml_path, &toml).map_err(|e| e.to_string())?;

    // .env — only manual providers (discovered env vars already exist)
    let env_path = config_dir.join(".env");
    let mut env_content = String::new();
    for p in &data.manual_providers {
        env_content.push_str(&format!(
            "{}_API_KEY={}\n",
            p.name.to_uppercase(),
            p.api_key
        ));
    }
    if let Some(ref token) = data.telegram_token {
        env_content.push_str(&format!("TELEGRAM_BOT_TOKEN={}\n", token));
    }
    std::fs::write(&env_path, &env_content).map_err(|e| e.to_string())?;

    Ok(())
}
```

- [ ] **Step 3: Update validate_api_key to support new providers**

Add cases for `"deepseek"`, `"mistral"`, `"xai"`, `"azure"` to the match block:

```rust
"deepseek" => {
    let resp = client
        .get("https://api.deepseek.com/v1/models")
        .bearer_auth(&key)
        .send()
        .await;
    parse_model_list_response(resp, "data", "id").await
}
"mistral" => {
    let resp = client
        .get("https://api.mistral.ai/v1/models")
        .bearer_auth(&key)
        .send()
        .await;
    parse_model_list_response(resp, "data", "id").await
}
"xai" => {
    let resp = client
        .get("https://api.x.ai/v1/models")
        .bearer_auth(&key)
        .send()
        .await;
    parse_model_list_response(resp, "data", "id").await
}
"azure" => {
    // Azure uses api-key header, not Bearer. Key format: endpoint|api_key
    let parts: Vec<&str> = key.splitn(2, '|').collect();
    if parts.len() != 2 {
        return Ok(ValidationResult {
            ok: false, models: vec![],
            error: Some("Format: endpoint|api_key".to_string()),
        });
    }
    let endpoint = parts[0].trim_end_matches('/');
    let api_key = parts[1];
    let url = format!("{}/openai/deployments?api-version=2024-02-01", endpoint);
    let resp = client.get(&url).header("api-key", api_key).send().await;
    match resp {
        Ok(r) if r.status().is_success() => Ok(ValidationResult {
            ok: true, models: vec!["gpt-4o".to_string()], error: None,
        }),
        Ok(r) => Ok(ValidationResult {
            ok: false, models: vec![],
            error: Some(format!("HTTP {}", r.status())),
        }),
        Err(e) => Ok(ValidationResult {
            ok: false, models: vec![], error: Some(e.to_string()),
        }),
    }
}
"bedrock" => {
    // Bedrock uses AWS IAM credentials — validate by checking if credentials exist
    // Actual API call requires AWS SDK, so we just check credential presence
    let has_env = std::env::var("AWS_ACCESS_KEY_ID").map(|v| !v.is_empty()).unwrap_or(false);
    let has_file = dirs::home_dir()
        .map(|h| h.join(".aws").join("credentials").exists())
        .unwrap_or(false);
    Ok(ValidationResult {
        ok: has_env || has_file,
        models: vec!["anthropic.claude-3-sonnet".to_string()],
        error: if !has_env && !has_file {
            Some("No AWS credentials found".to_string())
        } else {
            None
        },
    })
}
```

- [ ] **Step 4: Register commands in mod.rs and main.rs**

In `commands/mod.rs`, ensure `onboarding` module exports the new commands.

In `main.rs`, add to `generate_handler!`:
```rust
commands::onboarding::scan_credentials,
commands::onboarding::read_copilot_token,
commands::onboarding::read_gcloud_adc,
commands::onboarding::read_claude_cli_token,
```

- [ ] **Step 5: Verify compilation**

```bash
cd phantom-mesh-desktop/src-tauri && CARGO_TARGET_DIR=target_onboarding cargo check 2>&1 | tail -20
```

- [ ] **Step 6: Commit**

```bash
git add phantom-mesh-desktop/src-tauri/Cargo.toml phantom-mesh-desktop/src-tauri/src/commands/onboarding.rs phantom-mesh-desktop/src-tauri/src/commands/mod.rs phantom-mesh-desktop/src-tauri/src/main.rs
git commit -m "feat: add scan_credentials, read_copilot/gcloud/claude commands + updated write_config"
```

---

## Task 9: StepProviderDiscovery.tsx — Auto-Detection UI

**Files:**
- Create: `phantom-mesh-desktop/src/components/onboarding/StepProviderDiscovery.tsx`

**Reference:** Read `StepProviders.tsx` for existing provider UI patterns (checkbox, validate, invoke).

- [ ] **Step 1: Create StepProviderDiscovery.tsx**

```tsx
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type {
  OnboardingData,
  DiscoveredProvider,
  CopilotTokenStatus,
  GcloudAdcStatus,
  ClaudeCliStatus,
  WizardStep,
} from './types';

interface Props {
  data: OnboardingData;
  updateData: (partial: Partial<OnboardingData>) => void;
  goNext: () => void;
  goBack: () => void;
  goTo: (step: WizardStep) => void;
}

type ScanStatus = 'idle' | 'scanning' | 'done' | 'error';

export default function StepProviderDiscovery({
  data,
  updateData,
  goNext,
  goBack,
  goTo,
}: Props) {
  const [scanStatus, setScanStatus] = useState<ScanStatus>('idle');
  const [error, setError] = useState<string | null>(null);

  // Auto-scan on mount
  useEffect(() => {
    if (data.discoveredProviders.length > 0) {
      setScanStatus('done');
      return;
    }
    setScanStatus('scanning');
    invoke<Array<{
      name: string;
      providerType: string;
      source: string;
      tier: string;
      displayLabel: string;
      models: string[];
    }>>('scan_credentials')
      .then((results) => {
        const providers: DiscoveredProvider[] = results.map((r) => ({
          name: r.name,
          providerType: r.providerType,
          source: r.source as DiscoveredProvider['source'],
          enabled: true,
          tier: r.tier as DiscoveredProvider['tier'],
          models: r.models,
          displayLabel: r.displayLabel,
        }));
        updateData({ discoveredProviders: providers });

        // Sync Ollama state with legacy fields
        const ollama = providers.find((p) => p.name === 'ollama');
        if (ollama) {
          updateData({ ollamaEnabled: true });
        }

        setScanStatus('done');
      })
      .catch((e) => {
        setError(String(e));
        setScanStatus('error');
      });
  }, []);

  const toggleProvider = (name: string) => {
    const updated = data.discoveredProviders.map((p) =>
      p.name === name ? { ...p, enabled: !p.enabled } : p
    );
    updateData({ discoveredProviders: updated });
  };

  // One-click login handlers
  const handleCopilotLogin = async () => {
    const status = await invoke<CopilotTokenStatus>('read_copilot_token');
    if (status.found) {
      const exists = data.discoveredProviders.some((p) => p.name === 'copilot');
      if (!exists) {
        updateData({
          discoveredProviders: [
            ...data.discoveredProviders,
            {
              name: 'copilot',
              providerType: 'copilot',
              source: 'token_file',
              enabled: true,
              tier: 'subscription',
              models: [],
              displayLabel: `GitHub Copilot (${status.user ?? ''})`,
            },
          ],
        });
      }
    }
  };

  const handleGcloudLogin = async () => {
    const status = await invoke<GcloudAdcStatus>('read_gcloud_adc');
    if (status.found) {
      const exists = data.discoveredProviders.some(
        (p) => p.name === 'gemini' && p.source === 'token_file'
      );
      if (!exists) {
        updateData({
          discoveredProviders: [
            ...data.discoveredProviders,
            {
              name: 'gemini',
              providerType: 'gemini',
              source: 'token_file',
              enabled: true,
              tier: 'free',
              models: [],
              displayLabel: `Google Gemini (gcloud${status.project ? ` — ${status.project}` : ''})`,
            },
          ],
        });
      }
    }
  };

  const handleClaudeSync = async () => {
    const status = await invoke<ClaudeCliStatus>('read_claude_cli_token');
    if (status.found) {
      const exists = data.discoveredProviders.some((p) => p.name === 'claude_cli');
      if (!exists) {
        updateData({
          discoveredProviders: [
            ...data.discoveredProviders,
            {
              name: 'claude_cli',
              providerType: 'claude_cli',
              source: 'token_file',
              enabled: true,
              tier: 'subscription',
              models: [],
              displayLabel: 'Claude CLI',
            },
          ],
        });
      }
    }
  };

  const enabledCount = data.discoveredProviders.filter((p) => p.enabled).length;
  const hasProvider = enabledCount > 0;

  // Hide login buttons for already-detected services
  const hasCopilot = data.discoveredProviders.some((p) => p.name === 'copilot');
  const hasGcloud = data.discoveredProviders.some(
    (p) => p.name === 'gemini' && p.source === 'token_file'
  );
  const hasClaude = data.discoveredProviders.some((p) => p.name === 'claude_cli');

  return (
    <div className="space-y-6">
      <h2 className="text-xl font-bold">Provider 自動偵測</h2>

      {scanStatus === 'scanning' && (
        <div className="text-sm text-gray-400 animate-pulse">掃描中...</div>
      )}

      {scanStatus === 'error' && (
        <div className="text-sm text-red-400">掃描失敗: {error}</div>
      )}

      {/* Upper: Auto-detected results */}
      {data.discoveredProviders.length > 0 && (
        <div className="space-y-2">
          <h3 className="text-sm font-medium text-gray-300">已偵測到</h3>
          {data.discoveredProviders.map((p) => (
            <label
              key={`${p.name}-${p.source}`}
              className="flex items-center justify-between p-3 rounded border border-gray-700 hover:border-gray-500 cursor-pointer"
            >
              <div>
                <span className="text-green-400 mr-2">✅</span>
                <span className="font-medium">{p.displayLabel}</span>
                {p.models.length > 0 && (
                  <span className="text-sm text-gray-400 ml-2">
                    — {p.models.length} 個模型
                  </span>
                )}
              </div>
              <input
                type="checkbox"
                checked={p.enabled}
                onChange={() => toggleProvider(p.name)}
                className="w-5 h-5"
              />
            </label>
          ))}
        </div>
      )}

      {/* Middle: One-click login */}
      {(!hasCopilot || !hasGcloud || !hasClaude) && (
        <div className="space-y-2">
          <h3 className="text-sm font-medium text-gray-300">訂閱服務登入</h3>
          {!hasCopilot && (
            <button
              onClick={handleCopilotLogin}
              className="w-full text-left p-3 rounded border border-gray-700 hover:border-blue-500"
            >
              🐙 GitHub Copilot 登入
              <span className="text-xs text-gray-400 ml-2">需 Copilot 訂閱</span>
            </button>
          )}
          {!hasGcloud && (
            <button
              onClick={handleGcloudLogin}
              className="w-full text-left p-3 rounded border border-gray-700 hover:border-blue-500"
            >
              🔷 Google Gemini 登入
              <span className="text-xs text-gray-400 ml-2">需 gcloud CLI</span>
            </button>
          )}
          {!hasClaude && (
            <button
              onClick={handleClaudeSync}
              className="w-full text-left p-3 rounded border border-gray-700 hover:border-blue-500"
            >
              🟣 Claude CLI 同步
              <span className="text-xs text-gray-400 ml-2">讀取本地 token</span>
            </button>
          )}
        </div>
      )}

      {/* Bottom: Navigation */}
      <div className="text-sm text-gray-400">
        已啟用: {enabledCount} 個 provider {hasProvider && '✓'}
      </div>
      <div className="flex justify-between">
        <button onClick={goBack} className="px-4 py-2 rounded border border-gray-600">
          ← 上一步
        </button>
        <div className="flex gap-2">
          {hasProvider && (
            <button
              onClick={() => goTo(4)}
              className="px-4 py-2 rounded border border-gray-600 text-gray-400"
            >
              跳過手動設定 →→
            </button>
          )}
          <button
            onClick={goNext}
            disabled={!hasProvider}
            className="px-4 py-2 rounded bg-blue-600 disabled:opacity-50"
          >
            手動新增 API Key →
          </button>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Verify it renders**

```bash
cd phantom-mesh-desktop && npx tsc --noEmit 2>&1 | head -20
```

- [ ] **Step 3: Commit**

```bash
git add phantom-mesh-desktop/src/components/onboarding/StepProviderDiscovery.tsx
git commit -m "feat: add StepProviderDiscovery.tsx — auto-detection + one-click login UI"
```

---

## Task 10: StepProviders → StepProviderManual Rename + Expand

**Files:**
- Rename: `phantom-mesh-desktop/src/components/onboarding/StepProviders.tsx` → `StepProviderManual.tsx`

- [ ] **Step 1: Rename file**

```bash
git mv phantom-mesh-desktop/src/components/onboarding/StepProviders.tsx phantom-mesh-desktop/src/components/onboarding/StepProviderManual.tsx
```

- [ ] **Step 2: Update component name, rename data field, add new providers**

Key changes to apply in `StepProviderManual.tsx`:

1. Rename default export to `StepProviderManual`
2. Replace all `data.providers` → `data.manualProviders` throughout the file
3. Replace all `updateData({ providers:` → `updateData({ manualProviders:`
4. Expand the CLOUD_PROVIDERS array:

```typescript
const CLOUD_PROVIDERS = [
  { name: 'openai', label: 'OpenAI', type: 'openai' },
  { name: 'anthropic', label: 'Anthropic', type: 'anthropic' },
  { name: 'gemini', label: 'Gemini', type: 'gemini' },
  { name: 'groq', label: 'Groq', type: 'groq' },
  { name: 'openrouter', label: 'OpenRouter', type: 'openai_compat' },
  { name: 'deepseek', label: 'DeepSeek', type: 'openai_compat' },
  { name: 'mistral', label: 'Mistral', type: 'openai_compat' },
  { name: 'xai', label: 'xAI (Grok)', type: 'openai_compat' },
];
```

5. Add Enterprise Cloud section after the existing grid:

```tsx
{/* Enterprise Cloud */}
<div className="mt-6 space-y-4">
  <h3 className="text-sm font-medium text-gray-300">企業雲端</h3>

  {/* Azure OpenAI */}
  <div className="p-3 rounded border border-gray-700 space-y-2">
    <div className="font-medium">Azure OpenAI</div>
    <input
      type="text"
      placeholder="Endpoint URL (https://xxx.openai.azure.com)"
      value={azureEndpoint}
      onChange={(e) => setAzureEndpoint(e.target.value)}
      className="w-full p-2 rounded bg-gray-800 border border-gray-600"
    />
    <input
      type="password"
      placeholder="API Key"
      value={azureKey}
      onChange={(e) => setAzureKey(e.target.value)}
      className="w-full p-2 rounded bg-gray-800 border border-gray-600"
    />
    <button
      onClick={() => handleValidateAzure()}
      disabled={!azureEndpoint || !azureKey}
      className="px-3 py-1 rounded bg-blue-600 text-sm disabled:opacity-50"
    >
      驗證
    </button>
  </div>

  {/* AWS Bedrock */}
  <div className="p-3 rounded border border-gray-700 space-y-2">
    <div className="font-medium">AWS Bedrock</div>
    <select
      value={bedrockRegion}
      onChange={(e) => setBedrockRegion(e.target.value)}
      className="w-full p-2 rounded bg-gray-800 border border-gray-600"
    >
      <option value="us-east-1">us-east-1</option>
      <option value="us-west-2">us-west-2</option>
      <option value="eu-west-1">eu-west-1</option>
      <option value="ap-northeast-1">ap-northeast-1</option>
    </select>
    <button
      onClick={() => handleValidateBedrock()}
      className="px-3 py-1 rounded bg-blue-600 text-sm"
    >
      檢查 AWS 憑證
    </button>
  </div>
</div>
```

6. Add "已從上一步啟用" badges at the top:

```tsx
{data.discoveredProviders.filter(p => p.enabled).length > 0 && (
  <div className="text-sm text-gray-400 mb-4">
    已從上一步啟用:{' '}
    {data.discoveredProviders
      .filter(p => p.enabled)
      .map(p => p.displayLabel)
      .join(', ')} ✓
  </div>
)}
```

7. Azure validation sends `endpoint|api_key` format:
```typescript
const handleValidateAzure = async () => {
  const result = await invoke<ValidationResult>('validate_api_key', {
    provider: 'azure',
    key: `${azureEndpoint}|${azureKey}`,
  });
  if (result.ok) {
    updateData({
      manualProviders: [
        ...data.manualProviders,
        {
          name: 'azure', apiKey: azureKey, providerType: 'azure_openai',
          validated: true, models: result.models,
          endpoint: azureEndpoint,
        },
      ],
    });
  }
};
```

8. Bedrock validation:
```typescript
const handleValidateBedrock = async () => {
  const result = await invoke<ValidationResult>('validate_api_key', {
    provider: 'bedrock',
    key: '', // Not used — checks local AWS credentials
  });
  if (result.ok) {
    updateData({
      manualProviders: [
        ...data.manualProviders,
        {
          name: 'bedrock', apiKey: '', providerType: 'bedrock',
          validated: true, models: result.models,
          region: bedrockRegion,
        },
      ],
    });
  }
};
```

- [ ] **Step 3: Commit**

```bash
git add phantom-mesh-desktop/src/components/onboarding/StepProviderManual.tsx
git commit -m "feat: rename StepProviders → StepProviderManual, add DeepSeek/Mistral/xAI/Azure/Bedrock"
```

---

## Task 11: OnboardingWizard + StepComplete — Wire 6-Step Flow

**Files:**
- Modify: `phantom-mesh-desktop/src/components/onboarding/OnboardingWizard.tsx`
- Modify: `phantom-mesh-desktop/src/components/onboarding/StepComplete.tsx`

- [ ] **Step 1: Update OnboardingWizard.tsx**

Replace import and step rendering. Key changes:

```tsx
// Replace import
// OLD: import StepProviders from './StepProviders';
// NEW:
import StepProviderDiscovery from './StepProviderDiscovery';
import StepProviderManual from './StepProviderManual';
```

Update `useWizardState` destructuring to include `goTo`:
```tsx
const { currentStep, data, goNext, goBack, goTo, updateData, completeWizard } = useWizardState();
```

Update progress dots to 6 steps:
```tsx
{[0, 1, 2, 3, 4, 5].map((i) => (
  <div
    key={i}
    className={`w-3 h-3 rounded-full ${
      i === currentStep
        ? 'bg-blue-500'
        : i < currentStep
        ? 'bg-blue-500 opacity-50'
        : 'border border-gray-500'
    }`}
  />
))}
```

Update step rendering (switch/conditional):
```tsx
{currentStep === 0 && <StepWelcome data={data} updateData={updateData} goNext={goNext} />}
{currentStep === 1 && <StepSecurity data={data} updateData={updateData} goNext={goNext} goBack={goBack} />}
{currentStep === 2 && <StepProviderDiscovery data={data} updateData={updateData} goNext={goNext} goBack={goBack} goTo={goTo} />}
{currentStep === 3 && <StepProviderManual data={data} updateData={updateData} goNext={goNext} goBack={goBack} />}
{currentStep === 4 && <StepNetwork data={data} updateData={updateData} goNext={goNext} goBack={goBack} />}
{currentStep === 5 && <StepComplete data={data} onComplete={onComplete} />}
```

- [ ] **Step 2: Update StepComplete.tsx**

1. Change all `data.providers` → `data.manualProviders`
2. Update the summary table to show discovered + manual providers:

```tsx
{/* Providers summary */}
<div>
  <span className="text-gray-400">Providers:</span>
  <span className="ml-2">
    {data.ollamaEnabled && 'Ollama, '}
    {data.discoveredProviders
      .filter(p => p.enabled && p.name !== 'ollama')
      .map(p => p.displayLabel)
      .join(', ')}
    {data.discoveredProviders.filter(p => p.enabled && p.name !== 'ollama').length > 0 &&
      data.manualProviders.filter(p => p.validated).length > 0 && ', '}
    {data.manualProviders
      .filter(p => p.validated)
      .map(p => p.name.charAt(0).toUpperCase() + p.name.slice(1))
      .join(', ')}
  </span>
</div>
```

3. Replace the `write_config` invoke call with the spec's version:

```tsx
const firstProvider = data.discoveredProviders.find(p => p.enabled)
  ?? data.manualProviders.find(p => p.validated);

await invoke('write_config', {
  data: {
    port: data.hardwareScan?.available_port ?? 7878,
    discovered_providers: data.discoveredProviders
      .filter(p => p.enabled && p.source !== 'local_probe')
      .map(p => ({
        name: p.name,
        provider_type: p.providerType,
        tier: p.tier,
        token_source: p.source === 'token_file' || p.source === 'cli_tool' ? 'auto' : 'env',
        base_url: null,
        env_key_name: p.source === 'env_var' ? `${p.name.toUpperCase()}_API_KEY` : null,
      })),
    manual_providers: data.manualProviders
      .filter(p => p.validated)
      .map(p => ({
        name: p.name,
        provider_type: p.providerType,
        api_key: p.apiKey,
        tier: 'payg',
        base_url: p.baseUrl ?? null,
        endpoint: p.endpoint ?? null,
        region: p.region ?? null,
      })),
    ollama_endpoint: data.ollamaEnabled ? data.ollamaEndpoint : null,
    default_agent_provider: data.ollamaEnabled ? 'ollama'
      : firstProvider?.name ?? '',
    default_agent_model: '',
    auth_key: data.qrPayload?.auth_key ?? crypto.randomUUID(),
    telegram_token: data.telegramToken || null,
    identity_provider: data.identity?.provider ?? null,
    identity_sub: data.identity?.sub ?? null,
    identity_email: data.identity?.email ?? null,
    is_primary: true,
  }
});
```

- [ ] **Step 3: Verify full TypeScript compilation**

```bash
cd phantom-mesh-desktop && npx tsc --noEmit 2>&1 | tail -20
```
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add phantom-mesh-desktop/src/components/onboarding/OnboardingWizard.tsx phantom-mesh-desktop/src/components/onboarding/StepComplete.tsx
git commit -m "feat: wire 6-step wizard flow with discovery + manual provider steps"
```

---

## Task 12: Integration Verification

**Files:** None new — verification only.

- [ ] **Step 1: Run all Rust tests**

```bash
cd phantom-mesh && CARGO_TARGET_DIR=target_onboarding cargo test --lib 2>&1 | tail -30
```
Expected: all existing + new tests pass.

- [ ] **Step 2: Check Rust compilation with bedrock feature**

```bash
cd phantom-mesh && CARGO_TARGET_DIR=target_onboarding cargo check --features bedrock 2>&1 | tail -10
```

- [ ] **Step 3: Check Tauri desktop compilation**

```bash
cd phantom-mesh-desktop/src-tauri && CARGO_TARGET_DIR=target_onboarding cargo check 2>&1 | tail -10
```

- [ ] **Step 4: Check TypeScript compilation**

```bash
cd phantom-mesh-desktop && npx tsc --noEmit 2>&1 | tail -10
```

- [ ] **Step 5: Commit any fixes**

Only if needed. Then tag completion:

```bash
git log --oneline -15
```
