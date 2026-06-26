// Desktop bridge to the life-partner brain (NORTH-STAR Q2, partner MVP §B).
//
// Two thin wrappers the macOS UI uses:
//   1. sendToPartner() — POST /partner/message through the SAME clusterPost as
//      the mobile app, so 記東西 (記:…) + 查資料 behave on the desktop chat
//      exactly like the partner core. Reuses the existing HMAC-signed transport.
//   2. loadLatestReflection() — read the most recent daily alignment reflection
//      the coach daemon produced, via the offline `partner_latest_reflection`
//      Tauri command (no HTTP endpoint exists for it).

import { clusterPost } from "./clusterDispatch";
import { safeInvoke as invoke } from "./tauri-compat";

/** Parsed reply from the partner brain's /partner/message endpoint. */
export interface PartnerReply {
  reply: string;
  turns?: number;
  elapsedSecs?: number;
  deduped?: boolean;
}

/**
 * Send `text` to the partner brain and return its real reply. Routes through
 * clusterPost (HMAC-signed body, native transport) so it matches the partner
 * core contract — `記:…` captures a note, anything else runs an agent turn.
 * Throws on a transport failure or a non-2xx status so the caller can surface
 * the error; the partner endpoint requires the cluster secret.
 */
export async function sendToPartner(
  baseUrl: string,
  secret: string,
  text: string,
): Promise<PartnerReply> {
  const r = await clusterPost(baseUrl, secret, "/partner/message", { text });
  if (!r.ok) {
    const detail = (r.json as { error?: string } | undefined)?.error ?? r.text;
    throw new Error(`partner/message ${r.status}: ${detail?.slice(0, 200) || "(empty)"}`);
  }
  const j = (r.json ?? {}) as {
    reply?: string;
    turns?: number;
    elapsed_secs?: number;
    deduped?: boolean;
  };
  return {
    reply: j.reply ?? r.text ?? "",
    turns: j.turns,
    elapsedSecs: j.elapsed_secs,
    deduped: j.deduped,
  };
}

/** The latest daily alignment reflection, mirrors the Rust LatestReflection. */
export interface LatestReflection {
  text: string;
  summary: string;
  ts: number;
}

/**
 * Load the most recent daily alignment reflection the coach daemon produced
 * (read-only, offline — reads ~/.phantom-mesh/partner-signals.jsonl via the
 * `partner_latest_reflection` Tauri command). Returns null when none exists yet
 * (fresh install before the first 21:00 coach run) or in web/browser mode where
 * the command is unwired. Never throws.
 */
export async function loadLatestReflection(): Promise<LatestReflection | null> {
  try {
    const res = await invoke<LatestReflection | null>("partner_latest_reflection");
    if (!res || typeof (res as LatestReflection).text !== "string") return null;
    return res as LatestReflection;
  } catch {
    return null;
  }
}
