// SPEC-28 §7 + §8 — Onboarding wire types (single source of truth for the
// 30s-hello state machine the wizard, CLI, and TTFR / TTFT metrics share).
//
// Stage 4 (partial — FSM table promoted to phf::Map):
// `fsm_table_pseudo` now returns the next state via a compile-time
// `phf::Map<&'static str, OnboardingState>` keyed on `"state:transition"`
// slugs (enums don't implement `PhfHash`; the string-slug encoding is the
// shortest path to a real const table). `advance`, `rollback`, `compute_ttfr`,
// `start_demo_relay_handoff`, `precondition_check_pseudo`, `otel_emit_pseudo`,
// and `https_get_pseudo` remain `unimplemented!()` — they require crates /
// runtime hooks not in scope for this wave (opentelemetry pipeline, reqwest,
// SPEC-52 quota wire).
//
// 中文: 本檔對應 SPEC-28 §7（資料模型）與 §8（狀態機）。負責 onboarding（新手
// 上線流程）FSM 的詳細 transition（狀態轉換）邏輯與 helper 函式；對外曝光 6 個
// state 對應的 snapshot（快照）/ context（脈絡）/ 兩個 timing 指標（TTFR
// 「首次回應耗時」、TTFT「首個 token 耗時」）/ demo-relay（示範用中介伺服器）
// 配額 handoff。`OnboardingState` 與 `OnboardingTransition` 兩個 enum 已由
// `tauri_wire.rs` 為了 Tauri 命令目錄宣告過 — 本檔 re-export 避免 double
// definition，避免兩處不同步 drift（漂移）。
//
// TODO Stage 2:
//   - implement `advance()` per §7.1 transition table (8 legal edges +
//     `InvalidTransition` everywhere else)
//   - implement `rollback()` per §8 mermaid diagram (only 4 reversible edges:
//     PickedLanguage→FreshInstall / CreatedIdentity→PickedLanguage /
//     JoinedCluster→CreatedIdentity / SetProvider→JoinedCluster)
//   - implement `compute_ttfr()` enforcing the §1 / §12 p95 < 30s budget
//   - implement `should_fallback_to_demo_relay()` — true iff no cluster
//     joined AND no BYOM (Bring Your Own Model) key configured
//   - implement `start_demo_relay_handoff()` calling SPEC-52 GET
//     `demo.phantommesh.io/quota` per §9.7 wire contract
//   - persist `OnboardingStateSnapshot` to `~/.phantom-mesh/onboarding.json`
//     per §7.4 / §7.5 (out of scope for this wire file — lives in
//     `core/src/wizard.rs` Stage 2)

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ─── Re-export §7.1 enums from tauri_wire ────────────────────────────────────
//
// `OnboardingState` (6 variants per SPEC-28 §7.1) and `OnboardingTransition`
// (Forward / Rollback / NoOp per §9.2 / §9.3) are already declared in
// `tauri_wire.rs` so that the Tauri command catalog can reference them at the
// FFI boundary. We re-export them here so this module remains the spec-level
// owner of the FSM transition logic while staying single-definition.
//
// 中文: 重新匯出 `tauri_wire.rs` 已宣告的兩個 enum，避免 double-definition
// 造成兩處 drift（漂移）。Tauri 命令目錄那邊只是「目錄參照用」，本檔才是
// FSM transition 邏輯的 spec-level owner（規格層擁有者）。
pub use crate::tauri_wire::{OnboardingState, OnboardingTransition};

// ─── §7.1 / §8 OnboardingStateSnapshot — runtime FSM snapshot ────────────────

/// Snapshot of the onboarding FSM at a single moment in time. Persisted to
/// `~/.phantom-mesh/onboarding.json` per §7.4 so that killing and re-opening
/// the app resumes from the same step (per §8 "Resume guarantee").
///
/// `entered_at_ms` is the wall-clock millisecond timestamp the FSM moved
/// into `current_state`; together with the matching `OnboardingProgressEvent`
/// stream this lets the UI compute "time spent in step" without a separate
/// table.
///
/// `retry_count` increments each time the same `advance()` call from the
/// same `current_state` returned an `OnboardingError` (e.g. demo-relay
/// transient unreachable); Stage 2 will rate-limit retries per §9.2
/// "30 advance/min/device".
///
/// 中文: onboarding FSM 在某一刻的快照（snapshot），會被持久化（persist）到
/// `~/.phantom-mesh/onboarding.json`。`entered_at_ms` 是進入該 state 的牆鐘
/// 毫秒，配合 `OnboardingProgressEvent` 序列可算 step 耗時；`retry_count`
/// 記同一 state 上 `advance()` 失敗的次數，Stage 2 會接 §9.2 rate limit。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/onboarding/")]
#[serde(rename_all = "camelCase")]
pub struct OnboardingStateSnapshot {
    /// Current FSM state — one of the 6 variants per SPEC-28 §7.1.
    pub current_state: OnboardingState,
    /// Wall-clock millisecond timestamp the FSM moved into `current_state`.
    pub entered_at_ms: u64,
    /// Number of times `advance()` on this state returned an error and the
    /// caller retried. Reset to 0 every time the state advances forward.
    pub retry_count: u8,
    /// Most recent `OnboardingError` rendered as its `code` slug — None when
    /// the last `advance()` succeeded or no `advance()` has been attempted
    /// from this state yet.
    pub last_error: Option<String>,
}

// ─── §7.1 OnboardingContext — sanitised side-effect summary ──────────────────

/// Side-effect summary the FSM accumulates as the user moves through steps.
/// Every field is **derived / sanitised** so it can be persisted to
/// `~/.phantom-mesh/onboarding.json` plaintext per §7.5 without leaking any
/// secret material. Specifically:
///
/// - `cluster_id_hash` is the SHA-256 hash slug, never the raw cluster name
/// - `identity_fingerprint` is the 12-hex short fingerprint per SPEC-12 §7.1,
///   never the public key bytes
/// - `provider_slug` is a stable provider id (e.g. `"groq"` / `"openai"`),
///   never the API key prefix nor the raw key
/// - `demo_relay_used` is a single bool — no demo-relay URL, no quota
///   counter (the live counter lives in `DemoRelayHandoff` separately so
///   reloading the snapshot does not freeze the quota at an old value)
///
/// 中文: FSM 累積的副作用（side effect）摘要，**全部 derived（衍生）/
/// sanitised（消毒）**，可安全寫進明文 `onboarding.json`：cluster 名只存
/// SHA-256 hash 不存原文、identity 只存 12 字元短指紋不存公鑰、provider 只
/// 存穩定 slug 不存 API key、demo-relay 只存 bool 不存 URL 也不存 quota
/// 計數（避免重載 snapshot 時 quota 凍結成舊值）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/onboarding/")]
#[serde(rename_all = "camelCase")]
pub struct OnboardingContext {
    /// SHA-256 hex slug of the joined cluster id, or None if not yet joined.
    pub cluster_id_hash: Option<String>,
    /// 12-hex short fingerprint of the created identity, or None if not yet
    /// created. Mirrors `IdentityPublic.fingerprint` per SPEC-12 §7.1.
    pub identity_fingerprint: Option<String>,
    /// Stable provider slug the user picked, or None if not yet set.
    pub provider_slug: Option<String>,
    /// `true` iff the user took the demo-relay (SPEC-52) path instead of
    /// configuring a BYOM (Bring Your Own Model) key. Sticky for the whole
    /// onboarding even after the user later sets a real BYOM key — so the
    /// TTFR (Time To First Response) metric can be attributed correctly.
    pub demo_relay_used: bool,
    /// D1 (login-first): the OAuth provider slug the account was signed in with
    /// (e.g. `"google"` / `"apple"`), or None if the user has not logged in.
    /// Sourced from the broker login (`broker_login_finish`) and carried here
    /// so the FSM can enforce "login before identity" per the GUI D1 decision.
    /// Sanitised: a stable provider id, never a token.
    pub identity_provider: Option<String>,
    /// D1 (login-first): the OAuth account subject / fingerprint (the broker's
    /// stable account id, e.g. the email or `sub` claim), or None if not logged
    /// in. Sanitised display value only — never a token or raw credential.
    pub identity_sub: Option<String>,
}

// ─── §1 / §12 TTFRMetric — Time To First Response ───────────────────────────

/// TTFR (Time To First Response) — the SPEC-28 north-star metric. Measured
/// from the moment `install.sh` finishes (or App Store install completes)
/// until the first agent reply byte is rendered on screen. Budget per §1:
/// **p50 < 15s, p95 < 30s** — anything > 30s fails the 30s-hello SLO.
///
/// `total_ms` is redundant with `first_reply_at_ms - install_complete_at_ms`
/// but is stored explicitly so analytics pipelines do not have to re-derive
/// it (and so the wire shape stays stable if we ever add e.g. timezone
/// adjustments).
///
/// 中文: TTFR（首次回應耗時）— SPEC-28 北極星指標。從 `install.sh` 跑完
/// （或 App Store 裝完）到第一個 agent reply byte 上畫面為止。預算 p50 <
/// 15 秒、p95 < 30 秒；超過 30 秒就 fail 30s-hello SLO（服務水準目標）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/onboarding/")]
#[serde(rename_all = "camelCase")]
pub struct TTFRMetric {
    /// Wall-clock millisecond timestamp `install.sh` (or App Store install)
    /// finished. The 30s budget starts ticking here.
    pub install_complete_at_ms: u64,
    /// Wall-clock millisecond timestamp the first agent reply byte rendered.
    pub first_reply_at_ms: u64,
    /// `first_reply_at_ms - install_complete_at_ms`, pre-computed for
    /// analytics convenience. MUST equal the difference exactly.
    pub total_ms: u64,
}

// ─── §12 TTFTMetric — Time To First Token ───────────────────────────────────

/// TTFT (Time To First Token) — secondary latency metric for **existing**
/// conversations (i.e. after onboarding is complete and the user sends a new
/// message). Budget per §12: **p50 < 2s, p95 < 5s**. Distinct from TTFR
/// because TTFT does not include any one-time install / identity / cluster
/// cost; it is the steady-state UX latency.
///
/// 中文: TTFT（首個 token 耗時）— **既有對話**的次要延遲指標（onboarding
/// 已完成、user 送下一則新訊息時量）。預算 p50 < 2 秒、p95 < 5 秒。和
/// TTFR 不同：TTFT 不含任何 install / identity / cluster 一次性成本，是
/// 穩態使用體驗延遲。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/onboarding/")]
#[serde(rename_all = "camelCase")]
pub struct TTFTMetric {
    /// Wall-clock millisecond timestamp the user pressed Send.
    pub send_at_ms: u64,
    /// Wall-clock millisecond timestamp the first token from the LLM
    /// (Large Language Model) appeared in the chat UI.
    pub first_token_at_ms: u64,
    /// `first_token_at_ms - send_at_ms`, pre-computed for analytics.
    pub total_ms: u64,
}

// ─── §7.1 / §9.7 DemoRelayHandoff — SPEC-52 fallback contract ───────────────

/// Snapshot of the demo-relay (SPEC-52) handoff at the moment the user
/// picked the `DemoRelay` provider variant. The wizard reads this from the
/// SPEC-52 `GET demo.phantommesh.io/quota` endpoint per §9.7 to decide
/// whether to proceed (`quota_remaining > 0`) or surface the
/// `DemoRelayExhausted` error.
///
/// `ttl_seconds` is the lease window granted by the relay; the wizard MUST
/// re-handshake before it expires or surface the demo-quota-exhausted UI.
///
/// 中文: demo-relay（SPEC-52 示範用中介伺服器）handoff（接管）快照。Wizard
/// 從 §9.7 `GET demo.phantommesh.io/quota` 拿 `quota_remaining` 決定能不能
/// 走 demo 路線；`ttl_seconds` 是 relay 給的租期，過期前要 re-handshake
/// 不然要顯示「配額用完」UI。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/onboarding/")]
#[serde(rename_all = "camelCase")]
pub struct DemoRelayHandoff {
    /// HTTPS URL of the demo-relay endpoint the wizard will route through
    /// (e.g. `"https://demo.phantommesh.io"`). Always TLS — never bare HTTP.
    pub relay_url: String,
    /// Remaining quota for this device today (max 3 per §11.1
    /// `DEMO_RELAY_QUOTA_EXHAUSTED`). Reaches 0 → wizard MUST show the
    /// `DemoRelayExhausted` error and prompt for a BYOM key instead.
    pub quota_remaining: u8,
    /// Seconds until the demo-relay lease expires. Wizard re-handshakes
    /// before this hits 0 or surfaces the quota-exhausted error.
    pub ttl_seconds: u32,
}

// ─── §8 OnboardingProgressEvent — emitted on every FSM transition ───────────

/// Event emitted on the SPEC-23 event bus every time the FSM transitions
/// (forward or rollback). Consumed by:
///
/// - the wizard UI to update progress dots
/// - the SPEC-50 telemetry sink (opt-in only per §13) to compute step-time
///   distributions
/// - the SPEC-23 audit log so a user can later see when they finished each
///   onboarding step
///
/// `snapshot_at_ms` is the wall-clock millisecond at emit time and MUST
/// equal `snapshot.entered_at_ms` for forward transitions (rollback events
/// re-use the older timestamp so analytics can group rollback durations
/// separately).
///
/// 中文: 每次 FSM transition（前進或 rollback）都會在 SPEC-23 事件匯流排上
/// 發一個 event：wizard UI 更新進度點、SPEC-50 遙測（opt-in）算 step 耗時
/// 分布、SPEC-23 審計日誌記時間戳。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/onboarding/")]
#[serde(rename_all = "camelCase")]
pub struct OnboardingProgressEvent {
    /// FSM state after the transition completed.
    pub state: OnboardingState,
    /// Sanitised side-effect snapshot at emit time. MUST be a deep clone so
    /// later mutation does not retroactively change emitted events.
    pub context: OnboardingContext,
    /// Wall-clock millisecond timestamp at emit time.
    pub snapshot_at_ms: u64,
}

// ─── §11.1 OnboardingError — wire-facing error catalog ──────────────────────

/// Wire-facing error variants for the onboarding subsystem. Mirrors the
/// SPEC-28 §11.1 error catalog one-to-one (subset used by the wire layer;
/// the wizard layer also handles `InvalidTransition` / `AtInitialState` /
/// `OnboardingStateCorrupt` which are FSM-internal — see
/// `core/src/wizard.rs` Stage 2 for the full mapping).
///
/// 中文: SPEC-28 §11.1 error catalog 的 wire-facing 鏡像。Wire 層只用一個
/// 子集；wizard 層另外處理 `InvalidTransition` 等 FSM 內部錯誤，見
/// `core/src/wizard.rs` Stage 2。
#[derive(Debug, Clone, Serialize, Deserialize, TS, thiserror::Error)]
#[ts(export, export_to = "../../app/src/lib/generated/onboarding/")]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum OnboardingError {
    /// SPEC-12 keystore write failed during `CreatedIdentity` transition.
    /// User-recoverable: unlock device / free disk and retry.
    #[error("onboarding.identity_creation_failed: {detail}")]
    IdentityCreationFailed { detail: String },
    /// SPEC-11 mDNS join or SPEC-10 `/rpc/cluster/pair` failed during
    /// `JoinedCluster` transition. User-recoverable: check Wi-Fi / retry.
    #[error("onboarding.cluster_join_failed: {detail}")]
    ClusterJoinFailed { detail: String },
    /// Reached `SetProvider` step but no LLM provider has been configured
    /// (neither BYOM key nor demo-relay). Wizard MUST surface picker UI.
    #[error("onboarding.no_provider_configured")]
    NoProviderConfigured,
    /// Demo-relay (SPEC-52) returned `quota_remaining = 0`. Not retryable
    /// today — user must wait 24h or configure a BYOM key.
    #[error("onboarding.demo_relay_exhausted")]
    DemoRelayExhausted,
    /// Computed TTFR exceeded the §1 / §12 p95 budget of 30,000 ms. Stage 2
    /// will surface this to the telemetry sink so we can track regressions.
    #[error("onboarding.ttfr_budget_exceeded: total_ms={total_ms}")]
    TtfrBudgetExceeded { total_ms: u64 },
    /// Computed TTFT exceeded the §12 p95 budget of 5,000 ms.
    #[error("onboarding.ttft_budget_exceeded: total_ms={total_ms}")]
    TtftBudgetExceeded { total_ms: u64 },
}

// ─── Stage 2 stub helpers (Stage 1 leaves `unimplemented!()`) ───────────────

/// Apply one forward FSM transition per the §7.1 / §8 transition table.
/// Returns the **new** state on success.
///
/// Invariants Stage 2 MUST enforce:
///
/// - All 6 forward edges per §7.1 must be wired; everything else returns the
///   input state unchanged plus an `InvalidTransition` audit log (FSM-level
///   error type, not in the wire-facing `OnboardingError` catalog).
/// - `FreshInstall → PickedLanguage` requires `ctx.identity_fingerprint` is
///   None (else identity was already created — wizard bug, refuse).
/// - `PickedLanguage → CreatedIdentity` calls SPEC-12 `build_init_outcome`;
///   any failure maps to `OnboardingError::IdentityCreationFailed`.
/// - `CreatedIdentity → JoinedCluster` calls SPEC-11 join; any failure maps
///   to `OnboardingError::ClusterJoinFailed`.
/// - `SetProvider → FirstReplyReceived` requires `ctx.provider_slug.is_some()`
///   else returns `OnboardingError::NoProviderConfigured`.
///
/// 中文: 套用 §7.1 / §8 transition table 中的一條 forward 邊。回傳 **new**
/// state。所有非法 input 回 `InvalidTransition`（FSM 內部錯，不在 wire 層
/// 錯誤目錄）；合法 input 但 side effect fail（如 SPEC-12 keystore 寫失敗、
/// SPEC-11 join 失敗）回對應 wire `OnboardingError`。
pub fn advance(
    snapshot: &OnboardingStateSnapshot,
    ctx: &OnboardingContext,
) -> Result<OnboardingState, OnboardingError> {
    // Step 1 — Lookup (current, Forward) in the FSM transition table per
    // SPEC-28 §8. Table is a static phf::Map keyed on the
    // `(OnboardingState, OnboardingTransition)` pair so the dispatch stays
    // O(1) and is exhaustively code-reviewable in one place. A `None` here
    // means the terminal `FirstReplyReceived` state — onboarding is already
    // complete, so there is no forward edge. There is no dedicated wire
    // variant for "already complete" in the §11.1 catalog (it is FSM-internal
    // per the module docstring), so we surface the nearest stable terminal
    // signal: the journey is done, no provider step remains.
    let next_state = match fsm_table_pseudo(snapshot.current_state, OnboardingTransition::Forward) {
        Some(s) => s,
        None => return Ok(snapshot.current_state),
    };

    // Step 2 — Validate that the per-state preconditions encoded in §7.1 are
    // actually satisfied by the runtime `ctx`. E.g. moving INTO
    // `CreatedIdentity` requires `ctx.identity_fingerprint.is_some()`; moving
    // INTO `JoinedCluster` requires `ctx.cluster_id_hash.is_some()`; moving
    // INTO `SetProvider` requires `ctx.provider_slug.is_some()` OR
    // `ctx.demo_relay_used == true`. On failure the precondition check returns
    // the closest §11.1 wire variant already mapped, so we propagate it.
    precondition_check_pseudo(snapshot.current_state, ctx)?;

    // Step 3 — Preconditions held: return the new state.
    Ok(next_state)
}

// ─── D1–D5 effectful shell — the side-effects the GUI `advance` performs ─────
//
// `advance()` above stays pure (FSM lookup + precondition validation) so the
// unit suite can exercise the table without touching the filesystem, network,
// or spawning processes. The GUI flow needs the *real* side-effects from the
// CLI (a7c5701f) on each forward edge, so the Tauri command calls
// `advance_with_effects()` instead. It runs the transition's side-effect, folds
// the derived results back into `ctx`, then defers to the pure `advance()` for
// the FSM move. This is the standard pure-core / effectful-shell split and
// reuses the exact functions the shipped `phantom` CLI onboarding uses.

/// Result of running the side-effect for one forward edge: the (possibly)
/// mutated context plus the resulting next state. The caller persists both.
#[derive(Debug, Clone)]
pub struct AdvanceOutcome {
    /// Context after the side-effect folded its derived results in.
    pub context: OnboardingContext,
    /// FSM state after the (validated) forward transition.
    pub next_state: OnboardingState,
}

/// GUI-facing forward step: run the real side-effect for the current edge,
/// then advance the FSM. `login` carries the OAuth identity the GUI obtained
/// from the broker login BEFORE calling this (D1 login-first happens in the UI
/// via `broker_login_*`; we only fold its result in + mint the local key here).
///
/// Side-effects per edge (matching the shipped CLI `run_first_time_onboarding`):
/// - `FreshInstall → CreatedIdentity` (D1): fold in `login` (provider/sub),
///   then `identity::init` → ed25519 keystore mint → `identity_fingerprint`.
/// - `CreatedIdentity → JoinedCluster` (D4 staged): spawn detached
///   `phantom serve` (binds 127.0.0.1:7878 + mDNS-advertises this node) and
///   record a single-node `cluster_id_hash`. Peer-join / vault sync = Stage 2.
/// - `JoinedCluster → SetProvider` (D5): detect subscription CLIs + local
///   Ollama, rank them, set `provider_slug` to the best available.
/// - `SetProvider → FirstReplyReceived`: no side-effect — the first real LLM
///   call happens later in the chat UI; we only validate a provider exists.
pub async fn advance_with_effects(
    snapshot: &OnboardingStateSnapshot,
    ctx: &OnboardingContext,
    login: Option<OnboardingLogin>,
) -> Result<AdvanceOutcome, OnboardingError> {
    let mut next_ctx = ctx.clone();

    match snapshot.current_state {
        OnboardingState::FreshInstall => {
            // Talk-first: an account login is OPTIONAL here — if the GUI happened
            // to log in first, fold it in; otherwise we proceed straight to a
            // usable provider so the user can chat (login becomes a later
            // soft-prompt, not a gate).
            if let Some(l) = login {
                next_ctx.identity_provider = Some(l.provider);
                next_ctx.identity_sub = Some(l.sub);
            }
            // The ONE thing needed to reach a first reply: a provider.
            perform_provider_detection(&mut next_ctx).await?;
            // Device identity + the mesh node come up in the BACKGROUND —
            // best-effort, never fail the flow, never block the first reply.
            perform_identity_and_serve_background(&mut next_ctx);
        }
        // Legacy snapshots mid old-flow converge into set_provider: make sure a
        // provider is detected (idempotent) so the precondition holds.
        OnboardingState::CreatedIdentity | OnboardingState::JoinedCluster => {
            perform_provider_detection(&mut next_ctx).await?;
        }
        // SetProvider → FirstReplyReceived and the terminal state have no
        // side-effect.
        _ => {}
    }

    // Defer to the pure FSM for validation + the actual state move so there is
    // exactly one source of truth for the legal-edge table + preconditions.
    let next_state = advance(snapshot, &next_ctx)?;
    Ok(AdvanceOutcome {
        context: next_ctx,
        next_state,
    })
}

/// OAuth login result the GUI passes into the `FreshInstall → CreatedIdentity`
/// edge. Both fields are sanitised display values (a stable provider slug and
/// the broker account id / email) — never a token. Mirrors the broker login
/// response (`broker_login_finish`).
#[derive(Debug, Clone)]
pub struct OnboardingLogin {
    /// OAuth provider slug, e.g. `"google"` / `"apple"`.
    pub provider: String,
    /// Stable account subject (broker account id or email).
    pub sub: String,
}

// (The old login-first `perform_login_and_identity` was removed in the
// talk-first reorg: login is now optional + folded inline in
// `advance_with_effects`, and the no-login identity mint lives in
// `perform_identity_and_serve_background`.)

/// D4 (Stage 1) side-effect — spawn a detached `phantom serve`, which binds
/// 127.0.0.1:7878 and mDNS-advertises this machine as a mesh node, then record
/// a single-node `cluster_id_hash`. This matches the CLI wizard's Step 2: a
/// node is discoverable on the LAN. Full peer-join (pairing with an existing
/// cluster) + vault E2EE sync are Stage 2.
///
/// TODO(stage 2): interactive peer-join (SPEC-11 mDNS pair + SPEC-10
/// `/rpc/cluster/pair`) and pull shared provider keys from the broker vault.
fn perform_serve_advertise(ctx: &mut OnboardingContext) -> Result<(), OnboardingError> {
    // Platform split: on DESKTOP the node hosts its own `phantom serve` (the
    // mesh "home"), spawned detached. On MOBILE (iOS/Android) the OS sandbox
    // forbids spawning child processes (EPERM / "Operation not permitted"), AND
    // by design a phone is NOT a serve home — it is a client that reaches the
    // user's desktop/cloud node (anchor: "phone + cloud"). So on mobile we skip
    // the spawn entirely and just record single-node membership; the in-app
    // runtime / remote node handles serving. (Found via real-device TestFlight
    // dogfood 2026-06-07: cluster_join_failed os error 1 on iOS.)
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let self_exe =
            std::env::current_exe().map_err(|e| OnboardingError::ClusterJoinFailed {
                detail: format!("could not locate phantom binary: {e}"),
            })?;
        // Detached spawn — serve owns the daemon lifecycle + the mDNS advertise.
        // Redirect its output to ~/.phantom-mesh/serve.log (and detach stdin) so
        // the daemon's startup/tracing lines don't bleed into the talk-first
        // wizard's terminal.
        let (out, errout) = crate::cli_config::serve_log_stdio();
        std::process::Command::new(&self_exe)
            .arg("serve")
            .stdin(std::process::Stdio::null())
            .stdout(out)
            .stderr(errout)
            .spawn()
            .map_err(|e| OnboardingError::ClusterJoinFailed {
                detail: format!("could not start `phantom serve`: {e}"),
            })?;
    }

    // Stage 1: single-node membership. Derive a stable self-hash from the
    // identity fingerprint when present so re-runs are idempotent; otherwise a
    // fixed "single-node" sentinel. Stage 2 replaces this with the real joined
    // cluster id hash once peer-join lands.
    let self_hash = match &ctx.identity_fingerprint {
        Some(fp) => format!("self-{fp}"),
        None => "self-single-node".to_string(),
    };
    ctx.cluster_id_hash = Some(self_hash);
    Ok(())
}

/// TALK-FIRST background side-effect — mint the device identity + bring up the
/// mesh node WITHOUT requiring login and WITHOUT being able to fail the flow.
/// Identity (a fast local keystore op) and `phantom serve` (already detached)
/// must never block or break the path to a first reply; any error is swallowed
/// (the user can retry via `phantom auth keys init` / `phantom serve`). Account
/// login is deliberately NOT done here — it is a later soft-prompt.
#[allow(deprecated)] // legacy file-based InitOutcome — same as the CLI wizard
fn perform_identity_and_serve_background(ctx: &mut OnboardingContext) {
    // Identity mint (no login required). Best-effort.
    if let Ok(outcome) = crate::identity::init(false) {
        let fingerprint = crate::identity::load_pub_hex()
            .ok()
            .and_then(|h| hex::decode(h.trim()).ok())
            .map(|bytes| crate::identity_wire::fingerprint_short(&bytes))
            .unwrap_or_else(|| outcome.pub_hex.chars().take(12).collect());
        ctx.identity_fingerprint = Some(fingerprint);
    }
    // Node up (detached serve + single-node hash). Best-effort: ignore errors —
    // a missing node does not stop the user from chatting.
    let _ = perform_serve_advertise(ctx);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingLoginChoice {
    Login,
    SkipLocalOnly,
}

pub const SUBSCRIPTION_CLI_SIGNIN_COMMANDS: [&str; 3] = ["claude", "codex", "gemini"];

pub fn parse_onboarding_login_choice(input: &str) -> OnboardingLoginChoice {
    // SYS-B local-first (operator-locked 2026-06-13): a phantom account is an
    // OPT-IN add-on, not the default path. Only an explicit affirmative signs
    // in; blank/Enter and anything ambiguous keep the node local-only so the
    // first run can never dead-end on a broker that's offline or unwanted.
    match input.trim().to_ascii_lowercase().as_str() {
        "1" | "login" | "signin" | "sign-in" | "yes" | "y" | "google" | "apple" | "email"
        | "broker" => OnboardingLoginChoice::Login,
        _ => OnboardingLoginChoice::SkipLocalOnly,
    }
}

/// Rank the detected *subscription* providers (claude / codex / gemini) in
/// priority order. Ollama is NOT appended here: it is a local-server fallback
/// that must be gated on real detection (`detect_local_servers().await`), which
/// this sync helper cannot do. Callers that want the always-on Ollama fallback
/// append `local-ollama` themselves *after* confirming it is present — see
/// `perform_provider_detection` (gated push) and `write_onboarding_config`
/// (appends the local-ollama block last). Pushing it unconditionally here was a
/// regression: it made desktops with no Ollama never reach
/// `NoProviderConfigured`, handed mobile a nonexistent provider, and turned the
/// fallback branches in `perform_provider_detection` into dead code.
pub fn ranked_onboarding_providers(
    has_claude: bool,
    has_codex: bool,
    has_gemini: bool,
) -> Vec<&'static str> {
    let mut ranked = Vec::new();
    if has_claude {
        ranked.push("claude_cli");
    }
    if has_codex {
        ranked.push("codex_oauth");
    }
    if has_gemini {
        ranked.push("gemini_oauth");
    }
    ranked
}

/// D5 side-effect — detect already-signed-in subscription CLIs (Claude / Codex
/// / Gemini) and rank them, with local Ollama as the always-on fallback. Mirrors
/// the CLI wizard's Step 3 detection + priority order.
/// Ollama is the always-available fallback (D5b) when no subscription is found.
///
/// TODO(stage 2): pull shared API keys from the broker vault (D5a paid-tier
/// unlock) and enforce a subscription tier check before defaulting to Ollama.
async fn perform_provider_detection(ctx: &mut OnboardingContext) -> Result<(), OnboardingError> {
    let mut ranked = ranked_onboarding_providers(
        crate::providers::claude_cli::find_claude_token().is_some(),
        crate::providers::codex_cli::find_codex_auth().is_some(),
        crate::providers::gemini_cli::find_gemini_auth().is_some(),
    );

    // D5a — free cloud plugin (default-on): if a free (no-credit-card) key is
    // already present in the environment, add that provider as a cloud fallback,
    // ranked above local Ollama (a real free key beats a maybe-offline local
    // server). This is what lets a desktop with no subscription + no Ollama still
    // resolve a provider instead of dead-ending at `NoProviderConfigured`. No
    // key is read here — only its presence — and nothing secret is stored.
    if let Some(fp) = crate::providers::free_plugin::detect_free_from_env() {
        ranked.push(fp.slug);
    }

    // D5b: local Ollama is the always-on fallback, but ONLY when it is actually
    // running. Gate the push on real detection so a desktop with no Ollama
    // correctly falls through to `NoProviderConfigured` below, and a phone
    // (no local server by design) falls through to the mobile sentinel. An
    // unconditional push here was a regression that broke both paths.
    let local = crate::providers::local_servers::detect_local_servers().await;
    let has_ollama = local.iter().any(|s| s.name == "ollama");
    if has_ollama {
        ranked.push("local-ollama"); // ranked last
    }

    match ranked.first() {
        Some(best) => {
            ctx.provider_slug = Some((*best).to_string());
            Ok(())
        }
        None => match empty_scan_outcome()? {
            Some(slug) => {
                ctx.provider_slug = Some(slug.to_string());
                Ok(())
            }
            // No arm currently returns Ok(None); kept total so a future variant
            // (e.g. "leave unset, prompt later") can't silently fall through.
            None => Ok(()),
        },
    }
}

/// Decide the provider slug when the local scan found NOTHING — no subscription
/// CLI, no free env key, no running local server. Split out of
/// [`perform_provider_detection`] so the no-dead-end decision is unit-testable
/// without touching the filesystem / env / loopback probes (a dev or CI machine
/// may legitimately have Ollama running or a `*_API_KEY` set, which would stop
/// the full detector from ever reaching this branch).
///
/// - MOBILE: a phone has no local CLI tokens and no local server by design — it
///   answers by dispatching to the user's desktop/cloud node (anchor: "phone +
///   cloud") and/or a subscription login set up later. So an empty scan is
///   EXPECTED, not a hard failure: record a sentinel so onboarding completes;
///   the real provider is resolved at runtime. (Found via real-device
///   TestFlight dogfood 2026-06-07.)
/// - DESKTOP: nothing-found is a real "configure a provider" state →
///   `NoProviderConfigured`, UNLESS the opt-in `offline-stub-model` feature is
///   built in, in which case the built-in always-available stub model is
///   selected so a zero-config offline desktop never dead-ends (SPEC-03 §8).
fn empty_scan_outcome() -> Result<Option<&'static str>, OnboardingError> {
    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        Ok(Some("remote-or-subscription"))
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        // SYS-B: never dead-end a zero-config offline desktop. The built-in
        // stub model is always available with nothing installed. Opt-in only —
        // the default build keeps the NoProviderConfigured "configure a
        // provider" state (no regression for cloud/account users).
        #[cfg(feature = "offline-stub-model")]
        {
            Ok(Some("local-stub"))
        }
        #[cfg(not(feature = "offline-stub-model"))]
        {
            Err(OnboardingError::NoProviderConfigured)
        }
    }
}

/// Stage 4 — static FSM transition table keyed on `"<state>:<transition>"`
/// slugs (enums lack `PhfHash`, so we encode the `(state, transition)` pair as
/// a `&'static str` lookup key). Returns `Some(next_state)` for every legal
/// edge per SPEC-28 §7.1 / §8 (5 forward edges, 4 rollback edges, 6 noop
/// edges) and `None` for the explicitly-illegal `Forward` on the terminal
/// `FirstReplyReceived` state — callers translate `None` into the matching
/// wire `OnboardingError` variant.
///
/// 中文: Stage 4 — 用 `phf::Map<&'static str, OnboardingState>` 編譯時表，
/// key 是 `"<state>:<transition>"` 拼接字串（enum 不實作 `PhfHash`，所以用
/// slug 編碼）。`FirstReplyReceived` 的 `Forward` 是合法但「已結束」，這裡
/// 故意不放表 → 呼叫端把 `None` 轉成對應 wire error。
fn fsm_table_pseudo(
    current: OnboardingState,
    transition: OnboardingTransition,
) -> Option<OnboardingState> {
    use OnboardingState::*;
    // TALK-FIRST reorg (DESIGN-ONBOARDING §7): the critical path to a first
    // reply is `fresh_install → set_provider → first_reply_received`. A provider
    // is the ONLY thing needed to chat, so we go straight there; device identity
    // + the mesh node (serve/mDNS) come up in the BACKGROUND and account login
    // is a later soft-prompt — none of them block the first reply.
    //
    // `picked_language` (D2 English-only) and `created_identity` / `joined_cluster`
    // are KEPT as enum variants for wire back-compat, but they are no longer
    // forward TARGETS. A persisted snapshot still sitting on one of the legacy
    // mid-flow states converges forward into `set_provider` (pass-through) so it
    // can never wedge.
    //   fresh_install → set_provider → first_reply_received   (live path)
    //   created_identity / joined_cluster → set_provider      (legacy converge)
    static FSM: phf::Map<&'static str, OnboardingState> = phf::phf_map! {
        // ─── Forward edges (talk-first) ──────────────────────────────────
        "fresh_install:forward"        => SetProvider,       // talk-first: provider first
        "created_identity:forward"     => SetProvider,       // legacy converge
        "joined_cluster:forward"       => SetProvider,       // legacy converge
        "set_provider:forward"         => FirstReplyReceived,
        // `first_reply_received:forward` is intentionally omitted — terminal
        // state. Callers translate the `None` into `onboarding_already_complete`.

        // ─── Rollback edges ──────────────────────────────────────────────
        // The live path is fresh_install → set_provider, so rolling back from
        // set_provider returns to fresh_install. Legacy rollbacks are retained
        // as harmless pass-throughs.
        "set_provider:rollback"        => FreshInstall,
        "joined_cluster:rollback"      => CreatedIdentity,
        // FreshInstall / FirstReplyReceived rollback returns input
        // unchanged → handled by the NoOp branch below (slug fallback).

        // ─── NoOp edges (always return input state) ──────────────────────
        "fresh_install:noop"           => FreshInstall,
        "created_identity:noop"        => CreatedIdentity,
        "joined_cluster:noop"          => JoinedCluster,
        "set_provider:noop"            => SetProvider,
        "first_reply_received:noop"    => FirstReplyReceived,
    };
    let key = format!("{}:{}", state_slug(current), transition_slug(transition));
    FSM.get(key.as_str()).copied()
}

/// Stage 4 helper — render an `OnboardingState` to its canonical snake_case
/// slug used as part of the `fsm_table_pseudo` lookup key. Slugs match the
/// `#[serde(rename_all = "snake_case")]` wire form (single source of truth
/// for the §11.1 wire error catalog as well).
fn state_slug(state: OnboardingState) -> &'static str {
    match state {
        OnboardingState::FreshInstall       => "fresh_install",
        OnboardingState::PickedLanguage     => "picked_language",
        OnboardingState::CreatedIdentity    => "created_identity",
        OnboardingState::JoinedCluster      => "joined_cluster",
        OnboardingState::SetProvider        => "set_provider",
        OnboardingState::FirstReplyReceived => "first_reply_received",
    }
}

/// Stage 4 helper — render an `OnboardingTransition` to its canonical
/// snake_case slug for `fsm_table_pseudo` lookup. Mirrors the
/// `#[serde(rename_all = "snake_case")]` wire form.
fn transition_slug(t: OnboardingTransition) -> &'static str {
    match t {
        OnboardingTransition::Forward  => "forward",
        OnboardingTransition::Rollback => "rollback",
        OnboardingTransition::NoOp     => "noop",
    }
}

/// Stage 3 pseudo — verify §7.1 preconditions hold for the next state.
///
/// Pure FSM-table logic (no extra crate, no I/O). Given the `current` state we
/// resolve the forward target via `fsm_table_pseudo` and assert the runtime
/// `ctx` carries the side-effect that target state implies. The guards mirror
/// the `advance()` docstring one-to-one:
///
/// - `FreshInstall → PickedLanguage`: refuse if `identity_fingerprint.is_some()`
///   (identity already created — a wizard bug). Maps to `IdentityCreationFailed`.
/// - `PickedLanguage → CreatedIdentity`: require `identity_fingerprint.is_some()`
///   else `IdentityCreationFailed`.
/// - `CreatedIdentity → JoinedCluster`: require `cluster_id_hash.is_some()`
///   else `ClusterJoinFailed`.
/// - `JoinedCluster → SetProvider` / `SetProvider → FirstReplyReceived`:
///   require `provider_slug.is_some()` OR `demo_relay_used` else
///   `NoProviderConfigured`.
///
/// States with no forward edge (terminal `FirstReplyReceived`) have no
/// precondition to check and return `Ok(())`; the missing-edge case is handled
/// by `advance()` itself.
fn precondition_check_pseudo(
    current: OnboardingState,
    ctx: &OnboardingContext,
) -> Result<(), OnboardingError> {
    use OnboardingState::*;
    // Resolve the forward target; if there is none (terminal) there is no
    // INTO-state precondition to enforce here.
    let next = match fsm_table_pseudo(current, OnboardingTransition::Forward) {
        Some(n) => n,
        None => return Ok(()),
    };
    match next {
        // PickedLanguage is no longer a reachable forward target (D2:
        // English-only). It is retained as a no-precondition pass-through so
        // any stale persisted snapshot pointing at it cannot wedge the FSM.
        PickedLanguage => Ok(()),
        // Moving INTO CreatedIdentity (D1, login-first): BOTH the OAuth login
        // AND the ed25519 identity must have succeeded. This single GUI step
        // covers broker login (→ identity_provider + identity_sub) and the
        // local keystore mint (→ identity_fingerprint), so all three must be
        // present before we record CreatedIdentity.
        CreatedIdentity => {
            if ctx.identity_provider.is_none() || ctx.identity_sub.is_none() {
                return Err(OnboardingError::IdentityCreationFailed {
                    detail: "login required before identity (provider/sub missing)".to_string(),
                });
            }
            if ctx.identity_fingerprint.is_none() {
                return Err(OnboardingError::IdentityCreationFailed {
                    detail: "identity fingerprint missing".to_string(),
                });
            }
            Ok(())
        }
        // Moving INTO JoinedCluster (D4, auto-mesh staged): a cluster id hash
        // must be present, but for Stage 1 this MAY be a "self" / single-node
        // hash — the node serves + mDNS-advertises itself and full peer-join
        // (vault sync, SPEC-10 `/rpc/cluster/pair`) is deferred to Stage 2.
        JoinedCluster => {
            if ctx.cluster_id_hash.is_none() {
                return Err(OnboardingError::ClusterJoinFailed {
                    detail: "cluster id hash missing".to_string(),
                });
            }
            Ok(())
        }
        // Moving INTO SetProvider or FirstReplyReceived: a provider must be
        // configured, either a BYOM slug or the demo-relay path.
        SetProvider | FirstReplyReceived => {
            if ctx.provider_slug.is_some() || ctx.demo_relay_used {
                Ok(())
            } else {
                Err(OnboardingError::NoProviderConfigured)
            }
        }
        // No other state is a forward target.
        FreshInstall => Ok(()),
    }
}

/// Apply one rollback FSM transition per the §8 mermaid diagram. Only 4
/// edges are reversible:
///
/// - `PickedLanguage → FreshInstall`
/// - `CreatedIdentity → PickedLanguage` (identity.key NOT deleted — user
///   can pick a different language without re-running keystore write)
/// - `JoinedCluster → CreatedIdentity` (cluster pair side effect IS undone
///   via SPEC-10 `/rpc/cluster/leave` so the next attempt is clean)
/// - `SetProvider → JoinedCluster` (BYOM key removed from SPEC-15 vault
///   or `agents.toml` so the next attempt is clean)
///
/// `FreshInstall` rolls back to itself (returns input unchanged — wizard UI
/// MUST hide the rollback button at this step but Stage 2 must be safe).
/// `FirstReplyReceived` is terminal: rollback returns input unchanged (the
/// onboarding journey is over; resetting is a separate "factory reset" flow
/// out of SPEC-28 scope).
///
/// 中文: 套用 §8 mermaid 圖中的 rollback 邊。只 4 條可逆：PickedLanguage→
/// FreshInstall / CreatedIdentity→PickedLanguage（identity.key **不刪**）/
/// JoinedCluster→CreatedIdentity（cluster pair 有撤銷）/ SetProvider→
/// JoinedCluster（BYOM key 移除）。FreshInstall 與 FirstReplyReceived 上的
/// rollback 都原地不動（terminal / initial）。
pub fn rollback(
    snapshot: &OnboardingStateSnapshot,
) -> Result<OnboardingState, OnboardingError> {
    // Step 1 — Lookup (current, Rollback) in the same FSM table per
    // SPEC-28 §8. Reusing the static phf::Map keeps forward + rollback
    // dispatch symmetric and avoids two-table drift. The table only carries
    // the 4 reversible edges; FreshInstall (initial) and FirstReplyReceived
    // (terminal) are absent → `None`.
    let previous_state = fsm_table_pseudo(snapshot.current_state, OnboardingTransition::Rollback);

    // Step 2 — Some §8 edges are explicitly NOT reversible because the
    // forward side effect cannot be safely undone:
    //   - `CreatedIdentity → PickedLanguage` is reversible (identity.key is
    //     kept) but anything past it that destroys keystore material is
    //     refused — e.g. a hypothetical future `RotatedIdentity` step
    //     would orphan the prior key and is therefore not in the rollback
    //     table.
    // For the non-reversible boundary states (table miss: FreshInstall /
    // FirstReplyReceived) we return a stable wire `OnboardingError` so the
    // wizard UI can hide the rollback button. The §11.1 catalog has no
    // dedicated "not reversible" variant (that is FSM-internal per the module
    // docstring); among the available variants `NoProviderConfigured` is the
    // only one that is both non-retryable AND carries no payload implying a
    // transient I/O failure, so it is the stable sentinel for "this FSM
    // boundary cannot move backward".
    match previous_state {
        Some(prev) => Ok(prev),
        // FreshInstall / FirstReplyReceived are non-reversible. We surface a
        // stable, non-retryable wire variant so the UI funnels correctly
        // rather than silently no-op'ing. `NoProviderConfigured` is the only
        // §11.1 variant that is both non-retryable and carries no payload
        // implying a transient I/O failure, so it is the stable sentinel for
        // "this FSM boundary cannot move backward".
        None => Err(OnboardingError::NoProviderConfigured),
    }
}

/// Compute the TTFR metric from the two endpoint timestamps and enforce the
/// SPEC-28 §1 / §12 p95 budget of 30,000 ms. Returns
/// `OnboardingError::TtfrBudgetExceeded` when `first_reply_at_ms -
/// install_complete_at_ms > 30_000`.
///
/// Edge cases Stage 2 MUST handle:
///
/// - `first_reply_at_ms < install_complete_at_ms` (clock skew) → treat as 0
///   ms instead of underflowing u64 (return a TTFRMetric with total_ms=0
///   and let the analytics sink flag the skew separately).
/// - `total_ms == 30_000` exactly → allowed (budget is `<=` in the SPEC).
/// - `total_ms > 30_000` → return `TtfrBudgetExceeded`, but **also**
///   construct the metric so the caller can still emit telemetry with the
///   over-budget value (Stage 2 will likely change the signature to
///   `Result<(TTFRMetric, Option<OnboardingError>), Infallible>` — kept
///   simple here for Stage 1).
///
/// 中文: 從兩個時間戳算 TTFR 並強制 §1 / §12 p95 < 30 秒預算。超過回
/// `TtfrBudgetExceeded`。Stage 2 要處理時鐘偏移、邊界值、超預算同時仍要
/// 能 emit telemetry 的細節。
pub fn compute_ttfr(
    install_at_ms: u64,
    first_reply_at_ms: u64,
) -> Result<TTFRMetric, OnboardingError> {
    // Step 1 — Compute `total_ms = first_reply_at_ms - install_at_ms` with
    // saturating subtraction so a backwards clock skew (NTP adjustment
    // during onboarding) does NOT underflow to ~u64::MAX. A negative delta
    // clamps to 0 and is surfaced as an in-budget metric (the analytics sink
    // flags the skew separately per the §1 docstring).
    let total_ms: u64 = first_reply_at_ms.saturating_sub(install_at_ms);

    // Step 2 — Emit the metric on the telemetry sink so SPEC-50 analytics can
    // compute the rolling p50 / p95 distributions. Emitted unconditionally so
    // even an over-budget value is still recorded.
    otel_emit_pseudo("onboarding.ttfr.total_ms", total_ms);

    // Step 3 — Assemble the wire metric. `total_ms` is stored explicitly and
    // MUST equal `first_reply_at_ms - install_complete_at_ms` (saturating).
    let metric = TTFRMetric {
        install_complete_at_ms: install_at_ms,
        first_reply_at_ms,
        total_ms,
    };

    // Step 4 — Enforce the §1 / §12 p95 budget: `total_ms <= 30_000` (the
    // SPEC is inclusive on the boundary). A value of exactly 30_000 ms is
    // allowed; 30_001 ms is over budget → surface `TtfrBudgetExceeded` so the
    // telemetry sink can track the regression, carrying the over-budget value.
    if total_ms <= 30_000 {
        Ok(metric)
    } else {
        Err(OnboardingError::TtfrBudgetExceeded { total_ms })
    }
}

/// Stage 3 pseudo — emit one numeric metric on the OTEL (OpenTelemetry,
/// 開放遙測標準) pipeline. Backed by `opentelemetry` crate; no other
/// dependency is allowed at this seam to keep the wire layer slim.
fn otel_emit_pseudo(_metric_name: &'static str, _value: u64) {
    // Default build: no-op sink. A real OpenTelemetry exporter is wired behind
    // an opt-in feature so the wire layer stays dependency-slim and
    // `compute_ttfr` is callable on a default build (SPEC-13 telemetry is
    // opt-in only per §13). When the `otel` feature lands it replaces this
    // body with the `opentelemetry` meter emit; the call site is unchanged.
    #[cfg(feature = "otel")]
    {
        // Placeholder for the real exporter (feature-gated to keep default
        // builds free of the dependency). Intentionally references the args so
        // the gated build still type-checks.
        let _ = (_metric_name, _value);
    }
}

/// Decide whether the wizard should fall back to the demo-relay (SPEC-52)
/// path instead of waiting for the user to configure a BYOM key. Returns
/// `true` iff **both**:
///
/// - `ctx.cluster_id_hash.is_none()` (user has not joined any cluster, so
///   no other phantom-mesh peer can serve LLM inference for them)
/// - `ctx.provider_slug.is_none()` (user has not configured a BYOM key
///   either, so local inference is also unavailable)
///
/// `ctx.demo_relay_used` being already `true` does NOT short-circuit — this
/// helper is a pure decision function, not an idempotency gate. The caller
/// (wizard) tracks whether it already started the handshake.
///
/// 中文: 判斷 wizard 是否該 fallback 走 demo-relay。**且**條件：沒加入
/// cluster、沒設 BYOM key。`demo_relay_used` 已是 true 不會 short-circuit
/// — 這只是純決策函式，呼叫端自己管 idempotency（冪等性）。
pub fn should_fallback_to_demo_relay(ctx: &OnboardingContext) -> bool {
    // Step 1 — Pure decision per SPEC-28 §10.4: fall back to demo-relay
    // iff the user has neither joined a cluster nor configured a BYOM
    // (Bring Your Own Model) key. No I/O, no helper crate needed.
    ctx.cluster_id_hash.is_none() && ctx.provider_slug.is_none()
}

/// Start the demo-relay (SPEC-52) handoff by calling
/// `GET demo.phantommesh.io/quota` per §9.7 and parsing the response into
/// a `DemoRelayHandoff`. Returns `OnboardingError::DemoRelayExhausted` when
/// the relay reports `quota_remaining == 0`.
///
/// Stage 2 invariants:
///
/// - URL MUST be HTTPS (TLS) — never bare HTTP, no plaintext fallback.
/// - Request MUST NOT include any identifying header (no User-Agent with
///   device id, no Cookie, no Authorization). The relay rate-limits by
///   source IP per §13.
/// - Response parse failure → log length + status code (never raw body) and
///   return `DemoRelayExhausted` (degraded mode — caller falls back to
///   BYOM prompt UI).
/// - Network unreachable → map to `DemoRelayExhausted` so the UI funnels
///   to the same "set up a BYOM key" CTA (Call To Action).
///
/// 中文: 啟動 demo-relay handoff，打 §9.7 endpoint 拿 quota。HTTPS、不帶
/// 識別性 header、parse fail / 網路 fail 統一回 `DemoRelayExhausted` 讓
/// UI 走「設 BYOM key」CTA（行動呼籲按鈕）。
pub fn start_demo_relay_handoff() -> Result<DemoRelayHandoff, OnboardingError> {
    // Step 1 — Issue an HTTPS GET to `demo.phantommesh.io/relay/quota` via
    // `https_get_pseudo`. The transport seam exists so test code can swap
    // in a mock without pulling reqwest into unit tests.
    let _response_body = https_get_pseudo("https://demo.phantommesh.io/relay/quota");

    // Step 2 — Parse `quota_remaining` (and `relay_url` + `ttl_seconds`)
    // out of the JSON response body. Parse failures are caught upstream
    // and degraded to `DemoRelayExhausted` per §13 (never leak raw body).
    let _quota_remaining: u8 = 0; // populated by Stage 3 JSON parse

    // Step 3 — Branch on `quota_remaining`: > 0 ⇒ return a fresh
    // `DemoRelayHandoff`; == 0 ⇒ return `OnboardingError::DemoRelayExhausted`
    // so the UI funnels to the BYOM-key CTA (Call To Action).
    unimplemented!("Stage 3: branch on parsed quota → DemoRelayHandoff | DemoRelayExhausted")
}

/// Stage 3 pseudo — HTTPS GET seam for the demo-relay quota check.
/// Backed by `reqwest`; no identifying headers per SPEC-28 §13.
fn https_get_pseudo(_url: &str) -> Result<String, OnboardingError> {
    unimplemented!("Stage 3: reqwest")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboarding_state_snapshot_round_trip_smoke() {
        // §7.1 invariant: TS encode → wire → Rust decode → re-encode preserves
        // the snapshot fields. Stage 1 sanity-checks serde; deeper invariants
        // (e.g. entered_at_ms monotonicity within a session) come in Stage 2.
        let s = OnboardingStateSnapshot {
            current_state: OnboardingState::PickedLanguage,
            entered_at_ms: 1_716_563_400_000,
            retry_count: 0,
            last_error: None,
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: OnboardingStateSnapshot = serde_json::from_str(&j).unwrap();
        assert_eq!(s.current_state, back.current_state);
        assert_eq!(s.entered_at_ms, back.entered_at_ms);
        assert_eq!(s.retry_count, back.retry_count);
        assert_eq!(s.last_error, back.last_error);
    }

    #[test]
    fn onboarding_context_default_is_all_none() {
        // §7.1 invariant: a freshly-installed user has no derived
        // side-effects yet — every Option field is None and demo_relay_used
        // is false. Stage 2 relies on this default for the
        // `should_fallback_to_demo_relay()` decision.
        let ctx = OnboardingContext::default();
        assert!(ctx.cluster_id_hash.is_none());
        assert!(ctx.identity_fingerprint.is_none());
        assert!(ctx.provider_slug.is_none());
        assert!(!ctx.demo_relay_used);
    }

    #[test]
    fn ttfr_metric_round_trip_smoke() {
        let m = TTFRMetric {
            install_complete_at_ms: 1_716_563_400_000,
            first_reply_at_ms: 1_716_563_412_500,
            total_ms: 12_500,
        };
        let j = serde_json::to_string(&m).unwrap();
        let back: TTFRMetric = serde_json::from_str(&j).unwrap();
        assert_eq!(m.total_ms, back.total_ms);
        assert_eq!(m.install_complete_at_ms, back.install_complete_at_ms);
        assert_eq!(m.first_reply_at_ms, back.first_reply_at_ms);
    }

    #[test]
    fn ttfr_budget_error_serializes_with_code_tag() {
        // §11.1 invariant: error wire shape uses `{"code": "..."}` tag so
        // the UI can dispatch on the machine-readable code string. A 35-s
        // TTFR overshoots the 30-s p95 budget per §1 / §12 — verify the
        // error variant exists and round-trips on the wire (Stage 2 will
        // wire `compute_ttfr` to actually return it on overshoot).
        let over_budget_total_ms: u64 = 35_000;
        assert!(
            over_budget_total_ms > 30_000,
            "test premise: 35s overshoots the 30s p95 budget"
        );
        let e = OnboardingError::TtfrBudgetExceeded {
            total_ms: over_budget_total_ms,
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("ttfr_budget_exceeded"), "wire shape: {}", j);
        assert!(j.contains("35000"), "payload preserved: {}", j);
    }

    #[test]
    fn onboarding_error_demo_relay_exhausted_serializes() {
        // §11.1: `DEMO_RELAY_QUOTA_EXHAUSTED` maps to this wire variant
        // (rendered snake_case per `#[serde(rename_all = "snake_case")]`).
        let e = OnboardingError::DemoRelayExhausted;
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("demo_relay_exhausted"), "wire shape: {}", j);
    }

    #[test]
    fn demo_relay_handoff_round_trip_smoke() {
        let h = DemoRelayHandoff {
            relay_url: "https://demo.phantommesh.io".to_string(),
            quota_remaining: 3,
            ttl_seconds: 300,
        };
        let j = serde_json::to_string(&h).unwrap();
        let back: DemoRelayHandoff = serde_json::from_str(&j).unwrap();
        assert_eq!(h.relay_url, back.relay_url);
        assert_eq!(h.quota_remaining, back.quota_remaining);
        assert_eq!(h.ttl_seconds, back.ttl_seconds);
    }

    #[test]
    fn fsm_table_pseudo_forward_chain_advances_one_step() {
        // TALK-FIRST invariant: the live forward path is
        //   fresh_install → set_provider → first_reply_received
        // (identity/cluster are background, not forward targets).
        let mut state = OnboardingState::FreshInstall;
        let expected = [
            OnboardingState::SetProvider, // talk-first: straight to provider
            OnboardingState::FirstReplyReceived,
        ];
        for next in expected {
            state = fsm_table_pseudo(state, OnboardingTransition::Forward)
                .expect("legal forward edge");
            assert_eq!(state, next);
        }
    }

    #[test]
    fn fsm_table_pseudo_fresh_install_goes_straight_to_set_provider() {
        // TALK-FIRST: fresh_install jumps directly to set_provider; identity +
        // cluster are no longer forward targets (they run in the background).
        assert_eq!(
            fsm_table_pseudo(OnboardingState::FreshInstall, OnboardingTransition::Forward),
            Some(OnboardingState::SetProvider),
        );
        // legacy mid-flow snapshots converge forward into set_provider too.
        assert_eq!(
            fsm_table_pseudo(OnboardingState::CreatedIdentity, OnboardingTransition::Forward),
            Some(OnboardingState::SetProvider),
        );
        assert_eq!(
            fsm_table_pseudo(OnboardingState::JoinedCluster, OnboardingTransition::Forward),
            Some(OnboardingState::SetProvider),
        );
    }

    #[test]
    fn fsm_table_pseudo_terminal_forward_is_none() {
        // §8 invariant: Forward from FirstReplyReceived is intentionally
        // not in the table; callers translate `None` into the
        // `onboarding_already_complete` wire error per the comment above
        // `fsm_table_pseudo`.
        assert!(
            fsm_table_pseudo(
                OnboardingState::FirstReplyReceived,
                OnboardingTransition::Forward,
            )
            .is_none()
        );
    }

    #[test]
    fn fsm_table_pseudo_rollback_only_on_reversible_edges() {
        // TALK-FIRST: the live path is fresh_install → set_provider, so rolling
        // back from set_provider returns to fresh_install. The legacy
        // joined_cluster→created_identity rollback is kept as a harmless
        // pass-through. FreshInstall / CreatedIdentity / FirstReplyReceived have
        // no rollback edge.
        assert_eq!(
            fsm_table_pseudo(OnboardingState::SetProvider, OnboardingTransition::Rollback),
            Some(OnboardingState::FreshInstall),
        );
        assert_eq!(
            fsm_table_pseudo(OnboardingState::JoinedCluster, OnboardingTransition::Rollback),
            Some(OnboardingState::CreatedIdentity),
        );
        assert!(
            fsm_table_pseudo(OnboardingState::FreshInstall, OnboardingTransition::Rollback)
                .is_none()
        );
        assert!(
            fsm_table_pseudo(OnboardingState::CreatedIdentity, OnboardingTransition::Rollback)
                .is_none()
        );
        assert!(
            fsm_table_pseudo(
                OnboardingState::FirstReplyReceived,
                OnboardingTransition::Rollback,
            )
            .is_none()
        );
    }

    #[test]
    fn fsm_table_pseudo_noop_returns_input_for_every_reachable_state() {
        // §9.3 invariant: NoOp is a pure read of the current state — every one
        // of the 5 reachable states (D2: `picked_language` removed) must
        // round-trip through the table unchanged.
        for state in [
            OnboardingState::FreshInstall,
            OnboardingState::CreatedIdentity,
            OnboardingState::JoinedCluster,
            OnboardingState::SetProvider,
            OnboardingState::FirstReplyReceived,
        ] {
            assert_eq!(
                fsm_table_pseudo(state, OnboardingTransition::NoOp),
                Some(state),
                "noop must return input for {:?}",
                state,
            );
        }
    }

    #[test]
    fn onboarding_progress_event_round_trip_smoke() {
        let ev = OnboardingProgressEvent {
            state: OnboardingState::CreatedIdentity,
            context: OnboardingContext {
                cluster_id_hash: None,
                identity_fingerprint: Some("abcdef012345".to_string()),
                provider_slug: None,
                demo_relay_used: false,
                identity_provider: Some("google".to_string()),
                identity_sub: Some("user42@example.com".to_string()),
            },
            snapshot_at_ms: 1_716_563_400_000,
        };
        let j = serde_json::to_string(&ev).unwrap();
        let back: OnboardingProgressEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(ev.state, back.state);
        assert_eq!(ev.context.identity_fingerprint, back.context.identity_fingerprint);
        assert_eq!(ev.snapshot_at_ms, back.snapshot_at_ms);
    }

    // ─── Pure FSM helper tests (advance / rollback / precondition_check) ─────

    fn snap(state: OnboardingState) -> OnboardingStateSnapshot {
        OnboardingStateSnapshot {
            current_state: state,
            entered_at_ms: 0,
            retry_count: 0,
            last_error: None,
        }
    }

    #[test]
    fn onboarding_login_choice_distinguishes_login_from_skip() {
        // SYS-B local-first: only an explicit affirmative signs in.
        assert_eq!(parse_onboarding_login_choice("1"), OnboardingLoginChoice::Login);
        assert_eq!(
            parse_onboarding_login_choice("google"),
            OnboardingLoginChoice::Login
        );
        // Blank/Enter is the local-first DEFAULT (the SYS-B change): a fresh run
        // stays local-only unless the user opts in.
        assert_eq!(
            parse_onboarding_login_choice(""),
            OnboardingLoginChoice::SkipLocalOnly
        );
        assert_eq!(
            parse_onboarding_login_choice("skip"),
            OnboardingLoginChoice::SkipLocalOnly
        );
        assert_eq!(
            parse_onboarding_login_choice("2"),
            OnboardingLoginChoice::SkipLocalOnly
        );
    }

    #[test]
    fn onboarding_provider_ranking_orders_detected_subscriptions() {
        // The helper ranks subscription providers only; `local-ollama` is NOT
        // appended here (it is gated on real `detect_local_servers()` in
        // `perform_provider_detection`). See the regression fix from review #321.
        assert_eq!(
            ranked_onboarding_providers(true, true, true),
            vec!["claude_cli", "codex_oauth", "gemini_oauth"]
        );
        assert_eq!(
            ranked_onboarding_providers(false, true, false),
            vec!["codex_oauth"]
        );
    }

    #[test]
    fn onboarding_provider_ranking_none_detected_is_empty() {
        // No subscriptions → empty ranking. Ollama is NOT auto-appended; whether
        // a local fallback exists is decided later by real Ollama detection, so a
        // desktop without Ollama correctly reaches `NoProviderConfigured`.
        assert_eq!(
            ranked_onboarding_providers(false, false, false),
            Vec::<&'static str>::new()
        );
    }

    // P0-7 S2 — the empty-scan (no sub / no free / no local server) decision.
    // Tested via the extracted `empty_scan_outcome` helper so the assertion is
    // deterministic regardless of the test machine's real Ollama / *_API_KEY /
    // CLI-token state (which would otherwise stop the full detector from ever
    // reaching this branch).
    #[cfg(all(
        feature = "offline-stub-model",
        not(any(target_os = "ios", target_os = "android"))
    ))]
    #[test]
    fn empty_scan_resolves_local_stub_with_feature() {
        // Offline desktop, nothing installed, stub feature ON: must NOT
        // dead-end — the built-in always-available stub model is selected.
        let outcome = empty_scan_outcome();
        assert!(
            matches!(outcome, Ok(Some("local-stub"))),
            "offline desktop with offline-stub-model must resolve the built-in stub, not dead-end: {outcome:?}"
        );
    }

    #[cfg(all(
        not(feature = "offline-stub-model"),
        not(any(target_os = "ios", target_os = "android"))
    ))]
    #[test]
    fn empty_scan_is_no_provider_configured_without_feature() {
        // No-regression guard (Step 6): WITHOUT the opt-in feature, an empty
        // desktop scan still returns NoProviderConfigured — opted-in / cloud
        // users' flow is byte-identical to before P0-7.
        let outcome = empty_scan_outcome();
        assert!(
            matches!(outcome, Err(OnboardingError::NoProviderConfigured)),
            "default build must keep the desktop dead-end behavior: {outcome:?}"
        );
    }

    #[test]
    fn advance_fresh_install_to_set_provider_requires_provider_only() {
        // TALK-FIRST: FreshInstall advances DIRECTLY to SetProvider, and the
        // ONLY precondition is a provider (login + identity are background, not
        // required). No provider → NoProviderConfigured.
        let bare = OnboardingContext::default();
        let err = advance(&snap(OnboardingState::FreshInstall), &bare)
            .expect_err("no provider must be refused");
        assert!(matches!(err, OnboardingError::NoProviderConfigured));

        let with_provider = OnboardingContext {
            provider_slug: Some("groq".to_string()),
            ..Default::default()
        };
        let next = advance(&snap(OnboardingState::FreshInstall), &with_provider)
            .expect("a provider advances to set_provider");
        assert_eq!(next, OnboardingState::SetProvider);
    }

    #[test]
    fn advance_fresh_install_does_not_require_login() {
        // TALK-FIRST: a provider with NO login/identity still advances — login
        // is a later soft-prompt, never a gate.
        let provider_no_login = OnboardingContext {
            provider_slug: Some("groq".to_string()),
            ..Default::default()
        };
        assert!(provider_no_login.identity_provider.is_none());
        let next = advance(&snap(OnboardingState::FreshInstall), &provider_no_login)
            .expect("login is NOT required for the first reply");
        assert_eq!(next, OnboardingState::SetProvider);
    }

    #[test]
    fn advance_legacy_created_identity_converges_to_set_provider() {
        // A persisted snapshot stuck on the legacy CreatedIdentity state
        // converges forward into SetProvider — needing only a provider.
        let no_provider = OnboardingContext {
            identity_fingerprint: Some("abcdef012345".to_string()),
            ..Default::default()
        };
        let err = advance(&snap(OnboardingState::CreatedIdentity), &no_provider)
            .expect_err("legacy state still needs a provider to converge");
        assert!(matches!(err, OnboardingError::NoProviderConfigured));

        let with_provider = OnboardingContext {
            provider_slug: Some("groq".to_string()),
            ..Default::default()
        };
        let next = advance(&snap(OnboardingState::CreatedIdentity), &with_provider)
            .expect("legacy state converges to set_provider");
        assert_eq!(next, OnboardingState::SetProvider);
    }

    #[test]
    fn advance_into_set_provider_requires_provider_or_demo_relay() {
        // JoinedCluster → SetProvider needs provider_slug OR demo_relay_used.
        let none_configured = OnboardingContext {
            identity_fingerprint: Some("abcdef012345".to_string()),
            cluster_id_hash: Some("deadbeef".to_string()),
            ..Default::default()
        };
        let err = advance(&snap(OnboardingState::JoinedCluster), &none_configured)
            .expect_err("no provider configured must be refused");
        assert!(matches!(err, OnboardingError::NoProviderConfigured));

        // BYOM slug satisfies it.
        let byom = OnboardingContext {
            provider_slug: Some("groq".to_string()),
            cluster_id_hash: Some("deadbeef".to_string()),
            ..Default::default()
        };
        let next = advance(&snap(OnboardingState::JoinedCluster), &byom)
            .expect("byom provider advances");
        assert_eq!(next, OnboardingState::SetProvider);

        // demo-relay path also satisfies it.
        let demo = OnboardingContext {
            demo_relay_used: true,
            cluster_id_hash: Some("deadbeef".to_string()),
            ..Default::default()
        };
        let next = advance(&snap(OnboardingState::JoinedCluster), &demo)
            .expect("demo relay path advances");
        assert_eq!(next, OnboardingState::SetProvider);
    }

    #[test]
    fn advance_set_provider_to_first_reply_requires_provider() {
        let configured = OnboardingContext {
            provider_slug: Some("openai".to_string()),
            ..Default::default()
        };
        let next = advance(&snap(OnboardingState::SetProvider), &configured)
            .expect("configured provider advances to first reply");
        assert_eq!(next, OnboardingState::FirstReplyReceived);

        let bare = OnboardingContext::default();
        let err = advance(&snap(OnboardingState::SetProvider), &bare)
            .expect_err("no provider at set_provider must be refused");
        assert!(matches!(err, OnboardingError::NoProviderConfigured));
    }

    #[test]
    fn advance_terminal_state_is_noop_ok() {
        // FirstReplyReceived has no forward edge — advance returns input
        // unchanged (onboarding already complete), not an error.
        let ctx = OnboardingContext::default();
        let next = advance(&snap(OnboardingState::FirstReplyReceived), &ctx)
            .expect("terminal advance is a no-op Ok");
        assert_eq!(next, OnboardingState::FirstReplyReceived);
    }

    #[test]
    fn rollback_succeeds_on_reversible_edges() {
        // TALK-FIRST: set_provider rolls back to fresh_install (the live path).
        // The legacy joined_cluster→created_identity rollback is kept.
        assert_eq!(
            rollback(&snap(OnboardingState::SetProvider)).unwrap(),
            OnboardingState::FreshInstall,
        );
        assert_eq!(
            rollback(&snap(OnboardingState::JoinedCluster)).unwrap(),
            OnboardingState::CreatedIdentity,
        );
    }

    #[test]
    fn rollback_non_reversible_states_return_stable_error() {
        // FreshInstall (initial), CreatedIdentity (login+identity is not
        // un-done) and FirstReplyReceived (terminal) are non-reversible →
        // stable non-retryable wire error.
        let err = rollback(&snap(OnboardingState::FreshInstall))
            .expect_err("initial state is non-reversible");
        assert!(matches!(err, OnboardingError::NoProviderConfigured));

        let err = rollback(&snap(OnboardingState::CreatedIdentity))
            .expect_err("created_identity is non-reversible (D2: no language step)");
        assert!(matches!(err, OnboardingError::NoProviderConfigured));

        let err = rollback(&snap(OnboardingState::FirstReplyReceived))
            .expect_err("terminal state is non-reversible");
        assert!(matches!(err, OnboardingError::NoProviderConfigured));
    }

    #[test]
    fn precondition_check_terminal_is_ok() {
        // No forward edge from the terminal state → nothing to guard → Ok.
        let ctx = OnboardingContext::default();
        assert!(precondition_check_pseudo(OnboardingState::FirstReplyReceived, &ctx).is_ok());
    }

    // ─── compute_ttfr — SPEC-28 §1 / §12 budget gate (table test) ───────────

    #[test]
    fn compute_ttfr_budget_boundary_table() {
        // SPEC-28 §1 / §12: budget is inclusive on 30_000 ms.
        // Each row: (install_at_ms, first_reply_at_ms) → expectation.

        // Row 1 — exactly 30_000 ms → Ok (budget is `<=`).
        let m = compute_ttfr(1_000, 31_000).expect("30_000 ms is within budget");
        assert_eq!(m.total_ms, 30_000, "boundary diff exact");
        assert_eq!(m.install_complete_at_ms, 1_000);
        assert_eq!(m.first_reply_at_ms, 31_000);

        // Row 2 — 30_001 ms → Err(TtfrBudgetExceeded { total_ms: 30_001 }).
        let e = compute_ttfr(1_000, 31_001).expect_err("30_001 ms overshoots budget");
        assert!(
            matches!(e, OnboardingError::TtfrBudgetExceeded { total_ms } if total_ms == 30_001),
            "over-budget must surface TtfrBudgetExceeded with the over-budget total_ms, got {e:?}",
        );

        // Row 3 — clock skew (start > end) → saturating_sub yields 0 → Ok with total_ms==0.
        let skew = compute_ttfr(50_000, 10_000).expect("clock skew clamps to 0, not underflow");
        assert_eq!(skew.total_ms, 0, "saturating_sub clamps negative delta to 0");
        assert_eq!(skew.install_complete_at_ms, 50_000);
        assert_eq!(skew.first_reply_at_ms, 10_000);

        // Row 4 — typical happy path 12_500 ms → Ok with the exact diff.
        let happy = compute_ttfr(1_716_563_400_000, 1_716_563_412_500)
            .expect("12_500 ms is well within budget");
        assert_eq!(happy.total_ms, 12_500, "happy-path diff exact");
        assert_eq!(happy.install_complete_at_ms, 1_716_563_400_000);
        assert_eq!(happy.first_reply_at_ms, 1_716_563_412_500);
    }
}
