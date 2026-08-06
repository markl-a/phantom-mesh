# spectyn 於 iOS — 安裝指南

iOS 僅為**精簡客戶端（thin client，輕量前端）**。手機只負責執行使用者介面（UI）；所有工具、大型語言模型（LLM）呼叫，以及代理迴圈（agent loop）都在它透過 Tailscale 連線的 Mac／Linux／Windows spectyn serve 節點上執行。這是 iOS 沙盒（sandbox，隔離執行環境）模型的硬性限制 — 詳情請見 SESSION_RESUME 與 gap-analysis（落差分析）相關文件。

如果你想要一個真正在裝置上執行的代理（on-device agent），請改用 Android（Termux + spectyn 二進位檔）或桌面作業系統。

---

## 你會得到什麼

一個已簽署的 `spectyn-mesh-ios.ipa`（約 7 MB，arm64），由與 Android APK 相同的 React 精簡外殼（thin-shell）建置而成。啟動時會詢問協調者（coordinator，協調節點）的主機／連接埠，接著載入 `<host>:<port>/m`（我們隨附於 `core/web/mobile.html` 的行動聊天介面）。

此 IPA 以免費的 Apple Development 憑證簽署 — 這意味著：
- 它可在已用你的 Apple ID 註冊過的手機上運作
- 它在 **7 天**後過期（Apple 對免費開發憑證的政策）
- 每個 Apple ID 最多可旁載（sideload，繞過官方商店安裝）至 **3 部裝置**

若要在更多裝置上做更長效的安裝，你需要一個付費的 Apple Developer 帳號，並以 `iPhone Distribution` 憑證重新簽署；我們不提供該流程。

---

## 先決條件

- 一台執行 Xcode 的 Mac（或僅需 Apple Configurator 2）
- 一個免費的 Apple Developer 帳號，且已用你登入 iPhone 所使用的同一個 Apple ID 註冊
- 一部執行 iOS 16+ 的 iPhone 或 iPad，且與協調者位於同一個 Tailscale 網域（tailnet）
- 一條 USB-C／Lightning 傳輸線 — IPA 透過傳輸線旁載，而非透過空中傳輸（over the air）

---

## 取得 IPA

在 tailnet 上的任何裝置：

```bash
curl -O http://<mac-tailscale-ip>:7878/dist/spectyn-mesh-ios.ipa
```

（請將 IP 換成你協調者的 Tailscale 位址。）

或透過 Safari 下載 — 同一網址，存到 Files／Downloads。

---

## 安裝 — 三種選項

### 選項 A：Apple Configurator 2（Mac App Store，免費）

若你的 Mac 已安裝 Xcode（憑證鏈已受信任），這是最簡單的途徑。

1. 在 Mac 上開啟 Apple Configurator 2
2. 以傳輸線接上 iPhone、解鎖、「信任這部電腦」
3. 將 `spectyn-mesh-ios.ipa` 拖曳到裝置圖示上
4. 等待約 30 秒完成安裝
5. 在 iPhone 上：設定 → 一般 → VPN 與裝置管理 → 信任開發者描述檔（你的 Apple ID）

### 選項 B：Sideloadly（第三方，免費）

如果你不想用 Configurator，[Sideloadly](https://sideloadly.io) 透過更友善的介面完成相同的工作。拖放 IPA、在安裝時以你的 Apple ID 簽署，按下 Start。

### 選項 C：Xcode Devices 視窗

適合已開啟專案工作區的開發者。Window → Devices and Simulators → 將 IPA 拖入「Installed Apps」窗格。

---

## 首次啟動

點按 **Spectyn Mesh** 圖示。你會看到一個設定表單（與我們隨附給 Android 的 DOM-API 表單相同 — 已於 0a039ba 修正，使 Connect 按鈕不會在 Tauri 的 CSP 下變成壞像素）：

```
Connect to spectyn serve
Host:  localhost          ← change to coordinator TS IP
Port:  7878
                          [Connect]
```

填入協調者 IP（預設參考部署為 `<mac-tailscale-ip>`），點按 **Connect**。網頁檢視（webview）會導覽至行動聊天介面；之後的啟動會直接進入該介面。

---

## 在 7 天到期前更新

免費憑證簽署的 IPA 會在安裝後剛好 7 天停止啟動。有兩種方式可重新整理它：

### Xcode 一次性設定

在首次重建之前，將 Xcode 登入擁有該開發憑證的 Apple ID：

1. 開啟 Xcode → Settings（⌘,）→ Accounts
2. 點 `+` → Apple ID → 登入
3. 確認該團隊出現在「Manage Certificates…」之下

若沒有這步，`make ios-rebuild` 會以「No profiles for 'ai.spectynmesh.app'」失敗。光有鑰匙圈（keychain）身分（可在 `security find-identity` 中看到）並不足夠 — 自動佈建（automatic provisioning）需要即時的 Apple ID 工作階段，Xcode 才能從 developer.apple.com 取得描述檔。

### 手動單次執行

```bash
# On the coordinator (Mac):
APPLE_TEAM_ID=F7683B69U7 make ios-rebuild
```

（用 `security find-identity -v -p codesigning | grep Apple` 找出你的 team id — 它是括號內那個 10 字元的字串。）

全新的 IPA 會產生於 `dist/spectyn-mesh-ios.ipa`。從一台裝有 Apple Configurator 的機器旁載：

```bash
curl -O http://<coord>:7878/dist/spectyn-mesh-ios.ipa
```

### 每週日自動重建（建議）

安裝一個每週執行 `make ios-rebuild` 的單一使用者 LaunchAgent：

```bash
APPLE_TEAM_ID=F7683B69U7 ./scripts/install-ios-rebuild-agent.sh
```

排程：每週日 03:30。記錄檔位於 `~/Library/Logs/spectyn-ios-rebuild.log`。

注意事項：
- Mac 在排定時間必須是喚醒狀態（或設定 `pmset` 為 launchd 工作喚醒）
- 工作執行時登入鑰匙圈必須是已解鎖的 — 否則 codesign 無法讀取憑證的私密金鑰
- Apple 的自動佈建需要網路存取，以及 Xcode 偏好設定中已登入的 Apple ID

強制手動執行一次（適合首次驗證）：

```bash
launchctl kickstart -k gui/$(id -u)/ai.phantommesh.ios-rebuild
tail -f ~/Library/Logs/spectyn-ios-rebuild.log
```

解除安裝：

```bash
launchctl bootout gui/$(id -u)/ai.phantommesh.ios-rebuild
rm ~/Library/LaunchAgents/ai.phantommesh.ios-rebuild.plist
```

---

## 疑難排解

### 「Untrusted Developer」— 應用程式無法啟動

設定 → 一般 → VPN 與裝置管理 → 在 DEVELOPER APP 下點按該 Apple ID → 信任。

### Connect 按鈕沒有反應

如果你在早於 0a039ba（2026 年 4 月 28 日）的建置版本上看到這個情況，內嵌的事件處理常式正被 Tauri 的預設 CSP 封鎖。拉取最新的 IPA — 該修正以正規的 DOM API + addEventListener 取代了 document.write + onclick。

### 按下 Connect 後設定表單一直重新出現

網頁檢視的 localStorage 寫入成功了，但在設定 → 已載入協調者的轉換過程中未能持久保存。0a039ba 的修正也涵蓋了這點 — 它直接導覽，而非依賴重新載入來重讀 localStorage。

### 「App could not be installed at this time」

IPA 內的佈建描述檔不包含此裝置的 UUID。請擇一處理：
- 在這部 iPhone 連接的情況下，至少用 Xcode 開啟 IPA 專案一次 — Xcode 會自動將該裝置的 UDID 加入開發描述檔
- 或使用付費的 Apple Developer 帳號搭配明確的佈建描述檔

### 應用程式中無法連到 Spectyn serve

與 Android 相同的診斷 — 請見 TROUBLESHOOTING-MAC.md 的「healthz unreachable」段落。最常見的原因：iPhone 上 Tailscale 未連線（設定 → Tailscale → Connect）。

---

## iOS 接下來的計畫（roadmap，路線圖）

- **應用程式內協調者切換器** — 無需重新安裝即可更換主機
- **推播通知**，於長時間執行的任務完成時通知（`@tauri-apps/plugin-notification` 已在 package.json 中）
- **本地 LLM 推論**，透過 Apple Foundation Models 框架（macOS 26+／iOS 26+）— spectyn 會經由一個小型 Swift 墊片（shim）與裝置上的 LLM 溝通。與 Mac 端規劃中的 MLX 整合是相同概念。
- **真正本地的代理迴圈** — 撰寫一個 Swift 原生重新實作的代理迴圈，使其在 iOS 沙盒內執行，並搭配沙盒允許的任何工具（在應用程式容器內的 file_read／file_write、HTTPS fetch、無 shell）。這是一個為期數週的專案，且**不在**展示路徑（demo path）上。

在可預見的未來，Mac／Linux／Windows 協調者會完成所有實際工作，而 iOS 應用程式純粹是假裝在做事的使用者介面。
