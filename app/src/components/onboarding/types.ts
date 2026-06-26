// types.ts — Shared types for onboarding wizard

export interface GpuInfo {
  name: string;
  dedicated_mb: number;
  shared_mb: number;
}

export interface NpuInfo {
  name: string;
  tops: number;
  device_id: string;
}

export interface HardwareScanResult {
  gpu: string;
  vram_mb: number;
  gpus: GpuInfo[];
  npus: NpuInfo[];
  ram_mb: number;
  ollama_status: 'online' | 'offline';
  ollama_models: string[];
  daemon_binary_path: string | null;
  available_port: number;
}

export interface OllamaProbeResult {
  ok: boolean;
  models: string[];
  latency_ms: number;
  speed_tier: 'Fast' | 'Medium' | 'Slow' | 'Unknown';
}

export interface ValidationResult {
  ok: boolean;
  models: string[];
  error: string | null;
}

export interface DaemonStatus {
  ok: boolean;
  pid: number | null;
  port: number;
}

export interface QrPayload {
  type: string;
  version: number;
  hub_url: string;
  auth_key: string;
  node_id: string;
}

export interface ProviderConfig {
  name: string;
  apiKey: string;
  providerType: string;
  validated: boolean;
  models: string[];
  baseUrl?: string;    // For Azure
  endpoint?: string;   // For Azure
  region?: string;     // For Bedrock
}

export interface DiscoveredProvider {
  name: string;
  providerType: string;
  source: 'token_file' | 'env_var' | 'local_probe' | 'cli_tool';
  enabled: boolean;
  tier: 'local' | 'free' | 'subscription' | 'payg';
  models: string[];
  displayLabel: string;
  /** localhost base URL for detected local servers (Ollama/LM Studio/Lemonade). */
  baseUrl?: string;
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

export interface CodexCliStatus {
  found: boolean;
  /** true = ChatGPT subscription OAuth (→ codex_oauth); false = OPENAI_API_KEY (→ openai). */
  is_oauth: boolean;
}

export interface UserIdentity {
  provider: 'google' | 'apple' | 'local';
  sub: string;
  email: string;
  display_name: string;
  avatar_url: string | null;
  id_token: string | null;
}

export interface OnboardingData {
  hardwareScan: HardwareScanResult | null;
  identity: UserIdentity | null;  // OAuth identity (in-memory during wizard)
  vaultPin: string;               // 6-digit PIN, in-memory only, never persisted
  discoveredProviders: DiscoveredProvider[];  // Auto-detected credentials
  manualProviders: ProviderConfig[];          // User-entered API keys
  clusterEnabled: boolean;
  clusterNodes: string[];
  telegramToken: string;
  qrPayload: QrPayload | null;
  ollamaEndpoint: string;
  ollamaEnabled: boolean;
}

// What gets persisted to localStorage for crash recovery
// NOTE: vaultPin and provider apiKeys are EXCLUDED
export interface PersistedWizardState {
  currentStep: number;
  ollamaEnabled: boolean;
  ollamaEndpoint: string;
  providerNames: string[];        // Just names, not keys
  discoveredProviderNames?: string[];  // Auto-detected provider names
  clusterEnabled: boolean;
  clusterNodes: string[];
  telegramConfigured: boolean;
  identityEmail?: string;
  identityProvider?: string;
}

export type WizardStep = 0 | 1 | 2 | 3 | 4 | 5;

export const WIZARD_STORAGE_KEY = 'phantom_mesh_onboarding_state';
export const ONBOARDED_KEY = 'phantom_mesh_onboarded';
/** Local cosmetic identity profile (display name / email / avatar). NOT the
 *  on-device ed25519 identity key — that lives in the system keychain. */
export const IDENTITY_KEY = 'phantom_mesh_identity';

/** Clear the local onboarding/session footprint so the next launch restarts
 *  the D1–D5 login-first flow from `fresh_install`. Wipes the cosmetic identity
 *  profile, the `onboarded` flag, and the persisted SPEC-28 FSM snapshot +
 *  context. Lives here (not in a screen component) so both App.tsx and
 *  StartupCheck can reset without importing any particular onboarding screen.
 *
 *  Note: the broker OAuth token (system keychain via `broker_login_logout`) and
 *  the ed25519 identity key are intentionally NOT wiped here — those are a
 *  separate "switch account" / "factory reset" concern. */
export function clearSession(): void {
  try {
    localStorage.removeItem(IDENTITY_KEY);
    localStorage.removeItem(ONBOARDED_KEY);
    // FSM snapshot + context keys (mirrors lib/onboardingFsm.ts constants).
    localStorage.removeItem('phantom_mesh_onboarding_snapshot');
    localStorage.removeItem('phantom_mesh_onboarding_context');
  } catch {
    /* private mode / quota — ignore */
  }
}
