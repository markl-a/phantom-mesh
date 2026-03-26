# OAuth Identity + Vault PIN Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace vault master password with Google/Apple OAuth identity + 6-digit PIN across Desktop and Mobile.

**Architecture:** Two-layer separation — OAuth for identity (PKCE via localhost redirect), 6-digit PIN for Argon2id-based vault encryption. Desktop uses Tauri command `oauth_sign_in` with `tiny_http` callback server and JWKS-based id_token verification via `jsonwebtoken`. Mobile uses `expo-auth-session`. Cross-device sync interface defined but daemon endpoint deferred.

**Tech Stack:** Rust (Tauri v2, tiny_http, jsonwebtoken, reqwest, sha2), React/TypeScript, React Native/Expo (expo-auth-session, expo-apple-authentication)

**Spec:** `phantom-mesh/docs/superpowers/specs/2026-03-22-oauth-identity-vault-pin-design.md`

---

## File Structure

### New Files
| File | Responsibility |
|------|---------------|
| `phantom-mesh-desktop/src-tauri/src/commands/oauth.rs` | OAuth PKCE flow, localhost callback server, token exchange, JWKS verification |
| `mobile/phantom-mesh-worker-app/src/components/onboarding/StepAuth.tsx` | Mobile OAuth sign-in step |

### Modified Files
| File | Change |
|------|--------|
| `phantom-mesh-desktop/src-tauri/Cargo.toml` | Add `jsonwebtoken`, `base64`, `tiny_http`, `sha2`, `open`, `urlencoding` |
| `phantom-mesh-desktop/src-tauri/src/commands/mod.rs` | Add `pub mod oauth;` |
| `phantom-mesh-desktop/src-tauri/src/main.rs` | Register `oauth_sign_in` command |
| `phantom-mesh-desktop/src/components/onboarding/types.ts` | Add `UserIdentity`, rename `vaultPassword` → `vaultPin` |
| `phantom-mesh-desktop/src/components/onboarding/StepSecurity.tsx` | Full rewrite: OAuth buttons + PIN input |
| `phantom-mesh-desktop/src/components/onboarding/StepComplete.tsx` | `vaultPassword` → `vaultPin`, identity to Tauri Store + agents.toml, Primary Hub toggle |
| `phantom-mesh-desktop/src/components/onboarding/useWizardState.ts` | Update initial data, persist identity info, crash recovery with identity |
| `phantom-mesh-desktop/src/components/onboarding/OnboardingWizard.tsx` | Pass identity to StepSecurity |
| `phantom-mesh-desktop/src-tauri/src/commands/onboarding.rs` | `launch_daemon`: stdin pipe for PIN; `write_config`: identity + sync sections |
| `phantom-mesh/src/security/key_vault.rs` | Argon2id hardened params |
| `phantom-mesh/src/main.rs` | Add `--vault-password-stdin` flag to `Daemon` subcommand |
| `mobile/phantom-mesh-worker-app/package.json` | Add expo-auth-session, expo-apple-authentication |
| `mobile/phantom-mesh-worker-app/src/components/onboarding/types.ts` | Add `UserIdentity`, add `identity` field |
| `mobile/phantom-mesh-worker-app/src/components/onboarding/OnboardingWizard.tsx` | Insert StepAuth as step 1, `TOTAL_STEPS = 5` |
| `mobile/phantom-mesh-worker-app/src/components/onboarding/StepConnect.tsx` | Add `X-Phantom Mesh-Sub` header, update step label to `步驟 3 / 5` |
| `mobile/phantom-mesh-worker-app/src/components/onboarding/StepIdentity.tsx` | Update step label to `步驟 4 / 5` |
| `mobile/phantom-mesh-worker-app/src/components/onboarding/StepComplete.tsx` | Update step label to `步驟 5 / 5` |

### Deferred (Acknowledged in Spec, Not This Plan)
| Item | Reason |
|------|--------|
| PIN rate limiting (5 failed → 30s, 10 → 5min lockout) | Applies to post-onboarding vault unlock screen. During onboarding, user is *setting* a new PIN, not verifying an existing one. Rate limiting will be implemented when the vault unlock UI is built. |
| Apple client_secret JWT generation from .p8 key | Requires Apple Developer Portal setup + .p8 private key. Google Sign-In works immediately as a public client. Apple Sign-In is structurally complete but the .p8 key generation is marked TODO — will fail at runtime until key is configured. |
| Daemon sync endpoints (`/sync/pull`, `/sync/identity`) | Spec defines interface contract only; implementation is a future task. |

---

### Task 1: Add Rust Dependencies

**Files:**
- Modify: `phantom-mesh-desktop/src-tauri/Cargo.toml`

- [ ] **Step 1: Add OAuth-related dependencies to Cargo.toml**

Add under `[dependencies]`:

```toml
jsonwebtoken = "9"
base64 = "0.22"
tiny_http = "0.12"
sha2 = "0.10"
open = "5"
urlencoding = "2"
```

Check if `rand` is already present. If not, add `rand = "0.8"`.

- [ ] **Step 2: Verify it compiles**

Run: `cd phantom-mesh-desktop/src-tauri && CARGO_TARGET_DIR="C:/tmp/desktop-target" cargo check`
Expected: Finishes successfully (new crates downloaded)

- [ ] **Step 3: Commit**

```bash
git add phantom-mesh-desktop/src-tauri/Cargo.toml
git commit -m "chore: add jsonwebtoken, base64, tiny_http, sha2, open, urlencoding for OAuth support"
```

---

### Task 2: Harden Argon2id Parameters in KeyVault

**Files:**
- Modify: `phantom-mesh/src/security/key_vault.rs:172-178`

- [ ] **Step 1: Read the current derive_key function**

Read `phantom-mesh/src/security/key_vault.rs` and find the `derive_key` function (around line 172-178). It currently uses `Argon2::default()`.

- [ ] **Step 2: Replace with hardened parameters**

Change the `derive_key` function from:

```rust
fn derive_key(password: &str, salt: &[u8; SALT_SIZE]) -> KeyVaultResult<[u8; KEY_SIZE]> {
    let mut output = [0u8; KEY_SIZE];
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut output)
        .map_err(|e| KeyVaultError::Argon2(e.to_string()))?;
    Ok(output)
}
```

To:

```rust
fn derive_key(password: &str, salt: &[u8; SALT_SIZE]) -> KeyVaultResult<[u8; KEY_SIZE]> {
    let mut output = [0u8; KEY_SIZE];
    // Hardened for 6-digit PIN: m=256MiB, t=4, p=2 (~0.5s/hash, ~5.8 days brute-force)
    let params = argon2::Params::new(262_144, 4, 2, Some(KEY_SIZE))
        .map_err(|e| KeyVaultError::Argon2(e.to_string()))?;
    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut output)
        .map_err(|e| KeyVaultError::Argon2(e.to_string()))?;
    Ok(output)
}
```

Note: `Params::new` fourth parameter is `Option<usize>`, so pass `Some(KEY_SIZE)` directly (no `as u32` cast).

- [ ] **Step 3: Verify existing tests still pass**

Run: `cd phantom-mesh && CARGO_TARGET_DIR=target_oauth cargo test key_vault`
Expected: All key_vault tests pass (they test encrypt/decrypt roundtrip, not specific params)

- [ ] **Step 4: Commit**

```bash
git add phantom-mesh/src/security/key_vault.rs
git commit -m "security: harden Argon2id params for 6-digit PIN (m=256MiB, t=4, p=2)"
```

---

### Task 3: Add --vault-password-stdin to Daemon

**Files:**
- Modify: `phantom-mesh/src/main.rs`

- [ ] **Step 1: Read main.rs and find the Daemon subcommand**

Read `phantom-mesh/src/main.rs`. Find the `Command` enum (around line 87-100). The `Daemon` variant is currently a unit variant with no fields:

```rust
#[derive(Subcommand, Debug)]
enum Command {
    /// Start the daemon (default if no subcommand given)
    Daemon,
    ...
}
```

- [ ] **Step 2: Add vault password fields to Daemon subcommand**

Change the `Daemon` variant from a unit variant to a struct variant:

```rust
/// Start the daemon (default if no subcommand given)
Daemon {
    /// Vault password (insecure: visible in process listing, use --vault-password-stdin)
    #[arg(long)]
    vault_password: Option<String>,

    /// Read vault password from stdin instead of CLI argument
    #[arg(long)]
    vault_password_stdin: bool,
},
```

- [ ] **Step 3: Add stdin reading logic in the daemon startup match arm**

Find the `match` on `args.command` where `Command::Daemon` is handled (or `None` default). Add vault password resolution:

```rust
Some(Command::Daemon { vault_password, vault_password_stdin }) | None => {
    let vault_pw = if vault_password_stdin {
        use std::io::BufRead;
        let stdin = std::io::stdin();
        let mut line = String::new();
        stdin.lock().read_line(&mut line)
            .map_err(|e| anyhow::anyhow!("Failed to read vault password from stdin: {}", e))?;
        Some(line.trim().to_string())
    } else {
        vault_password
    };

    // ... existing daemon startup code continues here, using vault_pw where needed ...
}
```

If the match arm currently uses `Some(Command::Daemon)` (unit variant), update it to destructure the new fields.

- [ ] **Step 4: Verify it compiles**

Run: `cd phantom-mesh && CARGO_TARGET_DIR=target_oauth cargo check`
Expected: compiles

- [ ] **Step 5: Commit**

```bash
git add phantom-mesh/src/main.rs
git commit -m "feat: add --vault-password-stdin flag for secure PIN delivery"
```

---

### Task 4: Update Desktop TypeScript Types

**Files:**
- Modify: `phantom-mesh-desktop/src/components/onboarding/types.ts`

- [ ] **Step 1: Read current types.ts**

Read `phantom-mesh-desktop/src/components/onboarding/types.ts`.

- [ ] **Step 2: Add UserIdentity interface**

Add before the `OnboardingData` interface:

```typescript
export interface UserIdentity {
  provider: 'google' | 'apple';
  sub: string;
  email: string;
  display_name: string;
  avatar_url: string | null;
}
```

- [ ] **Step 3: Update OnboardingData**

Change in `OnboardingData`:
```typescript
// OLD
vaultPassword: string;          // In-memory only, never persisted

// NEW
identity: UserIdentity | null;  // OAuth identity (in-memory during wizard)
vaultPin: string;               // 6-digit PIN, in-memory only, never persisted
```

- [ ] **Step 4: Update PersistedWizardState**

Add to `PersistedWizardState`:

```typescript
identityEmail?: string;
identityProvider?: string;
```

- [ ] **Step 5: Verify TypeScript compiles (expect errors in consumers — that's OK for now)**

Run: `cd phantom-mesh-desktop && npx tsc --noEmit 2>&1 | head -20`
Expected: Errors in StepSecurity.tsx, StepComplete.tsx, useWizardState.ts (they reference `vaultPassword`). This is expected — we'll fix them in subsequent tasks.

- [ ] **Step 6: Commit**

```bash
git add phantom-mesh-desktop/src/components/onboarding/types.ts
git commit -m "feat: add UserIdentity type, rename vaultPassword to vaultPin"
```

---

### Task 5: Implement OAuth PKCE Command (Rust)

**Files:**
- Create: `phantom-mesh-desktop/src-tauri/src/commands/oauth.rs`
- Modify: `phantom-mesh-desktop/src-tauri/src/commands/mod.rs`
- Modify: `phantom-mesh-desktop/src-tauri/src/main.rs`

- [ ] **Step 1: Read mod.rs and main.rs to find insertion points**

Read `phantom-mesh-desktop/src-tauri/src/commands/mod.rs` — find where other `pub mod` declarations are.
Read `phantom-mesh-desktop/src-tauri/src/main.rs` — find the `tauri::generate_handler![]` macro call.

- [ ] **Step 2: Create oauth.rs with data structures and PKCE utilities**

Create `phantom-mesh-desktop/src-tauri/src/commands/oauth.rs`:

```rust
use base64::Engine;
use serde::Serialize;
use sha2::Digest;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize)]
pub struct UserIdentity {
    pub provider: String,
    pub sub: String,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
}

// ── PKCE Utilities ─────────────────────────────────────────

/// Generate PKCE code_verifier (43-128 chars, URL-safe)
fn generate_code_verifier() -> String {
    let mut random_bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut random_bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes)
}

/// Derive code_challenge from code_verifier (S256)
fn generate_code_challenge(verifier: &str) -> String {
    let digest = sha2::Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// Generate a random state parameter for CSRF protection
fn generate_state() -> String {
    let mut random_bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut random_bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes)
}
```

- [ ] **Step 3: Add the localhost callback server**

Append to `oauth.rs`:

```rust
// ── Localhost Callback Server ──────────────────────────────

/// Bind a temporary localhost HTTP server, return (port, receiver for callback params)
fn start_callback_server() -> Result<(u16, std::sync::mpsc::Receiver<(String, String)>), String> {
    let (tx, rx) = std::sync::mpsc::channel();

    // Try random ports in ephemeral range
    let mut server = None;
    for _ in 0..10 {
        let port = 49152 + (rand::random::<u16>() % 16383);
        match tiny_http::Server::http(format!("127.0.0.1:{}", port)) {
            Ok(s) => { server = Some((s, port)); break; }
            Err(_) => continue,
        }
    }

    let (srv, port) = server.ok_or("Cannot bind callback server")?;

    std::thread::spawn(move || {
        // Wait for one request with timeout
        let request = match srv.recv_timeout(Duration::from_secs(120)) {
            Ok(Some(req)) => req,
            _ => { let _ = tx.send(("".into(), "timeout".into())); return; }
        };

        let url = request.url().to_string();

        // Parse query params: /callback?code=XXX&state=YYY
        let params: HashMap<String, String> = url
            .split('?').nth(1).unwrap_or("")
            .split('&')
            .filter_map(|pair| {
                let mut kv = pair.splitn(2, '=');
                Some((kv.next()?.to_string(), kv.next().unwrap_or("").to_string()))
            })
            .collect();

        let code = params.get("code").cloned().unwrap_or_default();
        let state = params.get("state").cloned().unwrap_or_default();

        // Respond with success page
        let response = tiny_http::Response::from_string(
            "<html><body><h1>登入成功！</h1><p>你可以關閉此頁面，回到 Phantom Mesh。</p></body></html>"
        ).with_header("Content-Type: text/html; charset=utf-8".parse::<tiny_http::Header>().unwrap());
        let _ = request.respond(response);

        let _ = tx.send((code, state));
    });

    Ok((port, rx))
}
```

- [ ] **Step 4: Add JWKS fetching, caching, and id_token verification**

Append to `oauth.rs`:

```rust
// ── JWKS Verification ──────────────────────────────────────

/// Parsed JWK key from provider's JWKS endpoint
#[derive(Clone)]
struct JwkKey {
    kid: String,
    n: String,
    e: String,
}

/// JWKS cache: provider -> (keys, fetched_at). 24h TTL per spec.
static JWKS_CACHE: Mutex<Option<HashMap<String, (Vec<JwkKey>, Instant)>>> = Mutex::new(None);

/// Fetch JWKS from provider, with 24h caching
async fn get_jwks(provider: &str) -> Result<Vec<JwkKey>, String> {
    // Check cache
    {
        if let Ok(cache) = JWKS_CACHE.lock() {
            if let Some(ref map) = *cache {
                if let Some((keys, fetched_at)) = map.get(provider) {
                    if fetched_at.elapsed() < Duration::from_secs(86400) {
                        return Ok(keys.clone());
                    }
                }
            }
        }
    }

    let url = match provider {
        "google" => "https://www.googleapis.com/oauth2/v3/certs",
        "apple" => "https://appleid.apple.com/auth/keys",
        _ => return Err(format!("Unknown provider: {}", provider)),
    };

    let client = reqwest::Client::new();
    let resp = client.get(url).send().await
        .map_err(|_| "Cannot fetch provider keys".to_string())?;

    let body: serde_json::Value = resp.json().await
        .map_err(|_| "Cannot fetch provider keys".to_string())?;

    let keys: Vec<JwkKey> = body["keys"].as_array()
        .ok_or("Invalid JWKS format")?
        .iter()
        .filter_map(|k| Some(JwkKey {
            kid: k["kid"].as_str()?.to_string(),
            n: k["n"].as_str()?.to_string(),
            e: k["e"].as_str()?.to_string(),
        }))
        .collect();

    // Update cache
    if let Ok(mut cache) = JWKS_CACHE.lock() {
        let map = cache.get_or_insert_with(HashMap::new);
        map.insert(provider.to_string(), (keys.clone(), Instant::now()));
    }

    Ok(keys)
}

/// Verify id_token JWT signature and claims, return UserIdentity
fn verify_id_token(provider: &str, token: &str, jwks: &[JwkKey]) -> Result<UserIdentity, String> {
    use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};

    // 1. Decode header to get kid
    let header = decode_header(token)
        .map_err(|e| format!("Invalid identity token: {}", e))?;
    let kid = header.kid
        .ok_or("Invalid identity token: missing kid")?;

    // 2. Find matching key in JWKS
    let jwk = jwks.iter()
        .find(|k| k.kid == kid)
        .ok_or(format!("Invalid identity token: no key for kid {}", kid))?;

    // 3. Build decoding key from RSA components
    let decoding_key = DecodingKey::from_rsa_components(&jwk.n, &jwk.e)
        .map_err(|e| format!("Invalid identity token: bad key: {}", e))?;

    // 4. Validate aud (client_id), iss (provider), and exp
    let client_id_env = match provider {
        "google" => "PHANTOM_MESH_GOOGLE_CLIENT_ID",
        "apple" => "PHANTOM_MESH_APPLE_CLIENT_ID",
        _ => return Err(format!("Unknown provider: {}", provider)),
    };
    let client_id = std::env::var(client_id_env).unwrap_or_default();

    let expected_issuer = match provider {
        "google" => "https://accounts.google.com",
        "apple" => "https://appleid.apple.com",
        _ => "",
    };

    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[&client_id]);
    validation.set_issuer(&[expected_issuer]);

    // 5. Verify signature + claims
    let token_data = decode::<serde_json::Value>(token, &decoding_key, &validation)
        .map_err(|e| format!("Invalid identity token: {}", e))?;

    let claims = token_data.claims;

    Ok(UserIdentity {
        provider: provider.to_string(),
        sub: claims["sub"].as_str().unwrap_or("").to_string(),
        email: claims["email"].as_str().unwrap_or("").to_string(),
        display_name: claims["name"].as_str()
            .or_else(|| claims["email"].as_str())
            .unwrap_or("").to_string(),
        avatar_url: claims["picture"].as_str().map(String::from),
    })
}
```

- [ ] **Step 5: Add token exchange function**

Append to `oauth.rs`:

```rust
// ── Token Exchange ─────────────────────────────────────────

/// Exchange authorization code for id_token, verify via JWKS, return UserIdentity
async fn exchange_and_verify(
    provider: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<UserIdentity, String> {
    let client = reqwest::Client::new();

    let (token_url, client_id) = match provider {
        "google" => (
            "https://oauth2.googleapis.com/token",
            std::env::var("PHANTOM_MESH_GOOGLE_CLIENT_ID").unwrap_or_default(),
        ),
        "apple" => (
            "https://appleid.apple.com/auth/token",
            std::env::var("PHANTOM_MESH_APPLE_CLIENT_ID").unwrap_or_default(),
        ),
        _ => return Err(format!("Unknown OAuth provider: {}", provider)),
    };

    let mut form: Vec<(&str, String)> = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("code_verifier", code_verifier.to_string()),
        ("client_id", client_id),
    ];

    // Apple requires client_secret JWT even with PKCE
    // TODO: Generate Apple client_secret JWT from .p8 key
    // Apple Sign-In will fail at runtime until .p8 key generation is implemented
    if provider == "apple" {
        form.push(("client_secret", String::new()));
    }

    let resp = client.post(token_url)
        .form(&form)
        .send().await
        .map_err(|e| format!("Token exchange failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Token exchange failed: HTTP {} — {}", status, body));
    }

    let body: serde_json::Value = resp.json().await
        .map_err(|e| format!("Token exchange failed: {}", e))?;

    let id_token = body["id_token"].as_str()
        .ok_or("Token exchange failed: no id_token in response")?;

    // Fetch JWKS and verify id_token cryptographically
    let jwks = get_jwks(provider).await?;
    verify_id_token(provider, id_token, &jwks)
}
```

- [ ] **Step 6: Add the main oauth_sign_in Tauri command**

Append to `oauth.rs`:

```rust
// ── Tauri Command ──────────────────────────────────────────

#[tauri::command]
pub async fn oauth_sign_in(provider: String) -> Result<UserIdentity, String> {
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);
    let state = generate_state();

    // Start callback server
    let (port, rx) = start_callback_server()?;
    let redirect_uri = format!("http://localhost:{}/callback", port);

    // Build authorization URL
    let (client_id_env, auth_url_base) = match provider.as_str() {
        "google" => ("PHANTOM_MESH_GOOGLE_CLIENT_ID", "https://accounts.google.com/o/oauth2/v2/auth"),
        "apple" => ("PHANTOM_MESH_APPLE_CLIENT_ID", "https://appleid.apple.com/auth/authorize"),
        _ => return Err(format!("Unknown provider: {}", provider)),
    };

    let client_id = std::env::var(client_id_env)
        .map_err(|_| format!("{} environment variable not set", client_id_env))?;

    let scope = match provider.as_str() {
        "google" => "openid email profile",
        "apple" => "openid email name",
        _ => "openid email",
    };

    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        auth_url_base,
        urlencoding::encode(&client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(scope),
        urlencoding::encode(&state),
        urlencoding::encode(&code_challenge),
    );

    // Open system browser
    open::that(&auth_url).map_err(|e| format!("Cannot open browser: {}", e))?;

    // Wait for callback (125s > server's 120s timeout)
    let (code, returned_state) = rx.recv_timeout(Duration::from_secs(125))
        .map_err(|_| "OAuth timeout: no callback received".to_string())?;

    if code.is_empty() {
        return Err("OAuth timeout: no callback received".to_string());
    }

    // Verify state (CSRF protection)
    if returned_state != state {
        return Err("OAuth state mismatch — possible CSRF attack".to_string());
    }

    // Exchange code for identity (with JWKS verification)
    exchange_and_verify(&provider, &code, &code_verifier, &redirect_uri).await
}
```

- [ ] **Step 7: Register module and command**

In `phantom-mesh-desktop/src-tauri/src/commands/mod.rs`, add:
```rust
pub mod oauth;
```

In `phantom-mesh-desktop/src-tauri/src/main.rs`, add `commands::oauth::oauth_sign_in` to the `tauri::generate_handler![]` list.

- [ ] **Step 8: Verify it compiles**

Run: `cd phantom-mesh-desktop/src-tauri && CARGO_TARGET_DIR="C:/tmp/desktop-target" cargo check`
Expected: compiles

- [ ] **Step 9: Commit**

```bash
git add phantom-mesh-desktop/src-tauri/src/commands/oauth.rs \
        phantom-mesh-desktop/src-tauri/src/commands/mod.rs \
        phantom-mesh-desktop/src-tauri/src/main.rs \
        phantom-mesh-desktop/src-tauri/Cargo.toml
git commit -m "feat(desktop): add OAuth PKCE sign-in command with JWKS id_token verification"
```

---

### Task 6: Rewrite StepSecurity (Desktop)

**Files:**
- Modify: `phantom-mesh-desktop/src/components/onboarding/StepSecurity.tsx` (full rewrite)

- [ ] **Step 1: Read the current StepSecurity.tsx**

Read `phantom-mesh-desktop/src/components/onboarding/StepSecurity.tsx` to understand existing structure.

- [ ] **Step 2: Rewrite StepSecurity with OAuth buttons + PIN input**

Replace entire content of `StepSecurity.tsx`:

```tsx
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { OnboardingData, UserIdentity } from './types';

interface Props {
  data: OnboardingData;
  updateData: (partial: Partial<OnboardingData>) => void;
  onNext: () => void;
  onBack: () => void;
}

export default function StepSecurity({ data, updateData, onNext, onBack }: Props) {
  const [identity, setIdentity] = useState<UserIdentity | null>(data.identity);
  const [pin, setPin] = useState('');
  const [confirmPin, setConfirmPin] = useState('');
  const [oauthLoading, setOauthLoading] = useState<string | null>(null);
  const [oauthError, setOauthError] = useState<string | null>(null);

  const canProceed = identity !== null && pin.length === 6 && pin === confirmPin;

  const handleOAuth = async (provider: 'google' | 'apple') => {
    setOauthLoading(provider);
    setOauthError(null);
    try {
      const result = await invoke<UserIdentity>('oauth_sign_in', { provider });
      setIdentity(result);
      updateData({ identity: result });
    } catch (e) {
      setOauthError(String(e));
    }
    setOauthLoading(null);
  };

  const handlePinInput = (value: string) => {
    const digits = value.replace(/\D/g, '').slice(0, 6);
    setPin(digits);
  };

  const handleConfirmInput = (value: string) => {
    const digits = value.replace(/\D/g, '').slice(0, 6);
    setConfirmPin(digits);
  };

  const handleSubmit = () => {
    if (!canProceed) return;
    updateData({ vaultPin: pin });
    onNext();
  };

  return (
    <div>
      <h2 className="text-2xl font-bold text-white mb-2">KeyVault 安全設定</h2>
      <p className="text-phantom-mesh-muted text-sm mb-6">
        登入帳號以識別此 Hub 的擁有者，並設定 Vault PIN。
      </p>

      {/* OAuth Sign-In */}
      <div className="bg-phantom-mesh-card border border-phantom-mesh-border rounded-lg p-4 mb-4">
        <div className="text-sm text-phantom-mesh-text mb-3">帳號登入</div>

        {identity ? (
          <div className="flex items-center gap-3 py-2">
            {identity.avatar_url && (
              <img src={identity.avatar_url} alt="" className="w-8 h-8 rounded-full" />
            )}
            <div>
              <div className="text-sm text-white">{identity.display_name}</div>
              <div className="text-xs text-phantom-mesh-muted">{identity.email}</div>
            </div>
            <span className="text-phantom-mesh-success text-xs ml-auto">已登入</span>
          </div>
        ) : (
          <div className="space-y-2">
            <button
              onClick={() => handleOAuth('google')}
              disabled={oauthLoading !== null}
              className="w-full flex items-center justify-center gap-2 bg-white text-gray-800 px-4 py-2.5 rounded-lg
                         font-medium text-sm hover:bg-gray-100 transition disabled:opacity-50"
            >
              {oauthLoading === 'google' ? (
                <div className="w-4 h-4 border-2 border-gray-400 border-t-transparent rounded-full animate-spin" />
              ) : (
                <span>G</span>
              )}
              使用 Google 登入
            </button>
            <button
              onClick={() => handleOAuth('apple')}
              disabled={oauthLoading !== null}
              className="w-full flex items-center justify-center gap-2 bg-black text-white px-4 py-2.5 rounded-lg
                         font-medium text-sm border border-gray-600 hover:bg-gray-900 transition disabled:opacity-50"
            >
              {oauthLoading === 'apple' ? (
                <div className="w-4 h-4 border-2 border-gray-400 border-t-transparent rounded-full animate-spin" />
              ) : (
                <span></span>
              )}
              使用 Apple 登入
            </button>
          </div>
        )}

        {oauthError && (
          <div className="mt-2">
            <p className="text-phantom-mesh-danger text-xs">{oauthError}</p>
            <button
              onClick={() => setOauthError(null)}
              className="text-phantom-mesh-primary text-xs mt-1 hover:underline"
            >
              重試
            </button>
          </div>
        )}
      </div>

      {/* Vault PIN */}
      <div className="bg-phantom-mesh-card border border-phantom-mesh-border rounded-lg p-4 mb-4">
        <label className="block text-sm text-phantom-mesh-text mb-3">Vault PIN（6 位數字）</label>
        <input
          type="password"
          inputMode="numeric"
          maxLength={6}
          value={pin}
          onChange={e => handlePinInput(e.target.value)}
          placeholder="● ● ● ● ● ●"
          className="w-full bg-phantom-mesh-bg border border-phantom-mesh-border rounded px-3 py-2 text-white text-center text-lg tracking-[0.5em]
                     focus:outline-none focus:border-phantom-mesh-primary font-mono"
        />

        <label className="block text-sm text-phantom-mesh-text mt-4 mb-1.5">確認 PIN</label>
        <input
          type="password"
          inputMode="numeric"
          maxLength={6}
          value={confirmPin}
          onChange={e => handleConfirmInput(e.target.value)}
          placeholder="● ● ● ● ● ●"
          className="w-full bg-phantom-mesh-bg border border-phantom-mesh-border rounded px-3 py-2 text-white text-center text-lg tracking-[0.5em]
                     focus:outline-none focus:border-phantom-mesh-primary font-mono"
        />
        {confirmPin.length > 0 && confirmPin.length === 6 && pin !== confirmPin && (
          <p className="text-phantom-mesh-danger text-xs mt-1">PIN 不一致</p>
        )}
      </div>

      <div className="bg-phantom-mesh-warning/10 border border-phantom-mesh-warning/30 rounded-lg px-4 py-3 mb-6">
        <p className="text-phantom-mesh-warning text-xs">
          PIN 遺失後需重新設定，已加密的資料將無法恢復。
        </p>
      </div>

      <div className="flex justify-between">
        <button
          onClick={onBack}
          className="text-phantom-mesh-muted hover:text-white text-sm px-4 py-2 transition"
        >
          ← 上一步
        </button>
        <button
          onClick={handleSubmit}
          disabled={!canProceed}
          className="bg-phantom-mesh-primary text-phantom-mesh-bg px-6 py-2.5 rounded-lg font-medium
                     disabled:opacity-40 disabled:cursor-not-allowed hover:brightness-110 transition"
        >
          設定完成 →
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Verify TypeScript compiles (StepSecurity only)**

Run: `cd phantom-mesh-desktop && npx tsc --noEmit 2>&1 | grep -v 'StepComplete\|useWizardState'`
Expected: StepSecurity errors resolved. Other files may still have errors.

- [ ] **Step 4: Commit**

```bash
git add phantom-mesh-desktop/src/components/onboarding/StepSecurity.tsx
git commit -m "feat(desktop): rewrite StepSecurity with OAuth buttons + 6-digit PIN"
```

---

### Task 7: Update Downstream Desktop Files

**Files:**
- Modify: `phantom-mesh-desktop/src/components/onboarding/useWizardState.ts`
- Modify: `phantom-mesh-desktop/src/components/onboarding/StepComplete.tsx`
- Modify: `phantom-mesh-desktop/src/components/onboarding/OnboardingWizard.tsx`

- [ ] **Step 1: Read all three files**

Read `useWizardState.ts`, `StepComplete.tsx`, `OnboardingWizard.tsx`.

- [ ] **Step 2: Update useWizardState.ts**

Replace `INITIAL_DATA`:
```typescript
const INITIAL_DATA: OnboardingData = {
  hardwareScan: null,
  identity: null,       // NEW
  vaultPin: '',          // RENAMED from vaultPassword
  providers: [],
  clusterEnabled: false,
  clusterNodes: [],
  telegramToken: '',
  qrPayload: null,
  ollamaEndpoint: 'http://localhost:11434',
  ollamaEnabled: false,
};
```

Update `loadPersistedState()` to restore identity info on crash recovery:
```typescript
function loadPersistedState(): { step: WizardStep; data: Partial<OnboardingData> } {
  try {
    const raw = localStorage.getItem(WIZARD_STORAGE_KEY);
    if (!raw) return { step: 0, data: {} };
    const saved: PersistedWizardState = JSON.parse(raw);
    // Crash recovery: reset to step 1 (re-enter PIN) but keep identity if already authed
    const step = saved.currentStep >= 2 ? 1 as WizardStep : saved.currentStep as WizardStep;

    // Reconstruct minimal identity from persisted email/provider (skip OAuth re-auth)
    let identity: UserIdentity | null = null;
    if (saved.identityEmail && saved.identityProvider) {
      identity = {
        provider: saved.identityProvider as 'google' | 'apple',
        sub: '',  // sub not persisted — will be refreshed on next OAuth if needed
        email: saved.identityEmail,
        display_name: saved.identityEmail,
        avatar_url: null,
      };
    }

    return {
      step,
      data: {
        identity,
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
```

Add `UserIdentity` to imports:
```typescript
import {
  OnboardingData, PersistedWizardState, UserIdentity, WizardStep,
  WIZARD_STORAGE_KEY, ONBOARDED_KEY,
} from './types';
```

Update the `useEffect` that persists state to include identity info:
```typescript
useEffect(() => {
  const state: PersistedWizardState = {
    currentStep,
    ollamaEnabled: data.ollamaEnabled,
    ollamaEndpoint: data.ollamaEndpoint,
    providerNames: data.providers.map(p => p.name),
    clusterEnabled: data.clusterEnabled,
    clusterNodes: data.clusterNodes,
    telegramConfigured: !!data.telegramToken,
    identityEmail: data.identity?.email,
    identityProvider: data.identity?.provider,
  };
  localStorage.setItem(WIZARD_STORAGE_KEY, JSON.stringify(state));
}, [currentStep, data]);
```

- [ ] **Step 3: Update StepComplete.tsx**

Change the `summaryItems` array:
```typescript
const summaryItems = [
  { label: '帳號', value: data.identity?.email ?? '未登入' },
  { label: 'Daemon Port', value: String(port) },
  { label: 'KeyVault', value: '✓ 已設定 PIN' },
  { label: 'Providers', value: [
    ...(data.ollamaEnabled ? ['Ollama'] : []),
    ...data.providers.filter(p => p.validated).map(p => p.name),
  ].join(', ') || '無' },
  { label: '叢集', value: data.clusterEnabled ? '啟用' : '關閉' },
  { label: 'Telegram', value: data.telegramToken ? '已設定' : '未設定' },
  { label: '主 Hub', value: '✓ 此裝置' },
];
```

In the `launch` function, update the `invoke('launch_daemon', ...)` call:
```typescript
const status = await invoke<DaemonStatus>('launch_daemon', {
  vaultPin: data.vaultPin,   // RENAMED from vaultPassword
  port,
  binaryPath,
});
```

Update the `invoke('write_config', ...)` call to include identity and sync data:
```typescript
await invoke('write_config', {
  data: {
    port,
    providers: data.providers
      .filter(p => p.validated)
      .map(p => ({ name: p.name, api_key: p.apiKey, provider_type: p.providerType })),
    ollama_endpoint: data.ollamaEnabled ? data.ollamaEndpoint : null,
    default_agent_provider: defaultProvider,
    default_agent_model: defaultModel,
    auth_key: data.qrPayload?.auth_key ?? crypto.randomUUID().replace(/-/g, ''),
    telegram_token: data.telegramToken || null,
    identity_provider: data.identity?.provider ?? null,
    identity_sub: data.identity?.sub ?? null,
    identity_email: data.identity?.email ?? null,
    is_primary: true,  // First device is automatically Primary Hub
  },
});
```

After `completeWizard()`, persist identity to Tauri Store:
```typescript
// Persist identity to Tauri Store for post-onboarding access
if (data.identity) {
  try {
    const { Store } = await import('@tauri-apps/plugin-store');
    const store = new Store('phantom-mesh-store.json');
    await store.set('user_identity', data.identity);
    await store.save();
  } catch {
    // Tauri Store is best-effort
  }
}

setPhase('success');
completeWizard();
```

- [ ] **Step 4: Update OnboardingWizard.tsx**

No step count changes needed (OAuth is inside StepSecurity). Verify that `data` and `updateData` are correctly passed to StepSecurity. The existing pattern already does this — confirm by reading the file.

- [ ] **Step 5: Verify full TypeScript build**

Run: `cd phantom-mesh-desktop && npx tsc --noEmit`
Expected: No errors

- [ ] **Step 6: Commit**

```bash
git add phantom-mesh-desktop/src/components/onboarding/useWizardState.ts \
        phantom-mesh-desktop/src/components/onboarding/StepComplete.tsx \
        phantom-mesh-desktop/src/components/onboarding/OnboardingWizard.tsx
git commit -m "feat(desktop): update wizard state + StepComplete for vaultPin, identity, Primary Hub"
```

---

### Task 8: Update launch_daemon to Use stdin Pipe + write_config Identity

**Files:**
- Modify: `phantom-mesh-desktop/src-tauri/src/commands/onboarding.rs`

- [ ] **Step 1: Read the current onboarding.rs**

Read `phantom-mesh-desktop/src-tauri/src/commands/onboarding.rs`. Find the `launch_daemon` function (around line 406-458) and the `OnboardingConfig` struct (around line 56-64) and `write_config` function.

- [ ] **Step 2: Update launch_daemon to use stdin pipe**

Rename parameter `vault_password` → `vault_pin` and use stdin pipe:

```rust
#[tauri::command]
pub async fn launch_daemon(
    state: tauri::State<'_, crate::daemon::DaemonState>,
    http: tauri::State<'_, super::HttpClient>,
    vault_pin: String,
    port: u16,
    binary_path: String,
) -> Result<DaemonStatus, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new(&binary_path)
        .arg("--host")
        .arg("0.0.0.0")
        .arg("--port")
        .arg(port.to_string())
        .arg("daemon")
        .arg("--vault-password-stdin")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start daemon: {}", e))?;

    // Write PIN to stdin (not visible in process listing)
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(vault_pin.as_bytes())
            .map_err(|e| format!("Failed to send PIN to daemon: {}", e))?;
    }

    let pid = child.id();

    {
        let mut proc = state.process.lock().map_err(|e| e.to_string())?;
        *proc = Some(child);
    }

    // Wait for daemon to start and check health
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

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

- [ ] **Step 3: Add identity and sync fields to OnboardingConfig**

Update the `OnboardingConfig` struct:

```rust
#[derive(Debug, Deserialize)]
pub struct OnboardingConfig {
    pub port: u16,
    pub providers: Vec<ProviderEntry>,
    pub ollama_endpoint: Option<String>,
    pub default_agent_provider: String,
    pub default_agent_model: String,
    pub auth_key: String,
    pub telegram_token: Option<String>,
    // Identity fields (from OAuth)
    pub identity_provider: Option<String>,
    pub identity_sub: Option<String>,
    pub identity_email: Option<String>,
    // Sync config
    pub is_primary: Option<bool>,
}
```

- [ ] **Step 4: Write identity and sync sections in write_config**

In the `write_config` function, after the `[auth]` section, append identity and sync sections:

```rust
// [identity] section
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

// [sync] section
if data.is_primary.unwrap_or(false) {
    toml.push_str("\n[sync]\nis_primary = true\n");
}
```

- [ ] **Step 5: Verify it compiles**

Run: `cd phantom-mesh-desktop/src-tauri && CARGO_TARGET_DIR="C:/tmp/desktop-target" cargo check`
Expected: compiles

- [ ] **Step 6: Commit**

```bash
git add phantom-mesh-desktop/src-tauri/src/commands/onboarding.rs
git commit -m "security(desktop): pass vault PIN via stdin pipe, write identity + sync to agents.toml"
```

---

### Task 9: Add Mobile Dependencies + Types

**Files:**
- Modify: `mobile/phantom-mesh-worker-app/package.json`
- Modify: `mobile/phantom-mesh-worker-app/src/components/onboarding/types.ts`

- [ ] **Step 1: Read current mobile types.ts**

Read `mobile/phantom-mesh-worker-app/src/components/onboarding/types.ts`.

- [ ] **Step 2: Install mobile OAuth dependencies**

```bash
cd mobile/phantom-mesh-worker-app
npx expo install expo-auth-session expo-apple-authentication expo-crypto
```

- [ ] **Step 3: Update mobile types.ts**

Add `UserIdentity` interface and `identity` field to `MobileOnboardingData`:

```typescript
export interface UserIdentity {
  provider: 'google' | 'apple';
  sub: string;
  email: string;
  display_name: string;
  avatar_url: string | null;
}

export interface MobileOnboardingData {
  identity: UserIdentity | null;  // NEW
  hubUrl: string;
  authKey: string;
  workerName: string;
  agentName: string;
  connectionTested: boolean;
}
```

- [ ] **Step 4: Note expected TypeScript errors**

The addition of `identity` to `MobileOnboardingData` will cause errors in `OnboardingWizard.tsx` (missing `identity` in initial data), `StepConnect.tsx`, etc. This is expected and will be fixed in Tasks 10–11.

- [ ] **Step 5: Commit**

```bash
git add mobile/phantom-mesh-worker-app/package.json \
        mobile/phantom-mesh-worker-app/src/components/onboarding/types.ts
git commit -m "feat(mobile): add OAuth deps + UserIdentity type"
```

---

### Task 10: Create Mobile StepAuth

**Files:**
- Create: `mobile/phantom-mesh-worker-app/src/components/onboarding/StepAuth.tsx`

- [ ] **Step 1: Create StepAuth component**

```tsx
import { useState } from 'react';
import { View, Text, TouchableOpacity, Image, ActivityIndicator, StyleSheet } from 'react-native';
import * as AuthSession from 'expo-auth-session';
import * as Google from 'expo-auth-session/providers/google';
import * as AppleAuthentication from 'expo-apple-authentication';
import { UserIdentity, MobileOnboardingData } from './types';

interface Props {
  data: MobileOnboardingData;
  updateData: (partial: Partial<MobileOnboardingData>) => void;
  onNext: () => void;
  onBack: () => void;
}

export default function StepAuth({ data, updateData, onNext, onBack }: Props) {
  const [identity, setIdentity] = useState<UserIdentity | null>(data.identity);
  const [loading, setLoading] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [, googleResponse, googlePromptAsync] = Google.useAuthRequest({
    expoClientId: process.env.EXPO_PUBLIC_GOOGLE_CLIENT_ID,
  });

  const handleGoogle = async () => {
    setLoading('google');
    setError(null);
    try {
      const result = await googlePromptAsync();
      if (result?.type === 'success' && result.authentication?.accessToken) {
        const resp = await fetch('https://www.googleapis.com/oauth2/v3/userinfo', {
          headers: { Authorization: `Bearer ${result.authentication.accessToken}` },
        });
        const info = await resp.json();
        const id: UserIdentity = {
          provider: 'google',
          sub: info.sub,
          email: info.email,
          display_name: info.name || info.email,
          avatar_url: info.picture || null,
        };
        setIdentity(id);
        updateData({ identity: id });
      }
    } catch (e) {
      setError(String(e));
    }
    setLoading(null);
  };

  const handleApple = async () => {
    setLoading('apple');
    setError(null);
    try {
      const credential = await AppleAuthentication.signInAsync({
        requestedScopes: [
          AppleAuthentication.AppleAuthenticationScope.EMAIL,
          AppleAuthentication.AppleAuthenticationScope.FULL_NAME,
        ],
      });
      const id: UserIdentity = {
        provider: 'apple',
        sub: credential.user,
        email: credential.email || '',
        display_name: credential.fullName
          ? `${credential.fullName.givenName || ''} ${credential.fullName.familyName || ''}`.trim()
          : credential.email || '',
        avatar_url: null,
      };
      setIdentity(id);
      updateData({ identity: id });
    } catch (e) {
      setError(String(e));
    }
    setLoading(null);
  };

  return (
    <View style={styles.container}>
      <Text style={styles.stepLabel}>步驟 2 / 5</Text>
      <Text style={styles.title}>登入你的帳號</Text>
      <Text style={styles.subtitle}>用於識別裝置擁有者與跨裝置同步</Text>

      {identity ? (
        <View style={styles.card}>
          {identity.avatar_url && (
            <Image source={{ uri: identity.avatar_url }} style={styles.avatar} />
          )}
          <Text style={styles.email}>{identity.email}</Text>
          <Text style={styles.success}>已登入</Text>
        </View>
      ) : (
        <View style={styles.card}>
          <TouchableOpacity
            style={[styles.btn, styles.googleBtn]}
            onPress={handleGoogle}
            disabled={loading !== null}
          >
            {loading === 'google'
              ? <ActivityIndicator size="small" />
              : <Text style={styles.googleText}>使用 Google 登入</Text>}
          </TouchableOpacity>

          <TouchableOpacity
            style={[styles.btn, styles.appleBtn]}
            onPress={handleApple}
            disabled={loading !== null}
          >
            {loading === 'apple'
              ? <ActivityIndicator size="small" color="#fff" />
              : <Text style={styles.appleText}> 使用 Apple 登入</Text>}
          </TouchableOpacity>
        </View>
      )}

      {error && <Text style={styles.error}>{error}</Text>}

      <View style={styles.navRow}>
        <TouchableOpacity onPress={onBack}>
          <Text style={styles.backText}>← 上一步</Text>
        </TouchableOpacity>
        <TouchableOpacity
          style={[styles.nextBtn, !identity && styles.disabled]}
          onPress={onNext}
          disabled={!identity}
        >
          <Text style={styles.nextText}>下一步 →</Text>
        </TouchableOpacity>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, padding: 20 },
  stepLabel: { fontSize: 12, color: '#888', marginBottom: 8 },
  title: { fontSize: 22, fontWeight: 'bold', color: '#fff', marginBottom: 4 },
  subtitle: { fontSize: 13, color: '#888', marginBottom: 24 },
  card: { backgroundColor: '#1a1a2e', borderRadius: 12, padding: 16, marginBottom: 16 },
  btn: { paddingVertical: 14, borderRadius: 10, alignItems: 'center', marginBottom: 10 },
  googleBtn: { backgroundColor: '#fff' },
  googleText: { color: '#333', fontWeight: '600', fontSize: 15 },
  appleBtn: { backgroundColor: '#000', borderWidth: 1, borderColor: '#444' },
  appleText: { color: '#fff', fontWeight: '600', fontSize: 15 },
  avatar: { width: 40, height: 40, borderRadius: 20, alignSelf: 'center', marginBottom: 8 },
  email: { color: '#fff', textAlign: 'center', fontSize: 14 },
  success: { color: '#4ade80', textAlign: 'center', fontSize: 12, marginTop: 4 },
  error: { color: '#f87171', fontSize: 12, marginBottom: 12 },
  navRow: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', marginTop: 'auto' },
  backText: { color: '#888', fontSize: 14 },
  nextBtn: { backgroundColor: '#6366f1', paddingVertical: 14, paddingHorizontal: 24, borderRadius: 10, alignItems: 'center' },
  nextText: { color: '#fff', fontWeight: '600', fontSize: 15 },
  disabled: { opacity: 0.4 },
});
```

- [ ] **Step 2: Commit**

```bash
git add mobile/phantom-mesh-worker-app/src/components/onboarding/StepAuth.tsx
git commit -m "feat(mobile): add StepAuth with Google + Apple OAuth sign-in"
```

---

### Task 11: Update Mobile Wizard + Step Labels

**Files:**
- Modify: `mobile/phantom-mesh-worker-app/src/components/onboarding/OnboardingWizard.tsx`
- Modify: `mobile/phantom-mesh-worker-app/src/components/onboarding/StepConnect.tsx`
- Modify: `mobile/phantom-mesh-worker-app/src/components/onboarding/StepIdentity.tsx`
- Modify: `mobile/phantom-mesh-worker-app/src/components/onboarding/StepComplete.tsx`

- [ ] **Step 1: Read all four files**

Read `OnboardingWizard.tsx`, `StepConnect.tsx`, `StepIdentity.tsx`, `StepComplete.tsx`.

- [ ] **Step 2: Update OnboardingWizard.tsx**

Change `TOTAL_STEPS` from `4` to `5`.

Add import:
```typescript
import StepAuth from './StepAuth';
```

In the `initialData`, add:
```typescript
identity: null,
```

Update the step switch to insert StepAuth at case 1 and shift others:
```typescript
case 0: return <StepWelcome onNext={goNext} />;
case 1: return <StepAuth data={data} updateData={updateData} onNext={goNext} onBack={goBack} />;
case 2: return <StepConnect data={data} updateData={updateData} onNext={goNext} onBack={goBack} />;
case 3: return <StepIdentity data={data} updateData={updateData} onNext={goNext} onBack={goBack} />;
case 4: return <StepComplete data={data} completeWizard={completeWizard} onComplete={onComplete} onBack={goBack} />;
```

- [ ] **Step 3: Update step labels in all affected files**

In `StepConnect.tsx`: change `"步驟 2 / 4"` → `"步驟 3 / 5"`

In `StepIdentity.tsx`: change `"步驟 3 / 4"` → `"步驟 4 / 5"`

In `StepComplete.tsx`: change `"步驟 4 / 4"` → `"步驟 5 / 5"`

- [ ] **Step 4: Update StepConnect.tsx to add X-Phantom Mesh-Sub header**

Find the fetch call to `${hubUrl}/health` and add the identity header:

```typescript
const headers: Record<string, string> = {
  'Authorization': `Bearer ${data.authKey}`,
};
if (data.identity?.sub) {
  headers['X-Phantom Mesh-Sub'] = data.identity.sub;
}
```

- [ ] **Step 5: Commit**

```bash
git add mobile/phantom-mesh-worker-app/src/components/onboarding/OnboardingWizard.tsx \
        mobile/phantom-mesh-worker-app/src/components/onboarding/StepConnect.tsx \
        mobile/phantom-mesh-worker-app/src/components/onboarding/StepIdentity.tsx \
        mobile/phantom-mesh-worker-app/src/components/onboarding/StepComplete.tsx
git commit -m "feat(mobile): insert StepAuth in wizard, update step labels to /5, add X-Phantom Mesh-Sub"
```

---

### Task 12: Full Integration Verification

**Files:** None (verification only)

- [ ] **Step 1: Desktop Rust build**

Run: `cd phantom-mesh-desktop/src-tauri && CARGO_TARGET_DIR="C:/tmp/desktop-target" cargo check`
Expected: compiles

- [ ] **Step 2: Desktop TypeScript check**

Run: `cd phantom-mesh-desktop && npx tsc --noEmit`
Expected: No errors

- [ ] **Step 3: Core Rust tests**

Run: `cd phantom-mesh && CARGO_TARGET_DIR=target_oauth cargo test key_vault`
Expected: All key_vault tests pass with hardened params

- [ ] **Step 4: Core Rust build**

Run: `cd phantom-mesh && CARGO_TARGET_DIR=target_oauth cargo check`
Expected: compiles

- [ ] **Step 5: Mobile TypeScript check**

Run: `cd mobile/phantom-mesh-worker-app && npx tsc --noEmit`
Expected: No errors (or only unrelated pre-existing errors)

- [ ] **Step 6: Commit verification results (if any fixups needed)**

```bash
git add -A
git commit -m "fix: integration fixups for OAuth + Vault PIN"
```
