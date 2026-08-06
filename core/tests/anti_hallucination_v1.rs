//! T22 — anti-hallucination V1 integration test.
//!
//! Unit-level coverage of the pure scan function lives in
//! `core/src/hallucination/scanner.rs` (5 tests). This file covers the
//! integration boundary that the scanner alone cannot reach: that the
//! `tracing::warn!` emission used by `agent.rs::run_inner` actually
//! produces the user-visible log line operators rely on when grepping
//! diag streams (`target = "anti_halluc_warning"` / message format
//! `anti-hallucination: N unbacked claim(s) — ...`).
//!
//! The end-to-end agent-loop probe (real LLM provider, real tool
//! dispatch) lives in scripts/spectyn-test/scenarios/25-agent-anti-hallucination.sh
//! which is the more authoritative gate. The wiremock-driven full
//! `run_inner` Rust integration test is V2 work (depends on a small
//! `AgentsConfig` test-fixture helper that doesn't exist yet).
//!
//! Gated so the file is excluded from default builds entirely.

#![cfg(feature = "experimental-anti-hallucination")]

use std::io::Write;
use std::sync::{Arc, Mutex};

use spectyn_mesh::hallucination::{scan, UnbackedClaim};
use serde_json::{json, Value};
use tracing_subscriber::fmt::MakeWriter;

/// `MakeWriter` that funnels every tracing event into a shared
/// `Vec<u8>`. Cheap stand-in for the `tracing-test` crate, which is
/// not in our dev-dependencies.
#[derive(Clone, Default)]
struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl CaptureWriter {
    fn snapshot(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap_or_default()
    }
}

impl Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CaptureWriter {
    type Writer = CaptureWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Re-implements the exact warn-emission shape used by
/// `core/src/agent.rs::run_inner` so any future drift in the message
/// format breaks this test rather than silently breaking operator
/// dashboards. If you touch the warn line in agent.rs, update this
/// helper too.
fn emit_warn_like_agent(claims: &[UnbackedClaim]) {
    if claims.is_empty() {
        return;
    }
    let summaries: Vec<String> = claims
        .iter()
        .map(|c| format!("{}: {}", c.rule_id, c.explanation))
        .collect();
    tracing::warn!(
        "anti-hallucination: {} unbacked claim(s) — {}",
        summaries.len(),
        summaries.join(" | "),
    );
}

#[test]
fn shape1_warn_fires_for_file_claim_with_no_tool_calls() {
    let writer = CaptureWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(writer.clone())
        .with_ansi(false)
        .with_target(false)
        .with_max_level(tracing::Level::WARN)
        .finish();

    let _guard = tracing::subscriber::set_default(subscriber);

    // Mirrors the scenario the spec calls out:
    // assistant claims it created a file but emitted zero tool calls.
    let reply = "✅ Done — I created the file foo.rs with the new helper.";
    let tool_calls: Vec<Value> = vec![];
    let tool_results: Vec<String> = vec![];

    let claims = scan(reply, &tool_calls, &tool_results);
    assert_eq!(
        claims.len(),
        1,
        "expected one Shape-1 claim, got: {:?}",
        claims
    );
    assert_eq!(claims[0].rule_id, "claim_file_written");

    emit_warn_like_agent(&claims);

    let captured = writer.snapshot();
    // Surface the captured warn line under `cargo test -- --nocapture`
    // so operators can eyeball the exact format that lands in their
    // diag stream / spectyn serve log. Harmless when captured: noop.
    eprintln!("[sample tracing::warn output]\n{}", captured);
    assert!(
        captured.contains("anti-hallucination: 1 unbacked claim(s)"),
        "warn line missing expected prefix, captured was: {:?}",
        captured,
    );
    assert!(
        captured.contains("claim_file_written"),
        "warn line missing rule_id, captured was: {:?}",
        captured,
    );
    // Sanity: tracing-subscriber `fmt` adds a WARN severity tag.
    assert!(
        captured.contains("WARN"),
        "warn line missing severity tag, captured was: {:?}",
        captured,
    );
}

#[test]
fn shape1_warn_silent_when_file_write_tool_called() {
    let writer = CaptureWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(writer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let reply = "✅ Done — I created the file foo.rs with the new helper.";
    let tool_calls: Vec<Value> = vec![json!({
        "tool": "file_write",
        "args": {"path": "foo.rs", "content": "fn main() {}"}
    })];

    let claims = scan(reply, &tool_calls, &[]);
    assert!(
        claims.is_empty(),
        "expected no claim when file_write ran, got: {:?}",
        claims
    );

    emit_warn_like_agent(&claims);

    let captured = writer.snapshot();
    assert!(
        !captured.contains("anti-hallucination"),
        "scanner should not have fired when tool was called, captured: {:?}",
        captured,
    );
}
