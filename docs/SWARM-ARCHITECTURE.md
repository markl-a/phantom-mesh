# phantom-mesh — Cross-OS Swarm Architecture

> How a single prompt entered on any device (Linux / macOS / Windows /
> Android / iOS) is decomposed, dispatched, and executed across a
> heterogeneous fleet, and how far each OS can realistically be pushed
> as an agent host.
>
> Scope: design contract for the swarm layer above the existing single-node
> `phantom serve` daemon documented in [ARCHITECTURE.md](./ARCHITECTURE.md).
> Hub-and-spoke is the **shipping** topology for the 2026-05-20 launch;
> mesh-with-CRDT-state is a roadmap item, not a current design target.
>
> **Non-goal**: pretending iOS and Android are symmetric. They are not.
> A large part of this doc exists to make that asymmetry explicit and
> exploit each OS for what it actually does well.

---

## 1. Vision

A user holding any one of their devices types a prompt and gets the
right work done on the right machine, transparently:

- iPad on the couch → planner picks the home Mac for code edits and
  the always-on Z13 for the cluster heartbeat.
- Android in a coffee shop → planner runs a local 3B for "what does
  this error mean" but punts a 200-file refactor to the home cluster
  over Tailscale.
- Mac at the desk → coordinator. Dispatches; doesn't always execute.

The mental model is **one logical agent, many physical executors**.
The user does not pick which machine runs what; the swarm does, with
explicit override available.

---

## 2. Node Taxonomy

Four roles. A single device can hold more than one role.

| Role | Description | Typical Hosts |
|------|-------------|---------------|
| **Coordinator** | Owns the session log, runs the planner, dispatches subtasks. Exactly one per session (re-electable on failure). | Mac, Z13, any always-on Linux/Windows box |
| **Full Agent** | Can execute arbitrary subtasks: shell, git, browser, file edits, local LLM, cloud LLM. | Linux, macOS, Windows desktops, Termux-on-Android |
| **Lite Agent** | Can execute a restricted, declared subset of subtasks. No shell exec, no arbitrary subprocess. | iOS native app, browser-tab PWA, locked-down corporate laptop |
| **Thin Client** | UI surface only. Sends prompts, renders streamed output. Executes nothing. | iOS Safari / mobile.html, web `/projects`, any device on the tailnet without phantom installed |

The current `PeerInfo` / `PeerStatus` types in `core/src/mesh.rs` already
carry `capabilities: Vec<String>` and `worker_caps: Vec<String>`. The
swarm layer formalises what strings are allowed and extends the wire
type with a structured **Capability Manifest** (§4).

A device's role is **emergent** from its manifest, not declared as a
free-form tag. A node with no `shell` capability is automatically a
Lite Agent regardless of what its config says about itself.

---

## 3. Transport, Auth, Discovery

Already solved. Re-stated here so swarm design doesn't re-invent it.

- **Transport** — HTTP/1.1 + JSON over Tailscale's WireGuard tunnel.
  Coordinator IP is the tailnet 100.x address of the always-on hub.
  No NAT punching, no STUN, no bespoke P2P stack.
- **Auth** — shared HMAC secret per tailnet (`PHANTOM_HMAC_KEY`). Every
  RPC carries `X-Phantom-Sig: hmac-sha256(body)`. Coordinator rotates
  the secret on `phantom secret rotate`; peers reload on next ping.
- **Discovery** — MagicDNS (`mac.tailnet`, `rog.tailnet`). Peers boot
  with `coordinator_url = "http://mac:7878"` in `agents.toml` and call
  `/rpc/join` on start. The cluster-wide peer list propagates via
  `/rpc/ping` exchanges, so adding a new node only requires telling it
  about one existing peer.

The future commercial broker (see [COMMERCIAL-DESIGN.md](./COMMERCIAL-DESIGN.md))
exists to relay between users **without** a shared tailnet. OSS users
on a single tailnet do not need it.

---

## 4. Capability Manifest (proposed extension)

Today `PeerStatus.capabilities` is an unstructured `Vec<String>`. The
swarm planner needs structure. Proposed wire type, additive to
`PeerStatus`:

```jsonc
{
  "node_id": "rog-phone",
  "os": "android",                       // linux | macos | windows | android | ios
  "arch": "aarch64",
  "power": "battery",                    // battery | wall | always_on
  "role_hint": "full_agent",             // self-declared, planner verifies via tools

  "tools": {                             // declared tool capabilities
    "shell": true,
    "git": true,
    "browser": false,
    "subprocess": true,                  // can fork-exec arbitrary binaries
    "mobile_driver": "android",          // null | "android" | "ios"
    "gpu_inference": "vulkan",           // null | "metal" | "cuda" | "vulkan" | "ane"
    "file_edit_scope": "fs",             // "fs" = anywhere, "sandbox" = app-bundle only
    "long_running": true                 // can host >5min background tasks
  },

  "models": {
    "local": ["gemma-2b-q4_k", "phi3-mini-q4"],
    "remote_keys": ["anthropic", "openai", "groq"]
  },

  "limits": {
    "max_concurrent_tasks": 4,
    "max_task_seconds": 1800,
    "egress_bandwidth_kbps": 5000
  },

  "reachable_via": {
    "tailscale": "100.x.y.z:7878",
    "lan": "192.168.1.42:7878"
  }
}
```

The planner makes dispatch decisions by **set inclusion**: a subtask
declares `required_caps = ["shell", "git"]`, and the scheduler picks
the cheapest healthy node whose `tools` superset covers it. This is
already the shape of the existing `worker_caps` mechanism — we are
extending it from a flat capability list into a structured profile.

**Implementation**: add `manifest: Option<NodeManifest>` to
`PeerStatus`. Old peers without the field continue to work — planner
falls back to the legacy `capabilities` strings and treats them as
Full Agents on the reporting OS.

---

## 5. Prompt Lifecycle (ReAct, not big-DAG)

The planner does **not** produce a complete DAG upfront. It produces
the next subtask, sees the result, then decides the next one. This is
deliberately Claude-Code-style ReAct, not airflow-style DAG.

```
┌─────────────┐   prompt    ┌─────────────────────────────────┐
│   Client    │────────────▶│       Coordinator                │
│ (iOS/web/…) │             │                                  │
└─────▲───────┘             │  ┌─────────────────────────────┐ │
      │                     │  │ Session Log (SQLite)         │ │
      │  SSE stream         │  │  • all turns + tool calls    │ │
      │  of progress        │  │  • truth source of state     │ │
      │                     │  └─────────────────────────────┘ │
      │                     │              ▲                   │
      │                     │              │                   │
      │                     │   ┌──────────┴──────────┐        │
      │                     │   │ Planner (LLM call)  │        │
      │                     │   │  next subtask only  │        │
      │                     │   └──────────┬──────────┘        │
      │                     │              │                   │
      │                     │   ┌──────────▼──────────┐        │
      │                     │   │ Scheduler            │        │
      │                     │   │ caps ⊇ required_caps │        │
      │                     │   └──────────┬──────────┘        │
      │                     │              │                   │
      │                     └──────────────┼───────────────────┘
      │                                    │
      │                          dispatch (HMAC-signed)
      │                                    │
      │                ┌───────────────────┼───────────────────┐
      │                ▼                   ▼                   ▼
      │           ┌─────────┐         ┌─────────┐         ┌─────────┐
      │           │  Mac    │         │  Z13    │         │  ROG    │
      │           │ shell+  │         │ always- │         │ android │
      │           │ git+API │         │ on hub  │         │ + mobile│
      │           └────┬────┘         └────┬────┘         └────┬────┘
      │                │ result            │ result            │ result
      │                └───────────────────┼───────────────────┘
      │                                    │
      └────────────────────────────────────┘  next planner turn
```

Per-turn budget: planner LLM call (≤2s), scheduler decision (<10ms),
subtask execution (depends), result back to coordinator (<200ms over
Tailscale), session-log append (<50ms). The user sees streamed
tokens from the executing node, plus discrete `step.start` /
`step.complete` events on the SSE channel.

### Worked example

User on iPhone types:
> 跑一輪 phantom-mobile demo，把 log 整成週報 PDF 上傳 Drive

Planner emits subtasks one at a time:

| # | Subtask | required_caps | Selected node | Why |
|---|---------|---------------|---------------|-----|
| 1 | `phantom-mobile make demo-mock` | `shell, mobile_driver=android` | rog-phone | Only Android peer with the driver |
| 2 | `collect logs/*.json` (after #1) | `shell, file_edit_scope=fs` | rog-phone | Co-locate with #1 to avoid streaming raw logs over Tailscale |
| 3 | Summarise 2,300-line log into 8 bullets | `cloud_llm=anthropic` | mac | Has API key; local 3B accuracy insufficient at this length |
| 4 | Render Typst → PDF | `subprocess, has_typst` | mac | Typst installed only on Mac |
| 5 | Upload to Drive | `oauth_token=drive` | mac | OAuth token bound to Mac's keychain |

iPhone sees streamed progress on the SSE channel. It never executes
anything. Total latency dominated by step 1 (the actual demo run, ~40s);
everything else is sub-2s.

---

## 6. State Model

**Centralised session, stateless workers.** The coordinator's SQLite
session log is the single source of truth. Workers are pure
functions: `(subtask_payload, env) → result_blob`. No worker stores
state between subtasks; if the same worker gets two subtasks in a
session, each one carries the full required context.

Why not CRDT-replicated state? Because for a 1-coordinator deployment
it would cost us 10× the implementation complexity for 0× the user
benefit. Coordinator failover (re-elect a peer as coordinator) is on
the roadmap; until then, coordinator down = session paused, resume on
restart from the SQLite log. Acceptable.

### Failure semantics

- **Heartbeat** — every peer pings coordinator every 30 s. Three
  consecutive misses → peer marked unhealthy, in-flight subtasks
  re-dispatched.
- **Subtask timeout** — derived from the peer's `limits.max_task_seconds`
  capped against the planner's per-step budget. On timeout the
  coordinator cancels (best-effort) and retries on a different peer.
- **Idempotency** — every subtask carries a `task_id` UUID. Peers
  must dedupe on `task_id` so a retry after a flaky network does
  not double-execute a destructive op. This is the contract that
  makes re-dispatch safe.
- **Battery-host disappearance** — Android / iOS hosts go offline
  routinely (screen off, app suspended). Coordinator must not assume
  they will return; subtasks dispatched to them are eagerly re-tried
  on a wall-powered peer after `2 × typical_duration`.

---

## 7. Local-vs-Cloud LLM Routing

The planner running on the coordinator decides per-step whether to
call a local model or a remote API. Heuristics, in order:

1. **Subtask category** — classification / extraction / short
   summarisation → local 3B by default. Code reasoning, multi-file
   refactor, "explain this stack trace" → cloud.
2. **Input size** — local 3B caps at ~2 KB input for usable quality.
   Above that, escalate.
3. **Caller locality** — if the originating client is a battery
   device with a working local model, prefer doing the routing
   decision locally first (intent classification < 200 ms, avoids
   a Tailscale round-trip).
4. **Cost ceiling** — coordinator tracks $-spent per session. Above
   a configurable cap (`session.cloud_budget_usd`) the planner
   downgrades to local-only and warns the user.
5. **Failure fallback** — if cloud API returns 429 / 5xx twice in a
   row, planner switches to next-available cloud provider; if all
   fail, downgrades to local with a "degraded quality" banner.

The mapping from "subtask intent" to "category" is itself an LLM
call (cheap, e.g. Haiku or local 3B). The result is cached per
session so repeated subtasks of the same kind don't pay the routing
cost twice.

---

## 8. Per-OS Profiles

### 8.1 Linux

Full agent. Shell, git, browser (headless Chromium via playwright),
GPU inference (CUDA/Vulkan/ROCm), arbitrary subprocess, long-running
systemd service. Reference deployment is the Z13 always-on hub.

Coordinator-capable if `power = wall | always_on`.

### 8.2 macOS

Full agent **and** the reference coordinator host. Adds:
- MLX inference on Apple Silicon (`metal` GPU)
- Drive / iCloud / Calendar OAuth tokens in the system keychain
- LaunchAgent for autoevolve hourly loop

The 5/20 launch architecture assumes Mac is the primary coordinator;
Z13 becomes coordinator after item #6 of the manual checklist.

### 8.3 Windows

Full agent. Windows-specific quirks:
- Service registration via NSSM or `sc.exe create`
- WSL2 is *not* the same node — it shows up as a separate Linux peer
  with its own manifest. Both can coexist on the same physical box.
- No native CUDA in some configurations; planner reads
  `gpu_inference` from manifest, doesn't infer from OS.

### 8.4 Android (via Termux)

Full agent, with caveats. The `aarch64-linux-android` phantom binary
already builds and runs in Termux. What works:

- Real Linux userland: shell, git, curl, ssh, full POSIX
- Subprocess + fork-exec, no sandbox restrictions inside Termux
- Foreground service + wake lock → genuine long-running tasks
- Vulkan compute → llama.cpp can offload to Adreno/Mali GPU
- MediaPipe LLM API → Gemma 2 B running on phone NPU

What doesn't work:

- Anything outside Termux's home directory (Android scoped storage)
- Accelerometer / camera / native sensors (not relevant to terminal-class agents)
- Termux background limits on Android 12+ — must declare foreground
  service with `Termux:Boot` add-on for boot persistence

A flagship Android (SD 8 Gen 3 class) running phantom in Termux is
a legitimate cluster peer. The ROG device in the user's fleet fills
this role. Cheap phones with <8 GB RAM should be classified as
**Lite Agent** (cap `local_models = []`).

### 8.5 iOS — see §9

iOS gets its own section because the constraints and the optimisation
problem are qualitatively different.

---

## 9. iOS: How Close to Android Can We Push?

The current [INSTALL-IOS.md](./INSTALL-IOS.md) says "iOS is a thin
client only". This section is the design path to push that as far as
the platform actually allows. The honest ceiling is roughly **60 % of
Android's capability**, achievable in three layered tiers.

### 9.1 The hard constraints (non-negotiable)

| Constraint | Source | Implication |
|------------|--------|-------------|
| No JIT compilation | Apple `App Sandbox` policy | Rules out any LLM runtime requiring dynamic codegen. Rules out fast x86 emulation. |
| No fork-exec of non-bundled binaries | iOS kernel + sandbox | `git`, `cargo`, `npm`, `python` — all forbidden unless statically linked into the app bundle and signed |
| No unbounded background execution | iOS task assertion model | App suspended ~30 s after backgrounding. `BGProcessingTask` gives bursts up to a few minutes, no continuous loop. |
| App-bundle filesystem only | sandbox | Cannot edit "the user's git repo" because there is no such concept. The repo would have to be inside the app's container. |
| App Store review | policy, if distributed via Store | Bans "downloads code and executes it" — interpreted broadly. Sideload (TestFlight / AltStore / TrollStore) avoids this. |

### 9.2 Three tiers of iOS deployment

Pick the highest tier the user is willing to accept.

#### Tier 1 — Thin client (today's state)

`mobile.html` rendered in a WKWebView, talks SSE to coordinator over
Tailscale. Zero local execution. Zero local LLM. Works on any
non-jailbroken iPhone, including via App Store distribution.

What this can do: render output, accept input, push/receive
notifications. Useful 100 % of the time the coordinator is reachable;
useless offline.

#### Tier 2 — Lite Agent (the proposed upgrade)

A native SwiftUI app, sideloaded via TestFlight (paid Apple Dev
account) or AltStore/SideStore (free, 7-day re-sign). Embeds:

- **phantom core as a static library** — compile the Rust agent
  loop and HTTP client to a `staticlib` and link via `swift-bridge`
  or `UniFFI`. The agent loop runs in-process, no subprocess.
- **Apple Foundation Models** (iOS 26+) — the on-device 3B model
  Apple ships in the OS. Zero model download, exposed via the
  `FoundationModels` Swift framework. Use for intent classification,
  short summarisation, "did the user ask a question or issue a
  command" routing.
- **MLX-Swift fallback** for iOS 17 / 18, or when a stronger local
  model is wanted. Ship a 3 B Q4 weight (~1.8 GB) inside the app
  bundle, load via MLX. iPhone 15 Pro+ gives 15-25 tok/s.
- **Tailscale Network Extension** — Apple's official Tailscale app
  already does this. Our app talks to localhost; Tailscale's
  always-on tunnel makes the coordinator address reachable.
- **Bundled tools (no subprocess)** —
  - HTTP fetch: native `URLSession`
  - JSON: native `Codable`
  - Git read-only: `SwiftGit2` (libgit2 bound to Swift), works on
    repos inside the app container
  - File edit: sandbox-relative paths only
  - WebDAV / Drive / Dropbox upload: native HTTP, OAuth via
    `ASWebAuthenticationSession`

Declared `tools` in this manifest:

```jsonc
{
  "shell": false,
  "git": "read_only",
  "browser": false,
  "subprocess": false,
  "mobile_driver": null,
  "gpu_inference": "ane",
  "file_edit_scope": "sandbox",
  "long_running": false
}
```

The planner therefore dispatches to an iOS Lite Agent only when:
- intent is conversation / classification / summarisation, **and**
- input fits in local model context, **and**
- no shell or external file edit is needed.

For everything else, the iOS device acts as a thin client to the
home coordinator. The user sees the same UI either way; the
substitution is transparent.

#### Tier 3 — Power-user TrollStore / jailbreak

For users on supported iOS versions (14.0–16.6.1 and 17.0 via
TrollStore; jailbroken devices on any iOS), we can ship a build with
proper entitlements that allows `posix_spawn` of bundled binaries.
That unlocks ~80 % of Android capability — real `git`, real
`cargo`, real interpreters bundled inside the IPA.

This tier is not for the App Store path. We document it in
`docs/INSTALL-IOS-POWER.md` (to be written) and stop. It is not the
recommended path for any user who is not already comfortable with
TrollStore.

### 9.3 Bridging the gaps that remain

Even at Tier 2 there are jobs iOS cannot do natively. Three escape
hatches make them transparent:

1. **Silent push wake-up** — coordinator sends a `content-available: 1`
   APNs push when it has a low-latency subtask for the iOS lite agent
   (e.g. "classify this incoming message"). The app gets ~30 s to
   execute and report back.
2. **x-callback-url to a-Shell** — when the user explicitly wants a
   shell command, we open a-Shell via its `x-callback-url` scheme,
   pass the command, get the result back. This is genuine shell exec
   on iOS, just sandboxed in a-Shell's container, not ours. Useful
   for power users who already have a-Shell installed.
3. **Coordinator delegation (the default)** — for everything else,
   the iOS app is a remote control. This is the same as Tier 1, but
   the planner makes the substitution decision; the user doesn't.

### 9.4 What we deliberately don't do on iOS

- **No iSH** — x86 usermode emulation is genuinely interesting but
  too slow for our agent loop. We will not document it as a
  supported path.
- **No Apple Watch agent** — not enough RAM, no background, no
  practical use case beyond a "send prompt" surface that the iPhone
  already covers.
- **No "phantom as Shortcut action"** — the Shortcuts integration
  layer is shallow (URL scheme in, result out). Cute, but the same
  shortcut works against the coordinator over HTTPS without an iOS
  app at all.

---

## 10. Implementation Roadmap

Strictly ordered. Each step is a complete shippable improvement.

| # | Step | Effort | Pre-req | Status |
|---|------|--------|---------|--------|
| 1 | Extend `PeerStatus` with structured `NodeManifest` | 1 d | — | not started |
| 2 | Planner reads manifest for subtask dispatch | 2 d | #1 | not started |
| 3 | Local-vs-cloud LLM routing heuristic in planner | 1 d | #2 | not started |
| 4 | Per-task idempotency key (`task_id` UUID) + dedupe | 1 d | — | not started |
| 5 | Subtask timeout + re-dispatch on heartbeat miss | 2 d | #4 | partial (heartbeat exists, re-dispatch doesn't) |
| 6 | iOS Tier-2 SwiftUI app skeleton (no LLM yet) | 3 d | #1 | not started |
| 7 | iOS Apple Foundation Models integration | 2 d | #6, iOS 26 target | not started |
| 8 | iOS MLX-Swift fallback model bundle | 2 d | #6 | not started |
| 9 | Android Termux MediaPipe LLM provider | 2 d | — | not started |
| 10 | Coordinator failover (re-elect on heartbeat miss) | 5 d | #4, #5 | not started |
| 11 | TrollStore IPA build target + INSTALL-IOS-POWER.md | 2 d | #6 | not started |

Pre-5/20 launch scope: **none of the above**. The launch ships with
the existing hub-and-spoke, the existing flat capability list, and
the existing thin-client iOS HTML. The above is post-launch work.

---

## 11. Non-Goals

- Full mesh with CRDT-replicated state (10× complexity, 0 % user benefit at our scale)
- Federation across tailnets without the commercial broker (use the broker; that's why it exists)
- Symmetric iOS / Android feature parity (impossible by platform constraint; we will not pretend otherwise)
- Real-time collaborative editing across nodes (not the swarm's job; that's Y.js or Automerge territory)
- Sub-100 ms cross-node task dispatch (Tailscale + HMAC + JSON parse already costs ~50-150 ms; we live with it)

---

## 12. Open Questions

1. Should the manifest be self-reported or coordinator-verified? Self-report is simpler but lets a malicious peer overclaim. Defer until phantom-mesh has multi-tenant deployments.
2. When iOS Lite Agent does local inference but the result is wrong, who pays for the re-run on cloud? Likely policy: silent retry once, then surface the cost to the user.
3. Where does the planner itself run on the iOS-originated session when the home coordinator is unreachable? Two options:
   (a) iOS app degrades to local-only with a banner; (b) the app embeds a tiny planner that can decide "queue this for when coordinator is back". Lean toward (a) for simplicity.

---

*Last updated: 2026-05-13. Author: phantom-mesh maintainer. Companion to [ARCHITECTURE.md](./ARCHITECTURE.md) (single-node) and [CLUSTER-SCALE.md](./CLUSTER-SCALE.md) (existing cluster ops).*
