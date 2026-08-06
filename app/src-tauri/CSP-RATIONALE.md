# CSP 設計理由（tauri.conf.json `security.csp`）

> 為什麼放在這裡而不是寫成 JSON 註解：自 v0.6.x 工具鏈升級後，tauri-build
> 會拒絕 `tauri.conf.json` 中的未知欄位（unknown field），所以先前的
> `"_comment_csp"` 變通做法（workaround）會讓桌面版建置失敗。文件因此改放在
> 這個附屬檔（sidecar）中。

## F-CRIT-1 / V8 C-1：收窄 `connect-src`

CSP（內容安全政策，Content Security Policy）的 `connect-src` 指令從 Tauri
預設值（`http: https: ws: wss:`——也就是任意位址）收窄為一份硬編碼的
允許清單（allowlist），如此一來，渲染層（renderer）中的 XSS（跨站腳本攻擊）
小工具（gadget）就無法把 `broker_token` 或 vault（保險庫）金鑰外洩
（exfiltrate）到任意主機。

## 允許的目標

| 目標 | 原因 |
|---|---|
| `'self'` | 同源（same-origin）`/api/*` + Tauri IPC（行程間通訊）。 |
| `ipc:` + `http://ipc.localhost` | Tauri 2 IPC 橋接。 |
| `http://localhost:*` + `http://127.0.0.1:*` | 在動態埠（dynamic port）上的行程內（in-process）`spectyn` daemon（常駐服務）。 |
| `http://*:7878` | Tailscale／區域網路（LAN）叢集對等節點（cluster peers），使用預設的 `spectyn serve` 埠——供 `clusterDispatch` + 行動裝置上線（onboarding）使用。 |
| `https://api.telegram.org` | `StepNetwork` 中的 Telegram bot-token 驗證。 |
| `https://phantommesh.io` + `https://*.phantommesh.io` | Broker（中介伺服器）——縱深防禦（defense in depth）；正式環境的 broker 呼叫已經改走 Rust 命令。 |
| `https://*.ts.net` | Tailscale MagicDNS 主機名稱。 |

## 自架者（self-hoster）須知

- 自訂 broker 網域 → 把該主機加入 `connect-src`（位於 `tauri.conf.json` 的 CSP 字串中）。
- 叢集對等節點使用非 7878 埠 → 把 `http://*:7878` 換成實際的埠，或把該 dispatch（派工）移進 Rust 命令（完全不會碰到 CSP）。

## V8 HIGH-3：明確設定 `assetProtocol`（停用，已預先界定範圍）

`security.assetProtocol` 先前並未出現在 `tauri.conf.json` 中，這表示 Tauri 2
使用它的安全預設值（`enable: false`、`scope: []`）。實務上這沒問題，但屬於
隱含（implicit）行為——下一個要新增截圖／縮圖功能的人，可能會輕易地把它
翻成 `enable: true`，進而透過 `convertFileSrc()` / `asset://` 不小心把整個
檔案系統暴露給渲染層（由於 CSP 的 `img-src` 已經允許
`asset:` / `http://asset.localhost`，這會成為一條由 XSS 餵入的
路徑穿越（path-traversal）外洩管道）。

我們現在把它設成**明確且停用**，並把範圍（scope）預先塑形（pre-shape）為
目前唯一規劃中的使用者（`browser_screenshot`，今天還是 `v0.2` 階段的
樁程式（stub）——見 `app/src-tauri/src/commands/browser.rs` 與
`app/src/components/browser/ScreenshotView.tsx`）。

### 決策

- `enable: false`——執行期不會解析任何 `asset://` URL。`ScreenshotView.tsx`
  中那唯一一處 `convertFileSrc()` 呼叫點今天是死碼（dead code）
  （`browser_screenshot` 回傳 `Err`），所以這不會破壞任何東西。
- `scope.allow`：只有 `$HOME/.spectyn-mesh/screenshots/**`。這是
  進行中的瀏覽器工具將要寫入的路徑（依據原始的
  `2026-04-10-phase1b-browser-viewer.md` 計畫）。預先把它列入允許清單，意味著
  v0.6.x 的開啟動作只是單行變更（`enable: true`），而路徑
  暴露面（surface）已經最小化。
- `scope.deny`（優先於 allow）：硬性封鎖（hard-block）那些 COULD（有可能）
  在未來 allow 樣式被放寬時跑到 `~/.spectyn-mesh/` 底下的敏感應用程式內
  檔案——`auth.json`（broker token）、`env`（供應商金鑰）、
  `agents.toml`（供應商 URL），外加該目錄樹底下任何 `**/.env` 或 `**/auth.json`。
- `requireLiteralLeadingDot: false`——`~/.spectyn-mesh/` 本身以一個
  點開頭；若無此設定，`$HOME/.spectyn-mesh/...` 這個 glob 在 Unix 上不會匹配。
  Windows 會忽略此欄位。

### 哪些東西「不」需要 asset://

| 資產 | 為何不需要 |
|---|---|
| 隨附的 `icons/*.png` | 透過 `'self'` 載入（Tauri 將資源放在 `tauri://localhost/` 下提供）。 |
| `data:` PNG / SVG（小型 UI 元件） | CSP `img-src data:` 已涵蓋。 |
| 遠端截圖／縮圖 | CSP `img-src https:` 已涵蓋 HTTPS 來源。 |
| `scripts/detect_hardware.ps1`（隨附資源） | 在 Rust 中透過 `app.path()` 讀取，絕不經由渲染層。 |

### 重新啟用檢查清單（供 v0.6.x 瀏覽器截圖工作使用）

1. 落地真正的 `browser_screenshot` 命令（寫入
   `~/.spectyn-mesh/screenshots/<ts>.png`）。
2. 把 `enable` 翻成 `true`。
3. 確認 `ScreenshotView` 能正常算繪（render），且 devtools 主控台沒有
   `Failed to load resource`。
4. 若日後新增其他目錄的縮圖，請**明確地**把它們加入 `scope.allow`
   （不要 glob 展開成 `$HOME/**`）。

## 參考資料

- `docs/superpowers/audits/2026-05-16-tauri-audit.md` §C-1
- PR #112（最初的 F-CRIT-1 落地）
- PR #169（V8-HIGH-1+2——確立了本節所延伸的「明確優於隱含」模式）
- Tauri 2 schema：<https://schema.tauri.app/config/2> `AssetProtocolConfig`、
  `FsScope`
