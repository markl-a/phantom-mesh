//! Config validator for agents.toml — validates configuration on startup.
//!
//! Checks required fields, cross-references between agents and providers,
//! tool name validity, rate limit sanity, port ranges, budget values,
//! cron expressions, circular delegation, and cluster URL format.

use std::collections::{HashMap, HashSet};
use std::fmt;

/// Severity level for a validation issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Info => write!(f, "INFO"),
            Severity::Warning => write!(f, "WARNING"),
            Severity::Error => write!(f, "ERROR"),
        }
    }
}

/// A single validation finding with severity, field path, and message.
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub severity: Severity,
    pub field: String,
    pub message: String,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.severity, self.field, self.message)
    }
}

/// Aggregated result of config validation.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationError>,
    pub infos: Vec<ValidationError>,
}

impl ValidationResult {
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            warnings: Vec::new(),
            infos: Vec::new(),
        }
    }

    /// Add a finding and route it to the correct list by severity.
    pub fn add(&mut self, error: ValidationError) {
        match error.severity {
            Severity::Error => self.errors.push(error),
            Severity::Warning => self.warnings.push(error),
            Severity::Info => self.infos.push(error),
        }
    }

    /// Returns true if no errors were found (warnings and infos are acceptable).
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// Total number of findings across all severities.
    pub fn total_count(&self) -> usize {
        self.errors.len() + self.warnings.len() + self.infos.len()
    }
}

impl Default for ValidationResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Known tool names that the system supports.
/// This list is kept in sync with ToolRegistry::new_with_search() in tools/mod.rs,
/// plus dynamically registered tools (delegate, delegate_to_provider, etc.).
const KNOWN_TOOLS: &[&str] = &[
    "shell",
    "file_read",
    "file_write",
    "file_edit",
    "web_search",
    "http_request",
    "glob_search",
    "content_search",
    "browser",
    "video_compose",
    "youtube_upload",
    "music_generate",
    "knowledge_import",
    "calendar",
    "data_analysis",
    "screenshot",
    "qr_generate",
    "rss_reader",
    "archive_extract",
    "clipboard",
    "system_info",
    "weather",
    "calculator",
    "notification_center",
    "memory_store",
    "memory_recall",
    "memory_forget",
    "delegate",
    "delegate_to_provider",
    "ai_code",
    "computer_use",
    "vision",
    "email_send",
    "email_receive",
    "twitter",
    "blog_publish",
    "pdf_export",
    "run_hand",
    "skeleton_generate",
    "stripe",
    "render_deploy",
    "scaffold_saas",
    "cli_anything",
    "slack_send",
    "discord_send",
    "line_send",
    "whatsapp_send",
    "translate",
    "json_transform",
    "csv_parse",
    "summarize",
    "image_generate",
    "docx_export",
    "xlsx_export",
    "tts",
    // Mobile-only tools
    "sensor_gps",
    "sensor_camera",
    "sensor_accel",
    "sensor_audio",
    "local_llm",
    "js_exec",
];

/// ConfigValidator validates a parsed TOML config value.
pub struct ConfigValidator {
    known_tools: HashSet<String>,
}

impl ConfigValidator {
    pub fn new() -> Self {
        let known_tools: HashSet<String> = KNOWN_TOOLS.iter().map(|s| s.to_string()).collect();
        Self { known_tools }
    }

    /// Register additional tool names beyond the built-in list.
    pub fn add_known_tool(&mut self, name: &str) {
        self.known_tools.insert(name.to_string());
    }
}

impl Default for ConfigValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Validate a parsed TOML config and return all findings.
pub fn validate_config(config: &toml::Value) -> ValidationResult {
    let validator = ConfigValidator::new();
    validate_config_with(config, &validator)
}

/// Validate using a custom ConfigValidator (allows extra known tools).
pub fn validate_config_with(config: &toml::Value, validator: &ConfigValidator) -> ValidationResult {
    let mut result = ValidationResult::new();

    validate_telegram(&mut result, config);
    let provider_names = validate_providers(&mut result, config);
    validate_agents(&mut result, config, &provider_names, &validator.known_tools);
    validate_rate_limits(&mut result, config);
    validate_port(&mut result, config);
    validate_workspace(&mut result, config);
    validate_budgets(&mut result, config);
    validate_cron(&mut result, config);
    validate_cluster(&mut result, config);

    result
}

/// Print a formatted validation report to stdout.
pub fn print_report(result: &ValidationResult) {
    println!("=== Config Validation Report ===");
    println!();

    if result.is_valid() && result.warnings.is_empty() && result.infos.is_empty() {
        println!("All checks passed. No issues found.");
        return;
    }

    if !result.errors.is_empty() {
        println!("ERRORS ({}):", result.errors.len());
        for e in &result.errors {
            println!("  [ERROR] {}: {}", e.field, e.message);
        }
        println!();
    }

    if !result.warnings.is_empty() {
        println!("WARNINGS ({}):", result.warnings.len());
        for w in &result.warnings {
            println!("  [WARNING] {}: {}", w.field, w.message);
        }
        println!();
    }

    if !result.infos.is_empty() {
        println!("INFO ({}):", result.infos.len());
        for i in &result.infos {
            println!("  [INFO] {}: {}", i.field, i.message);
        }
        println!();
    }

    let status = if result.is_valid() { "PASS (with warnings)" } else { "FAIL" };
    println!(
        "Summary: {} error(s), {} warning(s), {} info(s) -- {}",
        result.errors.len(),
        result.warnings.len(),
        result.infos.len(),
        status,
    );
}

// ── Individual Validators ──────────────────────────────────────────────────────

/// Check that [telegram].bot_token exists.
fn validate_telegram(result: &mut ValidationResult, config: &toml::Value) {
    match config.get("telegram") {
        None => {
            result.add(ValidationError {
                severity: Severity::Warning,
                field: "telegram".to_string(),
                message: "No [telegram] section found. Telegram bot will not be available.".to_string(),
            });
        }
        Some(tg) => {
            match tg.get("bot_token") {
                None => {
                    result.add(ValidationError {
                        severity: Severity::Error,
                        field: "telegram.bot_token".to_string(),
                        message: "Required field 'bot_token' is missing in [telegram] section.".to_string(),
                    });
                }
                Some(token) => {
                    if let Some(s) = token.as_str() {
                        if s.is_empty() {
                            result.add(ValidationError {
                                severity: Severity::Error,
                                field: "telegram.bot_token".to_string(),
                                message: "bot_token is empty.".to_string(),
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Check that at least one provider is configured. Returns the set of configured provider names.
fn validate_providers(result: &mut ValidationResult, config: &toml::Value) -> HashSet<String> {
    let mut names = HashSet::new();

    match config.get("providers") {
        None => {
            result.add(ValidationError {
                severity: Severity::Error,
                field: "providers".to_string(),
                message: "No [providers] section found. At least one provider must be configured.".to_string(),
            });
        }
        Some(providers) => {
            if let Some(table) = providers.as_table() {
                if table.is_empty() {
                    result.add(ValidationError {
                        severity: Severity::Error,
                        field: "providers".to_string(),
                        message: "Providers section is empty. At least one provider must be configured.".to_string(),
                    });
                }
                for (name, _cfg) in table {
                    names.insert(name.clone());
                }
            }
        }
    }

    names
}

/// Validate agent configurations: provider references, tool names, circular delegation.
fn validate_agents(
    result: &mut ValidationResult,
    config: &toml::Value,
    provider_names: &HashSet<String>,
    known_tools: &HashSet<String>,
) {
    let agent_section = match config.get("agent") {
        Some(a) => a,
        None => {
            result.add(ValidationError {
                severity: Severity::Info,
                field: "agent".to_string(),
                message: "No [agent] section found. Default agents will be used.".to_string(),
            });
            return;
        }
    };

    let agents = match agent_section.as_table() {
        Some(t) => t,
        None => return,
    };

    // Collect agent names for circular delegation check
    let agent_names: HashSet<String> = agents.keys().cloned().collect();

    // Build delegation graph for cycle detection
    let mut delegation_graph: HashMap<String, Vec<String>> = HashMap::new();

    for (agent_name, agent_cfg) in agents {
        let prefix = format!("agent.{}", agent_name);

        // Check provider reference
        if let Some(provider_val) = agent_cfg.get("provider") {
            if let Some(provider_str) = provider_val.as_str() {
                if !provider_str.is_empty() && !provider_names.is_empty() && !provider_names.contains(provider_str) {
                    result.add(ValidationError {
                        severity: Severity::Error,
                        field: format!("{}.provider", prefix),
                        message: format!(
                            "Agent '{}' references provider '{}' which is not configured. Available: {:?}",
                            agent_name,
                            provider_str,
                            provider_names.iter().collect::<Vec<_>>()
                        ),
                    });
                }
            }
        }

        // Check tool names
        if let Some(tools_val) = agent_cfg.get("tools") {
            if let Some(tools_arr) = tools_val.as_array() {
                for (idx, tool_val) in tools_arr.iter().enumerate() {
                    if let Some(tool_name) = tool_val.as_str() {
                        if !known_tools.contains(tool_name) {
                            result.add(ValidationError {
                                severity: Severity::Warning,
                                field: format!("{}.tools[{}]", prefix, idx),
                                message: format!(
                                    "Tool '{}' in agent '{}' is not a recognized built-in tool.",
                                    tool_name, agent_name
                                ),
                            });
                        }
                    }
                }
            }
        }

        // Check budget
        if let Some(budget_val) = agent_cfg.get("daily_budget_usd") {
            if let Some(budget) = budget_val.as_float() {
                if budget < 0.0 {
                    result.add(ValidationError {
                        severity: Severity::Error,
                        field: format!("{}.daily_budget_usd", prefix),
                        message: format!(
                            "Agent '{}' has negative budget: {}. Budget must be non-negative.",
                            agent_name, budget
                        ),
                    });
                }
            } else if let Some(budget) = budget_val.as_integer() {
                if budget < 0 {
                    result.add(ValidationError {
                        severity: Severity::Error,
                        field: format!("{}.daily_budget_usd", prefix),
                        message: format!(
                            "Agent '{}' has negative budget: {}. Budget must be non-negative.",
                            agent_name, budget
                        ),
                    });
                }
            }
        }

        // Build delegation graph from subagents
        if let Some(subagents_val) = agent_cfg.get("subagents") {
            if let Some(subagents_arr) = subagents_val.as_array() {
                let mut targets = Vec::new();
                for sub_val in subagents_arr {
                    if let Some(sub_name) = sub_val.as_str() {
                        targets.push(sub_name.to_string());
                        // Warn if subagent not defined
                        if !agent_names.contains(sub_name) {
                            result.add(ValidationError {
                                severity: Severity::Warning,
                                field: format!("{}.subagents", prefix),
                                message: format!(
                                    "Agent '{}' references subagent '{}' which is not defined.",
                                    agent_name, sub_name
                                ),
                            });
                        }
                    }
                }
                delegation_graph.insert(agent_name.clone(), targets);
            }
        }
    }

    // Check for circular delegation
    detect_circular_delegation(result, &delegation_graph);
}

/// Detect cycles in the agent delegation graph using DFS.
fn detect_circular_delegation(
    result: &mut ValidationResult,
    graph: &HashMap<String, Vec<String>>,
) {
    let mut visited: HashSet<String> = HashSet::new();
    let mut in_stack: HashSet<String> = HashSet::new();

    for start_node in graph.keys() {
        if !visited.contains(start_node) {
            let mut stack: Vec<(String, usize)> = vec![(start_node.clone(), 0)];
            let mut path: Vec<String> = Vec::new();

            while let Some((node, child_idx)) = stack.last_mut() {
                if !in_stack.contains(node.as_str()) {
                    in_stack.insert(node.clone());
                    visited.insert(node.clone());
                    path.push(node.clone());
                }

                let children = graph.get(node.as_str());
                let has_next = children
                    .map(|c| *child_idx < c.len())
                    .unwrap_or(false);

                if has_next {
                    let children = children.unwrap();
                    let next = children[*child_idx].clone();
                    *child_idx += 1;

                    if in_stack.contains(&next) {
                        // Found a cycle
                        let cycle_start = path.iter().position(|n| n == &next).unwrap_or(0);
                        let cycle: Vec<_> = path[cycle_start..].to_vec();
                        result.add(ValidationError {
                            severity: Severity::Error,
                            field: "agent.subagents".to_string(),
                            message: format!(
                                "Circular delegation detected: {} -> {}",
                                cycle.join(" -> "),
                                next
                            ),
                        });
                    } else if !visited.contains(&next) {
                        stack.push((next, 0));
                    }
                } else {
                    let node = stack.pop().unwrap().0;
                    in_stack.remove(&node);
                    path.pop();
                }
            }
        }
    }
}

/// Validate rate limit values are positive.
fn validate_rate_limits(result: &mut ValidationResult, config: &toml::Value) {
    // Check security.rate_limit section
    if let Some(security) = config.get("security") {
        if let Some(rl) = security.get("rate_limit") {
            if let Some(max_actions) = rl.get("max_actions_per_hour") {
                if let Some(n) = max_actions.as_integer() {
                    if n <= 0 {
                        result.add(ValidationError {
                            severity: Severity::Error,
                            field: "security.rate_limit.max_actions_per_hour".to_string(),
                            message: format!("Rate limit must be a positive number, got {}.", n),
                        });
                    }
                }
            }
            if let Some(max_per_tool) = rl.get("max_per_tool_per_hour") {
                if let Some(n) = max_per_tool.as_integer() {
                    if n <= 0 {
                        result.add(ValidationError {
                            severity: Severity::Error,
                            field: "security.rate_limit.max_per_tool_per_hour".to_string(),
                            message: format!("Rate limit must be a positive number, got {}.", n),
                        });
                    }
                }
            }
        }
    }
}

/// Validate port number is in valid range (1-65535).
fn validate_port(result: &mut ValidationResult, config: &toml::Value) {
    if let Some(core) = config.get("core") {
        if let Some(port_val) = core.get("port") {
            if let Some(port) = port_val.as_integer() {
                if port < 1 || port > 65535 {
                    result.add(ValidationError {
                        severity: Severity::Error,
                        field: "core.port".to_string(),
                        message: format!("Port {} is out of valid range (1-65535).", port),
                    });
                }
            }
        }
    }
}

/// Validate workspace path exists or is creatable.
fn validate_workspace(result: &mut ValidationResult, config: &toml::Value) {
    if let Some(security) = config.get("security") {
        if let Some(ws_val) = security.get("workspace_dir") {
            if let Some(ws_str) = ws_val.as_str() {
                if ws_str.is_empty() {
                    result.add(ValidationError {
                        severity: Severity::Error,
                        field: "security.workspace_dir".to_string(),
                        message: "Workspace directory path is empty.".to_string(),
                    });
                    return;
                }
                // Expand ~ for checking
                let expanded = expand_home_simple(ws_str);
                let path = std::path::Path::new(&expanded);
                if !path.exists() {
                    // Check if parent exists (directory could be created)
                    if let Some(parent) = path.parent() {
                        if !parent.exists() {
                            result.add(ValidationError {
                                severity: Severity::Warning,
                                field: "security.workspace_dir".to_string(),
                                message: format!(
                                    "Workspace '{}' does not exist and parent directory does not exist either. It may fail to be created at startup.",
                                    ws_str
                                ),
                            });
                        } else {
                            result.add(ValidationError {
                                severity: Severity::Info,
                                field: "security.workspace_dir".to_string(),
                                message: format!(
                                    "Workspace '{}' does not exist yet but parent is valid. It will be created on startup.",
                                    ws_str
                                ),
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Validate budget values are non-negative (agent-level budgets are checked in validate_agents).
fn validate_budgets(result: &mut ValidationResult, config: &toml::Value) {
    // Check global budget if present at top level or under cost section
    for section_name in &["cost", "budget"] {
        if let Some(section) = config.get(*section_name) {
            for field in &["daily_budget_usd", "global_daily_budget_usd", "max_budget_usd"] {
                if let Some(val) = section.get(*field) {
                    let negative = if let Some(f) = val.as_float() {
                        f < 0.0
                    } else if let Some(i) = val.as_integer() {
                        i < 0
                    } else {
                        false
                    };
                    if negative {
                        result.add(ValidationError {
                            severity: Severity::Error,
                            field: format!("{}.{}", section_name, field),
                            message: "Budget value must be non-negative.".to_string(),
                        });
                    }
                }
            }
        }
    }
}

/// Basic validation of cron expressions.
/// Checks format: should have 5 or 6 space-separated fields, each containing
/// only valid cron characters (digits, *, /, -, comma, or named days/months).
fn validate_cron(result: &mut ValidationResult, config: &toml::Value) {
    if let Some(cron_section) = config.get("cron") {
        if let Some(table) = cron_section.as_table() {
            for (name, job) in table {
                if let Some(schedule) = job.get("schedule") {
                    if let Some(expr) = schedule.as_str() {
                        if !is_valid_cron_expression(expr) {
                            result.add(ValidationError {
                                severity: Severity::Error,
                                field: format!("cron.{}.schedule", name),
                                message: format!(
                                    "Invalid cron expression '{}'. Expected 5 or 6 space-separated fields.",
                                    expr
                                ),
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Validate cluster hub_url is a valid URL format if set.
fn validate_cluster(result: &mut ValidationResult, config: &toml::Value) {
    if let Some(cluster) = config.get("cluster") {
        if let Some(hub_url_val) = cluster.get("hub_url") {
            if let Some(hub_url) = hub_url_val.as_str() {
                if !hub_url.is_empty() && !is_valid_url(hub_url) {
                    result.add(ValidationError {
                        severity: Severity::Error,
                        field: "cluster.hub_url".to_string(),
                        message: format!(
                            "Cluster hub_url '{}' is not a valid URL. Expected format: http(s)://host:port",
                            hub_url
                        ),
                    });
                }
            }
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Simple ~ expansion for validation purposes.
fn expand_home_simple(path: &str) -> String {
    if path.starts_with("~/") || path.starts_with("~\\") {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".to_string());
        format!("{}/{}", home, &path[2..])
    } else if path == "~" {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".to_string())
    } else {
        path.to_string()
    }
}

/// Basic cron expression validity check.
/// Accepts 5-field (minute hour dom month dow) or 6-field (+ seconds/year) expressions.
fn is_valid_cron_expression(expr: &str) -> bool {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() < 5 || fields.len() > 7 {
        return false;
    }
    let valid_chars = |s: &str| -> bool {
        s.chars().all(|c| {
            c.is_ascii_digit()
                || c == '*'
                || c == '/'
                || c == '-'
                || c == ','
                || c == '?'
                || c == '#'
                || c == 'L'
                || c == 'W'
                || c.is_ascii_alphabetic()
        })
    };
    fields.iter().all(|f| !f.is_empty() && valid_chars(f))
}

/// Basic URL format validation.
fn is_valid_url(url: &str) -> bool {
    // Must start with http:// or https://
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return false;
    }
    let after_scheme = if url.starts_with("https://") {
        &url[8..]
    } else {
        &url[7..]
    };
    // Must have a host part (at least one character before optional port/path)
    if after_scheme.is_empty() {
        return false;
    }
    // Must not contain spaces
    if after_scheme.contains(' ') {
        return false;
    }
    true
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to build a TOML Value from a string.
    fn parse_toml(s: &str) -> toml::Value {
        s.parse::<toml::Value>().expect("invalid TOML in test")
    }

    #[test]
    fn test_valid_minimal_config() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123456:ABC"

            [providers.ollama]
            provider_type = "ollama"
            url = "http://localhost:11434"
        "#);
        let result = validate_config(&config);
        assert!(result.is_valid(), "minimal valid config should pass: {:?}", result.errors);
    }

    #[test]
    fn test_missing_telegram_section() {
        let config = parse_toml(r#"
            [providers.ollama]
            provider_type = "ollama"
        "#);
        let result = validate_config(&config);
        assert!(result.is_valid(), "missing telegram is a warning, not an error");
        assert!(!result.warnings.is_empty(), "should have a warning about missing telegram");
    }

    #[test]
    fn test_missing_bot_token() {
        let config = parse_toml(r#"
            [telegram]
            allowed_users = [123]

            [providers.ollama]
            provider_type = "ollama"
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid(), "missing bot_token should be an error");
        assert!(result.errors.iter().any(|e| e.field == "telegram.bot_token"));
    }

    #[test]
    fn test_empty_bot_token() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = ""

            [providers.ollama]
            provider_type = "ollama"
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid(), "empty bot_token should be an error");
        assert!(result.errors.iter().any(|e| e.field == "telegram.bot_token"));
    }

    #[test]
    fn test_no_providers() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid(), "no providers should be an error");
        assert!(result.errors.iter().any(|e| e.field == "providers"));
    }

    #[test]
    fn test_empty_providers() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers]
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid(), "empty providers section should be an error");
    }

    #[test]
    fn test_agent_references_invalid_provider() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [agent.master]
            provider = "nonexistent_provider"
            model = "gpt-4"
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e|
            e.field.contains("agent.master.provider")
            && e.message.contains("nonexistent_provider")
        ));
    }

    #[test]
    fn test_agent_references_valid_provider() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [agent.master]
            provider = "ollama"
            model = "llama3"
        "#);
        let result = validate_config(&config);
        assert!(result.is_valid(), "valid provider reference should pass: {:?}", result.errors);
    }

    #[test]
    fn test_agent_with_invalid_tool() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [agent.master]
            provider = "ollama"
            tools = ["shell", "file_read", "totally_fake_tool"]
        "#);
        let result = validate_config(&config);
        assert!(result.warnings.iter().any(|w|
            w.message.contains("totally_fake_tool")
        ), "unrecognized tool should produce a warning");
    }

    #[test]
    fn test_agent_with_valid_tools() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [agent.master]
            provider = "ollama"
            tools = ["shell", "file_read", "web_search", "delegate"]
        "#);
        let result = validate_config(&config);
        let tool_warnings: Vec<_> = result.warnings.iter()
            .filter(|w| w.field.contains("tools"))
            .collect();
        assert!(tool_warnings.is_empty(), "valid tools should not produce warnings: {:?}", tool_warnings);
    }

    #[test]
    fn test_negative_rate_limit() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [security.rate_limit]
            max_actions_per_hour = -5
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e|
            e.field.contains("max_actions_per_hour")
        ));
    }

    #[test]
    fn test_zero_rate_limit() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [security.rate_limit]
            max_per_tool_per_hour = 0
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid(), "zero rate limit should be an error");
    }

    #[test]
    fn test_positive_rate_limit() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [security.rate_limit]
            max_actions_per_hour = 100
            max_per_tool_per_hour = 50
        "#);
        let result = validate_config(&config);
        let rl_errors: Vec<_> = result.errors.iter()
            .filter(|e| e.field.contains("rate_limit"))
            .collect();
        assert!(rl_errors.is_empty(), "positive rate limits should be fine");
    }

    #[test]
    fn test_port_out_of_range_high() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [core]
            port = 70000
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e|
            e.field == "core.port" && e.message.contains("70000")
        ));
    }

    #[test]
    fn test_port_out_of_range_zero() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [core]
            port = 0
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.field == "core.port"));
    }

    #[test]
    fn test_port_valid() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [core]
            port = 7878
        "#);
        let result = validate_config(&config);
        let port_errors: Vec<_> = result.errors.iter()
            .filter(|e| e.field == "core.port")
            .collect();
        assert!(port_errors.is_empty());
    }

    #[test]
    fn test_negative_budget() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [agent.master]
            provider = "ollama"
            daily_budget_usd = -10.0
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e|
            e.field.contains("daily_budget_usd")
        ));
    }

    #[test]
    fn test_zero_budget_is_valid() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [agent.master]
            provider = "ollama"
            daily_budget_usd = 0.0
        "#);
        let result = validate_config(&config);
        let budget_errors: Vec<_> = result.errors.iter()
            .filter(|e| e.field.contains("budget"))
            .collect();
        assert!(budget_errors.is_empty(), "zero budget (no limit) should be valid");
    }

    #[test]
    fn test_global_negative_budget() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [cost]
            daily_budget_usd = -5.0
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e|
            e.field.contains("cost.daily_budget_usd")
        ));
    }

    #[test]
    fn test_invalid_cron_expression() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [cron.daily_report]
            schedule = "not a cron"
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e|
            e.field.contains("cron.daily_report.schedule")
        ));
    }

    #[test]
    fn test_valid_cron_expression() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [cron.daily_report]
            schedule = "0 8 * * *"
        "#);
        let result = validate_config(&config);
        let cron_errors: Vec<_> = result.errors.iter()
            .filter(|e| e.field.contains("cron"))
            .collect();
        assert!(cron_errors.is_empty(), "valid cron should not error: {:?}", cron_errors);
    }

    #[test]
    fn test_invalid_cluster_url() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [cluster]
            hub_url = "not_a_url"
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e|
            e.field == "cluster.hub_url"
        ));
    }

    #[test]
    fn test_valid_cluster_url() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [cluster]
            hub_url = "http://100.87.93.58:7878"
        "#);
        let result = validate_config(&config);
        let cluster_errors: Vec<_> = result.errors.iter()
            .filter(|e| e.field == "cluster.hub_url")
            .collect();
        assert!(cluster_errors.is_empty());
    }

    #[test]
    fn test_circular_delegation() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [agent.alpha]
            provider = "ollama"
            subagents = ["beta"]

            [agent.beta]
            provider = "ollama"
            subagents = ["alpha"]
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid(), "circular delegation should be an error");
        assert!(result.errors.iter().any(|e|
            e.field.contains("subagents") && e.message.contains("Circular")
        ));
    }

    #[test]
    fn test_no_circular_delegation() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [agent.master]
            provider = "ollama"
            subagents = ["coder"]

            [agent.coder]
            provider = "ollama"
        "#);
        let result = validate_config(&config);
        let cycle_errors: Vec<_> = result.errors.iter()
            .filter(|e| e.message.contains("Circular"))
            .collect();
        assert!(cycle_errors.is_empty(), "linear delegation should not trigger cycle detection");
    }

    #[test]
    fn test_self_delegation_cycle() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [agent.narcissist]
            provider = "ollama"
            subagents = ["narcissist"]
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid(), "self-delegation should be an error");
        assert!(result.errors.iter().any(|e|
            e.message.contains("Circular")
        ));
    }

    #[test]
    fn test_empty_workspace_dir() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [security]
            workspace_dir = ""
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e|
            e.field.contains("workspace_dir")
        ));
    }

    #[test]
    fn test_validation_result_total_count() {
        let mut result = ValidationResult::new();
        result.add(ValidationError {
            severity: Severity::Error,
            field: "a".to_string(),
            message: "err".to_string(),
        });
        result.add(ValidationError {
            severity: Severity::Warning,
            field: "b".to_string(),
            message: "warn".to_string(),
        });
        result.add(ValidationError {
            severity: Severity::Info,
            field: "c".to_string(),
            message: "info".to_string(),
        });
        assert_eq!(result.total_count(), 3);
        assert!(!result.is_valid());
    }

    #[test]
    fn test_validation_result_default() {
        let result = ValidationResult::default();
        assert!(result.is_valid());
        assert_eq!(result.total_count(), 0);
    }

    #[test]
    fn test_severity_display() {
        assert_eq!(format!("{}", Severity::Error), "ERROR");
        assert_eq!(format!("{}", Severity::Warning), "WARNING");
        assert_eq!(format!("{}", Severity::Info), "INFO");
    }

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
    }

    #[test]
    fn test_validation_error_display() {
        let err = ValidationError {
            severity: Severity::Error,
            field: "telegram.bot_token".to_string(),
            message: "Required field missing.".to_string(),
        };
        let display = format!("{}", err);
        assert!(display.contains("[ERROR]"));
        assert!(display.contains("telegram.bot_token"));
        assert!(display.contains("Required field missing."));
    }

    #[test]
    fn test_custom_known_tool() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [agent.master]
            provider = "ollama"
            tools = ["my_custom_tool"]
        "#);

        // Without custom tool: should warn
        let result_without = validate_config(&config);
        assert!(result_without.warnings.iter().any(|w| w.message.contains("my_custom_tool")));

        // With custom tool registered: no warning
        let mut validator = ConfigValidator::new();
        validator.add_known_tool("my_custom_tool");
        let result_with = validate_config_with(&config, &validator);
        let custom_warnings: Vec<_> = result_with.warnings.iter()
            .filter(|w| w.message.contains("my_custom_tool"))
            .collect();
        assert!(custom_warnings.is_empty(), "registered custom tool should not warn");
    }

    #[test]
    fn test_is_valid_cron_expression() {
        assert!(is_valid_cron_expression("0 8 * * *"));
        assert!(is_valid_cron_expression("*/5 * * * *"));
        assert!(is_valid_cron_expression("0 0 1 1 *"));
        assert!(is_valid_cron_expression("0 9 * * MON"));
        assert!(is_valid_cron_expression("0 0 * * * 2026"));
        assert!(!is_valid_cron_expression("not a cron"));
        assert!(!is_valid_cron_expression(""));
        assert!(!is_valid_cron_expression("* *"));
    }

    #[test]
    fn test_is_valid_url() {
        assert!(is_valid_url("http://localhost:7878"));
        assert!(is_valid_url("https://example.com"));
        assert!(is_valid_url("http://100.87.93.58:7878"));
        assert!(is_valid_url("https://api.example.com/v1"));
        assert!(!is_valid_url("ftp://files.example.com"));
        assert!(!is_valid_url("not_a_url"));
        assert!(!is_valid_url("http://"));
        assert!(!is_valid_url(""));
    }

    #[test]
    fn test_print_report_no_issues() {
        let result = ValidationResult::new();
        // Should not panic
        print_report(&result);
    }

    #[test]
    fn test_print_report_with_issues() {
        let mut result = ValidationResult::new();
        result.add(ValidationError {
            severity: Severity::Error,
            field: "test.field".to_string(),
            message: "Something is wrong.".to_string(),
        });
        result.add(ValidationError {
            severity: Severity::Warning,
            field: "test.warn".to_string(),
            message: "Be careful.".to_string(),
        });
        // Should not panic
        print_report(&result);
    }

    #[test]
    fn test_multiple_agents_mixed_validity() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [providers.gemini]
            provider_type = "gemini"

            [agent.master]
            provider = "ollama"
            tools = ["shell", "file_read"]

            [agent.researcher]
            provider = "gemini"
            tools = ["web_search"]

            [agent.broken]
            provider = "nonexistent"
            tools = ["fake_tool"]
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid());
        // Should have error for broken agent's provider
        assert!(result.errors.iter().any(|e|
            e.field.contains("agent.broken.provider")
        ));
        // Should have warning for broken agent's tool
        assert!(result.warnings.iter().any(|w|
            w.message.contains("fake_tool")
        ));
    }

    #[test]
    fn test_three_node_circular_delegation() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [agent.a]
            provider = "ollama"
            subagents = ["b"]

            [agent.b]
            provider = "ollama"
            subagents = ["c"]

            [agent.c]
            provider = "ollama"
            subagents = ["a"]
        "#);
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e|
            e.message.contains("Circular")
        ));
    }

    #[test]
    fn test_subagent_references_undefined_agent() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [agent.master]
            provider = "ollama"
            subagents = ["ghost_agent"]
        "#);
        let result = validate_config(&config);
        assert!(result.warnings.iter().any(|w|
            w.message.contains("ghost_agent") && w.message.contains("not defined")
        ));
    }

    #[test]
    fn test_cluster_url_with_https() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"

            [cluster]
            hub_url = "https://cluster.example.com:443"
        "#);
        let result = validate_config(&config);
        let cluster_errors: Vec<_> = result.errors.iter()
            .filter(|e| e.field == "cluster.hub_url")
            .collect();
        assert!(cluster_errors.is_empty(), "HTTPS URL should be valid");
    }

    #[test]
    fn test_port_boundary_values() {
        // Port 1 is valid
        let config1 = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"
            [providers.ollama]
            provider_type = "ollama"
            [core]
            port = 1
        "#);
        assert!(validate_config(&config1).errors.iter().all(|e| e.field != "core.port"));

        // Port 65535 is valid
        let config2 = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"
            [providers.ollama]
            provider_type = "ollama"
            [core]
            port = 65535
        "#);
        assert!(validate_config(&config2).errors.iter().all(|e| e.field != "core.port"));

        // Port 65536 is invalid
        let config3 = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"
            [providers.ollama]
            provider_type = "ollama"
            [core]
            port = 65536
        "#);
        assert!(validate_config(&config3).errors.iter().any(|e| e.field == "core.port"));
    }

    #[test]
    fn test_no_agent_section_is_info() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"

            [providers.ollama]
            provider_type = "ollama"
        "#);
        let result = validate_config(&config);
        assert!(result.is_valid());
        assert!(result.infos.iter().any(|i|
            i.field == "agent" && i.message.contains("Default agents")
        ));
    }

    #[test]
    fn test_cron_with_named_day() {
        let config = parse_toml(r#"
            [telegram]
            bot_token = "123:ABC"
            [providers.ollama]
            provider_type = "ollama"
            [cron.weekly]
            schedule = "0 10 * * MON"
        "#);
        let result = validate_config(&config);
        let cron_errors: Vec<_> = result.errors.iter()
            .filter(|e| e.field.contains("cron"))
            .collect();
        assert!(cron_errors.is_empty());
    }
}
