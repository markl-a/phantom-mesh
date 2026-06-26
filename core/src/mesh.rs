//! Cluster peer manager — mesh topology, health tracking, and task routing.
//!
//! This module is the runtime model of the phantom-mesh cluster: the set of
//! reachable peer nodes, how healthy each one is, and how a task or message is
//! routed to the best peer that can run it.
//!
//! # Core abstractions
//!
//! * [`PeerInfo`] — the persistent, on-disk record for one peer (URL, version,
//!   liveness, capabilities, [`PeerHealth`] state). Cached to
//!   `~/.phantom-mesh/peers.json` via [`save_peers`] / [`load_peers`].
//! * [`PeerStatus`] — the lighter wire type returned by `/rpc/ping` and the
//!   cluster status API; derived from a [`PeerInfo`].
//! * [`ClusterManager`] — owns the live peer list behind a lock and drives all
//!   network operations: ping, refresh, heartbeat, HMAC auth, and task assign /
//!   forward / poll.
//! * [`ClusterConfig`] — the `[cluster]` section of `agents.toml`, parsed once
//!   at startup.
//!
//! # Topology discovery
//!
//! Peers can be supplied three ways, in increasing order of dynamism:
//! statically via `[cluster] peers`, by mDNS on the LAN
//! ([`discover_local_peers`]), or over a Tailscale tailnet
//! ([`discover_tailscale_peers`]). A [`coordinator`](ClusterConfig::coordinator)
//! node can also hand out the live peer list at startup.
//!
//! # Routing & capabilities
//!
//! Routing prefers healthy peers with the lowest `consecutive_failures`. When a
//! task declares `required_caps`, only peers advertising the union of those
//! capabilities are eligible ([`peer_has_capabilities`],
//! [`select_best_peer_with_caps`]). Workers can additionally enforce
//! `required_caps` on inbound tasks ([`EnforceMode`], [`enforce_required_caps`])
//! and forward to a capable peer on a mismatch.
//!
//! # Configuration
//!
//! Peers are configured in `agents.toml` under `[cluster]`:
//!
//! ```toml
//! [cluster]
//! peers = ["http://192.168.1.2:7878", "http://100.x.x.5:7878"]
//! cluster_secret = "shared-hmac-key"
//! node_name = "my-node"
//! ```
//!
//! # Tailscale integration (SPEC-10 §6.4)
//!
//! Each [`PeerInfo`] carries an optional `tailscale_ip` populated from
//! `tailscale status --json` (see [`tailscale_status_json`]). When present, task
//! dispatch ([`ClusterManager::assign_task_to_best_peer`]) prefers
//! `http://<ts_ip>:7878` over the raw `peer.url`, giving stable end-to-end
//! encrypted addressing that survives NAT changes and Wi-Fi → cellular handoff.
//! If Tailscale is not installed, the field stays `None` and the LAN/WAN URL is
//! used as before — behaviour for single-network deploys is unchanged.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use uuid::Uuid;

// ── Dispatch error + response types ─────────────────────────────────────────
//
// `DispatchError` distinguishes failure modes of `assign_task_to_best_peer`
// so the CLI can print a useful message instead of a blanket "rejected".
//
// `DispatchResponse` uses `#[serde(flatten)] extra` so new server-side
// response fields (e.g. token counts, model name, trace id) don't require
// editing this code — see SWARM-ARCHITECTURE §4–§6.
//
// We hand-roll `impl Display + Error` rather than pulling in `thiserror`
// just for this module (thiserror is not currently a `core` crate dep).

/// Structured failure modes for cluster task dispatch.
#[derive(Debug)]
pub enum DispatchError {
    /// No online peer was available to take the task.
    NoPeersAvailable,
    /// HTTP-level failure reaching the peer (connection refused, DNS, timeout, ...).
    PeerUnreachable { url: String, source: String },
    /// HMAC authentication was rejected by the peer.
    HMACMismatch { url: String },
    /// Peer accepted the request but returned an `error` field.
    PeerRejected {
        url: String,
        code: Option<String>,
        message: String,
    },
    /// Peer rejected because its local `agents.toml` has no such agent.
    AgentMissing { url: String, agent: String },
    /// Peer accepted the request but did not respond within `elapsed`.
    Timeout {
        url: String,
        elapsed: std::time::Duration,
    },
    /// C1: online peers exist but none advertise the union of `required` caps.
    /// Distinct from `NoPeersAvailable` so the CLI can print actionable hints
    /// ("bring up a peer with worker_caps containing X" vs "no peers at all").
    /// `available_peers` carries the inventory of online (url, worker_caps)
    /// so operators can audit the routing decision.
    NoPeerSatisfiesCaps {
        required: Vec<String>,
        available_peers: Vec<(String, Vec<String>)>,
    },
    /// C1: forward attempt completed but the downstream peer returned non-2xx.
    /// Carries the inner HTTP status + body so the caller can surface them.
    ForwardRejected {
        peer: String,
        status: u16,
        body: String,
    },
    /// C1: forward attempt aborted because the forward chain would cycle —
    /// receiver either saw itself in the chain or the chain hit the hop limit.
    ForwardChainExhausted { chain: Vec<String>, reason: String },
    /// Fallback for anything we didn't model (malformed body, JSON decode error, ...).
    Other(String),
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPeersAvailable => write!(f, "no peers available"),
            Self::PeerUnreachable { url, source } => {
                write!(f, "peer {url} unreachable: {source}")
            }
            Self::HMACMismatch { url } => write!(f, "HMAC mismatch on peer {url}"),
            Self::PeerRejected { url, code, message } => {
                write!(f, "peer {url} rejected task: {message} (code: {code:?})")
            }
            Self::AgentMissing { url, agent } => {
                write!(f, "peer {url} has no agent named '{agent}'")
            }
            Self::Timeout { url, elapsed } => {
                write!(f, "peer {url} timed out after {elapsed:?}")
            }
            Self::NoPeerSatisfiesCaps {
                required,
                available_peers,
            } => {
                write!(
                    f,
                    "no peer satisfies required_caps {:?} (online inventory: {:?})",
                    required, available_peers,
                )
            }
            Self::ForwardRejected { peer, status, body } => {
                write!(f, "forward to {peer} rejected: HTTP {status} body={body}")
            }
            Self::ForwardChainExhausted { chain, reason } => {
                write!(f, "forward chain exhausted ({reason}): chain={chain:?}")
            }
            Self::Other(msg) => write!(f, "dispatch error: {msg}"),
        }
    }
}

impl std::error::Error for DispatchError {}

// ── P1-1: single capability-aware decision line ──────────────────────────────
//
// `select_peer` is the ONE deterministic, pure, injectable function that
// answers "which owned mesh node should run this task". It is a strict
// generalization of `select_best_peer_with_caps` (which becomes a thin
// wrapper). No I/O, no clock — it ranks on integer/string fields already
// materialized on `PeerInfo`, so the same fixture always yields the same pick.

/// Selection-time error taxonomy for `select_peer`. Distinct from the
/// post-dispatch [`DispatchError`]; mapped to it via `impl From` at the call
/// site so the public dispatch signatures keep returning `DispatchError`.
#[derive(Debug)]
pub enum RouteError {
    /// Zero online peers at all (every peer has `online == false`).
    NoPeersAvailable,
    /// Online peers exist, but none advertise the union of `required` caps.
    /// `online_inventory` carries every online peer's `(name, capabilities)`
    /// so operators can audit the routing decision.
    NoCapablePeer {
        required: Vec<String>,
        online_inventory: Vec<(String, Vec<String>)>,
    },
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPeersAvailable => write!(f, "no online peers available"),
            Self::NoCapablePeer {
                required,
                online_inventory,
            } => write!(
                f,
                "no online peer satisfies required_caps {required:?} \
                 (online inventory: {online_inventory:?})"
            ),
        }
    }
}

impl std::error::Error for RouteError {}

/// Map a selection-time `RouteError` onto the live dispatch error taxonomy so
/// the master-dispatch entry points keep their `Result<_, DispatchError>`
/// signatures unchanged.
impl From<RouteError> for DispatchError {
    fn from(e: RouteError) -> Self {
        match e {
            RouteError::NoPeersAvailable => DispatchError::NoPeersAvailable,
            RouteError::NoCapablePeer {
                required,
                online_inventory,
            } => DispatchError::NoPeerSatisfiesCaps {
                required,
                available_peers: online_inventory,
            },
        }
    }
}

/// The chosen peer plus its ordered fallback chain and a human-readable reason.
/// Borrows from the caller's `&[PeerInfo]` (no clone in the hot path).
#[derive(Debug)]
pub struct PeerSelection<'a> {
    /// Best-ranked capable peer to try first.
    pub head: &'a PeerInfo,
    /// Strictly-worse-ranked capable peers, best-first, capped at 2 entries
    /// (SPEC-26 §8 "max 1 reassign" tractability; keeps the walk O(1)).
    pub fallback: Vec<&'a PeerInfo>,
    /// Why `head` was chosen, e.g. "capable(gpu); healthy; load=0; fails=0".
    pub reason: String,
}

/// True when `peer` advertises the union of `required` caps, treating an
/// EMPTY `peer.capabilities` as a full worker (accepts any required) — this
/// matches the pre-existing `select_best_peer_with_caps` full-worker rule.
/// (`peer_has_capabilities` lacks the full-worker shortcut, so we cannot use
/// it verbatim here without regressing the wrapper.)
fn peer_caps_match(peer: &PeerInfo, required: &[String]) -> bool {
    peer.capabilities.is_empty() || peer_has_capabilities(peer, required)
}

/// Strict total-order ranking key for `select_peer`. Smaller tuple = better.
///   1. health tier: Healthy(0) before Unhealthy(1)
///   2. load: fewer `active_tasks` first (ascending)
///   3. reliability: fewer `consecutive_failures` first (ascending)
///   4. recency: larger `last_seen_unix` first (`Reverse` → ascending of
///      reversed = descending of value; fresher ping wins)
///   5. stable tiebreak: lexicographically smaller `name` (determinism guard)
fn rank_key(p: &PeerInfo) -> (u8, u32, u32, std::cmp::Reverse<u64>, &str) {
    let health_tier = if p.health.is_healthy() { 0 } else { 1 };
    (
        health_tier,
        p.active_tasks,
        p.consecutive_failures,
        std::cmp::Reverse(p.last_seen_unix),
        p.name.as_str(),
    )
}

/// Deterministic, capability-aware peer selection. The SINGLE decision line
/// for "which owned mesh node should run this task". Pure: no I/O, no clock.
///
/// Ranking (strict total order, first key dominates):
///   1. Hard filter: `online == true` AND `required ⊆ p.capabilities`
///      (empty `p.capabilities` = full worker, satisfies any `required`;
///      empty `required` = every online peer qualifies).
///   2. Health tier: Healthy peers rank above Unhealthy (fallback only).
///   3. Load: fewer `active_tasks` ranks higher (least-loaded wins).
///   4. Reliability: fewer `consecutive_failures` ranks higher.
///   5. Recency: larger `last_seen_unix` ranks higher (fresher ping).
///   6. Stable tiebreak: lexicographically smaller `name` (deterministic
///      across runs; the same fixture always selects the same peer).
///
/// Returns `PeerSelection { head, fallback, reason }`, or
/// `RouteError::{NoPeersAvailable, NoCapablePeer{..}}` when the filtered set
/// is empty (distinguishing "no peers at all" from "online, none capable").
pub fn select_peer<'a>(
    required: &[String],
    peers: &'a [PeerInfo],
) -> Result<PeerSelection<'a>, RouteError> {
    // 1. No online peers at all → NoPeersAvailable (distinct from NoCapablePeer).
    let online: Vec<&PeerInfo> = peers.iter().filter(|p| p.online).collect();
    if online.is_empty() {
        return Err(RouteError::NoPeersAvailable);
    }

    // 2. Capability hard filter over the online set.
    let mut capable: Vec<&PeerInfo> = online
        .iter()
        .copied()
        .filter(|p| peer_caps_match(p, required))
        .collect();
    if capable.is_empty() {
        return Err(RouteError::NoCapablePeer {
            required: required.to_vec(),
            online_inventory: online
                .iter()
                .map(|p| (p.name.clone(), p.capabilities.clone()))
                .collect(),
        });
    }

    // 3. Rank by the 6-key strict total order. `sort_by_key` over `rank_key`
    //    is total (no float) → fully deterministic down to the name tiebreak.
    capable.sort_by(|a, b| rank_key(a).cmp(&rank_key(b)));

    // 4. Split head + fallback (cap fallback at 2 entries).
    let head = capable[0];
    let fallback: Vec<&PeerInfo> = capable.iter().skip(1).take(2).copied().collect();

    let cap_facts = if required.is_empty() {
        "any".to_string()
    } else {
        required.join(",")
    };
    let health_word = if head.health.is_healthy() {
        "healthy"
    } else {
        "unhealthy"
    };
    let reason = format!(
        "capable({}); {}; load={}; fails={}",
        cap_facts, health_word, head.active_tasks, head.consecutive_failures
    );

    Ok(PeerSelection {
        head,
        fallback,
        reason,
    })
}

/// True for `DispatchError`s where walking to the next peer is pointless or
/// wrong, so the fallback walk must STOP immediately:
///   * `HMACMismatch` — shared `cluster_secret`; every peer rejects the same.
///   * `AgentMissing` — task-definition error, not a peer-health problem.
/// Everything else (`PeerUnreachable`, `Timeout`, `PeerRejected`, `Other`, …)
/// is retryable → try the next-best peer.
fn is_fatal(e: &DispatchError) -> bool {
    matches!(
        e,
        DispatchError::HMACMismatch { .. } | DispatchError::AgentMissing { .. }
    )
}

/// Walk `selection` head→fallback, calling `try_dispatch(peer)` for each until
/// one returns `Ok`. Stops early (no further peers) on a FATAL `DispatchError`
/// (see [`is_fatal`]). On exhaustion returns the LAST observed error, so the
/// user sees a concrete reason rather than a generic "all failed".
///
/// Invariants: bounded (`[head] ++ fallback`, already ≤ 3 by construction);
/// each peer is tried at most once (the ranked list has unique peers).
///
/// This is the pure, injectable form used by unit tests (oracle = closure).
/// The async master-dispatch path mirrors the same head→fallback / fatal-stop
/// / last-error logic with an async per-peer dispatch.
pub fn walk_with_fallback<F>(
    selection: &PeerSelection<'_>,
    mut try_dispatch: F,
) -> Result<String, DispatchError>
where
    F: FnMut(&PeerInfo) -> Result<String, DispatchError>,
{
    let mut last_err: Option<DispatchError> = None;
    for peer in std::iter::once(selection.head).chain(selection.fallback.iter().copied()) {
        match try_dispatch(peer) {
            Ok(out) => return Ok(out),
            Err(e) if is_fatal(&e) => return Err(e),
            Err(e) => last_err = Some(e),
        }
    }
    // Exhausted all candidates → surface the last concrete error. The
    // `unwrap_or` is unreachable in practice: the chain always has ≥1 entry
    // (the head), so at least one iteration ran and set `last_err` (or
    // returned Ok). Kept total for safety.
    Err(last_err.unwrap_or(DispatchError::NoPeersAvailable))
}

/// JSON envelope returned by `/rpc/message`.
///
/// `extra` swallows fields we don't recognize today so server-side additions
/// remain backward compatible — clients only deserialize what they need.
#[derive(Debug, Deserialize)]
struct DispatchResponse {
    output: Option<String>,
    error: Option<String>,
    error_code: Option<String>,
    /// Set by /rpc/task/assign (async dispatch) — the just-created job id.
    /// `None` for /rpc/message (synchronous) responses.
    job_id: Option<String>,
    #[allow(dead_code)] // reserved: future routing observability
    dispatched_to: Option<String>,
    #[serde(flatten)]
    #[allow(dead_code)] // reserved: forward-compat envelope for future fields
    extra: std::collections::HashMap<String, serde_json::Value>,
}

/// Parse a server-side "No agent configuration found (agent 'NAME')..."
/// error into the missing agent name, if it matches.
fn parse_missing_agent(msg: &str) -> Option<String> {
    // Match the exact phrasing produced by `core/src/agent.rs` (`agent.rs:357`).
    let needle = "No agent configuration";
    if !msg.contains(needle) {
        return None;
    }
    // Best-effort: extract NAME from "(agent 'NAME')".
    let start = msg.find("(agent '")? + "(agent '".len();
    let rest = &msg[start..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

// ── Config ─────────────────────────────────────────────────────────────────

// Config loaded from agents.toml [cluster] section
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClusterConfig {
    #[serde(default)]
    pub peers: Vec<String>, // e.g. ["http://100.x.x.2:7878"]
    pub cluster_secret: Option<String>,
    pub node_name: Option<String>,
    /// What this node can do, e.g. ["rust", "python", "analysis", "web"]
    /// Used by evolve --distributed to route tasks to capable nodes.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Sandbox-worker capability subset. Empty (default) = full worker
    /// (this node may run any tool, no restriction). Non-empty = only
    /// tasks whose `required_caps` ⊆ this set are dispatched here.
    /// Standard sandbox set per SPEC-FREEZE-V1 §3:
    /// `["file_in_container","memory","web","subagent","llm_local"]`
    /// Set this on iOS Tauri / Android Tauri APK builds via their
    /// bundled `agents.toml`; leave empty on Mac / Windows / Linux /
    /// Termux full workers.
    #[serde(default)]
    pub worker_caps: Vec<String>,
    /// Trust same-tailnet peers without the cluster HMAC on local-UI endpoints
    /// (e.g. /api/chat). When true, a request whose peer IP is an online
    /// Tailscale peer (from `tailscale status`) is exempt from the cluster HMAC
    /// — WireGuard already authenticated it. Off by default (opt-in);
    /// cluster_secret remains the fallback for non-tailnet peers.
    #[serde(default)]
    pub trust_tailnet_peers: bool,
    /// Optional coordinator URL. When set, this node registers itself on startup
    /// and fetches the live peer list from the coordinator instead of relying on
    /// hardcoded `peers`. e.g. "http://coordinator.example.com:7900"
    pub coordinator: Option<String>,
    /// T5 server-side enforcement mode for /rpc/task/assign. `None`
    /// (default) or `Some(Soft)` preserves pre-T5 behaviour: a
    /// `required_caps` mismatch is logged but the task still runs.
    /// `Some(Strict)` rejects the task with `409 Conflict` and a
    /// structured error body. Can be overridden at runtime via the
    /// `PHANTOM_ENFORCE_REQUIRED_CAPS=soft|strict` env var.
    #[serde(default)]
    pub enforce_caps: Option<EnforceMode>,
    /// C4: how often the heartbeat task probes each peer's `/rpc/ping`.
    /// `None` = use default (30s). Only consulted when the
    /// `experimental-cluster-heartbeat` feature is enabled and the cluster
    /// has at least one configured peer — otherwise the heartbeat task
    /// does not start at all (single-node deployments stay quiet).
    #[serde(default)]
    pub heartbeat_interval_secs: Option<u64>,
    /// C4: how many consecutive probe failures flip a peer from
    /// `Healthy` to `Unhealthy`. `None` = use default (3). Set to 1 for
    /// faster fail-over at the cost of false positives on transient blips.
    #[serde(default)]
    pub heartbeat_failure_threshold: Option<u32>,
}

/// C4: default heartbeat probe interval when `[cluster] heartbeat_interval_secs`
/// is unset. Mirrors the pre-C4 background loop in `bin/phantom.rs`.
pub const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// C4: default consecutive-failure threshold before a peer transitions
/// `Healthy → Unhealthy`. Three matches typical kubelet-style probes —
/// covers a single retry plus one transient packet loss.
pub const DEFAULT_HEARTBEAT_FAILURE_THRESHOLD: u32 = 3;

impl ClusterConfig {
    /// Resolve the effective enforcement mode, considering both the
    /// config field and the `PHANTOM_ENFORCE_REQUIRED_CAPS` env var.
    /// Precedence: env (if set to a recognised value) > config > Soft.
    pub fn effective_enforce_mode(&self) -> EnforceMode {
        if let Ok(raw) = std::env::var("PHANTOM_ENFORCE_REQUIRED_CAPS") {
            match raw.trim().to_ascii_lowercase().as_str() {
                "strict" => return EnforceMode::Strict,
                "soft" => return EnforceMode::Soft,
                _ => { /* fall through to config */ }
            }
        }
        self.enforce_caps.unwrap_or(EnforceMode::Soft)
    }
}

// ── Coordinator wire types ──────────────────────────────────────────────────

/// Payload a node POSTs to a coordinator on startup to join the mesh.
///
/// The coordinator verifies `secret_hash` against its own
/// `SHA-256(cluster_secret)` before accepting the registration, then folds the
/// node into the live peer list it hands back to all members.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorRegistration {
    /// Display name this node advertises (its `[cluster] node_name`).
    pub name: String,
    /// Base URL other peers should dial to reach this node.
    pub url: String,
    /// Capabilities this node declares it can run (its `[cluster] capabilities`).
    pub capabilities: Vec<String>,
    /// `SHA-256(cluster_secret)`, hex encoded — proves shared-secret membership
    /// without sending the secret itself.
    pub secret_hash: String,
}

// ── PeerHealth (C4) ─────────────────────────────────────────────────────────
//
// PeerHealth is the coarse "is this peer worth talking to" bit consumed by
// `select_best_peer_with_caps`. Distinct from `PeerInfo.online` (which simply
// reflects the most recent ping result) — `Unhealthy` requires N consecutive
// failures so a single transient blip does not flip routing decisions.
//
// `Instant` is not `Serialize`, so the disk format is the cheap two-variant
// shape; on load every peer comes back `Healthy` and the heartbeat task
// reconverges within a few intervals if the peer is actually down.

/// C4: routing-relevant health state for a peer.
///
/// `Healthy` is the default for newly-discovered peers (give them a chance
/// to respond before routing skips them). `Unhealthy` carries the `Instant`
/// of the first failure that pushed it over the threshold plus the running
/// `failure_count` — both useful for `phantom peer list` diagnostics.
///
/// Transitions Healthy → Unhealthy emit `tracing::warn!`; Unhealthy → Healthy
/// emit `tracing::info!`. The heartbeat task (gated by the
/// `experimental-cluster-heartbeat` feature) drives transitions via
/// `ClusterManager::record_probe_result`.
// Optimistic default rationale: new peers are `Healthy` so the routing
// filter does not skip them for the first probe interval on a healthy
// mesh. The heartbeat task converges them to `Unhealthy` after the
// configured threshold (`heartbeat_failure_threshold`) of failed pings.
#[derive(Debug, Clone, Default)]
pub enum PeerHealth {
    #[default]
    Healthy,
    Unhealthy {
        /// Wall-clock instant we first marked this peer Unhealthy.
        /// Not serialised — reconstructed on load as the daemon's startup.
        since: Instant,
        /// Consecutive probe failures observed since the last success.
        /// Continues to climb past the threshold so operators can see
        /// "this peer has been gone for hours" in diagnostics.
        failure_count: u32,
    },
}

impl PeerHealth {
    /// True if the peer is in the `Healthy` state.
    pub fn is_healthy(&self) -> bool {
        matches!(self, PeerHealth::Healthy)
    }
}

// Custom Serialize/Deserialize: persist only the discriminant (`"healthy"`
// vs `"unhealthy"`) so the on-disk peers.json stays human-readable and
// forward-compatible. `Instant` cannot be serialised, and re-serialising
// `failure_count` across daemon restarts would be misleading (the count
// reflects in-process probes, not history). On load we always reconstruct
// `Healthy`; the heartbeat task will re-derive Unhealthy if the peer is
// still down. Keeps invariant: cold-start = optimistic.
impl Serialize for PeerHealth {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        match self {
            PeerHealth::Healthy => ser.serialize_str("healthy"),
            PeerHealth::Unhealthy { .. } => ser.serialize_str("unhealthy"),
        }
    }
}

impl<'de> Deserialize<'de> for PeerHealth {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        // Accept anything — old peers.json files predate this field, in
        // which case `#[serde(default)]` on the PeerInfo field saves us.
        // For the "unhealthy" tag we reconstruct with current Instant and
        // failure_count = 0 (the heartbeat task will replenish it).
        let raw = String::deserialize(de)?;
        Ok(match raw.as_str() {
            "unhealthy" => PeerHealth::Unhealthy {
                since: Instant::now(),
                failure_count: 0,
            },
            // "healthy" or anything else: default to Healthy (optimistic).
            _ => PeerHealth::Healthy,
        })
    }
}

// ── PeerInfo ────────────────────────────────────────────────────────────────
//
// PeerInfo is the richer, persistent record stored on disk and used for health
// tracking and routing decisions.  PeerStatus (below) is the lighter wire type
// returned by /rpc/ping and the cluster status API.

/// Persistent peer record with health-tracking fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    /// Base URL of the peer, e.g. "http://192.168.1.2:7878"
    pub url: String,
    /// Human-readable display name reported by the peer.
    pub name: String,
    /// Semantic version string reported by the peer.
    pub version: String,
    /// Whether the last ping succeeded.
    pub online: bool,
    /// Number of active tasks the peer is currently handling.
    pub active_tasks: u32,
    /// Peer uptime in seconds (self-reported).
    pub uptime_secs: u64,
    /// Unix timestamp (seconds) of the last successful ping.
    pub last_seen_unix: u64,
    /// Capabilities declared by this peer (from their agents.toml).
    #[serde(default)]
    pub capabilities: Vec<String>,

    // ── Health-tracking fields ──────────────────────────────────────────
    /// Wall-clock instant of the last successful ping.
    /// Not serialised to disk — reconstructed on load as `None`.
    #[serde(skip)]
    pub last_seen: Option<Instant>,
    /// How many consecutive RPC failures we have seen since the last success.
    pub consecutive_failures: u32,
    /// C4: routing-relevant health state. Drives `select_best_peer_with_caps`
    /// preference ordering (Healthy first, fall through to Unhealthy as
    /// fallback). Defaults to `Healthy` for newly-created peers and on
    /// disk reload — see `PeerHealth::Default` rationale.
    #[serde(default)]
    pub health: PeerHealth,
    /// SPEC-10 §6.4 Tailscale stable address (IPv4) for this peer, when
    /// known. Populated by callers that walk `tailscale status --json`
    /// (see [`tailscale_status_json`]). When `Some`, dispatch helpers
    /// (see [`peer_dispatch_base_url`]) prefer
    /// `http://<tailscale_ip>:7878` over the raw `url`, which survives
    /// Wi-Fi → cellular handoff and NAT-changes without losing the peer.
    /// `None` (default) preserves pre-Tailscale routing — the LAN/WAN
    /// `url` field is used as-is, so single-network deploys see no
    /// behaviour change.
    #[serde(default)]
    pub tailscale_ip: Option<String>,
}

impl PeerInfo {
    /// Returns `true` when the peer has been seen within `timeout_secs` seconds.
    /// A peer that has never been seen (e.g. just loaded from disk) is always
    /// considered unhealthy until a successful ping updates `last_seen`.
    pub fn is_healthy(&self, timeout_secs: u64) -> bool {
        match self.last_seen {
            Some(instant) => instant.elapsed().as_secs() < timeout_secs,
            None => false,
        }
    }
}

// ── Tailscale integration (SPEC-10 §6.4) ────────────────────────────────────
//
// `tailscale_status_json` shells out to `tailscale status --json` and parses
// it into the shape consumed by the rest of mesh.rs (a per-peer IPv4 lookup).
// The CLI invocation is split from the byte-level parse so the parser stays
// pure + fully unit-testable from a fixture.

/// Failure modes for [`tailscale_status_json`]. Kept separate from
/// [`DispatchError`] because Tailscale lookup happens before any dispatch
/// attempt — callers care whether to retry, fall back to LAN, or surface
/// a user-actionable "install tailscale" hint.
#[derive(Debug)]
pub enum MeshError {
    /// `tailscale` CLI is not on `PATH` (or `which`/`exec` failed at the
    /// OS level). Most common cause: Tailscale not installed. Treat as
    /// soft failure — fall back to the LAN/WAN URL.
    TailscaleNotInstalled,
    /// `tailscale status --json` exited non-zero. Stderr is propagated
    /// so the operator can see "Logged out" / "no network" / etc.
    TailscaleCliFailed { exit_code: Option<i32>, stderr: String },
    /// CLI returned bytes that did not parse as `tailscale status --json`.
    /// Usually a version skew between phantom and the tailscale binary.
    JsonParse(String),
}

impl std::fmt::Display for MeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TailscaleNotInstalled => {
                write!(f, "tailscale CLI not installed or not on PATH")
            }
            Self::TailscaleCliFailed { exit_code, stderr } => write!(
                f,
                "tailscale status --json failed (exit={exit_code:?}): {stderr}",
            ),
            Self::JsonParse(msg) => write!(f, "tailscale status --json parse error: {msg}"),
        }
    }
}

impl std::error::Error for MeshError {}

/// Parsed view of `tailscale status --json` keyed by peer hostname.
/// Stores only the fields the mesh actually consumes; unknown fields are
/// ignored on parse (forward-compat with newer Tailscale releases).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TailscaleStatus {
    /// Hostname → first IPv4 address. Hostname is the Tailscale `HostName`
    /// (e.g. "node-a", "mac-mini") with the magicDNS suffix stripped. Only
    /// online peers are included; offline peers and IPv6-only peers are
    /// dropped (phantom dials IPv4:port).
    pub peers: std::collections::BTreeMap<String, String>,
}

impl TailscaleStatus {
    /// Look up a peer's Tailscale IPv4 by hostname (case-insensitive,
    /// trailing `.local.` / magicDNS suffix stripped). Returns `None`
    /// when no entry matches — caller should fall back to LAN/WAN URL.
    pub fn lookup(&self, hostname: &str) -> Option<&str> {
        let normalised = normalise_hostname(hostname);
        self.peers
            .iter()
            .find(|(name, _)| normalise_hostname(name) == normalised)
            .map(|(_, ip)| ip.as_str())
    }
}

fn normalise_hostname(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('.');
    let head = trimmed.split('.').next().unwrap_or(trimmed);
    head.to_ascii_lowercase()
}

/// Pure parser for `tailscale status --json` output. Split from the CLI
/// invocation so tests can feed fixture bytes without depending on a
/// real `tailscale` binary on the runner.
///
/// Mirrors the filter applied by `extract_tailscale_peer_ips` (online +
/// IPv4 only) so both code paths agree on what counts as a reachable
/// peer. Returns `Err(MeshError::JsonParse)` on malformed input.
pub fn parse_tailscale_status_json(bytes: &[u8]) -> Result<TailscaleStatus, MeshError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| MeshError::JsonParse(e.to_string()))?;
    let mut peers = std::collections::BTreeMap::new();
    let Some(peer_obj) = value.get("Peer").and_then(|p| p.as_object()) else {
        return Ok(TailscaleStatus { peers });
    };
    for entry in peer_obj.values() {
        let online = entry
            .get("Online")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !online {
            continue;
        }
        let hostname = entry
            .get("HostName")
            .and_then(|v| v.as_str())
            .map(normalise_hostname);
        let Some(name) = hostname else { continue };
        let Some(addrs) = entry.get("TailscaleIPs").and_then(|v| v.as_array()) else {
            continue;
        };
        let ipv4 = addrs.iter().find_map(|a| {
            a.as_str()
                .filter(|s| !s.contains(':'))
                .map(|s| s.to_string())
        });
        if let Some(ip) = ipv4 {
            peers.insert(name, ip);
        }
    }
    Ok(TailscaleStatus { peers })
}

/// Invoke `tailscale status --json` synchronously and parse the output
/// into a [`TailscaleStatus`]. Uses a blocking [`std::process::Command`]
/// rather than tokio so callers from non-async contexts (e.g. CLI setup,
/// `phantom selftest`) can use it without an executor.
///
/// Returns:
/// * `Err(MeshError::TailscaleNotInstalled)` — CLI missing (most common
///   reason: Tailscale not installed). Caller should fall back to LAN URL.
/// * `Err(MeshError::TailscaleCliFailed { .. })` — CLI present but failed
///   (e.g. logged out, no network).
/// * `Err(MeshError::JsonParse(_))` — CLI succeeded but emitted bytes the
///   parser does not understand (likely a tailscale version skew).
/// * `Ok(TailscaleStatus { peers })` — happy path. `peers` may be empty
///   when the local node is the only one in the tailnet.
pub fn tailscale_status_json() -> Result<TailscaleStatus, MeshError> {
    let output = std::process::Command::new(tailscale_bin())
        .args(["status", "--json"])
        .output()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => MeshError::TailscaleNotInstalled,
            _ => MeshError::TailscaleCliFailed {
                exit_code: None,
                stderr: e.to_string(),
            },
        })?;
    if !output.status.success() {
        return Err(MeshError::TailscaleCliFailed {
            exit_code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    parse_tailscale_status_json(&output.stdout)
}

/// SPEC-10 §6.4: pick the base URL to dial for `peer`. Prefers the
/// stable Tailscale address `http://<tailscale_ip>:7878` when known;
/// otherwise falls back to the LAN/WAN `peer.url` (trailing `/`
/// trimmed so callers can join `/rpc/...` without a double slash).
///
/// Kept a free function so callers outside `ClusterManager` (and tests)
/// can reuse the exact same selection logic.
pub fn peer_dispatch_base_url(peer: &PeerInfo) -> String {
    if let Some(ts_ip) = peer.tailscale_ip.as_deref().map(str::trim) {
        if !ts_ip.is_empty() {
            return format!("http://{}:7878", ts_ip);
        }
    }
    peer.url.trim_end_matches('/').to_string()
}

// ── PeerStatus ──────────────────────────────────────────────────────────────

/// Lightweight wire type returned by /rpc/ping and the cluster status API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerStatus {
    pub url: String,
    pub name: String,
    pub version: String,
    pub online: bool,
    pub active_tasks: u32,
    pub uptime_secs: u64,
    pub last_seen: u64, // unix timestamp
    /// Node capabilities declared in agents.toml [cluster] capabilities = [...]
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Sandbox worker capability subset (Squad Pipeline §11.2/§11.5 of
    /// SPEC-FREEZE-V1). Empty = full worker, may run any tool. Non-empty =
    /// only tasks whose `required_caps` ⊆ this set will be dispatched here.
    /// Standard sandbox set: ["file_in_container","memory","web","subagent","llm_local"].
    #[serde(default)]
    pub worker_caps: Vec<String>,
    /// Agents configured on this peer (keys of `[agent.*]`). Coordinator
    /// uses this to plan Squad Pipeline dispatch — it can't ask node-a to
    /// run "recon-agent" if node-a's agents.toml doesn't have that key.
    /// Populated by the daemon at /rpc/ping time, never persisted.
    #[serde(default)]
    pub agents: Vec<String>,
}

impl From<&PeerInfo> for PeerStatus {
    fn from(p: &PeerInfo) -> Self {
        PeerStatus {
            url: p.url.clone(),
            name: p.name.clone(),
            version: p.version.clone(),
            online: p.online,
            active_tasks: p.active_tasks,
            uptime_secs: p.uptime_secs,
            last_seen: p.last_seen_unix,
            capabilities: p.capabilities.clone(),
            // Cached PeerInfo doesn't track worker_caps / agents — these
            // come from each peer's own /rpc/ping payload, not from the
            // local cache. Default to "full worker, agents unknown".
            worker_caps: Vec::new(),
            agents: Vec::new(),
        }
    }
}

// ── Peer persistence ────────────────────────────────────────────────────────

/// Path to the peers list cache: `~/.phantom-mesh/peers.json`.
fn peers_path() -> Option<std::path::PathBuf> {
    crate::cli_config::phantom_data_dir().ok().map(|d| d.join("peers.json"))
}

/// Save the peer list to `~/.phantom-mesh/peers.json`.
/// Best-effort — errors are logged but not propagated.
pub async fn save_peers(peers: &[PeerInfo]) {
    let path = match peers_path() {
        Some(p) => p,
        None => {
            tracing::warn!("save_peers: could not determine home directory");
            return;
        }
    };

    // Ensure the parent directory exists.
    if let Some(dir) = path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(dir).await {
            tracing::warn!("save_peers: failed to create {:?}: {}", dir, e);
            return;
        }
    }

    let json = match serde_json::to_string_pretty(peers) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("save_peers: serialization failed: {}", e);
            return;
        }
    };

    if let Err(e) = tokio::fs::write(&path, json).await {
        tracing::warn!("save_peers: write to {:?} failed: {}", path, e);
    } else {
        tracing::debug!("save_peers: wrote {} peers to {:?}", peers.len(), path);
    }
}

/// Load the peer list from `~/.phantom-mesh/peers.json`.
/// Returns an empty vec if the file does not exist or cannot be parsed.
pub async fn load_peers() -> Vec<PeerInfo> {
    let path = match peers_path() {
        Some(p) => p,
        None => return vec![],
    };

    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(_) => return vec![],
    };

    match serde_json::from_slice::<Vec<PeerInfo>>(&bytes) {
        Ok(mut peers) => {
            // `last_seen` (Instant) is skipped during deserialization; leave as None
            // so callers know these peers need a fresh ping before being trusted.
            for p in &mut peers {
                p.last_seen = None;
            }
            tracing::debug!("load_peers: loaded {} peers from {:?}", peers.len(), path);
            peers
        }
        Err(e) => {
            tracing::warn!("load_peers: parse error in {:?}: {}", path, e);
            vec![]
        }
    }
}

// ── mDNS peer discovery ─────────────────────────────────────────────────────

/// Shell command used to browse mDNS via `dns-sd` (macOS / Bonjour).
const DNS_SD_BROWSE_CMD: &str = "dns-sd -B _phantom-mesh._tcp local. 2>/dev/null";
/// Shell command used to browse mDNS via `avahi-browse` (Linux).
const AVAHI_BROWSE_CMD: &str = "avahi-browse -t -r -p _phantom-mesh._tcp 2>/dev/null";

/// Whether the `sh`-based mDNS browse pipeline can run on this platform.
///
/// `discover_local_peers` shells out via `sh -c` to `dns-sd` (macOS) or
/// `avahi-browse` (Linux). Windows ships neither `sh` nor those tools, so
/// every spawn would fail (or quietly produce nothing). Callers on Windows
/// must fall back to configured peers — see
/// `docs/backlog/WIN-MESH-DISCOVERY.md` for the planned native implementation.
fn mdns_shell_discovery_supported() -> bool {
    cfg!(unix)
}

/// Extract peer base-URLs from `dns-sd` / `avahi-browse` output.
///
/// Looks for a `url=http…` field anywhere in each line (TXT record field);
/// lines without one are skipped.
fn parse_mdns_urls(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            // Look for a `url=http://…` field anywhere in the line,
            // case-insensitively. We must NOT index `line` with an offset
            // computed from `line.to_lowercase()`: lowercasing can change the
            // byte length (e.g. `İ` U+0130 → `i̇`, 2 bytes → 3 bytes), which
            // shifts every subsequent index and can land mid-char, slicing on a
            // non-char boundary and panicking. Instead scan the original bytes
            // directly so the returned offset is always valid for `line`.
            let needle = b"url=http";
            let bytes = line.as_bytes();
            // ASCII case-insensitive match keeps offsets aligned with `line`;
            // `url=http` is pure ASCII so case folding only affects ASCII bytes.
            let idx = bytes
                .windows(needle.len())
                .position(|w| w.eq_ignore_ascii_case(needle))?;
            let rest = &line[idx + 4..]; // skip "url="
            let end = rest
                .find(|c: char| c.is_whitespace() || c == ';')
                .unwrap_or(rest.len());
            Some(rest[..end].to_string())
        })
        .collect()
}

/// Discover local peers advertised under the `_phantom-mesh._tcp` service type.
///
/// Tries `dns-sd` (macOS / Bonjour) first, then falls back to `avahi-browse`
/// (Linux).  This is a best-effort, fire-and-forget operation — on failure or
/// when neither tool is available the function returns an empty vec.
///
/// On Windows the `sh`-based pipeline is skipped entirely (with an explicit
/// log) and the empty set is returned so callers fall back to configured
/// peers. Tracking spec: `docs/backlog/WIN-MESH-DISCOVERY.md`.
///
/// Returned strings are peer base-URLs inferred from the TXT record `url=…`
/// field.  If no `url=` field is present the entry is skipped.
pub async fn discover_local_peers() -> Vec<String> {
    if !mdns_shell_discovery_supported() {
        tracing::warn!(
            "discover_local_peers: local peer discovery via dns-sd/avahi-browse is \
             not supported on Windows yet — falling back to configured peers"
        );
        return vec![];
    }

    // macOS: dns-sd -B runs forever; spawn it, drain stdout for ~1500ms,
    // then kill the child. This makes the function self-terminating instead
    // of relying on the caller's tokio::time::timeout(...) wrapper.
    {
        use tokio::io::AsyncReadExt;
        let spawned = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(DNS_SD_BROWSE_CMD)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn();

        match spawned {
            Ok(mut child) => {
                let mut buf = Vec::with_capacity(4096);
                if let Some(mut stdout) = child.stdout.take() {
                    // The outer timeout elapsing is expected (dns-sd never
                    // exits on its own); an inner read error is not.
                    if let Ok(Err(e)) = tokio::time::timeout(
                        std::time::Duration::from_millis(1500),
                        stdout.read_to_end(&mut buf),
                    )
                    .await
                    {
                        tracing::debug!("discover_local_peers: dns-sd read error: {e}");
                    }
                }
                // Best-effort kill — child holds the descriptor open otherwise.
                let _ = child.start_kill();
                let _ = child.wait().await;

                let text = String::from_utf8_lossy(&buf);
                let urls = parse_mdns_urls(&text);
                if !urls.is_empty() {
                    tracing::debug!(
                        "discover_local_peers: found {} peers via dns-sd",
                        urls.len()
                    );
                    return urls;
                }
            }
            Err(e) => {
                tracing::debug!("discover_local_peers: failed to spawn dns-sd: {e}");
            }
        }
    }

    // Linux: avahi-browse -t already self-terminates, so a plain output() is fine.
    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(AVAHI_BROWSE_CMD)
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let urls = parse_mdns_urls(&text);
            if !urls.is_empty() {
                tracing::debug!(
                    "discover_local_peers: found {} peers via avahi-browse",
                    urls.len()
                );
                return urls;
            }
        }
        Ok(out) => {
            tracing::debug!(
                "discover_local_peers: avahi-browse exited with {}",
                out.status
            );
        }
        Err(e) => {
            tracing::debug!("discover_local_peers: failed to spawn avahi-browse: {e}");
        }
    }

    tracing::debug!("discover_local_peers: no peers found via mDNS");
    vec![]
}

/// Discover peers on the user's Tailscale tailnet running phantom serve.
///
/// Pipeline:
///   1. `tailscale status --json` — no sudo, requires user to be in a tailnet.
///   2. Collect each online peer's IPv4 TailscaleIP (skip IPv6 — some
///      routers don't reliably forward HTTP over v6 on the tailnet).
///   3. Probe each IP at the standard phantom port for `/info` —
///      `phantom serve` always replies 200 there.
///   4. Return base URLs of peers that responded inside `probe_timeout`.
///
/// Best-effort: returns empty Vec when `tailscale` is missing, the tailnet
/// is in single-machine mode, or no peer answers on either of the probe ports.
///
/// Probes both 7878 (the default) and 7879 (commonly used when 7878 is taken)
/// — these are the two ports the existing cluster nodes use in practice.
pub async fn discover_tailscale_peers() -> Vec<String> {
    discover_tailscale_peers_with(&[7878, 7879], Duration::from_secs(2)).await
}

/// Same as `discover_tailscale_peers` but probes the explicit list of ports
/// with the given per-probe timeout. Used by tests and by callers that
/// know the cluster's ports up front.
pub async fn discover_tailscale_peers_with(ports: &[u16], probe_timeout: Duration) -> Vec<String> {
    let output = tokio::process::Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .await;
    let stdout = match output {
        Ok(o) if o.status.success() => o.stdout,
        Ok(o) => {
            tracing::debug!(
                "discover_tailscale_peers: tailscale CLI returned {}",
                o.status,
            );
            return vec![];
        }
        Err(e) => {
            tracing::debug!("discover_tailscale_peers: tailscale CLI not found: {}", e);
            return vec![];
        }
    };

    let parsed: serde_json::Value = match serde_json::from_slice(&stdout) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("discover_tailscale_peers: bad JSON: {}", e);
            return vec![];
        }
    };

    let ips = extract_tailscale_peer_ips(&parsed);
    if ips.is_empty() {
        tracing::debug!("discover_tailscale_peers: no online tailnet peers reported");
        return vec![];
    }

    let client = match reqwest::Client::builder().timeout(probe_timeout).build() {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    // Cross-product (ip × port) — peers don't advertise their port, so
    // probe each candidate. The first port to answer wins per IP.
    let probes = ips
        .iter()
        .flat_map(|ip| ports.iter().map(move |port| (ip.clone(), *port)))
        .map(|(ip, port)| {
            let client = client.clone();
            async move {
                let url = format!("http://{}:{}/healthz", ip, port);
                match client.get(&url).send().await {
                    Ok(r) if r.status().is_success() => Some(format!("http://{}:{}", ip, port)),
                    _ => None,
                }
            }
        });

    let results: Vec<Option<String>> = futures::future::join_all(probes).await;
    // Dedupe: keep first hit per IP (lowest-port wins).
    let mut seen_ips = std::collections::HashSet::new();
    let mut alive = Vec::new();
    for url in results.into_iter().flatten() {
        if let Some(ip_part) = url
            .strip_prefix("http://")
            .and_then(|s| s.split(':').next())
        {
            if seen_ips.insert(ip_part.to_string()) {
                alive.push(url);
            }
        }
    }
    tracing::debug!(
        "discover_tailscale_peers: {}/{} tailnet peers responding (ports {:?})",
        alive.len(),
        ips.len(),
        ports,
    );
    alive
}

/// Pure-fn extractor — split out for unit tests. Walks the JSON shape
/// that `tailscale status --json` produces, returning IPv4 strings of
/// online peers.
fn extract_tailscale_peer_ips(status: &serde_json::Value) -> Vec<String> {
    let mut ips = Vec::new();
    let Some(peers) = status.get("Peer").and_then(|p| p.as_object()) else {
        return ips;
    };
    for peer in peers.values() {
        let online = peer
            .get("Online")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !online {
            continue;
        }
        let Some(addrs) = peer.get("TailscaleIPs").and_then(|v| v.as_array()) else {
            continue;
        };
        for addr in addrs {
            if let Some(s) = addr.as_str() {
                if !s.contains(':') {
                    ips.push(s.to_string());
                }
            }
        }
    }
    ips
}

/// Resolve the `tailscale` CLI without relying on `$PATH`. The serve runs
/// under launchd whose default PATH is `/usr/bin:/bin:/usr/sbin:/sbin` — it
/// does NOT include `/usr/local/bin` (macOS pkg/Homebrew) where the CLI lives,
/// so a bare `Command::new("tailscale")` fails with NotFound inside the daemon.
/// Probe well-known absolute locations first, fall back to PATH for shell use.
pub fn tailscale_bin() -> &'static str {
    const CANDIDATES: &[&str] = &[
        "/usr/local/bin/tailscale",                            // macOS pkg / Linux local
        "/opt/homebrew/bin/tailscale",                         // macOS arm Homebrew
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale", // macOS App Store
        "/usr/bin/tailscale",                                  // Linux distro
    ];
    for c in CANDIDATES {
        if std::path::Path::new(c).exists() {
            return c;
        }
    }
    "tailscale"
}

/// All online Tailscale peer IPv4s, **un-collapsed by hostname** (unlike
/// [`TailscaleStatus`], which keys peers by hostname and would drop two
/// devices that report the same `HostName`, e.g. both "localhost"). Runs
/// `tailscale status --json` synchronously. Returns an empty vec on any
/// failure (tailscale absent / logged out / parse error) so callers fail
/// closed. Used by the auth gate's tailnet-trust check.
pub fn online_tailnet_peer_ips() -> Vec<String> {
    let output = match std::process::Command::new(tailscale_bin())
        .args(["status", "--json"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    match serde_json::from_slice::<serde_json::Value>(&output.stdout) {
        Ok(v) => extract_tailscale_peer_ips(&v),
        Err(_) => Vec::new(),
    }
}

// ── RPC helper with retry ────────────────────────────────────────────────────

const RPC_MAX_RETRIES: u32 = 2;
const RPC_RETRY_DELAY_MS: u64 = 500;

/// Send a POST request with JSON body to `url`, retrying up to `RPC_MAX_RETRIES`
/// times with a `RPC_RETRY_DELAY_MS` ms delay on connection errors or timeouts.
async fn post_with_retry(
    client: &reqwest::Client,
    url: &str,
    body: String,
    auth_token: Option<&str>,
) -> Result<reqwest::Response, String> {
    let mut last_err = String::new();

    for attempt in 0..=RPC_MAX_RETRIES {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(RPC_RETRY_DELAY_MS)).await;
            tracing::debug!("post_with_retry: attempt {} for {}", attempt + 1, url);
        }

        let mut req = client
            .post(url)
            .header("Content-Type", "application/json")
            .body(body.clone());

        if let Some(token) = auth_token {
            req = req.header("X-Cluster-Auth", token);
        }

        match req.send().await {
            Ok(resp) => return Ok(resp),
            Err(e) => {
                last_err = e.to_string();
                // Only retry on connection / timeout errors.
                if !e.is_connect() && !e.is_timeout() {
                    break;
                }
                tracing::warn!(
                    "post_with_retry: transient error (attempt {}): {}",
                    attempt + 1,
                    e
                );
            }
        }
    }

    Err(last_err)
}

// ── Message routing ─────────────────────────────────────────────────────────

/// Send `message` to the healthiest available peer and return its response.
///
/// Selection policy:
/// 1. Filter to peers where `is_healthy(timeout_secs=30)` is true.
/// 2. Among those, pick the one with the lowest `consecutive_failures`.
/// 3. POST `{"message": message}` to `<peer_url>/rpc/message` with retry.
/// 4. Return the `"output"` field from the JSON response, or `None` on failure.
pub async fn route_to_best_peer(peers: &[PeerInfo], message: &str) -> Option<String> {
    let best = peers
        .iter()
        .filter(|p| p.is_healthy(30))
        .min_by_key(|p| p.consecutive_failures)?;

    let url = format!("{}/rpc/message", best.url.trim_end_matches('/'));
    let body = serde_json::json!({ "message": message }).to_string();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let resp = match post_with_retry(&client, &url, body, None).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(url = %url, "route_to_best_peer: post_with_retry failed: {}", e);
            return None;
        }
    };
    let data: serde_json::Value = match resp.json().await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(url = %url, "route_to_best_peer: json parse failed: {}", e);
            return None;
        }
    };
    data["output"].as_str().map(|s| s.to_string())
}

/// True if `peer` satisfies all `required_caps` (set inclusion: every
/// element of `required_caps` is present in `peer.capabilities`).
///
/// Empty `required_caps` always satisfies — every peer qualifies.
///
/// PF-7: pure helper exposed for callers that need to pre-filter peer
/// lists outside of `route_to_capable_peer` (e.g. UI rendering "which
/// peers can run this task" hints, RPC `peer assign` selection).
pub fn peer_has_capabilities(peer: &PeerInfo, required_caps: &[String]) -> bool {
    required_caps
        .iter()
        .all(|cap| peer.capabilities.iter().any(|p| p == cap))
}

// T-CORE-02: capability-query overlay wire types.
//
// A thin pub/sub style "who can do <caps>?" query computed locally from the
// already-cached peer roster (no extra network I/O). The request travels over
// the existing HTTP REST mesh at POST /rpc/capability-query, authed with the
// same X-Cluster-Auth dual scheme as the other /rpc/* routes.

/// serde default for `CapabilityQueryRequest::include_self`. When a client
/// omits the field we default to true so a bare `{}` body asks "who (including
/// me) can do nothing", which yields every peer plus self.
fn default_true() -> bool {
    true
}

/// Request body for POST /rpc/capability-query.
///
/// `required_caps` is the union of capability slugs a caller needs; an empty
/// list matches every peer (set inclusion of nothing is always satisfied).
/// `include_self` toggles whether the answering node evaluates and includes
/// its own capabilities in the result. Both fields default via serde so a
/// minimal `{}` body deserializes cleanly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityQueryRequest {
    #[serde(default)]
    pub required_caps: Vec<String>,
    #[serde(default = "default_true")]
    pub include_self: bool,
}

/// One matching node in a capability-query answer.
///
/// `self_node` is true only for the answering node's own entry (when
/// `include_self` was set and self satisfied the query); false for every
/// cached peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapablePeerAnswer {
    pub name: String,
    pub url: String,
    pub capabilities: Vec<String>,
    pub online: bool,
    #[serde(default)]
    pub self_node: bool,
}

/// Response body for POST /rpc/capability-query. Echoes the resolved
/// `required_caps`, the matching `answers`, and their `count` for convenience.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityQueryResponse {
    pub required_caps: Vec<String>,
    pub answers: Vec<CapablePeerAnswer>,
    pub count: usize,
}

/// Capability-aware variant of `route_to_best_peer`.
///
/// Filters `peers` to those whose `capabilities` ⊇ `required_caps`,
/// then picks the healthiest peer the same way `route_to_best_peer`
/// does (min consecutive_failures among healthy peers). Returns
/// `None` if no peer matches OR all matching peers are unhealthy.
///
/// Empty `required_caps` falls through to the same selection as
/// `route_to_best_peer` (no filter applied).
///
/// PF-7 from doc 25 §7. Foundation for v0.6.0 V4 (Android worker_caps
/// dispatch filter) + v0.7.0 C1/C2/C3 (cluster RPC capability-aware
/// forwarding). The scheduler now selects by advertised capability
/// instead of hard-coded `target_os` branching (which was already
/// absent from mesh.rs, but this makes the routing intent explicit
/// in the type signature).
pub async fn route_to_capable_peer(
    peers: &[PeerInfo],
    message: &str,
    required_caps: &[String],
) -> Option<String> {
    let best = peers
        .iter()
        .filter(|p| p.is_healthy(30))
        .filter(|p| peer_has_capabilities(p, required_caps))
        .min_by_key(|p| p.consecutive_failures)?;

    let url = format!("{}/rpc/message", best.url.trim_end_matches('/'));
    let body = serde_json::json!({ "message": message }).to_string();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let resp = match post_with_retry(&client, &url, body, None).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(url = %url, "route_to_capable_peer: post_with_retry failed: {}", e);
            return None;
        }
    };
    let data: serde_json::Value = match resp.json().await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(url = %url, "route_to_capable_peer: json parse failed: {}", e);
            return None;
        }
    };
    data["output"].as_str().map(|s| s.to_string())
}

// ── ClusterManager ──────────────────────────────────────────────────────────

/// Owns the live cluster topology and drives all peer network operations.
///
/// Holds the mutable peer list behind an [`RwLock`] (shared cheaply via
/// [`Arc`] — `ClusterManager` is `Clone`) plus a long-timeout HTTP client
/// reused across pings, task assigns, and forwards. Construct one per daemon
/// with [`ClusterManager::new`].
#[derive(Clone)]
pub struct ClusterManager {
    /// Parsed `[cluster]` section from `agents.toml`.
    pub config: ClusterConfig,
    /// Live peer list, shared across clones and updated by ping / heartbeat.
    peers: Arc<RwLock<Vec<PeerInfo>>>,
    /// Shared HTTP client (180s ceiling so cross-mesh `/rpc/message` survives a
    /// remote LLM turn — see [`ClusterManager::new`]).
    client: reqwest::Client,
    /// Daemon start instant, reported as `uptime_secs` in `/rpc/ping` responses.
    start_time: Instant,
}

impl ClusterManager {
    pub fn new(config: ClusterConfig) -> Self {
        let peers = config
            .peers
            .iter()
            .map(|url| PeerInfo {
                url: url.clone(),
                name: url.clone(),
                version: "unknown".into(),
                online: false,
                active_tasks: 0,
                uptime_secs: 0,
                last_seen_unix: 0,
                last_seen: None,
                consecutive_failures: 0,
                capabilities: vec![],
                health: PeerHealth::default(),
                tailscale_ip: None,
            })
            .collect();
        Self {
            config,
            peers: Arc::new(RwLock::new(peers)),
            // 180s ceiling so cross-mesh /rpc/message survives a remote agent
            // run (LLM streaming can take 30-90s for a complex task). Pings
            // hit /rpc/ping which always returns ~instantly so the headroom
            // is harmless for ping_peer; only meaningful for assign / message.
            //
            // If you want a true ping with a tight bound, use the dedicated
            // probe path in `discover_tailscale_peers_with(probe_timeout)`.
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(180))
                .build()
                .unwrap_or_default(),
            start_time: Instant::now(),
        }
    }

    /// Return this node's own status info as a JSON value suitable for /rpc/ping responses.
    pub fn own_info(&self, node_name: &str, version: &str) -> serde_json::Value {
        serde_json::json!({
            "name": node_name,
            "version": version,
            "uptime_secs": self.start_time.elapsed().as_secs(),
            "active_tasks": 0,
            "online": true,
            "capabilities": self.config.capabilities,
        })
    }

    /// Return this node's own PeerStatus, suitable for /rpc/ping serialization.
    /// Uses the configured node_name and the compiled package version.
    pub fn own_peer_status(&self) -> PeerStatus {
        let node_name = self
            .config
            .node_name
            .clone()
            .unwrap_or_else(|| "local".to_string());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        PeerStatus {
            url: String::new(),
            name: node_name,
            version: env!("CARGO_PKG_VERSION").to_string(),
            online: true,
            active_tasks: 0,
            uptime_secs: self.start_time.elapsed().as_secs(),
            last_seen: now,
            capabilities: self.config.capabilities.clone(),
            worker_caps: self.config.worker_caps.clone(),
            // `agents` populated by serve.rs::rpc_ping at request time
            // (it has access to AppState.agent_runtime.config().agent),
            // not from ClusterManager which doesn't see [agent.*] keys.
            agents: Vec::new(),
        }
    }

    /// Kept for backwards compatibility — delegates to `own_peer_status`.
    pub fn own_peer_info(&self) -> PeerStatus {
        self.own_peer_status()
    }

    /// Ping a single peer, update its PeerInfo in the list, and return the result.
    ///
    /// Wrapped in a 5s deadline regardless of `self.client`'s configured
    /// ceiling. The shared client is set to 180s so cross-mesh
    /// `/rpc/message` calls (which can stream a remote LLM turn) survive,
    /// but `/rpc/ping` should never need more than a couple seconds —
    /// without this guard, an offline peer made `phantom peer list` hang
    /// for the full 180s × N peers, which a CLI user reads as "frozen".
    pub async fn ping_peer(&self, url: &str) -> Result<PeerStatus, String> {
        const PING_DEADLINE: Duration = Duration::from_secs(5);
        // A scheme-less or malformed URL otherwise reaches reqwest and surfaces
        // as the opaque "builder error". Catch the common case up front with a
        // message that says what a peer URL should look like.
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(crate::i18n::tr_owned(
                format!("invalid peer URL '{url}' — expected http://host:port or https://host:port"),
                format!("無效的節點 URL '{url}' — 需要 http://host:port 或 https://host:port"),
            ));
        }
        let ping_url = format!("{}/rpc/ping", url.trim_end_matches('/'));
        let body = String::new();
        let result = match tokio::time::timeout(
            PING_DEADLINE,
            post_with_retry(&self.client, &ping_url, body, None),
        )
        .await
        {
            Ok(r) => r,
            Err(_) => Err(format!("ping timeout after {:?}", PING_DEADLINE)),
        };

        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        match result {
            Ok(resp) => {
                let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
                let capabilities = data["capabilities"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let tailscale_ip = data["tailscale_ip"]
                    .as_str()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                let info = PeerInfo {
                    url: url.to_string(),
                    name: data["name"].as_str().unwrap_or(url).to_string(),
                    version: data["version"].as_str().unwrap_or("unknown").to_string(),
                    online: true,
                    active_tasks: data["active_tasks"].as_u64().unwrap_or(0) as u32,
                    uptime_secs: data["uptime_secs"].as_u64().unwrap_or(0),
                    last_seen_unix: now_unix,
                    last_seen: Some(Instant::now()),
                    consecutive_failures: 0,
                    capabilities,
                    // Carried forward in the merge below — `record_probe_result`
                    // decides whether this success flips Unhealthy → Healthy.
                    health: PeerHealth::default(),
                    tailscale_ip,
                };
                let status = PeerStatus::from(&info);
                {
                    let mut peers = self.peers.write().await;
                    if let Some(p) = peers.iter_mut().find(|p| p.url == url) {
                        // Preserve the existing `health` slot so the transition
                        // log emitted by `record_probe_result` below sees the
                        // pre-success state.
                        let prior_health = std::mem::replace(&mut p.health, PeerHealth::Healthy);
                        *p = info;
                        p.health = prior_health;
                    }
                }
                // C4: drive PeerHealth transitions. No-op (just keeps state
                // `Healthy`) when the `experimental-cluster-heartbeat` feature
                // is off, so existing single-node clusters see zero behaviour
                // change.
                self.record_probe_result(url, true).await;
                Ok(status)
            }
            Err(e) => {
                // Record the failure without resetting last_seen.
                {
                    let mut peers = self.peers.write().await;
                    if let Some(p) = peers.iter_mut().find(|p| p.url == url) {
                        p.online = false;
                        p.consecutive_failures = p.consecutive_failures.saturating_add(1);
                    }
                }
                // C4: feed the failure into the health state machine. Gated
                // internally on the feature flag — see `record_probe_result`.
                self.record_probe_result(url, false).await;
                Err(e)
            }
        }
    }

    /// Refresh all configured peers in parallel and return updated status list.
    pub async fn refresh_all(&self) -> Vec<PeerStatus> {
        let futs: Vec<_> = self
            .config
            .peers
            .iter()
            .map(|url| self.ping_peer(url))
            .collect();
        futures::future::join_all(futs).await;
        self.status().await
    }

    /// Return all peers' current cached status (lightweight wire type).
    pub async fn status(&self) -> Vec<PeerStatus> {
        self.peers
            .read()
            .await
            .iter()
            .map(PeerStatus::from)
            .collect()
    }

    /// Return the full PeerInfo records (with health-tracking fields).
    pub async fn peer_infos(&self) -> Vec<PeerInfo> {
        self.peers.read().await.clone()
    }

    /// T-CORE-02: answer a capability query from the locally-cached roster.
    ///
    /// Computes "which nodes can do `required_caps`?" with zero extra network
    /// I/O: it reads the cached peer list via `peer_infos()` and filters with
    /// `peer_has_capabilities`. An empty `required_caps` matches every peer
    /// (set inclusion of nothing). When `include_self` is true the answering
    /// node also evaluates its own advertised capabilities against the same
    /// predicate and, on a match, appends itself with `self_node = true`.
    pub async fn query_capability(
        &self,
        required_caps: &[String],
        include_self: bool,
    ) -> Vec<CapablePeerAnswer> {
        // Read the cached roster only; no ping / refresh is issued here.
        let peers = self.peer_infos().await;
        let me = self.own_peer_status();
        // Exclude any roster entry that is actually THIS node (matched by the
        // advertised node name) so the answering node can never appear twice:
        // once as a cached peer and once as the explicit self answer below
        // (codex review: self de-dupe / self-loop guard). `include_self` then
        // solely controls whether the self answer is appended.
        let mut answers: Vec<CapablePeerAnswer> = peers
            .iter()
            .filter(|peer| peer.name != me.name)
            .filter(|peer| peer_has_capabilities(peer, required_caps))
            .map(|peer| CapablePeerAnswer {
                name: peer.name.clone(),
                url: peer.url.clone(),
                capabilities: peer.capabilities.clone(),
                online: peer.online,
                self_node: false,
            })
            .collect();

        if include_self {
            // own_peer_status() carries this node's capabilities; the predicate
            // operates on a PeerInfo, so build a minimal equivalent to reuse
            // the exact same set-inclusion check as for peers.
            let self_as_peer = PeerInfo {
                url: me.url.clone(),
                name: me.name.clone(),
                version: me.version.clone(),
                online: me.online,
                active_tasks: me.active_tasks,
                uptime_secs: me.uptime_secs,
                last_seen_unix: me.last_seen,
                last_seen: None,
                consecutive_failures: 0,
                capabilities: me.capabilities.clone(),
                health: PeerHealth::default(),
                tailscale_ip: None,
            };
            if peer_has_capabilities(&self_as_peer, required_caps) {
                answers.push(CapablePeerAnswer {
                    name: me.name,
                    url: me.url,
                    capabilities: me.capabilities,
                    online: me.online,
                    self_node: true,
                });
            }
        }

        answers
    }

    // ── C4: heartbeat + health state machine ────────────────────────────────

    /// C4: resolve the effective probe interval from config, falling back
    /// to `DEFAULT_HEARTBEAT_INTERVAL_SECS` when unset. Kept a method so
    /// `spawn_heartbeat_task` callers don't have to mirror the default.
    pub fn effective_heartbeat_interval(&self) -> Duration {
        Duration::from_secs(
            self.config
                .heartbeat_interval_secs
                .unwrap_or(DEFAULT_HEARTBEAT_INTERVAL_SECS),
        )
    }

    /// C4: resolve the effective consecutive-failure threshold from config.
    /// A peer flips Healthy → Unhealthy after this many failures in a row.
    pub fn effective_heartbeat_failure_threshold(&self) -> u32 {
        self.config
            .heartbeat_failure_threshold
            .unwrap_or(DEFAULT_HEARTBEAT_FAILURE_THRESHOLD)
    }

    /// C4: feed one probe outcome into the health state machine.
    /// `success = true` resets failure counters and may flip Unhealthy →
    /// Healthy (emits `tracing::info!`). `success = false` increments the
    /// counter; once it reaches the configured threshold it flips Healthy →
    /// Unhealthy (emits `tracing::warn!`).
    ///
    /// **Feature gate**: when the `experimental-cluster-heartbeat` feature
    /// is OFF (the v0.6.0 default), this function is a no-op as far as
    /// `health` is concerned — peers stay `Healthy` and
    /// `select_best_peer_with_caps`' health filter degenerates to "all
    /// peers are healthy", matching pre-C4 behaviour exactly. The function
    /// still updates `consecutive_failures` because that field predates
    /// C4 and is consumed by `route_to_best_peer`.
    pub async fn record_probe_result(&self, url: &str, success: bool) {
        let threshold = self.effective_heartbeat_failure_threshold();
        let mut peers = self.peers.write().await;
        let Some(peer) = peers.iter_mut().find(|p| p.url == url) else {
            return;
        };

        if success {
            // Always reset the counter on success — meaningful for
            // route_to_best_peer's `min_by_key(consecutive_failures)`.
            peer.consecutive_failures = 0;

            #[cfg(feature = "experimental-cluster-heartbeat")]
            {
                if matches!(peer.health, PeerHealth::Unhealthy { .. }) {
                    tracing::info!(
                        target: "phantom::cluster::heartbeat",
                        event = "peer_health_transition",
                        peer_url = %peer.url,
                        peer_name = %peer.name,
                        from = "unhealthy",
                        to = "healthy",
                        "peer recovered — routing now prefers it again"
                    );
                    peer.health = PeerHealth::Healthy;
                }
            }
        } else {
            // Bump the failure counter regardless of feature gate (existing
            // route_to_best_peer logic uses it as a tiebreaker).
            peer.consecutive_failures = peer.consecutive_failures.saturating_add(1);

            #[cfg(feature = "experimental-cluster-heartbeat")]
            {
                let new_count = peer.consecutive_failures;
                match &mut peer.health {
                    PeerHealth::Healthy => {
                        if new_count >= threshold {
                            tracing::warn!(
                                target: "phantom::cluster::heartbeat",
                                event = "peer_health_transition",
                                peer_url = %peer.url,
                                peer_name = %peer.name,
                                from = "healthy",
                                to = "unhealthy",
                                failure_count = new_count,
                                threshold = threshold,
                                "peer crossed failure threshold — routing will deprioritise"
                            );
                            peer.health = PeerHealth::Unhealthy {
                                since: Instant::now(),
                                failure_count: new_count,
                            };
                        }
                    }
                    PeerHealth::Unhealthy { failure_count, .. } => {
                        *failure_count = new_count;
                    }
                }
            }
            // Suppress the unused-variable warning when the feature is off.
            #[cfg(not(feature = "experimental-cluster-heartbeat"))]
            {
                let _ = threshold;
            }
        }
    }

    /// C4: spawn the background heartbeat task. Returns a `JoinHandle` so
    /// the caller can keep the task alive for the daemon's lifetime (or
    /// drop the handle, which detaches the task — tokio still runs it).
    ///
    /// Returns `None` when:
    ///   * the `experimental-cluster-heartbeat` feature is OFF, OR
    ///   * the cluster has zero configured peers (single-node deployment).
    ///
    /// The pre-C4 background loop in `bin/phantom.rs` already invokes
    /// `refresh_all` on a 30s tick for coordinator-driven topology refresh;
    /// this task is the C4-specific *health* loop and respects the
    /// configurable `heartbeat_interval_secs`. Both loops are safe to run
    /// concurrently — `refresh_all` ultimately funnels into `ping_peer`
    /// which feeds `record_probe_result` exactly once per outcome.
    #[cfg(feature = "experimental-cluster-heartbeat")]
    pub fn spawn_heartbeat_task(
        self: &std::sync::Arc<Self>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        if self.config.peers.is_empty() {
            tracing::debug!(
                target: "phantom::cluster::heartbeat",
                "skipping heartbeat task: cluster has zero peers"
            );
            return None;
        }
        let manager = std::sync::Arc::clone(self);
        let interval_dur = self.effective_heartbeat_interval();
        let threshold = self.effective_heartbeat_failure_threshold();
        tracing::info!(
            target: "phantom::cluster::heartbeat",
            event = "heartbeat_started",
            interval_secs = interval_dur.as_secs(),
            failure_threshold = threshold,
            peer_count = self.config.peers.len(),
            "C4 heartbeat task spawned"
        );
        Some(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval_dur);
            // Skip the immediate first tick so startup isn't bombarded —
            // give the peers a couple of seconds to come online with us.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await; // consume the immediate tick
            loop {
                ticker.tick().await;
                // refresh_all → ping_peer → record_probe_result (success/fail).
                // We deliberately ignore the returned Vec<PeerStatus>; the
                // diagnostic info is already in the per-peer tracing fields
                // emitted by record_probe_result on a state transition.
                // silent-ok: returns Vec (not Result), per-peer telemetry already in record_probe_result.
                let _ = manager.refresh_all().await;
            }
        }))
    }

    /// HMAC-SHA256 auth token: HMAC-SHA256(key=secret, msg=body) encoded as lowercase hex.
    pub fn make_auth_token(&self, body: &str) -> String {
        self.make_auth_token_bytes(body.as_bytes())
    }

    /// Make auth token from raw bytes (used internally for &Bytes compatibility).
    /// Uses real HMAC-SHA256 so it matches `openssl dgst -sha256 -hmac "$SECRET"`.
    /// Panics if `cluster_secret` is not configured — callers must ensure a secret
    /// is set before generating tokens.
    ///
    /// `pub(crate)` (C1): the forwarder in `serve::rpc_task_assign` needs
    /// to re-sign the body before posting to the next hop. Kept module-private
    /// outside the crate so external callers cannot mint tokens.
    pub(crate) fn make_auth_token_bytes(&self, body: &[u8]) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let secret = self
            .config
            .cluster_secret
            .as_deref()
            .filter(|s| !s.is_empty())
            .expect("cluster_secret must be set before generating auth tokens");
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }

    /// Verify an inbound cluster auth token against the expected HMAC-SHA256.
    /// Accepts `&[u8]` so it works with both `&str` and `axum::body::Bytes`.
    /// Returns `false` immediately when no `cluster_secret` is configured.
    /// Uses constant-time comparison (via the `subtle` crate) to prevent timing attacks.
    pub fn verify_auth(&self, token: &str, body: impl AsRef<[u8]>) -> bool {
        use subtle::ConstantTimeEq;
        // Reject all requests when no secret is configured.
        if self
            .config
            .cluster_secret
            .as_deref()
            .map(|s| s.is_empty())
            .unwrap_or(true)
        {
            return false;
        }
        let expected = self.make_auth_token_bytes(body.as_ref());
        bool::from(expected.as_bytes().ct_eq(token.as_bytes()))
    }

    /// Dual-accept inbound cluster auth for the SPEC-10 migration window.
    ///
    /// During the rollout a node must accept BOTH auth schemes so the live
    /// cluster never returns 401 mid-upgrade (the two are NOT wire-compatible):
    ///
    /// * **legacy** — `HMAC-SHA256(secret, raw_body)`, carried in the
    ///   `X-Cluster-Auth` header (this is [`verify_auth`](Self::verify_auth)).
    /// * **SPEC-10** — `HMAC-SHA256(secret, canonical)` where `canonical` is the
    ///   5-part string from [`rpc_wire::build_canonical_string`], carried in the
    ///   `X-Cluster-Auth` header (verified via [`rpc_wire::verify_hmac`]).
    ///
    /// Returns `true` if EITHER scheme verifies. Outbound signing stays legacy
    /// until a coordinated cluster-wide cutover (see T-CORE-01 Stage 3); this
    /// method only widens what we *accept*, so it is safe to ship unilaterally.
    ///
    /// 中文: SPEC-10 遷移期的雙重接受驗證 — 同時收舊版（對 body 簽）與新版
    /// （對 canonical 標準化字串簽）兩種 HMAC，任一通過即放行，避免升級期間
    /// 叢集互相回 401。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn verify_auth_dual(
        &self,
        legacy_token: Option<&str>,
        canonical_sig: Option<&str>,
        method: &str,
        path: &str,
        sorted_query: &str,
        body: &[u8],
        traceparent: Option<&str>,
    ) -> bool {
        // Legacy arm: HMAC over the raw body (X-Cluster-Auth). Reuses the
        // existing constant-time check; also returns false on empty secret.
        if let Some(tok) = legacy_token {
            if self.verify_auth(tok, body) {
                return true;
            }
        }
        // SPEC-10 arm: HMAC over the canonical string (X-Cluster-Auth).
        if let Some(sig) = canonical_sig {
            if let Some(secret) = self
                .config
                .cluster_secret
                .as_deref()
                .filter(|s| !s.is_empty())
            {
                let canonical = crate::rpc_wire::build_canonical_string(
                    method,
                    path,
                    sorted_query,
                    body,
                    traceparent,
                );
                if crate::rpc_wire::verify_hmac(secret.as_bytes(), &canonical, sig).is_ok() {
                    return true;
                }
            }
        }
        false
    }

    /// Forward a task to the least-loaded online peer and return its output.
    /// Calls `/rpc/message` for a synchronous round-trip.
    ///
    /// Returns a structured `DispatchError` rather than `Option` so the CLI
    /// can tell the user *why* the dispatch failed (HMAC mismatch vs missing
    /// agent vs upstream LLM error vs unreachable peer).
    pub async fn assign_task_to_best_peer(
        &self,
        agent: &str,
        prompt: &str,
    ) -> Result<String, DispatchError> {
        // P1-1: route through the single decision line. `required = &[]`
        // preserves the legacy "any online peer" default of this public
        // signature; the WIN is deterministic health+load+name selection
        // (least-loaded healthy peer) instead of the old naive
        // `filter(online).min_by_key(active_tasks)` first-match.
        let peers = self.peer_infos().await;
        let selection = select_peer(&[], &peers).map_err(DispatchError::from)?;

        // Walk head→fallback: on a RETRYABLE failure transparently retry the
        // next-best peer; STOP on a FATAL error (HMAC/AgentMissing); surface
        // the LAST error on exhaustion. Mirrors `walk_with_fallback` but stays
        // async per-peer (the sync helper is the unit-tested invariant oracle).
        let mut last_err: Option<DispatchError> = None;
        for peer in
            std::iter::once(selection.head).chain(selection.fallback.iter().copied())
        {
            match self.post_message_to_peer(peer, agent, prompt).await {
                Ok(out) => return Ok(out),
                Err(e) if is_fatal(&e) => return Err(e),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or(DispatchError::NoPeersAvailable))
    }

    /// Per-peer `/rpc/message` (synchronous round-trip) dispatch with the
    /// legacy raw-body HMAC auth + full error classification. Extracted from
    /// `assign_task_to_best_peer` so the fallback walk can call it per peer.
    async fn post_message_to_peer(
        &self,
        peer: &PeerInfo,
        agent: &str,
        prompt: &str,
    ) -> Result<String, DispatchError> {
        // `task_id` is a client-side stub for future server-side idempotency
        // keys (SWARM-ARCHITECTURE §6). The server ignores it today; including
        // it now lets us roll out server idempotency without a wire-format bump.
        let body = serde_json::json!({
            "message": prompt,
            "agent": agent,
            "task_id": Uuid::new_v4().to_string(),
        })
        .to_string();

        // SPEC-10 §6.4: prefer the Tailscale stable address when known.
        // Falls back to peer.url for LAN-only deploys (tailscale_ip = None).
        let url = format!("{}/rpc/message", peer_dispatch_base_url(peer));
        // SPEC-10 / auth: sign the raw body with the legacy raw-body HMAC the
        // server's `verify_auth` accepts. Without this, the best-peer path sent
        // NO `X-Cluster-Auth` header and the gated `/rpc/message` route returned
        // 401 (surfaced as DispatchError::HMACMismatch). Mirrors the sibling
        // `assign_task_to_peer_full`. Gated on a non-empty cluster_secret so the
        // unauthenticated (secret-not-configured) deployment still works.
        let auth_token = if self
            .config
            .cluster_secret
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
        {
            Some(self.make_auth_token_bytes(body.as_bytes()))
        } else {
            None
        };
        let resp = post_with_retry(&self.client, &url, body, auth_token.as_deref())
            .await
            .map_err(|source| DispatchError::PeerUnreachable {
                url: url.clone(),
                source,
            })?;

        let status = resp.status();
        // HMAC failures surface as 401 on the server (auth middleware in
        // `core/src/main.rs`); flag them distinctly so the user fixes the
        // cluster_secret rather than blaming the agent.
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(DispatchError::HMACMismatch { url: url.clone() });
        }

        let data: DispatchResponse = resp
            .json()
            .await
            .map_err(|e| DispatchError::Other(format!("decode response from {url}: {e}")))?;

        if let Some(err_msg) = data.error {
            // Classify by message content. Server doesn't yet emit machine-
            // readable error codes; once it does, prefer `data.error_code`.
            if let Some(missing) = parse_missing_agent(&err_msg) {
                return Err(DispatchError::AgentMissing {
                    url: url.clone(),
                    agent: missing,
                });
            }
            return Err(DispatchError::PeerRejected {
                url: url.clone(),
                code: data.error_code,
                message: err_msg,
            });
        }

        data.output.ok_or_else(|| {
            DispatchError::Other(format!("peer {url} returned neither output nor error"))
        })
    }

    /// Dispatch a task asynchronously to the best (least-loaded healthy) peer.
    /// Returns the `job_id` for polling via `/rpc/task/status/:id`.
    /// The request is HMAC-auth'd when `cluster_secret` is configured.
    pub async fn assign_task_async(
        &self,
        agent: &str,
        prompt: &str,
    ) -> Result<String, DispatchError> {
        // P1-1: same single-decision-line + fallback-walk as
        // `assign_task_to_best_peer`, over the async `/rpc/task/assign` path.
        let peers = self.peer_infos().await;
        let selection = select_peer(&[], &peers).map_err(DispatchError::from)?;

        let mut last_err: Option<DispatchError> = None;
        for peer in
            std::iter::once(selection.head).chain(selection.fallback.iter().copied())
        {
            // SPEC-10 §6.4: prefer Tailscale stable address when known.
            let url = format!("{}/rpc/task/assign", peer_dispatch_base_url(peer));
            match self.post_task_assign(&url, agent, prompt).await {
                Ok(job_id) => return Ok(job_id),
                Err(e) if is_fatal(&e) => return Err(e),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or(DispatchError::NoPeersAvailable))
    }

    /// Dispatch a task to a specific peer URL (with HMAC auth when configured).
    /// Returns the `job_id` for polling via `/rpc/task/status/:id`, or a
    /// structured `DispatchError` if the peer was unreachable / rejected / etc.
    ///
    /// Mirrors the error classification done by `assign_task_to_best_peer` so
    /// `phantom evolve --distributed` and `phantom peer send-async` surface
    /// the same reasons (HMAC mismatch, missing agent, peer rejection) rather
    /// than a blanket "failed to dispatch".
    pub async fn assign_task_to_peer(
        &self,
        peer_url: &str,
        agent: &str,
        prompt: &str,
    ) -> Result<String, DispatchError> {
        let url = format!("{}/rpc/task/assign", peer_url.trim_end_matches('/'));
        self.post_task_assign(&url, agent, prompt).await
    }

    /// Shared implementation: POST `/rpc/task/assign` with HMAC auth (when
    /// configured), parse a `DispatchResponse`, and classify the failure mode.
    /// `task_id` is included client-side per SWARM-ARCHITECTURE §6 — the
    /// server ignores it today but the wire format won't need a bump when
    /// idempotency keys land server-side.
    async fn post_task_assign(
        &self,
        url: &str,
        agent: &str,
        prompt: &str,
    ) -> Result<String, DispatchError> {
        let body = serde_json::json!({
            "agent": agent,
            "prompt": prompt,
            "task_id": Uuid::new_v4().to_string(),
        })
        .to_string();
        let auth_token = if self
            .config
            .cluster_secret
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
        {
            Some(self.make_auth_token(&body))
        } else {
            None
        };
        let resp = post_with_retry(&self.client, url, body, auth_token.as_deref())
            .await
            .map_err(|source| DispatchError::PeerUnreachable {
                url: url.to_string(),
                source,
            })?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(DispatchError::HMACMismatch {
                url: url.to_string(),
            });
        }

        let data: DispatchResponse = resp
            .json()
            .await
            .map_err(|e| DispatchError::Other(format!("decode response from {url}: {e}")))?;

        if let Some(err_msg) = data.error {
            if let Some(missing) = parse_missing_agent(&err_msg) {
                return Err(DispatchError::AgentMissing {
                    url: url.to_string(),
                    agent: missing,
                });
            }
            return Err(DispatchError::PeerRejected {
                url: url.to_string(),
                code: data.error_code,
                message: err_msg,
            });
        }

        data.job_id.ok_or_else(|| {
            DispatchError::Other(format!("peer {url} returned neither job_id nor error"))
        })
    }

    /// C1: dispatch a `TaskAssignRequest` (with `required_caps`,
    /// `forward_chain`, `idempotency_key`) to a specific peer URL. Used by:
    ///   * `phantom peer assign --required-caps ...` (C2 CLI) — first hop.
    ///   * `forward_task_to_capable_peer` (C3 server) — subsequent hops
    ///     after appending `self` to the chain.
    ///
    /// The wire body is the serialized `TaskAssignRequest` so the new
    /// fields ride through unchanged. HMAC is computed over the exact
    /// canonical bytes (see spec §6).
    pub async fn assign_task_to_peer_full(
        &self,
        peer_url: &str,
        req: &TaskAssignRequest,
    ) -> Result<String, DispatchError> {
        let url = format!("{}/rpc/task/assign", peer_url.trim_end_matches('/'));
        let canonical = serde_json::to_vec(req)
            .map_err(|e| DispatchError::Other(format!("encode TaskAssignRequest: {e}")))?;
        let body_string = String::from_utf8(canonical.clone())
            .map_err(|e| DispatchError::Other(format!("canonical body not utf8: {e}")))?;
        let auth_token = if self
            .config
            .cluster_secret
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
        {
            Some(self.make_auth_token_bytes(&canonical))
        } else {
            None
        };
        let resp = post_with_retry(&self.client, &url, body_string, auth_token.as_deref())
            .await
            .map_err(|source| DispatchError::PeerUnreachable {
                url: url.clone(),
                source,
            })?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(DispatchError::HMACMismatch { url: url.clone() });
        }

        // Capture body for richer error messages on non-2xx (e.g. cycle
        // rejection from a downstream forwarder).
        let raw = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(DispatchError::ForwardRejected {
                peer: url.clone(),
                status: status.as_u16(),
                body: raw,
            });
        }

        let data: DispatchResponse = serde_json::from_str(&raw).map_err(|e| {
            DispatchError::Other(format!("decode response from {url}: {e} (body={raw})"))
        })?;

        if let Some(err_msg) = data.error {
            if let Some(missing) = parse_missing_agent(&err_msg) {
                return Err(DispatchError::AgentMissing {
                    url: url.clone(),
                    agent: missing,
                });
            }
            return Err(DispatchError::PeerRejected {
                url: url.clone(),
                code: data.error_code,
                message: err_msg,
            });
        }

        data.job_id.ok_or_else(|| {
            DispatchError::Other(format!("peer {url} returned neither job_id nor error"))
        })
    }

    /// C1: forward an inbound `TaskAssignRequest` to a peer that satisfies
    /// `required_caps`. Called from `serve::rpc_task_assign` when the local
    /// capability gate returns `CapsDecision::ForwardTo`. The receiver
    /// appends its own `node_name` to the chain before re-signing.
    ///
    /// Cycle guard is done at the call site (in serve.rs) on the **inbound**
    /// chain length so the receiver short-circuits before doing any work.
    /// This function panics in debug builds if called with `forward_chain`
    /// at or above `FORWARD_CHAIN_LIMIT` to catch caller bugs.
    pub async fn forward_task_to_capable_peer(
        &self,
        original: &TaskAssignRequest,
        target: &PeerInfo,
        my_node_name: &str,
    ) -> Result<String, DispatchError> {
        let mut forwarded = original.clone();
        forwarded.forward_chain.push(my_node_name.to_string());
        // ForwardDecision telemetry — paired with the inbound-side warn
        // at the `capability_mismatch` site so log aggregation sees the
        // forward path end-to-end.
        tracing::info!(
            target: "phantom::dispatch::forward",
            event = "forward_decision",
            original_node = %forwarded.forward_chain.first().cloned().unwrap_or_else(|| my_node_name.to_string()),
            current_node = %my_node_name,
            target_node = %target.name,
            target_url = %target.url,
            caps_required = ?original.required_caps,
            chain_length = forwarded.forward_chain.len(),
            "forwarding task — local caps insufficient, downstream peer satisfies"
        );
        self.assign_task_to_peer_full(&target.url, &forwarded).await
    }

    /// Poll a job's result from the given peer.  Returns `(status, output)`.
    pub async fn poll_task(
        &self,
        peer_url: &str,
        job_id: &str,
    ) -> Option<(String, Option<String>)> {
        let url = format!(
            "{}/rpc/task/status/{}",
            peer_url.trim_end_matches('/'),
            job_id
        );
        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(url = %url, job_id = job_id, "poll_task: GET failed: {}", e);
                return None;
            }
        };
        let data: serde_json::Value = match resp.json().await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(url = %url, job_id = job_id, "poll_task: json parse failed: {}", e);
                return None;
            }
        };
        let status = data["status"].as_str()?.to_string();
        let output = data["output"].as_str().map(|s| s.to_string());
        Some((status, output))
    }

    /// Dynamically add a peer URL (from coordinator discovery) if not already known.
    pub async fn add_peer_url(&self, url: &str) {
        let mut peers = self.peers.write().await;
        if !peers.iter().any(|p| p.url == url) {
            peers.push(PeerInfo {
                url: url.to_string(),
                name: url.to_string(),
                version: "unknown".into(),
                online: false,
                active_tasks: 0,
                uptime_secs: 0,
                last_seen_unix: 0,
                last_seen: None,
                consecutive_failures: 0,
                capabilities: vec![],
                health: PeerHealth::default(),
                tailscale_ip: None,
            });
        }
    }

    // ── Coordinator client ──────────────────────────────────────────────────

    fn secret_hash(secret: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        secret.hash(&mut h);
        format!("{:016x}", h.finish())
    }

    /// Register this node with the coordinator and return Ok(()) on success.
    pub async fn register_with_coordinator(&self, self_url: &str) -> Result<(), String> {
        let coord = match &self.config.coordinator {
            Some(c) => c.clone(),
            None => return Ok(()),
        };
        let secret_hash = self
            .config
            .cluster_secret
            .as_deref()
            .map(Self::secret_hash)
            .unwrap_or_default();
        let reg = CoordinatorRegistration {
            name: self
                .config
                .node_name
                .clone()
                .unwrap_or_else(|| "phantom".into()),
            url: self_url.to_string(),
            capabilities: self.config.capabilities.clone(),
            secret_hash,
        };
        let url = format!("{}/register", coord.trim_end_matches('/'));
        self.client
            .post(&url)
            .header("content-type", "application/json")
            .body(serde_json::to_string(&reg).unwrap_or_default())
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Fetch active peers from the coordinator and add any new ones to our peer list.
    /// Returns the number of new peers added.
    pub async fn fetch_coordinator_peers(&self, self_url: &str) -> usize {
        let coord = match &self.config.coordinator {
            Some(c) => c.clone(),
            None => return 0,
        };
        let secret_hash = self
            .config
            .cluster_secret
            .as_deref()
            .map(Self::secret_hash)
            .unwrap_or_default();
        let url = format!(
            "{}/peers?secret_hash={}",
            coord.trim_end_matches('/'),
            secret_hash
        );
        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(_) => return 0,
        };
        let peers: Vec<CoordinatorRegistration> = match resp.json().await {
            Ok(v) => v,
            Err(_) => return 0,
        };
        let mut added = 0;
        for peer in &peers {
            if peer.url != self_url {
                let before = self.peers.read().await.len();
                self.add_peer_url(&peer.url).await;
                // update capabilities from coordinator info right away
                {
                    let mut list = self.peers.write().await;
                    if let Some(p) = list.iter_mut().find(|p| p.url == peer.url) {
                        if p.capabilities.is_empty() {
                            p.capabilities = peer.capabilities.clone();
                            p.name = peer.name.clone();
                        }
                    }
                }
                if self.peers.read().await.len() > before {
                    added += 1;
                }
            }
        }
        added
    }
}

impl Default for ClusterManager {
    fn default() -> Self {
        Self::new(ClusterConfig::default())
    }
}

// ── Task assign request/response ───────────────────────────────────────────

/// Inbound payload for POST /rpc/task/assign.
///
/// `Serialize` is derived so the C1 forwarder can re-emit the body verbatim
/// (minus the field-order-stable struct shape) and HMAC-sign it for the
/// next hop — see `ClusterManager::forward_task_to_capable_peer`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TaskAssignRequest {
    /// Name of the agent (a `[agent.*]` key) to run on the worker. Defaults to
    /// `"master"` when the client omits it.
    #[serde(default = "default_agent")]
    pub agent: String,
    /// The prompt / instruction text to hand to the chosen agent.
    pub prompt: String,
    /// Capability tags the task needs from the worker. Server-side
    /// enforcement (M1 defense-in-depth, track T5): if non-empty, the
    /// worker rejects in strict mode and warns in soft mode when any
    /// of these is missing from the worker's local `worker_caps`.
    /// Old clients omitting this field land here as `vec![]` =
    /// no restriction.
    #[serde(default)]
    pub required_caps: Vec<String>,
    /// C1: node_names this request has already transited. Empty on the
    /// first hop. Receiving node appends `self.config.node_name` before
    /// forwarding. Cycle guard rejects when `len() >= FORWARD_CHAIN_LIMIT`
    /// or when `self` is already in the chain.
    /// Old clients omitting this field land here as `vec![]` = first hop.
    #[serde(default)]
    pub forward_chain: Vec<String>,
    /// C1: caller-supplied stable key for at-most-once semantics. The
    /// forwarder preserves this byte-for-byte so the next-hop receiver
    /// can dedupe (planned for v0.7.0). Old clients omitting this field
    /// land here as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

fn default_agent() -> String {
    "master".into()
}

// ── Server-side dispatch capability enforcement (T5) ──────────────────────
//
// Defense-in-depth on the worker side of `/rpc/task/assign`. The M1
// client-side dispatch filter (`select_best_peer_with_caps`) already
// avoids sending tasks to workers that can't handle them, but a buggy
// or malicious peer holding the cluster_secret could still POST a task
// with `required_caps` this worker doesn't advertise. These types let
// the worker recognise that and (optionally) refuse.

/// How aggressively a worker enforces `required_caps` on inbound tasks.
///
/// `Soft` (default) preserves pre-T5 behaviour: the mismatch is
/// `tracing::warn!`'d but the task still runs — so old clusters keep
/// working unchanged.
///
/// `Strict` returns `409 Conflict` with a structured error body. Opt
/// in via `[cluster] enforce_caps = "strict"` in `agents.toml` or via
/// `PHANTOM_ENFORCE_REQUIRED_CAPS=strict` (env wins, see
/// `ClusterConfig::effective_enforce_mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnforceMode {
    /// Log a `tracing::warn!` on a capability mismatch but still run the task.
    #[default]
    Soft,
    /// Reject a capability mismatch with `409 Conflict` and a structured body.
    Strict,
}

/// Outcome of the capability check. `Allow` = run the task; `LogAndAllow`
/// = run it but emit a warning; `Reject` = bounce with 409 and surface
/// `missing` to the caller; `ForwardTo` (C1) = a peer in `peers.json`
/// satisfies the caps — forward the task there. The fourth arm is only
/// returned by `enforce_required_caps_with_forwarding` and only when
/// `PHANTOM_FORWARD_ON_CAPS_MISMATCH=1`.
///
/// `PartialEq`/`Eq` are intentionally NOT derived: `ForwardTo` carries a
/// `PeerInfo` which would need its own equality impl that strips
/// `last_seen: Instant` (not comparable across snapshots). Tests use
/// `matches!` + field destructure instead.
#[derive(Debug, Clone)]
pub enum CapsDecision {
    /// Worker satisfies all required caps — run the task.
    Allow,
    /// Soft mode: caps are `missing` but run anyway after a warning.
    LogAndAllow {
        /// Required caps this worker does not advertise.
        missing: Vec<String>,
    },
    /// Strict mode: bounce with 409 and surface `missing` to the caller.
    Reject {
        /// Required caps this worker does not advertise.
        missing: Vec<String>,
    },
    /// C1: missing locally, but a peer satisfies. Gated by
    /// `PHANTOM_FORWARD_ON_CAPS_MISMATCH=1` (default OFF) so existing
    /// single-node deployments see zero behaviour change.
    ForwardTo {
        peer: PeerInfo,
        missing: Vec<String>,
    },
}

/// C1: maximum forward hops permitted before the receiver short-circuits
/// with `cycle_detected`. Limit of 2 means the worst-case fan-out is 3
/// nodes total (origin + 2 forwarders). Spec §5 deliberately caps small
/// to limit blast radius of any forwarding bug; can be raised later once
/// real telemetry validates the path.
pub const FORWARD_CHAIN_LIMIT: usize = 2;

/// Read `PHANTOM_FORWARD_ON_CAPS_MISMATCH` at request time (not process
/// start) so integration tests can toggle the gate per-test by setting and
/// unsetting the env var around an `app.oneshot` call.
pub fn forward_on_caps_mismatch_enabled() -> bool {
    std::env::var("PHANTOM_FORWARD_ON_CAPS_MISMATCH")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Pure decision function: what to do with `required_caps` vs the
/// worker's advertised `worker_caps`, under the given mode.
///
/// Rules (matches the M1 client-side filter so client/server agree):
/// 1. `local.is_empty()` ⇒ full worker, accept anything → `Allow`.
/// 2. `required.is_empty()` ⇒ no restriction → `Allow`.
/// 3. `required ⊆ local` ⇒ `Allow`.
/// 4. otherwise compute `missing = required \ local` (order-preserving,
///    deduplicated) and emit `Reject` (strict) or `LogAndAllow` (soft).
pub fn enforce_required_caps(
    local: &[String],
    required: &[String],
    mode: EnforceMode,
) -> CapsDecision {
    // Rule 1: full worker
    if local.is_empty() {
        return CapsDecision::Allow;
    }
    // Rule 2: no restriction in request
    if required.is_empty() {
        return CapsDecision::Allow;
    }
    // Compute missing = required \ local, preserving first-seen order
    // and deduplicating so callers don't see "shell, shell" in errors.
    let mut missing: Vec<String> = Vec::new();
    for cap in required {
        if !local.iter().any(|l| l == cap) && !missing.iter().any(|m| m == cap) {
            missing.push(cap.clone());
        }
    }
    if missing.is_empty() {
        CapsDecision::Allow
    } else {
        match mode {
            EnforceMode::Strict => CapsDecision::Reject { missing },
            EnforceMode::Soft => CapsDecision::LogAndAllow { missing },
        }
    }
}

/// C1: caps-aware sibling of `enforce_required_caps`. When the env gate
/// `PHANTOM_FORWARD_ON_CAPS_MISMATCH=1` is set AND a peer in `peers` satisfies
/// the missing caps, returns `CapsDecision::ForwardTo { peer, missing }`
/// instead of the usual `Reject`/`LogAndAllow`. Otherwise delegates verbatim
/// to `enforce_required_caps` so soft-mode/strict-mode/full-worker behaviour
/// is unchanged when the gate is off — which is the default.
///
/// Kept as a sibling rather than replacing `enforce_required_caps` because
/// the existing pure function is used by client-side dispatch filters and
/// unit tests that should not need to thread a peer list through.
pub fn enforce_required_caps_with_forwarding(
    local: &[String],
    required: &[String],
    mode: EnforceMode,
    peers: &[PeerInfo],
) -> CapsDecision {
    // Fast path: behaviour unchanged from `enforce_required_caps` when
    // local satisfies (Allow) or required is empty (Allow). Only mismatch
    // cases consider forwarding.
    let base = enforce_required_caps(local, required, mode);
    match base {
        CapsDecision::Allow => CapsDecision::Allow,
        // Forwarding only kicks in when the env gate is set AND a peer can
        // satisfy. Otherwise we keep the original decision (Reject/LogAndAllow)
        // so misconfigured operators are not silently routed around.
        CapsDecision::Reject { ref missing } | CapsDecision::LogAndAllow { ref missing } => {
            if !forward_on_caps_mismatch_enabled() {
                return base;
            }
            match select_best_peer_with_caps(required, peers) {
                Some(peer) => CapsDecision::ForwardTo {
                    peer,
                    missing: missing.clone(),
                },
                None => base,
            }
        }
        CapsDecision::ForwardTo { .. } => base,
    }
}

/// C1: pick the best online peer whose `capabilities` superset `required`.
/// Returns `None` when no peer satisfies — the caller distinguishes between
/// "no peers at all" (`NoPeersAvailable`) and "online peers exist, none
/// satisfy" (`NoPeerSatisfiesCaps`) via the error taxonomy.
///
/// P1-1: this is now a THIN WRAPPER over the single decision line
/// [`select_peer`]. The ranking, capability hard-filter (incl. the
/// empty-caps=full-worker rule), and Healthy-before-Unhealthy tiering all
/// live in `select_peer`'s `rank_key`. Two behaviour deltas vs. the old
/// hand-rolled body, both intentional:
///   * Load now breaks ties toward the LEAST-loaded peer (the old body's
///     `b.active_tasks.cmp(&a.active_tasks)` preferred the MORE-loaded peer —
///     a latent bug; see P1-1 plan). No caller depended on the old order:
///     the only consumer is `enforce_required_caps_with_forwarding`, which
///     just wants *a* capable peer.
///   * Selection is a strict total order down to the `name` tiebreak, so the
///     pick is deterministic across runs.
///
/// Note on `capabilities` vs `worker_caps` (spec §14 Q1): `PeerInfo` only
/// persists `capabilities` (general node-capability tags from each peer's
/// agents.toml). The richer `worker_caps` sandbox subset lives only on
/// `PeerStatus` returned by `/rpc/ping` and is not cached on disk. For V1
/// we treat `capabilities` as the routing key.
pub fn select_best_peer_with_caps(required: &[String], peers: &[PeerInfo]) -> Option<PeerInfo> {
    select_peer(required, peers).ok().map(|sel| sel.head.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn peer_with_caps(name: &str, caps: &[&str]) -> PeerInfo {
        PeerInfo {
            url: format!("http://{name}:7878"),
            name: name.into(),
            version: "0.6.0".into(),
            online: true,
            active_tasks: 0,
            uptime_secs: 60,
            last_seen_unix: 1_700_000_000,
            last_seen: None,
            consecutive_failures: 0,
            capabilities: caps.iter().map(|s| s.to_string()).collect(),
            health: PeerHealth::default(),
            tailscale_ip: None,
        }
    }

    // ── PF-7: peer_has_capabilities ────────────────────────────────────────

    #[test]
    fn peer_has_capabilities_empty_required_always_matches() {
        let p = peer_with_caps("node-a", &["shell", "browser"]);
        assert!(
            peer_has_capabilities(&p, &[]),
            "empty required_caps must match every peer"
        );
    }

    #[test]
    fn peer_has_capabilities_single_required_matches_when_present() {
        let p = peer_with_caps("node-a", &["shell", "browser", "gpu_compute:cuda"]);
        assert!(peer_has_capabilities(&p, &["browser".into()]));
        assert!(peer_has_capabilities(&p, &["gpu_compute:cuda".into()]));
    }

    #[test]
    fn peer_has_capabilities_single_required_misses_when_absent() {
        let p = peer_with_caps("node-a", &["shell", "browser"]);
        assert!(!peer_has_capabilities(&p, &["camera".into()]));
        assert!(!peer_has_capabilities(&p, &["gpu_compute:metal".into()]));
    }

    #[test]
    fn peer_has_capabilities_multi_required_is_set_inclusion() {
        let p = peer_with_caps("m1", &["shell", "gpu_compute:metal", "local_llm:mlx"]);
        // All required present → match
        assert!(peer_has_capabilities(
            &p,
            &["shell".into(), "gpu_compute:metal".into()]
        ));
        // One missing → miss
        assert!(!peer_has_capabilities(
            &p,
            &["shell".into(), "camera".into()]
        ));
    }

    #[test]
    fn peer_has_capabilities_peer_with_no_caps_matches_only_empty() {
        let p = peer_with_caps("legacy", &[]);
        assert!(peer_has_capabilities(&p, &[]), "empty/empty matches");
        assert!(
            !peer_has_capabilities(&p, &["shell".into()]),
            "empty peer caps can't satisfy any non-empty requirement"
        );
    }

    // ── PF-7: capability-aware filtering selects correct peer ──────────────

    #[test]
    fn cap_filter_picks_only_capable_peers() {
        // Simulate the inline filter used by route_to_capable_peer.
        let peers = vec![
            peer_with_caps("node-a", &["shell", "browser"]),
            peer_with_caps("m1", &["shell", "gpu_compute:metal", "local_llm:mlx"]),
            peer_with_caps("node-b", &["shell", "browser", "camera"]),
        ];
        let required = vec!["camera".to_string()];
        let matches: Vec<&str> = peers
            .iter()
            .filter(|p| peer_has_capabilities(p, &required))
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(matches, vec!["node-b"], "only node-b has camera");

        let required = vec!["shell".to_string()];
        let matches: Vec<&str> = peers
            .iter()
            .filter(|p| peer_has_capabilities(p, &required))
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(matches, vec!["node-a", "m1", "node-b"]);
    }

    /// MAC P0 — when a request requires the `local_llm:mlx` capability,
    /// the dispatcher's peer filter must surface only peers that advertise
    /// it. This guards the production routing path that sends MLX-bound
    /// inference to Apple Silicon peers and never to e.g. a Win/Linux box
    /// where mlx_lm.server cannot run.
    #[test]
    fn dispatcher_picks_mlx_when_cap_matches() {
        let peers = vec![
            peer_with_caps("node-a", &["shell", "browser"]), // Win, no MLX
            peer_with_caps("m1", &["shell", "gpu_compute:metal", "local_llm:mlx"]),
            peer_with_caps("node-b", &["shell", "browser", "camera"]), // Win, no MLX
            peer_with_caps("yoyo", &["shell"]),                      // Linux, no MLX
        ];
        let required = vec!["local_llm:mlx".to_string()];

        let picked: Vec<&str> = peers
            .iter()
            .filter(|p| peer_has_capabilities(p, &required))
            .map(|p| p.name.as_str())
            .collect();

        assert_eq!(
            picked,
            vec!["m1"],
            "only m1 advertises local_llm:mlx; dispatcher must NOT route \
             MLX inference to non-Apple-Silicon peers"
        );

        // Inverse: a peer that has `local_llm:mlx` plus other caps must
        // still match. Defends against an over-strict equality match.
        let required = vec!["local_llm:mlx".to_string()];
        let m1_only = peer_with_caps("m1", &["local_llm:mlx", "gpu_compute:metal", "shell"]);
        assert!(
            peer_has_capabilities(&m1_only, &required),
            "peer with caps superset of the requirement must still match"
        );

        // Multi-cap request: both `local_llm:mlx` AND `gpu_compute:metal`
        // — m1 has both, others have neither.
        let required = vec!["local_llm:mlx".to_string(), "gpu_compute:metal".to_string()];
        let picked: Vec<&str> = peers
            .iter()
            .filter(|p| peer_has_capabilities(p, &required))
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(
            picked,
            vec!["m1"],
            "combined-cap requirement still picks the one peer that has both"
        );
    }

    #[test]
    fn dispatch_response_missing_agent_classifies_correctly() {
        // Server returns an error body shaped like `agent.rs:357` produces.
        let body = json!({
            "error": "No agent configuration found (agent 'master'). Check agents.toml."
        });
        let parsed: DispatchResponse = serde_json::from_value(body).expect("parse");
        let err_msg = parsed.error.expect("has error");
        let missing = parse_missing_agent(&err_msg).expect("matched missing-agent regex");
        assert_eq!(missing, "master");

        // And: the classifier itself, when wrapped into a DispatchError,
        // round-trips through Display sensibly.
        let de = DispatchError::AgentMissing {
            url: "http://example:7878".into(),
            agent: missing,
        };
        let rendered = format!("{de}");
        assert!(rendered.contains("'master'"), "got {rendered}");
        assert!(rendered.contains("example"), "got {rendered}");
    }

    #[test]
    fn dispatch_response_parses_assign_task_error_path() {
        // assign_task_to_peer's structured-error path: server returned an
        // error envelope from /rpc/task/assign (e.g. missing agent on peer).
        // Before this fix, the old `data["job_id"].as_str()` would silently
        // return None and the caller printed a blanket "failed to dispatch".
        // Now the error must round-trip into the structured DispatchError.
        let body = json!({
            "error": "No agent configuration found (agent 'reviewer'). Check agents.toml.",
            "error_code": "agent_missing",
        });
        let parsed: DispatchResponse = serde_json::from_value(body).expect("parse");
        assert!(
            parsed.job_id.is_none(),
            "error envelope must not contain job_id"
        );
        let err_msg = parsed.error.expect("has error");
        assert_eq!(parse_missing_agent(&err_msg).as_deref(), Some("reviewer"));
        assert_eq!(parsed.error_code.as_deref(), Some("agent_missing"));

        // And the success path still extracts job_id correctly.
        let ok_body = json!({ "job_id": "11111111-2222-3333-4444-555555555555" });
        let ok: DispatchResponse = serde_json::from_value(ok_body).expect("parse");
        assert_eq!(
            ok.job_id.as_deref(),
            Some("11111111-2222-3333-4444-555555555555"),
        );
        assert!(ok.error.is_none());
    }

    #[test]
    fn dispatch_response_extra_fields_ignored() {
        // Forward-compat: unknown server fields should not break decode.
        let body = json!({
            "output": "hello",
            "future_field": { "anything": 1 },
            "tokens_used": 42
        });
        let parsed: DispatchResponse = serde_json::from_value(body).expect("parse");
        assert_eq!(parsed.output.as_deref(), Some("hello"));
        assert!(parsed.error.is_none());
        assert!(parsed.extra.contains_key("future_field"));
        assert!(parsed.extra.contains_key("tokens_used"));
    }

    #[test]
    fn extract_tailscale_peer_ips_filters_offline_and_ipv6() {
        let status = json!({
            "Self": {
                "TailscaleIPs": ["100.64.0.1", "fd7a:115c:a1e0::1"],
                "Online": true
            },
            "Peer": {
                "node-a": {
                    "Online": true,
                    "TailscaleIPs": ["100.64.0.2", "fd7a:115c:a1e0::2"]
                },
                "node-b-offline": {
                    "Online": false,
                    "TailscaleIPs": ["100.64.0.3"]
                },
                "node-c": {
                    "Online": true,
                    "TailscaleIPs": ["100.64.0.4"]
                },
                "node-d-no-online-field": {
                    "TailscaleIPs": ["100.64.0.5"]
                }
            }
        });
        let ips = extract_tailscale_peer_ips(&status);
        // Self is intentionally excluded; only Peer.* online ipv4.
        assert_eq!(ips, vec!["100.64.0.2", "100.64.0.4"]);
    }

    #[test]
    fn extract_tailscale_peer_ips_handles_missing_peer_object() {
        let status = json!({ "Self": { "TailscaleIPs": ["100.64.0.1"] } });
        assert!(extract_tailscale_peer_ips(&status).is_empty());
    }

    #[test]
    fn extract_tailscale_peer_ips_handles_empty_tailnet() {
        let status = json!({ "Peer": {} });
        assert!(extract_tailscale_peer_ips(&status).is_empty());
    }

    #[test]
    fn extract_tailscale_peer_ips_skips_peers_with_no_ips_array() {
        let status = json!({
            "Peer": {
                "weird-node": { "Online": true /* no TailscaleIPs */ }
            }
        });
        assert!(extract_tailscale_peer_ips(&status).is_empty());
    }

    #[test]
    fn task_assign_request_parses_required_caps() {
        let body = json!({
            "agent": "master",
            "prompt": "hello",
            "required_caps": ["file_in_container", "web"],
        });
        let req: TaskAssignRequest = serde_json::from_value(body).expect("parse");
        assert_eq!(req.agent, "master");
        assert_eq!(req.prompt, "hello");
        assert_eq!(req.required_caps, vec!["file_in_container", "web"]);
    }

    #[test]
    fn task_assign_request_required_caps_defaults_to_empty() {
        // Backwards-compat: old clients that omit required_caps must
        // still deserialize, and required_caps must default to [].
        let body = json!({ "prompt": "hi" });
        let req: TaskAssignRequest = serde_json::from_value(body).expect("parse");
        assert_eq!(req.agent, "master");
        assert!(req.required_caps.is_empty());
    }

    #[test]
    fn enforce_full_worker_allows_anything() {
        // Empty local worker_caps = "full worker, no restriction" per
        // ClusterConfig::worker_caps doc.
        let local: Vec<String> = vec![];
        let required = vec!["file_in_container".to_string(), "shell".to_string()];

        // Even in strict mode, a full worker accepts everything.
        let d = enforce_required_caps(&local, &required, EnforceMode::Strict);
        assert!(matches!(d, CapsDecision::Allow), "got {d:?}");

        let d = enforce_required_caps(&local, &required, EnforceMode::Soft);
        assert!(matches!(d, CapsDecision::Allow), "got {d:?}");
    }

    #[test]
    fn enforce_empty_required_caps_always_allows() {
        // No required_caps in the request = no restriction. Even a
        // tight sandbox worker accepts. Soft and strict identical.
        let local = vec!["file_in_container".to_string()];
        let required: Vec<String> = vec![];
        assert!(matches!(
            enforce_required_caps(&local, &required, EnforceMode::Strict),
            CapsDecision::Allow
        ));
        assert!(matches!(
            enforce_required_caps(&local, &required, EnforceMode::Soft),
            CapsDecision::Allow
        ));
    }

    #[test]
    fn enforce_strict_rejects_missing_caps() {
        let local = vec!["file_in_container".to_string(), "memory".to_string()];
        let required = vec!["file_in_container".to_string(), "shell".to_string()];
        match enforce_required_caps(&local, &required, EnforceMode::Strict) {
            CapsDecision::Reject { missing } => {
                assert_eq!(missing, vec!["shell".to_string()]);
            }
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[test]
    fn enforce_strict_allows_subset_match() {
        let local = vec![
            "file_in_container".to_string(),
            "memory".to_string(),
            "web".to_string(),
        ];
        let required = vec!["file_in_container".to_string(), "memory".to_string()];
        assert!(matches!(
            enforce_required_caps(&local, &required, EnforceMode::Strict),
            CapsDecision::Allow
        ));
    }

    #[test]
    fn enforce_soft_logs_and_allows_on_mismatch() {
        let local = vec!["file_in_container".to_string()];
        let required = vec!["file_in_container".to_string(), "shell".to_string()];
        match enforce_required_caps(&local, &required, EnforceMode::Soft) {
            CapsDecision::LogAndAllow { missing } => {
                assert_eq!(missing, vec!["shell".to_string()]);
            }
            other => panic!("expected LogAndAllow, got {other:?}"),
        }
    }

    #[test]
    fn enforce_missing_caps_preserve_order_and_dedup() {
        let local = vec!["a".to_string()];
        // Note: duplicate "c" in required — output should be deduped.
        let required = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "c".to_string(),
        ];
        match enforce_required_caps(&local, &required, EnforceMode::Strict) {
            CapsDecision::Reject { missing } => {
                assert_eq!(missing, vec!["b".to_string(), "c".to_string()]);
            }
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[test]
    fn effective_enforce_mode_defaults_to_soft() {
        // PHANTOM_ENFORCE_REQUIRED_CAPS is process-global and also mutated by
        // serve.rs tests — serialize via the crate env mutex.
        let _g = crate::env_lock::acquire();
        // Bare ClusterConfig with no field set, no env override
        // → Soft (preserves pre-T5 behaviour for every existing
        // deployment).
        let cfg = ClusterConfig::default();
        // Make sure the env is not set for this thread / process.
        std::env::remove_var("PHANTOM_ENFORCE_REQUIRED_CAPS");
        assert_eq!(cfg.effective_enforce_mode(), EnforceMode::Soft);
    }

    #[test]
    fn effective_enforce_mode_config_strict_wins_when_env_unset() {
        let _g = crate::env_lock::acquire();
        let cfg = ClusterConfig {
            enforce_caps: Some(EnforceMode::Strict),
            ..ClusterConfig::default()
        };
        std::env::remove_var("PHANTOM_ENFORCE_REQUIRED_CAPS");
        assert_eq!(cfg.effective_enforce_mode(), EnforceMode::Strict);
    }

    #[test]
    fn effective_enforce_mode_env_overrides_config() {
        let _g = crate::env_lock::acquire();
        // Env var beats config (operator escape hatch — flip on a
        // running node without editing agents.toml).
        let cfg = ClusterConfig {
            enforce_caps: Some(EnforceMode::Soft),
            ..ClusterConfig::default()
        };
        std::env::set_var("PHANTOM_ENFORCE_REQUIRED_CAPS", "strict");
        assert_eq!(cfg.effective_enforce_mode(), EnforceMode::Strict);
        std::env::remove_var("PHANTOM_ENFORCE_REQUIRED_CAPS");
    }

    #[test]
    fn effective_enforce_mode_env_unknown_value_falls_back_to_config() {
        let _g = crate::env_lock::acquire();
        let cfg = ClusterConfig {
            enforce_caps: Some(EnforceMode::Strict),
            ..ClusterConfig::default()
        };
        std::env::set_var("PHANTOM_ENFORCE_REQUIRED_CAPS", "garbage");
        assert_eq!(cfg.effective_enforce_mode(), EnforceMode::Strict);
        std::env::remove_var("PHANTOM_ENFORCE_REQUIRED_CAPS");
    }

    // ── C1: RPC capability-aware forwarding ────────────────────────────
    //
    // Tests in this module mutate `PHANTOM_FORWARD_ON_CAPS_MISMATCH` so they
    // must serialise — reuse the env_guard pattern from test_security_t7.rs
    // but inlined because this is a unit-test module.
    fn fwd_env_guard() -> std::sync::MutexGuard<'static, ()> {
        // Delegate to the crate-wide env mutex so PHANTOM_FORWARD_ON_CAPS_MISMATCH
        // tests serialize against every other env-touching test process-wide.
        crate::env_lock::acquire()
    }

    fn fixture_peer(name: &str, caps: &[&str], online: bool, last_seen: u64) -> PeerInfo {
        PeerInfo {
            url: format!("http://127.0.0.1:0/{name}"),
            name: name.to_string(),
            version: "test".into(),
            online,
            active_tasks: 0,
            uptime_secs: 0,
            last_seen_unix: last_seen,
            last_seen: None,
            consecutive_failures: 0,
            capabilities: caps.iter().map(|s| s.to_string()).collect(),
            health: PeerHealth::default(),
            tailscale_ip: None,
        }
    }

    /// Richer builder for the load/health/reliability/recency cases that
    /// `peer_with_caps` / `fixture_peer` cannot express (they pin
    /// active_tasks=0, consecutive_failures=0, health=Healthy).
    #[allow(clippy::too_many_arguments)]
    fn peer_full(
        name: &str,
        caps: &[&str],
        online: bool,
        active: u32,
        fails: u32,
        last_seen: u64,
        health: PeerHealth,
    ) -> PeerInfo {
        PeerInfo {
            url: format!("http://{name}:7878"),
            name: name.to_string(),
            version: "test".into(),
            online,
            active_tasks: active,
            uptime_secs: 0,
            last_seen_unix: last_seen,
            last_seen: None,
            consecutive_failures: fails,
            capabilities: caps.iter().map(|s| s.to_string()).collect(),
            health,
            tailscale_ip: None,
        }
    }

    // ── P1-1 Task 1: select_peer pure core ──────────────────────────────────

    #[test]
    fn select_peer_picks_the_only_capable_peer() {
        let peers = vec![
            peer_with_caps("A", &["gpu"]),
            peer_with_caps("B", &["cpu"]),
        ];
        let sel = select_peer(&["gpu".to_string()], &peers).expect("A satisfies gpu");
        assert_eq!(sel.head.name, "A");
        assert!(sel.fallback.is_empty(), "only one capable peer → no fallback");
    }

    #[test]
    fn select_peer_empty_required_matches_any_online() {
        let peers = vec![
            peer_full("B", &["cpu"], true, 0, 0, 1_000, PeerHealth::Healthy),
            peer_full("A", &["gpu"], true, 0, 0, 1_000, PeerHealth::Healthy),
        ];
        // empty required → every online peer qualifies; all keys tie except
        // name, so the lexicographically-smaller "A" is the deterministic head.
        let sel = select_peer(&[], &peers).expect("any online peer qualifies");
        assert_eq!(sel.head.name, "A");
        assert_eq!(sel.fallback.len(), 1);
        assert_eq!(sel.fallback[0].name, "B");
    }

    #[test]
    fn select_peer_empty_caps_is_full_worker() {
        // capabilities=[] = full worker, satisfies any required (parity with
        // select_best_peer_with_caps_treats_empty_caps_as_full_worker).
        let peers = vec![peer_with_caps("full", &[])];
        let sel = select_peer(&["gpu".to_string(), "memory".to_string()], &peers)
            .expect("full worker accepts any required");
        assert_eq!(sel.head.name, "full");
    }

    #[test]
    fn select_peer_no_capable_peer_returns_error_with_inventory() {
        let peers = vec![
            peer_with_caps("A", &["memory"]),
            peer_with_caps("B", &["network"]),
        ];
        match select_peer(&["gpu".to_string()], &peers) {
            Err(RouteError::NoCapablePeer {
                required,
                online_inventory,
            }) => {
                assert_eq!(required, vec!["gpu".to_string()]);
                // inventory lists every online peer's (name, caps)
                assert!(online_inventory.contains(&(
                    "A".to_string(),
                    vec!["memory".to_string()]
                )));
                assert!(online_inventory.contains(&(
                    "B".to_string(),
                    vec!["network".to_string()]
                )));
            }
            other => panic!("expected NoCapablePeer, got {other:?}"),
        }
    }

    #[test]
    fn select_peer_no_online_peers_returns_no_peers_available() {
        let peers = vec![
            peer_full("A", &["gpu"], false, 0, 0, 1_000, PeerHealth::Healthy),
            peer_full("B", &["gpu"], false, 0, 0, 1_000, PeerHealth::Healthy),
        ];
        match select_peer(&["gpu".to_string()], &peers) {
            Err(RouteError::NoPeersAvailable) => {}
            other => panic!("expected NoPeersAvailable (distinct from NoCapablePeer), got {other:?}"),
        }
    }

    // ── P1-1 Task 2: load / health / reliability / recency / name ordering ──

    #[test]
    fn select_peer_breaks_tie_by_least_load() {
        // Bug-fix vs old select_best_peer_with_caps (which preferred the
        // MORE-loaded peer): least-loaded must win.
        let peers = vec![
            peer_full("A", &["gpu"], true, 3, 0, 1_000, PeerHealth::Healthy),
            peer_full("B", &["gpu"], true, 0, 0, 1_000, PeerHealth::Healthy),
        ];
        let sel = select_peer(&["gpu".to_string()], &peers).expect("both capable");
        assert_eq!(sel.head.name, "B", "least-loaded (load=0) must be head");
        assert_eq!(sel.fallback.len(), 1);
        assert_eq!(sel.fallback[0].name, "A");
    }

    #[test]
    fn select_peer_prefers_healthy_over_unhealthy() {
        // Health tier dominates load: healthy-but-busy beats unhealthy-but-idle.
        let peers = vec![
            peer_full("A", &["gpu"], true, 5, 0, 1_000, PeerHealth::Healthy),
            peer_full(
                "B",
                &["gpu"],
                true,
                0,
                0,
                1_000,
                PeerHealth::Unhealthy {
                    since: Instant::now(),
                    failure_count: 5,
                },
            ),
        ];
        let sel = select_peer(&["gpu".to_string()], &peers).expect("both capable");
        assert_eq!(sel.head.name, "A", "healthy peer wins despite higher load");
    }

    #[test]
    fn select_peer_load_breaks_tie_within_same_health_tier() {
        // Same health + same load → fewer consecutive_failures wins.
        let peers = vec![
            peer_full("A", &["gpu"], true, 0, 2, 1_000, PeerHealth::Healthy),
            peer_full("B", &["gpu"], true, 0, 0, 1_000, PeerHealth::Healthy),
        ];
        let sel = select_peer(&["gpu".to_string()], &peers).expect("both capable");
        assert_eq!(sel.head.name, "B", "fewer fails (0) beats more fails (2)");
    }

    #[test]
    fn select_peer_recency_then_name_final_tiebreak() {
        // Identical health/load/fails, different recency → fresher wins.
        let peers = vec![
            peer_full("stale", &["gpu"], true, 0, 0, 1_000, PeerHealth::Healthy),
            peer_full("fresh", &["gpu"], true, 0, 0, 9_000, PeerHealth::Healthy),
        ];
        let sel = select_peer(&["gpu".to_string()], &peers).expect("both capable");
        assert_eq!(sel.head.name, "fresh", "larger last_seen_unix wins");

        // Identical on EVERYTHING incl. recency → lexicographically smaller
        // name wins (the determinism guard).
        let peers2 = vec![
            peer_full("zeta", &["gpu"], true, 0, 0, 5_000, PeerHealth::Healthy),
            peer_full("alpha", &["gpu"], true, 0, 0, 5_000, PeerHealth::Healthy),
        ];
        let sel2 = select_peer(&["gpu".to_string()], &peers2).expect("both capable");
        assert_eq!(
            sel2.head.name, "alpha",
            "all keys tie → smaller name is the deterministic pick"
        );
    }

    // ── P1-1 Task 3: walk_with_fallback retry invariants (oracle-driven) ────

    /// Build a 3-peer selection (head + 2 fallback) from hermetic fixtures.
    fn three_peer_selection(peers: &[PeerInfo]) -> PeerSelection<'_> {
        select_peer(&[], peers).expect("≥1 online peer")
    }

    #[test]
    fn walk_dispatches_to_head_on_first_success() {
        let peers = vec![
            peer_full("A", &[], true, 0, 0, 1_000, PeerHealth::Healthy),
            peer_full("B", &[], true, 1, 0, 1_000, PeerHealth::Healthy),
        ];
        let sel = three_peer_selection(&peers);
        let mut seen: Vec<String> = Vec::new();
        let out = walk_with_fallback(&sel, |p| {
            seen.push(p.name.clone());
            Ok(format!("job-{}", p.name))
        })
        .expect("head succeeds");
        assert_eq!(out, "job-A", "head (least-loaded) used");
        assert_eq!(seen, vec!["A".to_string()], "oracle called once, no fallback");
    }

    #[test]
    fn walk_falls_through_to_next_peer_on_retryable_failure() {
        let peers = vec![
            peer_full("A", &[], true, 0, 0, 1_000, PeerHealth::Healthy),
            peer_full("B", &[], true, 1, 0, 1_000, PeerHealth::Healthy),
        ];
        let sel = three_peer_selection(&peers);
        let mut seen: Vec<String> = Vec::new();
        let out = walk_with_fallback(&sel, |p| {
            seen.push(p.name.clone());
            if p.name == "A" {
                Err(DispatchError::PeerUnreachable {
                    url: p.url.clone(),
                    source: "conn refused".into(),
                })
            } else {
                Ok(format!("job-{}", p.name))
            }
        })
        .expect("fallback B succeeds");
        assert_eq!(out, "job-B");
        assert_eq!(seen, vec!["A".to_string(), "B".to_string()], "head then fallback");
    }

    #[test]
    fn walk_stops_immediately_on_fatal_error() {
        let peers = vec![
            peer_full("A", &[], true, 0, 0, 1_000, PeerHealth::Healthy),
            peer_full("B", &[], true, 1, 0, 1_000, PeerHealth::Healthy),
        ];
        let sel = three_peer_selection(&peers);
        let mut seen: Vec<String> = Vec::new();
        let err = walk_with_fallback(&sel, |p| {
            seen.push(p.name.clone());
            Err::<String, _>(DispatchError::HMACMismatch { url: p.url.clone() })
        })
        .expect_err("fatal HMACMismatch must propagate");
        assert!(matches!(err, DispatchError::HMACMismatch { .. }));
        assert_eq!(seen, vec!["A".to_string()], "fatal → oracle called ONCE, no fallback");
    }

    #[test]
    fn walk_returns_last_error_when_all_peers_fail() {
        let peers = vec![
            peer_full("A", &[], true, 0, 0, 1_000, PeerHealth::Healthy),
            peer_full("B", &[], true, 1, 0, 1_000, PeerHealth::Healthy),
        ];
        let sel = three_peer_selection(&peers);
        let mut seen: Vec<String> = Vec::new();
        let err = walk_with_fallback(&sel, |p| {
            seen.push(p.name.clone());
            if p.name == "A" {
                Err::<String, _>(DispatchError::Timeout {
                    url: p.url.clone(),
                    elapsed: std::time::Duration::from_secs(1),
                })
            } else {
                Err::<String, _>(DispatchError::PeerUnreachable {
                    url: p.url.clone(),
                    source: "down".into(),
                })
            }
        })
        .expect_err("all peers fail");
        // LAST error (from B) is surfaced, not the first.
        assert!(
            matches!(err, DispatchError::PeerUnreachable { .. }),
            "last observed error (PeerUnreachable from B) must be returned, got {err:?}"
        );
        assert_eq!(seen, vec!["A".to_string(), "B".to_string()]);
    }

    #[test]
    fn walk_never_retries_same_peer() {
        // Three capable peers; all fail retryably → each name appears once.
        let peers = vec![
            peer_full("A", &[], true, 0, 0, 1_000, PeerHealth::Healthy),
            peer_full("B", &[], true, 1, 0, 1_000, PeerHealth::Healthy),
            peer_full("C", &[], true, 2, 0, 1_000, PeerHealth::Healthy),
        ];
        let sel = three_peer_selection(&peers);
        let mut seen: Vec<String> = Vec::new();
        let _ = walk_with_fallback(&sel, |p| {
            seen.push(p.name.clone());
            Err::<String, _>(DispatchError::Timeout {
                url: p.url.clone(),
                elapsed: std::time::Duration::from_secs(1),
            })
        });
        let mut unique = seen.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(seen.len(), unique.len(), "no peer tried twice: {seen:?}");
        assert_eq!(seen, vec!["A".to_string(), "B".to_string(), "C".to_string()]);
    }

    // ── P1-1 Task 5: master dispatch routes through select_peer ─────────────

    #[test]
    fn assign_task_to_best_peer_picks_least_loaded_of_two() {
        // Pure form (preferred per plan): pin the routing decision the async
        // dispatch path now makes — empty required (legacy "any online"),
        // least-loaded healthy peer is the head whose URL gets dispatched.
        let peers = vec![
            peer_full("busy", &[], true, 5, 0, 1_000, PeerHealth::Healthy),
            peer_full("idle", &[], true, 0, 0, 1_000, PeerHealth::Healthy),
        ];
        let sel = select_peer(&[], &peers).expect("two online peers");
        assert_eq!(sel.head.name, "idle", "least-loaded peer is the dispatch head");
        assert_eq!(sel.head.url, "http://idle:7878");
        // The busier peer is the first fallback (best-first ordering).
        assert_eq!(sel.fallback.len(), 1);
        assert_eq!(sel.fallback[0].name, "busy");
    }

    #[test]
    fn task_assign_request_forward_chain_defaults_to_empty() {
        // Back-compat: old clients omit `forward_chain` and
        // `idempotency_key`, and must continue to parse.
        let body = json!({ "prompt": "hi" });
        let req: TaskAssignRequest = serde_json::from_value(body).expect("parse");
        assert!(req.forward_chain.is_empty());
        assert!(req.idempotency_key.is_none());
    }

    #[test]
    fn task_assign_request_round_trips_with_new_fields() {
        // C1 forwarder relies on Serialize → byte-stable for HMAC.
        let req = TaskAssignRequest {
            agent: "master".into(),
            prompt: "echo hi".into(),
            required_caps: vec!["shell.write".into()],
            forward_chain: vec!["node-a".into()],
            idempotency_key: Some("test-001".into()),
        };
        let bytes = serde_json::to_vec(&req).expect("serialize");
        let parsed: TaskAssignRequest = serde_json::from_slice(&bytes).expect("parse");
        assert_eq!(parsed.agent, "master");
        assert_eq!(parsed.forward_chain, vec!["node-a".to_string()]);
        assert_eq!(parsed.idempotency_key.as_deref(), Some("test-001"));
        assert_eq!(parsed.required_caps, vec!["shell.write".to_string()]);
    }

    #[test]
    fn select_best_peer_with_caps_picks_capable_online_peer() {
        // C1 happy path: A has the required cap, B does not — A wins.
        let peers = vec![
            fixture_peer("A", &["shell.write", "network"], true, 1000),
            fixture_peer("B", &["memory"], true, 2000),
        ];
        let picked =
            select_best_peer_with_caps(&["shell.write".to_string()], &peers).expect("A satisfies");
        assert_eq!(picked.name, "A");
    }

    #[test]
    fn select_best_peer_with_caps_skips_offline_even_if_capable() {
        let peers = vec![
            fixture_peer("A", &["shell.write"], false, 9999), // offline
            fixture_peer("B", &["shell.write"], true, 100),
        ];
        let picked = select_best_peer_with_caps(&["shell.write".to_string()], &peers)
            .expect("B picked despite older last_seen");
        assert_eq!(picked.name, "B");
    }

    #[test]
    fn select_best_peer_with_caps_returns_none_when_nobody_matches() {
        // All online peers, none advertise shell.write — caller should
        // map this to NoPeerSatisfiesCaps, not NoPeersAvailable.
        let peers = vec![
            fixture_peer("A", &["memory"], true, 1000),
            fixture_peer("B", &["network"], true, 2000),
        ];
        assert!(select_best_peer_with_caps(&["shell.write".to_string()], &peers,).is_none());
    }

    #[test]
    fn select_best_peer_with_caps_treats_empty_caps_as_full_worker() {
        // PeerInfo.capabilities.is_empty() = full worker per the
        // ClusterConfig::worker_caps semantics — it accepts everything.
        let peers = vec![fixture_peer("full", &[], true, 1000)];
        let picked =
            select_best_peer_with_caps(&["shell.write".to_string(), "memory".to_string()], &peers)
                .expect("full worker accepts");
        assert_eq!(picked.name, "full");
    }

    #[test]
    fn select_best_peer_with_caps_breaks_ties_by_recency() {
        // Two equally capable peers — most-recently-pinged wins.
        let peers = vec![
            fixture_peer("old", &["shell.write"], true, 1_000),
            fixture_peer("new", &["shell.write"], true, 9_000),
        ];
        let picked =
            select_best_peer_with_caps(&["shell.write".to_string()], &peers).expect("at least one");
        assert_eq!(picked.name, "new", "newer last_seen should win the tie");
    }

    #[test]
    fn forward_decision_chooses_capable_peer_when_env_gate_on() {
        // C1 spec test #1: with PHANTOM_FORWARD_ON_CAPS_MISMATCH=1,
        // mismatch + a capable peer in `peers` returns ForwardTo(peer).
        let _g = fwd_env_guard();
        std::env::set_var("PHANTOM_FORWARD_ON_CAPS_MISMATCH", "1");

        let local = vec!["memory".to_string()];
        let required = vec!["shell.write".to_string()];
        let peers = vec![
            fixture_peer("A-can", &["shell.write"], true, 2000),
            fixture_peer("B-cant", &["memory"], true, 1000),
        ];
        let decision =
            enforce_required_caps_with_forwarding(&local, &required, EnforceMode::Strict, &peers);
        match decision {
            CapsDecision::ForwardTo { peer, missing } => {
                assert_eq!(peer.name, "A-can");
                assert_eq!(missing, vec!["shell.write".to_string()]);
            }
            other => panic!("expected ForwardTo, got {other:?}"),
        }

        std::env::remove_var("PHANTOM_FORWARD_ON_CAPS_MISMATCH");
    }

    #[test]
    fn forward_disabled_returns_reject_by_default() {
        // C1 spec test #5: without PHANTOM_FORWARD_ON_CAPS_MISMATCH,
        // behaviour is unchanged from pre-C1 — strict mode still 409s
        // even when a capable peer exists. Back-compat guarantee.
        let _g = fwd_env_guard();
        std::env::remove_var("PHANTOM_FORWARD_ON_CAPS_MISMATCH");

        let local = vec!["memory".to_string()];
        let required = vec!["shell.write".to_string()];
        let peers = vec![fixture_peer("A-can", &["shell.write"], true, 2000)];

        let decision =
            enforce_required_caps_with_forwarding(&local, &required, EnforceMode::Strict, &peers);
        match decision {
            CapsDecision::Reject { missing } => {
                assert_eq!(missing, vec!["shell.write".to_string()]);
            }
            other => panic!("expected Reject (forwarding disabled), got {other:?}"),
        }
    }

    #[test]
    fn forward_falls_back_to_reject_when_no_peer_satisfies() {
        // C1 spec test #4 (helper level): env gate on, but no peer
        // satisfies → the enforce sibling falls back to Reject. The
        // call site translates that to NoPeerSatisfiesCaps error.
        let _g = fwd_env_guard();
        std::env::set_var("PHANTOM_FORWARD_ON_CAPS_MISMATCH", "1");

        let local = vec!["memory".to_string()];
        let required = vec!["shell.write".to_string()];
        let peers = vec![fixture_peer("B-cant", &["memory"], true, 1000)];

        let decision =
            enforce_required_caps_with_forwarding(&local, &required, EnforceMode::Strict, &peers);
        assert!(
            matches!(decision, CapsDecision::Reject { .. }),
            "no capable peer ⇒ fall back to the original Reject (call site translates)"
        );
        std::env::remove_var("PHANTOM_FORWARD_ON_CAPS_MISMATCH");
    }

    #[test]
    fn dispatch_error_no_peer_satisfies_caps_renders_inventory() {
        // Display should surface both required and available_peers so the
        // CLI can echo verbatim — operators must see the actual mismatch.
        let e = DispatchError::NoPeerSatisfiesCaps {
            required: vec!["shell.write".into()],
            available_peers: vec![
                ("http://a:7878".into(), vec!["memory".into()]),
                ("http://b:7878".into(), vec!["network".into()]),
            ],
        };
        let s = format!("{e}");
        assert!(s.contains("shell.write"), "got {s}");
        assert!(s.contains("memory") && s.contains("network"), "got {s}");
    }

    #[test]
    fn dispatch_error_forward_chain_exhausted_renders_chain() {
        let e = DispatchError::ForwardChainExhausted {
            chain: vec!["A".into(), "B".into()],
            reason: "self_in_chain".into(),
        };
        let s = format!("{e}");
        assert!(s.contains("self_in_chain"));
        assert!(s.contains("\"A\"") && s.contains("\"B\""), "got {s}");
    }

    // ── C4: peer heartbeat + health-aware selection ────────────────────────
    //
    // Tests split into two groups:
    //   * Data-only tests (filter, defaults, fallback) — run on every build.
    //   * State-machine tests (transitions) — gated on the
    //     `experimental-cluster-heartbeat` feature so the test suite matches
    //     the production behaviour for the corresponding flag.

    fn unhealthy_fixture(
        name: &str,
        caps: &[&str],
        last_seen: u64,
        failure_count: u32,
    ) -> PeerInfo {
        let mut p = fixture_peer(name, caps, true, last_seen);
        p.health = PeerHealth::Unhealthy {
            since: Instant::now(),
            failure_count,
        };
        p
    }

    #[test]
    fn peer_health_default_is_healthy() {
        // Optimistic default: newly-loaded peers get a chance to respond
        // before routing skips them.
        assert!(PeerHealth::default().is_healthy());
        assert!(matches!(PeerHealth::default(), PeerHealth::Healthy));
    }

    #[test]
    fn peer_info_default_health_round_trips_through_serde() {
        // Backwards-compat: existing peers.json files predate the `health`
        // field. They must still deserialise (default = Healthy) and the
        // re-serialised form must contain `"health": "healthy"`.
        let body = json!({
            "url": "http://a:7878",
            "name": "A",
            "version": "0.5.0",
            "online": true,
            "active_tasks": 0,
            "uptime_secs": 0,
            "last_seen_unix": 0,
            "consecutive_failures": 0,
        });
        let p: PeerInfo = serde_json::from_value(body).expect("old format parses");
        assert!(p.health.is_healthy());

        let again = serde_json::to_string(&p).expect("serialize");
        assert!(again.contains("\"health\":\"healthy\""), "got {again}");
    }

    #[test]
    fn peer_health_unhealthy_deserialises_to_optimistic_healthy_on_cold_load() {
        // Documented invariant: cold-start reload = optimistic. Even if the
        // on-disk tag says "unhealthy", a fresh daemon should give the peer
        // one probe cycle before routing skips it.
        let body = json!({
            "url": "http://a:7878",
            "name": "A",
            "version": "0.5.0",
            "online": true,
            "active_tasks": 0,
            "uptime_secs": 0,
            "last_seen_unix": 0,
            "consecutive_failures": 0,
            "health": "unhealthy",
        });
        let p: PeerInfo = serde_json::from_value(body).expect("parse");
        // We accept the tag — heartbeat task will re-derive on-the-wire state.
        assert!(!p.health.is_healthy());
    }

    #[test]
    fn select_best_peer_prefers_healthy_over_unhealthy() {
        // C4 core contract: a Healthy peer wins even when an Unhealthy peer
        // has the same caps and a more-recent last_seen.
        let peers = vec![
            // Unhealthy but newer + capable.
            unhealthy_fixture("U", &["shell.write"], 9999, 5),
            // Healthy, older, capable.
            fixture_peer("H", &["shell.write"], true, 1000),
        ];
        let picked = select_best_peer_with_caps(&["shell.write".to_string()], &peers)
            .expect("must pick somebody");
        assert_eq!(
            picked.name, "H",
            "Healthy peer must win over Unhealthy even with worse last_seen",
        );
    }

    #[test]
    fn select_best_peer_falls_back_to_unhealthy_when_no_healthy_match() {
        // C4 fallback: if NO healthy peer satisfies caps, pick the best
        // Unhealthy one rather than returning None. Better to attempt a
        // possibly-stale peer than to fail the dispatch entirely.
        let peers = vec![
            // Healthy but wrong caps.
            fixture_peer("H-wrong-caps", &["memory"], true, 5000),
            // Unhealthy but capable.
            unhealthy_fixture("U", &["shell.write"], 1000, 4),
        ];
        let picked = select_best_peer_with_caps(&["shell.write".to_string()], &peers)
            .expect("fall back to Unhealthy U");
        assert_eq!(picked.name, "U");
    }

    #[test]
    fn select_best_peer_picks_most_recent_when_both_healthy() {
        // No regression: when both tiers have matches, last_seen tie-break
        // applies within the Healthy tier — same as pre-C4.
        let peers = vec![
            fixture_peer("H-old", &["shell.write"], true, 100),
            fixture_peer("H-new", &["shell.write"], true, 200),
        ];
        let picked =
            select_best_peer_with_caps(&["shell.write".to_string()], &peers).expect("must pick");
        assert_eq!(picked.name, "H-new");
    }

    #[test]
    fn cluster_config_heartbeat_defaults_are_30s_and_3() {
        // Defaults are public constants so operators can reference them in
        // sample configs without copy-paste drift.
        assert_eq!(DEFAULT_HEARTBEAT_INTERVAL_SECS, 30);
        assert_eq!(DEFAULT_HEARTBEAT_FAILURE_THRESHOLD, 3);

        let cfg = ClusterConfig::default();
        let mgr = ClusterManager::new(cfg);
        assert_eq!(mgr.effective_heartbeat_interval().as_secs(), 30);
        assert_eq!(mgr.effective_heartbeat_failure_threshold(), 3);
    }

    #[test]
    fn cluster_config_heartbeat_overrides_are_honoured() {
        let cfg = ClusterConfig {
            heartbeat_interval_secs: Some(7),
            heartbeat_failure_threshold: Some(2),
            ..ClusterConfig::default()
        };
        let mgr = ClusterManager::new(cfg);
        assert_eq!(mgr.effective_heartbeat_interval().as_secs(), 7);
        assert_eq!(mgr.effective_heartbeat_failure_threshold(), 2);
    }

    #[cfg(feature = "experimental-cluster-heartbeat")]
    #[tokio::test]
    async fn record_probe_result_transitions_healthy_to_unhealthy_after_threshold() {
        // C4 spec test 1: N consecutive failures flips Healthy → Unhealthy.
        // With threshold = 2: failures 1,2 — peer Unhealthy by the second.
        let url = "http://hb-test-a:7878";
        let cfg = ClusterConfig {
            peers: vec![url.to_string()],
            heartbeat_failure_threshold: Some(2),
            ..ClusterConfig::default()
        };
        let mgr = ClusterManager::new(cfg);
        // Sanity: starts Healthy.
        let initial = mgr.peer_infos().await;
        assert!(initial[0].health.is_healthy(), "starts Healthy");

        mgr.record_probe_result(url, false).await; // failure 1
        let after_one = mgr.peer_infos().await;
        assert!(
            after_one[0].health.is_healthy(),
            "still Healthy at 1 < threshold"
        );

        mgr.record_probe_result(url, false).await; // failure 2 = threshold
        let after_two = mgr.peer_infos().await;
        assert!(
            !after_two[0].health.is_healthy(),
            "flipped Unhealthy at threshold (2)",
        );
        match &after_two[0].health {
            PeerHealth::Unhealthy { failure_count, .. } => assert_eq!(*failure_count, 2),
            PeerHealth::Healthy => panic!("expected Unhealthy"),
        }
    }

    #[cfg(feature = "experimental-cluster-heartbeat")]
    #[tokio::test]
    async fn record_probe_result_recovers_unhealthy_to_healthy_on_success() {
        // C4 spec test 2: a single success after Unhealthy flips back.
        let url = "http://hb-test-b:7878";
        let cfg = ClusterConfig {
            peers: vec![url.to_string()],
            heartbeat_failure_threshold: Some(1),
            ..ClusterConfig::default()
        };
        let mgr = ClusterManager::new(cfg);

        mgr.record_probe_result(url, false).await; // threshold = 1 ⇒ Unhealthy
        assert!(!mgr.peer_infos().await[0].health.is_healthy());

        mgr.record_probe_result(url, true).await; // recovery
        let recovered = mgr.peer_infos().await;
        assert!(
            recovered[0].health.is_healthy(),
            "recovered Healthy on success"
        );
        assert_eq!(
            recovered[0].consecutive_failures, 0,
            "counter reset on success"
        );
    }

    #[cfg(feature = "experimental-cluster-heartbeat")]
    #[tokio::test]
    async fn record_probe_result_stays_healthy_below_threshold_then_recovers() {
        // Sanity: 2 failures with threshold 3 ⇒ stay Healthy; one success
        // resets the counter so the next 2 failures are not catastrophic.
        let url = "http://hb-test-c:7878";
        let cfg = ClusterConfig {
            peers: vec![url.to_string()],
            heartbeat_failure_threshold: Some(3),
            ..ClusterConfig::default()
        };
        let mgr = ClusterManager::new(cfg);

        mgr.record_probe_result(url, false).await;
        mgr.record_probe_result(url, false).await;
        let mid = mgr.peer_infos().await;
        assert!(
            mid[0].health.is_healthy(),
            "2 < threshold(3): still Healthy"
        );
        assert_eq!(mid[0].consecutive_failures, 2);

        mgr.record_probe_result(url, true).await;
        let after = mgr.peer_infos().await;
        assert_eq!(after[0].consecutive_failures, 0, "success resets counter");
        assert!(after[0].health.is_healthy());
    }

    #[cfg(feature = "experimental-cluster-heartbeat")]
    #[tokio::test]
    async fn spawn_heartbeat_task_skipped_when_no_peers() {
        // Spec constraint: heartbeat task must not start if cluster has
        // zero peers (single-node deploys see zero behaviour change).
        let mgr = std::sync::Arc::new(ClusterManager::new(ClusterConfig::default()));
        let handle = mgr.spawn_heartbeat_task();
        assert!(
            handle.is_none(),
            "spawn must return None when peers.is_empty()",
        );
    }

    #[cfg(feature = "experimental-cluster-heartbeat")]
    #[tokio::test]
    async fn spawn_heartbeat_task_starts_when_peers_present() {
        // The task itself loops forever — we just verify it spawned and
        // abort it immediately so the test process exits cleanly.
        let cfg = ClusterConfig {
            peers: vec!["http://nowhere.invalid:7878".to_string()],
            heartbeat_interval_secs: Some(60), // long, we abort below
            ..ClusterConfig::default()
        };
        let mgr = std::sync::Arc::new(ClusterManager::new(cfg));
        let handle = mgr.spawn_heartbeat_task().expect("spawned");
        handle.abort();
    }

    // ── SHARED P0: tailscale status parser ─────────────────────────────────

    /// Pins the shape that `extract_tailscale_peer_ips` consumes —
    /// a subset of what `tailscale status --json` emits in practice.
    /// Online peers contribute their IPv4 from `TailscaleIPs`; offline
    /// peers are skipped; IPv6 entries (those containing `:`) are
    /// dropped because phantom dials :7878 over IPv4 only.
    #[test]
    fn peer_list_parses_tailscale_status_json() {
        let status = json!({
            "Self": {
                "TailscaleIPs": ["100.64.0.1"],
                "Online": true,
            },
            "Peer": {
                "node-a": {
                    "Online": true,
                    "TailscaleIPs": ["100.64.0.10", "fd7a:115c:a1e0::a"],
                },
                "node-m1": {
                    "Online": true,
                    "TailscaleIPs": ["100.64.0.20"],
                },
                "node-b": {
                    "Online": false,
                    "TailscaleIPs": ["100.64.0.30"],
                },
            }
        });

        let ips = extract_tailscale_peer_ips(&status);
        assert!(
            ips.contains(&"100.64.0.10".to_string()),
            "node-a IPv4 must be included"
        );
        assert!(
            ips.contains(&"100.64.0.20".to_string()),
            "m1 IPv4 must be included"
        );
        assert!(
            !ips.iter().any(|s| s == "100.64.0.30"),
            "offline node IP must be skipped"
        );
        assert!(
            !ips.iter().any(|s| s.contains(':')),
            "IPv6 entries must be dropped (phantom dials IPv4 only)"
        );
        assert!(
            !ips.iter().any(|s| s == "100.64.0.1"),
            "Self entry must not appear in peer list"
        );
    }

    // ── SHARED P0: HMAC cluster auth ───────────────────────────────────────

    /// Verifies the round-trip of `make_auth_token_bytes` ↔ `verify_auth`:
    /// a token minted from `body` with `cluster_secret = S` must validate
    /// when re-checked with the same body and secret, and must FAIL on
    /// any mutation (body byte flip, secret change, or empty secret).
    /// This is the cluster's only line of defense against a third party
    /// posting `/rpc/task/assign` requests — if validate ever returns
    /// true for a tampered body, we ship a remote-exec hole.
    #[test]
    fn hmac_signature_validates_with_shared_secret() {
        let secret = "test-cluster-secret-xyz";
        let body = br#"{"task_id":"t1","payload":"hello"}"#;

        let mgr = ClusterManager::new(ClusterConfig {
            cluster_secret: Some(secret.to_string()),
            ..ClusterConfig::default()
        });

        let token = mgr.make_auth_token_bytes(body);
        // hex-encoded SHA-256 ⇒ exactly 64 hex chars
        assert_eq!(
            token.len(),
            64,
            "HMAC-SHA256 hex must be 64 chars, got {}",
            token.len()
        );
        assert!(
            token.chars().all(|c| c.is_ascii_hexdigit()),
            "token must be lowercase hex: {token}",
        );

        // Round-trip: same body + same secret ⇒ valid.
        assert!(mgr.verify_auth(&token, body), "fresh token must validate");

        // Tampered body ⇒ invalid (constant-time compare must still reject).
        let mut tampered = body.to_vec();
        tampered[10] ^= 0x01;
        assert!(
            !mgr.verify_auth(&token, &tampered),
            "tampered body must NOT validate"
        );

        // Different secret ⇒ invalid.
        let other_mgr = ClusterManager::new(ClusterConfig {
            cluster_secret: Some("different-secret".to_string()),
            ..ClusterConfig::default()
        });
        assert!(
            !other_mgr.verify_auth(&token, body),
            "token minted under secret A must NOT validate under secret B"
        );

        // No-secret ClusterManager ⇒ always rejects (fail-closed).
        let unconfigured = ClusterManager::new(ClusterConfig {
            cluster_secret: None,
            ..ClusterConfig::default()
        });
        assert!(
            !unconfigured.verify_auth(&token, body),
            "unconfigured cluster must reject all tokens (fail-closed)"
        );

        // Empty-string secret behaves the same as unconfigured.
        let empty = ClusterManager::new(ClusterConfig {
            cluster_secret: Some(String::new()),
            ..ClusterConfig::default()
        });
        assert!(
            !empty.verify_auth(&token, body),
            "empty-string secret must reject (fail-closed)"
        );
    }

    /// SPEC-10 migration: `verify_auth_dual` must accept BOTH the legacy
    /// body-HMAC (X-Cluster-Auth) and the new canonical-HMAC
    /// (X-Cluster-Auth), reject when neither matches, and stay
    /// fail-closed with no secret.
    #[test]
    fn verify_auth_dual_accepts_legacy_and_canonical() {
        let secret = "dual-accept-secret-123";
        let body = br#"{"agent":"master","prompt":"hi"}"#;
        let (method, path, query) = ("POST", "/rpc/task/assign", "");

        let mgr = ClusterManager::new(ClusterConfig {
            cluster_secret: Some(secret.to_string()),
            ..ClusterConfig::default()
        });

        // Legacy arm: token = HMAC over raw body.
        let legacy = mgr.make_auth_token_bytes(body);
        assert!(
            mgr.verify_auth_dual(Some(&legacy), None, method, path, query, body, None),
            "legacy X-Cluster-Auth token must verify via dual path"
        );

        // SPEC-10 arm: sig = HMAC over the canonical string.
        let canonical =
            crate::rpc_wire::build_canonical_string(method, path, query, body, None);
        let sig = crate::rpc_wire::sign_hmac(secret.as_bytes(), &canonical);
        assert!(
            mgr.verify_auth_dual(None, Some(&sig), method, path, query, body, None),
            "SPEC-10 X-Cluster-Auth must verify via dual path"
        );

        // The two schemes are NOT interchangeable: a legacy token presented in
        // the canonical slot (or vice-versa) must NOT verify.
        assert!(
            !mgr.verify_auth_dual(None, Some(&legacy), method, path, query, body, None),
            "legacy token must NOT validate as a canonical signature"
        );

        // Neither header → reject. Both wrong → reject.
        assert!(
            !mgr.verify_auth_dual(None, None, method, path, query, body, None),
            "no auth material must reject"
        );
        assert!(
            !mgr.verify_auth_dual(Some("deadbeef"), Some("deadbeef"), method, path, query, body, None),
            "garbage in both slots must reject"
        );

        // Wrong secret on the canonical arm → reject.
        let other = crate::rpc_wire::sign_hmac(b"wrong-secret", &canonical);
        assert!(
            !mgr.verify_auth_dual(None, Some(&other), method, path, query, body, None),
            "canonical sig under a different secret must reject"
        );

        // Fail-closed with no secret, even with otherwise-valid-looking input.
        let unconfigured = ClusterManager::new(ClusterConfig {
            cluster_secret: None,
            ..ClusterConfig::default()
        });
        assert!(
            !unconfigured.verify_auth_dual(Some(&legacy), Some(&sig), method, path, query, body, None),
            "unconfigured cluster must reject both arms (fail-closed)"
        );

        // Inverse non-interchangeability (review: codex): a canonical signature
        // presented in the LEGACY slot must NOT validate (legacy verifies HMAC
        // over the raw body, not the canonical string).
        assert!(
            !mgr.verify_auth_dual(Some(&sig), None, method, path, query, body, None),
            "canonical signature in the legacy slot must reject"
        );

        // Both arms valid simultaneously (review: opencode) → accept (first
        // match short-circuits; documents the both-valid path explicitly).
        assert!(
            mgr.verify_auth_dual(Some(&legacy), Some(&sig), method, path, query, body, None),
            "both valid arms must accept"
        );
    }

    // ── T-CORE-02: query_capability (local capability-query overlay) ────────

    /// Build a ClusterManager whose cached roster is the given peers. The
    /// `node_name` + `capabilities` configure what `own_peer_status()` reports
    /// so the `include_self` path is exercisable without any network I/O.
    async fn mgr_with_roster(
        node_name: &str,
        self_caps: &[&str],
        roster: Vec<PeerInfo>,
    ) -> ClusterManager {
        let mgr = ClusterManager::new(ClusterConfig {
            node_name: Some(node_name.to_string()),
            capabilities: self_caps.iter().map(|s| s.to_string()).collect(),
            ..ClusterConfig::default()
        });
        // Inject the roster directly into the cached peer list (same-module
        // access to the private `peers` field). No ping is issued.
        *mgr.peers.write().await = roster;
        mgr
    }

    #[tokio::test]
    async fn query_capability_returns_only_subset_matching_peers() {
        let roster = vec![
            peer_with_caps("node-a", &["shell", "browser"]),
            peer_with_caps("m1", &["shell", "gpu_compute:metal", "local_llm:mlx"]),
            peer_with_caps("node-b", &["shell", "browser", "camera"]),
        ];
        // self has no special caps and we exclude it, so only peers count.
        let mgr = mgr_with_roster("self", &[], roster).await;

        let required = vec!["camera".to_string()];
        let answers = mgr.query_capability(&required, false).await;
        let names: Vec<&str> = answers.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["node-b"], "only node-b advertises camera");
        assert!(
            answers.iter().all(|a| !a.self_node),
            "include_self=false must never flag a self_node answer"
        );

        // Multi-cap requirement is set inclusion: only m1 has both.
        let required = vec!["local_llm:mlx".to_string(), "gpu_compute:metal".to_string()];
        let answers = mgr.query_capability(&required, false).await;
        let names: Vec<&str> = answers.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["m1"], "only m1 has both required caps");
    }

    #[tokio::test]
    async fn query_capability_empty_required_returns_all_peers() {
        let roster = vec![
            peer_with_caps("node-a", &["shell"]),
            peer_with_caps("m1", &["shell", "local_llm:mlx"]),
            peer_with_caps("legacy", &[]),
        ];
        let mgr = mgr_with_roster("self", &[], roster).await;

        // Empty required_caps matches every peer (set inclusion of nothing).
        let answers = mgr.query_capability(&[], false).await;
        let names: Vec<&str> = answers.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["node-a", "m1", "legacy"],
            "empty required_caps must return the whole roster"
        );
    }

    #[tokio::test]
    async fn query_capability_include_self_toggles_self_answer() {
        let roster = vec![peer_with_caps("node-a", &["shell", "browser"])];
        // self advertises camera, which no peer has.
        let mgr = mgr_with_roster("my-node", &["shell", "camera"], roster).await;

        let required = vec!["camera".to_string()];

        // include_self=false: no peer matches camera, self excluded → empty.
        let excluded = mgr.query_capability(&required, false).await;
        assert!(
            excluded.is_empty(),
            "no peer has camera and self is excluded"
        );

        // include_self=true: self matches camera and is appended.
        let included = mgr.query_capability(&required, true).await;
        assert_eq!(included.len(), 1, "self must be the lone match");
        let me = &included[0];
        assert_eq!(me.name, "my-node");
        assert!(me.self_node, "the self answer must set self_node = true");
        assert!(
            me.capabilities.contains(&"camera".to_string()),
            "self answer carries this node's advertised capabilities"
        );

        // include_self=true but self does NOT satisfy the requirement →
        // self must NOT be appended.
        let unmet = vec!["gpu_compute:cuda".to_string()];
        let none = mgr.query_capability(&unmet, true).await;
        assert!(
            none.is_empty(),
            "self is only included when it satisfies required_caps"
        );
    }

    #[tokio::test]
    async fn query_capability_self_in_roster_not_duplicated() {
        // codex review (self de-dupe): if the cached roster ALREADY contains an
        // entry whose name matches this node, the answer must NOT list self
        // twice. The node-named roster entry is dropped; only the explicit
        // self answer (self_node = true) appears.
        let roster = vec![
            peer_with_caps("my-node", &["shell", "camera"]), // same name as self
            peer_with_caps("node-a", &["shell", "camera"]),
        ];
        let mgr = mgr_with_roster("my-node", &["shell", "camera"], roster).await;

        let required = vec!["camera".to_string()];
        let answers = mgr.query_capability(&required, true).await;

        let mine: Vec<&CapablePeerAnswer> =
            answers.iter().filter(|a| a.name == "my-node").collect();
        assert_eq!(
            mine.len(),
            1,
            "self must appear exactly once even when present in the roster, got {:?}",
            answers.iter().map(|a| (&a.name, a.self_node)).collect::<Vec<_>>()
        );
        assert!(mine[0].self_node, "the single self entry must be the self answer");
        // The genuine peer node-a is unaffected.
        assert!(answers.iter().any(|a| a.name == "node-a" && !a.self_node));

        // include_self = false: self is excluded entirely (no self-named entry).
        let without = mgr.query_capability(&required, false).await;
        assert!(
            without.iter().all(|a| a.name != "my-node"),
            "include_self=false must drop the self-named roster entry too"
        );
    }

    // ?? Regression: best-peer dispatch must sign /rpc/message ????????????
    //
    // Guards the bug where `assign_task_to_best_peer` (the default
    // `phantom peer assign <prompt>` path with no `--target`) passed `None`
    // for the auth token, so the POST to the *gated* `/rpc/message` route
    // carried no `X-Cluster-Auth` header and the server returned 401
    // (surfaced as DispatchError::HMACMismatch). The fix signs the raw body
    // with the legacy raw-body HMAC the server's `verify_auth` accepts.
    //
    // The matcher recomputes HMAC-SHA256(secret, raw_request_body) from the
    // bytes wiremock actually received and asserts the header equals it ??    // proving the client signs the *exact* serialized body, including the
    // random per-request `task_id`.
    struct LegacyHmacMatches {
        secret: String,
    }
    impl wiremock::Match for LegacyHmacMatches {
        fn matches(&self, req: &wiremock::Request) -> bool {
            use hmac::{Hmac, Mac};
            use sha2::Sha256;
            type HmacSha256 = Hmac<Sha256>;
            let header = match req.headers.get("x-cluster-auth") {
                Some(v) => match v.to_str() {
                    Ok(s) => s.to_string(),
                    Err(_) => return false,
                },
                None => return false,
            };
            let mut mac = HmacSha256::new_from_slice(self.secret.as_bytes())
                .expect("HMAC key of any size");
            mac.update(&req.body);
            let expected = hex::encode(mac.finalize().into_bytes());
            // Constant-time not required in a test; exact equality is the point.
            header == expected
        }
    }

    /// Matches only when NO `X-Cluster-Auth` header is present (wiremock has
    /// no built-in "header absent" matcher).
    struct NoAuthHeader;
    impl wiremock::Match for NoAuthHeader {
        fn matches(&self, req: &wiremock::Request) -> bool {
            req.headers.get("x-cluster-auth").is_none()
        }
    }

    fn online_peer_at(url: &str) -> PeerInfo {
        PeerInfo {
            url: url.to_string(),
            ..peer_with_caps("mock-peer", &[])
        }
    }

    #[tokio::test]
    async fn assign_task_to_best_peer_signs_rpc_message_with_legacy_hmac() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let secret = "best-peer-secret-xyz";
        let server = MockServer::start().await;

        // Only matches when the X-Cluster-Auth header is the correct
        // raw-body HMAC. If the client sends no header (the bug) or a wrong
        // signature, this Mock does not match -> wiremock returns 404, the
        // client decodes a non-JSON/empty body and the call fails the asserts.
        Mock::given(method("POST"))
            .and(path("/rpc/message"))
            .and(LegacyHmacMatches {
                secret: secret.to_string(),
            })
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "output": "pong" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let mgr = ClusterManager::new(ClusterConfig {
            cluster_secret: Some(secret.to_string()),
            ..ClusterConfig::default()
        });
        // Inject one online peer pointing at the mock server.
        mgr.peers.write().await.push(online_peer_at(&server.uri()));

        let out = mgr
            .assign_task_to_best_peer("master", "hello")
            .await
            .expect("signed best-peer dispatch must succeed (got HMACMismatch?)");
        assert_eq!(out, "pong");
        // `.expect(1)` on drop verifies the signed request actually hit the route.
    }

    #[tokio::test]
    async fn assign_task_to_best_peer_omits_auth_when_no_secret() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        // With no cluster_secret, no header is sent ??the route is open
        // (matches the secret-not-configured server behaviour). Assert that
        // no X-Cluster-Auth header is attached.
        Mock::given(method("POST"))
            .and(path("/rpc/message"))
            .and(NoAuthHeader)
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "output": "ok" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let mgr = ClusterManager::new(ClusterConfig {
            cluster_secret: None,
            ..ClusterConfig::default()
        });
        mgr.peers.write().await.push(online_peer_at(&server.uri()));

        let out = mgr
            .assign_task_to_best_peer("master", "hello")
            .await
            .expect("unauthenticated best-peer dispatch must succeed when no secret");
        assert_eq!(out, "ok");
    }

    // ── Windows-graceful mDNS discovery ─────────────────────────────────────

    /// On Windows the `sh -c dns-sd/avahi-browse` pipeline cannot run — the
    /// support predicate must say so, and stay enabled on unix so the
    /// existing macOS/Linux discovery path is untouched.
    #[test]
    fn mdns_shell_discovery_disabled_on_windows_enabled_on_unix() {
        if cfg!(windows) {
            assert!(
                !mdns_shell_discovery_supported(),
                "Windows has no `sh`/dns-sd/avahi-browse — shell mDNS discovery must be off"
            );
        } else {
            assert!(
                mdns_shell_discovery_supported(),
                "unix platforms must keep the dns-sd/avahi-browse pipeline enabled"
            );
        }
    }

    /// The browse commands must keep targeting the phantom-mesh service type.
    #[test]
    fn mdns_browse_commands_target_phantom_mesh_service() {
        assert!(DNS_SD_BROWSE_CMD.contains("_phantom-mesh._tcp"));
        assert!(AVAHI_BROWSE_CMD.contains("_phantom-mesh._tcp"));
    }

    #[test]
    fn parse_mdns_urls_extracts_url_fields() {
        // dns-sd-style line: whitespace-delimited, url= in the instance name.
        let dns_sd_like = "12:00:00.000  Add  2  4 local. _phantom-mesh._tcp. \
                           node-a url=http://192.168.1.10:7878\n\
                           12:00:00.001  Add  2  4 local. _phantom-mesh._tcp. no-url-here";
        assert_eq!(
            parse_mdns_urls(dns_sd_like),
            vec!["http://192.168.1.10:7878".to_string()]
        );

        // avahi-browse -p style line: `;`-separated fields, url= in TXT.
        let avahi_like = "=;eth0;IPv4;node-b;_phantom-mesh._tcp;local;host.local;\
                          192.168.1.11;7878;url=http://192.168.1.11:7878;extra";
        assert_eq!(
            parse_mdns_urls(avahi_like),
            vec!["http://192.168.1.11:7878".to_string()]
        );

        // Non-http url= fields and empty input yield nothing.
        assert!(parse_mdns_urls("url=ftp://nope").is_empty());
        assert!(parse_mdns_urls("").is_empty());
    }

    /// Regression: a line containing a non-ASCII char whose byte length grows
    /// under `to_lowercase()` (`İ` U+0130 is 2 bytes; lowercasing yields the
    /// 3-byte `i̇` = `i` + U+0307) must not panic. Previously the field offset
    /// was computed from `line.to_lowercase()` and used to slice the original
    /// `line`, so the index could land mid-char and panic on a char boundary.
    #[test]
    fn parse_mdns_urls_handles_non_ascii_length_change() {
        // `İstanbul-node` sits before the url= field; the dotted-capital-I is
        // 2 bytes in the original but 3 bytes when lowercased, so any
        // cross-string indexing would be misaligned by the time we reach url=.
        let line = "12:00:00.000  Add  2  4 local. _phantom-mesh._tcp. \
                    İstanbul-node url=http://192.168.1.42:7878 trailing";
        // Must not panic, and must extract the correct URL.
        let got = parse_mdns_urls(line);
        assert_eq!(got, vec!["http://192.168.1.42:7878".to_string()]);

        // Also verify case-insensitive matching of the `URL=HTTP` key still
        // works (the key is ASCII; folding must not disturb offsets).
        let upper = "İ URL=HTTP://10.0.0.1:7878;rest";
        assert_eq!(
            parse_mdns_urls(upper),
            vec!["HTTP://10.0.0.1:7878".to_string()]
        );
    }

    /// On Windows `discover_local_peers` must short-circuit gracefully:
    /// no `sh` spawn attempt, immediate empty result.
    #[cfg(windows)]
    #[tokio::test]
    async fn discover_local_peers_returns_empty_gracefully_on_windows() {
        let started = std::time::Instant::now();
        assert!(discover_local_peers().await.is_empty());
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "windows branch must short-circuit, not wait on a child process"
        );
    }
}
