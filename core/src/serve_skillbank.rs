//! F400 — RPC endpoints exposing the skill bank.
//!
//! Three GET routes under `/api/skills*`:
//!
//!   GET /api/skills?q=<search>&limit=<n>&offset=<n>
//!     → paginated `SkillListResponse`. `q` is optional — when absent the
//!       endpoint returns the newest skills by `created_at DESC`; when
//!       present it runs an FTS5 BM25 search restricted to `kind="skill"`.
//!   GET /api/skills/:id
//!     → single `SkillDetail` (full provenance: raw FTS5 text + parsed
//!       fields). 404 if the row is missing or isn't a skill.
//!   GET /api/skill-timeline?since=<unix_secs_or_iso>
//!     → chronological `SkillTimelineResponse` of skills with
//!       `created_at >= since`. Oldest-first, capped at 1024.
//!
//! ## Auth
//!
//! All three routes reuse the existing broker-token middleware
//! (`auth_gate::require_cluster_auth`) — same `X-Cluster-Auth` HMAC scheme
//! as `/api/chat` and `/rpc/*`. For GET requests with no body, the HMAC is
//! computed over an empty body (already supported by `make_auth_token`).
//! No new auth scheme is invented.
//!
//! ## Perf
//!
//! - List endpoint: <500ms for 1000 skills (single SQL SELECT + COUNT + DTO
//!   marshal). Verified via `core/tests/skill_rpc_skills.rs::list_endpoint
//!   _meets_500ms_perf_gate_for_1000_skills` which seeds 1000 rows and
//!   asserts elapsed < 500ms.
//! - FTS5 search: p99 <200ms — relies on the existing FTS5 BM25 index from
//!   migration 0007. No dedicated bench in this PR (would shadow the
//!   existing `benches/fts5_memory_query.rs`); the integration test gives a
//!   smoke ceiling instead.
//!
//! Gated behind `experimental-memory` so the default cargo build is
//! byte-identical to baseline (no new routes registered, no new code
//! compiled).

#![cfg(feature = "experimental-memory")]

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth_gate::require_cluster_auth;
use crate::skillbank::dto::{
    detail_from_row, summary_from_row, timeline_entry_from_row, SkillListResponse,
    SkillTimelineResponse,
};
use crate::skillbank::memory::escape_fts5_query;
use crate::AppState;

/// Hard cap on `limit` per request — prevents a single client from pulling
/// the entire bank in one shot. 200 is plenty for any reasonable UI page;
/// the F404 web frontend uses 25 / page by default.
const MAX_PAGE_LIMIT: usize = 200;

/// Default page size when the caller omits `limit`.
const DEFAULT_PAGE_LIMIT: usize = 25;

/// F400 — attach the three skill routes to an existing axum
/// `Router<Arc<AppState>>`. Called from `serve::router` behind the same
/// feature flag.
pub fn attach_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/skills", get(list_skills))
        .route("/api/skills/:id", get(get_skill_detail))
        .route("/api/skill-timeline", get(get_skill_timeline))
}

// ─── query param shapes ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ListQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct TimelineQuery {
    /// Either a unix-seconds integer (`"1700000000"`) or an RFC-3339-ish
    /// ISO timestamp (`"2026-05-17T00:00:00Z"`). Missing → 0 (entire bank).
    #[serde(default)]
    since: Option<String>,
}

// ─── handlers ────────────────────────────────────────────────────────────

async fn list_skills(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
    body: Bytes, // present for HMAC; GET bodies are typically empty
) -> Response {
    if let Err((code, json)) = require_cluster_auth(&state.cluster_manager, &headers, &body) {
        return (code, json).into_response();
    }

    let mem = match state.skill_memory.as_ref() {
        Some(m) => m,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "skill memory not configured on this node"})),
            )
                .into_response()
        }
    };

    let limit = q
        .limit
        .unwrap_or(DEFAULT_PAGE_LIMIT)
        .clamp(1, MAX_PAGE_LIMIT);
    let offset = q.offset.unwrap_or(0);

    let result = match q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(query) => {
            let escaped = escape_fts5_query(query);
            mem.search_by_kind_paginated("skill", &escaped, limit, offset)
                .await
        }
        None => mem.list_by_kind("skill", limit, offset).await,
    };

    let (rows, total) = match result {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("skill query failed: {e}")})),
            )
                .into_response()
        }
    };

    let items = rows.iter().filter_map(summary_from_row).collect();
    let resp = SkillListResponse {
        items,
        total,
        limit,
        offset,
    };
    Json(resp).into_response()
}

async fn get_skill_detail(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    body: Bytes,
) -> Response {
    if let Err((code, json)) = require_cluster_auth(&state.cluster_manager, &headers, &body) {
        return (code, json).into_response();
    }

    let mem = match state.skill_memory.as_ref() {
        Some(m) => m,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "skill memory not configured on this node"})),
            )
                .into_response()
        }
    };

    let row = match mem.get_by_id(id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("no skill row with id={id}")})),
            )
                .into_response()
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("skill lookup failed: {e}")})),
            )
                .into_response()
        }
    };

    match detail_from_row(&row) {
        Some(detail) => Json(detail).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": format!("row id={id} is not a skill (kind={})", row.kind)
            })),
        )
            .into_response(),
    }
}

async fn get_skill_timeline(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<TimelineQuery>,
    body: Bytes,
) -> Response {
    if let Err((code, json)) = require_cluster_auth(&state.cluster_manager, &headers, &body) {
        return (code, json).into_response();
    }

    let mem = match state.skill_memory.as_ref() {
        Some(m) => m,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "skill memory not configured on this node"})),
            )
                .into_response()
        }
    };

    let since_secs = match q.since.as_deref() {
        None | Some("") => 0_i64,
        Some(raw) => match parse_since(raw) {
            Ok(v) => v,
            Err(msg) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("bad `since` value: {msg}")})),
                )
                    .into_response()
            }
        },
    };

    let rows = match mem.list_since("skill", since_secs, 1024).await {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("skill timeline query failed: {e}")})),
            )
                .into_response()
        }
    };

    let items: Vec<_> = rows.iter().filter_map(timeline_entry_from_row).collect();
    Json(SkillTimelineResponse {
        items,
        since: since_secs,
    })
    .into_response()
}

/// Parse `since` into unix seconds. Accepts:
///   * A bare integer (`"1700000000"`).
///   * A minimal RFC-3339-ish UTC timestamp (`"2026-05-17T00:00:00Z"`).
///
/// We deliberately keep the parser dependency-free (no `chrono` in the
/// default-features path) by handling only the fixed `YYYY-MM-DDTHH:MM:SSZ`
/// form. Anything else is rejected with a clear error so a misformed query
/// surfaces as 400 not "0 epoch".
fn parse_since(raw: &str) -> Result<i64, String> {
    let trimmed = raw.trim();
    if let Ok(n) = trimmed.parse::<i64>() {
        return Ok(n);
    }
    parse_iso_utc(trimmed).ok_or_else(|| {
        format!("expected unix-seconds integer or `YYYY-MM-DDTHH:MM:SSZ`, got `{trimmed}`")
    })
}

/// Minimal UTC ISO-8601 parser: `YYYY-MM-DDTHH:MM:SSZ` → unix seconds.
/// Returns `None` on any deviation. Computed via Howard Hinnant's
/// civil_from_days algorithm — exact, branchless, no allocations.
fn parse_iso_utc(s: &str) -> Option<i64> {
    if s.len() != 20 {
        return None;
    }
    let b = s.as_bytes();
    if b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return None;
    }
    let y: i32 = s[0..4].parse().ok()?;
    let m: u32 = s[5..7].parse().ok()?;
    let d: u32 = s[8..10].parse().ok()?;
    let hh: u32 = s[11..13].parse().ok()?;
    let mm: u32 = s[14..16].parse().ok()?;
    let ss: u32 = s[17..19].parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    // civil_from_days (year-month-day → days since 1970-01-01)
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = (y - era * 400) as u32; // 0..=399
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // 0..=365
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // 0..=146096
    let days: i64 = (era as i64) * 146097 + (doe as i64) - 719468;
    Some(days * 86400 + (hh as i64) * 3600 + (mm as i64) * 60 + (ss as i64))
}

#[allow(dead_code)]
fn _wire_version_envelope(v: Value) -> Value {
    // Matches the helper in `serve.rs` so future callers can converge on
    // one wrapper. Currently unused — the skill endpoints don't carry the
    // peer-wire envelope because they're UI-facing, not peer-facing.
    json!({ "wire_version": crate::WIRE_VERSION, "data": v })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_since_accepts_unix_seconds() {
        assert_eq!(parse_since("1700000000").unwrap(), 1_700_000_000);
        assert_eq!(parse_since("0").unwrap(), 0);
    }

    #[test]
    fn parse_since_accepts_iso_utc() {
        // 2026-05-17T00:00:00Z = 1778976000 (verified via Python datetime).
        let got = parse_since("2026-05-17T00:00:00Z").unwrap();
        assert_eq!(got, 1778976000, "got {got}");
    }

    #[test]
    fn parse_since_rejects_garbage() {
        assert!(parse_since("not-a-date").is_err());
        assert!(parse_since("2026-05-17").is_err()); // missing time
        assert!(parse_since("2026/05/17T00:00:00Z").is_err()); // wrong separator
    }

    #[test]
    fn parse_iso_utc_known_epoch_anchors() {
        assert_eq!(parse_iso_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_utc("2000-01-01T00:00:00Z"), Some(946_684_800));
    }
}
