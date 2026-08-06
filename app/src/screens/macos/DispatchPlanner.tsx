// SPEC-26 cluster dispatch — "dispatch planner" (settings → 派工規劃).
//
// Pick the capabilities a task needs → plan_dispatch picks the best cluster
// peer (capability-tag Jaccard match + scoring) and lists fallbacks + the
// scoring reason. Backed by the fully-wired cluster_dispatch_wire::plan_dispatch
// (deterministic, no LLM/storage) via lib/clusterDispatchPlan.ts; peers come
// from the F101 useClusterPeers hook. Design lineage: SPEC-26 §6.2 scoring.

import { useState } from "react";
import { Send, Loader2, ArrowRight } from "lucide-react";
import { useClusterPeers } from "../../hooks/useClusterPeers";
import {
  CAP_OPTIONS, buildTask, peerToCaps, planDispatch, describeDispatchError,
} from "../../lib/clusterDispatchPlan";
import type { DispatchPlan } from "../../lib/generated/cluster_dispatch/DispatchPlan";

export default function DispatchPlanner() {
  const { peers } = useClusterPeers();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [plan, setPlan] = useState<DispatchPlan | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [planning, setPlanning] = useState(false);

  const toggle = (slug: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      next.has(slug) ? next.delete(slug) : next.add(slug);
      return next;
    });
  };

  const run = async () => {
    setPlanning(true);
    setError(null);
    setPlan(null);
    try {
      const result = await planDispatch(buildTask([...selected]), peers.map(peerToCaps));
      setPlan(result);
    } catch (e) {
      setError(describeDispatchError(e));
    } finally {
      setPlanning(false);
    }
  };

  const nameOf = (id: string) => peers.find((p) => p.peer_id === id)?.display_name ?? id;

  return (
    <div className="max-w-2xl space-y-5" data-testid="dispatch-planner">
      <header>
        <h1 className="text-lg font-bold text-spectyn-text">派工規劃</h1>
        <p className="text-xs text-spectyn-muted">
          選擇任務所需能力，預覽叢集會把它派給哪個 peer（SPEC-26 capability matching）
        </p>
      </header>

      <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-4 space-y-3">
        <p className="text-xs text-spectyn-muted">所需能力（required caps）：</p>
        <div className="flex flex-wrap gap-2">
          {CAP_OPTIONS.map((slug) => (
            <button
              key={slug}
              aria-pressed={selected.has(slug)}
              onClick={() => toggle(slug)}
              className={`px-2.5 py-1 rounded-full text-xs border transition ${
                selected.has(slug)
                  ? "bg-spectyn-primary/15 border-spectyn-primary/40 text-spectyn-primary"
                  : "bg-spectyn-bg border-spectyn-border text-spectyn-text hover:border-spectyn-primary/30"
              }`}
            >
              {slug}
            </button>
          ))}
        </div>
        <button
          onClick={() => void run()}
          disabled={planning || peers.length === 0}
          className="flex items-center gap-2 bg-spectyn-primary text-spectyn-bg px-4 py-2 rounded-lg text-sm font-medium hover:brightness-110 disabled:opacity-40 transition"
        >
          {planning ? <Loader2 size={15} className="animate-spin" /> : <Send size={15} />}
          規劃派工
        </button>
        {peers.length === 0 && (
          <p className="text-[11px] text-spectyn-muted">尚無連線的 peer — 連上叢集後才能規劃。</p>
        )}
      </div>

      {error && (
        <div className="bg-spectyn-warning/10 border border-spectyn-warning/40 rounded-lg p-3 text-sm text-spectyn-warning">
          {error}
        </div>
      )}

      {plan?.selectedPeerId && (
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-4 space-y-3">
          <div className="flex items-center gap-2">
            <span className="text-xs text-spectyn-muted">選定 peer</span>
            <ArrowRight size={14} className="text-spectyn-muted" />
            <span className="text-sm font-semibold text-spectyn-primary">{nameOf(plan.selectedPeerId)}</span>
          </div>
          {(plan.fallbackPeerIds ?? []).length > 0 && (
            <p className="text-xs text-spectyn-muted">
              後備：{(plan.fallbackPeerIds ?? []).map(nameOf).join("、")}
            </p>
          )}
          {plan.scoringReason && (
            <p className="text-xs text-spectyn-text bg-spectyn-bg border border-spectyn-border rounded p-2">
              {plan.scoringReason}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
