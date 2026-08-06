#!/data/data/com.termux/files/usr/bin/bash
# ─────────────────────────────────────────────────────────────────────────────
# setup-android-termux.sh — v0.6.0 three-node demo bring-up, Node C (mobile).
#
# Run this INSIDE Termux on an aarch64 Android device (tablet or phone). It
# installs base packages, drops a pre-built aarch64-android spectyn binary
# into ~/.spectyn-mesh/bin/, renders the cluster `agents.toml`, and starts
# `spectyn serve` via `nohup` (Termux has limited service support — see the
# foreground-app-keepalive notes below).
#
# Why a separate script from scripts/termux-setup.sh
# --------------------------------------------------
# `termux-setup.sh` is the v0.5.x single-worker setup (pulls binary from a
# specific coordinator URL, writes a much bigger agents.toml with Groq +
# Ollama provider blocks, hard-codes 7879). This script is purpose-built for
# the v0.6.0 three-node demo:
#   • Tiny `[cluster]` block only — provider = echo, no Groq key needed
#   • Pulls a pre-built android binary from a maintainer URL
#     (default: phantommesh.io/dist/spectyn-aarch64-linux-android)
#   • SHA256 verify is OPTIONAL but recommended
#   • Tailscale on Android is a SEPARATE APP (Play Store) — this script
#     only verifies it's installed + asks the operator to grant always-on
#
# Steps performed
# ---------------
#   1. Pre-flight: running under Termux, arch == aarch64
#   2. `pkg update && pkg install curl wget termux-services` (idempotent)
#   3. Tailscale check (script does NOT install — that's a separate
#      Play Store app), prints operator action if missing
#   4. Download spectyn binary from SPECTYN_BIN_URL
#   5. SHA256 verify if SPECTYN_BIN_SHA256 set; on mismatch, delete + exit 1
#   6. Write ~/.spectyn-mesh/agents.toml
#   7. Start `spectyn serve` via `nohup ... &` (Termux has no systemd; the
#      sv-enable / termux-services path is brittle on Android 14+, so we
#      keep this simple. The runbook documents the foreground-app keepalive
#      pattern: keep Termux notification visible + grant battery exemption).
#   8. Verify: localhost /healthz returns 200
#
# Environment contract (REQUIRED)
# -------------------------------
#   SPECTYN_CLUSTER_SECRET   — 64-hex shared secret (same on all 3 nodes)
#
# Environment contract (OPTIONAL)
# -------------------------------
#   SPECTYN_BIN_URL          — https:// URL to pre-built aarch64-android binary
#                              (default: https://phantommesh.io/dist/spectyn-aarch64-linux-android)
#   SPECTYN_BIN_SHA256       — expected sha256 (lowercase hex). Skipped if unset.
#   SPECTYN_NODE_NAME        — node_name in agents.toml (default: "mobile")
#   SPECTYN_CAPABILITIES     — comma-separated caps (default: "mobile,camera")
#   SPECTYN_HOME             — spectyn data dir (default: ~/.spectyn-mesh)
#   SPECTYN_PORT             — serve port (default: 7878)
#   SPECTYN_BIN_LOCAL        — already on-device binary path (skip download)
#   SPECTYN_SKIP_VERIFY=1    — explicit opt-out of SHA256 verification
#
# Operator workflow
# -----------------
#   # On the Android tablet, in Termux:
#   pkg install curl
#   export SPECTYN_CLUSTER_SECRET=<paste-from-workstation>
#   curl -fsSL https://raw.githubusercontent.com/markl-a/spectyn-mesh/main/scripts/bring-up/setup-android-termux.sh \
#     | bash
#
# Exit codes
# ----------
#   0   — spectyn serve answers /healthz with 200
#   1   — anything failed; reason on the last FAIL: line
#   77  — preconditions missing (not Termux, wrong arch, no SPECTYN_CLUSTER_SECRET)
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
die()   { fail "$*"; printf '\n%sFAIL: %s%s\n' "$C_RED" "$*" "$C_RESET" >&2; exit 1; }

# ── --help short-circuit ─────────────────────────────────────────────────────
case "${1:-}" in
  -h|--help)
    sed -n '2,65p' "$0" | sed -E 's/^# ?//'
    exit 0
    ;;
esac

# ── 1. preconditions ─────────────────────────────────────────────────────────
title "setup-android-termux · preconditions"

# Termux: $PREFIX is /data/data/com.termux/files/usr — sanity check.
case "${PREFIX:-}" in
  /data/data/com.termux/*) step "Termux detected (PREFIX=$PREFIX)" ;;
  *)
    warn "PREFIX=$PREFIX — this does not look like a Termux shell"
    warn "expected something like /data/data/com.termux/files/usr"
    warn "continuing anyway, but `pkg install` may fail"
    ;;
esac

ARCH="$(uname -m)"
case "$ARCH" in
  aarch64|arm64) step "arch: $ARCH (OK)" ;;
  armv7l|armv7|arm)
    die "32-bit ARM detected ($ARCH) — spectyn-aarch64-linux-android won't run"
    ;;
  *)
    warn "expected aarch64; got $ARCH — proceeding but the binary may not run"
    ;;
esac

if [ -z "${SPECTYN_CLUSTER_SECRET:-}" ]; then
  fail "SPECTYN_CLUSTER_SECRET is required (64-hex shared cluster secret)"
  fail "get it from your workstation (same value you used on the Oracle node)"
  exit 77
fi
SPECTYN_CLUSTER_SECRET="$(printf '%s' "$SPECTYN_CLUSTER_SECRET" | tr 'A-Z' 'a-z')"
if ! printf '%s' "$SPECTYN_CLUSTER_SECRET" | grep -Eq '^[0-9a-f]{64}$'; then
  fail "SPECTYN_CLUSTER_SECRET must be 64 lowercase hex chars (got ${#SPECTYN_CLUSTER_SECRET})"
  exit 77
fi

# Defaults
SPECTYN_NODE_NAME="${SPECTYN_NODE_NAME:-mobile}"
SPECTYN_CAPABILITIES="${SPECTYN_CAPABILITIES:-mobile,camera}"
SPECTYN_HOME="${SPECTYN_HOME:-$HOME/.spectyn-mesh}"
SPECTYN_PORT="${SPECTYN_PORT:-7878}"
SPECTYN_BIN_URL="${SPECTYN_BIN_URL:-https://phantommesh.io/dist/spectyn-aarch64-linux-android}"

step "node name : $SPECTYN_NODE_NAME"
step "capabilities: $SPECTYN_CAPABILITIES"
step "home dir  : $SPECTYN_HOME"
step "serve port: $SPECTYN_PORT"
step "binary URL: $SPECTYN_BIN_URL"

# ── 2. pkg install ──────────────────────────────────────────────────────────
title "setup-android-termux · install base packages"
step "pkg update (idempotent)"
pkg update -y >/dev/null 2>&1 || warn "pkg update warning (continuing)"
step "pkg install curl wget termux-services"
if ! pkg install -y curl wget termux-services >/dev/null 2>&1; then
  # termux-services is optional — try without it.
  warn "termux-services install failed; retrying without it"
  pkg install -y curl wget || die "pkg install curl wget failed"
fi
pass "base packages installed"

# ── 3. Tailscale check ──────────────────────────────────────────────────────
title "setup-android-termux · Tailscale (manual app install)"
# `tailscale` CLI on Android is NOT the same as desktop. There's a Termux pkg
# (`pkg install tailscale`) but its userspace-net mode is brittle on Android
# 14+. The reliable path is the official Tailscale app from Play Store /
# F-Droid, which provides VPN-mode that the Termux process can use.
if command -v tailscale >/dev/null 2>&1 && tailscale status >/dev/null 2>&1; then
  pass "tailscale CLI present + status OK: $(tailscale ip -4 2>/dev/null | head -1)"
else
  warn "Tailscale CLI not usable inside Termux (this is expected)."
  warn ""
  warn "Operator action — outside of Termux:"
  warn "  1. Install 'Tailscale' from Play Store (or F-Droid)"
  warn "  2. Open the app → sign in (Google/GitHub) → join your tailnet"
  warn "  3. Settings → 'Always-on VPN' = ON  (battery: 'Don't optimize')"
  warn "  4. Back here, verify with: ip route get 100.64.0.1   # should NOT fail"
  warn ""
  warn "Once Tailscale app is up, this script will still proceed — the binary"
  warn "binds 0.0.0.0:$SPECTYN_PORT and Android-side VPN routes both inbound"
  warn "and outbound Tailscale traffic to/from any process on the device."
fi

# ── 4. download spectyn binary ──────────────────────────────────────────────
title "setup-android-termux · spectyn binary"
mkdir -p "$SPECTYN_HOME/bin"
BIN_PATH="$SPECTYN_HOME/bin/spectyn"

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
    fail "Fallback: build aarch64-android binary on your WORKSTATION (NDK needed):"
    fail ""
    fail "  cd spectyn-mesh/core"
    fail "  cargo install cargo-ndk    # one-time"
    fail "  cargo ndk -t arm64-v8a build --release --bin spectyn"
    fail "  # then push via adb:"
    fail "  adb push target/aarch64-linux-android/release/spectyn /sdcard/Download/"
    fail "  # on the tablet, in Termux:"
    fail "  cp /sdcard/Download/spectyn $BIN_PATH && chmod +x $BIN_PATH"
    fail ""
    fail "Then re-run this script with SPECTYN_BIN_LOCAL=$BIN_PATH set."
    exit 1
  fi
fi
chmod +x "$BIN_PATH"

# ── 5. SHA256 verify (optional but recommended) ─────────────────────────────
if [ -n "${SPECTYN_BIN_SHA256:-}" ] && [ "${SPECTYN_SKIP_VERIFY:-0}" != "1" ]; then
  step "verifying SHA256 (expected: $SPECTYN_BIN_SHA256)"
  expected="$(printf '%s' "$SPECTYN_BIN_SHA256" | tr 'A-Z' 'a-z')"
  if ! printf '%s' "$expected" | grep -Eq '^[0-9a-f]{64}$'; then
    rm -f "$BIN_PATH"
    die "SPECTYN_BIN_SHA256 is not 64-hex (got: $expected)"
  fi
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$BIN_PATH" | awk '{print tolower($1)}')"
  elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$BIN_PATH" | awk '{print tolower($1)}')"
  else
    rm -f "$BIN_PATH"
    die "no sha256sum / shasum on PATH — cannot verify"
  fi
  if [ "$expected" != "$actual" ]; then
    rm -f "$BIN_PATH"
    fail "expected: $expected"
    fail "actual:   $actual"
    die "SHA256 mismatch — binary deleted"
  fi
  pass "SHA256 verified ($expected)"
else
  warn "SHA256 verification SKIPPED (SPECTYN_BIN_SHA256 unset or SPECTYN_SKIP_VERIFY=1)"
fi

if ! "$BIN_PATH" --version >/dev/null 2>&1; then
  die "binary does not execute (wrong arch? missing libc? try: file $BIN_PATH)"
fi
pass "binary OK: $($BIN_PATH --version 2>&1 | head -1)"

# ── 6. agents.toml ──────────────────────────────────────────────────────────
title "setup-android-termux · render agents.toml"
AGENTS_TOML="$SPECTYN_HOME/agents.toml"
CAPS_TOML="$(printf '%s' "$SPECTYN_CAPABILITIES" | awk -F, '{
  out=""; for (i=1;i<=NF;i++) {
    if (out != "") out = out ", "
    out = out "\"" $i "\""
  } print "[" out "]"
}')"

cat > "$AGENTS_TOML" <<EOF
# Auto-rendered by scripts/bring-up/setup-android-termux.sh
# (v0.6.0 three-node demo bring-up — Node C / mobile)
[core]
host = "0.0.0.0"
port = $SPECTYN_PORT

[cluster]
node_name      = "$SPECTYN_NODE_NAME"
cluster_secret = "$SPECTYN_CLUSTER_SECRET"
capabilities   = $CAPS_TOML
worker_caps    = $CAPS_TOML
enforce_caps   = "soft"
# peers: workstation lists this node; heartbeat does the rest.

[agent.master]
provider = "echo"
model    = "echo"
EOF
chmod 600 "$AGENTS_TOML"
pass "wrote $AGENTS_TOML (mode 600 — contains cluster_secret)"

# ── 7. start spectyn serve via nohup ────────────────────────────────────────
title "setup-android-termux · start spectyn serve"
# Termux has no systemd. termux-services / sv is available but fragile on
# Android 14+ where Doze + background-restrictions kill long-running shells.
# The reliable pattern is:
#   (a) Keep the Termux notification visible (acquire wake-lock via
#       termux-wake-lock — comes from termux-tools)
#   (b) Tell Android battery optimizer to leave Termux alone (manual: Settings
#       → Apps → Termux → Battery → "Unrestricted")
# Then nohup-spawn spectyn and detach. This is documented in the runbook.

# Acquire wake lock so Android doesn't pause Termux when the screen is off.
if command -v termux-wake-lock >/dev/null 2>&1; then
  termux-wake-lock || warn "termux-wake-lock failed (continuing)"
fi

# Sweep any prior instance from a re-run.
PID_FILE="$SPECTYN_HOME/spectyn-serve.pid"
if [ -f "$PID_FILE" ]; then
  oldpid="$(cat "$PID_FILE" 2>/dev/null || true)"
  if [ -n "$oldpid" ] && kill -0 "$oldpid" 2>/dev/null; then
    step "killing stale spectyn serve (pid=$oldpid)"
    kill "$oldpid" 2>/dev/null || true
    sleep 1
    kill -9 "$oldpid" 2>/dev/null || true
  fi
  rm -f "$PID_FILE"
fi

LOG_FILE="$SPECTYN_HOME/serve.log"
step "spawning: nohup $BIN_PATH serve --config $AGENTS_TOML > $LOG_FILE 2>&1 &"
SPECTYN_FORWARD_ON_CAPS_MISMATCH=1 \
SPECTYN_ENFORCE_REQUIRED_CAPS=soft \
nohup "$BIN_PATH" serve --config "$AGENTS_TOML" \
      > "$LOG_FILE" 2>&1 < /dev/null &
echo $! > "$PID_FILE"
pass "spawned pid=$(cat "$PID_FILE")"

sleep 3

# ── 8. verification ─────────────────────────────────────────────────────────
title "setup-android-termux · verification"
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
  fail "last 30 lines of $LOG_FILE:"
  tail -n 30 "$LOG_FILE" >&2 || true
  die "spectyn serve did not come up"
fi

# Try to print the Tailscale-facing address for the operator's convenience.
TS_IP=""
if command -v tailscale >/dev/null 2>&1; then
  TS_IP="$(tailscale ip -4 2>/dev/null | head -1 || true)"
fi
if [ -z "$TS_IP" ]; then
  # Best-effort: scan for a 100.x.y.z address on any interface.
  if command -v ip >/dev/null 2>&1; then
    TS_IP="$(ip -4 addr show 2>/dev/null | awk '/inet 100\./ {print $2}' \
             | head -1 | awk -F/ '{print $1}')"
  fi
fi

# ── done ────────────────────────────────────────────────────────────────────
title "setup-android-termux · DONE"
printf '\n'
printf '%sPASS%s — Node C (mobile / %s) is serving on :%s\n' \
       "$C_GREEN" "$C_RESET" "$SPECTYN_NODE_NAME" "$SPECTYN_PORT"
printf '\n'
printf 'Tailscale address: %s\n' "${TS_IP:-<unknown — check Tailscale app>}"
printf '\n'
printf 'Battery-optimizer reminder:\n'
printf '  Settings → Apps → Termux → Battery → Unrestricted\n'
printf '  Settings → Apps → Termux → notifications → ALLOW (keeps wake-lock alive)\n'
printf '\n'
printf 'Next steps (on your workstation):\n'
printf '  1. Add this node to workstation agents.toml [cluster].peers:\n'
printf '       "http://%s:%s"\n' "${TS_IP:-<tailscale-ip>}" "$SPECTYN_PORT"
printf '  2. Run scripts/spectyn-test/scenarios/three_node_demo.sh to verify.\n'
printf '\n'
printf 'Service controls (on the tablet, in Termux):\n'
printf '  tail -f %s\n' "$LOG_FILE"
printf '  kill "$(cat %s)"     # stop\n' "$PID_FILE"
printf '  bash %s                # restart (re-run this script)\n' "$0"
exit 0
