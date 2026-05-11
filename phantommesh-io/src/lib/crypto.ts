// At-rest encryption for the user_settings.env_json column.
//
// Threat model — what this protects against:
//   - A D1 dump or read leak (e.g. via a misconfigured wrangler token
//     scoped to D1-read but not Worker-secrets) reveals only ciphertext.
//   - One user's data key leaking does NOT decrypt another user's row,
//     because each user has a distinct HKDF-derived data key.
//
// Threat model — what this does NOT protect against:
//   - Full Cloudflare account compromise: the attacker reads the
//     ENV_VAULT_KEY wrangler secret too, derives every user's data key,
//     decrypts everything. The only fix for that is end-to-end (browser
//     holds the master, server stays blind). Deferred.
//   - JWT secret compromise: attacker forges a broker_token, hits
//     /api/me/settings/raw, gets plaintext through the normal API path
//     (which decrypts on the way out). Same E2E fix as above.
//   - Side-channel timing attacks on AES-GCM tag verification: Web Crypto
//     uses constant-time AES-GCM, so this is mitigated by the runtime.
//
// Wire format (base64-encoded into the env_json column):
//   "v1." || base64( iv[12] || ciphertext || tag[16] )
//   The "v1." prefix is a deliberate version marker — old plaintext rows
//   start with "{" and parse cleanly as JSON, so getUserSettings can
//   round-trip them once and re-encrypt on next write. Bumping to "v2."
//   later (rotated key, different cipher) follows the same pattern.

const VERSION_PREFIX = "v1.";
const HKDF_INFO = "phantommesh-env-vault-v1";

/// Decode the base64 master key from the wrangler secret. Throws if the
/// secret is missing or shorter than 32 bytes after decode (AES-256
/// needs 32 bytes; we don't pad to avoid masking misconfig).
async function getMasterKeyBytes(envKeyBase64: string): Promise<Uint8Array> {
  if (!envKeyBase64 || envKeyBase64.length < 16) {
    throw new Error("ENV_VAULT_KEY missing or too short — set via `wrangler secret put ENV_VAULT_KEY`");
  }
  const raw = Uint8Array.from(atob(envKeyBase64), c => c.charCodeAt(0));
  if (raw.length < 32) {
    throw new Error(`ENV_VAULT_KEY decoded to ${raw.length} bytes; need ≥ 32 for AES-256`);
  }
  return raw.subarray(0, 32);
}

/// Derive a per-user 32-byte AES key via HKDF-SHA256.
/// salt = textual user_id; info = stable label so derived keys are
/// scoped to this app and won't collide with future per-feature keys.
async function deriveUserKey(masterBytes: Uint8Array, user_id: number): Promise<CryptoKey> {
  // Convert raw master bytes to a fresh ArrayBuffer to avoid SharedArrayBuffer typing issues
  const masterBuffer = new ArrayBuffer(masterBytes.length);
  new Uint8Array(masterBuffer).set(masterBytes);
  const ikm = await crypto.subtle.importKey(
    "raw",
    masterBuffer,
    { name: "HKDF" },
    false,
    ["deriveKey"],
  );
  const saltBytes = new TextEncoder().encode(`user:${user_id}`);
  const saltBuffer = new ArrayBuffer(saltBytes.length);
  new Uint8Array(saltBuffer).set(saltBytes);
  const infoBytes = new TextEncoder().encode(HKDF_INFO);
  const infoBuffer = new ArrayBuffer(infoBytes.length);
  new Uint8Array(infoBuffer).set(infoBytes);
  return await crypto.subtle.deriveKey(
    {
      name: "HKDF",
      hash: "SHA-256",
      salt: saltBuffer,
      info: infoBuffer,
    },
    ikm,
    { name: "AES-GCM", length: 256 },
    false,
    ["encrypt", "decrypt"],
  );
}

/// Encrypt arbitrary JSON-serializable plaintext for storage.
/// Returns the wire format string ready to write into env_json.
export async function encryptForUser(
  envKeyBase64: string,
  user_id: number,
  plaintext: string,
): Promise<string> {
  const master = await getMasterKeyBytes(envKeyBase64);
  const key = await deriveUserKey(master, user_id);
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const ptBytes = new TextEncoder().encode(plaintext);
  const ct = new Uint8Array(await crypto.subtle.encrypt(
    { name: "AES-GCM", iv: iv as BufferSource },
    key,
    ptBytes as BufferSource,
  ));
  // Concat iv || ct (ct already includes the auth tag at the end per WebCrypto)
  const out = new Uint8Array(iv.length + ct.length);
  out.set(iv, 0);
  out.set(ct, iv.length);
  // base64 without using Buffer (Workers runtime may not have it)
  let bin = "";
  for (let i = 0; i < out.length; i++) bin += String.fromCharCode(out[i]);
  return VERSION_PREFIX + btoa(bin);
}

/// Decrypt a value produced by encryptForUser. Returns null when the
/// blob doesn't look like our format — caller is expected to fall back
/// to plaintext-JSON parsing for legacy rows.
export async function decryptForUser(
  envKeyBase64: string,
  user_id: number,
  blob: string,
): Promise<string | null> {
  if (!blob.startsWith(VERSION_PREFIX)) return null;
  const b64 = blob.slice(VERSION_PREFIX.length);
  const bin = atob(b64);
  const raw = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) raw[i] = bin.charCodeAt(i);
  if (raw.length < 12 + 16) return null; // iv + tag minimum
  const iv = raw.subarray(0, 12);
  const ct = raw.subarray(12);
  const master = await getMasterKeyBytes(envKeyBase64);
  const key = await deriveUserKey(master, user_id);
  try {
    const ptBytes = await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: iv as BufferSource },
      key,
      ct as BufferSource,
    );
    return new TextDecoder().decode(ptBytes);
  } catch {
    // GCM tag mismatch (wrong key, tampered data, wrong user) → null,
    // caller decides how to handle (usually: treat as "no data").
    return null;
  }
}

/// True when `blob` looks like the encrypted wire format (starts with the
/// version marker). Used by getUserSettings to choose the decode path
/// without trying both speculatively.
export function isEncryptedBlob(blob: string): boolean {
  return blob.startsWith(VERSION_PREFIX);
}
