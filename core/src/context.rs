use std::path::Path;

use crate::providers::traits::ChatMessage;

const SIZE_LIMIT: usize = 50 * 1024; // 50 KB
const TRUNCATE_SUFFIX: &str = "\n[...truncated at 50KB...]";

// ── Compaction ────────────────────────────────────────────────────────────────

/// Statistics returned by the sliding-window compaction pass.
#[derive(Debug, Clone, Default)]
pub struct CompactionStats {
    /// Approximate number of tokens saved (content bytes / 4).
    pub tokens_saved: usize,
    /// Number of conversation turns (user+assistant pairs) that were summarised.
    pub turns_summarised: usize,
}

/// Number of recent turns (user+assistant pairs) to preserve verbatim.
const KEEP_TURNS: usize = 8;
/// Number of the most-recent turns whose tool outputs are kept intact.
const KEEP_TOOL_OUTPUT_TURNS: usize = 3;
/// Maximum length of tool-output content in older messages before trimming.
const TOOL_OUTPUT_TRIM: usize = 200;
/// Maximum characters taken from each user message for the summary line.
const SUMMARY_SNIPPET: usize = 100;

/// Compact a conversation in-place using a sliding-window strategy:
///
/// * System messages are always preserved at the front.
/// * The last `KEEP_TURNS` user+assistant pairs are kept verbatim.
/// * Older turns are collapsed into a single summary message.
/// * Tool outputs in the oldest `(total_turns - KEEP_TOOL_OUTPUT_TURNS)` turns
///   are trimmed to `TOOL_OUTPUT_TRIM` chars.
///
/// Returns [`CompactionStats`] describing what was compacted.
pub fn compact_conversation(messages: &mut Vec<ChatMessage>) -> CompactionStats {
    let mut stats = CompactionStats::default();

    // Separate system messages from conversation messages.
    let system_msgs: Vec<ChatMessage> = messages
        .iter()
        .filter(|m| m.role == "system")
        .cloned()
        .collect();

    let conv_msgs: Vec<ChatMessage> = messages
        .iter()
        .filter(|m| m.role != "system")
        .cloned()
        .collect();

    // Group conversation into turns (user msg + following assistant/tool msgs).
    // A turn starts at each "user" message.
    let mut turns: Vec<Vec<ChatMessage>> = Vec::new();
    let mut current_turn: Vec<ChatMessage> = Vec::new();

    for msg in &conv_msgs {
        if msg.role == "user" && !current_turn.is_empty() {
            turns.push(current_turn);
            current_turn = Vec::new();
        }
        current_turn.push(msg.clone());
    }
    if !current_turn.is_empty() {
        turns.push(current_turn);
    }

    let total_turns = turns.len();

    // If we have no more turns than we want to keep, nothing to do.
    if total_turns <= KEEP_TURNS {
        // Still trim large tool outputs in older turns relative to KEEP_TOOL_OUTPUT_TURNS.
        if total_turns > KEEP_TOOL_OUTPUT_TURNS {
            let trim_up_to = total_turns - KEEP_TOOL_OUTPUT_TURNS;
            for turn in turns[..trim_up_to].iter_mut() {
                for msg in turn.iter_mut() {
                    if msg.role == "tool" || (msg.role == "assistant" && msg.tool_calls.is_some()) {
                        let before = msg.content.len();
                        trim_tool_output_content(msg);
                        stats.tokens_saved += before.saturating_sub(msg.content.len()) / 4;
                    }
                }
            }
        }
        // Reconstruct from (possibly trimmed) turns.
        *messages = system_msgs
            .into_iter()
            .chain(turns.into_iter().flatten())
            .collect();
        return stats;
    }

    // Split: older turns get summarised, recent turns are kept verbatim.
    let split = total_turns - KEEP_TURNS;
    let older_turns = &turns[..split];
    let recent_turns = &turns[split..];

    stats.turns_summarised = older_turns.len();

    // Build summary text from older turns.
    let mut summary_lines: Vec<String> = Vec::new();
    for turn in older_turns {
        let user_snippet: String = turn
            .iter()
            .find(|m| m.role == "user")
            .map(|m| m.content.chars().take(SUMMARY_SNIPPET).collect())
            .unwrap_or_default();

        // Collect tool names used in this turn.
        let tool_names: Vec<String> = turn
            .iter()
            .filter_map(|m| {
                // tool_calls is a JSON array of objects with a "function.name" or "name" field.
                if let Some(calls) = &m.tool_calls {
                    if let Some(arr) = calls.as_array() {
                        let names: Vec<String> = arr
                            .iter()
                            .filter_map(|c| {
                                c["function"]["name"]
                                    .as_str()
                                    .or_else(|| c["name"].as_str())
                                    .map(|s| s.to_string())
                            })
                            .collect();
                        if !names.is_empty() { return Some(names); }
                    }
                }
                None
            })
            .flatten()
            .collect();

        let mut line = user_snippet;
        if !tool_names.is_empty() {
            line.push_str(&format!(" → [{}]", tool_names.join(", ")));
        }
        if !line.trim().is_empty() {
            summary_lines.push(line);
        }
    }

    // Estimate tokens saved: sum of all content in older turns.
    for turn in older_turns {
        for msg in turn {
            stats.tokens_saved += (msg.content.len() / 4).max(1);
        }
    }

    let summary_content = if summary_lines.is_empty() {
        format!(
            "Summary of earlier conversation: {} turn(s) occurred before this point.",
            stats.turns_summarised
        )
    } else {
        format!(
            "Summary of earlier conversation:\n{}",
            summary_lines.join("\n")
        )
    };

    let summary_msg = ChatMessage {
        role: "user".to_string(),
        content: summary_content,
        tool_calls: None,
    };

    // Trim tool outputs in recent turns that fall outside KEEP_TOOL_OUTPUT_TURNS.
    let recent_trim_boundary = recent_turns
        .len()
        .saturating_sub(KEEP_TOOL_OUTPUT_TURNS);

    let mut recent_processed: Vec<ChatMessage> = Vec::new();
    for (i, turn) in recent_turns.iter().enumerate() {
        for msg in turn {
            let mut m = msg.clone();
            if i < recent_trim_boundary
                && (m.role == "tool" || (m.role == "assistant" && m.tool_calls.is_some()))
            {
                let before = m.content.len();
                trim_tool_output_content(&mut m);
                stats.tokens_saved += before.saturating_sub(m.content.len()) / 4;
            }
            recent_processed.push(m);
        }
    }

    // Assemble: system msgs → summary → recent turns.
    *messages = system_msgs
        .into_iter()
        .chain(std::iter::once(summary_msg))
        .chain(recent_processed)
        .collect();

    stats
}

/// Trim the content of a tool-output or large assistant message to
/// `TOOL_OUTPUT_TRIM` characters, appending `[trimmed]` if shortened.
fn trim_tool_output_content(msg: &mut ChatMessage) {
    if msg.content.len() > TOOL_OUTPUT_TRIM {
        let mut boundary = TOOL_OUTPUT_TRIM;
        while !msg.content.is_char_boundary(boundary) {
            boundary -= 1;
        }
        msg.content.truncate(boundary);
        msg.content.push_str("[trimmed]");
    }
}

/// Walk up from `cwd` toward the filesystem root, checking each directory for
/// known project-config filenames in priority order.  Returns the content of
/// the first file found, truncated to 50 KB if necessary.
pub fn load_project_config(cwd: &Path) -> Option<String> {
    // Project-config files, checked in priority order. Codex/Claude Code
    // ecosystem convention: PHANTOM.md = phantom-native, CLAUDE.md = Claude
    // Code, AGENTS.md = tool-neutral handoff (Codex / OpenCode / others),
    // GEMINI.md = Gemini Code.
    const CANDIDATES: &[&str] = &[
        "PHANTOM.md",
        "CLAUDE.md",
        "AGENTS.md",
        "GEMINI.md",
        ".phantom/config.md",
        ".phantom",
    ];

    // Walk up from `cwd` toward the filesystem root.
    let mut dir = cwd.to_path_buf();
    loop {
        for candidate in CANDIDATES {
            let path = dir.join(candidate);
            if path.exists() && path.is_file() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    // Truncate if necessary.
                    if content.len() > SIZE_LIMIT {
                        let mut boundary = SIZE_LIMIT;
                        while !content.is_char_boundary(boundary) {
                            boundary -= 1;
                        }
                        let mut truncated = content[..boundary].to_string();
                        truncated.push_str(TRUNCATE_SUFFIX);
                        return Some(truncated);
                    }
                    return Some(content);
                }
            }
        }

        // Advance toward the root; stop when we can no longer go up.
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent.to_path_buf(),
            _ => break,
        }
    }
    None
}

/// Read `Cargo.toml` dependency names (up to `limit`).
fn parse_cargo_deps(content: &str, limit: usize) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_deps = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect entering a [dependencies] / [dev-dependencies] / [build-dependencies] section.
        if trimmed.starts_with('[') {
            in_deps = matches!(
                trimmed,
                "[dependencies]" | "[dev-dependencies]" | "[build-dependencies]"
            );
            continue;
        }

        if in_deps && !trimmed.is_empty() && !trimmed.starts_with('#') {
            // A dependency line looks like: `name = "..."` or `name = { ... }`
            if let Some(eq_pos) = trimmed.find('=') {
                let name = trimmed[..eq_pos].trim().to_string();
                if !name.is_empty() && !name.starts_with('[') && !deps.contains(&name) {
                    deps.push(name);
                    if deps.len() >= limit {
                        break;
                    }
                }
            }
        }
    }
    deps
}

/// Read `package.json` dependency names (up to `limit`).
fn parse_package_json_deps(content: &str, limit: usize) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_deps_block = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect entering a dependencies or devDependencies block.
        if trimmed.contains("\"dependencies\"") || trimmed.contains("\"devDependencies\"") {
            in_deps_block = true;
            continue;
        }

        if in_deps_block {
            // A closing brace ends the block.
            if trimmed == "}" || trimmed == "}," {
                in_deps_block = false;
                continue;
            }

            // Lines look like: `"package-name": "^1.0.0",`
            if let Some(stripped) = trimmed.strip_prefix('"') {
                if let Some(end) = stripped.find('"') {
                    let name = stripped[..end].to_string();
                    if !name.is_empty() {
                        if !deps.contains(&name) {
                            deps.push(name);
                            if deps.len() >= limit {
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    deps
}

/// Read `pyproject.toml` dependency names (up to `limit`).
fn parse_pyproject_deps(content: &str, limit: usize) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_deps = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // PEP 621 [project] dependencies or [tool.poetry.dependencies]
        if trimmed == "[project]"
            || trimmed == "dependencies = ["
            || trimmed.starts_with("dependencies")
        {
            in_deps = true;
            continue;
        }

        if in_deps {
            if trimmed == "]" {
                in_deps = false;
                continue;
            }

            // Each line is like: `"requests>=2.0"` or `requests = "^2.0"`
            let raw = trimmed.trim_matches(|c| c == '"' || c == '\'' || c == ',');
            if raw.is_empty() || raw.starts_with('#') || raw.starts_with('[') {
                if raw.starts_with('[') {
                    in_deps = false;
                }
                continue;
            }

            // Strip version specifiers: >=, <=, ~=, !=, ==, ^, ~
            let name = raw
                .split(|c: char| {
                    c == '>' || c == '<' || c == '~' || c == '!' || c == '=' || c == '^'
                })
                .next()
                .unwrap_or(raw)
                .trim()
                .to_lowercase();

            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                && !deps.contains(&name)
            {
                deps.push(name);
                if deps.len() >= limit {
                    break;
                }
            }
        }
    }
    deps
}

/// Read `requirements.txt` package names (up to `limit`).
fn parse_requirements_txt(content: &str, limit: usize) -> Vec<String> {
    let mut deps = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
            continue;
        }

        // Strip version specifiers.
        let name = trimmed
            .split(|c: char| {
                c == '>'
                    || c == '<'
                    || c == '~'
                    || c == '!'
                    || c == '='
                    || c == '^'
                    || c == '['
            })
            .next()
            .unwrap_or(trimmed)
            .trim()
            .to_lowercase();

        if !name.is_empty() {
            if !deps.contains(&name) {
                deps.push(name);
                if deps.len() >= limit {
                    break;
                }
            }
        }
    }
    deps
}

/// Read `go.mod` module dependency names (up to `limit`).
fn parse_go_mod_deps(content: &str, limit: usize) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_require = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == "require (" {
            in_require = true;
            continue;
        }

        if in_require {
            if trimmed == ")" {
                in_require = false;
                continue;
            }

            // Lines look like: `github.com/foo/bar v1.2.3`
            if let Some(pkg) = trimmed.split_whitespace().next() {
                if !pkg.is_empty() && !pkg.starts_with("//") {
                    let short = pkg.split('/').last().unwrap_or(pkg).to_string();
                    if !deps.contains(&short) {
                        deps.push(short);
                        if deps.len() >= limit {
                            break;
                        }
                    }
                }
            }
        } else if trimmed.starts_with("require ") {
            // Single-line require: `require github.com/foo/bar v1.2.3`
            let rest = trimmed["require ".len()..].trim();
            if let Some(pkg) = rest.split_whitespace().next() {
                let short = pkg.split('/').last().unwrap_or(pkg).to_string();
                if !deps.contains(&short) {
                    deps.push(short);
                    if deps.len() >= limit {
                        break;
                    }
                }
            }
        }
    }
    deps
}

/// Detect project dependencies by reading manifest files in `cwd`.
/// Returns up to 20 dependency names from the first manifest found.
pub fn detect_dependencies(cwd: &Path) -> Vec<String> {
    const LIMIT: usize = 20;

    // Cargo.toml
    if let Ok(content) = std::fs::read_to_string(cwd.join("Cargo.toml")) {
        let deps = parse_cargo_deps(&content, LIMIT);
        if !deps.is_empty() {
            return deps;
        }
    }

    // package.json
    if let Ok(content) = std::fs::read_to_string(cwd.join("package.json")) {
        let deps = parse_package_json_deps(&content, LIMIT);
        if !deps.is_empty() {
            return deps;
        }
    }

    // pyproject.toml
    if let Ok(content) = std::fs::read_to_string(cwd.join("pyproject.toml")) {
        let deps = parse_pyproject_deps(&content, LIMIT);
        if !deps.is_empty() {
            return deps;
        }
    }

    // requirements.txt
    if let Ok(content) = std::fs::read_to_string(cwd.join("requirements.txt")) {
        let deps = parse_requirements_txt(&content, LIMIT);
        if !deps.is_empty() {
            return deps;
        }
    }

    // go.mod
    if let Ok(content) = std::fs::read_to_string(cwd.join("go.mod")) {
        let deps = parse_go_mod_deps(&content, LIMIT);
        if !deps.is_empty() {
            return deps;
        }
    }

    Vec::new()
}

/// Detect the primary framework/stack based on dependency names and files present.
pub fn detect_framework(cwd: &Path, deps: &[String]) -> Option<String> {
    let has = |name: &str| deps.iter().any(|d| d.to_lowercase() == name);
    let file_exists = |name: &str| cwd.join(name).exists();

    // Rust frameworks
    if has("axum") {
        return Some("Axum web framework".to_string());
    }
    if has("actix-web") || has("actix_web") {
        return Some("Actix-Web framework".to_string());
    }
    if has("tauri") {
        return Some("Tauri desktop app".to_string());
    }
    if has("rocket") {
        return Some("Rocket web framework".to_string());
    }

    // JavaScript/TypeScript frameworks
    if has("next") {
        return Some("Next.js".to_string());
    }
    if has("react") {
        if file_exists("tsconfig.json") {
            return Some("React (TypeScript)".to_string());
        }
        return Some("React".to_string());
    }
    if has("vue") {
        return Some("Vue.js".to_string());
    }
    if has("svelte") {
        return Some("Svelte".to_string());
    }
    if has("express") {
        return Some("Express.js".to_string());
    }
    if has("fastify") {
        return Some("Fastify".to_string());
    }
    if has("nestjs") || has("@nestjs/core") {
        return Some("NestJS".to_string());
    }

    // Python frameworks
    if has("django") {
        return Some("Django web framework".to_string());
    }
    if has("fastapi") {
        return Some("FastAPI".to_string());
    }
    if has("flask") {
        return Some("Flask".to_string());
    }

    // Go
    if has("gin") {
        return Some("Gin web framework".to_string());
    }
    if has("fiber") {
        return Some("Fiber web framework".to_string());
    }
    if has("echo") {
        return Some("Echo web framework".to_string());
    }

    None
}

/// Run `git log --oneline -5` and `git diff --stat HEAD` and return a compact
/// summary string.  Returns `None` if git is not available or the directory is
/// not a git repo.
pub fn recent_git_changes(cwd: &Path) -> Option<String> {
    let log_output = std::process::Command::new("git")
        .args(["log", "--oneline", "-5"])
        .current_dir(cwd)
        .output()
        .ok()
        .filter(|o| o.status.success())?;

    let log = String::from_utf8_lossy(&log_output.stdout).trim().to_string();
    if log.is_empty() {
        return None;
    }

    let diff_output = std::process::Command::new("git")
        .args(["diff", "--stat", "HEAD"])
        .current_dir(cwd)
        .output()
        .ok()
        .filter(|o| o.status.success());

    let diff = diff_output
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    let mut summary = format!("Recent commits:\n{}", log);
    if let Some(stat) = diff {
        summary.push_str("\nUnstaged diff stat:\n");
        summary.push_str(&stat);
    }
    Some(summary)
}

/// Workspace context injected into the agent's system prompt at startup.
/// Tells the agent where it is and what the repo looks like.
pub struct WorkspaceContext {
    pub cwd: std::path::PathBuf,
    pub git_branch: Option<String>,
    pub git_status: Option<String>,
    pub project_config: Option<String>,
    pub dependencies: Vec<String>,
    pub framework: Option<String>,
    pub recent_changes: Option<String>,
}

impl WorkspaceContext {
    pub fn capture() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

        let git_branch = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty() && s != "HEAD");

        let git_status = std::process::Command::new("git")
            .args(["status", "--short"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty());

        let project_config = load_project_config(&cwd);
        let dependencies = detect_dependencies(&cwd);
        let framework = detect_framework(&cwd, &dependencies);
        let recent_changes = recent_git_changes(&cwd);

        Self {
            cwd,
            git_branch,
            git_status,
            project_config,
            dependencies,
            framework,
            recent_changes,
        }
    }

    /// Returns a short one-line-per-field summary suitable for injection into
    /// already-compacted contexts where token budget is tight.
    ///
    /// Includes: cwd, git branch, and the first 3 lines of a README if present.
    pub fn to_system_context_brief(&self) -> String {
        let mut parts = vec![format!("cwd: {}", self.cwd.display())];

        if let Some(branch) = &self.git_branch {
            parts.push(format!("branch: {}", branch));
        }

        // Try to read the first 3 lines of README.md (or README) from cwd.
        let readme_content = ["README.md", "README", "readme.md", "readme"]
            .iter()
            .find_map(|name| std::fs::read_to_string(self.cwd.join(name)).ok());

        if let Some(readme) = readme_content {
            let first_three: String = readme
                .lines()
                .filter(|l| !l.trim().is_empty())
                .take(3)
                .collect::<Vec<_>>()
                .join(" | ");
            if !first_three.is_empty() {
                parts.push(format!("readme: {}", first_three));
            }
        }

        parts.join("; ")
    }

    pub fn to_system_context(&self) -> String {
        let mut lines = vec![format!("Working directory: {}", self.cwd.display())];

        if let Some(branch) = &self.git_branch {
            lines.push(format!("Git branch: {}", branch));
        }
        if let Some(status) = &self.git_status {
            lines.push(format!("Uncommitted changes:\n{}", status));
        }
        if !self.dependencies.is_empty() {
            lines.push(format!("Dependencies: {}", self.dependencies.join(", ")));
        }
        if let Some(framework) = &self.framework {
            lines.push(format!("Framework: {}", framework));
        }
        if let Some(changes) = &self.recent_changes {
            lines.push(format!(
                "Recent changes:\n  {}",
                changes.replace('\n', "\n  ")
            ));
        }

        let workspace_part = lines.join("\n");

        match &self.project_config {
            Some(config) => format!(
                "<project-config>\n{}\n</project-config>\n\n{}",
                config, workspace_part
            ),
            None => workspace_part,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── load_project_config ──────────────────────────────────────────────────

    #[test]
    fn test_load_project_config_finds_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("PHANTOM.md"), "hello world").unwrap();

        let result = load_project_config(dir.path());
        assert_eq!(result, Some("hello world".to_string()));
    }

    #[test]
    fn test_load_project_config_truncates_large_file() {
        let dir = tempfile::tempdir().unwrap();
        let big_content = "x".repeat(SIZE_LIMIT + 100);
        std::fs::write(dir.path().join("CLAUDE.md"), &big_content).unwrap();

        let result = load_project_config(dir.path()).unwrap();
        assert!(result.ends_with(TRUNCATE_SUFFIX));
        assert!(result.len() <= SIZE_LIMIT + TRUNCATE_SUFFIX.len());
    }

    #[test]
    fn test_load_project_config_walks_up() {
        let parent = tempfile::tempdir().unwrap();
        let child = parent.path().join("subdir");
        std::fs::create_dir(&child).unwrap();
        std::fs::write(parent.path().join("PHANTOM.md"), "from parent").unwrap();

        let result = load_project_config(&child);
        assert_eq!(result, Some("from parent".to_string()));
    }

    #[test]
    fn test_load_project_config_priority_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("PHANTOM.md"), "phantom").unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "claude").unwrap();

        let result = load_project_config(dir.path());
        assert_eq!(result, Some("phantom".to_string()));
    }

    #[test]
    fn test_load_project_config_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let result = load_project_config(dir.path());
        assert!(result.is_none());
    }

    // ── to_system_context ────────────────────────────────────────────────────

    #[test]
    fn test_to_system_context_with_config() {
        let ctx = WorkspaceContext {
            cwd: std::path::PathBuf::from("/tmp"),
            git_branch: None,
            git_status: None,
            project_config: Some("my config".to_string()),
            dependencies: vec![],
            framework: None,
            recent_changes: None,
        };
        let out = ctx.to_system_context();
        assert!(out.starts_with("<project-config>\n"));
        assert!(out.contains("</project-config>"));
        assert!(out.contains("Working directory:"));
    }

    #[test]
    fn test_to_system_context_without_config() {
        let ctx = WorkspaceContext {
            cwd: std::path::PathBuf::from("/tmp"),
            git_branch: None,
            git_status: None,
            project_config: None,
            dependencies: vec![],
            framework: None,
            recent_changes: None,
        };
        let out = ctx.to_system_context();
        assert!(!out.contains("<project-config>"));
        assert!(out.starts_with("Working directory:"));
    }

    #[test]
    fn test_to_system_context_includes_deps_and_framework() {
        let ctx = WorkspaceContext {
            cwd: std::path::PathBuf::from("/tmp"),
            git_branch: Some("main".to_string()),
            git_status: None,
            project_config: None,
            dependencies: vec!["axum".to_string(), "tokio".to_string()],
            framework: Some("Axum web framework".to_string()),
            recent_changes: None,
        };
        let out = ctx.to_system_context();
        assert!(out.contains("Dependencies: axum, tokio"));
        assert!(out.contains("Framework: Axum web framework"));
    }

    #[test]
    fn test_to_system_context_recent_changes() {
        let ctx = WorkspaceContext {
            cwd: std::path::PathBuf::from("/tmp"),
            git_branch: None,
            git_status: None,
            project_config: None,
            dependencies: vec![],
            framework: None,
            recent_changes: Some("Recent commits:\nabc1234 fix: something".to_string()),
        };
        let out = ctx.to_system_context();
        assert!(out.contains("Recent changes:"));
        assert!(out.contains("abc1234"));
    }

    // ── detect_dependencies ──────────────────────────────────────────────────

    #[test]
    fn test_detect_dependencies_cargo() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"foo\"\n\n[dependencies]\ntokio = \"1\"\nserde = \"1\"\n",
        )
        .unwrap();

        let deps = detect_dependencies(dir.path());
        assert!(deps.contains(&"tokio".to_string()));
        assert!(deps.contains(&"serde".to_string()));
    }

    #[test]
    fn test_detect_dependencies_package_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            "{\n  \"dependencies\": {\n    \"react\": \"^18\",\n    \"axios\": \"^1\"\n  }\n}",
        )
        .unwrap();

        let deps = detect_dependencies(dir.path());
        assert!(deps.contains(&"react".to_string()));
        assert!(deps.contains(&"axios".to_string()));
    }

    #[test]
    fn test_detect_dependencies_requirements_txt() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("requirements.txt"),
            "flask>=2.0\nrequests==2.28\n",
        )
        .unwrap();

        let deps = detect_dependencies(dir.path());
        assert!(deps.contains(&"flask".to_string()));
        assert!(deps.contains(&"requests".to_string()));
    }

    #[test]
    fn test_detect_dependencies_go_mod() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("go.mod"),
            "module example.com/myapp\n\nrequire (\n\tgithub.com/gin-gonic/gin v1.9.0\n)\n",
        )
        .unwrap();

        let deps = detect_dependencies(dir.path());
        assert!(deps.contains(&"gin".to_string()));
    }

    #[test]
    fn test_detect_dependencies_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let deps = detect_dependencies(dir.path());
        assert!(deps.is_empty());
    }

    #[test]
    fn test_detect_dependencies_limit() {
        let dir = tempfile::tempdir().unwrap();
        // Write 30 deps — should return at most 20.
        let mut toml = "[dependencies]\n".to_string();
        for i in 0..30 {
            toml.push_str(&format!("dep{} = \"1\"\n", i));
        }
        std::fs::write(dir.path().join("Cargo.toml"), toml).unwrap();

        let deps = detect_dependencies(dir.path());
        assert_eq!(deps.len(), 20);
    }

    // ── detect_framework ─────────────────────────────────────────────────────

    #[test]
    fn test_detect_framework_axum() {
        let dir = tempfile::tempdir().unwrap();
        let deps = vec!["tokio".to_string(), "axum".to_string()];
        assert_eq!(
            detect_framework(dir.path(), &deps),
            Some("Axum web framework".to_string())
        );
    }

    #[test]
    fn test_detect_framework_react_ts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();
        let deps = vec!["react".to_string()];
        assert_eq!(
            detect_framework(dir.path(), &deps),
            Some("React (TypeScript)".to_string())
        );
    }

    #[test]
    fn test_detect_framework_react_no_ts() {
        let dir = tempfile::tempdir().unwrap();
        let deps = vec!["react".to_string()];
        assert_eq!(
            detect_framework(dir.path(), &deps),
            Some("React".to_string())
        );
    }

    #[test]
    fn test_detect_framework_django() {
        let dir = tempfile::tempdir().unwrap();
        let deps = vec!["django".to_string()];
        assert_eq!(
            detect_framework(dir.path(), &deps),
            Some("Django web framework".to_string())
        );
    }

    #[test]
    fn test_detect_framework_none() {
        let dir = tempfile::tempdir().unwrap();
        let deps = vec!["serde".to_string(), "tokio".to_string()];
        assert_eq!(detect_framework(dir.path(), &deps), None);
    }

    // ── parse helpers ────────────────────────────────────────────────────────

    #[test]
    fn test_parse_cargo_deps_dev_and_build() {
        let toml =
            "[dev-dependencies]\ncriterion = \"0.5\"\n\n[build-dependencies]\nbuild-script = \"1\"\n";
        let deps = parse_cargo_deps(toml, 20);
        assert!(deps.contains(&"criterion".to_string()));
        assert!(deps.contains(&"build-script".to_string()));
    }

    #[test]
    fn test_parse_requirements_txt_skips_flags() {
        let txt = "-r other.txt\n# comment\ndjango>=3.2\n";
        let deps = parse_requirements_txt(txt, 20);
        assert_eq!(deps, vec!["django"]);
    }

    #[test]
    fn test_parse_go_mod_single_line_require() {
        let content = "module example.com/app\n\nrequire github.com/stretchr/testify v1.8.0\n";
        let deps = parse_go_mod_deps(content, 20);
        assert!(deps.contains(&"testify".to_string()));
    }
}
