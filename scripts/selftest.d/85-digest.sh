#!/usr/bin/env bash
# Validate `spectyn autoevolve digest` subcommand against a synthetic
# autoevolve.log. Asserts:
#   - runs_total counts entries within the time window
#   - by_status buckets are sane
#   - --since-hours filtering works (entries outside window excluded)
#   - --json output parses

selftest_feature_meta() {
  echo "name=digest"
  echo "priority=P2"
  echo "requires=python3"
  echo "description=spectyn autoevolve digest --json against a synthetic JSONL log"
  echo "hints=core/src/bin/spectyn.rs autoevolve_digest"
}

selftest_requires() {
  t_have python3 || { echo "python3 missing — needed to validate JSON shape" >&2; return 1; }
}

# Build a temp HOME with a synthetic ~/.spectyn-mesh/autoevolve.log.
# Entries timestamped in ms-from-epoch. Mix of statuses across the
# last 12 hours.
_digest_setup_home() {
  local td; td=$(mktemp -d)
  mkdir -p "$td/.spectyn-mesh"
  local now_ms
  now_ms=$(python3 -c 'import time; print(int(time.time()*1000))')
  # 5 entries: 3 within last 24h, 2 older. Statuses: 2 green, 1 fixed,
  # 1 queued-task-done, 1 queued-task-failed.
  python3 - "$td" "$now_ms" <<'PY'
import sys, json, os
td = sys.argv[1]
now = int(sys.argv[2])
entries = [
    {"started_at_ms": now - 3600_000,   "target":"check","status":"green",              "rounds":0,"elapsed_secs":0.5,"commit":None,                 "summary":"no-op"},
    {"started_at_ms": now - 7200_000,   "target":"check","status":"fixed",              "rounds":3,"elapsed_secs":42.1,"commit":"abc123def0",         "summary":"fixed cargo error"},
    {"started_at_ms": now - 10800_000,  "target":"check","status":"queued-task-done",   "rounds":0,"elapsed_secs":15.2,"commit":"deadbeef00",         "summary":"queued: refactor"},
    {"started_at_ms": now - 14400_000,  "target":"check","status":"queued-task-failed", "rounds":0,"elapsed_secs":3.0, "commit":None,                 "summary":"task broke build"},
    {"started_at_ms": now - 1000*3600*36,"target":"check","status":"green",             "rounds":0,"elapsed_secs":0.4,"commit":None,                 "summary":"old entry beyond 24h"},
]
with open(os.path.join(td, ".spectyn-mesh", "autoevolve.log"), "w") as f:
    for e in entries:
        f.write(json.dumps(e) + "\n")
PY
  echo "$td"
}

selftest_run() {
  local td out
  td=$(_digest_setup_home)

  out="$SELFTEST_ARTIFACTS/digest-24h.json"
  HOME="$td" "$SPECTYN" autoevolve digest --since-hours 24 --json > "$out" 2>&1
  T_ARTIFACT="$out"
  T_REPRO="HOME=$td $SPECTYN autoevolve digest --since-hours 24 --json"

  # 1. JSON parses + has expected top-level keys.
  if python3 -c '
import json, sys
d = json.load(open(sys.argv[1]))
assert "since_hours" in d, "missing since_hours"
assert "runs_total" in d, "missing runs_total"
assert "by_status" in d, "missing by_status"
assert "commits" in d, "missing commits"
assert "queue_pending" in d, "missing queue_pending"
print("OK")
' "$out" 2>/dev/null | grep -q "^OK$"; then
    t_pass "digest --json parses with required keys" ""
  else
    t_fail "digest --json parses with required keys" "JSON shape wrong"
  fi

  # 2. runs_total == 4 (5 synthesized entries, 1 outside 24h window)
  local runs_total
  runs_total=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("runs_total"))' "$out" 2>/dev/null)
  T_REPRO="python3 -c 'import json; print(json.load(open(\"$out\"))[\"runs_total\"])'"
  if [ "$runs_total" = "4" ]; then
    t_pass "runs_total respects --since-hours window" "got 4 (expected 4: 5 entries minus 1 older than 24h)"
  else
    t_fail "runs_total respects --since-hours window" "got '$runs_total', expected 4"
  fi

  # 3. by_status has expected buckets
  if python3 -c '
import json, sys
d = json.load(open(sys.argv[1]))
bs = d.get("by_status", {})
assert bs.get("green", 0) >= 1, f"green count low: {bs}"
assert bs.get("fixed", 0) == 1, f"fixed count wrong: {bs}"
assert bs.get("queued-task-done", 0) == 1, f"queued-task-done wrong: {bs}"
assert bs.get("queued-task-failed", 0) == 1, f"queued-task-failed wrong: {bs}"
print("OK")
' "$out" 2>/dev/null | grep -q "^OK$"; then
    t_pass "by_status buckets all 4 inside-window statuses" ""
  else
    t_fail "by_status buckets all 4 inside-window statuses" "see artifact"
  fi

  # 4. commits array picks up entries with commit field set
  local commits
  commits=$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1])).get("commits", [])))' "$out" 2>/dev/null)
  if [ "$commits" -ge 2 ]; then
    t_pass "commits array populated from log" "got $commits commit entries"
  else
    t_fail "commits array populated from log" "got $commits, expected ≥ 2"
  fi

  # 5. Wider window (--since-hours 72) catches the older entry too
  out2="$SELFTEST_ARTIFACTS/digest-72h.json"
  HOME="$td" "$SPECTYN" autoevolve digest --since-hours 72 --json > "$out2" 2>&1
  local runs_72
  runs_72=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("runs_total"))' "$out2" 2>/dev/null)
  T_ARTIFACT="$out2"
  if [ "$runs_72" = "5" ]; then
    t_pass "wider window includes older entry" "got 5 (all entries)"
  else
    t_fail "wider window includes older entry" "got '$runs_72', expected 5"
  fi

  rm -rf "$td"
}
