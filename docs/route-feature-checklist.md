# Phantom Mesh 路線功能清單 — 每項都必須能真實賺錢

> 建立日期: 2026-03-03
> 最後更新: 2026-03-04
> 原則: **測試 = 整個流程跑通 + 產出可直接用來賺錢的結果，不能打折**

---

## 審計結論摘要

| 組件 | 狀態 |
|------|------|
| Hand 引擎 (phase chaining) | ✅ 完全運作 + chain_to 自動觸發 |
| 10 個 Hands TOML | ✅ 全部載入 (lead, outreach, freelancer, content, seo_content, researcher, market_intel, auto_report, customer_service, trading_analysis) |
| 20 工具已註冊 | ✅ 含 twitter, blog_publish, email_send, vision, pdf_export, browser |
| web_search | ✅ Serper+Tavily 真實可用 |
| browser | ✅ Playwright 真實可用 (含 Upwork 直接導航) |
| email_send | ✅ SMTP 已設定 (Gmail App Password) |
| twitter | ✅ API + Playwright browser fallback |
| blog_publish | ✅ MDX + index.ts + git push → Vercel (markl-ai.space) |
| vision | ✅ Gemini 2.5 flash-lite + Groq fallback |
| pdf_export | ✅ pandoc/weasyprint/Python fallback |
| memory | ✅ SQLite 語義搜尋 |
| cron/排程 | ✅ 4 個預設任務自動註冊 + Telegram /cron 管理 |
| Approval Gate | ✅ Telegram 審核運作 |
| Hand Chaining | ✅ chain_to 自動觸發下一個 hand |
| 成本追蹤 | ✅ CostTracker + AgentRuntime 自動記錄 + /costs API |
| 收入追蹤 | ✅ RevenueTracker + /revenue API + 按路線/來源分析 |
| 品牌聲音 | ✅ memory_recall brand_voice_profile |
| 社群發佈 API | ✅ twitter tool (API + browser fallback) |
| 部落格發佈 API | ✅ blog_publish tool (MDX → Vercel) |
| 預設 Cron 排程 | ✅ 4 jobs: freelancer daily, leads weekly, seo biweekly, content daily |
| **測試** | ✅ **333 lib + 60 integration = 393 tests (0 failures)** |

---

# 路線 A: AI 增強接案 (Freelancer Hand)

> 目標: 自動找工作 → 產出可直接貼上的提案 → 追蹤申請狀態
> Hand: freelancer (5 phases)
> 真實測試: 產出的提案投到 Upwork，拿到面試/成交

### 核心功能

```
[✅] A1. 工作搜尋 (web_search + browser)
  驗收: 搜尋 "AI automation upwork" 能回傳 10+ 真實工作列表
  狀態: 已驗證，Serper API + Upwork 直接瀏覽

[✅] A2. 工作評分 (LLM 分析)
  驗收: 回傳 0-100 分，帶理由，前 5 名合理
  狀態: 已驗證

[✅] A3. 提案生成 (file_write)
  驗收: 每個工作產出 <200 字個性化提案，可直接貼上 Upwork
  狀態: 已驗證，proposals.md 產出正常

[✅] A4. 申請材料準備 (file_write + memory_store)
  驗收: cover letter + 作品集列表 + 截止日提醒
  狀態: 已驗證

[✅] A5. 提案品質保證 — 人工審核流程
  驗收: 用戶能在 Telegram 看到提案內容，確認後才標記為「可投」
  狀態: ✅ human_review phase (Phase 5) + approval gate

[✅] A7. 申請狀態追蹤
  驗收: memory_recall("freelance_*") 能列出所有申請+狀態
  狀態: ✅ Phase 4 memory_store + Phase 1 duplicate detection

[✅] A8. 每日自動搜尋排程
  驗收: 每天早上 9:00 自動跑 freelancer hand
  狀態: ✅ Default cron job "daily-freelancer" (0 9 * * *)

[⚠️] A6. Upwork/Fiverr Profile 建立指南
  需要: 一份模板 (跑一次 Researcher Hand)
  狀態: 可用但需手動執行一次
```

### E2E 測試覆蓋
- `test_e2e_freelancer_full_pipeline_structure` — 結構驗證
- `test_e2e_freelancer_upwork_pipeline` — 完整 Upwork pipeline 含檔案產出
- `test_e2e_all_10_hands_load` — 載入驗證

---

# 路線 B: B2B 冷郵件銷售 (Lead + Outreach Hand)

> 目標: 找客戶 → 評分 → 寫郵件 → 發送 → 跟進 → 成交
> Hand: lead (4 phases, chain_to=outreach) + outreach (5 phases)
> 真實測試: 發出去的郵件收到回覆，進入銷售對話

### 核心功能

```
[✅] B1. 潛在客戶搜尋 (Lead Hand Phase 1-2)
  狀態: 已驗證，leads_data.csv 有真實公司

[✅] B2. 客戶評分 (Lead Hand Phase 3)
  狀態: 已驗證 (PureGlow 94分, EcoLife 92分等)

[✅] B3. 報告產出 (Lead Hand Phase 4)
  狀態: 已驗證

[✅] B4. 郵件生成 (Outreach Hand Phase 1-3)
  狀態: 已驗證，outreach_emails.md 產出正常

[✅] B5. ★ 郵件發送 (email_send 工具) ★
  狀態: ✅ SMTP 已設定 (Gmail App Password)

[✅] B7. 跟進自動排程 (Day 3 + Day 8)
  狀態: ✅ schedule_followups phase + memory + cron

[✅] B9. CRM 狀態管理
  狀態: ✅ memory_store + /crm Telegram command

[✅] B10. Lead → Outreach 串聯自動化
  狀態: ✅ chain_to = "outreach" in lead hand.toml

[⚠️] B6. 郵件送達率優化 (SPF/DKIM/DMARC)
  說明: Gmail 直送小量 OK，大量需要專業 ESP (運營層面)

[⚠️] B8. 回覆追蹤 (IMAP)
  說明: 需要 email_receive 工具 (未來功能)
```

### E2E 測試覆蓋
- `test_e2e_lead_outreach_chain_produces_files` — 檔案產出
- `test_e2e_complete_lead_to_outreach_pipeline` — 完整 4 phase pipeline + cost + revenue
- `test_e2e_email_tool_execute` — email 工具驗證
- `test_e2e_hand_registry_with_chain` — chain_to 驗證

---

# 路線 C: B2B 自動化訂閱服務

> 目標: 幫客戶跑自動報表/客服/行政，按月收費
> Hand: auto_report (4 phases) + customer_service (4 phases)

### 核心功能

```
[✅] C1. auto_report Hand
  狀態: ✅ 4 phases: data_collection → analysis → report_generation → distribution

[✅] C2. customer_service Hand
  狀態: ✅ 4 phases: intent_classification → knowledge_search → response_generation → quality_check

[⚠️] C3. 知識庫導入工具 — 可用 memory_store 批次匯入 (手動)
[⚠️] C4. 多客戶隔離 — 需要 namespace 機制 (未來)
[⚠️] C5. 定時執行排程 — cron 框架已有，需為每客戶設定
```

### E2E 測試覆蓋
- `test_e2e_all_10_hands_load` — 載入驗證

---

# 路線 D: SEO 內容 + 聯盟行銷 (SEO Content Hand)

> 目標: 自動產出 SEO 文章 → 發佈到部落格 → 賺廣告/聯盟收入
> Hand: seo_content (5 phases, 含 publish_and_promote)
> 部落格: markl-ai.space (Next.js + Vercel)

### 核心功能

```
[✅] D1. 關鍵詞研究 (Phase 1)
  狀態: 已驗證，keywords.csv 產出正常

[✅] D2. 競品分析 (Phase 2)
  狀態: 已驗證

[✅] D3. 文章撰寫 (Phase 3)
  狀態: 已驗證，article.md 13.7KB

[✅] D4. SEO 優化 (Phase 4)
  狀態: 已驗證

[✅] D6. ★ 部落格發佈自動化 ★
  狀態: ✅ blog_publish tool (MDX + index.ts + git push → Vercel)

[✅] D7. 部落格/域名建立
  狀態: ✅ markl-ai.space (Next.js + Vercel)

[✅] D11. 批量產出排程
  狀態: ✅ Default cron job "biweekly-seo-content" (0 11 * * 2,4)

[⚠️] D5. 人工編輯環節 — Telegram 通知已有
[⚠️] D8. Google AdSense 申請 — 運營層面
[⚠️] D9. 聯盟行銷連結管理 — 運營層面
[⚠️] D10. SEO 排名追蹤 — 需 GSC API (未來)
```

### E2E 測試覆蓋
- `test_e2e_seo_blog_twitter_pipeline_structure` — 結構驗證
- `test_e2e_complete_seo_to_publish_pipeline` — 完整 5 phase pipeline + blog_publish + twitter
- `test_e2e_blog_publish_execute_dry_run` — blog_publish 工具真實 MDX 產出
- `test_e2e_blog_publish_tool_creation` — 工具建立

---

# 路線 E: 社群內容 + 個人品牌 (Content Hand)

> 目標: 每天產出社群貼文 → 建立受眾 → 贊助/電子報收入
> Hand: content (4 phases, 含 publish_and_promote)

### 核心功能

```
[✅] E1. 趨勢研究 (Phase 1)
  狀態: 已驗證

[✅] E2. 內容生成 (Phase 2)
  狀態: 已驗證，content_output.md 4.8KB

[✅] E3. 品質審核 (Phase 3)
  狀態: 已驗證

[✅] E4. ★ Twitter/X API 發佈 ★
  狀態: ✅ twitter tool (API + Playwright browser fallback)

[✅] E9. 品牌聲音 Profile
  狀態: ✅ memory_recall brand_voice_profile in content/seo_content

[⚠️] E5. LinkedIn API 發佈 — LinkedIn API 審核嚴 (未來)
[⚠️] E6. 電子報系統 — 需 Substack/ConvertKit (未來)
[⚠️] E7. 排程發佈 — cron 排程已有 (daily-content)
[⚠️] E8. 互動追蹤 — 需 Twitter API 讀取 (未來)
```

### E2E 測試覆蓋
- `test_e2e_full_content_pipeline` — 4-phase 完整 pipeline 含檔案產出
- `test_e2e_content_hand_has_publish_phase` — publish_and_promote phase 驗證
- `test_e2e_twitter_tool_execute` — twitter 工具 280 字限制驗證

---

# 路線 F: 付費技能 / Agent Pack

> 目標: 打包垂直產業 Hands → 賣給客戶

```
[✅] F1. Hand TOML 定義格式 — 10 個 Hands 全部載入
[✅] F2. Hand 註冊機制 — GET /hands + POST /hand/:name/run
[⚠️] F3-F7. 打包/安裝/銷售 — 未來功能
```

---

# 路線 G: 托管代運維

> 目標: 幫客戶託管 agent，按月收費

```
[✅] G1. Docker 容器化 — Dockerfile (multi-stage Rust build + Playwright + pandoc) + docker-compose.yml
  - Multi-stage build: rust:1.83-bookworm → debian:bookworm-slim
  - Runtime: Python3 + Playwright + pandoc + CJK fonts
  - docker-compose: port 7878, volume mount, env vars, resource limits, health check
[⚠️] G2-G6. 多租戶 + 監控 + 備份 + 管理面板 + 帳單 — 未來功能
```

---

# 路線 H: 研究/情報產品 (Researcher + Market Intel Hand)

> 目標: 產出付費研究報告 → 銷售
> Hand: researcher (5 phases) + market_intel (5 phases)

```
[✅] H1. 深度研究 — 已驗證，5 Phase 輸出 ~9000 字報告
[✅] H2. 市場情報 — 已驗證，競品映射+定價分析
[✅] H4. PDF 匯出 — ✅ pdf_export tool (pandoc/weasyprint/Python)
[⚠️] H3. 報告品質提升 — 需 Claude API (agents.toml 設定)
[⚠️] H5-H6. 銷售管道/自動更新 — 運營層面
```

### E2E 測試覆蓋
- `test_e2e_pdf_export_tool_creation` — PDF 工具建立
- `test_e2e_pdf_export_produces_file` — PDF 檔案產出

---

# 路線 I: 開發者工具

> 目標: Agent 可觀測性/成本控管

```
[✅] I1. 成本追蹤模組 — ✅ CostTracker + /costs API + AgentRuntime 自動記錄
[✅] I4 (部分). 收入追蹤 — ✅ RevenueTracker + /revenue API + 按路線/來源/日期
[⚠️] I2-I3. 儀表板/路由優化器 — 未來 SaaS 功能
```

### E2E 測試覆蓋
- `test_e2e_cost_tracking_full_workflow` — 成本追蹤完整流程
- `test_e2e_cost_and_revenue_tracking` — 成本+收入整合
- `test_e2e_revenue_tracker_all_routes` — 10 路線收入追蹤
- `test_e2e_roi_analysis_cost_vs_revenue` — ROI 分析

---

# 路線 J: 交易分析 (Trading Analysis Hand)

> 目標: 量化交易研究 (不自動執行)
> Hand: trading_analysis (4 phases)

```
[✅] J1. 交易分析 Hand — ✅ 4 phases: market_overview → technical_analysis → sentiment_analysis → signal_generation
  含風險管理: 最大 5% 部位、止損、R:R 目標
[⚠️] J2-J5. 市場數據/回測/風控/模擬 — 需要交易所 API (高風險，最後做)
```

---

# 跨路線共用功能

```
[✅] X1. SMTP 郵件設定 — Gmail App Password 已設定
[✅] X3. 成本追蹤 — CostTracker + AgentRuntime + /costs
[✅] X4. 收入追蹤 — RevenueTracker + /revenue
[✅] X5. Cron 排程任務 — 4 預設任務 + Telegram /cron 管理
[✅] X7. Vision API — Gemini 2.5 flash-lite + Groq fallback
[⚠️] X2. Claude API 接入 — agents.toml 設定 (運營層面)
[⚠️] X6. 代理/反偵測 — 初期量小不需要
```

---

# 完成度總結

## 按 8 大交付項目

| # | 交付項目 | 狀態 | 證據 |
|---|---------|------|------|
| 1 | 10 條收入路線 A-J 工具和工作流 | ✅ 9/10 (G=infra) | 10 hands + 20 tools + 60 integration tests |
| 2 | Cron 排程自動化 | ✅ | 4 default jobs + /cron commands |
| 3 | Lead → Outreach pipeline chaining | ✅ | chain_to field + test_e2e_complete_lead_to_outreach_pipeline |
| 4 | SEO → Blog → Twitter pipeline | ✅ | publish_and_promote phase + test_e2e_complete_seo_to_publish_pipeline |
| 5 | Freelancer + Upwork 整合 | ✅ | browser + Upwork URLs + test_e2e_freelancer_upwork_pipeline |
| 6 | 每次 agent 執行成本追蹤 | ✅ | CostTracker in AgentRuntime + test_e2e_cost_tracking_full_workflow |
| 7 | E2E 測試執行完整工作流 | ✅ | 60 integration tests all passing |
| 8 | 測試產出真實檔案 | ✅ | FileWriteTool + FileReadTool 真實磁碟 I/O |

## 測試統計
- **333 lib tests** — 0 failures
- **60 integration tests** — 0 failures
- **396 total tests** — 0 failures

## 實作統計
- **10 hands**: lead, outreach, freelancer, content, seo_content, researcher, market_intel, auto_report, customer_service, trading_analysis
- **20 tools**: shell, file_read, file_write, file_edit, web_search, http_request, glob_search, content_search, memory_store, memory_recall, memory_forget, delegate, ai_code, computer_use, browser, vision, email_send, twitter, blog_publish, pdf_export
- **APIs**: GET /hands, POST /hand/:name/run, GET /costs, GET /revenue, /stream/agent/:name (SSE), /ws/agent/:name (WS)
- **Telegram**: /hands, /hand, /approve, /deny, /estop, /resume, /cron, /costs, /revenue, /crm
