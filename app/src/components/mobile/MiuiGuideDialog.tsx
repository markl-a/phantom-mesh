// SPEC-34 G6 / J5 — MIUI (小米系統) compatibility guide dialog.
//
// Shown when a MIUI/Redmi device's background-kill would reap phantom's
// foreground service overnight. Per SPEC-33 §6(E) phantom can only *guide* —
// it can't toggle the MIUI auto-start whitelist or battery deny-list itself.
// So the manual steps are the substance; the two deep-link buttons are a
// best-effort convenience that fall back to the steps when the native intent
// isn't available.
//
// Controlled: parent owns `open` + `onClose`. "不再提示" persists the
// dont-show-again flag (SPEC-34 §9 miui_guide_dismiss) before closing.

import { useEffect, useState } from "react";
import { X, Battery, Power, ExternalLink } from "lucide-react";
import {
  checkShouldShowMiuiGuide,
  dismissMiuiGuide,
  openMiuiAutostart,
  openMiuiBatteryOptimization,
} from "../../lib/miuiGuide";

export default function MiuiGuideDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  // When a deep-link launch fails (native intent not wired / MIUI changed the
  // API), surface the manual steps for that target instead of silently doing
  // nothing.
  const [autostartFell, setAutostartFell] = useState(false);
  const [batteryFell, setBatteryFell] = useState(false);
  // The dialog stays mounted (returns null when closed), so reset the
  // per-session fallback flags + in-flight guard whenever it (re)opens —
  // otherwise a prior session's warning lingers on reopen.
  const [busy, setBusy] = useState(false);
  // null = still checking / unknown; true/false = native detection result.
  const [isMiui, setIsMiui] = useState<boolean | null>(null);
  useEffect(() => {
    if (open) {
      setAutostartFell(false);
      setBatteryFell(false);
      setBusy(false);
      setIsMiui(null);
      let cancelled = false;
      void checkShouldShowMiuiGuide().then((r) => {
        if (!cancelled) setIsMiui(r.is_miui);
      });
      return () => {
        cancelled = true;
      };
    }
  }, [open]);

  if (!open) return null;

  // Guard against double-tap launching two Settings intents.
  const handleAutostart = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const ok = await openMiuiAutostart();
      if (!ok) setAutostartFell(true);
    } finally {
      setBusy(false);
    }
  };
  const handleBattery = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const ok = await openMiuiBatteryOptimization();
      if (!ok) setBatteryFell(true);
    } finally {
      setBusy(false);
    }
  };
  const handleDontShowAgain = async () => {
    await dismissMiuiGuide(true);
    onClose();
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/50 p-3"
      role="dialog"
      aria-modal="true"
      aria-labelledby="miui-guide-title"
      aria-describedby="miui-guide-desc"
      data-testid="miui-guide-dialog"
      tabIndex={-1}
      onClick={onClose}
      onKeyDown={(e) => {
        if (e.key === "Escape") onClose();
      }}
    >
      <div
        className="w-full max-w-md bg-phantom-bg border border-phantom-border rounded-2xl p-4 space-y-3 max-h-[85vh] overflow-y-auto"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-start justify-between gap-2">
          <h2
            id="miui-guide-title"
            className="text-base font-semibold text-phantom-text"
          >
            偵測到 MIUI（小米系統）
          </h2>
          <button
            onClick={onClose}
            className="text-phantom-muted hover:text-phantom-text p-1 -m-1"
            aria-label="關閉"
          >
            <X size={18} />
          </button>
        </div>

        <p id="miui-guide-desc" className="text-sm text-phantom-muted leading-relaxed">
          小米 / Redmi 預設會把背景 app 殺得比一般 Android 兇,phantom
          的背景服務(focus session、mesh 連線)可能過夜後就被關掉。建議把
          phantom 加入「自啟動」白名單 + 「電池優化」例外。
        </p>

        {isMiui !== null && (
          <div
            className={`text-xs rounded-lg px-3 py-2 ${
              isMiui
                ? "bg-phantom-primary/10 text-phantom-primary"
                : "bg-phantom-card text-phantom-muted"
            }`}
            data-testid="miui-detect-banner"
          >
            {isMiui
              ? "已偵測到 MIUI 系統 — 以下設定建議完成。"
              : "此裝置看起來不是小米 / Redmi — 以下步驟僅 MIUI 裝置需要。"}
          </div>
        )}

        {/* Autostart */}
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-3 space-y-2">
          <button
            onClick={() => void handleAutostart()}
            disabled={busy}
            className="w-full flex items-center gap-2 text-sm text-phantom-text font-medium disabled:opacity-50"
          >
            <Power size={16} className="text-phantom-primary" />
            自啟設定
            <ExternalLink size={13} className="text-phantom-muted ml-auto" />
          </button>
          <ol className="text-[11px] text-phantom-muted leading-relaxed list-decimal pl-4">
            <li>開「安全中心」→「應用管理」→「自啟動」</li>
            <li>找到 Phantom Mesh,打開開關</li>
          </ol>
          {autostartFell && (
            <p className="text-[11px] text-phantom-warning" role="alert">
              無法直接跳轉 — 請依上面步驟手動開啟。
            </p>
          )}
        </div>

        {/* Battery optimization */}
        <div className="bg-phantom-card border border-phantom-border rounded-lg p-3 space-y-2">
          <button
            onClick={() => void handleBattery()}
            disabled={busy}
            className="w-full flex items-center gap-2 text-sm text-phantom-text font-medium disabled:opacity-50"
          >
            <Battery size={16} className="text-phantom-primary" />
            電池設定
            <ExternalLink size={13} className="text-phantom-muted ml-auto" />
          </button>
          <ol className="text-[11px] text-phantom-muted leading-relaxed list-decimal pl-4">
            <li>開「設定」→「電池與效能」→「應用智慧省電」</li>
            <li>把 Phantom Mesh 設為「無限制」</li>
          </ol>
          {batteryFell && (
            <p className="text-[11px] text-phantom-warning" role="alert">
              無法直接跳轉 — 請依上面步驟手動設定。
            </p>
          )}
        </div>

        <div className="flex items-center justify-between gap-2 pt-1">
          <button
            onClick={() => void handleDontShowAgain()}
            className="text-xs text-phantom-muted hover:text-phantom-text underline"
            data-testid="miui-dont-show-again"
          >
            不再提示
          </button>
          <button
            onClick={onClose}
            className="bg-phantom-primary text-phantom-bg text-sm font-medium px-4 py-2 rounded-lg active:opacity-80"
          >
            完成
          </button>
        </div>
      </div>
    </div>
  );
}
