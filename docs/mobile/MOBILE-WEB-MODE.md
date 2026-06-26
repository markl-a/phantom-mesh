# 行動裝置 Web 模式（Tauri 薄殼）

Phantom Mesh 出貨一份完整的 React 桌面 UI（使用者介面）（`app/src/`），以及一份獨立的
輕量級 web 前端（`core/web/`，由 `phantom serve` 在連接埠（port）
**7878** 上提供服務）。在 Android／iOS 上，Tauri 薄殼（thin-shell）會跳過桌面 UI，
改為載入遠端 web 前端，因此手機與桌面在瀏覽器中
看到的是同一個介面（surface）。

## 運作方式

`app/src/main.tsx` 在掛載（mount）React *之前*呼叫
`app/src/mobileThinShell.ts` 中的 `maybeRedirectToRemoteFrontend()`。這個輔助函式（helper）：

1. 偵測在 Android／iOS 上執行的 Tauri WebView。
2. 首次啟動時（尚未儲存任何主機），於頁面內渲染一個小型表單，詢問
   主機（host）+ 連接埠。
3. 後續啟動時，會以 `window.location.replace` 跳轉至
   `http://<host>:<port>/`，也就是 `phantom serve` 對外公開的 URL。

桌面版會立即短路（沒有 Tauri-mobile UA（使用者代理字串）→ 回傳
`false`），React 應用程式如往常般掛載。不需要任何 Rust、Tauri 設定，
也不需要任何建置系統（build-system）的變更。

## 工作流程

1. 在你的 Mac／PC 上，啟動伺服器：
   ```bash
   phantom serve            # listens on 0.0.0.0:7878
   ```
2. 將你的手機連上同一個 Tailnet（推薦）或 LAN（區域網路）。
3. 在手機上啟動 Phantom Mesh 的 Tauri 應用程式。
4. 首次啟動：輸入你的 Tailscale 主機名稱（例如 `mac-mini`）或 LAN
   IP，然後點按 **Connect**。設定會持久保存在 `localStorage`。
5. 後續啟動會自動載入儀表板（dashboard）。

## 重設／更換主機

開啟應用程式，然後在 WebView 主控台（console）中（或透過桌面版 Safari／
Chrome 的遠端除錯）執行：

```js
localStorage.removeItem('PHANTOM_HOST');
location.reload();
```

可選的鍵（key）：`PHANTOM_PORT`（預設 `7878`）、`PHANTOM_SCHEME`
（預設 `http`；若你在伺服器前端架設 TLS（傳輸層安全協定）
代理（proxy），請設為 `https`）。
