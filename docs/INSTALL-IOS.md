# phantom on iOS — Install Guide

iOS is a **thin client only**. The phone runs the UI; all tools, LLM
calls, and the agent loop run on a Mac/Linux/Windows phantom serve
node it talks to over Tailscale. This is a hard limit of the iOS
sandbox model — see SESSION_RESUME and the gap-analysis docs for
details.

If you want a real, on-device agent, use Android (Termux + phantom
binary) or a desktop OS.

---

## What you get

A signed `phantom-mesh-ios.ipa` (~7 MB, arm64) built from the same
React thin-shell as the Android APK. On launch it asks for a
coordinator host/port, then loads `<host>:<port>/m` (the mobile chat
UI we ship in `core/web/mobile.html`).

The IPA is signed with a free Apple Development certificate — that
means:
- It works on phones already enrolled with your Apple ID
- It expires after **7 days** (Apple policy for free dev certs)
- You can sideload to up to **3 devices** per Apple ID

For longer-lived install on more devices you'd need a paid Apple
Developer account and re-sign with a `iPhone Distribution` cert; we
do not ship that flow.

---

## Prerequisites

- Mac running Xcode (or just Apple Configurator 2)
- Free Apple Developer account already registered with the same
  Apple ID you sign into your iPhone with
- iPhone or iPad on iOS 16+ connected to the same Tailscale tailnet
  as the coordinator
- USB-C / Lightning cable — the IPA is sideloaded via cable, not
  over the air

---

## Get the IPA

On any device on the tailnet:

```bash
curl -O http://100.87.93.58:7878/dist/phantom-mesh-ios.ipa
```

(Replace the IP with your coordinator's Tailscale address.)

Or download via Safari — same URL, save to Files / Downloads.

---

## Install — three options

### Option A: Apple Configurator 2 (Mac App Store, free)

The simplest path if your Mac already has Xcode installed (cert chain
is already trusted).

1. Open Apple Configurator 2 on the Mac
2. Plug iPhone in via cable, unlock, "Trust this computer"
3. Drag `phantom-mesh-ios.ipa` onto the device tile
4. Wait ~30 s for installation
5. On the iPhone: Settings → General → VPN & Device Management →
   trust the developer profile (your Apple ID)

### Option B: Sideloadly (third-party, free)

If you don't want Configurator, [Sideloadly](https://sideloadly.io)
does the same job through a friendlier UI. Drag-drop the IPA, sign
with your Apple ID at install time, hit Start.

### Option C: Xcode Devices window

For developers who already have a project workspace open. Window →
Devices and Simulators → drag IPA into the "Installed Apps" pane.

---

## First launch

Tap the **Phantom Mesh** icon. You'll see a settings form (the same
DOM-API form we ship to Android — fixed in 0a039ba so the Connect
button doesn't dead-pixel under Tauri's CSP):

```
Connect to phantom serve
Host:  localhost          ← change to coordinator TS IP
Port:  7878
                          [Connect]
```

Fill in the coordinator IP (`100.87.93.58` for the default reference
deployment), tap **Connect**. The webview navigates to the mobile
chat UI; subsequent launches go straight there.

---

## Renew before 7 days expire

A free-cert IPA stops launching exactly 7 days after install. Two ways
to refresh it:

### One-time Xcode setup

Before the first rebuild, sign Xcode into the Apple ID that owns the
dev cert:

1. Open Xcode → Settings (⌘,) → Accounts
2. Click `+` → Apple ID → sign in
3. Confirm the team appears under "Manage Certificates…"

Without this, `make ios-rebuild` fails with "No profiles for
'ai.phantommesh.app'". The keychain identity alone (visible in
`security find-identity`) is not enough — automatic provisioning
needs the live Apple ID session for Xcode to fetch profiles from
developer.apple.com.

### Manual one-shot

```bash
# On the coordinator (Mac):
APPLE_TEAM_ID=YX7U4J39PX make ios-rebuild
```

(Find your team id with
`security find-identity -v -p codesigning | grep Apple` — it's the
10-char string in parentheses.)

The fresh IPA lands at `dist/phantom-mesh-ios.ipa`. Sideload from a
machine that has Apple Configurator:

```bash
curl -O http://<coord>:7878/dist/phantom-mesh-ios.ipa
```

### Auto-rebuild every Sunday (recommended)

Install a per-user LaunchAgent that runs `make ios-rebuild` weekly:

```bash
APPLE_TEAM_ID=YX7U4J39PX ./scripts/install-ios-rebuild-agent.sh
```

Schedule: every Sunday 03:30. Logs at
`~/Library/Logs/phantom-ios-rebuild.log`.

Caveats:
- The Mac must be awake at the scheduled time (or set up `pmset` to
  wake for launchd jobs)
- The login keychain must be unlocked when the job runs — otherwise
  codesign can't read the cert's private key
- Apple's automatic provisioning needs network access and a logged-in
  Apple ID in Xcode preferences

Force a manual run (good first-time validation):

```bash
launchctl kickstart -k gui/$(id -u)/ai.phantommesh.ios-rebuild
tail -f ~/Library/Logs/phantom-ios-rebuild.log
```

Uninstall:

```bash
launchctl bootout gui/$(id -u)/ai.phantommesh.ios-rebuild
rm ~/Library/LaunchAgents/ai.phantommesh.ios-rebuild.plist
```

---

## Troubleshooting

### "Untrusted Developer" — app won't launch

Settings → General → VPN & Device Management → tap the Apple ID
under DEVELOPER APP → Trust.

### Connect button does nothing

If you see this on a build older than 0a039ba (April 28, 2026), the
inline event handler is being blocked by Tauri's default CSP. Pull
the latest IPA — the fix replaces document.write + onclick with
proper DOM API + addEventListener.

### Settings form keeps reappearing after Connect

The webview's localStorage write succeeded but failed to persist
across the settings → loaded-coordinator transition. The 0a039ba fix
also covers this — it navigates directly instead of relying on a
reload to re-read localStorage.

### "App could not be installed at this time"

The provisioning profile inside the IPA doesn't include this device's
UUID. Either:
- Open the IPA project in Xcode at least once with this iPhone
  connected — Xcode auto-adds the device UDID to the dev profile
- Or use a paid Apple Developer account with an explicit
  provisioning profile

### Phantom serve unreachable in the app

Same diagnostic as Android — see TROUBLESHOOTING-MAC.md
"healthz unreachable" section. Most common: Tailscale not connected
on the iPhone (Settings → Tailscale → Connect).

---

## What's next on iOS (roadmap)

- **In-app coordinator switcher** — change host without reinstalling
- **Push notifications** when long-running tasks complete
  (`@tauri-apps/plugin-notification` is already in package.json)
- **Local LLM inference** via Apple Foundation Models framework
  (macOS 26+ / iOS 26+) — phantom would talk to the on-device LLM
  through a small Swift shim. Same idea as the planned MLX
  integration on the Mac side.
- **Truly local agent loop** — write a Swift native re-implementation
  of the agent loop that runs in the iOS sandbox, with whatever tools
  the sandbox allows (file_read/file_write within the app's container,
  HTTPS fetch, no shell). This is a multi-week project and is **not**
  on the demo path.

For the foreseeable future, the Mac/Linux/Windows coordinator does
all the real work and the iOS app is purely the UI that pretends to
do it.
