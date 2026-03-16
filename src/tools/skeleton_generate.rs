// skeleton_generate tool — Skeleton-of-Thought parallel content generation
// Splits a topic into outline sections, expands them in parallel across
// multiple providers (CPU/GPU/NPU), and merges into a complete document.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::info;

use super::{Tool, ToolResult};
use crate::llm_router::LlmRouter;
use crate::skeleton::{SkeletonConfig, SkeletonRunner};

pub struct SkeletonGenerateTool {
    llm_router: Arc<LlmRouter>,
}

impl SkeletonGenerateTool {
    pub fn new(llm_router: Arc<LlmRouter>) -> Self {
        Self { llm_router }
    }
}

#[async_trait]
impl Tool for SkeletonGenerateTool {
    fn name(&self) -> &str { "skeleton_generate" }

    fn description(&self) -> &str {
        "Generate long-form content using Skeleton-of-Thought (SoT) parallel generation. \
         Splits a topic into an outline, then expands each section in parallel across \
         multiple providers (CPU/GPU/NPU) for faster generation. Use this for articles, \
         guides, reports, and any long content that benefits from parallel generation."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "topic": {
                    "type": "string",
                    "description": "The topic or prompt to generate content about. Be specific for better results."
                },
                "skeleton_provider": {
                    "type": "string",
                    "description": "Provider for outline generation (default: auto). Use 'lemonade' for NPU, 'ollama' for CPU, 'lmstudio' for GPU."
                },
                "expansion_providers": {
                    "type": "string",
                    "description": "Comma-separated list of providers for parallel section expansion (default: lmstudio,lemonade,ollama)"
                },
                "max_sections": {
                    "type": "integer",
                    "description": "Maximum number of sections in the outline (default: 10, max: 15)"
                }
            },
            "required": ["topic"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let topic = args.get("topic").and_then(|v| v.as_str()).unwrap_or("");
        if topic.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Error: 'topic' is required.".to_string(),
            });
        }

        // Build config from args
        let mut config = SkeletonConfig::default();

        if let Some(sp) = args.get("skeleton_provider").and_then(|v| v.as_str()) {
            if !sp.is_empty() {
                config.skeleton_provider = sp.to_string();
            }
        }

        if let Some(ep) = args.get("expansion_providers").and_then(|v| v.as_str()) {
            if !ep.is_empty() {
                config.expansion_providers = ep.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }

        if let Some(ms) = args.get("max_sections").and_then(|v| v.as_u64()) {
            config.max_sections = (ms as usize).min(15);
        }

        let truncated: String = topic.chars().take(80).collect();
        info!("skeleton_generate: starting SoT for '{}...' (skeleton={}, expand={:?}, max={})",
            truncated, config.skeleton_provider, config.expansion_providers, config.max_sections);

        let runner = SkeletonRunner::new(self.llm_router.clone(), config);

        match runner.generate(topic).await {
            Ok(result) => {
                let summary = format!(
                    "SoT generation complete: {}/{} sections expanded\n\
                     Skeleton provider: {}\n\
                     Expansion providers used: {}\n\n\
                     --- Generated Content ---\n\n{}",
                    result.successful_sections,
                    result.total_sections,
                    result.skeleton_provider,
                    result.providers_used.join(", "),
                    result.merged_output,
                );

                info!("skeleton_generate: done — {}/{} sections",
                    result.successful_sections, result.total_sections);

                Ok(ToolResult {
                    success: true,
                    output: summary,
                })
            }
            Err(e) => {
                Ok(ToolResult {
                    success: false,
                    output: format!("SoT generation failed: {}", e),
                })
            }
        }
    }
}
