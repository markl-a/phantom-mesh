// Cluster-mode dispatch: thin client (mobile) → coordinator's
// /rpc/task/assign, then polls /rpc/task/status/:id until done | error.
//
// IMPORTANT — we use `fetch` from `@tauri-apps/plugin-http`, NOT the
// browser/WKWebView `window.fetch`. On iOS the Tauri webview origin is
// `https://tauri.localhost`, so any http:// cluster coordinator URL is
// blocked as mixed content (silently — TypeError: Load failed) by
// WebKit regardless of NSAppTransportSecurity. The plugin's fetch goes
// through the native reqwest client and is exempt from browser CSP /
// mixed-content / preflight CORS rules.
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import { invoke } from "@tauri-apps/api/core";

/**
 * iOS-only escape hatch. tauri-plugin-http's reqwest backend silently
 * times out when fetching Tailscale magic hostnames + private IPs from
 * physical iOS devices — likely because reqwest uses raw sockets that
 * don't satisfy the iOS network sandbox. The `swift_cluster_fetch`
 * Tauri command (app/src-tauri/src/lib.rs + PhantomFetch.swift) routes
 * the request through native NSURLSession.dataTask which goes through
 * iOS's standard URL loading stack and works reliably.
 *
 * Returns the same {ok, status, text()} shape the browser fetch returns
 * so the rest of the file's call sites don't need conditional logic.
 */
async function nativeFetch(
  url: string,
  init: { method?: string; headers?: Record<string, string>; body?: string } = {},
): Promise<{ ok: boolean; status: number; statusText: string; text(): Promise<string>; json(): Promise<unknown> }> {
  const method = (init.method || "GET").toUpperCase();
  const auth = init.headers?.["X-Cluster-Auth"] || init.headers?.["x-cluster-auth"] || "";
  const body = init.body || "";
  const diag = (msg: string) => (window as { phantomDiag?: (m: string) => void }).phantomDiag?.(msg);
  diag(`[fetch] ${method} ${url.slice(0, 60)} body=${body.length}B`);
  try {
    const r = await invoke<{ status: number; body: string }>("swift_cluster_fetch", {
      url, method, body, auth,
    });
    diag(`[fetch] ← status=${r.status} body=${r.body.length}B body[0..80]=${r.body.slice(0,80)}`);
    return {
      ok: r.status >= 200 && r.status < 300,
      status: r.status,
      statusText: r.status < 0 ? "native-error" : "",
      text: async () => r.body,
      json: async () => JSON.parse(r.body),
    };
  } catch (e) {
    diag(`[fetch] ✗ invoke threw: ${String(e).slice(0, 120)}`);
    throw e;
  }
}

// User-Agent contains "Mobile" / "iPhone" / "iPad" on iOS; gate native bridge on that.
const isIOS = typeof navigator !== "undefined"
  && /iPad|iPhone|iPod/.test(navigator.userAgent || "");

const httpFetch = isIOS ? nativeFetch : tauriFetch;
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
    maxWaitMs = 120_000,
    pollIntervalMs = 1500,
  } = args;

  if (!coordinatorUrl) return { ok: false, error: "coordinator URL missing" };
  if (!secret) return { ok: false, error: "cluster secret missing" };

  const started = performance.now();
  const base = coordinatorUrl.replace(/\/+$/, "");

  const body = JSON.stringify({ agent, prompt });
  const auth = await hmacSha256Hex(secret, body);

  let jobId: string;
  try {
    const r = await httpFetch(`${base}/rpc/task/assign`, {
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
      const r = await httpFetch(`${base}/rpc/task/status/${jobId}`);
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
