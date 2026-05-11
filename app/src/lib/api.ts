import { safeInvoke } from "./tauri-compat";
import type {
  HealthResponse,
  AgentRunResponse,
  CostsResponse,
  PeersResponse,
  ConversationListResponse,
  ConversationHistoryResponse,
  ConversationResetResponse,
  VersionResponse,
} from "./types";

// ── Generic low-level invoke wrapper ────────────────────────────────────────

export async function api<T = unknown>(
  command: string,
  args?: Record<string, unknown>
): Promise<T> {
  return safeInvoke<T>(command, args);
}

// ── Error handling wrapper ───────────────────────────────────────────────────

export async function apiCall<T>(
  fn: () => Promise<T>
): Promise<{ data: T; error: null } | { data: null; error: string }> {
  try {
    const data = await fn();
    return { data, error: null };
  } catch (e) {
    return { data: null, error: e instanceof Error ? e.message : String(e) };
  }
}

// ── Health / Version ─────────────────────────────────────────────────────────

export async function getHealth(): Promise<HealthResponse> {
  return api<HealthResponse>("get_health");
}

export async function getVersion(): Promise<VersionResponse> {
  return api<VersionResponse>("get_version");
}

// ── Conversations ────────────────────────────────────────────────────────────

export async function listConversations(): Promise<ConversationListResponse> {
  return api<ConversationListResponse>("list_conversations");
}

export async function getConversationHistory(
  chat_id: string
): Promise<ConversationHistoryResponse> {
  return api<ConversationHistoryResponse>("get_conversation_history", { chat_id });
}

export async function resetConversation(
  chat_id: string
): Promise<ConversationResetResponse> {
  return api<ConversationResetResponse>("reset_conversation", { chat_id });
}

// ── Costs ────────────────────────────────────────────────────────────────────

export async function getCosts(): Promise<CostsResponse> {
  return api<CostsResponse>("get_costs");
}

// ── Peers / Network ──────────────────────────────────────────────────────────

export async function getPeers(): Promise<PeersResponse> {
  return api<PeersResponse>("get_peers");
}

// ── Agent Run ────────────────────────────────────────────────────────────────

export async function runAgent(
  name: string,
  prompt: string,
  chat_id?: string
): Promise<AgentRunResponse> {
  return api<AgentRunResponse>("send_message", {
    agent: name,
    prompt,
    ...(chat_id !== undefined ? { chat_id } : {}),
  });
}
