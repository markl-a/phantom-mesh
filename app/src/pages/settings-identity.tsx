// SPEC-31 settings-identity — route /settings/identity; commands_used: identity_* (via lib/identity.ts)
//
// Mobile identity settings. Shows this device's cryptographic identity (display
// name + public-key fingerprint, monospace) read from lib/identity.ts, which
// wraps the read-only `identity_status` Tauri command. Import / Export actions
// are rendered DISABLED with a 尚未實作 (not implemented) note because the lib
// exposes no import/export helpers today — we never invent a backend command.
//
// Honesty contract: a thrown/rejected load → error state (role=alert); a
// null/empty result (browser/web mode, or no identity on device) → honest empty
// state, NOT faked as an error. Async load uses a sequence guard + alive ref so
// only the latest live request commits, and data is cleared at load start so a
// remount can't leave stale content on screen.

import { useEffect, useRef, useState } from "react";
import {
  Fingerprint,
  RefreshCw,
  Download,
  Upload,
  AlertTriangle,
  ShieldAlert,
} from "lucide-react";
import { loadIdentityStatus, type IdentityStatus } from "../lib/identity";
import { useHaptics } from "../lib/useHaptics";

type LoadState = "loading" | "ready" | "error";

export default function SettingsIdentity() {
  const { impact } = useHaptics();

  const [state, setState] = useState<LoadState>("loading");
  const [identity, setIdentity] = useState<IdentityStatus | null>(null);
  const [error, setError] = useState<string>("");

  // Sequence guard: only the latest live request may commit state. `alive`
  // guards against setState-after-unmount.
  const seqRef = useRef(0);
  const aliveRef = useRef(true);

  const runLoad = () => {
    const seq = ++seqRef.current;
    // Clear prior data at load start so a re-trigger can't leave stale content.
    setState("loading");
    setIdentity(null);
    setError("");

    void loadIdentityStatus()
      .then((res) => {
        if (!aliveRef.current || seq !== seqRef.current) return;
        setIdentity(res);
        setState("ready");
      })
      .catch((e: unknown) => {
        if (!aliveRef.current || seq !== seqRef.current) return;
        setError(e instanceof Error ? e.message : String(e));
        setState("error");
      });
  };

  useEffect(() => {
    aliveRef.current = true;
    runLoad();
    return () => {
      aliveRef.current = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleRefresh = () => {
    runLoad();
  };

  const handleReload = () => {
    impact("medium");
    runLoad();
  };

  // A non-null status that reports hasIdentity:false is a real, honest empty
  // state (device exists but has no key yet) — distinct from null (no backend).
  const hasKey = identity != null && identity.hasIdentity && (identity.fingerprint?.length ?? 0) > 0;

  return (
    <div
      data-testid="settings-identity"
      className="min-h-screen flex flex-col bg-phantom-bg text-phantom-text
        pt-[env(safe-area-inset-top)]
        pl-[env(safe-area-inset-left)] pr-[env(safe-area-inset-right)]"
    >
      {/* Header */}
      <header className="flex items-center gap-3 px-4 pt-4 pb-3 border-b border-phantom-border">
        <div className="w-10 h-10 rounded-lg bg-phantom-primary/15 flex items-center justify-center flex-shrink-0">
          <Fingerprint size={20} className="text-phantom-primary" aria-hidden="true" />
        </div>
        <div className="flex-1 min-w-0">
          <h1 className="text-lg font-bold text-phantom-text">裝置識別碼 Device identity</h1>
          <p className="text-sm text-phantom-muted mt-0.5 truncate">
            本機加密身分 This device&apos;s cryptographic identity
          </p>
        </div>
        <button
          type="button"
          onClick={handleRefresh}
          disabled={state === "loading"}
          aria-label="重新整理識別碼 Refresh identity"
          className="flex items-center gap-1.5 min-h-[44px] px-3 rounded-lg text-base
            bg-phantom-card border border-phantom-border text-phantom-text
            hover:border-phantom-primary/40 transition motion-reduce:transition-none
            disabled:opacity-60 flex-shrink-0"
        >
          <RefreshCw
            size={16}
            aria-hidden="true"
            className={state === "loading" ? "animate-spin motion-reduce:animate-none" : ""}
          />
          <span className="hidden sm:inline">重新整理</span>
        </button>
      </header>

      {/* Scrollable body — DOM order = visual order */}
      <main className="flex-1 overflow-y-auto px-4 py-4 space-y-4">
        {/* Error state (a rejected backend call) */}
        {state === "error" && (
          <div
            role="alert"
            className="bg-phantom-danger/10 border border-phantom-danger/40 rounded-lg p-3 text-base text-phantom-danger"
          >
            無法讀取識別碼：{error}
            <span className="block text-sm opacity-80 mt-1">Failed to read identity.</span>
          </div>
        )}

        {/* Loading (first load / reload, no data yet) */}
        {state === "loading" && (
          <div
            role="status"
            className="flex items-center justify-center gap-2 min-h-[44px] text-base text-phantom-muted py-8"
          >
            <RefreshCw
              size={18}
              aria-hidden="true"
              className="animate-spin motion-reduce:animate-none"
            />
            載入中… Loading…
          </div>
        )}

        {/* Honest empty state: backend unavailable (null) */}
        {state === "ready" && identity == null && (
          <div className="bg-phantom-card border border-phantom-border rounded-lg p-6 text-center">
            <p className="text-base text-phantom-text">無法取得識別碼 Identity unavailable</p>
            <p className="text-sm text-phantom-muted mt-1.5">
              此環境沒有可用的裝置後端（例如純瀏覽器模式）。
              <span className="block mt-0.5">
                No device backend available here (e.g. browser-only mode).
              </span>
            </p>
          </div>
        )}

        {/* Honest empty state: backend present but no key yet */}
        {state === "ready" && identity != null && !hasKey && (
          <div className="bg-phantom-card border border-phantom-border rounded-lg p-6 text-center">
            <p className="text-base text-phantom-text">尚未建立識別碼 No identity yet</p>
            <p className="text-sm text-phantom-muted mt-1.5">
              這台裝置還沒有加密身分金鑰。
              <span className="block mt-0.5">This device has no cryptographic key yet.</span>
            </p>
          </div>
        )}

        {/* Identity detail card */}
        {state === "ready" && identity != null && hasKey && (
          <section
            aria-label="識別碼詳情 Identity details"
            className="bg-phantom-card border border-phantom-border rounded-lg p-4 space-y-4"
          >
            {/* Display name / identity line */}
            <div>
              <p className="text-sm text-phantom-muted">顯示名稱 Display name</p>
              <p className="text-base text-phantom-text mt-1 break-words">
                {identity.identityLine ?? "（未命名 Unnamed）"}
              </p>
            </div>

            {/* Fingerprint (monospace) */}
            <div>
              <p className="text-sm text-phantom-muted">公鑰指紋 Public-key fingerprint</p>
              <p className="text-base font-mono text-phantom-text mt-1 break-all leading-relaxed">
                {identity.fingerprint}
              </p>
            </div>

            {/* Keystore + created */}
            <div className="grid grid-cols-1 gap-3">
              <div>
                <p className="text-sm text-phantom-muted">金鑰庫 Keystore</p>
                <p className="text-base font-mono text-phantom-text mt-1 break-all">
                  {identity.keystore || "—"}
                </p>
              </div>
              <div>
                <p className="text-sm text-phantom-muted">建立時間 Created</p>
                <p className="text-base text-phantom-text mt-1 break-words">
                  {identity.createdAt || "—"}
                </p>
              </div>
            </div>
          </section>
        )}

        {/* Sensitive-action warning before destructive import/export */}
        <div
          className="flex items-start gap-2 bg-phantom-warning/10 border border-phantom-warning/40
            rounded-lg p-3 text-sm text-phantom-warning"
        >
          <AlertTriangle size={18} aria-hidden="true" className="flex-shrink-0 mt-0.5" />
          <p>
            敏感操作：匯出會把私密金鑰寫出裝置；匯入會覆寫現有身分且無法復原。請小心保管識別碼檔案。
            <span className="block mt-1 opacity-80">
              Sensitive: exporting writes your private key off-device; importing overwrites the
              current identity and cannot be undone. Keep identity files safe.
            </span>
          </p>
        </div>

        {/* Import / Export — no lib helpers exist, so disabled + 尚未實作 */}
        <section aria-label="識別碼匯入匯出 Identity import and export" className="space-y-2">
          <button
            type="button"
            disabled
            aria-label="匯入識別碼（尚未實作）Import identity (not implemented)"
            className="w-full flex items-center gap-2 min-h-[44px] px-3 rounded-lg text-base
              bg-phantom-card border border-phantom-border text-phantom-muted
              opacity-60 cursor-not-allowed"
          >
            <Upload size={18} aria-hidden="true" className="flex-shrink-0" />
            <span className="flex-1 text-left">匯入識別碼 / Import</span>
            <span className="text-xs text-phantom-muted flex-shrink-0">尚未實作 N/A</span>
          </button>
          <button
            type="button"
            disabled
            aria-label="匯出識別碼（尚未實作）Export identity (not implemented)"
            className="w-full flex items-center gap-2 min-h-[44px] px-3 rounded-lg text-base
              bg-phantom-card border border-phantom-border text-phantom-muted
              opacity-60 cursor-not-allowed"
          >
            <Download size={18} aria-hidden="true" className="flex-shrink-0" />
            <span className="flex-1 text-left">匯出識別碼 / Export</span>
            <span className="text-xs text-phantom-muted flex-shrink-0">尚未實作 N/A</span>
          </button>
          <p className="text-sm text-phantom-muted px-1">
            目前版本尚未提供識別碼匯入／匯出。
            <span className="block">Import / export is not available in this build yet.</span>
          </p>
        </section>
      </main>

      {/* Sticky bottom CTA (reachability) — reload identity */}
      <footer className="sticky bottom-0 px-4 pt-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] border-t border-phantom-border bg-phantom-bg">
        <button
          type="button"
          onClick={handleReload}
          disabled={state === "loading"}
          aria-label="重新載入識別碼 Reload identity"
          className="w-full min-h-[48px] flex items-center justify-center gap-2 rounded-xl
            bg-phantom-primary text-phantom-bg text-base font-semibold
            hover:opacity-90 transition motion-reduce:transition-none disabled:opacity-60"
        >
          <ShieldAlert size={18} aria-hidden="true" />
          重新載入識別碼 / Reload identity
        </button>
      </footer>
    </div>
  );
}
