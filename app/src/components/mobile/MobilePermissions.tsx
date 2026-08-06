// SPEC-33 §11/§15 + §15.4 — central permission & background-survival panel.
//
// One screen to review/grant the three runtime permissions (RECORD_AUDIO,
// CAMERA, POST_NOTIFICATIONS) and, for MIUI devices, deep-link to the autostart
// + battery-optimization whitelists that keep the focus foreground service
// alive overnight (SPEC-33 §15.4 / SPEC-34 §10F). Reached from Settings.
//
// The permission rows reuse `usePermission` so the status reflects the real OS
// state and the buttons drive the same Web-API bridge as the inline gates.

import { useState } from "react";
import {
  Mic, Camera, Bell, Settings, ShieldCheck, CheckCircle2, XCircle,
  HelpCircle, BatteryCharging, RefreshCw, Loader2,
} from "lucide-react";
import {
  PERMISSION_META, type PermissionKind, type PermissionStatus,
} from "../../lib/permissions";
import { usePermission } from "../permissions/usePermission";
import { safeInvoke } from "../../lib/tauri-compat";

const ICON: Record<PermissionKind, typeof Mic> = {
  microphone: Mic,
  camera: Camera,
  notifications: Bell,
};

const KINDS: PermissionKind[] = ["microphone", "camera", "notifications"];

function StatusBadge({ status }: { status: PermissionStatus | "unknown" }) {
  if (status === "granted") {
    return (
      <span className="inline-flex items-center gap-1 text-[11px] text-spectyn-success">
        <CheckCircle2 size={13} /> 已授權
      </span>
    );
  }
  if (status === "denied") {
    return (
      <span className="inline-flex items-center gap-1 text-[11px] text-spectyn-warning">
        <XCircle size={13} /> 已拒絕
      </span>
    );
  }
  if (status === "unsupported") {
    return (
      <span className="inline-flex items-center gap-1 text-[11px] text-spectyn-muted">
        <HelpCircle size={13} /> 不適用
      </span>
    );
  }
  if (status === "unknown") {
    return <Loader2 size={13} className="text-spectyn-muted animate-spin" />;
  }
  return (
    <span className="inline-flex items-center gap-1 text-[11px] text-spectyn-muted">
      <HelpCircle size={13} /> 尚未決定
    </span>
  );
}

function PermissionRow({ kind }: { kind: PermissionKind }) {
  const meta = PERMISSION_META[kind];
  const Icon = ICON[kind];
  const { status, neverAskAgain, requesting, request, openSettings, refresh } =
    usePermission(kind);

  const granted = status === "granted" || status === "unsupported";

  return (
    <div className="bg-spectyn-card border border-spectyn-border rounded-2xl p-4 space-y-3" data-testid={`perm-row-${kind}`}>
      <div className="flex items-center gap-3">
        <div className="w-10 h-10 rounded-2xl bg-spectyn-primary/15 flex items-center justify-center flex-shrink-0">
          <Icon size={19} className="text-spectyn-primary" />
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium text-spectyn-text">{meta.label}</span>
            <StatusBadge status={status} />
          </div>
          <p className="text-[11px] text-spectyn-muted font-mono truncate">{meta.androidPermission}</p>
        </div>
      </div>

      <p className="text-xs text-spectyn-muted leading-relaxed">{meta.rationaleZh}</p>

      {!granted && (
        neverAskAgain ? (
          <button
            onClick={() => void openSettings().then((ok) => ok && setTimeout(refresh, 500))}
            className="w-full flex items-center justify-center gap-2 bg-spectyn-bg border border-spectyn-border text-spectyn-text py-2 rounded-xl text-sm hover:border-spectyn-primary/40 transition"
          >
            <Settings size={15} /> 前往系統設定開啟
          </button>
        ) : (
          <button
            onClick={() => void request()}
            disabled={requesting}
            className="w-full flex items-center justify-center gap-2 bg-spectyn-primary text-spectyn-bg py-2 rounded-xl text-sm font-medium hover:brightness-110 transition disabled:opacity-60"
          >
            {requesting ? <Loader2 size={15} className="animate-spin" /> : <ShieldCheck size={15} />}
            {requesting ? "等待授權…" : `允許使用${meta.label}`}
          </button>
        )
      )}
    </div>
  );
}

// SPEC-33 §15.4 — MIUI deep-links. No public API can set these, so we can only
// jump the user to the right settings page. The Tauri commands are best-effort:
// if the Rust side hasn't wired them yet, safeInvoke throws and we surface a
// hint instead of crashing.
async function tryMiuiCommand(cmd: string): Promise<"ok" | "unavailable"> {
  try {
    await safeInvoke(cmd, {});
    return "ok";
  } catch {
    return "unavailable";
  }
}

function MiuiGuide() {
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const run = async (cmd: string, label: string) => {
    setBusy(cmd);
    setNote(null);
    const res = await tryMiuiCommand(cmd);
    setBusy(null);
    setNote(
      res === "ok"
        ? `已開啟「${label}」設定頁`
        : `無法自動開啟「${label}」。請手動到「安全中心 → 應用管理 → ${label}」設定。`,
    );
  };

  return (
    <div className="bg-spectyn-card border border-spectyn-border rounded-2xl p-4 space-y-3">
      <div className="flex items-center gap-2">
        <BatteryCharging size={17} className="text-spectyn-warning" />
        <h3 className="text-sm font-medium text-spectyn-text">小米 / Redmi 背景存活</h3>
      </div>
      <p className="text-xs text-spectyn-muted leading-relaxed">
        MIUI（小米客製系統）預設會在過夜時殺掉背景 app，導致焦點計時中斷、教練回顧推播收不到。
        把 Spectyn 加入「自啟白名單」與「電池優化白名單」可避免（SPEC-33 §15.4）。
      </p>
      <div className="grid grid-cols-2 gap-2">
        <button
          onClick={() => void run("miui_guide_open_autostart", "自啟動")}
          disabled={busy !== null}
          className="flex items-center justify-center gap-1.5 bg-spectyn-bg border border-spectyn-border text-spectyn-text py-2 rounded-xl text-xs hover:border-spectyn-primary/40 transition disabled:opacity-60"
        >
          {busy === "miui_guide_open_autostart" ? <Loader2 size={13} className="animate-spin" /> : <RefreshCw size={13} />}
          自啟白名單
        </button>
        <button
          onClick={() => void run("miui_guide_open_battery_optimization", "電池優化")}
          disabled={busy !== null}
          className="flex items-center justify-center gap-1.5 bg-spectyn-bg border border-spectyn-border text-spectyn-text py-2 rounded-xl text-xs hover:border-spectyn-primary/40 transition disabled:opacity-60"
        >
          {busy === "miui_guide_open_battery_optimization" ? <Loader2 size={13} className="animate-spin" /> : <BatteryCharging size={13} />}
          電池優化
        </button>
      </div>
      {note && (
        <p className="text-[11px] text-spectyn-muted leading-relaxed bg-spectyn-bg border border-spectyn-border rounded-xl p-2.5">
          {note}
        </p>
      )}
    </div>
  );
}

export default function MobilePermissions() {
  return (
    <div className="space-y-3" data-testid="mobile-permissions">
      <p className="text-xs text-spectyn-muted leading-relaxed px-1">
        Spectyn 只在用到時才請求權限，且全部可拒絕後仍以降級模式運作（SPEC-33 §15.2）。
      </p>
      {KINDS.map((k) => (
        <PermissionRow key={k} kind={k} />
      ))}
      <MiuiGuide />
    </div>
  );
}
