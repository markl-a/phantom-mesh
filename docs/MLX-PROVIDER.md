# MLX Local LLM on Apple Silicon

Run large language models on-device via Apple's MLX framework. Zero
per-token API cost, fully offline once the model is downloaded, fast
enough for autoevolve / subagent flows on M1/M2/M3 Macs.

phantom doesn't bundle MLX or download models — it orchestrates
`mlx_lm.server` (which is OpenAI-compatible) and `huggingface-cli`.

---

## Install once

```bash
pip3 install mlx-lm
```

(Or use `uv tool install mlx-lm` / `pipx install mlx-lm`. Anything
that puts `mlx_lm` on your default `python3` import path works.)

`phantom doctor` will then show:

```
MLX local LLM
  ✓ mlx_lm: importable (`pip install mlx-lm` available)
```

---

## Pull a model

```bash
phantom mlx pull                         # default: Llama 3.1 8B 4-bit (~5 GB)
phantom mlx pull mlx-community/Llama-3.3-70B-Instruct-4bit   # ~38 GB, 32+ GB RAM
phantom mlx pull mlx-community/Qwen2.5-Coder-7B-Instruct-4bit  # ~4 GB, code-focused
```

The default `Llama-3.1-8B-Instruct-4bit` runs cleanly on a 16 GB M1.
70B variants need a 32+ GB Mac to avoid swapping.

---

## Serve

```bash
phantom mlx serve                # foreground, default model + port 8080
phantom mlx serve --port 9090    # custom port
phantom mlx serve --model mlx-community/Llama-3.3-70B-Instruct-4bit
```

`mlx_lm.server` exposes an OpenAI-compatible endpoint at
`http://127.0.0.1:<port>/v1`. Listens on 127.0.0.1 only — exposing it
to the cluster is a follow-up (use a Tailscale-aware reverse proxy or
`mlx_lm.server --host 0.0.0.0` if you trust your tailnet).

While serving, doctor's MLX section turns green:

```
MLX local LLM
  ✓ mlx_lm: importable
  ✓ server: mlx-community/Llama-3.1-8B-Instruct-4bit on :8080 reachable
```

---

## Wire into agents.toml

Append to `~/.phantom-mesh/agents.toml`:

```toml
[providers.mlx-local]
type          = "openai"
base_url      = "http://127.0.0.1:8080/v1"
api_key       = "mlx"
default_model = "mlx-community/Llama-3.1-8B-Instruct-4bit"

[agent.local]
provider     = "mlx-local"
model        = "mlx-community/Llama-3.1-8B-Instruct-4bit"
instructions = "You are phantom-mesh's on-device agent running via MLX. Use tools when needed; respond concisely."
tools        = ["shell", "file_read", "ls", "content_search"]
```

Use it from anywhere:

```bash
# CLI
phantom evolve --agent local "fix the failing tests"

# MCP (Claude Code / Codex)
mcp__phantom__subagent({ agent: "local", prompt: "..." })

# Web mobile UI agent dropdown will list 'local' alongside master/coder.
```

`phantom autoevolve --agent local` runs the hourly self-improvement
loop entirely on-device — autoevolve.log entries from a `local` agent
have `cost: $0.000` after the next cost-pricing update.

---

## Performance notes (M1 16 GB baseline)

| Model | Size | Cold load | Warm gen | Note |
|---|---|---|---|---|
| Llama-3.1-8B-Instruct-4bit | 4.2 GB | ~150 s | ~5-15 s for 50 tokens | default — fast, OK quality |
| Llama-3.3-70B-Instruct-4bit | ~38 GB | swaps badly on 16 GB | n/a | wait for 32+ GB RAM |
| Qwen2.5-Coder-7B-Instruct-4bit | 3.8 GB | ~120 s | ~5 s | better at tool schemas |

Tool-calling quality on 8B 4-bit is hit-and-miss — the small models
sometimes hallucinate phantom's tool schema. For evolve / autoevolve
where tool fidelity matters, prefer **Qwen2.5-Coder-7B** or fall back
to a paid Groq/Anthropic provider. For chat-only flows the 8B Llama
is fine.

---

## Stop / restart

```bash
phantom mlx stop                         # pkill -f mlx_lm.server
phantom mlx status                       # is it up?
phantom mlx serve --model Qwen2.5-Coder…  # swap model
```

The serve command is foreground. To run it as a background daemon,
wrap with launchd (similar to `phantom service install`'s LaunchAgent
template) — not yet shipped as a one-liner.

---

## Why this matters

- **Zero cost** for autoevolve's hourly fix attempts
- **Fully offline** once the model is downloaded — phantom Mac keeps
  fixing and committing on a flight, on a train, in a SCIF
- **Latency** — Apple Silicon's GPU (or NPU on M3+) is faster than the
  network round-trip to api.anthropic.com for prompts that don't need
  Sonnet-class reasoning
- **Privacy** — no token, prompt, or response leaves the machine
- **No competing CLI agent has this** — OpenCode / Codex CLI / Gemini
  CLI all assume an HTTPS round-trip to a vendor's cloud
