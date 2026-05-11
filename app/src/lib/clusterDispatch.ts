// Cluster-mode dispatch: thin client (mobile) → coordinator's
// /rpc/task/assign, then polls /rpc/task/status/:id until done | error.
//
// Wire format (matches core/src/serve.rs::rpc_task_assign and
// core/src/mesh.rs::make_auth_token_bytes):
//
//   POST /rpc/task/assign
//     Header: X-Cluster-Auth: hex(HMAC-SHA256(cluster_secret, body))
//     Body:   { "agent": "...", "prompt": "..." }
//     Reply:  202 { "job_id": "<uuid>" }
//
//   GET /rpc/task/status/:id
//     Reply:  { "job_id", "status": "running"|"done"|"error",
//               "output"?: string, "error"?: string }

export interface DispatchToClusterArgs {
  coordinatorUrl: string;
  secret: string;
  agent: string;
  prompt: string;
  maxWaitMs?: number;
  pollIntervalMs?: number;
}

export interface DispatchResult {
  ok: boolean;
  output?: string;
  error?: string;
  jobId?: string;
  elapsedMs?: number;
}

async function hmacSha256Hex(secret: string, body: string): Promise<string> {
  const enc = new TextEncoder();
  const key = await crypto.subtle.importKey(
    "raw",
    enc.encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = await crypto.subtle.sign("HMAC", key, enc.encode(body));
  return Array.from(new Uint8Array(sig))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

export async function dispatchToCluster(
  args: DispatchToClusterArgs,
): Promise<DispatchResult> {
  const {
    coordinatorUrl,
    secret,
    agent,
    prompt,
    maxWaitMs = 60_000,
    pollIntervalMs = 500,
  } = args;

  if (!coordinatorUrl) return { ok: false, error: "coordinator URL missing" };
  if (!secret) return { ok: false, error: "cluster secret missing" };

  const started = performance.now();
  const base = coordinatorUrl.replace(/\/+$/, "");

  const body = JSON.stringify({ agent, prompt });
  const auth = await hmacSha256Hex(secret, body);

  let jobId: string;
  try {
    const r = await fetch(`${base}/rpc/task/assign`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-Cluster-Auth": auth,
      },
      body,
    });
    if (!r.ok) {
      const txt = await r.text().catch(() => "");
      return { ok: false, error: `assign ${r.status}: ${txt || r.statusText}` };
    }
    const j = (await r.json()) as { job_id?: string; error?: string };
    if (!j.job_id) {
      return { ok: false, error: j.error || "no job_id in response" };
    }
    jobId = j.job_id;
  } catch (e) {
    return { ok: false, error: `assign: ${String(e)}` };
  }

  while (performance.now() - started < maxWaitMs) {
    await new Promise((res) => setTimeout(res, pollIntervalMs));
    try {
      const r = await fetch(`${base}/rpc/task/status/${jobId}`);
      if (!r.ok) continue;
      const j = (await r.json()) as {
        status?: string;
        output?: string;
        error?: string;
      };
      if (j.status === "done") {
        return {
          ok: true,
          output: j.output ?? "",
          jobId,
          elapsedMs: Math.round(performance.now() - started),
        };
      }
      if (j.status === "error") {
        return {
          ok: false,
          error: j.error || "task failed",
          jobId,
          elapsedMs: Math.round(performance.now() - started),
        };
      }
    } catch {
      // transient network blip; keep polling
    }
  }

  return {
    ok: false,
    error: `timeout after ${maxWaitMs}ms`,
    jobId,
    elapsedMs: Math.round(performance.now() - started),
  };
}
