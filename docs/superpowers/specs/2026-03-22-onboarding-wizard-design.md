# Onboarding Wizard Design — Desktop & Mobile

> Date: 2026-03-22
> Status: Approved
> Parent Spec: `phantom-mesh/docs/superpowers/specs/2026-03-21-phantom-mesh-app-platform-design.md`

---

## 1. Overview

Phantom Mesh currently has no first-run experience on Desktop and only a bare-minimum setup screen on Mobile. Users land on a 16-page sidebar (Desktop) or a settings form (Mobile) with no guidance on what to configure first.

This spec defines a **Progressive Wizard** (Desktop: 5 steps, Mobile: 4 steps) that guides new users through essential setup with background auto-detection, sensible defaults, and skip-able optional steps.

### Design Principles

1. **Background Detection** — Scan hardware, Ollama, daemon binary, and port availability while the user reads the welcome screen. Never block on detection.
2. **Security First** — Vault password is the only mandatory step (Desktop). All other steps can be skipped.
3. **One Decision Per Step** — Each wizard page asks for exactly one category of input.
4. **Crash Recovery** — Persist wizard state per-step so users resume where they left off after app restart.
5. **QR Code Bridge** — Desktop generates a QR code containing hub URL + auth key; Mobile scans it to auto-fill connection details.
6. **Post-Onboarding Checklist** — After the wizard, a dismissible banner on Dashboard shows 2-4 remaining setup items.

### Inspirations

| Source | Pattern Adopted |
|--------|----------------|
| Jan.ai | Background model download while user explores UI |
| Home Assistant | "One input, many derived settings" — location → timezone/units/currency |
| LM Studio | Hardware-aware model recommendations ranked by VRAM |
| Tailscale | Identity-first, then auto-mesh configuration |
| Open WebUI | First user = permanent admin (security-first) |

---

## 2. Desktop Wizard — 5 Steps

### 2.1 Step 1: Welcome + Hardware Scan

**UI:**
- Phantom Mesh logo + title "歡迎來到 Phantom Mesh"
- Three-line value proposition
- Animated scan results appearing one by one (checkmark or cross)
- Button: "開始設定 →"

**Background tasks (non-blocking, started immediately):**
- Detect GPU type (CUDA / Metal / ROCm / CPU-only)
- Scan RAM and VRAM
- Probe Ollama at `localhost:11434` via `GET /api/tags`
- Search for daemon binary in 4 locations:
  1. Explicit path from config (if exists)
  2. Same directory as desktop app executable
  3. Relative dev paths: `../../phantom-mesh/target/{release,debug}/phantom-mesh{.exe}`
  4. PATH environment variable
- Check port 7878 availability; find next free port if occupied

**Data collected:** `HardwareScanResult { gpu, vram_mb, ram_mb, ollama_status, ollama_models, daemon_binary_path, available_port }`

### 2.2 Step 2: Security (KeyVault)

**UI:**
- Explanation: "設定主密碼保護你的 API 金鑰"
- Password input (masked)
- Confirm password input (masked)
- Password strength indicator (weak/medium/strong) — min 8 chars, recommend 12+
- Warning text: "密碼遺失無法恢復"
- Buttons: "← 上一步" | "設定密碼 →"

**Validation:**
- Passwords must match
- Minimum 8 characters
- Strength meter: < 8 = blocked, 8-11 = weak, 12-15 = medium, 16+ = strong

**Skip:** Not skippable. This is the security foundation.

**Backend:** The Tauri app holds the vault password in memory (not persisted to disk). The password is passed to the daemon at startup (Step 5) via the `--vault-password` CLI flag or written to a temporary in-memory config. The Tauri app does NOT create a vault instance itself — vault creation is delegated to the daemon on first launch.

**Crash recovery:** If the user completes Step 2 but crashes before Step 5, the password is lost. On resume, the wizard returns to Step 2 to re-enter the password. The password is intentionally never persisted to disk for security.

### 2.3 Step 3: AI Engine (Providers)

**UI — Local LLM section (conditional):**
- If Step 1 detected Ollama: show "已偵測到 Ollama ✓" with auto-enabled toggle
- Display available models list with VRAM requirements
- Latency probe result and speed tier classification (Fast/Medium/Slow)
- If not detected: show "未偵測到本地 LLM" with collapsed manual endpoint input

**UI — Cloud Providers section:**
- Checkbox grid: OpenAI, Anthropic, Gemini, Groq (expandable for others)
- Each selected provider expands to show:
  - API Key input (masked, with show/hide toggle)
  - "驗證" button — calls provider-specific API to test key validity
  - Status indicator: ✓ valid / ✗ invalid / ... testing
- Four-tier routing explanation diagram (Local → FreeApi → Subscription → PayAsYouGo)

**Validation:** At least one provider must be configured to proceed.

**Backend:** The Tauri app validates API keys directly via HTTP (not through the daemon, which isn't running yet):
- OpenAI: `GET https://api.openai.com/v1/models` with `Authorization: Bearer {key}`
- Anthropic: `POST https://api.anthropic.com/v1/messages` with `x-api-key: {key}` (minimal request)
- Gemini: `GET https://generativelanguage.googleapis.com/v1beta/models?key={key}`
- Groq: `GET https://api.groq.com/openai/v1/models` with `Authorization: Bearer {key}`

Validated keys are held in the Tauri app's memory during the wizard. On Step 5, they are written to `agents.toml` as `api_key_name` references, and the daemon stores them in the KeyVault on first launch.

### 2.4 Step 4: Network + Cluster (Skippable)

**UI:**
- Toggle: "要組建叢集嗎？" — default off, entire section collapsible
- If enabled:
  - mDNS discovery results (auto-scanned): list of discovered nodes with one-click "加入" button
  - Manual node input: IP:port field
- Mobile connection section:
  - QR Code rendered from canonical payload (see Section 6 for full format)
  - Text instruction: "在手機 App 掃描此 QR Code"
  - Display hub URL and auth key in copyable text (fallback)
- Telegram Bot (optional toggle):
  - Bot Token input (masked)
  - "驗證" button — calls `GET https://api.telegram.org/bot{token}/getMe`
  - Status: ✓ connected / ✗ invalid

**Skip:** Fully skippable. Button: "跳過" navigates directly to Step 5.

**Backend:**
- `generate_qr_data()` → returns the canonical QR payload (see Section 6):
  ```json
  { "type": "phantom-mesh-hub", "version": 1, "hub_url": "http://{local_ip}:{port}", "auth_key": "{generated_token}", "node_id": "{desktop_node_id}" }
  ```
- `auth_key` is a 32-character random hex string generated by `generate_qr_data()`, stored in the wizard state and written to `agents.toml` on Step 5
- QR code rendering is frontend-only (using `qrcode.react` or similar library)

### 2.5 Step 5: Summary + Launch

**UI:**
- Summary card listing all configured items:
  - Daemon: port, binary path
  - Security: KeyVault ✓
  - Providers: list of enabled providers with model counts
  - Cluster: node count or "未設定"
  - Telegram: connected or "未設定"
- Large button: "🚀 啟動 Phantom Mesh"
- Progress: spinner → health check → success animation
- Auto-navigate to Dashboard after 3 seconds (with countdown, click to skip)

**Backend:**
1. Write `agents.toml` configuration from collected wizard data (see Section 4.6 for template)
2. Write `.env` file with API keys and tokens
3. Call `start_daemon(config)` — spawns daemon process with `--vault-password {password}` flag, waits 1.5s, runs health check with 5 retries
4. After daemon is live, call `POST /vault/store-keys` with the API keys collected in Step 3 (daemon encrypts and stores them in KeyVault)
5. Set `localStorage.setItem('phantom-mesh_onboarded', 'true')`
6. Clear wizard state from localStorage (except vault password, which was in-memory only)

**Auto-navigate:** Dashboard after 3 seconds, with visible countdown and "立即前往" click-to-skip option.

---

## 3. Mobile Wizard — 4 Steps

### 3.1 Step 1: Welcome

**UI:**
- Phantom Mesh Worker logo + "Mobile Light Node Setup"
- Three-line value proposition:
  - 將手機變成 AI 叢集的一部分
  - 執行 Hub 分配的任務
  - 即時推播通知任務結果
- Button: "開始設定"
- Swipe gesture support for navigation

### 3.2 Step 2: Connect Hub

**UI — QR Code option:**
- "📷 掃描 QR Code" button — opens camera with barcode scanner
- Parses canonical QR payload: `{ type: "phantom-mesh-hub", version: 1, hub_url, auth_key, node_id }`
- Validates `type === "phantom-mesh-hub"` and `version === 1`
- Auto-fills hub_url and auth_key fields, triggers automatic connection test

**UI — Manual option:**
- Divider: "— 或手動輸入 —"
- Hub URL input with placeholder: `http://192.168.1.100:7878`
- Auth Key input (optional, masked)
- "測試連線" button — calls `GET {hub_url}/health` with Bearer auth
- Status display: "✓ Hub 已連線 — Phantom Mesh v{version}" or error message

**Validation:** Hub must be reachable (successful health check) to proceed.

**Note:** The `/health` endpoint is unauthenticated (public). The auth_key is validated separately when the worker registers with the hub. If the user provides an auth_key, it is sent as `Authorization: Bearer {key}` but the health check itself does not require it.

**Backend:** Uses existing `fetch()` with `AbortController` timeout from bridge.ts pattern.

### 3.3 Step 3: Worker Identity

**UI:**
- Worker Name input with placeholder: `my-pixel-8`
  - Validation: alphanumeric + hyphens, must be unique in cluster
- Agent text input with default: "chatgpt"
  - Note: no `/agents` list endpoint exists on the daemon; user types agent name manually or uses default
- Auto-detected device info card (read-only):
  - Device model, OS version, RAM, battery level

**Validation:** Worker name is required and must be non-empty.

### 3.4 Step 4: Complete + Launch

**UI:**
- Summary of configured items:
  - Hub: URL
  - Worker: name
  - Agent: name
  - Poll interval: 15s (default, shown as info)
- Android-only: battery optimization warning banner
  - Explanation text
  - "前往電池設定" button → opens system battery settings
- Large button: "連線並啟動 Worker"
- On success: save config to AsyncStorage, navigate to Home tab

---

## 4. Technical Architecture

### 4.1 Desktop — New Files

```
phantom-mesh-desktop/src/
  components/onboarding/
    OnboardingWizard.tsx    -- Main container, step state machine
    WizardProgress.tsx      -- Top progress bar (step dots)
    StepWelcome.tsx         -- Step 1
    StepSecurity.tsx        -- Step 2
    StepProviders.tsx       -- Step 3
    StepNetwork.tsx         -- Step 4
    StepComplete.tsx        -- Step 5
    useHardwareScan.ts      -- Hook: background hardware detection
    useWizardState.ts       -- Hook: wizard state + crash recovery
    types.ts                -- OnboardingData type definition

phantom-mesh-desktop/src-tauri/src/commands/
    onboarding.rs           -- 6 Tauri commands
```

### 4.2 Desktop — Tauri Commands

```rust
#[tauri::command]
async fn scan_hardware() -> Result<HardwareScanResult, String>
// Detects GPU, VRAM, RAM, Ollama status, daemon binary path, port availability
// Uses sysinfo crate (already a dependency) + HTTP probe to Ollama

#[tauri::command]
async fn test_ollama(endpoint: String) -> Result<OllamaProbeResult, String>
// GET /api/tags, measure latency, classify speed tier (Fast/Medium/Slow)

#[tauri::command]
async fn validate_api_key(provider: String, key: String) -> Result<ValidationResult, String>
// Direct HTTP call to provider API (NOT through daemon — daemon isn't running yet)
// Returns { ok: bool, models: Vec<String>, error: Option<String> }

#[tauri::command]
async fn write_config(data: OnboardingConfig) -> Result<(), String>
// Writes agents.toml + .env from wizard data (see Section 4.6 for template)
// Config path: {app_data_dir}/agents.toml

#[tauri::command]
async fn launch_daemon(vault_password: String, port: u16, binary_path: String) -> Result<DaemonStatus, String>
// Spawns daemon with --vault-password flag, waits 1.5s, health check with 5 retries
// Returns { ok: bool, pid: u32, port: u16 }
// Note: extends existing start_daemon logic in daemon.rs, adds vault password passing

#[tauri::command]
fn generate_qr_data(hub_url: String, auth_key: String, node_id: String) -> QrPayload
// Returns canonical QR payload: { type: "phantom-mesh-hub", version: 1, hub_url, auth_key, node_id }
// auth_key: 32-char random hex, generated if not provided
```

### 4.3 Mobile — New Files

```
mobile/phantom-mesh-worker-app/src/
  components/onboarding/
    OnboardingWizard.tsx    -- Main container, swipeable pages
    StepWelcome.tsx         -- Step 1
    StepConnect.tsx         -- Step 2: QR scan + manual input
    StepIdentity.tsx        -- Step 3
    StepComplete.tsx        -- Step 4
    types.ts                -- MobileOnboardingData type definition
```

### 4.4 State Management

**Desktop — `useWizardState()` hook:**
```typescript
interface OnboardingData {
  hardwareScan?: HardwareScanResult;
  vaultPassword?: string;        // In-memory only, NEVER persisted to localStorage
  providers: ProviderConfig[];    // { name, apiKey, validated, models[] }
  clusterEnabled: boolean;
  clusterNodes: string[];
  telegramToken?: string;
  qrPayload?: QrPayload;         // { type, version, hub_url, auth_key, node_id }
}

// Persistence rules:
// - currentStep + non-sensitive data → localStorage key: 'phantom-mesh_onboarding_state'
// - vaultPassword → in-memory ONLY (never persisted)
// - API keys → in-memory ONLY (never persisted)
// - On crash: resume from saved step; Step 2 (vault) must be re-entered
// - On completion: clear all localStorage wizard state
```

**Mobile:**
- Step progress persisted to AsyncStorage key: `@phantom-mesh_onboarding_step`
- Completion determined by existing `loadConfig()` logic — config exists = onboarded
- QR scan result: parsed via `expo-camera` barcode scanner

### 4.5 First-Run Detection

**Desktop App.tsx:**
```typescript
const [onboarded, setOnboarded] = useState(() =>
  localStorage.getItem('phantom-mesh_onboarded') === 'true'
);

if (!onboarded) return <OnboardingWizard onComplete={() => {
  localStorage.setItem('phantom-mesh_onboarded', 'true');
  setOnboarded(true);
}} />;
```

**Mobile App.tsx:**
- Existing pattern: `if (!config) → show SettingsScreen as full-screen fallback`
- Change: replace `SettingsScreen` full-screen fallback with `OnboardingWizard`
- `SetupScreen.tsx` is already unused in current App.tsx (older artifact, can be removed)

**Desktop App.tsx routing:**
- `OnboardingWizard` renders full-screen, replacing the entire sidebar + routes layout
- It is NOT nested inside the existing `<Routes>` — it replaces the entire `<div className="flex h-screen">` block
- On completion, the wizard sets state and the normal app layout renders

### 4.6 Generated `agents.toml` Template

The wizard writes `agents.toml` to `{app_data_dir}/agents.toml` using this template, populated from `OnboardingData`:

```toml
[core]
host = "0.0.0.0"
port = {wizard.available_port}       # from HardwareScanResult

# --- Providers (one block per wizard-configured provider) ---

# If Ollama was detected:
[providers.ollama]
type = "ollama"
url = "{wizard.ollama_endpoint}"     # default "http://localhost:11434"
default_model = "{first_model}"      # first model from detected list

# If OpenAI was configured:
[providers.openai]
type = "openai"
api_key_env = "OPENAI_API_KEY"       # references .env file

# (similar blocks for anthropic, gemini, groq, etc.)

# --- Default Agent ---
[agent.master]
provider = "{best_provider}"         # local if available, else first cloud
model = "{best_model}"               # auto-selected based on tier routing
tools = ["web_search", "http_request"]
instructions = "You are a helpful AI assistant."

# --- Auth ---
[auth]
bearer_token = "{wizard.auth_key}"   # 32-char hex, for mobile worker auth
```

The `.env` file is written alongside with API keys:
```
OPENAI_API_KEY=sk-...
ANTHROPIC_API_KEY=sk-ant-...
# (only keys the user provided)
```

**Merge policy:** If `agents.toml` already exists, the wizard backs it up to `agents.toml.bak` and writes a fresh file. Users are warned in Step 5 summary if an existing config is found.

---

## 5. Post-Onboarding Checklist

After the wizard completes, the Dashboard shows a dismissible banner with 2-4 remaining setup tasks.

### 5.1 Desktop Checklist Items

| Item | Condition | Link |
|------|-----------|------|
| ✓ 設定 KeyVault 密碼 | Always done (mandatory) | — |
| ✓ 新增至少一個 Provider | Always done (mandatory) | — |
| ○ 新增第二個 Provider（備援） | providers.length < 2 | /providers |
| ○ 連接一台 Mobile Worker | No mobile workers connected | /cluster |
| ○ 設定 Search API | No search API key configured | /providers |
| ○ 發送第一則 Chat 訊息 | No chat history | /chat |

### 5.2 Mobile Checklist Items

| Item | Condition | Link |
|------|-----------|------|
| ✓ 連接 Hub | Always done (mandatory) | — |
| ✓ 設定 Worker 名稱 | Always done (mandatory) | — |
| ○ 關閉 Android 電池最佳化 | Android only, not yet disabled | Settings tab |
| ○ 完成第一個任務 | tasksCompleted === 0 | Tasks tab |
| ○ 嘗試自然語言指令（如「顯示任務」） | No chat messages sent | Chat tab |

### 5.3 Checklist Behavior

- Maximum 4 items displayed at once (not overwhelming)
- Clicking an incomplete item navigates to the relevant page
- Checklist can be permanently dismissed (stored in localStorage / AsyncStorage)
- Auto-dismisses with celebration animation when all items complete
- Component: `<OnboardingChecklist />` embedded in Dashboard/HomeScreen

---

## 6. QR Code Bridge Protocol

Desktop and Mobile are connected via a simple QR code flow:

1. **Desktop Step 4** generates QR payload:
   ```json
   {
     "type": "phantom-mesh-hub",
     "version": 1,
     "hub_url": "http://192.168.1.100:7878",
     "auth_key": "generated-bearer-token",
     "node_id": "desktop-abc123"
   }
   ```

2. **Mobile Step 2** scans QR code:
   - Opens camera via `expo-camera` BarCodeScanner
   - Parses JSON payload
   - Validates `type === "phantom-mesh-hub"` and `version === 1`
   - Auto-fills hub_url and auth_key fields
   - Triggers automatic connection test

3. **Fallback:** Manual entry of hub URL + auth key (displayed as copyable text on Desktop)

---

## 7. Error Handling

| Scenario | Error Message | Recovery |
|----------|---------------|----------|
| Daemon binary not found | "找不到 phantom-mesh 執行檔" | Manual path input field |
| Port 7878 in use | "Port 7878 已被佔用，改用 {next}" | Auto-switch to next available |
| Ollama not running | "未偵測到 Ollama" | Skip (use cloud providers) |
| Invalid API key | "API Key 驗證失敗: 401" | Re-enter key |
| Hub unreachable (Mobile) | "無法連線 {url} — 請確認 IP" | Retry with corrected URL |
| Passwords don't match | "密碼不一致" | Highlight mismatch |
| Weak password (< 8 chars) | "密碼至少需要 8 個字元" | Block proceed |
| QR code parse error | "無法辨識 QR Code 內容" | Fall back to manual input |
| Daemon health check fail | "Daemon 啟動失敗，請查看日誌" | Show log path, retry button |

---

## 8. Dependencies

### Desktop (new npm packages)
- `qrcode.react` — QR code rendering (Step 4)
- No new Rust crates needed (reuses existing: sysinfo, reqwest, serde)

### Mobile (new npm packages)
- `expo-camera` — already in dependencies (`"expo-camera": "~16.0.0"`), used for QR code scanning (Step 2)
- No other new dependencies needed

---

## 9. Scope Boundaries

### In Scope
- Desktop 5-step wizard
- Mobile 4-step wizard
- QR code bridge between Desktop and Mobile
- Post-onboarding checklist on Dashboard
- Crash recovery (state persistence)
- All Tauri commands for backend operations

### Out of Scope
- Feature tours / tooltip walkthroughs (future enhancement)
- agents.toml visual editor (use existing Settings page)
- Multi-language wizard (use existing i18n infrastructure if available)
- Automated testing of wizard flow (manual verification)

---

## 10. File Impact Summary

### New Files (Desktop)
| File | Purpose | Estimated LOC |
|------|---------|---------------|
| `src/components/onboarding/OnboardingWizard.tsx` | Main wizard container | ~120 |
| `src/components/onboarding/WizardProgress.tsx` | Progress dots | ~40 |
| `src/components/onboarding/StepWelcome.tsx` | Step 1 | ~100 |
| `src/components/onboarding/StepSecurity.tsx` | Step 2 | ~90 |
| `src/components/onboarding/StepProviders.tsx` | Step 3 | ~180 |
| `src/components/onboarding/StepNetwork.tsx` | Step 4 | ~160 |
| `src/components/onboarding/StepComplete.tsx` | Step 5 | ~120 |
| `src/components/onboarding/useHardwareScan.ts` | Hardware detection hook | ~60 |
| `src/components/onboarding/useWizardState.ts` | State management hook | ~80 |
| `src/components/onboarding/types.ts` | Type definitions | ~50 |
| `src/components/OnboardingChecklist.tsx` | Post-wizard checklist | ~100 |
| `src-tauri/src/commands/onboarding.rs` | Tauri backend commands (HW scan, API validation, config gen, daemon launch) | ~350 |

### New Files (Mobile)
| File | Purpose | Estimated LOC |
|------|---------|---------------|
| `src/components/onboarding/OnboardingWizard.tsx` | Main wizard container | ~80 |
| `src/components/onboarding/StepWelcome.tsx` | Step 1 | ~70 |
| `src/components/onboarding/StepConnect.tsx` | Step 2 (QR + manual) | ~150 |
| `src/components/onboarding/StepIdentity.tsx` | Step 3 | ~100 |
| `src/components/onboarding/StepComplete.tsx` | Step 4 | ~120 |
| `src/components/onboarding/types.ts` | Type definitions | ~30 |
| `src/components/OnboardingChecklist.tsx` | Post-wizard checklist | ~80 |

### Modified Files
| File | Change |
|------|--------|
| `phantom-mesh-desktop/src/App.tsx` | Add first-run detection + OnboardingWizard gate |
| `phantom-mesh-desktop/src/pages/Dashboard.tsx` | Add OnboardingChecklist component |
| `phantom-mesh-desktop/src-tauri/src/main.rs` | Register onboarding commands in invoke_handler |
| `mobile/phantom-mesh-worker-app/src/App.tsx` | Replace SettingsScreen fallback with OnboardingWizard |
| `mobile/phantom-mesh-worker-app/src/screens/HomeScreen.tsx` | Add OnboardingChecklist component |

### Total Estimated
- Desktop: ~1,450 LOC (11 new TS files + 1 Rust file)
- Mobile: ~630 LOC (7 new TS files)
- Modified: 5 existing files
- **Grand total: ~2,080 LOC**
