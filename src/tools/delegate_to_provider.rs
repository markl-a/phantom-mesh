// delegate_to_provider tool — route a subtask directly to a specific LLM provider
// e.g. delegate_to_provider(provider="lmstudio", prompt="analyze this data", model="deepseek-r1-distill-llama-70b")
//
// Unlike `delegate` (which targets a named agent), this tool constructs an ephemeral
// agent config at runtime, enabling dynamic provider selection for multi-agent coordination.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::info;

use super::{Tool, ToolResult, ToolRegistry};
use crate::agent_runtime::{AgentConfig, AgentRuntime};
use crate::llm_router::LlmRouter;

pub struct DelegateToProviderTool {
    agent_runtime: Arc<AgentRuntime>,
    llm_router: Arc<LlmRouter>,
    tool_registry: Arc<ToolRegistry>,
}

impl DelegateToProviderTool {
    pub fn new(
        agent_runtime: Arc<AgentRuntime>,
        llm_router: Arc<LlmRouter>,
        tool_registry: Arc<ToolRegistry>,
    ) -> Self {
        Self {
            agent_runtime,
            llm_router,
            tool_registry,
        }
    }
}

#[async_trait]
impl Tool for DelegateToProviderTool {
    fn name(&self) -> &str { "delegate_to_provider" }

    fn description(&self) -> &str {
        "Route a subtask to a specific LLM provider. Use this to leverage different models for different tasks \
         (e.g. a reasoning model for analysis, a code model for programming, a fast model for simple queries). \
         Available providers can be checked with the /status command."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "provider": {
                    "type": "string",
                    "description": "Provider name to route to (e.g. 'lmstudio', 'ollama', 'gemini', 'groq', 'anthropic', 'openai'). Use 'auto' for automatic selection."
                },
                "prompt": {
                    "type": "string",
                    "description": "The task/prompt to send to the provider"
                },
                "model": {
                    "type": "string",
                    "description": "Optional model override (e.g. 'deepseek-r1-distill-llama-70b'). If omitted, uses the provider's default model."
                },
                "system_prompt": {
                    "type": "string",
                    "description": "Optional system prompt to set the role/context for this subtask. If omitted, uses a generic assistant prompt."
                },
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of tool names the sub-agent can use (e.g. ['web_search', 'file_write']). If omitted, no tools are provided (pure LLM completion)."
                }
            },
            "required": ["provider", "prompt"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let provider = args.get("provider").and_then(|v| v.as_str()).unwrap_or("");
        let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
        let model = args.get("model").and_then(|v| v.as_str());
        let system_prompt = args.get("system_prompt").and_then(|v| v.as_str());
        let tools: Option<Vec<String>> = args.get("tools").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter().filter_map(|item| item.as_str().map(String::from)).collect()
            })
        });

        if provider.is_empty() || prompt.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Error: both 'provider' and 'prompt' are required".to_string(),
            });
        }

        // Validate provider exists
        if provider != "auto" && !self.llm_router.has_provider(provider) {
            let available = self.llm_router.provider_names().join(", ");
            return Ok(ToolResult {
                success: false,
                output: format!(
                    "Error: unknown provider '{}'. Available: {}",
                    provider, available
                ),
            });
        }

        info!(
            "delegate_to_provider: provider={}, model={}, tools={:?}, prompt={}...",
            provider,
            model.unwrap_or("(default)"),
            tools.as_ref().map(|t| t.join(",")),
            truncate_str(prompt, 60)
        );

        // Build ephemeral agent config
        let config = AgentConfig {
            provider: Some(provider.to_string()),
            model: model.map(String::from),
            tools,
            instructions: system_prompt.map(String::from),
            subagents: None,
            daily_budget_usd: 0.0,
            autonomy: crate::security::AutonomyLevel::Full,
        };

        let label = format!("provider:{}", provider);

        match self
            .agent_runtime
            .run_with_config(&label, &config, prompt, &[], &self.llm_router, &self.tool_registry, None, None, None)
            .await
        {
            Ok(result) => {
                info!(
                    "delegate_to_provider '{}' complete: {:.1}s, {} tool calls, {} tokens",
                    provider, result.elapsed_secs, result.tool_calls_made, result.total_tokens
                );
                Ok(ToolResult {
                    success: true,
                    output: result.output,
                })
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: format!("Provider delegation to '{}' failed: {}", provider, e),
            }),
        }
    }
}

/// Safely truncate a string at a character boundary
fn truncate_str(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_metadata() {
        let runtime = Arc::new(AgentRuntime::new("/nonexistent/path.toml").unwrap());
        let router = Arc::new(LlmRouter::new("/nonexistent/path.toml").unwrap());
        let registry = Arc::new(ToolRegistry::new(super::super::SecurityConfig::default()));

        let tool = DelegateToProviderTool::new(runtime, router, registry);
        assert_eq!(tool.name(), "delegate_to_provider");
        assert!(!tool.description().is_empty());

        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("provider").is_some());
        assert!(props.get("prompt").is_some());
        assert!(props.get("model").is_some());
        assert!(props.get("system_prompt").is_some());
        assert!(props.get("tools").is_some());

        let required = schema.get("required").unwrap().as_array().unwrap();
        assert_eq!(required.len(), 2);
    }

    #[tokio::test]
    async fn test_missing_params() {
        let runtime = Arc::new(AgentRuntime::new("/nonexistent/path.toml").unwrap());
        let router = Arc::new(LlmRouter::new("/nonexistent/path.toml").unwrap());
        let registry = Arc::new(ToolRegistry::new(super::super::SecurityConfig::default()));

        let tool = DelegateToProviderTool::new(runtime, router, registry);

        // Empty provider
        let result = tool.execute(json!({"provider": "", "prompt": "test"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("required"));

        // Empty prompt
        let result = tool.execute(json!({"provider": "ollama", "prompt": ""})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("required"));
    }

    #[tokio::test]
    async fn test_unknown_provider() {
        let runtime = Arc::new(AgentRuntime::new("/nonexistent/path.toml").unwrap());
        let router = Arc::new(LlmRouter::new("/nonexistent/path.toml").unwrap());
        let registry = Arc::new(ToolRegistry::new(super::super::SecurityConfig::default()));

        let tool = DelegateToProviderTool::new(runtime, router, registry);

        let result = tool.execute(json!({
            "provider": "nonexistent_provider",
            "prompt": "test task"
        })).await.unwrap();

        assert!(!result.success);
        assert!(result.output.contains("unknown provider"));
        assert!(result.output.contains("Available"));
    }

    #[tokio::test]
    async fn test_auto_provider_accepted() {
        let runtime = Arc::new(AgentRuntime::new("/nonexistent/path.toml").unwrap());
        let router = Arc::new(LlmRouter::new("/nonexistent/path.toml").unwrap());
        let registry = Arc::new(ToolRegistry::new(super::super::SecurityConfig::default()));

        let tool = DelegateToProviderTool::new(runtime, router, registry);

        // "auto" should pass validation (will fail at LLM call since no server running, but
        // that's fine — we just verify it doesn't reject "auto" as unknown)
        let result = tool.execute(json!({
            "provider": "auto",
            "prompt": "test task"
        })).await.unwrap();

        // Will fail because no LLM server is running, but shouldn't say "unknown provider"
        assert!(!result.output.contains("unknown provider"));
    }
}
