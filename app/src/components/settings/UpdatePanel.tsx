import { useState } from "react";
import { check, Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { RefreshCw, Download, CheckCircle, AlertCircle, Loader2 } from "lucide-react";

type State =
  | { type: "idle" }
  | { type: "checking" }
  | { type: "upToDate" }
  | { type: "available"; update: Update }
  | { type: "downloading"; progress: number }
  | { type: "error"; message: string };

export default function UpdatePanel() {
  const [state, setState] = useState<State>({ type: "idle" });

  async function handleCheck() {
    setState({ type: "checking" });
    try {
      const update = await check();
      if (update?.available) {
        setState({ type: "available", update });
      } else {
        setState({ type: "upToDate" });
      }
    } catch (e) {
      setState({ type: "error", message: String(e) });
    }
  }

  async function handleInstall(update: Update) {
    setState({ type: "downloading", progress: 0 });
    try {
      let downloaded = 0;
      let total = 0;
      await update.downloadAndInstall((event: import("@tauri-apps/plugin-updater").DownloadEvent) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? 0;
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          const pct = total > 0 ? Math.round((downloaded / total) * 100) : 0;
          setState({ type: "downloading", progress: pct });
        } else if (event.event === "Finished") {
          setState({ type: "downloading", progress: 100 });
        }
      });
      await relaunch();
    } catch (e) {
      setState({ type: "error", message: String(e) });
    }
  }

  // `__APP_VERSION__` is replaced at build time by the Vite `define` (see
  // vite.config.ts). It's a bare identifier, not a string literal, because
  // Vite's define only substitutes identifiers — a quoted "__APP_VERSION__"
  // would be left untouched and render literally.
  const appVersion = __APP_VERSION__;

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-lg font-semibold text-spectyn-text">應用程式更新</h2>
        <p className="text-sm text-spectyn-muted mt-1">目前版本：v{appVersion}</p>
      </div>

      <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-6 space-y-4">
        {state.type === "idle" && (
          <div className="text-center space-y-3">
            <RefreshCw size={32} className="mx-auto text-spectyn-muted" />
            <p className="text-sm text-spectyn-muted">點擊下方按鈕檢查是否有新版本</p>
          </div>
        )}

        {state.type === "checking" && (
          <div className="text-center space-y-3">
            <Loader2 size={32} className="mx-auto text-spectyn-primary animate-spin" />
            <p className="text-sm text-spectyn-muted">正在檢查更新...</p>
          </div>
        )}

        {state.type === "upToDate" && (
          <div className="text-center space-y-3">
            <CheckCircle size={32} className="mx-auto text-green-500" />
            <p className="text-sm text-spectyn-text font-medium">已是最新版本</p>
          </div>
        )}

        {state.type === "available" && (
          <div className="space-y-4">
            <div className="flex items-start gap-3">
              <Download size={20} className="text-spectyn-primary mt-0.5 flex-shrink-0" />
              <div>
                <p className="text-sm font-medium text-spectyn-text">
                  新版本 {state.update.version} 可以更新
                </p>
                {state.update.body && (
                  <p className="text-xs text-spectyn-muted mt-1 whitespace-pre-wrap">
                    {state.update.body}
                  </p>
                )}
              </div>
            </div>
            <button
              onClick={() => handleInstall(state.update)}
              className="w-full py-2 px-4 bg-spectyn-primary text-white rounded-lg text-sm font-medium hover:bg-spectyn-primary/90 transition-colors"
            >
              立即下載並安裝
            </button>
          </div>
        )}

        {state.type === "downloading" && (
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <Loader2 size={16} className="text-spectyn-primary animate-spin flex-shrink-0" />
              <p className="text-sm text-spectyn-text">
                {state.progress < 100 ? `下載中 ${state.progress}%` : "安裝中，即將重啟..."}
              </p>
            </div>
            <div className="h-2 bg-spectyn-border rounded-full overflow-hidden">
              <div
                className="h-full bg-spectyn-primary transition-all duration-300"
                style={{ width: `${state.progress}%` }}
              />
            </div>
          </div>
        )}

        {state.type === "error" && (
          <div className="space-y-3">
            <div className="flex items-start gap-3">
              <AlertCircle size={20} className="text-red-500 flex-shrink-0 mt-0.5" />
              <p className="text-sm text-spectyn-muted">{state.message}</p>
            </div>
          </div>
        )}

        {(state.type === "idle" || state.type === "upToDate" || state.type === "error") && (
          <button
            onClick={handleCheck}
            className="w-full py-2 px-4 border border-spectyn-border rounded-lg text-sm text-spectyn-text hover:bg-spectyn-card/80 transition-colors"
          >
            檢查更新
          </button>
        )}
      </div>

      <div className="text-xs text-spectyn-muted space-y-1">
        <p>• Android：更新後需手動安裝新 APK</p>
        <p>• iOS：透過 TestFlight 自動推送更新通知</p>
        <p>• Linux/桌面：下載後自動重啟套用</p>
      </div>
    </div>
  );
}
