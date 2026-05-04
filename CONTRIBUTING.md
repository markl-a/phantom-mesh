# Contributing

Phantom Mesh is in early access — the Rust core source isn't open yet
(planned May 2026). What you CAN contribute right now:

## What's accepted today

✓ **Bug reports** — see the [issue templates](.github/ISSUE_TEMPLATE/).
   Especially welcome: install failures, TUI crashes, cluster RPC weirdness,
   model-dispatch hallucinations.

✓ **Doc fixes** — README typos, install instructions that don't match
   reality, broken ecosystem links. PRs welcome.

✓ **Issue triage / repro** — confirming someone else's bug on your platform
   is actually a huge help.

✓ **Installer improvements** — `installers/install.ps1` and `installers/install.sh`
   are open. Edge cases on weird Windows / non-zsh shells / corporate
   proxies / etc. are valuable.

## What's coming once the Rust core opens (May 2026)

- New tools (under `core/src/tools/`)
- LLM provider integrations
- Cluster RPC features
- TUI improvements

Hold those PRs until the source lands; meanwhile, file feature requests
so I know what to prioritize.

## Repo conventions

- Branch names: `fix/<short-desc>` or `feat/<short-desc>` for PRs
- Commit messages: imperative mood, prefix with type
  (`docs(readme): ...`, `fix(installer): ...`)
- One topic per PR — easier to review, easier to revert if needed

## Scope: what I won't merge

- Adding telemetry / analytics beacons
- Auto-update mechanisms beyond the existing `phantom cluster upgrade`
- Bundling third-party LLM API keys into the installer
- Anything that changes installer behavior without making it visible
  in the script (no obfuscated steps)

## Questions

GitHub Discussions is the right channel for "is this a bug or am I
holding it wrong?" type questions. Issues are for confirmed bugs +
feature requests.
