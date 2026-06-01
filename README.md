# Phantom Mesh

> **一個跑在你自己機器、用 Telegram 跟你對話、會記住「上次怎麼解決」的 AI agent（人工智慧代理）。**
>
> **An AI agent that runs on your hardware, answers your Telegram messages, and remembers what worked so next time it's faster.**

**今天就跑得起來的兩個 wedge（切入點，v0.5.0）：**
- **Telegram bot（電報機器人）** — 把 BotFather token（機器人權杖）給 daemon（常駐程式），從手機傳訊息給它，它用你機器上的工具(shell / file / web)回答你。SQLite 對話歷史。
- **Curator skill loop（策展技能迴圈）** — 跑成功的任務每晚被萃取成可重用的 skill,存進 FTS5。Agent 漸進累積屬於你的工具箱,不是套用別人的 prompt 庫。

「所有裝置 · 24/7 · 自我演進的工作隊」是 [Three Pillars（三大支柱）](#three-pillars) 講的 **roadmap（路線圖）**,不是現在的 README claim。

**Two wedges that actually work today (v0.5.0):**
- **Telegram bot** — point a BotFather token at the daemon, message it from your phone, it answers using tools on your machine (shell / file / web). SQLite-backed conversation history.
- **Curator skill loop** — successful runs are extracted nightly into reusable skills stored in FTS5. The agent gradually builds a personal toolkit from your usage patterns, not from someone else's prompt library.

The "all your devices · 24/7 · self-improving workforce" framing is the [Three Pillars](#three-pillars) **roadmap**, not the current README claim.

[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)
[![Version](https://img.shields.io/badge/version-0.6.0--rc.1-blue.svg)](core/Cargo.toml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-green.svg)](#license)
[![Platforms](https://img.shields.io/badge/platforms-Mac%20·%20Linux%20·%20Win%20·%20Android%20·%20iOS-success.svg)](docs/)
[![CI](https://github.com/markl-a/phantom-mesh/actions/workflows/ci-fast.yml/badge.svg)](https://github.com/markl-a/phantom-mesh/actions/workflows/ci-fast.yml)
[![Build 5 OS](https://github.com/markl-a/phantom-mesh/actions/workflows/build-5os.yml/badge.svg)](https://github.com/markl-a/phantom-mesh/actions/workflows/build-5os.yml)

> 🚧 **Alpha / 早期預覽 — v0.6.0-rc.1,積極開發中。**
> 核心日常流程已經可以用,但介面、指令與文件都還在變動,預期會有粗糙處。
> 把這個 repo 當成早期預覽:歡迎試用、回報問題。
>
> 🚧 **Alpha / early preview — v0.6.0-rc.1, under active development.**
> Core daily flows work today, but interfaces, commands, and docs are still in flux — expect rough edges.
> Treat this repo as an early preview: try it out and report issues.
>
> _(v0.6.0 brings the mobile + web control plane.)_

---

## 為什麼選它而不選其他方案

如果你正在 Phantom Mesh 與常見方案之間抉擇,以下是 4 行內的誠實說明:

| 如果你想要 | 請用 |
|---|---|
| 編輯器內的自動補全 | **Cursor / Continue.dev / Copilot** — IDE 整合是它們的本業 |
| 用來串接 prompt 的 Python 框架 | **LangChain / LlamaIndex** — 生態系更大 |
| 一個跑在你自己硬體、橫跨你所有裝置、從手機操控、邊用邊學、不強制依賴雲端的 agent | **Phantom Mesh — 繼續往下讀** |
| 零設定的雲端託管 AI | **直接用 Anthropic / OpenAI** |

我們的競爭力在於 **擁有權（ownership）+ 分散部署（distribution）+ 自我演進（self-improvement）**,而不是跟雲端 SaaS 拚單一功能的對等性。

---

## 30 秒故事

- **E006 30 秒 Life Hello（生活軌道初體驗）** — Life Track（生活軌道）端到端流程:擷取飲食 / 專注 / 習慣 → 教練回顧並給出明天的行動

```bash
# 1. Install (Linux/Mac; L1 unblocks the curl path in v0.6.0)
curl -fsSL https://phantommesh.io/install | sh

# 2. First-run wizard asks for ONE LLM key (any of 12+ providers)
phantom

# 3. Dispatch from this machine
phantom dispatch "summarize today's git log"

# 4. From your phone (after Tauri mobile app install) — same cluster, same skills
#    Open the app, point at the same broker, dispatch from your pocket.

# 5. Add a Pi / spare laptop later — it joins the mesh, picks up cap-routed tasks.
phantom serve --cluster-peer phantommesh://your.broker
```

v0.5.0 已具備:安裝、首次 dispatch（派工）、cluster（叢集）轉發、Hermes 自我演進迴圈、透過 Telegram/Slack 的 OpenClaw 遠端操控。
v0.6.0 即將登場:行動裝置 app 的 cluster 畫面、網頁 dashboard（儀表板）、skill bank（技能庫）視覺化、跨主機 smoke test（冒煙測試）。

---

## 四大支柱（Four Pillars，2026-05-19 鎖定）

> 4 支柱 / 2 軌道的框架為內部 roadmap（`BIG-GOAL.md`,未公開）
> 的 Version B（於 2026-05-19 因 Life Node 轉向而重新鎖定）。

### **P1 · 跨裝置 Mesh（網狀網路）**
- Peer-to-peer mesh（點對點網狀網路）,而非 server/client（伺服器/客戶端）架構。單一 Rust 程式碼庫橫跨 5 種 OS:Mac · Windows · Linux · iOS · Android
- Cluster-aware(具叢集感知):節點彼此發現、共享能力、把任務轉發給對的 worker（工作節點）
- Local-first(本地優先):即使 air-gap（實體斷網）也能用本地 LLM 取得完整功能
- Cap-routed(依能力路由):「這個節點有 GPU / 攝影機 / 長時待機 / 大容量磁碟」會直接在 dispatch 時浮現

### **P2 · 多模態理解**
- 影像 + 音訊 + 文字 + 行為脈絡 — 全部納入
- 多模態擷取管線(`/api/events` + `phantom event capture`)把生活事件(飲食照片、專注時段音訊、環境文字)送進與程式碼/終端機事件相同的 agent 迴圈
- Provider trait（供應商特徵介面）原生支援多模態 — 今天是 Gemini,日後依需求增加

### **P3 · 進化網（Evolve Mesh）**
- Hermes 六步閉迴圈:judge（評判）→ extract skill（萃取技能）→ store（儲存）→ recall（喚回）→ apply（套用）→ measure（量測）
- 每個叢集一份 skill bank（技能庫）— 每個節點都貢獻;每個節點都受益
- FTS5 記憶後端,可跨 session（工作階段）搜尋
- 12+ 個 LLM 供應商(Anthropic / OpenAI / Groq / Mistral / xAI / Together / Fireworks / Cohere / Perplexity / AI21 / NVIDIA NIM / Claude CLI),全部統一在一個 trait 後面

### **P4 · 加密為先**
- 每台裝置的金鑰透過 HKDF-SHA256(雜湊金鑰衍生函數)從 `~/.phantom-mesh/identity.key` 衍生而來
- 事件 / 影像 / 音訊 / 分析結果使用 age v1 於靜態時加密
- `phantom data delete --all --yes` 是硬性的 kill switch（一鍵清除開關）
- BYO model + BYO key(自帶模型、自帶金鑰)— 沒有單一供應商鎖定

## 兩條軌道（共用 P1-P4 骨幹）

軌道是 *你用 Phantom 做什麼*,而不是 Phantom 是什麼。

| Track（軌道） | 情境 | 發布版本 |
|---|---|---|
| **Life Track 陪你進步** | 減脂 · 專注 · 習慣 · 每日回顧 · 環境記憶 | **v0.6.0 主打** (`phantom event capture` + `phantom coach review`) |
| **Work Track 替你做事** | 程式碼 · 自動化 · 跨機派工 · agent swarm（代理群）· 演進 | 已於 v0.5.0 發布;v0.7.0+ 深化 |

## 三項營運原則（我們如何「打造」）

- **Shame-free（不羞辱）** — 教練語氣絕不帶批判。Prompt 模板會檢查是否有羞辱性洩漏;UI 文案審查把關此項。
- **Consent-gated capture（同意才擷取）** — 預設不背景錄製、不自動截圖、不記錄按鍵。擷取一律由使用者主動觸發。
- **Reversible（可逆）** — 一個指令刪除所有資料。一次設定切換更換 LLM 供應商。雲端同步(若提供)一律 opt-in（選擇加入）才生效。

完整框架與權衡取捨記錄於內部 roadmap（尚未公開）。

---

## 今天實際有的東西（`main` 分支, v0.6.0-rc.1）

精選亮點(完整的模組對支柱稽核為內部文件):

**P1 — 跨裝置 Mesh**
- Cluster RPC（叢集遠端程序呼叫）依能力轉發 (PR #160) + peer heartbeat（節點心跳） (PR #175)
- 5 平台建置(CI 矩陣依 `.github/workflows/`)
- `hardware.rs` 中的能力偵測 + 每節點 `worker_caps` 設定
- 4 平台聯邦 smoke 已於 2026-05-19 早上驗證通過

**P2 — 多模態理解** *(v0.6.0 新增 — Life Track 主打)*
- 多模態擷取管線(E002, PR #261, 15 個 commit):`phantom event capture` CLI + `POST /api/events` multipart（多部分上傳） + Gemini 供應商 + 事件儲存
- 6 個 SHARED P0 測試綠燈(trait 來回轉換 × 3 + Gemini 環境/速率限制 + CLI multipart)

**P3 — 進化網**
- Curator V2 ensemble judge（集成評判,`hermes/curator_ensemble.rs`)
- Skill extractor（技能萃取器,失敗與成功兩種極性都涵蓋)
- FTS5 記憶喚回進 agent 脈絡
- 30 個 Hermes 工具 + 12 個供應商 + retry middleware（重試中介層）

**P4 — 加密為先** *(v0.6.0 新增)*
- 加密儲存層(E004, PR #262 + #270 復原, 7 個 commit):從 `identity.key` 衍生的 HKDF-SHA256 EventKey + age v1 加解密 + `EventStore::with_key` + `phantom data delete --all --yes`
- 4 個 SHARED P0 測試綠燈(HKDF 決定性、age 來回轉換、明文→加密遷移、資料刪除後同層狀態存活)

**Channels（通道,Work Track 基底,v0.5.0 已發布)**
- ChannelInboundAuth trait (PR #170) 涵蓋 Telegram + Slack + WhatsApp 樁(stub)
- Persona system（人格系統,PR #152)— bot 語氣與通道無關
- 每通道速率限制 token bucket（權杖桶,PR #145)

**Foundation（基礎）**
- 4 項 CRITICAL（極高嚴重度）安全發現已關閉 (PR #111, #112, #116)
- 5 項 V8 Tauri HIGH（高嚴重度）發現中的 4 項已關閉 (PR #169, #173, #174)
- 6 項 RUSTSEC 傳遞性(transitive)安全公告中的 5 項已清除 (PR #166)
- 反幻覺(Anti-hallucination) V1 (T22)
- 供應商路由修正 (PR #266) — 8 個 V1 供應商(mistral/together/nvidia/fireworks/xai/ai21/perplexity/cohere)現在路由到原生 endpoint（端點）,而非 openrouter 後援

---

## 程式碼庫結構

```
core/                     # Rust agent runtime + cluster mesh + Hermes loop
├── src/
│   ├── agent.rs              # Main agent loop
│   ├── mesh.rs               # P1: cluster RPC + heartbeat
│   ├── serve.rs              # RPC server + peer registration
│   ├── hermes/               # P2: 6-step self-improvement loop
│   ├── openclaw/             # P3: channel adapters (TG / Slack / WhatsApp)
│   ├── providers/            # 12 LLM provider adapters
│   ├── hallucination/        # Anti-halluc V1 scanner
│   └── ...
├── tests/                # Integration tests
└── benches/              # Perf benches

app/                      # Tauri 2 cross-platform desktop + mobile app
├── src-tauri/                # Rust app host + commands
├── src/                      # React UI
└── ...

phantommesh-io/           # Cloudflare Worker — OAuth broker + (coming) web dashboard
mobile/android/           # Android packaging + worker_caps runbook
scripts/                  # Install + bootstrap + setup
.github/workflows/        # CI (per-platform, security audit nightly, etc.)

docs/
├── superpowers/
│   ├── BIG-GOAL.md           # 🔒 Locked product anchor (immutable)
│   ├── ROADMAP-v0.6.0.md     # Master sequencer (E001-E007)
│   ├── AUDIT-2026-05-17.md   # Module ↔ pillar mapping
│   ├── specs/
│   │   ├── _current/         # Active epic specs (E001-E007)
│   │   └── _archived/        # Pre-Big-Goal history (don't follow for direction)
│   ├── plans/                # Per-track implementation plans
│   ├── audits/               # Security audits V7-V12 + CI workflows
│   ├── runbooks/             # L1 CF creds (more coming with E001/E004/E006/E007)
│   └── security/             # Live mitigation rationale (e.g. RSA Marvin)
└── DEPLOY-PHANTOMMESH-IO.md  # Cloudflare Worker deploy SOP
```

---

## 從原始碼建置

```bash
git clone https://github.com/markl-a/phantom-mesh.git
cd phantom-mesh
cd core && cargo build --release    # ~5-10 min cold; ~30s warm
./target/release/phantom --help
```

桌面 app:
```bash
cd app && pnpm install && pnpm tauri dev
```

Android(依 `mobile/android/README.md`):
```bash
cd app && pnpm tauri android dev
```

Feature flags（功能旗標,多數實驗性工作都受其閘控）:

| Flag（旗標） | 啟用內容 |
|---|---|
| `experimental-hermes-curator` | Curator + judge |
| `experimental-hermes-memory` | FTS5 memory backend |
| `experimental-hermes-providers` | Mistral / xAI / Together / Fireworks / Cohere / Perplexity / AI21 / NVIDIA NIM |
| `experimental-openclaw` | Telegram / Slack / WhatsApp channels |
| `experimental-cluster-heartbeat` | C4 heartbeat state transitions (PR #175) |
| `experimental-anti-hallucination` | T22 V1 scanner |

---

## 路線圖

亮點:

- **v0.6.0** — 7 個 epic（史詩任務）涵蓋行動裝置 + 網頁控制平面、跨主機 smoke、skill bank UI、OpenClaw 重新定位、30 秒上手流程
- **v0.6.x** — operator（操作者）後續事項(WhatsApp 的 Meta 驗證、Mac 硬體回收利用、GH 帳單)
- **v0.7.0** — Discord+、語音通道、IDE 端輔助工具、多叢集聯邦(週期後規劃)

---

## 生態系 — 7 個衛星項目

phantom-mesh 是底層 infrastructure（基礎建設);6 個衛星 + 1 對外網站 build on top:

| # | Repo | 角色 |
|---|---|---|
| ① | **phantom-mesh** (this) | 核心 mesh + Hermes + FTS5 + 12 provider trait |
| ② | [phantom-training](https://github.com/markl-a/phantom-training) | Agentic post-training orchestrator |
| ③ | [phantom-ai-feed](https://github.com/markl-a/phantom-ai-feed) | Daily AI news + RAG + interview questions |
| ④ | [phantom-secure-connector](https://github.com/markl-a/phantom-secure-connector) | PHI redactor + anomaly + compliance + MCP bridge |
| ⑤ | [phantom-enterprise](https://github.com/markl-a/phantom-enterprise) | LDAP/SSO/MES/ERP/VPN connectors |
| ⑥ | [phantom-flow](https://github.com/markl-a/phantom-flow) | Event-driven workflow engine (n8n / Zapier 替代) |
| ⑦ | [phantom-companion](https://github.com/markl-a/phantom-companion) | Personal behavior analytics + LLM insight |

衛星不重做底層,共用 phantom-mesh 的 FTS5 / provider trait / Hermes loop / cluster RPC / encryption。
詳見 [`docs/ECOSYSTEM.md`](docs/ECOSYSTEM.md)。

---

## 貢獻

本專案目前由維護者 + Claude 輔助的批次作業推進。
歡迎外部貢獻;門檻是 **「要服務某一支柱」**。開 PR 前請先讀 [`CONTRIBUTING.md`](CONTRIBUTING.md)。

安全相關發現:見 [`SECURITY.md`](SECURITY.md)(若存在)或開立私有 issue。

---

## 授權

採用以下兩者其一授權

- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE) 或 <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([`LICENSE-MIT`](LICENSE-MIT) 或 <http://opensource.org/licenses/MIT>)

由你自行選擇。

### 貢獻條款

除非你明確聲明其他條款,否則任何你有意提交以納入本作品的貢獻
(依 Apache-2.0 授權的定義),都應如上述以雙重授權方式授權,
不附加任何額外條款或條件。
