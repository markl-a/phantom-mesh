// MobileFirstLaunch — 3-button mode picker shown on cold install.
// Replaces the unconditional `localStorage.setItem('spectyn_mesh_v2_onboarded', true)`
// hack from the v1 redesign. See spec/2026-05-23-mobile-redesign-v2.md §4.1.
//
// Modes:
//   [A] Demo      — uses bundled Cerebras key, no setup. Shared free quota.
//   [B] Join      — has a coordinator (mDNS auto-discovered or pasted URL).
//   [C] Sign in   — phantommesh.io OAuth → vault sync.
//
// Persisted decision: localStorage key `spectyn_mesh_v2_onboarded_mode` ∈ {demo,join,broker}.
// Old `spectyn_mesh_v2_onboarded=true` is set for back-compat with App.tsx gates.

import { useEffect, useState } from "react";
import { Cloud, Wifi, LogIn, Loader2 } from "lucide-react";
// Wire helper around the Wave H1.1 Tauri commands `onboarding_advance`
// + `onboarding_rollback` (registered in src-tauri/src/lib.rs:572-573).
import { advanceOnboarding } from "../../lib/onboardingFsm";
import { requestPermission, permissionsGateApplies } from "../../lib/permissions";

// One-time guard so we only ever fire the OS notification prompt once.
const NOTIF_PROMPTED_KEY = "spectyn_notif_prompted";

// Fire a single, non-blocking notification-permission request the moment the
// user first engages with onboarding. On Android 13+ POST_NOTIFICATIONS is a
// runtime grant; without it the native Kotlin notifications from
// MeshNodeService (persistent node FGS, focus-session countdown, habit-capture
// confirmation — SPEC-34) are silently dropped. Mobile-only (gated by
// permissionsGateApplies), asks at most once (localStorage flag), and never
// blocks navigation — onboarding proceeds regardless of the outcome.
function maybePromptNotifications(): void {
  if (!permissionsGateApplies()) return; // no-op on desktop/web
  try {
    if (localStorage.getItem(NOTIF_PROMPTED_KEY)) return; // already asked once
    localStorage.setItem(NOTIF_PROMPTED_KEY, "1");
  } catch {
    // localStorage unavailable — skip rather than risk re-prompting in a loop.
    return;
  }
  // Fire-and-forget: do NOT await — onboarding navigation must not wait on the
  // permission dialog or its result.
  void requestPermission("notifications").catch(() => {
    /* permission outcome is best-effort; failures are non-fatal here */
  });
}

interface DiscoveredHost {
  host: string;        // e.g. "example-host.example.com"
  port: number;        // e.g. 7878
  url: string;         // ready-to-use base URL
}

interface Props {
  onPickedDemo: () => void;
  onPickedJoin: (discovered?: DiscoveredHost) => void;
  onPickedSignIn: () => void;
}

// Which mode button is mid-invoke — disables all 3 + shows spinner.
type PickInFlight = null | "demo" | "join" | "signin";

export default function MobileFirstLaunch({ onPickedDemo, onPickedJoin, onPickedSignIn }: Props) {
  const [discovered, setDiscovered] = useState<DiscoveredHost | null>(null);
  const [scanning, setScanning] = useState(true);
  const [inFlight, setInFlight] = useState<PickInFlight>(null);
  const [pickError, setPickError] = useState<string | null>(null);

  // Listen for mDNS hits from the Rust side (main.mm's NSNetServiceBrowser
  // forwards via Tauri event `deep-link://mdns-peer` when a `_spectyn-mesh._tcp`
  // service is resolved). 3-second window; whichever fires first wins.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        unlisten = await listen<{ host: string; port: number; url: string }>(
          "deep-link://mdns-peer",
          (e) => {
            if (cancelled) return;
            setDiscovered(e.payload);
          },
        );
      } catch (_e) {
        // Tauri event API may not be available in browser dev — ignore
      }
      // 3-second scan window
      setTimeout(() => {
        if (!cancelled) setScanning(false);
      }, 3000);
    })();

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  // Drive the SPEC-28 onboarding FSM forward when the user picks a mode.
  // Each mode patches the `OnboardingContext` so downstream steps know
  // which path was taken (e.g. `demoRelayUsed=true` skips later provider
  // configuration). Backend may still be `unimplemented!()` — the helper
  // soft-fails and we proceed regardless so the user is never blocked.
  const runPick = async (
    mode: PickInFlight,
    ctxPatch: Parameters<typeof advanceOnboarding>[0],
    next: () => void,
  ) => {
    if (inFlight) return;
    setPickError(null);
    setInFlight(mode);
    // User-initiated moment: ask once for notification permission so the
    // Android 13+ Kotlin notifications (SPEC-34) can actually appear. Non-
    // blocking — onboarding continues immediately below.
    maybePromptNotifications();
    try {
      await advanceOnboarding(ctxPatch);
      next();
    } catch (e) {
      setPickError(e instanceof Error ? e.message : String(e));
    } finally {
      setInFlight(null);
    }
  };

  const handleDemo = () => runPick("demo", { demoRelayUsed: true }, onPickedDemo);
  const handleJoin = () => runPick("join", {}, () => onPickedJoin(discovered ?? undefined));
  const handleSignIn = () => runPick("signin", {}, onPickedSignIn);

  return (
    <div className="h-[100dvh] bg-spectyn-bg flex flex-col items-center justify-center px-6">
      <div className="flex flex-col items-center mb-12">
        <div className="w-20 h-20 bg-spectyn-primary/20 rounded-full flex items-center justify-center mb-4">
          <span className="text-3xl">◆</span>
        </div>
        <h1 className="text-2xl font-bold text-spectyn-text mb-2">Spectyn Mesh</h1>
        <p className="text-sm text-spectyn-muted text-center">
          你的多裝置 AI agent · 私人加密 · 隨選 LLM
        </p>
      </div>

      <div className="w-full max-w-sm flex flex-col gap-3">

        {/* [A] Demo */}
        <button
          onClick={handleDemo}
          disabled={inFlight !== null}
          className="w-full p-4 bg-spectyn-bg-elevated border border-spectyn-border rounded-xl text-left hover:border-spectyn-primary transition disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <div className="flex items-center gap-3">
            {inFlight === "demo" ? (
              <Loader2 size={22} className="text-spectyn-muted shrink-0 animate-spin" />
            ) : (
              <Cloud size={22} className="text-spectyn-muted shrink-0" />
            )}
            <div className="flex-1">
              <div className="font-semibold text-spectyn-text">立即試用</div>
              <div className="text-xs text-spectyn-muted mt-0.5">
                免費共用 LLM · 不需設定 · 30 秒看到回應
              </div>
            </div>
          </div>
        </button>

        {/* [B] Join cluster */}
        <button
          onClick={handleJoin}
          disabled={inFlight !== null}
          className="w-full p-4 bg-spectyn-bg-elevated border border-spectyn-border rounded-xl text-left hover:border-spectyn-primary transition disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <div className="flex items-center gap-3">
            {inFlight === "join" ? (
              <Loader2 size={22} className="text-spectyn-muted shrink-0 animate-spin" />
            ) : (
              <Wifi size={22} className="text-spectyn-muted shrink-0" />
            )}
            <div className="flex-1">
              <div className="font-semibold text-spectyn-text">加入既有 cluster</div>
              {scanning ? (
                <div className="text-xs text-spectyn-muted mt-0.5 flex items-center gap-1">
                  <Loader2 size={12} className="animate-spin" />
                  本機網路搜尋中...
                </div>
              ) : discovered ? (
                <div className="text-xs text-spectyn-primary mt-0.5 truncate">
                  發現: {discovered.host}
                </div>
              ) : (
                <div className="text-xs text-spectyn-muted mt-0.5">
                  手動輸入 coordinator URL + secret
                </div>
              )}
            </div>
          </div>
        </button>

        {/* [C] Sign in */}
        <button
          onClick={handleSignIn}
          disabled={inFlight !== null}
          className="w-full p-4 bg-spectyn-bg-elevated border border-spectyn-border rounded-xl text-left hover:border-spectyn-primary transition disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <div className="flex items-center gap-3">
            {inFlight === "signin" ? (
              <Loader2 size={22} className="text-spectyn-muted shrink-0 animate-spin" />
            ) : (
              <LogIn size={22} className="text-spectyn-muted shrink-0" />
            )}
            <div className="flex-1">
              <div className="font-semibold text-spectyn-text">使用 phantommesh.io 登入</div>
              <div className="text-xs text-spectyn-muted mt-0.5">
                跨裝置同步設定、cluster、LLM keys
              </div>
            </div>
          </div>
        </button>
      </div>

      {pickError && (
        <div className="mt-4 w-full max-w-sm p-3 bg-red-500/10 border border-red-500/30 rounded-lg text-red-400 text-xs break-words">
          無法推進 onboarding：{pickError}
        </div>
      )}

      <div className="mt-12 text-xs text-spectyn-muted text-center max-w-xs">
        資料加密儲存於本機 · 不需要登入也能使用
      </div>
    </div>
  );
}
