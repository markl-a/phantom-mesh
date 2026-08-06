# Mac local deploys — staging env (不影響 phantommesh.io 線上)

> 用途：Mac 開發時想試 spectynmesh-io 改動，但不想動到 production 的
> phantommesh.io 服務。answer = 用 wrangler 的 `--env staging`。

---

## 兩個 env 對照

| | **production** (default) | **staging** |
|---|---|---|
| Worker name | `spectynmesh-io` | `spectynmesh-io-staging` |
| 公開 URL | https://phantommesh.io | https://spectynmesh-io-staging.`<account>`.workers.dev |
| 誰能 deploy | **GitHub Actions only** | **任何機器** (Mac/Win) `wrangler deploy --env staging` |
| 觸發方式 | git push → CI 自動 | 手動 `wrangler deploy --env staging` |
| Bindings (D1/R2/KV) | `spectynmesh-prod` 等 | **同上**（共享資料） |
| Routes | `phantommesh.io/*` | 無 route，純 workers.dev URL |

> 共享 bindings = staging 看得到 prod 的真實 user/keys/peers/binary data。
> 但 staging 的 **code** 改變不會影響 phantommesh.io 上面跑的版本。
> 想要連 data 都隔離見「[完全隔離 staging](#完全隔離-staging)」一節。

---

## Mac dev 流程

### 一次性 setup（每台機器跑一次）

```bash
# 1. clone repo (如果還沒)
cd ~/Projects
git clone https://github.com/markl-a/spectyn-mesh.git
cd spectyn-mesh/spectynmesh-io

# 2. 安裝 deps
npm install

# 3. wrangler login（瀏覽器一次性 OAuth）
npx wrangler login

# 4. 設 staging 環境的 secrets（跟 prod 同樣 4 個）
npx wrangler secret put GOOGLE_CLIENT_SECRET --env staging
npx wrangler secret put BROKER_JWT_SECRET    --env staging
npx wrangler secret put ENV_VAULT_KEY        --env staging
# Apple 兩個也要加（如果之後做 Apple 登入）
# npx wrangler secret put APPLE_KEY_ID        --env staging
# npx wrangler secret put APPLE_P8_PRIVATE_KEY --env staging
```

### 日常 dev

```bash
cd ~/Projects/spectyn-mesh/spectynmesh-io

# 改檔...

# 部到 staging（任意次數，無風險）
npx wrangler deploy --env staging

# 看 staging 跑得對不對：
curl -sS https://spectynmesh-io-staging.<your-account>.workers.dev/api/health
```

### 確認改動準備上 prod

當 staging 看起來 OK，要上 prod 的時候 ：
```bash
git add .
git commit -m "feat: ..."
git push origin phase1-r1-foundations
```

GitHub Actions 自動 deploy 到 prod = phantommesh.io。**Mac 永遠不該手動部 prod**。

---

## 規則總結

| Action | 允不允許 |
|---|---|
| Mac: `npx wrangler deploy --env staging` | ✅ 任何時候 |
| Mac: `npx wrangler deploy` (無 --env，等於 production) | ❌ **不要做**（會繞過 git，直接覆蓋 phantommesh.io） |
| Win: `npx wrangler deploy --env staging` | ✅ 跟 Mac 一樣 |
| Win: `npx wrangler deploy` | ❌ 同上 |
| `git push` 觸發 CI deploy 到 prod | ✅ 唯一 prod 部屬路徑 |

> 想徹底防呆：把 `[env.production]` 也加進 wrangler.toml 然後**移除頂層的 default env 設定**，這樣手動 `wrangler deploy` 不指 --env 會直接 error。但這需要改 GitHub Actions 工作流要傳 `--env production`。後續可以做。

---

## 完全隔離 staging（資料也分開）

如果 staging 要測 schema migration / 不想看到真實 user data，建立獨立的 D1/KV/R2：

```bash
cd spectynmesh-io

# 建新資源
npx wrangler d1 create spectynmesh-staging
# 印出 database_id，記下來

npx wrangler kv namespace create SESSIONS_STAGING
# 印出 id，記下來

npx wrangler r2 bucket create spectyn-binaries-staging

# 套用同樣的 schema migrations 到新 DB
npx wrangler d1 execute spectynmesh-staging --remote --file=./migrations/0001_init.sql
npx wrangler d1 execute spectynmesh-staging --remote --file=./migrations/0002_user_settings.sql
npx wrangler d1 execute spectynmesh-staging --remote --file=./migrations/0003_cluster_peers.sql
```

然後改 `wrangler.toml` 的 `[env.staging.*]` bindings 用新 id。

之後 staging 跟 prod 完全分離 — staging 寫 user 資料、修 schema，全都不影響 phantommesh.io。

---

## Smoke test staging 部好了

```bash
# 1. 看 worker 跑起來
curl -sS https://spectynmesh-io-staging.<account>.workers.dev/api/health
# expect: {"status":"ok","version":"0.1.1-staging",...}
#                                ↑ 注意是 0.1.1-staging 不是 0.1.1-deploy-2026-05-03

# 2. /api/me/cluster-peers 應該 401（沒登入）
curl -sS https://spectynmesh-io-staging.<account>.workers.dev/api/me/cluster-peers
# expect: {"error":"unauthenticated"}
```

---

## 常見錯誤

### `wrangler deploy --env staging` 失敗：「Worker not found」
你還沒第一次 deploy。這個是雞生蛋問題 — 第一次部會自動建。檢查 token 有 `Workers Scripts: Edit` 權限。

### Mac deploy 後 `phantommesh.io` 變了
你忘記加 `--env staging`，部到 default env 把 prod 蓋了。回 GitHub repo `git pull`，再 push 一次（CI 會把 prod 改回 git HEAD）。

### staging 用同個 D1 但你不想資料動
照「完全隔離 staging」一節。改 `[env.staging.d1_databases]` 用新 db id。

### 想把 staging 也接 GitHub Actions
.github/workflows/deploy-spectynmesh-io.yml 加一個 staging job，trigger 改成 push 到 `staging/*` branch。但通常不需要 — staging 就是給 dev 自由部，CI 只管 prod。
