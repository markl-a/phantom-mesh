# Facet ④ — 技能 Loop 安全啟用評估

> 對象:技能庫學習迴圈（curator + memory + extract + synthesize）
> 結論先行:**現在「不要」全面啟用。** 應先把 Phase 1（機器流量隔離）落地，並把整體啟用 **gate 在 partner 真實使用之後**,對齊 `ACCEL-FRAMEWORK §0.1`。閉環(`closes_loop=true`)成立,但目前缺三道安全閘:來源隔離、人類審核、event store 一致性。

---

## 0. TL;DR

| 問題 | 答案 |
|---|---|
| 閉環成立嗎? | **是**(萃取→存→召回→合成→寫回 memory 全鏈路存在) |
| 預設啟用嗎? | **否**,全部 feature 預設 OFF(`default = []`) |
| event store 真的有寫嗎? | **有寫**,但 FTS5 索引是 best-effort、無重試,可能與密文 body 失同步 |
| 萃取-存-用 哪段斷? | 斷在 **「存」之前無人審 + 「用」時無來源守門**;另有 FTS5 索引失同步的隱性斷點 |
| 護城河誠實帳本受影響嗎? | **是**,這是最高層級風險:機器流量混入 memory 會讓「自己軌跡」這條護城河變成自我汙染的數字 |
| 現在啟用 or 等 partner? | **等 partner 真用之後**,Phase 1 先行、其餘 gate 住 |

---

## 1. 現況盤點

### 1.1 Feature gates(預設狀態)

證據:`core/Cargo.toml` line 68–133。

```
default = []                         # 全部 OFF
experimental-skillbank = 傘狀 feature   # 組合 curator + memory + tools
```

啟用此 loop 需要的 features:

- `experimental-curator` — 集成裁決(ensemble verdict)
- `experimental-memory` — FTS5 後端 memory 存取
- `experimental-tools` — 工具側接線
- `experimental-extra-providers` — provider 路由
- `experimental-skillbank`(傘狀)— 一鍵組合上述

**判讀:** gate 設計本身是健康的(全部 OFF + 傘狀 feature),代表這條 loop 還在實驗區、未承諾給使用者。這給了我們安全 rollout 的空間 — **不要一次打開傘狀 feature。**

### 1.2 Event store 真的有寫嗎?

**有寫,但有隱性斷點。**

- `event_storage_wire.rs:251 write_event` — 真實寫入 `events/<uuid>/` 密文 body。
- `event_storage_wire.rs:603 index_fts5` — 寫 FTS5 索引,但為 **best-effort、失敗只記 diagnostics、無重試**(line 603–619)。
- 後果:**body 寫成功但 FTS5 索引缺失 → 該事件永遠搜不到。** 這不是「沒寫」,而是「寫了但召回端永遠看不到」,比沒寫更難察覺。
- 旁證:`count_tokens_pseudo` 用 naive 空白切詞,與 FTS5 `unicode61` tokenizer 分歧(低風險、僅 telemetry,但會影響 empty-summary 啟發式)。

這與既有記憶 [🔴 事件 store 對不上](finding_event_store.md) 一致 — 讀端可能永遠空的疑慮,在這裡找到了一個具體機制根因(index 失同步)。

### 1.3 萃取→存→用 哪一段斷?

```
[萃取] extract.rs ──► [存] SkillMemory ──► [用] recall / synthesize
   A1 routes              memory.rs              integration.rs
   ▲ 斷點①               ▲ 斷點②                ▲ 斷點③
   無人審 gate           無來源欄位             召回無來源守門
```

- **斷點①(存之前無審核):** `extract.rs:52–146` 雙 API(success / failure 以 score≥threshold 二分),`MIN_CONFIDENCE_SIGNALS=1` 過於寬鬆 — 單一弱訊號(1 次 dead end)就變成 memory 裡的「lesson learned」。**插入前無任何 gating。**
- **斷點②(存的當下無 provenance):** `memory.rs:1–200` FTS5 後端,`search_by_kind` / `list_by_kind` **無 origin/source 過濾**;`event_storage_wire.rs` 的 capture 也沒有 origin 欄位 — 人類與機器 capture 一視同仁走同一條 `write_event` + `index_fts5`。
- **斷點③(用的時候無守門):** `integration.rs:32–141` `SkillbankRuntime` seed catalog+sample,但 recall **接受所有 memory rows、不依 origin 過濾**。`synthesize.rs:187–220` propose→verify→iterate 把「verified skills」直接寫回 memory,**無 pre-storage review。**

**最關鍵的不一致:** `extract.rs A8` 雙 API(success≥5 / failure<5)與 Curator Ensemble V2(unanimous / consensus / needs_review)**可以彼此打架** — 同一個 session 在 memory 裡得到不一致的技能極性(同時是成功與失敗)。Custody 模型缺失:技能可在裁決存在之前就被寫入。

---

## 2. 風險矩陣

| # | 風險 | 層級 | 影響面 | 證據 |
|---|---|---|---|---|
| R1 | **機器流量汙染 learning loop** | 🔴 致命 | 護城河/誠實帳本 | `partner.rs` 已隔離 bot 到 `.machine.jsonl`(2026-06-05 fix),但 `event_storage_wire.rs` **無 origin check**;`capture_habit_wire.rs:~140` 的 `write_event` 也沒傳 `MessageOrigin` |
| R2 | **SkillMemory 召回無來源守門** | 🔴 致命 | 護城河 | `memory.rs` search/list 回傳全部 rows;自治 loop 寫的 memory 與人類技能無法區分,recall/synthesis 會 **不自知地放大 bot pattern** |
| R3 | **低品質萃取汙染 memory** | 🟠 高 | 技能品質 | `extract.rs` `MIN_CONFIDENCE_SIGNALS=1`,無人審即 route 到 SkillMemory |
| R4 | **Curator 裁決 vs 萃取路由不一致** | 🟠 高 | 技能極性 | `extract.rs A8` 雙 API 與 Ensemble V2 可分歧 → 同 session 不一致極性 |
| R5 | **FTS5 索引與密文 body 失同步** | 🟠 高 | 召回完整性 | `event_storage_wire.rs:603–619` index best-effort、無重試、只記 diagnostics |
| R6 | **confidence floor 過低** | 🟡 中 | 技能雜訊 | `MIN_CONFIDENCE_SIGNALS=1`:1 次網路抖動 = 1 dead end = 永久「lesson」技能 |
| R7 | **FTS5 token 估算分歧** | 🟢 低 | telemetry | `count_tokens_pseudo` naive split vs `unicode61` tokenizer |

**風險集群解讀:** R1+R2 是同一個根因(provenance 缺失)的「寫」與「讀」兩面,合起來直接攻擊護城河。R3+R4+R6 是同一個根因(無 pre-insertion gating + 過低門檻)。R5+R7 是 event store 的工程一致性問題。三個根因 → 三個 rollout 階段對應修。

---

## 3. 啟用所需 features(逐一)

| Feature | 作用 | 安全啟用前提 |
|---|---|---|
| `experimental-memory` | FTS5 memory 讀寫 | **必須先有 origin 欄位 + 召回後過濾**(否則 R2 立即觸發) |
| `experimental-curator` | Ensemble 裁決 | 需與 extract 路由統一(R4),否則寫入不一致極性 |
| `experimental-tools` | 工具接線 | 相對安全,但依賴 memory |
| `experimental-extra-providers` | provider 路由 | 安全,獨立性高 |
| `experimental-skillbank`(傘狀) | 一鍵全開 | **禁止在 Phase 4 完成前打開** |

---

## 4. 分階段安全 Rollout

> 原則:**實驗 flag → 限額(來源隔離)→ 人類審 skill → 才採用。** 每階段有可驗證的退出條件,不過關不進下一階段。

### Phase 1 — 機器來源隔離(立即,修 R1/R2)

- 加 `MessageOrigin` tag 到 `capture_note` / `capture_habit` / `write_event`。
- `event_storage_wire` 的 `index_fts5` 對 machine-origin **skip 或標記**。
- 技能記憶（skill memory）召回端 **post-filter `source != 'machine_*'`**。
- **退出條件:** bot loop 隔離測試通過 — 自治 loop 寫入的 memory 不出現在人類 recall 結果。
- **為何最先:** 這是唯一直接保護護城河誠實帳本的閘,且 `partner.rs` 已建立 `MessageOrigin` enum(line 47–69)可複用,成本最低。

### Phase 2 — 萃取前人類審核 gate(week 1,修 R3/R6)

- `SkillCandidate` 加 `pending_review` 狀態;**未 approve 不寫 SkillMemory**。
- CLI:`phantom skill list --pending` / `phantom skill approve <id>`。
- Curator V2 **僅 unanimous(stddev==0)自動 approve**,其餘進人審佇列。
- **退出條件:** 人審佇列可運作,且預設無人審不入庫。

### Phase 3 — Event store 一致性檢查(week 2,修 R5)

- `phantom data rebuild-fts5`:重掃 `events/<uuid>/` 重建索引(recovery)。
- `coach_wire` 每日 health check:比對 rows 數 vs dirs 數,**divergence > 5% 告警**。
- **退出條件:** 跑一次 rebuild + 連續健康檢查無告警。

### Phase 4 — Curator 與萃取對齊(week 3–4,修 R4)

- 統一路由為 ensemble verdict — **同一 checkpoint 不可同時產出 success+failure**。
- Custody 模型:**裁決必須先存在,技能才可寫。**
- unanimous 給 confidence boost。
- **退出條件:** 同 session 不再出現極性衝突;此後才可打開傘狀 `experimental-skillbank`。

---

## 5. 與護城河誠實帳本的關係

依競爭 gap 分析,phantom 的差異化護城河是 **「雙向技能 + 跨機 mesh + 真 eval 數字」**,其核心資產是 **「自己的軌跡」**。

> **這條 loop 若在 R1/R2 未修前啟用,等於親手汙染護城河。**

- 機器流量(自治 4 機開發 loop 本身)會被當成「人類技能/軌跡」寫進 memory。
- recall/synthesis 會放大 bot pattern,讓「我們學到的技能」這個數字 **無法區分是人在用還是 bot 在自我餵食**。
- 對外宣稱「基於自己真實軌跡的學習迴圈」就會變成 **不誠實的帳本** — 這直接牴觸記憶中反覆出現的「反假綠/誠實邊界」紀律。

因此 **Phase 1 的來源隔離不是優化、是護城河的前置條件。** 沒有 provenance,這條 loop 產出的任何 eval 數字都不可信。

---

## 6. 建議

### 6.1 現在啟用 or gate 在 partner 真用之後?

**Gate 在 partner 真實使用之後,對齊 `ACCEL-FRAMEWORK §0.1`。**

理由:
1. **§0.1 gate 的精神** = 先有真實使用訊號,再投資自動化深度。技能 loop 沒有真實 partner 使用流量,萃取出的「技能」是無源之水(目前主要訊號來自開發 bot 自身 → 正是 R1)。
2. 閉環雖成立,但三道安全閘(來源/人審/一致性)缺席,**現在全開 = 主動製造護城河汙染 + 不可信 eval**。
3. feature 預設 OFF 的設計給了我們不付代價的等待空間。

### 6.2 具體行動序列

| 時機 | 動作 |
|---|---|
| **現在** | 落地 **Phase 1**(來源隔離),但 **不啟用** `experimental-memory` 給人類使用路徑。可在隔離測試環境開 `-providers` / `-tools` 做煙霧測試。 |
| **partner 開始有真實使用流量** | 啟用 `experimental-memory`(此時 Phase 1 已保證隔離),搭配 Phase 2 人審 gate。 |
| **Phase 3 通過** | 啟用 `experimental-curator`。 |
| **Phase 4 通過** | 才允許打開傘狀 `experimental-skillbank`,宣布 loop GA。 |

### 6.3 不要做的事

- ❌ 不要為了「閉環看起來會動」就打開傘狀 feature。
- ❌ 不要在 Phase 1 前用任何 loop 產出的技能數字當護城河證據。
- ❌ 不要把 `MIN_CONFIDENCE_SIGNALS=1` 帶進生產 — 至少提到 ≥2 並要求人審。

---

## 7. 一句話總結

**閉環已成立、gate 設計健康,但 provenance(R1/R2)是護城河前置條件、人審(R3/R4/R6)是品質前置條件、FTS5 一致性(R5)是召回前置條件 — 三者皆缺。建議:Phase 1 立即做、整體啟用 gate 在 partner 真用之後,嚴格遵循「實驗 flag → 來源限額 → 人審 skill → 才採用」的四階段。**
