import { useState } from "react";
import { Loader2, CheckCircle2, XCircle, Clock, ChevronDown } from "lucide-react";
import type { ToolCall } from "../../lib/types";

// ─── Tool icon mapping ────────────────────────────────────────────────────────
function toolIcon(name: string): string {
  const n = name.toLowerCase();
  if (n.includes("shell") || n.includes("bash") || n.includes("exec") || n.includes("run")) return "🔧";
  if (n.includes("file_read") || n.includes("read_file") || n.includes("file_get")) return "📄";
  if (n.includes("file_write") || n.includes("write_file") || n.includes("file_put")) return "✏️";
  if (n.includes("web_search") || n.includes("search")) return "🌐";
  if (n.includes("browse") || n.includes("fetch") || n.includes("http")) return "🌐";
  if (n.includes("memory") || n.includes("recall")) return "🧠";
  if (n.includes("code") || n.includes("python") || n.includes("eval")) return "💻";
  if (n.includes("agent") || n.includes("task")) return "🤖";
  if (n.includes("list") || n.includes("ls") || n.includes("dir")) return "📁";
  return "⚙️";
}

// ─── Status icon ──────────────────────────────────────────────────────────────
function StatusIcon({ status }: { status: ToolCall["status"] | undefined }) {
  switch (status) {
    case "pending": return <Clock size={12} className="text-phantom-muted flex-shrink-0" />;
    case "running": return <Loader2 size={12} className="text-phantom-primary animate-spin flex-shrink-0" />;
    case "done":    return <CheckCircle2 size={12} className="text-phantom-success flex-shrink-0" />;
    case "error":   return <XCircle size={12} className="text-phantom-danger flex-shrink-0" />;
    default:        return <Clock size={12} className="text-phantom-muted flex-shrink-0" />;
  }
}

// ─── Collapsible code block ────────────────────────────────────────────────────
interface CollapsibleBlockProps {
  label: string;
  content: string;
  scrollable?: boolean;
}

function CollapsibleBlock({ label, content, scrollable = false }: CollapsibleBlockProps) {
  const [expanded, setExpanded] = useState(false);
  return (
    <div className="mt-1">
      <button
        onClick={() => setExpanded((v) => !v)}
        className="flex items-center gap-1 text-[10px] text-phantom-muted hover:text-phantom-text transition"
      >
        <ChevronDown size={10} className={`transition-transform ${expanded ? "rotate-180" : ""}`} />
        {label}
      </button>
      {expanded && (
        <pre
          className={`mt-1 text-[10px] bg-phantom-bg border border-phantom-border rounded p-2 overflow-x-auto whitespace-pre-wrap break-all text-phantom-muted ${
            scrollable ? "max-h-[200px] overflow-y-auto" : ""
          }`}
        >
          {content}
        </pre>
      )}
    </div>
  );
}

// ─── Single tool call row ─────────────────────────────────────────────────────
function SingleToolCall({ toolCall }: { toolCall: ToolCall }) {
  const icon = toolIcon(toolCall.name);
  const hasArgs = toolCall.args && Object.keys(toolCall.args).length > 0;
  const hasResult = !!toolCall.result;

  return (
    <div className="border border-phantom-border rounded p-2 text-xs bg-phantom-bg/50">
      <div className="flex items-center gap-1.5">
        <span className="text-sm leading-none">{icon}</span>
        <StatusIcon status={toolCall.status} />
        <span className="font-mono text-phantom-primary truncate">{toolCall.name}</span>
      </div>

      {hasArgs && (
        <CollapsibleBlock
          label="Args"
          content={JSON.stringify(toolCall.args, null, 2)}
        />
      )}

      {hasResult && (
        <CollapsibleBlock
          label="Result"
          content={toolCall.result!}
          scrollable
        />
      )}
    </div>
  );
}

// ─── Public component: handles summary + expandable list ──────────────────────
interface ToolCallDisplayProps {
  toolCalls: ToolCall[];
}

export default function ToolCallDisplay({ toolCalls }: ToolCallDisplayProps) {
  const [expanded, setExpanded] = useState(false);

  if (toolCalls.length === 0) return null;

  const doneCount = toolCalls.filter((tc) => tc.status === "done").length;
  const runningCount = toolCalls.filter((tc) => tc.status === "running").length;

  const summaryLabel = runningCount > 0
    ? `⚙️ Running ${runningCount} tool call${runningCount > 1 ? "s" : ""}…`
    : `✓ ${doneCount} tool call${doneCount !== 1 ? "s" : ""}`;

  return (
    <div className="mt-2">
      {/* Summary toggle */}
      <button
        onClick={() => setExpanded((v) => !v)}
        className="flex items-center gap-1.5 text-xs text-phantom-muted hover:text-phantom-text transition"
      >
        <ChevronDown size={12} className={`transition-transform ${expanded ? "rotate-180" : ""}`} />
        {summaryLabel}
      </button>

      {/* Expanded tool call list */}
      {expanded && (
        <div className="mt-1.5 space-y-1.5">
          {toolCalls.map((tc, i) => (
            <SingleToolCall key={i} toolCall={tc} />
          ))}
        </div>
      )}
    </div>
  );
}
