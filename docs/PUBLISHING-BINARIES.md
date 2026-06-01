# 發布 phantom binary 到 phantommesh.io 的流程

> **Status**: Working runbook — Windows side proven (2026-05-03 deploy).
> Mac/Linux sections are templated from the same pattern; adjust per-OS specifics.

整套機制 = **Cloudflare R2 (binary 儲存) + Worker 動態 install.sh/install.ps1 + 你的 phantommesh.io 自定義網域**。一行 `iwr | iex` (Windows) 或 `curl ... | sh` (Mac/Linux) 就能裝好。

---

## 架構圖

```
                ┌───────────────────────────────┐
                │  R2 bucket: phantom-binaries  │
                │  ├─ phantom-windows-x86_64.exe│
                │  ├─ phantom-darwin-arm64       │  ← 新加這行
                │  ├─ phantom-darwin-x86_64      │
                │  └─ phantom-linux-x86_64       │
                └────────────┬──────────────────┘
                             │
                             │ R2 binding (BINARIES)
                             ▼
        ┌──────────────────────────────────────────┐
        │  Cloudflare Worker (phantommesh.io)      │
        │  ├─ GET /install.ps1   → render PS 腳本   │
        │  ├─ GET /install.sh    → render bash 腳本 │  ← 新加
        │  └─ GET /dist/<name>   → R2 stream + cache│
        └────────────┬─────────────────────────────┘
                     │
                     │ HTTPS
                     ▼
              ┌────────────────────┐
              │   user terminal    │
              │   curl / iwr → sh  │
              └────────────────────┘
```

關鍵設計：**Worker 程式碼很小，binary 在 R2**。Workers 程式有 1MB script 大小限制，binary 24MB 塞不進去；R2 binding 讓 Worker 當 streaming proxy。

---

## 一次性 setup（已完成的部分，留作 reference）

### 1. 建 R2 bucket
```bash
cd phantommesh-io/
npx wrangler r2 bucket create phantom-binaries
```

### 2. wrangler.toml 加 binding
```toml
[[r2_buckets]]
binding     = "BINARIES"
bucket_name = "phantom-binaries"
```

### 3. types.ts 加 type
```typescript
export type Env = {
  // ...
  BINARIES: R2Bucket;
};
```

### 4. routes/dist.ts 設定 BIN_OBJECTS manifest
```typescript
const BIN_OBJECTS: Record<string, { object: string; contentType: string }> = {
  "phantom-windows-x86_64.exe": {
    object: "phantom-windows-x86_64.exe",
    contentType: "application/octet-stream",
  },
  // ← Mac entry 加在這
};
```

### 5. index.ts 路由
```typescript
app.get("/install.ps1",  installScript);    // Windows
app.get("/install.sh",   installShellScript); // Mac/Linux 加這行
app.get("/dist/:name",   distHandler);
```

---

## 發布新 binary 的標準流程（每次更新）

每次改了 phantom 程式碼想推到所有用戶機器，4 步：

### Step 1：build
```bash
# Windows binary（在 Windows + Z13 / Windows VM）
cd D:/Projects/phantom-mesh/.worktrees/deploy-tui/core
cargo build --release --bin phantom

# 或 cross-compile from any platform 用 cross:
cross build --release --target x86_64-pc-windows-msvc --bin phantom
```

對應產出：
| Platform | 路徑 | Filename in R2 |
|---|---|---|
| Windows x86_64 | `target/release/phantom.exe` | `phantom-windows-x86_64.exe` |
| macOS aarch64 (M1+) | `target/aarch64-apple-darwin/release/phantom` | `phantom-darwin-arm64` |
| macOS x86_64 (Intel) | `target/x86_64-apple-darwin/release/phantom` | `phantom-darwin-x86_64` |
| Linux x86_64 | `target/x86_64-unknown-linux-gnu/release/phantom` | `phantom-linux-x86_64` |

### Step 2：上傳 R2
```bash
cd phantommesh-io/

# Windows
npx wrangler r2 object put phantom-binaries/phantom-windows-x86_64.exe \
  --file "/path/to/phantom.exe" \
  --content-type application/octet-stream

# macOS arm64
npx wrangler r2 object put phantom-binaries/phantom-darwin-arm64 \
  --file "/path/to/phantom" \
  --content-type application/octet-stream
```

### Step 3：驗證 R2 + 邊緣 cache
```bash
# 檢查 R2 物件大小
curl -sSI https://phantommesh.io/dist/phantom-windows-x86_64.exe | grep -iE "Content-Length|ETag"
# Content-Length: 24257024
# ETag: "abc123..."

# 如果 ETag 跟你新上傳的一致，且 Content-Length 是新值就 OK
```

⚠️ **Cloudflare CDN 有 1 小時 cache (`Cache-Control: public, max-age=3600`)。** 緊急 deploy 要清 cache：
- Dashboard → Caching → Purge Everything（最暴力）
- 或 wait 1 hr
- 或改 dist.ts 加 cache-busting query param

### Step 4：使用者端裝
**Windows:**
```powershell
iwr -useb https://phantommesh.io/install.ps1 | iex
```

**macOS / Linux:**
```bash
curl -fsSL https://phantommesh.io/install.sh | sh
```

---

## 加新 platform 的步驟（例：Mac 新增）

### 1. 在 R2 manifest 加 entry
編 `phantommesh-io/src/routes/dist.ts`：
```typescript
const BIN_OBJECTS: Record<string, { object: string; contentType: string }> = {
  "phantom-windows-x86_64.exe": {
    object: "phantom-windows-x86_64.exe",
    contentType: "application/octet-stream",
  },
  "phantom-darwin-arm64": {
    object: "phantom-darwin-arm64",
    contentType: "application/octet-stream",
  },
  "phantom-darwin-x86_64": {
    object: "phantom-darwin-x86_64",
    contentType: "application/octet-stream",
  },
};
```

### 2. 加 install shell script renderer
同檔案加：
```typescript
export function installShellScript(c: Context<{ Bindings: Env }>) {
  const host = c.req.header("Host") ?? "phantommesh.io";
  const scheme = c.env.APP_URL.startsWith("https") ? "https" : "http";
  const baseUrl = `${scheme}://${host}`;
  const script = renderInstallSh(baseUrl);
  return new Response(script, {
    headers: {
      "Content-Type":  "text/plain; charset=utf-8",
      "Cache-Control": "public, max-age=300",
    },
  });
}

function renderInstallSh(baseUrl: string): string {
  return `#!/bin/sh
# phantom mesh — Unix installer
# Run via:
#   curl -fsSL ${baseUrl}/install.sh | sh
set -e

# 1. Detect platform
case "$(uname -s)" in
  Darwin) os=darwin ;;
  Linux)  os=linux  ;;
  *)      echo "unsupported OS: $(uname -s)"; exit 1 ;;
esac
case "$(uname -m)" in
  arm64|aarch64) arch=arm64 ;;
  x86_64|amd64)  arch=x86_64 ;;
  *) echo "unsupported arch: $(uname -m)"; exit 1 ;;
esac
asset="phantom-\${os}-\${arch}"
url="${baseUrl}/dist/\${asset}"

# 2. Locate install dir (preferred order: ~/.local/bin -> /usr/local/bin)
install_dir="\${HOME}/.local/bin"
mkdir -p "\$install_dir"

# 3. Download
echo "[1/4] downloading \$asset..."
curl -fsSL "\$url" -o "\$install_dir/phantom"
chmod +x "\$install_dir/phantom"
echo "  -> \$install_dir/phantom"

# 4. Ensure PATH (add to shell rc if missing)
case ":\$PATH:" in
  *":\$install_dir:"*) echo "[2/4] PATH already includes \$install_dir" ;;
  *)
    echo "[2/4] appending \$install_dir to PATH in ~/.zshrc + ~/.bashrc"
    for rc in ~/.zshrc ~/.bashrc; do
      [ -f "\$rc" ] && {
        grep -q "\$install_dir" "\$rc" || echo "export PATH=\\"\\\$HOME/.local/bin:\\\$PATH\\"" >> "\$rc"
      }
    done
    export PATH="\$install_dir:\$PATH"
    ;;
esac

# 5. Verify
echo "[3/4] verifying..."
"\$install_dir/phantom" --version

# 6. Auto-login + cluster join
if [ "\${PHANTOM_INSTALL_SKIP_LOGIN:-0}" = "1" ]; then
  echo "PHANTOM_INSTALL_SKIP_LOGIN=1 set -- skipping login."
else
  echo "[4/4] running phantom login..."
  "\$install_dir/phantom" login
fi

# 7. macOS: install launchd plist for auto-start (optional)
if [ "\$os" = "darwin" ] && [ "\${PHANTOM_INSTALL_NO_LAUNCHD:-0}" != "1" ]; then
  plist="\$HOME/Library/LaunchAgents/ai.phantommesh.serve.plist"
  cat > "\$plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>ai.phantommesh.serve</string>
  <key>ProgramArguments</key>
  <array>
    <string>\$install_dir/phantom</string>
    <string>serve</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>\$HOME/.phantom-mesh/serve.out.log</string>
  <key>StandardErrorPath</key><string>\$HOME/.phantom-mesh/serve.err.log</string>
</dict>
</plist>
PLIST
  launchctl unload "\$plist" 2>/dev/null || true
  launchctl load "\$plist"
  echo "launchd: ai.phantommesh.serve loaded (logs: ~/.phantom-mesh/serve.{out,err}.log)"
fi

echo ""
echo "=== installed ==="
echo "Try it:  phantom"
`;
}
```

### 3. index.ts 註冊路由
```typescript
import { distHandler, installScript, installShellScript } from "./routes/dist";

// ...
app.get("/install.ps1",  installScript);
app.get("/install.sh",   installShellScript);
app.get("/dist/:name",   distHandler);
```

### 4. Deploy
```bash
cd phantommesh-io/
npx wrangler deploy
```

### 5. Build + 上傳 Mac binary
```bash
# 在 M-series Mac 上
cd phantom-mesh/core
cargo build --release --bin phantom

# 上傳
cd ../../phantommesh-io
npx wrangler r2 object put phantom-binaries/phantom-darwin-arm64 \
  --file "../core/target/release/phantom" \
  --content-type application/octet-stream
```

### 6. 在 Mac 測試
```bash
curl -fsSL https://phantommesh.io/install.sh | sh
# 或 dry-run:
curl -fsSL https://phantommesh.io/install.sh | less
```

---

## Platform-specific 差異對照

| 元素 | Windows | macOS | Linux |
|---|---|---|---|
| Install one-liner | `iwr ... \| iex` | `curl ... \| sh` | `curl ... \| sh` |
| Install dir | `%USERPROFILE%\.local\bin` | `~/.local/bin` | `~/.local/bin` |
| PATH 設法 | `[Environment]::SetEnvironmentVariable('Path', ..., 'User')` | append to `~/.zshrc` / `~/.bashrc` | 同 Mac |
| Auto-start | Scheduled Task (`schtasks`) | launchd (`launchctl load`) | systemd user unit |
| Stop process | `Stop-Process -Name phantom -Force` | `pkill -f phantom` | `pkill -f phantom` |
| Tailscale CLI | `tailscale.exe ip -4` (PATH 通常有) | `tailscale ip -4` | 同 Mac |
| Hostname 偵測 | `hostname` (PowerShell built-in) | `hostname` | `hostname` |
| Admin / sudo | 需要 admin 開啟視窗才能建 schtask | 不需 sudo（launchd user agent） | 一般不需 sudo |

---

## 故障排除

### `curl ... | sh` 卡在 PATH 設定
- 檢查 `~/.zshrc` 是否有 `export PATH="$HOME/.local/bin:$PATH"` 那行
- 開新 terminal session 才會生效（或 `source ~/.zshrc`）

### 401 invalid_api_key 
- 跑 `phantom debug | pbcopy`（macOS）/ `xclip`（Linux）→ 貼到 chat 給我看
- 通常是 vault 裡的 key 過期 / 被 rotate

### launchd plist 跑不起來
- 看 `~/.phantom-mesh/serve.err.log`
- `launchctl list | grep phantom` 看 Status code
- 改 plist 後要 `launchctl unload && launchctl load`，光 reload 不夠

### R2 cache 沒清
- Dashboard → Caching → Configuration → Purge Cache → Purge by URL: `https://phantommesh.io/dist/<filename>`
- 或 1 小時自動過期
- 永久解：dist.ts 加 ETag check + 短 cache（要寫 conditional GET 邏輯）

---

## 安全注意事項

- `/install.sh` 完全 unauthenticated，**任何人都能下載** binary。Binary 本身沒 secret，是 OK 的
- 但 binary `iex / sh` 等於完全控制使用者機器。**必須走 HTTPS**（Cloudflare 強制）
- 簽署 binary（codesigning）這層**還沒做**：
  - macOS：未簽名 → Gatekeeper 會擋，user 要 `xattr -d com.apple.quarantine ./phantom` 或在 System Settings → Privacy 點 Allow
  - Windows：未簽名 → SmartScreen 警告，user 要 More info → Run anyway
  - 長期應該買 codesigning cert（Apple Developer $99/yr，Sectigo for Windows $200+/yr）
- Worker 程式碼開源（在這個 repo），密碼學上算法都公開，唯一私密的是 wrangler secret 裡的 master keys

---

## 完整新版發布 SOP（Quick reference）

```bash
# 1. 改 phantom code，commit
cd phantom-mesh/.worktrees/deploy-tui
# ... edit files ...
git add . && git commit -m "feat: ..."
git push

# 2. Build per-platform
cargo build --release --bin phantom              # 當前平台
cargo build --release --target aarch64-apple-darwin --bin phantom    # cross to Mac arm

# 3. Upload to R2
cd ../../phantommesh-io
for asset in phantom-windows-x86_64.exe phantom-darwin-arm64 phantom-linux-x86_64; do
  npx wrangler r2 object put phantom-binaries/$asset \
    --file "../core/target/<corresponding-path>" \
    --content-type application/octet-stream
done

# 4. (可選) Purge CDN cache
# Cloudflare dashboard → Caching → Purge

# 5. Verify
curl -sSI https://phantommesh.io/dist/phantom-darwin-arm64 | grep ETag
```

User 跑 `iwr | iex` / `curl | sh` 就拿到新版 binary，無感升級。
