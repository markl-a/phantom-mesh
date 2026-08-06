// SPEC-25 §7 + §9 — skillbank skill-extraction wire types (single source of
// truth for the 6-step skill self-evolution loop: judge → extract → store →
// recall → apply → measure). TS export to
// `app/src/lib/generated/skill/`.
//
// Stage 3 (real impl — pure-logic + sqlite + providers delegation live):
// the §8.1 chrono-based event window filter, the §20.1 / §20.2 / §20.3
// prompt templates, the serde_json-backed judge + extract response parsers,
// the §6 regex PII redactor (phone / email / IPv4 / *nix path / @-mention
// real-name strip), the §8.5 recall-strategy decider + hit-set merger /
// splitter, the `<recalled_skills>` apply-step XML builder, the rusqlite
// `skills` row SELECT (`skill_load`) + UPDATE (`skill_update`), the
// rusqlite `skills_fts` BM25 keyword search (`fts5_search`), and the
// `providers_wire::complete` delegation (`providers_complete` /
// `providers_complete_structured`) are now real, and `store_skill` now
// does a real rusqlite UPSERT into the `skills` row table (self-provisioning
// the 0008 schema). Helpers whose downstream is still Stage 2 — the
// per-provider `complete_*_pseudo` HTTP adapters inside `providers_wire`,
// the SPEC-13 age-encrypt wrapper for the cross-peer sync envelope — stay
// `unimplemented!("Stage 4: <crate>")`. The `ort` runtime for embedding
// cosine search is NOT yet wired, so `embedding_search` returns `Err(())`
// (not a panic): `recall_skills` consumes it via `.ok()` and degrades to
// FTS5-only keyword recall, the §8.5 intended fallback. Runtime panic
// boundary now lives inside the providers_wire HTTP layer (one module
// deeper than before); the recall path also degrades gracefully when the
// `skills_fts` virtual table is missing (empty hit set, no panic).
//
// 中文: 本檔對應 SPEC-25 §7（資料模型）與 §9（API 合約）。skillbank（技能
// 銀行，6 步迴圈）：judge（判定）→ extract（抽取）→ store（儲存）→
// recall（召回）→ apply（套用）→ measure（量測）。Stage 1 只排 wire 型
// 別與 stub；Stage 2 接 skillbank/ 既有模組與新增 scheduler / sync。
//
// **Cross-spec reuse**: import `crate::coach_wire::RecallPolicy` — SPEC-23
// coach 與本檔共用策略 struct（單向依賴，不會 cycle）。
//
// **Privacy invariant (SPEC-25 §6 + §13 audit fix)**: `SkillExample` 絕對
// **不可** 含 raw event（原始事件）原文。Audit 後改 `{event_id_hash,
// redacted_snippet}` — `event_id_hash` = SHA-256(event_id) 截前 16 hex
// chars（不可反查回原 id）；`redacted_snippet` 是經 PII（個人身份資料）
// 過濾後 ≤ 100 字摘錄。違反此 invariant = 跨 peer（對等節點）sync 把 raw
// events 帶出本機，違反 BIG-GOAL P4「資料在你手裡」。Stage 2 extract step
// 必須先走 redactor 才能填 `SkillExample`。
//
// > **縮寫對照表（acronym table）** — SPEC（Specification，規格）/ TS
// > （TypeScript，網頁腳本語言）/ FFI（Foreign Function Interface，跨語
// > 言介面）/ LLM（Large Language Model，大型語言模型）/ FTS5（Full-Text
// > Search 第 5 版）/ age（age v1，現代檔案加密格式）/ HKDF（雜湊金鑰
// > 衍生函數）/ HMAC（雜湊訊息認證碼）/ JSON（資料交換格式）/ RPC（遠
// > 端程序呼叫）/ UUID（全域唯一識別碼）/ PII（個人身份資料）/ LWW
// > （Last-Write-Wins，後寫者贏）/ peer（對等節點）/ vault（保險庫）/
// > broker（中介伺服器）/ tier（記憶層）/ skillbank（技能銀行）

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// `EventKey` Rust-private encryption material — import 但不 re-export
// （SPEC-13 §6.2 規定不可跨 FFI）。Stage 1 不使用，預留給 Stage 2。
#[allow(unused_imports)]
use crate::encryption_wire::EventKey;
// `EventMeta` 是 SPEC-16 公開 wire 型別；judge + extract step 都吃
// `&[EventMeta]`（近 7 天 events 摘要，不含 body 解密）。
#[allow(unused_imports)]
use crate::event_storage_wire::EventMeta;
// `RecallPolicy` 跨 SPEC 共用（SPEC-23 coach + 本檔 skillbank recall 同用一
// 個策略 struct），統一從 coach_wire import 避免欄位 drift。
use crate::coach_wire::RecallPolicy;

// ─── §7.1 Skill — core skill bank row ────────────────────────────────────────

/// 一筆 skill bank（技能銀行）row — skillbank 從過去 events 抽出的可重用 user
/// 行為模式。`id` UUIDv7；`trigger_pattern` 自然語言 + 可選 regex；`steps`
/// 有序動作（1-10 條）；`examples` **已 redact** 過的事件樣本（hash +
/// 過濾後 snippet，**不是** raw event）；`quality_score` 0.0-1.0 measure
/// 步驟更新；`version` 從 1 起，user 編輯時 +1；`last_applied_at` recall
/// 命中時更新；`source_event_count` 抽取時 anchor 的 events 數（≥ 5 才進
/// judge step per §8.1）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/skill/")]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub trigger_pattern: String,
    pub steps: Vec<String>,
    pub examples: Vec<SkillExample>,
    pub version: u16,
    #[serde(default = "default_quality_score")]
    pub quality_score: f32,
    #[serde(default)]
    pub last_applied_at: u64,
    pub source_event_count: u16,
}

fn default_quality_score() -> f32 {
    0.5
}

// ─── §7.1 SkillExample — privacy-redacted event anchor ───────────────────────

/// Privacy-redacted anchor pointing to a source event. **MUST NEVER carry
/// raw event body content** per SPEC-25 §6 + §13 audit fix. `event_id_hash`
/// = SHA-256(event_id) 截前 16 hex chars（不可反查回原 id）；
/// `redacted_snippet` 是 ≤ 100 字、經 PII 過濾器（移除電話 / email / IP /
/// 路徑 / 真名）後的摘錄。違反此 invariant = raw events 跟著 skill_sync
/// 帶出本機，違反 BIG-GOAL P4。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/skill/")]
#[serde(rename_all = "camelCase")]
pub struct SkillExample {
    pub event_id_hash: String,
    pub redacted_snippet: String,
}

// ─── §7.1 / §9.5 EncryptedSkillEnvelope — `/rpc/skill/sync` wire shape ───────

/// 跨 peer skill 同步的 wire envelope。內層 `Skill` struct 經 age v1 加密
/// 寫進 `ciphertext_b64`；外層 `signature_hex` = HMAC-SHA256(cluster_secret,
/// canonical_json(envelope - signature)) 的 hex string — 接收 peer 先驗簽
/// 再解密，簽錯直接 401。**4 個欄位固定**（skill_id / version /
/// ciphertext_b64 / signature_hex），加欄位 = wire break + SPEC bump。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/skill/")]
#[serde(rename_all = "camelCase")]
pub struct EncryptedSkillEnvelope {
    /// UUIDv7 of the skill (plaintext for de-dupe + LWW resolve).
    pub skill_id: String,
    /// Plaintext for §8.7 LWW tie-break (higher version wins; same version
    /// falls back to `updated_at_ms` inside the ciphertext).
    pub version: u16,
    /// Base64 of age v1 ciphertext of canonical-JSON `Skill`.
    pub ciphertext_b64: String,
    /// Lowercase hex (no `0x`) HMAC-SHA256 over envelope minus this field.
    pub signature_hex: String,
}

// ─── §7.1 JudgeCandidate — judge → extract handoff ───────────────────────────

/// judge step 產出的「候選技能」。`trigger_pattern` 是 LLM 給的一句話主題
/// （§20.1 prompt 的 `theme`）；`repeat_count` LLM 判斷的相似 events 數
/// （≥ 5 才進 extract per §8.1）；`sample_event_ids` 是 raw event id
/// （**local-only**，組 `SkillExample` 時才走 redactor hash，不過 wire）；
/// `judged_at` UTC ms。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/skill/")]
#[serde(rename_all = "camelCase")]
pub struct JudgeCandidate {
    pub trigger_pattern: String,
    pub repeat_count: u16,
    pub sample_event_ids: Vec<String>,
    pub judged_at: u64,
}

// ─── §8.4 RecallResult — recall step output ──────────────────────────────────

/// recall 步驟回傳。`skills` 與 `scores` 平行（同 index 對應）；`scores`
/// 是 0.0-1.0 的 hybrid 分數（FTS5 BM25 normalised + embedding cosine 加
/// 權）。`recall_strategy` 標示這次走的路徑 — embedding provider 掛了會自
/// 動降到 `Fts5Only`，UI 可據此提示 user「召回降級」。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/skill/")]
#[serde(rename_all = "camelCase")]
pub struct RecallResult {
    pub skills: Vec<Skill>,
    pub scores: Vec<f32>,
    pub recall_strategy: RecallStrategy,
}

// ─── §9.7 SkillSummary — `spectyn skill status` overview ─────────────────────

// NOTE: 3 `SkillSummary` types co-exist (different aggregations, module path
// disambiguates). See docs/superpowers/skill-summary-naming.md.
//   • THIS one (`skill_wire::SkillSummary`) — dashboard card (4 fields).
//   • `rpc_wire::SkillSummary`  — sync delta (5 fields) for mesh peer sync.
//   • `skillbank::dto::SkillSummary` — full record (9 fields) for HTTP list.

/// `spectyn skill status` 與 UI 概覽用的摘要。`count_total` 全部
/// （core + recall + archival）；`count_active` non-archival（core +
/// recall）；`last_extracted_at` 上次 scheduler 跑完 extract 的 UTC ms
/// （`0` = 從未跑）；`top_3_by_score` 依 `quality_score` 排序的前 3 個
/// `name`（卡片直接顯示，不必再 query 全表）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/skill/")]
#[serde(rename_all = "camelCase")]
pub struct SkillSummary {
    pub count_total: u32,
    pub count_active: u32,
    pub last_extracted_at: u64,
    pub top_3_by_score: Vec<String>,
}

// ─── §7.1 MeasureFeedback — measure step input ───────────────────────────────

/// 餵進 measure 步驟的 user 行為 signal。三個 bool 描述「這次 apply 後
/// user 做了什麼」：`was_applied` 是否真的被注入 prompt（vs 被 token 預算
/// 砍掉）；`was_decline` user 是否點「拒絕此次套用」；`user_edited` 是否
/// 在 detail screen 編輯了 skill。三者皆 false = 中性、不變 score。§8.6
/// 公式：accepted +0.05 / declined -0.10 / edited +0.02。`observed_at`
/// UTC ms（用於每週 staleness decay）。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/skill/")]
#[serde(rename_all = "camelCase")]
pub struct MeasureFeedback {
    pub skill_id: String,
    pub was_applied: bool,
    pub was_decline: bool,
    pub user_edited: bool,
    pub observed_at: u64,
}

// ─── §6 SkillStep — 6-step loop dispatcher tag ──────────────────────────────

/// 技能 6 步迴圈的 step 列舉，給 `run_skill_step` 派工。Stage 2 match
/// 各 variant 派到對應子模組（extract.rs / memory.rs / integration.rs /
/// curator.rs）。Scheduler 每日 23:00 按宣告順序跑一輪。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/skill/")]
#[serde(rename_all = "snake_case")]
pub enum SkillStep {
    /// Step 1: LLM scans past-7-days events for repeated patterns.
    Judge,
    /// Step 2: LLM turns each candidate into a structured `Skill`.
    Extract,
    /// Step 3: age-encrypt + sqlite INSERT + FTS5 index update.
    Store,
    /// Step 4: hybrid FTS5 + embedding top-k against incoming prompt.
    Recall,
    /// Step 5: build `<recalled_skills>` block + prepend to agent prompt.
    Apply,
    /// Step 6: observe user action + update `quality_score`.
    Measure,
}

// ─── §8.4 RecallStrategy — which recall path actually ran ────────────────────

/// recall 實際走的策略。`HybridUnion` 是 §8.4 happy path（FTS5 keyword ∪
/// embedding semantic）；`HybridIntersect` 精度優先（交集，Stage 2 由
/// RecallPolicy 額外旗標啟用）；`Fts5Only` / `EmbeddingOnly` 是其中一路
/// 掛掉時的降級。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/skill/")]
#[serde(rename_all = "snake_case")]
pub enum RecallStrategy {
    Fts5Only,
    EmbeddingOnly,
    HybridUnion,
    HybridIntersect,
}

// ─── §7.1 SkillSource — provenance tag ───────────────────────────────────────

/// skill provenance（來源 / 起源）標記。`LlmExtracted` skillbank 自動抽
/// （受 quality_score 自動 demote）；`UserDefined` user 手寫於 UI（**永
/// 不**自動 demote）；`Imported` 從外部 registry import（v0.9.0+ 預留
/// variant，現在不啟用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../app/src/lib/generated/skill/")]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    LlmExtracted,
    UserDefined,
    Imported,
}

// ─── §11 SkillError — error catalog ──────────────────────────────────────────

/// Wire-facing error variants. Mirrors SPEC-25 §11.1 error catalog.
/// `#[serde(tag = "code")]` 讓 UI 可 dispatch on machine-readable string.
///
/// 中文: §11.1 6 個 variants —
/// - `JudgeFailed`: judge LLM 超時 / provider 錯 / 空 JSON
/// - `ExtractSchemaInvalid`: extract retry 後仍非合法 JSON → 跳過該 candidate
/// - `StoreFull`: sqlite 配額滿 / 磁碟滿
/// - `RecallEmpty`: 空 bank 或無 match（非硬錯誤，UI 顯示空 list）
/// - `SyncSignatureBad`: `/rpc/skill/sync` HMAC 驗失敗
/// - `EmbeddingTimeout`: embedding provider 超過延遲預算（recall 降級走純 FTS5）
#[derive(Debug, Clone, Serialize, Deserialize, TS, thiserror::Error)]
#[ts(export, export_to = "../../app/src/lib/generated/skill/")]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum SkillError {
    #[error("skill.judge_failed: {detail}")]
    JudgeFailed { detail: String },
    #[error("skill.extract_schema_invalid: candidate={candidate_trigger}")]
    ExtractSchemaInvalid { candidate_trigger: String },
    #[error("skill.store_full")]
    StoreFull,
    #[error("skill.recall_empty")]
    RecallEmpty,
    #[error("skill.sync_signature_bad")]
    SyncSignatureBad,
    #[error("skill.embedding_timeout: provider={provider}")]
    EmbeddingTimeout { provider: String },
    #[error("skill.store_failed: {detail}")]
    StoreFailed { detail: String },
}

// ─── §6 / §9 Stage-1 stub helpers (Stage 2 implements) ───────────────────────

/// Dispatch a single skill loop step. Stage 2 pseudocode: match `step` →
/// invoke the per-step entrypoint with placeholder defaults; emit telemetry
/// span so SPEC-32 OTEL（開放遙測標準）can trace which step ran.
///
/// Stage 3 wiring: real `events` / `query` / `feedback` will be threaded in
/// from `core/src/skillbank/scheduler.rs` once the daily cron lands; here we
/// just route + log.
pub fn run_skill_step(step: SkillStep) -> Result<(), SkillError> {
    // Step 1 — emit telemetry span for SPEC-32 observability
    tracing::info!(target: "spectyn::skillbank", step = ?step, "run_skill_step dispatch");

    // Step 2 — match step variant → dispatch to corresponding skill fn
    match step {
        SkillStep::Judge => {
            // Stage 3: real `&[EventMeta]` comes from scheduler.rs window query
            let _ = judge_candidates(&[], 7)?;
            Ok(())
        }
        SkillStep::Extract => {
            // Stage 3: scheduler iterates Vec<JudgeCandidate> from prior step
            let placeholder = JudgeCandidate {
                trigger_pattern: String::new(),
                repeat_count: 0,
                sample_event_ids: Vec::new(),
                judged_at: 0,
            };
            let _ = extract_skill_from_candidate(&placeholder, &[])?;
            Ok(())
        }
        SkillStep::Store => {
            // Stage 4: persist extracted Skill via age-encrypt + sqlite INSERT
            skill_store()
        }
        SkillStep::Recall => {
            // Stage 3: live `query` + `RecallPolicy` arrive from coach handoff
            let _ = recall_skills("", RecallPolicy::default())?;
            Ok(())
        }
        SkillStep::Apply => {
            // Stage 3: prompt + recalled list arrive from integration.rs
            let _ = apply_skill_to_prompt("", &[]);
            Ok(())
        }
        SkillStep::Measure => {
            // Stage 3: feedback arrives from UI / agent callback
            let placeholder = MeasureFeedback {
                skill_id: String::new(),
                was_applied: false,
                was_decline: false,
                user_edited: false,
                observed_at: 0,
            };
            record_measure(placeholder)
        }
    }
}

/// Step 1 — judge: scan `events` (`window_days` 天) with SPEC-14 frontier
/// provider (latency class `Reasoning`) using §20.1 JSON-mode prompt;
/// hallucinate 抑制（無 candidate 必回空 Vec，不編造）。
///
/// Stage 2 pseudocode: filter window → build strict-JSON prompt → frontier
/// call → parse JSON. Each sub-step delegates to a `_pseudo` helper whose
/// body still panics `Stage 3: <crate>` so audit can grep for unfinished
/// wiring.
pub fn judge_candidates(
    events: &[EventMeta],
    window_days: u8,
) -> Result<Vec<JudgeCandidate>, SkillError> {
    // Step 1 — filter to last `window_days` days (chrono-based comparator)
    let recent = filter_recent(events, window_days);

    // Step 2 — build the §20.1 strict-JSON judge prompt
    let prompt = build_judge_prompt(&recent);

    // Step 3 — invoke SPEC-14 frontier provider via providers_wire::complete
    //          (still Stage 4 — providers_wire itself is Stage 2).
    let raw_json = providers_complete(&prompt)
        .map_err(|detail| SkillError::JudgeFailed { detail })?;

    // Step 4 — parse strict-JSON → Vec<JudgeCandidate>; empty Vec is legal
    let candidates = parse_judge_json(&raw_json)
        .map_err(|detail| SkillError::JudgeFailed { detail })?;

    Ok(candidates)
}

/// Step 2 — extract: turn one `JudgeCandidate` into a structured `Skill`.
/// Stage 2 pseudocode: collect sample events → build schema-constrained
/// extract prompt → LLM with structured response_format → parse Skill →
/// **redact every `SkillExample.snippet` through PII filter** before
/// returning. Privacy invariant: a raw snippet must NEVER reach the wire.
///
/// Stage 3 wiring: parse fail retries 1× at temperature=0; second fail
/// returns `ExtractSchemaInvalid`.
pub fn extract_skill_from_candidate(
    c: &JudgeCandidate,
    events: &[EventMeta],
) -> Result<Skill, SkillError> {
    // Step 1 — collect the sample events referenced by the candidate
    let samples = collect_sample_events(c, events);

    // Step 2 — build the §20.2 schema-constrained extract prompt
    let prompt = build_extract_prompt(c, &samples);

    // Step 3 — frontier LLM call with response_format=Structured (Stage 4)
    let raw_json = providers_complete_structured(&prompt).map_err(|_| {
        SkillError::ExtractSchemaInvalid {
            candidate_trigger: c.trigger_pattern.clone(),
        }
    })?;

    // Step 4 — parse JSON into Skill struct (Stage 4 retry-at-temp=0 lives
    //          in the scheduler caller, not in the wire module itself).
    let mut skill =
        parse_skill_json(&raw_json).map_err(|_| SkillError::ExtractSchemaInvalid {
            candidate_trigger: c.trigger_pattern.clone(),
        })?;

    // Step 5 — CRITICAL privacy invariant: redact every example snippet
    // before it can leave the local process (SPEC-25 §6 + §13 audit fix).
    for ex in skill.examples.iter_mut() {
        ex.redacted_snippet = redact_pii(&ex.redacted_snippet);
    }

    Ok(skill)
}

// ─── apex ② owned-memory PHASE-2b — the LLM learn loop (judge→extract→store) ──

/// Number of trailing days of events the daily learn tick judges over. Matches
/// the existing `run_skill_step(Judge)` window (skill_wire.rs:313) and SPEC-25
/// §8.1.
const LEARN_WINDOW_DAYS: u8 = 7;

/// PHASE-2b learn pass: judge a recent `events` window with the SPEC-14 frontier
/// provider, then for every surfaced candidate extract a structured `Skill` and
/// persist it — returning the count actually stored.
///
/// Degrade contract: `judge_candidates` returns `Err(SkillError::JudgeFailed)`
/// on the default install (no `agents.toml` ⇒ no provider), and that error is
/// propagated UP so the caller (`skill_learn_tick`) can swallow it and stay
/// drain-only. Per-candidate failures inside the fold are NON-fatal (logged and
/// skipped) so one bad extract/store never discards the rest.
pub fn run_skill_learning(
    events: &[EventMeta],
    window_days: u8,
) -> Result<usize, SkillError> {
    let candidates = judge_candidates(events, window_days)?; // JudgeFailed → caller degrades
    Ok(fold_candidates_to_store(&candidates, events))
}

/// Fold every judge candidate into the store, log-and-continue on each failure.
/// Returns the number of skills successfully stored. A failed extract (e.g. no
/// provider, schema-invalid) or a failed store (e.g. briefly-locked DB) on one
/// candidate is logged and SKIPPED — it must NOT discard the remaining
/// candidates. Pure orchestration over the (already-real) extract + store fns.
fn fold_candidates_to_store(candidates: &[JudgeCandidate], events: &[EventMeta]) -> usize {
    let mut stored = 0usize;
    for c in candidates {
        match extract_skill_from_candidate(c, events) {
            Ok(skill) => match store_skill(&skill) {
                Ok(()) => stored += 1,
                Err(e) => tracing::warn!(
                    skill = %skill.id,
                    "owned-memory: store of extracted skill failed, skipping: {e}"
                ),
            },
            Err(e) => tracing::warn!(
                trigger = %c.trigger_pattern,
                "owned-memory: extract failed for candidate, skipping: {e}"
            ),
        }
    }
    stored
}

/// Read the last `window_days` days of events from `event_storage_wire`,
/// projecting each `EventRecord` down to its queryable `EventMeta` (the judge
/// step never touches decrypted bodies). A storage error (locked keystore,
/// missing data dir mapped to I/O, ...) is folded onto
/// `SkillError::JudgeFailed{detail}` so the tick's single degrade arm catches
/// BOTH no-provider and no-events uniformly. An empty data dir surfaces as
/// `Ok(vec![])` from `query_events`, NOT an error — that flows on into
/// `run_skill_learning(&[], _)` which then degrades on the no-provider judge.
fn read_recent_event_window(_window_days: u8) -> Result<Vec<EventMeta>, SkillError> {
    use crate::event_storage_wire::{query_events, EventStoreQuery};
    // A generous limit (capped at 1000 inside query_events); date_iso=None ⇒
    // "recent up to limit" (the filter_recent inside judge_candidates applies
    // the actual day-window cut on the parsed UTC instant).
    let q = EventStoreQuery {
        date_iso: None,
        kind: None,
        tag: None,
        limit: Some(1000),
        offset: None,
    };
    let records = query_events(&q).map_err(|e| SkillError::JudgeFailed {
        detail: format!("event window read: {e}"),
    })?;
    Ok(records.into_iter().map(|r| r.meta).collect())
}

/// Step 4 — recall: hybrid FTS5 + embedding top-k against `query`;
/// `policy` (shared from `coach_wire`) controls tier-aware depth per
/// SPEC-01 §8.3 AC14. Stage 2 pseudocode: FTS5 first; if
/// `policy.embedding_k > 0` run embedding cosine in parallel; union /
/// intersect top-k per requested strategy; embedding timeout gracefully
/// downgrades to `Fts5Only` (per §13 fallback path).
pub fn recall_skills(
    query: &str,
    policy: RecallPolicy,
) -> Result<RecallResult, SkillError> {
    // Step 1 — FTS5 keyword search (always runs; cheap and local).
    //          Stage 4 — `skills_fts` schema not yet landed.
    let fts_hits = fts5_search(query, &policy);

    // Step 2 — optionally run embedding semantic search in parallel.
    // Stage 4 will treat `recall_k > 0` as the gate for the embedding leg;
    // a future RecallPolicy extension may add a dedicated `embedding_k`.
    let embed_hits = if policy.recall_k > 0 {
        // graceful degrade per §13 fallback path (Err -> None)
        embedding_search(query, &policy).ok()
    } else {
        None
    };

    // Step 3 — union / intersect top-k per the requested strategy (REAL)
    let strategy = decide_recall_strategy(&fts_hits, embed_hits.as_ref(), &policy);
    let merged = merge_hits(&fts_hits, embed_hits.as_ref(), strategy);

    // Step 4 — assemble RecallResult (skills + parallel scores)
    let (skills, scores) = split_skills_and_scores(&merged);
    Ok(RecallResult {
        skills,
        scores,
        recall_strategy: strategy,
    })
}

/// Step 5 — apply: build `<recalled_skills>` XML block per §20.3 and
/// prepend to `prompt`. **Pure function**, no I/O, no async — safe to call
/// inline from the agent runtime hot path. Budget ≤ 2000 tokens (caller
/// pre-slices `recalled`).
pub fn apply_skill_to_prompt(prompt: &str, recalled: &[Skill]) -> String {
    // Step 1 — build the <recalled_skills> XML block (REAL pure string concat)
    let block = format_skills_block(recalled);

    // Step 2 — prepend block to the incoming prompt
    format!("{block}{prompt}")
}

/// Minimum hybrid-recall score (FTS5/embedding merged) a skill must clear to be
/// injected. Below this the match is too weak to be worth a system-prompt slot.
const MIN_RECALL_SCORE: f32 = 0.30;
/// Minimum learned `quality_score` a skill must hold to be injected. Filters out
/// skills the operator has repeatedly declined (SPEC-25 §7.5 auto-demote floor).
const MIN_SKILL_QUALITY: f32 = 0.30;
/// Cap on how many recalled skills get injected per turn.
const MAX_RECALL_SKILLS: usize = 5;
/// Byte budget for the rendered `<recalled_skills>` block (~2000 tokens). Skills
/// are dropped from the tail until the rendered block fits.
const MAX_RECALL_BLOCK_BYTES: usize = 8000;

/// Common English filler words dropped before building the recall query. These
/// carry no topical signal but DO appear inside indexed skill names/triggers
/// (e.g. "the" in "deploy **the** staging cluster"), so an OR-of-terms query that
/// kept them would spuriously match every skill containing a stopword. Kept
/// deliberately small — high-frequency function words only, no domain terms.
const RECALL_STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "could", "did", "do", "does",
    "for", "from", "had", "has", "have", "i", "if", "in", "is", "it", "its", "me", "my", "no",
    "not", "now", "of", "on", "or", "our", "please", "shall", "should", "so", "that", "the",
    "then", "this", "to", "us", "was", "we", "will", "with", "would", "you", "your",
];

/// Sanitize a free-form user message into a safe FTS5 MATCH query.
///
/// `recall_skills` passes its `query` argument verbatim to `skills_fts MATCH ?1`.
/// A raw natural-language prompt is wrong on three counts there: (a) FTS5 treats
/// space-separated bare terms as an implicit AND, so any single unindexed word
/// (e.g. "now", "please") makes the whole match fail; (b) prompt punctuation
/// (`?`, `"`, `:`, `-`) is FTS5 query syntax and can raise a parse error that
/// silently degrades recall to empty; and (c) common stopwords ("the", "to") are
/// indexed inside skill names and would, under OR, match every skill. We
/// therefore lowercase, split on non-alphanumerics, drop empties + stopwords +
/// length-1 tokens, double-quote each surviving token (so it is a literal FTS5
/// phrase, never an operator), and join with ` OR ` — matching on topical term
/// *overlap*, which is the right recall semantics for a free-form message.
/// Returns `None` when no usable token remains (caller injects nothing).
fn fts5_or_query(query: &str) -> Option<String> {
    // Cap term count + dedup so a pathological multi-thousand-word prompt can't
    // build a giant OR-tree that exceeds SQLite's expression depth or slows the
    // hot path, and repeated words don't inflate the query.
    const MAX_FTS_TERMS: usize = 32;
    let mut seen = std::collections::HashSet::new();
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .filter(|t| t.chars().count() > 1 && !RECALL_STOPWORDS.contains(&t.as_str()))
        .filter(|t| seen.insert(t.clone()))
        .take(MAX_FTS_TERMS)
        .map(|t| format!("\"{t}\""))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

/// apex ② recall-before-run: build the `<recalled_skills>` system block for
/// `query` (the user's latest message), or `""` when nothing should be injected.
///
/// Pure-ish helper: the only side effect is a single read-only FTS5/embedding
/// recall against the canonical DB. It NEVER errors — a recall failure (missing
/// schema, locked DB) degrades to no injection — and respects the
/// [`owned_memory_enabled`] kill-switch. The returned string is either empty or a
/// ready-to-`push_str` `<recalled_skills>` block, already sliced to the relevance
/// gate (`score >= MIN_RECALL_SCORE && quality_score >= MIN_SKILL_QUALITY`),
/// top-[`MAX_RECALL_SKILLS`], and byte-budgeted (`<= MAX_RECALL_BLOCK_BYTES`).
pub fn owned_memory_system_block(query: &str) -> String {
    if !owned_memory_enabled() {
        return String::new();
    }
    // The user's latest message is free-form prose; sanitize it into a safe FTS5
    // OR-of-terms query (see `fts5_or_query`) before recall. No usable token ⇒
    // nothing to recall on.
    let match_query = match fts5_or_query(query) {
        Some(q) => q,
        None => return String::new(),
    };
    // HOT-PATH SAFETY: FTS5-only recall. `recall_k = 0` makes `recall_skills`
    // SKIP the embedding leg entirely (skill_wire.rs:443) while `fts5_search`
    // falls back to a 16-hit limit (:1028) — so inline recall is a local,
    // non-blocking sqlite read. The embedding leg makes a BLOCKING HTTP call via
    // `block_in_place`, which panics on a current-thread runtime and would
    // stall/break the agent turn; semantic recall therefore belongs on the
    // async daily scheduler (phase 2), never the inline hot path.
    // Recall must never break the loop — degrade to no injection on any error.
    let hot_policy = RecallPolicy {
        recall_k: 0,
        ..RecallPolicy::default()
    };
    let recalled = match recall_skills(&match_query, hot_policy) {
        Ok(r) => r,
        Err(_) => return String::new(),
    };

    // Gate by hybrid score AND learned quality (parallel arrays: skills[i] ↔
    // scores[i]); keep at most MAX_RECALL_SKILLS, highest-ranked first (recall
    // already returns merged top-k order).
    let mut kept: Vec<Skill> = recalled
        .skills
        .iter()
        .zip(recalled.scores.iter())
        .filter(|(skill, &score)| {
            score >= MIN_RECALL_SCORE && skill.quality_score >= MIN_SKILL_QUALITY
        })
        .take(MAX_RECALL_SKILLS)
        .map(|(skill, _)| skill.clone())
        .collect();

    // Byte-budget: drop from the tail (lowest-ranked) until the rendered block
    // fits. `apply_skill_to_prompt("", &kept)` is the exact render used below, so
    // the measured length matches the emitted block.
    while !kept.is_empty() && apply_skill_to_prompt("", &kept).len() > MAX_RECALL_BLOCK_BYTES {
        kept.pop();
    }

    if kept.is_empty() {
        return String::new();
    }
    apply_skill_to_prompt("", &kept)
}

/// First `n` whitespace-delimited tokens of `s`, re-joined with single spaces.
/// Used to build a compact, deterministic trigger/name surface from the user's
/// query without dragging the whole message into the skill row.
fn first_words(s: &str, n: usize) -> String {
    s.split_whitespace()
        .take(n)
        .collect::<Vec<_>>()
        .join(" ")
}

/// apex ② capture-after-correction: when the operator DENIES a tool call, mint a
/// candidate [`Skill`] recording the correction and enqueue it for the Store step
/// via [`handoff_extracted_skill`]. This is the only honest in-loop "learn from
/// the human" signal available today (a deny is an unambiguous "don't do that
/// here"). No-ops when the [`owned_memory_enabled`] kill-switch is OFF.
///
/// Privacy: the skill body carries ONLY the denied tool name and a
/// [`redact_pii`]-scrubbed reason — never the raw tool args or unredacted reason.
/// The skill `id` is deterministic in `(query, denied_tool)` so repeated denials
/// of the same tool for the same situation upsert one row instead of piling up
/// near-duplicates.
pub fn capture_correction(query: &str, denied_tool: &str, reason: &str) {
    if !owned_memory_enabled() {
        return;
    }
    // Deterministic id from (query, denied_tool) so repeat denials de-dup on
    // upsert. std `DefaultHasher` is stable within a build and sufficient here —
    // this is a local dedup key, not a security/cross-process digest.
    let id = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        query.hash(&mut h);
        denied_tool.hash(&mut h);
        format!("corr-{:016x}", h.finish())
    };
    // Privacy: redact the prompt-derived trigger too (not only the reason). The
    // first words of a user prompt can carry PII (names/emails/paths) and are
    // indexed into FTS5 — scrubbing here keeps the stated "no unredacted user
    // content in the skill store" posture honest.
    let trigger = redact_pii(&first_words(query, 8));
    let skill = Skill {
        id,
        name: format!("avoid {denied_tool} when: {trigger}"),
        trigger_pattern: trigger,
        steps: vec![format!(
            "operator denied {denied_tool}: {}",
            redact_pii(reason)
        )],
        examples: vec![],
        version: 1,
        quality_score: default_quality_score(),
        last_applied_at: 0,
        source_event_count: 1,
    };
    handoff_extracted_skill(skill);
}

/// Option A (slice-1 loop closer): drain the process-local hand-off queue and
/// persist every queued candidate via [`store_skill_with_embedding`], returning
/// the number stored. This is the Store seam the phase-2 daily scheduler and the
/// `spectyn skill` CLI will reuse; for slice 1 it lets the
/// capture→drain→recall loop close visibly in one process. Recovers a poisoned
/// queue lock (a panic mid-enqueue must not wedge the Store path).
pub fn drain_corrections_to_store() -> Result<usize, SkillError> {
    let drained: Vec<HandoffEntry> = {
        let mut q = handoff_queue()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        q.drain(..).collect()
    };
    // Log-and-continue: a transient store error (e.g. a briefly-locked DB) on
    // one candidate must NOT discard the already-drained remainder. Best-effort —
    // skip the failure, keep going, return the count actually persisted.
    let mut stored = 0usize;
    for (skill, emb) in drained {
        match store_skill_with_embedding(&skill, emb.as_deref()) {
            Ok(()) => stored += 1,
            Err(e) => {
                tracing::warn!(skill = %skill.id, "owned-memory: store of a captured correction failed, skipping: {e}")
            }
        }
    }
    Ok(stored)
}

/// apex ② daily learn tick — the unattended "turn the loop once" entrypoint the
/// phase-2 scheduler (and `spectyn skill learn`) fire. Two legs:
///   1. [`drain_corrections_to_store`] persists every captured
///      capture-after-correction candidate (the honest in-loop human signal).
///   2. a DEFENSIVE [`run_skill_step`]`(Store)` drains any scheduler hand-off
///      left on the process-local queue. Its ONLY expected failure on an empty
///      queue is `SkillError::StoreFailed` — that's a legal no-op (nothing was
///      handed off this tick), so it is SWALLOWED; any OTHER error propagates.
/// Returns the number of skills actually stored this tick. The Judge/Extract/
/// Measure steps need a provider + an event window and are deferred to phase-2b.
pub fn skill_learn_tick() -> Result<usize, SkillError> {
    // Leg 1 — drain captured corrections (local, no provider needed). This is
    // the honest in-loop human signal and is the BASE count for the tick.
    let mut stored = drain_corrections_to_store()?;

    // Leg 2 — PHASE-2b additive LLM learn: read a recent event window and run
    // judge→extract→store over it. Both sub-steps DEGRADE cleanly: a failed
    // event-window read OR a no-provider judge collapses onto an info-log and
    // leaves the tick drain-only. The learning leg must NEVER turn a good drain
    // into a tick error.
    match read_recent_event_window(LEARN_WINDOW_DAYS) {
        Ok(events) => match run_skill_learning(&events, LEARN_WINDOW_DAYS) {
            Ok(n) => stored += n,
            Err(SkillError::JudgeFailed { detail }) => tracing::info!(
                "owned-memory: skill learning unavailable, drain-only this tick: {detail}"
            ),
            Err(e) => return Err(e),
        },
        Err(e) => {
            tracing::info!("owned-memory: event window unread, drain-only: {e}")
        }
    }

    // Leg 3 — DEFENSIVE scheduler hand-off drain. Empty hand-off queue is the
    // expected no-op — `skill_store` returns `StoreFailed` when there is nothing
    // to drain. Swallow ONLY that.
    match run_skill_step(SkillStep::Store) {
        Ok(()) => stored += 1,
        Err(SkillError::StoreFailed { .. }) => {}
        Err(e) => return Err(e),
    }
    Ok(stored)
}

/// Step 6 — measure: observe `feedback`, update `quality_score` per §8.6
/// formula (`accepted +0.05 / declined -0.10 / edited +0.02`). Stage 2
/// pseudocode: load skill row → recompute score → sqlite UPDATE. Auto-
/// demote to archival when score falls below `0.3` per §7.5 (wired in
/// Stage 3 via curator.rs).
pub fn record_measure(feedback: MeasureFeedback) -> Result<(), SkillError> {
    // Step 1 — load the skill row by id from sqlite (Stage 4)
    let mut skill = skill_load(&feedback.skill_id)?;

    // Step 2 — adjust quality_score per §8.6 weights
    let mut delta = 0.0_f32;
    if feedback.was_applied {
        delta += 0.05;
    }
    if feedback.was_decline {
        delta -= 0.10;
    }
    if feedback.user_edited {
        delta += 0.02;
    }
    skill.quality_score = (skill.quality_score + delta).clamp(0.0, 1.0);
    skill.last_applied_at = feedback.observed_at;

    // Step 3 — persist updated row back to sqlite (Stage 4)
    skill_update(&skill)
}

// ─── Stage 3 helpers — real impl for pure logic + sqlite + delegates ────────
//
// Per docs/superpowers/SPEC-TO-CODE-PLAYBOOK.md the Stage 2 `_pseudo` stubs
// were promoted in this commit when (a) the required crate was already in
// `core/Cargo.toml` and (b) the algorithm could either be fully self-
// contained or could legitimately delegate to a Stage-2/3 sibling module
// (the delegation itself is then permanent — only the inner panic moves).
//
// The kept-deferred bucket now is narrower than the pre-Stage-3 cut:
//   • per-provider HTTP adapters inside `providers_wire` (Stage 2)
//   • SPEC-13 age-encrypt wrapper for the cross-peer sync envelope
//   • `ort` runtime for embedding cosine search (crate not in deps)
//   • `skill_store` rusqlite INSERT — pending scheduler hand-off that
//     threads the extracted `Skill` payload into the Store step
//
// Each remaining Stage-4 stub panics with `Stage 4: <crate hint>` so an
// auditor can grep the boundary.

/// SPEC-25 §8.1: narrow `events` to the last `window_days` days using the
/// chrono-parsed `EventMeta.timestamp` (RFC-3339 / ISO-8601 string). Pure
/// function — no I/O. Events with un-parseable timestamps are dropped
/// silently rather than panicking; a corrupt row should not abort an entire
/// judge pass.
fn filter_recent(events: &[EventMeta], window_days: u8) -> Vec<&EventMeta> {
    use chrono::{DateTime, Duration, Utc};
    let now: DateTime<Utc> = Utc::now();
    let cutoff = now - Duration::days(i64::from(window_days));
    events
        .iter()
        .filter(|e| match DateTime::parse_from_rfc3339(&e.timestamp) {
            Ok(t) => t.with_timezone(&Utc) >= cutoff,
            Err(_) => false,
        })
        .collect()
}

/// Render the SPEC-25 §20.1 strict-JSON judge prompt template. Pure string
/// templating — keeps the prompt copy-pasteable next to the spec so an
/// auditor can diff prompt vs. spec line-by-line. Inlines the count of
/// candidate events so the model can self-cap (no `repeat_count > N` heroics).
fn build_judge_prompt(recent: &[&EventMeta]) -> String {
    // Compact one-line summary per event: `<ts> <kind> [tags]`.
    // Stage 4 will swap to a richer payload that includes a redacted body
    // snippet once SPEC-16's encrypted-body read path is wired through.
    let mut lines = String::with_capacity(recent.len() * 64);
    for ev in recent {
        let kind = serde_json::to_string(&ev.kind)
            .unwrap_or_else(|_| "\"unknown\"".to_string());
        lines.push_str(&format!(
            "- {ts} {kind} [{tags}]\n",
            ts = ev.timestamp,
            kind = kind.trim_matches('"'),
            tags = ev.tags.join(","),
        ));
    }

    format!(
        "You are the SPEC-25 skill judge step. Scan the user's last \
         {n} events and emit STRICT JSON of recurring behaviour patterns.\n\
         No markdown fences, no commentary. Empty array is a legal answer \
         when nothing recurs ≥ 5 times.\n\
         \n\
         Schema:\n\
         {{\n\
         \x20 \"candidates\": [\n\
         \x20   {{\n\
         \x20     \"triggerPattern\": \"<one-line theme>\",\n\
         \x20     \"repeatCount\":    <int ≥ 5>,\n\
         \x20     \"sampleEventIds\": [\"<eventId>\"]\n\
         \x20   }}\n\
         \x20 ]\n\
         }}\n\
         \n\
         Events:\n\
         {lines}",
        n = recent.len(),
        lines = lines,
    )
}

/// SPEC-14 frontier completion through `providers_wire::complete`. The
/// wire-up itself is real (Stage 3): we assemble a `ProviderRequest`
/// targeting the canonical frontier model and forward to the SPEC-14
/// public surface. The downstream per-provider `complete_*_pseudo`
/// helpers are themselves still Stage 2, so a live call propagates the
/// inner panic with a `Stage 4: providers_wire — ...` marker — the
/// delegation itself, however, is permanent and survives the Stage 2
/// → Stage 3 promotion on the providers side.
fn providers_complete(prompt: &str) -> Result<String, String> {
    use crate::providers_wire::{
        complete, Message, MessageRole, ProviderRequest, ResponseFormat,
    };
    let req = ProviderRequest {
        // Default frontier slug — `resolve_model_to_provider_type` will
        // route to Anthropic when agents.toml carries the matching entry.
        model: "claude-opus-4.7".to_string(),
        system_prompt: Some(
            "You are the SPEC-25 skill judge step. Emit STRICT JSON only.".to_string(),
        ),
        messages: vec![Message::text(MessageRole::User, prompt.to_string())],
        max_tokens: Some(2048),
        temperature: Some(0.0),
        response_format: ResponseFormat::Json,
        // Text-only completion path — no tool-calling here.
        tools: Vec::new(),
    };
    complete(req)
        .map(|r| r.text)
        .map_err(|e| format!("providers_wire::complete failed: {e:?}"))
}

/// Parse the judge response JSON into `Vec<JudgeCandidate>`. Accepts both
/// the `{"candidates": [...]}` envelope and a bare array (LLMs disagree on
/// envelope habit). Returns an empty Vec for malformed payloads with the
/// error string carried back so the caller can re-emit as
/// `SkillError::JudgeFailed{detail}`.
fn parse_judge_json(raw: &str) -> Result<Vec<JudgeCandidate>, String> {
    #[derive(Deserialize)]
    struct Envelope {
        candidates: Vec<JudgeCandidate>,
    }
    if let Ok(env) = serde_json::from_str::<Envelope>(raw) {
        return Ok(env.candidates);
    }
    serde_json::from_str::<Vec<JudgeCandidate>>(raw)
        .map_err(|e| format!("judge json parse: {e}"))
}

/// Match `candidate.sample_event_ids` against `events` and return the
/// referenced subset (preserves spec order for downstream determinism).
/// Pure function — O(N · M) for tiny M (≤ 20 sample ids per §8.1).
fn collect_sample_events<'a>(
    candidate: &JudgeCandidate,
    events: &'a [EventMeta],
) -> Vec<&'a EventMeta> {
    candidate
        .sample_event_ids
        .iter()
        .filter_map(|id| events.iter().find(|e| &e.event_id == id))
        .collect()
}

/// Render the SPEC-25 §20.2 extract prompt with `Skill` schema constraints.
/// Pure templating — schema is hand-rolled JSON next to the spec so the
/// extractor model can copy-paste the shape. Empty `examples` array is
/// permitted (the redact step below will refuse to surface raw bodies).
fn build_extract_prompt(c: &JudgeCandidate, samples: &[&EventMeta]) -> String {
    let mut sample_lines = String::new();
    for ev in samples {
        sample_lines.push_str(&format!(
            "- eventId={id} ts={ts} tags=[{tags}]\n",
            id = ev.event_id,
            ts = ev.timestamp,
            tags = ev.tags.join(","),
        ));
    }
    format!(
        "You are the SPEC-25 skill extract step. Convert the candidate \
         pattern below into ONE structured Skill. Emit STRICT JSON only.\n\
         \n\
         Trigger: {trigger}\n\
         RepeatCount: {repeat}\n\
         Samples:\n\
         {samples}\n\
         \n\
         Schema (camelCase):\n\
         {{\n\
         \x20 \"id\":              \"<UUIDv7>\",\n\
         \x20 \"name\":            \"<short label>\",\n\
         \x20 \"triggerPattern\":  \"<echo of trigger>\",\n\
         \x20 \"steps\":           [\"step 1\", \"step 2\"],\n\
         \x20 \"examples\":        [],\n\
         \x20 \"version\":         1,\n\
         \x20 \"qualityScore\":    0.5,\n\
         \x20 \"sourceEventCount\":{repeat}\n\
         }}\n",
        trigger = c.trigger_pattern,
        repeat = c.repeat_count,
        samples = sample_lines,
    )
}

/// Structured (JSON-mode strict) frontier call — same delegation pattern
/// as `providers_complete` but with `ResponseFormat::Structured` so the
/// upstream adapter pins to the strict-schema mode (GPT-5.5 strict /
/// Gemini responseSchema / Anthropic tool_use JSON). The wire-up is
/// real Stage 3; the inner `complete_*_pseudo` panic propagates from
/// providers_wire until it lands its own Stage 3 promotion.
fn providers_complete_structured(prompt: &str) -> Result<String, String> {
    use crate::providers_wire::{
        complete, Message, MessageRole, ProviderRequest, ResponseFormat,
    };
    let req = ProviderRequest {
        model: "claude-opus-4.7".to_string(),
        system_prompt: Some(
            "You are the SPEC-25 skill extract step. Emit STRICT JSON matching the Skill schema.".to_string(),
        ),
        messages: vec![Message::text(MessageRole::User, prompt.to_string())],
        max_tokens: Some(4096),
        temperature: Some(0.0),
        response_format: ResponseFormat::Structured,
        // Text-only completion path — no tool-calling here.
        tools: Vec::new(),
    };
    complete(req)
        .map(|r| r.text)
        .map_err(|e| format!("providers_wire::complete (structured) failed: {e:?}"))
}

/// Parse the extract response JSON into a `Skill`. Surface
/// `serde_json::Error` text to the caller as a `String` so the public
/// surface can re-map to `SkillError::ExtractSchemaInvalid`.
fn parse_skill_json(raw: &str) -> Result<Skill, String> {
    serde_json::from_str::<Skill>(raw).map_err(|e| format!("skill json parse: {e}"))
}

/// Regex-based PII redactor per SPEC-25 §6 privacy invariant. Strips:
///   • email addresses (`name@host.tld`)
///   • IPv4 addresses (`192.168.1.1`)
///   • E.164-ish phone numbers (`+886-912-345-678`, `(02) 1234 5678`)
///   • absolute *nix file paths (`/Users/foo/...`, `/home/foo/...`)
///   • absolute Windows paths (`C:\Users\foo\...`)
///   • `@-mention` real-name tokens (`@user42`, `@john_doe`)
/// Each pattern is replaced with a placeholder so the surrounding sentence
/// stays grammatical. Caps the output at 100 chars per §6 invariant — even
/// if redaction expands the string, the final slice is bounded.
fn redact_pii(snippet: &str) -> String {
    use regex::Regex;
    // Lazy-static would be cleanest, but pulling `once_cell` in just for
    // this would be one new transitive dep. The patterns are short enough
    // that per-call compilation is cheap (extract runs on skillbank scheduler
    // tick, not the agent hot path); revisit if benchmarks complain.
    let patterns: &[(&str, &str)] = &[
        // email — order matters: redact before user@host gets caught by
        // the @-mention rule below.
        (r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}", "<email>"),
        // IPv4 dotted-quad
        (r"\b(?:\d{1,3}\.){3}\d{1,3}\b", "<ip>"),
        // phone (international or local with separators)
        (
            r"(?:\+?\d{1,3}[\s.-]?)?(?:\(?\d{2,4}\)?[\s.-]?)\d{3,4}[\s.-]?\d{3,4}",
            "<phone>",
        ),
        // *nix absolute path (Users / home / opt / etc / var / tmp)
        (r"/(?:Users|home|opt|etc|var|tmp|usr)/[^\s]+", "<path>"),
        // Windows absolute path (single letter drive)
        (r"[A-Z]:\\[^\s]+", "<path>"),
        // @-mention (alphanumeric + underscore, 2+ chars)
        (r"@[A-Za-z0-9_]{2,}", "<mention>"),
    ];

    let mut out = snippet.to_string();
    for (pat, repl) in patterns {
        if let Ok(re) = Regex::new(pat) {
            out = re.replace_all(&out, *repl).into_owned();
        }
    }

    // Cap at 100 chars per §6 invariant. Use `char_indices` to avoid
    // splitting in the middle of a UTF-8 sequence (CJK tag values etc).
    if out.chars().count() > 100 {
        let cut = out
            .char_indices()
            .nth(100)
            .map(|(i, _)| i)
            .unwrap_or(out.len());
        out.truncate(cut);
    }
    out
}

/// Owned-memory master switch (apex ② "compounding memory"). **Default ON** —
/// the recall-before-run + capture-after-correction loop is live on a plain
/// `cargo build` with no feature flags. The env var `SPECTYN_OWNED_MEMORY` is a
/// kill-switch: only the explicit off tokens `0` / `false` / `off` / `no`
/// (case-insensitive, trimmed) disable it; any other value — including `1`,
/// `true`, `yes`, or the empty string — leaves it ON. Mirrors the parse pattern
/// of `memory_seal::memory_e2ee_enabled` but INVERTS the default
/// (memory_seal defaults OFF; owned memory defaults ON).
pub fn owned_memory_enabled() -> bool {
    match std::env::var("SPECTYN_OWNED_MEMORY") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        ),
        Err(_) => true,
    }
}

/// Resolve the on-disk path of the SPEC-16 sqlite database. Reads
/// `SPECTYN_DB_PATH` from the environment so tests can redirect to a
/// scratch file; falls back to `~/.spectyn-mesh/spectyn.db` which is the
/// canonical home for the production deployment (matches the BIG-GOAL P4
/// "data lives in your home directory" invariant).
///
/// Home resolution goes through `dirs::home_dir()` — the same convention
/// as `tasks::store::TaskStore::open_default` and `cli_config`'s path
/// helpers — NOT a raw `$HOME` read. On Windows `$HOME` is normally
/// unset, so the old env-var read silently fell through to a CWD-relative
/// `spectyn.db`, splitting the skill DB from the canonical one the rest
/// of the codebase opens.
///
/// Pure helper — does not open the connection; just produces the path
/// string. Stage 4 wiring will fold this into a shared `DbHandle` once
/// the connection pool lands.
fn resolve_db_path() -> String {
    if let Ok(p) = std::env::var("SPECTYN_DB_PATH") {
        if !p.trim().is_empty() {
            return p;
        }
    }
    if let Ok(data) = crate::cli_config::spectyn_data_dir() {
        return data
            .join("spectyn.db")
            .to_string_lossy()
            .into_owned();
    }
    // Last-ditch fallback — in-process pwd; only hit when the platform
    // cannot resolve a home directory at all, which on production means
    // the deployment is broken.
    "spectyn.db".to_string()
}

/// FTS5 BM25 keyword search over the SPEC-25 `skills` row table via its
/// `skills_fts` external-content FTS5 mirror (the `skills` table itself is a
/// regular table; only `skills_fts` is virtual).
/// Real `rusqlite` call: opens the canonical DB, prepares a `MATCH ?`
/// query against `skills_fts`, and maps rows back to `(Skill, score)`
/// tuples. When the schema or table is absent (e.g. fresh install before
/// the 0008 migration lands) we return an empty hit set — that's the
/// `RecallEmpty` happy path, not a hard error.
fn fts5_search(query: &str, policy: &RecallPolicy) -> Vec<(Skill, f32)> {
    use rusqlite::Connection;

    // Open the DB read-only; abort gracefully on any failure (missing
    // file, locked, schema not yet migrated) — recall must never crash
    // the agent runtime over a transient storage problem.
    let path = resolve_db_path();
    let conn = match Connection::open(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    // `recall_k` caps the per-leg result count. 0 = take 16 as a sane
    // default so the merger has material to work with.
    let limit: i64 = if policy.recall_k == 0 {
        16
    } else {
        policy.recall_k as i64
    };

    // Stage 3 SQL: pull the canonical Skill columns + BM25 score in one
    // shot. The `skills_fts` virtual table is the FTS5 mirror of
    // `skills`; bm25() ranks lower = more relevant, so we flip the sign
    // and normalise into [0,1) downstream (caller chains `merge_hits`).
    let mut stmt = match conn.prepare(
        "SELECT s.id, s.name, s.trigger_pattern, s.steps_json, \
                s.examples_json, s.version, s.quality_score, \
                s.last_applied_at, s.source_event_count, \
                bm25(skills_fts) AS rank \
         FROM skills_fts \
         JOIN skills s ON s.rowid = skills_fts.rowid \
         WHERE skills_fts MATCH ?1 \
         ORDER BY rank ASC \
         LIMIT ?2",
    ) {
        Ok(s) => s,
        // Schema not landed → empty hit set, caller degrades cleanly.
        Err(_) => return Vec::new(),
    };

    let rows = stmt.query_map(rusqlite::params![query, limit], |row| {
        // P0-8: open() the possibly-sealed columns. A sealed value that won't
        // decrypt maps to a rusqlite error here so the surrounding
        // `filter_map(|r| r.ok())` simply drops that row from the hit set
        // (fail-closed — ciphertext is never returned as a recall result). When
        // not sealed, open() is a verbatim passthrough so OFF-path recall is
        // byte-identical.
        let raw_name: String = row.get(1)?;
        let raw_trigger: String = row.get(2)?;
        let raw_steps_json: String = row.get(3)?;
        let raw_examples_json: String = row.get(4)?;
        let open = |stored: &str| -> rusqlite::Result<String> {
            crate::skillbank::memory_seal::open(stored)
                .map_err(|_| rusqlite::Error::ExecuteReturnedResults)
        };
        let name = open(&raw_name)?;
        let trigger_pattern = open(&raw_trigger)?;
        let steps_json = open(&raw_steps_json)?;
        let examples_json = open(&raw_examples_json)?;
        let steps: Vec<String> =
            serde_json::from_str(&steps_json).unwrap_or_default();
        let examples: Vec<SkillExample> =
            serde_json::from_str(&examples_json).unwrap_or_default();
        let bm25: f64 = row.get(9)?;
        // bm25 is non-positive; map into [0, 1] via 1/(1+|bm25|).
        let score: f32 = (1.0 / (1.0 + bm25.abs())) as f32;
        let skill = Skill {
            id: row.get(0)?,
            name,
            trigger_pattern,
            steps,
            examples,
            version: row.get::<_, i64>(5)? as u16,
            quality_score: row.get::<_, f64>(6)? as f32,
            last_applied_at: row.get::<_, i64>(7)? as u64,
            source_event_count: row.get::<_, i64>(8)? as u16,
        };
        Ok((skill, score))
    });

    match rows {
        Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

// ─── §8.4 embedding-search provider hook (test-injectable) ───────────────────
//
// `embedding_search` semantic-recall leg (M3). The production `ort` + MiniLM
// runtime is still Stage 4 (crate not in deps), so there is NO embedder wired
// in the shipping lib build — the thread-local hook below is therefore `None`
// in production and `embedding_search` returns `Err(())`, exactly preserving
// the de-panic floor `recall_skills` relies on (`embedding_search(..).ok()` →
// FTS5-only fallback per §13). Tests install a deterministic `FixtureEmbedder`
// into the hook so the real cosine-ranking path can be exercised end-to-end
// without pulling the ONNX runtime. The hook is a `thread_local!` so a test on
// one thread never bleeds an embedder into another (and so production stays
// `None` with zero synchronization cost).

thread_local! {
    /// Per-thread optional embedder for the recall semantic leg. `None` in
    /// production (no `ort` runtime yet) → `embedding_search` Errs and recall
    /// degrades to FTS5-only. Tests set this via [`set_test_embedder`].
    static EMBEDDER_HOOK: std::cell::RefCell<Option<Box<dyn EmbeddingProvider>>> =
        const { std::cell::RefCell::new(None) };
}

/// Install a thread-local [`EmbeddingProvider`] for the duration of a test so
/// [`embedding_search`] can exercise the real cosine-ranking path. Test-only:
/// production never installs one, so the de-panic `Err(())` path stays intact.
/// Returns immediately; pair with [`clear_test_embedder`] to restore the
/// production (no-embedder) state.
#[cfg(test)]
fn set_test_embedder(provider: Box<dyn EmbeddingProvider>) {
    EMBEDDER_HOOK.with(|h| *h.borrow_mut() = Some(provider));
}

/// Remove any thread-local embedder, restoring the production no-embedder
/// state (so a later `embedding_search` on this thread Errs again). Test-only.
#[cfg(test)]
fn clear_test_embedder() {
    EMBEDDER_HOOK.with(|h| *h.borrow_mut() = None);
}

/// Load every stored skill together with its decoded embedding vector from the
/// `skills` table (M3 read side of the 0009 `embedding` BLOB column). Rows whose
/// `embedding` is NULL or a corrupt (non-multiple-of-4) BLOB are skipped — they
/// simply do not participate in the semantic leg, never panic. Returns an empty
/// Vec (not an error) when the DB / schema is absent so the caller can decide
/// whether that means "degrade to FTS5-only".
fn load_skill_embeddings() -> Vec<(Skill, Vec<f32>)> {
    use rusqlite::Connection;

    let path = resolve_db_path();
    let conn = match Connection::open(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut stmt = match conn.prepare(
        "SELECT id, name, trigger_pattern, steps_json, examples_json, \
                version, quality_score, last_applied_at, source_event_count, \
                embedding \
         FROM skills \
         WHERE embedding IS NOT NULL",
    ) {
        Ok(s) => s,
        // Schema not landed (no `embedding` column / no `skills` table) →
        // empty set, caller degrades cleanly.
        Err(_) => return Vec::new(),
    };

    let rows = stmt.query_map([], |row| {
        // P0-8: open() the possibly-sealed columns; an undecryptable sealed value
        // maps to a rusqlite error so the row is dropped by the downstream
        // `filter_map(|r| r.ok())` (fail-closed). Verbatim passthrough when not
        // sealed, so the OFF-path semantic leg is byte-identical.
        let raw_name: String = row.get(1)?;
        let raw_trigger: String = row.get(2)?;
        let raw_steps_json: String = row.get(3)?;
        let raw_examples_json: String = row.get(4)?;
        let open = |stored: &str| -> rusqlite::Result<String> {
            crate::skillbank::memory_seal::open(stored)
                .map_err(|_| rusqlite::Error::ExecuteReturnedResults)
        };
        let name = open(&raw_name)?;
        let trigger_pattern = open(&raw_trigger)?;
        let steps_json = open(&raw_steps_json)?;
        let examples_json = open(&raw_examples_json)?;
        let steps: Vec<String> = serde_json::from_str(&steps_json).unwrap_or_default();
        let examples: Vec<SkillExample> =
            serde_json::from_str(&examples_json).unwrap_or_default();
        let blob: Vec<u8> = row.get(9)?;
        let skill = Skill {
            id: row.get(0)?,
            name,
            trigger_pattern,
            steps,
            examples,
            version: row.get::<_, i64>(5)? as u16,
            quality_score: row.get::<_, f64>(6)? as f32,
            last_applied_at: row.get::<_, i64>(7)? as u64,
            source_event_count: row.get::<_, i64>(8)? as u16,
        };
        Ok((skill, blob))
    });

    let iter = match rows {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };

    iter.filter_map(|r| r.ok())
        // Decode the BLOB → Vec<f32>; drop corrupt rows (None) silently.
        .filter_map(|(skill, blob)| blob_to_embedding(&blob).map(|v| (skill, v)))
        .collect()
}

/// SPEC-25 §8.4 embedding (semantic) recall leg — M3. Embeds `query` via the
/// installed [`EmbeddingProvider`], loads every stored skill's persisted
/// embedding (the 0009 BLOB column, decoded via [`blob_to_embedding`]),
/// cosine-ranks them against the query vector, and returns the top-k highest
/// scorers per `policy.recall_k` as `Vec<(Skill, f32)>` (descending score).
///
/// **De-panic floor (CRUCIAL, unchanged from M2):** when there is NO provider
/// installed (the production lib build — `ort` MiniLM is still Stage 4) OR no
/// stored skill carries an embedding, this returns `Err(())`. `recall_skills`
/// consumes it via `.ok()` and degrades to FTS5-only — the §13 fallback the
/// `skill_floor_stubs_return_typed_errors_not_panic` test pins. A real call
/// must never crash the agent runtime over a missing / empty embedder.
fn embedding_search(query: &str, policy: &RecallPolicy) -> Result<Vec<(Skill, f32)>, ()> {
    // Step 1 — embed the query via an embedder, in priority order:
    //   (a) the thread-local test hook (`set_test_embedder`), if installed; else
    //   (b) the PRODUCTION `ApiEmbedder` — but ONLY when the operator has
    //       explicitly configured one (`SPECTYN_EMBED_PROVIDER` + a key).
    // When NEITHER is present — the default, shipping state — this returns
    // `Err(())` and the caller degrades to FTS5-only (the §13 fallback, exactly
    // as in M3). The test hook is consulted FIRST: when a test installs one,
    // `embed` succeeds and short-circuits before `from_env` is ever called, so
    // unit tests stay hermetic. (Tests never set `SPECTYN_EMBED_PROVIDER`, so
    // even if an installed hook's `embed` returned `Err`, the `or_else` arm's
    // `from_env()` would still be `None` — no accidental network call.)
    let query_vec = EMBEDDER_HOOK
        .with(|h| h.borrow().as_ref().and_then(|p| p.embed(query).ok()))
        .or_else(|| {
            // Default-OFF: `from_env` is `None` unless explicitly configured.
            ApiEmbedder::from_env().and_then(|e| e.embed(query).ok())
        });
    let query_vec = match query_vec {
        Some(v) if !v.is_empty() => v,
        // No provider, provider errored, or empty embedding → degrade.
        _ => return Err(()),
    };

    // Step 2 — load the stored skill embeddings (0009 BLOB column). No stored
    // embeddings (fresh DB / NULL columns / corrupt rows) → nothing to rank.
    let stored = load_skill_embeddings();
    if stored.is_empty() {
        return Err(());
    }

    // Step 3 — cosine-rank every stored embedding against the query vector.
    // `cosine` is total + de-panicked: a dimension mismatch yields 0.0, never
    // a crash, so a stray differently-sized vector just sorts to the bottom.
    let mut scored: Vec<(Skill, f32)> = stored
        .into_iter()
        .map(|(skill, vec)| {
            let score = cosine(&query_vec, &vec);
            (skill, score)
        })
        .collect();

    // Highest cosine first; stable secondary sort by id keeps ties
    // deterministic for the merge step + tests.
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.id.cmp(&b.0.id))
    });

    // Step 4 — take the top-k per policy (0 ⇒ a sane default of 16, matching
    // the FTS5 leg so the merger has symmetric material to work with).
    let k: usize = if policy.recall_k == 0 {
        16
    } else {
        policy.recall_k as usize
    };
    scored.truncate(k);

    Ok(scored)
}

/// SPEC-25 §8.5 recall-strategy decider — picks the variant that matches
/// what actually ran (vs. what was requested) so the UI can honestly
/// surface "降級 to FTS5 only" when the embedding leg fell over. Pure fn.
fn decide_recall_strategy(
    fts: &[(Skill, f32)],
    embed: Option<&Vec<(Skill, f32)>>,
    _policy: &RecallPolicy,
) -> RecallStrategy {
    match (fts.is_empty(), embed) {
        // Both legs returned hits → declare hybrid (caller picks union /
        // intersect downstream; default to union per §8.4 happy path).
        (false, Some(e)) if !e.is_empty() => RecallStrategy::HybridUnion,
        // Embedding leg ran but returned nothing → FTS-only
        (false, Some(_)) | (false, None) => RecallStrategy::Fts5Only,
        // FTS leg empty, embedding had hits
        (true, Some(e)) if !e.is_empty() => RecallStrategy::EmbeddingOnly,
        // Both empty (or no embedding leg at all) → still mark Fts5Only so
        // the UI doesn't claim a hybrid path that never ran.
        (true, _) => RecallStrategy::Fts5Only,
    }
}

/// Merge FTS + embedding hit sets per the chosen `RecallStrategy`. Pure fn.
/// De-duplication is by `Skill.id` (UUIDv7); when the same skill appears in
/// both legs the higher-scored variant wins. Intersection requires presence
/// in BOTH legs; union keeps everything.
fn merge_hits(
    fts: &[(Skill, f32)],
    embed: Option<&Vec<(Skill, f32)>>,
    strategy: RecallStrategy,
) -> Vec<(Skill, f32)> {
    use std::collections::HashMap;
    let empty: Vec<(Skill, f32)> = Vec::new();
    let e_slice: &[(Skill, f32)] = embed.map(|v| v.as_slice()).unwrap_or(&empty);

    match strategy {
        RecallStrategy::Fts5Only => fts.to_vec(),
        RecallStrategy::EmbeddingOnly => e_slice.to_vec(),
        RecallStrategy::HybridUnion => {
            let mut by_id: HashMap<String, (Skill, f32)> = HashMap::new();
            for (s, sc) in fts.iter().chain(e_slice.iter()).cloned() {
                by_id
                    .entry(s.id.clone())
                    .and_modify(|(_, existing)| {
                        if sc > *existing {
                            *existing = sc;
                        }
                    })
                    .or_insert((s, sc));
            }
            let mut v: Vec<(Skill, f32)> = by_id.into_values().collect();
            // Highest score first; stable secondary sort by id for tests.
            v.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.id.cmp(&b.0.id))
            });
            v
        }
        RecallStrategy::HybridIntersect => {
            let e_ids: std::collections::HashSet<&str> =
                e_slice.iter().map(|(s, _)| s.id.as_str()).collect();
            let mut v: Vec<(Skill, f32)> = fts
                .iter()
                .filter(|(s, _)| e_ids.contains(s.id.as_str()))
                .cloned()
                .collect();
            v.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.id.cmp(&b.0.id))
            });
            v
        }
    }
}

/// Fan out merged hits into the parallel `skills` / `scores` Vecs the
/// `RecallResult` wire shape requires. Pure fn.
fn split_skills_and_scores(merged: &[(Skill, f32)]) -> (Vec<Skill>, Vec<f32>) {
    let mut skills = Vec::with_capacity(merged.len());
    let mut scores = Vec::with_capacity(merged.len());
    for (s, sc) in merged {
        skills.push(s.clone());
        scores.push(*sc);
    }
    (skills, scores)
}

/// Recall@k metric: the fraction of `expected_ids` that appear among the first
/// `k` entries of `recalled` (ordered best-first). Returns a value in `[0.0, 1.0]`.
///
/// Semantics: `hits / expected_ids.len()`, where a hit is an expected id present
/// in `recalled[..min(k, recalled.len())]`. `expected_ids` empty ⇒ `0.0` (no
/// signal — never `NaN`/divide-by-zero). Duplicate ids in `recalled` are counted
/// once per expected id. Pure fn, no I/O — this is the "measurable recall
/// hit-rate" the apex-② acceptance test asserts a threshold over (Task 5).
pub fn recall_hit_rate(expected_ids: &[&str], recalled: &[Skill], k: usize) -> f32 {
    if expected_ids.is_empty() {
        return 0.0;
    }
    let cap = k.min(recalled.len());
    let topk: std::collections::HashSet<&str> =
        recalled[..cap].iter().map(|s| s.id.as_str()).collect();
    let hits = expected_ids.iter().filter(|id| topk.contains(*id)).count();
    hits as f32 / expected_ids.len() as f32
}

/// Render the SPEC-25 §20.3 `<recalled_skills>` XML block. Pure string
/// concat. Stays inside the 2000-token budget by the caller pre-slicing
/// `recalled` (this function does not truncate — that would silently hide
/// missing skills from the model).
fn format_skills_block(recalled: &[Skill]) -> String {
    if recalled.is_empty() {
        // Empty block still wraps so prepend logic stays uniform; the
        // model treats an empty container as "no skills retrieved".
        return String::from("<recalled_skills/>\n");
    }
    let mut out = String::from("<recalled_skills>\n");
    for s in recalled {
        out.push_str("  <skill>\n");
        out.push_str(&format!("    <name>{}</name>\n", escape_xml(&s.name)));
        out.push_str(&format!(
            "    <trigger>{}</trigger>\n",
            escape_xml(&s.trigger_pattern)
        ));
        out.push_str("    <steps>\n");
        for st in &s.steps {
            out.push_str(&format!("      <step>{}</step>\n", escape_xml(st)));
        }
        out.push_str("    </steps>\n");
        out.push_str("  </skill>\n");
    }
    out.push_str("</recalled_skills>\n");
    out
}

/// Minimal XML escape for the 5 reserved chars. Pure fn — no external dep.
fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// sqlite SELECT Skill row by id. Real rusqlite query against the SPEC-25
/// `skills` table. Returns `SkillError::RecallEmpty` when the id is
/// unknown (preferred over a generic "not found" so the UI surfaces a
/// single dispatchable error code) and `SkillError::StoreFull` when the
/// underlying connection cannot be opened (typically disk-full).
fn skill_load(skill_id: &str) -> Result<Skill, SkillError> {
    use rusqlite::Connection;

    let path = resolve_db_path();
    let conn = Connection::open(&path).map_err(|_| SkillError::StoreFull)?;

    let mut stmt = conn
        .prepare(
            "SELECT id, name, trigger_pattern, steps_json, examples_json, \
                    version, quality_score, last_applied_at, source_event_count \
             FROM skills WHERE id = ?1 LIMIT 1",
        )
        .map_err(|_| SkillError::RecallEmpty)?;

    let mut rows = stmt
        .query(rusqlite::params![skill_id])
        .map_err(|_| SkillError::RecallEmpty)?;

    let row = match rows.next() {
        Ok(Some(r)) => r,
        _ => return Err(SkillError::RecallEmpty),
    };

    // P0-8: open() the possibly-sealed columns. Passthrough when not sealed, so
    // the OFF-path read is byte-identical; fail-closed (RecallEmpty) on an
    // undecryptable sealed value so ciphertext is never surfaced as plaintext.
    let raw_name: String = row.get(1).map_err(|_| SkillError::RecallEmpty)?;
    let raw_trigger: String = row.get(2).map_err(|_| SkillError::RecallEmpty)?;
    let raw_steps_json: String = row.get(3).map_err(|_| SkillError::RecallEmpty)?;
    let raw_examples_json: String = row.get(4).map_err(|_| SkillError::RecallEmpty)?;
    let name = open_sealed(&raw_name)?;
    let trigger_pattern = open_sealed(&raw_trigger)?;
    let steps_json = open_sealed(&raw_steps_json)?;
    let examples_json = open_sealed(&raw_examples_json)?;
    let steps: Vec<String> = serde_json::from_str(&steps_json).unwrap_or_default();
    let examples: Vec<SkillExample> =
        serde_json::from_str(&examples_json).unwrap_or_default();

    Ok(Skill {
        id: row.get(0).map_err(|_| SkillError::RecallEmpty)?,
        name,
        trigger_pattern,
        steps,
        examples,
        version: row.get::<_, i64>(5).map_err(|_| SkillError::RecallEmpty)? as u16,
        quality_score: row.get::<_, f64>(6).map_err(|_| SkillError::RecallEmpty)? as f32,
        last_applied_at: row.get::<_, i64>(7).map_err(|_| SkillError::RecallEmpty)? as u64,
        source_event_count: row.get::<_, i64>(8).map_err(|_| SkillError::RecallEmpty)?
            as u16,
    })
}

/// One enumerated row for `skill_list` — the inspectable subset of a [`Skill`]
/// the `spectyn skill list` CLI surfaces (id + plaintext name + learned quality
/// + last-applied recency). Deliberately NOT the full [`Skill`] (no steps /
/// examples) so listing stays a cheap projection and never drags a redacted
/// example snippet onto the wire.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillListRow {
    pub id: String,
    pub name: String,
    pub quality_score: f32,
    pub last_applied_at: u64,
}

/// Enumerate every stored skill as a [`SkillListRow`], highest learned quality
/// first (ties broken by `id` for a stable order). Real `rusqlite` SELECT
/// against the SPEC-25 `skills` table; the `name` column is routed through
/// [`open_sealed`] so a sealed-at-rest store (`SPECTYN_ENCRYPT_MEMORY` ON) lists
/// plaintext names — fail-CLOSED, exactly like [`skill_load`]: a row whose name
/// won't decrypt is DROPPED, never surfaced as ciphertext. When the `skills`
/// table is absent (fresh install before the 0008 migration lands) the `prepare`
/// fails and we return an empty list — that's the happy "nothing stored yet"
/// path, not a hard error.
pub fn skill_list() -> Result<Vec<SkillListRow>, SkillError> {
    use rusqlite::Connection;

    let path = resolve_db_path();
    let conn = Connection::open(&path).map_err(|_| SkillError::StoreFull)?;

    // Missing table ⇒ prepare Errs ⇒ empty list (no skills stored yet).
    let mut stmt = match conn.prepare(
        "SELECT id, name, quality_score, last_applied_at \
         FROM skills ORDER BY quality_score DESC, id ASC",
    ) {
        Ok(s) => s,
        Err(_) => return Ok(vec![]),
    };

    let mut rows = stmt
        .query([])
        .map_err(|_| SkillError::RecallEmpty)?;

    let mut out: Vec<SkillListRow> = Vec::new();
    while let Some(row) = rows.next().map_err(|_| SkillError::RecallEmpty)? {
        let id: String = match row.get(0) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let raw_name: String = match row.get(1) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Fail-closed: drop a row whose (possibly-sealed) name won't decrypt
        // rather than surface ciphertext — mirrors skill_load's open_sealed rule.
        let name = match open_sealed(&raw_name) {
            Ok(n) => n,
            Err(_) => continue,
        };
        let quality_score = match row.get::<_, f64>(2) {
            Ok(v) => v as f32,
            Err(_) => continue,
        };
        let last_applied_at = match row.get::<_, i64>(3) {
            Ok(v) => v as u64,
            Err(_) => continue,
        };
        out.push(SkillListRow { id, name, quality_score, last_applied_at });
    }
    Ok(out)
}

/// Aggregate quality counters for the `skills` store — the inspectable summary
/// behind `spectyn skill stats`. `high`/`medium`/`low` partition by learned
/// [`Skill::quality_score`] (≥0.70 high, [0.30,0.70) medium, <0.30 low) so an
/// operator can see at a glance how much of the bank is trustworthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillStats {
    pub total: u32,
    pub high: u32,
    pub medium: u32,
    pub low: u32,
}

impl Default for SkillStats {
    fn default() -> Self {
        Self { total: 0, high: 0, medium: 0, low: 0 }
    }
}

/// Compute the [`SkillStats`] quality histogram in ONE aggregate query (reads
/// only the never-sealed `quality_score` column, so no [`open_sealed`] is
/// needed). A missing `skills` table (fresh install) ⇒ `Ok(SkillStats::default())`
/// (all zeros), the same "nothing stored yet" happy path as [`skill_list`]. The
/// conditional `SUM`s are `NULL` over an empty table, so each is read as
/// `Option<i64>` and coalesced to `0`.
pub fn skill_stats() -> Result<SkillStats, SkillError> {
    use rusqlite::Connection;

    let path = resolve_db_path();
    let conn = Connection::open(&path).map_err(|_| SkillError::StoreFull)?;

    // Missing table ⇒ prepare Errs ⇒ default (zeros).
    let mut stmt = match conn.prepare(
        "SELECT \
            COUNT(*), \
            SUM(quality_score >= 0.70), \
            SUM(quality_score >= 0.30 AND quality_score < 0.70), \
            SUM(quality_score < 0.30) \
         FROM skills",
    ) {
        Ok(s) => s,
        Err(_) => return Ok(SkillStats::default()),
    };

    let stats = stmt
        .query_row([], |row| {
            let total: i64 = row.get(0)?;
            // Conditional SUMs are NULL over an empty table → coalesce to 0.
            let high: i64 = row.get::<_, Option<i64>>(1)?.unwrap_or(0);
            let medium: i64 = row.get::<_, Option<i64>>(2)?.unwrap_or(0);
            let low: i64 = row.get::<_, Option<i64>>(3)?.unwrap_or(0);
            Ok(SkillStats {
                total: total.max(0) as u32,
                high: high.max(0) as u32,
                medium: medium.max(0) as u32,
                low: low.max(0) as u32,
            })
        })
        .map_err(|_| SkillError::RecallEmpty)?;
    Ok(stats)
}

/// Open one possibly-sealed `skills` column value back to plaintext for a read,
/// mapping a decrypt failure to `SkillError::RecallEmpty` (fail-closed — a
/// sealed value that won't decrypt is never surfaced as ciphertext). Passthrough
/// when the value is not sealed (flag-off / legacy), so OFF-path reads are
/// byte-identical. Thin wrapper over `memory_seal::open` so the read paths
/// (`skill_load`, `fts5_search`, `load_skill_embeddings`) share one rule.
fn open_sealed(stored: &str) -> Result<String, SkillError> {
    crate::skillbank::memory_seal::open(stored)
        .map_err(|_| SkillError::RecallEmpty)
}

/// sqlite UPDATE Skill row. Real rusqlite write — refreshes every mutable
/// column so a single `record_measure` call updates score + last_applied
/// + version + redacted examples atomically. Maps connection / write
/// failures to `SkillError::StoreFull` (the closest match in the §11.1
/// catalog; disk-full is the dominant real-world cause).
fn skill_update(skill: &Skill) -> Result<(), SkillError> {
    use rusqlite::Connection;

    let path = resolve_db_path();
    let conn = Connection::open(&path).map_err(|_| SkillError::StoreFull)?;

    let steps_json =
        serde_json::to_string(&skill.steps).map_err(|_| SkillError::StoreFull)?;
    let examples_json =
        serde_json::to_string(&skill.examples).map_err(|_| SkillError::StoreFull)?;

    conn.execute(
        "UPDATE skills SET \
            name = ?1, \
            trigger_pattern = ?2, \
            steps_json = ?3, \
            examples_json = ?4, \
            version = ?5, \
            quality_score = ?6, \
            last_applied_at = ?7, \
            source_event_count = ?8 \
         WHERE id = ?9",
        rusqlite::params![
            skill.name,
            skill.trigger_pattern,
            steps_json,
            examples_json,
            skill.version as i64,
            skill.quality_score as f64,
            skill.last_applied_at as i64,
            skill.source_event_count as i64,
            skill.id,
        ],
    )
    .map_err(|_| SkillError::StoreFull)?;

    Ok(())
}

/// sqlite INSERT for the store step — real rusqlite write into the
/// SPEC-25 `skills` table. The age-encryption wrapper (SPEC-13 EventKey)
/// is **not** in this code path because §6 + §13 audit fix requires the
/// row to be plaintext-searchable for FTS5; the encryption layer wraps
/// the **cross-peer sync envelope** (`EncryptedSkillEnvelope`), not the
/// local row. This split is intentional — see SPEC-25 §13.
fn skill_store() -> Result<(), SkillError> {
    // M1 store hand-off (apex ② owned-memory): the parameterless dispatch
    // from `run_skill_step(SkillStep::Store)` carries no `Skill` argument,
    // so the extracted payload is threaded in out-of-band through the
    // process-local hand-off queue (`handoff_extracted_skill`). Drain every
    // queued skill and persist each via the real `store_skill(&skill)` write
    // path so an extracted skill actually lands in the SPEC-25 `skills` table
    // and becomes FTS5-recallable.
    //
    // Contract preserved: when the queue is genuinely empty (no scheduler
    // hand-off happened) we still return the typed
    // `Err(SkillError::StoreFailed{..})` — never panic (v0.6 GA floor) and
    // never silently succeed on a no-op. This keeps the de-panic floor test
    // (`matches!(skill_store(), Err(SkillError::StoreFailed{..}))`) green.
    let drained: Vec<HandoffEntry> = {
        let mut q = handoff_queue()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        q.drain(..).collect()
    };

    if drained.is_empty() {
        return Err(SkillError::StoreFailed {
            detail: "Store dispatch has no extracted skill payload yet — \
                     hand one off via handoff_extracted_skill(skill)"
                .into(),
        });
    }

    // Persist every drained skill WITH its optional embedding (apex ②
    // embed-at-store): when the hand-off carried a precomputed vector, it
    // lands in `skills.embedding` so the semantic recall leg can rank it;
    // when `None`, the column stays NULL (FTS5-only, pre-P0-1 behaviour). A
    // single failure aborts and surfaces the typed error; skills already
    // written stay written (the real path is an idempotent upsert by `id`,
    // so a retry re-drains nothing but re-running extract is harmless).
    for (skill, embedding) in &drained {
        store_skill_with_embedding(skill, embedding.as_deref())?;
    }
    Ok(())
}

/// One queued Store hand-off: the extracted [`Skill`] plus an OPTIONAL
/// precomputed embedding vector (apex ② embed-at-store). `None` ⇒ store with
/// the `skills.embedding` column NULL (FTS5-only, pre-P0-1 behaviour).
type HandoffEntry = (Skill, Option<Vec<f32>>);

/// Process-local hand-off queue backing the Store step (M1, apex ②).
///
/// `run_skill_step(SkillStep::Store)` invokes the parameterless
/// [`skill_store`] which has no `Skill` argument to persist. Until the
/// scheduler grows a typed channel, the extract step (or a test) deposits the
/// freshly-extracted skill here with [`handoff_extracted_skill`] (or its
/// embedding-bearing variant); the Store step then drains and persists each
/// entry via [`store_skill_with_embedding`]. A `Mutex<VecDeque>` is sufficient:
/// the skillbank loop is single-process and the Store step runs on the scheduler
/// tick, not the agent hot path, so lock contention is a non-issue.
fn handoff_queue() -> &'static std::sync::Mutex<std::collections::VecDeque<HandoffEntry>> {
    use std::sync::{Mutex, OnceLock};
    static QUEUE: OnceLock<Mutex<std::collections::VecDeque<HandoffEntry>>> = OnceLock::new();
    QUEUE.get_or_init(|| Mutex::new(std::collections::VecDeque::new()))
}

/// Enqueue an extracted [`Skill`] for the next Store step to persist with NO
/// embedding (FTS5-only recall). Back-compat wrapper over
/// [`handoff_extracted_skill_with_embedding`]; existing callers/tests are
/// unchanged — the queued skill is drained and written to the SPEC-25 `skills`
/// table the next time `run_skill_step(SkillStep::Store)` (i.e.
/// [`skill_store`]) runs, leaving the `embedding` column NULL.
pub fn handoff_extracted_skill(skill: Skill) {
    handoff_extracted_skill_with_embedding(skill, None);
}

/// Enqueue an extracted [`Skill`] together with an OPTIONAL precomputed
/// embedding vector (apex ② embed-at-store). When `Some`, the next Store step
/// persists the vector into `skills.embedding` so the semantic recall leg
/// ([`embedding_search`]) can rank it; when `None`, behaviour is identical to
/// the pre-P0-1 hand-off (column stays NULL → FTS5-only). The caller is
/// responsible for embedding the de-PII'd [`embed_skill_text`] surface with a
/// configured provider; we never embed ciphertext (the `skills` row is
/// plaintext by SPEC-25 §13, so there is no seal to invert here).
///
/// ## P0-8 sealing interaction (owned-memory embeddings)
/// The `skills` table is plaintext-at-rest by SPEC-25 §13 (only the cross-peer
/// `EncryptedSkillEnvelope` is age-encrypted), so skill embeddings are computed
/// from the de-PII'd `embed_skill_text` surface with NO seal to invert. If a
/// future change embeds the SEALED `hermes_memory` store instead (P0-8:
/// `core/src/skillbank/memory_seal.rs` seals `text`/`source`), the embedding MUST
/// be computed from the plaintext (or the `fts_index_form` de-PII'd token form)
/// INSIDE `SkillMemory::insert` BEFORE the `seal()` call — never from the
/// sealed blob (embedding ciphertext yields meaningless cosine) and never from
/// raw PII (that would re-leak plaintext into a derived index, the same hazard
/// 0010 addresses for FTS5). This is a note only; no `skillbank/memory.rs` change
/// is in P0-1's scope.
pub fn handoff_extracted_skill_with_embedding(skill: Skill, embedding: Option<Vec<f32>>) {
    handoff_queue()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push_back((skill, embedding));
}

/// Test-only: clear the hand-off queue so unit tests don't bleed state into
/// one another (the queue is a process-global `static`). Not part of the
/// public hand-off contract.
#[cfg(test)]
fn clear_handoff_queue() {
    handoff_queue()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
}

/// Apply-on-write schema for the skills store. Bundled so a fresh DB
/// self-provisions on the first `store_skill` (mirrors `skillbank::memory`'s 0007).
const SKILLS_SCHEMA: &str = include_str!("../migrations/0008_hermes_skills.sql");

/// Apply-on-write migration that adds the nullable `embedding BLOB` column to
/// `skills` (SPEC-25 §8.4 semantic-recall vector). Bundled like
/// [`SKILLS_SCHEMA`] so a DB self-provisions the column on first write. The
/// bare `ALTER TABLE` inside is NOT idempotent on its own — it is applied via
/// the tolerant [`apply_embedding_column`] runner, never raw
/// `execute_batch`, so a re-apply on a DB that already has the column is a
/// no-op instead of a "duplicate column name" abort.
const EMBEDDINGS_SCHEMA: &str = include_str!("../migrations/0009_skill_embeddings.sql");

/// Apply-on-write migration that retires the 0008 `skills_ai`/`skills_au`/
/// `skills_ad` auto-sync triggers so the Rust write path controls the
/// `skills_fts` feed (P0-2; the `skills`-table analogue of 0010 for
/// `hermes_memory`). Idempotent (`DROP TRIGGER IF EXISTS`) — bundled into the
/// self-provision batch so a DB created before this migration simply loses the
/// auto-triggers and [`fts_feed`] takes over.
const SKILLS_FTS_FORM_SCHEMA: &str =
    include_str!("../migrations/0011_skills_fts_index_form.sql");

// ─── §8.4 Embedding provider + cosine similarity ─────────────────────────────

/// Pluggable text → vector embedder for the recall semantic leg (SPEC-25
/// §8.4). The production impl will wrap a local `ort` MiniLM runtime (still
/// Stage 4 — crate not in deps); decoupling it behind a trait lets the store
/// + recall paths persist and compare vectors today without pulling the ONNX
/// runtime, and lets tests substitute a deterministic fixture. `embed` is the
/// only required op: map `text` to a fixed-dimension `Vec<f32>` (the caller is
/// responsible for using one provider consistently — mixing dimensions is a
/// caller error surfaced as a non-match, not a panic, by [`cosine`]).
pub trait EmbeddingProvider {
    /// Embed `text` into a fixed-dimension vector. Errors map to the SPEC-25
    /// §11.1 catalog (typically `EmbeddingTimeout` for a slow provider).
    fn embed(&self, text: &str) -> Result<Vec<f32>, SkillError>;
}

// ─── M4 — production `ApiEmbedder` (OFF by default) ──────────────────────────
//
// `ApiEmbedder` is the FIRST real (online) `EmbeddingProvider`: it embeds text
// by POSTing to an OpenAI-compatible `/v1/embeddings` endpoint (default model
// `text-embedding-3-small`) and decoding the returned float vector. It reuses
// the EXISTING async `reqwest` 0.12 stack via `providers_wire::block_on_async`
// (the same sync→async bridge `providers_wire::complete` uses), so no new HTTP
// dependency or cargo feature is introduced.
//
// **Default-OFF invariant (the load-bearing M4 guarantee).** The shipping lib
// build installs NO embedder: [`resolve_production_embedder`] returns `None`
// unless BOTH (a) `SPECTYN_EMBED_PROVIDER` is set to a non-blank value AND
// (b) an API key resolves from the conventional env var. With neither set,
// `embedding_search` still returns `Err(())` and `recall_skills` degrades to
// FTS5-only — behavior is byte-for-byte unchanged from M3. The config + key
// are the ONLY switches that turn the semantic leg online.
//
// **Secret hygiene.** The resolved API key is held in the struct and sent ONLY
// as the `Authorization: Bearer` header. It is NEVER written to a log, an error
// message, or the `Debug` output — `ApiEmbedder` deliberately does NOT derive
// `Debug` over the key, and every `SkillError` this module produces carries the
// endpoint / model / status, never the key.

/// Default OpenAI-compatible embeddings endpoint.
const DEFAULT_EMBED_ENDPOINT: &str = "https://api.openai.com/v1/embeddings";
/// Default embedding model — the slug the `skill_wire` comments already name.
const DEFAULT_EMBED_MODEL: &str = "text-embedding-3-small";
/// Embedding HTTP budget. The SPEC-25 §8.4 recall leg is latency-sensitive
/// (it runs inline before the agent reply); a slow provider must time out and
/// degrade to FTS5-only rather than stall the runtime.
const EMBED_TIMEOUT_MS: u64 = 8_000;

/// Production online embedder backed by an OpenAI-compatible `/v1/embeddings`
/// API. Constructed only through [`ApiEmbedder::from_env`], which returns
/// `None` unless the operator has explicitly configured a provider AND a key
/// is present — see the module note above for the default-OFF invariant.
///
/// Intentionally NOT `#[derive(Debug)]`: the struct owns the API key and must
/// never leak it through a `{:?}` log line.
pub struct ApiEmbedder {
    endpoint: String,
    model: String,
    api_key: String,
}

impl ApiEmbedder {
    /// Build an `ApiEmbedder` from explicit parts. Public for testing the pure
    /// request-shaping / response-parsing path without touching the
    /// environment; production code goes through [`from_env`](Self::from_env).
    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        ApiEmbedder {
            endpoint: endpoint.into(),
            model: model.into(),
            api_key: api_key.into(),
        }
    }

    /// Resolve an `ApiEmbedder` from the environment, or `None` when the
    /// embedding leg is not configured (the default, shipping state).
    ///
    /// Switches (ALL must be satisfied to return `Some`):
    /// 1. `SPECTYN_EMBED_PROVIDER` — non-blank provider slug (e.g. `openai`).
    ///    Absent / blank ⇒ `None` ⇒ recall stays FTS5-only. This is the master
    ///    on/off switch.
    /// 2. An API key resolves for that slug (see [`resolve_embed_key`]). No key
    ///    ⇒ `None` ⇒ recall stays FTS5-only.
    ///
    /// Optional overrides (only consulted once the provider is configured):
    /// - `SPECTYN_EMBED_MODEL`   — defaults to `text-embedding-3-small`.
    /// - `SPECTYN_EMBED_BASE_URL`— full endpoint URL; defaults to the OpenAI
    ///   `/v1/embeddings` route. Set this to point at a compatible proxy or a
    ///   self-hosted gateway.
    pub fn from_env() -> Option<Self> {
        // (1) master switch — a non-blank provider slug is REQUIRED.
        let provider = std::env::var("SPECTYN_EMBED_PROVIDER")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())?;

        // (2) key — no key ⇒ stay offline (FTS5-only).
        let api_key = resolve_embed_key(&provider)?;

        let endpoint = std::env::var("SPECTYN_EMBED_BASE_URL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_EMBED_ENDPOINT.to_string());

        let model = std::env::var("SPECTYN_EMBED_MODEL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_EMBED_MODEL.to_string());

        Some(ApiEmbedder::new(endpoint, model, api_key))
    }

    /// The OpenAI-compatible request body: `{"model": ..., "input": <text>}`.
    /// Pure fn — no key, no I/O — so request shaping is unit-testable. The key
    /// travels in the `Authorization` header, never the body.
    fn build_request_body(&self, text: &str) -> serde_json::Value {
        serde_json::json!({
            "model": self.model,
            "input": text,
        })
    }

    /// Parse an OpenAI-compatible embeddings response body into the first
    /// embedding vector. Pure fn — unit-testable without network. Maps a
    /// missing / malformed `data[0].embedding` array to
    /// `SkillError::EmbeddingTimeout` (the §11.1 catalog's embedding-failure
    /// variant; `provider` carries the model, never the key).
    fn parse_embedding_response(&self, body: &str) -> Result<Vec<f32>, SkillError> {
        let v: serde_json::Value = serde_json::from_str(body).map_err(|e| {
            SkillError::EmbeddingTimeout {
                provider: format!("{}: response not JSON: {e}", self.model),
            }
        })?;
        // OpenAI shape: { "data": [ { "embedding": [f32, ...] }, ... ] }
        let arr = v
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|e| e.get("embedding"))
            .and_then(|e| e.as_array())
            .ok_or_else(|| SkillError::EmbeddingTimeout {
                provider: format!("{}: no data[0].embedding in response", self.model),
            })?;
        let mut out = Vec::with_capacity(arr.len());
        for n in arr {
            let f = n.as_f64().ok_or_else(|| SkillError::EmbeddingTimeout {
                provider: format!("{}: non-numeric value in embedding", self.model),
            })?;
            out.push(f as f32);
        }
        if out.is_empty() {
            return Err(SkillError::EmbeddingTimeout {
                provider: format!("{}: empty embedding vector", self.model),
            });
        }
        Ok(out)
    }
}

impl EmbeddingProvider for ApiEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, SkillError> {
        let body = self.build_request_body(text);
        // Reuse the existing async reqwest stack through providers_wire's
        // sync→async bridge — no new HTTP dependency, and it works whether or
        // not we're already inside a tokio runtime (agent hot path) or a plain
        // sync context (CLI / tests).
        let endpoint = self.endpoint.clone();
        let api_key = self.api_key.clone();
        let model = self.model.clone();
        let raw: Result<String, SkillError> =
            crate::providers_wire::block_on_async(async move {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_millis(EMBED_TIMEOUT_MS))
                    .build()
                    .map_err(|e| SkillError::EmbeddingTimeout {
                        // never includes the key
                        provider: format!("{model}: http client build: {e}"),
                    })?;
                let resp = client
                    .post(&endpoint)
                    .header("Authorization", format!("Bearer {api_key}"))
                    .header("Content-Type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| SkillError::EmbeddingTimeout {
                        // reqwest's Display never echoes request headers, so the
                        // key cannot leak here.
                        provider: format!("{model}: transport error: {e}"),
                    })?;
                let status = resp.status();
                let text = resp.text().await.map_err(|e| {
                    SkillError::EmbeddingTimeout {
                        provider: format!("{model}: reading body: {e}"),
                    }
                })?;
                if !status.is_success() {
                    // Surface ONLY the model + status code. We deliberately do
                    // NOT echo the response body: a misbehaving proxy could in
                    // theory reflect request headers (including the bearer key)
                    // into its error payload, and the "key never leaks" invariant
                    // outranks the marginal debuggability of a body snippet.
                    return Err(SkillError::EmbeddingTimeout {
                        provider: format!("{model}: HTTP {}", status.as_u16()),
                    });
                }
                Ok(text)
            });
        let raw = raw?;
        self.parse_embedding_response(&raw)
    }
}

/// Resolve an API key for the embedding `provider` slug, or `None` when no key
/// is present (which keeps the embedding leg OFF). Tries, in order:
/// 1. `SPECTYN_EMBED_API_KEY` — the embedding-specific override (lets the
///    embedder use a different key than the chat provider);
/// 2. `SPECTYN_MESH_<PROVIDER>_API_KEY` — the test/CLI-friendly namespaced var
///    (same convention as `providers_wire::resolve_api_key`);
/// 3. `<PROVIDER>_API_KEY` — the conventional dotfile var (e.g.
///    `OPENAI_API_KEY`).
///
/// Returns the key string itself — NEVER logged by any caller.
fn resolve_embed_key(provider: &str) -> Option<String> {
    let upper = provider.to_ascii_uppercase().replace('-', "_");
    let candidates = [
        "SPECTYN_EMBED_API_KEY".to_string(),
        format!("SPECTYN_MESH_{upper}_API_KEY"),
        format!("{upper}_API_KEY"),
    ];
    for k in &candidates {
        if let Ok(v) = std::env::var(k) {
            let v = v.trim().to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// Cosine similarity between two equal-length vectors. Pure fn — no I/O, no
/// allocation. Returns the dot product over the product of the L2 norms,
/// clamped into `[-1.0, 1.0]` to absorb floating-point drift. Returns `0.0`
/// (orthogonal / "no signal") rather than `NaN`/panicking when either vector
/// is empty, the lengths differ, or either norm is zero — recall must never
/// crash the agent over a malformed vector (matches the de-panic floor the
/// `embedding_search` leg already follows).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    (dot / (na.sqrt() * nb.sqrt())).clamp(-1.0, 1.0)
}

/// The canonical text that represents a `Skill` for embedding (SPEC-25 §8.4).
/// Store and recall MUST agree on this so a stored vector is comparable to a
/// query vector embedded from the same surface. We embed `name` + `trigger_pattern`
/// only — the human-meaningful recall signal — and deliberately EXCLUDE `steps`
/// and `examples`: examples carry (already-redacted) snippets that are not part
/// of the trigger signal, and including them would dilute cosine similarity.
/// Pure fn, no I/O.
pub fn embed_skill_text(skill: &Skill) -> String {
    format!("{} {}", skill.name, skill.trigger_pattern)
}

/// Serialize a `Vec<f32>` embedding to a little-endian byte BLOB for the
/// `skills.embedding` column. 4 bytes per element; an empty vec yields an
/// empty BLOB. Pure inverse of [`blob_to_embedding`].
fn embedding_to_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Deserialize a little-endian byte BLOB back into a `Vec<f32>`. Returns
/// `None` when the byte length is not a multiple of 4 (corrupt row) so the
/// caller can degrade to FTS5-only rather than panic. An empty BLOB maps to
/// an empty vec (a valid "stored but no signal" state).
///
/// This is the read side of the embedding column — the inverse of
/// [`embedding_to_blob`]. The Stage-4 `embedding_search` cosine leg will read
/// the column through it; today only the round-trip tests exercise it, so it
/// is `#[allow(dead_code)]` in the non-test lib build rather than deleted (the
/// write side is already live via `store_skill_with_embedding`).
#[allow(dead_code)]
fn blob_to_embedding(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Some(out)
}

/// Idempotently add the `embedding BLOB` column to `skills` (0009 migration).
/// SQLite lacks `ADD COLUMN IF NOT EXISTS` and `execute_batch` aborts the
/// whole batch on the first error, so applying the raw `ALTER` on a DB that
/// already has the column would fail. This runner probes
/// `PRAGMA table_info(skills)` and only runs the ALTER when the column is
/// absent — making re-application (every `store_skill` write) a clean no-op.
/// Assumes the 0008 base table already exists (caller applies `SKILLS_SCHEMA`
/// first).
fn apply_embedding_column(conn: &rusqlite::Connection) -> Result<(), SkillError> {
    let has_col = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(skills)")
            .map_err(|e| SkillError::StoreFailed { detail: format!("pragma prepare: {e}") })?;
        let mut rows = stmt
            .query([])
            .map_err(|e| SkillError::StoreFailed { detail: format!("pragma query: {e}") })?;
        let mut found = false;
        while let Some(row) = rows
            .next()
            .map_err(|e| SkillError::StoreFailed { detail: format!("pragma row: {e}") })?
        {
            // PRAGMA table_info columns: (cid, name, type, notnull, dflt, pk).
            let name: String = row
                .get(1)
                .map_err(|e| SkillError::StoreFailed { detail: format!("pragma name: {e}") })?;
            if name == "embedding" {
                found = true;
                break;
            }
        }
        found
    };
    if !has_col {
        conn.execute_batch(EMBEDDINGS_SCHEMA)
            .map_err(|e| SkillError::StoreFailed { detail: format!("apply 0009 schema: {e}") })?;
    }
    Ok(())
}

/// Durably upsert a skill into the `skills` table together with an OPTIONAL
/// embedding vector. Behaves exactly like [`store_skill`] when `embedding` is
/// `None` (the column is left NULL — current behavior preserved). When
/// `Some(vec)` is supplied the vector is serialized to a little-endian BLOB
/// (see [`embedding_to_blob`]) and written to the `skills.embedding` column so
/// the recall semantic leg can later read it back. Self-provisions both the
/// 0008 base schema and the 0009 embedding column on first write.
pub fn store_skill_with_embedding(
    skill: &Skill,
    embedding: Option<&[f32]>,
) -> Result<(), SkillError> {
    use rusqlite::{params, Connection};
    let conn = Connection::open(resolve_db_path())
        .map_err(|e| SkillError::StoreFailed { detail: format!("open db: {e}") })?;
    conn.execute_batch(SKILLS_SCHEMA)
        .map_err(|e| SkillError::StoreFailed { detail: format!("apply 0008 schema: {e}") })?;
    apply_embedding_column(&conn)?;
    // P0-2: retire the 0008 skills_ai/au/ad auto-sync triggers (idempotent) so
    // the FTS feed is Rust-controlled — required for P0-8 sealing, where the
    // canonical columns hold ciphertext but the FTS index must hold the de-PII'd
    // token form. On the default-OFF path the explicit feed writes the verbatim
    // values, byte-identical to the old trigger behaviour.
    conn.execute_batch(SKILLS_FTS_FORM_SCHEMA)
        .map_err(|e| SkillError::StoreFailed { detail: format!("apply 0011 schema: {e}") })?;

    let steps_json = serde_json::to_string(&skill.steps).unwrap_or_else(|_| "[]".into());
    let examples_json = serde_json::to_string(&skill.examples).unwrap_or_else(|_| "[]".into());
    let blob: Option<Vec<u8>> = embedding.map(embedding_to_blob);

    // P0-8 sealing: when SPECTYN_ENCRYPT_MEMORY is ON, seal the searchable
    // columns at rest and feed the FTS index the de-PII'd token form (NOT the
    // ciphertext, NOT the raw sentence). Fail CLOSED — seal() Err(NoKey) ⇒
    // StoreFailed, never a silent plaintext write. When OFF, every value is the
    // verbatim input, so the row + FTS feed are byte-identical to the
    // pre-sealing trigger behaviour. steps_json/examples_json carry redacted
    // snippets but are belt-and-braces sealed for parity; they are NOT FTS
    // columns so no token form is needed. (Mirrors skillbank/memory.rs::insert.)
    let sealing = crate::skillbank::memory_seal::memory_e2ee_enabled();
    let (stored_name, stored_trigger, stored_steps, stored_examples, fts_name, fts_trigger) =
        if sealing {
            use crate::skillbank::memory_seal::{fts_index_form, seal};
            (
                seal(&skill.name)
                    .map_err(|e| SkillError::StoreFailed { detail: format!("seal name: {e}") })?,
                seal(&skill.trigger_pattern).map_err(|e| SkillError::StoreFailed {
                    detail: format!("seal trigger: {e}"),
                })?,
                seal(&steps_json)
                    .map_err(|e| SkillError::StoreFailed { detail: format!("seal steps: {e}") })?,
                seal(&examples_json).map_err(|e| SkillError::StoreFailed {
                    detail: format!("seal examples: {e}"),
                })?,
                fts_index_form(&skill.name),
                fts_index_form(&skill.trigger_pattern),
            )
        } else {
            (
                skill.name.clone(),
                skill.trigger_pattern.clone(),
                steps_json.clone(),
                examples_json.clone(),
                skill.name.clone(),
                skill.trigger_pattern.clone(),
            )
        };

    // Capture the PRIOR indexed FTS values (if the row already exists) BEFORE the
    // upsert so the FTS feed can purge the old entry on an update — exactly the
    // retired skills_au delete-old-then-insert-new semantics, now in Rust. The
    // prior values are recomputed from the STORED columns (decrypting + re-token
    // -izing when sealed) so they match what was originally indexed, regardless
    // of whether the flag was toggled since.
    let prior = fts_prior_indexed(&conn, &skill.id)?;

    conn.execute(
        "INSERT INTO skills \
         (id, name, trigger_pattern, steps_json, examples_json, version, \
          quality_score, last_applied_at, source_event_count, embedding) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
         ON CONFLICT(id) DO UPDATE SET \
           name=excluded.name, trigger_pattern=excluded.trigger_pattern, \
           steps_json=excluded.steps_json, examples_json=excluded.examples_json, \
           version=excluded.version, quality_score=excluded.quality_score, \
           last_applied_at=excluded.last_applied_at, \
           source_event_count=excluded.source_event_count, \
           embedding=excluded.embedding",
        params![
            skill.id,
            stored_name,
            stored_trigger,
            stored_steps,
            stored_examples,
            skill.version as i64,
            skill.quality_score as f64,
            skill.last_applied_at as i64,
            skill.source_event_count as i64,
            blob,
        ],
    )
    .map_err(|e| SkillError::StoreFailed { detail: format!("insert skill: {e}") })?;

    // The 0008 auto-triggers are gone, so feed skills_fts explicitly: purge the
    // prior indexed values (update branch) then insert the new token form.
    let rowid: i64 = conn
        .query_row(
            "SELECT rowid FROM skills WHERE id = ?1",
            params![skill.id],
            |r| r.get(0),
        )
        .map_err(|e| SkillError::StoreFailed { detail: format!("lookup rowid: {e}") })?;
    fts_feed(&conn, rowid, &fts_name, &fts_trigger, prior.as_ref())?;
    Ok(())
}

/// Recover the FTS-indexed `(name, trigger_pattern)` for an EXISTING `skills`
/// row so an update can purge the prior entry before re-indexing — the
/// delete-half of the retired `skills_au` trigger, now in Rust. Returns `None`
/// when no row exists yet (a pure insert needs no purge). When the stored
/// columns are sealed they are decrypted and re-tokenized through
/// `fts_index_form` so the recovered values match EXACTLY what was originally
/// indexed (keys off `is_sealed(stored)`, NOT the current flag — the flag may
/// have toggled since the row was written; mirrors `memory.rs::delete_by_id`).
fn fts_prior_indexed(
    conn: &rusqlite::Connection,
    skill_id: &str,
) -> Result<Option<(String, String)>, SkillError> {
    use crate::skillbank::memory_seal::{fts_index_form, is_sealed, open};
    use rusqlite::OptionalExtension;
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT name, trigger_pattern FROM skills WHERE id = ?1",
            rusqlite::params![skill_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| SkillError::StoreFailed { detail: format!("lookup prior fts: {e}") })?;
    let Some((stored_name, stored_trigger)) = row else {
        return Ok(None);
    };
    let recover = |stored: String| -> Result<String, SkillError> {
        if is_sealed(&stored) {
            let plain = open(&stored).map_err(|e| SkillError::StoreFailed {
                detail: format!("decrypt prior fts: {e}"),
            })?;
            Ok(fts_index_form(&plain))
        } else {
            Ok(stored)
        }
    };
    Ok(Some((recover(stored_name)?, recover(stored_trigger)?)))
}

/// Feed the external-content `skills_fts` index explicitly (the 0008 auto-sync
/// triggers were retired by 0011). When `prior` is `Some`, this is an UPDATE:
/// issue the FTS5 `'delete'` for the previously-indexed values FIRST so no
/// orphan survives, then insert the new `name`/`trigger`. When `prior` is
/// `None` it is a pure insert. Single code path for insert + update so the two
/// stay in lock-step (parallels `skillbank/memory.rs`'s explicit FTS feed).
fn fts_feed(
    conn: &rusqlite::Connection,
    rowid: i64,
    name: &str,
    trigger: &str,
    prior: Option<&(String, String)>,
) -> Result<(), SkillError> {
    use rusqlite::params;
    if let Some((old_name, old_trigger)) = prior {
        conn.execute(
            "INSERT INTO skills_fts(skills_fts, rowid, name, trigger_pattern) \
             VALUES ('delete', ?1, ?2, ?3)",
            params![rowid, old_name, old_trigger],
        )
        .map_err(|e| SkillError::StoreFailed { detail: format!("fts purge: {e}") })?;
    }
    conn.execute(
        "INSERT INTO skills_fts(rowid, name, trigger_pattern) VALUES (?1, ?2, ?3)",
        params![rowid, name, trigger],
    )
    .map_err(|e| SkillError::StoreFailed { detail: format!("fts insert: {e}") })?;
    Ok(())
}

/// Durably upsert a skill into the SPEC-25 `skills` table so it becomes
/// FTS5-recallable via [`recall_skills`]. Self-provisions the 0008 schema on
/// first write. This is the real Store path the scheduler hand-off will call
/// once it threads the extracted `Skill` payload through; the parameterless
/// [`skill_store`] dispatch stub returns a typed error until then.
///
/// The row is plaintext-searchable for FTS5 (SPEC-25 §13: age-encryption wraps
/// only the cross-peer `EncryptedSkillEnvelope`, not the local row). Upsert by
/// `id` so re-extracting a skill updates it (the `skills_au` trigger keeps the
/// FTS index in sync).
pub fn store_skill(skill: &Skill) -> Result<(), SkillError> {
    // Single write path: storing without an embedding leaves the 0009
    // `embedding` column NULL (current behavior). The optional-embedding
    // variant carries the full INSERT so the two stay in lock-step.
    store_skill_with_embedding(skill, None)
}

/// Read the stored `version` of a skill by `id` — the last-writer-wins input for
/// cross-peer sync ([`crate::skillbank::sync::merge_decision`]). `Ok(None)` when
/// no row (or no `skills` table yet) exists: a never-seen skill, which is the
/// "accept" case. Mirrors the missing-table happy path of [`skill_list`] /
/// [`skill_stats`] (a fresh install before the 0008 migration ⇒ `None`, not an
/// error).
pub fn skill_version(id: &str) -> Result<Option<u16>, SkillError> {
    use rusqlite::{Connection, OptionalExtension};

    let path = resolve_db_path();
    let conn = Connection::open(&path).map_err(|_| SkillError::StoreFull)?;

    // Missing table ⇒ prepare Errs ⇒ None (nothing stored yet).
    let mut stmt = match conn.prepare("SELECT version FROM skills WHERE id = ?1") {
        Ok(s) => s,
        Err(_) => return Ok(None),
    };
    let v: Option<i64> = stmt
        .query_row(rusqlite::params![id], |r| r.get(0))
        .optional()
        .map_err(|e| SkillError::StoreFailed { detail: format!("skill_version: {e}") })?;
    // Checked, not `as u16`: an out-of-range stored value must NOT silently wrap
    // (e.g. 65536 → 0), which would corrupt the cross-peer LWW comparison and let
    // an older skill overwrite a newer local one. A bad row is a store error.
    v.map(u16::try_from)
        .transpose()
        .map_err(|e| SkillError::StoreFailed { detail: format!("skill_version out of range: {e}") })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC-25 §8.6: `record_measure` adjusts a skill's `quality_score` by the
    /// feedback weights (applied +0.05 / declined −0.10 / edited +0.02, clamped
    /// 0..1) and stamps `last_applied_at = observed_at`. Verified by a REAL
    /// store → measure → reload round-trip on the sqlite store, so a no-op /
    /// stubbed `record_measure` would fail this. (Round-1 build-now item; the
    /// function was REAL but had no behavioral test — DRIFT note 2026-06-26.)
    #[test]
    fn record_measure_adjusts_quality_and_stamps_time() {
        let _g = crate::env_lock::acquire();
        let saved_db = std::env::var_os("SPECTYN_DB_PATH");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());

        let base = Skill {
            id: "measure-rt".into(),
            name: "test skill".into(),
            trigger_pattern: "t".into(),
            steps: vec!["s".into()],
            examples: vec![],
            version: 1,
            quality_score: 0.50,
            last_applied_at: 0,
            source_event_count: 1,
        };
        store_skill(&base).expect("store");

        // applied + edited → +0.05 +0.02 = +0.07 ; stamps observed_at.
        record_measure(MeasureFeedback {
            skill_id: "measure-rt".into(),
            was_applied: true,
            was_decline: false,
            user_edited: true,
            observed_at: 1_716_000_000_000,
        })
        .expect("measure (applied+edited)");
        let after = skill_load("measure-rt").expect("reload");

        // declined → −0.10 (0.57 → 0.47), independent second call.
        record_measure(MeasureFeedback {
            skill_id: "measure-rt".into(),
            was_applied: false,
            was_decline: true,
            user_edited: false,
            observed_at: 1_716_000_111_000,
        })
        .expect("measure (declined)");
        let after2 = skill_load("measure-rt").expect("reload 2");

        // restore env BEFORE asserting so a failed assert can't leak state.
        match saved_db {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }

        assert!(
            (after.quality_score - 0.57).abs() < 1e-4,
            "0.50 +0.07 = 0.57, got {}",
            after.quality_score
        );
        assert_eq!(after.last_applied_at, 1_716_000_000_000, "observed_at stamped");
        assert!(
            (after2.quality_score - 0.47).abs() < 1e-4,
            "0.57 −0.10 = 0.47, got {}",
            after2.quality_score
        );
        assert_eq!(after2.last_applied_at, 1_716_000_111_000, "second stamp");
    }

    // ─── apex ② owned-memory live-wiring (slice 1) ──────────────────────────

    /// Task 1: `owned_memory_enabled` defaults ON and only the explicit off
    /// tokens (`0/false/off/no`, case-insensitive, trimmed) flip it OFF.
    #[test]
    fn owned_memory_enabled_defaults_on_and_respects_killswitch() {
        let _g = crate::env_lock::acquire();
        let saved = std::env::var_os("SPECTYN_OWNED_MEMORY");

        std::env::remove_var("SPECTYN_OWNED_MEMORY");
        assert!(owned_memory_enabled(), "unset ⇒ default ON");

        for off in ["0", "false", "off", "no", "FALSE", "Off", " false ", "NO"] {
            std::env::set_var("SPECTYN_OWNED_MEMORY", off);
            assert!(!owned_memory_enabled(), "{off:?} ⇒ OFF");
        }
        for on in ["1", "true", "yes", "", "anything"] {
            std::env::set_var("SPECTYN_OWNED_MEMORY", on);
            assert!(owned_memory_enabled(), "{on:?} ⇒ ON");
        }

        match saved {
            Some(v) => std::env::set_var("SPECTYN_OWNED_MEMORY", v),
            None => std::env::remove_var("SPECTYN_OWNED_MEMORY"),
        }
    }

    /// Task 2: `owned_memory_system_block` recalls the relevant skill (NOT the
    /// distractor), renders a `<recalled_skills>` block, returns `""` under the
    /// kill-switch, and returns `""` for an unrelated query.
    #[test]
    fn owned_memory_system_block_recalls_relevant_and_respects_killswitch() {
        let _g = crate::env_lock::acquire();
        let saved_db = std::env::var_os("SPECTYN_DB_PATH");
        let saved_om = std::env::var_os("SPECTYN_OWNED_MEMORY");
        std::env::remove_var("SPECTYN_OWNED_MEMORY"); // default ON
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());
        clear_handoff_queue();

        let relevant = Skill {
            id: "om-block-deploy".into(),
            name: "deploy the staging cluster".into(),
            trigger_pattern: "deploy staging".into(),
            steps: vec!["ssh staging".into(), "run deploy.sh".into()],
            examples: vec![],
            version: 1,
            quality_score: 0.9,
            last_applied_at: 0,
            source_event_count: 3,
        };
        let distractor = Skill {
            id: "om-block-rotate".into(),
            name: "rotate the vault secrets".into(),
            trigger_pattern: "rotate secrets".into(),
            steps: vec![],
            examples: vec![],
            version: 1,
            quality_score: 0.9,
            last_applied_at: 0,
            source_event_count: 2,
        };
        store_skill(&relevant).expect("store relevant");
        store_skill(&distractor).expect("store distractor");

        let block = owned_memory_system_block("deploy staging now");
        // kill-switch OFF path
        std::env::set_var("SPECTYN_OWNED_MEMORY", "0");
        let killed = owned_memory_system_block("deploy staging now");
        std::env::remove_var("SPECTYN_OWNED_MEMORY");
        // unrelated query shares no token
        let unrelated = owned_memory_system_block("compile the kernel");

        match saved_db {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }
        match saved_om {
            Some(v) => std::env::set_var("SPECTYN_OWNED_MEMORY", v),
            None => std::env::remove_var("SPECTYN_OWNED_MEMORY"),
        }

        assert!(
            block.contains("<recalled_skills>"),
            "block must carry the XML wrapper: {block:?}"
        );
        assert!(
            block.contains("deploy the staging cluster"),
            "block must contain the relevant skill: {block:?}"
        );
        assert!(
            !block.contains("rotate the vault secrets"),
            "block must NOT contain the distractor: {block:?}"
        );
        assert!(killed.is_empty(), "kill-switch ⇒ empty, got {killed:?}");
        assert!(
            unrelated.is_empty(),
            "unrelated query ⇒ empty, got {unrelated:?}"
        );
    }

    /// Task 2 (2nd): a low-quality (`0.1`) but keyword-matching skill is filtered
    /// out by the `MIN_SKILL_QUALITY` gate even though FTS5 would match it.
    #[test]
    fn owned_memory_system_block_filters_low_quality_skill() {
        let _g = crate::env_lock::acquire();
        let saved_db = std::env::var_os("SPECTYN_DB_PATH");
        let saved_om = std::env::var_os("SPECTYN_OWNED_MEMORY");
        std::env::remove_var("SPECTYN_OWNED_MEMORY");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());
        clear_handoff_queue();

        let low_quality = Skill {
            id: "om-block-lowq".into(),
            name: "deploy the staging cluster".into(),
            trigger_pattern: "deploy staging".into(),
            steps: vec![],
            examples: vec![],
            version: 1,
            quality_score: 0.1, // below MIN_SKILL_QUALITY (0.30)
            last_applied_at: 0,
            source_event_count: 1,
        };
        store_skill(&low_quality).expect("store low-quality");

        let block = owned_memory_system_block("deploy staging now");

        match saved_db {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }
        match saved_om {
            Some(v) => std::env::set_var("SPECTYN_OWNED_MEMORY", v),
            None => std::env::remove_var("SPECTYN_OWNED_MEMORY"),
        }

        assert!(
            block.is_empty(),
            "a low-quality match must be filtered out ⇒ empty block, got {block:?}"
        );
    }

    /// Task 4: `capture_correction` enqueues exactly one candidate whose first
    /// step names the denied tool; the kill-switch suppresses the enqueue.
    #[test]
    fn capture_correction_enqueues_candidate_and_respects_killswitch() {
        let _g = crate::env_lock::acquire();
        let saved_om = std::env::var_os("SPECTYN_OWNED_MEMORY");

        // ON (default): one candidate enqueued.
        std::env::remove_var("SPECTYN_OWNED_MEMORY");
        clear_handoff_queue();
        capture_correction("force push to the main branch", "shell", "protected branch");
        let len_on = handoff_queue()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len();
        let first_step_has_tool = handoff_queue()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .front()
            .map(|(s, _)| s.steps.first().is_some_and(|st| st.contains("shell")))
            .unwrap_or(false);

        // OFF: no candidate enqueued.
        std::env::set_var("SPECTYN_OWNED_MEMORY", "0");
        clear_handoff_queue();
        capture_correction("force push to the main branch", "shell", "protected branch");
        let len_off = handoff_queue()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len();

        clear_handoff_queue();
        match saved_om {
            Some(v) => std::env::set_var("SPECTYN_OWNED_MEMORY", v),
            None => std::env::remove_var("SPECTYN_OWNED_MEMORY"),
        }

        assert_eq!(len_on, 1, "ON ⇒ exactly one candidate enqueued");
        assert!(
            first_step_has_tool,
            "the candidate's first step must name the denied tool"
        );
        assert_eq!(len_off, 0, "kill-switch ⇒ nothing enqueued");
    }

    /// Task 1: `skill_list` enumerates every stored row, highest quality first
    /// (ties by id), projecting id + plaintext name + quality + last_applied.
    #[test]
    fn skill_list_enumerates_all_stored_rows_sorted_by_quality_desc() {
        let _g = crate::env_lock::acquire();
        let saved_db = std::env::var_os("SPECTYN_DB_PATH");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());
        clear_handoff_queue();

        let a = Skill {
            id: "sk-a".into(),
            name: "deploy the staging cluster".into(),
            trigger_pattern: "deploy staging".into(),
            steps: vec![],
            examples: vec![],
            version: 1,
            quality_score: 0.9,
            last_applied_at: 0,
            source_event_count: 1,
        };
        let b = Skill {
            id: "sk-b".into(),
            name: "rotate the vault secrets".into(),
            trigger_pattern: "rotate secrets".into(),
            steps: vec![],
            examples: vec![],
            version: 1,
            quality_score: 0.4,
            last_applied_at: 0,
            source_event_count: 1,
        };
        store_skill(&a).expect("store sk-a");
        store_skill(&b).expect("store sk-b");

        let rows = skill_list().expect("skill_list");

        match saved_db {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }

        assert_eq!(rows.len(), 2, "both stored skills enumerated: {rows:?}");
        assert_eq!(rows[0].id, "sk-a", "highest quality first");
        assert_eq!(rows[1].id, "sk-b", "lower quality second");
    }

    /// Task 2: `skill_stats` partitions the bank by learned quality into
    /// high(≥0.70)/medium([0.30,0.70))/low(<0.30) buckets in one aggregate query.
    #[test]
    fn skill_stats_buckets_by_quality_band() {
        let _g = crate::env_lock::acquire();
        let saved_db = std::env::var_os("SPECTYN_DB_PATH");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());
        clear_handoff_queue();

        for (id, q) in [("sk-hi", 0.85f32), ("sk-md", 0.55), ("sk-lo", 0.20)] {
            let s = Skill {
                id: id.into(),
                name: format!("name {id}"),
                trigger_pattern: format!("trig {id}"),
                steps: vec![],
                examples: vec![],
                version: 1,
                quality_score: q,
                last_applied_at: 0,
                source_event_count: 1,
            };
            store_skill(&s).unwrap_or_else(|e| panic!("store {id}: {e}"));
        }

        let stats = skill_stats().expect("skill_stats");

        match saved_db {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }

        assert_eq!(stats.total, 3, "total: {stats:?}");
        assert_eq!(stats.high, 1, "one high: {stats:?}");
        assert_eq!(stats.medium, 1, "one medium: {stats:?}");
        assert_eq!(stats.low, 1, "one low: {stats:?}");
    }

    /// Task 3a: one captured correction ⇒ `skill_learn_tick` stores it ⇒ 1.
    #[test]
    fn skill_learn_tick_stores_one_captured_correction() {
        let _g = crate::env_lock::acquire();
        let saved_db = std::env::var_os("SPECTYN_DB_PATH");
        let saved_om = std::env::var_os("SPECTYN_OWNED_MEMORY");
        std::env::set_var("SPECTYN_OWNED_MEMORY", "1");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());
        clear_handoff_queue();

        capture_correction("deploy prod cluster now", "bash", "use staging not prod");
        let stored = skill_learn_tick().expect("skill_learn_tick");

        match saved_db {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }
        match saved_om {
            Some(v) => std::env::set_var("SPECTYN_OWNED_MEMORY", v),
            None => std::env::remove_var("SPECTYN_OWNED_MEMORY"),
        }

        assert_eq!(stored, 1, "one captured correction must be stored");
    }

    /// Task 3b: an empty hand-off queue ⇒ `skill_learn_tick` returns Ok(0), NOT
    /// the `StoreFailed` error the defensive Store leg raises on an empty queue.
    #[test]
    fn skill_learn_tick_empty_queue_is_ok_zero() {
        let _g = crate::env_lock::acquire();
        let saved_db = std::env::var_os("SPECTYN_DB_PATH");
        let saved_om = std::env::var_os("SPECTYN_OWNED_MEMORY");
        std::env::set_var("SPECTYN_OWNED_MEMORY", "1");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());
        clear_handoff_queue();

        let stored = skill_learn_tick().expect("skill_learn_tick on empty queue is Ok");

        match saved_db {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }
        match saved_om {
            Some(v) => std::env::set_var("SPECTYN_OWNED_MEMORY", v),
            None => std::env::remove_var("SPECTYN_OWNED_MEMORY"),
        }

        assert_eq!(stored, 0, "empty queue ⇒ Ok(0), the empty-queue error swallowed");
    }

    /// Task B: PHASE-2b — the additive LLM learn leg must DEGRADE cleanly. With
    /// one captured correction and NO provider configured (default install),
    /// `skill_learn_tick` still returns 1: the drain leg stores the correction,
    /// the learn leg's `judge_candidates` fails with `JudgeFailed` (no provider)
    /// and is SWALLOWED into an info-log — it must NOT turn a good drain into a
    /// tick error. (Clones the env harness from
    /// `skill_learn_tick_stores_one_captured_correction`.)
    #[test]
    fn skill_learn_tick_degrades_to_drain_only_without_provider() {
        let _g = crate::env_lock::acquire();
        let saved_db = std::env::var_os("SPECTYN_DB_PATH");
        let saved_om = std::env::var_os("SPECTYN_OWNED_MEMORY");
        std::env::set_var("SPECTYN_OWNED_MEMORY", "1");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());
        clear_handoff_queue();

        capture_correction("deploy prod cluster now", "bash", "use staging not prod");
        let stored = skill_learn_tick().expect("skill_learn_tick must not error when judge fails");

        match saved_db {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }
        match saved_om {
            Some(v) => std::env::set_var("SPECTYN_OWNED_MEMORY", v),
            None => std::env::remove_var("SPECTYN_OWNED_MEMORY"),
        }

        assert_eq!(
            stored, 1,
            "drain stores the 1 correction; no-provider learn leg degrades (swallowed), not erroring the tick"
        );
    }

    #[test]
    fn skill_round_trip_smoke() {
        // §7 invariant: canonical `Skill` survives Rust → JSON → Rust
        // round-trip byte-identical. `app/src/lib/generated/skill/Skill.ts`
        // (ts-rs output) is UI-consumed; any field rename here = wire break.
        let s = Skill {
            id: "01923f8e-9b4c-7000-8c2d-2b9f0e1d4a55".to_string(),
            name: "commit message 格式偏好".to_string(),
            trigger_pattern: "當 user 問 commit format".to_string(),
            steps: vec![
                "先用 <type>(<scope>): <subject> 前綴".to_string(),
                "subject ≤ 50 字".to_string(),
            ],
            examples: vec![SkillExample {
                event_id_hash: "a3f9c2b1d4e57680".to_string(),
                redacted_snippet: "user asked about commit format prefix".to_string(),
            }],
            version: 2,
            quality_score: 0.85,
            last_applied_at: 1_716_563_400_000,
            source_event_count: 7,
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: Skill = serde_json::from_str(&j).unwrap();
        assert_eq!(s.id, back.id);
        assert_eq!(s.name, back.name);
        assert_eq!(s.trigger_pattern, back.trigger_pattern);
        assert_eq!(s.steps, back.steps);
        assert_eq!(s.version, back.version);
        assert!((s.quality_score - back.quality_score).abs() < f32::EPSILON);
        assert_eq!(s.last_applied_at, back.last_applied_at);
        assert_eq!(s.source_event_count, back.source_event_count);
        assert_eq!(s.examples.len(), back.examples.len());
        assert_eq!(s.examples[0].event_id_hash, back.examples[0].event_id_hash);
        assert_eq!(
            s.examples[0].redacted_snippet,
            back.examples[0].redacted_snippet
        );

        // Privacy invariant smoke: SkillExample MUST be the redacted shape
        // (event_id_hash + redacted_snippet); raw {event_id, snippet}
        // shape forbidden — catches the regression before it ships.
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        let ex = &v["examples"][0];
        assert!(
            ex.get("eventIdHash").is_some(),
            "SkillExample must carry eventIdHash (SPEC-25 §6/§13); got: {ex}"
        );
        assert!(
            ex.get("redactedSnippet").is_some(),
            "SkillExample must carry redactedSnippet (SPEC-25 §6/§13); got: {ex}"
        );
        assert!(
            ex.get("eventId").is_none(),
            "raw eventId field forbidden — privacy regression; got: {ex}"
        );
        assert!(
            ex.get("snippet").is_none(),
            "raw snippet field forbidden — privacy regression; got: {ex}"
        );
    }

    #[test]
    fn encrypted_skill_envelope_has_exactly_four_fields() {
        // §9.5 invariant: cross-peer sync envelope is EXACTLY 4 fields
        // (skill_id, version, ciphertext_b64, signature_hex). Any
        // addition = wire break + SPEC bump.
        let env = EncryptedSkillEnvelope {
            skill_id: "01923f8e-9b4c-7000-8c2d-2b9f0e1d4a55".to_string(),
            version: 3,
            ciphertext_b64: "YWdlLWVuY3J5cHRlZC1ibG9i".to_string(),
            signature_hex: "deadbeefcafef00d1234567890abcdef".to_string(),
        };
        let j = serde_json::to_string(&env).unwrap();
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        let obj = v.as_object().expect("envelope is JSON object");
        assert_eq!(
            obj.len(),
            4,
            "EncryptedSkillEnvelope must stay 4 fields (SPEC-25 §9.5); got keys: {:?}",
            obj.keys().collect::<Vec<_>>()
        );
        assert!(obj.contains_key("skillId"));
        assert!(obj.contains_key("version"));
        assert!(obj.contains_key("ciphertextB64"));
        assert!(obj.contains_key("signatureHex"));

        let back: EncryptedSkillEnvelope = serde_json::from_str(&j).unwrap();
        assert_eq!(env.skill_id, back.skill_id);
        assert_eq!(env.version, back.version);
        assert_eq!(env.ciphertext_b64, back.ciphertext_b64);
        assert_eq!(env.signature_hex, back.signature_hex);
    }

    #[test]
    fn skill_default_quality_score_is_neutral() {
        // §7.1: freshly-extracted skill before any measure run defaults to
        // 0.5 (neutral). Serde `default` fills it in when JSON omits it.
        let j = r#"{"id":"x","name":"n","triggerPattern":"t","steps":[],"examples":[],"version":1,"sourceEventCount":5}"#;
        let s: Skill = serde_json::from_str(j).unwrap();
        assert!((s.quality_score - 0.5).abs() < f32::EPSILON);
        assert_eq!(s.last_applied_at, 0);
    }

    #[test]
    fn skill_step_serializes_snake_case() {
        // §6.1 wire surface: step names must stay snake_case (CLI / UI
        // dispatch on these strings).
        let cases = [
            (SkillStep::Judge, "\"judge\""),
            (SkillStep::Extract, "\"extract\""),
            (SkillStep::Store, "\"store\""),
            (SkillStep::Recall, "\"recall\""),
            (SkillStep::Apply, "\"apply\""),
            (SkillStep::Measure, "\"measure\""),
        ];
        for (variant, expected) in cases {
            let j = serde_json::to_string(&variant).unwrap();
            assert_eq!(j, expected, "SkillStep::{:?} -> wire shape", variant);
        }
    }

    #[test]
    fn recall_strategy_and_skill_source_serialize_snake_case() {
        assert_eq!(
            serde_json::to_string(&RecallStrategy::HybridUnion).unwrap(),
            "\"hybrid_union\""
        );
        assert_eq!(
            serde_json::to_string(&RecallStrategy::Fts5Only).unwrap(),
            "\"fts5_only\""
        );
        assert_eq!(
            serde_json::to_string(&RecallStrategy::EmbeddingOnly).unwrap(),
            "\"embedding_only\""
        );
        assert_eq!(
            serde_json::to_string(&RecallStrategy::HybridIntersect).unwrap(),
            "\"hybrid_intersect\""
        );
        assert_eq!(
            serde_json::to_string(&SkillSource::LlmExtracted).unwrap(),
            "\"llm_extracted\""
        );
        assert_eq!(
            serde_json::to_string(&SkillSource::UserDefined).unwrap(),
            "\"user_defined\""
        );
        assert_eq!(
            serde_json::to_string(&SkillSource::Imported).unwrap(),
            "\"imported\""
        );
    }

    #[test]
    fn skill_error_serializes_with_code_tag() {
        // §11.1: error wire shape uses `{"code": "..."}` tag so UI can
        // dispatch on machine-readable code string.
        let e = SkillError::StoreFull;
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("store_full"), "wire shape: {}", j);

        let e2 = SkillError::JudgeFailed {
            detail: "timeout".to_string(),
        };
        let j2 = serde_json::to_string(&e2).unwrap();
        assert!(j2.contains("judge_failed"), "wire shape: {}", j2);
        assert!(j2.contains("timeout"), "detail preserved: {}", j2);

        let e3 = SkillError::EmbeddingTimeout {
            provider: "openai-text-embedding-3-small".to_string(),
        };
        let j3 = serde_json::to_string(&e3).unwrap();
        assert!(j3.contains("embedding_timeout"), "wire shape: {}", j3);

        assert!(serde_json::to_string(&SkillError::SyncSignatureBad)
            .unwrap()
            .contains("sync_signature_bad"));
        assert!(serde_json::to_string(&SkillError::RecallEmpty)
            .unwrap()
            .contains("recall_empty"));
        assert!(serde_json::to_string(&SkillError::ExtractSchemaInvalid {
            candidate_trigger: "commit format".to_string()
        })
        .unwrap()
        .contains("extract_schema_invalid"));
    }

    #[test]
    fn judge_candidate_and_measure_feedback_round_trip() {
        let c = JudgeCandidate {
            trigger_pattern: "user 重複問 commit format".to_string(),
            repeat_count: 7,
            sample_event_ids: vec!["e1".to_string(), "e2".to_string(), "e3".to_string()],
            judged_at: 1_716_563_400_000,
        };
        let back: JudgeCandidate =
            serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(c.trigger_pattern, back.trigger_pattern);
        assert_eq!(c.repeat_count, back.repeat_count);
        assert_eq!(c.sample_event_ids, back.sample_event_ids);
        assert_eq!(c.judged_at, back.judged_at);

        let m = MeasureFeedback {
            skill_id: "01923f8e-9b4c-7000-8c2d-2b9f0e1d4a55".to_string(),
            was_applied: true,
            was_decline: false,
            user_edited: false,
            observed_at: 1_716_563_400_000,
        };
        let back: MeasureFeedback =
            serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(m.skill_id, back.skill_id);
        assert_eq!(m.was_applied, back.was_applied);
        assert_eq!(m.was_decline, back.was_decline);
        assert_eq!(m.user_edited, back.user_edited);
        assert_eq!(m.observed_at, back.observed_at);
    }

    #[test]
    fn judge_candidates_surfaces_provider_error_as_judge_failed() {
        // Stage 3 promotion: `providers_complete` now really delegates to
        // `providers_wire::complete`. In a test environment without
        // `~/.spectyn-mesh/agents.toml`, the provider resolver returns an
        // `Err(ProviderError::...)` which `judge_candidates` maps to
        // `SkillError::JudgeFailed{detail}`. The wire-up is permanent;
        // when providers_wire lands its real HTTP path the inner step
        // will produce a Stage-4 panic instead — flip this test then.
        let now = chrono::Utc::now().to_rfc3339();
        let ev = EventMeta {
            event_id: "01923f8e-9b4c-7000-8c2d-2b9f0e1d4a55".to_string(),
            timestamp: now,
            kind: crate::event_storage_wire::EventKind::Text,
            tags: vec!["test".to_string()],
        };
        match judge_candidates(&[ev], 7) {
            Err(SkillError::JudgeFailed { .. }) => {}
            other => panic!(
                "expected JudgeFailed (real providers delegation), got {other:?}"
            ),
        }
    }

    /// Task A: `run_skill_learning` propagates `JudgeFailed` on the default
    /// install (no provider) — the caller relies on this to degrade to
    /// drain-only. Mirrors `judge_candidates_surfaces_provider_error_*`.
    #[test]
    fn run_skill_learning_no_provider_surfaces_judge_failed() {
        let _g = crate::env_lock::acquire();
        let now = chrono::Utc::now().to_rfc3339();
        let ev = EventMeta {
            event_id: "01923f8e-9b4c-7000-8c2d-2b9f0e1d4a55".to_string(),
            timestamp: now,
            kind: crate::event_storage_wire::EventKind::Text,
            tags: vec!["test".to_string()],
        };
        assert!(
            matches!(run_skill_learning(&[ev], 7), Err(SkillError::JudgeFailed { .. })),
            "no provider ⇒ run_skill_learning must surface JudgeFailed for the caller to degrade"
        );
    }

    /// Task C: `fold_candidates_to_store` is log-and-continue — with no provider
    /// EVERY `extract_skill_from_candidate` errs, so the fold stores 0 and never
    /// panics. That a 2-candidate fold returns 0 (not a panic, not a partial
    /// abort) is the proof of the per-candidate skip arm.
    #[test]
    fn fold_candidates_skips_failing_extract_keeps_going() {
        let cands = vec![
            JudgeCandidate {
                trigger_pattern: "deploy to staging not prod".to_string(),
                repeat_count: 5,
                sample_event_ids: vec!["e1".to_string()],
                judged_at: 0,
            },
            JudgeCandidate {
                trigger_pattern: "run lint before commit".to_string(),
                repeat_count: 6,
                sample_event_ids: vec![],
                judged_at: 0,
            },
        ];
        let stored = fold_candidates_to_store(&cands, &[]);
        assert_eq!(
            stored, 0,
            "no provider ⇒ every extract fails ⇒ fold stores 0, proving log-and-continue"
        );
    }

    /// Task D: `parse_judge_json` accepts both the `{"candidates":[...]}`
    /// envelope and a bare top-level array, and rejects garbage. (Companion to
    /// the existing `parse_judge_json_accepts_both_shapes`.)
    #[test]
    fn parse_judge_json_accepts_envelope_and_bare_array() {
        let one = r#"{"triggerPattern":"deploy staging","repeatCount":5,"sampleEventIds":["e1"],"judgedAt":0}"#;
        let envelope = format!(r#"{{"candidates":[{one}]}}"#);
        assert_eq!(
            parse_judge_json(&envelope).unwrap().len(),
            1,
            "envelope shape ⇒ one candidate"
        );
        let bare = format!("[{one}]");
        assert_eq!(
            parse_judge_json(&bare).unwrap().len(),
            1,
            "bare-array shape ⇒ one candidate"
        );
        assert!(parse_judge_json("garbage").is_err(), "garbage ⇒ Err");
    }

    /// Task E: live end-to-end happy path — needs a real provider configured.
    /// Documents the intended judge→extract→store flow over a repetitive event
    /// window. Compiles in CI; only RUN with `--ignored` + agents.toml + key.
    #[ignore = "live provider — run with --ignored + agents.toml + SPECTYN_MESH_<P>_API_KEY"]
    #[test]
    fn run_skill_learning_live_e2e() {
        let _g = crate::env_lock::acquire();
        let now = chrono::Utc::now().to_rfc3339();
        let mk = |i: usize| EventMeta {
            event_id: format!("01923f8e-9b4c-7000-8c2d-2b9f0e1d4a{i:02}"),
            timestamp: now.clone(),
            kind: crate::event_storage_wire::EventKind::Text,
            tags: vec!["deploy".to_string(), "staging".to_string()],
        };
        let evs: Vec<EventMeta> = (0..6).map(mk).collect();
        let out = run_skill_learning(&evs, 7);
        assert!(out.is_ok(), "live learn pass should succeed with a provider: {out:?}");
    }

    #[ignore = "integration / env-dependent — run via --ignored"]
    #[test]
    fn recall_skills_empty_when_db_absent() {
        // Stage 3 promotion: `fts5_search` is now a real rusqlite query —
        // when the DB is missing (test env) it returns an empty hit set
        // and `recall_skills` should produce `Ok(RecallResult{empty})`
        // with `RecallStrategy::Fts5Only` (the graceful-degrade path).
        // Use a non-existent path so the open fails cleanly.
        let _g = crate::env_lock::acquire();
        let saved = std::env::var_os("SPECTYN_DB_PATH");
        std::env::set_var(
            "SPECTYN_DB_PATH",
            "/tmp/__spectyn_nonexistent_test_db_skill_wire.sqlite",
        );
        let r = recall_skills("anything", RecallPolicy::default())
            .expect("recall must degrade, not panic");
        assert!(r.skills.is_empty());
        assert!(r.scores.is_empty());
        assert_eq!(r.recall_strategy, RecallStrategy::Fts5Only);
        match saved {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }
    }

    #[test]
    fn skill_floor_stubs_return_typed_errors_not_panic() {
        // v0.6 GA floor: the two Stage-4 stubs must NOT unimplemented!()-panic.
        // embedding_search returns Err (recall degrades to FTS5-only); the
        // parameterless skill_store() dispatch returns a typed StoreFailed
        // when nothing has been handed off. The hand-off queue is a process-
        // global static, so clear it first to keep this deterministic under
        // any test ordering.
        let _g = crate::env_lock::acquire();
        clear_handoff_queue();
        assert!(embedding_search("x", &RecallPolicy::default()).is_err());
        assert!(matches!(skill_store(), Err(SkillError::StoreFailed { .. })));
    }

    #[ignore = "integration / env-dependent (writes a temp sqlite) — run via --ignored"]
    #[test]
    fn store_skill_floor_degenerate_skill_flag_off_never_panics() {
        // P0-2 Task 6 floor extension: the direct `store_skill` write path must
        // NEVER panic on a degenerate (all-empty / zero) Skill when sealing is
        // OFF — it returns Ok (a valid empty-string row) or a typed Err, the same
        // de-panic floor the parameterless `skill_store()` dispatch already holds.
        // Guards against a regression where the new Rust FTS feed / seal branch
        // could introduce an .unwrap() panic on the empty path.
        let _g = crate::env_lock::acquire();
        let saved_db = std::env::var_os("SPECTYN_DB_PATH");
        let saved_flag = std::env::var_os("SPECTYN_ENCRYPT_MEMORY");
        // Ensure the seal flag is OFF for the floor (the byte-identical ship path).
        std::env::remove_var("SPECTYN_ENCRYPT_MEMORY");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());

        let degenerate = Skill {
            id: String::new(),
            name: String::new(),
            trigger_pattern: String::new(),
            steps: vec![],
            examples: vec![],
            version: 0,
            quality_score: 0.0,
            last_applied_at: 0,
            source_event_count: 0,
        };

        // Must return a Result (Ok or typed Err) — never unwind/panic.
        let result = std::panic::catch_unwind(|| store_skill(&degenerate));

        match saved_flag {
            Some(v) => std::env::set_var("SPECTYN_ENCRYPT_MEMORY", v),
            None => std::env::remove_var("SPECTYN_ENCRYPT_MEMORY"),
        }
        match saved_db {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }

        let stored = result.expect("store_skill must not panic on a degenerate skill");
        assert!(
            matches!(stored, Ok(()) | Err(SkillError::StoreFailed { .. })),
            "degenerate store must be Ok or typed StoreFailed, got {stored:?}"
        );
    }

    #[ignore = "integration / env-dependent (writes a temp sqlite) — run via --ignored"]
    #[test]
    fn skill_store_persists_queued_extract_handoff() {
        // M1 store hand-off (apex ② owned-memory): an extracted skill handed
        // off via `handoff_extracted_skill` must actually persist when the
        // parameterless Store dispatch (`skill_store`) runs, and then be
        // FTS5-recallable — mirrors `store_skill_then_recall_finds_it_via_fts5`
        // but exercises the queue → drain → store wiring end-to-end.
        let _g = crate::env_lock::acquire();
        clear_handoff_queue();
        let saved = std::env::var_os("SPECTYN_DB_PATH");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());

        let skill = Skill {
            id: "sk-handoff-1".into(),
            name: "rotate the cluster secret".into(),
            trigger_pattern: "rotate secret".into(),
            steps: vec!["fetch current secret".into(), "write new secret".into()],
            examples: vec![],
            version: 1,
            quality_score: 0.7,
            last_applied_at: 0,
            source_event_count: 4,
        };

        // Hand off, then run the parameterless Store dispatch — it must drain
        // the queue and persist via the real `store_skill` path → Ok.
        handoff_extracted_skill(skill);
        let stored = skill_store();
        // The queue is now empty again, so a second dispatch returns the typed
        // StoreFailed (nothing left to persist) — the floor contract holds.
        let empty_again = skill_store();

        // Recall it back via FTS5 (embedding leg skipped at recall_k=0).
        let recall = recall_skills("rotate", RecallPolicy::default());

        // Restore env + clear queue BEFORE asserting so a failure can't leak.
        match saved {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }
        clear_handoff_queue();

        stored.expect("skill_store drains the hand-off queue and persists");
        assert!(
            matches!(empty_again, Err(SkillError::StoreFailed { .. })),
            "drained queue → typed StoreFailed (floor contract): {empty_again:?}"
        );
        let res = recall.expect("recall must degrade, not panic");
        assert!(
            res.skills.iter().any(|s| s.id == "sk-handoff-1"),
            "handed-off + stored skill must be FTS5-recallable"
        );
    }

    #[ignore = "integration / env-dependent (writes a temp sqlite) — run via --ignored"]
    #[test]
    fn skill_store_drains_queue_fifo_and_persists_all() {
        // P0-2 Task 1: the Store hand-off drain is FIFO and persists EVERY
        // queued skill. Hand off three distinct skills in order, run the
        // parameterless `skill_store()` once, and assert all three landed in the
        // `skills` table (one row each). A second dispatch on the now-drained
        // queue returns the typed `StoreFailed` (the panic-floor contract).
        let _g = crate::env_lock::acquire();
        clear_handoff_queue();
        let saved = std::env::var_os("SPECTYN_DB_PATH");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());

        let mk = |id: &str, name: &str, trigger: &str| Skill {
            id: id.into(),
            name: name.into(),
            trigger_pattern: trigger.into(),
            steps: vec![],
            examples: vec![],
            version: 1,
            quality_score: 0.5,
            last_applied_at: 0,
            source_event_count: 3,
        };

        // Hand off three skills in FIFO order.
        handoff_extracted_skill(mk("sk-a", "alpha skill", "alpha"));
        handoff_extracted_skill(mk("sk-b", "bravo skill", "bravo"));
        handoff_extracted_skill(mk("sk-c", "charlie skill", "charlie"));

        let drained = skill_store();
        // The queue is now empty → a second dispatch must return StoreFailed.
        let empty_again = skill_store();

        // Read the persisted rows back through a FRESH connection.
        let read = || -> Result<(i64, Vec<String>), String> {
            use rusqlite::Connection;
            let conn = Connection::open(resolve_db_path()).map_err(|e| format!("open: {e}"))?;
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM skills", [], |r| r.get(0))
                .map_err(|e| format!("count: {e}"))?;
            let mut stmt = conn
                .prepare("SELECT id FROM skills ORDER BY id")
                .map_err(|e| format!("prepare: {e}"))?;
            let ids: Vec<String> = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| format!("query: {e}"))?
                .filter_map(|r| r.ok())
                .collect();
            Ok((count, ids))
        };
        let read_result = read();

        // Restore env + clear queue BEFORE asserting so a failure can't leak.
        match saved {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }
        clear_handoff_queue();

        drained.expect("skill_store must drain the queue and persist all three");
        assert!(
            matches!(empty_again, Err(SkillError::StoreFailed { .. })),
            "drained queue → typed StoreFailed (floor contract): {empty_again:?}"
        );
        let (count, ids) = read_result.expect("read back persisted rows");
        assert_eq!(count, 3, "all three queued skills must persist as distinct rows");
        assert_eq!(
            ids,
            vec!["sk-a".to_string(), "sk-b".to_string(), "sk-c".to_string()],
            "every queued skill id must be present"
        );
    }

    #[ignore = "integration / env-dependent (writes a temp sqlite) — run via --ignored"]
    #[test]
    fn store_skill_upsert_same_id_is_one_row_and_updates() {
        // P0-2 Task 2 (the headline): storing the SAME `skill.id` twice collapses
        // to exactly ONE `skills` row (idempotent upsert keyed on the stable id),
        // the second write's column values win, and the `skills_fts` mirror has
        // exactly one matching entry for the NEW name with NO orphan for the old
        // one (the delete-old-then-insert-new semantics, whether driven by the
        // 0008 skills_au trigger or — after Task 4 — the Rust FTS feed).
        //
        // The old/new names are deliberately distinct single tokens with NO
        // shared substring (`alphaword` vs `betaword`) so the orphan check is
        // unambiguous under the unicode61 tokenizer (a hyphenated name would
        // tokenize into overlapping tokens and the "old name gone" assertion
        // would be untrustworthy).
        let _g = crate::env_lock::acquire();
        let saved = std::env::var_os("SPECTYN_DB_PATH");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());

        let skill = Skill {
            id: "sk-dupe".into(),
            name: "alphaword".into(),
            trigger_pattern: "alpha trigger".into(),
            steps: vec!["one".into()],
            examples: vec![],
            version: 1,
            quality_score: 0.5,
            last_applied_at: 0,
            source_event_count: 3,
        };
        let updated = Skill {
            name: "betaword".into(),
            trigger_pattern: "changed".into(),
            version: 2,
            quality_score: 0.9,
            ..skill.clone()
        };

        let run = || -> Result<(i64, u16, f64, String, i64, i64), String> {
            store_skill(&skill).map_err(|e| format!("store 1: {e:?}"))?;
            store_skill(&updated).map_err(|e| format!("store 2: {e:?}"))?;

            use rusqlite::Connection;
            let conn = Connection::open(resolve_db_path()).map_err(|e| format!("open: {e}"))?;
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM skills", [], |r| r.get(0))
                .map_err(|e| format!("count: {e}"))?;
            let (version, quality, name): (i64, f64, String) = conn
                .query_row(
                    "SELECT version, quality_score, name FROM skills WHERE id = ?1",
                    rusqlite::params!["sk-dupe"],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(|e| format!("select row: {e}"))?;
            // FTS mirror: exactly one hit for the NEW name, zero for the OLD one.
            let fts_new: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM skills_fts WHERE skills_fts MATCH 'betaword'",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| format!("fts new: {e}"))?;
            let fts_old: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM skills_fts WHERE skills_fts MATCH 'alphaword'",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| format!("fts old: {e}"))?;
            Ok((count, version as u16, quality, name, fts_new, fts_old))
        };
        let result = run();

        match saved {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }

        let (count, version, quality, name, fts_new, fts_old) =
            result.expect("upsert + readback");
        assert_eq!(count, 1, "same id stored twice → exactly one row");
        assert_eq!(version, 2, "second write's version must win");
        assert!((quality - 0.9).abs() < 1e-6, "second write's quality must win: {quality}");
        assert_eq!(name, "betaword", "second write's name must win");
        assert_eq!(fts_new, 1, "FTS mirror must have exactly one entry for the new name");
        assert_eq!(
            fts_old, 0,
            "FTS mirror must NOT retain an orphan entry for the old name"
        );
    }

    #[ignore = "integration / env-dependent (writes a temp sqlite) — run via --ignored"]
    #[test]
    fn skills_fts_fed_from_rust_after_triggers_retired() {
        // P0-2 Task 4: the 0008 skills_ai/au/ad auto-sync triggers are retired by
        // 0011 and the Rust write path now feeds skills_fts explicitly. With
        // sealing OFF, a stored skill must STILL be FTS-recallable, AND the three
        // triggers must be gone (proving the Rust feed — not a leftover trigger —
        // is what keeps the index in sync).
        let _g = crate::env_lock::acquire();
        let saved = std::env::var_os("SPECTYN_DB_PATH");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());

        let skill = Skill {
            id: "sk-rustfeed-1".into(),
            name: "compile the kernel".into(),
            trigger_pattern: "compile kernel".into(),
            steps: vec![],
            examples: vec![],
            version: 1,
            quality_score: 0.5,
            last_applied_at: 0,
            source_event_count: 3,
        };

        let run = || -> Result<(bool, i64), String> {
            store_skill(&skill).map_err(|e| format!("store: {e:?}"))?;
            let res = recall_skills("kernel", RecallPolicy::default())
                .map_err(|e| format!("recall: {e:?}"))?;
            let found = res.skills.iter().any(|s| s.id == "sk-rustfeed-1");

            use rusqlite::Connection;
            let conn = Connection::open(resolve_db_path()).map_err(|e| format!("open: {e}"))?;
            let trigger_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' \
                     AND name IN ('skills_ai','skills_au','skills_ad')",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| format!("trigger count: {e}"))?;
            Ok((found, trigger_count))
        };
        let result = run();

        match saved {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }

        let (found, trigger_count) = result.expect("store + recall + trigger probe");
        assert!(found, "Rust-fed skills_fts must still be keyword-recallable");
        assert_eq!(
            trigger_count, 0,
            "skills_ai/au/ad auto-sync triggers must be retired by 0011"
        );
    }

    #[ignore = "integration / env-dependent (writes a temp sqlite) — run via --ignored"]
    #[test]
    fn store_skill_seals_name_and_trigger_on_disk_when_flag_on() {
        // P0-2 Task 5: with SPECTYN_ENCRYPT_MEMORY ON and an EventKey installed,
        // store_skill must seal name/trigger at rest (raw column probes as sealed
        // and does NOT contain the plaintext token), while the de-PII'd token
        // form keeps the skill keyword-recallable through the token-form FTS
        // index, and skill_load round-trips the plaintext name back via open().
        // Mirrors memory.rs::insert_seals_text_and_source_on_disk_when_flag_on.
        let _g = crate::env_lock::acquire();
        let saved_db = std::env::var_os("SPECTYN_DB_PATH");
        let saved_flag = std::env::var_os("SPECTYN_ENCRYPT_MEMORY");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());
        crate::encryption_wire::install_event_key_from_seed(&[0x42u8; 32])
            .expect("install event key");
        std::env::set_var("SPECTYN_ENCRYPT_MEMORY", "1");

        let skill = Skill {
            id: "sk-sealed-1".into(),
            name: "deploycluster".into(),
            trigger_pattern: "deploycluster trigger".into(),
            steps: vec!["step one".into()],
            examples: vec![],
            version: 1,
            quality_score: 0.6,
            last_applied_at: 0,
            source_event_count: 3,
        };

        let run = || -> Result<(bool, bool, bool, String), String> {
            store_skill(&skill).map_err(|e| format!("store: {e:?}"))?;

            use rusqlite::Connection;
            let conn = Connection::open(resolve_db_path()).map_err(|e| format!("open: {e}"))?;
            let raw_name: String = conn
                .query_row(
                    "SELECT name FROM skills WHERE id = ?1",
                    rusqlite::params!["sk-sealed-1"],
                    |r| r.get(0),
                )
                .map_err(|e| format!("select raw name: {e}"))?;
            let probed_sealed = crate::skillbank::memory_seal::is_sealed(&raw_name);
            let leaks_plaintext = raw_name.contains("deploycluster");

            // FTS recall still finds it via the de-PII'd token-form index.
            let recall = recall_skills("deploycluster", RecallPolicy::default())
                .map_err(|e| format!("recall: {e:?}"))?;
            let recalled = recall.skills.iter().any(|s| s.id == "sk-sealed-1");

            // skill_load round-trips the PLAINTEXT name back through open().
            let loaded = skill_load("sk-sealed-1").map_err(|e| format!("load: {e:?}"))?;
            Ok((probed_sealed, leaks_plaintext, recalled, loaded.name))
        };
        let result = run();

        // Restore env + key BEFORE asserting so a failure can't leak.
        std::env::remove_var("SPECTYN_ENCRYPT_MEMORY");
        crate::encryption_wire::clear_event_key_cache();
        match saved_flag {
            Some(v) => std::env::set_var("SPECTYN_ENCRYPT_MEMORY", v),
            None => std::env::remove_var("SPECTYN_ENCRYPT_MEMORY"),
        }
        match saved_db {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }

        let (probed_sealed, leaks_plaintext, recalled, loaded_name) =
            result.expect("sealed store round-trip");
        assert!(probed_sealed, "stored name must probe as sealed (age blob)");
        assert!(
            !leaks_plaintext,
            "sealed name column must NOT contain the plaintext token"
        );
        assert!(recalled, "sealed skill must stay keyword-recallable via token-form FTS");
        assert_eq!(loaded_name, "deploycluster", "skill_load must open() the plaintext back");
    }

    #[ignore = "integration / env-dependent (writes a temp sqlite) — run via --ignored"]
    #[test]
    fn store_skill_fails_closed_when_flag_on_but_no_key() {
        // P0-2 Task 5 fail-closed: flag ON but NO EventKey ⇒ store_skill returns
        // Err(StoreFailed) and writes NO row — never silently writes plaintext.
        // (In #[cfg(test)], lookup_or_derive never reads the real identity.key,
        // so an empty cache yields NoKey — mirrors memory_seal's fail-closed test.)
        let _g = crate::env_lock::acquire();
        let saved_db = std::env::var_os("SPECTYN_DB_PATH");
        let saved_flag = std::env::var_os("SPECTYN_ENCRYPT_MEMORY");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());
        crate::encryption_wire::clear_event_key_cache();
        std::env::set_var("SPECTYN_ENCRYPT_MEMORY", "1");

        let skill = Skill {
            id: "sk-noKey-1".into(),
            name: "should not land".into(),
            trigger_pattern: "no key".into(),
            steps: vec![],
            examples: vec![],
            version: 1,
            quality_score: 0.5,
            last_applied_at: 0,
            source_event_count: 1,
        };

        let stored = store_skill(&skill);

        // Count any rows that may have landed (provision the schema if the store
        // bailed before creating the table → treat "no table" as zero rows).
        let count = (|| -> i64 {
            use rusqlite::Connection;
            let conn = match Connection::open(resolve_db_path()) {
                Ok(c) => c,
                Err(_) => return 0,
            };
            conn.query_row("SELECT COUNT(*) FROM skills", [], |r| r.get(0))
                .unwrap_or(0)
        })();

        // Restore env + key BEFORE asserting.
        std::env::remove_var("SPECTYN_ENCRYPT_MEMORY");
        crate::encryption_wire::clear_event_key_cache();
        match saved_flag {
            Some(v) => std::env::set_var("SPECTYN_ENCRYPT_MEMORY", v),
            None => std::env::remove_var("SPECTYN_ENCRYPT_MEMORY"),
        }
        match saved_db {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }

        assert!(
            matches!(stored, Err(SkillError::StoreFailed { .. })),
            "flag ON + no EventKey must fail closed with StoreFailed: {stored:?}"
        );
        assert_eq!(count, 0, "fail-closed store must write NO row (no plaintext leak)");
    }

    #[ignore = "integration / env-dependent (writes a temp sqlite) — run via --ignored"]
    #[test]
    fn store_skill_then_recall_finds_it_via_fts5() {
        // The 🔴 win: store a skill → 0008 self-provisions → FTS5 keyword recall
        // finds it (embedding leg is skipped at recall_k=0, so the degrade path
        // is exercised end-to-end).
        let _g = crate::env_lock::acquire();
        let saved = std::env::var_os("SPECTYN_DB_PATH");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());

        let skill = Skill {
            id: "sk-roundtrip-1".into(),
            name: "deploy the staging cluster".into(),
            trigger_pattern: "deploy staging".into(),
            steps: vec!["ssh staging".into(), "run deploy.sh".into()],
            examples: vec![],
            version: 1,
            quality_score: 0.8,
            last_applied_at: 0,
            source_event_count: 3,
        };
        store_skill(&skill).expect("store_skill provisions 0008 + inserts");
        let res = recall_skills("staging", RecallPolicy::default()).expect("recall");
        let found = res.skills.iter().any(|s| s.id == "sk-roundtrip-1");

        // Restore env BEFORE asserting so a failure can't leak the override.
        match saved {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }
        assert!(found, "stored skill must be FTS5-recallable");
    }

    /// Regression for the Windows path divergence: `resolve_db_path` must
    /// follow the codebase-wide `dirs::home_dir()` convention (same as
    /// `tasks::store::TaskStore::open_default` and diagnostic's
    /// `spectyn_home()`), not a raw `$HOME` read. On Windows `$HOME` is
    /// normally unset, so the old code fell through to a CWD-relative
    /// `spectyn.db`, splitting the skill DB from the canonical
    /// `~/.spectyn-mesh/spectyn.db`. Resolution order under test:
    /// 1. non-blank `SPECTYN_DB_PATH` wins;
    /// 2. blank `SPECTYN_DB_PATH` is ignored;
    /// 3. otherwise the canonical `dirs::home_dir()`-anchored path — even
    ///    when `$HOME` is absent from the environment.
    #[test]
    fn resolve_db_path_resolution_order() {
        let _g = crate::env_lock::acquire();
        let saved_db = std::env::var("SPECTYN_DB_PATH").ok();
        let saved_home = std::env::var("HOME").ok();

        // 1. Explicit override wins over everything.
        std::env::set_var("SPECTYN_DB_PATH", "/tmp/explicit-override.sqlite");
        assert_eq!(resolve_db_path(), "/tmp/explicit-override.sqlite");

        // 2 + 3. Blank override is ignored, and even with $HOME removed the
        // path must stay anchored at the user's home directory —
        // `dirs::home_dir()` resolves the profile dir without consulting
        // $HOME on Windows (and falls back to the passwd entry on Unix).
        std::env::set_var("SPECTYN_DB_PATH", "   ");
        std::env::remove_var("HOME");
        let expected = dirs::home_dir().map(|h| {
            h.join(".spectyn-mesh")
                .join("spectyn.db")
                .to_string_lossy()
                .into_owned()
        });
        match &expected {
            Some(want) => {
                assert_eq!(
                    &resolve_db_path(),
                    want,
                    "blank override must fall through to the canonical home path"
                );
            }
            None => assert_eq!(resolve_db_path(), "spectyn.db"),
        }
        std::env::remove_var("SPECTYN_DB_PATH");
        match &expected {
            Some(want) => {
                let got = resolve_db_path();
                assert_eq!(
                    &got, want,
                    "must match the dirs::home_dir() convention used by the rest of the codebase"
                );
                assert_ne!(
                    got, "spectyn.db",
                    "CWD-relative fallback must be unreachable when a home dir exists"
                );
            }
            None => assert_eq!(resolve_db_path(), "spectyn.db"),
        }

        // Restore the prior environment for sibling tests.
        match saved_db {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }
        match saved_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    // ─── Stage 3 KAT (known-answer-test) vectors ─────────────────────────

    /// `filter_recent` drops events outside the window. Synthetic event with
    /// timestamp 60 days ago must be excluded when window = 7.
    #[test]
    fn filter_recent_drops_old_events() {
        let now = chrono::Utc::now();
        let old = (now - chrono::Duration::days(60)).to_rfc3339();
        let fresh = now.to_rfc3339();
        let events = vec![
            EventMeta {
                event_id: "old".into(),
                timestamp: old,
                kind: crate::event_storage_wire::EventKind::Text,
                tags: vec![],
            },
            EventMeta {
                event_id: "fresh".into(),
                timestamp: fresh,
                kind: crate::event_storage_wire::EventKind::Text,
                tags: vec![],
            },
        ];
        let kept = filter_recent(&events, 7);
        assert_eq!(kept.len(), 1, "only fresh event survives the 7-day window");
        assert_eq!(kept[0].event_id, "fresh");
    }

    /// `filter_recent` silently drops events whose `timestamp` field fails
    /// RFC-3339 parse — corrupt rows must not abort a judge pass.
    #[test]
    fn filter_recent_skips_unparseable_timestamps() {
        let events = vec![EventMeta {
            event_id: "corrupt".into(),
            timestamp: "not-a-timestamp".into(),
            kind: crate::event_storage_wire::EventKind::Text,
            tags: vec![],
        }];
        assert_eq!(filter_recent(&events, 30).len(), 0);
    }

    /// `parse_judge_json` accepts both envelope and bare-array shapes.
    #[test]
    fn parse_judge_json_accepts_both_shapes() {
        let env = r#"{"candidates":[{"triggerPattern":"x","repeatCount":5,"sampleEventIds":["e1"],"judgedAt":0}]}"#;
        assert_eq!(parse_judge_json(env).unwrap().len(), 1);
        let bare = r#"[{"triggerPattern":"y","repeatCount":6,"sampleEventIds":[],"judgedAt":0}]"#;
        assert_eq!(parse_judge_json(bare).unwrap().len(), 1);
        assert!(parse_judge_json("garbage").is_err());
    }

    /// `parse_skill_json` round-trips through the canonical `Skill` shape.
    #[test]
    fn parse_skill_json_round_trip() {
        let json = r#"{"id":"i","name":"n","triggerPattern":"t","steps":["a"],"examples":[],"version":1,"qualityScore":0.7,"lastAppliedAt":0,"sourceEventCount":5}"#;
        let s = parse_skill_json(json).expect("parse");
        assert_eq!(s.id, "i");
        assert!((s.quality_score - 0.7).abs() < f32::EPSILON);
        assert!(parse_skill_json("{}").is_err(), "missing fields → err");
    }

    /// `redact_pii` strips all 6 PII classes and caps at 100 chars.
    #[test]
    fn redact_pii_strips_all_classes() {
        let raw = "email foo@bar.com ip 192.168.1.1 phone +886-912-345-678 path /Users/example/secret @user42";
        let red = redact_pii(raw);
        assert!(!red.contains("foo@bar.com"), "email leak: {red}");
        assert!(!red.contains("192.168.1.1"), "ip leak: {red}");
        assert!(!red.contains("912-345-678"), "phone leak: {red}");
        assert!(!red.contains("example"), "path leak: {red}");
        assert!(!red.contains("@user42"), "mention leak: {red}");
        assert!(red.contains("<email>"));
        assert!(red.contains("<ip>"));
        assert!(red.contains("<path>"));
    }

    /// `redact_pii` caps at 100 chars even when redaction expands the input.
    #[test]
    fn redact_pii_caps_at_100_chars() {
        let raw = "a".repeat(500);
        let out = redact_pii(&raw);
        assert!(out.chars().count() <= 100, "got {} chars", out.chars().count());
    }

    /// `decide_recall_strategy` picks `HybridUnion` only when both legs
    /// returned at least one hit; downgrades to `Fts5Only` when embedding
    /// fell over or recall_k was 0.
    #[test]
    fn decide_recall_strategy_downgrades_when_embedding_empty() {
        let fts = vec![(make_skill("a"), 0.9)];
        let empty_embed: Vec<(Skill, f32)> = vec![];
        let policy = RecallPolicy::default();
        let s = decide_recall_strategy(&fts, Some(&empty_embed), &policy);
        assert_eq!(s, RecallStrategy::Fts5Only);

        let embed = vec![(make_skill("b"), 0.7)];
        let s = decide_recall_strategy(&fts, Some(&embed), &policy);
        assert_eq!(s, RecallStrategy::HybridUnion);

        let s = decide_recall_strategy(&[], Some(&embed), &policy);
        assert_eq!(s, RecallStrategy::EmbeddingOnly);
    }

    /// `merge_hits` de-dupes by skill id (UUIDv7) under HybridUnion and
    /// preserves the highest score when the same skill appears in both
    /// legs.
    #[test]
    fn merge_hits_dedupes_under_hybrid_union() {
        let a = make_skill("01923f8e-9b4c-7000-8c2d-2b9f0e1d4a55");
        let fts = vec![(a.clone(), 0.5)];
        let embed = vec![(a.clone(), 0.9), (make_skill("other"), 0.4)];
        let merged = merge_hits(&fts, Some(&embed), RecallStrategy::HybridUnion);
        assert_eq!(merged.len(), 2, "dedupe applied: {merged:?}");
        // highest first
        assert!((merged[0].1 - 0.9).abs() < f32::EPSILON);
    }

    /// `merge_hits` Intersection requires presence in BOTH legs.
    #[test]
    fn merge_hits_intersection_requires_both_legs() {
        let a = make_skill("a");
        let b = make_skill("b");
        let fts = vec![(a.clone(), 0.5), (b.clone(), 0.4)];
        let embed = vec![(a.clone(), 0.9)];
        let merged = merge_hits(&fts, Some(&embed), RecallStrategy::HybridIntersect);
        assert_eq!(merged.len(), 1, "only `a` is in both legs");
        assert_eq!(merged[0].0.id, "a");
    }

    /// `format_skills_block` renders empty + non-empty under correct XML
    /// shape; escapes reserved chars so a malicious `name` cannot inject
    /// the recalled_skills closing tag.
    #[test]
    fn format_skills_block_escapes_reserved_xml() {
        assert_eq!(format_skills_block(&[]), "<recalled_skills/>\n");
        let s = Skill {
            id: "i".into(),
            name: "<bad>name</bad>".into(),
            trigger_pattern: "amp & sign".into(),
            steps: vec!["do <thing>".into()],
            examples: vec![],
            version: 1,
            quality_score: 0.5,
            last_applied_at: 0,
            source_event_count: 5,
        };
        let xml = format_skills_block(std::slice::from_ref(&s));
        assert!(xml.contains("<recalled_skills>"));
        assert!(xml.contains("&lt;bad&gt;"), "name escaped: {xml}");
        assert!(xml.contains("amp &amp; sign"), "& escaped: {xml}");
        assert!(xml.contains("&lt;thing&gt;"), "step escaped: {xml}");
    }

    /// `collect_sample_events` returns the subset of events matching the
    /// candidate's id list in spec-stable order.
    #[test]
    fn collect_sample_events_filters_by_id_list() {
        let events = vec![
            EventMeta {
                event_id: "a".into(),
                timestamp: "2026-05-25T00:00:00Z".into(),
                kind: crate::event_storage_wire::EventKind::Text,
                tags: vec![],
            },
            EventMeta {
                event_id: "b".into(),
                timestamp: "2026-05-25T00:00:00Z".into(),
                kind: crate::event_storage_wire::EventKind::Text,
                tags: vec![],
            },
        ];
        let c = JudgeCandidate {
            trigger_pattern: "t".into(),
            repeat_count: 2,
            sample_event_ids: vec!["b".into()],
            judged_at: 0,
        };
        let got = collect_sample_events(&c, &events);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].event_id, "b");
    }

    /// Deterministic test-only embedder. Tokenizes `text` on ASCII
    /// whitespace, hashes each token with a tiny stable FNV-1a, and folds the
    /// hash into a fixed-`DIM` `Vec<f32>` bucket-count vector. No randomness,
    /// no clock, no I/O → identical input always yields the identical vector,
    /// which is exactly what the recall + store round-trip tests need (the
    /// production `ort` MiniLM impl is still Stage 4 / not in deps).
    struct FixtureEmbedder {
        dim: usize,
    }

    impl FixtureEmbedder {
        const DIM: usize = 8;
        fn new() -> Self {
            FixtureEmbedder { dim: Self::DIM }
        }
        /// 32-bit FNV-1a — stable across runs/platforms (no std Hasher seed).
        fn fnv1a(token: &str) -> u32 {
            let mut h: u32 = 0x811c_9dc5;
            for b in token.bytes() {
                h ^= b as u32;
                h = h.wrapping_mul(0x0100_0193);
            }
            h
        }
    }

    impl EmbeddingProvider for FixtureEmbedder {
        fn embed(&self, text: &str) -> Result<Vec<f32>, SkillError> {
            let mut v = vec![0.0f32; self.dim];
            for token in text.split_whitespace() {
                let h = Self::fnv1a(&token.to_lowercase());
                let bucket = (h as usize) % self.dim;
                v[bucket] += 1.0;
            }
            Ok(v)
        }
    }

    #[test]
    fn cosine_identical_vectors_is_one() {
        let a = vec![0.1, 0.2, 0.3, 0.4];
        assert!(
            (cosine(&a, &a) - 1.0).abs() < 1e-6,
            "cosine of a vector with itself must be ~1.0, got {}",
            cosine(&a, &a)
        );
        // Parallel-but-scaled vectors are also colinear → cosine ~1.0.
        let scaled: Vec<f32> = a.iter().map(|x| x * 3.0).collect();
        assert!((cosine(&a, &scaled) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_vectors_is_zero() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!(
            cosine(&a, &b).abs() < 1e-6,
            "orthogonal vectors must have cosine ~0.0, got {}",
            cosine(&a, &b)
        );
    }

    #[test]
    fn cosine_degrades_not_panics_on_bad_input() {
        // Empty, mismatched length, and zero-norm must return 0.0 (no NaN,
        // no panic) — mirrors the embedding_search de-panic floor.
        assert_eq!(cosine(&[], &[]), 0.0);
        assert_eq!(cosine(&[1.0, 2.0], &[1.0]), 0.0);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 2.0]), 0.0);
        assert!(!cosine(&[1.0, 1.0], &[1.0, 1.0]).is_nan());
    }

    #[test]
    fn fixture_embedder_is_deterministic() {
        let e = FixtureEmbedder::new();
        let v1 = e.embed("deploy the staging cluster").unwrap();
        let v2 = e.embed("deploy the staging cluster").unwrap();
        assert_eq!(v1, v2, "same input must yield the identical vector");
        assert_eq!(v1.len(), FixtureEmbedder::DIM);
        // Different input should (almost certainly) differ in the bucket sums.
        let v3 = e.embed("totally unrelated phrasing here").unwrap();
        assert_ne!(v1, v3, "distinct inputs should not collide to the same vector");
        // Self-similarity is maximal; cross-similarity is strictly lower for
        // these two non-colinear vectors.
        assert!(cosine(&v1, &v1) >= cosine(&v1, &v3));
    }

    #[test]
    fn embed_skill_text_is_name_then_trigger() {
        let s = Skill {
            id: "sk-t1".into(),
            name: "deploy the staging cluster".into(),
            trigger_pattern: "deploy staging".into(),
            steps: vec!["ignored".into()],
            examples: vec![],
            version: 1,
            quality_score: 0.5,
            last_applied_at: 0,
            source_event_count: 1,
        };
        // The embedded text is name + " " + trigger_pattern (steps/examples are NOT
        // embedded: they are PII-bearing and not part of the recall signal).
        assert_eq!(embed_skill_text(&s), "deploy the staging cluster deploy staging");
    }

    #[test]
    fn store_handoff_with_embedding_persists_vector_and_semantic_recalls() {
        let _g = crate::env_lock::acquire();
        clear_handoff_queue();
        let saved = std::env::var_os("SPECTYN_DB_PATH");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());

        let embedder = FixtureEmbedder::new();
        let skill = Skill {
            id: "sk-embed-handoff".into(),
            name: "rotate the cluster secret".into(),
            trigger_pattern: "rotate secret".into(),
            steps: vec![],
            examples: vec![],
            version: 1,
            quality_score: 0.7,
            last_applied_at: 0,
            source_event_count: 4,
        };
        let vec = embedder.embed(&embed_skill_text(&skill)).unwrap();

        // Run the env-touching body in a closure so env is restored before asserts.
        let (blob_present, top_id) = (|| -> (bool, String) {
            // Hand off WITH an embedding, then run the parameterless Store dispatch.
            handoff_extracted_skill_with_embedding(skill.clone(), Some(vec.clone()));
            skill_store().expect("Store drains queue and persists with embedding");

            // The embedding column must now be non-NULL (proves embed-at-store wired).
            use rusqlite::Connection;
            let conn = Connection::open(resolve_db_path()).unwrap();
            let blob: Option<Vec<u8>> = conn
                .query_row(
                    "SELECT embedding FROM skills WHERE id = ?1",
                    rusqlite::params!["sk-embed-handoff"],
                    |r| r.get(0),
                )
                .unwrap();

            // Semantic recall must now find it: install the fixture embedder, query
            // with the same text, expect this skill top-1 by cosine.
            set_test_embedder(Box::new(FixtureEmbedder::new()));
            let ranked = embedding_search("rotate secret", &RecallPolicy::default())
                .expect("embedding_search ranks now that a vector is stored");
            let top = ranked.first().map(|(s, _)| s.id.clone()).unwrap_or_default();
            clear_test_embedder();
            (blob.is_some(), top)
        })();

        match saved {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }
        clear_handoff_queue();

        assert!(blob_present, "embed-at-store must persist a non-NULL embedding BLOB");
        assert_eq!(top_id, "sk-embed-handoff", "stored vector must be semantically recallable top-1");
    }

    #[test]
    fn six_step_owned_memory_loop_round_trips_semantically() {
        // capture (events) -> extract (a Skill) -> store (with embedding) ->
        // recall (semantic top-k) -> apply (XML block). Hermetic: fixture embedder,
        // temp DB, no network, no live model, NOT #[ignore].
        let _g = crate::env_lock::acquire();
        clear_handoff_queue();
        let saved = std::env::var_os("SPECTYN_DB_PATH");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());

        let embedder = FixtureEmbedder::new();

        let (applied_contains_target, top_id, strategy) = (|| -> (bool, String, RecallStrategy) {
            // (2) EXTRACT — represent the extracted skill (extract_skill_from_candidate's
            //     LLM call is Stage 4 / not hermetic, so we use its OUTPUT type directly;
            //     the redact step is exercised by Task-independent tests).
            let target = Skill {
                id: "sk-loop-deploy".into(),
                name: "deploy the staging cluster".into(),
                trigger_pattern: "deploy staging".into(),
                steps: vec!["ssh staging".into(), "run deploy.sh".into()],
                examples: vec![],
                version: 1,
                quality_score: 0.8,
                last_applied_at: 0,
                source_event_count: 3,
            };
            let distractor = Skill {
                id: "sk-loop-rotate".into(),
                name: "rotate the vault secrets".into(),
                trigger_pattern: "rotate secrets".into(),
                steps: vec![],
                examples: vec![],
                version: 1,
                quality_score: 0.5,
                last_applied_at: 0,
                source_event_count: 2,
            };

            // (3+4) STORE + EMBED — embed the canonical surface, hand off, run Store.
            for s in [&target, &distractor] {
                let v = embedder.embed(&embed_skill_text(s)).unwrap();
                handoff_extracted_skill_with_embedding(s.clone(), Some(v));
            }
            skill_store().expect("Store persists both skills + embeddings");

            // (5) RECALL — semantic leg on. recall_k>0 gates the embedding leg.
            set_test_embedder(Box::new(FixtureEmbedder::new()));
            let policy = RecallPolicy { recall_k: 8, ..RecallPolicy::default() };
            let ranked = embedding_search("deploy staging", &policy)
                .expect("semantic leg ranks the stored vectors");
            let top = ranked.first().map(|(s, _)| s.id.clone()).unwrap_or_default();

            let recall = recall_skills("deploy staging", policy).expect("recall must not panic");

            // (6 / apply) APPLY — render the recalled skills and confirm the target is in it.
            let block = apply_skill_to_prompt("<task/>", &recall.skills);
            clear_test_embedder();
            (block.contains("deploy the staging cluster"), top, recall.recall_strategy)
        })();

        match saved {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }
        clear_handoff_queue();

        assert_eq!(top_id, "sk-loop-deploy", "semantic top-1 must be the query-closest skill");
        assert_eq!(strategy, RecallStrategy::HybridUnion, "both FTS5 + embedding legs hit");
        assert!(applied_contains_target, "apply step must render the recalled target skill");
    }

    #[test]
    fn recall_hit_rate_counts_expected_ids_in_topk() {
        let mk = |id: &str| Skill {
            id: id.into(),
            name: "n".into(),
            trigger_pattern: "t".into(),
            steps: vec![],
            examples: vec![],
            version: 1,
            quality_score: 0.5,
            last_applied_at: 0,
            source_event_count: 1,
        };
        let recalled = vec![mk("a"), mk("b"), mk("c"), mk("d")];

        // All 2 expected present in top-4 → 1.0
        assert!((recall_hit_rate(&["a", "c"], &recalled, 4) - 1.0).abs() < 1e-6);
        // "c" falls outside top-2 (a,b) → only "a" hits → 0.5
        assert!((recall_hit_rate(&["a", "c"], &recalled, 2) - 0.5).abs() < 1e-6);
        // Nothing expected present → 0.0
        assert!(recall_hit_rate(&["z"], &recalled, 4).abs() < 1e-6);
        // Empty expected set → 0.0 (no signal), never NaN/panic
        assert_eq!(recall_hit_rate(&[], &recalled, 4), 0.0);
        // k larger than the list is clamped to the list length (no panic).
        assert!((recall_hit_rate(&["d"], &recalled, 99) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn semantic_recall_hit_rate_over_fixture_corpus_meets_threshold() {
        let _g = crate::env_lock::acquire();
        clear_handoff_queue();
        let saved = std::env::var_os("SPECTYN_DB_PATH");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());

        let embedder = FixtureEmbedder::new();

        let hit_rate = (|| -> f32 {
            // Labelled corpus: (skill_id, embed-text == the exact query for that id).
            // FixtureEmbedder is exact-token-bucket, so an identical query → cosine 1.0
            // for its own skill and < 1.0 for the others → recall@1 should be 1.0.
            let corpus: &[(&str, &str)] = &[
                ("sk-deploy", "deploy staging cluster"),
                ("sk-rotate", "rotate vault secrets nightly"),
                ("sk-summarize", "summarize weekly standup notes"),
                ("sk-backup", "backup the postgres database"),
            ];
            for (id, text) in corpus {
                let s = Skill {
                    id: (*id).into(),
                    name: (*text).into(),
                    trigger_pattern: (*text).into(),
                    steps: vec![],
                    examples: vec![],
                    version: 1,
                    quality_score: 0.5,
                    last_applied_at: 0,
                    source_event_count: 1,
                };
                let v = embedder.embed(&embed_skill_text(&s)).unwrap();
                handoff_extracted_skill_with_embedding(s, Some(v));
            }
            skill_store().expect("store the labelled corpus");

            set_test_embedder(Box::new(FixtureEmbedder::new()));
            let policy = RecallPolicy { recall_k: 8, ..RecallPolicy::default() };
            let mut total = 0.0f32;
            for (expected_id, query) in corpus {
                let ranked =
                    embedding_search(query, &policy).expect("semantic leg ranks corpus");
                let recalled: Vec<Skill> = ranked.into_iter().map(|(s, _)| s).collect();
                total += recall_hit_rate(&[expected_id], &recalled, 1);
            }
            clear_test_embedder();
            total / corpus.len() as f32
        })();

        match saved {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }
        clear_handoff_queue();

        // Deterministic fixture exact-match corpus → perfect recall@1. A real model
        // would set a lower bar (e.g. >= 0.75); the fixture proves the PLUMBING is
        // correct end-to-end, not the model quality.
        assert!(
            hit_rate >= 0.99,
            "semantic recall@1 over the fixture corpus must be ~1.0, got {hit_rate}"
        );
    }

    #[ignore = "integration / env-dependent (writes a temp sqlite) — run via --ignored"]
    #[test]
    fn store_skill_with_embedding_round_trips_the_vector() {
        // Store a skill WITH an embedding → 0008 + 0009 self-provision → read
        // the `embedding` BLOB column back and confirm the f32 vector survives
        // the little-endian round-trip byte-for-byte.
        let _g = crate::env_lock::acquire();
        let saved = std::env::var_os("SPECTYN_DB_PATH");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());

        let embedder = FixtureEmbedder::new();
        let skill = Skill {
            id: "sk-embed-1".into(),
            name: "deploy the staging cluster".into(),
            trigger_pattern: "deploy staging".into(),
            steps: vec!["ssh staging".into(), "run deploy.sh".into()],
            examples: vec![],
            version: 1,
            quality_score: 0.8,
            last_applied_at: 0,
            source_event_count: 3,
        };
        let vec = embedder
            .embed(&format!("{} {}", skill.name, skill.trigger_pattern))
            .unwrap();

        let run = || -> Result<Vec<f32>, String> {
            store_skill_with_embedding(&skill, Some(&vec))
                .map_err(|e| format!("store: {e:?}"))?;
            // Re-applying must stay idempotent (0009 ALTER skipped 2nd time).
            store_skill_with_embedding(&skill, Some(&vec))
                .map_err(|e| format!("store re-apply: {e:?}"))?;

            use rusqlite::Connection;
            let conn = Connection::open(resolve_db_path())
                .map_err(|e| format!("open: {e}"))?;
            let blob: Vec<u8> = conn
                .query_row(
                    "SELECT embedding FROM skills WHERE id = ?1",
                    rusqlite::params!["sk-embed-1"],
                    |row| row.get(0),
                )
                .map_err(|e| format!("select: {e}"))?;
            blob_to_embedding(&blob).ok_or_else(|| "blob not a multiple of 4".to_string())
        };
        let result = run();

        // Restore env BEFORE asserting so a failure can't leak the override.
        match saved {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }

        let read_back = result.expect("store + read-back embedding");
        assert_eq!(read_back, vec, "embedding must survive the BLOB round-trip");
        // Round-tripped vector compared against itself is maximally similar.
        assert!((cosine(&read_back, &vec) - 1.0).abs() < 1e-6);
    }

    // ─── M3: semantic recall (embedding cosine) leg ──────────────────────────

    /// Build a `Skill` whose `name`/`trigger_pattern` carry searchable text,
    /// store it WITH a FixtureEmbedder embedding derived from `embed_text`, and
    /// return the skill. Keeps the M3 ranking test readable.
    #[cfg(test)]
    fn store_skill_with_fixture_embedding(
        embedder: &FixtureEmbedder,
        id: &str,
        name: &str,
        trigger: &str,
        embed_text: &str,
    ) -> Skill {
        let skill = Skill {
            id: id.into(),
            name: name.into(),
            trigger_pattern: trigger.into(),
            steps: vec![],
            examples: vec![],
            version: 1,
            quality_score: 0.5,
            last_applied_at: 0,
            source_event_count: 3,
        };
        let vec = embedder.embed(embed_text).unwrap();
        store_skill_with_embedding(&skill, Some(&vec))
            .expect("store_skill_with_embedding provisions 0008+0009 and inserts");
        skill
    }

    #[ignore = "integration / env-dependent (writes a temp sqlite) — run via --ignored"]
    #[test]
    fn embedding_search_ranks_closest_skill_top1_and_recall_is_hybrid_union() {
        // 🔴 The M3 win: store 3 skills each with a FixtureEmbedder embedding,
        // build a query embedding close to ONE of them, and assert
        //   (a) `embedding_search` returns that skill as top-1 (highest cosine),
        //   (b) `recall_skills` then reports `RecallStrategy::HybridUnion`
        //       (both the FTS5 keyword leg AND the embedding semantic leg hit).
        //
        // The provider is injected via the thread-local test hook so production
        // (no `ort` MiniLM yet) still Errs and degrades to FTS5-only — the
        // `skill_floor_stubs_return_typed_errors_not_panic` de-panic test in
        // this same module proves that path is untouched.
        let _g = crate::env_lock::acquire();
        let saved = std::env::var_os("SPECTYN_DB_PATH");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());

        let embedder = FixtureEmbedder::new();
        // Three distinct skills. Skill A's embedding text matches the query
        // verbatim → maximal cosine; B/C are deliberately off-topic so the
        // query vector ranks A strictly first. A's name/trigger also carry the
        // query keywords so the FTS5 leg ALSO hits A (→ HybridUnion, not
        // EmbeddingOnly).
        let query = "deploy staging";

        // Run the whole DB-touching body inside a closure so we can restore the
        // SPECTYN_DB_PATH override + clear the embedder hook BEFORE asserting,
        // even on a panic-free early failure path.
        let outcome = (|| -> (String, RecallStrategy, usize) {
            let a = store_skill_with_fixture_embedding(
                &embedder,
                "sk-m3-a",
                "deploy the staging cluster",
                "deploy staging",
                "deploy staging", // ← identical to the query → cosine 1.0
            );
            let _b = store_skill_with_fixture_embedding(
                &embedder,
                "sk-m3-b",
                "rotate the vault secrets",
                "rotate secrets",
                "rotate vault secrets nightly",
            );
            let _c = store_skill_with_fixture_embedding(
                &embedder,
                "sk-m3-c",
                "summarize the standup notes",
                "summarize standup",
                "summarize weekly standup notes",
            );

            // Install the deterministic embedder so the semantic leg runs.
            set_test_embedder(Box::new(FixtureEmbedder::new()));

            // (a) raw embedding leg: skill A must be the top-1 by cosine.
            let ranked = embedding_search(query, &RecallPolicy::default())
                .expect("embedding_search must rank when an embedder is installed");
            let top_id = ranked
                .first()
                .map(|(s, _)| s.id.clone())
                .unwrap_or_default();

            // (b) full hybrid recall: both legs hit → HybridUnion, and A is
            //     present in the merged result set.
            let recall = recall_skills(query, RecallPolicy::default())
                .expect("recall must not panic");
            let a_in_recall = recall.skills.iter().filter(|s| s.id == a.id).count();

            (top_id, recall.recall_strategy, a_in_recall)
        })();

        // Restore env + clear the hook BEFORE asserting so a failure can never
        // leak the override into sibling tests or leave an embedder installed.
        set_test_embedder_cleanup();
        match saved {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }

        let (top_id, strategy, a_in_recall) = outcome;
        assert_eq!(
            top_id, "sk-m3-a",
            "embedding_search must rank the query-closest skill (A) top-1"
        );
        assert_eq!(
            strategy,
            RecallStrategy::HybridUnion,
            "both FTS5 + embedding legs hit → recall strategy must be HybridUnion"
        );
        assert_eq!(
            a_in_recall, 1,
            "the top semantic match (A) must appear exactly once in the merged recall"
        );
    }

    /// Helper to clear the thread-local embedder from a test teardown without
    /// naming the `#[cfg(test)]` fn inline twice (keeps the asserting test
    /// body tidy). Restores the production no-embedder state.
    #[cfg(test)]
    fn set_test_embedder_cleanup() {
        clear_test_embedder();
    }

    #[test]
    fn embedding_search_errs_without_provider_even_with_stored_embeddings() {
        // De-panic floor (M3 reinforcement of the M2 contract): with NO
        // embedder installed, `embedding_search` returns Err REGARDLESS of
        // what is on disk — so `recall_skills` always degrades to FTS5-only in
        // production where the `ort` MiniLM runtime is not wired. This runs on
        // its own test thread, so the thread-local hook is `None`; we also
        // defensively clear it first.
        clear_test_embedder();
        assert!(
            embedding_search("x", &RecallPolicy::default()).is_err(),
            "no provider installed → embedding_search must Err (FTS5-only degrade)"
        );
    }

    #[test]
    fn embedding_search_runs_with_injected_provider_but_errs_without_stored_embeddings() {
        // With an embedder installed but an empty / absent skill store, the
        // semantic leg has nothing to rank → Err(()) → recall still degrades
        // to FTS5-only. Point at a guaranteed-absent DB so no row is loaded.
        let _g = crate::env_lock::acquire();
        let saved = std::env::var_os("SPECTYN_DB_PATH");
        std::env::set_var(
            "SPECTYN_DB_PATH",
            "/tmp/__spectyn_nonexistent_m3_no_rows.sqlite",
        );
        set_test_embedder(Box::new(FixtureEmbedder::new()));

        let r = embedding_search("deploy staging", &RecallPolicy::default());

        clear_test_embedder();
        match saved {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }
        assert!(
            r.is_err(),
            "embedder installed but no stored embeddings → still Err (degrade)"
        );
    }

    #[ignore = "integration / env-dependent (writes a temp sqlite) — run via --ignored"]
    #[test]
    fn store_skill_without_embedding_leaves_column_null() {
        // The None path must preserve current behavior: the `embedding`
        // column stays NULL and the existing store_skill caller is unchanged.
        let _g = crate::env_lock::acquire();
        let saved = std::env::var_os("SPECTYN_DB_PATH");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("SPECTYN_DB_PATH", tmp.path());

        let skill = Skill {
            id: "sk-noembed-1".into(),
            name: "no embedding here".into(),
            trigger_pattern: "plain store".into(),
            steps: vec![],
            examples: vec![],
            version: 1,
            quality_score: 0.5,
            last_applied_at: 0,
            source_event_count: 1,
        };

        let run = || -> Result<bool, String> {
            store_skill(&skill).map_err(|e| format!("store: {e:?}"))?;
            use rusqlite::Connection;
            let conn = Connection::open(resolve_db_path())
                .map_err(|e| format!("open: {e}"))?;
            let is_null: bool = conn
                .query_row(
                    "SELECT embedding IS NULL FROM skills WHERE id = ?1",
                    rusqlite::params!["sk-noembed-1"],
                    |row| row.get(0),
                )
                .map_err(|e| format!("select: {e}"))?;
            Ok(is_null)
        };
        let result = run();

        match saved {
            Some(v) => std::env::set_var("SPECTYN_DB_PATH", v),
            None => std::env::remove_var("SPECTYN_DB_PATH"),
        }
        assert!(result.expect("query"), "store_skill must leave embedding NULL");
    }

    #[test]
    fn embedding_blob_round_trip_is_lossless() {
        // Pure unit: serialize → deserialize is the identity for valid input,
        // and corrupt (non-multiple-of-4) byte lengths degrade to None.
        let v = vec![0.0f32, 1.5, -2.25, 3.125, f32::MIN, f32::MAX];
        let blob = embedding_to_blob(&v);
        assert_eq!(blob.len(), v.len() * 4);
        assert_eq!(blob_to_embedding(&blob), Some(v));
        assert_eq!(blob_to_embedding(&[]), Some(vec![]));
        assert_eq!(blob_to_embedding(&[1, 2, 3]), None, "ragged BLOB → None");
    }

    // ─── M4: production `ApiEmbedder` (request shaping + parsing + default-OFF)

    /// Request shaping: the OpenAI-compatible body must carry the configured
    /// model + the verbatim input text, and MUST NOT carry the API key (the key
    /// travels in the Authorization header only — secret-hygiene invariant).
    #[test]
    fn api_embedder_builds_openai_request_body() {
        let e = ApiEmbedder::new(
            "https://api.openai.com/v1/embeddings",
            "text-embedding-3-small",
            "sk-SECRET-do-not-leak",
        );
        let body = e.build_request_body("deploy the staging cluster");
        assert_eq!(body["model"], "text-embedding-3-small");
        assert_eq!(body["input"], "deploy the staging cluster");
        // The key must never appear in the serialized request body.
        let serialized = serde_json::to_string(&body).unwrap();
        assert!(
            !serialized.contains("sk-SECRET"),
            "API key leaked into request body: {serialized}"
        );
    }

    /// Response parsing: a well-formed OpenAI embeddings payload yields the
    /// first embedding vector as `Vec<f32>`.
    #[test]
    fn api_embedder_parses_openai_response() {
        let e = ApiEmbedder::new("u", "text-embedding-3-small", "k");
        let body = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.1,0.2,-0.3,0.4]}],"model":"text-embedding-3-small"}"#;
        let v = e.parse_embedding_response(body).expect("parse ok");
        assert_eq!(v.len(), 4);
        assert!((v[0] - 0.1).abs() < 1e-6);
        assert!((v[2] - (-0.3)).abs() < 1e-6);
    }

    /// Error mapping: a non-JSON / malformed / empty response must map to a
    /// `SkillError` (never panic), and the variant must be the §11.1
    /// embedding-failure code — with the MODEL in the detail, never the key.
    #[test]
    fn api_embedder_maps_parse_failures_to_skill_error() {
        let e = ApiEmbedder::new("u", "text-embedding-3-small", "sk-SECRET");

        // (a) not JSON at all
        match e.parse_embedding_response("<html>502 Bad Gateway</html>") {
            Err(SkillError::EmbeddingTimeout { provider }) => {
                assert!(provider.contains("text-embedding-3-small"));
                assert!(!provider.contains("sk-SECRET"), "key leaked: {provider}");
            }
            other => panic!("expected EmbeddingTimeout, got {other:?}"),
        }

        // (b) JSON but missing data[0].embedding
        match e.parse_embedding_response(r#"{"data":[]}"#) {
            Err(SkillError::EmbeddingTimeout { .. }) => {}
            other => panic!("expected EmbeddingTimeout for missing embedding, got {other:?}"),
        }

        // (c) embedding present but empty array
        match e.parse_embedding_response(r#"{"data":[{"embedding":[]}]}"#) {
            Err(SkillError::EmbeddingTimeout { .. }) => {}
            other => panic!("expected EmbeddingTimeout for empty vector, got {other:?}"),
        }

        // (d) non-numeric value inside the embedding array
        match e.parse_embedding_response(r#"{"data":[{"embedding":["oops"]}]}"#) {
            Err(SkillError::EmbeddingTimeout { .. }) => {}
            other => panic!("expected EmbeddingTimeout for non-numeric, got {other:?}"),
        }
    }

    /// Default-OFF: with NO embedding config in the environment,
    /// `ApiEmbedder::from_env()` returns `None`, so production recall stays
    /// FTS5-only. This is THE load-bearing M4 guarantee.
    #[test]
    fn api_embedder_from_env_is_none_when_unconfigured() {
        let _g = crate::env_lock::acquire();
        let keys = [
            "SPECTYN_EMBED_PROVIDER",
            "SPECTYN_EMBED_API_KEY",
            "SPECTYN_EMBED_MODEL",
            "SPECTYN_EMBED_BASE_URL",
        ];
        let saved: Vec<_> = keys.iter().map(|k| (*k, std::env::var_os(k))).collect();
        for k in &keys {
            std::env::remove_var(k);
        }

        let got = ApiEmbedder::from_env();

        // Restore BEFORE asserting so a failure can't leak the override.
        for (k, v) in saved {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
        assert!(
            got.is_none(),
            "from_env must be None when SPECTYN_EMBED_PROVIDER is unset (default-OFF)"
        );
    }

    /// Master switch: even WITH a key present, an unset (or blank) provider slug
    /// keeps the embedder OFF — the provider env var is the on/off switch.
    #[test]
    fn api_embedder_from_env_none_without_provider_even_with_key() {
        let _g = crate::env_lock::acquire();
        // NOTE: the "no key" sub-case below also has to clear the conventional
        // fallback vars `resolve_embed_key` consults (`SPECTYN_MESH_OPENAI_API_KEY`
        // and `OPENAI_API_KEY`) — otherwise on a developer machine that happens
        // to export `OPENAI_API_KEY` the key would resolve and the assertion
        // would flip. Save + clear them all under the env_lock, restore after.
        let keys = [
            "SPECTYN_EMBED_PROVIDER",
            "SPECTYN_EMBED_API_KEY",
            "SPECTYN_EMBED_MODEL",
            "SPECTYN_EMBED_BASE_URL",
            "SPECTYN_MESH_OPENAI_API_KEY",
            "OPENAI_API_KEY",
        ];
        let saved: Vec<_> = keys.iter().map(|k| (*k, std::env::var_os(k))).collect();
        for k in &keys {
            std::env::remove_var(k);
        }
        // Key present, but provider blank → still OFF.
        std::env::set_var("SPECTYN_EMBED_API_KEY", "sk-present");
        std::env::set_var("SPECTYN_EMBED_PROVIDER", "   ");
        let blank = ApiEmbedder::from_env();

        // Provider set but NO key anywhere → still OFF.
        std::env::remove_var("SPECTYN_EMBED_API_KEY");
        std::env::set_var("SPECTYN_EMBED_PROVIDER", "openai");
        let no_key = ApiEmbedder::from_env();

        for (k, v) in saved {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
        assert!(blank.is_none(), "blank provider slug must keep embedder OFF");
        assert!(no_key.is_none(), "missing key must keep embedder OFF");
    }

    /// Configured path: provider + embedding-specific key + overrides produce a
    /// `Some(ApiEmbedder)` whose request body honours the override model. Does
    /// NOT make a network call (only shapes the request).
    #[test]
    fn api_embedder_from_env_some_when_configured() {
        let _g = crate::env_lock::acquire();
        let keys = [
            "SPECTYN_EMBED_PROVIDER",
            "SPECTYN_EMBED_API_KEY",
            "SPECTYN_EMBED_MODEL",
            "SPECTYN_EMBED_BASE_URL",
        ];
        let saved: Vec<_> = keys.iter().map(|k| (*k, std::env::var_os(k))).collect();
        for k in &keys {
            std::env::remove_var(k);
        }
        std::env::set_var("SPECTYN_EMBED_PROVIDER", "openai");
        std::env::set_var("SPECTYN_EMBED_API_KEY", "sk-test-only");
        std::env::set_var("SPECTYN_EMBED_MODEL", "text-embedding-3-large");

        let got = ApiEmbedder::from_env();

        let body = got.as_ref().map(|e| e.build_request_body("hi"));

        for (k, v) in saved {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
        assert!(got.is_some(), "configured provider + key → Some(ApiEmbedder)");
        let body = body.unwrap();
        assert_eq!(body["model"], "text-embedding-3-large", "model override honoured");
        assert_eq!(body["input"], "hi");
    }

    /// GATED LIVE test: only runs when `SPECTYN_EMBED_LIVE_KEY` is set (a real
    /// OpenAI-compatible key). Stays `#[ignore]`d so the suite is hermetic by
    /// default; run with `--ignored` AND the env var present to make one real
    /// embedding call and confirm a non-empty vector comes back.
    #[ignore = "live network — set SPECTYN_EMBED_LIVE_KEY and run via --ignored"]
    #[test]
    fn api_embedder_live_embeds_when_key_present() {
        let key = match std::env::var("SPECTYN_EMBED_LIVE_KEY") {
            Ok(k) if !k.trim().is_empty() => k,
            _ => {
                eprintln!("SPECTYN_EMBED_LIVE_KEY not set — skipping live embed call");
                return;
            }
        };
        let endpoint = std::env::var("SPECTYN_EMBED_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_EMBED_ENDPOINT.to_string());
        let model = std::env::var("SPECTYN_EMBED_MODEL")
            .unwrap_or_else(|_| DEFAULT_EMBED_MODEL.to_string());
        let e = ApiEmbedder::new(endpoint, model, key);
        let v = e
            .embed("spectyn mesh owned memory semantic recall")
            .expect("live embedding call must succeed with a valid key");
        assert!(!v.is_empty(), "live embedding vector must be non-empty");
        // text-embedding-3-small is 1536-dim; a compatible model is also large.
        assert!(v.len() >= 256, "embedding dim unexpectedly small: {}", v.len());
    }

    /// Helper: build a minimal `Skill` with the given id for merge tests.
    fn make_skill(id: &str) -> Skill {
        Skill {
            id: id.to_string(),
            name: "n".into(),
            trigger_pattern: "t".into(),
            steps: vec![],
            examples: vec![],
            version: 1,
            quality_score: 0.5,
            last_applied_at: 0,
            source_event_count: 5,
        }
    }

    #[test]
    fn skill_summary_and_recall_result_round_trip() {
        let s = SkillSummary {
            count_total: 69,
            count_active: 28,
            last_extracted_at: 1_716_520_000_000,
            top_3_by_score: vec![
                "commit message 格式偏好".to_string(),
                "prefer rg over grep".to_string(),
                "morning standup style".to_string(),
            ],
        };
        let back: SkillSummary =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s.count_total, back.count_total);
        assert_eq!(s.count_active, back.count_active);
        assert_eq!(s.last_extracted_at, back.last_extracted_at);
        assert_eq!(s.top_3_by_score, back.top_3_by_score);

        // §8.4 parallel invariant: scores[i] is the score for skills[i] —
        // same length, same order.
        let rr = RecallResult {
            skills: vec![],
            scores: vec![],
            recall_strategy: RecallStrategy::HybridUnion,
        };
        assert_eq!(rr.skills.len(), rr.scores.len());
        let back: RecallResult =
            serde_json::from_str(&serde_json::to_string(&rr).unwrap()).unwrap();
        assert_eq!(rr.skills.len(), back.skills.len());
        assert_eq!(rr.scores.len(), back.scores.len());
        assert_eq!(rr.recall_strategy, back.recall_strategy);
    }
}
