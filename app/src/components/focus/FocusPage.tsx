// SPEC-21 capture_focus — standalone /focus page (H2.3 redo, task-2026052706).
//
// Implements the SPEC-21 §8.1 nine-state machine on its own surface (NOT the
// Dashboard — that earlier attempt was reverted in e195d3e). Backed by the
// four Tauri commands wrapped in `lib/captureFocus.ts`. `complete_session` is
// fully wired; `start_session` / `record_interruption` / `analyze_session`
// surface `focus.not_yet_wired:` until SPEC-21 Stage 4 lands, so the page
// degrades to a client-side timer instead of dead-ending — the user can still
// run a focus block and see a (locally synthesized) summary on Mac today.

import { useCallback, useEffect, useRef, useState } from "react";
import {
  Mic, MicOff, Play, Square, Pause, Bell, AppWindow, Lock, ShieldCheck,
  CheckCircle2, AlertTriangle, RotateCcw, Loader2, Sparkles, Timer,
} from "lucide-react";
import {
  buildSessionRequest, startSession, recordInterruption, completeSession,
  describeFocusError, listRecent, focusStatus, MODE_LABEL,
  type RecentFocusEvent,
} from "../../lib/captureFocus";
import { FOCUS_PRESETS, presetToPlan, type FocusPreset } from "../../lib/focusPresets";
import { usePermission } from "../permissions/usePermission";
import { PERMISSION_META } from "../../lib/permissions";
import type { FocusMode } from "../../lib/generated/capture_focus/FocusMode";
import type { FocusSessionResult } from "../../lib/generated/capture_focus/FocusSessionResult";
import type { InterruptionKind } from "../../lib/generated/capture_focus/InterruptionKind";

// SPEC-21 §8.1 FSM: Idle → Requesting → Recording → [Chunking|Interrupted]
// → Finalizing → Transcribing → SummaryGen → Done (+ Error sink). Chunking is
// shown as an inline pulse during long Recording; Finalizing/Transcribing/
// SummaryGen are surfaced as the visible phases of the single
// `complete_session` round-trip.
type FsmState =
  | "idle"
  | "requesting"
  | "recording"
  | "interrupted"
  | "finalizing"
  | "transcribing"
  | "summaryGen"
  | "done"
  | "error";

const INTERRUPTIONS: { kind: InterruptionKind; label: string; icon: typeof Bell }[] = [
  { kind: "user_pause", label: "暫停", icon: Pause },
  { kind: "notification", label: "通知打斷", icon: Bell },
  { kind: "app_switch", label: "切換 App", icon: AppWindow },
  { kind: "screen_lock", label: "螢幕鎖定", icon: Lock },
];

function fmtClock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

export default function FocusPage() {
  const [state, setState] = useState<FsmState>("idle");
  const [presetKey, setPresetKey] = useState<FocusPreset["key"]>("p25");
  const [label, setLabel] = useState("");
  const [customMin, setCustomMin] = useState(25);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [localOnly, setLocalOnly] = useState(false);
  const [startedAtMs, setStartedAtMs] = useState(0);
  const [elapsedMs, setElapsedMs] = useState(0);
  const [interruptions, setInterruptions] = useState(0);
  const [result, setResult] = useState<FocusSessionResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [recent, setRecent] = useState<RecentFocusEvent[]>([]);
  // SPEC-33 §11/§15: RECORD_AUDIO is a runtime permission asked at first
  // capture. The page still works as a pure timer if it's declined, so a
  // denial degrades (noAudio) rather than blocking the session.
  const mic = usePermission("microphone");
  const [noAudio, setNoAudio] = useState(false);
  const tickRef = useRef<number | null>(null);

  // The current session's mode/duration derive DIRECTLY from the picked preset
  // (no lagging `mode` state) so start()/stop() always use the value the user
  // sees — a quick preset-tap-then-start can't submit a stale enum.
  const { mode: planMode, plannedMs } = presetToPlan(presetKey, customMin);

  useEffect(() => { setRecent(listRecent()); }, []);

  // Adopt a focus session started in another surface (CLI `spectyn focus` /
  // TUI `/focus`) — they share the disk-backed session. On mount, if one is
  // active and the app is idle, hydrate into recording so the app shows it.
  useEffect(() => {
    let cancelled = false;
    void focusStatus().then((active) => {
      if (cancelled || !active) return;
      setState((s) => (s === "idle" ? "recording" : s));
      setSessionId(active.sessionId);
      setStartedAtMs(active.startedAtMs);
      setInterruptions(active.interruptions);
      setCustomMin(Math.max(1, Math.round(active.plannedDurationMs / 60000)));
      // Adopt as a custom-duration session; the sync effect derives mode from this.
      setPresetKey("custom");
      setElapsedMs(Date.now() - active.startedAtMs);
    }).catch(() => {});
    return () => { cancelled = true; };
  }, []);

  // Drive the elapsed-time ticker only while actively recording.
  useEffect(() => {
    if (state !== "recording") {
      if (tickRef.current) { window.clearInterval(tickRef.current); tickRef.current = null; }
      return;
    }
    tickRef.current = window.setInterval(() => {
      setElapsedMs(Date.now() - startedAtMs);
    }, 250);
    return () => { if (tickRef.current) window.clearInterval(tickRef.current); };
  }, [state, startedAtMs]);

  const reset = useCallback(() => {
    setState("idle");
    setSessionId(null);
    setLocalOnly(false);
    setStartedAtMs(0);
    setElapsedMs(0);
    setInterruptions(0);
    setResult(null);
    setError(null);
    setNoAudio(false);
  }, []);

  const start = useCallback(async () => {
    setState("requesting");
    setError(null);
    // SPEC-33 §15.2: ask for RECORD_AUDIO at first capture. The idle card has
    // already shown the rationale, so this is the OS-dialog step. A denial is
    // non-fatal — we fall back to a no-audio timer (SPEC-33 §11 fallback).
    if (mic.status !== "granted" && mic.status !== "unsupported") {
      const res = await mic.request();
      setNoAudio(res !== "granted");
    } else {
      setNoAudio(mic.status === "unsupported");
    }
    const req = buildSessionRequest(planMode, {
      plannedDurationMs: plannedMs,
      label: label.trim() || null,
    });
    let id = "";
    let local = false;
    try {
      id = await startSession(req);
    } catch (e) {
      // Backend not wired yet (SPEC-21 Stage 4) — fall through to the local
      // timer so the feature stays usable. Any other error is a hard failure.
      if (!String(e).includes("focus.not_yet_wired")) {
        setError(describeFocusError(e));
        setState("error");
        return;
      }
    }
    // startSession may also *resolve* with a non-id (the web/IPC fallback
    // returns `{}` for an unwired command instead of a string, or throws).
    // Anything that isn't a non-empty string means "not really started" →
    // synthesize a local id + run the timer locally. Without this, sessionId
    // is a bogus object and stop() later builds a malformed result.
    if (typeof id !== "string" || id.length === 0) {
      id = (globalThis.crypto?.randomUUID?.() ?? `local-${Date.now()}`);
      local = true;
    }
    setSessionId(id);
    setLocalOnly(local);
    setStartedAtMs(Date.now());
    setElapsedMs(0);
    setInterruptions(0);
    setState("recording");
  }, [planMode, plannedMs, label, mic]);

  const interrupt = useCallback(async (kind: InterruptionKind) => {
    setInterruptions((n) => n + 1);
    if (kind === "user_pause") setState("interrupted");
    if (sessionId && !localOnly) {
      try { await recordInterruption(sessionId, kind); } catch { /* best-effort */ }
    }
  }, [sessionId, localOnly]);

  const resume = useCallback(() => {
    // Keep the original start anchor so the planned budget still counts down.
    setStartedAtMs(Date.now() - elapsedMs);
    setState("recording");
  }, [elapsedMs]);

  const stop = useCallback(async () => {
    if (!sessionId) return;
    const actualMs = elapsedMs;
    setState("finalizing");
    // Visual phases of the single complete_session round-trip. Tracked so we
    // can clear them once the result lands — otherwise a fast complete_session
    // lets a pending timer regress the state from "done" back to "summaryGen",
    // and a mid-finalize unmount fires setState on an unmounted component.
    const phaseTimers = [
      window.setTimeout(() => setState("transcribing"), 400),
      window.setTimeout(() => setState("summaryGen"), 900),
    ];
    const clearPhases = () => phaseTimers.forEach((t) => window.clearTimeout(t));
    try {
      let res: FocusSessionResult;
      if (localOnly) {
        // Synthesize a result locally — backend not wired. Honest about it.
        res = {
          actualDurationMs: BigInt(actualMs),
          interruptions,
          completionPct: Math.min(100, (actualMs / plannedMs) * 100),
          summary: "本地計時完成（後端摘要待 SPEC-21 Stage 4 接上）",
          suggestion: interruptions > 3
            ? "中斷偏多，下次試試開啟勿擾模式"
            : "節奏不錯，保持下去",
        };
        await new Promise((r) => setTimeout(r, 1100));
      } else {
        res = await completeSession(sessionId, { mode: planMode, label: label.trim() || null });
      }
      clearPhases();
      setResult(res);
      setRecent(listRecent());
      setState("done");
    } catch (e) {
      clearPhases();
      setError(describeFocusError(e));
      setState("error");
    }
  }, [sessionId, elapsedMs, localOnly, interruptions, plannedMs, planMode, label]);

  const remainingMs = Math.max(0, plannedMs - elapsedMs);
  const progressPct = Math.min(100, (elapsedMs / plannedMs) * 100);

  return (
    <div className="max-w-2xl mx-auto space-y-6" data-testid="focus-page">
      <header className="flex items-center gap-3">
        <div className="w-10 h-10 rounded-lg bg-spectyn-primary/15 flex items-center justify-center">
          <Mic size={20} className="text-spectyn-primary" />
        </div>
        <div>
          <h1 className="text-xl font-bold text-spectyn-text">專注時段</h1>
          <p className="text-xs text-spectyn-muted">Focus · SPEC-21 capture_focus</p>
        </div>
        <span className="ml-auto text-[10px] uppercase tracking-wider text-spectyn-muted">
          {state}
        </span>
      </header>

      {/* ── Idle: pick a mode and start ───────────────────────────────── */}
      {state === "idle" && (
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-5 space-y-4" data-testid="focus-idle">
          <div className="grid grid-cols-2 gap-2">
            {FOCUS_PRESETS.map((p) => (
              <button
                key={p.key}
                onClick={() => setPresetKey(p.key)}
                aria-pressed={presetKey === p.key}
                aria-label={`${p.label}${p.minutes ? `，${p.minutes} 分鐘 focus session` : ""}`}
                className={`px-3 py-3 rounded-lg text-sm transition border ${
                  presetKey === p.key
                    ? "bg-spectyn-primary/15 border-spectyn-primary/40 text-spectyn-primary"
                    : "bg-spectyn-bg border-spectyn-border text-spectyn-text hover:border-spectyn-primary/30"
                }`}
              >
                {p.label}
              </button>
            ))}
          </div>

          {presetKey === "custom" && (
            <label className="flex items-center gap-2 text-sm text-spectyn-text">
              自訂分鐘
              <input
                type="number"
                min={1}
                max={240}
                value={customMin}
                onChange={(e) => setCustomMin(Number(e.target.value) || 1)}
                className="w-20 bg-spectyn-bg border border-spectyn-border rounded px-2 py-1 text-sm focus:outline-none focus:border-spectyn-primary"
              />
            </label>
          )}

          <input
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            placeholder="這段時間要做什麼？（選填）"
            className="w-full bg-spectyn-bg border border-spectyn-border rounded px-3 py-2 text-sm text-spectyn-text placeholder-spectyn-muted focus:outline-none focus:border-spectyn-primary"
          />

          {/* SPEC-33 §15.2 rationale (shown before the OS dialog). Lets the
              user grant RECORD_AUDIO ahead of time, or learn why it's asked. */}
          {mic.status !== "granted" && mic.status !== "unsupported" && mic.status !== "unknown" && (
            <div className="bg-spectyn-bg border border-spectyn-border rounded-lg p-3 flex gap-2" data-testid="focus-mic-rationale">
              <Mic size={15} className="text-spectyn-primary flex-shrink-0 mt-0.5" />
              <div className="flex-1 min-w-0">
                <p className="text-xs text-spectyn-text leading-relaxed">{PERMISSION_META.microphone.rationaleZh}</p>
                {mic.status === "denied" ? (
                  <p className="text-[11px] text-spectyn-warning mt-1">已拒絕麥克風 — 仍可純計時，或開始時再次授權。</p>
                ) : (
                  <button
                    onClick={() => void mic.request()}
                    disabled={mic.requesting}
                    className="mt-2 inline-flex items-center gap-1.5 text-[11px] text-spectyn-primary hover:underline disabled:opacity-60"
                  >
                    {mic.requesting ? <Loader2 size={12} className="animate-spin" /> : <ShieldCheck size={12} />}
                    允許使用麥克風
                  </button>
                )}
              </div>
            </div>
          )}

          <button
            onClick={() => void start()}
            className="w-full flex items-center justify-center gap-2 bg-spectyn-primary text-spectyn-bg py-2.5 rounded-lg text-sm font-medium hover:brightness-110 transition"
          >
            <Play size={16} />
            開始 {MODE_LABEL[planMode]}（{fmtClock(plannedMs)}）
          </button>
        </div>
      )}

      {/* ── Requesting ────────────────────────────────────────────────── */}
      {state === "requesting" && (
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-8 flex flex-col items-center gap-3">
          <Loader2 size={28} className="text-spectyn-primary animate-spin" />
          <p className="text-sm text-spectyn-muted">建立 session 中…</p>
        </div>
      )}

      {/* ── Recording / Interrupted ───────────────────────────────────── */}
      {(state === "recording" || state === "interrupted") && (
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-6 space-y-5">
          <div className="text-center">
            <p className="text-5xl font-mono font-bold text-spectyn-text tabular-nums">
              {fmtClock(remainingMs)}
            </p>
            <p className="text-xs text-spectyn-muted mt-1 inline-flex items-center gap-1">
              {MODE_LABEL[planMode]}{label.trim() ? ` · ${label.trim()}` : ""}
              {localOnly && " · 本地計時"}
              {noAudio && (
                <span className="inline-flex items-center gap-0.5 text-spectyn-warning">
                  · <MicOff size={11} /> 無錄音
                </span>
              )}
            </p>
          </div>

          <div className="w-full h-1.5 bg-spectyn-bg rounded-full overflow-hidden">
            <div
              className={`h-full rounded-full transition-all duration-300 ${
                state === "interrupted" ? "bg-spectyn-warning" : "bg-spectyn-primary"
              }`}
              style={{ width: `${progressPct}%` }}
            />
          </div>

          <div className="flex items-center justify-center gap-3 text-xs text-spectyn-muted">
            <span>已過 {fmtClock(elapsedMs)}</span>
            <span>·</span>
            <span>中斷 {interruptions} 次</span>
          </div>

          {state === "interrupted" ? (
            <button
              onClick={resume}
              className="w-full flex items-center justify-center gap-2 bg-spectyn-warning text-spectyn-bg py-2.5 rounded-lg text-sm font-medium hover:brightness-110 transition"
            >
              <Play size={16} /> 繼續
            </button>
          ) : (
            <div className="grid grid-cols-4 gap-2">
              {INTERRUPTIONS.map(({ kind, label: l, icon: Icon }) => (
                <button
                  key={kind}
                  onClick={() => void interrupt(kind)}
                  className="flex flex-col items-center gap-1 py-2 rounded-lg bg-spectyn-bg border border-spectyn-border text-spectyn-muted hover:text-spectyn-warning hover:border-spectyn-warning/40 transition text-[10px]"
                >
                  <Icon size={16} />
                  {l}
                </button>
              ))}
            </div>
          )}

          <button
            onClick={() => void stop()}
            className="w-full flex items-center justify-center gap-2 bg-spectyn-danger/15 text-spectyn-danger py-2.5 rounded-lg text-sm font-medium hover:bg-spectyn-danger/25 transition"
          >
            <Square size={15} /> 結束時段
          </button>
        </div>
      )}

      {/* ── Finalizing / Transcribing / SummaryGen ────────────────────── */}
      {(state === "finalizing" || state === "transcribing" || state === "summaryGen") && (
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-8 flex flex-col items-center gap-3">
          <Loader2 size={28} className="text-spectyn-primary animate-spin" />
          <p className="text-sm text-spectyn-text">
            {state === "finalizing" && "收尾中…"}
            {state === "transcribing" && "轉錄中…"}
            {state === "summaryGen" && "產生摘要中…"}
          </p>
          <p className="text-[10px] text-spectyn-muted">請稍候，正在整理這段時段</p>
        </div>
      )}

      {/* ── Done ──────────────────────────────────────────────────────── */}
      {state === "done" && result && (
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-6 space-y-4">
          <div className="flex items-center gap-2 text-spectyn-success">
            <CheckCircle2 size={20} />
            <h2 className="text-base font-semibold">時段完成</h2>
          </div>

          <div className="grid grid-cols-3 gap-3 text-center">
            <div>
              <p className="text-lg font-bold text-spectyn-text tabular-nums">
                {fmtClock(Number(result.actualDurationMs ?? 0))}
              </p>
              <p className="text-[10px] text-spectyn-muted">實際時長</p>
            </div>
            <div>
              <p className="text-lg font-bold text-spectyn-primary">{(result.completionPct ?? 0).toFixed(0)}%</p>
              <p className="text-[10px] text-spectyn-muted">完成度</p>
            </div>
            <div>
              <p className="text-lg font-bold text-spectyn-warning">{result.interruptions ?? 0}</p>
              <p className="text-[10px] text-spectyn-muted">中斷次數</p>
            </div>
          </div>

          {result.summary && (
            <div className="bg-spectyn-bg border border-spectyn-border rounded p-3">
              <p className="text-[10px] uppercase tracking-wider text-spectyn-muted mb-1">摘要</p>
              <p className="text-sm text-spectyn-text">{result.summary}</p>
            </div>
          )}
          {result.suggestion && (
            <div className="bg-spectyn-primary/10 border border-spectyn-primary/30 rounded p-3 flex gap-2">
              <Sparkles size={14} className="text-spectyn-primary flex-shrink-0 mt-0.5" />
              <p className="text-sm text-spectyn-text">{result.suggestion}</p>
            </div>
          )}

          <button
            onClick={reset}
            className="w-full flex items-center justify-center gap-2 bg-spectyn-primary text-spectyn-bg py-2.5 rounded-lg text-sm font-medium hover:brightness-110 transition"
          >
            <RotateCcw size={15} /> 開始新時段
          </button>
        </div>
      )}

      {/* ── Error ─────────────────────────────────────────────────────── */}
      {state === "error" && (
        <div className="bg-spectyn-danger/10 border border-spectyn-danger/40 rounded-lg p-6 space-y-3">
          <div className="flex items-center gap-2 text-spectyn-danger">
            <AlertTriangle size={20} />
            <h2 className="text-base font-semibold">無法完成</h2>
          </div>
          <p className="text-sm text-spectyn-text">{error}</p>
          <button
            onClick={reset}
            className="flex items-center gap-2 bg-spectyn-card border border-spectyn-border text-spectyn-text px-4 py-2 rounded-lg text-sm hover:border-spectyn-primary/40 transition"
          >
            <RotateCcw size={15} /> 重試
          </button>
        </div>
      )}

      {/* ── Recent sessions ───────────────────────────────────────────── */}
      {recent.length > 0 && (state === "idle" || state === "done") && (
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-4">
          <h3 className="text-sm font-medium text-spectyn-text mb-3 flex items-center gap-2">
            <Timer size={14} /> 最近時段
          </h3>
          <div className="space-y-2">
            {recent.slice(0, 5).map((ev) => (
              <div key={ev.sessionId} className="flex items-center gap-3 text-xs">
                <span className="text-spectyn-muted w-16 flex-shrink-0">
                  {MODE_LABEL[ev.mode]}
                </span>
                <span className="flex-1 truncate text-spectyn-text">
                  {ev.label || ev.result.summary || "(無標籤)"}
                </span>
                <span className="text-spectyn-primary tabular-nums">
                  {(ev.result.completionPct ?? 0).toFixed(0)}%
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
