# FINAL Development Plan — phantom-quant

## What it is + current state
`phantom-quant` is a Python 3.11 台股 (Taiwan stock) trading-engine package in the phantom-mesh
ecosystem, at P0/pre-alpha. Deterministic offline CSV backtests run end-to-end with a single
`SmaCross` strategy, 台股 fee/tax accounting, Parquet `save_bars`/`load_bars` helpers, and a short
Markdown report, with focused pytest coverage. Gaps: the engine applies every `Order` blindly with
no cash/lot/tick validation and no realistic fill model, `Portfolio.equity` marks a single symbol,
there is no cache/import workflow, no event loop, and no broker path. It is not yet a real data
platform, paper trader, or live system.

## Prioritized backlog

- **P1 — Backtest execution realism** (`backtest/engine.py`, `portfolio.py`): order-status model
  (filled/rejected/skipped) with cash/holdings/`LOT_SIZE`/`tick_size` validation, limit-order fills
  gated on bar high/low actually crossing the limit, explicit fill timing (default current
  fill-at-close) to eliminate lookahead bias, and 台股 ±10% 漲跌停 limit-lock fill blocking.
- **P1 — Data/cache workflow** (`data/store.py`, providers, CLI): a `CachedProvider`/`ParquetProvider`
  over `save_bars`/`load_bars` with schema validation (`ts,symbol,open,high,low,close,volume`),
  `CsvProvider.get_bars` honoring or explicitly rejecting `timeframe`, plus `import-csv` and
  `backtest --cache` so backtests, paper, and live share one validated offline contract. Absorbs
  Shioaji *historical download → Parquet* here (data only, no live routing).
- **P1 — Auditable result artifacts** (`backtest/`, `report.py`): export `trades.csv`/`.parquet`,
  `equity.csv`, run config + metadata, and richer metrics alongside `report.md`, with golden tests.
- **P2 — Multi-symbol portfolio/event model** (`portfolio.py`, engine): mark portfolio equity from
  per-symbol last-known prices instead of the single-current-bar assumption. **Sequenced before
  paper trading** — it is the foundation both paper and live depend on.
- **P2 — Strategy registry/config** (`strategy.py`, CLI): replace hardcoded `_STRATEGIES` dict and
  ad-hoc SMA flags with typed, loadable strategy config + tests.
- **P2 — Event-driven driver/scheduler** (new module): bar-clocked event generator (and tick→bar
  aggregator stub) that replays offline bars today and is the seam paper/live plug into. Prereq for
  any non-backtest mode.
- **P3 — Paper trading mode** (built on P2 multi-symbol + driver): simulated broker/account ledger,
  persistent state, no real-money path; reuses `Strategy.on_bar`.
- **P3 — Shioaji live order gateway** (behind existing `broker` extra, dry-run default): stateful
  websocket order routing only — depends on the P2 driver. (Historical fetch already lives in P1.)
- **P3 — phantom-mesh integration**: emit standardized backtest/paper/live events, reports, and
  portfolio snapshots for sibling mesh services.

## Top-3 task breakdown

### P1 — Backtest execution realism
- Add an `ExecutionReport`/order-status type (filled/rejected/skipped) so the engine stops
  blindly applying every `Order`.
- Enforce cash-on-buy, holdings-on-sell, `LOT_SIZE`, and `tick_size`; gate limit fills on the bar's
  high/low actually crossing the limit; block fills when locked at ±10% 漲跌停.
- Make fill timing configurable with current fill-at-close as default, documenting the no-lookahead
  contract (no acting on a bar's close earlier than it is known).
- pytest: overspend reject, bad lot/tick reject, limit no-cross vs cross, limit-lock block, cost
  invariants.

### P1 — Data/cache workflow
- Add `CachedProvider`/`ParquetProvider` over `save_bars`/`load_bars` with strict schema validation
  on `ts,symbol,open,high,low,close,volume`.
- Make `CsvProvider.get_bars` honor `timeframe` via path/metadata convention or reject unsupported
  values explicitly (no silent wrong timeframe).
- Add `import-csv` and `backtest --cache` CLI commands; fold Shioaji historical-download-to-Parquet
  into this provider contract (data only).
- Tests: malformed CSV, empty range, unsorted/duplicate bars, Parquet round-trip.

### P1 — Auditable result artifacts
- Extend `BacktestResult` with run metadata: symbol, timeframe, cash, strategy params, cost model,
  bar count, date range.
- Export `trades.csv`/`trades.parquet` and `equity.csv` next to `report.md`.
- Expand `report.metrics` to include realized PnL, open exposure, win/loss counts, and ending cash.
- Golden-output tests off `tests/fixtures/sample_2330_1d.csv` so README demo numbers stay
  reproducible.

## Changes from draft
- **Added (from valid agy review):** lookahead-bias resolution + 台股 ±10% 漲跌停 limit-lock and
  limit-cross fill gating folded into P1 execution; an explicit P2 event-driven driver/scheduler
  (no live mode can exist without it).
- **Re-sequenced (agy):** multi-symbol portfolio promoted ahead of paper trading so paper/live build
  on the right foundation; paper trading moved P2→P3 to sit after the driver lands.
- **De-scoped (agy):** the single P3 Shioaji adapter split — historical download pulled forward into
  the P1 data/cache contract, live order gateway kept as a later P3 dependent on the driver.
- **Kept:** the three P1s and the strategy-registry/mesh-integration items, essentially as drafted.
- **agy review usability:** usable and high-quality — five concrete, codebase-specific critiques
  (execution gap, lookahead, 漲跌停, sequencing, over-scope); all five were incorporated.
