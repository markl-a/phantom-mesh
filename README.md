# Phantom Mesh

<p align="center">
  <img src="site/logo.png" alt="Phantom Mesh" width="300">
</p>

<p align="center">
  <strong>你的電腦、手機和雲端主機，現在是你的 AI 下屬。</strong>
</p>

<p align="center">
  <a href="https://phantommesh.io">Website</a> ·
  <a href="#what-is-phantom-mesh">About</a> ·
  <a href="#roadmap">Roadmap</a> ·
  <a href="LICENSE">MIT License</a>
</p>

> ⚠️ **Early access — expect bugs.** 目前功能尚不穩定，可能會遇到各種 bug。
> 遇到問題請[開 issue](https://github.com/markl-a/phantom-mesh/issues/new) 或直接聯絡我，我會儘快修。

---


## Try it now (early access binary, May 2026)

The full source release is still being prepared (see [Roadmap](#roadmap)
below). In the meantime the **phantom CLI binary** is downloadable so
you can try the agent runtime + cluster + TUI yourself.

```powershell
# Windows (PowerShell)
iwr -useb https://phantommesh.io/install.ps1 | iex
```

```bash
# macOS 
curl -fsSL https://phantommesh.io/install.sh | sh
```

What the installer does (each script is in [`installers/`](installers/) — read before piping):

1. Downloads `phantom` binary to `~/.local/bin/`
2. Adds it to `PATH`
3. Runs `phantom login` (Google OAuth via phantommesh.io broker — used to vault your LLM API keys so they sync between machines)
4. Auto-registers this device + joins it to your cluster
5. Starts `phantom serve` in the background

After install:

```bash
phantom              # launch the TUI (press Tab to switch agents, /help for slash commands)
phantom cluster status   # see which mesh peers are alive
phantom dispatch --to <peer> "your prompt"    # run a task on a specific machine
phantom dispatch --all "your prompt"          # broadcast to every peer
```

**What's open today**: install scripts in [`installers/`](installers/) — what gets piped to `iex`/`sh`. Read them before running.

**What isn't open yet**: the phantom binary itself. The full Rust source release is on track for May 2026 per the section below — until then the binary is closed, but the install path is auditable so you can decide whether you trust running it.

---

## What is Phantom Mesh?

Phantom Mesh 將你已有的設備 — 電腦、手機、雲端主機 — 串成一個**私人 AI 代理網路**。

每台設備都是一個完整的 AI 節點，能自主執行任務並互相協作。資料留在你自己的設備上，不需要上傳至任何第三方雲端。

### 核心能力

| 能力 | 說明 |
|------|------|
| **P2P 設備網路** | 設備自動發現、組網、任務分配，無需中央伺服器 |
| **AI 代理自主運作** | 理解目標、拆解步驟、跨設備協作、持續執行 |
| **隱私優先** | 預設本地 AI 推理，資料不離開你的設備 |
| **多 LLM 支援** | 11 個 AI 供應商，本地模型與雲端 API 自由切換 |
| **59+ 內建工具** | 瀏覽器自動化、檔案管理、內容生成、資料分析等 |
| **跨平台** | Windows、macOS、Android、iOS、Linux（Rust + Tauri v2） |
| **工作流引擎** | TOML 定義的多階段自動化工作流程 |

### 它能做什麼

- 你睡覺時，雲端主機跑完一整批 SEO 文章
- 你通勤時，手機彙整客戶回覆並草擬回信
- 你開會時，筆電監控競品定價變動
- 你下一個指令，AI 代理們自己分配該誰做、在哪做、什麼時候做

---

## Roadmap

> Single maintainer, dates are best-effort (early access — see warning above).

### Platform availability

| 平台 | 狀態 | 預計 |
|---|---|---|
| Windows x86_64 | 🟢 Available now (early-access binary) | ✓ |
| macOS Apple Silicon | 🟢 Available now (early-access binary) | ✓ |
| **Linux x86_64** | 🟡 Build pipeline exists, binary not yet on R2 | **Week of May 11, 2026** |
| **Android (Tauri thin)** | 🟡 Tauri shell scaffolded, packaging pending | **End of May 2026** |
| **iOS (Tauri thin)** | 🟡 Tauri shell scaffolded, packaging pending | **End of May 2026** |
| macOS Intel x86_64 | 🔴 Not prioritised (Apple Silicon covers most users) | TBD |

### Stability milestones

| Version | Scope | Target |
|---|---|---|
| v0.4 (current) | Early access, expect bugs (see warning) | Now |
| **v0.5 stable** | Win + Mac stable, multi-provider failover battle-tested, install path frictionless | **End of May 2026** |
| **v1.0 stable** | All 5 platforms stable, source release Phase 1+2 (see below), incident response process in place | **Mid June 2026** |

### Source open-sourcing (Phase 1-4)

Today only the install scripts in [`installers/`](installers/) are open. Source release
will be staged as separate Cargo crates so each piece can land + get
reviewed independently:

| Phase | Crate | What | Target |
|---|---|---|---|
| 1 | `phantom-tools` | 23 LLM-callable tools (shell, file_*, search, git_*, http, memory, cluster_*) | **Mid May 2026** |
| 2 | `phantom-runtime` | Multi-provider router, failover, streaming, tool-call protocol | **Late May 2026** |
| 3 | `phantom-cluster` | P2P RPC, HMAC auth, self-update trampoline, admin shell | **Early June 2026** |
| 4 | `phantom` (binary) | TUI + CLI glue (top of crates above) | **Mid June 2026** |

Each phase = independently compileable crate. v1.0 stable lands when
all 4 phases are public + reviewed.

---

## Architecture

phantom 的核心引擎與設備組網層將分階段開源 — see [Roadmap](#roadmap)
above for phase / target dates per crate.

### 技術概覽

```
┌─────────────────────────────────────────────┐
│  Phantom Mesh Node                          │
│                                             │
│  ┌───────────────────────────────────────┐  │
│  │  Presentation Layer                   │  │
│  │  WebView UI · Voice · Dashboard       │  │
│  └──────────────────┬────────────────────┘  │
│                     │                       │
│  ┌──────────────────▼────────────────────┐  │
│  │  AI Agent Runtime                     │  │
│  │  Multi-turn · Tool calling · 11 LLMs  │  │
│  └──────────────────┬────────────────────┘  │
│                     │                       │
│  ┌──────────────────▼────────────────────┐  │
│  │  Capability Layer                     │  │
│  │  59+ tools · Browser · Shell · Files  │  │
│  └──────────────────┬────────────────────┘  │
│                     │                       │
│  ┌──────────────────▼────────────────────┐  │
│  │  Cluster Layer (P2P)                  │  │
│  │  Discovery · Election · Task dispatch │  │
│  └───────────────────────────────────────┘  │
└─────────────────────────────────────────────┘
```

**Built with Rust.** 單一二進位檔部署，記憶體效率是 Python 方案的 10 倍。

---

## 🌐 Ecosystem（已公開的衛星專案）

Phantom Mesh 核心 runtime 將於 5 月開源。在那之前，圍繞它建構的應用層
專案已經陸續開源 — 它們共用同一套 agent runtime 思想（這個 repo 描述的
那套），各自應用在不同領域。

| 專案 | 角色 | 狀態 |
|---|---|---|
| **[Automation_with_Agent](https://github.com/markl-a/Automation_with_Agent)** | 應用自動化 + AIOps + MLOps：17+ production tools、multi-provider LLM、RAG、agents、workflows | 🟢 Public |
| **[Data-Analysis-with-Agents](https://github.com/markl-a/Data-Analysis-with-Agents)** | 資料科學分析層：clustering（K-Means / DBSCAN / GMM / Hierarchical）、CLV、RFM、agent 遙測分析 | 🟢 Public |
| **[My-AI-Learning-Notes](https://github.com/markl-a/My-AI-Learning-Notes)** | 繁中 AI 工程師學習路徑 + 面試準備教材（17 ⭐） | 🟢 Public |
| **[GarageSwarm](https://github.com/markl-a/GarageSwarm)** | Phantom Mesh 的 Python 前傳 — 已被 Rust 重寫取代，保留作設計史料（含 [LESSONS.md](https://github.com/markl-a/GarageSwarm/blob/main/LESSONS.md)） | 📖 Predecessor |
| **[phantom-secops](https://github.com/markl-a/phantom-secops)** | 多 agent 紅藍隊資安模擬研究（基於 Juice Shop / DVWA isolated lab） | 🟢 Public |
| **[phantom-mobile](https://github.com/markl-a/phantom-mobile)** | Android agentic E2E 測試框架（含 simulation engine：弱網 / 低電 / locale / a11y / 生命週期） | 🟢 Public |

每個 satellite 都有自己的 README，並在開頭附上 phantom-mesh ecosystem
hook，互相連回主 repo。

---

## Stay Updated

開源發佈時會在此 repo 公告。Star this repo 以獲得通知。

---

## License

[MIT](LICENSE)
