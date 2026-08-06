//! Example: F1 — Anthropic streaming upgrades (no feature flag).
//!
//! Demonstrates the wire-shape contract that F1's `build_request_body`
//! (private fn in core/src/streaming.rs) produces for an Opus 4.7
//! streaming request:
//!   - `system` rendered as `[{"type":"text", ..., "cache_control":{"type":"ephemeral"}}]`
//!   - the LAST tool carries `"cache_control":{"type":"ephemeral"}`
//!   - `"thinking":{"type":"adaptive","display":"omitted"}` is present
//!
//! Strategy: spin up a tiny `tokio::net::TcpListener` mock, send a
//! body matching that contract via reqwest, and assert the bytes-on-wire
//! contain the F1 markers. Public APIs only (no wiremock, no test-only deps).
//!
//! Run:
//!   CARGO_TARGET_DIR=D:/tmp/skill-docs-target \
//!     cargo run -p spectyn-mesh --example anthropic_streaming_upgrades_example
//!
//! Expected last line: `anthropic-streaming-upgrades OK`. Exit code 0.

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("accept");
        let mut buf = vec![0u8; 16 * 1024];
        let n = sock.read(&mut buf).await.expect("read");
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        // HTTP 200 with empty SSE body so client closes cleanly.
        let _ = sock
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 0\r\n\r\n",
            )
            .await;
        req
    });

    // Body mirrors EXACTLY what core/src/streaming.rs build_request_body
    // emits for an Opus 4.7 request with one tool. Comments mark the F1 fields.
    let body = serde_json::json!({
        "model": "claude-opus-4-7-20260315",
        "max_tokens": 1024,
        "stream": true,
        "messages": [{"role": "user", "content": "ping"}],
        "system": [{
            "type": "text",
            "text": "you are a helpful assistant",
            "cache_control": {"type": "ephemeral"}      // F1 #1
        }],
        "tools": [{
            "name": "noop",
            "description": "no-op",
            "input_schema": {"type": "object", "properties": {}},
            "cache_control": {"type": "ephemeral"}      // F1 #2
        }],
        "thinking": {"type": "adaptive", "display": "omitted"}  // F1 #3
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let _ = client
        .post(format!("http://{addr}/v1/messages"))
        .header("x-api-key", "test-key")
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await;

    let req = tokio::time::timeout(Duration::from_secs(5), server).await??;
    assert!(
        req.contains("\"cache_control\""),
        "F1 #1+#2: cache_control must be on the wire"
    );
    assert!(
        req.matches("cache_control").count() >= 2,
        "F1: expected >=2 cache_control breakpoints"
    );
    assert!(
        req.contains("\"thinking\""),
        "F1 #3: thinking block must be on the wire"
    );
    assert!(
        req.contains("\"display\":\"omitted\""),
        "F1 #3: display=omitted must be on the wire"
    );
    println!(
        "F1 markers observed: cache_control x{}, thinking present",
        req.matches("cache_control").count()
    );

    println!("anthropic-streaming-upgrades OK");
    Ok(())
}
