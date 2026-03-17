# Clawtex Core API Guide

Clawtex Core is a Rust daemon running on port **7878** (hub) that provides:

- LLM routing across 10+ providers
- Multi-agent orchestration with tool calling
- 29-hand workflow automation engine
- 8-device distributed cluster management
- Full observability (metrics, costs, trajectories, audit)

---

## Authentication

All endpoints except `/health` and `/dashboard` require:

```
Authorization: Bearer <hub_api_key>
```

The key is either set in `~/.clawtex/agents.toml`:

```toml
[core]
hub_api_key = "your-key-here"
```

Or auto-generated on daemon startup and printed to logs. The default configured key for this cluster is `clawtex-hub-2026`.

---

## Base URLs

| Node | URL | Role |
|------|-----|------|
| Z13 (Hub) | http://localhost:7878 | Primary hub + LLM |
| M1 Mac | http://100.87.93.58:7879 | Full worker |
| AYANEO | http://100.107.205.98:7880 | NPU worker |
| Acer | http://192.168.1.115:7881 | Light worker |

---

## Health

### GET /health
No auth required.

```bash
curl http://localhost:7878/health
# {"status":"ok","version":"0.1.0","service":"clawtex-core"}
```

### GET /metrics
Prometheus exposition format.

```bash
curl -H "Authorization: Bearer clawtex-hub-2026" http://localhost:7878/metrics
```

### GET /metrics/health
JSON health summary with uptime, worker count, tool count.

```bash
curl -H "Authorization: Bearer clawtex-hub-2026" http://localhost:7878/metrics/health
```

### GET /dashboard?token=\<token\>
HTML web dashboard. Token is printed on daemon startup. No Bearer auth required.

---

## Agent

### POST /llm/route
Route a prompt directly to an LLM provider (bypasses agent tool loop).

```bash
curl -s -X POST http://localhost:7878/llm/route \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{"prompt": "What is Rust?", "provider": "auto"}'
```

Providers: `auto`, `ollama`, `gemini`, `groq`, `chatgpt`, `codex`, `opencode`, `cerebras`, `openrouter`, `lmstudio`, `npu`

### POST /task
Add a task to the queue.

```bash
curl -s -X POST http://localhost:7878/task \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{"title": "Summarize news", "prompt": "Summarize AI news from today"}'
# {"task_id":"abc123","status":"pending"}
```

### POST /task/:id/run
Execute a queued task.

```bash
curl -s -X POST http://localhost:7878/task/abc123/run \
  -H "Authorization: Bearer clawtex-hub-2026"
```

### GET /task/history
List last 20 completed tasks.

```bash
curl -s http://localhost:7878/task/history \
  -H "Authorization: Bearer clawtex-hub-2026"
```

### POST /agent/:name/run
Run a named agent with a prompt. Stateless (no conversation history).

```bash
curl -s -X POST http://localhost:7878/agent/master/run \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{"prompt": "Search for Rust async tutorials and summarize them"}'
```

Response:
```json
{
  "agent": "master",
  "result": "...",
  "tool_calls": 3,
  "elapsed": 8.4
}
```

### GET /stream/agent/:name?prompt=...
Server-Sent Events stream for real-time agent responses.

```bash
curl -N "http://localhost:7878/stream/agent/master?prompt=Hello&Authorization=Bearer%20clawtex-hub-2026"
# Alternatively use EventSource in browser
```

SSE events: `content`, `tool_start`, `tool_args`, `done`, `error`

### GET /ws/agent/:name
WebSocket connection. Send `{"prompt": "..."}`, receive `{"event": "content", "data": "..."}`.

### POST /agent/think
Hub-side reasoning for mobile/tablet workers. Rate limited to 10 calls/min per worker.

```bash
curl -s -X POST http://localhost:7878/agent/think \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{
    "worker": "rog6",
    "prompt": "Search for today weather in Tokyo",
    "available_tools": ["web_search", "http_request"]
  }'
```

### GET /events/agent/:name
Subscribe to agent event bus (SSE). Receives all AgentEvent objects in real-time.

---

## Hands

Hands are multi-phase agentic workflows defined in TOML files under `~/.clawtex/hands/`.

### GET /hands
List all loaded hands with metadata.

```bash
curl -s http://localhost:7878/hands \
  -H "Authorization: Bearer clawtex-hub-2026"
```

Response:
```json
{
  "hands": [
    {"name": "content", "description": "...", "category": "marketing", "phases": 4, "tools": ["web_search",...]}
  ],
  "count": 29
}
```

### POST /hand/:name/run
Execute a hand workflow. Long-running (may take minutes).

```bash
curl -s -X POST http://localhost:7878/hand/content/run \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{"prompt": "Write 5 tweets about distributed AI systems"}'
```

Response includes phase-by-phase details and final output. If the hand has `chain_to` configured, the chained hand runs automatically and results are included under `chained_hand`.

---

## Tools

### GET /tools
List all 42+ registered tools.

```bash
curl -s http://localhost:7878/tools \
  -H "Authorization: Bearer clawtex-hub-2026"
```

### GET /workspace/files
List files in the agent workspace directory (`~/.clawtex/workspace/`).

```bash
curl -s http://localhost:7878/workspace/files \
  -H "Authorization: Bearer clawtex-hub-2026"
```

---

## Cluster

### GET /cluster/status
All registered nodes including offline ones.

```bash
curl -s http://localhost:7878/cluster/status \
  -H "Authorization: Bearer clawtex-hub-2026"
```

### GET /cluster/workers
Only currently online workers.

```bash
curl -s http://localhost:7878/cluster/workers \
  -H "Authorization: Bearer clawtex-hub-2026"
```

### POST /cluster/register
Worker self-registration (called by worker daemon on startup).

```bash
curl -s -X POST http://localhost:7878/cluster/register \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "acer",
    "host": "192.168.1.115",
    "port": 7881,
    "capabilities": ["web_search", "http_request"],
    "device_type": "light"
  }'
```

### POST /cluster/heartbeat
Workers send this every 30s to stay marked as online.

```bash
curl -s -X POST http://localhost:7878/cluster/heartbeat \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{"name": "acer", "cpu_load": 0.23}'
```

### POST /cluster/dispatch
Dispatch a tool to the best available worker. Supports targeted dispatch.

```bash
# Auto-route (load-balanced)
curl -s -X POST http://localhost:7878/cluster/dispatch \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{"tool": "web_search", "input": {"query": "AI news"}}'

# Target specific worker
curl -s -X POST http://localhost:7878/cluster/dispatch \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{"tool": "shell", "input": {"command": "uname -a"}, "worker": "m1-mac"}'

# Target by capability
curl -s -X POST http://localhost:7878/cluster/dispatch \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{"tool": "sensor_gps", "input": {}, "capability": "gps"}'
```

### GET /cluster/poll?worker=\<name\>
Mobile workers call this to get their next task (15s interval). Also serves as heartbeat.

```bash
curl -s "http://localhost:7878/cluster/poll?worker=rog6" \
  -H "Authorization: Bearer clawtex-hub-2026"
# {"status":"idle"} or {"status":"task","task_id":"...","tool":"web_search","input":{...}}
```

### POST /cluster/result
Mobile worker submits completed task result.

```bash
curl -s -X POST http://localhost:7878/cluster/result \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{"task_id": "abc123", "success": true, "output": "...", "worker": "rog6"}'
```

### GET /cluster/metrics
Cluster-wide performance data (total dispatches, success rates, latencies).

### GET /cluster/metrics/:worker
Per-worker statistics.

### GET /cluster/health
Circuit breaker states for all providers + watchdog recovery events.

```bash
curl -s http://localhost:7878/cluster/health \
  -H "Authorization: Bearer clawtex-hub-2026"
```

### POST /cluster/onboard
Automated worker onboarding via SSH (installs binary, configures, starts).

### GET /cluster/onboard/status/:worker
Check onboarding progress for a worker.

### GET /cluster/onboard/verify/:worker
Verify worker health (reachability, capabilities, tool test).

### POST /cluster/onboard/mobile
Generate mobile app deep link for joining the cluster.

```bash
curl -s -X POST http://localhost:7878/cluster/onboard/mobile \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{
    "worker_name": "my-phone",
    "hub_url": "http://100.107.205.98:7878",
    "auth_token": "clawtex-hub-2026",
    "capabilities": ["web_search", "sensor_gps"]
  }'
```

### POST /cluster/consistency-test
Run cross-device consistency tests (compare LLM outputs across workers).

```bash
curl -s -X POST http://localhost:7878/cluster/consistency-test \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{"predefined": true, "threshold": 0.85}'
```

### GET /cluster/consistency-history?limit=20
View historical consistency test results.

### GET /cluster/preemption/pending
Pending high-priority task preemption restorations.

### GET /cluster/preemption/history?limit=50
Preemption event history.

### GET /cluster/scores
Node capability rankings with grades (A-F).

### GET /cluster/scores/:node
Detailed score for one node (stability, speed, cost_efficiency, quality).

### POST /cluster/scores/:node
Update node performance metrics (called by monitoring systems).

---

## Admin

### Costs

```bash
# 7-day cost summary
curl -s http://localhost:7878/costs \
  -H "Authorization: Bearer clawtex-hub-2026"
```

### Revenue

```bash
# 30-day revenue summary
curl -s http://localhost:7878/revenue \
  -H "Authorization: Bearer clawtex-hub-2026"
```

### Reports

```bash
# Daily ops report (JSON)
curl -s http://localhost:7878/reports/daily \
  -H "Authorization: Bearer clawtex-hub-2026"

# Weekly ops report
curl -s http://localhost:7878/reports/weekly \
  -H "Authorization: Bearer clawtex-hub-2026"

# Generate Telegram-formatted daily report
curl -s -X POST http://localhost:7878/reports/send \
  -H "Authorization: Bearer clawtex-hub-2026"
```

### Service Tiers

```bash
# Get agent tier
curl -s http://localhost:7878/tier/master \
  -H "Authorization: Bearer clawtex-hub-2026"

# Set agent tier
curl -s -X PUT http://localhost:7878/tier/master \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{"tier": "pro"}'

# Get usage stats
curl -s http://localhost:7878/tier/master/usage \
  -H "Authorization: Bearer clawtex-hub-2026"
```

Tiers: `lite`, `standard`, `pro`, `enterprise`

### Audit Log

```bash
# Query audit log
curl -s "http://localhost:7878/audit?agent=master&limit=50" \
  -H "Authorization: Bearer clawtex-hub-2026"

# Filter by tool and risk level
curl -s "http://localhost:7878/audit?tool=shell&risk_level=high" \
  -H "Authorization: Bearer clawtex-hub-2026"
```

### Tenants (Multi-tenant)

```bash
# Create tenant
curl -s -X POST http://localhost:7878/tenants \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{"name": "Acme Corp", "tier": "standard"}'

# List tenants
curl -s http://localhost:7878/tenants \
  -H "Authorization: Bearer clawtex-hub-2026"

# Validate API key
curl -s "http://localhost:7878/tenants/validate?key=<tenant-api-key>" \
  -H "Authorization: Bearer clawtex-hub-2026"

# Update tenant tier
curl -s -X PUT http://localhost:7878/tenants/<id>/tier \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{"tier": "pro"}'
```

### Load Testing

```bash
# Start stress test with named profile
curl -s -X POST http://localhost:7878/test/stress \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{"profile": "light"}'

# Check status
curl -s http://localhost:7878/test/stress/status \
  -H "Authorization: Bearer clawtex-hub-2026"

# List available profiles
curl -s http://localhost:7878/test/profiles \
  -H "Authorization: Bearer clawtex-hub-2026"
```

Profiles: `light` (low load), `medium`, `heavy`

### Auto-Diagnosis

```bash
# Submit error for diagnosis
curl -s -X POST http://localhost:7878/diagnose \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{
    "error_message": "connection refused to ollama",
    "agent_name": "master",
    "tool_name": "shell"
  }'

# Get recent diagnoses
curl -s http://localhost:7878/diagnose/recent?limit=10 \
  -H "Authorization: Bearer clawtex-hub-2026"

# List known issue patterns
curl -s http://localhost:7878/diagnose/known-issues \
  -H "Authorization: Bearer clawtex-hub-2026"
```

### Observational Memory

```bash
# Compress conversation into observation
curl -s -X POST http://localhost:7878/memory/observe \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{
    "session_id": "chat_123",
    "messages": [
      {"role": "user", "content": "How do I use Rust async?"},
      {"role": "assistant", "content": "Use tokio::spawn..."}
    ]
  }'

# Search observations
curl -s "http://localhost:7878/memory/observations?query=rust+async&limit=5" \
  -H "Authorization: Bearer clawtex-hub-2026"

# Get stats (tokens saved)
curl -s http://localhost:7878/memory/observations/stats \
  -H "Authorization: Bearer clawtex-hub-2026"
```

### Customer Health

```bash
# List all customer health scores
curl -s http://localhost:7878/customers/health \
  -H "Authorization: Bearer clawtex-hub-2026"

# Update scores for a customer
curl -s -X PUT http://localhost:7878/customers/health/cust_001 \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{"name": "ACME", "efficiency": 0.85, "quality": 0.90, "speed": 0.75, "satisfaction": 0.88}'

# Get at-risk customers
curl -s http://localhost:7878/customers/at-risk \
  -H "Authorization: Bearer clawtex-hub-2026"

# Get churn alerts
curl -s http://localhost:7878/customers/churn-alerts \
  -H "Authorization: Bearer clawtex-hub-2026"

# Record activity (resets churn timer)
curl -s -X POST http://localhost:7878/customers/cust_001/activity \
  -H "Authorization: Bearer clawtex-hub-2026"
```

### Order Workflow

```bash
# Create order
curl -s -X POST http://localhost:7878/orders \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{"customer_name": "ACME", "customer_email": "ops@acme.com", "service_tier": "pro", "amount_usd": 499}'

# List orders (filter by status)
curl -s "http://localhost:7878/orders?status=in_progress" \
  -H "Authorization: Bearer clawtex-hub-2026"

# Pipeline summary
curl -s http://localhost:7878/orders/pipeline \
  -H "Authorization: Bearer clawtex-hub-2026"

# Transition status
curl -s -X PUT http://localhost:7878/orders/<id>/status \
  -H "Authorization: Bearer clawtex-hub-2026" \
  -H "Content-Type: application/json" \
  -d '{"status": "completed"}'
```

Order statuses: `new` -> `in_progress` -> `review` -> `completed` (or `cancelled`)

### E-Stop (Emergency Stop)

```bash
# Halt all agent operations
curl -s -X POST http://localhost:7878/estop \
  -H "Authorization: Bearer clawtex-hub-2026"

# Resume normal operation
curl -s -X DELETE http://localhost:7878/estop \
  -H "Authorization: Bearer clawtex-hub-2026"

# Check current state
curl -s http://localhost:7878/estop \
  -H "Authorization: Bearer clawtex-hub-2026"
```

### Trajectories

```bash
# Query recent trajectories
curl -s "http://localhost:7878/trajectories?days=7&limit=50" \
  -H "Authorization: Bearer clawtex-hub-2026"

# Filter by hand
curl -s "http://localhost:7878/trajectories?hand=seo_content" \
  -H "Authorization: Bearer clawtex-hub-2026"

# Quality stats per provider
curl -s http://localhost:7878/trajectories/stats \
  -H "Authorization: Bearer clawtex-hub-2026"
```

---

## Cron Jobs

Cron is managed via Telegram bot commands. On first daemon startup, 6 default jobs are registered:

| Job | Schedule | Action |
|-----|----------|--------|
| daily-freelancer | 9:00 AM daily | hand:freelancer |
| weekly-leads | Mon 10:00 AM | hand:lead |
| biweekly-seo-content | Tue+Thu 11:00 AM | hand:seo_content |
| daily-content | 8:00 AM daily | hand:content |
| cluster-health | Every 4 hours | hand:cluster_health |
| weekly-self-optimize | Sun 3:00 AM | hand:self_optimize |

Bot commands: `/cron list`, `/cron add <schedule> <action> [name]`, `/cron remove <id_prefix>`

---

## Error Codes

| Range | Category |
|-------|----------|
| E1xx | Provider errors |
| E2xx | Tool errors |
| E3xx | Cluster errors |
| E4xx | Config errors |
| E5xx | Agent errors |
| E999 | General errors |

---

## Rate Limits

Default (configurable in `agents.toml` under `[security.rate_limit]`):

- `max_actions_per_hour`: 840 (global tool calls)
- `max_per_tool_per_hour`: 280 (per individual tool)
- `/agent/think`: 10 requests/min per worker
