// Manual API-key entry — fallback for users who can't / don't want to
// use the broker login flow. Lets them paste OPENAI_API_KEY etc directly
// and the chat starts working immediately (writes to ~/.spectyn-mesh/env
// + std::env::set_var inside the same process).

import { useEffect, useState } from "react";
import {
  ALLOWED_KEYS,
  listProviderKeys,
  setProviderKey,
  setProviderKeysBulk,
  parseEnvBlock,
  type KeyStatus,
} from "../../lib/localKeys";

export default function MobileLocalKeys() {
  const [snapshot, setSnapshot] = useState<KeyStatus[] | null>(null);
  const [envPath, setEnvPath] = useState<string>("");
  const [editing, setEditing] = useState<Record<string, string>>({});
  const [bulkText, setBulkText] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [okMessage, setOk] = useState<string | null>(null);

  const refresh = async () => {
    try {
      const snap = await listProviderKeys();
      setSnapshot(snap.keys);
      setEnvPath(snap.env_path);
    } catch (e) {
      setError(String(e));
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const saveOne = async (name: string) => {
    const value = editing[name] ?? "";
    setBusy(name);
    setError(null);
    setOk(null);
    try {
      const snap = await setProviderKey(name, value);
      setSnapshot(snap.keys);
      setEditing((s) => ({ ...s, [name]: "" }));
      setOk(value.trim() ? `${name} 已存` : `${name} 已清除`);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const saveBulk = async () => {
    const entries = parseEnvBlock(bulkText);
    if (Object.keys(entries).length === 0) {
      setError("沒解到任何 KEY=VALUE 行");
      return;
    }
    setBusy("__bulk__");
    setError(null);
    setOk(null);
    try {
      const snap = await setProviderKeysBulk(entries);
      setSnapshot(snap.keys);
      setBulkText("");
      const applied = Object.keys(entries).filter((k) =>
        (ALLOWED_KEYS as readonly string[]).includes(k),
      ).length;
      setOk(`貼了 ${applied} 個 key`);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="space-y-3">
      <div className="text-xs text-spectyn-muted leading-relaxed">
        直接貼 LLM API key 進這台 iPhone 的沙盒 — 不需要 phantommesh.io
        登入。Key 馬上載進 process env，下一句 chat 就會用到。
      </div>

      {error && (
        <div className="bg-red-900/30 border border-red-500/40 rounded-lg p-2 text-xs text-red-300">
          {error}
        </div>
      )}
      {okMessage && (
        <div className="bg-emerald-900/30 border border-emerald-500/40 rounded-lg p-2 text-xs text-emerald-300">
          ✓ {okMessage}
        </div>
      )}

      <details>
        <summary className="cursor-pointer text-sm text-spectyn-text">
          一次貼一塊（多行 KEY=VALUE）
        </summary>
        <div className="mt-2 space-y-2">
          <textarea
            value={bulkText}
            onChange={(e) => setBulkText(e.target.value)}
            rows={6}
            placeholder={"OPENAI_API_KEY=sk-...\nGROQ_API_KEY=gsk_...\n# unknown keys are silently skipped"}
            className="w-full bg-spectyn-bg border border-spectyn-border rounded-md px-3 py-2 text-xs text-spectyn-text font-mono"
          />
          <button
            onClick={saveBulk}
            disabled={busy === "__bulk__" || !bulkText.trim()}
            className="w-full bg-spectyn-primary text-spectyn-bg font-medium px-4 py-2.5 rounded-lg active:opacity-80 text-sm disabled:opacity-50"
          >
            {busy === "__bulk__" ? "儲存中…" : "解析 + 存"}
          </button>
        </div>
      </details>

      <div className="text-xs text-spectyn-muted">逐個 key</div>

      {snapshot === null ? (
        <div className="text-sm text-spectyn-muted">讀取中 …</div>
      ) : (
        <div className="space-y-2">
          {snapshot.map((k) => (
            <div
              key={k.name}
              className="bg-spectyn-card border border-spectyn-border rounded-md p-2"
            >
              <div className="flex items-center justify-between">
                <span className="text-spectyn-text text-xs font-medium">
                  {k.name}
                </span>
                <span
                  className={`text-[10px] ${
                    k.set ? "text-emerald-400" : "text-spectyn-muted"
                  }`}
                >
                  {k.set ? `已設 (${k.preview ?? "set"})` : "未設"}
                </span>
              </div>
              <div className="flex gap-2 mt-1.5">
                <input
                  type="password"
                  value={editing[k.name] ?? ""}
                  onChange={(e) =>
                    setEditing((s) => ({ ...s, [k.name]: e.target.value }))
                  }
                  placeholder={k.set ? "貼新值取代…（清空則刪除）" : "貼 key 進來…"}
                  className="flex-1 bg-spectyn-bg border border-spectyn-border rounded px-2 py-1 text-[11px] text-spectyn-text font-mono"
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                />
                <button
                  onClick={() => saveOne(k.name)}
                  disabled={busy === k.name}
                  className="bg-spectyn-primary text-spectyn-bg px-3 rounded text-xs disabled:opacity-50"
                >
                  {busy === k.name ? "…" : "存"}
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      <div className="text-[10px] text-spectyn-muted break-all">
        檔案位置：{envPath || "(未知)"}
      </div>
    </div>
  );
}
