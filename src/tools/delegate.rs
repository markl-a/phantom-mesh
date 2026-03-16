// delegate tool — master agent can delegate tasks to sub-agents
// e.g. delegate(agent="coder", prompt="write a fibonacci function")

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::info;

use super::{Tool, ToolResult, ToolRegistry};
use crate::agent_runtime::AgentRuntime;
use crate::llm_router::LlmRouter;

pub struct DelegateTool {
    agent_runtime: Arc<AgentRuntime>,
    llm_router: Arc<LlmRouter>,
    tool_registry: Arc<ToolRegistry>,
}

impl DelegateTool {
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
impl Tool for DelegateTool {
    fn name(&self) -> &str { "delegate" }

    fn description(&self) -> &str {
        "Delegate a task to another agent (e.g. 'coder'). The sub-agent will execute the task and return its result."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "description": "Name of the agent to delegate to (e.g. 'coder')"
                },
                "prompt": {
                    "type": "string",
                    "description": "The task/prompt to send to the sub-agent"
                }
            },
            "required": ["agent", "prompt"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let agent_name = args
            .get("agent")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if agent_name.is_empty() || prompt.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Error: both 'agent' and 'prompt' are required".to_string(),
            });
        }

        // Prevent infinite delegation loops
        if agent_name == "master" {
            return Ok(ToolResult {
                success: false,
                output: "Error: cannot delegate to self (master)".to_string(),
            });
        }

        info!("Delegating to agent '{}': {}...", agent_name, truncate_str(prompt, 60));

        match self
            .agent_runtime
            .run(agent_name, prompt, &[], &self.llm_router, &self.tool_registry, None)
            .await
        {
            Ok(result) => {
                info!(
                    "Delegation to '{}' complete: {:.1}s, {} tool calls",
                    agent_name, result.elapsed_secs, result.tool_calls_made
                );
                Ok(ToolResult {
                    success: true,
                    output: result.output,
                })
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: format!("Delegation to '{}' failed: {}", agent_name, e),
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
