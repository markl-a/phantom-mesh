#!/usr/bin/env bash
# selftest helper library — sourced by the orchestrator and by each feature test.
#
# Contract that every feature test file in scripts/selftest.d/ must follow:
#
#   1. Define `selftest_feature_meta` that prints `key=value` lines for at
#      least: name, priority (P0|P1|P2), requires (space-separated tags),
#      description. Optional: hints=<space-separated source paths>, used by
#      LLM agents as a starting grep set when this feature breaks.
#
#   2. Define `selftest_run` that performs the checks. Inside `selftest_run`,
#      call ONLY these helpers to record results — never `echo PASS` directly,
#      because the orchestrator parses the structured log to produce JSON.
#
#        t_pass  <test-name> [detail]
#        t_fail  <test-name> [detail]
#        t_skip  <test-name> [reason]
#        t_run   <test-name> <command...>     # auto pass/fail; full output
#                                             # captured to artifact, command
#                                             # recorded as repro
#        t_check <test-name> <repro-cmd>      # pass iff repro-cmd exits 0;
#                                             # full output captured
#
#      To attach an artifact / repro to a manual t_pass / t_fail, set:
#        T_REPRO="<command to re-run just this check>"
#        T_ARTIFACT="<absolute path to a log file>"
#      *immediately before* the t_pass/t_fail call. They auto-clear after.
#
#   3. Optional: define `selftest_requires` that exits 0 if prerequisites are
#      met (e.g. daemon running) or non-zero with a one-line reason on stderr
#      to skip the whole feature.
#
# Globals provided by the orchestrator: SPECTYN, COORD, TMP, SELFTEST_LOG,
# SELFTEST_FEATURE, SELFTEST_ARTIFACTS (per-feature dir, already created).

# ── log writer ────────────────────────────────────────────────────────────────
# Format (tab-separated, six fields, one row per recorded check):
#   feature  status  name  detail  repro  artifact

T_REPRO=""
T_ARTIFACT=""

_t_clean() {
  printf '%s' "$1" | tr '\t\n\r' '   '
}

_t_log() {
  local st="$1" name="$2" detail="${3:-}"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$SELFTEST_FEATURE" "$st" \
    "$(_t_clean "$name")" "$(_t_clean "$detail")" \
    "$(_t_clean "$T_REPRO")" "$(_t_clean "$T_ARTIFACT")" \
    >> "$SELFTEST_LOG"
  T_REPRO=""
  T_ARTIFACT=""
}

t_pass() {
  printf "  \033[32m✓\033[0m %-44s \033[90m%s\033[0m\n" "$1" "${2:-}"
  _t_log pass "$1" "${2:-}"
}

t_fail() {
  printf "  \033[31m✗\033[0m %-44s \033[90m%s\033[0m\n" "$1" "${2:-}"
  if [ -n "$T_ARTIFACT" ]; then
    printf "        \033[90mlog: %s\033[0m\n" "$T_ARTIFACT"
  fi
  if [ -n "$T_REPRO" ]; then
    printf "        \033[90mrepro: %s\033[0m\n" "$T_REPRO"
  fi
  _t_log fail "$1" "${2:-}"
}

t_skip() {
  printf "  \033[33m○\033[0m %-44s \033[90m%s\033[0m\n" "$1" "${2:-}"
  _t_log skip "$1" "${2:-}"
}

# slugify "GET /api/version" -> "get-api-version"
_t_slug() {
  printf '%s' "$1" | tr '[:upper:]' '[:lower:]' \
    | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//' | cut -c1-60
}

# Internal: run a shell-string command, capture all output to an artifact,
# return exit status. Used by t_run and t_check.
_t_capture() {
  local name="$1" cmd="$2"
  local slug; slug="$(_t_slug "$name")"
  local art="$SELFTEST_ARTIFACTS/${slug}.log"
  {
    printf '# command: %s\n' "$cmd"
    printf '# date:    %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf '# cwd:     %s\n' "$(pwd)"
    printf '# ──── stdout+stderr ────\n'
  } > "$art"
  bash -c "$cmd" >> "$art" 2>&1
  local rc=$?
  printf '# exit:    %s\n' "$rc" >> "$art"
  T_ARTIFACT="$art"
  T_REPRO="$cmd"
  return $rc
}

# t_run <name> <argv...>
# Pass on exit 0, fail otherwise. Full output captured to artifact; a
# re-runnable shell command stored as repro.
t_run() {
  local name="$1"; shift
  # Re-quote argv so the artifact's repro line is a literal pasteable command.
  local cmd=""
  for a in "$@"; do
    cmd+=" $(printf '%q' "$a")"
  done
  cmd="${cmd# }"
  if _t_capture "$name" "$cmd"; then
    local first; first="$(grep -v '^#' "$T_ARTIFACT" | head -1 | cut -c1-60)"
    t_pass "$name" "$first"
  else
    local last; last="$(grep -v '^#' "$T_ARTIFACT" | tail -1 | cut -c1-80)"
    t_fail "$name" "$last"
  fi
}

# t_check <name> <shell-string>
# Same as t_run but takes a shell command as a single string (so you can use
# pipes, redirection, etc.). The string IS the repro command.
t_check() {
  local name="$1" cmd="$2"
  if _t_capture "$name" "$cmd"; then
    local first; first="$(grep -v '^#' "$T_ARTIFACT" | head -1 | cut -c1-60)"
    t_pass "$name" "$first"
  else
    local last; last="$(grep -v '^#' "$T_ARTIFACT" | tail -1 | cut -c1-80)"
    t_fail "$name" "$last"
  fi
}

# ── small utilities feature tests can use ────────────────────────────────────

t_have() { command -v "$1" >/dev/null 2>&1; }

t_http() {
  local url="$1" want="${2:-200}" code
  code="$(curl -s --max-time 5 -o /dev/null -w '%{http_code}' "$url" 2>/dev/null || echo)"
  [ "$code" = "$want" ]
}

t_http_json() {
  local url="$1" filter="$2"
  curl -s --max-time 5 "$url" 2>/dev/null | jq -r "$filter" 2>/dev/null || true
}
