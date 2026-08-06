//! Escalation seam: on a high-risk action, alert the operator and (for blocking
//! enforcement) await their decision, correlated by approval_id. Production uses
//! notifications + inbox; tests use MockEscalator.

use crate::execution_contract::{ApprovalDecision, RiskLevel};
use crate::inbox::{self, InboxMessage};
use crate::notifications::{Notification, NotificationDispatcher};
use pm_types::TaskStatus;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use uuid::Uuid;

pub trait Escalator: Send {
    /// Blocking alert (claude pre-action): notify + wait for a decision (or the
    /// policy timeout, handled by the caller). Returns the operator's decision.
    fn await_decision(&mut self, approval_id: &str, tool: &str, risk: RiskLevel) -> ApprovalDecision;
    /// Non-blocking alert (post-action): notify; return true if the operator says STOP.
    fn alert_observed(&mut self, approval_id: &str, tool: &str, risk: RiskLevel) -> bool;
}

/// Test escalator: scripted decisions + STOP flags by approval_id, plus `force_*`
/// overrides for tests that can't know the fresh uuid `drive` mints internally.
#[derive(Default)]
pub struct MockEscalator {
    pub decisions: std::collections::HashMap<String, ApprovalDecision>,
    pub stops: std::collections::HashMap<String, bool>,
    pub sent: Vec<String>,
    /// If set, overrides the per-id decision lookup (the id is a runtime uuid).
    pub force_decision: Option<ApprovalDecision>,
    /// If true, every `alert_observed` returns STOP regardless of id.
    pub force_stop: bool,
}
impl Escalator for MockEscalator {
    fn await_decision(&mut self, approval_id: &str, tool: &str, _risk: RiskLevel) -> ApprovalDecision {
        self.sent.push(format!("await:{approval_id}:{tool}"));
        // `ApprovalDecision` is no longer `Copy` (the `Redirect(String)` variant
        // carries a payload), so clone out of the `&mut self` borrow.
        if let Some(d) = &self.force_decision {
            return d.clone();
        }
        self.decisions
            .get(approval_id)
            .cloned()
            .unwrap_or(ApprovalDecision::Deny) // fail-safe default
    }
    fn alert_observed(&mut self, approval_id: &str, tool: &str, _risk: RiskLevel) -> bool {
        self.sent.push(format!("alert:{approval_id}:{tool}"));
        if self.force_stop {
            return true;
        }
        *self.stops.get(approval_id).unwrap_or(&false)
    }
}

/// Pull an ApprovalDecision out of a free-form reply ("approve", "yes",
/// "deny contract-123", ...): exact parse first, else the first token.
fn parse_decision(text: &str) -> Option<ApprovalDecision> {
    ApprovalDecision::from_str(text)
        .or_else(|| text.split_whitespace().next().and_then(ApprovalDecision::from_str))
}

/// A reply correlates to a pending approval when its topic IS the approval_id, or
/// the id appears anywhere in the text (a phone reply that quotes the card).
fn correlated(m: &InboxMessage, approval_id: &str) -> bool {
    m.topic.as_deref() == Some(approval_id) || m.text.contains(approval_id)
}

/// Unix milliseconds, saturating to 0 if the clock is before the epoch — the
/// `created_ms` sort key for a pending approval card.
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Production escalator: sends a phone notification and reads the operator's reply
/// from the inbox. Bridges the sync `Escalator` trait to the async dispatcher via
/// a tokio runtime `Handle`, so it MUST run on a blocking (non-worker) thread.
pub struct PhoneEscalator {
    home: PathBuf,
    dispatcher: NotificationDispatcher,
    handle: Handle,
    task_id: Uuid,
    workspace_id: String,
    poll: Duration,
    deadline: Duration,
    fallback: ApprovalDecision,
    /// apex-④ dispatch↔govern correlation: when this run is governing a DISPATCHED
    /// task, the `TaskStore` holding that dispatch row (`task_id` IS the dispatch
    /// `job_uuid` — see `run_govern_folded`). At pending-card-write time the
    /// escalator stamps the pending `approval_id` (= `ExecutionContract.id`) onto
    /// the dispatch row AND transitions it `Running -> AwaitingApproval`, live (the
    /// run is blocked in `await_decision`). Best-effort: a store error never changes
    /// the returned decision. `None` (standalone `spectyn govern`, ungoverned) =
    /// no row to correlate, behavior unchanged.
    store: Option<crate::tasks::TaskStore>,
}

impl PhoneEscalator {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        home: PathBuf,
        dispatcher: NotificationDispatcher,
        handle: Handle,
        task_id: Uuid,
        workspace_id: impl Into<String>,
        poll: Duration,
        deadline: Duration,
        fallback: ApprovalDecision,
    ) -> Self {
        Self {
            home,
            dispatcher,
            handle,
            task_id,
            workspace_id: workspace_id.into(),
            poll,
            deadline,
            fallback,
            store: None,
        }
    }

    /// apex-④: attach the dispatch row's `TaskStore` so the escalator correlates a
    /// pending approval onto the dispatch task row (stamps `approval_id` + moves it
    /// to `AwaitingApproval`) at pending-card-write time. Builder-style; additive
    /// (an escalator with no store behaves byte-identically to today).
    pub fn with_dispatch_store(mut self, store: crate::tasks::TaskStore) -> Self {
        self.store = Some(store);
        self
    }

    /// Best-effort: correlate `approval_id` onto the dispatch row (`self.task_id`)
    /// and transition it `Running -> AwaitingApproval`, while the run is blocked
    /// awaiting the decision. A store error is logged and swallowed — it must NEVER
    /// change the decision the escalator returns (the run's safety does not depend
    /// on this bookkeeping). No-op when no dispatch store is attached.
    fn correlate_dispatch_row(&self, approval_id: &str) {
        let Some(store) = &self.store else {
            return;
        };
        let task_id = self.task_id;
        self.handle.block_on(async {
            if let Err(e) = store.set_approval_id(task_id, approval_id).await {
                tracing::warn!(
                    target: "spectyn::govern",
                    task_id = %task_id,
                    "set_approval_id on dispatch row failed (best-effort): {e}"
                );
            }
            // Move the dispatch row into AwaitingApproval so `/tasks` /
            // `/rpc/task/status/:job_id` reflect "blocked on a phone approval".
            // Running -> AwaitingApproval is a legal transition; on a dispatch row
            // that is already AwaitingApproval (a later approval in the same run)
            // this no-ops the status while the latest approval_id is still stamped.
            if let Err(e) = store
                .update_status(task_id, TaskStatus::AwaitingApproval, None)
                .await
            {
                tracing::warn!(
                    target: "spectyn::govern",
                    task_id = %task_id,
                    "transition dispatch row to AwaitingApproval failed (best-effort): {e}"
                );
            }
        });
    }

    fn send(&self, title: String, body: String) {
        let n = Notification::task_update(
            self.task_id,
            self.workspace_id.clone(),
            TaskStatus::AwaitingApproval,
            title,
            body,
        );
        // Bridge sync -> async. Safe because the escalator runs on a blocking thread.
        self.handle.block_on(self.dispatcher.notify(n));
    }

    /// Look for a STOP/ABORT reply that references THIS action (`approval_id`) OR
    /// the whole run (`task_id`). The run-level form is what lets an operator's
    /// STOP — sent in reply to an earlier observed action — be honored at a LATER
    /// event, instead of being orphaned to the action it replied to.
    fn stop_request_id(&self, approval_id: &str) -> Option<String> {
        let run = self.task_id.to_string();
        let msgs = inbox::list_messages(&self.home).ok()?;
        for m in msgs {
            let txt = m.text.to_ascii_uppercase();
            let says_stop = txt.contains("STOP") || txt.contains("ABORT");
            let refs_run = m.topic.as_deref() == Some(run.as_str()) || m.text.contains(&run);
            if says_stop && (correlated(&m, approval_id) || refs_run) {
                return Some(m.id);
            }
        }
        None
    }
}

impl Escalator for PhoneEscalator {
    fn await_decision(&mut self, approval_id: &str, tool: &str, risk: RiskLevel) -> ApprovalDecision {
        self.send(
            format!("Approve {tool}?"),
            format!("risk={} id={approval_id} -- reply approve / deny", risk.as_str()),
        );
        // Mirror this pending approval to the filesystem store so a phone app can
        // LIST what's awaiting a decision (apex-④). Best-effort: a store error
        // must NEVER change the decision we return, so `let _ =` everything.
        // The card is removed on EVERY return path below (operator decided AND
        // timeout fallback) — keep that invariant if you add a new return.
        let _ = crate::pending_approvals::write_pending(
            &self.home,
            &crate::pending_approvals::PendingCard {
                approval_id: approval_id.to_string(),
                task_id: self.task_id.to_string(),
                tool: tool.to_string(),
                risk: risk.as_str().to_string(),
                reason: "pre-action approval".to_string(),
                created_ms: now_unix_ms(),
            },
        );
        // apex-④ dispatch↔govern correlation: while the run is blocked here, stamp
        // this approval_id onto the dispatch task row and mark it AwaitingApproval
        // so a phone listing `/tasks` / `/rpc/task/status/:job_id` can map the job
        // to its pending card by the shared id. Best-effort (a store error never
        // changes the decision); no-op when no dispatch store is attached.
        self.correlate_dispatch_row(approval_id);
        let start = Instant::now();
        loop {
            if let Ok(msgs) = inbox::list_messages(&self.home) {
                for m in &msgs {
                    if correlated(m, approval_id) {
                        if let Some(d) = parse_decision(&m.text) {
                            let _ = inbox::ack_message(&self.home, &m.id);
                            // Operator decided — clear the pending card before returning.
                            let _ = crate::pending_approvals::remove_pending(&self.home, approval_id);
                            return d;
                        }
                    }
                }
            }
            if start.elapsed() >= self.deadline {
                // Timeout fallback — clear the pending card before returning.
                let _ = crate::pending_approvals::remove_pending(&self.home, approval_id);
                // `ApprovalDecision` is no longer `Copy` (`Redirect(String)` payload),
                // so clone the fail-safe fallback out of `&mut self`.
                return self.fallback.clone(); // fail-safe: no operator response within deadline
            }
            std::thread::sleep(self.poll);
        }
    }

    fn alert_observed(&mut self, approval_id: &str, tool: &str, risk: RiskLevel) -> bool {
        let run = self.task_id;
        self.send(
            format!("Ran {tool}"),
            format!("risk={} id={approval_id} run={run} -- reply STOP {run} to abort the run", risk.as_str()),
        );
        // A STOP referencing this action OR the run (task_id) aborts; the run-level
        // form is honored at any later event, not orphaned to one approval_id.
        if let Some(id) = self.stop_request_id(approval_id) {
            let _ = inbox::ack_message(&self.home, &id);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_returns_scripted_decision_else_deny() {
        let mut e = MockEscalator::default();
        e.decisions.insert("a1".into(), ApprovalDecision::ApproveOnce);
        assert_eq!(
            e.await_decision("a1", "Bash", RiskLevel::ExecuteHigh),
            ApprovalDecision::ApproveOnce
        );
        assert_eq!(
            e.await_decision("a2", "Bash", RiskLevel::ExecuteHigh),
            ApprovalDecision::Deny
        ); // fail-safe
        assert!(e.sent.iter().any(|s| s.starts_with("await:a1")));
    }

    #[test]
    fn force_decision_overrides_lookup() {
        let mut e = MockEscalator::default();
        e.force_decision = Some(ApprovalDecision::ApproveOnce);
        assert_eq!(
            e.await_decision("unknown-uuid", "Bash", RiskLevel::ExecuteHigh),
            ApprovalDecision::ApproveOnce
        );
    }

    #[test]
    fn force_stop_makes_alert_say_stop() {
        let mut e = MockEscalator::default();
        e.force_stop = true;
        assert!(e.alert_observed("unknown-uuid", "Bash", RiskLevel::ExecuteHigh));
    }

    #[test]
    fn parse_decision_handles_extra_tokens() {
        assert_eq!(parse_decision("approve"), Some(ApprovalDecision::ApproveOnce));
        assert_eq!(parse_decision("deny contract-123"), Some(ApprovalDecision::Deny));
        assert_eq!(parse_decision("yes"), Some(ApprovalDecision::ApproveOnce));
        assert_eq!(parse_decision("maybe later"), None);
    }

    #[test]
    fn parse_decision_routes_redirect_with_instruction() {
        // Apex ④: a `redirect <new instruction>` reply flows through ApprovalDecision
        // parsing and the FULL instruction (not just the first token) survives.
        assert_eq!(
            parse_decision("redirect work on the login bug instead"),
            Some(ApprovalDecision::Redirect("work on the login bug instead".to_string()))
        );
    }

    #[test]
    fn phone_escalator_reads_redirect_from_inbox() {
        // The operator's inbox reply is a redirect; await_decision returns it with
        // the new instruction intact.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let home = std::env::temp_dir().join(format!("gr-esc-redir-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let approval_id = "contract-redir";
        inbox::write_message(&home, "phone", "redirect ship the hotfix", Some(approval_id))
            .unwrap();

        let mut esc = PhoneEscalator::new(
            home.clone(),
            NotificationDispatcher::new(),
            rt.handle().clone(),
            Uuid::new_v4(),
            "default",
            Duration::from_millis(50),
            Duration::from_secs(2),
            ApprovalDecision::Deny,
        );
        let d = esc.await_decision(approval_id, "Bash", RiskLevel::ExecuteHigh);
        assert_eq!(d, ApprovalDecision::Redirect("ship the hotfix".to_string()));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn force_decision_overrides_with_redirect() {
        // MockEscalator can force a Redirect (used by the drive_fold redirect test).
        let mut e = MockEscalator::default();
        e.force_decision = Some(ApprovalDecision::Redirect("new goal".into()));
        assert_eq!(
            e.await_decision("unknown-uuid", "Bash", RiskLevel::ExecuteHigh),
            ApprovalDecision::Redirect("new goal".into())
        );
    }

    #[test]
    fn phone_escalator_reads_decision_from_inbox() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let home = std::env::temp_dir().join(format!("gr-esc-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let approval_id = "contract-xyz";
        inbox::write_message(&home, "phone", "approve", Some(approval_id)).unwrap();

        let mut esc = PhoneEscalator::new(
            home.clone(),
            NotificationDispatcher::new(),
            rt.handle().clone(),
            Uuid::new_v4(),
            "default",
            Duration::from_millis(50),
            Duration::from_secs(2),
            ApprovalDecision::Deny,
        );
        let d = esc.await_decision(approval_id, "Bash", RiskLevel::ExecuteHigh);
        assert_eq!(d, ApprovalDecision::ApproveOnce);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn phone_escalator_times_out_to_fallback() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let home = std::env::temp_dir().join(format!("gr-esc-to-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();

        let mut esc = PhoneEscalator::new(
            home.clone(),
            NotificationDispatcher::new(),
            rt.handle().clone(),
            Uuid::new_v4(),
            "default",
            Duration::from_millis(20),
            Duration::from_millis(60),
            ApprovalDecision::Deny,
        );
        let d = esc.await_decision("nobody-replies", "Bash", RiskLevel::ExecuteHigh);
        assert_eq!(d, ApprovalDecision::Deny); // fail-safe fallback
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn phone_escalator_timeout_honors_configured_nondefault_fallback() {
        // Invariant ④: a timed-out approval resolves to the CONFIGURED fallback, not
        // a hardcoded Deny. With fallback = Cancel (a non-default, still-refusing
        // decision) and no operator reply, the timeout must resolve to Cancel —
        // proving the configured value genuinely flows through to the return path.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let home = std::env::temp_dir().join(format!("gr-esc-cfgfb-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();

        let mut esc = PhoneEscalator::new(
            home.clone(),
            NotificationDispatcher::new(),
            rt.handle().clone(),
            Uuid::new_v4(),
            "default",
            Duration::from_millis(20),
            Duration::from_millis(60),
            ApprovalDecision::Cancel, // non-default configured fallback
        );
        let d = esc.await_decision("nobody-replies", "Bash", RiskLevel::ExecuteHigh);
        assert_eq!(
            d,
            ApprovalDecision::Cancel,
            "timeout must resolve to the CONFIGURED fallback, not a hardcoded Deny"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn phone_escalator_alert_detects_stop() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let home = std::env::temp_dir().join(format!("gr-esc-stop-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let approval_id = "obs-1";
        inbox::write_message(&home, "phone", "STOP", Some(approval_id)).unwrap();

        let mut esc = PhoneEscalator::new(
            home.clone(),
            NotificationDispatcher::new(),
            rt.handle().clone(),
            Uuid::new_v4(),
            "default",
            Duration::from_millis(20),
            Duration::from_secs(1),
            ApprovalDecision::Deny,
        );
        assert!(esc.alert_observed(approval_id, "Bash", RiskLevel::ExecuteHigh));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn await_decision_writes_then_removes_pending_card_on_decision() {
        // When the operator's reply is already waiting, await_decision returns
        // immediately AND leaves no pending card behind (it removed the one it
        // wrote on entry).
        let rt = tokio::runtime::Runtime::new().unwrap();
        let home = std::env::temp_dir().join(format!("gr-esc-pend-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let approval_id = "contract-pend";
        inbox::write_message(&home, "phone", "approve", Some(approval_id)).unwrap();

        let mut esc = PhoneEscalator::new(
            home.clone(),
            NotificationDispatcher::new(),
            rt.handle().clone(),
            Uuid::new_v4(),
            "default",
            Duration::from_millis(20),
            Duration::from_secs(2),
            ApprovalDecision::Deny,
        );
        let d = esc.await_decision(approval_id, "Bash", RiskLevel::ExecuteHigh);
        assert_eq!(d, ApprovalDecision::ApproveOnce);
        // No card lingers after the decision return path.
        assert!(
            crate::pending_approvals::list_pending(&home).unwrap().is_empty(),
            "pending card must be removed on the decision return path"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn await_decision_removes_pending_card_on_timeout() {
        // No operator reply -> times out to fallback AND removes the card it
        // wrote on entry (the timeout return path also clears the store).
        let rt = tokio::runtime::Runtime::new().unwrap();
        let home = std::env::temp_dir().join(format!("gr-esc-pend-to-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();

        let mut esc = PhoneEscalator::new(
            home.clone(),
            NotificationDispatcher::new(),
            rt.handle().clone(),
            Uuid::new_v4(),
            "default",
            Duration::from_millis(20),
            Duration::from_millis(60),
            ApprovalDecision::Deny,
        );
        let d = esc.await_decision("nobody-replies", "Bash", RiskLevel::ExecuteHigh);
        assert_eq!(d, ApprovalDecision::Deny);
        assert!(
            crate::pending_approvals::list_pending(&home).unwrap().is_empty(),
            "pending card must be removed on the timeout return path"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn phone_escalator_alert_detects_run_level_stop_at_later_event() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let home = std::env::temp_dir().join(format!("gr-esc-runstop-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let run = Uuid::new_v4();
        // Operator replied "STOP <run>" referencing the RUN (an earlier event), not
        // this later action's approval_id.
        inbox::write_message(&home, "phone", &format!("STOP {run}"), None).unwrap();

        let mut esc = PhoneEscalator::new(
            home.clone(),
            NotificationDispatcher::new(),
            rt.handle().clone(),
            run,
            "default",
            Duration::from_millis(20),
            Duration::from_secs(1),
            ApprovalDecision::Deny,
        );
        // A DIFFERENT approval_id (a later observed action) still sees the run STOP.
        assert!(esc.alert_observed("a-later-unrelated-approval", "Bash", RiskLevel::ExecuteHigh));
        let _ = std::fs::remove_dir_all(&home);
    }
}
