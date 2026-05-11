#!/usr/bin/env bash
source "$PHANTOM_TEST_LIB/common.sh"
source "$PHANTOM_TEST_LIB/cluster-rpc.sh"

scenario "Cluster RPC — concurrent dispatch, all jobs reach done independently"

ASSERT_HTTP "$(rpc_url)/healthz" 200 "serve up"

# Real LLM is needed because the master agent calls a provider. Skip if
# none of the common keys are set.
if [ -z "${OPENCODE_API_KEY:-}${ANTHROPIC_API_KEY:-}${OPENROUTER_API_KEY:-}${GROQ_API_KEY:-}${GEMINI_API_KEY:-}" ]; then
    warn "no LLM key in env — concurrent dispatch needs a provider; skipping"
    exit 77
fi

N=3   # how many in-flight jobs to fire
step "firing $N concurrent dispatches"

# Background-spawn the curl POSTs, write each job_id to a per-job file
# inside the scenario's tmpdir.
tmp="$(tmpdir)"
declare -a PIDS=()
for i in $(seq 1 $N); do
    (
        BODY="{\"agent\":\"master\",\"prompt\":\"Reply with the single word: tag-${i}\"}"
        AUTH=$(printf '%s' "$BODY" | openssl dgst -sha256 -hmac "$PHANTOM_CLUSTER_SECRET" -hex | awk '{print $2}')
        curl -sS --max-time 10 -X POST "$(rpc_url)/rpc/task/assign" \
             -H "X-Cluster-Auth: $AUTH" \
             -H "Content-Type: application/json" \
             -d "$BODY" > "$tmp/dispatch-$i.json"
    ) &
    PIDS+=($!)
done

# Wait for all dispatch POSTs to return.
for pid in "${PIDS[@]}"; do wait "$pid"; done

# Collect job_ids — fail early if any dispatch returned no id.
# Use grep-based parse (avoids passing MSYS-style paths to native-Python).
declare -a JOBS=()
for i in $(seq 1 $N); do
    JID=$(grep -oE '"job_id":"[^"]+"' "$tmp/dispatch-$i.json" | head -1 \
          | sed -E 's/.*"job_id":"([^"]+)".*/\1/')
    if [ -z "$JID" ]; then
        fail "dispatch #$i returned no job_id (body was: $(cat "$tmp/dispatch-$i.json"))"
        exit 1
    fi
    JOBS+=("$JID")
done
pass "$N dispatches accepted: ${JOBS[*]}"

# Poll all jobs in parallel; record done states. Shared deadline so a slow
# job doesn't keep us waiting beyond a sensible budget.
deadline=$(( $(date +%s) + 90 ))
declare -A STATE
for j in "${JOBS[@]}"; do STATE[$j]="pending"; done

while [ "$(date +%s)" -lt "$deadline" ]; do
    pending=0
    for j in "${JOBS[@]}"; do
        if [ "${STATE[$j]}" != "done" ] && [ "${STATE[$j]}" != "failed" ]; then
            s=$(rpc_state "$j")
            case "$s" in
                done|completed) STATE[$j]="done"   ;;
                failed|error)   STATE[$j]="failed" ;;
                *)              STATE[$j]="pending"; pending=$((pending+1)) ;;
            esac
        fi
    done
    [ "$pending" -eq 0 ] && break
    sleep 2
done

# Tally + assert all done.
done_count=0; failed_count=0; pending_count=0
for j in "${JOBS[@]}"; do
    case "${STATE[$j]}" in
        done)    done_count=$((done_count+1)) ;;
        failed)  failed_count=$((failed_count+1)) ;;
        *)       pending_count=$((pending_count+1)) ;;
    esac
done
step "results — done: $done_count, failed: $failed_count, still pending: $pending_count"

if [ "$done_count" -eq "$N" ]; then
    pass "all $N concurrent jobs reached done"
else
    fail "only $done_count of $N jobs completed (failed=$failed_count, pending=$pending_count)"
fi

# Also: each job's output should reflect its tag (proves jobs didn't get
# their replies crossed). Best-effort, not a hard fail since LLMs vary.
crossed=0
for i in $(seq 1 $N); do
    j="${JOBS[$((i-1))]}"
    [ "${STATE[$j]}" = "done" ] || continue
    out=$(rpc_output "$j")
    if printf '%s' "$out" | grep -q "tag-$i"; then
        :  # correct
    else
        crossed=$((crossed+1))
    fi
done
if [ "$crossed" -eq 0 ]; then
    pass "all per-job outputs contained their own tag (no crossed wires)"
else
    warn "$crossed job(s) didn't contain expected tag — could be LLM variance, not necessarily a bug"
fi

[ "$PHANTOM_TEST_FAILED" -eq 0 ]
