# Phantom Mesh Documentation Index

[繁體中文版](INDEX.zh-TW.md)

This page is the practical entry point for product specifications and test
documentation. It distinguishes current sources of truth from historical
material so contributors do not accidentally implement superseded plans.

## Start Here

Read these documents in order before implementing a feature:

1. [`../AGENTS.md`](../AGENTS.md) - repository rules, boundaries, and TDD workflow.
2. [`superpowers/BIG-GOAL.md`](superpowers/BIG-GOAL.md) - current product direction:
   4 pillars, 2 tracks, and 3 operational principles. Re-locked 2026-05-19.
3. [`superpowers/specs/2026-05-19-life-node-pivot.md`](superpowers/specs/2026-05-19-life-node-pivot.md)
   - v0.6.0 Life Node pivot, active epics, feature scope, and roadmap.
4. [`superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md`](superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md)
   - deep-spec catalog for implementation work.
5. [`../SESSION_RESUME.md`](../SESSION_RESUME.md) - latest tactical handoff and next
   concrete step.

Use [`ARCHITECTURE.md`](ARCHITECTURE.md) as the architecture reference after the
product-direction documents above. It predates the Life Node pivot, so it is not
the authority for current product scope.

## Current Specifications

### Product Direction

| Document | Purpose |
|---|---|
| [`superpowers/BIG-GOAL.md`](superpowers/BIG-GOAL.md) | Immutable product anchor for the v0.6.0 cycle |
| [`superpowers/specs/2026-05-19-life-node-pivot.md`](superpowers/specs/2026-05-19-life-node-pivot.md) | Approved pivot spec and epic reframe |
| [`superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md`](superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md) | Deep-spec catalog and reading order |

### Active Epics

The active v0.6.0 epic specifications live in
[`superpowers/specs/_current/`](superpowers/specs/_current/):

| Epic | Specification | Status recorded in spec |
|---|---|---|
| E001 | [`Cross-host cluster smoke`](superpowers/specs/_current/E001-cross-host-cluster-smoke.md) | Maintenance |
| E002 | [`Multimodal capture pipeline`](superpowers/specs/_current/E002-multimodal-capture-pipeline.md) | Shipped |
| E003 | [`Coach node and daily review`](superpowers/specs/_current/E003-coach-node-daily-review.md) | Not started |
| E004 | [`Encrypted storage layer`](superpowers/specs/_current/E004-encrypted-storage-layer.md) | Shipped |
| E005 | [`Hermes skill extraction`](superpowers/specs/_current/E005-hermes-skill-extraction.md) | Not started |
| E006 | [`30-second Life hello`](superpowers/specs/_current/E006-30-second-hello-world.md) | Not started |
| E007 | [`v0.6.0 release prep`](superpowers/specs/_current/E007-v060-release-prep.md) | Accepted |

### Spec-to-Code Workflow

| Document | Purpose |
|---|---|
| [`superpowers/CONTRIBUTING-spec-to-product.md`](superpowers/CONTRIBUTING-spec-to-product.md) | Contributor guide: spec to types to implementation to product |
| [`superpowers/SPEC-TO-CODE-PLAYBOOK.md`](superpowers/SPEC-TO-CODE-PLAYBOOK.md) | Detailed staged implementation playbook |

## Test Documentation

### GA Gate: Use This First

| Document | Purpose |
|---|---|
| [`tdd/INDEX.md`](tdd/INDEX.md) | Live P0 checklist and source of truth for TDD scripts |
| [`tdd/workflow.md`](tdd/workflow.md) | Red-green-mark-done workflow and deviation rules |
| [`tdd/README.md`](tdd/README.md) | TDD directory overview |
| [`../scripts/tdd/README.md`](../scripts/tdd/README.md) | Driver script usage |

At the time this index was written, the live checklist contains 168 P0 items:
80 complete and 88 open. Treat [`tdd/INDEX.md`](tdd/INDEX.md), not older planning
counts, as authoritative.

Common commands:

```bash
./scripts/tdd/tdd-status.sh
./scripts/tdd/tdd-next.sh
./scripts/tdd/tdd-run.sh <test-name>
./scripts/tdd/tdd-mark-done.sh <test-name>
```

### Test Planning and Coverage

| Document | Purpose |
|---|---|
| [`planning/sprint-2026-05-18/31-phantom-mesh-tdd-comprehensive-plan-2026-05-18.md`](planning/sprint-2026-05-18/31-phantom-mesh-tdd-comprehensive-plan-2026-05-18.md) | P0/P1/P2 TDD plan and platform allocation |
| [`planning/lifecycle-tests/README.md`](planning/lifecycle-tests/README.md) | Five-platform lifecycle-test encyclopedia |
| [`superpowers/specs/v060-deep-spec/SPEC-60-TESTING-strategy.md`](superpowers/specs/v060-deep-spec/SPEC-60-TESTING-strategy.md) | V-track testing strategy |
| [`superpowers/specs/v060-deep-spec/SPEC-61-TESTING-scenarios.md`](superpowers/specs/v060-deep-spec/SPEC-61-TESTING-scenarios.md) | Scenario catalog |

The lifecycle files under [`planning/lifecycle-tests/`](planning/lifecycle-tests/)
are a wide-net test encyclopedia. They are not the GA checklist.

### Executable and Manual Scenarios

| Document | Purpose |
|---|---|
| [`../scripts/phantom-test/README.md`](../scripts/phantom-test/README.md) | Black-box CLI, HTTP/RPC, disk-state, and real round-trip harness |
| [`../tests-e2e/README.md`](../tests-e2e/README.md) | Human-assisted Tier-1 E2E scenarios |
| [`architecture/selftest-harness.md`](architecture/selftest-harness.md) | Self-test harness architecture |

At the time this index was written, `scripts/phantom-test/scenarios/` contains
36 scripts and `tests-e2e/scenarios/` contains 8 Tier-1 scenario documents.

## Historical Material

Do not use historical documents as current implementation authority:

| Path | Use |
|---|---|
| [`../_planning-audit/MASTER-PLAN.md`](../_planning-audit/MASTER-PLAN.md) | Strategic history and audit trail |
| [`../_planning-audit/archived/`](../_planning-audit/archived/) | Superseded planning material; read only for historical research |
| [`superpowers/specs/_archived/`](superpowers/specs/_archived/) | Superseded feature specs retained for traceability |

## Known Documentation Drift

These mismatches existed when this index was created:

- Older TDD documents mention 150 or 152 P0 items. The live
  [`tdd/INDEX.md`](tdd/INDEX.md) contains 168.
- [`../scripts/phantom-test/README.md`](../scripts/phantom-test/README.md) lists an
  older 16-scenario snapshot, while the scenario directory currently contains
  36 scripts.
- The deep-spec catalog and filesystem counts are not fully aligned. Use
  [`superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md`](superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md)
  as the catalog, then inspect the directory before adding or renumbering specs.

## Quick Decision Guide

| Question | Read |
|---|---|
| What product are we building? | [`superpowers/BIG-GOAL.md`](superpowers/BIG-GOAL.md) |
| What changed in the Life Node pivot? | [`superpowers/specs/2026-05-19-life-node-pivot.md`](superpowers/specs/2026-05-19-life-node-pivot.md) |
| Which spec governs my implementation? | [`superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md`](superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md) |
| What P0 test should I do next? | [`tdd/INDEX.md`](tdd/INDEX.md) or `./scripts/tdd/tdd-next.sh` |
| How do I run black-box verification? | [`../scripts/phantom-test/README.md`](../scripts/phantom-test/README.md) |
| What is the latest tactical state? | [`../SESSION_RESUME.md`](../SESSION_RESUME.md) |
