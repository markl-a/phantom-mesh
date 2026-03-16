# Ralph Agent Configuration — clawtex-core

## Build Instructions

```bash
# Debug build
cargo build

# Release build (for daemon)
cargo build --release
```

## Test Instructions

```bash
# Run all lib tests
cargo test --lib

# Run integration tests
cargo test --test integration

# Run specific test
cargo test test_name
```

## Run Instructions

```bash
# Start daemon (release mode, background)
cargo run --release

# Daemon runs on http://127.0.0.1:7878
# Telegram bot connects automatically if configured in ~/.clawtex/agents.toml
```

## Key Paths

- Config: `~/.clawtex/agents.toml`
- Workspace: `~/.clawtex/workspace/`
- Hands: `~/.clawtex/hands/<name>/hand.toml`
- Browser helper: `~/.clawtex/browser_helper.py`
- Email helper: `~/.clawtex/email_helper.py`
- Secret key: `~/.clawtex/.secret_key`

## Known Issues

- Windows linker LNK1104: kill stale `clawtex-core.exe` before rebuild
- `Instant::now() - Duration` overflows on Windows — use `checked_sub`
- SQLite `:memory:` creates separate DB per connection — use tempfile for persistence tests
- Release binary often locked by running daemon — kill first or build debug

## Architecture

- Provider trait: `src/providers/` (ollama, openai_compat, anthropic, openai, gemini, groq)
- Tools: `src/tools/` (15 tools total)
- MCP Client: `src/mcp/` (JSON-RPC 2.0 over stdio)
- Hooks: `src/hooks/` (LlmHook, ToolHook, MessageHook)
- Hands: `src/hands/` (TOML workflow engine)
- Security: `src/security/` (ChaCha20-Poly1305 encrypted secrets)
- E-Stop: `src/estop.rs` (AtomicBool emergency stop)
- Gateway: `src/gateway.rs` (SSE + WebSocket)
