//! Durable approval ledger (sprint MVP T8) — the bridge that records
//! [`ExecutionContract`](crate::execution_contract::ExecutionContract) approval
//! decisions onto the append-only task [`EventStore`] (the FlightRecorder spine).
//!
//! `execution_contract::ApprovalLedger` is the in-memory model; this is its
//! durable backing. Each contract decision becomes a task event
//! ([`TaskEventKind::ApprovalRequested`] / [`Approved`](TaskEventKind::Approved)
//! / [`Denied`](TaskEventKind::Denied)) whose `detail` carries the structured
//! payload, so a task's approval history is replayable/exportable alongside its
//! lifecycle events — and survives a restart.

use super::events::{EventStore, TaskEventKind};
use crate::execution_contract::{
    resolve, ApprovalDecision, ContractState, ExecutionContract, RiskLevel,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Structured `detail` payload for a recorded approval decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionDetail {
    pub contract_id: String,
    pub decision: ApprovalDecision,
    pub state_after: ContractState,
}

/// Record that a high-risk contract was raised and awaits approval. The full
/// contract is stored as JSON in `detail` (it carries `contract.id`). Returns
/// the appended event `seq`.
pub async fn record_request(
    events: &EventStore,
    task_id: Uuid,
    contract: &ExecutionContract,
) -> Result<i64> {
    let detail = serde_json::to_string(contract)?;
    events
        .append(task_id, TaskEventKind::ApprovalRequested, Some(&detail))
        .await
}

/// Record an operator decision on a contract + the resulting state. The event
/// kind reflects the outcome (`Approved` when `state_after == Approved`, else
/// `Denied`); `detail` carries the exact [`DecisionDetail`]. Returns the seq.
pub async fn record_decision(
    events: &EventStore,
    task_id: Uuid,
    contract_id: &str,
    decision: ApprovalDecision,
    state_after: ContractState,
) -> Result<i64> {
    let kind = if state_after == ContractState::Approved {
        TaskEventKind::Approved
    } else {
        TaskEventKind::Denied
    };
    let detail = serde_json::to_string(&DecisionDetail {
        contract_id: contract_id.to_string(),
        decision,
        state_after,
    })?;
    events.append(task_id, kind, Some(&detail)).await
}

/// Reconstruct the latest recorded [`ContractState`] for `contract_id` from the
/// durable event log. A contract that only has an `ApprovalRequested` event is
/// `Pending`; the most recent decision event wins. Returns `None` if the
/// contract is unknown to this task's log.
pub async fn latest_state(
    events: &EventStore,
    task_id: Uuid,
    contract_id: &str,
) -> Result<Option<ContractState>> {
    let log = events.events_for(task_id).await?;
    let mut state: Option<ContractState> = None;
    for ev in &log {
        match ev.kind {
            TaskEventKind::ApprovalRequested => {
                if let Some(d) = &ev.detail {
                    if let Ok(c) = serde_json::from_str::<ExecutionContract>(d) {
                        if c.id == contract_id {
                            state = Some(ContractState::Pending);
                        }
                    }
                }
            }
            TaskEventKind::Approved | TaskEventKind::Denied => {
                if let Some(d) = &ev.detail {
                    if let Ok(dd) = serde_json::from_str::<DecisionDetail>(d) {
                        if dd.contract_id == contract_id {
                            state = Some(dd.state_after);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(state)
}

/// What a task runner must do when it reaches a contracted action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateOutcome {
    /// Approved — the action may run.
    Allow,
    /// Denied / cancelled / expired — the action must NOT run.
    Deny,
    /// Still pending — block and route an approval card to the operator.
    NeedsApproval,
}

/// Tokenize a tool name into lowercase WHOLE-WORD tokens: split on `_ - . / : space`
/// and at camelCase boundaries. `WebFetch` -> `[web, fetch]`, `mutate_state` ->
/// `[mutate, state]`, `devastate` -> `[devastate]`. Classification then matches whole
/// tokens, so a benign substring inside a dangerous name (e.g. `stat` inside
/// `mutate_state`, or `read` inside `http_read`) can NEVER trip the read-only path.
fn tokenize_tool(name: &str) -> Vec<String> {
    let mut spaced = String::with_capacity(name.len() * 2);
    let mut prev_alnum_lower = false;
    for ch in name.chars() {
        if matches!(ch, '_' | '-' | '.' | '/' | ':' | ' ') {
            spaced.push(' ');
            prev_alnum_lower = false;
            continue;
        }
        if ch.is_ascii_uppercase() && prev_alnum_lower {
            spaced.push(' '); // camelCase boundary (lower|digit -> Upper)
        }
        spaced.push(ch.to_ascii_lowercase());
        prev_alnum_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
    }
    spaced.split_whitespace().map(|s| s.to_string()).collect()
}

/// Classify a tool call into a [`RiskLevel`] so the runner knows whether it needs an
/// approval contract. Only `ReadOnly` / `ExecuteLow` are auto-allowed
/// ([`RiskLevel::requires_approval`]), so the ONE dangerous misclassification is
/// toward read-only. Three rules guarantee against it:
/// 1. **DENYLIST-FIRST, whole-token match** — any exec/network/write token wins
///    immediately (most-dangerous capability first), so a dangerous verb can never be
///    shadowed by a read token.
/// 2. **ReadOnly demands FULL confidence** — granted only when there is a read token
///    AND *every* token is known-innocuous (a read verb or a SAFE noun). An UNKNOWN
///    token (e.g. an unrecognised dangerous verb) forces conservative `ExecuteHigh`;
///    it cannot hide behind a read token.
/// 3. **Conservative default** — anything else is `ExecuteHigh` (needs approval).
///
/// The EXEC/NETWORK/WRITE lists may over-match (that only over-GATES = safe); the
/// READONLY + SAFE lists are the only ones that auto-allow, so they are kept precise.
///
/// ARGS-AWARE UPGRADE (pure raise, never downgrade): the name-based result is a
/// FLOOR. We additionally scan every string value in `args` (recursively, through
/// nested objects/arrays) for danger indicators — a `rm -rf` buried in a
/// `command` field, a `~/.ssh/id_rsa` path, an `http://` egress URL, a write into
/// `/etc/` — and RAISE the risk to the MAX of the name-based floor and what the
/// args reveal. ReadOnly with empty/benign args stays ReadOnly; a name-based
/// `ExecuteHigh` is never lowered by benign args.
pub fn classify_tool(name: &str, args: &serde_json::Value) -> RiskLevel {
    // SENSITIVE: secret/credential material. Even a READ of these is high-risk
    // (exfiltration), so any of these tokens forces approval regardless of the verb —
    // checked FIRST so `read_keys` / `show_env` cannot auto-allow. (agy review.)
    const SENSITIVE: &[&str] = &[
        "secret", "secrets", "credential", "credentials", "password", "passwd",
        "passphrase", "token", "tokens", "apikey", "key", "keys", "keystore",
        "private", "privatekey", "env", "environ", "environment", "identity", "vault",
        "cert", "certs", "certificate", "pem", "seed", "mnemonic",
    ];
    // Arbitrary code execution / process control.
    const EXEC: &[&str] = &[
        "bash", "sh", "shell", "zsh", "fish", "cmd", "powershell", "pwsh", "exec",
        "execute", "run", "eval", "spawn", "system", "popen", "kill", "terminal", "pty",
        "sudo", "su", "start", "stop", "restart", "terminate", "launch", "invoke",
        "call", "trigger", "boot", "init", "daemon", "serve", "server", "process",
    ];
    // Outbound network / egress.
    const NETWORK: &[&str] = &[
        "http", "https", "web", "fetch", "curl", "wget", "url", "uri", "download",
        "upload", "request", "ssh", "scp", "sftp", "ftp", "api", "socket", "websocket",
        "ws", "wss", "smtp", "email", "send", "ping", "connect", "listen", "bind",
        "publish", "subscribe", "pull", "clone", "sync", "dispatch", "notify",
    ];
    // Filesystem / state mutation, VCS writes, destructive ops.
    const WRITE: &[&str] = &[
        "write", "edit", "patch", "commit", "push", "apply", "rm", "delete", "remove",
        "unlink", "create", "mkdir", "touch", "mv", "move", "rename", "copy", "cp",
        "mutate", "modify", "update", "set", "store", "save", "install", "uninstall",
        "chmod", "chown", "append", "truncate", "drop", "insert", "put", "replace",
        "format", "overwrite", "destroy", "clear", "reset", "wipe", "purge", "flush",
        "revert", "restore", "stash", "checkout", "merge", "rebase", "add", "register",
        "enable", "disable", "grant", "revoke", "lock", "unlock", "encrypt", "decrypt",
        "sign", "seal", "schedule", "generate",
    ];
    // Pure read / inspection VERBS (the only verbs that may auto-allow).
    const READONLY: &[&str] = &[
        "read", "ls", "glob", "grep", "search", "show", "list", "status", "stat", "cat",
        "head", "tail", "view", "find", "locate", "diff", "log", "blame", "inspect",
        "info", "describe", "recall", "dir", "pwd", "whoami", "exists", "count", "peek",
        "preview",
    ];
    // Known-innocuous NOUNS / qualifiers that may accompany a read verb without
    // forcing a gate. Kept conservative — an unknown noun gates (safe direction).
    // Known-innocuous NOUNS/qualifiers. EXCLUDES: secrets (-> SENSITIVE),
    // network-implying nouns (page/peer), and directional prepositions (to/from/by/of)
    // — those enable bypasses like `read_to_file` (a write) / `read_page` (egress), so
    // they are NOT innocuous and gate via the unknown-token rule.
    const SAFE_NOUNS: &[&str] = &[
        "file", "files", "content", "git", "memory", "mcp", "spectyn", "dir",
        "directory", "path", "paths", "project", "repo", "repository", "workspace",
        "cluster", "session", "sessions", "node", "nodes", "task", "tasks", "todo",
        "todos", "diag", "info", "meta", "metadata", "data", "all", "current", "local",
        "recent", "last", "first", "full", "raw", "text", "json", "doc", "docs",
        "config", "configs", "setting", "settings", "version", "snapshot", "history",
        "line", "lines", "range", "entry", "entries", "item", "items", "count", "size",
        "id", "ids", "name", "names", "num", "the", "and", "or", "with", "a", "n",
        "tree", "state",
    ];

    let tokens = tokenize_tool(name);
    let any = |set: &[&str]| tokens.iter().any(|t| set.contains(&t.as_str()));

    // --- name-based result (the FLOOR; never downgraded below this) ---
    let name_risk = {
        // Secret/credential material — gate even a "read" (exfiltration risk).
        if any(SENSITIVE) {
            RiskLevel::ExecuteHigh
        } else if any(EXEC) {
            RiskLevel::ExecuteHigh
        } else if any(NETWORK) {
            RiskLevel::Network
        } else if any(WRITE) {
            RiskLevel::Write
        } else {
            // ReadOnly only with FULL confidence: a read verb present AND no unknown token.
            let has_read = any(READONLY);
            let all_innocuous = !tokens.is_empty()
                && tokens.iter().all(|t| {
                    READONLY.contains(&t.as_str()) || SAFE_NOUNS.contains(&t.as_str())
                });
            if has_read && all_innocuous {
                RiskLevel::ReadOnly
            } else {
                // Unknown / unrecognised capability → conservative: require approval.
                RiskLevel::ExecuteHigh
            }
        }
    };

    // --- args-based result (a pure RAISE) ---
    // Collect every string value in the args JSON and score each against the
    // danger heuristics; the args contribution is the MAX score seen. The final
    // risk is MAX(name_risk, args_risk) — args can only RAISE the floor.
    let args_risk = score_args(args);
    std::cmp::max(name_risk, args_risk.unwrap_or(name_risk))
}

/// Recursively push every string value found in a JSON value (through nested
/// objects and arrays) into `out`. Object KEYS are intentionally ignored — only
/// the operator-/agent-supplied VALUES carry the dangerous payload.
fn collect_strings<'a>(v: &'a serde_json::Value, out: &mut Vec<&'a str>) {
    match v {
        serde_json::Value::String(s) => out.push(s.as_str()),
        serde_json::Value::Array(arr) => {
            for item in arr {
                collect_strings(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for val in map.values() {
                collect_strings(val, out);
            }
        }
        // numbers / bools / null carry no string payload to scan.
        _ => {}
    }
}

/// Score the whole args blob: collect all string values, score each, and return
/// the MAX (by [`RiskLevel`] `Ord`). `None` means no danger indicator matched —
/// e.g. empty `{}` args, or a benign `{"path":"src/main.rs"}` — so the caller
/// leaves the name-based floor untouched.
fn score_args(args: &serde_json::Value) -> Option<RiskLevel> {
    let mut strings: Vec<&str> = Vec::new();
    collect_strings(args, &mut strings);
    strings.iter().filter_map(|s| score_string(s)).max()
}

/// Heuristically score ONE string value against danger indicators, returning the
/// raised risk it warrants (or `None` if benign). Matching is case-insensitive.
/// Indicators are grouped most-specific-first; the function returns the MAX of
/// every group that matches so e.g. an `http://` arch onto an `rm -rf` line still
/// lands at the highest applicable level. Patterns are kept TIGHT so a benign
/// path/URL fragment cannot trip them (see the `*_keeps_benign_args` test).
fn score_string(s: &str) -> Option<RiskLevel> {
    let l = s.to_ascii_lowercase();
    let mut risk: Option<RiskLevel> = None;
    let mut raise = |lvl: RiskLevel| {
        risk = Some(match risk {
            Some(cur) => std::cmp::max(cur, lvl),
            None => lvl,
        });
    };

    // --- shell-dangerous → ExecuteHigh ---
    // Destructive/privileged shell, fork bombs, pipe-to-interpreter, code-exec.
    let shell_dangerous = l.contains("rm -rf")
        // `sudo ` (with the command space) — bare `sudo` matched `sudoku`.
        || l.contains("sudo ")
        || l.contains("mkfs")
        || l.contains("dd if=")
        || l.contains(":(){")
        || l.contains("| sh")
        || l.contains("|sh")
        || l.contains("| bash")
        || l.contains("|bash")
        // curl piped into something (download-and-run) — both markers present.
        || (l.contains("curl ") && l.contains('|'))
        // redirecting output INTO a system dir (`> /etc/...`, `>>/bin/...`).
        || ((l.contains("> /etc") || l.contains(">/etc"))
            || (l.contains("> /bin") || l.contains(">/bin")))
        || l.contains("chmod 777")
        || l.contains("invoke-expression")
        || l.contains("iex ")
        || l.contains("powershell")
        || l.contains("reg delete");
    if shell_dangerous {
        raise(RiskLevel::ExecuteHigh);
    }

    // --- secret material (read OR write of it is high-risk) → ExecuteHigh ---
    let secret = l.contains(".ssh")
        || l.contains("id_rsa")
        || l.contains("id_ed25519")
        || l.contains(".env")
        || l.contains("credentials")
        || l.contains("secret")
        || l.contains(".pem")
        || l.contains("identity.key")
        || l.contains("cluster_secret")
        || l.contains("private key")
        || l.contains("apikey")
        || l.contains("password");
    if secret {
        raise(RiskLevel::ExecuteHigh);
    }

    // --- system / absolute write path → Write ---
    // (Below Network in Ord, so a combined http+/etc still resolves to Network.)
    let system_write = l.contains("/etc/")
        || l.contains("/bin/")
        || l.contains("/usr/")
        || l.contains("/boot/")
        || l.contains("c:\\windows")
        || l.contains("c:/windows")
        || l.contains("/system/");
    if system_write {
        raise(RiskLevel::Write);
    }

    // --- network egress → Network (the highest level) ---
    let network = l.contains("http://")
        || l.contains("https://")
        || l.contains("ftp://")
        || l.contains("ssh://")
        || l.contains("scp ")
        || l.contains("wget ")
        // netcat as a WHOLE word — `contains("nc ")` alone matched `func `/`sync `.
        || l.starts_with("nc ")
        || l.contains(" nc ");
    if network {
        raise(RiskLevel::Network);
    }

    risk
}

/// The deny-until-approved gate: map a contract's current [`ContractState`] to
/// the runner's decision. The default (`Pending`) is `NeedsApproval`, never
/// `Allow` — a high-risk action can only proceed once explicitly `Approved`.
pub fn gate(state: ContractState) -> GateOutcome {
    match state {
        ContractState::Approved => GateOutcome::Allow,
        ContractState::Pending => GateOutcome::NeedsApproval,
        ContractState::Denied | ContractState::Cancelled | ContractState::Expired => {
            GateOutcome::Deny
        }
    }
}

/// All contracts on `task_id` whose latest recorded state is still `Pending`
/// (i.e. awaiting an operator decision), in request order. Drives a
/// `spectyn task approvals <id>` / phone "what needs me?" view.
pub async fn pending_for(
    events: &EventStore,
    task_id: Uuid,
) -> Result<Vec<ExecutionContract>> {
    let log = events.events_for(task_id).await?;
    // Collect requested contracts in order, then keep only those whose latest
    // decision (if any) left them Pending. A single scan suffices: track the
    // latest state per contract id alongside the contract payload.
    let mut order: Vec<String> = Vec::new();
    let mut contracts: std::collections::HashMap<String, ExecutionContract> =
        std::collections::HashMap::new();
    let mut state: std::collections::HashMap<String, ContractState> =
        std::collections::HashMap::new();
    for ev in &log {
        match ev.kind {
            TaskEventKind::ApprovalRequested => {
                if let Some(d) = &ev.detail {
                    if let Ok(c) = serde_json::from_str::<ExecutionContract>(d) {
                        if !contracts.contains_key(&c.id) {
                            order.push(c.id.clone());
                        }
                        state.insert(c.id.clone(), ContractState::Pending);
                        contracts.insert(c.id.clone(), c);
                    }
                }
            }
            TaskEventKind::Approved | TaskEventKind::Denied => {
                if let Some(d) = &ev.detail {
                    if let Ok(dd) = serde_json::from_str::<DecisionDetail>(d) {
                        state.insert(dd.contract_id, dd.state_after);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(order
        .into_iter()
        .filter(|id| state.get(id) == Some(&ContractState::Pending))
        .filter_map(|id| contracts.remove(&id))
        .collect())
}

/// All contract ids on `task_id` whose latest recorded state is `Approved` —
/// the set the runner loads into the sync `contract_gate` snapshot before an
/// agent loop so already-approved actions run without re-prompting.
pub async fn approved_for(events: &EventStore, task_id: Uuid) -> Result<Vec<String>> {
    let log = events.events_for(task_id).await?;
    let mut state: std::collections::HashMap<String, ContractState> =
        std::collections::HashMap::new();
    for ev in &log {
        match ev.kind {
            TaskEventKind::ApprovalRequested => {
                if let Some(d) = &ev.detail {
                    if let Ok(c) = serde_json::from_str::<ExecutionContract>(d) {
                        state.entry(c.id).or_insert(ContractState::Pending);
                    }
                }
            }
            TaskEventKind::Approved | TaskEventKind::Denied => {
                if let Some(d) = &ev.detail {
                    if let Ok(dd) = serde_json::from_str::<DecisionDetail>(d) {
                        state.insert(dd.contract_id, dd.state_after);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(state
        .into_iter()
        .filter(|(_, s)| *s == ContractState::Approved)
        .map(|(id, _)| id)
        .collect())
}

/// Enforcement entry point a task runner calls before a contracted action.
///
/// - A low-risk contract (`!requires_approval`) is auto-allowed, no ledger row.
/// - Otherwise: the FIRST time a contract is seen it is durably raised
///   (`record_request` → Pending); subsequent calls reuse the recorded state
///   (idempotent — no duplicate requests). Expiry is applied, then the gate
///   decision is returned. The runner blocks on `NeedsApproval`, refuses on
///   `Deny`, and proceeds only on `Allow`.
pub async fn enforce(
    events: &EventStore,
    task_id: Uuid,
    contract: &ExecutionContract,
    now_ms: i64,
) -> Result<GateOutcome> {
    if !contract.risk.requires_approval() {
        return Ok(GateOutcome::Allow);
    }
    let state = match latest_state(events, task_id, &contract.id).await? {
        Some(s) => s,
        None => {
            record_request(events, task_id, contract).await?;
            ContractState::Pending
        }
    };
    Ok(gate(resolve(contract, state, now_ms)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_contract::{apply, RiskLevel};
    use crate::tasks::store::TaskStore;

    fn store() -> EventStore {
        let s = TaskStore::open_at(std::path::PathBuf::from(":memory:")).unwrap();
        EventStore::from_conn(s.conn())
    }

    fn contract() -> ExecutionContract {
        ExecutionContract::new(
            "z13",
            "Claude Code",
            "shell.run",
            "cargo test",
            "/repo",
            vec![],
            RiskLevel::ExecuteHigh,
            "verify",
            600,
        )
    }

    #[tokio::test]
    async fn request_then_approve_persists_and_replays() {
        let ev = store();
        let task = Uuid::new_v4();
        let c = contract();

        // Raised -> Pending in the durable log.
        record_request(&ev, task, &c).await.unwrap();
        assert_eq!(
            latest_state(&ev, task, &c.id).await.unwrap(),
            Some(ContractState::Pending)
        );

        // Operator approves -> Approved.
        let st = apply(ContractState::Pending, ApprovalDecision::ApproveOnce);
        record_decision(&ev, task, &c.id, ApprovalDecision::ApproveOnce, st)
            .await
            .unwrap();
        assert_eq!(
            latest_state(&ev, task, &c.id).await.unwrap(),
            Some(ContractState::Approved)
        );
    }

    #[tokio::test]
    async fn deny_is_recorded_and_terminal() {
        let ev = store();
        let task = Uuid::new_v4();
        let c = contract();
        record_request(&ev, task, &c).await.unwrap();
        let st = apply(ContractState::Pending, ApprovalDecision::Deny);
        record_decision(&ev, task, &c.id, ApprovalDecision::Deny, st)
            .await
            .unwrap();
        assert_eq!(
            latest_state(&ev, task, &c.id).await.unwrap(),
            Some(ContractState::Denied)
        );
    }

    #[tokio::test]
    async fn unknown_contract_has_no_state() {
        let ev = store();
        let task = Uuid::new_v4();
        assert_eq!(latest_state(&ev, task, "nope").await.unwrap(), None);
    }

    #[tokio::test]
    async fn two_contracts_on_one_task_are_independent() {
        let ev = store();
        let task = Uuid::new_v4();
        let a = contract();
        let b = contract();
        record_request(&ev, task, &a).await.unwrap();
        record_request(&ev, task, &b).await.unwrap();
        record_decision(
            &ev,
            task,
            &a.id,
            ApprovalDecision::ApproveOnce,
            ContractState::Approved,
        )
        .await
        .unwrap();
        // a approved, b still pending — independent.
        assert_eq!(
            latest_state(&ev, task, &a.id).await.unwrap(),
            Some(ContractState::Approved)
        );
        assert_eq!(
            latest_state(&ev, task, &b.id).await.unwrap(),
            Some(ContractState::Pending)
        );
    }

    #[test]
    fn gate_is_deny_until_approved() {
        assert_eq!(gate(ContractState::Approved), GateOutcome::Allow);
        assert_eq!(gate(ContractState::Pending), GateOutcome::NeedsApproval);
        assert_eq!(gate(ContractState::Denied), GateOutcome::Deny);
        assert_eq!(gate(ContractState::Cancelled), GateOutcome::Deny);
        assert_eq!(gate(ContractState::Expired), GateOutcome::Deny);
    }

    #[tokio::test]
    async fn pending_for_lists_only_undecided_contracts_in_order() {
        let ev = store();
        let task = Uuid::new_v4();
        let a = contract();
        let b = contract();
        let c = contract();
        record_request(&ev, task, &a).await.unwrap();
        record_request(&ev, task, &b).await.unwrap();
        record_request(&ev, task, &c).await.unwrap();
        // a approved, b denied → only c stays pending.
        record_decision(&ev, task, &a.id, ApprovalDecision::ApproveOnce, ContractState::Approved)
            .await
            .unwrap();
        record_decision(&ev, task, &b.id, ApprovalDecision::Deny, ContractState::Denied)
            .await
            .unwrap();
        let pending = pending_for(&ev, task).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, c.id);
        // gate() agrees: c needs approval, a allowed, b denied.
        assert_eq!(
            gate(latest_state(&ev, task, &c.id).await.unwrap().unwrap()),
            GateOutcome::NeedsApproval
        );
        assert_eq!(
            gate(latest_state(&ev, task, &a.id).await.unwrap().unwrap()),
            GateOutcome::Allow
        );
    }

    #[tokio::test]
    async fn malformed_approval_detail_is_skipped_not_fatal() {
        // A corrupt/garbage approval-event detail must not crash latest_state or
        // pending_for — it is silently skipped (the `if let Ok` guards). A valid
        // contract recorded alongside it still resolves correctly.
        let ev = store();
        let task = Uuid::new_v4();
        // Raw event with non-deserialisable detail.
        ev.append(task, TaskEventKind::ApprovalRequested, Some("{not json"))
            .await
            .unwrap();
        let c = contract();
        record_request(&ev, task, &c).await.unwrap();

        // The garbage row contributes no contract; the valid one is Pending.
        assert_eq!(latest_state(&ev, task, "whatever").await.unwrap(), None);
        assert_eq!(
            latest_state(&ev, task, &c.id).await.unwrap(),
            Some(ContractState::Pending)
        );
        let pending = pending_for(&ev, task).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, c.id);
    }

    #[test]
    fn classify_tool_is_conservative() {
        let v = serde_json::json!({});
        assert_eq!(classify_tool("file_read", &v), RiskLevel::ReadOnly);
        assert_eq!(classify_tool("glob_search", &v), RiskLevel::ReadOnly);
        assert_eq!(classify_tool("http_post", &v), RiskLevel::Network);
        assert_eq!(classify_tool("file_write", &v), RiskLevel::Write);
        assert_eq!(classify_tool("git_commit", &v), RiskLevel::Write);
        assert_eq!(classify_tool("bash", &v), RiskLevel::ExecuteHigh);
        // unknown tool → conservative (needs approval)
        assert_eq!(classify_tool("frobnicate", &v), RiskLevel::ExecuteHigh);
    }

    #[test]
    fn classify_tool_closes_substring_auto_allow_holes() {
        // These names CONTAIN a read-only substring but are NOT read-only. The old
        // substring classifier auto-allowed them (a fail-open); whole-token matching
        // + denylist-first must make every one of them require approval.
        let v = serde_json::json!({});
        for (name, why) in [
            ("mutate_state", "contains `stat` but mutates"),
            ("devastate", "contains `stat` but is destructive"),
            ("reinstate", "contains `stat`"),
            ("http_read", "contains `read` but is network egress"),
            ("read_exec", "contains `read` but executes"),
            ("search_and_delete", "contains `search` but deletes"),
            ("grep_then_rm", "contains `grep` but removes"),
            ("status_push", "contains `status` but pushes"),
        ] {
            let risk = classify_tool(name, &v);
            assert!(
                risk.requires_approval(),
                "`{name}` ({why}) must require approval, got {risk:?}"
            );
        }
        // Specific labels (most-dangerous capability wins).
        assert_eq!(classify_tool("mutate_state", &v), RiskLevel::Write);
        assert_eq!(classify_tool("http_read", &v), RiskLevel::Network);
        assert_eq!(classify_tool("read_exec", &v), RiskLevel::ExecuteHigh);
        assert_eq!(classify_tool("devastate", &v), RiskLevel::ExecuteHigh);
    }

    #[test]
    fn classify_tool_unknown_token_cannot_hide_behind_a_read_token() {
        // The agy-review structural hole: a tool whose name pairs a read token with
        // an UNRECOGNISED token (an unknown — possibly dangerous — verb) must NOT be
        // auto-allowed. Full-confidence ReadOnly (no unknown token) + denylist verbs
        // close this.
        let v = serde_json::json!({});
        for name in [
            // missing-denylist-verb cases agy named
            "execute_read", "execute_search", "destroy_list", "clear_log",
            "reset_status", "connect_status", "listen_status", "start_read",
            "terminate_status",
            // the structural unknown-token-shadow case
            "obliterate_list", "zorblefy_read", "exfiltrate_status", "frobnicate_view",
        ] {
            let risk = classify_tool(name, &v);
            assert!(
                risk.requires_approval(),
                "`{name}` pairs a read token with an unknown/dangerous one — must require approval, got {risk:?}"
            );
        }
    }

    #[test]
    fn classify_tool_gates_sensitive_reads_and_preposition_bypasses() {
        // agy review round 2: a "read" of secrets is exfiltration (gate it), and
        // directional/network nouns must not make a write/egress look read-only.
        let v = serde_json::json!({});
        for name in [
            // SENSITIVE: reading secrets is high-risk even though the verb is a read
            "read_keys", "show_keys", "read_env", "show_env", "get_secret",
            "read_credentials", "list_tokens", "view_password", "read_identity",
            "memory_read_private_key", "show_vault",
            // preposition/network bypasses (to=destination=write, page/peer=egress)
            "read_to_file", "read_to_peer", "read_page", "view_page", "copy_from_to",
        ] {
            let risk = classify_tool(name, &v);
            assert!(
                risk.requires_approval(),
                "`{name}` must require approval (sensitive/egress/write), got {risk:?}"
            );
        }
        // Sanity: a plain dict/key-less read still auto-allows.
        assert_eq!(classify_tool("memory_list", &v), RiskLevel::ReadOnly);
        assert_eq!(classify_tool("git_status", &v), RiskLevel::ReadOnly);
    }

    #[test]
    fn classify_tool_keeps_real_read_tools_auto_allowed() {
        // Genuine read tools (spectyn MCP + claude built-ins) STAY ReadOnly so a
        // governed run is not buried in approval prompts. `stat` is now safe as a
        // WHOLE token (the `stat` tool) without matching `mutate_state`.
        let v = serde_json::json!({});
        for name in [
            "Read", "Glob", "Grep", "LS", "file_read", "content_search", "glob_search",
            "git_status", "git_log", "git_diff", "git_show", "git_blame", "memory_list",
            "stat", "ls", "list_files", "show", "diff_files",
        ] {
            assert_eq!(
                classify_tool(name, &v),
                RiskLevel::ReadOnly,
                "`{name}` should stay ReadOnly (auto-allowed)"
            );
        }
        // claude built-ins that ARE dangerous classify high (camelCase tokenized).
        assert!(classify_tool("Bash", &v).requires_approval());
        assert!(classify_tool("Write", &v).requires_approval());
        assert!(classify_tool("Edit", &v).requires_approval());
        assert_eq!(classify_tool("WebFetch", &v), RiskLevel::Network); // [web, fetch]
        assert_eq!(classify_tool("WebSearch", &v), RiskLevel::Network); // web egress
    }

    #[test]
    fn classify_tool_args_aware_raises_on_dangerous_args() {
        use serde_json::json;
        // (a) reading a secret path upgrades a benign-named read to require approval.
        assert!(
            classify_tool("file_read", &json!({"path": "~/.ssh/id_rsa"}))
                .requires_approval(),
            "secret read must be upgraded to require approval"
        );
        // (b) a destructive shell command in args → ExecuteHigh.
        assert_eq!(
            classify_tool("helper", &json!({"command": "rm -rf /"})),
            RiskLevel::ExecuteHigh
        );
        // (c) an http:// egress URL → at least Network.
        assert!(
            classify_tool("fetch_thing", &json!({"url": "http://evil"})) >= RiskLevel::Network,
            "network egress arg must raise to at least Network"
        );
        // (d) a benign read with a benign path STAYS ReadOnly (no false positive).
        assert_eq!(
            classify_tool("file_read", &json!({"path": "src/main.rs"})),
            RiskLevel::ReadOnly
        );
        // (e) a name-based ExecuteHigh is NEVER downgraded by benign args.
        assert_eq!(
            classify_tool("bash", &json!({"note": "totally harmless"})),
            RiskLevel::ExecuteHigh
        );
    }

    #[test]
    fn classify_tool_args_aware_scans_nested_and_combines_max() {
        use serde_json::json;
        // Danger buried in a nested object/array is still found.
        assert!(
            classify_tool("helper", &json!({"steps": [{"cmd": "sudo rm -rf /var"}]}))
                .requires_approval(),
            "nested dangerous arg must be detected"
        );
        // A secret value in an array element raises.
        assert!(
            classify_tool("file_read", &json!({"paths": ["README.md", "config/credentials"]}))
                .requires_approval()
        );
        // Network beats a system-write floor: http:// + /etc/ → Network (the MAX).
        assert_eq!(
            classify_tool("helper", &json!({"a": "/etc/hosts", "b": "https://x"})),
            RiskLevel::Network
        );
        // A bare system path with no other danger raises a benign read to Write.
        assert_eq!(
            classify_tool("file_read", &json!({"path": "/etc/passwd"})),
            RiskLevel::Write
        );
    }

    #[test]
    fn classify_tool_args_aware_keeps_benign_args_unraised() {
        use serde_json::json;
        // Ordinary project paths / values must NOT trip any heuristic — these stay
        // exactly at their name-based floor.
        for args in [
            json!({}),
            json!({"path": "src/main.rs"}),
            json!({"path": "core/src/tasks/approvals.rs", "limit": 200}),
            json!({"query": "fn classify_tool", "glob": "**/*.rs"}),
            json!({"name": "my-project", "count": 3, "ok": true}),
        ] {
            assert_eq!(
                classify_tool("file_read", &args),
                RiskLevel::ReadOnly,
                "benign args {args:?} must stay ReadOnly"
            );
        }
    }

    #[test]
    fn tokenize_tool_splits_separators_and_camelcase() {
        assert_eq!(tokenize_tool("mutate_state"), vec!["mutate", "state"]);
        assert_eq!(tokenize_tool("WebFetch"), vec!["web", "fetch"]);
        assert_eq!(tokenize_tool("devastate"), vec!["devastate"]);
        assert_eq!(tokenize_tool("git.commit-now"), vec!["git", "commit", "now"]);
        assert_eq!(tokenize_tool("mcp__spectyn__file_read"), vec!["mcp", "spectyn", "file", "read"]);
    }

    #[tokio::test]
    async fn enforce_blocks_until_approved_and_is_idempotent() {
        let ev = store();
        let task = Uuid::new_v4();
        let c = contract(); // ExecuteHigh
        let now = c.created_ms;
        // First call raises the contract + returns NeedsApproval.
        assert_eq!(enforce(&ev, task, &c, now).await.unwrap(), GateOutcome::NeedsApproval);
        // Idempotent: a second call does NOT re-request (still exactly one pending).
        assert_eq!(enforce(&ev, task, &c, now).await.unwrap(), GateOutcome::NeedsApproval);
        assert_eq!(pending_for(&ev, task).await.unwrap().len(), 1);
        // Operator approves → enforce now allows the action.
        record_decision(&ev, task, &c.id, ApprovalDecision::ApproveOnce, ContractState::Approved)
            .await
            .unwrap();
        assert_eq!(enforce(&ev, task, &c, now).await.unwrap(), GateOutcome::Allow);
    }

    #[tokio::test]
    async fn enforce_auto_allows_low_risk_without_ledger() {
        let ev = store();
        let task = Uuid::new_v4();
        let c = ExecutionContract::new(
            "z13", "x", "read", "cat f", "/r", vec![], RiskLevel::ReadOnly, "r", 600,
        );
        assert_eq!(enforce(&ev, task, &c, c.created_ms).await.unwrap(), GateOutcome::Allow);
        // No ledger row was created for an auto-allowed low-risk action.
        assert!(pending_for(&ev, task).await.unwrap().is_empty());
        assert_eq!(latest_state(&ev, task, &c.id).await.unwrap(), None);
    }

    #[tokio::test]
    async fn enforce_denies_an_expired_pending_contract() {
        let ev = store();
        let task = Uuid::new_v4();
        let c = contract(); // ttl 600s
        // raise it
        assert_eq!(enforce(&ev, task, &c, c.created_ms).await.unwrap(), GateOutcome::NeedsApproval);
        // far past expiry, still Pending → Deny (expired)
        assert_eq!(enforce(&ev, task, &c, c.expires_ms + 1).await.unwrap(), GateOutcome::Deny);
    }
}
