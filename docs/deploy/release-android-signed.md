# 已簽章的 Android APK 發佈 — 操作者指南

本文件說明如何設定 GitHub Actions secrets（密鑰），讓
`Release Mobile (Signed APK)` 工作流程（`.github/workflows/release-mobile-signed.yml`）
能在每次推送 `v*` tag（標籤）時，產生一份正式環境簽章（production-signed）的 APK。

**Spec（規格）：** v0.6.0 V4 硬性把關（hard-gated）發佈（T53）。

## 工作流程一覽

| 觸發條件 | 效果 |
|---|---|
| `git push origin v0.6.0` | 建置已簽章的 APK 與 `spectyn-aarch64-linux-android`，並將兩者以**預發佈（prerelease）**資產（asset）形式附加到 `v0.6.0` GitHub Release（發佈）。 |
| 帶有 `tag` 輸入的 `workflow_dispatch` | 同上，但針對任意一個已經存在的 tag。 |
| 四個 `ANDROID_*` secrets 中任一缺漏 | 工作流程在 `verify-secrets` job（工作）中**快速失敗（fails fast）**——不浪費任何 Android 工具鏈（toolchain）的時間。 |

未簽章／debug 簽章的 CI 建置仍可透過既有的
`release-mobile.yml` 工作流程取得；兩者刻意保持獨立，因此一份
損壞的 keystore（金鑰庫）不會阻斷 CI。

## 必要的 GitHub Actions secrets

請在儲存庫的 **Settings -> Secrets and variables -> Actions** 設定以下四個 secrets：

| Secret 名稱 | 說明 |
|---|---|
| `ANDROID_KEYSTORE_BASE64` | `keystore.jks` 內容的 Base64 編碼（不需要換行，但有換行也無害）。 |
| `ANDROID_KEYSTORE_PASSWORD` | 建立 keystore 時使用的 `-storepass` 值。 |
| `ANDROID_KEY_ALIAS` | keystore 內簽章金鑰的 alias（別名，例如 `spectyn-mesh-release`）。 |
| `ANDROID_KEY_PASSWORD` | 每把金鑰各自的 `-keypass` 值。可以與 storepass 相同，但建議使用不同的值。 |

> **命名注意：** 舊版的 `release-mobile.yml` 工作流程使用
> `ANDROID_STORE_PASSWORD`（沒有 `KEY` 中綴）。新的 T53 工作流程依照
> v0.6.0 V4 規格，統一採用 `ANDROID_KEYSTORE_PASSWORD`。如果你已經設定了
> `ANDROID_STORE_PASSWORD`，請將它的值同步到 `ANDROID_KEYSTORE_PASSWORD`，
> 而不是改名——這樣可讓兩個工作流程在過渡期都能運作。

## 一次性的 keystore 建立

請在受信任的本機上執行（不要在 CI 執行）。keystore 是每一次未來 APK 更新的
信任根（root of trust）——一旦遺失，就代表現有的安裝無法原地（in place）升級。

```bash
# 1. Pick strong, distinct passwords. Store them in a password manager.
STORE_PW='<long random string>'
KEY_PW='<different long random string>'
KEY_ALIAS='spectyn-mesh-release'

# 2. Generate a 25-year RSA key.
keytool -genkeypair \
  -v \
  -keystore keystore.jks \
  -alias "$KEY_ALIAS" \
  -keyalg RSA \
  -keysize 4096 \
  -validity 9125 \
  -storepass "$STORE_PW" \
  -keypass   "$KEY_PW" \
  -dname "CN=Spectyn Mesh, OU=Release, O=Spectyn Mesh, L=Taipei, C=TW"

# 3. Verify.
keytool -list -keystore keystore.jks -storepass "$STORE_PW"

# 4. Base64-encode for GitHub Actions (no line wraps; macOS uses `base64 -i`).
base64 -w0 keystore.jks > keystore.jks.b64
```

## 設定 secrets

### 選項 A — gh CLI（推薦）

```bash
gh secret set ANDROID_KEYSTORE_BASE64    --body "$(cat keystore.jks.b64)"
gh secret set ANDROID_KEYSTORE_PASSWORD  --body "$STORE_PW"
gh secret set ANDROID_KEY_ALIAS          --body "$KEY_ALIAS"
gh secret set ANDROID_KEY_PASSWORD       --body "$KEY_PW"
```

### 選項 B — 網頁介面（Web UI）

1. Settings -> Secrets and variables -> Actions -> **New repository secret**。
2. 對上述四個 secret 名稱各重複一次。
3. 對於 `ANDROID_KEYSTORE_BASE64`，請把 `keystore.jks.b64` 的全部內容
   貼到值欄位（value field）。前後不要有空白。

## 設定 secrets 之後

```bash
# Sanity-check from your laptop — no toolchain needed.
gh secret list | grep ANDROID_

# Trigger the workflow on an existing tag without cutting a new one.
gh workflow run release-mobile-signed.yml -f tag=v0.6.0
gh run watch
```

該工作流程會：

1. 先執行 `verify-secrets`（若四個 secrets 中任一缺漏則快速失敗）。
2. 用 Tauri + Gradle 建置未簽章的 release APK。
3. 把 keystore 解碼到 `$RUNNER_TEMP/keystore.jks`，用
   `apksigner` 為 APK 簽章，並驗證簽章。
4. 清除已解碼的 keystore（縱深防禦（defense-in-depth）——無論如何 `$RUNNER_TEMP`
   都會被 runner 自動清理）。
5. 把 `spectyn-mesh-<tag>-signed.apk` 上傳到對應的 GitHub Release，作為
   **預發佈（prerelease）**資產。
6. 同時並行建置並附加 `spectyn-aarch64-linux-android`（Termux
   coordinator（協調者）二進位檔），讓終端使用者可以從同一個 Release 頁面安裝這兩個部分。

在實體裝置上對 APK 做冒煙測試（smoke-test）後，透過 Releases UI 將該預發佈
提升（promote）為「latest（最新）」。

## 本機預檢（選用）

如果你想在不推送 tag 的情況下驗證簽章流程：

```bash
# Build an unsigned APK locally.
cd app && npm install && npm run tauri android build -- --apk

UNSIGNED=src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk

# Sign with the same keystore CI will use.
"$ANDROID_HOME/build-tools/35.0.0/apksigner" sign \
  --ks /path/to/keystore.jks \
  --ks-key-alias "$KEY_ALIAS" \
  --ks-pass "pass:$STORE_PW" \
  --key-pass "pass:$KEY_PW" \
  --out spectyn-mesh-local-signed.apk \
  "$UNSIGNED"

"$ANDROID_HOME/build-tools/35.0.0/apksigner" verify --verbose spectyn-mesh-local-signed.apk
```

如果本機簽章成功，CI 簽章也會成功。

## 威脅模型與操作注意事項

- **Secret 外洩：** GitHub Actions 永遠不會回顯（echo）secret 的值，但一個
  惡意的工作流程變更有可能將它們外洩（exfiltrate）。請以必要審查（required reviews）
  與針對 `.github/workflows/` 的 CODEOWNERS 保護 `main` 分支。
- **Keystore 輪替（rotation）：** Android 不支援安裝後的簽章者輪替
  （除非使用 Play App Signing，否則使用者必須先解除安裝）。請把
  keystore 視為長期存在的資產；將它備份到兩個不同的
  加密位置。
- **遭入侵應變：** 如果 keystore 外洩，立即的緩解措施
  是用一份新的 keystore 簽章並發佈新的 APK，並指示使用者
  解除安裝後重新安裝。對於以側載（sideload）方式散佈的
  APK，並沒有原地復原（in-place recovery）的方法。
- **不在 commit 歷史中曝光：** 這四個 secrets 只會存在於 GitHub
  Actions 的 secret 儲存區，以及單一 job 執行期間的 `$RUNNER_TEMP` 中。它們
  絕不可被提交（commit）進儲存庫，即使是加密形式也不行。

## 相關工作流程

- `.github/workflows/release-mobile.yml` — 舊版整合 Android + iOS 的
  工作流程。當 `ANDROID_KEYSTORE_BASE64` 缺漏時，會退回到 debug 簽章的 APK。
  保留它是為了 iOS 以及未簽章的測試者（tester）建置。
- `.github/workflows/release-daemon.yml` — 為所有桌面目標
  以及 `spectyn-aarch64-linux-android` 建置 coordinator 二進位檔。已簽章的
  工作流程會重新建置 Android 二進位檔，因此即使 `release-daemon.yml` 正在執行中，
  它仍能附加到 Release。
