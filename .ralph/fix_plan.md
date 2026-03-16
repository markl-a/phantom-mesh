# Clawtex Money-Making Features — Implementation Plan

> Updated: 2026-03-04
> Goal: All income routes functional with E2E tests proving real revenue capability

## Current State (Post Stage 5)
- 17 tools registered (incl. twitter, blog_publish, email_send, vision)
- 7 hands (lead, outreach, researcher, content, seo_content, freelancer, market_intel)
- SMTP configured ✅, Gemini API ✅, Twitter OAuth ✅, Blog repo ✅
- 321 lib tests passing

---

## Tasks (ordered by priority — implement one per loop)

### 🔴 BATCH 1: Pipeline Chaining + Cron (directly enables revenue automation)

#### Task 1: Hand Chaining — Lead → Outreach pipeline [B10]
- **What**: After lead hand completes, automatically trigger outreach hand with the scored leads
- **Where**: `src/hands/mod.rs` — add `chain_to` field to Hand struct + HandRunner::run() logic
- **How**:
  1. Add `chain_to = "outreach"` optional field in hand.toml
  2. HandRunner checks after completion, if chain_to exists, start next hand with output as input
  3. Add to lead hand.toml: `chain_to = "outreach"`
- **Test**: E2E test that runs lead hand and verifies outreach hand auto-starts
- **Status**: DONE

#### Task 2: Cron → Hands Integration [X5]
- **What**: Schedule hands to run automatically via cron
- **Where**: `src/cron.rs` + `src/main.rs`
- **How**:
  1. Add `JobAction::Hand { hand_name, input }` variant to cron.rs
  2. In main.rs cron executor, handle Hand action by calling HandRunner
  3. Add cron jobs via Telegram: `/cron add "0 9 * * *" hand:freelancer "AI automation jobs"`
  4. Add Telegram command `/cron list` and `/cron remove <id>`
- **Test**: Unit test for Hand JobAction, test cron parsing with hand action
- **Status**: DONE

#### Task 3: SEO Content → Blog → Twitter Pipeline [D→E chain]
- **What**: Chain seo_content hand output → blog_publish → twitter post
- **Where**: seo_content hand.toml phases + new "publish" phase
- **How**:
  1. Add 5th phase "publish_and_promote" to seo_content hand
  2. System prompt: read article_final.md → call blog_publish tool → call twitter tool with teaser
  3. Tools: ["file_read", "blog_publish", "twitter"]
- **Test**: E2E test that verifies seo_content produces article + calls blog_publish + tweets
- **Status**: DONE

#### Task 4: Follow-up Email Scheduling [B7]
- **What**: Auto-send follow-up emails (Day 3 Email 2, Day 8 Email 3)
- **Where**: outreach hand phase 4 + cron
- **How**:
  1. Outreach Phase 4 stores follow-up schedule in memory_store
  2. New cron job: daily check memory for follow_up_due
  3. Auto-trigger outreach hand "follow-up" mode
- **Test**: memory_store stores correct follow-up dates
- **Status**: DONE

### 🟡 BATCH 2: Tracking + Quality (ROI visibility)

#### Task 5: Cost Tracking [X3, I1]
- **What**: Track token usage and estimated cost per agent run
- **Where**: New `src/cost_tracker.rs` + agent_runtime.rs
- **How**:
  1. Create CostTracker struct with SQLite persistence
  2. After each LLM call in agent_runtime, record: agent, provider, model, tokens_in, tokens_out, cost_estimate
  3. Add GET /costs endpoint (total by day/agent/provider)
  4. Add /costs Telegram command
- **Test**: CostTracker record + query tests
- **Status**: DONE

#### Task 6: Application Status Tracking [A7]
- **What**: Track freelance application statuses via memory
- **Where**: freelancer hand.toml Phase 4
- **How**:
  1. Add memory_store calls in Phase 4 system prompt
  2. Key: "freelance_{company}_{date}", Value: status + proposal
  3. Add memory_recall in Phase 1 to check past applications (avoid duplicates)
- **Test**: Memory operations in freelancer hand
- **Status**: DONE

#### Task 7: Approval Gate for Proposals [A5]
- **What**: Freelancer hand pauses for human review before marking proposals as "ready"
- **Where**: freelancer hand.toml
- **How**:
  1. Add phase 5 "human_review" that uses approval gate
  2. System prompt sends proposals to Telegram, waits for /approve
- **Test**: Approval flow test
- **Status**: DONE

#### Task 8: CRM Status Management [B9]
- **What**: Track outreach pipeline status (sent/opened/replied/closed)
- **Where**: outreach hand Phase 4 + memory
- **How**:
  1. Phase 4 stores: memory_store("outreach_{company}", status + sent_date + next_action)
  2. Add /crm Telegram command to list all outreach statuses
- **Test**: CRM memory operations
- **Status**: DONE

### 🟢 BATCH 3: New Hands + Export

#### Task 9: auto_report Hand [C1]
- **What**: B2B subscription service — auto-generate reports from data sources
- **Where**: New `~/.clawtex/hands/auto_report/hand.toml`
- **How**: 4 phases: data_collection → analysis → report_generation → distribution
- **Test**: E2E with mock data
- **Status**: DONE

#### Task 10: PDF Export [H4]
- **What**: Convert Markdown reports to professional PDF
- **Where**: New tool `src/tools/pdf_export.rs` using pandoc/weasyprint
- **Test**: Generate PDF from markdown
- **Status**: DONE

#### Task 11: Brand Voice Profile [E9]
- **What**: Store brand voice definition in memory for consistent content
- **Where**: memory_store + content hand Phase 2
- **Test**: Content matches stored brand voice
- **Status**: DONE

### 🔵 BATCH 4: E2E Integration Tests + Route Coverage

#### Task 12-15: E2E Tests — real file output + full pipeline verification
- **Status**: DONE (11 real E2E tests + 40 structural)

#### Task 16: Revenue Tracker [X4]
- **What**: Track income by route (A-J), source, client, status
- **Where**: `src/revenue_tracker.rs` + main.rs + /revenue Telegram + GET /revenue
- **Status**: DONE

#### Task 17: Cost Tracking wired into AgentRuntime [I1]
- **What**: Automatic cost recording after every LLM call
- **Where**: agent_runtime.rs — set_cost_tracker() + record_run_cost()
- **Status**: DONE

#### Task 18: customer_service Hand [C2]
- **What**: 4-phase customer support hand (intent → knowledge → response → QA)
- **Where**: `~/.clawtex/hands/customer_service/hand.toml`
- **Status**: DONE

#### Task 19: trading_analysis Hand [J1]
- **What**: 4-phase trading research hand (market → TA → sentiment → signals)
- **Where**: `~/.clawtex/hands/trading_analysis/hand.toml`
- **Status**: DONE

#### Task 20: Fix conversation test flakiness
- **What**: Tests used fixed temp dirs causing stale data between runs
- **Where**: `src/conversation.rs` — add remove_dir_all before create_dir_all
- **Status**: DONE (333 lib tests now all pass)

---

## Completed
- ✅ X1: SMTP configured (Gmail app password)
- ✅ X7: Vision API (Gemini 2.5 flash-lite key set)
- ✅ E4: Twitter tool (API + Playwright browser fallback)
- ✅ D6: Blog publish tool (MDX + index.ts + git push → Vercel)
- ✅ D7: Blog exists (markl-ai.space, Next.js + Vercel)
- ✅ Stage 1-4: All 7 hands, 17 tools, 321 tests
- ✅ Task 1: Hand Chaining — chain_to field + auto-trigger in Telegram & HTTP
- ✅ Task 2: Cron → Hands — JobAction::Hand variant + executor + /cron Telegram commands
- ✅ Task 3: SEO → Blog → Twitter — publish_and_promote phase added to seo_content
- ✅ Task 4: Follow-up Email Scheduling — schedule_followups phase in outreach hand
- ✅ Task 5: Cost Tracking — cost_tracker.rs module + /costs command + GET /costs API
- ✅ Task 6: Application Status Tracking — memory-based duplicate detection in freelancer
- ✅ Task 7: Approval Gate for Proposals — human_review phase in freelancer hand
- ✅ Task 8: CRM Status Management — outreach memory + /crm Telegram command
- ✅ Task 9: auto_report Hand — 4-phase B2B report generator
- ✅ Task 10: PDF Export — pandoc/weasyprint/Python fallback tool
- ✅ Task 11: Brand Voice Profile — memory_recall brand_voice_profile in content/seo_content
- ✅ Task 12-15: E2E Tests — real pipeline tests with file output verification
- ✅ Task 16: Revenue Tracker — revenue_tracker.rs + /revenue command + GET /revenue API
- ✅ Task 17: Cost Tracking in AgentRuntime — automatic per-call recording
- ✅ Task 18: customer_service Hand — 4-phase B2B customer support (Route C)
- ✅ Task 19: trading_analysis Hand — 4-phase trading research (Route J)
- ✅ Task 20: Conversation test fix — stale temp dir cleanup
- ✅ Task 21: Content hand publish phase — 4th phase auto-posts via twitter + blog_publish
- ✅ Task 22: Freelancer Upwork integration — browser tool navigates Upwork job listings directly
- ✅ Task 23: Default cron jobs — 4 jobs auto-registered on startup (freelancer daily, leads weekly, seo biweekly, content daily)
- ✅ Task 24: Twitter tool execution test — validates input, enforces 280 char limit
- ✅ Task 25: Blog publish execution test — creates real MDX files + updates index.ts in dry_run
- ✅ Task 26: Email tool execution test — validates inputs, handles missing SMTP gracefully
- ✅ Task 27: Full content pipeline test — 4-phase write→read→validate→report with real files
- ✅ Task 28: Content hand publish phase test — verifies twitter/blog_publish in system prompts
- ✅ Task 29: Complete lead→outreach pipeline test — full chain with tools + costs + revenue + email + files
- ✅ Task 30: Complete SEO→publish pipeline test — 5-phase blog_publish dry_run + twitter + cost tracking
- ✅ Task 31: Freelancer Upwork pipeline test — job data + scoring + proposals with real file verification
- ✅ Task 32: Cron default jobs test — 4 default jobs match real hand names + schedules
- ✅ Task 33: TOML parsing bug fix — all 10 hand.toml files had `tools` inside `[settings]` or after `[[phases]]` (silently ignored by serde). Moved `tools` to top-level before `[settings]`
- ✅ Task 34: HandRunner integration tests — 3 tests that actually call HandRunner::run() end-to-end (graceful failure without LLM, real hand validation, chain propagation)
- ✅ Task 35: Dockerfile + docker-compose.yml — Route G containerization (multi-stage Rust build + Playwright + pandoc)
- **Final: 10 hands, 20 tools, 333 lib + 63 integration = 396 tests (0 failures)**

## Route Coverage Matrix (All 10 Routes)

| Route | Name | Hands | Status |
|-------|------|-------|--------|
| A | Freelance Dev | freelancer | ✅ 5 phases + approval + cron + memory |
| B | B2B Cold Email | lead → outreach (chain) | ✅ pipeline + CRM + follow-up |
| C | B2B Subscription | auto_report + customer_service | ✅ 4+4 phases |
| D | SEO Content | seo_content | ✅ 5 phases + blog_publish + twitter |
| E | Social Content | content | ✅ 4 phases + twitter + blog_publish + brand voice |
| F | Agent Packs | 10 hand TOMLs | ✅ loadable, GET /hands API |
| G | Managed Hosting | Dockerfile + docker-compose | ✅ containerized (multi-tenant = future) |
| H | Research Products | researcher + market_intel | ✅ 5+5 phases + PDF export |
| I | Developer Tools | cost_tracker + revenue_tracker | ✅ /costs + /revenue API |
| J | Trading Analysis | trading_analysis | ✅ 4 phases (research, no execution) |
