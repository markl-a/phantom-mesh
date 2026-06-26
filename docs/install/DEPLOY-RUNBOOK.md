# Phantom Mesh — Deploy Runbook (fresh box → installed + logged in + owned-memory usable)

This is the turnkey deploy guide for standing up phantom-mesh on a clean
machine across all five OS targets. The goal for each target is the same:
**go from a fresh box to a node that is installed, has an LLM key, is running
`phantom serve`, and can store + recall a private memory** — the "owned
memory" moat that makes phantom compound the more you use it.

> One source of truth: build from `origin/main`. The per-OS install docs
> (`INSTALL-WINDOWS.md`, `INSTALL-MAC.md`, `INSTALL-ANDROID.md`,
> `INSTALL-IOS.md`, `INSTALL-LINUX.md`) cover the everyday happy path. This
> runbook is the **deploy** view: it pins the build to the owned-memory
> learning loop and ends every section on a memory-recall verification.

---

## 0. Read this first — the owned-memory build

Slice-1 recall (inject relevant past notes before each run, capture on deny)
is **default-on** and needs no feature flags — a plain `cargo build` ships it,
and the kill-switch is the env var `PHANTOM_OWNED_MEMORY` (only `0` / `false`
/ `off` / `no` disable it; anything else, including unset, leaves it ON).

To get the **full learning loop** — the judge → extract → store auto-learning
that turns finished runs into reusable skills — build the CLI with the three
hermes features:

```
--features experimental-hermes-curator,experimental-hermes-memory,experimental-hermes-tools
```

Everywhere below where a deploy builds the `phantom` binary from source, it
uses exactly those features. Verified behaviour: recall works, and the
six-step loop (judge → extract → store → recall → apply → measure)
round-trips.

Common facts across all targets:

- The data root is `~/.phantom-mesh` (on Windows, `%USERPROFILE%\.phantom-mesh`).
  The owned-memory database lives at `~/.phantom-mesh/phantom.db`. Everything
  except the binary lives under `$HOME` — easy to back up, easy to wipe. You
  can redirect the data root with `PHANTOM_HOME` and the database alone with
  `PHANTOM_DB_PATH`.
- phantom is **BYOK (bring your own key)** — it never ships a provider key.
  Supply at least one of Anthropic / OpenAI / Groq / Gemini / OpenRouter, via
  the `phantom onboarding` wizard or an env var.
- Every **mobile / thin client** (Android app, iOS app) and every **remote
  client** needs **Tailscale joined to the same tailnet** as a running
  `phantom serve` coordinator. The mobile apps reach that coordinator by its
  Tailscale IP; use the placeholder `<coordinator-tailscale-ip>` throughout.
- Verify health any time with `phantom doctor` (11 colour-coded sections; `✓`
  green or `⚠` yellow are fine — `⚠` is normal for opt-in features you have
  not turned on; only a red `✗` needs fixing).

---

## 1. Windows (desktop / laptop node)

A Windows node can be a full coordinator (`phantom serve`) and a desktop-app
host. Tested native on Windows 11 (not WSL). PowerShell 7 recommended.

### 1.1 Prerequisites

```powershell
winget install --silent Git.Git Rustlang.Rustup tailscale.tailscale
winget install --silent --id Microsoft.PowerShell
# Close + reopen PowerShell so PATH is fresh, then:
$env:PATH = "$env:USERPROFILE\.cargo\bin;C:\Program Files\Git\cmd;$env:PATH"
```

If `cargo` later fails to find `link.exe`, install the MSVC build tools:

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

### 1.2 Build (owned-memory features, Defender-safe out-of-tree target)

Clone `origin/main` and build via the bundled helper. `scripts\build-windows.ps1`
puts `CARGO_TARGET_DIR` **outside** the worktree (default
`D:\tmp\phantom-windows-target`) so Defender's real-time scan does not lock
freshly-emitted `.exe`/build-script artifacts mid-link (the `LNK1104` /
`access denied (os error 5)` failure). It also stops any running `phantom.exe`
first so the link step can overwrite the binary.

```powershell
cd $env:USERPROFILE\Documents
git clone https://github.com/markl-a/phantom-mesh
cd phantom-mesh

# Build the CLI from origin/main WITH the owned-memory learning loop, then
# deploy it to the canonical install paths (~/.phantom-mesh/bin + ~/.local/bin).
$env:CARGO_BUILD_FLAGS = ""   # (none needed)
.\scripts\build-windows.ps1 -Deploy `
  -TargetDir D:\tmp\phantom-windows-target
```

> The helper builds `cargo build --release --bin phantom`. To bake in the
> learning-loop features, either build directly with them:
>
> ```powershell
> $env:CARGO_TARGET_DIR = "D:\tmp\phantom-windows-target"
> cd core
> cargo build --release --bin phantom `
>   --features experimental-hermes-curator,experimental-hermes-memory,experimental-hermes-tools
> # then copy target\release\phantom.exe to ~\.phantom-mesh\bin and ~\.local\bin
> ```
>
> or run `.\scripts\build-windows.ps1 -Deploy` for the default build and add
> the features on your next rebuild — slice-1 recall works either way; the
> features only add the auto-learning leg.

`-Deploy` copies `phantom.exe` to:
- `%USERPROFILE%\.phantom-mesh\bin\phantom.exe` (used by the serve Scheduled Task)
- `%USERPROFILE%\.local\bin\phantom.exe` (on the PowerShell user PATH)

Confirm: `phantom --version`.

### 1.3 Login / provider key

```powershell
phantom onboarding      # 90-second wizard, writes %USERPROFILE%\.phantom-mesh\agents.toml
```

Or skip the wizard and set a key in the environment (example: OpenRouter):

```powershell
setx OPENROUTER_API_KEY "sk-or-..."   # persists; reopen the shell to pick it up
```

### 1.4 Start serve

Run it as a logon Scheduled Task so it survives reboots:

```powershell
$action   = New-ScheduledTaskAction -Execute "$env:USERPROFILE\.phantom-mesh\bin\phantom.exe" -Argument "serve"
$trigger  = New-ScheduledTaskTrigger -AtLogon
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask -TaskName "phantom-serve" -Action $action -Trigger $trigger -Settings $settings
Start-ScheduledTask -TaskName "phantom-serve"
```

(For a quick foreground run instead: `Start-Process phantom -ArgumentList "serve" -WindowStyle Hidden`.)

If this node is a coordinator for mobile clients, also bring up Tailscale and
open the port:

```powershell
tailscale up
New-NetFirewallRule -DisplayName "phantom serve" -Direction Inbound -LocalPort 7878 -Protocol TCP -Action Allow
```

### 1.5 Desktop app (optional)

```powershell
.\scripts\package-windows.ps1 -Sign        # builds .msi + setup.exe, dev self-signs them
```

The dev self-signed cert is structurally valid but **SmartScreen will warn**
until a production EV/Authenticode cert lands. Install the `.msi`/`-setup.exe`
from `dist\`; if SmartScreen blocks it, right-click → Run / "More info → Run
anyway".

### 1.6 Verify memory recall works

```powershell
phantom doctor
phantom note "deploy check: my favorite test fruit is durian" --tag selftest
phantom recall "favorite test fruit"
```

The `recall` output should contain the note you just stored. See §6 for the
full moat check.

---

## 2. macOS (Apple Silicon — recommended coordinator; also the iOS build host)

A Mac is the recommended coordinator: native launchd auto-start, APFS
snapshot rollback, optional on-device MLX LLM, and the deepest `doctor`.
It is also the **only host that can build the iOS app** (see §5).

### 2.1 Prerequisites

- macOS 13+ (Apple Silicon strongly recommended).
- Rust toolchain (`rustup`).
- At least one provider API key (BYOK).
- For iOS builds: Xcode 15+ (`xcode-select --install`, plus
  `xcodebuild -downloadPlatform iOS`).
- Optional: a Tailscale account for the mesh.

### 2.2 Build (owned-memory features)

```bash
git clone https://github.com/markl-a/phantom-mesh
cd phantom-mesh/core
cargo install --path . \
  --features experimental-hermes-curator,experimental-hermes-memory,experimental-hermes-tools
phantom --version
```

`cargo install --path .` puts `phantom` on PATH at `~/.cargo/bin/phantom`.
First build on Apple Silicon is ~2 minutes.

### 2.3 Login / provider key

```bash
phantom onboarding     # writes ~/.phantom-mesh/agents.toml, runs doctor, prints next steps
```

(Or export a key, e.g. `export OPENROUTER_API_KEY=sk-or-...` in your shell rc.)

### 2.4 Start serve (launchd, auto-start at every login)

```bash
phantom service install     # copies a TCC-safe binary + loads ai.phantommesh.serve.plist
phantom service status      # registered / pid / healthz
```

If this Mac is the coordinator for mobile clients, join Tailscale
(`tailscale up`) and confirm the `network` row of `phantom doctor` is green.

### 2.5 Desktop app (optional)

```bash
./scripts/package-macos.sh        # builds + signs the .app/.dmg into dist/
```

On a machine with only an Apple Development (or ad-hoc) identity, **Gatekeeper
rejects for distribution** — that is expected. To open the app locally:
right-click the `.app` → Open → Open. A production Developer ID cert +
notarization is deferred.

### 2.6 Verify memory recall works

```bash
phantom doctor
phantom note "deploy check: my favorite test fruit is durian" --tag selftest
phantom recall "favorite test fruit"
```

The note should appear in the recall output. See §6.

---

## 3. Linux (server / desktop node)

A Linux box is a first-class coordinator (often the cheapest always-on one).

### 3.1 Prerequisites

- Rust toolchain (`rustup`), `git`, `curl`, build-essential (a C linker).
- At least one provider API key (BYOK).
- Optional: Tailscale (`curl -fsSL https://tailscale.com/install.sh | sh`).

### 3.2 Build (owned-memory features)

```bash
git clone https://github.com/markl-a/phantom-mesh
cd phantom-mesh/core
cargo install --path . \
  --features experimental-hermes-curator,experimental-hermes-memory,experimental-hermes-tools
phantom --version
```

> A prebuilt binary is also served by `scripts/install.sh`
> (`curl -fsSL https://phantommesh.io/install | sh`, SHA256-verified,
> installs to `~/.phantom-mesh/bin/phantom`). The prebuilt path is the
> fastest first touch, but **build from source with the features above** when
> you want the full learning loop on a deploy node.

### 3.3 Login / provider key

```bash
phantom onboarding
# or: export OPENROUTER_API_KEY=sk-or-...   (in ~/.bashrc / ~/.profile)
```

### 3.4 Start serve (systemd or foreground)

```bash
phantom service install      # installs a systemd user/service unit where supported
phantom service status
# Fallback if service install is unavailable on this distro:
#   nohup phantom serve >> ~/.phantom-mesh/phantom-serve.log 2>&1 &
```

If this node is a coordinator for mobile clients: `tailscale up`, then verify
the `network` row in `phantom doctor`.

### 3.5 Verify memory recall works

```bash
phantom doctor
phantom note "deploy check: my favorite test fruit is durian" --tag selftest
phantom recall "favorite test fruit"
```

See §6.

---

## 4. Android (arm64 device)

Two independent flavors, both possible on the same phone:

- **Tauri APK (thin client)** — a home-screen app that talks to a `phantom
  serve` coordinator over Tailscale. This is the supervisor UI. **Recommended
  for a deploy.**
- **Termux worker (headless / TUI)** — a real on-device `phantom serve` that
  joins the cluster. Heavier; see `INSTALL-ANDROID.md` part B.

This section covers the APK thin client.

### 4.1 Prerequisites

- An arm64 (aarch64) Android device.
- Tailscale installed on the phone and joined to the **same tailnet** as your
  coordinator. From the phone, `ping <coordinator-tailscale-ip>` must succeed.
- A reachable coordinator already running `phantom serve` (a Windows/macOS/
  Linux node from §1–§3, built with the owned-memory features).

### 4.2 Build the arm64 APK

On a build host with the Android toolchain (Android SDK/NDK + Rust
`aarch64-linux-android` target + Node):

```bash
cd phantom-mesh/app
npm install
npx tauri android build --target aarch64
```

The signed arm64 APK lands under the Tauri android output
(`app/src-tauri/gen/android/.../*.apk`). Copy it to the device, or serve it
from the coordinator (`http://<coordinator-tailscale-ip>:7878/dist/...`) and
download it in the phone browser.

### 4.3 Install

```bash
adb install path/to/phantom-mesh-arm64.apk
```

(Or tap the downloaded APK on the device and allow "install from this
source" once for that browser.)

### 4.4 Configure the supervisor (login / reach the coordinator)

Launch the **Phantom Mesh** app. It is a 7-tab supervisor. Point it at your
coordinator:

- **Base URL**: `http://<coordinator-tailscale-ip>:7878`
- **HMAC secret**: the cluster shared secret configured on the coordinator
  (the `cluster_secret` / `PHANTOM_CLUSTER_SECRET` value).

The app stores this locally; subsequent launches go straight to the UI. The
agent loop, LLM calls, and owned-memory all run on the **coordinator** — the
phone is a control surface.

### 4.5 Verify memory recall works (via the coordinator)

Because the agent runs on the coordinator, verify the moat **there** (§6) —
e.g. `phantom note ...` + `phantom recall ...` on the coordinator, then
confirm the same note surfaces in a chat you drive from the Android app. A
quick reachability check from the phone:

```
http://<coordinator-tailscale-ip>:7878/healthz   → ok
```

---

## 5. iOS / iPadOS (TestFlight thin client)

iOS is a **thin client only** — no on-device agent loop. The phone runs the
UI; all tools, LLM calls, and the agent/owned-memory live on a desktop
coordinator it reaches over Tailscale. iOS apps are built **on a Mac** (§2).

### 5.1 Prerequisites

- A Mac with Xcode 15+ (the iOS build host from §2).
- An App Store Connect app record for bundle id **`ai.phantommesh.app`**, plus
  an App Store Connect API key (key id + issuer id) for the upload.
- The phone joined to the **same tailnet** as a running `phantom serve`
  coordinator.

### 5.2 Build the IPA (TestFlight / app-store-connect export)

On the Mac, first make sure Xcode is signed into the Apple ID that owns the
team (Xcode → Settings → Accounts), then:

```bash
TESTFLIGHT=1 APPLE_TEAM_ID=F7683B69U7 bash scripts/package-ios.sh
```

`TESTFLIGHT=1` flips the export method to `app-store-connect` (distribution
cert) and stamps a fresh, strictly-increasing `CFBundleVersion`. The signed
IPA lands at `dist/phantom-mesh-ios.ipa`.

### 5.3 Upload to TestFlight

```bash
xcrun altool --upload-app -f dist/phantom-mesh-ios.ipa \
  --type ios --apiKey "$ASC_API_KEY_ID" --apiIssuer "$ASC_API_KEY_ISSUER_ID"
```

(Or via Xcode: Organizer → Distribute App → App Store Connect → Upload.)
Once processing finishes in App Store Connect, the build is available in
TestFlight.

### 5.4 Install + login

Install via the **TestFlight** app on the device. Launch **Phantom Mesh**, and
in the connect form set the coordinator host/port:

- **Host**: `<coordinator-tailscale-ip>`
- **Port**: `7878`

Tap Connect; the webview loads the coordinator's mobile UI. If the app cannot
connect, the most common cause is Tailscale not connected on the phone
(Settings → Tailscale → Connect).

### 5.5 Verify memory recall works (via the coordinator)

As with Android, the moat lives on the coordinator. Verify there per §6, then
confirm the stored note shows up in a chat driven from the iOS app. Quick
reachability from any tailnet device:

```bash
curl -fsS http://<coordinator-tailscale-ip>:7878/healthz   # → ok
```

---

## 6. Verify the moat — store a note, confirm recall injects it

This is the single check that proves owned-memory is live. Run it on any node
that runs the agent loop (any desktop coordinator from §1–§3). Mobile clients
verify against their coordinator.

```bash
# 1. Store a private fact as an event in ~/.phantom-mesh/phantom.db
phantom note "deploy check: the magic deploy phrase is BLUE-OTTER-42" --tag selftest

# 2. Full-text recall (FTS5) — the stored note must come back
phantom recall "magic deploy phrase"
# → expect a line containing BLUE-OTTER-42

# 3. Confirm the agent loop would INJECT it before a run
#    (this is the exact system block recall prepends to the model context)
phantom skill recall "magic deploy phrase"
# → expect the BLUE-OTTER-42 memory in the recalled block,
#   NOT "(no skills recalled for this query)"
```

(On Windows, the same three commands run unchanged in PowerShell.)

What this proves:
- **Storage**: `phantom note` wrote an encrypted-at-rest event into the
  owned-memory database under `~/.phantom-mesh`.
- **Recall**: `phantom recall` retrieved it via the FTS5 hot path.
- **Injection**: `phantom skill recall` shows the memory being assembled into
  the system block the agent prepends before each run — i.e. the agent will
  actually *use* what you stored. With the
  `experimental-hermes-{curator,memory,tools}` features built in, finished
  runs also feed the judge → extract → store learning loop, so the node gets
  better at your recurring tasks over time.

If step 2 or 3 comes back empty: confirm `PHANTOM_OWNED_MEMORY` is not set to
`0`/`false`/`off`/`no`, confirm the note landed (`phantom recall --json`), and
re-run `phantom doctor` to check the binary/config rows are green.

---

## Quick reference

| Target | Build from origin/main | Serve | Memory check |
|---|---|---|---|
| Windows | `scripts\build-windows.ps1 -Deploy` (out-of-tree target) | logon Scheduled Task `phantom serve` | `phantom note` + `phantom recall` (§6) |
| macOS | `cargo install --path . --features experimental-hermes-curator,experimental-hermes-memory,experimental-hermes-tools` | `phantom service install` (launchd) | §6 |
| Linux | same `cargo install` line as macOS | `phantom service install` (systemd) | §6 |
| Android | `npx tauri android build --target aarch64` → `adb install` | thin client → coordinator | verify on coordinator (§6) |
| iOS | `TESTFLIGHT=1 APPLE_TEAM_ID=F7683B69U7 bash scripts/package-ios.sh` → TestFlight | thin client → coordinator | verify on coordinator (§6) |

All mobile/remote clients require Tailscale on the **same tailnet** as a
running `phantom serve` coordinator, reached at `<coordinator-tailscale-ip>`.