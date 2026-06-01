//! `hermes_json_to_csv` — render an array of JSON objects as CSV with a
//! header row. Reverse of `hermes_csv_to_json` for the common case.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{HermesTool, ToolError, ToolResult};

pub struct JsonToCsv;

#[async_trait]
impl HermesTool for JsonToCsv {
    fn name(&self) -> &'static str {
        "hermes_json_to_csv"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "hermes_json_to_csv",
                "description": "Render `rows` (array of objects) as CSV with a header row. \
                    If `columns` is provided, use that order; else union of keys in \
                    first-appearance order. `delimiter` defaults to ','.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "rows":      {"type": "array"},
                        "columns":   {"type": "array", "items": {"type": "string"}},
                        "delimiter": {"type": "string"}
                    },
                    "required": ["rows"]
                }
            }
        })
    }

    async fn call(&self, args: &Value) -> ToolResult {
        let rows_val = args
            .get("rows")
            .ok_or_else(|| ToolError::BadArgs("rows required".into()))?;
        let rows = rows_val
            .as_array()
            .ok_or_else(|| ToolError::BadArgs("rows must be an array".into()))?;
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

        let columns: Vec<String> = match args.get("columns").and_then(|v| v.as_array()) {
            Some(cols) => cols
                .iter()
                .map(|c| {
                    c.as_str().map(|s| s.to_string()).ok_or_else(|| {
                        ToolError::BadArgs("columns must be array of strings".into())
                    })
                })
                .collect::<Result<_, _>>()?,
            None => {
                let mut seen: Vec<String> = Vec::new();
                for r in rows {
                    let obj = r.as_object().ok_or_else(|| {
                        ToolError::Invalid("each row must be a JSON object".into())
                    })?;
                    for k in obj.keys() {
                        if !seen.iter().any(|s| s == k) {
                            seen.push(k.clone());
                        }
                    }
                }
                seen
            }
        };

        let mut out = String::new();
        push_csv_row(&mut out, columns.iter().map(|s| s.as_str()), delim);
        for r in rows {
            let obj = r
                .as_object()
                .ok_or_else(|| ToolError::Invalid("each row must be a JSON object".into()))?;
            let cells: Vec<String> = columns.iter().map(|c| render_cell(obj.get(c))).collect();
            push_csv_row(&mut out, cells.iter().map(|s| s.as_str()), delim);
        }
        Ok(json!({ "csv": out }))
    }
}

fn push_csv_row<'a, I: Iterator<Item = &'a str>>(out: &mut String, fields: I, delim: char) {
    let mut first = true;
    for f in fields {
        if !first {
            out.push(delim);
        }
        first = false;
        if needs_quoting(f, delim) {
            out.push('"');
            for c in f.chars() {
                if c == '"' {
                    out.push('"');
                }
                out.push(c);
            }
            out.push('"');
        } else {
            out.push_str(f);
        }
    }
    out.push('\n');
}

fn needs_quoting(s: &str, delim: char) -> bool {
    s.chars()
        .any(|c| c == delim || c == '"' || c == '\n' || c == '\r')
}

fn render_cell(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn renders_objects_to_csv_with_header() {
        let tool = JsonToCsv;
        // NOTE: serde_json::Map is a BTreeMap by default (no `preserve_order`
        // feature), so key iteration yields keys in lexicographic order.
        // The "first-appearance" column ordering therefore degenerates to
        // alphabetical when columns are inferred from the rows. Explicit
        // `columns` (see the third test) lets the caller override this.
        let r = tool
            .call(&json!({
                "rows": [{"name": "alice", "age": 30}, {"name": "bob", "age": 25}]
            }))
            .await
            .unwrap();
        let csv = r["csv"].as_str().unwrap();
        assert_eq!(csv, "age,name\n30,alice\n25,bob\n");
    }

    #[tokio::test]
    async fn quotes_fields_with_commas_and_quotes() {
        let tool = JsonToCsv;
        let r = tool
            .call(&json!({
                "rows": [{"a": "hello, world", "b": "she said \"hi\""}]
            }))
            .await
            .unwrap();
        let csv = r["csv"].as_str().unwrap();
        assert_eq!(csv, "a,b\n\"hello, world\",\"she said \"\"hi\"\"\"\n");
    }

    #[tokio::test]
    async fn explicit_columns_control_order_and_subset() {
        let tool = JsonToCsv;
        let r = tool
            .call(&json!({
                "rows": [{"a": 1, "b": 2, "c": 3}],
                "columns": ["c", "a"]
            }))
            .await
            .unwrap();
        assert_eq!(r["csv"].as_str().unwrap(), "c,a\n3,1\n");
    }
}
