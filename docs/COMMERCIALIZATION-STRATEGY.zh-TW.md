# Spectyn Mesh — 商業化策略

> **狀態：** 決策版建議。取代較早的 [`docs/design/COMMERCIAL-DESIGN.md`](design/COMMERCIAL-DESIGN.md)
> （該文件假設 Apache 核心 / BSL broker / Tailscale 翻版的形狀）。本文件自 v0.6.0
> 週期起為權威的商業骨幹。
>
> **英文正本：** [`COMMERCIALIZATION-STRATEGY.md`](COMMERCIALIZATION-STRATEGY.md)
> （內含完整參考來源編號與引用）。本檔依照 [`superpowers/BIG-GOAL.md`](superpowers/BIG-GOAL.md)
> 的雙語慣例，作為其繁體中文companion。
>
> **本策略必須遵守的約束：** 大部分開源；單人維護者＋AI agent 艦隊；無 VC；攸關維護者生計；
> 且——不可違反——[大目標](superpowers/BIG-GOAL.md)：*跑在你自己的裝置上、local-first、
> 資料加密只有你能讀。*

---

## 0. 本文件為何存在，以及它如何對齊大目標

[大目標](superpowers/BIG-GOAL.md) 的四項承諾，是底下每一個商業決策的承重牆：

- **P1 — 跨裝置 mesh、local-first。** 產品必須能在你自己的硬體上、用本地模型、air-gapped
  地完整運作。→ *核心絕不能放在付費牆或登入之後。*
- **P2 — 多模態理解。** → *大目標本身的反目標：核心多模態絕不鎖在付費層之後。*
- **P3 — 進化網（「越用越懂你」）。** → *閘門讓 Spectyn 學習的能力＝自殺；skill bank 與
  evolve loop 維持免費。*
- **P4 — 加密為先（「資料加密，只你能讀」）。** → *任何付費服務只能搬運它讀不到的加密位元組；
  對加密本身收費會摧毀整個產品立足的信任敘事。*

推薦模式是**唯一**能同時符合這四點的。它只收一樣東西——一個**零知識託管中繼（relay）**——
這是 P4 加密承諾的嚴格*延伸*（relay 傳的是它無法解密的密文），而非違反。Mesh 在無 relay
時仍 air-gapped 全功能，滿足 P1 的 local-first 反目標。沒有任何核心能力、行動 App、多模態或
evolve 功能要付費。對齊是精準且刻意的。

---

## 1. 推薦模式——「Nabu Casa 模式」：免費開源核心永遠免費 ＋ 一個付費雲端便利層

**唯一主模式：** 整個 mesh——引擎、agents、evolve loop、加密、所有平台 client *包含行動
App*——**永遠免費開源**；唯一收費品是訂閱制的**託管便利服務「Spectyn Relay」**：為 NAT
後面的手機/筆電提供零知識中繼/會合點、推播通知橋、以及 E2E 加密的異地備份。

### 為何選這個模式而非其他（推理，全部來自研究證據）

1. **這是唯一被證明「單人/極小團隊、零 VC」可活的模式。** Home Assistant 的商業臂 Nabu Casa
   用單一 ~$6.50/月 SKU（遠端中繼＋雲端語音＋異地備份）在 200 萬安裝基數上養活整個基金會與
   數十名員工，**從未拿外部投資** [1][2]。Coolify 以單人創辦人身分做到 ~$15.7k/月毛、~$12.9k/月
   淨，收費 $5/月託管控制平面＋贊助 [3][4]。Ollama 在免費本地核心之上變現*託管推理便利*——
   近乎 bootstrap，無大型 VC [5][6]。
2. **其他模式全都需要 VC 或銷售團隊。** n8n 的 fair-code＋企業銷售（$180M C 輪）[7]、Supabase
   託管資料庫（募資 >$1B，因為穩定地跑 Postgres 真的很難）[8][9]、Tailscale 閉源協調器＋席次制
   B2B（$160M C 輪）[10]。單人都做不起。
3. **付費層正是「自架者*做得到*但不想 24/7 維運的東西」。** 中繼、會合、推播、異地備份正是這類。
   且 relay 只傳加密位元組（zero-knowledge），**不破壞大目標「資料只你能讀」**——反而*延伸*了
   加密承諾。Mesh 在無 relay 時仍 air-gapped 全功能，符合 local-first 反目標。
4. **消費端「私有 AI 軟體」本體已被商品化到免費**（Ollama/LM Studio/Jan/Open WebUI 全免費）；
   prosumer 願花 $1k–$15k 買硬體，但對「只是編排本地模型的訂閱軟體」容忍度近零 [11]。所以絕不能
   對核心能力收費，只能對*便利*收費。
5. **個人錢包對「免費之上的便利」天花板約 $5–20/月**（Coolify $5、Nabu Casa $6.50、Ollama Pro
   $20）。Relay 類定價屬於這個帶的低端。

### 兩個備援槓桿（fallback levers，不是並行主線）

- **槓桿 A — Immich 式自願終身支持者授權。** 零功能閘門、純支持性質的一次性授權
  （$29 個人 / $99 每 cluster）。前置現金流、零社群風險，可在 relay 上線前就賣 [12]。
- **槓桿 B — 受監管 SMB 的付費支援/商業授權（v0.8.0+ 才啟動）。** 法律/醫療/金融後台已證明
  願為「on-prem 就是賣點」付 $100–500/人/年（Lemony $499/月/節點、Tabby $19–24/人/月）[13][14]。
  只在有 inbound 訊號時才投入——這需要銷售動作。

---

## 2. 開源 / 付費分界線

**設計原則：** 研究中所有社群反叛（Plex 行動端先收費、Emby 閘門 app 被 Jellyfin 吃掉、
Open WebUI 事後改授權）都源自**對「使用者認為理所當然的東西」收費或事後收緊** [15][16]。因此：

| 元件 | 開源（永遠免費） | 付費 | 理由 |
|---|---|---|---|
| 核心引擎 / mesh P2P / CLI | ✅ AGPL | — | 產品本體；大目標已承諾不付費牆 |
| 四角色 squad、evolve loop、skill bank | ✅ AGPL | — | 「越用越懂你」是大目標，閘門它＝自殺 |
| 加密（at-rest、per-device key） | ✅ AGPL | — | 加密功能收費摧毀信任敘事 |
| **行動 App（Android/iOS client）** | ✅ 免費 | — | Plex/Emby 教訓：對 mobile client 收費招來 fork |
| Skills 市集 / SDK | ✅ MIT/Apache | 永遠免費、不抽成 | HACS/VS Code/Raycast 全免費市集＝採用護城河 [17] |
| **Spectyn Relay**（託管會合＋推播橋＋E2E 異地備份） | 協定開放；伺服器碼 FSL | 💰 ~$6/月 | 唯一收費品；自架 relay 永遠可行（Tailscale 對 Headscale 的善意 [18]） |
| 支持者終身授權 | — | 💰 自願、零閘門 | Immich 模式 |
| 企業 SSO/RBAC/稽核（未來） | — | 💰 商業授權 | 「buyer-based open core」：經理買的東西才收費 [19] |

**要寫進公開文件的承諾：** 單一使用者、自己裝置上的完整體驗——含多模態、evolve、加密、
*以及行動 App*——**永遠 100% 免費**；自架 relay 的協定**永遠公開**。免費版對標 Tailscale
Personal（6 人/無限裝置免費 [20]）。

---

## 3. License 建議

| 元件 | 授權 | 理由 |
|---|---|---|
| 核心（engine/mesh/CLI/agents） | **AGPLv3** | 防雲端 strip-mining：Google 等 hyperscaler 對 AGPL 有全面內部禁令，根本不會做「Spectyn Mesh Cloud」[21]；對自架終端使用者零義務，local-first 產品的 AGPL 成本/效益異常划算。Grafana/Bitwarden/Nextcloud 已驗證 AGPL＋商業雙授權。 |
| Skills SDK / 協定 lib / client 嵌入層 | **Apache-2.0 或 MIT** | 讓寫 skill、做整合的人不被傳染，市集才長得起來。 |
| Relay 伺服器 | **FSL-1.1-Apache-2.0** | 2 年後自動轉 Apache（DOSP）＝可信承諾，又防「Spectyn Relay 競品 SaaS」[22][23]；避開 BSL（HashiCorp 後信譽有毒）。 |
| 貢獻機制 | **CLA（用 cla-assistant），並在 CONTRIBUTING.md 誠實揭露用途** | 保留雙授權選項：iOS App Store 與 AGPL 衝突，唯一著作權人可自授商店例外；事後 DCO→CLA 才是信任事件，day one 就 CLA 並講明白則不是 [24]。 |
| 商標 | **註冊「Spectyn Mesh」字標（USPTO ~$350/類）＋公開商標政策**（fork 須改名、允許指名性使用） | 單人維護者買得起的最高槓桿保護；OpenTofu 被迫改名就是商標的力量 [25][26]。 |

**最重要的一條：** *day one 就用保護性授權。* 2024–2026 所有社群反叛（HashiCorp/Redis/Elastic/
Open WebUI）全是「先寬鬆、後收緊」造成的；從第一天就 AGPL 的專案沒有發生過等量反叛 [27][28]。
v0.6.0 公開前就把 `LICENSE` 定案，之後永不改。

> **⚠️ 目前 repo 狀態：** 本 repo 現在出貨 dual `LICENSE-APACHE` + `LICENSE-MIT`。採用 AGPL
> 核心因此是一個必須在公開鏡像形成社群之前完成的*上線前重新授權*決策。這正是本節主張的
> 「在社群存在前鎖定授權」動作——現在做，不要事後做。

---

## 4. 定價草案（含對標）

| 方案 | 價格 | 內容 | 對標 |
|---|---|---|---|
| **Free** | $0 | 全功能：無限自有裝置、全 OS node、squad、evolve、加密、skills 市集 | Tailscale Personal（免費 6 人/無限裝置）、Ollama、Jellyfin |
| **Supporter（終身，自願）** | **$29 個人 / $99 每 cluster** | 零功能差異；徽章＋CHANGELOG 鳴謝＋搶先測試 | Immich $24.99/人、$99.99/server；Obsidian Catalyst $25 |
| **Spectyn Relay** | **$6/月 或 $60/年** | 零知識中繼（NAT 穿透）、推播橋、E2E 異地加密備份、「支持開發」框架 | Nabu Casa $6.50/$65；Coolify $5；Obsidian Sync $4–10 帶 |
| **Team / Compliance（v0.8.0+，僅有訊號才做）** | ~$15/人/月（席次制，**絕不按裝置計價**） | SSO、稽核日誌、優先支援、商業授權 | Tabby $19–24/人/月；Msty Teams $300/人/年；Tailscale Standard $8/人/月 |

**計價單位教訓：按「人」不按「裝置」。** Tailscale 2026 v4 收掉裝置型 Personal Plus
（「造成摩擦卻無實質差異」），ZeroTier 砍免費裝置數招致反彈 [29][30]。

**誠實的收入數學：** 業界免費→付費轉換中位數 ~2.6%，3–5% 算好 [31]；純捐贈 <1%。要到 $3k/月
（單人可活）需 ~500 個 relay 訂戶 ≈ 以 3% 轉換推算 ~17,000 個活躍 cluster。**這是 12–24 個月
的路，不是 90 天的路**——所以 Supporter 終身授權（槓桿 A）要同步開賣來前置現金。

---

## 5. 操作者方向——免費開放權重的自家蒸餾模型作為預設大腦

> 由維護者在研究之上加上，並與大目標的 BYOM（自帶模型）反綁定立場、以及 P3「12+ provider
> 同一 trait」一致。

**方向：** 出貨一個免費、開放權重、自家蒸餾的模型作為 Spectyn 的*預設大腦*——讓 air-gapped、
無 API key 的體驗開箱即好用。然後**變現託管推理與個人化蒸餾，絕不變現權重本身。**

如何與策略其餘部分契合：

- **權重維持免費開放。** 送出一個好的預設模型，跟 Ollama/Jan 同一招——它商品化*runner*、讓免費
  層極佳，這就是採用護城河。也保持 BYOM 承諾完整：自家模型只是預設，絕非綁定；12+ provider
  仍可逐請求切換。
- **付費的是便利與個人化，不是能力：**
  - *託管推理*——給沒有、或不想 24/7 跑 GPU 機的使用者。與 relay 同一個「自架者做得到但不想
    維運」邏輯，乾淨地併入（或並列）Spectyn Relay SKU。GPU 時間計價，非 token 計價——Ollama
    Cloud 模式 [5]。
  - *個人化蒸餾*——Spectyn 整個論點就是「越用越懂你」（P3）。一個從使用者自己（加密、同意過的）
    skill bank 與使用紀錄蒸餾出*個人化*小模型的付費服務，是疊在 evolve loop 上的便利，產出的
    模型屬於使用者。其訓練管線由 [spectyn-training](ECOSYSTEM.md) 衛星實作。
- **這*不是*：** 不是賣基礎權重，也不是閘門本地 evolve loop。你自己在自己硬體上跑的蒸餾維持
  免費。只有當你要*我們*替你跑 GPU 與管線時才付費。

這讓模式保持一致：**免費開源核心（現在包含一個免費預設大腦）、一個付費零知識便利層（relay
＋選用的託管/個人化推理）、以及自願支持者授權。** 使用者*有權享有*的東西絕不移到付費牆後。

---

## 6. GTM——90 天順序

**Phase 0 — v0.6.0 出貨前（現在→上線，~4 天）：法務地基比功能重要。** 在公開 repo 定案
`LICENSE`（AGPL 核心）、`TRADEMARK.md`、`SUSTAINABILITY.md`（公開分界線承諾＋誠實揭露 CLA 用途）
與 CLA bot。**這些必須在社群形成前就位**——事後補＝反叛觸發器。

**Phase 1 — 上市（前 ~4 週）：** 殺手級素材是已驗證的四平台聯邦 demo（Windows + Linux +
Android + Mac mesh）。錄 2 分鐘影片：*「拍一張午餐照，手機上的 coach 回應，而推理跑在你家的
桌機上，全程加密」*。發 Show HN、r/selfhosted、r/LocalLLaMA、r/homelab、Lobsters（錯開 2–3 天）。
這群人就是「買 $1k–$10k 硬體、要求軟體免費」的 homelab 早期採用者——完美契合免費核心策略。同時
上線 GitHub Sponsors ＋ merchant-of-record（Polar：5%＋50¢ MoR，可賣授權金鑰 [32]）賣 Supporter
授權，並架 Relay waitlist 落地頁。

**Phase 2 — v0.7.0（~Q3）：** 加密擴到 `agents.toml`/conversations/`memory.db`（補完 P4 敘事）
＋ Relay MVP（會合伺服器跑 $5 VPS，先服務 waitlist）。Relay 解的痛點正是大目標「從手機/瀏覽器
指揮」在 NAT 後的現實阻礙——產品與商業在同一條路上。

**規模化決策指標（90 天時看）：Relay waitlist 報名數。** ≥200 → relay GA 全力投入；50–200 →
慢速 beta；<50 → 主力轉槓桿 A+B，relay 降為自用功能。輔助指標：GitHub stars ≥2,000、Supporter
收入累計 ≥$500。

### 90 天行動清單（依序；🤖＝agent 艦隊可自動化）

**第 1 週（出貨前）：**
1. 🤖 核心 repo 加 AGPLv3 `LICENSE`；SDK 子 crate 改 Apache-2.0。
2. 🤖 寫 `SUSTAINABILITY.md`（分界線承諾＋CLA 揭露）＋ `TRADEMARK.md` 草稿（agent 寫，維護者終審）。
3. 🤖 架 cla-assistant bot。
4. 🤖 依 public-leak 協定掃公開 repo。
5. 🤖 README 重寫：tagline *「Your AI mesh. Your data. Your devices.」* ＋安裝一行指令＋demo GIF。
6. v0.6.0 出貨。

**第 2–4 週（上市）：**
7. 錄四平台聯邦 demo 影片（維護者錄，agents 寫腳本/剪輯清單）。
8. 🤖 Show HN 文案草稿；維護者親自發文並守 48 小時回留言（HN 回覆不可外包語氣）。
9. 🤖 r/selfhosted、r/LocalLLaMA、r/homelab 貼文草稿，錯開發布。
10. 開 GitHub Sponsors ＋ Polar，上架 Supporter $29/$99。
11. 🤖 Relay waitlist 落地頁（靜態頁＋表單）。
12. 🤖 docs 快速上手站（mdBook，GitHub Pages）。

**第 5–12 週（複利）：**
13. 🤖 每週 devlog（agent 初稿、維護者定稿）發 blog＋Reddit。
14. 🤖 issue 分流 <24h 回應（agent 艦隊輪值，符合 no-idle loop）。
15. 🤖 種 10 個示範 skills 進 skill bank（市集冷啟動）。
16. USPTO 商標申請——找 OSS 律師一次性諮詢（~$1–2k，**唯一需要現金的項目**）；同場確認 CLA
    條文＋App Store 例外條款。
17. 🤖 Relay MVP：會合伺服器（複用 mesh 碼）部署到 $5 VPS，FSL 授權。
18. 🤖 競品對比頁（vs Ollama/Jan/AnythingLLM：「他們是單機 runner，我們是跨裝置 mesh」）。
19. 🤖 自動化 release pipeline（5 平台 binary，降 bus-factor）。
20. 第 90 天：對照本節指標開 go/no-go 決策。

---

## 7. 風險表

| # | 風險 | 緩解 |
|---|---|---|
| 1 | **Strip-mining：** 有人推出「Spectyn Mesh Cloud」搶走變現層 | AGPL 核心（hyperscaler 政策禁用）＋註冊商標（競品不能用你的名字）＋relay 伺服器 FSL（2 年內禁競爭性使用）。三層疊加正是研究結論的聯合威懾 [21][22]。 |
| 2 | **單人 bus-factor：** 你倒下，專案與收入歸零，用戶因此不敢付費 | 短期：🤖release/triage 全自動化＋公開營運手冊。中期：從社群提拔 1–2 位 co-maintainer。寫死承諾：relay 碼有 FSL DOSP（公司死了碼自動變 Apache）——比口頭承諾可信 [23]。*誠實註記：這是本策略最無法完全消除的風險。* |
| 3 | **OSS 社群反叛**（被解讀為 crippled-core 或怕你 rug-pull） | 唯一可靠解法是「永不事後收緊」：day one AGPL、公開分界線承諾、CLA 用途誠實揭露、Supporter 授權零閘門、行動 App 永遠免費。所有反叛案例都是事後改授權，沒有 day-one 保護性授權被反叛的前例 [28]。 |
| 4 | **轉換率太低，養不活生計**（最存在性的風險） | 誠實面對：2–5% 是現實帶、捐贈 <1%；用槓桿 A（終身授權）前置現金、保留其他收入直到 MRR 穩定；90 天指標不達標就立刻轉槓桿 B（SMB 支援，$100–500/人/年）而非加碼燒 relay。隱私付費意願「嘴上說」遠高於「實際掏」（privacy paradox）——只信 waitlist 和刷卡數。 |
| 5 | **AGPL 摩擦：** 企業法務禁用導致潛在貢獻者/嵌入者卻步；iOS App Store 不收 AGPL | SDK/協定層用 Apache/MIT（整合不被傳染）；CLA 保留雙授權——唯一著作權人可自授 App Store 例外與商業授權，真有企業需求時這反而變成槓桿 B 的收費入口。App Store 例外條款需律師過目（判例稀薄）[33]。 |

---

## 8. 誠實的不確定性聲明

本文件是策略備忘錄，不是法律或財務意見。證據有真實的侷限：

- **錨定案例是推論，非公開揭露。** Nabu Casa 訂戶數與營收非公開；~5% 轉換 / ~10 萬訂戶是從
  「50+ FTE 主要靠訂閱養」＋「200 萬戶」推論而來。Coolify 數字是創辦人在 X 自報。
- **轉換率基準是借來的。** 2.6% 中位 / 3–5%「好」帶來自 B2B SaaS freemium 一般數據；**沒有**
  自架開源產品的公開轉換數據。本文的收入數學可能偏樂觀或偏悲觀一倍。
- **私人公司營收為估算。** Tailscale/n8n/Supabase 營收來自估算聚合站（getlatka、Sacra），未經審計。
- **此市場定價多變。** DGX Spark 上線數週內漲 ~$700；Tailscale 2026-04「pricing v4」收掉了
  mid-2025 來源仍引用的方案；Bitwarden 經典 $10/年 Premium 在 ~2026-01 結束。每個報價都當作
  mid-2026 快照。
- **受監管產業機會是缺口主張，非既有收入池。** 今天醫生/律師用的多數付費「私有 AI」其實是
  合規包裝的*雲端* SaaS，非本地推理；那些領域真正本地的付費市場目前是小額工具。槓桿 B 是押在
  缺口上的賭注，不是接既有收入的水龍頭。
- **隱私付費意願是軟證據。** 最強的隱私溢價調查數字來自對結論有利益的來源；硬的揭示性偏好證據
  是硬體售罄與 on-prem 合約，不是調查百分比。
- **法律機制需要真正的審查。** 商標、CLA 條文、App Store/AGPL 例外（判例稀薄）、FSL 條款，都需
  在任何公開承諾前找 OSS 律師諮詢一次（行動清單 #16）。
- **研究美國中心。** WebSearch 僅限美國；歐盟數位主權動態（偏好 AGPL）與台灣/亞太付費行為被
  低估——不過早期社群（HN/r/selfhosted）本來就是全球英語圈。

---

## 一句話總結

**用 day-one AGPL＋商標把「跑在你所有裝置上、加密只有你能讀」釘死為永遠免費，然後只賣一樣
東西——~$6/月的零知識 Spectyn Relay（NAT 穿透＋推播＋加密異地備份，可選搭配在免費自家蒸餾模型
上的託管/個人化推理）——這是 Nabu Casa 已證明能零融資養活整個團隊的路，輔以 $29/$99 終身支持者
授權前置現金，90 天後用 relay waitlist 人數決定加碼或轉向。**

---

## 參考來源

完整來源編號與引用見英文正本 [`COMMERCIALIZATION-STRATEGY.md`](COMMERCIALIZATION-STRATEGY.md)
的 References 一節（≈130 個引用 URL）。歸因於這些來源的數字，承載 §8 的各項保留。

---

## 內部備註（公開鏡像前移除）/ Internal notes (strip before public mirror)

<!-- INTERNAL: 此行以下在本檔進入公開鏡像（markl-a/spectyn-mesh）前必須移除或淨化。 -->

- **與其他 track 的排序：** Phase-0 法務地基是 v0.6.0 公開推送前的 4 天窗口，獨立於功能 track，
  可在 agent 艦隊間平行扇出；只有商標申請（#16）卡在維護者＋外部律師。
- **具體公開 repo 目標：** §3 的重新授權與 §6 的 README 重寫落在公開鏡像；推送前先做 public-leak
  淨化（E-epic/F-feature ID、內部節點名、fleet IP、內部絕對路徑）。本 repo root 的 dual
  `LICENSE-APACHE` + `LICENSE-MIT` 就是需被 AGPL 核心決策取代的檔案。
- **衛星串接：** §5 的個人化蒸餾 SKU 由 `spectyn-training` 衛星實作（見 `docs/ECOSYSTEM.md`）；
  relay 會合伺服器複用既有 mesh RPC 碼；OAuth/login broker 是 `docs/commercial/CONTRIBUTOR-FUNNEL.md` 已述
  的 `phantommesh.io` Cloudflare Worker。
- **決策成品：** 本文件是決策版內部策略備忘錄的潤飾形；底層研究成品（≈130 個來源含各記錄
  findings/caveats）保留在操作者的 plans 目錄，不在 repo。
```
