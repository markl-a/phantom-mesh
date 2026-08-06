#!/usr/bin/env bash
# spectyn-mesh self-test orchestrator.
#
# Runs every feature test file under scripts/selftest.d/ and emits both a
# human-readable table and a machine-readable JSON report. Designed so an
# LLM agent (Claude Code, spectyn itself, etc.) can ingest results
# unattended, and so each new feature is a single drop-in file.
#
# Usage:
#   scripts/selftest.sh                       # all features, text output
#   scripts/selftest.sh --json                # JSON only on stdout
#   scripts/selftest.sh --json --out a.json   # JSON to file, summary on stdout
#   scripts/selftest.sh --feature serve       # just one feature
#   scripts/selftest.sh --p0-only             # skip P1 / P2
#   scripts/selftest.sh --list                # list registered features
#
# Env:
#   SPECTYN_BIN=...   path to spectyn (default: $(command -v spectyn))
#   COORD=...         daemon URL (default: http://127.0.0.1:7878)
#
# Exit codes:
#   0  no P0 failures
#   1  one or more P0 failures
#   2  orchestrator usage / setup error

set -o pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="$ROOT_DIR/scripts/selftest.d"
LIB="$DIR/_lib.sh"

[ -d "$DIR" ] || { echo "selftest.d/ missing at $DIR" >&2; exit 2; }
[ -f "$LIB" ] || { echo "selftest.d/_lib.sh missing" >&2; exit 2; }

# ── args ──────────────────────────────────────────────────────────────────────
JSON_OUT=""        # path; empty = no file
JSON_ONLY=0        # 1 = print JSON to stdout, suppress text table
FEATURE_FILTER=""
P0_ONLY=0
LIST_ONLY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --json)        JSON_ONLY=1 ;;
    --out)         shift; JSON_OUT="$1" ;;
    --out=*)       JSON_OUT="${1#--out=}" ;;
    --feature)     shift; FEATURE_FILTER="$1" ;;
    --feature=*)   FEATURE_FILTER="${1#--feature=}" ;;
    --p0-only)     P0_ONLY=1 ;;
    --list)        LIST_ONLY=1 ;;
    -h|--help)     sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
  shift
done

# Locate the spectyn binary. On Windows the binary is `spectyn.exe`, and
# Git Bash's `command -v spectyn` may or may not strip the suffix depending
# on PATHEXT — try both spellings before giving up.
if [ -n "${SPECTYN_BIN:-}" ]; then
  SPECTYN="$SPECTYN_BIN"
elif command -v spectyn >/dev/null 2>&1; then
  SPECTYN="$(command -v spectyn)"
elif command -v spectyn.exe >/dev/null 2>&1; then
  SPECTYN="$(command -v spectyn.exe)"
elif [ -x "$HOME/.cargo/bin/spectyn.exe" ]; then
  SPECTYN="$HOME/.cargo/bin/spectyn.exe"
else
  SPECTYN="$HOME/.cargo/bin/spectyn"
fi
export SPECTYN
export COORD="${COORD:-http://127.0.0.1:7878}"
export TMP="$(mktemp -d)"

# Artifacts (per-test stdout/stderr) live under test-results/selftest-<ts>/
# and are KEPT after the run so an agent can read full failure context. Old
# runs are pruned (keep last 10) so this doesn't grow unbounded.
RUN_TS="$(date -u +%Y%m%dT%H%M%SZ)"
RUN_DIR="$ROOT_DIR/test-results/selftest-$RUN_TS"
mkdir -p "$RUN_DIR"
export SELFTEST_LOG="$RUN_DIR/selftest.log"
: > "$SELFTEST_LOG"
ls -1dt "$ROOT_DIR/test-results/selftest-"* 2>/dev/null | tail -n +11 | xargs -I{} rm -rf {} 2>/dev/null || true
trap 'rm -rf "$TMP"' EXIT

# Pretty printers go to FD 3, which we route to either stdout or /dev/null
# depending on --json. Test files use t_pass/t_fail/t_skip from _lib.sh, which
# print to stdout — but we redirect the per-feature execution block below.
say() { printf '%s\n' "$*"; }

# Source the library so helpers exist in this shell too (we don't actually use
# them here, but feature tests sourced into this shell need them).
# shellcheck source=scripts/selftest.d/_lib.sh
. "$LIB"

# ── feature discovery ────────────────────────────────────────────────────────
# Bash 3.2 (macOS default) has no `mapfile`; build the array portably.
FEATURE_FILES=()
while IFS= read -r _line; do
  FEATURE_FILES+=("$_line")
done < <(find "$DIR" -maxdepth 1 -name '[0-9]*.sh' -type f | sort)

if [ "$LIST_ONLY" = 1 ]; then
  printf "%-6s %-20s %-12s %s\n" "FILE" "FEATURE" "PRIORITY" "DESCRIPTION"
  for f in "${FEATURE_FILES[@]}"; do
    (
      # shellcheck disable=SC1090
      . "$LIB"; . "$f"
      meta="$(selftest_feature_meta)"
      n="$(echo "$meta" | sed -n 's/^name=//p')"
      p="$(echo "$meta" | sed -n 's/^priority=//p')"
      d="$(echo "$meta" | sed -n 's/^description=//p')"
      printf "%-6s %-20s %-12s %s\n" "$(basename "$f" .sh | cut -c1-2)" "$n" "$p" "$d"
    )
  done
  exit 0
fi

# ── header ───────────────────────────────────────────────────────────────────
SPECTYN_VER="(missing)"
if [ -x "$SPECTYN" ]; then
  SPECTYN_VER="$("$SPECTYN" --version 2>&1 | head -1)"
fi
START_EPOCH="$(date +%s)"
START_ISO="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

if [ "$JSON_ONLY" = 0 ]; then
  printf '\033[36mspectyn self-test\033[0m  %s\n' "$START_ISO"
  printf '  binary  : %s\n' "$SPECTYN"
  printf '  version : %s\n' "$SPECTYN_VER"
  printf '  coord   : %s\n' "$COORD"
fi

# ── run features ─────────────────────────────────────────────────────────────
TOTAL_PASS=0; TOTAL_FAIL=0; TOTAL_SKIP=0
P0_FAILS=0
SKIPPED_FEATURES=()

for f in "${FEATURE_FILES[@]}"; do
  # Run each feature in a subshell so its functions don't leak between files.
  (
    # shellcheck disable=SC1090
    . "$LIB"; . "$f"

    meta="$(selftest_feature_meta)"
    name="$(echo "$meta" | sed -n 's/^name=//p')"
    pri="$( echo "$meta" | sed -n 's/^priority=//p')"
    desc="$(echo "$meta" | sed -n 's/^description=//p')"

    # Filter by --feature.
    if [ -n "$FEATURE_FILTER" ] && [ "$FEATURE_FILTER" != "$name" ]; then
      exit 0
    fi
    # Filter by --p0-only.
    if [ "$P0_ONLY" = 1 ] && [ "$pri" != "P0" ]; then
      exit 0
    fi

    export SELFTEST_FEATURE="$name"
    export SELFTEST_ARTIFACTS="$RUN_DIR/$name"
    mkdir -p "$SELFTEST_ARTIFACTS"

    if [ "$JSON_ONLY" = 0 ]; then
      printf '\n\033[35m── %s \033[90m(%s)\033[0m  %s\n' "$name" "$pri" "$desc"
    fi

    # Optional gate.
    if declare -F selftest_requires >/dev/null 2>&1; then
      reason="$(selftest_requires 2>&1 1>/dev/null)" || {
        if [ "$JSON_ONLY" = 0 ]; then
          printf '  \033[33m○\033[0m feature skipped — %s\n' "$reason"
        fi
        # Record a single SKIP row so JSON reports something.
        printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "skip" "feature precondition" "$reason" "" "" >> "$SELFTEST_LOG"
        exit 0
      }
    fi

    if [ "$JSON_ONLY" = 1 ]; then
      selftest_run >/dev/null 2>&1
    else
      selftest_run
    fi
  )
done

END_EPOCH="$(date +%s)"
DUR=$((END_EPOCH - START_EPOCH))

# ── tally ────────────────────────────────────────────────────────────────────
while IFS=$'\t' read -r feat st _name _detail _repro _artifact; do
  case "$st" in
    pass) TOTAL_PASS=$((TOTAL_PASS+1)) ;;
    fail) TOTAL_FAIL=$((TOTAL_FAIL+1))
          # P0-fail check needs the feature's priority — re-discover.
          for f in "${FEATURE_FILES[@]}"; do
            (
              # shellcheck disable=SC1090
              . "$LIB"; . "$f"
              meta="$(selftest_feature_meta)"
              n="$(echo "$meta" | sed -n 's/^name=//p')"
              p="$(echo "$meta" | sed -n 's/^priority=//p')"
              [ "$n" = "$feat" ] && [ "$p" = "P0" ] && exit 7
              exit 0
            )
            rc=$?
            [ "$rc" = 7 ] && P0_FAILS=$((P0_FAILS+1)) && break
          done
          ;;
    skip) TOTAL_SKIP=$((TOTAL_SKIP+1)) ;;
  esac
done < "$SELFTEST_LOG"

# ── text summary ─────────────────────────────────────────────────────────────
if [ "$JSON_ONLY" = 0 ]; then
  printf '\n\033[1msummary\033[0m  %s pass, %s fail, %s skip  (%ds)\n' \
    "$TOTAL_PASS" "$TOTAL_FAIL" "$TOTAL_SKIP" "$DUR"
  if [ "$TOTAL_FAIL" -gt 0 ]; then
    printf '\n\033[31mfailures:\033[0m\n'
    awk -F'\t' '$2=="fail"{
      printf "  %-20s %s — %s\n", $1, $3, $4
      if ($6 != "") printf "    log:   %s\n", $6
      if ($5 != "") printf "    repro: %s\n", $5
    }' "$SELFTEST_LOG"
  fi
  printf '\n  artifacts dir : %s\n' "$RUN_DIR"
fi

# ── JSON report ──────────────────────────────────────────────────────────────
# Two builders. python3 is preferred when present (pretty-printed output);
# otherwise we fall back to a pure-bash builder so Windows users running under
# Git Bash (which doesn't ship python3) still get a valid report.

build_json() {
  if command -v python3 >/dev/null 2>&1; then
    build_json_python python3
  elif command -v python >/dev/null 2>&1; then
    build_json_python python
  else
    build_json_bash
  fi
}

build_json_python() {
  local py="$1"
  "$py" - "$SELFTEST_LOG" "$DIR" "$LIB" "$SPECTYN_VER" "$START_ISO" "$DUR" \
    "$TOTAL_PASS" "$TOTAL_FAIL" "$TOTAL_SKIP" "$P0_FAILS" "$RUN_DIR" <<'PY'
import json, os, subprocess, sys

(log_path, feat_dir, _lib, ver, started, dur,
 p, f, s, p0fails, run_dir) = sys.argv[1:]

feats = []
for fn in sorted(os.listdir(feat_dir)):
    if not (fn.endswith('.sh') and fn[0].isdigit()):
        continue
    path = os.path.join(feat_dir, fn)
    out = subprocess.run(
        ['bash', '-c', f'. "{_lib}"; . "{path}"; selftest_feature_meta'],
        capture_output=True, text=True
    ).stdout
    meta = {}
    for line in out.splitlines():
        if '=' in line:
            k, v = line.split('=', 1)
            meta[k.strip()] = v.strip()
    if meta.get('name'):
        meta['file'] = fn
        meta['hints'] = [h for h in meta.get('hints', '').split() if h]
        meta['tests'] = []
        feats.append(meta)

by_name = {m['name']: m for m in feats}

with open(log_path) as fh:
    for ln in fh:
        parts = ln.rstrip('\n').split('\t')
        while len(parts) < 6:
            parts.append('')
        feat, st, name, detail, repro, artifact = parts[:6]
        if feat in by_name:
            row = {'name': name, 'status': st, 'detail': detail}
            if repro:    row['repro']    = repro
            if artifact: row['artifact'] = artifact
            by_name[feat]['tests'].append(row)

report = {
    'spectyn_version': ver,
    'started_at': started,
    'duration_s': int(dur),
    'artifacts_dir': run_dir,
    'summary': {
        'pass': int(p), 'fail': int(f), 'skip': int(s),
        'p0_failures': int(p0fails),
    },
    'features': feats,
}
print(json.dumps(report, indent=2, ensure_ascii=False))
PY
}

# JSON-escape a string for use between double quotes. Handles \, ", and the
# common control chars; non-ASCII bytes pass through verbatim (UTF-8 valid
# JSON). Output has NO surrounding quotes.
_json_esc() {
  local s="$1"
  s=${s//\\/\\\\}
  s=${s//\"/\\\"}
  s=${s//$'\t'/\\t}
  s=${s//$'\n'/\\n}
  s=${s//$'\r'/\\r}
  printf '%s' "$s"
}

build_json_bash() {
  printf '{\n'
  printf '  "spectyn_version": "%s",\n' "$(_json_esc "$SPECTYN_VER")"
  printf '  "started_at": "%s",\n'      "$(_json_esc "$START_ISO")"
  printf '  "duration_s": %s,\n'        "$DUR"
  printf '  "artifacts_dir": "%s",\n'   "$(_json_esc "$RUN_DIR")"
  printf '  "summary": {"pass": %s, "fail": %s, "skip": %s, "p0_failures": %s},\n' \
    "$TOTAL_PASS" "$TOTAL_FAIL" "$TOTAL_SKIP" "$P0_FAILS"
  printf '  "features": [\n'

  local first_feat=1
  for fpath in "${FEATURE_FILES[@]}"; do
    # Pull meta in a subshell (consistent with how features are sourced).
    local meta name pri requires desc hints
    meta="$(. "$LIB"; . "$fpath"; selftest_feature_meta)"
    name="$(    echo "$meta" | sed -n 's/^name=//p')"
    pri="$(     echo "$meta" | sed -n 's/^priority=//p')"
    requires="$(echo "$meta" | sed -n 's/^requires=//p')"
    desc="$(    echo "$meta" | sed -n 's/^description=//p')"
    hints="$(   echo "$meta" | sed -n 's/^hints=//p')"
    [ -z "$name" ] && continue

    [ "$first_feat" = 1 ] || printf ',\n'
    first_feat=0

    printf '    {\n'
    printf '      "name": "%s",\n'        "$(_json_esc "$name")"
    printf '      "priority": "%s",\n'    "$(_json_esc "$pri")"
    printf '      "requires": "%s",\n'    "$(_json_esc "$requires")"
    printf '      "description": "%s",\n' "$(_json_esc "$desc")"
    printf '      "file": "%s",\n'        "$(_json_esc "$(basename "$fpath")")"

    # hints array
    printf '      "hints": ['
    if [ -n "$hints" ]; then
      local first_h=1 h
      for h in $hints; do
        [ "$first_h" = 1 ] || printf ', '
        first_h=0
        printf '"%s"' "$(_json_esc "$h")"
      done
    fi
    printf '],\n'

    # tests array — filter SELFTEST_LOG by feature.
    # Bash collapses consecutive tabs when IFS is whitespace-only, which
    # would lose empty `detail` fields. Translate tab → US (\x1f, non-
    # whitespace) so `read` preserves empty fields verbatim.
    printf '      "tests": ['
    local first_t=1
    while IFS=$'\037' read -r tfeat tst tname tdetail trepro tartifact; do
      [ "$tfeat" = "$name" ] || continue
      [ "$first_t" = 1 ] || printf ', '
      first_t=0
      printf '\n        {"name": "%s", "status": "%s", "detail": "%s"' \
        "$(_json_esc "$tname")" "$(_json_esc "$tst")" "$(_json_esc "$tdetail")"
      [ -n "$trepro"    ] && printf ', "repro": "%s"'    "$(_json_esc "$trepro")"
      [ -n "$tartifact" ] && printf ', "artifact": "%s"' "$(_json_esc "$tartifact")"
      printf '}'
    done < <(tr '\t' '\037' < "$SELFTEST_LOG")
    if [ "$first_t" = 0 ]; then
      printf '\n      ]\n'
    else
      printf ']\n'
    fi
    printf '    }'
  done

  printf '\n  ]\n}\n'
}

if [ -n "$JSON_OUT" ]; then
  build_json > "$JSON_OUT"
  if [ "$JSON_ONLY" = 0 ]; then
    printf '\n  json report → %s\n' "$JSON_OUT"
  fi
fi
if [ "$JSON_ONLY" = 1 ] && [ -z "$JSON_OUT" ]; then
  build_json
fi

# ── exit ─────────────────────────────────────────────────────────────────────
[ "$P0_FAILS" -gt 0 ] && exit 1
exit 0
