#!/usr/bin/env bash
# Bootstrap checks: spectyn binary is on PATH and basic CLI responds.
# If this feature fails, downstream features almost certainly will too.

selftest_feature_meta() {
  echo "name=binary"
  echo "priority=P0"
  echo "requires="
  echo "description=spectyn binary present, --version and --help respond"
  echo "hints=core/src/bin/spectyn.rs core/src/main.rs core/Cargo.toml"
}

selftest_run() {
  if [ -x "$SPECTYN" ]; then
    T_REPRO="ls -la $SPECTYN"
    t_pass "spectyn executable" "$SPECTYN"
  else
    T_REPRO="ls -la $SPECTYN"
    t_fail "spectyn executable" "not found at $SPECTYN (set SPECTYN_BIN)"
    return
  fi

  P=$(printf '%q' "$SPECTYN")

  # t_check captures full stdout+stderr to an artifact and records the shell
  # command as repro, so a failure tells the agent BOTH the full output AND
  # how to re-run just this check in isolation.
  t_check "spectyn --version" \
    "$P --version | grep -qE '^spectyn [0-9]+\.[0-9]+'"

  t_check "version provenance" \
    "$P --version | grep -qE '\([0-9a-f]{7,}'"

  for sub in serve mcp repl doctor onboarding autoevolve evolve; do
    t_check "subcommand: $sub" \
      "$P --help 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -qE '^[[:space:]]*spectyn $sub( |\$)'"
  done
}
