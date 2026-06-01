//! Hermes tool catalog — top-10 high-utility tools ported as Rust
//! idiomatic impls. Every tool is gated behind the parent module's
//! `experimental-hermes-tools` feature flag.
//!
//! Concepts ported from the NousResearch/hermes-agent README (MIT);
//! no verbatim code is copied. Each tool lives in its own file and
//! ships with at least one passing unit test.

use async_trait::async_trait;
use serde_json::Value;

/// Successful tool output, returned as a JSON value so callers can
/// embed it in tool-call response envelopes without re-serialisation.
pub type ToolResult = Result<Value, ToolError>;

/// Small structured error type. Tools should prefer these variants
/// over panicking; the catalog never unwraps user input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    /// Required arg missing or wrong type.
    BadArgs(String),
    /// Tool ran but the input was unprocessable (e.g. bad regex).
    Invalid(String),
    /// External resource not available (e.g. network, optional binary).
    Unavailable(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::BadArgs(s) => write!(f, "bad args: {}", s),
            ToolError::Invalid(s) => write!(f, "invalid input: {}", s),
            ToolError::Unavailable(s) => write!(f, "unavailable: {}", s),
        }
    }
}

impl std::error::Error for ToolError {}

/// The Hermes tool surface. Mirrors the existing `crate::tools::Tool`
/// trait but returns a structured `ToolResult` instead of a free-form
/// `String` so JSON downstream is preserved.
#[async_trait]
pub trait HermesTool: Send + Sync {
    /// LLM-facing name (must be unique within the catalog).
    fn name(&self) -> &'static str;

    /// OpenAI-style `{"type":"function","function":{...}}` envelope.
    fn schema(&self) -> Value;

    /// Invoke. Args are the raw JSON object the LLM produced.
    async fn call(&self, args: &Value) -> ToolResult;
}

// Submodules — one per tool.
pub mod base64_codec;
pub mod calculator;
pub mod color_hex_rgb;
pub mod csv_to_json;
pub mod datetime;
pub mod diff;
pub mod grep;
pub mod hash;
pub mod html_to_text;
pub mod jaro_winkler;
pub mod jq;
pub mod json_query;
pub mod json_to_csv;
pub mod random_string;
pub mod regex_extract;
pub mod sort_lines;
pub mod string_metrics;
pub mod template_render;
pub mod text_stats;
pub mod text_summarize;
pub mod unit_convert;
pub mod url_decode;
pub mod url_encode;
pub mod url_parse;
pub mod uuid_gen;
pub mod uuid_v7;
pub mod word_count_lines;
pub mod word_freq;
pub mod xml_to_json;
pub mod yaml_to_json;

/// Build the full catalog. Order is stable so tests can index by
/// position; new tools should be appended.
pub fn catalog() -> Vec<Box<dyn HermesTool>> {
    vec![
        // Indices 0..19 are the original T3+H5 tools — DO NOT reorder; existing
        // memory rows / integration tests assume this layout.
        Box::new(calculator::Calculator),
        Box::new(datetime::DateTimeTool),
        Box::new(regex_extract::RegexExtract),
        Box::new(json_query::JsonQuery),
        Box::new(text_stats::TextStats),
        Box::new(text_summarize::TextSummarize),
        Box::new(unit_convert::UnitConvert),
        Box::new(base64_codec::Base64Codec),
        Box::new(url_parse::UrlParse),
        Box::new(uuid_gen::UuidGen),
        Box::new(grep::Grep),
        Box::new(diff::Diff),
        Box::new(word_freq::WordFreq),
        Box::new(csv_to_json::CsvToJson),
        Box::new(json_to_csv::JsonToCsv),
        Box::new(html_to_text::HtmlToText),
        Box::new(template_render::TemplateRender),
        Box::new(hash::Hash),
        Box::new(sort_lines::SortLines),
        Box::new(string_metrics::StringMetrics),
        // Indices 20..29 — T50 expansion (v0.6.0 V1 spec, 20 → 30).
        Box::new(jq::Jq),
        Box::new(xml_to_json::XmlToJson),
        Box::new(yaml_to_json::YamlToJson),
        Box::new(url_encode::UrlEncode),
        Box::new(url_decode::UrlDecode),
        Box::new(color_hex_rgb::ColorHexRgb),
        Box::new(uuid_v7::UuidV7),
        Box::new(jaro_winkler::JaroWinkler),
        Box::new(word_count_lines::WordCountLines),
        Box::new(random_string::RandomString),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_thirty_tools() {
        let cat = catalog();
        assert_eq!(
            cat.len(),
            30,
            "Hermes catalog must have exactly 30 tools (T50 expansion)"
        );
    }

    #[test]
    fn every_tool_has_unique_name() {
        let cat = catalog();
        let mut names: Vec<&str> = cat.iter().map(|t| t.name()).collect();
        let original = names.len();
        names.sort();
        names.dedup();
        assert_eq!(original, names.len(), "tool names must be unique");
    }

    #[test]
    fn every_tool_schema_is_well_formed() {
        let cat = catalog();
        for tool in cat.iter() {
            let schema = tool.schema();
            assert_eq!(
                schema["type"],
                "function",
                "tool {} bad schema type",
                tool.name()
            );
            assert_eq!(
                schema["function"]["name"],
                tool.name(),
                "tool {} schema/name mismatch",
                tool.name()
            );
        }
    }

    #[test]
    fn tool_error_display_is_useful() {
        assert_eq!(format!("{}", ToolError::BadArgs("x".into())), "bad args: x");
        assert_eq!(
            format!("{}", ToolError::Invalid("y".into())),
            "invalid input: y"
        );
        assert_eq!(
            format!("{}", ToolError::Unavailable("z".into())),
            "unavailable: z"
        );
    }
}
