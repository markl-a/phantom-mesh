#!/usr/bin/env bash
# full-lifecycle-mac.sh — G-DBG-1: run the user's whole Mac CLI lifecycle end to
# end against a clean isolated $HOME, hard-gating every step on its exit code,
# while capturing logs + (optional) screenshots for debugging (G-DBG-2/3).
#
# This is L2 of the test pyramid (terminal E2E with the REAL binary) — it ADDS
# to, does not replace, the cargo tests. No step is "passed" without a 0 exit
# code; the script prints `E2E RESULT: PASS|FAIL` and, on FAIL, auto-collects a
# debug bundle (G-DBG-4).
#
# Usage:
#   scripts/e2e/full-lifecycle-mac.sh                # uses ~/.cargo/bin/spectyn
#   SPECTYN_BIN=/path/to/spectyn scripts/e2e/full-lifecycle-mac.sh
#   KEEP_HOME=1 scripts/e2e/...                      # don't delete the temp HOME
#
# Honesty: a step that the binary does not yet implement is reported FAIL, not
# skipped silently. Steps known to be partial are marked [partial] in the name
# but still gated — so the script tells the truth about what works today.

set -uo pipefail

BIN="${SPECTYN_BIN:-$HOME/.cargo/bin/spectyn}"
TS="$(date +%Y%m%d-%H%M%S)"
RUN_HOME="$(mktemp -d "${TMPDIR:-/tmp}/spectyn-e2e-$TS.XXXXXX")"
LOG="${TMPDIR:-/tmp}/spectyn-e2e-$TS.log"
SHOTS_DIR="${TMPDIR:-/tmp}/spectyn-e2e-$TS.shots"
mkdir -p "$SHOTS_DIR"

steps=0; passed=0; failed=0
FAILED_STEPS=""

note() { printf '%s\n' "$*" | tee -a "$LOG"; }

# step "<name>" <cmd...> — run cmd under the isolated HOME with debug logging,
# tee everything to $LOG, gate on exit code.
step() {
  local name="$1"; shift
  steps=$((steps+1))
  note "▶ STEP $steps: $name"
  note "  \$ $*"
  # SPECTYN_LOG=debug for verbose logs; HOME isolated; merge stderr→stdout→log.
  if HOME="$RUN_HOME" SPECTYN_LOG=debug "$@" >>"$LOG" 2>&1; then
    note "  ✓ PASS (exit 0)"
    passed=$((passed+1))
    return 0
  else
    local rc=$?
    note "  ✗ FAIL (exit $rc)"
    failed=$((failed+1))
    FAILED_STEPS="$FAILED_STEPS\n    - step $steps: $name (exit $rc)"
    return 1
  fi
}

# A step whose stdout we want to ASSERT on (not just exit code). Captures to a
# temp file, greps, gates on both exit code AND the match.
step_expect() {
  local name="$1" pat="$2"; shift 2
  steps=$((steps+1))
  note "▶ STEP $steps: $name  (expect: /$pat/)"
  note "  \$ $*"
  local out; out="$(HOME="$RUN_HOME" SPECTYN_LOG=debug "$@" 2>>"$LOG")"
  local rc=$?
  printf '%s\n' "$out" >>"$LOG"
  if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q "$pat"; then
    note "  ✓ PASS (exit 0, matched)"
    passed=$((passed+1)); return 0
  fi
  note "  ✗ FAIL (exit $rc, match=$(printf '%s' "$out" | grep -qc "$pat" && echo yes || echo no))"
  failed=$((failed+1))
  FAILED_STEPS="$FAILED_STEPS\n    - step $steps: $name (exit $rc / pattern)"
  return 1
}

note "=== spectyn Mac CLI full-lifecycle E2E ==="
note "bin:  $BIN"
note "HOME: $RUN_HOME"
note "log:  $LOG"
note ""

[ -x "$BIN" ] || { note "✗ spectyn binary not found/executable at $BIN"; exit 2; }

# ── The user's lifecycle, in order ──────────────────────────────────────────
# CUJ-01 activation
step_expect "spectyn --version"            "0\.6\.0"          "$BIN" --version
step        "identity init (keys init)"                       "$BIN" keys init
step        "habit create water"                              "$BIN" habit create water --label "水"
step        "habit checkin water"                             "$BIN" habit checkin water "morning glass"
step        "habit list"                                      "$BIN" habit list
# CUJ-02 capture loop (focus is timer-mode; coach review is offline aggregate)
step        "focus start 1min"                                "$BIN" focus start --minutes 1 --task "deep work"
step        "focus status"                                    "$BIN" focus status
step        "focus stop"                                      "$BIN" focus stop
step        "coach review (offline aggregate)"                "$BIN" coach review --date "$(date +%Y-%m-%d)"
# CUJ-05 export + delete (data portability / kill switch)
step        "data stats"                                      "$BIN" data stats
step        "data export --format json"                       "$BIN" data export --format json --out "$RUN_HOME/export.json"
step        "data export --format md"                         "$BIN" data export --format md --out "$RUN_HOME/export.md"
step        "data delete --all --yes (kill switch)"           "$BIN" data delete --all --yes

# ── Result + debug bundle on failure ────────────────────────────────────────
note ""
note "E2E RESULT: $([ "$failed" -eq 0 ] && echo PASS || echo FAIL) ($passed/$steps steps, $(ls "$SHOTS_DIR" 2>/dev/null | wc -l | tr -d ' ') screenshots, log=$LOG)"
[ "$failed" -ne 0 ] && note "failed steps:$(printf '%b' "$FAILED_STEPS")"

if [ "$failed" -ne 0 ]; then
  BUNDLE_DIR="$RUN_HOME" LOG="$LOG" SHOTS="$SHOTS_DIR" \
    bash "$(dirname "$0")/collect-debug-bundle.sh" "$RUN_HOME" "$LOG" "$SHOTS_DIR" || true
fi

if [ "${KEEP_HOME:-0}" = "1" ]; then
  note "(KEEP_HOME=1 — left $RUN_HOME in place)"
else
  rm -rf "$RUN_HOME"
fi

[ "$failed" -eq 0 ]
