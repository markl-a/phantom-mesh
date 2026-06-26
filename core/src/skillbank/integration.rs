//! Skillbank runtime integration façade.
//!
//! Wires the four skillbank submodules (curator, skill, memory,
//! tools) into a single coherent runtime so a session can:
//!
//! 1. Run an agent and produce an `EvolveCheckpoint`.
//! 2. Ask the Curator to score the checkpoint (existing PR #33 work).
//! 3. Automatically persist the verdict into FTS5 memory keyed by the
//!    checkpoint's session_id so future runs can recall past judgments.
//! 4. Pre-seed memory with the tool catalog + the sample skill at startup
//!    so the agent can search for tools by intent.
//!
//! Gated behind the umbrella feature `experimental-skillbank`. The four
//! sub-features (-curator, -memory, -tools) must all be enabled for this
//! file to compile; the umbrella turns them on together.

#![cfg(all(
    feature = "experimental-curator",
    feature = "experimental-memory",
    feature = "experimental-tools",
))]

use std::path::PathBuf;

use anyhow::Result;

use crate::skillbank::curator::{NoopSkillExtractor, SkillExtractor};
use crate::skillbank::memory::SkillMemory;
use crate::skillbank::skill::{parse_str as parse_skill, serialize as serialize_skill, SkillDocument};
use crate::skillbank::tools::{catalog, SkillTool};

/// Façade that owns the four skillbank subsystems and the wiring between them.
///
/// Construct once per process via [`SkillbankRuntime::new`]; clone (cheap — all
/// owned state is `Arc`-backed inside `SkillMemory`) into worker tasks as
/// needed. The runtime is `Send + Sync` by virtue of its members.
pub struct SkillbankRuntime {
    memory: SkillMemory,
    #[allow(dead_code)]
    tools: Vec<Box<dyn SkillTool>>,
    curator: Option<crate::skillbank::curator::Curator>,
    /// A2/T92: pluggable skill extractor used by `judge_and_auto_register`.
    /// Defaults to `NoopSkillExtractor` so the orchestration path exists
    /// before A1 (the real extractor) lands.
    extractor: Box<dyn SkillExtractor>,
}

impl SkillbankRuntime {
    /// Open (or create) the FTS5 memory database at `db_path`, build the
    /// in-memory tool catalog, and seed the catalog (one row per tool) into
    /// FTS5 memory. Idempotent: re-opening an existing DB does not duplicate
    /// rows. See Task 4 in the integration plan.
    pub async fn new(db_path: PathBuf) -> Result<Self> {
        use crate::skillbank::memory::NewMemory;

        let memory = SkillMemory::open_at(db_path)?;
        let tools = catalog();

        // Seed: one row per tool. Each row's `text` body is
        // `<name>\n<description>` so FTS5 BM25 can rank both name and
        // description tokens. Tags carry the bare name so callers can
        // post-filter. Idempotency: probe for an existing kind=tool row
        // whose body starts with this tool's name; skip if present.
        for tool in tools.iter() {
            let schema = tool.schema();
            let description = schema
                .get("function")
                .and_then(|f| f.get("description"))
                .and_then(|d| d.as_str())
                .unwrap_or("");
            let text = format!("{}\n{}", tool.name(), description);
            let probe = crate::skillbank::memory::escape_fts5_query(tool.name());
            let existing = match memory.search(&probe, 5).await {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::warn!(
                        tool = tool.name(),
                        "tool dedup search failed, skipping insert to avoid duplicate: {}",
                        e
                    );
                    continue;
                }
            };
            if existing
                .iter()
                .any(|r| r.kind == "tool" && r.text.starts_with(tool.name()))
            {
                continue;
            }
            memory
                .insert(NewMemory {
                    kind: "tool",
                    source: "skill_catalog",
                    text: &text,
                    tags: tool.name(),
                })
                .await?;
        }

        // Sample skill seeding: embed the canonical sample at compile time so
        // the runtime works in environments without the docs/ directory on
        // disk (e.g., installed binary). Parsing is infallible because the
        // file is shipped in-tree and round-trip tested by skill.rs.
        const SAMPLE_SKILL_MD: &str = include_str!("../../../docs/skills/sample-skill.md");
        let skill: SkillDocument = parse_skill(SAMPLE_SKILL_MD)
            .map_err(|e| anyhow::anyhow!("parse embedded sample skill: {}", e))?;

        let skill_text = format!(
            "{}\n{}\n{}",
            skill.frontmatter.name, skill.frontmatter.description, skill.body
        );
        let skill_tags = skill.frontmatter.tags.join(" ");
        let probe = crate::skillbank::memory::escape_fts5_query(&skill.frontmatter.name);
        // Fail-closed on DB search error: skip insert to avoid silently creating
        // duplicate skill rows on every boot. Log so operator can see DB drift.
        let search_result = memory.search(&probe, 5).await;
        let skip_insert = match &search_result {
            Ok(rows) => rows.iter().any(|r| r.kind == "skill"),
            Err(e) => {
                tracing::warn!(skill = %skill.frontmatter.name, "skill dedup search failed, skipping insert to avoid duplicate: {}", e);
                true
            }
        };
        if !skip_insert {
            memory
                .insert(NewMemory {
                    kind: "skill",
                    source: "skill_bank",
                    text: &skill_text,
                    tags: &skill_tags,
                })
                .await?;
        }

        Ok(Self {
            memory,
            tools,
            curator: None,
            extractor: Box::new(NoopSkillExtractor),
        })
    }

    /// A2/T92: install (or replace) the skill extractor used by
    /// `judge_and_auto_register`. Defaults to a no-op extractor so the call
    /// path compiles before A1 lands; tests inject mocks to drive coverage.
    pub fn set_skill_extractor(&mut self, extractor: Box<dyn SkillExtractor>) {
        self.extractor = extractor;
    }

    /// Install (or replace) the Curator used by `judge_and_record`.
    /// Callers that want to run a real Anthropic round-trip construct the
    /// Curator with `api_base = "https://api.anthropic.com"` and a live API
    /// key; tests pass a wiremock URL.
    pub fn set_curator(&mut self, curator: crate::skillbank::curator::Curator) {
        self.curator = Some(curator);
    }

    /// Score `checkpoint` via the installed Curator and persist the verdict
    /// into FTS5 memory.
    ///
    /// On success the checkpoint carries the `JudgeVerdict` and memory has a
    /// new row of kind=`judge_verdict` whose tags contain the
    /// `checkpoint.session_id` so `recall_verdicts_for` can find it later.
    ///
    /// Returns an error (and writes nothing) if:
    /// - no Curator has been installed (`set_curator` not called),
    /// - the Curator HTTP call fails,
    /// - the LLM reply doesn't parse.
    pub async fn judge_and_record(
        &self,
        checkpoint: &mut crate::evolve_checkpoint::EvolveCheckpoint,
    ) -> Result<()> {
        let curator = self
            .curator
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no curator installed — call set_curator first"))?;

        curator
            .judge(checkpoint)
            .await
            .map_err(|e| anyhow::anyhow!("curator.judge failed: {}", e))?;

        // Curator wrote the verdict onto the checkpoint. Persist it.
        let v = checkpoint
            .judge_score
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("curator returned ok but no verdict was recorded"))?;

        let text = format!(
            "score={} rubric={} model={} rationale={}",
            v.score, v.rubric_version, v.model, v.rationale
        );
        // Tags carry the session_id (so recall_verdicts_for can find it) and
        // a stable kind marker.
        let tags = format!("{} verdict", checkpoint.session_id);

        self.memory
            .insert(crate::skillbank::memory::NewMemory {
                kind: "judge_verdict",
                source: "skill_curator",
                text: &text,
                tags: &tags,
            })
            .await?;

        Ok(())
    }

    /// A2/T92: judge `checkpoint`, persist the verdict (same as
    /// `judge_and_record`), and — if the verdict clears the curator's
    /// `CONFIDENCE_THRESHOLD` — extract a skill from `agent_output` and
    /// auto-register it in FTS5 memory.
    ///
    /// Returns `(verdict, registered_skill_name)`:
    ///   - `registered_skill_name = Some(name)` ⇒ a new `kind="skill"` row
    ///     was inserted (or would have been; see idempotency probe).
    ///   - `registered_skill_name = None` ⇒ either the score was below
    ///     threshold, OR the extractor produced no skill, OR an identical
    ///     skill already existed (idempotency hit).
    ///
    /// Idempotency mirrors the `seed_sample_skill` probe in `new()`
    /// (integration.rs:99-109): a name-prefixed FTS5 probe rejects re-inserts
    /// of an existing `kind="skill"` row, so calling this twice with the
    /// same agent_output writes only one row.
    pub async fn judge_and_auto_register(
        &self,
        checkpoint: &mut crate::evolve_checkpoint::EvolveCheckpoint,
        agent_output: &str,
    ) -> Result<(crate::evolve_checkpoint::JudgeVerdict, Option<String>)> {
        let curator = self
            .curator
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no curator installed — call set_curator first"))?;

        let (verdict, maybe_skill) = curator
            .judge_and_maybe_extract(checkpoint, agent_output, self.extractor.as_ref())
            .await
            .map_err(|e| anyhow::anyhow!("curator.judge_and_maybe_extract failed: {}", e))?;

        // Always persist the verdict — matches `judge_and_record` semantics.
        let v_text = format!(
            "score={} rubric={} model={} rationale={}",
            verdict.score, verdict.rubric_version, verdict.model, verdict.rationale
        );
        let v_tags = format!("{} verdict", checkpoint.session_id);
        self.memory
            .insert(crate::skillbank::memory::NewMemory {
                kind: "judge_verdict",
                source: "skill_curator",
                text: &v_text,
                tags: &v_tags,
            })
            .await?;

        // Try to register the extracted skill (if any).
        let registered = match maybe_skill {
            Some(skill) => self.register_extracted_skill(&skill).await?,
            None => None,
        };

        Ok((verdict, registered))
    }

    /// A2/T92: insert an extracted `SkillDocument` into FTS5 memory, with
    /// the same name-probe idempotency check that `new()` uses for the sample
    /// skill. Returns the skill's name iff a row was actually written.
    async fn register_extracted_skill(&self, skill: &SkillDocument) -> Result<Option<String>> {
        // Probe FTS5 for an existing kind=skill row whose body starts with
        // this name. If found, skip — the caller already has it.
        let probe = crate::skillbank::memory::escape_fts5_query(&skill.frontmatter.name);
        // Fail-closed on DB search error: skip insert to avoid silently creating
        // duplicate skill rows on every extraction. Log so operator can see DB drift.
        match self.memory.search(&probe, 5).await {
            Ok(rows) if rows.iter().any(|r| r.kind == "skill") => return Ok(None),
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(skill = %skill.frontmatter.name, "skill dedup search failed, skipping insert to avoid duplicate: {}", e);
                return Ok(None);
            }
        }

        // Serialize the whole skill (frontmatter + body) so we can round-trip
        // it back into a SkillDocument later if a consumer wants the structured
        // form. The `text` field is what FTS5 indexes for BM25.
        let serialized = serialize_skill(skill)
            .map_err(|e| anyhow::anyhow!("serialize extracted skill: {}", e))?;
        let tags = skill.frontmatter.tags.join(" ");

        self.memory
            .insert(crate::skillbank::memory::NewMemory {
                kind: "skill",
                source: "skill_auto_register",
                text: &serialized,
                tags: &tags,
            })
            .await?;
        Ok(Some(skill.frontmatter.name.clone()))
    }

    /// Recall past judge verdicts whose tags include `sha_or_session_id`.
    ///
    /// `sha_or_session_id` may be a git commit SHA (when callers identify
    /// checkpoints by their committed SHA) or an evolve session_id (when
    /// recalling within a single autoevolve loop). The argument is FTS5-
    /// escaped, so callers may pass arbitrary user-controlled strings safely.
    ///
    /// Empty input returns an empty Vec without touching the DB.
    pub async fn recall_verdicts_for(
        &self,
        sha_or_session_id: &str,
    ) -> Result<Vec<crate::skillbank::memory::MemoryRow>> {
        let trimmed = sha_or_session_id.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let escaped = crate::skillbank::memory::escape_fts5_query(trimmed);
        let rows = self.memory.search(&escaped, 25).await?;
        Ok(rows
            .into_iter()
            .filter(|r| r.kind == "judge_verdict")
            .collect())
    }

    /// Search the seeded tool catalog by free-text intent. Returns tool names
    /// (canonical `name()` strings) ordered by FTS5 BM25 rank, best-first.
    /// Empty/whitespace-only queries return an empty Vec without touching the
    /// DB (FTS5 rejects empty MATCH expressions).
    ///
    /// Multi-token queries are split on whitespace; each token is wrapped as
    /// an FTS5 literal phrase and joined with `OR` so a user-typed "arithmetic
    /// expression" matches any tool whose body contains either word. Using
    /// per-token phrase wrapping (vs. one giant phrase) keeps semantics
    /// recall-friendly while still neutralizing FTS5 operator syntax for each
    /// individual token.
    pub async fn find_tool_by_intent(&self, query: &str) -> Result<Vec<String>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let escaped = trimmed
            .split_whitespace()
            .map(crate::skillbank::memory::escape_fts5_query)
            .collect::<Vec<_>>()
            .join(" OR ");
        let rows = self.memory.search(&escaped, 25).await?;
        Ok(rows
            .into_iter()
            .filter(|r| r.kind == "tool")
            .map(|r| r.tags) // tags carries the bare tool name (see Task 4)
            .collect())
    }

    /// Test-only accessor — keeps the memory field private otherwise.
    #[cfg(test)]
    pub(crate) fn memory_for_test(&self) -> &SkillMemory {
        &self.memory
    }

    /// Recall up to `k` past memory rows (any kind) whose body matches the
    /// caller-supplied `prompt`. Used by [`crate::agent::AgentRuntime`] to
    /// pre-seed the system prompt with relevant prior knowledge before each
    /// turn (Task 6 of the integration plan: "close the loop — agent reads
    /// what it learned last time").
    ///
    /// The query is FTS5-escaped per [`escape_fts5_query`] so any user text
    /// is safe to pass directly. Multi-token prompts are split on whitespace
    /// and joined with `OR` for recall-friendly matching (same strategy as
    /// [`SkillbankRuntime::find_tool_by_intent`]). Each returned string is
    /// hard-capped at [`MEMORY_ROW_CHAR_CAP`] (200) chars to keep the token
    /// budget impact predictable regardless of how verbose past memory rows
    /// were.
    ///
    /// Returns an empty Vec for empty/whitespace-only prompts (FTS5 rejects
    /// empty MATCH expressions).
    pub async fn recall_context_for(&self, prompt: &str, k: usize) -> Result<Vec<String>> {
        let trimmed = prompt.trim();
        if trimmed.is_empty() || k == 0 {
            return Ok(Vec::new());
        }
        let escaped = trimmed
            .split_whitespace()
            .map(crate::skillbank::memory::escape_fts5_query)
            .collect::<Vec<_>>()
            .join(" OR ");
        let rows = self.memory.search(&escaped, k).await?;
        Ok(rows
            .into_iter()
            .take(k)
            .map(|r| {
                let mut t = r.text;
                if t.len() > MEMORY_ROW_CHAR_CAP {
                    // Truncate on a UTF-8 char boundary to avoid panics.
                    let mut end = MEMORY_ROW_CHAR_CAP;
                    while end > 0 && !t.is_char_boundary(end) {
                        end -= 1;
                    }
                    t.truncate(end);
                    t.push('…');
                }
                t
            })
            .collect())
    }
}

/// Per-row character cap applied by [`SkillbankRuntime::recall_context_for`] so
/// the agent prompt never blows up when prior memory rows are unusually long.
/// 200 chars ≈ 50 tokens at the agent's 4-chars/token estimator (see
/// `agent.rs::estimate_message_tokens`).
pub const MEMORY_ROW_CHAR_CAP: usize = 200;

/// Maximum memory rows ever injected into the agent's system prompt by the
/// integration. Keeps the worst-case token impact bounded at
/// `MEMORY_CONTEXT_MAX_ROWS * MEMORY_ROW_CHAR_CAP / 4` ≈ 250 tokens.
pub const MEMORY_CONTEXT_MAX_ROWS: usize = 5;

/// Header that prefixes the memory block injected into the system prompt.
/// Used as a stable cut line by `agent.rs::compact_if_needed` so the block
/// is the first thing dropped when the prompt overflows the token budget.
pub const MEMORY_CONTEXT_HEADER: &str = "[memory]";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evolve_checkpoint::EvolveCheckpoint;
    use crate::skillbank::curator::{Curator, DEFAULT_JUDGE_MODEL};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn anthropic_text_response(text: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "msg_test",
            "type": "message",
            "role": "assistant",
            "model": DEFAULT_JUDGE_MODEL,
            "content": [{"type": "text", "text": text}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })
    }

    #[tokio::test]
    async fn new_opens_db_at_given_path() {
        let td = tempfile::tempdir().unwrap();
        let db_path = td.path().join("hermes-runtime.db");
        let rt = SkillbankRuntime::new(db_path.clone()).await.expect("new ok");
        // The DB file must exist after construction.
        assert!(db_path.exists(), "db file should be created");
        // And the memory backend is reachable.
        let _ = rt.memory_for_test();
    }

    #[tokio::test]
    async fn new_seeds_memory_with_each_tool_in_catalog() {
        let td = tempfile::tempdir().unwrap();
        let rt = SkillbankRuntime::new(td.path().join("h.db")).await.unwrap();
        // Every tool in the catalog should appear as one memory row
        // with kind=tool. Catalog size tracks crate::skillbank::tools::catalog()
        // (30 after T50's tool catalog expansion to v0.6.0 V1).
        let hits = rt
            .memory_for_test()
            .search("kind:tool", 100)
            .await
            .unwrap_or_default();
        // FTS5 column filters require the column to be indexed, but our
        // FTS5 table indexes `kind` so `kind:tool` is a valid query.
        assert_eq!(hits.len(), 30, "expected 30 tool rows, got {}", hits.len());
        // Each tool's name should be findable as a phrase.
        let calc = rt
            .memory_for_test()
            .search(
                &crate::skillbank::memory::escape_fts5_query("skill_calculator"),
                5,
            )
            .await
            .unwrap();
        assert!(!calc.is_empty(), "calculator tool not stored");
        assert!(calc[0].text.contains("skill_calculator"));
    }

    #[tokio::test]
    async fn new_seeds_sample_skill_into_memory() {
        let td = tempfile::tempdir().unwrap();
        let rt = SkillbankRuntime::new(td.path().join("h.db")).await.unwrap();
        let hits = rt.memory_for_test().search("kind:skill", 10).await.unwrap();
        assert_eq!(hits.len(), 1, "expected exactly one seeded skill row");
        // The sample skill's name is rebase-onto-main (see
        // docs/skills/sample-skill.md).
        assert!(hits[0].text.contains("rebase-onto-main"));
    }

    #[tokio::test]
    async fn reopening_same_db_does_not_duplicate_skill_rows() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("h.db");
        let _ = SkillbankRuntime::new(p.clone()).await.unwrap();
        let r2 = SkillbankRuntime::new(p).await.unwrap();
        let hits = r2.memory_for_test().search("kind:skill", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn judge_and_record_writes_verdict_into_checkpoint_and_memory() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(anthropic_text_response(
                    r#"{"score": 7, "rationale": "made progress"}"#,
                )),
            )
            .mount(&server)
            .await;

        let td = tempfile::tempdir().unwrap();
        let mut rt = SkillbankRuntime::new(td.path().join("h.db")).await.unwrap();
        rt.set_curator(Curator {
            api_base: server.uri(),
            api_key: "k".into(),
            model: DEFAULT_JUDGE_MODEL.into(),
            timeout_secs: 5,
        });

        let mut c = EvolveCheckpoint::new("fix the lint", "check", "test-node");
        rt.judge_and_record(&mut c).await.expect("judge ok");

        // 1. The checkpoint must carry the verdict (Curator does this).
        let v = c.judge_score.as_ref().expect("verdict on checkpoint");
        assert_eq!(v.score, 7);
        assert_eq!(v.rationale, "made progress");

        // 2. Memory must have one judge_verdict row tagged with the session_id.
        let hits = rt
            .memory_for_test()
            .search(&crate::skillbank::memory::escape_fts5_query(&c.session_id), 10)
            .await
            .unwrap();
        let verdicts: Vec<_> = hits.iter().filter(|r| r.kind == "judge_verdict").collect();
        assert_eq!(
            verdicts.len(),
            1,
            "expected 1 verdict row, got {:?}",
            verdicts
        );
        assert!(verdicts[0].text.contains("score=7"));
        assert!(verdicts[0].text.contains("made progress"));
        assert!(verdicts[0].tags.contains(&c.session_id));
    }

    #[tokio::test]
    async fn judge_and_record_without_curator_returns_error() {
        let td = tempfile::tempdir().unwrap();
        let rt = SkillbankRuntime::new(td.path().join("h.db")).await.unwrap();
        let mut c = EvolveCheckpoint::new("g", "check", "n");
        let err = rt.judge_and_record(&mut c).await.unwrap_err();
        assert!(
            format!("{:#}", err).to_lowercase().contains("curator"),
            "error should mention missing curator: {:#}",
            err
        );
    }

    #[tokio::test]
    async fn judge_and_record_writes_no_memory_row_when_curator_fails() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let td = tempfile::tempdir().unwrap();
        let mut rt = SkillbankRuntime::new(td.path().join("h.db")).await.unwrap();
        rt.set_curator(Curator {
            api_base: server.uri(),
            api_key: "k".into(),
            model: DEFAULT_JUDGE_MODEL.into(),
            timeout_secs: 5,
        });

        let mut c = EvolveCheckpoint::new("g", "check", "n");
        let err = rt.judge_and_record(&mut c).await.unwrap_err();
        assert!(
            format!("{:#}", err).to_lowercase().contains("500")
                || format!("{:#}", err).to_lowercase().contains("status")
        );
        // Checkpoint must be unmodified.
        assert!(c.judge_score.is_none());
        // Memory must have no judge_verdict rows.
        let leaked = rt.recall_verdicts_for(&c.session_id).await.unwrap();
        assert!(
            leaked.is_empty(),
            "no verdict row should have been written: {:?}",
            leaked
        );
    }

    #[tokio::test]
    async fn end_to_end_session_loop_persists_then_recalls() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(anthropic_text_response(
                    r#"{"score": 8, "rationale": "clean fix"}"#,
                )),
            )
            .mount(&server)
            .await;

        let td = tempfile::tempdir().unwrap();
        let db = td.path().join("loop.db");

        // === Session 1 ===
        let mut rt1 = SkillbankRuntime::new(db.clone()).await.unwrap();
        rt1.set_curator(Curator {
            api_base: server.uri(),
            api_key: "k".into(),
            model: DEFAULT_JUDGE_MODEL.into(),
            timeout_secs: 5,
        });
        let mut c = EvolveCheckpoint::new("recurring task", "check", "node-1");
        let sid = c.session_id.clone();
        rt1.judge_and_record(&mut c).await.unwrap();
        drop(rt1);

        // === Session 2 — fresh process boots, opens same DB ===
        let rt2 = SkillbankRuntime::new(db).await.unwrap();
        // Tools + skill still single-copy (Task 4/5 idempotency holds).
        let tools = rt2
            .memory_for_test()
            .search("kind:tool", 100)
            .await
            .unwrap();
        assert_eq!(tools.len(), 30);
        let skills = rt2
            .memory_for_test()
            .search("kind:skill", 100)
            .await
            .unwrap();
        assert_eq!(skills.len(), 1);
        // Prior verdict is recallable across the process boundary.
        let prior = rt2.recall_verdicts_for(&sid).await.unwrap();
        assert_eq!(prior.len(), 1, "verdict from session 1 must persist");
        assert!(prior[0].text.contains("score=8"));
    }

    #[tokio::test]
    async fn recall_verdicts_for_returns_prior_verdict_rows_by_session_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(anthropic_text_response(
                    r#"{"score": 5, "rationale": "ok"}"#,
                )),
            )
            .mount(&server)
            .await;

        let td = tempfile::tempdir().unwrap();
        let mut rt = SkillbankRuntime::new(td.path().join("h.db")).await.unwrap();
        rt.set_curator(Curator {
            api_base: server.uri(),
            api_key: "k".into(),
            model: DEFAULT_JUDGE_MODEL.into(),
            timeout_secs: 5,
        });

        // Produce + judge two distinct checkpoints.
        let mut c1 = EvolveCheckpoint::new("goal A", "check", "node-A");
        rt.judge_and_record(&mut c1).await.unwrap();
        let mut c2 = EvolveCheckpoint::new("goal B", "check", "node-B");
        rt.judge_and_record(&mut c2).await.unwrap();

        // Recall for c1.session_id should return exactly one verdict, mentioning score=5.
        let hits = rt.recall_verdicts_for(&c1.session_id).await.unwrap();
        assert_eq!(hits.len(), 1, "expected 1 verdict for c1, got {:?}", hits);
        assert!(hits[0].text.contains("score=5"));

        // Recall for an unrelated SHA returns empty.
        let none = rt.recall_verdicts_for("nonexistent-sha").await.unwrap();
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn recall_verdicts_for_empty_query_is_empty_not_error() {
        let td = tempfile::tempdir().unwrap();
        let rt = SkillbankRuntime::new(td.path().join("h.db")).await.unwrap();
        let hits = rt.recall_verdicts_for("").await.unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn find_tool_by_intent_returns_matching_tools_ranked() {
        let td = tempfile::tempdir().unwrap();
        let rt = SkillbankRuntime::new(td.path().join("h.db")).await.unwrap();
        // "calculate" / "arithmetic" should rank the calculator tool first.
        let hits = rt
            .find_tool_by_intent("arithmetic expression")
            .await
            .unwrap();
        assert!(!hits.is_empty(), "expected at least one hit");
        assert_eq!(
            hits[0], "skill_calculator",
            "calculator should rank first; got {:?}",
            hits
        );
    }

    #[tokio::test]
    async fn find_tool_by_intent_empty_query_returns_empty() {
        let td = tempfile::tempdir().unwrap();
        let rt = SkillbankRuntime::new(td.path().join("h.db")).await.unwrap();
        // Empty/whitespace input should not panic and should yield no hits
        // rather than every row.
        let hits = rt.find_tool_by_intent("   ").await.unwrap();
        assert!(
            hits.is_empty(),
            "empty query must not match anything: {:?}",
            hits
        );
    }

    // ─── A2/T92: judge_and_auto_register ─────────────────────────────────
    mod auto_register {
        use super::*;
        use crate::skillbank::curator::SkillExtractor;
        use crate::skillbank::skill::{SkillDocument, SkillFrontmatter};
        use std::collections::BTreeMap;

        struct StubExtractor(Option<SkillDocument>);
        impl SkillExtractor for StubExtractor {
            fn extract_skill(
                &self,
                _c: &EvolveCheckpoint,
                _o: &str,
            ) -> Result<Option<SkillDocument>, String> {
                Ok(self.0.clone())
            }
        }

        fn fake_skill(name: &str) -> SkillDocument {
            SkillDocument {
                frontmatter: SkillFrontmatter {
                    name: name.into(),
                    version: "0.1.0".into(),
                    description: "extracted in test".into(),
                    triggers: vec!["t".into()],
                    tools: vec![],
                    inputs: BTreeMap::new(),
                    outputs: vec![],
                    tags: vec!["a2".into()],
                    created_at: None,
                    author: None,
                },
                body: "step 1\n".into(),
            }
        }

        async fn make_runtime_with_score(
            score: i64,
        ) -> (SkillbankRuntime, wiremock::MockServer, tempfile::TempDir) {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/messages"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(anthropic_text_response(&format!(
                        r#"{{"score": {}, "rationale": "test"}}"#,
                        score
                    ))),
                )
                .mount(&server)
                .await;

            let td = tempfile::tempdir().unwrap();
            let mut rt = SkillbankRuntime::new(td.path().join("h.db")).await.unwrap();
            rt.set_curator(Curator {
                api_base: server.uri(),
                api_key: "k".into(),
                model: DEFAULT_JUDGE_MODEL.into(),
                timeout_secs: 5,
            });
            (rt, server, td)
        }

        #[tokio::test]
        async fn high_score_with_extracted_skill_writes_new_memory_row() {
            let (mut rt, _server, _td) = make_runtime_with_score(8).await;
            rt.set_skill_extractor(Box::new(StubExtractor(Some(fake_skill("learned-1")))));

            // Baseline: exactly 1 skill row (the seeded sample).
            let before = rt
                .memory_for_test()
                .search("kind:skill", 100)
                .await
                .unwrap();
            assert_eq!(before.len(), 1);

            let mut c = EvolveCheckpoint::new("g", "check", "n");
            let (verdict, registered) = rt
                .judge_and_auto_register(&mut c, "agent output")
                .await
                .expect("ok");
            assert_eq!(verdict.score, 8);
            assert_eq!(registered.as_deref(), Some("learned-1"));

            // Row count must grow by exactly 1.
            let after = rt
                .memory_for_test()
                .search("kind:skill", 100)
                .await
                .unwrap();
            assert_eq!(
                after.len(),
                2,
                "expected 1 new skill row, got {} → {}",
                after.len() - before.len(),
                after.len()
            );
        }

        #[tokio::test]
        async fn idempotent_when_skill_already_exists() {
            let (mut rt, _server, _td) = make_runtime_with_score(9).await;
            rt.set_skill_extractor(Box::new(StubExtractor(Some(fake_skill("dup-skill")))));

            // First call: writes one new row.
            let mut c1 = EvolveCheckpoint::new("g1", "check", "n");
            let (_v, r1) = rt
                .judge_and_auto_register(&mut c1, "out 1")
                .await
                .expect("ok");
            assert_eq!(r1.as_deref(), Some("dup-skill"));
            let after_first = rt
                .memory_for_test()
                .search("kind:skill", 100)
                .await
                .unwrap();
            assert_eq!(after_first.len(), 2, "sample + new = 2");

            // Second call with the SAME extracted skill: must NOT add another row.
            let mut c2 = EvolveCheckpoint::new("g2", "check", "n");
            let (_v, r2) = rt
                .judge_and_auto_register(&mut c2, "out 2")
                .await
                .expect("ok");
            assert_eq!(r2, None, "duplicate skill must be reported as not-written");
            let after_second = rt
                .memory_for_test()
                .search("kind:skill", 100)
                .await
                .unwrap();
            assert_eq!(
                after_second.len(),
                2,
                "row count must NOT grow on duplicate"
            );
        }

        #[tokio::test]
        async fn low_score_with_extractor_emitting_some_still_writes_row_a8() {
            // A8: the score gate has moved from A2 → A1. A2 now invokes the
            // extractor on every verdict and trusts whatever the extractor
            // returns. A stub that hands back Some(_) on a low score will
            // therefore write a row — the "low score short-circuits" guard
            // is no longer A2's responsibility (A1's failure-side
            // classifier is). This test pins that contract.
            let (mut rt, _server, _td) = make_runtime_with_score(2).await;
            rt.set_skill_extractor(Box::new(StubExtractor(Some(fake_skill("low-score-row")))));

            let before = rt
                .memory_for_test()
                .search("kind:skill", 100)
                .await
                .unwrap();
            let mut c = EvolveCheckpoint::new("g", "check", "n");
            let (verdict, registered) = rt.judge_and_auto_register(&mut c, "x").await.expect("ok");
            assert_eq!(verdict.score, 2);
            assert_eq!(
                registered.as_deref(),
                Some("low-score-row"),
                "A8: A2 invokes extractor unconditionally; stub returning Some surfaces"
            );

            let after = rt
                .memory_for_test()
                .search("kind:skill", 100)
                .await
                .unwrap();
            assert_eq!(
                after.len(),
                before.len() + 1,
                "skill row count grows when stub emits — A1 (not A2) is now the polarity gate"
            );
        }

        #[tokio::test]
        async fn low_score_with_extractor_returning_none_writes_no_row() {
            // Companion to the test above: when the stub mirrors A1's actual
            // contract (returns None for a low-confidence failure case),
            // A2 still writes nothing. This pins the "extractor None ⇒ no
            // row" path independently of the score-gate question.
            let (mut rt, _server, _td) = make_runtime_with_score(2).await;
            rt.set_skill_extractor(Box::new(StubExtractor(None)));

            let before = rt
                .memory_for_test()
                .search("kind:skill", 100)
                .await
                .unwrap();
            let mut c = EvolveCheckpoint::new("g", "check", "n");
            let (verdict, registered) = rt.judge_and_auto_register(&mut c, "x").await.expect("ok");
            assert_eq!(verdict.score, 2);
            assert!(registered.is_none(), "extractor None ⇒ no row");

            let after = rt
                .memory_for_test()
                .search("kind:skill", 100)
                .await
                .unwrap();
            assert_eq!(
                after.len(),
                before.len(),
                "no skill row added when extractor abstains"
            );
        }

        #[tokio::test]
        async fn default_extractor_is_noop_so_no_skill_registered_even_on_high_score() {
            // Don't call set_skill_extractor — runtime should default to NoopSkillExtractor.
            let (rt, _server, _td) = make_runtime_with_score(10).await;

            let before = rt
                .memory_for_test()
                .search("kind:skill", 100)
                .await
                .unwrap();
            let mut c = EvolveCheckpoint::new("g", "check", "n");
            let (verdict, registered) = rt.judge_and_auto_register(&mut c, "x").await.expect("ok");
            assert_eq!(verdict.score, 10);
            assert!(registered.is_none(), "noop extractor ⇒ no skill registered");

            let after = rt
                .memory_for_test()
                .search("kind:skill", 100)
                .await
                .unwrap();
            assert_eq!(after.len(), before.len(), "no rows added by noop path");

            // But the verdict was still recorded.
            let verdicts = rt.recall_verdicts_for(&c.session_id).await.unwrap();
            assert_eq!(verdicts.len(), 1, "verdict must always be persisted");
        }

        #[tokio::test]
        async fn no_curator_installed_returns_error() {
            let td = tempfile::tempdir().unwrap();
            let rt = SkillbankRuntime::new(td.path().join("h.db")).await.unwrap();
            let mut c = EvolveCheckpoint::new("g", "check", "n");
            let err = rt.judge_and_auto_register(&mut c, "x").await.unwrap_err();
            assert!(
                format!("{:#}", err).to_lowercase().contains("curator"),
                "must mention missing curator: {:#}",
                err
            );
        }
    }

    // ---- Task 6 (A4/T94): recall_context_for ----------------------------

    /// Seed N synthetic memory rows whose text shares the token "needle" so
    /// FTS5 will rank them all when we recall on that token. Row index is
    /// embedded so the tests can verify ordering / cap behaviour.
    async fn seed_needle_rows(rt: &SkillbankRuntime, n: usize) {
        for i in 0..n {
            let text = format!("needle row {} body content", i);
            rt.memory_for_test()
                .insert(crate::skillbank::memory::NewMemory {
                    kind: "lesson",
                    source: "test_seed",
                    text: &text,
                    tags: "needle",
                })
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn recall_context_for_caps_at_k() {
        let td = tempfile::tempdir().unwrap();
        let rt = SkillbankRuntime::new(td.path().join("h.db")).await.unwrap();
        seed_needle_rows(&rt, 50).await;
        let hits = rt.recall_context_for("needle", 5).await.unwrap();
        assert!(
            hits.len() <= 5,
            "expected ≤5 rows, got {} ({:?})",
            hits.len(),
            hits
        );
        assert!(!hits.is_empty(), "expected at least one matching row");
        for h in &hits {
            assert!(h.contains("needle"), "row missing keyword: {h}");
        }
    }

    #[tokio::test]
    async fn recall_context_for_truncates_long_rows() {
        let td = tempfile::tempdir().unwrap();
        let rt = SkillbankRuntime::new(td.path().join("h.db")).await.unwrap();
        let long = format!("needle {}", "x".repeat(1000));
        rt.memory_for_test()
            .insert(crate::skillbank::memory::NewMemory {
                kind: "lesson",
                source: "t",
                text: &long,
                tags: "",
            })
            .await
            .unwrap();
        let hits = rt.recall_context_for("needle", 5).await.unwrap();
        assert_eq!(hits.len(), 1);
        // Cap (200) + ellipsis (3 bytes) — keep us well under the original 1006.
        assert!(
            hits[0].chars().count() <= MEMORY_ROW_CHAR_CAP + 1,
            "row not truncated: {} chars",
            hits[0].chars().count()
        );
    }

    #[tokio::test]
    async fn recall_context_for_empty_prompt_is_empty() {
        let td = tempfile::tempdir().unwrap();
        let rt = SkillbankRuntime::new(td.path().join("h.db")).await.unwrap();
        seed_needle_rows(&rt, 3).await;
        assert!(rt.recall_context_for("", 5).await.unwrap().is_empty());
        assert!(rt.recall_context_for("   ", 5).await.unwrap().is_empty());
        assert!(rt.recall_context_for("needle", 0).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn recall_context_for_unknown_prompt_is_empty_not_error() {
        let td = tempfile::tempdir().unwrap();
        let rt = SkillbankRuntime::new(td.path().join("h.db")).await.unwrap();
        seed_needle_rows(&rt, 3).await;
        let hits = rt
            .recall_context_for("totally-unrelated-zzqxyplover", 5)
            .await
            .unwrap();
        assert!(hits.is_empty(), "unknown query should not match: {hits:?}");
    }

    #[tokio::test]
    async fn reopening_same_db_does_not_duplicate_tool_rows() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("h.db");
        let _r1 = SkillbankRuntime::new(p.clone()).await.unwrap();
        drop(_r1);
        let r2 = SkillbankRuntime::new(p).await.unwrap();
        // Still 30 tool rows after the second open, not 60 (T50 expansion).
        let hits = r2.memory_for_test().search("kind:tool", 100).await.unwrap();
        assert_eq!(
            hits.len(),
            30,
            "tools were re-seeded on reopen — not idempotent"
        );
    }
}
