# phantommesh.io broker（中介伺服器）— Hono + Cloudflare Workers + D1

`spectyn login` 的網頁端。實作來自
[../docs/design/SPECTYNMESH-IO-DESIGN.md](../docs/design/SPECTYNMESH-IO-DESIGN.md) 的線路契約（wire contract，通訊協定規格）。

```
src/
  index.ts                  Hono 路由器（8 條路由）
  types.ts                  Env 綁定型別
  routes/
    health.ts               GET /api/health
    oauth.ts                /auth/cli/start + google + apple callbacks
    email.ts                POST /auth/email/{login,register}
    api.ts                  已驗證的 /api/me /api/devices
    pages.ts                伺服器端渲染 / /login /account
  lib/
    oauth.ts                PKCE + Google/Apple OAuth + broker JWT
    db.ts                   D1 包裝層（users / devices / tokens）
migrations/
  0001_init.sql             D1 schema
wrangler.toml               Cloudflare Worker 設定
```

---

## 一次性設定

```bash
# 1. 工具鏈
npm install -g wrangler
cd spectynmesh-io
npm install                 # 安裝 hono + jose + types

# 2. Cloudflare 帳號
wrangler login

# 3. D1 資料庫
wrangler d1 create spectynmesh-prod
# → 把 database_id 貼進 wrangler.toml 的 [[d1_databases]] 區段
wrangler d1 execute spectynmesh-prod --file=./migrations/0001_init.sql

# 4. 給 OAuth 工作階段用的 KV namespace（鍵值命名空間）
wrangler kv:namespace create SESSIONS
# → 把 id 貼進 wrangler.toml 的 [[kv_namespaces]] 區段

# 5. 機密（secrets）
wrangler secret put GOOGLE_CLIENT_SECRET    # 來自 Google Cloud Console
wrangler secret put APPLE_KEY_ID            # 來自 Apple Developer / Keys
wrangler secret put APPLE_P8_PRIVATE_KEY    # 貼上 .p8 內容（含 -----BEGIN PRIVATE KEY-----）
wrangler secret put BROKER_JWT_SECRET       # 任意 32+ 隨機位元組（base64 亦可）

# 6. 部署
wrangler deploy

# 7. 綁定到 phantommesh.io
#    - Cloudflare DNS：phantommesh.io 的 A/AAAA 紀錄 → Cloudflare
#    - 在 wrangler.toml 取消註解 [routes] 區段，然後再次執行 `wrangler deploy`
```

---

## OAuth 提供者設定

### Google Cloud Console

- 建立一個 OAuth 2.0 Client ID，類型選 **Web application**
- 授權重新導向 URI（redirect URI）：`https://phantommesh.io/auth/google/callback`
- CLIENT_ID 可沿用既有的 `869770808980-…`（已在
  `wrangler.toml [vars]` 中）；CLIENT_SECRET 則是你透過
  `wrangler secret put` 設定的新機密。

### Apple Developer Portal

- **Service ID**：`ai.spectynmesh.auth`（已在
  `wrangler.toml [vars]` 中 — 於 Tauri 整合期間建立）
- **Domain + Return URL**：domain = `phantommesh.io`，return URL =
  `https://phantommesh.io/auth/apple/callback`
- **Key**：建立一把新的「Sign in with Apple」金鑰，下載 `.p8`
- **Team ID**、**Key ID**、**.p8 內容** → 填入 `wrangler secret put`

---

## 本機開發

```bash
wrangler dev
# → http://localhost:8787

# CLI 冒煙測試（把 spectyn 指向開發用 broker）
SPECTYN_AUTH_URL=http://localhost:8787 spectyn login
```

即使是本機開發，你也需要真實的 Google/Apple OAuth 機密，
或者改測不碰任何 IdP（身分提供者）的 email 流程：

```bash
SPECTYN_AUTH_URL=http://localhost:8787 spectyn login   # menu → email
```

---

## 線路契約（wire contract）— 必須與 CLI 的 `login_broker` 一致

spectyn CLI（`core/src/bin/spectyn.rs`）使用此協定；任何
偏差都會破壞外界每一份已安裝的 spectyn。

| 端點 | 使用者 | 契約 |
|---|---|---|
| `GET /api/health` | CLI 的 3 秒探測 | 200 → 上線；非 200／逾時 → CLI 退回本機 provider 選單 |
| `GET /auth/cli/start?device_id=&port=&redirect=` | CLI 在此重新導向瀏覽器 | 必須驗證 UUID + port + http-loopback 重新導向；建立 KV 工作階段；重新導向至 `/login?state=…` |
| `GET /auth/google/start?state=` | 登入頁按鈕 | 標準 PKCE → accounts.google.com |
| `GET /auth/google/callback` | Google IdP 重新導向 | 以 code 換取 → 呼叫 `redirectToLoopback` |
| `POST /auth/apple/callback` | Apple IdP form_post | 相同結構；每次呼叫重新產生 client_secret JWT |
| Loopback `?p=base64(json)` | 將身分交給 CLI | Payload 必須包含 `provider`、`email`、`sub?`、`name?`、`picture?`、`id_token`、`access_token`、`broker_token`、`broker_token_expires_at_ms` |
| `GET /api/me` | 已驗證的 CLI 呼叫 | Bearer broker_token → 使用者個人資料 |
| `GET /api/devices` | 已驗證的 CLI 呼叫 | Bearer broker_token → 使用者的裝置清單 |

---

## 信任與隱私保證

broker 絕對不可（這是規則，不是功能旗標 — 見
[../docs/design/COMMERCIAL-DESIGN.md](../docs/design/COMMERCIAL-DESIGN.md) §2）：

- 以明文（plaintext）儲存 provider 密碼（我們採用 PBKDF2 SHA-256 100K）
- 接收任何 LLM provider API key（這些一律留在 `~/.spectyn-mesh/agents.toml`）
- 接收提示詞（prompts）／代理輸出／檔案路徑／提交訊息
- 在 loopback 重新導向後保留 Google/Apple 的 `id_token`／`access_token`
  （它們會轉發給 CLI，並在 v1 於伺服器端丟棄；
  未來：可選的每帳號「remember tokens」並另行
  徵得同意）
- 將 broker_token 的 TTL（存活時間）設定超過 7 天
- 在遙測日誌（telemetry-log）中記錄任何指名特定 repo／檔案／提示詞的內容

---

## 成本預估（Cloudflare 免費方案）

| 級別 | 免費額度 |
|---|---|
| Workers 請求 | 100,000/day |
| KV 讀取 | 100,000/day |
| KV 寫入 | 1,000/day（登入嘗試；這是綁定瓶頸） |
| D1 讀取 | 5M/day |
| D1 儲存空間 | 總計 5 GB |

一次登入嘗試 = 約 3 次 KV 寫入（開始工作階段、過程中更新、
回呼時刪除）。1k 次登入嘗試／天綽綽有餘；10k／天則會
推進到每月 5 美元的付費方案。真實世界的穩態使用量主要由
`/api/me` 呼叫主導（當我們加入工作階段快取後，每次呼叫 1 次 KV
讀取），其餘裕高出 100 倍。

---

## 發展藍圖（Roadmap）

- ✅ MVP（最小可行產品）：`/api/health`、`/auth/cli/start`、Google + Apple OAuth、email
  級別、broker JWT、D1 migrations、伺服器端渲染的登入頁。
- ⏳ 位於 `/account` 的帳號儀表板（消費 `/api/me` + `/api/devices`）
- ⏳ 網頁端 mesh（網狀網路）探索 — ping 某裝置的 Tailscale IP 以確認其
  可連通；並呈現於儀表板
- ⏳ Tauri/iOS/Android 應用內瀏覽器流程（目前已可透過既有的
  loopback 重新導向運作；僅需補文件 + 截圖）
- ⏳ 計費 — Pro / Team / Enterprise，依
  [docs/design/COMMERCIAL-DESIGN.md](../docs/design/COMMERCIAL-DESIGN.md) §3
