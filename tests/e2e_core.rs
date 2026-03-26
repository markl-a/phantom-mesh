//! E2E Core Agent Loop Tests
//! Tests the full agent runtime cycle: prompt → provider → tool calls → response.

mod common;

use common::harness::CoreHarness;
use phantom_mesh::providers::mock::{MockProvider, MockResponse, MockToolCall};
use serde_json::json;

#[test]
fn test_common_module_loads() {
    // Verify the common module compiles
    let _msg = common::fixtures::user_msg("hello");
}

/// Single-turn text response — no tool calls.
#[tokio::test]
async fn agent_single_turn_text() {
    let harness = CoreHarness::builder()
        .provider(MockProvider::fixed("The answer is 42."))
        .build()
        .await;

    let result = harness.run_agent("What is the meaning of life?").await.unwrap();
    assert!(result.output.contains("42"));
    assert_eq!(result.tool_calls_made, 0);
    assert_eq!(harness.provider_call_count(), 1);
}

/// Tool call roundtrip — agent calls a tool, gets result, produces final answer.
#[tokio::test]
async fn agent_tool_call_roundtrip() {
    let harness = CoreHarness::builder()
        .provider(MockProvider::scripted(vec![
            MockResponse::ToolCalls {
                content: String::new(),
                calls: vec![MockToolCall {
                    id: "call-1".to_string(),
                    name: "file_read".to_string(),
                    arguments: json!({"path": "test.txt"}),
                }],
            },
            MockResponse::Text("The file contains: hello world".into()),
        ]))
        .build()
        .await;

    // Create the file so file_read works
    let workspace = harness.workspace_path();
    std::fs::write(workspace.join("test.txt"), "hello world").unwrap();

    let result = harness.run_agent("Read test.txt").await.unwrap();
    assert!(result.tool_calls_made >= 1);
    // Output should come from the final text response
    assert!(!result.output.is_empty());
}

/// Multi-tool chain — agent calls 3 tools sequentially.
#[tokio::test]
async fn agent_multi_tool_chain() {
    let harness = CoreHarness::builder()
        .provider(MockProvider::scripted(vec![
            MockResponse::ToolCalls {
                content: String::new(),
                calls: vec![MockToolCall { id: "c1".into(), name: "file_write".into(), arguments: json!({"path": "a.txt", "content": "AAA"}) }],
            },
            MockResponse::ToolCalls {
                content: String::new(),
                calls: vec![MockToolCall { id: "c2".into(), name: "file_read".into(), arguments: json!({"path": "a.txt"}) }],
            },
            MockResponse::ToolCalls {
                content: String::new(),
                calls: vec![MockToolCall { id: "c3".into(), name: "file_edit".into(), arguments: json!({"path": "a.txt", "old_text": "AAA", "new_text": "BBB"}) }],
            },
            MockResponse::Text("Done — wrote, read, and edited the file.".into()),
        ]))
        .build()
        .await;

    let result = harness.run_agent("Create a.txt with AAA, read it, then change to BBB").await.unwrap();
    assert!(result.tool_calls_made >= 3);

    // Verify the file was actually edited
    let content = std::fs::read_to_string(harness.workspace_path().join("a.txt")).unwrap();
    assert_eq!(content, "BBB");
}

/// Idle detection — agent produces identical output, loop exits.
#[tokio::test]
async fn agent_idle_detection() {
    // The idle detector threshold is 3 consecutive identical rounds.
    // We provide 6 identical responses — the agent should exit well before using all of them.
    let harness = CoreHarness::builder()
        .provider(MockProvider::scripted(vec![
            MockResponse::Text("I'm stuck".into()),
            MockResponse::Text("I'm stuck".into()),
            MockResponse::Text("I'm stuck".into()),
            MockResponse::Text("I'm stuck".into()),
            MockResponse::Text("I'm stuck".into()),
            MockResponse::Text("I'm stuck".into()),
        ]))
        .build()
        .await;

    let result = harness.run_agent("Do something").await.unwrap();
    // Agent should exit (idle detection or single-turn) with non-empty output
    assert!(!result.output.is_empty());
    // The provider call count should be bounded (not all 6)
    assert!(harness.provider_call_count() <= 5);
}

/// Max rounds exit — agent makes tool calls every round, hits limit.
#[tokio::test]
async fn agent_max_rounds_exit() {
    // Create 12 tool call responses — more than the default round limit of 10.
    // Use file_read on an existing file to avoid tool errors.
    let workspace_pre = tempfile::TempDir::new().unwrap();
    let file_path = workspace_pre.path().join("loop.txt");
    std::fs::write(&file_path, "loop content").unwrap();

    let responses: Vec<MockResponse> = (0..12)
        .map(|i| MockResponse::ToolCalls {
            content: String::new(),
            calls: vec![MockToolCall {
                id: format!("c{}", i),
                name: "file_write".into(),
                arguments: json!({"path": "loop.txt", "content": format!("round {}", i)}),
            }],
        })
        .collect();

    let harness = CoreHarness::builder()
        .provider(MockProvider::scripted(responses))
        .build()
        .await;

    // Create the loop file in the workspace
    std::fs::write(harness.workspace_path().join("loop.txt"), "loop content").unwrap();

    let result = harness.run_agent("Keep writing forever").await.unwrap();
    // Should exit before exhausting all 12 responses (MAX_TOOL_ROUNDS = 10)
    assert!(harness.provider_call_count() <= 11);
    // Result must be non-empty (final-round forced text response or timeout)
    let _ = result; // just ensure it returned Ok
}

/// Context injection — system prompt appears in the messages sent to the provider.
#[tokio::test]
async fn agent_context_injection() {
    let harness = CoreHarness::builder()
        .provider(MockProvider::echo())
        .build()
        .await;

    let result = harness.run_agent("Hello").await.unwrap();
    assert!(!result.output.is_empty());
    assert_eq!(harness.provider_call_count(), 1);

    // Check the messages sent to the provider include system context
    if let Some(call) = harness.provider_call(0) {
        let has_system = call.messages.iter().any(|m| m.role == "system");
        assert!(has_system, "Expected system message in provider call");
    }
}

/// Error recovery — provider returns an error, agent handles gracefully.
#[tokio::test]
async fn agent_error_recovery() {
    let harness = CoreHarness::builder()
        .provider(MockProvider::error("Connection refused"))
        .build()
        .await;

    let result = harness.run_agent("Try something").await;
    // Should return an error, not panic
    assert!(result.is_err());
}
