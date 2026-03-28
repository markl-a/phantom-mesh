//! Integration tests for tool capability-based routing.

use phantom_mesh::tools::{can_run_tool, required_for_tool};

#[test]
fn test_shell_requires_shell_capability() {
    let caps = required_for_tool("shell");
    assert!(
        caps.contains(&"shell".to_string()),
        "shell tool should require 'shell' capability, got: {:?}",
        caps
    );
}

#[test]
fn test_web_search_no_special_requirements() {
    let caps = required_for_tool("web_search");
    assert!(
        caps.is_empty(),
        "web_search should have no special requirements, got: {:?}",
        caps
    );
}

#[test]
fn test_calculator_no_requirements() {
    let caps = required_for_tool("calculator");
    assert!(
        caps.is_empty(),
        "calculator should have no special requirements, got: {:?}",
        caps
    );
}

#[test]
fn test_can_run_tool_with_capability() {
    assert!(
        can_run_tool("shell", &["shell".to_string()]),
        "node with 'shell' capability should be able to run 'shell' tool"
    );
}

#[test]
fn test_cannot_run_tool_without_capability() {
    assert!(
        !can_run_tool("shell", &["file_system".to_string()]),
        "node without 'shell' capability should NOT be able to run 'shell' tool"
    );
}

#[test]
fn test_can_run_tool_no_requirements() {
    // Any node can run web_search, even with zero capabilities
    assert!(
        can_run_tool("web_search", &[]),
        "any node should be able to run 'web_search' (no requirements)"
    );
}

#[test]
fn test_multi_capability_requirement() {
    // code_evolution needs both shell + file_system; node has both
    let node_caps = vec!["shell".to_string(), "file_system".to_string()];
    assert!(
        can_run_tool("code_evolution", &node_caps),
        "node with shell + file_system should run code_evolution"
    );
}

#[test]
fn test_multi_capability_partial_miss() {
    // code_evolution needs shell + file_system; node only has shell
    let node_caps = vec!["shell".to_string()];
    assert!(
        !can_run_tool("code_evolution", &node_caps),
        "node with only shell should NOT run code_evolution (needs file_system too)"
    );
}
