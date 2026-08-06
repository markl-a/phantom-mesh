# 自我迭代：spectyn 修改自己的程式碼

`spectyn evolve` 是自主開發迴圈（autonomous development loop，自動運行的開發循環）。給定一個目標後，代理（agent）會
讀取相關檔案、做出最小幅度的編輯、執行 `cargo check` /
`cargo test` 來驗證，然後回報結果。

## 首次成功的自我修復（2026-04-27）

**目標（Goal）**：
> 修正 core/src/cost.rs 中「method persist is never used」（persist 方法從未被使用）的警告。先用 file_read
> 讀取檔案以了解上下文，再用 file_edit
> 把函式名稱加上底線前綴，或為其標註
> `#[allow(dead_code)]`。請「不要」刪除該函式。編輯完成後，透過 shell 執行
> cargo_check 以確認警告已消失。請勿提交（commit）。

**執行**（單一命令，約 60 秒，成本 $0）：

```bash
spectyn evolve "Fix the 'method persist is never used' warning in core/src/cost.rs ..." \
  --max-rounds 4 --agent coder
```

**spectyn 實際做了什麼**（一個回合，三次工具呼叫）：

```
── Round 1/4 ───────────────────────────────────────
  ⟳ file_read   {"path":"core/src/cost.rs"}
  ✓ file_read   use std::collections::HashMap; …
  ⟳ file_edit   {"new_string":"    #[allow(dead_code)]\n    fn persist(&self, inner: &Co…
  ✓ file_edit   Edited /Users/<you>/.../core/src/cost.rs successfully
  ⟳ shell       {"command":"cargo check","cwd":"core","timeout_secs":300}
  ✓ shell       STDERR: Checking hyper-rustls v0.27.9 …  Finished `dev` profile
```

**產生的差異（diff）**（代理實際套用的變更）：

```diff
@@ impl CostTracker {
+    #[allow(dead_code)]
     fn persist(&self, inner: &CostTrackerInner) {
         if let Some(parent) = self.path.parent() {
             let _ = std::fs::create_dir_all(parent);
```

**驗證**（執行結束後手動進行）：

```bash
cd core && cargo build --release --bin spectyn 2>&1 | grep warning
# → no output (0 warnings — was 1 before)
```

## 為了讓這件事能運作而完成的接線設定

1. **無需付費 Claude 金鑰，供應商鏈（provider chain，供應商串接）也能運作。** `coder` 代理以
   `groq`（`llama-3.3-70b-versatile`）作為主要供應商。同一份設定中的 `opencode` 供應商
   會回退（fall through）至 `minimax-m2.5-free`（opencode.ai zen 閘道上的免費方案）。當被
   給定明確的 `tools = [...]` 區塊時，兩者都能可靠地發出 OpenAI 格式的 `tool_calls`。
2. **每個代理的工具清單是必填的。** 若沒有 `tools = [...]`，代理
   執行時（runtime）會向 LLM（大型語言模型）送出零個工具定義，模型便會幻覺（hallucinate）出
   工具呼叫，而非真正去呼叫它們。`~/.spectyn-mesh/agents.toml` 中的 `coder` 代理
   列出了約 25 個必要工具（file_*、
   content_search、glob_search、shell、git_*、cargo_check、cargo_test 等）。
3. **`max_tokens` 從 256 提高到 4096。** 推理型（reasoning-style）模型（minimax、
   nemotron）會在思考階段就耗盡預算，導致在較小上限下無法
   發出內容。4096 為兩者都留下了空間。
4. **REPL 中的串流（streaming）與可見的工具呼叫。** 每個回合在開始時印出
   `⟳ tool_name(args)`、在完成時印出 `✓ tool_name preview`，因此
   迴圈在執行時是可觀測的。
5. **`/show <n>` 顯示完整輸出。** 工具結果在即時顯示中會被截斷為 5 行；
   `/show 1`（以此類推）會傾印出完整的輸出。

## 分散式自我演化（尚未驗證）

`spectyn evolve --distributed` 會把目標拆分成子任務，並
派發給叢集（cluster）中的對等節點（peers）（node-a / node-b / laptop）。其接線
已存在；驗證仍在驗證階梯（validation ladder）上待辦。

## 成本

在免費的 Groq 方案上為 $0（在 opencode `*-free` 模型上也是 $0）。上述整個
「代理修復自己的警告」迴圈完全不花錢。
