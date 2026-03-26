# Phantom Mesh 賺錢方案 — 完整執行流程分析

> 文件建立日期: 2026-03-03
> 最後更新: 2026-03-03

---

## 目錄

1. [系統架構總覽](#系統架構總覽)
2. [7 個賺錢 Hands 詳細分析](#7-個賺錢-hands-詳細分析)
   - [1. Outreach — 冷郵件銷售](#1-outreach--冷郵件銷售)
   - [2. Freelancer — 自由接案](#2-freelancer--自由接案)
   - [3. SEO Content — SEO 文章生產](#3-seo-content--seo-文章生產)
   - [4. Market Intel — 市場情報](#4-market-intel--市場情報)
   - [5. Lead — 潛在客戶開發](#5-lead--潛在客戶開發)
   - [6. Researcher — 深度研究](#6-researcher--深度研究)
   - [7. Content — 社群內容生產](#7-content--社群內容生產)
3. [工具與基礎設施](#工具與基礎設施)
4. [實際收入估算](#實際收入估算)
5. [Hands 組合策略（流水線）](#hands-組合策略流水線)
6. [部署架構圖](#部署架構圖)
7. [下一步待完成項目](#下一步待完成項目)

---

## 系統架構總覽

```
使用者 (Telegram)
    ↓ 發送指令: /hand outreach "web design for restaurants"
phantom-mesh daemon (Rust, port 7878)
    ├── Telegram Handler → 解析指令
    ├── Hand Registry → 載入 ~/.phantom-mesh/hands/<name>/hand.toml
    ├── Hand Runner → 逐 Phase 執行
    │     ├── Phase 1 → Agent + Tools → 輸出
    │     ├── Phase 2 → Agent + Tools + Phase1輸出 → 輸出
    │     ├── ...
    │     └── Phase N → 最終結果
    ├── Tool Registry (15 工具)
    │     ├── web_search (Serper/Tavily)
    │     ├── browser (Playwright)
    │     ├── email_send (SMTP, 需要 Approval)
    │     ├── file_write/read/edit
    │     ├── memory_store/recall/forget
    │     ├── vision (Gemini/Groq)
    │     └── ... 其他
    ├── Approval Gate → Telegram 人工確認
    ├── LLM Router
    │     ├── LM Studio (本地, qwen3-coder)
    │     ├── Ollama (本地備援)
    │     ├── Gemini API (雲端, 視覺)
    │     └── Groq API (雲端, 視覺備援)
    └── 輸出 → ~/.phantom-mesh/workspace/
```

**核心流程**: 每個 Hand 是一個 TOML 定義的多階段工作流，Phase 之間用上下文串聯。

---

## 7 個賺錢 Hands 詳細分析

---

### 1. Outreach — 冷郵件銷售

**類別**: sales
**目的**: 自動化冷郵件行銷 — 找潛在客戶、個性化郵件、發送跟進序列
**收入模式**: 服務銷售 (每客戶 $500-5000/月)

#### 執行流程 (4 Phases)

```
輸入: "web design services for restaurants in Taipei"
         ↓
┌─────────────────────────────────────────────┐
│ Phase 1: prospect_research (max 10 rounds)  │
│ 工具: web_search, browser                    │
│ 動作:                                        │
│   1. 搜尋目標產業的公司                        │
│   2. 瀏覽公司網站收集聯絡資訊                   │
│   3. 分析公司痛點和需求信號                     │
│ 輸出: 15-25 個潛在客戶清單                     │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 2: prospect_scoring (max 3 rounds)    │
│ 工具: 無 (純 LLM 分析)                       │
│ 動作:                                        │
│   1. 按 ICP 適配度評分 (0-100)                │
│   2. 評估: 痛點信號、預算能力、可接觸性         │
│   3. 排名取前 5-10 名                         │
│ 輸出: 評分排序的潛在客戶清單                    │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 3: email_generation (max 5 rounds)    │
│ 工具: file_write                             │
│ 動作:                                        │
│   1. 為每個客戶寫 3 封個性化郵件               │
│      - Email 1: 初次接觸                      │
│      - Email 2: 3 天後跟進                    │
│      - Email 3: 5 天後最後機會                 │
│   2. 儲存到 outreach_emails.md                │
│ 輸出: 15-30 封個性化郵件                       │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 4: send_and_track (max 8 rounds)      │
│ 工具: email_send, file_write, memory_store   │
│ ⚠️ 需要 Telegram Approval Gate               │
│ 動作:                                        │
│   1. 發送 Telegram 確認: "要發送郵件嗎？"      │
│   2. 等待用戶 /approve 或 /deny (5分鐘超時)   │
│   3. 收到 approve → 發送 Email 1              │
│   4. 記錄到 outreach_tracker.csv              │
│   5. memory_store 記住發送狀態                 │
│ 輸出: outreach_tracker.csv, outreach_report.md│
└─────────────────────────────────────────────┘
```

#### 設定參數
| 參數 | 說明 | 範例 |
|------|------|------|
| `service_offering` | 販售的服務 | "web design" |
| `target_industry` | 目標產業 | "restaurants" |
| `target_location` | 地理範圍 | "Taipei" |
| `max_prospects` | 最大客戶數 | 20 |
| `email_style` | 郵件風格 | professional/casual/witty |

#### 產出檔案
- `outreach_emails.md` — 所有個性化郵件 (可預覽編輯)
- `outreach_tracker.csv` — 公司, 聯絡人, 郵件, 發送日期, 狀態, 跟進日期
- `outreach_report.md` — 總結: 潛在客戶數, 已發送數, 前3名線索

#### 💰 收入估算
- 執行時間: 5-10 分鐘
- 回覆率: 2-5% (20 封 → 0-2 回覆)
- 每個回覆潛在價值: $500-5000/月
- **保守估計: $1,000-4,000/次執行**

---

### 2. Freelancer — 自由接案

**類別**: sales
**目的**: 自動搜尋接案平台，找到匹配工作，撰寫提案
**收入模式**: 接案收入 (每案 $500-2000+)

#### 執行流程 (4 Phases)

```
輸入: "AI automation, web dev, data analysis"
         ↓
┌─────────────────────────────────────────────┐
│ Phase 1: job_search (max 10 rounds)         │
│ 工具: web_search, browser                    │
│ 動作:                                        │
│   1. 搜尋 Upwork, Freelancer, Toptal         │
│   2. 瀏覽工作列表，篩選預算 ≥ $500            │
│   3. 收集 15-20 個工作機會                     │
│ 輸出: 工作機會清單 (含預算、描述、客戶資訊)     │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 2: opportunity_scoring (max 3 rounds) │
│ 工具: 無 (純 LLM 分析)                       │
│ 動作:                                        │
│   1. 評分 0-100: 技能匹配、預算品質            │
│   2. 分析: 客戶品質、競爭程度、成長潛力         │
│   3. 取前 5-10 名                             │
│ 輸出: 排序過的工作機會                         │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 3: proposal_generation (max 5 rounds) │
│ 工具: file_write                             │
│ 動作:                                        │
│   1. 為每個工作寫 <200 字提案                  │
│   2. 強調相關經驗 + 具體方案                   │
│   3. 儲存到 proposals.md                      │
│ 輸出: 5-10 份提案 (可直接複製貼上)             │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 4: application_prep (max 3 rounds)    │
│ 工具: file_write, memory_store               │
│ 動作:                                        │
│   1. 為每個工作建立客製化 cover letter          │
│   2. 列出相關作品集項目                        │
│   3. 設定申請截止日提醒                        │
│ 輸出: job_opportunities.csv, freelance_report.md│
└─────────────────────────────────────────────┘
```

#### 設定參數
| 參數 | 說明 | 範例 |
|------|------|------|
| `skills` | 你的技能 | "web dev, AI automation" |
| `min_budget` | 最低預算 | 500 |
| `platforms` | 平台 | "upwork, freelancer, toptal" |
| `max_jobs` | 搜尋數量 | 10 |
| `experience_level` | 經驗等級 | expert/intermediate |

#### 💰 收入估算
- 執行時間: 8-15 分鐘
- 提案成功率: 5-15%
- **保守估計: $100-500/次執行 (1 個成功案子)**
- 重複執行建立回頭客 → 被動收入

---

### 3. SEO Content — SEO 文章生產

**類別**: content
**目的**: 自動產出 SEO 優化長文，產生被動搜尋流量 + 聯盟行銷收入
**收入模式**: 廣告 + 聯盟行銷 (每篇 $50-500/月被動收入)

#### 執行流程 (4 Phases)

```
輸入: "Best AI tools for freelancers"
         ↓
┌─────────────────────────────────────────────┐
│ Phase 1: keyword_research (max 8 rounds)    │
│ 工具: web_search, browser                    │
│ 動作:                                        │
│   1. 搜尋主要關鍵詞 + 相關詞                  │
│   2. 分析搜尋意圖 (商業/資訊/交易)             │
│   3. 找出 1 主 + 3-5 次要 + 5-10 LSI 關鍵詞  │
│   4. 收集 "People Also Ask" 問題              │
│ 輸出: keywords.csv (關鍵詞, 意圖, 競爭度, 優先級)│
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 2: competitor_analysis (max 8 rounds) │
│ 工具: web_search, browser                    │
│ 動作:                                        │
│   1. 分析 Google 前 5 名結果                  │
│   2. 記錄: 標題、字數、結構、缺口              │
│   3. 找出差異化角度                            │
│ 輸出: 競爭對手分析報告                         │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 3: article_writing (max 5 rounds)     │
│ 工具: file_write                             │
│ 動作:                                        │
│   1. 撰寫 1500+ 字 SEO 文章                  │
│   2. 結構: 標題 → 引言 → H2/H3 → FAQ         │
│   3. 嵌入關鍵詞 + [AFFILIATE] 推薦連結        │
│   4. 儲存 article.md                          │
│ 輸出: article.md (1500-15000 字)              │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 4: seo_optimization (max 3 rounds)    │
│ 工具: file_write                             │
│ 動作:                                        │
│   1. SEO 檢查清單 (標題、meta、關鍵詞密度)     │
│   2. 可讀性評分                               │
│   3. 優化後產出 article_final.md              │
│ 輸出: article_final.md, seo_report.md         │
└─────────────────────────────────────────────┘
```

#### 設定參數
| 參數 | 說明 | 範例 |
|------|------|------|
| `topic` | 文章主題 | "Best AI tools for freelancers" |
| `target_audience` | 目標讀者 | "freelance developers" |
| `content_length` | 目標字數 | 1500 |
| `language` | 語言 | en / zh-tw |
| `monetization` | 變現模式 | affiliate / ad_revenue |

#### 💰 收入估算
- 執行時間: 15-20 分鐘
- 每篇文章被動收入: $50-500/月 (AdSense + 聯盟行銷)
- **10 篇文章 × $100/月 = $1,000/月被動收入**
- **複利效應: 1 年 50 篇 = $5,000+/月**

---

### 4. Market Intel — 市場情報

**類別**: research
**目的**: 自動化市場調研 — 了解競爭格局、定價、機會，用於產品上市前驗證
**收入模式**: 避免錯誤投資 + 發現套利機會

#### 執行流程 (4 Phases)

```
輸入: "AI automation tools market"
         ↓
┌─────────────────────────────────────────────┐
│ Phase 1: market_overview (max 8 rounds)     │
│ 工具: web_search, browser                    │
│ 動作:                                        │
│   1. 調查 TAM (市場總量)、成長率               │
│   2. 分析: 關鍵細分、趨勢、進入障礙            │
│   3. 了解分銷渠道                             │
│ 輸出: 市場概覽報告                             │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 2: competitor_mapping (max 10 rounds) │
│ 工具: web_search, browser, memory_store      │
│ 動作:                                        │
│   1. 找出 8-12 個競爭對手                     │
│   2. 分析: 產品、定價、USP、優劣勢、評價       │
│   3. 記錄到 memory 方便後續追蹤                │
│ 輸出: competitors.csv                         │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 3: pricing_analysis (max 5 rounds)    │
│ 工具: file_write                             │
│ 動作:                                        │
│   1. 收集定價數據: 方案、功能、目標客群         │
│   2. 分析定價模式: 訂閱/使用量/免費增值         │
│   3. 儲存 pricing_analysis.csv                │
│ 輸出: pricing_analysis.csv                    │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 4: opportunity_identification (5 rds) │
│ 工具: file_write, memory_store               │
│ 動作:                                        │
│   1. 找出市場缺口 + 未被服務的細分市場          │
│   2. 識別: 趨勢衝浪、定價套利、分銷缺口        │
│   3. 儲存 opportunities.json                  │
│ 輸出: market_intelligence.md, opportunities.json│
└─────────────────────────────────────────────┘
```

#### 💰 收入估算
- 執行時間: 20-30 分鐘
- **價值: 避免 $10K+ 的錯誤投資**
- 發現套利機會 → 可直接轉化為新產品/服務
- 也可以賣市場報告 ($500-2000/份)

---

### 5. Lead — 潛在客戶開發

**類別**: sales
**目的**: 自動找到匹配 ICP (理想客戶畫像) 的公司 + 聯絡人
**收入模式**: 銷售管道的源頭 (配合 Outreach Hand 使用)

#### 執行流程 (4 Phases)

```
輸入: "e-commerce companies in Taiwan"
         ↓
┌─────────────────────────────────────────────┐
│ Phase 1: icp_definition (max 3 rounds)      │
│ 工具: 無                                     │
│ 動作: 定義理想客戶畫像                        │
│   - 產業、規模、地理、痛點、決策者             │
│ 輸出: ICP 定義文件                            │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 2: web_research (max 8 rounds)        │
│ 工具: web_search, browser                    │
│ 動作:                                        │
│   1. 搜尋匹配 ICP 的公司                      │
│   2. 收集: 網站、聯絡人、新聞、信號            │
│   3. 找 10-20 家公司                          │
│ 輸出: 公司清單 (含詳細資訊)                    │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 3: lead_scoring (max 3 rounds)        │
│ 工具: 無                                     │
│ 動作:                                        │
│   - 評分 0-100: ICP 適配、時機信號             │
│   - 排序取前 10 名                             │
│ 輸出: 評分後的潛在客戶清單                     │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 4: report_generation (max 5 rounds)   │
│ 工具: file_write                             │
│ 動作:                                        │
│   1. 輸出 leads_data.csv                      │
│   2. 撰寫 leads_report.md 摘要               │
│ 輸出: leads_data.csv, leads_report.md         │
└─────────────────────────────────────────────┘
```

#### 已驗證的實際輸出 (台灣電商)
| 公司 | 評分 | 信號 |
|------|------|------|
| PureGlow Skincare | 94 | 部落格有 AI 聊天機器人需求 |
| EcoLife TW | 92 | 種子輪資金到位，預算充足 |
| UrbanTaste | 89 | 正在招聘物流專員 = 痛點 |
| StyleSync TW | 86 | 時尚科技新創，創辦人主導 |
| ShopSwift | 84 | LinkedIn 上明確表示物流困難 |

#### 💰 收入估算
- 執行時間: 10-15 分鐘
- 20 leads × 5% 成交率 × $2000 = **$2,000/次執行**
- **搭配 Outreach Hand = 自動化銷售漏斗**

---

### 6. Researcher — 深度研究

**類別**: research
**目的**: 全面深入研究任何主題，產出專業研究報告
**收入模式**: 白皮書銷售 / 付費報告 / 顧問知識庫

#### 執行流程 (5 Phases)

```
輸入: "Local LLM trends in 2025"
         ↓
┌─────────────────────────────────────────────┐
│ Phase 1: question_decomposition (3 rounds)  │
│ 工具: 無                                     │
│ 動作: 拆解主題為 3-5 個核心研究問題            │
│ 輸出: 結構化研究計畫                          │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 2: multi_source_research (10 rounds)  │
│ 工具: web_search, browser, memory_store      │
│ 動作:                                        │
│   1. 找 10-15 個高品質來源                    │
│   2. CRAAP 評估法: 時效性、相關性、權威性      │
│ 輸出: 經過評估的來源列表 + 關鍵發現            │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 3: cross_referencing (max 5 rounds)   │
│ 工具: memory_recall                          │
│ 動作: 交叉驗證、找出矛盾、評估可信度           │
│ 輸出: 驗證後的發現 + 可信度評分                │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 4: synthesis (max 3 rounds)           │
│ 工具: 無                                     │
│ 動作: 綜合分析 → 行政摘要 + 關鍵發現 + 影響    │
│ 輸出: 結構化研究敘述                          │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 5: report_generation (max 5 rounds)   │
│ 工具: file_write                             │
│ 動作: 輸出完整報告 + 引用 + 方法論 + 參考文獻  │
│ 輸出: research_report.md (~9000 字)           │
└─────────────────────────────────────────────┘
```

#### 💰 收入估算
- 執行時間: 20-30 分鐘
- 白皮書: $2K-5K/份
- Gumroad 付費報告: $500/份
- **10 份報告 × $500 = $5,000+**

---

### 7. Content — 社群內容生產

**類別**: marketing
**目的**: 產出社群媒體內容 — 推文、串文、文章、郵件文案
**收入模式**: 個人品牌 → 贊助 / 電子報 / 課程

#### 執行流程 (3 Phases)

```
輸入: "AI trends for developers"
         ↓
┌─────────────────────────────────────────────┐
│ Phase 1: topic_research (max 5 rounds)      │
│ 工具: web_search                             │
│ 動作: 找趨勢角度、病毒式模式、關鍵統計數據     │
│ 輸出: 研究素材                               │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 2: content_generation (max 5 rounds)  │
│ 工具: file_write                             │
│ 動作:                                        │
│   1. 5 條推文變體                             │
│   2. 2 個串文構想 (各 3-5 條)                  │
│   3. 1 篇 800-1500 字完整文章                 │
│   4. 5 個郵件主旨 + 正文                      │
│ 輸出: content_output.md                       │
└──────────────────┬──────────────────────────┘
                   ↓
┌─────────────────────────────────────────────┐
│ Phase 3: quality_review (max 3 rounds)      │
│ 工具: file_write                             │
│ 動作: 審核準確性、品牌聲音、參與度、合規性      │
│ 輸出: content_output.md (最終版), content_queue.json│
└─────────────────────────────────────────────┘
```

#### 💰 收入估算
- 執行時間: ~10 分鐘
- 贊助貼文: $500-2K/條
- 電子報: 1000 訂閱者 × $5/月 = $5K/月
- 課程: 內容作為引流 → $10K+ 課程銷售

---

## 工具與基礎設施

### 核心工具 (15 個)

| 工具 | 說明 | 需要 API Key | 需要 Approval |
|------|------|-------------|--------------|
| `web_search` | 網路搜尋 | Serper (2500次免費/月) | 否 |
| `browser` | Playwright 瀏覽器自動化 | 無 | 否 |
| `file_write` | 寫檔到 workspace | 無 | 否 |
| `file_read` | 讀 workspace 檔案 | 無 | 否 |
| `file_edit` | 編輯既有檔案 | 無 | 否 |
| `email_send` | SMTP 發郵件 | SMTP 帳密 | ⚠️ 是 |
| `http_request` | HTTP API 呼叫 | 視情況 | ⚠️ POST/PUT/DELETE 需要 |
| `vision` | 圖片分析 | Gemini (免費) | 否 |
| `memory_store` | 儲存記憶 | 無 | 否 |
| `memory_recall` | 回憶記憶 | 無 | 否 |
| `memory_forget` | 忘記記憶 | 無 | 否 |
| `glob_search` | 檔案搜尋 | 無 | 否 |
| `content_search` | 內容搜尋 | 無 | 否 |
| `delegate` | 委託其他 agent | 無 | 否 |
| `ai_code` | AI 程式碼生成 | 無 | 否 |

### Approval Gate 機制

```
Agent 呼叫敏感工具 (email_send / http POST)
    ↓
系統攔截 → 發送 Telegram 訊息:
  ⚠️ Approval Required
  Tool: email_send
  Action: Send email to xxx@example.com
  Reply: /approve abc123 or /deny abc123
  (5 分鐘後自動拒絕)
    ↓
用戶 /approve → 執行
用戶 /deny → 跳過
超時 → 自動拒絕
```

### 外部 API 使用量

| 服務 | 免費額度 | 用途 |
|------|---------|------|
| Serper | 2500 次/月 | 網路搜尋 (主要) |
| Tavily | 1000 次/月 | 網路搜尋 (備援) |
| Gemini | 免費 tier | 視覺分析 |
| Groq | 免費 tier | 視覺備援 |

---

## 實際收入估算

### 保守月收入 (每天執行 1 次)

| Hand | 頻率 | 單次收入 | 月收入 |
|------|------|---------|--------|
| Outreach | 4次/週 | $1,000-4,000 (成交後) | $4,000-16,000 |
| Freelancer | 5次/週 | $100-500 | $2,000-10,000 |
| SEO Content | 2次/週 | $100/月被動 | $800/月 (累積中) |
| Lead | 2次/週 | $500-2,000 (成交後) | $4,000-16,000 |
| Content | 每天 | 間接 (品牌建設) | $500-5,000 |
| Market Intel | 1次/月 | 避免損失 | 防禦性價值 |
| Researcher | 2次/月 | $500-2,000/報告 | $1,000-4,000 |

### 第一個月 (最現實估算)
- **Freelancer**: 40 個提案 → 2-4 個成交 → **$1,000-4,000**
- **SEO Content**: 8 篇文章 → 3-6 個月後開始有流量 → **$0 (投資期)**
- **Lead + Outreach**: 80 封郵件 → 2-4 個回覆 → 1 成交 → **$500-2,000**
- **總計: $1,500-6,000/月**

### 第六個月 (複利效應)
- **Freelancer**: 回頭客 + 新案 → **$3,000-8,000/月**
- **SEO Content**: 40+ 篇文章產生流量 → **$2,000-5,000/月被動**
- **Lead + Outreach**: 建立客戶群 → **$5,000-15,000/月**
- **Content**: 建立品牌 → 贊助 + 電子報 → **$1,000-5,000/月**
- **總計: $11,000-33,000/月**

---

## Hands 組合策略（流水線）

### 策略 1: 銷售漏斗自動化

```
Market Intel (了解市場)
    ↓
Lead (找潛在客戶)
    ↓
Researcher (深入了解客戶痛點)
    ↓
Outreach (發送冷郵件)
    ↓
跟進 → 成交
```

### 策略 2: 內容行銷飛輪

```
Researcher (主題研究)
    ↓
SEO Content (寫 SEO 文章)
    ↓
Content (社群推廣)
    ↓
流量 → 聯盟行銷收入 / 客戶詢問
    ↓
Lead (收集詢問) → Outreach (跟進)
```

### 策略 3: 接案快速啟動

```
Freelancer (找工作)
    ↓
立即申請
    ↓
Content (在 LinkedIn 分享作品)
    ↓
被動詢問 → Lead → Outreach
```

---

## 部署架構圖

```
┌────────────────────────────────────────────────────┐
│                  使用者 (你)                         │
│  Telegram Desktop / 手機                            │
│  指令: /hand outreach "..."                         │
│  審核: /approve abc123                              │
└────────────────────┬───────────────────────────────┘
                     ↓ Telegram Bot API
┌────────────────────────────────────────────────────┐
│           phantom-mesh daemon (Rust)                │
│           localhost:7878                             │
├────────────────────────────────────────────────────┤
│  Telegram Handler  │  HTTP API (Gateway)            │
│  /hand /approve    │  GET /hands                    │
│  /deny /estop      │  POST /hand/:name/run          │
│  /resume           │  GET /workspace/files           │
├────────────────────┤  POST/DELETE/GET /estop         │
│  Hand Runner       │  SSE /stream/agent/:name        │
│  Phase 1→2→3→N     │  WS  /ws/agent/:name            │
├────────────────────┴───────────────────────────────┤
│                Tool Registry (15 工具)               │
│  web_search │ browser │ email │ vision │ file_*     │
│  memory_*   │ http    │ delegate │ ai_code │ ...    │
├────────────────────────────────────────────────────┤
│                LLM Router                           │
│  ┌──────────┐ ┌──────────┐ ┌────────┐ ┌─────────┐ │
│  │LM Studio │ │ Ollama   │ │ Gemini │ │  Groq   │ │
│  │本地:1234  │ │本地:11434│ │雲端免費 │ │雲端免費  │ │
│  │qwen3-coder│ │llama3.2 │ │vision  │ │vision   │ │
│  └──────────┘ └──────────┘ └────────┘ └─────────┘ │
├────────────────────────────────────────────────────┤
│  SQLite (memory.db)  │  Workspace (~/.phantom-mesh/ws/)  │
│  記憶/歷史/狀態       │  CSV, MD, JSON 輸出檔        │
└────────────────────────────────────────────────────┘

外部 API:
  Serper (搜尋) → Tavily (備援) → Brave/EXA (其他)
  Gmail SMTP (郵件發送)
```

---

## 下一步待完成項目

### 優先度高 (直接影響收入)
- [ ] **排程系統**: 定時執行 Hands (如: 每天早上跑一次 Freelancer)
- [ ] **跟進自動化**: Outreach 的 Email 2/3 自動排程發送
- [ ] **收入追蹤儀表板**: 追蹤每個 Hand 的 ROI
- [ ] **成本追蹤**: API 使用量 vs 收入

### 優先度中 (改善效率)
- [ ] **記憶體清理**: SQLite 定期清理舊記憶
- [ ] **Hand 模板**: 可快速複製修改的 Hand 模板
- [ ] **Webhook 整合**: 外部事件觸發 Hand (如: 新郵件到 → 自動回覆)
- [ ] **多用戶**: 支援多個 Telegram 使用者

### 優先度低 (長期)
- [ ] **分析儀表板**: Web UI 顯示統計
- [ ] **資料庫備份/還原**
- [ ] **每個 Hand 類型的速率限制**
- [ ] **完整桌面電腦控制** (OmniParser V2)

---

## 速查: Telegram 指令

| 指令 | 說明 |
|------|------|
| `/hands` | 列出所有可用的 Hands |
| `/hand <name> <prompt>` | 執行指定的 Hand |
| `/approve <id>` | 核准敏感操作 |
| `/deny <id>` | 拒絕敏感操作 |
| `/estop` | 緊急停止所有操作 |
| `/resume` | 恢復運行 |

## 速查: HTTP API

| 端點 | 方法 | 說明 |
|------|------|------|
| `/hands` | GET | 列出所有 Hands |
| `/hand/:name/run` | POST | 執行 Hand (body: `{"prompt": "..."}`) |
| `/workspace/files` | GET | 列出輸出檔案 |
| `/tools` | GET | 列出所有工具 |
| `/estop` | POST | 緊急停止 |
| `/estop` | DELETE | 恢復運行 |
| `/estop` | GET | 查詢狀態 |
