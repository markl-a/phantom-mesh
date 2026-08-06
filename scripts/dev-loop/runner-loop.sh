#!/usr/bin/env bash
# runner-loop.sh — persistent dev runner for a headless node (S4).
#
# The headless/codex form of the dev-node routine: a SUPERVISED while-loop that
# each tick (1) heartbeats via `spectyn status set`, (2) reads the node inbox
# (a "stop" directive shuts it down cleanly), (3) claims ONE open spec from the
# SHARED repo backlog (backlog.sh — atomic via git ref CAS), (4) lets the writer
# AI implement it on the claim's work branch, (5) runs the same gates as
# commute-loop (dev_verify + >=2-AI review + deviation-handler), and (6) lands
# BRANCHES-ONLY: PASS -> `backlog.sh done` (branch pushed, spec retired);
# anything else -> branch pushed for human review, honest status reported.
# NEVER touches main. Bounded by --max-minutes/--max-tasks (supervised Stage-1;
# unattended self-ignition stays §0.1-gated).
#
# Claude Code sessions run the SAME tick via the dev-node-routine skill (they
# are their own writer); this script is for nodes driven by codex/headless.
#
# Usage:
#   runner-loop.sh [--writer codex] [--tick-secs 300] [--max-tasks 5] [--max-minutes 480] [--once]
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
BACKLOG="$HERE/backlog.sh"
SPEC_GATE="$HERE/spec-gate.sh"
DEVIATION="$HERE/deviation-handler.sh"
REVIEW="$ROOT/scripts/local-ai/review.sh"
ASK="$ROOT/.claude/skills/local-ai/ask.sh"   # per-tool headless writer dispatch (codex/opencode/agy/claude)
. "$HERE/spec-lib.sh"

WRITER="${RUNNER_WRITER:-codex}"
TICK="${RUNNER_TICK_SECS:-300}"
MAX_TASKS="${RUNNER_MAX_TASKS:-5}"
MAX_MINUTES="${RUNNER_MAX_MINUTES:-480}"
BASE="${BACKLOG_BASE:-step3-coach-install-schedule}"
NODE="${BACKLOG_NODE:-$(hostname -s 2>/dev/null || hostname 2>/dev/null || echo unknown)}"
ONCE=0
while [ $# -gt 0 ]; do case "$1" in
  --writer) WRITER="${2:?}"; shift;;
  --tick-secs) TICK="${2:?}"; shift;;
  --max-tasks) MAX_TASKS="${2:?}"; shift;;
  --max-minutes) MAX_MINUTES="${2:?}"; shift;;
  --once) ONCE=1;;
  -h|--help) sed -n '2,21p' "$0"; exit 0;;
  *) echo "runner-loop: unknown arg '$1'" >&2; exit 2;;
esac; shift; done

cd "$ROOT" || exit 3

# ── spectyn binary resolution + watchdog ────────────────────────────────────
# Prefer the repo's OWN build (newest of debug/release — it matches the code
# this script shipped with). A PATH spectyn can be months old, and an old
# binary treats unknown subcommands ("status set …") as an implicit LLM
# prompt — a "heartbeat" that wedges the tick for minutes burning provider
# calls (caught live on a remote mac node, 6/10: PID ran `spectyn status set` as a
# chat for 1+ min).
newest_bin() {
  local best="" b
  for b in "$@"; do
    [ -x "$b" ] || continue
    if [ -z "$best" ] || [ "$b" -nt "$best" ]; then best="$b"; fi
  done
  printf '%s' "$best"
}
SPECTYN="$(newest_bin "$ROOT/core/target/debug/spectyn" "$ROOT/core/target/release/spectyn")"
[ -n "$SPECTYN" ] || SPECTYN="$(command -v spectyn 2>/dev/null || true)"

run_to() { # run_to <secs> <cmd…> — portable watchdog (macOS ships no `timeout`)
  local secs="$1"; shift
  ( "$@" ) & local c=$!
  ( sleep "$secs"; kill "$c" 2>/dev/null ) & local w=$!
  wait "$c" 2>/dev/null; local rc=$?
  kill "$w" 2>/dev/null; wait "$w" 2>/dev/null
  return "$rc"
}

# Heartbeats/inbox only when the binary really speaks them: the probe itself
# is watchdogged because on an old binary `status help` IS the LLM-prompt
# trap. Probe fails → run honestly without heartbeats instead of wedging.
if [ -n "$SPECTYN" ] && run_to 5 "$SPECTYN" status help 2>&1 | grep -q "dev-session heartbeat"; then
  SPECTYN_OK=1
else
  SPECTYN_OK=0
  echo "note: no status-capable spectyn binary on this node — running without heartbeats/inbox"
fi

hb() { # heartbeat — best-effort + bounded; must never kill or wedge the loop
  [ "$SPECTYN_OK" = 1 ] || return 0
  run_to 10 "$SPECTYN" status set --state "$1" ${2:+--task "$2"} ${3:+--branch "$3"} ${4:+--verdict "$4"} >/dev/null 2>&1 || true
}

inbox_stop_requested() {
  [ "$SPECTYN_OK" = 1 ] || return 1
  local msgs ids
  msgs="$(run_to 10 "$SPECTYN" inbox list --json 2>/dev/null)" || return 1
  [ -n "$msgs" ] && [ "$msgs" != "[]" ] || return 1
  echo "── inbox has messages ──"; run_to 10 "$SPECTYN" inbox list 2>/dev/null || true
  # Stage-1 directive vocabulary: a message whose text starts with "stop"
  # (topic optional) shuts the runner down; everything else is surfaced only.
  ids="$(printf '%s' "$msgs" | sed -n 's/.*"id": *"\([^"]*\)".*/\1/p')"
  if printf '%s' "$msgs" | grep -qi '"text": *"stop'; then
    for i in $ids; do run_to 10 "$SPECTYN" inbox ack "$i" >/dev/null 2>&1 || true; done
    return 0
  fi
  return 1
}

first_open_spec() {
  # `next` returns the first OPEN spec THIS node is allowed to claim — caps-matched,
  # so a node only picks up tasks for the platforms it can build (platform routing).
  BACKLOG_BASE="$BASE" BACKLOG_NODE="$NODE" bash "$BACKLOG" next 2>/dev/null | head -1
}

process_claimed() { # on work branch dev/<id>, tree clean at origin/BASE
  local id="$1" spec="backlog/$id.toml" scope prompt werc vexit rexit dv
  scope="$(spec_list "$(spec_section "$spec")" scope_allow | tr '\n' ' ')"
  prompt="Implement this task by editing ONLY these files: ${scope}. Component: $(spec_val "$(spec_section "$spec")" component). Acceptance: $(spec_val "$(spec_section "$spec")" acceptance). Add tests where appropriate. Apply edits directly to the files; touch no other file."
  hb working "$id" "dev/$id"
  # NOTE: never glue a multibyte char (e.g. U+2026 …) directly onto $VAR —
  # bash 3.2 under `set -u` parses the bytes into the variable name and dies
  # with "unbound variable" (caught live on a remote mac node, 6/10).
  echo "  [$id] dispatching writer: $WRITER"
  # Route through ask.sh's per-tool invocation (not codex-only `exec --dangerously…`
  # syntax): codex still runs the EXACT same `codex exec --dangerously-bypass-…`
  # command, while agy/opencode/claude now dispatch with their own correct headless
  # form instead of failing at exit-127 and burning a claim+release each tick.
  ASK_TIMEOUT="${RUNNER_WRITER_TIMEOUT:-$TICK}" "$ASK" "$WRITER" "$prompt" </dev/null >/dev/null 2>&1; werc=$?
  # Stage ONLY scope_allow paths — a pre-existing untracked stray (or an
  # out-of-scope writer edit) must never ride the AI commit (review: codex).
  # Out-of-scope writes simply stay unstaged and the diff check below treats
  # an in-scope-empty result honestly as "no change written".
  for p in $scope; do [ -e "$p" ] && git add -- "$p"; done
  # …and even INSIDE scope, files that were already untracked before this
  # tick are not the writer's work — unstage them (the snapshot guards both
  # staging here and deletion later). Caught live: a stray from an earlier
  # crashed run rode the demo commit and forged "writer wrote changes".
  if [ -n "$UNTRACKED_BASE" ]; then
    printf '%s\n' "$UNTRACKED_BASE" | while IFS= read -r u; do
      [ -n "$u" ] && git reset -q -- "$u" 2>/dev/null
    done
  fi
  if git diff --cached --quiet; then
    echo "  [$id] writer wrote nothing (exit $werc) — releasing claim for retry elsewhere"
    git checkout -q "$BASE" 2>/dev/null || git checkout -q -; git branch -qD "dev/$id" 2>/dev/null || true
    BACKLOG_BASE="$BASE" bash "$BACKLOG" release "$id" >/dev/null 2>&1 || true
    hb idle "" "" "writer:no-output"; return 0
  fi
  git commit -q -m "runner($NODE): $id (written by $WRITER)" || { echo "  [$id] commit failed"; return 0; }
  # Gates must judge EXACTLY the committed tree (review: codex r3): discard the
  # writer's out-of-scope tracked edits and its NEW untracked strays, so cargo
  # can't pass on a file the pushed branch doesn't contain. Untracked paths
  # that existed BEFORE this tick (snapshot in $UNTRACKED_BASE) are preserved.
  leftover_tracked="$(git status --porcelain | grep -v '^??' || true)"
  if [ -n "$leftover_tracked" ]; then
    echo "  [$id] discarding writer's out-of-scope tracked edits:"; printf '%s\n' "$leftover_tracked" | sed 's/^/      /'
    git checkout -q -- . 2>/dev/null || true
  fi
  git status --porcelain | sed -n 's/^?? //p' | while IFS= read -r u; do
    printf '%s\n' "$UNTRACKED_BASE" | grep -Fxq "$u" \
      || { rm -rf -- "$u"; echo "  [$id] discarded out-of-scope new path: $u"; }
  done
  if [ "$werc" -ne 0 ]; then
    echo "  [$id] writer exited $werc but wrote changes — pushing for human review, NOT gated-landing"
    git push -q -u origin "dev/$id" 2>/dev/null || true
    hb blocked "$id" "dev/$id" "writer:exit-$werc"; return 0
  fi
  if git diff --name-only HEAD~1..HEAD | grep -qE '\.rs$|(^|/)Cargo\.(toml|lock)$|(^|/)build\.rs$'; then
    if command -v cargo >/dev/null 2>&1; then ( cd "$ROOT/core" && cargo test --lib --bins >/dev/null 2>&1 ); vexit=$?
    else vexit=70; fi
  else vexit=0; fi
  "$REVIEW" HEAD~1..HEAD >/dev/null 2>&1; rexit=$?
  "$DEVIATION" --spec "$spec" --range HEAD~1..HEAD --verify-exit "$vexit" --review-exit "$rexit" >/dev/null 2>&1; dv=$?
  if [ "$dv" = 0 ]; then
    if BACKLOG_BASE="$BASE" BACKLOG_NODE="$NODE" bash "$BACKLOG" done "$id"; then
      echo "  [$id] ✅ PASS — dev/$id pushed, spec retired (verify=$vexit review=$rexit)"
      # §0.1 GUARDED auto-merge (owner 2026-06-10, "有護欄地開"): merge dev/<id> into
      # BASE ONLY when enabled + all-green + no R2 zone + clean; else stays branches-only
      # (the safe default). Kill-switch / dry-run live in ~/.spectyn-mesh — see auto-merge.sh.
      BACKLOG_BASE="$BASE" BACKLOG_NODE="$NODE" bash "$HERE/auto-merge.sh" "$id" \
        --verify-exit "$vexit" --review-exit "$rexit" --deviation "$dv"; amrc=$?
      case "$amrc" in
        0)  echo "  [$id] ✅✅ AUTO-MERGED dev/$id into $BASE (§0.1 guarded)"; hb idle "" "" "merged:$id";;
        20) echo "  [$id] auto-merge REFUSED (R2 zone) — branch kept for human"; hb idle "" "" "pass:$id";;
        10) hb idle "" "" "pass:$id";;   # disabled / dry-run / stop → branches-only (normal)
        *)  echo "  [$id] auto-merge deferred (rc=$amrc) — branches-only, branch pushed"; hb idle "" "" "pass:$id";;
      esac
    else
      echo "  [$id] gates passed but done/push failed — branch left locally"
    fi
  else
    git push -q -u origin "dev/$id" 2>/dev/null || true
    echo "  [$id] ⚠ deviation=$dv (verify=$vexit review=$rexit) — dev/$id pushed for human review, claim kept"
    hb blocked "$id" "dev/$id" "deviation:$dv"
  fi
  git checkout -q "$BASE" 2>/dev/null || true
}

echo "runner-loop on $NODE: writer=$WRITER base=$BASE tick=${TICK}s budget=${MAX_TASKS} tasks/${MAX_MINUTES} min (branches-only, supervised)"
# Preflight the writer BEFORE any claim — a missing writer must be a clear
# startup error, not an exit-127 discovered after a task is already claimed
# (caught live on a remote mac node: plain-ssh shells don't have the login PATH).
case "$WRITER" in
  codex|opencode|agy|claude) ;;   # known-good headless writers ask.sh can dispatch
  *) echo "✗ unsupported --writer '$WRITER' — must be one of: codex opencode agy claude (dispatched via ask.sh)"; exit 3;;
esac
[ -x "$ASK" ] || { echo "✗ writer dispatcher missing/not executable: $ASK"; exit 3; }
command -v "$WRITER" >/dev/null 2>&1 || {
  echo "✗ writer '$WRITER' not on PATH — start me from a login shell (bash -lc) or pass --writer <tool>"; exit 3; }
git config user.email >/dev/null 2>&1 || git config user.email "runner-loop@spectyn.local"
git config user.name  >/dev/null 2>&1 || git config user.name  "runner-loop ($NODE)"
START="$(date +%s)"; DONE_TASKS=0
while :; do
  if [ $(( $(date +%s) - START )) -ge $(( MAX_MINUTES * 60 )) ]; then echo "budget: max-minutes reached — stopping"; break; fi
  if [ "$DONE_TASKS" -ge "$MAX_TASKS" ]; then echo "budget: max-tasks reached — stopping"; break; fi
  if inbox_stop_requested; then echo "inbox: stop directive received — stopping"; break; fi
  dirty="$(git status --porcelain | grep -v '^??' || true)"
  if [ -n "$dirty" ]; then echo "working tree dirty — refusing to run (human state?); stopping"; break; fi
  # NOTE: framework self-update (S7) is deliberately NOT run here. Rebuilding the
  # repo mid-loop would rewrite this very script on disk, and bash reads an
  # executing file lazily by offset → corruption. Self-update runs from a CLEAN
  # OUTER trigger instead (cron / `/loop self-update` — the harness re-execs
  # cleanly), so this loop always runs a stable on-disk version. See
  # scripts/dev-loop/self-update.sh and the dev-node-update skill.
  id="$(first_open_spec)"
  if [ -z "$id" ]; then
    hb idle
    [ "$ONCE" = 1 ] && { echo "backlog empty — once mode, exiting"; break; }
    sleep "$TICK"; continue
  fi
  # snapshot of untracked paths BEFORE the writer runs — anything new after
  # the in-scope commit is a writer stray and gets discarded (codex r3)
  UNTRACKED_BASE="$(git status --porcelain | sed -n 's/^?? //p')"
  if BACKLOG_BASE="$BASE" BACKLOG_NODE="$NODE" bash "$BACKLOG" claim "$id"; then
    process_claimed "$id"
    DONE_TASKS=$((DONE_TASKS+1))
  else
    echo "  [$id] claim lost (raced) — next tick"
  fi
  [ "$ONCE" = 1 ] && break
  sleep "$TICK"
done
hb idle "" "" "runner:stopped"
echo "runner-loop stopped after $DONE_TASKS task(s)."
