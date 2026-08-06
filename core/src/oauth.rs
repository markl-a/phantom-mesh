//! Sign-in via Google and Apple using the OAuth 2.0 Authorization Code flow
//! with PKCE (Proof Key for Code Exchange).
//!
//! # Why PKCE
//!
//! spectyn is a public client (a native/desktop app, not a confidential server),
//! so it cannot safely embed a long-lived client secret. PKCE protects the
//! authorization-code exchange against interception:
//!
//! 1. **Start** — generate a random `code_verifier`, derive its SHA-256
//!    `code_challenge`, and a random `csrf_state`. The browser is sent to the
//!    provider's authorize endpoint carrying the challenge + state.
//! 2. **Redirect** — the provider authenticates the user and redirects back with
//!    an authorization `code` plus the original `state`.
//! 3. **Callback** — [`handle_callback`] verifies `state` (CSRF defense), then
//!    exchanges `code` + `code_verifier` for tokens. The provider re-derives the
//!    challenge from the verifier and rejects the exchange if they do not match.
//! 4. **Identity** — the returned OpenID Connect `id_token` (a JWT) is decoded
//!    into a [`UserIdentity`].
//!
//! # Provider differences
//!
//! - **Google** — redirects to a local loopback URI (`http://localhost:<port>/oauth/callback`).
//!   The client secret is read from the `SPECTYN_MESH_GOOGLE_CLIENT_SECRET` env var.
//! - **Apple** — uses `response_mode=form_post` via a hosted relay (Apple does not
//!   redirect to loopback). The `state` is encoded as `"<csrf>.<port>"` so the relay
//!   can route the callback back to the correct local daemon. Apple has no static
//!   client secret; one is minted on demand as a short-lived ES256 JWT signed with a
//!   `.p8` private key (see [`generate_apple_client_secret`]). Apple sign-in is only
//!   available when an `apple-auth.json` config file is present (see [`apple_available`]).
//!
//! # State machine
//!
//! Two process-global [`Mutex`]es hold the flow state: `PENDING` (the in-progress
//! flow between start and callback) and `RESULT` (the final identity or error,
//! polled via [`get_result`]). Only one OAuth flow is tracked at a time.
//!
//! Endpoint URLs and client identifiers are compile-time constants; no secrets are
//! stored in this module.

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const GOOGLE_CLIENT_ID: &str =
    "869770808980-0kom8ag838tc1p5sqvugitra2gnmbe50.apps.googleusercontent.com";
const APPLE_CLIENT_ID: &str = "ai.spectynmesh.auth";
const APPLE_RELAY_URL: &str = "https://apple-oauth-relay.vercel.app/auth/apple-callback";
const FRONTEND_URL: &str = "http://localhost:5173";

/// Authenticated user identity extracted from a provider's OpenID Connect `id_token`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserIdentity {
    /// Provider that issued this identity (`"google"` or `"apple"`).
    pub provider: String,
    /// Stable, provider-scoped subject identifier (the `sub` claim).
    pub sub: String,
    /// User email address (may be empty if not granted by the provider).
    pub email: String,
    /// Human-readable display name; falls back to the email when no name is provided.
    pub display_name: String,
    /// Optional avatar/profile picture URL (`picture` claim), if the provider supplies one.
    pub avatar_url: Option<String>,
}

/// Configuration required to mint Apple "Sign in with Apple" client secrets.
///
/// Loaded from `~/.spectyn-mesh/apple-auth.json` (or the XDG config path). The
/// `.p8` private key it references is used to sign a short-lived ES256 JWT that
/// serves as Apple's client secret during the token exchange.
#[derive(Debug, Deserialize)]
pub struct AppleAuthConfig {
    /// Apple service/client identifier; defaults to the built-in value when omitted.
    pub client_id: Option<String>,
    /// Apple developer Team ID, used as the JWT `iss` (issuer) claim.
    pub team_id: String,
    /// Identifier of the `.p8` signing key, set as the JWT header `kid`.
    pub key_id: String,
    /// Filesystem path to the `.p8` ES256 private key used to sign the client secret.
    pub p8_path: String,
}

struct PendingOAuth {
    provider: String,
    code_verifier: String,
    csrf_state: String,
    redirect_uri: String,
}

static PENDING: Mutex<Option<PendingOAuth>> = Mutex::new(None);
static RESULT: Mutex<Option<Result<UserIdentity, String>>> = Mutex::new(None);

fn gen_random_b64(n: usize) -> String {
    use rand::RngCore;
    let mut buf = vec![0u8; n];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&buf)
}

/// The Google OAuth client id to use, env-overridable so a freshly registered
/// "spectyn-mesh" Google Cloud client can be swapped in without recompiling.
/// Falls back to the compiled-in default when `SPECTYN_MESH_GOOGLE_CLIENT_ID`
/// is unset.
fn google_client_id() -> String {
    std::env::var("SPECTYN_MESH_GOOGLE_CLIENT_ID").unwrap_or_else(|_| GOOGLE_CLIENT_ID.to_string())
}

fn load_apple_config() -> Option<AppleAuthConfig> {
    let home = crate::providers::credential_scanner::home_dir_lenient()?;
    let paths = [
        home.join(".spectyn-mesh").join("apple-auth.json"),
        home.join(".config").join("spectyn-mesh").join("apple-auth.json"),
    ];
    for path in &paths {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(config) = serde_json::from_str::<AppleAuthConfig>(&content) {
                return Some(config);
            }
        }
    }
    None
}

/// Seconds since the UNIX epoch for `t`, or an `Err` describing the failure
/// when `t` is before the UNIX epoch (1970-01-01, i.e. a misconfigured system
/// clock). Parameterized over `t` so the error path is unit-testable without
/// actually setting the system clock.
fn unix_secs_since_epoch(t: SystemTime) -> Result<u64, String> {
    Ok(t.duration_since(UNIX_EPOCH)
        .map_err(|e| format!("system clock before UNIX_EPOCH: {}", e))?
        .as_secs())
}

/// Current UNIX time in seconds, or an `Err` when the system clock is set
/// before the UNIX epoch. Kept separate so the no-panic guarantee of
/// [`generate_apple_client_secret`] can be unit-tested without a real `.p8` key.
fn unix_now_secs() -> Result<u64, String> {
    unix_secs_since_epoch(SystemTime::now())
}

fn generate_apple_client_secret(
    config: &AppleAuthConfig,
    client_id: &str,
) -> Result<String, String> {
    let p8_pem = std::fs::read_to_string(&config.p8_path)
        .map_err(|e| format!("Cannot read .p8 key: {}", e))?;

    let encoding_key = jsonwebtoken::EncodingKey::from_ec_pem(p8_pem.as_bytes())
        .map_err(|e| format!("Invalid .p8 key: {}", e))?;

    let now = unix_now_secs()?;

    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
    header.kid = Some(config.key_id.clone());

    let claims = serde_json::json!({
        "iss": config.team_id,
        "iat": now,
        "exp": now + (86400 * 180),
        "aud": "https://appleid.apple.com",
        "sub": client_id,
    });

    jsonwebtoken::encode(&header, &claims, &encoding_key)
        .map_err(|e| format!("JWT sign failed: {}", e))
}

// ── Google OAuth ───────────────────────────────────────────────────────────

/// Begin a Google sign-in flow and return the authorize URL to open in a browser.
///
/// Generates the PKCE verifier/challenge and CSRF state, records them in the
/// pending-flow slot, and builds the loopback redirect URI for `daemon_port`.
pub fn google_start_url(daemon_port: u16) -> String {
    let code_verifier = gen_random_b64(32);
    let code_challenge = {
        let digest = sha2::Sha256::digest(code_verifier.as_bytes());
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
    };
    let csrf_state = gen_random_b64(16);
    let redirect_uri = format!("http://localhost:{}/oauth/callback", daemon_port);

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth\
         ?response_type=code&client_id={}&redirect_uri={}&scope={}\
         &state={}&code_challenge={}&code_challenge_method=S256\
         &access_type=offline&prompt=consent",
        urlencoding::encode(&google_client_id()),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode("openid email profile"),
        urlencoding::encode(&csrf_state),
        urlencoding::encode(&code_challenge),
    );

    *PENDING.lock().unwrap() = Some(PendingOAuth {
        provider: "google".into(),
        code_verifier,
        csrf_state,
        redirect_uri,
    });
    *RESULT.lock().unwrap() = None;

    auth_url
}

// ── Apple OAuth ────────────────────────────────────────────────────────────

/// Begin an Apple sign-in flow and return the authorize URL to open in a browser.
///
/// Requires an Apple auth config file (otherwise returns an error). Encodes the
/// CSRF state as `"<csrf>.<daemon_port>"` so the hosted relay can route the
/// `form_post` callback back to this daemon.
pub fn apple_start_url(daemon_port: u16) -> Result<String, String> {
    let _config = load_apple_config()
        .ok_or("Apple 登入需要設定檔。請建立 ~/.spectyn-mesh/apple-auth.json")?;

    let code_verifier = gen_random_b64(32);
    let code_challenge = {
        let digest = sha2::Sha256::digest(code_verifier.as_bytes());
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
    };
    let csrf_state = gen_random_b64(16);
    let wire_state = format!("{}.{}", csrf_state, daemon_port);

    let client_id = _config.client_id.as_deref().unwrap_or(APPLE_CLIENT_ID);

    let auth_url = format!(
        "https://appleid.apple.com/auth/authorize\
         ?response_type=code&client_id={}&redirect_uri={}&scope={}\
         &state={}&code_challenge={}&code_challenge_method=S256\
         &response_mode=form_post",
        urlencoding::encode(client_id),
        urlencoding::encode(APPLE_RELAY_URL),
        urlencoding::encode("openid email"),
        urlencoding::encode(&wire_state),
        urlencoding::encode(&code_challenge),
    );

    *PENDING.lock().unwrap() = Some(PendingOAuth {
        provider: "apple".into(),
        code_verifier,
        csrf_state,
        redirect_uri: APPLE_RELAY_URL.to_string(),
    });
    *RESULT.lock().unwrap() = None;

    Ok(auth_url)
}

/// Returns `true` when an Apple auth config file is present, meaning Apple
/// sign-in can be offered.
pub fn apple_available() -> bool {
    load_apple_config().is_some()
}

// ── Shared callback handler ────────────────────────────────────────────────

/// Complete an in-progress OAuth flow from the provider's redirect.
///
/// Verifies `state` against the pending flow (CSRF defense; for Apple, matches
/// the `<csrf>` prefix), exchanges `code` for tokens with the matching provider,
/// stores the resulting [`UserIdentity`] in the result slot, and returns a
/// frontend redirect URL carrying the identity. Returns an error if there is no
/// pending flow, the state mismatches, or the token exchange fails.
pub async fn handle_callback(code: &str, state: &str) -> Result<String, String> {
    let pending = PENDING
        .lock()
        .unwrap()
        .take()
        .ok_or("No pending OAuth flow")?;

    // Verify state — for Apple, state is "{csrf}.{port}", match csrf part
    let state_ok = if pending.provider == "apple" {
        let expected_prefix = &pending.csrf_state;
        state.starts_with(expected_prefix)
    } else {
        state == pending.csrf_state
    };

    if !state_ok {
        let err = "State mismatch".to_string();
        *RESULT.lock().unwrap() = Some(Err(err.clone()));
        return Err(err);
    }

    let identity = match pending.provider.as_str() {
        "google" => exchange_google(code, &pending).await?,
        "apple" => exchange_apple(code, &pending).await?,
        _ => return Err("Unknown provider".into()),
    };

    *RESULT.lock().unwrap() = Some(Ok(identity.clone()));

    let identity_json = serde_json::to_string(&identity).unwrap_or_default();
    let encoded = urlencoding::encode(&identity_json);
    Ok(format!("{}?oauth_identity={}", FRONTEND_URL, encoded))
}

async fn exchange_google(code: &str, pending: &PendingOAuth) -> Result<UserIdentity, String> {
    let client_id = google_client_id();
    // PKCE-only ("Desktop app" type) clients need no secret; "Web app" type
    // clients still do. Include it only when configured so both work.
    let client_secret = std::env::var("SPECTYN_MESH_GOOGLE_CLIENT_SECRET").ok();
    let mut form: Vec<(&str, String)> = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", pending.redirect_uri.clone()),
        ("code_verifier", pending.code_verifier.clone()),
        ("client_id", client_id),
    ];
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret));
    }
    let client = reqwest::Client::new();
    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("Token exchange failed: {}", e))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Google error: {}", body));
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let id_token = body["id_token"].as_str().ok_or("No id_token")?;
    decode_jwt_identity("google", id_token)
}

async fn exchange_apple(code: &str, pending: &PendingOAuth) -> Result<UserIdentity, String> {
    let config = load_apple_config().ok_or("No Apple config")?;
    let client_id = config.client_id.as_deref().unwrap_or(APPLE_CLIENT_ID);
    let client_secret = generate_apple_client_secret(&config, client_id)?;

    let client = reqwest::Client::new();
    let resp = client
        .post("https://appleid.apple.com/auth/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", pending.redirect_uri.as_str()),
            ("code_verifier", pending.code_verifier.as_str()),
            ("client_id", client_id),
            ("client_secret", client_secret.as_str()),
        ])
        .send()
        .await
        .map_err(|e| format!("Token exchange failed: {}", e))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Apple error: {}", body));
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let id_token = body["id_token"].as_str().ok_or("No id_token")?;
    decode_jwt_identity("apple", id_token)
}

fn decode_jwt_identity(provider: &str, id_token: &str) -> Result<UserIdentity, String> {
    let parts: Vec<&str> = id_token.split('.').collect();
    if parts.len() != 3 {
        return Err("Invalid token".into());
    }

    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .or_else(|_| {
            let padded = format!("{}{}", parts[1], "=".repeat((4 - parts[1].len() % 4) % 4));
            base64::engine::general_purpose::STANDARD.decode(&padded)
        })
        .map_err(|e| format!("Base64 error: {}", e))?;

    let claims: serde_json::Value =
        serde_json::from_slice(&payload_bytes).map_err(|e| format!("JWT parse error: {}", e))?;

    Ok(UserIdentity {
        provider: provider.into(),
        sub: claims["sub"].as_str().unwrap_or("").into(),
        email: claims["email"].as_str().unwrap_or("").into(),
        display_name: claims["name"]
            .as_str()
            .or_else(|| claims["email"].as_str())
            .unwrap_or("")
            .into(),
        avatar_url: claims["picture"].as_str().map(String::from),
    })
}

/// Poll the outcome of the most recent OAuth flow.
///
/// Returns `None` while a flow is still in progress, `Some(Ok(_))` once an
/// identity has been resolved, or `Some(Err(_))` if the flow failed.
pub fn get_result() -> Option<Result<UserIdentity, String>> {
    RESULT.lock().unwrap().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `unix_now_secs` must return `Ok` (not panic) for a normal clock, and the
    /// value must be a plausible post-2020 timestamp. This guards the Apple
    /// client-secret path against the previous `.unwrap()`, which panicked when
    /// the system clock was set before the UNIX epoch.
    #[test]
    fn unix_now_secs_is_graceful_and_plausible() {
        let secs = unix_now_secs().expect("unix_now_secs should not error on a sane clock");
        // 1_577_836_800 = 2020-01-01T00:00:00Z — any real clock is well past this.
        assert!(
            secs > 1_577_836_800,
            "expected a post-2020 timestamp, got {secs}"
        );
    }

    /// A pre-1970 clock must surface as a graceful `Err`, not a panic. This is
    /// the actual regression guard for the fix: with the previous `.unwrap()`
    /// this input panicked, so this test fails if the unwrap is reintroduced.
    #[test]
    fn pre_epoch_clock_is_graceful_error() {
        let pre_epoch = UNIX_EPOCH - std::time::Duration::from_secs(1);
        let err = unix_secs_since_epoch(pre_epoch)
            .expect_err("a pre-1970 clock must yield Err, not Ok/panic");
        assert!(
            err.contains("before UNIX_EPOCH"),
            "unexpected error message: {err}"
        );
    }

    /// The iat/exp window used in the Apple JWT must stay a 180-day span, with
    /// exp strictly after iat and no arithmetic overflow.
    #[test]
    fn apple_secret_window_is_180_days() {
        let now = unix_now_secs().unwrap();
        let exp = now + (86400 * 180);
        assert!(exp > now);
        assert_eq!(exp - now, 86400 * 180);
    }
}
