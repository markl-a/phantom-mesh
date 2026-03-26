# Phantom Mesh Tools Reference

All 42 tools registered in Phantom Mesh Core. Tools are invoked by agents during task execution and can also be dispatched directly to cluster workers.

Use `GET /tools` to retrieve the live list at runtime.

---

## Category: Shell / System

### shell
Execute shell commands with allowlist enforcement.
- **Parameters**: `command` (string) — the shell command to run
- **Security**: Only commands in `security.allowed_commands` whitelist are permitted
- **Routing**: LOCAL_ONLY (hub) or FullWorkerOnly (Z13/M1)
- **Preflight**: Checks command against allowlist before execution

### computer_use
Simulate keyboard/mouse input on the host desktop (GUI automation).
- **Parameters**: `action` (string), `coordinate` (array), `text` (string), `key` (string)
- **Routing**: FullWorkerOnly

### cli_anything
Run arbitrary CLI tools with more flexible invocation than `shell`.
- **Parameters**: `command` (string), `args` (array), `stdin` (string)

---

## Category: File Operations

### file_read
Read a file from the workspace sandbox.
- **Parameters**: `path` (string) — relative to `~/.phantom-mesh/workspace/`
- **Security**: Workspace-only; cannot traverse outside sandbox
- **Preflight**: Checks file exists before reading
- **Routing**: LOCAL_ONLY (hub)

### file_write
Write content to a file in the workspace sandbox.
- **Parameters**: `path` (string), `content` (string), `append` (boolean, optional)
- **Security**: Workspace-only
- **Routing**: LOCAL_ONLY (hub)

### file_edit
Perform targeted find-and-replace edits on an existing workspace file.
- **Parameters**: `path` (string), `old_text` (string), `new_text` (string)
- **Security**: Workspace-only
- **Routing**: LOCAL_ONLY (hub)

### glob_search
Find files matching a glob pattern in the workspace.
- **Parameters**: `pattern` (string) — glob pattern (e.g., `*.md`, `**/*.rs`)
- **Routing**: LOCAL_ONLY (hub)

### content_search
Search file contents using regex within the workspace.
- **Parameters**: `pattern` (string), `path` (string, optional)
- **Routing**: LOCAL_ONLY (hub)

---

## Category: Web / HTTP

### web_search
Multi-backend web search with automatic fallback chain.
- **Parameters**: `query` (string), `num_results` (integer, optional)
- **Backends**: Serper API (primary) -> Tavily API (fallback) -> Google News RSS -> direct URL fetch
- **Routing**: AnyWorker (prefer Acer for light load)
- **Mobile**: Supported (Phase 1)

### http_request
Make arbitrary HTTP requests.
- **Parameters**: `url` (string), `method` (string), `headers` (object), `body` (string/object)
- **Routing**: AnyWorker
- **Mobile**: Supported (Phase 1)

### browser
Headless browser automation (navigate, click, scrape pages).
- **Parameters**: `url` (string), `action` (string: navigate/click/extract/screenshot), `selector` (string), `wait` (integer)
- **Routing**: FullWorkerOnly (requires Playwright/Chromium)

---

## Category: Memory

### memory_store
Store a value in the persistent semantic memory database.
- **Parameters**: `key` (string), `content` (string), `category` (string: conversation/fact/preference/skill/task)
- **Database**: `~/.phantom-mesh/memory.db`
- **Routing**: LOCAL_ONLY (hub)

### memory_recall
Recall stored memories by keyword similarity search.
- **Parameters**: `query` (string), `limit` (integer), `category` (string, optional)
- **Routing**: LOCAL_ONLY (hub)

### memory_forget
Delete a stored memory by key.
- **Parameters**: `key` (string)
- **Routing**: LOCAL_ONLY (hub)

---

## Category: AI / Code Generation

### ai_code
Generate, review, or refactor code using LLM with iteration.
- **Parameters**: `task` (string), `language` (string), `context` (string), `iterations` (integer)
- **Routing**: FullWorkerOnly (Z13/M1 for GPU acceleration)

### skeleton_generate
Skeleton-of-Thought parallel content generation (fan out to multiple providers).
- **Parameters**: `topic` (string), `sections` (integer), `providers` (array)
- **Use case**: Long-form content with parallel section expansion

### delegate
Delegate a task to another named agent.
- **Parameters**: `agent` (string), `prompt` (string)
- **Use case**: Sub-agent orchestration

### delegate_to_provider
Send a prompt directly to a specific LLM provider.
- **Parameters**: `provider` (string), `prompt` (string), `model` (string)

### run_hand
Trigger a hand workflow from within an agent loop.
- **Parameters**: `hand_name` (string), `input` (string)

---

## Category: Vision / Media

### vision
Analyze images using multimodal LLM.
- **Parameters**: `image_path` (string), `prompt` (string)
- **Supports**: workspace image files or URLs

### image_generate
Generate images using AI (Gemini Imagen or compatible API).
- **Parameters**: `prompt` (string), `output_path` (string), `size` (string), `style` (string)
- **Config**: Requires `image_generate.gemini_api_key` in agents.toml

### video_compose
Compose video from image frames, audio, and text overlay.
- **Parameters**: `frames` (array), `audio` (string), `output` (string), `fps` (integer)

### tts
Text-to-speech synthesis.
- **Parameters**: `text` (string), `voice` (string), `output_path` (string), `speed` (number)

---

## Category: Communication

### email_send
Send emails via SMTP.
- **Parameters**: `to` (string/array), `subject` (string), `body` (string), `attachments` (array)
- **Config**: Requires `[email]` section in agents.toml

### email_receive
Receive and parse emails via IMAP.
- **Parameters**: `folder` (string), `limit` (integer), `since` (string), `search` (string)
- **Config**: Requires `[imap]` section in agents.toml
- **Note**: Uses `python3` on macOS/Linux (with fallback), `python` on Windows

### twitter
Post tweets, threads, search, and read timeline.
- **Parameters**: `action` (string: post/thread/search/read), `text` (string/array), `query` (string)
- **Config**: Requires `[twitter]` section with API keys

### slack
Send messages to Slack channels.
- **Parameters**: `channel` (string), `text` (string), `blocks` (array)
- **Config**: Requires `[slack]` section

### discord
Send messages to Discord channels.
- **Parameters**: `channel_id` (string), `content` (string), `embeds` (array)
- **Config**: Requires `[discord]` section

### line_notify
Send LINE Notify notifications.
- **Parameters**: `message` (string), `image_url` (string)
- **Config**: Requires `[line]` section

### whatsapp
Send WhatsApp messages.
- **Parameters**: `to` (string), `message` (string)
- **Config**: Requires `[whatsapp]` section

---

## Category: Publishing / Content

### blog_publish
Publish articles to a blog platform (Ghost, WordPress, etc.).
- **Parameters**: `title` (string), `content` (string), `tags` (array), `dry_run` (boolean)
- **Config**: Requires `[blog]` section

### pdf_export
Export content to PDF file.
- **Parameters**: `content` (string), `output_path` (string), `title` (string), `format` (string)

### docx_export
Export content to DOCX Word file.
- **Parameters**: `content` (string), `output_path` (string), `title` (string)

### xlsx_export
Export tabular data to XLSX Excel file.
- **Parameters**: `data` (array of arrays), `output_path` (string), `sheet_name` (string), `headers` (array)

### youtube_upload
Upload videos to YouTube.
- **Parameters**: `video_path` (string), `title` (string), `description` (string), `tags` (array), `privacy` (string)

---

## Category: Music / Audio

### music_generate
Generate music using AI (Suno, Udio, or compatible).
- **Parameters**: `prompt` (string), `duration` (integer), `style` (string), `output_path` (string)

---

## Category: Data Processing

### json_transform
Apply jq-like transformations to JSON data.
- **Parameters**: `input` (object/array), `transform` (string — JSONPath or dot notation)

### csv_parse
Parse and analyze CSV data.
- **Parameters**: `content` (string), `operation` (string: parse/filter/aggregate/sort), `filter` (string), `column` (string)

### translate
Translate text between languages.
- **Parameters**: `text` (string), `target_language` (string), `source_language` (string, optional)

### summarize
Summarize long text using LLM.
- **Parameters**: `text` (string), `max_words` (integer), `style` (string: bullet/paragraph/headline)

---

## Category: Business / Monetization

### stripe
Stripe payment operations (create customer, product, payment link).
- **Parameters**: `action` (string: create_customer/create_product/create_payment_link/list_customers), various action-specific fields
- **Config**: Requires `[stripe] secret_key` in agents.toml

### render_deploy
Deploy web services to Render.com.
- **Parameters**: `service_name` (string), `repo_url` (string), `env_vars` (object), `plan` (string)
- **Config**: Requires `[render] api_key` in agents.toml

### scaffold_saas
Generate a complete SaaS project scaffold (Node.js + Stripe + Docker).
- **Parameters**: `project_name` (string), `description` (string), `features` (array), `output_dir` (string)

---

## Tool Security Model

All tools run through a shared security/rate-limit pipeline:

1. **Preflight check** — each tool can declare preflight conditions (e.g., file_read checks existence, shell checks allowlist)
2. **Rate limit check** — global 840 calls/hr, per-tool 280 calls/hr (sliding window)
3. **InjectionGuard** — 8 regex patterns detect prompt injection before LLM tool calls
4. **Credential scrubbing** — tool output is scrubbed for API keys, tokens, and secrets before returning
5. **Audit logging** — every tool call is logged to `~/.phantom-mesh/audit.db` with risk level and outcome

## Tool Routing

| Routing Class | Tools | Notes |
|---------------|-------|-------|
| LOCAL_ONLY | file_*, memory_*, glob_search, content_search | Always run on hub (Z13) |
| AnyWorker | web_search, http_request, email_send | Auto-distributed; prefer Acer |
| FullWorkerOnly | shell, ai_code, browser, skeleton_generate | Only Z13 or M1 Mac |
| MobileOnly | sensor_gps, camera, accelerometer, audio, local_llm, js_exec | Only mobile devices (Phase 2-4) |
