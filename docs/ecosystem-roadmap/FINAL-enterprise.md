# FINAL Development Plan — phantom-enterprise

## What it is + current state
`phantom-enterprise` is the on-prem / 台廠-enterprise connector pack of the phantom-mesh
ecosystem: VPN-aware routing, on-prem Git access, LDAP/SSO interfaces, private-code Q&A via a
**local** `phantom exec`, plus deferred MES/ERP/Atlassian/Apple-Silicon-HA hooks. Maturity is
**alpha scaffold — 2 of 7 connectors live** (per the README badge). Real, tested code exists in
`vpn_aware_routing/` (live Tailscale, `tailscale_route()` / `list_peers()`), `on_prem_gitlab/`
(live Gitea over Tailscale, `list_repos` / `list_repo_files` / `get_repo_file`), and `code_qa/`
(repo-context → `phantom exec`). `ldap_sso/auth.py` is three `NotImplementedError` ABCs
(`LdapAuth` / `SamlAuth` / `OidcAuth`); `confluence_jira/`, `mes_connector/`, `erp_connector/`
are README-only placeholders; `apple_silicon_ha/` is a `docs/` runbook. README §"Status" and
`docs/05-phantom-enterprise.md` lock a deliberate strategy: **stay scaffold; activate a
connector only when a real customer / employer defines the target system** — it is the 7th and
intentionally lowest-priority sibling (scheduled M4 W13-14, ~2026-08).

## Guiding constraint (overrides any "implement everything" instinct)
**Don't build mocks against nothing.** Work is in-scope only if it (a) hardens the 2 live
connectors + `code_qa`, (b) improves testability with faked transport (no live server in CI), or
(c) delivers the one documented MVP demo. LDAP/SAML/OIDC, WireGuard/OpenVPN, Confluence/Jira, and
MES/ERP stay **stub or spec-only** until a real target exists — writing integration code against
systems we cannot test just yields fragile, untested abstractions that die on contact with a real
corp AD/ERP.

## Prioritized backlog

- **P1 — Provider-aware on-prem Git** (`on_prem_gitlab/connector.py`): extract an
  `OnPremGitClient` with one shared config/token-header/timeout path; keep Gitea `/api/v1`
  working, add the GitLab `/api/v4` path mapping (the README already names this as the first
  swap). Strict prerequisite for the `code_qa` work below.
- **P1 — Productize `phantom-enterprise ask`** (`code_qa/ask.py`, `context.py`, new `cli.py`):
  context-size caps, include/exclude globs, `--phantom-bin`, `--dry-run-context`, forced
  local-vs-on-prem source, and a citation / empty-context output contract — routed through the
  new client and tested in local + Gitea + GitLab modes. **Sequence after** the Git P1.
- **P1 — VPN-mesh demo milestone** (the single `docs/05` "Must have" demo): a reproducible,
  host-leak-safe scenario proving a phantom node on a corp/Tailscale segment reaches a home-mesh
  node via `vpn_aware_routing`, captured self-hosted like the existing `docs/demo.cast`.
- **P2 — Apple-Silicon-HA runbook → probes** (`apple_silicon_ha/`): turn the runbook into
  testable, fail-soft checks (launchd state, `phantom serve` port, Tailscale IP, `/cluster/peers`
  reachability, failover dry-run). The most testable HA work available **today** on the author's
  own M-series Mac.
- **P2 — `phantom-enterprise status` probe**: one read-only, fail-soft command — Tailscale peer
  state, Gitea/GitLab reachability + version, configured auth backend, HA readiness. Reuses the
  live routing + Git clients; no new external dependency.
- **P3 — LDAP activation SPEC only** (`ldap_sso/auth.py`): write the `LdapConfig` shape, the
  `authenticate()` contract (filter-escaping, normalized `AuthResult`, distinct failure modes:
  bad-credential / unreachable / TLS-config / missing-group), and env-gated integration-test
  instructions — **but keep the ABCs raising `NotImplementedError`** until a real AD/SAML target
  exists.

### Explicitly out of scope (until a real target / customer)
- WireGuard / OpenVPN route providers — speculative; only Tailscale is testable today.
- `confluence_jira` read connector — no corp Atlassian instance to test against.
- "Freeze MES/ERP dataclasses" — README already marks these placeholder-until-employer; a frozen
  schema guessed without real `鼎新 T100` / `鴻海 MES` access is just another untested mock.
- Implementing LDAP/SAML/OIDC `authenticate()` bodies — spec only (see P3).

## Top-3 task breakdown

### P1.1 — Provider-aware on-prem Git
- Add `OnPremGitClient` + provider config (`gitea` → `/api/v1`, `gitlab` → `/api/v4`) with a
  single auth-header / token / timeout path; raise a provider-neutral `OnPremGitUnreachable`
  (alias the existing `GiteaUnreachable` so callers don't break).
- Preserve `list_repos`, `list_repo_files`, `get_repo_file` as thin compatibility wrappers; alias
  `build_gitea_context` → `build_on_prem_git_context` keeping the old name importable.
- Map the per-provider response shapes (Gitea `{"data":[...]}` search vs GitLab list, tree, raw
  endpoints) behind the client so `code_qa` sees one contract.
- Extend tests for both providers plus non-JSON / timeout / HTTP-error paths using faked
  transport; keep the GitLab path config-gated so CI needs no live GitLab.

### P1.2 — Productize `phantom-enterprise ask`
- Add a real `cli.py` entrypoint with flags: context byte/file caps, include/exclude globs,
  `--phantom-bin`, `--dry-run-context` (print selected files + char count, skip the LLM call),
  and explicit `--source local|gitea|gitlab`.
- Route Gitea/GitLab context through the new `OnPremGitClient`; improve file ranking (path hits,
  README/pyproject/config, exact symbol match) while preserving `.gitignore` behavior.
- Lock the output contract: list files used, require citations (prompt already enforces "cite the
  file path"), a dedicated empty-context exit code, and explicit local-only-execution privacy
  wording (bytes never leave the machine).
- Test happy + unreachable-source paths with faked transport; assert **zero** network calls in
  local mode.

### P1.3 — VPN-mesh demo milestone
- Script the demo: a phantom node on a Tailscale/corp segment resolving + reaching a home-mesh
  node via `vpn_aware_routing.tailscale_route()`; use a non-existent / placeholder host so no
  real tailnet IPs leak (match the existing `docs/demo.cast` convention).
- Capture as a self-hosted asciinema cast + a short `docs/` walkthrough (no upload, no
  third-party tracking — consistent with the repo's stated privacy posture).
- Add a smoke test exercising the routing path against mocked `tailscale status --json` output so
  the demo's core logic is CI-verified without a live tailnet.

## Changes from draft
- **Demoted LDAP from P1 → P3 (spec-only).** agy correctly flagged that implementing `LdapAuth`
  now directly violates the README's documented "scaffold until a real AD" strategy (verified in
  `auth.py` — three `NotImplementedError` ABCs — and README §Status). Kept codex's strong
  config/contract design, but as the *spec*, not the implementation.
- **Added the VPN-mesh demo as P1.** Both drafts under-weighted it; agy flagged the omission of
  the one explicit `docs/05` "Must have" demo, and it is achievable with the already-live
  Tailscale connector.
- **Promoted Apple-Silicon-HA probes P3 → P2** (agy): the most testable HA work available today on
  the author's own Mac, far lower-risk than any stub connector.
- **Cut WireGuard/OpenVPN, the Confluence/Jira connector, and the MES/ERP dataclass-freeze** (agy
  scope-creep call, confirmed — those dirs are README-only placeholders with no test target).
- **Kept** codex's codebase-specific breakdowns for provider-aware Git, the `ask` CLI, and
  `status` — these target genuinely-live code with faked-transport tests, and I enforced agy's
  Git→QA sequencing by ordering P1.1 strictly before P1.2.
- **agy's review was usable and accurate.** All five points checked out against the source and
  docs; four were adopted directly and the fifth (Git→QA sequencing) was folded into P1 ordering.
