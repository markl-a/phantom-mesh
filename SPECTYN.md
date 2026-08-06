# Project: spectyn-mesh

## Overview

TODO: Add project description

## Project Type

Unknown

## Key Files
- (none found)

## Directory Structure
- Payload/
- _planning-audit/
- app/
- configs/
- core/
- crates/
- demos/
- dist/
- docs/
- evals/
- ios-sandbox/
- mobile/
- spectynmesh-io/
- scripts/
- target-tdd/
- tasks/
- templates/
- test-results/
- tests-e2e/

**File counts:** .md: 13, .example: 2, .toml: 2, .yaml: 1, .json: 1

## Existing Docs
- `README.md`
- `CONTRIBUTING.md`
- `CHANGELOG.md`

## Build & Test

```bash
make build     # build
make test      # run tests
make check     # type check / lint
```

## Agent Instructions

- Always run `make check` after editing Unknown files
- Read files before editing them
- Create tests for new functionality
- Follow existing code style
- Prefer editing existing files over creating new ones
- Check git status before committing

## README Excerpt

# Spectyn Mesh

> **一個跑在你自己機器、用 Telegram 跟你對話、會記住「上次怎麼解決」的 AI agent（人工智慧代理）。**
>
> **An AI agent that runs on your hardware, answers your Telegram messages, and remembers what worked so next time it's faster.**

**今天就跑得起來的兩個 wedge（切入點，v0.5.0）：**
- **Telegram bot（電報機器人）** — 把 BotFather token（機器人權杖）給 daemon（常駐程式），從手機傳訊息給它，它…
