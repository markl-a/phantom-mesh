//! Skillbank — skill catalog, memory store, curator/judge, and tool catalog.
//! the experimental Skill Document parser (`skill`), the experimental
//! FTS5-backed memory store (`memory`), and the experimental LLM-as-judge
//! `curator` for evolve sessions. Module is always compiled so the path
//! resolves; submodules are feature-gated so default builds carry no
//! extra code.
//!
//! See docs/superpowers/specs/2026-05-15-weekend-multi-agent-push-design.md
//! §4 H1 (curator) + §5 H2 (skill) + §5 H3 (memory) + §5 H5 (tools).

#[cfg(feature = "experimental-tools")]
pub mod tools;

// E005 backend slice — skill extractor (production-default,
// reads E003 daily reviews). No feature gate: this is the v0.6.0
// shipped extraction path, not an experimental track.
pub mod skill_extractor;

#[cfg(feature = "experimental-curator")]
pub mod skill;

#[cfg(feature = "experimental-curator")]
pub mod curator;

#[cfg(feature = "experimental-curator")]
pub mod skill_executor;

#[cfg(feature = "experimental-curator")]
pub mod curator_ensemble;

#[cfg(feature = "experimental-curator")]
pub mod extract;

#[cfg(feature = "experimental-memory")]
pub mod memory;

// P0-8: at-rest sealing adapter for the owned-memory store. Always compiled:
// `skill_wire` (an ungated module) uses `memory_seal::{seal,open,is_sealed,
// fts_index_form,memory_e2ee_enabled}` unconditionally, and this module only
// depends on always-compiled code (`encryption_wire` + `life_node::crypto`),
// so gating it behind the memory feature broke the default build (E0433).
pub mod memory_seal;

// apex P4: at-rest sealing for `agents.toml` provider API keys. Always compiled
// (NOT feature-gated): it is wired into the always-compiled `config::load_path`
// + `keys::set_api_key`, and only depends on `memory_seal` (itself always
// compiled) + `config`. Gating it would break the default `cargo check --lib`.
pub mod agents_seal;

/// SPEC-25 §8.7 cross-peer skill sync ingest core (verify + decrypt + LWW-merge).
/// Default-on (no feature gate): the `/rpc/skill/sync` handler is a shipping
/// security surface, and the crypto deps are already in the base build.
pub mod sync;

#[cfg(feature = "experimental-memory")]
pub mod dto;

#[cfg(feature = "experimental-memory")]
pub use memory::{escape_fts5_query, SkillMemory, MemoryRow, NewMemory};

#[cfg(feature = "experimental-memory")]
pub use dto::{
    derive_polarity, detail_from_row, split_skill_text, summary_from_row, timeline_entry_from_row,
    SkillDetail, SkillListResponse, SkillSummary, SkillTimelineEntry, SkillTimelineResponse,
};

#[cfg(feature = "experimental-curator")]
pub use curator::{
    build_judge_user_prompt, parse_judge_reply, parse_judge_reply_strict, verdict_from_parsed,
    NoopSkillExtractor, SkillExtractor, CONFIDENCE_THRESHOLD, DEFAULT_JUDGE_MODEL,
    MAX_RATIONALE_CHARS, RUBRIC_VERSION,
};

#[cfg(feature = "experimental-curator")]
pub use curator_ensemble::{
    aggregate, median_score, population_stddev, AnthropicJudge, EnsembleCurator, EnsembleOutcome,
    JudgeError, JudgeProvider, OpenAICompatJudge,
};

#[cfg(feature = "experimental-curator")]
pub use skill_executor::{
    ExecutionMode, ExecutionOpts, SkillExecError, SkillExecutionResult, SkillExecutor, SkillStep,
    StepOutcome,
};

#[cfg(feature = "experimental-curator")]
pub use extract::{
    extract_skill, extract_skill_from_failure, extract_skill_from_success,
    extract_skill_with_threshold, DEFAULT_SCORE_THRESHOLD,
};

// Integration façade: requires all three skillbank sub-features at once.
// Gated by the cfg(all(...)) inside integration.rs.
#[cfg(all(
    feature = "experimental-curator",
    feature = "experimental-memory",
    feature = "experimental-tools",
))]
pub mod integration;

#[cfg(all(
    feature = "experimental-curator",
    feature = "experimental-memory",
    feature = "experimental-tools",
))]
pub use integration::{
    SkillbankRuntime, MEMORY_CONTEXT_HEADER, MEMORY_CONTEXT_MAX_ROWS, MEMORY_ROW_CHAR_CAP,
};

// Top-down skill synthesis (counterpart to `extract`). Needs the curator
// track (skill parser + executor) and the memory track (store), so it carries
// the same cfg(all(...)) gate as its `#![cfg(...)]` file attribute.
#[cfg(all(
    feature = "experimental-curator",
    feature = "experimental-memory"
))]
pub mod synthesize;
