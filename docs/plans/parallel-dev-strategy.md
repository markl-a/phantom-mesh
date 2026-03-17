# 並行開發策略

> 生效日期：2026-03-16
> 截止日期：2026-03-27（功能完成）→ 2026-03-31（商業化上線）

## 架構原則

### 衝突瓶頸檔案（只有 Session A 可碰）

| 檔案 | 原因 |
|------|------|
| `src/main.rs` | tool/module 註冊入口 |
| `src/tools/mod.rs` | `pub mod xxx;` 宣告 |
| `src/lib.rs` | `pub mod xxx;` 宣告 |
| `Cargo.toml` | dependency 管理 |

### 零衝突檔案（各 Session 自由寫）

| 路徑模式 | 原因 |
|----------|------|
| `src/tools/<name>.rs` | 每個 tool 獨立檔案 |
| `src/providers/<name>.rs` | 每個 provider 獨立檔案 |
| `~/.clawtex/hands/<name>/hand.toml` | 每個 hand 獨立目錄 |
| `src/<new_module>.rs` | 新系統模組獨立檔案 |
| `docs/**` | 文件互不干涉 |

---

## 4 Session 分工

### Session A — Core Integration（Z13, master branch）

**唯一可碰共用檔案的 session。** 負責：
- `main.rs` tool/module 註冊
- `tools/mod.rs`, `lib.rs` module 宣告
- `Cargo.toml` dependency
- `agent_runtime.rs` 改進
- `cluster_hub.rs` 改進
- provider 系統改進
- merge 其他 branch + 全量 `cargo test`

**交付物：**
- agent_runtime 改進（LoopDelegate, SQ/EQ pattern）
- cluster_hub 改進（SLA Priority Queue, Idempotency, Task Taxonomy）
- provider 改進（UnsupportedParam filter, retry 策略）
- 每日 merge + 統一註冊 B/D 的新 code

### Session B — New Tools（Z13, `feat/tools` branch）

**只寫 `src/tools/<name>.rs`，不碰 mod.rs/main.rs。**

待實作 tools：
| Tool | 檔案 | 用途 |
|------|------|------|
| `image_generate` | `src/tools/image_generate.rs` | AI 圖片生成 |
| `docx_export` | `src/tools/docx_export.rs` | Word 文件匯出 |
| `xlsx_export` | `src/tools/xlsx_export.rs` | Excel 匯出 |
| `tts` | `src/tools/tts.rs` | 文字轉語音 |
| `email_receive` | `src/tools/email_receive.rs` | IMAP 收信 |
| `video_compose` | `src/tools/video_compose.rs` | 影片合成 |
| `youtube_upload` | `src/tools/youtube_upload.rs` | YouTube 上傳 |
| `music_generate` | `src/tools/music_generate.rs` | AI 音樂生成 |
| `knowledge_import` | `src/tools/knowledge_import.rs` | 知識庫匯入 |
| `linkedin` | `src/tools/linkedin.rs` | LinkedIn 操作 |
| `search_console` | `src/tools/search_console.rs` | Google Search Console |
| `engagement_tracking` | `src/tools/engagement_tracking.rs` | 互動追蹤 |

每個 tool 完成後通知 Session A 做註冊。

### Session C — Hands + Config（Acer, `feat/hands` branch）

**只寫 hand TOML 和文件，不需要 Rust 編譯。**

待實作 hands：
| Hand | 路徑 |
|------|------|
| `youtube` | `~/.clawtex/hands/youtube/hand.toml` |
| `report` | `~/.clawtex/hands/report/hand.toml` |
| `novel` | `~/.clawtex/hands/novel/hand.toml` |
| `design` | `~/.clawtex/hands/design/hand.toml` |
| `comic` | `~/.clawtex/hands/comic/hand.toml` |
| `ecommerce_ops` | `~/.clawtex/hands/ecommerce_ops/hand.toml` |
| `music` | `~/.clawtex/hands/music/hand.toml` |
| `game_dev` | `~/.clawtex/hands/game_dev/hand.toml` |
| `micro_saas` | `~/.clawtex/hands/micro_saas/hand.toml` |

另外負責：
- agents.toml 調優
- 所有文件更新
- README / 使用者指南

### Session D — New Modules（Z13, `feat/modules` branch）

**只寫 `src/<module>.rs`，不碰 lib.rs/main.rs。**

待實作 modules：
| Module | 檔案 | 用途 |
|--------|------|------|
| Knowledge Capture | `src/knowledge_capture.rs` | 知識擷取系統 |
| Context Pack | `src/context_pack.rs` | 上下文壓縮 |
| Knowledge Graph | `src/knowledge_graph.rs` | 知識圖譜 |
| Observational Memory | `src/observational_memory.rs` | 觀察式記憶 (Mastra pattern) |
| Condenser | `src/condenser.rs` | 記憶壓縮 Pipeline |
| Skills System | `src/skills.rs` | 可組合技能系統 |
| Governance Rules | `src/governance.rs` | 治理規則引擎 |
| Injection Guard | `src/injection_guard.rs` | 注入攻擊防護 |
| RBAC | `src/rbac.rs` | 角色權限控制 |
| Audit Log | `src/audit.rs` | 審計日誌 |
| Tiered Approval | `src/tiered_approval.rs` | 分級審批 |
| Service Tier | `src/service_tier.rs` | 服務分級 |
| Pipeline Metrics | `src/pipeline_metrics.rs` | 管線指標 |
| Error Codes | `src/error_codes.rs` | 統一錯誤碼 |
| Cost Budget | `src/cost_budget.rs` | 成本預算控制 |
| Usage Meter | `src/usage_meter.rs` | 用量計量 |

每個 module 完成後通知 Session A 做註冊。

---

## 每日流程（台灣時間 UTC+8）

```
09:00  各 Session 開工，從各自 branch 開始
       B/C/D rebase 到最新 master

12:00  午間 sync（可選）
       B/D 完成的 module 通知 A

18:00  Session A 開始 merge
       1. merge feat/tools → master（解決 mod.rs 衝突）
       2. merge feat/modules → master（解決 lib.rs 衝突）
       3. merge feat/hands → master（通常無衝突）
       4. 統一做 tool/module 註冊
       5. cargo test 全跑
       6. push master

19:00  B/C/D rebase 到最新 master
       確認 build 正常後繼續
```

---

## Branch 規則

```bash
master          ← Session A 直接在此開發 + merge
feat/tools      ← Session B
feat/hands      ← Session C
feat/modules    ← Session D
```

- **禁止** B/C/D 直接 push 到 master
- **禁止** B/C/D 碰瓶頸檔案（main.rs, mod.rs, lib.rs, Cargo.toml）
- B/D 每完成一個 tool/module 就 commit，不要積壓
- A 每天 18:00 統一 merge + 註冊 + 測試

---

## 機器分配

| 機器 | Session | 原因 |
|------|---------|------|
| Z13 (64GB, Ryzen AI MAX+) | A, B, D | Rust 編譯需要高 CPU/RAM |
| Acer | C | Hand TOML + 文件不需要編譯 |
| AYANEO | 備用 | NPU 對編譯無幫助，作為測試機 |

Z13 同時跑 3 個 Claude Code session 沒問題（64GB RAM）。

---

## 風險控管

| 風險 | 機率 | 對策 |
|------|------|------|
| merge 衝突 | 低 | 只有 A 碰共用檔 |
| Z13 記憶體不足 | 低 | 64GB 夠 3 session + cargo build |
| B/D 完成但 A 來不及註冊 | 中 | A 優先做 merge，自己的開發放後面 |
| 某 tool 依賴未加的 crate | 中 | B/D 先寫不含外部 crate 的版本，A merge 時加 |
| Acer 網路斷線 | 低 | C 的工作可以離線做，之後 push |

---

## 完成定義

- **3/27**: 所有 tool/module/hand 實作完成 + 註冊 + cargo test 全過
- **3/31**: Multi-tenant API + Stripe 訂閱 + Landing Page + 對外上線
