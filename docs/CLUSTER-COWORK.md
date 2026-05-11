# Cluster Cowork — 3 台機器同時攻一個 repo 的 SOP

> **Status**: Phase 1 SOP for the Z13 + ayaneo + acer mesh.
> **Updated**: 2026-05-03

## TL;DR

- 每台機器**自己 worktree、自己 branch、自己 phantom 實例**
- 進度共享靠 `_planning/STATE.md`（用 git push/pull 同步）
- 衝突解決靠標準 git merge，不要自建分散式檔案系統
- 完成 push → 任一台 review → merge to main

---

## Why this pattern (not "all machines edit one shared dir")

**❌ 不要做的事**
- Mount Tailscale Drive 讓三台同時編輯一份檔案 → race condition + 鎖定地獄
- 用 phantom 自製 cross-machine file sync → 重新發明 git，但寫得更差
- 把 conversation history 跨機共享 → 兩台同時改一個 session 的衝突無解

**✅ 該做的事**
- 每台機 isolated worktree，**檔案不互相踩**
- 用 git push/pull 在「task 邊界」同步（已是分散式 VCS 設計就是要做的事）
- 用 markdown 檔（`_planning/STATE.md`）當共享協調看板
- 失敗就 `git worktree remove ... && git worktree add ...` 重來，零副作用

---

## 完整流程

### 1. 一次性 setup（每台機都做一次）

```powershell
# 每台機 clone 同一 repo（如果還沒）
cd $env:USERPROFILE\Projects
git clone https://github.com/<you>/<repo>.git foo
cd foo
```

### 2. 每台機 pin 一個 worktree 給自己

```powershell
# 假設主 repo 在 Z13 上的 ~/Projects/foo
# Z13 自己用主 worktree（main 分支留給 review/merge）

# ayaneo 上：
cd $env:USERPROFILE\Projects\foo
git fetch origin
git worktree add ../foo-api feat/api      # 開個叫 foo-api 的 worktree 對應 feat/api branch

# acer 上：
git worktree add ../foo-tests tests/cov   # 同理
```

### 3. 每台機把 phantom 釘到自己的 worktree

```powershell
# Z13:
phantom workspace set "C:\Users\m4932\Projects\foo" master

# ayaneo:
phantom workspace set "C:\Users\m4932\Projects\foo-api" coder

# acer:
phantom workspace set "C:\Users\user\Projects\foo-tests" tester
```

之後每台機**直接打 `phantom`**，就會：
- 自動 cd 到該機的 worktree
- 自動載入該機 pinned 的 agent（master/coder/tester）
- conversation history 跟著這個 worktree 的 cwd-hash 走，不會跟其他 worktree 混

### 4. 共享狀態檔（在 main worktree 上維護）

在 repo root 建一個 `_planning/STATE.md`，每台機 commit 自己進度時順便更新它：

```markdown
# State of foo project — 2026-05-03

## Active worktrees

| machine | branch        | task                  | status      | blockers |
|---------|---------------|-----------------------|-------------|----------|
| Z13     | main          | code review + merge   | 等 ayaneo   | -        |
| ayaneo  | feat/api      | 補 /users endpoint    | 進行中      | -        |
| acer    | tests/cov     | 寫 auth 模組測試      | 完成 → 等 review | feat/auth 還沒 merge |

## Next checkpoints

- [ ] feat/api → main (when /users + /sessions endpoints land)
- [ ] tests/cov → main (after feat/api merged)

## Decisions log

- 2026-05-03: 統一 API path 用 /api/v1/*（不是 /v1/*）— Z13 + ayaneo 議定
```

每次 phantom agent 完成一個 task：
1. agent 自己 update `_planning/STATE.md` 的對應行
2. `git add _planning/ <changed-files> && git commit && git push`
3. 其他機器下次 `git pull` 拿到最新狀態

### 5. Merge flow

```powershell
# 任何時候在 Z13（主 worktree）上：
cd ~/Projects/foo
git fetch --all
git checkout main
git merge --no-ff feat/api
git push origin main

# Push 出去後 ayaneo / acer 各自 pull：
cd ~/Projects/foo-api
git pull --rebase origin main      # 拿到 main 的最新（含剛 merge 的 feat/api）
```

---

## 用 phantom cluster RPC 呼叫遠端機

如果你想在 Z13 直接叫 ayaneo 跑 build：

```powershell
# Z13 PowerShell
phantom rpc assign --target http://100.107.205.98:7878 --agent coder \
  "在 feat/api branch 上加一個 /api/v1/health endpoint，commit 完 push"
```

ayaneo 的 phantom serve 收到後在 ayaneo 的 worktree 上跑該 task，完成後 push。Z13 next pull 看得到。

> **Note**: 目前 RPC 用法繁瑣（要記 IP）。Phase 2 會加 capability-based dispatch 讓你寫 `phantom dispatch --tag api "..."` 自動 route。

---

## 衝突 + 故障處理

| 狀況 | 解法 |
|---|---|
| 兩台機改同一檔案 push 撞 | 後 push 那台 `git pull --rebase` → 解 conflict → 再 push |
| Worktree 進入奇怪狀態 | `git worktree remove ../foo-api --force && git worktree add ../foo-api feat/api` 重來 |
| Agent 跑出來的 commit 訊息很爛 | 在 [agent.X].instructions 加「commit message 要遵循 conventional commits」 |
| 分支 stuck on conflict 太久 | `git worktree remove ../foo-api && git branch -D feat/api && git push origin --delete feat/api` 重新開 |
| Phantom serve 在某台跑掛 | `phantom debug \| Set-Clipboard` 看 events，重啟 serve |

---

## Per-machine 角色建議

對你的 setup（Z13 桌機 + ayaneo handheld + acer 筆電）：

| 機器 | 角色 | pinned_agent | 適合的工作 |
|---|---|---|---|
| **Z13** | 主開發 + merge 中心 | `master` | 主 worktree、review、merge、整合測試 |
| **ayaneo** | 平行 task runner | `coder` 或 `worker` | feature branch 的實作（你不在電腦前的時候它也能跑）|
| **acer** | 輕度任務 + 測試 | `tester` | 跑測試、寫測試、scrape data、生 doc |

每台 agents.toml 在 `[agent.<name>]` 設不同 model + tools 配對它的角色：

```toml
# Z13 - 主機，需要強模型 + 全 tools
[agent.master]
provider = "opencode"
model    = "minimax-m2.5-free"   # 或之後付費 claude-sonnet
tools    = ["shell","file_read","file_write","file_edit","content_search","glob_search","git_status","git_diff","git_log","git_commit"]
providers = ["opencode:minimax-m2.5-free"]

# ayaneo - long-running coder
[agent.coder]
provider = "opencode"
model    = "minimax-m2.5-free"
tools    = ["shell","file_read","file_write","file_edit","content_search","glob_search","git_status","git_diff","git_commit"]

# acer - tester role
[agent.tester]
provider = "opencode"
model    = "minimax-m2.5-free"
tools    = ["shell","file_read","file_write","content_search","glob_search","git_status","git_diff"]
instructions = "You are a senior test engineer. Write missing tests, run them, fix failures. Don't change non-test code unless required to make tests pass."
```

---

## 實際使用範例：開新 feature

**情境**：你想加一個「使用者頭像上傳」功能。

```powershell
# 1. 在 Z13 開 issue + 在 STATE.md 規劃任務
# 2. 在 Z13 的 main 上開 branch + 推到 origin
cd ~/Projects/foo
git checkout -b feat/avatar
git push -u origin feat/avatar

# 3. 在 ayaneo 上開對應 worktree 接手實作
cd ~/Projects/foo
git fetch
git worktree add ../foo-avatar feat/avatar

# 4. ayaneo 上跑 phantom（已 pin 到 ../foo-avatar）
phantom
# 在 TUI 裡 type: 「實作頭像上傳到 /api/v1/users/me/avatar，用 multipart/form-data，後端存到 R2」
# agent 寫 code → 跑測試 → commit → push

# 5. 在 acer 上接手寫測試
git fetch
git worktree add ../foo-avatar-tests feat/avatar       # 同 branch 不同 worktree（test 用獨立目錄）
phantom    # acer pinned to coder/tester role
# 「為剛剛 push 上來的 avatar 上傳功能寫整合測試」

# 6. 兩台都 push 完，回 Z13
cd ~/Projects/foo
git checkout main
git merge --no-ff feat/avatar
git push
# done
```

---

## 哪些事情這個 SOP **不解決**（之後 Phase 2/3 才處理）

- ✗ Cross-machine 對話接續（你在 ayaneo 開的 phantom session 不能在 acer 繼續）→ Phase 3 (Path C)
- ✗ Capability-based dispatch（你還是要手動指定 `--target http://ayaneo:7878`）→ Phase 2 (Path B)
- ✗ Task DAG / 並行 reduce（適合批次 workload，你目前還用不到）→ Phase 4 (Path E)

---

## 維護指令速查

```powershell
phantom workspace show              # 看當前 pin
phantom workspace set <dir> [agent] # 改 pin
phantom workspace clear             # 取消 pin (回到 caller's cwd)
phantom cluster status              # 看三台 + mac 是否互通
phantom debug | Set-Clipboard       # 故障排除（complete diag bundle）
phantom logs --kind provider --tail 20   # 看最近的 LLM 呼叫
```
