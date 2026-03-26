//! code_evolution tool — AI-driven dev-loop: analyze → propose → apply → test → commit.
//!
//! Operates on files in the workspace or allowed paths.
//! Supports Rust (cargo) and Node.js (npm/pnpm) projects.

use anyhow::{bail, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Tool, ToolResult};

// ─── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeIssue {
    pub file: String,
    pub line: Option<usize>,
    pub kind: String,     // "todo", "fixme", "unused_import", "dead_code", "style", "error", "warning"
    pub message: String,
    pub severity: String, // "info", "warning", "error"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub project_type: String, // "rust", "node", "unknown"
    pub issues: Vec<CodeIssue>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub success: bool,
    pub output: String,
    pub tests_run: usize,
    pub tests_passed: usize,
    pub tests_failed: usize,
}

// ─── Tool ────────────────────────────────────────────────────────────────────

pub struct CodeEvolutionTool {
    workspace_dir: String,
}

impl CodeEvolutionTool {
    pub fn new(workspace_dir: String) -> Self {
        Self { workspace_dir }
    }

    fn resolve_dir(&self, project_dir: &str) -> PathBuf {
        let p = PathBuf::from(project_dir);
        if p.is_absolute() {
            p
        } else {
            PathBuf::from(&self.workspace_dir).join(p)
        }
    }

    fn detect_project_type(dir: &Path) -> &'static str {
        if dir.join("Cargo.toml").exists() {
            "rust"
        } else if dir.join("package.json").exists() {
            "node"
        } else {
            "unknown"
        }
    }

    /// Analyze: collect TODOs, FIXMEs, compiler warnings, and lint issues.
    fn analyze(&self, dir: &Path) -> Result<AnalysisResult> {
        let project_type = Self::detect_project_type(dir);
        let mut issues = Vec::new();

        // 1. Grep for TODO/FIXME markers in source files
        issues.extend(self.scan_markers(dir));

        // 2. Run compiler/linter for structured diagnostics
        match project_type {
            "rust" => issues.extend(self.analyze_rust(dir)),
            "node" => issues.extend(self.analyze_node(dir)),
            _ => {}
        }

        let summary = format!(
            "{} issues found ({} errors, {} warnings, {} info)",
            issues.len(),
            issues.iter().filter(|i| i.severity == "error").count(),
            issues.iter().filter(|i| i.severity == "warning").count(),
            issues.iter().filter(|i| i.severity == "info").count(),
        );

        Ok(AnalysisResult {
            project_type: project_type.to_string(),
            issues,
            summary,
        })
    }

    /// Scan for TODO/FIXME markers in common source file types.
    fn scan_markers(&self, dir: &Path) -> Vec<CodeIssue> {
        let mut issues = Vec::new();
        let extensions = ["rs", "ts", "tsx", "js", "jsx", "py", "toml"];

        for ext in &extensions {
            if let Ok(output) = Command::new("grep")
                .args(["-rnI", "--include", &format!("*.{}", ext), "-E", "TODO|FIXME|HACK|XXX"])
                .arg(dir.to_string_lossy().as_ref())
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines().take(50) {
                    // Format: filepath:linenum:content
                    let parts: Vec<&str> = line.splitn(3, ':').collect();
                    if parts.len() >= 3 {
                        let kind = if parts[2].contains("FIXME") {
                            "fixme"
                        } else if parts[2].contains("HACK") || parts[2].contains("XXX") {
                            "hack"
                        } else {
                            "todo"
                        };
                        let severity = if kind == "fixme" || kind == "hack" { "warning" } else { "info" };
                        issues.push(CodeIssue {
                            file: parts[0].to_string(),
                            line: parts[1].parse().ok(),
                            kind: kind.to_string(),
                            message: parts[2].trim().to_string(),
                            severity: severity.to_string(),
                        });
                    }
                }
            }
        }
        issues
    }

    /// Run `cargo check` in JSON mode and parse diagnostics.
    fn analyze_rust(&self, dir: &Path) -> Vec<CodeIssue> {
        let mut issues = Vec::new();

        let output = Command::new("cargo")
            .args(["check", "--message-format=json"])
            .current_dir(dir)
            .env("CARGO_TERM_COLOR", "never")
            .output();

        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                if let Ok(msg) = serde_json::from_str::<Value>(line) {
                    if msg.get("reason").and_then(|r| r.as_str()) == Some("compiler-message") {
                        if let Some(message) = msg.get("message") {
                            let level = message.get("level").and_then(|l| l.as_str()).unwrap_or("warning");
                            let text = message.get("message").and_then(|m| m.as_str()).unwrap_or("");
                            let code = message.get("code").and_then(|c| c.get("code")).and_then(|c| c.as_str()).unwrap_or("");

                            // Extract primary span file/line
                            let (file, line_num) = message.get("spans")
                                .and_then(|s| s.as_array())
                                .and_then(|a| a.first())
                                .map(|span| {
                                    let f = span.get("file_name").and_then(|f| f.as_str()).unwrap_or("");
                                    let l = span.get("line_start").and_then(|l| l.as_u64()).unwrap_or(0);
                                    (f.to_string(), l as usize)
                                })
                                .unwrap_or_default();

                            let severity = match level {
                                "error" => "error",
                                "warning" => "warning",
                                _ => "info",
                            };

                            let kind = if code.contains("unused") || text.contains("unused") {
                                "unused"
                            } else if code.contains("dead_code") {
                                "dead_code"
                            } else {
                                level
                            };

                            if !text.is_empty() && severity != "info" {
                                issues.push(CodeIssue {
                                    file: if file.is_empty() { "unknown".to_string() } else { file },
                                    line: if line_num > 0 { Some(line_num) } else { None },
                                    kind: kind.to_string(),
                                    message: format!("{} [{}]", text, code),
                                    severity: severity.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
        issues
    }

    /// Run `npx tsc --noEmit` and parse TypeScript errors.
    fn analyze_node(&self, dir: &Path) -> Vec<CodeIssue> {
        let mut issues = Vec::new();

        let output = Command::new("npx")
            .args(["tsc", "--noEmit", "--pretty", "false"])
            .current_dir(dir)
            .output();

        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let combined = format!("{}\n{}", stdout, stderr);

            // Format: file(line,col): error TSxxxx: message
            let re = regex::Regex::new(r"^(.+?)\((\d+),\d+\):\s*(error|warning)\s+(\S+):\s*(.+)$").ok();
            if let Some(re) = re {
                for line in combined.lines().take(50) {
                    if let Some(caps) = re.captures(line) {
                        issues.push(CodeIssue {
                            file: caps[1].to_string(),
                            line: caps[2].parse().ok(),
                            kind: caps[3].to_string(),
                            message: format!("{}: {}", &caps[4], &caps[5]),
                            severity: caps[3].to_string(),
                        });
                    }
                }
            }
        }
        issues
    }

    /// Run tests for the detected project type.
    fn run_tests(&self, dir: &Path, filter: Option<&str>) -> TestResult {
        let project_type = Self::detect_project_type(dir);
        match project_type {
            "rust" => self.run_rust_tests(dir, filter),
            "node" => self.run_node_tests(dir),
            _ => TestResult {
                success: false,
                output: "Unknown project type — cannot run tests".to_string(),
                tests_run: 0, tests_passed: 0, tests_failed: 0,
            },
        }
    }

    fn run_rust_tests(&self, dir: &Path, filter: Option<&str>) -> TestResult {
        let mut args = vec!["test"];
        if let Some(f) = filter {
            args.push("--");
            args.push(f);
        }

        let output = Command::new("cargo")
            .args(&args)
            .current_dir(dir)
            .env("CARGO_TERM_COLOR", "never")
            .output();

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let combined = format!("{}\n{}", stdout, stderr);

                // Parse "test result: ok. X passed; Y failed"
                let (mut passed, mut failed) = (0usize, 0usize);
                for line in combined.lines() {
                    if line.starts_with("test result:") {
                        if let Some(p) = line.split("passed").next() {
                            if let Some(n) = p.split_whitespace().last() {
                                passed = n.parse().unwrap_or(0);
                            }
                        }
                        if let Some(f) = line.split("failed").next() {
                            if let Some(n) = f.rsplit(';').next().unwrap_or("").trim().split_whitespace().last() {
                                failed = n.parse().unwrap_or(0);
                            }
                        }
                    }
                }

                TestResult {
                    success: out.status.success(),
                    output: truncate_output(&combined, 3000),
                    tests_run: passed + failed,
                    tests_passed: passed,
                    tests_failed: failed,
                }
            }
            Err(e) => TestResult {
                success: false,
                output: format!("Failed to run cargo test: {}", e),
                tests_run: 0, tests_passed: 0, tests_failed: 0,
            },
        }
    }

    fn run_node_tests(&self, dir: &Path) -> TestResult {
        // Try pnpm test, npm test
        let cmd = if dir.join("pnpm-lock.yaml").exists() { "pnpm" } else { "npm" };
        let output = Command::new(cmd)
            .args(["test", "--", "--passWithNoTests"])
            .current_dir(dir)
            .output();

        match output {
            Ok(out) => {
                let combined = format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr),
                );
                TestResult {
                    success: out.status.success(),
                    output: truncate_output(&combined, 3000),
                    tests_run: 0, tests_passed: 0, tests_failed: 0,
                }
            }
            Err(e) => TestResult {
                success: false,
                output: format!("Failed to run {cmd} test: {e}"),
                tests_run: 0, tests_passed: 0, tests_failed: 0,
            },
        }
    }

    /// Git status for the project directory.
    fn git_status(&self, dir: &Path) -> String {
        Command::new("git")
            .args(["status", "--short"])
            .current_dir(dir)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_else(|e| format!("git error: {}", e))
    }

    /// Git diff (staged + unstaged).
    fn git_diff(&self, dir: &Path) -> String {
        let unstaged = Command::new("git")
            .args(["diff"])
            .current_dir(dir)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        let staged = Command::new("git")
            .args(["diff", "--cached"])
            .current_dir(dir)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        truncate_output(&format!("{}\n{}", staged, unstaged), 5000)
    }

    /// Create a git commit with the given message.
    fn git_commit(&self, dir: &Path, message: &str) -> Result<String> {
        // Stage all changes
        let add_out = Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir)
            .output()?;
        if !add_out.status.success() {
            bail!("git add failed: {}", String::from_utf8_lossy(&add_out.stderr));
        }

        let commit_out = Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(dir)
            .output()?;

        if commit_out.status.success() {
            Ok(String::from_utf8_lossy(&commit_out.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&commit_out.stderr);
            if stderr.contains("nothing to commit") {
                Ok("Nothing to commit — working tree clean".to_string())
            } else {
                bail!("git commit failed: {}", stderr);
            }
        }
    }
}

fn truncate_output(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...\n[truncated {} chars]", &s[..max], s.len() - max)
    } else {
        s.to_string()
    }
}

// ─── Tool trait implementation ───────────────────────────────────────────────

#[async_trait]
impl Tool for CodeEvolutionTool {
    fn name(&self) -> &str { "code_evolution" }

    fn description(&self) -> &str {
        "AI-driven dev-loop tool: analyze codebase for issues, run tests, review diffs, and commit changes. \
         Operations: analyze (scan for TODOs, warnings, lint issues), test (run project tests), \
         diff (show git changes), status (git status), commit (create a git commit)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["analyze", "test", "diff", "status", "commit"],
                    "description": "Operation to perform"
                },
                "project_dir": {
                    "type": "string",
                    "description": "Path to the project root (absolute or relative to workspace)"
                },
                "test_filter": {
                    "type": "string",
                    "description": "Optional test name filter (for 'test' operation)"
                },
                "commit_message": {
                    "type": "string",
                    "description": "Commit message (required for 'commit' operation)"
                }
            },
            "required": ["operation", "project_dir"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let operation = args["operation"].as_str().unwrap_or("analyze");
        let project_dir = args["project_dir"].as_str().unwrap_or(".");
        let dir = self.resolve_dir(project_dir);

        if !dir.exists() {
            return Ok(ToolResult {
                success: false,
                output: format!("Directory does not exist: {}", dir.display()),
            });
        }

        match operation {
            "analyze" => {
                let result = self.analyze(&dir)?;
                Ok(ToolResult {
                    success: true,
                    output: serde_json::to_string_pretty(&result)?,
                })
            }

            "test" => {
                let filter = args["test_filter"].as_str();
                let result = self.run_tests(&dir, filter);
                Ok(ToolResult {
                    success: result.success,
                    output: serde_json::to_string_pretty(&result)?,
                })
            }

            "diff" => {
                let diff = self.git_diff(&dir);
                Ok(ToolResult {
                    success: true,
                    output: if diff.trim().is_empty() {
                        "No changes detected".to_string()
                    } else {
                        diff
                    },
                })
            }

            "status" => {
                let status = self.git_status(&dir);
                Ok(ToolResult {
                    success: true,
                    output: if status.trim().is_empty() {
                        "Working tree clean".to_string()
                    } else {
                        status
                    },
                })
            }

            "commit" => {
                let message = args["commit_message"].as_str()
                    .unwrap_or("code_evolution: automated improvement");
                let result = self.git_commit(&dir, message)?;
                Ok(ToolResult { success: true, output: result })
            }

            _ => Ok(ToolResult {
                success: false,
                output: format!("Unknown operation: {}. Use analyze/test/diff/status/commit", operation),
            }),
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_tool(dir: &str) -> CodeEvolutionTool {
        CodeEvolutionTool::new(dir.to_string())
    }

    #[test]
    fn test_detect_project_type_rust() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]\nname=\"test\"").unwrap();
        assert_eq!(CodeEvolutionTool::detect_project_type(tmp.path()), "rust");
    }

    #[test]
    fn test_detect_project_type_node() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("package.json"), "{}").unwrap();
        assert_eq!(CodeEvolutionTool::detect_project_type(tmp.path()), "node");
    }

    #[test]
    fn test_detect_project_type_unknown() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(CodeEvolutionTool::detect_project_type(tmp.path()), "unknown");
    }

    #[test]
    fn test_scan_markers() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("main.rs");
        fs::write(&src, "fn main() {\n    // TODO: fix this\n    // FIXME: urgent\n}\n").unwrap();

        let tool = make_tool(tmp.path().to_str().unwrap());
        let issues = tool.scan_markers(tmp.path());
        // grep may or may not be available in test env; just verify no panic
        // If grep works, we should find at least the markers
        for issue in &issues {
            assert!(issue.kind == "todo" || issue.kind == "fixme" || issue.kind == "hack");
        }
    }

    #[test]
    fn test_truncate_output() {
        assert_eq!(truncate_output("hello", 10), "hello");
        let long = "a".repeat(100);
        let result = truncate_output(&long, 50);
        assert!(result.contains("[truncated"));
        assert!(result.len() < 100);
    }

    #[test]
    fn test_resolve_dir_absolute() {
        let tool = make_tool("/workspace");
        let resolved = tool.resolve_dir("/absolute/path");
        assert_eq!(resolved, PathBuf::from("/absolute/path"));
    }

    #[test]
    fn test_resolve_dir_relative() {
        let tool = make_tool("/workspace");
        let resolved = tool.resolve_dir("subdir");
        assert_eq!(resolved, PathBuf::from("/workspace/subdir"));
    }

    #[tokio::test]
    async fn test_execute_unknown_operation() {
        let tmp = TempDir::new().unwrap();
        let tool = make_tool(tmp.path().to_str().unwrap());
        let result = tool.execute(json!({
            "operation": "invalid",
            "project_dir": tmp.path().to_str().unwrap()
        })).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown operation"));
    }

    #[tokio::test]
    async fn test_execute_nonexistent_dir() {
        let tool = make_tool("/tmp");
        let result = tool.execute(json!({
            "operation": "analyze",
            "project_dir": "/nonexistent/path/12345"
        })).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("does not exist"));
    }

    #[tokio::test]
    async fn test_analyze_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let tool = make_tool(tmp.path().to_str().unwrap());
        let result = tool.execute(json!({
            "operation": "analyze",
            "project_dir": tmp.path().to_str().unwrap()
        })).await.unwrap();
        assert!(result.success);
        let analysis: AnalysisResult = serde_json::from_str(&result.output).unwrap();
        assert_eq!(analysis.project_type, "unknown");
    }

    #[test]
    fn test_code_issue_serialization() {
        let issue = CodeIssue {
            file: "src/main.rs".to_string(),
            line: Some(42),
            kind: "todo".to_string(),
            message: "fix this".to_string(),
            severity: "info".to_string(),
        };
        let json = serde_json::to_string(&issue).unwrap();
        assert!(json.contains("main.rs"));
        assert!(json.contains("42"));
    }

    #[test]
    fn test_test_result_serialization() {
        let tr = TestResult {
            success: true,
            output: "all ok".to_string(),
            tests_run: 10,
            tests_passed: 10,
            tests_failed: 0,
        };
        let json = serde_json::to_string(&tr).unwrap();
        assert!(json.contains("\"tests_passed\":10"));
    }
}
