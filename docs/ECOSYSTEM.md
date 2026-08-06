# Spectyn Mesh Ecosystem — 7 個衛星項目

> **底層 = spectyn-mesh,衛星 = 6 個專業領域延伸 + 1 個對外網站**
> 全部對齊 [BIG-GOAL.md](superpowers/BIG-GOAL.md) 的 4 pillar / 2 track

```
                    ┌────────────────────────────────────────┐
                    │  ① spectyn-mesh (this repo)             │
                    │  Rust + 5 OS client + skill evolve      │
                    │  P1 跨裝置 / P2 多模態 / P3 evolve       │
                    │  P4 加密 / 12 provider trait / FTS5      │
                    └────────────────────────────────────────┘
                                       ▲
                                       │
        ┌──────┬──────┬────────┬───────┴───────┬───────┬──────┐
        │      │      │        │               │       │      │
       ②      ③      ④       ⑤              ⑥      ⑦
   training feed   secure  enterprise        flow   companion
```

## 各衛星項目

### ② [spectyn-training](https://github.com/markl-a/spectyn-training)

**Agentic post-training orchestrator on spectyn-mesh.**

跑在 spectyn-mesh 上的 AI agent 自動 fine-tune / 蒸餾 / benchmark 自己的小模型。
站在 Unsloth + Axolotl 巨人肩膀,加 spectyn 的 multi-agent + cross-device 優勢。

- **基礎**:Unsloth + Axolotl + DSPy(prompt-side) + LaMDAgent(pipeline 編排)
- **獨特**:第一個 self-hosted + cross-device + agentic post-training framework
- **目標客戶**:NVIDIA / Anthropic / Modal / Together / 工研院 / 中研院

### ③ [spectyn-ai-feed](https://github.com/markl-a/spectyn-ai-feed)

**Daily AI news + knowledge base + interview question generator,改自 [My-AI-Learning-Notes](https://github.com/markl-a/My-AI-Learning-Notes)。**

每天自動爬 arxiv / HN / r/LocalLLaMA,LLM 摘要進 spectyn FTS5,每週六出面試題。

- **基礎**:技能庫策展器(Curator) + FTS5 + 12-provider trait
- **獨特**:第一個中文 AI 工程師導向 + 面試題自動生成 + on-prem RAG
- **目標客戶**:Substack 訂閱者 + 中文 LLM 學習者(Hahow 課程)

### ④ [spectyn-secure-connector](https://github.com/markl-a/spectyn-secure-connector)

**PHI redactor + time-series anomaly + compliance check + red/blue team simulation + MCP bridge.**

spectyn-mesh 的安全資料 + 工具暴露套件,讓敏感資料安全進來、可信工具安全出去。

- **包含**:
  - PHI redactor(從 Medical RAG 抽出 sanitized)
  - anomaly detector(從 AHI Detection v2 通用化)
  - compliance checker(SOC2 / HIPAA / GDPR / 個資法)
  - secops simulator(從 [spectyn-secops](https://github.com/markl-a/spectyn-secops) 整合)
  - MCP bridge(spectyn → Claude Desktop / Cursor)
- **目標客戶**:趨勢科技 / 中信 / 國泰 / 富邦 / 醫材公司 / 工研院生醫 / 鼎新

### ⑤ [spectyn-enterprise](https://github.com/markl-a/spectyn-enterprise)

**LDAP / SSO / VPN / MES / ERP / Confluence 企業 on-prem connector pack。**

spectyn-mesh 在企業內網無痛跑 — 接公司 SSO、跨企業 VPN routing、串接 ERP/MES。

- **連接器**:LDAP / SAML 2.0 / OIDC / Tailscale-aware / 鼎新 T100 / 鴻海 MES / GitLab on-prem
- **目標客戶**:鼎新 / 中信 / 國泰 / 鴻海 / 聯發科 / 中型 SI

### ⑥ [spectyn-flow](https://github.com/markl-a/spectyn-flow)

**Event-driven 跨服務工作流引擎,merged from [Automation_with_Agent](https://github.com/markl-a/Automation_with_Agent) + [Data-Analysis-with-Agents](https://github.com/markl-a/Data-Analysis-with-Agents)。**

spectyn-mesh 上的 n8n / Zapier —從 spectyn 的 trigger / RAG / skill 組成 pipeline,自動對外執行。

- **基礎**:17+ 自動化工具 + RAG + agent framework + 聚類/RFM 算法(reused)
- **獨特**:第一個 self-hosted + cluster-aware + AI-native + cross-device workflow engine
- **目標客戶**:鴻海 C3 AI Service / 中型 AI SaaS / Modal / Together

### ⑦ [spectyn-companion](https://github.com/markl-a/spectyn-companion)

**Personal behavior analytics + LLM insight + proactive optimization(Life Track 主推)。**

不只是 daily coach — spectyn-companion 觀察你怎麼用 spectyn + sensor 資料,
找出 pattern,主動建議優化。對應 BIG-GOAL Life Track wedge(v0.6.0 lead)。

- **觀察**:LLM 使用 / commit 產出 / context switch / 健康指標 / 學習 ROI / 投履歷 follow-up
- **獨特**:第一個 self-hosted + cross-device + LLM-powered 行為分析優化
- **目標客戶**:Garmin / Anthropic / Micron AIoT / 醫療 AI / 自我量化社群

## 對外網站(衛星,不算 7 個之一)

- **[spectynmesh.com](https://spectynmesh.com)** — Cloudflare Worker(OAuth broker + 主站)
- **[markl-ai.space](https://markl-ai.space)** — maintainer 個人網站

## 共用底層架構

每個衛星都用到 spectyn-mesh 的:

| 共用 | 提供者 |
|---|---|
| LLM router | provider trait(12 個 LLM) |
| RAG / Memory | FTS5 backend |
| Skill bank | 技能庫六步閉環 |
| Cross-device dispatch | cluster RPC + capability-aware forwarding |
| Encryption | HKDF-SHA256 + age v1 + per-device key |
| Channel adapters | Telegram / Slack / WhatsApp / future SSE web |
| Multimodal capture | E002 pipeline(image / audio / text) |

→ **衛星不重做底層,直接 build on top**。

## 雙版本架構

每個項目都有兩個 repo:

| 版本 | location | 內容 |
|---|---|---|
| **public** | `github.com/markl-a/<name>` | sanitized(透過 [sync mechanism](#sync-mechanism))|
| **private** | self-host on node-a (Gitea + Tailscale-only) | 真實 secrets + WIP code + 個人 integration |

### Sync mechanism

從 private → public 走 **三層防護**:

1. **手動觸發** `spectyn sync release <repo>` — sanitize + push
2. **週日 09:00 電鐘 dry-run** — 自動發現遺漏 / 差異
3. **Tag-based release** — `git tag v1.x` 觸發 GitHub Actions sanitize check + push

任一層擋下 = 不會誤推 secrets。

## 開發順序(M1-M4,16 週)

```
M1 ──── W1 spectyn-mesh polish + markl-ai.space 升級
        W2-4 ⑥ spectyn-flow(已有最完整改裝資產)

M2 ──── W5-7 ④ spectyn-secure-connector(招聘覆蓋最廣)
        W8 ③ spectyn-ai-feed MVP

M3 ──── W9-10 ⑦ spectyn-companion(需 spectyn 跑 2 月後有資料)
        W11-12 ② spectyn-training

M4 ──── W13-14 ⑤ spectyn-enterprise
        W15-16 7 篇英文 blog + 對外發布 + 衝 star
```

## 為什麼 spectyn-mesh 是必要核心

- 跨衛星共用 FTS5 → 跨領域 insight(spectyn-companion 用 spectyn-ai-feed 學的 + spectyn-flow 觸發紀錄 + spectyn-secure-connector 健康)
- 跨衛星共用 provider trait → 全域 LLM 成本 view
- 跨衛星共用 evolve loop → 用 spectyn 一段時間後,所有衛星都「更懂你」
- **如果衛星各自重做底層,我們會變成另一個 LangChain ecosystem**(分散、無 cross-vertical insight)。spectyn-mesh 是這個生態系的 keystone

## 衛星項目的優先級依據

依「招聘 + 副業 + 人生痛點」三條件交集評分:

| # | 招聘 | 副業 | 人生 | 總分 |
|---|---|---|---|---|
| ① spectyn-mesh | 5 | 4 | 5 | **14** ⭐ |
| ② spectyn-training | 5 | 4 | 3 | 12 |
| ③ spectyn-ai-feed | 3 | 4 | 5 | 12 |
| ④ spectyn-secure-connector | 5 | 4 | 5 | **14** ⭐ |
| ⑤ spectyn-enterprise | 5 | 3 | 2 | 10 |
| ⑥ spectyn-flow | 4 | 5 | 5 | **14** ⭐ |
| ⑦ spectyn-companion | 4 | 3 | 5 | 12 |

**三個 14 分項目**:spectyn-mesh、spectyn-secure-connector、spectyn-flow → 主力 ship 序。
