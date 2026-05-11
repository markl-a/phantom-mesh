# phantom on Android — Install Guide

Two flavors, you can run **both on the same device**:

| Flavor | What it gives you | Install time |
|---|---|---|
| **Tauri APK (thin client)** | Native app icon on the home screen, opens a webview that talks to your Mac/Linux phantom serve. Touch-friendly chat UI. | ~30 s |
| **Termux worker (headless or TUI)** | Real `phantom serve` daemon on the phone. Joins the cluster, accepts dispatched tasks, and gives you the same ratatui TUI you have on Mac. | ~2 min |

> Prerequisites for both: phone connected to the same Tailscale tailnet as
> the coordinator, and the coordinator (a Mac/Linux running
> `phantom serve`) reachable on its Tailscale IP. From the phone:
> `ping <coord-ts-ip>` should succeed.

---

## A. Tauri APK — native thin client

Use this when you want a **home-screen icon** that opens directly to the
phantom mobile UI. Best for the "I just want to chat with my cluster"
experience.

### A.1. Download the APK

In the phone's browser (Chrome / Firefox / Samsung Internet — anything):

```
http://<COORDINATOR-TS-IP>:7878/dist/phantom-mesh-android.apk
```

Replace `<COORDINATOR-TS-IP>` with your Mac/Linux node's Tailscale IP
(e.g. `100.87.93.58`). The APK is ~96 MB (universal: arm64-v8a + armv7
+ x86 + x86_64 in one bundle, signed, v2/v3 schemes).

### A.2. Install

Tap the downloaded APK. Android will show "For your security, your phone
isn't allowed to install unknown apps from this source" — tap **Settings
→ Allow from this source → back → Install**.

You only have to do this once per browser.

### A.3. First launch

Tap the new **Phantom Mesh** icon. You'll see a settings form:

```
Connect to phantom serve

Host:  localhost          ← change to your coordinator's TS IP
Port:  7878
                          [Connect]
```

Fill in the coordinator IP, leave the port at `7878`, tap **Connect**.
The app remembers this in localStorage; subsequent launches go straight
to the chat UI.

### A.4. What you see

The same dark-cream mobile chat UI shipped at `/m` on the coordinator:
ANSI-colored streams, modifier bar (`@` `/` `⇥` `↑` `↓` `■`),
collapsible tool calls, and live SSE token streams.

### A.5. Switching coordinators later

Open Chrome → `phantom://localhost:1430` is **not** how — Tauri's
WebView doesn't expose its localStorage from outside. Instead, just
**uninstall and reinstall** the APK to reset, or use the upcoming
`/Settings` route in the mobile UI (see roadmap below).

---

## B. Termux worker — real `phantom serve` on the phone

Use this when you want the **phone to be an actual cluster member** —
the Mac (or any peer) can dispatch `subagent({node: "<phone-ts-ip>:7879"})`
tasks to it. Also gives you the **ratatui TUI** identical to the Mac
version, just inside Termux.

### B.1. Install Termux from F-Droid

**Important**: install [Termux from F-Droid](https://f-droid.org/en/packages/com.termux/),
**not** from the Play Store. The Play Store version stopped receiving
updates in 2020 and lacks current packages.

### B.2. Run the bootstrap script

Open Termux, paste this **one line** (replace the Groq key with your
own from `~/.phantom-mesh/env` on the coordinator):

```bash
COORD=http://100.87.93.58:7878 \
GROQ_KEY=gsk_your_groq_key_here \
  curl -fsSL "$COORD/scripts/termux-setup.sh" | sh
```

What the script does (~2 min):
- `pkg install` curl/wget/git/termux-tools
- pulls the latest `phantom-aarch64-linux-android` from
  `<COORD>/dist/...`
- writes `~/.phantom-mesh/agents.toml` with the cluster_secret +
  coordinator URL
- starts `phantom serve --port 7879` in the background and verifies
  healthz
- prints a 3-way menu (TUI / browser / cluster worker) so you know
  what to do next

### B.3. Three ways to use it

After the script finishes:

**1) ratatui TUI** — the full-screen interactive UI, identical to Mac:
```bash
phantom
```
This blocks the Termux session. Open a second Termux session if you
want the worker to keep running while you use the TUI.

**2) Browser / PWA** — chat-style UI:
```
http://<phone-ts-ip>:7879/    ← this device's serve
http://<coord-ts-ip>:7878/m   ← Mac coordinator's mobile UI
```
Add to home screen for a PWA-style icon.

**3) Stay headless (worker only)** — Mac dispatches tasks to it:
```ts
mcp__phantom__subagent({
  agent: "master",
  prompt: "echo hello from rog",
  node: "<phone-ts-ip>:7879",
})
```

### B.4. Auto-start on Termux boot (optional)

Install [Termux:Boot](https://f-droid.org/packages/com.termux.boot/),
then create the boot script:

```bash
mkdir -p ~/.termux/boot
cat > ~/.termux/boot/phantom-serve <<'EOF'
#!/data/data/com.termux/files/usr/bin/sh
~/.phantom-mesh/bin/phantom serve >> ~/.phantom-mesh/data/phantom-serve.log 2>&1 &
EOF
chmod +x ~/.termux/boot/phantom-serve
```

After a phone reboot, Termux:Boot launches the worker silently.

---

## Combining A and B on the same device

There's no conflict — A is a webview client, B is a daemon. Common
setup on the ROG:

- **B running headless** → 100.84.223.59:7879 is a true worker the
  cluster can dispatch to.
- **A on the home screen** → tapping it opens the chat UI (pointed at
  either the Mac at `:7878` or its own local serve at `:7879`).

If you point A at the **phone's own** serve (`localhost:7879`), the
phone runs both the UI and its own LLM calls — fully offline-capable
(once provider keys are filled in).

---

## Troubleshooting

### "App not installed as package appears to be invalid"

The APK was downloaded over a flaky connection and is corrupt. Re-download
and verify:

```bash
# On the phone after download:
curl -I http://<coord>:7878/dist/phantom-mesh-android.apk
# Content-Length should be ≥ 90 MB. If not, re-download.
```

### Tauri app shows "Connection refused"

The coordinator's `phantom serve` is not reachable from the phone:

1. Phone Tailscale on? (notification-bar VPN icon)
2. Coordinator `phantom doctor` shows healthz OK?
3. macOS firewall not blocking :7878 outbound? System Settings →
   Network → Firewall → Allow phantom

### Termux script fails on `pkg install`

```bash
pkg upgrade -y
pkg install -y curl wget git termux-tools
# then re-run the bootstrap line
```

### Phone keeps killing the worker (especially Xiaomi/Vivo)

OEM battery-optimization quirks. Settings → Apps → Termux → Battery →
**Unrestricted**. On Xiaomi MIUI you may also need:
Settings → Apps → Termux → Other permissions → Autostart **on**.

### `phantom doctor` row "APFS snapshots: tmutil reachable" looks weird

That row is macOS-only. The Android phantom binary doesn't have the
APFS snapshot tooling (it's `#[cfg(target_os = "macos")]`-gated). On
Android `phantom doctor` simply omits the macOS-integrations section.

### Worker started but Mac can't dispatch — `node` not found

The phone's TS IP may have changed:

```bash
# In Termux:
ip -4 addr show | awk '/100\./ {print $2}' | cut -d/ -f1
```

Update the `node:` argument in your subagent call (or your
`agents.toml` `[cluster].peers`) accordingly.

---

## Verify the round trip

From the Mac (with both phantom serve running on the Mac and the worker
running on the phone):

```bash
# 1. Phone reachable on its Tailscale IP from the Mac:
curl -fsS http://<phone-ts-ip>:7879/healthz
# Expect: ok

# 2. HMAC dispatch through cluster RPC:
SECRET="phantom-cluster-2026"
BODY='{"agent":"master","prompt":"reply: OK from android"}'
AUTH=$(printf '%s' "$BODY" | openssl dgst -sha256 -hmac "$SECRET" -hex | awk '{print $2}')
RESP=$(curl -s -X POST "http://<phone-ts-ip>:7879/rpc/task/assign" \
  -H "X-Cluster-Auth: $AUTH" -H "Content-Type: application/json" -d "$BODY")
JOB=$(echo "$RESP" | sed -n 's/.*"job_id":"\([^"]*\)".*/\1/p')
sleep 5
curl -s "http://<phone-ts-ip>:7879/rpc/task/status/$JOB"
# Expect: {"status":"done","output":"OK from android",...}
```

If both pass, the ROG (or any Android device) is a fully functional
cluster worker.

For a deeper end-to-end validation (15 phases covering CLI / daemon /
endpoint matrix / web / MCP stdio / HMAC / real LLM / cluster /
TUI / autoevolve / Termux:Boot / stress / failure modes / cleanup),
see [`SMOKE-ANDROID.md`](SMOKE-ANDROID.md).

---

## Roadmap (not yet shipped)

- **Inline coordinator switcher** — change host/port from inside the
  app without uninstall/reinstall
- **Foreground Service** in the APK — keep the worker alive even when
  Android wants to kill background apps
- **GitHub Releases** as the canonical APK source — `phantom serve`
  will pull from there instead of needing a peer to host the binary
- **Push notifications** when a long-running task completes
