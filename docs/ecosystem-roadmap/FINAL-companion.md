# FINAL Development Plan — spectyn-companion

> Finalized from the codex draft (`PLAN-companion.md`) + agy second-opinion
> (`REVIEW-companion.md`), reconciled against the actual code (`aggregator.py`,
> `reporter.py`, `README.md`). Status: **alpha · Tier 1 (gathering baseline)**.

## What it is / current state
`spectyn-companion` is the local-only **keystone** of spectyn-mesh — the only project that
consumes the other six satellites' outputs (events, ai-feed digests, flow jobseek logs,
training runs, secure-connector health/anomaly, enterprise) and renders a **structurally
shame-free daily/weekly insight report**. Maturity is alpha / Tier 1: CLI, aggregator (the
decrypting `spectyn recall --json` read path + raw-events fallback + satellite log/heartbeat
scan), five insight modules (`llm_usage`, `attention_switches`, `learning_roi`,
`jobseek_followup`, `health_productivity_correlation`), a reporter whose `shame_free_check()`
hard-gates every emitted byte, and a standalone `anomaly_detector` all exist — but the data
plane is thin. **Verified gaps:** `aggregate_range()` (aggregator.py:179) is a bare
dict-comprehension over single days; `reporter._run_insights()` (reporter.py:58) hard-codes
`health_data={}, commits=[]`; on-disk `events/<id>/*.json` are age-encrypted so `recall` is the
only real read path; weekly reports are event-count rollups only. Nothing here is genuinely
useful until ~30–60 days of mesh events accumulate — **so the plan's job is to make the data
plane real and trustworthy, not to bolt on push delivery that leaks private data off-device.**

## Prioritized backlog (P1/P2/P3)

- **P1 — Mock/stub data harness FIRST.** A deterministic fixture generator for multi-day
  health + commit/output + event streams so the window and correlation work is validatable
  *offline today*, without waiting 30+ days for `spectyn-secure-connector` (Garmin/iOS) data.
  Bake a shared `MIN_SAMPLES` (≈14) guardrail constant here for everything else to consult. (agy)
- **P1 — `AggregateWindow` + normalized schemas + SQLite cache.** Replace the dict-comprehension
  `aggregate_range()` with a typed cross-day window; normalize recall output, raw-events
  fallback, ai-feed/flow logs, and heartbeat state once. Cache decrypted per-day recall results
  in a local SQLite index keyed `(day, source)` so reports don't re-spawn `spectyn recall` every
  run — the real bottleneck is the subprocess, not JSON parsing (agy's perf point, re-scoped).
- **P1 — Wire real health + output into `analyze_health_vs_output`.** Extend `DailyAggregate`
  with `health_data` and `commits`/output; delete the `={}` / `=[]` hard-codes at reporter.py:58;
  parse the secure-connector export (sleep, HRV, resting HR, activity, source). **Gate the math:**
  directional daily summary below `MIN_SAMPLES`, statistical correlation only above it. (draft + agy)
- **P2 — Weekly cross-satellite pattern summaries.** Lift `render_weekly_report()` from per-day
  event counts to pattern rollups across LLM usage, attention, learning ROI, and jobseek
  follow-up, reading from the `AggregateWindow`/SQLite plane.
- **P2 — `anomaly_detector.detect()` behind a density gate.** Apply rolling-MAD detection to
  health / LLM-cost / attention series, but emit alerts only above `MIN_SAMPLES` to prevent the
  short-window false positives that would violate the shame-free invariant. (draft + agy)
- **P3 — Local-only / opt-in notification relay** (reframed from draft's push delivery). Default
  sink stays the local `<date>-report.md`; any off-device delivery is strictly opt-in,
  consent-gated, payload-minimized (summary, not raw health/jobseek/LLM-prompt data),
  per-message `shame_free_check()`'d, and routed through the local relay — **no raw SMTP/Telegram
  of sensitive data**, per the P4 encryption-first / local-only pillar. (agy security correction)
- **P3 — Nightly subjective check-in** (mood/gut/sleep/notes) merged into daily/weekly, plus
  monthly/quarterly trend + intervention tracking once 30–60 days of baseline exist.

> **Explicitly NOT doing (cut from draft):** swapping the `_invoke_coach()` subprocess
> (reporter.py:71) for a broker/meter IPC path. The subprocess boundary is the *correct*
> decoupling — direct Rust-core IPC adds cross-language coupling and Windows fragility for no
> user-visible gain. Keep the clean CLI seam. (agy)

## Top-3 task breakdowns

### 1. P1 — Mock/stub data harness + `MIN_SAMPLES` guardrail
- Extend `_fake_mesh/` into `gen_fixture(days=N, seed=...)` emitting synthetic health samples,
  commit/output samples, and multi-day event streams with realistic shapes.
- Add one shared `MIN_SAMPLES` constant; have health, anomaly, and weekly-pattern code import it
  rather than re-deciding thresholds locally.
- Expose a `--mesh-root <fixture-dir>` / env hook so CLI + reporter run end-to-end offline.
- Tests: N-day fixture renders a clean report; below `MIN_SAMPLES` the correlation/anomaly
  modules stay baseline (no false signal); shame-free lint still passes.

### 2. P1 — `AggregateWindow` + normalized schemas + SQLite cache
- Define typed records (normalized event, satellite daily log, health sample, output sample) and
  an `AggregateWindow`; refactor `aggregate_range()` to build it once with stable day ordering.
- Add a SQLite cache keyed `(day, source)`: on miss call `_events_via_recall`, on hit skip the
  subprocess; keep the raw `events/` scan only as the test/no-spectyn fallback.
- Keep `DailyAggregate`'s dict shape via adapters so the five insight modules migrate
  incrementally (no big-bang break of `reporter._run_insights()`).
- Tests: missing satellite dirs, encrypted-recall fallback, malformed JSON, 7-day ordering,
  cache hit vs cold-build equivalence.

### 3. P1 — Real health + output inputs (gated correlation)
- Extend `DailyAggregate` with `health_data` + `commits`; remove the `={}` / `=[]` hard-codes
  and source from the window/harness — no implicit repo scraping.
- Specify + parse the secure-connector daily export (sleep h, HRV, resting HR, activity, source).
- Upgrade `analyze_health_vs_output()` to a two-mode gate: directional summary below
  `MIN_SAMPLES`, Pearson/Spearman only above; never assert causation in shame-free copy.
- Tests with fixture data covering below-threshold, above-threshold, and missing-source cases.

## Changes from draft
- **Added (new P1) the mock/stub harness as the first move** — agy correctly flagged the
  health-correlation work is otherwise blocked on 30+ days of secure-connector data; `_fake_mesh/`
  already exists as precedent. Highest-leverage unblock.
- **Reframed Telegram/email push from P1 down to a P3 local-only, opt-in, consent-gated relay** —
  agy's valid privacy / encryption-first objection (confirmed by the README's local-only pillar);
  the draft would have shipped raw health/jobseek/LLM-prompt data over SMTP/Telegram bots.
- **Folded agy's perf point into P1 #2 but corrected it**: the bottleneck is the per-day
  `spectyn recall` subprocess (on-disk events are age-encrypted ciphertext), so the win is a
  decrypted-result SQLite cache — not "stop parsing raw JSON on disk" as agy framed it.
- **Added explicit `MIN_SAMPLES` density guardrails** to the correlation + anomaly items, per
  agy's statistical-insufficiency / alert-fatigue concern — this directly protects the shame-free
  invariant.
- **Cut** the draft's `_invoke_coach` → broker/meter IPC swap; agreed with agy to keep the
  subprocess decoupling (no eager Rust-core IPC coupling on Windows).
- **agy review verdict: usable.** Independent and codebase-specific (correct file/line refs);
  four of five points adopted (fixtures-first, privacy reframe, SQLite cache, density gates,
  no-IPC). Each claim was verified against the real source and the one mislocated perf diagnosis
  was corrected.

---
**Highest-priority next move:** build the `_fake_mesh` N-day fixture generator + the shared
`MIN_SAMPLES` guardrail — it unblocks every other P1 offline today instead of waiting 30–60 days
for real baseline data.
