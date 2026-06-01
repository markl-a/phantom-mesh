# 安全覆寫設定（環境變數）

Phantom-mesh 預設即安全（secure-by-default）——每個叢集（cluster）RPC 端點都需要
HMAC（雜湊訊息驗證碼），每個跨來源（cross-origin）請求都會被拒絕，每個工作區（workspace）工具都會拒絕
工作區以外的路徑。有一小組環境變數的存在*僅僅*是為了簡化
從較早（較不嚴格）版本的遷移（migration），而且每一個都會在 `phantom serve` 啟動時印出一行醒目的
`SECURITY WARNING:`，讓操作者（operator）絕不會在不知情下
讓某個節點（node）停留在舊版的寬鬆模式（legacy-permissive mode）。

本頁列出所有由 `serve::emit_boot_security_warnings_with_config` 揭露的覆寫設定。
如果你正在閱讀本頁是因為看到一行 `SECURITY WARNING:`，請在
下方表格中以環境變數名稱搜尋，並依照遷移建議（migration ask）操作。

## 覆寫設定

| 環境變數 | 預設值 | 用途 | 遷移建議 |
| --- | --- | --- | --- |
| `PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET` | 未設定 | 還原 T7（#69）之前的行為，亦即 `/api/chat`、`/rpc/message`、`/rpc/task/assign`、`/mcp`、`/ws`、`/api/onboarding`、`/onboarding/*`，以及常駐服務（daemon）的 `/agent/:name/run[-async]` 接受未經驗證（unauthenticated）的請求。若未設定此變數，且 `agents.toml` 中也沒有 `[cluster].cluster_secret`，則每個叢集 RPC 都會回傳 `403 Forbidden`。 | 在 `agents.toml` 中設定 `[cluster].cluster_secret = "..."`（32 位元組以上的隨機位元組，採 base64 編碼），然後 `unset PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET`。此覆寫設定將於**下一個小版本（minor release）中移除**。 |
| `PHANTOM_CORS_ALLOW_ANY` | 未設定 | 在儀表板（dashboard）／API 上還原 T7c 之前的寬鬆 CORS（跨來源資源共用，`Access-Control-Allow-Origin: *`）。預設不會送出 `Access-Control-Allow-Origin` 標頭，因此瀏覽器會拒絕跨來源的 XHR/fetch——同來源（same-origin，即內嵌的儀表板）不受影響。 | 將所有跨來源用戶端移到同一來源，或以反向代理（reverse proxy）作為前端中介。然後 `unset PHANTOM_CORS_ALLOW_ANY`。 |
| `PHANTOM_FETCH_ALLOW_LOCAL` | 未設定 | 允許 `tools::web_fetch` 與 `tools::http_client` 連向迴路位址（loopback，`127.0.0.1`、`::1`）、RFC1918 私有位址範圍（`10/8`、`172.16/12`、`192.168/16`），以及鏈路本地位址（link-local，`169.254/16`——包含位於 `169.254.169.254` 的 AWS／GCP 中繼資料服務）。預設為**封鎖**並回傳 `ERROR: blocked`，以緩解代理（agent）提示詞中的 SSRF（伺服器端請求偽造）攻擊。 | 在任何遠端代理提示詞可觸及的節點上保持未設定。僅在開發者筆電上除錯本機服務時設定；切勿在正式環境（production）使用。 |
| `PHANTOM_EXTRA_ALLOWED_ROOTS` | 未設定 | 以逗號分隔的額外絕對路徑清單，`tools::file::safe_path` 會將這些路徑視為位於工作區內，作為 `cwd` 與 `~/.phantom-mesh` 之外的補充。供測試框架（test harness）與將工作區掛載在非標準路徑的 CI 執行器（runner）使用。 | 建議從工作區根目錄執行 `phantom`，讓 `cwd` 涵蓋你需要的檔案。僅在一次性指令稿（one-off scripts）時使用此變數。 |
| `PHANTOM_AUTO_APPROVE` | 未設定 | 在執行破壞性的 shell／fs 工具呼叫前，略過互動式的 `[y/N]` 提示。適用於無人值守的 CI；在可由網路觸及的節點上**切勿**與 `PHANTOM_FETCH_ALLOW_LOCAL=1` 併用。 | 互動式工作階段（session）請保持未設定。 |

## 開機時的可見性

`phantom serve` 會在綁定其 TCP 監聽器（listener）**之前**呼叫
`emit_boot_security_warnings_with_config(...)`。你會在 stderr 上看到三種結果之一：

1. **兩個覆寫都已設定**——兩行 `SECURITY WARNING:`，沒有 fail-closed（失敗即封鎖）那一行。
2. **設定了一個覆寫**——一行 `SECURITY WARNING:`。
3. **未設定任何覆寫，且 `cluster_secret` 為空**——一行資訊性訊息：
   ```
   phantom serve: cluster_secret not configured and \
     PHANTOM_ALLOW_EMPTY_CLUSTER_SECRET unset — \
     deployment is failing-closed (cluster RPC endpoints will return \
     403 until [cluster].cluster_secret is set in agents.toml).
   ```
4. **未設定任何覆寫，且 `cluster_secret` 已設定**——靜默（這是安全的
   預設狀態；沒有什麼值得說的）。

同樣的警告也會在 `phantom-mesh broker`（中介伺服器）端觸發（T7c）；該變體
請參閱 broker 的 README。

## 為什麼這些是環境變數（而非設定檔欄位）

遷移覆寫被刻意設計得難以設定。若把它們放進
`agents.toml`，會讓一個舊的設定檔在升級後悄悄地重新啟用不安全的
預設值。環境變數必須在每個 shell 或
systemd 單元中重新提供，這會強迫操作者做出刻意的動作，並會在 `ps`／
日誌（journal）輸出中顯示，便於事件回應（incident response）。

當某個版本**移除**一個覆寫時，現有操作者會立即看到該關卡
回傳 `403`/`ERROR:`——不會有悄無聲息的行為變更。
