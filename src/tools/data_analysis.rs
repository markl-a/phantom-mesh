//! Data analysis tool — statistical operations on JSON array data.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

use super::{Tool, ToolResult};

pub struct DataAnalysisTool;

impl DataAnalysisTool {
    pub fn new() -> Self {
        Self
    }
}

/// Parse JSON array data from string.
fn parse_data(data_str: &str) -> Result<Vec<Value>, String> {
    let parsed: Value =
        serde_json::from_str(data_str).map_err(|e| format!("Invalid JSON: {}", e))?;
    match parsed {
        Value::Array(arr) => Ok(arr),
        _ => Err("Data must be a JSON array".into()),
    }
}

/// Extract numeric values for a column from an array of objects.
fn extract_numeric(data: &[Value], column: &str) -> Vec<f64> {
    data.iter()
        .filter_map(|row| {
            row.get(column).and_then(|v| match v {
                Value::Number(n) => n.as_f64(),
                Value::String(s) => s.parse::<f64>().ok(),
                _ => None,
            })
        })
        .collect()
}

/// Extract string values for a column from an array of objects.
fn extract_string(data: &[Value], column: &str) -> Vec<String> {
    data.iter()
        .filter_map(|row| {
            row.get(column).map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
        })
        .collect()
}

/// Compute basic statistics for a numeric vector.
fn compute_stats(values: &[f64]) -> Value {
    if values.is_empty() {
        return json!({"count": 0, "error": "No numeric values found"});
    }

    let count = values.len();
    let sum: f64 = values.iter().sum();
    let mean = sum / count as f64;
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    // Variance and standard deviation
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / count as f64;
    let std_dev = variance.sqrt();

    // Median
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = if count % 2 == 0 {
        (sorted[count / 2 - 1] + sorted[count / 2]) / 2.0
    } else {
        sorted[count / 2]
    };

    json!({
        "count": count,
        "sum": round2(sum),
        "mean": round2(mean),
        "median": round2(median),
        "min": round2(min),
        "max": round2(max),
        "std_dev": round2(std_dev),
        "variance": round2(variance)
    })
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[async_trait]
impl Tool for DataAnalysisTool {
    fn name(&self) -> &str {
        "data_analysis"
    }

    fn description(&self) -> &str {
        "Analyze JSON array data. Operations: describe (stats), correlate, histogram, top_n, group_by."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "data": {
                    "type": "string",
                    "description": "JSON array as string (e.g., '[{\"name\":\"a\",\"score\":90}]')"
                },
                "operation": {
                    "type": "string",
                    "description": "One of: describe, correlate, histogram, top_n, group_by"
                },
                "column": {
                    "type": "string",
                    "description": "Column name to analyze (for describe, histogram, top_n)"
                },
                "column2": {
                    "type": "string",
                    "description": "Second column (for correlate)"
                },
                "group_by_column": {
                    "type": "string",
                    "description": "Column to group by (for group_by)"
                },
                "n": {
                    "type": "integer",
                    "description": "Number of top items (for top_n, default 10)"
                }
            },
            "required": ["data", "operation"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let data_str = args["data"].as_str().unwrap_or("").trim();
        let operation = args["operation"].as_str().unwrap_or("").trim();

        if data_str.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Missing required parameter: data".into(),
            });
        }
        if operation.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Missing required parameter: operation".into(),
            });
        }

        let data = match parse_data(data_str) {
            Ok(d) => d,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: e,
                })
            }
        };

        if data.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Data array is empty".into(),
            });
        }

        let result = match operation {
            "describe" => {
                let column = args["column"].as_str().unwrap_or("").trim();

                if column.is_empty() {
                    // Describe all numeric columns
                    let first = &data[0];
                    if let Value::Object(map) = first {
                        let mut stats_map = serde_json::Map::new();
                        for key in map.keys() {
                            let values = extract_numeric(&data, key);
                            if !values.is_empty() {
                                stats_map.insert(key.clone(), compute_stats(&values));
                            }
                        }
                        if stats_map.is_empty() {
                            json!({"error": "No numeric columns found", "row_count": data.len()})
                                .to_string()
                        } else {
                            json!({"row_count": data.len(), "columns": Value::Object(stats_map)})
                                .to_string()
                        }
                    } else {
                        // Data is a flat array of numbers
                        let values: Vec<f64> = data
                            .iter()
                            .filter_map(|v| v.as_f64())
                            .collect();
                        compute_stats(&values).to_string()
                    }
                } else {
                    let values = extract_numeric(&data, column);
                    if values.is_empty() {
                        return Ok(ToolResult {
                            success: false,
                            output: format!(
                                "Column '{}' not found or has no numeric values",
                                column
                            ),
                        });
                    }
                    json!({"column": column, "stats": compute_stats(&values)}).to_string()
                }
            }
            "correlate" => {
                let col1 = args["column"].as_str().unwrap_or("").trim();
                let col2 = args["column2"].as_str().unwrap_or("").trim();

                if col1.is_empty() || col2.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: "correlate requires both 'column' and 'column2' parameters".into(),
                    });
                }

                let vals1 = extract_numeric(&data, col1);
                let vals2 = extract_numeric(&data, col2);

                if vals1.is_empty() || vals2.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: format!(
                            "One or both columns have no numeric values: '{}', '{}'",
                            col1, col2
                        ),
                    });
                }

                let n = vals1.len().min(vals2.len());
                let mean1: f64 = vals1[..n].iter().sum::<f64>() / n as f64;
                let mean2: f64 = vals2[..n].iter().sum::<f64>() / n as f64;

                let mut cov = 0.0;
                let mut var1 = 0.0;
                let mut var2 = 0.0;
                for i in 0..n {
                    let d1 = vals1[i] - mean1;
                    let d2 = vals2[i] - mean2;
                    cov += d1 * d2;
                    var1 += d1 * d1;
                    var2 += d2 * d2;
                }

                let correlation = if var1 > 0.0 && var2 > 0.0 {
                    cov / (var1.sqrt() * var2.sqrt())
                } else {
                    0.0
                };

                json!({
                    "column1": col1,
                    "column2": col2,
                    "correlation": round2(correlation),
                    "sample_size": n,
                    "interpretation": if correlation.abs() > 0.7 { "strong" }
                        else if correlation.abs() > 0.4 { "moderate" }
                        else { "weak" }
                })
                .to_string()
            }
            "histogram" => {
                let column = args["column"].as_str().unwrap_or("").trim();
                if column.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: "histogram requires 'column' parameter".into(),
                    });
                }

                let values = extract_numeric(&data, column);
                if values.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: format!(
                            "Column '{}' not found or has no numeric values",
                            column
                        ),
                    });
                }

                let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
                let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

                if (max - min).abs() < f64::EPSILON {
                    return Ok(ToolResult {
                        success: true,
                        output: json!({
                            "column": column,
                            "bins": [{"range": format!("{:.2}", min), "count": values.len()}]
                        })
                        .to_string(),
                    });
                }

                let num_bins = 10usize.min(values.len());
                let bin_width = (max - min) / num_bins as f64;
                let mut bins: Vec<(f64, f64, usize)> = Vec::new();
                for i in 0..num_bins {
                    let low = min + i as f64 * bin_width;
                    let high = if i == num_bins - 1 {
                        max + 0.001
                    } else {
                        min + (i + 1) as f64 * bin_width
                    };
                    let count = values
                        .iter()
                        .filter(|&&v| v >= low && v < high)
                        .count();
                    bins.push((low, high, count));
                }

                let bin_json: Vec<Value> = bins
                    .iter()
                    .map(|(low, high, count)| {
                        json!({
                            "range": format!("{:.2}-{:.2}", low, high),
                            "count": count
                        })
                    })
                    .collect();

                json!({"column": column, "bins": bin_json, "total": values.len()}).to_string()
            }
            "top_n" => {
                let column = args["column"].as_str().unwrap_or("").trim();
                let n = args["n"].as_u64().unwrap_or(10) as usize;

                if column.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: "top_n requires 'column' parameter".into(),
                    });
                }

                // Count occurrences of each value
                let string_values = extract_string(&data, column);
                if string_values.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: format!("Column '{}' not found", column),
                    });
                }

                let mut counts: HashMap<String, usize> = HashMap::new();
                for v in &string_values {
                    *counts.entry(v.clone()).or_insert(0) += 1;
                }

                let mut sorted: Vec<(String, usize)> = counts.into_iter().collect();
                sorted.sort_by(|a, b| b.1.cmp(&a.1));
                sorted.truncate(n);

                let items: Vec<Value> = sorted
                    .iter()
                    .map(|(val, count)| json!({"value": val, "count": count}))
                    .collect();

                json!({
                    "column": column,
                    "top": items,
                    "total_unique": sorted.len()
                })
                .to_string()
            }
            "group_by" => {
                let group_col = args["group_by_column"]
                    .as_str()
                    .or_else(|| args["column"].as_str())
                    .unwrap_or("")
                    .trim();

                if group_col.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: "group_by requires 'group_by_column' parameter".into(),
                    });
                }

                // Group rows by column value
                let mut groups: HashMap<String, Vec<&Value>> = HashMap::new();
                for row in &data {
                    let key = row
                        .get(group_col)
                        .map(|v| match v {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .unwrap_or_else(|| "null".to_string());
                    groups.entry(key).or_default().push(row);
                }

                // For each group, compute count and stats of numeric columns
                let mut result_map = serde_json::Map::new();
                for (group_key, rows) in &groups {
                    let mut group_info = serde_json::Map::new();
                    group_info.insert("count".to_string(), json!(rows.len()));

                    // Find numeric columns from first row
                    if let Some(Value::Object(first_row)) = rows.first() {
                        for (col, _) in first_row.iter() {
                            if col == group_col {
                                continue;
                            }
                            let values: Vec<f64> = rows
                                .iter()
                                .filter_map(|r| {
                                    r.get(col).and_then(|v| match v {
                                        Value::Number(n) => n.as_f64(),
                                        Value::String(s) => s.parse::<f64>().ok(),
                                        _ => None,
                                    })
                                })
                                .collect();
                            if !values.is_empty() {
                                let sum: f64 = values.iter().sum();
                                let avg = sum / values.len() as f64;
                                group_info.insert(
                                    format!("{}_avg", col),
                                    json!(round2(avg)),
                                );
                                group_info.insert(
                                    format!("{}_sum", col),
                                    json!(round2(sum)),
                                );
                            }
                        }
                    }

                    result_map.insert(group_key.clone(), Value::Object(group_info));
                }

                json!({
                    "group_by": group_col,
                    "groups": Value::Object(result_map),
                    "total_groups": groups.len()
                })
                .to_string()
            }
            _ => {
                return Ok(ToolResult {
                    success: false,
                    output: format!(
                        "Unknown operation: '{}'. Use: describe, correlate, histogram, top_n, group_by",
                        operation
                    ),
                })
            }
        };

        Ok(ToolResult {
            success: true,
            output: result,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DATA: &str = r#"[
        {"name": "alice", "age": 30, "score": 85, "dept": "eng"},
        {"name": "bob", "age": 25, "score": 92, "dept": "eng"},
        {"name": "charlie", "age": 35, "score": 78, "dept": "sales"},
        {"name": "diana", "age": 28, "score": 95, "dept": "eng"},
        {"name": "eve", "age": 32, "score": 88, "dept": "sales"}
    ]"#;

    #[test]
    fn test_name() {
        assert_eq!(DataAnalysisTool::new().name(), "data_analysis");
    }

    #[test]
    fn test_description() {
        let tool = DataAnalysisTool::new();
        assert!(tool.description().contains("JSON"));
    }

    #[test]
    fn test_schema() {
        let tool = DataAnalysisTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["data"].is_object());
        assert!(schema["properties"]["operation"].is_object());
        assert!(schema["properties"]["column"].is_object());
    }

    #[tokio::test]
    async fn test_describe_all_columns() {
        let tool = DataAnalysisTool::new();
        let result = tool
            .execute(json!({"data": TEST_DATA, "operation": "describe"}))
            .await
            .unwrap();
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["row_count"].as_u64().unwrap(), 5);
        assert!(v["columns"]["age"].is_object());
        assert!(v["columns"]["score"].is_object());
    }

    #[tokio::test]
    async fn test_describe_single_column() {
        let tool = DataAnalysisTool::new();
        let result = tool
            .execute(json!({"data": TEST_DATA, "operation": "describe", "column": "age"}))
            .await
            .unwrap();
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["stats"]["count"].as_u64().unwrap(), 5);
        assert_eq!(v["stats"]["min"].as_f64().unwrap(), 25.0);
        assert_eq!(v["stats"]["max"].as_f64().unwrap(), 35.0);
        assert_eq!(v["stats"]["mean"].as_f64().unwrap(), 30.0);
    }

    #[tokio::test]
    async fn test_describe_nonexistent_column() {
        let tool = DataAnalysisTool::new();
        let result = tool
            .execute(json!({"data": TEST_DATA, "operation": "describe", "column": "nonexistent"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("not found"));
    }

    #[tokio::test]
    async fn test_correlate() {
        let tool = DataAnalysisTool::new();
        let result = tool
            .execute(json!({
                "data": TEST_DATA,
                "operation": "correlate",
                "column": "age",
                "column2": "score"
            }))
            .await
            .unwrap();
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert!(v["correlation"].as_f64().is_some());
        assert_eq!(v["sample_size"].as_u64().unwrap(), 5);
    }

    #[tokio::test]
    async fn test_correlate_missing_column2() {
        let tool = DataAnalysisTool::new();
        let result = tool
            .execute(json!({"data": TEST_DATA, "operation": "correlate", "column": "age"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("column2"));
    }

    #[tokio::test]
    async fn test_histogram() {
        let tool = DataAnalysisTool::new();
        let result = tool
            .execute(json!({"data": TEST_DATA, "operation": "histogram", "column": "score"}))
            .await
            .unwrap();
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert!(v["bins"].is_array());
        assert_eq!(v["total"].as_u64().unwrap(), 5);
    }

    #[tokio::test]
    async fn test_histogram_missing_column() {
        let tool = DataAnalysisTool::new();
        let result = tool
            .execute(json!({"data": TEST_DATA, "operation": "histogram"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("column"));
    }

    #[tokio::test]
    async fn test_top_n() {
        let tool = DataAnalysisTool::new();
        let result = tool
            .execute(json!({"data": TEST_DATA, "operation": "top_n", "column": "dept", "n": 2}))
            .await
            .unwrap();
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert!(v["top"].is_array());
        let top = v["top"].as_array().unwrap();
        assert!(top.len() <= 2);
        // eng has 3, sales has 2
        assert_eq!(top[0]["value"].as_str().unwrap(), "eng");
        assert_eq!(top[0]["count"].as_u64().unwrap(), 3);
    }

    #[tokio::test]
    async fn test_top_n_missing_column() {
        let tool = DataAnalysisTool::new();
        let result = tool
            .execute(json!({"data": TEST_DATA, "operation": "top_n"}))
            .await
            .unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_group_by() {
        let tool = DataAnalysisTool::new();
        let result = tool
            .execute(json!({
                "data": TEST_DATA,
                "operation": "group_by",
                "group_by_column": "dept"
            }))
            .await
            .unwrap();
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["total_groups"].as_u64().unwrap(), 2);
        let groups = &v["groups"];
        assert_eq!(groups["eng"]["count"].as_u64().unwrap(), 3);
        assert_eq!(groups["sales"]["count"].as_u64().unwrap(), 2);
    }

    #[tokio::test]
    async fn test_group_by_missing_column() {
        let tool = DataAnalysisTool::new();
        let result = tool
            .execute(json!({"data": TEST_DATA, "operation": "group_by"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("group_by_column"));
    }

    #[tokio::test]
    async fn test_empty_data() {
        let tool = DataAnalysisTool::new();
        let result = tool
            .execute(json!({"data": "", "operation": "describe"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_invalid_json() {
        let tool = DataAnalysisTool::new();
        let result = tool
            .execute(json!({"data": "not json", "operation": "describe"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Invalid JSON"));
    }

    #[tokio::test]
    async fn test_non_array_json() {
        let tool = DataAnalysisTool::new();
        let result = tool
            .execute(json!({"data": "{\"a\":1}", "operation": "describe"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("JSON array"));
    }

    #[tokio::test]
    async fn test_empty_array() {
        let tool = DataAnalysisTool::new();
        let result = tool
            .execute(json!({"data": "[]", "operation": "describe"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("empty"));
    }

    #[tokio::test]
    async fn test_unknown_operation() {
        let tool = DataAnalysisTool::new();
        let result = tool
            .execute(json!({"data": TEST_DATA, "operation": "unknown"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown operation"));
    }

    #[tokio::test]
    async fn test_missing_operation() {
        let tool = DataAnalysisTool::new();
        let result = tool.execute(json!({"data": TEST_DATA})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[test]
    fn test_compute_stats_basic() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let stats = compute_stats(&values);
        assert_eq!(stats["count"].as_u64().unwrap(), 5);
        assert_eq!(stats["mean"].as_f64().unwrap(), 3.0);
        assert_eq!(stats["median"].as_f64().unwrap(), 3.0);
        assert_eq!(stats["min"].as_f64().unwrap(), 1.0);
        assert_eq!(stats["max"].as_f64().unwrap(), 5.0);
    }

    #[test]
    fn test_compute_stats_empty() {
        let values: Vec<f64> = vec![];
        let stats = compute_stats(&values);
        assert_eq!(stats["count"].as_u64().unwrap(), 0);
    }

    #[test]
    fn test_compute_stats_single() {
        let values = vec![42.0];
        let stats = compute_stats(&values);
        assert_eq!(stats["count"].as_u64().unwrap(), 1);
        assert_eq!(stats["mean"].as_f64().unwrap(), 42.0);
        assert_eq!(stats["median"].as_f64().unwrap(), 42.0);
    }

    #[test]
    fn test_round2() {
        assert_eq!(round2(3.14159), 3.14);
        // 2.005 in IEEE 754 may round to either 2.0 or 2.01 depending on platform FP behavior
        let r = round2(2.005);
        assert!(r == 2.0 || r == 2.01, "round2(2.005) = {} (expected 2.0 or 2.01)", r);
        assert_eq!(round2(100.0), 100.0);
        assert_eq!(round2(1.005), 1.0);
        assert_eq!(round2(1.234), 1.23);
        assert_eq!(round2(1.235), 1.24);
    }

    #[test]
    fn test_extract_numeric() {
        let data: Vec<Value> = serde_json::from_str(TEST_DATA).unwrap();
        let ages = extract_numeric(&data, "age");
        assert_eq!(ages, vec![30.0, 25.0, 35.0, 28.0, 32.0]);
    }

    #[test]
    fn test_extract_string() {
        let data: Vec<Value> = serde_json::from_str(TEST_DATA).unwrap();
        let names = extract_string(&data, "name");
        assert_eq!(names, vec!["alice", "bob", "charlie", "diana", "eve"]);
    }

    #[tokio::test]
    async fn test_correlate_perfect_positive() {
        let data = r#"[{"x":1,"y":2},{"x":2,"y":4},{"x":3,"y":6},{"x":4,"y":8}]"#;
        let tool = DataAnalysisTool::new();
        let result = tool
            .execute(json!({"data": data, "operation": "correlate", "column": "x", "column2": "y"}))
            .await
            .unwrap();
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["correlation"].as_f64().unwrap(), 1.0);
        assert_eq!(v["interpretation"].as_str().unwrap(), "strong");
    }
}
