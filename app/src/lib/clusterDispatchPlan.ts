// Helper for SPEC-26 cluster dispatch planning — wraps the Tauri commands in
// `app/src-tauri/src/commands/cluster_dispatch_wire.rs`:
//   - dispatch_plan(task, peers)      → DispatchPlan
//   - dispatch_score_peer(peer, task) → PeerScore
//
// Unlike the capture wires, plan_dispatch is fully-wired deterministic
// capability-tag matching, so this returns a real plan. Maps the F101
// PeerSummary (string caps) into the wire-shape PeerCapabilities.

import { safeInvoke as invoke } from "./tauri-compat";
import type { DispatchTask } from "./generated/cluster_dispatch/DispatchTask";
import type { DispatchPlan } from "./generated/cluster_dispatch/DispatchPlan";
import type { CapabilityTag } from "./generated/cluster_dispatch/CapabilityTag";
import type { PeerCapabilities } from "./generated/cluster_dispatch/PeerCapabilities";
import type { PeerSummary } from "../stores/clusterPeersStore";

/** Common capability slugs a task can require (SPEC-26 §6.2 examples). */
export const CAP_OPTIONS = [
  "cargo", "cargo-test", "rust-edit", "tauri-build", "git",
  "webSearch", "gpu", "role-coder", "role-researcher", "always-on",
] as const;

function tag(slug: string): CapabilityTag {
  return { slug, value: null };
}

/** Build a DispatchTask from a set of required capability slugs. */
export function buildTask(requiredSlugs: string[]): DispatchTask {
  return {
    taskId: globalThis.crypto?.randomUUID?.() ?? `task-${Date.now()}`,
    requiredCaps: requiredSlugs.map(tag),
    preferredCaps: [],
    // payload is a String field on the wire (SPEC-26 §7); planning ignores its
    // content, so Stage-1 callers send the literal "null" string.
    payload: "null",
    deadlineMs: null,
  };
}

/** Project a F101 PeerSummary into the dispatch-wire PeerCapabilities shape. */
export function peerToCaps(p: PeerSummary): PeerCapabilities {
  return {
    peerId: p.peer_id,
    tags: (p.caps ?? []).map(tag),
    lastReportedAt: BigInt((p.last_seen_unix ?? 0) * 1000),
  };
}

/** Plan a dispatch: which peer would handle this task + fallbacks + reason. */
export async function planDispatch(
  task: DispatchTask,
  peers: PeerCapabilities[],
): Promise<DispatchPlan> {
  // Each peer's `lastReportedAt` is a BigInt (ts-rs) → JSON-incompatible in Tauri
  // invoke ("Do not know how to serialize a BigInt"). Coerce to number on the wire
  // (unix-ms « 2^53). Same BigInt-invoke class as onboarding/focus/food/habit.
  const wirePeers = peers.map((p) => ({ ...p, lastReportedAt: Number(p.lastReportedAt) }));
  return invoke<DispatchPlan>("dispatch_plan", { task, peers: wirePeers });
}

/** Map a dispatch wire error to a UI string. */
export function describeDispatchError(err: unknown): string {
  const s = String(err ?? "").trim();
  if (s.toLowerCase().includes("nomatchingpeer") || s.toLowerCase().includes("no matching"))
    return "沒有符合所需能力的 peer";
  if (s.startsWith("dispatch.not_yet_wired")) return "派工規劃後端暫時無法使用";
  return s || "未知錯誤";
}
