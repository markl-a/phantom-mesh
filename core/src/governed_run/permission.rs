//! Task 6: claude TRUE pre-action gate. claude is spawned headless with a
//! `PreToolUse` HOOK (matcher `*`); claude calls the hook BEFORE running each tool
//! and BLOCKS on its reply, so a risky action is paused BEFORE it runs.
//!
//! ⚠️ The hook — NOT `--permission-prompt-tool` — is the working mechanism on the
//! installed claude (2.1.x): the `--permission-prompt-tool` flag exists but its
//! trigger conditions are undocumented and it did NOT fire in a live z13 test
//! (a non-allowlisted Bash ran without the tool being called). The PreToolUse hook
//! IS documented and verified to block/allow + to wait while the hook deliberates.
//!
//! The hook contract (captured live, claude 2.1.170):
//!   STDIN : {"tool_name":"Bash","tool_input":{...},"session_id":..,"cwd":..,
//!            "tool_use_id":..,"hook_event_name":"PreToolUse", ...}
//!   STDOUT: {"hookSpecificOutput":{"hookEventName":"PreToolUse",
//!            "permissionDecision":"allow"|"deny","permissionDecisionReason":".."}}  exit 0
//!
//! `decide_core` is the shared decision (classify → escalate-on-high-risk → gate);
//! `decide_pretooluse_hook` maps it to the hook STDOUT shape. The legacy MCP
//! `decide_permission` (`{"behavior":..}` shape) is kept as a pure function for the
//! `--permission-prompt-tool` path should a future claude honor it, but the hook
//! path is what the governed claude spawn actually wires.

use crate::execution_contract::{ContractState, ExecutionContract, apply};
use crate::governed_run::escalation::Escalator;
use crate::tasks::approvals::{GateOutcome, classify_tool, gate};
use serde_json::{Value, json};

/// The protocol-neutral outcome of the governor's pre-action decision, so the MCP
/// and the PreToolUse-hook entry points share ONE classify→escalate→gate core.
pub(crate) enum GateDecision {
    /// The tool may run (low-risk auto-allow, or operator approved).
    Allow,
    /// The tool is blocked; the string is the operator-facing reason.
    Deny(String),
}

/// Shared core: classify `tool_name(input)`; low-risk auto-allows, high-risk awaits
/// the operator's decision via `escalator` (reuses the governor's `classify_tool` +
/// `apply` + `gate`). The escalator is injected: `PhoneEscalator` in production,
/// `MockEscalator` in tests. Protocol-agnostic — callers map `GateDecision` to their
/// wire shape (MCP PermissionResult or PreToolUse hookSpecificOutput).
pub(crate) fn decide_core(
    tool_name: &str,
    input: &Value,
    escalator: &mut dyn Escalator,
) -> GateDecision {
    let risk = classify_tool(tool_name, input);
    if !risk.requires_approval() {
        return GateDecision::Allow; // ReadOnly / ExecuteLow → auto-allow, no escalation
    }
    let contract = ExecutionContract::new(
        "local",
        "claude",
        "tool.call",
        tool_name,
        ".",
        vec![],
        risk,
        "claude pre-action gate",
        300,
    );
    let decision = escalator.await_decision(&contract.id, tool_name, risk);
    match gate(apply(ContractState::Pending, decision)) {
        GateOutcome::Allow => GateDecision::Allow,
        _ => GateDecision::Deny(format!("operator denied tool '{tool_name}'")),
    }
}

/// claude's permission-prompt-tool "allow" reply (optionally with modified input).
pub fn allow(input: &Value) -> Value {
    json!({ "behavior": "allow", "updatedInput": input })
}
/// claude's permission-prompt-tool "deny" reply with a human reason.
pub fn deny(message: impl AsRef<str>) -> Value {
    json!({ "behavior": "deny", "message": message.as_ref() })
}

/// Legacy MCP `--permission-prompt-tool` shape. Kept as a pure function; the wired
/// path is the PreToolUse hook (`decide_pretooluse_hook`).
pub fn decide_permission(tool_name: &str, input: &Value, escalator: &mut dyn Escalator) -> Value {
    match decide_core(tool_name, input, escalator) {
        GateDecision::Allow => allow(input),
        GateDecision::Deny(reason) => deny(reason),
    }
}

/// A PreToolUse-hook "allow" reply: the tool may run (no input rewrite).
pub fn hook_allow(reason: impl AsRef<str>) -> Value {
    json!({ "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "allow",
        "permissionDecisionReason": reason.as_ref(),
    }})
}
/// A PreToolUse-hook "deny" reply: the tool is blocked BEFORE it runs.
pub fn hook_deny(reason: impl AsRef<str>) -> Value {
    json!({ "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": reason.as_ref(),
    }})
}

/// Decide a claude `PreToolUse` hook call. `hook_input` is the JSON claude writes to
/// the hook's stdin (`{"tool_name":..,"tool_input":{..}, ..}`); the return is the
/// `hookSpecificOutput` JSON the hook must print. A malformed call (no `tool_name`)
/// is a fail-safe DENY. This is the genuine apex-④ pre-action gate for claude.
pub fn decide_pretooluse_hook(hook_input: &Value, escalator: &mut dyn Escalator) -> Value {
    let tool_name = hook_input
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if tool_name.is_empty() {
        return hook_deny("spectyn governor: missing tool_name (fail-safe deny)");
    }
    let tool_input = hook_input
        .get("tool_input")
        .cloned()
        .unwrap_or_else(|| json!({}));
    match decide_core(tool_name, &tool_input, escalator) {
        GateDecision::Allow => hook_allow(format!("spectyn governor: '{tool_name}' allowed")),
        GateDecision::Deny(reason) => hook_deny(reason),
    }
}

/// Production entry for the MCP `permission_request` tool: build the real phone
/// escalator (OS notification + inbox) and decide. Async; runs the sync decision
/// on a blocking thread because the escalator `block_on`-bridges async I/O (needs
/// a non-async-worker thread on the multi-thread MCP runtime).
pub async fn prod_decide_permission(tool_name: String, input: Value) -> Value {
    use crate::execution_contract::ApprovalDecision;
    use crate::governed_run::escalation::PhoneEscalator;
    use crate::notifications::NotificationDispatcher;
    use std::time::Duration;

    let home = match crate::cli_config::resolve_home_dir() {
        Ok(h) => h,
        Err(_) => return deny("spectyn home could not be resolved"),
    };
    let dispatcher = NotificationDispatcher::new();
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    dispatcher
        .add_channel(std::sync::Arc::new(
            crate::notifications::channels::OsChannel,
        ))
        .await;

    let handle = tokio::runtime::Handle::current();
    let mut escalator = PhoneEscalator::new(
        home,
        dispatcher,
        handle,
        uuid::Uuid::new_v4(),
        "default",
        Duration::from_secs(2),
        Duration::from_secs(300),
        ApprovalDecision::Deny, // fail-safe: no reply within deadline => deny
    );
    match tokio::task::spawn_blocking(move || {
        decide_permission(&tool_name, &input, &mut escalator)
    })
    .await
    {
        Ok(v) => v,
        Err(_) => deny("permission task failed"),
    }
}

/// Production entry for the `spectyn pretooluse-gate` hook subcommand: build the
/// REAL phone escalator bound to the GOVERNED RUN's identity and decide.
///
/// The escalator's `task_id` is the run's id, read from `SPECTYN_GOVERN_TASK_ID`
/// (set by the parent when it spawns the governed claude) — NOT a fresh uuid — so a
/// phone reply correlates to this run and there is exactly ONE awaiter (the hook).
/// The parent loop observes claude's stream WITHOUT a second await (the agy-#3 fix).
///
/// Fail-safe: any missing identity / unresolvable home / task panic => DENY (never
/// let a high-risk tool run because the governor could not reach the operator).
pub async fn prod_decide_pretooluse_hook(hook_input: Value) -> Value {
    use crate::execution_contract::ApprovalDecision;
    use crate::governed_run::escalation::PhoneEscalator;
    use crate::notifications::NotificationDispatcher;
    use std::time::Duration;

    let home = match crate::cli_config::resolve_home_dir() {
        Ok(h) => h,
        Err(_) => return hook_deny("spectyn governor: home unresolved (fail-safe deny)"),
    };
    let task_id = match std::env::var("SPECTYN_GOVERN_TASK_ID")
        .ok()
        .and_then(|s| uuid::Uuid::parse_str(s.trim()).ok())
    {
        Some(id) => id,
        None => return hook_deny("spectyn governor: no run identity (fail-safe deny)"),
    };

    let dispatcher = NotificationDispatcher::new();
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    dispatcher
        .add_channel(std::sync::Arc::new(
            crate::notifications::channels::OsChannel,
        ))
        .await;
    // TELEGRAM REACH: register the telegram channel the SAME way `run.rs` does, so a
    // high-risk pre-action escalation raised inside THIS hook reaches the operator's
    // PHONE — not just a desktop toast. Default-OFF: when telegram is unconfigured
    // `resolve_telegram_channel` returns None and only OsChannel is registered
    // (behavior unchanged).
    if let Some(ch) = crate::governed_run::run::resolve_telegram_channel(
        crate::config::AgentsConfig::find_and_load()
            .and_then(|c| c.telegram)
            .as_ref(),
        |name| std::env::var(name).ok(),
    ) {
        dispatcher.add_channel(ch).await;
    }

    // Deadline/poll are env-tunable (the live test uses a short deadline to exercise
    // the fail-safe deny without a 5-minute wait); default 300s deadline / 2s poll.
    let deadline = std::env::var("SPECTYN_GOVERN_DEADLINE_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(300);
    let poll = std::env::var("SPECTYN_GOVERN_POLL_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(2);
    // STRUCTURED FLIGHT-RECORDING (best-effort): classify the tool BEFORE the gate.
    // Only HIGH-RISK tools (which actually escalate) are recorded to the shared S0
    // EventStore — a low-risk auto-allow is skipped (it never reaches the operator).
    // Captured here because `home`/`hook_input` are moved into the gate below.
    let tool_name = hook_input
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let tool_input = hook_input
        .get("tool_input")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let risk = crate::tasks::approvals::classify_tool(&tool_name, &tool_input);
    let record_db_path = if !tool_name.is_empty() && risk.requires_approval() {
        Some(crate::cli_config::spectyn_dir_under(&home).join("spectyn.db"))
    } else {
        None
    };

    let handle = tokio::runtime::Handle::current();
    let mut escalator = PhoneEscalator::new(
        home.clone(),
        dispatcher,
        handle,
        task_id,
        "default",
        Duration::from_secs(poll),
        Duration::from_secs(deadline),
        ApprovalDecision::Deny, // fail-safe: no reply within deadline => deny
    );
    // apex-④ DISPATCH ↔ GOVERN correlation (the LIVE production path). The claude
    // PreToolUse hook is the ONLY production code that raises a pre-action
    // Approve/Deny (claude => PreActionDelegated; codex/opencode/agy =>
    // PostActionObserved, which alert-observes and never awaits a pre-action
    // decision). So this hook process is where a dispatched governed run's approval
    // must be correlated onto its dispatch row. `task_id` here is
    // SPECTYN_GOVERN_TASK_ID, which AFTER the D2 unification EQUALS the dispatch
    // `job_uuid` for a dispatched governed run. Open the SAME canonical spectyn.db
    // the parent (`run.rs`) opened and attach it so `await_decision` stamps the
    // approval_id + AwaitingApproval onto that row at pending-card-write time.
    //
    // SAFE TO ALWAYS ATTACH: `set_approval_id` / `update_status` UPDATE BY task_id.
    // For a STANDALONE `spectyn govern` run (no dispatch row) the task_id matches no
    // tasks row → 0 rows affected → a harmless no-op (rusqlite `execute` returns the
    // affected-row count, not an error, on 0 matches). Best-effort: a failure to
    // open the DB (e.g. a cross-process lock — `serve` may hold the same spectyn.db)
    // is swallowed; it must never change the gate decision.
    let db_path = crate::cli_config::spectyn_dir_under(&home).join("spectyn.db");
    if let Ok(store) = crate::tasks::TaskStore::open_at(db_path) {
        escalator = escalator.with_dispatch_store(store);
    }
    let decision = tokio::task::spawn_blocking(move || {
        decide_pretooluse_hook(&hook_input, &mut escalator)
    })
    .await
    .unwrap_or_else(|_| hook_deny("spectyn governor: gate task failed (fail-safe deny)"));

    // Append the governance moment (ApprovalRequested → Approved/Denied) under the
    // run's task_id. BEST-EFFORT: a sqlite open/append failure (e.g. a cross-process
    // lock — `serve` may hold the same spectyn.db) is IGNORED; it must NOT change the
    // returned decision or fail the gate.
    if let Some(db_path) = record_db_path {
        let approved = decision
            .get("hookSpecificOutput")
            .and_then(|h| h.get("permissionDecision"))
            .and_then(|d| d.as_str())
            == Some("allow");
        record_governance_best_effort(db_path, task_id, &tool_name, risk, approved).await;
    }

    decision
}

/// Best-effort: record the pre-action governance moment to the SHARED S0
/// `EventStore` under `task_id` — one `ApprovalRequested` (Pending) then one
/// `Approved`/`Denied` per the decision, mirroring `recorder.rs`'s append usage so
/// the hook's HIGH-RISK decisions are replayable alongside `spectyn govern`'s.
///
/// Every failure is swallowed: opening the DB can fail under a cross-process lock
/// (`serve` holds the same `spectyn.db`), and the append itself is fire-and-forget.
/// This MUST never change the gate's decision.
async fn record_governance_best_effort(
    db_path: std::path::PathBuf,
    task_id: uuid::Uuid,
    tool_name: &str,
    risk: crate::execution_contract::RiskLevel,
    approved: bool,
) {
    use crate::tasks::{EventStore, TaskEventKind, TaskStore};
    let store = match TaskStore::open_at(db_path) {
        Ok(s) => s,
        Err(_) => return, // cross-process lock / unreadable DB: skip silently.
    };
    let events = EventStore::from_conn(store.conn());
    let detail = serde_json::json!({
        "tool_name": tool_name,
        "risk": risk.as_str(),
        "decision": if approved { "approved" } else { "denied" },
    })
    .to_string();
    let _ = events
        .append(task_id, TaskEventKind::ApprovalRequested, Some(&detail))
        .await;
    let kind = if approved {
        TaskEventKind::Approved
    } else {
        TaskEventKind::Denied
    };
    let _ = events.append(task_id, kind, Some(&detail)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_contract::ApprovalDecision;
    use crate::governed_run::escalation::MockEscalator;

    #[test]
    fn low_risk_tool_allowed_without_escalation() {
        let mut esc = MockEscalator::default();
        let r = decide_permission("Read", &json!({"path":"x"}), &mut esc);
        assert_eq!(r["behavior"], "allow");
        assert_eq!(r["updatedInput"]["path"], "x");
        assert!(esc.sent.is_empty(), "low-risk must NOT escalate, got {:?}", esc.sent);
    }

    #[test]
    fn high_risk_approved_yields_allow() {
        let mut esc = MockEscalator::default();
        esc.force_decision = Some(ApprovalDecision::ApproveOnce);
        let r = decide_permission("Bash", &json!({"cmd":"ls"}), &mut esc);
        assert_eq!(r["behavior"], "allow");
        assert_eq!(r["updatedInput"]["cmd"], "ls");
        assert!(esc.sent.iter().any(|s| s.starts_with("await:")), "claude blocks on a decision");
    }

    #[test]
    fn high_risk_denied_yields_deny() {
        let mut esc = MockEscalator::default();
        esc.force_decision = Some(ApprovalDecision::Deny);
        let r = decide_permission("Bash", &json!({}), &mut esc);
        assert_eq!(r["behavior"], "deny");
        assert!(r["message"].as_str().unwrap().contains("Bash"));
    }

    #[test]
    fn unknown_tool_is_high_risk_and_gated() {
        // classify_tool defaults unknown -> ExecuteHigh, so an unrecognised tool
        // is gated, not silently allowed.
        let mut esc = MockEscalator::default();
        esc.force_decision = Some(ApprovalDecision::Deny);
        let r = decide_permission("frobnicate", &json!({}), &mut esc);
        assert_eq!(r["behavior"], "deny");
    }

    // ---- PreToolUse hook path (the WIRED claude pre-action gate) ----

    fn hook_in(tool: &str, input: Value) -> Value {
        json!({
            "hook_event_name": "PreToolUse",
            "tool_name": tool,
            "tool_input": input,
            "session_id": "s1",
            "cwd": ".",
            "tool_use_id": "toolu_x",
        })
    }

    #[test]
    fn hook_low_risk_allows_without_escalation() {
        let mut esc = MockEscalator::default();
        let r = decide_pretooluse_hook(&hook_in("Read", json!({"path": "x"})), &mut esc);
        assert_eq!(r["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(r["hookSpecificOutput"]["permissionDecision"], "allow");
        assert!(esc.sent.is_empty(), "low-risk must NOT escalate, got {:?}", esc.sent);
    }

    #[test]
    fn hook_high_risk_approved_allows_and_blocks_first() {
        let mut esc = MockEscalator::default();
        esc.force_decision = Some(ApprovalDecision::ApproveOnce);
        let r = decide_pretooluse_hook(&hook_in("Bash", json!({"command": "ls"})), &mut esc);
        assert_eq!(r["hookSpecificOutput"]["permissionDecision"], "allow");
        assert!(
            esc.sent.iter().any(|s| s.starts_with("await:")),
            "the hook must BLOCK on the operator decision before allowing"
        );
    }

    #[test]
    fn hook_high_risk_denied_blocks_the_tool() {
        let mut esc = MockEscalator::default();
        esc.force_decision = Some(ApprovalDecision::Deny);
        let r = decide_pretooluse_hook(&hook_in("Bash", json!({"command": "rm -rf /"})), &mut esc);
        assert_eq!(r["hookSpecificOutput"]["permissionDecision"], "deny");
        assert!(
            r["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .unwrap()
                .contains("Bash")
        );
    }

    #[test]
    fn hook_malformed_input_is_failsafe_deny() {
        // No tool_name => the governor must DENY (never fail-open).
        let mut esc = MockEscalator::default();
        let r = decide_pretooluse_hook(&json!({"hook_event_name": "PreToolUse"}), &mut esc);
        assert_eq!(r["hookSpecificOutput"]["permissionDecision"], "deny");
        assert!(esc.sent.is_empty(), "a malformed call must not even escalate");
    }

    #[test]
    fn hook_high_risk_denies_when_operator_unreachable() {
        // Apex ④ invariant ③ FAIL-SAFE DENY, end-to-end through the REAL gate: a
        // genuine PhoneEscalator whose dispatcher has NO channels (operator
        // unreachable) and an empty inbox (no reply) must DENY a high-risk tool —
        // never fail-open to allow. This exercises decide_pretooluse_hook ->
        // decide_core -> PhoneEscalator::await_decision -> timeout fallback, rather
        // than the MockEscalator used by the other hook tests. Short deadline so the
        // fallback fires fast; zero channels means no real notification is sent.
        use crate::execution_contract::ApprovalDecision;
        use crate::governed_run::escalation::PhoneEscalator;
        use crate::notifications::NotificationDispatcher;
        use std::time::Duration;
        use uuid::Uuid;

        let rt = tokio::runtime::Runtime::new().unwrap();
        let home = std::env::temp_dir().join(format!("gr-perm-failsafe-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let mut esc = PhoneEscalator::new(
            home.clone(),
            NotificationDispatcher::new(), // no channels registered => unreachable
            rt.handle().clone(),
            Uuid::new_v4(),
            "default",
            Duration::from_millis(20),
            Duration::from_millis(60),
            ApprovalDecision::Deny, // fail-safe fallback
        );
        let r = decide_pretooluse_hook(&hook_in("Bash", json!({"command": "rm -rf /"})), &mut esc);
        assert_eq!(
            r["hookSpecificOutput"]["permissionDecision"], "deny",
            "an unreachable operator must DENY a high-risk tool (fail-safe, never fail-open)"
        );
        // No pending card may linger after the timeout return path (invariant ④).
        assert!(
            crate::pending_approvals::list_pending(&home).unwrap().is_empty(),
            "the pending card is cleaned up on the fail-safe timeout path"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn hook_redirect_reply_denies_the_pending_tool() {
        // A phone REDIRECT reply is, for THIS pending tool, a deny (apply maps
        // Redirect -> Denied); the new instruction is surfaced elsewhere.
        let mut esc = MockEscalator::default();
        esc.force_decision = Some(ApprovalDecision::Redirect("do something else".into()));
        let r = decide_pretooluse_hook(&hook_in("Bash", json!({"command": "ls"})), &mut esc);
        assert_eq!(r["hookSpecificOutput"]["permissionDecision"], "deny");
    }

    // ---- STRUCTURED FLIGHT-RECORDING (the new best-effort EventStore append) ----

    // Hermetic: drive `record_governance_best_effort` against a REAL TaskStore at a
    // temp path (no env mutation, no escalator, no cross-process lock) and assert the
    // governance moment lands as ApprovalRequested -> Approved/Denied under the
    // run's task_id — exactly what the hook records for a HIGH-RISK tool.
    #[tokio::test]
    async fn high_risk_records_request_then_decision_to_eventstore() {
        use crate::execution_contract::RiskLevel;
        use crate::tasks::{EventStore, TaskEventKind, TaskStore};
        use uuid::Uuid;

        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("spectyn.db");
        let task_id = Uuid::new_v4();

        // Approved path: ApprovalRequested (Pending) then Approved.
        record_governance_best_effort(db.clone(), task_id, "Bash", RiskLevel::ExecuteHigh, true)
            .await;
        // Denied path under a second task id: ApprovalRequested then Denied.
        let denied_id = Uuid::new_v4();
        record_governance_best_effort(
            db.clone(),
            denied_id,
            "Bash",
            RiskLevel::ExecuteHigh,
            false,
        )
        .await;

        let store = TaskStore::open_at(db).unwrap();
        let events = EventStore::from_conn(store.conn());

        let approved = events.events_for(task_id).await.unwrap();
        assert_eq!(approved.len(), 2, "request + decision recorded");
        assert_eq!(approved[0].kind, TaskEventKind::ApprovalRequested);
        assert_eq!(approved[1].kind, TaskEventKind::Approved);
        let d: serde_json::Value =
            serde_json::from_str(approved[1].detail.as_deref().unwrap()).unwrap();
        assert_eq!(d["tool_name"], "Bash");
        assert_eq!(d["risk"], "execute_high");
        assert_eq!(d["decision"], "approved");

        let denied = events.events_for(denied_id).await.unwrap();
        assert_eq!(denied.len(), 2);
        assert_eq!(denied[0].kind, TaskEventKind::ApprovalRequested);
        assert_eq!(denied[1].kind, TaskEventKind::Denied);
    }

    // Best-effort: an unopenable DB path (parent dir does not exist) must be
    // swallowed silently — it must NEVER panic or surface an error to the gate.
    #[tokio::test]
    async fn unopenable_db_is_swallowed() {
        use crate::execution_contract::RiskLevel;
        use uuid::Uuid;
        let bogus = std::path::Path::new("/nonexistent-spectyn-dir-xyz/sub/spectyn.db");
        // Returns normally (no panic); the failed open is ignored.
        record_governance_best_effort(
            bogus.to_path_buf(),
            Uuid::new_v4(),
            "Bash",
            RiskLevel::ExecuteHigh,
            true,
        )
        .await;
    }
}
