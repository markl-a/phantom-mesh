// SPEC-31 capture-focus — route /capture/focus; commands_used: capture_focus_start (via lib/captureFocus.ts)

import { useMemo, useState } from "react";
import { Loader2, Play, TimerReset } from "lucide-react";
import {
  DEFAULT_DURATION_MS,
  buildSessionRequest,
  describeFocusError,
  startSession,
} from "../lib/captureFocus";
import type { FocusMode } from "../lib/generated/capture_focus/FocusMode";
import { useHaptics } from "../lib/useHaptics";

type Choice = {
  mode: FocusMode;
  label: string;
  caption: string;
};

const choices: Choice[] = [
  { mode: "pomodoro25", label: "25 分鐘", caption: "Pomodoro 25" },
  { mode: "deep_work50", label: "50 分鐘", caption: "Deep work 50" },
  { mode: "custom", label: "自訂", caption: "Custom" },
];

export default function CaptureFocus() {
  const [mode, setMode] = useState<FocusMode>("deep_work50");
  const [customMin, setCustomMin] = useState(30);
  const [note, setNote] = useState("");
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const { impact } = useHaptics();

  const plannedMs = useMemo(
    () =>
      mode === "custom"
        ? Math.max(1, customMin) * 60_000
        : DEFAULT_DURATION_MS[mode],
    [customMin, mode],
  );

  const plannedLabel = `${Math.round(plannedMs / 60_000)} 分鐘 / min`;

  async function start() {
    setStarting(true);
    setError(null);
    setDone(null);

    try {
      const req = buildSessionRequest(mode, {
        plannedDurationMs: plannedMs,
        label: note.trim() || null,
        tag: ["focus"],
      });
      const id = await startSession(req);

      if (typeof id === "string" && id.length > 0) {
        setDone("已開始焦點 session");
        impact("medium");
      } else {
        setError("無法開始 session（後端未就緒）");
      }
    } catch (e) {
      setError(describeFocusError(e));
    } finally {
      setStarting(false);
    }
  }

  return (
    <div
      data-testid="capture-focus"
      className="min-h-screen bg-spectyn-bg text-spectyn-text pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)] px-[env(safe-area-inset-left)] pr-[env(safe-area-inset-right)]"
    >
      <div className="flex min-h-screen flex-col">
        <main className="flex-1 overflow-y-auto px-5 pb-5 pt-6">
          <header className="mb-6">
            <p className="text-base font-medium text-spectyn-muted">
              專注 / Focus
            </p>
            <h1 className="mt-2 text-2xl font-semibold tracking-normal text-spectyn-text">
              開始焦點 session / Start focus
            </h1>
          </header>

          <section className="rounded-lg border border-spectyn-border bg-spectyn-card p-4">
            <div className="mb-4 flex items-center gap-3">
              <div className="flex min-h-[44px] min-w-[44px] items-center justify-center rounded-lg bg-spectyn-primary text-spectyn-bg">
                <TimerReset aria-hidden="true" size={22} />
              </div>
              <div>
                <p className="text-lg font-semibold text-spectyn-text">
                  {plannedLabel}
                </p>
                <p className="text-base text-spectyn-muted">
                  預計專注時長 / Planned duration
                </p>
              </div>
            </div>

            <fieldset>
              <legend className="mb-3 text-base font-medium text-spectyn-text">
                選擇時長 / Duration
              </legend>

              <div className="space-y-3">
                {choices.map((choice) => {
                  const selected = mode === choice.mode;

                  return (
                    <label
                      key={choice.mode}
                      className={`flex min-h-[44px] items-center gap-3 rounded-lg border p-3 transition motion-reduce:transition-none ${
                        selected
                          ? "border-spectyn-primary bg-spectyn-primary/10"
                          : "border-spectyn-border bg-spectyn-bg"
                      }`}
                    >
                      <input
                        type="radio"
                        name="focus-duration"
                        value={choice.mode}
                        checked={selected}
                        onChange={() => {
                          setMode(choice.mode);
                          setError(null);
                          setDone(null);
                        }}
                        aria-label={`${choice.label} / ${choice.caption}`}
                        className="min-h-[20px] min-w-[20px] accent-spectyn-primary"
                      />
                      <span className="flex flex-1 items-center justify-between gap-3">
                        <span>
                          <span className="block text-base font-medium text-spectyn-text">
                            {choice.label}
                          </span>
                          <span className="block text-base text-spectyn-muted">
                            {choice.caption}
                          </span>
                        </span>
                        {choice.mode === "custom" ? (
                          <span className="flex items-center gap-2">
                            <input
                              type="number"
                              min={1}
                              max={240}
                              value={customMin}
                              disabled={mode !== "custom"}
                              onChange={(event) =>
                                setCustomMin(
                                  Math.min(
                                    240,
                                    Math.max(
                                      1,
                                      Number(event.currentTarget.value) || 1,
                                    ),
                                  ),
                                )
                              }
                              aria-label="自訂分鐘 / Custom minutes"
                              className="min-h-[44px] w-20 rounded-lg border border-spectyn-border bg-spectyn-card px-3 text-base text-spectyn-text disabled:opacity-50"
                            />
                            <span className="text-base text-spectyn-muted">
                              min
                            </span>
                          </span>
                        ) : null}
                      </span>
                    </label>
                  );
                })}
              </div>
            </fieldset>
          </section>

          <section className="mt-4 rounded-lg border border-spectyn-border bg-spectyn-card p-4">
            <label className="block text-base font-medium text-spectyn-text">
              備註 / Note
              <textarea
                value={note}
                onChange={(event) => {
                  setNote(event.currentTarget.value);
                  setError(null);
                  setDone(null);
                }}
                aria-label="備註 / Note"
                rows={4}
                className="mt-3 min-h-[96px] w-full resize-y rounded-lg border border-spectyn-border bg-spectyn-bg px-3 py-3 text-base text-spectyn-text placeholder:text-spectyn-muted"
                placeholder="例如：寫作、研究、程式碼審查"
              />
            </label>
          </section>

          {error ? (
            <p role="alert" className="mt-4 text-base text-spectyn-danger">
              {error}
            </p>
          ) : null}

          {done ? (
            <p role="status" className="mt-4 text-base text-spectyn-muted">
              {done}
            </p>
          ) : null}
        </main>

        <footer className="sticky bottom-0 border-t border-spectyn-border bg-spectyn-bg/95 px-5 py-4 backdrop-blur">
          <button
            type="button"
            onClick={start}
            disabled={starting}
            aria-label="開始 / Start"
            className="flex min-h-[48px] w-full items-center justify-center gap-2 rounded-lg bg-spectyn-primary px-4 py-3 text-base font-semibold text-spectyn-bg transition disabled:opacity-60 motion-reduce:transition-none"
          >
            {starting ? (
              <Loader2
                aria-hidden="true"
                size={20}
                className="animate-spin motion-reduce:animate-none"
              />
            ) : (
              <Play aria-hidden="true" size={20} />
            )}
            {starting ? "開始中 / Starting" : "開始 / Start"}
          </button>
        </footer>
      </div>
    </div>
  );
}
