// F205 — dispatch history routes (consumed by F203 History screen).

import type { Context } from "hono";
import type { Env } from "../types";
import { authn } from "./api";
import { getDispatch, listDispatches } from "../lib/db";

/// GET /api/me/dispatches?peer=&cap=&status=&from=&to=&q=&page=&page_size=
export async function listHistory(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  const url = new URL(c.req.url);
  const q = url.searchParams;
  const page = parseIntOrUndef(q.get("page"));
  const page_size = parseIntOrUndef(q.get("page_size"));
  const from = parseIntOrUndef(q.get("from"));
  const to = parseIntOrUndef(q.get("to"));
  const res = await listDispatches(c.env, id.userId, {
    peer: q.get("peer") ?? undefined,
    status: q.get("status") ?? undefined,
    cap: q.get("cap") ?? undefined,
    from, to,
    q: q.get("q") ?? undefined,
    page, page_size,
  });
  return c.json(res);
}

/// GET /api/me/dispatches/:job_id
export async function getHistoryItem(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  const job_id = c.req.param("job_id") ?? "";
  if (!job_id) return c.json({ error: "missing job_id" }, 400);
  const row = await getDispatch(c.env, id.userId, job_id);
  if (!row) return c.json({ error: "not found" }, 404);
  return c.json({ dispatch: row });
}

/// GET /api/me/dispatches/export?... — streams NDJSON, one row per line.
export async function exportHistory(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  const url = new URL(c.req.url);
  const q = url.searchParams;
  // Bounded export so a runaway request can't OOM the worker. Caller
  // paginates via `from`/`to` for >5k rows; we cap each call at 5k.
  const res = await listDispatches(c.env, id.userId, {
    peer: q.get("peer") ?? undefined,
    status: q.get("status") ?? undefined,
    cap: q.get("cap") ?? undefined,
    from: parseIntOrUndef(q.get("from")),
    to: parseIntOrUndef(q.get("to")),
    q: q.get("q") ?? undefined,
    page: 1,
    page_size: 5000,
  });
  const body = res.rows.map(r => JSON.stringify(r)).join("\n") + (res.rows.length > 0 ? "\n" : "");
  return new Response(body, {
    headers: {
      "Content-Type": "application/x-ndjson",
      "Content-Disposition": `attachment; filename="dispatches.ndjson"`,
    },
  });
}

function parseIntOrUndef(s: string | null): number | undefined {
  if (s === null || s.length === 0) return undefined;
  const n = Number.parseInt(s, 10);
  return Number.isFinite(n) ? n : undefined;
}
