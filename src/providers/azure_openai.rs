use anyhow::{anyhow, Result};
use reqwest::Client;
use serde_json::Value;

use super::{ChatMessage, ChatResponse, ToolCall, ToolCallFunction, TokenUsage};

pub fn build_azure_url(endpoint: &str, model: &str, api_version: &str) -> String {
    let base = endpoint.trim_end_matches('/');
    format!(
        "{}/openai/deployments/{}/chat/completions?api-version={}",
        base, model, api_version
    )
}

pub struct AzureOpenAiProvider {
    endpoint: String,
    api_key: String,
    api_version: String,
    default_model: String,
    client: Client,
}

impl AzureOpenAiProvider {
    pub fn new(
        endpoint: String,
        api_key: String,
        api_version: String,
        default_model: String,
    ) -> Self {
        Self {
            endpoint,
            api_key,
            api_version,
            default_model,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .unwrap_or_default(),
        }
    }

    fn resolve_model<'a>(&'a self, model: &'a str) -> &'a str {
        if model.is_empty() {
            &self.default_model
        } else {
            model
        }
    }

    pub async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<ChatResponse> {
        let model = self.resolve_model(model);
        let url = build_azure_url(&self.endpoint, model, &self.api_version);

        let mut body = serde_json::json!({
            "messages": messages,
            "max_tokens": 4096,
        });
        // Note: model is NOT in the body for Azure — it's in the URL path
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools.to_vec());
        }

        let resp = self
            .client
            .post(&url)
            .header("api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Azure OpenAI request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Azure OpenAI HTTP {}: {}", status, text));
        }

        let json: Value = resp.json().await
            .map_err(|e| anyhow!("Azure OpenAI parse error: {}", e))?;

        if let Some(err) = json.get("error") {
            return Err(anyhow!("Azure OpenAI error: {}", err));
        }

        let msg = json.pointer("/choices/0/message")
            .ok_or_else(|| anyhow!("Azure response missing choices[0].message"))?;

        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("assistant").to_string();
        let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let tool_calls = parse_tool_calls(msg);
        let usage = parse_usage(&json);

        Ok(ChatResponse {
            message: ChatMessage { role, content, tool_calls, tool_call_id: None },
            usage,
        })
    }

    pub fn name(&self) -> &str {
        "azure_openai"
    }

    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    pub async fn is_alive(&self) -> bool {
        let url = format!(
            "{}/openai/deployments?api-version={}",
            self.endpoint.trim_end_matches('/'),
            self.api_version
        );
        self.client
            .get(&url)
            .header("api-key", &self.api_key)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

fn parse_tool_calls(msg: &Value) -> Option<Vec<ToolCall>> {
    let arr = msg.get("tool_calls")?.as_array()?;
    let calls: Vec<ToolCall> = arr
        .iter()
        .filter_map(|tc| {
            let id = tc.get("id").and_then(|v| v.as_str()).map(String::from);
            let func = tc.get("function")?;
            let name = func.get("name")?.as_str()?.to_string();
            let arguments = func.get("arguments").cloned().unwrap_or(Value::Null);
            Some(ToolCall {
                id,
                function: ToolCallFunction { name, arguments },
            })
        })
        .collect();
    if calls.is_empty() { None } else { Some(calls) }
}

fn parse_usage(json: &Value) -> Option<TokenUsage> {
    let u = json.get("usage")?;
    let prompt = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let completion = u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    Some(TokenUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt + completion,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn azure_url_construction() {
        let url = build_azure_url(
            "https://mydeployment.openai.azure.com",
            "gpt-4o",
            "2024-02-01",
        );
        assert_eq!(
            url,
            "https://mydeployment.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version=2024-02-01"
        );
    }

    #[test]
    fn azure_url_strips_trailing_slash() {
        let url = build_azure_url(
            "https://mydeployment.openai.azure.com/",
            "gpt-4o",
            "2024-02-01",
        );
        assert_eq!(
            url,
            "https://mydeployment.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version=2024-02-01"
        );
    }

    #[test]
    fn azure_provider_name() {
        let provider = AzureOpenAiProvider::new(
            "https://test.openai.azure.com".to_string(),
            "test-key".to_string(),
            "2024-02-01".to_string(),
            "gpt-4o".to_string(),
        );
        assert_eq!(provider.name(), "azure_openai");
    }

    #[test]
    fn azure_default_model() {
        let provider = AzureOpenAiProvider::new(
            "https://test.openai.azure.com".to_string(),
            "test-key".to_string(),
            "2024-02-01".to_string(),
            "gpt-4o".to_string(),
        );
        assert_eq!(provider.default_model(), "gpt-4o");
    }
}
