# Contributor Funnel — User-side autoevolve as upstream contribution loop

**Status**: 🟡 DESIGN DRAFT — captures user 5/2 vision; targets v0.2-v0.3 sprints
**Effective**: not yet — v0.1.0 ships without this; this is the post-launch evolution
**Companion**: `docs/CO-EVOLUTION.md` (Tier 1/2/3 sandbox model — the foundation)
**Authority**: this doc extends CO-EVOLUTION.md; doesn't replace it.

---

## 0. The vision (user's framing, 2026-05-02)

> "Each user runs phantom and uses LLM (or modifies their own version);
> there should be a mechanism that makes their changes a candidate for
> a new version that all systems can run. The user's name gets added
> to the contributor list. Their version may stay specialized for them,
> but their modifications can flow up to developer review. Issues they
> resolve, features they build, can land in the next version."

This is **the contribution funnel**: user modifies → automatically
becomes candidate → maintainer reviews → next release contains their
name → all users get the fix on `phantom upgrade`.

It's the OSS dream: **every user is potentially a contributor; every
successful local fix is potentially a global upgrade**; no friction
beyond the user's choice to share.

---

## 1. What current autoevolve already does (the foundation)

| Capability | Status (5/2) |
|---|---|
| Single-shot agent loop (`phantom evolve "<goal>"`) | ✅ first success 4/27, $0 on free tier |
| EvolveCheckpoint as content-addressed JSON (atomic save, audit-trail) | ✅ Phase 1 ship |
| Mesh handoff RPC (HMAC-secured cross-machine baton) | ✅ Phase 2 ship (commit `027afe8`) |
| Goals queue (`EVOLVE-GOALS.md` round-trip parse) | ✅ shipped |
| Free-tier provider chain (Groq + opencode `*-free`) | ✅ shipped — autoevolve is $0 to run |
| Distributed evolve (`--distributed` decompose-and-fan-out) | 🟡 wiring exists, not real-mesh-verified |

→ The **EvolveCheckpoint JSON** is already the right unit of value.
It carries goal + plan + dead-ends + patch + journey. The funnel just
needs to add: identity, broker ingestion, upstream PR pipeline,
attribution.

---

## 2. What's missing for the user's vision (concrete gap list)

| # | Gap | Status (5/2 update) | Lands in |
|---|---|---|---|
| 1 | **Per-user identity** — ed25519 keypair | ✅ shipped (commit `4a61a0c`) | v0.1.0 — `phantom keys init` |
| 2a | Recipe export — `phantom evolve publish` (local + ed25519 sign) | ✅ shipped (commit `cbbbe50`) | v0.1.0 — `--private` default |
| 2b | `phantom evolve adopt <recipe>` — verify + apply | ⏸ deferred | v0.2 |
| 3 | **Broker as recipe inbox** — phantommesh.io accepts signed recipes, classifies tier, queues | ⏸ deferred (Cloudflare DNS migration prereq) | v0.2 |
| 4 | **Opt-in auto-publish** — `phantom autoevolve --share-recipes` | ⏸ deferred | v0.2 (depends on §3) |
| 5 | **Auto-PR pipeline** — broker forks upstream, pushes patch, opens PR | ⏸ deferred | v0.2 |
| 6 | **CONTRIBUTORS.md auto-append on merge** | ⏸ deferred | v0.2 (depends on §5) |
| 7 | **Reputation / public contributor dashboard** | ⏸ deferred | v0.4 |
| 8 | **Specialization preservation** — `~/.phantom-mesh/extensions/{prompts,skills,hooks}/` folder convention | ✅ shipped (commit `ed6e2dd`) | v0.1.0 — folder exists; loader ships in v0.1.0 too |
| 9 | **Issue → solution attribution** — `phantom evolve --solve <issue-num>` closes the issue | ⏸ deferred | v0.3 — needs gh api integration |
| 10a | **Privacy default** — `--private` is the default for `phantom evolve publish` | ✅ shipped (commit `cbbbe50`) | v0.1.0 |
| 10b | `--share` flag for explicit opt-in upload | ⏸ deferred | v0.2 (depends on §3) |
| 11 | **CO-EVO Phase 1 sandbox guard** — autoevolve refuses `core/` `app/` `templates/` `scripts/` writes by default | ✅ shipped (commit `fcd9bd1`) | v0.1.0 — `--allow-core-evolve` opts out |

**Tally**: 5 of 11 shipped in v0.1.0 (45%); 6 deferred to v0.2/v0.3/v0.4.

The five shipped (#1, #2a, #8, #10a, #11) form the **day-1 OSS user
infrastructure**: identity + workspace + sandbox + signed-but-local
recipe export. The remaining six need the broker + GitHub OAuth
flow + automerge bot to be live, which depends on Cloudflare DNS
migration (Phase 0 of the post-launch sprint).

---

## 3. The full architecture (3-layer flow)

```
═══════════════════════════════════════════════════════════════════════
USER LAYER (each user's machine)
═══════════════════════════════════════════════════════════════════════

  Onboarding (one-time):
    $ phantom keys init
      → ~/.phantom-mesh/keys/{ed25519.priv, ed25519.pub}
    $ phantom keys link --github
      → OAuth flow with broker
      → broker stores: { pub_key → github_user → email }
      → user is now identifiable across the mesh

  Daily flow:
    $ phantom autoevolve --watch --share-recipes
      ↓
    LLM agent makes a change in core/*.rs (or in extensions/)
      ↓
    cargo test passes
      ↓
    EvolveCheckpoint serialised to ~/.phantom-mesh/evolve-checkpoints/
      ↓
    phantom evolve publish (auto, if --share-recipes)
      ↓
    Recipe = ed25519-signed JSON containing:
      - goal, plan, dead_ends, journey
      - patch (git format-patch blob, if touches code)
      - descriptor: {platform, phantom_version, target_files_class}
      - signature: ed25519(body, user's priv key)
      - author: { pub_key, github_user (linked at onboarding) }
      ↓
    POST https://phantommesh.io/recipe

═══════════════════════════════════════════════════════════════════════
BROKER LAYER (phantommesh.io / Cloudflare Workers)
═══════════════════════════════════════════════════════════════════════

  POST /recipe handler:
    1. verify ed25519 signature against user's known pub key
    2. classify by file paths in patch:
         - touches only ~/.phantom-mesh/extensions/   → Tier 1 catalog
         - touches scripts/, docs/, tests/             → Tier 2 fast-track
         - touches core/*.rs, app/*.rs                 → Tier 3 PR queue
         - touches sensitive (auth/, mesh.rs, keys.rs) → Tier 3 + human
    3. record in D1: {recipe_sha, author, tier, classification, status}
    4. return: { recipe_url, tier, status: "queued" }

  Tier 1 (catalog only):
    → recipe stays in registry, others can `phantom evolve adopt <url>`
    → no upstream change

  Tier 2 (fast-track):
    → broker auto-creates PR via gh api
    → CI runs (cross-platform test matrix)
    → if green → automerge to upstream
    → if amber → notification to maintainer

  Tier 3 (core/* PR):
    → broker forks markl-a/phantom-mesh-private to a sandbox repo
    → push patch to a new branch named auto/<sha>
    → open PR to upstream with body = EvolveCheckpoint markdown
    → tag PR with: auto-evolve, <platform>, <classification>
    → PR body Co-Authored-By: <github_user> <noreply email>

═══════════════════════════════════════════════════════════════════════
UPSTREAM LAYER (github.com/markl-a/phantom-mesh)
═══════════════════════════════════════════════════════════════════════

  GitHub Actions / CI:
    .github/workflows/co-evolution.yml
    - cargo test on macos-latest + windows-latest + ubuntu-latest
    - CodeQL
    - cargo-audit
    - clippy -D warnings
    - 4-agent QA review (subagent + codex + gemini + opencode)

  Automerge bot rules (3 conditions, all AND):
    (a) all platform CI green
    (b) no sensitive paths touched (core/auth/, mesh.rs, keys.rs,
        serve.rs, templates/)
    (c) author's pub key in MAINTAINERS.md trusted list

  All ✓ → automerge → release Action triggers
  Any ✗ → label "human-review-required" → maintainer manual review

  Post-merge automations:
    1. .github/workflows/credit-contributor.yml
       - extracts Co-Authored-By from merged commit
       - appends to CONTRIBUTORS.md (if not present)
       - opens follow-up PR to update CONTRIBUTORS.md
    2. CHANGELOG entry auto-generated:
       "fix(<area>): <goal-summary> by @github_user"
    3. Tag release if auto-tag rules met

  Each user's `phantom upgrade`:
    - Curl latest tag's matching artefact
    - Verify maintainer signature
    - Atomic swap (bootout → swap → ad-hoc codesign → bootstrap)
    - User A who contributed sees:
        $ phantom --version
        → "phantom 0.2.1 (... built 2026-05-22)"
        → release notes credit them

  User A continues to run:
    canonical phantom 0.2.1 core
    + ~/.phantom-mesh/extensions/ (their personal customization)

  → "Same canonical core for everyone, plus user-specific extensions"
```

---

## 4. Specialization preservation (the "personal fork" mechanic)

Every user has TWO scopes:

```
~/.phantom-mesh/
├─ keys/                  ← identity (signed recipes)
├─ extensions/            ← Tier 1: personal customization, NEVER goes upstream
│  ├─ prompts/
│  │  ├─ coder-vim.md             # User's vim-style coder prompt override
│  │  └─ master-zh-tw.md          # User's zh-TW system prompt
│  ├─ skills/
│  │  ├─ git-rebase-helper.json   # 3-step composite tool
│  │  └─ deploy-staging.json
│  └─ hooks/
│     ├─ pre-tool/
│     │  └─ audit-shell.sh        # Logs every shell command
│     └─ post-agent/
│        └─ notify-slack.sh
├─ recipes/               ← Tier 2 candidates (locally generated, opt-in publish)
│  ├─ <sha1>.json
│  └─ <sha2>.json
├─ evolve-checkpoints/    ← autoevolve state (ephemeral)
└─ events.jsonl           ← diagnostic log (NEVER auto-uploaded)
```

When `phantom upgrade` swaps the canonical binary (e.g. v0.1.0 → v0.2.0):
1. binary is replaced atomically
2. **`extensions/` is preserved untouched**
3. phantom on next start re-loads extensions, applying them on top of new core
4. If extension API broke (rare; Tier 1 has stable contract): user prompted to resolve via:
   - `phantom extensions migrate <ext-name>` (auto-fix where possible)
   - or accept loss of extension with summary of what changed

**Key invariant**: the upgrade NEVER silently drops customization.
Either it migrates cleanly, or it explicitly tells the user.

This is exactly the **Emacs/Neovim plugin system** model:
- Same canonical Emacs everywhere
- Each user's `init.el` / `init.lua` adds personal customization
- New Emacs version preserves user's `init.el`

---

## 5. Identity + attribution (the "your name in the credits" piece)

### 5.1 ed25519 keypair (per-user, per-machine)

```
$ phantom keys init
✓ Generated keypair at ~/.phantom-mesh/keys/
  - ed25519.priv (0600 perms; never leaves this machine)
  - ed25519.pub  (broadcast to broker on first sync)
```

### 5.2 GitHub OAuth link (one-time)

```
$ phantom keys link --github
→ opens browser → OAuth flow on phantommesh.io
→ broker stores: pub_key_b64 → github_username → noreply_email
→ this is the ONLY identifying info phantom carries upstream
```

### 5.3 Auto-recipe publication

```
$ phantom autoevolve --watch --share-recipes
... [agent succeeds at goal, cargo test green]
✓ Recipe ~/.phantom-mesh/recipes/cdf3a8b9.json (signed)
✓ Published to phantommesh.io/recipe → tier=2, status=queued
✓ PR https://github.com/markl-a/phantom-mesh/pull/847 opened by phantom-bot
  Co-Authored-By: yourname <yourname@users.noreply.github.com>
```

### 5.4 Auto-credit on merge

`.github/workflows/credit-contributor.yml`:

```yaml
on:
  pull_request:
    types: [closed]

jobs:
  credit:
    if: github.event.pull_request.merged == true
    steps:
      - extract Co-Authored-By trailer from PR commits
      - if not in CONTRIBUTORS.md: open PR appending the user
      - on the next release CHANGELOG, group commits by author
```

After merge, `CONTRIBUTORS.md` looks like:

```markdown
## Contributors

- @markl-a (maintainer)
- @user-a (4 PRs: CJK render, ...)
- @user-b (2 PRs: ...)
- @user-c (1 PR: fix the spinner overflow)
```

And next release CHANGELOG:

```markdown
## v0.2.1 (2026-05-22)

### Fixes
- fix(tui): CJK render width on combining diacritics — by @user-a (#847)
- fix(mesh): retry deadline jitter — by @user-c (#891)

### Features
- feat(tools): web_search via Brave API — by @user-b (#852)
```

User opens the PR, sees their name in CONTRIBUTORS.md, sees their fix
in CHANGELOG.

---

## 6. Issue → solution attribution

For users who want to actively contribute (not just incidentally):

```
$ phantom evolve --solve 234
→ phantom fetches issue #234 body via gh api
→ agent reads issue, plans approach
→ standard autoevolve loop with extra context: "this is to close #234"
→ on success, recipe carries "solved_issue: 234"
→ PR opened with body line "Closes #234"
→ on merge, GitHub auto-closes #234 + credits user
```

This makes phantom a **personal solo contribution agent**: file an
issue you want fixed, run `phantom evolve --solve <num>`, PR opens
automatically.

---

## 7. Reputation + visibility (optional but valuable)

phantommesh.io public dashboard:
- Top contributors by recipes accepted
- Top contributors by Tier 3 PRs merged
- Recipes adopted N times (popularity)
- Issues solved by phantom this week

→ Users have a public profile they can share. Becomes a portfolio
piece. Some contributors may earn:
- Reviewer rights for non-sensitive paths
- Direct merge access for trusted contributors
- Invitation to private channels (testing pre-release)

---

## 8. Privacy + opt-out (the responsible path)

### 8.1 Default: opt-in publish

```
$ phantom autoevolve --watch
→ runs locally only; no broker calls
$ phantom autoevolve --watch --share-recipes
→ explicit opt-in
```

### 8.2 Per-recipe override

```
$ phantom evolve "<sensitive personal automation>"
... [success]
$ phantom evolve publish --private
→ saved locally, NOT pushed to broker
```

### 8.3 What's never uploaded

| Data type | Goes to broker? |
|---|---|
| Recipe body (goal, plan, patch) | ✅ when --share-recipes set |
| ed25519 public key | ✅ |
| GitHub username | ✅ (you opted in via OAuth) |
| ed25519 private key | ❌ NEVER |
| Crash logs | ❌ NEVER (stays local in ~/.phantom-mesh/crashes/) |
| Conversation transcripts | ❌ NEVER |
| Tool-call output (file_read of your private files) | ❌ NEVER |

The only thing that leaves your machine is **the recipe** (what you
opted to share). And only if you opt in.

---

## 9. Concrete EVOLVE-GOALS for v0.2-v0.3 sprints

In priority order:

### v0.2 sprint (5/15 → 5/22; 1 week)

```
- [ ] CO-EVO Phase 1 — sandbox guard (autoevolve refuses to write
      outside ~/.phantom-mesh/extensions/ without --allow-core-evolve flag)
- [ ] MULTI-DEV Gap 1 — GitHub Actions release matrix (single binary
      truth across 5 platforms)
- [ ] MULTI-DEV Gap 3 — phantom doctor --mesh (drift detection)
- [ ] MULTI-DEV Gap 4 — phantom upgrade (atomic swap with extension
      preservation)
- [ ] CO-EVO Phase 2 — phantom evolve publish/adopt (recipe export +
      ed25519 sign, locally only, no broker yet)
- [ ] CO-EVO Phase 3 — phantom keys init + GitHub OAuth link via broker
```

### v0.3 sprint (5/22 → 5/29; 1 week)

```
- [ ] CO-EVO Phase 4 — auto-PR pipeline (broker forks + pushes + opens
      PR; CI runs the .yml from Phase 5)
- [ ] CO-EVO Phase 5 — CI gate + automerge bot
- [ ] CONTRIBUTOR-FUNNEL §5 — CONTRIBUTORS.md auto-append on merge
- [ ] CONTRIBUTOR-FUNNEL §6 — phantom evolve --solve <issue> integration
- [ ] CONTRIBUTOR-FUNNEL §8 — privacy + opt-out flags + audit
- [ ] phantom upgrade with extension migration prompts
```

### v0.4 sprint (5/29 → 6/5; 1 week)

```
- [ ] CONTRIBUTOR-FUNNEL §7 — phantommesh.io contributor dashboard
- [ ] CO-EVO Phase 6 — phantom evolve sync (daily upstream pull
      optional; signature verification before swap)
- [ ] Recipe registry / popularity counter / search
- [ ] Reputation-based reviewer rights for trusted contributors
```

---

## 10. Why this works (the trust model)

The 3-tier model from CO-EVOLUTION.md provides containment. The
contributor funnel adds attribution + bidirectional flow. Combined:

| Concern | Resolution |
|---|---|
| 「user changed it but I'm forced to follow」 | Tier 1 stays local; only Tier 2/3 upstream after CI + (often) human review |
| 「user specialized version」 | Tier 1 extensions are isolated; canonical core stays the same; both update independently |
| 「change quality varies」 | CI matrix on every PR; sensitive paths require human; trusted contributors earn fast-track |
| 「does the contributor get credit」 | CONTRIBUTORS.md auto-append, CHANGELOG by-author grouping, public dashboard |
| 「user privacy」 | Default opt-in; nothing leaves machine without explicit `--share-recipes` |
| 「bad actor problem」 | ed25519 signature on every recipe; broker can revoke a key; sensitive paths gate human review |
| 「fork drift like jcode」 | release matrix = single binary truth; `phantom doctor --mesh` drift detection; `phantom upgrade` atomic swap |

This is the **Linux kernel model** with auto-PR ergonomics:
- Linus / lieutenants = automerge bot + maintainers for sensitive paths
- Anyone can patch upstream = `phantom evolve --share-recipes`
- Linux Foundation handles release matrix = GitHub Actions
- Long-tail kernel users keep custom config / modules = Tier 1 extensions
- New kernel rebases against your config = `phantom upgrade --migrate-extensions`

---

## 11. What it WOULD NOT do (intentional limits)

- **No silent uploads.** Every share is opt-in.
- **No mandatory updates.** Users can stay on old phantom forever.
- **No central control of forks.** Anyone can fork phantom-mesh and
  run their own broker; the auto-PR flow only works against
  `markl-a/phantom-mesh-private`'s upstream because that's where the
  broker points.
- **No code-quality enforcement at recipe time.** Quality check is at
  CI in upstream, not at broker (broker is a thin classification layer).
- **No anonymous contributions.** If you want your name in
  CONTRIBUTORS.md, you must link a GitHub account. (You can use a
  pseudonym GitHub account.)

---

## 12. Open questions (for v0.5+)

- **Multi-broker federation** — should there be alternative brokers
  (not just phantommesh.io)? Useful for orgs that want a private
  contributor funnel inside their company.
- **Recipe versioning** — recipe says "for phantom 0.2.0"; can it be
  auto-rebased onto 0.3.0?
- **Recipe registry search** — fuzzy-match on goal text, "this fix
  is similar to issue #X"
- **Cross-recipe composition** — recipe A and recipe B both touch
  tui.rs; can broker queue them serially or do auto-merge resolution?
- **Recipe reputation** — recipe adopted by 1000 users; should that
  auto-promote to Tier 3 fast-track?

---

## References

- `docs/CO-EVOLUTION.md` — 3-tier sandbox / recipe / core PR model (the foundation this extends)
- `docs/SELF-EVOLVE.md` — first successful self-fix transcript (4/27)
- `core/src/evolve_checkpoint.rs` — EvolveCheckpoint serialised JSON shape
- `EVOLVE-GOALS.md` — v0.2 sprint goal queue (CO-EVO Phase 1-6 + MULTI-DEV Gap 1-6)
- Sakana AI's Evolutionary Model Merging — recipe pattern inspiration
- jcode `ReloadContext` — EvolveCheckpoint inspiration (single-machine; this extends to mesh)
