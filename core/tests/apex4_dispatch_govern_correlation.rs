//! apex-④ DISPATCH ↔ GOVERN correlation gate (hermetic; the REAL PRODUCTION seam).
//!
//! Proves the keystone of the "approve a DISPATCHED task from your phone" loop on
//! the path that ACTUALLY raises a pre-action approval in production: the claude
//! `PreToolUse` hook (`prod_decide_pretooluse_hook`). claude is the only CLI whose
//! enforcement is `PreActionDelegated` — its child-side hook is the sole pre-action
//! awaiter. codex/opencode/agy are `PostActionObserved` (alert-only, no pre-action
//! decision → nothing to correlate). So the live correlation MUST happen inside the
//! hook process, and that is exactly what `prod_decide_pretooluse_hook` now wires
//! (it opens the canonical `spectyn.db` and `.with_dispatch_store(..)` on its
//! escalator).
//!
//! Why this is not fake-green (the batch-6 DEFECT bar):
//!   * The dispatch row is seeded in a REAL `TaskQueue` over a temp canonical
//!     `spectyn.db` (at `SPECTYN_HOME`) in `Running` — exactly what `serve.rs`
//!     `rpc_task_assign` does before launching the runner.
//!   * The decision is driven through the REAL PRODUCTION entry point
//!     `prod_decide_pretooluse_hook` (the claude PreToolUse-hook path — the only
//!     production path that raises a pre-action approval). It
//!     resolves the home, opens the SAME canonical `spectyn.db`, builds the REAL
//!     `PhoneEscalator` with the dispatch store attached, mints a real
//!     `ExecutionContract`, and calls the REAL `PhoneEscalator::await_decision` —
//!     the production code that stamps the dispatch row.
//!   * The contract id is read from the LIVE pending card the production escalator
//!     wrote (what production minted), then used to send the operator's reply — it
//!     is NEVER injected into the row by the test.
//!   * The assertion RE-READS the task store: `approval_id == contract.id` AND the
//!     row is `AwaitingApproval`. A `/tasks`-shape JSON (the same `json!({"tasks":
//!     tasks})` the supervisor view serializes) is checked to emit the id.
//!   * A control run with a LOW-RISK tool raises no approval, so the dispatch row's
//!     `approval_id` stays `None` (the stamp can only come from the real escalation
//!     path, not from merely attaching the store).
//!
//! TEST-ONLY — drives production code, modifies none.

use spectyn_mesh::governed_run::permission::prod_decide_pretooluse_hook;
use spectyn_mesh::tasks::{TaskQueue, TaskStatus, TaskStore};
use serde_json::json;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tempfile::tempdir;
use tokio::runtime::Builder;
use uuid::Uuid;

/// `prod_decide_pretooluse_hook` reads PROCESS env (`SPECTYN_HOME`,
/// `SPECTYN_GOVERN_TASK_ID`, deadline/poll), so the two tests that mutate it must
/// not race each other (or other integration tests in this binary).
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn hook_in(tool: &str, input: serde_json::Value) -> serde_json::Value {
    json!({
        "hook_event_name": "PreToolUse",
        "tool_name": tool,
        "tool_input": input,
        "session_id": "s1",
        "cwd": ".",
        "tool_use_id": "toolu_x",
    })
}

/// Seed a dispatch row (known `job_uuid`) into the canonical `spectyn.db` and move
/// it to `Running` — exactly what `serve.rs` `rpc_task_assign` does before the run.
fn seed_running_dispatch_row(rt: &tokio::runtime::Runtime, store: &TaskStore, job_uuid: Uuid) {
    let queue = TaskQueue::new(store.clone());
    rt.block_on(async {
        queue
            .create_with_id(job_uuid, "default", "researcher", "do the dispatched work")
            .await
            .expect("seed dispatch row");
        queue
            .transition(job_uuid, TaskStatus::Running, None)
            .await
            .expect("dispatch row -> Running");
    });
}

#[test]
fn prod_hook_approval_stamps_dispatch_row_with_contract_id_and_awaiting_approval() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    let tmp = tempdir().expect("tempdir must be created");
    // SPECTYN_HOME IS the data root: spectyn.db / pending / inbox all live under it
    // (spectyn_dir_under returns SPECTYN_HOME verbatim). This is the same data dir
    // the parent `run.rs` passes the hook as SPECTYN_HOME.
    let data_dir = tmp.path().to_path_buf();
    let db_path = data_dir.join("spectyn.db");
    let job_uuid = Uuid::new_v4();

    let rt = Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime must be created");

    // DISPATCH universe: the seeded row in the canonical spectyn.db.
    let store = TaskStore::open_at(db_path.clone()).expect("open task store");
    seed_running_dispatch_row(&rt, &store, job_uuid);

    let before = rt
        .block_on(store.get(job_uuid))
        .expect("read seeded row")
        .expect("seeded row exists");
    assert_eq!(before.status, TaskStatus::Running, "seed must start Running");
    assert_eq!(before.approval_id, None, "seed must carry no approval_id");

    // Wire the PRODUCTION hook's process env: SPECTYN_HOME = the data dir,
    // SPECTYN_GOVERN_TASK_ID = the dispatch job_uuid (after D2 the govern task_id IS
    // the dispatch id). A generous deadline + short poll so the operator's reply
    // (written once the live pending card reveals the minted contract id) resolves
    // the gate promptly instead of via the fail-safe timeout.
    // SAFETY: these are restored at the end of the test (still under ENV_LOCK).
    let prev_home = std::env::var_os("SPECTYN_HOME");
    let prev_task = std::env::var_os("SPECTYN_GOVERN_TASK_ID");
    let prev_dl = std::env::var_os("SPECTYN_GOVERN_DEADLINE_SECS");
    let prev_poll = std::env::var_os("SPECTYN_GOVERN_POLL_SECS");
    std::env::set_var("SPECTYN_HOME", &data_dir);
    std::env::set_var("SPECTYN_GOVERN_TASK_ID", job_uuid.to_string());
    std::env::set_var("SPECTYN_GOVERN_DEADLINE_SECS", "30");
    std::env::set_var("SPECTYN_GOVERN_POLL_SECS", "1");

    // Drive the REAL production hook decision for a HIGH-RISK Bash tool. It runs on
    // the runtime; meanwhile the main thread watches for the live pending card the
    // production escalator writes, captures the minted contract id, and replies
    // "approve" correlated by that id so the gate resolves to allow.
    let hook_input = hook_in("Bash", json!({ "command": "rm -rf /tmp/x" }));
    let decision_handle = rt.spawn(async move { prod_decide_pretooluse_hook(hook_input).await });

    // Capture the production-minted contract id from the LIVE pending card (NOT
    // injected): poll the pending store (under SPECTYN_HOME) until the escalator's
    // card appears, then send the operator's approval referencing that id.
    let contract_id = {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let cards = spectyn_mesh::pending_approvals::list_pending(&data_dir)
                .expect("list pending cards");
            if let Some(card) = cards.into_iter().next() {
                break card.approval_id;
            }
            assert!(
                Instant::now() < deadline,
                "the production escalator must write a pending card for the high-risk tool"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    };
    // The minted contract id is a fresh ExecutionContract uuid — definitively NOT a
    // value the test supplied, and NOT the dispatch job_uuid being echoed.
    assert!(
        Uuid::parse_str(&contract_id).is_ok(),
        "the contract id must be a freshly minted uuid (production-minted), got {contract_id:?}"
    );
    assert_ne!(
        contract_id,
        job_uuid.to_string(),
        "the approval id must be the minted contract.id, not the dispatch task id echoed back"
    );

    // Operator approves, correlated to the minted contract id (topic = the id).
    spectyn_mesh::inbox::write_message(&data_dir, "phone", "approve", Some(&contract_id))
        .expect("write the operator's approval reply");

    let decision = rt
        .block_on(decision_handle)
        .expect("hook decision task must join");
    assert_eq!(
        decision["hookSpecificOutput"]["permissionDecision"], "allow",
        "the approved high-risk tool must be allowed by the production gate"
    );

    // RE-READ the dispatch row: the production escalation path must have stamped the
    // minted contract id onto it AND moved it to AwaitingApproval (live, while the
    // run was blocked in await_decision).
    let after = rt
        .block_on(store.get(job_uuid))
        .expect("read dispatch row after escalation")
        .expect("dispatch row still exists");
    assert_eq!(
        after.approval_id.as_deref(),
        Some(contract_id.as_str()),
        "the dispatch row's approval_id must equal the contract.id the production hook minted"
    );
    assert_eq!(
        after.status,
        TaskStatus::AwaitingApproval,
        "the dispatch row must be parked in AwaitingApproval while blocked on the phone decision"
    );

    // The /tasks-shape JSON (same `json!({"tasks": tasks})` the supervisor view
    // serializes a TaskRecord into) emits the correlated approval_id for the job.
    let tasks_json = json!({ "tasks": [after.clone()] });
    let row = &tasks_json["tasks"][0];
    assert_eq!(
        row["task_id"].as_str(),
        Some(job_uuid.to_string().as_str()),
        "the /tasks row must be the dispatch job"
    );
    assert_eq!(
        row["approval_id"].as_str(),
        Some(contract_id.as_str()),
        "the /tasks-shape JSON must emit the correlated approval_id for the awaiting-approval job"
    );

    // Restore the process env (still under ENV_LOCK).
    restore("SPECTYN_HOME", prev_home);
    restore("SPECTYN_GOVERN_TASK_ID", prev_task);
    restore("SPECTYN_GOVERN_DEADLINE_SECS", prev_dl);
    restore("SPECTYN_GOVERN_POLL_SECS", prev_poll);
}

#[test]
fn prod_hook_low_risk_leaves_dispatch_row_without_approval_id() {
    // CONTROL: a low-risk tool through the SAME production hook raises no approval,
    // so the escalation path is never reached and the dispatch row keeps
    // approval_id None / status Running (proving the positive test's stamp comes
    // from the REAL escalation path, not a side effect of attaching the store).
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    let tmp = tempdir().expect("tempdir must be created");
    let data_dir = tmp.path().to_path_buf();
    let db_path = data_dir.join("spectyn.db");
    let job_uuid = Uuid::new_v4();

    let rt = Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime must be created");

    let store = TaskStore::open_at(db_path).expect("open task store");
    seed_running_dispatch_row(&rt, &store, job_uuid);

    let prev_home = std::env::var_os("SPECTYN_HOME");
    let prev_task = std::env::var_os("SPECTYN_GOVERN_TASK_ID");
    std::env::set_var("SPECTYN_HOME", &data_dir);
    std::env::set_var("SPECTYN_GOVERN_TASK_ID", job_uuid.to_string());

    // A low-risk Read never escalates: prod_decide_pretooluse_hook auto-allows it
    // WITHOUT calling await_decision, so the dispatch row is untouched. Resolves
    // immediately (no inbox reply needed).
    let decision = rt.block_on(prod_decide_pretooluse_hook(hook_in(
        "Read",
        json!({ "path": "notes.txt" }),
    )));
    assert_eq!(
        decision["hookSpecificOutput"]["permissionDecision"], "allow",
        "a low-risk tool is auto-allowed by the production gate"
    );

    let after = rt
        .block_on(store.get(job_uuid))
        .expect("read dispatch row")
        .expect("dispatch row exists");
    assert_eq!(
        after.approval_id, None,
        "a run with no high-risk approval must leave the dispatch row's approval_id None"
    );
    assert_eq!(
        after.status,
        TaskStatus::Running,
        "a run with no approval must leave the dispatch row Running (not AwaitingApproval)"
    );

    restore("SPECTYN_HOME", prev_home);
    restore("SPECTYN_GOVERN_TASK_ID", prev_task);
}

/// Restore (or remove) a process env var to its captured prior value.
fn restore(key: &str, prev: Option<std::ffi::OsString>) {
    match prev {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
}
