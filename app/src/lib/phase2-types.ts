// Phase 2 Tauri-bridge type definitions.
//
// These mirror the JSON shapes returned by the Rust commands in
// app/src-tauri/src/commands/{project,agents_config,agent,provider}.rs.
// All commands return `Result<Value, String>` on the Rust side, so these
// interfaces are the de-facto wire contract — keep them in sync when the
// underlying Rust serialization changes.

// ── Project ───────────────────────────────────────────────────────────────

/** Returned by `get_project_info` / `set_project_cwd` / `list_recent_projects`. */
export interface ProjectInfo {
  cwd: string;
  name: string;
  project_type: 'rust' | 'node' | 'python' | 'go' | 'unknown';
  has_git: boolean;
}

// ── Providers ─────────────────────────────────────────────────────────────

export interface ProviderInfo {
  name: string;
  type: string;
  default_model: string;
  has_api_key: boolean;
  api_key_env?: string;
}

// ── Streaming ─────────────────────────────────────────────────────────────

export interface ActiveTool {
  id: string;
  tool: string;
  args_preview: string;
  status: 'running' | 'done';
  output_preview?: string;
  startedAt: number;
}

export interface AgentTokenEvent {
  agent: string;
  content: string;
}

export interface AgentToolStartEvent {
  agent: string;
  tool: string;
  args_preview: string;
}

export interface AgentToolDoneEvent {
  agent: string;
  tool: string;
  output_preview: string;
}

export interface AgentDoneEvent {
  agent: string;
  output: string;
  cost_usd?: number;
  elapsed?: number;
}

// ── Diff ──────────────────────────────────────────────────────────────────

export interface DiffLine {
  type: 'add' | 'remove' | 'context';
  content: string;
  lineNo?: number;
}

export interface DiffHunk {
  header: string;
  lines: DiffLine[];
}

export interface FileDiff {
  path: string;
  hunks: DiffHunk[];
}

// ── Conversation ──────────────────────────────────────────────────────────

export interface ConversationSummary {
  id: string;
  lastMessage: string;
  messageCount: number;
  updatedAt: number;
}

// ── Cost ──────────────────────────────────────────────────────────────────

/** Returned by `get_costs` (proxies the broker's `/costs` endpoint). */
export interface CostSummary {
  total_usd: number;
  requests: number;
  prompt_tokens?: number;
  completion_tokens?: number;
  by_model?: Record<string, { usd: number; requests: number }>;
  by_provider?: Record<string, number>;
}

// ── send_message ──────────────────────────────────────────────────────────

/** Returned by `send_message`. */
export interface SendMessageResult {
  agent: string;
  output: string;
  tool_calls: number;
  elapsed: number;
  cost_usd: number;
}
