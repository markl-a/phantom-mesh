//! L1 — governed orchestration. Wraps an L0 cli_session under the governor +
//! flight-recorder + phone escalation. See
//! docs/superpowers/specs/2026-06-16-l1-governed-orchestration-design.md
pub mod decision;
pub mod escalation;
pub mod permission;
pub mod recorder;
pub mod run;

use crate::cli_session::CliKind;
use crate::cli_session::event::{CliEvent, EventKind};
use crate::execution_contract::{
    ApprovalDecision, ContractState, ExecutionContract, RiskLevel, apply,
};
use crate::tasks::approvals::{GateOutcome, gate};
use decision::{Enforcement, classify_event, enforcement_for};
use escalation::Escalator;
use recorder::{RunRecord, RunRecorder};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// The configurable governance knobs.
#[derive(Clone, Debug)]
pub struct GovernPolicy {
    /// On a no-response timeout for a blocking approval, this decision applies.
    /// Consumed when constructing the escalator (e.g. `PhoneEscalator`'s fallback).
    pub timeout_fallback: ApprovalDecision, // default: Deny
    /// Strict mode: a denied pre-action decision aborts the whole run rather than
    /// just skipping that one tool. Default `false` — claude continues with its
    /// other work after a single tool is denied (per-tool gate semantics).
    pub deny_aborts_run: bool,
    /// Apex ④ HARD-BRAKE: wall-clock budget. If `Some(secs)`, the run is aborted
    /// once it has been running longer than `secs` (an unattended run can't keep
    /// burning time without stopping). `None` (default) = no wall-clock limit.
    pub max_wall_secs: Option<u64>,
    /// Apex ④ HARD-BRAKE: token budget. If `Some(n)`, the run is aborted once the
    /// cumulative `output_tokens` reported by `Usage` events exceeds `n` (an
    /// unattended run can't keep burning money/tokens without stopping). `None`
    /// (default) = no token limit.
    pub max_output_tokens: Option<u64>,
    /// Apex ④ HARD-BRAKE: battery floor. If `Some(pct)`, the run is aborted once
    /// the device battery is read BELOW `pct` percent (don't keep burning a long
    /// unattended run when the device is about to die on battery). `None` (default)
    /// = no battery limit, behavior unchanged.
    ///
    /// NOTE on stuck/hang coverage: a run that *hangs* (no events, no progress) is
    /// already bounded — the L0 per-turn `timeout_secs` watchdog kills a stalled
    /// turn, and `max_wall_secs` above caps total run time. So we deliberately do
    /// NOT add a separate stuck-detector here; the budget + battery brakes plus the
    /// L0 watchdog already cover the "unattended run won't stop" failure modes.
    pub min_battery_pct: Option<u8>,
    /// SYS-C (operator-locked 2026-06-13, `design-decisions-2026-06-13.md` C-2):
    /// per-run opt-in to AUTO-CONTINUE a LOW-RISK governor escalation boundary
    /// (e.g. a soft budget warning) instead of pausing for the owner. Default
    /// `false` — the governor's default posture is fail-safe PAUSE (C-1). This
    /// opt-in NEVER applies to a high-risk / destructive boundary nor to a
    /// no-response timeout: those always force-pause (see
    /// [`GovernPolicy::governor_pauses_at`]), so opting in here can never turn
    /// ④ into the §12-excluded fire-and-forget AutoGPT form.
    pub auto_continue_low_risk: bool,
}
impl Default for GovernPolicy {
    fn default() -> Self {
        Self {
            timeout_fallback: ApprovalDecision::Deny,
            deny_aborts_run: false,
            max_wall_secs: None,
            max_output_tokens: None,
            min_battery_pct: None,
            auto_continue_low_risk: false,
        }
    }
}

/// SYS-C ④ — a governor escalation boundary. When an unattended run hits one of
/// these the governor must decide whether to PAUSE-and-escalate to the owner or
/// (only for a low-risk boundary a run explicitly opted into) AUTO-CONTINUE.
/// See [`GovernPolicy::governor_pauses_at`] for the policy and its hard
/// guardrails (operator-locked 2026-06-13, `design-decisions-2026-06-13.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GovernorBoundary {
    /// A high-risk / destructive tool-call (write outside cwd, delete, external
    /// send, spend over a hard limit). Maps from any [`RiskLevel`] that
    /// `requires_approval()`. C-2 HARD GUARDRAIL: ALWAYS pauses — no config can
    /// auto-continue it.
    HighRiskAction,
    /// A low-risk / soft boundary (e.g. a soft token/cost warning). May
    /// auto-continue when the run opted in via `auto_continue_low_risk`;
    /// otherwise pauses (the C-1 default).
    SoftBudget,
    /// No-response: escalation timed out, the phone was offline, or the owner
    /// refused. C-1: ALWAYS fail-safe PAUSE + hold — never auto-continue, never
    /// kill — regardless of config.
    NoResponse,
}

impl GovernorBoundary {
    /// The governor escalation boundary an action at `risk` presents, or `None`
    /// when it is NOT a governor boundary at all. Anything that
    /// `requires_approval()` (ExecuteHigh / Write / Network) is a high-risk,
    /// force-pause boundary; `ReadOnly` / `ExecuteLow` auto-run in the drive
    /// loop (see `low_risk_tool_auto_allows*`) and so present NO boundary. We
    /// return `None` for them rather than `Some(SoftBudget)`, because a low-risk
    /// TOOL CALL is not a soft-budget boundary — mapping it to one would wrongly
    /// imply `governor_pauses_at(for_risk(low_risk))` pauses it by default.
    pub fn for_risk(risk: RiskLevel) -> Option<GovernorBoundary> {
        if risk.requires_approval() {
            Some(GovernorBoundary::HighRiskAction)
        } else {
            None
        }
    }
}

impl GovernPolicy {
    /// SYS-C: does the governor PAUSE (escalate + safe-hold) at `boundary`
    /// rather than auto-continue? The default posture is PAUSE at EVERY boundary
    /// (C-1 fail-safe). A run MAY opt into auto-continuing a `SoftBudget`
    /// boundary via `auto_continue_low_risk` (C-2). HARD GUARDRAILS that no
    /// config can lift: a `HighRiskAction` ALWAYS pauses, and a `NoResponse`
    /// timeout/offline/refusal ALWAYS fail-safe pauses — so ④ can never degrade
    /// into fire-and-forget (§12). This is the single source of truth the
    /// [PLANNED] auto-continue path must consult; the live high-risk force-pause
    /// is already enforced in the drive loop (P0-3 `governor_gate_tests`).
    pub fn governor_pauses_at(&self, boundary: GovernorBoundary) -> bool {
        match boundary {
            // C-2 hard guardrail: high-risk can never be auto-continued.
            GovernorBoundary::HighRiskAction => true,
            // C-1 fail-safe: no-response always pauses + holds.
            GovernorBoundary::NoResponse => true,
            // C-1 default PAUSE; C-2 per-run opt-in to auto-continue.
            GovernorBoundary::SoftBudget => !self.auto_continue_low_risk,
        }
    }

    /// SYS-C guardrail predicate: must an action at this `RiskLevel` force-pause
    /// for owner approval, regardless of any auto-continue opt-in? True for
    /// every risk that `requires_approval()`. The [PLANNED] auto-continue
    /// feature MUST gate on this so a high-risk / destructive action can never
    /// be configured to auto-run.
    pub fn force_pauses_high_risk(&self, risk: RiskLevel) -> bool {
        match GovernorBoundary::for_risk(risk) {
            // A high-risk boundary always pauses (governor_pauses_at == true).
            Some(boundary) => self.governor_pauses_at(boundary),
            // Low-risk is not a force-pause boundary — it auto-runs.
            None => false,
        }
    }
}

/// Why the budget HARD-BRAKE fired — recorded as the governance reason so the
/// flight recording shows exactly which limit stopped the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BudgetBrake {
    /// Cumulative `output_tokens` exceeded `GovernPolicy::max_output_tokens`.
    Tokens,
    /// Wall-clock elapsed exceeded `GovernPolicy::max_wall_secs`.
    WallClock,
    /// Device battery read BELOW `GovernPolicy::min_battery_pct`.
    Battery,
}
impl BudgetBrake {
    /// A stable, greppable approval-id prefix for the governance moment so an
    /// operator (or replay) can see the run was stopped by a budget brake.
    fn approval_id(self) -> &'static str {
        match self {
            BudgetBrake::Tokens => "budget-brake:max_output_tokens",
            BudgetBrake::WallClock => "budget-brake:max_wall_secs",
            BudgetBrake::Battery => "battery-brake:min_battery_pct",
        }
    }
}

/// Read the device battery as a whole percent (0..=100). This is the production
/// battery source injected into the drive loop; the wall-clock brake has an
/// injectable `now`, the battery brake has this.
///
/// TODO: wire a real cross-platform battery reader (e.g. a `sysinfo`/`battery`
/// crate or per-OS power query). Until then this returns 100 = "unknown / treat
/// as plugged-in", so the battery brake never false-fires when the policy opts in
/// on a machine without a sensor. Tests inject a fixed value instead of calling
/// this, so the brake is exercised WITHOUT real hardware.
fn read_hardware_battery_pct() -> u8 {
    100
}

/// The terminal state of a governed run.
#[derive(Debug, PartialEq, Eq)]
pub enum RunOutcome {
    Completed,
    Aborted,
    Denied,
    /// Apex ④ PHONE REDIRECT: the operator steered the run with a new instruction
    /// instead of approving the pending high-risk action. The run stops here (the
    /// pending tool is NOT run); the caller re-dispatches with the carried
    /// instruction. Distinct from `Denied`/`Aborted` so the caller can branch on
    /// "redirect -> re-dispatch" rather than "stop".
    Redirected(String),
    /// Apex (4) REDIRECT RE-DISPATCH SAFETY: the operator kept steering the run
    /// (`Redirected` -> re-dispatch) until the redirect-depth cap was reached. The
    /// run ends HERE with a CLEAR terminal outcome (never a silent drop) carrying
    /// the last unconsumed instruction so the caller can surface it. Bounds the
    /// re-dispatch loop against an infinite "steer -> steer -> steer ..." cycle.
    RedirectCapExhausted(String),
}

/// Default cap on how many times a governed run may be RE-DISPATCHED on an
/// operator `Redirected` steer before the chain ends in
/// [`RunOutcome::RedirectCapExhausted`]. Bounds the re-dispatch loop so a
/// pathological "steer -> steer -> steer ..." cycle cannot run forever. The
/// INITIAL pass does not count against the cap; only re-dispatches do.
pub const DEFAULT_REDIRECT_CAP: u32 = 3;

/// Apex (4) REDIRECT RE-DISPATCH driver (testable, I/O-free). Runs the governed
/// pass via `run_pass(prompt)`, and while the pass ends in
/// [`RunOutcome::Redirected(instruction)`] RE-ENTERS `run_pass` with the carried
/// `instruction` as the new prompt -- preserving everything else the closure
/// captures (cwd / cli / policy). Bounded by `cap` RE-DISPATCHES: when the cap is
/// hit the final outcome is rewritten to [`RunOutcome::RedirectCapExhausted`] so
/// the run ends with a CLEAR outcome instead of silently dropping the steer.
///
/// `run_pass` is `FnMut(String) -> GovernedFold`: production wraps the real
/// `run_govern_folded` (one governed L0 pass); tests inject a closure backed by
/// `drive_fold_with_enforcement` + a `MockEscalator`.
pub fn drive_redirect_chain<F>(initial_prompt: String, cap: u32, mut run_pass: F) -> GovernedFold
where
    F: FnMut(String) -> GovernedFold,
{
    let mut fold = run_pass(initial_prompt);
    let mut redirects: u32 = 0;
    while let RunOutcome::Redirected(instruction) = &fold.outcome {
        if redirects >= cap {
            // Cap reached: end with a clear terminal outcome carrying the last
            // (unconsumed) steer -- NOT a silent drop. Move the instruction out of
            // the borrowed `fold.outcome` before overwriting it.
            let last = instruction.clone();
            fold.outcome = RunOutcome::RedirectCapExhausted(last);
            return fold;
        }
        let next_prompt = instruction.clone();
        redirects += 1;
        fold = run_pass(next_prompt);
    }
    fold
}

/// The assistant output captured WHILE governing a run — so a worker dispatch can
/// both enforce the governor AND return the CLI's answer. `error` is set if the
/// CLI emitted an `Error` event (the caller surfaces it as a failure).
#[derive(Debug)]
pub struct GovernedFold {
    pub outcome: RunOutcome,
    pub text: String,
    pub usage: serde_json::Value,
    pub error: Option<(String, String)>,
}

/// Drive an L0 event stream under governance, discarding the assistant text.
/// Pure w.r.t. I/O — the recorder + escalator are injected (production wires the
/// real ones; tests inject mocks). Thin wrapper over [`drive_fold`].
pub fn drive(
    cli: CliKind,
    events: Receiver<CliEvent>,
    recorder: &mut dyn RunRecorder,
    escalator: &mut dyn Escalator,
    policy: &GovernPolicy,
) -> RunOutcome {
    drive_fold(cli, events, recorder, escalator, policy).outcome
}

/// Drive an L0 event stream under governance AND fold the assistant text/usage
/// (and any error) out of the same stream. The governance behaviour is identical
/// to [`drive`]; this variant additionally returns what the CLI produced so the
/// worker path can answer the dispatched task while it records + escalates.
pub fn drive_fold(
    cli: CliKind,
    events: Receiver<CliEvent>,
    recorder: &mut dyn RunRecorder,
    escalator: &mut dyn Escalator,
    policy: &GovernPolicy,
) -> GovernedFold {
    // Production wall-clock = a real monotonic clock anchored at the loop start;
    // production battery = the real hardware reader.
    let start = Instant::now();
    drive_fold_with_clock(
        cli,
        events,
        recorder,
        escalator,
        policy,
        move || start.elapsed(),
        read_hardware_battery_pct,
    )
}

/// [`drive_fold`] with injectable elapsed-wall-clock + battery sources. `now`
/// returns the elapsed `Duration` since the run began; `battery_pct` returns the
/// current device battery percent (0..=100). Production passes a monotonic
/// `Instant` + the real hardware reader; tests pass deterministic sources so the
/// wall-clock and battery HARD-BRAKEs are exercised WITHOUT real sleeping or real
/// hardware.
pub(crate) fn drive_fold_with_clock(
    cli: CliKind,
    events: Receiver<CliEvent>,
    recorder: &mut dyn RunRecorder,
    escalator: &mut dyn Escalator,
    policy: &GovernPolicy,
    now: impl Fn() -> Duration,
    battery_pct: impl Fn() -> u8,
) -> GovernedFold {
    drive_fold_with_enforcement(
        enforcement_for(cli),
        events,
        recorder,
        escalator,
        policy,
        now,
        battery_pct,
    )
}

/// The governed drive loop, parameterized by the resolved [`Enforcement`]. Split out
/// from [`drive_fold_with_clock`] so a test can exercise a specific enforcement
/// (e.g. the reserved parent-side `PreActionBlocking` path) independently of the
/// CLI→enforcement mapping.
pub(crate) fn drive_fold_with_enforcement(
    enforcement: Enforcement,
    events: Receiver<CliEvent>,
    recorder: &mut dyn RunRecorder,
    escalator: &mut dyn Escalator,
    policy: &GovernPolicy,
    now: impl Fn() -> Duration,
    battery_pct: impl Fn() -> u8,
) -> GovernedFold {
    let mut outcome = RunOutcome::Completed;
    let mut text = String::new();
    let mut usage = serde_json::Value::Null;
    let mut error: Option<(String, String)> = None;
    // Apex ④ HARD-BRAKE accumulators (only consulted when the policy sets a limit).
    let mut output_tokens_total: u64 = 0;
    for ev in events.iter() {
        recorder.record(RunRecord::Event(ev.clone()));
        match &ev.event {
            EventKind::AssistantText { delta } => text.push_str(delta),
            EventKind::Usage { input_tokens, output_tokens, cost_usd } => {
                output_tokens_total = output_tokens_total.saturating_add(*output_tokens);
                usage = serde_json::json!({
                    "input_tokens": input_tokens,
                    "output_tokens": output_tokens,
                    "cost_usd": cost_usd,
                });
            }
            EventKind::Error { error_kind, detail } => {
                error = Some((error_kind.clone(), detail.clone()));
            }
            _ => {}
        }
        // HARD-BRAKE: check budgets + battery every event. A configured limit that
        // is exceeded aborts the whole run (same shape as a post-action STOP). When
        // no limit is configured (all `None`) this is inert and behavior is
        // unchanged.
        if let Some(brake) = budget_exceeded(policy, output_tokens_total, &now, &battery_pct) {
            record_budget_brake(recorder, enforcement, brake);
            outcome = RunOutcome::Aborted;
            break;
        }
        if let Some((name, _args, risk)) = classify_event(&ev) {
            if !risk.requires_approval() {
                continue; // low-risk: recorded, auto-allowed
            }
            // claude's pre-action gate runs CHILD-SIDE (the PreToolUse hook is the
            // sole awaiter and writes the authoritative ApprovalRequested/Approved/
            // Denied to the shared EventStore under this run's task_id). The parent
            // only OBSERVES: the raw tool_use is already in the signed transcript
            // (recorded above), so skip the parent-side governance round-trip — no
            // second await, no competing governance record (the agy-#3 fix).
            if enforcement == Enforcement::PreActionDelegated {
                continue;
            }
            let contract = ExecutionContract::new(
                "local",
                "cli-session",
                "tool.call",
                &name,
                ".",
                vec![],
                risk,
                "governed AI-CLI ToolCall",
                300,
            );
            let approval_id = contract.id.clone();
            recorder.record(RunRecord::Governance {
                approval_id: approval_id.clone(),
                risk,
                state: ContractState::Pending,
                enforcement: enf_str(enforcement),
            });
            match enforcement {
                Enforcement::PreActionBlocking => {
                    let decision = escalator.await_decision(&approval_id, &name, risk);
                    // Apex ④ PHONE REDIRECT: the operator steered the run instead of
                    // approving the pending action. Record the governance moment (the
                    // pending tool is DENIED — `apply` maps a Redirect to `Denied`)
                    // and stop here with the new instruction; the caller re-dispatches.
                    if let ApprovalDecision::Redirect(instruction) = decision {
                        recorder.record(RunRecord::Governance {
                            approval_id,
                            risk,
                            state: apply(ContractState::Pending, ApprovalDecision::Redirect(
                                instruction.clone(),
                            )),
                            enforcement: enf_str(enforcement),
                        });
                        outcome = RunOutcome::Redirected(instruction);
                        break;
                    }
                    let state = apply(ContractState::Pending, decision);
                    recorder.record(RunRecord::Governance {
                        approval_id,
                        risk,
                        state,
                        enforcement: enf_str(enforcement),
                    });
                    if gate(state) == GateOutcome::Deny {
                        outcome = RunOutcome::Denied;
                        if policy.deny_aborts_run {
                            break; // strict mode: a denial ends the whole run
                        }
                    }
                    // (allow/deny is communicated back to claude via the MCP permission tool — Task 6)
                }
                Enforcement::PostActionObserved => {
                    let stop = escalator.alert_observed(&approval_id, &name, risk);
                    if stop {
                        outcome = RunOutcome::Aborted;
                        break;
                    }
                }
                // claude's delegated path is handled by the early `continue` above.
                Enforcement::PreActionDelegated => unreachable!("delegated short-circuits earlier"),
            }
        }
    }
    GovernedFold { outcome, text, usage, error }
}

/// Has a configured HARD-BRAKE limit been exceeded? Token budget is checked
/// against the cumulative output tokens; wall-clock against the elapsed `now()`;
/// battery against the device `battery_pct()`. Returns `None` when no limit is set
/// or none is exceeded (the default, so behavior is unchanged). Tokens are checked
/// first (cheap, no clock/sensor read), then wall-clock, then battery — each gated
/// behind its `Option` so an unset limit performs no read.
fn budget_exceeded(
    policy: &GovernPolicy,
    output_tokens_total: u64,
    now: &impl Fn() -> Duration,
    battery_pct: &impl Fn() -> u8,
) -> Option<BudgetBrake> {
    if let Some(max) = policy.max_output_tokens {
        if output_tokens_total > max {
            return Some(BudgetBrake::Tokens);
        }
    }
    if let Some(max) = policy.max_wall_secs {
        if now().as_secs() > max {
            return Some(BudgetBrake::WallClock);
        }
    }
    if let Some(min) = policy.min_battery_pct {
        if battery_pct() < min {
            return Some(BudgetBrake::Battery);
        }
    }
    None
}

/// Record the budget HARD-BRAKE as a governance moment (reuses the RunRecord::
/// Governance machinery, same as a denied/observed action) so the flight
/// recording shows the run was stopped by the budget governor and which limit.
fn record_budget_brake(
    recorder: &mut dyn RunRecorder,
    enforcement: Enforcement,
    brake: BudgetBrake,
) {
    recorder.record(RunRecord::Governance {
        approval_id: brake.approval_id().to_string(),
        // A budget overrun is a high-impact event — surface it at ExecuteHigh so
        // replay/operators see it alongside the other high-risk governance moments.
        risk: crate::execution_contract::RiskLevel::ExecuteHigh,
        // Cancelled = the governor deliberately stopped the run (distinct from a
        // per-tool Denied); recorder maps it onto the EventStore's Denied kind.
        state: ContractState::Cancelled,
        enforcement: enf_str(enforcement),
    });
}

fn enf_str(e: Enforcement) -> &'static str {
    match e {
        Enforcement::PreActionBlocking => "pre_action_blocking",
        Enforcement::PreActionDelegated => "pre_action_delegated",
        Enforcement::PostActionObserved => "post_action_observed",
    }
}

#[cfg(test)]
mod budget_tests {
    //! HARD-BRAKE (apex ④ budget/time) tests — hermetic: synthetic event streams,
    //! a MemRecorder + MockEscalator, and a deterministic wall-clock source. No real
    //! CLI, no real phone, NO real sleeping.
    use super::*;
    use crate::cli_session::event::{CliEvent, Fidelity, Source};
    use crate::cli_session::CliKind;
    use escalation::MockEscalator;
    use recorder::MemRecorder;
    use serde_json::json;
    use std::sync::mpsc::{Receiver, channel};

    fn stream(events: Vec<CliEvent>) -> Receiver<CliEvent> {
        let (tx, rx) = channel();
        for e in events {
            tx.send(e).unwrap();
        }
        drop(tx);
        rx
    }
    fn ev(k: EventKind) -> CliEvent {
        CliEvent::new(k, Fidelity::StructuredVerified, Source::LiveStream)
    }
    fn was_budget_braked(rec: &MemRecorder) -> bool {
        rec.records.iter().any(|r| {
            matches!(r,
                RunRecord::Governance { approval_id, state: ContractState::Cancelled, .. }
                if approval_id.starts_with("budget-brake:"))
        })
    }
    fn was_battery_braked(rec: &MemRecorder) -> bool {
        rec.records.iter().any(|r| {
            matches!(r,
                RunRecord::Governance { approval_id, state: ContractState::Cancelled, .. }
                if approval_id.starts_with("battery-brake:"))
        })
    }
    /// A battery reader that always reports full charge — the default for tests
    /// that aren't exercising the battery brake, so it never false-fires.
    fn full_battery() -> u8 {
        100
    }

    #[test]
    fn token_budget_exceeded_aborts_with_brake_recorded() {
        // Usage reports 500 output tokens; the policy budget is 100 → abort.
        let events = stream(vec![
            ev(EventKind::AssistantText { delta: "working".into() }),
            ev(EventKind::Usage { input_tokens: 10, output_tokens: 500, cost_usd: 0.0 }),
            // This later tool call must NOT be governed — the run already stopped.
            ev(EventKind::ToolCall { name: "Bash".into(), args: json!({"cmd":"rm -rf /"}) }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        let policy = GovernPolicy { max_output_tokens: Some(100), ..Default::default() };
        let fold = drive_fold(CliKind::Codex, events, &mut rec, &mut esc, &policy);

        assert_eq!(fold.outcome, RunOutcome::Aborted, "token budget overrun must abort");
        assert!(was_budget_braked(&rec), "a budget-brake governance moment is recorded");
        assert!(
            rec.records.iter().any(|r| matches!(r,
                RunRecord::Governance { approval_id, .. }
                if approval_id == "budget-brake:max_output_tokens")),
            "the reason names the token budget"
        );
        // The post-budget Bash ToolCall was never escalated (loop stopped first).
        assert!(esc.sent.is_empty(), "no governance after the brake, got {:?}", esc.sent);
    }

    #[test]
    fn token_budget_at_limit_does_not_abort() {
        // Exactly at the budget (100 <= 100) is NOT an overrun — completes normally.
        let events = stream(vec![
            ev(EventKind::Usage { input_tokens: 1, output_tokens: 100, cost_usd: 0.0 }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        let policy = GovernPolicy { max_output_tokens: Some(100), ..Default::default() };
        let fold = drive_fold(CliKind::Codex, events, &mut rec, &mut esc, &policy);
        assert_eq!(fold.outcome, RunOutcome::Completed);
        assert!(!was_budget_braked(&rec));
    }

    #[test]
    fn token_budget_accumulates_across_usage_events() {
        // Two Usage events (60 + 60 = 120) cross a budget of 100 on the SECOND one.
        let events = stream(vec![
            ev(EventKind::Usage { input_tokens: 1, output_tokens: 60, cost_usd: 0.0 }),
            ev(EventKind::Usage { input_tokens: 1, output_tokens: 60, cost_usd: 0.0 }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        let policy = GovernPolicy { max_output_tokens: Some(100), ..Default::default() };
        let fold = drive_fold(CliKind::Codex, events, &mut rec, &mut esc, &policy);
        assert_eq!(fold.outcome, RunOutcome::Aborted, "cumulative tokens trip the brake");
        assert!(was_budget_braked(&rec));
    }

    #[test]
    fn wall_clock_budget_exceeded_aborts_with_brake_recorded() {
        // A deterministic clock that jumps PAST the 30s budget on the 2nd read.
        // No real sleep — the test owns the clock.
        let elapsed = std::cell::Cell::new(Duration::ZERO);
        let clock = || {
            let cur = elapsed.get();
            elapsed.set(cur + Duration::from_secs(20)); // 0s, 20s, 40s, ...
            cur
        };
        let events = stream(vec![
            ev(EventKind::AssistantText { delta: "tick".into() }), // clock=0s  (ok)
            ev(EventKind::AssistantText { delta: "tick".into() }), // clock=20s (ok)
            ev(EventKind::AssistantText { delta: "tick".into() }), // clock=40s (>30s → abort)
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        let policy = GovernPolicy { max_wall_secs: Some(30), ..Default::default() };
        let fold = drive_fold_with_clock(
            CliKind::Codex, events, &mut rec, &mut esc, &policy, clock, full_battery,
        );

        assert_eq!(fold.outcome, RunOutcome::Aborted, "wall-clock overrun must abort");
        assert!(
            rec.records.iter().any(|r| matches!(r,
                RunRecord::Governance { approval_id, .. }
                if approval_id == "budget-brake:max_wall_secs")),
            "the reason names the wall-clock budget"
        );
        // The 3rd event's delta is folded (record+fold happen BEFORE the budget
        // check), then the brake fires; the trailing TurnDone never runs.
        assert_eq!(fold.text, "tickticktick", "3 ticks folded, then braked");
    }

    #[test]
    fn no_budget_set_completes_unchanged() {
        // Control: default policy (both budgets None) — a big Usage + a long elapsed
        // clock change NOTHING; the run completes exactly as before.
        let elapsed = std::cell::Cell::new(Duration::from_secs(99_999));
        let clock = || elapsed.replace(elapsed.get() + Duration::from_secs(1));
        let events = stream(vec![
            ev(EventKind::Usage { input_tokens: 9, output_tokens: 1_000_000, cost_usd: 9.0 }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        let policy = GovernPolicy::default(); // all None
        let fold = drive_fold_with_clock(
            CliKind::Codex, events, &mut rec, &mut esc, &policy, clock, full_battery,
        );
        assert_eq!(fold.outcome, RunOutcome::Completed, "no budget = behavior unchanged");
        assert!(!was_budget_braked(&rec), "no budget-brake recorded when unset");
        assert_eq!(fold.usage["output_tokens"], 1_000_000); // usage still folded
    }

    #[test]
    fn low_battery_aborts_with_battery_brake_recorded() {
        // Battery reads 5%; the policy floor is 20% → abort on the first event.
        let events = stream(vec![
            ev(EventKind::AssistantText { delta: "working".into() }),
            // This later tool call must NOT be governed — the run already stopped.
            ev(EventKind::ToolCall { name: "Bash".into(), args: json!({"cmd":"rm -rf /"}) }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        let policy = GovernPolicy { min_battery_pct: Some(20), ..Default::default() };
        let clock = || Duration::ZERO;
        let low_battery = || 5u8;
        let fold = drive_fold_with_clock(
            CliKind::Codex, events, &mut rec, &mut esc, &policy, clock, low_battery,
        );

        assert_eq!(fold.outcome, RunOutcome::Aborted, "low battery must abort");
        assert!(was_battery_braked(&rec), "a battery-brake governance moment is recorded");
        assert!(
            rec.records.iter().any(|r| matches!(r,
                RunRecord::Governance { approval_id, .. }
                if approval_id == "battery-brake:min_battery_pct")),
            "the reason names the battery floor"
        );
        // The post-brake Bash ToolCall was never escalated (loop stopped first).
        assert!(esc.sent.is_empty(), "no governance after the brake, got {:?}", esc.sent);
    }

    #[test]
    fn battery_at_or_above_floor_does_not_abort() {
        // Battery reads exactly the floor (20 >= 20) — NOT below, so it completes.
        let events = stream(vec![
            ev(EventKind::AssistantText { delta: "ok".into() }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        let policy = GovernPolicy { min_battery_pct: Some(20), ..Default::default() };
        let clock = || Duration::ZERO;
        let at_floor = || 20u8;
        let fold = drive_fold_with_clock(
            CliKind::Codex, events, &mut rec, &mut esc, &policy, clock, at_floor,
        );
        assert_eq!(fold.outcome, RunOutcome::Completed, "at/above floor = no abort");
        assert!(!was_battery_braked(&rec), "no battery-brake recorded at/above floor");
    }

    #[test]
    fn phone_redirect_yields_redirected_outcome_and_records_governance() {
        // Apex ④ PHONE REDIRECT on the reserved parent-side PreActionBlocking path:
        // the operator replies `redirect <new goal>`; the run stops with
        // RunOutcome::Redirected(<new goal>) and a governance moment records the
        // (denied) pending action. (claude itself now gates CHILD-SIDE via the
        // PreToolUse hook — see governed_run::permission's hook_redirect test — so
        // this exercises the PreActionBlocking arm directly, not via CliKind::Claude.)
        let events = stream(vec![
            ev(EventKind::AssistantText { delta: "thinking".into() }),
            ev(EventKind::ToolCall { name: "Bash".into(), args: json!({"cmd":"rm -rf /"}) }),
            // Anything after the redirect must NOT run — the loop broke.
            ev(EventKind::ToolCall { name: "Bash".into(), args: json!({"cmd":"echo late"}) }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        esc.force_decision = Some(ApprovalDecision::Redirect("new goal".into()));
        let policy = GovernPolicy::default();
        let fold = drive_fold_with_enforcement(
            Enforcement::PreActionBlocking,
            events,
            &mut rec,
            &mut esc,
            &policy,
            || Duration::from_secs(0),
            || 100,
        );

        assert_eq!(
            fold.outcome,
            RunOutcome::Redirected("new goal".to_string()),
            "a phone redirect stops the run with the new instruction"
        );
        // Exactly one high-risk action was escalated (the loop broke on the first).
        assert_eq!(
            esc.sent.iter().filter(|s| s.starts_with("await:")).count(),
            1,
            "only the first high-risk action was escalated, got {:?}",
            esc.sent
        );
        // A governance moment for the pending action records it as Denied (a redirect
        // does NOT approve the pending tool).
        assert!(
            rec.records.iter().any(|r| matches!(r,
                RunRecord::Governance { state: ContractState::Denied, .. })),
            "the redirected (denied) pending action is recorded as a governance moment"
        );
        // The folded text up to the redirect is preserved.
        assert_eq!(fold.text, "thinking");
    }

    #[test]
    fn claude_delegated_observes_without_parent_await() {
        // agy-#3 fix: claude gates CHILD-SIDE (the PreToolUse hook is the sole
        // awaiter). The PARENT must NOT await again — it only observes the stream.
        // A high-risk ToolCall in claude's stream => NO parent escalation and NO
        // parent governance record; the run completes (the hook enforced it
        // child-side). The raw tool_use is still captured for the flight-recorder.
        let events = stream(vec![
            ev(EventKind::AssistantText { delta: "ok".into() }),
            ev(EventKind::ToolCall { name: "Bash".into(), args: json!({"cmd":"ls"}) }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        // Would force a deny IF the parent (wrongly) awaited — proving it does not.
        esc.force_decision = Some(ApprovalDecision::Deny);
        let policy = GovernPolicy::default();
        let fold = drive_fold(CliKind::Claude, events, &mut rec, &mut esc, &policy);

        assert_eq!(
            fold.outcome,
            RunOutcome::Completed,
            "parent observes only; it never gates the tool itself"
        );
        assert!(
            esc.sent.is_empty(),
            "parent must NOT await/alert for claude (hook gates child-side), got {:?}",
            esc.sent
        );
        assert!(
            !rec.records.iter().any(|r| matches!(r, RunRecord::Governance { .. })),
            "parent records NO governance for a delegated claude tool (the hook is authoritative)"
        );
        assert!(
            rec.records.iter().any(|r| matches!(r,
                RunRecord::Event(e) if matches!(&e.event, EventKind::ToolCall { .. }))),
            "the raw tool_use is still recorded for the signed flight transcript"
        );
    }

    #[test]
    fn no_battery_floor_set_completes_unchanged() {
        // Control: default policy (min_battery_pct None) — even a flat 0% battery
        // changes NOTHING; the run completes exactly as before.
        let events = stream(vec![
            ev(EventKind::AssistantText { delta: "ok".into() }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        let policy = GovernPolicy::default(); // min_battery_pct None
        let clock = || Duration::ZERO;
        let dead_battery = || 0u8;
        let fold = drive_fold_with_clock(
            CliKind::Codex, events, &mut rec, &mut esc, &policy, clock, dead_battery,
        );
        assert_eq!(fold.outcome, RunOutcome::Completed, "no battery floor = unchanged");
        assert!(!was_battery_braked(&rec), "no battery-brake recorded when unset");
    }

    #[test]
    fn cli_flag_derived_policy_aborts_on_wall_clock_through_drive_loop() {
        // SEAM: a GovernPolicy built FROM the `phantom govern --max-wall-secs` CLI
        // flag (via GovernConfig::apply_flags) — NOT a struct literal — fed through
        // the drive loop, must ABORT on wall-clock overrun. The flag->policy path
        // (govern_flag_tests in run.rs) and the policy->abort path (the other
        // budget_tests here) were only proven SEPARATELY; this exercises the whole
        // flags->policy->drive_fold->Abort wire so it FAILS if that seam breaks.
        use crate::governed_run::run::GovernConfig;

        let mut cfg = GovernConfig::new(CliKind::Codex, "noop");
        cfg.apply_flags(["--max-wall-secs", "30"])
            .expect("the CLI brake flag must parse");
        // Guard: the flag actually populated the brake we are about to exercise
        // (so a regression in apply_flags fails here, not silently).
        assert_eq!(cfg.policy.max_wall_secs, Some(30), "flag must populate the wall-clock brake");

        // Deterministic clock: 0s, 20s, 40s — trips the 30s budget on the 3rd read.
        // No real sleep; the test owns the clock.
        let elapsed = std::cell::Cell::new(Duration::ZERO);
        let clock = || {
            let cur = elapsed.get();
            elapsed.set(cur + Duration::from_secs(20));
            cur
        };
        let events = stream(vec![
            ev(EventKind::AssistantText { delta: "tick".into() }),
            ev(EventKind::AssistantText { delta: "tick".into() }),
            ev(EventKind::AssistantText { delta: "tick".into() }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        // Drive the FLAG-DERIVED policy (cfg.policy) — not a hand-built literal.
        let fold = drive_fold_with_clock(
            CliKind::Codex, events, &mut rec, &mut esc, &cfg.policy, clock, full_battery,
        );

        assert_eq!(
            fold.outcome, RunOutcome::Aborted,
            "a wall-clock budget set via the CLI flag must abort through the drive loop"
        );
        assert!(
            rec.records.iter().any(|r| matches!(r,
                RunRecord::Governance { approval_id, .. }
                if approval_id == "budget-brake:max_wall_secs")),
            "the recorded brake reason names the wall-clock budget"
        );
    }

    #[test]
    fn max_wallclock_flag_aborts_cleanly_with_wallclock_event_recorded() {
        // GATE (apex-④ "your hard brakes"): a governed run started with the CLI
        // `--max-wallclock 1s` flag drives a SLOW task whose wall clock crosses the
        // 1s deadline; the run must end CLEANLY (RunOutcome::Aborted — never a panic)
        // and a wallclock-exceeded flight-recorder event must be recorded. Exercises
        // the whole `--max-wallclock` flag -> policy -> drive_fold -> Abort + record
        // wire. Hermetic: the test owns the clock (NO real sleeping), so the ~2s
        // budget is never spent in real time.
        use crate::governed_run::run::GovernConfig;

        let mut cfg = GovernConfig::new(CliKind::Codex, "slow task");
        cfg.apply_flags(["--max-wallclock", "1s"])
            .expect("the --max-wallclock brake flag must parse");
        assert_eq!(
            cfg.policy.max_wall_secs,
            Some(1),
            "--max-wallclock 1s must populate the 1s wall-clock brake"
        );

        // A "slow" task: a long stream of work whose wall clock advances past the 1s
        // deadline on the 2nd read. The test owns the clock — no real sleep.
        let elapsed = std::cell::Cell::new(Duration::ZERO);
        let clock = || {
            let cur = elapsed.get();
            elapsed.set(cur + Duration::from_millis(800)); // 0ms, 800ms, 1600ms (>1s)
            cur
        };
        let events = stream(vec![
            ev(EventKind::AssistantText { delta: "step1".into() }), // clock=0ms     (ok)
            ev(EventKind::AssistantText { delta: "step2".into() }), // clock=800ms   (ok)
            ev(EventKind::AssistantText { delta: "step3".into() }), // clock=1600ms  (>1s → abort)
            // A high-risk tool AFTER the deadline must NOT run — the brake fired first.
            ev(EventKind::ToolCall { name: "Bash".into(), args: json!({"cmd":"rm -rf /"}) }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        let fold = drive_fold_with_clock(
            CliKind::Codex, events, &mut rec, &mut esc, &cfg.policy, clock, full_battery,
        );

        // CLEAN shutdown (Aborted / cap-reached), NOT a 101 panic.
        assert_eq!(
            fold.outcome, RunOutcome::Aborted,
            "a run past its --max-wallclock deadline must abort cleanly"
        );
        // A wallclock-exceeded flight-recorder event was recorded to the EventStore
        // (the budget-brake governance moment names the wall-clock budget).
        assert!(
            was_budget_braked(&rec),
            "a budget-brake governance moment must be recorded"
        );
        assert!(
            rec.records.iter().any(|r| matches!(r,
                RunRecord::Governance { approval_id, .. }
                if approval_id == "budget-brake:max_wall_secs")),
            "the recorded event names the wall-clock (wallclock-exceeded) brake"
        );
        // The post-deadline high-risk Bash tool was NEVER escalated (loop stopped).
        assert!(esc.sent.is_empty(), "no governance after the brake, got {:?}", esc.sent);
    }
}

#[cfg(test)]
mod governor_gate_tests {
    //! Apex ④ invariant ① — DENY-UNTIL-APPROVED at the GOVERNOR drive-loop level
    //! (the reserved parent-side `PreActionBlocking` gate primitive). A HIGH-RISK
    //! ToolCall BLOCKS on an operator decision (escalates); only an APPROVE lets the
    //! run continue, a DENY denies/aborts it, and a LOW-RISK tool auto-allows WITHOUT
    //! escalation. Plus invariant ④'s safety default (`timeout_fallback == Deny`).
    //! Hermetic: synthetic event stream + MemRecorder + MockEscalator, no real
    //! CLI / phone / clock. These LOCK behaviour already enforced by `drive_fold`.
    use super::*;
    use crate::cli_session::event::{CliEvent, Fidelity, Source};
    use escalation::MockEscalator;
    use recorder::MemRecorder;
    use serde_json::json;
    use std::sync::mpsc::{Receiver, channel};

    fn stream(events: Vec<CliEvent>) -> Receiver<CliEvent> {
        let (tx, rx) = channel();
        for e in events {
            tx.send(e).unwrap();
        }
        drop(tx);
        rx
    }
    fn ev(k: EventKind) -> CliEvent {
        CliEvent::new(k, Fidelity::StructuredVerified, Source::LiveStream)
    }
    /// Drive a stream through the parent-side blocking gate with no budgets, a
    /// frozen clock, and a full battery — so ONLY the deny-until-approved path is
    /// exercised (never a brake).
    fn drive_blocking(
        events: Receiver<CliEvent>,
        rec: &mut MemRecorder,
        esc: &mut MockEscalator,
        policy: &GovernPolicy,
    ) -> GovernedFold {
        drive_fold_with_enforcement(
            Enforcement::PreActionBlocking,
            events,
            rec,
            esc,
            policy,
            || Duration::ZERO,
            || 100,
        )
    }
    fn awaits(esc: &MockEscalator) -> usize {
        esc.sent.iter().filter(|s| s.starts_with("await:")).count()
    }

    #[test]
    fn high_risk_denied_blocks_and_denies_the_run() {
        // A high-risk Bash tool is escalated (it BLOCKS on the decision); a Deny
        // yields RunOutcome::Denied and a Denied governance moment. With the default
        // policy (deny_aborts_run = false) the run is denied but not torn down.
        let events = stream(vec![
            ev(EventKind::ToolCall { name: "Bash".into(), args: json!({"command":"rm -rf /"}) }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        esc.force_decision = Some(ApprovalDecision::Deny);
        let fold = drive_blocking(events, &mut rec, &mut esc, &GovernPolicy::default());

        assert_eq!(fold.outcome, RunOutcome::Denied, "a denied high-risk action denies the run");
        assert_eq!(awaits(&esc), 1, "the high-risk tool BLOCKED on a decision before running");
        assert!(
            rec.records.iter().any(|r| matches!(r,
                RunRecord::Governance { state: ContractState::Denied, .. })),
            "the denied action is recorded as a governance moment"
        );
    }

    #[test]
    fn high_risk_denied_aborts_run_in_strict_mode() {
        // deny_aborts_run = true: a denial ABORTS the whole run — a LATER high-risk
        // tool is never reached (only the first is escalated).
        let events = stream(vec![
            ev(EventKind::ToolCall { name: "Bash".into(), args: json!({"command":"first"}) }),
            ev(EventKind::ToolCall { name: "Bash".into(), args: json!({"command":"second"}) }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        esc.force_decision = Some(ApprovalDecision::Deny);
        let policy = GovernPolicy { deny_aborts_run: true, ..Default::default() };
        let fold = drive_blocking(events, &mut rec, &mut esc, &policy);

        assert_eq!(fold.outcome, RunOutcome::Denied, "strict-mode denial denies the run");
        assert_eq!(
            awaits(&esc),
            1,
            "strict mode stops after the FIRST denial — the second tool is never escalated, got {:?}",
            esc.sent
        );
    }

    #[test]
    fn high_risk_approved_blocks_first_then_run_continues() {
        // An ApproveOnce on the blocking path is recorded Approved and the run
        // completes — but the tool still BLOCKED on the decision first (escalated once).
        let events = stream(vec![
            ev(EventKind::ToolCall { name: "Bash".into(), args: json!({"command":"ls"}) }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        esc.force_decision = Some(ApprovalDecision::ApproveOnce);
        let fold = drive_blocking(events, &mut rec, &mut esc, &GovernPolicy::default());

        assert_eq!(fold.outcome, RunOutcome::Completed, "an approved high-risk action lets the run continue");
        assert_eq!(awaits(&esc), 1, "the approved tool still BLOCKED on the decision first");
        assert!(
            rec.records.iter().any(|r| matches!(r,
                RunRecord::Governance { state: ContractState::Approved, .. })),
            "the approved action is recorded as a governance moment"
        );
    }

    #[test]
    fn low_risk_tool_auto_allows_without_escalation() {
        // A ReadOnly tool (Read) must NOT escalate and must NOT record a governance
        // moment — only ReadOnly/ExecuteLow auto-allow; the run completes.
        let events = stream(vec![
            ev(EventKind::ToolCall { name: "Read".into(), args: json!({"path":"src/main.rs"}) }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        // Would force a DENY if a low-risk tool (wrongly) escalated — proving it does not.
        esc.force_decision = Some(ApprovalDecision::Deny);
        let fold = drive_blocking(events, &mut rec, &mut esc, &GovernPolicy::default());

        assert_eq!(fold.outcome, RunOutcome::Completed, "low-risk auto-allows; no gate");
        assert!(esc.sent.is_empty(), "a low-risk tool must NOT escalate, got {:?}", esc.sent);
        assert!(
            !rec.records.iter().any(|r| matches!(r, RunRecord::Governance { .. })),
            "no governance moment for an auto-allowed low-risk tool"
        );
    }

    #[test]
    fn govern_policy_default_timeout_fallback_is_deny() {
        // Invariant ④ safety default: an unconfigured policy's timeout fallback is
        // Deny (fail-safe), so a pending approval that times out never auto-allows;
        // and no brake/strict default weakens the safe baseline.
        let p = GovernPolicy::default();
        assert_eq!(p.timeout_fallback, ApprovalDecision::Deny, "default timeout fallback must be Deny");
        assert!(!p.deny_aborts_run, "per-tool gate is the default (no surprise run teardown)");
        assert_eq!(p.max_wall_secs, None, "no wall-clock brake unless opted in");
        assert_eq!(p.max_output_tokens, None, "no token brake unless opted in");
        assert_eq!(p.min_battery_pct, None, "no battery brake unless opted in");
    }

    /// apex-④ timeout -> fallback-deny: when the operator NEVER replies, the blocking gate's escalator returns its fail-safe default (Deny) and the run is Denied — proving a pending high-risk action is never silently auto-allowed on no response.
    #[test]
    fn high_risk_no_reply_times_out_to_fail_safe_deny() {
        let events = stream(vec![
            ev(EventKind::ToolCall { name: "Bash".into(), args: json!({"command":"rm -rf /important"}) }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        let fold = drive_blocking(events, &mut rec, &mut esc, &GovernPolicy::default());

        assert_eq!(
            fold.outcome,
            RunOutcome::Denied,
            "no operator reply must fail-safe to a denied run"
        );
        assert_eq!(
            awaits(&esc),
            1,
            "the high-risk tool BLOCKED awaiting a decision before running"
        );
        assert!(
            rec.records.iter().any(|r| matches!(r,
                RunRecord::Governance { state: ContractState::Denied, .. })),
            "the timed-out action is recorded as a Denied governance moment"
        );
    }

    /// apex-④ phone REDIRECT: the operator steers the run instead of approving the pending high-risk action; the run stops with RunOutcome::Redirected(<instruction>) and records a governance moment (Redirect maps to Denied for the pending tool).
    #[test]
    fn high_risk_redirect_steers_and_stops_the_run() {
        let events = stream(vec![
            ev(EventKind::ToolCall { name: "Bash".into(), args: json!({"command":"deploy prod"}) }),
            ev(EventKind::ToolCall { name: "Bash".into(), args: json!({"command":"second"}) }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        esc.force_decision = Some(ApprovalDecision::Redirect("deploy to staging first".into()));
        let fold = drive_blocking(events, &mut rec, &mut esc, &GovernPolicy::default());

        assert_eq!(
            fold.outcome,
            RunOutcome::Redirected("deploy to staging first".into()),
            "a redirect steers the run and stops it with the new instruction"
        );
        assert_eq!(
            awaits(&esc),
            1,
            "redirect stops after the FIRST high-risk tool — the second is never escalated"
        );
        assert!(
            rec.records.iter().any(|r| matches!(r, RunRecord::Governance { .. })),
            "the redirected action is recorded as a governance moment"
        );
    }
}

#[cfg(test)]
mod sys_c_governor_pause_tests {
    //! SYS-C (operator-locked 2026-06-13) — the GOVERNOR's escalation posture is
    //! a configurable PAUSE: default = fail-safe pause at every boundary (C-1); a
    //! run may opt into auto-continuing a LOW-RISK (`SoftBudget`) boundary (C-2);
    //! but a HIGH-RISK / destructive boundary and a NO-RESPONSE timeout ALWAYS
    //! force-pause regardless of config — ④ can never be configured into the §12
    //! fire-and-forget AutoGPT form. These LOCK the policy-layer guardrails that
    //! the [PLANNED] auto-continue feature must obey; the live high-risk
    //! force-pause is already locked by `governor_gate_tests` (P0-3), which these
    //! build on rather than duplicate.
    use super::*;

    /// C-1: the default governor posture is PAUSE at EVERY boundary (high-risk,
    /// soft-budget, and no-response). Auto-continue is OFF by default.
    #[test]
    fn default_governor_posture_is_pause_at_every_boundary() {
        let p = GovernPolicy::default();
        assert!(!p.auto_continue_low_risk, "auto-continue is opt-in, default OFF (C-2)");
        assert!(p.governor_pauses_at(GovernorBoundary::HighRiskAction), "C-1 default: high-risk pauses");
        assert!(p.governor_pauses_at(GovernorBoundary::SoftBudget), "C-1 default: soft-budget pauses");
        assert!(p.governor_pauses_at(GovernorBoundary::NoResponse), "C-1 default: no-response pauses");
    }

    /// C-2: the SOFT-BUDGET (low-risk) boundary is the ONLY one whose pause is
    /// configurable — opting in flips it to auto-continue.
    #[test]
    fn soft_budget_boundary_is_configurable_to_auto_continue() {
        let opted_in = GovernPolicy { auto_continue_low_risk: true, ..Default::default() };
        assert!(
            !opted_in.governor_pauses_at(GovernorBoundary::SoftBudget),
            "C-2: a run that opted in auto-continues a low-risk boundary"
        );
        // The default still pauses — proving the knob actually changed behaviour.
        assert!(GovernPolicy::default().governor_pauses_at(GovernorBoundary::SoftBudget));
    }

    /// C-2 HARD GUARDRAIL: a high-risk boundary ALWAYS force-pauses, for EVERY
    /// configuration of the auto-continue knob — it can never be auto-continued.
    #[test]
    fn high_risk_boundary_always_force_pauses_regardless_of_config() {
        for auto_continue_low_risk in [false, true] {
            let p = GovernPolicy { auto_continue_low_risk, ..Default::default() };
            assert!(
                p.governor_pauses_at(GovernorBoundary::HighRiskAction),
                "C-2: high-risk must force-pause even with auto_continue_low_risk={auto_continue_low_risk}"
            );
        }
    }

    /// C-1 fail-safe: a no-response (timeout / phone offline / refusal) boundary
    /// ALWAYS pauses + holds, for EVERY config — never auto-continue, never kill.
    #[test]
    fn no_response_always_fail_safe_pauses_regardless_of_config() {
        for auto_continue_low_risk in [false, true] {
            let p = GovernPolicy { auto_continue_low_risk, ..Default::default() };
            assert!(
                p.governor_pauses_at(GovernorBoundary::NoResponse),
                "C-1: no-response must fail-safe pause even with auto_continue_low_risk={auto_continue_low_risk}"
            );
        }
    }

    /// THE SYS-C KEYSTONE: every `RiskLevel` that `requires_approval()` maps to
    /// a force-pause boundary that NO configuration can auto-allow. Iterates the
    /// real risk model so the guardrail is grounded, not free-floating.
    #[test]
    fn high_risk_can_never_be_configured_to_auto_allow() {
        let high_risks = [RiskLevel::ExecuteHigh, RiskLevel::Write, RiskLevel::Network];
        let low_risks = [RiskLevel::ReadOnly, RiskLevel::ExecuteLow];
        // The most permissive policy a config can express.
        let permissive = GovernPolicy { auto_continue_low_risk: true, deny_aborts_run: false, ..Default::default() };
        for risk in high_risks {
            assert!(risk.requires_approval(), "test premise: {risk:?} is high-risk");
            assert_eq!(
                GovernorBoundary::for_risk(risk),
                Some(GovernorBoundary::HighRiskAction),
                "{risk:?} is a high-risk boundary"
            );
            assert!(
                permissive.force_pauses_high_risk(risk),
                "{risk:?} must force-pause even under the most permissive config"
            );
        }
        for risk in low_risks {
            assert!(!risk.requires_approval(), "test premise: {risk:?} is low-risk");
            // A low-risk TOOL CALL is NOT a governor boundary — it auto-runs and
            // must never be mapped to a pausing (SoftBudget) boundary.
            assert_eq!(
                GovernorBoundary::for_risk(risk),
                None,
                "{risk:?} is not a governor boundary — it auto-runs"
            );
            assert!(
                !permissive.force_pauses_high_risk(risk),
                "{risk:?} is not a high-risk force-pause boundary"
            );
        }
    }
}

#[cfg(test)]
mod redirect_redispatch_tests {
    //! Apex (4) REDIRECT RE-DISPATCH: when the operator STEERS a governed run
    //! ("do X instead"), the run ends `Redirected("do X")` and the chain driver
    //! must RE-DISPATCH a SECOND governed pass with prompt "do X" (preserving
    //! cwd/cli/policy via the closure), bounded by a redirect-depth cap so a
    //! "steer -> steer -> steer ..." cycle can't run forever. Hermetic: each pass
    //! is a real `drive_fold_with_enforcement` over a synthetic stream + a
    //! MemRecorder + MockEscalator; the cap-exhaustion path ends in a CLEAR
    //! outcome, never a silent drop.
    use super::*;
    use crate::cli_session::event::{CliEvent, Fidelity, Source};
    use crate::cli_session::CliKind;
    use escalation::MockEscalator;
    use recorder::MemRecorder;
    use serde_json::json;
    use std::sync::mpsc::{Receiver, channel};

    fn stream(events: Vec<CliEvent>) -> Receiver<CliEvent> {
        let (tx, rx) = channel();
        for e in events {
            tx.send(e).unwrap();
        }
        drop(tx);
        rx
    }
    fn ev(k: EventKind) -> CliEvent {
        CliEvent::new(k, Fidelity::StructuredVerified, Source::LiveStream)
    }

    /// One governed pass over a stream whose FIRST high-risk tool the
    /// MockEscalator answers with `decision`. Returns the GovernedFold; the
    /// recorder is local (we only assert on the fold/outcome here).
    fn pass_with(decision: ApprovalDecision) -> GovernedFold {
        let events = stream(vec![
            ev(EventKind::AssistantText { delta: "working".into() }),
            ev(EventKind::ToolCall { name: "Bash".into(), args: json!({"command":"deploy prod"}) }),
            ev(EventKind::TurnDone { stop_reason: "end".into() }),
        ]);
        let mut rec = MemRecorder::default();
        let mut esc = MockEscalator::default();
        esc.force_decision = Some(decision);
        drive_fold_with_enforcement(
            Enforcement::PreActionBlocking,
            events,
            &mut rec,
            &mut esc,
            &GovernPolicy::default(),
            || Duration::ZERO,
            || 100,
        )
    }

    #[test]
    fn redirect_redispatches_second_pass_with_carried_instruction_then_completes() {
        // GATE: the operator forces Redirect("do X") on the FIRST pass's high-risk
        // tool. The chain driver must RE-ENTER run_pass a SECOND time with prompt
        // "do X" (proving the steer is actually re-dispatched, not stringified +
        // dropped) and the final outcome is Completed.
        let prompts = std::cell::RefCell::new(Vec::<String>::new());
        let fold = drive_redirect_chain("original task".to_string(), DEFAULT_REDIRECT_CAP, |prompt| {
            prompts.borrow_mut().push(prompt.clone());
            // First pass (the original task) is steered; every later pass (the
            // re-dispatched "do X") is allowed through to completion.
            if prompt == "original task" {
                pass_with(ApprovalDecision::Redirect("do X".into()))
            } else {
                pass_with(ApprovalDecision::ApproveOnce)
            }
        });

        let seen = prompts.borrow();
        assert_eq!(
            seen.len(),
            2,
            "exactly two governed passes ran: the original + ONE re-dispatch, got {seen:?}"
        );
        assert_eq!(seen[0], "original task", "the first pass ran the original prompt");
        assert_eq!(
            seen[1], "do X",
            "the SECOND pass re-dispatched with the operator's carried steer instruction"
        );
        assert_eq!(
            fold.outcome,
            RunOutcome::Completed,
            "the re-dispatched run was approved and COMPLETED (the steer was honored, not dropped)"
        );
    }

    #[test]
    fn three_deep_redirect_chain_stops_at_the_cap_with_clear_outcome() {
        // GATE: a run that is steered on EVERY pass must not loop forever. With the
        // default cap of 3 re-dispatches, the chain runs the initial pass + 3
        // re-dispatches = 4 passes, then ends in RedirectCapExhausted (a CLEAR
        // terminal outcome carrying the last steer) — never a silent drop, never
        // an infinite loop.
        let calls = std::cell::Cell::new(0u32);
        let fold = drive_redirect_chain("start".to_string(), DEFAULT_REDIRECT_CAP, |_prompt| {
            calls.set(calls.get() + 1);
            // Always steered -> the chain would loop forever without the cap.
            pass_with(ApprovalDecision::Redirect("steer-again".into()))
        });

        assert_eq!(
            calls.get(),
            DEFAULT_REDIRECT_CAP + 1,
            "initial pass + {DEFAULT_REDIRECT_CAP} re-dispatches = {} total passes, then the cap stops it",
            DEFAULT_REDIRECT_CAP + 1
        );
        assert_eq!(
            fold.outcome,
            RunOutcome::RedirectCapExhausted("steer-again".to_string()),
            "exhausting the cap ends in a CLEAR outcome carrying the last steer, not a silent drop"
        );
    }

    #[test]
    fn non_redirect_outcome_runs_exactly_once() {
        // Control: a pass that does NOT redirect (here: approved -> Completed) must
        // run exactly once — the chain driver is inert when there is no steer.
        let calls = std::cell::Cell::new(0u32);
        let fold = drive_redirect_chain("just do it".to_string(), DEFAULT_REDIRECT_CAP, |_prompt| {
            calls.set(calls.get() + 1);
            pass_with(ApprovalDecision::ApproveOnce)
        });
        assert_eq!(calls.get(), 1, "no redirect => exactly one governed pass");
        assert_eq!(fold.outcome, RunOutcome::Completed);
    }
}
