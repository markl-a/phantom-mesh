import { useState, useEffect, useCallback } from "react";
import { safeInvoke as invoke } from "../../lib/tauri-compat";

type RiskLevel = "low" | "medium" | "high" | "critical";

interface AuditEvent {
  id: string;
  timestamp: string;
  agent: string;
  action: string;
  tool: string;
  riskLevel: RiskLevel;
  result: "允許" | "阻擋" | "審核中";
}

interface SecurityPageState {
  isOffline: boolean;
  loading: boolean;
  error: string | null;
  events: AuditEvent[];
}

const RISK_CONFIG: Record<RiskLevel, { label: string; color: string }> = {
  low: { label: "Low", color: "bg-phantom-success/20 text-phantom-success" },
  medium: { label: "Medium", color: "bg-phantom-warning/20 text-phantom-warning" },
  high: { label: "High", color: "bg-orange-500/20 text-orange-400" },
  critical: { label: "Critical", color: "bg-phantom-danger/20 text-phantom-danger" },
};

const RESULT_STYLE: Record<string, string> = {
  "允許": "text-phantom-success",
  "阻擋": "text-phantom-danger",
  "審核中": "text-phantom-warning",
};

const MOCK_EVENTS: AuditEvent[] = [
  {
    id: "AUD-001",
    timestamp: "2026-03-21 11:42",
    agent: "Coder",
    action: "寫入檔案",
    tool: "fs_write",
    riskLevel: "low",
    result: "允許",
  },
  {
    id: "AUD-002",
    timestamp: "2026-03-21 11:38",
    agent: "Browser",
    action: "訪問外部 URL",
    tool: "http_fetch",
    riskLevel: "medium",
    result: "允許",
  },
  {
    id: "AUD-003",
    timestamp: "2026-03-21 11:35",
    agent: "Coder",
    action: "執行 Shell 命令",
    tool: "shell_exec",
    riskLevel: "high",
    result: "允許",
  },
  {
    id: "AUD-004",
    timestamp: "2026-03-21 11:30",
    agent: "Master",
    action: "修改系統設定",
    tool: "config_set",
    riskLevel: "critical",
    result: "審核中",
  },
  {
    id: "AUD-005",
    timestamp: "2026-03-21 11:22",
    agent: "Coder",
    action: "讀取環境變數",
    tool: "env_read",
    riskLevel: "medium",
    result: "允許",
  },
  {
    id: "AUD-006",
    timestamp: "2026-03-21 11:15",
    agent: "Browser",
    action: "上傳檔案至外部",
    tool: "http_upload",
    riskLevel: "critical",
    result: "阻擋",
  },
  {
    id: "AUD-007",
    timestamp: "2026-03-21 11:10",
    agent: "Reviewer",
    action: "讀取原始碼",
    tool: "fs_read",
    riskLevel: "low",
    result: "允許",
  },
  {
    id: "AUD-008",
    timestamp: "2026-03-21 11:05",
    agent: "Analyst",
    action: "查詢資料庫",
    tool: "db_query",
    riskLevel: "medium",
    result: "允許",
  },
  {
    id: "AUD-009",
    timestamp: "2026-03-21 10:58",
    agent: "Coder",
    action: "安裝套件",
    tool: "package_install",
    riskLevel: "high",
    result: "允許",
  },
  {
    id: "AUD-010",
    timestamp: "2026-03-21 10:45",
    agent: "Master",
    action: "刪除任務記錄",
    tool: "task_delete",
    riskLevel: "high",
    result: "阻擋",
  },
  {
    id: "AUD-011",
    timestamp: "2026-03-21 10:30",
    agent: "Coder",
    action: "讀取檔案",
    tool: "fs_read",
    riskLevel: "low",
    result: "允許",
  },
  {
    id: "AUD-012",
    timestamp: "2026-03-21 10:20",
    agent: "Reviewer",
    action: "掃描依賴漏洞",
    tool: "vuln_scan",
    riskLevel: "low",
    result: "允許",
  },
];

function isValidRiskLevel(value: string): value is RiskLevel {
  return value === "low" || value === "medium" || value === "high" || value === "critical";
}

function isValidResult(value: string): value is AuditEvent["result"] {
  return value === "允許" || value === "阻擋" || value === "審核中";
}

function parseAuditEvent(raw: Record<string, unknown>, index: number): AuditEvent {
  const id = raw["id"] ?? `AUD-${String(index + 1).padStart(3, "0")}`;
  const timestamp = raw["timestamp"] ?? raw["created_at"] ?? raw["time"] ?? "";
  const agent = raw["agent"] ?? raw["agent_name"] ?? raw["actor"] ?? "Unknown";
  const action = raw["action"] ?? raw["description"] ?? raw["event"] ?? "";
  const tool = raw["tool"] ?? raw["tool_name"] ?? raw["resource"] ?? "";
  const riskRaw = String(raw["riskLevel"] ?? raw["risk_level"] ?? raw["risk"] ?? "low").toLowerCase();
  const resultRaw = raw["result"] ?? raw["outcome"] ?? raw["status"] ?? "";

  // Map English results to Chinese if needed
  let resultStr = String(resultRaw);
  if (resultStr.toLowerCase() === "allowed" || resultStr.toLowerCase() === "permit") resultStr = "允許";
  if (resultStr.toLowerCase() === "blocked" || resultStr.toLowerCase() === "denied") resultStr = "阻擋";
  if (resultStr.toLowerCase() === "pending" || resultStr.toLowerCase() === "reviewing") resultStr = "審核中";

  return {
    id: String(id),
    timestamp: String(timestamp),
    agent: String(agent),
    action: String(action),
    tool: String(tool),
    riskLevel: isValidRiskLevel(riskRaw) ? riskRaw : "low",
    result: isValidResult(resultStr) ? resultStr : "允許",
  };
}

export default function SecurityPanel() {
  const [riskFilter, setRiskFilter] = useState<RiskLevel | "all">("all");
  const [state, setState] = useState<SecurityPageState>({
    isOffline: false,
    loading: true,
    error: null,
    events: MOCK_EVENTS,
  });

  const fetchEvents = useCallback(async (filterLevel?: RiskLevel | "all") => {
    setState((prev) => ({ ...prev, loading: true, error: null }));
    const riskArg = filterLevel && filterLevel !== "all" ? filterLevel : null;
    try {
      const result = await invoke("get_audit_log", {
        risk_level: riskArg,
        limit: 50,
      }) as unknown;

      let events: AuditEvent[] = [];

      if (Array.isArray(result)) {
        events = (result as Record<string, unknown>[]).map(parseAuditEvent);
      } else if (result && typeof result === "object") {
        const obj = result as Record<string, unknown>;
        const data = obj["entries"] ?? obj["events"] ?? obj["data"] ?? obj["log"] ?? obj["items"];
        if (Array.isArray(data)) {
          events = (data as Record<string, unknown>[]).map(parseAuditEvent);
        }
      }

      setState({
        isOffline: false,
        loading: false,
        error: null,
        events,
      });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setState({
        isOffline: true,
        loading: false,
        error: message,
        events: MOCK_EVENTS,
      });
    }
  }, []);

  useEffect(() => {
    fetchEvents(riskFilter);
  }, [fetchEvents, riskFilter]);

  const handleFilterChange = (level: RiskLevel | "all") => {
    setRiskFilter(level);
    // fetchEvents will be triggered by the useEffect dependency on riskFilter
  };

  // When offline, do client-side filtering
  const displayedEvents = state.isOffline && riskFilter !== "all"
    ? state.events.filter((e) => e.riskLevel === riskFilter)
    : state.events;

  const statsSource = state.isOffline ? MOCK_EVENTS : state.events;
  const stats = {
    totalToday: statsSource.length,
    blocked: statsSource.filter((e) => e.result === "阻擋").length,
    highRisk: statsSource.filter((e) => e.riskLevel === "high" || e.riskLevel === "critical").length,
    reviewing: statsSource.filter((e) => e.result === "審核中").length,
  };

  return (
    <div>
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-3">
          <h1 className="text-2xl font-bold">安全與審計</h1>
          {state.isOffline && (
            <span className="text-xs px-2 py-0.5 rounded bg-phantom-warning/20 text-phantom-warning">
              (離線模式)
            </span>
          )}
        </div>
        {!state.loading && (
          <button
            onClick={() => fetchEvents(riskFilter)}
            className="text-xs text-phantom-muted hover:text-phantom-text border border-phantom-border rounded px-2 py-1"
          >
            重新整理
          </button>
        )}
      </div>

      {/* Error Banner */}
      {state.error && (
        <div className="mb-4 bg-phantom-danger/10 border border-phantom-danger/30 rounded-lg px-4 py-3 flex items-center justify-between">
          <span className="text-sm text-phantom-danger">
            無法取得審計日誌: {state.error}
          </span>
          <button
            onClick={() => fetchEvents(riskFilter)}
            className="text-xs text-phantom-danger border border-phantom-danger/30 rounded px-2 py-1 hover:bg-phantom-danger/10"
          >
            重試
          </button>
        </div>
      )}

      {/* Loading State */}
      {state.loading && (
        <div className="flex items-center justify-center py-12">
          <div className="flex items-center gap-3 text-phantom-muted">
            <svg className="animate-spin h-5 w-5" viewBox="0 0 24 24" fill="none">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
            </svg>
            <span className="text-sm">載入審計日誌中...</span>
          </div>
        </div>
      )}

      {!state.loading && (
        <>
          {/* Stats */}
          <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-6">
            <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
              <p className="text-phantom-muted text-xs">今日事件</p>
              <p className="text-2xl font-bold mt-1">{stats.totalToday}</p>
            </div>
            <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
              <p className="text-phantom-muted text-xs">已阻擋</p>
              <p className="text-2xl font-bold mt-1 text-phantom-danger">{stats.blocked}</p>
            </div>
            <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
              <p className="text-phantom-muted text-xs">高風險動作</p>
              <p className="text-2xl font-bold mt-1 text-orange-400">{stats.highRisk}</p>
            </div>
            <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
              <p className="text-phantom-muted text-xs">審核中</p>
              <p className="text-2xl font-bold mt-1 text-phantom-warning">{stats.reviewing}</p>
            </div>
          </div>

          {/* Filter */}
          <div className="flex items-center gap-2 mb-4">
            <span className="text-sm text-phantom-muted">篩選風險等級:</span>
            {(["all", "low", "medium", "high", "critical"] as const).map((level) => (
              <button
                key={level}
                onClick={() => handleFilterChange(level)}
                className={`px-3 py-1 rounded text-xs font-medium transition-colors ${
                  riskFilter === level
                    ? level === "all"
                      ? "bg-phantom-primary text-phantom-bg"
                      : RISK_CONFIG[level].color
                    : "bg-phantom-card border border-phantom-border text-phantom-muted hover:text-phantom-text"
                }`}
              >
                {level === "all" ? "全部" : RISK_CONFIG[level].label}
              </button>
            ))}
          </div>

          {/* Audit Log Table */}
          <div className="bg-phantom-card border border-phantom-border rounded-lg overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-phantom-border">
                  <th className="text-left px-4 py-3 text-phantom-muted font-medium">時間</th>
                  <th className="text-left px-4 py-3 text-phantom-muted font-medium">Agent</th>
                  <th className="text-left px-4 py-3 text-phantom-muted font-medium">動作</th>
                  <th className="text-left px-4 py-3 text-phantom-muted font-medium">工具</th>
                  <th className="text-left px-4 py-3 text-phantom-muted font-medium">風險等級</th>
                  <th className="text-left px-4 py-3 text-phantom-muted font-medium">結果</th>
                </tr>
              </thead>
              <tbody>
                {displayedEvents.map((event, i) => (
                  <tr
                    key={event.id}
                    className={`border-b border-phantom-border last:border-0 ${
                      i % 2 === 1 ? "bg-phantom-bg/50" : ""
                    }`}
                  >
                    <td className="px-4 py-3 text-phantom-muted font-mono text-xs">
                      {event.timestamp}
                    </td>
                    <td className="px-4 py-3">{event.agent}</td>
                    <td className="px-4 py-3">{event.action}</td>
                    <td className="px-4 py-3">
                      <span className="font-mono text-xs bg-phantom-bg px-1.5 py-0.5 rounded border border-phantom-border">
                        {event.tool}
                      </span>
                    </td>
                    <td className="px-4 py-3">
                      <span
                        className={`inline-block px-2 py-0.5 rounded text-xs font-medium ${
                          RISK_CONFIG[event.riskLevel].color
                        }`}
                      >
                        {RISK_CONFIG[event.riskLevel].label}
                      </span>
                    </td>
                    <td className="px-4 py-3">
                      <span className={`text-xs font-medium ${RESULT_STYLE[event.result] || ""}`}>
                        {event.result}
                      </span>
                    </td>
                  </tr>
                ))}
                {displayedEvents.length === 0 && (
                  <tr>
                    <td colSpan={6} className="px-4 py-8 text-center text-phantom-muted">
                      沒有符合條件的審計事件
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>

          {/* Security Policy Info */}
          <div className="mt-6 bg-phantom-card border border-phantom-border rounded-lg p-4">
            <h3 className="text-sm font-medium mb-2">安全策略</h3>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-xs text-phantom-muted leading-relaxed">
              <div>
                <p className="font-medium text-phantom-text mb-1">工具權限</p>
                <p>
                  每個 Agent 僅能存取已授權的工具。High/Critical 風險動作需經 Master Agent 核准，
                  或由使用者透過 Telegram 即時確認。
                </p>
              </div>
              <div>
                <p className="font-medium text-phantom-text mb-1">自動阻擋規則</p>
                <p>
                  外部上傳、系統檔案刪除、認證資訊洩露等行為自動阻擋。
                  所有 Agent 動作均記錄於不可竄改的審計日誌中。
                </p>
              </div>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
