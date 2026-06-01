// J1 Step 2 — Permissions (SPEC-34 Screen 1 / SPEC-33 §11). Requests
// CAMERA + RECORD_AUDIO + POST_NOTIFICATIONS, each with a one-line rationale.
// All are non-blocking: 稍後再說 advances with whatever was granted.
import { useState } from "react";
import {
  requestPermission,
  openAppSettings,
  PERMISSION_META,
  type PermissionKind,
  type PermissionStatus,
} from "../../../lib/permissions";

const KINDS: PermissionKind[] = ["camera", "microphone", "notifications"];
const RATIONALE: Record<PermissionKind, string> = {
  camera: "拍照記錄飲食（可稍後再開）",
  microphone: "語音記錄與專注時段（可稍後再開）",
  notifications: "習慣/專注提醒與背景節點狀態",
};

export default function PermissionsStep({ onNext }: { onNext: () => void }) {
  const [status, setStatus] = useState<Partial<Record<PermissionKind, PermissionStatus>>>({});
  const [busy, setBusy] = useState<PermissionKind | null>(null);

  const ask = async (k: PermissionKind) => {
    setBusy(k);
    const res = await requestPermission(k);
    setStatus((s) => ({ ...s, [k]: res.status }));
    if (res.neverAskAgain) await openAppSettings();
    setBusy(null);
  };

  return (
    <div className="px-6 space-y-4">
      <h2 className="text-lg font-bold text-phantom-text text-center">需要幾項權限</h2>
      <div className="space-y-2">
        {KINDS.map((k) => {
          const st = status[k];
          return (
            <div
              key={k}
              className="flex items-center gap-3 bg-phantom-card border border-phantom-border rounded-lg p-3"
              aria-label={`${PERMISSION_META[k].label}，${st ?? "未決定"}，${RATIONALE[k]}`}
            >
              <div className="flex-1 min-w-0">
                <p className="text-sm text-phantom-text">{PERMISSION_META[k].label}</p>
                <p className="text-xs text-phantom-muted">{RATIONALE[k]}</p>
                {st === "denied" && (
                  <p className="text-[11px] text-phantom-warning mt-1">
                    請到 系統設定 → 應用程式 → Phantom Mesh 開啟權限
                  </p>
                )}
              </div>
              <button
                onClick={() => void ask(k)}
                disabled={busy === k || st === "granted"}
                className="text-xs px-3 py-1.5 rounded border border-phantom-border text-phantom-primary disabled:opacity-50"
              >
                {st === "granted" ? "✓" : busy === k ? "…" : "允許"}
              </button>
            </div>
          );
        })}
      </div>
      <button
        onClick={onNext}
        className="w-full bg-phantom-primary text-phantom-bg py-2.5 rounded-lg text-sm font-medium hover:brightness-110 transition"
      >
        下一步
      </button>
      <button onClick={onNext} className="w-full text-xs text-phantom-muted">
        稍後再說
      </button>
    </div>
  );
}
