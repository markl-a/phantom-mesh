# Phantom Mesh — Deployment Guide

This guide walks you through deploying Phantom Mesh from a single local node all the way to a full 9-device mesh with a 24/7 cloud node and Telegram mobile access.

---

## Table of Contents

1. [Prerequisites](#prerequisites)
2. [Quick Start — Single Node](#quick-start--single-node)
3. [Network Topology](#network-topology)
4. [Step-by-Step Multi-Node Setup](#step-by-step-multi-node-setup)
   - [Step 1: Coordinator Node](#step-1-coordinator-node)
   - [Step 2: Tailscale Mesh VPN](#step-2-tailscale-mesh-vpn)
   - [Step 3: Local Worker Nodes (Acer / AYANEO)](#step-3-local-worker-nodes-acer--ayaneo)
   - [Step 4: Form the Local Cluster](#step-4-form-the-local-cluster)
   - [Step 5: Remote Worker Node (M1 Mac)](#step-5-remote-worker-node-m1-mac)
   - [Step 5.5: Cloud Node (Oracle / GCP)](#step-55-cloud-node-oracle--gcp)
   - [Step 6: Mobile Access via Telegram](#step-6-mobile-access-via-telegram)
   - [Step 7: Full Mesh Verification](#step-7-full-mesh-verification)
5. [Config Templates](#config-templates)
6. [Telegram Bot Setup](#telegram-bot-setup)
7. [Troubleshooting](#troubleshooting)

---

## Prerequisites

| Item | Where to get it | Cost |
|------|----------------|------|
| Tailscale account | https://tailscale.com | Free (up to 100 devices) |
| Anthropic API key | https://console.anthropic.com | Pay-per-use |
| Telegram bot token | @BotFather on Telegram | Free |
| Oracle Cloud account | https://cloud.oracle.com | Free tier (Always Free VM) |
| Rust toolchain | https://rustup.rs | Free (for building from source) |

---

## Quick Start — Single Node

Get a single node running in under 5 minutes.

```bash
# 1. Build the daemon
cargo build --release -p phantom-mesh-daemon

# 2. Copy the example config
#   macOS
cp agents.toml.example ~/Library/Application\ Support/ai.phantommesh.app/agents.toml
#   Linux
cp agents.toml.example ~/.config/phantom-mesh/agents.toml

# 3. Edit the config — set at least one provider API key
$EDITOR ~/.config/phantom-mesh/agents.toml

# 4. Start the daemon
./target/release/phantom-mesh daemon

# 5. Verify it's running
curl http://localhost:7878/health
# → {"status":"ok"}
```

---

## Network Topology

```
                    ┌─────────── Tailscale Mesh VPN ───────────┐
                    │                                           │
  Local LAN         │   Remote                                  │   Cloud
  ┌──────────┐      │   ┌──────────┐                           │   ┌──────────────┐
  │ Z13      │◄────►│◄─►│ M1 Mac   │                           │◄─►│ Oracle Cloud  │
  │ coord    │      │   │ worker   │                           │   │ cloud node   │
  ├──────────┤      │   └──────────┘                           │   │ Telegram bot │
  │ Acer     │◄────►│                                           │   └──────┬───────┘
  │ worker   │      │   ┌───────────────────────────────────┐  │          │
  ├──────────┤      │   │ Mobile (Telegram clients)         │  │          │
  │ AYANEO   │◄────►│   │  ROG Phone  iPhone  iPad  MiPad   │──┼──────────┘
  │ worker   │      │   └───────────────────────────────────┘  │  Telegram API
  └──────────┘      └───────────────────────────────────────────┘
```

**Node roles:**

| Node | Role | Tailscale IP |
|------|------|-------------|
| Z13 (Windows) | Coordinator — main dev hub | 100.x.x.1 |
| Acer (Windows) | Worker — storage + backup inference | 100.x.x.2 |
| AYANEO (Windows) | Worker — edge GPU inference | 100.x.x.3 |
| M1 Mac (macOS) | Worker — lightweight inference | 100.x.x.4 |
| Oracle Cloud VM (Linux ARM) | Cloud node — 24/7, Telegram bot | 100.x.x.5 |
| ROG Phone / iPhone / iPad / MiPad | Mobile — Telegram clients only | 100.x.x.6-9 |

---

## Step-by-Step Multi-Node Setup

Follow these steps in order. Verify each step passes before proceeding.

---

### Step 1: Coordinator Node

Set up the main coordinator (Z13 or Mac) as a standalone daemon first.

1. Build and install the daemon binary on the coordinator machine.

2. Create the config directory and copy the coordinator template:

   ```bash
   # macOS
   mkdir -p ~/Library/Application\ Support/ai.phantommesh.app
   cp configs/agents.coordinator.toml \
      ~/Library/Application\ Support/ai.phantommesh.app/agents.toml

   # Windows (PowerShell)
   New-Item -ItemType Directory -Force "$env:APPDATA\ai.phantommesh.app"
   Copy-Item configs\agents.coordinator.toml `
             "$env:APPDATA\ai.phantommesh.app\agents.toml"
   ```

3. Edit the config — fill in your Anthropic API key and (optionally) Brave search key.

4. Start the daemon:
   ```bash
   # macOS / Linux
   ./phantom-mesh daemon

   # Windows
   .\phantom-mesh.exe daemon
   ```

**Acceptance criteria:**
```
curl http://localhost:7878/health  →  {"status":"ok"}
```

---

### Step 2: Tailscale Mesh VPN

Install Tailscale on every machine to give them stable, routable IPs across networks.

1. Install Tailscale:
   - Windows / macOS: download from https://tailscale.com/download
   - Linux: `curl -fsSL https://tailscale.com/install.sh | sh`
   - Android / iOS: install the Tailscale app from the app store

2. Log in on each device with the same Tailscale account:
   ```bash
   tailscale up
   ```

3. Note down each device's Tailscale IP:
   ```bash
   tailscale status
   ```

4. Update `cluster.peers` in each node's `agents.toml` with the real IPs.

**Acceptance criteria:**
```bash
tailscale ping acer     # replies with latency
tailscale ping ayaneo   # replies
tailscale ping m1-mac   # replies
tailscale status        # all devices show "online"
```

---

### Step 3: Local Worker Nodes (Acer / AYANEO)

Deploy the daemon to each local Windows worker.

1. Copy the daemon binary to each machine (via LAN share or SCP over Tailscale):
   ```bash
   scp phantom-mesh.exe user@acer-tailscale-ip:C:/phantom-mesh/
   ```

2. Copy the worker config template:
   ```bash
   scp configs/agents.worker.toml \
       user@acer-tailscale-ip:"%APPDATA%/ai.phantommesh.app/agents.toml"
   ```

3. On each worker machine, edit `agents.toml`:
   - Set `node_name` to the machine name (e.g. `"acer"` or `"ayaneo"`)
   - Update `cluster.peers` with the coordinator's Tailscale IP

4. (Optional) Install Ollama for local inference:
   ```bash
   # On Windows: download from https://ollama.ai
   # Pull a model
   ollama pull qwen2.5-coder:14b
   ```

5. Start the daemon on each worker:
   ```batch
   .\phantom-mesh.exe daemon
   ```

**Acceptance criteria:**
```bash
curl http://<acer-tailscale-ip>:7878/health   # → {"status":"ok"}
curl http://<ayaneo-tailscale-ip>:7878/health # → {"status":"ok"}
# If Ollama installed:
curl http://<acer-ip>:11434/api/tags          # → lists models
```

---

### Step 4: Form the Local Cluster

Connect the 3 local nodes (Z13 + Acer + AYANEO) into a cluster.

1. On each node, edit `agents.toml` and add the `[cluster]` section with all peer IPs. Use the template in `configs/agents.coordinator.toml` as reference.

2. Choose a strong shared secret and use the same value on all nodes:
   ```bash
   # Generate a random secret
   openssl rand -hex 32
   ```

3. Set `cluster.cluster_secret` to this value in every node's config.

4. Restart the daemon on all 3 nodes.

**Acceptance criteria:**
```
Z13 → Acer:   rpc.ping → pong
Z13 → AYANEO: rpc.ping → pong
Acer → Z13:   rpc.ping → pong (bidirectional)
Cluster shows 3 nodes online
Coordinator election completed
```

---

### Step 5: Remote Worker Node (M1 Mac)

Add the M1 Mac as a remote worker over Tailscale.

1. On the Mac, build the daemon:
   ```bash
   git clone <repo-url> phantom-mesh && cd phantom-mesh
   cargo build --release -p phantom-mesh-daemon
   ```

2. Copy the worker config:
   ```bash
   mkdir -p ~/Library/Application\ Support/ai.phantommesh.app
   cp configs/agents.worker.toml \
      ~/Library/Application\ Support/ai.phantommesh.app/agents.toml
   ```

3. Edit `agents.toml`:
   - Set `node_name = "m1-mac"`
   - Add coordinator and cloud IPs to `cluster.peers`
   - Set `cluster.cluster_secret` to the shared secret

4. Start the daemon.

**Acceptance criteria:**
```bash
curl http://<mac-tailscale-ip>:7878/health  # → {"status":"ok"}
# Z13 → Mac: rpc.ping → pong (cross-network)
# Cluster shows 4 nodes online
```

---

### Step 5.5: Cloud Node (Oracle / GCP)

Set up the 24/7 always-on cloud node. This node also runs the Telegram bot.

#### Oracle Cloud Free Tier Setup

1. Sign up at https://cloud.oracle.com (credit card required for verification; charges nothing under the Always Free tier).

2. Create an **Ampere A1 ARM** instance:
   - Shape: `VM.Standard.A1.Flex`
   - OCPU: 4, RAM: 24 GB (maximum Always Free allocation)
   - OS: Ubuntu 22.04 (aarch64)
   - Add your SSH public key

3. Open port 7878 in the Oracle security list (Networking > VCN > Security Lists).

#### Deploy the daemon

```bash
# On your dev machine: cross-compile for aarch64 Linux
rustup target add aarch64-unknown-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu -p phantom-mesh-daemon

# Copy binary to Oracle VM
scp target/aarch64-unknown-linux-gnu/release/phantom-mesh \
    ubuntu@<oracle-public-ip>:~/phantom-mesh
```

Alternatively, build directly on the VM:
```bash
ssh ubuntu@<oracle-public-ip>
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
git clone <repo-url> phantom-mesh && cd phantom-mesh
cargo build --release -p phantom-mesh-daemon
```

#### Configure the cloud node

```bash
ssh ubuntu@<oracle-public-ip>
mkdir -p ~/.config/phantom-mesh
cp configs/agents.cloud.toml ~/.config/phantom-mesh/agents.toml
nano ~/.config/phantom-mesh/agents.toml
# - Set cluster.peers with Tailscale IPs of all other nodes
# - Set cluster.cluster_secret
```

#### Set environment variables

Create `/etc/phantom-mesh.env`:
```
ANTHROPIC_API_KEY=sk-ant-YOUR_KEY
TELEGRAM_BOT_TOKEN=123456:YOUR_BOT_TOKEN
```

#### Install as a systemd service

```bash
sudo tee /etc/systemd/system/phantom-mesh.service > /dev/null <<EOF
[Unit]
Description=Phantom Mesh Daemon
After=network-online.target tailscaled.service
Wants=network-online.target

[Service]
Type=simple
User=ubuntu
EnvironmentFile=/etc/phantom-mesh.env
ExecStart=/home/ubuntu/phantom-mesh daemon
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable --now phantom-mesh
```

#### Install Tailscale on Oracle VM

```bash
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up
# Note the Tailscale IP — update cluster.peers on all other nodes
```

**Acceptance criteria:**
```bash
ssh oracle-vm                                          # can SSH in
curl http://<oracle-tailscale-ip>:7878/health         # → {"status":"ok"}
# Z13 → Oracle: rpc.ping → pong
# Cluster shows 5 daemon nodes online
# Turn off Z13 → send Telegram message → Oracle node replies (24/7 verification)
```

---

### Step 6: Mobile Access via Telegram

The Telegram bot on the cloud node provides mobile access for all devices.

1. Ensure the cloud node is running with `[telegram]` configured (see [Telegram Bot Setup](#telegram-bot-setup) below).

2. On each mobile device, open Telegram and find your bot by username.

3. Send a test message.

**Acceptance criteria:**
```
ROG Phone → "你好" → reply received
iPhone    → "你好" → reply received
iPad      → same
MiPad     → same
```

---

### Step 7: Full Mesh Verification

Run a full end-to-end test with no computer intervention:

1. From iPhone Telegram, send: `"幫我研究 2026 年 AI agent 市場趨勢，整理成報告"`
2. Oracle cloud node receives the task via Telegram
3. Task is delegated to a GPU worker (AYANEO or Mac) for inference
4. `web_search` tool executes the research
5. Results are synthesized and returned to Telegram
6. Entire flow completes without touching any computer

Expected `tailscale status`:
```
z13        100.x.x.1   online   Windows    daemon (coordinator)
acer       100.x.x.2   online   Windows    daemon (worker)
ayaneo     100.x.x.3   online   Windows    daemon (worker, GPU)
m1-mac     100.x.x.4   online   macOS      daemon (worker)
oracle-vm  100.x.x.5   online   Linux ARM  daemon (cloud, 24/7)
rog-phone  100.x.x.6   online   Android    Telegram client
iphone     100.x.x.7   online   iOS        Telegram client
ipad       100.x.x.8   online   iOS        Telegram client
mipad      100.x.x.9   online   Android    Telegram client
```

---

## Config Templates

Three ready-to-use templates are provided in `configs/`:

| File | For | Key settings |
|------|-----|-------------|
| `configs/agents.coordinator.toml` | Z13 / Mac (main dev machine) | All 13 tools, cluster peers, Anthropic primary |
| `configs/agents.cloud.toml` | Oracle Cloud / GCP VM | Telegram bot enabled, `host = "0.0.0.0"`, Haiku model |
| `configs/agents.worker.toml` | Acer / AYANEO / Linux workers | Ollama primary + Anthropic fallback, no git tools |

All templates include `[cluster]` sections. Replace `100.x.x.*` placeholders with real Tailscale IPs and set a shared `cluster_secret` on all nodes.

---

## Telegram Bot Setup

1. Open Telegram and message **@BotFather**.

2. Send `/newbot` and follow the prompts to choose a name and username.

3. BotFather will give you a token like `123456789:ABCdef...`

4. Set the token as an environment variable on the cloud node:
   ```bash
   # Add to /etc/phantom-mesh.env
   TELEGRAM_BOT_TOKEN=123456789:ABCdef...
   ```

5. Uncomment the `[telegram]` section in the cloud node's `agents.toml`:
   ```toml
   [telegram]
   bot_token_env = "TELEGRAM_BOT_TOKEN"
   allowed_users = []         # empty = public; or list numeric user IDs for private bot
   agent = "master"
   ```

6. Restart the daemon. The bot is now live.

**Tip:** To find your Telegram user ID, message @userinfobot. Add your ID to `allowed_users` to make the bot private:
```toml
allowed_users = [123456789]   # your numeric user ID
```

---

## Troubleshooting

### Daemon won't start

```bash
# Check logs
journalctl -u phantom-mesh -f   # Linux systemd
# or run interactively:
./phantom-mesh daemon 2>&1 | tee daemon.log
```

Common causes:
- Missing or invalid API key in config
- Port 7878 already in use: `lsof -i :7878`
- Config file not found — check the platform-specific path

### Health check fails

```bash
curl -v http://localhost:7878/health
```

- If connection refused: daemon is not running
- If 401/403: check `cluster_secret` matches on all nodes
- If timeout: firewall blocking port 7878

### Nodes can't reach each other over Tailscale

```bash
tailscale status           # check all nodes show "online"
tailscale ping <node-name> # test connectivity
```

- Ensure Tailscale is running on both nodes: `sudo systemctl status tailscaled`
- Check that port 7878 is open in the cloud provider's firewall/security group
- On Oracle Cloud: verify the VCN security list has an ingress rule for TCP 7878

### Telegram bot not responding

- Check the daemon is running on the cloud node: `curl http://localhost:7878/health`
- Verify `TELEGRAM_BOT_TOKEN` is set: `echo $TELEGRAM_BOT_TOKEN`
- Check daemon logs for Telegram-related errors
- Ensure `bot_token_env` in `[telegram]` matches the env var name exactly

### Ollama not working on worker nodes

```bash
# Check Ollama is running
curl http://localhost:11434/api/tags

# Pull the model if missing
ollama pull qwen2.5-coder:14b

# Check provider config matches the model name exactly
```

### Out of memory on Oracle Free Tier ARM VM

The Ampere A1 Always Free allocation is 4 OCPU / 24 GB RAM — use a smaller model if needed:

```toml
[providers.ollama]
default_model = "qwen2.5-coder:7b"   # lighter than 14b
```

---

## Estimated Setup Time

| Step | Time |
|------|------|
| Step 1 — Coordinator | ~30 min |
| Step 2 — Tailscale | ~30 min |
| Step 3 — Local workers | ~45 min |
| Step 4 — Local cluster | ~30 min |
| Step 5 — M1 Mac | ~45 min |
| Step 5.5 — Oracle Cloud | ~1–2 hr (includes account setup + deploy) |
| Step 6 — Telegram mobile | ~15 min |
| Step 7 — Full verification | ~15 min |
| **Total** | **~4–5 hr** |
