# Installing Phantom Mesh on Oracle Cloud (Linux VM)

**Audience:** operator deploying a phantom-mesh coordinator node on
Oracle Cloud Infrastructure (OCI) Always-Free.

**Canonical target** (per `docs/SPEC-FREEZE-V1.md` §6.1): **A1 ARM**
(`VM.Standard.A1.Flex`, up to 4 OCPU / 24 GB RAM, `aarch64-unknown-linux-gnu`).

**Scaffolding target** (Path C, until A1 capacity is grabbed): **AMD micro**
(`VM.Standard.E2.1.Micro`, 1 OCPU / 1 GB RAM, `x86_64-unknown-linux-gnu`).

The same procedure works on both; the only differences are noted inline.

---

## 1. OCI console — one-time setup

These steps must be done in the Oracle Cloud web console. The setup
scripts can't reach OCI APIs.

### 1.1 Provision the VM

| Setting | Value |
|---|---|
| Image | Oracle Linux 9 (RHEL family) **or** Ubuntu 22.04 LTS |
| Shape | `VM.Standard.A1.Flex` (4 OCPU / 24 GB) — preferred, "Always Free" |
| ↳ fallback | `VM.Standard.E2.1.Micro` (1 OCPU / 1 GB) — if A1 is "Out of host capacity" |
| Region | Closest to operator (Singapore, Tokyo, Osaka, Seoul) |
| SSH key | Paste your public key |
| VCN | Default created during launch is fine |

A1 capacity in popular Asian regions is frequently full — keep retrying
launch every few hours, or write a polling cron.

### 1.2 Assign a public IP

Console → Compute → Instances → *your VM* → Attached VNICs → Primary VNIC →
**Reserve a public IP** (or assign an ephemeral one).

### 1.3 Security List inbound rules

Console → Networking → Virtual Cloud Networks → *your VCN* → Security Lists →
*Default Security List* → Add Ingress Rules:

| Source CIDR | Protocol | Dest Port | Notes |
|---|---|---|---|
| `<your home IP>/32` | TCP | `22` | SSH bootstrap (replace with your actual IP) |
| `0.0.0.0/0` | UDP | `41641` | Tailscale STUN — required for direct path |

**Do not** add `7878/tcp` to the public list. The phantom daemon is
reached over Tailscale only; the script's `firewalld`/`ufw` rules already
gate it to the `tailscale0` interface.

### 1.4 SSH bootstrap

```bash
# ~/.ssh/config on operator's laptop
Host oci-coord
  HostName <PUBLIC_IP>
  User opc                  # opc on Oracle Linux; ubuntu on Ubuntu
  IdentityFile ~/.ssh/id_ed25519
```

Test: `ssh oci-coord true` — must succeed before proceeding.

---

## 2. Tailscale pre-auth key

phantom-mesh assumes the VM joins a Tailscale tailnet (per
`docs/MULTI-DEVICE-COORDINATION.md` Rule 4 + `docs/SESSION-ONBOARDING.md`
§3.2). The setup script wants this key non-interactively:

1. Visit <https://login.tailscale.com/admin/settings/keys>
2. **Generate auth key** → reusable=false, ephemeral=false, tag=`tag:phantom-mesh`
3. Copy the resulting `tskey-auth-...` (one-time view)

You'll pass this as `TAILSCALE_AUTH_KEY` to `setup-oci.sh`.

---

## 3. On the VM — clone, build, configure

### 3.0 Get the source onto the VM

The repo is private, so a bare `git clone https://...` will hang for
credentials. Pick one of three:

**Option A — `gh` CLI (recommended; least setup; supports `git pull`):**

```bash
ssh oci-coord
sudo dnf install -y gh git                  # Oracle Linux 9
# OR: sudo apt-get install -y gh git        # Ubuntu / Debian
gh auth login                                # GitHub.com → HTTPS → device code
gh repo clone markl-a/phantom-mesh-private ~/repos/phantom-mesh
cd ~/repos/phantom-mesh && git checkout platform/linux
```

**Option B — SSH deploy key (most secure; revocable; `git pull` works):**

```bash
ssh oci-coord
ssh-keygen -t ed25519 -f ~/.ssh/gh-phantom -C "oci-coord" -N ""
cat ~/.ssh/gh-phantom.pub
# Copy the printed line. In a browser:
#   github.com/markl-a/phantom-mesh-private/settings/keys → Add deploy key
#   Title: oci-coord    Allow write access: NO    Key: paste pubkey

cat >> ~/.ssh/config <<'EOF'
Host github-phantom
  HostName github.com
  User git
  IdentityFile ~/.ssh/gh-phantom
EOF
chmod 600 ~/.ssh/config ~/.ssh/gh-phantom

git clone git@github-phantom:markl-a/phantom-mesh-private.git ~/repos/phantom-mesh
cd ~/repos/phantom-mesh && git checkout platform/linux
```

**Option C — `scp` tarball from operator's machine (no GitHub auth on VM):**

From operator's local repo (Windows PowerShell or macOS/Linux shell):

```bash
cd <repo-root>
tar --exclude='target' --exclude='node_modules' --exclude='dist' \
    --exclude='.worktrees' -czf /tmp/phantom-mesh.tar.gz .
scp /tmp/phantom-mesh.tar.gz oci-coord:~/
ssh oci-coord '
  mkdir -p ~/repos/phantom-mesh &&
  tar -xzf ~/phantom-mesh.tar.gz -C ~/repos/phantom-mesh &&
  cd ~/repos/phantom-mesh && git checkout platform/linux
'
```

Tarball is ~200–400 MB; on E2.1.Micro 0.5 Gbps uplink expect 1–2 min.
Updates after first deploy still need GitHub access on the VM (or
re-scp), so this is a one-shot bootstrap option.

### 3.1 Build the binary

```bash
# Install Rust if needed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env

# Distro deps for build
sudo dnf install -y gcc gcc-c++ openssl-devel pkgconf-pkg-config       # Oracle Linux / RHEL
# OR
sudo apt-get update && sudo apt-get install -y build-essential pkg-config libssl-dev   # Ubuntu / Debian

# Native release build
./scripts/build-linux.sh
```

This places the binary at `dist/phantom-<host-triple>`.

**On E2.1.Micro (1 GB RAM)** the build will OOM-kill during the link
phase. Two options:

- **Add swap first** (the build-linux script will warn; setup-oci does it
  later but you need it before build):
  ```bash
  sudo fallocate -l 2G /swapfile && sudo chmod 600 /swapfile
  sudo mkswap /swapfile && sudo swapon /swapfile
  echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab
  ```
  Build will succeed but take ~15–25 min.

- **Or build elsewhere and scp**: cross-compile on a more capable
  Linux host, then `scp dist/phantom-x86_64-unknown-linux-gnu oci-coord:~/repos/phantom-mesh/dist/`.
  This is unsupported by `build-linux.sh` (per spec §6.1 prefers
  on-target build).

### 3.2 Run setup-oci.sh

```bash
TAILSCALE_AUTH_KEY=tskey-auth-XXXXXXXX \
NODE_NAME=oci-singapore-coord \
  ./scripts/setup-oci.sh
```

This is idempotent — re-run any time. It performs:

1. Swap file (if RAM <2 GB)
2. Package deps
3. Tailscale install + non-interactive `up`
4. Firewall: 7878/tcp on `tailscale0` only
5. `~/.local/bin/phantom` install + SELinux context (RHEL)
6. systemd **user** unit (`~/.config/systemd/user/phantom-mesh.service`)
7. `loginctl enable-linger` (so service stays up across logouts)
8. `~/.phantom-mesh/agents.toml` from `configs/agents.cloud.toml`
9. `~/.phantom-mesh/local.toml` skeleton + `~/.phantom-mesh/env` skeleton

**It does not start the daemon** — secrets aren't filled yet.

### 3.3 Fill secrets

```bash
# 1. Cluster secret (must match every other node)
$EDITOR ~/.phantom-mesh/local.toml
# Paste cluster_secret = "<32-byte hex from 1Password / Mac M1>"

# 2. API keys
$EDITOR ~/.phantom-mesh/env
# Uncomment + fill:
#   ANTHROPIC_API_KEY=sk-ant-...
#   OPENROUTER_API_KEY=sk-or-...
#   TELEGRAM_BOT_TOKEN=...      (optional, only if this node is the bot gateway)
chmod 600 ~/.phantom-mesh/env ~/.phantom-mesh/local.toml
```

### 3.4 Start + verify

```bash
systemctl --user start phantom-mesh
journalctl --user -fu phantom-mesh        # tail logs

# In another shell:
curl -s http://localhost:7878/rpc/ping
# Expected JSON: {"core_sha": "...", "wire_version": 1, "phantom_version": "0.4.0", ...}

# From another tailnet device (Mac, phone, etc):
curl -s http://oci-singapore-coord:7878/rpc/ping
# Same response — confirms Tailscale + firewall rules let tailnet through

# From outside the tailnet (e.g. laptop without Tailscale, using public IP):
curl -m 5 http://<PUBLIC_IP>:7878/rpc/ping
# MUST fail (timeout / refused). If it succeeds, firewall is wrong.
```

---

## 4. Daily verification (per Rule 6)

```bash
phantom doctor --mesh
```

Three exit codes per `docs/MULTI-DEVICE-COORDINATION.md` Rule 6:
- `0` all peers green
- `1` degraded (some warnings)
- `2` broken (HMAC mismatch / schema mismatch)

If `phantom doctor --mesh` flags `core_sha` drift between this node and
the Mac M1 reference impl, run `phantom upgrade` (or rebuild via `git
pull && ./scripts/build-linux.sh && systemctl --user restart phantom-mesh`).

---

## 5. Troubleshooting

### `MemoryMax` OOM (E2.1.Micro)

The systemd user unit doesn't pin memory by default. If the daemon
gets OOM-killed under load on 1 GB:

```ini
# Add to ~/.config/systemd/user/phantom-mesh.service [Service] section:
MemoryMax=600M
MemoryHigh=500M
```

Then `systemctl --user daemon-reload && systemctl --user restart phantom-mesh`.

### SELinux denials (Oracle Linux)

```bash
sudo ausearch -m avc -ts recent | grep phantom
sudo audit2why -al | grep phantom
```

If `bin_t` context wasn't applied:
```bash
sudo semanage fcontext -a -t bin_t "$HOME/.local/bin/phantom"
sudo restorecon -v "$HOME/.local/bin/phantom"
```

Last-resort (don't leave on): `sudo setenforce 0`.

### firewalld zone diagnosis (RHEL)

```bash
sudo firewall-cmd --list-all-zones | grep -A8 trusted
# Expect: interfaces: tailscale0  /  ports: 7878/tcp
```

### `loginctl enable-linger` failed

Without linger, the user systemd service stops when the session ends.
Verify:
```bash
loginctl show-user "$(whoami)" | grep -i linger
# Linger=yes
```

### Tailscale auth key expired / invalid

Re-generate at <https://login.tailscale.com/admin/settings/keys>, then:
```bash
TAILSCALE_AUTH_KEY=tskey-auth-... ./scripts/setup-oci.sh
```
(idempotent — only re-runs missing steps).

### Building with 1 GB RAM keeps OOMing even with swap

```bash
# Reduce parallelism so peak memory is lower:
CARGO_BUILD_JOBS=1 ./scripts/build-linux.sh
```

---

## 6. Migrating from E2.1.Micro to A1 (when capacity opens)

Path C handover:

1. Provision new A1 instance per §1.1
2. SSH to A1, `git clone ...`, `git checkout platform/linux`
3. Run `./scripts/build-linux.sh` → produces canonical `dist/phantom-aarch64-unknown-linux-gnu`
4. Copy `cluster_secret` from old node's `~/.phantom-mesh/local.toml`
5. Run `setup-oci.sh` on A1 with same `NODE_NAME`
6. Verify mesh reaches it (`phantom doctor --mesh` on Mac M1)
7. Stop old E2.1.Micro service: `systemctl --user stop phantom-mesh`
8. Optionally terminate the E2.1.Micro VM after a soak period

The A1 binary is the spec-canonical artefact (per SPEC-FREEZE-V1 §6.1).
The E2.1.Micro binary is scaffolding — not a release target.

---

## 7. Cross-references

- `docs/SESSION-ONBOARDING.md` §3.2 — task list when you open the
  Oracle Cloud session for development
- `docs/MULTI-DEVICE-COORDINATION.md` §Rule 4 — config split rationale
- `docs/SPEC-FREEZE-V1.md` §6.1 — canonical artefact table
- `templates/phantom-mesh.service.tmpl` — the systemd template this
  script renders
- `scripts/build-linux.sh` — produces the binary
- `scripts/setup-oci.sh` — VM-side post-build deploy
