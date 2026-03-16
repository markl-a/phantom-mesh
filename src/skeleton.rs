// Skeleton-of-Thought (SoT) parallel generation engine
// Splits long content into skeleton (outline) + parallel section expansion
// across multiple providers (CPU/GPU/NPU) for 3-5x speedup.

use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::llm_router::LlmRouter;
use crate::providers::ChatMessage;

// ── Config ───────────────────────────────────────────────────────────────────

/// Configuration for the SoT engine
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SkeletonConfig {
    /// Provider to use for skeleton (outline) generation (e.g. "lemonade" for NPU)
    #[serde(default = "default_skeleton_provider")]
    pub skeleton_provider: String,
    /// Providers to round-robin for parallel section expansion
    #[serde(default = "default_expansion_providers")]
    pub expansion_providers: Vec<String>,
    /// Maximum number of sections in the outline
    #[serde(default = "default_max_sections")]
    pub max_sections: usize,
    /// Max tokens hint per section expansion
    #[serde(default = "default_section_max_tokens")]
    pub section_max_tokens: usize,
    /// Timeout in seconds per section expansion (0 = no timeout)
    #[serde(default = "default_section_timeout")]
    pub section_timeout_secs: u64,
}

fn default_skeleton_provider() -> String { "auto".to_string() }
fn default_section_timeout() -> u64 { 120 }
fn default_expansion_providers() -> Vec<String> {
    vec!["lmstudio".into(), "npu".into(), "ollama".into()]
}
fn default_max_sections() -> usize { 10 }
fn default_section_max_tokens() -> usize { 800 }

impl Default for SkeletonConfig {
    fn default() -> Self {
        Self {
            skeleton_provider: default_skeleton_provider(),
            expansion_providers: default_expansion_providers(),
            max_sections: default_max_sections(),
            section_max_tokens: default_section_max_tokens(),
            section_timeout_secs: default_section_timeout(),
        }
    }
}

// ── Types ────────────────────────────────────────────────────────────────────

/// A single section parsed from the skeleton outline
#[derive(Debug, Clone, Serialize)]
pub struct SkeletonSection {
    pub index: usize,
    pub title: String,
    pub description: String,
}

/// Result of expanding a single section
#[derive(Debug, Clone, Serialize)]
pub struct SectionResult {
    pub index: usize,
    pub title: String,
    pub content: String,
    pub provider: String,
    pub success: bool,
}

/// Final result of the full SoT pipeline
#[derive(Debug, Clone, Serialize)]
pub struct SkeletonResult {
    pub topic: String,
    pub sections: Vec<SectionResult>,
    pub merged_output: String,
    pub skeleton_provider: String,
    pub providers_used: Vec<String>,
    pub total_sections: usize,
    pub successful_sections: usize,
}

// ── Engine ───────────────────────────────────────────────────────────────────

/// Skeleton-of-Thought runner
pub struct SkeletonRunner {
    llm_router: Arc<LlmRouter>,
    config: SkeletonConfig,
}

impl SkeletonRunner {
    pub fn new(llm_router: Arc<LlmRouter>, config: SkeletonConfig) -> Self {
        Self { llm_router, config }
    }

    /// Full pipeline: skeleton → parallel expand → merge
    pub async fn generate(&self, topic: &str) -> Result<SkeletonResult> {
        info!("SoT: starting for topic: {}", &topic[..topic.len().min(80)]);

        // Step 1: Generate skeleton
        let sections = self.generate_skeleton(topic).await
            .context("Failed to generate skeleton outline")?;
        info!("SoT: skeleton has {} sections", sections.len());

        // Step 2: Parallel expansion
        let results = self.expand_parallel(topic, &sections).await;
        let providers_used: Vec<String> = results.iter()
            .map(|r| r.provider.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let successful = results.iter().filter(|r| r.success).count();
        info!("SoT: expanded {}/{} sections across {} providers",
            successful, results.len(), providers_used.len());

        // Step 3: Merge
        let merged = Self::merge(&results);

        Ok(SkeletonResult {
            topic: topic.to_string(),
            sections: results,
            merged_output: merged,
            skeleton_provider: self.config.skeleton_provider.clone(),
            providers_used,
            total_sections: sections.len(),
            successful_sections: successful,
        })
    }

    /// Generate a skeleton outline using the designated provider
    pub async fn generate_skeleton(&self, topic: &str) -> Result<Vec<SkeletonSection>> {
        let prompt = format!(
            "You are an expert content architect. Create a detailed outline for the following topic.\n\
             Format each section EXACTLY as:\n\
             N. Title --- Description\n\n\
             Where N is the section number, Title is a concise section heading, \
             and Description is a 1-2 sentence summary of what this section should cover.\n\n\
             Topic: {}\n\n\
             Create {} sections maximum. Output ONLY the numbered outline, nothing else.",
            topic, self.config.max_sections
        );

        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
            tool_calls: None,
            tool_call_id: None,
        }];

        let skeleton_timeout = Duration::from_secs(self.config.section_timeout_secs.max(60));
        let response = tokio::time::timeout(
            skeleton_timeout,
            self.llm_router.chat_with_tools(&messages, &[], &self.config.skeleton_provider),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Skeleton generation timed out after {}s", skeleton_timeout.as_secs()))?
        .context("Skeleton generation LLM call failed")?;

        let text = response.message.content.clone();
        let sections = parse_skeleton(&text, self.config.max_sections);

        if sections.is_empty() {
            anyhow::bail!("Failed to parse any sections from skeleton output:\n{}", text);
        }

        Ok(sections)
    }

    /// Expand all sections in parallel, round-robin across alive providers
    pub async fn expand_parallel(&self, topic: &str, sections: &[SkeletonSection]) -> Vec<SectionResult> {
        let configured = &self.config.expansion_providers;

        // Filter to only alive providers (probe each one)
        let mut alive_providers = Vec::new();
        for name in configured {
            if self.llm_router.has_provider(name) && self.llm_router.is_alive(name).await {
                alive_providers.push(name.clone());
            } else {
                warn!("SoT: provider '{}' not alive, skipping", name);
            }
        }

        // Fallback: if none alive, try "auto"
        if alive_providers.is_empty() {
            warn!("SoT: no configured expansion providers alive, falling back to 'auto'");
            if self.llm_router.any_alive().await {
                alive_providers.push("auto".to_string());
            } else {
                warn!("SoT: no providers alive at all!");
                return sections.iter().map(|s| SectionResult {
                    index: s.index,
                    title: s.title.clone(),
                    content: "Error: no LLM providers available".into(),
                    provider: "none".into(),
                    success: false,
                }).collect();
            }
        }

        info!("SoT: expanding {} sections across alive providers: {:?}", sections.len(), alive_providers);

        let timeout_dur = if self.config.section_timeout_secs > 0 {
            Some(Duration::from_secs(self.config.section_timeout_secs))
        } else {
            None
        };

        let mut handles = Vec::new();

        for section in sections {
            let provider = alive_providers[section.index % alive_providers.len()].clone();

            let router = self.llm_router.clone();
            let topic = topic.to_string();
            let section = section.clone();
            let max_tokens = self.config.section_max_tokens;

            let handle = tokio::spawn(async move {
                let fut = expand_section(&router, &topic, &section, &provider, max_tokens);
                if let Some(timeout) = timeout_dur {
                    match tokio::time::timeout(timeout, fut).await {
                        Ok(result) => result,
                        Err(_) => {
                            warn!("SoT: section {} '{}' timed out on provider {} ({}s)",
                                section.index, section.title, provider, timeout.as_secs());
                            SectionResult {
                                index: section.index,
                                title: section.title.clone(),
                                content: format!("Error: timed out after {}s", timeout.as_secs()),
                                provider: provider.to_string(),
                                success: false,
                            }
                        }
                    }
                } else {
                    fut.await
                }
            });
            handles.push(handle);
        }

        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(e) => {
                    warn!("SoT: section expansion task panicked: {}", e);
                }
            }
        }

        results
    }

    /// Merge section results by index order into a single markdown document
    pub fn merge(results: &[SectionResult]) -> String {
        let mut sorted: Vec<&SectionResult> = results.iter().collect();
        sorted.sort_by_key(|r| r.index);

        let mut output = String::new();
        for result in sorted {
            if result.success {
                output.push_str(&format!("## {}\n\n{}\n\n", result.title, result.content));
            } else {
                output.push_str(&format!("## {}\n\n*[Section generation failed]*\n\n", result.title));
            }
        }

        output.trim_end().to_string()
    }
}

/// Expand a single section using the given provider
async fn expand_section(
    router: &LlmRouter,
    topic: &str,
    section: &SkeletonSection,
    provider: &str,
    _max_tokens: usize,
) -> SectionResult {
    let prompt = format!(
        "You are writing section {} of an article about: {}\n\n\
         Section title: {}\n\
         Section description: {}\n\n\
         Write this section in detail with clear explanations. \
         Use markdown formatting where appropriate (bullet points, code blocks, etc). \
         Output ONLY the section content, no title heading.",
        section.index + 1, topic, section.title, section.description
    );

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: prompt,
        tool_calls: None,
        tool_call_id: None,
    }];

    match router.chat_with_tools(&messages, &[], provider).await {
        Ok(response) => {
            let content = response.message.content.clone();
            debug!("SoT: section {} '{}' expanded via {} ({} chars)",
                section.index, section.title, provider, content.len());
            SectionResult {
                index: section.index,
                title: section.title.clone(),
                content,
                provider: provider.to_string(),
                success: true,
            }
        }
        Err(e) => {
            warn!("SoT: section {} '{}' failed on provider {}: {}",
                section.index, section.title, provider, e);
            SectionResult {
                index: section.index,
                title: section.title.clone(),
                content: format!("Error: {}", e),
                provider: provider.to_string(),
                success: false,
            }
        }
    }
}

// ── Parsing ──────────────────────────────────────────────────────────────────

/// Parse skeleton output into sections with 3-tier fallback:
/// 1. "N. Title --- Description" format
/// 2. "N. Title: Description" or "N. Title - Description" format
/// 3. "N. Title" (no description) format
pub fn parse_skeleton(text: &str, max_sections: usize) -> Vec<SkeletonSection> {
    // Tier 1: "N. Title --- Description"
    let re_tier1 = Regex::new(r"(?m)^\s*(\d+)\.\s+(.+?)\s*---\s*(.+)$").unwrap();
    let mut sections: Vec<SkeletonSection> = re_tier1.captures_iter(text)
        .filter_map(|cap| {
            let index = cap[1].parse::<usize>().ok()?.checked_sub(1)?;
            Some(SkeletonSection {
                index,
                title: cap[2].trim().to_string(),
                description: cap[3].trim().to_string(),
            })
        })
        .take(max_sections)
        .collect();

    if !sections.is_empty() {
        return sections;
    }

    // Tier 2: "N. Title: Description" or "N. Title - Description"
    let re_tier2 = Regex::new(r"(?m)^\s*(\d+)\.\s+(.+?)\s*[:\-–—]\s+(.+)$").unwrap();
    sections = re_tier2.captures_iter(text)
        .filter_map(|cap| {
            let index = cap[1].parse::<usize>().ok()?.checked_sub(1)?;
            Some(SkeletonSection {
                index,
                title: cap[2].trim().to_string(),
                description: cap[3].trim().to_string(),
            })
        })
        .take(max_sections)
        .collect();

    if !sections.is_empty() {
        return sections;
    }

    // Tier 3: "N. Title" (bare numbered list)
    let re_tier3 = Regex::new(r"(?m)^\s*(\d+)\.\s+(.+)$").unwrap();
    sections = re_tier3.captures_iter(text)
        .filter_map(|cap| {
            let index = cap[1].parse::<usize>().ok()?.checked_sub(1)?;
            let title = cap[2].trim().to_string();
            Some(SkeletonSection {
                index,
                title: title.clone(),
                description: format!("Expand on: {}", title),
            })
        })
        .take(max_sections)
        .collect();

    sections
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tier1_normal() {
        let text = "\
1. Introduction --- Overview of the topic and why it matters
2. Core Concepts --- Key principles and fundamentals
3. Implementation --- Step-by-step guide to building it";
        let sections = parse_skeleton(text, 10);
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].index, 0);
        assert_eq!(sections[0].title, "Introduction");
        assert_eq!(sections[0].description, "Overview of the topic and why it matters");
        assert_eq!(sections[2].index, 2);
        assert_eq!(sections[2].title, "Implementation");
    }

    #[test]
    fn test_parse_tier2_colon_format() {
        let text = "\
1. Introduction: Overview of the topic
2. Architecture: System design and components
3. Conclusion: Final thoughts";
        let sections = parse_skeleton(text, 10);
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].title, "Introduction");
        assert_eq!(sections[0].description, "Overview of the topic");
    }

    #[test]
    fn test_parse_tier3_bare_list() {
        let text = "\
1. Introduction
2. Background
3. Method
4. Results";
        let sections = parse_skeleton(text, 10);
        assert_eq!(sections.len(), 4);
        assert_eq!(sections[0].title, "Introduction");
        assert!(sections[0].description.contains("Introduction"));
    }

    #[test]
    fn test_parse_with_noise() {
        let text = "\
Here is the outline for your article:\n\n\
1. Getting Started --- How to set up the environment\n\
Some extra noise here\n\
2. Deep Dive --- Advanced techniques and patterns\n\
More noise\n\
3. Best Practices --- Tips for production use";
        let sections = parse_skeleton(text, 10);
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[1].title, "Deep Dive");
    }

    #[test]
    fn test_parse_respects_max_sections() {
        let text = "\
1. A --- desc\n2. B --- desc\n3. C --- desc\n4. D --- desc\n5. E --- desc";
        let sections = parse_skeleton(text, 3);
        assert_eq!(sections.len(), 3);
    }

    #[test]
    fn test_parse_empty_input() {
        let sections = parse_skeleton("", 10);
        assert!(sections.is_empty());
    }

    #[test]
    fn test_parse_no_sections() {
        let sections = parse_skeleton("This is just some random text with no numbered items.", 10);
        assert!(sections.is_empty());
    }

    #[test]
    fn test_merge_sorts_by_index() {
        let results = vec![
            SectionResult { index: 2, title: "C".into(), content: "Third".into(), provider: "p1".into(), success: true },
            SectionResult { index: 0, title: "A".into(), content: "First".into(), provider: "p2".into(), success: true },
            SectionResult { index: 1, title: "B".into(), content: "Second".into(), provider: "p3".into(), success: true },
        ];
        let merged = SkeletonRunner::merge(&results);
        let lines: Vec<&str> = merged.lines().collect();
        assert_eq!(lines[0], "## A");
        assert_eq!(lines[2], "First");
        assert_eq!(lines[4], "## B");
    }

    #[test]
    fn test_merge_handles_failed_section() {
        let results = vec![
            SectionResult { index: 0, title: "OK".into(), content: "Works".into(), provider: "p1".into(), success: true },
            SectionResult { index: 1, title: "Failed".into(), content: "Error".into(), provider: "p2".into(), success: false },
        ];
        let merged = SkeletonRunner::merge(&results);
        assert!(merged.contains("## OK"));
        assert!(merged.contains("Works"));
        assert!(merged.contains("[Section generation failed]"));
    }

    #[test]
    fn test_config_defaults() {
        let config = SkeletonConfig::default();
        assert_eq!(config.skeleton_provider, "auto");
        assert_eq!(config.expansion_providers, vec!["lmstudio", "npu", "ollama"]);
        assert_eq!(config.max_sections, 10);
        assert_eq!(config.section_max_tokens, 800);
        assert_eq!(config.section_timeout_secs, 120);
    }
}
