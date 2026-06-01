# iOS Shortcuts 擷取代理（iOS Shortcuts Fetch Agent）

## 概念
iPhone/iPad 無法常駐 HTTP server（HTTP 伺服器），但可以用 **Shortcuts Automation（捷徑自動化）** 接收 webhook（網路鉤子）。

## 設定方式

### 1. 在 iPhone 建立 Shortcut（捷徑）
名稱：`PhantomFetch`

步驟（Steps）：
1. **Receive input from**（接收輸入來源）Quick Actions / Shortcuts app
2. **URL** = Ask Each Time（每次詢問；或從 input 取得）
3. **Get contents of URL**（取得 URL 內容，等同 curl）
4. **Return**（回傳）內容為文字

### 2. 用 Shortcuts URL Scheme（捷徑 URL 協定）觸發
```
shortcuts://run-shortcut?name=PhantomFetch&input=https://example.com
```

### 3. phantom 呼叫方式（從 Mac/Windows）
```bash
# 透過 Tailscale 呼叫 iPhone 上的 Shortcuts
# 需要 iPhone 安裝 shortcuts-server 之類的 app
# 或用 SSH over Tailscale + iSH
```

## 更實際的方案：a-Shell / iSH

### a-Shell（推薦）
- App Store 免費
- 支援 Python、JavaScript、curl
- 可在背景跑輕量 HTTP server（HTTP 伺服器）

```bash
# 在 a-Shell 內：
pip install flask
python3 -c "
from flask import Flask, request, jsonify
import subprocess
app = Flask(__name__)
@app.route('/fetch', methods=['POST'])
def fetch():
    url = request.json['url']
    result = subprocess.run(['curl', '-s', '-L', '--max-time', '10', url], capture_output=True, text=True)
    return jsonify({'content': result.stdout[:10000]})
app.run(host='0.0.0.0', port=7878)
"
```

### 加入 agents.toml peer（對等節點，iPhone Tailscale IP）
```toml
"http://100.64.0.14:7878",  # iphone (example tailnet IP)
```

## 當前 Tailscale IPs（iOS 設備）
- iPhone: 100.64.0.14
- iPad: 100.64.0.15
