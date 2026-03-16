use anyhow::{anyhow, Result};
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::providers::ChatMessage;
use super::traits::*;

/// Runs hooks in the appropriate order.
/// - Void hooks run in parallel (fire-and-forget)
/// - Modifying hooks run sequentially by priority (lowest first)
pub struct HookRunner {
    llm_hooks: Vec<Box<dyn LlmHook>>,
    tool_hooks: Vec<Box<dyn ToolHook>>,
    message_hooks: Vec<Box<dyn MessageHook>>,
}

impl HookRunner {
    pub fn new() -> Self {
        Self {
            llm_hooks: Vec::new(),
            tool_hooks: Vec::new(),
            message_hooks: Vec::new(),
        }
    }

    pub fn add_llm_hook(&mut self, hook: Box<dyn LlmHook>) {
        info!("Registered LLM hook: {}", hook.name());
        self.llm_hooks.push(hook);
        self.llm_hooks.sort_by_key(|h| h.priority());
    }

    pub fn add_tool_hook(&mut self, hook: Box<dyn ToolHook>) {
        info!("Registered tool hook: {}", hook.name());
        self.tool_hooks.push(hook);
        self.tool_hooks.sort_by_key(|h| h.priority());
    }

    pub fn add_message_hook(&mut self, hook: Box<dyn MessageHook>) {
        info!("Registered message hook: {}", hook.name());
        self.message_hooks.push(hook);
        self.message_hooks.sort_by_key(|h| h.priority());
    }

    /// Run before_llm_call hooks (modifying, sequential by priority).
    /// Returns modified messages or Err if blocked.
    pub async fn run_before_llm(
        &self,
        ctx: &HookContext,
        mut messages: Vec<ChatMessage>,
        model: &str,
    ) -> Result<Vec<ChatMessage>> {
        for hook in &self.llm_hooks {
            match hook.before_llm_call(ctx, messages, model).await {
                HookResult::Continue(msgs) => {
                    messages = msgs;
                }
                HookResult::Modified(msgs) => {
                    debug!("Hook '{}' modified LLM messages", hook.name());
                    messages = msgs;
                }
                HookResult::Block(reason) => {
                    warn!("Hook '{}' blocked LLM call: {}", hook.name(), reason);
                    return Err(anyhow!("LLM call blocked by hook '{}': {}", hook.name(), reason));
                }
            }
        }
        Ok(messages)
    }

    /// Run on_llm_output hooks (void, parallel).
    pub async fn run_on_llm_output(
        &self,
        ctx: &HookContext,
        response: &ChatMessage,
    ) {
        // Run void hooks in parallel via join
        let futs: Vec<_> = self.llm_hooks.iter()
            .map(|hook| hook.on_llm_output(ctx, response))
            .collect();
        futures_util::future::join_all(futs).await;
    }

    /// Run before_tool_call hooks (modifying, sequential by priority).
    /// Returns modified arguments or Err if blocked.
    pub async fn run_before_tool(
        &self,
        ctx: &HookContext,
        tool_name: &str,
        mut arguments: Value,
    ) -> Result<Value> {
        for hook in &self.tool_hooks {
            match hook.before_tool_call(ctx, tool_name, arguments).await {
                HookResult::Continue(args) => {
                    arguments = args;
                }
                HookResult::Modified(args) => {
                    debug!("Hook '{}' modified tool '{}' arguments", hook.name(), tool_name);
                    arguments = args;
                }
                HookResult::Block(reason) => {
                    warn!("Hook '{}' blocked tool '{}': {}", hook.name(), tool_name, reason);
                    return Err(anyhow!("Tool '{}' blocked by hook '{}': {}", tool_name, hook.name(), reason));
                }
            }
        }
        Ok(arguments)
    }

    /// Run on_after_tool_call hooks (void, parallel).
    pub async fn run_after_tool(
        &self,
        ctx: &HookContext,
        tool_name: &str,
        arguments: &Value,
        result: &str,
        success: bool,
    ) {
        let futs: Vec<_> = self.tool_hooks.iter()
            .map(|hook| hook.on_after_tool_call(ctx, tool_name, arguments, result, success))
            .collect();
        futures_util::future::join_all(futs).await;
    }

    /// Run on_message_received hooks (modifying, sequential by priority).
    /// Returns modified text or Err if blocked.
    pub async fn run_on_message(
        &self,
        ctx: &HookContext,
        mut text: String,
    ) -> Result<String> {
        for hook in &self.message_hooks {
            match hook.on_message_received(ctx, text).await {
                HookResult::Continue(t) => {
                    text = t;
                }
                HookResult::Modified(t) => {
                    debug!("Hook '{}' modified incoming message", hook.name());
                    text = t;
                }
                HookResult::Block(reason) => {
                    warn!("Hook '{}' blocked message: {}", hook.name(), reason);
                    return Err(anyhow!("Message blocked by hook '{}': {}", hook.name(), reason));
                }
            }
        }
        Ok(text)
    }

    /// Get hook counts for status display
    pub fn hook_counts(&self) -> (usize, usize, usize) {
        (self.llm_hooks.len(), self.tool_hooks.len(), self.message_hooks.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hook_runner_empty() {
        let runner = HookRunner::new();
        let (llm, tool, msg) = runner.hook_counts();
        assert_eq!(llm, 0);
        assert_eq!(tool, 0);
        assert_eq!(msg, 0);
    }

    struct TestToolHook {
        name: String,
        priority: HookPriority,
    }

    #[async_trait::async_trait]
    impl ToolHook for TestToolHook {
        fn name(&self) -> &str { &self.name }
        fn priority(&self) -> HookPriority { self.priority }
    }

    #[test]
    fn test_hook_runner_add_tool_hook() {
        let mut runner = HookRunner::new();
        runner.add_tool_hook(Box::new(TestToolHook { name: "test".into(), priority: 50 }));
        let (_, tool, _) = runner.hook_counts();
        assert_eq!(tool, 1);
    }

    #[tokio::test]
    async fn test_run_before_tool_passthrough() {
        let runner = HookRunner::new();
        let ctx = HookContext { agent_name: "test".into(), chat_id: None };
        let result = runner.run_before_tool(&ctx, "shell", serde_json::json!({"cmd": "ls"})).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), serde_json::json!({"cmd": "ls"}));
    }

    #[tokio::test]
    async fn test_run_before_llm_passthrough() {
        let runner = HookRunner::new();
        let ctx = HookContext { agent_name: "test".into(), chat_id: None };
        let msgs = vec![ChatMessage {
            role: "user".into(), content: "hi".into(), tool_calls: None, tool_call_id: None,
        }];
        let result = runner.run_before_llm(&ctx, msgs, "model").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_run_on_message_passthrough() {
        let runner = HookRunner::new();
        let ctx = HookContext { agent_name: "test".into(), chat_id: None };
        let result = runner.run_on_message(&ctx, "hello".to_string()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "hello");
    }
}
