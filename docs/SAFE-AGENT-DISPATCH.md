# Safe agent dispatch — how to drive non-Claude workers without blowing up the repo

**Problem (observed repeatedly):** when we dispatch a dev task to a remote `codex` cluster
worker, the worker auto-`git add -A`-commits **everything in its tree** — so a stray
`cargo fmt` (whole-repo reformat), a mass deletion, or edits to unrelated files all land on
the pushed branch and bury the real change. Claude subagents stay clean only because we hand
them an ad-hoc "stage only file X" instruction. We need this **systematized**.

**The industry answer (researched 2026-06-17; sources below) is one pattern: propose → GATE → apply.**
Validated by aider, SWE-agent, Cursor, Cline/Roo, OpenHands, Codex CLI, and a cluster of GitHub
guardrail tools. It maps directly onto our own apex ④ "safe unattended runs" governor — this is
the same idea applied to the **dev cluster itself** (dogfood).

## The 6 rules (what we adopt)

1. **Orchestrator–worker, never peers on shared state.** One decision-maker (Claude) holds the
   plan + the merge authority; workers are bounded and don't co-edit. *Parallelize reading/scoping/review,
   serialize or pre-contract the writes.* (Cognition "Don't Build Multi-Agents"; MAST: multi-agent
   write systems fail 41–87%, mostly from conflicting **implicit decisions** + missing verification.)
2. **Tight task contract per worker.** Objective + output format + tools + **explicit file boundaries** +
   "no `cargo fmt`, no unrelated deletions". Small enough to be **one reviewable diff, one green check**.
3. **Worktree / branch isolation.** One task → one branch → one worktree/clone. The worker's `git add -A`
   can then only damage the isolated copy; integration is a reviewable merge, not a fait accompli.
4. **GATE the branch before it reaches you/main** (`scripts/safe-dispatch.sh gate`): allowlist (in-scope
   files only) + denylist (never touch `.github`, `Cargo.lock`, secrets, `identity.key`) + budget
   (max files/lines) + **blow-up detectors** (whole-repo reformat via the `git diff -w` whitespace
   heuristic — safe for `.rs`; mass-deletion guard). Plain git only (no third-party action → no
   supply-chain risk per the tj-actions compromise; no file-count cap per the paths-filter bug).
5. **Never trust a worker's self-reported "done."** Independent verification (the gate + build/test +
   the project's ≥2-AI double-gate review) before merge.
6. **Keep the rollback free.** Every worker works on its own branch = one-command discard. The gate
   auto-`--delete`s a rejected branch so a blow-up never lingers.

## Usage

```bash
# Gate any branch a worker pushed (reject + reasons on violation):
bash scripts/safe-dispatch.sh gate <branch> origin/main '<allow_regex>' <max_files> <max_lines>
#   e.g.  ... gate overnight-p4/foo origin/main '^core/src/governed_run/' 3 400

# Dispatch a tightly-scoped task to a cluster worker AND gate it automatically
# (resets the clone, prepends the scope contract, dispatches, polls, gates, auto-discards a blow-up):
bash scripts/safe-dispatch.sh run ayaneo <slug> '<allow_regex>' <max_files> <max_lines> <task-body-file>
```

The `run` path prepends a STRICT SCOPE CONTRACT to the prompt (allowlist + no-fmt + budget) **and**
enforces it with the gate afterward — two layers, because the prompt is advisory and the gate is the
guarantee.

## When to use which executor (empirical, this project)

| Executor | Clean? | Use for |
|---|---|---|
| **Claude subagent** (Agent tool, worktree, "stage only X") | reliable | the actual code-writing, esp. sensitive/cross-file changes |
| **agy** (z13/Mac) | reliable for **read-only** | audits, second-opinion reviews, gap analysis (NOT tool-gated for edits) |
| **codex cluster** (ayaneo) | **needs the gate** | self-contained single-file tasks; ALWAYS run through `safe-dispatch` so a blow-up is auto-rejected |

## Sources
aider (git/repo-map), SWE-agent ACI (reject invalid edits), Codex CLI sandbox+approval, Claude Code
permission modes, Cursor agent security (allowlist>denylist), Cline/Roo (diff gate, checkpoints,
workspace boundary); Cognition *Don't Build Multi-Agents*; MAST (arXiv 2503.13657); Anthropic
multi-agent research system; LangGraph interrupt/checkpoint; git-worktree isolation; GitHub:
`commit-bloat-watcher`, `dorny/paths-filter`, `maidsafe/pr_size_checker`, `BerriAI/self-improving-agent`
(`onBeforeApply`), `KrxGu/ai-agent-guardrails`, the `agent-policy.yaml` writable/blocked-paths pattern.
Full URLs in the research notes (overnight/ research transcripts).
