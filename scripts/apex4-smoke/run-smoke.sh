#!/usr/bin/env bash
# apex4-smoke/run-smoke.sh — drive the apex-④ flagship loop end-to-end from the
# COORDINATOR and assert the govern↔dispatch correlation (D7).
#
#   dispatch a HIGH-RISK claude task  →  worker governor escalates
#     →  /rpc/approvals/list shows a pending card with approval_id (= contract.id)
#     →  /rpc/task/status/<job_id> shows the SAME approval_id   ← THE D7 ASSERT
#     →  approve (auto curl-POST /rpc/inbox, or manual from your phone)
#     →  task continues / completes
#     →  print  APEX4 SMOKE: PASS|FAIL  + save artifacts.
#
# This is the operator validation of docs/superpowers/specs/
# 2026-06-24-govern-dispatch-correlation-design.md (D7) using the runbook
# docs/superpowers/runbooks/apex4-loop-live-smoke.md.
#
# LEAK-SAFE: NO hardcoded IPs / node names / secrets. The peer is an arg; the
# cluster secret + port come from the local agents.toml (or env). The secret is
# never printed.
#
# Usage:
#   ./run-smoke.sh --peer <name-or-url> --agent <claude-agent> \
#                  [--approve auto|manual] [--secret-from agents.toml|env] \
#                  [--prompt "<task>"] [--timeout-await 120] [--timeout-finish 180]
#
#   --peer         worker peer NAME (resolved from ~/.phantom-mesh/peers.json) OR
#                  a full base URL like http://<host>:<port>
#   --agent        the worker agent that drives claude (PreActionDelegated)
#   --approve      auto  = this script signs+POSTs the approval (default)
#                  manual = print the card + wait for you to approve from phone
#   --secret-from  agents.toml (default) | env (PHANTOM_CLUSTER_SECRET)
#   --prompt       override the default deterministic high-risk prompt
#
# HMAC: legacy scheme = HMAC-SHA256(cluster_secret, raw_body) hex, in header
# X-Cluster-Auth. POST bodies sign the JSON string; the GET status poll signs
# the EMPTY body "". Exactly what the worker's require_cluster_auth verifies.
set -euo pipefail

err()  { printf '\033[31mERROR:\033[0m %s\n' "$*" >&2; }
ok()   { printf '\033[32m✓\033[0m %s\n'      "$*" >&2; }
info() { printf '\033[36m◆\033[0m %s\n'      "$*" >&2; }
warn() { printf '\033[33m!\033[0m %s\n'      "$*" >&2; }
die()  { err "$*"; exit 1; }

# ── args ───────────────────────────────────────────────────────────────────
PEER="" ; AGENT="" ; APPROVE="auto" ; SECRET_FROM="agents.toml"
# Deterministic HIGH-RISK prompt: drives claude to run a shell command. The
# governor's classify_tool tokenizes the tool name "Bash" -> [bash] which is in
# the EXEC denylist -> RiskLevel::ExecuteHigh -> a pre-action approval card.
# We make it a SAFE command (echo to a temp file) so an APPROVE does no harm.
PROMPT='Run a shell command (the Bash tool) to print the text APEX4-SMOKE-OK into a temp file under the system temp dir, e.g. echo APEX4-SMOKE-OK > "$TMPDIR/apex4-smoke.txt" (or /tmp on Linux). Use the Bash tool exactly once, then stop.'
TIMEOUT_AWAIT=120     # seconds to wait for the task to reach awaiting-approval
TIMEOUT_FINISH=180    # seconds to wait for the task to finish after approve

while [ $# -gt 0 ]; do
  case "$1" in
    --peer)           PEER="${2:?--peer needs a value}"; shift 2 ;;
    --agent)          AGENT="${2:?--agent needs a value}"; shift 2 ;;
    --approve)        APPROVE="${2:?}"; shift 2 ;;
    --secret-from)    SECRET_FROM="${2:?}"; shift 2 ;;
    --prompt)         PROMPT="${2:?}"; shift 2 ;;
    --timeout-await)  TIMEOUT_AWAIT="${2:?}"; shift 2 ;;
    --timeout-finish) TIMEOUT_FINISH="${2:?}"; shift 2 ;;
    -h|--help)        grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *)                die "unknown arg: $1 (see --help)" ;;
  esac
done

[ -n "$PEER" ]  || die "--peer is required (worker name from peers.json, or a base URL)"
[ -n "$AGENT" ] || die "--agent is required (the worker agent that drives claude)"
case "$APPROVE" in auto|manual) ;; *) die "--approve must be auto|manual (got '$APPROVE')";; esac

command -v curl    >/dev/null 2>&1 || die "curl not on PATH"
command -v openssl >/dev/null 2>&1 || die "openssl not on PATH (needed to sign HMAC)"
command -v python3 >/dev/null 2>&1 && JSON=python3 || JSON=""
# We parse JSON with python3 if present, else a grep/sed fallback (less robust).

# ── locate agents.toml + read secret/port ──────────────────────────────────
AGENTS_TOML=""
for cand in "${PHANTOM_HOME:-$HOME/.phantom-mesh}/agents.toml" "$HOME/.phantom-mesh/agents.toml" "./agents.toml"; do
  if [ -f "$cand" ]; then AGENTS_TOML="$cand"; break; fi
done

toml_scalar() {
  [ -n "$AGENTS_TOML" ] || return 1
  sed -n -E "s/^[[:space:]]*$1[[:space:]]*=[[:space:]]*(.*)$/\1/p" "$AGENTS_TOML" \
    | head -n1 | sed -E 's/[[:space:]]*#.*$//; s/^"//; s/"$//; s/^'"'"'//; s/'"'"'$//'
}

if [ "$SECRET_FROM" = "env" ]; then
  SECRET="${PHANTOM_CLUSTER_SECRET:-}"
  [ -n "$SECRET" ] || die "--secret-from env but PHANTOM_CLUSTER_SECRET is unset/empty"
  ok "cluster secret from env PHANTOM_CLUSTER_SECRET (hidden)"
else
  [ -n "$AGENTS_TOML" ] || die "no agents.toml found (looked in \$PHANTOM_HOME, ~/.phantom-mesh, .). Use --secret-from env, or run \`phantom cluster join\`."
  SECRET="$(toml_scalar cluster_secret || true)"
  [ -n "$SECRET" ] || die "[cluster].cluster_secret missing/empty in $AGENTS_TOML. Run \`phantom cluster join <name>\` or use --secret-from env."
  ok "cluster secret from $AGENTS_TOML (hidden)"
fi

# ── resolve peer to a base URL ─────────────────────────────────────────────
# If --peer looks like a URL use it; else look it up in peers.json by name.
PEER_URL=""
case "$PEER" in
  http://*|https://*) PEER_URL="$PEER" ;;
  *)
    PEERS_JSON="${PHANTOM_HOME:-$HOME/.phantom-mesh}/peers.json"
    [ -f "$PEERS_JSON" ] || PEERS_JSON="$HOME/.phantom-mesh/peers.json"
    [ -f "$PEERS_JSON" ] || die "--peer '$PEER' is a name but no peers.json found (~/.phantom-mesh/peers.json). Run \`phantom config pull\`, or pass --peer http://<host>:<port>."
    if [ -n "$JSON" ]; then
      PEER_URL="$(python3 - "$PEERS_JSON" "$PEER" <<'PY'
import json,sys
data=json.load(open(sys.argv[1]))
peers=data if isinstance(data,list) else data.get("peers",[])
for p in peers:
    if p.get("name")==sys.argv[2]:
        print((p.get("url") or "").rstrip("/")); break
PY
)"
    fi
    [ -n "$PEER_URL" ] || die "peer '$PEER' not found in $PEERS_JSON. Check \`phantom peer ls\`, or pass --peer http://<host>:<port>."
    ;;
esac
PEER_URL="${PEER_URL%/}"
ok "peer '$PEER' -> $PEER_URL"

# ── HMAC helper (legacy scheme) ────────────────────────────────────────────
# hmac_hex <body-string>  -> hex HMAC-SHA256(secret, body). Matches
# core/src/mesh.rs make_auth_token_bytes / openssl dgst -sha256 -hmac.
hmac_hex() {
  printf '%s' "$1" | openssl dgst -sha256 -hmac "$SECRET" 2>/dev/null \
    | sed -E 's/^.*= *//'
}

# json_get <json> <jq-ish dotted key>   — python3 if available, else grep/sed.
json_get() {
  local body="$1" key="$2"
  if [ -n "$JSON" ]; then
    printf '%s' "$body" | python3 - "$key" <<'PY'
import json,sys
key=sys.argv[1]
try: d=json.load(sys.stdin)
except Exception: sys.exit(0)
cur=d
for part in key.split("."):
    if part.startswith("[") and part.endswith("]"):
        i=int(part[1:-1])
        cur=cur[i] if isinstance(cur,list) and len(cur)>i else None
    else:
        cur=cur.get(part) if isinstance(cur,dict) else None
    if cur is None: break
if cur is not None:
    print(cur if not isinstance(cur,(dict,list)) else json.dumps(cur))
PY
  else
    # crude fallback: first "key":"value" or "key":value (no nesting awareness)
    printf '%s' "$body" \
      | grep -oE "\"${key##*.}\"[[:space:]]*:[[:space:]]*\"?[^\",}]*" \
      | head -n1 | sed -E "s/.*:[[:space:]]*\"?//"
  fi
}

# ── preflight: peer reachable ──────────────────────────────────────────────
info "preflight: GET $PEER_URL/healthz"
curl -fsS --max-time 5 "$PEER_URL/healthz" >/dev/null 2>&1 \
  || die "peer $PEER_URL is not reachable on /healthz. Is \`phantom serve\` up on the worker (run worker-up.sh) and is the tailnet/route up?"
ok "peer healthy"

# ── artifacts ──────────────────────────────────────────────────────────────
TS="$(date +%Y%m%d-%H%M%S)"
ART_DIR="${TMPDIR:-/tmp}/apex4-smoke-$TS"
mkdir -p "$ART_DIR"
info "artifacts → $ART_DIR"
save() { printf '%s' "$2" >"$ART_DIR/$1"; }

# ── 1. dispatch the high-risk claude task (async: we poll ourselves) ───────
ASSIGN_BODY="$(printf '{"agent":%s,"prompt":%s}' \
  "$( [ -n "$JSON" ] && python3 -c 'import json,sys;print(json.dumps(sys.argv[1]))' "$AGENT"  || printf '"%s"' "$AGENT" )" \
  "$( [ -n "$JSON" ] && python3 -c 'import json,sys;print(json.dumps(sys.argv[1]))' "$PROMPT" || printf '"%s"' "$PROMPT" )" )"
ASSIGN_SIG="$(hmac_hex "$ASSIGN_BODY")"
info "dispatching HIGH-RISK claude task to '$AGENT' on $PEER…"
ASSIGN_RESP="$(curl -fsS --max-time 20 \
  -H "X-Cluster-Auth: $ASSIGN_SIG" -H 'Content-Type: application/json' \
  --data "$ASSIGN_BODY" "$PEER_URL/rpc/task/assign" || true)"
save assign-response.json "$ASSIGN_RESP"
JOB_ID="$(json_get "$ASSIGN_RESP" job_id)"
[ -n "$JOB_ID" ] || { err "dispatch failed — no job_id. Response:"; echo "$ASSIGN_RESP" >&2; \
  err "  Common causes: HMAC rejected (secret mismatch), agent '$AGENT' missing on worker, wire-version mismatch."; \
  echo "APEX4 SMOKE: FAIL (dispatch)"; exit 1; }
ok "job_id = $JOB_ID"
save job_id.txt "$JOB_ID"

# ── 2. poll /rpc/task/status until awaiting-approval (status 'running' WITH a
#       pending card on the worker). We detect the awaiting state by polling
#       /rpc/approvals/list for a card whose task_id == job_id. ──────────────
EMPTY_SIG="$(hmac_hex "")"     # GET status signs the empty body
LIST_SIG="$(hmac_hex "{}")"    # POST /rpc/approvals/list with empty-ish body "{}"

get_status() {
  curl -fsS --max-time 8 -H "X-Cluster-Auth: $EMPTY_SIG" \
    "$PEER_URL/rpc/task/status/$JOB_ID" 2>/dev/null || true
}
list_approvals() {
  curl -fsS --max-time 8 -H "X-Cluster-Auth: $LIST_SIG" -H 'Content-Type: application/json' \
    --data '{}' "$PEER_URL/rpc/approvals/list" 2>/dev/null || true
}

info "waiting up to ${TIMEOUT_AWAIT}s for the task to reach AWAITING-APPROVAL…"
CARD_APPROVAL_ID="" ; CARD_JSON="" ; LAST_STATUS=""
DEADLINE=$(( $(date +%s) + TIMEOUT_AWAIT ))
while [ "$(date +%s)" -lt "$DEADLINE" ]; do
  ST="$(get_status)"; LAST_STATUS="$(json_get "$ST" status)"
  case "$LAST_STATUS" in
    error)
      save status-error.json "$ST"
      err "task went to ERROR before any approval. Status:"; echo "$ST" >&2
      echo "APEX4 SMOKE: FAIL (task errored pre-approval)"; exit 1 ;;
    done)
      # Completed without ever pausing => the run never hit the pre-action gate.
      save status-done-noapproval.json "$ST"
      err "task COMPLETED without ever awaiting approval."
      err "  This is the CURRENT-STATE gap the smoke is built to expose:"
      err "  dispatched claude work does not reach the PreToolUse pre-action gate"
      err "  (no provider key maps a dispatched agent to a governed CliKind::Claude;"
      err "  claude_agent uses 'claude -p' which is NOT governed). See README 'Known gap'."
      echo "APEX4 SMOKE: FAIL (no pre-action approval reached)"; exit 1 ;;
  esac
  LIST="$(list_approvals)"
  # find a card whose task_id == JOB_ID and grab its approval_id
  if [ -n "$JSON" ]; then
    CARD_APPROVAL_ID="$(printf '%s' "$LIST" | python3 - "$JOB_ID" <<'PY'
import json,sys
job=sys.argv[1]
try: d=json.load(sys.stdin)
except Exception: sys.exit(0)
for c in d.get("pending",[]):
    if str(c.get("task_id"))==job: print(c.get("approval_id","")); break
PY
)"
    if [ -n "$CARD_APPROVAL_ID" ]; then
      CARD_JSON="$(printf '%s' "$LIST" | python3 - "$JOB_ID" <<'PY'
import json,sys
job=sys.argv[1]
d=json.load(sys.stdin)
for c in d.get("pending",[]):
    if str(c.get("task_id"))==job: print(json.dumps(c)); break
PY
)"
    fi
  else
    # fallback: any approval_id in the list (best-effort; can't match task_id robustly)
    CARD_APPROVAL_ID="$(json_get "$LIST" approval_id)"
  fi
  if [ -n "$CARD_APPROVAL_ID" ]; then break; fi
  sleep 2
done

save approvals-list.json "${LIST:-}"
[ -n "$CARD_JSON" ] && save pending-card.json "$CARD_JSON"
if [ -z "$CARD_APPROVAL_ID" ]; then
  err "task never produced a pending approval card within ${TIMEOUT_AWAIT}s (last status: ${LAST_STATUS:-?})."
  err "  Either the run isn't governed (PHANTOM_GOVERN_CLI=1 on the worker?),"
  err "  the agent doesn't drive claude (only claude is PreActionDelegated),"
  err "  or the dispatched claude path doesn't reach the gate (see README 'Known gap')."
  echo "APEX4 SMOKE: FAIL (never reached awaiting-approval)"; exit 1
fi
ok "pending card found — approval_id (contract.id) = $CARD_APPROVAL_ID"

# ── 3. THE D7 ASSERT: dispatch row's approval_id == the card's approval_id ──
ST="$(get_status)"; save status-awaiting.json "$ST"
ROW_APPROVAL_ID="$(json_get "$ST" approval_id)"
info "dispatch row /rpc/task/status approval_id = '${ROW_APPROVAL_ID:-<empty>}'"
info "live pending card approval_id            = '$CARD_APPROVAL_ID'"
CORRELATION_PASS=0
if [ -n "$ROW_APPROVAL_ID" ] && [ "$ROW_APPROVAL_ID" = "$CARD_APPROVAL_ID" ]; then
  CORRELATION_PASS=1
  ok "D7 CORRELATION: PASS — dispatch row approval_id == card contract.id"
else
  err "D7 CORRELATION: FAIL — the dispatch row's approval_id does NOT equal the card's."
  err "  empty row approval_id = the pre-fix gap (set_approval_id never reached the row);"
  err "  the fix stamps it via the claude PreToolUse hook's with_dispatch_store at card-write time."
fi
printf '%s' "$ROW_APPROVAL_ID" >"$ART_DIR/row_approval_id.txt"
printf '%s' "$CARD_APPROVAL_ID" >"$ART_DIR/card_approval_id.txt"

# ── 4. approve ─────────────────────────────────────────────────────────────
approve_auto() {
  local body sig
  body="$(printf '{"topic":%s,"text":"approve","from":"apex4-smoke"}' \
    "$( [ -n "$JSON" ] && python3 -c 'import json,sys;print(json.dumps(sys.argv[1]))' "$CARD_APPROVAL_ID" || printf '"%s"' "$CARD_APPROVAL_ID" )")"
  sig="$(hmac_hex "$body")"
  info "approving (auto): POST /rpc/inbox {topic:$CARD_APPROVAL_ID, text:approve}"
  curl -fsS --max-time 10 -H "X-Cluster-Auth: $sig" -H 'Content-Type: application/json' \
    --data "$body" "$PEER_URL/rpc/inbox" >"$ART_DIR/approve-response.json" 2>&1 \
    || die "approve POST failed — see $ART_DIR/approve-response.json"
  ok "approval submitted"
}

if [ "$APPROVE" = "auto" ]; then
  approve_auto
else
  echo >&2
  warn "MANUAL approve mode. Approve this card from your phone (审核 tab) NOW:"
  echo  "    approval_id : $CARD_APPROVAL_ID" >&2
  echo  "    job_id      : $JOB_ID" >&2
  [ -n "$CARD_JSON" ] && { echo "    card        : $CARD_JSON" >&2; }
  echo  "    (or: POST $PEER_URL/rpc/inbox {topic:\"$CARD_APPROVAL_ID\", text:\"approve\"})" >&2
  echo  "  waiting for the card to clear / the task to leave awaiting…" >&2
fi

# ── 5. poll status until the task continues / completes ────────────────────
info "waiting up to ${TIMEOUT_FINISH}s for the task to continue + finish…"
FINAL_STATUS="" ; DEADLINE=$(( $(date +%s) + TIMEOUT_FINISH ))
while [ "$(date +%s)" -lt "$DEADLINE" ]; do
  ST="$(get_status)"; FINAL_STATUS="$(json_get "$ST" status)"
  case "$FINAL_STATUS" in
    done|error) break ;;
  esac
  sleep 3
done
save status-final.json "${ST:-}"
ok "final status = ${FINAL_STATUS:-?}"

# ── 6. verdict ─────────────────────────────────────────────────────────────
echo >&2
info "── apex-④ smoke summary ──"
echo  "  peer        : $PEER ($PEER_URL)" >&2
echo  "  agent       : $AGENT" >&2
echo  "  job_id      : $JOB_ID" >&2
echo  "  approval_id : $CARD_APPROVAL_ID" >&2
echo  "  row approval_id (status): ${ROW_APPROVAL_ID:-<empty>}" >&2
echo  "  D7 correlation : $( [ "$CORRELATION_PASS" -eq 1 ] && echo PASS || echo FAIL )" >&2
echo  "  final status: ${FINAL_STATUS:-?}" >&2
echo  "  artifacts   : $ART_DIR" >&2

if [ "$CORRELATION_PASS" -eq 1 ] && [ "$FINAL_STATUS" = "done" ]; then
  echo "APEX4 SMOKE: PASS"
  exit 0
else
  REASONS=""
  [ "$CORRELATION_PASS" -ne 1 ] && REASONS="$REASONS correlation-mismatch"
  [ "$FINAL_STATUS" != "done" ] && REASONS="$REASONS task-not-completed(${FINAL_STATUS:-?})"
  echo "APEX4 SMOKE: FAIL —$REASONS"
  exit 1
fi
