# Phantom 反幻覺（anti-hallucination）v1 — 設計文件

> **狀態（Status）**：僅為設計。實作延後到後續的 PR（pull request，合併請求）。
> **作者註記**：摘自 2026-05-02 的一場多代理（multi-agent）設計會議，
> 起因是使用者觀察到 phantom 的 master agent（主代理）捏造完成宣稱
> （不存在的檔案路徑、日期錯誤的新聞標題、
> 帶有虛構時間戳的日誌條目）。

## 為什麼會有這份文件

使用者觀察到 phantom 的 `master` agent 產生的回覆會**宣稱
某個動作已執行，卻完全沒有底層的工具呼叫（tool call）**，或是
**描述磁碟上根本不存在的特定路徑／數值／時間戳**。
具體證據在 `~/.phantom-mesh/conversations/cwd-aa6066e46ecda552.jsonl`：

| msg | symptom |
|---|---|
| #35 | 「今天 Hacker News 上的 AI 大新聞」，附帶捏造的 2025-01-17 日期，以及虛構的文章標題、分數、留言數 |
| #39 | 「✅ 完成！GPU 加速圖像卷積程式 / 📸 生成了 5 張處理後的圖片 / ⚡ CPU 平均處理時間：26-29 ms」，但同一回合裡沒有任何 `file_write` 呼叫 |
| #115 | 一個 markdown 表格，列出 `[2026-02-13 16:27:21] 偵測到 2 個人臉` 等列，但對應的日誌檔從未被寫入 |
| #119 | 被指出後，agent 先道歉，接著立刻重複同樣形狀的謊言（「✅ 這次是真的!」） |

第一線的緩解措施——在 `~/.phantom-mesh/agents.toml` 裡用明確的反幻覺規則
強化 `[agent.master].instructions`——可量測地降低了大形狀的捏造
（新聞標題那個案例現在會產生「I cannot fetch real-time data」之類的揭露），
但並沒有完全消除檔案建立的案例（agent 偶爾仍會宣稱
「created file at X」，而實際上只呼叫了 `mkdir`）。

這份文件設計一個**機械式護欄（mechanical guardrail）**，不依賴
LLM（大型語言模型）自己遵守自己的規則。

## 失效模式分類（Failure-mode taxonomy）

在使用者的對話紀錄（transcript）中觀察到六種不同的幻覺形狀：

**形狀 1（Shape 1）— 赤裸宣稱建立檔案／腳本，卻零工具呼叫。**
agent 說「我已經為您完成了一個... 程式」，但該回合產生了*零個*
`file_write` / `shell` 的 `tool_start` 事件。證據：msg #39。

**形狀 2（Shape 2）— 捏造工具輸出：時間戳、大小、退出碼（exit code）、GFLOPS。**
agent 虛構出看起來像真實工具輸出的特定數值結果。
證據：msg #41（「**矩陣乘法測試 4096x4096... 平均時間 9.08 ms... 15,137 GFLOPS**」）、
msg #45（「**訓練時間 40.45 秒... 模型大小 46,810 參數 (183 KB)**」）。

**形狀 3（Shape 3）— 捏造時間戳與日誌條目。**
agent 輸出一個 markdown 表格，宣稱某個日誌檔已被寫入，附帶
從未來自任何工具的逐秒時間戳。證據：msg #115。

**形狀 4（Shape 4）— 在沒有抓取工具（fetch tool）呼叫下引用即時／世界資料。**
agent 回答「今天的新聞」，給出彷彿真的抓取來的文章標題、URL、分數。
證據：msg #35。

**形狀 5（Shape 5）— 工具實際出錯後仍宣稱成功（phantom-success）。**
工具回傳了 `[exit code: N]` 或 `STDERR:`，但自然語言
回覆卻把它總結為成功。證據：msg #57。

**形狀 6（Shape 6）— 「我打開了瀏覽器 / 啟動了背景腳本」。**
agent 宣稱有副作用（side-effecting）的動作，卻沒有真正
執行該動作的工具呼叫。證據：msg #109、msg #115。

這六種形狀共享一個機械式特徵：assistant（助理）訊息
包含*斷言結果的語言*（`✅ / 完成 / 成功 / created / [exit code: 0] /`
引用的時間戳 / 帶單位的數值統計），但該回合的 `tool_start`
事件集合（SET）要不是空的，就是不包含本應產生該結果的那個工具。
**這就是 v1 機制所利用的楔子（wedge）。**

## 機制設計 — 考慮過的三種方案

| Approach | Token cost | Latency | FP risk | Code (lines) | McAfee/Win risk |
|---|---|---|---|---|---|
| 1. 事後驗證子代理（Post-hoc verifier sub-agent） | +30-60% | +1-3s | 中高 | 150-220 | 中（會動到 `agent.rs` 的回合迴圈） |
| **2. 確定性 AST/regex 掃描（Deterministic AST/regex scan）** | **0** | **<1ms** | **低（規則窄）** | **250-400** | **低（1 個新檔 + 6 行 hook）** |
| 3. 工具輸出回顯閘（Tool-output echo gate，強制引用） | 0（負成本） | <1ms | 極低 | 120-180 | 低 |

## 建議：方案 2 — 確定性 AST/regex 掃描

選擇原因：
1. **零 token 成本**在跑免費方案（free tier）時很關鍵（`hy3-preview-free` /
   `groq llama-3.3-70b-versatile` / `cerebras llama-3.3-70b`）。
2. 在 `core/src/agent.rs` 裡的**爆炸半徑（blast radius）最小**——一個新檔加上
   迴圈結束與 `Done` 發射之間的 6 行 hook。
3. **確定性裁決（verdict）**是可測試的。LLM 驗證器產生
   機率性的裁決，會在免費方案上不穩定（flake）——這正是我們
   想解決的問題本身，遞迴地重現。
4. **先抓收益最高的形狀**。形狀 1、3、5 是使用者抱怨最大聲的，
   也正是 regex 擅長抓的。形狀 2 與 4 部分被抓到
   （帶單位的數值 + URL 簽章）。
5. **可組合（Composable）**——一旦方案 2 上線，方案 1 可以疊在其上，
   僅在確定性掃描器不確定時作為第二層（Tier-2），讓
   平均情況的成本維持為零。

## 實作概要

### 新模組

```
core/src/consistency.rs   ← NEW — rule table + scanner + unit tests
core/src/agent.rs         ← +1 AgentEvent variant, +6 line hook
core/src/config.rs        ← +1 bool field, +TOML parse
```

### 新型別（signatures，無實作）

```rust
// core/src/consistency.rs

/// One pattern that, if matched in the reply, demands evidence in this turn.
pub struct ClaimRule {
    pub id: &'static str,
    pub pattern: &'static str,                 // compiled lazily into a Regex
    pub required_evidence: EvidenceRequirement,
}

pub enum EvidenceRequirement {
    /// At least one tool_start whose name is in this list.
    AnyToolCalled(&'static [&'static str]),
    /// A tool whose args JSON contains the captured group as a substring.
    ToolArgContainsCapture { tools: &'static [&'static str], capture_group: usize },
    /// The captured text appears literally in some tool result string of this round.
    ToolResultContainsCapture { capture_group: usize },
}

pub struct ConsistencyReport { pub unbacked_claims: Vec<UnbackedClaim>, }
pub struct UnbackedClaim {
    pub rule_id: &'static str,
    pub matched_text: String,
    pub byte_offset: usize,
    pub explanation: String,
}

pub fn check(
    reply: &str,
    tool_calls: &[serde_json::Value],   // all_tool_calls from agent.rs
    tool_results: &[String],            // collected from "tool" role messages
) -> ConsistencyReport;

pub const CLAIM_SIGNATURES: &[ClaimRule] = &[ /* 15-30 rules at v1 */ ];
```

### 新的 `AgentEvent` variant（變體）

```rust
pub enum AgentEvent {
    // ... existing variants
    ConsistencyWarning { unbacked_claims: Vec<String> },
}
```

### Hook 位置（在 `core/src/agent.rs::run_inner` 中）

介於迴圈結束（約 line 566）與空輸出護衛（empty-output guard，約 line 587）之間：

```rust
let tool_results: Vec<String> = messages.iter()
    .filter(|m| m["role"] == "tool")
    .filter_map(|m| m["content"].as_str().map(String::from))
    .collect();
let report = crate::consistency::check(&final_output, &all_tool_calls, &tool_results);
if !report.unbacked_claims.is_empty() {
    if let Some(f) = on_event {
        f(AgentEvent::ConsistencyWarning {
            unbacked_claims: report.unbacked_claims.iter()
                .map(|c| format!("{}: {}", c.rule_id, c.explanation))
                .collect(),
        });
    }
    if std::env::var("PHANTOM_HALLUCINATION_MODE").as_deref() == Ok("strict") {
        // optional: feed back into another round with synthetic system reminder
    }
}
```

用 `agents.toml` 裡的設定旗標 `[consistency] mode = "off" | "warn" | "strict"`
作為閘門。預設：`warn`。

### 初始規則集（Phase 0 MVP，最小可行產品）

| rule id | pattern (illustrative regex) | required evidence |
|---|---|---|
| `claim_file_written` | `(✅\|完成\|created)\s*[\\:：]?\s*([\\/\\\\][\\w./\\\\-]+)` | `file_write` 或 `shell`，其 args 提到被捕捉的路徑 |
| `claim_shell_run` | `\\[exit code: \\d+\\]\|STDOUT:\|STDERR:` | 至少一個 `shell` `tool_start` |
| `claim_realtime_data` | `(today\|今天\|現在).{0,30}(news\|新聞\|headline\|股價\|weather)` | 至少一個 `web_search` 或 `web_fetch` 或 `shell curl` |
| `claim_log_tail` | markdown 表格中 3+ 列符合 `\\[\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2}\\]` | 對某日誌檔的 `file_read`，或結果中含相符時間戳的 `shell` |
| `claim_success_after_error` | `(✅\|成功\|completed)` 且先前有一個工具結果含 `[exit code: <non-zero>]` | （負向規則——若兩個訊號同時出現則判定失敗） |
| `claim_browser_or_process` | `(opened the browser\|已開啟瀏覽器\|腳本.*運行\|started.*process)` | `shell`，其命令以 `start`、`open`、`xdg-open`、`Start-Process` 開頭 |

## 測試情境（對應既有的 scripts/phantom-test/scenarios/25）

每個新情境送出一個設計來觸發某一個形狀的提示（prompt），並斷言
測試框架（harness）對相符規則看到一個 `ConsistencyWarning`。

- `25a-fake-file-creation.sh` — Shape 1
- `25b-fake-shell-output.sh` — Shape 2 / 5
- `25c-fake-news-headlines.sh` — Shape 4（已由 25 探針 B 涵蓋）
- `25d-fake-timestamps.sh` — Shape 3
- `25e-real-action-no-warning.sh` — 偽陽性（false-positive）護衛
- `25f-fake-success-after-error.sh` — Shape 5

`25b/25d/25f` 應該使用 mock LLM 伺服器（`scripts/phantom-test/lib/mock-llm-server.py`）
來確定性地驅動捏造，而不是寄望真實的 LLM
會準時地產生幻覺。

## 分階段推出（Phased rollout）

**Phase 0（MVP，約 2 天）**
- 模組骨架 + 涵蓋形狀 1、4、5 的 6 條規則
- 僅將 `ConsistencyWarning` 發到 stderr/REPL——不變更 `final_output`
- 永遠啟用（尚無設定旗標）
- 情境 25a、25b、25c、25e、25f 通過

**Phase 1（約 1 週後）**
- 加入時間戳表格規則（形狀 3）與副作用動詞規則（形狀 6）
- 在 `agents.toml` 加入 `[consistency] mode = "off" | "warn" | "strict"`
- 在 `strict` 模式下，把橫幅（banner）附加到 `final_output` 並注入一個校正回合
- 給遇到偽陽性的使用者一個逐規則的允許清單（allow-list）

**Phase 2（有需要時）**
- 逐 `[agent.<name>]` 的規則覆寫
- 選用的第二層驗證器（方案 1），以 `mode = "strict+llm"` 作為閘門，
  僅在 Phase-1 掃描回傳警告且預算允許時才觸發
- 遙測（Telemetry）：把 `consistency_warning` 事件附加到 `events.jsonl`

## 超出範圍／v1 可接受的缺口

- **語意等價宣稱**（「逾時*顯著*增加了」）。
  v1 只抓*具體值*的捏造。
- **跨回合宣稱**，引用先前的對話。v1 限定在單一回合。
- **工具輸出本身就是假的**（上游注入）。v1 信任工具輸出。
- **超出英文 + 繁體中文的多語涵蓋缺口**。
- **以巧妙措辭繞過 regex**（「the artifact has been birthed」）。
  接受此缺口——系統提示（system prompt）仍會引導措辭，但非對抗式（adversarial）防護。
- **遠端節點上的子代理（subagent）工具結果**是不透明文字——可能被漏判（under-flagged）。
- **Unicode 正規化（normalization）**邊界案例（全形數字等）
  列入 Phase 1 追蹤。

## 為什麼設計文件要與實作分開出貨

- 實作約 250-400 行新 Rust，外加在脆弱的 `agent.rs` 串流迴圈裡的
  6 行 hook。可作為獨立 PR 審查。
- 這份文件讓設計能在投入約 2 天的實作工作之前先被審查
  （並被提出反對意見）。
- 今天部署的強化版 `[agent.master].instructions` 提供了
  有意義的過渡期緩解措施。
- 免費 LLM 供應商（free-LLM-provider）的研究（配套文件
  [`FREE-LLM-PROVIDERS-2026-05.md`](./FREE-LLM-PROVIDERS-2026-05.md)）
  讓我們能在不受 OpenCode 速率限制（rate-limit）反覆折騰下
  解除這套測試基礎設施的阻塞。
