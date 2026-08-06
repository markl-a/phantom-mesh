import { useState, useRef } from "react";
import {
  STARTER_PALETTE, ensureCheckin, describeHabitError, isQuantifiable,
} from "../../lib/captureHabit";
import HabitQtyPicker from "./HabitQtyPicker";

// slug → human label, so aria-live announces "水" not "water" (and "自由記錄"
// for the free-text bucket) for TalkBack.
const LABEL_BY_SLUG: Record<string, string> = {
  ...Object.fromEntries(STARTER_PALETTE.map((c) => [c.slug, c.label])),
  freetext: "自由記錄",
};

// SPEC-22 / SPEC-34 Screen 9 in-app habit quick-capture (記錄 tab). Tapping a
// chip calls ensureCheckin (create-if-missing + record via the live
// record_checkin pipeline → encrypted event). Quantifiable chips (water/coffee/
// …) open a qty quick-pick first; a free-text row covers anything off-palette.
// Success flashes "已記錄" on the chip for ~1.5s and announces via aria-live.

const FEEDBACK_MS = 1500;

type ChipState =
  | { kind: "idle" }
  | { kind: "pending" }
  | { kind: "done"; streak: number }
  | { kind: "error"; message: string };

export default function HabitChipQuickCapture() {
  // Per-slug transient state; absent slug == idle.
  const [states, setStates] = useState<Record<string, ChipState>>({});
  const [qtyFor, setQtyFor] = useState<{ slug: string; label: string } | null>(null);
  const [freeText, setFreeText] = useState("");
  // Synchronous in-flight guard per slug — the React-state "pending" check has a
  // pre-render race window where a double-tap/submit could fire two checkins.
  const inFlight = useRef<Set<string>>(new Set());

  const stateFor = (slug: string): ChipState => states[slug] ?? { kind: "idle" };

  const setSlug = (slug: string, next: ChipState) =>
    setStates((prev) => ({ ...prev, [slug]: next }));

  const commit = async (slug: string, label: string, note: string | null) => {
    if (inFlight.current.has(slug)) return; // synchronous dup guard
    inFlight.current.add(slug);
    setSlug(slug, { kind: "pending" });
    try {
      const streak = await ensureCheckin(slug, label, { note });
      setSlug(slug, { kind: "done", streak: streak.currentStreak });
    } catch (err) {
      setSlug(slug, { kind: "error", message: describeHabitError(err) });
    } finally {
      inFlight.current.delete(slug);
    }
    setTimeout(() => setSlug(slug, { kind: "idle" }), FEEDBACK_MS);
  };

  const onTap = (slug: string, label: string) => {
    if (stateFor(slug).kind === "pending") return;
    // Don't let a second tap swap the qty-sheet target mid-flow.
    if (qtyFor) return;
    // Quantifiable chips ask for a quantity first (SPEC-34 Screen 9 / ≤3-tap).
    if (isQuantifiable(slug)) {
      setQtyFor({ slug, label });
      return;
    }
    void commit(slug, label, null);
  };

  const onFreeTextSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const text = freeText.trim();
    if (!text) return;
    // Block rapid resubmits that would collide on the shared "freetext" state.
    if (stateFor("freetext").kind === "pending") return;
    void commit("freetext", "自由記錄", text);
    setFreeText("");
  };

  return (
    <section>
      <h2 className="text-xs font-semibold text-spectyn-muted uppercase tracking-wide px-1 mb-2">
        快速習慣
      </h2>
      <div className="grid grid-cols-3 gap-2">
        {STARTER_PALETTE.map((chip) => {
          const st = stateFor(chip.slug);
          const pending = st.kind === "pending";
          const done = st.kind === "done";
          const error = st.kind === "error";
          return (
            <button
              key={chip.slug}
              onClick={() => onTap(chip.slug, chip.label)}
              disabled={pending}
              aria-label={`記錄習慣 ${chip.label}`}
              className={`flex flex-col items-center justify-center gap-1 rounded-lg border px-2 py-3 text-center transition-colors ${
                done
                  ? "border-spectyn-success bg-spectyn-success/10"
                  : error
                  ? "border-spectyn-danger bg-spectyn-danger/10"
                  : "border-spectyn-border bg-spectyn-card hover:border-spectyn-primary"
              } ${pending ? "opacity-60" : ""}`}
            >
              <span className="text-xl leading-none" aria-hidden="true">
                {done ? "✓" : chip.emoji}
              </span>
              <span
                className={`text-[11px] leading-tight ${
                  done
                    ? "text-spectyn-success"
                    : error
                    ? "text-spectyn-danger"
                    : "text-spectyn-text"
                }`}
              >
                {done
                  ? `已記錄 · ${st.streak}天`
                  : error
                  ? st.message
                  : pending
                  ? "記錄中…"
                  : chip.label}
              </span>
            </button>
          );
        })}
      </div>

      {/* TalkBack announcement of the latest success(es). */}
      <p className="sr-only" aria-live="polite">
        {Object.entries(states)
          .filter(([, s]) => s.kind === "done")
          .map(([slug, s]) => `${LABEL_BY_SLUG[slug] ?? slug} 已記錄，連續 ${(s as { streak: number }).streak} 天`)
          .join("；")}
      </p>

      {/* Free-text fallback for anything off-palette (SPEC-34 Screen 9). */}
      <form className="mt-3 flex gap-2" onSubmit={onFreeTextSubmit}>
        <input
          value={freeText}
          onChange={(e) => setFreeText(e.target.value)}
          placeholder="自由打字：例 戒菸 87 天"
          aria-label="自由記錄習慣"
          className="flex-1 bg-spectyn-bg border border-spectyn-border rounded px-3 py-2 text-sm text-spectyn-text placeholder-spectyn-muted focus:outline-none focus:border-spectyn-primary"
        />
        <button type="submit" className="px-4 rounded bg-spectyn-primary text-spectyn-bg text-sm">
          送出
        </button>
      </form>

      {qtyFor && (
        <HabitQtyPicker
          slug={qtyFor.slug}
          label={qtyFor.label}
          onPick={(note) => {
            const q = qtyFor;
            setQtyFor(null);
            void commit(q.slug, q.label, note || null);
          }}
          onClose={() => setQtyFor(null)}
        />
      )}
    </section>
  );
}
