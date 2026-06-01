import { useEffect, useState } from "react";
import { BellOff } from "lucide-react";

// SPEC-34 Screen 17 (Lock / 拒絕通知 fallback): when the user denied
// POST_NOTIFICATIONS but the node service is still running, show a static
// card so the app doesn't look dead. Copy per SPEC-33 §10.4. Modal overlay,
// no route — rendered by MobileShell; self-gates on the live permission state.

const ACK_KEY = "notif_perm_acknowledged";

export default function NotifLockCard() {
  const [show, setShow] = useState(false);

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        if (localStorage.getItem(ACK_KEY) === "true") return;
      } catch { return; }
      try {
        const { isPermissionGranted } = await import("@tauri-apps/plugin-notification");
        const granted = await isPermissionGranted();
        if (alive && !granted) setShow(true);
      } catch {
        // Not a Tauri runtime (web/dev) or plugin unavailable — never show.
      }
    })();
    return () => { alive = false; };
  }, []);

  if (!show) return null;

  const goSettings = async () => {
    try {
      const { requestPermission } = await import("@tauri-apps/plugin-notification");
      const res = await requestPermission();
      if (res === "granted") {
        try { localStorage.removeItem(ACK_KEY); } catch { /* ignore */ }
        setShow(false);
      }
    } catch {
      // permanently denied / no prompt — leave the card; user can dismiss below
    }
  };

  const useInAppCards = () => {
    try { localStorage.setItem(ACK_KEY, "true"); } catch { /* ignore */ }
    setShow(false);
  };

  return (
    <div
      role="alertdialog"
      aria-modal="true"
      aria-label="無通知權限"
      className="fixed inset-0 z-[60] flex items-end justify-center bg-black/50 p-4"
    >
      <div className="w-full max-w-sm bg-phantom-card border border-phantom-border rounded-xl p-5 mb-[env(safe-area-inset-bottom)]">
        <div className="flex items-center gap-2">
          <BellOff size={20} className="text-phantom-muted" aria-label="通知關閉圖示" />
          <h2 className="text-sm font-semibold text-phantom-text">無通知權限</h2>
        </div>
        <p className="text-xs text-phantom-muted mt-2 leading-relaxed">
          Phantom 無法發送通知;改以 app 內卡片顯示。可在設定隨時恢復。
        </p>
        <div className="flex gap-2 mt-4">
          <button
            onClick={() => void goSettings()}
            className="flex-1 bg-phantom-primary/15 text-phantom-primary py-2.5 rounded-lg text-sm font-medium hover:bg-phantom-primary/25 transition"
          >
            去設定
          </button>
          <button
            onClick={useInAppCards}
            className="flex-1 bg-phantom-bg border border-phantom-border text-phantom-text py-2.5 rounded-lg text-sm hover:border-phantom-primary transition"
          >
            先用 app 內卡片
          </button>
        </div>
      </div>
    </div>
  );
}
