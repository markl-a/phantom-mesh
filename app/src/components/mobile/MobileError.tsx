import { useParams } from "react-router-dom";
import { AlertTriangle, RotateCcw, Flag, Trash2 } from "lucide-react";

// SPEC-34 Screen 16 (Error / 通用錯誤頁): unrecoverable-error fallback
// (e.g. KeyStore corrupt / config damaged). Standalone full-screen route
// /error/:code — error boundaries / handlers navigate here. Backend-free.
//
// MobileErrorView is the presentational shell, shared by the /error/:code
// route (below) and ErrorBoundary's crash fallback (which can't use router
// hooks). Keep it hook-free apart from window.* so the boundary can render it.

export function MobileErrorView({ code }: { code: string }) {
  const errorCode = code || "UNKNOWN";

  // External link via <a target=_blank> (the codebase deliberately avoids
  // @tauri-apps/plugin-shell; window.open is unreliable in the Android WebView).
  const reportUrl =
    "https://github.com/markl-a/spectyn-mesh/issues/new?title=" +
    encodeURIComponent(`[android] error ${errorCode}`);

  const retry = () => window.location.assign("/");

  const reset = () => {
    if (!window.confirm("確定要重設並重新設定 app 嗎？此動作會清除本機 onboarding 狀態。")) return;
    try {
      localStorage.removeItem("spectyn_mesh_v2_onboarded");
      localStorage.removeItem("spectyn_mesh_v2_onboarded_mode");
    } catch { /* localStorage may be restricted */ }
    window.location.assign("/");
  };

  return (
    <div className="flex flex-col items-center justify-center h-[100dvh] bg-spectyn-bg px-6 text-center">
      <AlertTriangle
        size={56}
        className="text-spectyn-danger mb-4"
        aria-label="錯誤圖示"
      />
      <h1 className="text-lg font-semibold text-spectyn-text">發生未預期錯誤</h1>
      <p className="text-sm text-spectyn-muted mt-2 max-w-xs">
        Spectyn 遇到無法自動復原的問題。你可以重試、回報，或重設後重新設定。
      </p>
      <code className="text-xs text-spectyn-muted font-mono mt-3">code: {errorCode}</code>

      <div className="flex flex-col gap-2 w-full max-w-xs mt-6">
        <button
          onClick={retry}
          className="flex items-center justify-center gap-2 bg-spectyn-primary/15 text-spectyn-primary py-2.5 rounded-lg text-sm font-medium hover:bg-spectyn-primary/25 transition"
        >
          <RotateCcw size={16} /> 重試
        </button>
        <a
          href={reportUrl}
          target="_blank"
          rel="noreferrer noopener"
          aria-label="回報問題，外部連結"
          className="flex items-center justify-center gap-2 bg-spectyn-card border border-spectyn-border text-spectyn-text py-2.5 rounded-lg text-sm hover:border-spectyn-primary transition"
        >
          <Flag size={16} /> 回報問題
        </a>
        <button
          onClick={reset}
          className="flex items-center justify-center gap-2 bg-spectyn-card border border-spectyn-border text-spectyn-danger py-2.5 rounded-lg text-sm hover:border-spectyn-danger transition"
        >
          <Trash2 size={16} /> 重設並重新設定
        </button>
      </div>
    </div>
  );
}

// Route wrapper: reads :code from the URL and renders the shared view.
export default function MobileError() {
  const { code } = useParams<{ code: string }>();
  return <MobileErrorView code={code ?? "UNKNOWN"} />;
}
