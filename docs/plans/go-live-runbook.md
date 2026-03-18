# Clawtex-Core Go-Live Production Runbook

> **Version**: 1.0
> **Date**: 2026-03-17
> **System**: clawtex-core (Rust daemon) v0.1.0
> **Scope**: 8-device cluster (1 Hub + 3 PC Workers + 4 Mobile Workers)
> **Port**: 7878 (Hub), 7879-7881 (Workers)
> **Auth**: Bearer `your-hub-token-here`
> **Status**: Production-ready

---

## Table of Contents

1. [Cluster Topology](#1-cluster-topology)
2. [Pre-Deploy Checklist](#2-pre-deploy-checklist)
3. [Deploy Steps](#3-deploy-steps)
4. [Health Verification](#4-health-verification)
5. [Rollback Procedure](#5-rollback-procedure)
6. [Post-Deploy Monitoring (First 24h)](#6-post-deploy-monitoring-first-24h)
7. [Cron Job Verification](#7-cron-job-verification)
8. [Known Issues and Workarounds](#8-known-issues-and-workarounds)
9. [Emergency Procedures](#9-emergency-procedures)
10. [Appendix: API Endpoint Reference](#appendix-a-api-endpoint-reference)

---

## 1. Cluster Topology

### Node Inventory

| Node | Role | Hardware | Port | Network | SSH |
|------|------|----------|------|---------|-----|
| Z13 | **Hub** + Main LLM | Ryzen AI MAX+ 395, Radeon 8060S, 64GB, NPU 50T | 7878 | localhost | localhost |
| M1 Mac | Full Worker | Apple M1 | 7879 | Tailscale VPN | `ssh worker@10.0.2.1` (no password) |
| AYANEO | NPU Worker | AMD NPU (XDNA) | 7880 | Tailscale | `ssh worker@10.0.2.2` |
| Acer | Light Worker | Basic spec, 7TB HDD | 7881 | LAN | `ssh worker@10.0.1.1` (no password) |
| ROG6 | Mobile Worker | ROG Phone 6 | polling | WiFi/Mobile | App polling mode |
| Mi Pad | Mobile Worker | Xiaomi Mi Pad | polling | WiFi | App polling mode |
| iPhone | Mobile Worker | iPhone | polling | WiFi/Mobile | App polling mode |
| iPad | Mobile Worker | iPad | polling | WiFi | App polling mode |

### Z13 Local Services

| Service | Port | Purpose |
|---------|------|---------|
| clawtex-core | 7878 | Hub daemon |
| LM Studio | 1234 | qwen3-coder-next |
| Ollama | 11434 | llama3.2:1b |
| NPU Server | 8000 | Mistral-7B (AMD XDNA) |

### Critical Data Assets

| Data | Path | Criticality | Recoverable? |
|------|------|-------------|--------------|
| `core.db` | `~/.clawtex/core.db` | **Critical** | No -- sessions, tasks, cluster state |
| `costs.db` | `~/.clawtex/costs.db` | **High** | No -- operational cost records |
| `memory.db` | `~/.clawtex/memory.db` | **Critical** | No -- AI accumulated knowledge |
| `knowledge.db` | `~/.clawtex/knowledge.db` | **High** | No -- captured problem/decision/lesson |
| `revenue.db` | `~/.clawtex/revenue.db` | **Critical** | No -- financial data |
| `trajectories.db` | `~/.clawtex/trajectories.db` | **Medium** | No -- trajectory logs |
| `agents.toml` | `~/.clawtex/agents.toml` | **Critical** | Yes -- Git version controlled |
| `hands/` | `~/.clawtex/hands/` | **High** | Yes -- Git version controlled |
| `.secret_key` | `~/.clawtex/.secret_key` | **Critical** | No -- loss means all enc2: secrets unrecoverable |

---

## 2. Pre-Deploy Checklist

Complete every item before proceeding to deploy steps. Mark each item as verified.

### 2.1 Code Quality

- [ ] **All tests pass**: Run `cargo test` from the project root. Expect 1011+ tests, 0 failures.
  ```bash
  cd C:\Users\worker\Desktop\adreanalai\LLM-Cluster-Project\clawtex-core
  cargo test 2>&1 | tail -5
  # Expected: test result: ok. 1011 passed; 0 failed; 0 ignored
  ```

- [ ] **No P0/P1 issues open**: Review the master plan and tech backlog for critical blockers.

- [ ] **All TOML parse warnings fixed**: Verify that `prompt_evolve` and `report` hand TOML files parse cleanly.
  ```bash
  # Dry-run the daemon to catch TOML parse warnings
  cargo run --release -- --host 127.0.0.1 daemon 2>&1 | grep -i "warn\|error\|parse" | head -20
  # Kill after 10 seconds (Ctrl+C) — look for zero TOML warnings
  ```

- [ ] **Release binary builds cleanly**:
  ```bash
  cargo build --release 2>&1 | tail -3
  # Confirm: Finished `release` profile
  ```

- [ ] **No uncommitted changes**: Ensure Git is clean or all changes are committed.
  ```bash
  git status
  ```

### 2.2 Database Backups

Create timestamped backups of all databases before deployment.

```bash
# Create backup directory with timestamp
$BACKUP_DIR="$HOME/.clawtex/backups/$(date +%Y%m%d_%H%M%S)"
mkdir -p "$BACKUP_DIR"

# Backup all critical databases
cp "$HOME/.clawtex/core.db" "$BACKUP_DIR/core.db"
cp "$HOME/.clawtex/core.db-wal" "$BACKUP_DIR/core.db-wal" 2>/dev/null
cp "$HOME/.clawtex/core.db-shm" "$BACKUP_DIR/core.db-shm" 2>/dev/null
cp "$HOME/.clawtex/costs.db" "$BACKUP_DIR/costs.db"
cp "$HOME/.clawtex/memory.db" "$BACKUP_DIR/memory.db"
cp "$HOME/.clawtex/knowledge.db" "$BACKUP_DIR/knowledge.db"
cp "$HOME/.clawtex/revenue.db" "$BACKUP_DIR/revenue.db"
cp "$HOME/.clawtex/trajectories.db" "$BACKUP_DIR/trajectories.db"

# Backup the encryption key (HANDLE WITH EXTREME CARE)
cp "$HOME/.clawtex/.secret_key" "$BACKUP_DIR/.secret_key"

echo "Backup created at: $BACKUP_DIR"
ls -la "$BACKUP_DIR"
```

- [ ] **Backups verified**: Confirm all files exist and have non-zero sizes.
- [ ] **Off-node backup**: Copy backups to Acer for redundancy.
  ```bash
  scp -r "$BACKUP_DIR" worker@10.0.1.1:'C:\Users\worker\clawtex-backups\'
  ```

### 2.3 Config Backup

```bash
# Backup configuration
cp "$HOME/.clawtex/agents.toml" "$BACKUP_DIR/agents.toml"
cp -r "$HOME/.clawtex/hands" "$BACKUP_DIR/hands"
```

- [ ] **Config backup verified**: Confirm `agents.toml` and all hand TOML files are backed up.

### 2.4 Provider Health Check

Verify all 10 providers are responding before deploy.

```bash
# Quick LLM ping test (requires daemon running)
curl -s -X POST http://localhost:7878/llm/route \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-hub-token-here" \
  -d '{"prompt": "ping", "provider": "auto"}' | head -c 200
```

- [ ] **Ollama responding**: `curl -s http://localhost:11434/api/tags | head -c 200`
- [ ] **LM Studio responding**: `curl -s http://localhost:1234/v1/models | head -c 200`
- [ ] **NPU Server responding**: `curl -s http://localhost:8000/v1/models | head -c 200`
- [ ] **Gemini API key valid**: Check `agents.toml` for valid Gemini key (primary free provider).
- [ ] **Telegram Bot token valid**: Send `/status` to bot and confirm response.

### 2.5 Disk Space Check

```bash
# Z13 Hub
df -h / | tail -1
# Ensure at least 5GB free for build artifacts and workspace outputs

# Remote nodes
ssh worker@10.0.2.1 'df -h / | tail -1'
ssh worker@10.0.2.2 'wmic logicaldisk get size,freespace,caption'
ssh worker@10.0.1.1 'wmic logicaldisk get size,freespace,caption'
```

- [ ] **Z13**: At least 5GB free
- [ ] **M1 Mac**: At least 1GB free
- [ ] **AYANEO**: At least 1GB free
- [ ] **Acer**: At least 1GB free

### 2.6 Network Connectivity

- [ ] **Tailscale active**: `tailscale status` (Z13 should show 10.0.2.0)
- [ ] **M1 Mac reachable**: `ping -c 3 10.0.2.1`
- [ ] **AYANEO reachable**: `ping -c 3 10.0.2.2`
- [ ] **Acer reachable**: `ping -c 3 10.0.1.1`

---

## 3. Deploy Steps

### Overview

```
Step 1: Build release binary (Z13)
Step 2: Stop current daemon (Z13)
Step 3: Backup old binary + Deploy new binary (Z13)
Step 4: Deploy worker updates (M1 Mac, AYANEO, Acer)
Step 5: Start daemon (Z13)
Step 6: Start workers (M1 Mac, AYANEO, Acer)
Step 7: Verify cluster registration (all 8 nodes)
```

### Step 1: Build Release Binary

```bash
cd C:\Users\worker\Desktop\adreanalai\LLM-Cluster-Project\clawtex-core
cargo build --release 2>&1 | tail -5
# Confirm: Finished `release` profile [optimized] target
# Binary location: target/release/clawtex-core.exe
```

**Estimated time**: 3-8 minutes (first build), 30-90 seconds (incremental).

### Step 2: Stop Current Daemon

```bash
# Graceful stop attempt (give 5 seconds for in-flight requests)
taskkill //F //IM clawtex-core.exe
# Wait for process to fully terminate
sleep 3
# Confirm stopped
tasklist | grep -i clawtex || echo "Daemon stopped successfully"
```

**WARNING**: This will immediately stop all:
- Telegram bot responses
- Cron job execution
- Cluster task dispatch
- HTTP API endpoints

Coordinate timing to avoid disrupting in-progress cron jobs. Check the cron schedule in section 7 to pick a safe window.

### Step 3: Backup Old Binary and Deploy New Binary

```bash
# Backup the currently running binary
cp target/release/clawtex-core.exe target/release/clawtex-core.exe.bak

# The new binary is already at target/release/clawtex-core.exe from Step 1
# Verify file size is reasonable (should be ~20-30MB)
ls -la target/release/clawtex-core.exe
```

### Step 4: Deploy Worker Updates to Remote Nodes

The Python lightweight worker (`clawtex-worker.py`) must be pure ASCII -- Windows cp950 cannot decode UTF-8 special characters (em-dash, box-drawing).

```bash
cd C:\Users\worker\Desktop\adreanalai\LLM-Cluster-Project\clawtex-core

# 4a. Deploy to M1 Mac
scp deploy/lightweight-worker/clawtex-worker.py worker@10.0.2.1:~/clawtex-worker.py

# 4b. Deploy to AYANEO
scp deploy/lightweight-worker/clawtex-worker.py worker@10.0.2.2:'C:\Users\worker\clawtex-worker.py'

# 4c. Deploy to Acer
scp deploy/lightweight-worker/clawtex-worker.py worker@10.0.1.1:'C:\Users\worker\clawtex-worker.py'
```

**Verify**: Confirm each SCP reports 100% transfer with no errors.

### Step 5: Start Daemon on Z13

```bash
cd C:\Users\worker\Desktop\adreanalai\LLM-Cluster-Project\clawtex-core

# CRITICAL: --host 0.0.0.0 is a TOP-LEVEL argument, placed BEFORE the daemon subcommand
# Using "daemon --host 0.0.0.0" will fail with: unexpected argument '--host' found
cargo run --release -- --host 0.0.0.0 daemon
```

**Alternative -- background mode (Windows PowerShell)**:
```powershell
Start-Process -WindowStyle Hidden -FilePath "target\release\clawtex-core.exe" -ArgumentList "--host","0.0.0.0","daemon"
```

**Expected startup output**:
```
INFO  Loading config from C:\Users\worker\.clawtex\agents.toml
INFO  Loaded 10 providers
INFO  Loaded XX hands
INFO  Scheduler started with XX cron jobs
INFO  Watchdog monitoring loop started (60s interval)
INFO  Listening on http://0.0.0.0:7878
```

**Wait 10 seconds** for initialization to complete before proceeding.

### Step 6: Start Workers on Each Node

#### 6a. Stop Old Workers (all nodes)

```bash
# M1 Mac
ssh worker@10.0.2.1 'pkill -f clawtex-worker'

# AYANEO
ssh worker@10.0.2.2 'wmic process where "commandline like '\''%%clawtex-worker%%'\''" call terminate'

# Acer
ssh worker@10.0.1.1 'wmic process where "commandline like '\''%%clawtex-worker%%'\''" call terminate'
```

#### 6b. Start New Workers

```bash
# M1 Mac (macOS -- use nohup + &)
ssh worker@10.0.2.1 'nohup /opt/homebrew/bin/python3 ~/clawtex-worker.py --hub http://10.0.2.0:7878 --name m1-mac --port 7879 > ~/clawtex-worker.log 2>&1 &'

# AYANEO (Windows -- must use wmic, Start-Process fails via SSH)
ssh worker@10.0.2.2 'wmic process call create "C:\\Python314\\python.exe C:\\Users\\worker\\clawtex-worker.py --hub http://10.0.2.0:7878 --name ayaneo --port 7880"'

# Acer (Windows -- must use wmic)
# IMPORTANT: Use C:\Python314\python.exe, NOT the WindowsApps python3 (has network sandbox)
ssh worker@10.0.1.1 'wmic process call create "C:\\Python314\\python.exe C:\\Users\\worker\\clawtex-worker.py --hub http://10.0.2.0:7878 --name acer --port 7881"'
```

#### 6c. Mobile Workers

Mobile workers operate in polling mode. Ensure the Clawtex Worker App is running in the foreground on each device:
- **ROG6**: Open Clawtex Worker app, verify hub URL `http://10.0.2.0:7878`
- **Mi Pad**: Open Clawtex Worker app, verify hub URL
- **iPhone**: Open Clawtex Worker app (background limited to ~15min via expo-background-fetch)
- **iPad**: Open Clawtex Worker app, verify hub URL

### Step 7: Verify Cluster Registration

Heartbeat auto-registration sometimes fails silently on AYANEO and Acer. Manually register if needed.

#### 7a. Check Current Workers

```bash
curl -s http://10.0.2.0:7878/cluster/workers \
  -H "Authorization: Bearer your-hub-token-here" | python -m json.tool
```

**Expected**: All 8 workers listed with status "online".

#### 7b. Manual Registration (if auto-registration fails)

```bash
# M1 Mac
curl -X POST http://10.0.2.0:7878/cluster/register \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-hub-token-here" \
  -d '{"name":"m1-mac","url":"http://10.0.2.1:7879","capabilities":["shell","web_search","http_request"],"device_type":"pc"}'

# AYANEO
curl -X POST http://10.0.2.0:7878/cluster/register \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-hub-token-here" \
  -d '{"name":"ayaneo","url":"http://10.0.1.2:7880","capabilities":["shell","web_search","http_request"],"device_type":"pc"}'

# Acer
curl -X POST http://10.0.2.0:7878/cluster/register \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-hub-token-here" \
  -d '{"name":"acer","url":"http://10.0.1.1:7881","capabilities":["shell","web_search","http_request"],"device_type":"pc"}'

# iPad (example mobile)
curl -X POST http://10.0.2.0:7878/cluster/register \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-hub-token-here" \
  -d '{"name":"ipad","url":"http://10.0.2.3:0","capabilities":["web_search","http_request"],"device_type":"mobile"}'

# ROG6
curl -X POST http://10.0.2.0:7878/cluster/register \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-hub-token-here" \
  -d '{"name":"rog6","url":"http://0.0.0.0:0","capabilities":["web_search","http_request"],"device_type":"mobile"}'

# Mi Pad
curl -X POST http://10.0.2.0:7878/cluster/register \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-hub-token-here" \
  -d '{"name":"mipad","url":"http://0.0.0.0:0","capabilities":["web_search","http_request"],"device_type":"mobile"}'

# iPhone
curl -X POST http://10.0.2.0:7878/cluster/register \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-hub-token-here" \
  -d '{"name":"iphone","url":"http://0.0.0.0:0","capabilities":["web_search","http_request"],"device_type":"mobile"}'
```

---

## 4. Health Verification

Run all verification checks within 5 minutes of starting the daemon.

### 4.1 Endpoint Health Checks

```bash
# Basic health endpoint (no auth required)
curl -s http://localhost:7878/health
# Expected: 200 OK with system info

# Prometheus metrics
curl -s http://localhost:7878/metrics \
  -H "Authorization: Bearer your-hub-token-here" | head -20
# Expected: Prometheus text exposition format with counters

# JSON health summary
curl -s http://localhost:7878/metrics/health \
  -H "Authorization: Bearer your-hub-token-here" | python -m json.tool
# Expected: JSON with provider status, worker counts, error rates

# Cluster health
curl -s http://localhost:7878/cluster/health \
  -H "Authorization: Bearer your-hub-token-here" | python -m json.tool
# Expected: JSON with per-worker health status
```

### 4.2 Cluster Status Verification

```bash
# Full cluster status
curl -s http://localhost:7878/cluster/status \
  -H "Authorization: Bearer your-hub-token-here" | python -m json.tool

# Cluster metrics
curl -s http://localhost:7878/cluster/metrics \
  -H "Authorization: Bearer your-hub-token-here" | python -m json.tool
```

**Verify**:
- [ ] All 4 PC workers (z13, m1-mac, ayaneo, acer) show status "online"
- [ ] All 4 mobile workers (rog6, mipad, iphone, ipad) show registered
- [ ] `dispatch_count` is 0 (fresh start)
- [ ] `dispatch_failures` is 0

### 4.3 Worker-Level Health Checks

```bash
# M1 Mac worker
curl -s http://10.0.2.1:7879/health
curl -s http://10.0.2.1:7879/worker/status

# AYANEO worker
curl -s http://10.0.1.2:7880/health
curl -s http://10.0.1.2:7880/worker/status

# Acer worker
curl -s http://10.0.1.1:7881/health
curl -s http://10.0.1.1:7881/worker/status
```

### 4.4 Smoke Tests

Run one task through each major pathway to confirm end-to-end functionality.

#### Smoke Test 1: LLM Routing

```bash
curl -s -X POST http://localhost:7878/llm/route \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-hub-token-here" \
  -d '{"prompt": "Say hello in one sentence.", "provider": "auto"}' | head -c 500
# Expected: A valid LLM response
```

#### Smoke Test 2: Cluster Dispatch (web_search)

```bash
curl -s -X POST http://localhost:7878/cluster/dispatch \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-hub-token-here" \
  -d '{"tool":"web_search","input":{"query":"clawtex test ping"}}' | head -c 500
# Expected: Search results returned from a worker
```

#### Smoke Test 3: Hand Execution (content)

```bash
curl -s -X POST http://localhost:7878/hand/content/run \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-hub-token-here" \
  -d '{"prompt": "Write a 50-word test paragraph about AI agents."}' | head -c 1000
# Expected: Multi-phase hand execution result
```

#### Smoke Test 4: Telegram Bot

Send `/status` to the Clawtex Telegram bot.

**Expected response**: System status summary with provider info, worker counts, and uptime.

#### Smoke Test 5: Tools List

```bash
curl -s http://localhost:7878/tools \
  -H "Authorization: Bearer your-hub-token-here" | python -m json.tool | head -30
# Expected: List of 36 tools
```

#### Smoke Test 6: Hands List

```bash
curl -s http://localhost:7878/hands \
  -H "Authorization: Bearer your-hub-token-here" | python -m json.tool | head -30
# Expected: List of 22 hands
```

#### Smoke Test 7: E-Stop (non-destructive check)

```bash
curl -s http://localhost:7878/estop \
  -H "Authorization: Bearer your-hub-token-here"
# Expected: E-Stop status showing "inactive"
```

### 4.5 Verification Summary

- [ ] `/health` returns 200
- [ ] `/metrics` returns Prometheus format data
- [ ] `/metrics/health` returns JSON health summary
- [ ] `/cluster/health` returns cluster health data
- [ ] `/cluster/workers` shows 8 workers
- [ ] LLM routing returns valid response
- [ ] Cluster dispatch completes successfully
- [ ] Hand execution runs (at least 1 phase)
- [ ] Telegram bot responds to `/status`
- [ ] 36 tools listed
- [ ] 22 hands listed
- [ ] E-Stop is inactive

---

## 5. Rollback Procedure

If critical issues are discovered after deployment, follow this rollback procedure.

### 5.1 Decision Criteria for Rollback

Rollback **immediately** if any of these occur:
- Daemon crashes on startup (panic, segfault)
- Telegram bot completely unresponsive after 2 minutes
- Database corruption detected (SQLite errors in logs)
- All providers returning errors (total LLM outage)
- Cluster dispatch 100% failure rate

Rollback **within 30 minutes** if:
- More than 50% of smoke tests fail
- Cron jobs fail to execute
- Memory leak detected (RAM usage climbing >80% within 10 minutes)

### 5.2 Rollback Steps

```bash
# Step 1: Stop the new daemon
taskkill //F //IM clawtex-core.exe
sleep 3

# Step 2: Restore the old binary
cd C:\Users\worker\Desktop\adreanalai\LLM-Cluster-Project\clawtex-core
cp target/release/clawtex-core.exe.bak target/release/clawtex-core.exe

# Step 3: Restore databases (if corrupted)
# Only do this if database corruption is suspected
BACKUP_DIR="$HOME/.clawtex/backups/YYYYMMDD_HHMMSS"  # Use the actual backup timestamp
cp "$BACKUP_DIR/core.db" "$HOME/.clawtex/core.db"
cp "$BACKUP_DIR/costs.db" "$HOME/.clawtex/costs.db"
cp "$BACKUP_DIR/memory.db" "$HOME/.clawtex/memory.db"
cp "$BACKUP_DIR/knowledge.db" "$HOME/.clawtex/knowledge.db"
cp "$BACKUP_DIR/revenue.db" "$HOME/.clawtex/revenue.db"

# Step 4: Restore config (if changed)
cp "$BACKUP_DIR/agents.toml" "$HOME/.clawtex/agents.toml"
cp -r "$BACKUP_DIR/hands" "$HOME/.clawtex/hands"

# Step 5: Start the old daemon
cargo run --release -- --host 0.0.0.0 daemon

# Step 6: Verify cluster reconnects
# Workers should auto-reconnect via heartbeat within 90 seconds
# If not, manually restart workers (see Step 6 in Deploy Steps)
sleep 90
curl -s http://localhost:7878/cluster/workers \
  -H "Authorization: Bearer your-hub-token-here" | python -m json.tool
```

### 5.3 Post-Rollback Verification

- [ ] Daemon is running on old binary
- [ ] `/health` returns 200
- [ ] Telegram bot responding
- [ ] All workers reconnected (check `/cluster/workers`)
- [ ] Databases intact (no corruption errors in logs)

---

## 6. Post-Deploy Monitoring (First 24h)

### 6.1 Monitoring Schedule

| Time After Deploy | Check | Command |
|-------------------|-------|---------|
| +5 min | All health checks pass | Section 4 above |
| +15 min | First cron job executes (if within window) | Check Telegram for output |
| +30 min | Error rate check | `curl /metrics/health` |
| +1 hour | Cost tracker check | `curl /costs` |
| +2 hours | Worker staleness check | `curl /cluster/workers` |
| +4 hours | Memory usage check | `tasklist /FI "IMAGENAME eq clawtex-core.exe"` |
| +6 hours | Full cluster dispatch test | 6 parallel dispatches (see below) |
| +12 hours | Overnight cron pre-check | Verify daemon still running |
| +24 hours | Full post-deploy review | All metrics, logs, costs |

### 6.2 Error Rate Monitoring

```bash
# Check error rates (repeat every 30 minutes for first 4 hours)
curl -s http://localhost:7878/metrics/health \
  -H "Authorization: Bearer your-hub-token-here" | python -m json.tool

# Check provider rotation (should not show excessive cooldowns)
# Look for "rotation" or "cooldown" in daemon stdout/stderr logs
```

**Alert thresholds**:
- Error rate > 10% in any 30-minute window: **Investigate**
- Error rate > 30%: **Consider rollback**
- Provider in permanent cooldown: **Check API key / quota**

### 6.3 Cost Tracker Monitoring

```bash
curl -s http://localhost:7878/costs \
  -H "Authorization: Bearer your-hub-token-here" | python -m json.tool
```

**Alert thresholds**:
- Single hand execution > $1.00: **Investigate** (possible prompt loop)
- Daily cost > $5.00: **Review cron job outputs**
- Any provider showing costs when it should be free (Gemini, Groq): **Check routing**

### 6.4 Memory and CPU Monitoring

```bash
# Z13 Hub process memory
tasklist /FI "IMAGENAME eq clawtex-core.exe" /FO TABLE

# M1 Mac worker
ssh worker@10.0.2.1 'ps aux | grep clawtex-worker | grep -v grep'

# AYANEO worker
ssh worker@10.0.2.2 'tasklist | findstr python'

# Acer worker
ssh worker@10.0.1.1 'tasklist | findstr python'
```

**Alert thresholds**:
- Hub process > 2GB RAM: **Investigate** (possible memory leak)
- Worker process > 500MB RAM: **Investigate**
- CPU sustained > 80% for 10+ minutes without active tasks: **Investigate**

### 6.5 Cluster Dispatch Stress Test (at +6 hours)

```bash
# 6 parallel dispatches to verify load distribution
for i in 1 2 3 4 5 6; do
  curl -s -X POST http://10.0.2.0:7878/cluster/dispatch \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer your-hub-token-here" \
    -d "{\"tool\":\"web_search\",\"input\":{\"query\":\"monitoring test query $i\"}}" &
done
wait
echo "All dispatches completed"
```

**Verify**: Tasks should distribute across workers via inflight-aware routing (`effective_load = cpu_load + (inflight * 0.15)`).

### 6.6 Provider Rotation Check

```bash
# Check if any providers are in cooldown
curl -s http://localhost:7878/metrics \
  -H "Authorization: Bearer your-hub-token-here" | grep -i "cooldown\|rotation\|rate_limit"
```

**Expected**: All providers should have base_cooldown of 15s and max_cooldown of 120s. If Gemini shows 600s cooldown, the rotation tuning was not applied.

---

## 7. Cron Job Verification

### 7.1 Active Cron Schedule

| Time (UTC+8) | Name | Hand | Frequency |
|--------------|------|------|-----------|
| 01:00 | nightly-review-agents | review_agents | Daily |
| 02:00 | nightly-self-evolve | self_evolve | Daily |
| 03:00 | nightly-cluster-evolve | cluster_evolve | Daily |
| 04:00 (Sun) | weekly-prompt-evolve | prompt_evolve | Weekly (Sunday) |
| 08:00 | daily-content | content | Daily |
| 09:00 | daily-freelancer | freelancer | Daily |
| 09:00 (Wed) | weekly-market-intel | market_intel | Weekly (Wednesday) |
| 10:00 (Mon) | weekly-leads | lead | Weekly (Monday) |
| 11:00 (Tue/Thu) | biweekly-seo-content | seo_content | Twice weekly |
| 14:00 | daily-researcher | researcher | Daily |
| 15:00 (Mon/Wed/Fri) | triweekly-outreach | outreach | Three times weekly |

### 7.2 Verify Cron Jobs via Telegram

Send `/cron` or `/cron list` to the Telegram bot. Confirm all 11 jobs are listed.

### 7.3 Safe Deploy Windows

To minimize cron disruption, deploy during these windows:
- **Best**: 04:00 - 07:59 (Mon-Sat) -- no cron jobs running
- **Good**: 12:00 - 13:59 -- gap between morning and afternoon jobs
- **Avoid**: 01:00 - 03:00 (evolution jobs), 08:00 - 11:00 (revenue jobs)

---

## 8. Known Issues and Workarounds

### 8.1 TOML Parse Warnings

| Hand | Issue | Status |
|------|-------|--------|
| `prompt_evolve` | Triple-quote string syntax issue in TOML | TODO fix |
| `report` | Map vs string type mismatch in `tool_calls` | TODO fix |

**Impact**: Warnings only -- hands still execute but may have unexpected behavior. Monitor outputs.

### 8.2 Worker Auto-Registration Failures

**Symptom**: AYANEO and Acer workers start but do not appear in `/cluster/workers`.
**Cause**: Heartbeat auto-registration silently fails.
**Workaround**: Always manually POST to `/cluster/register` after starting workers (see Step 7b).

### 8.3 Windows Python Path

| Node | Correct Python | Wrong Python |
|------|----------------|--------------|
| Acer | `C:\Python314\python.exe` | `python3` (WindowsApps -- has network sandbox) |
| AYANEO | `C:\Python314\python.exe` | `python3` (may not exist) |

### 8.4 SSH Background Process on Windows

**Rule**: On AYANEO and Acer, always use `wmic process call create` for background processes. PowerShell `Start-Process` fails when invoked via SSH.

### 8.5 Qwen Model Language Issue

**Symptom**: Qwen model responds in Simplified Chinese despite Traditional Chinese instructions.
**Workaround**: Use non-Qwen models for Traditional Chinese content, or add explicit language enforcement in prompts.

### 8.6 Codex CLI Tool Limitation

**CRITICAL**: The Codex CLI (chatgpt provider) handles tools internally. This means `tool_calls=0` in the response, resulting in no workspace writes and no cluster dispatch. Do not rely on the chatgpt provider for tasks requiring tool execution.

### 8.7 Instant Overflow on Windows

`Instant::now() - Duration` can panic on Windows due to underflow. All such calls should use `checked_sub()`. If a panic occurs with "time went backwards" or similar, this is the cause.

### 8.8 M1 Mac Xcode

The M1 Mac requires Xcode license acceptance for system Python and iOS builds:
```bash
sudo xcodebuild -license accept
xcodebuild -runFirstLaunch
```
This requires physical access to the Mac.

---

## 9. Emergency Procedures

### 9.1 Emergency Stop (E-Stop)

If the system must be halted immediately (e.g., runaway costs, security breach, rogue agent behavior):

```bash
# Activate E-Stop via API
curl -X POST http://localhost:7878/estop \
  -H "Authorization: Bearer your-hub-token-here"

# Or via Telegram: send /estop to the bot

# Verify E-Stop is active
curl -s http://localhost:7878/estop \
  -H "Authorization: Bearer your-hub-token-here"
```

**E-Stop effect**: All agent execution, hand runs, and tool calls are blocked. The daemon stays running but refuses work.

**To reset E-Stop**:
```bash
curl -X DELETE http://localhost:7878/estop \
  -H "Authorization: Bearer your-hub-token-here"
```

### 9.2 Full System Shutdown

```bash
# 1. Stop Hub daemon
taskkill //F //IM clawtex-core.exe

# 2. Stop all PC workers
ssh worker@10.0.2.1 'pkill -f clawtex-worker'
ssh worker@10.0.2.2 'wmic process where "commandline like '\''%%clawtex-worker%%'\''" call terminate'
ssh worker@10.0.1.1 'wmic process where "commandline like '\''%%clawtex-worker%%'\''" call terminate'

# 3. Mobile workers: close the app on each device
```

### 9.3 Hub Crash Recovery

If the daemon crashes unexpectedly:

```bash
# 1. Check for crash artifacts
ls -la C:\Users\worker\Desktop\adreanalai\LLM-Cluster-Project\clawtex-core\target\release\

# 2. Check WAL file integrity
ls -la "$HOME/.clawtex/core.db-wal"
# If WAL is very large (>100MB), SQLite may need recovery

# 3. Restart daemon
cd C:\Users\worker\Desktop\adreanalai\LLM-Cluster-Project\clawtex-core
cargo run --release -- --host 0.0.0.0 daemon

# 4. Workers will auto-reconnect via heartbeat within 90 seconds
# Monitor: curl /cluster/workers every 30 seconds
```

### 9.4 Database Corruption Recovery

```bash
# 1. Stop daemon
taskkill //F //IM clawtex-core.exe

# 2. Try SQLite integrity check
sqlite3 "$HOME/.clawtex/core.db" "PRAGMA integrity_check;"
sqlite3 "$HOME/.clawtex/costs.db" "PRAGMA integrity_check;"
sqlite3 "$HOME/.clawtex/memory.db" "PRAGMA integrity_check;"

# 3. If integrity check fails, restore from backup
BACKUP_DIR="$HOME/.clawtex/backups/LATEST_TIMESTAMP"
cp "$BACKUP_DIR/core.db" "$HOME/.clawtex/core.db"
# Repeat for affected databases

# 4. Restart daemon
cargo run --release -- --host 0.0.0.0 daemon
```

### 9.5 Single Worker Down

If one worker goes offline:

```bash
# 1. Check worker status
curl -s http://localhost:7878/cluster/workers \
  -H "Authorization: Bearer your-hub-token-here" | python -m json.tool

# 2. The hub automatically marks workers offline after 90 seconds of missed heartbeats
# 3. Tasks are automatically routed to remaining workers via inflight-aware routing
# 4. To recover, restart the worker (see Step 6 in Deploy Steps)
```

**Impact**: Minimal -- tasks auto-redistribute to remaining workers. Only a concern if the failed worker is the only one with a required capability.

### 9.6 Provider API Key Expired / Quota Exceeded

```bash
# 1. Check which provider is failing
curl -s http://localhost:7878/metrics/health \
  -H "Authorization: Bearer your-hub-token-here" | python -m json.tool

# 2. The rotation system automatically marks rate-limited providers with cooldown
#    Keywords detected: "usage limit", "quota", "exceeded your"
#    Cooldown: 15s base, 120s max

# 3. If a provider is permanently down, update agents.toml and restart daemon
```

### 9.7 Telegram Bot Unresponsive

```bash
# 1. Check if daemon is running
tasklist | grep -i clawtex

# 2. If running, check Telegram webhook/polling
# The bot uses long-polling, so if the daemon is running it should be connected

# 3. Restart daemon if needed
taskkill //F //IM clawtex-core.exe
cargo run --release -- --host 0.0.0.0 daemon
```

### 9.8 Runaway Cost Alert

If costs are accumulating unexpectedly:

```bash
# 1. Activate E-Stop immediately
curl -X POST http://localhost:7878/estop \
  -H "Authorization: Bearer your-hub-token-here"

# 2. Check cost records
curl -s http://localhost:7878/costs \
  -H "Authorization: Bearer your-hub-token-here" | python -m json.tool

# 3. Identify the source (which agent/hand is generating costs)
# 4. If it's a specific cron job, remove it:
#    Send "/cron remove <job_name>" to Telegram bot

# 5. Reset E-Stop after fixing the issue
curl -X DELETE http://localhost:7878/estop \
  -H "Authorization: Bearer your-hub-token-here"
```

### 9.9 Security Incident Response

If a prompt injection, unauthorized access, or data leak is suspected:

```bash
# 1. Activate E-Stop
curl -X POST http://localhost:7878/estop \
  -H "Authorization: Bearer your-hub-token-here"

# 2. Rotate the hub auth token
#    Edit ~/.clawtex/agents.toml: change the Bearer token
#    Restart daemon + update all workers with new token

# 3. Check injection guard logs
#    InjectionGuard has 8 regex patterns detecting:
#    system_override, role_switch, encoding_bypass, delimiter_injection,
#    prompt_leak, jailbreak, instruction_smuggle, system_inject

# 4. Review audit logs if enabled

# 5. If .secret_key may be compromised:
#    - Generate new key
#    - Re-encrypt all enc2: secrets in agents.toml
#    - Restart daemon
```

---

## Appendix A: API Endpoint Reference

All endpoints require `Authorization: Bearer your-hub-token-here` header except `/health` and `/dashboard`.

### Core Endpoints

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/health` | Basic health check (no auth) |
| GET | `/dashboard` | HTML dashboard (no auth) |
| GET | `/metrics` | Prometheus text exposition |
| GET | `/metrics/health` | JSON health summary |
| POST | `/llm/route` | Route prompt to LLM provider |
| POST | `/estop` | Activate emergency stop |
| DELETE | `/estop` | Reset emergency stop |
| GET | `/estop` | Check E-Stop status |
| GET | `/tools` | List all registered tools |
| GET | `/hands` | List all loaded hands |
| GET | `/costs` | Cost summary |
| GET | `/revenue` | Revenue summary |
| GET | `/workspace/files` | List workspace output files |

### Task Endpoints

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/task` | Add a new task |
| POST | `/task/:id/run` | Run a specific task |
| GET | `/task/history` | Task execution history |
| POST | `/agent/:name/run` | Run a named agent |
| POST | `/hand/:name/run` | Execute a hand workflow |

### Cluster Endpoints

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/cluster/status` | Full cluster status |
| POST | `/cluster/register` | Worker self-registration |
| POST | `/cluster/heartbeat` | Worker heartbeat (cpu_load) |
| GET | `/cluster/workers` | List online workers |
| POST | `/cluster/dispatch` | Dispatch tool to a worker |
| GET | `/cluster/metrics` | Cluster performance data |
| GET | `/cluster/metrics/:worker` | Per-worker stats |
| GET | `/cluster/poll` | Mobile worker polls for tasks |
| POST | `/cluster/result` | Mobile worker submits result |
| GET | `/cluster/health` | Cluster health overview |

### Streaming / Gateway Endpoints

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/stream/agent/:name` | SSE agent streaming |
| GET | `/ws/agent/:name` | WebSocket agent connection |
| POST | `/agent/think` | Agent think (gateway) |
| GET | `/trajectories` | List trajectory logs |
| GET | `/trajectories/stats` | Trajectory statistics |

### Telegram Bot Commands

| Command | Purpose |
|---------|---------|
| `/status` | System status summary |
| `/cron` or `/cron list` | List scheduled jobs |
| `/cron add <schedule> <action>` | Add a cron job |
| `/cron remove <id>` | Remove a cron job |
| `/estop` | Emergency stop |
| `/dashboard` | Link to web dashboard |
| `/hand <name> <prompt>` | Execute a hand |

---

## Appendix B: Quick Reference Card

### Restart Daemon (One-liner)

```bash
taskkill //F //IM clawtex-core.exe && cd C:\Users\worker\Desktop\adreanalai\LLM-Cluster-Project\clawtex-core && cargo run --release -- --host 0.0.0.0 daemon
```

### Restart All Workers (One-liner per node)

```bash
ssh worker@10.0.2.1 'pkill -f clawtex-worker; nohup /opt/homebrew/bin/python3 ~/clawtex-worker.py --hub http://10.0.2.0:7878 --name m1-mac --port 7879 > ~/clawtex-worker.log 2>&1 &'
ssh worker@10.0.2.2 'wmic process where "commandline like '\''%%clawtex-worker%%'\''" call terminate & wmic process call create "C:\\Python314\\python.exe C:\\Users\\worker\\clawtex-worker.py --hub http://10.0.2.0:7878 --name ayaneo --port 7880"'
ssh worker@10.0.1.1 'wmic process where "commandline like '\''%%clawtex-worker%%'\''" call terminate & wmic process call create "C:\\Python314\\python.exe C:\\Users\\worker\\clawtex-worker.py --hub http://10.0.2.0:7878 --name acer --port 7881"'
```

### Check Everything (5-second health check)

```bash
echo "=== Hub ===" && curl -s http://localhost:7878/health && echo ""
echo "=== Workers ===" && curl -s http://localhost:7878/cluster/workers -H "Authorization: Bearer your-hub-token-here" | python -m json.tool
echo "=== Metrics ===" && curl -s http://localhost:7878/metrics/health -H "Authorization: Bearer your-hub-token-here" | python -m json.tool
```

### Backup Script (run daily)

```bash
#!/bin/bash
BACKUP_DIR="$HOME/.clawtex/backups/$(date +%Y%m%d_%H%M%S)"
mkdir -p "$BACKUP_DIR"
for db in core.db costs.db memory.db knowledge.db revenue.db trajectories.db; do
  cp "$HOME/.clawtex/$db" "$BACKUP_DIR/$db" 2>/dev/null
done
cp "$HOME/.clawtex/.secret_key" "$BACKUP_DIR/.secret_key"
cp "$HOME/.clawtex/agents.toml" "$BACKUP_DIR/agents.toml"
cp -r "$HOME/.clawtex/hands" "$BACKUP_DIR/hands"
# Prune backups older than 7 days
find "$HOME/.clawtex/backups" -maxdepth 1 -mtime +7 -type d -exec rm -rf {} \;
echo "Backup complete: $BACKUP_DIR"
# Off-site copy to Acer
scp -r "$BACKUP_DIR" worker@10.0.1.1:'C:\Users\worker\clawtex-backups\' 2>/dev/null
```

---

> **Document maintained by**: Clawtex operations team
> **Last updated**: 2026-03-17
> **Related documents**:
> - [Disaster Recovery Plan](archive/2026-03-05-disaster-recovery-plan.md)
> - [Cluster Deployment Runbook](../../.claude/projects/memory/cluster-deployment-runbook.md)
> - [Master Plan 2026](../../.claude/projects/memory/master-plan-2026.md)
> - [Tech Backlog](tech-backlog.md)
