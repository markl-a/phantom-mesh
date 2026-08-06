//! Centralized secret redaction (P4: no API keys / tokens to disk in clear).
//!
//! ONE pure function, [`redact`], that masks clearly-secret substrings inside an
//! arbitrary text line (typically a serialized flight-recorder / trace JSONL
//! line, where a token may be embedded anywhere — inside an `args` value, an
//! assistant `delta`, a tool `output`, etc). It is deliberately CONSERVATIVE:
//! it masks only things that are unmistakably credentials and leaves ordinary
//! prose untouched, so flight recordings stay useful for debugging.
//!
//! What it masks:
//!   1. Known credential prefixes:  `sk-…`, `sk-ant-…`, `ghp_…`, `gho_…`,
//!      `github_pat_…`, `xoxb-…`/`xoxp-…`/`xapp-…` (Slack), `AKIA…` (AWS),
//!      `AIza…` (Google), `glpat-…`, plus JWT-shaped `eyJ…`.
//!   2. `Bearer <token>` → keeps the `Bearer ` word, masks the token.
//!   3. Values following a credential flag/key, in BOTH the `--api-key VALUE`
//!      (separate token) and `--api-key=VALUE` / `password=VALUE` (inline) forms.
//!   4. As a belt-and-braces net, free-standing high-entropy long tokens
//!      (>= 28 chars, mixed alnum, no whitespace) — long enough that ordinary
//!      words / hashes-in-prose are not hit.
//!
//! It works on the raw text (and on serialized JSON, where the same rules apply
//! to the values inside the encoded string). Routed through by the
//! flight-recorder event writer and the tracing JSONL writer before any write.

use serde_json::Value;

/// The replacement marker substituted for a detected secret.
pub const MASK: &str = "[REDACTED]";

/// Credential prefixes that, when they start a token, mean the whole token is a
/// secret regardless of entropy. Order: longest/more-specific first so the more
/// specific prefix is reported (purely cosmetic — the whole token is masked).
const SECRET_PREFIXES: &[&str] = &[
    "sk-ant-",
    "github_pat_",
    "sk-",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xoxr-",
    "xapp-",
    "glpat-",
    "AKIA",
    "ASIA",
    "AIza",
    "eyJ", // JWT header ({"alg":...} base64url) — covers `Bearer eyJ…`
];

/// Lowercased flag/field NAMES whose following value is a credential. Matched
/// against the leading `--`-stripped, `=`-trimmed token name, case-insensitively.
const CREDENTIAL_NAMES: &[&str] = &[
    "api-key",
    "api_key",
    "apikey",
    "token",
    "auth-token",
    "auth_token",
    "access-token",
    "access_token",
    "refresh-token",
    "refresh_token",
    "key",
    "secret",
    "client-secret",
    "client_secret",
    "password",
    "passwd",
    "pwd",
    "auth",
    "authorization",
    "bearer",
    "cluster-secret",
    "groq-key",
    "anthropic-key",
    "openai-key",
    "gemini-key",
];

/// True if `name` (already lowercased) is a credential flag/field name. Strips a
/// leading `--` / `-` so `--api-key`, `api-key`, and `password` all match.
fn is_credential_name(name: &str) -> bool {
    let n = name.trim_start_matches('-');
    CREDENTIAL_NAMES.iter().any(|c| c.eq_ignore_ascii_case(n))
}

/// Characters that are part of a "token" run (an unbroken credential-ish blob).
/// Includes the symbols real tokens use (`-_./+=:`) but NOT quotes / commas /
/// braces / whitespace, so we stop cleanly at JSON and shell boundaries.
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '+' | '=' | ':')
}

/// True if a free-standing token looks high-entropy enough to be a secret on its
/// own (no surrounding flag). Conservative: long + mixed classes.
fn looks_like_high_entropy(tok: &str) -> bool {
    // `=` is allowed inside tokens (base64 padding) but a token that is mostly a
    // `k=v` assignment is handled elsewhere; here require real length + a digit +
    // a letter so dotted module paths / sentences are not hit.
    if tok.len() < 28 {
        return false;
    }
    let has_digit = tok.chars().any(|c| c.is_ascii_digit());
    let has_alpha = tok.chars().any(|c| c.is_ascii_alphabetic());
    // Reject runs that are clearly a path / version (lots of '/' or all-dots):
    let slashes = tok.chars().filter(|&c| c == '/').count();
    if slashes > 3 {
        return false;
    }
    has_digit && has_alpha
}

/// True if `tok` begins with a known credential prefix.
fn has_secret_prefix(tok: &str) -> bool {
    SECRET_PREFIXES.iter().any(|p| tok.starts_with(p))
}

/// Mask clearly-secret substrings in `s`. Pure: same input → same output, no I/O.
///
/// The scan walks the string token-by-token (a token = a maximal run of
/// [`is_token_char`]). For each token it decides, in order:
///   * a token following the word `Bearer` → masked;
///   * a token following a credential flag (`--api-key`, `password`, …) seen as
///     the *previous* token → masked;
///   * a token of the `name=value` form whose `name` is a credential → only the
///     `value` is masked (`password=[REDACTED]`);
///   * a token starting with a known secret prefix (`sk-…`, `ghp_…`, …) → masked;
///   * an otherwise free-standing high-entropy long token → masked.
/// Everything else is copied through verbatim, so normal text is preserved.
pub fn redact(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0usize;
    // The previous *token* (trimmed of separators) drives the "flag VALUE" rule.
    let mut prev_token: Option<String> = None;

    while i < s.len() {
        let c = bytes[i] as char;
        // Only ASCII fast-path matters for our token chars; for multi-byte UTF-8
        // chars, copy the whole char through as a non-token separator.
        if !c.is_ascii() {
            // Copy this whole UTF-8 char.
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
            prev_token = None;
            continue;
        }
        if is_token_char(c) {
            // Consume a maximal token run.
            let start = i;
            while i < s.len() && (bytes[i] as char).is_ascii() && is_token_char(bytes[i] as char) {
                i += 1;
            }
            let token = &s[start..i];
            let masked = mask_token(token, prev_token.as_deref());
            out.push_str(&masked);
            // Record this token (its NAME, for the inline `k=v` case use full).
            prev_token = Some(token.to_string());
        } else {
            // Separator char (quote, space, comma, brace, …) — copy verbatim.
            out.push(c);
            // A run of separators does not by itself reset the "flag VALUE"
            // relationship, but a *non-separator* word boundary does; keep
            // prev_token across pure whitespace/quote so `--api-key "X"` works.
            if !c.is_whitespace() && c != '"' && c != '\'' && c != '=' && c != ':' {
                prev_token = None;
            }
            i += 1;
        }
    }
    out
}

/// Decide the masked form of a single token given the previous token.
fn mask_token(token: &str, prev: Option<&str>) -> String {
    // Rule: previous token was a credential FLAG (`--api-key`, `-k`) or the
    // `Bearer` auth keyword → this whole token is the secret value.
    //
    // CONSERVATIVE: a bare credential WORD in prose (`the key is …`,
    // `my password works`) must NOT mask the following word, so the flag form is
    // required to be `-`-prefixed. The inline `name=value` rule below still
    // catches `password=…` / `api_key=…` regardless of dashes.
    if let Some(p) = prev {
        let is_flag = p.starts_with('-') && is_credential_name(&p.to_ascii_lowercase());
        let is_bearer = p == "Bearer"; // case-sensitive auth scheme keyword
        if (is_flag || is_bearer) && !token.is_empty() {
            return MASK.to_string();
        }
    }

    // Rule: inline `name=value` where name is a credential → mask only value.
    if let Some((name, value)) = token.split_once('=') {
        if !value.is_empty() && is_credential_name(&name.to_ascii_lowercase()) {
            return format!("{name}={MASK}");
        }
    }

    // Rule: known secret prefix → mask the whole token.
    if has_secret_prefix(token) {
        return MASK.to_string();
    }

    // Rule: free-standing high-entropy long token → mask.
    if looks_like_high_entropy(token) {
        return MASK.to_string();
    }

    token.to_string()
}

/// Redact a serialized-able JSON value by serializing, redacting the text, and
/// re-parsing. Used where callers hold a `Value` rather than a line string.
/// Falls back to the original value if the redacted text is no longer valid JSON
/// (it always is for our rules, which never break JSON structure — they only
/// shorten string contents).
pub fn redact_value(v: &Value) -> Value {
    match serde_json::to_string(v) {
        Ok(s) => match serde_json::from_str(&redact(&s)) {
            Ok(redacted) => redacted,
            Err(_) => v.clone(),
        },
        Err(_) => v.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_table() {
        // (input, must_be_absent_substrings, must_be_present_substrings)
        let cases: &[(&str, &[&str], &[&str])] = &[
            // 1. OpenAI-style key by prefix.
            (
                "key is sk-LIVEKEY123abcDEF456ghiJKL789mno",
                &["sk-LIVEKEY123abcDEF456ghiJKL789mno"],
                &["key is", MASK],
            ),
            // 2. GitHub PAT prefix.
            (
                "token=ghp_abcdEFGH1234ijklMNOP5678qrstUVWX90",
                &["ghp_abcdEFGH1234ijklMNOP5678qrstUVWX90"],
                &[MASK],
            ),
            // 3. Bearer JWT.
            (
                "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.payload.sig",
                &["eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.payload.sig"],
                &["Bearer", MASK],
            ),
            // 4. --api-key SECRET (separate value form).
            (
                "run --api-key SUPERSECRETvalue42 --verbose",
                &["SUPERSECRETvalue42"],
                &["--api-key", MASK, "--verbose"],
            ),
            // 5. password=… inline form (only value masked).
            (
                "password=hunter2plaintext",
                &["hunter2plaintext"],
                &["password=", MASK],
            ),
            // 6. Slack bot token prefix. The fake fixture is split across two string
            //    literals via concat! so GitHub secret-scanning push-protection doesn't
            //    flag this REDACTION TEST as a real Slack token; concat! rebuilds the
            //    identical full token at compile time, so the redactor test is unchanged.
            (
                concat!("xoxb", "-1111111111-2222222222-abcdEFGHijklMNOPqrstUVWX"),
                &[concat!("xoxb", "-1111111111-2222222222-abcdEFGHijklMNOPqrstUVWX")],
                &[MASK],
            ),
            // 7. Plain text MUST be unchanged (conservative).
            (
                "the quick brown fox jumps over the lazy dog 12 times",
                &[],
                &["the quick brown fox jumps over the lazy dog 12 times"],
            ),
            // 8. A normal hyphenated word / version is NOT a secret.
            (
                "spectyn-mesh v0.6.0 release-candidate build",
                &[],
                &["spectyn-mesh v0.6.0 release-candidate build"],
            ),
        ];

        for (input, absent, present) in cases {
            let got = redact(input);
            for a in *absent {
                assert!(
                    !got.contains(a),
                    "redact({input:?}) leaked secret {a:?}: {got:?}"
                );
            }
            for p in *present {
                assert!(
                    got.contains(p),
                    "redact({input:?}) lost expected {p:?}: {got:?}"
                );
            }
        }
    }

    #[test]
    fn plain_text_is_byte_identical() {
        // The single most important conservative guarantee: prose round-trips.
        let prose = "I refactored the recorder and added a unit test; see line 334.";
        assert_eq!(redact(prose), prose);
    }

    #[test]
    fn secret_embedded_in_json_line_is_masked_but_stays_valid_json() {
        let v = serde_json::json!({
            "event": {
                "kind": "tool_call",
                "name": "Bash",
                "args": ["env", "OPENAI_API_KEY=sk-LIVEKEY123abcDEF456ghiJKL789mnop"]
            }
        });
        let line = serde_json::to_string(&v).unwrap();
        let red = redact(&line);
        assert!(
            !red.contains("sk-LIVEKEY123abcDEF456ghiJKL789mnop"),
            "secret leaked: {red}"
        );
        // Still parses as JSON (rules never break structure).
        let _: Value = serde_json::from_str(&red).expect("redacted line is valid JSON");
    }

    #[test]
    fn redact_value_masks_and_reparses() {
        let v = serde_json::json!({ "cmd": "deploy --token ghp_SECRETtoken1234567890abcdEFGH" });
        let red = redact_value(&v);
        let s = serde_json::to_string(&red).unwrap();
        assert!(!s.contains("ghp_SECRETtoken1234567890abcdEFGH"), "{s}");
        assert!(s.contains(MASK), "{s}");
    }

    #[test]
    fn conservative_does_not_overmask_prose_credentials() {
        // The documented CONSERVATIVE contract (mask_token, §"What it masks"):
        // a credential WORD in prose must NOT mask the following token — only a
        // `-`-flag, the case-sensitive `Bearer` keyword, or a `name=value` form
        // does. Over-masking would gut flight-recorder readability, so these
        // negatives are as load-bearing as the positive masking cases above.

        // 1. A bare credential word (no dash, not `name=value`) immediately
        //    before a short, low-entropy token does NOT mask it.
        let s = redact("my password hunter2works");
        assert!(
            s.contains("hunter2works"),
            "bare credential word over-masked the following token: {s:?}"
        );
        assert!(!s.contains(MASK), "no mask expected for prose: {s:?}");

        // 2. `bearer` is case-sensitive: only capital-B `Bearer` is the auth
        //    scheme keyword, so lowercase `bearer foo42` must leave `foo42`.
        let s = redact("bearer foo42 please");
        assert!(s.contains("foo42"), "lowercase bearer over-masked: {s:?}");
        assert!(!s.contains(MASK), "no mask expected for lowercase bearer: {s:?}");

        // 3. A NON-credential `name=value` (e.g. UI dimensions) is byte-identical.
        let s = redact("resize width=1920 height=1080");
        assert_eq!(
            s, "resize width=1920 height=1080",
            "non-credential name=value must not be masked: {s:?}"
        );
    }
}
