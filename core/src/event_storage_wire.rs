// SPEC-16 §7 — Event storage wire types (single source of truth for the
// on-device event storage layer: events.sqlite primary table + FTS5 contentless
// virtual table + age-encrypted blob folder).
//
// Stage 4 (full real impl): the filesystem (mkdir / read / write / shred /
// remove_dir_all), structured-query iteration, FTS5 sqlite open / insert /
// query helpers, AND the encryption-key bridge are all live. The bridge
// delegates to `encryption_wire`'s per-process `EventKey` cache
// (`event_key_loaded` + `decrypt_raw_age_blob`); a missing key surfaces as
// `EventStoreError::DecryptionUnavailable` so the iOS app cold-start path
// (keychain not yet unlocked) remains explicit rather than panicking.
//
// 中文: 本檔對應 SPEC-16 §7（資料模型）。儲存層 (storage layer) 把每一筆
// capture（食物 / 焦點 / 習慣 / 圖像 / 音訊）落到 sqlite + 加密 blob 檔；
// Stage 4 完成後所有 helper 都已實作，包含對 `encryption_wire` per-process
// EventKey cache 的橋接（key 未載入時回 `DecryptionUnavailable`，不 panic）。
//
// Relationship to existing `core/src/life_node/event_store.rs`:
//   - That file already ships an early append + decrypt-by-day path (E002).
//   - This file is the wire-shape view (ts-rs exported, camelCase JSON) used by
//     RPC handlers + Tauri commands + the upcoming refactor.
//   - A future cleanup may unify the two via a shared `phantom_types` crate
//     (tracked in SPEC-16 §7.5 既有 pub fn anchor).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ─── EventKind: re-export from rpc_wire (Option A per orchestrator prompt) ────
//
// rpc_wire.rs §9.17 (commit 1d5ff89) already declares EventKind with variants
// { Food, Focus, Habit, Text }. SPEC-16 §8 lists a richer set
// { Food, Focus, Habit, Image, Audio, Coach, Skill, Memory } — note the
// divergence. Per the orchestrator prompt, prefer Option A (re-export) for now
// to avoid double-definition during Stage 1. Stage 2 owners must:
//   (a) decide whether to widen rpc_wire::EventKind to the SPEC-16 set, OR
//   (b) split the two enums (`WireEventKind` for RPC, `StorageEventKind` for
//       events.sqlite) and add a Display/From impl bridge.
// For Stage 1 the re-export keeps cargo check green and signals the design
// debt explicitly. 中文：rpc_wire 已宣告 EventKind 4 變體，SPEC-16 §8 要 8
// 變體；Stage 1 先 re-export 不擋路，Stage 2 統一。
//
// TODO Stage 2: unify EventKind via shared `phantom_types` crate OR widen
// rpc_wire::EventKind to the SPEC-16 §8 superset { Food, Focus, Habit, Image,
// Audio, Coach, Skill, Memory }.
pub use crate::rpc_wire::EventKind;

// ─── §7.1.2-equivalent EventMeta (wire-shape) ─────────────────────────────────

/// Lightweight, queryable metadata pulled out of an `EventRecord` for indexing
/// and listing. Mirrors SPEC-16 §7.1.2 `EventRow` columns that are *not* part
/// of the encrypted `metadata_json` blob — i.e. the bits FTS5 / structured
/// query / app list views see in plaintext.
///
/// 中文: 事件中介資料 (metadata)，給 FTS5 索引 + 結構化查詢 + UI 列表用的
/// 「明文小欄位」。完整內文走 encrypted body (見 EventRecord.encrypted_body_path)。
///
/// Note: This is the *wire-shape* metadata. The on-disk `events` table
/// (SPEC-16 §7.1.1) also carries `metadata_json BLOB` (age-encrypted) — that
/// payload is *not* surfaced here; callers read it via `read_event` and decrypt
/// in-memory only.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/event_storage/")]
#[serde(rename_all = "camelCase")]
pub struct EventMeta {
    /// UUIDv7 (RFC 9562) — string form, 36 chars with dashes. SPEC-16 §8 forces
    /// `Uuid::now_v7()` for natural time-ordered primary key.
    pub event_id: String,
    /// RFC 3339 timestamp in **UTC** (e.g. `"2026-05-25T00:00:00Z"`). SPEC-16 §8
    /// G2 forbids local-time strings, bare seconds, and bare ISO dates — the
    /// single source of truth is UTC milliseconds since the Unix epoch. This
    /// field is kept as a String for wire stability + backward-readable JSON,
    /// but producers MUST derive it from a UTC epoch-ms `i64` via
    /// [`ts_ms_to_rfc3339_utc`] (or normalise an existing string through
    /// [`normalize_to_utc_rfc3339`]). The authoritative sort/range key is the
    /// parsed UTC instant ([`parse_ts_to_utc_ms`]), NOT the raw string bytes —
    /// this is what fixes the 2026-05-22 coach-review 0-events bug: a producer
    /// that wrote a local-offset string (`...+08:00`) used to silently fall out
    /// of a `starts_with("YYYY-MM-DD")` date filter; range matching on the UTC
    /// instant is offset-agnostic.
    pub timestamp: String,
    pub kind: EventKind,
    /// Free-form labels attached at capture time (`"fat_loss"`, `"work"`, ...).
    /// Used by `EventStoreQuery::tag` filter. Plain text; do not put PII here
    /// per SPEC-16 §12.1.
    pub tags: Vec<String>,
}

// ─── §7.1.3-adjacent AnalysisResult (LLM analysis side-car) ───────────────────

/// Output of running an LLM analysis pass over an event (e.g. coach scoring a
/// food entry against `fat_loss` goal). Stored alongside the event as a
/// side-car file `analysis.json` per SPEC-16 §6.1 (file-per-event layout
/// variant requested by orchestrator) — Stage 2 reconciles with the
/// sqlite-only main-table layout.
///
/// 中文: LLM 分析結果。每筆 event 可選擇性掛一份；包含摘要、信心度、目標影響、
/// 建議、成本、延遲、模型 ID、原始回應。所有欄位 plaintext-on-disk-OK 與否
/// delegate 給 SPEC-13 (encryption layer)；本 wire type 只定型別。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/event_storage/")]
#[serde(rename_all = "camelCase")]
pub struct AnalysisResult {
    /// Short bilingual summary (≤ 280 chars typical). Does *not* feed FTS5 by
    /// default — see SPEC-16 §8 `summary` rule (capture pipeline supplies the
    /// FTS5-bound text separately, scrubbed of PII).
    pub summary: String,
    /// Model self-reported confidence in [0.0, 1.0].
    pub confidence: f32,
    /// Free-form note about how this event affects active goals (e.g.
    /// `"+200kcal vs target"`). 中文 + 英文混用 OK.
    pub goal_impact: String,
    /// Imperative next-step suggestion shown in coach review. Empty string if
    /// the model declined to suggest.
    pub suggestion: String,
    /// LLM call cost in USD (provider-reported). 0.0 for local models.
    pub cost_usd: f64,
    /// Wall-clock latency of the analysis call.
    pub latency_ms: u64,
    /// Canonical provider:model identifier (`"groq:llama-3.1-8b-instant"`,
    /// `"anthropic:claude-opus-4.7"`, ...). See SPEC-09 model registry.
    pub model_id: String,
    /// Verbatim provider response body for audit / debugging. May be JSON or
    /// plain text depending on provider. Stage 2 may switch to
    /// `serde_json::Value` once we lock the providers' shape — kept as String
    /// here for wire stability.
    pub raw_response: String,
}

// ─── §7.1.3 EventRecord (wire-shape: meta + body pointer + analysis) ──────────

/// Full event as surfaced to consumers: the queryable `EventMeta` + a pointer
/// to the encrypted body on disk + optional `AnalysisResult`. The encrypted
/// body itself is *not* inlined into the wire shape; callers fetch it lazily
/// via `read_event` (returns this struct with the body still on disk) and
/// decrypt only the bytes they need (SPEC-16 §10 lazy-load rule).
///
/// 中文: 事件紀錄。meta + 加密內文檔案路徑 + 可選分析。Body 不內嵌、必要時才
/// 解密；對齊 SPEC-16 §10「list 預設只拉 metadata，點開才 fetch + decrypt」。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/event_storage/")]
#[serde(rename_all = "camelCase")]
pub struct EventRecord {
    pub meta: EventMeta,
    /// Relative path under the data-dir to the age-encrypted body. Layout per
    /// orchestrator prompt: `events/<uuid>/body.age` (file-per-event variant).
    /// The on-disk SPEC-16 §7.1.1 layout uses `blobs/<sha256>.age` indexed by
    /// `events.blob_ref`; Stage 2 picks one or maps between them.
    pub encrypted_body_path: String,
    /// Optional analysis side-car. `None` = analysis not yet run (or skipped
    /// by user). Coach review pipeline calls `index_fts5` + writes this.
    pub analysis: Option<AnalysisResult>,
}

// ─── §9.2-adjacent EventStoreQuery (CRUD query shape) ─────────────────────────

/// Structured filter for `query_events`. All fields optional; an empty query
/// returns recent events up to `limit`. Mirrors SPEC-16 §9.2 `EventFilter`
/// but with the slimmer field set requested by the orchestrator prompt.
///
/// 中文: 查詢過濾器。日期 / 種類 / 標籤 / limit / offset 全 optional；空查詢
/// 回最近 N 筆。Stage 2 翻成 sqlite WHERE clause。
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/event_storage/")]
#[serde(rename_all = "camelCase")]
pub struct EventStoreQuery {
    /// Local-calendar date in `YYYY-MM-DD` form. Stage 2 converts to UTC
    /// ts_ms range `[00:00 local, 24:00 local)` *in the caller's local TZ*.
    /// This is the field that fixed the 2026-05-22 coach-review 0-events bug
    /// (kept as wall-clock ISO date; conversion centralised one place).
    pub date_iso: Option<String>,
    pub kind: Option<EventKind>,
    /// Match events whose `meta.tags` contains this exact tag string.
    pub tag: Option<String>,
    /// Max rows to return. Stage 2 caps at 1000 per SPEC-16 §7.1.5 hard limit.
    pub limit: Option<u32>,
    /// Skip the first N rows after sorting by `meta.timestamp` ascending.
    pub offset: Option<u32>,
}

// ─── §11-adjacent EventStoreError ─────────────────────────────────────────────

/// Error catalog for the event storage layer. Codes map onto SPEC-04
/// `STORE-XXX` series per SPEC-16 §11. Each variant carries a human-readable
/// message; bilingual user-facing strings live in SPEC-05 i18n layer.
///
/// 中文: 儲存層錯誤分類。對齊 SPEC-04 STORE-001..006；UI 字串走 SPEC-05 i18n。
#[derive(Debug, Clone, Serialize, Deserialize, TS, thiserror::Error)]
#[ts(export, export_to = "../../app/src/lib/generated/event_storage/")]
#[serde(rename_all = "camelCase", tag = "code", content = "message")]
pub enum EventStoreError {
    /// `STORE-001` — sqlite open / data-dir creation failed.
    #[error("open failed: {0}")]
    OpenFailed(String),
    /// `STORE-002` — schema migration failed.
    #[error("migration failed: {0}")]
    MigrationFailed(String),
    /// `STORE-003` — ts_ms ≤ 0, NULL, or non-integer; or `date_iso` malformed.
    #[error("invalid timestamp: {0}")]
    InvalidTimestamp(String),
    /// `STORE-004` — multi-writer attempt (SPEC-16 NG1); second process gets
    /// `SQLITE_BUSY`.
    #[error("storage busy (multi-writer not supported)")]
    Busy,
    /// `STORE-005` — decryption failed (wrong key, tamper, or
    /// `EventKey` not yet loaded — e.g. iOS app cold-start before SPEC-12
    /// keychain unlock).
    #[error("decryption unavailable: {0}")]
    DecryptionUnavailable(String),
    /// `STORE-006` — blob file missing on disk (GC'd or never written).
    /// Callers may render placeholder per SPEC-16 §10.
    #[error("blob missing: {0}")]
    BlobMissing(String),
    /// `STORE-007` — event_id not present in events table.
    #[error("event not found: {0}")]
    NotFound(String),
    /// `STORE-008` — generic I/O error (sqlite file unreadable, fs full, ...).
    #[error("I/O error: {0}")]
    IoError(String),
}

// ─── §7.2 FTS5Index (search index metadata) ───────────────────────────────────

/// Metadata about a single FTS5 (Full-Text Search v5) index entry. Returned
/// by `index_fts5` so callers can confirm the write + log telemetry. The
/// actual searchable text lives inside the sqlite FTS5 virtual table; this
/// struct is the *meta about the meta* — what got indexed, when.
///
/// 中文: FTS5 索引項中介資料。回傳給呼叫端確認入 index 成功 + 多少 term 被索引。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/event_storage/")]
#[serde(rename_all = "camelCase")]
pub struct FTS5Index {
    pub event_id: String,
    /// Count of tokens FTS5 split the input into (approx; `unicode61`
    /// tokenizer). Useful for catching empty-summary bugs (terms_indexed == 0).
    pub terms_indexed_count: u32,
    /// ISO-8601 UTC when the index entry was written.
    pub indexed_at: String,
}

// ─── Stub functions (Stage 2 implements; Stage 1 leaves `unimplemented!()`) ──

/// Write a new event to disk. Stage 2 layout per SPEC-16 §6.1 + orchestrator
/// prompt: `mkdir -p events/<uuid>/`, write `meta.json` (plain or
/// age-encrypted per SPEC-13), `body.age` (always age-encrypted), and
/// `analysis.json` if `analysis` is `Some`. Returns the assigned event_id
/// (UUIDv7) on success.
///
/// 中文: Stage 2 mkdir events/<uuid>/ 後寫 meta.json + body.age (+ analysis.json
/// if 有)；回傳 UUIDv7。
pub fn write_event(
    meta: &EventMeta,
    encrypted_body: &[u8],
    analysis: Option<&AnalysisResult>,
) -> Result<String, EventStoreError> {
    // Step 1: ensure ~/.phantom-mesh/events/<uuid>/ directory exists
    let event_dir = format!("~/.phantom-mesh/events/{}", meta.event_id);
    mkdir_p_pseudo(&event_dir)?;
    // Step 2: serialise EventMeta → meta.json (serde_json) and write
    write_json_pseudo(&format!("{}/meta.json", event_dir), meta)?;
    // Step 3: write age-encrypted body bytes → body.age
    write_bytes_pseudo(&format!("{}/body.age", event_dir), encrypted_body)?;
    // Step 4: optionally write analysis side-car → analysis.json
    if let Some(a) = analysis {
        write_json_pseudo(&format!("{}/analysis.json", event_dir), a)?;
    }
    // Step 5: return the assigned event_id (UUIDv7) to the caller
    Ok(meta.event_id.clone())
}

fn mkdir_p_pseudo(path: &str) -> Result<(), EventStoreError> {
    // Stage 3: `std::fs::create_dir_all` is the canonical `mkdir -p` analogue —
    // it is idempotent (re-calling on an existing dir is `Ok(())`) and creates
    // every missing parent up the chain, matching the SPEC-16 §6.1 layout
    // requirement that `events/<uuid>/` springs into existence on first write.
    let expanded = expand_tilde(path);
    std::fs::create_dir_all(&expanded).map_err(|e| EventStoreError::IoError(e.to_string()))
}

fn write_json_pseudo<T: Serialize>(path: &str, value: &T) -> Result<(), EventStoreError> {
    // Stage 3: serialise to a `Vec<u8>` first (so a serde failure surfaces as
    // a typed error before we touch the disk) then write the buffer in one
    // call. We deliberately do not pretty-print — JSON sidecars are read by
    // programs, not humans, and the compact form keeps blob-store bytes down.
    let expanded = expand_tilde(path);
    let bytes = serde_json::to_vec(value)
        .map_err(|e| EventStoreError::IoError(format!("json serialize: {}", e)))?;
    std::fs::write(&expanded, bytes).map_err(|e| EventStoreError::IoError(e.to_string()))
}

fn write_bytes_pseudo(path: &str, bytes: &[u8]) -> Result<(), EventStoreError> {
    // Stage 3: one-shot truncating write via `std::fs::write` — matches the
    // append-only invariant at the public API level (callers never re-write
    // an existing event; only the kill-switch / correction path goes through
    // `delete_event` first).
    let expanded = expand_tilde(path);
    std::fs::write(&expanded, bytes).map_err(|e| EventStoreError::IoError(e.to_string()))
}

/// Read a full event back from disk. Returns the `EventRecord` (meta + body
/// path + analysis) — body bytes themselves are *not* loaded here (see
/// SPEC-16 §10 lazy-load rule). If the `EventKey` is missing (e.g. iOS app
/// before keychain unlock), returns `EventStoreError::DecryptionUnavailable`.
///
/// 中文: 讀 3 個檔回 EventRecord；body 不解密、key 缺則回 DecryptionUnavailable。
pub fn read_event(event_id: &str) -> Result<EventRecord, EventStoreError> {
    let event_dir = format!("~/.phantom-mesh/events/{}", event_id);
    // Step 1: read meta.json → EventMeta
    let meta: EventMeta = read_json_pseudo(&format!("{}/meta.json", event_dir))?;
    // Step 2: read body.age ciphertext bytes
    let _body_bytes: Vec<u8> = read_bytes_pseudo(&format!("{}/body.age", event_dir))?;
    // Step 3: try decrypt via encryption_wire::decrypt_event; if EventKey is
    // missing (e.g. iOS app cold-start before keychain unlock), bubble up
    // EventStoreError::DecryptionUnavailable instead of panicking.
    if !encryption_key_available_pseudo() {
        return Err(EventStoreError::DecryptionUnavailable(
            "EventKey not loaded".into(),
        ));
    }
    let _plaintext_body: Vec<u8> = decrypt_event_pseudo(&_body_bytes)?;
    // Step 4: optionally read analysis.json side-car (None if absent)
    let analysis: Option<AnalysisResult> =
        try_read_json_pseudo(&format!("{}/analysis.json", event_dir))?;
    // Step 5: assemble EventRecord (body bytes themselves not inlined; only
    // path is surfaced per SPEC-16 §10 lazy-load rule)
    Ok(EventRecord {
        meta,
        encrypted_body_path: format!("events/{}/body.age", event_id),
        analysis,
    })
}

fn read_json_pseudo<T: for<'de> Deserialize<'de>>(path: &str) -> Result<T, EventStoreError> {
    // Stage 3: a missing `meta.json` is the on-disk way of saying "event_id
    // not present" — bubble that up as `NotFound` so callers can render the
    // SPEC-16 §11 STORE-007 user-facing message; everything else is IoError.
    let expanded = expand_tilde(path);
    let bytes = std::fs::read(&expanded).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => EventStoreError::NotFound(path.to_string()),
        _ => EventStoreError::IoError(e.to_string()),
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|e| EventStoreError::IoError(format!("json parse: {}", e)))
}

fn read_bytes_pseudo(path: &str) -> Result<Vec<u8>, EventStoreError> {
    // Stage 3: `body.age` is per SPEC-16 §11 STORE-006 distinct from a missing
    // event — surface as `BlobMissing` so the UI can render the placeholder
    // card rather than a generic I/O failure.
    let expanded = expand_tilde(path);
    std::fs::read(&expanded).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => EventStoreError::BlobMissing(path.to_string()),
        _ => EventStoreError::IoError(e.to_string()),
    })
}

fn try_read_json_pseudo<T: for<'de> Deserialize<'de>>(
    path: &str,
) -> Result<Option<T>, EventStoreError> {
    // Stage 3: distinguish "file absent" (-> Ok(None)) from "file unreadable"
    // (-> Err(IoError)). `analysis.json` is optional — most events ship without
    // one, so missing must NOT propagate as an error per SPEC-16 §10 lazy-load.
    let expanded = expand_tilde(path);
    match std::fs::read(&expanded) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| EventStoreError::IoError(format!("json parse: {}", e))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(EventStoreError::IoError(e.to_string())),
    }
}

fn encryption_key_available_pseudo() -> bool {
    // Stage 4: the per-process `EventKey` cache lives in `encryption_wire`
    // (`install_event_key_from_seed` / `lookup_or_derive_event_key`).
    //
    // D30: use `lookup_or_derive_event_key`, NOT the passive `event_key_loaded`
    // cache-check. In a CLI one-shot (`phantom habit list`) nothing installs the
    // cache at startup, so the passive check returned false and reads
    // short-circuited to "decryption unavailable" even though
    // `~/.phantom-mesh/identity.key` existed. `lookup_or_derive_event_key`
    // derives-on-miss from that file (and the long-running daemon still hits the
    // already-installed cache on the fast path), so a present identity always
    // resolves here regardless of entry point.
    crate::encryption_wire::lookup_or_derive_event_key().is_some()
}

fn decrypt_event_pseudo(ciphertext: &[u8]) -> Result<Vec<u8>, EventStoreError> {
    // Stage 4: delegate to `encryption_wire::decrypt_raw_age_blob`, which
    // pulls the cached `EventKey`, projects it to an `X25519Identity`, and
    // runs the age v1 recipient-mode decrypt pipeline. Any failure (wrong
    // key / tampered blob / no key loaded) collapses to a single error per
    // SPEC-13 §12.1 STRIDE-Tampering oracle-leak prevention; we map all of
    // them onto `STORE-005 DecryptionUnavailable` for the storage caller.
    crate::encryption_wire::decrypt_raw_age_blob(ciphertext)
        .map_err(|e| EventStoreError::DecryptionUnavailable(format!("{:?}", e)))
}

/// Structured query. Stage 2: `read_dir(events/)`, filter by `date_iso` /
/// `kind` / `tag`, sort by `meta.timestamp` ascending, apply `offset` then
/// `limit` (default 100, cap 1000). Returns decrypted `EventRecord` rows.
///
/// 中文: read_dir + filter；按 timestamp 升冪；套用 offset + limit。
pub fn query_events(query: &EventStoreQuery) -> Result<Vec<EventRecord>, EventStoreError> {
    // Step 1: read_dir of the events root → vector of <uuid> sub-directories
    let entries: Vec<String> = read_dir_pseudo("~/.phantom-mesh/events/")?;
    // Step 2: for each entry, read meta and filter by date_iso starts_with
    // (local-calendar prefix match) / kind / tag predicates from query
    // SPEC-16 §8 G2: precompute the UTC day-bounds for `date_iso` ONCE (not per
    // row) so the date filter compares on the parsed UTC instant, never on
    // fragile string bytes. This is the 0-events bug fix: a stored local-offset
    // string used to fall out of `starts_with("YYYY-MM-DD")` silently.
    let date_bounds: Option<(i64, i64)> = match &query.date_iso {
        Some(d) => Some(utc_day_bounds_ms(d)?),
        None => None,
    };
    let mut matched: Vec<EventRecord> = Vec::new();
    for event_id in entries {
        // D30: the `events/` directory can hold events written by OTHER stores
        // sharing the same root — notably the life_node note/focus path, which
        // age-encrypts its OWN `meta.json` when an identity.key is present. Its
        // bytes are not this reader's JSON schema, so a hard `?` here made the
        // WHOLE query fail ("json parse: expected value at line 1 column 1")
        // after any such capture. A dir whose meta can't be read/parsed is, by
        // definition, not one of our events — SKIP it instead of aborting. (Our
        // own events still parse; the kind/tag filters below exclude the rest.)
        let meta: EventMeta = match read_json_pseudo(&format!(
            "~/.phantom-mesh/events/{}/meta.json",
            event_id
        )) {
            Ok(m) => m,
            Err(e) => {
                // Expected for foreign dirs (life_node's encrypted meta.json);
                // log at debug so a genuinely-corrupt OWN event is still
                // diagnosable rather than vanishing without a trace.
                tracing::debug!(event_id = %event_id, error = %e, "query_events: skipping unreadable event meta");
                continue;
            }
        };
        if let Some((start_ms, end_ms)) = date_bounds {
            // Parse the stored timestamp to its UTC instant and keep only rows
            // inside `[00:00 UTC, 24:00 UTC)` for the requested calendar day.
            let ts_ms = parse_ts_to_utc_ms(&meta.timestamp)?;
            if ts_ms < start_ms || ts_ms >= end_ms {
                continue;
            }
        }
        if let Some(kind) = &query.kind {
            if std::mem::discriminant(&meta.kind) != std::mem::discriminant(kind) {
                continue;
            }
        }
        if let Some(tag) = &query.tag {
            if !meta.tags.iter().any(|t| t == tag) {
                continue;
            }
        }
        // Same defensiveness for the full read (body.age decrypt etc.): a single
        // unreadable matching event must not sink the whole query. This one IS
        // alarming (meta already parsed as ours + matched the filter), so log at
        // a higher level than the foreign-dir meta skip above.
        match read_event(&event_id) {
            Ok(rec) => matched.push(rec),
            Err(e) => {
                tracing::warn!(event_id = %event_id, error = %e, "query_events: matched event failed to read — skipping (possible corruption)");
                continue;
            }
        }
    }
    // Step 3: sort by the parsed UTC instant ascending (SPEC-16 §8 G2). String
    // compare is wrong when producers wrote mixed offset forms (`Z` vs
    // `+08:00`) — only the parsed UTC ms ordering is authoritative. Unparseable
    // strings sort to the front (i64::MIN) but cannot occur post-filter since
    // any `date_iso` query already validated them via `parse_ts_to_utc_ms`.
    matched.sort_by_key(|r| parse_ts_to_utc_ms(&r.meta.timestamp).unwrap_or(i64::MIN));
    // Step 4: apply offset then take limit slice (default 100, cap 1000)
    let offset = query.offset.unwrap_or(0) as usize;
    let limit = query.limit.unwrap_or(100).min(1000) as usize;
    let sliced: Vec<EventRecord> = matched.into_iter().skip(offset).take(limit).collect();
    Ok(sliced)
}

/// Convert a `YYYY-MM-DD` calendar date into the half-open UTC millisecond
/// range `[00:00:00 UTC, next-day 00:00:00 UTC)`. SPEC-16 §8 G2 / JS2: the
/// coach.agent yesterday-review query must pull every row whose UTC instant
/// lands inside the requested UTC day — independent of the producer's local
/// timezone. A malformed date surfaces `InvalidTimestamp` (STORE-003) instead
/// of silently matching nothing.
fn utc_day_bounds_ms(date_iso: &str) -> Result<(i64, i64), EventStoreError> {
    use chrono::TimeZone as _;
    let date = chrono::NaiveDate::parse_from_str(date_iso, "%Y-%m-%d")
        .map_err(|e| EventStoreError::InvalidTimestamp(format!("date_iso {}: {}", date_iso, e)))?;
    let start = chrono::Utc
        .from_utc_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight is valid"))
        .timestamp_millis();
    // +1 day for the exclusive upper bound; 86_400_000 ms = 24 h (no DST in UTC).
    let end = start + 86_400_000;
    Ok((start, end))
}

fn read_dir_pseudo(path: &str) -> Result<Vec<String>, EventStoreError> {
    // Stage 3: every immediate sub-directory of `events/` is an event_id (the
    // file-per-event SPEC-16 §6.1 layout). We deliberately filter to
    // directories — stray top-level files (e.g. `.DS_Store` on macOS, lock
    // files, ...) must not show up in the query result set.
    let expanded = expand_tilde(path);
    let iter = match std::fs::read_dir(&expanded) {
        Ok(it) => it,
        // Empty data-dir on first launch → caller sees zero events, not error.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(EventStoreError::IoError(e.to_string())),
    };
    let mut out: Vec<String> = Vec::new();
    for entry_res in iter {
        let entry = entry_res.map_err(|e| EventStoreError::IoError(e.to_string()))?;
        let file_type = entry
            .file_type()
            .map_err(|e| EventStoreError::IoError(e.to_string()))?;
        if !file_type.is_dir() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            out.push(name.to_string());
        }
    }
    Ok(out)
}

/// Delete an event. Stage 2: shred (3-pass overwrite) the ciphertext file
/// then `remove_dir_all` the event directory. Note: SPEC-16 §3.1 G8 declares
/// the storage layer append-only at the *public API* level — this function
/// exists for the kill-switch path (SPEC-16 §16 `phantom data delete --all`)
/// and for `correction_of` cleanup, *not* for everyday update.
///
/// 中文: 3-pass 覆寫密文後移除目錄；只給 kill switch + correction 路徑用，
/// 一般 append-only。
pub fn delete_event(event_id: &str) -> Result<(), EventStoreError> {
    let event_dir = format!("~/.phantom-mesh/events/{}", event_id);
    // Step 1: resolve the body.age path (the only file carrying ciphertext
    // we have to scrub; meta.json + analysis.json are removed wholesale in
    // step 3 via remove_dir_all)
    let body_path = format!("{}/body.age", event_dir);
    // Step 2: 3-pass overwrite with zeros to defeat naive forensic recovery
    // per SPEC-16 §6.4 (best-effort on flash media; full guarantee requires
    // disk-level secure erase which the OS owns)
    shred_pseudo(&body_path, 3)?;
    // Step 3: remove_dir_all on the event directory (meta + body + analysis)
    remove_dir_all_pseudo(&event_dir)?;
    Ok(())
}

fn shred_pseudo(path: &str, passes: u8) -> Result<(), EventStoreError> {
    // Stage 3: best-effort N-pass overwrite per SPEC-16 §6.4. We zero the
    // existing byte-range in place (not truncate-then-write — truncate may
    // free the underlying inode on COW filesystems before we get the chance
    // to overwrite the original blocks), then `sync_data` each pass to push
    // the dirty pages out of the page cache. Flash media gives no guarantee
    // here (FTL wear-levelling rewrites to fresh cells) — SPEC-16 §6.4
    // documents that disk-level secure-erase is the OS's responsibility.
    use std::io::{Seek, SeekFrom, Write};
    let expanded = expand_tilde(path);
    let meta = match std::fs::metadata(&expanded) {
        Ok(m) => m,
        // Already gone — treat as success (delete_event is idempotent).
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(EventStoreError::IoError(e.to_string())),
    };
    let len = meta.len() as usize;
    let zeros = vec![0u8; len];
    for _ in 0..passes {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(&expanded)
            .map_err(|e| EventStoreError::IoError(e.to_string()))?;
        f.seek(SeekFrom::Start(0))
            .map_err(|e| EventStoreError::IoError(e.to_string()))?;
        f.write_all(&zeros)
            .map_err(|e| EventStoreError::IoError(e.to_string()))?;
        f.sync_data()
            .map_err(|e| EventStoreError::IoError(e.to_string()))?;
    }
    Ok(())
}

fn remove_dir_all_pseudo(path: &str) -> Result<(), EventStoreError> {
    // Stage 3: `std::fs::remove_dir_all` mirrors `rm -rf`. We swallow a
    // `NotFound` so `delete_event` stays idempotent (caller can retry safely
    // after a partial failure).
    let expanded = expand_tilde(path);
    match std::fs::remove_dir_all(&expanded) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(EventStoreError::IoError(e.to_string())),
    }
}

/// Insert a row into the FTS5 (Full-Text Search v5) index. Idempotent: a
/// repeat call with the same `event_id` updates the existing entry rather
/// than duplicating. `plaintext_summary` must already be PII-scrubbed by the
/// caller per SPEC-16 §8 + §12.1.
///
/// 中文: sqlite FTS5 INSERT，idempotent；summary 必須由 caller 脫敏。
pub fn index_fts5(
    event_id: &str,
    plaintext_summary: &str,
) -> Result<FTS5Index, EventStoreError> {
    // Step 1: open the sqlite handle for ~/.phantom-mesh/events.sqlite
    let conn = sqlite_open_pseudo("~/.phantom-mesh/events.sqlite")?;
    // Step 2: prepare idempotent upsert into the FTS5 contentless virtual table
    let sql = "INSERT OR REPLACE INTO fts5_events(event_id, content) VALUES(?, ?)";
    // Step 3: execute with (event_id, plaintext_summary) bound; assemble the
    // FTS5Index meta-about-meta receipt for the caller
    sqlite_execute_pseudo(&conn, sql, &[event_id, plaintext_summary])?;
    Ok(FTS5Index {
        event_id: event_id.to_string(),
        terms_indexed_count: count_tokens_pseudo(plaintext_summary)?,
        indexed_at: now_iso_pseudo(),
    })
}

fn sqlite_open_pseudo(path: &str) -> Result<SqliteHandle, EventStoreError> {
    // Stage 3: open (or create) the sqlite file at the resolved path. The
    // FTS5 virtual table is provisioned lazily here so callers don't have
    // to run a separate migration step — `IF NOT EXISTS` keeps the open
    // call idempotent. `contentless` (`content=''`) avoids storing a second
    // copy of the plaintext summary; the canonical text lives in the
    // encrypted `meta`/`body` files, FTS5 only owns the inverted index.
    let expanded = expand_tilde(path);
    if let Some(parent) = std::path::Path::new(&expanded).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| EventStoreError::OpenFailed(e.to_string()))?;
        }
    }
    // CUJ-04 DB-001: sqlite corruption recovery.
    //
    // A previous phantom run that died mid-fsync, a disk-level error, or
    // an OS-killed VACUUM can leave the sqlite file with a torn-page header
    // or a broken btree. Without this guard, the next `phantom habit ...`
    // would panic on the first SELECT and the user is locked out of every
    // capture command until they manually `rm` the file. Behaviour now:
    //
    //   1. Open the file. If `Connection::open` itself fails (file
    //      exists but isn't sqlite at all), salvage + reopen.
    //   2. Run `PRAGMA integrity_check`. sqlite returns the single row
    //      `"ok"` for a healthy db. Anything else (including multiple
    //      rows describing the damage) is treated as corruption.
    //   3. On corruption: close the broken handle, rename the file to
    //      `<path>.corrupt-<unix-ts>` so the user can forensically pull
    //      data from it later (e.g. `sqlite3 .corrupt-... .dump`), and
    //      reopen a fresh empty db. The user's events written before
    //      corruption are NOT lost — they live in the encrypted
    //      `events/<uuid>/` directory (events.sqlite is just the FTS5
    //      index + metadata cache); a `phantom data export` will
    //      rebuild the index from the canonical files.
    //
    // The CREATE VIRTUAL TABLE below stays idempotent so the post-
    // recovery fresh db gets the same FTS5 schema as a clean install.
    let conn = match rusqlite::Connection::open(&expanded) {
        Ok(c) => c,
        Err(e) => {
            // File exists but isn't a valid sqlite (e.g. truncated to 0,
            // garbage). Salvage + reopen.
            let salvaged = rotate_corrupt_sqlite(expanded.to_str().unwrap_or_default(), &e.to_string());
            if !salvaged {
                return Err(EventStoreError::OpenFailed(e.to_string()));
            }
            rusqlite::Connection::open(&expanded)
                .map_err(|e2| EventStoreError::OpenFailed(e2.to_string()))?
        }
    };
    // PRAGMA integrity_check on an empty fresh db also returns "ok", so this
    // gate is safe to run every open (no first-run skip needed).
    let integrity: String = match conn.query_row("PRAGMA integrity_check", [], |row| row.get(0)) {
        Ok(s) => s,
        Err(e) => {
            // Even the integrity probe itself failed (e.g. severe corruption
            // that prevents any query). Rotate + reopen + retry.
            drop(conn);
            if rotate_corrupt_sqlite(expanded.to_str().unwrap_or_default(), &e.to_string()) {
                let conn2 = rusqlite::Connection::open(&expanded)
                    .map_err(|e2| EventStoreError::OpenFailed(e2.to_string()))?;
                conn2
                    .execute_batch(
                        "CREATE VIRTUAL TABLE IF NOT EXISTS fts5_events \
                         USING fts5(event_id UNINDEXED, content, tokenize='unicode61');",
                    )
                    .map_err(|e2| EventStoreError::MigrationFailed(e2.to_string()))?;
                return Ok(SqliteHandle { conn: conn2 });
            }
            return Err(EventStoreError::OpenFailed(format!(
                "PRAGMA integrity_check failed and rotation failed: {}",
                e
            )));
        }
    };
    let conn = if integrity != "ok" {
        // Corruption detected. Rotate the bad file aside and reopen fresh.
        drop(conn);
        if rotate_corrupt_sqlite(expanded.to_str().unwrap_or_default(), &integrity) {
            rusqlite::Connection::open(&expanded)
                .map_err(|e| EventStoreError::OpenFailed(e.to_string()))?
        } else {
            return Err(EventStoreError::OpenFailed(format!(
                "sqlite integrity_check failed ({}) and rotation failed",
                integrity
            )));
        }
    } else {
        conn
    };
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS fts5_events \
         USING fts5(event_id UNINDEXED, content, tokenize='unicode61');",
    )
    .map_err(|e| EventStoreError::MigrationFailed(e.to_string()))?;
    Ok(SqliteHandle { conn })
}

/// Move a corrupt sqlite file aside to `<path>.corrupt-<unix-ts>` so a
/// fresh one can take its place. Returns `true` on successful rename (the
/// caller should now be safe to `Connection::open` the original path and
/// get an empty db), `false` if the rename itself fails (in which case
/// the caller MUST NOT proceed — we'd corrupt the fresh db too). Emits a
/// stderr line (suppressed under the TUI flag) so the user sees the
/// rotation even when it happens transparently during `phantom habit`.
fn rotate_corrupt_sqlite(path: &str, reason: &str) -> bool {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = format!("{}.corrupt-{}", path, ts);
    match std::fs::rename(path, &backup) {
        Ok(()) => {
            if !crate::diag::is_tui_active() {
                eprintln!(
                    "phantom: sqlite at {} was corrupt ({}) — moved to {} and starting fresh. \
                     Underlying events in events/<uuid>/ are unaffected; \
                     a future `phantom data export` will rebuild the index.",
                    path, reason, backup
                );
            }
            crate::diag::record(
                "sqlite_corrupt_rotated",
                format!("{} → {} ({})", path, backup, reason),
            );
            true
        }
        Err(e) => {
            if !crate::diag::is_tui_active() {
                eprintln!(
                    "phantom: sqlite at {} is corrupt ({}) but rotate failed: {}",
                    path, reason, e
                );
            }
            crate::diag::record(
                "sqlite_corrupt_rotate_failed",
                format!("{}: {}", path, e),
            );
            false
        }
    }
}

fn sqlite_execute_pseudo(
    handle: &SqliteHandle,
    sql: &str,
    params: &[&str],
) -> Result<(), EventStoreError> {
    // Stage 3: prepared-statement execute with positional `?` bindings. We
    // box the `&str` slice into a `rusqlite::params_from_iter` view so the
    // public surface (`&[&str]`) stays unchanged for SPEC-16 callers.
    handle
        .conn
        .execute(sql, rusqlite::params_from_iter(params.iter()))
        .map(|_| ())
        .map_err(|e| EventStoreError::IoError(e.to_string()))
}

fn count_tokens_pseudo(text: &str) -> Result<u32, EventStoreError> {
    // Stage 3: a cheap whitespace + punctuation token estimate that mirrors
    // FTS5's `unicode61` tokenizer closely enough for the receipt count.
    // We deliberately do NOT round-trip through sqlite (would require a
    // second connection + `fts5_tokenize`) — the count is telemetry only,
    // empty-summary detection only needs `> 0`.
    let n = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .count();
    Ok(n as u32)
}

fn now_iso_pseudo() -> String {
    // Stage 3 / SPEC-16 §8 G2: canonical UTC RFC 3339 with the `Z` zulu suffix
    // (NOT a local-time string, NOT a `+08:00` offset). The TS side can feed
    // the string straight into `new Date(...)`.
    ts_ms_to_rfc3339_utc(now_ts_ms())
}

// ─── SPEC-16 §8 G2 canonical UTC time helpers ─────────────────────────────────
//
// 中文: SPEC-16 §8 G2 鎖死「ts_ms = UTC 毫秒 INTEGER」為單一真實來源
// （single source of truth）。本區三個 helper 是「正確時間戳」的唯一入口：
// 產生（now_ts_ms）、格式化成 wire 字串（ts_ms_to_rfc3339_utc）、把任意
// RFC 3339 字串解析回 UTC 毫秒（parse_ts_to_utc_ms）。禁止 local time、禁止秒、
// 禁止裸 ISO 日期。

/// Current time as Unix-epoch **UTC milliseconds** (`i64`). This is the SPEC-16
/// §8 G2 single source of truth for event time — equivalent to TS `Date.now()`.
/// All `EventMeta.timestamp` strings MUST be derived from a value of this shape
/// via [`ts_ms_to_rfc3339_utc`]; never format a local-time clock directly.
pub fn now_ts_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Format UTC epoch-ms into a canonical RFC 3339 string with the `Z` (zulu)
/// suffix. `1764547200000` → `"2025-12-01T00:00:00Z"`. A negative / out-of-range
/// ms value (cannot happen for real captures — SPEC-16 §8 `CHECK (ts_ms > 0)`)
/// falls back to the Unix epoch so callers never panic.
pub fn ts_ms_to_rfc3339_utc(ts_ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts_ms)
        .unwrap_or_else(|| chrono::DateTime::<chrono::Utc>::from_timestamp_millis(0).unwrap())
        // `Rfc3339` of a `DateTime<Utc>` emits the `Z` suffix (not `+00:00`).
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Parse any RFC 3339 timestamp string (UTC `Z`, or a `+08:00` local-offset
/// string an older producer may have written) back into the canonical UTC
/// epoch-ms `i64`. Returns `EventStoreError::InvalidTimestamp` for anything that
/// is not parseable RFC 3339 — this is the chokepoint that turns the old
/// silent-skip (a malformed/local string vanishing from a string-prefix date
/// filter) into an explicit, surfaced error.
pub fn parse_ts_to_utc_ms(timestamp: &str) -> Result<i64, EventStoreError> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|dt| dt.with_timezone(&chrono::Utc).timestamp_millis())
        .map_err(|e| EventStoreError::InvalidTimestamp(format!("{}: {}", timestamp, e)))
}

/// Re-emit any parseable RFC 3339 string as the canonical UTC `Z` form. A
/// local-offset string (`2026-05-25T02:00:00+08:00`) becomes its UTC
/// equivalent (`2026-05-24T18:00:00Z`). Used by producers to scrub a
/// client-supplied timestamp into the SPEC-16 §8 G2 canonical shape before it
/// reaches `write_event`.
pub fn normalize_to_utc_rfc3339(timestamp: &str) -> Result<String, EventStoreError> {
    parse_ts_to_utc_ms(timestamp).map(ts_ms_to_rfc3339_utc)
}

// ─── LOCAL-day bucketing helpers (T-STOR-01, production read path) ───────────
// `query_events` above buckets by UTC day (utc_day_bounds_ms). The production
// per-day review/stats/recall instead bucket by the user's LOCAL calendar day —
// "today's food" must match the user's wall clock, not UTC — so they use these.
// (capture_habit streaks separately use parse_ts_to_utc_ms for absolute-instant
// math.) ts_epoch_ms is a mixed-offset-safe chronological sort key.

/// Calendar date (`YYYY-MM-DD`) of `timestamp` in a fixed offset — the
/// deterministic, machine-tz-independent (hence unit-testable) core of
/// [`ts_local_date`]. Falls back to the raw first 10 chars if unparseable.
pub fn ts_date_in_offset(timestamp: &str, offset: chrono::FixedOffset) -> String {
    match chrono::DateTime::parse_from_rfc3339(timestamp) {
        Ok(dt) => dt.with_timezone(&offset).format("%Y-%m-%d").to_string(),
        Err(_) => timestamp.chars().take(10).collect(),
    }
}

/// The user's LOCAL calendar date for an event timestamp. `with_timezone(&Local)`
/// resolves the offset at the event's own instant (DST-correct).
pub fn ts_local_date(timestamp: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(timestamp) {
        Ok(dt) => dt
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d")
            .to_string(),
        Err(_) => timestamp.chars().take(10).collect(),
    }
}

/// True when `timestamp` falls on the given LOCAL calendar date (`YYYY-MM-DD`) —
/// the tz-correct replacement for `meta.timestamp.starts_with(date_iso)`.
pub fn ts_on_local_date(timestamp: &str, local_date: &str) -> bool {
    ts_local_date(timestamp) == local_date
}

/// Absolute-instant sort key (epoch ms). Lexical compare of RFC3339 strings is
/// only chronological at a single offset; the store mixes legacy local-offset
/// and UTC timestamps, so chronological sorts compare the parsed instant.
/// Unparseable values sort first (`i64::MIN`), deterministically.
pub fn ts_epoch_ms(timestamp: &str) -> i64 {
    match chrono::DateTime::parse_from_rfc3339(timestamp) {
        Ok(dt) => dt.timestamp_millis(),
        Err(_) => i64::MIN,
    }
}

/// Opaque sqlite connection handle. Wraps `rusqlite::Connection` so the rest
/// of the module stays decoupled from rusqlite's surface.
pub struct SqliteHandle {
    conn: rusqlite::Connection,
}

/// FTS5 MATCH query. Returns a Vec of event_id strings ordered by BM25 score.
/// Caller follows up with `read_event` per id to materialise full records.
/// `limit` is capped at 1000 by Stage 2.
///
/// 中文: FTS5 MATCH 查詢回 event_id list，呼叫端再 read_event 取 record。
pub fn search_fts5(query: &str, limit: usize) -> Result<Vec<String>, EventStoreError> {
    // Step 1: prepare BM25-ranked MATCH query against the FTS5 virtual table.
    // Cap limit at 1000 per SPEC-16 §7.1.5 to keep memory bounded.
    let conn = sqlite_open_pseudo("~/.phantom-mesh/events.sqlite")?;
    let capped = limit.min(1000);
    let sql = "SELECT event_id FROM fts5_events WHERE fts5_events MATCH ? ORDER BY rank LIMIT ?";
    // Step 2: execute via the typed query helper, binding the user query and
    // the capped limit
    let rows: Vec<String> =
        sqlite_query_pseudo(&conn, sql, &[query, &capped.to_string()])?;
    // Step 3: collect event_ids in BM25 order; caller follows up with
    // read_event(id) to materialise the full EventRecord per id
    Ok(rows)
}

fn sqlite_query_pseudo(
    handle: &SqliteHandle,
    sql: &str,
    params: &[&str],
) -> Result<Vec<String>, EventStoreError> {
    // Stage 3: prepared-statement query — every row is expected to project a
    // single `TEXT` column (the `event_id`). FTS5 MATCH callers bind `(query,
    // limit_str)`; the second arg is a stringified u32 which sqlite coerces.
    let mut stmt = handle
        .conn
        .prepare(sql)
        .map_err(|e| EventStoreError::IoError(e.to_string()))?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            row.get::<_, String>(0)
        })
        .map_err(|e| EventStoreError::IoError(e.to_string()))?;
    let mut out: Vec<String> = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| EventStoreError::IoError(e.to_string()))?);
    }
    Ok(out)
}

// ─── Stage 3 path helper (tilde expansion) ────────────────────────────────────

/// Expand a leading `~/` to the current user's home directory using the
/// `dirs` crate (already a top-level dep). Non-tilde paths pass through
/// unchanged. This keeps the rest of the module free of `~/` literals
/// embedded in `format!` strings.
fn expand_tilde(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    std::path::PathBuf::from(path)
}

// ─── Smoke tests (Stage 1 sanity only; deeper invariants in Stage 2) ─────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_record_round_trip_smoke() {
        let r = EventRecord {
            meta: EventMeta {
                event_id: "uuid".into(),
                timestamp: "2026-05-25T00:00:00Z".into(),
                kind: EventKind::Food,
                tags: vec!["fat_loss".into()],
            },
            encrypted_body_path: "events/uuid/body.age".into(),
            analysis: None,
        };
        let j = serde_json::to_string(&r).unwrap();
        let back: EventRecord = serde_json::from_str(&j).unwrap();
        assert_eq!(r.meta.event_id, back.meta.event_id);
        assert_eq!(r.meta.tags, back.meta.tags);
        assert_eq!(r.encrypted_body_path, back.encrypted_body_path);
    }

    #[test]
    fn event_store_query_default_is_empty() {
        let q = EventStoreQuery::default();
        assert!(q.date_iso.is_none());
        assert!(q.kind.is_none());
        assert!(q.tag.is_none());
        assert!(q.limit.is_none());
        assert!(q.offset.is_none());
    }

    #[test]
    fn analysis_result_round_trip_smoke() {
        let a = AnalysisResult {
            summary: "lunch was on plan".into(),
            confidence: 0.82,
            goal_impact: "+100kcal vs target".into(),
            suggestion: "skip dessert".into(),
            cost_usd: 0.0012,
            latency_ms: 850,
            model_id: "groq:llama-3.1-8b-instant".into(),
            raw_response: "{}".into(),
        };
        let j = serde_json::to_string(&a).unwrap();
        let back: AnalysisResult = serde_json::from_str(&j).unwrap();
        assert_eq!(a.model_id, back.model_id);
        assert!((a.confidence - back.confidence).abs() < f32::EPSILON);
    }

    #[test]
    fn event_store_error_serialises_with_code_tag() {
        let e = EventStoreError::BlobMissing("events/xyz/body.age".into());
        let j = serde_json::to_string(&e).unwrap();
        // tag=code, content=message → `{"code":"blobMissing","message":"..."}`
        assert!(j.contains("\"code\""));
        assert!(j.contains("blobMissing"));
    }

    // ── Stage 3 + Stage 4 KAT (known-answer-test) vectors ─────────────────
    //
    // `write_event` runs real `std::fs` underneath (Stage 3) and
    // `read_event` now drives the `encryption_wire` per-process EventKey
    // cache (Stage 4) — there is no remaining `#[should_panic]` marker.
    // `read_event_stage4_bridge_round_trip` exercises both the cache-loaded
    // happy path and the cache-empty `DecryptionUnavailable` branch.

    /// SPEC-16 §6.1 — write_event must create the event directory and drop
    /// `meta.json` + `body.age` (plus optional `analysis.json`). We point the
    /// data root at a `tempfile::TempDir` via `$HOME` override so the test
    /// never touches the real `~/.phantom-mesh/`.
    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn write_event_round_trip_creates_expected_files() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        // `expand_tilde` reads `dirs::home_dir()` which honours `$HOME` on
        // unix — point HOME at the tempdir so writes land inside it.
        let prev = std::env::var_os("HOME");
        // Tests share a process; restore HOME at the end via a RAII guard.
        struct HomeGuard(Option<std::ffi::OsString>);
        impl Drop for HomeGuard {
            fn drop(&mut self) {
                match &self.0 {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
            }
        }
        std::env::set_var("HOME", tmp.path());
        let _guard = HomeGuard(prev);

        let meta = EventMeta {
            event_id: "kat-uuid-001".into(),
            timestamp: "2026-05-25T00:00:00Z".into(),
            kind: EventKind::Food,
            tags: vec!["fat_loss".into()],
        };
        let returned = write_event(&meta, b"ciphertext-bytes", None).expect("write_event");
        assert_eq!(returned, "kat-uuid-001");
        let event_dir = tmp.path().join(".phantom-mesh/events/kat-uuid-001");
        assert!(event_dir.join("meta.json").exists());
        assert!(event_dir.join("body.age").exists());
        assert!(!event_dir.join("analysis.json").exists());
        let body_bytes = std::fs::read(event_dir.join("body.age")).expect("read body");
        assert_eq!(body_bytes, b"ciphertext-bytes");
    }

    /// SPEC-16 §10 — `read_event` routes through the Stage 4 encryption
    /// bridge. Two paths are pinned here:
    ///
    ///   1. Cache empty → `DecryptionUnavailable` (iOS cold-start before
    ///      keychain unlock). Must NOT panic, must NOT return an arbitrary
    ///      I/O error — STORE-005 is the only acceptable surface.
    ///   2. Cache populated with a matching key → round-trip succeeds and
    ///      `read_event` returns an `EventRecord` shaped per §10 lazy-load
    ///      (body bytes not inlined; only path surfaced).
    ///
    /// We deliberately exercise both branches in one test so the cache
    /// install/clear pair stays balanced even on early-return failure.
    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn read_event_stage4_bridge_round_trip() {
        use crate::encryption_wire;
        let tmp = tempfile::TempDir::new().expect("tempdir");
        struct HomeGuard(Option<std::ffi::OsString>);
        impl Drop for HomeGuard {
            fn drop(&mut self) {
                match &self.0 {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
            }
        }
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", tmp.path());
        let _guard = HomeGuard(prev);

        // Seed disk: meta + a real age-encrypted body the cached key can decrypt.
        let seed = [0x33u8; 32];
        let key = encryption_wire::install_event_key_from_seed(&seed).expect("install");
        let identity = encryption_wire::event_key_to_age_identity(&key).expect("identity");
        let recipient = encryption_wire::derive_recipient_from_identity(&identity);
        let plaintext = b"stage4 bridge sample body";
        let envelope = encryption_wire::encrypt_event(plaintext, &recipient).expect("encrypt");
        // `body.age` carries the raw age v1 binary blob, not the base64 wrapper —
        // decode the envelope's `ciphertext_b64` back to bytes for on-disk shape.
        use base64::Engine as _;
        let raw_blob = base64::engine::general_purpose::STANDARD
            .decode(&envelope.ciphertext_b64)
            .expect("base64 decode");

        let meta = EventMeta {
            event_id: "stage4-bridge".into(),
            timestamp: "2026-05-26T00:00:00Z".into(),
            kind: EventKind::Food,
            tags: vec![],
        };
        write_event(&meta, &raw_blob, None).expect("seed write");

        // Path A: happy path with key cached.
        let record = read_event("stage4-bridge").expect("read_event with key");
        assert_eq!(record.meta.event_id, "stage4-bridge");
        assert_eq!(record.encrypted_body_path, "events/stage4-bridge/body.age");
        assert!(record.analysis.is_none());

        // Path B: clear cache → DecryptionUnavailable (NOT panic, NOT IoError).
        encryption_wire::clear_event_key_cache();
        let err = read_event("stage4-bridge").expect_err("must fail without key");
        match err {
            EventStoreError::DecryptionUnavailable(_) => {}
            other => panic!("expected DecryptionUnavailable, got: {:?}", other),
        }

        // Restore the cache state we leave on neighbouring tests as we found it.
        encryption_wire::clear_event_key_cache();
    }

    /// SPEC-16 §7.2 — index_fts5 must upsert the row and return a non-zero
    /// `terms_indexed_count` for a non-empty summary (catches the
    /// empty-summary bug class). Uses a tempdir-backed `events.sqlite`.
    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn index_fts5_round_trip_against_real_sqlite() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        struct HomeGuard(Option<std::ffi::OsString>);
        impl Drop for HomeGuard {
            fn drop(&mut self) {
                match &self.0 {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
            }
        }
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", tmp.path());
        let _guard = HomeGuard(prev);

        let receipt = index_fts5("evt-1", "lunch was on plan").expect("index_fts5");
        assert_eq!(receipt.event_id, "evt-1");
        assert!(receipt.terms_indexed_count >= 4, "token count: {}", receipt.terms_indexed_count);
        // FTS5 MATCH should now find the row by a token from the summary.
        let hits = search_fts5("plan", 10).expect("search_fts5");
        assert!(hits.iter().any(|id| id == "evt-1"), "hits: {:?}", hits);
    }

    /// `count_tokens_pseudo` mirrors FTS5 `unicode61` closely enough — empty
    /// returns 0, ASCII splits on whitespace, punctuation is a delimiter.
    #[test]
    fn count_tokens_matches_intuitive_split() {
        assert_eq!(count_tokens_pseudo("").unwrap(), 0);
        assert_eq!(count_tokens_pseudo("one two three").unwrap(), 3);
        assert_eq!(count_tokens_pseudo("hello, world!").unwrap(), 2);
    }

    /// `now_iso_pseudo` must emit a canonical RFC-3339 **UTC** string (`Z`
    /// suffix per SPEC-16 §8 G2) so the TS side can `new Date(...)` it without
    /// timezone ambiguity.
    #[test]
    fn now_iso_is_rfc3339_utc() {
        let s = now_iso_pseudo();
        // SPEC-16 §8 G2 forbids local-time strings; we emit the `Z` zulu form.
        assert!(s.ends_with('Z'), "expected `Z` UTC suffix, got {}", s);
        assert!(s.contains('T'), "expected ISO-8601 date/time separator: {}", s);
    }

    /// SPEC-16 §8 G2 — the canonical UTC time helpers must round-trip
    /// epoch-ms ↔ RFC 3339 `Z` string losslessly, and a local-offset string
    /// must normalise to its UTC equivalent (the drift the spec forbids).
    #[test]
    fn ts_ms_utc_helpers_round_trip() {
        // 1764547200000 ms = 2025-12-01T00:00:00Z (KAT vector from §8).
        let ts_ms: i64 = 1_764_547_200_000;
        let s = ts_ms_to_rfc3339_utc(ts_ms);
        assert_eq!(s, "2025-12-01T00:00:00Z", "canonical Z form");
        assert_eq!(parse_ts_to_utc_ms(&s).unwrap(), ts_ms, "lossless round-trip");

        // A local-offset string (the forbidden local-time drift) must collapse
        // to the same UTC instant: 2025-12-01T08:00:00+08:00 == 00:00:00Z.
        let local = "2025-12-01T08:00:00+08:00";
        assert_eq!(parse_ts_to_utc_ms(local).unwrap(), ts_ms);
        assert_eq!(normalize_to_utc_rfc3339(local).unwrap(), "2025-12-01T00:00:00Z");

        // A non-RFC-3339 string surfaces STORE-003 rather than silently skipping.
        match parse_ts_to_utc_ms("not-a-timestamp") {
            Err(EventStoreError::InvalidTimestamp(_)) => {}
            other => panic!("expected InvalidTimestamp, got {:?}", other),
        }
    }

    /// SPEC-16 §8 G2 / JS2 — `utc_day_bounds_ms` must give the half-open UTC
    /// day range, and a timestamp written with a *local* offset must still land
    /// inside the correct UTC day (the 2026-05-22 0-events bug class). This is
    /// the unit-level proof that the date filter no longer silently skips.
    #[test]
    fn date_filter_matches_on_utc_instant_not_string_bytes() {
        let (start, end) = utc_day_bounds_ms("2025-12-01").unwrap();
        assert_eq!(start, 1_764_547_200_000); // 2025-12-01T00:00:00Z
        assert_eq!(end - start, 86_400_000); // exactly 24h

        // Event stamped 2025-12-01T08:00:00+08:00 == 2025-12-01T00:00:00Z →
        // its UTC instant IS inside the 2025-12-01 UTC day. The OLD
        // `starts_with("2025-12-01")` check would also pass here, but an event
        // stamped 2025-12-02T02:00:00+08:00 == 2025-12-01T18:00:00Z (UTC day
        // 12-01) used to be filtered OUT by the string-prefix bug. Prove the
        // instant-based filter keeps it.
        let utc_evening = parse_ts_to_utc_ms("2025-12-02T02:00:00+08:00").unwrap();
        assert_eq!(utc_evening, 1_764_612_000_000); // 2025-12-01T18:00:00Z
        assert!(
            utc_evening >= start && utc_evening < end,
            "event in UTC day 12-01 must NOT be silently skipped"
        );
        // Its raw string starts with "2025-12-02" — the byte-prefix filter
        // would have wrongly excluded it. Pin that to lock the regression.
        assert!("2025-12-02T02:00:00+08:00".starts_with("2025-12-02"));
        assert!(!"2025-12-02T02:00:00+08:00".starts_with("2025-12-01"));
    }

    /// SPEC-16 §8 G2 + §10 — end-to-end proof that a written event with a
    /// canonical UTC timestamp round-trips back through `read_event` AND is
    /// returned (not silently skipped) by a `date_iso` `query_events`, even
    /// when the event was stamped with a local offset that resolves into the
    /// queried UTC day. Exercises the real `std::fs` + `encryption_wire` bridge.
    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn write_then_query_by_utc_date_returns_event() {
        use crate::encryption_wire;
        let tmp = tempfile::TempDir::new().expect("tempdir");
        struct HomeGuard(Option<std::ffi::OsString>);
        impl Drop for HomeGuard {
            fn drop(&mut self) {
                match &self.0 {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
            }
        }
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", tmp.path());
        let _guard = HomeGuard(prev);

        // Seed: encrypt a real body the cached key can decrypt.
        let seed = [0x42u8; 32];
        let key = encryption_wire::install_event_key_from_seed(&seed).expect("install");
        let identity = encryption_wire::event_key_to_age_identity(&key).expect("identity");
        let recipient = encryption_wire::derive_recipient_from_identity(&identity);
        let envelope =
            encryption_wire::encrypt_event(b"ramen for lunch", &recipient).expect("encrypt");
        use base64::Engine as _;
        let raw_blob = base64::engine::general_purpose::STANDARD
            .decode(&envelope.ciphertext_b64)
            .expect("base64 decode");

        // Canonical UTC timestamp derived from epoch-ms (the SPEC-correct path).
        // 1764612000000 ms = 2025-12-01T18:00:00Z, but we deliberately store the
        // equivalent *local-offset* string to prove the filter is offset-agnostic.
        let ts_ms: i64 = 1_764_612_000_000;
        assert_eq!(ts_ms_to_rfc3339_utc(ts_ms), "2025-12-01T18:00:00Z");
        let meta = EventMeta {
            event_id: "utc-date-evt".into(),
            timestamp: "2025-12-02T02:00:00+08:00".into(), // == 2025-12-01T18:00Z
            kind: EventKind::Food,
            tags: vec!["fat_loss".into()],
        };
        write_event(&meta, &raw_blob, None).expect("write_event");

        // read_event returns the record with its timestamp intact.
        let rec = read_event("utc-date-evt").expect("read_event");
        assert_eq!(rec.meta.event_id, "utc-date-evt");
        assert_eq!(parse_ts_to_utc_ms(&rec.meta.timestamp).unwrap(), ts_ms);

        // query_events for the UTC calendar day 2025-12-01 MUST return it —
        // the old `starts_with("2025-12-01")` bug would have skipped it (the
        // string begins "2025-12-02"). This is the 0-events regression lock.
        let q = EventStoreQuery {
            date_iso: Some("2025-12-01".into()),
            ..Default::default()
        };
        let hits = query_events(&q).expect("query_events");
        assert_eq!(hits.len(), 1, "event must NOT be silently skipped");
        assert_eq!(hits[0].meta.event_id, "utc-date-evt");

        // And it must NOT appear under the wrong UTC day.
        let q_wrong = EventStoreQuery {
            date_iso: Some("2025-12-02".into()),
            ..Default::default()
        };
        assert_eq!(query_events(&q_wrong).expect("query").len(), 0);

        encryption_wire::clear_event_key_cache();
    }

    #[test]
    fn fts5_index_round_trip_smoke() {
        let i = FTS5Index {
            event_id: "uuid".into(),
            terms_indexed_count: 7,
            indexed_at: "2026-05-25T00:00:00Z".into(),
        };
        let j = serde_json::to_string(&i).unwrap();
        let back: FTS5Index = serde_json::from_str(&j).unwrap();
        assert_eq!(i.event_id, back.event_id);
        assert_eq!(i.terms_indexed_count, back.terms_indexed_count);
    }
}
