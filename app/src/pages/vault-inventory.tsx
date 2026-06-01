// SPEC-31 vault-inventory — route /vault; commands_used: none (backend unwired — honest empty)
//
// Reality check (verified 2026-05-29): there is NO read-only "list vault
// entries" Tauri command exposed to the frontend. The only registered
// vault-touching command is `broker_sync_from_vault` (a *write/sync* action
// that pulls sealed items + writes ~/.phantom-mesh/env — not a read-only
// inventory). The broker's `GET /vault/get` list-mode returns
// `{ items: [{ service, key, ts_ms, byte_len }, ...] }`, but that is reached
// only from Rust inside the sync command; it is not surfaced as a list to the
// UI, and `safeInvoke` cannot reach it without inventing a command. The
// generated shapes under app/src/lib/generated/broker_vault/ describe single
// sealed items (VaultGetResponse: { service, key, valueSealed, tsMs,
// ageRecipientHint }) and the set/wipe request bodies — none is a list query.
//
// Per NO-FAKING: we do NOT invent a command or fabricate vault rows. We render
// honest loading → locked / empty / error states, probe login status only to
// distinguish "vault locked (not logged in)" from "no entries", and disable
// every write action with a 尚未實作 note.

import { useEffect, useRef, useState } from "react";
import { Loader2, Lock, ShieldOff, KeyRound, RefreshCw } from "lucide-react";
import { safeInvoke } from "../lib/tauri-compat";
import { useHaptics } from "../lib/useHaptics";

// ── Honest state machine ──────────────────────────────────────────────
// loading  — probing login/identity status
// locked   — no identity key / not logged in → vault is sealed, can't list
// empty    — reachable but the read-only list command does not exist yet
//            (backend unwired): we have nothing real to show, so show nothing
// error    — the probe call itself threw/rejected (distinct from empty)
type LoadState = "loading" | "locked" | "empty" | "error";

// `broker_login_status` returns the finish response when logged in, or
// null/undefined when not. We only read its truthiness — no field access —
// so we don't couple to an exact shape we might mis-name.
const LOGIN_STATUS_CMD = "broker_login_status";

export default function VaultInventory() {
  const [state, setState] = useState<LoadState>("loading");
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const { impact } = useHaptics();

  // ── async safety: seq counter + alive ref so only the latest live request
  //    commits; no setState after unmount, no stale overwrite. ──
  const seqRef = useRef(0);
  const aliveRef = useRef(true);

  async function load() {
    const seq = ++seqRef.current;
    // clear prior result at load start
    setState("loading");
    setErrorMsg(null);

    try {
      // Probe whether we're logged in (= vault key reachable). A null/undefined
      // result is an honest "not logged in" → locked; it is NOT an error.
      const status = await safeInvoke<unknown>(LOGIN_STATUS_CMD);

      if (!aliveRef.current || seq !== seqRef.current) return; // stale / unmounted

      if (status == null) {
        setState("locked");
        return;
      }

      // Logged in / reachable, but there is no read-only list command wired.
      // Honest empty — we have no real entries to render and refuse to fake any.
      setState("empty");
    } catch (e) {
      if (!aliveRef.current || seq !== seqRef.current) return;
      // A thrown/rejected call is a genuine error state, distinct from empty.
      setErrorMsg(e instanceof Error ? e.message : String(e));
      setState("error");
    }
  }

  useEffect(() => {
    aliveRef.current = true;
    void load();
    return () => {
      aliveRef.current = false;
      seqRef.current++; // invalidate any in-flight request
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function handleRetry() {
    impact("medium");
    void load();
  }

  return (
    <div
      data-testid="vault-inventory"
      className="min-h-screen flex flex-col bg-phantom-bg text-phantom-fg pt-[env(safe-area-inset-top)] pl-[env(safe-area-inset-left)] pr-[env(safe-area-inset-right)]"
    >
      {/* ── Header ── */}
      <header className="px-4 pt-3 pb-2">
        <h1 className="text-lg font-semibold flex items-center gap-2">
          <KeyRound className="w-5 h-5 text-phantom-accent" aria-hidden="true" />
          <span>保險庫 / Vault</span>
        </h1>
        <p className="text-base text-phantom-muted mt-1">
          已加密項目清單（唯讀）/ Encrypted entries (read-only)
        </p>
      </header>

      {/* ── Body: honest states only ── */}
      <main className="flex-1 px-4 py-3">
        {state === "loading" && (
          <div
            role="status"
            aria-label="載入中 / Loading vault"
            className="flex flex-col items-center justify-center gap-3 py-16 text-phantom-muted"
          >
            <Loader2
              className="w-8 h-8 animate-spin motion-reduce:animate-none"
              aria-hidden="true"
            />
            <p className="text-base">讀取保險庫狀態… / Checking vault…</p>
          </div>
        )}

        {state === "locked" && (
          <section
            role="status"
            aria-label="保險庫已鎖定 / Vault locked"
            className="flex flex-col items-center justify-center gap-3 py-16 text-center"
          >
            <Lock className="w-10 h-10 text-phantom-muted" aria-hidden="true" />
            <h2 className="text-lg font-medium">保險庫已鎖定 / Vault locked</h2>
            <p className="text-base text-phantom-muted max-w-xs">
              尚未登入或找不到身分金鑰，無法列出已加密項目。請先完成登入。
              <br />
              No identity key / not logged in — entries can&apos;t be listed.
              Sign in first.
            </p>
          </section>
        )}

        {state === "empty" && (
          <section
            role="status"
            aria-label="目前沒有可顯示的項目 / No entries to show"
            className="flex flex-col items-center justify-center gap-3 py-16 text-center"
          >
            <ShieldOff
              className="w-10 h-10 text-phantom-muted"
              aria-hidden="true"
            />
            <h2 className="text-lg font-medium">尚無項目可顯示 / Nothing to show</h2>
            <p className="text-base text-phantom-muted max-w-xs">
              唯讀清單功能尚未實作，因此沒有真實資料可呈現（不會偽造任何項目）。
              <br />
              The read-only list command isn&apos;t wired yet, so there is no
              real data to display (we won&apos;t fabricate entries).
            </p>
          </section>
        )}

        {state === "error" && (
          <section
            role="alert"
            aria-label="讀取保險庫失敗 / Failed to read vault"
            className="flex flex-col items-center justify-center gap-3 py-16 text-center"
          >
            <ShieldOff
              className="w-10 h-10 text-phantom-danger"
              aria-hidden="true"
            />
            <h2 className="text-lg font-medium text-phantom-danger">
              讀取失敗 / Read failed
            </h2>
            {errorMsg && (
              <p className="text-base text-phantom-muted max-w-xs break-words">
                {errorMsg}
              </p>
            )}
          </section>
        )}
      </main>

      {/* ── Sticky footer: reachability. Primary CTA = retry the read.
           No destructive actions in v0.6.0 — add/edit/wipe are disabled
           with a 尚未實作 note rather than faked. ── */}
      <footer className="sticky bottom-0 bg-phantom-bg/95 backdrop-blur border-t border-phantom-border px-4 pt-3 pb-[max(0.75rem,env(safe-area-inset-bottom))]">
        <button
          type="button"
          onClick={handleRetry}
          disabled={state === "loading"}
          aria-label="重新讀取保險庫 / Reload vault"
          className="w-full min-h-[48px] rounded-xl bg-phantom-accent text-phantom-bg font-medium flex items-center justify-center gap-2 transition-colors motion-reduce:transition-none disabled:opacity-50"
        >
          <RefreshCw
            className={
              state === "loading"
                ? "w-5 h-5 animate-spin motion-reduce:animate-none"
                : "w-5 h-5"
            }
            aria-hidden="true"
          />
          <span>重新讀取 / Reload</span>
        </button>

        <button
          type="button"
          disabled
          aria-label="新增項目（尚未實作）/ Add entry (not yet implemented)"
          aria-disabled="true"
          className="mt-2 w-full min-h-[44px] rounded-xl border border-phantom-border text-phantom-muted text-base flex items-center justify-center gap-2 opacity-60 transition-none"
        >
          <span>新增項目（尚未實作）/ Add entry (not yet implemented)</span>
        </button>
      </footer>
    </div>
  );
}
