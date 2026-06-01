// SPEC-33 §11 / §15 — Android runtime permission gates (frontend layer).
//
// phantom-mesh declares three *dangerous / runtime* Android permissions that
// must be requested at use-time rather than install-time (SPEC-33 §11.1):
//
//   - RECORD_AUDIO       SPEC-21 focus audio + SPEC-22 habit voice  →「麥克風」
//   - CAMERA             SPEC-20 food photo capture                  →「相機」
//   - POST_NOTIFICATIONS Android 13+ notification runtime grant      →「通知」
//
// The actual OS prompt is driven through Web APIs on purpose: the Tauri Android
// WebView's `RustWebChromeClient.onPermissionRequest` already bridges
// `getUserMedia({audio})` → `RECORD_AUDIO` and `getUserMedia({video})` →
// `CAMERA` through `ActivityCompat.requestPermissions`, and
// `Notification.requestPermission()` maps to `POST_NOTIFICATIONS`. This means
// the gates work in the *real* WebView today with no extra Rust command, and
// degrade to a non-blocking "granted" in desktop/browser dev so callers never
// dead-end — the same honest-fallback philosophy as `lib/captureFocus.ts`.
//
// The §15.2 three-step flow (rationale dialog → system dialog → handle result)
// lives in `components/permissions/PermissionGate.tsx`; this module owns the
// platform plumbing + the "never ask again" heuristic the gate renders against.

import { isTauri } from "./tauri-compat";

/** The three runtime permission gates phantom requests. */
export type PermissionKind = "microphone" | "camera" | "notifications";

/**
 * Normalized status, independent of the underlying Web API quirks.
 * - `granted`     — usable now, render the feature.
 * - `denied`      — user said no (this round); a retry is still allowed unless
 *                   `neverAskAgain` is also set.
 * - `prompt`      — not yet decided; requesting will show the OS dialog.
 * - `unsupported` — neither the Web API nor a bridge is available (e.g. a
 *                   desktop build with no camera) — treat as soft-skip.
 */
export type PermissionStatus = "granted" | "denied" | "prompt" | "unsupported";

export interface PermissionResult {
  status: PermissionStatus;
  /**
   * Heuristic: the user has denied this permission enough times that the OS
   * dialog will no longer appear, so the UI should deep-link to system
   * settings instead of re-asking (SPEC-33 §15.2 step 4 "never_ask_again").
   */
  neverAskAgain: boolean;
}

interface PermissionMeta {
  /** Canonical Android permission string (SPEC-33 §11.1), for display/debug. */
  androidPermission: string;
  /** Short label shown in chips / headers. */
  label: string;
  /** Why phantom needs it — Traditional Chinese, shown in the rationale step. */
  rationaleZh: string;
  /** Why phantom needs it — English, shown beneath the zh copy. */
  rationaleEn: string;
  /** What the user loses if they decline (fallback behaviour copy). */
  fallbackZh: string;
}

export const PERMISSION_META: Record<PermissionKind, PermissionMeta> = {
  microphone: {
    androidPermission: "android.permission.RECORD_AUDIO",
    label: "麥克風",
    rationaleZh:
      "專注時段（SPEC-21）需要麥克風錄下環境音，才能在結束後產生這段時間的摘要與建議。",
    rationaleEn:
      "Focus sessions record ambient audio so phantom can summarise the block when it ends.",
    fallbackZh: "沒有麥克風權限仍可純計時，但不會有錄音摘要。",
  },
  camera: {
    androidPermission: "android.permission.CAMERA",
    label: "相機",
    rationaleZh:
      "食物紀錄（SPEC-20）用相機拍下餐點，由 AI 估算營養與熱量。照片只加密儲存在本機。",
    rationaleEn:
      "Food capture uses the camera to log meals for on-device AI macro estimation.",
    fallbackZh: "沒有相機權限可改用相簿選圖。",
  },
  notifications: {
    androidPermission: "android.permission.POST_NOTIFICATIONS",
    label: "通知",
    rationaleZh:
      "通知權限讓 phantom 在背景節點與焦點計時結束、教練回顧（SPEC-24）抵達時提醒你。",
    rationaleEn:
      "Notifications let phantom alert you when a focus block ends or a coach review arrives.",
    fallbackZh: "沒有通知權限仍可使用，但需自行打開 app 才看得到提醒。",
  },
};

// ─── "never ask again" heuristic ───────────────────────────────────────────
// The Web permission APIs don't expose Android's `shouldShowRequestPermission
// Rationale` signal, so we approximate: count consecutive denials per kind. On
// the 2nd+ denial we assume the OS will stop showing its dialog and the UI
// should route to settings instead. Cleared on the first grant.

const DENY_COUNT_KEY = "phantom_mesh_perm_deny_v1";
const NEVER_ASK_THRESHOLD = 2;

type DenyMap = Partial<Record<PermissionKind, number>>;

function readDenyMap(): DenyMap {
  if (typeof localStorage === "undefined") return {};
  try {
    const raw = localStorage.getItem(DENY_COUNT_KEY);
    return raw ? (JSON.parse(raw) as DenyMap) : {};
  } catch {
    return {};
  }
}

function writeDenyMap(map: DenyMap): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(DENY_COUNT_KEY, JSON.stringify(map));
  } catch {
    /* quota / disabled — best effort */
  }
}

function bumpDeny(kind: PermissionKind): number {
  const map = readDenyMap();
  const next = (map[kind] ?? 0) + 1;
  map[kind] = next;
  writeDenyMap(map);
  return next;
}

function clearDeny(kind: PermissionKind): void {
  const map = readDenyMap();
  if (map[kind]) {
    delete map[kind];
    writeDenyMap(map);
  }
}

function denyCount(kind: PermissionKind): number {
  return readDenyMap()[kind] ?? 0;
}

// ─── status query (non-prompting) ──────────────────────────────────────────

function permissionsApiName(kind: PermissionKind): PermissionName | null {
  switch (kind) {
    case "microphone":
      return "microphone" as PermissionName;
    case "camera":
      return "camera" as PermissionName;
    case "notifications":
      // Some engines expose "notifications" via the Permissions API; we prefer
      // the dedicated Notification.permission below, so skip it here.
      return null;
  }
}

/**
 * Read the current status WITHOUT triggering an OS prompt. Best-effort: returns
 * `prompt` when the platform can't tell us, so the caller will ask on demand.
 */
export async function checkPermission(kind: PermissionKind): Promise<PermissionStatus> {
  if (kind === "notifications") {
    if (typeof Notification === "undefined") return "unsupported";
    switch (Notification.permission) {
      case "granted":
        return "granted";
      case "denied":
        return "denied";
      default:
        return "prompt";
    }
  }

  // microphone / camera need mediaDevices to be requestable at all.
  if (typeof navigator === "undefined" || !navigator.mediaDevices?.getUserMedia) {
    return "unsupported";
  }

  const apiName = permissionsApiName(kind);
  if (apiName && navigator.permissions?.query) {
    try {
      const st = await navigator.permissions.query({ name: apiName });
      if (st.state === "granted") {
        clearDeny(kind);
        return "granted";
      }
      if (st.state === "denied") return "denied";
      return "prompt";
    } catch {
      // Permissions API present but doesn't know this name — fall through.
    }
  }
  // Can't introspect → assume we must ask.
  return "prompt";
}

// ─── request (prompting) ───────────────────────────────────────────────────

async function requestMedia(kind: "microphone" | "camera"): Promise<PermissionStatus> {
  if (typeof navigator === "undefined" || !navigator.mediaDevices?.getUserMedia) {
    return "unsupported";
  }
  const constraints: MediaStreamConstraints =
    kind === "microphone" ? { audio: true } : { video: true };
  try {
    const stream = await navigator.mediaDevices.getUserMedia(constraints);
    // Immediately release the device — we only wanted the grant, not a capture.
    stream.getTracks().forEach((t) => t.stop());
    return "granted";
  } catch (e) {
    // NotAllowedError / SecurityError → denied; NotFoundError → no device.
    const name = (e as { name?: string })?.name ?? "";
    if (name === "NotFoundError" || name === "OverconstrainedError") return "unsupported";
    return "denied";
  }
}

/**
 * Trigger the OS permission dialog for `kind` and return the resolved status
 * plus the "never ask again" heuristic. Safe to call from a click handler.
 */
export async function requestPermission(kind: PermissionKind): Promise<PermissionResult> {
  let status: PermissionStatus;

  if (kind === "notifications") {
    if (typeof Notification === "undefined") {
      status = "unsupported";
    } else {
      try {
        const res = await Notification.requestPermission();
        status = res === "granted" ? "granted" : res === "denied" ? "denied" : "prompt";
      } catch {
        status = "denied";
      }
    }
  } else {
    status = await requestMedia(kind);
  }

  if (status === "granted") {
    clearDeny(kind);
    return { status, neverAskAgain: false };
  }
  if (status === "denied") {
    const count = bumpDeny(kind);
    return { status, neverAskAgain: count >= NEVER_ASK_THRESHOLD };
  }
  return { status, neverAskAgain: denyCount(kind) >= NEVER_ASK_THRESHOLD };
}

// ─── settings deep-link (best effort) ──────────────────────────────────────

/**
 * Open the OS app-details settings page so the user can flip a permission that
 * the in-app dialog can no longer prompt for (SPEC-33 §15.2 step 4 + §15.4).
 *
 * The dedicated Rust command (`Settings.ACTION_APPLICATION_DETAILS_SETTINGS`
 * bridge) is not registered yet, so we probe a couple of candidate commands and
 * report whether any handled it. Returns `false` when the caller should fall
 * back to showing manual "Settings → Apps → phantom → Permissions" steps.
 */
export async function openAppSettings(): Promise<boolean> {
  if (!isTauri()) return false;
  try {
    const { safeInvoke } = await import("./tauri-compat");
    // `open_app_settings` is the planned command name (SPEC-33 §9.2 neighbours
    // miui_guide_*). Until it lands, safeInvoke surfaces a thrown error and we
    // report failure cleanly.
    await safeInvoke("open_app_settings", {});
    return true;
  } catch {
    return false;
  }
}

/**
 * Whether runtime permission gates are meaningful on this platform. Mobile
 * (Tauri Android/iOS WebView) always; desktop browsers still honour the gate
 * for mic/camera but never need the notification gate. Used by callers that
 * want to skip gating entirely on, e.g., a server-rendered dev build.
 */
export function permissionsGateApplies(): boolean {
  return typeof navigator !== "undefined";
}
