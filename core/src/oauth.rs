use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const GOOGLE_CLIENT_ID: &str = "869770808980-0kom8ag838tc1p5sqvugitra2gnmbe50.apps.googleusercontent.com";
const APPLE_CLIENT_ID: &str = "ai.phantommesh.auth";
const APPLE_RELAY_URL: &str = "https://apple-oauth-relay.vercel.app/auth/apple-callback";
const FRONTEND_URL: &str = "http://localhost:5173";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserIdentity {
    pub provider: String,
    pub sub: String,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AppleAuthConfig {
    pub client_id: Option<String>,
    pub team_id: String,
    pub key_id: String,
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

fn load_apple_config() -> Option<AppleAuthConfig> {
    let home = std::env::var("HOME").ok()?;
    let paths = [
        format!("{}/.phantom-mesh/apple-auth.json", home),
        format!("{}/.config/phantom-mesh/apple-auth.json", home),
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

fn generate_apple_client_secret(config: &AppleAuthConfig, client_id: &str) -> Result<String, String> {
    let p8_pem = std::fs::read_to_string(&config.p8_path)
        .map_err(|e| format!("Cannot read .p8 key: {}", e))?;

    let encoding_key = jsonwebtoken::EncodingKey::from_ec_pem(p8_pem.as_bytes())
        .map_err(|e| format!("Invalid .p8 key: {}", e))?;

    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

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
        urlencoding::encode(GOOGLE_CLIENT_ID),
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

pub fn apple_start_url(daemon_port: u16) -> Result<String, String> {
    let _config = load_apple_config()
        .ok_or("Apple 登入需要設定檔。請建立 ~/.phantom-mesh/apple-auth.json")?;

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

pub fn apple_available() -> bool {
    load_apple_config().is_some()
}

// ── Shared callback handler ────────────────────────────────────────────────

pub async fn handle_callback(code: &str, state: &str) -> Result<String, String> {
    let pending = PENDING.lock().unwrap().take()
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
    let client_secret = std::env::var("PHANTOM_MESH_GOOGLE_CLIENT_SECRET")
        .map_err(|_| "PHANTOM_MESH_GOOGLE_CLIENT_SECRET env var not set".to_string())?;
    let client = reqwest::Client::new();
    let resp = client.post("https://oauth2.googleapis.com/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", pending.redirect_uri.as_str()),
            ("code_verifier", pending.code_verifier.as_str()),
            ("client_id", GOOGLE_CLIENT_ID),
            ("client_secret", client_secret.as_str()),
        ])
        .send().await.map_err(|e| format!("Token exchange failed: {}", e))?;

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
    let resp = client.post("https://appleid.apple.com/auth/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", pending.redirect_uri.as_str()),
            ("code_verifier", pending.code_verifier.as_str()),
            ("client_id", client_id),
            ("client_secret", client_secret.as_str()),
        ])
        .send().await.map_err(|e| format!("Token exchange failed: {}", e))?;

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
    if parts.len() != 3 { return Err("Invalid token".into()); }

    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .or_else(|_| {
            let padded = format!("{}{}", parts[1], "=".repeat((4 - parts[1].len() % 4) % 4));
            base64::engine::general_purpose::STANDARD.decode(&padded)
        })
        .map_err(|e| format!("Base64 error: {}", e))?;

    let claims: serde_json::Value = serde_json::from_slice(&payload_bytes)
        .map_err(|e| format!("JWT parse error: {}", e))?;

    Ok(UserIdentity {
        provider: provider.into(),
        sub: claims["sub"].as_str().unwrap_or("").into(),
        email: claims["email"].as_str().unwrap_or("").into(),
        display_name: claims["name"].as_str()
            .or_else(|| claims["email"].as_str())
            .unwrap_or("").into(),
        avatar_url: claims["picture"].as_str().map(String::from),
    })
}

pub fn get_result() -> Option<Result<UserIdentity, String>> {
    RESULT.lock().unwrap().clone()
}
