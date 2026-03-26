# Onboarding Wizard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a first-run onboarding wizard to Desktop (5 steps) and Mobile (4 steps), with QR code bridge and post-onboarding checklist.

**Architecture:** Desktop wizard is a full-screen React component gating `App.tsx`, backed by Tauri commands in `onboarding.rs` for hardware scanning, API validation, config generation, and daemon launch. Mobile wizard replaces the existing `SettingsScreen` fallback with a 4-step swipeable flow. Both persist wizard state for crash recovery.

**Tech Stack:** React 19 + TypeScript (Desktop), React Native + Expo (Mobile), Tauri v2 commands (Rust), `qrcode.react` (QR generation), `expo-camera` (QR scanning, already installed), `sysinfo` crate (hardware detection)

**Spec:** `phantom-mesh/docs/superpowers/specs/2026-03-22-onboarding-wizard-design.md`

---

## File Map

### Desktop — New Files

| File | Responsibility |
|------|---------------|
| `src-tauri/src/commands/onboarding.rs` | 6 Tauri commands: scan_hardware, test_ollama, validate_api_key, write_config, launch_daemon, generate_qr_data |
| `src/components/onboarding/types.ts` | Shared TypeScript types for wizard state |
| `src/components/onboarding/useWizardState.ts` | Hook: step navigation, crash recovery, state persistence |
| `src/components/onboarding/useHardwareScan.ts` | Hook: background hardware detection via Tauri commands |
| `src/components/onboarding/OnboardingWizard.tsx` | Main container: step routing, progress bar, navigation buttons |
| `src/components/onboarding/StepWelcome.tsx` | Step 1: welcome screen + animated scan results |
| `src/components/onboarding/StepSecurity.tsx` | Step 2: vault password input + strength meter |
| `src/components/onboarding/StepProviders.tsx` | Step 3: provider selection + API key validation |
| `src/components/onboarding/StepNetwork.tsx` | Step 4: cluster setup + QR code + Telegram |
| `src/components/onboarding/StepComplete.tsx` | Step 5: summary + daemon launch |
| `src/components/OnboardingChecklist.tsx` | Post-wizard checklist banner for Dashboard |

### Desktop — Modified Files

| File | Change |
|------|--------|
| `src-tauri/src/commands/mod.rs` | Add `pub mod onboarding;` |
| `src-tauri/src/main.rs` | Register 6 onboarding commands in invoke_handler |
| `src-tauri/Cargo.toml` | Add `sysinfo = "0.33"`, `which = "7"` dependencies |
| `src/App.tsx` | Add first-run gate: if not onboarded → show OnboardingWizard |
| `src/pages/Dashboard.tsx` | Add OnboardingChecklist component |
| `package.json` | Add `qrcode.react` dependency |

### Mobile — New Files

| File | Responsibility |
|------|---------------|
| `src/components/onboarding/types.ts` | Mobile onboarding types |
| `src/components/onboarding/OnboardingWizard.tsx` | Main container with swipeable pages |
| `src/components/onboarding/StepWelcome.tsx` | Step 1: welcome |
| `src/components/onboarding/StepConnect.tsx` | Step 2: QR scan + manual input + connection test |
| `src/components/onboarding/StepIdentity.tsx` | Step 3: worker name + agent |
| `src/components/onboarding/StepComplete.tsx` | Step 4: summary + battery warning + launch |
| `src/components/OnboardingChecklist.tsx` | Post-wizard checklist for HomeScreen |

### Mobile — Modified Files

| File | Change |
|------|--------|
| `src/App.tsx` | Replace SettingsScreen fallback with OnboardingWizard |
| `src/screens/HomeScreen.tsx` | Add OnboardingChecklist component |

---

## Milestone Overview

| # | Milestone | Tasks | Deliverable |
|---|-----------|-------|-------------|
| M1 | Desktop Backend | 1-2 | `onboarding.rs` compiled + registered |
| M2 | Desktop Wizard Infra | 3-5 | Types + hooks + container working |
| M3 | Desktop Steps | 6-10 | All 5 wizard steps functional |
| M4 | Desktop Integration | 11-12 | App.tsx gating + Checklist |
| M5 | Mobile Wizard | 13-16 | All 4 mobile steps functional |
| M6 | Mobile Integration | 17-18 | App.tsx update + Checklist |

---

## Task 1: Desktop — Add Dependencies

**Files:**
- Modify: `phantom-mesh-desktop/src-tauri/Cargo.toml`
- Modify: `phantom-mesh-desktop/package.json`

- [ ] **Step 1: Add Rust crates**

In `phantom-mesh-desktop/src-tauri/Cargo.toml`, add to `[dependencies]`:
```toml
sysinfo = "0.33"
which = "7"
```

- [ ] **Step 2: Add qrcode.react npm package**

```bash
cd phantom-mesh-desktop && npm install qrcode.react
```

- [ ] **Step 3: Verify both compile**

```bash
cd phantom-mesh-desktop/src-tauri && CARGO_TARGET_DIR="C:/tmp/desktop-target" cargo check
cd phantom-mesh-desktop && npx tsc --noEmit
```
Expected: zero errors.

- [ ] **Step 4: Commit**

```bash
git add phantom-mesh-desktop/src-tauri/Cargo.toml phantom-mesh-desktop/package.json phantom-mesh-desktop/package-lock.json
git commit -m "chore: add sysinfo + qrcode.react deps for onboarding wizard"
```

---

## Task 2: Desktop — Tauri Onboarding Commands (`onboarding.rs`)

**Files:**
- Create: `phantom-mesh-desktop/src-tauri/src/commands/onboarding.rs`
- Modify: `phantom-mesh-desktop/src-tauri/src/commands/mod.rs`
- Modify: `phantom-mesh-desktop/src-tauri/src/main.rs`

**Reference:** Read existing command patterns:
- `commands/health.rs` — uses `State<'_, AppConfig>`, `State<'_, HttpClient>`, returns `Result<T, String>`
- `daemon.rs` — `DaemonState` struct with `find_binary()`, `start()`, `kill()`, `process: Mutex<Option<Child>>`
- All HTTP commands use shared `HttpClient` state for connection pooling

- [ ] **Step 1: Create onboarding.rs with type definitions**

Create `phantom-mesh-desktop/src-tauri/src/commands/onboarding.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::net::TcpListener;
use sysinfo::System;

// ── Response Types ──────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct HardwareScanResult {
    pub gpu: String,           // "NVIDIA RTX 4090" or "CPU-only"
    pub vram_mb: u64,
    pub ram_mb: u64,
    pub ollama_status: String, // "online", "offline"
    pub ollama_models: Vec<String>,
    pub daemon_binary_path: Option<String>,
    pub available_port: u16,
}

#[derive(Debug, Serialize)]
pub struct OllamaProbeResult {
    pub ok: bool,
    pub models: Vec<String>,
    pub latency_ms: u64,
    pub speed_tier: String, // "Fast", "Medium", "Slow"
}

#[derive(Debug, Serialize)]
pub struct ValidationResult {
    pub ok: bool,
    pub models: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DaemonStatus {
    pub ok: bool,
    pub pid: Option<u32>,
    pub port: u16,
}

#[derive(Debug, Serialize)]
pub struct QrPayload {
    #[serde(rename = "type")]
    pub payload_type: String,
    pub version: u32,
    pub hub_url: String,
    pub auth_key: String,
    pub node_id: String,
}

#[derive(Debug, Deserialize)]
pub struct OnboardingConfig {
    pub port: u16,
    pub providers: Vec<ProviderEntry>,
    pub ollama_endpoint: Option<String>,
    pub default_agent_provider: String,
    pub default_agent_model: String,
    pub auth_key: String,
    pub telegram_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProviderEntry {
    pub name: String,         // "openai", "anthropic", etc.
    pub api_key: String,
    pub provider_type: String, // "openai", "anthropic", "gemini", "groq"
}
```

- [ ] **Step 2: Implement scan_hardware command**

Add to `onboarding.rs`:

```rust
#[tauri::command]
pub async fn scan_hardware() -> Result<HardwareScanResult, String> {
    let mut sys = System::new_all();
    sys.refresh_all();

    let ram_mb = sys.total_memory() / (1024 * 1024);

    // GPU detection (basic — check for known GPU processes or env vars)
    let gpu = detect_gpu();
    let vram_mb = 0; // Placeholder — real detection needs platform-specific code

    // Probe Ollama
    let (ollama_status, ollama_models) = probe_ollama_quick().await;

    // Find daemon binary — reuse DaemonState::find_binary() logic
    let daemon_binary_path = find_daemon_binary();

    // Find available port
    let available_port = find_available_port(7878);

    Ok(HardwareScanResult {
        gpu,
        vram_mb,
        ram_mb,
        ollama_status,
        ollama_models,
        daemon_binary_path,
        available_port,
    })
}

fn detect_gpu() -> String {
    // Check CUDA_VISIBLE_DEVICES or common GPU indicators
    if std::env::var("CUDA_VISIBLE_DEVICES").is_ok() {
        return "NVIDIA GPU (CUDA)".to_string();
    }
    #[cfg(target_os = "macos")]
    return "Apple Silicon (Metal)".to_string();
    #[cfg(not(target_os = "macos"))]
    "CPU-only".to_string()
}

async fn probe_ollama_quick() -> (String, Vec<String>) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    match client.get("http://localhost:11434/api/tags").send().await {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                let models = body["models"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| m["name"].as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                ("online".to_string(), models)
            } else {
                ("online".to_string(), vec![])
            }
        }
        _ => ("offline".to_string(), vec![]),
    }
}

fn find_daemon_binary() -> Option<String> {
    // Cross-platform binary name
    let bin_name = if cfg!(windows) { "phantom-mesh.exe" } else { "phantom-mesh" };

    // Same directory as this executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(bin_name);
            if candidate.exists() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }

    // Relative dev paths
    for profile in ["release", "debug"] {
        let candidate = std::path::PathBuf::from(format!(
            "../../phantom-mesh/target/{}/{}",
            profile, bin_name
        ));
        if candidate.exists() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }

    // Check PATH
    which::which("phantom-mesh")
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

fn find_available_port(preferred: u16) -> u16 {
    for port in preferred..preferred + 100 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    preferred
}
```

- [ ] **Step 3: Implement test_ollama command**

```rust
#[tauri::command]
pub async fn test_ollama(endpoint: String) -> Result<OllamaProbeResult, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let start = std::time::Instant::now();
    let resp = client
        .get(format!("{}/api/tags", endpoint))
        .send()
        .await
        .map_err(|e| format!("Cannot reach Ollama: {}", e))?;

    let latency_ms = start.elapsed().as_millis() as u64;

    if !resp.status().is_success() {
        return Ok(OllamaProbeResult {
            ok: false,
            models: vec![],
            latency_ms,
            speed_tier: "Unknown".to_string(),
        });
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let models = body["models"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let speed_tier = match latency_ms {
        0..=500 => "Fast",
        501..=3000 => "Medium",
        _ => "Slow",
    }
    .to_string();

    Ok(OllamaProbeResult {
        ok: true,
        models,
        latency_ms,
        speed_tier,
    })
}
```

- [ ] **Step 4: Implement validate_api_key command**

```rust
#[tauri::command]
pub async fn validate_api_key(
    http: tauri::State<'_, super::HttpClient>,
    provider: String,
    key: String,
) -> Result<ValidationResult, String> {
    let client = &http.0;

    let result = match provider.as_str() {
        "openai" => {
            let resp = client
                .get("https://api.openai.com/v1/models")
                .bearer_auth(&key)
                .send()
                .await;
            parse_model_list_response(resp, "data", "id").await
        }
        "anthropic" => {
            // Use /v1/models (read-only). Fallback: POST /v1/messages if this fails.
            let resp = client
                .get("https://api.anthropic.com/v1/models")
                .header("x-api-key", &key)
                .header("anthropic-version", "2023-06-01")
                .send()
                .await;
            parse_model_list_response(resp, "data", "id").await
        }
        "gemini" => {
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models?key={}",
                key
            );
            let resp = client.get(&url).send().await;
            parse_model_list_response(resp, "models", "name").await
        }
        "groq" => {
            let resp = client
                .get("https://api.groq.com/openai/v1/models")
                .bearer_auth(&key)
                .send()
                .await;
            parse_model_list_response(resp, "data", "id").await
        }
        _ => Err(format!("Unknown provider: {}", provider)),
    };

    result
}

async fn parse_model_list_response(
    resp: Result<reqwest::Response, reqwest::Error>,
    array_key: &str,
    name_key: &str,
) -> Result<ValidationResult, String> {
    match resp {
        Ok(r) if r.status().is_success() => {
            let body: serde_json::Value = r.json().await.unwrap_or_default();
            let models = body[array_key]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m[name_key].as_str().map(String::from))
                        .take(20) // Limit to 20 models
                        .collect()
                })
                .unwrap_or_default();
            Ok(ValidationResult {
                ok: true,
                models,
                error: None,
            })
        }
        Ok(r) => Ok(ValidationResult {
            ok: false,
            models: vec![],
            error: Some(format!("HTTP {}", r.status())),
        }),
        Err(e) => Ok(ValidationResult {
            ok: false,
            models: vec![],
            error: Some(e.to_string()),
        }),
    }
}
```

- [ ] **Step 5: Implement write_config and generate_qr_data commands**

```rust
#[tauri::command]
pub async fn write_config(
    app: tauri::AppHandle,
    data: OnboardingConfig,
) -> Result<(), String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;

    let toml_path = config_dir.join("agents.toml");

    // Backup existing config
    if toml_path.exists() {
        let backup = config_dir.join("agents.toml.bak");
        std::fs::copy(&toml_path, &backup).ok();
    }

    // Build agents.toml content
    let mut toml = format!(
        "[core]\nhost = \"0.0.0.0\"\nport = {}\n\n",
        data.port
    );

    // Ollama provider
    if let Some(ref endpoint) = data.ollama_endpoint {
        toml.push_str(&format!(
            "[providers.ollama]\ntype = \"ollama\"\nurl = \"{}\"\n\n",
            endpoint
        ));
    }

    // Cloud providers
    for p in &data.providers {
        toml.push_str(&format!(
            "[providers.{}]\ntype = \"{}\"\napi_key_env = \"{}\"\n\n",
            p.name,
            p.provider_type,
            format!("{}_API_KEY", p.name.to_uppercase())
        ));
    }

    // Default agent
    toml.push_str(&format!(
        "[agent.master]\nprovider = \"{}\"\nmodel = \"{}\"\ntools = [\"web_search\", \"http_request\"]\ninstructions = \"You are a helpful AI assistant.\"\n\n",
        data.default_agent_provider, data.default_agent_model
    ));

    // Auth
    toml.push_str(&format!(
        "[auth]\nbearer_token = \"{}\"\n",
        data.auth_key
    ));

    std::fs::write(&toml_path, &toml).map_err(|e| e.to_string())?;

    // Write .env file with API keys
    let env_path = config_dir.join(".env");
    let mut env_content = String::new();
    for p in &data.providers {
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

#[tauri::command]
pub fn generate_qr_data(hub_url: String, auth_key: String, node_id: String) -> QrPayload {
    QrPayload {
        payload_type: "phantom-mesh-hub".to_string(),
        version: 1,
        hub_url,
        auth_key,
        node_id,
    }
}
```

- [ ] **Step 6: Implement launch_daemon command**

```rust
#[tauri::command]
pub async fn launch_daemon(
    state: tauri::State<'_, crate::daemon::DaemonState>,
    http: tauri::State<'_, super::HttpClient>,
    vault_password: String,
    port: u16,
    binary_path: String,
) -> Result<DaemonStatus, String> {
    use std::process::Command;

    // Use the existing DaemonState to track the child process
    // This ensures stop_daemon, daemon_status, and tray quit all work correctly
    let child = Command::new(&binary_path)
        .arg("--host")
        .arg("0.0.0.0")
        .arg("--port")
        .arg(port.to_string())
        .arg("--vault-password")
        .arg(&vault_password)
        .arg("daemon")
        .spawn()
        .map_err(|e| format!("Failed to start daemon: {}", e))?;

    let pid = child.id();

    // Store child in DaemonState so existing stop/status commands work
    {
        let mut proc = state.process.lock().map_err(|e| e.to_string())?;
        *proc = Some(child);
    }

    // Wait for startup
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    // Health check with retries using shared HttpClient
    let url = format!("http://localhost:{}/health", port);
    for i in 0..5 {
        match http.0.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                return Ok(DaemonStatus {
                    ok: true,
                    pid: Some(pid),
                    port,
                });
            }
            _ => {
                if i < 4 {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        }
    }

    Ok(DaemonStatus {
        ok: false,
        pid: Some(pid),
        port,
    })
}
```

- [ ] **Step 7: Register module and commands**

In `commands/mod.rs`, add:
```rust
pub mod onboarding;
```

In `main.rs`, add to the `tauri::generate_handler![...]` macro:
```rust
onboarding::scan_hardware,
onboarding::test_ollama,
onboarding::validate_api_key,
onboarding::write_config,
onboarding::launch_daemon,
onboarding::generate_qr_data,
```

- [ ] **Step 8: Verify compilation**

```bash
cd phantom-mesh-desktop/src-tauri && CARGO_TARGET_DIR="C:/tmp/desktop-target" cargo check
```
Expected: zero errors.

- [ ] **Step 9: Commit**

```bash
git add phantom-mesh-desktop/src-tauri/src/commands/onboarding.rs phantom-mesh-desktop/src-tauri/src/commands/mod.rs phantom-mesh-desktop/src-tauri/src/main.rs
git commit -m "feat(desktop): add onboarding Tauri commands — hardware scan, API validation, config gen, daemon launch"
```

---

## Task 3: Desktop — Onboarding Types (`types.ts`)

**Files:**
- Create: `phantom-mesh-desktop/src/components/onboarding/types.ts`

- [ ] **Step 1: Create types file**

```typescript
// types.ts — Shared types for onboarding wizard

export interface HardwareScanResult {
  gpu: string;
  vram_mb: number;
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
}

export interface OnboardingData {
  hardwareScan: HardwareScanResult | null;
  vaultPassword: string;          // In-memory only, never persisted
  providers: ProviderConfig[];
  clusterEnabled: boolean;
  clusterNodes: string[];
  telegramToken: string;
  qrPayload: QrPayload | null;
  ollamaEndpoint: string;
  ollamaEnabled: boolean;
}

// What gets persisted to localStorage for crash recovery
// NOTE: vaultPassword and provider apiKeys are EXCLUDED
export interface PersistedWizardState {
  currentStep: number;
  ollamaEnabled: boolean;
  ollamaEndpoint: string;
  providerNames: string[];        // Just names, not keys
  clusterEnabled: boolean;
  clusterNodes: string[];
  telegramConfigured: boolean;
}

export type WizardStep = 0 | 1 | 2 | 3 | 4;

export const WIZARD_STORAGE_KEY = 'phantom-mesh_onboarding_state';
export const ONBOARDED_KEY = 'phantom-mesh_onboarded';
```

- [ ] **Step 2: Verify TS compiles**

```bash
cd phantom-mesh-desktop && npx tsc --noEmit
```

- [ ] **Step 3: Commit**

```bash
git add phantom-mesh-desktop/src/components/onboarding/types.ts
git commit -m "feat(desktop): add onboarding type definitions"
```

---

## Task 4: Desktop — Wizard State Hook (`useWizardState.ts`)

**Files:**
- Create: `phantom-mesh-desktop/src/components/onboarding/useWizardState.ts`

- [ ] **Step 1: Create the hook**

```typescript
import { useState, useCallback, useEffect } from 'react';
import {
  OnboardingData, PersistedWizardState, WizardStep,
  WIZARD_STORAGE_KEY, ONBOARDED_KEY,
} from './types';

const INITIAL_DATA: OnboardingData = {
  hardwareScan: null,
  vaultPassword: '',
  providers: [],
  clusterEnabled: false,
  clusterNodes: [],
  telegramToken: '',
  qrPayload: null,
  ollamaEndpoint: 'http://localhost:11434',
  ollamaEnabled: false,
};

function loadPersistedState(): { step: WizardStep; data: Partial<OnboardingData> } {
  try {
    const raw = localStorage.getItem(WIZARD_STORAGE_KEY);
    if (!raw) return { step: 0, data: {} };
    const saved: PersistedWizardState = JSON.parse(raw);
    // If we had a vault step completed but crashed, reset to step 1 (re-enter password)
    const step = saved.currentStep >= 2 ? 1 as WizardStep : saved.currentStep as WizardStep;
    return {
      step,
      data: {
        ollamaEnabled: saved.ollamaEnabled,
        ollamaEndpoint: saved.ollamaEndpoint,
        clusterEnabled: saved.clusterEnabled,
        clusterNodes: saved.clusterNodes,
      },
    };
  } catch {
    return { step: 0, data: {} };
  }
}

export function useWizardState() {
  const persisted = loadPersistedState();

  const [currentStep, setCurrentStep] = useState<WizardStep>(persisted.step);
  const [data, setData] = useState<OnboardingData>({
    ...INITIAL_DATA,
    ...persisted.data,
  });

  // Persist non-sensitive state on step change
  useEffect(() => {
    const state: PersistedWizardState = {
      currentStep,
      ollamaEnabled: data.ollamaEnabled,
      ollamaEndpoint: data.ollamaEndpoint,
      providerNames: data.providers.map(p => p.name),
      clusterEnabled: data.clusterEnabled,
      clusterNodes: data.clusterNodes,
      telegramConfigured: !!data.telegramToken,
    };
    localStorage.setItem(WIZARD_STORAGE_KEY, JSON.stringify(state));
  }, [currentStep, data]);

  const goNext = useCallback(() => {
    setCurrentStep(s => Math.min(s + 1, 4) as WizardStep);
  }, []);

  const goBack = useCallback(() => {
    setCurrentStep(s => Math.max(s - 1, 0) as WizardStep);
  }, []);

  const updateData = useCallback((partial: Partial<OnboardingData>) => {
    setData(prev => ({ ...prev, ...partial }));
  }, []);

  const completeWizard = useCallback(() => {
    localStorage.setItem(ONBOARDED_KEY, 'true');
    localStorage.removeItem(WIZARD_STORAGE_KEY);
  }, []);

  return { currentStep, data, goNext, goBack, updateData, completeWizard };
}
```

- [ ] **Step 2: Verify TS compiles**

```bash
cd phantom-mesh-desktop && npx tsc --noEmit
```

- [ ] **Step 3: Commit**

```bash
git add phantom-mesh-desktop/src/components/onboarding/useWizardState.ts
git commit -m "feat(desktop): add wizard state hook with crash recovery"
```

---

## Task 5: Desktop — Hardware Scan Hook (`useHardwareScan.ts`)

**Files:**
- Create: `phantom-mesh-desktop/src/components/onboarding/useHardwareScan.ts`

- [ ] **Step 1: Create the hook**

```typescript
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { HardwareScanResult } from './types';

export type ScanStatus = 'idle' | 'scanning' | 'done' | 'error';

export function useHardwareScan(autoStart: boolean) {
  const [result, setResult] = useState<HardwareScanResult | null>(null);
  const [status, setStatus] = useState<ScanStatus>('idle');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!autoStart) return;

    setStatus('scanning');
    invoke<HardwareScanResult>('scan_hardware')
      .then((r) => {
        setResult(r);
        setStatus('done');
      })
      .catch((e) => {
        setError(String(e));
        setStatus('error');
      });
  }, [autoStart]);

  return { result, status, error };
}
```

- [ ] **Step 2: Verify TS compiles**

```bash
cd phantom-mesh-desktop && npx tsc --noEmit
```

- [ ] **Step 3: Commit**

```bash
git add phantom-mesh-desktop/src/components/onboarding/useHardwareScan.ts
git commit -m "feat(desktop): add hardware scan hook"
```

---

## Task 6: Desktop — Step 1 Welcome (`StepWelcome.tsx`)

**Files:**
- Create: `phantom-mesh-desktop/src/components/onboarding/StepWelcome.tsx`

- [ ] **Step 1: Create component**

Build the welcome screen showing Phantom Mesh logo, value proposition, and animated hardware scan results from `useHardwareScan`. Display scan items one by one with ✓/✗ indicators. Pass `onNext` callback and `hardwareScan` result up via `updateData`.

Key elements:
- Title: "歡迎來到 Phantom Mesh"
- Three value propositions
- Scan result list: GPU, RAM, Ollama, Daemon binary, Port — each appearing with animation
- "開始設定 →" button

- [ ] **Step 2: Verify TS compiles**

```bash
cd phantom-mesh-desktop && npx tsc --noEmit
```

- [ ] **Step 3: Commit**

```bash
git add phantom-mesh-desktop/src/components/onboarding/StepWelcome.tsx
git commit -m "feat(desktop): add onboarding Step 1 — welcome + hardware scan"
```

---

## Task 7: Desktop — Step 2 Security (`StepSecurity.tsx`)

**Files:**
- Create: `phantom-mesh-desktop/src/components/onboarding/StepSecurity.tsx`

- [ ] **Step 1: Create component**

Build vault password input with:
- Password input (masked) + confirm input
- Strength meter: < 8 blocked (red), 8-11 weak (yellow), 12-15 medium (blue), 16+ strong (green)
- Warning: "密碼遺失無法恢復"
- Match validation on confirm
- Calls `updateData({ vaultPassword })` on valid submit
- Buttons: "← 上一步" | "設定密碼 →" (disabled until valid)

- [ ] **Step 2: Verify TS compiles + commit**

```bash
cd phantom-mesh-desktop && npx tsc --noEmit
git add phantom-mesh-desktop/src/components/onboarding/StepSecurity.tsx
git commit -m "feat(desktop): add onboarding Step 2 — vault password with strength meter"
```

---

## Task 8: Desktop — Step 3 Providers (`StepProviders.tsx`)

**Files:**
- Create: `phantom-mesh-desktop/src/components/onboarding/StepProviders.tsx`

- [ ] **Step 1: Create component**

Build provider configuration UI with:
- **Ollama section** (conditional): if `hardwareScan.ollama_status === 'online'`, show "已偵測到 Ollama ✓" with toggle + model list
- **Cloud providers grid**: checkboxes for OpenAI, Anthropic, Gemini, Groq
- Each checked provider expands: API key input (masked) + "驗證" button
- "驗證" calls `invoke('validate_api_key', { provider, key })` → shows ✓/✗ result
- Four-tier routing explanation text
- Must have at least 1 provider to proceed
- Calls `updateData({ providers, ollamaEnabled, ollamaEndpoint })`

- [ ] **Step 2: Verify TS compiles + commit**

```bash
cd phantom-mesh-desktop && npx tsc --noEmit
git add phantom-mesh-desktop/src/components/onboarding/StepProviders.tsx
git commit -m "feat(desktop): add onboarding Step 3 — provider setup with API validation"
```

---

## Task 9: Desktop — Step 4 Network (`StepNetwork.tsx`)

**Files:**
- Create: `phantom-mesh-desktop/src/components/onboarding/StepNetwork.tsx`

- [ ] **Step 1: Create component**

Build network/cluster configuration with:
- Toggle: "要組建叢集嗎？" (default off)
- If enabled: placeholder for discovered nodes + manual IP input
- **QR Code section**: render QR from `qrPayload` using `qrcode.react`
  - Call `invoke('generate_qr_data', { hub_url, auth_key, node_id })` to get payload
  - Display hub URL and auth key as copyable text
- **Telegram section**: optional toggle + token input + validate button
  - Validate: `fetch('https://api.telegram.org/bot{token}/getMe')`
- "跳過" button skips to Step 5

- [ ] **Step 2: Verify TS compiles + commit**

```bash
cd phantom-mesh-desktop && npx tsc --noEmit
git add phantom-mesh-desktop/src/components/onboarding/StepNetwork.tsx
git commit -m "feat(desktop): add onboarding Step 4 — network, QR code, Telegram"
```

---

## Task 10: Desktop — Step 5 Complete (`StepComplete.tsx`)

**Files:**
- Create: `phantom-mesh-desktop/src/components/onboarding/StepComplete.tsx`

- [ ] **Step 1: Create component**

Build summary + launch screen:
- Summary card showing all configured items (daemon port, vault ✓, providers list, cluster status, Telegram status)
- Warning if existing `agents.toml` found (will be backed up)
- "🚀 啟動 Phantom Mesh" button triggers:
  1. `invoke('write_config', { data: onboardingConfig })` — writes agents.toml + .env
  2. `invoke('launch_daemon', { vault_password, port, binary_path })` — starts daemon (stored in DaemonState)
  3. After daemon is live, `POST http://localhost:{port}/vault/store-keys` with API keys — daemon encrypts and stores in KeyVault (if endpoint exists; fallback: .env file is already written with keys)
  4. Show progress spinner → success animation
  5. Call `completeWizard()` from hook
  6. Auto-navigate with 3s countdown + "立即前往" skip button
- Error handling: if daemon fails, show error + retry button

- [ ] **Step 2: Verify TS compiles + commit**

```bash
cd phantom-mesh-desktop && npx tsc --noEmit
git add phantom-mesh-desktop/src/components/onboarding/StepComplete.tsx
git commit -m "feat(desktop): add onboarding Step 5 — summary + daemon launch"
```

---

## Task 11: Desktop — Wizard Container + App.tsx Integration

**Files:**
- Create: `phantom-mesh-desktop/src/components/onboarding/OnboardingWizard.tsx`
- Modify: `phantom-mesh-desktop/src/App.tsx`

- [ ] **Step 1: Create OnboardingWizard container**

Main wizard component that:
- Uses `useWizardState()` for step navigation
- Uses `useHardwareScan(true)` to start scan immediately
- Renders progress dots at top (5 dots, current highlighted)
- Renders current step component based on `currentStep`
- Full-screen dark background (#0f0f1a), centered content max-width ~600px
- Passes `data`, `updateData`, `goNext`, `goBack` to each step

```typescript
import { useWizardState } from './useWizardState';
import { useHardwareScan } from './useHardwareScan';
import StepWelcome from './StepWelcome';
import StepSecurity from './StepSecurity';
import StepProviders from './StepProviders';
import StepNetwork from './StepNetwork';
import StepComplete from './StepComplete';

interface Props {
  onComplete: () => void;
}

export default function OnboardingWizard({ onComplete }: Props) {
  const { currentStep, data, goNext, goBack, updateData, completeWizard } = useWizardState();
  const scan = useHardwareScan(true);

  // Pass scan result to data when ready
  // ... (useEffect to updateData when scan completes)

  const steps = [
    <StepWelcome scan={scan} onNext={goNext} />,
    <StepSecurity data={data} updateData={updateData} onNext={goNext} onBack={goBack} />,
    <StepProviders data={data} updateData={updateData} onNext={goNext} onBack={goBack} />,
    <StepNetwork data={data} updateData={updateData} onNext={goNext} onBack={goBack} />,
    <StepComplete data={data} completeWizard={completeWizard} onComplete={onComplete} onBack={goBack} />,
  ];

  return (
    <div className="min-h-screen bg-[#0f0f1a] flex flex-col items-center justify-center p-8">
      {/* Progress dots */}
      <div className="flex gap-2 mb-8">
        {[0, 1, 2, 3, 4].map(i => (
          <div key={i} className={`w-2.5 h-2.5 rounded-full ${
            i === currentStep ? 'bg-blue-400' : i < currentStep ? 'bg-blue-400/50' : 'bg-gray-700'
          }`} />
        ))}
      </div>
      {/* Current step */}
      <div className="w-full max-w-xl">
        {steps[currentStep]}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Modify App.tsx — add first-run gate**

In `phantom-mesh-desktop/src/App.tsx`, add before the existing return:

```typescript
import { useState } from 'react';
import OnboardingWizard from './components/onboarding/OnboardingWizard';
import { ONBOARDED_KEY } from './components/onboarding/types';

export default function App() {
  const [onboarded, setOnboarded] = useState(() =>
    localStorage.getItem(ONBOARDED_KEY) === 'true'
  );

  if (!onboarded) {
    return <OnboardingWizard onComplete={() => setOnboarded(true)} />;
  }

  // ... existing sidebar + routes layout
}
```

- [ ] **Step 3: Verify TS compiles**

```bash
cd phantom-mesh-desktop && npx tsc --noEmit
```

- [ ] **Step 4: Commit**

```bash
git add phantom-mesh-desktop/src/components/onboarding/OnboardingWizard.tsx phantom-mesh-desktop/src/App.tsx
git commit -m "feat(desktop): add wizard container + App.tsx first-run gate"
```

---

## Task 12: Desktop — Post-Onboarding Checklist

**Files:**
- Create: `phantom-mesh-desktop/src/components/OnboardingChecklist.tsx`
- Modify: `phantom-mesh-desktop/src/pages/Dashboard.tsx`

- [ ] **Step 1: Create checklist component**

```typescript
import { useState, useEffect } from 'react';
import { useNavigate } from 'react-router-dom';

interface CheckItem {
  label: string;
  done: boolean;
  link?: string;
}

export default function OnboardingChecklist() {
  const navigate = useNavigate();
  const [dismissed, setDismissed] = useState(() =>
    localStorage.getItem('phantom-mesh_checklist_dismissed') === 'true'
  );

  // TODO: check actual conditions via API calls
  const items: CheckItem[] = [
    { label: '設定 KeyVault 密碼', done: true },
    { label: '新增至少一個 Provider', done: true },
    { label: '新增第二個 Provider（備援）', done: false, link: '/providers' },
    { label: '連接一台 Mobile Worker', done: false, link: '/cluster' },
    { label: '設定 Search API', done: false, link: '/providers' },
    { label: '發送第一則 Chat 訊息', done: false, link: '/chat' },
  ];

  const doneCount = items.filter(i => i.done).length;

  if (dismissed || doneCount === items.length) return null;

  const handleDismiss = () => {
    localStorage.setItem('phantom-mesh_checklist_dismissed', 'true');
    setDismissed(true);
  };

  return (
    <div className="bg-phantom-mesh-card border border-phantom-mesh-border rounded-xl p-4 mb-6">
      <div className="flex justify-between items-center mb-3">
        <span className="text-phantom-mesh-primary font-semibold text-sm">
          完成設定（{doneCount}/{items.length}）
        </span>
        <button onClick={handleDismiss} className="text-phantom-mesh-muted text-xs hover:text-white">
          ✕ 關閉
        </button>
      </div>
      <div className="grid grid-cols-2 gap-2">
        {items.map((item, i) => (
          <div
            key={i}
            onClick={() => item.link && !item.done && navigate(item.link)}
            className={`rounded-lg px-3 py-2 text-xs flex items-center gap-2 ${
              item.done
                ? 'bg-green-900/30 border border-green-800 text-green-400 line-through'
                : 'bg-phantom-mesh-bg border border-phantom-mesh-border text-phantom-mesh-text cursor-pointer hover:border-phantom-mesh-primary'
            }`}
          >
            <span>{item.done ? '✓' : '○'}</span>
            <span>{item.label}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Add to Dashboard.tsx**

At top of Dashboard component's return, add:
```typescript
import OnboardingChecklist from '../components/OnboardingChecklist';

// Inside the return, before existing content:
<OnboardingChecklist />
```

- [ ] **Step 3: Verify TS compiles + commit**

```bash
cd phantom-mesh-desktop && npx tsc --noEmit
git add phantom-mesh-desktop/src/components/OnboardingChecklist.tsx phantom-mesh-desktop/src/pages/Dashboard.tsx
git commit -m "feat(desktop): add post-onboarding checklist on Dashboard"
```

---

## Task 13: Mobile — Onboarding Types

**Files:**
- Create: `mobile/phantom-mesh-worker-app/src/components/onboarding/types.ts`

- [ ] **Step 1: Create types**

```typescript
export interface MobileOnboardingData {
  hubUrl: string;
  authKey: string;
  workerName: string;
  agentName: string;
  connectionTested: boolean;
}

export const INITIAL_MOBILE_DATA: MobileOnboardingData = {
  hubUrl: '',
  authKey: '',
  workerName: '',
  agentName: 'chatgpt',
  connectionTested: false,
};
```

- [ ] **Step 2: Verify TS + commit**

```bash
cd mobile/phantom-mesh-worker-app && npx tsc --noEmit
git add mobile/phantom-mesh-worker-app/src/components/onboarding/types.ts
git commit -m "feat(mobile): add onboarding type definitions"
```

---

## Task 14: Mobile — Steps 1-2 (Welcome + Connect)

**Files:**
- Create: `mobile/phantom-mesh-worker-app/src/components/onboarding/StepWelcome.tsx`
- Create: `mobile/phantom-mesh-worker-app/src/components/onboarding/StepConnect.tsx`

- [ ] **Step 1: Create StepWelcome**

Simple welcome screen with Phantom Mesh Worker branding, 3 value props, "開始設定" button. Use existing dark theme (#0f0f1a).

- [ ] **Step 2: Create StepConnect**

Hub connection screen with:
- "📷 掃描 QR Code" button → opens `expo-camera` BarCodeScanner
- Parse QR JSON: validate `type === "phantom-mesh-hub"`, extract hub_url + auth_key
- Manual input fallback: Hub URL + Auth Key fields
- "測試連線" button → `fetch(${hubUrl}/health)` with AbortController 5s timeout
- Connection status display

- [ ] **Step 3: Verify TS + commit**

```bash
cd mobile/phantom-mesh-worker-app && npx tsc --noEmit
git add mobile/phantom-mesh-worker-app/src/components/onboarding/StepWelcome.tsx mobile/phantom-mesh-worker-app/src/components/onboarding/StepConnect.tsx
git commit -m "feat(mobile): add onboarding Steps 1-2 — welcome + hub connection"
```

---

## Task 15: Mobile — Steps 3-4 (Identity + Complete)

**Files:**
- Create: `mobile/phantom-mesh-worker-app/src/components/onboarding/StepIdentity.tsx`
- Create: `mobile/phantom-mesh-worker-app/src/components/onboarding/StepComplete.tsx`

- [ ] **Step 1: Create StepIdentity**

Worker identity screen:
- Worker Name input (alphanumeric + hyphens validation)
- Agent name input (default "chatgpt")
- Auto-detected device info card (Device model, OS, RAM, battery via React Native APIs)

- [ ] **Step 2: Create StepComplete**

Summary + launch screen:
- Config summary (Hub URL, Worker Name, Agent)
- Android battery optimization warning with "前往電池設定" button (uses `Linking.openSettings()`)
- "連線並啟動 Worker" button → converts `MobileOnboardingData` to `WorkerConfig`:
  ```typescript
  const config: WorkerConfig = {
    hubUrl: data.hubUrl,
    workerName: data.workerName,
    authKey: data.authKey,
    agentName: data.agentName,
    pollIntervalMs: 15000,
  };
  await saveConfig(config);
  onComplete(config);
  ```

- [ ] **Step 3: Verify TS + commit**

```bash
cd mobile/phantom-mesh-worker-app && npx tsc --noEmit
git add mobile/phantom-mesh-worker-app/src/components/onboarding/StepIdentity.tsx mobile/phantom-mesh-worker-app/src/components/onboarding/StepComplete.tsx
git commit -m "feat(mobile): add onboarding Steps 3-4 — identity + launch"
```

---

## Task 16: Mobile — Wizard Container

**Files:**
- Create: `mobile/phantom-mesh-worker-app/src/components/onboarding/OnboardingWizard.tsx`

- [ ] **Step 1: Create container**

Main wizard component with:
- useState for `currentStep` (0-3) and `data` (MobileOnboardingData)
- Persist current step to `AsyncStorage` key `@phantom-mesh_onboarding_step` for crash recovery
- On mount, check AsyncStorage for saved step and resume
- ScrollView with step-based rendering
- Progress indicator (4 dots)
- Navigation: goNext, goBack callbacks
- Dark theme consistent with existing app (#0f0f1a)
- Use `export default` (consistent with desktop)

- [ ] **Step 2: Verify TS + commit**

```bash
cd mobile/phantom-mesh-worker-app && npx tsc --noEmit
git add mobile/phantom-mesh-worker-app/src/components/onboarding/OnboardingWizard.tsx
git commit -m "feat(mobile): add onboarding wizard container"
```

---

## Task 17: Mobile — App.tsx Integration

**Files:**
- Modify: `mobile/phantom-mesh-worker-app/src/App.tsx`

- [ ] **Step 1: Replace SettingsScreen fallback with OnboardingWizard**

In `App.tsx`, find the block:
```typescript
if (!config) {
  return (
    <>
      <StatusBar style="light" />
      <SettingsScreen config={null} onSave={handleSaveConfig} onDisconnect={handleDisconnect} />
    </>
  );
}
```

Replace with:
```typescript
import OnboardingWizard from './components/onboarding/OnboardingWizard';

if (!config) {
  return (
    <>
      <StatusBar style="light" />
      <OnboardingWizard onComplete={(newConfig) => handleSaveConfig(newConfig)} />
    </>
  );
}
```

- [ ] **Step 2: Verify TS compiles**

```bash
cd mobile/phantom-mesh-worker-app && npx tsc --noEmit
```

- [ ] **Step 3: Commit**

```bash
git add mobile/phantom-mesh-worker-app/src/App.tsx
git commit -m "feat(mobile): integrate onboarding wizard into App.tsx"
```

---

## Task 18: Mobile — Post-Onboarding Checklist

**Files:**
- Create: `mobile/phantom-mesh-worker-app/src/components/OnboardingChecklist.tsx`
- Modify: `mobile/phantom-mesh-worker-app/src/screens/HomeScreen.tsx`

- [ ] **Step 1: Create checklist component**

React Native checklist banner:
- "完成設定（N/M）" header with dismiss button
- Items: Hub connected ✓, Worker name set ✓, Android battery optimization ○, First task ○, NL command ○
- Each incomplete item navigates to relevant tab
- Stored in AsyncStorage `@phantom-mesh_checklist_dismissed`

- [ ] **Step 2: Add to HomeScreen.tsx**

Import and render `<OnboardingChecklist />` at top of HomeScreen, above the existing stat cards.

- [ ] **Step 3: Verify TS + commit**

```bash
cd mobile/phantom-mesh-worker-app && npx tsc --noEmit
git add mobile/phantom-mesh-worker-app/src/components/OnboardingChecklist.tsx mobile/phantom-mesh-worker-app/src/screens/HomeScreen.tsx
git commit -m "feat(mobile): add post-onboarding checklist on HomeScreen"
```

---

## Final Verification

After all tasks are complete:

- [ ] **Desktop full check**

```bash
cd phantom-mesh-desktop/src-tauri && CARGO_TARGET_DIR="C:/tmp/desktop-target" cargo check
cd phantom-mesh-desktop && npx tsc --noEmit
```

- [ ] **Mobile full check**

```bash
cd mobile/phantom-mesh-worker-app && npx tsc --noEmit
```

- [ ] **Core regression check**

```bash
cd phantom-mesh && CARGO_TARGET_DIR="C:/tmp/phantom-mesh-target" cargo test 2>&1 | grep "test result:"
```
Expected: all existing tests still pass (3771+).
