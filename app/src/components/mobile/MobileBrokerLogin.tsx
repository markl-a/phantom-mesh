// MobileBrokerLogin — UI for the iOS broker-login flow.
//
// Pairs with app/src/lib/brokerLogin.ts (JS bridge) and
// app/src-tauri/src/commands/broker_login.rs (Rust commands).
// Shows current login state if any, "Sign in" button otherwise.
// Listens for the deep-link callback so the UI flips to "logged in"
// the moment Safari hands the OAuth payload back to the app.

import { useEffect, useState } from "react";
import {
  startBrokerLogin,
  loadBrokerLoginStatus,
  logoutBroker,
  onBrokerLoginResult,
  syncFromVault,
  type BrokerLoginFinishResponse,
  type BrokerSyncResponse,
} from "../../lib/brokerLogin";

type Phase =
  | { kind: "loading" }
  | { kind: "logged-out" }
  | { kind: "redirecting" } // waiting for Safari → deep-link round-trip
  | { kind: "logged-in"; identity: BrokerLoginFinishResponse; sync?: BrokerSyncResponse }
  | { kind: "error"; message: string };

export default function MobileBrokerLogin() {
  const [phase, setPhase] = useState<Phase>({ kind: "loading" });

  // Bootstrap: load any existing AuthState.
  useEffect(() => {
    let cancelled = false;
    loadBrokerLoginStatus()
      .then((status) => {
        if (cancelled) return;
        setPhase(status ? { kind: "logged-in", identity: status } : { kind: "logged-out" });
      })
      .catch((e) => {
        if (cancelled) return;
        setPhase({ kind: "error", message: String(e) });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Subscribe to deep-link callback results — installed once globally
  // by main.tsx's installBrokerLoginBridge(); we just react to its events.
  useEffect(() => {
    const unsubscribe = onBrokerLoginResult((result) => {
      if (result.ok) {
        setPhase({ kind: "logged-in", identity: result.identity, sync: result.sync });
      } else {
        setPhase({ kind: "error", message: result.error });
      }
    });
    return unsubscribe;
  }, []);

  const handleResync = async () => {
    if (phase.kind !== "logged-in") return;
    try {
      const sync = await syncFromVault();
      setPhase({ kind: "logged-in", identity: phase.identity, sync });
    } catch (e) {
      setPhase({ kind: "error", message: `re-sync failed: ${String(e)}` });
    }
  };

  const handleSignIn = async () => {
    setPhase({ kind: "redirecting" });
    try {
      await startBrokerLogin("https://phantommesh.io");
      // The deep-link listener takes over; phase stays "redirecting"
      // until onBrokerLoginResult fires.
    } catch (e) {
      setPhase({ kind: "error", message: String(e) });
    }
  };

  const handleLogout = async () => {
    try {
      await logoutBroker();
      setPhase({ kind: "logged-out" });
    } catch (e) {
      setPhase({ kind: "error", message: String(e) });
    }
  };

  /** Save peer's URL into thin-shell localStorage keys + enable thin-shell
   *  + reload — mobileThinShell.ts then redirects WebView to <peer>/m
   *  where the coordinator handles chat. iOS becomes a pure UI shell. */
  const pickCoordinator = (peerUrl: string) => {
    try {
      const u = new URL(peerUrl);
      const host = u.hostname;
      const port = u.port || (u.protocol === "https:" ? "443" : "80");
      const scheme = u.protocol.replace(":", "");
      localStorage.setItem("PHANTOM_HOST", host);
      localStorage.setItem("PHANTOM_PORT", port);
      localStorage.setItem("PHANTOM_SCHEME", scheme);
      localStorage.setItem("PHANTOM_THIN_SHELL", "1");
      // Reload — main.tsx's thin-shell branch will redirect WebView to
      // <peer>/m on next boot.
      window.location.reload();
    } catch (e) {
      setPhase({ kind: "error", message: `Bad peer URL "${peerUrl}": ${String(e)}` });
    }
  };

  return (
    <div className="space-y-3">
      <div className="text-xs text-phantom-muted leading-relaxed">
        登入後 broker_token 會存到本機沙盒，之後 <code>/api/me/*</code>
        都用它呼叫 — 換手機重灌再登一次拿回同帳號的 cluster + LLM keys。
      </div>

      {phase.kind === "loading" && (
        <div className="text-sm text-phantom-muted">讀取登入狀態 …</div>
      )}

      {phase.kind === "logged-out" && (
        <button
          onClick={handleSignIn}
          className="w-full bg-phantom-primary text-phantom-bg font-medium px-4 py-3 rounded-lg active:opacity-80"
        >
          以 phantommesh.io 登入
        </button>
      )}

      {phase.kind === "redirecting" && (
        <div className="space-y-2">
          <div className="text-sm text-phantom-text">
            已開啟 Safari · 完成 Google / Apple 登入後會自動回到 app …
          </div>
          <button
            onClick={() => setPhase({ kind: "logged-out" })}
            className="text-xs text-phantom-muted underline"
          >
            取消
          </button>
        </div>
      )}

      {phase.kind === "logged-in" && (
        <div className="space-y-3">
          <div className="bg-phantom-card border border-phantom-border rounded-lg p-3 text-sm">
            <div className="text-phantom-muted text-xs">已登入</div>
            <div className="text-phantom-text font-medium mt-0.5">
              {phase.identity.email}
            </div>
            <div className="text-phantom-muted text-xs mt-1">
              provider: {phase.identity.provider}
              {phase.identity.display_name ? ` · ${phase.identity.display_name}` : ""}
            </div>
            <div className="text-phantom-muted text-[11px] mt-1">
              token 有效期至{" "}
              {phase.identity.broker_token_expires_at_ms
                ? new Date(phase.identity.broker_token_expires_at_ms).toLocaleString()
                : "—"}
            </div>
          </div>

          <div className="bg-phantom-card border border-phantom-border rounded-lg p-3 text-sm">
            <div className="text-phantom-muted text-xs flex items-center justify-between">
              <span>已同步</span>
              <button
                onClick={handleResync}
                className="text-phantom-primary text-[11px] underline"
              >
                重新同步
              </button>
            </div>
            {phase.sync ? (
              <div className="mt-2 space-y-1">
                <div className="text-phantom-text text-xs">
                  LLM keys: {phase.sync.keys_written.length} 個
                </div>
                {phase.sync.keys_written.length > 0 && (
                  <div className="text-phantom-muted text-[11px]">
                    {phase.sync.keys_written.join(", ")}
                  </div>
                )}
                <div className="text-phantom-text text-xs">
                  Cluster peers: {phase.sync.peers_count} 個
                </div>
                <div className="text-phantom-muted text-[10px] break-all">
                  {phase.sync.env_path}
                </div>
              </div>
            ) : (
              <div className="text-phantom-muted text-xs mt-1">
                vault 同步尚未完成 — 點「重新同步」拉 LLM keys + cluster peers
              </div>
            )}
          </div>

          {/* Coordinator picker — Android variant A pattern: each peer is
              a phantom serve node; pick one and the WebView reloads to
              <peer.url>/m where the coordinator's mobile UI handles all
              chat (LLM calls run on coordinator, never on this device). */}
          {phase.sync && phase.sync.peers.length > 0 && (
            <div className="bg-phantom-card border border-phantom-border rounded-lg p-3 text-sm">
              <div className="text-phantom-muted text-xs">選協調者（Coordinator）</div>
              <div className="text-phantom-muted text-[11px] mt-0.5 mb-2">
                點擊一台讓這支 iPhone / iPad 變成它的 thin-client。LLM call 都會在那台機跑。
              </div>
              <div className="flex flex-col gap-2">
                {phase.sync.peers.map((p) => (
                  <button
                    key={`${p.name}-${p.url}`}
                    onClick={() => pickCoordinator(p.url)}
                    className="text-left bg-phantom-bg border border-phantom-border rounded-md px-3 py-2 hover:border-phantom-primary active:opacity-80"
                  >
                    <div className="text-phantom-text text-xs font-medium">{p.name}</div>
                    <div className="text-phantom-muted text-[11px] break-all mt-0.5">{p.url}</div>
                    {p.label && (
                      <div className="text-phantom-muted text-[10px] mt-0.5">{p.label}</div>
                    )}
                  </button>
                ))}
              </div>
              <div className="text-phantom-muted text-[10px] mt-2">
                不在列表？到桌面端跑 <code>phantom cluster join</code> 把那台註冊進你的帳號。
              </div>
            </div>
          )}

          <button
            onClick={handleLogout}
            className="w-full bg-phantom-card border border-phantom-border text-phantom-text px-4 py-2.5 rounded-lg active:opacity-80 text-sm"
          >
            登出
          </button>
        </div>
      )}

      {phase.kind === "error" && (
        <div className="space-y-2">
          <div className="text-sm text-red-400">登入失敗：{phase.message}</div>
          <button
            onClick={() => setPhase({ kind: "logged-out" })}
            className="text-xs text-phantom-muted underline"
          >
            重試
          </button>
        </div>
      )}
    </div>
  );
}
