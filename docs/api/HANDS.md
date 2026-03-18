# Clawtex Hands Reference

Hands are multi-phase agentic workflows defined as TOML files under `~/.clawtex/hands/<name>/hand.toml`.
Each hand runs a sequence of phases; the output of each phase is passed as context to the next.

Use `GET /hands` for the live list, or `POST /hand/<name>/run` with `{"prompt": "..."}` to execute.

Total: **29 hands** across 7 categories.

---

## Category: Marketing

### content
**Description**: Content creation agent — generate social media posts, articles, and marketing copy.
**Provider**: gemini (gemini-2.5-flash)
**Phases**: 4
- `topic_research` — web search for trending angles and viral patterns
- `content_generation` — generate multiple variants (tweets/articles/emails) with brand voice
- `quality_review` — fact-check, grammar, engagement scoring
- `publish_and_promote` — post to Twitter and blog

**Tools**: web_search, browser, file_write, file_read, memory_store, memory_recall, ai_code, twitter, blog_publish

**Cron**: Daily 8:00 AM

```bash
curl -X POST http://localhost:7878/hand/content/run \
  -H "Authorization: Bearer your-hub-token-here" \
  -d '{"prompt": "Write 5 tweets about distributed AI systems in 2026"}'
```

---

### seo_content
**Description**: SEO-optimized content pipeline — keyword research, article writing, publishing.
**Provider**: gemini
**Phases**: 4
**Tools**: web_search, browser, file_write, file_read, memory_store, memory_recall, blog_publish

**Cron**: Tue+Thu 11:00 AM

```bash
curl -X POST http://localhost:7878/hand/seo_content/run \
  -d '{"prompt": "Write SEO article about Rust vs Go for backend development"}'
```

---

### youtube
**Description**: YouTube content creation pipeline — research, script, thumbnail, SEO, description, checklist.
**Provider**: gemini (gemini-2.5-flash)
**Phases**: 4+
**Tools**: web_search, file_write, file_read, memory_store, memory_recall

```bash
curl -X POST http://localhost:7878/hand/youtube/run \
  -d '{"prompt": "Create a YouTube video about building AI agents in Rust"}'
```

---

### outreach
**Description**: Sales outreach automation — find leads, craft personalized messages, track pipeline.
**Provider**: gemini
**Phases**: 4
**Tools**: web_search, browser, email_send, memory_store, memory_recall, file_write

**Cron**: Mon/Wed/Fri 3:00 PM

```bash
curl -X POST http://localhost:7878/hand/outreach/run \
  -d '{"prompt": "Reach out to 5 SaaS founders about AI automation services"}'
```

---

## Category: Sales / Revenue

### freelancer
**Description**: Freelance job finder — search job boards, score opportunities, generate proposals.
**Provider**: gemini (gemini-2.5-flash)
**Phases**: 3+
- `job_search` — search Upwork/Freelancer/Toptal, deduplicate against applied history
- `job_scoring` — score each opportunity (fit, budget, competition)
- `proposal_generation` — write tailored proposals for top opportunities

**Tools**: web_search, browser, file_write, file_read, memory_store, memory_recall, ai_code

**Cron**: Daily 9:00 AM

```bash
curl -X POST http://localhost:7878/hand/freelancer/run \
  -d '{"prompt": "AI automation and Rust development jobs, minimum $1000 budget"}'
```

---

### lead
**Description**: Lead generation — find qualified prospects, score them, add to CRM pipeline.
**Provider**: gemini (gemini-2.5-flash)
**Phases**: 4
**Tools**: web_search, browser, file_write, file_read, memory_store, memory_recall, ai_code

**Cron**: Monday 10:00 AM

```bash
curl -X POST http://localhost:7878/hand/lead/run \
  -d '{"prompt": "SaaS companies needing AI automation in healthcare and fintech"}'
```

---

### market_intel
**Description**: Market intelligence — competitor analysis, pricing research, opportunity identification.
**Provider**: gemini (gemini-2.5-flash)
**Category**: research
**Phases**: 4
**Settings**: `focus_area` (competitors/pricing/trends), `depth` (quick/standard/deep)
**Tools**: web_search, browser, file_write, file_read, memory_store, memory_recall, ai_code

**Cron**: Wednesday 9:00 AM

```bash
curl -X POST http://localhost:7878/hand/market_intel/run \
  -d '{"prompt": "Analyze competitor pricing for AI coding assistants (Cursor, Copilot, Codeium)"}'
```

---

## Category: Research

### researcher
**Description**: Deep research agent — exhaustive investigation with source evaluation and structured reports.
**Provider**: gemini (gemini-2.5-flash)
**Phases**: 4
**Tools**: web_search, browser, file_write, memory_store, memory_recall, ai_code

**Cron**: Daily 2:00 PM

```bash
curl -X POST http://localhost:7878/hand/researcher/run \
  -d '{"prompt": "Research the current state of open-source LLM inference optimization"}'
```

---

### report
**Description**: Research report generator — collect data, analyze, plan charts, export final report.
**Provider**: gemini (gemini-2.5-flash)
**Phases**: 4
**Tools**: web_search, http_request, file_write, file_read, memory_store, memory_recall

```bash
curl -X POST http://localhost:7878/hand/report/run \
  -d '{"prompt": "Generate Q1 2026 AI tooling market report"}'
```

---

### trading_analysis
**Description**: Crypto and stock market analysis — trend detection, sentiment analysis, trading signal generation (human review only, no auto-execution).
**Provider**: auto
**Category**: finance
**Phases**: 4
**Tools**: web_search, http_request, file_write, file_read, memory_store, memory_recall

```bash
curl -X POST http://localhost:7878/hand/trading_analysis/run \
  -d '{"prompt": "Analyze BTC/USDT trend and generate trading signals for the next 24h"}'
```

---

## Category: Product

### product_spec
**Description**: Generate SaaS product specification from market research — creates OpenAPI spec, pricing strategy, and technical architecture.
**Category**: product
**Chain**: Automatically chains to `code_gen` on completion
**Phases**: 3+
**Tools**: web_search, browser, file_write, file_read, memory_store, memory_recall, ai_code

```bash
curl -X POST http://localhost:7878/hand/product_spec/run \
  -d '{"prompt": "AI-powered resume screening API for HR teams"}'
# Automatically chains to code_gen
```

---

### code_gen
**Description**: Generate complete SaaS API service code from product specification — produces deployable Node.js project with Stripe billing, tests, and Docker.
**Category**: product
**Chain**: Automatically chains to `saas_deploy` on completion
**Phases**: 3+
**Tools**: scaffold_saas, file_write, file_read, file_edit, shell, ai_code, memory_recall, memory_store

```bash
curl -X POST http://localhost:7878/hand/code_gen/run \
  -d '{"prompt": "Build from the product_spec in workspace/product_spec.md"}'
# Automatically chains to saas_deploy
```

---

### saas_deploy
**Description**: Deploy SaaS API to Render + set up Stripe billing + announce — takes a built project and makes it live with payment collection.
**Category**: product
**Phases**: 3+
**Tools**: shell, render_deploy, stripe, file_write, file_read, http_request, blog_publish, twitter, memory_store, memory_recall

```bash
curl -X POST http://localhost:7878/hand/saas_deploy/run \
  -d '{"prompt": "Deploy the project in workspace/my-saas/ to production"}'
```

---

### micro_saas
**Description**: Micro SaaS creation pipeline — market validation, MVP spec, code generation, deployment, launch.
**Provider**: gemini
**Category**: business
**Phases**: 5
**Tools**: web_search, http_request, ai_code, file_write, file_read, scaffold_saas, render_deploy, stripe, memory_store

```bash
curl -X POST http://localhost:7878/hand/micro_saas/run \
  -d '{"prompt": "Build and launch a minimal URL shortener with click analytics ($5/month tier)"}'
```

---

## Category: Creative

### novel
**Description**: Novel generation pipeline — worldbuilding, characters, outline, chapter writing, editing, PDF export.
**Provider**: gemini (gemini-2.5-flash)
**Category**: content
**Phases**: 5
**Tools**: web_search, file_write, file_read, summarize, pdf_export, memory_store, memory_recall

```bash
curl -X POST http://localhost:7878/hand/novel/run \
  -d '{"prompt": "Write a sci-fi thriller about AI systems that develop emergent consciousness in 2040"}'
```

---

### design
**Description**: Design and artwork pipeline — brief analysis, concept generation, AI self-evaluation, final delivery.
**Provider**: gemini (gemini-2.5-flash)
**Category**: creative
**Phases**: 4
**Tools**: web_search, image_generate, vision, file_write, file_read, memory_store

```bash
curl -X POST http://localhost:7878/hand/design/run \
  -d '{"prompt": "Design a modern logo for a distributed AI cluster platform called Clawtex"}'
```

---

### comic
**Description**: Comic creation pipeline — script, storyboard, panel generation, layout, PDF export.
**Provider**: gemini (gemini-2.5-flash)
**Category**: creative
**Phases**: 4
**Tools**: web_search, image_generate, vision, file_write, file_read, pdf_export, memory_store

```bash
curl -X POST http://localhost:7878/hand/comic/run \
  -d '{"prompt": "Create a 4-page comic about a robot learning to write code"}'
```

---

### music
**Description**: Music creation pipeline — concept research, lyrics writing, AI generation, publishing preparation.
**Provider**: gemini (gemini-2.5-flash)
**Category**: creative
**Phases**: 4
**Tools**: web_search, music_generate, file_write, file_read, translate, memory_store

```bash
curl -X POST http://localhost:7878/hand/music/run \
  -d '{"prompt": "Create an upbeat lo-fi track for a productivity app promotional video"}'
```

---

## Category: Business / Operations

### ecommerce_ops
**Description**: E-commerce operations — market research, listing optimization, competitor analysis, operations report.
**Provider**: gemini (gemini-2.5-flash)
**Category**: business
**Phases**: 4
**Tools**: web_search, http_request, csv_parse, file_write, file_read, translate, summarize, memory_store

```bash
curl -X POST http://localhost:7878/hand/ecommerce_ops/run \
  -d '{"prompt": "Optimize my Amazon listings for ergonomic desk accessories, analyze top 10 competitors"}'
```

---

### game_dev
**Description**: Game development pipeline — concept, design document, code generation, asset creation, packaging.
**Provider**: gemini (gemini-2.5-flash)
**Category**: development
**Phases**: 5
**Tools**: web_search, ai_code, file_write, file_read, image_generate, shell, memory_store

```bash
curl -X POST http://localhost:7878/hand/game_dev/run \
  -d '{"prompt": "Build a simple Rust terminal game - Snake with leaderboard"}'
```

---

### customer_service
**Description**: Automated customer service agent — classifies intent, searches knowledge base, generates professional responses.
**Provider**: auto
**Category**: service
**Phases**: 3
**Tools**: memory_recall, memory_store, file_read, web_search

```bash
curl -X POST http://localhost:7878/hand/customer_service/run \
  -d '{"prompt": "Customer message: I cannot access my account, getting 403 error"}'
```

---

### auto_report
**Description**: Automated report generator — collect data, analyze, generate professional reports for B2B clients.
**Provider**: auto
**Category**: service
**Phases**: 4
**Tools**: web_search, browser, file_write, file_read, memory_store, memory_recall, email_send, ai_code, pdf_export

```bash
curl -X POST http://localhost:7878/hand/auto_report/run \
  -d '{"prompt": "Generate monthly performance report for client Acme Corp (SaaS metrics)"}'
```

---

## Category: Infrastructure / DevOps

### self_evolve
**Description**: Nightly self-evolution — review today's conversations and costs, find one improvement, store the lesson in memory for future recall. Felix-style 1% compound nightly improvement.
**Provider**: lmstudio
**Category**: infrastructure
**Phases**: 4
**Cron**: Daily 2:00 AM

```bash
curl -X POST http://localhost:7878/hand/self_evolve/run \
  -d '{"prompt": "Review today and find one improvement"}'
```

---

### review_agents
**Description**: Nightly sub-agent review — analyze hand execution performance, identify weakest performer, suggest improvements.
**Provider**: lmstudio
**Category**: infrastructure
**Phases**: 2
**Cron**: Daily 1:00 AM (runs before self_evolve)

```bash
curl -X POST http://localhost:7878/hand/review_agents/run \
  -d '{"prompt": "Review all hands from today and identify the weakest"}'
```

---

### self_optimize
**Description**: Self-optimize cluster configuration and code based on health data.
**Provider**: auto
**Category**: infrastructure
**Phases**: 3
**Tools**: shell, file_read, file_edit, file_write, http_request, memory_store, memory_recall, glob_search, content_search
**Cron**: Sunday 3:00 AM

```bash
curl -X POST http://localhost:7878/hand/self_optimize/run \
  -d '{"prompt": "Optimize cluster configuration based on last week health data"}'
```

---

### cluster_evolve
**Description**: Distributed self-evolution across all cluster devices — analyze metrics, dispatch AI-driven improvements, integrate, test, and deploy.
**Provider**: auto
**Category**: automation
**Phases**: 4
**Tools**: shell, file_read, file_write, file_edit, http_request, memory_store, memory_recall, delegate_to_provider, web_search
**Cron**: Daily 3:00 AM

```bash
curl -X POST http://localhost:7878/hand/cluster_evolve/run \
  -d '{"prompt": "Run weekly cluster evolution cycle"}'
```

---

### cluster_health
**Description**: Monitor cluster health — check all nodes, analyze bottlenecks, report status.
**Provider**: auto
**Category**: infrastructure
**Phases**: 3
**Tools**: shell, http_request, memory_store, memory_recall
**Cron**: Every 4 hours

```bash
curl -X POST http://localhost:7878/hand/cluster_health/run \
  -d '{"prompt": "Check all cluster nodes and report issues"}'
```

---

### prompt_evolve
**Description**: Weekly prompt evolution — analyze trajectory data to find low-scoring phases, generate improved system_prompt variants, and apply the best ones with safe backups.
**Category**: infrastructure
**Phases**: 4
**Cron**: Sunday 4:00 AM

```bash
curl -X POST http://localhost:7878/hand/prompt_evolve/run \
  -d '{"prompt": "Evolve prompts for the 3 lowest-scoring hands this week"}'
```

---

### build_mobile
**Description**: Build Clawtex Worker mobile app — Android on Acer, iOS on M1 Mac. Tests cluster targeted dispatch.
**Provider**: auto
**Category**: devops
**Phases**: 3
**Tools**: shell, file_read, file_write, http_request

```bash
curl -X POST http://localhost:7878/hand/build_mobile/run \
  -d '{"prompt": "Build Android APK and iOS IPA for clawtex worker app"}'
```

---

## Hand Chaining

Some hands automatically chain to the next hand on successful completion:

| Hand | Chains To |
|------|-----------|
| product_spec | code_gen |
| code_gen | saas_deploy |

The `/product` Telegram command also manually chains: `product_spec -> code_gen -> saas_deploy`

When chaining occurs, the HTTP API response includes a `chained_hand` field with the results of the next hand.

---

## Cron Schedule Summary

| Hand | Schedule | Description |
|------|----------|-------------|
| content | Daily 8 AM | Daily content creation |
| freelancer | Daily 9 AM | Freelance job search |
| lead | Monday 10 AM | Weekly lead generation |
| seo_content | Tue + Thu 11 AM | SEO content publishing |
| market_intel | Wednesday 9 AM | Market intelligence |
| researcher | Daily 2 PM | Research tasks |
| outreach | Mon/Wed/Fri 3 PM | Sales outreach |
| review_agents | Daily 1 AM | Sub-agent performance review |
| self_evolve | Daily 2 AM | Nightly self-improvement |
| cluster_evolve | Daily 3 AM | Distributed evolution |
| self_optimize | Sunday 3 AM | Infrastructure optimization |
| prompt_evolve | Sunday 4 AM | Prompt evolution |
| cluster_health | Every 4 hours | Cluster monitoring |

---

## TOML Hand Format

```toml
name = "my_hand"
description = "What this hand does"
category = "marketing"
provider = "gemini"              # optional, overrides master agent
model = "gemini-2.5-flash"      # optional
output_format = "markdown"      # optional
tools = ["web_search", "file_write"]
chain_to = "next_hand"          # optional: auto-chain on completion
schedule = "0 9 * * *"         # optional: cron expression

[settings]                      # optional: hand-specific config
key = "value"                   # all values must be strings

[[phases]]
name = "phase_one"
system_prompt = """Your agent instructions here."""
max_rounds = 5                  # max LLM iterations per phase
parallel_queries = [            # optional: pre-fetch via batch dispatch
    "search query 1",
    "search query 2",
]
condition = "{{prev_output}} contains 'success'"  # optional gate

[[phases]]
name = "phase_two"
system_prompt = """Next phase instructions..."""
max_rounds = 3
```
