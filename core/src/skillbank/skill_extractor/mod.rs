//! Skill Extractor — turns daily reviews (E003 output) into
//! skill candidates that the FTS5 memory backend can recall.
//!
//! Per E005 spec, input sources are pluggable. First impl
//! (`from_daily_review`) reads `~/.spectyn-mesh/reviews/<date>.md`.
//! Future inputs (raw conversations, captured-event clusters) drop in
//! as additional submodules sharing the same `SkillCandidate` shape.
//!
//! Backend-only — no UI, no DB write (a later slice will add a
//! `spectyn skill extract --date <date> --commit` CLI that persists to
//! `memory.db` via the existing FTS5 infra).

pub mod from_daily_review;

/// Provenance — where this candidate came from. Lets the Curator V2
/// judge cite specific source events, and lets the recall trace
/// surface "this skill was extracted from your 2026-05-22 review".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    pub source_kind: &'static str, // "daily_review" | "conversation" | ...
    pub source_date: String,       // ISO date for daily reviews
    pub event_count: usize,        // events that fed the source section
    pub analysis_quote: Option<String>, // first analysis summary from the section
}

/// A pre-skill that has not yet been judged or persisted. The Curator V2
/// ensemble is responsible for assigning `confidence` (out of scope for
/// this slice — left as `None`).
#[derive(Debug, Clone, PartialEq)]
pub struct SkillCandidate {
    pub title: String,
    pub body: String,
    pub goal_tag: String,
    pub provenance: Provenance,
    pub confidence: Option<f32>,
}
