// Helper for the real on-device identity (BIG-GOAL P4). Wraps the read-only
// Tauri command `identity_status` (app/src-tauri/src/commands/identity_status.rs)
// which reports the per-device identity.key fingerprint — the actual
// cryptographic identity, as opposed to the cosmetic email "login" profile.

import { safeInvoke as invoke } from "./tauri-compat";

// Hand-written (not ts-rs) — the backend struct lives in app-tauri, not core,
// so there is no generated binding. Keep in sync with IdentityStatus there.
export interface IdentityStatus {
  hasIdentity: boolean;
  fingerprint: string;
  createdAt: string;
  keystore: string;
  identityLine: string | null;
}

/** Read the on-device identity status. Returns null when the command is
 *  unavailable (e.g. browser/web mode without the Tauri backend). */
export async function loadIdentityStatus(): Promise<IdentityStatus | null> {
  const res = await invoke<IdentityStatus>("identity_status", {});
  if (!res || typeof (res as IdentityStatus).fingerprint !== "string") return null;
  return res as IdentityStatus;
}
