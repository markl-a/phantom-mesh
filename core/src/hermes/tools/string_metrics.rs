//! `hermes_string_metrics` — Levenshtein edit distance + a normalised
//! similarity ratio in [0.0, 1.0].
//!
//! Two-row DP, O(min(n,m)) memory. Strings are compared as `char`
//! sequences (Unicode scalar values), so `café` vs `cafe` = distance 1.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{HermesTool, ToolError, ToolResult};

pub struct StringMetrics;

#[async_trait]
impl HermesTool for StringMetrics {
    fn name(&self) -> &'static str {
        "hermes_string_metrics"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "hermes_string_metrics",
                "description": "Compute Levenshtein edit distance between `a` and `b` (by Unicode chars), \
                    plus a similarity ratio in [0.0, 1.0] = 1 - distance / max(len_a, len_b). \
                    Both empty → ratio 1.0.",
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
        let a = args
            .get("a")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("a required".into()))?;
        let b = args
            .get("b")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("b required".into()))?;
        let distance = levenshtein(a, b);
        let len_a = a.chars().count();
        let len_b = b.chars().count();
        let denom = len_a.max(len_b);
        let similarity = if denom == 0 {
            1.0
        } else {
            1.0 - (distance as f64) / (denom as f64)
        };
        Ok(json!({ "distance": distance, "similarity": similarity }))
    }
}

pub(crate) fn levenshtein(a: &str, b: &str) -> usize {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    if ac.is_empty() {
        return bc.len();
    }
    if bc.is_empty() {
        return ac.len();
    }
    // Ensure bc is the shorter so we allocate O(min) memory.
    let (s, t) = if ac.len() < bc.len() {
        (&bc, &ac)
    } else {
        (&ac, &bc)
    };
    let mut prev: Vec<usize> = (0..=t.len()).collect();
    let mut curr: Vec<usize> = vec![0; t.len() + 1];
    for (i, sc) in s.iter().enumerate() {
        curr[0] = i + 1;
        for (j, tc) in t.iter().enumerate() {
            let cost = if sc == tc { 0 } else { 1 };
            curr[j + 1] = (curr[j] + 1).min(prev[j + 1] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[t.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn identical_strings_have_distance_zero_and_full_similarity() {
        let tool = StringMetrics;
        let r = tool
            .call(&json!({"a": "kitten", "b": "kitten"}))
            .await
            .unwrap();
        assert_eq!(r["distance"], 0);
        assert_eq!(r["similarity"], 1.0);
    }

    #[tokio::test]
    async fn classic_kitten_sitting_distance_is_three() {
        let tool = StringMetrics;
        let r = tool
            .call(&json!({"a": "kitten", "b": "sitting"}))
            .await
            .unwrap();
        assert_eq!(r["distance"], 3);
        // 1 - 3/7 ≈ 0.5714285714...
        let sim = r["similarity"].as_f64().unwrap();
        assert!((sim - (1.0 - 3.0 / 7.0)).abs() < 1e-9, "got {}", sim);
    }

    #[tokio::test]
    async fn both_empty_strings_yield_similarity_one() {
        let tool = StringMetrics;
        let r = tool.call(&json!({"a": "", "b": ""})).await.unwrap();
        assert_eq!(r["distance"], 0);
        assert_eq!(r["similarity"], 1.0);
    }
}
