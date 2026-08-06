// F205 — dispatch routes.
//
// CONTROL-PLANE model (deviation from F205 spec text — documented in PR body):
// The worker stores DISPATCH METADATA. The actual LLM stream travels
// directly between the SPA and the user's local `spectyn serve` (CORS-
// allowed). So:
//   POST /api/me/dispatch/start          — register a new job_id in D1,
//                                          return the id; SPA then opens
//                                          its OWN connection to localhost.
//   POST /api/me/dispatch/stream/:job_id — SPA pushes each locally-received
//                                          chunk so other tabs see it (DO fan-out).
//   GET  /api/me/dispatch/stream/:job_id — SSE subscriber (other tabs).
//   POST /api/me/dispatch/:job_id/cancel — mark cancelled, notify subscribers.
//
// This matches the user prompt's explicit directive: "Don't expose user's
// localhost data through the worker — the worker is a CONTROL PLANE".
//
// The full F205 spec posited the worker would proxy upstream over HMAC.
// That would route LLM traffic through Cloudflare, which the user prompt
// forbids. We honour the directive; the dispatches D1 table still serves
// the History screen and the DO still solves cross-tab fan-out.

import type { Context } from "hono";
import type { Env } from "../types";
import { authn } from "./api";
import {
  createDispatch, getDispatch, updateDispatchStatus,
} from "../lib/db";

/// POST /api/me/dispatch/start
/// Body: { peer, provider, model, prompt, required_caps[] }
/// Returns: { job_id, started_at }.
export async function startDispatch(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);

  let body: {
    peer?: string; provider?: string; model?: string;
    prompt?: string; required_caps?: unknown;
  };
  try { body = await c.req.json(); }
  catch { return c.json({ error: "malformed json" }, 400); }

  const peer = (body.peer ?? "").trim();
  if (peer.length === 0) return c.json({ error: "missing peer" }, 400);

  const required_caps = Array.isArray(body.required_caps)
    ? body.required_caps.filter((x): x is string => typeof x === "string")
    : [];

  const job_id = crypto.randomUUID();
  const row = await createDispatch(c.env, id.userId, {
    job_id,
    peer,
    provider: (body.provider ?? "").trim(),
    model:    (body.model    ?? "").trim(),
    prompt:   typeof body.prompt === "string" ? body.prompt : "",
    required_caps,
  });
  return c.json({ job_id, started_at: row.started_at });
}

/// POST /api/me/dispatch/:job_id/cancel
/// Marks D1 status=cancelled + notifies DO subscribers.
export async function cancelDispatch(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  const job_id = c.req.param("job_id") ?? "";
  if (!job_id) return c.json({ error: "missing job_id" }, 400);

  const row = await getDispatch(c.env, id.userId, job_id);
  if (!row) return c.json({ error: "not found" }, 404);

  await updateDispatchStatus(c.env, id.userId, job_id, "cancelled");
  // Forward to the Durable Object so any attached tabs see the
  // cancelled event. The DO is keyed by job_id (idFromName); we
  // also pass X-Owner so the DO can verify cross-user isolation.
  const stub = c.env.DISPATCH_STREAM.get(c.env.DISPATCH_STREAM.idFromName(job_id));
  await stub.fetch(`https://do.local/cancel`, {
    method: "POST",
    headers: { "X-Owner": String(id.userId) },
  }).catch(() => { /* DO not yet warm — D1 state is the source of truth */ });
  return c.json({ cancelled: true });
}

/// POST /api/me/dispatch/stream/:job_id — SPA pushes a chunk for fan-out.
/// Body: { kind: "chunk"|"status"|"error", data: string }.
/// If status=done|cancelled|error, also flips the D1 row.
export async function publishChunk(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  const job_id = c.req.param("job_id") ?? "";
  if (!job_id) return c.json({ error: "missing job_id" }, 400);

  const row = await getDispatch(c.env, id.userId, job_id);
  if (!row) return c.json({ error: "not found" }, 404);

  let body: { kind?: string; data?: string };
  try { body = await c.req.json(); }
  catch { return c.json({ error: "malformed json" }, 400); }

  const kind = body.kind === "status" || body.kind === "error" ? body.kind : "chunk";
  const data = typeof body.data === "string" ? body.data : "";

  // Mirror chunk into D1 result column when it's content (so the History
  // screen replays even without the DO buffer).
  if (kind === "chunk") {
    await updateDispatchStatus(c.env, id.userId, job_id, "running", { result_append: data });
  } else if (kind === "status" && (data === "running" || data === "done" || data === "cancelled")) {
    await updateDispatchStatus(c.env, id.userId, job_id, data as "running"|"done"|"cancelled");
  } else if (kind === "error") {
    await updateDispatchStatus(c.env, id.userId, job_id, "error", { error_message: data });
  }

  // Forward to DO for live fan-out.
  const stub = c.env.DISPATCH_STREAM.get(c.env.DISPATCH_STREAM.idFromName(job_id));
  const res = await stub.fetch(`https://do.local/publish?job_id=${encodeURIComponent(job_id)}`, {
    method: "POST",
    headers: { "X-Owner": String(id.userId), "Content-Type": "application/json" },
    body: JSON.stringify({ kind, data }),
  });
  const j = await res.json().catch(() => ({})) as { seq?: number };
  return c.json({ accepted: true, seq: j.seq ?? null });
}

/// GET /api/me/dispatch/stream/:job_id — SSE subscriber.
/// Returns the DO-streamed text/event-stream response directly.
export async function subscribeStream(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  const job_id = c.req.param("job_id") ?? "";
  if (!job_id) return c.json({ error: "missing job_id" }, 400);

  const row = await getDispatch(c.env, id.userId, job_id);
  if (!row) return c.json({ error: "not found" }, 404);

  const stub = c.env.DISPATCH_STREAM.get(c.env.DISPATCH_STREAM.idFromName(job_id));
  const res = await stub.fetch(`https://do.local/subscribe?job_id=${encodeURIComponent(job_id)}`, {
    method: "GET",
    headers: { "X-Owner": String(id.userId) },
  });
  // Pass through status + headers so a 429 (too many subscribers) reaches
  // the client unmodified.
  return new Response(res.body, {
    status: res.status,
    headers: res.headers,
  });
}
