//! Tool Dispatcher — handles native function calling and XML text-based tool invocation.
//! Inspired by ZeroClaw's NativeToolDispatcher / XmlToolDispatcher pattern.
//!
//! NativeToolDispatcher: Passes tool specs to the LLM and parses structured tool_calls.
//! XmlToolDispatcher: Injects tool instructions into the prompt and parses <tool_call> XML tags.

use crate::providers::{ChatMessage, ChatResponse, ToolCall, ToolCallFunction};
use regex::Regex;
use serde_json::Value;
use tracing::{debug, warn};

/// Dispatch mode for tool calling
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchMode {
    /// Provider supports native function calling (OpenAI, Anthropic, etc.)
    Native,
    /// Use XML-based tool calling in text (for models without native support)
    Xml,
    /// Auto-detect: try native first, fall back to XML parsing
    Auto,
}

impl Default for DispatchMode {
    fn default() -> Self {
        Self::Auto
    }
}

/// Parse a ChatResponse into text content and extracted tool calls.
/// Handles both native tool_calls and XML <tool_call> tags in content.
pub fn parse_tool_calls(response: &ChatResponse, mode: DispatchMode) -> (String, Vec<ToolCall>) {
    let msg = &response.message;

    match mode {
        DispatchMode::Native => {
            // Only use native tool_calls, ignore XML in content
            let calls = msg.tool_calls.clone().unwrap_or_default();
            (msg.content.clone(), calls)
        }
        DispatchMode::Xml => {
            // Only parse XML from content, ignore native tool_calls
            let (text, calls) = parse_xml_tool_calls(&msg.content);
            (text, calls)
        }
        DispatchMode::Auto => {
            // Try native first
            if let Some(ref tool_calls) = msg.tool_calls {
                if !tool_calls.is_empty() {
                    return (msg.content.clone(), tool_calls.clone());
                }
            }
            // Fall back to XML parsing
            let (text, calls) = parse_xml_tool_calls(&msg.content);
            if !calls.is_empty() {
                return (text, calls);
            }
            // No tool calls found
            (msg.content.clone(), vec![])
        }
    }
}

/// Build tool instruction text for XML-mode prompting.
/// This tells the model how to call tools using XML tags.
pub fn xml_tool_instructions(tool_specs: &[crate::tools::ToolSpec]) -> String {
    if tool_specs.is_empty() {
        return String::new();
    }

    let mut instructions = String::from(
        "\n\n## Available Tools\n\
         You can call tools by writing a <tool_call> XML block with JSON inside.\n\
         Format:\n\
         ```\n\
         <tool_call>\n\
         {\"name\": \"tool_name\", \"arguments\": {\"arg1\": \"value1\"}}\n\
         </tool_call>\n\
         ```\n\n\
         Available tools:\n"
    );

    for spec in tool_specs {
        instructions.push_str(&format!(
            "- **{}**: {}\n  Parameters: {}\n",
            spec.name,
            spec.description,
            serde_json::to_string(&spec.parameters).unwrap_or_default()
        ));
    }

    instructions.push_str(
        "\nYou may call multiple tools in one response. \
         Write your reasoning before/between tool calls."
    );

    instructions
}

/// Parse XML tool call tags from text content.
/// Supports: <tool_call>, <toolcall>, <tool-call>, <invoke>, <function=name>
fn parse_xml_tool_calls(content: &str) -> (String, Vec<ToolCall>) {
    // Normalize variant tags to canonical form
    let normalized = normalize_tool_tags(content);

    let re = Regex::new(r"(?s)<tool_call>\s*(.*?)\s*</tool_call>").unwrap();

    let mut tool_calls = Vec::new();
    let mut text_parts = Vec::new();
    let mut last_end = 0;

    for cap in re.captures_iter(&normalized) {
        let full_match = cap.get(0).unwrap();
        let json_str = cap.get(1).unwrap().as_str().trim();

        // Collect text before this tag
        let text_before = normalized[last_end..full_match.start()].trim();
        if !text_before.is_empty() {
            text_parts.push(text_before.to_string());
        }
        last_end = full_match.end();

        // Parse the JSON inside the tag
        match serde_json::from_str::<Value>(json_str) {
            Ok(obj) => {
                let name = obj.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if name.is_empty() {
                    warn!("XML tool_call missing 'name' field: {}", json_str);
                    continue;
                }

                let arguments = obj.get("arguments")
                    .cloned()
                    .unwrap_or(Value::Object(serde_json::Map::new()));

                debug!("Parsed XML tool call: {}", name);
                tool_calls.push(ToolCall {
                    id: Some(format!("xml_{}", tool_calls.len())),
                    function: ToolCallFunction {
                        name,
                        arguments,
                    },
                });
            }
            Err(e) => {
                warn!("Failed to parse XML tool_call JSON: {} — {}", json_str, e);
            }
        }
    }

    // Collect remaining text after last tag
    let remaining = normalized[last_end..].trim();
    if !remaining.is_empty() {
        text_parts.push(remaining.to_string());
    }

    // If no tool_call tags found, try <function=name> format (Qwen-style)
    if tool_calls.is_empty() {
        let (fn_text, fn_calls) = parse_function_tag_calls(content);
        if !fn_calls.is_empty() {
            return (fn_text, fn_calls);
        }
    }

    // If still no tool calls, try raw JSON objects: {"name": "tool", "parameters": {...}}
    if tool_calls.is_empty() {
        let (json_text, json_calls) = parse_raw_json_tool_calls(content);
        if !json_calls.is_empty() {
            return (json_text, json_calls);
        }
    }

    let clean_text = text_parts.join("\n");
    (clean_text, tool_calls)
}

/// Parse Qwen-style <function=name><parameter=key>value</parameter></function> tags.
fn parse_function_tag_calls(content: &str) -> (String, Vec<ToolCall>) {
    let re = Regex::new(r"(?s)<function=(\w+)>(.*?)</function>").unwrap();

    let mut tool_calls = Vec::new();
    let mut text_parts = Vec::new();
    let mut last_end = 0;

    for cap in re.captures_iter(content) {
        let full_match = cap.get(0).unwrap();
        let func_name = cap.get(1).unwrap().as_str().to_string();
        let body = cap.get(2).unwrap().as_str();

        let text_before = content[last_end..full_match.start()].trim();
        if !text_before.is_empty() {
            text_parts.push(text_before.to_string());
        }
        last_end = full_match.end();

        // Parse <parameter=key>value</parameter> pairs
        let param_re = Regex::new(r"(?s)<parameter=(\w+)>\s*(.*?)\s*</parameter>").unwrap();
        let mut args = serde_json::Map::new();
        for pcap in param_re.captures_iter(body) {
            let key = pcap.get(1).unwrap().as_str().to_string();
            let val = pcap.get(2).unwrap().as_str().trim().to_string();
            args.insert(key, Value::String(val));
        }

        if !func_name.is_empty() {
            debug!("Parsed function-tag tool call: {}", func_name);
            tool_calls.push(ToolCall {
                id: Some(format!("fn_{}", tool_calls.len())),
                function: ToolCallFunction {
                    name: func_name,
                    arguments: Value::Object(args),
                },
            });
        }
    }

    let remaining = content[last_end..].trim();
    if !remaining.is_empty() {
        text_parts.push(remaining.to_string());
    }

    (text_parts.join("\n"), tool_calls)
}

/// Parse raw JSON tool call objects embedded in text.
/// Matches standalone `{"name": "tool_name", "parameters": {...}}` or `{"name": "...", "arguments": {...}}`.
/// Only matches if the JSON has a "name" string field (to avoid false positives on regular JSON).
fn parse_raw_json_tool_calls(content: &str) -> (String, Vec<ToolCall>) {
    let mut tool_calls = Vec::new();
    let mut text_parts = Vec::new();
    let mut last_end = 0;

    // Find JSON objects at line boundaries
    let mut chars = content.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if ch == '{' && start >= last_end {
            // Try to find matching closing brace
            let mut depth = 1;
            let mut end = start;
            for (i, c) in content[start + 1..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = start + 1 + i;
                            break;
                        }
                    }
                    _ => {}
                }
            }

            if depth == 0 {
                let json_str = &content[start..=end];
                // Try to parse as a tool call
                if let Ok(obj) = serde_json::from_str::<Value>(json_str) {
                    if let Some(name) = obj.get("name").and_then(|v| v.as_str()) {
                        if !name.is_empty() && name.len() < 64 {
                            // Accept "parameters" or "arguments"
                            let arguments = obj
                                .get("arguments")
                                .or_else(|| obj.get("parameters"))
                                .cloned()
                                .unwrap_or(Value::Object(serde_json::Map::new()));

                            debug!("Parsed raw JSON tool call: {}", name);
                            tool_calls.push(ToolCall {
                                id: Some(format!("json_{}", tool_calls.len())),
                                function: ToolCallFunction {
                                    name: name.to_string(),
                                    arguments,
                                },
                            });

                            // Collect text before this JSON
                            let text_before = content[last_end..start].trim();
                            if !text_before.is_empty() {
                                text_parts.push(text_before.to_string());
                            }
                            last_end = end + 1;
                        }
                    }
                }
            }
        }
    }

    let remaining = content[last_end..].trim();
    if !remaining.is_empty() {
        text_parts.push(remaining.to_string());
    }

    (text_parts.join("\n"), tool_calls)
}

/// Normalize variant tool call tags to canonical <tool_call> form.
fn normalize_tool_tags(content: &str) -> String {
    let mut result = content.to_string();
    // Normalize opening tags
    for variant in &["<toolcall>", "<tool-call>", "<invoke>"] {
        result = result.replace(variant, "<tool_call>");
    }
    // Normalize closing tags
    for variant in &["</toolcall>", "</tool-call>", "</invoke>"] {
        result = result.replace(variant, "</tool_call>");
    }
    result
}

/// Determine dispatch mode from provider name.
/// Providers known to support native function calling get Native mode;
/// local models that may not support it get Auto mode.
pub fn dispatch_mode_for_provider(provider: &str) -> DispatchMode {
    match provider {
        "anthropic" | "openai" => DispatchMode::Native,
        "gemini" => DispatchMode::Auto, // Gemini sometimes outputs tool calls as text JSON
        "groq" => DispatchMode::Auto, // Groq Llama may output <function=name> tags instead of native tool_calls
        "chatgpt" => DispatchMode::Auto, // Codex CLI handles tools internally; parse XML fallback
        "ollama" | "lmstudio" => DispatchMode::Auto,
        _ => DispatchMode::Auto,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{ChatResponse, TokenUsage};

    fn make_response(content: &str, tool_calls: Vec<ToolCall>) -> ChatResponse {
        ChatResponse {
            message: ChatMessage {
                role: "assistant".to_string(),
                content: content.to_string(),
                tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
                tool_call_id: None,
            },
            usage: Some(TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 10,
                total_tokens: 20,
            }),
        }
    }

    fn native_tc(name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: Some("tc_1".to_string()),
            function: ToolCallFunction {
                name: name.to_string(),
                arguments: serde_json::from_str(args).unwrap(),
            },
        }
    }

    // ── Native mode tests ────────────────────────────────────────────────────

    #[test]
    fn test_native_parse_with_tool_calls() {
        let resp = make_response("", vec![native_tc("shell", r#"{"command":"ls"}"#)]);
        let (text, calls) = parse_tool_calls(&resp, DispatchMode::Native);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "shell");
        assert!(text.is_empty());
    }

    #[test]
    fn test_native_parse_no_tool_calls() {
        let resp = make_response("Hello world", vec![]);
        let (text, calls) = parse_tool_calls(&resp, DispatchMode::Native);
        assert!(calls.is_empty());
        assert_eq!(text, "Hello world");
    }

    // ── XML mode tests ───────────────────────────────────────────────────────

    #[test]
    fn test_xml_parse_single_tool_call() {
        let content = r#"Let me check.
<tool_call>
{"name": "shell", "arguments": {"command": "ls -la"}}
</tool_call>"#;
        let resp = make_response(content, vec![]);
        let (text, calls) = parse_tool_calls(&resp, DispatchMode::Xml);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "shell");
        assert_eq!(calls[0].function.arguments["command"], "ls -la");
        assert_eq!(text, "Let me check.");
    }

    #[test]
    fn test_xml_parse_multiple_tool_calls() {
        let content = r#"Searching...
<tool_call>
{"name": "web_search", "arguments": {"query": "rust async"}}
</tool_call>
Also reading a file.
<tool_call>
{"name": "file_read", "arguments": {"path": "src/main.rs"}}
</tool_call>"#;
        let resp = make_response(content, vec![]);
        let (text, calls) = parse_tool_calls(&resp, DispatchMode::Xml);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.name, "web_search");
        assert_eq!(calls[1].function.name, "file_read");
        assert!(text.contains("Searching..."));
        assert!(text.contains("Also reading a file."));
    }

    #[test]
    fn test_xml_parse_variant_tags() {
        for tag in &["toolcall", "tool-call", "invoke"] {
            let content = format!(
                "<{0}>{{\"name\": \"shell\", \"arguments\": {{\"command\": \"ls\"}}}}</{0}>",
                tag
            );
            let resp = make_response(&content, vec![]);
            let (_, calls) = parse_tool_calls(&resp, DispatchMode::Xml);
            assert_eq!(calls.len(), 1, "Failed for tag variant: {}", tag);
        }
    }

    #[test]
    fn test_xml_parse_malformed_json() {
        let content = r#"<tool_call>not valid json</tool_call>"#;
        let resp = make_response(content, vec![]);
        let (_, calls) = parse_tool_calls(&resp, DispatchMode::Xml);
        assert!(calls.is_empty()); // Gracefully skipped
    }

    #[test]
    fn test_xml_parse_missing_name() {
        let content = r#"<tool_call>{"arguments": {"x": 1}}</tool_call>"#;
        let resp = make_response(content, vec![]);
        let (_, calls) = parse_tool_calls(&resp, DispatchMode::Xml);
        assert!(calls.is_empty()); // Skipped due to missing name
    }

    // ── Auto mode tests ──────────────────────────────────────────────────────

    #[test]
    fn test_auto_prefers_native() {
        let content = r#"<tool_call>{"name": "shell", "arguments": {"command": "ls"}}</tool_call>"#;
        let native = vec![native_tc("file_read", r#"{"path":"a.txt"}"#)];
        let resp = make_response(content, native);
        let (_, calls) = parse_tool_calls(&resp, DispatchMode::Auto);
        // Should use native, not XML
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "file_read");
    }

    #[test]
    fn test_auto_falls_back_to_xml() {
        let content = r#"Let me run that.
<tool_call>{"name": "shell", "arguments": {"command": "pwd"}}</tool_call>"#;
        let resp = make_response(content, vec![]);
        let (text, calls) = parse_tool_calls(&resp, DispatchMode::Auto);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "shell");
        assert_eq!(text, "Let me run that.");
    }

    #[test]
    fn test_auto_no_tool_calls() {
        let resp = make_response("Just a normal response.", vec![]);
        let (text, calls) = parse_tool_calls(&resp, DispatchMode::Auto);
        assert!(calls.is_empty());
        assert_eq!(text, "Just a normal response.");
    }

    // ── function tag (Qwen-style) tests ─────────────────────────────────────

    #[test]
    fn test_function_tag_parse() {
        let content = r#"Let me run that.
<function=shell>
<parameter=command>sqlite3 ~/.clawtex/costs.db "SELECT * FROM cost_records;"</parameter>
<parameter=timeout_secs>60</parameter>
</function>"#;
        let resp = make_response(content, vec![]);
        let (text, calls) = parse_tool_calls(&resp, DispatchMode::Auto);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "shell");
        assert!(calls[0].function.arguments["command"].as_str().unwrap().contains("sqlite3"));
        assert_eq!(calls[0].function.arguments["timeout_secs"], "60");
        assert_eq!(text, "Let me run that.");
    }

    #[test]
    fn test_function_tag_multiple() {
        let content = r#"<function=shell>
<parameter=command>ls</parameter>
</function>
<function=file_read>
<parameter=path>test.txt</parameter>
</function>"#;
        let resp = make_response(content, vec![]);
        let (_, calls) = parse_tool_calls(&resp, DispatchMode::Auto);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.name, "shell");
        assert_eq!(calls[1].function.name, "file_read");
    }

    // ── raw JSON tool call tests ──────────────────────────────────────────────

    #[test]
    fn test_raw_json_single_tool_call() {
        let content = r#"I'll search for that.
{"name": "web_search", "parameters": {"query": "AI frameworks 2026"}}
Let me also check memory."#;
        let resp = make_response(content, vec![]);
        let (text, calls) = parse_tool_calls(&resp, DispatchMode::Auto);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "web_search");
        assert_eq!(calls[0].function.arguments["query"], "AI frameworks 2026");
        assert!(text.contains("I'll search for that."));
    }

    #[test]
    fn test_raw_json_multiple_tool_calls() {
        let content = r#"{"name": "web_search", "parameters": {"query": "top AI agents"}}
{"name": "memory_store", "arguments": {"key": "research", "content": "test"}}"#;
        let resp = make_response(content, vec![]);
        let (_, calls) = parse_tool_calls(&resp, DispatchMode::Auto);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.name, "web_search");
        assert_eq!(calls[1].function.name, "memory_store");
    }

    #[test]
    fn test_raw_json_no_name_field_ignored() {
        let content = r#"Here is some data: {"key": "value", "count": 42}"#;
        let resp = make_response(content, vec![]);
        let (text, calls) = parse_tool_calls(&resp, DispatchMode::Auto);
        assert!(calls.is_empty());
        assert!(text.contains("some data"));
    }

    // ── dispatch_mode_for_provider tests ─────────────────────────────────────

    #[test]
    fn test_dispatch_mode_providers() {
        assert_eq!(dispatch_mode_for_provider("anthropic"), DispatchMode::Native);
        assert_eq!(dispatch_mode_for_provider("openai"), DispatchMode::Native);
        assert_eq!(dispatch_mode_for_provider("groq"), DispatchMode::Auto);
        assert_eq!(dispatch_mode_for_provider("ollama"), DispatchMode::Auto);
        assert_eq!(dispatch_mode_for_provider("lmstudio"), DispatchMode::Auto);
        assert_eq!(dispatch_mode_for_provider("unknown"), DispatchMode::Auto);
    }

    // ── xml_tool_instructions tests ──────────────────────────────────────────

    #[test]
    fn test_xml_instructions_empty() {
        assert_eq!(xml_tool_instructions(&[]), "");
    }

    #[test]
    fn test_xml_instructions_with_tools() {
        let specs = vec![crate::tools::ToolSpec {
            name: "shell".to_string(),
            description: "Run a shell command".to_string(),
            parameters: serde_json::json!({"type": "object", "properties": {"command": {"type": "string"}}}),
        }];
        let instructions = xml_tool_instructions(&specs);
        assert!(instructions.contains("<tool_call>"));
        assert!(instructions.contains("shell"));
        assert!(instructions.contains("Run a shell command"));
    }
}
