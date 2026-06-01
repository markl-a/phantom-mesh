// SPEC-31 capture-habit — route /capture/habit; commands_used: capture_habit_log (via lib/captureHabit.ts)

import { useState } from "react";
import { Check, Flame, Loader2, PencilLine } from "lucide-react";
import {
  STARTER_PALETTE,
  describeHabitError,
  ensureCheckin,
} from "../lib/captureHabit";
import { useHaptics } from "../lib/useHaptics";

export default function CaptureHabit() {
  const [selected, setSelected] = useState<string | null>(null);
  const [qty, setQty] = useState("");
  const [freeMode, setFreeMode] = useState(false);
  const [freeText, setFreeText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const { impact } = useHaptics();

  const canSubmit = freeMode ? freeText.trim().length > 0 : selected !== null;

  async function submit() {
    if (busy || !canSubmit) return;

    setBusy(true);
    setError(null);
    setDone(null);

    try {
      const slug = freeMode ? "free_text" : selected;
      if (!slug) return;

      const note = freeMode ? freeText.trim() : qty.trim() || null;
      const label = freeMode
        ? "自由記錄"
        : STARTER_PALETTE.find((c) => c.slug === slug)?.label ?? slug;

      const s = await ensureCheckin(slug, label, { note });
      setDone(`已記錄 🔥 連續 ${s?.currentStreak ?? 0} 天`);
      impact("medium");
    } catch (e) {
      setError(describeHabitError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      data-testid="capture-habit"
      className="min-h-screen bg-phantom-bg text-phantom-text pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)] px-[env(safe-area-inset-left)] pr-[env(safe-area-inset-right)]"
    >
      <div className="flex min-h-screen flex-col">
        <main className="flex-1 overflow-y-auto px-4 pb-6 pt-5">
          <header className="mb-5">
            <div className="mb-2 flex items-center gap-2 text-phantom-primary">
              <Flame aria-hidden="true" className="size-5" />
              <span className="text-base font-medium">習慣 / Habit</span>
            </div>
            <h1 className="text-lg font-semibold">記錄習慣 / Log habit</h1>
            <p className="mt-2 text-base text-phantom-muted">
              選一個習慣，或切換成自由記錄。
            </p>
          </header>

          <section aria-label="習慣選項 / Habit options" className="grid grid-cols-4 gap-2">
            {STARTER_PALETTE.map((c) => {
              const active = !freeMode && selected === c.slug;
              return (
                <button
                  key={c.slug}
                  type="button"
                  aria-label={`${c.label} habit｜log ${c.slug}`}
                  aria-pressed={active}
                  onClick={() => {
                    setFreeMode(false);
                    setSelected(c.slug);
                    setError(null);
                    setDone(null);
                  }}
                  className={[
                    "min-h-[44px] rounded-lg border px-2 py-2 text-base transition",
                    "motion-reduce:transition-none",
                    active
                      ? "border-phantom-primary bg-phantom-primary text-phantom-bg"
                      : "border-phantom-border bg-phantom-card text-phantom-text",
                  ].join(" ")}
                >
                  <span aria-hidden="true" className="block text-lg">
                    {c.emoji}
                  </span>
                  <span className="block truncate">{c.label}</span>
                </button>
              );
            })}
          </section>

          <section className="mt-5 space-y-3">
            <label className="block">
              <span className="mb-2 block text-base text-phantom-muted">
                補充數量或備註 / Quantity or note
              </span>
              <input
                value={qty}
                onChange={(e) => setQty(e.target.value)}
                disabled={freeMode}
                aria-label="補充數量或備註 / Quantity or note"
                placeholder="例：20 分鐘、3 組、睡前完成"
                className="min-h-[44px] w-full rounded-lg border border-phantom-border bg-phantom-card px-3 py-2 text-base text-phantom-text placeholder:text-phantom-muted disabled:opacity-50"
              />
            </label>

            <button
              type="button"
              aria-label="自由記錄切換 / Toggle free text mode"
              aria-pressed={freeMode}
              onClick={() => {
                setFreeMode((v) => !v);
                setError(null);
                setDone(null);
              }}
              className="flex min-h-[44px] w-full items-center justify-between rounded-lg border border-phantom-border bg-phantom-card px-3 py-2 text-left text-base transition motion-reduce:transition-none"
            >
              <span className="flex items-center gap-2">
                <PencilLine aria-hidden="true" className="size-5 text-phantom-primary" />
                自由記錄 / Free text
              </span>
              <span className={freeMode ? "text-phantom-primary" : "text-phantom-muted"}>
                {freeMode ? "On" : "Off"}
              </span>
            </button>

            {freeMode && (
              <label className="block">
                <span className="mb-2 block text-base text-phantom-muted">
                  自由記錄內容 / Free text entry
                </span>
                <textarea
                  value={freeText}
                  onChange={(e) => setFreeText(e.target.value)}
                  aria-label="自由記錄內容 / Free text entry"
                  placeholder="今天完成了什麼？"
                  className="min-h-[96px] w-full resize-none rounded-lg border border-phantom-border bg-phantom-card px-3 py-2 text-base text-phantom-text placeholder:text-phantom-muted"
                />
              </label>
            )}

            {error && (
              <p role="alert" className="text-base text-phantom-warning">
                {error}
              </p>
            )}
            {done && (
              <p role="status" className="flex items-center gap-2 text-base text-phantom-success">
                <Check aria-hidden="true" className="size-5" />
                {done}
              </p>
            )}
          </section>
        </main>

        <footer className="sticky bottom-0 border-t border-phantom-border bg-phantom-bg px-4 py-3">
          <button
            type="button"
            aria-label="記錄習慣 / Log habit"
            disabled={busy || !canSubmit}
            onClick={submit}
            className="flex min-h-[48px] w-full items-center justify-center gap-2 rounded-lg bg-phantom-primary px-4 py-3 text-base font-semibold text-phantom-bg transition disabled:opacity-50 motion-reduce:transition-none"
          >
            {busy && <Loader2 aria-hidden="true" className="size-5 animate-spin motion-reduce:animate-none" />}
            {busy ? "記錄中 / Logging" : "記錄習慣 / Log habit"}
          </button>
        </footer>
      </div>
    </div>
  );
}
