// SPEC-31 skill-bank — route /skills; commands_used: none (backend unwired — honest empty)
//
// Mobile skill bank (技能銀行): a list of Hermes-extracted reusable user-behaviour
// skills. The ONLY real backend is the HTTP route GET /api/hermes/skills in
// core/src/serve_hermes.rs, which (a) is gated behind the `experimental-hermes-
// memory` cargo feature, (b) requires an `X-Cluster-Auth` HMAC header, and
// (c) has NO Tauri `#[command]` wrapper and NO case in tauri-compat's
// httpFallback. Calling safeInvoke("...skill...") today therefore hits the
// httpFallback `default` branch and returns `{}` — a fabricated empty object,
// NOT a real result. Per the SPEC-31 "NO FAKING" rule we must not pretend that
// is data: there is no confirmed callable command, so this screen renders an
// honest empty/disabled state with a 尚未實作 (not yet implemented) note.
//
// The row shape is modelled on the REAL server DTO `SkillSummary` returned by
// `/api/hermes/skills` (core/src/hermes/dto.rs: id/name/description/source/
// curator_score/created_at/tags/polarity). The generated app/src/lib/generated/
// skill/Skill.ts richer shape (triggerPattern/version/qualityScore/lastAppliedAt/
// sourceEventCount) is the on-device extracted form; neither is exposed through a
// command yet, so we display only fields the real list endpoint would provide and
// omit invoke-count / last-used rather than fabricate them. The moment a real
// command lands, swap LIST_COMMAND to its name and the screen lights up.

import { useEffect, useRef, useState } from "react";
import { Library, RefreshCw, ChevronRight, AlertTriangle, Sparkles } from "lucide-react";
import { safeInvoke } from "../lib/tauri-compat";
import { useHaptics } from "../lib/useHaptics";

// ─── confirmed-command toggle ────────────────────────────────────────────────
// No Tauri command for the skill bank exists in app/src-tauri/src, and the
// /api/hermes/skills HTTP route has no httpFallback case (it would need HMAC
// auth + the experimental feature flag). Until one is wired, BACKEND_WIRED stays
// false: we never invoke (which would fabricate `{}`), and the screen shows the
// honest unwired state. When a command lands, set LIST_COMMAND to its real name
// and BACKEND_WIRED to true.
const BACKEND_WIRED = false;
const LIST_COMMAND = ""; // e.g. "skill_list" / "hermes_skills_list" once it exists

// Mirrors core/src/hermes/dto.rs::SkillSummary — the real list-endpoint row.
interface SkillRow {
  id: number;
  name: string;
  description: string;
  source: string;
  polarity: string;
  curatorScore: number | null;
  createdAt: number; // unix seconds
}

type LoadState = "loading" | "ready" | "empty" | "error" | "unwired";

interface SkillBankProps {
  /** Tapping a skill row. Router wiring (navigation) is out of scope. */
  onOpenSkill?: (id: string) => void;
}

/** Normalise one raw row from the server DTO (snake_case) into SkillRow. */
function toRow(raw: Record<string, unknown>): SkillRow | null {
  if (raw == null || typeof raw !== "object") return null;
  const id = typeof raw.id === "number" ? raw.id : Number(raw.id);
  if (!Number.isFinite(id)) return null;
  const name = typeof raw.name === "string" ? raw.name : "";
  if (!name) return null;
  const cs = raw.curator_score ?? (raw as Record<string, unknown>).curatorScore;
  return {
    id,
    name,
    description: typeof raw.description === "string" ? raw.description : "",
    source: typeof raw.source === "string" ? raw.source : "",
    polarity: typeof raw.polarity === "string" ? raw.polarity : "",
    curatorScore: typeof cs === "number" ? cs : null,
    createdAt:
      typeof raw.created_at === "number"
        ? raw.created_at
        : Number((raw as Record<string, unknown>).createdAt) || 0,
  };
}

/** Short bilingual relative time from a unix-seconds timestamp. */
function relTime(unixSecs: number): string {
  if (!unixSecs) return "—";
  const secs = Math.max(0, Math.floor(Date.now() / 1000 - unixSecs));
  if (secs < 60) return `${secs} 秒前`;
  if (secs < 3600) return `${Math.floor(secs / 60)} 分前`;
  if (secs < 86400) return `${Math.floor(secs / 3600)} 小時前`;
  return `${Math.floor(secs / 86400)} 天前`;
}

export default function SkillBank({ onOpenSkill }: SkillBankProps) {
  const { impact } = useHaptics();

  const [state, setState] = useState<LoadState>(BACKEND_WIRED ? "loading" : "unwired");
  const [skills, setSkills] = useState<SkillRow[]>([]);
  const [errMsg, setErrMsg] = useState<string>("");

  // ── async safety: only the latest live request may commit ──
  const seqRef = useRef(0);
  const aliveRef = useRef(true);

  async function load() {
    if (!BACKEND_WIRED) {
      setState("unwired");
      return;
    }
    const mySeq = ++seqRef.current;
    setState("loading");
    setErrMsg("");
    setSkills([]); // clear prior data at load start — no stale overwrite
    try {
      const resp = await safeInvoke<unknown>(LIST_COMMAND, { limit: 200, offset: 0 });
      if (!aliveRef.current || mySeq !== seqRef.current) return; // stale / unmounted
      // The real list endpoint returns SkillListResponse { items, total, ... }.
      // Accept either that envelope or a bare array; anything else = honest empty
      // (a resolved-but-shapeless result is NOT an error — distinguish from throw).
      const items: unknown = Array.isArray(resp)
        ? resp
        : (resp as Record<string, unknown> | null)?.items;
      const rows = Array.isArray(items)
        ? items.map((r) => toRow(r as Record<string, unknown>)).filter((r): r is SkillRow => r !== null)
        : [];
      setSkills(rows);
      setState(rows.length > 0 ? "ready" : "empty");
    } catch (e) {
      if (!aliveRef.current || mySeq !== seqRef.current) return;
      setErrMsg(e instanceof Error ? e.message : String(e));
      setState("error"); // a thrown/rejected call is an error, never faked as empty
    }
  }

  useEffect(() => {
    aliveRef.current = true;
    void load();
    return () => {
      aliveRef.current = false;
      seqRef.current++; // invalidate any in-flight request
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleRefresh = () => {
    impact("medium"); // haptic on the primary action
    void load();
  };

  const handleOpen = (row: SkillRow) => {
    impact("light");
    onOpenSkill?.(String(row.id));
  };

  const polarityTone = (p: string) =>
    p === "positive"
      ? "bg-phantom-success/15 text-phantom-success"
      : p === "negative"
        ? "bg-phantom-danger/15 text-phantom-danger"
        : "bg-phantom-muted/15 text-phantom-muted";

  return (
    <div data-testid="skill-bank" className="flex min-h-screen flex-col bg-phantom-bg text-phantom-text pt-[env(safe-area-inset-top)] pl-[env(safe-area-inset-left)] pr-[env(safe-area-inset-right)]">
      {/* ── Header ── */}
      <header className="flex items-center gap-3 px-4 py-4 border-b border-phantom-border">
        <Library className="h-6 w-6 text-phantom-primary shrink-0" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <h1 className="text-lg font-semibold leading-tight">技能銀行 Skill Bank</h1>
          <p className="text-base text-phantom-muted leading-snug">
            從你的行為自動萃取的可重用技能 Reusable skills
          </p>
        </div>
      </header>

      {/* ── Body (scroll region) ── */}
      <main className="flex-1 overflow-y-auto px-4 py-3">
        {/* Loading */}
        {state === "loading" && (
          <div
            role="status"
            aria-label="載入技能中 Loading skills"
            className="flex flex-col items-center justify-center gap-3 py-16 text-phantom-muted"
          >
            <RefreshCw className="h-6 w-6 animate-spin motion-reduce:animate-none" aria-hidden="true" />
            <span className="text-base">載入中… Loading…</span>
          </div>
        )}

        {/* Error */}
        {state === "error" && (
          <div
            role="alert"
            className="flex flex-col items-center gap-3 rounded-xl border border-phantom-danger/40 bg-phantom-danger/10 px-4 py-8 text-center"
          >
            <AlertTriangle className="h-7 w-7 text-phantom-danger" aria-hidden="true" />
            <p className="text-base font-medium text-phantom-danger">載入失敗 Failed to load</p>
            <p className="text-base text-phantom-muted break-words">{errMsg}</p>
          </div>
        )}

        {/* Honest empty — backend present, no skills yet */}
        {state === "empty" && (
          <div
            role="status"
            aria-label="尚無技能 No skills yet"
            className="flex flex-col items-center gap-3 px-4 py-16 text-center text-phantom-muted"
          >
            <Sparkles className="h-8 w-8 text-phantom-muted" aria-hidden="true" />
            <p className="text-base font-medium text-phantom-text">尚無技能 No skills yet</p>
            <p className="text-base">隨著使用，phantom 會自動萃取技能 Skills appear as you use phantom</p>
          </div>
        )}

        {/* Honest unwired — no real command to call */}
        {state === "unwired" && (
          <div
            role="status"
            aria-label="技能銀行尚未實作 Skill bank not yet implemented"
            className="flex flex-col items-center gap-3 rounded-xl border border-dashed border-phantom-border bg-phantom-card/40 px-4 py-16 text-center text-phantom-muted"
          >
            <Library className="h-8 w-8 text-phantom-muted" aria-hidden="true" />
            <p className="text-base font-medium text-phantom-text">尚未實作 Not yet wired</p>
            <p className="text-base">技能銀行後端尚未接上 The skill-bank backend isn't connected yet</p>
          </div>
        )}

        {/* Ready — skill list */}
        {state === "ready" && (
          <ul className="flex flex-col gap-2" aria-label="技能列表 Skill list">
            {skills.map((s) => (
              <li key={s.id}>
                <button
                  type="button"
                  onClick={() => handleOpen(s)}
                  aria-label={`開啟技能 Open skill: ${s.name}`}
                  className="flex w-full min-h-[44px] items-center gap-3 rounded-xl border border-phantom-border bg-phantom-card px-4 py-3 text-left transition-colors motion-reduce:transition-none active:bg-phantom-border/40"
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-base font-medium text-phantom-text">{s.name}</span>
                      {s.polarity && (
                        <span
                          className={`shrink-0 rounded-full px-2 py-0.5 text-xs font-medium ${polarityTone(s.polarity)}`}
                          aria-label={`極性 Polarity: ${s.polarity}`}
                        >
                          {s.polarity}
                        </span>
                      )}
                    </div>
                    {s.description && (
                      <p className="mt-0.5 line-clamp-2 text-base text-phantom-muted leading-snug">
                        {s.description}
                      </p>
                    )}
                    <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-phantom-muted">
                      {s.source && <span aria-label={`來源 Source: ${s.source}`}>來源 {s.source}</span>}
                      {s.curatorScore != null && (
                        <span aria-label={`品質分數 Quality score: ${s.curatorScore}`}>
                          分數 {s.curatorScore}
                        </span>
                      )}
                      <span aria-label={`建立時間 Created: ${relTime(s.createdAt)}`}>
                        {relTime(s.createdAt)}
                      </span>
                    </div>
                  </div>
                  <ChevronRight className="h-5 w-5 shrink-0 text-phantom-muted" aria-hidden="true" />
                </button>
              </li>
            ))}
          </ul>
        )}
      </main>

      {/* ── Sticky footer: primary CTA (reachability) ── */}
      <footer className="sticky bottom-0 border-t border-phantom-border bg-phantom-bg px-4 pt-3 pb-[max(0.75rem,env(safe-area-inset-bottom))]">
        <button
          type="button"
          onClick={handleRefresh}
          disabled={!BACKEND_WIRED || state === "loading"}
          aria-label="重新整理技能 Refresh skills"
          className="flex w-full min-h-[48px] items-center justify-center gap-2 rounded-xl bg-phantom-primary px-4 text-base font-semibold text-phantom-bg transition-opacity motion-reduce:transition-none disabled:opacity-40"
        >
          <RefreshCw
            className={`h-5 w-5 ${state === "loading" ? "animate-spin motion-reduce:animate-none" : ""}`}
            aria-hidden="true"
          />
          重新整理 Refresh
        </button>
        {!BACKEND_WIRED && (
          <p className="mt-2 text-center text-xs text-phantom-muted">尚未實作 Not yet implemented</p>
        )}
      </footer>
    </div>
  );
}
