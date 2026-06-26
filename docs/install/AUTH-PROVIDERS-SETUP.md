# Auth Providers Setup — Google / Apple Sign-In

身分登入(IdP)的 provisioning 指南。**Code 已實作**(`core/src/oauth.rs` + CLI `login_google` / broker apple path);
這份只處理「在你自己的 Google Cloud / Apple Developer 帳號下建立 phantom-mesh app + 設定」。

> 背景:目前 code 內建的 `GOOGLE_CLIENT_ID`(`869770808980-…`)和 `APPLE_CLIENT_ID`(`ai.phantommesh.auth`)
> 是**舊 app** 的設定。下面用新的環境變數覆蓋,不必改 code 重編。

---

## 新增的環境變數(本次 code 改動)

| Env | 用途 | 預設行為 |
|---|---|---|
| `PHANTOM_MESH_GOOGLE_CLIENT_ID` | 覆蓋 Google OAuth client id（CLI + app/web 都吃）| 未設 → 用內建舊 default |
| `PHANTOM_MESH_GOOGLE_CLIENT_SECRET` | **可選**。只有「Web application」型 client 需要 | 未設 → 走純 PKCE（Desktop app 型不需 secret）|

Apple 走 `~/.phantom-mesh/apple-auth.json`(見下),不用 env。

---

## Google

### 建議路線：Desktop app 型(給 CLI,免 secret)
1. Google Cloud Console → **APIs & Services → Credentials → Create OAuth client ID**
2. Application type = **Desktop app**,Name = `phantom-mesh`
3. Desktop 型自動允許 loopback;CLI 用的 redirect 是 `http://127.0.0.1:48181/oauth/callback`
4. 複製 client id → `export PHANTOM_MESH_GOOGLE_CLIENT_ID=<新id>`
5. **OAuth consent screen**:app name 設 `phantom-mesh`,scopes = `openid email profile`
6. 實測:`phantom login google` → 開瀏覽器 → 同意 → 看到 "✓ Login complete"
   - Desktop 型 + PKCE → **不需** `PHANTOM_MESH_GOOGLE_CLIENT_SECRET`

### App / Web 路線：Web application 型
- Application type = **Web application**
- Authorized redirect URIs 加:
  - `http://localhost:5173`(app 前端)
  - `http://127.0.0.1:48181/oauth/callback`(CLI)
  - `http://localhost:<daemon_port>/oauth/callback`(daemon)
- Web 型**必須**有 secret → `export PHANTOM_MESH_GOOGLE_CLIENT_SECRET=<secret>`

---

## Apple Sign-In

**需要付費 Apple Developer Program 會員**(免費憑證不行)。

1. Apple Developer → **Certificates, Identifiers & Profiles**
2. **Identifiers → Services ID** → 建 `phantom-mesh`,identifier 例如 `ai.phantommesh.auth`
   (要換名就改 `apple-auth.json` 的 `client_id`)
3. 開啟 **Sign in with Apple** capability
4. **Keys** → 新建 key、勾 Sign in with Apple → 下載 `.p8`(只能下載一次)→ 記下 **Key ID**
5. 記下你的 **Team ID**
6. Services ID 的 Sign in with Apple 設定 → Return URL = relay:
   `https://apple-oauth-relay.vercel.app/auth/apple-callback`
7. 寫 `~/.phantom-mesh/apple-auth.json`:
   ```json
   {
     "client_id": "ai.phantommesh.auth",
     "team_id": "<YOUR_TEAM_ID>",
     "key_id": "<YOUR_KEY_ID>",
     "p8_path": "/Users/<you>/.phantom-mesh/AuthKey_<KEYID>.p8"
   }
   ```
   檔案存在後 `apple_available()` 自動變 true。
8. 確認 relay `apple-oauth-relay.vercel.app` 有部署且能轉回你的 daemon。
9. 實測:`phantom login apple`(走 broker relay)或 app/web 的 Apple 按鈕。

### iOS 原生
- App Store 規定:有第三方登入就**必須**提供 Sign in with Apple。
- 需要**付費** Program Team ID(≠ 免費簽章憑證 `YX7U4J39PX`)。
- iOS 原生走 `ASAuthorization`(與本文 relay flow 不同),待實作。

---

## 收尾(等新 app 穩定後)
把新的 phantom-mesh client id 烤進 `oauth.rs` / `phantom.rs` 的內建 default,取代舊的 `869770808980-…`,
這樣不靠 env 也是新值。在那之前 env 覆蓋即可。
