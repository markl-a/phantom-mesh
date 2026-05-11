# 部屬 phantommesh.io — 從手動 deploy 改成 git push 觸發

> Why: 2026-05-03 出過事 — Mac 上跑 `wrangler deploy` 推了舊 checkout，把
> Cloudflare Worker rolled back，弄丟了 cluster-peers endpoints 跟 dashboard
> UI。這個 SOP 杜絕同類問題。

## 結果

**改後**：你不再從任何本地機器跑 `wrangler deploy`。改 phantommesh-io/ 的程式碼 → 推 git → GitHub Actions 自動 deploy。

```
本地 phantommesh-io/ 改檔
   └─ git commit + git push origin phase1-r1-foundations
        └─ GitHub Actions 觸發 (.github/workflows/deploy-phantommesh-io.yml)
             ├─ 安裝 deps + type check
             ├─ wrangler deploy （用 GitHub secret 裡的 CF API token）
             └─ smoke test /api/health 確認 deploy 起來
```

Mac + Win + 任何未來機器都是 git 觀察者，沒有「我比你晚 push 所以我贏」的問題（git 本身就是 last-write-wins，但 git diff 可看誰幹的）。

---

## 一次性 setup（你需要做）

### 1. 建 Cloudflare API token

開 https://dash.cloudflare.com/profile/api-tokens → 點 **Create Token**

選 **Custom token**，**name**: `phantommesh-io GitHub Actions deploy`

**Permissions**:
| Resource | Permission |
|---|---|
| Account · Cloudflare Workers Scripts | Edit |
| Account · Cloudflare Pages | Edit |
| Account · Account Settings | Read |
| Account · Workers KV Storage | Edit |
| Account · Workers R2 Storage | Edit |
| Account · D1 | Edit |
| Zone · Workers Routes | Edit |
| Zone · DNS | Read |

**Account Resources**: Include → 你的 account（`9dc655af3fd4ac4487eade25edcbaa7d`）

**Zone Resources**: Include → Specific zone → `phantommesh.io`

按 **Continue to summary → Create Token**。**複製出來的 token 字串**（看不到第二次）。

### 2. 找 Cloudflare Account ID

開 https://dash.cloudflare.com/ → 隨便選個 site → 右側欄 **Account ID** = `9dc655af3fd4ac4487eade25edcbaa7d`

### 3. 加 GitHub repo secrets

開 https://github.com/markl-a/phantom-mesh-private/settings/secrets/actions → **New repository secret**

加兩個：
- `CLOUDFLARE_API_TOKEN` = 步驟 1 那串 token
- `CLOUDFLARE_ACCOUNT_ID` = `9dc655af3fd4ac4487eade25edcbaa7d`

---

## 驗證

加完 secrets 後第一次 push：
```powershell
cd D:\Projects\phantom-mesh-private
echo "" >> phantommesh-io/.gitignore   # trivial change
git add phantommesh-io/.gitignore
git commit -m "ci: trigger first auto-deploy"
git push origin phase1-r1-foundations
```

開 https://github.com/markl-a/phantom-mesh-private/actions → 看到 **Deploy phantommesh.io** workflow 跑起來。預期約 1 分鐘完成。

之後 phantommesh.io 應該還是正常 serve（沒事發生）。

---

## 改 phantommesh-io 的新流程

**舊流程**（會出包）：
```bash
cd phantommesh-io
# edit files
npx wrangler deploy   # ← 任何機器跑都 deploy 自己 local 的版本
```

**新流程**：
```bash
cd phantommesh-io
# edit files
cd ..
git add phantommesh-io/
git commit -m "feat: ..."
git push origin phase1-r1-foundations
# 等 GitHub Actions 30-60 秒 → 自動 deploy → 完成
```

---

## 失誤情境

### 「我急著 deploy，CI 排隊太慢」
- workflow_dispatch 已加 → 可以直接到 Actions tab 手動觸發 latest commit deploy
- **真的**急的話可以 local `wrangler deploy`，但要清楚知道你 local 的 phantommesh-io/ 跟 origin/phase1-r1-foundations 是不是同步（先 `git pull` + `git status`）

### 「workflow 失敗」
- Actions tab → 點失敗的 run → 看哪一步紅
- 常見：
  - secret 沒設 → 「Error: Cloudflare API Token not set」
  - tsc 編不過 → 看 type error 訊息（可能是新 commit 的 TS 問題）
  - wrangler deploy 失敗 → 通常是 wrangler.toml 跟 cf 帳號不匹配

### 「想暫停 auto-deploy」
- Actions tab → 「Deploy phantommesh.io」workflow 右上角 `...` → Disable workflow
- 之後手動 `wrangler deploy` 一次就好

---

## 為何 R2 binary upload 不在這個 workflow

R2 上傳是 phantom binary（Rust build 產物），跟 Worker code 是不同 build。
- Worker = TypeScript，CI 跑 `wrangler deploy`，秒級
- phantom binary = Rust 跨平台 build，CI 要跑 ~5 分鐘 + 多 platform matrix

未來可以加 `release-phantom-binary.yml` 在 git tag 時 cross-build + 上 R2，但這條 workflow 暫時專注 phantommesh.io worker 本身。

---

## Snapshot：目前的 deploy 來源（手動踩過的）

| 機器 | 部屬過 phantommesh-io | 部屬過 R2 binary |
|---|---|---|
| Z13 (yoyogood) | ✓ 大部分 | ✓ 全部 |
| Mac coordinator | ⚠ 一次（造成 rollback 事件） | ✗ |
| ayaneo / acer | ✗ | ✗ |

**改 git push 觸發後**：上表第一行變成「✓ 自動」，其他全部 ✗（無人機器持有 deploy 權）。
