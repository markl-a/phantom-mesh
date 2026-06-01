# 效能基準 — 週末衝刺（2026-05-15）

> **狀態：** 基準擷取（baseline capture，基準量測）。此處數字為*首次量測值*，
> 並非目標值。未來的效能退化（regression）應以這些
> 中位數（median）為對照基準量測；每當任何底層
> 功能（F1 / H3 / H5）有實質修改時，都應重新擷取數字。

## 硬體與環境

- Host（主機）: Windows 11 Home (10.0.26200), x86_64-pc-windows-msvc
- Rust toolchain（工具鏈）: `rustc 1.93.1 (01f6ddf75 2026-02-11)`
- Criterion: 0.5 (`html_reports`, `async_tokio`)
- `CARGO_TARGET_DIR`: `D:/tmp/perf-benchmarks-target`
- Build profile（建置設定檔）: `bench`（release + LTO=thin，依工作區預設值）
- 從乾淨狀態重跑？是 — 於 2026-05-15 對每個 binary 全新執行 `cargo bench`。

## 主要數字

| Bench | Median（中位數） | 95% CI（信賴區間） | Source（來源） |
|---|---|---|---|
| Anthropic cache MISS（wiremock 來回） | 5.8741 s | [4.4730, 7.4729] s | `anthropic_cache_hit_vs_miss/miss` |
| Anthropic cache HIT（wiremock 來回） | 3.5214 s | [2.9982, 4.1626] s | `anthropic_cache_hit_vs_miss/hit` |
| FTS5 單列插入（全新 DB） | 3.4425 ms | [3.3216, 3.5678] ms | `fts5_memory_insert/single_row` |
| FTS5 單詞 BM25 查詢（1 萬列） | 681.15 µs | [622.56, 738.10] µs | `fts5_memory_query/single_term_bm25_10k_rows` |
| Hermes 工具查找 — first | 7.5036 ns | [6.9802, 7.9152] ns | `hermes_tool_lookup/first` |
| Hermes 工具查找 — last | 34.520 ns | [33.677, 35.299] ns | `hermes_tool_lookup/last` |
| Hermes 工具查找 — miss | 22.937 ns | [22.378, 23.510] ns | `hermes_tool_lookup/miss` |
| Hermes 工具分派（calculator） | 639.43 ns | [616.36, 662.27] ns | `hermes_tool_dispatch/calculator_simple_expr` |

## 每個數字的意義

### F1 — Anthropic prompt cache（提示快取，PR #31）

此處的 cache MISS 與 HIT 數字受 wiremock 綁定 — 它們量測的是
透過 `stream_agent_full` 端到端的本地 SSE 解析速度，而非
真實的 Anthropic 來回往返。此 bench 的*真正*價值在於 setup 階段的
斷言（assertion）：它證明 `StreamResult.cache_read_input_tokens` 在
預先準備好的 hit 回應上有正確填入 > 0，這是我們在沒有 API key 的
情況下唯一能驗證的 F1 合約（contract）片段。

**發現：** wiremock 的牆鐘時間（wall-time）之所以每次呼叫達數秒之久，是因為
wiremock 伺服器在送出預設的 SSE body 後乾淨地關閉連線，而
`stream_agent_full` 在回傳前會執行重連重試（`MAX_RECONNECT_ATTEMPTS = 2`，
每次延遲 500 ms）。HIT 情況（約 3.5 s）比 MISS（約 5.9 s）更快，
主要是因為重連時序的隨機性，而非解析成本。真實 API 的數字將會
截然不同（單次 TCP 來回、無重試迴圈）— 此 bench 的價值在於 cache
token 計數器的斷言，而非牆鐘時間數字本身。

### H3 — FTS5 記憶體後端（memory backend，PR #30）

插入吞吐量（throughput）：約 290 inserts/sec（1 / 3.44 ms）。低於
PR #30 宣稱的「1000 inserts/sec 目標」。根本原因是單列插入搭配
SQLite 預設 journal 模式（rollback journal + 每次插入皆 full
fsync）。此 bench 每次迭代（iteration）建立一個全新的 tempdir DB
（透過 `iter_batched`），因此我們量測的是穩態（steady-state）插入延遲，而非
攤銷（amortised）後的成長。

**緩解方式：** 將插入包進單一交易（transaction）中（或
啟用 `PRAGMA journal_mode=WAL`）是針對此問題的標準 SQLite 解法。
此 benchmark 刻意量測未批次化（unbatched）的路徑，因為那正是
`HermesMemory::insert` 今天所暴露的介面。後續 PR 可加入
`insert_many(&[NewMemory])` 以採用單一交易；預期可獲得
10–100× 的吞吐量提升。

**推導出的 inserts/sec：** ≈ 290 inserts/sec（單列、磁碟上、預設 journal）。
**推導出的 queries/sec：** ≈ 1,469 queries/sec（1 萬列上的單詞 BM25）。

在一個 1 萬列語料庫（corpus）中、搜尋詞命中約 10% 列的情況下，
681 µs 的 BM25 查詢延遲，可代表 agent 活動數天後真實的長期
記憶體儲存的表現。次毫秒（sub-millisecond）意味著
「對任何互動式呼叫者而言都快得綽綽有餘」。

### H5 — Hermes 工具目錄（tool catalog，PR #27）

此目錄是一個含 10 個項目的 `Vec<Box<dyn HermesTool>>`。`first`、
`last`、`miss` 全都執行相同的線性掃描（linear scan），但掃描深度不同。

**發現：** 三個數字皆如預期落在奈秒（nanosecond）帶內，
但其分散程度超出預測：
- `first`（7.5 ns）比 `last`（34.5 ns）顯著更快 —
  掃描所做的比較次數較少。
- `miss`（22.9 ns）落在 `first` 與 `last` *之間*。對於 10 個項目，
  這看似不尋常，但它反映出 `last` 執行了約 10 次 vtable
  呼叫加上對一個 16 位元組字面值（literal）的最終字串相等比較，而 `miss`
  執行了約 10 次 vtable 呼叫加上 10 次在第一個字元上短路（short-circuit）的
  字串相等比較失敗。分支預測（branch-prediction）與 L1d 區域性（locality）
  完成了其餘的事。

實務上的結論：在約 35 ns 的最壞情況下，線性掃描是免費的。
無需引入 HashMap、FNV 或完美雜湊（perfect-hash）目錄；
整個分派（`bench_hermes_tool_dispatch` = 639 ns）的時間主要由
`serde_json::Value` 擷取以及 async-trait `.call().await`
開銷所主導，而非名稱比對。

## 方法說明

- 每個 Criterion bench 暖機（warm up）1 s 並量測 2 – 3 s，採用
  預設的 `sample_size = 50`（anthropic bench 降為 30，因為
  每次迭代耗時數秒）。
- 非同步（async）bench 在每個 bench binary 上以單一共享的
  `tokio::runtime::Runtime` 執行；該 runtime 透過
  `b.to_async(&rt)` 或 `iter_batched` 內的 `rt.block_on(...)` 排除在計時主體之外。
- FTS5 查詢 bench 透過 `OnceLock` 在所有迭代間共享一個 1 萬列儲存，
  因此每次迭代的 setup 成本為零。1 萬列的種子（seed）
  在首次執行時約耗 30 s；同一行程（process）內後續的 bench
  則重複使用該靜態快取。
- FTS5 插入 bench 使用 `iter_batched`，每次迭代搭配一個全新的 tempdir DB，
  因此我們量測的是穩態（而非攤銷後）的插入延遲。
- Anthropic wiremock bench 基於相同理由，在各迭代間快取其 mock
  伺服器 URL — 否則 TCP listener 啟動將會主導時間。
- 每個 bench 中 setup 階段的斷言（cache tokens > 0、calculator
  結果 == 14、FTS5 search 非空）扮演冒煙測試（smoke test）的角色 — Criterion
  沒有斷言 API，因此我們在 `c.benchmark_group(...)` 之前於 harness 函式
  內執行它們，以便在已接線的合約漂移（drift）時快速失敗（fail fast）。

## 如何重現

從 `feat/perf-benchmarks` worktree（`core/` 為 crate 根目錄）：

```powershell
$env:CARGO_TARGET_DIR = "D:/tmp/perf-benchmarks-target"
cd core
cargo bench --bench anthropic_cache
cargo bench --bench fts5_memory_insert --features experimental-hermes-memory
cargo bench --bench fts5_memory_query  --features experimental-hermes-memory
cargo bench --bench hermes_tool_lookup   --features experimental-hermes-tools
cargo bench --bench hermes_tool_dispatch --features experimental-hermes-tools
```

Bash 等效寫法：`export CARGO_TARGET_DIR=D:/tmp/perf-benchmarks-target`，
然後從 `core/` 子目錄執行。

詳細的 HTML 報告會輸出於
`$CARGO_TARGET_DIR/criterion/<bench-name>/report/index.html`。

若要在 GitHub Actions 內重跑：開啟 **Actions** 分頁 → **Perf
Baseline (manual)** → **Run workflow**。此工作流程會將
`target/criterion/**` 產物（artifact）以 30 天保存期的 artifact 形式提供下載。

## 真實 API 快取驗證（手動，不在 CI 內）

wiremock bench 無法證明*真實*的 Anthropic prompt caching
確實能省錢。要手動驗證這點：

1. 將 `ANTHROPIC_API_KEY` 設為一把真實的 key。
2. 以相同的前約 5 KB 指令，連續執行任一 `phantom evolve`
   工作階段（session）兩次。
3. 檢查第二次執行的 `tracing` 輸出中是否有 `cache_read_input_tokens=...`。
   非零值即確認快取斷點（cache breakpoint）正在運作。

來自該手動執行的數字，應在收集完成後以後續章節形式
附加到本檔案中。
