# Other-Session Onboarding & Verification

**For**: any Claude / Cursor / human session opening on Z13 (Windows),
Oracle Cloud (Linux), iPhone/iPad iOS-build host (Mac M1 worktree),
Mipad/ROG Phone Tauri-APK build host, or any future Linux box.
**Authority**: Mac M1 (`Session: mac-m1`) is the reference impl per
SPEC-FREEZE-V1 §1. Anything here that contradicts SPEC-FREEZE-V1
loses.
**Effective**: 2026-05-01 → 2026-05-15
**Companion docs (read in order at session start)**:
1. `docs/MULTI-DEVICE-COORDINATION.md` — the canonical protocol
2. `docs/SPEC-FREEZE-V1.md` — the frozen contract
3. `docs/PORTFOLIO-SPEC-FREEZE-V1.md` — the 5 satellite repos
4. `docs/MULTI-AGENT-QA.md` — the PR review pipeline

---

## 0. The 30-second version

```
1. git fetch && git pull origin platform/<your-os>
2. read the 4 companion docs above (top to bottom, no skipping)
3. write your "scope" commit announcing yourself
4. do your platform's task from §3
5. before push: run pre-push (§4)
6. after deploy: report back via §5
```

If you skip step 2 you will break something the other 4 sessions are
relying on. Spec is short on purpose — read it.

---

## 1. First-session onboarding (run once when you open the session)

### 1.1 Git clean-room

```bash
# Mac M1 (orchestrator):
cd ~/Documents/workspace/hailmary/phantom-mesh

# Z13 (Windows PowerShell):
cd C:\Users\m4932\repos\phantom-mesh

# Oracle Cloud A1 (Linux):
cd ~/repos/phantom-mesh

# iOS build worktree (Mac M1):
cd ~/Documents/workspace/hailmary/phantom-mesh-ios
```

```bash
# Always:
git fetch --all --tags
git status                                # must be clean
git checkout platform/<your-os>           # macos / windows / linux / ios / android
git rebase origin/platform/<your-os>      # incorporate any remote work
```

### 1.2 Read the 4 docs

Don't skim. The interlocking rules in MULTI-DEVICE-COORDINATION.md +
SPEC-FREEZE-V1.md only make sense together. Total ~15 minutes.

### 1.3 State your scope in commit #1 of this session

Per MULTI-DEVICE-COORDINATION.md §Onboarding, your first commit
announces who you are:

```bash
git commit --allow-empty -m "$(cat <<'EOF'
[<scope>] add session: <session-name> joining the mesh

Scope:    <what paths you own; copy from MULTI-DEVICE-COORDINATION.md §2 table>
Reads:    everything (read-only outside scope)
Coordinates: <paths in §2 "Coordinates" column for your row>

Today's intended work:
  - <bullet>
  - <bullet>

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
Session: <your-session-name>
EOF
)"

git push origin platform/<your-os>
```

This isn't ceremony — it lets the Mac M1 session see "session opened"
on the next pull and avoid clashing.

### 1.4 Run a baseline `phantom doctor`

```bash
# After installing phantom binary per §3 of this doc
phantom doctor
```

If your machine doesn't have phantom installed yet, that's fine — §3
covers install path per platform.

---

## 2. Daily verification (run at the START of every working day during the freeze)

```bash
# 1. Sync
git fetch --all --tags
git rebase origin/platform/<your-os>

# 2. If any Rust file in core/ or app/src-tauri/ changed, run tests
( cd core && cargo test --lib --release 2>&1 | tail -3 )

# 3. Verify your peer's core_sha matches Mac's reference
curl -sm5 http://<mac-tailscale-ip>:7878/rpc/ping | jq -r .core_sha
curl -sm5 http://<your-tailscale-ip>:7878/rpc/ping | jq -r .core_sha
# expect: SAME string. If different, you're on a stale binary.

# 4. Check who else is online
phantom peer list                         # ≤5s now (per Bug #23 fix)

# 5. Check today's spec drift
git diff main..origin/platform/macos -- docs/SPEC-FREEZE-V1.md
# review any frozen-surface change before you do work that depends on it
```

If `core_sha` mismatches: redeploy your binary per §3.

If `git diff` shows a wire/RPC change: STOP and check whether your
in-progress work is still compatible. Ping Mac session via commit
comment if unsure.

---

## 3. Per-platform task list — what your session ships

### 3.1 Z13 session (Windows + Android-APK build host)

**Owns**:
- `scripts/build-windows.ps1` — native build helper (handles AV-lock workaround + deploy)
- `scripts/install-phantom-windows.ps1` — fresh-machine install one-liner
- `app/src-tauri/gen/android/` — Tauri's generated Android scaffold (committed; do not regenerate casually)
- `app/src-tauri/tauri.conf.json` `bundle.windows` block — MSI/NSIS settings (wix lang, nsis perUser install mode, allowDowngrades for self-update rollback)
- `.github/workflows/release-windows.yml` — native windows-latest build + Tauri MSI/NSIS via `tauri-action`, triggered by tag push or manual dispatch. Uploads `phantom-x86_64-pc-windows.exe` + sha256 + MSI + NSIS-setup.exe to the release.
- (Deferred — not yet authored): `app/src-tauri/win/` for any custom WiX templates beyond what `bundle.windows.wix` defaults give us. Add only if the MSI needs custom upgrade-code logic, registry overrides, or per-machine install paths.

**Reads**: everything else.

**Coordinates with Mac M1 first**: any `core/` change, any change
to `core/src/serve.rs::rpc_*`, any wire-format change.

**Windows-specific gotchas you must know up front** (all surfaced during the 2026-05-01 Z13 hardening sweep):

- **Defender real-time scan locks newly-emitted `build-script-build.exe`** inside `.worktrees/<topic>/core/target/`. Surfaces as `存取被拒 (os error 5)` mid-build. Always set `CARGO_TARGET_DIR` outside the worktree (e.g. `D:\tmp\phantom-windows-target`). `scripts/build-windows.ps1` sets it for you.
- **`phantom service install` requires PowerShell, not cmd.** It uses `Register-ScheduledTask -AtLogOn` because `schtasks /SC ONLOGON` is denied on managed/Enterprise Windows. The Scheduled Task name is **`PhantomServe`** (matches this doc + the binary's `WINDOWS_TASK_NAME` constant).
- **Configured port** — phantom serve reads `[core] port` from `~/.phantom-mesh/agents.toml`. Doctor / service status / firewall rule / verify hint all honor it. Don't assume `:7878`.
- **PowerShell vs git-bash PATH** — git-bash auto-adds `~/bin`, PowerShell doesn't. Deploy to **`~/.local/bin/phantom.exe`** (already on Windows User PATH) so both shells see the same binary. The Scheduled Task points at `~/.phantom-mesh/bin/phantom.exe` (managed by `phantom service install` itself).
- **Provider key precedence** — `agents.toml` should use `api_key_env = "OPENROUTER_API_KEY"` (or whichever provider). Set the env var via `[Environment]::SetEnvironmentVariable('OPENROUTER_API_KEY', '<key>', 'User')` (User registry → persists across PowerShell sessions). Never bake literal keys into agents.toml.
- **Pre-existing admin-owned `PhantomServe` task** — if you inherited a Z13 box that already has a `PhantomServe` task and `phantom service install` returns `PermissionDenied (HRESULT 0x80070005)`, an earlier admin-elevated install registered the task in the system tree. User-level `Register-ScheduledTask -Force` can't overwrite it. One-time fix from an **elevated PowerShell**:
  ```powershell
  Unregister-ScheduledTask -TaskName 'PhantomServe' -Confirm:$false
  ```
  Then re-run `phantom service install` from your normal (non-admin) shell.

**Task list for 5/9 demo**:

```powershell
# A. Windows binary (native cargo via the helper)
git checkout v0.4.x-demo                  # whatever tag Mac M1 publishes
.\scripts\build-windows.ps1 -Deploy       # build + cp to ~/.phantom-mesh/bin/ + ~/.local/bin/
$bin = "$env:USERPROFILE\.phantom-mesh\bin\phantom.exe"
& $bin --version                          # confirm new build live

# Deploy to 3 Windows machines:
# Z13 (this box — `phantom service install` is idempotent: PowerShell
# Register-ScheduledTask -Force overwrites any prior PhantomServe task,
# and it picks up the configured port for the Defender firewall rule too).
& $bin service install

# Acer (gur943mk):
scp $bin user@100.106.176.125:.phantom-mesh/bin/phantom.exe
ssh user@100.106.176.125 'taskkill /F /IM phantom.exe; ".phantom-mesh\bin\phantom.exe" service install'

# AYANEO 2:
scp $bin m4932@100.107.205.98:.phantom-mesh/bin/phantom.exe
ssh m4932@100.107.205.98 '".phantom-mesh\bin\phantom.exe" service install'

# Verify all three respond (use whatever port their agents.toml is set to)
foreach ($peer in @("100.87.70.65:7878", "100.106.176.125:7878", "100.107.205.98:7878")) {
  Invoke-WebRequest -Uri "http://$peer/rpc/ping" -TimeoutSec 5 | Select-Object -ExpandProperty Content
}

# B.1 Android Tauri APK — UI shell (per Q8.1 user decision: Z13 is the build host)
# Prerequisite (one-time): Android SDK + NDK r25+ + Tauri CLI
cd app
pnpm tauri android build --apk
$apk = "src-tauri\gen\android\app\build\outputs\apk\universal\release\app-universal-release.apk"
Test-Path $apk
# distribute to Mipad + ROG Phone via USB or scp

# B.2 Android CLI binary — Termux cluster worker (per PR #4)
# Prerequisite (one-time):
#   rustup target add aarch64-linux-android
#   cargo install cargo-ndk --locked
#   Android NDK 27+ via Android Studio SDK Manager  (script auto-finds it under
#   $env:LOCALAPPDATA\Android\Sdk\ndk\<version> on Windows)
bash scripts/package-android.sh
$tarball = "dist\phantom-android-arm64.tar.gz"
Test-Path $tarball
# Or pull the CI-published artifact instead of building locally:
#   gh release download <tag> -p "phantom-aarch64-linux-android"
#   (produced by .github/workflows/release-daemon.yml's build-android job)
```

**Device-side install + verify** (run on the phone, not Z13):
```bash
# In Termux on Mipad / ROG (one-time):
COORD=http://100.87.93.58:7878  GROQ_KEY=gsk_...  \
  curl -fsSL "$COORD/scripts/termux-setup.sh" | sh
# pulls phantom-aarch64-linux-android from $COORD/dist/, writes
# ~/.phantom-mesh/agents.toml with cluster_secret pre-wired,
# starts `phantom serve --port 7879` in background.

# Cluster-dispatch verify (run from any peer that knows the cluster_secret):
SECRET="phantom-cluster-2026"
BODY='{"agent":"master","prompt":"reply: OK from <node>"}'
AUTH=$(printf '%s' "$BODY" | openssl dgst -sha256 -hmac "$SECRET" -hex | awk '{print $2}')
RESP=$(curl -sS -X POST http://<phone-ts-ip>:7879/rpc/task/assign \
  -H "X-Cluster-Auth: $AUTH" -H "Content-Type: application/json" -d "$BODY")
JOB=$(echo "$RESP" | sed -n 's/.*"job_id":"\([^"]*\)".*/\1/p')
sleep 6
curl -sS "http://<phone-ts-ip>:7879/rpc/task/status/$JOB"
# expect: {"status":"done","output":"...","error":null}
```

**Pre-push** (cargo test + clippy on the Windows toolchain):
```powershell
.\scripts\build-windows.ps1 -Test         # build + cargo test --lib --release (285 tests pass on this toolchain)
$env:CARGO_TARGET_DIR = 'D:\tmp\phantom-windows-target'
cd core
cargo clippy --release -- -W clippy::all  # ~15 cross-platform style warnings remain — non-blocking
```

**Commit prefix to use**: `[win]` for Windows-specific paths,
`[android-tauri]` for Android Tauri APK paths,
`[android-cli]` for Android CLI binary / Termux paths (per PR #4).

### 3.2 Oracle Cloud A1 session (Linux ARM, full worker + always-on hub)

**Owns**: `scripts/build-linux.sh`, `templates/phantom-mesh.service.tmpl`,
`dist/phantom-aarch64-unknown-linux-gnu`,
`.github/workflows/release-linux.yml` (TBD).

**Reads**: everything else.

**Coordinates with Mac M1 first**: any `core/` change.

**Task list for 5/9 demo**:

```bash
# Initial setup (one-time):
sudo apt update && sudo apt install -y build-essential pkg-config libssl-dev curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
git clone https://github.com/markl-a/phantom-mesh-private ~/repos/phantom-mesh

# Build (native ARM cargo — option C from SPEC-FREEZE-V1 §6.1):
cd ~/repos/phantom-mesh/core
git checkout v0.4.x-demo
cargo build --release --bin phantom
cp target/release/phantom ~/.local/bin/phantom

# Install as systemd user unit:
mkdir -p ~/.config/systemd/user
cp ~/repos/phantom-mesh/templates/phantom-mesh.service.tmpl \
   ~/.config/systemd/user/phantom-mesh.service
systemctl --user daemon-reload
systemctl --user enable --now phantom-mesh
systemctl --user status phantom-mesh

# Configure cluster_secret (out-of-band — same secret as Mac):
mkdir -p ~/.phantom-mesh
cat > ~/.phantom-mesh/local.toml <<EOF
[cluster]
node_name      = "oracle-a1"
cluster_secret = "<paste from 1Password>"
EOF
chmod 0600 ~/.phantom-mesh/local.toml

# Verify
curl -sm5 http://localhost:7878/rpc/ping | jq -r '.core_sha,.wire_version,.phantom_version'
# expect: same core_sha as Mac's, wire_version=1, version=0.4.0

# Open firewall + Tailscale registration
sudo ufw allow 7878/tcp                    # if needed
sudo tailscale up --advertise-tags=tag:phantom-mesh
```

**Pre-push**:
```bash
cd ~/repos/phantom-mesh/core
cargo test --lib --release 2>&1 | tail -3   # 289+ tests must pass
cargo build --release --bin phantom         # must succeed
```

**Commit prefix to use**: `[linux]`.

### 3.3 iOS session (runs on Mac M1, builds for iPhone + iPad)

**Owns**:
- `scripts/package-ios.sh` — sim + device build pipeline (drives
  `npx tauri ios build`, not raw xcodebuild — see commit `bb542fc`).
- `app/src/components/mobile/` — `MobileShell`, `MobileConversation`,
  `MobileClusterSettings`, `MobileOnboarding`, etc.
- `app/src/lib/clusterDispatch.ts` — HMAC-SHA256 wire to coordinator's
  `/rpc/task/assign` (added `1006b6d`; matches `core/src/serve.rs`
  + `core/src/mesh.rs::make_auth_token_bytes`).
- `app/src-tauri/gen/apple/` is **gitignored** — regenerated by
  `tauri ios init` on every fresh build. Do not commit.

**Reads**: everything else.

**Coordinates with Mac M1**: same machine, so this is really an
iOS-specific worktree + branch (`platform/ios`) on the Mac.

**Task list**:

```bash
cd ~/Documents/workspace/hailmary/phantom-mesh-ios
git fetch origin && git checkout platform/ios && git pull --ff-only

# APPLE_TEAM_ID is hardcoded in app/src-tauri/tauri.conf.json
# (bundle.iOS.developmentTeam = "F7683B69U7"). No env var needed.
# package-ios.sh reads it from tauri.conf.json automatically.

# Sim build (debug, --no-sign, ~2 min incremental, ~8 min cold):
./scripts/package-ios.sh --sim
#   → dist/phantom-ios-sim.app (~97 MB, includes libapp.a as resource)

# Device build (release, signed Apple Development cert, IPA):
./scripts/package-ios.sh
#   → dist/phantom-ios.ipa  (~31 MB, signed by team F7683B69U7)

# Sim install + launch:
SIM_ID=$(xcrun simctl list devices available | grep "iPhone 17 Pro" \
         | grep -oE "[A-F0-9-]{36}" | tail -1)
xcrun simctl install  "$SIM_ID" dist/phantom-ios-sim.app
xcrun simctl launch   "$SIM_ID" ai.phantommesh.app

# Device install over wifi (devicectl tunnel, no USB needed once paired):
#   iPhone 13 mini: 00008110-001134A026F2801E   (HITRON-9527)
#   iPad Pro 12.9": 00008027-0018296E2E22002E   (MarL 的 iPad)
xcrun devicectl device install app --device <UDID> dist/phantom-ios.ipa
xcrun devicectl device process launch --device <UDID> ai.phantommesh.app
# Devices must be unlocked at install time so the developer disk image
# can mount; otherwise it fails with `kAMDMobileImageMounterDeviceLocked`.
```

**First-launch flow on iOS** (post-install, what the user sees):
1. App boots straight into MobileShell (3 bottom tabs: 對話 / 節點 / 設定).
2. On the chat tab, typing + send shows a red banner
   `尚未設定 cluster — 點此到「設定 → Cluster 派送」`.
3. Tap → routes to `/settings/cluster` (deep-link wired in commit
   `e1f4b14`). Fill **Coordinator URL** + **Cluster Secret** (must match
   `[cluster]` block of coordinator's `~/.phantom-mesh/agents.toml`),
   tap **測試 dispatch**.
4. On first ✓ test, cluster mode auto-enables (no extra toggle tap).
5. Back on chat tab → send messages, they go to coordinator's
   `/rpc/task/assign` and the response polls back via
   `/rpc/task/status/:id`.

iOS local mode is intentionally non-functional (no shell, no
sidecars, no agents.toml unless imported via MobileOnboarding); the
banner CTA in step 2 is the sole supported recovery path.

**Build-pipeline gotchas already worked around in `package-ios.sh`**:
- Tauri 2.10 npm pkg vs Rust crate are version-mismatched; build flags
  `--ignore-version-mismatches` (revisit when 2.11+ Rust crate
  stabilises — naive `cargo update -p tauri` to 2.11 broke sim archive).
- `xcodegen path: Externals` recursive resource copy collides on
  `libapp.a` between sim debug + device release runs; script wipes the
  opposite-config dir + DerivedData before each build.
- `tauri::generate_context!()` bakes `frontendDist` via `include_dir!`
  at compile time; `app/src-tauri/build.rs` declares
  `cargo:rerun-if-changed=../dist/{index.html,assets}` so frontend-only
  edits trigger a recompile + re-bake.
- `tauri ios xcode-script` panics on a missing `<bundle-id>-server-addr`
  unless invoked through `tauri ios build` — never use raw `xcodebuild`
  on this project.

**Pre-push checklist**:
```bash
cd ~/Documents/workspace/hailmary/phantom-mesh-ios
cargo check --manifest-path app/src-tauri/Cargo.toml \
            --target aarch64-apple-ios-sim --lib    # fast (~30s)
./scripts/package-ios.sh --sim                       # full sim build
# If you touched app/src/components/mobile/* or routing, also re-test
# at least one physical device (sim doesn't catch every wkwebview gap).
```

**Free-cert renewal**: signed IPAs from a free Apple Developer cert
expire 7 days after build. A `/schedule` routine
(`trig_01VySWaHMoTodsWcqZvRQtKA`) opens a reminder issue every Thursday
09:00 Asia/Taipei to rerun the device build before the window closes.

**Commit prefix to use**: `[ios]`. Cross-platform / shared-frontend
fixes (e.g. `app/src/lib/`, `.gitignore`) use `[shared]` and should
PR up to `phase1-r1-foundations` per Rule 1 of `MULTI-DEVICE-COORDINATION.md`.

### 3.4 Android Tauri APK installation (Mipad + ROG Phone — operator task, not a session)

This isn't a coding session — it's the user's manual install on each
device. Z13 builds the APK; user installs.

```
1. Z13 publishes phantom-mesh-android.apk to a shared location
   (e.g. Tailscale-served HTTP via `phantom serve` /dist/, or USB)

2. On Mipad / ROG Phone:
   - Settings → Apps → unknown-source-install allowed for the
     installer source
   - Open the APK file → install
   - Open the app once → grant foreground service notification
     permission → set Tailscale up → app should auto-discover
     Mac coordinator and join mesh

3. Verify from Mac:
   curl -sm5 http://<phone-tailscale-ip>:7878/rpc/ping
   # expect: wire_version=1, worker_caps=[file_in_container, memory,
   # web, subagent, llm_local]
```

---

## 4. Pre-push checklist (every session, before every push)

Per SPEC-FREEZE-V1 §7 and MULTI-DEVICE-COORDINATION.md §8:

```bash
# 1. cargo fmt --check (always)
cargo fmt --check

# 2. cargo clippy on the platform's actual target
cargo clippy --release --all-targets -- -D warnings
# NOTE: pre-existing 60 clippy lints exist as of 5/1; mark known
# pre-existing failures in your PR comment.

# 3. cargo test --lib (always)
cd core && cargo test --lib --release 2>&1 | tail -3

# 4. cargo build --release --target <triple> (per platform)
cargo build --release --bin phantom --target <your-triple>

# 5. shellcheck on touched scripts
shellcheck -x scripts/build-*.sh scripts/install-*.sh

# 6. actionlint on touched workflows
actionlint .github/workflows/*.yml

# 7. core_sha drift check
PEER_SHA=$(curl -sm5 http://<your-tailscale-ip>:7878/rpc/ping | jq -r .core_sha)
LOCAL_SHA=$(git rev-parse --short HEAD)
echo "peer says: $PEER_SHA, local: $LOCAL_SHA — must match if you redeployed"
```

If any of 1–4 fails: don't push. Fix and re-run.

---

## 5. Reporting back to Mac M1 + other sessions

After your work merges (or you push to your `platform/<os>` branch):

### 5.1 Commit body conventions

```
[<scope>] type(area): one-liner

What changed:
  - …
  - …

Cross-platform impact:
  - core_sha now <hash>
  - tests: <count> pass / <count> fail
  - any spec drift? <yes/no>

Reviewed-by: <agent1> + <agent2>     (per MULTI-AGENT-QA.md §3)
Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
Session: <your-session-name>
```

### 5.2 If you change anything in §2 of SPEC-FREEZE-V1 — STOP and ping

```bash
# 1. Open a draft PR to phase1-r1-foundations:
gh pr create --draft --title "[<scope>] proposal: change to <frozen-surface>" \
   --body "$(cat <<'EOF'
This PR changes a §2 frozen surface in SPEC-FREEZE-V1.

Frozen surface affected: <which §2.X>
Reason: <regression test / blocker / what>

Before merging:
  - waiting for Mac M1 review
  - tagging gemini for spec drift check
  - this is the bug-fix exception path per SPEC-FREEZE-V1 §7

EOF
)"

# 2. Wait for Mac M1 to comment. Don't merge.
```

### 5.3 Daily sync commit (optional but recommended)

If you've been heads-down, leave a one-line empty commit at end of day:

```bash
git commit --allow-empty -m "[<scope>] sync: end of <date> — <one-line status>"
git push origin platform/<your-os>
```

So Mac M1's morning pull surfaces "Z13 session is alive, made
progress on X today".

---

## 6. If you're stuck

Three escape hatches in increasing order of disruption:

1. **Open an EVOLVE-GOAL** for the gap: append a `- [ ]` line to
   `EVOLVE-GOALS.md` describing the blocker. Mac M1 picks it up next
   ping.

2. **Comment on a PR** in the relevant scope: tag `@<owner>` (the
   session that owns the affected paths per
   MULTI-DEVICE-COORDINATION.md §2). E.g. an `[android]` issue → tag
   the Z13 session.

3. **Ping Mac M1 via commit body**: open an empty commit with body
   `Blocked: <description>. Need: <decision> from mac-m1.` Mac M1's
   daily sync will see it.

**Don't improvise**. Particularly:
- Don't silently change wire / HMAC behaviour
- Don't add a new RPC endpoint
- Don't bump WIRE_VERSION

If it's hot enough that you'd improvise, that's exactly the case for
SPEC-FREEZE-V1 §7's bug-fix exception — but write the regression test
+ `Frozen-surface: yes` line in the commit body.

---

## 7. Daily check-in template (paste this into your session header)

```
Session:        <name>          (e.g. z13-windows, oracle-linux, ios-mac)
Branch:         platform/<os>
Date:           <YYYY-MM-DD>
Last sync:      git fetch + rebase origin/platform/<os>  ✓
core_sha:       <local>  ↔  <Mac M1's>           [match/drift]
Doctor green?   yes/no
Today's task:   <bullet from §3>
Reviewers:      <agents per MULTI-AGENT-QA §6 trigger rule>
```

---

## 8. Glossary

| Term | Meaning |
|---|---|
| Mac M1 | the orchestrator session on MacBook Air M1 |
| reference impl | what the rest of the mesh aligns to (Mac M1's behaviour) |
| frozen surface | anything in SPEC-FREEZE-V1 §2 |
| `[scope]` | commit prefix per MULTI-DEVICE-COORDINATION.md §1 (`[mac]`, `[win]`, `[linux]`, `[ios]`, `[android]`, `[core]`, `[shared]`) |
| `Reviewed-by:` | commit footer per MULTI-AGENT-QA.md §3 step 7 |
| `Frozen-surface: yes` | bug-fix exception flag per SPEC-FREEZE-V1 §7 |
| `Session: <name>` | commit footer per MULTI-DEVICE-COORDINATION.md Rule 9 |
| dress rehearsal | 5/8 full-mesh demo run with real hardware |
| unfreeze | 5/15 — when SPEC-FREEZE-V1 retires, see §13 of that doc |

---

## 9. Tomorrow morning's first 5 minutes (any session)

```bash
git fetch --all --tags
git pull --rebase origin platform/<your-os>
phantom doctor                                     # baseline
curl -sm5 http://<mac>:7878/rpc/ping | jq -r .core_sha    # match local?
ls docs/SPEC-FREEZE-V1.md                          # exists, recent date?
git log --since="yesterday" --grep "[$scope]"     # any drift in scope?
```

If all 5 lines come up clean, you're cleared to start work on §3.

If any of them looks wrong: stop, read what changed, ping the relevant
session via §6's escape hatches.
