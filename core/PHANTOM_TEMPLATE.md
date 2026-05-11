# Project: {PROJECT_NAME}

## Overview

{Brief description of what this project does and what problem it solves.
Include: language/runtime, primary entry points, and the core use-case.}

Example:
> my-service is a Rust HTTP API that processes webhook events from GitHub and
> dispatches them to internal worker queues. It runs as a single binary on
> Linux, configured via a TOML file.

## Build & Test

```bash
# Compile-check (run after every source edit — fast, no linking)
{check command}
# Example: cargo check  |  tsc --noEmit  |  pylint src/

# Full build
{build command}
# Example: cargo build  |  npm run build  |  go build ./...

# Release / production build
{release build command}
# Example: cargo build --release  |  npm run build -- --mode production

# Run all tests
{test command}
# Example: cargo test  |  npm test  |  pytest  |  go test ./...

# Lint / format check
{lint command}
# Example: cargo clippy  |  npm run lint  |  ruff check .
```

**Rule:** Always run `{check command}` after making code changes and confirm
it exits 0 before committing.

## Key Files

```
{entry_point}               — {description, e.g. "main server entry point"}
{config_file}               — {description, e.g. "project config loaded at startup"}
{core_module}               — {description, e.g. "primary business logic"}
{test_directory}/           — {description, e.g. "integration and unit tests"}
{docs_directory}/           — {description, e.g. "architecture and deployment docs"}
```

Example:
```
src/main.rs                 — Axum HTTP server; defines routes in build_router()
src/lib.rs                  — Public API surface, re-exports, shared state
src/config.rs               — Config structs loaded from agents.toml
src/handlers/               — One file per route group
Cargo.toml                  — Package manifest; add dependencies here
tests/                      — Integration tests
docs/                       — Architecture and deployment documentation
```

## Agent Instructions

- Always run `{check command}` after making code changes to catch errors early.
- Read files before editing them — never guess at file contents.
- Use exact-string replacement edits rather than full rewrites when modifying existing files.
- Write tests for new functionality and run `{test command}` before committing.
- Stage specific files for commits — do not use `git commit -am` or `git add .`.
- Use conventional commit messages: `feat:`, `fix:`, `chore:`, `docs:`, `refactor:`.
- Search before assuming — use content search or glob search to locate definitions and usages before editing.
- {project-specific rule 1}
- {project-specific rule 2}

## Architecture

{Describe the high-level structure in 3-8 bullet points or a short diagram.
Focus on the boundaries between components, not implementation details.}

Example:

```
HTTP Layer (handlers/)
    |
    v
Service Layer (services/)      — business logic, no HTTP types
    |
    v
Storage Layer (db/ / store/)   — database, cache, file persistence
```

Key design decisions:
- {Decision 1 and why it was made}
- {Decision 2 and why it was made}
- {Decision 3 and why it was made}

## Configuration

Config file location(s) and format:
```
{path to config file}       — {description}
{path to example/template}  — copy this to get started
```

Important config fields:
- `{field}` — {what it does, required vs optional}
- `{field}` — {what it does, required vs optional}

**Security:** {Note on secrets handling — e.g. "use environment variables for API keys, never commit secrets to config files."}

## Adding New {Components}

When adding a new {tool / route / module / service}:

1. Create `{path}/{name}.{ext}` with the implementation.
2. Register it in `{registration file}` — {specific instructions}.
3. Add tests in `{test location}`.
4. Run `{check command}` and `{test command}` to verify.
5. {Any other required steps}

**Gotcha:** {Common mistake when adding this type of component and how to avoid it.}

## Known Gotchas

- {Gotcha 1}: {explanation and how to avoid or fix it}
- {Gotcha 2}: {explanation and how to avoid or fix it}
- {Gotcha 3}: {explanation and how to avoid or fix it}

Example:
- **State must be Clone:** `AppState` is passed by value to handlers; every field
  must implement `Clone` (wrap mutable state in `Arc<Mutex<_>>`).
- **Both sides of the registry:** When adding a new tool, update both the dispatch
  table AND the schema definitions — missing either causes a silent failure.
- **Env vars for secrets:** Never inline API keys in config files; always use an
  environment variable reference.

## Testing Strategy

- {Unit test location and what they cover}
- {Integration test location and what they cover}
- {How to run a subset of tests for fast iteration}
- {Any manual testing steps required before a PR}

Example:
- Unit tests: `src/**/*_test.{ext}` — pure logic, no I/O
- Integration tests: `tests/` — full stack with real DB (requires `docker compose up`)
- Fast iteration: `{test command} -- {module_name}` to run a single module's tests
- Before PRs: run the full suite and check that no snapshot files changed unexpectedly

## Security Notes

- {Secret handling policy}
- {Auth / access control notes}
- {Any known sensitive areas of the codebase}
- {Guidance on what not to log}
