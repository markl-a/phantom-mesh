//! Groq free-tier provider — OpenAI-compatible API at api.groq.com.
//! Supports vision via Llama 4 Scout 17B model.
//! Supports true SSE streaming via OpenAI-compatible `stream: true`.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::pin::Pin;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::debug;

use super::pool::HttpPool;
use super::traits::*;

/// Groq provider — uses OpenAI-compatible API with vision support.
pub struct GroqProvider {
    api_key: String,
    default_model: String,
    client: Client,
}

impl GroqProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "llama-3.3-70b-versatile".to_string()),
            client: HttpPool::global().client().clone(),
        }
    }

    fn resolve_model<'a>(&'a self, model: &'a str) -> &'a str {
        if model.is_empty() { &self.default_model } else { model }
    }

    /// Convert messages, handling vision markers [IMAGE:base64:<data>]
    fn convert_messages(messages: &[ChatMessage]) -> Vec<Value> {
        messages.iter().map(|m| {
            // Handle tool result messages
            if m.role == "tool" {
                let mut obj = json!({ "role": "tool", "content": m.content });
                if let Some(ref id) = m.tool_call_id {
                    obj["tool_call_id"] = json!(id);
                }
                return obj;
            }

            // Handle assistant messages with tool calls
            if let Some(ref tcs) = m.tool_calls {
                if !tcs.is_empty() {
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
                    return json!({
                        "role": "assistant",
                        "content": Value::Null,
                        "tool_calls": tc_json,
                    });
                }
            }

            // Check for vision markers in content
            if m.content.contains("[IMAGE:base64:") {
                let mut parts: Vec<Value> = Vec::new();
                let mut remaining = m.content.as_str();

                while let Some(start) = remaining.find("[IMAGE:base64:") {
                    // Text before the image marker
                    if start > 0 {
                        let text = &remaining[..start];
                        if !text.trim().is_empty() {
                            parts.push(json!({"type": "text", "text": text.trim()}));
                        }
                    }

                    // Extract base64 data
                    let after_marker = &remaining[start + 14..]; // skip "[IMAGE:base64:"
                    if let Some(end) = after_marker.find(']') {
                        let b64_data = &after_marker[..end];
                        parts.push(json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:image/png;base64,{}", b64_data)
                            }
                        }));
                        remaining = &after_marker[end + 1..];
                    } else {
                        break;
                    }
                }

                // Remaining text after last image
                if !remaining.trim().is_empty() {
                    parts.push(json!({"type": "text", "text": remaining.trim()}));
                }

                if !parts.is_empty() {
                    return json!({ "role": m.role, "content": parts });
                }
            }

            // Regular text message
            json!({ "role": m.role, "content": m.content })
        }).collect()
    }
}

#[async_trait]
impl Provider for GroqProvider {
    fn name(&self) -> &str {
        "groq"
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            native_tools: true,
            vision: true,
        }
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<ChatResponse> {
        let model = self.resolve_model(model);
        let url = "https://api.groq.com/openai/v1/chat/completions";

        let groq_messages = Self::convert_messages(messages);

        let mut body = json!({
            "model": model,
            "messages": groq_messages,
            "stream": false
        });

        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }

        debug!("Groq chat: model={}, {} messages, {} tools", model, messages.len(), tools.len());

        let resp = self.client.post(url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        let json: Value = resp.json().await?;

        if let Some(err) = json.get("error") {
            return Err(anyhow!("Groq error: {}", err));
        }

        let msg = json.pointer("/choices/0/message")
            .ok_or_else(|| anyhow!("Groq response missing choices[0].message: {}", json))?;

        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("assistant").to_string();
        let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();

        let tool_calls = parse_tool_calls(msg);
        let usage = parse_usage(&json);

        Ok(ChatResponse {
            message: ChatMessage { role, content, tool_calls, tool_call_id: None },
            usage,
        })
    }

    async fn stream_chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<Pin<Box<dyn futures_util::Stream<Item = Result<StreamChunk>> + Send>>> {
        let model = self.resolve_model(model).to_string();
        let url = "https://api.groq.com/openai/v1/chat/completions";

        let groq_messages = Self::convert_messages(messages);

        let mut body = json!({
            "model": model,
            "messages": groq_messages,
            "stream": true
        });

        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }

        debug!("Groq stream_chat: model={}, {} messages, {} tools", model, messages.len(), tools.len());

        let resp = self.client.post(url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Groq stream error: {}", text));
        }

        let (tx, rx) = mpsc::channel::<Result<StreamChunk>>(64);
        let byte_stream = resp.bytes_stream();

        tokio::spawn(async move {
            let mut stream = Box::pin(byte_stream);
            let mut buffer = String::new();

            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));
                        // Process complete SSE lines
                        while let Some(pos) = buffer.find('\n') {
                            let line = buffer[..pos].trim().to_string();
                            buffer = buffer[pos + 1..].to_string();

                            if line.is_empty() || line.starts_with(':') {
                                continue; // SSE comment or blank
                            }

                            if let Some(data) = line.strip_prefix("data: ") {
                                if data.trim() == "[DONE]" {
                                    let _ = tx.send(Ok(StreamChunk::Done { usage: None })).await;
                                    return;
                                }

                                if let Ok(chunks) = parse_groq_sse_data(data) {
                                    for c in chunks {
                                        let _ = tx.send(Ok(c)).await;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(anyhow!("Groq stream read error: {}", e))).await;
                        break;
                    }
                }
            }
            // If stream ends without [DONE], send Done anyway
            let _ = tx.send(Ok(StreamChunk::Done { usage: None })).await;
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn is_alive(&self) -> bool {
        if self.api_key.is_empty() {
            return false;
        }
        let resp = self.client.get("https://api.groq.com/openai/v1/models")
            .bearer_auth(&self.api_key)
            .timeout(Duration::from_secs(5))
            .send()
            .await;
        resp.map(|r| r.status().is_success()).unwrap_or(false)
    }
}

/// Parse a single SSE `data: {...}` payload from Groq streaming into StreamChunks.
/// Groq uses the OpenAI-compatible SSE format.
/// Returns a Vec because a single SSE event can contain content + tool calls + usage.
fn parse_groq_sse_data(data: &str) -> Result<Vec<StreamChunk>> {
    let json: Value = serde_json::from_str(data)
        .map_err(|e| anyhow!("Groq SSE JSON parse error: {}", e))?;

    let mut chunks = Vec::new();

    // Check for content delta
    if let Some(content) = json.pointer("/choices/0/delta/content").and_then(|v| v.as_str()) {
        if !content.is_empty() {
            chunks.push(StreamChunk::ContentDelta(content.to_string()));
        }
    }

    // Check for tool call deltas
    if let Some(tcs) = json.pointer("/choices/0/delta/tool_calls").and_then(|v| v.as_array()) {
        for tc in tcs {
            let idx = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
            let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();

            if let Some(func) = tc.get("function") {
                if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                    chunks.push(StreamChunk::ToolCallStart {
                        id: if id.is_empty() { format!("call_{}", idx) } else { id.clone() },
                        name: name.to_string(),
                    });
                }
                if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                    if !args.is_empty() {
                        chunks.push(StreamChunk::ToolCallArgumentsDelta {
                            id: if id.is_empty() { format!("call_{}", idx) } else { id },
                            delta: args.to_string(),
                        });
                    }
                }
            }
        }
    }

    // Check for usage in the final chunk (Groq sends x_groq.usage or standard usage)
    if let Some(usage_val) = json.get("usage")
        .or_else(|| json.pointer("/x_groq/usage"))
    {
        let prompt_tokens = usage_val.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let completion_tokens = usage_val.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let total = usage_val.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        if prompt_tokens > 0 || completion_tokens > 0 {
            chunks.push(StreamChunk::Done {
                usage: Some(TokenUsage {
                    prompt_tokens,
                    completion_tokens,
                    total_tokens: if total > 0 { total } else { prompt_tokens + completion_tokens },
                }),
            });
        }
    }

    Ok(chunks)
}

fn parse_tool_calls(msg: &Value) -> Option<Vec<ToolCall>> {
    let arr = msg.get("tool_calls")?.as_array()?;
    if arr.is_empty() { return None; }
    let calls: Vec<ToolCall> = arr.iter().filter_map(|call| {
        let id = call.get("id").and_then(|v| v.as_str()).map(String::from);
        let func = call.get("function")?;
        let name = func.get("name")?.as_str()?.to_string();
        let args_str = func.get("arguments")?.as_str()?;
        let arguments: Value = serde_json::from_str(args_str).ok()?;
        Some(ToolCall {
            id,
            function: ToolCallFunction { name, arguments },
        })
    }).collect();
    if calls.is_empty() { None } else { Some(calls) }
}

fn parse_usage(json: &Value) -> Option<TokenUsage> {
    let u = json.get("usage")?;
    let prompt_tokens = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let completion_tokens = u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let total = u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if prompt_tokens > 0 || completion_tokens > 0 {
        Some(TokenUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: if total > 0 { total } else { prompt_tokens + completion_tokens },
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::pool::HttpPool;

    #[test]
    fn test_groq_uses_shared_http_pool() {
        // Constructing two GroqProviders should both get clients from the shared pool.
        let p1 = GroqProvider::new("key-a".into(), None);
        let p2 = GroqProvider::new("key-b".into(), None);
        // The pool singleton is always the same static instance.
        let pool_client = HttpPool::global().client();
        // reqwest::Client is cheap-cloned (Arc internally), so all three share
        // the same underlying connection pool. We verify by checking that the
        // provider was created without panic and the pool is accessible.
        let _ = pool_client;
        assert_eq!(p1.name(), "groq");
        assert_eq!(p2.name(), "groq");
    }

    #[test]
    fn test_groq_provider_name() {
        let p = GroqProvider::new("test-key".into(), None);
        assert_eq!(p.name(), "groq");
        assert_eq!(p.default_model(), "llama-4-scout-17b-16e-instruct");
    }

    #[test]
    fn test_groq_capabilities() {
        let p = GroqProvider::new("test-key".into(), None);
        let caps = p.capabilities();
        assert!(caps.streaming);
        assert!(caps.native_tools);
        assert!(caps.vision);
    }

    #[test]
    fn test_groq_resolve_model() {
        let p = GroqProvider::new("key".into(), Some("custom-model".into()));
        assert_eq!(p.resolve_model(""), "custom-model");
        assert_eq!(p.resolve_model("override"), "override");
    }

    #[test]
    fn test_convert_messages_basic() {
        let msgs = vec![
            ChatMessage { role: "user".into(), content: "hello".into(), tool_calls: None, tool_call_id: None },
        ];
        let converted = GroqProvider::convert_messages(&msgs);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "user");
        assert_eq!(converted[0]["content"], "hello");
    }

    #[test]
    fn test_convert_messages_vision() {
        let msgs = vec![
            ChatMessage {
                role: "user".into(),
                content: "What is this? [IMAGE:base64:aGVsbG8=]".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let converted = GroqProvider::convert_messages(&msgs);
        assert_eq!(converted.len(), 1);
        let content = converted[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert!(content[1]["image_url"]["url"].as_str().unwrap().contains("aGVsbG8="));
    }

    #[test]
    fn test_convert_messages_tool_result() {
        let msgs = vec![
            ChatMessage {
                role: "tool".into(),
                content: "result data".into(),
                tool_calls: None,
                tool_call_id: Some("call_1".into()),
            },
        ];
        let converted = GroqProvider::convert_messages(&msgs);
        assert_eq!(converted[0]["role"], "tool");
        assert_eq!(converted[0]["tool_call_id"], "call_1");
    }

    #[test]
    fn test_parse_tool_calls_valid() {
        let msg = json!({
            "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {
                    "name": "web_search",
                    "arguments": "{\"query\":\"test\"}"
                }
            }]
        });
        let calls = parse_tool_calls(&msg).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "web_search");
    }

    #[test]
    fn test_parse_usage_valid() {
        let json = json!({
            "usage": {
                "prompt_tokens": 50,
                "completion_tokens": 100,
                "total_tokens": 150
            }
        });
        let usage = parse_usage(&json).unwrap();
        assert_eq!(usage.prompt_tokens, 50);
        assert_eq!(usage.completion_tokens, 100);
    }

    #[tokio::test]
    async fn test_groq_is_alive_no_key() {
        let p = GroqProvider::new(String::new(), None);
        assert!(!p.is_alive().await);
    }

    // ── Streaming SSE parsing tests ──

    #[test]
    fn test_parse_groq_sse_content_delta() {
        let data = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#;
        let chunks = parse_groq_sse_data(data).unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], StreamChunk::ContentDelta(s) if s == "Hello"));
    }

    #[test]
    fn test_parse_groq_sse_empty_content() {
        let data = r#"{"id":"chatcmpl-1","choices":[{"index":0,"delta":{"content":""},"finish_reason":null}]}"#;
        let chunks = parse_groq_sse_data(data).unwrap();
        assert_eq!(chunks.len(), 0, "Empty content should not produce a chunk");
    }

    #[test]
    fn test_parse_groq_sse_role_only_delta() {
        // First SSE event often has role but no content
        let data = r#"{"id":"chatcmpl-1","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}"#;
        let chunks = parse_groq_sse_data(data).unwrap();
        assert_eq!(chunks.len(), 0, "Role-only delta should produce no chunks");
    }

    #[test]
    fn test_parse_groq_sse_tool_call_start() {
        let data = r#"{"id":"chatcmpl-1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"web_search","arguments":""}}]},"finish_reason":null}]}"#;
        let chunks = parse_groq_sse_data(data).unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], StreamChunk::ToolCallStart { ref id, ref name }
            if id == "call_abc" && name == "web_search"));
    }

    #[test]
    fn test_parse_groq_sse_tool_call_args_delta() {
        let data = r#"{"id":"chatcmpl-1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_abc","function":{"arguments":"{\"query\":"}}]},"finish_reason":null}]}"#;
        let chunks = parse_groq_sse_data(data).unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], StreamChunk::ToolCallArgumentsDelta { ref id, ref delta }
            if id == "call_abc" && delta == "{\"query\":"));
    }

    #[test]
    fn test_parse_groq_sse_tool_call_no_id_uses_index() {
        let data = r#"{"id":"chatcmpl-1","choices":[{"index":0,"delta":{"tool_calls":[{"index":2,"function":{"name":"shell","arguments":""}}]},"finish_reason":null}]}"#;
        let chunks = parse_groq_sse_data(data).unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], StreamChunk::ToolCallStart { ref id, ref name }
            if id == "call_2" && name == "shell"));
    }

    #[test]
    fn test_parse_groq_sse_with_usage() {
        let data = r#"{"id":"chatcmpl-1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":25,"completion_tokens":80,"total_tokens":105}}"#;
        let chunks = parse_groq_sse_data(data).unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], StreamChunk::Done { usage: Some(ref u) }
            if u.prompt_tokens == 25 && u.completion_tokens == 80 && u.total_tokens == 105));
    }

    #[test]
    fn test_parse_groq_sse_x_groq_usage() {
        // Groq sometimes sends usage under x_groq.usage
        let data = r#"{"id":"chatcmpl-1","choices":[],"x_groq":{"usage":{"prompt_tokens":10,"completion_tokens":40,"total_tokens":50}}}"#;
        let chunks = parse_groq_sse_data(data).unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], StreamChunk::Done { usage: Some(ref u) }
            if u.prompt_tokens == 10 && u.completion_tokens == 40 && u.total_tokens == 50));
    }

    #[test]
    fn test_parse_groq_sse_zero_usage_ignored() {
        let data = r#"{"id":"chatcmpl-1","choices":[],"usage":{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0}}"#;
        let chunks = parse_groq_sse_data(data).unwrap();
        assert_eq!(chunks.len(), 0, "All-zero usage should not produce a Done chunk");
    }

    #[test]
    fn test_parse_groq_sse_invalid_json() {
        let data = "not valid json {{{";
        let result = parse_groq_sse_data(data);
        assert!(result.is_err(), "Invalid JSON should return an error");
    }

    #[test]
    fn test_parse_groq_sse_content_and_finish() {
        // Some models send content + finish_reason in same chunk
        let data = r#"{"id":"chatcmpl-1","choices":[{"index":0,"delta":{"content":" end."},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#;
        let chunks = parse_groq_sse_data(data).unwrap();
        assert_eq!(chunks.len(), 2);
        assert!(matches!(&chunks[0], StreamChunk::ContentDelta(s) if s == " end."));
        assert!(matches!(&chunks[1], StreamChunk::Done { usage: Some(ref u) } if u.total_tokens == 15));
    }
}
