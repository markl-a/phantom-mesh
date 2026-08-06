use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ── Default OAuth Client IDs ──
const DEFAULT_GOOGLE_CLIENT_ID: &str =
    "869770808980-0kom8ag838tc1p5sqvugitra2gnmbe50.apps.googleusercontent.com";
const DEFAULT_APPLE_CLIENT_ID: &str = "ai.spectynmesh.auth";

/// Get OAuth client ID, preferring env var override
fn get_client_id(provider: &str) -> Result<String, String> {
    match provider {
        "google" => Ok(std::env::var("SPECTYN_MESH_GOOGLE_CLIENT_ID")
            .unwrap_or_else(|_| DEFAULT_GOOGLE_CLIENT_ID.to_string())),
        "apple" => Ok(std::env::var("SPECTYN_MESH_APPLE_CLIENT_ID")
            .unwrap_or_else(|_| DEFAULT_APPLE_CLIENT_ID.to_string())),
        _ => Err(format!("Unknown provider: {}", provider)),
    }
}

/// Apple client ID from config file or env
fn get_apple_client_id() -> Result<String, String> {
    // Try reading from apple-auth.json config
    if let Some(config_dir) = dirs::config_dir() {
        let path = config_dir.join("spectyn-mesh").join("apple-auth.json");
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(id) = v["client_id"].as_str() {
                        return Ok(id.to_string());
                    }
                }
            }
        }
    }
    Err("Apple Sign-In 尚未設定。請先設定 apple-auth.json。".to_string())
}

// ── Apple Sign-In Config ─────────────────────────────────

/// apple-auth.json structure:
/// {
///   "client_id": "com.example.spectynmesh",
///   "team_id": "XXXXXXXXXX",
///   "key_id": "YYYYYYYYYY",
///   "p8_path": "C:/path/to/AuthKey_YYYYYYYYYY.p8"
/// }
#[derive(Debug, Clone, Deserialize)]
struct AppleAuthConfig {
    client_id: String,
    team_id: String,
    key_id: String,
    p8_path: String,
}

/// Load Apple auth config from %APPDATA%/spectyn-mesh/apple-auth.json
fn load_apple_config() -> Result<AppleAuthConfig, String> {
    let config_dir = dirs::config_dir()
        .ok_or("Cannot determine config directory")?;
    let path = config_dir.join("spectyn-mesh").join("apple-auth.json");
    let content = std::fs::read_to_string(&path)
        .map_err(|_| format!(
            "找不到 Apple Sign-In 設定檔: {}。請建立此檔案。",
            path.display()
        ))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("apple-auth.json 格式錯誤: {}", e))
}

/// Generate Apple client_secret JWT (ES256, 180-day expiry)
///
/// Apple requires this JWT for every token exchange:
///   Header: {"alg": "ES256", "kid": "<key_id>"}
///   Payload: {"iss": "<team_id>", "iat": now, "exp": now+180d,
///             "aud": "https://appleid.apple.com", "sub": "<client_id>"}
///   Signed with: .p8 private key (PKCS#8 EC P-256)
fn generate_apple_client_secret(client_id: &str) -> Result<String, String> {
    let config = load_apple_config()?;

    // Read .p8 private key
    let p8_pem = std::fs::read_to_string(&config.p8_path)
        .map_err(|e| format!("無法讀取 .p8 金鑰檔 {}: {}", config.p8_path, e))?;

    let encoding_key = jsonwebtoken::EncodingKey::from_ec_pem(p8_pem.as_bytes())
        .map_err(|e| format!(".p8 金鑰格式錯誤: {}", e))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
    header.kid = Some(config.key_id.clone());

    let claims = serde_json::json!({
        "iss": config.team_id,
        "iat": now,
        "exp": now + (86400 * 180),  // 180 days (Apple max)
        "aud": "https://appleid.apple.com",
        "sub": client_id,
    });

    jsonwebtoken::encode(&header, &claims, &encoding_key)
        .map_err(|e| format!("生成 Apple client_secret 失敗: {}", e))
}

#[derive(Debug, Clone, Serialize)]
pub struct UserIdentity {
    pub provider: String,
    pub sub: String,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    /// Raw id_token JWT — needed for Supabase Auth signInWithIdToken
    pub id_token: Option<String>,
}

// ── PKCE Utilities ─────────────────────────────────────────

/// Generate PKCE code_verifier (43-128 chars, URL-safe)
fn generate_code_verifier() -> String {
    use rand::RngCore;
    let mut random_bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut random_bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes)
}

/// Derive code_challenge from code_verifier (S256)
fn generate_code_challenge(verifier: &str) -> String {
    let digest = sha2::Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// Generate a random state parameter for CSRF protection
fn generate_state() -> String {
    use rand::RngCore;
    let mut random_bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut random_bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes)
}

// ── Localhost Callback Server ──────────────────────────────

/// Bind a temporary localhost HTTP server, return (port, receiver for callback params)
fn start_callback_server() -> Result<(u16, std::sync::mpsc::Receiver<(String, String, String)>), String> {
    use rand::Rng;

    let (tx, rx) = std::sync::mpsc::channel();

    // Try random ports in ephemeral range
    let mut server = None;
    for _ in 0..10 {
        let port = 49152 + (rand::thread_rng().gen::<u16>() % 16383);
        match tiny_http::Server::http(format!("127.0.0.1:{}", port)) {
            Ok(s) => {
                server = Some((s, port));
                break;
            }
            Err(_) => continue,
        }
    }

    let (srv, port) = server.ok_or("Cannot bind callback server")?;

    std::thread::spawn(move || {
        // Wait for one request with timeout
        let request = match srv.recv_timeout(Duration::from_secs(120)) {
            Ok(Some(req)) => req,
            _ => {
                let _ = tx.send(("".into(), "timeout".into(), "".into()));
                return;
            }
        };

        let url = request.url().to_string();

        // Parse query params: /callback?code=XXX&state=YYY
        let params: HashMap<String, String> = url
            .split('?')
            .nth(1)
            .unwrap_or("")
            .split('&')
            .filter_map(|pair| {
                let mut kv = pair.splitn(2, '=');
                Some((kv.next()?.to_string(), kv.next().unwrap_or("").to_string()))
            })
            .collect();

        let decode = |s: &str| urlencoding::decode(s).unwrap_or(std::borrow::Cow::Borrowed(s)).into_owned();
        let code = params.get("code").map(|s| decode(s)).unwrap_or_default();
        let state = params.get("state").map(|s| decode(s)).unwrap_or_default();
        let error = params.get("error").map(|s| decode(s)).unwrap_or_default();

        // Respond with success page
        let response = tiny_http::Response::from_string(
            "<html><body><h1>登入成功！</h1><p>你可以關閉此頁面，回到 Spectyn Mesh。</p></body></html>",
        )
        .with_header(
            "Content-Type: text/html; charset=utf-8"
                .parse::<tiny_http::Header>()
                .unwrap(),
        );
        let _ = request.respond(response);

        let _ = tx.send((code, state, error));
    });

    Ok((port, rx))
}

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
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|_| "Cannot fetch provider keys".to_string())?;

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|_| "Cannot fetch provider keys".to_string())?;

    let keys: Vec<JwkKey> = body["keys"]
        .as_array()
        .ok_or("Invalid JWKS format")?
        .iter()
        .filter_map(|k| {
            Some(JwkKey {
                kid: k["kid"].as_str()?.to_string(),
                n: k["n"].as_str()?.to_string(),
                e: k["e"].as_str()?.to_string(),
            })
        })
        .collect();

    // Update cache
    if let Ok(mut cache) = JWKS_CACHE.lock() {
        let map = cache.get_or_insert_with(HashMap::new);
        map.insert(provider.to_string(), (keys.clone(), Instant::now()));
    }

    Ok(keys)
}

/// Verify id_token JWT signature and claims, return UserIdentity
fn verify_id_token(
    provider: &str,
    token: &str,
    jwks: &[JwkKey],
) -> Result<UserIdentity, String> {
    use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};

    // 1. Decode header to get kid
    let header =
        decode_header(token).map_err(|e| format!("Invalid identity token: {}", e))?;
    let kid = header
        .kid
        .ok_or("Invalid identity token: missing kid")?;

    // 2. Find matching key in JWKS
    let jwk = jwks
        .iter()
        .find(|k| k.kid == kid)
        .ok_or(format!(
            "Invalid identity token: no key for kid {}",
            kid
        ))?;

    // 3. Build decoding key from RSA components
    let decoding_key = DecodingKey::from_rsa_components(&jwk.n, &jwk.e)
        .map_err(|e| format!("Invalid identity token: bad key: {}", e))?;

    // 4. Validate aud (client_id), iss (provider), and exp
    let client_id = get_client_id(provider)?;

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
        display_name: claims["name"]
            .as_str()
            .or_else(|| claims["email"].as_str())
            .unwrap_or("")
            .to_string(),
        avatar_url: claims["picture"].as_str().map(String::from),
        id_token: None, // Set by caller after verification
    })
}

// ── Token Exchange ─────────────────────────────────────────

/// Exchange authorization code for id_token, verify via JWKS, return UserIdentity
async fn exchange_and_verify(
    provider: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<UserIdentity, String> {
    let client = reqwest::Client::new();

    let token_url = match provider {
        "google" => "https://oauth2.googleapis.com/token",
        "apple" => "https://appleid.apple.com/auth/token",
        _ => return Err(format!("Unknown OAuth provider: {}", provider)),
    };
    let client_id = get_client_id(provider)?;

    // Both Google (web type) and Apple require client_secret
    let client_secret = match provider {
        "google" => std::env::var("SPECTYN_MESH_GOOGLE_CLIENT_SECRET")
            .map_err(|_| "SPECTYN_MESH_GOOGLE_CLIENT_SECRET env var not set".to_string())?,
        "apple" => generate_apple_client_secret(&client_id)?,
        _ => String::new(),
    };

    let mut form: Vec<(&str, String)> = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("code_verifier", code_verifier.to_string()),
        ("client_id", client_id),
    ];

    if !client_secret.is_empty() {
        form.push(("client_secret", client_secret));
    }

    let resp = client
        .post(token_url)
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("Token exchange failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Token exchange failed: HTTP {} — {}",
            status, body
        ));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Token exchange failed: {}", e))?;

    let id_token_str = body["id_token"]
        .as_str()
        .ok_or("Token exchange failed: no id_token in response")?;

    // Fetch JWKS and verify id_token cryptographically
    let jwks = get_jwks(provider).await?;
    let mut identity = verify_id_token(provider, id_token_str, &jwks)?;
    identity.id_token = Some(id_token_str.to_string());
    Ok(identity)
}

// ── Apple relay URL (HTTPS redirect to localhost) ────────
const APPLE_RELAY_URL: &str = "https://apple-oauth-relay.vercel.app/auth/apple-callback";

// ── Tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_client_id_google() {
        let id = get_client_id("google").unwrap();
        assert!(id.contains(".apps.googleusercontent.com"));
    }

    #[test]
    fn test_get_client_id_apple() {
        let id = get_client_id("apple").unwrap();
        assert_eq!(id, "ai.spectynmesh.auth");
    }

    #[test]
    fn test_get_client_id_unknown() {
        let result = get_client_id("facebook");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown provider"));
    }

    #[test]
    fn test_code_verifier_format() {
        let verifier = generate_code_verifier();
        // base64url of 32 bytes = 43 chars (no padding)
        assert_eq!(verifier.len(), 43);
        // Only URL-safe chars
        assert!(verifier.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn test_code_challenge_deterministic() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = generate_code_challenge(verifier);
        // SHA256 of the verifier, base64url encoded
        assert!(!challenge.is_empty());
        assert!(challenge.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        // Same input → same output
        assert_eq!(challenge, generate_code_challenge(verifier));
    }

    #[test]
    fn test_state_uniqueness() {
        let s1 = generate_state();
        let s2 = generate_state();
        assert_ne!(s1, s2);
        assert!(!s1.is_empty());
    }

    #[test]
    fn test_code_verifier_uniqueness() {
        let v1 = generate_code_verifier();
        let v2 = generate_code_verifier();
        assert_ne!(v1, v2);
    }
}

// ── Tauri Command ──────────────────────────────────────────

#[tauri::command]
pub async fn oauth_sign_in(provider: String) -> Result<UserIdentity, String> {
    tracing::info!("[OAuth] Starting sign-in for provider: {}", provider);

    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);
    let csrf_state = generate_state();

    // Start localhost callback server
    let (port, rx) = start_callback_server().map_err(|e| {
        tracing::error!("[OAuth] Failed to start callback server: {}", e);
        e
    })?;
    tracing::info!("[OAuth] Callback server listening on 127.0.0.1:{}", port);

    // For Apple: redirect_uri is HTTPS relay, state encodes port for relay → localhost
    // For Google: redirect_uri is localhost directly
    let (redirect_uri, wire_state) = match provider.as_str() {
        "apple" => (
            APPLE_RELAY_URL.to_string(),
            format!("{}.{}", csrf_state, port),
        ),
        _ => (
            format!("http://localhost:{}/callback", port),
            csrf_state.clone(),
        ),
    };
    tracing::info!("[OAuth] redirect_uri: {}", redirect_uri);

    // Build authorization URL
    let auth_url_base = match provider.as_str() {
        "google" => "https://accounts.google.com/o/oauth2/v2/auth",
        "apple" => "https://appleid.apple.com/auth/authorize",
        _ => return Err(format!("Unknown provider: {}", provider)),
    };

    let client_id = get_client_id(&provider)?;
    tracing::info!("[OAuth] client_id: {}...{}", &client_id[..8], &client_id[client_id.len()-8..]);

    let scope = match provider.as_str() {
        "google" => "openid email profile",
        "apple" => "openid email",
        _ => "openid email",
    };

    let mut auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        auth_url_base,
        urlencoding::encode(&client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(scope),
        urlencoding::encode(&wire_state),
        urlencoding::encode(&code_challenge),
    );

    if provider == "apple" {
        auth_url.push_str("&response_mode=form_post");
    }

    tracing::info!("[OAuth] Opening browser...");
    open::that(&auth_url).map_err(|e| {
        tracing::error!("[OAuth] Failed to open browser: {}", e);
        format!("Cannot open browser: {}", e)
    })?;
    tracing::info!("[OAuth] Browser opened, waiting for callback (120s timeout)...");

    // Wait for callback
    let (code, returned_state, error) = rx
        .recv_timeout(Duration::from_secs(125))
        .map_err(|e| {
            tracing::error!("[OAuth] Callback timeout: {}", e);
            "OAuth timeout: no callback received".to_string()
        })?;

    tracing::info!("[OAuth] Callback received — code_len={}, state_len={}, error='{}'",
        code.len(), returned_state.len(), error);

    if !error.is_empty() {
        tracing::error!("[OAuth] Provider returned error: {}", error);
        return Err(format!("OAuth cancelled: {}", error));
    }

    if code.is_empty() {
        tracing::error!("[OAuth] Empty code received (timeout?)");
        return Err("OAuth timeout: no callback received".to_string());
    }

    // Verify state (CSRF protection)
    let expected_state = if provider == "apple" {
        &wire_state
    } else {
        &csrf_state
    };
    if returned_state != *expected_state {
        tracing::error!("[OAuth] State mismatch! expected={}, got={}", expected_state, returned_state);
        return Err("OAuth state mismatch — possible CSRF attack".to_string());
    }

    tracing::info!("[OAuth] State verified, exchanging code for token...");
    let result = exchange_and_verify(&provider, &code, &code_verifier, &redirect_uri).await;
    match &result {
        Ok(identity) => tracing::info!("[OAuth] Success! email={}", identity.email),
        Err(e) => tracing::error!("[OAuth] Token exchange failed: {}", e),
    }
    result
}
