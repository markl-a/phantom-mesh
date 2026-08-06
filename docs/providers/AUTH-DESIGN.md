# Spectyn Auth / Identity / Onboarding — 設計定案

> 2026-06-02。鎖定設計(經多輪探討 + Phase 1 跨裝置驗證後定案)。改動前先讀這份。

## 核心:三個獨立 plane,各用最小機制

### ① 連通 plane — 裝置彼此連得到
- **Tailscale / Headscale(WireGuard)**。現用 Tailscale SaaS;要無 vendor → 換 **Headscale**(drop-in,沿用官方 TS client)。
- 裝置加入靠 Tailscale 自己的 auth / pre-auth key。spectyn 不插手。

### ② mesh-信任 plane — spectyn 信任哪些節點/client ✅ 已實作+驗證
- **Tailscale-native 信任**(Phase 1,commit `ca2ccc78`+`e8e658db`):local-UI 端點(/api/chat 等)信任「whois 驗證過的 online tailnet peer」(`mesh::online_tailnet_peer_ips`,absolute-path 找 tailscale,fail-closed)。
- `cluster_secret` 退為「非-tailnet 節點」fallback。撤銷單台 = Tailscale 後台移除裝置。
- **無靜態 secret 當主、無每台 spectyn OAuth**。已用 iPhone 跨裝置驗證免 401。
- opt-in:`[cluster] trust_tailnet_peers = true`。
- ⚠️ **精確說法(別誤會成「零登入」)**:裝置加入 tailnet **仍需 Tailscale 一次性認證**(SSO 或 pre-auth key)。spectyn 的 win 不是「不用登入」,而是 **沿用 Tailscale 那層你本來就要做的認證、不另加 spectyn 自己的帳號/OAuth**——**一次認證而非兩次**;**每台入網時一次,而非每 session**。**安全 mesh 必有某種 auth**(SSO / 自發 key / 憑證 / 預共享鑰),差別只在「放哪」與「多頻繁」。要連第三方帳號都不要 → Headscale(自架控制面、自己的 user + pre-auth key)或 Nebula(憑證式 PKI,無登入)。

### ③ 身分 plane — 你是誰(只給雲端/多人功能)— 選用,非必要
- mesh+chat **不需要**它(已證明)。只在以下才需要:雲同步到非-mesh 裝置 / markl-ai.space 公開用戶 / Pro 訂閱 / 跨人分享。
- 要做時:**優先 passkey/WebAuthn**(無第三方、抗釣魚、local-first)> Google/Apple OAuth。
- 現況:Google 走 broker(`spectyn login broker`)可用;`spectyn login google` 直連壞(client_secret missing);**Apple BLOCKED**(broker /api/health 無 apple,#4 portal+deploy 未做)。

## Onboarding = mesh-first
新裝置 = ① 加 tailnet(Tailscale up / auth key)② 跑 `spectyn serve`(或 thin client 連 coordinator)。**不每台點瀏覽器 OAuth**。OAuth 只在 opt-in 雲端功能時。

## 開源版信任根策略(tiered)+ tag-gate 預設
OSS 版把信任根做成分層,**「自帶 tailnet」為預設**(最方便),同時保留無-vendor 與無-mesh 退路:
| 層 | 給誰 | 信任根 | 門檻 |
|---|---|---|---|
| **預設(最方便)** | 一般 self-host | **Tailscale SaaS**(自帶帳號,免費 tier)| 裝 Tailscale + 跑 spectyn |
| **自主(無 SaaS)** | OSS 純粹派 | **Headscale**(同 client、自架控制面)| 自架控制面 |
| **fallback(無 mesh 工具)** | 離線/手動 | **cluster_secret** 預共享 | 手動填 secret |

文件對外框成「**bring your own tailnet(Tailscale 或 Headscale)**」——方便(預設 Tailscale)又不被罵綁 SaaS(有 Headscale 路)。
為何 OSS 靠 Tailscale 划算:重用其 NAT 穿透 / 裝置認證 / whois → spectyn **零信任系統維護**;目標用戶(homelab/self-host)多已有 Tailscale;定位「**跑在你現有 tailnet 上**」(Zeabur 等同模式)。

### 🔒 tag-gate 預設開(OSS 安全關鍵)
Phase 1 目前信任「**所有** online tailnet peer」——對**單人 tailnet** OK,但 OSS 後別人可能在**多人/公司 tailnet** 跑 → 太寬(會信任不該信的同 tailnet 裝置)。
- **OSS 預設應 gate by `tag:spectyn`**(只信任打了 spectyn tag 的 peer)。
- 機制:`trust_tailnet_peers` 之外加 `trust_tailnet_tag`(預設 `"tag:spectyn"`);單人方便模式才放寬成「任何 peer」。
- → **Phase 1 follow-up**:把 tag 過濾從「目前未實作」變成「OSS 預設要求」。`online_tailnet_peer_ips` 需從 tailscale status 多抽每 peer 的 Tags,gate 時比對。

## 決策
- **OAuth/Apple Sign In(#4)→ 降級**:沒有對應雲端/多人功能前不投入 portal+deploy。
- **mesh / onboarding → 鎖定**(Phase 1 已核心)。roadmap:Phase 2(scoped 能力 token biscuit/JWT、agent 驅動 onboarding、auto-enroll)+ Headscale 無 vendor 選項。Phase 3(per-request 零信任、mDNS LAN、passkey)。

## 自建網路層?(build-vs-buy 決策,2026-06-02)
問題:spectyn 要不要內建類似 Tailscale 的功能?**決策:不重做核心,只做一層有界的 LAN 內建 mesh。**

**不自建**(協調面 + 中繼 relay + NAT 穿透):
- 離題——spectyn 的價值在 agent mesh 編排,不在網路傳輸。
- NAT 穿透/DERP/hole-punching 是真難,自幹版又爛又不安全。
- 對「被頂尖公司看見」是**紅旗**:自滾 VPN/crypto = 判斷力差。騎 WireGuard/Tailscale、只加獨有的 agent 信任層 = 好工程判斷,才是想要的訊號。
- 已有更好解:Headscale 給「無 SaaS、自主」開源故事,零額外程式碼。

**值得內建(便宜、有界、強化「開箱即用個人 mesh」賣點)= LAN 內建 mesh**:
- mDNS 區網發現(serve 已在做:`_spectyn-mesh._tcp` 註冊)。
- 選配:同網段用 **boringtun**(Cloudflare Rust WireGuard userspace)直連 → 免 relay、免 NAT 穿透。
- 故事:「回到家所有裝置自動成網,零設定、零 Tailscale」,範圍可控。
- 跨網段/NAT 後 → 繼續 Tailscale/Headscale(BYO tailnet),當賣點講不是道歉。

**🔒 鐵則**:傳輸/crypto 用經審計的函式庫(WireGuard/boringtun);**永遠不自滾協調面或 crypto**。novel 力氣全投 agent 層。

**時機**:Phase 2/3 roadmap,**非 6/7 月**。LAN mesh 最好當發射後、用戶反饋驅動的功能,別排擠 eval/發射/影響力。

## Naming canonical = `ai.spectynmesh.*`
理由:出貨 app(iOS/Android/桌面)bundle id 已是 `ai.spectynmesh.app`,改不得 → 其餘配合它。
- Apple Services ID → **`ai.spectynmesh.signin`**(取代文件的 `io.` / 早期決定的 `com.`)
- Apple App ID(Sign in with Apple)→ `ai.spectynmesh.app`
- 衛星 launchd `com.markl-a.spectyn-*` → 可選 `ai.spectynmesh.<name>`(cosmetic,低優先)
- 修 `docs/install/APPLE-SIGN-IN-SETUP.md` 的 `io.spectynmesh.signin` → `ai.spectynmesh.signin`
- 已一致:Tauri/iOS/Android = `ai.spectynmesh.app`;launchd serve/nosleep/autoevolve = `ai.spectynmesh.*`;顯示名 `Spectyn Mesh`;網域 `spectynmesh.com`。
