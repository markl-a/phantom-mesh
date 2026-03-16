// run_hand tool — master agent can trigger Hand workflows via natural language
// e.g. user says "幫我找工作" → LLM calls run_hand(name="freelancer", input="Find web dev jobs")

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::info;

use super::{Tool, ToolResult};
use crate::agent_runtime::AgentRuntime;
use crate::hands::{HandRegistry, HandRunner};
use crate::llm_router::LlmRouter;
use crate::tools::ToolRegistry;

pub struct RunHandTool {
    agent_runtime: Arc<AgentRuntime>,
    llm_router: Arc<LlmRouter>,
    tool_registry: Arc<ToolRegistry>,
    hand_registry: Arc<HandRegistry>,
}

impl RunHandTool {
    pub fn new(
        agent_runtime: Arc<AgentRuntime>,
        llm_router: Arc<LlmRouter>,
        tool_registry: Arc<ToolRegistry>,
        hand_registry: Arc<HandRegistry>,
    ) -> Self {
        Self {
            agent_runtime,
            llm_router,
            tool_registry,
            hand_registry,
        }
    }
}

#[async_trait]
impl Tool for RunHandTool {
    fn name(&self) -> &str { "run_hand" }

    fn description(&self) -> &str {
        "Run a workflow Hand by name. Available hands: \
         freelancer (find jobs & write proposals), \
         lead (find potential clients), \
         outreach (cold email campaign), \
         seo_content (write SEO articles & publish), \
         content (create social media content & post), \
         researcher (deep research report), \
         market_intel (market & competitor analysis), \
         auto_report (automated data reports), \
         customer_service (FAQ & support), \
         trading_analysis (crypto/stock technical analysis). \
         Use this tool when the user wants to run a multi-step workflow."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Hand name: freelancer, lead, outreach, seo_content, content, researcher, market_intel, auto_report, customer_service, trading_analysis"
                },
                "input": {
                    "type": "string",
                    "description": "The input/request to pass to the hand workflow. Be specific about what to search for, write about, or analyze."
                }
            },
            "required": ["name", "input"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let hand_name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let input = args.get("input").and_then(|v| v.as_str()).unwrap_or("");

        if hand_name.is_empty() || input.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: format!(
                    "Error: 'name' and 'input' are required. Available hands: {}",
                    self.hand_registry.names().join(", ")
                ),
            });
        }

        let hand = match self.hand_registry.get(hand_name) {
            Some(h) => h.clone(),
            None => {
                return Ok(ToolResult {
                    success: false,
                    output: format!(
                        "Unknown hand '{}'. Available: {}",
                        hand_name,
                        self.hand_registry.names().join(", ")
                    ),
                });
            }
        };

        let truncated: String = input.chars().take(80).collect();
        info!("run_hand: starting '{}' with input: {}...", hand_name, truncated);

        match HandRunner::run(
            &hand, input,
            &self.agent_runtime, &self.llm_router, &self.tool_registry,
            None,
        ).await {
            Ok(result) => {
                let phase_summary: Vec<String> = result.outputs.iter().map(|o| {
                    let status = if o.skipped { "⏭ skipped" } else { "✅" };
                    format!("  {} {} ({} tool calls)", status, o.phase_name, o.tool_calls)
                }).collect();

                let summary = format!(
                    "Hand '{}' completed ({}/{} phases, {:.1}s)\n\nPhases:\n{}\n\n--- Output ---\n{}",
                    hand_name,
                    result.phases_completed, result.total_phases,
                    result.elapsed_secs,
                    phase_summary.join("\n"),
                    result.final_output,
                );

                info!("run_hand: '{}' done in {:.1}s", hand_name, result.elapsed_secs);

                Ok(ToolResult {
                    success: true,
                    output: summary,
                })
            }
            Err(e) => {
                Ok(ToolResult {
                    success: false,
                    output: format!("Hand '{}' failed: {}", hand_name, e),
                })
            }
        }
    }
}
