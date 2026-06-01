---
id: ADR-006
date: 2026-05-30
status: accepted
title: Mobile execution model = embedded-core-no-serve (iOS + Android)
context: |
  桌機（desktop）跑 `phantom serve` 常駐 HTTP daemon 在 port 7878 對外服務，但 mobile 不能：iOS 受 App Store 限制不准常駐後台 daemon、退背景幾十秒就被暫停；Android 受 doze 省電休眠限流/殺背景程序。SPEC-30（iOS）與 SPEC-33（Android）原本都沒有「execution model」章節，導致 mobile 該如何跑核心邏輯未鎖定——可能是 (a) 純 WS client 靠桌機運算、(b) embedded core 不起 serve、或 (c) hybrid。不定案則桌機 vs mobile 實作會 drift。
decision: |
  iOS 與 Android App 都採用 (b) Embedded core no-serve：phantom Rust core 直接連進 App 程序（iOS 經 Tauri invoke→Rust core→`swift_cluster_fetch`→Swift、Android 經 Tauri invoke→Rust core→JNI），不 bind TCP listener（桌機那條 `axum::serve(listener, serve::router(…))` 路徑不跑）、不開任何監聽 port。capture / coach engine / 加解密全在 App 程序內 in-process 跑。mobile 加入 cluster 時自表 `serve = false`（不可連入、只主動連出），仍可主動 fan-out task 給桌機 peer（iOS NSURLSession / Android OkHttp，outbound HTTPS）。長背景任務：iOS 靠 Background Mode entitlement、Android 靠 foreground service + notification 規避 doze。寫入 SPEC-30 §6.5 與 SPEC-33 §6.5（對稱，差異在 Swift vs JNI、與 Android foreground service 能力）。
consequences: |
  - 對齊 BIG-GOAL P1「mobile is peer not client」——手機桌機全關 / 飛航模式下仍能本機 capture + coach
  - 桌機 vs mobile 共用同一份 Rust core 邏輯，無分叉的 client-only code path
  - 代價：背景執行受 OS 限制（iOS ~30s 暫停、Android doze），長任務需 entitlement / foreground service
  - 不做 hybrid（前景 embedded + 背景 WS 委派）——測試組合爆炸，留 v0.7.0
related_specs:
  - SPEC-30-PLATFORM-iOS-foundations
  - SPEC-33-PLATFORM-Android-foundations
alternatives_considered:
  - name: (a) WS client only — 所有運算靠桌機，mobile 只是遙控器
    why_rejected: 桌機關機時手機完全失能（連 capture 都不能），違反 BIG-GOAL P1「mobile is peer」
    when_to_revisit: null
  - name: (c) Hybrid — 前景 embedded、背景轉 WS 委派給桌機
    why_rejected: 實作最複雜，前景/背景兩條 code path 測試組合爆炸，v0.6.0 GA 時程內做不出
    when_to_revisit: v0.7.0
supersedes: []
superseded_by: null
---

## Long-form rationale

詳見 SPEC-30 §6.5（iOS）與 SPEC-33 §6.5（Android）執行模型章節。此 ADR 為跨 spec 的正式決策紀錄，解 ARCH-EXECUTION-ENTITIES.md §5 G3（Mobile 執行模型未定）。核心取捨：mobile 必須是能離線自足的 peer（embedded core），而非依賴桌機常開的純 client——這直接服務 BIG-GOAL P1 跨裝置 Mesh 的「每台裝置都是對等節點」原則。
