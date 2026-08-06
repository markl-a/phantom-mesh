# FINAL Development Plan — `spectyn-ai-feed`

## What it is + current state

`spectyn-ai-feed` is a stdlib-only Python alpha in the spectyn-mesh ecosystem.
RSS/Atom sources (8 feeds in `sources/feeds.toml`) flow through
`fetch.py → summarize.py → digest.py / weekly.py / interview_questions.py`
into Markdown logs under `~/.spectyn-mesh/logs/spectyn-ai-feed/`, with
best-effort `spectyn event capture` into FTS5 and a 3-tier summariser
(`spectyn exec` → Gemini REST → stdlib stub). An `eval.py` harness scores
generated questions against a **synthetic** gold set. Daily/weekly CLIs,
question generation, and a thin test suite (`test_fetch`, `test_summarize_stub`,
`test_eval_harness` + fixtures) work end-to-end in stub mode. Maturity: usable
but thin — reliability, prompt correctness, real-recall verification, dedup,
and publishing are immature.

---

## Prioritized features (P1/P2/P3)

- **P1 — Fix prompt double-wrapping (CORRECTNESS BUG).** `weekly.py` and
  `interview_questions.py` hand a *fully-formed* prompt into the summariser, but
  `summarize_spectyn`/`summarize_gemini` unconditionally re-wrap every input with
  the daily-RSS `_build_prompt()`. Weekly/interview prompts are double-wrapped and
  polluted with daily-summary instructions. *(Verified: `summarize.py:69,101` →
  `_build_prompt`; callers `weekly.py:91`, `interview_questions.py:89`.)*
- **P1 — Fetch/summarize reliability hardening.** Bounded retry/backoff in
  `_http_get`, HTML stripping for `summary_excerpt` (arXiv/Reddit), per-feed
  status counts in digest headers, fake-RSS fixtures so tests stop relying on
  the live network.
- **P1 — FTS5 capture adapter (unit layer).** Extract `_try_capture_fts5()` from
  `digest.py` into `spectyn_ai_feed/spectyn.py` with `capture_entry(dry_run=...)`;
  unit-test missing-CLI / failed-CLI / success via monkeypatched
  `shutil.which` + `subprocess.run`. (Real recall is P2 — see below.)
- **P2 — FTS5 recall verification (integration).** A gated smoke test against the
  real `spectyn` binary that captures an `ai-feed` entry and asserts
  `spectyn recall "<query>"` finds it. Distinct from the P1 unit layer; needs the
  binary, so it cannot be a mocked unit test.
- **P2 — Cross-source dedup / topic clustering.** Collapse the same story across
  arXiv/Reddit/HN (URL + title-shingle / token-overlap) **before** `weekly.py`
  ranks. Sequenced ahead of newsletter generation.
- **P2 — Source credibility weighting.** Per-`category` trust + fetch-success
  history from `sources/feeds.toml` to bias weekly ranking and dedup tie-breaks.
- **P3 — Substack/newsletter draft generator.** Turn `weekly-*.md` + questions
  into a human-reviewed draft. Depends on P2 dedup (else redundant drafts).
- **P3 — Eval gold-set calibration + answer/grading loop.** Replace the synthetic
  `eval.py` gold set with a real ~20-question gold set; calibrate generator
  quality before building any answer-and-grade feature.
- **P3 — Expand sources** beyond the 8 feeds (Chinese AI sources, optional
  manual/premium feeds).

---

## Top 3 — task breakdown

### P1.1 — Fix prompt double-wrapping
- Add a raw-passthrough path to `summarize.py`: e.g. `prompt: str | None` (or a
  `raw=True` flag) on `summarize`, `summarize_spectyn`, `summarize_gemini` that
  sends the caller's text verbatim and **skips** `_build_prompt`.
- Update `weekly.py:91` and `interview_questions.py:89` to use the raw path;
  leave `digest.py` (the only correct caller — passes plain text) on the
  wrapping path.
- Add tests that assert the exact bytes handed to `subprocess.run` / the Gemini
  payload contain the weekly/interview prompt and **not** the daily-summary
  preamble.
- Eyeball one real weekly + interview run (stub mode) to confirm clean output.

### P1.2 — Fetch/summarize reliability hardening
- Wrap `_http_get` with bounded retry/backoff handling 429 / timeout / transient
  network errors; cap attempts and total time.
- Strip common HTML from RSS descriptions before `summary_excerpt`
  (arXiv `<p>`/LaTeX noise, Reddit markup).
- Emit per-feed status counts (fetched / failed / empty / summarized / captured)
  in digest + weekly headers.
- Add fake RSS/Atom fixtures under `tests/fixtures/`; keep the live HN fetch as an
  **optional** network smoke test, not a default-run dependency.

### P1.3 — FTS5 capture adapter (unit layer)
- Move `_try_capture_fts5()` into `spectyn_ai_feed/spectyn.py` as
  `capture_entry(entry, *, dry_run=False)`; `digest.py` imports it.
- Preserve a stable text blob of `title / summary / link / source / category /
  date` compatible with `spectyn event capture --kind ai-feed --text`.
- Unit-test the three branches (no CLI / CLI fails / CLI ok) via monkeypatched
  `shutil.which` + `subprocess.run`; assert `dry_run` builds the command without
  executing.
- Leave **real** recall assertions to the P2 integration test (do not claim
  recall is verified from mocked subprocess).

---

## Changes from draft

- **Added the top P1** — the prompt double-wrapping bug. agy caught it and it is
  **verified true** in code (`weekly.py`/`interview_questions.py` re-wrapped by
  `_build_prompt`); the codex draft missed it entirely. This is the real blocker.
- **Split FTS5 into P1 (unit adapter) + P2 (real-recall integration).** Kept the
  cheap adapter extraction at P1 but moved recall *verification* to a binary-gated
  P2 test, per agy — mocked subprocess does not prove recall.
- **Demoted/trimmed SM-2 SRS** out of the P1 set. agy is right that a stand-alone
  Python `due`/`grade`/JSONL reviewer is likely throwaway versus the Rust/Tauri
  DB layer; folded into the P3 answer/grading loop rather than built as its own
  CLI now.
- **Sequenced dedup (P2) before the Substack generator (P3)** to avoid redundant
  drafts (arXiv/Reddit/HN overlap), per agy.
- **Added eval gold-set calibration** as an explicit P3 prerequisite before
  answer/grading — `eval.py` currently scores only a synthetic set.
- **Kept** the codex draft's fetch/summarize hardening (promoted to top-3) and
  source-credibility + expand-sources items.
- **agy's review was usable** — accurate, code-grounded, and it surfaced a real
  correctness bug plus three valid sequencing corrections.
