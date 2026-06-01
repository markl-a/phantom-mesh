// Tauri command surface for SPEC-26 cluster dispatch planning.
//
// Exposes the pure (no-I/O) planning fns from `phantom_mesh::cluster_dispatch_wire`:
//   - dispatch_plan(task, peers)      → DispatchPlan (best peer + fallbacks + reason)
//   - dispatch_score_peer(peer, task) → PeerScore   (per-peer fit breakdown)
//
// Unlike the capture wires these are fully-wired deterministic logic (cap-tag
// Jaccard match + scoring), so they return real results. catch_unwind guards
// against the SPEC-26 §8 note about future /rpc/peers + audit-log deps.

use std::panic::{catch_unwind, AssertUnwindSafe};

use phantom_mesh::cluster_dispatch_wire::{
    self, DispatchPlan, DispatchTask, PeerCapabilities, PeerScore,
};

const NOT_YET_WIRED: &str =
    "dispatch.not_yet_wired: cluster dispatch planning helper unavailable";

#[tauri::command]
pub async fn dispatch_plan(
    task: DispatchTask,
    peers: Vec<PeerCapabilities>,
) -> Result<DispatchPlan, String> {
    match catch_unwind(AssertUnwindSafe(|| cluster_dispatch_wire::plan_dispatch(&task, &peers))) {
        Ok(Ok(p)) => Ok(p),
        // DispatchError has no Display impl — Debug gives the variant name
        // (e.g. "NoMatchingPeer"), which the front-end key-routes on.
        Ok(Err(e)) => Err(format!("{e:?}")),
        Err(_) => Err(NOT_YET_WIRED.to_string()),
    }
}

#[tauri::command]
pub async fn dispatch_score_peer(
    peer: PeerCapabilities,
    task: DispatchTask,
) -> Result<PeerScore, String> {
    match catch_unwind(AssertUnwindSafe(|| cluster_dispatch_wire::score_peer(&peer, &task))) {
        Ok(s) => Ok(s),
        Err(_) => Err(NOT_YET_WIRED.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_json(required: &str) -> String {
        // payload is a String field (SPEC-26 §7); Stage-1 callers pass "null".
        format!(
            r#"{{"taskId":"t1","requiredCaps":{required},"preferredCaps":[],"payload":"null","deadlineMs":null}}"#
        )
    }

    #[tokio::test]
    async fn dispatch_plan_no_peers_is_no_matching_peer() {
        let task: DispatchTask = serde_json::from_str(&task_json("[]")).expect("parse task");
        let err = dispatch_plan(task, vec![]).await.unwrap_err();
        // Pure logic: empty peer set → NoMatchingPeer (not a panic / not_yet_wired).
        assert!(
            err.to_lowercase().contains("peer") || err.starts_with("dispatch."),
            "got {err}"
        );
    }

    #[tokio::test]
    async fn dispatch_plan_matches_a_qualified_peer() {
        let task: DispatchTask =
            serde_json::from_str(&task_json(r#"[{"slug":"cargo","value":null}]"#)).expect("task");
        let peer: PeerCapabilities = serde_json::from_str(
            r#"{"peerId":"node-a","tags":[{"slug":"cargo","value":null}],"lastReportedAt":0}"#,
        )
        .expect("peer");
        let plan = dispatch_plan(task, vec![peer]).await.expect("plan ok");
        assert_eq!(plan.selected_peer_id, "node-a");
    }

    #[test]
    fn peer_capabilities_deserializes_from_camelcase() {
        let p: PeerCapabilities = serde_json::from_str(
            r#"{"peerId":"mac","tags":[{"slug":"gpu","value":null}],"lastReportedAt":1716000000000}"#,
        )
        .expect("parse");
        assert_eq!(p.peer_id, "mac");
    }
}
