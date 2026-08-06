// SPEC-26 §7 + §9 — Cluster dispatch wire types (single source of truth for the
// capability-tag based cross-peer task dispatch contract: peer capability
// advertisement → task → scored plan → outcome).
//
// Stage 3 (real impl — pure scoring + planning + RPC primitives live):
// the §6.2 weighted-sum scoring formula, the Jaccard-style cap-match,
// the freshness-based latency proxy, the cached in-flight `load_score`,
// the rolling failure-window `recent_failure_penalty`, the
// `plan_dispatch` pure algorithm (filter → score → sort → top-1 +
// fallback chain), the process-local PeerRegistry cache
// (`OnceLock<RwLock<HashMap>>`), the `rpc_post` / `rpc_get` /
// `rpc_poll_status` HMAC-signed reqwest helpers (delegating to
// `crate::rpc_wire::sign_hmac` so all signing flows through one
// canonical builder), the `cap_cache_update` async writer, the
// `refresh_capabilities` end-to-end flow, and the `execute_plan`
// dispatch + poll + fallback walk are now real. The Stage 4 surface
// that remains: the master orchestrator's periodic capability-refresh
// loop, the age-encrypt wrapper for cross-peer task payload privacy
// (SPEC-13), and the SPEC-16 audit-log decay pass that ages the
// `failures_last_5_min` counter (master-side, not in this wire).
//
// 中文: 本檔對應 SPEC-26 §7（資料模型）與 §9（API 合約）。叢集派工
// （cluster dispatch）的線路型別 — 把 user input 拆出的 task 配對到對的
// peer（capability tag matching，能力標籤媒合）+ scoring（評分）+ fallback
// chain（後備順位鏈）。Stage 1 只把 wire 型別 + stub 排好；Stage 2 才把
// 真實 scoring 公式（§6.2 cap-match + latency + load + recent-failure
// penalty）接進 `core/src/cluster_dispatch/roles.rs` 與 master orchestrator
// （協調者），並把 `execute_plan` 接到 SPEC-10 `/rpc/task/assign` 線路 +
// SPEC-13 age 加密 payload + SPEC-12 cluster_secret HMAC 簽章。
//
// **Cycle-break note (cross-spec)**: SPEC-27 smart-task-decompose（智慧任務
// 拆解，LLM-based）depends on **this** spec (was previously listed as a
// reverse-cycle — fixed earlier; SPEC-27 consumes the dispatch slot defined
// here). To avoid forcing SPEC-27 type-shape decisions now, `DispatchTask`
// carries `payload: serde_json::Value`（不透明 opaque payload）— SPEC-27
// Stage 2 will introduce a typed `DecomposedTask` wrapper that serializes
// **into** this `Value` field. This keeps the wire stable while SPEC-27
// iterates on the decomposition schema.
//
// TODO Stage 4:
//   - 把 `execute_plan` 接成「POST /rpc/task/assign with HMAC + age payload
//     → poll /rpc/task/status/:id every 2s → on timeout/fail walk
//     fallback_peer_ids → 任一 success 回 `DispatchOutcome`；全 fail 回
//     `AllPeersBusy` or `RouteTimeout`」。需要 rpc_wire HTTP client +
//     SPEC-10 envelope 整合。
//   - 把 `refresh_capabilities` 接到 `/node/capabilities`（serve.rs 實際
//     註冊的 capability 報告路由；非 SPEC-10 §9.13 的 PUSH 端
//     /rpc/capabilities/refresh），把回傳塞進 master 的 `PeerRegistry`。
//   - 把 `peer_active_load_pseudo` 接到 `/rpc/peers` snapshot cache（master
//     端維護的 in-flight count；現在 stub 為 fully-idle (load=1.0)）。
//   - 把 `failure_history_pseudo` 從 in-memory stub 換到真正的 SPEC-16
//     audit log read（rolling 5-min window）— 現在 stub 為 0 penalty。
//   - 把 `DispatchTask.payload` 的 typed wrapper 留給 SPEC-27 Stage 2 —
//     本檔保留 `serde_json::Value` 不動，cycle-break invariant。

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use ts_rs::TS;

// ─── §7 CapabilityTag — peer 自報能力的最小單位 ─────────────────────────────

/// A single capability advertised by a peer. The `slug` is a stable
/// machine-readable identifier (e.g. `"always-on"`, `"role-coder"`, `"ram"`,
/// `"webSearch"`); the optional `value` carries a parametric refinement
/// (`{slug: "ram", value: Some("16gb")}`) for tags whose presence alone is
/// insufficient. Tag semantics are deliberately open-ended — SPEC-26 §6.2
/// scoring treats them as opaque strings for Jaccard-style intersect/union.
///
/// 中文: peer 自己廣播的「能力標籤」（capability tag）最小單位。`slug`
/// 是機器可讀識別字（如 `"always-on"`、`"role-coder"`、`"ram"`），`value`
/// 是可選的「補充值」（給 `ram=16gb` 這類需要參數的 tag 用）。
/// Stage 2 §6.2 scoring 對 tag 一律當 opaque string 處理，不做語意解析。
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq, Hash)]
#[ts(export, export_to = "../../app/src/lib/generated/cluster_dispatch/")]
#[serde(rename_all = "camelCase")]
pub struct CapabilityTag {
    /// Machine-readable identifier; stable across releases. Examples:
    /// `"always-on"`, `"role-coder"`, `"role-researcher"`, `"cargo"`,
    /// `"git"`, `"webSearch"`, `"ram"`, `"gpu"`.
    pub slug: String,
    /// Optional parametric value. `None` for boolean-style tags (presence
    /// = true); `Some("16gb")` / `Some("apple-silicon")` for parametric.
    pub value: Option<String>,
}

// ─── §7 PeerCapabilities — 一個 peer 完整的能力廣播 snapshot ─────────────

/// All capability tags currently advertised by one peer, plus a freshness
/// timestamp so the scorer can down-weight stale advertisements (SPEC-26
/// §6.2 — Stage 2 will penalize entries older than 60 s).
///
/// 中文: 一個 peer 的完整能力廣播 snapshot — 所有 capability tag + 最後
/// 更新時間戳（用來判 advertisement 是否過期；Stage 2 §6.2 對 > 60 s
/// 舊資料降權）。`peer_id` 與 SPEC-10 `/rpc/peers` 回傳的 id 一致。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/cluster_dispatch/")]
#[serde(rename_all = "camelCase")]
pub struct PeerCapabilities {
    /// Peer identifier (matches SPEC-10 `/rpc/peers` `peer_id`).
    pub peer_id: String,
    /// Capability tags currently advertised by this peer. An empty Vec means
    /// the peer is reachable but advertises nothing. When a task lists any
    /// `required_caps`, such a peer is filtered out *before* scoring
    /// (`plan_dispatch` step 1 keeps only peers advertising every required cap),
    /// so it is never selected. When a task requires no caps, capability
    /// scoring is vacuous (`tag_intersect` returns its empty-criteria constant)
    /// and all peers tie on the cap-match term — selection then falls to the
    /// freshness / load terms.
    pub tags: Vec<CapabilityTag>,
    /// Unix millisecond timestamp when this snapshot was received. Used by
    /// Stage 2 to expire stale advertisements (> 60 s old → recompute via
    /// `refresh_capabilities`).
    pub last_reported_at: u64,
}

// ─── §7 DispatchTask — master 派出去的 task 線路型別 ─────────────────────

/// One task ready to be dispatched. The payload is an opaque
/// `serde_json::Value` — see the cycle-break note at file top: SPEC-27
/// smart-task-decompose serializes its typed `DecomposedTask` into this
/// field, but SPEC-26 must not depend on SPEC-27 (reverse-cycle).
///
/// 中文: 一筆待派的 task。`required_caps` 是 hard constraint（必須要有），
/// `preferred_caps` 是 soft preference（有更好，沒有也接受）。`payload`
/// 是不透明（opaque）的 JSON value — cycle-break invariant，避免本檔
/// 依賴 SPEC-27 typed schema。`deadline_ms` 是 task 的 wall-clock budget
/// （牆上時間預算），超時走 §11 `RouteTimeout`。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/cluster_dispatch/")]
#[serde(rename_all = "camelCase")]
pub struct DispatchTask {
    /// Globally unique task id (UUIDv7 string form, 36 chars).
    pub task_id: String,
    /// Hard-constraint capability tags — peer **must** advertise all of
    /// these or it is excluded from candidate set.
    pub required_caps: Vec<CapabilityTag>,
    /// Soft-preference capability tags — presence raises score, absence
    /// is tolerated. Used by §6.2 `cap_match_score` numerator.
    pub preferred_caps: Vec<CapabilityTag>,
    /// Opaque payload — SPEC-27 typed wrapper serializes into this field
    /// at Stage 2. Stage 1 callers can pass `"null"` (JSON string) for
    /// scoring-only fixtures. Stored as String here because ts-rs does
    /// not implement `TS` for `serde_json::Value`; Stage 2 will swap to
    /// a discriminated-union enum once SPEC-27 types are stable.
    #[ts(type = "unknown")]
    pub payload: String,
    /// Optional wall-clock budget in milliseconds. `None` = use master
    /// default (90 000 ms per SPEC-26 §3.1 G4). Exceeding → `RouteTimeout`.
    pub deadline_ms: Option<u64>,
}

// ─── §7 DispatchPlan — scoring 完、準備執行的派工計畫 ────────────────────

/// The output of `plan_dispatch` — one selected peer + an ordered fallback
/// chain + a human-readable scoring reason for UI / audit. The selected
/// peer is always the highest-scoring candidate; the fallback chain is the
/// remaining candidates sorted by score descending (so on first-peer fail
/// the executor walks `fallback_peer_ids[0]` next).
///
/// 中文: scoring 完的派工計畫（dispatch plan）。`selected_peer_id` 是當下
/// 最高分 peer；`fallback_peer_ids` 是剩下的 peer 按分數遞減排序，給
/// `execute_plan` 在第一順位失敗時依序重派（reassign，重派）。
/// `scoring_reason` 是給 UI 顯示「為什麼派給這台」的人話解釋
/// （例如 `"role-coder + cargo present; rtt 45ms"`）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/cluster_dispatch/")]
#[serde(rename_all = "camelCase")]
pub struct DispatchPlan {
    /// Task this plan dispatches.
    pub task_id: String,
    /// Top-scoring peer; receives the first POST /rpc/task/assign.
    pub selected_peer_id: String,
    /// Remaining peers sorted by score descending. On `selected_peer_id`
    /// failure the executor pops `fallback_peer_ids[0]` and retries. Empty
    /// = no fallback (single-candidate cluster).
    pub fallback_peer_ids: Vec<String>,
    /// Human-readable scoring narrative for UI / audit log. Stage 2 fills
    /// from `ScoreBreakdown` (e.g. `"cap_match 0.83 + latency 0.91 + load
    /// 0.75 → 0.83; required:[role-coder,cargo] present"`).
    pub scoring_reason: String,
    /// Unix millisecond timestamp when the plan was computed. Used by
    /// `execute_plan` to detect stale plans (> 30 s old → re-plan).
    pub planned_at_ms: u64,
}

// ─── §7 DispatchOutcome — 一輪 dispatch 的最終結果 ─────────────────────

/// Final outcome of a single dispatch attempt — which peer actually
/// executed, what state it ended in, and (for the UI) a brief
/// `result_summary` plus optional error string. `completed_at_ms` is None
/// while the task is still Running.
///
/// 中文: 一輪 dispatch 的最終結果。`executed_by_peer_id` 是真正執行的
/// peer（可能不是 plan 第一順位 — fallback chain 走到第幾就是哪台）；
/// `status` 是 §8 state machine（狀態機）的終點 state；`completed_at_ms`
/// 在 task 還在 Running 時為 None。`result_summary` 是給 UI 用的短摘要
/// （≤ 256 字），完整 markdown / diff 仍寫進 SPEC-16 events row。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/cluster_dispatch/")]
#[serde(rename_all = "camelCase")]
pub struct DispatchOutcome {
    /// Task that produced this outcome.
    pub task_id: String,
    /// Peer that actually executed (may differ from `DispatchPlan.
    /// selected_peer_id` if a fallback was used).
    pub executed_by_peer_id: String,
    /// Terminal status of the dispatch attempt.
    pub status: DispatchStatus,
    /// Unix millisecond timestamp when the task started executing on the
    /// chosen peer (first successful `/rpc/task/assign` ack).
    pub started_at_ms: u64,
    /// Unix millisecond timestamp when the task reached a terminal state.
    /// `None` while `status == Running`.
    pub completed_at_ms: Option<u64>,
    /// Short human-readable summary for UI (≤ 256 chars). `None` while
    /// task still running or when degraded with no partial output.
    pub result_summary: Option<String>,
    /// Error string when `status` is Failed / Timeout / NoCandidate.
    /// `None` for success terminals (Completed). The machine-readable
    /// code lives in `DispatchError`; this field is the rendered message.
    pub error: Option<String>,
    /// USD cost attributed to executing this subtask on its peer (SPEC-26 G6/J5).
    /// `0.0` for local / no-cost peers and until the peer's RPC `TaskResult`
    /// carries a real per-task cost (that cross-peer wiring is a deferred
    /// follow-up); `integrate` sums these into `IntegratedResult.total_cost_usd`.
    #[serde(default)]
    pub cost_usd: f64,
}

// ─── §7 PeerScore — `score_peer` 的回傳 ──────────────────────────────────

/// Score assigned by `score_peer` to one candidate peer. The aggregate
/// `score` (0.0–1.0, higher = better) is the weighted sum of the four
/// `breakdown` components per SPEC-26 §6.2. `plan_dispatch` picks the
/// peer with the maximum `score`.
///
/// 中文: 一個候選 peer 的評分結果。`score` 是 0.0–1.0 的加權總分（高分
/// = 越適合），`breakdown` 拆開來顯示每一維貢獻 — UI 在 hover 時可秀
/// 「為什麼這台贏」的明細。Stage 2 §6.2 加權：cap_match × 0.5 + latency
/// × 0.3 + load × 0.15 + recent_failure_penalty × 0.05。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/cluster_dispatch/")]
#[serde(rename_all = "camelCase")]
pub struct PeerScore {
    /// Peer being scored.
    pub peer_id: String,
    /// Aggregate score in [0.0, 1.0]. Higher = better candidate. Stage 2
    /// pins `score < 0.1` → exclude from plan (returns `NoMatchingPeer`).
    pub score: f32,
    /// Per-dimension breakdown for UI + audit. All four components are
    /// in [0.0, 1.0] except `recent_failure_penalty` which is negative.
    pub breakdown: ScoreBreakdown,
}

// ─── §7 ScoreBreakdown — 四維評分明細 ────────────────────────────────────

/// Per-dimension contributions to a `PeerScore`. Stage 2 fills these from
/// §6.2 formulas; Stage 1 stubs may emit all zeros. The UI surfaces this
/// shape verbatim in the dispatch progress card hover tooltip.
///
/// 中文: 評分四維明細。`cap_match_score` = required + preferred tag 媒合度
/// （Jaccard-style 交集除聯集）；`latency_score` = RTT（round-trip time，
/// 來回延遲）愈低分愈高；`load_score` = peer 當下 in-flight task 愈少
/// 分愈高；`recent_failure_penalty` = 近 5 分鐘失敗次數的扣分（負值）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/cluster_dispatch/")]
#[serde(rename_all = "camelCase")]
pub struct ScoreBreakdown {
    /// Capability tag match score (0.0–1.0). Jaccard-style:
    /// `|required ∩ peer_tags| / |required| × 0.7 + |preferred ∩
    /// peer_tags| / max(1, |preferred|) × 0.3`. Missing any required tag
    /// → 0.0 (peer is excluded earlier in `plan_dispatch`).
    pub cap_match_score: f32,
    /// Latency score (0.0–1.0). `1.0 - min(rtt_ms, 500) / 500`. Local
    /// peers (rtt < 10 ms) ≈ 0.98; remote LAN (rtt ~50 ms) ≈ 0.90;
    /// degraded (rtt > 500 ms) clamps to 0.0.
    pub latency_score: f32,
    /// Load score (0.0–1.0). `1.0 - in_flight_tasks / 4`. Idle peer = 1.0;
    /// 4+ in-flight = 0.0 (clamped). Stage 2 reads `in_flight` from peer
    /// status broadcast.
    pub load_score: f32,
    /// Recent-failure penalty (negative; typical range [-0.25, 0.0]).
    /// `-0.05 × failures_in_last_5_min` clamped to `-0.25`. Decays as the
    /// 5-minute window slides forward.
    pub recent_failure_penalty: f32,
}

// ─── §8 DispatchStatus — state machine terminals + in-flight states ──────

/// Terminal + in-flight states for a dispatch attempt. Wire shape is
/// snake_case (`"planned"`, `"dispatched"`, …) to match SPEC-26 §8 state
/// machine vocabulary. The UI dispatches on these strings to pick the
/// right progress-card icon.
///
/// 中文: 一次 dispatch 的狀態列舉。對應 §8 state machine 的五個正常 state
/// + 三個結束 state。snake_case 是線路真值（wire truth），UI 直接 switch
/// 字串 render 對應圖示。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export, export_to = "../../app/src/lib/generated/cluster_dispatch/")]
#[serde(rename_all = "snake_case")]
pub enum DispatchStatus {
    /// `plan_dispatch` succeeded; `execute_plan` not yet called.
    Planned,
    /// `/rpc/task/assign` POST sent; awaiting peer ack.
    Dispatched,
    /// Peer acked; task executing on remote.
    Running,
    /// Peer reported success; `result_summary` populated.
    Completed,
    /// Peer reported task-level failure (post-dispatch). Distinct from
    /// `DispatchError` which fires before/during dispatch RPC.
    Failed,
    /// Wall-clock budget exceeded (deadline_ms or 90 s default).
    Timeout,
    /// `plan_dispatch` returned `NoMatchingPeer` — no candidate satisfied
    /// the required_caps + score ≥ 0.1 threshold.
    NoCandidate,
}

// ─── §11 DispatchError — pre-dispatch + RPC-level failures ──────────────

/// Errors raised by `score_peer`, `plan_dispatch`, `execute_plan`, and
/// `refresh_capabilities`. Distinct from `DispatchStatus::Failed` which is
/// a task-level failure reported **by** the executing peer. Wire shape
/// uses serde-tagged `{"code": "..."}` so the UI can dispatch on a
/// machine-readable string.
///
/// 中文: 派工流程本身（pre-dispatch / RPC 層）的錯誤。
/// 跟 `DispatchStatus::Failed`（peer 跑完回報 task 失敗）不同 — 這裡是
/// 派工這件事本身失敗。對應 SPEC-26 §11 error catalog 五碼。
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export, export_to = "../../app/src/lib/generated/cluster_dispatch/")]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum DispatchError {
    /// No peer in the cluster satisfies the required_caps set (or all
    /// candidates scored below the 0.1 threshold). Maps to `NoCandidate`
    /// terminal status. UI: "找不到符合能力的 peer，請開另一台 spectyn serve"。
    NoMatchingPeer,
    /// All matching peers are currently saturated (in_flight ≥ 4). User
    /// should retry after current dispatches drain. Stage 2 may auto-retry
    /// once with `tokio::time::sleep(2 s)`.
    AllPeersBusy,
    /// Wall-clock budget exceeded during `execute_plan` walk. The
    /// `deadline_ms` field of the originating `DispatchTask` (or the
    /// 90 s default) tripped.
    RouteTimeout,
    /// HMAC signature on `/rpc/task/assign` rejected by remote peer
    /// (cluster_secret mismatch or replay). Fatal — likely cluster split
    /// brain or compromised secret; user must re-pair.
    DispatchAuthFailed,
    /// Task payload (`DispatchTask.payload` serialized) exceeded the
    /// per-RPC 16 KB cap (SPEC-26 §3.1 G1 input budget). Caller must
    /// shrink the payload (e.g. drop verbose context) and retry.
    PayloadTooLarge,
}

// --- SPEC-26 #2 decompose: master rule-based task split (no LLM) ---

/// Which tri-role a subtask is targeted at (SPEC-26 master / coder / researcher).
/// `Master` is the fallback: when an input matches no role keywords the master
/// node runs it itself (degenerate single-node path).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/cluster_dispatch/")]
#[serde(rename_all = "snake_case")]
pub enum DispatchRole {
    Master,
    Coder,
    Researcher,
    /// Non-author second-AI reviewer (CROSS-REVIEW-AUTOMATION spec §2.1).
    /// Wire string is the deterministic `"reviewer"` via the existing
    /// `#[serde(rename_all = "snake_case")]` derive — additive only; existing
    /// dispatch paths (`decompose`, `assign_subtasks` match arms) remain
    /// unchanged. The merge gate constructs Reviewer subtasks directly via
    /// `assign_subtasks`, bypassing `decompose` per spec §2.4.
    Reviewer,
}

/// One unit the master split a user input into (SPEC-26 #2). `required_caps`
/// are the capability tags a peer must advertise to receive this subtask
/// (reused by `plan_dispatch` for tag matching). Stage 1 keeps `prompt` = the
/// original input text; smarter slicing is SPEC-27's job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/cluster_dispatch/")]
#[serde(rename_all = "camelCase")]
pub struct Subtask {
    pub role: DispatchRole,
    pub prompt: String,
    pub required_caps: Vec<CapabilityTag>,
}

// Keyword sets are matched by `matches_any` below: single ASCII tokens match on
// WHOLE-WORD boundaries (so "struct" does NOT fire on "infrastructure"), while
// entries containing a space (phrases) or any non-ASCII char (CJK) match by
// substring. Kept as const arrays so the rule set is auditable + unit-testable.

/// Code-related trigger words -> a `Coder` subtask.
const CODER_KEYWORDS: &[&str] = &[
    "refactor", "cargo", "async", "fn", "impl", "struct", "trait", "compile",
    "build", "cargo test", "git", "bug", "debug", "clippy", "lint", "function",
    "rustc", "borrow checker", "重構", "程式", "編譯", "函式", "除錯",
];

/// Research-related trigger words -> a `Researcher` subtask.
const RESEARCHER_KEYWORDS: &[&str] = &[
    "how", "why", "what is", "best practice", "compare", "research", "find out",
    "explain", "docs", "documentation", "look up", "search", "查", "為什麼",
    "怎麼", "文件", "解釋", "比較", "研究",
];

/// Match `lower` (already lower-cased input) against a keyword set. Single ASCII
/// tokens must match a WHOLE word (input split on non-alphanumeric chars) to
/// avoid substring traps (codex review: "struct" must not fire on
/// "infrastructure", "compare" not on "comparable", etc.). Phrases (containing a
/// space) and CJK terms (no ASCII word boundaries) fall back to substring match.
fn matches_any(lower: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|k| {
        if k.contains(' ') || !k.is_ascii() {
            lower.contains(k)
        } else {
            lower
                .split(|c: char| !c.is_alphanumeric())
                .any(|word| word == *k)
        }
    })
}

/// Build the capability tags a given role's peer must advertise.
fn role_required_caps(role: DispatchRole) -> Vec<CapabilityTag> {
    let slugs: &[&str] = match role {
        DispatchRole::Coder => &["role-coder", "cargo", "git"],
        DispatchRole::Researcher => &["role-researcher", "webSearch"],
        DispatchRole::Master => &[],
        // CROSS-REVIEW-AUTOMATION spec §2.2: a Reviewer subtask must land on a
        // peer that opted into the reviewer pool (`role-reviewer`), can fetch
        // the diff read-only (`git`), and has the canonical green-gate primitive
        // registered (`dev-verify`). plan_dispatch's existing required-cap
        // filter (line ~965) keeps Coder-only / unverified peers out of the pool.
        DispatchRole::Reviewer => &["role-reviewer", "git", "dev-verify"],
    };
    slugs
        .iter()
        .map(|s| CapabilityTag {
            slug: (*s).to_string(),
            value: None,
        })
        .collect()
}

/// SPEC-26 #2 (G1): deterministically split one user input into 1-2
/// role-targeted subtasks via pure keyword matching (NO LLM). Code keywords
/// emit a `Coder` subtask; research keywords emit a `Researcher` subtask; both
/// -> two subtasks (coder first, stable order) to run in parallel; neither ->
/// a single `Master` subtask the originating node runs itself. SPEC-27 layers
/// LLM-based decomposition on top later.
pub fn decompose(input: &str) -> Vec<Subtask> {
    let lower = input.to_lowercase();
    let wants_coder = matches_any(&lower, CODER_KEYWORDS);
    let wants_researcher = matches_any(&lower, RESEARCHER_KEYWORDS);

    let mut out = Vec::new();
    if wants_coder {
        out.push(Subtask {
            role: DispatchRole::Coder,
            prompt: input.to_string(),
            required_caps: role_required_caps(DispatchRole::Coder),
        });
    }
    if wants_researcher {
        out.push(Subtask {
            role: DispatchRole::Researcher,
            prompt: input.to_string(),
            required_caps: role_required_caps(DispatchRole::Researcher),
        });
    }
    if out.is_empty() {
        out.push(Subtask {
            role: DispatchRole::Master,
            prompt: input.to_string(),
            required_caps: Vec::new(),
        });
    }
    out
}

// --- SPEC-26 #3 capability-match: bridge decompose() -> plan_dispatch() ---

/// One subtask paired with the peer the capability scorer chose for it. `plan`
/// is `Some` when a capable peer was found; otherwise `error` records why
/// (e.g. `NoMatchingPeer`) so the master can surface "no peer can do X" per
/// subtask instead of failing the whole dispatch.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/cluster_dispatch/")]
#[serde(rename_all = "camelCase")]
pub struct SubtaskAssignment {
    /// Deterministic id `base-{role}-{index}`. Present on BOTH the success and
    /// the gap path (review: codex) so the master can correlate a subtask with
    /// its outcome even when no capable peer was found.
    pub task_id: String,
    pub subtask: Subtask,
    pub plan: Option<DispatchPlan>,
    pub error: Option<DispatchError>,
}

/// SPEC-26 #2 -> #3 bridge: for each subtask from [`decompose`], build a
/// [`DispatchTask`] from its `required_caps` (+ prompt as the opaque payload)
/// and run the existing [`plan_dispatch`] capability scorer to pick a peer.
/// `base_task_id` is suffixed with role + index to give each subtask a stable,
/// deterministic task id (no uuid dependency at this layer). The order mirrors
/// `subtasks` (coder before researcher, per decompose).
pub fn assign_subtasks(
    base_task_id: &str,
    subtasks: &[Subtask],
    peers: &[PeerCapabilities],
) -> Vec<SubtaskAssignment> {
    subtasks
        .iter()
        .enumerate()
        .map(|(i, st)| {
            let role_slug = match st.role {
                DispatchRole::Master => "master",
                DispatchRole::Coder => "coder",
                DispatchRole::Researcher => "researcher",
                DispatchRole::Reviewer => "reviewer",
            };
            let task_id = format!("{base_task_id}-{role_slug}-{i}");
            let task = DispatchTask {
                task_id: task_id.clone(),
                required_caps: st.required_caps.clone(),
                preferred_caps: Vec::new(),
                payload: st.prompt.clone(),
                deadline_ms: None,
            };
            match plan_dispatch(&task, peers) {
                Ok(plan) => SubtaskAssignment {
                    task_id,
                    subtask: st.clone(),
                    plan: Some(plan),
                    error: None,
                },
                Err(e) => SubtaskAssignment {
                    task_id,
                    subtask: st.clone(),
                    plan: None,
                    error: Some(e),
                },
            }
        })
        .collect()
}

// --- SPEC-26 #5 integrate: fold per-subtask outcomes into a master result ---

/// Deterministic master-side integration of all subtask outcomes (SPEC-26 #5).
/// No LLM: renders a stable markdown summary, counts succeeded/failed, and
/// reports `total_latency_ms` as the PARALLEL wall-clock span
/// (`max(completed_at_ms) - min(started_at_ms)`) since subtasks run
/// concurrently (0 when nothing completed).
// NOTE: `Eq` is intentionally NOT derived — `total_cost_usd` is `f64` (not `Eq`).
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[ts(export, export_to = "../../app/src/lib/generated/cluster_dispatch/")]
#[serde(rename_all = "camelCase")]
pub struct IntegratedResult {
    pub markdown: String,
    pub succeeded: usize,
    pub failed: usize,
    pub total_latency_ms: u64,
    /// Sum of every subtask's `cost_usd` (SPEC-26 J5 — the "$0.0238 across N
    /// machines" headline). Per the G6 invariant the master decompose/integrate
    /// step costs 0, so this equals exactly the sum of per-subtask costs.
    #[serde(default)]
    pub total_cost_usd: f64,
}

/// Fold subtask [`DispatchOutcome`]s into one [`IntegratedResult`]. Pure +
/// deterministic (order follows `outcomes`). `Completed` counts as success;
/// `Failed`/`Timeout`/`NoCandidate` count as failed; non-terminal states
/// (`Planned`/`Dispatched`/`Running`) count as neither.
pub fn integrate(outcomes: &[DispatchOutcome]) -> IntegratedResult {
    let succeeded = outcomes
        .iter()
        .filter(|o| matches!(o.status, DispatchStatus::Completed))
        .count();
    let failed = outcomes
        .iter()
        .filter(|o| {
            matches!(
                o.status,
                DispatchStatus::Failed | DispatchStatus::Timeout | DispatchStatus::NoCandidate
            )
        })
        .count();

    // Wall-clock span of the work that REACHED a terminal state: earliest start
    // among outcomes that carry a completion timestamp -> latest completion.
    // Both bounds range over the SAME (terminal) set, so an unfinished subtask
    // that started early cannot inflate the span (review: codex). The `c >= s`
    // guard also yields 0 on malformed (completed < started) timestamps.
    let max_completed = outcomes.iter().filter_map(|o| o.completed_at_ms).max();
    let min_started = outcomes
        .iter()
        .filter(|o| o.completed_at_ms.is_some())
        .map(|o| o.started_at_ms)
        .min();
    let total_latency_ms = match (min_started, max_completed) {
        (Some(s), Some(c)) if c >= s => c - s,
        _ => 0,
    };

    let mut markdown = format!(
        "# Dispatch result ({} subtask(s): {} ok, {} failed)\n",
        outcomes.len(),
        succeeded,
        failed
    );
    for o in outcomes {
        let detail = o
            .result_summary
            .as_deref()
            .or(o.error.as_deref())
            .unwrap_or("(no output)");
        markdown.push_str(&format!(
            "- `{}` @ {}: {:?} - {}\n",
            o.task_id, o.executed_by_peer_id, o.status, detail
        ));
    }

    // G6 invariant: the master decompose/integrate step itself costs 0, so the
    // total is exactly the sum of per-subtask costs.
    let total_cost_usd = outcomes.iter().map(|o| o.cost_usd).sum();

    IntegratedResult {
        markdown,
        succeeded,
        failed,
        total_latency_ms,
        total_cost_usd,
    }
}

// ─── SPEC-26 master orchestrator (the tri-role loop) ─────────────────────
//
// decompose() / assign_subtasks() / integrate() are the pieces; THIS is the
// loop that chains them and fans the matched subtasks out to peers IN PARALLEL.
// The per-subtask peer execution sits behind `SubtaskRunner` so the loop is
// unit-testable with a mock runner (no network). Stage 2 supplies the real
// runner that delegates to `execute_plan` (HMAC POST + poll + fallback).

use async_trait::async_trait;

/// Runs ONE subtask's [`DispatchPlan`] to a terminal [`DispatchOutcome`].
/// Implemented for real by an RPC runner (Stage 2, delegating to
/// [`execute_plan`]); mocked in tests so the orchestrator loop is verifiable
/// without a live cluster. `Sync` so `&dyn`/`&R` is `Send`-shareable across the
/// parallel `join_all`.
#[async_trait]
pub trait SubtaskRunner: Sync {
    async fn run(&self, plan: &DispatchPlan) -> DispatchOutcome;
}

/// SPEC-26 master orchestrator loop (#2 → #3 → #4 → #5): decompose `input` into
/// role-targeted subtasks, capability-match each to a peer, run the matched
/// subtasks **in parallel** via `runner`, then `integrate` the outcomes into one
/// master result. A subtask with no capable peer (`plan: None`) becomes a
/// `NoCandidate` outcome (degraded path, SPEC-26 J6) rather than aborting the
/// whole dispatch — the other role still runs. `runner` is injected so this is
/// testable without a cluster; `base_task_id` is caller-supplied for
/// deterministic subtask ids (Stage 2/3 pass a fresh uuid).
pub async fn run_dispatch_with<R: SubtaskRunner>(
    input: &str,
    base_task_id: &str,
    peers: &[PeerCapabilities],
    runner: &R,
) -> IntegratedResult {
    let subtasks = decompose(input);
    let assignments = assign_subtasks(base_task_id, &subtasks, peers);
    // One future per subtask; join_all runs them concurrently (SPEC-26 G4).
    let outcomes = futures::future::join_all(assignments.into_iter().map(|a| async move {
        match a.plan {
            Some(plan) => runner.run(&plan).await,
            None => DispatchOutcome {
                task_id: a.task_id,
                executed_by_peer_id: String::new(),
                status: DispatchStatus::NoCandidate,
                started_at_ms: 0,
                completed_at_ms: None,
                result_summary: None,
                error: Some(
                    a.error
                        .map(|e| format!("{:?}", e))
                        .unwrap_or_else(|| "no capable peer for subtask".to_string()),
                ),
                cost_usd: 0.0,
            },
        }
    }))
    .await;
    integrate(&outcomes)
}

/// Current epoch milliseconds (saturating to 0 on a pre-epoch clock).
fn epoch_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Map an [`execute_plan`] failure to a TERMINAL [`DispatchOutcome`] so a failed
/// subtask still shows up in [`integrate`]'s tally instead of being silently
/// dropped. `NoMatchingPeer` → `NoCandidate`, `RouteTimeout` → `Timeout`, all
/// other pre/post-dispatch errors → `Failed`.
fn dispatch_error_to_outcome(
    task_id: &str,
    peer_id: &str,
    started_at_ms: u64,
    err: &DispatchError,
) -> DispatchOutcome {
    let status = match err {
        DispatchError::NoMatchingPeer => DispatchStatus::NoCandidate,
        DispatchError::RouteTimeout => DispatchStatus::Timeout,
        DispatchError::AllPeersBusy
        | DispatchError::DispatchAuthFailed
        | DispatchError::PayloadTooLarge => DispatchStatus::Failed,
    };
    DispatchOutcome {
        task_id: task_id.to_string(),
        executed_by_peer_id: peer_id.to_string(),
        status,
        started_at_ms,
        completed_at_ms: Some(epoch_ms_now()),
        result_summary: None,
        error: Some(format!("{:?}", err)),
        cost_usd: 0.0,
    }
}

/// Production [`SubtaskRunner`]: dispatches a plan to a real peer via
/// [`execute_plan`] (HMAC POST → status poll → one fallback hop). On error it
/// maps to a terminal outcome (never drops the subtask). The network success
/// path is covered by `execute_plan`'s own tests; this wrapper only adds the
/// error→outcome mapping (unit-tested) so the orchestrator never aborts.
pub struct RpcRunner;

#[async_trait]
impl SubtaskRunner for RpcRunner {
    async fn run(&self, plan: &DispatchPlan) -> DispatchOutcome {
        let started = epoch_ms_now();
        match execute_plan(plan).await {
            Ok(outcome) => outcome,
            Err(e) => {
                dispatch_error_to_outcome(&plan.task_id, &plan.selected_peer_id, started, &e)
            }
        }
    }
}

/// SPEC-26 production entry point: orchestrate `input` across the live `peers`
/// using the real RPC runner ([`RpcRunner`]). Thin wrapper over
/// [`run_dispatch_with`]; the CLI/Tauri entry points (Stage 3/4) call this.
pub async fn run_dispatch(
    input: &str,
    base_task_id: &str,
    peers: &[PeerCapabilities],
) -> IntegratedResult {
    run_dispatch_with(input, base_task_id, peers, &RpcRunner).await
}

// ─── §9 Stub helpers — Stage 2 will fill ────────────────────────────────

/// Score one peer against one task. Stage 2 implements §6.2 weighted-sum
/// formula:
///
/// ```text
/// score = cap_match × 0.5
///       + latency_score × 0.3
///       + load_score × 0.15
///       + recent_failure_penalty × 0.05  // negative contributor
/// ```
///
/// Missing any `required_caps` tag → caller should exclude this peer
/// **before** calling `score_peer` (this fn does not check requireds).
///
/// 中文: 對單一 peer 跟單一 task 算分。Stage 2 接 §6.2 加權公式。
/// 缺 required tag 的 peer 應由呼叫端（`plan_dispatch`）先過濾掉，
/// 不要進 `score_peer`。
pub fn score_peer(peer_caps: &PeerCapabilities, task: &DispatchTask) -> PeerScore {
    // Step 1 — cap_match_score: Jaccard-style intersect of required + preferred
    // caps with peer.tags (pure set math; no I/O).
    let cap_match = tag_intersect(
        &task.required_caps,
        &task.preferred_caps,
        &peer_caps.tags,
    );

    // Step 2 — latency_score: freshness proxy from `last_reported_at`. Newer
    // ping (low age) ≈ higher score. Pure `std::time` arithmetic — no live RTT
    // sampler is wired here yet, but the proxy is faithful enough for §6.2
    // ranking so the formula stays comparable across peers.
    let latency = latency_from_last_ping(peer_caps.last_reported_at);

    // Step 3 — load_score: count peer's currently in-flight tasks via the
    // rpc_wire `/rpc/peers` status broadcast. Stage 4 integration point —
    // panics with a Stage 4 marker so a half-wired execute_plan path is
    // surfaced loudly during CI, not silently masked.
    let load = peer_active_load(&peer_caps.peer_id);

    // Step 4 — recent_failure_penalty: rolling 5-minute failure window from
    // the master's local audit log. Stage 4 integration point.
    let penalty = failure_history(&peer_caps.peer_id);

    // Step 5 — weighted sum per SPEC-26 §6.2:
    //   score = cap_match × 0.5 + latency × 0.3 + load × 0.15 + penalty × 0.05
    let aggregate: f32 =
        cap_match * 0.5 + latency * 0.3 + load * 0.15 + penalty * 0.05;
    // Clamp to [0.0, 1.0] — the only negative contributor is `penalty`, but
    // we still defensively clamp because callers downstream (UI tooltips,
    // sorters) assume the closed interval.
    let score = aggregate.clamp(0.0, 1.0);

    PeerScore {
        peer_id: peer_caps.peer_id.clone(),
        score,
        breakdown: ScoreBreakdown {
            cap_match_score: cap_match,
            latency_score: latency,
            load_score: load,
            recent_failure_penalty: penalty,
        },
    }
}

// ─── Stage 3 helpers — pure score-peer dimensions ───────────────────────

/// Compute the §6.2 cap-match score: `|required ∩ peer| / |required| × 0.7 +
/// |preferred ∩ peer| / max(1, |preferred|) × 0.3`. Pure set math, no I/O.
///
/// `CapabilityTag` membership is determined by structural equality (`slug` +
/// `value`), which is what `HashSet<&CapabilityTag>` gives us via the
/// `PartialEq + Eq + Hash` derives on the type itself.
///
/// Edge cases:
///   * Empty `required` and empty `preferred` → returns `1.0` (no constraints,
///     any peer satisfies vacuously). This is the soft-default for tagless
///     tasks the scorer should not penalise.
///   * Empty `required` but non-empty `preferred` → required term contributes
///     `0.7` (vacuously), preferred term contributes per Jaccard.
///   * Empty `preferred` but non-empty `required` → preferred term contributes
///     `0.3` (vacuously via the `max(1, |preferred|)` clamp).
///
/// 中文: 算 capability tag 的交集分數（required 權 0.7、preferred 權 0.3）。
fn tag_intersect(
    required: &[CapabilityTag],
    preferred: &[CapabilityTag],
    peer_tags: &[CapabilityTag],
) -> f32 {
    let peer_set: HashSet<&CapabilityTag> = peer_tags.iter().collect();

    let required_term: f32 = if required.is_empty() {
        0.7
    } else {
        let hit = required.iter().filter(|t| peer_set.contains(*t)).count();
        (hit as f32 / required.len() as f32) * 0.7
    };
    let preferred_term: f32 = if preferred.is_empty() {
        0.3
    } else {
        let hit = preferred.iter().filter(|t| peer_set.contains(*t)).count();
        (hit as f32 / preferred.len() as f32) * 0.3
    };
    (required_term + preferred_term).clamp(0.0, 1.0)
}

/// Derive the §6.2 latency proxy from `last_reported_at`. Newer ping (small
/// age) ≈ higher score. Uses `SystemTime` for the wall-clock reference;
/// monotonic-clock RTT samples will replace this when the rpc_wire ping
/// subsystem lands (Stage 4 swap target).
///
/// Mapping (matches the SPEC §6.2 spirit `1.0 - min(rtt_ms, 500) / 500`):
///   * age ≤ 0 ms (clock skew / future ping) → `1.0`
///   * age ≥ 500 ms                          → `0.0` (clamped)
///   * 0 < age < 500 ms                      → `1.0 - age/500`
///
/// 中文: 用 `last_reported_at` 當 latency proxy；Stage 4 接真實 RTT 量測。
fn latency_from_last_ping(last_reported_at: u64) -> f32 {
    let now_ms: u128 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let last_ms: u128 = last_reported_at as u128;
    // Saturating subtraction — clock skew (last > now) clamps to 0 age.
    let age_ms: u128 = now_ms.saturating_sub(last_ms);
    let bounded_ms: f32 = age_ms.min(500) as f32;
    1.0 - bounded_ms / 500.0
}

// ─── Stage 3 process-local PeerRegistry cache (OnceLock + RwLock) ───────
//
// The master keeps a per-process snapshot of every peer's capability +
// in-flight load + recent failure count. The cache key is `peer_id`. We
// use `std::sync::OnceLock<tokio::sync::RwLock<...>>` so:
//   • no third-party crate is added (the task forbids touching Cargo.toml)
//   • reads from sync contexts (`score_peer`, `plan_dispatch`) can still
//     observe the cache via `try_read` — they degrade to a sensible
//     default (idle load = 1.0, no penalty = 0.0) when a write holder
//     happens to be in-flight, so the scoring path never blocks the
//     agent runtime hot loop on RPC.
//   • async callers (`refresh_capabilities`, `execute_plan` Stage 4)
//     use the awaiting `.read().await` / `.write().await` for full
//     consistency.

/// One row in the master's per-peer cache. `capabilities` is the most
/// recent advertisement; `in_flight` is the count of dispatched tasks
/// that haven't reported a terminal status; `failures_last_5_min` is a
/// short rolling counter the scorer turns into a negative penalty.
#[derive(Debug, Clone, Default)]
struct PeerCacheEntry {
    capabilities: Option<PeerCapabilities>,
    in_flight: u8,
    failures_last_5_min: u8,
}

fn peer_registry() -> &'static RwLock<HashMap<String, PeerCacheEntry>> {
    static REGISTRY: OnceLock<RwLock<HashMap<String, PeerCacheEntry>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Read the peer's current in-flight task count from the master's local
/// PeerRegistry cache and turn it into the §6.2 load score. Pure local
/// lookup — no RPC. The cache is refreshed by `refresh_capabilities` +
/// `execute_plan` (the dispatch path bumps `in_flight` on assign and
/// decrements on terminal). Falls back to 1.0 (idle) when the cache has
/// no entry for this peer or when a writer holds the lock — the scorer
/// must never block the agent runtime hot path waiting on a sibling.
///
/// Mapping (matches §6.2): `1.0 - in_flight / 4`, clamped to `[0.0, 1.0]`.
///
/// 中文: 從 master 本地 PeerRegistry 快取讀該 peer 目前 in-flight 數;
/// `try_read` 失敗（lock contended）退回 1.0（idle），不阻塞 hot path。
fn peer_active_load(peer_id: &str) -> f32 {
    let guard = match peer_registry().try_read() {
        Ok(g) => g,
        // Writer in flight — fall back to "idle" so the scorer keeps moving.
        Err(_) => return 1.0,
    };
    let in_flight = guard
        .get(peer_id)
        .map(|e| e.in_flight)
        .unwrap_or(0);
    let s = 1.0_f32 - (in_flight as f32) / 4.0;
    s.clamp(0.0, 1.0)
}

/// Count this peer's failures in the rolling 5-minute window from the
/// master's local PeerRegistry cache; returns a negative penalty in
/// `[-0.25, 0.0]` per the §6.2 formula `-0.05 × failures_last_5_min`.
///
/// The cache is bumped by `execute_plan` on each dispatched-task failure
/// and decayed by a SPEC-16 audit-log read pass (Stage 4 — the decay
/// loop itself lives in the master orchestrator, not in this wire). For
/// now the field stays at zero until a real failure is recorded, so the
/// penalty contributes nothing to scoring until the dispatcher lights up.
///
/// 中文: 從 PeerRegistry 快取讀該 peer 最近 5 分鐘失敗數，回負值扣分。
fn failure_history(peer_id: &str) -> f32 {
    let guard = match peer_registry().try_read() {
        Ok(g) => g,
        Err(_) => return 0.0,
    };
    let failures = guard
        .get(peer_id)
        .map(|e| e.failures_last_5_min)
        .unwrap_or(0);
    (-0.05_f32 * failures as f32).clamp(-0.25, 0.0)
}

/// Compute the best dispatch plan for `task` given the current set of
/// `peers` (capability snapshots). Stage 2 algorithm:
///
/// 1. Filter `peers` to those advertising **all** `task.required_caps`.
/// 2. If filtered set empty → `Err(DispatchError::NoMatchingPeer)`.
/// 3. Score each remaining peer with `score_peer`.
/// 4. Sort by `score` descending.
/// 5. If top score < 0.1 → `Err(DispatchError::NoMatchingPeer)`.
/// 6. Build `DispatchPlan { selected = top, fallback = rest }`.
///
/// 中文: 給定 task 與當下 peer capability snapshot 集，回最佳派工計畫。
/// 順序：篩 required → 全部 score → 排序 → 取頂 + fallback chain。
pub fn plan_dispatch(
    task: &DispatchTask,
    peers: &[PeerCapabilities],
) -> Result<DispatchPlan, DispatchError> {
    // Step 1 — filter peers to those advertising every required cap. Empty
    // `required_caps` is vacuously satisfied by all peers.
    let required_set: HashSet<&CapabilityTag> = task.required_caps.iter().collect();
    let qualified: Vec<&PeerCapabilities> = peers
        .iter()
        .filter(|p| {
            let peer_set: HashSet<&CapabilityTag> = p.tags.iter().collect();
            required_set.iter().all(|t| peer_set.contains(*t))
        })
        .collect();
    if qualified.is_empty() {
        return Err(DispatchError::NoMatchingPeer);
    }

    // Step 2 — score every qualified peer (uses `score_peer`; Stage 4
    // dependencies on /rpc/peers + audit log will panic until wired).
    let mut scored: Vec<PeerScore> = qualified
        .iter()
        .map(|p| score_peer(p, task))
        .collect();

    // Step 3 — sort descending by `score` using `total_cmp` so NaN sinks to
    // the tail and never wins selection.
    scored.sort_by(|a, b| b.score.total_cmp(&a.score));

    // Step 4 — enforce the ≥ 0.1 minimum on the top candidate. Anything lower
    // means "no peer is actually a good fit" → NoMatchingPeer.
    let top = scored.first().ok_or(DispatchError::NoMatchingPeer)?;
    if top.score < 0.1 {
        return Err(DispatchError::NoMatchingPeer);
    }

    // Step 5 — build the plan. Cap fallback chain at 2 entries per SPEC-26
    // §8 "max 1 reassign" tractability guard.
    let selected_peer_id = top.peer_id.clone();
    let fallback_peer_ids: Vec<String> = scored
        .iter()
        .skip(1)
        .take(2)
        .map(|s| s.peer_id.clone())
        .collect();
    let scoring_reason = format!(
        "cap_match {:.2} + latency {:.2} + load {:.2} + penalty {:.2} → {:.2}",
        top.breakdown.cap_match_score,
        top.breakdown.latency_score,
        top.breakdown.load_score,
        top.breakdown.recent_failure_penalty,
        top.score,
    );
    let planned_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    Ok(DispatchPlan {
        task_id: task.task_id.clone(),
        selected_peer_id,
        fallback_peer_ids,
        scoring_reason,
        planned_at_ms,
    })
}

/// Execute a previously-computed plan against the live mesh. Stage 3
/// wire-up: assigns the selected peer via real HMAC-signed reqwest POST,
/// polls status, walks the fallback chain on failure. The age-encryption
/// of `task.payload` (SPEC-13) is **not** in this path — that's the
/// cross-peer privacy envelope which lives one layer up in the agent
/// runtime. `cluster_secret` is read from `SPECTYN_CLUSTER_SECRET` env
/// var; absent → `DispatchAuthFailed` (cluster not bootstrapped).
///
/// Algorithm:
///   1. Try `plan.selected_peer_id` via `rpc_post("/rpc/task/assign")`
///   2. On HMAC reject → `DispatchAuthFailed` (no fallback — secret bad)
///   3. On other failure: walk `plan.fallback_peer_ids` (max 1 reassign)
///   4. On accept: `rpc_poll_status` every 2 s until terminal / deadline
///   5. Build `DispatchOutcome`; map deadline exceed → `RouteTimeout`,
///      all peers refused → `AllPeersBusy`.
///
/// 中文: 真正派工 — HMAC POST、輪詢 status、失敗走 fallback chain。
pub async fn execute_plan(plan: &DispatchPlan) -> Result<DispatchOutcome, DispatchError> {
    let started_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    // Build the ordered candidate list: selected first, then fallbacks.
    // SPEC-26 §8 caps the reassign walk at one — selected + first
    // fallback only. Any further entries are ignored at this layer.
    let mut candidates: Vec<&str> = Vec::with_capacity(2);
    candidates.push(plan.selected_peer_id.as_str());
    if let Some(fb) = plan.fallback_peer_ids.first() {
        candidates.push(fb.as_str());
    }

    let mut last_err: DispatchError = DispatchError::AllPeersBusy;
    for peer_id in candidates {
        match rpc_post(peer_id, "/rpc/task/assign").await {
            Ok(()) => {
                // Bump the in-flight counter so concurrent scorers see
                // the load delta immediately.
                {
                    let mut guard = peer_registry().write().await;
                    let entry = guard.entry(peer_id.to_string()).or_default();
                    entry.in_flight = entry.in_flight.saturating_add(1);
                }
                // Step 4: poll status. On terminal Completed → success.
                let status = match rpc_poll_status(&plan.task_id).await {
                    Ok(s) => s,
                    Err(e) => {
                        // Decrement on failure too — peer is done with it.
                        let mut guard = peer_registry().write().await;
                        if let Some(entry) = guard.get_mut(peer_id) {
                            entry.in_flight = entry.in_flight.saturating_sub(1);
                            entry.failures_last_5_min =
                                entry.failures_last_5_min.saturating_add(1);
                        }
                        last_err = e;
                        continue;
                    }
                };
                // Decrement on terminal.
                {
                    let mut guard = peer_registry().write().await;
                    if let Some(entry) = guard.get_mut(peer_id) {
                        entry.in_flight = entry.in_flight.saturating_sub(1);
                        if matches!(
                            status,
                            DispatchStatus::Failed | DispatchStatus::Timeout
                        ) {
                            entry.failures_last_5_min =
                                entry.failures_last_5_min.saturating_add(1);
                        }
                    }
                }
                let completed_at_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .ok();
                return Ok(DispatchOutcome {
                    task_id: plan.task_id.clone(),
                    executed_by_peer_id: peer_id.to_string(),
                    status,
                    started_at_ms,
                    completed_at_ms,
                    result_summary: None,
                    error: None,
                    // Real per-task cost from the peer's RPC TaskResult is a
                    // deferred cross-peer wiring follow-up; 0.0 until then.
                    cost_usd: 0.0,
                });
            }
            Err(DispatchError::DispatchAuthFailed) => {
                // Fatal — all peers share the same cluster_secret, so
                // walking fallbacks would just hit the same wall.
                return Err(DispatchError::DispatchAuthFailed);
            }
            Err(e) => {
                last_err = e;
                continue;
            }
        }
    }

    Err(last_err)
}

// ─── Stage 3 RPC primitives — real reqwest + HMAC envelope ──────────────

/// Resolve the base URL for a peer's HTTP RPC surface. Stage 3 reads
/// `SPECTYN_PEER_<PEERID>_URL` (with `-` replaced by `_`) from the
/// process env so tests can redirect to a local mock; falls back to a
/// conventional `http://<peer_id>:7878` (the canonical spectyn serve
/// port). Pure helper — no I/O.
fn peer_base_url(peer_id: &str) -> String {
    let env_key = format!(
        "SPECTYN_PEER_{}_URL",
        peer_id.to_uppercase().replace('-', "_")
    );
    if let Ok(u) = std::env::var(&env_key) {
        if !u.trim().is_empty() {
            return u;
        }
    }
    format!("http://{peer_id}:7878")
}

/// Read the cluster secret from the environment. SPEC-12 stores it on
/// disk under `~/.spectyn-mesh/cluster_secret`; that loader lives in a
/// sibling module so we accept the pre-loaded value via env here as the
/// Stage 3 boundary. Absent / empty → `DispatchAuthFailed` so the caller
/// returns the canonical error without a network round-trip.
fn load_cluster_secret() -> Result<Vec<u8>, DispatchError> {
    match std::env::var("SPECTYN_CLUSTER_SECRET") {
        Ok(s) if !s.trim().is_empty() => Ok(s.into_bytes()),
        _ => Err(DispatchError::DispatchAuthFailed),
    }
}

/// POST a JSON-bodied RPC to a peer's `/rpc/...` endpoint with the
/// cluster_secret HMAC envelope (SPEC-12 derivation). Uses
/// `crate::rpc_wire::sign_hmac` to compute the `X-Cluster-Auth`
/// header so all signing flows through one canonical builder.
///
/// 中文: 對 peer 發 HMAC 簽章的 RPC POST，走 rpc_wire::sign_hmac 統一簽章。
async fn rpc_post(peer_id: &str, path: &str) -> Result<(), DispatchError> {
    use crate::rpc_wire::{build_canonical_string, sign_hmac};

    let secret = load_cluster_secret()?;
    let url = format!("{}{}", peer_base_url(peer_id), path);

    // SPEC-26 task assign carries a JSON envelope; we ship an empty `{}`
    // here because the typed payload comes from `DispatchTask.payload`
    // (caller already serialised). Stage 4 hand-off threads that through.
    let body: &[u8] = b"{}";

    let canonical = build_canonical_string("POST", path, "", body, None);
    let sig = sign_hmac(&secret, &canonical);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| DispatchError::RouteTimeout)?;

    let resp = client
        .post(&url)
        .header("X-Cluster-Auth", sig)
        .header("Content-Type", "application/json")
        .body(body.to_vec())
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                DispatchError::RouteTimeout
            } else {
                DispatchError::AllPeersBusy
            }
        })?;

    let status = resp.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(DispatchError::DispatchAuthFailed);
    }
    if status.as_u16() == 413 {
        return Err(DispatchError::PayloadTooLarge);
    }
    if !status.is_success() {
        return Err(DispatchError::AllPeersBusy);
    }
    Ok(())
}

/// Poll a remote task's `/rpc/task/status/:id` every 2 s until a terminal
/// status is observed; returns the final `DispatchStatus`. Bounded at
/// 45 polls (≈ 90 s, the SPEC-26 §3.1 G4 default deadline). Returns
/// `RouteTimeout` when the bound is exceeded.
///
/// 中文: 每 2 秒輪詢遠端 task 狀態，最多 90 秒；超時回 RouteTimeout。
async fn rpc_poll_status(task_id: &str) -> Result<DispatchStatus, DispatchError> {
    use crate::rpc_wire::{build_canonical_string, sign_hmac};

    let secret = load_cluster_secret()?;
    // Stage 3 simplification: poll path on the local serve API (the
    // master keeps its own /rpc/task/status mirror that aggregates peer
    // updates). Real cross-peer broker handoff is Stage 4.
    let path = format!("/rpc/task/status/{task_id}");
    // The master polls its OWN local serve API for the task-status mirror. Base
    // defaults to the canonical local serve addr; SPECTYN_POLL_URL overrides it
    // (test seam → wiremock; also lets a non-default local serve port work).
    let poll_base = std::env::var("SPECTYN_POLL_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:7878".to_string());
    let url = format!("{}{path}", poll_base.trim_end_matches('/'));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|_| DispatchError::RouteTimeout)?;

    for _ in 0..45 {
        let canonical = build_canonical_string("GET", &path, "", b"", None);
        let sig = sign_hmac(&secret, &canonical);
        let resp = match client
            .get(&url)
            .header("X-Cluster-Auth", sig)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) if e.is_timeout() => return Err(DispatchError::RouteTimeout),
            Err(_) => return Err(DispatchError::AllPeersBusy),
        };

        if resp.status().as_u16() == 401 || resp.status().as_u16() == 403 {
            return Err(DispatchError::DispatchAuthFailed);
        }

        if let Ok(body) = resp.text().await {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(s) = v.get("status").and_then(|x| x.as_str()) {
                    match s {
                        "completed" => return Ok(DispatchStatus::Completed),
                        "failed" => return Ok(DispatchStatus::Failed),
                        "timeout" => return Ok(DispatchStatus::Timeout),
                        // Non-terminal — keep polling.
                        _ => {}
                    }
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    Err(DispatchError::RouteTimeout)
}

/// Map a `/node/capabilities` JSON body's `capability_ids` (Capability::id()
/// strings like `"shell"` / `"gpu_compute:metal"`) into `CapabilityTag`s,
/// splitting an optional `":value"` suffix (parametric ids get a value; plain
/// ids get `value: None`). Returns empty if `capability_ids` is missing/malformed.
fn capability_ids_to_tags(parsed: &serde_json::Value) -> Vec<CapabilityTag> {
    parsed
        .get("capability_ids")
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|id| match id.split_once(':') {
            Some((slug, value)) => CapabilityTag {
                slug: slug.to_string(),
                value: Some(value.to_string()),
            },
            None => CapabilityTag {
                slug: id,
                value: None,
            },
        })
        .collect()
}

/// Fetch the latest capability report for one peer via the canonical
/// `GET /node/capabilities` endpoint (the route spectyn serve actually
/// registers — see serve.rs `node_capabilities`; SPEC-10 §9.13's
/// `/rpc/capabilities/refresh` is the PUSH side, not a GET-pull path).
/// Stage 3: real reqwest GET + HMAC-signed envelope + JSON parse + local
/// PeerRegistry cache update. `last_reported_at` is stamped with the
/// **local** clock at receipt time (NOT the remote-reported timestamp —
/// that protects against clock skew between peers).
///
/// 中文: 主動向某 peer 拉最新 capability 報告 — 真實 HTTP GET + HMAC 簽章 +
/// JSON parse + 本地快取更新。`last_reported_at` 用本地時鐘蓋避免時鐘漂移。
pub async fn refresh_capabilities(peer_id: &str) -> Result<PeerCapabilities, DispatchError> {
    // Step 1 — HMAC-signed GET /node/capabilities on the named peer (the
    // server route; /rpc/capabilities does not exist and would 404).
    let body = rpc_get(peer_id, "/node/capabilities").await?;

    // Step 2 — parse the JSON body into PeerCapabilities; stamp
    // `last_reported_at` with the local clock at receipt time.
    let parsed: serde_json::Value = serde_json::from_str(&body)
        .map_err(|_| DispatchError::AllPeersBusy)?;
    let tags = capability_ids_to_tags(&parsed);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let snapshot = PeerCapabilities {
        peer_id: peer_id.to_string(),
        tags,
        last_reported_at: now_ms,
    };

    // Step 3 — write into the local PeerRegistry cache so subsequent
    // score_peer calls see the fresh snapshot without another RPC.
    cap_cache_update(peer_id, snapshot.clone()).await;

    Ok(snapshot)
}

// ─── Stage 3 refresh_capabilities primitives — reqwest + RwLock cache ──

/// GET a JSON-bodied RPC endpoint on a peer with cluster_secret HMAC
/// signature; returns the response body as `String`. Maps HTTP 401/403
/// to `DispatchAuthFailed`, timeout to `RouteTimeout`, everything else
/// to `AllPeersBusy` so the upstream walker can fall through.
///
/// 中文: HMAC 簽章的 GET RPC；走 rpc_wire::sign_hmac 統一簽章流程。
async fn rpc_get(peer_id: &str, path: &str) -> Result<String, DispatchError> {
    use crate::rpc_wire::{build_canonical_string, sign_hmac};

    let secret = load_cluster_secret()?;
    let url = format!("{}{}", peer_base_url(peer_id), path);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| DispatchError::RouteTimeout)?;

    let canonical = build_canonical_string("GET", path, "", b"", None);
    let sig = sign_hmac(&secret, &canonical);

    let resp = client
        .get(&url)
        .header("X-Cluster-Auth", sig)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                DispatchError::RouteTimeout
            } else {
                DispatchError::AllPeersBusy
            }
        })?;

    let status = resp.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(DispatchError::DispatchAuthFailed);
    }
    if !status.is_success() {
        return Err(DispatchError::AllPeersBusy);
    }
    resp.text().await.map_err(|_| DispatchError::AllPeersBusy)
}

/// Insert / replace the cached `PeerCapabilities` snapshot for one peer
/// in the master's local PeerRegistry (`OnceLock<RwLock<HashMap>>`).
/// Pure local write — no I/O. Idempotent; safe to call from multiple
/// awaiting tasks.
///
/// 中文: 用 await 寫入本地 PeerRegistry 快取，覆寫舊 snapshot。
async fn cap_cache_update(peer_id: &str, snapshot: PeerCapabilities) {
    let mut guard = peer_registry().write().await;
    let entry = guard.entry(peer_id.to_string()).or_default();
    entry.capabilities = Some(snapshot);
}

// ─── Smoke tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_ids_to_tags_maps_node_capabilities_payload() {
        // Regression: refresh_capabilities pulls /node/capabilities, whose body
        // is `{ capability_ids: [...] }` (NOT a `tags` array). The old parser
        // read `tags` and silently produced an empty Vec -> peers advertised
        // nothing -> cap_match_score=0. This pins the capability_ids mapping.
        let body = serde_json::json!({
            "schema_version": 1,
            "platform": "windows",
            "capability_ids": ["shell", "network", "gpu_compute:metal", "local_llm:llamacpp"],
            "capabilities": {}
        });
        let tags = capability_ids_to_tags(&body);
        assert_eq!(tags.len(), 4, "all 4 capability_ids must map to tags");
        // plain id -> value None
        assert!(tags.iter().any(|t| t.slug == "shell" && t.value.is_none()));
        assert!(tags.iter().any(|t| t.slug == "network" && t.value.is_none()));
        // parametric id "slug:value" -> split
        assert!(tags
            .iter()
            .any(|t| t.slug == "gpu_compute" && t.value.as_deref() == Some("metal")));
        assert!(tags
            .iter()
            .any(|t| t.slug == "local_llm" && t.value.as_deref() == Some("llamacpp")));
    }

    #[test]
    fn capability_ids_to_tags_empty_on_missing_or_bad() {
        // Missing capability_ids (e.g. an old/`tags`-shaped body) -> empty, no panic.
        assert!(capability_ids_to_tags(&serde_json::json!({"tags": []})).is_empty());
        assert!(capability_ids_to_tags(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn dispatch_plan_round_trip_smoke() {
        // SPEC-26 §7 invariant: `DispatchPlan` survives a Rust → JSON →
        // Rust round-trip byte-identical on all six fields. Any field
        // rename here is a wire-break that ripples into the UI + SPEC-27
        // typed-payload consumer (cycle-break boundary).
        let p = DispatchPlan {
            task_id: "01923f8e-7a4c-7000-8c2d-2b9f0e1d4a55".to_string(),
            selected_peer_id: "peer-mac-01".to_string(),
            fallback_peer_ids: vec!["peer-linux-02".to_string(), "peer-pi-03".to_string()],
            scoring_reason: "role-coder + cargo present; rtt 45ms; load 0/4".to_string(),
            planned_at_ms: 1_716_586_800_000,
        };
        let j = serde_json::to_string(&p).unwrap();
        let back: DispatchPlan = serde_json::from_str(&j).unwrap();
        assert_eq!(p.task_id, back.task_id);
        assert_eq!(p.selected_peer_id, back.selected_peer_id);
        assert_eq!(p.fallback_peer_ids, back.fallback_peer_ids);
        assert_eq!(p.scoring_reason, back.scoring_reason);
        assert_eq!(p.planned_at_ms, back.planned_at_ms);

        // Verify the wire shape uses camelCase keys (matches §7
        // TypeScript interface) — guards against accidental snake_case
        // regression that would break the generated TS binding.
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        let obj = v.as_object().expect("plan is JSON object");
        assert!(obj.contains_key("taskId"));
        assert!(obj.contains_key("selectedPeerId"));
        assert!(obj.contains_key("fallbackPeerIds"));
        assert!(obj.contains_key("scoringReason"));
        assert!(obj.contains_key("plannedAtMs"));
        assert_eq!(
            obj.len(),
            5,
            "DispatchPlan must stay 5 fields (SPEC-26 §7); got keys: {:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn capability_tag_value_none_serializes_as_null() {
        // Cycle-break + wire-stability invariant: `CapabilityTag` with
        // `value: None` must serialize to `{"slug": "...", "value": null}`
        // — not `{"slug": "..."}` (Option default skip would break the
        // round-trip for the SPEC-27 typed wrapper that introspects
        // both keys).
        let t = CapabilityTag {
            slug: "always-on".to_string(),
            value: None,
        };
        let j = serde_json::to_string(&t).unwrap();
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        let obj = v.as_object().expect("tag is JSON object");
        assert!(obj.contains_key("slug"));
        assert!(obj.contains_key("value"), "value key must be present even when None");
        assert!(obj.get("value").unwrap().is_null(), "None must serialize as JSON null");

        // Round-trip back to Rust preserves None.
        let back: CapabilityTag = serde_json::from_str(&j).unwrap();
        assert_eq!(back.slug, "always-on");
        assert_eq!(back.value, None);

        // And the Some(...) case carries the value through unchanged.
        let t2 = CapabilityTag {
            slug: "ram".to_string(),
            value: Some("16gb".to_string()),
        };
        let j2 = serde_json::to_string(&t2).unwrap();
        let back2: CapabilityTag = serde_json::from_str(&j2).unwrap();
        assert_eq!(back2.slug, "ram");
        assert_eq!(back2.value.as_deref(), Some("16gb"));
    }

    #[test]
    fn reviewer_role_caps_match_cross_review_spec() {
        // CROSS-REVIEW-AUTOMATION spec §2.1 + §2.2 — additive Reviewer variant.
        // Pins: wire string is the deterministic `"reviewer"` (snake_case derive),
        // and `role_required_caps(Reviewer)` yields exactly the three slugs the
        // spec names — `role-reviewer`, `git`, `dev-verify` — each with no
        // parametric `value`. Existing role bundles MUST stay untouched
        // (additive-only invariant).
        let j = serde_json::to_string(&DispatchRole::Reviewer).unwrap();
        assert_eq!(j, "\"reviewer\"", "wire string must be snake_case 'reviewer'");

        let caps = role_required_caps(DispatchRole::Reviewer);
        let slugs: Vec<&str> = caps.iter().map(|t| t.slug.as_str()).collect();
        assert_eq!(slugs, vec!["role-reviewer", "git", "dev-verify"]);
        assert!(
            caps.iter().all(|t| t.value.is_none()),
            "Reviewer caps are boolean-style; no parametric values"
        );

        // Additive invariant: pre-existing role bundles are NOT modified.
        let coder = role_required_caps(DispatchRole::Coder);
        let coder_slugs: Vec<&str> = coder.iter().map(|t| t.slug.as_str()).collect();
        assert_eq!(coder_slugs, vec!["role-coder", "cargo", "git"]);
        let researcher = role_required_caps(DispatchRole::Researcher);
        let researcher_slugs: Vec<&str> =
            researcher.iter().map(|t| t.slug.as_str()).collect();
        assert_eq!(researcher_slugs, vec!["role-researcher", "webSearch"]);
        assert!(role_required_caps(DispatchRole::Master).is_empty());
    }

    #[test]
    fn dispatch_status_serializes_snake_case() {
        // §8 state-machine wire vocabulary: status strings on the wire
        // must stay snake_case so the UI switch-case stays stable.
        let j = serde_json::to_string(&DispatchStatus::Planned).unwrap();
        assert_eq!(j, "\"planned\"");
        let j = serde_json::to_string(&DispatchStatus::Dispatched).unwrap();
        assert_eq!(j, "\"dispatched\"");
        let j = serde_json::to_string(&DispatchStatus::NoCandidate).unwrap();
        assert_eq!(j, "\"no_candidate\"");
        let j = serde_json::to_string(&DispatchStatus::Timeout).unwrap();
        assert_eq!(j, "\"timeout\"");
    }

    #[test]
    fn stage3_score_peer_emits_full_breakdown() {
        // Stage 3 promotion marker: `score_peer` now runs all four
        // dimensions end-to-end with real impls — `peer_active_load`
        // reads the OnceLock<RwLock> cache (defaults to idle = 1.0
        // when the entry is absent) and `failure_history` reads the
        // same cache for the rolling failure counter (defaults to 0).
        // The pre-Stage-3 panic boundary is gone; the dispatcher path
        // is now end-to-end real except for the live `/rpc/peers`
        // refresher (Stage 4 master orchestrator).
        let peer = PeerCapabilities {
            peer_id: "peer-x".to_string(),
            tags: vec![],
            last_reported_at: 0,
        };
        let task = DispatchTask {
            task_id: "t1".to_string(),
            required_caps: vec![],
            preferred_caps: vec![],
            payload: "null".to_string(),
            deadline_ms: None,
        };
        let s = score_peer(&peer, &task);
        // cap_match vacuous (no constraints) = 1.0; latency stale = 0.0;
        // load idle = 1.0; penalty zero. Aggregate = 0.5 + 0.0 + 0.15 + 0
        // = 0.65 — well above the 0.1 plan_dispatch floor.
        assert!(s.score > 0.5);
        assert!((s.breakdown.load_score - 1.0).abs() < 1e-6);
        assert!((s.breakdown.recent_failure_penalty - 0.0).abs() < 1e-6);
    }

    // ─── Stage 3 KAT (known-answer-test) vectors ────────────────────────

    #[test]
    fn tag_intersect_all_present_scores_one() {
        // §6.2 KAT: peer has both required (role-coder) + preferred (cargo).
        // Required term = 1.0 × 0.7 = 0.7; preferred term = 1.0 × 0.3 = 0.3;
        // sum = 1.0.
        let req = vec![CapabilityTag { slug: "role-coder".into(), value: None }];
        let pref = vec![CapabilityTag { slug: "cargo".into(), value: None }];
        let peer_tags = vec![
            CapabilityTag { slug: "role-coder".into(), value: None },
            CapabilityTag { slug: "cargo".into(), value: None },
        ];
        let s = tag_intersect(&req, &pref, &peer_tags);
        assert!((s - 1.0).abs() < 1e-6, "expected 1.0, got {}", s);
    }

    #[test]
    fn tag_intersect_required_missing_scores_partial() {
        // §6.2 KAT: peer missing the required tag but has the preferred one.
        // Required term = 0/1 × 0.7 = 0.0; preferred term = 1/1 × 0.3 = 0.3;
        // sum = 0.3. (plan_dispatch would still filter this peer out earlier
        // — score_peer itself does not re-check requireds.)
        let req = vec![CapabilityTag { slug: "role-coder".into(), value: None }];
        let pref = vec![CapabilityTag { slug: "cargo".into(), value: None }];
        let peer_tags = vec![CapabilityTag { slug: "cargo".into(), value: None }];
        let s = tag_intersect(&req, &pref, &peer_tags);
        assert!((s - 0.3).abs() < 1e-6, "expected 0.3, got {}", s);
    }

    #[test]
    fn tag_intersect_empty_constraints_score_one() {
        // Edge case: tagless task should not penalise any peer.
        let s = tag_intersect(&[], &[], &[]);
        assert!((s - 1.0).abs() < 1e-6, "expected 1.0, got {}", s);
    }

    #[test]
    fn tag_intersect_capability_value_is_part_of_identity() {
        // §7 invariant: `CapabilityTag` equality includes the optional value.
        // A peer advertising `ram=8gb` does NOT satisfy a `ram=16gb` requirement.
        let req = vec![CapabilityTag { slug: "ram".into(), value: Some("16gb".into()) }];
        let peer_tags = vec![CapabilityTag { slug: "ram".into(), value: Some("8gb".into()) }];
        let s = tag_intersect(&req, &[], &peer_tags);
        // Required hit = 0/1; preferred vacuous = 0.3. Total 0.3.
        assert!((s - 0.3).abs() < 1e-6, "expected 0.3, got {}", s);
    }

    #[test]
    fn latency_from_last_ping_fresh_returns_one() {
        // KAT: a ping issued "now" → age 0 → score 1.0 (within clock jitter).
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let s = latency_from_last_ping(now_ms);
        assert!(s > 0.99, "expected ~1.0 for fresh ping, got {}", s);
    }

    #[test]
    fn latency_from_last_ping_stale_returns_zero() {
        // KAT: a ping older than 500 ms clamps to 0.0 latency score.
        let s = latency_from_last_ping(0);
        assert!(s.abs() < 1e-6, "expected 0.0 for stale ping, got {}", s);
    }

    #[test]
    fn plan_dispatch_returns_no_matching_peer_when_required_absent() {
        // §6.2 + §11 KAT: no peer carries the required tag → NoMatchingPeer
        // BEFORE any Stage 4 helper is reached (filter happens first).
        let task = DispatchTask {
            task_id: "t1".into(),
            required_caps: vec![CapabilityTag { slug: "gpu".into(), value: None }],
            preferred_caps: vec![],
            payload: "null".into(),
            deadline_ms: None,
        };
        let peers = vec![PeerCapabilities {
            peer_id: "peer-cpu-only".into(),
            tags: vec![CapabilityTag { slug: "cargo".into(), value: None }],
            last_reported_at: 0,
        }];
        let r = plan_dispatch(&task, &peers);
        assert!(matches!(r, Err(DispatchError::NoMatchingPeer)));
    }

    #[test]
    fn plan_dispatch_empty_peer_set_returns_no_matching_peer() {
        // §11 KAT: empty cluster → NoMatchingPeer (filter step yields empty,
        // we exit before touching score_peer).
        let task = DispatchTask {
            task_id: "t1".into(),
            required_caps: vec![],
            preferred_caps: vec![],
            payload: "null".into(),
            deadline_ms: None,
        };
        let r = plan_dispatch(&task, &[]);
        assert!(matches!(r, Err(DispatchError::NoMatchingPeer)));
    }

    #[test]
    fn dispatch_error_serializes_with_code_tag() {
        // §11 wire-shape: error envelope uses `{"code": "..."}` tag so
        // the UI can dispatch on the machine-readable code string.
        let e = DispatchError::NoMatchingPeer;
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("no_matching_peer"), "wire shape: {}", j);

        let e2 = DispatchError::PayloadTooLarge;
        let j2 = serde_json::to_string(&e2).unwrap();
        assert!(j2.contains("payload_too_large"), "wire shape: {}", j2);
    }

    // --- SPEC-26 #2 decompose (G1): deterministic rule-based split ---

    fn roles(subs: &[Subtask]) -> Vec<DispatchRole> {
        subs.iter().map(|s| s.role).collect()
    }

    #[test]
    fn decompose_code_only_input_yields_single_coder() {
        let subs = decompose("Refactor mod foo and run cargo test");
        assert_eq!(roles(&subs), vec![DispatchRole::Coder]);
        // coder subtask must require the coder role tag.
        assert!(subs[0].required_caps.iter().any(|c| c.slug == "role-coder"));
    }

    #[test]
    fn decompose_research_only_input_yields_single_researcher() {
        let subs = decompose("Why is this the best practice, please explain");
        assert_eq!(roles(&subs), vec![DispatchRole::Researcher]);
        assert!(subs[0]
            .required_caps
            .iter()
            .any(|c| c.slug == "role-researcher"));
    }

    #[test]
    fn decompose_mixed_input_yields_coder_then_researcher() {
        // Both signals present -> two parallel subtasks, coder first (stable).
        let subs = decompose("Refactor the async fn and explain why it is faster");
        assert_eq!(
            roles(&subs),
            vec![DispatchRole::Coder, DispatchRole::Researcher]
        );
    }

    #[test]
    fn decompose_neither_input_falls_back_to_master() {
        let subs = decompose("hello there, please greet the team");
        assert_eq!(roles(&subs), vec![DispatchRole::Master]);
        assert!(
            subs[0].required_caps.is_empty(),
            "master fallback requires no special caps"
        );
    }

    #[test]
    fn decompose_substring_traps_do_not_misroute() {
        // codex review: naive contains() matched "struct" in "infrastructure",
        // "trait" in "portrait", "compare" in "comparable", "build" in
        // "rebuild". Whole-word matching must treat these as Master (no role kw).
        let subs = decompose("the infrastructure portrait is comparable to a rebuild");
        assert_eq!(
            roles(&subs),
            vec![DispatchRole::Master],
            "substring traps must NOT route to coder/researcher: {:?}",
            subs.iter().map(|s| s.role).collect::<Vec<_>>()
        );
    }

    #[test]
    fn assign_subtasks_routes_capable_and_flags_gaps() {
        let tag = |s: &str| CapabilityTag {
            slug: s.to_string(),
            value: None,
        };
        // Only a coder-capable peer is online (no researcher peer).
        let peers = vec![PeerCapabilities {
            peer_id: "node-a".to_string(),
            tags: vec![tag("role-coder"), tag("cargo"), tag("git")],
            last_reported_at: 0,
        }];

        // Mixed input -> [Coder, Researcher] subtasks.
        let subs = decompose("refactor the async fn and explain why");
        let assigns = assign_subtasks("job-1", &subs, &peers);
        assert_eq!(assigns.len(), 2);

        // Coder subtask routes to node-a with a stable, deterministic task id.
        let coder = assigns
            .iter()
            .find(|a| a.subtask.role == DispatchRole::Coder)
            .expect("a coder subtask exists");
        assert!(coder.error.is_none());
        assert_eq!(coder.task_id, "job-1-coder-0");
        let plan = coder.plan.as_ref().expect("coder gets a plan");
        assert_eq!(plan.selected_peer_id, "node-a");
        assert_eq!(plan.task_id, "job-1-coder-0");

        // Researcher subtask needs role-researcher + webSearch which no peer
        // advertises -> recorded as NoMatchingPeer (per-subtask gap, not a
        // whole-dispatch failure).
        let researcher = assigns
            .iter()
            .find(|a| a.subtask.role == DispatchRole::Researcher)
            .expect("a researcher subtask exists");
        assert!(researcher.plan.is_none());
        assert_eq!(researcher.error, Some(DispatchError::NoMatchingPeer));
        // task_id is preserved on the gap path too (review: codex).
        assert_eq!(researcher.task_id, "job-1-researcher-1");
    }

    // --- SPEC-26 #5 integrate ---

    fn outcome(
        task_id: &str,
        status: DispatchStatus,
        started: u64,
        completed: Option<u64>,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> DispatchOutcome {
        DispatchOutcome {
            task_id: task_id.to_string(),
            executed_by_peer_id: "node-a".to_string(),
            status,
            started_at_ms: started,
            completed_at_ms: completed,
            result_summary: summary.map(|s| s.to_string()),
            error: error.map(|s| s.to_string()),
            cost_usd: 0.0,
        }
    }

    #[test]
    fn integrate_counts_and_parallel_latency() {
        let outcomes = vec![
            outcome("j-coder-0", DispatchStatus::Completed, 1000, Some(1500), Some("refactored"), None),
            outcome("j-researcher-1", DispatchStatus::Failed, 1200, Some(1800), None, Some("peer error")),
        ];
        let r = integrate(&outcomes);
        assert_eq!(r.succeeded, 1);
        assert_eq!(r.failed, 1);
        // parallel span = max completed (1800) - min started (1000) = 800.
        assert_eq!(r.total_latency_ms, 800);
        assert!(r.markdown.contains("j-coder-0"), "md: {}", r.markdown);
        assert!(r.markdown.contains("j-researcher-1"));
        assert!(r.markdown.contains("1 ok, 1 failed"));
        // failure with no summary falls back to the error string.
        assert!(r.markdown.contains("peer error"));
    }

    #[test]
    fn integrate_sums_per_subtask_cost_into_total() {
        // SPEC-26 J5 worked example: $0.015 + $0.0088 + $0.0 == $0.0238.
        let mut a = outcome("j-0", DispatchStatus::Completed, 1000, Some(1500), Some("ok"), None);
        a.cost_usd = 0.015;
        let mut b = outcome("j-1", DispatchStatus::Completed, 1000, Some(1600), Some("ok"), None);
        b.cost_usd = 0.0088;
        let c = outcome("j-2", DispatchStatus::Completed, 1000, Some(1400), Some("ok"), None); // 0.0
        let r = integrate(&[a, b, c]);
        // EXACT sum (G6: master adds 0.0) — assert equality, not `> 0.0`, so a
        // stubbed `total_cost_usd: 0.0` cannot fake-green this.
        let expected: f64 = 0.015 + 0.0088 + 0.0;
        assert!((r.total_cost_usd - expected).abs() < 1e-9, "got {}", r.total_cost_usd);
        assert!((r.total_cost_usd - 0.0238).abs() < 1e-9, "got {}", r.total_cost_usd);
    }

    #[test]
    fn integrate_empty_has_zero_cost() {
        assert_eq!(integrate(&[]).total_cost_usd, 0.0);
    }

    #[test]
    fn integrate_empty_is_zeroed() {
        let r = integrate(&[]);
        assert_eq!(r.succeeded, 0);
        assert_eq!(r.failed, 0);
        assert_eq!(r.total_latency_ms, 0);
        assert!(r.markdown.contains("0 subtask(s)"));
    }

    #[test]
    fn integrate_no_completion_yields_zero_latency() {
        // Still running -> not terminal -> neither ok nor failed, latency 0.
        let outcomes = vec![outcome(
            "j-coder-0",
            DispatchStatus::Running,
            1000,
            None,
            None,
            None,
        )];
        let r = integrate(&outcomes);
        assert_eq!(r.succeeded, 0);
        assert_eq!(r.failed, 0);
        assert_eq!(r.total_latency_ms, 0);
    }

    #[test]
    fn integrate_all_terminal_failure_statuses_count_failed() {
        // Failed, Timeout, and NoCandidate are all "failed" (review: codex).
        let outcomes = vec![
            outcome("a", DispatchStatus::Failed, 0, Some(10), None, Some("boom")),
            outcome("b", DispatchStatus::Timeout, 0, Some(20), None, Some("slow")),
            outcome("c", DispatchStatus::NoCandidate, 0, None, None, Some("no peer")),
        ];
        let r = integrate(&outcomes);
        assert_eq!(r.succeeded, 0);
        assert_eq!(r.failed, 3, "Failed + Timeout + NoCandidate all count as failed");
    }

    #[test]
    fn integrate_malformed_timestamp_is_zero_latency() {
        // completed < started must not underflow -> guarded to 0 (review: codex).
        let outcomes = vec![outcome(
            "x",
            DispatchStatus::Completed,
            2000,
            Some(1000),
            Some("ok"),
            None,
        )];
        let r = integrate(&outcomes);
        assert_eq!(r.total_latency_ms, 0, "completed<started must yield 0, not underflow");
        assert_eq!(r.succeeded, 1);
    }

    #[test]
    fn integrate_markdown_preserves_input_order() {
        // Deterministic, stable line order follows the outcomes slice.
        let outcomes = vec![
            outcome("first", DispatchStatus::Completed, 0, Some(1), Some("A"), None),
            outcome("second", DispatchStatus::Completed, 0, Some(1), Some("B"), None),
        ];
        let md = integrate(&outcomes).markdown;
        let i_first = md.find("first").expect("first present");
        let i_second = md.find("second").expect("second present");
        assert!(i_first < i_second, "markdown must preserve input order");
    }

    #[test]
    fn decompose_is_deterministic() {
        let input = "refactor and explain why";
        assert_eq!(decompose(input), decompose(input), "must be pure/stable");
    }

    #[test]
    fn subtask_and_role_serde_round_trip() {
        let s = Subtask {
            role: DispatchRole::Coder,
            prompt: "x".to_string(),
            required_caps: vec![CapabilityTag {
                slug: "role-coder".to_string(),
                value: None,
            }],
        };
        let j = serde_json::to_string(&s).unwrap();
        // camelCase field on the wire.
        assert!(j.contains("requiredCaps"), "camelCase wire: {}", j);
        // role enum serializes snake_case.
        assert!(j.contains("\"coder\""), "role wire: {}", j);
        let back: Subtask = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }

    // ── SPEC-26 master orchestrator (run_dispatch_with) ──────────────────────
    use async_trait::async_trait;
    use std::time::Duration;

    fn now_ms_test() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }
    fn cap_tag(s: &str) -> CapabilityTag {
        CapabilityTag {
            slug: s.to_string(),
            value: None,
        }
    }

    /// Mock peer-runner: sleeps `sleep_ms` (to exercise parallelism) then returns
    /// a terminal outcome with the configured status.
    struct MockRunner {
        sleep_ms: u64,
        status: DispatchStatus,
    }
    #[async_trait]
    impl SubtaskRunner for MockRunner {
        async fn run(&self, plan: &DispatchPlan) -> DispatchOutcome {
            let started = now_ms_test();
            tokio::time::sleep(Duration::from_millis(self.sleep_ms)).await;
            DispatchOutcome {
                task_id: plan.task_id.clone(),
                executed_by_peer_id: plan.selected_peer_id.clone(),
                status: self.status,
                started_at_ms: started,
                completed_at_ms: Some(now_ms_test()),
                result_summary: Some("mock".into()),
                error: None,
                cost_usd: 0.0,
            }
        }
    }

    #[tokio::test]
    async fn orchestrator_runs_subtasks_in_parallel() {
        // coder + researcher peers online → mixed input → 2 subtasks. Each mock
        // sleeps 200ms; parallel wall-clock must be well under the ~400ms a
        // sequential run would take (SPEC-26 G4 parallel fan-out).
        let peers = vec![
            PeerCapabilities {
                peer_id: "coder-peer".into(),
                tags: vec![cap_tag("role-coder"), cap_tag("cargo"), cap_tag("git")],
                last_reported_at: 0,
            },
            PeerCapabilities {
                peer_id: "research-peer".into(),
                tags: vec![cap_tag("role-researcher"), cap_tag("webSearch")],
                last_reported_at: 0,
            },
        ];
        let runner = MockRunner {
            sleep_ms: 200,
            status: DispatchStatus::Completed,
        };
        let t0 = now_ms_test();
        let r =
            run_dispatch_with("refactor the async fn and explain why", "job-par", &peers, &runner)
                .await;
        let wall = now_ms_test() - t0;
        assert_eq!(r.succeeded, 2, "both subtasks completed");
        assert_eq!(r.failed, 0);
        assert!(
            wall < 380,
            "parallel dispatch must be ~200ms, not ~400ms sequential; was {}ms",
            wall
        );
    }

    #[tokio::test]
    async fn orchestrator_gap_subtask_becomes_nocandidate() {
        // Only a coder peer online; mixed input also wants a researcher → the
        // researcher subtask has no capable peer → NoCandidate (counted failed),
        // while the coder subtask still completes (degraded path J6).
        let peers = vec![PeerCapabilities {
            peer_id: "coder-only".into(),
            tags: vec![cap_tag("role-coder"), cap_tag("cargo"), cap_tag("git")],
            last_reported_at: 0,
        }];
        let runner = MockRunner {
            sleep_ms: 5,
            status: DispatchStatus::Completed,
        };
        let r = run_dispatch_with(
            "refactor the async fn and explain why",
            "job-gap",
            &peers,
            &runner,
        )
        .await;
        assert_eq!(r.succeeded, 1, "coder subtask ran");
        assert_eq!(r.failed, 1, "researcher had no peer → NoCandidate counted as failed");
    }

    #[tokio::test]
    async fn orchestrator_single_master_subtask() {
        // Neither keyword → exactly one Master subtask → exactly one integrated
        // outcome (whether it resolves to a peer or NoCandidate).
        let peers = vec![PeerCapabilities {
            peer_id: "any".into(),
            tags: vec![],
            last_reported_at: 0,
        }];
        let runner = MockRunner {
            sleep_ms: 5,
            status: DispatchStatus::Completed,
        };
        let r = run_dispatch_with("hello there, greet the team", "job-master", &peers, &runner).await;
        assert_eq!(r.succeeded + r.failed, 1, "exactly one (master) subtask integrated");
    }

    // ── SPEC-26 Stage 2: RpcRunner error mapping + production run_dispatch ────
    #[test]
    fn dispatch_error_maps_to_terminal_outcome() {
        let o = dispatch_error_to_outcome("t1", "p1", 100, &DispatchError::NoMatchingPeer);
        assert_eq!(o.status, DispatchStatus::NoCandidate);
        assert_eq!(o.task_id, "t1");
        assert!(o.completed_at_ms.is_some(), "error outcome must be terminal");
        assert!(o.error.is_some());
        assert_eq!(
            dispatch_error_to_outcome("t", "p", 0, &DispatchError::RouteTimeout).status,
            DispatchStatus::Timeout
        );
        for e in [
            DispatchError::AllPeersBusy,
            DispatchError::DispatchAuthFailed,
            DispatchError::PayloadTooLarge,
        ] {
            assert_eq!(
                dispatch_error_to_outcome("t", "p", 0, &e).status,
                DispatchStatus::Failed,
                "{:?} should map to Failed",
                e
            );
        }
    }

    #[tokio::test]
    async fn run_dispatch_with_no_peers_yields_failed_no_network() {
        // Production entry (real RpcRunner) but with ZERO peers: the coder
        // subtask has no candidate → assign_subtasks returns plan:None → the
        // orchestrator synthesizes NoCandidate BEFORE the runner is ever called,
        // so RpcRunner makes no network call. Verifies the run_dispatch wiring
        // end-to-end without a live cluster.
        let r = run_dispatch("refactor the async fn", "job-real", &[]).await;
        assert_eq!(r.succeeded, 0);
        assert_eq!(r.failed, 1, "no peer → NoCandidate counted failed");
        assert!(r.markdown.contains("0 ok, 1 failed"));
    }
}
