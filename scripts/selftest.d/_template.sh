#!/usr/bin/env bash
# Copy this file to scripts/selftest.d/<NN>-<feature-name>.sh when adding tests
# for a new feature. NN orders execution (00–09 = bootstrap, 10–29 = core CLI,
# 30–49 = network surfaces, 50–69 = integrations, 70+ = optional/expensive).
#
# Read scripts/selftest.d/_lib.sh for the full helper contract.

selftest_feature_meta() {
  echo "name=template"
  echo "priority=P2"        # P0 must pass for ship; P1 expected; P2 nice-to-have
  echo "requires="          # space-separated tags: daemon mcp tmux network mlx
  echo "description=copy this file when adding a new feature self-test"
}

# Optional. Return non-zero + a stderr reason to skip the whole feature.
# Use this for prereqs the orchestrator can't infer (e.g. a port being open,
# a model file present, a peer reachable).
# selftest_requires() {
#   t_have jq || { echo "jq not on PATH" >&2; return 1; }
# }

selftest_run() {
  # Replace the body with real checks. Use t_pass / t_fail / t_skip / t_run.
  t_pass "example check"  "always passes"
  t_run  "phantom --help" "$PHANTOM" --help
}
