use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::pin::Pin;

use super::openai_compat::OpenAiCompatProvider;
use super::traits::*;

/// Official OpenAI API provider.
/// Thin wrapper over OpenAiCompatProvider with OpenAI-specific defaults.
pub struct OpenAiProvider {
    inner: OpenAiCompatProvider,
}

impl OpenAiProvider {
    pub fn new(api_key: String, default_model: String) -> Self {
        Self {
            inner: OpenAiCompatProvider::new(
                "openai".to_string(),
                "https://api.openai.com".to_string(),
                default_model,
                Some(api_key),
            ),
        }
    }

    pub fn with_base_url(api_key: String, default_model: String, base_url: String) -> Self {
        Self {
            inner: OpenAiCompatProvider::new(
                "openai".to_string(),
                base_url,
                default_model,
                Some(api_key),
            ),
        }
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn default_model(&self) -> &str {
        self.inner.default_model()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            native_tools: true,
            vision: true, // GPT-4o supports vision
        }
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<ChatResponse> {
        self.inner.chat(messages, tools, model).await
    }

    async fn stream_chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<Pin<Box<dyn futures_util::Stream<Item = Result<StreamChunk>> + Send>>> {
        self.inner.stream_chat(messages, tools, model).await
    }

    async fn is_alive(&self) -> bool {
        self.inner.is_alive().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_provider_name() {
        let p = OpenAiProvider::new("sk-test".into(), "gpt-4o".into());
        assert_eq!(p.name(), "openai");
        assert_eq!(p.default_model(), "gpt-4o");
    }

    #[test]
    fn test_openai_capabilities() {
        let p = OpenAiProvider::new("sk-test".into(), "gpt-4o".into());
        let caps = p.capabilities();
        assert!(caps.streaming);
        assert!(caps.native_tools);
        assert!(caps.vision);
    }

    #[test]
    fn test_openai_with_base_url() {
        let p = OpenAiProvider::with_base_url(
            "sk-test".into(),
            "gpt-4o".into(),
            "https://custom-proxy.example.com".into(),
        );
        assert_eq!(p.name(), "openai");
    }
}
