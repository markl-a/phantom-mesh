//! Capability-based routing for tools.
//!
//! Maps tool names to required capability IDs. Tools not listed have no special
//! requirements and can run on any node. This replaces the hardcoded `ToolRouting`
//! enum with dynamic, capability-driven dispatch.

use once_cell::sync::Lazy;
use std::collections::HashMap;

/// Map of tool_name -> required capability IDs.
/// Tools not listed here have no special requirements.
static TOOL_CAPABILITIES: Lazy<HashMap<&str, Vec<&str>>> = Lazy::new(|| {
    let mut m = HashMap::new();

    // Shell tools
    m.insert("shell", vec!["shell"]);
    m.insert("shell_session", vec!["shell"]);
    m.insert("cli_anything", vec!["shell"]);

    // File tools (desktop only)
    m.insert("file_read", vec!["file_system"]);
    m.insert("file_write", vec!["file_system"]);
    m.insert("file_edit", vec!["file_system"]);
    m.insert("glob_search", vec!["file_system"]);
    m.insert("content_search", vec!["file_system"]);
    m.insert("archive_extract", vec!["shell"]);

    // Shell-dependent export tools
    m.insert("pdf_export", vec!["shell"]);
    m.insert("docx_export", vec!["shell"]);
    m.insert("xlsx_export", vec!["shell"]);
    m.insert("video_compose", vec!["shell"]);

    // Display/UI tools
    m.insert("screenshot", vec!["shell"]);
    m.insert("computer_use", vec!["shell"]);

    // Code tools
    m.insert("ai_code", vec!["shell"]);
    m.insert("code_evolution", vec!["shell", "file_system"]);
    m.insert("scaffold_saas", vec!["file_system"]);

    // Network tools (all devices have network)
    // web_search, http_request, etc. -> no special requirements

    // Deployment
    m.insert("blog_publish", vec!["shell", "file_system"]);

    m
});

/// Get required capabilities for a tool by name.
pub fn required_for_tool(tool_name: &str) -> Vec<String> {
    TOOL_CAPABILITIES
        .get(tool_name)
        .map(|caps| caps.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

/// Check if a node with given capabilities can run a tool.
pub fn can_run_tool(tool_name: &str, node_capabilities: &[String]) -> bool {
    let required = required_for_tool(tool_name);
    required.iter().all(|req| node_capabilities.contains(req))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_requires_shell_capability() {
        let caps = required_for_tool("shell");
        assert!(caps.contains(&"shell".to_string()));
    }

    #[test]
    fn test_web_search_no_special_requirements() {
        let caps = required_for_tool("web_search");
        assert!(caps.is_empty());
    }

    #[test]
    fn test_calculator_no_requirements() {
        let caps = required_for_tool("calculator");
        assert!(caps.is_empty());
    }

    #[test]
    fn test_can_run_tool_with_capability() {
        assert!(can_run_tool("shell", &["shell".to_string()]));
    }

    #[test]
    fn test_cannot_run_tool_without_capability() {
        assert!(!can_run_tool("shell", &["file_system".to_string()]));
    }

    #[test]
    fn test_can_run_tool_no_requirements() {
        // Any node can run "web_search" — even with no capabilities
        assert!(can_run_tool("web_search", &[]));
    }

    #[test]
    fn test_multi_capability_requirement() {
        // code_evolution needs shell + file_system; node has both
        assert!(can_run_tool(
            "code_evolution",
            &["shell".to_string(), "file_system".to_string()]
        ));
    }

    #[test]
    fn test_multi_capability_partial_miss() {
        // code_evolution needs shell + file_system; node has only shell
        assert!(!can_run_tool("code_evolution", &["shell".to_string()]));
    }
}
