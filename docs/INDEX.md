# Documentation Index

_Navigator for the Phantom Mesh docs — updated 2026-06-19._

This page links every document under `docs/`. For project **status** (what is
shipped / in progress / planned), see the single source of truth:
[`/ROADMAP.md`](../ROADMAP.md). For repository rules and contribution workflow,
see [`/CONTRIBUTING.md`](../CONTRIBUTING.md).

> Authority note: when a document and the as-built code disagree, the code wins;
> [`/ROADMAP.md`](../ROADMAP.md) and [`FEATURE-MATRIX.md`](FEATURE-MATRIX.md) are
> the honest status references. Architecture docs are generated from the source
> and kept in sync per change.

---

## Getting started

| Document | Description |
|---|---|
| [QUICKSTART.md](QUICKSTART.md) | Fastest path from download to a running agent |
| [GETTING-STARTED.md](GETTING-STARTED.md) | Step-by-step first-run and second-machine mesh setup |
| [FAQ.md](FAQ.md) | Frequently asked questions |
| [configuration.md](configuration.md) | Configuration reference |
| [PERMISSIONS.md](PERMISSIONS.md) | Tool-permission model and prompts |
| [troubleshooting.md](troubleshooting.md) | Common problems and fixes |

## Install

| Document | Description |
|---|---|
| [INSTALL-MAC.md](INSTALL-MAC.md) | macOS install |
| [INSTALL-LINUX.md](INSTALL-LINUX.md) | Linux install |
| [INSTALL-WINDOWS.md](INSTALL-WINDOWS.md) | Windows install |
| [INSTALL-ANDROID.md](INSTALL-ANDROID.md) | Android install |
| [INSTALL-IOS.md](INSTALL-IOS.md) | iOS install / sideload |
| [INSTALL-OCI.md](INSTALL-OCI.md) | Container / OCI install |
| [install-binary-verification.md](install-binary-verification.md) | Verifying downloaded binaries |

## Architecture

| Document | Description |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Top-level architecture overview ([繁中](ARCHITECTURE.zh-TW.md)) |
| [architecture/README.md](architecture/README.md) | Subsystem architecture index |
| [architecture/agent-runtime.md](architecture/agent-runtime.md) | Multi-provider agent loop, tool dispatch, compaction |
| [architecture/provider-routing.md](architecture/provider-routing.md) | LLM provider selection, fallback, retry |
| [architecture/auth-security-gate.md](architecture/auth-security-gate.md) | Secure-by-default auth gate for tools / RPC |
| [architecture/cli-phantom.md](architecture/cli-phantom.md) | `phantom` CLI surface |
| [architecture/mcp-server.md](architecture/mcp-server.md) | MCP server that exposes tools |
| [architecture/cluster-dispatch.md](architecture/cluster-dispatch.md) | Cross-host task dispatch + capability routing |
| [architecture/capabilities-routing.md](architecture/capabilities-routing.md) | Capability discovery and routing |
| [architecture/broker-vault.md](architecture/broker-vault.md) | Broker JWT + per-user encrypted token vault |
| [architecture/at-rest-crypto-storage.md](architecture/at-rest-crypto-storage.md) | HKDF / age / HMAC at-rest encryption |
| [architecture/event-storage.md](architecture/event-storage.md) | Device-side event store + FTS5 |
| [architecture/capture-wires.md](architecture/capture-wires.md) | Focus / food / habit capture pipeline |
| [architecture/coach-daily-review.md](architecture/coach-daily-review.md) | Coach engine + daily review |
| [architecture/evolve-goals.md](architecture/evolve-goals.md) | Goal evolution + checkpoints |
| [architecture/hermes-skills.md](architecture/hermes-skills.md) | Skill extraction / curation loop |
| [architecture/channels-telegram.md](architecture/channels-telegram.md) | Telegram bot channel |
| [architecture/app-tauri-frontend.md](architecture/app-tauri-frontend.md) | Tauri desktop / mobile frontend |
| [architecture/i18n-localization.md](architecture/i18n-localization.md) | Localization (en / zh-TW) |
| [architecture/selftest-harness.md](architecture/selftest-harness.md) | `phantom selftest` harness ([繁中](architecture/selftest-harness.zh-TW.md)) |
| [SWARM-ARCHITECTURE.md](SWARM-ARCHITECTURE.md) | Distributed swarm architecture |
| [adr/ADR-001-cap-xx-to-slug-rename.md](adr/ADR-001-cap-xx-to-slug-rename.md) | ADR: capability slug rename |
| [adr/ADR-002-agy-not-used-on-remote.md](adr/ADR-002-agy-not-used-on-remote.md) | ADR: remote CLI choice |
| [adr/ADR-003-ai-with-memory-default-off.md](adr/ADR-003-ai-with-memory-default-off.md) | ADR: memory default off |
| [adr/ADR-006-mobile-execution-model.md](adr/ADR-006-mobile-execution-model.md) | ADR: mobile execution model |

## Cluster & mesh

| Document | Description |
|---|---|
| [CLUSTER-COWORK.md](CLUSTER-COWORK.md) | Multi-node co-working on the mesh |
| [CLUSTER-SCALE.md](CLUSTER-SCALE.md) | Scaling the cluster |
| [MULTI-DEVICE-COORDINATION.md](MULTI-DEVICE-COORDINATION.md) | Coordinating across devices |
| [TAILSCALE-SETUP.md](TAILSCALE-SETUP.md) | Private-network mesh setup |
| [SCENARIOS-MULTIAGENT.md](SCENARIOS-MULTIAGENT.md) | Multi-agent scenarios |

## Deploy & release

| Document | Description |
|---|---|
| [DEPLOY.md](DEPLOY.md) | Deployment guide |
| [DEPLOYMENT.md](DEPLOYMENT.md) | Deployment reference |
| [DEPLOY-MAC-STAGING.md](DEPLOY-MAC-STAGING.md) | macOS staging deploy |
| [PUBLISHING-BINARIES.md](PUBLISHING-BINARIES.md) | Publishing release binaries |
| [release-android-signed.md](release-android-signed.md) | Signed Android release |
| [mcp-registry-submission.md](mcp-registry-submission.md) | MCP registry submission |
| [RELEASE-NOTES-v0.6.0-rc1.md](RELEASE-NOTES-v0.6.0-rc1.md) | v0.6.0-rc1 release notes |

## Platform & app

| Document | Description |
|---|---|
| [MOBILE-VS-DESKTOP.md](MOBILE-VS-DESKTOP.md) | Mobile vs desktop capabilities |
| [MOBILE-WEB-MODE.md](MOBILE-WEB-MODE.md) | Mobile web mode |
| [cli/linux-cli-spec.md](cli/linux-cli-spec.md) | Linux CLI behaviour reference |
| [CLAUDE-CODE-SETUP.md](CLAUDE-CODE-SETUP.md) | Using phantom as a Claude Code subagent |

## Providers & integrations

| Document | Description |
|---|---|
| [INTEGRATIONS.md](INTEGRATIONS.md) | Integrations overview |
| [FREE-LLM-PROVIDERS-2026-05.md](FREE-LLM-PROVIDERS-2026-05.md) | Free LLM provider notes (May 2026 snapshot) |
| [MLX-PROVIDER.md](MLX-PROVIDER.md) | MLX provider notes |
| [anthropic-streaming-upgrades.md](anthropic-streaming-upgrades.md) | Anthropic streaming notes |

## Capture, coach & skills

| Document | Description |
|---|---|
| [cuj/README.md](cuj/README.md) | Critical user journeys index |
| [cuj/01-install-to-first-habit.md](cuj/01-install-to-first-habit.md) | CUJ: install to first habit |
| [cuj/02-daily-capture-loop.md](cuj/02-daily-capture-loop.md) | CUJ: daily capture loop |
| [cuj/03-cross-device-resume.md](cuj/03-cross-device-resume.md) | CUJ: cross-device resume |
| [cuj/04-degraded-states.md](cuj/04-degraded-states.md) | CUJ: degraded states |
| [cuj/05-export-and-uninstall.md](cuj/05-export-and-uninstall.md) | CUJ: export and uninstall |
| [flow/cuj-02/habit.md](flow/cuj-02/habit.md) | Flow detail: habit capture |
| [playbook/cuj-02/habit.md](playbook/cuj-02/habit.md) | Playbook: habit capture |
| [SELF-EVOLVE.md](SELF-EVOLVE.md) | Self-evolution loop |
| [EVOLVE-GOALS.md](EVOLVE-GOALS.md) | Evolve goals |
| [GOAL-LIST.md](GOAL-LIST.md) | Goal list |
| [CO-EVOLUTION.md](CO-EVOLUTION.md) | Co-evolution notes |
| [hermes-skills/README.md](hermes-skills/README.md) | Hermes skills index |
| [hermes-skills/sample-skill.md](hermes-skills/sample-skill.md) | Sample skill |
| [hermes-skills/audit-deps.md](hermes-skills/audit-deps.md) | Skill: audit deps |
| [hermes-skills/generate-changelog.md](hermes-skills/generate-changelog.md) | Skill: generate changelog |
| [hermes-skills/pre-release-check.md](hermes-skills/pre-release-check.md) | Skill: pre-release check |
| [hermes-skills/run-tests.md](hermes-skills/run-tests.md) | Skill: run tests |

## Security

| Document | Description |
|---|---|
| [PERMISSIONS.md](PERMISSIONS.md) | Permission model |
| [security-overrides.md](security-overrides.md) | Security override env-vars |
| [ANTI-HALLUCINATION-V1-DESIGN.md](ANTI-HALLUCINATION-V1-DESIGN.md) | Anti-hallucination design |

## Testing & quality

| Document | Description |
|---|---|
| [SELFTEST.md](SELFTEST.md) | Self-test guide |
| [TESTING-WINDOWS.md](TESTING-WINDOWS.md) | Windows testing notes |
| [TROUBLESHOOTING-MAC.md](TROUBLESHOOTING-MAC.md) | macOS troubleshooting |
| [test-cases/mac.md](test-cases/mac.md) | macOS test cases |
| [test-cases/COVERAGE-MAP-mac.md](test-cases/COVERAGE-MAP-mac.md) | macOS coverage map |
| [manual-playbook/mac.md](manual-playbook/mac.md) | macOS manual playbook |
| [e2e-mac-real-testing.md](e2e-mac-real-testing.md) | macOS real-device E2E |
| [e2e-app-native-webdriver.md](e2e-app-native-webdriver.md) | Native app WebDriver E2E |
| [IOS-TEST-FLOW.md](IOS-TEST-FLOW.md) | iOS test flow |
| [SMOKE-ANDROID.md](SMOKE-ANDROID.md) | Android smoke test |
| [tdd/workflow.zh-TW.md](tdd/workflow.zh-TW.md) | TDD workflow (繁中) |
| [tdd/README.zh-TW.md](tdd/README.zh-TW.md) | TDD overview (繁中) |

## Project & process

| Document | Description |
|---|---|
| [positioning.md](positioning.md) | Product positioning |
| [ECOSYSTEM.md](ECOSYSTEM.md) | Ecosystem overview |
| [FEATURE-MATRIX.md](FEATURE-MATRIX.md) | Honest per-feature status matrix |
| [CONTRIBUTOR-FUNNEL.md](CONTRIBUTOR-FUNNEL.md) | Contributor funnel |
| [CROSS-TOOL-DESIGN.md](CROSS-TOOL-DESIGN.md) | Cross-tool design |
| [dev-process-v2.md](dev-process-v2.md) | Development process |
| [PHANTOMMESH-IO-DESIGN.md](PHANTOMMESH-IO-DESIGN.md) | phantommesh.io site/broker design |
| [badges.md](badges.md) | README badge reference |

## Experimental

| Document | Description |
|---|---|
| [experimental-hermes-curator.md](experimental-hermes-curator.md) | Experimental: Hermes curator |
| [experimental-hermes-memory.md](experimental-hermes-memory.md) | Experimental: Hermes memory |
| [experimental-hermes-providers.md](experimental-hermes-providers.md) | Experimental: Hermes providers |
| [experimental-hermes-tools.md](experimental-hermes-tools.md) | Experimental: Hermes tools |
| [experimental-openclaw.md](experimental-openclaw.md) | Experimental: OpenClaw |
| [experimental-openclaw-telegram.md](experimental-openclaw-telegram.md) | Experimental: OpenClaw + Telegram |

## Blog & history

| Document | Description |
|---|---|
| [blog/2026-05-15-v0.5.0-pre-launch.md](blog/2026-05-15-v0.5.0-pre-launch.md) | v0.5.0 pre-launch post |
| [perf/2026-05-15-baselines.md](perf/2026-05-15-baselines.md) | Performance baselines (May 2026) |
| [ios-dev-loop-record-2026-05-29.md](ios-dev-loop-record-2026-05-29.md) | iOS dev-loop record |
| [integration/2026-05-30-mac-app-cli-test-playbook.md](integration/2026-05-30-mac-app-cli-test-playbook.md) | macOS app/CLI test playbook |
| [integration/test-cases/CLI-mac-results-2026-05-30.md](integration/test-cases/CLI-mac-results-2026-05-30.md) | macOS CLI test results |
| [spec/history/2026-05-30/README.md](spec/history/2026-05-30/README.md) | Historical spec snapshot (deferred specs + design variants) |
| [_archive/](_archive/) | Archived/superseded docs (kept for history) |
