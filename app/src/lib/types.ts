export interface ToolCall {
  name: string;
  status?: "pending" | "running" | "done" | "error";
  args?: Record<string, unknown>;
  result?: string;
}

// ── API Response Types ──────────────────────────────────────────────────────

export interface HealthResponse {
  status: string;
  version: string;
  uptime_seconds: number;
  service: string;
  mode: string;
}

export interface AgentRunResponse {
  agent: string;
  output: string;
  tool_calls: ToolCall[];
  elapsed: number;
}

export interface CostsResponse {
  total_usd: number;
  requests: number;
  prompt_tokens: number;
  completion_tokens: number;
}

export interface PeerNode {
  name: string;
  host?: string;
  online: boolean;
  active_tasks: number;
}

export interface PeersResponse {
  peers: PeerNode[];
  self: { name: string; version: string };
}

export interface ConversationMessage {
  role: string;
  content: string;
}

export interface ConversationHistoryResponse {
  chat_id?: string;
  messages: ConversationMessage[];
}

export interface ConversationListResponse {
  conversations: string[];
}

export interface ConversationResetResponse {
  chat_id: string;
  reset: boolean;
}

export interface VersionResponse {
  version: string;
  commit: string;
}

export interface Message {
  role: "user" | "assistant";
  content: string;
  tool_calls?: ToolCall[];
  /**
   * Optional provider slug (e.g. `groq`, `anthropic`) — populated when the
   * message was produced by the SPEC-14 providers wire. Used by MessageList /
   * MobileConversation to render a small attribution label on the AI bubble.
   */
  provider?: string;
  /**
   * Optional model id actually used by the provider (after fallback). Pairs
   * with `provider` for the attribution label.
   */
  model?: string;
}

export interface AgentEvent {
  type: "chunk" | "tool_call" | "tool_result" | "done" | "error";
  content?: string;
  tool_call?: ToolCall;
}

export interface DaemonInfo {
  healthy: boolean;
  version?: string;
}

export interface ConversationInfo {
  id: string;
  message_count?: number;
  last_message?: string;
}

export type TaskStatus = "pending" | "running" | "done" | "failed" | "cancelled";

export interface TaskItem {
  id: string;
  title: string;
  status: TaskStatus;
  agent: string;
}

export interface CostRecord {
  provider?: string;
  name?: string;
  cost_usd?: number;
  cost?: number;
  total_cost?: number;
  tokens_in?: number;
  input_tokens?: number;
  prompt_tokens?: number;
  tokens_out?: number;
  output_tokens?: number;
  completion_tokens?: number;
}

export interface ClusterNode {
  name?: string;
  status?: string;
  host?: string;
  port?: number;
  role?: string;
  capabilities?: string[];
}
