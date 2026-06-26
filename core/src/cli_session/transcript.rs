//! Locate + read each CLI's on-disk transcript and reuse the live parsers to turn
//! it into events (Source::Transcript). Used by the normalizer as the second source.

use crate::cli_session::event::{CliEvent, EventKind, Fidelity, Source};
use crate::cli_session::parse;
use std::path::PathBuf;

/// claude writes its session JSONL under ~/.claude/projects/<slug>/<session>.jsonl.
/// Returns the transcript-derived events (same parser, re-tagged Source::Transcript).
pub fn claude_transcript_events(session_jsonl: &PathBuf) -> Vec<CliEvent> {
    let body = std::fs::read_to_string(session_jsonl).unwrap_or_default();
    let lines: Vec<&str> = body.lines().collect();
    retag(parse::parse_claude_stream(&lines))
}

/// agy writes ~/.gemini/antigravity-cli/brain/<id>/.system_generated/logs/transcript.jsonl.
/// v1: pull any line's `text` field as AssistantText (pinned by the captured fixture).
pub fn agy_transcript_events(transcript_jsonl: &PathBuf) -> Vec<CliEvent> {
    let body = std::fs::read_to_string(transcript_jsonl).unwrap_or_default();
    let mut out = Vec::new();
    for line in body.lines() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(t) = v.get("text").and_then(|t| t.as_str()) {
                out.push(CliEvent::new(
                    EventKind::AssistantText { delta: t.to_string() },
                    Fidelity::StructuredBestEffort,
                    Source::Transcript));
            }
        }
    }
    out
}

fn retag(mut events: Vec<CliEvent>) -> Vec<CliEvent> {
    for e in &mut events { e.source = Source::Transcript; }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_transcript_yields_empty_not_panic() {
        let events = claude_transcript_events(&PathBuf::from("/no/such/file.jsonl"));
        assert!(events.is_empty());
    }

    #[test]
    fn agy_missing_transcript_yields_empty() {
        let events = agy_transcript_events(&PathBuf::from("/no/such/agy.jsonl"));
        assert!(events.is_empty());
    }
}
