//! Hermes-style modules. Hosts the experimental tool catalog (`tools`),
//! the experimental Skill Document parser (`skill`), the experimental
//! FTS5-backed memory store (`memory`), and the experimental LLM-as-judge
//! `curator` for evolve sessions. Module is always compiled so the path
//! resolves; submodules are feature-gated so default builds carry no
//! extra code.
//!
//! See docs/superpowers/specs/2026-05-15-weekend-multi-agent-push-design.md
//! §4 H1 (curator) + §5 H2 (skill) + §5 H3 (memory) + §5 H5 (tools).

#[cfg(feature = "experimental-hermes-tools")]
pub mod tools;

// E005 backend slice — Hermes Skill Extractor (production-default,
// reads E003 daily reviews). No feature gate: this is the v0.6.0
// shipped extraction path, not an experimental track.
pub mod skill_extractor;

#[cfg(feature = "experimental-hermes-curator")]
pub mod skill;

#[cfg(feature = "experimental-hermes-curator")]
pub mod curator;

#[cfg(feature = "experimental-hermes-curator")]
pub mod skill_executor;

#[cfg(feature = "experimental-hermes-curator")]
pub mod curator_ensemble;

#[cfg(feature = "experimental-hermes-curator")]
pub mod extract;

#[cfg(feature = "experimental-hermes-memory")]
pub mod memory;

#[cfg(feature = "experimental-hermes-memory")]
pub mod dto;

#[cfg(feature = "experimental-hermes-memory")]
pub use memory::{escape_fts5_query, HermesMemory, MemoryRow, NewMemory};

#[cfg(feature = "experimental-hermes-memory")]
pub use dto::{
    derive_polarity, detail_from_row, split_skill_text, summary_from_row, timeline_entry_from_row,
    SkillDetail, SkillListResponse, SkillSummary, SkillTimelineEntry, SkillTimelineResponse,
};

#[cfg(feature = "experimental-hermes-curator")]
pub use curator::{
    build_judge_user_prompt, parse_judge_reply, parse_judge_reply_strict, verdict_from_parsed,
    NoopSkillExtractor, SkillExtractor, CONFIDENCE_THRESHOLD, DEFAULT_JUDGE_MODEL,
    MAX_RATIONALE_CHARS, RUBRIC_VERSION,
};

#[cfg(feature = "experimental-hermes-curator")]
pub use curator_ensemble::{
    aggregate, median_score, population_stddev, AnthropicJudge, EnsembleCurator, EnsembleOutcome,
    JudgeError, JudgeProvider, OpenAICompatJudge,
};

#[cfg(feature = "experimental-hermes-curator")]
pub use skill_executor::{
    ExecutionMode, ExecutionOpts, SkillExecError, SkillExecutionResult, SkillExecutor, SkillStep,
    StepOutcome,
};

#[cfg(feature = "experimental-hermes-curator")]
pub use extract::{
    extract_skill, extract_skill_from_failure, extract_skill_from_success,
    extract_skill_with_threshold, DEFAULT_SCORE_THRESHOLD,
};

// Integration façade: requires all three Hermes sub-features at once.
// Gated by the cfg(all(...)) inside integration.rs.
#[cfg(all(
    feature = "experimental-hermes-curator",
    feature = "experimental-hermes-memory",
    feature = "experimental-hermes-tools",
))]
pub mod integration;

#[cfg(all(
    feature = "experimental-hermes-curator",
    feature = "experimental-hermes-memory",
    feature = "experimental-hermes-tools",
))]
pub use integration::{
    HermesRuntime, MEMORY_CONTEXT_HEADER, MEMORY_CONTEXT_MAX_ROWS, MEMORY_ROW_CHAR_CAP,
};
