# `core/tests/fixtures/`

phantom-mesh **獨有工具（unique tools）** 的 L1 測試黃金 I/O 固件（golden fixture，黃金基準測試樣本）——
即由 phantom-mesh 自行撰寫的工具（不是從 Hermes 上游繼承來的）。

> 為什麼只測「獨有工具」？見 `goal_plan/docs/29 §2 紅線 2`：
> > phantom-mesh 自己寫的工具/能力才需要 PM 自己出力測。
> > Hermes 上游的 25 個 tool + 11 個 provider 繼承上游 CI 即可,不重測。

## Schema（綱要）

每個固件都是符合以下結構的 JSON 檔：

```jsonc
{
  "tool": "service",                  // tool name as exposed to LLM
  "scenario": "install-then-status",  // short scenario id (unique per tool)
  "platform": "windows",              // "windows" | "linux" | "macos" | "android" | "any"
  "input": {                          // arguments passed to the tool
    "action": "install"
  },
  "expected": {                       // expected response shape
    "ok": true,
    "subset_match": {                 // subset of fields that must match
      "task_name": "PhantomMesh",
      "status": "registered"
    }
  },
  "notes": "..."                      // why this fixture exists
}
```

測試執行器（test runner）會載入每個固件，以 `input` 呼叫該工具，並
斷言（assert）回應符合 `expected.subset_match`（允許出現額外
欄位）。`ok: true` 代表工具以成功狀態結束；`ok: false` 代表
該測試預期會發生一個受控的錯誤（controlled error）。

## Coverage（涵蓋範圍）

| 工具 | 固件 | 狀態 |
|---|---|---|
| `service` | `tools/service.json` | 🟢 |
| `mobile_bridge` | — | 🟡 v0.6.0 PF-3(等 port from public)|
| `worker_setup` | — | 🟡 v0.6.0 PF-8 |
| `get_node_capabilities` | — | 🟡 v0.6.0 PF-3 + PF-4 |
| `vault` | — | 🟡 v0.6.0 PF-5 |

Hermes 上游工具（`fs:read`、`web_search`、`shell:exec` 等）
不在此涵蓋——它們依附於上游的 CI。
