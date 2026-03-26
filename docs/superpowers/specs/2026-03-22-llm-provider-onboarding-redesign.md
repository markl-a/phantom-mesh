# LLM Provider Onboarding Redesign

## Goal

Overhaul the Desktop onboarding provider setup to support auto-detection of existing credentials, OAuth-based subscription services, and a broader set of LLM providers — split into a two-step flow (Discovery + Manual) with unified authentication profiles and OpenClaw-style failover classification.

## Architecture

### Two-Layer Onboarding

| Display | Index | Name | Purpose |
|---------|-------|------|---------|
| Step 3 | `2` | **StepProviderDiscovery** (new) | Auto-detect local tokens, env vars, running services; one-click OAuth login for subscription services |
| Step 4 | `3` | **StepProviderManual** (renamed from StepProviders) | Manual API key input for pay-as-you-go providers, enterprise cloud (Azure, Bedrock) |

**Convention:** "Step N" = display label (1-based). Code indices are 0-based: `Hardware(0) → Security(1) → ProviderDiscovery(2) → ProviderManual(3) → Network(4) → Complete(5)`.

Desktop wizard changes from 5 steps to 6: `Hardware → Security → ProviderDiscovery → ProviderManual → Cluster → Complete`

Step count update required in:
- `types.ts`: `WizardStep` type → `0 | 1 | 2 | 3 | 4 | 5`
- `useWizardState.ts`: `goNext` max → `5`, `goBack` min unchanged, add `goTo(step: WizardStep)` for skip navigation
- `OnboardingWizard.tsx`: progress dots → `[0, 1, 2, 3, 4, 5]`, step labels updated

**Note:** The existing `StepNetwork.tsx` handles cluster config (referred to as "StepCluster" conceptually in this spec). It is not renamed — remains `StepNetwork.tsx` at step index 4.

### Unified Authentication Model

All provider credentials are represented by three types (inspired by OpenClaw's `AuthProfileCredential`):

```rust
// src/providers/auth_profile.rs

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

pub struct AuthProfile {
    pub provider_name: String,
    pub credential: CredentialType,
    pub tier: ProviderTier,
    pub usage_stats: ProfileUsageStats,
}

pub struct ProfileUsageStats {
    pub last_used: Option<DateTime<Utc>>,
    pub success_count: u64,
    pub failure_count: u64,
    pub last_error: Option<String>,
    pub cooldown_until: Option<DateTime<Utc>>,
}
```

`ProfileUsageStats` uses `DateTime<Utc>` (not `Instant`) so it can be serialized and persisted across process restarts, consistent with `subscription_pacer.rs`.

### Failover Error Classification

New `FailoverReason` enum lives in `auth_profile.rs` alongside `AuthProfile`. It is a **higher-level abstraction** that maps to the existing `ErrorClass` in `reliable.rs`:

```rust
// src/providers/auth_profile.rs
use super::reliable::ErrorClass;

pub enum FailoverReason {
    Auth,            // Authentication failed → immediate switch
    Billing,         // Quota exhausted → immediate switch
    RateLimit,       // Rate limited → backoff then switch
    Overload,        // Service overloaded → backoff then switch
    ContextOverflow, // Context too long → switch to provider with larger context
    Timeout,         // Request timeout → switch
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

The existing `reliable.rs` retry logic (`try_chain()`, `classify_error()`) is unchanged. `FailoverReason` adds richer context for profile rotation decisions. The existing `ErrorClass` backoff parameters in `reliable.rs` are preserved; `FailoverReason`-specific backoff (250ms initial, 1500ms max, factor 2, jitter 0.2) applies only to the profile rotation layer in `auth_profile.rs`, not the retry layer.

## Provider Landscape

### Full Provider Table

| Provider | Tier | Auth Method | Backend Implementation | Status |
|----------|------|-------------|----------------------|--------|
| Ollama | Local | Endpoint probe | `ollama.rs` | Existing |
| GitHub Copilot | Subscription | Token file (`hosts.json`) | **New** `copilot.rs` | **New** |
| Gemini (free) | FreeApi | API Key / gcloud ADC | `gemini.rs` | Modify (add gcloud detection) |
| Groq (free) | FreeApi | API Key | `groq.rs` | Existing |
| Codex (ChatGPT Plus) | Subscription | OAuth auto-refresh | `codex.rs` | Existing |
| ChatGPT WS | Subscription | Codex token | `chatgpt_ws.rs` | Existing |
| OpenCode | Subscription | CLI subprocess | `opencode_backend.rs` | Existing |
| Claude CLI | Subscription | Token file | **New** `claude_cli.rs` | **New** |
| OpenAI | PayAsYouGo | API Key | `openai.rs` | Existing |
| Anthropic | PayAsYouGo | API Key | `anthropic.rs` | Existing |
| Gemini (paid) | PayAsYouGo | API Key | `gemini.rs` | Existing |
| DeepSeek | PayAsYouGo | API Key | `openai_compat` (no new file) | **New** |
| Mistral | PayAsYouGo | API Key | `openai_compat` (no new file) | **New** |
| xAI (Grok) | PayAsYouGo | API Key | `openai_compat` (no new file) | **New** |
| OpenRouter | PayAsYouGo | API Key | `openai_compat` | Existing |
| Azure OpenAI | PayAsYouGo | API Key + Endpoint | **New** `azure_openai.rs` | **New** |
| AWS Bedrock | PayAsYouGo | IAM credentials | **New** `bedrock.rs` | **New** |

**Copilot tier note:** GitHub Copilot is classified as `Subscription` (not `FreeApi`) because it requires a GitHub Copilot subscription (Individual/Business/Enterprise). Copilot Free tier has severe rate limits (~50 messages/day). A `SubscriptionPacer` is attached to manage daily quota. If Copilot Free (no subscription) is detected, the pacer uses conservative daily limits.

### OpenAI-Compatible Providers

DeepSeek, Mistral, xAI reuse `OpenAiCompatProvider` with different base URLs:

| Provider | Base URL |
|----------|----------|
| DeepSeek | `https://api.deepseek.com/v1` |
| Mistral | `https://api.mistral.ai/v1` |
| xAI (Grok) | `https://api.x.ai/v1` |

No separate `.rs` files needed. Configured via `agents.toml`:

```toml
[providers.deepseek]
type = "openai_compat"
base_url = "https://api.deepseek.com/v1"
api_key_env = "DEEPSEEK_API_KEY"
tier = "payg"
```

### Azure OpenAI

Gets its own file `azure_openai.rs` (not modifying `openai_compat.rs`) because the URL construction and auth header are fundamentally different:

- URL pattern: `https://{endpoint}/openai/deployments/{model}/chat/completions?api-version={version}` — model name is in the URL path, not the request body
- Auth header: `api-key: {key}` instead of `Authorization: Bearer {key}`
- Requires `api_version` query parameter

`AzureOpenAiProvider` implements the `Provider` trait directly, internally using `reqwest::Client` (same as `openai_compat.rs` but with Azure-specific URL/header logic).

```toml
[providers.azure]
type = "azure_openai"
endpoint = "https://mydeployment.openai.azure.com"
api_key_env = "AZURE_OPENAI_API_KEY"
api_version = "2024-02-01"
tier = "payg"
```

## Credential Scanner

New file `src/providers/credential_scanner.rs`. Scans all known credential sources in parallel at onboarding time.

### Scan Sources

| Source | Location | Provider | Detection Method |
|--------|----------|----------|-----------------|
| Ollama | `localhost:11434/api/tags` | ollama | HTTP probe |
| Codex CLI | `~/.codex/auth.json`, `~/.codex-cli/auth.json` | codex | File read + JSON parse |
| GitHub Copilot | `~/.config/github-copilot/hosts.json` or `apps.json` (Linux/macOS), `%LOCALAPPDATA%/github-copilot/` (Windows) | copilot | File read + JSON parse |
| Claude CLI | Best-effort: scan `~/.claude/.credentials.json`, `~/.claude/credentials.json`, `~/.claude/auth.json` (Unix), `%APPDATA%\claude\` equivalents (Windows via `dirs::config_dir()`) | claude_cli | File read + JSON parse |
| gcloud ADC | `~/.config/gcloud/application_default_credentials.json` | gemini | File read + JSON parse |
| OpenCode | `which opencode` or `where opencode` | opencode | Binary existence check |
| Environment vars | `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`, `GROQ_API_KEY`, `OPENROUTER_API_KEY`, `DEEPSEEK_API_KEY`, `MISTRAL_API_KEY`, `XAI_API_KEY` | Respective providers | `std::env::var()` |

**Provider naming convention:** `DiscoveredCredential.provider_name` uses the env var prefix in lowercase (e.g., `"xai"` not `"grok"`, `"openrouter"` not `"open_router"`). This ensures `name.to_uppercase() + "_API_KEY"` produces the correct env var name. Display labels (e.g., "xAI (Grok)") are in `display_label`.
| AWS credentials | `~/.aws/credentials` + `AWS_ACCESS_KEY_ID` env var | bedrock | File read / env var |

### Error Handling

Each scan is independent. Failures (corrupted files, invalid JSON, unreachable services) are logged at `warn` level and the scan result is omitted from the returned list. The UI does not show failed scans. No scan failure blocks other scans.

### Data Structures

```rust
pub enum CredentialSource {
    TokenFile(PathBuf),
    EnvVar(String),
    LocalProbe(String),  // URL probed
    CliTool(String),     // Binary name
}

/// Internal result from credential scanning (contains secrets — never sent to frontend)
pub struct DiscoveredCredential {
    pub provider_name: String,
    pub source: CredentialSource,
    pub credential: CredentialType,
    pub tier: ProviderTier,
    pub display_label: String,        // e.g. "Codex (ChatGPT Plus)"
    pub available_models: Vec<String>, // For Ollama, Copilot
}

/// Frontend-safe view of a discovered credential (no secrets)
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredProviderInfo {
    pub name: String,                // Matches TS DiscoveredProvider.name
    pub provider_type: String,       // e.g. "copilot", "codex", "openai_compat"
    pub source: String,              // "token_file" | "env_var" | "local_probe" | "cli_tool"
    pub tier: String,                // "local" | "free" | "subscription" | "payg"
    pub display_label: String,
    pub models: Vec<String>,         // Matches TS DiscoveredProvider.models
}

impl DiscoveredCredential {
    /// Convert to frontend-safe view, stripping credentials
    pub fn to_frontend_info(&self) -> DiscoveredProviderInfo { ... }
}

pub async fn scan_all() -> Vec<DiscoveredCredential> {
    // Each scan returns its own Vec, then merged (no shared mutable state)
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
```

**Tauri command converts `DiscoveredCredential` → `DiscoveredProviderInfo` before sending to frontend.** Secrets (tokens, API keys) stay in the Rust backend and are never exposed to the webview. The `enabled` field in TypeScript's `DiscoveredProvider` is frontend-only (defaults to `true` on scan results) and does not exist in the Rust struct.

### Claude CLI Token — Best-Effort

Claude CLI token format is not stable across versions. The scanner tries multiple known paths (`~/.claude/.credentials.json`, `~/.claude/credentials.json`, `~/.claude/auth.json`) and multiple JSON schemas. If none match, the scan silently returns empty. Implementation should add a `// TODO: update if Claude CLI auth format changes` comment and log the detected path/schema for debugging.

## GitHub Copilot Provider

### Token Lifecycle

1. Read `oauth_token` from `hosts.json` / `apps.json`
2. Exchange for short-lived API token via `POST https://api.github.com/copilot_internal/v2/token` with `Authorization: token gho_xxxx`
3. Response: `{ "token": "tid=xxx;...", "expires_at": 1234567890 }`
4. Use API token to call `https://api.githubcopilot.com/chat/completions` (OpenAI-compat)
5. Auto-refresh when token expires (similar to `CodexTokenManager`)

### Architecture

```rust
pub struct CopilotTokenManager {
    oauth_token: Mutex<Option<String>>,      // From hosts.json
    api_token: Mutex<Option<CopilotApiToken>>, // Short-lived
    token_file_paths: Vec<PathBuf>,
    client: Client,
}

pub struct CopilotApiToken {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}
```

`CopilotAwareProvider` wraps `OpenAiCompatProvider` with base URL `https://api.githubcopilot.com`, injecting the auto-refreshed API token.

### Models

GPT-4o, GPT-4o-mini, Claude 3.5 Sonnet, Claude 4 Sonnet, Gemini 2.0 Flash, o3-mini, o4-mini, and more. Full list fetched via `/models` endpoint at runtime.

## Claude CLI Token Provider

### Token Source

Best-effort scan of multiple paths:
```
~/.claude/.credentials.json
~/.claude/credentials.json
~/.claude/auth.json
```

Format to be confirmed during implementation. If no valid auth file is found, the provider is not registered. On Windows, also check `%APPDATA%\claude\` equivalents — use `dirs::config_dir()` for platform-appropriate path resolution.

### Architecture

`ClaudeCliTokenManager` reads the local auth file, extracts session token, wraps existing `anthropic.rs` provider. Same pattern as `CodexTokenManager`.

```rust
pub struct ClaudeCliTokenManager {
    credential: Mutex<Option<ClaudeCliCredential>>,
    auth_file_paths: Vec<PathBuf>,
}
```

## AWS Bedrock Provider

### Dependencies

`aws-sdk-bedrockruntime` crate, added behind a cargo feature flag `bedrock`:

```toml
[features]
bedrock = ["dep:aws-sdk-bedrockruntime", "dep:aws-config"]

[dependencies]
aws-sdk-bedrockruntime = { version = "1", optional = true }
aws-config = { version = "1", optional = true }
```

### Authentication

Uses standard AWS credential chain: env vars → `~/.aws/credentials` → IAM role. No API key stored in Phantom Mesh.

### API Format

Bedrock uses its own `ConverseStream` API, not OpenAI-compat. Model names follow `provider.model-name-version` format (e.g., `anthropic.claude-3-sonnet-20240229-v1:0`).

## Desktop Onboarding UI

### StepProviderDiscovery (New, Step 3)

Three sections:

**Upper: Auto-Detection Results**

Shows each discovered credential with an enable/disable toggle:
- `✅ Ollama (本地) — 3 個模型 [啟用 ✓]`
- `✅ Codex (ChatGPT Plus) — 已登入 [啟用 ✓]`
- `✅ OPENAI_API_KEY — 環境變數 [啟用 ✓]`
- `⚠️ gcloud — 需要登入 [登入 →]`

Items that aren't detected don't appear.

**Middle: One-Click Login**

Buttons for subscription services that weren't auto-detected:
- `[🐙 GitHub Copilot 登入] 需 Copilot 訂閱` — Reads `hosts.json`; if not found, shows install guide
- `[🔷 Google Gemini 登入] 需 gcloud CLI` — Reads gcloud ADC; if not found, prompts `gcloud auth application-default login`
- `[🟣 Claude CLI 同步] 讀取本地 token` — Reads `~/.claude/` auth file

Already-detected services are hidden from this section (moved to upper section).

**Bottom: Navigation**

```
已啟用: N 個 provider ✓
[← 上一步]    [跳過手動設定 →→]    [手動新增 API Key →]
```

- At least 1 provider must be enabled to proceed
- "跳過手動設定" jumps from StepProviderDiscovery (index 2) directly to StepNetwork (index 4), skipping StepProviderManual (index 3). Requires a `goTo(step: WizardStep)` function in `useWizardState.ts` (since `goNext()` only increments by 1)
- "手動新增 API Key" goes to StepProviderManual (index 3, normal `goNext()`)

### StepProviderManual (Renamed, Step 4)

Extends current `StepProviders.tsx` grid with more providers:

**Cloud API section (checkbox grid):**

OpenAI, Anthropic, Gemini, Groq, OpenRouter, DeepSeek, Mistral, xAI (Grok)

Each with API key input + validate button (same UX as current).

**Enterprise Cloud section:**

Azure OpenAI: endpoint URL + API key input
AWS Bedrock: region selector (uses local AWS credentials)

**Already-enabled indicators:**

Providers enabled in StepProviderDiscovery shown as read-only badges: `已從上一步啟用: Ollama ✓, Codex ✓`

Environment-variable-detected API keys auto-check their provider (no re-entry needed).

### Tauri Backend Commands

**Note:** `credential_scanner.rs` lives in `phantom-mesh` (library crate). `phantom-mesh-desktop/src-tauri/Cargo.toml` already depends on `phantom-mesh` as a library — Tauri commands call into it. The scanner uses `dirs` crate / `std::env::var()` for platform-aware path resolution (e.g., `%LOCALAPPDATA%` on Windows).

```rust
#[tauri::command]
async fn scan_credentials() -> Result<Vec<DiscoveredProviderInfo>, String>
// Calls credential_scanner::scan_all(), converts to frontend-safe DiscoveredProviderInfo

#[tauri::command]
async fn read_copilot_token() -> Result<CopilotTokenStatus, String>
// Reads hosts.json/apps.json, returns { found: bool, user: Option<String> }
// Does NOT return the actual token to the frontend

#[tauri::command]
async fn read_gcloud_adc() -> Result<GcloudAdcStatus, String>
// Reads ADC file, returns { found: bool, project: Option<String> }

#[tauri::command]
async fn read_claude_cli_token() -> Result<ClaudeCliStatus, String>
// Scans Claude CLI auth files, returns { found: bool }
```

**Response types (Rust + TypeScript):**

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

```typescript
interface CopilotTokenStatus { found: boolean; user: string | null; }
interface GcloudAdcStatus { found: boolean; project: string | null; }
interface ClaudeCliStatus { found: boolean; }
```

### OnboardingData Changes

```typescript
interface OnboardingData {
  hardwareScan: HardwareScanResult | null;
  identity: UserIdentity | null;
  vaultPin: string;
  discoveredProviders: DiscoveredProvider[];  // NEW: from auto-detection
  manualProviders: ProviderConfig[];          // RENAMED: was providers
  ollamaEnabled: boolean;                    // KEPT for backward compat, takes precedence
  ollamaEndpoint: string;                    // KEPT for backward compat
  clusterEnabled: boolean;
  clusterNodes: string[];
  telegramToken: string;
  qrPayload: QrPayload | null;
}

interface DiscoveredProvider {
  name: string;
  providerType: string;
  source: 'token_file' | 'env_var' | 'local_probe' | 'cli_tool';
  enabled: boolean;
  tier: 'local' | 'free' | 'subscription' | 'payg';
  models: string[];
  displayLabel: string;
}
```

`ollamaEnabled` and `ollamaEndpoint` are kept for backward compatibility. When Ollama appears in both `discoveredProviders` and the legacy fields, `ollamaEnabled` takes precedence. At `write_config` time, Ollama from `discoveredProviders` is excluded from the providers TOML — it's handled by the existing `ollama_endpoint` field.

### INITIAL_DATA Shape

```typescript
const INITIAL_DATA: OnboardingData = {
  hardwareScan: null,
  identity: null,
  vaultPin: '',
  discoveredProviders: [],
  manualProviders: [],      // Was: providers
  ollamaEnabled: false,
  ollamaEndpoint: 'http://localhost:11434',
  clusterEnabled: false,
  clusterNodes: [],
  telegramToken: '',
  qrPayload: null,
};
```

### PersistedWizardState Changes

```typescript
interface PersistedWizardState {
  // ... existing fields (including providerNames for manual providers — kept as-is)
  discoveredProviderNames?: string[];  // NEW: only names, no secrets
  identityEmail?: string;
  identityProvider?: string;
}
```

Crash recovery: if wizard crashes after StepProviderDiscovery, the discovered provider names are persisted. On recovery, `scan_credentials` is re-invoked but previously-enabled providers are pre-checked.

### StepComplete write_config Call

```typescript
// StepComplete.tsx
const firstProvider = data.discoveredProviders.find(p => p.enabled)
  ?? data.manualProviders.find(p => p.validated);

await invoke('write_config', {
  data: {
    port: data.hardwareScan?.available_port ?? 7878,
    discovered_providers: data.discoveredProviders
      .filter(p => p.enabled && p.source !== 'local_probe')  // Ollama excluded — handled by ollama_endpoint
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
    default_agent_model: '',  // Derived by daemon from first available model
    auth_key: data.qrPayload?.auth_key ?? crypto.randomUUID(),
    telegram_token: data.telegramToken || null,
    identity_provider: data.identity?.provider ?? null,
    identity_sub: data.identity?.sub ?? null,
    identity_email: data.identity?.email ?? null,
    is_primary: true,  // First device is primary by default
  }
});
```

## agents.toml Structure

Each provider entry now includes `tier` and optionally `token_source`:

```toml
[providers.copilot]
type = "copilot"
tier = "subscription"
token_source = "auto"

[providers.codex]
type = "codex"
tier = "subscription"
token_source = "auto"

[providers.deepseek]
type = "openai_compat"
base_url = "https://api.deepseek.com/v1"
api_key_env = "DEEPSEEK_API_KEY"
tier = "payg"

[providers.azure]
type = "azure_openai"
endpoint = "https://mydeployment.openai.azure.com"
api_key_env = "AZURE_OPENAI_API_KEY"
api_version = "2024-02-01"
tier = "payg"

[providers.bedrock]
type = "bedrock"
region = "us-east-1"
tier = "payg"
```

### tier Values & Serde Mapping

The TOML `tier` field uses lowercase shorthand. `ProviderTier` enum needs custom serde mapping:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProviderTier {
    #[serde(rename = "local")]
    Local,
    #[serde(rename = "free", alias = "FreeApi")]
    FreeApi,
    #[serde(rename = "subscription")]
    Subscription,
    #[serde(rename = "payg", alias = "PayAsYouGo")]
    PayAsYouGo,
}
```

**Migration note:** Existing `agents.toml` files from pre-redesign installations do not use `tier` fields — tiers are determined by provider type in code. The `alias` attributes ensure backward compatibility if any serialized state uses the old variant names.

### token_source Values

| Value | Meaning | Used By |
|-------|---------|---------|
| `"auto"` | Read token from provider's standard file location at runtime | copilot, codex, claude_cli, gcloud |
| `"env"` | Read API key from environment variable specified in `api_key_env` | openai, anthropic, etc. |

Daemon startup uses `token_source` to decide which token manager to instantiate:
- `"auto"` → `CopilotTokenManager::new()`, `CodexTokenManager::new()`, etc.
- `"env"` → read `api_key_env` from environment, create `OpenAiCompatProvider`

### write_config Changes

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
    pub token_source: String,        // "auto" | "env"
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
    pub endpoint: Option<String>,    // Azure
    pub region: Option<String>,      // Bedrock
}
```

```typescript
// TypeScript equivalents for Tauri IPC
interface DiscoveredProviderEntry {
  name: string;
  provider_type: string;
  tier: string;
  token_source: string;
  base_url: string | null;
  env_key_name: string | null;
}

interface ManualProviderEntry {
  name: string;
  provider_type: string;
  api_key: string;
  tier: string;
  base_url: string | null;
  endpoint: string | null;
  region: string | null;
}
```

### write_config Logic

The updated `write_config` function handles both provider types:

1. **Discovered providers (`token_source = "auto"`)**: Write `[providers.{name}]` with `type`, `tier`, `token_source = "auto"` to TOML. No entry in `.env` — tokens are read at runtime from their original source files (Copilot `hosts.json`, Codex `auth.json`, etc.).
2. **Discovered providers (`token_source = "env"`)**: Write `[providers.{name}]` with `type`, `tier`, `api_key_env = "{env_key_name}"` to TOML. No entry in `.env` — these keys already exist in the environment. If `env_key_name` is set, reference it; if null, skip `api_key_env` field.
3. **Manual providers**: Write `[providers.{name}]` with `type`, `tier = "payg"`, `api_key_env = "{NAME}_API_KEY"` to TOML. Write `{NAME}_API_KEY={key}` to `.env`. Handle Azure-specific fields (`endpoint`, `api_version`) and Bedrock-specific fields (`region`).
4. **Ollama**: Handled separately by existing `ollama_endpoint` field, same as current code.

## TierRouter Integration

Daemon startup reads `agents.toml`, registers all providers into `TierRouter` by their `tier` field. Subscription-tier providers automatically get `SubscriptionPacer` attached. No changes to `tier.rs` or `subscription_pacer.rs`.

```
agents.toml providers
    → Parse tier field
    → TierRouter::new(providers)
    → Combine with LocalProbe speed measurement
    → Subscription tier → attach SubscriptionPacer
    → FailoverReason classification in auth_profile.rs
```

## Mobile Impact

No changes to mobile onboarding. Mobile is a worker that receives provider configuration from Hub via `/sync/pull`. The sync response can be enriched with tier/model info in a future iteration, but it's not part of this spec.

## Testing

### Unit Tests

| Module | Test Focus |
|--------|-----------|
| `credential_scanner.rs` | Mock file reads (existing/missing/corrupted), mock HTTP probes (Ollama online/offline), env var detection |
| `copilot.rs` | Token file parsing (hosts.json/apps.json formats), API token exchange mock, token refresh logic, expiry detection |
| `claude_cli.rs` | Multiple path scanning, various JSON formats, missing file handling |
| `bedrock.rs` | AWS credential detection, model name format conversion |
| `auth_profile.rs` | FailoverReason → ErrorClass mapping, ProfileUsageStats cooldown logic, backoff calculation |
| `azure_openai.rs` | URL construction with model/version, api-key header format |

### Integration Tests

| Area | Test |
|------|------|
| `write_config` | Verify agents.toml output contains correct tier/token_source for discovered + manual providers |
| `scan_credentials` Tauri command | Verify secrets are stripped from response (no tokens/keys in DiscoveredProviderInfo) |

## File Impact Summary

### New Files (phantom-mesh)

| File | Purpose |
|------|---------|
| `src/providers/copilot.rs` | Copilot token management + OpenAI-compat API (~400 lines) |
| `src/providers/claude_cli.rs` | Claude CLI token reader (~150 lines) |
| `src/providers/bedrock.rs` | AWS Bedrock integration (~350 lines) |
| `src/providers/azure_openai.rs` | Azure OpenAI provider (~200 lines) |
| `src/providers/credential_scanner.rs` | Unified credential scanning engine (~300 lines) |
| `src/providers/auth_profile.rs` | Unified auth model + FailoverReason + ProfileUsageStats (~200 lines) |

### New Files (phantom-mesh-desktop)

| File | Purpose |
|------|---------|
| `src/components/onboarding/StepProviderDiscovery.tsx` | Auto-detect + OAuth login UI |

### Modified Files (phantom-mesh)

| File | Change |
|------|--------|
| `src/providers/mod.rs` | Add `pub mod copilot, claude_cli, bedrock, azure_openai, credential_scanner, auth_profile` |
| `Cargo.toml` | Add `aws-sdk-bedrockruntime`, `aws-config` (optional, behind `bedrock` feature) |

### Modified Files (phantom-mesh-desktop)

| File | Change |
|------|--------|
| `src/components/onboarding/StepProviders.tsx` | File renamed to `StepProviderManual.tsx` (`git mv`), component renamed to `StepProviderManual`, expand provider list (+DeepSeek, Mistral, xAI, Azure, Bedrock). `OnboardingWizard.tsx` import updated accordingly. |
| `src/components/onboarding/OnboardingWizard.tsx` | Insert StepProviderDiscovery at step 3, shift subsequent steps, update progress dots to 6 |
| `src/components/onboarding/types.ts` | Add `DiscoveredProvider`, `DiscoveredProviderEntry`, `ManualProviderEntry`, update `OnboardingData`, update `WizardStep` to `0|1|2|3|4|5`, add `discoveredProviderNames` to `PersistedWizardState`, extend `ProviderConfig` with optional `baseUrl?: string`, `endpoint?: string`, `region?: string` for Azure/Bedrock |
| `src/components/onboarding/useWizardState.ts` | Update INITIAL_DATA (see shape above), goNext max to 5, add `goTo(step)` for skip navigation, rename `providers` → `manualProviders` in all references, crash recovery for discoveredProviderNames |
| `src/components/onboarding/StepComplete.tsx` | Merge discovered + manual providers into write_config call (see StepComplete write_config Call section), change `data.providers` → `data.manualProviders` |
| `src-tauri/src/commands/onboarding.rs` | Add `scan_credentials`, `read_copilot_token`, `read_gcloud_adc`, `read_claude_cli_token` commands; update `write_config` for `DiscoveredProviderEntry` + `ManualProviderEntry` |
| `src-tauri/src/commands/mod.rs` | Register new commands |
| `src-tauri/src/main.rs` | Register new commands in handler |

### Not Modified

| File | Reason |
|------|--------|
| Mobile onboarding | Provider info comes from Hub sync |
| `tier.rs` | Unchanged — agents.toml carries tier info |
| `subscription_pacer.rs` | Unchanged — daemon auto-attaches to subscription tier |
| `key_vault.rs` | Unchanged — API key encryption logic same |
| `codex.rs` | Unchanged — already complete |
| `chatgpt_backend.rs` / `chatgpt_ws.rs` | Unchanged |
| `reliable.rs` | Unchanged — ErrorClass and retry logic preserved; FailoverReason lives in auth_profile.rs |
| `openai_compat.rs` | Unchanged — Azure gets its own file |
| `StepNetwork.tsx` | Unchanged — step index shifts but component logic same |
