# Onboarding 手動測試腳本

## 啟動

```bash
cd phantom-mesh-app
pnpm install
cargo tauri dev
```

## 重置 onboarding（如果已完成過）

DevTools Console (F12):
```js
localStorage.removeItem('phantom_mesh_onboarded');
localStorage.removeItem('phantom_mesh_onboarding_state');
location.reload();
```

## 測試流程

### Step 0: Welcome
- [ ] 進度點顯示 6 個圓點（第 1 個亮）
- [ ] 硬體掃描結果顯示（GPU / RAM / Ollama 狀態）
- [ ] 點「下一步」

### Step 1: Security
- [ ] OAuth 登入 or 跳過
- [ ] PIN 設定
- [ ] 點「下一步」

### Step 2: Provider Discovery (新)
- [ ] 標題「Provider 自動偵測」
- [ ] 掃描動畫出現後消失
- [ ] 如果有 Ollama → 顯示在已偵測列表
- [ ] 如果有環境變數 (OPENAI_API_KEY 等) → 顯示對應 provider
- [ ] 每個 provider 有 checkbox 可 toggle
- [ ] 測試一鍵登入按鈕：
  - [ ] GitHub Copilot（需有 ~/.config/github-copilot/hosts.json）
  - [ ] Google Gemini（需有 gcloud ADC）
  - [ ] Claude CLI（需有 ~/.claude.json）
- [ ] 有偵測到 provider 時 →「跳過手動設定 →→」按鈕出現
- [ ] 點「跳過手動設定」→ 跳到 Step 4 (Network)
- [ ] 或點「手動新增 API Key →」→ 到 Step 3

### Step 3: Manual API Key (改版)
- [ ] 標題「手動新增 API Key」
- [ ] 上方顯示已偵測 provider 綠色 badge
- [ ] 雲端 Provider 格子顯示 10 個：
  - OpenAI, Anthropic, Gemini, Groq, DeepSeek, Mistral, xAI, OpenRouter, Codex, OpenCode
- [ ] 勾選 provider → 出現 API key 輸入框 + 驗證按鈕
- [ ] 輸入 key → 點驗證 → 顯示 ✓ 或失敗
- [ ] Azure OpenAI 區塊：
  - [ ] endpoint 輸入框 + API key 輸入框
  - [ ] 驗證按鈕
- [ ] AWS Bedrock 區塊：
  - [ ] Region 下拉選單（5 個 region）
  - [ ] 「檢查 AWS 憑證」按鈕
- [ ] 點「下一步」

### Step 4: Network
- [ ] 叢集設定正常
- [ ] 點「下一步」

### Step 5: Complete (改版)
- [ ] Summary 同時列出 discovered + manual providers
- [ ] 點「啟動 Phantom Mesh」
- [ ] write_config 成功（觀察 DevTools Network/Console）
- [ ] Daemon 啟動成功 → 倒數跳轉

## 快速煙霧測試（最短路徑）

1. Step 0 → 下一步
2. Step 1 → 跳過 OAuth → 設 PIN → 下一步
3. Step 2 → 如果有 Ollama 自動偵測到 → 點「跳過手動設定」
4. Step 4 → 下一步
5. Step 5 → 確認 summary 有 Ollama → 啟動

## 常見問題

**Q: scan_credentials 報錯？**
A: 檢查 DevTools Console，可能是 Tauri command 未正確註冊

**Q: 驗證 API key 失敗？**
A: 確認 daemon 是否已啟動，validate_api_key 需要網路連線

**Q: 跳過按鈕沒出現？**
A: 至少要有一個 enabled 的 discovered provider
