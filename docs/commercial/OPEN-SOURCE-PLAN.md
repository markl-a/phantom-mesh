# Open-Source Plan & Public/Private Split — spectyn-mesh

> 2026-06-01. Rewritten after actually cloning + scanning the live public repo.
> Supersedes my first draft (which was built on truncated reads and was wrong on
> several points). Purpose: you forgot the original plan — this is the verified
> current reality + what to do next.

## 0. TL;DR (the things that changed my recommendation)

- **A public repo + a full sync pipeline ALREADY EXIST.** I don't need to invent
  either. `markl-a/spectyn-mesh` is public, and the private repo already has
  `.public-exclude` + `scripts/sync-to-public.sh` + `prepare-public-release.sh` +
  `release-public.sh`. The public commits literally read *"sync from
  spectyn-mesh @ <sha>"*. The job now is **run the existing sync to refresh
  to v0.6.0**, not build something new.
- **The live public repo leaks no CREDENTIALS** (verified by clone + gitleaks +
  grep): no API keys/tokens/passwords/private keys (gitleaks clean, no
  sk-/ghp_/AKIA), no personal email. ✅
- ⚠️ **CORRECTION (was wrong in my first pass):** the public broker config is NOT
  placeholder-only. `spectynmesh-io/wrangler.toml` + `docs/DEPLOY-SPECTYNMESH-IO.md`
  carry REAL Cloudflare resource identifiers, currently public:
  D1 `database_id = 1d49ebb9-8a63-4116-a2de-f590e48d6a8b`, KV
  `id = 54738150373145229745d88f18a86a15`, CF account id
  `9dc655af3fd4ac4487eade25edcbaa7d`, Google OAuth `client_id 869770808980-...`
  (public by design), zone `spectynmesh.com`, R2 `phantom-binaries`, D1
  `phantommesh-prod`. These are IDENTIFIERS not CREDENTIALS — you can't access
  anything with them without the (un-committed) CF API token — but they fingerprint
  the production infra. **`.public-exclude` does NOT exclude `spectynmesh-io/`, so
  the next sync keeps publishing them** unless we placeholderize or exclude.
- **The public repo already publishes MORE than "just core"** — it includes `app/`,
  `spectynmesh-io/` (broker), `installers/`, `configs/`, `templates/`, `site/`,
  `docs/`. So the real decision for you is *strategic* (is exposing the broker +
  app what you want?), not *safety* (it's safe).

## 1. Verified current reality

### Public repo `github.com/markl-a/spectyn-mesh`
- Visibility PUBLIC. **License = apache-2.0** (recognized by GitHub — my earlier
  "NOASSERTION/MIT" note was wrong).
- Description (current, NOT the old v0.5.0 Telegram one): *"Self-hostable AI agent
  runtime — single Rust binary, Tailscale cluster, runs across
  Mac/Linux/Windows/Android/iOS without a cloud account. Apache 2.0."*
- **Default branch = `master`**, HEAD `9ca9b58` ("revert: restore README to
  download-link era"). 30 commits. Last push 2026-05-17.
- ⚠️ **Two divergent branches**: `master` (default/served) and `main` carry
  *different trees*. `master` top-level: `core app crates spectynmesh-io installers
  configs templates site docs scripts .github` + AGENTS/SPECTYN/CHANGELOG/
  RELEASE-NOTES/CONTRIBUTING/SECURITY/LICENSE/README/.gitleaks.toml/.mcp.json/
  Cross.toml/Makefile/EVOLVE-GOALS/agents.toml.example. The `main` tree additionally
  showed `.superpowers/ _bmad/ mobile/ test-results/` — **those should NOT be public
  and `.public-exclude` lists them as excluded**, so `main` may be a stale/dirty
  branch that predates the exclude rules. → **Action: confirm `main` doesn't expose
  `.superpowers/_bmad/mobile`; if it does, delete or overwrite the `main` branch.**
- History is clean of the API-key commits (`3abf406`, `0d5c714` → "Not Found").

### Existing sync pipeline (already in the private repo)
- `.public-exclude` — rsync exclude list. Already covers: `.env .dev.vars *.key
  *.pem credentials.json secrets/`, `.superpowers/ _bmad/ docs/superpowers/
  BIG-GOAL* docs/runbooks/ *-PRIVATE.md AUTONOMOUS-WORKLOG* SNAPSHOT* *.plist
  docs/INSTALL-* docs/DEPLOY* docs/CLUSTER* docs/TAILSCALE* mobile/ test-results/
  ios-sandbox/ Payload/ *.ipa *.mobileprovision`, build artifacts.
- `scripts/sync-to-public.sh` — snapshots the working tree (NO git history),
  rsyncs with `--exclude-from=.public-exclude`, commits to public with a single
  "sync from spectyn-mesh @ <sha>" message. **This is exactly the clean-
  snapshot pattern — it already does it right.**
- `scripts/prepare-public-release.sh`, `scripts/release-public.sh` — release wrappers.

### My local hand-built snapshot (now redundant given the above)
- `~/Documents/GitHub/spectyn-mesh-public/` (core + crates/pm-types + docs +
  installer + LICENSE×2 + templates). Built it before I found the existing pipeline.
- After adding `templates/`, it **builds standalone**: `cargo build` RC=0, 73 MB
  binary. gitleaks = 13 findings, ALL false-positive (test fixtures, vendor
  `xterm.js`, broker endpoint-name string constants — no real secret).
- **Recommendation: discard this; use `scripts/sync-to-public.sh` instead** (it's
  the maintained path and covers app/installers/etc. that my hand snapshot dropped).

## 2. Public / private split — the rule (reconciled with `.public-exclude`)

| Component | State now | Keep? |
|---|---|---|
| `core/`, `crates/`, `templates/`, `configs/`, `installers/`, `scripts/` | PUBLIC | ✅ yes |
| `app/` (Tauri) | PUBLIC | ✅ ok (already out; clean) |
| `spectynmesh-io/` (broker) | PUBLIC, placeholder-only config | 🟡 YOUR CALL — safe, but exposes broker business logic. Keep public or move to exclude. |
| `docs/` (generic) | PUBLIC | ✅ yes |
| `.superpowers/ _bmad/ docs/superpowers/ mobile/ test-results/ ios-sandbox/` | excluded by `.public-exclude` | 🔴 stay private (verify `main` branch isn't still exposing them) |
| INSTALL-*/DEPLOY*/CLUSTER*/TAILSCALE*/BIG-GOAL*/runbooks/*.plist | excluded | 🔴 stay private |
| secrets (`.env .dev.vars *.key *.pem credentials.json secrets/`) | excluded | 🔴 never |
| git history w/ API keys | not in public | 🔴 never + rotate keys at provider |

**The split is already encoded in `.public-exclude` and it's sound.** Only two open
items: (a) decide broker public-or-not, (b) confirm the `main` branch isn't leaking
the excluded dirs from before the exclude rules existed.

## 3. Recommended next actions (nothing pushed yet)

1. **Read `scripts/sync-to-public.sh` fully** + dry-run it (rsync `--dry-run`) to
   see exactly what v0.6.0 would publish vs the current public tree. (I can do this.)
2. **Decide the broker**: keep `spectynmesh-io/` public (it's clean) or add it to
   `.public-exclude`. Strategic, not safety.
3. **Fix the `main`/`master` branch split**: make the public repo single-branch
   (point default at one, delete/overwrite the other) so there's no stale tree
   silently exposing excluded dirs.
4. **Run the sync** to refresh public to v0.6.0 via `sync-to-public.sh` (clean
   snapshot, no history) → review the diff → push.
5. **Update the public README/intro** (your "更新簡介" ask) as part of the sync —
   align it with v0.6.0 (Three Pillars, current feature set), keep the dual
   license note accurate.
6. **Rotate the 2 leaked API keys** at the provider — independent of all the above;
   they're only in *private* history but should be rotated regardless.

## 4. Sustainable workflow going forward
- Keep using `sync-to-public.sh` per tagged release (v0.6.0, v0.7.0…), not per
  commit. Public lags private by a release — normal and safe.
- Add a `gitleaks` gate to `sync-to-public.sh` if not already there, so a sync
  aborts on any real-secret finding.
- Keep the public repo single-branch to avoid stale-tree exposure.

Nothing here is pushed. Public repo is untouched. The verified facts above are from
a live clone + scan on 2026-06-01.

---

## 6. Morning handoff — exact steps (prep done 2026-06-01, autonomous)

**What I changed in the PRIVATE repo (all reversible, nothing pushed, nothing committed):**
- `.public-exclude` — added exclusions: `/docs/superpowers/` (~190 internal files),
  `/.ai-shared/`, `/scripts/ai/`, `/scripts/test-matrix.nodes` (tailnet IPs),
  `/scripts/win-dev-loop.ps1` (personal path), `/RESUME_SEND_CHECKLIST_*.md`,
  `/WHAT-SHIPPED.md`, `/docs/DEPLOY-MAC-STAGING.md`, `/docs/PUBLISHING-BINARIES.md`,
  `/docs/REMOTE-CLAUDE-CODE.md`, `/docs/TOMORROW-GOBAG.md`, `/docs/OPEN-SOURCE-PLAN.md`
  (this file — self-excludes because it catalogs the real CF IDs),
  `/scripts/scrub-public.sh`. Removed the earlier WRONG broker block (the private
  `wrangler.toml` is already placeholderized, so it ships as a template).
- New `scripts/scrub-public.sh` — rewrites `spectyn-mesh` → `spectyn-mesh`
  in the staged public tree only. Wired into `sync-to-public.sh` (runs after rsync
  on --apply; previews on dry-run).
- Scrubbed `core/tests/fixtures/README.md` (`spectyn-mesh` literal).

**Verified clean (file-based, 2026-06-01):** staged publish set (21,206 files) →
markers EMAIL=0, TAILNET=0, CF-IDs=0, SLUG=0 (after scrub); gitleaks 1.86 GB scan
= "no leaks found" RC=0; standalone `cargo build --bin spectyn` of the staged tree
= RC=0. Sensitive dirs (superpowers/.ai-shared/scripts/ai/tasks) confirmed ABSENT
from the staged set; key public files (core, crates/pm-types, spectynmesh-io/src,
wrangler.toml placeholder, templates, LICENSE-MIT/APACHE, README) confirmed PRESENT.

**The morning sequence (each step waits for your OK; the channel was glitchy
overnight so re-verify on screen before any push):**

1. **Dry-run review** (no writes):
   `bash scripts/sync-to-public.sh /tmp/pub-clone`
   → reads the rsync itemized list + the slug-scrub preview. Skim for surprises.
2. **Apply to the local public clone** (writes to /tmp/pub-clone only, still not pushed):
   `bash scripts/sync-to-public.sh /tmp/pub-clone --apply --delete`
   `--delete` is REQUIRED so files already public but now excluded get removed
   (the old leaky `wrangler.toml` with real CF IDs, plus stale build artifacts).
   The script auto-runs scrub-public.sh after rsync.
3. **Re-verify the clone before pushing** (belt-and-suspenders):
   `gitleaks detect --source /tmp/pub-clone --no-git` → expect no leaks.
   `grep -rI spectyn-mesh /tmp/pub-clone --exclude-dir=.git` → expect none.
   Confirm `/tmp/pub-clone/spectynmesh-io/wrangler.toml` shows `__CF_ACCOUNT_ID__`
   etc., NOT the real IDs.
4. **Commit + push master** (THIS is the first irreversible/public step — needs
   explicit go):
   `cd /tmp/pub-clone && git add -A && git commit -m "sync from spectyn-mesh @ <sha>" && git push origin master`
5. **Handle the public `main` branch** (separate decision — it's the ~97-file leak:
   `_bmad/ .superpowers/ docs/superpowers/ mobile/`). Options: delete it
   (`git push origin --delete main`) or overwrite it from clean master. Deletion
   does NOT un-expose what's already been cloned/indexed; it only stops further
   exposure. Also consider pruning the ~14 dependabot branches +
   `monorepo-runtime-2026-05-08-clean` + `chore/restore-download-link-readme`.
6. **Update README/intro** for v0.6.0 as part of (or right after) the sync — the
   "更新簡介" ask. The public description is already current ("self-hostable AI
   agent runtime…").
7. **Rotate the 2 leaked API keys** (commits `3abf406`/`0d5c714`) at the provider —
   independent of all the above; they're only in PRIVATE history but rotate anyway.

**Local folders right now (cleanup note):**
- `~/Documents/GitHub/hailmary/spectyn-mesh` — the PRIVATE source (this repo).
- `/tmp/pub-clone` — clone of the live PUBLIC repo (the sync destination).
- `/tmp/publish-staging` — the verified clean publish set (scratch; safe to delete).
- `~/Documents/GitHub/spectyn-mesh-public` — my earlier hand-built snapshot, now
  REDUNDANT (the sync pipeline supersedes it). Safe to delete to avoid confusion.
