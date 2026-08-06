//! Execution contracts + deny-until-approved approval ledger (sprint MVP T7/T8).
//!
//! The differentiator of spectyn's "safe unattended runs" (apex ④) is that you
//! approve an **exact execution contract** — *this* command, in *this* cwd, at
//! *this* risk level — not a blank cheque for an agent to improvise. This module
//! is the pure type + state-machine layer:
//!
//!   * [`RiskLevel`] — orders actions low→high; high-risk requires approval.
//!   * [`ExecutionContract`] — the exact command/cwd/files/risk, with an
//!     ASCII approval-card [`ExecutionContract::render`] (the Telegram / future
//!     PWA card shape) and an expiry.
//!   * [`ContractState`] / [`ApprovalDecision`] / [`apply`] — a deny-until-
//!     approved state machine: a high-risk contract is NEVER runnable until an
//!     explicit affirmative decision moves it out of `Pending`.
//!   * [`ApprovalLedger`] — append-only record of decisions (in-memory for now;
//!     a durable backing lands when this wires into the task store).
//!
//! Transport (Telegram bot, iPhone) and the live governor wiring are deliberately
//! NOT here — this layer is unit-testable in isolation and carries no I/O.

use serde::{Deserialize, Serialize};

/// Milliseconds since the Unix epoch (same idiom as `tasks::events`).
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Risk class of a contracted action. `Ord` runs low → high so callers can
/// compare/threshold (`risk >= RiskLevel::ExecuteHigh`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RiskLevel {
    /// Pure reads — list/show/inspect. Never needs approval.
    ReadOnly,
    /// Low-impact, reversible execution (e.g. `cargo test`). Auto-allowed.
    ExecuteLow,
    /// High-impact execution (arbitrary shell, installs). Needs approval.
    ExecuteHigh,
    /// Filesystem writes / patches / commits. Needs approval.
    Write,
    /// Outbound network actions. Needs approval.
    Network,
}

impl RiskLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            RiskLevel::ReadOnly => "read_only",
            RiskLevel::ExecuteLow => "execute_low",
            RiskLevel::ExecuteHigh => "execute_high",
            RiskLevel::Write => "write",
            RiskLevel::Network => "network",
        }
    }

    // inherent from_str: returns Option (not Result), so it can't be FromStr; callers depend on the Option form.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "read_only" | "readonly" => Some(RiskLevel::ReadOnly),
            "execute_low" | "executelow" => Some(RiskLevel::ExecuteLow),
            "execute_high" | "executehigh" => Some(RiskLevel::ExecuteHigh),
            "write" => Some(RiskLevel::Write),
            "network" => Some(RiskLevel::Network),
            _ => None,
        }
    }

    /// Whether an action at this risk level must be explicitly approved before it
    /// may run. ReadOnly + ExecuteLow are auto-allowed; everything higher is
    /// deny-until-approved.
    pub fn requires_approval(&self) -> bool {
        *self >= RiskLevel::ExecuteHigh
    }
}

/// The exact, approvable unit of work. What you approve is THIS contract — not
/// the agent's freedom to act.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionContract {
    pub id: String,
    pub node: String,
    pub agent: String,
    /// Coarse action verb, e.g. `shell.run`, `file.write`, `git.commit`.
    pub action: String,
    pub command: String,
    pub cwd: String,
    pub files_touched: Vec<String>,
    pub risk: RiskLevel,
    pub reason: String,
    pub created_ms: i64,
    pub expires_ms: i64,
}

impl ExecutionContract {
    /// Build a contract that expires `ttl_secs` from now. `id` is a fresh v4 UUID.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        node: impl Into<String>,
        agent: impl Into<String>,
        action: impl Into<String>,
        command: impl Into<String>,
        cwd: impl Into<String>,
        files_touched: Vec<String>,
        risk: RiskLevel,
        reason: impl Into<String>,
        ttl_secs: i64,
    ) -> Self {
        let created_ms = now_ms();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            node: node.into(),
            agent: agent.into(),
            action: action.into(),
            command: command.into(),
            cwd: cwd.into(),
            files_touched,
            risk,
            reason: reason.into(),
            created_ms,
            expires_ms: created_ms + ttl_secs * 1000,
        }
    }

    pub fn is_expired(&self, now: i64) -> bool {
        now >= self.expires_ms
    }

    /// The ASCII approval-card text routed to the phone (Telegram now, PWA later).
    /// ASCII only (I7: CP950 / PowerShell 5.1 consoles).
    pub fn render(&self, now: i64) -> String {
        let files = if self.files_touched.is_empty() {
            "none".to_string()
        } else {
            self.files_touched.join(", ")
        };
        let mins_left = ((self.expires_ms - now).max(0) + 59_000) / 60_000;
        format!(
            "[spectyn-mesh approval]\n\
             Task: {id}\n\
             Node: {node}\n\
             Agent: {agent}\n\
             Action: {action}\n\
             Command: {command}\n\
             cwd: {cwd}\n\
             Files touched: {files}\n\
             Risk: {risk}\n\
             Reason: {reason}\n\
             Expires in: {mins} min",
            id = self.id,
            node = self.node,
            agent = self.agent,
            action = self.action,
            command = self.command,
            cwd = self.cwd,
            files = files,
            risk = self.risk.as_str(),
            reason = self.reason,
            mins = mins_left,
        )
    }
}

/// The operator's decision on a pending contract (the approval-card buttons).
///
/// NOTE: this enum is intentionally NOT `Copy` — the apex-④ phone `Redirect`
/// variant carries the operator's new instruction (`String`), which is not a
/// `Copy` payload. Call sites that previously relied on copy-out semantics use
/// `.clone()` instead (the value is small and decisions are infrequent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalDecision {
    /// Approve just this one execution.
    ApproveOnce,
    /// Approve this and the rest of the task (cache the decision).
    ApproveTask,
    /// Downgrade to a dry-run (allowed, side-effect-free).
    DryRun,
    Deny,
    /// Deny AND cancel the whole task.
    Cancel,
    /// Apex ④ PHONE REDIRECT: the operator does NOT approve the pending action —
    /// instead they steer the run with a new instruction. For the CURRENT pending
    /// action this behaves exactly like a deny/cancel (the pending tool is NOT
    /// run); the carried `String` is the new instruction the caller re-dispatches
    /// with.
    Redirect(String),
}

impl ApprovalDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApprovalDecision::ApproveOnce => "approve_once",
            ApprovalDecision::ApproveTask => "approve_task",
            ApprovalDecision::DryRun => "dry_run",
            ApprovalDecision::Deny => "deny",
            ApprovalDecision::Cancel => "cancel",
            ApprovalDecision::Redirect(_) => "redirect",
        }
    }

    // inherent from_str: returns Option (not Result), so it can't be FromStr; callers depend on the Option form.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        let trimmed = s.trim();
        // A `redirect <new instruction>` reply carries a payload, so it can't be
        // matched as a fixed keyword — strip the leading (case-insensitive)
        // `redirect` keyword and keep the rest verbatim (original case) as the new
        // instruction. Accept ANY ASCII whitespace as the separator (a phone reply
        // may wrap, e.g. `redirect\n<instruction>`). `to_ascii_lowercase` only maps
        // ASCII A-Z, so `redirect` is always exactly 8 leading bytes here — byte
        // index 8 is a valid char boundary whenever `lower` starts with it.
        let lower = trimmed.to_ascii_lowercase();
        if lower == "redirect" {
            // Bare "redirect" with no instruction is not actionable.
            return None;
        }
        if let Some(rest) = lower.strip_prefix("redirect") {
            if rest.starts_with(|c: char| c.is_ascii_whitespace()) {
                // Keep the operator's original wording/case for the instruction.
                let instruction = trimmed["redirect".len()..].trim().to_string();
                if instruction.is_empty() {
                    return None;
                }
                return Some(ApprovalDecision::Redirect(instruction));
            }
        }
        match lower.as_str() {
            "approve_once" | "approve" | "yes" => Some(ApprovalDecision::ApproveOnce),
            "approve_task" | "approve_all" => Some(ApprovalDecision::ApproveTask),
            "dry_run" | "dryrun" => Some(ApprovalDecision::DryRun),
            "deny" | "no" => Some(ApprovalDecision::Deny),
            // `cancel`/`stop` are the strong refusal: the phone "Stop" button sends
            // `stop`. For a pre-action gate this resolves the pending tool as
            // Cancelled (gate => Deny), a stronger refusal than a plain `deny`.
            "cancel" | "stop" => Some(ApprovalDecision::Cancel),
            _ => None,
        }
    }
}

/// State of a contract under the deny-until-approved policy. A high-risk action
/// is runnable ONLY in `Approved`; `Pending` (the default) is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContractState {
    Pending,
    Approved,
    Denied,
    Cancelled,
    Expired,
}

impl ContractState {
    /// A terminal state never transitions again.
    pub fn is_terminal(&self) -> bool {
        !matches!(self, ContractState::Pending)
    }
}

/// Apply a decision to a state. Decisions on a terminal state are a no-op (return
/// the same state) — the ledger still records the attempt, but the outcome can't
/// be overturned.
pub fn apply(state: ContractState, decision: ApprovalDecision) -> ContractState {
    if state.is_terminal() {
        return state;
    }
    match decision {
        // DryRun is an allowed downgrade — the side-effect-free variant runs.
        ApprovalDecision::ApproveOnce
        | ApprovalDecision::ApproveTask
        | ApprovalDecision::DryRun => ContractState::Approved,
        // A Redirect does NOT approve the pending action — for THIS contract it is a
        // denial (the pending tool is not run); the new instruction is handled by
        // the run loop, which re-dispatches. Same ContractState as a plain Deny.
        ApprovalDecision::Deny | ApprovalDecision::Redirect(_) => ContractState::Denied,
        ApprovalDecision::Cancel => ContractState::Cancelled,
    }
}

/// Resolve the effective state of a contract: a still-`Pending` contract past its
/// expiry is `Expired` (and therefore not runnable).
pub fn resolve(contract: &ExecutionContract, state: ContractState, now: i64) -> ContractState {
    if state == ContractState::Pending && contract.is_expired(now) {
        ContractState::Expired
    } else {
        state
    }
}

/// One append-only entry recording a decision and the resulting state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub contract_id: String,
    pub decision: ApprovalDecision,
    pub at_ms: i64,
    pub state_after: ContractState,
}

/// Append-only ledger of approval decisions. In-memory for now; a durable
/// backing (the task store's append-only event table) lands when this wires in.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ApprovalLedger {
    entries: Vec<LedgerEntry>,
}

impl ApprovalLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a decision. Returns the assigned index (append order).
    pub fn record(
        &mut self,
        contract_id: impl Into<String>,
        decision: ApprovalDecision,
        at_ms: i64,
        state_after: ContractState,
    ) -> usize {
        self.entries.push(LedgerEntry {
            contract_id: contract_id.into(),
            decision,
            at_ms,
            state_after,
        });
        self.entries.len() - 1
    }

    pub fn entries_for(&self, contract_id: &str) -> Vec<&LedgerEntry> {
        self.entries
            .iter()
            .filter(|e| e.contract_id == contract_id)
            .collect()
    }

    /// The most recently recorded state for a contract, if any.
    pub fn latest_state(&self, contract_id: &str) -> Option<ContractState> {
        self.entries
            .iter()
            .rev()
            .find(|e| e.contract_id == contract_id)
            .map(|e| e.state_after)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract(risk: RiskLevel, ttl_secs: i64) -> ExecutionContract {
        ExecutionContract::new(
            "z13-heavy",
            "Claude Code",
            "shell.run",
            "cargo test --workspace",
            "~/projects/spectyn-mesh/core",
            vec![],
            risk,
            "verify FlightRecorder patch",
            ttl_secs,
        )
    }

    #[test]
    fn requires_approval_thresholds_at_execute_high() {
        assert!(!RiskLevel::ReadOnly.requires_approval());
        assert!(!RiskLevel::ExecuteLow.requires_approval());
        assert!(RiskLevel::ExecuteHigh.requires_approval());
        assert!(RiskLevel::Write.requires_approval());
        assert!(RiskLevel::Network.requires_approval());
    }

    #[test]
    fn risk_orders_low_to_high() {
        assert!(RiskLevel::ReadOnly < RiskLevel::ExecuteLow);
        assert!(RiskLevel::ExecuteLow < RiskLevel::ExecuteHigh);
        assert!(RiskLevel::ExecuteHigh < RiskLevel::Write);
        assert!(RiskLevel::Write < RiskLevel::Network);
    }

    #[test]
    fn risk_str_round_trips() {
        for r in [
            RiskLevel::ReadOnly,
            RiskLevel::ExecuteLow,
            RiskLevel::ExecuteHigh,
            RiskLevel::Write,
            RiskLevel::Network,
        ] {
            assert_eq!(RiskLevel::from_str(r.as_str()), Some(r));
        }
    }

    #[test]
    fn high_risk_pending_is_not_approved_by_default() {
        // Deny-until-approved: a fresh high-risk contract sits in Pending, which
        // is NOT a runnable (Approved) state.
        let c = contract(RiskLevel::ExecuteHigh, 600);
        let st = ContractState::Pending;
        assert_ne!(resolve(&c, st, c.created_ms), ContractState::Approved);
        assert!(c.risk.requires_approval());
    }

    #[test]
    fn apply_transitions_from_pending() {
        use ApprovalDecision::*;
        let p = ContractState::Pending;
        assert_eq!(apply(p, ApproveOnce), ContractState::Approved);
        assert_eq!(apply(p, ApproveTask), ContractState::Approved);
        assert_eq!(apply(p, DryRun), ContractState::Approved);
        assert_eq!(apply(p, Deny), ContractState::Denied);
        assert_eq!(apply(p, Cancel), ContractState::Cancelled);
    }

    #[test]
    fn terminal_states_are_no_ops() {
        // Once denied, an approve cannot overturn it.
        assert_eq!(
            apply(ContractState::Denied, ApprovalDecision::ApproveOnce),
            ContractState::Denied
        );
        assert_eq!(
            apply(ContractState::Approved, ApprovalDecision::Deny),
            ContractState::Approved
        );
        assert_eq!(
            apply(ContractState::Cancelled, ApprovalDecision::ApproveTask),
            ContractState::Cancelled
        );
        // Expired is terminal too — an approve after expiry can't revive it.
        assert!(ContractState::Expired.is_terminal());
        assert_eq!(
            apply(ContractState::Expired, ApprovalDecision::ApproveOnce),
            ContractState::Expired
        );
    }

    #[test]
    fn pending_past_expiry_resolves_expired() {
        let c = contract(RiskLevel::Write, 600);
        // now well past expiry while still Pending
        assert_eq!(
            resolve(&c, ContractState::Pending, c.expires_ms + 1),
            ContractState::Expired
        );
        // an already-approved contract is not retro-expired
        assert_eq!(
            resolve(&c, ContractState::Approved, c.expires_ms + 1),
            ContractState::Approved
        );
        assert!(c.is_expired(c.expires_ms));
        assert!(!c.is_expired(c.created_ms));
    }

    #[test]
    fn render_carries_exact_fields() {
        let c = contract(RiskLevel::ExecuteLow, 600);
        let card = c.render(c.created_ms);
        for needle in [
            "[spectyn-mesh approval]",
            "Node: z13-heavy",
            "Agent: Claude Code",
            "Action: shell.run",
            "Command: cargo test --workspace",
            "Risk: execute_low",
            "Files touched: none",
            "Expires in: 10 min",
        ] {
            assert!(card.contains(needle), "card missing {needle:?}:\n{card}");
        }
        assert!(card.is_ascii(), "approval card must be ASCII (I7)");
    }

    #[test]
    fn decision_str_round_trips() {
        for d in [
            ApprovalDecision::ApproveOnce,
            ApprovalDecision::ApproveTask,
            ApprovalDecision::DryRun,
            ApprovalDecision::Deny,
            ApprovalDecision::Cancel,
        ] {
            assert_eq!(ApprovalDecision::from_str(d.as_str()), Some(d));
        }
        // The phone "Stop" button sends `stop` — it must resolve to a strong
        // refusal (Cancel), not be ignored (which would hang to the timeout).
        assert_eq!(ApprovalDecision::from_str("stop"), Some(ApprovalDecision::Cancel));
        assert_eq!(ApprovalDecision::from_str("STOP"), Some(ApprovalDecision::Cancel));
    }

    #[test]
    fn parse_redirect_strips_keyword_and_keeps_instruction() {
        // Apex ④ PHONE REDIRECT: `redirect <instruction>` → Redirect("<instruction>").
        assert_eq!(
            ApprovalDecision::from_str("redirect do X instead"),
            Some(ApprovalDecision::Redirect("do X instead".to_string()))
        );
        assert_eq!(
            ApprovalDecision::from_str("redirect foo"),
            Some(ApprovalDecision::Redirect("foo".to_string()))
        );
        // The keyword match is case-insensitive; the instruction keeps its case.
        assert_eq!(
            ApprovalDecision::from_str("REDIRECT Build The Other Thing"),
            Some(ApprovalDecision::Redirect("Build The Other Thing".to_string()))
        );
        // Leading/trailing whitespace around the whole reply is trimmed.
        assert_eq!(
            ApprovalDecision::from_str("  redirect   tidy up  "),
            Some(ApprovalDecision::Redirect("tidy up".to_string()))
        );
        // Bare "redirect" with no instruction is not actionable.
        assert_eq!(ApprovalDecision::from_str("redirect"), None);
        assert_eq!(ApprovalDecision::from_str("redirect   "), None);
        // Any ASCII whitespace separates the keyword from the instruction — a phone
        // reply may wrap the instruction onto the next line or use a tab.
        assert_eq!(
            ApprovalDecision::from_str("redirect\ndo X instead"),
            Some(ApprovalDecision::Redirect("do X instead".to_string()))
        );
        assert_eq!(
            ApprovalDecision::from_str("redirect\tfoo"),
            Some(ApprovalDecision::Redirect("foo".to_string()))
        );
        // No separator at all is NOT a redirect (falls through to the keyword table).
        assert_eq!(ApprovalDecision::from_str("redirectfoo"), None);
        // `as_str` is stable for the variant (payload is not part of the tag).
        assert_eq!(ApprovalDecision::Redirect("x".into()).as_str(), "redirect");
    }

    #[test]
    fn apply_redirect_denies_the_current_action() {
        // A Redirect does NOT approve the pending tool — for THIS contract it is a
        // denial (identical ContractState to a plain Deny), so the gate denies it.
        let p = ContractState::Pending;
        assert_eq!(
            apply(p, ApprovalDecision::Redirect("new goal".into())),
            ContractState::Denied,
        );
        assert_eq!(
            apply(p, ApprovalDecision::Redirect("new goal".into())),
            apply(p, ApprovalDecision::Deny),
            "redirect yields the same ContractState as a deny for the pending action",
        );
        // On a terminal state a redirect (like any decision) is a no-op.
        assert_eq!(
            apply(ContractState::Approved, ApprovalDecision::Redirect("x".into())),
            ContractState::Approved,
        );
    }

    #[test]
    fn ledger_records_and_reports_latest_state() {
        let mut l = ApprovalLedger::new();
        assert!(l.is_empty());
        l.record("c1", ApprovalDecision::Deny, 1, ContractState::Denied);
        l.record("c2", ApprovalDecision::ApproveOnce, 2, ContractState::Approved);
        // a later attempt on c1 (no-op state) is still recorded append-only
        l.record("c1", ApprovalDecision::ApproveOnce, 3, ContractState::Denied);
        assert_eq!(l.len(), 3);
        assert_eq!(l.entries_for("c1").len(), 2);
        assert_eq!(l.latest_state("c1"), Some(ContractState::Denied));
        assert_eq!(l.latest_state("c2"), Some(ContractState::Approved));
        assert_eq!(l.latest_state("nope"), None);
    }
}
