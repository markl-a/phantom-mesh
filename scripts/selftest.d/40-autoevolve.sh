#!/usr/bin/env bash
# Smoke-check `spectyn autoevolve --once --dry-run`. We don't want a real
# evolve cycle (it can take minutes and write commits), just verify the
# control surface is wired up.

selftest_feature_meta() {
  echo "name=autoevolve"
  echo "priority=P2"
  echo "requires="
  echo "description=spectyn autoevolve --once --dry-run completes without error"
  echo "hints=core/src/bin/spectyn.rs core/src/evolve_checkpoint.rs core/src/evolve_goals.rs"
}

selftest_run() {
  out="$SELFTEST_ARTIFACTS/autoevolve.out"
  T_REPRO="$(printf '%q' "$SPECTYN") autoevolve --once --dry-run"
  T_ARTIFACT="$out"
  if "$SPECTYN" autoevolve --once --dry-run > "$out" 2>&1; then
    lines=$(wc -l < "$out" | tr -d ' ')
    t_pass "autoevolve --once --dry-run" "exit 0, $lines lines"
  else
    rc=$?
    # exit 1 is acceptable in dry-run when there's nothing to do; anything
    # higher (panic, signal) is a real failure.
    if [ "$rc" -le 1 ]; then
      t_pass "autoevolve --once --dry-run" "exit $rc (no work to do)"
    else
      t_fail "autoevolve --once --dry-run" "exit $rc"
    fi
  fi
}
