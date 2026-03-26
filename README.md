# Phantom Mesh

**Distributed AI Agent Daemon** -- 53 tools, 48 hands, 11 providers, 4-node cluster + mobile workers, self-evolution -- built in Rust.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.78%2B-orange.svg)](https://www.rust-lang.org)
[![Build](https://img.shields.io/badge/build-passing-brightgreen.svg)](.github/workflows)
[![Tests](https://img.shields.io/badge/tests-3914%20passing-brightgreen.svg)](tests/)
[![LOC](https://img.shields.io/badge/LOC-142%2C000%2B-blue.svg)](src/)

---

## What is Phantom Mesh?

Phantom Mesh is a production-grade autonomous agent daemon written in Rust. It receives tasks over Telegram (or HTTP), routes them through 10 LLM providers, executes 53 built-in tools, runs 29 multi-phase workflow automations called **Hands**, and distributes work across an 8-device heterogeneous cluster -- all while tracking costs, enforcing budgets, and continuously self-improving via nightly evolution cycles.

---

## Features

- **53 Tools** -- shell, file I/O, web search, HTTP, browser, vision, image/video/music generation, email send/receive, Twitter, YouTube upload, Stripe, Render deploy, SaaS scaffolding, TTS, PDF/DOCX/XLSX export, knowledge import, calendar, data analysis, screenshot, QR generate, RSS reader, archive extract, clipboard, system info, weather, calculator, notification center, and more
- **48 Hands** -- pre-built multi-phase automation workflows: content, SEO, lead gen, freelancer, researcher, novel, music, game dev, comic, micro-SaaS, trading analysis, customer service, and more
- **11 Providers** -- Ollama, LM Studio, NPU, Gemini, OpenRouter, Groq, Cerebras, Mistral, Codex CLI, ChatGPT, OpenCode CLI
- **4-Node Cluster + Mobile** -- distributed task dispatch across PCs, Macs, and mobile workers; inflight-aware load balancing
- **Self-Evolution** -- nightly DSPy-style prompt optimization, trajectory logging, circuit breaker, watchdog SSH auto-recovery
- **Security** -- ChaCha20-Poly1305 encrypted secrets, injection guard (8 regex patterns), L1 guardrail + L2 LLM-as-Judge, audit log
- **Observability** -- Prometheus metrics endpoint, structured error codes (E1xx-E5xx), auto-diagnosis, cost/revenue tracking
- **Human-in-the-Loop** -- async Telegram approval gates for sensitive tool calls
- **MCP Client** -- JSON-RPC 2.0 over stdio for external tool servers

---

## Architecture

```
                         +------------------+
  Telegram / HTTP -----> |   phantom-mesh   |
                         |   (Rust daemon)  |
                         +--------+---------+
                                  |
              +-------------------+-------------------+
              |                   |                   |
     +--------+-------+  +--------+-------+  +-------+--------+
     |  Tool Registry |  |  Hands Engine  |  |  LLM Router    |
     |  (53 tools)    |  |  (29 workflows)|  |  (10 providers)|
     +--------+-------+  +--------+-------+  +-------+--------+
              |                   |                   |
     +--------+-------------------+-------------------+--------+
     |                    Cluster Hub                          |
     |          (task dispatch + load balancing)               |
     +----+----------+----------+----------+------------------+
          |          |          |          |
       Z13 PC    M1 Mac     Acer PC   AYANEO PC   ... mobile workers
      (Hub+LLM) (Worker)  (Worker)   (NPU Worker)
```

**Data stores** (all SQLite, stored in `~/.phantom-mesh/`):

| Store      | File             | Contents                               |
|------------|------------------|----------------------------------------|
| Sessions   | `core.db`        | conversation history, cron jobs        |
| Costs      | `costs.db`       | per-agent token usage + USD estimates  |
| Memory     | `memory.db`      | key/value agent memories               |
| Revenue    | `revenue.db`     | revenue pipeline records               |
| Knowledge  | `knowledge.db`   | captured facts, decisions, lessons     |
| Trajectory | `trajectories.db`| agent execution traces for evolution   |

---

## Quick Start

### Prerequisites

- Rust 1.78+ (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- A Telegram Bot Token (create via @BotFather on Telegram)
- At least one LLM provider API key (Gemini free tier works out-of-the-box)

### Install

```bash
git clone https://github.com/phantom-mesh/phantom-mesh
cd phantom-mesh
cp .env.example .env
# Edit .env and fill in TELEGRAM_BOT_TOKEN and at least one LLM API key
```

### Configure Agents

Copy the example config:

```bash
mkdir -p ~/.phantom-mesh
cp config/agents.example.toml ~/.phantom-mesh/agents.toml
# Edit ~/.phantom-mesh/agents.toml to set your master agent and providers
```

### Build and Run

```bash
cargo build --release
./target/release/phantom-mesh --host 0.0.0.0 daemon
```

The daemon starts on port **7878**. Open Telegram, message your bot, and it will respond.

### Run Tests

```bash
cargo test
```

---

## Supported Platforms

| Platform | Architecture | Status |
|----------|-------------|--------|
| Windows 11 | x86_64 | Fully supported (primary development platform) |
| macOS | ARM (Apple Silicon) | Fully supported |
| macOS | Intel (x86_64) | Supported |
| Linux | x86_64 / ARM64 | Supported |

All platforms require Rust 1.78+ and a working C linker. Mobile workers (Android/iOS) use the React Native companion app and communicate with the hub via HTTP polling.

---

## Tool Reference

| Name | Category | Description |
|------|----------|-------------|
| `shell` | System | Execute shell commands (allowlist-gated) |
| `file_read` | File I/O | Read files from the workspace sandbox |
| `file_write` | File I/O | Write files to the workspace sandbox |
| `file_edit` | File I/O | Patch files with diff-style edits |
| `glob_search` | Search | Find files matching glob patterns |
| `content_search` | Search | Full-text search across workspace files |
| `web_search` | Web | Multi-engine web search (Serper/Tavily/Brave/Exa) |
| `http_request` | Web | Make arbitrary HTTP requests |
| `browser` | Web | Headless browser automation |
| `vision` | AI | Analyze images with vision models |
| `image_generate` | AI | Generate images via diffusion APIs |
| `video_compose` | Media | Compose and edit video clips |
| `music_generate` | Media | Generate music via AI audio APIs |
| `tts` | Media | Text-to-speech synthesis |
| `youtube_upload` | Media | Upload videos to YouTube |
| `email_send` | Communication | Send email via SMTP |
| `email_receive` | Communication | Fetch and parse incoming email |
| `twitter` | Social | Post tweets, read timeline |
| `discord` | Social | Send Discord messages |
| `slack` | Social | Send Slack messages |
| `whatsapp` | Social | Send WhatsApp messages via API |
| `line_notify` | Social | Send LINE notifications |
| `blog_publish` | Content | Publish posts to Ghost/WordPress |
| `pdf_export` | Export | Export documents to PDF |
| `docx_export` | Export | Export documents to DOCX |
| `xlsx_export` | Export | Export structured data to XLSX |
| `csv_parse` | Data | Parse and transform CSV data |
| `json_transform` | Data | Transform JSON with jq-style queries |
| `summarize` | AI | Summarize long documents |
| `translate` | AI | Translate text between languages |
| `ai_code` | Dev | AI-assisted code generation and review |
| `skeleton_generate` | Dev | Generate project scaffolding |
| `scaffold_saas` | Dev | Scaffold full SaaS application |
| `render_deploy` | Deploy | Deploy to Render.com |
| `stripe` | Business | Stripe payment integration |
| `knowledge_import` | Knowledge | Import and index external knowledge sources |
| `memory_store` | Memory | Persist key/value data across sessions |
| `memory_recall` | Memory | Retrieve stored memories by key/category |
| `memory_forget` | Memory | Delete stored memories |
| `delegate` | Orchestration | Delegate sub-tasks to another agent |
| `delegate_to_provider` | Orchestration | Route a prompt directly to a provider |
| `run_hand` | Orchestration | Trigger a named Hand workflow |
| `computer_use` | System | Control GUI via computer-use API |

---

## Hands (Workflow Automations)

Hands are multi-phase TOML-defined workflows stored in `~/.phantom-mesh/hands/<name>/hand.toml`.

| Name | Phases | Description |
|------|--------|-------------|
| `content` | 4 | Research topic, write article, SEO optimize, publish |
| `seo_content` | 4 | Keyword research, content brief, write, optimize |
| `freelancer` | 4 | Find jobs, apply, draft proposal, follow-up |
| `lead_gen` | 4 | Identify prospects, score leads, enrich data, outreach |
| `researcher` | 3 | Literature review, synthesis, report generation |
| `market_intel` | 3 | Competitive analysis, trend spotting, summary |
| `outreach` | 3 | Contact discovery, message drafting, send campaign |
| `novel` | 5 | World-build, outline, write chapters, edit, format |
| `music` | 4 | Compose lyrics, generate music, mix, publish |
| `comic` | 4 | Script, panel layout, image generation, assemble |
| `game_dev` | 5 | GDD, art prompts, code scaffold, test, package |
| `design` | 3 | Brief, concept generation, export assets |
| `micro_saas` | 5 | Idea validation, spec, scaffold, deploy, market |
| `ecommerce_ops` | 4 | Product listing, pricing, inventory, ads |
| `youtube` | 4 | Script, record/edit, thumbnail, upload |
| `order_workflow` | 3 | Order intake, fulfillment, confirmation |
| `customer_health` | 3 | Collect signals, score health, trigger actions |
| `self_evolve` | 4 | Review agents, analyze, implement improvements, document |
| `review_agents` | 2 | Collect performance data, identify weakest agents |
| `prompt_evolve` | 3 | Sample trajectories, optimize prompts, deploy |
| `cluster_evolve` | 4 | Analyze cluster, dispatch tests, collect results, apply |
| `auto_diagnosis` | 3 | Detect anomalies, root cause analysis, remediation |
| `ops_report` | 3 | Gather metrics, summarize, send report |
| `report` | 2 | Collect data, generate report |
| `trading_analysis` | 3 | Fetch prices, technical analysis, signal generation |
| `product_spec` | 3 | Requirements gathering, spec writing, review |
| `code_gen` | 4 | Design, implement, test, document |
| `saas_deploy` | 3 | Build, deploy, configure billing |
| `customer_service` | 3 | Intake ticket, resolve, follow-up |

Trigger a hand via HTTP:
```bash
curl -X POST http://localhost:7878/hand/content/run \
  -H 'Content-Type: application/json' \
  -d '{"prompt": "Write an article about Rust async programming"}'
```

---

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Health check |
| GET | `/metrics` | Prometheus metrics |
| GET | `/metrics/health` | JSON health summary |
| POST | `/message` | Send a message to the master agent |
| GET | `/agents` | List configured agents |
| GET | `/tools` | List available tools |
| GET | `/hands` | List available hands |
| POST | `/hand/:name/run` | Run a named hand |
| GET | `/sessions` | List active sessions |
| GET | `/costs` | Cost summary |
| GET | `/cluster/workers` | List cluster workers |
| POST | `/cluster/register` | Register a new worker |
| GET | `/cluster/poll` | Worker polling endpoint |
| POST | `/cluster/result` | Submit task result |
| GET | `/trajectories` | List recorded trajectories |
| GET | `/trajectories/stats` | Trajectory statistics |
| GET | `/cluster/health` | Cluster health overview |

---

## Cluster Setup

Phantom Mesh supports distributing work across multiple machines. Any device that can run the Rust binary (or the lightweight Python worker) can join the cluster.

### Add a PC Worker

```bash
# On the worker machine -- build and start in worker mode
cargo build --release
./target/release/phantom-mesh worker --hub http://<hub-ip>:7878 --name my-worker

# Register with the hub (if auto-registration fails)
curl -X POST http://<hub-ip>:7878/cluster/register \
  -H 'Content-Type: application/json' \
  -d '{"name": "my-worker", "address": "http://<worker-ip>:7879", "device_type": "pc"}'
```

### Add a Mobile Worker

Mobile devices use a polling model (no inbound HTTP server needed):

1. Install the Phantom Mesh Worker app (React Native / Expo)
2. Set Hub URL and worker name in the app settings
3. The app polls `GET /cluster/poll?worker=<name>` every 15 seconds

### Python Lightweight Worker

For minimal-resource devices, use the Python worker script in `deploy/worker.py`:

```bash
python3 deploy/worker.py --hub http://<hub-ip>:7878 --name my-light-worker
```

---

## Configuration Reference

`~/.phantom-mesh/agents.toml` controls agents, providers, and routing.

```toml
[master]
name = "phantom-mesh"
provider = "gemini"
model = "gemini-2.0-flash"

[[providers]]
name = "gemini"
type = "gemini"
api_key_env = "GEMINI_API_KEY"

[[providers]]
name = "ollama"
type = "ollama"
base_url = "http://localhost:11434"
model = "llama3.2:1b"

[routing]
simple = ["ollama"]
medium = ["gemini"]
complex = ["gemini"]
```

---

## Security

- All secrets stored encrypted at rest (ChaCha20-Poly1305, `enc2:` prefix)
- Injection guard blocks 8 categories of prompt injection attacks
- Shell tool gated by allowlist -- only pre-approved commands can run
- Rate limiting: 840 actions/hour global, 280/hour per tool
- L1 rule-based guardrail + L2 LLM-as-Judge quality gate on every Hand phase
- Sensitive operations require human Telegram approval before execution

---

## License

MIT License. See [LICENSE](LICENSE) for details.

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, code style, and PR process.

## Security Reporting

See [SECURITY.md](SECURITY.md) for vulnerability reporting.
