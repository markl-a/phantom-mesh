//! `hermes_jaro_winkler` — Jaro and Jaro-Winkler string similarity.
//!
//! Two metrics in [0.0, 1.0] where 1.0 is identical. Jaro-Winkler boosts
//! Jaro for strings sharing a common prefix (max 4 chars, scaling factor
//! p=0.1 — Winkler's original parameters).
//!
//! Pure Rust, no external crates.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{HermesTool, ToolError, ToolResult};

pub struct JaroWinkler;

#[async_trait]
impl HermesTool for JaroWinkler {
    fn name(&self) -> &'static str {
        "hermes_jaro_winkler"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "hermes_jaro_winkler",
                "description": "Compute Jaro and Jaro-Winkler similarity in [0.0, 1.0] for strings `a` and `b`. \
                    Returns {jaro, jaro_winkler}. Jaro-Winkler boosts scores for strings sharing a common prefix.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "a": {"type": "string"},
                        "b": {"type": "string"}
                    },
                    "required": ["a", "b"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        const MAX_INPUT_LEN: usize = 4096;
        let a = args
            .get("a")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("a required".into()))?;
        let b = args
            .get("b")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("b required".into()))?;
        if a.chars().count() > MAX_INPUT_LEN {
            return Err(ToolError::BadArgs("a too long: max 4096 chars".into()));
        }
        if b.chars().count() > MAX_INPUT_LEN {
            return Err(ToolError::BadArgs("b too long: max 4096 chars".into()));
        }
        let j = jaro(a, b);
        let jw = jaro_winkler(a, b, j);
        Ok(json!({ "jaro": j, "jaro_winkler": jw }))
    }
}

pub(crate) fn jaro(a: &str, b: &str) -> f64 {
    let s1: Vec<char> = a.chars().collect();
    let s2: Vec<char> = b.chars().collect();
    if s1.is_empty() && s2.is_empty() {
        return 1.0;
    }
    if s1.is_empty() || s2.is_empty() {
        return 0.0;
    }
    let max_dist = (s1.len().max(s2.len()) / 2).saturating_sub(1);
    let mut s1_matches = vec![false; s1.len()];
    let mut s2_matches = vec![false; s2.len()];
    let mut matches = 0;
    for i in 0..s1.len() {
        let lo = i.saturating_sub(max_dist);
        let hi = (i + max_dist + 1).min(s2.len());
        for j in lo..hi {
            if s2_matches[j] {
                continue;
            }
            if s1[i] != s2[j] {
                continue;
            }
            s1_matches[i] = true;
            s2_matches[j] = true;
            matches += 1;
            break;
        }
    }
    if matches == 0 {
        return 0.0;
    }
    let mut t = 0.0;
    let mut k = 0;
    for i in 0..s1.len() {
        if !s1_matches[i] {
            continue;
        }
        while !s2_matches[k] {
            k += 1;
        }
        if s1[i] != s2[k] {
            t += 0.5;
        }
        k += 1;
    }
    let m = matches as f64;
    (m / s1.len() as f64 + m / s2.len() as f64 + (m - t) / m) / 3.0
}

pub(crate) fn jaro_winkler(a: &str, b: &str, jaro_score: f64) -> f64 {
    // Common prefix up to 4 chars.
    let prefix_len = a
        .chars()
        .zip(b.chars())
        .take(4)
        .take_while(|(x, y)| x == y)
        .count();
    jaro_score + prefix_len as f64 * 0.1 * (1.0 - jaro_score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn identical_strings_score_one() {
        let tool = JaroWinkler;
        let r = tool
            .call(&json!({"a": "martha", "b": "martha"}))
            .await
            .unwrap();
        assert_eq!(r["jaro"], 1.0);
        assert_eq!(r["jaro_winkler"], 1.0);
    }

    #[tokio::test]
    async fn classic_martha_marhta_matches_reference() {
        // Reference: jaro = 0.9444..., jaro-winkler ≈ 0.9611
        let tool = JaroWinkler;
        let r = tool
            .call(&json!({"a": "MARTHA", "b": "MARHTA"}))
            .await
            .unwrap();
        let j = r["jaro"].as_f64().unwrap();
        let jw = r["jaro_winkler"].as_f64().unwrap();
        assert!((j - 0.9444444).abs() < 1e-4, "jaro = {}", j);
        assert!((jw - 0.9611111).abs() < 1e-4, "jw = {}", jw);
    }

    #[tokio::test]
    async fn jaro_winkler_boosts_strings_sharing_prefix() {
        let tool = JaroWinkler;
        let r = tool
            .call(&json!({"a": "DWAYNE", "b": "DUANE"}))
            .await
            .unwrap();
        let j = r["jaro"].as_f64().unwrap();
        let jw = r["jaro_winkler"].as_f64().unwrap();
        // Shared prefix "D" → JW should be strictly greater than Jaro.
        assert!(jw > j, "jw {} should exceed jaro {}", jw, j);
    }

    #[tokio::test]
    async fn rejects_input_over_max_len_fast() {
        let tool = JaroWinkler;
        let big: String = "x".repeat(10_001);
        let start = std::time::Instant::now();
        let r = tool.call(&json!({"a": big, "b": "short"})).await;
        let elapsed = start.elapsed();
        assert!(r.is_err(), "10001-char input must be rejected");
        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "rejection must be fast (<50ms), took {:?}",
            elapsed
        );
        // Same check for `b`.
        let big2: String = "y".repeat(10_001);
        let r2 = tool.call(&json!({"a": "short", "b": big2})).await;
        assert!(r2.is_err(), "10001-char b must be rejected");
    }
}
