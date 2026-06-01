//! `hermes_jq` — JSON manipulation via dotted paths with a tiny filter
//! grammar. Extension of `hermes_json_query` that adds:
//!   * splat:  `users[*].name`            → array of names
//!   * filter: `users[?age>30].name`      → array of names where age>30
//!
//! Filter grammar (intentionally tiny — no AND/OR, no nested paths,
//! one comparison per `[?...]`): `<key> <op> <literal>` where
//! `op` ∈ {`==`, `!=`, `<`, `<=`, `>`, `>=`} and literal is a quoted
//! string or a JSON number.
//!
//! Returns `{ "value": <result> }` for single hits or
//! `{ "values": [...] }` for splat / filter.
//!
//! Pure-Rust; no new crates pulled in.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{HermesTool, ToolError, ToolResult};

pub struct Jq;

#[async_trait]
impl HermesTool for Jq {
    fn name(&self) -> &'static str {
        "hermes_jq"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "hermes_jq",
                "description": "Query JSON via dotted path with splat (`[*]`) and \
                    filter (`[?key OP literal]`) where OP is one of ==, !=, <, <=, >, >=. \
                    Examples: 'users[*].name', 'users[?age>30].name'. \
                    Returns {value: ...} for single hit, {values: [...]} for collections.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "json": {"description": "JSON document to query."},
                        "path": {"type": "string", "description": "Path expression."}
                    },
                    "required": ["json", "path"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let doc = args
            .get("json")
            .ok_or_else(|| ToolError::BadArgs("json required".into()))?;
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("path required".into()))?;
        let segs = parse_segments(path).map_err(ToolError::Invalid)?;
        let collected = walk(&[doc], &segs).map_err(ToolError::Invalid)?;
        // If the path contained no splat/filter, collected has exactly 1 element →
        // emit `{value: ...}`. Otherwise emit `{values: [...]}`.
        let multi = segs
            .iter()
            .any(|s| matches!(s, Segment::Splat | Segment::Filter(_)));
        if multi {
            Ok(json!({ "values": collected }))
        } else {
            let v = collected.into_iter().next().unwrap_or(Value::Null);
            Ok(json!({ "value": v }))
        }
    }
}

#[derive(Debug)]
enum Segment {
    Key(String),
    Index(usize),
    Splat,
    Filter(FilterExpr),
}

#[derive(Debug)]
struct FilterExpr {
    key: String,
    op: String,
    lit: Value,
}

fn parse_segments(path: &str) -> Result<Vec<Segment>, String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '.' => {
                if !buf.is_empty() {
                    out.push(Segment::Key(std::mem::take(&mut buf)));
                }
            }
            '[' => {
                if !buf.is_empty() {
                    out.push(Segment::Key(std::mem::take(&mut buf)));
                }
                let mut inner = String::new();
                for nc in chars.by_ref() {
                    if nc == ']' {
                        break;
                    }
                    inner.push(nc);
                }
                let inner = inner.trim();
                if inner == "*" {
                    out.push(Segment::Splat);
                } else if let Some(rest) = inner.strip_prefix('?') {
                    out.push(Segment::Filter(parse_filter(rest.trim())?));
                } else {
                    let i: usize = inner.parse().map_err(|_| format!("bad index: {}", inner))?;
                    out.push(Segment::Index(i));
                }
            }
            other => buf.push(other),
        }
    }
    if !buf.is_empty() {
        out.push(Segment::Key(buf));
    }
    Ok(out)
}

fn parse_filter(s: &str) -> Result<FilterExpr, String> {
    // Find the longest matching operator first so '>=' beats '>'.
    for op in ["==", "!=", "<=", ">=", "<", ">"] {
        if let Some(idx) = s.find(op) {
            let key = s[..idx].trim().to_string();
            let rhs = s[idx + op.len()..].trim();
            if key.is_empty() {
                return Err("filter key empty".into());
            }
            let lit = parse_literal(rhs)?;
            return Ok(FilterExpr {
                key,
                op: op.into(),
                lit,
            });
        }
    }
    Err(format!("no operator in filter: {}", s))
}

fn parse_literal(s: &str) -> Result<Value, String> {
    if let Some(rest) = s.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        return Ok(Value::String(rest.into()));
    }
    if let Some(rest) = s.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        return Ok(Value::String(rest.into()));
    }
    if let Ok(n) = s.parse::<f64>() {
        return Ok(json!(n));
    }
    if s == "true" {
        return Ok(json!(true));
    }
    if s == "false" {
        return Ok(json!(false));
    }
    if s == "null" {
        return Ok(Value::Null);
    }
    Err(format!("bad literal: {}", s))
}

fn walk(roots: &[&Value], segs: &[Segment]) -> Result<Vec<Value>, String> {
    let mut current: Vec<Value> = roots.iter().map(|v| (*v).clone()).collect();
    for seg in segs {
        let mut next: Vec<Value> = Vec::new();
        for v in &current {
            match seg {
                Segment::Key(k) => {
                    if let Some(x) = v.get(k) {
                        next.push(x.clone());
                    }
                }
                Segment::Index(i) => {
                    if let Some(x) = v.get(*i) {
                        next.push(x.clone());
                    }
                }
                Segment::Splat => {
                    if let Some(arr) = v.as_array() {
                        for item in arr {
                            next.push(item.clone());
                        }
                    } else if let Some(obj) = v.as_object() {
                        for (_k, item) in obj {
                            next.push(item.clone());
                        }
                    }
                }
                Segment::Filter(f) => {
                    let arr = v
                        .as_array()
                        .ok_or_else(|| "filter applied to non-array".to_string())?;
                    for item in arr {
                        if matches_filter(item, f) {
                            next.push(item.clone());
                        }
                    }
                }
            }
        }
        current = next;
    }
    Ok(current)
}

fn matches_filter(v: &Value, f: &FilterExpr) -> bool {
    let lhs = match v.get(&f.key) {
        Some(x) => x,
        None => return false,
    };
    match f.op.as_str() {
        "==" => lhs == &f.lit,
        "!=" => lhs != &f.lit,
        "<" | "<=" | ">" | ">=" => {
            let a = lhs.as_f64();
            let b = f.lit.as_f64();
            match (a, b) {
                (Some(a), Some(b)) => match f.op.as_str() {
                    "<" => a < b,
                    "<=" => a <= b,
                    ">" => a > b,
                    ">=" => a >= b,
                    _ => false,
                },
                _ => false,
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn splat_collects_all_array_items() {
        let tool = Jq;
        let r = tool
            .call(&json!({
                "json": {"users": [{"name": "alice"}, {"name": "bob"}]},
                "path": "users[*].name"
            }))
            .await
            .unwrap();
        assert_eq!(r["values"], json!(["alice", "bob"]));
    }

    #[tokio::test]
    async fn filter_keeps_only_matching_items() {
        let tool = Jq;
        let r = tool
            .call(&json!({
                "json": {"users": [
                    {"name": "alice", "age": 32},
                    {"name": "bob",   "age": 25},
                    {"name": "cara",  "age": 41}
                ]},
                "path": "users[?age>30].name"
            }))
            .await
            .unwrap();
        assert_eq!(r["values"], json!(["alice", "cara"]));
    }

    #[tokio::test]
    async fn plain_dotted_path_returns_single_value() {
        let tool = Jq;
        let r = tool
            .call(&json!({
                "json": {"a": {"b": 7}},
                "path": "a.b"
            }))
            .await
            .unwrap();
        assert_eq!(r["value"], 7);
    }
}
