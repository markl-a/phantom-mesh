# iOS Shortcuts Fetch Agent

## 概念
iPhone/iPad 無法常駐 HTTP server，但可以用 **Shortcuts Automation** 接收 webhook。

## 設定方式

### 1. 在 iPhone 建立 Shortcut
名稱：`PhantomFetch`

Steps:
1. **Receive input from** Quick Actions / Shortcuts app
2. **URL** = Ask Each Time（或從 input 取得）
3. **Get contents of URL** (curl equivalent)
4. **Return** contents as text

### 2. 用 Shortcuts URL Scheme 觸發
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
- 支援 Python, JavaScript, curl
- 可在背景跑輕量 HTTP server

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

### 加入 agents.toml peer（iPhone Tailscale IP）
```toml
"http://100.71.5.50:7878",  # iphone-13-mini
```

## 當前 Tailscale IPs（iOS 設備）
- iPhone 13 mini: 100.71.5.50
- iPad Pro 12.9: 100.77.117.80
