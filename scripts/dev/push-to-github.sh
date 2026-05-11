#!/bin/bash
# ═══════════════════════════════════════════════════════════
# 一鍵推上 GitHub — 在你 gh auth login 之後執行
# ═══════════════════════════════════════════════════════════
set -e

cd "C:\Users\m4932\Desktop\adreanalai\LLM-Cluster-Project\core"

# 1. 建立 GitHub repo (private)
echo "[1/3] 建立 GitHub repo..."
gh repo create core --private --source=. --push 2>/dev/null || {
    echo "Repo 可能已存在，嘗試設定 remote..."
    git remote add origin https://github.com/$(gh api user -q .login)/core.git 2>/dev/null || true
}

# 2. 加所有檔案
echo "[2/3] 加入檔案..."
git add -A
git commit -m "feat: cluster deployment + mobile workers + chatgpt gpt-5.4 backend

- ChatGPT backend: bypass .cmd wrapper, call node directly (Unicode fix)
- Mobile workers: Android (Termux) + iOS (iSH/a-Shell) install scripts
- Port assignments: M1=7879, AYANEO=7880, Acer=7881, iPhone=7882, Android=7883-7884
- Hub IP: 192.168.1.104:7878
- All configs updated with hub address
- agency-agents reference analysis (#29)

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"

# 3. 推上去
echo "[3/3] 推送..."
git push -u origin master

echo ""
echo "✅ 完成！其他機器可以用以下指令 clone："
echo "   git clone https://github.com/\$(gh api user -q .login)/core.git"
echo ""
echo "M1 Mac:     cd core && cargo build --release && ./target/release/core worker --hub http://192.168.1.104:7878 --name m1-mac --port 7879"
echo "Android:    bash core/deploy/mobile/android/install-termux.sh"
echo "iOS:        bash core/deploy/mobile/ios/install-ish.sh"
