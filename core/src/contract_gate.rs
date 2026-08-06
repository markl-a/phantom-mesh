//! Process-wide ExecutionContract gate — the agent-loop injection of the
//! deny-until-approved policy (sprint MVP T7).
//!
//! **OPT-IN, default OFF.** Enabled only by `SPECTYN_CONTRACT_GATE=1`. When
//! disabled, [`check`] is a byte-identical pass-through (`Ok(())`), so the live
//! `tool_gate` chokepoint behaves exactly as before — this is the safety
//! guarantee for shipping it inert.
//!
//! ## Why a sync snapshot (no async in the gate)
//! The live tool gate ([`crate::tool_gate`]) is a SYNC closure run inside
//! [`crate::tools::execute`]; the durable approval ledger
//! ([`crate::tasks::approvals`]) is ASYNC (sqlite). Calling async from the sync
//! gate would panic inside the runtime. So the gate consults a process-global
//! **approved-fingerprint snapshot** ([`approved`]) that the runner loads from
//! the durable ledger BEFORE the agent loop (async, no friction). High-risk
//! tool calls whose fingerprint is not yet approved are blocked and recorded in
//! [`pending`] for the runner to raise durably; the operator approves via
//! `spectyn task approve`, the runner reloads the snapshot, and the re-run is
//! allowed. No async, no `block_on`, no panic.

use crate::execution_contract::{ExecutionContract, RiskLevel};
use crate::tasks::events::EventStore;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

/// Whether the contract gate is engaged. Default OFF.
pub fn enabled() -> bool {
    std::env::var("SPECTYN_CONTRACT_GATE").as_deref() == Ok("1")
}

/// Process-global set of APPROVED action fingerprints (= contract ids). The
/// runner populates this from the durable approval ledger ([`load_approved`])
/// before executing an agent loop.
pub fn approved() -> &'static Mutex<HashSet<String>> {
    static A: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    A.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Process-global map of contracts the gate BLOCKED, keyed by fingerprint
/// (= the contract id). The runner drains this after the loop ([`flush_pending`])
/// to durably raise the contracts so `spectyn task approvals` can show them.
pub fn pending() -> &'static Mutex<HashMap<String, ExecutionContract>> {
    static P: OnceLock<Mutex<HashMap<String, ExecutionContract>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Build the ExecutionContract for a blocked tool call. The id is the stable
/// fingerprint so it matches across the gate snapshot, the durable ledger, and
/// `spectyn task approve`. ASCII-only command summary (I7).
fn build_contract(id: String, name: &str, args: &serde_json::Value, risk: RiskLevel) -> ExecutionContract {
    let raw = serde_json::to_string(args).unwrap_or_default();
    let command = if raw.chars().count() > 200 {
        format!("{}...", raw.chars().take(200).collect::<String>())
    } else {
        raw
    };
    let now = now_ms();
    ExecutionContract {
        id,
        node: std::env::var("SPECTYN_NODE").unwrap_or_else(|_| "local".to_string()),
        agent: "agent".to_string(),
        action: name.to_string(),
        command,
        cwd: std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        files_touched: vec![],
        risk,
        reason: format!("tool call: {name}"),
        created_ms: now,
        expires_ms: now + 600_000,
    }
}

/// Load the durably-approved contract ids for `task_id` into the sync
/// [`approved`] snapshot. The runner calls this (async, no friction) BEFORE an
/// agent loop so already-approved actions run without re-blocking.
pub async fn load_approved(events: &EventStore, task_id: Uuid) -> anyhow::Result<usize> {
    let ids = crate::tasks::approvals::approved_for(events, task_id).await?;
    let n = ids.len();
    if let Ok(mut a) = approved().lock() {
        a.extend(ids);
    }
    Ok(n)
}

/// Durably raise every contract the gate blocked this run (drains [`pending`]),
/// skipping ones already in the task's ledger. The runner calls this AFTER an
/// agent loop so `spectyn task approvals <id>` shows what needs the operator.
pub async fn flush_pending(events: &EventStore, task_id: Uuid) -> anyhow::Result<usize> {
    let drained: Vec<ExecutionContract> = {
        match pending().lock() {
            Ok(mut p) => p.drain().map(|(_, c)| c).collect(),
            Err(_) => return Ok(0),
        }
    };
    let mut raised = 0;
    for c in &drained {
        // Skip if already recorded (idempotent across runs).
        if crate::tasks::approvals::latest_state(events, task_id, &c.id)
            .await?
            .is_none()
        {
            crate::tasks::approvals::record_request(events, task_id, c).await?;
            raised += 1;
        }
    }
    Ok(raised)
}

/// Stable fingerprint of a tool call — reuses the existing key-order-invariant
/// tool-approval-cache hash, so the same action always maps to the same
/// contract id (an approval persists across re-runs).
pub fn fingerprint(name: &str, args: &serde_json::Value) -> String {
    crate::approval::ToolApprovalCache::fingerprint(name, args)
}

/// The gate decision for a tool call.
///
/// `Ok(())` when: the gate is disabled (pass-through), the tool is low-risk
/// (auto-allowed), or its fingerprint has been approved. `Err(reason)` when a
/// high-risk action is not yet approved — the reason tells the operator the
/// exact `spectyn task approve` command. Blocked fingerprints are added to
/// [`pending`].
pub fn check(name: &str, args: &serde_json::Value) -> Result<(), String> {
    if !enabled() {
        return Ok(());
    }
    let risk = crate::tasks::approvals::classify_tool(name, args);
    if !risk.requires_approval() {
        return Ok(());
    }
    let fp = fingerprint(name, args);
    if approved()
        .lock()
        .map(|s| s.contains(&fp))
        .unwrap_or(false)
    {
        return Ok(());
    }
    if let Ok(mut p) = pending().lock() {
        p.entry(fp.clone())
            .or_insert_with(|| build_contract(fp.clone(), name, args, risk));
    }
    Err(format!(
        "blocked: '{name}' is {} and needs approval (contract {fp}). \
         Approve with `spectyn task approve <task-id> {fp}`, then re-run.",
        risk.as_str()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Serialise: these mutate the process-global SPECTYN_CONTRACT_GATE env + the
    // approved/pending snapshots.
    fn with_gate<T>(on: bool, f: impl FnOnce() -> T) -> T {
        let _g = crate::env_lock::acquire();
        let saved = std::env::var_os("SPECTYN_CONTRACT_GATE");
        if on {
            std::env::set_var("SPECTYN_CONTRACT_GATE", "1");
        } else {
            std::env::remove_var("SPECTYN_CONTRACT_GATE");
        }
        approved().lock().unwrap().clear();
        pending().lock().unwrap().clear();
        let out = f();
        match saved {
            Some(v) => std::env::set_var("SPECTYN_CONTRACT_GATE", v),
            None => std::env::remove_var("SPECTYN_CONTRACT_GATE"),
        }
        out
    }

    #[test]
    fn disabled_is_passthrough_for_everything() {
        with_gate(false, || {
            // Even a high-risk tool is allowed when the gate is off.
            assert!(check("bash", &json!({"cmd": "rm -rf /"})).is_ok());
            assert!(check("file_write", &json!({"path": "x"})).is_ok());
            // and nothing is recorded as pending.
            assert!(pending().lock().unwrap().is_empty());
        });
    }

    #[test]
    fn enabled_blocks_high_risk_until_approved() {
        with_gate(true, || {
            let args = json!({"cmd": "cargo test"});
            // first: blocked + recorded pending.
            let err = check("bash", &args).unwrap_err();
            assert!(err.contains("needs approval"));
            let fp = fingerprint("bash", &args);
            assert!(pending().lock().unwrap().contains_key(&fp));
            // operator approves (runner loads the fingerprint into `approved`).
            approved().lock().unwrap().insert(fp.clone());
            assert!(check("bash", &args).is_ok());
        });
    }

    #[tokio::test]
    async fn full_round_trip_block_flush_approve_load_allow() {
        // The end-to-end live-wiring: gate blocks a high-risk tool → runner
        // flushes the contract to the durable ledger → operator approves it
        // there → runner loads the approved snapshot → gate now allows. Proven
        // without running a live agent.
        use crate::execution_contract::{ApprovalDecision, ContractState};
        use crate::tasks::store::TaskStore;
        let _g = crate::env_lock::acquire();
        let saved = std::env::var_os("SPECTYN_CONTRACT_GATE");
        std::env::set_var("SPECTYN_CONTRACT_GATE", "1");
        approved().lock().unwrap().clear();
        pending().lock().unwrap().clear();

        let store = TaskStore::open_at(std::path::PathBuf::from(":memory:")).unwrap();
        let events = EventStore::from_conn(store.conn());
        let task = Uuid::new_v4();
        let args = serde_json::json!({"cmd": "cargo test --workspace"});

        // 1) gate blocks + records the contract in pending.
        let err = check("bash", &args).unwrap_err();
        assert!(err.contains("needs approval"));
        let fp = fingerprint("bash", &args);

        // 2) runner flushes pending → durable ledger (one contract raised).
        assert_eq!(flush_pending(&events, task).await.unwrap(), 1);
        assert_eq!(
            crate::tasks::approvals::latest_state(&events, task, &fp).await.unwrap(),
            Some(ContractState::Pending)
        );

        // 3) operator approves it in the durable ledger.
        crate::tasks::approvals::record_decision(
            &events, task, &fp, ApprovalDecision::ApproveOnce, ContractState::Approved,
        )
        .await
        .unwrap();

        // 4) runner loads approved snapshot → gate now ALLOWS the same action.
        assert_eq!(load_approved(&events, task).await.unwrap(), 1);
        assert!(check("bash", &args).is_ok(), "approved action must pass the gate");

        // cleanup
        approved().lock().unwrap().clear();
        pending().lock().unwrap().clear();
        match saved {
            Some(v) => std::env::set_var("SPECTYN_CONTRACT_GATE", v),
            None => std::env::remove_var("SPECTYN_CONTRACT_GATE"),
        }
    }

    #[test]
    fn enabled_auto_allows_low_risk() {
        with_gate(true, || {
            assert!(check("file_read", &json!({"path": "x"})).is_ok());
            assert!(check("glob_search", &json!({"pattern": "*.rs"})).is_ok());
            // reads never get recorded as pending.
            assert!(pending().lock().unwrap().is_empty());
        });
    }
}
