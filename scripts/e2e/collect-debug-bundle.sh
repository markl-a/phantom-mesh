#!/usr/bin/env bash
# collect-debug-bundle.sh — G-DBG-4: tar up everything needed to debug a failed
# E2E run: the run log, screenshots, the isolated ~/.spectyn-mesh tree, a sqlite
# dump, and a KEY-MASKED agents.toml. Output: /tmp/spectyn-debug-<ts>.tar.gz.
#
# Usage: collect-debug-bundle.sh <run_home> <log_file> [shots_dir]
set -uo pipefail
RUN_HOME="${1:?run_home required}"
LOG="${2:-}"
SHOTS="${3:-}"
TS="$(date +%Y%m%d-%H%M%S)"
STAGE="$(mktemp -d "${TMPDIR:-/tmp}/spectyn-debug-stage-$TS.XXXXXX")"
OUT="${TMPDIR:-/tmp}/spectyn-debug-$TS.tar.gz"

mkdir -p "$STAGE/bundle"

# 1. run log
[ -n "$LOG" ] && [ -f "$LOG" ] && cp "$LOG" "$STAGE/bundle/run.log"

# 2. screenshots
[ -n "$SHOTS" ] && [ -d "$SHOTS" ] && cp -R "$SHOTS" "$STAGE/bundle/screenshots" 2>/dev/null || true

# 3. the ~/.spectyn-mesh tree (structure + small non-secret files), but NEVER
#    identity.key (the root secret) — copy a listing + safe files only.
pm="$RUN_HOME/.spectyn-mesh"
if [ -d "$pm" ]; then
  ( cd "$pm" && find . -type f | sort ) > "$STAGE/bundle/spectyn-mesh.filelist.txt" 2>/dev/null || true
  # sqlite dumps (schema + row counts; not full PII rows)
  for db in "$pm"/*.sqlite; do
    [ -f "$db" ] || continue
    name="$(basename "$db")"
    {
      echo "### $name — schema"; sqlite3 "$db" ".schema" 2>/dev/null
      echo; echo "### $name — table row counts"
      sqlite3 "$db" "SELECT name FROM sqlite_master WHERE type='table';" 2>/dev/null \
        | while IFS= read -r t; do printf '%s: ' "$t"; sqlite3 "$db" "SELECT count(*) FROM \"$t\";" 2>/dev/null; done
    } > "$STAGE/bundle/$name.dump.txt" 2>/dev/null || true
  done
  # agents.toml with API keys masked
  if [ -f "$pm/agents.toml" ]; then
    sed -E 's/(api_key[[:space:]]*=[[:space:]]*").*(")/\1***MASKED***\2/; s/(_API_KEY=).*/\1***MASKED***/' \
      "$pm/agents.toml" > "$STAGE/bundle/agents.toml.masked" 2>/dev/null || true
  fi
fi

# 4. env snapshot (spectyn-relevant, masked)
env | grep -iE '^SPECTYN_|_API_KEY=' | sed -E 's/(_API_KEY=).*/\1***MASKED***/' \
  > "$STAGE/bundle/env.masked.txt" 2>/dev/null || true

( cd "$STAGE" && tar czf "$OUT" bundle ) 2>/dev/null
rm -rf "$STAGE"
echo "📦 debug bundle: $OUT"
echo "   contents: run.log, screenshots/, *.dump.txt, agents.toml.masked, env.masked.txt (identity.key excluded)"
