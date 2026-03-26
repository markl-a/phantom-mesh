//! Shared test fixtures — agent configs, messages, profiles.

use std::path::Path;

/// Write a minimal test agents.toml that routes everything to "mock" provider.
pub fn write_test_agents_toml(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("agents.toml");
    let content = r#"
[agent.master]
provider = "mock"
instructions = "You are a test agent. Use tools when asked."
tools = ["file_read", "file_edit", "file_write", "shell"]

[agent.coder]
provider = "mock"
instructions = "You are a coding agent."
tools = ["file_read", "file_edit", "shell"]
"#;
    std::fs::write(&path, content).unwrap();
    path
}

/// Create a ChatMessage with role and content.
pub fn msg(role: &str, content: &str) -> phantom_mesh::ChatMessage {
    phantom_mesh::ChatMessage {
        role: role.to_string(),
        content: content.to_string(),
        tool_calls: None,
        tool_call_id: None,
    }
}

/// Create a user message.
pub fn user_msg(text: &str) -> phantom_mesh::ChatMessage {
    msg("user", text)
}

/// Create an assistant message.
pub fn assistant_msg(text: &str) -> phantom_mesh::ChatMessage {
    msg("assistant", text)
}
