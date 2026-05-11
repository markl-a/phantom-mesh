// Manual LLM-provider key management — fallback for users who can't /
// don't want to go through the broker (Google login). Reads / writes
// the same ~/.phantom-mesh/env file the broker_sync_from_vault uses.

import { invoke } from "@tauri-apps/api/core";

export interface KeyStatus {
  name: string;
  set: boolean;
  preview: string | null;
}

export interface LocalKeysSnapshot {
  keys: KeyStatus[];
  env_path: string;
}

export const ALLOWED_KEYS = [
  "OPENCODE_API_KEY",
  "GROQ_API_KEY",
  "ANTHROPIC_API_KEY",
  "OPENAI_API_KEY",
  "GEMINI_API_KEY",
  "OPENROUTER_API_KEY",
  "CEREBRAS_API_KEY",
  "DEEPSEEK_API_KEY",
  "MISTRAL_API_KEY",
  "TOGETHER_API_KEY",
  "NVIDIA_NIM_API_KEY",
  "CLUSTER_SECRET",
] as const;

export async function listProviderKeys(): Promise<LocalKeysSnapshot> {
  return invoke<LocalKeysSnapshot>("list_provider_keys");
}

export async function setProviderKey(
  name: string,
  value: string,
): Promise<LocalKeysSnapshot> {
  return invoke<LocalKeysSnapshot>("set_provider_key", { name, value });
}

/** Parse a multi-line "KEY=value" or "export KEY=value" block and apply
 *  any allowlisted matches. Skips blanks, comments, and unknown keys. */
export function parseEnvBlock(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const raw of text.split(/\r?\n/)) {
    let line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    if (line.startsWith("export ")) line = line.slice(7).trimStart();
    const eq = line.indexOf("=");
    if (eq < 0) continue;
    const key = line.slice(0, eq).trim();
    let val = line.slice(eq + 1).trim();
    // strip surrounding quotes if any
    if (val.length >= 2) {
      const first = val[0];
      const last = val[val.length - 1];
      if ((first === '"' && last === '"') || (first === "'" && last === "'")) {
        val = val.slice(1, -1);
      }
    }
    if (key) out[key] = val;
  }
  return out;
}

export async function setProviderKeysBulk(
  entries: Record<string, string>,
): Promise<LocalKeysSnapshot> {
  return invoke<LocalKeysSnapshot>("set_provider_keys_bulk", { entries });
}
