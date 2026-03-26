# SaaS Product Pipeline Design

## Date: 2026-03-05

## Goal
Enable phantom-mesh agents to autonomously: research market → generate API service code → deploy to Render → integrate Stripe payments → produce a live, paying SaaS product.

## User Choices
- Product type: SaaS API services
- Deployment: Render
- Payment: Stripe

## Architecture

```
/product <idea>
    ↓
market_intel hand (existing) → opportunities.json
    ↓
product_spec hand (NEW) → openapi.yaml + pricing.json + architecture.md
    ↓
code_gen hand (NEW) → complete Node.js project (src/ + tests/ + Dockerfile + package.json)
    ↓
saas_deploy hand (NEW) → live URL on Render + Stripe products/prices + checkout link
    ↓
Revenue auto-tracked via Stripe webhook → revenue_tracker
```

## New Components

### Tool 1: `stripe` (src/tools/stripe.rs)
- Shell-based: calls `stripe` CLI or `curl` to Stripe REST API
- Actions:
  - `create_product` → product_id
  - `create_price` → price_id (supports recurring + one-time + metered)
  - `create_checkout_link` → checkout URL
  - `list_customers` → customer list
  - `get_balance` → current balance
- Auth: STRIPE_SECRET_KEY env var
- No external crate needed (HTTP via reqwest, already in deps)

### Tool 2: `render_deploy` (src/tools/render_deploy.rs)
- Calls Render REST API (api.render.com)
- Actions:
  - `create_service` → service_id (from GitHub repo or Docker image)
  - `deploy` → trigger deploy
  - `get_status` → service status + URL
  - `set_env` → set environment variables
  - `delete_service` → teardown
- Auth: RENDER_API_KEY env var

### Hand 1: `product_spec` (~/.phantom-mesh/hands/product_spec/hand.toml)
- Phase 1: `market_analysis` — recall market_intel output, identify top opportunity
- Phase 2: `api_design` — generate OpenAPI 3.0 spec (endpoints, models, auth)
- Phase 3: `pricing_strategy` — define Stripe pricing (free tier, paid tiers, metered)
- Phase 4: `spec_output` — write spec files to workspace
- Tools: web_search, file_write, file_read, memory_store, memory_recall
- Output: openapi.yaml, pricing.json, architecture.md

### Hand 2: `code_gen` (~/.phantom-mesh/hands/code_gen/hand.toml)
- Phase 1: `scaffold` — generate project structure (package.json, tsconfig, Dockerfile)
- Phase 2: `implement` — generate API endpoints, middleware, DB schema from OpenAPI spec
- Phase 3: `auth_and_billing` — add Stripe integration (API key auth, usage tracking, webhook handler)
- Phase 4: `testing` — generate test suite, run tests via shell
- Phase 5: `package` — final review, README, git init + commit
- Tools: file_write, file_read, file_edit, shell, ai_code, memory_recall
- Output: complete project directory in workspace

### Hand 3: `saas_deploy` (~/.phantom-mesh/hands/saas_deploy/hand.toml)
- Phase 1: `github_push` — create GitHub repo, push code
- Phase 2: `render_deploy` — create Render service, set env vars, deploy
- Phase 3: `stripe_setup` — create Stripe product + prices, generate checkout link
- Phase 4: `verify` — hit API health endpoint, verify Stripe webhook
- Phase 5: `announce` — publish blog post + tweet about new product
- Tools: shell, render_deploy, stripe, file_write, http_request, blog_publish, twitter, memory_store
- Output: live URL, checkout link, blog post

### Main.rs: `/product <idea>` command
- Triggers pipeline: market_intel → product_spec → code_gen → saas_deploy
- Shows progress per phase via Telegram
- Stores result in memory (product_{name})

## Implementation Order
1. stripe tool
2. render_deploy tool
3. product_spec hand
4. code_gen hand
5. saas_deploy hand
6. /product command + module registration
7. agents.toml update
8. tests
