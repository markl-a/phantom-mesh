# Security policy

Phantom Mesh is early-access software. Bugs are expected. **Security
bugs deserve a private channel before disclosure** so they can be
patched before bad actors learn about them.

## Reporting a vulnerability

**Don't open a public issue for a security report.** Instead:

- Email: open via the [GitHub Security Advisory form](https://github.com/markl-a/phantom-mesh/security/advisories/new) (preferred — encrypted, structured)
- Or: DM on the contact channel listed at https://phantommesh.io

What helps in the report:
- What you found (RCE / auth bypass / info leak / supply-chain / etc.)
- Phantom version (`phantom --version`)
- Reproduction steps with the smallest input that triggers it
- Whether you've shared with anyone else yet

## Response timeline

- **48 hours**: acknowledgement that I've received and read it
- **7 days**: triage — assess severity, confirm reproduction, draft fix plan
- **30 days target**: ship a patched binary + advisory (faster for criticals)

I'm a single maintainer right now (see [README](README.md) — early
access). These timelines are best-effort, not contractual.

## Scope

In scope:
- The phantom CLI binary (downloaded via install.ps1 / install.sh)
- The phantommesh.io broker (OAuth, key vault, cluster RPC dispatch)
- The cluster-RPC protocol (HMAC, peer auth)
- Anything in this repo's `installers/` directory

Out of scope (report to the upstream maintainers):
- LLM provider issues (opencode.ai, groq, openrouter, ollama — report to them)
- Cloudflare Workers platform issues
- Issues that require physical access to your own machine

## Coordinated disclosure

If you'd like to publish your finding after the fix lands, that's
encouraged — happy to coordinate timing + CVE assignment if relevant.
Credit will be in the patch commit + advisory unless you ask to stay
anonymous.
