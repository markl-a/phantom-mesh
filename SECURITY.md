# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in Clawtex, please **do not** open a public GitHub issue. Instead, report it privately so we can address it before public disclosure.

**Report via**: Open a [GitHub Security Advisory](https://github.com/clawtex/clawtex-core/security/advisories/new) (preferred), or email the maintainers directly if you cannot use GitHub.

Please include:
- A clear description of the vulnerability
- Steps to reproduce (proof-of-concept code if possible)
- The potential impact and affected versions
- Any suggested mitigations

We aim to:
- Acknowledge receipt within **48 hours**
- Provide an initial assessment within **7 days**
- Release a patch within **30 days** for confirmed critical issues

We will credit reporters in the release notes unless you prefer to remain anonymous.

---

## Supported Versions

| Version | Supported |
|---------|-----------|
| latest (main) | Yes |
| older releases | No -- please update to the latest version |

---

## Security Features

### Secret Encryption

All sensitive configuration values (API keys, passwords, tokens) are encrypted at rest using **ChaCha20-Poly1305** authenticated encryption. Encrypted values are stored with an `enc2:` prefix in `~/.clawtex/agents.toml` and related config files. The encryption key is derived from a master passphrase and never written to disk in plaintext.

### Prompt Injection Guard

`src/injection_guard.rs` scans every incoming message for 8 categories of prompt injection attacks before routing to an agent:

1. System override attempts (`"ignore previous instructions"`, etc.)
2. Role-switching attacks (`"you are now a different AI"`)
3. Encoding-based bypasses (base64, rot13 obfuscation)
4. Delimiter injection (`---`, `===` separator manipulation)
5. Prompt leak attempts (`"repeat your system prompt"`)
6. Jailbreak patterns (DAN, STAN, etc.)
7. Instruction smuggling (hidden Unicode, whitespace tricks)
8. System injection via context (injected tool outputs)

Detected injections are classified as Low/Medium/High severity. High-severity detections are blocked immediately; lower severity triggers a warning in the audit log.

### Shell Tool Allowlist

The `shell` tool only executes commands that appear on a pre-configured allowlist in `agents.toml`. Any attempt to run an unlisted command is blocked and logged. This prevents arbitrary code execution even if an agent is compromised.

### Rate Limiting

Built-in rate limiting prevents abuse:
- **840 actions/hour** per agent globally
- **280 actions/hour** per tool per agent

Exceeding limits returns a structured error code (E2xx) and triggers a cooldown.

### Human-in-the-Loop Approval

Sensitive tool calls (configurable per tool and per agent) require human approval via Telegram before execution. The approval gate is asynchronous -- the agent pauses until the operator approves or denies via a Telegram message. This applies by default to: `stripe`, `render_deploy`, `scaffold_saas`, `email_send` (bulk), and `shell` (allowlist exceptions).

### L1 + L2 Quality Gates

Every Hand workflow phase output passes through two quality gates:

- **L1 Guardrail** (`src/guardrail.rs`): Rule-based checks (output length, forbidden content patterns, format validation). No LLM calls -- fast and deterministic.
- **L2 LLM-as-Judge** (`src/evaluate.rs`): A separate LLM call evaluates the output for quality, accuracy, and safety. Failures halt the Hand and return an error.

### Audit Logging

All tool executions, approval decisions, cost events, and security events are written to the audit log (`src/audit_log.rs`). Logs include timestamps, agent identity, tool name, parameters (with secrets redacted), and outcomes.

### Budget Circuit Breaker

`src/agent_runtime.rs` includes a BudgetBreaker that tracks cumulative cost per agent. When an agent exceeds its configured budget, further LLM calls are blocked for a cooldown period. This prevents runaway cost from loops or adversarial inputs.

### Encrypted Transport

All cluster communication between Hub and Workers uses HTTPS when configured with TLS certificates. The Hub authenticates Workers with a shared Bearer token. The default token (`clawtex-hub-2026`) should be changed to a random secret in any production deployment.

---

## Security Hardening Checklist (for production deployments)

- [ ] Change the default Hub auth token in `agents.toml` to a long random string
- [ ] Run the daemon behind a reverse proxy (nginx, Caddy) with TLS
- [ ] Set `RUST_LOG=warn` in production (avoid logging sensitive data at `debug` level)
- [ ] Restrict the shell tool allowlist to only commands your workflows actually need
- [ ] Configure per-agent budget limits to prevent cost runaway
- [ ] Use environment variables or the encrypted secrets system for all API keys -- never put plaintext keys in `agents.toml`
- [ ] Limit filesystem access: run the daemon as a non-root user with write access only to `~/.clawtex/` and `~/.clawtex/workspace/`
- [ ] Enable Telegram approval gates for all destructive tools

---

## Known Limitations

- **Mobile workers** communicate over HTTP polling with no end-to-end encryption beyond the Bearer token. Use a VPN (e.g., Tailscale) for mobile workers on untrusted networks.
- **MCP stdio servers** are spawned as child processes with the same OS permissions as the daemon. Vet any MCP server code before adding it.
- **The LLM-as-Judge (L2) gate** uses an LLM, which can itself be manipulated. It is a defense-in-depth layer, not a complete security boundary.
