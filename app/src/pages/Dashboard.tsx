import { useState } from "react";
import { RefreshCw } from "lucide-react";
import TasksPanel from "../components/dashboard/TasksPanel";
import CostPanel from "../components/dashboard/CostPanel";
import NodeInfoPanel from "../components/dashboard/NodeInfoPanel";

export default function Dashboard() {
  // Incrementing key forces child panels to re-mount (fresh fetch)
  const [refreshKey, setRefreshKey] = useState(0);

  return (
    <div className="flex flex-col h-full gap-0">
      {/* Header */}
      <div className="flex items-center justify-between mb-4">
        <h1 className="text-2xl font-bold">儀表板</h1>
        <button
          onClick={() => setRefreshKey((k) => k + 1)}
          className="flex items-center gap-2 border border-phantom-border text-phantom-muted px-3 py-1.5 rounded text-sm hover:text-phantom-text hover:border-phantom-primary/50 transition-colors"
          title="重新整理所有面板"
        >
          <RefreshCw size={14} />
          重新整理
        </button>
      </div>

      {/* G2 two-column layout */}
      <div className="flex gap-4 flex-1 min-h-0">
        {/* Left ~60% — Tasks */}
        <div className="flex-[3] min-w-0 flex flex-col">
          <div className="bg-phantom-card border border-phantom-border rounded-xl p-4 flex-1 overflow-y-auto">
            <h2 className="text-sm font-semibold text-phantom-muted uppercase tracking-wider mb-4">
              任務
            </h2>
            <TasksPanel key={`tasks-${refreshKey}`} />
          </div>
        </div>

        {/* Right ~40% — NodeInfo (top) + Cost (bottom) */}
        <div className="flex-[2] min-w-0 flex flex-col gap-4">
          {/* Node Info */}
          <div className="bg-phantom-card border border-phantom-border rounded-xl p-4 overflow-y-auto max-h-[55%]">
            <h2 className="text-sm font-semibold text-phantom-muted uppercase tracking-wider mb-4">
              節點
            </h2>
            <NodeInfoPanel key={`nodes-${refreshKey}`} />
          </div>

          {/* Cost */}
          <div className="bg-phantom-card border border-phantom-border rounded-xl p-4 overflow-y-auto flex-1">
            <h2 className="text-sm font-semibold text-phantom-muted uppercase tracking-wider mb-4">
              成本
            </h2>
            <CostPanel key={`cost-${refreshKey}`} />
          </div>
        </div>
      </div>
    </div>
  );
}
