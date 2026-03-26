// Tool system — inspired by ZeroClaw's Tool trait
// Provides: Tool trait, ToolResult, ToolRegistry, built-in tools
// Rate limiting + credential scrubbing (ZeroClaw-inspired)

pub mod shell;
pub mod file_read;
pub mod file_write;
pub mod file_edit;
pub mod web_search;
pub mod http_request;
pub mod memory_tools;
pub mod glob_search;
pub mod content_search;
pub mod delegate;
pub mod delegate_to_provider;
pub mod ai_code;
pub mod computer_use;
pub mod browser;
pub mod vision;
pub mod email;
pub mod twitter;
pub mod blog_publish;
pub mod pdf_export;
pub mod run_hand;
pub mod skeleton_generate;
pub mod stripe;
pub mod render_deploy;
pub mod scaffold_saas;
pub mod cli_anything;
pub mod slack;
pub mod discord;
pub mod line_notify;
pub mod whatsapp;
pub mod translate;
pub mod json_transform;
pub mod csv_parse;
pub mod summarize;
pub mod image_generate;
pub mod docx_export;
pub mod xlsx_export;
pub mod tts;
pub mod email_receive;
pub mod video_compose;
pub mod youtube_upload;
pub mod music_generate;
pub mod knowledge_import;
pub mod calendar;
pub mod data_analysis;
pub mod screenshot;
pub mod qr_generate;
pub mod rss_reader;
pub mod archive_extract;
pub mod clipboard;
pub mod system_info;
pub mod weather;
pub mod calculator;
pub mod notification_center;
pub mod error_middleware;
pub mod github;
pub mod db_query;
pub mod cron_manage;
pub mod payment_tracker;
pub mod input_sanitizer;
pub mod code_evolution;
pub mod shell_session;

use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::warn;

use crate::audit_log::{AuditLogger, ActionType, Outcome, risk_level_for_tool};
use crate::output_stash::OutputStash;
use crate::tools::input_sanitizer::sanitize_tool_args;

// ── Rate Limiting ─────────────────────────────────────────────────────────────

/// Sliding-window action tracker (inspired by ZeroClaw's ActionTracker)
#[derive(Debug)]
pub struct ActionTracker {
    /// Timestamps of recent actions within the window
    actions: Mutex<Vec<Instant>>,
    /// Window duration (default: 1 hour)
    window: Duration,
}

impl ActionTracker {
    pub fn new(window: Duration) -> Self {
        Self {
            actions: Mutex::new(Vec::new()),
            window,
        }
    }

    /// Record an action and return the count in the current window
    pub fn record(&self) -> usize {
        let mut actions = self.actions.lock().unwrap();
        let now = Instant::now();
        // Use checked_sub to avoid overflow on Windows where Instant epoch may be recent
        if let Some(cutoff) = now.checked_sub(self.window) {
            actions.retain(|t| *t > cutoff);
        }
        actions.push(now);
        actions.len()
    }

    /// Get the count of actions in the current window (non-recording)
    pub fn count(&self) -> usize {
        let mut actions = self.actions.lock().unwrap();
        let now = Instant::now();
        if let Some(cutoff) = now.checked_sub(self.window) {
            actions.retain(|t| *t > cutoff);
        }
        actions.len()
    }

    /// Reset the tracker
    pub fn reset(&self) {
        let mut actions = self.actions.lock().unwrap();
        actions.clear();
    }
}

impl Default for ActionTracker {
    fn default() -> Self {
        Self::new(Duration::from_secs(3600)) // 1 hour window
    }
}

/// Rate limit configuration
#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitConfig {
    /// Max tool calls per hour (global across all tools)
    #[serde(default = "default_max_actions")]
    pub max_actions_per_hour: u32,
    /// Max calls per tool per hour (per-tool limit)
    #[serde(default = "default_max_per_tool")]
    pub max_per_tool_per_hour: u32,
}

fn default_max_actions() -> u32 { 60 }
fn default_max_per_tool() -> u32 { 30 }

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_actions_per_hour: default_max_actions(),
            max_per_tool_per_hour: default_max_per_tool(),
        }
    }
}

// ── Credential Scrubbing ──────────────────────────────────────────────────────

/// Regex patterns for detecting sensitive key-value pairs in tool output
static SENSITIVE_KV_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?i)(token|api[_\-]?key|password|passwd|secret|user[_\-]?key|bearer|credential|auth[_\-]?token|access[_\-]?key|private[_\-]?key)["']?\s*[:=]\s*(?:"([^"]{8,})"|'([^']{8,})'|([a-zA-Z0-9_\-\.\/\+]{8,}))"#
    ).unwrap()
});

/// Regex for common API key formats (standalone, not key-value)
static API_KEY_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // AWS Access Key ID
        Regex::new(r"(?:^|[^a-zA-Z0-9])AKIA[0-9A-Z]{16}(?:[^a-zA-Z0-9]|$)").unwrap(),
        // Stripe secret key
        Regex::new(r"sk_live_[a-zA-Z0-9]{20,}").unwrap(),
        // OpenAI API key
        Regex::new(r"sk-[a-zA-Z0-9]{32,}").unwrap(),
        // GitHub token
        Regex::new(r"gh[ps]_[a-zA-Z0-9]{36,}").unwrap(),
        // Generic Bearer token
        Regex::new(r"Bearer\s+[a-zA-Z0-9\-_\.]{20,}").unwrap(),
        // Private key block
        Regex::new(r"-----BEGIN (?:RSA |EC |DSA )?PRIVATE KEY-----").unwrap(),
        // JWT token (3 base64 segments with dots)
        Regex::new(r"eyJ[a-zA-Z0-9\-_]+\.eyJ[a-zA-Z0-9\-_]+\.[a-zA-Z0-9\-_]+").unwrap(),
    ]
});

/// Scrub credentials from text output (inspired by ZeroClaw's scrub_credentials)
pub fn scrub_credentials(input: &str) -> String {
    let mut output = SENSITIVE_KV_REGEX.replace_all(input, |caps: &regex::Captures| {
        let key = caps.get(1).map(|m| m.as_str()).unwrap_or("key");
        // Find the actual value (could be in group 2, 3, or 4)
        let val = caps.get(2)
            .or_else(|| caps.get(3))
            .or_else(|| caps.get(4))
            .map(|m| m.as_str())
            .unwrap_or("");
        let prefix = if val.len() > 4 { &val[..4] } else { "" };
        format!("{}: {}****[REDACTED]", key, prefix)
    }).to_string();

    // Scrub standalone API key patterns
    for pattern in API_KEY_PATTERNS.iter() {
        output = pattern.replace_all(&output, "[REDACTED_KEY]").to_string();
    }

    output
}

// ── Tool Types ────────────────────────────────────────────────────────────────

/// Result of a tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
}

/// Tool specification for LLM function calling
#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Tool trait — any executable tool implements this
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;
    async fn execute(&self, args: Value) -> Result<ToolResult>;

    /// Lightweight pre-execution check. Return Err to block execution early.
    /// Default implementation passes all checks.
    fn preflight(&self, _args: &Value) -> Result<()> {
        Ok(())
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}

/// Security config for tool execution
#[derive(Debug, Clone, Deserialize)]
pub struct SecurityConfig {
    #[serde(default = "default_workspace")]
    pub workspace_dir: String,
    #[serde(default = "default_true")]
    pub workspace_only: bool,
    #[serde(default = "default_allowed_commands")]
    pub allowed_commands: Vec<String>,
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
    /// Additional paths allowed when workspace_only is true (e.g., src/ for self-modify)
    #[serde(default)]
    pub allowed_paths: Vec<String>,
}

fn default_workspace() -> String {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    format!("{}/.phantom-mesh/workspace", home)
}

fn default_true() -> bool { true }

pub(crate) fn default_allowed_commands() -> Vec<String> {
    vec![
        "git", "python", "python3", "pip", "pip3",
        "cargo", "npm", "node", "npx",
        "ls", "dir", "cat", "head", "tail", "grep", "find", "echo",
        "mkdir", "cp", "mv", "pwd", "which", "where", "wc", "sort", "tree",
        "date", "time", "whoami", "hostname",
        "sqlite3", "jq",
    ].into_iter().map(String::from).collect()
}

// ── Path Normalization (ACI poka-yoke) ───────────────────────────────────────

/// Expand ~ or ~/ to the user's home directory.
pub(crate) fn expand_home(path: &str) -> String {
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

/// Normalize a file path that an LLM might produce incorrectly.
/// Fixes: ~/ expansion, /home/user/ → Windows home, duplicate workspace prefix, forward slashes.
pub(crate) fn normalize_llm_path(path: &str, workspace: &std::path::Path) -> String {
    let mut p = path.to_string();

    // 1. Expand ~/
    p = expand_home(&p);

    // 2. Fix Linux-style /home/user/ on Windows
    #[cfg(target_os = "windows")]
    {
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        let username = home.rsplit(['\\', '/']).next().unwrap_or("");
        if !username.is_empty() {
            let linux_home = format!("/home/{}/", username);
            if p.starts_with(&linux_home) {
                p = format!("{}/{}", home, &p[linux_home.len()..]);
            }
        }
    }

    // 3. Remove duplicate workspace prefix (workspace/~/.phantom-mesh/workspace/X → workspace/X)
    let ws_str = workspace.to_string_lossy().replace('\\', "/");
    let _ws_suffix = if ws_str.ends_with('/') { &ws_str } else { &format!("{}/", ws_str) };
    // Check if path contains workspace path embedded after workspace join
    if p.replace('\\', "/").matches(&ws_str.replace('\\', "/")).count() > 1 {
        // e.g., ~/.phantom-mesh/workspace/~/.phantom-mesh/workspace/file.py
        // Keep only the last occurrence
        if let Some(last_idx) = p.replace('\\', "/").rfind(&ws_str.replace('\\', "/")) {
            p = p[last_idx..].to_string();
        }
    }

    // 4. Normalize slashes to platform style
    #[cfg(target_os = "windows")]
    {
        // Keep forward slashes (Rust/Git handle them fine on Windows)
    }

    p
}

/// Normalize a shell command's embedded paths.
/// Expands ~ references inside command strings.
pub(crate) fn normalize_shell_command(command: &str) -> String {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());

    // Replace ~/ with home dir — this is the #1 LLM mistake on Windows
    // Safe: ~/ in shell commands virtually always means home directory
    if command.contains("~/") {
        command.replace("~/", &format!("{}/", home))
    } else {
        command.to_string()
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            workspace_dir: default_workspace(),
            workspace_only: true,
            allowed_commands: default_allowed_commands(),
            rate_limit: RateLimitConfig::default(),
            allowed_paths: Vec::new(),
        }
    }
}

impl SecurityConfig {
    pub fn workspace_path(&self) -> PathBuf {
        PathBuf::from(expand_home(&self.workspace_dir))
    }

    /// Check if a path is within workspace OR any allowed_paths entry.
    pub fn is_allowed_path(&self, path: &std::path::Path) -> bool {
        let workspace = self.workspace_path();
        let ws_canonical = workspace.canonicalize().unwrap_or(workspace);
        if path.starts_with(&ws_canonical) {
            return true;
        }
        for allowed in &self.allowed_paths {
            let allowed_path = PathBuf::from(allowed);
            let allowed_canonical = allowed_path.canonicalize().unwrap_or(allowed_path);
            if path.starts_with(&allowed_canonical) {
                return true;
            }
        }
        false
    }
}

// ── Tool Registry ─────────────────────────────────────────────────────────────

/// Registry of available tools — with rate limiting and credential scrubbing
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
    workspace_dir: String,
    /// Global action tracker (all tools combined)
    global_tracker: ActionTracker,
    /// Per-tool action trackers
    tool_trackers: Mutex<HashMap<String, ActionTracker>>,
    /// Rate limit config
    rate_limit: RateLimitConfig,
    /// Whether to scrub credentials from tool output
    scrub_enabled: bool,
    /// Optional audit logger for recording tool executions
    audit_logger: Option<Arc<AuditLogger>>,
}

impl ToolRegistry {
    /// Create registry with default tools (no search API keys)
    pub fn new(security: SecurityConfig) -> Self {
        Self::new_with_search(security, web_search::SearchConfig::default())
    }

    /// Create registry with search API configuration
    pub fn new_with_search(security: SecurityConfig, search_config: web_search::SearchConfig) -> Self {
        let mut tools: HashMap<String, Box<dyn Tool>> = HashMap::new();

        // Ensure workspace exists
        let ws = security.workspace_path();
        let _ = std::fs::create_dir_all(&ws);

        // Shared file snapshots for TOCTOU protection between file_read and file_edit
        let file_snapshots: file_read::FileSnapshots = Arc::new(Mutex::new(HashMap::new()));

        let session_manager = Arc::new(shell_session::ShellSessionManager::new(
            std::path::PathBuf::from(&security.workspace_dir)
        ));
        tools.insert("shell".to_string(), Box::new(shell::ShellTool::new_with_sessions(security.clone(), session_manager)));
        tools.insert("file_read".to_string(), Box::new(file_read::FileReadTool::new_with_snapshots(security.clone(), file_snapshots.clone())));
        let workspace_dir = security.workspace_dir.clone();
        let rate_limit = security.rate_limit.clone();
        let allowed_paths = security.allowed_paths.clone();
        tools.insert("file_write".to_string(), Box::new(file_write::FileWriteTool::new(security)));
        tools.insert("web_search".to_string(), Box::new(web_search::WebSearchTool::new(search_config)));

        // Sprint 2 tools — inherit allowed_paths from main security config
        let sec_for_tools = SecurityConfig {
            workspace_dir: workspace_dir.clone(),
            rate_limit: rate_limit.clone(),
            allowed_paths: allowed_paths,
            ..SecurityConfig::default()
        };
        tools.insert("file_edit".to_string(), Box::new(file_edit::FileEditTool::new_with_snapshots(sec_for_tools.clone(), file_snapshots.clone())));
        tools.insert("http_request".to_string(), Box::new(http_request::HttpRequestTool::new(vec![])));
        tools.insert("glob_search".to_string(), Box::new(glob_search::GlobSearchTool::new(sec_for_tools.clone())));
        tools.insert("content_search".to_string(), Box::new(content_search::ContentSearchTool::new(sec_for_tools)));
        tools.insert("browser".to_string(), Box::new(browser::BrowserTool::new()));
        tools.insert("video_compose".to_string(), Box::new(video_compose::VideoComposeTool::new()));
        tools.insert("youtube_upload".to_string(), Box::new(youtube_upload::YouTubeUploadTool::new()));
        tools.insert("music_generate".to_string(), Box::new(music_generate::MusicGenerateTool::new()));
        tools.insert("knowledge_import".to_string(), Box::new(knowledge_import::KnowledgeImportTool::new()));
        tools.insert("calendar".to_string(), Box::new(calendar::CalendarTool::new()));
        tools.insert("data_analysis".to_string(), Box::new(data_analysis::DataAnalysisTool::new()));
        tools.insert("screenshot".to_string(), Box::new(screenshot::ScreenshotTool::new()));
        tools.insert("qr_generate".to_string(), Box::new(qr_generate::QrGenerateTool::new()));
        tools.insert("rss_reader".to_string(), Box::new(rss_reader::RssReaderTool::new()));
        tools.insert("archive_extract".to_string(), Box::new(archive_extract::ArchiveExtractTool::new()));
        tools.insert("clipboard".to_string(), Box::new(clipboard::ClipboardTool::new()));
        tools.insert("system_info".to_string(), Box::new(system_info::SystemInfoTool::new()));
        tools.insert("weather".to_string(), Box::new(weather::WeatherTool::new()));
        tools.insert("calculator".to_string(), Box::new(calculator::CalculatorTool::new()));
        tools.insert("notification_center".to_string(), Box::new(notification_center::NotificationCenterTool::new()));
        tools.insert("code_evolution".to_string(), Box::new(code_evolution::CodeEvolutionTool::new(workspace_dir.clone())));

        // Delegate tools — register with default (empty) dependencies so they appear in
        // names()/specs() even before register_delegate_tools() is called with the real
        // AgentRuntime/LlmRouter. The main.rs daemon path overwrites these entries with
        // fully-wired instances via register_delegate_tools().
        let default_runtime = Arc::new(crate::agent_runtime::AgentRuntime::new("").unwrap());
        let default_router = Arc::new(crate::llm_router::LlmRouter::new("").unwrap());
        let default_sub_registry = Arc::new(Self::_bare_registry(&workspace_dir, &rate_limit));
        tools.insert("delegate".to_string(), Box::new(delegate::DelegateTool::new(
            default_runtime.clone(),
            default_router.clone(),
            default_sub_registry.clone(),
        )));
        tools.insert("delegate_to_provider".to_string(), Box::new(delegate_to_provider::DelegateToProviderTool::new(
            default_runtime,
            default_router,
            default_sub_registry,
        )));

        Self {
            tools,
            workspace_dir,
            global_tracker: ActionTracker::default(),
            tool_trackers: Mutex::new(HashMap::new()),
            rate_limit,
            scrub_enabled: true,
            audit_logger: None,
        }
    }

    /// Create a minimal, empty registry (no tools) for use as a sub-agent registry
    /// placeholder. Avoids infinite recursion when constructing delegate tools inside
    /// new_with_search.
    fn _bare_registry(workspace_dir: &str, rate_limit: &RateLimitConfig) -> Self {
        Self {
            tools: HashMap::new(),
            workspace_dir: workspace_dir.to_string(),
            global_tracker: ActionTracker::default(),
            tool_trackers: Mutex::new(HashMap::new()),
            rate_limit: rate_limit.clone(),
            scrub_enabled: true,
            audit_logger: None,
        }
    }

    /// Set the audit logger for recording tool executions.
    pub fn set_audit_logger(&mut self, logger: Arc<AuditLogger>) {
        self.audit_logger = Some(logger);
    }

    pub fn workspace_dir(&self) -> &str {
        &self.workspace_dir
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.values().map(|t| t.spec()).collect()
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.tools.keys().cloned().collect();
        names.sort();
        names
    }

    /// Register an additional tool (used for delegate which needs Arcs created after init)
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, tool);
    }

    /// Register the delegate tools that require Arcs created after initial registry construction.
    ///
    /// This must be called after the AgentRuntime, LlmRouter, and a sub-agent ToolRegistry
    /// have been created, since the delegate tools hold Arc references to all three.
    ///
    /// `subagent_registry` should be a base-tools-only registry (no delegate tool) to prevent
    /// infinite delegation loops.
    pub fn register_delegate_tools(
        &mut self,
        agent_runtime: Arc<crate::agent_runtime::AgentRuntime>,
        llm_router: Arc<crate::llm_router::LlmRouter>,
        subagent_registry: Arc<ToolRegistry>,
    ) {
        self.register(Box::new(delegate::DelegateTool::new(
            agent_runtime.clone(),
            llm_router.clone(),
            subagent_registry.clone(),
        )));
        self.register(Box::new(delegate_to_provider::DelegateToProviderTool::new(
            agent_runtime,
            llm_router,
            subagent_registry,
        )));
    }

    /// Check if a tool call is within rate limits
    pub fn check_rate_limit(&self, tool_name: &str) -> Result<(), String> {
        // Check global limit
        let global_count = self.global_tracker.count();
        if global_count >= self.rate_limit.max_actions_per_hour as usize {
            return Err(format!(
                "Global rate limit exceeded: {}/{} actions per hour",
                global_count, self.rate_limit.max_actions_per_hour
            ));
        }

        // Check per-tool limit
        let mut trackers = self.tool_trackers.lock().unwrap();
        let tracker = trackers
            .entry(tool_name.to_string())
            .or_insert_with(ActionTracker::default);
        let tool_count = tracker.count();
        if tool_count >= self.rate_limit.max_per_tool_per_hour as usize {
            return Err(format!(
                "Tool '{}' rate limit exceeded: {}/{} per hour",
                tool_name, tool_count, self.rate_limit.max_per_tool_per_hour
            ));
        }

        Ok(())
    }

    /// Record a tool call (after successful execution)
    pub fn record_tool_call(&self, tool_name: &str) {
        self.global_tracker.record();
        let mut trackers = self.tool_trackers.lock().unwrap();
        let tracker = trackers
            .entry(tool_name.to_string())
            .or_insert_with(ActionTracker::default);
        tracker.record();
    }

    /// Execute a tool with rate limiting, credential scrubbing, audit logging, and structured error classification.
    pub async fn execute_tool(&self, tool_name: &str, args: Value) -> Result<ToolResult> {
        use crate::tools::error_middleware::{ToolError, classify_tool_error};

        // 1. Check rate limit
        if let Err(msg) = self.check_rate_limit(tool_name) {
            let tool_err = ToolError::rate_limited(tool_name, &msg);
            warn!("ToolError [{}]: {} (retryable={}, retry_after={:?})",
                tool_err.category, tool_err.message, tool_err.retryable, tool_err.retry_after_secs);
            return Ok(ToolResult {
                success: false,
                output: format!("Rate limit exceeded: {}", msg),
            });
        }

        // 1b. Sanitize tool arguments (path traversal, shell injection, SQL injection)
        let args = match sanitize_tool_args(tool_name, &args) {
            Ok(sanitized) => sanitized,
            Err(reason) => {
                warn!("Input sanitizer blocked tool '{}': {}", tool_name, reason);
                return Ok(ToolResult {
                    success: false,
                    output: format!("Input validation failed: {}", reason),
                });
            }
        };

        // 2. Get tool and run preflight check
        let tool = match self.tools.get(tool_name) {
            Some(t) => t,
            None => {
                let tool_err = ToolError::not_found(tool_name);
                warn!("ToolError [{}]: {}", tool_err.category, tool_err.message);
                return Err(anyhow::anyhow!("Unknown tool: {}", tool_name));
            }
        };

        if let Err(e) = tool.preflight(&args) {
            let tool_err = ToolError::preflight_failed(tool_name, &e.to_string());
            warn!("ToolError [{}]: {}", tool_err.category, tool_err.message);
            // Audit the preflight failure
            if let Some(ref audit) = self.audit_logger {
                let action_type = action_type_for_tool(tool_name);
                let target = extract_target_from_args(tool_name, &args);
                let _ = audit.log_action(
                    "system",
                    action_type,
                    Some(tool_name),
                    target.as_deref(),
                    Some(serde_json::json!({"error": e.to_string(), "preflight": true, "category": tool_err.category.to_string()})),
                    Outcome::Failure,
                    None,
                    risk_level_for_tool(tool_name),
                ).await;
            }
            return Ok(ToolResult {
                success: false,
                output: format!("Preflight check failed: {}", e),
            });
        }

        let result = match tool.execute(args.clone()).await {
            Ok(r) => r,
            Err(e) => {
                let classified = classify_tool_error(tool_name, &e.to_string());
                warn!("ToolError [{}]: {} (retryable={})",
                    classified.category, classified.message, classified.retryable);
                return Err(e);
            }
        };

        // 3. Record the action
        self.record_tool_call(tool_name);

        // 4. Audit log the tool execution
        if let Some(ref audit) = self.audit_logger {
            let action_type = action_type_for_tool(tool_name);
            let target = extract_target_from_args(tool_name, &args);
            let outcome = if result.success { Outcome::Success } else { Outcome::Failure };
            let details = serde_json::json!({
                "output_len": result.output.len(),
            });
            let _ = audit.log_action(
                "system",
                action_type,
                Some(tool_name),
                target.as_deref(),
                Some(details),
                outcome,
                None,
                risk_level_for_tool(tool_name),
            ).await;
        }

        // 5. Scrub credentials from output
        let mut final_output = if self.scrub_enabled {
            scrub_credentials(&result.output)
        } else {
            result.output.clone()
        };

        // 6. Stash large outputs (> 32 000 chars ≈ 8 000 tokens) to disk
        const STASH_THRESHOLD: usize = 32_000;
        if final_output.len() > STASH_THRESHOLD {
            let stash = OutputStash::new();
            match stash.stash(&final_output) {
                Ok(handle) => {
                    warn!(
                        "Tool '{}' produced {} chars — stashed to disk",
                        tool_name,
                        final_output.len()
                    );
                    final_output = handle;
                }
                Err(e) => {
                    // Stash failure is non-fatal: truncate with a note instead
                    warn!("OutputStash failed for '{}': {}", tool_name, e);
                    final_output.truncate(STASH_THRESHOLD);
                    final_output.push_str("\n[... output truncated — stash unavailable ...]");
                }
            }
        }

        Ok(ToolResult {
            success: result.success,
            output: final_output,
        })
    }

    /// Get rate limit stats for monitoring
    pub fn rate_limit_stats(&self) -> HashMap<String, usize> {
        let mut stats = HashMap::new();
        stats.insert("global".to_string(), self.global_tracker.count());
        let trackers = self.tool_trackers.lock().unwrap();
        for (name, tracker) in trackers.iter() {
            stats.insert(name.clone(), tracker.count());
        }
        stats
    }
}

// ── Audit Helpers ─────────────────────────────────────────────────────────────

/// Map a tool name to the most appropriate ActionType for audit logging.
fn action_type_for_tool(tool_name: &str) -> ActionType {
    match tool_name {
        "shell" => ActionType::ShellCommand,
        "file_write" | "file_edit" => ActionType::FileWrite,
        "email" | "email_send" | "twitter" | "blog_publish" | "slack_send" | "discord_send"
        | "line_send" | "whatsapp_send" => ActionType::ExternalSend,
        "pdf_export" | "docx_export" | "xlsx_export" | "csv_parse" => ActionType::DataExport,
        _ => ActionType::ToolExecution,
    }
}

/// Extract a human-readable target from tool arguments for audit context.
fn extract_target_from_args(tool_name: &str, args: &Value) -> Option<String> {
    match tool_name {
        "shell" => args.get("command").and_then(|v| v.as_str()).map(|s| {
            if s.len() > 200 {
                format!("{}...", &s[..s.char_indices().nth(200).map(|(i, _)| i).unwrap_or(s.len())])
            } else {
                s.to_string()
            }
        }),
        "file_read" | "file_write" | "file_edit" => {
            args.get("path").and_then(|v| v.as_str()).map(String::from)
        }
        "web_search" => args.get("query").and_then(|v| v.as_str()).map(String::from),
        "http_request" => args.get("url").and_then(|v| v.as_str()).map(String::from),
        "email" | "email_send" => args.get("to").and_then(|v| v.as_str()).map(String::from),
        "email_receive" => {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("check");
            let folder = args.get("folder").and_then(|v| v.as_str()).unwrap_or("INBOX");
            Some(format!("{}:{}", action, folder))
        }
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_tracker_records() {
        let tracker = ActionTracker::new(Duration::from_secs(60));
        assert_eq!(tracker.count(), 0);
        assert_eq!(tracker.record(), 1);
        assert_eq!(tracker.record(), 2);
        assert_eq!(tracker.count(), 2);
    }

    #[test]
    fn test_action_tracker_reset() {
        let tracker = ActionTracker::new(Duration::from_secs(60));
        tracker.record();
        tracker.record();
        assert_eq!(tracker.count(), 2);
        tracker.reset();
        assert_eq!(tracker.count(), 0);
    }

    #[test]
    fn test_rate_limit_check() {
        let security = SecurityConfig {
            rate_limit: RateLimitConfig {
                max_actions_per_hour: 3,
                max_per_tool_per_hour: 2,
            },
            ..Default::default()
        };
        let registry = ToolRegistry::new(security);

        // First 2 calls should pass
        assert!(registry.check_rate_limit("shell").is_ok());
        registry.record_tool_call("shell");
        assert!(registry.check_rate_limit("shell").is_ok());
        registry.record_tool_call("shell");

        // 3rd call for same tool should fail (per-tool limit = 2)
        assert!(registry.check_rate_limit("shell").is_err());

        // Different tool should still work (global = 2/3)
        assert!(registry.check_rate_limit("file_read").is_ok());
        registry.record_tool_call("file_read");

        // Now global limit (3) is hit
        assert!(registry.check_rate_limit("file_read").is_err());
    }

    #[test]
    fn test_scrub_kv_patterns() {
        let input = r#"api_key = "sk-abcdefghijklmnop""#;
        let scrubbed = scrub_credentials(input);
        assert!(scrubbed.contains("[REDACTED]"));
        assert!(!scrubbed.contains("abcdefghijklmnop"));
    }

    #[test]
    fn test_scrub_token_pattern() {
        let input = "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abc123def456";
        let scrubbed = scrub_credentials(input);
        assert!(!scrubbed.contains("eyJhbGciOiJIUzI1NiJ9"));
    }

    #[test]
    fn test_scrub_aws_key() {
        let input = "Found key: AKIAIOSFODNN7EXAMPLE in config";
        let scrubbed = scrub_credentials(input);
        assert!(scrubbed.contains("[REDACTED_KEY]"));
        assert!(!scrubbed.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn test_scrub_openai_key() {
        let input = "key is sk-abc123def456ghi789jkl012mno345pqr678";
        let scrubbed = scrub_credentials(input);
        assert!(scrubbed.contains("[REDACTED_KEY]"));
    }

    #[test]
    fn test_scrub_preserves_normal_text() {
        let input = "Hello world, this is a normal message with no secrets.";
        let scrubbed = scrub_credentials(input);
        assert_eq!(input, scrubbed);
    }

    #[test]
    fn test_scrub_short_values_ignored() {
        // Values shorter than 8 chars should NOT be redacted (too many false positives)
        let input = r#"token = "abc""#;
        let scrubbed = scrub_credentials(input);
        assert_eq!(input, scrubbed);
    }

    #[test]
    fn test_scrub_private_key() {
        let input = "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBg...";
        let scrubbed = scrub_credentials(input);
        assert!(scrubbed.contains("[REDACTED_KEY]"));
    }

    #[test]
    fn test_rate_limit_stats() {
        let registry = ToolRegistry::new(SecurityConfig::default());
        registry.record_tool_call("shell");
        registry.record_tool_call("shell");
        registry.record_tool_call("file_read");
        let stats = registry.rate_limit_stats();
        assert_eq!(stats.get("global"), Some(&3));
        assert_eq!(stats.get("shell"), Some(&2));
        assert_eq!(stats.get("file_read"), Some(&1));
    }

    // ── Preflight Tests ──────────────────────────────────────────────────────

    #[test]
    fn test_preflight_default_passes() {
        // A tool with default preflight should always pass
        let tool = super::browser::BrowserTool::new();
        assert!(tool.preflight(&serde_json::json!({})).is_ok());
    }

    #[test]
    fn test_preflight_shell_allowed_command() {
        let tool = super::shell::ShellTool::new(SecurityConfig::default());
        let args = serde_json::json!({"command": "git status"});
        assert!(tool.preflight(&args).is_ok());
    }

    #[test]
    fn test_preflight_shell_blocked_command() {
        let tool = super::shell::ShellTool::new(SecurityConfig::default());
        let args = serde_json::json!({"command": "rm -rf /"});
        assert!(tool.preflight(&args).is_err());
        let err = tool.preflight(&args).unwrap_err().to_string();
        assert!(err.contains("not in the allowed list"));
    }

    #[test]
    fn test_preflight_shell_empty_command() {
        let tool = super::shell::ShellTool::new(SecurityConfig::default());
        let args = serde_json::json!({"command": ""});
        assert!(tool.preflight(&args).is_err());
    }

    #[test]
    fn test_preflight_file_read_missing_path() {
        let tool = super::file_read::FileReadTool::new(SecurityConfig::default());
        let args = serde_json::json!({});
        assert!(tool.preflight(&args).is_err());
    }

    #[test]
    fn test_preflight_file_read_nonexistent() {
        let tool = super::file_read::FileReadTool::new(SecurityConfig::default());
        let args = serde_json::json!({"path": "/nonexistent/path/abc123.txt"});
        assert!(tool.preflight(&args).is_err());
        let err = tool.preflight(&args).unwrap_err().to_string();
        assert!(err.contains("does not exist"));
    }

    #[test]
    fn test_preflight_file_read_existing_file() {
        // Use Cargo.toml which always exists in the workspace
        let tool = super::file_read::FileReadTool::new(SecurityConfig {
            workspace_only: false,
            ..SecurityConfig::default()
        });
        // Use a path we know exists
        let cargo_path = std::env::current_dir().unwrap().join("Cargo.toml");
        let args = serde_json::json!({"path": cargo_path.to_string_lossy()});
        assert!(tool.preflight(&args).is_ok());
    }

    #[tokio::test]
    async fn test_preflight_blocks_execution() {
        let registry = ToolRegistry::new(SecurityConfig::default());
        // shell tool with disallowed command should fail at preflight
        let result = registry.execute_tool("shell", serde_json::json!({"command": "rm -rf /"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Preflight"));
    }

    #[test]
    fn test_preflight_shell_python_allowed() {
        let tool = super::shell::ShellTool::new(SecurityConfig::default());
        let args = serde_json::json!({"command": "python --version"});
        assert!(tool.preflight(&args).is_ok());
    }

    #[test]
    fn test_preflight_shell_npm_allowed() {
        let tool = super::shell::ShellTool::new(SecurityConfig::default());
        let args = serde_json::json!({"command": "npm list"});
        assert!(tool.preflight(&args).is_ok());
    }

    // ── Input Sanitizer Integration Tests ──────────────────────────────────

    #[tokio::test]
    async fn test_sanitizer_clean_args_pass_through() {
        let registry = ToolRegistry::new(SecurityConfig {
            workspace_only: false,
            ..SecurityConfig::default()
        });
        // Use a tool with args that are completely clean — calculator is side-effect-free
        let result = registry.execute_tool("calculator", serde_json::json!({"expression": "2 + 2"})).await.unwrap();
        // Clean args should reach the tool (success depends on the tool itself, not the sanitizer)
        // The key assertion: the sanitizer did NOT block the call
        assert!(result.success || result.output.contains("calculator") || !result.output.contains("Input validation failed"));
        assert!(!result.output.contains("Input validation failed"));
    }

    #[tokio::test]
    async fn test_sanitizer_blocks_path_traversal_in_file_read() {
        let registry = ToolRegistry::new(SecurityConfig::default());
        let result = registry.execute_tool(
            "file_read",
            serde_json::json!({"path": "../../../etc/passwd"}),
        ).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Input validation failed"));
        assert!(result.output.contains("path traversal"));
    }

    #[tokio::test]
    async fn test_sanitizer_blocks_shell_injection() {
        let registry = ToolRegistry::new(SecurityConfig::default());
        let result = registry.execute_tool(
            "shell",
            serde_json::json!({"command": "echo hello; rm -rf /"}),
        ).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Input validation failed"));
        assert!(result.output.contains("dangerous pattern"));
    }

    #[tokio::test]
    async fn test_sanitizer_blocks_sql_injection_in_data_analysis() {
        let registry = ToolRegistry::new(SecurityConfig::default());
        let result = registry.execute_tool(
            "data_analysis",
            serde_json::json!({"query": "SELECT 1; DROP TABLE users"}),
        ).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Input validation failed"));
    }
}
