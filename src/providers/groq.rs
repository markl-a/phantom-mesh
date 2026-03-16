//! Groq free-tier provider — OpenAI-compatible API at api.groq.com.
//! Supports vision via Llama 4 Scout 17B model.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::debug;

use super::traits::*;

/// Groq provider — uses OpenAI-compatible API with vision support.
pub struct GroqProvider {
    api_key: String,
    default_model: String,
    client: Client,
}

impl GroqProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("Failed to build HTTP client");
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "llama-4-scout-17b-16e-instruct".to_string()),
            client,
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
}
