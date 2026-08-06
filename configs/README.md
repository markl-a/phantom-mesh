# Per-Machine Config Templates

> **這個目錄是「每台機器要放哪個 agents.toml」的選擇表。**
> 完整多機協作架構見 [`docs/mesh/MULTI-DEVICE-COORDINATION.md`](../docs/mesh/MULTI-DEVICE-COORDINATION.md)。

## 機器 → 模板對照

| 機器 | 用這個範本 | 拷貝到 |
|---|---|---|
| **node-a** (macOS, Apple primary + leader) | `agents.coordinator.toml` | `~/Library/Application Support/ai.spectynmesh.app/agents.toml` |
| **node-b** (Windows 11 + WSL2 主力) | `agents.coordinator.toml` | `%APPDATA%\ai.spectynmesh.app\agents.toml` |
| **node-c** (Windows, Android 主力 + Win 副手) | `agents.worker.toml` | `%APPDATA%\ai.spectynmesh.app\agents.toml` |
| **node-d** (Windows handheld, 第 3 線) | `agents.worker.toml` | `%APPDATA%\ai.spectynmesh.app\agents.toml` |
| **cloud** (always-on Linux server) | `agents.cloud.toml` | `~/.config/spectyn-mesh/agents.toml` |
| **iPhone 13 mini** | `agents.iphone13mini.toml` | (Tauri iOS app embedded — see SPEC-30) |
| **iPad Pro 2018** | `agents.ipadpro2018.toml` | 同上 |
| **Xiaomi Pad** | `agents.mipad.toml` | 同上 |
| **ROG Phone 6** | `agents.rog6.toml` | 同上 |
| **Raspberry Pi 4** | `agents.raspberrypi.toml` | `~/.config/spectyn-mesh/agents.toml` |

## 共通必填項

每個範本都需要替換：

1. **`cluster_secret`** — 所有 cluster 內節點必須相同。建議用 `openssl rand -hex 32` 生 64-char。
2. **`peers`** — 其他節點的 Tailscale IP 或 LAN IP 列表（不含自己）。
3. **API key 環境變數** — `ANTHROPIC_API_KEY` / `GROQ_API_KEY` etc。**絕不要把 key 寫進 toml**，用 `api_key_env` 指向 shell env var。

## 範本差異速覽

| 範本 | 角色 | 用途 |
|---|---|---|
| **agents.coordinator.toml** | Coordinator | dev primary、跑 leader、寫 brief、有重 Rust toolchain |
| **agents.worker.toml** | Worker | 跑 follower、claim task、不 leader |
| **agents.cloud.toml** | Always-on cloud | 接 Telegram、24/7 接 incoming task |
| **agents.ipadpro2018.toml / .iphone13mini.toml / .mipad.toml / .rog6.toml** | Mobile runtime peer | 不 worker；只跑 user-facing app + 接 push 來的 task |
| **agents.raspberrypi.toml** | Always-on / always-listening | 房間裡的 ambient sensor 節點（語音 / camera）|

## 安全紀律

- ✅ Repo 裡只放 `*.toml` 模板（公開可看）
- ❌ 真實 `agents.toml`（含 cluster_secret）不進 git ── 已在 `.gitignore`
- ✅ API key 用 env var 引用、`.env` 也 gitignored
- ❌ 不把 secret 寫進範本 commit ── 用 `<change-me>` placeholder

## 第一次設定流程

```bash
# 1. 從 configs/ 拷貝合適範本
cp configs/agents.coordinator.toml ~/.config/spectyn-mesh/agents.toml

# 2. 編輯 cluster_secret + peers
$EDITOR ~/.config/spectyn-mesh/agents.toml

# 3. 設 env vars
cp .env.example ~/.config/spectyn-mesh/.env  # or your shell profile
$EDITOR ~/.config/spectyn-mesh/.env

# 4. 啟動 daemon
spectyn serve

# 5. 看 cluster 看到別的 peer 沒
spectyn cluster status
```

## 加入 spectyn-coord dev workflow

如果這台機器要參與分布式開發（node-a / node-b / node-c / node-d 之一）：

```bash
# 一行 bootstrap
curl -sSL https://raw.githubusercontent.com/markl-a/spectyn-mesh/main/scripts/ai/coord/bootstrap-remote.sh | bash -s -- --host <name>
```

詳見 [`docs/mesh/MULTI-DEVICE-COORDINATION.md`](../docs/mesh/MULTI-DEVICE-COORDINATION.md) §4 bootstrap 章節。
