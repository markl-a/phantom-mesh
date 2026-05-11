# iOS End-to-End Test Flow

Run-book for verifying the iOS thin client (sim + iPhone + iPad) talks
to a Mac coordinator over the cluster-dispatch HMAC wire. Targeting
the post-`9607f44` state of `platform/ios`.

Total wall time on a warm tree: ~10 min (most is build + install).
Cold tree (no DerivedData / Externals): ~25 min.

---

## 0. Prerequisites

Run all commands from
`/Users/marklight/Documents/workspace/hailmary/phantom-mesh-ios`
unless noted otherwise.

```bash
# Branch + worktree
git checkout platform/ios
git pull --ff-only origin platform/ios

# Coordinator on Mac (must be reachable from devices over Tailscale
# or the same Wi-Fi)
curl -s -m 5 http://127.0.0.1:7878/healthz   # → "ok"
grep cluster_secret ~/.phantom-mesh/agents.toml   # capture for step 5

# Devices paired + on same network as Mac (`localNetwork` transport
# means they're already wifi-debug ready)
xcrun devicectl list devices
#  iPhone 13 mini  (HITRON-9527)   00008110-001134A026F2801E   localNetwork
#  iPad Pro 12.9"  (MarL 的 iPad)  00008027-0018296E2E22002E   localNetwork
```

Devices must be **unlocked** at install time so the developer disk
image can mount; otherwise `kAMDMobileImageMounterDeviceLocked`.

---

## 1. Build — wire-level test (skips Xcode entirely)

Confirms the coordinator + cluster-dispatch contract before involving
any iOS toolchain. Catches coordinator regressions in seconds.

```bash
cat > /tmp/test-cluster-dispatch.mjs <<'EOF'
import crypto from "node:crypto";
const COORD = "http://127.0.0.1:7878";
const SECRET = "<paste cluster_secret here>";
const body = JSON.stringify({ agent: "master", prompt: "1+1=" });
const auth = crypto.createHmac("sha256", SECRET).update(body).digest("hex");
const r1 = await fetch(`${COORD}/rpc/task/assign`, {
  method: "POST",
  headers: { "Content-Type": "application/json", "X-Cluster-Auth": auth },
  body,
});
const { job_id } = await r1.json();
console.log(`assign → ${job_id}`);
for (let i = 0; i < 60; i++) {
  await new Promise(r => setTimeout(r, 500));
  const j = await (await fetch(`${COORD}/rpc/task/status/${job_id}`)).json();
  if (j.status === "done")  { console.log(`done: ${j.output}`); process.exit(0); }
  if (j.status === "error") { console.log(`error: ${j.error}`); process.exit(1); }
}
console.log("timeout"); process.exit(1);
EOF
node /tmp/test-cluster-dispatch.mjs
```

Pass: prints `done: 2` (or similar arithmetic answer) within ~10 s.

If this fails, no point continuing — fix the coordinator side
(`/rpc/task/assign`, `core/src/mesh.rs::make_auth_token_bytes`, agent
runtime providers in `~/.phantom-mesh/agents.toml`) before touching
iOS.

---

## 2. Build — iOS simulator app

```bash
./scripts/package-ios.sh --sim
#  → dist/phantom-ios-sim.app  (~97 MB, debug, unsigned)
```

Cold tree the script will:
1. Pre-create the `Externals/{arm64,x86_64}/{debug,release}` dir tree
   so `xcodegen generate` validates.
2. Wipe DerivedData + the opposite-config dirs to avoid `duplicate
   output file libapp.a`.
3. Stub `Externals/arm64/debug/libapp.a` so xcode's upfront input
   check passes pre-script-phase.
4. Run `npx tauri ios build --debug --target aarch64-sim --no-sign
   --ci --ignore-version-mismatches`.

If you see `error: The file "libapp.a" couldn't be opened because
there is no such file` on a cold run, retry once — known flaky xcode
race. The script's stubbing should prevent it but Xcode's incremental
state isn't fully deterministic.

---

## 3. Verify in simulator

Pick a sim that's already booted (Xcode menu → Open Developer Tool →
Simulator) or boot one explicitly:

```bash
SIM_ID=$(xcrun simctl list devices available \
         | grep -E "iPhone 17 Pro " | grep -oE "[A-F0-9-]{36}" | tail -1)
xcrun simctl boot      "$SIM_ID" 2>/dev/null || true
open -a Simulator      --args -CurrentDeviceUDID "$SIM_ID"
xcrun simctl terminate "$SIM_ID" ai.phantommesh.app 2>/dev/null
xcrun simctl uninstall "$SIM_ID" ai.phantommesh.app 2>/dev/null
xcrun simctl install   "$SIM_ID" dist/phantom-ios-sim.app
xcrun simctl launch    "$SIM_ID" ai.phantommesh.app
sleep 4
xcrun simctl io        "$SIM_ID" screenshot /tmp/ios-sim-launch.png
```

Log expectations (`xcrun simctl spawn "$SIM_ID" log show --last 30s
--predicate 'process == "Phantom Mesh"' --info`):

- ✓ `PhantomMeshRuntime ready in 0.0Xs`
- ✓ NO `HTTP server failed: Address already in use`
  (`lib.rs` cfg(desktop)-gate, commit `1abf2f5`)
- ⚠ `agents.toml not found; checked: [...]` is expected and harmless;
  cluster mode doesn't need a local agents.toml.

Screenshot expectations (`/tmp/ios-sim-launch.png`):

- 3 bottom tabs: 對話 / 節點 / 設定
- Chat tab header has cluster toggle (disabled until configured)
- Welcome state shows prompt cards (學習 / 工作 / 生活 / 創作)

---

## 4. Build + install — device IPA over wifi

```bash
./scripts/package-ios.sh
#  → dist/phantom-ios.ipa  (~31 MB, signed by team F7683B69U7)
```

`bundle.iOS.developmentTeam` is hardcoded in `tauri.conf.json`; no
env var needed. Free Apple Development cert → IPA expires 7 days
after build.

```bash
IPHONE=00008110-001134A026F2801E
IPAD=00008027-0018296E2E22002E
IPA=dist/phantom-ios.ipa

xcrun devicectl device install app --device "$IPHONE" "$IPA"
xcrun devicectl device install app --device "$IPAD"   "$IPA"
xcrun devicectl device process launch --device "$IPHONE" ai.phantommesh.app
xcrun devicectl device process launch --device "$IPAD"   ai.phantommesh.app
```

Verify:

```bash
xcrun devicectl device info processes --device "$IPHONE" | grep -i phantom
xcrun devicectl device info processes --device "$IPAD"   | grep -i phantom
# Each prints one PID + bundle path under
# /private/var/containers/Bundle/Application/<GUID>/Phantom Mesh.app/Phantom Mesh
```

If launch errors with `FBSOpenApplicationErrorDomain error 7
(Locked)`, unlock the device and retry — install succeeded but
launch needs the device awake.

---

## 5. Cluster setup (on device)

Each device, manually:

1. Tap **Phantom Mesh** → lands on chat tab.
2. Type any prompt → **Send**. Red banner appears:
   `尚未設定 cluster — 點此到「設定 → Cluster 派送」`.
3. Tap the banner. Routes to `/settings/cluster` via the deep-link
   wired in commit `e1f4b14`.
4. Fill in:
   - **Coordinator URL**: `http://100.87.93.58:7878`
     (Mac Tailscale IP; replace with whatever `tailscale ip -4`
     reports on the coordinator).
   - **Cluster Secret**: paste the `cluster_secret` value from
     `~/.phantom-mesh/agents.toml` on the Mac.
5. Tap **測試 dispatch**. On ✓:
   - `回應：<short answer> (<ms>ms, job <8-char>)` shown.
   - Cluster mode toggle auto-flips ON (commit `e1f4b14`).
6. Tap the back chevron → returns to chat tab. Cluster toggle
   in the chat header should now be green.

---

## 6. Forward dispatch — chat through cluster

On the device:

1. Type `1+1=` → **Send**.
2. Spinner shows briefly, then the answer (`2`) appears.
3. Reset conversation (trash icon) and try a longer prompt:
   `用 5 句話講解 Rust ownership`. Expect a multi-paragraph response
   in 5–15 s depending on the agent's provider.

On the Mac (coordinator side, optional — confirms requests are
arriving):

```bash
# Tail the coordinator log for incoming /rpc/task/assign hits
journalctl --user -u phantom-mesh -f 2>/dev/null \
  || tail -f ~/.phantom-mesh/logs/app.log.* 2>/dev/null \
  || true
```

Each successful chat bubble corresponds to one POST to
`/rpc/task/assign` + N polls of `/rpc/task/status/<id>`.

---

## 7. Failure-mode checklist

Quick triage table for what each visible failure means:

| Symptom on device | Likely cause | Fix |
|---|---|---|
| Red banner won't go away after configuring | Test dispatch failed → toggle didn't auto-enable | Look at banner detail; hit 測試 again, fix URL/secret |
| `assign 401: unauthorized` | Wrong cluster_secret | Re-paste from coordinator's `agents.toml` |
| `assign 404` or fetch failure | Coordinator unreachable | Check Tailscale on device; `curl <URL>/healthz` from Mac |
| `timeout after 60000ms` | Agent runtime stalled (provider rate-limit / no api-key) | Check coordinator log for the failing provider |
| `(no output)` in chat bubble | Agent succeeded but returned empty | Provider issue, not iOS issue |
| App white-screen on launch | Stale frontend bundle in binary | Touch `app/src-tauri/src/lib.rs` + rebuild (cargo's `rerun-if-changed=../dist` should handle it but a manual touch forces it) |
| App crashes immediately | Tauri ABI break or symbol-strip went too far | Check `xcrun simctl spawn <SIM> log show` for panic / SIGABRT |

---

## 8. Free-cert renewal

Free Apple Development certs expire 7 days after the IPA is built.
A `/schedule` routine (`trig_01VySWaHMoTodsWcqZvRQtKA`) auto-opens a
GitHub issue every Thursday 09:00 Asia/Taipei reminding to rerun
steps 4-5 of this doc.

After rebuild + reinstall, the localStorage settings
(`phantom-mesh-cluster-mode` Zustand persist key) survive — devices
don't need re-configuration unless the user uninstalls.
