//! Pure per-CLI parsers: raw output lines -> Vec<CliEvent>. No I/O, no process.

use crate::cli_session::event::{CliEvent, EventKind, Fidelity, Source};
use serde_json::Value;

/// Parse Claude Code `--output-format stream-json` NDJSON (one JSON object per line).
pub fn parse_claude_stream(lines: &[&str]) -> Vec<CliEvent> {
    let mut out = Vec::new();
    let mut seen_session = false;
    let emit = |out: &mut Vec<CliEvent>, e: EventKind| {
        out.push(CliEvent::new(e, Fidelity::StructuredBestEffort, Source::LiveStream));
    };
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if !seen_session {
            if let Some(id) = v.get("session_id").and_then(|s| s.as_str()) {
                emit(&mut out, EventKind::SessionStarted { id: id.to_string() });
                seen_session = true;
            }
        }
        match v.get("type").and_then(|t| t.as_str()) {
            Some("assistant") => {
                if let Some(content) = v.pointer("/message/content").and_then(|c| c.as_array()) {
                    for block in content {
                        match block.get("type").and_then(|t| t.as_str()) {
                            Some("text") => {
                                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                    emit(&mut out, EventKind::AssistantText { delta: t.to_string() });
                                }
                            }
                            Some("tool_use") => {
                                let name = block
                                    .get("name")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let args = block.get("input").cloned().unwrap_or(Value::Null);
                                emit(&mut out, EventKind::ToolCall { name, args });
                            }
                            _ => {}
                        }
                    }
                }
            }
            Some("result") => {
                let it = v
                    .pointer("/usage/input_tokens")
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                let ot = v
                    .pointer("/usage/output_tokens")
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                let cost = v
                    .get("total_cost_usd")
                    .and_then(|n| n.as_f64())
                    .unwrap_or(0.0);
                emit(&mut out, EventKind::Usage { input_tokens: it, output_tokens: ot, cost_usd: cost });
                let stop = v
                    .get("subtype")
                    .and_then(|s| s.as_str())
                    .unwrap_or("end_turn")
                    .to_string();
                emit(&mut out, EventKind::TurnDone { stop_reason: stop });
            }
            _ => {}
        }
    }
    out
}

/// Parse `codex exec --json` JSONL. Real envelope: thread.started{thread_id} ->
/// turn.started -> item.completed{item:{type,text}} -> turn.completed{usage}.
pub fn parse_codex_jsonl(lines: &[&str]) -> Vec<CliEvent> {
    let mut out = Vec::new();
    let emit = |out: &mut Vec<CliEvent>, e: EventKind| {
        out.push(CliEvent::new(e, Fidelity::StructuredBestEffort, Source::LiveStream));
    };
    for line in lines {
        let v: Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("thread.started") => {
                if let Some(id) = v.get("thread_id").and_then(|s| s.as_str()) {
                    emit(&mut out, EventKind::SessionStarted { id: id.to_string() });
                }
            }
            Some("item.completed") => {
                let item = v.get("item");
                let itype = item
                    .and_then(|i| i.get("type"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                if itype == "agent_message" {
                    if let Some(t) = item.and_then(|i| i.get("text")).and_then(|t| t.as_str()) {
                        emit(&mut out, EventKind::AssistantText { delta: t.to_string() });
                    }
                } else {
                    emit(&mut out, EventKind::ToolCall {
                        name: itype.to_string(),
                        args: item.cloned().unwrap_or(Value::Null),
                    });
                }
            }
            Some("turn.completed") => {
                let it = v
                    .pointer("/usage/input_tokens")
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                let ot = v
                    .pointer("/usage/output_tokens")
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                emit(&mut out, EventKind::Usage { input_tokens: it, output_tokens: ot, cost_usd: 0.0 });
                emit(&mut out, EventKind::TurnDone { stop_reason: "completed".into() });
            }
            _ => {}
        }
    }
    out
}

/// Parse `opencode run --format json` NDJSON. Exact paths pinned by the fixture.
/// Envelope: step_start -> tool_use{part.type="tool", part.state.output} ->
/// step_finish -> step_start -> text{part.text} -> step_finish.
pub fn parse_opencode_json(lines: &[&str]) -> Vec<CliEvent> {
    let mut out = Vec::new();
    let mut seen_session = false;
    let emit = |out: &mut Vec<CliEvent>, e: EventKind| {
        out.push(CliEvent::new(e, Fidelity::StructuredBestEffort, Source::LiveStream));
    };
    for line in lines {
        let v: Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if !seen_session {
            if let Some(id) = v.get("sessionID").or_else(|| v.get("session_id")).and_then(|s| s.as_str()) {
                emit(&mut out, EventKind::SessionStarted { id: id.to_string() });
                seen_session = true;
            }
        }
        // assistant text: type="text", part.text = "PONG"
        if v.get("type").and_then(|t| t.as_str()) == Some("text") {
            if let Some(t) = v.pointer("/part/text").and_then(|s| s.as_str()) {
                emit(&mut out, EventKind::AssistantText { delta: t.to_string() });
            }
        }
        // tool part: type="tool_use", part.type="tool", part.state.output = "PONG\n"
        if v.pointer("/part/type").and_then(|t| t.as_str()) == Some("tool") {
            let name = v
                .pointer("/part/tool")
                .and_then(|t| t.as_str())
                .unwrap_or("tool")
                .to_string();
            let output = v
                .pointer("/part/state/output")
                .and_then(|o| o.as_str())
                .unwrap_or("")
                .to_string();
            emit(&mut out, EventKind::ToolResult { name, output, ok: true });
        }
        if v.get("type").and_then(|t| t.as_str()) == Some("step_finish") {
            emit(&mut out, EventKind::TurnDone { stop_reason: "step_finish".into() });
        }
    }
    out
}

/// External plain-text oneshot: prints ONLY the final answer to stdout (no banner, no
/// JSON, no session line). Collapse the captured lines into one AssistantText plus a
/// TurnDone. StructuredBestEffort: clean stdout, but not a verifiable per-event stream.
pub fn parse_external_plain(lines: &[&str]) -> Vec<CliEvent> {
    let answer = strip_ansi(&lines.join("\n")).trim().to_string();
    let mut out = Vec::new();
    if !answer.is_empty() {
        out.push(CliEvent::new(
            EventKind::AssistantText { delta: answer },
            Fidelity::StructuredBestEffort,
            Source::LiveStream,
        ));
    }
    out.push(CliEvent::new(
        EventKind::TurnDone { stop_reason: "oneshot".into() },
        Fidelity::StructuredBestEffort,
        Source::LiveStream,
    ));
    out
}

/// External JSON-payload gateway: prints one JSON object whose reply is the concatenation
/// of `payloads[].text`. Stdout may contain interleaved non-JSON plugin-log lines, so we
/// extract the outermost `{...}` spans before parsing and fall back to raw ansi-stripped
/// text on any failure.
pub fn parse_external_json(lines: &[&str]) -> Vec<CliEvent> {
    let raw = lines.join("\n");
    let answer = external_json_reply(&raw).unwrap_or_else(|| strip_ansi(&raw).trim().to_string());
    let mut out = Vec::new();
    if !answer.is_empty() {
        out.push(CliEvent::new(
            EventKind::AssistantText { delta: answer },
            Fidelity::StructuredBestEffort,
            Source::LiveStream,
        ));
    }
    out.push(CliEvent::new(
        EventKind::TurnDone { stop_reason: "agent_turn".into() },
        Fidelity::StructuredBestEffort,
        Source::LiveStream,
    ));
    out
}

/// Extract `payloads[].text` from a JSON-payload gateway's stdout. Interleaved non-JSON
/// plugin-log lines and multiple JSON objects are tolerated: scan every balanced top-level
/// `{...}` span (string-aware), parse each, and return the text of the LAST object that
/// carries a non-empty `payloads[].text`. `None` if none found (caller falls back to raw).
fn external_json_reply(raw: &str) -> Option<String> {
    let mut best: Option<String> = None;
    for span in json_object_spans(raw) {
        let v: Value = match serde_json::from_str(&span) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let payloads = match v.get("payloads").and_then(|p| p.as_array()) {
            Some(p) => p,
            None => continue,
        };
        let mut s = String::new();
        for p in payloads {
            if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                s.push_str(t);
            }
        }
        let s = s.trim();
        if !s.is_empty() {
            best = Some(s.to_string());
        }
    }
    best
}

/// Yield each balanced top-level `{...}` span in `raw` (brace-depth scan that ignores
/// braces appearing inside JSON strings). Each span starts at a `{` and ends at its
/// matching `}`; `{`/`}`/`"` are ASCII so the byte indices are always char boundaries.
fn json_object_spans(raw: &str) -> Vec<String> {
    let bytes = raw.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let start = i;
        let mut depth = 0i32;
        let mut in_str = false;
        let mut esc = false;
        while i < bytes.len() {
            let c = bytes[i];
            if in_str {
                if esc {
                    esc = false;
                } else if c == b'\\' {
                    esc = true;
                } else if c == b'"' {
                    in_str = false;
                }
            } else {
                match c {
                    b'"' => in_str = true,
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            i += 1;
        }
        if depth == 0 {
            spans.push(raw[start..i].to_string());
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    const CLAUDE_FIXTURE: &str =
        include_str!("../../tests/fixtures/cli_session/claude.stream-json.ndjson");

    #[test]
    fn claude_fixture_yields_session_text_and_done() {
        let lines: Vec<&str> = CLAUDE_FIXTURE.lines().collect();
        let events = parse_claude_stream(&lines);
        assert!(
            events.iter().any(|e| matches!(e.event, EventKind::SessionStarted { .. })),
            "no SessionStarted"
        );
        assert!(
            events.iter().any(|e| matches!(e.event, EventKind::AssistantText { .. })),
            "no AssistantText"
        );
        assert!(
            events.iter().any(|e| matches!(e.event, EventKind::TurnDone { .. })),
            "no TurnDone"
        );
    }

    #[test]
    fn external_plain_oneshot_yields_answer_and_done() {
        let events = parse_external_plain(&["ONESHOT_ANSWER"]);
        assert!(
            events.iter().any(|e| matches!(&e.event,
                EventKind::AssistantText { delta } if delta == "ONESHOT_ANSWER")),
            "no AssistantText with the answer, got {events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(e.event, EventKind::TurnDone { .. })),
            "no TurnDone"
        );
    }

    #[test]
    fn external_json_extracts_payload_text_despite_log_leak() {
        // a leaked plugin-log line precedes the JSON object
        let lines = [
            "[plugins] plugins.allow is empty; discovered non-bundled plugins may auto-load",
            "{\"payloads\":[{\"text\":\"JSON_ANSWER\",\"mediaUrl\":null}],\"meta\":{\"durationMs\":12}}",
        ];
        let events = parse_external_json(&lines);
        assert!(
            events.iter().any(|e| matches!(&e.event,
                EventKind::AssistantText { delta } if delta == "JSON_ANSWER")),
            "did not extract payloads[].text, got {events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(e.event, EventKind::TurnDone { .. })),
            "no TurnDone"
        );
    }

    #[test]
    fn external_json_picks_payload_object_among_multiple_json_objects() {
        // a JSON log object (no payloads) precedes the real result object
        let lines = [
            "{\"level\":\"warn\",\"msg\":\"plugin {x} loaded\"}",
            "{\"payloads\":[{\"text\":\"JSON_ANSWER\"}],\"meta\":{\"durationMs\":3}}",
        ];
        let events = parse_external_json(&lines);
        assert!(
            events.iter().any(|e| matches!(&e.event,
                EventKind::AssistantText { delta } if delta == "JSON_ANSWER")),
            "did not pick the payloads-bearing object, got {events:?}"
        );
    }

    #[test]
    fn external_plain_preserves_non_ascii_and_strips_ansi() {
        // ANSI-wrapped CJK answer: strip_ansi must not corrupt the multibyte chars.
        let events = parse_external_plain(&["\u{1b}[32m你好 PLAIN_ANSWER\u{1b}[0m"]);
        assert!(
            events.iter().any(|e| matches!(&e.event,
                EventKind::AssistantText { delta } if delta == "你好 PLAIN_ANSWER")),
            "non-ascii corrupted or ansi not stripped, got {events:?}"
        );
    }
}

/// Parse agy's two sources into events:
///  - klog lines (from `--log-file`) -> lifecycle: SessionStarted (Created conversation
///    <uuid>), TurnDone (Stopping conversation stream), Error (send failures).
///  - stdout (PTY-captured plain text) -> one AssistantText (the answer).
/// stdout is tagged PtyScraped; klog-derived events are StructuredBestEffort.
pub fn parse_agy(stdout: &str, klog_lines: &[&str]) -> Vec<CliEvent> {
    let mut out = Vec::new();
    for line in klog_lines {
        if let Some(rest) = line.split("Created conversation ").nth(1) {
            let id = rest.split_whitespace().next().unwrap_or("").to_string();
            if !id.is_empty() {
                out.push(CliEvent::new(EventKind::SessionStarted { id },
                    Fidelity::StructuredBestEffort, Source::Klog));
            }
        }
        if line.contains("Print mode: SendUserMessage failed") {
            out.push(CliEvent::new(
                EventKind::Error { error_kind: "send_failed".into(), detail: (*line).to_string() },
                Fidelity::StructuredBestEffort, Source::Klog));
        }
    }
    let answer = strip_ansi(stdout).trim().to_string();
    if !answer.is_empty() {
        out.push(CliEvent::new(EventKind::AssistantText { delta: answer },
            Fidelity::PtyScraped, Source::LiveStream));
    }
    if klog_lines.iter().any(|l| l.contains("Stopping conversation stream"))
        || klog_lines.iter().any(|l| l.contains("Language server shutting down")) {
        out.push(CliEvent::new(EventKind::TurnDone { stop_reason: "stream_stopped".into() },
            Fidelity::StructuredBestEffort, Source::Klog));
    }
    out
}

/// Strip ANSI/control sequences a ConPTY/pty capture carries (incl. private-mode
/// like ESC[?1004h), mirroring the skill's ask.sh clean().
pub fn strip_ansi(s: &str) -> String {
    // Char-oriented: a byte-oriented `bytes[i] as char` corrupts multibyte UTF-8
    // (e.g. CJK answers), so iterate over chars and drop only ESC-prefixed sequences.
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // CSI sequence: ESC '[' ... <final ASCII letter>. Drop the whole run; a
            // lone ESC (or other ESC-prefixed control) is dropped too.
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&n) = chars.peek() {
                    chars.next();
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod agy_tests {
    use super::*;
    use crate::cli_session::event::EventKind;
    const AGY_OUT: &str = include_str!("../../tests/fixtures/cli_session/agy.stdout.txt");
    const AGY_KLOG: &str = include_str!("../../tests/fixtures/cli_session/agy.klog.log");

    #[test]
    fn agy_fixture_recovers_session_text_and_done() {
        let klog: Vec<&str> = AGY_KLOG.lines().collect();
        let ev = parse_agy(AGY_OUT, &klog);
        assert!(ev.iter().any(|e| matches!(e.event, EventKind::SessionStarted { .. })), "no session: {ev:?}");
        assert!(ev.iter().any(|e| matches!(&e.event, EventKind::AssistantText { delta } if delta.contains("PONG"))), "no PONG: {ev:?}");
        assert!(ev.iter().any(|e| matches!(e.event, EventKind::TurnDone { .. })), "no done: {ev:?}");
    }

    #[test]
    fn strip_ansi_removes_escape_sequences() {
        assert_eq!(strip_ansi("\x1b[?1004h\x1b[1mPONG\x1b[0m"), "PONG");
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;
    use crate::cli_session::event::EventKind;
    const CODEX: &str = include_str!("../../tests/fixtures/cli_session/codex.exec.jsonl");
    const OPENCODE: &str = include_str!("../../tests/fixtures/cli_session/opencode.run.json");

    #[test]
    fn codex_fixture_session_text_done() {
        let lines: Vec<&str> = CODEX.lines().collect();
        let ev = parse_codex_jsonl(&lines);
        assert!(ev.iter().any(|e| matches!(e.event, EventKind::SessionStarted { .. })), "no session: {ev:?}");
        assert!(ev.iter().any(|e| matches!(&e.event, EventKind::AssistantText { delta } if delta.contains("PONG"))), "no PONG text: {ev:?}");
        assert!(ev.iter().any(|e| matches!(e.event, EventKind::TurnDone { .. })), "no done: {ev:?}");
    }

    #[test]
    fn opencode_fixture_session_and_done() {
        let lines: Vec<&str> = OPENCODE.lines().collect();
        let ev = parse_opencode_json(&lines);
        assert!(ev.iter().any(|e| matches!(e.event, EventKind::SessionStarted { .. })), "no session: {ev:?}");
        assert!(ev.iter().any(|e| matches!(e.event, EventKind::TurnDone { .. })), "no done: {ev:?}");
        // the PONG answer must appear somewhere (assistant text OR a tool output)
        assert!(ev.iter().any(|e| matches!(&e.event, EventKind::AssistantText { delta } if delta.contains("PONG"))
            || matches!(&e.event, EventKind::ToolResult { output, .. } if output.contains("PONG"))), "no PONG anywhere: {ev:?}");
    }
}
