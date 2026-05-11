import { useState, useEffect, useCallback, useRef } from "react";
import { safeInvoke as invoke } from "../lib/tauri-compat";
import { RefreshCw } from "lucide-react";
import UrlBar from "../components/browser/UrlBar";
import ScreenshotView from "../components/browser/ScreenshotView";
import ActionLog, { type BrowserAction } from "../components/browser/ActionLog";

interface NavigateResult {
  success: boolean;
  output: string;
  screenshot_path: string | null;
}

interface StatusResult {
  active: boolean;
  current_url: string | null;
}

interface SnapshotResult {
  success: boolean;
  text: string;
}

function timeNow(): string {
  return new Date().toLocaleTimeString("zh-TW", { hour: "2-digit", minute: "2-digit" });
}

export default function Browser() {
  const [currentUrl, setCurrentUrl] = useState("");
  const [screenshotPath, setScreenshotPath] = useState<string | null>(null);
  const [pageText, setPageText] = useState<string | null>(null);
  const [actions, setActions] = useState<BrowserAction[]>([]);
  const [loading, setLoading] = useState(false);
  const [active, setActive] = useState(false);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const lastUrlRef = useRef("");

  const addAction = useCallback((action: string, detail?: string) => {
    setActions(prev => [...prev, { time: timeNow(), action, detail }]);
  }, []);

  // Poll browser status every 3s
  useEffect(() => {
    const poll = async () => {
      try {
        const status = await invoke<StatusResult>("browser_status");
        setActive(status.active);
        if (status.current_url && status.current_url !== lastUrlRef.current) {
          lastUrlRef.current = status.current_url;
          setCurrentUrl(status.current_url);
          try {
            const path = await invoke<string>("browser_screenshot");
            setScreenshotPath(path);
          } catch { /* ignore */ }
        }
      } catch { /* ignore */ }
    };
    pollRef.current = setInterval(poll, 3000);
    poll();
    return () => { if (pollRef.current) clearInterval(pollRef.current); };
  }, []);

  const navigate = async (url: string) => {
    setLoading(true);
    addAction("navigate", url);
    try {
      const result = await invoke<NavigateResult>("browser_navigate", { url });
      if (result.success) {
        setCurrentUrl(url);
        lastUrlRef.current = url;
        setActive(true);
        if (result.screenshot_path) {
          setScreenshotPath(result.screenshot_path);
          addAction("screenshot", "done");
        }
        try {
          const snap = await invoke<SnapshotResult>("browser_snapshot");
          if (snap.success) setPageText(snap.text);
        } catch { /* ignore */ }
      } else {
        addAction("error", result.output);
      }
    } catch (e) {
      addAction("error", String(e));
    } finally {
      setLoading(false);
    }
  };

  const refresh = async () => {
    setLoading(true);
    addAction("refresh", "screenshot");
    try {
      const path = await invoke<string>("browser_screenshot");
      setScreenshotPath(path);
      const snap = await invoke<SnapshotResult>("browser_snapshot");
      if (snap.success) setPageText(snap.text);
    } catch (e) {
      addAction("error", String(e));
    } finally {
      setLoading(false);
    }
  };

  const closeBrowser = async () => {
    try {
      await invoke("browser_close");
    } catch { /* ignore */ }
    setActive(false);
    setScreenshotPath(null);
    setCurrentUrl("");
    lastUrlRef.current = "";
    setPageText(null);
    addAction("close", "session ended");
  };

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between mb-4">
        <h1 className="text-2xl font-bold">瀏覽器</h1>
        {active && (
          <button
            onClick={refresh}
            disabled={loading}
            className="flex items-center gap-2 border border-phantom-border text-phantom-muted px-3 py-1.5 rounded text-sm hover:text-phantom-text disabled:opacity-50"
          >
            <RefreshCw size={14} className={loading ? "animate-spin" : ""} />
            重新整理
          </button>
        )}
      </div>

      <UrlBar currentUrl={currentUrl} onNavigate={navigate} onClose={closeBrowser} loading={loading} />

      <div className="flex gap-4 flex-1 min-h-0">
        <div className="flex-[2] min-w-0">
          <ScreenshotView screenshotPath={screenshotPath} loading={loading} />
        </div>
        <div className="flex-[1] min-w-0 bg-phantom-card border border-phantom-border rounded-lg p-3">
          <ActionLog actions={actions} pageText={pageText} />
        </div>
      </div>
    </div>
  );
}
