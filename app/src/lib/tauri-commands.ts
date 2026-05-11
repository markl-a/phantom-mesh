// Typed wrappers around `safeInvoke` for the Phase 2 Tauri commands.
//
// All command shapes are defined in `./phase2-types`; this file is the
// single import point so component code never has to remember the raw
// command name or the wire shape.
//
// If a command's JSON shape changes on the Rust side, update
// `phase2-types.ts` first; this file generally stays the same.

import { safeInvoke } from './tauri-compat';
import type {
  CostSummary,
  ProjectInfo,
  ProviderInfo,
  SendMessageResult,
} from './phase2-types';

// Re-export so callers can `import { ProjectInfo } from './tauri-commands'`
// without a second import line — small ergonomic win for component code.
export type { CostSummary, ProjectInfo, ProviderInfo, SendMessageResult };

// ── Agent ─────────────────────────────────────────────────────────────────

export const sendMessage = (prompt: string, agent?: string) =>
  safeInvoke<SendMessageResult>('send_message', { prompt, agent });

// ── Project ───────────────────────────────────────────────────────────────

export const getProjectInfo = () =>
  safeInvoke<ProjectInfo>('get_project_info');

export const setProjectCwd = (path: string) =>
  safeInvoke<ProjectInfo>('set_project_cwd', { path });

export const listRecentProjects = () =>
  safeInvoke<ProjectInfo[]>('list_recent_projects');

export const addRecentProject = (cwd: string) =>
  safeInvoke<void>('add_recent_project', { cwd });

// ── Settings / Providers ──────────────────────────────────────────────────

export const readAgentsToml = () =>
  safeInvoke<string>('read_agents_toml');

export const writeAgentsToml = (content: string) =>
  safeInvoke<void>('write_agents_toml', { content });

export const getProviders = () =>
  safeInvoke<ProviderInfo[]>('get_providers');

export const setProviderApiKey = (provider_name: string, api_key: string) =>
  safeInvoke<void>('set_provider_api_key', { provider_name, api_key });

// ── Conversations ─────────────────────────────────────────────────────────

export const getConversations = () =>
  safeInvoke<{ active_sessions: number }>('get_conversations');

// ── Costs ─────────────────────────────────────────────────────────────────

export const getCosts = () =>
  safeInvoke<CostSummary>('get_costs');
