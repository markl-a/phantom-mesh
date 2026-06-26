//! Source-redundant reconcile: cross-check the live event stream against the
//! transcript-derived stream so a dropped/garbled live stream is still complete,
//! and upgrade fidelity where both agree.

use crate::cli_session::event::{CliEvent, EventKind, Fidelity, Source};

fn kind_disc(e: &EventKind) -> u8 {
    match e {
        EventKind::SessionStarted { .. } => 0,
        EventKind::AssistantText { .. } => 1,
        EventKind::ToolCall { .. } => 2,
        EventKind::ToolResult { .. } => 3,
        EventKind::Usage { .. } => 4,
        EventKind::TurnDone { .. } => 5,
        EventKind::Error { .. } => 6,
    }
}

/// Reconcile the live-stream events with the transcript-derived events (spec §2):
///  - a live event whose `kind` is also present in the transcript -> upgrade to
///    StructuredVerified;
///  - a transcript event with no matching live event (live dropped it) -> emit it
///    from the transcript (rescues agy silent-drop / opencode race), Source::Transcript;
///  - live-only events keep their original fidelity.
pub fn reconcile(live: Vec<CliEvent>, transcript: Vec<CliEvent>) -> Vec<CliEvent> {
    let mut out: Vec<CliEvent> = Vec::new();
    for mut e in live {
        let matched = transcript.iter().any(|t| kind_disc(&t.event) == kind_disc(&e.event));
        if matched {
            e.fidelity = Fidelity::StructuredVerified;
        }
        out.push(e);
    }
    for t in transcript {
        let already = out.iter().any(|o| kind_disc(&o.event) == kind_disc(&t.event));
        if !already {
            out.push(CliEvent::new(t.event, Fidelity::StructuredBestEffort, Source::Transcript));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(k: EventKind, f: Fidelity, s: Source) -> CliEvent { CliEvent::new(k, f, s) }

    #[test]
    fn live_dropped_event_is_recovered_from_transcript() {
        // live only saw SessionStarted; the answer silent-dropped (agy #76) but the
        // transcript has the AssistantText.
        let live = vec![ev(EventKind::SessionStarted { id: "x".into() },
            Fidelity::StructuredBestEffort, Source::LiveStream)];
        let transcript = vec![
            ev(EventKind::SessionStarted { id: "x".into() }, Fidelity::StructuredBestEffort, Source::Transcript),
            ev(EventKind::AssistantText { delta: "the answer".into() }, Fidelity::StructuredBestEffort, Source::Transcript),
        ];
        let out = reconcile(live, transcript);
        assert!(out.iter().any(|e| matches!(e.event, EventKind::SessionStarted { .. })
            && e.fidelity == Fidelity::StructuredVerified), "session not upgraded: {out:?}");
        let recovered = out.iter().find(|e| matches!(e.event, EventKind::AssistantText { .. })).expect("no recovered text");
        assert_eq!(recovered.source, Source::Transcript);
    }

    #[test]
    fn live_only_event_keeps_its_original_fidelity() {
        // Doc contract bullet 3: a live event with NO corroborating transcript
        // kind keeps its original fidelity — only events the transcript confirms
        // are upgraded to StructuredVerified. A regression that upgraded every
        // live event would falsely mark unverified live output as verified,
        // defeating the whole point of the source-redundant cross-check.
        let live = vec![
            ev(EventKind::SessionStarted { id: "x".into() }, Fidelity::StructuredBestEffort, Source::LiveStream),
            ev(EventKind::AssistantText { delta: "live only".into() }, Fidelity::StructuredBestEffort, Source::LiveStream),
        ];
        // Transcript corroborates ONLY SessionStarted, not the AssistantText.
        let transcript = vec![
            ev(EventKind::SessionStarted { id: "x".into() }, Fidelity::StructuredBestEffort, Source::Transcript),
        ];
        let out = reconcile(live, transcript);

        let session = out.iter().find(|e| matches!(e.event, EventKind::SessionStarted { .. })).unwrap();
        assert_eq!(session.fidelity, Fidelity::StructuredVerified, "corroborated event must upgrade");

        let text = out.iter().find(|e| matches!(e.event, EventKind::AssistantText { .. })).unwrap();
        assert_eq!(text.fidelity, Fidelity::StructuredBestEffort, "live-only event must NOT upgrade");
        assert_eq!(text.source, Source::LiveStream, "live-only event keeps its source");

        // Transcript had nothing live didn't already have → no events appended.
        assert_eq!(out.len(), 2, "no spurious events added: {out:?}");
    }
}
