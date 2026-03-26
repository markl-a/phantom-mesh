//! Mock LLM Provider — 用於 CI 環境和測試
//!
//! 提供確定性的 LLM 回應，不需要任何外部服務。
//! 支援多種模式：
//! - Echo: 回傳使用者最後的訊息
//! - Scripted: 按照預定義腳本依序回應
//! - ToolCall: 模擬 tool call 回應
//! - Error: 模擬錯誤場景

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::traits::*;

/// Mock 回應模式
#[derive(Debug, Clone)]
pub enum MockMode {
    /// 回傳最後一條使用者訊息（加上前綴）
    Echo,
    /// 按照腳本依序回應（循環）
    Scripted(Vec<MockResponse>),
    /// 固定回傳同一段文字
    Fixed(String),
    /// 模擬錯誤
    Error(String),
}

/// 單一腳本回應
#[derive(Debug, Clone)]
pub enum MockResponse {
    /// 純文字回應
    Text(String),
    /// 帶 tool call 的回應
    ToolCalls {
        content: String,
        calls: Vec<MockToolCall>,
    },
    /// 錯誤回應
    Error(String),
}

/// Mock tool call 定義
#[derive(Debug, Clone)]
pub struct MockToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// Mock LLM Provider
pub struct MockProvider {
    mode: MockMode,
    model: String,
    /// 腳本模式的當前索引
    script_index: Arc<AtomicUsize>,
    /// 記錄所有收到的請求（用於斷言）
    pub call_log: Arc<Mutex<Vec<MockCallRecord>>>,
    /// 人為延遲（模擬推理時間）
    pub latency_ms: u64,
}

impl Clone for MockProvider {
    fn clone(&self) -> Self {
        Self {
            mode: self.mode.clone(),
            model: self.model.clone(),
            script_index: self.script_index.clone(),
            call_log: self.call_log.clone(), // shares the same Arc
            latency_ms: self.latency_ms,
        }
    }
}

/// 記錄一次 chat 呼叫的參數
#[derive(Debug, Clone)]
pub struct MockCallRecord {
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<Value>,
    pub model: String,
}

impl MockProvider {
    /// 建立 Echo 模式的 mock provider
    pub fn echo() -> Self {
        Self {
            mode: MockMode::Echo,
            model: "mock-echo".to_string(),
            script_index: Arc::new(AtomicUsize::new(0)),
            call_log: Arc::new(Mutex::new(Vec::new())),
            latency_ms: 0,
        }
    }

    /// 建立固定回應模式的 mock provider
    pub fn fixed(response: impl Into<String>) -> Self {
        Self {
            mode: MockMode::Fixed(response.into()),
            model: "mock-fixed".to_string(),
            script_index: Arc::new(AtomicUsize::new(0)),
            call_log: Arc::new(Mutex::new(Vec::new())),
            latency_ms: 0,
        }
    }

    /// 建立腳本模式的 mock provider
    pub fn scripted(responses: Vec<MockResponse>) -> Self {
        Self {
            mode: MockMode::Scripted(responses),
            model: "mock-scripted".to_string(),
            script_index: Arc::new(AtomicUsize::new(0)),
            call_log: Arc::new(Mutex::new(Vec::new())),
            latency_ms: 0,
        }
    }

    /// 建立錯誤模式的 mock provider
    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            mode: MockMode::Error(msg.into()),
            model: "mock-error".to_string(),
            script_index: Arc::new(AtomicUsize::new(0)),
            call_log: Arc::new(Mutex::new(Vec::new())),
            latency_ms: 0,
        }
    }

    /// 設定模擬延遲
    pub fn with_latency(mut self, ms: u64) -> Self {
        self.latency_ms = ms;
        self
    }

    /// 設定模型名稱
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// 取得呼叫記錄
    pub fn call_count(&self) -> usize {
        self.call_log.lock().unwrap().len()
    }

    /// 取得第 n 次呼叫的記錄
    pub fn get_call(&self, index: usize) -> Option<MockCallRecord> {
        self.call_log.lock().unwrap().get(index).cloned()
    }

    /// 從腳本取得下一個回應
    fn next_scripted_response(&self, scripts: &[MockResponse]) -> MockResponse {
        let idx = self.script_index.fetch_add(1, Ordering::SeqCst);
        scripts[idx % scripts.len()].clone()
    }

    /// 將 MockToolCall 轉為 ToolCall
    fn mock_to_tool_call(mock: &MockToolCall) -> ToolCall {
        ToolCall {
            id: Some(mock.id.clone()),
            function: ToolCallFunction {
                name: mock.name.clone(),
                arguments: mock.arguments.clone(),
            },
        }
    }

    /// 記錄呼叫
    fn record_call(&self, messages: &[ChatMessage], tools: &[Value], model: &str) {
        if let Ok(mut log) = self.call_log.lock() {
            log.push(MockCallRecord {
                messages: messages.to_vec(),
                tools: tools.to_vec(),
                model: model.to_string(),
            });
        }
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn default_model(&self) -> &str {
        &self.model
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            native_tools: true,
            vision: false,
        }
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<ChatResponse> {
        // 記錄呼叫
        self.record_call(messages, tools, model);

        // 模擬延遲
        if self.latency_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.latency_ms)).await;
        }

        match &self.mode {
            MockMode::Echo => {
                // 找到最後一條 user 訊息
                let last_user = messages
                    .iter()
                    .rev()
                    .find(|m| m.role == "user")
                    .map(|m| m.content.clone())
                    .unwrap_or_else(|| "(no user message)".to_string());

                Ok(ChatResponse {
                    message: ChatMessage {
                        role: "assistant".to_string(),
                        content: format!("[mock-echo] {}", last_user),
                        tool_calls: None,
                        tool_call_id: None,
                    },
                    usage: Some(TokenUsage {
                        prompt_tokens: messages.iter().map(|m| m.content.len() as u32 / 4).sum(),
                        completion_tokens: last_user.len() as u32 / 4 + 3,
                        total_tokens: 0, // 會在下面計算
                    }),
                })
            }

            MockMode::Fixed(text) => Ok(ChatResponse {
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content: text.clone(),
                    tool_calls: None,
                    tool_call_id: None,
                },
                usage: Some(TokenUsage {
                    prompt_tokens: 10,
                    completion_tokens: text.len() as u32 / 4,
                    total_tokens: 10 + text.len() as u32 / 4,
                }),
            }),

            MockMode::Scripted(scripts) => {
                let response = self.next_scripted_response(scripts);
                match response {
                    MockResponse::Text(text) => Ok(ChatResponse {
                        message: ChatMessage {
                            role: "assistant".to_string(),
                            content: text.clone(),
                            tool_calls: None,
                            tool_call_id: None,
                        },
                        usage: Some(TokenUsage {
                            prompt_tokens: 10,
                            completion_tokens: text.len() as u32 / 4,
                            total_tokens: 10 + text.len() as u32 / 4,
                        }),
                    }),
                    MockResponse::ToolCalls { content, calls } => {
                        let tool_calls: Vec<ToolCall> =
                            calls.iter().map(Self::mock_to_tool_call).collect();
                        Ok(ChatResponse {
                            message: ChatMessage {
                                role: "assistant".to_string(),
                                content,
                                tool_calls: Some(tool_calls),
                                tool_call_id: None,
                            },
                            usage: Some(TokenUsage {
                                prompt_tokens: 10,
                                completion_tokens: 20,
                                total_tokens: 30,
                            }),
                        })
                    }
                    MockResponse::Error(msg) => Err(anyhow!("Mock error: {}", msg)),
                }
            }

            MockMode::Error(msg) => Err(anyhow!("Mock error: {}", msg)),
        }
    }

    async fn stream_chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
    ) -> Result<Pin<Box<dyn futures_util::Stream<Item = Result<StreamChunk>> + Send>>> {
        // Mock streaming: 先拿到完整回應，再切成 chunks
        let resp = self.chat(messages, tools, model).await?;
        let text = resp.message.content;
        let usage = resp.usage;

        // 把文字切成每 20 字元一個 chunk
        let chunks: Vec<String> = text
            .chars()
            .collect::<Vec<_>>()
            .chunks(20)
            .map(|c| c.iter().collect::<String>())
            .collect();

        let mut stream_items: Vec<Result<StreamChunk>> = chunks
            .into_iter()
            .map(|c| Ok(StreamChunk::ContentDelta(c)))
            .collect();
        stream_items.push(Ok(StreamChunk::Done { usage }));

        Ok(Box::pin(futures_util::stream::iter(stream_items)))
    }

    async fn is_alive(&self) -> bool {
        !matches!(self.mode, MockMode::Error(_))
    }
}

// ── 便利建構函式 ────────────────────────────────────────────────────────────

/// 建立一個模擬 tool call 後回傳文字的腳本
pub fn tool_then_text_script(
    tool_name: &str,
    tool_args: Value,
    final_text: &str,
) -> Vec<MockResponse> {
    vec![
        MockResponse::ToolCalls {
            content: String::new(),
            calls: vec![MockToolCall {
                id: "mock_call_1".to_string(),
                name: tool_name.to_string(),
                arguments: tool_args,
            }],
        },
        MockResponse::Text(final_text.to_string()),
    ]
}

/// 建立一個多輪 tool call 腳本
pub fn multi_tool_script(steps: Vec<(&str, Value)>, final_text: &str) -> Vec<MockResponse> {
    let mut responses: Vec<MockResponse> = steps
        .into_iter()
        .enumerate()
        .map(|(i, (name, args))| MockResponse::ToolCalls {
            content: String::new(),
            calls: vec![MockToolCall {
                id: format!("mock_call_{}", i + 1),
                name: name.to_string(),
                arguments: args,
            }],
        })
        .collect();
    responses.push(MockResponse::Text(final_text.to_string()));
    responses
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_mock_echo() {
        let provider = MockProvider::echo();
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "Hello world".to_string(),
            tool_calls: None,
            tool_call_id: None,
        }];

        let resp = provider.chat(&messages, &[], "").await.unwrap();
        assert!(resp.message.content.contains("Hello world"));
        assert_eq!(resp.message.role, "assistant");
        assert_eq!(provider.call_count(), 1);
    }

    #[tokio::test]
    async fn test_mock_fixed() {
        let provider = MockProvider::fixed("I am a fixed response");
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "anything".to_string(),
            tool_calls: None,
            tool_call_id: None,
        }];

        let resp = provider.chat(&messages, &[], "").await.unwrap();
        assert_eq!(resp.message.content, "I am a fixed response");
    }

    #[tokio::test]
    async fn test_mock_scripted() {
        let provider = MockProvider::scripted(vec![
            MockResponse::Text("First response".to_string()),
            MockResponse::Text("Second response".to_string()),
        ]);
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
            tool_calls: None,
            tool_call_id: None,
        }];

        let r1 = provider.chat(&messages, &[], "").await.unwrap();
        assert_eq!(r1.message.content, "First response");

        let r2 = provider.chat(&messages, &[], "").await.unwrap();
        assert_eq!(r2.message.content, "Second response");

        // 循環
        let r3 = provider.chat(&messages, &[], "").await.unwrap();
        assert_eq!(r3.message.content, "First response");
    }

    #[tokio::test]
    async fn test_mock_tool_calls() {
        let provider = MockProvider::scripted(tool_then_text_script(
            "shell",
            json!({"command": "ls"}),
            "Done listing files",
        ));
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "list files".to_string(),
            tool_calls: None,
            tool_call_id: None,
        }];

        let r1 = provider.chat(&messages, &[], "").await.unwrap();
        assert!(r1.message.tool_calls.is_some());
        let tc = &r1.message.tool_calls.unwrap()[0];
        assert_eq!(tc.function.name, "shell");

        let r2 = provider.chat(&messages, &[], "").await.unwrap();
        assert_eq!(r2.message.content, "Done listing files");
        assert!(r2.message.tool_calls.is_none());
    }

    #[tokio::test]
    async fn test_mock_error() {
        let provider = MockProvider::error("connection refused");
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
            tool_calls: None,
            tool_call_id: None,
        }];

        let result = provider.chat(&messages, &[], "").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("connection refused"));
    }

    #[tokio::test]
    async fn test_mock_is_alive() {
        assert!(MockProvider::echo().is_alive().await);
        assert!(MockProvider::fixed("x").is_alive().await);
        assert!(!MockProvider::error("dead").is_alive().await);
    }

    #[tokio::test]
    async fn test_mock_call_log() {
        let provider = MockProvider::echo();
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "test".to_string(),
            tool_calls: None,
            tool_call_id: None,
        }];

        let _ = provider.chat(&messages, &[], "gpt-test").await;
        let record = provider.get_call(0).unwrap();
        assert_eq!(record.model, "gpt-test");
        assert_eq!(record.messages.len(), 1);
    }

    #[tokio::test]
    async fn test_mock_streaming() {
        use futures_util::StreamExt;

        let provider = MockProvider::fixed("Hello streaming world!");
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
            tool_calls: None,
            tool_call_id: None,
        }];

        let mut stream = provider.stream_chat(&messages, &[], "").await.unwrap();
        let mut full_text = String::new();
        while let Some(chunk) = stream.next().await {
            match chunk.unwrap() {
                StreamChunk::ContentDelta(s) => full_text.push_str(&s),
                StreamChunk::Done { .. } => break,
                _ => {}
            }
        }
        assert_eq!(full_text, "Hello streaming world!");
    }

    #[tokio::test]
    async fn test_multi_tool_script() {
        let provider = MockProvider::scripted(multi_tool_script(
            vec![
                ("web_search", json!({"query": "rust ci"})),
                ("file_write", json!({"path": "out.md", "content": "# Result"})),
            ],
            "All done!",
        ));
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "do stuff".to_string(),
            tool_calls: None,
            tool_call_id: None,
        }];

        // Round 1: web_search
        let r1 = provider.chat(&messages, &[], "").await.unwrap();
        let tc1 = &r1.message.tool_calls.as_ref().unwrap()[0];
        assert_eq!(tc1.function.name, "web_search");

        // Round 2: file_write
        let r2 = provider.chat(&messages, &[], "").await.unwrap();
        let tc2 = &r2.message.tool_calls.as_ref().unwrap()[0];
        assert_eq!(tc2.function.name, "file_write");

        // Round 3: final text
        let r3 = provider.chat(&messages, &[], "").await.unwrap();
        assert_eq!(r3.message.content, "All done!");
        assert!(r3.message.tool_calls.is_none());
    }

    #[tokio::test]
    async fn test_mock_usage_tracking() {
        let provider = MockProvider::fixed("short");
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
            tool_calls: None,
            tool_call_id: None,
        }];

        let resp = provider.chat(&messages, &[], "").await.unwrap();
        let usage = resp.usage.unwrap();
        assert!(usage.prompt_tokens > 0 || usage.completion_tokens > 0);
        assert!(usage.total_tokens > 0);
    }

    #[test]
    fn test_cloned_mock_shares_call_log() {
        let mock1 = MockProvider::fixed("hello");
        let mock2 = mock1.clone();
        // Both should share the same call_log
        mock1.call_log.lock().unwrap().push(MockCallRecord {
            messages: vec![],
            tools: vec![],
            model: "test".to_string(),
        });
        assert_eq!(mock2.call_count(), 1);
    }
}
