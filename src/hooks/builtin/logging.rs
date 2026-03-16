use async_trait::async_trait;
use serde_json::Value;
use tracing::info;

use crate::providers::ChatMessage;
use crate::hooks::traits::*;

/// Built-in logging hook that records all tool calls and LLM interactions.
pub struct LoggingHook;

#[async_trait]
impl ToolHook for LoggingHook {
    fn name(&self) -> &str {
        "builtin:logging"
    }

    fn priority(&self) -> HookPriority {
        // Run last among hooks (highest priority number)
        200
    }

    async fn on_after_tool_call(
        &self,
        ctx: &HookContext,
        tool_name: &str,
        arguments: &Value,
        result: &str,
        success: bool,
    ) {
        let args_str = serde_json::to_string(arguments).unwrap_or_default();
        let args_preview = if args_str.len() > 100 { &args_str[..100] } else { &args_str };
        let result_preview = if result.len() > 200 { &result[..200] } else { result };
        info!(
            "[hook:logging] agent={} tool={} args={} success={} result={}...",
            ctx.agent_name, tool_name, args_preview, success, result_preview
        );
    }
}

#[async_trait]
impl LlmHook for LoggingHook {
    fn name(&self) -> &str {
        "builtin:logging"
    }

    fn priority(&self) -> HookPriority {
        200
    }

    async fn on_llm_output(
        &self,
        ctx: &HookContext,
        response: &ChatMessage,
    ) {
        let content_preview = if response.content.len() > 200 {
            &response.content[..200]
        } else {
            &response.content
        };
        let has_tools = response.tool_calls.as_ref().map(|tc| tc.len()).unwrap_or(0);
        info!(
            "[hook:logging] agent={} llm_output: {}... (tool_calls={})",
            ctx.agent_name, content_preview, has_tools
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logging_hook_name() {
        let hook = LoggingHook;
        assert_eq!(ToolHook::name(&hook), "builtin:logging");
        assert_eq!(LlmHook::name(&hook), "builtin:logging");
    }

    #[test]
    fn test_logging_hook_priority() {
        let hook = LoggingHook;
        assert_eq!(ToolHook::priority(&hook), 200);
        assert_eq!(LlmHook::priority(&hook), 200);
    }
}
