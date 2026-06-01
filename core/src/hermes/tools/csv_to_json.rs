//! `hermes_csv_to_json` — parse a CSV string (first row = header) and
//! return a JSON array of objects.
//!
//! Minimal in-house parser: supports `""` escape, quoted newlines, CRLF.

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use super::{HermesTool, ToolError, ToolResult};

pub struct CsvToJson;

#[async_trait]
impl HermesTool for CsvToJson {
    fn name(&self) -> &'static str {
        "hermes_csv_to_json"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "hermes_csv_to_json",
                "description": "Parse CSV `text` whose first row is the header. \
                    Returns an array of objects keyed by header name. \
                    `delimiter` (default ',') must be a single ASCII character.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text":      {"type": "string"},
                        "delimiter": {"type": "string"}
                    },
                    "required": ["text"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::BadArgs("text required".into()))?;
        let delim_str = args
            .get("delimiter")
            .and_then(|v| v.as_str())
            .unwrap_or(",");
        if delim_str.chars().count() != 1 || !delim_str.is_ascii() {
            return Err(ToolError::BadArgs(
                "delimiter must be a single ASCII char".into(),
            ));
        }
        let delim = delim_str.chars().next().unwrap();

        let rows = parse_csv(text, delim);
        let mut iter = rows.into_iter();
        let header = match iter.next() {
            Some(h) => h,
            None => return Ok(json!({ "rows": [] })),
        };
        let mut out = Vec::new();
        for row in iter {
            let mut obj = Map::new();
            for (i, key) in header.iter().enumerate() {
                let val = row.get(i).cloned().unwrap_or_default();
                obj.insert(key.clone(), Value::String(val));
            }
            out.push(Value::Object(obj));
        }
        Ok(json!({ "rows": out }))
    }
}

/// Parse CSV into rows-of-strings. Trailing empty line (CRLF or LF at EOF)
/// is dropped.
pub(crate) fn parse_csv(text: &str, delim: char) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == delim {
            row.push(std::mem::take(&mut field));
        } else if c == '\n' || c == '\r' {
            // End of row. Eat \r\n as a unit.
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            rows.push(std::mem::take(&mut row));
        } else {
            field.push(c);
        }
    }
    // Flush dangling field/row if input did not end with a newline.
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parses_basic_csv() {
        let tool = CsvToJson;
        let r = tool
            .call(&json!({"text": "name,age\nalice,30\nbob,25"}))
            .await
            .unwrap();
        let rows = r["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], "alice");
        assert_eq!(rows[0]["age"], "30");
        assert_eq!(rows[1]["name"], "bob");
        assert_eq!(rows[1]["age"], "25");
    }

    #[tokio::test]
    async fn handles_quoted_fields_with_commas_and_escapes() {
        let tool = CsvToJson;
        let r = tool
            .call(&json!({
                "text": "a,b\n\"hello, world\",\"she said \"\"hi\"\"\""
            }))
            .await
            .unwrap();
        let rows = r["rows"].as_array().unwrap();
        assert_eq!(rows[0]["a"], "hello, world");
        assert_eq!(rows[0]["b"], r#"she said "hi""#);
    }

    #[tokio::test]
    async fn empty_input_returns_empty_rows() {
        let tool = CsvToJson;
        let r = tool.call(&json!({"text": ""})).await.unwrap();
        assert_eq!(r["rows"], json!([]));
    }
}
