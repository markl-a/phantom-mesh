//! F400 — Flat JSON DTOs for the skill RPC endpoints.
//!
//! The mobile (F403) and web (F404) UIs render skill cards / timelines from
//! these shapes. Keep them deliberately flat and stable: the internal
//! `MemoryRow` struct mixes concerns (tools, skills, judge verdicts, lessons
//! all share one row type), and we don't want UI clients to learn that
//! coupling. The DTO layer also lets us *enrich* a row with parsed-out
//! signals — polarity, Curator confidence — without bloating `MemoryRow`.
//!
//! Gated behind `experimental-memory` so the default build doesn't
//! pull in `serde::Serialize` impls it never serializes.
//!
//! Spec source: user-provided F400 quick-summary (no on-disk spec yet, per
//! the parent feature ticket).

#![cfg(feature = "experimental-memory")]

use serde::Serialize;

use crate::skillbank::memory::MemoryRow;

// TODO: unify once SPEC-25 stage 4 lands. The wire type
// `crate::skill_wire::SkillSummary` is an *aggregate dashboard* shape
// (count_total / count_active / last_extracted_at / top_3_by_score) used by
// `spectyn skill status`; this `SkillSummary` is a *per-row* shape used by
// the F400 list/detail endpoints. They share a name but encode different
// concepts — same-name collision is intentional (each name describes its
// own surface) and bridging via `From` would mislead callers. Wave 12
// review flagged the overlap; Stage 4 will rename one of them
// (likely `skillbank::dto::SkillSummary` → `SkillListEntry`) once SPEC-25 §9.7
// formalises the dashboard contract.

/// Single skill entry in the list/detail response. Flat by design — the UI
/// can render this without any join logic.
///
/// Fields:
/// * `id`               — opaque numeric handle (FTS5 rowid). Stable for the
///                        life of the DB; pass back to `/api/skills/:id`.
/// * `created_at`       — unix seconds (UTC) when the row was written.
/// * `source`           — `skill_bank` (seed) | `skill_auto_register`
///                        (extracted from a successful/failed run) | future.
/// * `name`             — first line of `text` (skill name).
/// * `description`      — second line of `text` (skill description), if present.
/// * `body`             — remainder of `text` (Markdown body).
/// * `tags`             — flattened tag tokens (space-separated → Vec).
/// * `polarity`         — `"success"` | `"failure"` | `"unknown"`, derived from
///                        tags (`recipe`/`lesson` markers — see [`derive_polarity`]).
/// * `curator_score`    — 0..=10 if the row's source links to a known
///                        verdict, else `None`. Always `None` on plain seed
///                        rows; future enrichment hook.
// NOTE: there are 3 `SkillSummary` types in this codebase, intentionally. Each
// serves a distinct aggregation purpose; module path disambiguates. See
// docs/superpowers/skill-summary-naming.md for the design rationale.
//   • `skillbank::dto::SkillSummary`  — THIS one: full skill record for the
//     `/api/skills` HTTP paginated list (9 fields incl. body Markdown).
//   • `skill_wire::SkillSummary`   — dashboard overview (4 fields: counts +
//     top-3 names) for `spectyn skill status` card.
//   • `rpc_wire::SkillSummary`     — peer-to-peer sync delta (5 fields)
//     for `/rpc/skill/since/:ts` mesh sync.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SkillSummary {
    pub id: i64,
    pub created_at: i64,
    pub source: String,
    pub name: String,
    pub description: String,
    pub body: String,
    pub tags: Vec<String>,
    pub polarity: String,
    pub curator_score: Option<u8>,
}

/// Full skill detail. Currently identical to [`SkillSummary`] (no extra
/// fields), but kept as a distinct type so a future spec bump can attach
/// provenance bundles (e.g. linked verdict id, session_id) without breaking
/// the list response shape.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SkillDetail {
    #[serde(flatten)]
    pub summary: SkillSummary,
    /// Raw, unparsed FTS5 body — what the curator/extractor actually wrote.
    /// UIs that want the canonical Markdown (frontmatter + body) round-trip
    /// it via the F404 client; we serve the bytes verbatim.
    pub raw_text: String,
}

/// Paginated list response.
#[derive(Debug, Clone, Serialize)]
pub struct SkillListResponse {
    pub items: Vec<SkillSummary>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}

/// Single timeline entry — chronological "today's learnings" view.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SkillTimelineEntry {
    pub id: i64,
    pub created_at: i64,
    pub name: String,
    pub polarity: String,
    pub source: String,
}

/// Timeline response (`since` timestamp echoed back so the UI can paginate
/// forward by passing back the max `created_at` it saw).
#[derive(Debug, Clone, Serialize)]
pub struct SkillTimelineResponse {
    pub items: Vec<SkillTimelineEntry>,
    pub since: i64,
}

/// Split `text` (as stored in `hermes_memory`) into `(name, description, body)`.
///
/// Convention from `integration.rs::register_extracted_skill`:
///   `text` is either:
///     A. The serialized skill — `---\n<yaml>\n---\n<markdown body>` (auto-
///        register path). First non-fence line of YAML carries `name:`.
///     B. The seed format `<name>\n<description>\n<body>` (sample-skill seed
///        path).
///
/// We accept both. Heuristic:
///   * If `text` starts with `---\n` → parse YAML frontmatter, pull `name:`
///     and `description:` lines; the body is everything after the closing
///     fence.
///   * Otherwise → split on `\n` once for name, once for description.
pub fn split_skill_text(text: &str) -> (String, String, String) {
    if let Some(rest) = text.strip_prefix("---\n") {
        if let Some((yaml, body)) = rest.split_once("\n---\n") {
            let mut name = String::new();
            let mut description = String::new();
            for line in yaml.lines() {
                if let Some(v) = line.strip_prefix("name:") {
                    name = v.trim().trim_matches('"').to_string();
                } else if let Some(v) = line.strip_prefix("description:") {
                    description = v.trim().trim_matches('"').to_string();
                }
            }
            return (name, description, body.to_string());
        }
    }

    // Fallback: line-split format.
    let mut lines = text.splitn(3, '\n');
    let name = lines.next().unwrap_or("").to_string();
    let description = lines.next().unwrap_or("").to_string();
    let body = lines.next().unwrap_or("").to_string();
    (name, description, body)
}

/// Derive a coarse polarity classification from a skill row's tag string.
///
/// The A8 extractor emits two skill flavours:
///   * **success** → tags include `recipe`, `solution_pattern`, `workflow_pattern`,
///     `confirmed_hypothesis`, or `stuck_pattern_resolved`.
///   * **failure** → tags include `lesson`, `retry_loop`, `wrong_hypothesis`,
///     `wrong_direction`, or `stuck`.
///
/// Everything else (seeds, manual inserts) maps to `"unknown"` so the UI can
/// fall through to a neutral icon.
pub fn derive_polarity(tags: &str) -> &'static str {
    const SUCCESS: &[&str] = &[
        "recipe",
        "solution_pattern",
        "workflow_pattern",
        "confirmed_hypothesis",
        "stuck_pattern_resolved",
    ];
    const FAILURE: &[&str] = &[
        "lesson",
        "retry_loop",
        "wrong_hypothesis",
        "wrong_direction",
        "stuck",
    ];
    let tokens: Vec<&str> = tags.split_whitespace().collect();
    if tokens.iter().any(|t| SUCCESS.contains(t)) {
        "success"
    } else if tokens.iter().any(|t| FAILURE.contains(t)) {
        "failure"
    } else {
        "unknown"
    }
}

/// Adapter: build a [`SkillSummary`] from a stored `MemoryRow`.
///
/// Returns `None` if the row is not a skill (defensive — endpoints filter
/// `kind="skill"` at the SQL layer, but this guards against accidental
/// misuse by future callers).
pub fn summary_from_row(row: &MemoryRow) -> Option<SkillSummary> {
    if row.kind != "skill" {
        return None;
    }
    let (name, description, body) = split_skill_text(&row.text);
    let polarity = derive_polarity(&row.tags).to_string();
    let tags: Vec<String> = row.tags.split_whitespace().map(|s| s.to_string()).collect();
    Some(SkillSummary {
        id: row.id,
        created_at: row.created_at,
        source: row.source.clone(),
        name,
        description,
        body,
        tags,
        polarity,
        curator_score: None,
    })
}

/// Adapter: build a [`SkillDetail`] from a stored `MemoryRow`. See
/// `summary_from_row` for the polarity/parse rules.
pub fn detail_from_row(row: &MemoryRow) -> Option<SkillDetail> {
    let summary = summary_from_row(row)?;
    Some(SkillDetail {
        summary,
        raw_text: row.text.clone(),
    })
}

/// Adapter: build a [`SkillTimelineEntry`] from a stored `MemoryRow`.
pub fn timeline_entry_from_row(row: &MemoryRow) -> Option<SkillTimelineEntry> {
    if row.kind != "skill" {
        return None;
    }
    let (name, _, _) = split_skill_text(&row.text);
    Some(SkillTimelineEntry {
        id: row.id,
        created_at: row.created_at,
        name,
        polarity: derive_polarity(&row.tags).to_string(),
        source: row.source.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, created: i64, kind: &str, source: &str, text: &str, tags: &str) -> MemoryRow {
        MemoryRow {
            id,
            created_at: created,
            kind: kind.into(),
            source: source.into(),
            text: text.into(),
            tags: tags.into(),
        }
    }

    #[test]
    fn split_skill_text_handles_seed_format() {
        let (n, d, b) =
            split_skill_text("rebase-onto-main\nRebase the current branch.\nstep 1\nstep 2\n");
        assert_eq!(n, "rebase-onto-main");
        assert_eq!(d, "Rebase the current branch.");
        assert!(b.starts_with("step 1"));
    }

    #[test]
    fn split_skill_text_handles_serialized_frontmatter() {
        let text = "---\nname: my-skill\nversion: 0.1.0\ndescription: A test skill.\ntriggers:\n  - go\n---\n# Body\n\nHello.\n";
        let (n, d, b) = split_skill_text(text);
        assert_eq!(n, "my-skill");
        assert_eq!(d, "A test skill.");
        assert!(b.starts_with("# Body"));
    }

    #[test]
    fn derive_polarity_marks_success_recipes() {
        assert_eq!(derive_polarity("recipe rebase git"), "success");
        assert_eq!(derive_polarity("workflow_pattern"), "success");
    }

    #[test]
    fn derive_polarity_marks_failure_lessons() {
        assert_eq!(derive_polarity("lesson retry_loop"), "failure");
        assert_eq!(derive_polarity("stuck"), "failure");
    }

    #[test]
    fn derive_polarity_unknown_when_no_marker() {
        assert_eq!(derive_polarity(""), "unknown");
        assert_eq!(derive_polarity("git arbitrary"), "unknown");
    }

    #[test]
    fn summary_from_row_rejects_non_skill_kinds() {
        let r = row(1, 100, "tool", "x", "name\ndesc\nbody", "");
        assert!(summary_from_row(&r).is_none());
    }

    #[test]
    fn summary_from_row_parses_skill_row() {
        let r = row(
            7,
            1700000000,
            "skill",
            "skill_bank",
            "my-skill\nA cool one.\nstep 1\n",
            "recipe foo",
        );
        let s = summary_from_row(&r).expect("skill row");
        assert_eq!(s.id, 7);
        assert_eq!(s.name, "my-skill");
        assert_eq!(s.description, "A cool one.");
        assert_eq!(s.polarity, "success");
        assert_eq!(s.tags, vec!["recipe", "foo"]);
        assert_eq!(s.source, "skill_bank");
        assert!(s.curator_score.is_none());
    }

    #[test]
    fn detail_from_row_preserves_raw_text() {
        let r = row(
            8,
            1700000001,
            "skill",
            "skill_auto_register",
            "x\ny\nz\n",
            "lesson",
        );
        let d = detail_from_row(&r).expect("detail");
        assert_eq!(d.raw_text, "x\ny\nz\n");
        assert_eq!(d.summary.polarity, "failure");
    }

    #[test]
    fn timeline_entry_minimal_shape() {
        let r = row(
            9,
            1700000002,
            "skill",
            "skill_bank",
            "name\ndesc\n",
            "recipe",
        );
        let e = timeline_entry_from_row(&r).expect("entry");
        assert_eq!(e.id, 9);
        assert_eq!(e.name, "name");
        assert_eq!(e.polarity, "success");
    }
}
