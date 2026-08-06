# shellcheck shell=bash
# inspect.sh — read spectyn's runtime state from disk (events, DBs, conversations).
#
# Public surface:
#   events_count                 -> total lines in events.jsonl
#   events_tail <n>              -> last n events (raw JSONL)
#   events_since <ts_ms>         -> events with ts_ms > given timestamp
#   conversations_count          -> number of *.jsonl in conversations/
#   conversation_latest          -> path to most recently modified jsonl
#   costs_recent <n>             -> last n cost records (csv-ish)
#   cluster_peer_count           -> rows in cluster_nodes table
#   doctor_summary               -> spectyn doctor stripped of ANSI

events_count() {
  wc -l < "$SPECTYN_CONFIG_DIR/events.jsonl" 2>/dev/null | tr -d ' \n'
}

events_tail() {
  tail -n "${1:-10}" "$SPECTYN_CONFIG_DIR/events.jsonl" 2>/dev/null
}

events_since() {
  local since="$1"
  python -c "
import json, sys
since = int('$since' or 0)
with open(r'$SPECTYN_CONFIG_DIR/events.jsonl') as f:
    for line in f:
        try:
            e = json.loads(line)
            if e.get('ts_ms', 0) > since:
                print(line.rstrip())
        except Exception:
            pass
" 2>/dev/null
}

conversations_count() {
  ls "$SPECTYN_CONFIG_DIR/conversations"/*.jsonl 2>/dev/null | wc -l | tr -d ' \n'
}

conversation_latest() {
  ls -t "$SPECTYN_CONFIG_DIR/conversations"/*.jsonl 2>/dev/null | head -1
}

costs_recent() {
  local n="${1:-5}"
  python -c "
import sqlite3
db = sqlite3.connect(r'$SPECTYN_CONFIG_DIR/costs.db')
for r in db.execute(
    'SELECT timestamp, agent, provider, model, tokens_in, tokens_out, estimated_cost_usd '
    'FROM cost_records ORDER BY timestamp DESC LIMIT ?', ($n,)
):
    print(' | '.join(str(x) for x in r))
" 2>/dev/null
}

cluster_peer_count() {
  python -c "
import sqlite3
db = sqlite3.connect(r'$SPECTYN_CONFIG_DIR/cluster.db')
print(db.execute('SELECT count(*) FROM cluster_nodes').fetchone()[0])
" 2>/dev/null
}

doctor_summary() {
  "$SPECTYN_BIN" doctor 2>&1 | sed -E 's/\x1b\[[0-9;]*m//g'
}

# now_ms — current time in milliseconds (matches events.jsonl ts_ms)
now_ms() {
  python -c "import time; print(int(time.time()*1000))" 2>/dev/null \
    || date +%s%3N 2>/dev/null \
    || echo $(($(date +%s) * 1000))
}
