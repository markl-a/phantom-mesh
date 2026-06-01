//! `hermes_random_string` — secure random string generator.
//!
//! Three encodings:
//!   * `alphanumeric`: A-Z a-z 0-9 (62 chars), `length` = output char count
//!   * `hex`: lowercase hex, `length` = output char count (must be even
//!     for clean byte boundary; odd lengths just emit length/2 bytes,
//!     truncated to length chars)
//!   * `base64`: URL-safe base64 (no padding), `length` = output char count
//!
//! Uses `OsRng` via the existing `rand` dep — cryptographically secure.

use async_trait::async_trait;
use base64::Engine;
use rand::{rngs::OsRng, RngCore};
use serde_json::{json, Value};

use super::{HermesTool, ToolError, ToolResult};

pub struct RandomString;

#[async_trait]
impl HermesTool for RandomString {
    fn name(&self) -> &'static str {
        "hermes_random_string"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "hermes_random_string",
                "description": "Generate a cryptographically secure random string. \
                    `encoding` ∈ {alphanumeric, hex, base64} (URL-safe base64, no padding). \
                    `length` is the output char count (1..=4096).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "encoding": { "type": "string", "enum": ["alphanumeric", "hex", "base64"] },
                        "length":   { "type": "integer", "minimum": 1, "maximum": 4096 }
                    },
                    "required": ["encoding", "length"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let enc = args
            .get("encoding")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("encoding required".into()))?;
        let len =
            args.get("length")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| ToolError::BadArgs("length required".into()))? as usize;
        if !(1..=4096).contains(&len) {
            return Err(ToolError::BadArgs("length must be 1..=4096".into()));
        }
        let s = match enc {
            "alphanumeric" => gen_alphanumeric(len),
            "hex" => {
                let bytes_needed = len.div_ceil(2);
                let mut bytes = vec![0u8; bytes_needed];
                OsRng.fill_bytes(&mut bytes);
                let mut s = hex::encode(bytes);
                s.truncate(len);
                s
            }
            "base64" => {
                // URL-safe no-pad: 4 b64 chars per 3 bytes, ceil.
                let bytes_needed = (len * 3).div_ceil(4);
                let mut bytes = vec![0u8; bytes_needed];
                OsRng.fill_bytes(&mut bytes);
                let mut s = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
                s.truncate(len);
                s
            }
            other => return Err(ToolError::BadArgs(format!("unknown encoding: {}", other))),
        };
        Ok(json!({ "value": s }))
    }
}

fn gen_alphanumeric(len: usize) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut out = String::with_capacity(len);
    let mut buf = [0u8; 32];
    let mut i = 0;
    while i < len {
        OsRng.fill_bytes(&mut buf);
        for &b in &buf {
            if i >= len {
                break;
            }
            // Rejection-sample to avoid modulo bias on the small 62-char set.
            // 256 mod 62 = 8 → values 248..=255 (highest 4 cycles' tails) leak bias,
            // so simply skip them.
            if b >= 248 {
                continue;
            }
            out.push(CHARS[(b as usize) % CHARS.len()] as char);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn alphanumeric_length_and_alphabet() {
        let tool = RandomString;
        let r = tool
            .call(&json!({"encoding": "alphanumeric", "length": 32}))
            .await
            .unwrap();
        let s = r["value"].as_str().unwrap();
        assert_eq!(s.len(), 32);
        assert!(
            s.chars().all(|c| c.is_ascii_alphanumeric()),
            "alphanumeric output should only contain A-Z a-z 0-9: {}",
            s
        );
    }

    #[tokio::test]
    async fn hex_length_and_alphabet() {
        let tool = RandomString;
        let r = tool
            .call(&json!({"encoding": "hex", "length": 16}))
            .await
            .unwrap();
        let s = r["value"].as_str().unwrap();
        assert_eq!(s.len(), 16);
        assert!(
            s.chars().all(|c| c.is_ascii_hexdigit()),
            "hex output should only contain hex digits: {}",
            s
        );
    }

    #[tokio::test]
    async fn out_of_range_length_is_bad_args() {
        let tool = RandomString;
        let err = tool
            .call(&json!({"encoding": "hex", "length": 0}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::BadArgs(_)));
        let err = tool
            .call(&json!({"encoding": "hex", "length": 9999}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::BadArgs(_)));
    }
}
