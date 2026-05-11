# Contributing to Phantom Mesh

Thank you for your interest in contributing to Phantom Mesh!

Before making architecture or product-shaping changes, read:

- `AGENTS.md`
- `docs/ARCHITECTURE-FREEZE.md`
- `docs/ACTIVE-STATUS.md`

## Development Setup

### Prerequisites

- **Rust stable toolchain** (install via [rustup](https://rustup.rs/))
- **SQLite** (bundled with the Rust crate, no external install needed)
- **Optional**: [Ollama](https://ollama.ai) for local LLM testing

### Building

```bash
cd core
cargo build
```

### Running Tests

```bash
cd core
cargo test --lib          # Unit tests (~2,700 tests, ~55s)
cargo test --lib -q       # Quiet mode
cargo test --lib "module" # Run tests for a specific module
```

### Running the Daemon

```bash
cd core
cargo run -- health              # Check system health
cargo run -- init                # First-time setup
cargo run -- daemon              # Start daemon on port 7878
cargo run -- run "hello world"   # Single prompt execution
```

## Project Structure

```
phantom-mesh/
├── core/                  # Rust daemon (main codebase)
│   ├── src/
│   │   ├── lib.rs         # Library entry point
│   │   ├── main.rs        # CLI wrapper
│   │   ├── runtime.rs     # PhantomMesh::init() API
│   │   ├── agent_runtime.rs  # Multi-round tool-calling loop
│   │   ├── events/        # DomainEvent spine + persistence
│   │   ├── providers/     # LLM providers (Ollama, Claude, OpenAI, etc.)
│   │   ├── tools/         # Agent tools (shell, file, web search, etc.)
│   │   └── ...
│   └── tests/             # Integration tests
├── crates/
│   └── pm-types/          # Shared type definitions
├── app/                   # Tauri v2 desktop app (React)
└── .github/workflows/     # CI pipelines
```

## Code Style

- Follow standard Rust conventions (`cargo clippy` must pass)
- Unit tests go in the same file under `#[cfg(test)] mod tests`
- Integration tests go in `core/tests/`
- All public enums use `#[non_exhaustive]`

## Pull Request Process

1. Fork the repository
2. Create a feature branch from `main`
3. Make your changes with tests
4. Ensure `cargo test --lib` passes
5. Ensure `cargo clippy -- -D warnings` passes
6. Submit a pull request with a clear description

## License

By contributing, you agree that your contributions will be licensed under the MIT OR Apache-2.0 license.
