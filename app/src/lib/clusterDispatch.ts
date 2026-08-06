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
 * Tauri command (app/src-tauri/src/lib.rs + SpectynFetch.swift) routes
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
  const diag = (msg: string) => (window as { spectynDiag?: (m: string) => void }).spectynDiag?.(msg);
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

// Plain-browser (Web build) escape hatch. tauriFetch (@tauri-apps/plugin-http)
// requires window.__TAURI_INTERNALS__; in a normal browser it is absent and any
// call throws "Cannot read properties of undefined (reading 'transformCallback')".
// So when there is no Tauri runtime we fall back to the standard window.fetch,
// wrapped to the same {ok,status,text(),json()} shape the other backends return.
// NOTE: in a plain browser the coordinator must allow CORS for the page origin
// (start serve with SPECTYN_CORS_ALLOW_LOCALHOST=1, or SPECTYN_CORS_ALLOW_ANY=1
// for a custom static port, or front it with a same-origin reverse proxy).
async function browserFetch(
  url: string,
  init: { method?: string; headers?: Record<string, string>; body?: string } = {},
): Promise<{ ok: boolean; status: number; statusText: string; text(): Promise<string>; json(): Promise<unknown> }> {
  const r = await window.fetch(url, {
    method: (init.method || "GET").toUpperCase(),
    headers: init.headers,
    body: init.body || undefined,
  });
  return {
    ok: r.ok,
    status: r.status,
    statusText: r.statusText,
    text: () => r.text(),
    json: () => r.json(),
  };
}

// User-Agent contains "Mobile" / "iPhone" / "iPad" on iOS; gate native bridge on that.
const isIOS = typeof navigator !== "undefined"
  && /iPad|iPhone|iPod/.test(navigator.userAgent || "");

// Real Tauri runtime (desktop / Android / iOS webview) exposes __TAURI_INTERNALS__;
// a plain browser (Web build) does not — route it through window.fetch instead.
const isTauri = typeof window !== "undefined"
  && "__TAURI_INTERNALS__" in (window as object);

const httpFetch = isIOS ? nativeFetch : isTauri ? tauriFetch : browserFetch;
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

/**
 * Low-level reusable POST that signs the raw body with HMAC-SHA256 and
 * routes through the SAME transport dispatchToCluster uses (native
 * NSURLSession on iOS via swift_cluster_fetch, tauri-plugin-http
 * elsewhere). Returns the parsed JSON plus the HTTP status so callers can
 * surface errors. Used by the minimal Demo screen and by dispatchToCluster.
 *
 *   path examples: "/partner/message", "/partner/signal"
 *   bodyObj is JSON.stringify'd; the EXACT serialized string is what gets
 *   HMAC-signed (server recomputes over the raw body, so order matters —
 *   we sign exactly what we send).
 */
export async function clusterPost(
  baseUrl: string,
  secret: string,
  path: string,
  bodyObj: unknown,
): Promise<{ ok: boolean; status: number; json: unknown; text: string }> {
  if (!baseUrl) throw new Error("base URL missing");
  if (!secret) throw new Error("cluster secret missing");
  const base = baseUrl.replace(/\/+$/, "");
  const body = JSON.stringify(bodyObj);
  const auth = await hmacSha256Hex(secret, body);
  const r = await httpFetch(`${base}${path}`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-Cluster-Auth": auth,
    },
    body,
  });
  const text = await r.text().catch(() => "");
  let json: unknown = undefined;
  try { json = text ? JSON.parse(text) : undefined; } catch { /* non-JSON body */ }
  return { ok: r.ok, status: r.status, json, text };
}

/**
 * Low-level GET that routes through the same native transport (NSURLSession
 * on iOS). Used for unauthenticated read endpoints like GET /rpc/peers
 * (live cluster status). No body, no HMAC — the endpoint is read-only.
 */
export async function clusterGet(
  baseUrl: string,
  path: string,
): Promise<{ ok: boolean; status: number; json: unknown; text: string }> {
  if (!baseUrl) throw new Error("base URL missing");
  const base = baseUrl.replace(/\/+$/, "");
  const r = await httpFetch(`${base}${path}`, { method: "GET" });
  const text = await r.text().catch(() => "");
  let json: unknown = undefined;
  try { json = text ? JSON.parse(text) : undefined; } catch { /* non-JSON body */ }
  return { ok: r.ok, status: r.status, json, text };
}

/**
 * Best-effort current location. On iOS routes through the native
 * `swift_get_location` Tauri command (CoreLocation), which returns
 * { lat, lon, accuracy, error }. On non-iOS falls back to the browser
 * geolocation API (10s timeout). On any failure returns lat/lon 0 with a
 * human-readable `error` so callers can degrade gracefully.
 */
export async function getLocation(): Promise<{
  lat: number;
  lon: number;
  accuracy: number;
  error?: string;
}> {
  if (isIOS) {
    try {
      const r = await invoke<{
        lat: number;
        lon: number;
        accuracy: number;
        error: string | null;
      }>("swift_get_location");
      return {
        lat: r.lat,
        lon: r.lon,
        accuracy: r.accuracy,
        error: r.error == null ? undefined : r.error,
      };
    } catch (e) {
      return { lat: 0, lon: 0, accuracy: -1, error: String(e) };
    }
  }

  if (typeof navigator !== "undefined" && navigator.geolocation) {
    try {
      return await new Promise((resolve) => {
        navigator.geolocation.getCurrentPosition(
          (pos) =>
            resolve({
              lat: pos.coords.latitude,
              lon: pos.coords.longitude,
              accuracy: pos.coords.accuracy ?? -1,
            }),
          (err) =>
            resolve({ lat: 0, lon: 0, accuracy: -1, error: err.message || "geolocation error" }),
          { enableHighAccuracy: true, timeout: 10_000, maximumAge: 0 },
        );
      });
    } catch {
      return { lat: 0, lon: 0, accuracy: -1, error: "geolocation unavailable" };
    }
  }

  return { lat: 0, lon: 0, accuracy: -1, error: "geolocation unavailable" };
}

/**
 * Best-effort phone sensor snapshot (iOS only). Routes through the native
 * `swift_get_sensors` Tauri command (CoreMotion + UIDevice) and returns the
 * raw JSON object: battery, accel/gyro/attitude/magnetometer, plus best-effort
 * steps_today / activity (walking/running/automotive/stationary). The AI partner
 * reads this as the "behaviour" half of sensing. Returns null on non-iOS or any
 * failure so callers degrade gracefully.
 */
export async function getSensors(): Promise<Record<string, unknown> | null> {
  if (!isIOS) return null;
  try {
    const raw = await invoke<string>("swift_get_sensors");
    return JSON.parse(raw) as Record<string, unknown>;
  } catch {
    return null;
  }
}

/**
 * LLM intent classifier. Asks the partner endpoint to classify a free-text
 * message into one of {chat, dispatch, sense, swarm} and (for dispatch)
 * optionally pick a target machine + extract the task. Routes through
 * clusterPost so it shares the native NSURLSession transport.
 *
 * GRACEFUL FALLBACK: on ANY failure (network, non-JSON reply, parse error,
 * unknown intent) this returns {intent:"chat"} so the app stays usable — it
 * NEVER throws.
 */
export async function classifyIntent(
  baseUrl: string,
  secret: string,
  text: string,
  machineLabels: string[],
): Promise<{ intent: "chat" | "dispatch" | "sense" | "swarm"; machine?: string; task?: string }> {
  const PROMPT = `你是 spectyn 的意圖分類器,只回一行 JSON,不要任何其他文字。
把使用者輸入分類成:
- "dispatch":想在「某一台」機器上執行程式碼/任務(寫程式、改檔、跑指令、派工、實作)
- "swarm":使用者想讓整個機隊/多台機器一起檢查或開發這個專案(例:「派機隊檢查 bug」「讓四台機器一起修」「檢查 spectyn-mesh 的 bug 跟未完成功能」「整個機隊一起開發」)
- "sense":詢問自己的所在位置、現場狀況、附近環境、是否到了某地點
- "chat":一般對話、陪伴、心情、提問、閒聊
可用機器:${machineLabels.join("、")}
只回這個格式:{"intent":"chat|dispatch|sense|swarm","machine":"<dispatch時從可用機器挑一個完整label,沒明確指定就空字串>","task":"<dispatch時要在機器上執行的任務,否則空字串>"}
使用者輸入:${text}`;
  try {
    // Dogfood-moat guard: this is a machine round-trip (the classifier), NOT a
    // human message. Tag it `origin:"machine"` so the server segregates it out of
    // the human-usage ledger (partner-signals.jsonl) — only genuine human chat
    // counts as "我真天天在用". See core/src/partner.rs MessageOrigin.
    const r = await clusterPost(baseUrl, secret, "/partner/message", {
      text: PROMPT,
      origin: "machine",
    });
    if (!r.ok) return { intent: "chat" };
    const reply = (r.json as { reply?: string } | undefined)?.reply ?? r.text;
    if (!reply) return { intent: "chat" };
    const m = reply.match(/\{[\s\S]*\}/);
    if (!m) return { intent: "chat" };
    const parsed = JSON.parse(m[0]) as { intent?: string; machine?: string; task?: string };
    if (
      parsed.intent !== "chat" &&
      parsed.intent !== "dispatch" &&
      parsed.intent !== "sense" &&
      parsed.intent !== "swarm"
    ) {
      return { intent: "chat" };
    }
    return {
      intent: parsed.intent,
      machine: typeof parsed.machine === "string" ? parsed.machine : "",
      task: typeof parsed.task === "string" ? parsed.task : "",
    };
  } catch {
    return { intent: "chat" };
  }
}

/**
 * Derive the orchestrator base URL from the configured cluster baseUrl.
 * The orchestrator runs on the same host as the coordinator but on port 7900
 * (the coordinator is on e.g. 7878). We replace any explicit :PORT with :7900,
 * or insert :7900 before any path if no port is present. Trailing slashes are
 * stripped.
 *
 *   "http://backend.example:7878"      → "http://backend.example:7900"
 *   "http://backend.example"           → "http://backend.example:7900"
 *   "http://backend.example:7878/x/"   → "http://backend.example:7900/x"
 */
export function orchBase(baseUrl: string): string {
  const trimmed = (baseUrl || "").replace(/\/+$/, "");
  // Split off scheme so we don't confuse "://" with a host:port colon.
  const schemeMatch = trimmed.match(/^([a-zA-Z]+:\/\/)?(.*)$/);
  const scheme = schemeMatch?.[1] ?? "";
  const rest = schemeMatch?.[2] ?? trimmed;
  // rest = host[:port][/path...]
  const slash = rest.indexOf("/");
  const authority = slash === -1 ? rest : rest.slice(0, slash);
  const path = slash === -1 ? "" : rest.slice(slash);
  // Strip an existing :port from the authority (host may itself contain no port).
  const host = authority.replace(/:\d+$/, "");
  return `${scheme}${host}:7900${path}`;
}

/**
 * Kick off a swarm job on the orchestrator. The orchestrator drives `claude`
 * across the fleet and exposes a progress feed. Read-only-style GET (no auth)
 * routed through the same native transport as clusterGet.
 *
 * Returns {jobId} on success or {error} on any failure (never throws).
 */
export async function swarmStart(
  orchBaseUrl: string,
  goal: string,
): Promise<{ jobId?: string; error?: string }> {
  try {
    const r = await clusterGet(orchBaseUrl, "/swarm/start?goal=" + encodeURIComponent(goal));
    if (!r.ok) {
      return { error: `start ${r.status}: ${r.text.slice(0, 160) || "(empty)"}` };
    }
    const jobId = (r.json as { job_id?: string } | undefined)?.job_id;
    if (!jobId) return { error: r.text ? `no job_id in: ${r.text.slice(0, 120)}` : "no job_id in response" };
    return { jobId };
  } catch (e) {
    return { error: String(e).slice(0, 160) };
  }
}

/**
 * Poll a swarm job's live feed. The orchestrator returns the FULL messages
 * array so far plus an overall status ("running" | "done" | "error"). On any
 * failure returns {status:"error", messages:[]} so the caller can stop cleanly.
 */
export async function swarmFeed(
  orchBaseUrl: string,
  jobId: string,
): Promise<{ status: string; messages: { machine: string; text: string }[] }> {
  try {
    const r = await clusterGet(orchBaseUrl, "/swarm/feed?job=" + encodeURIComponent(jobId));
    if (!r.ok) return { status: "error", messages: [] };
    const j = (r.json as { status?: string; messages?: unknown[] } | undefined) ?? {};
    const status = typeof j.status === "string" ? j.status : "running";
    const rawMsgs = Array.isArray(j.messages) ? j.messages : [];
    const messages = rawMsgs.map((m) => {
      const o = (m ?? {}) as Record<string, unknown>;
      return { machine: String(o.machine ?? ""), text: String(o.text ?? "") };
    });
    return { status, messages };
  } catch {
    return { status: "error", messages: [] };
  }
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
