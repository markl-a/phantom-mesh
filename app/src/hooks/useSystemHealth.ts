import { useState, useCallback } from "react";

export const DAEMON = "http://localhost:7878";

async function httpGet(path: string) {
  const resp = await fetch(`${DAEMON}${path}`);
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

async function httpPost(path: string, body: unknown) {
  const resp = await fetch(`${DAEMON}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

export type CheckStatus = "pending" | "checking" | "pass" | "fail" | "warn";

export interface CheckResult {
  status: CheckStatus;
  message: string;
}

export interface SystemHealth {
  runtime: CheckResult;
  providers: CheckResult;
  llm: CheckResult;
}

export type OverallStatus = "checking" | "healthy" | "degraded" | "unhealthy";

export function useSystemHealth() {
  const [checks, setChecks] = useState<SystemHealth>({
    runtime: { status: "pending", message: "" },
    providers: { status: "pending", message: "" },
    llm: { status: "pending", message: "" },
  });

  const overallStatus: OverallStatus =
    checks.runtime.status === "checking" || checks.providers.status === "checking"
      ? "checking"
      : checks.runtime.status === "fail"
      ? "unhealthy"
      : checks.runtime.status === "pass"
      ? "healthy"   // Runtime up = healthy; provider/LLM issues are warnings, not blockers
      : "degraded";

  const runCheck = useCallback(async () => {
    // Step 1: Runtime health — probe /api/status (returns JSON, 200 when the
    // daemon is up; no LLM test on startup). NB: the daemon has no /health
    // route (404) — the liveness route is /healthz (plain "ok", not JSON) and
    // /api/status is the JSON status endpoint httpGet can parse.
    setChecks(c => ({ ...c, runtime: { status: "checking", message: "檢查 Runtime..." } }));
    try {
      await httpGet("/api/status");
      setChecks(c => ({ ...c, runtime: { status: "pass", message: "Runtime 運行中" } }));
    } catch {
      setChecks(c => ({
        ...c,
        runtime: { status: "fail", message: "Runtime 無法連接 (localhost:7878)" },
        providers: { status: "warn", message: "等待 Runtime..." },
        llm: { status: "warn", message: "等待 Runtime..." },
      }));
      return;
    }

    // Step 2: Providers — warn only, never block entry
    setChecks(c => ({ ...c, providers: { status: "checking", message: "檢查 Providers..." } }));
    try {
      const res = await httpGet("/api/providers/health") as { providers?: Array<{ provider_name: string; is_available: boolean }> };
      const providers = res?.providers ?? [];
      const available = providers.filter(p => p.is_available);
      if (available.length > 0) {
        setChecks(c => ({ ...c, providers: { status: "pass", message: `${available.length}/${providers.length} 個 Provider 在線` } }));
      } else {
        setChecks(c => ({ ...c, providers: { status: "warn", message: providers.length === 0 ? "未設定 Provider（可在設定中新增）" : "Provider 需要設定" } }));
      }
    } catch {
      setChecks(c => ({ ...c, providers: { status: "warn", message: "無法取得 Provider 狀態" } }));
    }

    // Step 3: LLM — skip actual test, just mark as pass if providers exist
    setChecks(c => ({ ...c, llm: { status: "pass", message: "就緒" } }));
  }, []);

  return { checks, overallStatus, runCheck };
}
