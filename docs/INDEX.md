# Phantom Mesh Documentation Index

[繁體中文版](INDEX.zh-TW.md)

This page is a top-level map of the `docs/` tree, grouped by purpose. It
distinguishes current sources of truth from historical material so contributors
do not accidentally implement superseded plans.

## Start Here

1. [`OPERATING-STANDARD.md`](OPERATING-STANDARD.md) — 運行唯一 SSOT(HOW + 路線圖 + 治理 + 檔案標準摘要)。已折入原 GOVERNANCE / FLEET-DEV / JOINT-DEV / ROADMAP-VISUAL 四份文件。
2. [`superpowers/BIG-GOAL.md`](superpowers/BIG-GOAL.md) — the **locked apex / constitution** (final re-lock 2026-06-11): 4 pillars P1–P4, 2 tracks (Life/Work), governance pyramid in §10.
3. [`../AGENTS.md`](../AGENTS.md) — repository rules, boundaries, and TDD workflow.
4. [`superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md`](superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md) — deep-spec catalog and reading order for implementation work.
5. [`../SESSION_RESUME.md`](../SESSION_RESUME.md) — latest tactical handoff and next concrete step.

> **Note:** [`_archive/NORTH-STAR.md`](_archive/NORTH-STAR.md) and [`_archive/2026-06-19-BIG-GOAL.zh-TW.md`](_archive/2026-06-19-BIG-GOAL.zh-TW.md) are **superseded** by BIG-GOAL.md (both already bannered; the zh-TW mirror is now archived since BIG-GOAL.md is the 繁中 正本). Do not treat them as current direction.

## Governance & Vision

| Document | Purpose |
|---|---|
| [`OPERATING-STANDARD.md`](OPERATING-STANDARD.md) | 運行唯一 SSOT — HOW + 路線圖 + 治理(§4 金字塔/導航/真相鏈) + 檔案標準摘要 |
| [`superpowers/GOVERNANCE.md`](superpowers/GOVERNANCE.md) | 🪦 折入 OPERATING-STANDARD.md §4；此路徑為轉址 stub (保留 inbound 連結) |
| [`superpowers/BIG-GOAL.md`](superpowers/BIG-GOAL.md) | Locked apex / constitution for the v0.6.0+ cycle (4 pillars, 2 tracks) |
| [`superpowers/specs/v060-deep-spec/SPEC-01-FOUNDATION-bigGoal-mapping.md`](superpowers/specs/v060-deep-spec/SPEC-01-FOUNDATION-bigGoal-mapping.md) | Maps each pillar to implementable sub-capabilities |
| [`superpowers/ROADMAP-v0.6.0.md`](superpowers/ROADMAP-v0.6.0.md) | v0.6.0 roadmap DAG (scoreboard authority is V0.6.0-RELEASE-PLAN.md) |
| [`superpowers/V0.6.0-RELEASE-PLAN.md`](superpowers/V0.6.0-RELEASE-PLAN.md) | Release scoreboard and dates |
| [`superpowers/V0_7_0_DEFERRAL_INVENTORY.md`](superpowers/V0_7_0_DEFERRAL_INVENTORY.md) | What is explicitly deferred to v0.7.0+ |

## 開發運作 / 生態整理 (Dev & Ecosystem)

| Document | Purpose |
|---|---|
| [`OPERATING-STANDARD.md`](OPERATING-STANDARD.md) | 運行唯一 SSOT — §2 怎麼開發(艦隊/多AI/協調層/派工樹) · §3 路線圖 · §4 治理。已折入 FLEET-DEV/JOINT-DEV/ROADMAP-VISUAL/GOVERNANCE |
| `FLEET-DEV-OPERATING-MODEL.md` | 🪦 折入 OPERATING-STANDARD.md §2(轉址 stub) · 歷史版 [`_archive/2026-06-19-FLEET-DEV-OPERATING-MODEL.md`](_archive/2026-06-19-FLEET-DEV-OPERATING-MODEL.md) |
| `JOINT-DEV-FRAMEWORKS-OSS-2026-06.md` | 🪦 折入 OPERATING-STANDARD.md §2.4(轉址 stub) · 歷史版 [`_archive/2026-06-19-JOINT-DEV-FRAMEWORKS-OSS-2026-06.md`](_archive/2026-06-19-JOINT-DEV-FRAMEWORKS-OSS-2026-06.md) |
| [`DOC-SETUP-AUDIT-2026-06.md`](DOC-SETUP-AUDIT-2026-06.md) | 11 專案文件設定對照表 |
| `ROADMAP-VISUAL.zh-TW.md` | 🪦 折入 OPERATING-STANDARD.md §3(轉址 stub) · 歷史版 [`_archive/2026-06-19-ROADMAP-VISUAL.zh-TW.md`](_archive/2026-06-19-ROADMAP-VISUAL.zh-TW.md) |

> 規劃文件:[`../plans/KB-CONSOLIDATION-PLAN-2026-06-18.md`](../plans/KB-CONSOLIDATION-PLAN-2026-06-18.md) · 現行路線圖真相見 [`OPERATING-STANDARD.md §3`](OPERATING-STANDARD.md)(`PROJECT-FINAL-FORMS-2026-06-18` 已歸檔 → [`_archive/2026-06-18-PROJECT-FINAL-FORMS.md`](_archive/2026-06-18-PROJECT-FINAL-FORMS.md))

## Specs

### Deep Spec (implementable)

| Document | Purpose |
|---|---|
| [`superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md`](superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md) | Deep-spec catalog and reading order (the directory entry point) |
| [`superpowers/specs/v060-deep-spec/`](superpowers/specs/v060-deep-spec/) | All implementable SPEC-NN files (Foundation / Protocol / System / Platform / Server / Testing / Experimental) |
| [`superpowers/SPEC-TO-CODE-PLAYBOOK.md`](superpowers/SPEC-TO-CODE-PLAYBOOK.md) | Staged spec-to-implementation playbook |

### Active Epics

The active v0.6.0 epic specifications live in
[`superpowers/specs/_current/`](superpowers/specs/_current/):

| Epic | Specification | Status recorded in spec |
|---|---|---|
| E001 | [`Cross-host cluster smoke`](superpowers/specs/_current/E001-cross-host-cluster-smoke.md) | Maintenance |
| E002 | [`Multimodal capture pipeline`](superpowers/specs/_current/E002-multimodal-capture-pipeline.md) | Shipped |
| E003 | [`Coach node and daily review`](superpowers/specs/_current/E003-coach-node-daily-review.md) | Not started |
| E004 | [`Encrypted storage layer`](superpowers/specs/_current/E004-encrypted-storage-layer.md) | Shipped |
| E005 | [`Skill extraction`](superpowers/specs/_current/E005-hermes-skill-extraction.md) | Not started |
| E006 | [`30-second Life hello`](superpowers/specs/_current/E006-30-second-hello-world.md) | Not started |
| E007 | [`v0.6.0 release prep`](superpowers/specs/_current/E007-v060-release-prep.md) | Accepted |

Also in `_current/` (behavior spec, not an epic): [`linux-cli-spec.md`](superpowers/specs/_current/linux-cli-spec.md) — code-grounded `phantom` CLI behavior reference for Linux (moved here 2026-06-19 from `docs/cli/`).

The 2026-05-19 pivot spec is
[`superpowers/specs/2026-05-19-life-node-pivot.md`](superpowers/specs/2026-05-19-life-node-pivot.md).

## Features

Feature specs (F001+) live in [`superpowers/features/`](superpowers/features/).
Each declares its `Parent epic` and `Pillar(s) served` in the file header.

## Runbooks

| Document | Purpose |
|---|---|
| [`GETTING-STARTED.md`](GETTING-STARTED.md) / [`QUICKSTART.md`](QUICKSTART.md) | First steps |
| [`install/INSTALL-WINDOWS.md`](install/INSTALL-WINDOWS.md) / [`install/INSTALL-MAC.md`](install/INSTALL-MAC.md) / [`install/INSTALL-LINUX.md`](install/INSTALL-LINUX.md) / [`install/INSTALL-ANDROID.md`](install/INSTALL-ANDROID.md) / [`install/INSTALL-IOS.md`](install/INSTALL-IOS.md) / [`install/INSTALL-OCI.md`](install/INSTALL-OCI.md) | Per-platform install |
| [`mesh/FLEET-SSH.md`](mesh/FLEET-SSH.md) / [`mesh/MESH-FLEET-ONBOARDING.md`](mesh/MESH-FLEET-ONBOARDING.md) / [`mesh/TAILSCALE-SETUP.md`](mesh/TAILSCALE-SETUP.md) | Fleet / mesh networking |
| [`deploy/DEPLOYMENT.md`](deploy/DEPLOYMENT.md) / [`deploy/DEPLOY-AUTOUPDATE.md`](deploy/DEPLOY-AUTOUPDATE.md) / [`deploy/DEPLOY-MAC-STAGING.md`](deploy/DEPLOY-MAC-STAGING.md) | Deployment (`DEPLOY-AUTOUPDATE` = signing + release CI + OTA) |
| [`deploy/PUBLISHING-BINARIES.md`](deploy/PUBLISHING-BINARIES.md) / [`mobile/SMOKE-ANDROID.md`](mobile/SMOKE-ANDROID.md) / [`SELFTEST.md`](SELFTEST.md) / [`DIAGNOSTICS.md`](DIAGNOSTICS.md) | Release / smoke / diagnostics |
| [`../scripts/phantom-test/README.md`](../scripts/phantom-test/README.md) | Black-box CLI / HTTP-RPC / round-trip test harness |
| [`../tests-e2e/README.md`](../tests-e2e/README.md) | Human-assisted Tier-1 E2E scenarios |

## Topical Subdirectories

The `docs/` root was reorganized (2026-06-19) — most topic docs now live in grouped
subdirectories. Browse by topic:

| Subdir | Contents |
|---|---|
| [`install/`](install/) | Per-platform install (Windows/Mac/Linux/Android/iOS/OCI), binary verification, Apple Sign-In + auth-provider setup |
| [`deploy/`](deploy/) | Deployment, auto-update/OTA, Mac staging, publishing binaries, mcp-registry submission, signed Android release |
| [`providers/`](providers/) | LLM provider/auth design (`DESIGN-PROVIDER-AUTH`, `AUTH-DESIGN`), MLX provider, free-LLM-provider survey |
| [`experimental/`](experimental/) | Skill bank (curator/memory/extra-providers/tools) + remote-control experimental notes |
| [`mesh/`](mesh/) | Cluster co-work/scale, fleet onboarding, FLEET-SSH, Tailscale, multi-device coordination, multi-agent analysis/QA, mobile-vs-desktop |
| [`mobile/`](mobile/) | iOS test flow, mobile web mode, e2e (mac-real / native-webdriver), Android smoke |
| [`design/`](design/) | Subsystem/design docs (cross-tool, phantommesh-io, platform-impl, anti-hallucination, commercial, swarm-architecture, dispatch-followups) |
| [`commercial/`](commercial/) | Open-source plan, contributor funnel, portfolio spec freeze (strategy SSOT docs stay at root) |
| [`dev/`](dev/) | Dev-acceleration framework + mesh, dev process, autonomous dev-loop, Claude Code setup, autonomy governance, anthropic streaming |
| [`dev-notes/`](dev-notes/) | Working dev notes (current, 2026-06-11): error-handling, Windows login-LLM verify, backlog/inbox/status scratch-pad |
| [`cuj/`](cuj/) | 5 Critical User Journeys (install→first-habit, daily-capture, cross-device-resume, degraded-states, export-uninstall) — see [`cuj/README.md`](cuj/README.md) |
| [`test-cases/`](test-cases/) | Per-surface test-case DBs (mac/win-cli/linux-cli/mac-app/win-app/android/ios) + COVERAGE-MAP + shared schema — see [`test-cases/README.md`](test-cases/README.md) |
| [`skills/`](skills/) | Skill-document library (YAML-frontmatter Markdown) for the curator/router — see [`skills/README.md`](skills/README.md) |
| [`ai-reviews/`](ai-reviews/) | Cross-AI review artifacts (adversarial-reader per-spec outputs, wave12, agy/codex/gemini review passes) — moved out of `scripts/ai/output` |
| [`ecosystem-roadmap/`](ecosystem-roadmap/) | Finalized roadmaps for the 9 satellite projects + main P0 decomposition — see [`ecosystem-roadmap/ECOSYSTEM-ROADMAP-FINAL.md`](ecosystem-roadmap/ECOSYSTEM-ROADMAP-FINAL.md) |
| [`plain/`](plain/) | 白話（plain-language, non-technical）explainer set — what the tool is / how I use it / real scenarios — see [`plain/00-索引.md`](plain/00-索引.md) |
| [`_archive/`](_archive/) | Superseded plans/specs (NORTH-STAR, MASTER-SPEC, EXECUTION-PLAN, …) — history only, not authority |

### Inside `superpowers/`

| Subdir | Contents |
|---|---|
| [`superpowers/plans/`](superpowers/plans/) | Dated implementation plans (the canonical plans home; epic/spec/feature landing plans) |
| [`superpowers/runbooks/`](superpowers/runbooks/) | Ship-gate / operator runbooks (E007-release-smoke, E001-testbed-setup, distributed-dev, mobile-cluster-dispatch-ui-smoke, …) |
| [`superpowers/specs/2026-06-12-platform-flows-design/`](superpowers/specs/2026-06-12-platform-flows-design/) | 16-file surface×ability×reality design-reference layer (non-SPEC authority; conclusions must sink into a SPEC leaf — Charter §A.2 rule 6) |

## Architecture & Design (reference)

> ⚠️ Items marked *pre-pivot* predate the 2026-05-19 Life-Node pivot. They describe
> implemented mechanisms but are **not** authority for current product scope —
> governance is [`superpowers/GOVERNANCE.md`](superpowers/GOVERNANCE.md).

| Document | Purpose |
|---|---|
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | Implemented daemon architecture (**pre-pivot reference**) |
| [`_archive/MASTER-SPEC.md`](_archive/MASTER-SPEC.md) | Historical 2026-06-07 inventory; pre-2026-06-11 relock, superseded by BIG-GOAL.md |
| [`architecture/`](architecture/) | Component design docs (agent runtime, Tauri frontend, crypto storage, selftest harness, …) |
| [`adr/`](adr/) | Architecture Decision Records (ADR-001+) |
| [`superpowers/design/`](superpowers/design/) | TUI / CLI screen design notes |
| [`superpowers/ARCH-EXECUTION-ENTITIES.md`](superpowers/ARCH-EXECUTION-ENTITIES.md) | Execution-entity architecture (A/B/C stacks) |
| [`providers/AUTH-DESIGN.md`](providers/AUTH-DESIGN.md) / [`design/SWARM-ARCHITECTURE.md`](design/SWARM-ARCHITECTURE.md) / [`design/PHANTOMMESH-IO-DESIGN.md`](design/PHANTOMMESH-IO-DESIGN.md) | Subsystem designs |

## Commercial & Strategy

> Downstream of the apex (subordinate); does not shape product. See BIG-GOAL §7.

| Document | Purpose |
|---|---|
| [`COMMERCIALIZATION-STRATEGY.md`](COMMERCIALIZATION-STRATEGY.md) | Side-business scope (subordinate to apex) |
| [`STRATEGY-DIFFERENTIATION.md`](STRATEGY-DIFFERENTIATION.md) | Execution-layer sequencing (subordinate to apex) |
| [`positioning.md`](positioning.md) | External positioning (**pre-pivot**) |
| [`design/COMMERCIAL-DESIGN.md`](design/COMMERCIAL-DESIGN.md) / [`commercial/OPEN-SOURCE-PLAN.md`](commercial/OPEN-SOURCE-PLAN.md) | Commercial / OSS planning |

## Archive

Dated snapshots, reports, and pre-Rust fossils are kept under
[`_archive/`](_archive/) for history. **Do not use as current authority.**
See [`_archive/README.md`](_archive/README.md).

## Quick Decision Guide

| Question | Read |
|---|---|
| How is the docs tree governed? Where do things live? | [`superpowers/GOVERNANCE.md`](superpowers/GOVERNANCE.md) |
| What product are we building? | [`superpowers/BIG-GOAL.md`](superpowers/BIG-GOAL.md) |
| Which spec governs my implementation? | [`superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md`](superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md) |
| How do I run black-box verification? | [`../scripts/phantom-test/README.md`](../scripts/phantom-test/README.md) |
| What is the latest tactical state? | [`../SESSION_RESUME.md`](../SESSION_RESUME.md) |
