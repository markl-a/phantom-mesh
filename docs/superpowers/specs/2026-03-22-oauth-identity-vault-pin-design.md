# OAuth Identity + Vault PIN Redesign

## Goal

Replace the vault master password in `StepSecurity` with a two-layer architecture: **OAuth identity** (Google/Apple Sign-In) for user identification and cross-device sync, plus a **6-digit PIN** for vault encryption. Desktop and Mobile both get OAuth support.

## Architecture

Two-layer separation:

| Layer | Mechanism | Purpose |
|-------|-----------|---------|
| **Identity** | Google / Apple OAuth2 PKCE | Hub ownership, cross-device sync, pairing verification |
| **Encryption** | 6-digit PIN → Argon2id (hardened) → AES-256-GCM | Vault encryption/decryption, daemon startup, offline operation |

The PIN encrypts the vault locally. The OAuth identity proves "who owns this hub" for sync and pairing. Neither layer depends on the other for its core function.

**Desktop wizard remains at 5 steps (0–4).** OAuth + PIN are integrated into the existing StepSecurity (step 1), not a separate step.

## OAuth Flow (Desktop)

Authorization Code + PKCE via localhost redirect. No deep-link dependency.

```
User clicks "Sign in with Google"
  → Tauri generates PKCE code_verifier + state
  → Tauri binds a temporary localhost:{random_port} HTTP server
  → Tauri opens system browser to Google OAuth URL
  → User authenticates in browser
  → Google redirects to http://localhost:{port}/callback?code=...&state=...
  → Tauri receives auth code, shuts down temp server
  → Tauri exchanges code + code_verifier for id_token
  → Tauri verifies id_token JWT signature against provider JWKS
  → Tauri parses verified id_token → UserIdentity
  → Returns to frontend
```

### OAuth Provider Configuration

| Provider | Auth URL | Token URL | Scope | Client Secret |
|----------|----------|-----------|-------|---------------|
| Google | `accounts.google.com/o/oauth2/v2/auth` | `oauth2.googleapis.com/token` | `openid email profile` | Not required (public client) |
| Apple | `appleid.apple.com/auth/authorize` | `appleid.apple.com/auth/token` | `openid email name` | Required (signed JWT from Apple private key) |

Both use PKCE. Redirect URI: `http://localhost:{random_port}/callback`.

**Apple Sign-In asymmetry:** Apple requires a `client_secret` JWT even with PKCE. The Rust backend must generate this JWT on-the-fly using the Apple private key (`.p8` file). The key file path is configured in the app settings. Google does not require a client_secret (true public client).

Requirements:
- Google: OAuth client in GCP Console (Desktop app type)
- Apple: Service ID + private key (.p8) in Apple Developer Portal

### id_token Verification

The `id_token` JWT **must be cryptographically verified**, not just base64-decoded:

1. Fetch provider's JWKS (Google: `googleapis.com/oauth2/v3/certs`, Apple: `appleid.apple.com/auth/keys`)
2. Verify JWT signature against JWKS public key
3. Validate `aud` matches our client_id
4. Validate `exp` is not expired
5. Validate `iss` matches expected issuer

JWKS keys are cached in memory with 24h TTL (they rotate infrequently).

### Tauri Command

```rust
#[tauri::command]
async fn oauth_sign_in(provider: String) -> Result<UserIdentity, String>
```

Internally: generate PKCE → bind localhost server → open browser → await callback → exchange token → verify & parse id_token.

### Error Handling & Timeouts

| Scenario | Behavior |
|----------|----------|
| User closes browser without completing OAuth | Localhost server times out after **120 seconds**, returns `Err("OAuth timeout: no callback received")` |
| Localhost port already in use | Try up to 10 random ports (49152–65535), fail with `Err("Cannot bind callback server")` |
| Token exchange fails (network error) | Return `Err("Token exchange failed: {details}")` |
| id_token verification fails | Return `Err("Invalid identity token: {reason}")` |
| JWKS fetch fails | Return `Err("Cannot fetch provider keys")`, user can retry |

Frontend shows an error message with a "Retry" button for all failure cases.

### Data Structures

```typescript
interface UserIdentity {
  provider: 'google' | 'apple';
  sub: string;            // Stable unique ID from OAuth provider
  email: string;
  display_name: string;
  avatar_url: string | null;
}
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserIdentity {
    pub provider: String,     // "google" | "apple"
    pub sub: String,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
}
```

### Identity Persistence

After successful OAuth sign-in during onboarding:

1. **During onboarding**: `UserIdentity` is held in React state (in-memory), passed through wizard steps
2. **After onboarding completes** (`StepComplete`): Identity is persisted to **Tauri Store** (`tauri-plugin-store`) at key `"user_identity"` — this is a JSON file managed by the Tauri Store plugin, stored in the app data directory
3. **Post-onboarding access**: Any component or Tauri command can read the identity via the Store plugin
4. **No tokens stored**: Only the `UserIdentity` profile is persisted. OAuth access/refresh tokens are discarded after extracting the identity. Re-authentication requires a new OAuth flow.
5. **Primary Hub**: The daemon also receives the `sub` and `provider` via the config file (`agents.toml` under `[identity]`) so it can verify sync requests without needing the Tauri Store

```toml
# agents.toml — written by StepComplete
[identity]
provider = "google"
sub = "1234567890"
email = "user@gmail.com"
```

## StepSecurity UI (Desktop)

Two sections in one step (step 1 of 5, unchanged step count):

**Upper section: OAuth Sign-In**
- "Sign in with Google" button
- "Sign in with Apple" button
- After sign-in: shows "Signed in as: user@gmail.com" with avatar

**Lower section: Vault PIN**
- 6 individual digit input boxes (numeric only)
- Confirm PIN (6 boxes)
- Warning: "PIN 遺失後需重新設定，已加密的資料將無法恢復。" (accurate: PIN loss = vault data loss)

**Proceed condition:**
```typescript
const canProceed = identity !== null && pin.length === 6 && pin === confirmPin;
```

## PIN Security: Mandatory Argon2id Hardening

A 6-digit PIN has only 10^6 (1,000,000) possible values. The current `key_vault.rs` uses `Argon2::default()` which allows GPU attackers to exhaust the PIN space in minutes.

**Mandatory change to `key_vault.rs`:**

```rust
// OLD (default): m=19456 KiB, t=2, p=1 — INSECURE for 6-digit PIN
let argon2 = Argon2::default();

// NEW (hardened): m=256 MiB, t=4, p=2 — ~0.5s per hash, ~5.8 days to brute-force
let params = argon2::Params::new(262_144, 4, 2, Some(32))
    .map_err(|e| format!("Argon2 params: {}", e))?;
let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
```

Additionally, add application-level rate limiting:
- **5 failed PIN attempts → 30 second lockout**
- **10 failed PIN attempts → 5 minute lockout**
- Failed attempt counter persisted to Tauri Store (survives app restart)

## Vault PIN Delivery to Daemon

**Problem:** The current `launch_daemon` passes the vault password as a CLI argument (`--vault-password`), which is visible in process listing (`ps aux` / Task Manager).

**Fix:** Pass the PIN via **stdin pipe** instead of CLI argument.

```rust
// OLD (insecure): visible in process listing
Command::new(&binary_path)
    .arg("--vault-password")
    .arg(&vault_pin)
    .spawn()

// NEW (secure): passed via stdin, not visible in process listing
let mut child = Command::new(&binary_path)
    .arg("--vault-password-stdin")
    .stdin(Stdio::piped())
    .spawn()?;

child.stdin.take().unwrap().write_all(vault_pin.as_bytes())?;
```

The daemon reads from stdin when `--vault-password-stdin` flag is present. Falls back to `--vault-password` for manual CLI usage (dev/debug).

## OnboardingData Changes (Desktop)

```typescript
interface OnboardingData {
  hardwareScan: HardwareScanResult | null;
  identity: UserIdentity | null;    // NEW: OAuth identity
  vaultPin: string;                 // RENAMED: was vaultPassword
  providers: ProviderConfig[];
  clusterEnabled: boolean;
  clusterNodes: string[];
  telegramToken: string;
  qrPayload: QrPayload | null;
  ollamaEndpoint: string;
  ollamaEnabled: boolean;
}
```

Removed: `vaultPassword: string`

### Downstream Impact

| Location | Change |
|----------|--------|
| `StepComplete.tsx` | `vaultPassword` → `vaultPin` in `launch_daemon` call |
| `launch_daemon` command (Rust) | Use `--vault-password-stdin` + stdin pipe |
| `key_vault.rs` (phantom-mesh) | **Mandatory:** increase Argon2id to m=256MiB, t=4, p=2 |
| `phantom-mesh main.rs` | Add `--vault-password-stdin` flag, read from stdin |
| `useWizardState.ts` | Crash recovery resets to step 1 (re-enter PIN, but persist identity so OAuth is skipped if already done) |
| `PersistedWizardState` | Add `identityEmail?: string` and `identityProvider?: string` |

## Cross-Device Sync

### Model

User designates one machine as "Primary Hub". Other devices pull configuration from it.

```
        ┌───────────────┐
        │  Primary Hub   │  ← User-designated
        │  (Desktop A)   │
        └──┬─────────┬──┘
           │         │
      sync pull   sync pull
           │         │
    ┌──────▼──┐  ┌──▼────────┐
    │ Desktop B│  │  Mobile    │
    └─────────┘  └───────────┘
```

### Primary Hub Designation

- **Where stored:** `agents.toml` under `[sync]` section:
  ```toml
  [sync]
  is_primary = true
  ```
- **UI:** In `StepComplete` (or future Settings page), a toggle: "設定此裝置為主 Hub"
- **Default:** The first device to complete onboarding is automatically set as Primary
- **Can be changed later** via Settings page (future scope, not this spec)

### What Syncs

| Data | Sync? | Reason |
|------|-------|--------|
| Provider list (names, types) | Yes | Secondary nodes need provider info |
| API Keys (encrypted) | Yes | Wrapped with one-time shared secret during transfer |
| Cluster node list | Yes | New devices auto-join cluster |
| Telegram token | Yes | Shared notification config |
| Identity (OAuth sub) | Yes | Verify "same person" |
| Ollama config | No | Each machine has different Ollama setup |
| Hardware scan | No | Each machine has different hardware |
| Vault PIN | No | Each device has independent PIN |

### Sync auth_key Lifecycle

| Question | Answer |
|----------|--------|
| Where does `auth_key` come from? | Generated during onboarding by the Primary Hub (stored in `agents.toml [auth]`) |
| Distribution to mobile? | Via QR code (existing `QrPayload.auth_key`) |
| Distribution to secondary desktop? | Via QR code displayed in Settings, or manually copied from Primary's `agents.toml` |
| Lifetime? | Permanent until user regenerates it in Settings |
| Revocation? | User generates a new `auth_key` in Settings; old one stops working immediately |

### Sync Protocol

```
Secondary Node                          Primary Hub
  │                                       │
  │ 1. GET /sync/identity                 │
  │──────────────────────────────────────→│
  │     { sub, provider }         ←───────│
  │                                       │
  │ 2. POST /sync/pull                    │
  │    Authorization: Bearer auth_key     │
  │    { requester_sub: "..." }           │
  │──────────────────────────────────────→│
  │                                       │  ← Verify same sub
  │     {                         ←───────│
  │       providers: [...],               │
  │       encrypted_keys: "base64...",    │
  │       cluster_nodes: [...],           │
  │       telegram_token: "..."           │
  │     }                                 │
  │                                       │
  │ 3. Secondary decrypts with shared     │
  │    secret, re-encrypts with own PIN   │
```

### Security

- Sync is LAN-only (`http://{hub_ip}:{port}/sync/...`)
- Primary Hub only responds to requests where `requester_sub` matches its own OAuth `sub`
- API Keys are wrapped with a one-time shared secret (distributed via QR pairing)
- Bearer token (`auth_key`) required for all sync endpoints
- No public internet exposure, no TLS required (LAN)

### Scope

This spec defines:
1. Desktop/Mobile frontend: identity storage, Primary Hub toggle in StepComplete
2. Tauri backend: `oauth_sign_in` command, identity persistence to Tauri Store + agents.toml
3. Interface contract: `/sync/pull` request/response schema

The daemon-side `/sync/pull` endpoint implementation is a future task, not part of this onboarding spec.

## Mobile Integration

### Dependencies

- `expo-auth-session` — OAuth PKCE flow
- `expo-apple-authentication` — Native Apple Sign-In on iOS
- `expo-crypto` — PKCE code verifier generation (if not bundled)

### Mobile Onboarding Flow Change

Current: 4 steps (Welcome → Connect → Identity → Complete)
New: 5 steps (Welcome → **Auth** → Connect → Identity → Complete)

All mobile step labels must be updated to reflect the new total of 5 steps (e.g., "Step 2 / 4" → "Step 2 / 5").

### StepAuth (Mobile, new)

- "Sign in with Google" button (uses `expo-auth-session/providers/google`)
- "Sign in with Apple" button (uses `expo-apple-authentication` on iOS, web fallback on Android)
- After sign-in: shows email, proceed enabled
- No PIN on mobile (mobile is a worker, doesn't hold vault)

### MobileOnboardingData Change

```typescript
interface MobileOnboardingData {
  identity: UserIdentity | null;  // NEW
  hubUrl: string;
  authKey: string;
  workerName: string;
  agentName: string;              // NOTE: existing field name, not "selectedAgent"
  connectionTested: boolean;
}
```

### QR Pairing Enhancement

After QR scan in `StepConnect`, the mobile app sends its OAuth `sub` in the connection test header:

```typescript
const resp = await fetch(`${hubUrl}/health`, {
  headers: {
    'Authorization': `Bearer ${authKey}`,
    'X-Phantom Mesh-Sub': identity.sub,
  },
});
```

The Primary Hub can optionally verify `X-Phantom Mesh-Sub` matches its own identity, rejecting unknown users. This is an optional security hardening, not mandatory.

## CSP Update (Desktop)

`tauri.conf.json` `connect-src` needs to allow OAuth token exchange from the Rust backend. Since `validate_api_key` and `oauth_sign_in` both run in Rust (not webview), no CSP change is needed — the webview only calls Tauri commands via IPC, not direct HTTP.

## File Impact Summary

### New Files
| File | Purpose |
|------|---------|
| `src-tauri/src/commands/oauth.rs` | OAuth PKCE flow, localhost callback server (`tiny_http`), token exchange, JWKS verification |
| `mobile/.../onboarding/StepAuth.tsx` | Mobile OAuth sign-in step |

### Modified Files
| File | Change |
|------|--------|
| `src-tauri/src/commands/mod.rs` | Add `pub mod oauth;` |
| `src-tauri/src/main.rs` | Register `oauth_sign_in` command |
| `src-tauri/Cargo.toml` | Add `jsonwebtoken` (id_token verification), `base64`, `tiny_http` (callback server) |
| `src/components/onboarding/StepSecurity.tsx` | Full rewrite: OAuth buttons + PIN input |
| `src/components/onboarding/types.ts` | Add `UserIdentity`, change `vaultPassword` → `vaultPin` |
| `src/components/onboarding/useWizardState.ts` | Update initial data, persist identity info |
| `src/components/onboarding/StepComplete.tsx` | `vaultPassword` → `vaultPin`, add Primary Hub toggle, write identity to agents.toml |
| `src/components/onboarding/OnboardingWizard.tsx` | Pass identity to steps |
| `phantom-mesh/src/security/key_vault.rs` | **Mandatory:** increase Argon2id params to m=256MiB, t=4, p=2 |
| `phantom-mesh/src/main.rs` | Add `--vault-password-stdin` flag |
| `phantom-mesh-desktop/src-tauri/src/commands/onboarding.rs` | `launch_daemon`: use stdin pipe for PIN |
| `mobile/.../onboarding/types.ts` | Add `UserIdentity`, add `identity` field |
| `mobile/.../onboarding/OnboardingWizard.tsx` | Add StepAuth as step 1, shift others, update step labels |
| `mobile/.../onboarding/StepConnect.tsx` | Add `X-Phantom Mesh-Sub` header |
| `mobile/package.json` | Add `expo-auth-session`, `expo-apple-authentication` |

### Not Modified (This Spec)
| File | Reason |
|------|--------|
| Daemon sync endpoints (`/sync/pull`, `/sync/identity`) | Future task, only interface contract defined here |
