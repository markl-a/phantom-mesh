// SPEC-22 habit dashboard — the /habit page: streak cards + quick-log popover.
//
// Unlike the other capture screens, the habit backend is fully Stage-3 wired
// (capture_habit_wire persists to ~/.spectyn-mesh/habits.sqlite), so habit_list
// returns real HabitSummary rows. This page shows the logged palette with
// current streak + 7d/30d counts, and opens the ChipPopover (§10.3) to log.
// Design lineage: BIG-GOAL P2 → SPEC-22 §7.1.5 (dashboard habit cards).

import { useCallback, useEffect, useState } from "react";
import { ListChecks, Plus, Flame, RefreshCw } from "lucide-react";
import ChipPopover from "./ChipPopover";
import { listHabits, describeHabitError, STARTER_PALETTE } from "../../lib/captureHabit";
import type { HabitSummary } from "../../lib/generated/capture_habit/HabitSummary";

const LABEL = new Map(STARTER_PALETTE.map((c) => [c.slug, c]));

export default function HabitPage() {
  const [summaries, setSummaries] = useState<HabitSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [logging, setLogging] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const list = await listHabits();
      setSummaries(Array.isArray(list) ? list : []);
    } catch (e) {
      setError(describeHabitError(e));
      setSummaries([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  return (
    <div className="max-w-2xl mx-auto space-y-5" data-testid="habit-page">
      <header className="flex items-center gap-3">
        <div className="w-10 h-10 rounded-lg bg-spectyn-primary/15 flex items-center justify-center">
          <ListChecks size={20} className="text-spectyn-primary" />
        </div>
        <div className="flex-1">
          <h1 className="text-xl font-bold text-spectyn-text">習慣</h1>
          <p className="text-xs text-spectyn-muted">Habits · SPEC-22 capture-habit</p>
        </div>
        <button
          onClick={() => void refresh()}
          className="text-spectyn-muted hover:text-spectyn-text p-1.5"
          title="重新整理"
        >
          <RefreshCw size={16} className={loading ? "animate-spin" : ""} />
        </button>
        <button
          onClick={() => setLogging(true)}
          className="flex items-center gap-1.5 bg-spectyn-primary text-spectyn-bg px-3 py-1.5 rounded-lg text-sm font-medium hover:brightness-110 transition"
        >
          <Plus size={15} /> 記錄
        </button>
      </header>

      {error && (
        <div className="bg-spectyn-danger/10 border border-spectyn-danger/40 rounded-lg p-3 text-sm text-spectyn-danger">
          {error}
        </div>
      )}

      {!loading && summaries.length === 0 && !error ? (
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-6 text-center">
          <p className="text-sm text-spectyn-text">還沒有習慣紀錄</p>
          <p className="text-xs text-spectyn-muted mt-1">點「記錄」開始打卡，連續天數會在這裡顯示。</p>
        </div>
      ) : (
        <div className="grid grid-cols-2 gap-3">
          {summaries.map((s) => {
            const chip = LABEL.get(s.habitSlug);
            return (
              <div key={s.habitSlug} className="bg-spectyn-card border border-spectyn-border rounded-lg p-4">
                <div className="flex items-center gap-2 mb-2">
                  <span className="text-lg">{chip?.emoji ?? "•"}</span>
                  <span className="text-sm font-medium text-spectyn-text">{chip?.label ?? s.habitSlug}</span>
                  {(s.streak?.currentStreak ?? 0) > 0 && (
                    <span className="ml-auto flex items-center gap-0.5 text-spectyn-warning text-sm">
                      <Flame size={13} />{s.streak?.currentStreak}
                    </span>
                  )}
                </div>
                <div className="flex items-center gap-4 text-[11px] text-spectyn-muted">
                  <span>7 天 {s.last7dCount}</span>
                  <span>30 天 {s.last30dCount}</span>
                  {(s.streak?.longestStreak ?? 0) > 0 && <span>最長 {s.streak?.longestStreak}</span>}
                </div>
              </div>
            );
          })}
        </div>
      )}

      {logging && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
          <ChipPopover
            onLogged={() => { setLogging(false); void refresh(); }}
            onCancel={() => setLogging(false)}
          />
        </div>
      )}
    </div>
  );
}
