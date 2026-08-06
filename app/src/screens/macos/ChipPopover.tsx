// SPEC-41 §10.3 — S2 chip quick-log popover (Cmd+Shift+H).
//
// Transient popover to log a habit in ≤ 3 taps: pick a chip → (optional qty) →
// Enter. Free-text fallback for non-palette entries. Reuses lib/captureHabit.ts
// over the SPEC-22 capture_habit_wire backend (Stage 2 deferred → graceful
// "尚未實作" note). The native NSPopover/global-shortcut trigger is SPEC-40
// (deferred); interim surface mounted by callers (e.g. /habit route).
// Wireframe: SPEC-41 §10.3.

import { useRef, useState } from "react";
import { Plus, X, CornerDownLeft } from "lucide-react";
import {
  STARTER_PALETTE, ensureCheckin, describeHabitError,
} from "../../lib/captureHabit";

interface Props {
  onLogged?: (slug: string) => void;
  onCancel?: () => void;
}

export default function ChipPopover({ onLogged, onCancel }: Props) {
  const [selected, setSelected] = useState<string | null>(null);
  const [qty, setQty] = useState("");
  const [freeMode, setFreeMode] = useState(false);
  const [freeText, setFreeText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const qtyRef = useRef<HTMLInputElement>(null);

  const pickChip = (slug: string) => {
    setSelected(slug);
    setFreeMode(false);
    setError(null);
    setDone(null);
    // SPEC-41 §10.3: tapping a chip moves focus to the qty input.
    setTimeout(() => qtyRef.current?.focus(), 0);
  };

  const submit = async () => {
    const slug = freeMode ? "free_text" : selected;
    if (!slug) return;
    if (freeMode && !freeText.trim()) return;
    setBusy(true);
    setError(null);
    const note = freeMode ? freeText.trim() : (qty.trim() || null);
    const label = freeMode
      ? "自由記錄"
      : (STARTER_PALETTE.find((c) => c.slug === slug)?.label ?? slug);
    try {
      // ensureCheckin registers the palette chip on first use, then checks in
      // (record_checkin alone errors ChipNotFound on an un-created slug).
      const s = await ensureCheckin(slug, label, { note });
      // s.currentStreak can be undefined if the backend returns a partial shape
      // (e.g. the web fallback's {} for an unwired command) — guard the display.
      setDone(`已記錄 🔥 連續 ${s?.currentStreak ?? 0} 天`);
      onLogged?.(slug);
    } catch (e) {
      // Backend Stage-2 deferred surfaces here; show it honestly rather than
      // pretending the log persisted.
      setError(describeHabitError(e));
    } finally {
      setBusy(false);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") onCancel?.();
    if (e.key === "Enter") void submit();
  };

  return (
    <div
      data-testid="chip-popover"
      onKeyDown={onKeyDown}
      className="w-[340px] max-w-full bg-spectyn-card border border-spectyn-border rounded-xl shadow-xl overflow-hidden text-spectyn-text"
    >
      <header className="px-4 py-2.5 border-b border-spectyn-border">
        <h2 className="text-sm font-semibold">記一個習慣</h2>
      </header>

      {!freeMode && (
        <div className="px-4 py-3 grid grid-cols-4 gap-2">
          {STARTER_PALETTE.map((c) => (
            <button
              key={c.slug}
              aria-label={c.label}
              aria-pressed={selected === c.slug}
              onClick={() => pickChip(c.slug)}
              className={`flex flex-col items-center gap-0.5 py-2 rounded-lg border text-[11px] transition ${
                selected === c.slug
                  ? "bg-spectyn-primary/15 border-spectyn-primary/40 text-spectyn-primary"
                  : "bg-spectyn-bg border-spectyn-border hover:border-spectyn-primary/30"
              }`}
            >
              <span className="text-lg leading-none">{c.emoji}</span>
              {c.label}
            </button>
          ))}
        </div>
      )}

      <div className="px-4 pb-3 space-y-2">
        {freeMode ? (
          <input
            autoFocus
            value={freeText}
            onChange={(e) => setFreeText(e.target.value)}
            placeholder="自由打字記錄…"
            className="w-full bg-spectyn-bg border border-spectyn-border rounded px-3 py-1.5 text-sm placeholder-spectyn-muted focus:outline-none focus:border-spectyn-primary"
          />
        ) : (
          <input
            ref={qtyRef}
            value={qty}
            onChange={(e) => setQty(e.target.value)}
            placeholder={selected ? "數量 / 備註（選填）" : "先選一個 chip"}
            disabled={!selected}
            className="w-full bg-spectyn-bg border border-spectyn-border rounded px-3 py-1.5 text-sm placeholder-spectyn-muted focus:outline-none focus:border-spectyn-primary disabled:opacity-50"
          />
        )}

        <button
          onClick={() => { setFreeMode((v) => !v); setError(null); setDone(null); }}
          className="flex items-center gap-1 text-xs text-spectyn-muted hover:text-spectyn-text transition"
        >
          <Plus size={12} />{freeMode ? "改用 chip" : "自由打字…"}
        </button>

        {error && <p className="text-xs text-spectyn-warning" role="alert">{error}</p>}
        {done && <p className="text-xs text-spectyn-success" role="status">{done}</p>}
      </div>

      <footer className="px-4 py-2.5 border-t border-spectyn-border flex items-center justify-between text-sm">
        <button onClick={() => onCancel?.()} className="text-spectyn-muted hover:text-spectyn-text transition flex items-center gap-1">
          <X size={13} /> Esc 取消
        </button>
        <button
          onClick={() => void submit()}
          disabled={busy || (freeMode ? !freeText.trim() : !selected)}
          className="flex items-center gap-1 px-3 py-1 rounded-lg font-medium bg-spectyn-primary text-spectyn-bg hover:brightness-110 disabled:opacity-40 transition"
        >
          <CornerDownLeft size={13} /> {busy ? "送出中…" : "送出"}
        </button>
      </footer>
    </div>
  );
}
