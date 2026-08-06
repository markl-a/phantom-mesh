import { useEffect, useRef } from "react";
import { useSystemHealth, CheckStatus } from "../hooks/useSystemHealth";
import { clearSession } from "./onboarding/types";

interface Props {
  onPass: () => void;
  onResetOnboarding: () => void;
}

function StatusIcon({ status }: { status: CheckStatus }) {
  switch (status) {
    case "checking":
      return <div className="w-5 h-5 border-2 border-spectyn-primary border-t-transparent rounded-full animate-spin" />;
    case "pass":
      return <div className="w-5 h-5 rounded-full bg-spectyn-success flex items-center justify-center text-white text-xs">✓</div>;
    case "fail":
      return <div className="w-5 h-5 rounded-full bg-spectyn-danger flex items-center justify-center text-white text-xs">✗</div>;
    case "warn":
      return <div className="w-5 h-5 rounded-full bg-amber-500 flex items-center justify-center text-white text-xs">!</div>;
    default:
      return <div className="w-5 h-5 rounded-full bg-spectyn-border" />;
  }
}

export default function StartupCheck({ onPass, onResetOnboarding }: Props) {
  const { checks, overallStatus, runCheck } = useSystemHealth();
  const advancedRef = useRef(false);

  useEffect(() => {
    runCheck();
  }, [runCheck]);

  useEffect(() => {
    // Auto-advance once when healthy. Guard with a ref (NOT state): a state
    // guard puts itself in the dep array, so flipping it re-runs this effect
    // and the cleanup clears the 800ms timer before onPass ever fires —
    // leaving the user stuck on the "正在進入..." screen.
    if (overallStatus === "healthy" && !advancedRef.current) {
      advancedRef.current = true;
      const timer = setTimeout(() => { onPass(); }, 800);
      return () => clearTimeout(timer);
    }
  }, [overallStatus, onPass]);

  const hasFail = checks.runtime.status === "fail" || checks.providers.status === "fail" || checks.llm.status === "fail";

  return (
    <div className="h-screen bg-spectyn-bg flex flex-col items-center justify-center p-8">
      <div className="w-full max-w-md">
        <div className="text-center mb-8">
          <h1 className="text-2xl font-bold text-spectyn-text mb-2">系統自檢</h1>
          <p className="text-spectyn-muted text-sm">
            {overallStatus === "checking" && "正在檢查系統狀態..."}
            {overallStatus === "healthy" && "一切正常，正在進入..."}
            {overallStatus === "degraded" && "部分功能異常"}
            {overallStatus === "unhealthy" && "系統無法正常運作"}
          </p>
        </div>

        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-5 space-y-4 mb-6">
          {/* Runtime */}
          <div className="flex items-center gap-3">
            <StatusIcon status={checks.runtime.status} />
            <div className="flex-1">
              <p className="text-sm text-spectyn-text">Runtime 引擎</p>
              <p className="text-xs text-spectyn-muted">{checks.runtime.message}</p>
            </div>
          </div>

          {/* Providers */}
          <div className="flex items-center gap-3">
            <StatusIcon status={checks.providers.status} />
            <div className="flex-1">
              <p className="text-sm text-spectyn-text">LLM Providers</p>
              <p className="text-xs text-spectyn-muted">{checks.providers.message}</p>
            </div>
          </div>

          {/* LLM */}
          <div className="flex items-center gap-3">
            <StatusIcon status={checks.llm.status} />
            <div className="flex-1">
              <p className="text-sm text-spectyn-text">LLM 連線測試</p>
              <p className="text-xs text-spectyn-muted">{checks.llm.message}</p>
            </div>
          </div>
        </div>

        {/* Actions when failed */}
        {hasFail && overallStatus !== "checking" && (
          <div className="space-y-2">
            <button
              onClick={() => runCheck()}
              className="w-full bg-spectyn-primary text-spectyn-bg py-2.5 rounded-lg font-medium text-sm hover:brightness-110 transition"
            >
              重新檢查
            </button>
            <button
              onClick={() => {
                clearSession();
                onResetOnboarding();
              }}
              className="w-full bg-spectyn-card border border-spectyn-border text-spectyn-text py-2.5 rounded-lg text-sm hover:border-spectyn-primary/50 transition"
            >
              重新設定（Onboarding）
            </button>
            <button
              onClick={onPass}
              className="w-full text-spectyn-muted text-xs py-2 hover:text-spectyn-text transition"
            >
              跳過，強制進入主介面
            </button>
          </div>
        )}

        {/* All pass */}
        {overallStatus === "healthy" && (
          <div className="space-y-2">
            <button
              onClick={onPass}
              className="w-full bg-spectyn-primary text-spectyn-bg py-2.5 rounded-lg font-medium text-sm hover:brightness-110 transition"
            >
              進入主介面
            </button>
            <p className="text-center text-spectyn-success text-xs">系統正常</p>
          </div>
        )}

        {/* Degraded but not critical */}
        {overallStatus === "degraded" && !hasFail && (
          <div className="space-y-2">
            <button
              onClick={onPass}
              className="w-full bg-spectyn-primary text-spectyn-bg py-2.5 rounded-lg font-medium text-sm hover:brightness-110 transition"
            >
              繼續進入主介面
            </button>
            <button
              onClick={() => runCheck()}
              className="w-full text-spectyn-muted text-xs py-2 hover:text-spectyn-text transition"
            >
              重新檢查
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
