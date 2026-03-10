use std::sync::Arc;
use super::traits::{ChatMessage, Provider};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestComplexity {
    Simple,
    Medium,
    Complex,
}

impl RequestComplexity {
    pub fn from_response(text: &str) -> Self {
        let text = text.trim().to_uppercase();
        if text.contains("SIMPLE") { Self::Simple }
        else if text.contains("COMPLEX") { Self::Complex }
        else { Self::Medium }
    }
}

const CLASSIFIER_PROMPT: &str = r#"Classify this request as SIMPLE, MEDIUM, or COMPLEX.

SIMPLE: greetings, yes/no, single-fact lookups, short translations, acknowledgments
MEDIUM: summarization, general Q&A, multi-sentence replies, explanations, basic coding questions
COMPLEX: code generation, debugging, multi-step reasoning, analysis, planning, tool-heavy tasks

Request: {INPUT}

Reply with one word only: SIMPLE, MEDIUM, or COMPLEX"#;

pub struct RequestClassifier {
    provider: Arc<dyn Provider>,
    model: String,
}

impl RequestClassifier {
    pub fn new(provider: Arc<dyn Provider>, model: String) -> Self {
        Self { provider, model }
    }

    pub async fn classify(&self, messages: &[ChatMessage]) -> RequestComplexity {
        let last_user = messages.iter().rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");
        if last_user.is_empty() { return RequestComplexity::Medium; }

        // Quick heuristic: very short messages are likely simple
        let word_count = last_user.split_whitespace().count();
        if word_count <= 3 { return RequestComplexity::Simple; }

        let prompt = CLASSIFIER_PROMPT.replace("{INPUT}", last_user);
        let classify_messages = vec![ChatMessage {
            role: "user".to_string(), content: prompt, tool_calls: None, tool_call_id: None,
        }];
        match self.provider.chat(&classify_messages, &[], &self.model).await {
            Ok(resp) => RequestComplexity::from_response(&resp.message.content),
            Err(e) => {
                tracing::warn!("Classifier failed, defaulting to Medium: {}", e);
                RequestComplexity::Medium
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_response_simple() {
        assert_eq!(RequestComplexity::from_response("SIMPLE"), RequestComplexity::Simple);
        assert_eq!(RequestComplexity::from_response("  simple  "), RequestComplexity::Simple);
        assert_eq!(RequestComplexity::from_response("I think this is SIMPLE."), RequestComplexity::Simple);
    }

    #[test]
    fn test_from_response_complex() {
        assert_eq!(RequestComplexity::from_response("COMPLEX"), RequestComplexity::Complex);
        assert_eq!(RequestComplexity::from_response("This is complex"), RequestComplexity::Complex);
    }

    #[test]
    fn test_from_response_medium() {
        assert_eq!(RequestComplexity::from_response("MEDIUM"), RequestComplexity::Medium);
        assert_eq!(RequestComplexity::from_response("unknown"), RequestComplexity::Medium);
        assert_eq!(RequestComplexity::from_response(""), RequestComplexity::Medium);
    }

    #[test]
    fn test_short_message_is_simple() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            use super::super::mock::MockProvider;
            let mock = Arc::new(MockProvider::fixed("MEDIUM"));
            let classifier = RequestClassifier::new(mock, "test".to_string());
            let messages = vec![ChatMessage { role: "user".into(), content: "Hi".into(), tool_calls: None, tool_call_id: None }];
            let result = classifier.classify(&messages).await;
            assert_eq!(result, RequestComplexity::Simple); // Short = simple heuristic
        });
    }

    #[test]
    fn test_classify_with_mock_provider() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            use super::super::mock::MockProvider;
            let mock = Arc::new(MockProvider::fixed("COMPLEX"));
            let classifier = RequestClassifier::new(mock, "test".to_string());
            let messages = vec![ChatMessage {
                role: "user".into(),
                content: "Write a Rust function that implements a binary search tree with insert delete and rebalance operations".into(),
                tool_calls: None, tool_call_id: None,
            }];
            let result = classifier.classify(&messages).await;
            assert_eq!(result, RequestComplexity::Complex);
        });
    }
}
