// SPEC-34 Screen 9 — quantity quick-pick for quantifiable habit chips. Opens as
// a bottom-sheet; a tap on a preset writes immediately (≤3-tap budget). The
// chosen quantity is returned as a "<n><unit>" note (interim encoding until the
// structured HabitMetadata wire field lands — 🔒 core).
import { useState } from "react";
import { QUANTIFIABLE } from "../../lib/captureHabit";

export default function HabitQtyPicker({
  slug,
  label,
  onPick,
  onClose,
}: {
  slug: string;
  label: string;
  onPick: (note: string) => void;
  onClose: () => void;
}) {
  const meta = QUANTIFIABLE[slug];
  const [custom, setCustom] = useState("");
  if (!meta) return null;
  return (
    <div className="fixed inset-0 z-50 flex items-end bg-black/40" onClick={onClose}>
      <div
        className="w-full bg-phantom-card rounded-t-2xl p-5 space-y-3"
        onClick={(e) => e.stopPropagation()}
      >
        <p className="text-sm font-semibold text-phantom-text">{label} — 數量</p>
        <div className="grid grid-cols-3 gap-2">
          {meta.quick.map((q) => (
            <button
              key={q}
              aria-label={`${q} ${meta.unit}`}
              onClick={() => onPick(`${q}${meta.unit}`)}
              className="py-3 rounded-lg border border-phantom-border bg-phantom-bg text-sm text-phantom-text"
            >
              {q}
              {meta.unit}
            </button>
          ))}
          <input
            value={custom}
            onChange={(e) => setCustom(e.target.value)}
            inputMode="numeric"
            placeholder="自訂"
            className="py-3 rounded-lg border border-phantom-border bg-phantom-bg text-sm px-2 text-phantom-text placeholder-phantom-muted"
          />
        </div>
        <button
          onClick={() => onPick(custom ? `${custom}${meta.unit}` : "")}
          disabled={!custom}
          className="w-full bg-phantom-primary text-phantom-bg py-2.5 rounded-lg text-sm disabled:opacity-50"
        >
          送出自訂
        </button>
      </div>
    </div>
  );
}
