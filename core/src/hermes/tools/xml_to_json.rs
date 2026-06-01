//! `hermes_xml_to_json` — minimal XML → JSON converter.
//!
//! In-house tokeniser + tree builder; no new crates pulled in.
//! Conversion rules (chosen to match common patterns; documented so
//! callers can rely on them):
//!   * Each element becomes an object: `{ "<tag>": <value> }`.
//!   * Element with no children + no attrs → string of its inner text.
//!   * Element with attrs → attrs become `@name` keys; text becomes `#text`.
//!   * Repeated child tags become a JSON array.
//!   * Self-closing tags (`<x/>`) → `{ "x": null }`.
//!   * XML declarations (`<?xml ...?>`) and comments are skipped.
//!
//! NOT supported (kept out of scope to stay ≤150 LOC): namespaces,
//! CDATA sections, entities other than the five XML built-ins.

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use super::{HermesTool, ToolError, ToolResult};

/// Max nesting depth for XML elements; guards against stack overflow on
/// adversarial input (e.g. `<a><a><a>...` thousands deep).
const MAX_DEPTH: usize = 256;
/// Max input length in bytes (1 MiB); guards against pathological
/// O(n^2) tokeniser cost on huge attacker-controlled blobs.
const MAX_INPUT_LEN: usize = 1_048_576;

pub struct XmlToJson;

#[async_trait]
impl HermesTool for XmlToJson {
    fn name(&self) -> &'static str {
        "hermes_xml_to_json"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "hermes_xml_to_json",
                "description": "Parse a small XML string and return a JSON tree. \
                    Attributes become `@name` keys; repeated children become arrays. \
                    Does NOT support namespaces, CDATA, or custom entities.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "xml": {"type": "string"}
                    },
                    "required": ["xml"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let xml = args
            .get("xml")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("xml required".into()))?;
        if xml.len() > MAX_INPUT_LEN {
            return Err(ToolError::BadArgs(format!(
                "xml input size {} exceeds max {} bytes (too large)",
                xml.len(),
                MAX_INPUT_LEN
            )));
        }
        let node = parse_root(xml).map_err(ToolError::Invalid)?;
        Ok(node)
    }
}

fn parse_root(src: &str) -> Result<Value, String> {
    let tokens = tokenise(src)?;
    let mut idx = 0;
    let node = parse_element(&tokens, &mut idx, 0)?;
    Ok(node)
}

#[derive(Debug)]
enum Tok<'a> {
    Open {
        name: &'a str,
        attrs: Vec<(String, String)>,
        self_close: bool,
    },
    Close {
        name: &'a str,
    },
    Text(String),
}

fn tokenise(src: &str) -> Result<Vec<Tok<'_>>, String> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Skip XML decl / comments.
            if src[i..].starts_with("<?") {
                let end = src[i..].find("?>").ok_or("unterminated <?...?>")? + i + 2;
                i = end;
                continue;
            }
            if src[i..].starts_with("<!--") {
                let end = src[i..].find("-->").ok_or("unterminated comment")? + i + 3;
                i = end;
                continue;
            }
            let close_tag = src[i..].starts_with("</");
            let end_tag = src[i..].find('>').ok_or("unterminated tag")? + i;
            let inside = &src[if close_tag { i + 2 } else { i + 1 }..end_tag];
            let self_close = inside.ends_with('/');
            let inside = if self_close {
                &inside[..inside.len() - 1]
            } else {
                inside
            };
            let inside = inside.trim();
            let (name, rest) = match inside.find(char::is_whitespace) {
                Some(p) => (&inside[..p], inside[p..].trim()),
                None => (inside, ""),
            };
            if close_tag {
                out.push(Tok::Close { name });
            } else {
                let attrs = parse_attrs(rest)?;
                out.push(Tok::Open {
                    name,
                    attrs,
                    self_close,
                });
            }
            i = end_tag + 1;
        } else {
            let next = src[i..].find('<').map(|p| p + i).unwrap_or(bytes.len());
            let raw = &src[i..next];
            if !raw.trim().is_empty() {
                out.push(Tok::Text(decode_entities(raw)));
            }
            i = next;
        }
    }
    Ok(out)
}

fn parse_attrs(src: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    let mut chars = src.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let mut key = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' || c.is_whitespace() {
                break;
            }
            key.push(c);
            chars.next();
        }
        // skip '=' and quote
        while let Some(&c) = chars.peek() {
            if c == '=' {
                chars.next();
                break;
            }
            chars.next();
        }
        let quote = match chars.next() {
            Some(q @ ('"' | '\'')) => q,
            _ => return Err(format!("bad attr quote near {}", key)),
        };
        let mut val = String::new();
        for c in chars.by_ref() {
            if c == quote {
                break;
            } else {
                val.push(c);
            }
        }
        out.push((key, decode_entities(&val)));
    }
    Ok(out)
}

fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn parse_element(toks: &[Tok<'_>], idx: &mut usize, depth: usize) -> Result<Value, String> {
    if depth >= MAX_DEPTH {
        return Err(format!("XML nesting depth exceeds {}", MAX_DEPTH));
    }
    let (name, attrs, self_close) = match toks.get(*idx) {
        Some(Tok::Open {
            name,
            attrs,
            self_close,
        }) => (name.to_string(), attrs.clone(), *self_close),
        _ => return Err("expected open tag".into()),
    };
    *idx += 1;
    let mut wrapper = Map::new();
    if self_close {
        let inner = if attrs.is_empty() {
            Value::Null
        } else {
            let mut m = Map::new();
            for (k, v) in attrs {
                m.insert(format!("@{}", k), Value::String(v));
            }
            Value::Object(m)
        };
        wrapper.insert(name, inner);
        return Ok(Value::Object(wrapper));
    }
    let mut children: Map<String, Value> = Map::new();
    let mut text = String::new();
    while let Some(t) = toks.get(*idx) {
        match t {
            Tok::Close { name: cn } if *cn == name.as_str() => {
                *idx += 1;
                break;
            }
            Tok::Open { .. } => {
                let child = parse_element(toks, idx, depth + 1)?;
                if let Value::Object(m) = child {
                    for (k, v) in m {
                        merge(&mut children, k, v);
                    }
                }
            }
            Tok::Text(s) => {
                text.push_str(s);
                *idx += 1;
            }
            Tok::Close { name: cn } => return Err(format!("unexpected </{}>", cn)),
        }
    }
    let inner = if attrs.is_empty() && children.is_empty() {
        Value::String(text)
    } else {
        let mut m = Map::new();
        for (k, v) in attrs {
            m.insert(format!("@{}", k), Value::String(v));
        }
        for (k, v) in children {
            m.insert(k, v);
        }
        if !text.trim().is_empty() {
            m.insert("#text".into(), Value::String(text));
        }
        Value::Object(m)
    };
    wrapper.insert(name, inner);
    Ok(Value::Object(wrapper))
}

fn merge(map: &mut Map<String, Value>, key: String, value: Value) {
    match map.remove(&key) {
        None => {
            map.insert(key, value);
        }
        Some(Value::Array(mut arr)) => {
            arr.push(value);
            map.insert(key, Value::Array(arr));
        }
        Some(prev) => {
            map.insert(key, Value::Array(vec![prev, value]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn simple_element_with_text() {
        let tool = XmlToJson;
        let r = tool.call(&json!({"xml": "<a>hello</a>"})).await.unwrap();
        assert_eq!(r, json!({"a": "hello"}));
    }

    #[tokio::test]
    async fn attributes_become_at_keys() {
        let tool = XmlToJson;
        let r = tool
            .call(&json!({"xml": r#"<book id="42">Title</book>"#}))
            .await
            .unwrap();
        assert_eq!(r, json!({"book": {"@id": "42", "#text": "Title"}}));
    }

    #[tokio::test]
    async fn repeated_children_become_array() {
        let tool = XmlToJson;
        let r = tool
            .call(&json!({"xml": "<list><i>a</i><i>b</i></list>"}))
            .await
            .unwrap();
        assert_eq!(r, json!({"list": {"i": ["a", "b"]}}));
    }

    #[tokio::test]
    async fn rejects_deeply_nested_xml_at_max_depth() {
        let tool = XmlToJson;
        let n = 300;
        let xml = "<a>".repeat(n) + "x" + &"</a>".repeat(n);
        let err = tool.call(&json!({"xml": xml})).await.unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.to_lowercase().contains("depth"),
            "expected depth-limit error, got: {}",
            msg
        );
    }

    #[tokio::test]
    async fn accepts_xml_at_safe_depth_50() {
        let tool = XmlToJson;
        let n = 50;
        let xml = "<a>".repeat(n) + "x" + &"</a>".repeat(n);
        let r = tool.call(&json!({"xml": xml})).await;
        assert!(r.is_ok(), "expected Ok at depth 50, got {:?}", r);
    }

    #[tokio::test]
    async fn rejects_oversized_xml_input() {
        let tool = XmlToJson;
        // 2 MiB body padded inside a single root element.
        let payload = "x".repeat(2 * 1024 * 1024);
        let xml = format!("<a>{}</a>", payload);
        let err = tool.call(&json!({"xml": xml})).await.unwrap_err();
        let msg = format!("{}", err).to_lowercase();
        assert!(
            msg.contains("size") || msg.contains("too large"),
            "expected size-limit error, got: {}",
            msg
        );
    }
}
