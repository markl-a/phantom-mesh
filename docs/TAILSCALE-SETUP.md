# Tailscale Setup for Phantom Mesh

This guide connects multiple devices running the phantom-mesh daemon into a single compute mesh over Tailscale.

---

## Overview

Tailscale creates a WireGuard-based VPN mesh between your devices. Each device gets a stable `100.x.x.x` IP that works regardless of NAT, firewalls, or network changes. Phantom Mesh uses these IPs so that daemon nodes can call each other's HTTP APIs directly.

**Why Tailscale and not a plain VPN or port forwarding?**

- No port forwarding required on home routers or cloud providers
- Stable IPs survive network changes (laptop moving between Wi-Fi networks, phone switching to LTE)
- Free for up to 100 devices on a single account
- Encrypted transport at the network layer — phantom-mesh HMAC auth adds an application-level second factor

---

## Topology Options

Choose the setup that matches your hardware:

| Topology | Devices | What you get |
|----------|---------|--------------|
| **Minimal** | Mac + GCP/Oracle Linux VM | Dev machine + cloud node; 24/7 uptime via cloud |
| **Mobile access** | Mac + cloud VM + iPhone | Telegram bot on cloud; control mesh from phone |
| **Full mesh** | All devices | Distribute tasks to any node; full resilience |

The steps below cover the most common case: one Mac as coordinator, one Linux cloud VM as the always-on node, and optionally iPhone via Telegram.

---

## Step 1: Install Tailscale on All Devices

**macOS:**
```bash
brew install tailscale
# or download the app from https://tailscale.com/download
sudo tailscaled &   # if installed via brew
tailscale up
```

**Linux (GCP / Oracle Cloud / any Debian-based distro):**
```bash
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up
```

**iOS / Android:**
Install the Tailscale app from the App Store or Google Play, sign in with the same account.

Sign in to the same Tailscale account on every device. All devices on the same account form a single network.

---

## Step 2: Get Tailscale IPs

On each device after running `tailscale up`:

```bash
tailscale ip -4
```

Or list all devices in your network:

```bash
tailscale status
```

Note the `100.x.x.x` address for each machine — you will put these into `agents.toml`.

Example output:
```
100.101.1.1   mac-coordinator  markl@  macOS   -
100.101.1.2   gcp-worker       markl@  linux   -
```

---

## Step 3: Configure agents.toml on Each Node

Each node needs a `[cluster]` section that lists the Tailscale IPs of its peers and a shared secret for authentication.

First, generate a strong shared secret (run this once, use the same value on all nodes):

```bash
openssl rand -hex 32
```

**Mac coordinator** (`~/Library/Application Support/ai.phantommesh.app/agents.toml` on macOS, or `~/.config/phantom-mesh/agents.toml` on Linux):

```toml
[core]
host = "0.0.0.0"      # listen on Tailscale interface so peers can reach it
port = 7878
node_name = "mac-coordinator"

[providers.anthropic]
type = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"
default_model = "claude-sonnet-4-6"

[cluster]
node_name = "mac-coordinator"
cluster_secret = "your-shared-secret"   # output from openssl rand -hex 32
peers = ["http://100.x.y.z:7878"]       # GCP node Tailscale IP

[agent.master]
provider = "anthropic"
model = "claude-sonnet-4-6"
tools = ["shell", "file_read", "file_write", "file_edit", "content_search",
         "glob_search", "web_search", "memory_store", "memory_recall",
         "git_status", "git_diff", "git_commit", "git_log"]
instructions = "You are a senior software engineer AI assistant. ALWAYS use tools to accomplish tasks."
```

**GCP / cloud worker** (`~/.config/phantom-mesh/agents.toml`):

```toml
[core]
host = "0.0.0.0"      # must listen on 0.0.0.0 to accept cluster traffic
port = 7878
node_name = "gcp-worker"

[providers.anthropic]
type = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"
default_model = "claude-haiku-4-5"

[cluster]
node_name = "gcp-worker"
cluster_secret = "your-shared-secret"   # same value as coordinator
peers = ["http://100.a.b.c:7878"]       # Mac Tailscale IP

[telegram]
bot_token_env = "TELEGRAM_BOT_TOKEN"
allowed_users = [123456789]             # your Telegram user ID
agent = "master"

[agent.master]
provider = "anthropic"
model = "claude-haiku-4-5"
tools = ["shell", "file_read", "file_write", "web_search", "memory_store", "memory_recall"]
instructions = "You are an AI assistant on a 24/7 cloud node. Respond concisely for mobile users."
```

A ready-to-use coordinator template is in `configs/agents.coordinator.toml` and a cloud template is in `configs/agents.cloud.toml`.

**Important:** Set `host = "0.0.0.0"` on any node that needs to accept incoming connections from peers. The default `127.0.0.1` only accepts local connections.

---

## Step 4: Start Both Daemons

**On the Mac:**
```bash
export ANTHROPIC_API_KEY=sk-ant-...
phantom-mesh daemon
```

**On the GCP VM:**
```bash
export ANTHROPIC_API_KEY=sk-ant-...
export TELEGRAM_BOT_TOKEN=123456789:ABCdef...
phantom-mesh daemon
```

Verify health on each node:

```bash
# from your Mac, check the GCP node
curl http://<GCP_TAILSCALE_IP>:7878/health
# → {"status":"ok"}

# from the GCP VM, check the Mac
curl http://<MAC_TAILSCALE_IP>:7878/health
# → {"status":"ok"}
```

---

## Step 5: Test a Cross-Node Task

Phantom Mesh uses SHA-256 HMAC to authenticate RPC calls between nodes. The `X-Cluster-Auth` header carries the auth token.

Submit a task to the Mac coordinator and have it run on the GCP worker:

```bash
# Compute the HMAC token
BODY='{"agent":"master","prompt":"What is the hostname of this machine?"}'
SECRET="your-shared-secret"
TOKEN=$(echo -n "${BODY}" | openssl dgst -sha256 -hmac "${SECRET}" | awk '{print $2}')

# Submit to the GCP worker directly
curl -X POST http://<GCP_TAILSCALE_IP>:7878/rpc/task/assign \
  -H "Content-Type: application/json" \
  -H "X-Cluster-Auth: ${TOKEN}" \
  -d "${BODY}"
# → {"job_id":"..."}

# Poll for result
curl http://<GCP_TAILSCALE_IP>:7878/rpc/task/status/<job_id>
```

Or submit to the coordinator and let it delegate:

```bash
curl -X POST http://localhost:7878/agent/master/run \
  -H "Content-Type: application/json" \
  -d '{"prompt": "Ask the gcp-worker node what its hostname is"}'
```

---

## Step 6: Add Telegram for Mobile Access

The Telegram bot lets you control the mesh from any phone without opening a laptop.

1. Open Telegram and message `@BotFather`. Send `/newbot` and follow the prompts to choose a name and username.

2. BotFather gives you a token like `123456789:ABCdef...`.

3. Set it on the always-on cloud node:
   ```bash
   export TELEGRAM_BOT_TOKEN=123456789:ABCdef...
   ```
   For a persistent daemon, add it to `/etc/phantom-mesh.env` and reference it in your systemd unit.

4. Add the `[telegram]` block to the cloud node's `agents.toml` (already shown in step 3 above).

5. Get your Telegram user ID by messaging `@userinfobot`. Set it in `allowed_users` to make the bot private.

6. Restart the daemon on the cloud node. Open Telegram, find your bot by username, and send it a message.

The cloud node handles the Telegram polling and forwards tasks into the mesh. The Mac does not need to be online for Telegram to work.

---

## Troubleshooting

### Nodes cannot reach each other

```bash
# Check all devices appear in your Tailscale network
tailscale status

# Test reachability directly
tailscale ping gcp-worker    # replace with device name from tailscale status
```

If `tailscale ping` succeeds but port 7878 is unreachable:

- **Cloud provider firewall:** add a TCP ingress rule for port 7878.
  - GCP: VPC network > Firewall > add rule for TCP 7878
  - Oracle Cloud: Networking > VCN > Security Lists > add Ingress Rule for TCP 7878
- **Daemon listening on wrong interface:** ensure `host = "0.0.0.0"` in `[core]` on the node receiving connections.

### Tailscale ACLs blocking traffic

If your Tailscale account has custom ACLs, add a rule to allow traffic on port 7878:

```json
{
  "action": "accept",
  "src":    ["*"],
  "dst":    ["*:7878"]
}
```

Edit ACLs at [login.tailscale.com/admin/acls](https://login.tailscale.com/admin/acls).

### Cluster auth errors (401 / 403)

- Verify `cluster_secret` is identical on every node — copy-paste the `openssl rand -hex 32` output exactly.
- Check there are no trailing spaces or newlines in the secret value.

### Tailscale not running

```bash
# Linux
sudo systemctl status tailscaled
sudo systemctl start tailscaled
tailscale up

# macOS (brew install)
sudo tailscaled &
tailscale up
```

### Daemon won't start on cloud VM

```bash
# Run interactively to see errors
./phantom-mesh daemon 2>&1 | tee /tmp/phantom.log

# Check the log for:
# - "config file not found" → wrong path
# - "provider error" → bad or missing API key
# - "address already in use" → port 7878 taken: lsof -i :7878
```

### Telegram bot not responding

```bash
# Verify the daemon is running on the cloud node
curl http://localhost:7878/health

# Check the bot token is set
echo $TELEGRAM_BOT_TOKEN

# Check daemon logs for Telegram errors
journalctl -u phantom-mesh -f   # if running under systemd
```

---

## Running as a systemd Service (Cloud Node)

To keep the daemon running after logout and across reboots:

```bash
sudo tee /etc/phantom-mesh.env > /dev/null <<EOF
ANTHROPIC_API_KEY=sk-ant-YOUR_KEY
TELEGRAM_BOT_TOKEN=123456789:YOUR_TOKEN
EOF

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

---

For the full multi-node deployment walkthrough (coordinator, local workers, cloud VM), see [DEPLOYMENT.md](DEPLOYMENT.md).
