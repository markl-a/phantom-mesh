//! Tool integration tests — 50+ tests covering data, communication,
//! content, file, and web tools without requiring external services.

use clawtex_core::tools::csv_parse::CsvParseTool;
use clawtex_core::tools::json_transform::JsonTransformTool;
use clawtex_core::tools::summarize::SummarizeTool;
use clawtex_core::tools::slack::{SlackTool, SlackConfig};
use clawtex_core::tools::discord::{DiscordTool, DiscordConfig};
use clawtex_core::tools::line_notify::{LineTool, LineConfig};
use clawtex_core::tools::whatsapp::{WhatsAppTool, WhatsAppConfig};
use clawtex_core::tools::image_generate::{ImageGenerateTool, ImageGenerateConfig};
use clawtex_core::tools::file_read::FileReadTool;
use clawtex_core::tools::file_write::FileWriteTool;
use clawtex_core::tools::file_edit::FileEditTool;
use clawtex_core::tools::http_request::HttpRequestTool;
use clawtex_core::tools::web_search::{WebSearchTool, SearchConfig};
use clawtex_core::tools::{Tool, SecurityConfig};
use serde_json::json;
use std::fs;
use std::collections::HashMap;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_workspace(suffix: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("clawtex_tt_{}", suffix));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn make_security(dir: &std::path::Path) -> SecurityConfig {
    SecurityConfig {
        workspace_dir: dir.to_string_lossy().to_string(),
        workspace_only: true,
        allowed_commands: vec![],
        ..Default::default()
    }
}

const TEST_CSV: &str = "name,age,score,city\nalice,30,85,taipei\nbob,25,92,tokyo\ncharlie,35,78,seoul\ndiana,28,95,bangkok\neve,42,67,jakarta";
const TEST_CSV_MINIMAL: &str = "a,b\n1,2\n3,4";
const TEST_JSON_OBJ: &str = r#"{"name":"alice","age":30,"scores":[85,92,78],"address":{"city":"taipei","zip":"100"}}"#;
const TEST_JSON_ARR: &str = r#"[{"id":1,"status":"active"},{"id":2,"status":"inactive"},{"id":3,"status":"active"}]"#;

const SAMPLE_TEXT: &str = "Artificial intelligence is reshaping every industry globally. \
Machine learning models are now deployed in production at massive scale. \
Deep learning has fundamentally changed how we approach computer vision. \
Natural language processing enables unprecedented human-computer interaction. \
The economic impact of AI adoption will exceed trillions of dollars. \
Research labs compete aggressively to publish the latest breakthroughs. \
Open source communities accelerate the pace of AI innovation worldwide.";

// ── CSV Parse Tool Tests ──────────────────────────────────────────────────────

#[test]
fn csv_parse_name_and_description() {
    let t = CsvParseTool::new();
    assert_eq!(t.name(), "csv_parse");
    assert!(t.description().len() > 5);
}

#[test]
fn csv_parse_schema_has_required_fields() {
    let schema = CsvParseTool::new().parameters_schema();
    assert!(schema["properties"]["data"].is_object());
    assert!(schema["properties"]["operation"].is_object());
    let req = schema["required"].as_array().unwrap();
    assert!(req.iter().any(|v| v == "data"));
    assert!(req.iter().any(|v| v == "operation"));
}

#[tokio::test]
async fn csv_parse_headers_returns_all_columns() {
    let t = CsvParseTool::new();
    let r = t.execute(json!({"data": TEST_CSV, "operation": "headers"})).await.unwrap();
    assert!(r.success, "headers op failed: {}", r.output);
    assert!(r.output.contains("name"));
    assert!(r.output.contains("age"));
    assert!(r.output.contains("score"));
    assert!(r.output.contains("city"));
}

#[tokio::test]
async fn csv_parse_count_returns_correct_row_count() {
    let t = CsvParseTool::new();
    let r = t.execute(json!({"data": TEST_CSV, "operation": "count"})).await.unwrap();
    assert!(r.success);
    assert_eq!(r.output.trim(), "5");
}

#[tokio::test]
async fn csv_parse_count_minimal_csv() {
    let t = CsvParseTool::new();
    let r = t.execute(json!({"data": TEST_CSV_MINIMAL, "operation": "count"})).await.unwrap();
    assert!(r.success);
    assert_eq!(r.output.trim(), "2");
}

#[tokio::test]
async fn csv_parse_select_specific_columns() {
    let t = CsvParseTool::new();
    let r = t.execute(json!({
        "data": TEST_CSV,
        "operation": "select",
        "columns": ["name", "score"]
    })).await.unwrap();
    assert!(r.success);
    assert!(r.output.contains("name"));
    assert!(r.output.contains("score"));
    assert!(r.output.contains("alice"));
    let parsed: serde_json::Value = serde_json::from_str(&r.output).unwrap();
    let cols = parsed["columns"].as_array().unwrap();
    assert_eq!(cols.len(), 2);
}

#[tokio::test]
async fn csv_parse_select_with_limit() {
    let t = CsvParseTool::new();
    let r = t.execute(json!({
        "data": TEST_CSV,
        "operation": "select",
        "columns": ["name"],
        "limit": 2
    })).await.unwrap();
    assert!(r.success);
    let parsed: serde_json::Value = serde_json::from_str(&r.output).unwrap();
    let rows = parsed["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
}

#[tokio::test]
async fn csv_parse_filter_finds_matching_rows() {
    let t = CsvParseTool::new();
    let r = t.execute(json!({
        "data": TEST_CSV,
        "operation": "filter",
        "filter_column": "name",
        "filter_value": "bob"
    })).await.unwrap();
    assert!(r.success);
    assert!(r.output.contains("bob"));
    assert!(!r.output.contains("alice"));
}

#[tokio::test]
async fn csv_parse_filter_missing_filter_column_fails() {
    let t = CsvParseTool::new();
    let r = t.execute(json!({
        "data": TEST_CSV,
        "operation": "filter",
        "filter_value": "bob"
    })).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("filter_column"));
}

#[tokio::test]
async fn csv_parse_filter_nonexistent_column_fails() {
    let t = CsvParseTool::new();
    let r = t.execute(json!({
        "data": TEST_CSV,
        "operation": "filter",
        "filter_column": "nonexistent",
        "filter_value": "x"
    })).await.unwrap();
    assert!(!r.success);
    assert!(r.output.to_lowercase().contains("not found") || r.output.contains("Column"));
}

#[tokio::test]
async fn csv_parse_stats_returns_numeric_stats() {
    let t = CsvParseTool::new();
    let r = t.execute(json!({"data": TEST_CSV, "operation": "stats"})).await.unwrap();
    assert!(r.success);
    let stats: serde_json::Value = serde_json::from_str(&r.output).unwrap();
    assert!(stats["age"]["min"].as_f64().unwrap() == 25.0);
    assert!(stats["age"]["max"].as_f64().unwrap() == 42.0);
    assert!(stats["score"]["max"].as_f64().unwrap() == 95.0);
}

#[tokio::test]
async fn csv_parse_to_json_returns_array_of_objects() {
    let t = CsvParseTool::new();
    let r = t.execute(json!({"data": TEST_CSV, "operation": "to_json"})).await.unwrap();
    assert!(r.success);
    let parsed: Vec<HashMap<String, String>> = serde_json::from_str(&r.output).unwrap();
    assert_eq!(parsed.len(), 5);
    assert_eq!(parsed[0]["name"], "alice");
    assert_eq!(parsed[1]["city"], "tokyo");
}

#[tokio::test]
async fn csv_parse_to_json_respects_limit() {
    let t = CsvParseTool::new();
    let r = t.execute(json!({"data": TEST_CSV, "operation": "to_json", "limit": 3})).await.unwrap();
    assert!(r.success);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&r.output).unwrap();
    assert_eq!(parsed.len(), 3);
}

#[tokio::test]
async fn csv_parse_empty_data_fails_gracefully() {
    let t = CsvParseTool::new();
    let r = t.execute(json!({"data": "", "operation": "count"})).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("Missing"));
}

#[tokio::test]
async fn csv_parse_empty_operation_fails_gracefully() {
    let t = CsvParseTool::new();
    let r = t.execute(json!({"data": TEST_CSV, "operation": ""})).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("Missing"));
}

#[tokio::test]
async fn csv_parse_unknown_operation_fails_with_hint() {
    let t = CsvParseTool::new();
    let r = t.execute(json!({"data": TEST_CSV, "operation": "explode"})).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("Unknown operation"));
}

#[tokio::test]
async fn csv_parse_single_row_csv() {
    let data = "x,y\n1,2";
    let t = CsvParseTool::new();
    let r = t.execute(json!({"data": data, "operation": "count"})).await.unwrap();
    assert!(r.success);
    assert_eq!(r.output.trim(), "1");
}

// ── JSON Transform Tool Tests ─────────────────────────────────────────────────

#[test]
fn json_transform_name_and_description() {
    let t = JsonTransformTool::new();
    assert_eq!(t.name(), "json_transform");
    assert!(t.description().len() > 5);
}

#[tokio::test]
async fn json_transform_get_nested_path() {
    let t = JsonTransformTool::new();
    let r = t.execute(json!({
        "json_input": TEST_JSON_OBJ,
        "operation": "get",
        "path": "/address/city"
    })).await.unwrap();
    assert!(r.success);
    assert!(r.output.contains("taipei"));
}

#[tokio::test]
async fn json_transform_get_array_element() {
    let t = JsonTransformTool::new();
    let r = t.execute(json!({
        "json_input": TEST_JSON_OBJ,
        "operation": "get",
        "path": "/scores/1"
    })).await.unwrap();
    assert!(r.success);
    assert_eq!(r.output.trim(), "92");
}

#[tokio::test]
async fn json_transform_get_nonexistent_path_fails() {
    let t = JsonTransformTool::new();
    let r = t.execute(json!({
        "json_input": TEST_JSON_OBJ,
        "operation": "get",
        "path": "/does/not/exist"
    })).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("not found"));
}

#[tokio::test]
async fn json_transform_keys_on_object() {
    let t = JsonTransformTool::new();
    let r = t.execute(json!({
        "json_input": TEST_JSON_OBJ,
        "operation": "keys"
    })).await.unwrap();
    assert!(r.success);
    assert!(r.output.contains("name"));
    assert!(r.output.contains("age"));
    assert!(r.output.contains("scores"));
    assert!(r.output.contains("address"));
}

#[tokio::test]
async fn json_transform_keys_on_array_fails() {
    let t = JsonTransformTool::new();
    let r = t.execute(json!({
        "json_input": TEST_JSON_ARR,
        "operation": "keys"
    })).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("not an object"));
}

#[tokio::test]
async fn json_transform_values_on_object() {
    let t = JsonTransformTool::new();
    let r = t.execute(json!({
        "json_input": r#"{"a":1,"b":2}"#,
        "operation": "values"
    })).await.unwrap();
    assert!(r.success);
    assert!(r.output.contains("1"));
    assert!(r.output.contains("2"));
}

#[tokio::test]
async fn json_transform_count_array() {
    let t = JsonTransformTool::new();
    let r = t.execute(json!({
        "json_input": TEST_JSON_ARR,
        "operation": "count"
    })).await.unwrap();
    assert!(r.success);
    assert_eq!(r.output.trim(), "3");
}

#[tokio::test]
async fn json_transform_count_object_keys() {
    let t = JsonTransformTool::new();
    let r = t.execute(json!({
        "json_input": r#"{"a":1,"b":2,"c":3}"#,
        "operation": "count"
    })).await.unwrap();
    assert!(r.success);
    assert_eq!(r.output.trim(), "3");
}

#[tokio::test]
async fn json_transform_count_scalar_fails() {
    let t = JsonTransformTool::new();
    let r = t.execute(json!({
        "json_input": "42",
        "operation": "count"
    })).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("not an array or object"));
}

#[tokio::test]
async fn json_transform_flatten_nested_object() {
    let t = JsonTransformTool::new();
    let r = t.execute(json!({
        "json_input": r#"{"a":{"b":{"c":1}},"d":2}"#,
        "operation": "flatten"
    })).await.unwrap();
    assert!(r.success);
    assert!(r.output.contains("a.b.c"));
    assert!(r.output.contains("1"));
    assert!(r.output.contains("d"));
}

#[tokio::test]
async fn json_transform_pretty_formats_nicely() {
    let t = JsonTransformTool::new();
    let r = t.execute(json!({
        "json_input": r#"{"a":1,"b":2}"#,
        "operation": "pretty"
    })).await.unwrap();
    assert!(r.success);
    assert!(r.output.contains('\n'));
    assert!(r.output.contains("  ")); // indentation
}

#[tokio::test]
async fn json_transform_filter_array_by_key() {
    let t = JsonTransformTool::new();
    let r = t.execute(json!({
        "json_input": TEST_JSON_ARR,
        "operation": "filter",
        "filter_key": "status",
        "filter_value": "active"
    })).await.unwrap();
    assert!(r.success);
    let arr: Vec<serde_json::Value> = serde_json::from_str(&r.output).unwrap();
    assert_eq!(arr.len(), 2);
    for item in &arr {
        assert_eq!(item["status"], "active");
    }
}

#[tokio::test]
async fn json_transform_filter_missing_filter_key_fails() {
    let t = JsonTransformTool::new();
    let r = t.execute(json!({
        "json_input": TEST_JSON_ARR,
        "operation": "filter"
    })).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("filter_key"));
}

#[tokio::test]
async fn json_transform_invalid_json_fails() {
    let t = JsonTransformTool::new();
    let r = t.execute(json!({
        "json_input": "{not valid json}",
        "operation": "get"
    })).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("Invalid JSON"));
}

#[tokio::test]
async fn json_transform_unknown_operation_fails() {
    let t = JsonTransformTool::new();
    let r = t.execute(json!({
        "json_input": TEST_JSON_OBJ,
        "operation": "zap"
    })).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("Unknown operation"));
}

#[tokio::test]
async fn json_transform_empty_json_input_fails() {
    let t = JsonTransformTool::new();
    let r = t.execute(json!({
        "json_input": "",
        "operation": "get"
    })).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("Missing"));
}

// ── Summarize Tool Tests ──────────────────────────────────────────────────────

#[test]
fn summarize_name_and_description() {
    let t = SummarizeTool::new();
    assert_eq!(t.name(), "summarize");
    assert!(t.description().to_lowercase().contains("summarize"));
}

#[tokio::test]
async fn summarize_empty_text_fails() {
    let t = SummarizeTool::new();
    let r = t.execute(json!({"text": ""})).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("Missing"));
}

#[tokio::test]
async fn summarize_short_text_returns_result() {
    let t = SummarizeTool::new();
    let short = "AI is great. It helps people.";
    let r = t.execute(json!({"text": short})).await.unwrap();
    assert!(r.success);
    assert!(!r.output.is_empty());
}

#[tokio::test]
async fn summarize_long_text_is_shorter() {
    let t = SummarizeTool::new();
    let r = t.execute(json!({"text": SAMPLE_TEXT, "max_sentences": 2})).await.unwrap();
    assert!(r.success);
    assert!(r.output.len() < SAMPLE_TEXT.len());
}

#[tokio::test]
async fn summarize_bullets_style() {
    let t = SummarizeTool::new();
    let r = t.execute(json!({"text": SAMPLE_TEXT, "max_sentences": 3, "style": "bullets"})).await.unwrap();
    assert!(r.success);
    assert!(r.output.starts_with("- "));
    assert!(r.output.contains("\n- "));
}

#[tokio::test]
async fn summarize_max_sentences_one() {
    let t = SummarizeTool::new();
    let r = t.execute(json!({"text": SAMPLE_TEXT, "max_sentences": 1})).await.unwrap();
    assert!(r.success);
    assert!(!r.output.is_empty());
}

#[tokio::test]
async fn summarize_default_max_sentences_applied() {
    let t = SummarizeTool::new();
    let r = t.execute(json!({"text": SAMPLE_TEXT})).await.unwrap();
    assert!(r.success);
    assert!(!r.output.is_empty());
}

// ── Communication Tool Tests (Graceful Config-Missing Failures) ───────────────

#[test]
fn slack_name_and_description() {
    let t = SlackTool::new(SlackConfig::default());
    assert_eq!(t.name(), "slack_send");
    assert!(t.description().contains("Slack"));
}

#[tokio::test]
async fn slack_empty_text_fails() {
    let t = SlackTool::new(SlackConfig::default());
    let r = t.execute(json!({"text": ""})).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("Missing"));
}

#[tokio::test]
async fn slack_no_webhook_configured_fails() {
    let t = SlackTool::new(SlackConfig::default());
    let r = t.execute(json!({"text": "hello team"})).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("not configured"));
}

#[test]
fn slack_schema_has_text_required() {
    let schema = SlackTool::new(SlackConfig::default()).parameters_schema();
    let req = schema["required"].as_array().unwrap();
    assert!(req.iter().any(|v| v == "text"));
}

#[test]
fn discord_name_and_description() {
    let t = DiscordTool::new(DiscordConfig::default());
    assert_eq!(t.name(), "discord_send");
    assert!(t.description().contains("Discord"));
}

#[tokio::test]
async fn discord_empty_text_fails() {
    let t = DiscordTool::new(DiscordConfig::default());
    let r = t.execute(json!({"text": ""})).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("Missing"));
}

#[tokio::test]
async fn discord_no_webhook_configured_fails() {
    let t = DiscordTool::new(DiscordConfig::default());
    let r = t.execute(json!({"text": "hello discord"})).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("not configured"));
}

#[test]
fn discord_schema_has_text_required() {
    let schema = DiscordTool::new(DiscordConfig::default()).parameters_schema();
    let req = schema["required"].as_array().unwrap();
    assert!(req.iter().any(|v| v == "text"));
}

#[test]
fn line_notify_name() {
    let t = LineTool::new(LineConfig::default());
    assert_eq!(t.name(), "line_send");
}

#[tokio::test]
async fn line_notify_no_token_fails() {
    let t = LineTool::new(LineConfig::default());
    let r = t.execute(json!({"message": "hello"})).await.unwrap();
    assert!(!r.success);
}

#[test]
fn whatsapp_name() {
    let t = WhatsAppTool::new(WhatsAppConfig::default());
    assert_eq!(t.name(), "whatsapp_send");
}

#[tokio::test]
async fn whatsapp_no_config_fails() {
    let t = WhatsAppTool::new(WhatsAppConfig::default());
    let r = t.execute(json!({"to": "+1234567890", "message": "hi"})).await.unwrap();
    assert!(!r.success);
}

// ── Content Tool Tests (Missing Deps Error Handling) ─────────────────────────

#[test]
fn image_generate_name() {
    let t = ImageGenerateTool::new(ImageGenerateConfig { gemini_api_key: String::new() });
    assert_eq!(t.name(), "image_generate");
}

#[tokio::test]
async fn image_generate_empty_prompt_fails() {
    let t = ImageGenerateTool::new(ImageGenerateConfig { gemini_api_key: "test-key".into() });
    let r = t.execute(json!({"prompt": ""})).await.unwrap();
    assert!(!r.success);
}

#[tokio::test]
async fn image_generate_no_api_key_fails() {
    let t = ImageGenerateTool::new(ImageGenerateConfig { gemini_api_key: String::new() });
    let r = t.execute(json!({"prompt": "a beautiful sunset"})).await.unwrap();
    assert!(!r.success);
}

#[test]
fn image_generate_preflight_empty_prompt() {
    let t = ImageGenerateTool::new(ImageGenerateConfig { gemini_api_key: "key".into() });
    let result = t.preflight(&json!({"prompt": ""}));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Preflight"));
}

#[test]
fn image_generate_preflight_no_api_key() {
    let t = ImageGenerateTool::new(ImageGenerateConfig { gemini_api_key: String::new() });
    let result = t.preflight(&json!({"prompt": "a cat"}));
    assert!(result.is_err());
}

// ── File Tool Tests ───────────────────────────────────────────────────────────

#[tokio::test]
async fn file_read_existing_file() {
    let dir = make_workspace("fr_1");
    let security = make_security(&dir);
    let t = FileReadTool::new(security);
    fs::write(dir.join("hello.txt"), "hello clawtex").unwrap();
    let r = t.execute(json!({"path": "hello.txt"})).await.unwrap();
    assert!(r.success, "read failed: {}", r.output);
    assert!(r.output.contains("hello clawtex"));
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn file_read_nonexistent_file_fails() {
    let dir = make_workspace("fr_2");
    let security = make_security(&dir);
    let t = FileReadTool::new(security);
    let r = t.execute(json!({"path": "missing.txt"})).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("not found") || r.output.contains("Error"));
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn file_read_empty_path_fails() {
    let dir = make_workspace("fr_3");
    let security = make_security(&dir);
    let t = FileReadTool::new(security);
    let r = t.execute(json!({"path": ""})).await.unwrap();
    assert!(!r.success);
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn file_read_large_file_truncated() {
    let dir = make_workspace("fr_4");
    let security = make_security(&dir);
    let t = FileReadTool::new(security);
    // Write 20KB of ASCII content (exceeds 8000 byte truncation threshold)
    let content = "x".repeat(20_000);
    fs::write(dir.join("large.txt"), &content).unwrap();
    let r = t.execute(json!({"path": "large.txt"})).await.unwrap();
    assert!(r.success);
    assert!(r.output.contains("truncated"));
    assert!(r.output.len() < content.len());
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn file_read_preflight_missing_path_fails() {
    let dir = make_workspace("fr_5");
    let security = make_security(&dir);
    let t = FileReadTool::new(security);
    let result = t.preflight(&json!({}));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Preflight"));
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn file_write_creates_new_file() {
    let dir = make_workspace("fw_1");
    let security = make_security(&dir);
    let t = FileWriteTool::new(security);
    let r = t.execute(json!({"path": "output.txt", "content": "written by test"})).await.unwrap();
    assert!(r.success, "write failed: {}", r.output);
    let content = fs::read_to_string(dir.join("output.txt")).unwrap();
    assert_eq!(content, "written by test");
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn file_write_creates_parent_directories() {
    let dir = make_workspace("fw_2");
    let security = make_security(&dir);
    let t = FileWriteTool::new(security);
    let r = t.execute(json!({
        "path": "nested/sub/deep.txt",
        "content": "deep file"
    })).await.unwrap();
    assert!(r.success, "nested write failed: {}", r.output);
    assert!(dir.join("nested/sub/deep.txt").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn file_write_rejects_path_traversal() {
    let dir = make_workspace("fw_3");
    let security = make_security(&dir);
    let t = FileWriteTool::new(security);
    let r = t.execute(json!({"path": "../../escape.txt", "content": "nope"})).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("traversal"));
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn file_write_empty_path_fails() {
    let dir = make_workspace("fw_4");
    let security = make_security(&dir);
    let t = FileWriteTool::new(security);
    let r = t.execute(json!({"path": "", "content": "data"})).await.unwrap();
    assert!(!r.success);
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn file_write_overwrites_existing_file() {
    let dir = make_workspace("fw_5");
    let security = make_security(&dir);
    let t = FileWriteTool::new(security);
    fs::write(dir.join("update.txt"), "original").unwrap();
    let r = t.execute(json!({"path": "update.txt", "content": "updated"})).await.unwrap();
    assert!(r.success);
    let content = fs::read_to_string(dir.join("update.txt")).unwrap();
    assert_eq!(content, "updated");
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn file_write_reports_byte_count() {
    let dir = make_workspace("fw_6");
    let security = make_security(&dir);
    let t = FileWriteTool::new(security);
    let r = t.execute(json!({"path": "sized.txt", "content": "12345"})).await.unwrap();
    assert!(r.success);
    assert!(r.output.contains("5 bytes"));
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn file_edit_replaces_text_in_file() {
    let dir = make_workspace("fe_1");
    let sec = SecurityConfig {
        workspace_dir: dir.to_string_lossy().to_string(),
        workspace_only: false,
        ..Default::default()
    };
    let t = FileEditTool::new(sec);
    let path = dir.join("edit.txt");
    fs::write(&path, "hello world").unwrap();
    let r = t.execute(json!({
        "path": path.to_string_lossy(),
        "old_text": "world",
        "new_text": "clawtex"
    })).await.unwrap();
    assert!(r.success, "edit failed: {}", r.output);
    let updated = fs::read_to_string(&path).unwrap();
    assert_eq!(updated, "hello clawtex");
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn file_edit_missing_old_text_fails() {
    let t = FileEditTool::new(SecurityConfig::default());
    let r = t.execute(json!({"path": "test.txt", "new_text": "x"})).await.unwrap();
    assert!(!r.success);
}

#[tokio::test]
async fn file_edit_missing_path_fails() {
    let t = FileEditTool::new(SecurityConfig::default());
    let r = t.execute(json!({"old_text": "a", "new_text": "b"})).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("Missing 'path'"));
}

#[tokio::test]
async fn file_edit_old_text_not_found_fails() {
    let dir = make_workspace("fe_2");
    let sec = SecurityConfig {
        workspace_dir: dir.to_string_lossy().to_string(),
        workspace_only: false,
        ..Default::default()
    };
    let t = FileEditTool::new(sec);
    let path = dir.join("edit2.txt");
    fs::write(&path, "content here").unwrap();
    let r = t.execute(json!({
        "path": path.to_string_lossy(),
        "old_text": "MISSING_TEXT_XYZ",
        "new_text": "replacement"
    })).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("not found"));
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn file_edit_rejects_path_traversal() {
    let t = FileEditTool::new(SecurityConfig::default());
    let r = t.execute(json!({
        "path": "../../etc/passwd",
        "old_text": "a",
        "new_text": "b"
    })).await.unwrap();
    assert!(!r.success);
}

// ── HTTP Request Tool Tests ───────────────────────────────────────────────────

#[test]
fn http_request_name_and_description() {
    let t = HttpRequestTool::new(vec![]);
    assert_eq!(t.name(), "http_request");
    assert!(t.description().to_lowercase().contains("http"));
}

#[tokio::test]
async fn http_request_wildcard_domain_allows_all_execute() {
    // Wildcard allowlist — blocked.example.com should be attempted (not blocked by allowlist).
    // Connection will fail (no server), but the failure must NOT be "allowlist" related.
    let t = HttpRequestTool::new(vec!["*".into()]);
    let r = t.execute(json!({"url": "http://127.0.0.1:19999/no-server", "method": "GET"})).await.unwrap();
    // Should fail with a connection error, NOT an "allowlist" error
    assert!(!r.output.contains("allowlist"), "Wildcard should not block any domain: {}", r.output);
}

#[tokio::test]
async fn http_request_empty_allowlist_allows_all_execute() {
    // Empty allowlist means no restriction — same as wildcard
    let t = HttpRequestTool::new(vec![]);
    let r = t.execute(json!({"url": "http://127.0.0.1:19999/no-server", "method": "GET"})).await.unwrap();
    assert!(!r.output.contains("allowlist"), "Empty allowlist should not block any domain: {}", r.output);
}

#[tokio::test]
async fn http_request_specific_domain_blocks_other_domains() {
    // Only "allowed.com" in allowlist — "blocked.com" should be rejected
    let t = HttpRequestTool::new(vec!["allowed.com".into()]);
    let r = t.execute(json!({"url": "https://blocked.com/api"})).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("allowlist"), "Should report allowlist rejection: {}", r.output);
}

#[tokio::test]
async fn http_request_specific_domain_allows_configured_domain() {
    // "example.com" in allowlist — same domain should not be blocked by allowlist.
    // It will fail with a network error (no server), but NOT an allowlist error.
    let t = HttpRequestTool::new(vec!["example.com".into()]);
    let r = t.execute(json!({"url": "http://example.com/api"})).await.unwrap();
    // Either succeeds (unlikely) or fails with a connection error (not allowlist)
    assert!(!r.output.contains("allowlist"), "Configured domain must not be blocked: {}", r.output);
}

#[tokio::test]
async fn http_request_missing_url_fails() {
    let t = HttpRequestTool::new(vec![]);
    let r = t.execute(json!({})).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("Missing 'url'"));
}

#[tokio::test]
async fn http_request_blocked_domain_fails() {
    let t = HttpRequestTool::new(vec!["allowed.com".into()]);
    let r = t.execute(json!({"url": "https://blocked.com/api"})).await.unwrap();
    assert!(!r.success);
    assert!(r.output.contains("allowlist"));
}

#[test]
fn http_request_schema_has_url_required() {
    let schema = HttpRequestTool::new(vec![]).parameters_schema();
    let req = schema["required"].as_array().unwrap();
    assert!(req.iter().any(|v| v == "url"));
}

// ── Web Search Tool Tests ─────────────────────────────────────────────────────

#[test]
fn web_search_name_and_description() {
    let t = WebSearchTool::new(SearchConfig::default());
    assert_eq!(t.name(), "web_search");
    assert!(t.description().len() > 10);
}

#[test]
fn web_search_schema_has_query_required() {
    let schema = WebSearchTool::new(SearchConfig::default()).parameters_schema();
    let req = schema["required"].as_array().unwrap();
    assert!(req.iter().any(|v| v == "query"));
}

#[tokio::test]
async fn web_search_no_api_keys_does_not_panic() {
    let t = WebSearchTool::new(SearchConfig::default());
    // With no API keys, should either fail gracefully or fall back to RSS — must not panic
    let r = t.execute(json!({"query": "test query", "mode": "search"})).await;
    assert!(r.is_ok(), "execute must not return Err (panic safety)");
}

#[tokio::test]
async fn web_search_empty_query_handled_gracefully() {
    let t = WebSearchTool::new(SearchConfig::default());
    let r = t.execute(json!({"query": "", "mode": "search"})).await.unwrap();
    // Empty query should fail or return an empty result — not panic
    let _ = r;
}

#[tokio::test]
async fn web_search_fetch_mode_with_invalid_url_handled_gracefully() {
    let t = WebSearchTool::new(SearchConfig::default());
    let r = t.execute(json!({"query": "not_a_url", "mode": "fetch"})).await;
    // Should not panic regardless of URL validity
    assert!(r.is_ok(), "fetch mode must not panic on invalid URL");
}
