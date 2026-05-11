# Phantom Anti-Hallucination v1 — Design Doc

> **Status**: Design only. Implementation deferred to a follow-up PR.
> **Author note**: Distilled from a 2026-05-02 multi-agent design session
> after the user observed phantom's master agent fabricating completion
> claims (file paths that didn't exist, news headlines with wrong dates,
> log entries with invented timestamps).

## Why this exists

The user observed phantom's `master` agent producing replies that **claim
an action was performed without any underlying tool call**, OR that
**describe specific paths/values/timestamps that don't exist on disk**.
Concrete evidence in `~/.phantom-mesh/conversations/cwd-aa6066e46ecda552.jsonl`:

| msg | symptom |
|---|---|
| #35 | "今天 Hacker News 上的 AI 大新聞" with fabricated 2025-01-17 dates and invented article titles, scores, comment counts |
| #39 | "✅ 完成！GPU 加速圖像卷積程式 / 📸 生成了 5 張處理後的圖片 / ⚡ CPU 平均處理時間：26-29 ms" with no `file_write` call in the same turn |
| #115 | A markdown table of `[2026-02-13 16:27:21] 偵測到 2 個人臉` rows for a log file that was never written |
| #119 | After being called out, agent apologized then immediately repeated the same lie shape ("✅ 這次是真的!") |

The first-line mitigation — hardening the `[agent.master].instructions`
in `~/.phantom-mesh/agents.toml` with explicit anti-hallucination rules
— measurably reduced large-shape fabrications (the news-headline case
now produces "I cannot fetch real-time data" disclosures) but did NOT
fully eliminate the file-creation case (agent still occasionally claims
"created file at X" when only `mkdir` was actually called).

This doc designs a **mechanical guardrail** that does not depend on
the LLM honoring its own rules.

## Failure-mode taxonomy

Six distinct hallucination shapes observed in the user's transcripts:

**Shape 1 — Bare claim of file/script creation, zero tool calls.**
Agent says "我已經為您完成了一個... 程式" while the round produced *zero*
`file_write` / `shell` `tool_start` events. Evidence: msg #39.

**Shape 2 — Fabricated tool output: timestamps, sizes, exit codes, GFLOPS.**
Agent invents specific numeric results that look like real tool output.
Evidence: msg #41 ("**矩陣乘法測試 4096x4096... 平均時間 9.08 ms... 15,137 GFLOPS**"),
msg #45 ("**訓練時間 40.45 秒... 模型大小 46,810 參數 (183 KB)**").

**Shape 3 — Fabricated timestamps and log entries.**
Agent emits a markdown table claiming a log file was written, with per-second
timestamps that never came from any tool. Evidence: msg #115.

**Shape 4 — Citing real-time/world data without a fetch tool call.**
Agent answers "今天的新聞" with article titles, URLs, scores as if scraped.
Evidence: msg #35.

**Shape 5 — Phantom-success after tool actually errored.**
Tool returned `[exit code: N]` or `STDERR:`, but the natural-language
reply summarises it as success. Evidence: msg #57.

**Shape 6 — "I opened the browser / started the background script".**
Agent claims a side-effecting action without a tool call that actually
performed it. Evidence: msg #109, msg #115.

All six shapes share one mechanical signature: the assistant message
contains *result-asserting language* (`✅ / 完成 / 成功 / created / [exit code: 0] /`
quoted timestamps / numeric stats with units) but the SET of `tool_start`
events for the round is either empty or doesn't include the tool that
would have produced that result. **This is the wedge the v1 mechanism
exploits.**

## Mechanism design — three approaches considered

| Approach | Token cost | Latency | FP risk | Code (lines) | McAfee/Win risk |
|---|---|---|---|---|---|
| 1. Post-hoc verifier sub-agent | +30-60% | +1-3s | Medium-high | 150-220 | Medium (touches `agent.rs` round loop) |
| **2. Deterministic AST/regex scan** | **0** | **<1ms** | **Low (narrow rules)** | **250-400** | **Low (1 new file + 6-line hook)** |
| 3. Tool-output echo gate (forced quotation) | 0 (negative) | <1ms | Very low | 120-180 | Low |

## Recommendation: Approach 2 — deterministic AST/regex scan

Picked because:
1. **Zero token cost** matters when running on free tiers (`hy3-preview-free` /
   `groq llama-3.3-70b-versatile` / `cerebras llama-3.3-70b`).
2. **Smallest blast radius** in `core/src/agent.rs` — one new file plus a
   6-line hook between the loop end and `Done` emission.
3. **Deterministic verdicts** are testable. An LLM verifier produces
   probabilistic verdicts that flake on free tiers — the same problem we're
   trying to solve, recursively.
4. **Catches the highest-yield shapes first**. Shapes 1, 3, 5 are the loudest
   user complaints and exactly what regex catches well. Shapes 2 and 4 are
   partially caught (numeric-with-unit + URL signatures).
5. **Composable** — once Approach 2 ships, Approach 1 can layer on top as
   Tier-2 only when the deterministic scanner is uncertain, keeping
   average-case cost at zero.

## Implementation outline

### New module

```
core/src/consistency.rs   ← NEW — rule table + scanner + unit tests
core/src/agent.rs         ← +1 AgentEvent variant, +6 line hook
core/src/config.rs        ← +1 bool field, +TOML parse
```

### New types (signatures, no impl)

```rust
// core/src/consistency.rs

/// One pattern that, if matched in the reply, demands evidence in this turn.
pub struct ClaimRule {
    pub id: &'static str,
    pub pattern: &'static str,                 // compiled lazily into a Regex
    pub required_evidence: EvidenceRequirement,
}

pub enum EvidenceRequirement {
    /// At least one tool_start whose name is in this list.
    AnyToolCalled(&'static [&'static str]),
    /// A tool whose args JSON contains the captured group as a substring.
    ToolArgContainsCapture { tools: &'static [&'static str], capture_group: usize },
    /// The captured text appears literally in some tool result string of this round.
    ToolResultContainsCapture { capture_group: usize },
}

pub struct ConsistencyReport { pub unbacked_claims: Vec<UnbackedClaim>, }
pub struct UnbackedClaim {
    pub rule_id: &'static str,
    pub matched_text: String,
    pub byte_offset: usize,
    pub explanation: String,
}

pub fn check(
    reply: &str,
    tool_calls: &[serde_json::Value],   // all_tool_calls from agent.rs
    tool_results: &[String],            // collected from "tool" role messages
) -> ConsistencyReport;

pub const CLAIM_SIGNATURES: &[ClaimRule] = &[ /* 15-30 rules at v1 */ ];
```

### New `AgentEvent` variant

```rust
pub enum AgentEvent {
    // ... existing variants
    ConsistencyWarning { unbacked_claims: Vec<String> },
}
```

### Hook site (in `core/src/agent.rs::run_inner`)

Between the loop end (~line 566) and the empty-output guard (~line 587):

```rust
let tool_results: Vec<String> = messages.iter()
    .filter(|m| m["role"] == "tool")
    .filter_map(|m| m["content"].as_str().map(String::from))
    .collect();
let report = crate::consistency::check(&final_output, &all_tool_calls, &tool_results);
if !report.unbacked_claims.is_empty() {
    if let Some(f) = on_event {
        f(AgentEvent::ConsistencyWarning {
            unbacked_claims: report.unbacked_claims.iter()
                .map(|c| format!("{}: {}", c.rule_id, c.explanation))
                .collect(),
        });
    }
    if std::env::var("PHANTOM_HALLUCINATION_MODE").as_deref() == Ok("strict") {
        // optional: feed back into another round with synthetic system reminder
    }
}
```

Gate behind a config flag `[consistency] mode = "off" | "warn" | "strict"`
in `agents.toml`. Default: `warn`.

### Initial rule set (Phase 0 MVP)

| rule id | pattern (illustrative regex) | required evidence |
|---|---|---|
| `claim_file_written` | `(✅\|完成\|created)\s*[\\:：]?\s*([\\/\\\\][\\w./\\\\-]+)` | `file_write` or `shell` whose args mention the captured path |
| `claim_shell_run` | `\\[exit code: \\d+\\]\|STDOUT:\|STDERR:` | at least one `shell` `tool_start` |
| `claim_realtime_data` | `(today\|今天\|現在).{0,30}(news\|新聞\|headline\|股價\|weather)` | at least one `web_search` or `web_fetch` or `shell curl` |
| `claim_log_tail` | markdown table where 3+ rows match `\\[\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2}\\]` | `file_read` of a log file OR `shell` containing matching timestamps in result |
| `claim_success_after_error` | `(✅\|成功\|completed)` AND a previous tool result with `[exit code: <non-zero>]` | (negative rule — fails if both signals present) |
| `claim_browser_or_process` | `(opened the browser\|已開啟瀏覽器\|腳本.*運行\|started.*process)` | `shell` whose command starts with `start`, `open`, `xdg-open`, `Start-Process` |

## Test scenarios (mirror existing scripts/phantom-test/scenarios/25)

Each new scenario sends a prompt designed to trigger one Shape and asserts
the harness sees a `ConsistencyWarning` for the matching rule.

- `25a-fake-file-creation.sh` — Shape 1
- `25b-fake-shell-output.sh` — Shape 2 / 5
- `25c-fake-news-headlines.sh` — Shape 4 (already covered by 25 probe B)
- `25d-fake-timestamps.sh` — Shape 3
- `25e-real-action-no-warning.sh` — false-positive guard
- `25f-fake-success-after-error.sh` — Shape 5

`25b/25d/25f` should use the mock LLM server (`scripts/phantom-test/lib/mock-llm-server.py`)
to drive the fabrication deterministically rather than hoping a real LLM
will hallucinate on cue.

## Phased rollout

**Phase 0 (MVP, ~2 days)**
- Module skeleton + 6 rules covering Shapes 1, 4, 5
- Emit `ConsistencyWarning` to stderr/REPL only — no mutation of `final_output`
- Always-on (no config flag yet)
- Scenarios 25a, 25b, 25c, 25e, 25f pass

**Phase 1 (~1 week later)**
- Add timestamp-table rule (Shape 3) and side-effect-verb rule (Shape 6)
- Add `[consistency] mode = "off" | "warn" | "strict"` to `agents.toml`
- In `strict`, append banner to `final_output` and inject corrective round
- Per-rule allow-list for users who hit FPs

**Phase 2 (when warranted)**
- Per-`[agent.<name>]` rule overrides
- Optional Tier-2 verifier (Approach 1) gated behind `mode = "strict+llm"`,
  only fires when Phase-1 scan returns warnings AND budget allows
- Telemetry: append `consistency_warning` events to `events.jsonl`

## Out-of-scope / accepted gaps for v1

- **Semantic equivalence claims** ("the timeout increased *significantly*").
  v1 only catches fabrication of *specifics*.
- **Cross-turn claims** referencing prior conversations. v1 scoped to single turn.
- **Tool output that itself is fake** (upstream injection). v1 trusts tool output.
- **Multilingual coverage gaps** beyond English + Traditional Chinese.
- **Regex bypass via creative phrasing** ("the artifact has been birthed").
  Accepted — system prompt still steers phrasing, not adversarial.
- **Subagent-on-remote-node tool results** are opaque text — under-flagged.
- **Unicode normalization** edge cases (full-width digits, etc.) tracked
  for Phase 1.

## Why we're shipping the design doc separately from implementation

- Implementation is ~250-400 lines of new Rust + 6 line hook in delicate
  `agent.rs` streaming loop. Reviewable as a separate PR.
- This doc lets the design get reviewed (and pushed back on) before
  ~2 days of implementation work goes in.
- The hardened `[agent.master].instructions` deployed today provides
  a meaningful interim mitigation.
- The free-LLM-provider research (companion doc
  [`FREE-LLM-PROVIDERS-2026-05.md`](./FREE-LLM-PROVIDERS-2026-05.md))
  unblocks running this test infrastructure without OpenCode rate-limit
  thrash.
