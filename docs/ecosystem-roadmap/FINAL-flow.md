# FINAL Development Plan — spectyn-flow

## What it is + current state
`spectyn-flow` is the weakest satellite in the spectyn-mesh ecosystem: a ~500-LOC local-first
YAML workflow runner (`spectyn_flow/runner.py`) with a pluggable block registry
(http / regex / filter / llm-summarize / subprocess), a `spectyn exec` LLM wrapper
(`llm_driver.py`), and one genuinely-working e2e flow (`youtube-summarize`: transcript → LLM
summary). It is positioned as an "honest minimal local n8n" (Automation). The repo is dragged
down by ~100k LOC of **vendored, unintegrated** subtrees (`ai_automation_framework/`,
`data_analysis/`) and CI that is only an import smoke test. Per the locked ecosystem framing
(`plans/RESUME-SATELLITE-FRAMING.md:75-79`, `plans/PRODUCT-GRADE-7-3-FOUNDATION.md:60`), the
mandate for this project is **clean up and de-claim**, NOT expand. Target = clean ~500-LOC
runner, real tests, claims = reality, no exposure leaks. Maturity: prototype/demo.

## Prioritized feature/spec list

- **P1 — Delete the dead subtrees.** Remove `ai_automation_framework/` and `data_analysis/`
  (~100k LOC, never wired into the runner). This is an explicit locked directive, not a judgment
  call. Shrinks the repo to its real surface and unblocks honest framing.
- **P1 — Real test contract + CI beyond import-smoke.** pytest for `runner.py`
  (`_resolve`, `_gate_passes`, block registry, dry-run, unknown-block error) and `llm_driver.py`
  (provider-noise cleanup, timeout/error fallback, `SPECTYN_FLOW_STUB_LLM`), all network/LLM-free.
- **P1 — Harden the subprocess boundary** in `SpectynLLM.complete` and `_block_subprocess`:
  reliable cross-OS `spectyn exec` invocation, timeout, captured stderr, graceful degradation to
  stub. (The codex draft's own run shows Windows `CreateProcessAsUserW 1312` sandbox failures —
  this is the real-world fragility, and the genuine differentiator vs. a toy runner.)
- **P2 — De-claim README/DESIGN to reality.** Remove "cluster-aware" and "event-driven" claims
  (unimplemented per locked framing). Triggers stay file-based / cron / manual only. README =
  3 sections + the one working `youtube-summarize` demo.
- **P2 — Validated flow schema + structured run records.** Formal schema for
  `name/version/trigger/pipeline/outbound/when`; per-step `status/duration/result/error/skipped`
  + `run_id`; reject declaration-only flows in execute mode; max-body-size guard.
- **P2 — Add 1-2 honest stdlib-only flows** that exercise existing blocks end-to-end (e.g. a
  second transcript/summarize variant, or an http→regex→llm digest). No new heavy deps.
- **P3 — 60-90s asciinema demo + ROADMAP.md** marking cluster/event-driven/visual-editor as
  explicitly future (the "honest roadmap" itself is a maturity signal for the resume use-case).

### Cut from the draft (over-scoped / unsafe / contradicts locked framing)
- ~~Expose vendored tools as first-class blocks (web_scraper, slack, analysis.cluster, rfm,
  clv)~~ — depends on subtrees that P1 deletes; directly violates the de-claim mandate.
- ~~Cluster-aware dispatch (`spectyn dispatch`, capability labels)~~ — explicit overclaim to
  CUT, not build (`RESUME-SATELLITE-FRAMING.md:79`).
- ~~HTTP webhook listener server~~ — adds a network-listener dependency to a local-first engine;
  agy flagged as high-risk. Triggers remain file/cron/manual.
- ~~Sweep vendored LLM call-sites onto SpectynLLM~~ — eliminated once subtrees are deleted.
- ~~Visual editor / template marketplace~~ — far beyond the floor; defer to ROADMAP note only.

## Top-3 task breakdown

### P1 — Delete dead subtrees
- `git rm -r ai_automation_framework/ data_analysis/`; grep the repo to confirm `runner.py`,
  `llm_driver.py`, and `flows/*` have zero imports into them (expected: none).
- Strip any now-dangling references in `README.md` / `DESIGN.md` / `docs/`.
- Trim `requirements`/packaging to core deps only (`pyyaml`; `youtube_transcript_api` optional).
- Run the `youtube-summarize` e2e once post-deletion to prove the live path still works.

### P1 — Test contract + CI
- `runner.py` tests: `_resolve` placeholder substitution, `_gate_passes` (`when`), block-registry
  dispatch, `--dry-run` planning, unknown-block raises, in-process log/stdout actions — all
  offline.
- `llm_driver.py` tests: provider-noise stripping, timeout/error → fallback, `SPECTYN_FLOW_STUB_LLM`
  short-circuit (no network, no real `spectyn`).
- CI: run `pytest` + `python -m spectyn_flow.runner flows/<flow>.yaml --dry-run --json` as the gate
  (replace the import-only smoke).

### P1 — Harden subprocess boundary
- In `SpectynLLM.complete` / `_block_subprocess`: resolve the `spectyn` binary robustly, set an
  explicit timeout, capture+surface stderr, and fall back to the stub on non-zero/timeout.
- Add a `SPECTYN_FLOW_STUB_LLM` deterministic path so tests and offline/sandboxed machines never
  shell out.
- Cover OS-policy failures (e.g. the observed Windows `1312`) with a graceful, logged degradation
  rather than a crash; add a regression test around the failure path.

## Changes from draft
- **Cut** the four expansion items (vendored-tool blocks, cluster dispatch, webhook listener,
  vendored LLM-sweep) and the visual-editor P3 — all contradict the locked "delete dead code +
  de-claim" mandate or add network/scope risk.
- **Added/promoted** two cleanup items the draft missed: P1 *delete the ~100k-LOC subtrees* and
  P2 *de-claim README/DESIGN* (cluster/event-driven are overclaims to remove). Promoted
  subprocess-hardening to P1 (the draft's own logs prove it's the real fragility).
- **Kept** the draft's strongest cores: test contract + CI, validated-schema/structured-run, and
  honest stdlib-only flows.
- **agy review: usable.** Despite a noisy thinking-trace preamble, agy's verdict was correct and
  codebase-specific — I verified its two citations against `RESUME-SATELLITE-FRAMING.md` and
  `PRODUCT-GRADE-7-3-FOUNDATION.md`; both confirm the over-scope/de-claim correction. Its points
  (delete subtrees, cut cluster/webhook, harden subprocess) drove every change above.
