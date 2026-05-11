import { useState, useRef, useEffect, useCallback } from "react";

type LogLevel = "INFO" | "WARN" | "ERROR" | "DEBUG";

interface LogEntry {
  timestamp: string;
  level: LogLevel;
  module: string;
  message: string;
}

const LEVEL_COLOR: Record<LogLevel, string> = {
  INFO: "text-phantom-text",
  WARN: "text-phantom-warning",
  ERROR: "text-phantom-danger",
  DEBUG: "text-phantom-muted",
};

const LEVEL_BADGE: Record<LogLevel, string> = {
  INFO: "bg-phantom-primary/20 text-phantom-primary",
  WARN: "bg-phantom-warning/20 text-phantom-warning",
  ERROR: "bg-phantom-danger/20 text-phantom-danger",
  DEBUG: "bg-phantom-muted/20 text-phantom-muted",
};

const MOCK_LOGS: LogEntry[] = [
  { timestamp: "2026-03-21 23:14:02", level: "INFO", module: "daemon", message: "Clawtex daemon started on port 7878" },
  { timestamp: "2026-03-21 23:14:02", level: "INFO", module: "plugin_bus", message: "Initializing Phase 1: Config, Logger, Metrics" },
  { timestamp: "2026-03-21 23:14:03", level: "INFO", module: "plugin_bus", message: "Phase 1 complete (3 modules)" },
  { timestamp: "2026-03-21 23:14:03", level: "INFO", module: "plugin_bus", message: "Initializing Phase 2: OptimizerStore, MemoryStore..." },
  { timestamp: "2026-03-21 23:14:04", level: "WARN", module: "provider", message: "Ollama connection timeout, retrying..." },
  { timestamp: "2026-03-21 23:14:05", level: "INFO", module: "provider", message: "Ollama connected: qwen3:8b ready" },
  { timestamp: "2026-03-21 23:14:05", level: "INFO", module: "cluster", message: "Hub listening on 0.0.0.0:7878" },
  { timestamp: "2026-03-21 23:14:06", level: "INFO", module: "cluster", message: "Worker 'm1-mac' registered (capabilities: [llm, sandbox])" },
  { timestamp: "2026-03-21 23:14:10", level: "INFO", module: "telegram", message: "Bot @clawtex_bot connected" },
  { timestamp: "2026-03-21 23:14:15", level: "DEBUG", module: "cron", message: "Next scheduled task: embed-pipeline at 00:00" },
  { timestamp: "2026-03-21 23:14:30", level: "INFO", module: "agent", message: "Task #42 assigned to master agent" },
  { timestamp: "2026-03-21 23:14:31", level: "INFO", module: "agent", message: 'Tool call: web_search("clawtex documentation")' },
  { timestamp: "2026-03-21 23:14:33", level: "INFO", module: "agent", message: "Task #42 completed (2.1s, 3 tool calls)" },
  { timestamp: "2026-03-21 23:14:45", level: "WARN", module: "circuit_breaker", message: "Provider 'groq' circuit OPEN (3 consecutive failures)" },
  { timestamp: "2026-03-21 23:15:00", level: "ERROR", module: "sandbox", message: "Docker container timeout after 60s, killed" },
  { timestamp: "2026-03-21 23:15:01", level: "INFO", module: "cost_tracker", message: "Daily spend: $0.42 / $5.00 budget" },
  { timestamp: "2026-03-21 23:15:15", level: "DEBUG", module: "memory", message: "Compacting episodic memory (128 entries -> 42)" },
  { timestamp: "2026-03-21 23:15:30", level: "WARN", module: "provider", message: "OpenAI rate limit approaching (80% of quota)" },
  { timestamp: "2026-03-21 23:15:45", level: "ERROR", module: "agent", message: "SubAgent 'browser' crashed: OOM killed (512MB limit)" },
  { timestamp: "2026-03-21 23:16:00", level: "INFO", module: "agent", message: "SubAgent 'browser' restarted successfully" },
  { timestamp: "2026-03-21 23:16:10", level: "INFO", module: "evolution", message: "Skill 'web-search' auto-updated to v1.2.1" },
  { timestamp: "2026-03-21 23:16:20", level: "ERROR", module: "plugin_bus", message: "Plugin 'pdf-extractor' sandbox violation: unauthorized syscall" },
  { timestamp: "2026-03-21 23:16:30", level: "DEBUG", module: "cluster", message: "Heartbeat: 3/3 workers healthy, avg latency 12ms" },
];

const ALL_MODULES = Array.from(new Set(MOCK_LOGS.map((l) => l.module))).sort();

export default function LogsPanel() {
  const [logs, setLogs] = useState<LogEntry[]>(MOCK_LOGS);
  const [levelFilter, setLevelFilter] = useState<LogLevel | "ALL">("ALL");
  const [moduleFilter, setModuleFilter] = useState<string>("ALL");
  const [autoScroll, setAutoScroll] = useState(true);
  const logEndRef = useRef<HTMLDivElement>(null);

  const scrollToBottom = useCallback(() => {
    if (autoScroll && logEndRef.current) {
      logEndRef.current.scrollIntoView({ behavior: "smooth" });
    }
  }, [autoScroll]);

  useEffect(() => {
    scrollToBottom();
  }, [logs, scrollToBottom]);

  const filtered = logs.filter((entry) => {
    if (levelFilter !== "ALL" && entry.level !== levelFilter) return false;
    if (moduleFilter !== "ALL" && entry.module !== moduleFilter) return false;
    return true;
  });

  const stats = {
    total: logs.length,
    errors: logs.filter((l) => l.level === "ERROR").length,
    warnings: logs.filter((l) => l.level === "WARN").length,
    recentHour: 156,
  };

  const handleClear = () => {
    setLogs([]);
  };

  const handleExport = () => {
    // Placeholder for export functionality
  };

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">系統日誌</h1>
      </div>

      {/* Stats */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-6">
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
          <p className="text-phantom-muted text-xs">總日誌數</p>
          <p className="text-2xl font-bold mt-1">{stats.total.toLocaleString()}</p>
        </div>
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
          <p className="text-phantom-muted text-xs">錯誤</p>
          <p className="text-2xl font-bold mt-1 text-phantom-danger">{stats.errors}</p>
        </div>
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
          <p className="text-phantom-muted text-xs">警告</p>
          <p className="text-2xl font-bold mt-1 text-phantom-warning">{stats.warnings}</p>
        </div>
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-4">
          <p className="text-phantom-muted text-xs">最近 1 小時</p>
          <p className="text-2xl font-bold mt-1">{stats.recentHour}</p>
        </div>
      </div>

      {/* Filter Bar */}
      <div className="flex items-center gap-3 mb-4 flex-wrap">
        <div className="flex items-center gap-2">
          <span className="text-sm text-phantom-muted">等級:</span>
          {(["ALL", "INFO", "WARN", "ERROR", "DEBUG"] as const).map((level) => (
            <button
              key={level}
              onClick={() => setLevelFilter(level)}
              className={`px-3 py-1 rounded text-xs font-medium transition-colors ${
                levelFilter === level
                  ? level === "ALL"
                    ? "bg-phantom-primary text-phantom-bg"
                    : LEVEL_BADGE[level]
                  : "bg-phantom-card border border-phantom-border text-phantom-muted hover:text-phantom-text"
              }`}
            >
              {level}
            </button>
          ))}
        </div>

        <div className="flex items-center gap-2">
          <span className="text-sm text-phantom-muted">模組:</span>
          <select
            value={moduleFilter}
            onChange={(e) => setModuleFilter(e.target.value)}
            className="bg-phantom-card border border-phantom-border rounded px-3 py-1 text-xs text-phantom-text focus:outline-none focus:border-phantom-primary"
          >
            <option value="ALL">全部</option>
            {ALL_MODULES.map((mod) => (
              <option key={mod} value={mod}>
                {mod}
              </option>
            ))}
          </select>
        </div>

        <div className="flex items-center gap-2 ml-auto">
          <button
            onClick={() => setAutoScroll(!autoScroll)}
            className={`px-3 py-1 rounded text-xs font-medium transition-colors ${
              autoScroll
                ? "bg-phantom-success/20 text-phantom-success"
                : "bg-phantom-card border border-phantom-border text-phantom-muted hover:text-phantom-text"
            }`}
          >
            自動捲動 {autoScroll ? "ON" : "OFF"}
          </button>
          <button
            onClick={handleClear}
            className="px-3 py-1 rounded text-xs font-medium bg-phantom-card border border-phantom-border text-phantom-muted hover:text-phantom-danger transition-colors"
          >
            清除
          </button>
          <button
            onClick={handleExport}
            className="px-3 py-1 rounded text-xs font-medium bg-phantom-card border border-phantom-border text-phantom-muted hover:text-phantom-text transition-colors"
          >
            匯出
          </button>
        </div>
      </div>

      {/* Log Viewer */}
      <div className="bg-phantom-card border border-phantom-border rounded-lg overflow-hidden flex-1 min-h-0">
        <div className="overflow-auto max-h-[calc(100vh-420px)] p-4 font-mono text-xs leading-relaxed">
          {filtered.length === 0 ? (
            <p className="text-phantom-muted text-center py-8">沒有符合條件的日誌</p>
          ) : (
            filtered.map((entry, i) => (
              <div key={i} className={`py-0.5 ${LEVEL_COLOR[entry.level]}`}>
                <span className="text-phantom-muted">[{entry.timestamp}]</span>{" "}
                <span
                  className={`inline-block w-14 text-center font-medium ${LEVEL_COLOR[entry.level]}`}
                >
                  [{entry.level}]
                </span>{" "}
                <span className="text-phantom-primary">[{entry.module}]</span>{" "}
                <span>{entry.message}</span>
              </div>
            ))
          )}
          <div ref={logEndRef} />
        </div>
      </div>
    </div>
  );
}
