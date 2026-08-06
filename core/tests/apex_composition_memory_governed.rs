//! apex COMPOSITION capstone (ADDITIVE, TEST-ONLY): proves the abilities COMPOSE,
//! not just work in isolation. The single-ability tests each prove ONE layer
//! (owned_memory_loop_e2e = (2) store/recall/apply; offline_provider_resolution_hermetic
//! = (1) offline-first; apex4_*.rs = (4) governed drive loop). This test proves the
//! strongest tractable composition the PUBLIC API supports in-process:
//!
//!   (2) OWNED MEMORY  ->  (4) GOVERNED RUN, on the (1) OFFLINE-first substrate.
//!
//! A captured owned skill is stored and RECALLED ((2), the real public skill_wire
//! path) entirely OFFLINE -- no network, no live model, no `ort`/MiniLM embedder, so
//! recall runs the production FTS5-only keyword leg (that offline-first property IS
//! ability (1)). The action the governor then drives is DERIVED FROM the recalled
//! skill's own content, and fed through the real `governed_run::drive` loop ((4)),
//! which classifies + records/escalates it. The governance OUTCOME is therefore a
//! function of WHAT RECALL RETURNED: a recalled high-risk owned skill must escalate
//! and be recorded as a governance moment; a recalled low-risk owned skill must
//! auto-allow with no governance moment. This composes (2) and (4) through real public
//! APIs and FAILS if EITHER layer breaks:
//!   * if recall ((2)) returns the wrong skill / nothing, the derived tool flips (or is
//!     absent) and the governance arm's assertion fails;
//!   * if the governor ((4)) stops gating, the escalate/auto-allow assertions fail.
//! It is NOT a copy of the single-ability tests: owned_memory_loop_e2e never drives a
//! governed run, and the apex4_*.rs tests hand-build literal "Bash"/"Read" events --
//! they never derive the governed action from a recalled owned skill.
//!
//! SEAM (HONEST-BAIL -- what is NOT proven here, and why):
//!   1. There is NO in-process, deterministic skill->ToolCall compiler in the public
//!      API: production's skill->action translation is the LLM/agent (the (2) apply step
//!      renders `<recalled_skills>` into a prompt and the model emits tool calls). So
//!      THIS test performs that translation via a documented convention -- the recalled
//!      skill's NAME leads with the tool token it maps to (e.g. "Bash ..." / "Read ...").
//!      What is proven is that a recalled owned skill's action flows into, and is gated
//!      by, the governor -- not that the model picks the right tool.
//!   2. Ability (1) is exercised as the OFFLINE SUBSTRATE (zero network / live model /
//!      embedder), NOT as an in-process data dependency: the provider RESOLVER does not
//!      sit on the recall->drive path (`drive` consumes a pre-built event stream; recall
//!      hits sqlite). Wiring the resolver onto this path is not reachable in-process
//!      without a production change, so per HONEST-BAIL we do not fake it.
//! CI-visible (NOT `#[ignore]`).

use spectyn_mesh::cli_session::CliKind;
use spectyn_mesh::cli_session::event::{CliEvent, EventKind, Fidelity, Source};
use spectyn_mesh::coach_wire::RecallPolicy;
use spectyn_mesh::governed_run::escalation::MockEscalator;
use spectyn_mesh::governed_run::recorder::{MemRecorder, RunRecord};
use spectyn_mesh::governed_run::{drive, GovernPolicy, RunOutcome};
use spectyn_mesh::skill_wire::{recall_skills, store_skill, Skill};
use serde_json::json;

/// Build a single-event-then-done stream carrying one derived ToolCall.
fn tool_stream(tool: &str) -> std::sync::mpsc::Receiver<CliEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(CliEvent::new(
        EventKind::ToolCall { name: tool.to_string(), args: json!({}) },
        Fidelity::StructuredVerified,
        Source::LiveStream,
    ))
    .unwrap();
    tx.send(CliEvent::new(
        EventKind::TurnDone { stop_reason: "end".into() },
        Fidelity::StructuredVerified,
        Source::LiveStream,
    ))
    .unwrap();
    drop(tx);
    rx
}

/// Outcome of governing the action DERIVED from a recalled owned skill.
struct GovernedRecall {
    top_id: Option<String>,
    derived_tool: Option<String>,
    outcome: RunOutcome,
    escalated: bool,
    governance_recorded: bool,
    raw_toolcall_recorded: Option<String>,
}

/// Recall an owned skill OFFLINE for `query`, derive its action's tool from the
/// recalled skill's name (the documented skill->action convention -- see SEAM), and
/// drive that one action through the real governor. Returns what recall surfaced
/// AND how the governor treated the derived action.
fn recall_then_govern(query: &str) -> Result<GovernedRecall, String> {
    // (2) recall -- the REAL public skill_wire path, offline (FTS5-only production leg).
    let recall = recall_skills(query, RecallPolicy::default())
        .map_err(|e| format!("recall must not error: {e:?}"))?;
    let top = recall.skills.first().cloned();
    let top_id = top.as_ref().map(|s| s.id.clone());
    // Derive the governed action's tool from the recalled skill's own content.
    let derived_tool = top
        .as_ref()
        .and_then(|s| s.name.split_whitespace().next().map(|t| t.to_string()));

    let (outcome, escalated, governance_recorded, raw_toolcall_recorded) = match &derived_tool {
        Some(tool) => {
            // (4) governed run -- the REAL public drive loop over the derived action.
            let mut rec = MemRecorder::default();
            let mut esc = MockEscalator::default();
            let outcome = drive(
                CliKind::Codex,
                tool_stream(tool),
                &mut rec,
                &mut esc,
                &GovernPolicy::default(),
            );
            let escalated = esc.sent.iter().any(|s| s.contains(tool.as_str()));
            let governance_recorded = rec.records.iter().any(|r| {
                matches!(r, RunRecord::Governance { enforcement: "post_action_observed", .. })
            });
            let raw_toolcall_recorded = rec.records.iter().find_map(|r| match r {
                RunRecord::Event(e) => match &e.event {
                    EventKind::ToolCall { name, .. } => Some(name.clone()),
                    _ => None,
                },
                _ => None,
            });
            (outcome, escalated, governance_recorded, raw_toolcall_recorded)
        }
        None => (RunOutcome::Completed, false, false, None),
    };

    Ok(GovernedRecall {
        top_id,
        derived_tool,
        outcome,
        escalated,
        governance_recorded,
        raw_toolcall_recorded,
    })
}

#[test]
fn recalled_owned_skill_action_is_governed_offline() {
    // Hermetic: a scratch DB, no network. This is the ONLY test in its own
    // integration binary, so the process-global SPECTYN_DB_PATH mutation races
    // nothing (mirrors owned_memory_loop_e2e).
    let db = tempfile::NamedTempFile::new().expect("temp DB file");
    let saved = std::env::var_os("SPECTYN_DB_PATH");
    std::env::set_var("SPECTYN_DB_PATH", db.path());

    // (capture->extract) Two owned skills. By the documented skill->action convention
    // their NAME leads with the tool the action maps to: the deploy skill maps to a
    // HIGH-RISK "Bash" action; the inspect skill maps to a LOW-RISK "Read" action.
    // Their triggers/names share NO token, so a relevance-respecting recall keeps
    // them apart.
    let deploy = Skill {
        id: "sk-comp-deploy".into(),
        name: "Bash deploy the staging cluster".into(),
        trigger_pattern: "deploy staging cluster".into(),
        steps: vec!["ssh staging".into(), "run deploy.sh".into()],
        examples: vec![],
        version: 1,
        quality_score: 0.8,
        last_applied_at: 0,
        source_event_count: 3,
    };
    let inspect = Skill {
        id: "sk-comp-inspect".into(),
        name: "Read the service log files".into(),
        trigger_pattern: "inspect service logs".into(),
        steps: vec!["open log viewer".into()],
        examples: vec![],
        version: 1,
        quality_score: 0.6,
        last_applied_at: 0,
        source_event_count: 2,
    };

    let result = (|| -> Result<(GovernedRecall, GovernedRecall), String> {
        // (store) Both owned skills via the REAL public persistence path.
        store_skill(&deploy).map_err(|e| format!("store deploy: {e:?}"))?;
        store_skill(&inspect).map_err(|e| format!("store inspect: {e:?}"))?;
        // Recall each owned skill OFFLINE, then govern the action derived from it.
        let high = recall_then_govern("deploy staging cluster")?;
        let low = recall_then_govern("inspect service logs")?;
        Ok((high, low))
    })();

    // Restore SPECTYN_DB_PATH BEFORE asserting so a panicking assert never leaks the
    // temp path into the process for any later-linked test.
    match saved {
        Some(value) => std::env::set_var("SPECTYN_DB_PATH", value),
        None => std::env::remove_var("SPECTYN_DB_PATH"),
    }

    let (high, low) = result.expect("recall->govern composition round-trip");

    // -- HIGH-RISK arm: recalled owned skill -> governed run escalates + records --
    assert_eq!(
        high.top_id.as_deref(),
        Some("sk-comp-deploy"),
        "(2) the captured high-risk owned skill must be recalled TOP-1 for its trigger"
    );
    assert_eq!(
        high.derived_tool.as_deref(),
        Some("Bash"),
        "the governed action's tool is DERIVED from the recalled owned skill's name"
    );
    assert_eq!(
        high.outcome,
        RunOutcome::Completed,
        "(4) a post-action-observed high-risk run completes (records + alerts, no STOP)"
    );
    assert!(
        high.escalated,
        "(4) the action derived from the recalled HIGH-RISK owned skill MUST escalate"
    );
    assert!(
        high.governance_recorded,
        "(4) the recalled high-risk action is recorded as a governance moment"
    );
    assert_eq!(
        high.raw_toolcall_recorded.as_deref(),
        Some("Bash"),
        "(4) the governed action recorded in the flight transcript is the one recall derived"
    );

    // -- LOW-RISK arm: a DIFFERENT recalled owned skill auto-allows (governor tracks
    //    the recalled identity -- proves the high-risk arm wasn't escalating blindly) --
    assert_eq!(
        low.top_id.as_deref(),
        Some("sk-comp-inspect"),
        "(2) the captured low-risk owned skill must be recalled TOP-1 for its trigger"
    );
    assert_eq!(
        low.derived_tool.as_deref(),
        Some("Read"),
        "the low-risk governed action's tool is DERIVED from its recalled owned skill"
    );
    assert_eq!(low.outcome, RunOutcome::Completed, "(4) a low-risk run completes");
    assert!(
        !low.escalated,
        "(4) the action derived from a recalled LOW-RISK owned skill must auto-allow (no escalation)"
    );
    assert!(
        !low.governance_recorded,
        "(4) no governance moment for an auto-allowed low-risk recalled action"
    );
}
