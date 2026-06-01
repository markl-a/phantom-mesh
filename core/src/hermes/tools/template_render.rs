//! `hermes_template_render` — minimal `{{name}}` substitution from a
//! JSON variable map. Intentionally non-Turing-complete (no loops, no
//! conditionals) to keep the surface small and predictable.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{HermesTool, ToolError, ToolResult};

pub struct TemplateRender;

#[async_trait]
impl HermesTool for TemplateRender {
    fn name(&self) -> &'static str {
        "hermes_template_render"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "hermes_template_render",
                "description": "Substitute `{{name}}` placeholders in `template` with values from `vars`. \
                    `name` must match `[A-Za-z0-9_.]+`. By default missing keys are an error; \
                    pass `strict=false` to substitute the empty string instead.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "template": {"type": "string"},
                        "vars":     {"type": "object"},
                        "strict":   {"type": "boolean"}
                    },
                    "required": ["template", "vars"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let template = args
            .get("template")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("template required".into()))?;
        let vars = args
            .get("vars")
            .and_then(|v| v.as_object())
            .ok_or_else(|| ToolError::BadArgs("vars must be an object".into()))?;
        let strict = args.get("strict").and_then(|v| v.as_bool()).unwrap_or(true);

        let mut out = String::with_capacity(template.len());
        let bytes = template.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
                // Find matching '}}'.
                let rest = &template[i + 2..];
                let end_rel = rest
                    .find("}}")
                    .ok_or_else(|| ToolError::Invalid("unterminated {{ ... }}".into()))?;
                let name = rest[..end_rel].trim();
                if name.is_empty()
                    || !name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
                {
                    return Err(ToolError::Invalid(format!(
                        "invalid placeholder name: {:?}",
                        name
                    )));
                }
                match vars.get(name) {
                    Some(v) => out.push_str(&render_value(v)),
                    None => {
                        if strict {
                            return Err(ToolError::Invalid(format!("missing var: {}", name)));
                        }
                    }
                }
                i += 2 + end_rel + 2;
            } else {
                let ch_end = next_char_boundary(bytes, i);
                out.push_str(&template[i..ch_end]);
                i = ch_end;
            }
        }
        Ok(json!({ "text": out }))
    }
}

fn next_char_boundary(bytes: &[u8], i: usize) -> usize {
    let first = bytes[i];
    let width = if first < 0x80 {
        1
    } else if first < 0xC0 {
        1
    } else if first < 0xE0 {
        2
    } else if first < 0xF0 {
        3
    } else {
        4
    };
    (i + width).min(bytes.len())
}

fn render_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn substitutes_simple_placeholders() {
        let tool = TemplateRender;
        let r = tool
            .call(&json!({
                "template": "Hello {{name}}, you have {{count}} new messages.",
                "vars":     {"name": "Alice", "count": 3}
            }))
            .await
            .unwrap();
        assert_eq!(r["text"], "Hello Alice, you have 3 new messages.");
    }

    #[tokio::test]
    async fn missing_var_is_invalid_in_strict_mode() {
        let tool = TemplateRender;
        let err = tool
            .call(&json!({
                "template": "{{a}} {{b}}",
                "vars":     {"a": "x"}
            }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Invalid(_)));
    }

    #[tokio::test]
    async fn non_strict_mode_substitutes_empty_for_missing() {
        let tool = TemplateRender;
        let r = tool
            .call(&json!({
                "template": "[{{a}}][{{b}}]",
                "vars":     {"a": "x"},
                "strict":   false
            }))
            .await
            .unwrap();
        assert_eq!(r["text"], "[x][]");
    }
}
