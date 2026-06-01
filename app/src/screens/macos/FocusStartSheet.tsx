// SPEC-41 §10.4 — S3 focus start sheet (macOS screen catalog, screen #7).
//
// The compact "start a focus session" surface triggered by Cmd+Shift+F. Per
// SPEC-41 this is a transient sheet over the current app; the native trigger
// (NSStatusItem global shortcut) lives in SPEC-40 menubar.rs (deferred), so for
// now this React screen is mounted via the /focus/start route. It reuses the
// already-wired SPEC-21 capture_focus_wire backend through lib/captureFocus.ts
// (no new backend needed). Wireframe: SPEC-41 §10.4; copy aligned to SPEC-21 §10.

import { useState } from "react";
import { Mic, Play, X } from "lucide-react";
import {
  buildSessionRequest, startSession, describeFocusError, DEFAULT_DURATION_MS,
} from "../../lib/captureFocus";
import type { FocusMode } from "../../lib/generated/capture_focus/FocusMode";

interface Props {
  /** Called with the new session id once focus_start_session succeeds. */
  onStart?: (sessionId: string) => void;
  /** Called when the user dismisses the sheet (取消 / Esc). */
  onCancel?: () => void;
  /** Mic permission state (SPEC-41 §10.4 edge case). When false, on-device ASR
   *  is disabled with a "需開麥克風權限" hint. Defaults to granted. */
  micGranted?: boolean;
}

// SPEC-41 §10.4 offers 25 / 50 / custom. Maps to SPEC-21 FocusMode.
const PRESETS: { mode: FocusMode; label: string }[] = [
  { mode: "pomodoro25", label: "25 min（Pomodoro 標準）" },
  { mode: "deep_work50", label: "50 min（Pomodoro 長）" },
];

export default function FocusStartSheet({ onStart, onCancel, micGranted = true }: Props) {
  const [mode, setMode] = useState<FocusMode>("deep_work50"); // wireframe default ●
  const [customMin, setCustomMin] = useState(30);
  const [syncRecording, setSyncRecording] = useState(micGranted);
  const [cloudAsr, setCloudAsr] = useState(false);
  const [note, setNote] = useState("");
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const plannedMs = mode === "custom"
    ? Math.max(1, customMin) * 60 * 1000
    : DEFAULT_DURATION_MS[mode];

  const start = async () => {
    setStarting(true);
    setError(null);
    const tag = ["focus"];
    if (syncRecording && micGranted) tag.push("rec:on-device");
    if (cloudAsr) tag.push("rec:cloud-asr");
    const req = buildSessionRequest(mode, {
      plannedDurationMs: plannedMs,
      label: note.trim() || null,
      tag,
    });
    try {
      const id = await startSession(req);
      // startSession can resolve with a non-id ({} from the web fallback for an
      // unwired command) instead of throwing — don't forward that as a session.
      if (typeof id === "string" && id.length > 0) {
        onStart?.(id);
      } else {
        setError("無法開始 session（後端未就緒）");
      }
    } catch (e) {
      setError(describeFocusError(e));
    } finally {
      setStarting(false);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") onCancel?.();
  };

  return (
    <div
      data-testid="focus-start-sheet"
      onKeyDown={onKeyDown}
      className="w-[480px] max-w-full bg-phantom-card border border-phantom-border rounded-xl shadow-xl overflow-hidden text-phantom-text"
    >
      <header className="px-5 py-3 border-b border-phantom-border">
        <h2 className="text-sm font-semibold">開始焦點 session</h2>
      </header>

      <div className="px-5 py-4 space-y-4">
        {/* 選擇時長 */}
        <fieldset>
          <legend className="text-xs text-phantom-muted mb-2">選擇時長：</legend>
          <div className="space-y-1.5">
            {PRESETS.map((p) => (
              <label key={p.mode} className="flex items-center gap-2 text-sm cursor-pointer">
                <input
                  type="radio" name="focus-mode" checked={mode === p.mode}
                  onChange={() => setMode(p.mode)}
                  className="accent-phantom-primary"
                />
                {p.label}
              </label>
            ))}
            <label className="flex items-center gap-2 text-sm cursor-pointer">
              <input
                type="radio" name="focus-mode" checked={mode === "custom"}
                onChange={() => setMode("custom")}
                className="accent-phantom-primary"
              />
              自訂：
              <input
                type="number" min={1} max={240} value={customMin}
                onChange={(e) => { setMode("custom"); setCustomMin(Number(e.target.value) || 1); }}
                className="w-16 bg-phantom-bg border border-phantom-border rounded px-2 py-0.5 text-sm focus:outline-none focus:border-phantom-primary"
              />
              min
            </label>
          </div>
        </fieldset>

        {/* 錄音 */}
        <fieldset>
          <legend className="text-xs text-phantom-muted mb-2">錄音：</legend>
          <div className="space-y-1.5">
            <label className={`flex items-center gap-2 text-sm ${micGranted ? "cursor-pointer" : "opacity-50"}`}>
              <input
                type="checkbox" checked={syncRecording && micGranted} disabled={!micGranted}
                onChange={(e) => setSyncRecording(e.target.checked)}
                className="accent-phantom-primary"
              />
              同步錄音（on-device ASR）
            </label>
            {!micGranted && (
              <p className="text-[11px] text-phantom-warning pl-6">需開麥克風權限（系統設定 → 隱私權）</p>
            )}
            <label className="flex items-center gap-2 text-sm cursor-pointer">
              <input
                type="checkbox" checked={cloudAsr}
                onChange={(e) => setCloudAsr(e.target.checked)}
                className="accent-phantom-primary"
              />
              Cloud ASR fallback（明確 opt-in）
            </label>
          </div>
        </fieldset>

        {/* 備註 */}
        <div>
          <label className="text-xs text-phantom-muted block mb-1.5">備註（選填）：</label>
          <input
            value={note}
            onChange={(e) => setNote(e.target.value)}
            placeholder="這段時間要做什麼？"
            className="w-full bg-phantom-bg border border-phantom-border rounded px-3 py-1.5 text-sm placeholder-phantom-muted focus:outline-none focus:border-phantom-primary"
          />
        </div>

        {error && (
          <p className="text-xs text-phantom-danger" role="alert">{error}</p>
        )}
      </div>

      <footer className="px-5 py-3 border-t border-phantom-border flex items-center justify-end gap-2">
        <button
          onClick={() => onCancel?.()}
          className="px-4 py-1.5 rounded-lg text-sm text-phantom-muted hover:bg-phantom-bg transition"
        >
          <X size={14} className="inline -mt-0.5 mr-1" />取消
        </button>
        <button
          onClick={() => void start()}
          disabled={starting}
          className="px-4 py-1.5 rounded-lg text-sm font-medium bg-phantom-primary text-phantom-bg hover:brightness-110 disabled:opacity-40 transition"
        >
          <Play size={14} className="inline -mt-0.5 mr-1" />
          {starting ? "開始中…" : "開始"}
        </button>
      </footer>
    </div>
  );
}
