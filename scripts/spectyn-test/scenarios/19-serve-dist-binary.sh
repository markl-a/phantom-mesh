#!/usr/bin/env bash
source "$SPECTYN_TEST_LIB/common.sh"
source "$SPECTYN_TEST_LIB/cluster-rpc.sh"

scenario "spectyn serve — /dist/ serves a real binary with correct content-type"

ASSERT_HTTP "$(rpc_url)/healthz" 200 "serve up"

# We don't know which platform binary is published locally, so probe a few
# common names and pick the first that returns 200.
declare -a CANDIDATES=(
    "spectyn-x86_64-pc-windows.exe"
    "spectyn-x86_64-unknown-linux-gnu"
    "spectyn-aarch64-apple-darwin"
    "spectyn-aarch64-linux-android"
)
found=""
for name in "${CANDIDATES[@]}"; do
    code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 3 "$(rpc_url)/dist/$name")
    if [ "$code" = "200" ]; then
        found="$name"
        break
    fi
done

if [ -z "$found" ]; then
    warn "no dist binary served at any known name — this peer doesn't publish dist artifacts; skipping"
    exit 77
fi
step "found served binary: $found"

# Headers should declare octet-stream + content-disposition with the filename.
hdrs=$(curl -sSI --max-time 4 "$(rpc_url)/dist/$found")
ASSERT_CONTAINS "$hdrs" "200" "GET /dist/$found returns 200"
ASSERT_CONTAINS "$hdrs" "application/octet-stream" "content-type is binary"
ASSERT_CONTAINS "$hdrs" "$found" "content-disposition mentions filename"

# Download a chunk and check magic bytes.
tmp="$SPECTYN_TEST_TMP/dist-probe.bin"
curl -sS --max-time 10 -o "$tmp" "$(rpc_url)/dist/$found"
size=$(stat -c %s "$tmp" 2>/dev/null)
if [ "${size:-0}" -gt 1000 ]; then
    pass "downloaded $size bytes"
else
    fail "binary suspiciously small: $size bytes"
fi

# Magic byte sanity: PE (Windows) starts MZ (4d5a), ELF starts 7f454c46,
# Mach-O starts cffaedfe / cefaedfe / feedface / feedfacf. We accept any.
hdr=$(xxd -l 4 -p "$tmp" 2>/dev/null)
case "$hdr" in
    4d5a*)                               magic="PE/Windows"  ;;
    7f45*4c46*|7f454c46)                 magic="ELF/Linux"   ;;
    cffaedfe|cefaedfe|feedface|feedfacf) magic="Mach-O"      ;;
    *)                                   magic="unknown"     ;;
esac
if [ "$magic" != "unknown" ]; then
    pass "binary header recognized as $magic (first 4 bytes: $hdr)"
else
    fail "binary header not recognized: $hdr"
fi

[ "$SPECTYN_TEST_FAILED" -eq 0 ]
