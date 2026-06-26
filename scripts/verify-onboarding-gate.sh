#!/usr/bin/env bash
# Hermetic verification of the onboarding permission/trust gate.
# Uses a throwaway HOME — does NOT touch your real ~/.phantom-mesh.
# Run from the repo root:  bash scripts/verify-onboarding-gate.sh
set -u
cd "$(dirname "$0")/.." || exit 1

echo "▶ building phantom (debug)…"
( cd core && cargo build --bin phantom 2>/dev/null ) || { echo "build failed"; exit 1; }
BIN="$PWD/core/target/debug/phantom"

H=$(mktemp -d); C=$(mktemp -d)            # throwaway HOME + project dir
mkdir -p "$H/.phantom-mesh"; printf 'x' > "$H/.phantom-mesh/identity.key"
cfg="$H/.phantom-mesh/agents.toml"
say() { printf '\n\033[36m== %s ==\033[0m\n' "$1"; }
run() { ( cd "$C" && HOME="$H" "$BIN" "$@" ); }
wrote() { run tool file_write --args "{\"path\":\"$C/x.txt\",\"content\":\"hi\"}" 2>&1 | head -1; }
read1(){ run tool file_read  --args "{\"path\":\"$cfg\"}" 2>&1 | head -1; }

printf '[providers.groq]\ntype="groq"\napi_key_env="GROQ_API_KEY"\n\n[permissions]\nprofile="observe"\n' > "$cfg"
say "profile = observe  (read-only)"
echo "permissions list:"; run permissions list 2>&1 | sed 's/^/  /'
echo "file_write → expect [denied]:"; echo "  $(wrote)"
echo "file_read  → expect allowed:";  echo "  $(read1 | cut -c1-40)"

say "escape hatch  PHANTOM_TRUST_ALL=1  → expect write allowed"
esc=$( cd "$C" && PHANTOM_TRUST_ALL=1 HOME="$H" "$BIN" tool file_write \
        --args "{\"path\":\"$C/y.txt\",\"content\":\"hi\"}" 2>&1 | head -1 )
echo "  $esc"

say "malformed home config  → expect FAIL-CLOSED [denied]"
echo 'this is { not valid toml' > "$cfg"
echo "  $(read1)"

say "cwd config CANNOT weaken home policy  → expect [denied]"
printf '[providers.groq]\ntype="groq"\napi_key_env="GROQ_API_KEY"\n\n[permissions]\nprofile="observe"\n' > "$cfg"
printf '[permissions]\nprofile="developer-full"\n' > "$C/agents.toml"
echo "  $(wrote)"

say "trust enforcement = observe  → untrusted denies write, trusted allows"
printf '[providers.groq]\ntype="groq"\napi_key_env="GROQ_API_KEY"\n\n[trust]\nenforcement="observe"\n' > "$cfg"; rm -f "$C/agents.toml"
echo "untrusted → expect [denied]:"; echo "  $(wrote)"
run trust add >/dev/null 2>&1
echo "after 'trust add' → expect allowed:"; echo "  $(wrote)"

say "your REAL doctor (read-only, your config)"
"$BIN" doctor 2>&1 | sed 's/^/  /' | head -40

rm -rf "$H" "$C"
echo; echo "✓ done — temp dirs cleaned; your ~/.phantom-mesh was untouched."
