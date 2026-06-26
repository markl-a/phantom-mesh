# phantom-mesh — 商業 / 開源設計

> **⚠️ 已被取代（SUPERSEDED）。** 本文件記錄的是較早的方向：Apache-2.0 核心 ＋ BSL broker ＋
> Tailscale 翻版。權威的商業策略現在是
> [`docs/COMMERCIALIZATION-STRATEGY.md`](COMMERCIALIZATION-STRATEGY.md)
> （繁中：[`COMMERCIALIZATION-STRATEGY.zh-TW.md`](COMMERCIALIZATION-STRATEGY.zh-TW.md)），
> 採 **AGPL 核心 ＋ FSL relay ＋ Nabu Casa 模式**。三處關鍵不同：
> (1) 核心授權 Apache → **AGPL**；(2) 收費伺服器 BSL broker → **FSL Phantom Relay**；
> (3) 整體形狀 Tailscale 翻版 → **Nabu Casa「免費核心＋一個零知識便利層」**。
> 本檔保留作歷史脈絡；如有衝突，以 COMMERCIALIZATION-STRATEGY.md 為準。
>
> ---
>
> 目標：讓 phantom-core 維持完全開源且可自架（self-hostable），
> 同時用一小組可選的商業服務來支付伺服器、我們叢集 broker（中介伺服器）
> 為免費層使用者燒掉的 LLM token，以及（最終）一兩位工程師的時間成本。
> 本文件是這兩半之間的架構契約。
>
> **非目標（Non-goal）**：純 SaaS 的「你必須登入才能用 phantom」——這條路在設計上
> 就被封死，凡是想把我們推往那個方向的創投投資條款（VC term sheet）我們都會婉拒。

---

## 1. 兩個世界，一份程式碼

直接受 **Tailscale**、**GitLab**、**Sentry**、**Plausible**、與 **Supabase** 啟發
——這些專案都有一個真正的開源核心，搭配一層不會把核心功能集鎖起來的商業雲端層。

```
┌───────────────────────────────────────────────────────────────┐
│  phantom-core (Apache 2.0, public GitHub)                     │
│   • CLI + TUI + serve + MCP + autoevolve + snapshot + MLX     │
│   • Tauri APK / IPA + Termux bootstrap + Win/Linux service    │
│   • All single-machine and per-tailnet cluster features       │
│   • BYOK for every provider; fully self-hostable              │
└───────────────────────────────────────────────────────────────┘
                          ▲
                          │  optional, opt-in
                          ▼
┌───────────────────────────────────────────────────────────────┐
│  phantom-cloud / phantom-team-broker (Commercial, source-     │
│   available BSL or MPL, NOT Apache; eventually GPL after      │
│   N years per Tailscale BSL practice)                         │
│   • Account & device discovery broker (`/login`)              │
│   • Cross-tailnet relay for users without their own VPN       │
│   • Hosted MLX / API gateway with shared rate-limit pool      │
│   • Audit log + SSO/LDAP for org admins                       │
│   • Per-org Pro / Team / Enterprise quotas                    │
└───────────────────────────────────────────────────────────────┘
```

核心永遠不會在自己的行程內（in-process）執行商業程式碼。商業層是
**一個 phantom-core 可以對話的服務**，透過一個薄客戶端
（`phantom-cloud-client` crate，藏在 `--cloud` cfg feature 之後）來溝通
——形狀與 Tailscale 的 `tailscaled` ↔ `coordinator`（協調者）拆分相同。

---

## 2. 硬性規則（一旦違反就會失去社群信任）

這些規則來自觀察每一個曾在此轉型上失手的專案（Hashicorp、MongoDB、Elastic、
Redis、Terraform），並理解社群認為什麼算是背叛、什麼算是可接受的演進。

1. **每一項既有的開源功能永遠維持 Apache 2.0。** 不移除功能、不對已發行的程式碼
   變更授權（license）。
2. **開源二進位檔（binary）自身完全可用。** 一位沒有任何雲端帳號、沒有網際網路、
   且使用 BYOK（自帶金鑰，Bring Your Own Key）的使用者，能做到今天 `phantom`
   二進位檔能做的一切——這是該二進位檔終其一生的契約。
3. **遙測（telemetry）預設關閉且永遠採選擇加入（opt-in）。** 開啟時，原始資料存放於
   `~/.phantom-mesh/telemetry.jsonl`。使用者得執行 `phantom telemetry upload` 才會送出；
   我們絕不自動回傳（auto-phone-home）。
4. **商業層可自架。** Team/Enterprise 的 broker 跑在客戶自己的基礎設施上
   （`docker run phantom/broker:latest`）。我們賣的是支援 + 託管便利——不是
   存取該二進位檔的權利。
5. **叢集規模不設人為閘門。** 開源使用者可以在自己的 tailnet 上跑 1000 個節點的叢集。
   雲端 broker 買到的是便利（`/login` 探索），而非能力。
6. **供應商金鑰永遠不離開使用者的機器。** 即使有雲端帳號，BYOK 仍是預設值。
   託管 gateway（閘道）是選擇加入。

若某項功能必須違反上述任何一條，我們就把它移進 `phantom-cloud` repo，
它永遠不會出現在 `phantom` 裡。

---

## 3. 層級對照表（Tier Map）

| 功能 | OSS | Pro | Team | Enterprise |
|---|---|---|---|---|
| 所有已發行的 phantom-core（CLI/TUI/serve/MCP/autoevolve/MLX/snapshot/全部 50 個工具） | ✅ | ✅ | ✅ | ✅ |
| `/login` mesh broker 探索 | — | ✅（3 台裝置） | ✅（依席次） | ✅ |
| 跨 tailnet relay（中繼，給沒有 WireGuard / Tailscale 的使用者） | — | 100 GB/月 | 1 TB/月 | 不計量 |
| 託管 LLM gateway（給免費層使用者共用的 API 金鑰） | — | 每月上限 $5 | 依席次計量 | 依席次計量 |
| 稽核日誌（audit log）保留期 | — | 7 天 | 90 天 | 無限 / 地端（on-prem） |
| SSO / SAML / LDAP | — | — | ✅ | ✅ |
| 全組織政策（哪個 agent 可做什麼） | — | — | ✅ | ✅ |
| 自架 broker | — | — | — | ✅ |
| 支援 SLA（服務水準協議） | 社群 | 盡力而為 | 1 個工作日 | 99.9% / 24×7 |
| 定價（目標） | $0 | $7/使用者/月 | $15/使用者/月 | 業務洽詢 |

這些層級買到的是**便利與維運**，而非 phantom-core 的能力。
一位有決心的開源使用者，靠著自己跑一個基於開源 `phantom-cloud-broker`
source-available 二進位檔的 broker，能重現 Pro 約 80% 的功能。

---

## 4. `/login` 的故事（mesh 探索）

當一位沒有 Tailscale tailnet 的使用者想「從手機看到我的 Mac」時，
今天他得自己把 Tailscale 設定起來。那是真實的摩擦。雲端 broker 解決它：

```
phantom login                 # device-flow OAuth, opens browser
phantom devices               # lists this user's mesh
phantom claim <hostname>      # adds this device to the user's mesh
```

底層運作：
- 一個無協調者（coordinator-less）變體的 `/rpc/peers`，跑在
  `broker.phantommesh.dev/api/peers/<account-id>`
- 每台裝置都跑 phantom-core，並可選擇性地呼叫
  `broker.phantommesh.dev` 並註冊它的 WireGuard 端點
  （或者，在沒有 WG 的情況下，一個經身分驗證的 long-poll 連線）
- 該 broker 本身就是 phantom-cloud-broker——Pro 使用者用我們的，
  Team / Enterprise 可以自架

沒用 `/login` 的開源版不受影響。使用者自己接好自家 tailnet，
在 agents.toml 裡設 `[cluster]` peers，跟今天完全一樣。

---

## 5. 遙測政策（唯一容易搞砸的東西）

```
phantom telemetry status            disabled (default forever)
phantom telemetry enable            opt-in, prints what it captures
phantom telemetry disable           one-shot off + clears local buffer
phantom telemetry upload            send buffered file to broker
phantom telemetry preview           print the next-to-be-sent payload
```

擷取了什麼（開啟時）：
- slash 命令、工具呼叫、agent 派發（dispatch）的次數計數
- 錯誤分類（沒有堆疊追蹤、沒有提示詞、沒有輸出）
- 平台 / phantom 版本 / 設定了哪些供應商
- 一個 `device_id`（隨機 uuid，會持久化，可重設）

絕不擷取什麼（這是規則，不是 feature flag）：
- 提示詞內容
- LLM 回應
- agent 碰過的檔案路徑
- API 金鑰 / token / cluster_secret / commit 訊息
- IP 位址（broker 日誌不記 ip）

若使用者在某次工作階段（session）開啟遙測，所有資料都存在
`~/.phantom-mesh/telemetry.jsonl`。在他們 `upload` 之前，沒有任何東西離開
這台機器。上傳後，broker 最多保存 30 天。

這是我們做出**最像 Tailscale 的一步**。如果搞錯了，社群會罵我們是抓耙子，
而我們也活該。如果做對了，我們就能贏得 Tailscale 所贏得的同樣善意。

---

## 6. 定價模型——為什麼 Pro 是 $7

我們從 Linear / Vercel / Sentry / Tailscale 為對應的
「個人取得一堆小型維運功能」如何定價，借來**每位開發者每月**這個數字：

| 層級 | 為何是這個價 |
|---|---|
| Free | 永遠免費。CLI 能用。你的 tailnet 能用。我們不補貼你。 |
| **Pro $7/月** | 涵蓋透過託管 gateway 的約 $1 LLM token + 約 $2 relay 頻寬 + 約 $3 維運 + 50% 毛利。對獨立開發者（indie）來說很舒適。 |
| **Team $15/席/月** | 涵蓋以上 + 稽核日誌 + SSO + 5 倍 relay 配額。可比 Tailscale Team。 |
| **Enterprise** | 一律業務洽詢。地端 broker、客製 SLA、可選工程工時。 |

一位已經有 Tailscale 又用 BYOK 的使用者，沒有任何理由付我們錢
——而這是對的。Pro 是給那些想要 `phantom login` 能「直接就動」的
使用者的便利層。

我們也對**經驗證的學生 / 非營利組織 / 開源維護者提供免費 Pro 層**
——跟 GitHub Copilot 一樣。這正好把我們想要的早期採用者導流進來，
而不必向他們收 $7。

---

## 7. 授權機制（License Mechanics）

- `phantom-core/` — Apache 2.0
- `phantom-cloud-client/`（只有在 `--features cloud` 時才會編進去）
  — Apache 2.0（如此一來廠商鎖定 vendor lock-in 不可能發生）
- `phantom-cloud-broker/` — **BSL 1.1，4 年後變更日期（change date）轉為
  Apache 2.0**（Tailscale 的同款做法）。Source-available，你可以稽核 + 在自己的
  基礎設施上跑，但 4 年內不得轉售為服務。
- `phantom-cloud-web/`（儀表板 dashboard）— 同樣 BSL。
- 商標 `phantom-mesh` — 由我們持有，AGPL 風格：可以跑去品牌化（unbranded）的
  分支；但不得使用我們的商標來做競爭性服務。

---

## 8. 實作路徑（依時間順序）

在 phantom-core 達到 1k stars + 約 50 個每週活躍自架者（從開源目前的二進位檔而來）
之前，我們**不會**建造這當中的任何東西。在那之前就建造商業層是一種過早優化
（premature optimization），它已殺死過許多開源-商業轉型。

當我們到了那一步：

| 階段 | 月份 | 範圍 |
|---|---|---|
| **0：開源 phantom-core** | 現在 | 把目前的程式碼以 Apache 2.0 開源，GitHub release + `brew tap markl-a/phantom-mesh` |
| **1：穩定開源版** | 第 1-3 月 | 只做修錯 + 文件 + 社群 PR。不寫商業程式碼。 |
| **2：雲端 broker MVP** | 第 4-6 月 | `phantom login`、peer 探索、免費層 3 台裝置。託管於 Fly.io / Render。一開始先單一定價（$7/月 Pro）。 |
| **3：託管 gateway** | 第 7-9 月 | 每租戶（per-tenant）LLM API 金鑰池（我們替 Pro 層使用者吃掉 token 成本）。成本上限保護我們。 |
| **4：Team 層** | 第 10-12 月 | 稽核日誌 + SSO。瞄準想要叢集但不要 Tailscale 的 5-50 人小型工程團隊。 |
| **5：Enterprise + 地端 broker** | 第 2 年 | 業務驅動；只在 Team 已有付費客戶（logos）之後才做。 |

營收計畫目標：**在 200 位付費 Pro 使用者時損益兩平**（約
$16,800 ARR（年度經常性收入）——足以涵蓋一位開發者的成本 + 基礎設施）。
超過這個數字的部分都拿來再投資。

---

## 9. 我刻意避開的事（與原因）

| 反模式（Anti-pattern） | 原因 |
|---|---|
| 沒有開源對應物的純雲端功能 | Hashicorp 的錯誤——社群離開了。 |
| 對已發行的 Apache 程式碼變更授權 | Elastic / Redis / MongoDB——社群分叉（OpenSearch、Valkey）。信任一旦失去就拿不回來。 |
| 為了「穩定性資料」而強制遙測 | npm audit / Yarn 1——再也沒被信任過。 |
| 「對開源免費」+ 對商業收費 | LegitGuru、JetBrains gradle 外掛等——授權若能執行就行得通；執行不了就一團亂。 |
| 為偵測盜版而做 AI 浮水印 / 回傳 | 太過激進，破壞信任，傷到合法企業客戶比傷到盜版者還多。 |
| 二進位檔內的升級推銷提示 | Fastlane 之類——惱怒 > 營收。 |

---

## 10. 目前已符合規範的架構選擇

phantom-core 中下列現行決策讓門保持敞開：

- **agents.toml 是唯一真相來源（source of truth）。** 雲端 broker 寫 / 讀
  這個檔案就跟使用者一樣——絕不另開一份設定。
- **`[cluster] peers` 是一份 URL 清單。** 這些 URL 是來自
  `tailscale ip` 的輸出，或是 `phantom devices --json`（雲端），對二進位檔的其餘部分
  來說是看不見的。
- **MCP 是整合平面（integration plane）。** Pro/Team/Enterprise 擴充功能都是
  `[[mcp_servers]]` 底下的 MCP 伺服器——與標準函式庫（stdlib）工具相同的
  插接面。
- **Costs.json 是本機的。** 遙測關閉時資料只存在磁碟上。開啟時，由使用者驅動的
  `phantom telemetry upload` 是它離開裝置的唯一路徑。
- **launchd / systemd / Scheduled Task 自動啟動開源二進位檔。**
  雲端功能透過獨立的 LaunchAgents 疊加於其上
  （`ai.phantommesh.cloud-relay.plist`）——可獨立解除安裝。

如果你在審查一個 PR，而它會強制走純雲端路徑，審查者範本應該問
「這在純開源模式下也能運作嗎？若不能，請移到 phantom-cloud。」

---

## 11. 待解問題（在第 2 階段前決定）

| 問題 | 傾向 |
|---|---|
| 單一二進位檔 `phantom` 配 `--features cloud`，還是兩個二進位檔 `phantom` + `phantom-cloud-client`？ | **單一二進位檔**——Pro/Team build 用 `cargo build --features cloud`。UX 較簡單。 |
| broker 託管在哪？ | Fly.io 區域部署 + Cloudflare R2 存日誌；若規模需要再重新評估。 |
| 把 iOS Tauri 殼開源還是維持商業？ | **開源**——它只是個薄 webview，沒有護城河（moat）。 |
| MLX 供應商能否託管（我們替使用者跑推論 inference）？ | **可以**，但要做成一個*明確區隔*的 SKU。免費層只支援 BYOK。 |
| 對開源採用 CLA（貢獻者授權協議，contributor licence agreement）嗎？ | **採用**但要極簡——DCO sign-off（無權利轉移），像 Linux kernel。別做會賦予我們重新授權權力的 CLA；社群討厭那種。 |

---

## 12. 北極星（North Star）

**phantom-core 是寫給進階使用者的情書；phantom-cloud 是一門
能支付自身存在的生意。** 兩者出貨的是同一個二進位檔；使用者自選想住在哪個世界。
我們絕不混淆兩者、絕不用其中一個去閘住另一個，也絕不讓季度營收目標
把我們推向預設開啟遙測。

Tailscale、Plausible 與 Supabase 已證明這個模型行得通。
phantom 的機會，是從他們的劇本學習，而不是發明一個新的錯誤。
