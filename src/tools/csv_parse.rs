//! CSV parse tool — read, query, and analyze CSV data.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

use super::{Tool, ToolResult};

pub struct CsvParseTool;

impl CsvParseTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl Tool for CsvParseTool {
    fn name(&self) -> &str { "csv_parse" }

    fn description(&self) -> &str {
        "Parse and query CSV data. Operations: headers, count, select, filter, stats, to_json."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "data": { "type": "string", "description": "CSV content as string" },
                "operation": { "type": "string", "description": "One of: headers, count, select, filter, stats, to_json" },
                "columns": { "type": "array", "items": { "type": "string" }, "description": "Columns to select (for 'select' operation)" },
                "filter_column": { "type": "string", "description": "Column name to filter by" },
                "filter_value": { "type": "string", "description": "Value to match" },
                "limit": { "type": "integer", "description": "Max rows to return (default 100)" }
            },
            "required": ["data", "operation"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let data = args["data"].as_str().unwrap_or("").trim();
        let operation = args["operation"].as_str().unwrap_or("").trim();
        let limit = args["limit"].as_u64().unwrap_or(100) as usize;

        if data.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing required parameter: data".into() });
        }
        if operation.is_empty() {
            return Ok(ToolResult { success: false, output: "Missing required parameter: operation".into() });
        }

        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .from_reader(data.as_bytes());

        let headers: Vec<String> = match reader.headers() {
            Ok(h) => h.iter().map(|s| s.to_string()).collect(),
            Err(e) => return Ok(ToolResult { success: false, output: format!("Failed to parse CSV headers: {}", e) }),
        };

        if headers.is_empty() {
            return Ok(ToolResult { success: false, output: "CSV has no headers".into() });
        }

        let result = match operation {
            "headers" => {
                serde_json::to_string(&headers)?
            }
            "count" => {
                let count = reader.records().count();
                count.to_string()
            }
            "select" => {
                let columns: Vec<String> = args["columns"].as_array()
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_else(|| headers.clone());

                let col_indices: Vec<usize> = columns.iter()
                    .filter_map(|col| headers.iter().position(|h| h == col))
                    .collect();

                let mut rows: Vec<Vec<String>> = Vec::new();
                for record in reader.records().take(limit) {
                    if let Ok(rec) = record {
                        let row: Vec<String> = col_indices.iter()
                            .map(|&i| rec.get(i).unwrap_or("").to_string())
                            .collect();
                        rows.push(row);
                    }
                }
                json!({"columns": columns, "rows": rows}).to_string()
            }
            "filter" => {
                let filter_col = args["filter_column"].as_str().unwrap_or("").trim();
                let filter_val = args["filter_value"].as_str().unwrap_or("").trim();
                if filter_col.is_empty() {
                    return Ok(ToolResult { success: false, output: "filter operation requires filter_column".into() });
                }
                let col_idx = match headers.iter().position(|h| h == filter_col) {
                    Some(i) => i,
                    None => return Ok(ToolResult { success: false, output: format!("Column '{}' not found", filter_col) }),
                };

                let mut rows: Vec<HashMap<String, String>> = Vec::new();
                for record in reader.records().take(limit * 10) {
                    if let Ok(rec) = record {
                        if rec.get(col_idx).map(|v| v == filter_val).unwrap_or(false) {
                            let row: HashMap<String, String> = headers.iter()
                                .enumerate()
                                .map(|(i, h)| (h.clone(), rec.get(i).unwrap_or("").to_string()))
                                .collect();
                            rows.push(row);
                            if rows.len() >= limit { break; }
                        }
                    }
                }
                json!({"matched": rows.len(), "rows": rows}).to_string()
            }
            "stats" => {
                let mut col_values: HashMap<String, Vec<f64>> = HashMap::new();
                for record in reader.records() {
                    if let Ok(rec) = record {
                        for (i, h) in headers.iter().enumerate() {
                            if let Some(val) = rec.get(i) {
                                if let Ok(num) = val.parse::<f64>() {
                                    col_values.entry(h.clone()).or_default().push(num);
                                }
                            }
                        }
                    }
                }

                let mut stats = serde_json::Map::new();
                for (col, vals) in &col_values {
                    if vals.is_empty() { continue; }
                    let min = vals.iter().cloned().fold(f64::INFINITY, f64::min);
                    let max = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                    let sum: f64 = vals.iter().sum();
                    let avg = sum / vals.len() as f64;
                    stats.insert(col.clone(), json!({
                        "count": vals.len(),
                        "min": min,
                        "max": max,
                        "avg": (avg * 100.0).round() / 100.0,
                        "sum": sum,
                    }));
                }
                serde_json::to_string(&stats)?
            }
            "to_json" => {
                let mut rows: Vec<HashMap<String, String>> = Vec::new();
                for record in reader.records().take(limit) {
                    if let Ok(rec) = record {
                        let row: HashMap<String, String> = headers.iter()
                            .enumerate()
                            .map(|(i, h)| (h.clone(), rec.get(i).unwrap_or("").to_string()))
                            .collect();
                        rows.push(row);
                    }
                }
                serde_json::to_string(&rows)?
            }
            _ => return Ok(ToolResult { success: false, output: format!("Unknown operation: '{}'. Use: headers, count, select, filter, stats, to_json", operation) }),
        };

        Ok(ToolResult { success: true, output: result })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_CSV: &str = "name,age,score\nalice,30,85\nbob,25,92\ncharlie,35,78";

    #[test]
    fn test_name() {
        assert_eq!(CsvParseTool::new().name(), "csv_parse");
    }

    #[test]
    fn test_schema() {
        let tool = CsvParseTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["data"].is_object());
        assert!(schema["properties"]["operation"].is_object());
    }

    #[tokio::test]
    async fn test_headers() {
        let tool = CsvParseTool::new();
        let result = tool.execute(json!({"data": TEST_CSV, "operation": "headers"})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("name"));
        assert!(result.output.contains("age"));
        assert!(result.output.contains("score"));
    }

    #[tokio::test]
    async fn test_count() {
        let tool = CsvParseTool::new();
        let result = tool.execute(json!({"data": TEST_CSV, "operation": "count"})).await.unwrap();
        assert!(result.success);
        assert_eq!(result.output, "3");
    }

    #[tokio::test]
    async fn test_select() {
        let tool = CsvParseTool::new();
        let result = tool.execute(json!({"data": TEST_CSV, "operation": "select", "columns": ["name", "score"]})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("alice"));
        assert!(result.output.contains("85"));
    }

    #[tokio::test]
    async fn test_filter() {
        let tool = CsvParseTool::new();
        let result = tool.execute(json!({"data": TEST_CSV, "operation": "filter", "filter_column": "name", "filter_value": "bob"})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("bob"));
        assert!(!result.output.contains("alice"));
    }

    #[tokio::test]
    async fn test_to_json() {
        let tool = CsvParseTool::new();
        let result = tool.execute(json!({"data": TEST_CSV, "operation": "to_json"})).await.unwrap();
        assert!(result.success);
        let parsed: Vec<HashMap<String, String>> = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed.len(), 3);
    }

    #[tokio::test]
    async fn test_stats() {
        let tool = CsvParseTool::new();
        let result = tool.execute(json!({"data": TEST_CSV, "operation": "stats"})).await.unwrap();
        assert!(result.success);
        let stats: Value = serde_json::from_str(&result.output).unwrap();
        assert!(stats["age"]["min"].as_f64().unwrap() == 25.0);
        assert!(stats["age"]["max"].as_f64().unwrap() == 35.0);
    }

    #[tokio::test]
    async fn test_empty_data() {
        let tool = CsvParseTool::new();
        let result = tool.execute(json!({"data": "", "operation": "count"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_limit() {
        let tool = CsvParseTool::new();
        let result = tool.execute(json!({"data": TEST_CSV, "operation": "to_json", "limit": 1})).await.unwrap();
        assert!(result.success);
        let parsed: Vec<Value> = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed.len(), 1);
    }
}
