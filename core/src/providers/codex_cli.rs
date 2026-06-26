//! Codex CLI (ChatGPT subscription) credential discovery.
//!
//! The OpenAI Codex CLI caches its login at `~/.codex/auth.json` (or
//! `$CODEX_HOME/auth.json`). Two shapes exist:
//!
//!   - **ChatGPT subscription (OAuth)** — drives the ChatGPT-account backend
//!     (`chatgpt.com/backend-api/codex/responses`) rather than the metered
//!     `api.openai.com` platform API:
//!     ```json
//!     { "OPENAI_API_KEY": null,
//!       "tokens": { "access_token": "...", "refresh_token": "...",
//!                   "id_token": "...", "account_id": "..." },
//!       "last_refresh": "..." }
//!     ```
//!   - **API key** — a plain platform key usable against `api.openai.com`:
//!     ```json
//!     { "OPENAI_API_KEY": "sk-...", "tokens": null }
//!     ```
//!
//! NOTE: using a ChatGPT subscription token from a non-Codex client is NOT
//! publicly sanctioned by OpenAI and MAY get the account flagged. This module is
//! detection-only — it *reads* whatever the official CLI cached. There is a
//! SEPARATE, OPT-IN, disclosed path (`providers::openai_oauth` + `phantom auth
//! chatgpt`) that mints a token via the public Codex OAuth client and writes it
//! to a phantom-owned `~/.phantom-mesh/openai_oauth.json` (added to
//! `codex_paths()`); that path carries the account-flag risk and is never a
//! default. The official `~/.codex/auth.json` is never written by phantom.

use serde_json::Value;

/// Resolved Codex credential + how it must be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAuth {
    /// Bearer token: the ChatGPT OAuth `access_token`, or the `OPENAI_API_KEY`.
    pub token: String,
    /// ChatGPT account id (`chatgpt-account-id` header) — present in OAuth mode.
    pub account_id: Option<String>,
    /// `true` → ChatGPT subscription OAuth (use the ChatGPT backend + Responses
    /// protocol). `false` → a plain API key (use api.openai.com chat/completions).
    pub is_oauth: bool,
}

/// Extract a [`CodexAuth`] from a parsed `auth.json`.
pub fn extract_codex_auth(json: &Value) -> Option<CodexAuth> {
    // API-key mode wins when a non-empty platform key is present.
    if let Some(key) = json
        .get("OPENAI_API_KEY")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Some(CodexAuth {
            token: key.to_string(),
            account_id: None,
            is_oauth: false,
        });
    }
    // ChatGPT subscription (OAuth) mode.
    if let Some(tokens) = json.get("tokens") {
        if let Some(access) = tokens
            .get("access_token")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            let account_id = tokens
                .get("account_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);
            return Some(CodexAuth {
                token: access.to_string(),
                account_id,
                is_oauth: true,
            });
        }
    }
    None
}

/// Locate the current Codex credential from the local CLI cache.
///
/// Tries `$CODEX_HOME/auth.json` then `~/.codex/auth.json`. Returns the live
/// credential so callers use the auto-refreshed token rather than a stale copy.
pub fn find_codex_auth() -> Option<CodexAuth> {
    for path in super::credential_scanner::codex_paths() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(json) = serde_json::from_str::<Value>(&content) {
                if let Some(auth) = extract_codex_auth(&json) {
                    return Some(auth);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_chatgpt_oauth() {
        let j = serde_json::json!({
            "OPENAI_API_KEY": null,
            "tokens": { "access_token": "eyJ-access", "refresh_token": "r", "account_id": "acct_123" },
            "last_refresh": "2026-06-01T00:00:00Z"
        });
        let a = extract_codex_auth(&j).unwrap();
        assert_eq!(a.token, "eyJ-access");
        assert_eq!(a.account_id.as_deref(), Some("acct_123"));
        assert!(a.is_oauth);
    }

    #[test]
    fn extracts_api_key_mode() {
        let j = serde_json::json!({ "OPENAI_API_KEY": "sk-proj-xyz", "tokens": null });
        let a = extract_codex_auth(&j).unwrap();
        assert_eq!(a.token, "sk-proj-xyz");
        assert!(!a.is_oauth);
        assert!(a.account_id.is_none());
    }

    #[test]
    fn none_when_empty() {
        assert!(extract_codex_auth(&serde_json::json!({ "OPENAI_API_KEY": null, "tokens": null })).is_none());
    }
}
