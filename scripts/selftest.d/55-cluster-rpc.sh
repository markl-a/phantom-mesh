#!/usr/bin/env bash
# Cluster RPC integration test — spawns TWO phantom serve instances
# on localhost (different ports, different HOMEs, shared cluster_secret)
# and verifies they can see each other via the /rpc/* endpoints.
#
# What this proves end-to-end:
#   1. /rpc/peers returns the configured peer list + self
#   2. /rpc/ping responds with serverInfo + wire_version
#   3. HMAC auth: /rpc/task/assign rejects wrong cluster_secret with 401
#   4. HMAC auth: /rpc/task/assign accepts correct HMAC-SHA256(secret, body)
#   5. Wire-version negotiation: mismatched wire_version → clean error
#
# Without this test, the README's "Tailscale-based cluster" + "subagent
# with cross-machine dispatch" claims are unverified. With it, every
# CI run + selftest sweep confirms the cluster RPC actually works.

selftest_feature_meta() {
  echo "name=cluster-rpc"
  echo "priority=P1"
  echo "requires=python3"
  echo "description=2-node phantom serve cluster RPC + HMAC auth + wire-version negotiation"
  echo "hints=core/src/serve.rs core/src/mesh.rs"
}

selftest_requires() {
  t_have python3 || { echo "python3 missing — needed for HMAC hex computation" >&2; return 1; }
  t_have openssl || { echo "openssl missing — needed for HMAC verification" >&2; return 1; }
}

_pick_port() {
  python3 -c '
import socket, random
for _ in range(20):
    p = random.randint(17900, 17999)
    s = socket.socket()
    try:
        s.bind(("127.0.0.1", p))
        s.close()
        print(p); break
    except OSError:
        continue
'
}

# Write a minimal agents.toml with the given cluster setup.
_write_config() {
  local home_dir="$1" node_name="$2" port="$3" peer_url="$4" secret="$5"
  mkdir -p "$home_dir/.phantom-mesh"
  cat > "$home_dir/.phantom-mesh/agents.toml" <<EOF
[core]
host = "127.0.0.1"
port = $port

[providers.fake]
type    = "anthropic"
api_key = "sk-ant-test-fake-key-only"

[agent.master]
provider     = "fake"
instructions = "test"
tools        = ["shell"]

[cluster]
node_name      = "$node_name"
cluster_secret = "$secret"
peers = ["$peer_url"]
EOF
}

selftest_run() {
  local td_a td_b port_a port_b pid_a pid_b
  td_a=$(mktemp -d)
  td_b=$(mktemp -d)
  port_a=$(_pick_port)
  port_b=$(_pick_port)
  [ "$port_a" = "$port_b" ] && port_b=$(_pick_port)
  local secret="test-cluster-secret-only-for-selftest"

  _write_config "$td_a" "node-a" "$port_a" "http://127.0.0.1:$port_b" "$secret"
  _write_config "$td_b" "node-b" "$port_b" "http://127.0.0.1:$port_a" "$secret"

  # Spawn both serves
  HOME="$td_a" "$PHANTOM" serve --port "$port_a" > "$td_a/serve.log" 2>&1 &
  pid_a=$!
  HOME="$td_b" "$PHANTOM" serve --port "$port_b" > "$td_b/serve.log" 2>&1 &
  pid_b=$!
  trap "kill $pid_a $pid_b 2>/dev/null; rm -rf $td_a $td_b" RETURN

  # Wait for both /healthz to come up
  local both_up=0
  for _ in $(seq 1 30); do
    if curl -sf "http://127.0.0.1:$port_a/healthz" >/dev/null 2>&1 \
       && curl -sf "http://127.0.0.1:$port_b/healthz" >/dev/null 2>&1; then
      both_up=1; break
    fi
    sleep 0.2
  done
  T_ARTIFACT="$td_a/serve.log"
  T_REPRO="HOME=$td_a $PHANTOM serve --port $port_a  AND  HOME=$td_b $PHANTOM serve --port $port_b"
  if [ "$both_up" != "1" ]; then
    t_fail "2 phantom serves up + /healthz reachable" "didn't both come up after 6s"
    return
  fi
  t_pass "2 phantom serves up + /healthz reachable" "ports $port_a + $port_b"

  # ── 1. /rpc/peers includes the other node ──────────────────────────────
  local peers_a
  peers_a=$(curl -sf "http://127.0.0.1:$port_a/rpc/peers" \
    | python3 -c '
import json, sys
d = json.load(sys.stdin)
peers = d.get("peers", [])
print("|".join(p.get("url", "") for p in peers))
' 2>/dev/null)
  if echo "$peers_a" | grep -q "$port_b"; then
    t_pass "/rpc/peers on A lists B as configured peer" ""
  else
    t_fail "/rpc/peers on A lists B as configured peer" "got: $peers_a"
  fi

  # ── 2. /rpc/ping returns wire_version + serverInfo-ish payload ─────────
  local ping_a
  ping_a=$(curl -sf "http://127.0.0.1:$port_a/rpc/ping" \
    | python3 -c '
import json, sys
d = json.load(sys.stdin)
print("wire_version=" + str(d.get("wire_version", "?")))
print("has_node_name=" + str("node_name" in d or "node_name" in str(d)))
')
  if echo "$ping_a" | grep -q "wire_version=1"; then
    t_pass "/rpc/ping returns wire_version=1" ""
  else
    t_fail "/rpc/ping returns wire_version=1" "got: $ping_a"
  fi

  # ── 3. HMAC auth — wrong secret rejected ───────────────────────────────
  local body='{"agent":"master","prompt":"hi","wire_version":1}'
  local wrong_hmac
  wrong_hmac=$(echo -n "$body" | openssl dgst -sha256 -hmac "wrong-secret" -hex | cut -d' ' -f2)
  local rc_wrong
  rc_wrong=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "http://127.0.0.1:$port_a/rpc/task/assign" \
    -H "content-type: application/json" \
    -H "X-Cluster-Auth: $wrong_hmac" \
    -d "$body")
  if [ "$rc_wrong" = "401" ]; then
    t_pass "HMAC: wrong secret → 401 Unauthorized" ""
  else
    t_fail "HMAC: wrong secret → 401 Unauthorized" "got HTTP $rc_wrong"
  fi

  # ── 4. HMAC auth — correct secret accepted (not 401) ──────────────────
  local right_hmac
  right_hmac=$(echo -n "$body" | openssl dgst -sha256 -hmac "$secret" -hex | cut -d' ' -f2)
  local rc_right
  rc_right=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "http://127.0.0.1:$port_a/rpc/task/assign" \
    -H "content-type: application/json" \
    -H "X-Cluster-Auth: $right_hmac" \
    -d "$body")
  # Any 2xx OR 5xx (LLM error with fake key) is fine — what we're verifying
  # is the HMAC stage passes. The body is BAD_REQUEST is also OK because it
  # parsed past auth. We just don't want 401.
  if [ "$rc_right" != "401" ]; then
    t_pass "HMAC: correct secret bypasses auth (HTTP $rc_right ≠ 401)" ""
  else
    t_fail "HMAC: correct secret bypasses auth" "still got 401 — secret mismatch?"
  fi

  # ── 5. Wire-version mismatch surfaces clearly ──────────────────────────
  local bad_body='{"agent":"master","prompt":"hi","wire_version":99999}'
  local bad_hmac
  bad_hmac=$(echo -n "$bad_body" | openssl dgst -sha256 -hmac "$secret" -hex | cut -d' ' -f2)
  local rc_bad_ver
  rc_bad_ver=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "http://127.0.0.1:$port_a/rpc/task/assign" \
    -H "content-type: application/json" \
    -H "X-Cluster-Auth: $bad_hmac" \
    -d "$bad_body")
  # Accept any 4xx — implementation may use 426 (Upgrade Required), 400,
  # 412, or a JSON error body with 200. As long as it's not 200 silently.
  if [ "${rc_bad_ver:0:1}" = "4" ] || [ "$rc_bad_ver" = "200" ]; then
    # If 200, peek the body to confirm it's an error envelope.
    if [ "$rc_bad_ver" = "200" ]; then
      curl -sf -X POST "http://127.0.0.1:$port_a/rpc/task/assign" \
           -H "content-type: application/json" \
           -H "X-Cluster-Auth: $bad_hmac" \
           -d "$bad_body" \
        | grep -qi "error\|wire\|version" \
        && t_pass "wire-version mismatch surfaces" "200 with error body" \
        || t_fail "wire-version mismatch surfaces" "200 with no error indicator"
    else
      t_pass "wire-version mismatch surfaces" "HTTP $rc_bad_ver"
    fi
  else
    t_fail "wire-version mismatch surfaces" "got HTTP $rc_bad_ver (expected 4xx or error JSON)"
  fi

  # Cleanup happens via the RETURN trap.
}
