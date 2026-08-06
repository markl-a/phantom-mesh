#!/usr/bin/env bash
# SPEC-60 P2-2 ship-gate evidence map health self-test.
#
# Runs the cheap, offline `spectyn test gate map --check` resolve-lint and asserts
# exit 0 — i.e. every non-manual check in appendix/ship-gate-map.toml resolves to
# real code/scripts (no fake-green rot). This surfaces "is the evidence map
# healthy?" as a first-class P1 selftest feature. No network, no heavy gate runs.

selftest_feature_meta() {
  echo "name=ship-gate-evidence"
  echo "priority=P1"
  echo "requires="
  echo "description=SPEC-60 ship-gate-map resolve-lint (spectyn test gate map --check)"
  echo "hints=docs/superpowers/specs/v060-deep-spec/appendix/ship-gate-map.toml core/src/test_report/mod.rs"
}

selftest_run() {
  local out rc
  out=$("$SPECTYN" test gate map --check 2>&1)
  rc=$?
  T_ARTIFACT_TEXT="$out"
  T_REPRO="$SPECTYN test gate map --check"
  if [ "$rc" -eq 0 ]; then
    t_pass "gate-map resolve-lint" "every non-manual check resolves (exit 0)"
  else
    t_fail "gate-map resolve-lint" "exit $rc — unresolved check(s) in ship-gate-map.toml: $out"
  fi
}
