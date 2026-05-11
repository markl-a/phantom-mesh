#!/usr/bin/env bash
# Bootstrap checks: phantom binary is on PATH and basic CLI responds.
# If this feature fails, downstream features almost certainly will too.

selftest_feature_meta() {
  echo "name=binary"
  echo "priority=P0"
  echo "requires="
  echo "description=phantom binary present, --version and --help respond"
  echo "hints=core/src/bin/phantom.rs core/src/main.rs core/Cargo.toml"
}

selftest_run() {
  if [ -x "$PHANTOM" ]; then
    T_REPRO="ls -la $PHANTOM"
    t_pass "phantom executable" "$PHANTOM"
  else
    T_REPRO="ls -la $PHANTOM"
    t_fail "phantom executable" "not found at $PHANTOM (set PHANTOM_BIN)"
    return
  fi

  P=$(printf '%q' "$PHANTOM")

  # t_check captures full stdout+stderr to an artifact and records the shell
  # command as repro, so a failure tells the agent BOTH the full output AND
  # how to re-run just this check in isolation.
  t_check "phantom --version" \
    "$P --version | grep -qE '^phantom [0-9]+\.[0-9]+'"

  t_check "version provenance" \
    "$P --version | grep -qE '\([0-9a-f]{7,}'"

  for sub in serve mcp repl doctor onboarding autoevolve evolve; do
    t_check "subcommand: $sub" \
      "$P --help 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -qE '^[[:space:]]*phantom $sub( |\$)'"
  done
}
