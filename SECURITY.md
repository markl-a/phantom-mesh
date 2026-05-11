# Security Policy

## Reporting Vulnerabilities

**Please do not open public issues for security vulnerabilities.**

Email security reports to: `mark@phantom-mesh.dev`

Include:
- Description of the vulnerability
- Steps to reproduce
- Affected versions
- Your contact information

We acknowledge reports within **72 hours** and aim to patch critical issues within 7 days.

## Implementation Status (v0.1.0-alpha)

The following features are planned but not yet implemented in v0.1:

- **WASM sandbox** for tool isolation (planned v0.2)
- **Ed25519 node identity** keypair per node (planned v0.2)
- **ChaCha20-Poly1305 credential encryption** (`enc2:` prefix in agents.toml) (planned v0.2)

Currently implemented security features:

- Shell command allowlist (only pre-approved commands can execute)
- Workspace sandbox (file I/O restricted to designated workspace directory)
- Full confirmation mode (all AI operations require user confirmation)
- Path canonicalization against whitelist before execution
- L1 regex guardrail (8+ patterns blocking prompt injection and jailbreak attempts)
- L2 LLM-as-Judge for tool output safety evaluation
- Injection guard (multilingual override, ChatML, base64, obfuscation detection)
- Shared-secret Bearer token auth for cluster communication
- No telemetry — Phantom Mesh does not phone home

## Security Model

Phantom Mesh uses a **local-first** architecture with defense-in-depth.

### Node Identity & Authentication

- **Shared secret** for cluster authentication (Bearer token over HTTP)
- Ed25519 keypair per node and ChaCha20-Poly1305 key encryption are planned for v0.2

### Injection Defense

- **L1 Guardrail**: 8+ regex patterns blocking prompt injection, jailbreak attempts, and dangerous instructions
- **L2 LLM-as-Judge**: Separate LLM evaluates tool outputs for safety
- **Injection Guard**: Detects multilingual override, ChatML injection, base64 payloads, obfuscation

### Tool Execution Safety

- **Shell command allowlist**: Only pre-approved commands can execute
- **Workspace sandbox**: File I/O restricted to designated workspace directory
- **Full confirmation mode** (v0.1): All AI operations require user confirmation before execution
- **Path canonicalization**: All file paths expanded and checked against whitelist before execution

### Network Security

- **No telemetry**: Phantom Mesh does not phone home or collect usage data
- **Cluster communication**: HTTP with Bearer token auth between nodes
- **WireGuard/Tailscale**: Recommended for cross-network cluster nodes

## Data Handling

All data stored locally in `~/.phantom-mesh/`:

| Store | File | Contents |
|-------|------|----------|
| Core DB | `core.db` | Tasks, conversations, cron jobs |
| Conversations | `conversations.db` | Chat session history |
| Cluster | `cluster.db` | Peer registry, node state |
| Events | `data_event_index` table | Domain events with per-node sequencing |

SQLite databases use **WAL mode** for crash safety. Schema migrations create automatic backups before applying changes.

## Known History Issue (Pre-Launch)

**Commits `3abf406` and `0d5c714` contain exposed API keys.** These keys have already been revoked. Before making this repository public, run `scripts/clean-history.sh` to rewrite history and remove the leaked secrets.

Steps required before 2026-05-15 public launch:
1. Make a full backup of the repository
2. Run `bash scripts/clean-history.sh` and follow the prompts
3. Run `bash scripts/pre-open-source-checklist.sh` to verify the repo is clean
4. Force-push the rewritten history to all remotes
5. Confirm all previously exposed keys are revoked and replaced

Affected credential types: Google OAuth (`GOCSPX-`), Anthropic API keys (`sk-ant-`).

## Contributor Guidelines: Avoiding Secret Leaks

- **Never commit `agents.toml`** — it contains live API keys. It is in `.gitignore`.
- Use `agents.toml.example` as a template with placeholder values only.
- Before committing, run `git diff --staged` and check for any key-shaped strings.
- If you accidentally commit a secret: revoke the key immediately, then use `scripts/clean-history.sh` to scrub history before pushing.
- The CI pipeline (`security.yml`) runs [gitleaks](https://github.com/gitleaks/gitleaks) on every push and PR to catch leaks automatically.

## Threat Model

Phantom Mesh trusts the local environment. It does **not** protect against:
- Compromised operating systems
- Malicious users with local filesystem access
- Network attackers who have breached your VPN

Secure your machines and VPN credentials to protect your data.

---

*Last updated: 2026-04-23*
