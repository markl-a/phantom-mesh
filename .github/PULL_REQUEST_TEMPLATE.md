<!--
SPEC-01 漂移拒絕關卡（drift-reject gate）。Big Goal（大目標）規定：「每個功能都必須服務 >=1
支柱（pillar）；跨支柱 > 單一支柱」以及「在審查 PR 嗎？先檢查：它是否
服務某個支柱？」（docs/superpowers/BIG-GOAL.md）。這個範本讓該檢查
明確化，使 spec<->prototype（規格<->原型）漂移在審查時就被抓出來，而非數月之後。
只有在某段落確實不適用時才刪除它，並說明原因。
-->

## 內容與原因（What & why）

<!-- One or two sentences. What does this change do, and what gap/bug/spec does it close? -->

## 服務（Big Goal 對齊）— 必填（REQUIRED）

至少勾選一個支柱（pillar）。若都不適用，這個 PR 很可能不應該合併
（見 BIG-GOAL.md 中的「What this Big Goal RULES OUT」）。

- [ ] **P1 · 跨裝置 Mesh** (runs anywhere, cross-device)
- [ ] **P2 · 多模態理解** (multimodal capture/understanding)
- [ ] **P3 · 進化網 Evolve Mesh** (self-improvement / agent swarm)
- [ ] **P4 · 加密為先** (encryption-first, only-you-can-read)
- [ ] **Infra / CI / packaging / docs** (enables a pillar without being a feature itself — name which: ____)

**Track（軌道）：** [ ] Life Track 陪你進步  [ ] Work Track 替你做事  [ ] N/A (infra)

<!--
REQUIRED machine-checked line. CI greps for a `Serves:` line naming a §8
capability slug from SPEC-01 (docs/.../SPEC-01-FOUNDATION-bigGoal-mapping.md §8,
the 23-CAP taxonomy). Format: `Serves: <pillar>.<slug>`. Keep the line below,
replacing the example slug with the real one. No valid slug = drift = CI reject.
Examples: P3.mcp · P1.peer-wire · P4.age-encrypt · X.coach · X.release-infra
-->
Serves: P3.mcp

**它所關閉的 Spec / 漂移項目（ID 或連結）：** <!-- e.g. SPEC-42, T-WL-03, drift report 2026-05-29 line N -->

## 驗證（Verification）

<!-- How did you confirm this works? Paste the command + result. "cargo test", CI run link, manual smoke, screenshot. Evidence before assertions. -->

- [ ] `cargo check` / 相關測試在本機或 CI 通過
- [ ] 已考量跨平台影響（this repo ships Win + Linux + macOS + Android + iOS）
- [ ] 未加入任何密鑰（secrets）/ 個人資料 / 內部主機名稱（hostnames）

## 給審查者的備註（Notes for reviewers）

<!-- Anything risky, deferred, or out of scope. Follow-ups you filed. -->
