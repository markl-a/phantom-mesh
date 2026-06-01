#!/usr/bin/env bash
# SPEC-60 V2 ship-gate — local coverage runner.
#
# Spec source: docs/superpowers/specs/v060-deep-spec/SPEC-60-TESTING-strategy.md §8.2
# Reader guide: docs/superpowers/v2-coverage-gate.md
# CI counterpart: .github/workflows/ship-gate-coverage.yml
#
# What this script does
# ---------------------
#   default mode (operator pre-push)
#     1. Run `cargo llvm-cov --summary-only --workspace` for an at-a-glance
#        view in the terminal.
#     2. Also emit `coverage.json` to the repo root for the per-spec parser.
#     3. Parse it against the exception list and exit 0 / 1 with a clear
#        red-bar / green-bar report.
#
#   --ci mode (called from ship-gate-coverage.yml)
#     1. Skip the `cargo llvm-cov` run because the CI step already did it
#        (saves ~5-8 minutes — we just need to parse the existing JSON).
#     2. Read paths from env vars `PHANTOM_V2_COVERAGE_JSON` +
#        `PHANTOM_V2_EXCEPTIONS_YAML` + `PHANTOM_V2_REPORT_OUT` so the
#        workflow does not have to know the file layout.
#     3. Write the human-readable report to `PHANTOM_V2_REPORT_OUT` so the
#        artifact upload step can grab it.
#
# Exit codes
#   0  = all specs ≥ threshold OR every shortfall covered by a non-expired
#        exception in `ship-gate-coverage-exceptions.yaml`
#   1  = at least one spec below threshold without (or with expired) exception
#   2  = environment problem (cargo-llvm-cov missing / yq missing / paths bad)
#
# Hard rule (CLAUDE.md): OSS-safe output — no operator hostnames or paths.
# We deliberately print repo-relative paths only.

set -euo pipefail

# ─── arg parse ────────────────────────────────────────────────────────────
CI_MODE=0
for arg in "$@"; do
  case "$arg" in
    --ci) CI_MODE=1 ;;
    -h|--help)
      cat <<'EOF'
Usage: scripts/ship-gate/v2-coverage-local.sh [--ci]

  (no args)  Run cargo llvm-cov + per-spec gate (operator pre-push mode).
  --ci       Skip cargo-llvm-cov run; only parse pre-generated JSON
             (called from .github/workflows/ship-gate-coverage.yml).

Env vars honoured (CI mode):
  PHANTOM_V2_COVERAGE_THRESHOLD  default 80
  PHANTOM_V2_COVERAGE_JSON       default ./coverage.json
  PHANTOM_V2_EXCEPTIONS_YAML     default .github/workflows/ship-gate-coverage-exceptions.yaml
  PHANTOM_V2_REPORT_OUT          default ./coverage-per-spec-report.txt

Exit: 0 pass, 1 coverage shortfall, 2 environment problem.
EOF
      exit 0
      ;;
    *)
      echo "unknown arg: $arg (try --help)" >&2
      exit 2
      ;;
  esac
done

# ─── locate repo root (script is always called from anywhere) ─────────────
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/../.." && pwd)
cd "$REPO_ROOT"

# ─── env defaults ─────────────────────────────────────────────────────────
THRESHOLD="${PHANTOM_V2_COVERAGE_THRESHOLD:-80}"
COVERAGE_JSON="${PHANTOM_V2_COVERAGE_JSON:-$REPO_ROOT/coverage.json}"
EXCEPTIONS_YAML="${PHANTOM_V2_EXCEPTIONS_YAML:-$REPO_ROOT/.github/workflows/ship-gate-coverage-exceptions.yaml}"
REPORT_OUT="${PHANTOM_V2_REPORT_OUT:-$REPO_ROOT/coverage-per-spec-report.txt}"

# ─── colour helpers (off in CI for clean logs) ────────────────────────────
if [[ "$CI_MODE" -eq 1 || -z "${TERM:-}" ]]; then
  C_RED=''; C_GRN=''; C_YEL=''; C_DIM=''; C_OFF=''
else
  C_RED=$'\033[31m'; C_GRN=$'\033[32m'; C_YEL=$'\033[33m'
  C_DIM=$'\033[2m';  C_OFF=$'\033[0m'
fi

# ─── tool checks ──────────────────────────────────────────────────────────
require_bin() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "${C_RED}ERROR${C_OFF}: required binary '$1' not on PATH." >&2
    echo "  install hint: $2" >&2
    exit 2
  fi
}

require_bin jq "brew install jq    (macOS) | apt install jq    (linux)"
require_bin yq "brew install yq    (macOS) | snap install yq   (linux)"

# Check exceptions YAML exists and parses.
if [[ ! -f "$EXCEPTIONS_YAML" ]]; then
  echo "${C_RED}ERROR${C_OFF}: exceptions YAML not found: $EXCEPTIONS_YAML" >&2
  exit 2
fi
if ! yq eval '.exceptions' "$EXCEPTIONS_YAML" >/dev/null 2>&1; then
  echo "${C_RED}ERROR${C_OFF}: exceptions YAML failed yq parse: $EXCEPTIONS_YAML" >&2
  exit 2
fi

# ─── step 1: run cargo llvm-cov (skipped in --ci mode) ────────────────────
if [[ "$CI_MODE" -eq 0 ]]; then
  require_bin cargo "install rustup from https://rustup.rs"
  if ! cargo llvm-cov --version >/dev/null 2>&1; then
    echo "${C_RED}ERROR${C_OFF}: cargo-llvm-cov not installed." >&2
    echo "  install: cargo install cargo-llvm-cov --locked" >&2
    echo "  (also requires:  rustup component add llvm-tools-preview)" >&2
    exit 2
  fi

  echo "${C_DIM}[v2-coverage] running cargo llvm-cov --workspace ...${C_OFF}"
  (cd core && cargo llvm-cov --workspace --summary-only)
  (cd core && cargo llvm-cov report --json --output-path "$COVERAGE_JSON")
  echo
fi

if [[ ! -f "$COVERAGE_JSON" ]]; then
  echo "${C_RED}ERROR${C_OFF}: coverage JSON not found: $COVERAGE_JSON" >&2
  echo "  (in --ci mode the workflow should have generated it before this step)" >&2
  exit 2
fi

# ─── step 2: per-spec module → wire-file mapping ──────────────────────────
# This is the source of truth for "which Rust file backs which leaf spec".
# Aligned with project memory `reference_phantom_wire_module_pattern.md`:
# 18 wire modules at core/src/<slug>_wire.rs. Add new specs here as they
# graduate to Stage 3+.
#
# Format: "SPEC-XX|core/src/<file>.rs" per line.
SPEC_MAP=$(cat <<'EOF'
SPEC-10|core/src/rpc_wire.rs
SPEC-11|core/src/mdns_wire.rs
SPEC-12|core/src/identity_wire.rs
SPEC-13|core/src/encryption_wire.rs
SPEC-14|core/src/providers_wire.rs
SPEC-15|core/src/broker_vault_wire.rs
SPEC-16|core/src/event_storage_wire.rs
SPEC-17|core/src/tauri_wire.rs
SPEC-20|core/src/capture_food_wire.rs
SPEC-21|core/src/capture_focus_wire.rs
SPEC-22|core/src/capture_habit_wire.rs
SPEC-23|core/src/coach_wire.rs
SPEC-24|core/src/coach_delivery_wire.rs
SPEC-25|core/src/skill_wire.rs
SPEC-26|core/src/cluster_dispatch_wire.rs
SPEC-27|core/src/smart_decompose_wire.rs
SPEC-28|core/src/onboarding_wire.rs
SPEC-29|core/src/release_pipeline_wire.rs
EOF
)

# ─── step 3: helper — look up line coverage for a path from coverage.json ─
# cargo-llvm-cov JSON shape: { "data": [ { "files": [ {filename, summary:
# { lines: { count, covered, percent }}}, ... ] } ] }
file_line_pct() {
  local target="$1"
  jq -r --arg t "$target" '
    .data[0].files[]
    | select(.filename | endswith($t))
    | .summary.lines.percent
  ' "$COVERAGE_JSON" 2>/dev/null | head -n1
}

# ─── step 4: helper — look up exception override + expiry for a glob ──────
# Returns: "OVERRIDE|EXPIRES_AT|REASON" if the path_glob matches, else empty.
# We compare the spec_id AND check that the path matches via shell glob.
exception_for() {
  local spec_id="$1"
  local path="$2"
  local count
  count=$(yq eval ".exceptions | length" "$EXCEPTIONS_YAML")
  if [[ -z "$count" || "$count" == "null" ]]; then return; fi
  local i=0
  while [[ $i -lt $count ]]; do
    local e_spec e_glob e_over e_exp e_reason
    e_spec=$(yq eval ".exceptions[$i].spec_id" "$EXCEPTIONS_YAML")
    e_glob=$(yq eval ".exceptions[$i].path_glob" "$EXCEPTIONS_YAML")
    e_over=$(yq eval ".exceptions[$i].coverage_override" "$EXCEPTIONS_YAML")
    e_exp=$(yq  eval ".exceptions[$i].expires_at" "$EXCEPTIONS_YAML")
    e_reason=$(yq eval ".exceptions[$i].reason" "$EXCEPTIONS_YAML")
    if [[ "$e_spec" == "$spec_id" ]]; then
      # Simple suffix glob match — exception path_glob is expected to be the
      # repo-relative path of the file (no `**` expansion needed for current
      # entries; if you add a true glob, extend this comparison).
      if [[ "$path" == "$e_glob" || "$path" == */"$e_glob" ]]; then
        echo "${e_over}|${e_exp}|${e_reason}"
        return
      fi
    fi
    i=$((i + 1))
  done
}

# ─── step 5: walk the spec map, decide pass/fail per row ──────────────────
today=$(date -u +%Y-%m-%d)
fail_count=0
warn_count=0
ok_count=0

# Tee report to both stdout and REPORT_OUT.
exec > >(tee "$REPORT_OUT") 2>&1

printf "%s\n" "─── SPEC-60 V2 ship-gate — per-spec line coverage report ───"
printf "%-9s %-46s %7s %9s %s\n" "spec" "file" "line%" "threshold" "verdict"
printf "%s\n" "──────────────────────────────────────────────────────────────────────────────────────"

while IFS='|' read -r spec_id file; do
  [[ -z "$spec_id" ]] && continue
  pct=$(file_line_pct "$file")
  if [[ -z "$pct" || "$pct" == "null" ]]; then
    printf "%-9s %-46s %7s %9s %s\n" \
      "$spec_id" "$file" "n/a" "${THRESHOLD}%" \
      "${C_YEL}warn: no coverage data${C_OFF}"
    warn_count=$((warn_count + 1))
    continue
  fi

  # Round down to integer for clean comparison.
  pct_int=${pct%.*}
  threshold_used=$THRESHOLD
  verdict=""

  if [[ $pct_int -ge $THRESHOLD ]]; then
    ok_count=$((ok_count + 1))
    verdict="${C_GRN}pass${C_OFF}"
  else
    # Below 80 — check exception list.
    excpt=$(exception_for "$spec_id" "$file")
    if [[ -n "$excpt" ]]; then
      e_over=${excpt%%|*}
      e_rest=${excpt#*|}
      e_exp=${e_rest%%|*}
      e_reason=${e_rest#*|}
      threshold_used=$e_over
      if [[ "$e_exp" < "$today" ]]; then
        verdict="${C_RED}fail: exception EXPIRED ($e_exp)${C_OFF}"
        fail_count=$((fail_count + 1))
      elif [[ $pct_int -ge $e_over ]]; then
        verdict="${C_YEL}pass (exception, expires $e_exp)${C_OFF}"
        warn_count=$((warn_count + 1))
      else
        verdict="${C_RED}fail: below exception override ${e_over}%${C_OFF}"
        fail_count=$((fail_count + 1))
      fi
    else
      verdict="${C_RED}fail${C_OFF}"
      fail_count=$((fail_count + 1))
    fi
  fi

  printf "%-9s %-46s %6s%% %8s%% %s\n" \
    "$spec_id" "$file" "$pct_int" "$threshold_used" "$verdict"
done <<< "$SPEC_MAP"

printf "%s\n" "──────────────────────────────────────────────────────────────────────────────────────"
printf "summary: %d pass, %d pass-with-exception/warn, %d fail   (threshold=%s%%)\n" \
  "$ok_count" "$warn_count" "$fail_count" "$THRESHOLD"

if [[ $fail_count -gt 0 ]]; then
  printf "%s\n" "${C_RED}V2 GATE: FAIL${C_OFF} — see docs/superpowers/v2-coverage-gate.md for next steps."
  exit 1
fi
printf "%s\n" "${C_GRN}V2 GATE: PASS${C_OFF}"
exit 0
