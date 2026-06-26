# Skills（技能庫）

Skill Document（技能文件）是帶有結構化 YAML frontmatter（前置設定區塊）的 Markdown 檔案。
frontmatter 是 curator/router（策展器／路由器）用來推理判斷的依據；Markdown 本文則是
給 LLM（大型語言模型）在執行階段閱讀的散文說明。

- **Schema（結構定義）:** `../skill-schema.json` (JSON Schema draft-07).
- **Parser（解析器）:** `core/src/skillbank/skill.rs` (Rust, gated behind cargo feature
  `experimental-curator`).
- **Sample（範例）:** `sample-skill.md` in this directory.

## 撰寫規則

1. 檔案必須在第 1 行以 `---` fence（圍欄）開頭。
2. frontmatter 必須是有效的 YAML 並符合 schema。
3. 結尾的 `---` fence 必須獨立成一行。
4. 結尾 fence 之後的所有內容都屬於技能本文（Markdown，自由格式）。

## Round-trip 保證（往返一致性保證）

Rust 解析器會斷言 `parse(serialize(parse(input))) == parse(input)`。
序列化器會將結尾換行正規化（本文結尾固定一個 `\n`）；本文內部的其他
空白字元則原封不動地保留。
