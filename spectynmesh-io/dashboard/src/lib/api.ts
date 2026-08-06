// Thin fetch wrapper for the dashboard SPA. Responsibilities:
//   - attach `Authorization: Bearer <broker_token>` when one is stored
//   - always send credentials so the HttpOnly spectyn_session cookie rides
//   - JSON-serialize plain-object bodies (+ Content-Type)
//   - surface non-2xx as a typed ApiError carrying the parsed body
//   - on 401, clear the stale mirror token + bounce to /login (opt-out via
//     redirectOn401:false so a first-paint probe can drive its own UX)
//
// Contract is pinned by tests/api.test.ts — keep them in sync.

import { clearBrokerLogin, getBearerToken } from "./auth";

export class ApiError extends Error {
  readonly status: number;
  readonly body: unknown;
  constructor(status: number, body: unknown, message?: string) {
    super(message ?? `request failed with status ${status}`);
    this.name = "ApiError";
    this.status = status;
    this.body = body;
  }
}

export interface ApiOptions {
  method?: string;
  /** Plain object → JSON body; string → sent verbatim. */
  body?: unknown;
  headers?: HeadersInit;
  signal?: AbortSignal;
  /** Default true: a 401 clears the token mirror and redirects to /login. */
  redirectOn401?: boolean;
}

function isJsonResponse(res: Response): boolean {
  return (res.headers.get("Content-Type") ?? "").includes("application/json");
}

async function parseBody(res: Response): Promise<unknown> {
  if (res.status === 204) return undefined;
  return isJsonResponse(res) ? res.json() : res.text();
}

export async function apiFetch<T = unknown>(
  path: string,
  opts: ApiOptions = {},
): Promise<T | undefined> {
  const { method, body, headers: extraHeaders, signal, redirectOn401 } = opts;

  const headers = new Headers(extraHeaders);
  const token = getBearerToken();
  if (token) headers.set("Authorization", `Bearer ${token}`);

  const init: RequestInit = {
    method: method ?? (body !== undefined ? "POST" : "GET"),
    headers,
    credentials: "include",
    signal,
  };

  if (body !== undefined) {
    if (typeof body === "string") {
      init.body = body;
    } else {
      init.body = JSON.stringify(body);
      if (!headers.has("Content-Type")) {
        headers.set("Content-Type", "application/json");
      }
    }
  }

  const res = await fetch(path, init);

  if (res.status === 401) {
    const parsed = await parseBody(res).catch(() => undefined);
    if (redirectOn401 !== false) {
      clearBrokerLogin();
      const next = encodeURIComponent(
        window.location.pathname + window.location.search,
      );
      window.location.replace(`/login?next=${next}`);
    }
    throw new ApiError(401, parsed);
  }

  if (!res.ok) {
    const parsed = await parseBody(res).catch(() => undefined);
    throw new ApiError(res.status, parsed);
  }

  if (res.status === 204) return undefined;
  return (await parseBody(res)) as T;
}

export function apiGet<T = unknown>(
  path: string,
  opts: Omit<ApiOptions, "method" | "body"> = {},
): Promise<T | undefined> {
  return apiFetch<T>(path, { ...opts, method: "GET" });
}

export function apiPost<T = unknown>(
  path: string,
  body?: unknown,
  opts: Omit<ApiOptions, "method" | "body"> = {},
): Promise<T | undefined> {
  return apiFetch<T>(path, { ...opts, method: "POST", body });
}
