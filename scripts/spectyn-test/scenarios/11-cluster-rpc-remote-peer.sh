#!/usr/bin/env bash
source "$SPECTYN_TEST_LIB/common.sh"
source "$SPECTYN_TEST_LIB/cluster-rpc.sh"

scenario "Cluster RPC — dispatch to a remote peer (not localhost)"

# Pick the target. Either explicit env var (preferred for CI / scripted runs)
# or auto-discover the first non-self online peer from `spectyn peer list`.
target="${SPECTYN_REMOTE_PEER:-}"

if [ -z "$target" ]; then
  step "SPECTYN_REMOTE_PEER unset — auto-discovering first online non-self peer"
  require_cmd "$SPECTYN_BIN"

  self_url="$(rpc_url)"
  raw=$("$SPECTYN_BIN" peer list 2>&1 | sed -E 's/\x1b\[[0-9;]*m//g')

  # Each peer line in `spectyn peer list` looks like:
  #   http://100.x.y.z:7878    name    online    0
  # Pick the first http URL that is online AND not our own self_url.
  target=$(echo "$raw" \
    | awk '/online/ && /^http/ {print $1}' \
    | grep -v -F "${self_url}" \
    | head -1)

  if [ -z "$target" ]; then
    warn "no online non-self peer in spectyn peer list — skipping"
    exit 77
  fi
  step "discovered: $target"
fi

# Override host/port for the RPC helpers so they POST to the remote peer.
SPECTYN_HOST=$(echo "$target" | sed -E 's|^https?://([^:/]+).*|\1|')
SPECTYN_PORT=$(echo "$target" | sed -E 's|^https?://[^:]+:([0-9]+).*|\1|')
[ -z "$SPECTYN_PORT" ] && SPECTYN_PORT=80

step "target host=$SPECTYN_HOST port=$SPECTYN_PORT"

# Cheap reachability check first.
ASSERT_HTTP "http://$SPECTYN_HOST:$SPECTYN_PORT/healthz" 200 "remote serve healthz"

step "dispatching agent.master to remote peer with deterministic prompt"
job=$(rpc_dispatch master "Reply with the single word: pong")

if [ -z "$job" ]; then
  fail "rpc_dispatch returned empty job_id (HMAC mismatch or remote rejected)"
  exit 1
fi
pass "remote accepted dispatch: $job"

# Allow generous timeout: a remote peer might be on a slower / less-warmed
# provider, or the cross-machine RPC could take longer than localhost.
step "polling remote /rpc/task/status (up to 90s)"
if rpc_wait_done "$job" 90; then
  pass "remote job reached terminal state: done"
else
  ec=$?
  if [ $ec -eq 1 ]; then
    fail "remote job ended in failed/error state: $(rpc_error "$job")"
  else
    fail "remote job did not finish within 90s (still: $(rpc_state "$job"))"
  fi
  exit 1
fi

out=$(rpc_output "$job")
step "remote output: ${out:0:120}"

# The remote peer might be on a different LLM provider/model and emit
# "pong" embedded in surrounding text rather than literally that one word —
# accept either case.
ASSERT_CONTAINS "$out" "pong" "remote agent output contains pong"

[ "$SPECTYN_TEST_FAILED" -eq 0 ]
