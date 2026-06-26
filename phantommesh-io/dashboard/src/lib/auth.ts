// Dashboard-side mirror of the broker login. The authoritative session is
// the HttpOnly `phantom_session` cookie set by the worker after the OAuth
// dance; this localStorage entry is a cheap, readable copy of the bearer
// token + identity so the SPA can gate its first paint and attach
// `Authorization: Bearer` without a round-trip to /api/me on every nav.
//
// Contract is pinned by tests/auth.test.ts — keep them in sync.

const STORAGE_KEY = "phantom.broker_token";

export interface BrokerLogin {
  token: string;
  email: string;
  /** Absolute expiry in epoch milliseconds. */
  expires_at_ms: number;
}

function readStorage(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    // Private-mode / disabled storage — treat as logged out.
    return null;
  }
}

/** Persist a fresh login. Overwrites any previous entry. */
export function setBrokerLogin(login: BrokerLogin): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(login));
  } catch {
    // Best-effort; a failed write just means the SPA re-probes the cookie.
  }
}

/** Drop the stored login (logout, expiry GC, or stale-token cleanup). */
export function clearBrokerLogin(): void {
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    /* ignore */
  }
}

function isBrokerLogin(value: unknown): value is BrokerLogin {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.token === "string" &&
    typeof v.email === "string" &&
    typeof v.expires_at_ms === "number"
  );
}

/**
 * Parse the stored login. Returns null for a missing, malformed, or
 * partially-shaped entry — never throws. Does NOT apply expiry (callers
 * use hasBrokerToken/getBearerToken for the live check).
 */
export function getBrokerLogin(): BrokerLogin | null {
  const raw = readStorage();
  if (raw === null) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  return isBrokerLogin(parsed) ? parsed : null;
}

/**
 * True iff a well-shaped, non-expired login is stored. Opportunistically
 * GCs an expired entry so the next paint sees a clean logged-out state.
 */
export function hasBrokerToken(): boolean {
  const login = getBrokerLogin();
  if (login === null) return false;
  if (login.expires_at_ms <= Date.now()) {
    clearBrokerLogin();
    return false;
  }
  return true;
}

/** The bearer token for Authorization headers, or null when not usable. */
export function getBearerToken(): string | null {
  if (!hasBrokerToken()) return null;
  return getBrokerLogin()!.token;
}
