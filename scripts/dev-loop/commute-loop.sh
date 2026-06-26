#!/usr/bin/env bash
# commute-loop.sh — SUPERVISED unattended dev loop over a BOUNDED backlog.
#
# You start this on a STAY machine (one with cargo + >=2 headless AIs, e.g. acer/ayaneo) BEFORE
# you leave with your laptop. It walks a bounded backlog of spec-gated tasks within a budget;
# per task it dispatches a local AI to write the change on a fresh branch, runs dev_verify + the
# >=2-AI review gate + the deviation-handler, and either lands it BRANCHES-ONLY on a commute
# integration branch or leaves a needs-human note. It NEVER touches main, writes only to the
# dev-loop ledger (防污染牆, never partner-signals), and leaves a report you read on return.
# This is the SUPERVISED form (you authorised this bounded run): no self-ignition, no auto-merge
# to main — those are §0.1-gated.
#
# Usage:
#   commute-loop.sh [--backlog DIR] [--max-tasks N] [--max-minutes M] [--writer codex] [--dry-run]
#     backlog: dir of *.toml spec files (default ~/.phantom-mesh/backlog). Write them before leaving.
#     --dry-run: validate the backlog + print the plan, calling NO AIs / cargo.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
ASK="$ROOT/.claude/skills/local-ai/ask.sh"
SPEC_GATE="$HERE/spec-gate.sh"
DEVIATION="$HERE/deviation-handler.sh"
REVIEW="$ROOT/scripts/local-ai/review.sh"
STATE_DIR="${PHANTOM_STATE_DIR:-${HOME}/.phantom-mesh}"
. "$HERE/spec-lib.sh"   # shared [spec] parser (single/double quotes, section-anchored)

BACKLOG="${COMMUTE_BACKLOG:-$STATE_DIR/backlog}"
MAX_TASKS="${COMMUTE_MAX_TASKS:-5}"
MAX_MINUTES="${COMMUTE_MAX_MINUTES:-120}"
WRITER="${COMMUTE_WRITER:-codex}"
INTEGRATION="${COMMUTE_BRANCH:-commute-integration}"
DRY=0
while [ $# -gt 0 ]; do case "$1" in
  --backlog) BACKLOG="${2:?--backlog needs a directory}"; shift;;
  --max-tasks) MAX_TASKS="${2:?--max-tasks needs a number}"; shift;;
  --max-minutes) MAX_MINUTES="${2:?--max-minutes needs a number}"; shift;;
  --writer) WRITER="${2:?--writer needs a tool}"; shift;;
  --dry-run) DRY=1;; -h|--help) sed -n '2,20p' "$0"; exit 0;;
  *) echo "commute-loop: unknown arg '$1'" >&2; exit 2;;
esac; shift; done

mkdir -p "$STATE_DIR"
REPORT="$STATE_DIR/commute-report-$(date +%Y%m%d-%H%M%S 2>/dev/null || echo run).md"
START="$(date +%s 2>/dev/null || echo 0)"
DEADLINE=$(( START + MAX_MINUTES * 60 ))
RUN_ID="$(date +%H%M%S 2>/dev/null || echo run)"   # per-run suffix so re-runs never overwrite a saved branch
START_BRANCH="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)"

log() { printf '%s\n' "$*"; printf '%s\n' "$*" >> "$REPORT"; }

# [spec] field accessors via the shared parser (handles single/double quotes, section-anchored).
_specval()   { local sec; sec="$(spec_section "$1")"; spec_val "$sec" "$2"; }
_specscope() { local sec; sec="$(spec_section "$1")"; spec_list "$sec" scope_allow | tr '\n' ' '; }

preflight() {
  local n=0 t
  command -v cargo >/dev/null 2>&1 && CARGO_OK=1 || { CARGO_OK=0; log "⚠ no cargo here — Rust tasks can't be dev_verified and will be left for you."; }
  for t in codex opencode agy; do command -v "$t" >/dev/null 2>&1 && n=$((n+1)); done
  [ "$n" -ge 2 ] || log "⚠ only $n local AI(s) here — the >=2-AI review gate may report INSUFFICIENT; those tasks are left for you."
  [ -x "$ASK" ] && [ -x "$REVIEW" ] && [ -x "$DEVIATION" ] && [ -x "$SPEC_GATE" ] || { log "✗ missing helper scripts under $HERE / local-ai — aborting."; return 3; }
  case "$WRITER" in
    codex|opencode|agy|claude) ;;   # known-good headless writers ask.sh can dispatch
    *) log "✗ unsupported --writer '$WRITER' — must be one of: codex opencode agy claude (dispatched via ask.sh). Aborting."; return 3;;
  esac
  command -v "$WRITER" >/dev/null 2>&1 || { log "✗ writer '$WRITER' not on PATH — aborting."; return 3; }
  return 0
}

process_task() {
  local spec="$1" name branch prompt scope werc vexit rexit dv
  name="$(basename "$spec" .toml)"; branch="dev/commute-$name-$RUN_ID"   # run-unique: never clobber a saved branch
  if ! "$SPEC_GATE" validate "$spec" >/dev/null 2>&1; then log "  [$name] SKIP — spec failed spec-gate"; return 0; fi
  scope="$(_specscope "$spec")"
  # Always start each task from a CLEAN integration state — a prior task or the writer may have
  # left the tree dirty, and that must never carry into the next branch.
  git checkout -q "$INTEGRATION" 2>/dev/null || true
  git reset -q --hard 2>/dev/null || true; git clean -qfd 2>/dev/null || true
  git checkout -q -B "$branch" "$INTEGRATION" 2>/dev/null || { log "  [$name] SKIP — could not branch"; return 0; }
  prompt="Implement this task by editing ONLY these files: ${scope}. Component: $(_specval "$spec" component). Acceptance: $(_specval "$spec" acceptance). Add tests where appropriate. Apply edits directly to the files; touch no other file."
  log "  [$name] dispatching $WRITER (scope: ${scope})…"
  # Route through ask.sh's per-tool invocation (not codex-only `exec --dangerously…`
  # syntax): codex still runs the EXACT same `codex exec --dangerously-bypass-…`
  # command, while agy/opencode/claude now dispatch with their own correct headless
  # form instead of failing at exit-127.
  ASK_TIMEOUT="${COMMUTE_WRITER_TIMEOUT:-600}" "$ASK" "$WRITER" "$prompt" </dev/null >/dev/null 2>&1; werc=$?
  git add -A
  if git diff --cached --quiet; then
    if [ "$werc" -ne 0 ]; then log "  [$name] writer '$WRITER' FAILED (exit $werc) — left for you"; else log "  [$name] no change written — SKIP"; fi
    git reset -q --hard 2>/dev/null || true; git checkout -q "$INTEGRATION"; return 0
  fi
  if ! git commit -q -m "commute: $name (written by $WRITER)"; then
    log "  [$name] git commit FAILED (hooks / git config?) — skipped"
    git reset -q --hard 2>/dev/null || true; git checkout -q "$INTEGRATION"; return 0
  fi
  # A writer that errored MID-run may have written an incomplete change — never auto-land it.
  if [ "$werc" -ne 0 ]; then
    log "  [$name] writer '$WRITER' exited $werc but wrote changes — committed on $branch, left for your review (NOT auto-landed)"
    git checkout -q "$INTEGRATION"; return 0
  fi
  # dev_verify: a Rust OR cargo-manifest/build change must compile + test (lib + bins, so binary
  # compile errors are caught too); other changes skip cargo (treated verified). Needs cargo to land.
  if git diff --name-only HEAD~1..HEAD | grep -qE '\.rs$|(^|/)Cargo\.(toml|lock)$|(^|/)build\.rs$'; then
    if [ "${CARGO_OK:-0}" = 1 ]; then ( cd "$ROOT/core" && cargo test --lib --bins >/dev/null 2>&1 ); vexit=$?
    else vexit=70; fi
  else vexit=0; fi
  # >=2-AI review + deviation
  "$REVIEW" HEAD~1..HEAD >/dev/null 2>&1; rexit=$?
  "$DEVIATION" --spec "$spec" --range HEAD~1..HEAD --verify-exit "$vexit" --review-exit "$rexit" >/dev/null 2>&1; dv=$?
  case "$dv" in
    0)  git checkout -q "$INTEGRATION"; git merge --no-ff -q "$branch" -m "commute land: $name (verify=$vexit review=$rexit)"
        log "  [$name] ✅ LANDED on $INTEGRATION  (verify=$vexit review=$rexit)";;
    10) git checkout -q "$INTEGRATION"; log "  [$name] ⚠ RETRY/CHANGES — left on $branch for your review (verify=$vexit review=$rexit)";;
    20) git checkout -q "$INTEGRATION"; log "  [$name] 🚩 NEEDS-HUMAN (escalated) — proposal filed; branch $branch";;
    30) git checkout -q "$INTEGRATION"; log "  [$name] 🛑 CONTAINED (R2) — branch $branch left isolated";;
    *)  git checkout -q "$INTEGRATION"; log "  [$name] ✗ setup error (deviation=$dv, verify=$vexit) — left on $branch";;
  esac
}

main() {
  log "# commute-loop report — $(date 2>/dev/null || echo run) on $(hostname -s 2>/dev/null || echo host)"
  log "backlog=$BACKLOG  max-tasks=$MAX_TASKS  max-minutes=$MAX_MINUTES  writer=$WRITER  integration=$INTEGRATION"
  local specs=() s
  if [ -d "$BACKLOG" ]; then for s in "$BACKLOG"/*.toml; do [ -f "$s" ] && specs+=("$s"); done; fi
  [ "${#specs[@]}" -gt 0 ] || { log "backlog empty ($BACKLOG) — nothing to do. Drop spec .toml files there before leaving."; return 0; }
  log "found ${#specs[@]} spec(s) in backlog."
  preflight || return 3
  if [ "$DRY" = 1 ]; then
    log "--- DRY RUN (no AI/cargo; validating backlog + plan) ---"
    local i=0
    for s in "${specs[@]}"; do
      i=$((i+1)); [ "$i" -gt "$MAX_TASKS" ] && { log "  (budget: max-tasks=$MAX_TASKS — remaining deferred)"; break; }
      if "$SPEC_GATE" validate "$s" >/dev/null 2>&1; then log "  [$(basename "$s" .toml)] OK — would dispatch $WRITER, scope: $(_specscope "$s")"
      else log "  [$(basename "$s" .toml)] REJECT — spec incomplete (would skip)"; fi
    done
    log "--- dry run done; report: $REPORT ---"; return 0
  fi
  # headless-safe git + a clean tree (both break an unattended run otherwise).
  git config user.email >/dev/null 2>&1 || git config user.email "commute-loop@phantom.local"
  git config user.name  >/dev/null 2>&1 || git config user.name  "phantom-commute-loop"
  git config commit.gpgsign false 2>/dev/null || true
  if [ -n "$(git status --porcelain 2>/dev/null)" ]; then
    log "✗ working tree not clean — commit or stash your local changes first (won't risk mixing them into tasks). Aborting."
    return 3
  fi
  # create the integration branch off where we started, branches-only
  git checkout -q -B "$INTEGRATION" "$START_BRANCH" 2>/dev/null || { log "✗ could not create integration branch"; return 3; }
  local n=0 now
  for s in "${specs[@]}"; do
    now="$(date +%s 2>/dev/null || echo 0)"
    [ "$n" -ge "$MAX_TASKS" ] && { log "budget: reached max-tasks=$MAX_TASKS — stopping."; break; }
    [ "$now" -ge "$DEADLINE" ] && { log "budget: reached max-minutes=$MAX_MINUTES — stopping."; break; }
    n=$((n+1)); log "[task $n/$MAX_TASKS] $(basename "$s" .toml)"
    process_task "$s"
  done
  git checkout -q "$START_BRANCH" 2>/dev/null || true
  log ""
  log "## done: $n task(s) processed. Review on return:"
  log "   • landed work:  git log --oneline $INTEGRATION   (branches-only; cherry-pick/merge what you approve)"
  log "   • escalations:  $STATE_DIR/deviation-proposals.jsonl ; $HERE/status.sh"
  log "   • main untouched. Report: $REPORT"
}
main
