#!/usr/bin/env bash
# commute-keepalive.sh — persistent UNATTENDED branches-only dev loop (M5).
#
# Runs `runner-loop.sh` in repeating BOUNDED cycles so the fleet keeps working the
# shared backlog while you're away ("通勤續跑"). Start it DETACHED (nohup / launchd
# on macOS / a Windows scheduled task) and it survives the session/SSH that
# launched it. Each node only picks up tasks for its own caps (platform routing).
#
# SAFETY (this is the branches-only commute mode the plan opens BEFORE the §0.1
# gate — it never merges, so there is nothing to review-pollute):
#   * branches-only + governed: every cycle runs runner-loop, which enforces
#     spec-gate + dev_verify + ≥2-AI review + deviation-handler + the pollution
#     wall, and lands work ONLY on dev/<id> branches you review later.
#   * bounded per cycle: COMMUTE_CYCLE_TASKS / COMMUTE_CYCLE_MINUTES cap each
#     burst, then a breather — never an unbounded runaway.
#   * NEVER auto-merges to the base branch (that stays §0.1-gated).
#   * clean stop: `touch ~/.spectyn-mesh/commute-stop` (the loop finishes its
#     current cycle and exits); the file is consumed on exit.
#
# Usage:
#   bash scripts/dev-loop/commute-keepalive.sh [--writer codex] [--once]
#   # detached for a real commute (survives the launching shell). log() already
#   # writes a durable trail to ~/.spectyn-mesh/commute.log, so send stdout to
#   # /dev/null — redirecting it back to the same file double-writes every line.
#   nohup bash scripts/dev-loop/commute-keepalive.sh >/dev/null 2>&1 &
#   tail -f ~/.spectyn-mesh/commute.log   # watch it
# Env: COMMUTE_WRITER (default codex), COMMUTE_CYCLE_TASKS (3),
#      COMMUTE_CYCLE_MINUTES (30), COMMUTE_IDLE_SLEEP (300), COMMUTE_MAX_HOURS (0=∞)
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNNER="${COMMUTE_RUNNER:-$HERE/runner-loop.sh}"   # overridable for tests
STATE_DIR="${SPECTYN_STATE_DIR:-$HOME/.spectyn-mesh}"
export SPECTYN_STATE_DIR="$STATE_DIR"   # so runner-loop coordinates in the same dir
STOP="$STATE_DIR/commute-stop"
LOG="$STATE_DIR/commute.log"

WRITER="${COMMUTE_WRITER:-codex}"
CYCLE_TASKS="${COMMUTE_CYCLE_TASKS:-3}"
CYCLE_MINUTES="${COMMUTE_CYCLE_MINUTES:-30}"
IDLE_SLEEP="${COMMUTE_IDLE_SLEEP:-300}"
MAX_HOURS="${COMMUTE_MAX_HOURS:-0}"     # 0 = run until the stop file appears
ONCE=0
while [ $# -gt 0 ]; do case "$1" in
  --writer) WRITER="${2:?--writer needs a tool}"; shift;;
  --once) ONCE=1;;
  -h|--help) sed -n '2,28p' "$0"; exit 0;;
  *) echo "commute-keepalive: unknown arg '$1'" >&2; exit 2;;
esac; shift; done

mkdir -p "$STATE_DIR"
# Tee to both stdout (so a foreground/redirected run shows it) and a durable log
# file (so a detached run always leaves a trail even if stdout went nowhere).
log() { local m; m="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date)  $*"; printf '%s\n' "$m"; printf '%s\n' "$m" >> "$LOG" 2>/dev/null || true; }

# Preflight: a missing writer must fail loudly, not spin forever doing nothing.
command -v "$WRITER" >/dev/null 2>&1 || {
  log "✗ writer '$WRITER' not on PATH — start from a login shell (bash -lc) or pass --writer"; exit 3; }
[ -f "$RUNNER" ] || { log "✗ runner-loop.sh not found at $RUNNER"; exit 3; }

rm -f "$STOP"   # a stale stop file must not abort us before we even start
START="$(date +%s 2>/dev/null || echo 0)"
log "commute-keepalive START (writer=$WRITER, cycle=${CYCLE_TASKS} tasks/${CYCLE_MINUTES} min, idle=${IDLE_SLEEP}s, max-hours=${MAX_HOURS}, branches-only)"

while :; do
  if [ -f "$STOP" ]; then log "stop file present — exiting cleanly"; break; fi
  # Only enforce the wall-clock cap when we actually got a START stamp; if the
  # date(1) at startup failed (START=0) we must not exit on the very first tick.
  if [ "$MAX_HOURS" != 0 ] && [ "$START" != 0 ] && \
     [ "$(( $(date +%s) - START ))" -ge "$(( MAX_HOURS * 3600 ))" ]; then
    log "max-hours reached — exiting"; break
  fi
  log "── cycle: runner-loop (writer=$WRITER, ≤${CYCLE_TASKS} tasks / ${CYCLE_MINUTES} min) ──"
  # runner-loop is itself bounded + branches-only + governed; one cycle = one bounded burst.
  RUNNER_MAX_TASKS="$CYCLE_TASKS" RUNNER_MAX_MINUTES="$CYCLE_MINUTES" \
    bash "$RUNNER" --writer "$WRITER" 2>&1 \
    | while IFS= read -r l || [ -n "$l" ]; do log "  $l"; done
  [ "$ONCE" = 1 ] && { log "--once: one cycle done, exiting"; break; }
  [ -f "$STOP" ] && { log "stop file present after cycle — exiting"; break; }
  log "cycle done — breather ${IDLE_SLEEP}s (touch $STOP to stop)"
  sleep "$IDLE_SLEEP"
done

rm -f "$STOP"
log "commute-keepalive STOPPED"
