# 差異化策略分析（誠實版）

> 2026-06-11。Owner 提出的戰略質疑 +（多 lens、對抗式批判的）分析 + owner 拍板的結論。
> 這份是內部誠實評估，不是行銷。與 `_archive/NORTH-STAR.md`（願景）並存——本文挑戰並收斂方向。
> 另見 `COMMERCIALIZATION-STRATEGY.md`（並行的商業化思考）。

---

> ## ⚖️ 治理校準（2026-06-11，owner 拍板 (a)）
>
> **本文從屬於 [`superpowers/BIG-GOAL.md`](superpowers/BIG-GOAL.md)（已鎖定、不可變動的錨）。**
> 本文是**執行層的 sequencing / wedge 指引**,不是產品定義;**它無權重新定義或刪減 BIG GOAL 的四大支柱**。
> 凡本文讀起來像在**降級或砍掉一根支柱**的地方,一律以 BIG GOAL 為準、視為「**排序/聚焦**」而非「**砍**」:
>
> - **§6「mesh/swarm/cluster 降到 P2 彩蛋」= 出界。** P1 跨裝置 Mesh 是 BIG GOAL 第一等公民,**不降級**;它已是做好的 substrate,維持一等地位(命名上的 safety-classifier 顧慮另解,不靠砍支柱)。
> - **§6「停掉 proactive-life」= 出界。** Life Track 是 v0.6.0 首發軌道、服務 P2 多模態,**不砍**;這裡的有效核心只是「**不要在記憶引擎好之前過度鋪 proactive 面的廣度**」。
> - **保留的有效核心(對齊 BIG GOAL,當執行指引)**:① 記憶/recall 引擎(P3 進化網)是**第一個 wedge**——最鋒利的差異化值;② 「1 個深的 > 10 個淺的」是**節奏紀律**;③ moat 不是「記憶」單一功能,而是 **記憶 × 你的 mesh(P1)× 加密只你能讀(P4)× BYOM** 的**組合**——這正是 BIG GOAL 本身,Anthropic 結構上抄不走。
> - **portfolio / 副業**是建好 BIG GOAL 的**下游回報**(工作 + 副業兩者皆要,relay 商業化門虛掩 → 見 COMMERCIALIZATION 的 AGPL 決定),**不得倒過來改寫產品願景**。
>
> 若日後真要縮 BIG GOAL 範圍:按 BIG GOAL 自身條款,那等於**開新專案 / 一次刻意的 re-lock**,要正式做,不得靠本文偷渡。

## 0. Owner 的戰略質疑（出發點）

> 「假如我們功能都跟 OpenClaw / Hermes / Claude Code 差不多甚至更爛，被使用或被加入開發的機會就很低。除了極低的接入成本外，還要有現有工具沒滿足的需求才行。我這種人叫我去試第二、第三個 OpenClaw-like 工具都懶，更何況別人。」
>
> 追加：「memory 那個感覺很薄、Anthropic 過幾個月就會加到 Claude 上、很容易被別人做到。」

**這兩點都對，而且比第一眼更狠。** 下面每一節都假設最懷疑的版本是對的。

## 1. 直答：現在有夠強、守得住的差異化打破「懶得試」門檻嗎？

**沒有——還沒。** 最被當賣點的兩根支柱（sensor/GPS 即時 nudge、phone+cloud-only）最不成熟（GPS 只是 enum 字串、iOS 零 .swift）；最成熟的（cross-machine mesh）是 homelab 玩具、不是大眾採用理由。

## 2. 「memory moat」為什麼薄（owner 的追問是對的）

- cross-repo 記憶本身只是 6–12 個月時間差（Anthropic 已有 memory API、Cursor 已上 codebase 記憶）——**不是護城河**。
- 連 narrowing 到「cross-tool + local-first + 歸你所有」也薄：多數人最終**收斂到一個 agent**，「跨工具」價值依賴過渡期/power-user 行為，不耐久；local-first 是價值觀 niche，大眾不為它換工具。

## 3. 真正的硬真相

**以 solo dev，幾乎贏不了任何「功能護城河」vs Anthropic——句點。** 只要功能有價值且技術可行，他們幾個月內抄走。**「找一個它抄不走的功能」這個遊戲本身是輸的。**

solo dev 真的建得起來、非功能的護城河只有三種：**社群/distribution**、**速度/觀點**（永遠早 6 個月、對特定用戶更有主見）、**它「不願」做的事**（不是不能——打穿其雲端 lock-in 的 local-first，或太小/太怪/受監管的 niche）。

## 4. 結論：moat 是錯的問題——對的問題是「spectyn 為了什麼」

**Owner 拍板：1 + 2（不是 3）。**
1. **個人工具 + 職涯 portfolio**：你天天用的工具 + 一個能證明你會建難系統（agent 編排 / 跨機 / 加密安全 / full-stack）的作品。
2. **針對 power-user 的 niche 產品**：接受它小（幾千個重度開發者），靠「擁有權 / cross-tool / local-first」這種價值觀防禦，而非功能。Anthropic 不會追（太小 + 會自噬）。
- **明確不走 3（venture / 做大）。**

**這直接解掉「moat 太薄」焦慮**：既然目標不是規模化打贏 Anthropic，moat 薄不薄就不重要。對的成功指標變成三條，沒一條需要贏過 Anthropic：(a) 你自己天天用嗎（dogfood）、(b) 夠深夠 polish 能當作品嗎、(c) niche 拿到清楚價值嗎。

## 5. 1+2 指向同一個東西（不衝突）

> **一個 local-first、跑在你自己機器上、你完全擁有的個人 AI——把最強的 agent 當可替換引擎編排，並跨工具/跨機器記得你。**

- 這正是 niche(2) 的價值，也正是 portfolio(1) 會亮的系統（比「又一個 chat app」難太多、好說故事）。
- 「不要競爭、要包住」：agents 是引擎，spectyn 是它們全插進去的記憶 + context 層。定位＝**「繼續用你的 Claude Code。spectyn 是它沒有的長期記憶，跑在 Anthropic 不擁有的硬體上。」**（routing 是 table stakes，永遠當水管別當賣點。）

## 6. 該聚焦 / 該停

**深耕**：記憶/recall 層 + agent 編排 + 跨機（既是 niche 價值、又是作品亮點）。
**停掉**：GPS/sensor/proactive-life（不成熟、賽道最擠、不是 niche 在乎的、也非作品差異點）；跟 coding 深度競爭（你 wrap Claude Code，定義上贏不了）；mesh/swarm/cluster 當頭牌（降到 P2 彩蛋，且名稱會觸發 safety classifier）；一邊 claim「phone+cloud only」一邊要桌面當大腦（這是 false-green）。
**freeze**：在 1+2 下變溫和但方向不變——停止鋪太廣，把 polish 集中在「一個深的、能跑的核心」。**1 個深的 > 10 個淺的。**

## 7. 最該動的那塊（三件事匯流成一件）

「越用越懂」的引擎——`skill_wire.rs` 的 `embedding_search()`/`skill_store()` **已實作**(不再 panic;`skill_store` 持久化 hand-off,`embedding_search` 預設回 `Err(())` → FTS5 keyword recall fallback,語意 `ort` 腿仍 deferred)。它同時是：niche 的核心價值 + 作品最難最亮的一塊 + 讓「今日/recall」首頁真的有料的地基。

**day-1 cold-start 硬傷 + 解法**：記憶產品 day-1 是空的，無法在空記憶上 demo（硬塞假資料＝false-green）。解法＝把 aha 錨在**回溯既有歷史**：install 後索引你硬碟上已有的 git log/commit/檔案，第 5 分鐘說「你三個月前在 commit abc123 解過這個一模一樣的 error，這是當時 diff」。day-1 hook＝回溯式 cross-repo 挖掘；「越用越懂」是 retention hook，不是 acquisition hook。**警告：在 recall 引擎真的好之前沒有誠實的 5 分鐘 demo——絕不 soft-launch keyword-FTS5 半成品（會教會懶人「這東西不 work」，不可逆）。**

## 8. 真實 vs 要建（別 overclaim）

**今天就真**：cross-machine dispatch mesh（contributor hook 非採用 pitch）、多 provider failover、local-first age 加密 event store + FTS5 recall（真差異點的地基）、shame-free nudge、開源/資料歸你。
**aspirational、做出來前別 claim**：複利機制**端到端 dogfood 證明**（引擎已實作:store/recall/apply/measure 都真;缺語意 `ort` 腿 + 天天用證明）、phone+cloud-only path（零 .swift、mobile 殼只 redirect 桌面——現在是假的）、GPS/sensor 攝取（只有 enum）、public repo（還沒 contributor-ready）。
**未解矛盾**：local-first（防禦力）⟷ phone+cloud-only（觸及力）直接相衝——不蓋會重引入 cloud-trust 的同步層，沒辦法又純 local 又手機隨處可達。碰 code 前先決定。

## 9. 殘酷收尾

這是一個「可防禦的小 niche」，曾被誤標成「moat」。在 owner 拍板 1+2 後，這變成**正確且低焦慮的定位**——但成敗綁的不是 idea，是「一個慣性求廣的 solo effort 能不能停止鋪廣、把深度集中在記憶引擎 + 一個好到能見人的回溯式 demo」。**先守得住那個聚焦，定位才有意義。**
