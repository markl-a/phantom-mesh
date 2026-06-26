# phantommesh.io — Broker（中介伺服器）前端 + 後端設計

> `phantom login` 的網頁端。本文件是實作契約（implementation
> contract）：它明確告訴你 phantommesh.io 必須提供哪些 URL，以及
> 回應必須是什麼形狀，因為 phantom CLI（命令列工具）已經會講這套
> 協定（見 `core/src/bin/phantom.rs` 的 `login_broker`）。
>
> 一旦 phantommesh.io 在下列 URL 上就位，**`phantom login` 就能直接
> 運作（Just Works）** — CLI 端不需要再改任何程式碼。

---

## 0. 兩個部件

```
   phantom CLI (your laptop)        phantommesh.io                  OAuth providers
                                    ┌──────────────────┐             ┌──────────┐
                                    │  Frontend (web)  │             │  Google  │
   phantom login   ─────────────►   │   /              │  ─────►    │  Apple   │
                                    │   /login         │             │  email   │
                                    │   /auth/cli/start│             └──────────┘
                                    │                  │
                                    ├──────────────────┤
                                    │  Backend (API)   │
                                    │   /api/health    │
                                    │   /auth/*/start  │
                                    │   /auth/*/cb     │             ┌──────────┐
                                    │   /api/devices   │  ─────►    │ Database │
                                    │   /api/me        │             │ (D1/PG)  │
                                    └──────────────────┘             └──────────┘

   ◄───────  loopback :48181/oauth/callback  ─────────
```

phantommesh.io 是**受信任的中間人（trusted middleman）**。它代表使用者
向 Google / Apple 講 OAuth（開放授權），然後透過 localhost loopback
（本機回送）把產生的身分交給 phantom CLI。CLI 永遠看不到使用者的
Google 密碼，永遠看不到 Apple 的 id_token 簽章金鑰，只會看到一份
說「這就是你是誰」的 JSON payload（資料負載）。

---

## 1. 傳輸格式（Wire Format，CLI 已實作；請凍結此格式）

### 1.1 健康探測（Health probe）

```
GET https://phantommesh.io/api/health

  200 OK   { "status": "ok", "version": "1.0.0", "providers": ["google","apple","email"] }
  503      anything else → CLI falls back to local provider menu
```

使用者一執行 `phantom login`（不帶參數）的當下，phantom CLI 就會以
3 秒逾時（timeout）探測此端點。若逾時或回 4xx/5xx，CLI 會顯示本機
備援選單（fallback menu）（email / google direct / apple stub）。

### 1.2 啟動登入流程

```
GET https://phantommesh.io/auth/cli/start
  ?device_id=8de1a55b-7412-...           (CLI's stable uuid)
  &port=48181                             (CLI loopback listener port)
  &redirect=http%3A%2F%2F127.0.0.1%3A48181%2Foauth%2Fcallback
```

伺服器行為：

1. 驗證 `device_id` 是一個 uuid、`port` 落在 `[1024, 65535]`、`redirect`
   符合 `^http://(127\.0\.0\.1|localhost):\d+/oauth/callback$`。
2. 設定一個伺服器端的 session cookie（工作階段 cookie），攜帶
   `(device_id, redirect)`。
3. `302 → /login?cli=1`（或直接 render 一個登入頁）。

### 1.3 供應商 OAuth 交握（server-side，伺服器端）

使用者在 `/login` 上挑選供應商（provider）後，broker 就像任何網頁
應用程式一樣處理這套 OAuth 交握（dance）：

| 供應商 | 流程 |
|---|---|
| Google | 透過 `accounts.google.com` 的標準 PKCE OAuth。把你的 Google OAuth Client（Web App 類型）設定成以 `https://phantommesh.io/auth/google/callback` 作為 redirect URI（重新導向網址）。 |
| Apple | 「Sign in with Apple」網頁流程。把你的 Apple Service ID（`ai.phantommesh.auth` — 已存在於 `core/src/oauth.rs`）設定成相同的 redirect domain（重新導向網域）。使用 `.p8` 私鑰 + Team ID + Key ID 來產生 client secret JWT。 |
| email | 對 broker 的使用者資料庫做伺服器端 bcrypt 驗證。沒有外部 IdP（身分供應商）。 |

IdP 重新導向回來後，broker 會：
1. 用 code 換取 access_token + id_token（Google），或驗證 Apple
   id_token 的簽章。
2. 在 `users` 資料表中以 `(provider, sub)` 或 `(email)` 為鍵查找 /
   建立一列資料。
3. 在 `devices` 資料表中記錄裝置認領（device claim）：
   `(device_id, owner_user_id, claimed_at, label=hostname)`。
4. 建構 **payload**（見 1.4）。

### 1.4 把身分交給 CLI（the load-bearing step，承重的關鍵步驟）

IdP 交握完成後，broker 把 **瀏覽器** 重新導向到 CLI 的 loopback URL，
並將身分 payload 以 base64 編碼的 JSON 放在查詢字串（query string）中：

```
302 Location: http://127.0.0.1:48181/oauth/callback?p=<base64url(json)>
```

JSON payload 內容：

```json
{
  "provider":     "google",
  "email":        "alice@example.com",
  "sub":          "1234567890",
  "name":         "Alice Cooper",
  "picture":      "https://lh3.googleusercontent.com/...",
  "id_token":     "<google id_token JWT>",
  "access_token": "<google access_token>",
  "broker_token": "<short-lived JWT issued by us, used for /api/* calls>",
  "broker_token_expires_at_ms": 1730000000000
}
```

CLI 的 `login_broker` 已經會從這個確切的形狀中擷取 `provider / email /
sub / name / picture / id_token / access_token`（見
core/src/bin/phantom.rs）。新增欄位是可累加的（additive） — CLI 會
忽略它們。

> **為何用 query-string-base64 而非 POST**：CORS（跨來源資源共享）在
> CLI loopback 端很麻煩，redirect+query 少一個會動的零件。token 只會
> 在本機使用者的網址列上曝光幾毫秒。若你想更嚴格，請改用一次性的
> 兌換碼（one-time exchange code）：
>   `Location: http://127.0.0.1:48181/oauth/callback?code=<short-lived>`
>   然後讓 CLI 把該 code 以 POST 送到
>   `https://phantommesh.io/api/exchange` 以取得真正的 payload。
>   代價是多一趟來回（round-trip）；好處是 URL 中零 token 曝光。

### 1.5 已驗證的 API 介面（供日後 `phantom devices` 等使用）

CLI 取得 `broker_token` 後，便可呼叫：

```
GET  /api/me                            → user profile
GET  /api/devices                       → [ {device_id, label, last_seen, ...}, ... ]
POST /api/devices/{device_id}/claim     → claim a discovery'd device
DELETE /api/devices/{device_id}         → revoke
```

全部以 `Authorization: Bearer <broker_token>` 驗證。這些對 v1 登入
都不是必要的 — 它們是給日後加入的叢集探索層（cluster discovery
layer）用的。

---

## 2. 推薦的技術堆疊（tech stack）

對於 broker，我們在這個規模的專案上看過最省錢、營運負擔最低的
組合是：

| 層 | 推薦 | 原因 |
|---|---|---|
| Hosting（主機代管） | **Cloudflare Workers + Pages** | $0 免費方案涵蓋每天 100K 次請求；無冷啟動；Workers KV / D1 存放狀態 |
| 前端 | 在 Cloudflare Pages 上的 **Next.js 15 / SvelteKit** | 部署於邊緣（edge），OAuth callback 在邊緣執行 |
| 後端 | **Hono**（對 Cloudflare-Worker 友善）或 Next.js API routes | 極簡、類似 Express 的介面；剛好契合 broker 的 6 條 route |
| 資料庫 | **Cloudflare D1**（邊緣上的 SQLite） | 免費 5 GB，完美適合 users + devices + sessions |
| Sessions（工作階段） | **Cloudflare KV**（帶 TTL，存活時間） | OAuth 交握用 60 秒的 session，broker_token 用 7 天 |
| OAuth 函式庫 | **`@auth/core`**（Auth.js v5） | 會講 Google + Apple + email；與框架無關（framework-agnostic） |
| Apple `.p8` 金鑰簽章 | **`jose`**（npm） | EC 私鑰 → JWT，3 行搞定 |

1k MAU（月活躍使用者）時的基礎設施成本估計：**\$0/月**（全部塞進
免費方案）。10k MAU 時：約 $5/月。100k 時：約 $25/月。主要是 D1
列儲存成長。

如果你寧可不用 Cloudflare：

- **Fly.io machine + Postgres** — 更熟悉；一台 $1.94/月的 VM
- **Vercel + Neon** — 一樣的使用體感（ergonomics）；定價相近
- **自架在 Mac coordinator（協調者）上** — phantom serve 已經
  內含 axum + reqwest + sqlite；如果你想保持精簡，大可直接從
  同一個 binary 跑 broker。對公開服務較不理想（NAT / 重啟 / TLS），
  但對隔離的團隊部署很棒

---

## 3. 資料庫綱要（Database schema，D1 / Postgres）

```sql
CREATE TABLE users (
    id           INTEGER PRIMARY KEY,
    email        TEXT NOT NULL UNIQUE,
    provider     TEXT NOT NULL,        -- 'google' | 'apple' | 'email'
    sub          TEXT,                  -- IdP subject
    display_name TEXT,
    avatar_url   TEXT,
    password_hash TEXT,                 -- bcrypt; only set when provider='email'
    created_at   INTEGER NOT NULL,
    last_login_at INTEGER NOT NULL
);

CREATE TABLE devices (
    device_id    TEXT PRIMARY KEY,      -- the phantom CLI's uuid
    user_id      INTEGER NOT NULL REFERENCES users(id),
    label        TEXT,                  -- hostname, mac model, etc.
    public_addr  TEXT,                  -- last-seen tailscale IP
    claimed_at   INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL
);

CREATE TABLE oauth_sessions (
    state        TEXT PRIMARY KEY,
    device_id    TEXT NOT NULL,
    redirect     TEXT NOT NULL,
    code_verifier TEXT NOT NULL,
    created_at   INTEGER NOT NULL
);                                       -- TTL'd by Cloudflare KV after 5 min

CREATE TABLE broker_tokens (
    token_hash   TEXT PRIMARY KEY,       -- SHA-256 of the actual JWT
    user_id      INTEGER NOT NULL REFERENCES users(id),
    device_id    TEXT,                   -- which device the token was issued for
    issued_at    INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,
    revoked_at   INTEGER
);

CREATE INDEX idx_devices_user ON devices(user_id);
CREATE INDEX idx_tokens_user ON broker_tokens(user_id);
```

把所有 join（連接）算進去後，每位活躍使用者約佔 ~200 KB；5 GB 的
D1 免費方案大約可容納 25k 位使用者。

---

## 4. 你需要寫的五個檔案

如果你採用 Cloudflare Workers + Hono 堆疊，整個 broker 塞進五個檔案
就夠了。以下是骨架（skeleton，可直接複製）：

### 4.1 `src/index.ts`

```typescript
import { Hono } from "hono";
import { cors } from "hono/cors";
import { health } from "./routes/health";
import { authStart, googleCallback, appleCallback } from "./routes/oauth";
import { me, devices } from "./routes/api";
import { login } from "./routes/login";

const app = new Hono<{ Bindings: Env }>();
app.use("*", cors({ origin: ["https://phantommesh.io"] }));

app.get("/api/health", health);
app.get("/auth/cli/start", authStart);
app.get("/auth/google/callback", googleCallback);
app.get("/auth/apple/callback", appleCallback);
app.get("/api/me", me);
app.get("/api/devices", devices);
app.get("/login", login);          // server-renders the buttons page

export default app;
```

### 4.2 `src/routes/oauth.ts`（承重的那一個）

```typescript
import type { Context } from "hono";
import { generatePkce, exchangeGoogleCode, validateApple } from "../lib/oauth";
import { upsertUser, claimDevice, mintBrokerToken } from "../lib/db";

export async function authStart(c: Context) {
  const device_id = c.req.query("device_id");
  const port = parseInt(c.req.query("port") ?? "0");
  const redirect = c.req.query("redirect") ?? "";
  if (!isUuid(device_id) || port < 1024 || port > 65535) {
    return c.text("invalid params", 400);
  }
  if (!/^http:\/\/(127\.0\.0\.1|localhost):\d+\/oauth\/callback$/.test(redirect)) {
    return c.text("invalid redirect", 400);
  }
  const state = crypto.randomUUID();
  const { verifier, challenge } = generatePkce();
  await c.env.SESSIONS.put(state, JSON.stringify({ device_id, redirect, verifier }), { expirationTtl: 300 });
  return c.redirect(`/login?state=${state}&challenge=${challenge}`);
}

export async function googleCallback(c: Context) {
  const code = c.req.query("code");
  const state = c.req.query("state");
  const session = JSON.parse(await c.env.SESSIONS.get(state) ?? "{}");
  if (!session.device_id) return c.text("session expired", 400);

  const { id_token, access_token, claims } = await exchangeGoogleCode(c.env, code, session.verifier);
  const user = await upsertUser(c.env, {
    email: claims.email, sub: claims.sub, provider: "google",
    display_name: claims.name, avatar_url: claims.picture,
  });
  await claimDevice(c.env, session.device_id, user.id);
  const broker_token = await mintBrokerToken(c.env, user.id, session.device_id);

  const payload = btoa(JSON.stringify({
    provider: "google", email: claims.email, sub: claims.sub,
    name: claims.name, picture: claims.picture,
    id_token, access_token,
    broker_token,
    broker_token_expires_at_ms: Date.now() + 7*86400_000,
  }));
  return c.redirect(`${session.redirect}?p=${encodeURIComponent(payload)}`);
}

export async function appleCallback(c: Context) {
  // Apple returns id_token as form_post — same shape as Google after that.
  // Use jose to verify Apple's RS256 signature against
  // https://appleid.apple.com/auth/keys
  // ... (mirror googleCallback)
}
```

### 4.3 `src/lib/oauth.ts` — Google + Apple OAuth 輔助函式

標準 PKCE；約 80 行。若你想要現成的，就用
`@auth/core/providers/google` 和 `@auth/core/providers/apple`，或自己
手刻 — Google 的流程不過就是帶著正確表單欄位向
`POST oauth2.googleapis.com/token` 發請求。

### 4.4 `src/lib/db.ts` — D1 包裝函式

`upsertUser`、`getUser`、`claimDevice`、`getDevices`、`mintBrokerToken`、
`verifyBrokerToken`。約 120 行直白的 SQL。

### 4.5 `src/routes/login.tsx` — 真正的登入頁

一個 50 行的伺服器渲染頁，含三顆按鈕：
「Continue with Google」、「Continue with Apple」、「Sign in with email」。
每顆按鈕都是一個 form POST，它會從查詢字串接住 `state`，並重新導向
到對應的 `/auth/{provider}/start` route。

---

## 5. 實作路線圖（roadmap）

| 天 | 範圍 |
|---|---|
| **1** | 把 Cloudflare 帳號 + D1 + KV + Workers 專案初始化好。`/api/health` 回 200。phantom CLI 的 broker 探測從「離線」變成「上線」 — 但其他一切仍是 404。 |
| **2** | `/auth/cli/start` + `/login` 頁 + Google OAuth callback 端到端串接完成。第一次成功的 `phantom login` 來回（僅 Google）。 |
| **3** | 串好 Apple Sign In（這是單一整合中最耗時的 — 需要在 Apple Developer console 中設定 .p8 金鑰、Team ID、Service ID）。 |
| **4** | Email/password（bcrypt）+ `/api/me`/`/api/devices` 端點。 |
| **5** | phantommesh.io 的 DNS + TLS 憑證（Cloudflare proxy 一鍵搞定）。打磨登入頁。加入帳號儀表板（dashboard）。 |
| **6** | 速率限制（rate limiting）+ 濫用防護 + 監控。部署到正式環境（production）。 |

這就是 broker 的 MVP（最小可行產品）。一位專注的工程師約 6 天；若你
讓 phantom autoevolve 幫忙做掉部分工作會更快（把 OAuth 函式庫的文件
放進 context，對 broker repo 跑 `phantom evolve`）。

---

## 6. CLI 端今天保證的事

- CLI **已經** 會在 `phantom login` 不帶參數執行的當下，以 3 秒逾時
  探測 `https://phantommesh.io/api/health`。當它回 200，CLI 就完全
  交棒過去。
- CLI 在 `127.0.0.1:48181/oauth/callback` 上監聽，接受 GET（帶
  `?p=base64(json)`）或 POST（帶 `body: json`）。兩種形狀皆可 —
  挑 broker 端比較好做的那種。
- CLI 會把 broker 回傳的任何 JSON 存到 `~/.phantom-mesh/auth.json`，
  權限模式 0600，並保留這些鍵：`provider`、`email`、`sub`、
  `display_name`、`avatar_url`、`id_token`、`access_token`。payload 中
  其餘的一律捨棄。
- `device_id` 是一個穩定的 UUID v4，CLI 在首次登入時產生，並在
  登出/登入間重複使用。broker 應把 `(user_id, device_id)` 這一對
  視為標準的「Alice 的這台 Mac」。
- 使用者可用
  `PHANTOM_AUTH_URL=https://my-self-hosted-broker.example.com`
  覆寫 broker URL，讓企業（Enterprise）能自架（見
  [docs/design/COMMERCIAL-DESIGN.md](COMMERCIAL-DESIGN.md) §2 硬規則 #4）。
- CLI 永遠不會在身分以外的任何事情上信任 broker — agent 執行、
  工具、secret（密鑰）全都留在本機。broker 只能說「這個 email +
  裝置這一對，就是他們所聲稱的身分」。

---

## 7. broker 必須遵守的信任 + 隱私保證

登入信任界線（trust line）：當使用者把驗證委派給 phantommesh.io 時，
他們信任我們的只有三件事，也僅有這三件事：

1. **他們的 email**（讓我們能辨識他們的帳號）
2. **他們的裝置 ID**（讓他們能看到哪些裝置是自己的）
3. **一個短壽命的 broker_token**（讓我們能驗證來自 CLI 的 `/api/me`
   呼叫）

broker 絕對不可以（MUST NOT）：

- 以明文（plaintext）儲存供應商密碼（email 層請用 bcrypt）
- 接收任何 LLM（大型語言模型）供應商的 API 金鑰 — 那些留在使用者的
  `~/.phantom-mesh/agents.toml` 裡，永遠不該靠近 phantommesh.io
- 接收使用者的 prompt（提示詞）、agent 輸出、檔案路徑或 commit
  訊息
- 把來自 Google / Apple 的 `id_token` / `access_token` 保留得比驗證
  OAuth 交握所需更久 — CLI 收到後即刻丟棄
- 把 broker_token 的 TTL 設得超過 7 天
- 把任何指名特定 repo / 檔案 / 使用者 agent prompt 的東西記入遙測
  （telemetry）日誌

第一條被違反的條目，就是會害我們被 fork（分叉另立）的那一條。
（見 `docs/design/COMMERCIAL-DESIGN.md` §2。）

---

## 8. 反向檢查清單（這些別做）

| ❌ 別做 | 原因 |
|---|---|
| 要求登入才能使用 `phantom` | OSS-binary 契約（開源二進位檔契約） — 永遠不需帳號即可運作。 |
| 在免費方案對 `/api/me` 呼叫加上計量（metering） | OSS 使用者的可被發現性（discoverability）比速率限制帶來的營收更重要。 |
| 打造任何由 broker 在二進位檔內強制執行的「Pro 功能」 | 把它移到獨立的套件；phantom-core 保持開放。 |
| 把我們的雲端 broker URL 以無法關閉的方式綑進二進位檔 | `PHANTOM_AUTH_URL=''` 必須永遠有效。 |
| 在 phantommesh.io 上使用第一方分析工具（GA / Mixpanel） | 只用自架的 Plausible / Umami。 |

---

## 9. 待決問題（在第 1 天開始時決定）

| 問題 | 傾向 |
|---|---|
| 要保留「Sign in with phantom-mesh GitHub OAuth」嗎？ | 第 2 週加入作為第 4 個供應商。受眾與我們的使用者群高度重疊。 |
| 使用者能自行合併兩個 email 嗎（例如先用 Google 登入，之後再把 Apple 加到同一帳號）？ | 可以，第 4 天。綱要已支援（provider 是 per-user，不是 per-row）。 |
| 免費方案的裝置數上限？ | v1 不設限。加入 Pro 時上限設為 10 台裝置。 |
| 要把 broker 開源嗎？ | 要 — BSL 1.1 → 4 年後轉 Apache 2.0。與 docs/design/COMMERCIAL-DESIGN.md §7 相同的 Tailscale 模式。 |

---

## 10. phantommesh.io 上線之後

一旦 `https://phantommesh.io/api/health` 回 200 且 OAuth 交握成功，
既有的 `phantom` 二進位檔就能直接運作 — 不需重新建置（rebuild）、
不需重新燒錄（re-flash）。已安裝 phantom v0.1.0 的使用者，在下一次
執行 `phantom login` 時就會拿到 broker 登入，因為 broker URL 在建置
時就被硬寫（hardcode）成預設值。

這正是 CLI 端要先接好線的承重原因（load-bearing reason）。
