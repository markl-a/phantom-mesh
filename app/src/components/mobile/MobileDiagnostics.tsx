// Diagnostic panel — shows the broker-login + vault-sync chain at a
// glance so the user can pinpoint where things break instead of "the
// app says I'm not logged in but I tapped Login twice".
//
// Each row is one prerequisite of "iOS chat actually calls LLM via
// broker-pulled keys":
//
//   1. Auth state saved        ← broker_login_finish landed
//   2. broker_token not expired ← still usable for /api/me/* calls
//   3. env file exists          ← broker_sync_from_vault wrote it
//   4. process env has keys     ← lib.rs setup() loaded them
//   5. Try Sync Now             ← live re-fetch from phantommesh.io
//
// First row that's ✗ tells you what to fix.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  loadBrokerLoginStatus,
  syncFromVault,
  type BrokerLoginFinishResponse,
  type BrokerSyncResponse,
} from "../../lib/brokerLogin";
import { listProviderKeys, type LocalKeysSnapshot } from "../../lib/localKeys";

interface ChatResult {
  ok: boolean;
  output?: string;
  elapsed?: number;
  error?: string;
}

interface DiagState {
  auth: BrokerLoginFinishResponse | null;
  keys: LocalKeysSnapshot | null;
  loaded: boolean;
}

export default function MobileDiagnostics() {
  const [state, setState] = useState<DiagState>({
    auth: null,
    keys: null,
    loaded: false,
  });
  const [busy, setBusy] = useState(false);
  const [syncResult, setSyncResult] = useState<BrokerSyncResponse | null>(null);
  const [syncError, setSyncError] = useState<string | null>(null);

  const refresh = async () => {
    try {
      const [auth, keys] = await Promise.all([
        loadBrokerLoginStatus().catch(() => null),
        listProviderKeys().catch(() => null),
      ]);
      setState({ auth, keys, loaded: true });
    } catch (e) {
      setState((s) => ({ ...s, loaded: true }));
      console.error("[diag] refresh failed:", e);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const trySync = async () => {
    setBusy(true);
    setSyncError(null);
    setSyncResult(null);
    try {
      const r = await syncFromVault();
      setSyncResult(r);
      await refresh();
    } catch (e) {
      setSyncError(String(e));
    } finally {
      setBusy(false);
    }
  };

  // ── Self-test chat ─────────────────────────────────────────────────────
  const [chatBusy, setChatBusy] = useState(false);
  const [chatResult, setChatResult] = useState<ChatResult | null>(null);
  const trySelfChat = async () => {
    setChatBusy(true);
    setChatResult(null);
    const t0 = Date.now();
    try {
      const r = (await invoke("send_message", {
        prompt: "Reply with just the word 'ok' (no punctuation).",
        agent: "master",
      })) as { output?: string; elapsed?: number };
      setChatResult({
        ok: true,
        output: r.output ?? "(empty output)",
        elapsed: r.elapsed ?? (Date.now() - t0) / 1000,
      });
    } catch (e) {
      setChatResult({ ok: false, error: String(e) });
    } finally {
      setChatBusy(false);
    }
  };

  if (!state.loaded) {
    return <div className="text-sm text-phantom-muted">讀取中 …</div>;
  }

  const now = Date.now();
  const tokenExpired = state.auth
    ? state.auth.broker_token_expires_at_ms <= now
    : true;
  const setKeys = state.keys?.keys.filter((k) => k.set) ?? [];

  // Build the diagnostic chain
  const rows: { label: string; ok: boolean; detail: string }[] = [
    {
      label: "1. Broker login 完成",
      ok: !!state.auth,
      detail: state.auth
        ? `${state.auth.email} · ${state.auth.provider}`
        : "尚未登入。回上一頁點「登入 phantommesh.io」",
    },
    {
      label: "2. broker_token 仍有效",
      ok: !!state.auth && !tokenExpired,
      detail: state.auth
        ? tokenExpired
          ? "已過期，要重新登入"
          : `到期 ${new Date(state.auth.broker_token_expires_at_ms).toLocaleString()}`
        : "—",
    },
    {
      label: "3. env 檔已寫入沙盒",
      ok: setKeys.length > 0,
      detail:
        setKeys.length > 0
          ? `${setKeys.length} 個 key（${setKeys.map((k) => k.name).join(", ")}）`
          : "0 個 key — 點下方「立刻試 sync」或手動填",
    },
    {
      label: "4. process env 已載入",
      ok: setKeys.length > 0,
      detail:
        setKeys.length > 0
          ? "set_var 在 broker_sync_from_vault 即時做 + lib.rs setup() 也補一次"
          : "等到 #3 通了才會綠",
    },
  ];

  return (
    <div className="space-y-3">
      <div className="text-xs text-phantom-muted leading-relaxed">
        每行是 iPhone 上 LLM call 能跑的一個前提。從上往下看，第一個
        紅 ✗ 就是要修的地方。
      </div>

      <div className="bg-phantom-card border border-phantom-border rounded-lg divide-y divide-phantom-border">
        {rows.map((r, i) => (
          <div key={i} className="p-3">
            <div className="flex items-start gap-2">
              <span
                className={`mt-0.5 text-base ${
                  r.ok ? "text-emerald-400" : "text-red-400"
                }`}
              >
                {r.ok ? "✓" : "✗"}
              </span>
              <div className="flex-1 min-w-0">
                <div className="text-phantom-text text-sm font-medium">
                  {r.label}
                </div>
                <div className="text-phantom-muted text-[11px] mt-0.5 break-words">
                  {r.detail}
                </div>
              </div>
            </div>
          </div>
        ))}
      </div>

      <button
        onClick={trySync}
        disabled={busy || !state.auth}
        className="w-full bg-phantom-primary text-phantom-bg font-medium px-4 py-3 rounded-lg active:opacity-80 disabled:opacity-50"
      >
        {busy ? "同步中 …" : "立刻試 sync（從 vault 拉 keys）"}
      </button>

      <button
        onClick={trySelfChat}
        disabled={chatBusy}
        className="w-full bg-phantom-card border border-phantom-primary text-phantom-primary font-medium px-4 py-3 rounded-lg active:opacity-80 disabled:opacity-50"
      >
        {chatBusy ? "LLM 呼叫中 …" : "🧪 自我測試 chat（一鍵打 send_message）"}
      </button>
      {chatResult && chatResult.ok && (
        <div className="bg-emerald-900/30 border border-emerald-500/40 rounded-lg p-3 text-xs text-emerald-300 space-y-1">
          <div>✓ chat OK ({chatResult.elapsed?.toFixed(1)}s)</div>
          <div className="text-phantom-text bg-phantom-bg/40 rounded p-2 mt-1 font-mono break-words whitespace-pre-wrap">
            {chatResult.output}
          </div>
        </div>
      )}
      {chatResult && !chatResult.ok && (
        <div className="bg-red-900/30 border border-red-500/40 rounded-lg p-3 text-xs text-red-300 break-words whitespace-pre-wrap">
          ✗ chat 失敗：{chatResult.error}
        </div>
      )}
      {!state.auth && (
        <div className="text-[11px] text-phantom-muted text-center">
          還沒登入，按鈕暫無作用 — 請先到「登入 phantommesh.io」完成 OAuth
        </div>
      )}

      {syncError && (
        <div className="bg-red-900/30 border border-red-500/40 rounded-lg p-3 text-xs text-red-300">
          ✗ sync 失敗：{syncError}
        </div>
      )}

      {syncResult && (
        <div className="bg-emerald-900/30 border border-emerald-500/40 rounded-lg p-3 text-xs text-emerald-300 space-y-1">
          <div>✓ sync OK</div>
          <div>從 vault 拉到 {syncResult.keys_written.length} 個 key</div>
          {syncResult.keys_written.length === 0 && (
            <div className="text-yellow-300 mt-1">
              vault 是空的 — 去 https://phantommesh.io/account 登入後填 keys，再回來按一次
            </div>
          )}
          {syncResult.keys_written.length > 0 && (
            <div className="text-emerald-200/80">
              {syncResult.keys_written.join(", ")}
            </div>
          )}
          <div>cluster peers: {syncResult.peers_count}</div>
        </div>
      )}

      <div className="text-[10px] text-phantom-muted leading-relaxed mt-4">
        什麼都試了還不行？
        <br />
        a. 直接到「手動填 LLM API key」貼一個 OPENAI_API_KEY 試 chat
        <br />
        b. 點「重新同步」（broker login 那頁）— 通常 OAuth 跑完就立刻同步，
        但有時 deep-link race 會漏掉
      </div>
    </div>
  );
}
