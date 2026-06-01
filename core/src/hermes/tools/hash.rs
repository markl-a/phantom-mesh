//! `hermes_hash` — sha256 or sha512 of input text.
//!
//! Uses the existing `sha2` + `hex` deps (no new Cargo cost). Treats
//! `input` as UTF-8 bytes; if you need to hash raw bytes, encode
//! base64 first and hash that string (caller's choice — keeps surface
//! single-arg-string simple).

use async_trait::async_trait;
use serde_json::{json, Value};
use sha2::{Digest, Sha256, Sha512};

use super::{HermesTool, ToolError, ToolResult};

pub struct Hash;

#[async_trait]
impl HermesTool for Hash {
    fn name(&self) -> &'static str {
        "hermes_hash"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "hermes_hash",
                "description": "Return the lowercase hex digest of `input` under `algorithm` \
                    (one of: \"sha256\", \"sha512\"). UTF-8 bytes are hashed.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "input":     {"type": "string"},
                        "algorithm": {"type": "string", "enum": ["sha256", "sha512"]}
                    },
                    "required": ["input", "algorithm"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let input = args
            .get("input")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("input required".into()))?;
        let algo = args
            .get("algorithm")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("algorithm required".into()))?;
        let digest = match algo {
            "sha256" => {
                let mut h = Sha256::new();
                h.update(input.as_bytes());
                hex::encode(h.finalize())
            }
            "sha512" => {
                let mut h = Sha512::new();
                h.update(input.as_bytes());
                hex::encode(h.finalize())
            }
            other => return Err(ToolError::BadArgs(format!("unknown algorithm: {}", other))),
        };
        Ok(json!({ "digest": digest, "algorithm": algo }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sha256_of_known_input_matches_spec_vector() {
        let tool = Hash;
        let r = tool
            .call(&json!({"input": "abc", "algorithm": "sha256"}))
            .await
            .unwrap();
        // RFC 6234 test vector.
        assert_eq!(
            r["digest"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(r["algorithm"], "sha256");
    }

    #[tokio::test]
    async fn sha512_of_known_input_matches_spec_vector() {
        let tool = Hash;
        let r = tool
            .call(&json!({"input": "abc", "algorithm": "sha512"}))
            .await
            .unwrap();
        // RFC 6234 test vector (single contiguous 128-char hex literal).
        assert_eq!(
            r["digest"],
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
    }

    #[tokio::test]
    async fn unknown_algorithm_is_bad_args() {
        let tool = Hash;
        let err = tool
            .call(&json!({"input": "x", "algorithm": "md5"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::BadArgs(_)));
    }
}
