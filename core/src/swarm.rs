//! Cluster-wide parallel prompt fan-out ("swarm").
//!
//! Extracted from `bin/phantom.rs::run_swarm` so both the CLI and the
//! `/rpc/swarm` HTTP endpoint share one implementation. The flow:
//!
//!   1. `refresh_all()` to find online peers.
//!   2. For each online peer call `assign_task_to_peer(agent, prompt)` —
//!      collects `(peer_url, remote_job_id)`.
//!   3. Optionally run the prompt locally via a direct single-shot
//!      provider call (no agent loop, no tools — fast fan-out path).
//!      Skipping local is useful for the HTTP endpoint when the caller
//!      only wants peer results.
//!   4. Poll all remote sub-jobs until done/error or `max_wait_ms`
//!      elapses, whichever first.
//!   5. Return a `SwarmResult` that the caller can serialise into either
//!      pretty CLI output or a JSON blob for the ClusterJobStore.
//!
//! Wire shape returned by `SwarmResult::to_json_string`:
//!
//! ```json
//! {
//!   "local":      "...",                    // null when include_local=false
//!   "local_error": null,
//!   "peers": [
//!     { "url": "http://h:7878", "status": "done",  "output": "...", "error": null },
//!     { "url": "http://h:7878", "status": "error", "output": null,  "error": "..." }
//!   ],
//!   "peer_count": 4
//! }
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::AppState;

/// Single peer's contribution to a swarm fan-out.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerOutput {
    pub url: String,
    /// "done" | "error" | "timeout" | "dispatch_failed"
    pub status: String,
    pub output: Option<String>,
    pub error: Option<String>,
}

/// Aggregate result of a swarm fan-out.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SwarmResult {
    pub local: Option<String>,
    pub local_error: Option<String>,
    pub peers: Vec<PeerOutput>,
    /// Count of online peers we attempted (does not include local).
    pub peer_count: usize,
}

impl SwarmResult {
    pub fn to_json_string(&self) -> String {
        serde_json::to_string(&json!({
            "local":       self.local,
            "local_error": self.local_error,
            "peers":       self.peers,
            "peer_count":  self.peer_count,
        }))
        .unwrap_or_else(|_| "{}".to_string())
    }
}

/// Run the prompt locally with a direct single-shot provider call.
/// No agent loop, no tools — keeps fan-out cheap and avoids burning TPD
/// quota in the multi-iteration agent loop. Mirrors the local path in
/// the original `run_swarm` (commit 89152ee).
pub async fn run_local_single_shot(prompt: &str) -> Result<String, String> {
    use crate::life_node::multimodal::{
        AnalysisInput, Modality, MultimodalProvider, ResponseFormat,
    };
    let input = AnalysisInput {
        modalities: vec![Modality::Text(prompt.to_string())],
        system_prompt: Some("Answer in one short line. Do not use tools.".into()),
        user_prompt: prompt.to_string(),
        max_output_tokens: Some(120),
        response_format: ResponseFormat::PlainText,
        response_schema: None,
    };
    if let Ok(p) = crate::life_node::providers::groq::GroqTextProvider::from_env() {
        return p
            .analyze(input)
            .await
            .map(|r| r.summary)
            .map_err(|e| e.to_string());
    }
    if let Ok(p) = crate::life_node::providers::gemini::GeminiMultimodalProvider::from_env() {
        return p
            .analyze(input)
            .await
            .map(|r| r.summary)
            .map_err(|e| e.to_string());
    }
    Err("no provider available (set GROQ_API_KEY or GEMINI_API_KEY)".to_string())
}

/// Poll all dispatched sub-jobs until each one reports done/error, or
/// `deadline` is reached (any still-pending peer becomes a "timeout"
/// entry). Returns one [`PeerOutput`] per input job in dispatch order.
async fn poll_all_jobs(
    client: &reqwest::Client,
    jobs: Vec<(String, String)>,
    deadline: Instant,
) -> Vec<PeerOutput> {
    use std::collections::HashMap;
    let mut results: HashMap<usize, PeerOutput> = HashMap::new();
    let mut pending: Vec<(usize, String, String)> = jobs
        .iter()
        .enumerate()
        .map(|(i, (u, j))| (i, u.clone(), j.clone()))
        .collect();

    while !pending.is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let mut still_pending = vec![];
        for (idx, peer_url, job_id) in &pending {
            let url = format!(
                "{}/rpc/task/status/{}",
                peer_url.trim_end_matches('/'),
                job_id
            );
            let resp = match client.get(&url).send().await {
                Ok(r) => r,
                Err(_) => {
                    still_pending.push((*idx, peer_url.clone(), job_id.clone()));
                    continue;
                }
            };
            let data: Value = match resp.json().await {
                Ok(v) => v,
                Err(_) => {
                    still_pending.push((*idx, peer_url.clone(), job_id.clone()));
                    continue;
                }
            };
            let status = data["status"].as_str().unwrap_or("unknown");
            if status == "done" || status == "completed" {
                let output = data["output"].as_str().map(|s| s.to_string());
                results.insert(
                    *idx,
                    PeerOutput {
                        url: peer_url.clone(),
                        status: "done".to_string(),
                        output,
                        error: None,
                    },
                );
            } else if status == "error" {
                let err_text = data["error"]
                    .as_str()
                    .unwrap_or("(no error message)")
                    .to_string();
                results.insert(
                    *idx,
                    PeerOutput {
                        url: peer_url.clone(),
                        status: "error".to_string(),
                        output: None,
                        error: Some(err_text),
                    },
                );
            } else {
                still_pending.push((*idx, peer_url.clone(), job_id.clone()));
            }
        }
        pending = still_pending;
    }

    // Anything still pending after deadline is a timeout.
    for (idx, peer_url, job_id) in pending {
        results.insert(
            idx,
            PeerOutput {
                url: peer_url,
                status: "timeout".to_string(),
                output: None,
                error: Some(format!("polling deadline elapsed (job_id={job_id})")),
            },
        );
    }

    // Re-emit in original dispatch order.
    let mut ordered: Vec<PeerOutput> = (0..jobs.len()).filter_map(|i| results.remove(&i)).collect();
    // Fallback: anything we somehow missed.
    ordered.extend(results.into_values());
    ordered
}

/// Core fan-out used by both `phantom swarm` (CLI) and `POST /rpc/swarm`
/// (HTTP). See module docs for the wire shape.
///
/// `max_wait` caps the total time spent polling remote sub-jobs. The
/// local single-shot runs in parallel; if it blocks past `max_wait` the
/// whole call still returns (local_error: "...timeout...").
pub async fn do_swarm(
    state: Arc<AppState>,
    agent: &str,
    prompt: &str,
    include_local: bool,
    max_wait: Duration,
) -> SwarmResult {
    do_swarm_with_throttle(state, agent, prompt, include_local, max_wait, None).await
}

/// Variant of [`do_swarm`] that sleeps `throttle_secs` between successive
/// peer dispatches. CLI uses this to spread Groq TPM (6000) load across
/// peers when fanning a heavy prompt; HTTP callers normally pass `None`.
pub async fn do_swarm_with_throttle(
    state: Arc<AppState>,
    agent: &str,
    prompt: &str,
    include_local: bool,
    max_wait: Duration,
    throttle_secs: Option<u64>,
) -> SwarmResult {
    let cluster = state.cluster_manager.clone();
    let statuses = cluster.refresh_all().await;
    let online_peers: Vec<String> = statuses
        .iter()
        .filter(|s| s.online)
        .map(|s| s.url.clone())
        .collect();

    // Build reqwest client used for status polling. Per-request timeout
    // is short (status calls are tiny); the polling deadline is what
    // caps the whole fan-out.
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return SwarmResult {
                local: None,
                local_error: Some(format!("reqwest client build: {e}")),
                peers: vec![],
                peer_count: online_peers.len(),
            };
        }
    };

    // Dispatch to each peer.
    let mut jobs: Vec<(String, String)> = vec![];
    let mut dispatch_failures: Vec<PeerOutput> = vec![];
    let last_idx = online_peers.len().saturating_sub(1);
    for (idx, peer_url) in online_peers.iter().enumerate() {
        match cluster.assign_task_to_peer(peer_url, agent, prompt).await {
            Ok(job_id) => jobs.push((peer_url.clone(), job_id)),
            Err(e) => dispatch_failures.push(PeerOutput {
                url: peer_url.clone(),
                status: "dispatch_failed".to_string(),
                output: None,
                error: Some(e.to_string()),
            }),
        }
        if let Some(secs) = throttle_secs {
            if idx < last_idx {
                tokio::time::sleep(Duration::from_secs(secs)).await;
            }
        }
    }

    let deadline = Instant::now() + max_wait;
    let prompt_owned = prompt.to_string();

    // Run local single-shot in parallel with polling (when requested).
    let (local_pair, mut peer_outputs) = if include_local {
        let local_fut = async move {
            match tokio::time::timeout(max_wait, run_local_single_shot(&prompt_owned)).await {
                Ok(Ok(s)) => (Some(s), None),
                Ok(Err(e)) => (None, Some(e)),
                Err(_) => (None, Some("local single-shot timed out".to_string())),
            }
        };
        let poll_fut = poll_all_jobs(&client, jobs, deadline);
        let (local_pair, peer_outputs) = tokio::join!(local_fut, poll_fut);
        (local_pair, peer_outputs)
    } else {
        let peer_outputs = poll_all_jobs(&client, jobs, deadline).await;
        ((None, None), peer_outputs)
    };

    // Surface dispatch failures alongside polled results.
    peer_outputs.extend(dispatch_failures);

    let (local, local_error) = local_pair;
    SwarmResult {
        local,
        local_error,
        peers: peer_outputs,
        peer_count: online_peers.len(),
    }
}
