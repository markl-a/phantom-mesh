// J1 Step 3 — Starter habit chips (SPEC-34 Screen 1 / SPEC-22 G1 6–12 bound).
// Pick ≥6 chips; seeds the habit palette via the live createHabit pipeline.
import { useState } from "react";
import { STARTER_PALETTE, createHabit, listHabits } from "../../../lib/captureHabit";

export default function PaletteStep({ onNext }: { onNext: () => void }) {
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [seeding, setSeeding] = useState(false);

  const toggle = (slug: string) =>
    setPicked((prev) => {
      const n = new Set(prev);
      if (n.has(slug)) n.delete(slug);
      else if (n.size < 12) n.add(slug);
      return n;
    });

  const seed = async () => {
    setSeeding(true);
    const existing = await listHabits().catch(() => []);
    const have = new Set(existing.map((h) => h.habitSlug));
    for (const chip of STARTER_PALETTE) {
      if (picked.has(chip.slug) && !have.has(chip.slug)) {
        await createHabit({
          slug: chip.slug,
          label: chip.label,
          targetFrequency: { kind: "daily" },
          tags: [],
          createdAt: new Date().toISOString(),
        }).catch(() => {});
      }
    }
    setSeeding(false);
    onNext();
  };

  return (
    <div className="px-6 space-y-4">
      <h2 className="text-lg font-bold text-phantom-text text-center">挑幾個想養成的習慣</h2>
      <p className="text-center text-xs text-phantom-muted" aria-live="polite">
        已選 {picked.size} / 至少 6
      </p>
      <div className="grid grid-cols-3 gap-2">
        {STARTER_PALETTE.map((chip) => {
          const on = picked.has(chip.slug);
          return (
            <button
              key={chip.slug}
              onClick={() => toggle(chip.slug)}
              aria-label={`習慣 ${chip.label}，${on ? "已選" : "未選"}`}
              aria-pressed={on}
              className={`flex flex-col items-center gap-1 rounded-lg border px-2 py-3 ${
                on
                  ? "border-phantom-primary bg-phantom-primary/10"
                  : "border-phantom-border bg-phantom-card"
              }`}
            >
              <span className="text-xl leading-none" aria-hidden="true">
                {on ? "✓" : chip.emoji}
              </span>
              <span className="text-[11px] leading-tight">{chip.label}</span>
            </button>
          );
        })}
      </div>
      <button
        onClick={() => void seed()}
        disabled={picked.size < 6 || seeding}
        className="w-full bg-phantom-primary text-phantom-bg py-2.5 rounded-lg text-sm font-medium disabled:opacity-50 hover:brightness-110 transition"
      >
        {seeding ? "儲存中…" : "就這些"}
      </button>
    </div>
  );
}
