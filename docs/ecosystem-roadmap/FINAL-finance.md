# FINAL Development Plan — spectyn-finance

> Status: alpha · Tier 1 shipped. Synthesized from the codex draft (which FAILED —
> see "Changes from draft") and a code-verified pass over the actual repo at
> `D:\Projects\spectyn-finance`. Every item below is checked against real source lines.

## What it is + current state

spectyn-finance is the **personal-finance satellite of the spectyn-mesh life-track ecosystem**:
bank-CSV / manual ingest → rule-based zh+en categorizer (LLM hook reserved) → Decimal JSONL
ledger (dedupe-idempotent) → monthly budgets → shame-free monthly markdown report that also
emits a mesh `event` under `~/.spectyn-mesh/events/` for spectyn-companion cross-domain
correlation. Local-first by construction (no cloud; `.gitignore` blocks `ledger.jsonl`/`*.csv`).

**Maturity:** Tier 1 is genuinely complete and tested — `ledger.py`, `budget.py`, `reporter.py`,
`events.py`, `paths.py` (env-overridable: `SPECTYN_FINANCE_HOME` / `SPECTYN_MESH_HOME`), plus a
fully-built TW-bank preset layer (`presets.py`: cathay/ctbc/esun/taishin with ROC dates,
two-column debit/credit, skip-rows). The wiring has **one confirmed dead-code defect** and the
roadmap (LLM router, recurring-charge detection) is unstarted. This is a polish-and-connect
phase, not a greenfield one.

## Prioritized feature/spec list

| Pri | Item | Why / evidence |
|-----|------|----------------|
| **P1** | **Wire `--bank` preset through the CLI** | `cli.py:71` calls `ingest.import_csv(args.csv_path, account=args.account)` — drops `args.bank`. The entire `presets.py` layer is unreachable via CLI. One-line fix + regression test. |
| **P1** | **User rules file (`finance/rules.json`) loaded into the categorizer** | `DEFAULT_RULES` is hardcoded in `categorize.py:16`; `categorize_one`/`apply` already accept `rules=` but nothing loads from disk. Unknown zh merchants can only be fixed by editing Python. Must land **before** the LLM hook so local rules stay deterministic + take precedence. |
| **P2** | **Wire the local-first LLM fallback hook** | `LlmCategorizer` signature is fixed (`categorize.py:55`) but never injected. Wire to the **spectyn-mesh local model router only** (never a remote API by default) — financial privacy is the niche. Local rules win first; LLM only fills `uncategorized`. |
| **P2** | **Recurring-charge / subscription-creep detection** | README Tier-2 goal. Group ledger by normalized merchant + cadence; flag a subscription whose amount rose vs prior occurrence. Emits a `subscription-alert` event (reuse `events.emit`). |
| **P2** | **Ledger-rewrite backup + lock** | `ledger.rewrite` (`ledger.py:94`) is atomic (`tmp.replace(p)`, so agy's "data loss mid-swap" is overstated), but writes no `.bak` and takes no lock — concurrent `recat` + `import` can interleave. Add a `.jsonl.bak` snapshot before swap + a simple lockfile. |
| **P2** | **Real-statement preset validation harness (owner-gated)** | Presets are synthetic-fixture-only by their own admission (`presets.py` docstring). Build a redaction + golden-fixture harness now; actual validation stays **owner-blocked** on real redacted CSVs — do NOT claim presets are field-verified until then. |
| **P3** | **`spectyn skill` top-down query integration** | README Tier-3: answer "這個月外食多少?" via a skill that reads the ledger — the spectyn-mesh user-facing surface. |
| **P3** | **Multi-currency + asset (non-cashflow) accounts** | README Tier-3. `Transaction.currency` exists (defaults `TWD`) but is unused in budget/report math; needs FX + account-type separation. Largest scope; do last. |

## Top-3 task breakdowns

### P1 — Wire `--bank` through the CLI
- In `cli.py` import branch, pass the parsed flag: `ingest.import_csv(args.csv_path, account=args.account, bank=args.bank)`.
- Surface the preset name in the success line (e.g. `imported N txns (esun preset)`); on unknown bank, catch `KeyError` from `presets.get` and exit non-zero with the known-list message.
- Add a CLI-level test that runs `import ... --bank esun` against `tests/fixtures/tw_esun.csv` and asserts ROC-date conversion + skip-rows actually applied (proves the path is live, not just the library).
- Update README quickstart so the documented `--account cathay` example actually uses `--bank`.

### P1 — User rules file loaded into the categorizer
- Add `categorize.load_rules()` reading `finance_home()/rules.json` (`paths.py` pattern), merged **over** `DEFAULT_RULES` so user entries win; missing/empty file → defaults only.
- Thread loaded rules into the `recat` and `import` paths in `cli.py` so disk rules apply on every categorize, not just programmatic calls.
- Add a `spectyn-finance rule add <keyword> <category>` subcommand that writes `rules.json` (keeps users out of Python).
- Tests: user rule overrides a default; unknown merchant + matching user rule categorizes correctly; absent file is a no-op.

### P2 — Local-first LLM fallback hook
- Build a `spectyn-mesh router` adapter implementing `LlmCategorizer = Callable[[str], Optional[str]]`, calling the **local** model endpoint; hard-fail closed (return `None`) if no local router is reachable — never silently hit a remote API.
- Inject it only when explicitly enabled (e.g. `--llm` flag / config), passing `llm=` into `categorize.apply`; ordering already correct (rules → llm → sign fallback in `categorize_one`).
- Cache description→category guesses to a local file to avoid re-querying identical merchants and to keep runs cheap/offline-repeatable.
- Tests with a stub `llm` callable: only `uncategorized` rows reach the hook; a guess is accepted; router-unavailable degrades to `uncategorized` (no crash, no network).

## Changes from draft

- **codex draft was unusable — it produced NO plan.** The run hit a Windows sandbox failure
  (`CreateProcessAsUserW failed: 1312`) before reading a single file, then honestly declined.
  This FINAL is therefore built from a direct code-verified pass, not the draft.
- **agy's review WAS usable and high-quality** — unusually, the "review" was the real source. I
  independently verified all five of its findings against source:
  - **#1 CLI `--bank` not wired (`cli.py:71`)** — CONFIRMED, promoted to the #1 P1 item.
  - **#3 custom-rule persistence** — CONFIRMED, kept as P1 (correct "before LLM" sequencing).
  - **#4 LLM must be local-first** — CONFIRMED as a design constraint, folded into the P2 hook task.
  - **#2 preset real-data validation** — CONFIRMED but **owner-blocked**, so demoted to P2 as a
    harness-now / validate-later item (not a pure-dev deliverable).
  - **#5 ledger-rewrite durability** — partially corrected: the rename **is** atomic
    (`tmp.replace(p)`), so the "crash = data loss" framing is overstated; kept only the valid
    no-backup / no-lock gap as a scoped P2.
- **Added** beyond agy: recurring-charge detection, `spectyn skill` query integration, and
  multi-currency/asset accounts (from the README Tier-2/3 roadmap) to keep a full P1→P3 spine.
