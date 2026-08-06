#!/usr/bin/env bash
# Validate `spectyn doctor --json` emits a parseable object with
# the documented top-level schema. Schema drift here breaks CI gates,
# dashboard health probes, and monitoring scrapers downstream.

selftest_feature_meta() {
  echo "name=doctor-json"
  echo "priority=P1"
  echo "requires=python3"
  echo "description=spectyn doctor --json schema regression gate"
  echo "hints=core/src/bin/spectyn.rs run_doctor_json"
}

selftest_requires() {
  t_have python3 || { echo "python3 missing — needed for JSON checks" >&2; return 1; }
}

selftest_run() {
  local out="$SELFTEST_ARTIFACTS/doctor.json"
  T_ARTIFACT="$out"
  T_REPRO="$SPECTYN doctor --json"

  "$SPECTYN" doctor --json > "$out" 2>/dev/null

  # 1. Output is valid JSON.
  if python3 -c 'import json,sys; json.load(open(sys.argv[1])); print("OK")' "$out" 2>/dev/null | grep -q "^OK$"; then
    t_pass "doctor --json emits valid JSON" ""
  else
    t_fail "doctor --json emits valid JSON" "parse failed"
    return
  fi

  # 2. Has the canonical top-level keys.
  if python3 -c '
import json, sys
d = json.load(open(sys.argv[1]))
required = ["version", "git", "os", "arch", "config", "permissions",
            "providers", "serve", "autoevolve", "tailscale", "tools",
            "identity", "status"]
missing = [k for k in required if k not in d]
assert not missing, "missing keys: " + str(missing)
print("OK")
' "$out" 2>/dev/null | grep -q "^OK$"; then
    t_pass "all 13 canonical top-level keys present" ""
  else
    t_fail "all 13 canonical top-level keys present" "schema drift"
  fi

  # 3. status is one of the documented enum values.
  if python3 -c '
import json, sys
d = json.load(open(sys.argv[1]))
s = d.get("status")
assert s in ("ok", "warn", "fail"), "unexpected status: " + str(s)
print("OK")
' "$out" 2>/dev/null | grep -q "^OK$"; then
    t_pass "status field is ok/warn/fail" ""
  else
    t_fail "status field is ok/warn/fail" "wrong enum"
  fi

  # 4. providers is an array with 7 entries (the canonical providers).
  if python3 -c '
import json, sys
d = json.load(open(sys.argv[1]))
assert isinstance(d["providers"], list), "providers not a list"
assert len(d["providers"]) == 7, "expected 7 providers, got " + str(len(d["providers"]))
for p in d["providers"]:
    assert "name" in p and "available" in p, "provider entry missing keys"
print("OK")
' "$out" 2>/dev/null | grep -q "^OK$"; then
    t_pass "providers list has 7 entries with name+available" ""
  else
    t_fail "providers list has 7 entries with name+available" "shape wrong"
  fi

  # 5. autoevolve.queue_pending is a number.
  if python3 -c '
import json, sys
d = json.load(open(sys.argv[1]))
qp = d["autoevolve"]["queue_pending"]
assert isinstance(qp, int) and qp >= 0, "queue_pending bad: " + str(qp)
print("OK")
' "$out" 2>/dev/null | grep -q "^OK$"; then
    t_pass "autoevolve.queue_pending is non-negative int" ""
  else
    t_fail "autoevolve.queue_pending is non-negative int" "type wrong"
  fi
}
