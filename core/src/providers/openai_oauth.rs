//! "Sign in with ChatGPT" subscription OAuth (opt-in) — mint a ChatGPT-subscription
//! access token from phantom itself instead of only reading the official Codex
//! CLI's cache.
//!
//! ⚠️ RISK / DISCLOSURE (owner-accepted, opt-in only): this drives the ChatGPT
//! subscription backend using the PUBLIC Codex CLI OAuth client. OpenAI does NOT
//! publicly document third-party use of this flow, and using a ChatGPT
//! subscription token from a non-official client MAY get the account flagged or
//! banned. This path is therefore OPT-IN, never a default, and the CLI command
//! prints this warning before running it. The legitimate, zero-risk alternative
//! is a metered `api.openai.com` API key (paste flow) — see onboarding.
//!
//! The minted token is stored in a phantom-owned, codex-SHAPED file
//! (`~/.phantom-mesh/openai_oauth.json`) which is added to
//! `credential_scanner::codex_paths()`, so the existing `codex_oauth::run_codex`
//! runtime picks it up unchanged. The official `~/.codex/auth.json` is NEVER
//! touched.
//!
//! Flow (matches common CLI OAuth device flows): PKCE S256 → browser to
//! `auth.openai.com/oauth/authorize` → loopback capture on
//! `http://localhost:1455/auth/callback` (or paste the redirect URL) → exchange
//! at `auth.openai.com/oauth/token` → `{access_token, refresh_token, id_token}`;
//! `chatgpt_account_id` is read from the access-token JWT claims. Refresh on
//! expiry via the stored `refresh_token`.

use base64::Engine;
use serde_json::Value;
use std::path::{Path, PathBuf};

// Public Codex CLI OAuth client (PKCE, no secret). The redirect_uri is fixed by
// what this client_id has registered — it MUST be exactly this.
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const SCOPES: &str = "openid profile email offline_access";
/// Loopback port — fixed because the redirect_uri above is registered to it.
pub const CALLBACK_PORT: u16 = 1455;

/// Minted ChatGPT-subscription tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub account_id: Option<String>,
}

/// `<home>/.phantom-mesh/openai_oauth.json` — phantom-owned, codex-shaped.
pub fn storage_path(home: &Path) -> PathBuf {
    home.join(".phantom-mesh").join("openai_oauth.json")
}

/// Build the authorize URL for a fresh PKCE verifier. Returns `(url, verifier,
/// state)`; the caller keeps verifier+state for the exchange / CSRF check.
pub fn authorize_url(verifier: &str, state: &str) -> String {
    let challenge = {
        let digest = sha2::Sha256::digest_str(verifier);
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
    };
    format!(
        "{AUTHORIZE_URL}?response_type=code&client_id={}&redirect_uri={}&scope={}\
         &state={}&code_challenge={}&code_challenge_method=S256",
        urlencoding::encode(CLIENT_ID),
        urlencoding::encode(REDIRECT_URI),
        urlencoding::encode(SCOPES),
        urlencoding::encode(state),
        urlencoding::encode(&challenge),
    )
}

/// Decode the `chatgpt_account_id` from a ChatGPT access-token JWT. The id lives
/// under the `https://api.openai.com/auth` claim. Returns None if absent/malformed.
pub fn account_id_from_access_token(jwt: &str) -> Option<String> {
    let payload_b64 = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .get("https://api.openai.com/auth")
        .and_then(|a| a.get("chatgpt_account_id"))
        .and_then(|v| v.as_str())
        .map(String::from)
        // fallback shapes seen in the wild
        .or_else(|| claims.get("chatgpt_account_id").and_then(|v| v.as_str()).map(String::from))
}

/// Read the `exp` (unix seconds) claim from a JWT, if present.
pub fn jwt_exp_secs(jwt: &str) -> Option<u64> {
    let payload_b64 = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims.get("exp").and_then(|v| v.as_u64())
}

/// True when `exp` is within `skew_secs` of `now_secs` (or already past).
pub fn is_expired(exp_secs: u64, now_secs: u64, skew_secs: u64) -> bool {
    exp_secs <= now_secs.saturating_add(skew_secs)
}

/// Serialize tokens into the codex `auth.json` shape `extract_codex_auth` reads.
pub fn to_codex_shaped_json(t: &OpenAiTokens) -> Value {
    serde_json::json!({
        "OPENAI_API_KEY": Value::Null,
        "tokens": {
            "access_token": t.access_token,
            "refresh_token": t.refresh_token,
            "id_token": t.id_token,
            "account_id": t.account_id,
        },
        // marker so a reader can tell this is phantom-minted (vs official codex).
        "minted_by": "phantom",
    })
}

/// Parse the OAuth token endpoint response into [`OpenAiTokens`], folding in the
/// account id from the access-token JWT.
pub fn parse_token_response(body: &Value) -> Option<OpenAiTokens> {
    let access_token = body.get("access_token").and_then(|v| v.as_str())?.to_string();
    let account_id = account_id_from_access_token(&access_token);
    Some(OpenAiTokens {
        refresh_token: body.get("refresh_token").and_then(|v| v.as_str()).map(String::from),
        id_token: body.get("id_token").and_then(|v| v.as_str()).map(String::from),
        account_id,
        access_token,
    })
}

/// Atomically persist tokens to the phantom-owned codex-shaped file (0600).
pub fn save(home: &Path, t: &OpenAiTokens) -> std::io::Result<()> {
    let path = storage_path(home);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(&to_codex_shaped_json(t)).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Load tokens from the phantom-owned file, if present + parseable.
pub fn load(home: &Path) -> Option<OpenAiTokens> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(storage_path(home)).ok()?).ok()?;
    let tokens = v.get("tokens")?;
    let access_token = tokens.get("access_token").and_then(|x| x.as_str()).filter(|s| !s.is_empty())?;
    Some(OpenAiTokens {
        access_token: access_token.to_string(),
        refresh_token: tokens.get("refresh_token").and_then(|x| x.as_str()).map(String::from),
        id_token: tokens.get("id_token").and_then(|x| x.as_str()).map(String::from),
        account_id: tokens.get("account_id").and_then(|x| x.as_str()).map(String::from),
    })
}

/// Exchange an authorization `code` (with its PKCE `verifier`) for tokens.
pub async fn exchange_code(client: &reqwest::Client, code: &str, verifier: &str) -> Result<OpenAiTokens, String> {
    let resp = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(|e| format!("token request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.map_err(|e| format!("token response not JSON: {e}"))?;
    if !status.is_success() {
        return Err(format!("token endpoint {status}: {body}"));
    }
    parse_token_response(&body).ok_or_else(|| "token response missing access_token".to_string())
}

/// Refresh using a stored `refresh_token`.
pub async fn refresh(client: &reqwest::Client, refresh_token: &str) -> Result<OpenAiTokens, String> {
    let resp = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .map_err(|e| format!("refresh request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.map_err(|e| format!("refresh response not JSON: {e}"))?;
    if !status.is_success() {
        return Err(format!("refresh endpoint {status}: {body}"));
    }
    let mut t = parse_token_response(&body).ok_or_else(|| "refresh missing access_token".to_string())?;
    // OpenAI may omit the refresh_token on refresh — keep the old one.
    if t.refresh_token.is_none() {
        t.refresh_token = Some(refresh_token.to_string());
    }
    Ok(t)
}

/// Refresh-on-use (#3): if a phantom-minted token exists and its JWT `exp` is
/// within 60s, refresh it and rewrite the file. Best-effort; never panics.
pub async fn ensure_fresh_if_present(home: &Path, now_secs: u64) {
    let Some(t) = load(home) else { return };
    let needs = jwt_exp_secs(&t.access_token).is_some_and(|exp| is_expired(exp, now_secs, 60));
    if !needs {
        return;
    }
    let Some(rt) = t.refresh_token.clone() else { return };
    let client = reqwest::Client::new();
    if let Ok(fresh) = refresh(&client, &rt).await {
        let _ = save(home, &fresh);
    }
}

/// Random PKCE verifier — 64 hex chars (two v4 UUIDs), within the PKCE charset
/// and the 43–128 length bound.
pub fn gen_verifier() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Bind the fixed loopback port, wait for the OAuth redirect, verify `state`,
/// and return the `code`. One-shot + blocking. The fixed port is mandated by the
/// client_id's registered redirect_uri.
pub fn capture_code_on_loopback(expected_state: &str) -> Result<String, String> {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind(("127.0.0.1", CALLBACK_PORT)).map_err(|e| {
        format!(
            "could not bind 127.0.0.1:{CALLBACK_PORT}: {e} \
             (close the official `codex` login if it's holding the port)"
        )
    })?;
    let (mut stream, _) = listener
        .accept()
        .map_err(|e| format!("callback accept failed: {e}"))?;
    let mut buf = [0u8; 8192];
    let n = stream
        .read(&mut buf)
        .map_err(|e| format!("callback read failed: {e}"))?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let (code, state) = parse_callback_query(&req)?;
    let _ = stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
          <html><body style='font-family:sans-serif'><h2>Signed in \xe2\x9c\x93</h2>\
          <p>Return to your terminal \xe2\x80\x94 you can close this tab.</p></body></html>",
    );
    if state != expected_state {
        return Err("state mismatch (possible CSRF) — aborted".into());
    }
    Ok(code)
}

/// Parse `code` + `state` from an HTTP request's first line
/// (`GET /auth/callback?code=..&state=..`). Surfaces an `error=` param too.
fn parse_callback_query(req: &str) -> Result<(String, String), String> {
    let first = req.lines().next().unwrap_or("");
    let path = first
        .split_whitespace()
        .nth(1)
        .ok_or("malformed callback request")?;
    let query = path
        .split('?')
        .nth(1)
        .ok_or("callback had no query string (no code)")?;
    let (mut code, mut state) = (None, None);
    for kv in query.split('&') {
        let mut it = kv.splitn(2, '=');
        match (it.next(), it.next()) {
            (Some("code"), Some(v)) => code = Some(urldecode(v)),
            (Some("state"), Some(v)) => state = Some(urldecode(v)),
            (Some("error"), Some(v)) => {
                return Err(format!("authorize returned error: {}", urldecode(v)))
            }
            _ => {}
        }
    }
    Ok((code.ok_or("callback missing `code`")?, state.unwrap_or_default()))
}

fn urldecode(s: &str) -> String {
    urlencoding::decode(s)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| s.to_string())
}

// sha2 convenience (the crate exposes Digest; mirror oauth.rs's usage).
trait ShaStr {
    fn digest_str(s: &str) -> Vec<u8>;
}
impl ShaStr for sha2::Sha256 {
    fn digest_str(s: &str) -> Vec<u8> {
        use sha2::Digest;
        sha2::Sha256::digest(s.as_bytes()).to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt_with(payload: serde_json::Value) -> String {
        let p = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        format!("h.{p}.s")
    }

    #[test]
    fn authorize_url_has_pkce_and_fixed_redirect() {
        let u = authorize_url("verifier123", "state456");
        assert!(u.starts_with(AUTHORIZE_URL));
        assert!(u.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
        assert!(u.contains("code_challenge_method=S256"));
        assert!(u.contains(&urlencoding::encode(REDIRECT_URI).into_owned()));
        assert!(u.contains("state=state456"));
        assert!(u.contains(&urlencoding::encode(SCOPES).into_owned()));
    }

    #[test]
    fn account_id_extracted_from_nested_claim() {
        let jwt = jwt_with(serde_json::json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "acct_xyz" }, "exp": 99
        }));
        assert_eq!(account_id_from_access_token(&jwt).as_deref(), Some("acct_xyz"));
    }

    #[test]
    fn account_id_none_when_absent() {
        assert!(account_id_from_access_token(&jwt_with(serde_json::json!({"exp": 1}))).is_none());
    }

    #[test]
    fn exp_and_expiry_logic() {
        let jwt = jwt_with(serde_json::json!({ "exp": 1000 }));
        assert_eq!(jwt_exp_secs(&jwt), Some(1000));
        assert!(is_expired(1000, 1000, 0)); // exactly at exp → expired
        assert!(is_expired(1000, 950, 60)); // within skew → expired
        assert!(!is_expired(1000, 800, 60)); // far enough → fresh
    }

    #[test]
    fn parse_token_response_folds_account_id() {
        let access = jwt_with(serde_json::json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "acct_1" }
        }));
        let body = serde_json::json!({
            "access_token": access, "refresh_token": "r1", "id_token": "i1"
        });
        let t = parse_token_response(&body).unwrap();
        assert_eq!(t.refresh_token.as_deref(), Some("r1"));
        assert_eq!(t.account_id.as_deref(), Some("acct_1"));
    }

    #[test]
    fn parse_callback_extracts_code_and_state() {
        let req = "GET /auth/callback?code=abc123&state=st42 HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let (code, state) = parse_callback_query(req).unwrap();
        assert_eq!(code, "abc123");
        assert_eq!(state, "st42");
    }

    #[test]
    fn parse_callback_surfaces_error_param() {
        let req = "GET /auth/callback?error=access_denied HTTP/1.1\r\n\r\n";
        assert!(parse_callback_query(req).unwrap_err().contains("access_denied"));
    }

    #[test]
    fn gen_verifier_is_valid_pkce_length() {
        let v = gen_verifier();
        assert!(v.len() >= 43 && v.len() <= 128, "len {}", v.len());
        assert!(v.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn save_load_round_trip_codex_shaped() {
        let home = tempfile::tempdir().unwrap();
        let t = OpenAiTokens {
            access_token: "a".into(),
            refresh_token: Some("r".into()),
            id_token: Some("i".into()),
            account_id: Some("acct".into()),
        };
        save(home.path(), &t).unwrap();
        // codex-shaped: extract_codex_auth must read it as OAuth mode
        let raw: Value = serde_json::from_str(
            &std::fs::read_to_string(storage_path(home.path())).unwrap(),
        )
        .unwrap();
        let ca = crate::providers::codex_cli::extract_codex_auth(&raw).unwrap();
        assert_eq!(ca.token, "a");
        assert_eq!(ca.account_id.as_deref(), Some("acct"));
        assert!(ca.is_oauth);
        // and our own loader round-trips
        assert_eq!(load(home.path()).unwrap(), t);
    }
}
