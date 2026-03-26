use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::pin::Pin;
use futures_util::Stream;

// ── Core types (canonical location, re-exported from llm_router for compat) ──

/// LLM provider configuration from agents.toml
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    #[serde(rename = "type")]
    pub provider_type: String,   // "ollama" | "openai_compat" | "openai" | "anthropic"
    pub url: Option<String>,
    pub default_model: Option<String>,
    pub api_key: Option<String>,
}

/// Route hint configuration: maps hint names to provider+model combos
#[derive(Debug, Clone, Deserialize)]
pub struct RouteHint {
    pub hint: String,
    pub provider: String,
    pub model: Option<String>,
}

/// LLM routing result
#[derive(Debug, Serialize)]
pub struct LlmResponse {
    pub text: String,
    pub provider_used: String,
    pub model_used: String,
}

/// Chat message for multi-turn conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// For tool result messages (role="tool"), the ID of the tool call this responds to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// A single tool call from the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique ID for this tool call (OpenAI-compat requires this)
    #[serde(default)]
    pub id: Option<String>,
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: Value,
}

/// Token usage stats from an LLM call
#[derive(Debug, Clone, Default, Serialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Chat response from LLM (with possible tool calls)
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub message: ChatMessage,
    pub usage: Option<TokenUsage>,
}

// ── Streaming types ──

/// A chunk from a streaming LLM response
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Partial text content
    ContentDelta(String),
    /// Start of a tool call
    ToolCallStart { id: String, name: String },
    /// Partial arguments JSON for a tool call
    ToolCallArgumentsDelta { id: String, delta: String },
    /// Stream complete
    Done { usage: Option<TokenUsage> },
}

/// Provider capabilities (informational)
#[derive(Debug, Clone, Default)]
pub struct ProviderCapabilities {
    pub streaming: bool,
    pub native_tools: bool,
    pub vision: bool,
}

// ── Provider trait ──

/// Trait implemented by all LLM providers
#[async_trait]
pub trait Provider: Send + Sync {
    /// Provider display name (e.g. "ollama", "anthropic")
    fn name(&self) -> &str;

    /// Default model for this provider
    fn default_model(&self) -> &str;

    /// Advertised capabilities
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }

    /// Non-streaming chat with optional tool definitions.
    /// `model` can be empty to use the provider's default.
    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<ChatResponse>;

    /// Streaming chat — returns a stream of chunks.
    /// Default implementation wraps `chat()` into a single-chunk stream.
    async fn stream_chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let resp = self.chat(messages, tools, model).await?;
        let text = resp.message.content.clone();
        let usage = resp.usage.clone();
        Ok(Box::pin(futures_util::stream::iter(vec![
            Ok(StreamChunk::ContentDelta(text)),
            Ok(StreamChunk::Done { usage }),
        ])))
    }

    /// Health check — is this provider reachable?
    async fn is_alive(&self) -> bool;
}

// ── Helper functions ──

/// Convert ChatMessage array to OpenAI-compatible JSON format
/// (handles tool_calls serialization with "type":"function" and string arguments)
pub fn messages_to_openai_json(messages: &[ChatMessage]) -> Vec<Value> {
    messages.iter().map(|m| {
        let mut obj = json!({ "role": m.role });

        // tool result messages need tool_call_id, content
        if m.role == "tool" {
            obj["content"] = json!(m.content);
            if let Some(ref id) = m.tool_call_id {
                obj["tool_call_id"] = json!(id);
            }
            return obj;
        }

        // Assistant messages with tool_calls
        if let Some(ref tcs) = m.tool_calls {
            if !tcs.is_empty() {
                obj["content"] = Value::Null;
                let tc_json: Vec<Value> = tcs.iter().map(|tc| {
                    json!({
                        "id": tc.id.as_deref().unwrap_or("call_0"),
                        "type": "function",
                        "function": {
                            "name": tc.function.name,
                            "arguments": serde_json::to_string(&tc.function.arguments).unwrap_or_default()
                        }
                    })
                }).collect();
                obj["tool_calls"] = json!(tc_json);
                return obj;
            }
        }

        obj["content"] = json!(m.content);
        obj
    }).collect()
}

/// Convert OpenAI-style tool definitions to Anthropic format
pub fn tools_to_anthropic_json(tools: &[Value]) -> Vec<Value> {
    tools.iter().filter_map(|t| {
        let func = t.get("function")?;
        let name = func.get("name")?.as_str()?;
        let desc = func.get("description").and_then(|v| v.as_str()).unwrap_or("");
        let params = func.get("parameters").cloned().unwrap_or(json!({"type": "object", "properties": {}}));
        Some(json!({
            "name": name,
            "description": desc,
            "input_schema": params,
        }))
    }).collect()
}

/// Convert ChatMessage array to Anthropic Messages API format.
/// Returns (system_prompt, messages_array).
/// Extracts system messages, merges consecutive same-role messages.
pub fn messages_to_anthropic_json(messages: &[ChatMessage]) -> (Option<Value>, Vec<Value>) {
    let mut system_parts: Vec<String> = Vec::new();
    let mut result: Vec<Value> = Vec::new();

    for m in messages {
        if m.role == "system" {
            system_parts.push(m.content.clone());
            continue;
        }

        if m.role == "tool" {
            // Anthropic: tool results are user messages with tool_result content block
            let block = json!({
                "type": "tool_result",
                "tool_use_id": m.tool_call_id.as_deref().unwrap_or(""),
                "content": m.content,
            });
            // Merge into previous user message or create new one
            if let Some(last) = result.last_mut() {
                if last.get("role").and_then(|v| v.as_str()) == Some("user") {
                    if let Some(arr) = last.get_mut("content").and_then(|v| v.as_array_mut()) {
                        arr.push(block);
                        continue;
                    }
                }
            }
            result.push(json!({
                "role": "user",
                "content": [block],
            }));
            continue;
        }

        if m.role == "assistant" {
            if let Some(ref tcs) = m.tool_calls {
                if !tcs.is_empty() {
                    // Anthropic: tool calls are content blocks of type "tool_use"
                    let mut blocks: Vec<Value> = Vec::new();
                    if !m.content.is_empty() {
                        blocks.push(json!({"type": "text", "text": m.content}));
                    }
                    for tc in tcs {
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": tc.id.as_deref().unwrap_or("call_0"),
                            "name": tc.function.name,
                            "input": tc.function.arguments,
                        }));
                    }
                    result.push(json!({
                        "role": "assistant",
                        "content": blocks,
                    }));
                    continue;
                }
            }
        }

        // Regular user/assistant message
        // Merge consecutive same-role messages
        if let Some(last) = result.last_mut() {
            if last.get("role").and_then(|v| v.as_str()) == Some(&m.role) {
                // Merge content
                if let Some(existing) = last.get("content").and_then(|v| v.as_str()) {
                    let merged = format!("{}\n{}", existing, m.content);
                    last["content"] = json!(merged);
                    continue;
                }
            }
        }

        result.push(json!({
            "role": m.role,
            "content": m.content,
        }));
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        let text = system_parts.join("\n");
        Some(json!([{
            "type": "text",
            "text": text,
            "cache_control": { "type": "ephemeral" }
        }]))
    };

    (system, result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_chunk_variants() {
        let delta = StreamChunk::ContentDelta("hello".into());
        assert!(matches!(delta, StreamChunk::ContentDelta(ref s) if s == "hello"));

        let done = StreamChunk::Done { usage: None };
        assert!(matches!(done, StreamChunk::Done { usage: None }));

        let tc_start = StreamChunk::ToolCallStart { id: "1".into(), name: "shell".into() };
        assert!(matches!(tc_start, StreamChunk::ToolCallStart { ref id, ref name } if id == "1" && name == "shell"));
    }

    #[test]
    fn test_provider_capabilities_default() {
        let caps = ProviderCapabilities::default();
        assert!(!caps.streaming);
        assert!(!caps.native_tools);
        assert!(!caps.vision);
    }

    #[test]
    fn test_messages_to_openai_json_basic() {
        let msgs = vec![
            ChatMessage { role: "user".into(), content: "hi".into(), tool_calls: None, tool_call_id: None },
        ];
        let json = messages_to_openai_json(&msgs);
        assert_eq!(json.len(), 1);
        assert_eq!(json[0]["role"], "user");
        assert_eq!(json[0]["content"], "hi");
    }

    #[test]
    fn test_messages_to_openai_json_tool_calls() {
        let msgs = vec![
            ChatMessage {
                role: "assistant".into(),
                content: "".into(),
                tool_calls: Some(vec![ToolCall {
                    id: Some("call_1".into()),
                    function: ToolCallFunction {
                        name: "shell".into(),
                        arguments: json!({"command": "ls"}),
                    },
                }]),
                tool_call_id: None,
            },
        ];
        let json = messages_to_openai_json(&msgs);
        assert_eq!(json[0]["tool_calls"].as_array().unwrap().len(), 1);
        assert!(json[0]["content"].is_null());
    }

    #[test]
    fn test_messages_to_openai_json_tool_result() {
        let msgs = vec![
            ChatMessage {
                role: "tool".into(),
                content: "file list".into(),
                tool_calls: None,
                tool_call_id: Some("call_1".into()),
            },
        ];
        let json = messages_to_openai_json(&msgs);
        assert_eq!(json[0]["role"], "tool");
        assert_eq!(json[0]["content"], "file list");
        assert_eq!(json[0]["tool_call_id"], "call_1");
    }

    #[test]
    fn test_tools_to_anthropic_json() {
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "shell",
                "description": "Run a command",
                "parameters": {"type": "object", "properties": {"command": {"type": "string"}}}
            }
        })];
        let result = tools_to_anthropic_json(&tools);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["name"], "shell");
        assert!(result[0].get("input_schema").is_some());
    }

    #[test]
    fn test_messages_to_anthropic_extracts_system() {
        let msgs = vec![
            ChatMessage { role: "system".into(), content: "You are helpful".into(), tool_calls: None, tool_call_id: None },
            ChatMessage { role: "user".into(), content: "hello".into(), tool_calls: None, tool_call_id: None },
        ];
        let (sys, msgs_out) = messages_to_anthropic_json(&msgs);
        let sys = sys.unwrap();
        let blocks = sys.as_array().unwrap();
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[0]["text"], "You are helpful");
        assert_eq!(blocks[0]["cache_control"]["type"], "ephemeral");
        assert_eq!(msgs_out.len(), 1);
        assert_eq!(msgs_out[0]["role"], "user");
    }

    #[test]
    fn test_messages_to_anthropic_tool_roundtrip() {
        let msgs = vec![
            ChatMessage {
                role: "assistant".into(), content: "".into(),
                tool_calls: Some(vec![ToolCall {
                    id: Some("tc1".into()),
                    function: ToolCallFunction { name: "shell".into(), arguments: json!({"command": "ls"}) },
                }]),
                tool_call_id: None,
            },
            ChatMessage {
                role: "tool".into(), content: "file1.rs".into(),
                tool_calls: None, tool_call_id: Some("tc1".into()),
            },
        ];
        let (_, msgs_out) = messages_to_anthropic_json(&msgs);
        assert_eq!(msgs_out.len(), 2);
        // Assistant message has tool_use block
        let blocks = msgs_out[0]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "tool_use");
        // Tool result is user message with tool_result block
        assert_eq!(msgs_out[1]["role"], "user");
        let result_blocks = msgs_out[1]["content"].as_array().unwrap();
        assert_eq!(result_blocks[0]["type"], "tool_result");
    }

    #[test]
    fn test_messages_to_anthropic_merges_consecutive() {
        let msgs = vec![
            ChatMessage { role: "user".into(), content: "first".into(), tool_calls: None, tool_call_id: None },
            ChatMessage { role: "user".into(), content: "second".into(), tool_calls: None, tool_call_id: None },
        ];
        let (_, msgs_out) = messages_to_anthropic_json(&msgs);
        assert_eq!(msgs_out.len(), 1);
        assert!(msgs_out[0]["content"].as_str().unwrap().contains("first"));
        assert!(msgs_out[0]["content"].as_str().unwrap().contains("second"));
    }

    #[test]
    fn test_token_usage_default() {
        let usage = TokenUsage::default();
        assert_eq!(usage.prompt_tokens, 0);
        assert_eq!(usage.completion_tokens, 0);
        assert_eq!(usage.total_tokens, 0);
    }
}
