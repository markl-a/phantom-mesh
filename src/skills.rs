// Skills system — loadable skill packs that extend agent capabilities
//
// Skill files: TOML format in ~/.clawtex/skills/<name>/SKILL.toml
// Trust levels: Trusted (user-placed) vs Installed (from registry, read-only)
//
// Skills inject additional instructions + tool requirements into agent runs.
// Selection is trigger-based (keyword matching) with token budget limits.

use anyhow::Result;
use serde::Deserialize;
use std::path::Path;
use tracing::{debug, info, warn};

// ── Types ─────────────────────────────────────────────────────────────────────

/// Trust level determines tool access for a skill
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// User-placed skills: full tool access
    Trusted,
    /// Registry-installed skills: read-only tools only
    Installed,
}

impl Default for TrustLevel {
    fn default() -> Self {
        TrustLevel::Trusted
    }
}

/// A skill definition loaded from SKILL.toml
#[derive(Debug, Clone, Deserialize)]
pub struct SkillDef {
    /// Unique skill name
    pub name: String,
    /// Short description
    #[serde(default)]
    pub description: String,
    /// Activation triggers — keywords or regex patterns
    #[serde(default)]
    pub triggers: Vec<String>,
    /// Tools this skill requires (added to agent's tool list)
    #[serde(default)]
    pub requires_tools: Vec<String>,
    /// External binaries required (skill skipped if missing)
    #[serde(default)]
    pub requires_bins: Vec<String>,
    /// Environment variables required (skill skipped if missing)
    #[serde(default)]
    pub requires_env: Vec<String>,
    /// Approximate token budget for this skill's instructions
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    /// Trust level (determines tool access)
    #[serde(default)]
    pub trust: TrustLevel,
    /// Skill instructions (injected into system prompt)
    #[serde(default)]
    pub content: String,
    /// Agents this skill is restricted to (empty = all agents)
    #[serde(default)]
    pub agents: Vec<String>,
}

fn default_max_tokens() -> usize { 2000 }

/// A loaded skill with source metadata
#[derive(Debug, Clone)]
pub struct LoadedSkill {
    pub def: SkillDef,
    pub source_path: String,
    pub available: bool, // false if prerequisites are missing
}

// ── Skill Registry ────────────────────────────────────────────────────────────

pub struct SkillRegistry {
    skills: Vec<LoadedSkill>,
}

impl SkillRegistry {
    /// Create an empty skill registry (useful for tests).
    pub fn new() -> Self {
        Self { skills: Vec::new() }
    }

    /// Load skills from one or more directories.
    /// Each directory is scanned for subdirectories containing SKILL.toml.
    /// The trust_level parameter applies to all skills in the directory.
    pub fn load(dirs: &[(&str, TrustLevel)]) -> Result<Self> {
        let mut skills = Vec::new();

        for (dir, trust_level) in dirs {
            let dir_path = Path::new(dir);
            if !dir_path.exists() {
                debug!("Skills directory not found: {}", dir);
                continue;
            }

            let entries = match std::fs::read_dir(dir_path) {
                Ok(e) => e,
                Err(e) => {
                    warn!("Failed to read skills dir {}: {}", dir, e);
                    continue;
                }
            };

            for entry in entries.flatten() {
                let skill_dir = entry.path();
                if !skill_dir.is_dir() {
                    continue;
                }

                let toml_path = skill_dir.join("SKILL.toml");
                if !toml_path.exists() {
                    continue;
                }

                match std::fs::read_to_string(&toml_path) {
                    Ok(content) => {
                        match toml::from_str::<SkillDef>(&content) {
                            Ok(mut def) => {
                                // Override trust level from directory
                                def.trust = trust_level.clone();

                                // Check prerequisites
                                let available = check_prerequisites(&def);

                                info!(
                                    "Loaded skill '{}' from {} (trust={:?}, available={})",
                                    def.name,
                                    toml_path.display(),
                                    def.trust,
                                    available,
                                );

                                skills.push(LoadedSkill {
                                    def,
                                    source_path: toml_path.to_string_lossy().to_string(),
                                    available,
                                });
                            }
                            Err(e) => {
                                warn!("Failed to parse {}: {}", toml_path.display(), e);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to read {}: {}", toml_path.display(), e);
                    }
                }
            }
        }

        info!("Loaded {} skills ({} available)", skills.len(), skills.iter().filter(|s| s.available).count());
        Ok(Self { skills })
    }

    /// Select skills relevant to the user's message, within a token budget.
    /// Returns skills sorted by relevance score (descending).
    pub fn select_for_prompt(
        &self,
        user_message: &str,
        agent_name: &str,
        budget_tokens: usize,
    ) -> Vec<&LoadedSkill> {
        let msg_lower = user_message.to_lowercase();

        // Score each available skill by trigger matches
        let mut scored: Vec<(&LoadedSkill, usize)> = self
            .skills
            .iter()
            .filter(|s| s.available)
            .filter(|s| s.def.agents.is_empty() || s.def.agents.iter().any(|a| a == agent_name))
            .filter_map(|skill| {
                let score = score_triggers(&skill.def.triggers, &msg_lower);
                if score > 0 {
                    Some((skill, score))
                } else {
                    None
                }
            })
            .collect();

        // Sort by score descending
        scored.sort_by(|a, b| b.1.cmp(&a.1));

        // Select within budget
        let mut total_tokens = 0;
        let mut selected = Vec::new();

        for (skill, _score) in scored {
            if total_tokens + skill.def.max_tokens > budget_tokens {
                continue; // Skip skills that don't fit
            }
            total_tokens += skill.def.max_tokens;
            selected.push(skill);
        }

        debug!(
            "Selected {} skills for agent '{}' (~{} tokens)",
            selected.len(),
            agent_name,
            total_tokens,
        );

        selected
    }

    /// Format selected skills as context for system prompt injection
    pub fn format_context(selected: &[&LoadedSkill]) -> String {
        if selected.is_empty() {
            return String::new();
        }
        let mut ctx = String::from("\n[Active Skills]\n");
        for skill in selected {
            ctx.push_str(&format!(
                "\n--- {} ---\n{}\n",
                skill.def.name, skill.def.content
            ));
        }
        ctx
    }

    /// Get additional tools required by selected skills
    pub fn required_tools(selected: &[&LoadedSkill]) -> Vec<String> {
        let mut tools: Vec<String> = selected
            .iter()
            .flat_map(|s| s.def.requires_tools.iter().cloned())
            .collect();
        tools.sort();
        tools.dedup();
        tools
    }

    /// Get tool access restrictions based on trust level
    /// Returns a list of tool names that should be blocked for installed skills
    pub fn blocked_tools_for_installed() -> Vec<&'static str> {
        vec!["shell", "file_write", "delegate", "ai_code", "computer_use"]
    }

    /// Check if any selected skill is Installed (restricted) trust level
    pub fn has_installed_skills(selected: &[&LoadedSkill]) -> bool {
        selected.iter().any(|s| s.def.trust == TrustLevel::Installed)
    }

    pub fn list(&self) -> Vec<&LoadedSkill> {
        self.skills.iter().collect()
    }

    pub fn count(&self) -> usize {
        self.skills.len()
    }

    pub fn available_count(&self) -> usize {
        self.skills.iter().filter(|s| s.available).count()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Score how well a message matches skill triggers (0 = no match)
fn score_triggers(triggers: &[String], message_lower: &str) -> usize {
    let mut score = 0;
    for trigger in triggers {
        let trigger_lower = trigger.to_lowercase();
        // Exact word boundary matching
        if message_lower.contains(&trigger_lower) {
            score += 1;
            // Bonus for longer matches (more specific triggers)
            if trigger_lower.len() > 6 {
                score += 1;
            }
        }
    }
    score
}

/// Check if all prerequisites (bins + env vars) are available
fn check_prerequisites(def: &SkillDef) -> bool {
    // Check required binaries
    for bin in &def.requires_bins {
        if which_exists(bin).is_none() {
            debug!("Skill '{}' unavailable: missing binary '{}'", def.name, bin);
            return false;
        }
    }
    // Check required env vars
    for env_var in &def.requires_env {
        if std::env::var(env_var).is_err() {
            debug!(
                "Skill '{}' unavailable: missing env var '{}'",
                def.name, env_var
            );
            return false;
        }
    }
    true
}

/// Simple cross-platform check if a binary exists in PATH
fn which_exists(name: &str) -> Option<()> {
    let path_var = std::env::var("PATH").unwrap_or_default();
    let separator = if cfg!(windows) { ';' } else { ':' };
    let extensions: Vec<&str> = if cfg!(windows) {
        vec![".exe", ".cmd", ".bat", ".com"]
    } else {
        vec![""]
    };

    for dir in path_var.split(separator) {
        for ext in &extensions {
            let full_path = Path::new(dir).join(format!("{}{}", name, ext));
            if full_path.exists() {
                return Some(());
            }
        }
    }
    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_triggers() {
        let triggers = vec![
            "docker".to_string(),
            "container".to_string(),
            "kubernetes".to_string(),
        ];
        // Single match
        assert!(score_triggers(&triggers, "build a docker image") > 0);
        // Multiple matches
        let score = score_triggers(&triggers, "docker container running");
        assert!(score >= 2);
        // No match
        assert_eq!(score_triggers(&triggers, "hello world"), 0);
        // Long trigger bonus
        assert!(score_triggers(&triggers, "deploy to kubernetes") > 1);
    }

    #[test]
    fn test_check_prerequisites_missing_bin() {
        let def = SkillDef {
            name: "test".to_string(),
            description: String::new(),
            triggers: vec![],
            requires_tools: vec![],
            requires_bins: vec!["nonexistent_binary_xyz123".to_string()],
            requires_env: vec![],
            max_tokens: 1000,
            trust: TrustLevel::Trusted,
            content: String::new(),
            agents: vec![],
        };
        assert!(!check_prerequisites(&def));
    }

    #[test]
    fn test_check_prerequisites_no_requirements() {
        let def = SkillDef {
            name: "test".to_string(),
            description: String::new(),
            triggers: vec![],
            requires_tools: vec![],
            requires_bins: vec![],
            requires_env: vec![],
            max_tokens: 1000,
            trust: TrustLevel::Trusted,
            content: String::new(),
            agents: vec![],
        };
        assert!(check_prerequisites(&def));
    }

    #[test]
    fn test_empty_registry() {
        let registry = SkillRegistry { skills: vec![] };
        assert_eq!(registry.count(), 0);
        assert_eq!(registry.available_count(), 0);
        let selected = registry.select_for_prompt("hello", "master", 5000);
        assert!(selected.is_empty());
    }

    #[test]
    fn test_format_context_empty() {
        assert_eq!(SkillRegistry::format_context(&[]), "");
    }

    #[test]
    fn test_required_tools_dedup() {
        let skill1 = LoadedSkill {
            def: SkillDef {
                name: "a".to_string(),
                description: String::new(),
                triggers: vec![],
                requires_tools: vec!["shell".to_string(), "file_read".to_string()],
                requires_bins: vec![],
                requires_env: vec![],
                max_tokens: 1000,
                trust: TrustLevel::Trusted,
                content: String::new(),
                agents: vec![],
            },
            source_path: String::new(),
            available: true,
        };
        let skill2 = LoadedSkill {
            def: SkillDef {
                name: "b".to_string(),
                description: String::new(),
                triggers: vec![],
                requires_tools: vec!["shell".to_string(), "web_search".to_string()],
                requires_bins: vec![],
                requires_env: vec![],
                max_tokens: 1000,
                trust: TrustLevel::Trusted,
                content: String::new(),
                agents: vec![],
            },
            source_path: String::new(),
            available: true,
        };
        let selected: Vec<&LoadedSkill> = vec![&skill1, &skill2];
        let tools = SkillRegistry::required_tools(&selected);
        assert_eq!(tools, vec!["file_read", "shell", "web_search"]);
    }

    #[test]
    fn test_skill_selection_with_budget() {
        let registry = SkillRegistry {
            skills: vec![
                LoadedSkill {
                    def: SkillDef {
                        name: "docker".to_string(),
                        description: "Docker expert".to_string(),
                        triggers: vec!["docker".to_string(), "container".to_string()],
                        requires_tools: vec![],
                        requires_bins: vec![],
                        requires_env: vec![],
                        max_tokens: 3000,
                        trust: TrustLevel::Trusted,
                        content: "Docker instructions here".to_string(),
                        agents: vec![],
                    },
                    source_path: String::new(),
                    available: true,
                },
                LoadedSkill {
                    def: SkillDef {
                        name: "python".to_string(),
                        description: "Python expert".to_string(),
                        triggers: vec!["python".to_string(), "pip".to_string()],
                        requires_tools: vec![],
                        requires_bins: vec![],
                        requires_env: vec![],
                        max_tokens: 3000,
                        trust: TrustLevel::Trusted,
                        content: "Python instructions here".to_string(),
                        agents: vec![],
                    },
                    source_path: String::new(),
                    available: true,
                },
            ],
        };

        // Both match but budget only fits one
        let selected = registry.select_for_prompt("docker python container", "master", 4000);
        assert_eq!(selected.len(), 1);

        // Budget fits both
        let selected = registry.select_for_prompt("docker python container", "master", 10000);
        assert_eq!(selected.len(), 2);

        // No match
        let selected = registry.select_for_prompt("hello world", "master", 10000);
        assert!(selected.is_empty());
    }

    #[test]
    fn test_skill_agent_restriction() {
        let registry = SkillRegistry {
            skills: vec![LoadedSkill {
                def: SkillDef {
                    name: "coder-only".to_string(),
                    description: String::new(),
                    triggers: vec!["code".to_string()],
                    requires_tools: vec![],
                    requires_bins: vec![],
                    requires_env: vec![],
                    max_tokens: 1000,
                    trust: TrustLevel::Trusted,
                    content: "Coder instructions".to_string(),
                    agents: vec!["coder".to_string()],
                },
                source_path: String::new(),
                available: true,
            }],
        };

        // Should match for "coder" agent
        let selected = registry.select_for_prompt("write code", "coder", 5000);
        assert_eq!(selected.len(), 1);

        // Should NOT match for "master" agent
        let selected = registry.select_for_prompt("write code", "master", 5000);
        assert!(selected.is_empty());
    }

    #[test]
    fn test_parse_skill_toml() {
        let toml_str = r#"
name = "docker-expert"
description = "Expert at Docker operations"
triggers = ["docker", "container", "compose"]
requires_tools = ["shell"]
requires_bins = ["docker"]
max_tokens = 3000

content = """
You are a Docker expert.
Use docker CLI commands via the shell tool.
"""
"#;
        let skill: SkillDef = toml::from_str(toml_str).unwrap();
        assert_eq!(skill.name, "docker-expert");
        assert_eq!(skill.triggers.len(), 3);
        assert_eq!(skill.requires_tools, vec!["shell"]);
        assert!(skill.content.contains("Docker expert"));
    }
}
