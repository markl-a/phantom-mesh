# 部署與自動更新設定指南

## 一次性設定（GitHub Secrets）

到 GitHub Repo → Settings → Secrets and variables → Actions，新增以下 secrets：

### Android
| Secret 名稱 | 說明 | 取得方式 |
|---|---|---|
| `ANDROID_KEYSTORE_BASE64` | Keystore 的 base64 | `keytool -genkey ...` 後 `base64 phantom-mesh.keystore` |
| `ANDROID_KEY_ALIAS` | Key 別名 | 建立 keystore 時設定 |
| `ANDROID_KEY_PASSWORD` | Key 密碼 | 建立 keystore 時設定 |
| `ANDROID_STORE_PASSWORD` | Store 密碼 | 建立 keystore 時設定 |
| `FIREBASE_APP_ID` | Firebase App ID | Firebase Console → 專案設定 |
| `FIREBASE_SERVICE_ACCOUNT` | Firebase 服務帳號 JSON | Firebase Console → 服務帳號 |

```bash
# 產生 Android keystore（只需一次）
keytool -genkey -v \
  -keystore phantom-mesh.keystore \
  -alias phantom-mesh \
  -keyalg RSA -keysize 2048 -validity 36500 \
  -dname "CN=Phantom Mesh, O=Your Org, C=TW"

# 轉成 base64 存入 secret
base64 -i phantom-mesh.keystore | pbcopy  # macOS
```

### iOS
| Secret 名稱 | 說明 |
|---|---|
| `APPLE_CERTIFICATE_P12_BASE64` | 簽名憑證 P12 的 base64 |
| `APPLE_CERTIFICATE_PASSWORD` | P12 密碼 |
| `APPLE_PROVISIONING_PROFILE_BASE64` | Provisioning Profile 的 base64 |
| `APPLE_TEAM_ID` | Apple Developer Team ID（例如 `YOUR_APPLE_TEAM_ID`）|
| `APPLE_API_KEY_ID` | App Store Connect API Key ID |
| `APPLE_API_ISSUER` | App Store Connect API Issuer |

```bash
# 從 Keychain 匯出憑證
# Xcode → Preferences → Accounts → 匯出憑證為 .p12
base64 -i certificate.p12 | pbcopy

# Provisioning Profile
base64 -i profile.mobileprovision | pbcopy
```

### Oracle Cloud
| Secret 名稱 | 說明 |
|---|---|
| `ORACLE_SSH_KEY` | SSH 私鑰內容（整個 PEM 文字）|
| `ORACLE_VM_IP` | Oracle VM 的公開 IP |
| `ORACLE_VM_USER` | SSH 使用者（通常是 `opc`）|

### Tauri Updater 簽名
| Secret 名稱 | 說明 |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | 更新簽名私鑰 |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 私鑰密碼 |

```bash
# 產生 Tauri 更新簽名 key（只需一次，在 app/ 目錄執行）
cd app
npm run tauri signer generate -- -w ~/.tauri/phantom-mesh.key

# 輸出：
# Finished generating private key at: ~/.tauri/phantom-mesh.key
# Your pubkey: dW5t...（複製這個填入 tauri.conf.json 的 pubkey 欄位）
#
# 私鑰內容存入 TAURI_SIGNING_PRIVATE_KEY secret：
cat ~/.tauri/phantom-mesh.key | pbcopy
```

---

## 發布新版本

```bash
# 1. 更新版本號
# 修改 app/src-tauri/tauri.conf.json 的 "version" 欄位
# 修改 core/Cargo.toml 的 version 欄位

# 2. Push tag 觸發 CI
git tag v1.2.0
git push origin v1.2.0

# 自動執行：
# ✓ release-mobile.yml  → 建置 APK + IPA，上傳 Firebase/TestFlight
# ✓ release-oracle.yml  → 建置 Linux binary，部署到 Oracle VM
# ✓ release-desktop.yml → 建置 macOS/Windows/Linux app，產生 latest.json
```

---

## Oracle VM 初次設定

```bash
# 在 Oracle VM 上建立 systemd 服務（只需一次）
sudo tee /etc/systemd/system/phantom-mesh.service > /dev/null << 'EOF'
[Unit]
Description=Phantom Mesh
After=network.target

[Service]
ExecStart=/home/opc/phantom-mesh
Restart=on-failure
RestartSec=5
User=opc
Environment="RUST_LOG=info"

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl enable phantom-mesh

# 開放防火牆（OCI Security List 也要開）
sudo firewall-cmd --permanent --add-port=7878/tcp
sudo firewall-cmd --reload
```
