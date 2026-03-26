//! Pipeline Orchestrator — chains multiple Hands into sequential pipelines with data flow.
//!
//! Each pipeline is a sequence of steps, where each step runs a Hand and can thread
//! its output into the next step via template substitution. Steps can be conditional
//! (only run if previous output contains a keyword) or optional (pipeline continues
//! on failure).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::agent_runtime::AgentRuntime;
use crate::approval::ApprovalGate;
use crate::hands::{Hand, HandRunner, HandResult};
use crate::llm_router::LlmRouter;
use crate::tools::ToolRegistry;

/// A single step in a pipeline, referencing a Hand by name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStep {
    /// Name of the Hand to execute for this step.
    pub hand_name: String,
    /// Template for constructing the input to this step.
    /// Supports `{{prev_output}}` (output from previous step) and
    /// `{{user_input}}` (original user input) placeholders.
    pub input_template: String,
    /// If true, the pipeline continues even if this step fails.
    pub optional: bool,
    /// If set, this step only runs when the previous step's output
    /// contains this string (case-insensitive match).
    pub condition: Option<String>,
}

/// Definition of a complete pipeline — a named sequence of steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineDefinition {
    /// Pipeline name (e.g., "revenue-hunt", "content-publish").
    pub name: String,
    /// Human-readable description of what this pipeline does.
    pub description: String,
    /// Ordered list of steps to execute.
    pub steps: Vec<PipelineStep>,
}

/// Result of executing a pipeline.
#[derive(Debug, Clone, Serialize)]
pub struct PipelineResult {
    /// Name of the pipeline that was executed.
    pub pipeline_name: String,
    /// Number of steps that completed successfully.
    pub steps_completed: u32,
    /// Total number of steps in the pipeline.
    pub steps_total: u32,
    /// Output from each completed step: (hand_name, output).
    pub step_outputs: Vec<(String, String)>,
    /// Total wall-clock time across all steps (sum of elapsed_secs).
    pub total_elapsed_secs: f64,
    /// Whether the entire pipeline completed successfully.
    pub success: bool,
    /// Error message if the pipeline failed.
    pub error: Option<String>,
}

/// Pipeline Orchestrator — manages pipeline definitions and provides lookup.
///
/// Actual execution is handled by the standalone [`run_pipeline`] function,
/// which accepts all the runtime dependencies explicitly rather than storing
/// them on this struct.
pub struct PipelineOrchestrator {
    pipelines: HashMap<String, PipelineDefinition>,
}

impl PipelineOrchestrator {
    /// Create a new orchestrator pre-loaded with built-in pipelines.
    pub fn new() -> Self {
        let mut pipelines = HashMap::new();

        // Built-in: revenue-hunt — find freelance leads then do outreach
        let revenue_hunt = PipelineDefinition {
            name: "revenue-hunt".to_string(),
            description: "Find freelance leads then send outreach messages".to_string(),
            steps: vec![
                PipelineStep {
                    hand_name: "freelancer".to_string(),
                    input_template: "{{user_input}}".to_string(),
                    optional: false,
                    condition: None,
                },
                PipelineStep {
                    hand_name: "outreach".to_string(),
                    input_template: "Based on the following leads:\n{{prev_output}}\n\nOriginal request: {{user_input}}".to_string(),
                    optional: false,
                    condition: Some("found".to_string()),
                },
            ],
        };
        pipelines.insert(revenue_hunt.name.clone(), revenue_hunt);

        // Built-in: content-publish — generate SEO content then publish
        let content_publish = PipelineDefinition {
            name: "content-publish".to_string(),
            description: "Generate SEO-optimized content then publish it".to_string(),
            steps: vec![
                PipelineStep {
                    hand_name: "seo_content".to_string(),
                    input_template: "{{user_input}}".to_string(),
                    optional: false,
                    condition: None,
                },
                PipelineStep {
                    hand_name: "content".to_string(),
                    input_template: "Publish the following content:\n{{prev_output}}".to_string(),
                    optional: true,
                    condition: None,
                },
            ],
        };
        pipelines.insert(content_publish.name.clone(), content_publish);

        Self { pipelines }
    }

    /// Register a custom pipeline definition.
    pub fn register_pipeline(&mut self, def: PipelineDefinition) {
        info!("Registering pipeline: {}", def.name);
        self.pipelines.insert(def.name.clone(), def);
    }

    /// List all registered pipelines.
    pub fn list_pipelines(&self) -> Vec<&PipelineDefinition> {
        self.pipelines.values().collect()
    }

    /// Look up a pipeline by name.
    pub fn get_pipeline(&self, name: &str) -> Option<&PipelineDefinition> {
        self.pipelines.get(name)
    }
}

/// Apply template substitution, replacing `{{prev_output}}` and `{{user_input}}`
/// with the provided values.
fn apply_template(template: &str, prev_output: &str, user_input: &str) -> String {
    template
        .replace("{{prev_output}}", prev_output)
        .replace("{{user_input}}", user_input)
}

/// Check whether a condition is satisfied by the previous output.
/// Conditions are matched case-insensitively.
fn condition_matches(condition: &str, prev_output: &str) -> bool {
    let cond_lower = condition.to_lowercase();
    let output_lower = prev_output.to_lowercase();
    output_lower.contains(&cond_lower)
}

/// Execute a pipeline definition end-to-end.
///
/// Iterates through each step, resolves the input template, checks conditions,
/// executes the referenced Hand, and threads the output to the next step.
///
/// This is a standalone function rather than a method on `PipelineOrchestrator`
/// because it requires many runtime dependencies (router, tools, etc.) that
/// should not be stored on the orchestrator.
pub async fn run_pipeline(
    definition: &PipelineDefinition,
    user_input: &str,
    hand_runner: &HandRunner,
    hands: &HashMap<String, Hand>,
    runtime: &AgentRuntime,
    router: &LlmRouter,
    tool_registry: &ToolRegistry,
    approval_gate: Option<&Arc<ApprovalGate>>,
) -> Result<PipelineResult> {
    let steps_total = definition.steps.len() as u32;
    let mut step_outputs: Vec<(String, String)> = Vec::new();
    let mut prev_output = String::new();
    let mut total_elapsed = 0.0_f64;

    info!(
        "Starting pipeline '{}' with {} steps",
        definition.name, steps_total
    );

    for (idx, step) in definition.steps.iter().enumerate() {
        debug!(
            "Pipeline '{}' step {}/{}: hand='{}'",
            definition.name,
            idx + 1,
            steps_total,
            step.hand_name
        );

        // Check condition (if set)
        if let Some(ref cond) = step.condition {
            if !condition_matches(cond, &prev_output) {
                info!(
                    "Pipeline '{}' step {} skipped: condition '{}' not met",
                    definition.name,
                    idx + 1,
                    cond
                );
                // Skipped steps don't produce output; prev_output stays the same
                continue;
            }
        }

        // Resolve the hand definition
        let hand = match hands.get(&step.hand_name) {
            Some(h) => h,
            None => {
                let msg = format!(
                    "Pipeline '{}' step {}: hand '{}' not found",
                    definition.name,
                    idx + 1,
                    step.hand_name
                );
                if step.optional {
                    warn!("{} (optional, continuing)", msg);
                    continue;
                }
                return Ok(PipelineResult {
                    pipeline_name: definition.name.clone(),
                    steps_completed: step_outputs.len() as u32,
                    steps_total,
                    step_outputs,
                    total_elapsed_secs: total_elapsed,
                    success: false,
                    error: Some(msg),
                });
            }
        };

        // Build the input from the template
        let step_input = apply_template(&step.input_template, &prev_output, user_input);

        // Execute the hand
        let result: Result<HandResult> = hand_runner
            .execute(hand, &step_input, runtime, router, tool_registry, approval_gate)
            .await;

        match result {
            Ok(hand_result) => {
                total_elapsed += hand_result.elapsed_secs;
                prev_output = hand_result.final_output.clone();
                step_outputs.push((step.hand_name.clone(), hand_result.final_output));
                info!(
                    "Pipeline '{}' step {}/{} completed in {:.2}s",
                    definition.name,
                    idx + 1,
                    steps_total,
                    hand_result.elapsed_secs
                );
            }
            Err(e) => {
                let msg = format!(
                    "Pipeline '{}' step {} (hand '{}') failed: {}",
                    definition.name,
                    idx + 1,
                    step.hand_name,
                    e
                );
                if step.optional {
                    warn!("{} (optional, continuing)", msg);
                    continue;
                }
                return Ok(PipelineResult {
                    pipeline_name: definition.name.clone(),
                    steps_completed: step_outputs.len() as u32,
                    steps_total,
                    step_outputs,
                    total_elapsed_secs: total_elapsed,
                    success: false,
                    error: Some(msg),
                });
            }
        }
    }

    info!(
        "Pipeline '{}' completed: {}/{} steps in {:.2}s",
        definition.name,
        step_outputs.len(),
        steps_total,
        total_elapsed
    );

    Ok(PipelineResult {
        pipeline_name: definition.name.clone(),
        steps_completed: step_outputs.len() as u32,
        steps_total,
        step_outputs,
        total_elapsed_secs: total_elapsed,
        success: true,
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_pipelines_exist() {
        let orchestrator = PipelineOrchestrator::new();

        // revenue-hunt should exist with 2 steps
        let rh = orchestrator.get_pipeline("revenue-hunt");
        assert!(rh.is_some(), "revenue-hunt pipeline should exist");
        let rh = rh.unwrap();
        assert_eq!(rh.name, "revenue-hunt");
        assert_eq!(rh.steps.len(), 2);
        assert_eq!(rh.steps[0].hand_name, "freelancer");
        assert_eq!(rh.steps[1].hand_name, "outreach");
        assert!(!rh.steps[0].optional);
        assert!(!rh.steps[1].optional);
        assert!(rh.steps[0].condition.is_none());
        assert_eq!(rh.steps[1].condition.as_deref(), Some("found"));

        // content-publish should exist with 2 steps
        let cp = orchestrator.get_pipeline("content-publish");
        assert!(cp.is_some(), "content-publish pipeline should exist");
        let cp = cp.unwrap();
        assert_eq!(cp.name, "content-publish");
        assert_eq!(cp.steps.len(), 2);
        assert_eq!(cp.steps[0].hand_name, "seo_content");
        assert_eq!(cp.steps[1].hand_name, "content");
        assert!(!cp.steps[0].optional);
        assert!(cp.steps[1].optional);

        // list_pipelines should return both
        let all = orchestrator.list_pipelines();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_register_custom_pipeline() {
        let mut orchestrator = PipelineOrchestrator::new();
        assert_eq!(orchestrator.list_pipelines().len(), 2);

        let custom = PipelineDefinition {
            name: "my-custom".to_string(),
            description: "A custom test pipeline".to_string(),
            steps: vec![
                PipelineStep {
                    hand_name: "research".to_string(),
                    input_template: "Research: {{user_input}}".to_string(),
                    optional: false,
                    condition: None,
                },
                PipelineStep {
                    hand_name: "summarize".to_string(),
                    input_template: "Summarize: {{prev_output}}".to_string(),
                    optional: true,
                    condition: None,
                },
            ],
        };

        orchestrator.register_pipeline(custom);
        assert_eq!(orchestrator.list_pipelines().len(), 3);

        let retrieved = orchestrator.get_pipeline("my-custom");
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.steps.len(), 2);
        assert_eq!(retrieved.steps[0].hand_name, "research");
        assert_eq!(retrieved.description, "A custom test pipeline");
    }

    #[test]
    fn test_input_template_substitution() {
        // Both placeholders
        let result = apply_template(
            "Previous: {{prev_output}}\nInput: {{user_input}}",
            "some previous data",
            "the user query",
        );
        assert_eq!(result, "Previous: some previous data\nInput: the user query");

        // Only user_input
        let result = apply_template("{{user_input}}", "", "hello world");
        assert_eq!(result, "hello world");

        // Only prev_output
        let result = apply_template("Data: {{prev_output}}", "output from step 1", "");
        assert_eq!(result, "Data: output from step 1");

        // No placeholders — template is passed through as-is
        let result = apply_template("static text", "ignored", "also ignored");
        assert_eq!(result, "static text");

        // Multiple occurrences of the same placeholder
        let result = apply_template(
            "{{user_input}} and again {{user_input}}",
            "",
            "repeat",
        );
        assert_eq!(result, "repeat and again repeat");

        // Empty template
        let result = apply_template("", "prev", "input");
        assert_eq!(result, "");
    }

    #[test]
    fn test_condition_check() {
        // Basic case-insensitive match
        assert!(condition_matches("found", "We found 5 leads today"));
        assert!(condition_matches("FOUND", "We found 5 leads today"));
        assert!(condition_matches("Found", "we FOUND 5 leads today"));
        assert!(condition_matches("found", "FOUND SOMETHING"));

        // No match
        assert!(!condition_matches("found", "Nothing here"));
        assert!(!condition_matches("success", "The operation failed"));

        // Partial match within a word
        assert!(condition_matches("lead", "freelancer_leads_list"));

        // Empty condition always matches (empty string is contained in everything)
        assert!(condition_matches("", "any output"));
        assert!(condition_matches("", ""));

        // Empty output — non-empty condition should not match
        assert!(!condition_matches("found", ""));

        // Multi-word condition
        assert!(condition_matches("leads found", "We have leads found in the results"));
        assert!(!condition_matches("leads found", "We have leads but nothing found"));
    }
}
