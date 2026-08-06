# Spectyn Mesh 文件索引

[English version](INDEX.md)

本頁是 `docs/` 樹的頂層地圖,依「用途」分組,並區分**現行真相**與**歷史material**,
讓貢獻者不會誤把已被取代的計畫拿去實作。

## 從這裡開始（Start Here）

1. [`OPERATING-STANDARD.md`](OPERATING-STANDARD.md) — 運行唯一 SSOT（HOW + 路線圖 + 治理 + 檔案標準摘要）。已折入原 GOVERNANCE / FLEET-DEV / JOINT-DEV / ROADMAP-VISUAL 四份文件。
2. [`superpowers/BIG-GOAL.md`](superpowers/BIG-GOAL.md) — **鎖定的 apex / 憲法**(2026-06-11 最終重鎖定):4 支柱 P1–P4、2 軌道(Life/Work)、治理金字塔在 §10。
3. [`../AGENTS.md`](../AGENTS.md) — 倉庫規則、邊界、TDD 流程。
4. [`superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md`](superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md) — deep-spec 目錄與實作閱讀順序。
5. [`../SESSION_RESUME.md`](../SESSION_RESUME.md) — 最新戰術交接與下一個具體步驟。

> **注意:** [`_archive/NORTH-STAR.md`](_archive/NORTH-STAR.md) 與 [`_archive/2026-06-19-BIG-GOAL.zh-TW.md`](_archive/2026-06-19-BIG-GOAL.zh-TW.md) 已被 BIG-GOAL.md **取代**(兩者皆已加橫幅)。**不要當成現行方向。**

## 治理與願景（Governance & Vision）

| 文件 | 用途 |
|---|---|
| [`OPERATING-STANDARD.md`](OPERATING-STANDARD.md) | 運行唯一 SSOT — HOW + 路線圖 + 治理(§4 金字塔/導航/真相鏈) + 檔案標準摘要 |
| [`superpowers/GOVERNANCE.md`](superpowers/GOVERNANCE.md) | 🪦 折入 OPERATING-STANDARD.md §4；此路徑為轉址 stub(保留 inbound 連結) |
| [`superpowers/BIG-GOAL.md`](superpowers/BIG-GOAL.md) | v0.6.0+ 週期鎖定的 apex / 憲法(4 支柱、2 軌道) |
| [`superpowers/specs/v060-deep-spec/SPEC-01-FOUNDATION-bigGoal-mapping.md`](superpowers/specs/v060-deep-spec/SPEC-01-FOUNDATION-bigGoal-mapping.md) | 把每根支柱映射成可實作的子能力 |
| [`superpowers/ROADMAP-v0.6.0.md`](superpowers/ROADMAP-v0.6.0.md) | v0.6.0 路線圖 DAG(計分板以 V0.6.0-RELEASE-PLAN.md 為準) |
| [`superpowers/V0.6.0-RELEASE-PLAN.md`](superpowers/V0.6.0-RELEASE-PLAN.md) | 釋出計分板與日期 |
| [`superpowers/V0_7_0_DEFERRAL_INVENTORY.md`](superpowers/V0_7_0_DEFERRAL_INVENTORY.md) | 明確延後到 v0.7.0+ 的項目 |

## 規格（Specs）

### Deep Spec（可實作）

| 文件 | 用途 |
|---|---|
| [`superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md`](superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md) | deep-spec 目錄與閱讀順序(目錄進入點) |
| [`superpowers/specs/v060-deep-spec/`](superpowers/specs/v060-deep-spec/) | 全部可實作的 SPEC-NN(Foundation / Protocol / System / Platform / Server / Testing / Experimental) |
| [`superpowers/SPEC-TO-CODE-PLAYBOOK.md`](superpowers/SPEC-TO-CODE-PLAYBOOK.md) | 分階段的 spec→實作 playbook |

### 現行 Epics（Active Epics）

現行 v0.6.0 epic 規格在 [`superpowers/specs/_current/`](superpowers/specs/_current/):

| Epic | 規格 | spec 內記錄的狀態 |
|---|---|---|
| E001 | [`跨主機叢集冒煙`](superpowers/specs/_current/E001-cross-host-cluster-smoke.md) | 維護中 |
| E002 | [`多模態擷取流程`](superpowers/specs/_current/E002-multimodal-capture-pipeline.md) | 已交付 |
| E003 | [`Coach 節點與每日回顧`](superpowers/specs/_current/E003-coach-node-daily-review.md) | 未開始 |
| E004 | [`加密儲存層`](superpowers/specs/_current/E004-encrypted-storage-layer.md) | 已交付 |
| E005 | [`技能萃取`](superpowers/specs/_current/E005-hermes-skill-extraction.md) | 未開始 |
| E006 | [`30 秒 Life hello`](superpowers/specs/_current/E006-30-second-hello-world.md) | 未開始 |
| E007 | [`v0.6.0 釋出準備`](superpowers/specs/_current/E007-v060-release-prep.md) | 已接受 |

另在 `_current/`（行為 spec，非 epic）：[`linux-cli-spec.md`](superpowers/specs/_current/linux-cli-spec.md) —— code-grounded 的 Linux `spectyn` CLI 行為參考（2026-06-19 由 `docs/cli/` 移入）。

2026-05-19 的 pivot spec 是 [`superpowers/specs/2026-05-19-life-node-pivot.md`](superpowers/specs/2026-05-19-life-node-pivot.md)。

## 功能（Features）

Feature 規格(F001+)在 [`superpowers/features/`](superpowers/features/);每檔檔頭宣告 `Parent epic` 與 `Pillar(s) served`。

## 操作手冊（Runbooks）

| 文件 | 用途 |
|---|---|
| [`GETTING-STARTED.md`](GETTING-STARTED.md) / [`QUICKSTART.md`](QUICKSTART.md) | 第一步 |
| `INSTALL-{WINDOWS,MAC,LINUX,ANDROID,IOS,OCI}.md` | 各平台安裝 |
| [`mesh/FLEET-SSH.md`](mesh/FLEET-SSH.md) / [`mesh/MESH-FLEET-ONBOARDING.md`](mesh/MESH-FLEET-ONBOARDING.md) / [`mesh/TAILSCALE-SETUP.md`](mesh/TAILSCALE-SETUP.md) | 艦隊 / mesh 網路 |
| [`deploy/DEPLOYMENT.md`](deploy/DEPLOYMENT.md) / [`deploy/DEPLOY-AUTOUPDATE.md`](deploy/DEPLOY-AUTOUPDATE.md) / [`deploy/DEPLOY-MAC-STAGING.md`](deploy/DEPLOY-MAC-STAGING.md) | 部署（`DEPLOY-AUTOUPDATE` = 簽章 + release CI + OTA） |
| [`deploy/PUBLISHING-BINARIES.md`](deploy/PUBLISHING-BINARIES.md) / [`mobile/SMOKE-ANDROID.md`](mobile/SMOKE-ANDROID.md) / [`SELFTEST.md`](SELFTEST.md) / [`DIAGNOSTICS.md`](DIAGNOSTICS.md) | 釋出 / 冒煙 / 診斷 |
| [`../scripts/spectyn-test/README.md`](../scripts/spectyn-test/README.md) | 黑箱 CLI / HTTP-RPC / round-trip 測試框架 |
| [`../tests-e2e/README.md`](../tests-e2e/README.md) | 人工協助 Tier-1 E2E 情境 |

## 主題子目錄（Topical Subdirectories）

`docs/` 根目錄已於 2026-06-19 重整 —— 多數主題文件移入分組子目錄,依主題瀏覽:

| 子目錄 | 內容 |
|---|---|
| [`install/`](install/) | 各平台安裝(Windows/Mac/Linux/Android/iOS/OCI)、二進位驗證、Apple 登入 + 認證供應商設定 |
| [`deploy/`](deploy/) | 部署、自動更新/OTA、Mac staging、發佈二進位、mcp-registry 提交、簽章 Android 釋出 |
| [`providers/`](providers/) | LLM 供應商/認證設計(`DESIGN-PROVIDER-AUTH`、`AUTH-DESIGN`)、MLX 供應商、免費 LLM 供應商調查 |
| [`experimental/`](experimental/) | 技能庫(curator/memory/extra-providers/tools)+ 遠端控制實驗筆記 |
| [`mesh/`](mesh/) | 叢集協作/擴展、艦隊上線、FLEET-SSH、Tailscale、多裝置協調、多代理分析/QA、行動 vs 桌面 |
| [`mobile/`](mobile/) | iOS 測試流程、行動 web 模式、e2e(mac-real / native-webdriver)、Android 冒煙 |
| [`design/`](design/) | 子系統/設計文件(cross-tool、spectynmesh-io、platform-impl、anti-hallucination、commercial、swarm-architecture、dispatch-followups) |
| [`commercial/`](commercial/) | 開源計畫、貢獻者漏斗、作品集規格凍結(戰略 SSOT 文件留在根目錄) |
| [`dev/`](dev/) | 開發加速框架 + mesh、開發流程、自主開發迴圈、Claude Code 設定、自主治理、anthropic streaming |
| [`dev-notes/`](dev-notes/) | 現行開發筆記(2026-06-11):error-handling、Windows login-LLM 驗證、backlog/inbox/status |
| [`cuj/`](cuj/) | 5 條關鍵使用者旅程(安裝→首個習慣、每日擷取、跨裝置接續、降級狀態、匯出/解除安裝)—— 見 [`cuj/README.md`](cuj/README.md) |
| [`test-cases/`](test-cases/) | 各 surface 測試 case DB(mac/win-cli/linux-cli/mac-app/win-app/android/ios)+ COVERAGE-MAP + 共用 schema —— 見 [`test-cases/README.md`](test-cases/README.md) |
| [`skills/`](skills/) | 技能文件庫(YAML frontmatter Markdown)供 curator/router 使用 —— 見 [`skills/README.md`](skills/README.md) |
| [`ai-reviews/`](ai-reviews/) | 跨 AI 審查產物(adversarial-reader 逐 spec 輸出、wave12、agy/codex/gemini 審查)—— 由 `scripts/ai/output` 移入 |
| [`ecosystem-roadmap/`](ecosystem-roadmap/) | 9 個衛星專案最終路線圖 + main P0 分解 —— 見 [`ecosystem-roadmap/ECOSYSTEM-ROADMAP-FINAL.md`](ecosystem-roadmap/ECOSYSTEM-ROADMAP-FINAL.md) |
| [`plain/`](plain/) | 白話(非技術)說明集 —— 這工具是什麼 / 我怎麼用 / 真實情境 —— 見 [`plain/00-索引.md`](plain/00-索引.md) |
| [`_archive/`](_archive/) | 被取代的計畫/規格(NORTH-STAR、MASTER-SPEC、EXECUTION-PLAN…)—— 僅供歷史,非權威 |

### `superpowers/` 內

| 子目錄 | 內容 |
|---|---|
| [`superpowers/plans/`](superpowers/plans/) | 有日期的實作計畫(計畫唯一的家;epic/spec/feature 落地計畫) |
| [`superpowers/runbooks/`](superpowers/runbooks/) | 出貨閘 / 操作員 runbook(E007-release-smoke、E001-testbed-setup、distributed-dev、mobile-cluster-dispatch-ui-smoke…) |
| [`superpowers/specs/2026-06-12-platform-flows-design/`](superpowers/specs/2026-06-12-platform-flows-design/) | 16 檔 surface×ability×reality 設計參考層(非 SPEC 權威;結論須下沉成 SPEC leaf —— Charter §A.2 規則 6) |

## 架構與設計（參考）

> ⚠️ 標 *pre-pivot* 者早於 2026-05-19 Life-Node pivot,描述已實作機制但**非**現行產品範圍的權威 —— 治理見 [`superpowers/GOVERNANCE.md`](superpowers/GOVERNANCE.md)。

| 文件 | 用途 |
|---|---|
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | 已實作 daemon 架構(**pre-pivot 參考**) |
| [`architecture/`](architecture/) | 元件設計(agent runtime、Tauri 前端、加密儲存、selftest…) |
| [`adr/`](adr/) | 架構決策紀錄(ADR-001+) |
| [`superpowers/design/`](superpowers/design/) | TUI / CLI 畫面設計 |
| [`superpowers/ARCH-EXECUTION-ENTITIES.md`](superpowers/ARCH-EXECUTION-ENTITIES.md) | 執行實體架構(A/B/C stacks) |
| [`providers/AUTH-DESIGN.md`](providers/AUTH-DESIGN.md) / [`design/SWARM-ARCHITECTURE.md`](design/SWARM-ARCHITECTURE.md) / [`design/SPECTYNMESH-IO-DESIGN.md`](design/SPECTYNMESH-IO-DESIGN.md) | 子系統設計 |

## 商業與戰略（Commercial & Strategy）

> 在 apex 下游(從屬),不塑造產品。見 BIG-GOAL §7。

| 文件 | 用途 |
|---|---|
| [`COMMERCIALIZATION-STRATEGY.md`](COMMERCIALIZATION-STRATEGY.md) | 副業尺度(從屬於 apex) |
| [`STRATEGY-DIFFERENTIATION.md`](STRATEGY-DIFFERENTIATION.md) | 執行層 sequencing(從屬於 apex) |
| [`positioning.md`](positioning.md) | 對外定位(**pre-pivot**) |
| [`design/COMMERCIAL-DESIGN.md`](design/COMMERCIAL-DESIGN.md) / [`commercial/OPEN-SOURCE-PLAN.md`](commercial/OPEN-SOURCE-PLAN.md) | 商業 / OSS 規劃 |

## 封存（Archive）

有日期的快照、報告、pre-Rust 化石放在 [`_archive/`](_archive/) 留作歷史。**不要當現行權威。**
見 [`_archive/README.md`](_archive/README.md)。pre-pivot 規格的封存在 [`superpowers/specs/_archived/`](superpowers/specs/_archived/)。

## 快速決策指南（Quick Decision Guide）

| 問題 | 看 |
|---|---|
| 文件樹怎麼治理?什麼放哪? | [`superpowers/GOVERNANCE.md`](superpowers/GOVERNANCE.md) |
| 我們在建什麼產品? | [`superpowers/BIG-GOAL.md`](superpowers/BIG-GOAL.md) |
| 哪份 spec 管我的實作? | [`superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md`](superpowers/specs/v060-deep-spec/SPEC-00-INDEX.md) |
| 怎麼跑黑箱驗證? | [`../scripts/spectyn-test/README.md`](../scripts/spectyn-test/README.md) |
| 最新戰術狀態? | [`../SESSION_RESUME.md`](../SESSION_RESUME.md) |
