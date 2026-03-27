# Contributing to Phantom Mesh

Thank you for your interest in contributing to Phantom Mesh. This guide covers development setup, code style, architecture, and the PR process.

Phantom Mesh is licensed under the [MIT License](LICENSE).

---

## Prerequisites

- Rust 1.78 or newer
- SQLite 3 (pre-installed on Linux/macOS; install from https://sqlite.org on Windows)
- (Optional) Python 3.9+ for the lightweight cluster worker

## Build

```bash
git clone https://github.com/phantom-mesh/phantom-mesh
cd phantom-mesh
cargo build
```

## Run

```bash
cargo run -- --host 0.0.0.0 daemon
```

The daemon starts on port 7878 by default. Configuration lives in `~/.phantom-mesh/agents.toml`.

## Test

```bash
cargo test
```

All 3914 tests should pass. If any fail, verify your SQLite installation and environment variables.

Integration tests are in the `tests/` directory and run alongside unit tests.

### Test Guidelines

- Use `tempfile::NamedTempFile` for SQLite in tests (not `:memory:`, which creates a separate DB per connection).
- Mock external HTTP calls; never make real API calls in tests.
- Tests must not write outside `~/.phantom-mesh/` or the system temp directory.

---

## Architecture Overview

```
Telegram Bot API --> phantom-mesh (Rust daemon) --> LLM Providers --> Models
```

### Source Layout

| Directory | Purpose | Count |
|-----------|---------|-------|
| `src/tools/` | Tool implementations (Tool trait) | 42 tools |
| `src/providers/` | LLM provider backends (Provider trait) | 10 providers |
| `src/hands/` | Multi-phase workflow engine | 29 hands |
| `src/cluster_hub.rs` | Hub dispatch to cluster workers | 8-device cluster |
| `src/agent_runtime.rs` | Multi-round tool-calling agent loop | |
| `src/dispatcher.rs` | Native/XML/function-tag tool call parsing | |
| `src/guardrail.rs` | L1 content safety + L2 LLM-as-Judge | |
| `src/security/` | Encryption, RBAC, secrets management | |

### Adding a Tool

1. Create `src/tools/your_tool.rs` implementing the `Tool` trait.
2. Register it in `src/tools/mod.rs` in `ToolRegistry::default()`.
3. Add unit tests in a `#[cfg(test)]` module (at least one success and one error test).
4. Update the tool reference in `README.md`.

### Adding a Hand (Workflow)

Create a TOML file at `~/.phantom-mesh/hands/your_hand/hand.toml`. Key rules:
- Use `system_prompt` (not `instructions`) for phase prompts.
- `[settings]` values must be strings.
- Each phase can override `tools`, `provider`, and `model`.

### Adding a Provider

1. Create `src/providers/your_provider.rs` implementing the `Provider` trait.
2. Register the provider in `src/providers/router.rs`.
3. Update the provider list in `README.md`.

---

## Code Style

### Formatting and Linting

```bash
cargo fmt
cargo clippy -- -D warnings
```

Both must pass before submitting a PR. Configuration is in `rustfmt.toml` and `clippy.toml`.

### Conventions

- Modules: `snake_case`. Types/traits: `PascalCase`. Constants: `SCREAMING_SNAKE_CASE`.
- Tool names in `ToolDef`: lowercase with underscores (e.g., `web_search`).
- Use structured error codes from `src/error_codes.rs`.
- Return `Result<T, anyhow::Error>` from internal functions.
- Avoid `unwrap()` in non-test code; use `?` or explicit error handling.
- No `unsafe` blocks without a detailed justification comment.
- All secrets must use the encrypted secrets system (`src/security/secrets.rs`), never hardcoded.

---

## PR Process

1. **Fork** the repository and create a feature branch:
   ```bash
   git checkout -b feat/your-feature
   ```

2. **Implement** your changes following the code style guidelines.

3. **Verify** before pushing:
   ```bash
   cargo fmt && cargo clippy -- -D warnings && cargo test
   ```

4. **Commit** with a clear message following conventional commits:
   ```
   feat(tools): add your_tool for X purpose
   ```

5. **Open a PR** against the `main` branch. Fill out the PR template describing what changed, why, and how to test it.

6. **Review** -- a maintainer will review within a few business days. Push new commits to address feedback (do not force-push during review).

7. **Merge** -- PRs are merged by a maintainer using squash-merge.

---

## PR Checklist

- [ ] `cargo fmt` applied
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes (all 3914+ tests green)
- [ ] New tests added for new functionality
- [ ] README updated if adding tools, hands, or providers
- [ ] No hardcoded secrets or API keys
- [ ] No new `unwrap()` in non-test code without justification

---

## Questions?

Open an issue or start a discussion on GitHub. We welcome contributions of all sizes.
