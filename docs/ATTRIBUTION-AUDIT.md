# Attribution Audit — `hermes/` (skillbank) + `openclaw/` (channels) modules

**Phase 0 of the OSS-debrand effort.** Purpose: before renaming the
`core/src/hermes/` and `core/src/openclaw/` subsystems to neutral functional
names, establish whether the CODE is original (borrows only the *name/concept*
of the external MIT products `NousResearch/hermes-agent` and `openclaw/openclaw`)
or contains genuinely PORTED upstream source. Borrowing a name/concept needs no
attribution; copied MIT source must retain a NOTICE. The operator marked the
origin "uncertain/mixed", so this audit gates the rename. **No source files were
edited.** Verdicts are based on INTERNAL evidence only — the upstream repos are
**not** vendored in this tree (`references/hermes-agent/` does not exist), so any
verdict that would require a literal line-by-line diff to be *certain* is flagged
in the "Needs upstream comparison" subsection rather than asserted as fact.

## Verdict legend
- **original** — clean-room Rust; only the name/concept is borrowed; implements
  standard/well-known functionality independently.
- **concept-port** — architecture/algorithm idea borrowed from upstream and
  re-implemented in Rust; self-documented with an "inspired by / concept ported"
  comment. No copied source text.
- **code-port** — reproduces upstream's specific source structure, identifiers,
  comments, or non-obvious implementation. (Legally sensitive.)

---

## `core/src/hermes/` and friends

| File | Verdict | Evidence (1 line) | Upstream + license | NOTICE to retain |
|------|---------|-------------------|--------------------|------------------|
| `hermes/mod.rs` | original | Module wiring + feature gates only; doc cites internal spec §4/§5. | n/a | n/a |
| `hermes/curator.rs` | concept-port | Header: "Architectural inspiration only from the Hermes-agent-self-evolution repo — NO code vendored". LLM-as-judge over internal `EvolveCheckpoint`. | NousResearch/hermes-agent (concept); upstream self-evolution repo license missing per comment | n/a (no code) |
| `hermes/curator_ensemble.rs` | original | V2 multi-judge ensemble built on internal `curator.rs`; tokio `JoinSet` + median/stddev — standard stats. | n/a | n/a |
| `hermes/skill.rs` | original | YAML-frontmatter + Markdown parser matching internal `docs/hermes-skill-schema.json`; follows the PUBLIC SKILL.md / agentskills.io convention, not Hermes-private source. | SKILL.md format = public convention | n/a |
| `hermes/skill_executor.rs` | original | Executes a `SkillDocument` body (bash/note/prompt steps); internal `ExecutionOpts`/sandbox design, internal track refs (T10/T29). | n/a | n/a |
| `hermes/extract.rs` | original | Distills `JudgeVerdict` → `SkillDocument`; logic keyed to internal PR #144/#146/#157 lineage. | n/a | n/a |
| `hermes/synthesize.rs` | original | Goal → SkillDocument top-down synthesis + sandbox verify loop; internal `LlmProvider`/`SkillExecutor`. | n/a | n/a |
| `hermes/memory.rs` | original | SQLite FTS5 store; schema is internal `migrations/0007_hermes_fts5.sql` (single source of truth). | n/a | n/a |
| `hermes/memory_seal.rs` | original | At-rest age-v1 column sealing reusing internal `EventKey`/`encryption_wire` (P0-8). | n/a | n/a |
| `hermes/agents_seal.rs` | original | API-key sealing mirroring `memory_seal`; internal crypto only (apex P4). | n/a | n/a |
| `hermes/dto.rs` | original | Flat JSON DTOs for internal F400 RPC; shapes derived from internal `MemoryRow`. | n/a | n/a |
| `hermes/integration.rs` | original | Façade wiring internal curator/skill/memory/tools; internal PR refs. | n/a | n/a |
| `hermes/skill_extractor/mod.rs` | original | Pluggable extractor over E003 daily reviews (internal format). | n/a | n/a |
| `hermes/skill_extractor/from_daily_review.rs` | original | Parses internal `spectyn coach review` markdown contract. | n/a | n/a |
| `serve_hermes.rs` | original | Axum RPC routes (`/api/hermes/skills*`) reusing internal broker-token auth + FTS5. | n/a | n/a |
| `interrupt.rs` | concept-port | Header: "Modeled on Hermes Agent's `_interrupt_requested` + `_interrupt_message`" with line refs; explicitly a "Rust port" using tokio `CancellationToken` (different primitive — Hermes uses a 300ms thread poll). Flag-poll-unwind is a generic pattern; no Python copied. | NousResearch/hermes-agent (MIT) — concept | n/a (no code) |

### `core/src/hermes/tools/` (the "ported as Rust" catalog)

`tools/mod.rs:1` self-describes as *"top-10 high-utility tools ported as Rust
idiomatic impls"* and adds *"Concepts ported from the NousResearch/hermes-agent
README (MIT); no verbatim code is copied."* The word "ported" triggered the
audit. Per-tool review confirms the catalog is standard transformations
implemented from scratch in Rust or thin wrappers over standard crates
(`base64`, `sha2`, `uuid`, `urlencoding`, `url`, `regex`, `chrono`,
`serde_yaml`). NONE mirror upstream source code.

| File | Verdict | Evidence |
|------|---------|----------|
| `tools/mod.rs` | original | Trait surface + error enum; doc says concepts from README, no verbatim code. |
| `tools/base64_codec.rs` | original | Wrapper over `base64` crate. |
| `tools/calculator.rs` | concept-port | Header credits hermes-agent example calculator; fresh shunting-yard impl. |
| `tools/color_hex_rgb.rs` | original | Standard hex↔rgb math. |
| `tools/csv_to_json.rs` | original | Hand-written CSV parser w/ quote handling. |
| `tools/datetime.rs` | concept-port | Header credits hermes-agent datetime helper; chrono impl. |
| `tools/diff.rs` | original | Clean-room LCS DP. |
| `tools/grep.rs` | original | `regex`-crate line matcher. |
| `tools/hash.rs` | original | `sha2` wrapper. |
| `tools/html_to_text.rs` | original | In-house tag-strip state machine. |
| `tools/jaro_winkler.rs` | original | Public Jaro-Winkler algorithm, in-house. |
| `tools/jq.rs` | original | In-house JSON path parser. |
| `tools/json_query.rs` | original | Dotted-path JSON lookup. |
| `tools/json_to_csv.rs` | original | Hand-written CSV row render. |
| `tools/random_string.rs` | original | `OsRng` + standard encodings. |
| `tools/regex_extract.rs` | concept-port | Header credits hermes-agent regex tool; `regex` impl. |
| `tools/sort_lines.rs` | original | Standard sort (numeric/unique/desc). |
| `tools/string_metrics.rs` | original | Two-row Levenshtein DP. |
| `tools/template_render.rs` | original | `{{name}}` substitution engine. |
| `tools/text_stats.rs` | concept-port | Header credits hermes-agent text-analytics helper; word/line/char counts. |
| `tools/text_summarize.rs` | original | Head+tail extractive summary. |
| `tools/unit_convert.rs` | original | Hand-written conversion tables. |
| `tools/url_decode.rs` | original | `urlencoding` wrapper. |
| `tools/url_encode.rs` | original | `urlencoding` wrapper. |
| `tools/url_parse.rs` | original | `url` crate wrapper. |
| `tools/uuid_gen.rs` | original | `uuid::new_v4` wrapper. |
| `tools/uuid_v7.rs` | original | `uuid::now_v7` wrapper. |
| `tools/word_count_lines.rs` | original | Hand-written text stats. |
| `tools/word_freq.rs` | original | Hash-based word frequency. |
| `tools/xml_to_json.rs` | original | In-house XML tokenizer. |
| `tools/yaml_to_json.rs` | original | `serde_yaml::Value` → `serde_json::Value` mapping. |

---

## `core/src/openclaw/` (chat-channel remote control)

All adapters are standard Rust HTTP/bot-API integrations (teloxide, reqwest,
serde, subtle, dashmap). No file carries an "inspired by / ported" comment
pointing at upstream openclaw; docs cite internal tracks/specs (B2/B3/B5,
E004, V3). The system NAME is borrowed; the code is clean-room.

| File | Verdict | Evidence |
|------|---------|----------|
| `openclaw/mod.rs` | original | Module wiring; doc cites internal BIG-GOAL §P3 / E004 spec. |
| `openclaw/channel_trait.rs` | original | Async `Channel` trait + error enum; Rust-idiomatic. |
| `openclaw/dispatch.rs` | original | `PersonaDispatcher` wrapper; internal design. |
| `openclaw/dispatcher.rs` | original | Multi-bot routing via `Arc<RwLock<HashMap>>`. |
| `openclaw/inbound_auth.rs` | original | Cross-cutting auth trait (V3 gap 2). |
| `openclaw/media.rs` | original | Channel-agnostic media download w/ size cap. |
| `openclaw/persona.rs` | original | TOML persona schema + lookup, custom. |
| `openclaw/rate_limit.rs` | original | Token bucket + per-channel DashMap (standard algorithm). |
| `openclaw/slack.rs` | original | Slack `chat.postMessage` + HMAC-SHA256 verify via reqwest. |
| `openclaw/telegram.rs` | original | teloxide long-poll round-trip adapter. |
| `openclaw/telegram_agent_dispatcher.rs` | original | Per-chat history via tokio Mutex. |
| `openclaw/webhook_auth.rs` | original | Constant-time token check via `subtle`. |
| `openclaw/whatsapp.rs` | original | Compile-only `Channel` stub returning NotImplemented. |

---

## Summary

- **original:** 56
- **concept-port:** 6 (`hermes/curator.rs`, `hermes/interrupt.rs` [`interrupt.rs`],
  `tools/calculator.rs`, `tools/datetime.rs`, `tools/regex_extract.rs`,
  `tools/text_stats.rs`)
- **code-port:** 0

(62 files total: 45 under `hermes/` incl. 31 tool files, 13 under `openclaw/`,
plus `interrupt.rs` and `serve_hermes.rs`.)

## Needs upstream comparison to be certain

The upstream repos are **not** vendored in this tree, so the following were
classified on internal evidence (doc comments + standard-algorithm reasoning)
and would need a line-by-line diff against the upstream source to be *certain*
they are not `code-port`. None show internal evidence of copying; this list is
about residual uncertainty, not suspicion:

- `hermes/curator.rs` — header asserts "NO code vendored"; logic is internal,
  but the rubric/judge-prompt phrasing could in principle echo upstream. Low risk.
- `hermes/interrupt.rs` (`core/src/interrupt.rs`) — cites specific upstream line
  numbers (`run_agent.py:1178/4416/6460`); the *pattern* is generic and the
  primitive differs (tokio `CancellationToken` vs 300ms thread poll), so almost
  certainly concept-port, but the cited proximity warrants a glance if upstream
  is ever obtained. Low risk.
- `tools/calculator.rs`, `tools/datetime.rs`, `tools/regex_extract.rs`,
  `tools/text_stats.rs` — self-labelled "concept ported"; upstream is Python so
  a Rust file cannot be a verbatim copy, but the API surface could mirror
  upstream's. Very low risk.

If a stricter posture is desired, fetch `NousResearch/hermes-agent` (MIT) and
diff these six. Given (a) the upstream is Python vs this Rust, (b) every flagged
file self-documents concept-only borrowing, and (c) no copied identifiers or
comments were found, the residual `code-port` risk is assessed as negligible.

---

## Rename mode decision

**ZERO `code-port` files were found.**

Therefore: **Phase 1 may rename the `hermes/` and `openclaw/` modules freely.**
No file needs to RETAIN an attribution header through the rename. The six
`concept-port` inspiration comments (curator, interrupt, and the four tools)
carry no legal obligation — they may stay as-is or be reworded/removed in
Phase 3 as part of neutralizing the branding. No NOTICE file or per-file
attribution header is required by this audit.

Caveat: this conclusion rests on internal evidence because the upstream sources
are not present to diff against (see "Needs upstream comparison"). The residual
risk is assessed negligible (Python upstream vs Rust here; concept-only
self-documentation; no copied text found), but if the operator wants certainty
before the public push, run the six-file diff noted above first.
