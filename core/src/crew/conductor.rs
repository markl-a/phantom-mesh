//! The crew conductor — the round-runner brain. Drives the role pipeline on a
//! task: implementer → firewall-A test gate → reviewers → the distinct-vendor
//! gate → LAND / ITERATE / ESCALATE, with firewall-B circuit breakers
//! (no-progress / wall-clock / abort) and quota-aware auto-substitution. Ported
//! from the `ensemble` project (Apache-2.0) §6 crew port.
//!
//! Slice-2d ports the in-memory `run()` brain (hermetically testable via
//! `MockAdapter`). The repo-isolated `run_in_repo` / `run_many` entry points
//! (which need a git worktree + a run journal) are a later slice and will reuse
//! spectyn's own worktree infra rather than ensemble's copy.

use super::adapter::Adapter;
use super::blackboard::Blackboard;
use super::config::{CrewConfig, OnFlake};
use super::gate::{decide, GateDecision, RoleVerdict};
use super::verdict::parse_verdict;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug)]
pub enum Decision {
    Landed,
    Escalated(String),
}

#[derive(Debug)]
pub struct RunOutcome {
    pub decision: Decision,
    pub rounds: u32,
    pub blackboard: Blackboard,
    /// On a LANDED repo-isolated run (a later slice), the branch the committed work was kept on.
    /// Always `None` for this slice's in-memory `run()` (no worktree) and for escalated runs.
    pub branch: Option<String>,
}

pub struct Conductor {
    crew: CrewConfig,
    adapters: HashMap<String, Box<dyn Adapter>>,
    /// Firewall B: a Ctrl-C handler flips this; the conductor bails cleanly at the next round
    /// boundary. Defaults to a never-set flag (so a plain `Conductor::new` is unaffected).
    abort: Arc<AtomicBool>,
    /// S1a live supervision: when set, every blackboard post is mirrored here so a run can be
    /// tailed in real time. `None` ⇒ no streaming (unchanged behaviour).
    stream: Option<Box<dyn super::supervise::RunObserver>>,
    /// S1b control plane: when set, the operator's steer prompts are injected into the next round and a
    /// control abort stops the run (alongside the Ctrl-C `abort` flag). `None` ⇒ no control.
    control: Option<Arc<super::supervise::ControlState>>,
}

/// Firewall B: true when `elapsed_secs` has exceeded a wall-clock budget (`budget == 0` ⇒ no budget).
fn over_budget(elapsed_secs: u64, budget: u64) -> bool {
    budget > 0 && elapsed_secs >= budget
}

impl Conductor {
    pub fn new(crew: CrewConfig, adapters: HashMap<String, Box<dyn Adapter>>) -> Self {
        Self {
            crew,
            adapters,
            abort: Arc::new(AtomicBool::new(false)),
            stream: None,
            control: None,
        }
    }

    /// Wire an external abort flag (set by a Ctrl-C handler) so a run stops cleanly at the next
    /// round boundary (firewall B).
    pub fn with_abort(mut self, flag: Arc<AtomicBool>) -> Self {
        self.abort = flag;
        self
    }

    /// S1a: wire a live stream observer — every blackboard post is mirrored to it so the run is
    /// watchable in real time. Best-effort; it never changes a run's outcome.
    pub fn with_stream(mut self, obs: Box<dyn super::supervise::RunObserver>) -> Self {
        self.stream = Some(obs);
        self
    }

    /// S1b: wire the control plane — the operator's steer prompts (injected next round) and aborts
    /// reach the run via this shared state.
    pub fn with_control(mut self, ctrl: Arc<super::supervise::ControlState>) -> Self {
        self.control = Some(ctrl);
        self
    }

    /// Post to the blackboard AND mirror to the live stream observer (if any) — the single funnel so
    /// the run transcript and the live feed can never drift.
    fn note(&self, bb: &mut Blackboard, from: &str, kind: &str, body: &str) {
        bb.post(from, kind, body);
        if let Some(s) = &self.stream {
            s.post(&super::blackboard::Message {
                from: from.to_string(),
                kind: kind.to_string(),
                body: body.to_string(),
            });
        }
    }

    fn escalated(&self, mut bb: Blackboard, rounds: u32, why: impl Into<String>) -> RunOutcome {
        let why = why.into();
        self.note(&mut bb, "conductor", "decision", &format!("escalated: {why}"));
        RunOutcome {
            decision: Decision::Escalated(why),
            rounds,
            blackboard: bb,
            branch: None,
        }
    }

    /// Whether the operator has aborted (firewall B). A driver loop checks this to stop claiming new
    /// work rather than fail-marking the whole queue.
    pub fn aborted(&self) -> bool {
        self.abort.load(Ordering::Relaxed) || self.control.as_ref().is_some_and(|c| c.aborted())
    }

    fn adapter_for_role(&self, role: &str) -> Option<&dyn Adapter> {
        let agent = &self.crew.roles.get(role)?.agent;
        self.adapters.get(agent).map(|b| b.as_ref())
    }

    /// Run one adapter turn, first arming it with the control plane's hard-abort flag (S1b) so an
    /// `abort --hard` issued mid-turn kills THIS CLI immediately. A no-op arm when no control is wired.
    fn run_adapter(
        &self,
        a: &dyn Adapter,
        prompt: &str,
        cwd: &Path,
    ) -> Result<super::adapter::AgentOutput, super::adapter::AdapterError> {
        if let Some(ctrl) = &self.control {
            a.set_abort(ctrl.hard_flag());
        }
        a.run(prompt, cwd)
    }

    /// Run `role`'s agent, and if it comes back RATE-LIMITED, automatically substitute the agent's
    /// configured backup (a DIFFERENT vendor) once. A quota exhaustion is not a transient flake:
    /// retrying the same CLI before its reset time is futile, and excluding it would weaken the
    /// quorum — so substitution is the correct automatic remediation, applied REGARDLESS of the
    /// `on_flake` policy (which still governs ordinary flakes). The quota reason + reset time are
    /// logged either way. Returns the (possibly substituted) result and the effective agent name.
    fn run_role_with_quota_fallback(
        &self,
        bb: &mut Blackboard,
        role: &str,
        agent_name: &str,
        prompt: &str,
        cwd: &Path,
    ) -> (
        Option<Result<super::adapter::AgentOutput, super::adapter::AdapterError>>,
        String,
    ) {
        let mut effective = agent_name.to_string();
        let mut result = self
            .adapter_for_role(role)
            .map(|a| self.run_adapter(a, prompt, cwd));

        // Snapshot the quota detail (Display carries reason + "retry after <when>") before mutating.
        let detail = match &result {
            Some(Err(super::adapter::AdapterError::RateLimited(info))) => Some(info.to_string()),
            _ => None,
        };
        if let Some(detail) = detail {
            match self.crew.backup_for(agent_name) {
                Some(backup) if self.adapters.contains_key(backup) => {
                    self.note(
                        bb,
                        role,
                        "finding",
                        &format!(
                            "'{agent_name}' rate-limited{detail} — auto-substituting backup '{backup}'"
                        ),
                    );
                    effective = backup.to_string();
                    let b = self.adapters.get(backup).expect("presence checked above");
                    result = Some(self.run_adapter(b.as_ref(), prompt, cwd));
                }
                _ => {
                    self.note(
                        bb,
                        role,
                        "finding",
                        &format!("'{agent_name}' rate-limited{detail} — no backup configured"),
                    );
                }
            }
        }
        (result, effective)
    }

    /// Run the role pipeline on `task` until the gate lands it, escalates, or rounds run out.
    pub fn run(&self, task: &str, cwd: &Path) -> RunOutcome {
        let mut bb = Blackboard::new();
        let mut feedback: Vec<String> = Vec::new();
        let max = self.crew.gate.max_rounds.max(1);
        let started = Instant::now();
        let mut last_sig: Option<String> = None; // firewall B: previous round's progress signature
        let mut same = 0u32; // consecutive identical-signature rounds

        for round in 0..max {
            // ── S1b: drain the operator's steer prompts into this round's implementer prompt (a steer
            // issued during the previous round redirects the next one). ──
            if let Some(ctrl) = &self.control {
                for steer in ctrl.take_steers() {
                    self.note(&mut bb, "operator", "steer", &steer);
                    feedback.push(format!("OPERATOR STEER (follow this now): {steer}"));
                }
            }
            // ── Firewall B: abort (Ctrl-C OR control plane) + wall-clock budget, at each round boundary ──
            if self.aborted() {
                let hard = self.control.as_ref().is_some_and(|c| c.hard());
                self.note(
                    &mut bb,
                    "operator",
                    "interrupted",
                    if hard { "hard abort" } else { "abort" },
                );
                return self.escalated(bb, round, "aborted by operator");
            }
            if over_budget(started.elapsed().as_secs(), self.crew.gate.max_task_secs) {
                return self.escalated(
                    bb,
                    round,
                    format!("timed out after {}s", self.crew.gate.max_task_secs),
                );
            }

            // 1) implementer — quota-aware: a rate-limited implementer is auto-substituted to its
            // backup (a different vendor) rather than escalating the whole run on a transient quota.
            let impl_role = self.crew.implementer_role();
            let impl_agent = self
                .crew
                .roles
                .get(impl_role)
                .map(|r| r.agent.clone())
                .unwrap_or_default();
            let impl_prompt = build_prompt(task, &bb, &feedback, impl_role, false);
            let impl_text;
            let (impl_result, _impl_effective) =
                self.run_role_with_quota_fallback(&mut bb, impl_role, &impl_agent, &impl_prompt, cwd);
            match impl_result {
                Some(Ok(out)) => {
                    impl_text = out.text.clone();
                    self.note(&mut bb, &out.agent, "result", &out.text);
                }
                Some(Err(e)) => {
                    return self.escalated(
                        bb,
                        round + 1,
                        format!("implementer '{impl_role}' failed: {e}"),
                    );
                }
                None => {
                    return self.escalated(
                        bb,
                        round + 1,
                        format!("no adapter for implementer role '{impl_role}'"),
                    );
                }
            }

            // ── TEST GATE (firewall A) ── the project's real tests must be GREEN before the AI
            // reviewers run. RED bounces the traceback back to the implementer (don't spend reviewer
            // turns on code that doesn't pass); a suite that never goes green can NEVER land.
            let mut test_passed = true;
            let mut test_out = String::new();
            if let Some(test) = &self.crew.test {
                let t = super::test_gate::run_tests(cwd, &test.command);
                test_passed = t.passed;
                test_out = t.output.clone();
                self.note(
                    &mut bb,
                    "test",
                    if t.passed { "test_pass" } else { "test_failure" },
                    &t.output,
                );
            }

            // ── CIRCUIT BREAKER (firewall B) ── break early on NO PROGRESS: the implementer's output
            // and the test result are byte-identical to the previous round (it's spinning). Trips
            // before grinding to `max_rounds`. Repeated identical test failures are the sharpest
            // signal — so this sits before the red-bounce below.
            let sig = format!("{impl_text}\u{1}{test_out}");
            if last_sig.as_deref() == Some(sig.as_str()) {
                same += 1;
            } else {
                same = 1;
                last_sig = Some(sig);
            }
            // `.max(2)` so a misconfigured `stall_limit = 1` can't trip at round 0 (the first round
            // has nothing to compare against) — the minimum meaningful value is 2 identical rounds.
            if self.crew.gate.stall_limit > 0 && same >= self.crew.gate.stall_limit.max(2) {
                return self.escalated(
                    bb,
                    round + 1,
                    format!("circuit-broken: no progress across {same} identical rounds"),
                );
            }

            // test RED → bounce the traceback back to the implementer, skip reviewers this round; a
            // suite that never goes green can NEVER land.
            if !test_passed {
                if round + 1 >= max {
                    return self.escalated(
                        bb,
                        round + 1,
                        format!("tests never passed after {} round(s)", round + 1),
                    );
                }
                feedback = vec![format!(
                    "Your changes did not pass the test suite. Fix WITHOUT breaking existing \
                     behaviour. Test output:\n{}",
                    test_out
                )];
                continue;
            }

            // 2) reviewers — a flaked reviewer is EXCLUDED (logged), never counted as approval.
            // When the primary adapter errors, consult `on_flake`: Retry re-runs the same agent
            // once; Substitute falls back to the agent's configured backup. A verdict only enters
            // the quorum from a real `Some(Ok(..))` — a flake is never faked into an approval.
            let mut verdicts: Vec<RoleVerdict> = Vec::new();
            for role in self.crew.reviewer_roles() {
                let prompt = build_prompt(task, &bb, &feedback, role, true);
                let agent_name = self
                    .crew
                    .roles
                    .get(role)
                    .map(|r| r.agent.clone())
                    .unwrap_or_default();
                // Quota-aware first: a rate-limited reviewer is auto-substituted to its backup
                // (preserves the quorum) regardless of on_flake. Ordinary (non-quota) flakes then
                // fall to the configured on_flake policy below.
                let (mut result, mut effective_agent) =
                    self.run_role_with_quota_fallback(&mut bb, role, &agent_name, &prompt, cwd);

                let was_quota = matches!(
                    result,
                    Some(Err(super::adapter::AdapterError::RateLimited(_)))
                );
                if matches!(result, Some(Err(_))) && !was_quota {
                    match self.crew.gate.on_flake {
                        OnFlake::Exclude => {}
                        OnFlake::Retry => {
                            self.note(&mut bb, role, "finding", "reviewer flaked — retrying once");
                            result = self
                                .adapter_for_role(role)
                                .map(|a| self.run_adapter(a, &prompt, cwd));
                        }
                        OnFlake::Substitute => {
                            if let Some(backup) = self.crew.backup_for(&agent_name) {
                                if let Some(b) = self.adapters.get(backup) {
                                    self.note(
                                        &mut bb,
                                        role,
                                        "finding",
                                        &format!("reviewer flaked — substituting backup '{backup}'"),
                                    );
                                    effective_agent = backup.to_string();
                                    result = Some(self.run_adapter(b.as_ref(), &prompt, cwd));
                                }
                            }
                        }
                    }
                }

                match result {
                    Some(Ok(out)) => {
                        let v = parse_verdict(&out.text);
                        self.note(&mut bb, &out.agent, "verdict", &out.text);
                        verdicts.push(RoleVerdict {
                            role: role.to_string(),
                            agent: effective_agent,
                            verdict: v,
                        });
                    }
                    Some(Err(e)) => {
                        self.note(
                            &mut bb,
                            role,
                            "finding",
                            &format!("reviewer excluded — flaked: {e}"),
                        );
                    }
                    None => {
                        self.note(
                            &mut bb,
                            role,
                            "finding",
                            &format!("reviewer excluded — no adapter for role '{role}'"),
                        );
                    }
                }
            }

            // 3) gate
            match decide(&verdicts, &self.crew.gate, round) {
                GateDecision::Land => {
                    // Honor an operator abort that arrived mid-round: don't land work the operator
                    // asked to stop (firewall B). A budget overrun on a SUCCESSFUL round still lands
                    // — we don't discard completed work for the wall-clock net.
                    if self.aborted() {
                        let hard = self.control.as_ref().is_some_and(|c| c.hard());
                        self.note(
                            &mut bb,
                            "operator",
                            "interrupted",
                            if hard { "hard abort" } else { "abort" },
                        );
                        return self.escalated(bb, round + 1, "aborted by operator");
                    }
                    self.note(&mut bb, "conductor", "decision", "LANDED");
                    return RunOutcome {
                        decision: Decision::Landed,
                        rounds: round + 1,
                        blackboard: bb,
                        branch: None,
                    };
                }
                GateDecision::Escalate(why) => {
                    return self.escalated(bb, round + 1, why);
                }
                GateDecision::Iterate(changes) => {
                    feedback = changes;
                }
            }
        }
        self.escalated(bb, max, "max rounds reached")
    }
}

fn build_prompt(
    task: &str,
    bb: &Blackboard,
    feedback: &[String],
    role: &str,
    is_reviewer: bool,
) -> String {
    let _ = role;
    let summary = bb.summary();
    if is_reviewer {
        // Reviewer: judge the IMPLEMENTER's work — do NOT redo the task — and end with a parseable
        // VERDICT line (else `parse_verdict` conservatively reads it as changes-requested and the
        // task can never land).
        let mut p = format!(
            "You are a REVIEWER on a dev crew. A teammate (the implementer) was asked to do:\n\
             TASK: {task}\n\n"
        );
        if !summary.is_empty() {
            p.push_str("Activity so far (the implementer's output is the `result` entry):\n");
            p.push_str(&summary);
            p.push('\n');
        }
        p.push_str(
            "Judge ONLY whether the implementer's work satisfies the task. Do NOT do the task \
             yourself. Give a one-line reason, then end with EXACTLY one final line:\n\
             VERDICT: LGTM                    (if it satisfies the task)\n\
             VERDICT: CHANGES: <what to fix>  (otherwise)\n",
        );
        p
    } else {
        // Implementer: do the task now and produce the deliverable.
        let mut p = format!(
            "You are the IMPLEMENTER on a dev crew. Do this task now and produce the deliverable:\n\
             TASK: {task}\n"
        );
        if !feedback.is_empty() {
            p.push_str("\nA reviewer asked you to fix:\n");
            for f in feedback {
                p.push_str(&format!("- {f}\n"));
            }
        }
        if !summary.is_empty() {
            p.push('\n');
            p.push_str(&summary);
            p.push('\n');
        }
        p.push_str("\nAfter doing it, briefly state what you produced.\n");
        p
    }
}

#[cfg(test)]
mod tests {
    use super::over_budget;

    #[test]
    fn over_budget_respects_a_zero_disabled_budget() {
        assert!(!over_budget(0, 0)); // 0 budget = disabled
        assert!(!over_budget(9_999, 0)); // disabled even at large elapsed
        assert!(!over_budget(2, 3)); // under budget
        assert!(over_budget(3, 3)); // at budget
        assert!(over_budget(5, 3)); // over budget
    }

    #[test]
    fn run_mirrors_blackboard_posts_to_the_observer() {
        use super::*;
        use crate::crew::adapter::{Adapter, MockAdapter};
        use crate::crew::supervise::RunObserver;
        use crate::crew::{CrewConfig, GatePolicy, OnFlake, RoleConfig};
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        // an observer that records (from, kind) of each mirrored post, behind an Arc so the test
        // can inspect it after the conductor has consumed the Box<dyn RunObserver>.
        struct Rec(Arc<Mutex<Vec<(String, String)>>>);
        impl RunObserver for Rec {
            fn post(&self, m: &crate::crew::blackboard::Message) {
                self.0.lock().unwrap().push((m.from.clone(), m.kind.clone()));
            }
        }

        // a crew that lands in one round: impl -> one approving reviewer, min_approvals = 1
        let crew = CrewConfig {
            gate: GatePolicy {
                min_approvals: 1,
                max_rounds: 1,
                on_flake: OnFlake::Exclude,
                stall_limit: 0,
                max_task_secs: 0,
            },
            pipeline: vec!["implement".to_string(), "review".to_string()],
            roles: HashMap::from([
                (
                    "implement".to_string(),
                    RoleConfig { agent: "impl".to_string(), blind: false },
                ),
                (
                    "review".to_string(),
                    RoleConfig { agent: "rev".to_string(), blind: false },
                ),
            ]),
            agents: HashMap::new(),
            test: None,
        };
        let mut adapters: HashMap<String, Box<dyn Adapter>> = HashMap::new();
        adapters.insert(
            "impl".to_string(),
            Box::new(MockAdapter::new("impl", vec![Ok("implemented it".to_string())])),
        );
        adapters.insert(
            "rev".to_string(),
            Box::new(MockAdapter::new("rev", vec![Ok("VERDICT: LGTM".to_string())])),
        );

        let log = Arc::new(Mutex::new(Vec::new()));
        let c = Conductor::new(crew, adapters).with_stream(Box::new(Rec(log.clone())));
        let out = c.run("do the thing", std::path::Path::new("."));
        assert!(
            matches!(out.decision, Decision::Landed),
            "should land: {:?}",
            out.decision
        );

        let seen = log.lock().unwrap().clone();
        assert!(
            seen.iter().any(|(f, k)| f == "impl" && k == "result"),
            "implementer result streamed: {seen:?}"
        );
        assert!(
            seen.iter().any(|(f, k)| f == "rev" && k == "verdict"),
            "reviewer verdict streamed: {seen:?}"
        );
        assert!(
            seen.iter().any(|(f, k)| f == "conductor" && k == "decision"),
            "terminal decision streamed: {seen:?}"
        );
    }

    #[test]
    fn implementer_failure_streams_terminal_decision() {
        use super::*;
        use crate::crew::adapter::{Adapter, AdapterError, MockAdapter};
        use crate::crew::blackboard::Message;
        use crate::crew::supervise::RunObserver;
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        struct Rec(Arc<Mutex<Vec<(String, String, String)>>>);
        impl RunObserver for Rec {
            fn post(&self, m: &Message) {
                self.0
                    .lock()
                    .unwrap()
                    .push((m.from.clone(), m.kind.clone(), m.body.clone()));
            }
        }

        let mut adapters: HashMap<String, Box<dyn Adapter>> = HashMap::new();
        adapters.insert(
            "impl".to_string(),
            Box::new(MockAdapter::new(
                "impl",
                vec![Err(AdapterError::Flaked("boom".to_string()))],
            )),
        );

        let log = Arc::new(Mutex::new(Vec::new()));
        let c =
            Conductor::new(impl_review_crew(1), adapters).with_stream(Box::new(Rec(log.clone())));
        let out = c.run("do the thing", std::path::Path::new("."));

        assert!(
            matches!(out.decision, Decision::Escalated(ref why) if why.contains("implementer")),
            "expected implementer escalation, got {:?}",
            out.decision
        );
        let seen = log.lock().unwrap();
        assert!(
            seen.iter().any(|(from, kind, body)| {
                from == "conductor" && kind == "decision" && body.contains("escalated")
            }),
            "terminal decision streamed: {seen:?}"
        );
    }

    // ---- feature 3: quota-aware auto-remediation ----

    #[test]
    fn rate_limited_implementer_is_auto_substituted_to_backup_and_lands() {
        use super::*;
        use crate::crew::adapter::{Adapter, AdapterError, MockAdapter, RateLimitInfo};
        use crate::crew::{AgentConfig, CrewConfig, GatePolicy, OnFlake, RoleConfig};
        use std::collections::HashMap;

        // implement=codex (rate-limited, backup=opencode), review=claude (approves).
        let crew = CrewConfig {
            gate: GatePolicy {
                min_approvals: 1,
                max_rounds: 1,
                on_flake: OnFlake::Exclude,
                stall_limit: 0,
                max_task_secs: 0,
            },
            pipeline: vec!["implement".to_string(), "review".to_string()],
            roles: HashMap::from([
                (
                    "implement".to_string(),
                    RoleConfig { agent: "codex".to_string(), blind: false },
                ),
                (
                    "review".to_string(),
                    RoleConfig { agent: "claude".to_string(), blind: false },
                ),
            ]),
            agents: HashMap::from([(
                "codex".to_string(),
                AgentConfig {
                    backup: Some("opencode".to_string()),
                    node: None,
                    args: None,
                    timeout: None,
                },
            )]),
            test: None,
        };
        let mut adapters: HashMap<String, Box<dyn Adapter>> = HashMap::new();
        adapters.insert(
            "codex".to_string(),
            Box::new(MockAdapter::new(
                "codex",
                vec![Err(AdapterError::RateLimited(RateLimitInfo {
                    reason: "You've hit your usage limit".to_string(),
                    retry_at: Some("Jun 25th, 2026 5:33 AM".to_string()),
                }))],
            )),
        );
        adapters.insert(
            "opencode".to_string(),
            Box::new(MockAdapter::new("opencode", vec![Ok("implemented it".to_string())])),
        );
        adapters.insert(
            "claude".to_string(),
            Box::new(MockAdapter::new("claude", vec![Ok("VERDICT: LGTM".to_string())])),
        );

        let out = Conductor::new(crew, adapters).run("task", std::path::Path::new("."));
        // Before feature 3 this escalated ("implementer 'implement' failed"); now it substitutes.
        assert!(
            matches!(out.decision, Decision::Landed),
            "a rate-limited implementer must fall back to its backup and land: {:?}",
            out.decision
        );
        let msgs = out.blackboard.read_since(0);
        assert!(
            msgs.iter().any(|m| m.from == "opencode" && m.kind == "result"),
            "backup 'opencode' should have produced the implementation: {msgs:?}"
        );
        assert!(
            msgs.iter().any(|m| m.kind == "finding"
                && m.body.contains("retry after Jun 25th, 2026 5:33 AM")),
            "the quota reason + reset time must be logged: {msgs:?}"
        );
    }

    #[test]
    fn rate_limited_reviewer_is_substituted_even_when_on_flake_is_exclude() {
        use super::*;
        use crate::crew::adapter::{Adapter, AdapterError, MockAdapter, RateLimitInfo};
        use crate::crew::{AgentConfig, CrewConfig, GatePolicy, OnFlake, RoleConfig};
        use std::collections::HashMap;

        // on_flake=Exclude would normally DROP a flaked reviewer (losing the quorum). A quota is
        // different: the reviewer must be substituted so the single required approval still arrives.
        let crew = CrewConfig {
            gate: GatePolicy {
                min_approvals: 1,
                max_rounds: 1,
                on_flake: OnFlake::Exclude,
                stall_limit: 0,
                max_task_secs: 0,
            },
            pipeline: vec!["implement".to_string(), "review".to_string()],
            roles: HashMap::from([
                (
                    "implement".to_string(),
                    RoleConfig { agent: "codex".to_string(), blind: false },
                ),
                (
                    "review".to_string(),
                    RoleConfig { agent: "claude".to_string(), blind: false },
                ),
            ]),
            agents: HashMap::from([(
                "claude".to_string(),
                AgentConfig {
                    backup: Some("opencode".to_string()),
                    node: None,
                    args: None,
                    timeout: None,
                },
            )]),
            test: None,
        };
        let mut adapters: HashMap<String, Box<dyn Adapter>> = HashMap::new();
        adapters.insert(
            "codex".to_string(),
            Box::new(MockAdapter::new("codex", vec![Ok("implemented it".to_string())])),
        );
        adapters.insert(
            "claude".to_string(),
            Box::new(MockAdapter::new(
                "claude",
                vec![Err(AdapterError::RateLimited(RateLimitInfo::default()))],
            )),
        );
        adapters.insert(
            "opencode".to_string(),
            Box::new(MockAdapter::new("opencode", vec![Ok("VERDICT: LGTM".to_string())])),
        );

        let out = Conductor::new(crew, adapters).run("task", std::path::Path::new("."));
        assert!(
            matches!(out.decision, Decision::Landed),
            "a rate-limited reviewer must be substituted (not excluded) so the quorum is met: {:?}",
            out.decision
        );
        let msgs = out.blackboard.read_since(0);
        assert!(
            msgs.iter().any(|m| m.from == "opencode" && m.kind == "verdict"),
            "backup 'opencode' should have cast the approving verdict: {msgs:?}"
        );
    }

    // ---- S1b control-plane tests ----

    fn impl_review_crew(min_approvals: u32) -> crate::crew::CrewConfig {
        use crate::crew::{CrewConfig, GatePolicy, OnFlake, RoleConfig};
        use std::collections::HashMap;
        CrewConfig {
            gate: GatePolicy {
                min_approvals,
                max_rounds: 1,
                on_flake: OnFlake::Exclude,
                stall_limit: 0,
                max_task_secs: 0,
            },
            pipeline: vec!["implement".to_string(), "review".to_string()],
            roles: HashMap::from([
                (
                    "implement".to_string(),
                    RoleConfig { agent: "impl".to_string(), blind: false },
                ),
                (
                    "review".to_string(),
                    RoleConfig { agent: "rev".to_string(), blind: false },
                ),
            ]),
            agents: HashMap::new(),
            test: None,
        }
    }

    #[test]
    fn steer_reaches_the_next_round_implementer_prompt() {
        use super::*;
        use crate::crew::adapter::{Adapter, AdapterError, AgentOutput, MockAdapter};
        use crate::crew::supervise::ControlState;
        use std::collections::HashMap;
        use std::path::Path;
        use std::sync::{Arc, Mutex};

        // a recorder implementer that captures the prompt it was handed
        struct Recorder {
            prompts: Arc<Mutex<Vec<String>>>,
        }
        impl Adapter for Recorder {
            fn name(&self) -> &str {
                "impl"
            }
            fn run(&self, prompt: &str, _cwd: &Path) -> Result<AgentOutput, AdapterError> {
                self.prompts.lock().unwrap().push(prompt.to_string());
                Ok(AgentOutput {
                    agent: "impl".to_string(),
                    text: "did it".to_string(),
                })
            }
        }
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let mut adapters: HashMap<String, Box<dyn Adapter>> = HashMap::new();
        adapters.insert(
            "impl".to_string(),
            Box::new(Recorder { prompts: prompts.clone() }),
        );
        adapters.insert(
            "rev".to_string(),
            Box::new(MockAdapter::new("rev", vec![Ok("VERDICT: LGTM".to_string())])),
        );

        let ctrl = Arc::new(ControlState::default());
        ctrl.push_steer("FOCUS-ON-ERROR-HANDLING"); // seed a steer before the run
        let c = Conductor::new(impl_review_crew(1), adapters).with_control(ctrl);
        let out = c.run("do the thing", Path::new("."));
        assert!(
            matches!(out.decision, Decision::Landed),
            "should land: {:?}",
            out.decision
        );
        let seen = prompts.lock().unwrap();
        assert!(
            seen.iter().any(|p| p.contains("FOCUS-ON-ERROR-HANDLING")),
            "the steer must be injected into the implementer's prompt: {seen:?}"
        );
    }

    #[test]
    fn control_abort_stops_the_run_and_streams_interrupted() {
        use super::*;
        use crate::crew::adapter::{Adapter, MockAdapter};
        use crate::crew::blackboard::Message;
        use crate::crew::supervise::{ControlState, RunObserver};
        use std::collections::HashMap;
        use std::path::Path;
        use std::sync::atomic::Ordering;
        use std::sync::{Arc, Mutex};

        struct Rec(Arc<Mutex<Vec<(String, String, String)>>>);
        impl RunObserver for Rec {
            fn post(&self, m: &Message) {
                self.0
                    .lock()
                    .unwrap()
                    .push((m.from.clone(), m.kind.clone(), m.body.clone()));
            }
        }
        let mut adapters: HashMap<String, Box<dyn Adapter>> = HashMap::new();
        adapters.insert(
            "impl".to_string(),
            Box::new(MockAdapter::new("impl", vec![Ok("x".to_string())])),
        );
        adapters.insert(
            "rev".to_string(),
            Box::new(MockAdapter::new("rev", vec![Ok("VERDICT: LGTM".to_string())])),
        );

        let ctrl = Arc::new(ControlState::default());
        ctrl.abort_flag().store(true, Ordering::Relaxed); // operator already aborted (clean)
        let log = Arc::new(Mutex::new(Vec::new()));
        let c = Conductor::new(impl_review_crew(1), adapters)
            .with_control(ctrl)
            .with_stream(Box::new(Rec(log.clone())));
        let out = c.run("do the thing", Path::new("."));
        assert!(
            matches!(out.decision, Decision::Escalated(ref w) if w.contains("aborted")),
            "an abort must stop the run, got {:?}",
            out.decision
        );
        let seen = log.lock().unwrap();
        assert!(
            seen.iter().any(|(f, k, _)| f == "operator" && k == "interrupted"),
            "interrupted streamed: {seen:?}"
        );
        assert!(
            seen.iter().any(|(from, kind, body)| {
                from == "conductor" && kind == "decision" && body.contains("escalated")
            }),
            "terminal decision streamed: {seen:?}"
        );
    }
}
