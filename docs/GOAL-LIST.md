# GOAL-LIST — Mac + iOS 全場景 / 全功能 / 協同覆蓋 → 全測試通過

> 由 skill `phantom-mac-coverage`(統一開發操作法)驅動。把 BIG-GOAL 往下拆成可驗收、
> 可追蹤完成度的 goal,三軸覆蓋,硬 exit-code gate,沒過修到過。
>
> **生成依據(真實掃描,非臆測):**
> - 場景軸 = `mac-coverage-sweep` workflow(2026-05-31,8 agent,1.2M tok)→ `docs/test-cases/COVERAGE-MAP-mac.md` + 18 bug
> - 功能軸 = `feature-matrix-sweep` workflow(進行中 `wtqqop9sa`)→ 待補
> - anchor = `docs/superpowers/BIG-GOAL.md`(2026-05-19 鎖定)

---

## BIG-GOAL anchor(不可變)

> 跑在你所有裝置上的私人 AI 隊伍,看得到你的生活與程式,越用越懂你,陪你進步也替你做事 —— 資料加密,只你能讀。

- **4 支柱**:P1 跨裝置 Mesh · P2 多模態 · P3 進化網 · P4 加密為先(每條 goal 須服務 ≥1)
- **2 軌道**:Life Track(v0.6.0 領頭:食/專注/習慣/教練)· Work Track(派工/演化)
- **3 原則**:無羞辱 · 同意把關擷取 · 可逆
- **平台真相**:Mac + iOS **共用單一 Rust `core/` crate**;iOS 殼在主 repo `app/`(Tauri mobile + `app/src-tauri/gen/apple` Xcode),**非獨立 repo**。`GitHub/hailmary/phantom-mesh-ios/` 是空殼可忽略。

---

## 三軸 × 三池

| 軸 | 是什麼 | ground truth 來源 |
|---|---|---|
| **軸1 場景** | 使用者全場景(CUJ-01~05 + PLAT + P4)| `mac.md` 136 case;iOS 待產 `ios.md` |
| **軸2 功能** | 全功能/特性(45 SPEC × code×test)| `feature-matrix-sweep`(待補)|
| **軸3 協同** | Mac↔iOS 互通(SPEC-10~17)| sweep §3 + matrix interop rollup |

| 池 | 定義 | 能否「修到過」 |
|---|---|---|
| **Pool A** | 本機可 hermetic 自動化 + 硬 exit-code gate | ✅ 立刻能做能驗 |
| **Pool B** | 需實機 / 網路 / 2-device / GUI | ⚠ 需設備,留 manual playbook |
| **Pool C** | 需先建文件(產 ios.md、補 SPEC↔CUJ↔case 對映)| 📝 前置 |

**驗收條件(每條 leaf goal 通用)**:`cd core && cargo test --test <name> > out 2>&1; echo RC=$?` → RC=0 + 出示 `test result:` 行;高風險再過對抗 review(codex+subagent,≤4 round)。

---

## 軸1 — 場景覆蓋

### Mac(`mac.md` 136 case;掃描統計 ~40 covered / ~27 partial / ~53 可自動化未做 / ~30 需實機)

> ⚠ 「covered」含多筆只有 lib-level、CLI/integ 未驗 → 真 E2E 覆蓋低於 29%。**~39% (53 條) 今天就可 hermetic 自動化但無人守** = 真 backlog。

**Pool A(可立即 TDD,對應 sweep top-10 gaps)**:見下方 `EVOLVE-GOALS.md` G1–G10。

**Pool B(需設備)**:INST-001 live `curl|sh`、INST-010 no-network、SYN-003/005 2-device sync SLO、PERM-001 TCC、SIGN-002 Gatekeeper dialog、FH-007 TUI(=Bug A)、FOOD-004 real LLM、COA-005 launchd 定時。

### iOS(無 `ios.md` — Pool C 前置)

- **G-IOS-0(前置)**:對照 `mac.md` + SPEC-30/31/32 產出 `docs/test-cases/ios.md`(iOS 全場景 case)。完成前 iOS 場景軸零 ground truth。

---

## 軸2 — 功能覆蓋(45 SPEC)— feature-matrix-sweep 結果(2026-05-31)

> 完整逐功能表見 `docs/FEATURE-MATRIX.md`(8-agent 唯讀掃描,3M tok)。狀態 = code × test。
> Honesty:`*_wire.rs` 合約檔 + serde round-trip 只算 DONE-untested,**不算** DONE+TESTED。

**支柱 rollup**

| 支柱 | DONE+TESTED | PARTIAL | STUB | MISSING | 結論 |
|---|---|---|---|---|---|
| P1 mesh | 6 | 14 | 4 | 17 | HMAC+dispatch 腦真且測;使用者面/mDNS/peer UI 多缺 |
| P2 多模態 | 3 | 8 | 3 | 9 | 最弱:像素分析只走 daemon CLI、音訊全 STUB、mobile capture UI 全 MISSING |
| P3 evolve | 7 | 9 | 4 | 6 | 功能最完整;但 6 步閉環只 ~3.5(measure/sync/embedding STUB)|
| P4 加密 | 8 | 7 | 2 | 14 | client 加密原語強且測;**at-rest 2/5 OS、broker E2EE 是死碼** |

**軌道**:Life 9 DONE+TESTED(daemon/CLI 真;mobile 半全 MISSING)· Work 6(SPEC-26 dispatch 腦最佳測但無 user surface)· shared-core 13(最大 MISSING bucket 也在此:foundation codegen/OTEL/signing/ship-gate)。

**平台(關鍵)**:Mac ~5 真行為測(唯一)· **iOS 0**(`phantom-mesh-ios` 只剩 Vite cache、**無原生 code**;真 iOS 全在主 repo `app/` Tauri;6/7 Swift bridge cmd 未測)· Android shell 在 `app/src-tauri/android/kotlin/`。

**對「全功能覆蓋」的硬意涵(改變完善定義):**
1. **「全功能」≠ 45 SPEC 全做。** Foundation(01-09 ~90% 設計文件:tokens/routes/error-catalog/a11y/OTEL codegen 全 MISSING)/ 平台原生 / server 多是「藍圖」非「待測功能」。真要測的核心 = **Protocol(10-17)+ System(20-29)+ 共用 app/**。
2. **in-scope 分母先定義。** v0.6.0 ship = Protocol + System-Life。foundation codegen/OTEL/原生平台標 v0.7.0+/設計階段,**排除出「測試通過 100%」分母**,否則不可能達標。
3. **三個系統性過度宣稱(matrix Tier-1,直接打臉 BIG-GOAL):**
   - **P4「加密只你能讀」在 4/5 OS 失效** — macOS/iOS Keychain、Win DPAPI、Android KeyStore 全 `unimplemented!()`;`identity.key` 在 Mac/iOS/Linux 是明文;iOS provider key 寫進明文 `~/.phantom-mesh/env`。
   - **P4 broker E2EE 是死碼** — 出貨 broker 伺服器端解密;「zero-knowledge sync」目前是假的。
   - **P2「圖片/音訊輸入」大半不真** — 多模態 wire 送檔名非像素;音訊全 STUB;mobile capture UI 全 MISSING。

**軸2 goal:**
- [ ] G-FEAT-1 定義 v0.6.0 in-scope 功能集(Protocol 10-17 + System 20-29 + app/),foundation codegen/OTEL/原生平台標 v0.7.0+/設計階段(排除出測試分母)
- [ ] G-FEAT-2 對 in-scope 的 DONE×(NONE/WEAK) 補真行為測試 — 真覆蓋率缺口
- [ ] G-FEAT-3 `FEATURE-MATRIX.md` 已落地;每補一測回寫狀態
- [ ] G-FEAT-4 [誠實性] P4 at-rest Keychain + broker E2EE 接線(目前打臉 BIG-GOAL,開源前必處理或明確降級宣稱)

---

## 軸3 — Mac↔iOS 協同(SPEC-10~17)

> matrix §6 interop 結論:**wire/演算法層(10/12/13/16/17)是全專案最可信區**,Mac 側真測;
> 但**每一條的 Mac↔iOS 裝置橋接都零執行測試**,且 mDNS / broker-E2EE / vault-handoff **連 shipping code 都沒有**。
> 一句話:協同底層「紙上 + Mac 上」是真的,**「Mac↔iOS」那半是未建的**。

| SPEC | interop | real test? | reality |
|---|---|---|---|
| 10 mesh-rpc | HMAC 兩端一致 | ✅ shared-core | protocol 測過;iOS transport 只 `swift_cluster_fetch` 未測 |
| 11 mdns | Mac 廣播↔iOS 瀏覽 | ❌ | Mac pipeline 零生產呼叫 + live test ignore;iOS NWBrowser MISSING |
| 12 identity | 跨裝置驗證 | ✅ algo / ❌ iOS persist | core 測;iOS Keychain 持久化 MISSING |
| 13 encryption | Mac 封 iOS 解 | ✅ algo | round-trip 測;無跨裝置 handoff 測;iOS EventKey 落磁碟非 Keychain |
| 15 broker-vault | Mac 封→broker→iOS 解 | ❌ real path | crypto 測過但**死碼**;live broker 伺服器解密;handoff MISSING |
| 16 event-storage | 共用 sqlite/FTS5 | ✅ shared-core | round-trip 測;EventKind 4-vs-8 分歧未測 |
| 17 tauri-bridge | invoke + deep-link | ✅ Rust / ❌ iOS | core 測;iOS cold-launch pull `deeplink_consume_pending` MISSING |

已知 sweep bug:`mesh.rs:879` 探測 `/healthz` 但 doc 說 `/info`(discovery 漂移)。

---

## Bug backlog(sweep 18 條,完整見 COVERAGE-MAP / sweep 輸出)

**最高槓桿(Pool A,先修)**:
1. 🔴 **slug 驗證**`capture_habit_wire.rs:320` — `create_habit` 從不驗 slug,`InvalidSlug` 死碼,壞 slug 寫進加密 event。**confirmed live**。→ G1
2. 🔴 **假綠 badge**:DB-001 復原(#143 badge 無測)→ G2;P4-003 identity 不外洩(claimed 無測)→ G3
3. 🔴 **測試名漂移=假綠**:`mac.md` 指 `seal_unseal_roundtrip`/`invalid_slug`/`focus_duration_ms` 等不存在的 fn → cargo 匹配 0 個靜默通過。**審計全部 cmd/test 名**。
4. 🟠 corrupt-identity 政策不一致(habit/food/coach 不 fail-loud)`encryption_wire.rs:601`
5. 🟠 export 非原子寫 `bin/phantom.rs:4462`;broker JWT 未 zeroize;`--include-broker` 半抹除

---

## 執行順序(主軸:沒過修到過)

1. **Pool A 佇列**(`EVOLVE-GOALS.md` G1–G10)逐條 TDD:寫測試(紅)→ 修/實作 → 硬 gate(RC=0)→ commit。
2. G1(slug bug)優先 —— confirmed live + 是假綠的解藥示範。
3. Pool C 前置(ios.md)+ 功能軸 matrix 回來後 enrich 軸2。
4. Pool B 累積成 manual playbook,等設備。

> 進度回寫:每條完成 → 更新 `mac.md` 狀態欄 + `COVERAGE-MAP` + 本檔。push 經使用者同意。

---

## 軸4 — E2E 全生命週期測試(Appium 三介面,外加在現有測試之上)

> **不取代**現有 cargo 測試(L1),而是**外加**兩層真實使用者操作的 E2E。三層全部硬 gate。
> 本機環境已就緒(2026-05-31 盤點):Appium 3.4.2 · node v22 · Xcode 26.4.1 + simctl ·
> 6 個 iOS simulator(含 `phantom-iphone15-ios17`)· phantom binary `~/.cargo/bin/phantom` 0.6.0-rc.1。
> maestro 未裝(有 Appium 即可)。

### 三層測試金字塔

| 層 | 介面 | 工具 | 跑什麼 | 硬 gate |
|---|---|---|---|---|
| **L1 邏輯** | core crate | `cargo test`(from `core/`)| 單元 + hermetic 整合(現有 + Pool A 新增)| `cargo test --test <name>` RC=0 + 出示 `test result:` |
| **L2 CLI E2E** | terminal | Appium + 真 `phantom` binary | 整條 CUJ 用真 binary 跑(install→habit→export…),斷言 exit code + stdout + sqlite/檔案落地 | spawn binary exit code + 斷言通過 |
| **L3 GUI/iOS E2E** | app / iOS simulator | Appium(XCUITest driver)+ simctl boot | 使用者點擊全生命週期(onboarding→capture→review),逐步截圖比對 | Appium session pass + 截圖證據 |

### Pool 對映
- **L2(terminal)= Pool A 升級版**:Pool A 已是 hermetic CLI 可驗;L2 把它們串成**完整 CUJ 流程**真 binary E2E。
- **L3(iOS sim)= Pool B 變 Pool A**:本機有 simulator → iOS GUI E2E **不再需要實機**,可進 hermetic 自動化(這是 ios.md 場景軸的執行載具)。

### E2E goal(加進佇列)
- [ ] G-E2E-1 [前置] 建 `e2e/` harness:`scripts/e2e-cli.sh`(terminal driver,spawn 真 binary 跑 CUJ-01→05)+ `e2e/appium/`(iOS XCUITest config,boot `phantom-iphone15-ios17` + install .app + driver)。先跑通 1 條 smoke 證明可行。
- [ ] G-E2E-2 CUJ-01 全流程 L2:install→first habit→streak,真 binary,斷言每步 exit code + sqlite 落地。
- [ ] G-E2E-3 CUJ-02 全流程 L3:iOS sim 點擊 onboarding→habit chip→daily review,逐步截圖。
- [x] G-E2E-4 Bug A(TUI render leak)L2/L3:✅ done 2026-05-31。`scripts/e2e/tui-provider-error.sh` 用 tmux 真 PTY 跑真 binary、tmux capture-pane 抓真渲染 frame、斷言無溢位/無 escape leak/邊框完整/error 有出現。60×20 / 100×30 / 200×50 全 PASS → Bug A 未重現（transcript Wrap 守住，疑早版已修）。headless regression guard 另在 commit 29376959。但書：尚未測「上游 error body 帶原始 ANSI escape」這條路徑。

---

## 軸5 — 使用者全流程測試腳本 + log/畫面即時抓取(除錯用)

> 目標:一鍵生成「使用者從頭到尾」的可重跑測試流程,並在跑的同時抓 log + 即時截圖,
> 出問題能立刻定位。產出落在 `docs/manual-playbook/` + `scripts/e2e/`。

### 要生成的東西
- [ ] G-DBG-1 **全生命週期測試腳本**:`scripts/e2e/full-lifecycle-mac.sh` —— 從乾淨 `$HOME` 開始,依序跑
  install → identity init → first habit → daily capture(food/focus/habit)→ coach review →
  export → delete,每步印 `▶ STEP / ✓ PASS / ✗ FAIL` + exit code。對 iOS 版 `full-lifecycle-ios.sh`
  走 simctl boot + Appium 驅動同一條 CUJ。
- [ ] G-DBG-2 **log 抓取**:
  - Mac CLI:`PHANTOM_LOG=debug phantom <cmd> 2>&1 | tee /tmp/phantom-e2e-<ts>.log`;daemon log `~/.phantom-mesh/logs/` + `phantom diag`(crash ring / events)。
  - iOS sim:`xcrun simctl spawn <udid> log stream --predicate 'subsystem CONTAINS "phantommesh"' > /tmp/ios-e2e-<ts>.log &`;app container log 用 `xcrun simctl spawn <udid> log collect` 或拉 `appDataContainer` 的 `phantom-mesh.log`。
- [ ] G-DBG-3 **即時畫面抓取**:
  - iOS sim:`xcrun simctl io <udid> screenshot /tmp/shot-<step>.png`(每 step 一張);錄影 `xcrun simctl io <udid> recordVideo /tmp/run.mp4`。
  - Mac TUI:`screencapture -x /tmp/tui-<step>.png` 或 vt100 文字快照(供 Bug A 回歸)。
  - Appium 內建:每個 step `driver.save_screenshot()`,失敗自動附最後一張。
- [ ] G-DBG-4 **一鍵除錯包**:`scripts/e2e/collect-debug-bundle.sh` —— 把該次 run 的 log + 截圖 + sqlite dump + agents.toml(遮蔽 key)打包成 `/tmp/phantom-debug-<ts>.tar.gz`,方便貼回來除錯。

### 硬 gate
L2/L3 E2E 與這些腳本同樣遵守:**無 exit code / 無截圖證據不算通過**。腳本最後印
`E2E RESULT: PASS|FAIL (N steps, M screenshots, log=<path>)`,FAIL 自動觸發 G-DBG-4 收包。

---

## 環境就緒清單(2026-05-31 盤點,L2/L3 的前提)

| 元件 | 狀態 | 位置/版本 |
|---|---|---|
| Appium | ✅ | `/opt/homebrew/bin/appium` 3.4.2 |
| node / npm | ✅ | v22.14.0 / 10.9.2(nvm)|
| Xcode + simctl | ✅ | Xcode 26.4.1 / `/Applications/Xcode.app/.../simctl` |
| iOS simulator | ✅ | 6 台含 `phantom-iphone15-ios17` |
| phantom binary | ✅ | `~/.cargo/bin/phantom` 0.6.0-rc.1 |
| Appium iOS driver(XCUITest)| ⬜ 待裝 | `appium driver install xcuitest`(G-E2E-1 前置)|
| maestro | ⬜ 未裝(非必須,有 Appium 即可)| — |
