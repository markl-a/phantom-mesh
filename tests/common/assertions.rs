//! Domain-specific assertion macros for e2e tests.

/// Assert an AgentResult's output contains the expected substring.
#[macro_export]
macro_rules! assert_agent_output_contains {
    ($result:expr, $expected:expr) => {
        assert!(
            $result.output.contains($expected),
            "Expected agent output to contain '{}', got: '{}'",
            $expected,
            $result.output
        );
    };
}

/// Assert an HTTP response has status 200 OK.
#[macro_export]
macro_rules! assert_http_ok {
    ($resp:expr) => {
        assert!(
            $resp.status().is_success(),
            "Expected 2xx, got {}",
            $resp.status(),
        );
    };
}

/// Assert the MockChannel received a reply containing the expected text.
#[macro_export]
macro_rules! assert_channel_replied {
    ($mock:expr, $expected:expr) => {
        let replies = $mock.drain_replies();
        let found = replies.iter().any(|(_, text)| text.contains($expected));
        assert!(
            found,
            "Expected a reply containing '{}', got: {:?}",
            $expected, replies
        );
    };
}

/// Assert an AgentResult made at least one tool call.
#[macro_export]
macro_rules! assert_agent_used_tools {
    ($result:expr) => {
        assert!(
            $result.tool_calls_made > 0,
            "Expected agent to make tool calls, but tool_calls_made = 0"
        );
    };
}
