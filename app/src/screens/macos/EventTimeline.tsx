// SPEC-16 event storage — Life Timeline (/timeline).
//
// Unified chronological view of every captured event (food / focus / habit /
// text) across the capture pipelines. Reads plaintext event metadata via the
// real query_events (no decryption — bodies stay encrypted at rest). Filter by
// kind. Design lineage: BIG-GOAL P2 multimodal → SPEC-16 event storage §7.

import { useCallback, useEffect, useState } from "react";
import { History, RefreshCw, Trash2, X } from "lucide-react";
import { queryEvents, buildQuery, describeEventError, captureNote, deleteEvent, showEvent, KIND_META } from "../../lib/eventStore";
import type { EventDetail } from "../../lib/eventStore";
import type { EventRecord } from "../../lib/generated/event_storage/EventRecord";
import type { EventKind } from "../../lib/generated/rpc/EventKind";

const KINDS: EventKind[] = ["food", "focus", "habit", "text"];

function fmtTime(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

export default function EventTimeline() {
  const [filter, setFilter] = useState<EventKind | null>(null);
  const [events, setEvents] = useState<EventRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState("");
  const [noting, setNoting] = useState(false);
  const [confirmId, setConfirmId] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [detail, setDetail] = useState<EventDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);

  const refresh = useCallback(async (kind: EventKind | null) => {
    setLoading(true);
    setError(null);
    try {
      const list = await queryEvents(buildQuery({ kind, limit: 100 }));
      // newest first
      list.sort((a, b) => (b.meta?.timestamp ?? "").localeCompare(a.meta?.timestamp ?? ""));
      setEvents(list);
    } catch (e) {
      setError(describeEventError(e));
      setEvents([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void refresh(filter); }, [filter, refresh]);

  const saveNote = useCallback(async () => {
    const text = note.trim();
    if (!text) return;
    setNoting(true);
    try {
      const id = await captureNote(text);
      if (id) { setNote(""); await refresh(filter); }
      else setError("筆記後端暫時無法使用(需在桌面 app 中執行)");
    } catch (e) {
      setError(String(e ?? "未知錯誤"));
    } finally {
      setNoting(false);
    }
  }, [note, filter, refresh]);

  const doDelete = useCallback(async (id: string) => {
    setDeleting(true);
    setError(null);
    try {
      await deleteEvent(id);
      setConfirmId(null);
      await refresh(filter);
    } catch (e) {
      setError(describeEventError(e));
    } finally {
      setDeleting(false);
    }
  }, [filter, refresh]);

  const openDetail = useCallback(async (id: string) => {
    setDetailLoading(true);
    setDetail({ eventId: id, timestamp: "", kind: "", tags: [], summary: null, suggestion: null, goalImpact: null, confidence: null, modelId: null });
    try {
      const d = await showEvent(id);
      if (d) setDetail(d);
      else { setDetail(null); setError("事件詳情需在桌面 app 中檢視（瀏覽器模式不支援）"); }
    } catch (e) {
      setDetail(null);
      setError(describeEventError(e));
    } finally {
      setDetailLoading(false);
    }
  }, []);

  return (
    <div className="max-w-2xl mx-auto space-y-5" data-testid="event-timeline">
      <header className="flex items-center gap-3">
        <div className="w-10 h-10 rounded-lg bg-spectyn-primary/15 flex items-center justify-center">
          <History size={20} className="text-spectyn-primary" />
        </div>
        <div className="flex-1">
          <h1 className="text-xl font-bold text-spectyn-text">生活時間軸</h1>
          <p className="text-xs text-spectyn-muted">Life timeline · SPEC-16 events</p>
        </div>
        <button onClick={() => void refresh(filter)} className="text-spectyn-muted hover:text-spectyn-text p-1.5" title="重新整理">
          <RefreshCw size={16} className={loading ? "animate-spin" : ""} />
        </button>
      </header>

      <form onSubmit={(e) => { e.preventDefault(); void saveNote(); }} className="flex gap-2">
        <input
          type="text"
          value={note}
          onChange={(e) => setNote(e.target.value)}
          placeholder="快速記一則筆記…(寫成 Life Node 事件)"
          className="flex-1 bg-spectyn-bg border border-spectyn-border rounded-lg px-3 py-2 text-sm text-spectyn-text placeholder:text-spectyn-muted focus:border-spectyn-primary outline-none"
        />
        <button type="submit" disabled={noting || !note.trim()}
          className="px-4 py-2 rounded-lg bg-spectyn-primary/15 border border-spectyn-primary/40 text-spectyn-primary text-sm hover:bg-spectyn-primary/25 disabled:opacity-50">記下</button>
      </form>

      <div className="flex flex-wrap gap-2">
        <button
          onClick={() => setFilter(null)}
          aria-pressed={filter === null}
          className={`px-3 py-1 rounded-full text-xs border transition ${filter === null ? "bg-spectyn-primary/15 border-spectyn-primary/40 text-spectyn-primary" : "bg-spectyn-bg border-spectyn-border text-spectyn-text hover:border-spectyn-primary/30"}`}
        >全部</button>
        {KINDS.map((k) => (
          <button
            key={k}
            onClick={() => setFilter(k)}
            aria-pressed={filter === k}
            className={`px-3 py-1 rounded-full text-xs border transition ${filter === k ? "bg-spectyn-primary/15 border-spectyn-primary/40 text-spectyn-primary" : "bg-spectyn-bg border-spectyn-border text-spectyn-text hover:border-spectyn-primary/30"}`}
          >{KIND_META[k].emoji} {KIND_META[k].label}</button>
        ))}
      </div>

      {error && (
        <div className="bg-spectyn-warning/10 border border-spectyn-warning/40 rounded-lg p-3 text-sm text-spectyn-warning">{error}</div>
      )}

      {!loading && events.length === 0 && !error ? (
        <div className="bg-spectyn-card border border-spectyn-border rounded-lg p-6 text-center">
          <p className="text-sm text-spectyn-text">尚無事件紀錄</p>
          <p className="text-xs text-spectyn-muted mt-1">透過專注 / 習慣 / 飲食頁記錄後，會在這裡按時間排列。</p>
        </div>
      ) : (
        <div className="space-y-1.5">
          {events.map((ev, i) => {
            const meta = KIND_META[ev.meta?.kind as EventKind] ?? { label: ev.meta?.kind ?? "?", emoji: "•" };
            return (
              <div key={ev.meta?.eventId || `ev-${i}`} className="flex items-center gap-3 px-3 py-2 rounded bg-spectyn-card border border-spectyn-border">
                <span className="text-lg w-6 text-center flex-shrink-0">{meta.emoji}</span>
                <button
                  type="button"
                  onClick={() => ev.meta?.eventId && void openDetail(ev.meta.eventId)}
                  disabled={!ev.meta?.eventId}
                  className="flex-1 min-w-0 text-left hover:opacity-80 transition disabled:cursor-default"
                  title="檢視詳情"
                >
                  <span className="text-sm text-spectyn-text">{meta.label}</span>
                  {(ev.meta?.tags ?? []).length > 0 && (
                    <span className="ml-2 text-[10px] text-spectyn-muted">{(ev.meta?.tags ?? []).join(" · ")}</span>
                  )}
                </button>
                <span className="text-[11px] text-spectyn-muted flex-shrink-0">{fmtTime(ev.meta?.timestamp ?? "")}</span>
                {ev.meta?.eventId && (
                  confirmId === ev.meta.eventId ? (
                    <span className="flex items-center gap-1.5 flex-shrink-0">
                      <button onClick={() => void doDelete(ev.meta!.eventId)} disabled={deleting}
                        className="text-[11px] text-spectyn-danger hover:underline disabled:opacity-50">確定刪除</button>
                      <button onClick={() => setConfirmId(null)}
                        className="text-[11px] text-spectyn-muted hover:underline">取消</button>
                    </span>
                  ) : (
                    <button onClick={() => setConfirmId(ev.meta!.eventId)} title="刪除這筆事件" aria-label="刪除"
                      className="text-spectyn-muted hover:text-spectyn-danger flex-shrink-0 p-1 transition">
                      <Trash2 size={13} />
                    </button>
                  )
                )}
              </div>
            );
          })}
        </div>
      )}

      {detail && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4" onClick={() => setDetail(null)}>
          <div
            className="bg-spectyn-card border border-spectyn-border rounded-xl shadow-xl w-full max-w-md max-h-[80vh] overflow-y-auto"
            onClick={(e) => e.stopPropagation()}
            data-testid="event-detail-modal"
          >
            <header className="flex items-center justify-between px-4 py-3 border-b border-spectyn-border">
              <h2 className="text-sm font-semibold text-spectyn-text flex items-center gap-2">
                <span>{KIND_META[detail.kind as EventKind]?.emoji ?? "•"}</span>
                {KIND_META[detail.kind as EventKind]?.label ?? detail.kind ?? "事件"}
              </h2>
              <button onClick={() => setDetail(null)} className="text-spectyn-muted hover:text-spectyn-text p-1" aria-label="關閉"><X size={16} /></button>
            </header>
            <div className="px-4 py-3 space-y-3 text-sm">
              {detailLoading && !detail.timestamp ? (
                <p className="text-xs text-spectyn-muted">載入中…</p>
              ) : (
                <>
                  {detail.timestamp && <p className="text-xs text-spectyn-muted">{fmtTime(detail.timestamp)}</p>}
                  {detail.tags.length > 0 && (
                    <div className="flex flex-wrap gap-1.5">
                      {detail.tags.map((t) => (
                        <span key={t} className="text-[10px] px-2 py-0.5 rounded-full bg-spectyn-bg border border-spectyn-border text-spectyn-muted">{t}</span>
                      ))}
                    </div>
                  )}
                  {detail.summary ? (
                    <div>
                      <p className="text-[11px] font-semibold text-spectyn-muted mb-1">摘要</p>
                      <p className="text-spectyn-text whitespace-pre-wrap break-words">{detail.summary}</p>
                    </div>
                  ) : (
                    <p className="text-xs text-spectyn-muted">這筆事件沒有分析摘要。</p>
                  )}
                  {detail.suggestion && (
                    <div>
                      <p className="text-[11px] font-semibold text-spectyn-muted mb-1">建議</p>
                      <p className="text-spectyn-text whitespace-pre-wrap break-words">{detail.suggestion}</p>
                    </div>
                  )}
                  {detail.goalImpact && (
                    <div>
                      <p className="text-[11px] font-semibold text-spectyn-muted mb-1">目標影響</p>
                      <p className="text-spectyn-text">{detail.goalImpact}</p>
                    </div>
                  )}
                  {detail.modelId && (
                    <p className="text-[10px] text-spectyn-muted/70">
                      {detail.modelId}{detail.confidence != null ? ` · 信心 ${(detail.confidence * 100).toFixed(0)}%` : ""}
                    </p>
                  )}
                  <p className="text-[10px] text-spectyn-muted/60 font-mono break-all pt-1 border-t border-spectyn-border">{detail.eventId}</p>
                </>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
