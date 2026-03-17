# Contributing to Clawtex

Thank you for your interest in contributing to Clawtex! This document covers development setup, code style, how to add tools/hands/providers, and the PR process.

---

## Table of Contents

- [Dev Setup](#dev-setup)
- [Code Style](#code-style)
- [How to Add a Tool](#how-to-add-a-tool)
- [How to Add a Hand](#how-to-add-a-hand)
- [How to Add a Provider](#how-to-add-a-provider)
- [Testing](#testing)
- [PR Process](#pr-process)

---

## Dev Setup

### Prerequisites

- Rust 1.78 or newer
- SQLite 3 (usually pre-installed on Linux/macOS; on Windows install from https://sqlite.org)
- (Optional) Python 3.9+ for the lightweight Python worker

### Clone and Build

```bash
git clone https://github.com/clawtex/clawtex-core
cd clawtex-core
cp .env.example .env
# Fill in at least TELEGRAM_BOT_TOKEN and one LLM API key
cargo build
```

### Run the Daemon in Dev Mode

```bash
RUST_LOG=debug cargo run -- --host 0.0.0.0 daemon
```

### Run Tests

```bash
cargo test
```

All 1594 tests should pass. If any fail, check your environment variables and SQLite installation.

---

## Code Style

Clawtex follows standard Rust formatting and linting conventions.

### Format

Always format before committing:

```bash
cargo fmt
```

The `rustfmt.toml` in the repo root configures formatting rules (max line width, imports style, etc.).

### Lint

Fix all clippy warnings before submitting a PR:

```bash
cargo clippy -- -D warnings
```

The `clippy.toml` in the repo root lists any allowed lints. Do not add new `#[allow(...)]` attributes without a comment explaining why.

### Naming Conventions

- Modules: `snake_case`
- Types and traits: `PascalCase`
- Functions and variables: `snake_case`
- Constants: `SCREAMING_SNAKE_CASE`
- Tool names (in `ToolDef`): lowercase with underscores (e.g., `web_search`, `file_read`)

### Error Handling

- Use structured error codes from `src/error_codes.rs` (E1xx=Provider, E2xx=Tool, E3xx=Cluster, E4xx=Config, E5xx=Agent)
- Return `Result<T, anyhow::Error>` from internal functions
- Avoid `unwrap()` in non-test code; use `?` or explicit error handling

### Safety

- No `unsafe` blocks without a detailed comment justifying necessity
- All secrets must go through the encrypted secrets system (`src/security/secrets.rs`), never hardcoded
- Shell commands must go through the allowlist check in `src/tools/shell.rs`

---

## How to Add a Tool

1. **Create the tool file** in `src/tools/your_tool.rs`

Implement the `Tool` trait:

```rust
use crate::tools::Tool;
use anyhow::Result;
use serde_json::Value;

pub struct YourTool;

#[async_trait::async_trait]
impl Tool for YourTool {
    fn name(&self) -> &str {
        "your_tool"
    }

    fn description(&self) -> &str {
        "Short description of what your tool does."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "The input to process"
                }
            },
            "required": ["input"]
        })
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let input = params["input"].as_str().unwrap_or("");
        // ... implementation ...
        Ok(serde_json::json!({ "result": "..." }))
    }

    // Optional: add preflight checks (called before execute)
    async fn preflight(&self, params: &Value) -> Result<()> {
        // Validate params, check resource availability, etc.
        Ok(())
    }
}
```

2. **Register the tool** in `src/tools/mod.rs` -- add it to the `ToolRegistry::default()` constructor.

3. **Write tests** -- add a `#[cfg(test)]` module at the bottom of your tool file with at least:
   - A success path test
   - An error/invalid-input test

4. **Document** -- add a row to the Tool Reference table in `README.md`.

---

## How to Add a Hand

Hands are TOML files, not Rust code. Create the directory and file:

```
~/.clawtex/hands/your_hand/hand.toml
```

Example structure:

```toml
name = "your_hand"
description = "What this hand does"
provider = "gemini"
model = "gemini-2.0-flash"
tools = ["web_search", "file_write"]

[[phases]]
name = "Research"
system_prompt = """
You are a research assistant. Search for information about {{topic}} and
summarize the key findings.
"""
tools = ["web_search"]

[[phases]]
name = "Write"
system_prompt = """
Using the research from the previous phase, write a comprehensive report.
Save it to the workspace using file_write.
"""
tools = ["file_write"]
```

Key rules:
- Use `system_prompt` (not `instructions`) for phase prompts
- `[settings]` values must be strings (not numbers/booleans)
- Each phase can override the top-level `tools`, `provider`, and `model`
- Condition gates: add `condition = "previous_output contains 'success'"` to a phase to skip it conditionally

After creating the TOML, restart the daemon and the hand will be available immediately.

---

## How to Add a Provider

1. **Create the provider file** in `src/providers/your_provider.rs`

Implement the `Provider` trait from `src/providers/traits.rs`:

```rust
use crate::providers::traits::{ChatMessage, Provider, ProviderResponse};
use anyhow::Result;

pub struct YourProvider {
    api_key: String,
    model: String,
}

#[async_trait::async_trait]
impl Provider for YourProvider {
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<serde_json::Value>>,
    ) -> Result<ProviderResponse> {
        // Call your provider's API and return a ProviderResponse
        todo!()
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn provider_name(&self) -> &str {
        "your_provider"
    }
}
```

2. **Register** the provider type in `src/providers/router.rs` -- add a match arm in the provider factory function.

3. **Document** -- add the provider to the providers list in `README.md`.

---

## Testing

### Unit Tests

Each source file should have a `#[cfg(test)]` module. Run with:

```bash
cargo test
```

### Integration Tests

Integration tests live in `tests/`. They test end-to-end flows including the HTTP API. Run with:

```bash
cargo test --test '*'
```

### Test Guidelines

- Use `tempfile::NamedTempFile` for SQLite databases in tests (not `:memory:` -- each connection gets a separate in-memory DB)
- Mock external HTTP calls using `wiremock` or `httpmock`
- Never make real API calls in tests (use mock providers from `src/providers/mock.rs`)
- Tests must not write outside `~/.clawtex/` or the system temp directory

---

## PR Process

1. **Fork** the repository and create a feature branch:
   ```bash
   git checkout -b feat/your-feature
   ```

2. **Implement** your changes following the code style guidelines above.

3. **Test** -- ensure all existing tests still pass and add new tests for your changes:
   ```bash
   cargo fmt && cargo clippy -- -D warnings && cargo test
   ```

4. **Commit** with a clear message:
   ```
   feat(tools): add your_tool for X purpose

   - Implements Tool trait with preflight validation
   - Adds 8 unit tests covering success and error paths
   - Registered in ToolRegistry
   ```

5. **Open a PR** against the `main` branch. The PR description should include:
   - What the change does and why
   - How to test it
   - Any breaking changes or migration notes

6. **Review** -- a maintainer will review within a few business days. Address feedback by pushing new commits (do not force-push during review).

7. **Merge** -- PRs are merged by a maintainer using squash-merge.

### PR Checklist

- [ ] `cargo fmt` applied
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes (all 1594+ tests green)
- [ ] New tests added for new functionality
- [ ] README updated if new tools/hands/providers added
- [ ] No hardcoded secrets or API keys
- [ ] No new `unwrap()` calls in non-test code without justification

---

## Questions?

Open an issue or start a discussion on GitHub. We welcome questions, bug reports, feature requests, and contributions of all sizes.
