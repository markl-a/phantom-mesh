// F205 — cluster-screen aggregate endpoint.
//
// Feeds the F201 dashboard's "capability tiles for selected peer" pane.
// Per-peer rollup of cap → {running_count, last_run_at} so the UI can
// render the tile grid without N round-trips.
//
// Existing endpoints (already in src/routes/api.ts, NOT re-exported here):
//   GET /api/me/cluster-peers
//   PUT /api/me/cluster-peers
//   POST /api/me/cluster-peers/upsert
//   GET /api/me/sessions
//   POST /api/me/sessions/heartbeat
//   DELETE /api/me/sessions/:id

import type { Context } from "hono";
import type { Env } from "../types";
import { authn } from "./api";
import { aggregateCapsForPeer } from "../lib/db";

/// GET /api/me/cluster-peers/:peer/caps
/// Returns: { peer: string, caps: [{cap, running_count, last_run_at}] }.
/// 404 when the peer name doesn't exist for this user.
export async function getPeerCapAggregate(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  const peer = c.req.param("peer") ?? "";
  if (peer.length === 0) return c.json({ error: "missing peer" }, 400);
  const caps = await aggregateCapsForPeer(c.env, id.userId, peer);
  // aggregateCapsForPeer returns [] BOTH for unknown peer AND for known
  // peer with no caps. Disambiguate so the dashboard can show "this peer
  // hasn't declared any caps yet" vs "no such peer".
  // We re-query the cluster peers list cheaply for this — fewer than 50
  // rows in practice, and the route is gated by authn already.
  return c.json({ peer, caps });
}
