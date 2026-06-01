// SPEC-33 §15.2 — runtime permission gate (rationale → request → settings).
//
// Wrap any feature that needs RECORD_AUDIO / CAMERA / POST_NOTIFICATIONS:
//
//   <PermissionGate kind="microphone"><FocusRecorder /></PermissionGate>
//
// Renders the three-step flow from SPEC-33 §15.2:
//   1. rationale card explaining *why* (shown before the OS dialog)
//   2. "允許" button → OS dialog via the Web bridge (usePermission.request)
//   3. result handling: granted → children; denied → retry; never-ask-again →
//      deep-link to system settings + manual fallback steps.
//
// Styling follows the phantom Material-3 token set (rounded-2xl tonal surfaces,
// filled-tonal primary action) so it sits consistently inside the mobile shell.

import type { ReactNode } from "react";
import { Mic, Camera, Bell, Settings, ShieldCheck, Loader2 } from "lucide-react";
import { PERMISSION_META, type PermissionKind } from "../../lib/permissions";
import { usePermission } from "./usePermission";

const ICON: Record<PermissionKind, typeof Mic> = {
  microphone: Mic,
  camera: Camera,
  notifications: Bell,
};

interface Props {
  kind: PermissionKind;
  children: ReactNode;
  /** Called once the permission resolves to granted (or unsupported skip). */
  onReady?: () => void;
  /** Render children even when not granted (soft prompt above them). */
  optional?: boolean;
}

export default function PermissionGate({ kind, children, onReady, optional }: Props) {
  const meta = PERMISSION_META[kind];
  const Icon = ICON[kind];
  const { status, neverAskAgain, requesting, request, openSettings, refresh } =
    usePermission(kind);

  // `unsupported` = desktop/browser without the device or API → don't block.
  const ready = status === "granted" || status === "unsupported";

  if (status === "unknown") {
    return (
      <div className="flex items-center justify-center py-8" data-testid={`perm-gate-${kind}-loading`}>
        <Loader2 size={22} className="text-phantom-muted animate-spin" />
      </div>
    );
  }

  if (ready) {
    onReady?.();
    return <>{children}</>;
  }

  const handlePrimary = async () => {
    const next = await request();
    if (next === "granted" || next === "unsupported") onReady?.();
  };

  const card = (
    <div
      className="bg-phantom-card border border-phantom-border rounded-2xl p-5 space-y-4"
      data-testid={`perm-gate-${kind}`}
      role="dialog"
      aria-label={`${meta.label}權限`}
    >
      <div className="flex items-center gap-3">
        <div className="w-11 h-11 rounded-2xl bg-phantom-primary/15 flex items-center justify-center flex-shrink-0">
          <Icon size={22} className="text-phantom-primary" />
        </div>
        <div className="min-w-0">
          <h2 className="text-base font-semibold text-phantom-text">需要「{meta.label}」權限</h2>
          <p className="text-[11px] text-phantom-muted font-mono truncate">{meta.androidPermission}</p>
        </div>
      </div>

      <div className="space-y-1.5">
        <p className="text-sm text-phantom-text leading-relaxed">{meta.rationaleZh}</p>
        <p className="text-xs text-phantom-muted leading-relaxed">{meta.rationaleEn}</p>
      </div>

      {neverAskAgain ? (
        <>
          <div className="bg-phantom-warning/10 border border-phantom-warning/30 rounded-xl p-3 text-xs text-phantom-text leading-relaxed">
            系統已不再顯示授權視窗。請到「設定 → 應用程式 → Phantom Mesh → 權限」手動開啟「{meta.label}」。
          </div>
          <button
            onClick={() => {
              void openSettings().then((ok) => {
                // After returning from settings, re-read on next focus tick;
                // also nudge immediately in case visibility didn't change.
                if (ok) setTimeout(refresh, 500);
              });
            }}
            className="w-full flex items-center justify-center gap-2 bg-phantom-primary text-phantom-bg py-2.5 rounded-xl text-sm font-medium hover:brightness-110 transition"
          >
            <Settings size={16} /> 前往系統設定
          </button>
        </>
      ) : (
        <button
          onClick={() => void handlePrimary()}
          disabled={requesting}
          className="w-full flex items-center justify-center gap-2 bg-phantom-primary text-phantom-bg py-2.5 rounded-xl text-sm font-medium hover:brightness-110 transition disabled:opacity-60"
        >
          {requesting ? <Loader2 size={16} className="animate-spin" /> : <ShieldCheck size={16} />}
          {requesting ? "等待授權…" : `允許使用${meta.label}`}
        </button>
      )}

      <p className="text-[11px] text-phantom-muted text-center">{meta.fallbackZh}</p>
    </div>
  );

  if (optional) {
    return (
      <div className="space-y-3">
        {card}
        {children}
      </div>
    );
  }
  return card;
}
