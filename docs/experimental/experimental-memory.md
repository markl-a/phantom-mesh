# experimental-memory

**狀態（Status）：** 實驗性（experimental）。預設關閉（Default OFF）。
**Cargo feature（Cargo 功能旗標）：** `experimental-memory`（沒有新的 crate 相依套件 — 沿用既有的 `rusqlite`）。
**發佈（Shipped）：** 2026-05-15 週末衝刺（PR #30）。

## 功能說明

一個為技能型代理人（agent）設計的長期技能記憶儲存庫（skill memory store），底層採用 SQLite FTS5
全文檢索（full-text search）。可插入以 `kind` + `source` + `tags` 標記的
事實（facts）／片段（snippets）／觀察（observations），再透過自由文字檢索搭配 BM25 排序（BM25 ranking）將它們召回（recall）。
資料庫綱要（schema）位於 `core/migrations/0007_hermes_fts5.sql`（檔名保留以維持帳本不可變性（ledger immutability））。

## 公開 API 介面（Public API surface）

- `SkillMemory::open_at(path) -> Result<Self>` — 開啟／建立資料庫（DB），並套用綱要（schema）。
- `SkillMemory::insert(NewMemory) -> Result<i64>` — 插入資料，回傳 rowid（列識別碼）。
- `SkillMemory::get_by_id(id) -> Result<Option<MemoryRow>>`。
- `SkillMemory::search(query, limit) -> Result<Vec<MemoryRow>>` — 以 BM25 排序。
- `escape_fts5_query(raw) -> String` — 把不可信的使用者文字包裝成
  字面片語（literal phrase），讓 FTS5 運算子（`AND`、`OR`、`NOT`、`NEAR`、`*`、`:`、
  括號、引號）無法被惡意利用。

## 如何啟用

```toml
phantom-mesh = { path = "core", features = ["experimental-memory"] }
```

## 快速體驗

```rust,ignore
use phantom_mesh::skillbank::{SkillMemory, NewMemory, escape_fts5_query};

let mem = SkillMemory::open_at("/tmp/skills.db".into())?;
mem.insert(NewMemory {
    kind: "fact", source: "seed",
    text: "rust is a memory-safe language",
    tags: "programming",
}).await?;

let hits = mem.search("rust", 10).await?;
assert_eq!(hits.len(), 1);

// Untrusted text — always escape first（不可信文字 — 務必先跳脫）
let raw = "AND OR NOT NEAR";  // would normally be a parse error
let safe = escape_fts5_query(raw);
let _ = mem.search(&safe, 10).await?;
```

## 執行範例

```bash
CARGO_TARGET_DIR=D:/tmp/skillbank-docs-target \
  cargo run -p phantom-mesh \
    --example experimental_memory_example \
    --features experimental-memory
```

預期的最後一行：`experimental-memory OK`。結束代碼（Exit code）為 0。

## 原始碼（Source）

- `core/src/skillbank/memory.rs` — `SkillMemory`、`NewMemory`、`MemoryRow`、`escape_fts5_query`。
- `core/migrations/0007_hermes_fts5.sql` — 正式綱要（canonical schema）；檔名保留以維持帳本不可變性。

## 安全注意事項（Safety notes）

- 所有插入都使用綁定參數（bound parameters） — 不存在透過使用者文字進行的 SQL 注入（SQL injection）破口。
- 對於來自程式外部的自由文字，若你「不」想要 FTS5 運算子語意，
  在傳給 `search` 之前務必先呼叫 `escape_fts5_query`。
