# iOS 端對端測試流程（End-to-End Test Flow）

驗證 iOS 輕量客戶端（thin client，瘦客戶端：sim 模擬器 + iPhone + iPad）
透過 cluster-dispatch（叢集派送）HMAC（雜湊訊息驗證碼）連線與 Mac coordinator（協調者）
溝通的操作手冊（run-book）。對應 `platform/ios` 在 `9607f44` 之後的狀態。

暖樹（warm tree，已建置過的工作樹）總耗時：約 10 分鐘（大部分花在 build 建置 + install 安裝）。
冷樹（cold tree，無 DerivedData / Externals）：約 25 分鐘。

---

## 0. 前置需求（Prerequisites）

除非另有說明，所有命令都從
`/Users/<you>/path/to/spectyn-mesh-ios`
執行。

```bash
# Branch + worktree
git checkout platform/ios
git pull --ff-only origin platform/ios

# Coordinator on Mac (must be reachable from devices over Tailscale
# or the same Wi-Fi)
curl -s -m 5 http://127.0.0.1:7878/healthz   # → "ok"
grep cluster_secret ~/.spectyn-mesh/agents.toml   # capture for step 5

# Devices paired + on same network as Mac (`localNetwork` transport
# means they're already wifi-debug ready)
xcrun devicectl list devices
#  iPhone 13 mini  (HITRON-9527)   00008110-001134A026F2801E   localNetwork
#  iPad Pro 12.9"  (MarL 的 iPad)  00008027-0018296E2E22002E   localNetwork
```

安裝時裝置必須處於**解鎖**狀態，developer disk image（開發者磁碟映像檔）才能掛載；
否則會出現 `kAMDMobileImageMounterDeviceLocked`。

---

## 1. Build — 線路層級測試（wire-level test，完全跳過 Xcode）

在牽涉任何 iOS 工具鏈之前，先確認 coordinator + cluster-dispatch contract（契約）。
能在數秒內抓出 coordinator 的回歸錯誤（regression）。

```bash
cat > /tmp/test-cluster-dispatch.mjs <<'EOF'
import crypto from "node:crypto";
const COORD = "http://127.0.0.1:7878";
const SECRET = "<paste cluster_secret here>";
const body = JSON.stringify({ agent: "master", prompt: "1+1=" });
const auth = crypto.createHmac("sha256", SECRET).update(body).digest("hex");
const r1 = await fetch(`${COORD}/rpc/task/assign`, {
  method: "POST",
  headers: { "Content-Type": "application/json", "X-Cluster-Auth": auth },
  body,
});
const { job_id } = await r1.json();
console.log(`assign → ${job_id}`);
for (let i = 0; i < 60; i++) {
  await new Promise(r => setTimeout(r, 500));
  const j = await (await fetch(`${COORD}/rpc/task/status/${job_id}`)).json();
  if (j.status === "done")  { console.log(`done: ${j.output}`); process.exit(0); }
  if (j.status === "error") { console.log(`error: ${j.error}`); process.exit(1); }
}
console.log("timeout"); process.exit(1);
EOF
node /tmp/test-cluster-dispatch.mjs
```

通過條件：約 10 秒內印出 `done: 2`（或類似的算術答案）。

若此步驟失敗，繼續下去沒有意義 — 請先修好 coordinator 端
（`/rpc/task/assign`、`core/src/mesh.rs::make_auth_token_bytes`、
`~/.spectyn-mesh/agents.toml` 裡的 agent runtime providers，代理執行時供應商），
再去碰 iOS。

---

## 2. Build — iOS 模擬器 app

```bash
./scripts/package-ios.sh --sim
#  → dist/spectyn-ios-sim.app  (~97 MB, debug, unsigned)
```

冷樹時此腳本會：
1. 預先建立 `Externals/{arm64,x86_64}/{debug,release}` 目錄樹，
   讓 `xcodegen generate` 通過驗證。
2. 清掉 DerivedData + 相反組態（opposite-config）的目錄，避免 `duplicate
   output file libapp.a`。
3. 對 `Externals/arm64/debug/libapp.a` 放置 stub（占位檔），讓 xcode 的前置
   輸入檢查在 script phase 之前就通過。
4. 執行 `npx tauri ios build --debug --target aarch64-sim --no-sign
   --ci --ignore-version-mismatches`。

若冷樹執行時看到 `error: The file "libapp.a" couldn't be opened because
there is no such file`，重試一次即可 — 這是已知的 xcode 不穩定競態（flaky race）。
腳本的 stubbing 理應能避免它，但 Xcode 的增量狀態（incremental state）
並非完全可預期（deterministic）。

---

## 3. 在模擬器中驗證

挑一個已經開機的 sim（Xcode 選單 → Open Developer Tool →
Simulator），或明確啟動一個：

```bash
SIM_ID=$(xcrun simctl list devices available \
         | grep -E "iPhone 17 Pro " | grep -oE "[A-F0-9-]{36}" | tail -1)
xcrun simctl boot      "$SIM_ID" 2>/dev/null || true
open -a Simulator      --args -CurrentDeviceUDID "$SIM_ID"
xcrun simctl terminate "$SIM_ID" ai.spectynmesh.app 2>/dev/null
xcrun simctl uninstall "$SIM_ID" ai.spectynmesh.app 2>/dev/null
xcrun simctl install   "$SIM_ID" dist/spectyn-ios-sim.app
xcrun simctl launch    "$SIM_ID" ai.spectynmesh.app
sleep 4
xcrun simctl io        "$SIM_ID" screenshot /tmp/ios-sim-launch.png
```

Log（日誌）預期（`xcrun simctl spawn "$SIM_ID" log show --last 30s
--predicate 'process == "Spectyn Mesh"' --info`）：

- ✓ `SpectynMeshRuntime ready in 0.0Xs`
- ✓ 不應出現 `HTTP server failed: Address already in use`
  （`lib.rs` 的 cfg(desktop)-gate，commit `1abf2f5`）
- ⚠ `agents.toml not found; checked: [...]` 是預期且無害的；
  cluster 模式不需要本機的 agents.toml。

截圖預期（`/tmp/ios-sim-launch.png`）：

- 底部 3 個分頁：對話 / 節點 / 設定
- Chat 分頁標題列有 cluster toggle（叢集開關，未設定前為停用）
- 歡迎畫面顯示 prompt cards（提示卡：學習 / 工作 / 生活 / 創作）

---

## 4. Build + install — 透過 wifi 安裝裝置 IPA

```bash
./scripts/package-ios.sh
#  → dist/spectyn-ios.ipa  (~31 MB, signed by team F7683B69U7)
```

`bundle.iOS.developmentTeam` 已寫死（hardcoded）在 `tauri.conf.json`；
不需要環境變數。免費的 Apple Development 憑證 → IPA 在 build 後 7 天過期。

```bash
IPHONE=00008110-001134A026F2801E
IPAD=00008027-0018296E2E22002E
IPA=dist/spectyn-ios.ipa

xcrun devicectl device install app --device "$IPHONE" "$IPA"
xcrun devicectl device install app --device "$IPAD"   "$IPA"
xcrun devicectl device process launch --device "$IPHONE" ai.spectynmesh.app
xcrun devicectl device process launch --device "$IPAD"   ai.spectynmesh.app
```

驗證：

```bash
xcrun devicectl device info processes --device "$IPHONE" | grep -i spectyn
xcrun devicectl device info processes --device "$IPAD"   | grep -i spectyn
# Each prints one PID + bundle path under
# /private/var/containers/Bundle/Application/<GUID>/Spectyn Mesh.app/Spectyn Mesh
```

若啟動時出現 `FBSOpenApplicationErrorDomain error 7
(Locked)` 錯誤，解鎖裝置後重試 — 安裝其實已成功，只是
啟動需要裝置處於喚醒狀態。

---

## 5. Cluster 設定（在裝置上）

每台裝置手動執行：

1. 點 **Spectyn Mesh** → 進入 chat 分頁。
2. 輸入任意 prompt → **Send**。出現紅色橫幅（banner）：
   `尚未設定 cluster — 點此到「設定 → Cluster 派送」`。
3. 點該橫幅。透過 commit `e1f4b14` 接好的 deep-link（深層連結），
   路由（route）到 `/settings/cluster`。
4. 填入：
   - **Coordinator URL**：`http://<mac-tailscale-ip>:7878`
     （Mac 的 Tailscale IP；替換成 coordinator 上 `tailscale ip -4`
     回報的數值）。
   - **Cluster Secret**：貼上 Mac 上 `~/.spectyn-mesh/agents.toml`
     裡的 `cluster_secret` 值。
5. 點 **測試 dispatch**。成功（✓）時：
   - 顯示 `回應：<short answer> (<ms>ms, job <8-char>)`。
   - Cluster 模式開關自動翻成 ON（commit `e1f4b14`）。
6. 點返回箭號（back chevron）→ 回到 chat 分頁。chat 標題列裡的
   cluster toggle 現在應呈綠色。

---

## 6. 正向派送（Forward dispatch）— 透過 cluster 聊天

在裝置上：

1. 輸入 `1+1=` → **Send**。
2. 短暫出現轉圈（spinner），接著答案（`2`）出現。
3. 重設對話（垃圾桶圖示），試一個較長的 prompt：
   `用 5 句話講解 Rust ownership`。視該 agent 的 provider 而定，
   預期在 5–15 秒內得到多段落回應。

在 Mac 上（coordinator 端，可選 — 用來確認請求有送達）：

```bash
# Tail the coordinator log for incoming /rpc/task/assign hits
journalctl --user -u spectyn-mesh -f 2>/dev/null \
  || tail -f ~/.spectyn-mesh/logs/app.log.* 2>/dev/null \
  || true
```

每一個成功的聊天氣泡（chat bubble）對應一次對 `/rpc/task/assign`
的 POST + N 次對 `/rpc/task/status/<id>` 的輪詢（poll）。

---

## 7. 失敗模式檢查表（Failure-mode checklist）

針對每種可見失敗代表什麼的快速分流（triage）表：

| 裝置上的徵狀 | 可能原因 | 修法 |
|---|---|---|
| 設定後紅色橫幅不消失 | 測試 dispatch 失敗 → toggle 沒自動啟用 | 看橫幅明細；再點一次 測試，修正 URL/secret |
| `assign 401: unauthorized` | cluster_secret 錯誤 | 從 coordinator 的 `agents.toml` 重新貼一次 |
| `assign 404` 或 fetch 失敗 | coordinator 無法連線 | 檢查裝置上的 Tailscale；從 Mac 執行 `curl <URL>/healthz` |
| `timeout after 60000ms` | agent runtime 卡住（provider 限流 / 無 api-key） | 檢查 coordinator log 找出失敗的 provider |
| 聊天氣泡中出現 `(no output)` | agent 成功但回傳空白 | 是 provider 問題，不是 iOS 問題 |
| App 啟動時白畫面（white-screen） | binary 內的前端 bundle 過期（stale） | Touch `app/src-tauri/src/lib.rs` + 重新建置（cargo 的 `rerun-if-changed=../dist` 理應處理，但手動 touch 可強制觸發） |
| App 立即崩潰 | Tauri ABI 破壞或 symbol-strip（符號剝除）做過頭 | 檢查 `xcrun simctl spawn <SIM> log show` 找 panic / SIGABRT |

---

## 8. 免費憑證更新（Free-cert renewal）

免費的 Apple Development 憑證在 IPA build 後 7 天過期。
一個 `/schedule` routine（排程例程，`trig_01VySWaHMoTodsWcqZvRQtKA`）會在每週四
09:00 Asia/Taipei 自動開一個 GitHub issue，提醒重跑本文件的步驟 4-5。

重新 build + reinstall 之後，localStorage 設定
（`spectyn-mesh-cluster-mode` Zustand persist key）會保留 — 除非
使用者反安裝（uninstall），否則裝置不需要重新設定。
