# Co-Evolution Architecture

How phantom-mesh handles the tension between **agent self-modification** and
**a coherent shared codebase** across all installed instances.

The short version: every user's phantom can autonomously fix issues it hits
on its own machine, and those fixes flow upstream as PRs to a single canonical
release. Three tiers (sandbox → recipes → core PR) keep "phantom v0.1.x" the
same thing on every machine while preserving each user's freedom to evolve.

## The problem

`autoevolve` lets a phantom binary modify its own source. If we ship that as-is,
every user's installed phantom diverges into a private fork after a week. The
project-level meaning of "phantom v0.1.0" evaporates: bug fixes don't propagate,
new features stay siloed, two phantoms in the same mesh can't agree on the wire
format, and any "release" we publish is immediately overwritten by local
autoevolve commits.

This is a real architectural decision, not a hypothetical. As of 2026, of the
14 mainstream AI agent CLIs surveyed (Aider, Goose, OpenHands, Continue, Cline,
Roo, Claude Code, Codex CLI, Gemini CLI, sst/opencode, fabric, llm, mods,
jcode), **only jcode lets the agent modify the agent's own source**, and even
jcode has not addressed divergence — they bet that the user is a power-user
who will git-push manually. We don't get to make that bet at OSS launch.

## The two patterns we are NOT taking

**(1) Pure source-mod-and-rebuild (jcode):** maximum power, no version story.
Every user gets a fork after week one. Wrong for OSS.

**(2) Pure extensions-outside-binary (Goose, Cline, Continue):** binary stays
immutable; customization is markdown/YAML in `~/.tool/`. Coherent but loses
phantom's distinguishing feature — the binary itself can't get smarter.

We take a third path that combines both.

## The model: Sandbox + Recipes + Gated Core PR

```
┌────────────────────────────────────────────────────────────────────┐
│  Tier 1 — SANDBOX  (autoevolve default)                            │
│  Writable: ~/.phantom-mesh/extensions/{prompts,skills,hooks}/      │
│  Read-only from agent: core/*.rs, anything under repo root         │
│  Distribution: optional. Local until user chooses to share.        │
└────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼ user opts to share
┌────────────────────────────────────────────────────────────────────┐
│  Tier 2 — RECIPE  (shareable artifact)                             │
│  Unit: one EvolveCheckpoint exported as content-addressed JSON     │
│  Carries: goal, plan, dead-ends, journey, patch (if any), descriptor│
│  Signed: ed25519 (per-user key in ~/.phantom-mesh/keys/)           │
│  Distribution: gist, git remote, or registry repo (Tier 2.5)       │
└────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼ recipe touches core/*.rs
┌────────────────────────────────────────────────────────────────────┐
│  Tier 3 — CORE PR  (gated upstream merge)                          │
│  Gate: --allow-core-evolve flag + interactive consent + signature   │
│  Output: NOT a local commit. A `git format-patch` on a fork branch │
│          + automated PR via `gh api` to the upstream repo          │
│  CI: cross-platform test matrix (mac/win/linux), CodeQL,           │
│       sensitive-path human-review gate (auth/, mesh.rs, keys.rs)   │
│  Merge: automerge bot if green AND no sensitive paths touched      │
│         else: human review label                                   │
│  Release: tagged version → all phantoms `phantom upgrade`          │
└────────────────────────────────────────────────────────────────────┘
```

### Why this works

- **Tier 1 is autoevolve's default.** A user installing phantom and turning
  on autoevolve does not silently start mutating Rust source. Their `core/*.rs`
  on disk continues to match upstream `phantom v0.1.x`. The agent improves
  prompts, hooks, and user-specific adaptations — exactly the surface the rest
  of the agent CLI ecosystem agrees should be customizable.

- **Tier 2 is the unit of cross-pollination.** EvolveCheckpoint is already a
  content-addressed JSON document with full audit trail (goal, plan, dead-ends,
  journey, artifacts, binary swaps). Adding `phantom evolve publish` and
  `phantom evolve adopt <recipe>` turns it into a recipe ecosystem, exactly
  Sakana AI's Evolutionary Model Merging pattern: small declarative artifact,
  heavy thing rebuilt locally.

- **Tier 3 is how the upstream stays alive.** Most OSS projects survive
  because contributions flow back. We just automate the flow: phantom on user
  A's Mac discovers a CJK render bug, autoevolve fixes it, the recipe is
  marked as touching core source, `--allow-core-evolve` is interactively
  approved, the patch becomes a PR, CI runs the same cross-platform matrix
  every human-PR runs, automerge bot lands it, next release ships it to user
  B and C.

### Why three tiers, not one

If we only did Tier 1, phantom never becomes a self-improving Rust agent —
just a self-improving prompt collection. We lose the differentiator.

If we only did Tier 3 (gated PR for everything), routine personal tweaks like
"I prefer the input box at the top" require a CI roundtrip. Friction kills
adoption.

If we only did Tier 2 (free recipe sharing without core PR), phantom becomes
unbounded plugin soup with no canonical version, the same place jcode is.

Three tiers gives users jcode-level power on their own slice (Tier 1+2) while
preserving "phantom v0.1.x means the same thing everywhere" (Tier 3 gates the
canonical release).

## Implementation phases

| Phase | Goal | Status |
|---|---|---|
| **0. Foundations** | EvolveCheckpoint module + mesh handoff | ✓ shipped (Phase 1+2) |
| **1. Sandbox** | autoevolve restricted to `~/.phantom-mesh/extensions/`; `core/` read-only | pending |
| **2. Recipe export/import** | `phantom evolve publish/adopt` + content-addressed signed JSON | pending |
| **3. Trust chain** | ed25519 signing of every published recipe + maintainer keychain | pending |
| **4. Core-PR pipeline** | `--allow-core-evolve` flag → fork branch + auto-PR via `gh api` | pending |
| **5. CI gate + automerge** | GitHub Actions cross-platform test matrix + automerge bot | pending |
| **6. Sync** | `phantom upgrade` pulls signed releases; daily timer to fetch new recipes | pending |

Each phase is one commit. None depend on the next; we can stop after Phase 1
and have a usefully-sandboxed product. Phase 4+5+6 together unlock the
co-evolution loop.

## Trust model

The hard problem with federated auto-PR is hostile patches: a bad actor
publishes a "fix" that smuggles in a backdoor, CI passes, automerge lands it,
1000 phantoms upgrade, all rooted.

Defenses, in order of importance:

1. **Sensitive-path human-review gate.** Any patch touching `core/src/auth/`,
   `core/src/mesh.rs`, `core/src/keys.rs`, `core/src/serve.rs::rpc_*`, or
   `templates/*.plist.tmpl` requires human review regardless of CI status.
   Defined as a label in `.github/co-evolution.toml`.

2. **Maintainer keychain.** Every published recipe is ed25519-signed.
   The upstream repo maintains `MAINTAINERS.md` listing trusted public keys.
   Recipes from non-listed keys go to a separate "community" queue with stricter
   gates (more reviewers, longer soak time before automerge).

3. **Sandboxed CI.** Tests run in fresh containers with no secrets, no
   network access except to crates.io, no write access outside the test tree.
   Even if a patch tries to exfiltrate, there's nothing to take.

4. **CodeQL + cargo-audit + clippy `-D warnings`.** Standard supply-chain hygiene.

5. **Signed releases.** Every release tag is signed. `phantom upgrade` verifies
   the signature against a hardcoded maintainer key embedded in the previous
   binary. Compromise requires control of both upstream AND a previous release.

6. **Automerge requires green CI on at least 2 platforms.** Single-platform
   CI failure is enough to require human review. Defends against
   platform-specific malicious branches.

## Versioning

`phantom --version` will print three numbers:

```
phantom 0.1.4 / core-sha 7a3f2b1 / extensions-rev 23
        ↑           ↑                  ↑
        │           │                  └── monotonic counter, user-local
        │           └── content hash of core/ — should match
        │                upstream tag's sha; if not, you forked
        └── upstream release semver
```

Existing tools collapse all three into one number. We keep them separate so
"have I forked?" is mechanically answerable: `core-sha` differs from upstream
release sha iff the user (or their autoevolve) modified `core/`. That's the
single bit answer to "are you running canonical phantom?"

## Roadmap

- **5/2 (Sat)** — this doc + 5 goals queued in `EVOLVE-GOALS.md`
- **5/2-3 weekend** — Phase 1 (Sandbox)
- **5/3-4** — Phase 2 (Recipe export/import)
- **5/5 Mon** — Phase 3 (Signing)
- **5/6-7** — Phase 4+5 (Core-PR pipeline + CI gate)
- **5/8** — Phase 6 (Sync) + first end-to-end test
- **5/9** — interview demo: phantom on Mac, evolve a bug fix, auto-PR'd,
  Linux phantom pulls the merged release
- **5/15** — OSS launch

## References

Synthesized from research surveys conducted 2026-05-01 covering jcode, Aider,
Goose, Continue, Cline, Roo Code, Claude Code, Codex CLI, Gemini CLI,
sst/opencode, fabric, llm, mods, OpenHands, NixOS overlays/flakes,
Sakana AI's CycleQD + evolutionary model merging, POET/Enhanced-POET,
MAP-Elites, Homebrew taps, OSS-Fuzz, GitHub Copilot Autofix, Project Naptime.
