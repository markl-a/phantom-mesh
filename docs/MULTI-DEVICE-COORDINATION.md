# Multi-Device Coordination Protocol

**This document is the canonical working agreement for any Claude / Cursor /
human session that's developing phantom-mesh on more than one machine
in parallel.** Every session — Mac, Z13 Windows, Linux box, iOS, Android —
reads this first. New session opening on a fresh machine? **Start here.**

The goal: **let 5 platforms develop simultaneously without the codebase or
the deployed mesh fragmenting.** Phantom-mesh's design lets the binary
self-modify; that's only safe if the humans+agents working across machines
agree on a coordination protocol. This document IS that agreement.

---

## TL;DR — six rules

1. **One canonical repo, one main branch.** Sessions work on
   `platform/<os>` branches. Cross-platform / `core/` edits go through a
   shared integration branch and PR to main.
2. **Each session has a scope.** It owns one branch, owns specific paths,
   reads everything else, never silently edits another session's
   territory.
3. **Binary distribution is via GitHub release tags, not local cargo
   build.** Tag triggers cross-platform CI matrix; every machine pulls
   its release artifact. Local builds are for dev only.
4. **Configuration is split: base (committed) + local (per-machine).**
   `agents.base.toml` in repo. `~/.phantom-mesh/local.toml` holds node
   name + cluster secret, never committed.
5. **Wire-protocol versioning is explicit.** Every RPC response carries
   `wire_version`; stale peers reject incompatible neighbours with a
   clear error rather than silently failing handshake.
6. **Verification is daily, automated, and one command.**
   `phantom doctor --mesh` is the green/red light for "are all 5 talking
   to each other right now."

The rest of this document spells out each rule, gives the conflict-
resolution process, and lists what's not yet implemented.

---

## Rule 1 — Branch & merge protocol

```
main  ────────────────────────────────────────────────────▶ (protected)
   ▲   ▲                ▲                  ▲           ▲
   │   │                │                  │           │
   │   └─ phase1-r1-foundations ── shared integration branch (current
   │                                "what's everyone working on" trunk)
   │
   ├─ platform/macos     ── Mac session work: core/, app/src-tauri/{macos,ios}/
   ├─ platform/windows   ── Z13 session work: scripts/build-windows.sh, app/src-tauri/win/
   │                        ALIAS: feat/windows is currently used by Z13 as the
   │                        live working branch (commit f670ab1+). Treated as
   │                        equivalent to platform/windows during the v0.1.0
   │                        freeze. Renames to platform/windows on 5/15 unfreeze
   │                        per SPEC-FREEZE-V1 §13 cleanup.
   ├─ platform/linux     ── Linux box work: scripts/build-linux.sh, systemd templates
   │                        Currently no live branch — Oracle Cloud A1 session
   │                        will create it per SESSION-ONBOARDING.md §3.2.
   ├─ platform/ios       ── iOS-specific Tauri work (lives on Mac)
   └─ platform/android   ── Android Tauri work + phantom-mobile (lives on Z13)
                            ALIAS: feat/android is currently Z13's live
                            working branch (commit 963c3fe). Same equivalence
                            + rename schedule as feat/windows.
```

### Daily flow

1. **Session start:** `git fetch && git rebase origin/main` on this
   session's `platform/*` branch. If conflicts: see Rule 7.
2. **During session:** commit freely to your `platform/*` branch.
   Push every 30-60 min so other sessions see your activity.
3. **Session end:** if you touched anything outside your owned paths,
   open a PR from `platform/<os>` → `phase1-r1-foundations`. Tag with
   `multi-session` so other sessions see it.
4. **Weekly:** `phase1-r1-foundations` → `main` PR after green CI on
   all 5 platform builds. **Only main is what release tags come from.**

### Commit message scope tag

Every commit message starts with a scope prefix:
```
[mac]    fix(tui): cursor position with combining diacritics
[win]    feat(scheduled-task): bootout race fix in restart logic
[core]   fix(mesh): wire_version field on /rpc/ping
[shared] docs: update CO-EVOLUTION roadmap for Phase 4 land
```

`[core]` and `[shared]` commits draw extra attention at PR review time
because they affect every session.

---

## Rule 2 — Scope discipline

Each session is given a 3-tier permission.

| Session | Owns (free edit) | Reads (context only) | Coordinates (announce first) |
|---|---|---|---|
| **Mac M1 (MacBook Air)** | `core/` (default), `app/src-tauri/`, `templates/`, `docs/`, `.github/workflows/`, `scripts/build-mac.sh`, `app/src-tauri/ios/` | everything | `core/src/mesh.rs`, `core/src/serve.rs::rpc_*`, `core/src/keys.rs` (others depend) |
| **Z13 Win** | `scripts/build-windows.sh`, `.github/workflows/release-windows.yml`, `app/src-tauri/android/`, anything under `app/src-tauri/gen/android/` | everything | `core/` is read-only by default |
| **Linux** | `scripts/build-linux.sh`, `templates/phantom-mesh.service.tmpl` (additions), `dist/linux*` | everything | `core/` is read-only by default |
| **iOS** | `app/src-tauri/ios/`, signing config files | everything | nothing — runs on Mac, isolates to iOS-only paths |
| **Android** | `phantom-mobile/` repo, `app/src-tauri/android/` (when Z13 also runs Android session) | everything | nothing |

**"Coordinates"** means: before editing one of those paths, leave a note
in `EVOLVE-GOALS.md` or open a draft PR. Don't silently push `mesh.rs` —
it breaks every other session's `core-sha`.

### What "owns" really means

If session A owns a path, session B sees changes there appear in main but
**must not edit them**. If B believes a change is needed, B opens an
issue / EVOLVE-GOAL describing the desired change, A acts on it. This
prevents the "two sessions both fix the Win path bug differently" race.

---

## Rule 3 — Single binary truth: GitHub release tags

```
[trigger]                       [process]                    [distribution]
git tag v0.1.x ────────▶  GitHub Actions matrix    ────▶  release artifacts:
git push --tags             ├─ macos-arm64   build      phantom-macos-arm64.tar.gz
                            ├─ macos-x86_64  build      phantom-macos-x86_64.tar.gz
                            ├─ windows-x64   build      phantom-windows-x64.zip
                            ├─ linux-x64     build      phantom-linux-x64.tar.gz
                            ├─ linux-arm64   build      phantom-linux-arm64.tar.gz
                            └─ codesign each artefact   each signed by maintainer key
                                                ▼
                                  every machine: `phantom upgrade`
                                  → curl latest tag's matching artefact
                                  → verify signature
                                  → atomic swap (bootout → swap → bootstrap)
                                  → restart healthcheck
```

**No local `cargo build && cp` for production deploys.** That was the
all-day pattern that introduced the codesign SIGKILL bug (see commit
`85c8377`). Local cargo build stays — but only for development /
testing the change you're about to PR. **The mesh runs binaries from
release tags.**

**Tag cadence:** at minimum once per coordinated multi-session sprint.
A `v0.1.x-multidevice-N` series is fine for the rapid pre-launch phase;
proper semver after 5/15.

---

## Rule 4 — Configuration as code, secrets out-of-band

```
[committed]                           [per-machine, .gitignore]
agents.base.toml                      ~/.phantom-mesh/local.toml
├── [providers.*]                     ├── [cluster]
│   default_model, type, url          │   node_name = "mac-coordinator"
├── [agent.*]                         │   cluster_secret = "..."         ← from 1Password
│   provider, tools                   ├── [overrides.providers.opencode]
└── [cluster]                         │   api_key = "..."                ← from shell env
    peers = [list of all 5]               (or rely on api_key_env which we propagate
                                            via service install — see commit dfadc9d)
```

`AgentsConfig::load()` reads `agents.base.toml`, then deep-merges
`~/.phantom-mesh/local.toml` on top. Any field present in local
overrides base.

**Cluster secret distribution.** Generated once on Mac M1 (MacBook Air):
```
$ phantom keys generate-cluster-secret
   Wrote ~/.phantom-mesh/local.toml with cluster_secret=<16 random bytes>
   Now: copy that line to the local.toml on every other machine.
```

Out-of-band: 1Password shared item, encrypted gist, signed message via
ssh. The secret never appears in commits.

**Per-machine api_key**: rely on `phantom service install` to copy
shell env → plist EnvironmentVariables (commit `dfadc9d`). The keys
never live in any committed file.

---

## Rule 5 — Wire-protocol versioning

The schema for `EvolveCheckpoint`, mesh handoff payload, RPC bodies, etc.
will change. We need old peers to refuse new payloads with a clear
error rather than crash on a `serde::de::Error`.

### What we add

Every RPC response includes `wire_version: u32`:
```json
{ "ok": true, "wire_version": 3, "data": {...} }
```

`/rpc/ping` becomes the canonical compatibility check:
```json
GET /rpc/ping
→ { "wire_version": 3, "phantom_version": "0.1.4", "core_sha": "7a3f2b1" }
```

A peer with `wire_version` lower than this binary's: **degraded warning**,
but RPCs still flow (best-effort backward compat). Higher: **refuse**,
explicit error: "peer is wire v4, this binary is v3, run `phantom upgrade`".

### When to bump

- Add a field to existing schema: **no bump** (forward-compatible)
- Remove a field: **bump**
- Change a field's type or semantics: **bump**
- Add a new RPC endpoint: **no bump** (callers expect 404 for unknown)

Single integer kept in `core/src/lib.rs::WIRE_VERSION`.

---

## Rule 6 — Daily verification: `phantom doctor --mesh`

This is the green/red light. Open every session, every workday, before
any other work:

```
$ phantom doctor --mesh

◆ phantom 0.1.4 / wire 3 / core-sha 7a3f2b1
        ↑          ↑           ↑
        │          │           └── content hash; if differs from upstream tag,
        │          │                you've got a local fork
        │          └── this binary's wire version
        └── upstream tag

Peers (configured):
  ✓ mac-coordinator    100.87.93.58:7878    wire 3   ↔  same
  ✓ z13-windows        100.87.70.65:7879    wire 3   ↔  same
  ✗ linux-arm          100.106.176.125:7878  unreachable (no response 5s)
  ⚠ ios-iphone         100.108.x.x:7878     wire 2   ⚠ stale, run phantom upgrade
  ○ android-pixel      100.103.x.x:7878     wire 3   ↔  same  (not currently configured as peer)

Cross-checks:
  ✓ all peers' agents.base.toml SHA matches mine
  ✗ z13-windows local.toml.cluster_secret SHA differs — HMAC will fail!
  ✓ EvolveCheckpoint schema_version matches across all peers

Summary: 3/5 peers fully aligned. Issues: linux-arm unreachable, z13 secret drift.
```

Three exit codes:
- `0` — all peers green
- `1` — degraded (one or more peers warn)
- `2` — broken (one or more peers can't HMAC or schema-mismatch)

`phantom doctor --mesh --fix` for the auto-fixable cases (rotate
secret, prompt for `phantom upgrade`, etc).

---

## Rule 7 — Conflict resolution

### Same file, different sessions

1. Whoever pulls / rebases first wins. Second session hits conflict.
2. Try `git mergetool`. If trivial, resolve and force-push to your
   `platform/*` branch.
3. If conflict involves business logic (not whitespace / format), **stop**.
   Open a draft PR with both diffs side-by-side. Ping the other session
   in commit message body: `Resolve in coordination with @session-zwin`.
4. Use `git rerere` enabled by default — repeating mechanical conflicts
   auto-resolve after first.

### Concurrent edits to `core/`

Strict rule: **no two sessions edit `core/` at the same time without
explicit hand-off**. Use a "lock file" pattern:

```
~/.phantom-mesh/core-lock.json — committed, in repo root as `.phantom-core-lock.json`
{
  "owner_session": "mac-m3",
  "acquired_at_ms": 1777580000000,
  "intent": "fixing TUI scroll calc",
  "expires_at_ms": 1777583600000
}
```

A session takes the lock by editing this file in its first commit, releases
by deleting it in the last. Lock auto-expires after 1 hour. Sessions
poll-on-fetch: if the lock file exists and isn't yours, work on `app/`
or platform-specific paths instead.

**Yes this is informal.** It's good enough for 5 humans+agents who can
talk to each other; we don't need a real lock service. If it scales
beyond that, replace with GitHub branch protection + required reviewer.

---

## Rule 8 — What sessions MUST run before pushing

Pre-push checklist (each session enforces locally):

| Check | When | Command |
|---|---|---|
| `cargo fmt --check` | always | already in pre-commit |
| `cargo clippy -D warnings` | always (per platform) | makes platform-specific bugs visible |
| `cargo test --lib` | always | catches breakage in shared code |
| `cargo build --release --target <platform>` | platform-specific | proves the platform still compiles |
| `phantom doctor --mesh` | before merging to main | proves runtime mesh still works |

Failed checks → don't push. Open a draft PR documenting the failure
and ping for help.

---

## Rule 9 — Audit trail

Every commit is reviewable later by `git log` alone. The new things
this protocol adds:

- **Scope prefix** (`[mac]`, `[win]`, `[core]`, `[shared]`) — searchable
- **Co-Authored-By: Claude Opus 4.7 (1M context)** trailer — already
  established convention (memory `feedback_commit_attribution.md`)
- **`[Session: <name>]`** trailer when push comes from a non-default
  session, e.g.:
  ```
  [win] feat(scheduled-task): bootout race fix
  
  Add 300ms sleep between bootout and binary copy so launchd
  releases the binary mapping before we overwrite. Reproduces on
  Win10/Win11 alike.
  
  Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
  Session: z13-windows
  ```

`git log --grep '\[Session: z13-windows\]'` shows everything that
session ever did.

---

## Onboarding a new session

When you (Claude / human / etc) open a session on a new machine:

1. **Read this file end-to-end.** No skipping.
2. **Read `EVOLVE-GOALS.md`** — see what's in flight, what's blocked.
3. **`git pull origin main && git checkout -b platform/<your-os>`** if
   one doesn't exist; otherwise `git checkout platform/<your-os>` and
   rebase.
4. **`phantom doctor --mesh`** — see who else is up.
5. **State your scope in your first commit** so others see you appeared:
   ```
   [scope] add session: linux-arm joining the mesh

   Scope:    Linux platform binary, systemd template, /etc/phantom-mesh
   Reads:    everything
   Coordinates: nothing currently
   
   Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
   Session: linux-arm
   ```

---

## What's NOT yet implemented (gaps + mitigation)

| Gap | Impact today | Mitigation until shipped |
|---|---|---|
| ~~`wire_version` on RPC~~ | ✅ Shipped 2026-05-01 — `WIRE_VERSION = 1` in `core/src/lib.rs`, every peer-facing RPC carries the field, mismatches return HTTP 400 with `phantom upgrade` hint | — |
| `phantom doctor --mesh` | No one-command health check (now has `wire_version`/`core_sha` on `/rpc/ping` to consume) | Manual `curl /healthz` to each peer |
| `phantom upgrade` | Each machine must `cargo build` itself | Use the redeploy bash from earlier sessions; codesign step is critical (see commit 85c8377) |
| GitHub Actions release matrix | No central binary truth | Builds happen per-machine; verify md5 manually |
| `agents.toml` split | Drift risk on cluster_secret | Manual sync; check via `sha256sum agents.toml` across boxes |
| `.phantom-core-lock.json` | Multi-session core/ edit unprotected | Verbal coordination via commit messages |

These are tracked as goals in `EVOLVE-GOALS.md` (the multi-device
coordination block). Implementing them is the natural extension of
this protocol from "agreement we read" to "agreement enforced by code."

Phase target: ALL 6 gaps closed by 5/8 (one day before interview).
That's tight but possible if the platform sessions don't compete with
the core sessions too much.

---

## Quick reference card

Pin this on every session:

```
1. git rebase origin/main          ← session start
2. work in platform/<your-os>       ← never main directly
3. commit with [scope] prefix       ← every time
4. phantom doctor --mesh            ← before merge
5. tag releases on main             ← never on platform/*
6. local.toml stays out of git      ← always
7. cluster_secret never in commits  ← always
8. wire_version mismatch = stop     ← upgrade first
```
