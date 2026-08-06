#!/usr/bin/env bash
# `spectyn doctor` is the canonical health summary. We assert it runs cleanly
# and emits the sections we expect — so a regression that quietly drops a
# section (e.g. provider keys) fails the self-test loudly.

selftest_feature_meta() {
  echo "name=doctor"
  echo "priority=P0"
  echo "requires="
  echo "description=spectyn doctor exits 0 and contains all expected sections"
  echo "hints=core/src/bin/spectyn.rs core/src/diag.rs"
}

selftest_run() {
  P=$(printf '%q' "$SPECTYN")
  out="$SELFTEST_ARTIFACTS/doctor.out"
  T_REPRO="$P doctor"
  T_ARTIFACT="$out"
  if "$SPECTYN" doctor > "$out" 2>&1; then
    lines=$(wc -l < "$out" | tr -d ' ')
    t_pass "spectyn doctor runs" "$lines lines of output"
  else
    t_fail "spectyn doctor runs" "non-zero exit (see $out)"
    return
  fi

  # Stable structural markers in the current `spectyn doctor` line-label output
  # (the older "binary"/"provider keys"/"network"/"diagnostics" section HEADERS
  # were never emitted by this format — doctor uses per-line labels). Each marker
  # below is state-independent: version (always), Anthropic (always in the
  # known-provider list, set or not), healthz (serve reachability, up or down),
  # crash logs (always checked). A regression that drops one fails loudly.
  for sec in version config Anthropic "spectyn serve" healthz tools autoevolve identity "crash logs"; do
    if grep -qF "$sec" "$out"; then
      t_pass "section: $sec" ""
    else
      t_fail "section: $sec" "not present in doctor output"
    fi
  done

  # Platform-specific section: nice-to-have on the matching host.
  case "$(uname -s)" in
    Darwin)
      if grep -q "macOS integrations" "$out"; then
        t_pass "macOS integrations section" ""
      else
        t_skip "macOS integrations section" "not present (older build?)"
      fi
      ;;
    MINGW*|MSYS*|CYGWIN*)
      # Git Bash on Windows — spectyn doctor's Windows-integrations block
      # only runs when spectyn itself was built for windows-msvc/gnu.
      if grep -q "Windows integrations" "$out"; then
        t_pass "Windows integrations section" ""
      else
        t_skip "Windows integrations section" "not present (running a non-windows spectyn build?)"
      fi
      ;;
  esac

  red=$(grep -c '✗' "$out" || true)
  if [ "$red" -gt 0 ]; then
    t_fail "doctor red checks" "$red red ✗ lines — run \`spectyn doctor\` to see"
  else
    t_pass "doctor red checks" "none"
  fi
}
