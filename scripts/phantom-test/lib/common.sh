# shellcheck shell=bash
# common.sh — shared helpers for phantom-test scenarios.
# Sourced by every scenario via:
#   source "$(dirname "$0")/../lib/common.sh"
#
# Public surface:
#   scenario "<title>"           — declare scenario name (printed in summary)
#   step "<what we are doing>"   — log a step (info, no assertion)
#   pass "<msg>" / fail "<msg>"  — record outcome (used by ASSERT_*)
#   ASSERT_EQ <a> <b> [label]
#   ASSERT_CONTAINS <haystack> <needle> [label]
#   ASSERT_NOT_CONTAINS <haystack> <needle> [label]
#   ASSERT_HTTP <url> <expected_code> [label]
#   ASSERT_FILE_GREW <path> <since_size> [label]
#   require_cmd <cmd>            — abort scenario if cmd not on PATH
#   tmpdir                       — echo a per-scenario temp dir (auto-cleaned on exit)
#
# Exit codes: 0 = all asserts pass / 1 = any fail / 77 = scenario skipped.

set -u  # Treat unset vars as errors. Don't `set -e` — we want to keep running after a fail.

# ── Colors (auto-disable when not a TTY) ─────────────────────────────────────
if [ -t 1 ] && [ "${NO_COLOR:-}" = "" ]; then
  C_RESET=$'\033[0m'; C_DIM=$'\033[2m'
  C_RED=$'\033[31m'; C_GREEN=$'\033[32m'; C_YELLOW=$'\033[33m'; C_CYAN=$'\033[36m'
else
  C_RESET=''; C_DIM=''; C_RED=''; C_GREEN=''; C_YELLOW=''; C_CYAN=''
fi

# ── Per-scenario state ───────────────────────────────────────────────────────
# These get reset per scenario by the runner; scenarios should not assume
# a clean slate from the previous scenario.
PHANTOM_TEST_NAME="${0##*/}"
PHANTOM_TEST_PASSED=0
PHANTOM_TEST_FAILED=0
PHANTOM_TEST_TMP="${TMPDIR:-/tmp}/phantom-test-$$-${PHANTOM_TEST_NAME%.sh}"
mkdir -p "$PHANTOM_TEST_TMP"
trap 'rm -rf "$PHANTOM_TEST_TMP"' EXIT

# ── Logging ──────────────────────────────────────────────────────────────────
scenario() {
  printf '%s━━ %s ━━%s\n' "$C_CYAN" "$*" "$C_RESET"
}
step() {
  printf '  %s→%s %s\n' "$C_DIM" "$C_RESET" "$*"
}
pass() {
  PHANTOM_TEST_PASSED=$((PHANTOM_TEST_PASSED + 1))
  printf '  %s✓%s %s\n' "$C_GREEN" "$C_RESET" "$*"
}
fail() {
  PHANTOM_TEST_FAILED=$((PHANTOM_TEST_FAILED + 1))
  printf '  %s✗%s %s\n' "$C_RED" "$C_RESET" "$*" >&2
}
warn() {
  printf '  %s⚠%s %s\n' "$C_YELLOW" "$C_RESET" "$*" >&2
}

# ── Assertions ───────────────────────────────────────────────────────────────
ASSERT_EQ() {
  local a="$1" b="$2" label="${3:-equal}"
  if [ "$a" = "$b" ]; then
    pass "$label: '$a' == '$b'"
  else
    fail "$label: expected '$b', got '$a'"
  fi
}

ASSERT_CONTAINS() {
  local hay="$1" needle="$2" label="${3:-contains}"
  case "$hay" in
    *"$needle"*) pass "$label: found '$needle'" ;;
    *)           fail "$label: '$needle' not in: ${hay:0:200}" ;;
  esac
}

ASSERT_NOT_CONTAINS() {
  local hay="$1" needle="$2" label="${3:-not contains}"
  case "$hay" in
    *"$needle"*) fail "$label: '$needle' should NOT appear in: ${hay:0:200}" ;;
    *)           pass "$label: '$needle' absent" ;;
  esac
}

ASSERT_HTTP() {
  local url="$1" want="$2" label="${3:-http}"
  local got
  got=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 5 "$url" 2>/dev/null || echo "000")
  if [ "$got" = "$want" ]; then
    pass "$label: $url -> $got"
  else
    fail "$label: $url -> $got (wanted $want)"
  fi
}

ASSERT_FILE_GREW() {
  local path="$1" prev="$2" label="${3:-file grew}"
  local now
  now=$(stat -c %s "$path" 2>/dev/null || stat -f %z "$path" 2>/dev/null || echo 0)
  if [ "$now" -gt "$prev" ]; then
    pass "$label: $path size $prev → $now bytes (+$((now-prev)))"
  else
    fail "$label: $path size $prev → $now bytes (no growth)"
  fi
}

# ── Utilities ────────────────────────────────────────────────────────────────
require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    warn "skipping scenario — required command not on PATH: $1"
    exit 77
  fi
}

tmpdir() { printf '%s\n' "$PHANTOM_TEST_TMP"; }

# Each scenario sets these via env; the runner provides defaults.
PHANTOM_BIN="${PHANTOM_BIN:-phantom}"
PHANTOM_HOST="${PHANTOM_HOST:-127.0.0.1}"
PHANTOM_PORT="${PHANTOM_PORT:-7879}"
PHANTOM_CLUSTER_SECRET="${PHANTOM_CLUSTER_SECRET:-changeme-cluster-secret}"
PHANTOM_CONFIG_DIR="${PHANTOM_CONFIG_DIR:-$HOME/.phantom-mesh}"
