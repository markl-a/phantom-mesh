// SPEC-10 §7 — RPC wire types (single source of truth for the 16 RPC + 1 API
// endpoints that make up the mesh peer-to-peer protocol).
//
// Stage 3 (real impl — HMAC live): the five crypto helpers below are now
// backed by `sha2` / `hmac` / `hex` / `subtle` (constant-time compare). The
// pseudocode bodies + `#[should_panic(expected = "Stage 3")]` marker tests
// from Stage 2 have been removed; KAT (known-answer-test) vectors take their
// place in the `#[cfg(test)] mod tests` block.
//
// 中文: 本檔對應 SPEC-10 §7（資料模型）。所有 17 個 endpoint 共用 RpcRequest /
// RpcResponse 外殼（envelope），data 欄位裝 endpoint-specific payload。HMAC
// canonical-string 公式請見 build_canonical_string 函式 doc-comment。
//
// TODO Stage 4: migrate `core/src/serve.rs` HTTP handlers + the legacy
// `pm-types::rpc::RpcRequest` (different shape: request_id/signature/chain) to
// use the types in this module.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ─── §7.1/§7.2 RpcRequest envelope ────────────────────────────────────────────

/// Wire envelope for every outbound RPC. `data` is endpoint-specific payload.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct RpcRequest<T> {
    pub meta: RpcRequestMeta,
    pub data: T,
}

/// Request-side meta header (carried in body, not HTTP header — see §7.1 for
/// why iOS Tauri prefers body-JSON for cross-platform robustness).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct RpcRequestMeta {
    /// Wire protocol major version. v0.6.0 = 1; v0.7.0 E2EE bumps to 2.
    pub protocol_version: u8,
    /// 32-hex W3C Trace Context trace-id (echo of `traceparent` HTTP header).
    pub trace_id: String,
    /// Optional client dedupe key; server returns same response for 24h.
    pub idempotency_key: Option<String>,
    pub client_os: ClientOs,
    /// Semver, e.g. `"0.6.0"`.
    pub client_version: String,
}

/// Originating OS of the calling peer (for cross-OS stats + capability gating).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "lowercase")]
pub enum ClientOs {
    Mac,
    Win,
    Linux,
    Ios,
    Android,
}

// ─── §7.3 RpcResponse envelope ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct RpcResponse<T> {
    pub meta: RpcResponseMeta,
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct RpcResponseMeta {
    pub protocol_version: u8,
    pub trace_id: String,
    pub server_peer_name: String,
    pub server_version: String,
    /// ISO-8601 UTC (e.g. `"2026-05-24T10:30:00Z"`). Kept as String for byte-
    /// identical round-trip per §7.4 invariant — Stage 2 may switch to
    /// `chrono::DateTime<chrono::Utc>` once serde format is locked.
    pub handled_at: String,
}

/// Bilingual user-facing error payload (zh-TW + en) — see SPEC-04 error catalog
/// for the canonical machine codes referenced by `code`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct RpcError {
    /// Machine-readable code, e.g. `"auth_invalid"`.
    pub code: String,
    pub message_zh_tw: String,
    pub message_en: String,
    /// Short imperative phrase shown to the user.
    pub recovery_action: String,
    pub retryable: bool,
    pub retry_after_s: Option<u32>,
}

// ─── §9 Per-endpoint payloads ────────────────────────────────────────────────
//
// Each endpoint has a Request payload (`T` placed inside `RpcRequest<T>`) and a
// Response payload (`T` placed inside `RpcResponse<T>`). For read-only endpoints
// (`/rpc/ping`, `/rpc/peers`, `/rpc/task/status/:id`, `/rpc/skill/since/:ts`,
// `DELETE /rpc/task/:id`) the request payload is the empty struct
// `EmptyRequest` to keep the envelope shape uniform.

/// Marker for endpoints whose `data` is `{}` (read-only or path-param only).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
pub struct EmptyRequest {}

// 9.1/9.2 — Ping + peer-list responses share PeerStatus shape (§6.2)

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct PingResponse {
    pub peer_name: String,
    pub os: ClientOs,
    pub version: String,
    pub capabilities: Vec<String>,
    pub cluster_fingerprint: String,
    pub uptime_s: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct PeerSummary {
    pub peer_name: String,
    pub os: ClientOs,
    pub version: String,
    pub capabilities: Vec<String>,
    pub last_seen_s_ago: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct PeersResponse {
    pub peers: Vec<PeerSummary>,
}

// 9.3 — /rpc/message

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct MessageRequest {
    pub agent: String,
    pub prompt: String,
    pub max_tokens: u32,
    pub timeout_s: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct MessageResponse {
    pub agent: String,
    pub response: String,
    pub tokens_used: u32,
    pub elapsed_ms: u64,
    pub model_used: String,
}

// 9.4 — /rpc/task/assign

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct TaskAssignRequest {
    pub agent: String,
    pub prompt: String,
    pub tools: Vec<String>,
    pub max_tokens: u32,
    pub tag_filter: Option<String>,
    pub preferred_peer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct TaskAssignResponse {
    pub task_id: String,
    pub queued_at: String,
    pub assigned_to: String,
    pub estimated_ms: u64,
}

// 9.5 — /rpc/task/status/:id

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct TaskStatusResponse {
    pub task_id: String,
    pub state: TaskState,
    pub progress_pct: u8,
    pub started_at: Option<String>,
    pub running_on: Option<String>,
    /// When `state == Done`, full agent output object (untyped here — the
    /// concrete shape depends on which agent ran and is locked down per-agent
    /// in SPEC-26). ts-rs has no TS impl for `serde_json::Value`, so we
    /// override the emitted TS type with `unknown` (matches the spec §7.3
    /// `data: TData | null` polymorphism for unknown payloads).
    #[ts(type = "unknown")]
    pub result: Option<serde_json::Value>,
    pub checkpoint_id: Option<String>,
}

// 9.6 — /rpc/swarm

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum SwarmAggregateMode {
    Concat,
    First,
    Vote,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct SwarmRequest {
    pub agent: String,
    pub prompt: String,
    pub tag_filter: Option<String>,
    pub aggregate: SwarmAggregateMode,
    pub max_peers: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct SwarmPeerResponse {
    pub peer_name: String,
    pub response: String,
    pub tokens_used: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct SwarmResponse {
    pub swarm_id: String,
    pub responses: Vec<SwarmPeerResponse>,
    pub aggregated: String,
}

// 9.7 — /rpc/dispatch/supervised

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct DispatchSupervisedRequest {
    pub agent: String,
    pub prompt: String,
    pub supervisor_peer: String,
    pub worker_tag: Option<String>,
    pub worktree_path: Option<String>,
    pub max_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct DispatchSupervisedResponse {
    pub task_id: String,
    pub supervisor: String,
    pub worker: String,
    pub worktree_path: Option<String>,
    pub queued_at: String,
}

// 9.8 — DELETE /rpc/task/:id

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct TaskDeleteResponse {
    pub task_id: String,
    pub previous_state: TaskState,
    pub new_state: TaskState,
    pub cancelled_at: String,
}

// 9.9 — /rpc/task/resume

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct TaskResumeRequest {
    pub original_task_id: String,
    pub checkpoint_id: String,
    pub resume_on_peer: String,
    pub force_takeover: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct TaskResumeResponse {
    pub task_id: String,
    pub resumed_from_checkpoint: String,
    pub running_on: String,
    pub resumed_at: String,
    pub estimated_remaining_ms: u64,
}

// 9.10 — /rpc/evolve-handoff

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum EvolveStep {
    Judge,
    Extract,
    Store,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct EvolveHandoffRequest {
    pub evolve_session_id: String,
    pub current_step: EvolveStep,
    /// Base64-encoded opaque evolve-state blob.
    pub state_blob_b64: String,
    pub target_peer: String,
    pub skill_candidate_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct EvolveHandoffResponse {
    pub evolve_session_id: String,
    pub accepted_by: String,
    pub handed_off_at: String,
    pub resume_token: String,
}

// 9.11 — /rpc/skill/sync

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct SkillSyncRequest {
    pub skill_id: String,
    pub name: String,
    pub prompt_template: String,
    pub tags: Vec<String>,
    pub extracted_at: String,
    pub extractor_peer: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct SkillSyncResponse {
    pub skill_id: String,
    pub accepted: bool,
    pub already_existed: bool,
    pub stored_at: String,
}

// 9.12 — /rpc/skill/since/:ts
//
// NOTE: 3 `SkillSummary` types co-exist (different aggregations, module path
// disambiguates). See docs/superpowers/skill-summary-naming.md.
//   • THIS one (`rpc_wire::SkillSummary`) — peer-to-peer sync delta (5 fields).
//   • `skill_wire::SkillSummary`  — dashboard overview card (4 fields).
//   • `skillbank::dto::SkillSummary` — full record (9 fields) for HTTP list.

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct SkillSummary {
    pub skill_id: String,
    pub name: String,
    pub tags: Vec<String>,
    pub extracted_at: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct SkillSinceResponse {
    pub since: String,
    pub skills: Vec<SkillSummary>,
    pub count: u32,
    pub next_page_after_ts: Option<String>,
}

// 9.13 — /rpc/capabilities/refresh

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct CapabilitiesRefreshRequest {
    pub peer_name: String,
    pub capabilities: Vec<String>,
    pub removed: Vec<String>,
    pub changed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct CapabilitiesRefreshResponse {
    pub broadcast_id: String,
    pub received_by_peers: u32,
    pub broadcast_at: String,
}

// 9.14 — /rpc/data/wipe-notify

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum WipeScope {
    All,
    Events,
    Skills,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct WipeNotifyRequest {
    pub peer_name: String,
    pub wiped_at: String,
    pub wipe_scope: WipeScope,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct WipeNotifyResponse {
    pub ack_by_peers: u32,
    pub removed_from_peer_lists_at: String,
}

// 9.15 — /rpc/admin/self-update

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct AdminSelfUpdateRequest {
    pub target_version: String,
    pub download_url: String,
    pub sha256: String,
    pub restart_after: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct AdminSelfUpdateResponse {
    pub peer_name: String,
    pub previous_version: String,
    pub new_version: String,
    pub downloaded_at: String,
    pub will_restart_at: String,
}

// 9.16 — /rpc/admin/shell

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct AdminShellRequest {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub timeout_s: u32,
    pub stdin: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct AdminShellResponse {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub elapsed_ms: u64,
    pub ran_at: String,
}

// 9.17 — POST /api/events

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Food,
    Focus,
    Habit,
    /// Cross-node task dispatch recorded by `spectyn dispatch`, so ability ④
    /// ("do things for you" / cross-machine mesh) is observable via
    /// `spectyn recall --kind dispatch` instead of reading back as plain text.
    Dispatch,
    Text,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct ApiEventRequest {
    pub event_id: String,
    pub kind: EventKind,
    pub captured_at: String,
    pub client_os: ClientOs,
    pub blob_ref: Option<String>,
    pub note: Option<String>,
    pub tags: Vec<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct ApiEventResponse {
    pub event_id: String,
    pub stored_at: String,
    pub duplicated: bool,
}

// ─── Stage 3 helpers — real HMAC-SHA256 + canonical string crypto ────────────
//
// Per docs/superpowers/SPEC-TO-CODE-PLAYBOOK.md the Stage 2 pseudocode bodies
// + `_pseudo` inner helpers were replaced wholesale in this Stage 3 commit:
//   • `sha2::Sha256` for body hash + HMAC inner hash
//   • `hmac::Hmac<Sha256>` for HMAC-SHA256
//   • `hex` for lower-case hex encode/decode
//   • `subtle::ConstantTimeEq` for timing-attack-resistant tag compare
//
// All four crates were already in core/Cargo.toml at the time of the Stage 3
// switch (used elsewhere for E004 encrypted-events + JWT signing), so the
// pseudocode → real swap added zero new transitive dependencies.

/// Build the HMAC canonical-string per SPEC-10 §7.1:
/// `"${method}\n${path}\n${sorted_query}\n${sha256_hex(body)}\n${traceparent || ""}"`
///
/// Returns the exact string that gets fed into `HMAC-SHA256(cluster_secret, _)`.
///
/// 中文: 把 method / path / 排序後 query / body 雜湊 / traceparent 拼成 canonical
/// 字串（標準化字串），後續交給 verify_hmac 簽 / 驗。
pub fn build_canonical_string(
    method: &str,
    path: &str,
    sorted_query: &str,
    body: &[u8],
    traceparent: Option<&str>,
) -> String {
    // Step 1: SHA-256 of body (empty body → fixed constant e3b0c442...
    //         the well-known SHA-256 of the zero-length input).
    let body_hash_hex: String = sha256_hex(body);

    // Step 2: concat per SPEC-10 §7.1 5-part canonical form, separated by
    //         single `\n` (LF). Pre-size the buffer to avoid re-alloc; the
    //         sha256 hex is always 64 chars, traceparent ≤ ~55 chars.
    let mut out = String::with_capacity(
        method.len() + path.len() + sorted_query.len() + 64 + 64,
    );
    out.push_str(method);
    out.push('\n');
    out.push_str(path);
    out.push('\n');
    out.push_str(sorted_query);
    out.push('\n');
    out.push_str(&body_hash_hex);
    out.push('\n');
    // Step 3: traceparent is optional — `None` → empty trailing segment
    //         (still preserves the final `\n` separator before it).
    out.push_str(traceparent.unwrap_or(""));
    out
}

/// Verify an HMAC-SHA256 signature against the canonical-string of a request.
/// Returns `Ok(())` on match, or `Err(RpcError { code: "auth_invalid", .. })`
/// on mismatch. Constant-time comparison via `subtle::ConstantTimeEq`.
///
/// 中文: 用 cluster_secret 對 canonical 算 HMAC-SHA256，與 hex_sig 等時比對。
pub fn verify_hmac(
    cluster_secret: &[u8],
    canonical: &str,
    hex_sig: &str,
) -> Result<(), RpcError> {
    // Step 1: hex-decode the provided signature → 32 raw bytes.
    //         Wrong length / non-hex chars → InvalidPayload-style error.
    let provided: [u8; 32] = decode_hex_32(hex_sig)?;

    // Step 2: compute the expected HMAC-SHA256(cluster_secret, canonical).
    let expected: [u8; 32] = hmac_sha256(cluster_secret, canonical.as_bytes());

    // Step 3: constant-time compare — never short-circuit on first byte
    //         diff (subtle::ConstantTimeEq prevents timing oracle attacks).
    if !constant_time_eq(&provided, &expected) {
        // Step 4: map mismatch to the canonical SPEC-04 `auth_invalid` error.
        return Err(RpcError {
            code: "auth_invalid".to_string(),
            message_zh_tw: "簽章驗證失敗".to_string(),
            message_en: "HMAC signature mismatch".to_string(),
            recovery_action: "Check cluster secret + clock skew".to_string(),
            retryable: false,
            retry_after_s: None,
        });
    }
    Ok(())
}

/// Sign a canonical-string with the cluster secret. Returns hex-encoded
/// HMAC-SHA256 digest carried in the `X-Cluster-Auth` header (SPEC-10 §7.1).
///
/// 中文: 用 cluster_secret 對 canonical 算 HMAC-SHA256，輸出 64 字 hex（十六進制）。
pub fn sign_hmac(cluster_secret: &[u8], canonical: &str) -> String {
    // Step 1: compute HMAC-SHA256 → 32 raw bytes.
    let tag: [u8; 32] = hmac_sha256(cluster_secret, canonical.as_bytes());
    // Step 2: hex-encode (lower-case, 64 chars) — the wire format expected
    //         by the `X-Cluster-Auth` header per SPEC-10 §7.1.
    hex_encode(&tag)
}

// ─── Stage 3 inner crypto helpers (real impl) ────────────────────────────────
//
// These five helpers replace the Stage 2 `_pseudo` panicking stubs. Each is a
// thin one-liner over the relevant RustCrypto crate so the algorithm flow in
// the three public functions above (`build_canonical_string`, `verify_hmac`,
// `sign_hmac`) stays linear and auditable. Keep them `fn` (not pub) — the only
// entry points the rest of the crate should reach for are the three publics.

/// SHA-256 of an arbitrary byte slice, returned as 64-char lower-case hex.
/// Empty input → `"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"`
/// (the canonical RFC 6234 test vector).
fn sha256_hex(body: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(body))
}

/// Hex-decode a string that MUST represent exactly 32 raw bytes (64 hex chars).
/// Wrong length or non-hex characters → `RpcError { code: "auth_invalid", .. }`
/// so the caller can reply with the same SPEC-04 error as a real tag mismatch
/// (don't leak whether failure was malformed-sig vs. wrong-sig to attackers).
fn decode_hex_32(h: &str) -> Result<[u8; 32], RpcError> {
    let bytes = hex::decode(h).map_err(|_| RpcError {
        code: "auth_invalid".to_string(),
        message_zh_tw: "簽章格式錯誤".to_string(),
        message_en: "signature hex decode failed".to_string(),
        recovery_action: "Verify X-Cluster-Auth is 64 lowercase hex chars"
            .to_string(),
        retryable: false,
        retry_after_s: None,
    })?;
    if bytes.len() != 32 {
        return Err(RpcError {
            code: "auth_invalid".to_string(),
            message_zh_tw: "簽章長度錯誤".to_string(),
            message_en: format!(
                "signature must be 32 bytes (64 hex chars), got {}",
                bytes.len()
            ),
            recovery_action: "Re-sign with HMAC-SHA256 (32-byte tag)".to_string(),
            retryable: false,
            retry_after_s: None,
        });
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

/// HMAC-SHA256(key, msg) → 32 raw bytes. `new_from_slice` only ever returns
/// `Err` when the chosen MAC has a fixed key size; HMAC accepts any length,
/// so `.expect` here is genuinely unreachable.
fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key)
        .expect("HMAC accepts keys of any length — unreachable");
    mac.update(msg);
    mac.finalize().into_bytes().into()
}

/// Constant-time equality on two 32-byte tags. Backed by `subtle::ConstantTimeEq`
/// so the running time is independent of where the first differing byte sits —
/// closes a class of timing side-channel attacks against signature verifiers.
fn constant_time_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    use subtle::ConstantTimeEq;
    a.ct_eq(b).into()
}

/// Lower-case hex of a 32-byte HMAC tag (64 chars, no separators) — exactly
/// the wire format the `X-Cluster-Auth` header carries per SPEC-10 §7.1.
fn hex_encode(bytes: &[u8; 32]) -> String {
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_round_trip_smoke() {
        // §7.4 invariant: TS encode → wire → Rust decode → re-encode must be
        // byte-identical. Stage 1 only sanity-checks that the types are
        // serde-compatible; deeper invariant tests come in Stage 2 alongside
        // the canonical-string + HMAC verifier.
        let req = RpcRequest {
            meta: RpcRequestMeta {
                protocol_version: 1,
                trace_id: "4bf92f3577b34da6a3ce929d0e0e4736".into(),
                idempotency_key: None,
                client_os: ClientOs::Mac,
                client_version: "0.6.0".into(),
            },
            data: EmptyRequest {},
        };
        let json = serde_json::to_string(&req).expect("serialize");
        let back: RpcRequest<EmptyRequest> =
            serde_json::from_str(&json).expect("round-trip");
        assert_eq!(back.meta.protocol_version, 1);
    }

    // ─── Stage 3 KAT (known-answer-test) vectors ─────────────────────────
    //
    // These replace the Stage 2 `#[should_panic(expected = "Stage 3")]`
    // markers. Each test pins down a specific contract the public crypto
    // surface promises so future refactors can't silently drift away from
    // the spec.

    /// SPEC-10 §7.1 — canonical string format pin: 5 lines separated by `\n`,
    /// trailing traceparent slot empty when `None`, body hash present even for
    /// short bodies. Body "hello" → SHA-256 prefix `2cf24dba…`.
    #[test]
    fn canonical_string_matches_spec_format() {
        let c = build_canonical_string(
            "POST",
            "/rpc/task/assign",
            "",
            b"hello",
            None,
        );
        let expected_sha = sha256_hex(b"hello");
        let expected =
            format!("POST\n/rpc/task/assign\n\n{}\n", expected_sha);
        assert_eq!(c, expected);
        // SHA-256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        assert!(
            expected_sha.starts_with("2cf24dba"),
            "sha256(\"hello\") expected to start with 2cf24dba, got {}",
            expected_sha
        );
    }

    /// SPEC-10 §7.1 — sign / verify round trip + wrong-key rejection. Proves
    /// the HMAC pipeline (canonical → 32-byte tag → hex → decode → ct compare)
    /// is symmetric and that swapping the key yields `auth_invalid`.
    #[test]
    fn hmac_sign_verify_round_trip() {
        let secret = b"test-cluster-secret-32-bytes-len";
        let canonical = "GET\n/rpc/ping\n\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n";
        let sig = sign_hmac(secret, canonical);
        assert_eq!(sig.len(), 64, "HMAC-SHA256 hex tag must be 64 chars");
        assert!(verify_hmac(secret, canonical, &sig).is_ok());
        let bad = verify_hmac(
            b"wrong-secret-xxxxxxxxxxxxxxxxxxx",
            canonical,
            &sig,
        );
        let err = bad.expect_err("wrong key must fail verification");
        assert_eq!(err.code, "auth_invalid");
    }

    /// RFC 6234 §8.5 test vector — `SHA-256("")` constant. If this digest ever
    /// changes the `sha2` crate is broken or we swapped algorithms by mistake.
    #[test]
    fn sha256_hex_empty_body_matches_rfc6234_vector() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    /// `decode_hex_32` must reject wrong-length input AND non-hex characters,
    /// always mapping to `auth_invalid` so a malformed-sig response is
    /// indistinguishable from a wrong-sig response (no timing/error oracle).
    #[test]
    fn verify_rejects_malformed_signature() {
        let secret = b"any-secret";
        let canonical = "GET\n/\n\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n";
        // Too short
        let too_short = verify_hmac(secret, canonical, "deadbeef");
        assert_eq!(too_short.unwrap_err().code, "auth_invalid");
        // Non-hex
        let non_hex = verify_hmac(
            secret,
            canonical,
            "ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ",
        );
        assert_eq!(non_hex.unwrap_err().code, "auth_invalid");
    }
}
