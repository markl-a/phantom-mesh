# Clawtex 持續營利自動化引擎 (Continuous Revenue Automation Engine)

> 設計日期: 2026-03-05
> 模組: `src/revenue_engine.rs`
> 依賴: revenue_tracker, cost_tracker, cron, hands, scheduler, telegram

---

## 目錄

1. [系統總覽](#1-系統總覽)
2. [Revenue Optimization Loop (營收優化迴圈)](#2-revenue-optimization-loop)
3. [每日自動化排程](#3-每日自動化排程)
4. [營收再投資算法](#4-營收再投資算法)
5. [失敗恢復機制](#5-失敗恢復機制)
6. [增長策略自動執行](#6-增長策略自動執行)
7. [Dashboard 設計](#7-dashboard-設計)
8. [Rust 結構設計](#8-rust-結構設計)
9. [決策流程圖](#9-決策流程圖)
10. [實施計劃](#10-實施計劃)

---

## 1. 系統總覽

### 核心理念

整個引擎的運作邏輯如下:

```
             +------------------+
             |   Cron Scheduler |
             |  (30s tick loop) |
             +--------+---------+
                      |
          +-----------+-----------+
          |                       |
    [每日排程執行]          [優化迴圈觸發]
          |                       |
    +-----------+         +-------+--------+
    | HandRunner|         | RevenueEngine  |
    | (13 Hands)|         | (分析+決策)     |
    +-----------+         +-------+--------+
          |                       |
    +-----+-----+        +-------+--------+
    |  Tool 執行 |        |  ROI 計算器    |
    | (24 Tools) |        |  排程調整器    |
    +-----+-----+        |  告警系統      |
          |               +-------+--------+
    +-----+-----+                |
    | revenue_   |        +------+-------+
    | tracker    |<-------| 自動調頻     |
    | cost_      |        | (cron CRUD)  |
    | tracker    |        +--------------+
    +-----------+
```

### 設計原則

1. **資料驅動決策**: 所有調整都基於 SQLite 中的真實營收/成本數據
2. **保守啟動, 激進優化**: 初始使用固定排程, 有數據後逐步自動調整
3. **人類在迴圈中**: 重大決策 (停用路線、大額支出) 必須經過 Telegram 批准
4. **故障安全**: 任何異常觸發降級而非停機, E-Stop 仍然可用

---

## 2. Revenue Optimization Loop

### 2.1 ROI 計算模型

每條路線的 ROI 計算:

```
ROI(route) = (revenue_7d(route) - cost_7d(route)) / cost_7d(route)

其中:
  revenue_7d = 過去 7 天該路線的 confirmed+paid 營收總額
  cost_7d    = 過去 7 天該路線相關 hands 的 LLM 成本總額
```

路線到 Hands 的映射:

| 路線 | 主要 Hands | 補充 Hands |
|------|-----------|-----------|
| A: freelance_dev | freelancer | - |
| B: saas_products | product_spec, code_gen, saas_deploy | - |
| C: content_monetization | seo_content, content | - |
| D: consulting | market_intel, lead | outreach |
| E: api_services | code_gen, saas_deploy | - |
| F: affiliate_marketing | seo_content, content | - |
| G: digital_products | product_spec, content | - |
| H: automation_services | lead, outreach | market_intel |
| I: data_services | market_intel | - |
| J: training_education | content, seo_content | - |

### 2.2 優化決策邏輯

```
每日 22:00 執行 (self_optimize hand):

1. 拉取過去 7 天各路線 ROI
2. 排名: top_routes = ROI > 0 的路線, 按 ROI 降序
3. 排名: dead_routes = 連續 3+ 天營收為 0 的路線
4. 排名: bleeding_routes = ROI < -1.0 (花了 $1 但虧超過 $1) 的路線

決策:
  IF top_routes 不為空:
    - 最賺錢的路線: 頻率 x1.5 (每日 -> 每日兩次, 或時段擴展)
    - 第二賺錢: 維持當前頻率
    - 其餘: 頻率 x0.75 (減少但不停止)

  IF dead_routes 不為空:
    - 執行問題診斷 (delegate_to_provider 分析最近 3 次執行日誌)
    - 如果可修復: 自動調整手的 system_prompt 或 settings
    - 如果不可修復: 降至每週一次 (保持最低活躍度)

  IF bleeding_routes 不為空:
    - 暫停該路線的 cron 排程
    - 發送 Telegram 告警: "路線 X 過去 7 天虧損 $Y, 已暫停"
    - 等待人工 /approve 或 /deny

  成本優化:
    IF 今日總成本 > daily_budget * 0.8:
      - 切換付費 provider -> 免費 provider (Groq/Gemini/LMStudio)
      - 減少 max_rounds (5 -> 3)
    IF 今日總成本 < daily_budget * 0.3:
      - 可以使用更好的模型提升品質
      - 增加 max_rounds (3 -> 5)
```

### 2.3 Cron 排程動態調整

```rust
// 頻率調整的具體操作:
//
// 增加頻率 (x1.5):
//   "0 9 * * *" (每天9:00) -> 追加 "0 15 * * *" (再加下午3:00)
//
// 減少頻率 (x0.75):
//   "0 9 * * *" (每天) -> "0 9 * * 1,3,5" (只有一三五)
//
// 降至最低:
//   any -> "0 9 * * 1" (只有週一)
```

---

## 3. 每日自動化排程

### 3.1 完整排程表 (UTC+8 台灣時間)

```
時間    Hand                系統動作              備註
─────   ─────────────────   ──────────────────    ──────────────────
05:00   market_intel        掃描市場機會           Route D,H,I 上游
06:00   lead                找潛在客戶            Route D,H 上游
07:00   freelancer          搜尋工作機會           Route A 核心
08:00   seo_content         生成 SEO 文章          Route C,F,J
09:00   content             社群內容+發布          Route C,G
10:00   outreach            冷郵件發送             Route D,H (chain from lead)
12:00   trading_analysis    午間市場分析報告        Route I 輔助
14:00   [保留]              ROI 中期檢查            cost check + alert
18:00   auto_report         每日營運報告            推送到 Telegram
20:00   customer_service    回覆當日諮詢            Route D,H,J
22:00   self_optimize       分析表現+調整排程       Revenue Engine 核心
```

### 3.2 排程的 Cron 表達式 (UTC, 台灣時間 -8)

```toml
# 所有時間為 UTC (台灣時間 = UTC+8)
# 05:00 TWD = 21:00 UTC (前一天)
# 注意: cron 欄位順序 = 分 時 日 月 週

# Route 上游 (21:00-01:00 UTC = 05:00-09:00 TWD)
market_intel  = "0 21 * * *"   # 05:00 TWD
lead          = "0 22 * * *"   # 06:00 TWD
freelancer    = "0 23 * * *"   # 07:00 TWD
seo_content   = "0 0  * * *"   # 08:00 TWD
content       = "0 1  * * *"   # 09:00 TWD

# Route 執行 (02:00-04:00 UTC = 10:00-12:00 TWD)
outreach         = "0 2  * * *"   # 10:00 TWD
trading_analysis = "0 4  * * *"   # 12:00 TWD

# 監控+報告 (06:00-14:00 UTC = 14:00-22:00 TWD)
roi_midcheck      = "0 6  * * *"   # 14:00 TWD
auto_report       = "0 10 * * *"   # 18:00 TWD
customer_service  = "0 12 * * *"   # 20:00 TWD
self_optimize     = "0 14 * * *"   # 22:00 TWD
```

### 3.3 週末/特殊日調整

```
週一: lead 增加一次 (14:00 TWD), freelancer 增加一次 (14:00 TWD)
週五: auto_report 提前到 16:00 TWD (週報 + 日報)
週日: 只執行 content + seo_content (社群不休息)
月初: 月報生成 (auto_report 特殊模式)
季初: 季度回顧 (growth_review hand)
```

### 3.4 排程註冊代碼

```rust
/// 註冊預設的每日自動化排程
pub async fn register_default_schedule(scheduler: &Scheduler) -> Result<()> {
    let schedules = vec![
        // (名稱, cron表達式, hand名稱, 預設輸入)
        ("daily_market_intel", "0 21 * * *", "market_intel",
         "掃描 AI/SaaS/自動化 市場機會, 重點關注台灣和東南亞市場"),
        ("daily_lead_gen", "0 22 * * *", "lead",
         "尋找需要 AI 自動化、LLM 整合的企業客戶, 重點: 中小企業"),
        ("daily_freelancer", "0 23 * * *", "freelancer",
         "搜尋 Upwork/Fiverr AI automation, LLM integration, Rust development 工作"),
        ("daily_seo_content", "0 0 * * *", "seo_content",
         "生成 AI 自動化相關 SEO 文章, 關鍵字: AI agent, LLM automation, Rust AI"),
        ("daily_content", "0 1 * * *", "content",
         "生成社群媒體內容: AI 工具推薦、自動化技巧、技術心得"),
        ("daily_outreach", "0 2 * * *", "outreach",
         "根據今日 lead 結果發送冷郵件, 個人化每封信"),
        ("daily_trading", "0 4 * * *", "trading_analysis",
         "分析 AI 股票/加密貨幣趨勢, 重點: NVIDIA, AMD, AI ETF"),
        ("daily_report", "0 10 * * *", "auto_report",
         "生成今日營運報告: 營收、成本、ROI、Hand 執行結果"),
        ("daily_customer", "0 12 * * *", "customer_service",
         "回覆今日所有客戶諮詢, 禮貌專業, 引導成交"),
        ("daily_optimize", "0 14 * * *", "self_optimize",
         "分析今日營收/成本數據, 計算各路線 ROI, 調整明日排程"),
    ];

    for (name, cron_expr, hand, input) in schedules {
        scheduler.add_job(
            name,
            Schedule::Cron { expr: cron_expr.to_string() },
            JobAction::Hand {
                hand_name: hand.to_string(),
                input: input.to_string(),
            },
            None, // unlimited runs
        ).await?;
    }
    Ok(())
}
```

---

## 4. 營收再投資算法

### 4.1 每週結算流程

```
每週日 23:00 (UTC) 觸發 weekly_settlement:

1. 計算本週淨利潤:
   weekly_revenue = revenue_tracker.by_day(7).sum()
   weekly_cost    = cost_tracker.by_day(7).sum()
   net_profit     = weekly_revenue - weekly_cost

2. 如果 net_profit > 0:
   expansion_fund  += net_profit * 0.60   # 60% 擴展基金
   api_budget      += net_profit * 0.20   # 20% API 額度
   tools_budget    += net_profit * 0.20   # 20% 工具/域名

3. 更新預算:
   daily_api_limit = (api_budget / 7.0)   # 平攤到每天
   如果 daily_api_limit > 當前限制 * 1.5:
     daily_api_limit = 當前限制 * 1.5     # 緩慢增長, 避免暴增

4. 觸發擴展檢查:
   IF expansion_fund > NT$15,000 (約 $470 USD):
     -> Telegram 通知: "擴展基金達到 NT$15,000, 建議購買新機器"
     -> 等待 /approve
   IF expansion_fund > NT$30,000:
     -> Telegram 通知: "擴展基金達到 NT$30,000, 建議第二台機器"
```

### 4.2 預算分配模型

```
                    淨利潤
                      |
          +-----------+-----------+
          |           |           |
       60%         20%         20%
     擴展基金    API 額度     工具預算
          |           |           |
     [存起來]    [提升品質]   [購買資源]
          |           |           |
     達到門檻?   分配到每日    域名/服務
          |      API 預算     /訂閱
          |           |           |
     NT$15K   Anthropic升級   新域名
     買機器    GPT-4o 額度    Render 方案
     NT$30K   更高 max_rounds SerpAPI
     第二台                   Stripe Plus
```

### 4.3 工具預算自動購買邏輯

```
tools_budget 分配優先級:

1. 域名 (如果有 SaaS 產品需要): ~$12/yr
2. Render.com 升級 (如果流量超過免費額度): ~$7/mo
3. SerpAPI 付費 (如果 web_search 頻繁): ~$50/mo
4. Stripe 進階功能 (如果交易量增加): 按交易量
5. 備用: 存入 expansion_fund
```

---

## 5. 失敗恢復機制

### 5.1 告警層級

```
Level 1 - INFO (自動處理):
  - 單次 hand 執行失敗 -> 記錄日誌, 下次 cron 重試
  - 某路線當日營收為 0 -> 正常, 繼續觀察

Level 2 - WARNING (Telegram 通知):
  - 某路線連續 2 天營收為 0 -> 發送提醒
  - 今日成本超過預算 80% -> 切換到免費 provider
  - 某 hand 連續 3 次失敗 -> 通知 + 自動診斷

Level 3 - CRITICAL (需人工介入):
  - 某路線連續 3 天營收為 0 -> 暫停 + 診斷報告
  - 整體營收下降 50% (7日均值 vs 前7日) -> 緊急策略告警
  - 所有付費 provider 都不可用 -> E-Stop 考慮

Level 4 - EMERGENCY (自動降級):
  - E-Stop 觸發 -> 所有 hand 暫停
  - 成本超過硬性上限 (daily_hard_limit) -> 暫停付費 provider
  - 異常大量 API 調用 (>正常 3x) -> 限流 + 告警
```

### 5.2 問題診斷流程

```
觸發: 某路線連續 3 天營收為 0

步驟:
1. 收集最近 3 次該路線 hand 的執行日誌
2. 分析日誌:
   - 是否有 tool 調用失敗? (API 變更、網站封鎖)
   - 是否有 LLM 輸出品質問題? (模型太弱、prompt 需調整)
   - 是否有外部原因? (市場變化、季節性)
3. 生成診斷報告
4. 根據診斷自動嘗試修復:
   - Tool 失敗 -> 嘗試替代 tool 或更新參數
   - LLM 品質 -> 提升 max_rounds 或切換 provider
   - 外部原因 -> 調整 hand 的 system_prompt (加入新策略)
5. 發送診斷報告到 Telegram
6. 執行一次測試運行
7. 如果測試成功 -> 恢復正常排程; 如果失敗 -> 降至每週一次
```

### 5.3 整體營收告警

```
每日 14:00 (roi_midcheck):

1. 計算:
   avg_7d     = 過去 7 天每日平均營收
   avg_prev7d = 前 7 天每日平均營收 (第 8-14 天)

2. 趨勢判斷:
   change_pct = (avg_7d - avg_prev7d) / avg_prev7d * 100

3. 告警:
   change_pct < -50% -> CRITICAL: 緊急策略調整
     -> 暫停低 ROI 路線
     -> 增加高 ROI 路線頻率
     -> 發送緊急報告

   change_pct < -25% -> WARNING: 下降趨勢
     -> 分析哪條路線下降最多
     -> 發送通知

   change_pct > +50% -> INFO: 增長趨勢
     -> 分析哪條路線增長最快
     -> 考慮增加該路線投入
```

### 5.4 自動切換策略

```
如果路線 A (freelancer) 連續 3 天零營收:
  1. 切換到路線 H (automation_services) 的 lead+outreach
  2. 增加路線 C (content_monetization) 的頻率
  3. 原因: 被動收入路線可以補充主動收入的缺口

如果路線 C (content) 下降:
  1. 增加 seo_content 頻率 (更多文章 = 更多流量)
  2. 調整關鍵字策略 (memory_recall 品牌聲量數據)
  3. 增加社群發布頻率 (content hand 2x/day)

如果路線 B (SaaS) 有產品但沒客戶:
  1. 增加 outreach 頻率 (冷郵件推銷產品)
  2. 增加 seo_content (產品相關關鍵字)
  3. 考慮降價策略 (修改 Stripe pricing)
```

---

## 6. 增長策略自動執行

### 6.1 季度回顧 (Quarterly Review)

```
觸發: 每季度第一天 (1/1, 4/1, 7/1, 10/1) 08:00 TWD
Cron: "0 0 1 1,4,7,10 *"
Hand: growth_review (新建)

流程:
Phase 1 - 數據收集 (max_rounds=5):
  - revenue_tracker.by_route(90)  # 90天各路線營收
  - cost_tracker.by_provider(90)  # 90天各provider成本
  - 計算每路線 90 天 ROI
  - 計算月度趨勢 (M1 vs M2 vs M3)

Phase 2 - 分析 (max_rounds=3):
  - 識別最佳路線 (持續盈利)
  - 識別最差路線 (持續虧損)
  - 識別增長路線 (趨勢向上)
  - 識別衰退路線 (趨勢向下)
  - 計算整體投入產出比

Phase 3 - 策略建議 (max_rounds=5):
  - 基於分析結果建議:
    - 要不要開新路線?
    - 要不要關閉某路線?
    - 要不要調整定價?
    - 硬體是否需要擴展?
    - API 預算是否需要調整?
  - 生成可執行的行動計劃

Phase 4 - 報告與分發 (max_rounds=3):
  - 生成 PDF 季報 (pdf_export tool)
  - 發送到 Telegram
  - 存入 memory_store (作為下季度參考)
```

### 6.2 A/B 測試框架

```
用途: 測試不同的定價、文案、策略

結構:
  ab_test = {
    id: "price_test_001",
    route: "B:saas_products",
    variable: "pricing",
    variant_a: { price: "$29/mo", start: "2026-03-01" },
    variant_b: { price: "$19/mo", start: "2026-03-01" },
    duration_days: 14,
    metric: "conversion_rate",  // revenue / leads
  }

執行:
  - 前 7 天: variant_a (修改 Stripe price, outreach 使用 A 定價文案)
  - 後 7 天: variant_b
  - 14 天後: 自動比較, 選擇勝出方案
  - 發送結果報告到 Telegram

存儲: memory_store key = "ab_test_{id}"
```

### 6.3 新路線探索

```
每月觸發一次 (monthly_exploration):
Cron: "0 0 15 * *"  # 每月15日

Phase 1 - 市場掃描:
  - web_search: 最新 AI 賺錢方式
  - web_search: 自動化服務新需求
  - 分析競爭對手動態

Phase 2 - 可行性評估:
  - 我們的工具能做嗎?
  - 需要多少額外成本?
  - 預估 ROI

Phase 3 - 建議:
  - 輸出: "建議新路線: [描述], 預估 ROI: X%, 實施成本: $Y"
  - 發送到 Telegram, 等待 /approve

如果批准:
  - 自動創建新 hand (scaffold)
  - 加入 cron 排程 (每週一次測試)
  - 2 週後回顧成效
```

---

## 7. Dashboard 設計

### 7.1 Telegram 指令增強

```
現有指令:
  /revenue          — 今日營收總覽
  /costs            — 今日成本總覽

新增指令:
  /dashboard        — 完整儀表板 (營收+成本+ROI+趨勢)
  /roi              — 各路線 ROI 排名
  /roi <route>      — 特定路線詳細 ROI
  /trend            — 7 天趨勢圖 (文字版)
  /trend 30         — 30 天趨勢
  /weekly           — 本週報告
  /monthly          — 本月報告
  /budget           — 預算使用狀況
  /fund             — 擴展基金餘額
  /optimize         — 立即觸發優化迴圈
  /diagnose <route> — 手動觸發問題診斷
  /schedule         — 查看明日排程
  /ab_test          — 查看進行中的 A/B 測試
```

### 7.2 /dashboard 輸出格式

```
===== CLAWTEX DASHBOARD =====
2026-03-05 22:00 (TWD)

--- 今日 ---
營收: $128.50 (4 筆)
成本: $2.34 (47 次 LLM 呼叫)
淨利: $126.16
ROI:  5,393%

--- 本週 (3/1 - 3/5) ---
營收: $843.20 (12 筆)
成本: $11.75 (198 次)
淨利: $831.45

--- 路線排名 (7天) ---
1. A:freelance   $500.00  ROI: inf (免費LLM)
2. C:content     $180.00  ROI: 7,200%
3. D:consulting  $120.00  ROI: 4,800%
4. H:automation   $43.20  ROI: 1,728%
5-10: 暫無營收

--- 趨勢 (7天) ---
03/01: $180.00 ████████████
03/02: $165.00 ███████████
03/03: $200.00 █████████████
03/04: $170.00 ███████████
03/05: $128.50 ████████

--- 預算 ---
API 額度:  $3.50 / $10.00 (35%)
擴展基金:  NT$4,500 / NT$15,000
工具預算:  $25.00 (可用)

--- 明日排程 ---
05:00 market_intel
06:00 lead
07:00 freelancer
08:00 seo_content
09:00 content
10:00 outreach
12:00 trading_analysis
18:00 auto_report
20:00 customer_service
22:00 self_optimize
============================
```

### 7.3 自動推送報告

```
每日 18:00 TWD (auto_report hand):
  -> Telegram 推送日報 (簡化版 dashboard)

每週日 20:00 TWD:
  -> Telegram 推送週報 (含趨勢 + 建議)
  -> PDF 版存檔

每月 1 日 08:00 TWD:
  -> Telegram 推送月報
  -> PDF 版存檔
  -> 含: 月度 ROI、路線排名、成本分析、下月建議
```

### 7.4 HTTP API 端點

```
GET  /dashboard                — JSON 版完整儀表板
GET  /dashboard/roi            — ROI 排名
GET  /dashboard/trend?days=7   — 趨勢數據
GET  /dashboard/budget         — 預算狀況
GET  /dashboard/schedule       — 當前排程
POST /dashboard/optimize       — 觸發優化迴圈
POST /dashboard/diagnose/:route — 觸發問題診斷
```

---

## 8. Rust 結構設計

### 8.1 核心結構: RevenueEngine

```rust
//! Revenue Engine — 持續營利自動化引擎
//! 監控營收/成本, 計算 ROI, 自動調整排程, 觸發告警

use anyhow::Result;
use chrono::{DateTime, Utc, Duration as ChronoDuration, Datelike, Weekday};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn, error};

use crate::cost_tracker::CostTracker;
use crate::cron::{Scheduler, Schedule, JobAction};
use crate::revenue_tracker::{RevenueTracker, ALL_ROUTES};

// ── 路線到 Hand 映射 ──────────────────────────────────────────────

/// 每條營收路線對應的 Hands
pub fn route_hands(route: &str) -> Vec<&'static str> {
    match route {
        "A:freelance_dev"       => vec!["freelancer"],
        "B:saas_products"       => vec!["product_spec", "code_gen", "saas_deploy"],
        "C:content_monetization"=> vec!["seo_content", "content"],
        "D:consulting"          => vec!["market_intel", "lead", "outreach"],
        "E:api_services"        => vec!["code_gen", "saas_deploy"],
        "F:affiliate_marketing" => vec!["seo_content", "content"],
        "G:digital_products"    => vec!["product_spec", "content"],
        "H:automation_services" => vec!["lead", "outreach", "market_intel"],
        "I:data_services"       => vec!["market_intel"],
        "J:training_education"  => vec!["content", "seo_content"],
        _ => vec![],
    }
}

// ── 告警層級 ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertLevel {
    Info,
    Warning,
    Critical,
    Emergency,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub level: AlertLevel,
    pub route: Option<String>,
    pub message: String,
    pub timestamp: DateTime<Utc>,
    pub suggested_action: Option<String>,
}

// ── ROI 計算結果 ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteROI {
    pub route: String,
    pub revenue_7d: f64,
    pub cost_7d: f64,
    pub roi: f64,             // (revenue - cost) / cost, or f64::INFINITY if cost == 0
    pub daily_avg_revenue: f64,
    pub zero_revenue_days: u32, // 連續零營收天數
    pub trend: TrendDirection,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TrendDirection {
    Rising,
    Stable,
    Falling,
    Inactive,
}

// ── 預算狀態 ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetState {
    pub expansion_fund_twd: f64,     // 擴展基金 (台幣)
    pub api_budget_usd: f64,         // API 額度 (美金)
    pub tools_budget_usd: f64,       // 工具預算 (美金)
    pub daily_api_limit_usd: f64,    // 每日 API 上限
    pub daily_hard_limit_usd: f64,   // 每日硬性上限 (不可超過)
    pub last_settlement: DateTime<Utc>,
}

impl Default for BudgetState {
    fn default() -> Self {
        Self {
            expansion_fund_twd: 0.0,
            api_budget_usd: 0.0,
            tools_budget_usd: 0.0,
            daily_api_limit_usd: 5.0,   // 初始每日 $5
            daily_hard_limit_usd: 20.0,  // 硬性上限 $20/day
            last_settlement: Utc::now(),
        }
    }
}

// ── 優化決策 ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationDecision {
    pub timestamp: DateTime<Utc>,
    pub route_adjustments: Vec<RouteAdjustment>,
    pub provider_switches: Vec<ProviderSwitch>,
    pub alerts: Vec<Alert>,
    pub budget_update: Option<BudgetState>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteAdjustment {
    pub route: String,
    pub action: AdjustmentAction,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdjustmentAction {
    /// 增加頻率 (multiplier, e.g. 1.5)
    IncreaseFrequency { multiplier: f64 },
    /// 減少頻率
    DecreaseFrequency { multiplier: f64 },
    /// 暫停排程
    Pause,
    /// 恢復排程
    Resume,
    /// 降至最低 (每週一次)
    MinimumFrequency,
    /// 觸發問題診斷
    Diagnose,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSwitch {
    pub from_provider: String,
    pub to_provider: String,
    pub reason: String,
}

// ── 儀表板數據 ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardData {
    pub generated_at: DateTime<Utc>,
    // 今日
    pub today_revenue: f64,
    pub today_cost: f64,
    pub today_net: f64,
    pub today_transactions: u32,
    pub today_llm_calls: u32,
    // 本週
    pub week_revenue: f64,
    pub week_cost: f64,
    pub week_net: f64,
    // 路線排名
    pub route_rankings: Vec<RouteROI>,
    // 趨勢 (每日營收)
    pub daily_trend: Vec<(String, f64)>,  // (date, revenue)
    // 預算
    pub budget: BudgetState,
    // 明日排程
    pub tomorrow_schedule: Vec<ScheduleEntry>,
    // 告警
    pub active_alerts: Vec<Alert>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleEntry {
    pub time_twd: String,   // "05:00"
    pub hand_name: String,
    pub description: String,
}

// ── RevenueEngine 主結構 ──────────────────────────────────────────

pub struct RevenueEngine {
    revenue_tracker: Arc<RevenueTracker>,
    cost_tracker: Arc<CostTracker>,
    scheduler: Arc<Scheduler>,
    budget: Arc<tokio::sync::RwLock<BudgetState>>,
    /// 路線到 cron job ID 的映射
    route_job_ids: Arc<tokio::sync::RwLock<HashMap<String, Vec<String>>>>,
    /// 歷史告警
    alerts: Arc<tokio::sync::RwLock<Vec<Alert>>>,
    /// USD to TWD 匯率 (簡化, 固定值)
    usd_to_twd: f64,
}

impl RevenueEngine {
    pub fn new(
        revenue_tracker: Arc<RevenueTracker>,
        cost_tracker: Arc<CostTracker>,
        scheduler: Arc<Scheduler>,
    ) -> Self {
        Self {
            revenue_tracker,
            cost_tracker,
            scheduler,
            budget: Arc::new(tokio::sync::RwLock::new(BudgetState::default())),
            route_job_ids: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            alerts: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            usd_to_twd: 32.0, // 大約匯率
        }
    }

    // ── ROI 計算 ──────────────────────────────────────────────────

    /// 計算所有路線的 7 天 ROI
    pub fn calculate_all_roi(&self) -> Result<Vec<RouteROI>> {
        let revenue_by_route = self.revenue_tracker.by_route(7)?;
        let cost_by_day = self.cost_tracker.by_day(7)?;
        let revenue_by_day = self.revenue_tracker.by_day(7)?;

        let total_cost_7d: f64 = cost_by_day.iter().map(|s| s.total_cost_usd).sum();
        let total_days = revenue_by_day.len().max(1) as f64;

        let mut results = Vec::new();

        for route_const in ALL_ROUTES {
            let route = route_const.to_string();
            let rev_summary = revenue_by_route.iter()
                .find(|s| s.group == route);

            let revenue_7d = rev_summary.map(|s| s.total_usd).unwrap_or(0.0);
            let hands = route_hands(&route);
            let hand_count = hands.len().max(1) as f64;

            // 按 hand 數量比例分配成本 (簡化)
            let total_hands: f64 = ALL_ROUTES.iter()
                .flat_map(|r| route_hands(r))
                .collect::<Vec<_>>()
                .len() as f64;
            let cost_7d = total_cost_7d * (hand_count / total_hands.max(1.0));

            let roi = if cost_7d > 0.001 {
                (revenue_7d - cost_7d) / cost_7d
            } else if revenue_7d > 0.0 {
                f64::INFINITY
            } else {
                0.0
            };

            // 計算連續零營收天數
            let zero_days = self.count_zero_revenue_days(&route)?;

            // 趨勢計算
            let trend = self.calculate_trend(&route)?;

            results.push(RouteROI {
                route,
                revenue_7d,
                cost_7d,
                roi,
                daily_avg_revenue: revenue_7d / total_days,
                zero_revenue_days: zero_days,
                trend,
            });
        }

        // 按 ROI 降序排列
        results.sort_by(|a, b| b.roi.partial_cmp(&a.roi).unwrap_or(std::cmp::Ordering::Equal));

        Ok(results)
    }

    /// 計算某路線連續零營收天數
    fn count_zero_revenue_days(&self, route: &str) -> Result<u32> {
        let daily = self.revenue_tracker.by_day(30)?;
        let mut consecutive = 0u32;

        for day_summary in &daily {
            // 需要按路線過濾 - 此處簡化為檢查整體
            // 實際應查詢特定路線的每日數據
            if day_summary.total_usd == 0.0 {
                consecutive += 1;
            } else {
                break;
            }
        }

        Ok(consecutive)
    }

    /// 計算路線趨勢 (最近 7 天 vs 前 7 天)
    fn calculate_trend(&self, _route: &str) -> Result<TrendDirection> {
        let recent = self.revenue_tracker.by_day(7)?;
        let older = self.revenue_tracker.by_day(14)?;

        let recent_sum: f64 = recent.iter().map(|s| s.total_usd).sum();
        let older_sum: f64 = older.iter().map(|s| s.total_usd).sum::<f64>() - recent_sum;

        if recent_sum == 0.0 && older_sum == 0.0 {
            return Ok(TrendDirection::Inactive);
        }

        if older_sum < 0.01 {
            return Ok(if recent_sum > 0.0 { TrendDirection::Rising } else { TrendDirection::Inactive });
        }

        let change = (recent_sum - older_sum) / older_sum;
        Ok(match change {
            c if c > 0.10 => TrendDirection::Rising,
            c if c < -0.10 => TrendDirection::Falling,
            _ => TrendDirection::Stable,
        })
    }

    // ── 優化決策 ──────────────────────────────────────────────────

    /// 執行每日優化迴圈 (22:00 TWD 觸發)
    pub async fn run_optimization_loop(&self) -> Result<OptimizationDecision> {
        info!("Revenue Engine: 開始每日優化迴圈");

        let roi_data = self.calculate_all_roi()?;
        let today_cost = self.cost_tracker.today_total()?;
        let budget = self.budget.read().await.clone();

        let mut adjustments = Vec::new();
        let mut alerts = Vec::new();
        let mut provider_switches = Vec::new();

        // 1. 識別最賺錢路線
        let top_routes: Vec<&RouteROI> = roi_data.iter()
            .filter(|r| r.roi > 0.0 && r.revenue_7d > 0.0)
            .collect();

        if let Some(best) = top_routes.first() {
            if best.roi > 1.0 {
                adjustments.push(RouteAdjustment {
                    route: best.route.clone(),
                    action: AdjustmentAction::IncreaseFrequency { multiplier: 1.5 },
                    reason: format!("最高 ROI: {:.0}%, 增加頻率", best.roi * 100.0),
                });
            }
        }

        // 2. 識別死亡路線 (連續 3+ 天零營收)
        for roi in &roi_data {
            if roi.zero_revenue_days >= 3 {
                adjustments.push(RouteAdjustment {
                    route: roi.route.clone(),
                    action: AdjustmentAction::Diagnose,
                    reason: format!("連續 {} 天零營收", roi.zero_revenue_days),
                });
                alerts.push(Alert {
                    level: AlertLevel::Critical,
                    route: Some(roi.route.clone()),
                    message: format!(
                        "路線 {} 連續 {} 天營收為 0, 已觸發問題診斷",
                        roi.route, roi.zero_revenue_days
                    ),
                    timestamp: Utc::now(),
                    suggested_action: Some("檢查相關 Hand 執行日誌, 考慮調整策略".to_string()),
                });
            }
        }

        // 3. 識別虧損路線 (ROI < -1.0)
        for roi in &roi_data {
            if roi.roi < -1.0 && roi.cost_7d > 1.0 {
                adjustments.push(RouteAdjustment {
                    route: roi.route.clone(),
                    action: AdjustmentAction::Pause,
                    reason: format!("ROI {:.0}%, 虧損嚴重", roi.roi * 100.0),
                });
                alerts.push(Alert {
                    level: AlertLevel::Critical,
                    route: Some(roi.route.clone()),
                    message: format!(
                        "路線 {} 過去 7 天虧損 ${:.2}, ROI={:.0}%, 已暫停",
                        roi.route, roi.cost_7d - roi.revenue_7d, roi.roi * 100.0
                    ),
                    timestamp: Utc::now(),
                    suggested_action: Some("使用 /approve 恢復或 /deny 永久停用".to_string()),
                });
            }
        }

        // 4. 成本控制
        if today_cost.total_cost_usd > budget.daily_api_limit_usd * 0.8 {
            provider_switches.push(ProviderSwitch {
                from_provider: "anthropic".to_string(),
                to_provider: "lmstudio".to_string(),
                reason: format!(
                    "今日成本 ${:.2} 超過預算 80% (${:.2}), 切換到免費 provider",
                    today_cost.total_cost_usd, budget.daily_api_limit_usd
                ),
            });
            alerts.push(Alert {
                level: AlertLevel::Warning,
                route: None,
                message: format!(
                    "今日 API 成本 ${:.2} 接近上限 ${:.2}, 已切換到免費 provider",
                    today_cost.total_cost_usd, budget.daily_api_limit_usd
                ),
                timestamp: Utc::now(),
                suggested_action: None,
            });
        }

        // 5. 整體趨勢告警
        let trend_alert = self.check_overall_trend()?;
        if let Some(alert) = trend_alert {
            alerts.push(alert);
        }

        // 生成摘要
        let summary = format!(
            "優化完成: {} 條路線調整, {} 條告警, {} 條 provider 切換",
            adjustments.len(), alerts.len(), provider_switches.len()
        );
        info!("Revenue Engine: {}", summary);

        // 儲存告警
        {
            let mut alert_store = self.alerts.write().await;
            alert_store.extend(alerts.clone());
            // 只保留最近 100 條
            if alert_store.len() > 100 {
                let drain_count = alert_store.len() - 100;
                alert_store.drain(0..drain_count);
            }
        }

        Ok(OptimizationDecision {
            timestamp: Utc::now(),
            route_adjustments: adjustments,
            provider_switches,
            alerts,
            budget_update: None,
            summary,
        })
    }

    /// 檢查整體營收趨勢
    fn check_overall_trend(&self) -> Result<Option<Alert>> {
        let recent = self.revenue_tracker.by_day(7)?;
        let all_14 = self.revenue_tracker.by_day(14)?;

        let recent_sum: f64 = recent.iter().map(|s| s.total_usd).sum();
        let total_14: f64 = all_14.iter().map(|s| s.total_usd).sum();
        let prev_sum = total_14 - recent_sum;

        if prev_sum < 0.01 {
            return Ok(None);
        }

        let change_pct = (recent_sum - prev_sum) / prev_sum * 100.0;

        if change_pct < -50.0 {
            Ok(Some(Alert {
                level: AlertLevel::Critical,
                route: None,
                message: format!(
                    "整體營收下降 {:.0}% (本週 ${:.2} vs 上週 ${:.2}), 需要緊急策略調整",
                    change_pct.abs(), recent_sum, prev_sum
                ),
                timestamp: Utc::now(),
                suggested_action: Some(
                    "建議: 暫停低 ROI 路線, 增加高 ROI 路線, 檢查是否有外部因素".to_string()
                ),
            }))
        } else if change_pct < -25.0 {
            Ok(Some(Alert {
                level: AlertLevel::Warning,
                route: None,
                message: format!(
                    "整體營收下降 {:.0}% (本週 ${:.2} vs 上週 ${:.2})",
                    change_pct.abs(), recent_sum, prev_sum
                ),
                timestamp: Utc::now(),
                suggested_action: Some("觀察趨勢, 如持續下降考慮調整策略".to_string()),
            }))
        } else {
            Ok(None)
        }
    }

    // ── 每週結算 ──────────────────────────────────────────────────

    /// 每週結算 (每週日 23:00 UTC)
    pub async fn weekly_settlement(&self) -> Result<String> {
        let revenue_days = self.revenue_tracker.by_day(7)?;
        let cost_days = self.cost_tracker.by_day(7)?;

        let weekly_revenue: f64 = revenue_days.iter().map(|s| s.total_usd).sum();
        let weekly_cost: f64 = cost_days.iter().map(|s| s.total_cost_usd).sum();
        let net_profit = weekly_revenue - weekly_cost;

        let mut budget = self.budget.write().await;
        let mut report = format!(
            "=== 每週結算報告 ===\n\
             營收: ${:.2}\n\
             成本: ${:.2}\n\
             淨利: ${:.2}\n\n",
            weekly_revenue, weekly_cost, net_profit
        );

        if net_profit > 0.0 {
            let expansion = net_profit * 0.60 * self.usd_to_twd;
            let api = net_profit * 0.20;
            let tools = net_profit * 0.20;

            budget.expansion_fund_twd += expansion;
            budget.api_budget_usd += api;
            budget.tools_budget_usd += tools;

            // 調整每日 API 限制 (平攤, 上限 1.5x)
            let new_daily = budget.api_budget_usd / 7.0;
            let max_daily = budget.daily_api_limit_usd * 1.5;
            if new_daily > budget.daily_api_limit_usd && new_daily <= max_daily {
                budget.daily_api_limit_usd = new_daily;
            }

            budget.last_settlement = Utc::now();

            report += &format!(
                "分配:\n\
                 - 擴展基金: +NT${:.0} (累計 NT${:.0})\n\
                 - API 額度: +${:.2} (累計 ${:.2})\n\
                 - 工具預算: +${:.2} (累計 ${:.2})\n\
                 - 每日 API 上限: ${:.2}\n",
                expansion, budget.expansion_fund_twd,
                api, budget.api_budget_usd,
                tools, budget.tools_budget_usd,
                budget.daily_api_limit_usd,
            );

            // 擴展門檻檢查
            if budget.expansion_fund_twd >= 30_000.0 {
                report += "\n[擴展通知] 基金達到 NT$30,000, 建議購買第二台機器!\n";
            } else if budget.expansion_fund_twd >= 15_000.0 {
                report += "\n[擴展通知] 基金達到 NT$15,000, 建議購買新機器!\n";
            }
        } else {
            report += "本週淨利為負, 不進行分配\n";
            report += "建議: 減少付費 API 使用, 增加免費 provider 比例\n";
        }

        info!("Revenue Engine: 每週結算完成 — 淨利 ${:.2}", net_profit);
        Ok(report)
    }

    // ── 儀表板 ────────────────────────────────────────────────────

    /// 生成完整儀表板數據
    pub async fn generate_dashboard(&self) -> Result<DashboardData> {
        let today_rev = self.revenue_tracker.today_total()?;
        let today_cost = self.cost_tracker.today_total()?;
        let week_rev_days = self.revenue_tracker.by_day(7)?;
        let week_cost_days = self.cost_tracker.by_day(7)?;
        let roi_data = self.calculate_all_roi()?;
        let budget = self.budget.read().await.clone();
        let alerts = self.alerts.read().await.clone();

        let week_revenue: f64 = week_rev_days.iter().map(|s| s.total_usd).sum();
        let week_cost: f64 = week_cost_days.iter().map(|s| s.total_cost_usd).sum();

        let daily_trend: Vec<(String, f64)> = week_rev_days.iter()
            .map(|s| (s.group.clone(), s.total_usd))
            .collect();

        // 構建明日排程
        let tomorrow_schedule = self.build_schedule_entries();

        Ok(DashboardData {
            generated_at: Utc::now(),
            today_revenue: today_rev.total_usd,
            today_cost: today_cost.total_cost_usd,
            today_net: today_rev.total_usd - today_cost.total_cost_usd,
            today_transactions: today_rev.count,
            today_llm_calls: today_cost.call_count,
            week_revenue,
            week_cost,
            week_net: week_revenue - week_cost,
            route_rankings: roi_data,
            daily_trend,
            budget,
            tomorrow_schedule,
            active_alerts: alerts.into_iter()
                .filter(|a| a.level == AlertLevel::Critical || a.level == AlertLevel::Warning)
                .take(10)
                .collect(),
        })
    }

    /// 生成 Telegram 格式的儀表板文字
    pub async fn format_dashboard_telegram(&self) -> Result<String> {
        let data = self.generate_dashboard().await?;

        let today_roi = if data.today_cost > 0.001 {
            format!("{:.0}%", ((data.today_revenue - data.today_cost) / data.today_cost) * 100.0)
        } else if data.today_revenue > 0.0 {
            "inf".to_string()
        } else {
            "N/A".to_string()
        };

        let mut text = format!(
            "===== CLAWTEX DASHBOARD =====\n\
             {}\n\n\
             --- 今日 ---\n\
             營收: ${:.2} ({} 筆)\n\
             成本: ${:.2} ({} 次 LLM)\n\
             淨利: ${:.2}\n\
             ROI:  {}\n\n\
             --- 本週 ---\n\
             營收: ${:.2}\n\
             成本: ${:.2}\n\
             淨利: ${:.2}\n\n\
             --- 路線排名 (7天) ---\n",
            Utc::now().format("%Y-%m-%d %H:%M UTC"),
            data.today_revenue, data.today_transactions,
            data.today_cost, data.today_llm_calls,
            data.today_net,
            today_roi,
            data.week_revenue,
            data.week_cost,
            data.week_net,
        );

        // 路線排名
        for (i, roi) in data.route_rankings.iter().enumerate().take(5) {
            let roi_str = if roi.roi == f64::INFINITY {
                "inf".to_string()
            } else {
                format!("{:.0}%", roi.roi * 100.0)
            };
            let trend_icon = match roi.trend {
                TrendDirection::Rising => "^",
                TrendDirection::Falling => "v",
                TrendDirection::Stable => "=",
                TrendDirection::Inactive => "-",
            };
            text += &format!(
                "{}. {} ${:.2}  ROI:{} {}\n",
                i + 1, roi.route, roi.revenue_7d, roi_str, trend_icon
            );
        }

        // 趨勢
        text += "\n--- 趨勢 (7天) ---\n";
        let max_rev = data.daily_trend.iter()
            .map(|(_, v)| *v)
            .fold(0.0f64, f64::max)
            .max(1.0);
        for (date, rev) in &data.daily_trend {
            let bar_len = ((*rev / max_rev) * 20.0) as usize;
            let bar: String = std::iter::repeat('#').take(bar_len).collect();
            text += &format!("{}: ${:.2} {}\n", &date[5..], rev, bar);
        }

        // 預算
        let budget_pct = if data.budget.daily_api_limit_usd > 0.0 {
            (data.today_cost / data.budget.daily_api_limit_usd * 100.0) as u32
        } else {
            0
        };
        text += &format!(
            "\n--- 預算 ---\n\
             API 額度: ${:.2} / ${:.2} ({}%)\n\
             擴展基金: NT${:.0} / NT$15,000\n\
             工具預算: ${:.2}\n",
            data.today_cost, data.budget.daily_api_limit_usd, budget_pct,
            data.budget.expansion_fund_twd,
            data.budget.tools_budget_usd,
        );

        // 明日排程
        text += "\n--- 排程 ---\n";
        for entry in &data.tomorrow_schedule {
            text += &format!("{} {}\n", entry.time_twd, entry.hand_name);
        }

        // 告警
        if !data.active_alerts.is_empty() {
            text += "\n--- 告警 ---\n";
            for alert in &data.active_alerts {
                let level = match alert.level {
                    AlertLevel::Critical => "[!]",
                    AlertLevel::Warning => "[W]",
                    _ => "[i]",
                };
                text += &format!("{} {}\n", level, alert.message);
            }
        }

        text += "============================";
        Ok(text)
    }

    /// 構建排程條目
    fn build_schedule_entries(&self) -> Vec<ScheduleEntry> {
        vec![
            ScheduleEntry { time_twd: "05:00".into(), hand_name: "market_intel".into(), description: "市場機會掃描".into() },
            ScheduleEntry { time_twd: "06:00".into(), hand_name: "lead".into(), description: "潛在客戶搜尋".into() },
            ScheduleEntry { time_twd: "07:00".into(), hand_name: "freelancer".into(), description: "工作機會搜尋".into() },
            ScheduleEntry { time_twd: "08:00".into(), hand_name: "seo_content".into(), description: "SEO 文章生成".into() },
            ScheduleEntry { time_twd: "09:00".into(), hand_name: "content".into(), description: "社群內容發布".into() },
            ScheduleEntry { time_twd: "10:00".into(), hand_name: "outreach".into(), description: "冷郵件發送".into() },
            ScheduleEntry { time_twd: "12:00".into(), hand_name: "trading_analysis".into(), description: "市場分析報告".into() },
            ScheduleEntry { time_twd: "18:00".into(), hand_name: "auto_report".into(), description: "每日營運報告".into() },
            ScheduleEntry { time_twd: "20:00".into(), hand_name: "customer_service".into(), description: "客戶諮詢回覆".into() },
            ScheduleEntry { time_twd: "22:00".into(), hand_name: "self_optimize".into(), description: "自我優化分析".into() },
        ]
    }

    // ── 告警查詢 ──────────────────────────────────────────────────

    /// 取得最近的告警
    pub async fn recent_alerts(&self, limit: usize) -> Vec<Alert> {
        let alerts = self.alerts.read().await;
        alerts.iter().rev().take(limit).cloned().collect()
    }

    /// 取得預算狀態
    pub async fn get_budget(&self) -> BudgetState {
        self.budget.read().await.clone()
    }

    /// 更新預算 (手動或自動)
    pub async fn update_budget(&self, budget: BudgetState) {
        *self.budget.write().await = budget;
    }
}
```

### 8.2 self_optimize Hand 定義

```toml
# ~/.clawtex/hands/self_optimize/hand.toml

name = "self_optimize"
description = "每日自我優化: 分析營收/成本數據, 調整排程策略"
category = "optimization"
provider = "auto"

tools = [
    "memory_recall",
    "memory_store",
    "web_search",
    "file_write"
]

[settings]
analysis_window_days = "7"
min_roi_threshold = "0.0"
max_frequency_multiplier = "2.0"
cost_alert_threshold = "0.8"

[[phases]]
name = "collect_data"
system_prompt = """
你是 Clawtex 營收優化分析師。
任務: 收集並整理今日的營收和成本數據。

步驟:
1. 使用 memory_recall 取得 key="daily_revenue_summary" 的最新數據
2. 使用 memory_recall 取得 key="daily_cost_summary" 的最新數據
3. 使用 memory_recall 取得 key="route_performance" 的歷史表現

輸出格式:
- 今日營收: $X.XX (N 筆)
- 今日成本: $X.XX (N 次 LLM 呼叫)
- 各路線營收明細
- 各路線成本明細
"""
max_rounds = 5

[[phases]]
name = "analyze_roi"
system_prompt = """
基於上一階段收集的數據, 分析各路線的 ROI。

計算方式:
- ROI = (營收 - 成本) / 成本 * 100%
- 如果成本為 0 且有營收 = 無限大 ROI (好事)
- 如果成本為 0 且無營收 = 0 ROI (未活躍)

識別:
1. TOP 路線: ROI > 100% 的路線
2. 問題路線: 連續 3+ 天零營收
3. 虧損路線: ROI < -50%
4. 趨勢: 營收上升/下降/持平

輸出: 排名表 + 問題分析
"""
max_rounds = 3

[[phases]]
name = "generate_strategy"
system_prompt = """
基於 ROI 分析結果, 生成明日策略調整建議。

原則:
- 保守調整: 每次只改 1-2 個路線
- 不要完全停止任何路線 (至少每週一次)
- 優先增加已證明賺錢的路線
- 成本超標時優先切到免費 provider

輸出格式:
```
排程調整:
- [路線X]: 從每天1次 -> 每天2次 (原因: ROI 最高)
- [路線Y]: 從每天1次 -> 每週3次 (原因: 連續零營收)

Provider 調整:
- [情況]: 切換到 [provider] (原因: 成本控制)

告警:
- [級別]: [訊息]
```

使用 memory_store 存儲策略到 key="optimization_strategy_YYYY-MM-DD"
使用 file_write 保存到 ~/.clawtex/workspace/optimization_report.md
"""
max_rounds = 5

[[phases]]
name = "apply_changes"
system_prompt = """
基於策略建議, 準備具體的執行指令。

注意: 你不能直接修改 cron 排程, 但你可以:
1. 生成建議的 cron 表達式變更清單
2. 使用 memory_store 存儲待執行的變更
3. RevenueEngine 會在下一個 tick 讀取並執行

使用 memory_store 存儲:
- key="pending_schedule_changes": JSON 格式的排程變更
- key="pending_provider_changes": JSON 格式的 provider 切換
- key="daily_optimization_log": 今日優化日誌

輸出: 確認所有變更已準備好
"""
max_rounds = 3
```

### 8.3 growth_review Hand 定義

```toml
# ~/.clawtex/hands/growth_review/hand.toml

name = "growth_review"
description = "季度增長回顧: 90天數據分析 + 策略建議"
category = "strategy"
provider = "auto"

tools = [
    "memory_recall",
    "memory_store",
    "web_search",
    "file_write",
    "pdf_export"
]

[settings]
review_period_days = "90"
currency = "USD"

[[phases]]
name = "data_collection"
system_prompt = """
收集過去 90 天的完整營運數據:
- 各路線月度營收
- 各 provider 月度成本
- 各 Hand 執行次數和成功率
- 客戶數量和客單價變化

使用 memory_recall 獲取所有 optimization_strategy_* 記錄
使用 memory_recall 獲取 ab_test_* 結果
"""
max_rounds = 5

[[phases]]
name = "deep_analysis"
system_prompt = """
進行深入分析:
1. 月度 ROI 趨勢 (M1 vs M2 vs M3)
2. 最佳路線 (持續盈利)
3. 最差路線 (持續虧損)
4. 客戶獲取成本 (CAC)
5. 客戶生命周期價值 (LTV)
6. 投入產出比

使用 web_search 研究市場趨勢
"""
max_rounds = 5

[[phases]]
name = "strategy_recommendations"
system_prompt = """
基於數據分析生成下季度策略:
1. 要新開哪些路線?
2. 要關閉哪些路線?
3. 定價調整建議
4. 硬體擴展建議
5. API 預算調整
6. 新工具/服務購買建議
7. 增長目標設定

輸出結構化建議, 每項標明優先級和預期 ROI
"""
max_rounds = 5

[[phases]]
name = "report_generation"
system_prompt = """
生成正式季度報告:
1. 使用 file_write 寫入 ~/.clawtex/workspace/quarterly_review_YYYY_Q*.md
2. 使用 pdf_export 生成 PDF 版本
3. 使用 memory_store 存儲 key="quarterly_review_YYYY_Q*"

報告格式:
# Clawtex 季度營運報告 (YYYY Q*)
## 執行摘要
## 營收概覽
## 路線分析
## 成本分析
## 趨勢與機會
## 下季度策略
## 行動計劃
"""
max_rounds = 5
```

---

## 9. 決策流程圖

### 9.1 每日優化主流程

```
               [每日 22:00 觸發]
                      |
              +-------+-------+
              | 拉取 7 天數據  |
              | Revenue + Cost |
              +-------+-------+
                      |
              +-------+-------+
              | 計算各路線 ROI |
              +-------+-------+
                      |
        +-------------+-------------+
        |             |             |
   [top_routes]  [dead_routes]  [bleeding]
   ROI > 0       0收入 >= 3天   ROI < -100%
        |             |             |
   增加頻率      問題診斷      暫停排程
   (x1.5)       (分析日誌)    (通知人類)
        |             |             |
        +------+------+------+-----+
               |             |
        [成本檢查]      [趨勢檢查]
               |             |
        >80% 預算?      下降 >50%?
        |     |         |     |
       Yes    No       Yes    No
        |     |         |     |
    切免費  保持     緊急告警  正常
    provider         調整策略
        |     |         |     |
        +-----+---------+-----+
                   |
           [生成決策報告]
           [存入 memory]
           [推送 Telegram]
```

### 9.2 每週結算流程

```
         [每週日 23:00]
               |
       +-------+-------+
       | 計算週營收/成本 |
       +-------+-------+
               |
          淨利 > 0?
         /         \
        Yes         No
       /             \
  60% 擴展基金     發送告警
  20% API 額度     建議減少
  20% 工具預算     付費 API
       |               |
  更新每日限制     凍結預算
       |
  基金 > NT$15K?
  /         \
 Yes         No
 /             \
通知買機器    繼續累積
```

### 9.3 失敗恢復流程

```
    [Hand 執行完成]
          |
     成功?  失敗?
    /           \
  記錄營收    記錄失敗
    |             |
    |        連續失敗 >= 3?
    |        /         \
    |      Yes          No
    |      /              \
    | 觸發診斷          等待重試
    | 收集日誌
    | 分析原因
    |      |
    | +----+----+----+
    | | Tool    | LLM | 外部
    | | 失敗    | 品質 | 因素
    | |         |      |
    | | 換 Tool | 換   | 改
    | | 更新參數| Model| Prompt
    | |         |      |
    | +----+----+----+
    |      |
    |  測試執行
    |  /      \
    | 成功    失敗
    | /          \
    | 恢復排程   降至每週
```

---

## 10. 實施計劃

### Sprint 1: 核心引擎 (Day 1-2)

```
[ ] src/revenue_engine.rs — RevenueEngine struct + ROI 計算
[ ] RouteROI, BudgetState, Alert 數據結構
[ ] calculate_all_roi() 方法
[ ] run_optimization_loop() 方法
[ ] weekly_settlement() 方法
[ ] generate_dashboard() + format_dashboard_telegram()
[ ] lib.rs 註冊模組
[ ] 15+ 單元測試
```

### Sprint 2: 排程整合 (Day 2-3)

```
[ ] register_default_schedule() — 10 個每日排程
[ ] self_optimize hand TOML
[ ] growth_review hand TOML
[ ] main.rs — RevenueEngine 整合到 AppState
[ ] cron executor — Hand 執行完後觸發 revenue_engine 記錄
[ ] weekly_settlement cron job
[ ] 排程動態調整 (增加/減少頻率)
```

### Sprint 3: Telegram + HTTP (Day 3-4)

```
[ ] /dashboard Telegram 指令
[ ] /roi, /trend, /budget, /fund 指令
[ ] /optimize 手動觸發
[ ] /diagnose <route> 手動診斷
[ ] GET /dashboard HTTP 端點
[ ] GET /dashboard/roi
[ ] GET /dashboard/trend
[ ] POST /dashboard/optimize
[ ] 日報/週報自動推送
```

### Sprint 4: 進階功能 (Day 4-5)

```
[ ] 失敗恢復: 自動診斷 + 策略調整
[ ] A/B 測試框架 (memory-based)
[ ] 季度回顧自動觸發
[ ] 擴展門檻通知
[ ] provider 自動切換邏輯
[ ] BudgetState SQLite 持久化
[ ] 20+ 整合測試
```

### 測試策略

```
單元測試:
- ROI 計算正確性 (各種 edge case: 零成本/零營收/負 ROI)
- 告警觸發邏輯 (Level 1-4)
- 預算分配算法
- 趨勢計算
- 儀表板格式化

整合測試:
- RevenueEngine + RevenueTracker + CostTracker 聯動
- 優化迴圈 -> 排程調整 -> cron 更新
- weekly_settlement -> 預算更新 -> 每日限制變化
- 連續零營收 -> 診斷觸發 -> 降頻
- 成本超標 -> provider 切換

E2E 測試:
- 完整 24 小時模擬 (時間快轉)
- Telegram 指令回應
- HTTP API 回應格式
```

---

## 附錄 A: 配置項

```toml
# ~/.clawtex/agents.toml 新增區段

[revenue_engine]
# 預設每日 API 預算 (USD)
daily_api_limit = 5.0
# 硬性上限 (不可超過)
daily_hard_limit = 20.0
# USD to TWD 匯率
usd_to_twd = 32.0
# 擴展基金門檻 (TWD)
expansion_threshold_1 = 15000
expansion_threshold_2 = 30000
# 淨利分配比例
profit_expansion_pct = 60
profit_api_pct = 20
profit_tools_pct = 20
# 告警: 連續零營收天數門檻
zero_revenue_alert_days = 3
# 告警: 整體營收下降百分比門檻
revenue_drop_warning_pct = 25
revenue_drop_critical_pct = 50
# 優化: 最大頻率倍數
max_frequency_multiplier = 2.0
# 優化: 最小頻率 (每週幾次)
min_weekly_frequency = 1
```

## 附錄 B: 新增 SQLite 表

```sql
-- 預算狀態持久化
CREATE TABLE IF NOT EXISTS budget_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),  -- 只有一筆記錄
    expansion_fund_twd REAL NOT NULL DEFAULT 0.0,
    api_budget_usd REAL NOT NULL DEFAULT 0.0,
    tools_budget_usd REAL NOT NULL DEFAULT 0.0,
    daily_api_limit_usd REAL NOT NULL DEFAULT 5.0,
    daily_hard_limit_usd REAL NOT NULL DEFAULT 20.0,
    last_settlement TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- 優化決策歷史
CREATE TABLE IF NOT EXISTS optimization_history (
    id TEXT PRIMARY KEY,
    timestamp TEXT NOT NULL,
    decision_json TEXT NOT NULL,
    summary TEXT NOT NULL,
    applied INTEGER NOT NULL DEFAULT 0
);

-- A/B 測試
CREATE TABLE IF NOT EXISTS ab_tests (
    id TEXT PRIMARY KEY,
    route TEXT NOT NULL,
    variable TEXT NOT NULL,
    variant_a_json TEXT NOT NULL,
    variant_b_json TEXT NOT NULL,
    start_date TEXT NOT NULL,
    end_date TEXT NOT NULL,
    result_json TEXT,
    status TEXT NOT NULL DEFAULT 'running'
);

-- 路線排程映射
CREATE TABLE IF NOT EXISTS route_schedules (
    route TEXT NOT NULL,
    cron_job_id TEXT NOT NULL,
    hand_name TEXT NOT NULL,
    frequency TEXT NOT NULL DEFAULT 'daily',
    is_active INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (route, cron_job_id)
);
```

## 附錄 C: 路線-Hand-排程 完整映射表

```
路線                  | 主 Hand          | 排程(TWD)  | 狀態
────────────────────  | ───────────────  | ─────────  | ──────
A: freelance_dev      | freelancer       | 07:00 daily| Active
B: saas_products      | product_spec     | on-demand  | Active
C: content_monetiz.   | seo_content      | 08:00 daily| Active
                      | content          | 09:00 daily|
D: consulting         | lead             | 06:00 daily| Active
                      | outreach         | 10:00 daily|
E: api_services       | code_gen         | on-demand  | Active
F: affiliate_market.  | seo_content      | (共用C)    | Active
                      | content          | (共用C)    |
G: digital_products   | product_spec     | (共用B)    | Active
                      | content          | (共用C)    |
H: automation_svcs    | lead             | (共用D)    | Active
                      | outreach         | (共用D)    |
I: data_services      | market_intel     | 05:00 daily| Active
J: training_educ.     | content          | (共用C)    | Active
                      | seo_content      | (共用C)    |
-- 輔助 --
-                     | trading_analysis | 12:00 daily| Active
-                     | auto_report      | 18:00 daily| Active
-                     | customer_service | 20:00 daily| Active
-                     | self_optimize    | 22:00 daily| Active
```
