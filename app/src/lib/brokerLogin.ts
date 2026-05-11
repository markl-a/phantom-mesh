// Broker login bridge — JS side of the iOS / Tauri OAuth flow.
//
// Pairs with app/src-tauri/src/commands/broker_login.rs. The Rust side:
//   1. broker_login_start(broker_url) → returns auth_url that the
//      front-end opens in Mobile Safari via tauri-plugin-shell.
//   2. tauri-plugin-deep-link's onOpenUrl fires when iOS routes the
//      `phantom://oauth/callback?p=<b64>` URL back to the app, and emits
//      a `deep-link://oauth-callback` Tauri event.
//   3. broker_login_finish(payload_b64) decodes the base64 payload,
//      builds AuthState, and persists via phantom_mesh::auth::save().
//
// installBrokerLoginBridge() should be called once at app startup so the
// listener is wired before Safari → app handoff.

import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

// On iOS Tauri the shell plugin's `open(url)` hits a sandbox "Operation
// not permitted" (os error 1). We bypass it by navigating the webview
// itself to the URL — phantom://oauth/callback redirects on the broker
// side still get routed to tauri-plugin-deep-link's onOpenUrl handler
// because iOS recognises the registered URL scheme.
async function openExternal(url: string): Promise<void> {
  // Try shell plugin first for desktop (where it works fine + opens
  // user's actual default browser). On iOS this throws, fall through to
  // webview navigation.
  try {
    const { open: shellOpen } = await import("@tauri-apps/plugin-shell");
    await shellOpen(url);
    return;
  } catch (_e) {
    // Sandbox-blocked → navigate the current webview instead. Note this
    // unmounts the React tree; the deep-link callback is what brings us
    // back. Onboarding state is reconstructed on next launch from
    // AuthState (already saved by broker_login_finish).
    window.location.href = url;
  }
}

export interface BrokerLoginStartResponse {
  auth_url: string;
  device_id: string;
  redirect: string;
}

export interface BrokerLoginFinishResponse {
  email: string;
  provider: string;
  display_name: string | null;
  broker_token_expires_at_ms: number;
  auth_path: string;
}

/** Trigger the broker login flow. Opens system browser; the deep-link
 *  listener (installed by installBrokerLoginBridge) handles the callback. */
export async function startBrokerLogin(
  brokerUrl: string = "https://phantommesh.io",
): Promise<BrokerLoginStartResponse> {
  const resp = await invoke<BrokerLoginStartResponse>("broker_login_start", {
    brokerUrl,
  });
  // Hand off to Mobile Safari / desktop default browser. tauri-plugin-shell
  // is platform-aware — on iOS it goes through openURL: which respects
  // user's default browser choice (or Safari).
  await openExternal(resp.auth_url);
  return resp;
}

/** Convenience: returns the AuthState summary if we have a saved token. */
export async function loadBrokerLoginStatus(): Promise<BrokerLoginFinishResponse | null> {
  const result = await invoke<BrokerLoginFinishResponse | null>("broker_login_status");
  return result ?? null;
}

/** Wipe local broker auth (for switch-account or token-rotation cases). */
export async function logoutBroker(): Promise<void> {
  await invoke("broker_login_logout");
}

export interface ClusterPeer {
  name: string;
  url: string;
  label?: string | null;
}

export interface BrokerSyncResponse {
  keys_written: string[];
  env_path: string;
  peers_count: number;
  peers_path: string | null;
  peers: ClusterPeer[];
}

/** Returns the cluster peers cached at ~/.phantom-mesh/peers.json (last
 *  vault sync). Used on app boot to decide if the user already has a
 *  coordinator to pick from without re-syncing. */
export async function listCachedPeers(): Promise<ClusterPeer[]> {
  return invoke<ClusterPeer[]>("broker_list_cached_peers");
}

/** Pull LLM keys + cluster peers from phantommesh.io vault. Idempotent;
 *  call right after broker_login_finish or whenever the user wants to
 *  re-sync. Requires a saved AuthState with a non-empty broker_token. */
export async function syncFromVault(
  brokerUrl: string = "https://phantommesh.io",
): Promise<BrokerSyncResponse> {
  return invoke<BrokerSyncResponse>("broker_sync_from_vault", { brokerUrl });
}

/** Register THIS device on the user's cluster peer list so other peers
 *  discover it. Returns total peer count after upsert. */
export async function registerSelfPeer(args: {
  name: string;
  url: string;
  label?: string;
  brokerUrl?: string;
}): Promise<number> {
  return invoke<number>("broker_register_self_peer", args);
}

/** Extract the `?p=<b64>` query value from a phantom://oauth/callback URL.
 *  Returns null when the URL isn't an OAuth callback or has no `p` param. */
export function extractPayload(url: string): string | null {
  // URL constructor doesn't always parse custom schemes — work the
  // string directly. Format: phantom://oauth/callback?p=<base64url>
  const qIdx = url.indexOf("?");
  if (qIdx < 0) return null;
  const query = url.slice(qIdx + 1);
  for (const kv of query.split("&")) {
    const eq = kv.indexOf("=");
    if (eq < 0) continue;
    if (kv.slice(0, eq) === "p") {
      return decodeURIComponent(kv.slice(eq + 1));
    }
  }
  return null;
}

export type BrokerLoginListener = (
  result:
    | { ok: true; identity: BrokerLoginFinishResponse; sync?: BrokerSyncResponse }
    | { ok: false; error: string },
) => void;

let bridgeInstalled = false;
const listeners = new Set<BrokerLoginListener>();

export function onBrokerLoginResult(cb: BrokerLoginListener): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

function fanOut(result: Parameters<BrokerLoginListener>[0]): void {
  for (const cb of Array.from(listeners)) {
    try {
      cb(result);
    } catch (e) {
      console.error("[brokerLogin] listener threw:", e);
    }
  }
}

/** Install the deep-link → broker_login_finish bridge. Idempotent. Call
 *  once at app startup (before any Safari handoff happens). */
export async function installBrokerLoginBridge(): Promise<void> {
  if (bridgeInstalled) return;
  bridgeInstalled = true;

  await listen<string>("deep-link://oauth-callback", async (evt) => {
    const url = evt.payload;
    console.log("[brokerLogin] deep-link received:", url);
    const payload = extractPayload(url);
    if (!payload) {
      const err = `deep-link missing ?p= query: ${url}`;
      console.warn("[brokerLogin]", err);
      fanOut({ ok: false, error: err });
      return;
    }
    try {
      const identity = await invoke<BrokerLoginFinishResponse>(
        "broker_login_finish",
        { payloadB64: payload },
      );
      console.log("[brokerLogin] login finished:", identity);

      // Chain into vault sync — pull LLM keys + cluster peers using the
      // freshly-saved broker_token. Errors here surface but don't cancel
      // the "logged-in" state (user can retry sync separately).
      let sync: BrokerSyncResponse | undefined;
      try {
        sync = await syncFromVault();
        console.log("[brokerLogin] vault sync:", sync);
      } catch (e) {
        console.warn("[brokerLogin] vault sync failed (login still ok):", e);
      }

      fanOut({ ok: true, identity, sync });
    } catch (e) {
      const err = String(e);
      console.error("[brokerLogin] broker_login_finish failed:", err);
      fanOut({ ok: false, error: err });
    }
  });
}
