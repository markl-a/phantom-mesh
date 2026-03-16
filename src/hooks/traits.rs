use async_trait::async_trait;
use serde_json::Value;

use crate::providers::ChatMessage;

/// Hook priority — lower runs first
pub type HookPriority = u32;

/// Result of a modifying hook.
/// Allows hooks to pass through, modify, or block the operation.
#[derive(Debug)]
pub enum HookResult<T> {
    /// Pass through unchanged
    Continue(T),
    /// Modified value
    Modified(T),
    /// Block the operation with a reason
    Block(String),
}

impl<T> HookResult<T> {
    pub fn into_inner(self) -> Option<T> {
        match self {
            HookResult::Continue(v) | HookResult::Modified(v) => Some(v),
            HookResult::Block(_) => None,
        }
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, HookResult::Block(_))
    }
}

/// Context passed to hooks with information about the current operation
#[derive(Debug, Clone)]
pub struct HookContext {
    pub agent_name: String,
    pub chat_id: Option<String>,
}

/// Hook for observing/modifying LLM calls
#[async_trait]
pub trait LlmHook: Send + Sync {
    fn name(&self) -> &str;
    fn priority(&self) -> HookPriority { 100 }

    /// Called before sending messages to LLM.
    /// Modifying hook: can alter messages or block the call.
    async fn before_llm_call(
        &self,
        _ctx: &HookContext,
        messages: Vec<ChatMessage>,
        _model: &str,
    ) -> HookResult<Vec<ChatMessage>> {
        HookResult::Continue(messages)
    }

    /// Called after receiving LLM response.
    /// Void hook: observe only (return value ignored).
    async fn on_llm_output(
        &self,
        _ctx: &HookContext,
        _response: &ChatMessage,
    ) {}
}

/// Hook for observing/modifying tool calls
#[async_trait]
pub trait ToolHook: Send + Sync {
    fn name(&self) -> &str;
    fn priority(&self) -> HookPriority { 100 }

    /// Called before executing a tool.
    /// Modifying hook: can alter arguments or block the call.
    async fn before_tool_call(
        &self,
        ctx: &HookContext,
        tool_name: &str,
        arguments: Value,
    ) -> HookResult<Value> {
        let _ = (ctx, tool_name);
        HookResult::Continue(arguments)
    }

    /// Called after tool execution.
    /// Void hook: observe only.
    async fn on_after_tool_call(
        &self,
        _ctx: &HookContext,
        _tool_name: &str,
        _arguments: &Value,
        _result: &str,
        _success: bool,
    ) {}
}

/// Hook for observing/modifying incoming messages
#[async_trait]
pub trait MessageHook: Send + Sync {
    fn name(&self) -> &str;
    fn priority(&self) -> HookPriority { 100 }

    /// Called when a message is received from a channel.
    /// Modifying hook: can alter the message text or block it.
    async fn on_message_received(
        &self,
        _ctx: &HookContext,
        text: String,
    ) -> HookResult<String> {
        HookResult::Continue(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hook_result_continue() {
        let r: HookResult<String> = HookResult::Continue("hello".into());
        assert!(!r.is_blocked());
        assert_eq!(r.into_inner().unwrap(), "hello");
    }

    #[test]
    fn test_hook_result_modified() {
        let r: HookResult<String> = HookResult::Modified("modified".into());
        assert!(!r.is_blocked());
        assert_eq!(r.into_inner().unwrap(), "modified");
    }

    #[test]
    fn test_hook_result_block() {
        let r: HookResult<String> = HookResult::Block("denied".into());
        assert!(r.is_blocked());
        assert!(r.into_inner().is_none());
    }
}
