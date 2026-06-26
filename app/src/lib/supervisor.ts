// P1-2 supervisor reads: thin, never-throwing parsers + fetch helpers that
// reuse the existing HMAC transport (clusterPost). These supervise the BACKEND
// node the phone points at (baseUrl/secret), distinct from the device-local
// daily_review Tauri commands.
//
// Wire shapes mirror core/src/serve.rs:
//   /rpc/tasks/list   → { tasks: TaskRecordWire[], pending: PendingCard[] }
//   /rpc/captures/recent → { captures: { event_id, timestamp, kind, tags }[] }
//   /rpc/review       → { date, markdown }
// (`kind` is the snake_case EventKind: "food" | "focus" | "habit" | "dispatch"
//  | "text" — matches KIND_EMOJI keys in lib/dailyReview.ts.)
import { clusterPost } from "./clusterDispatch";

export interface SupTask {
  id: string;
  agent: string;
  prompt: string;
  status: string;
  createdAt: number;
  costUsd: number;
  turns: number;
  error: string | null;
  output: string | null;
}
export interface SupPending {
  approvalId: string;
  taskId: string;
  tool: string;
  risk: string;
  reason: string;
  createdMs: number;
}
export interface SupCapture {
  id: string;
  timestamp: string;
  kind: string;
  tags: string[];
}
export interface SupReview {
  date: string;
  markdown: string;
}

const arr = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);
const str = (v: unknown): string => (typeof v === "string" ? v : "");
const num = (v: unknown): number => (typeof v === "number" ? v : 0);
const obj = (v: unknown): Record<string, unknown> =>
  v && typeof v === "object" ? (v as Record<string, unknown>) : {};

export function parseTasks(j: unknown): { tasks: SupTask[]; pending: SupPending[] } {
  const o = obj(j);
  const tasks = arr(o.tasks).map((t) => {
    const r = obj(t);
    return {
      id: str(r.task_id),
      agent: str(r.agent_name),
      prompt: str(r.prompt),
      status: str(r.status),
      createdAt: num(r.created_at),
      costUsd: num(r.cost_usd),
      turns: num(r.turns),
      error: typeof r.error === "string" ? r.error : null,
      output: typeof r.output === "string" ? r.output : null,
    };
  });
  const pending = arr(o.pending).map((p) => {
    const r = obj(p);
    return {
      approvalId: str(r.approval_id),
      taskId: str(r.task_id),
      tool: str(r.tool),
      risk: str(r.risk),
      reason: str(r.reason),
      createdMs: num(r.created_ms),
    };
  });
  return { tasks, pending };
}

export function parseCaptures(j: unknown): SupCapture[] {
  const o = obj(j);
  return arr(o.captures).map((c) => {
    const r = obj(c);
    // Backend emits `tags`; tolerate a `goal_tags` alias defensively.
    const rawTags = Array.isArray(r.tags) ? r.tags : r.goal_tags;
    return {
      id: str(r.event_id),
      timestamp: str(r.timestamp),
      kind: str(r.kind),
      tags: arr(rawTags).map(str).filter(Boolean),
    };
  });
}

export function parseReview(j: unknown): SupReview {
  const o = obj(j);
  return { date: str(o.date), markdown: str(o.markdown) };
}

export async function fetchTasks(baseUrl: string, secret: string) {
  const r = await clusterPost(baseUrl, secret, "/rpc/tasks/list", { limit: 50 });
  if (!r.ok) throw new Error(`tasks ${r.status}`);
  return parseTasks(r.json);
}
export async function fetchCaptures(baseUrl: string, secret: string) {
  const r = await clusterPost(baseUrl, secret, "/rpc/captures/recent", { limit: 50 });
  if (!r.ok) throw new Error(`captures ${r.status}`);
  return parseCaptures(r.json);
}
export async function fetchReview(baseUrl: string, secret: string, date?: string) {
  const r = await clusterPost(baseUrl, secret, "/rpc/review", date ? { date } : {});
  if (!r.ok) throw new Error(`review ${r.status}`);
  return parseReview(r.json);
}
