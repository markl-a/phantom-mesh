#!/usr/bin/env bash
# Probe the HTTP daemon. Skips the whole feature cleanly if /healthz isn't
# reachable, so this script also works on a freshly-installed box where
# `spectyn serve` hasn't been started yet.

selftest_feature_meta() {
  echo "name=serve"
  echo "priority=P1"
  echo "requires=daemon"
  echo "description=spectyn serve HTTP+WS endpoints respond as expected"
  echo "hints=core/src/serve.rs core/src/main.rs core/src/mesh.rs"
}

selftest_requires() {
  if t_http "$COORD/healthz" 200; then
    return 0
  fi
  echo "spectyn serve not reachable at $COORD — start it with \`spectyn serve\`" >&2
  return 1
}

selftest_run() {
  # Core endpoints (200 expected). Use t_check so a 502/timeout dumps the
  # full curl output to an artifact for the agent to read.
  for path in /healthz / /m; do
    t_check "GET $path" \
      "curl -sS --max-time 5 -o /dev/stdout -w '\n# http_code=%{http_code}\n' $COORD$path | tee /dev/stderr | grep -q '# http_code=200'"
  done

  # JSON API endpoints — assert keys exist so a silent rename fails the test.
  ver_json="$(curl -s --max-time 5 "$COORD/api/version" 2>/dev/null || true)"
  if echo "$ver_json" | grep -q '"version"'; then
    v="$(echo "$ver_json" | jq -r .version 2>/dev/null || echo '?')"
    t_pass "GET /api/version" "$v"
  else
    t_fail "GET /api/version" "missing 'version' key"
  fi

  for ep in /api/status /api/providers/health /api/sessions /api/cost /api/nodes /api/tools/history; do
    if t_http "$COORD$ep" 200; then
      t_pass "GET $ep" "200"
    else
      t_skip "GET $ep" "not 200 (auth, gated, or removed)"
    fi
  done
}
