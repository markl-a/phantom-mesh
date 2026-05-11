#!/usr/bin/env bash
# Run the workspace's cargo test suite. Heavy compared to other features
# (~30s on a warm cache, several minutes cold), so this is P2 — opt-out via
# `phantom selftest --p0-only` or `--feature <other>`.
#
# We don't gate on cargo availability via `requires=cargo` because cargo is
# de-facto required to ship phantom; if it's missing the run fails loudly,
# which is the right signal.

selftest_feature_meta() {
  echo "name=cargo-test"
  echo "priority=P2"
  echo "requires=cargo"
  echo "description=cargo test suite passes (workspace-wide)"
  echo "hints=core/tests core/src/lib.rs core/Cargo.toml"
}

selftest_requires() {
  t_have cargo || { echo "cargo not on PATH — install Rust toolchain" >&2; return 1; }
  # Just verify the workspace exists; selftest_run re-locates because state
  # set in this function lives in a different subshell.
  local cwd; cwd="$(pwd)"
  while [ "$cwd" != "/" ]; do
    [ -f "$cwd/core/Cargo.toml" ] && return 0
    cwd="$(dirname "$cwd")"
  done
  echo "could not find core/Cargo.toml from $(pwd) — run from the phantom-mesh repo" >&2
  return 1
}

# Walk up from cwd to find core/Cargo.toml.
_cargo_workspace() {
  local cwd; cwd="$(pwd)"
  while [ "$cwd" != "/" ]; do
    if [ -f "$cwd/core/Cargo.toml" ]; then
      echo "$cwd/core"
      return 0
    fi
    cwd="$(dirname "$cwd")"
  done
  return 1
}

selftest_run() {
  local cargo_dir; cargo_dir="$(_cargo_workspace)" || {
    t_fail "cargo test" "core/Cargo.toml not found"
    return
  }
  local out="$SELFTEST_ARTIFACTS/cargo-test.out"
  # `--test-threads=1` is paranoia about env-var races between unit tests
  # that mutate $HOME (see crate::env_lock). We've serialized the known
  # offenders, but the suite is fast enough serially that we'd rather
  # be deterministic than discover the next race in production.
  T_REPRO="cd $(printf '%q' "$cargo_dir") && cargo test -- --test-threads=1"
  T_ARTIFACT="$out"
  if (cd "$cargo_dir" && cargo test -- --test-threads=1) > "$out" 2>&1; then
    # Sum pass counts across all binaries — `test result: ok. N passed; ...`
    local total
    total=$(grep -E '^test result' "$out" | awk '{print $4}' | paste -sd+ - | bc 2>/dev/null || echo "?")
    t_pass "cargo test" "${total:-?} tests passed"
  else
    rc=$?
    local fails
    fails=$(grep -cE '^test .* FAILED|FAILED$' "$out" | head -1)
    t_fail "cargo test" "${fails:-?} test(s) failed (exit $rc)"
  fi
}
