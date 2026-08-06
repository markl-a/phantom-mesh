#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# setup-oracle-ampere.sh — v0.6.0 three-node demo bring-up, Node B (cloud).
#
# Run this ON the Oracle Cloud Free Tier Ampere A1 instance (Ubuntu 22.04
# aarch64). It installs Tailscale, drops a pre-built aarch64-linux `spectyn`
# binary onto the box, renders a cluster-ready `agents.toml`, and starts
# `spectyn serve` as a systemd user service so it survives logout.
#
# Why this exists separately from scripts/setup-oci.sh
# ----------------------------------------------------
# `setup-oci.sh` is the v0.5.x single-node coordinator setup (writes a much
# bigger config with provider keys, opens UFW for tailscale0, builds-from-source
# expectation). This script is purpose-built for the v0.6.0 three-node demo:
#   • Tiny `[cluster]` block only — no provider keys, no UFW dance
#   • Pulls a pre-built binary from a maintainer URL (operator memory:
#     `phantommesh.io/dist/spectyn-linux-aarch64` once L1 publishes it)
#   • SHA256 verify is OPTIONAL (env-var driven) so first-day operators can
#     run it before the .sha256 sidecar exists
#   • Systemd USER service (no sudo needed past `apt install`)
#
# Steps performed
# ---------------
#   1. Pre-flight: arch == aarch64, OS == Linux, sudo available
#   2. Install Tailscale via official one-liner (operator does `tailscale up`
#      themselves — we don't take an auth key over the wire)
#   3. Wait up to TAILSCALE_TIMEOUT_S for `tailscale status` to be Healthy
#   4. Download spectyn binary from SPECTYN_BIN_URL (or, fallback, print the
#      cargo zigbuild one-liner the operator can run on their workstation and
#      `scp` over — building on the Ampere VM itself works but is ~20 min)
#   5. If SPECTYN_BIN_SHA256 is set, verify (sha256sum); on mismatch, delete
#      the binary and exit 1 (matches F-CRIT-3 install-script safety policy)
#   6. Write ~/.spectyn-mesh/agents.toml with the cluster section
#   7. Write ~/.config/systemd/user/spectyn-serve.service
#   8. `loginctl enable-linger` + `systemctl --user enable --now spectyn-serve`
#   9. Verify: tailscale IP printed, `curl /healthz` returns 200
#
# Environment contract (REQUIRED)
# -------------------------------
#   SPECTYN_CLUSTER_SECRET   — 64-hex shared secret (same on all 3 nodes)
#                              generate once with: openssl rand -hex 32
#
# Environment contract (OPTIONAL)
# -------------------------------
#   SPECTYN_BIN_URL          — https:// URL to a pre-built spectyn binary for
#                              aarch64-linux. Default:
#                              https://phantommesh.io/dist/spectyn-linux-aarch64
#   SPECTYN_BIN_SHA256       — expected sha256 (lowercase hex). If unset,
#                              SHA256 verification is SKIPPED with a loud warn.
#   SPECTYN_NODE_NAME        — node_name written to agents.toml
#                              (default: "cloud")
#   SPECTYN_CAPABILITIES     — comma-separated capability list
#                              (default: "always_on,big_disk")
#   SPECTYN_HOME             — spectyn data dir (default: ~/.spectyn-mesh)
#   SPECTYN_PORT             — serve port (default: 7878)
#   TAILSCALE_TIMEOUT_S      — wait for `tailscale up` to become Healthy
#                              (default: 120)
#   SPECTYN_SKIP_TAILSCALE=1 — skip Tailscale install/wait (already configured)
#   SPECTYN_SKIP_VERIFY=1    — explicit opt-out of SHA256 even if hash given
#
# Operator workflow
# -----------------
#   ssh ubuntu@<oracle-public-ip>
#   export SPECTYN_CLUSTER_SECRET=$(cat /tmp/cluster-secret)   # from workstation
#   export SPECTYN_BIN_URL=https://phantommesh.io/dist/spectyn-linux-aarch64
#   curl -fsSL https://raw.githubusercontent.com/markl-a/spectyn-mesh/main/scripts/bring-up/setup-oracle-ampere.sh \
#     | bash
#   # ... then `sudo tailscale up` when prompted (script pauses for this)
#
# Exit codes
# ----------
#   0   — Tailscale Healthy + spectyn serve answers /healthz with 200
#   1   — anything failed; reason on the last FAIL: line
#   77  — preconditions missing (wrong arch, no sudo, missing env)
#
# ─────────────────────────────────────────────────────────────────────────────

set -u

# ── pretty output ────────────────────────────────────────────────────────────
if [ -t 1 ] && [ "${NO_COLOR:-}" = "" ]; then
  C_RESET=$'\033[0m'; C_DIM=$'\033[2m'
  C_RED=$'\033[31m'; C_GREEN=$'\033[32m'; C_YELLOW=$'\033[33m'; C_CYAN=$'\033[36m'
else
  C_RESET=''; C_DIM=''; C_RED=''; C_GREEN=''; C_YELLOW=''; C_CYAN=''
fi
step()  { printf '  %s→%s %s\n'  "$C_DIM"  "$C_RESET" "$*"; }
pass()  { printf '  %s✓%s %s\n'  "$C_GREEN" "$C_RESET" "$*"; }
fail()  { printf '  %s✗%s %s\n'  "$C_RED"   "$C_RESET" "$*" >&2; }
warn()  { printf '  %s⚠%s %s\n'  "$C_YELLOW" "$C_RESET" "$*" >&2; }
title() { printf '\n%s━━ %s ━━%s\n' "$C_CYAN" "$*" "$C_RESET"; }

die() { fail "$*"; printf '\n%sFAIL: %s%s\n' "$C_RED" "$*" "$C_RESET" >&2; exit 1; }

# ── --help short-circuit ─────────────────────────────────────────────────────
case "${1:-}" in
  -h|--help)
    sed -n '2,75p' "$0" | sed -E 's/^# ?//'
    exit 0
    ;;
esac

# ── 1. preconditions ─────────────────────────────────────────────────────────
title "setup-oracle-ampere · preconditions"

ARCH="$(uname -m)"
case "$ARCH" in
  aarch64|arm64) step "arch: $ARCH (OK)" ;;
  *) warn "expected aarch64 (Oracle Ampere A1); got: $ARCH — continuing anyway" ;;
esac

if [ "$(uname -s)" != "Linux" ]; then
  warn "skipping — host is $(uname -s), expected Linux"
  exit 77
fi

if [ -z "${SPECTYN_CLUSTER_SECRET:-}" ]; then
  fail "SPECTYN_CLUSTER_SECRET is required (64-hex shared cluster secret)"
  fail "generate one on your workstation: openssl rand -hex 32"
  exit 77
fi

# Validate cluster secret shape (64 lowercase hex chars). Tolerate uppercase by
# downcasing.
SPECTYN_CLUSTER_SECRET="$(printf '%s' "$SPECTYN_CLUSTER_SECRET" | tr 'A-Z' 'a-z')"
if ! printf '%s' "$SPECTYN_CLUSTER_SECRET" | grep -Eq '^[0-9a-f]{64}$'; then
  fail "SPECTYN_CLUSTER_SECRET must be 64 lowercase hex chars (got ${#SPECTYN_CLUSTER_SECRET})"
  exit 77
fi

for cmd in curl sudo systemctl; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    fail "required command not on PATH: $cmd"
    exit 77
  fi
done

# Defaults
SPECTYN_NODE_NAME="${SPECTYN_NODE_NAME:-cloud}"
SPECTYN_CAPABILITIES="${SPECTYN_CAPABILITIES:-always_on,big_disk}"
SPECTYN_HOME="${SPECTYN_HOME:-$HOME/.spectyn-mesh}"
SPECTYN_PORT="${SPECTYN_PORT:-7878}"
SPECTYN_BIN_URL="${SPECTYN_BIN_URL:-https://phantommesh.io/dist/spectyn-linux-aarch64}"
TAILSCALE_TIMEOUT_S="${TAILSCALE_TIMEOUT_S:-120}"

step "node name : $SPECTYN_NODE_NAME"
step "capabilities: $SPECTYN_CAPABILITIES"
step "home dir  : $SPECTYN_HOME"
step "serve port: $SPECTYN_PORT"
step "binary URL: $SPECTYN_BIN_URL"

# ── 2. install Tailscale ─────────────────────────────────────────────────────
if [ "${SPECTYN_SKIP_TAILSCALE:-0}" = "1" ]; then
  title "setup-oracle-ampere · Tailscale (SKIPPED via SPECTYN_SKIP_TAILSCALE=1)"
else
  title "setup-oracle-ampere · install Tailscale"
  if command -v tailscale >/dev/null 2>&1; then
    step "tailscale already installed: $(tailscale version | head -1)"
  else
    step "installing via official one-liner (curl | sh) — needs sudo"
    if ! curl -fsSL https://tailscale.com/install.sh | sh; then
      die "tailscale install failed"
    fi
    pass "tailscale installed"
  fi

  # Operator brings up Tailscale themselves (we don't want to take an auth
  # key via env-var because it would end up in `ps` output).
  title "setup-oracle-ampere · Tailscale up (operator action)"
  if ! tailscale status >/dev/null 2>&1; then
    warn "tailscale is NOT up yet. Run in a SEPARATE ssh session:"
    warn "    sudo tailscale up"
    warn "(opens a browser login URL — approve in your tailnet)"
    warn ""
    warn "This script will wait up to ${TAILSCALE_TIMEOUT_S}s for it to succeed."
  fi
  deadline=$(( $(date +%s) + TAILSCALE_TIMEOUT_S ))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    if tailscale status >/dev/null 2>&1; then
      break
    fi
    sleep 3
  done
  if ! tailscale status >/dev/null 2>&1; then
    die "tailscale never became Healthy within ${TAILSCALE_TIMEOUT_S}s"
  fi
  TS_IP="$(tailscale ip -4 2>/dev/null | head -1)"
  if [ -z "$TS_IP" ]; then
    die "tailscale up succeeded but no IPv4 assigned (?!)"
  fi
  pass "tailscale up: $TS_IP"
fi

# ── 3. download spectyn binary ───────────────────────────────────────────────
title "setup-oracle-ampere · spectyn binary"

mkdir -p "$SPECTYN_HOME/bin"
BIN_PATH="$SPECTYN_HOME/bin/spectyn"

# Honour explicit local override (advanced users who scp'd a binary).
if [ -n "${SPECTYN_BIN_LOCAL:-}" ]; then
  if [ ! -x "$SPECTYN_BIN_LOCAL" ]; then
    die "SPECTYN_BIN_LOCAL=$SPECTYN_BIN_LOCAL is not executable"
  fi
  step "using local binary: $SPECTYN_BIN_LOCAL"
  cp -f "$SPECTYN_BIN_LOCAL" "$BIN_PATH"
else
  step "downloading: $SPECTYN_BIN_URL"
  case "$SPECTYN_BIN_URL" in
    https://*) ;;
    http://*)
      if [ "${SPECTYN_ALLOW_INSECURE:-0}" != "1" ]; then
        die "refusing http:// binary URL (set SPECTYN_ALLOW_INSECURE=1 to bypass)"
      fi
      warn "SPECTYN_ALLOW_INSECURE=1 — downloading over plain http://"
      ;;
    *) die "unsupported SPECTYN_BIN_URL scheme: $SPECTYN_BIN_URL" ;;
  esac
  if ! curl -fsSL --max-time 120 "$SPECTYN_BIN_URL" -o "$BIN_PATH"; then
    fail "download failed from $SPECTYN_BIN_URL"
    fail ""
    fail "Fallback: build aarch64-linux binary on your WORKSTATION (~5 min, not"
    fail "          on this Ampere VM where it takes ~20 min and risks OOM):"
    fail ""
    fail "  cd spectyn-mesh/core"
    fail "  cargo install cargo-zigbuild   # one-time"
    fail "  cargo zigbuild --release --target aarch64-unknown-linux-gnu --bin spectyn"
    fail "  scp target/aarch64-unknown-linux-gnu/release/spectyn \\"
    fail "      ubuntu@<oracle-public-ip>:$BIN_PATH"
    fail ""
    fail "Then re-run this script with SPECTYN_BIN_LOCAL=$BIN_PATH set."
    exit 1
  fi
fi

chmod +x "$BIN_PATH"

# ── 4. SHA256 verify (optional but recommended) ──────────────────────────────
if [ -n "${SPECTYN_BIN_SHA256:-}" ] && [ "${SPECTYN_SKIP_VERIFY:-0}" != "1" ]; then
  step "verifying SHA256 (expected: $SPECTYN_BIN_SHA256)"
  expected="$(printf '%s' "$SPECTYN_BIN_SHA256" | tr 'A-Z' 'a-z')"
  if ! printf '%s' "$expected" | grep -Eq '^[0-9a-f]{64}$'; then
    rm -f "$BIN_PATH"
    die "SPECTYN_BIN_SHA256 is not 64-hex (got: $expected)"
  fi
  actual="$(sha256sum "$BIN_PATH" | awk '{print tolower($1)}')"
  if [ "$expected" != "$actual" ]; then
    rm -f "$BIN_PATH"
    fail "expected: $expected"
    fail "actual:   $actual"
    die "SHA256 mismatch — binary deleted, refusing to install"
  fi
  pass "SHA256 verified ($expected)"
else
  warn "SHA256 verification SKIPPED (SPECTYN_BIN_SHA256 unset or SPECTYN_SKIP_VERIFY=1)"
  warn "  set SPECTYN_BIN_SHA256=<hex> from your maintainer to enable verify"
fi

# Smoke test the binary runs at all.
if ! "$BIN_PATH" --version >/dev/null 2>&1; then
  die "binary at $BIN_PATH does not execute (wrong arch? corrupt download?)"
fi
pass "binary OK: $($BIN_PATH --version 2>&1 | head -1)"

# ── 5. agents.toml ───────────────────────────────────────────────────────────
title "setup-oracle-ampere · render agents.toml"

mkdir -p "$SPECTYN_HOME"
AGENTS_TOML="$SPECTYN_HOME/agents.toml"

# Convert comma-separated caps to TOML array literal.
CAPS_TOML="$(printf '%s' "$SPECTYN_CAPABILITIES" | awk -F, '{
  out=""; for (i=1;i<=NF;i++) {
    if (out != "") out = out ", "
    out = out "\"" $i "\""
  } print "[" out "]"
}')"

cat > "$AGENTS_TOML" <<EOF
# Auto-rendered by scripts/bring-up/setup-oracle-ampere.sh
# (v0.6.0 three-node demo bring-up — Node B / cloud)
[core]
host = "0.0.0.0"   # listen on all interfaces (Tailscale needs this)
port = $SPECTYN_PORT

[cluster]
node_name      = "$SPECTYN_NODE_NAME"
cluster_secret = "$SPECTYN_CLUSTER_SECRET"
capabilities   = $CAPS_TOML
worker_caps    = $CAPS_TOML
enforce_caps   = "soft"
# peers: leave empty here — the workstation node lists this cloud node in
# its peers, heartbeat handshake then makes the relationship bidirectional.

[agent.master]
provider = "echo"
model    = "echo"
EOF
chmod 600 "$AGENTS_TOML"
pass "wrote $AGENTS_TOML (mode 600 — contains cluster_secret)"

# ── 6. systemd user service ──────────────────────────────────────────────────
title "setup-oracle-ampere · systemd user service"

# Linger so the service persists across logout (`systemctl --user` daemons die
# with the last session otherwise).
if ! loginctl show-user "$(whoami)" 2>/dev/null | grep -q '^Linger=yes$'; then
  step "enabling linger so service survives logout (needs sudo)"
  sudo loginctl enable-linger "$(whoami)" || warn "enable-linger failed (continuing)"
fi

mkdir -p "$HOME/.config/systemd/user"
SVC_FILE="$HOME/.config/systemd/user/spectyn-serve.service"
cat > "$SVC_FILE" <<EOF
# Auto-rendered by scripts/bring-up/setup-oracle-ampere.sh
[Unit]
Description=Spectyn Mesh — cluster node (v0.6.0 three-node demo)
After=network-online.target tailscaled.service
Wants=network-online.target

[Service]
Type=simple
ExecStart=$BIN_PATH serve --config $AGENTS_TOML
WorkingDirectory=$SPECTYN_HOME
Environment=SPECTYN_FORWARD_ON_CAPS_MISMATCH=1
Environment=SPECTYN_ENFORCE_REQUIRED_CAPS=soft
Restart=on-failure
RestartSec=10
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
EOF
pass "wrote $SVC_FILE"

systemctl --user daemon-reload
if systemctl --user enable --now spectyn-serve.service; then
  pass "systemctl --user enable --now spectyn-serve.service"
else
  die "systemctl --user enable --now failed — see: journalctl --user -u spectyn-serve -n 50"
fi

# Give it a moment to bind the port.
sleep 3

# ── 7. verification ──────────────────────────────────────────────────────────
title "setup-oracle-ampere · verification"

TS_IP_OUT="$(tailscale ip -4 2>/dev/null | head -1 || true)"
if [ -n "$TS_IP_OUT" ]; then
  pass "tailscale IPv4: $TS_IP_OUT"
else
  warn "tailscale ip -4 returned nothing (skipped step 2?)"
fi

deadline=$(( $(date +%s) + 30 ))
healthz_ok=0
while [ "$(date +%s)" -lt "$deadline" ]; do
  code="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 3 \
              "http://localhost:$SPECTYN_PORT/healthz" 2>/dev/null || echo 000)"
  if [ "$code" = "200" ]; then healthz_ok=1; break; fi
  sleep 1
done

if [ "$healthz_ok" = "1" ]; then
  pass "GET http://localhost:$SPECTYN_PORT/healthz → 200"
else
  fail "GET /healthz never returned 200 within 30s"
  fail "last 20 journal lines:"
  journalctl --user -u spectyn-serve -n 20 --no-pager >&2 || true
  die "spectyn serve did not come up"
fi

# ── done ─────────────────────────────────────────────────────────────────────
title "setup-oracle-ampere · DONE"
printf '\n'
printf '%sPASS%s — Node B (cloud / %s) is on Tailscale and serving on :%s\n' \
       "$C_GREEN" "$C_RESET" "$SPECTYN_NODE_NAME" "$SPECTYN_PORT"
printf '\n'
printf 'Next steps (on your workstation):\n'
printf '  1. Add this node to your workstation agents.toml [cluster].peers:\n'
printf '       "http://%s:%s"\n' "${TS_IP_OUT:-<tailscale-ip>}" "$SPECTYN_PORT"
printf '  2. Run scripts/bring-up/setup-android-termux.sh on the tablet.\n'
printf '  3. Run scripts/spectyn-test/scenarios/three_node_demo.sh to verify.\n'
printf '\n'
printf 'Service controls (on this VM):\n'
printf '  systemctl --user status spectyn-serve\n'
printf '  systemctl --user restart spectyn-serve\n'
printf '  journalctl --user -u spectyn-serve -f\n'
exit 0
