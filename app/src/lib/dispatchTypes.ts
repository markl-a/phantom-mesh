// F103 · Wire types for dispatch SSE frames.
//
// Mirrors `enum DispatchFrame` in app/src-tauri/src/commands/dispatch.rs.
// Single source of truth lives in the Rust side; this file is the
// hand-mirrored TS view. Frame payloads come over the
// `dispatch::token::<id>` Tauri event channel.

export type DispatchFrame =
  | { type: 'token'; text: string }
  | { type: 'status'; phase: string }
  | { type: 'done'; result: string }
  | { type: 'error'; code: string; message: string };
// Unknown variants forwarded by Rust as `Other(serde_json::Value)` arrive at
// runtime with `type` set to whatever string the broker sent (e.g.
// "heartbeat"). The store's switch `default:` arm absorbs them. We do NOT add
// a catch-all union member here because it widens narrowed types (`frame.text`
// becomes `unknown`) and breaks `tsc -b`.

export interface DispatchHandle {
  dispatch_id: string;
  started_at_unix: number;
}

export interface DispatchProvider {
  name: string;
  configured: boolean;
}

/** Capability chips offered in the F103 UI. Default empty = "no constraint". */
export const AVAILABLE_CAPS: readonly string[] = [
  'gpu',
  'vision',
  'camera',
  'audio',
  'gps',
] as const;

export const MAX_CAPS_SELECTABLE = 3;
