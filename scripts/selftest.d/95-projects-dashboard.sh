#!/usr/bin/env bash
# /projects dashboard self-test — boots spectyn serve on a free port,
# probes all 4 dashboard endpoints, verifies their shapes, kills the
# process. Catches regressions in:
#   - /projects               (HTML page)
#   - /api/projects           (6-entry JSON registry)
#   - /api/projects/<id>/run  (subprocess dispatcher)
#   - /api/activity           (autoevolve + subagent feed)
#
# Self-contained — uses curl + python3 + a free port. No tmux, no
# manual setup. Runtime: ~5 s including spectyn serve startup.

selftest_feature_meta() {
  echo "name=projects-dashboard"
  echo "priority=P1"
  echo "requires=spectyn-serve"
  echo "description=/projects dashboard endpoints (HTML, JSON, run, activity)"
  echo "hints=core/src/serve.rs core/src/projects.rs core/web/projects.html"
}

selftest_requires() {
  t_have curl    || { echo "curl missing" >&2; return 1; }
  t_have python3 || { echo "python3 missing — needed for JSON shape checks" >&2; return 1; }
}

# Pick a port unlikely to clash with real services. Range 17800-17999.
_pick_port() {
  python3 -c '
import socket, random
for _ in range(20):
    p = random.randint(17800, 17999)
    s = socket.socket()
    try:
        s.bind(("127.0.0.1", p))
        s.close()
        print(p)
        break
    except OSError:
        continue
'
}

selftest_run() {
  local port pid log out td
  port=$(_pick_port)
  if [ -z "$port" ]; then
    t_fail "find free port" "no free port in 17800-17999"
    return
  fi

  td=$(mktemp -d)
  log="$td/serve.log"
  T_ARTIFACT="$log"

  # Start spectyn serve in the background.
  HOME="$td" "$SPECTYN" serve --port "$port" > "$log" 2>&1 &
  pid=$!
  T_REPRO="HOME=$td $SPECTYN serve --port $port (pid would be $pid)"
  trap "kill $pid 2>/dev/null; rm -rf $td" RETURN

  # Wait for /healthz to come up (max 5s).
  local ready=0
  for _ in $(seq 1 25); do
    if curl -sf "http://127.0.0.1:$port/healthz" >/dev/null 2>&1; then
      ready=1
      break
    fi
    sleep 0.2
  done
  if [ "$ready" != "1" ]; then
    t_fail "spectyn serve started + /healthz reachable" "no response after 5s — see $log"
    return
  fi
  t_pass "spectyn serve started + /healthz reachable" "port $port"

  # ── 1. /projects HTML page ──────────────────────────────────────────────
  out=$(curl -s "http://127.0.0.1:$port/projects")
  if echo "$out" | grep -qE '<title>.*spectyn-mesh.*projects</title>'; then
    t_pass "/projects → HTML page renders" ""
  else
    t_fail "/projects → HTML page renders" "no recognizable title"
  fi
  if echo "$out" | grep -q "loadProjects\|loadCluster\|loadActivity"; then
    t_pass "/projects HTML embeds dashboard JS" ""
  else
    t_fail "/projects HTML embeds dashboard JS" "missing one of loadProjects/loadCluster/loadActivity"
  fi

  # ── 2. /api/projects JSON ───────────────────────────────────────────────
  out=$(curl -s "http://127.0.0.1:$port/api/projects")
  local n
  n=$(echo "$out" | python3 -c '
import json, sys
arr = json.loads(sys.stdin.read())
assert isinstance(arr, list), "not a list"
print(len(arr))
' 2>/dev/null || echo "")
  if [ "$n" = "6" ]; then
    t_pass "/api/projects → exactly 6 entries" ""
  else
    t_fail "/api/projects → exactly 6 entries" "got '$n', expected 6"
  fi
  # Every entry has the canonical shape.
  if echo "$out" | python3 -c '
import json, sys
arr = json.loads(sys.stdin.read())
for e in arr:
    for k in ("id", "name", "tagline", "repo_url", "status", "stack"):
        assert k in e, "missing " + k + " in " + str(e.get("id", "unknown"))
print("OK")
' 2>/dev/null | grep -q "^OK$"; then
    t_pass "/api/projects → each entry has canonical fields" ""
  else
    t_fail "/api/projects → each entry has canonical fields" "schema drift"
  fi

  # ── 3. /api/projects/<id>/run for a known-cheap demo ────────────────────
  # spectyn-mesh's demo command is `spectyn autoevolve --once --no-commit --max-rounds 1`
  # which on a clean repo just emits "cargo check green" in <2s. Cheapest demo
  # we have. We accept either ok=true OR ok=false (cwd missing on CI is fine
  # — what matters is the endpoint responds with valid JSON).
  out=$(curl -s -X POST "http://127.0.0.1:$port/api/projects/spectyn-mesh/run" \
        --max-time 10 || echo "{}")
  if echo "$out" | python3 -c '
import json, sys
d = json.loads(sys.stdin.read())
assert "ok" in d, "missing ok field"
print("OK")
' 2>/dev/null | grep -q "^OK$"; then
    t_pass "/api/projects/<id>/run → returns {ok:bool, ...}" ""
  else
    t_fail "/api/projects/<id>/run → returns {ok:bool, ...}" "no ok field"
  fi

  # Unknown id → ok:false with error
  out=$(curl -s -X POST "http://127.0.0.1:$port/api/projects/this-does-not-exist/run")
  if echo "$out" | python3 -c '
import json, sys
d = json.loads(sys.stdin.read())
assert d.get("ok") == False, "expected ok:false"
assert "error" in d or "unknown" in str(d).lower()
print("OK")
' 2>/dev/null | grep -q "^OK$"; then
    t_pass "/api/projects/<unknown>/run → clean error" ""
  else
    t_fail "/api/projects/<unknown>/run → clean error" "unexpected shape"
  fi

  # ── 4. /api/projects/<id>/run-stream → SSE format ──────────────────────
  # We grab the first chunk and check it starts with SSE event syntax.
  # The selftest HOME is a tempdir so the demo cwd won't exist — the
  # supervisor emits a single `event: done` with an error payload.
  # That's fine: either `event: line` (real demo) OR `event: done`
  # (error or fast finish) proves the SSE protocol shape is correct.
  out=$(curl -s -N --max-time 8 "http://127.0.0.1:$port/api/projects/spectyn-mesh/run-stream" 2>&1 | head -10)
  if echo "$out" | grep -qE "^event: (line|done)"; then
    t_pass "/api/projects/<id>/run-stream → SSE events" ""
  else
    t_fail "/api/projects/<id>/run-stream → SSE events" "no SSE event lines in first 10 lines"
  fi

  # ── 5. /api/activity feed ──────────────────────────────────────────────
  out=$(curl -s "http://127.0.0.1:$port/api/activity")
  if echo "$out" | python3 -c '
import json, sys
d = json.loads(sys.stdin.read())
assert "items" in d, "missing items"
assert isinstance(d["items"], list), "items not a list"
# Every entry should have at least kind + status fields if non-empty
for it in d["items"]:
    assert "kind" in it, f"missing kind in {it}"
    assert "status" in it, f"missing status in {it}"
print("OK")
' 2>/dev/null | grep -q "^OK$"; then
    t_pass "/api/activity → {items:[…]} with valid entries" ""
  else
    t_fail "/api/activity → {items:[…]} with valid entries" "schema wrong"
  fi
}
