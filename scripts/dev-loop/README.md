# scripts/dev-loop — autonomy governance mechanisms (Stage-1, shell)

The **safety net that must exist before any unattended dev loop runs** —
`../../docs/dev/AUTONOMY-GOVERNANCE.md`'s 支柱 1 (spec envelope) + 支柱 2 (deviation handling),
implemented in shell so they compose with the existing dev framework
(`scripts/dev-cluster/`, `scripts/local-ai/review.sh`) and run **now**, with no
SSH and no native build. They are the Stage-1 form of the future native
`spectyn dev spec validate` / `core/src/dev_loop/{spec_gate,deviation}.rs`
(the design doc lists those as "behind the gate" — so we emulate in shell first,
exactly as M2/M3 were, and port later).

> Core contract (governance §0): **無人自燃可以,但 (1) WHAT 由人+AI 事先共定,AI 只在
> spec 信封內動;(2) 不符合 spec 時按規則做無害化 + 正常化 + 通知 —— 不停死、也不亂跑。**

## Pieces

| script | pillar | what it does |
|---|---|---|
| `spec-gate.sh validate <spec.toml>` | 支柱1 | Validates the spec envelope. **No/incomplete spec → REJECT (don't do the task).** Requires `capability` ∈ {sense\|learn\|nudge\|dispatch} + `component` + `acceptance` + non-empty `scope_allow`. |
| `deviation-handler.sh --spec <f> [--range R\|--staged] [--verify-exit N] [--review-exit N]` | 支柱2 | After a work attempt: **detect (R1) → contain (R2) → normalize (R3) → notify (R4)**. Consumes the real `dev_verify` + `review.sh` exit codes (no fake green). |
| `status.sh [--all]` | — | Lists pending needs-human escalations + recent outcomes (stand-in for `spectyn dev status`). |
| `demo-governance.sh` | — | The **§4 demonstrable acceptance** — runs every R1–R5 path in a hermetic repo and asserts main is untouched + the moat ledger is byte-identical. |
| `spec-lib.sh` | — | Shared, section-anchored `[spec]` parser sourced by both gates (one parser, no drift). Robust to single/double quotes, trailing comments, and same-named keys in other sections. **Arrays must be single-line** (a multi-line array → 0 entries → spec-gate REJECT — fail-closed, never a silent empty scope). |
| `examples/spec.example.toml` | — | A complete spec envelope to copy. |

## R1–R5 (LOCKED 2026-06-08) → how the handler implements them

- **R1 deviation = ANY of**: (i) change outside `scope_allow`; (ii) `dev_verify` red
  (`--verify-exit != 0`); (iii) ≥1 reviewer REQUEST_CHANGES (`--review-exit != 0`,
  per `review.sh`/R5); (iv) over the bounded cap (`> max_files`, or an R2 zone);
  (v) same task fails ≥2× in a row (per-branch retry counter).
- **R2 contain (structural)**: the handler **never** merges, deletes, force-pushes,
  or touches CI/secret/schema — it only reads the diff and writes a ledger / proposal /
  notification. A diff that itself **touches an R2 zone or deletes a file → CONTAINED
  immediately (exit 30), no retry**. It also **refuses to run on `main`/`master`**.
- **R3 normalize**: a retryable deviation → **RETRY** (exit 10) up to
  `DEVIATION_MAX_ROUNDS` (default 2); still failing → **downgrade to a `needs-human`
  proposal** (diff + reason, **not merged**, isolated) + notify.
- **R4 stuck**: ≥2 consecutive deviations → **ESCALATE** (exit 20) = stop + notify owner.
  No skill-synthesis to force-patch.
- **R5 consensus**: consumed via `--review-exit` (review.sh: 0 APPROVE / else block).

### Exit codes
`0` PASS (land on branch, --no-ff) · `10` RETRY (normalize) · `20` ESCALATE
needs-human · `30` CONTAINED (R2 forbidden/destructive) · `3` setup error.

## 防污染牆 (支柱3)
The handler writes **only** to `~/.spectyn-mesh/dev-loop-log.jsonl` (+ escalations to
`deviation-proposals.jsonl`, notices to `notifications.log`). It contains **zero**
references to `partner-signals*` (the moat ledger). `demo-governance.sh` asserts the
real `partner-signals.jsonl` is byte-identical before/after.

## Where this plugs into the loop
```
spec-gate.sh validate spec.toml          # 支柱1: no spec → don't queue
  → loop runs the AI on the spec-bound task (on a dev branch)
  → dev_verify  → capture exit code
  → scripts/local-ai/review.sh → capture exit code        # 支柱3 (M3), R5
  → deviation-handler.sh --spec spec.toml --range … \
        --verify-exit $V --review-exit $R                 # 支柱2: decide
       0  → land (git merge --no-ff, branch only)
       10 → auto-correct toward spec, re-run
       20 → needs-human proposal, stop, owner notified
       30 → contained (R2), stop, owner notified
```

See `docs/dev/AUTONOMY-GOVERNANCE.md` (R1–R5 + §4) and `docs/_archive/EXECUTION-PLAN.md` §1.3
(Stage-1 acceptance items ②③⑤). Per `feedback_naming_no_c2_jargon` /
`feedback_test_real_path_not_skip`: plain dev terms, real paths — the demo exercises
the real handler on real git diffs, never a skipped path.
