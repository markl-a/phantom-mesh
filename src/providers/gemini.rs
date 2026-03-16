//! Gemini provider — Google's free-tier vision + text API.
//! Uses generativelanguage.googleapis.com REST API.
//! Free tier: ~1000 req/day (Flash-Lite), supports vision (base64 images).

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::debug;

use super::traits::*;

pub struct GeminiProvider {
    api_key: String,
    default_model: String,
    client: reqwest::Client,
}

impl GeminiProvider {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            default_model: default_model.unwrap_or_else(|| "gemini-2.5-flash-lite".to_string()),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Build Gemini API contents from ChatMessages.
    /// Supports vision, function calls, and function responses.
    fn build_contents(&self, messages: &[ChatMessage]) -> (Option<String>, Vec<Value>) {
        let mut contents = Vec::new();
        let mut system_text = None;

        for msg in messages {
            if msg.role == "system" {
                system_text = Some(msg.content.clone());
                continue;
            }

            // Tool result message → Gemini functionResponse
            if msg.role == "tool" {
                if let Some(ref tc_id) = msg.tool_call_id {
                    // tc_id is the tool name (or we extract from it)
                    let func_name = tc_id.clone();
                    contents.push(json!({
                        "role": "user",
                        "parts": [{
                            "functionResponse": {
                                "name": func_name,
                                "response": {
                                    "content": msg.content
                                }
                            }
                        }]
                    }));
                    continue;
                }
            }

            // Assistant message with tool_calls → Gemini functionCall parts
            if msg.role == "assistant" {
                if let Some(ref tool_calls) = msg.tool_calls {
                    if !tool_calls.is_empty() {
                        let mut parts = Vec::new();
                        // Include text content if present
                        if !msg.content.is_empty() {
                            parts.push(json!({"text": msg.content}));
                        }
                        // Add functionCall parts
                        for tc in tool_calls {
                            parts.push(json!({
                                "functionCall": {
                                    "name": tc.function.name,
                                    "args": tc.function.arguments
                                }
                            }));
                        }
                        contents.push(json!({
                            "role": "model",
                            "parts": parts
                        }));
                        continue;
                    }
                }
            }

            let role = match msg.role.as_str() {
                "assistant" => "model",
                _ => "user",
            };

            // Check for embedded image data
            if msg.content.contains("[IMAGE:base64:") {
                let parts = self.parse_multimodal_content(&msg.content);
                contents.push(json!({
                    "role": role,
                    "parts": parts
                }));
            } else {
                contents.push(json!({
                    "role": role,
                    "parts": [{"text": msg.content}]
                }));
            }
        }

        // Gemini requires alternating user/model. Merge consecutive same-role.
        merge_consecutive_roles(&mut contents);

        (system_text, contents)
    }

    /// Parse content with embedded images: "[IMAGE:base64:<data>]"
    fn parse_multimodal_content(&self, content: &str) -> Vec<Value> {
        let mut parts = Vec::new();
        let mut remaining = content;

        while let Some(start) = remaining.find("[IMAGE:base64:") {
            if start > 0 {
                let text = &remaining[..start];
                if !text.trim().is_empty() {
                    parts.push(json!({"text": text.trim()}));
                }
            }

            let data_start = start + "[IMAGE:base64:".len();
            if let Some(end) = remaining[data_start..].find(']') {
                let b64_data = &remaining[data_start..data_start + end];
                parts.push(json!({
                    "inline_data": {
                        "mime_type": "image/png",
                        "data": b64_data
                    }
                }));
                remaining = &remaining[data_start + end + 1..];
            } else {
                parts.push(json!({"text": remaining}));
                remaining = "";
            }
        }

        if !remaining.trim().is_empty() {
            parts.push(json!({"text": remaining.trim()}));
        }

        if parts.is_empty() {
            parts.push(json!({"text": content}));
        }

        parts
    }

    /// Build tool declarations for Gemini format
    fn build_tools(&self, tools: &[Value]) -> Option<Value> {
        if tools.is_empty() {
            return None;
        }

        let declarations: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                // Support both flat format and OpenAI function-wrapped format
                let (name, desc, params) = if let Some(func) = t.get("function") {
                    let name = func.get("name")?.as_str()?;
                    let desc = func.get("description").and_then(|v| v.as_str()).unwrap_or("");
                    let params = func.get("parameters").cloned()
                        .unwrap_or(json!({"type": "object", "properties": {}}));
                    (name, desc, params)
                } else {
                    let name = t.get("name")?.as_str()?;
                    let desc = t.get("description").and_then(|v| v.as_str()).unwrap_or("");
                    let params = t.get("parameters").cloned()
                        .unwrap_or(json!({"type": "object", "properties": {}}));
                    (name, desc, params)
                };
                Some(json!({
                    "name": name,
                    "description": desc,
                    "parameters": params
                }))
            })
            .collect();

        if declarations.is_empty() {
            None
        } else {
            Some(json!([{
                "function_declarations": declarations
            }]))
        }
    }
}

#[async_trait]
impl Provider for GeminiProvider {
    fn name(&self) -> &str {
        "gemini"
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
        let model = if model.is_empty() || model == "default" {
            &self.default_model
        } else {
            model
        };

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            model, self.api_key
        );

        let (system_text, contents) = self.build_contents(messages);

        let mut body = json!({
            "contents": contents,
            "generationConfig": {
                "temperature": 0.7,
                "maxOutputTokens": 4096
            }
        });

        // Add system instruction if present
        if let Some(sys) = system_text {
            body["systemInstruction"] = json!({
                "parts": [{"text": sys}]
            });
        }

        // Add tools if provided, with toolConfig to enable function calling
        if let Some(tool_decls) = self.build_tools(tools) {
            body["tools"] = tool_decls;
            body["tool_config"] = json!({
                "function_calling_config": {
                    "mode": "AUTO"
                }
            });
        }

        debug!("Gemini request: model={}, messages={}", model, messages.len());

        let resp = self.client.post(&url).json(&body).send().await?;
        let status = resp.status();

        if !status.is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Gemini API error ({}): {}",
                status,
                &err_body[..err_body.len().min(500)]
            ));
        }

        let json_resp: Value = resp.json().await?;

        // Parse response
        let candidate = json_resp
            .pointer("/candidates/0/content/parts")
            .and_then(|p| p.as_array());

        let mut content = String::new();
        let mut tool_calls = Vec::new();

        if let Some(parts) = candidate {
            for part in parts {
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    content.push_str(text);
                }
                if let Some(fc) = part.get("functionCall") {
                    let func_name = fc
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let tc = ToolCall {
                        // Use function name as ID so tool results can reference it in functionResponse
                        id: Some(func_name.clone()),
                        function: ToolCallFunction {
                            name: func_name,
                            arguments: fc
                                .get("args")
                                .cloned()
                                .unwrap_or(json!({})),
                        },
                    };
                    tool_calls.push(tc);
                }
            }
        }

        // Parse token usage
        let usage_meta = json_resp.get("usageMetadata");
        let prompt_tokens = usage_meta
            .and_then(|u| u.get("promptTokenCount"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let completion_tokens = usage_meta
            .and_then(|u| u.get("candidatesTokenCount"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        Ok(ChatResponse {
            message: ChatMessage {
                role: "assistant".to_string(),
                content,
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
                tool_call_id: None,
            },
            usage: Some(TokenUsage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
            }),
        })
    }

    async fn is_alive(&self) -> bool {
        // Quick check: list models endpoint
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models?key={}",
            self.api_key
        );
        match self.client.get(&url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }
}

/// Merge consecutive same-role messages (Gemini requirement)
fn merge_consecutive_roles(contents: &mut Vec<Value>) {
    if contents.len() < 2 {
        return;
    }

    let mut merged = Vec::new();
    let mut current = contents[0].clone();

    for next in contents.iter().skip(1) {
        let cur_role = current.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let next_role = next.get("role").and_then(|v| v.as_str()).unwrap_or("");

        if cur_role == next_role {
            if let (Some(cur_parts), Some(next_parts)) = (
                current.get("parts").and_then(|v| v.as_array()).cloned(),
                next.get("parts").and_then(|v| v.as_array()),
            ) {
                let mut all_parts = cur_parts;
                all_parts.extend(next_parts.iter().cloned());
                current["parts"] = json!(all_parts);
            }
        } else {
            merged.push(current);
            current = next.clone();
        }
    }
    merged.push(current);

    *contents = merged;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gemini_provider_name() {
        let p = GeminiProvider::new("test-key".into(), None);
        assert_eq!(p.name(), "gemini");
        assert_eq!(p.default_model(), "gemini-2.5-flash-lite");
    }

    #[test]
    fn test_build_contents_basic() {
        let p = GeminiProvider::new("test-key".into(), None);
        let msgs = vec![
            ChatMessage {
                role: "user".into(),
                content: "Hello".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let (sys, contents) = p.build_contents(&msgs);
        assert!(sys.is_none());
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");
    }

    #[test]
    fn test_build_contents_extracts_system() {
        let p = GeminiProvider::new("test-key".into(), None);
        let msgs = vec![
            ChatMessage {
                role: "system".into(),
                content: "Be helpful".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: "user".into(),
                content: "Hi".into(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        let (sys, contents) = p.build_contents(&msgs);
        assert_eq!(sys.unwrap(), "Be helpful");
        assert_eq!(contents.len(), 1); // Only user message, system extracted
    }

    #[test]
    fn test_build_contents_with_image() {
        let p = GeminiProvider::new("test-key".into(), None);
        let msgs = vec![ChatMessage {
            role: "user".into(),
            content: "Describe this: [IMAGE:base64:abc123]".into(),
            tool_calls: None,
            tool_call_id: None,
        }];
        let (_, contents) = p.build_contents(&msgs);
        let parts = contents[0]["parts"].as_array().unwrap();
        assert!(parts.len() >= 2);
        assert!(parts.iter().any(|p| p.get("inline_data").is_some()));
    }

    #[test]
    fn test_merge_consecutive_roles() {
        let mut contents = vec![
            json!({"role": "user", "parts": [{"text": "a"}]}),
            json!({"role": "user", "parts": [{"text": "b"}]}),
            json!({"role": "model", "parts": [{"text": "c"}]}),
        ];
        merge_consecutive_roles(&mut contents);
        assert_eq!(contents.len(), 2);
    }

    #[test]
    fn test_build_tools_flat_format() {
        let p = GeminiProvider::new("test-key".into(), None);
        let tools = vec![json!({
            "name": "web_search",
            "description": "Search the web",
            "parameters": {"type": "object", "properties": {"query": {"type": "string"}}}
        })];
        let decls = p.build_tools(&tools);
        assert!(decls.is_some());
    }

    #[test]
    fn test_build_tools_openai_format() {
        let p = GeminiProvider::new("test-key".into(), None);
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "shell",
                "description": "Run command",
                "parameters": {"type": "object", "properties": {}}
            }
        })];
        let decls = p.build_tools(&tools);
        assert!(decls.is_some());
    }

    #[test]
    fn test_capabilities() {
        let p = GeminiProvider::new("test-key".into(), None);
        let caps = p.capabilities();
        assert!(caps.vision);
        assert!(caps.streaming);
        assert!(caps.native_tools);
    }
}
