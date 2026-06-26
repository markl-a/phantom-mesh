// AppTemplate — a polished, product-grade mobile shell for the phantom mesh
// thin client. Built for an interview demo: it must LOOK like a shipping
// product (dark premium theme, iOS-native feel) AND actually connect.
//
// Why this is self-contained (inline `S` style object, no Tailwind / no
// existing mobile theme): the legacy mobile theme is currently broken, so we
// deliberately avoid depending on it — this renders reliably on device.
//
// Networking is REUSED, not reinvented: every request goes through
// `clusterPost()` in ../../lib/clusterDispatch, which on iOS routes through the
// native `swift_cluster_fetch` (NSURLSession) bridge + HMAC-SHA256 auth — the
// only reliable iOS path over Tailscale. Every handler is a real React onClick
// in module scope (the app CSP blocks inline DOM handlers).

import { useState, useRef, useEffect, useCallback, createContext, useContext } from "react";
import { clusterPost, clusterGet, dispatchToCluster, getLocation, getSensors, classifyIntent, orchBase, swarmStart, swarmFeed } from "../../lib/clusterDispatch";
import MobileApprovals from "./MobileApprovals";
import SupervisorTasks from "./SupervisorTasks";
import SupervisorCaptures from "./SupervisorCaptures";
import SupervisorCoach from "./SupervisorCoach";

// No hardcoded backend IP (ANDAPP-LEAK-002): the user enters their backend
// URL once in the Settings tab; we persist it in localStorage so it survives
// relaunch. Empty string until set.
const BASE_URL_KEY = "phantom.baseUrl";
const DEFAULT_BASE_URL = "";
// Never hardcode the cluster secret — it authenticates cross-machine dispatch.
// Enter it once in the 設定 (Settings) tab.
const DEFAULT_SECRET = "";

// ── Palette ──────────────────────────────────────────────────────────────────
const C = {
  bg: "#0b0d12",
  surface: "#161a22",
  surface2: "#1d222c",
  border: "#272d3a",
  borderSoft: "#21262f",
  accent: "#7c5cff",
  accentSoft: "rgba(124,92,255,0.16)",
  accentDim: "#5a45c0",
  success: "#34d399",
  successSoft: "rgba(52,211,153,0.14)",
  danger: "#f87171",
  warn: "#fbbf24",
  text: "#e7eaf0",
  muted: "#8b93a6",
  faint: "#5b6373",
};

// ── Shared connection + diagnostics state (a tiny context, shared across tabs)─
type LogKind = "info" | "ok" | "err" | "sent";
interface LogEntry { id: number; ts: string; kind: LogKind; text: string }
type ConnStatus = "unknown" | "connecting" | "online" | "offline";

// An online machine the user can dispatch to. {label, url, capabilities}.
interface PeerOpt { label: string; url: string; capabilities?: string[] }

interface AppCtx {
  baseUrl: string;
  secret: string;
  setBaseUrl: (v: string) => void;
  setSecret: (v: string) => void;
  conn: ConnStatus;
  setConn: (v: ConnStatus) => void;
  log: LogEntry[];
  addLog: (kind: LogKind, text: string) => void;
  // Online machines + selection (shared by chat dispatch + 機器 tab).
  peers: PeerOpt[];
  refreshPeers: () => Promise<void>;
  selectedMachines: Set<string>;
  toggleMachine: (url: string) => void;
  setAllMachines: (all: boolean) => void;
}
const Ctx = createContext<AppCtx | null>(null);
// Exported so the P1-2 supervisor tabs (SupervisorTasks/Captures/Coach) consume
// the SAME connection context (baseUrl/secret/addLog) — no parallel state
// manager. This is the single sanctioned cross-file reuse of this context.
export const useApp = () => {
  const v = useContext(Ctx);
  if (!v) throw new Error("useApp outside provider");
  return v;
};

let _logId = 0;

// Host label for the status pill (the Mac node IP, sans scheme/port).
const hostLabel = (url: string) => {
  try { return new URL(url).hostname; } catch { return url.replace(/^https?:\/\//, "").split(":")[0]; }
};

// Extract a human reply from a partner response (json.reply ?? raw text).
const replyOf = (r: { json: unknown; text: string }) =>
  (r.json as { reply?: string } | undefined)?.reply ?? r.text;

// ═══════════════════════════════════════════════════════════════════════════
// Root
// ═══════════════════════════════════════════════════════════════════════════
type Tab = "chat" | "tasks" | "captures" | "coach" | "machines" | "approvals" | "settings";

export default function AppTemplate() {
  const [baseUrl, setBaseUrlRaw] = useState<string>(() => {
    try { return localStorage.getItem(BASE_URL_KEY) ?? DEFAULT_BASE_URL; } catch { return DEFAULT_BASE_URL; }
  });
  // Persist every backend-URL change so it survives relaunch (LEAK-002 fix).
  const setBaseUrl = useCallback((v: string) => {
    try { if (v) localStorage.setItem(BASE_URL_KEY, v); else localStorage.removeItem(BASE_URL_KEY); } catch { /* ignore */ }
    setBaseUrlRaw(v);
  }, []);
  const [secret, setSecret] = useState(DEFAULT_SECRET);
  const [conn, setConn] = useState<ConnStatus>("unknown");
  const [log, setLog] = useState<LogEntry[]>([]);
  const [tab, setTab] = useState<Tab>("chat");
  const [peers, setPeers] = useState<PeerOpt[]>([]);
  const [selectedMachines, setSelectedMachines] = useState<Set<string>>(new Set());

  const addLog = useCallback((kind: LogKind, text: string) => {
    const ts = new Date().toLocaleTimeString();
    setLog((prev) => [...prev.slice(-199), { id: ++_logId, ts, kind, text }]);
  }, []);

  // Load online peers from /rpc/peers, map IP→friendly label, filter online,
  // and default-select the first online peer (if nothing selected yet).
  const refreshPeers = useCallback(async () => {
    addLog("sent", "peers → GET /rpc/peers");
    try {
      const r = await clusterGet(baseUrl, "/rpc/peers");
      const raw = (r.json as { peers?: unknown[] } | undefined)?.peers ?? [];
      const arr = Array.isArray(raw) ? raw : [];
      const opts: PeerOpt[] = [];
      for (const p of arr) {
        const o = (p ?? {}) as Record<string, unknown>;
        const url = String(o.url ?? "");
        if (!url) continue;
        if (o.online === false) continue;
        const ip = url.replace(/^https?:\/\//, "").replace(/:\d+.*$/, "");
        // Label comes straight from the peer's /rpc/peers fields (name/host/id), never a hardcoded IP map.
        const label = String(o.name ?? o.host ?? o.id ?? ip);
        const capabilities = Array.isArray(o.capabilities) ? (o.capabilities as string[]) : [];
        opts.push({ label, url, capabilities });
      }
      if (opts.length) {
        setPeers(opts);
        setSelectedMachines((prev) => (prev.size ? prev : new Set([opts[0].url])));
        addLog("ok", `peers: ${opts.length} online`);
      } else {
        const fallback: PeerOpt = { label: "Mac", url: baseUrl, capabilities: [] };
        setPeers([fallback]);
        setSelectedMachines((prev) => (prev.size ? prev : new Set([fallback.url])));
        addLog("info", "peers: none online — falling back to Mac");
      }
    } catch (e) {
      const fallback: PeerOpt = { label: "Mac", url: baseUrl, capabilities: [] };
      setPeers([fallback]);
      setSelectedMachines((prev) => (prev.size ? prev : new Set([fallback.url])));
      addLog("err", `peers ERROR: ${String(e).slice(0, 120)}`);
    }
  }, [baseUrl, addLog]);

  const toggleMachine = useCallback((url: string) => {
    setSelectedMachines((prev) => {
      const next = new Set(prev);
      if (next.has(url)) next.delete(url); else next.add(url);
      return next;
    });
  }, []);

  const setAllMachines = useCallback((all: boolean) => {
    setSelectedMachines(all ? new Set(peers.map((p) => p.url)) : new Set());
  }, [peers]);

  // Fetch peers once on mount.
  useEffect(() => { refreshPeers(); }, [refreshPeers]);

  // Bridge the native diagnostics callback into our log.
  useEffect(() => {
    (window as { phantomDiag?: (m: string) => void }).phantomDiag = (m: string) =>
      addLog("info", m);
  }, [addLog]);

  // A quiet background health probe on mount so the pill reflects reality.
  // Uses the lightweight GET /healthz (instant, no LLM call, no auth) instead
  // of a full partner/message round-trip — the pill should reflect *reachability*
  // and not burn an LLM turn on every launch.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      setConn("connecting");
      try {
        const r = await clusterGet(baseUrl, "/healthz");
        if (!cancelled) setConn(r.ok ? "online" : "offline");
      } catch {
        if (!cancelled) setConn("offline");
      }
    })();
    return () => { cancelled = true; };
    // run once on mount
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const ctx: AppCtx = {
    baseUrl, secret, setBaseUrl, setSecret, conn, setConn, log, addLog,
    peers, refreshPeers, selectedMachines, toggleMachine, setAllMachines,
  };

  return (
    <Ctx.Provider value={ctx}>
      <div style={S.app}>
        <TopBar />
        <main style={S.main}>
          {tab === "chat" && <ChatTab />}
          {tab === "tasks" && <SupervisorTasks />}
          {tab === "captures" && <SupervisorCaptures />}
          {tab === "coach" && <SupervisorCoach />}
          {tab === "machines" && <MachineTab />}
          {tab === "approvals" && <ApprovalsTab />}
          {tab === "settings" && <SettingsTab />}
        </main>
        <TabBar tab={tab} onChange={setTab} />
      </div>
    </Ctx.Provider>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// Top bar — brand + live connection pill
// ═══════════════════════════════════════════════════════════════════════════
function TopBar() {
  const { conn, baseUrl } = useApp();
  const online = conn === "online";
  const connecting = conn === "connecting";
  const dotColor = online ? C.success : connecting ? C.warn : C.faint;
  const label = online
    ? hostLabel(baseUrl)
    : connecting ? "連線中…" : "offline";
  return (
    <header style={S.topbar}>
      <div style={S.brand}>
        <span style={S.diamond}>◆</span>
        <span style={S.brandText}>phantom mesh</span>
      </div>
      <div style={{ ...S.pill, borderColor: online ? "rgba(52,211,153,0.4)" : C.border }}>
        <span style={{ ...S.pillDot, background: dotColor, boxShadow: online ? `0 0 8px ${C.success}` : "none" }} />
        <span style={{ ...S.pillText, color: online ? C.success : C.muted }}>{label}</span>
      </div>
    </header>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// Bottom tab bar — iOS style
// ═══════════════════════════════════════════════════════════════════════════
const TABS: { key: Tab; icon: string; label: string }[] = [
  { key: "chat", icon: "💬", label: "對話" },
  { key: "tasks", icon: "📋", label: "任務" },
  { key: "captures", icon: "📸", label: "擷取" },
  { key: "coach", icon: "🧭", label: "回顧" },
  { key: "machines", icon: "🖥️", label: "機器" },
  { key: "approvals", icon: "🛡️", label: "審核" },
  { key: "settings", icon: "⚙️", label: "設定" },
];

function TabBar({ tab, onChange }: { tab: Tab; onChange: (t: Tab) => void }) {
  return (
    <nav style={S.tabbar}>
      {TABS.map((t) => {
        const active = t.key === tab;
        return (
          <button key={t.key} style={S.tabBtn} onClick={() => onChange(t.key)}>
            <span style={{ ...S.tabIcon, opacity: active ? 1 : 0.5, transform: active ? "scale(1.08)" : "none" }}>
              {t.icon}
            </span>
            <span style={{ ...S.tabLabel, color: active ? C.accent : C.faint, fontWeight: active ? 700 : 500 }}>
              {t.label}
            </span>
          </button>
        );
      })}
    </nav>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// Tab 1 — Unified Chat (chat + dispatch + sense, routed by LLM intent)
// ═══════════════════════════════════════════════════════════════════════════
// A bubble carries an optional small intent/source tag and may be a "system"
// note (e.g. "select a machine first").
interface Bubble {
  id: number;
  role: "user" | "partner" | "system";
  text: string;
  pending?: boolean;
  error?: boolean;
  tag?: string;        // small intent/source tag, e.g. "⚡ 派工 · Z13"
}
let _bubbleId = 0;

function ChatTab() {
  const {
    baseUrl, secret, addLog, setConn,
    peers, selectedMachines,
  } = useApp();
  const [messages, setMessages] = useState<Bubble[]>([
    { id: ++_bubbleId, role: "partner", text: "嗨,我是你的 phantom 夥伴。隨時告訴我你在想什麼、想去哪、或想在機器上做什麼 — 我會自動判斷並幫你處理。" },
  ]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const endRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => { endRef.current?.scrollIntoView({ behavior: "smooth" }); }, [messages]);

  // Small bubble helpers (mutate the message list immutably).
  const addBubble = (b: Omit<Bubble, "id">): number => {
    const id = ++_bubbleId;
    setMessages((m) => [...m, { ...b, id }]);
    return id;
  };
  const patchBubble = (id: number, patch: Partial<Bubble>) =>
    setMessages((m) => m.map((b) => (b.id === id ? { ...b, ...patch } : b)));

  // ── chat intent ──────────────────────────────────────────────────────────
  const runChat = async (text: string, pendingId: number) => {
    addLog("sent", `chat → "${text.slice(0, 60)}"`);
    try {
      const r = await clusterPost(baseUrl, secret, "/partner/message", { text });
      if (!r.ok) {
        setConn("offline");
        addLog("err", `chat FAILED ${r.status}`);
        patchBubble(pendingId, { pending: false, error: true, text: `連線失敗 (${r.status})。請到「設定」確認連線。` });
        return;
      }
      setConn("online");
      const reply = replyOf(r) || "(空白回覆)";
      addLog("ok", `chat reply (${reply.length} chars)`);
      patchBubble(pendingId, { pending: false, text: reply });
    } catch (e) {
      setConn("offline");
      addLog("err", `chat ERROR: ${String(e).slice(0, 120)}`);
      patchBubble(pendingId, { pending: false, error: true, text: `發生例外:${String(e).slice(0, 120)}` });
    }
  };

  // ── sense intent ─────────────────────────────────────────────────────────
  const runSense = async (text: string, pendingId: number) => {
    addLog("sent", "sense → getLocation()");
    let loc: { lat: number; lon: number; accuracy: number; error?: string } | null = null;
    try {
      const r = await getLocation();
      if (!r.error && !(r.lat === 0 && r.lon === 0)) loc = r;
      else addLog("err", `location failed: ${r.error ?? "0,0"}`);
    } catch (e) {
      addLog("err", `location ERROR: ${String(e).slice(0, 120)}`);
    }

    // No GPS → note the error but still try a chat reply with the raw text.
    if (!loc) {
      patchBubble(pendingId, { tag: "📍 感知", text: "" });
      try {
        const r = await clusterPost(baseUrl, secret, "/partner/message", { text });
        if (!r.ok) {
          setConn("offline");
          patchBubble(pendingId, { pending: false, error: true, tag: "📍 感知", text: `無法取得 GPS 定位,且連線失敗 (${r.status})。` });
          return;
        }
        setConn("online");
        const reply = replyOf(r) || "(空白回覆)";
        patchBubble(pendingId, { pending: false, tag: "📍 感知", text: `（目前無法取得 GPS 定位)\n${reply}` });
      } catch (e) {
        setConn("offline");
        patchBubble(pendingId, { pending: false, error: true, tag: "📍 感知", text: `定位失敗且發生例外:${String(e).slice(0, 120)}` });
      }
      return;
    }

    const lat = loc.lat, lon = loc.lon, acc = Math.round(loc.accuracy);
    addLog("ok", `location ${lat.toFixed(5)}, ${lon.toFixed(5)} (±${acc}m)`);
    try {
      // 1) real location signal
      addLog("sent", "sense → POST /partner/signal");
      const sig = await clusterPost(baseUrl, secret, "/partner/signal", { kind: "location", lat, lon, accuracy: loc.accuracy });
      if (!sig.ok) addLog("err", `signal FAILED ${sig.status}`); else addLog("ok", `signal OK (${sig.status})`);

      // 1b) phone sensors (battery / motion / activity) — best-effort
      let sensorLine = "";
      try {
        const s = await getSensors();
        if (s) {
          const parts: string[] = [];
          if (s.activity) parts.push(`行為=${s.activity}`);
          if (typeof s.steps_today === "number") parts.push(`今日步數=${s.steps_today}`);
          if (typeof s.battery_level === "number") parts.push(`電量=${Math.round((s.battery_level as number) * 100)}%`);
          if (s.battery_state) parts.push(`充電=${s.battery_state}`);
          if (parts.length) { sensorLine = `手機感測:${parts.join("、")}。`; addLog("ok", `sensors: ${parts.join(" ")}`); }
        }
      } catch { /* sensors are best-effort */ }

      // 2) situational request built from REAL data
      const ask = `我現在的實際位置:緯度${lat}、經度${lon}(精度約${acc}公尺)。${sensorLine}我說:${text}。請根據我實際所在位置、手機感測到的行為狀況給我貼合當下的提醒或幫助。`;
      addLog("sent", "sense → POST /partner/message (situational)");
      const msg = await clusterPost(baseUrl, secret, "/partner/message", { text: ask });
      if (!msg.ok) {
        setConn("offline");
        patchBubble(pendingId, { pending: false, error: true, tag: "📍 感知", text: `分析請求失敗 (${msg.status})。請到「設定」確認連線。` });
        return;
      }
      setConn("online");
      const reply = replyOf(msg) || "(空白回覆)";
      addLog("ok", `sense reply (${reply.length} chars)`);
      patchBubble(pendingId, { pending: false, tag: "📍 感知", text: reply });
    } catch (e) {
      setConn("offline");
      addLog("err", `sense ERROR: ${String(e).slice(0, 120)}`);
      patchBubble(pendingId, { pending: false, error: true, tag: "📍 感知", text: `發生例外:${String(e).slice(0, 120)}` });
    }
  };

  // ── dispatch intent ──────────────────────────────────────────────────────
  const runDispatch = async (text: string, pendingId: number, machine: string, task: string) => {
    // Pick targets: a machine the classifier named takes priority; else the
    // machines selected in the 機器 tab. Fuzzy match — the classifier may say
    // "Z13" while the peer label is "Z13 (Windows)" (case-insensitive, either
    // direction contains, also compare the short token before " (").
    let targets: PeerOpt[] = [];
    const m = (machine || "").trim().toLowerCase();
    const named = m
      ? peers.find((p) => {
          const lbl = p.label.toLowerCase();
          const short = lbl.split(" (")[0];
          return lbl === m || short === m || lbl.includes(m) || m.includes(short);
        })
      : undefined;
    if (named) targets = [named];
    else targets = peers.filter((p) => selectedMachines.has(p.url));

    if (targets.length === 0) {
      patchBubble(pendingId, { role: "system", pending: false, text: "請先在「機器」tab 選一台機器,我才能幫你派工。" });
      return;
    }

    const taskText = (task && task.trim()) ? task.trim() : text;
    const prompt = `${taskText}\n\n完成後請用幾句話說明你做了什麼/改了什麼。`;

    // The first pending bubble becomes the first target; create more as needed.
    const bubbleIds: Record<string, number> = {};
    targets.forEach((t, i) => {
      if (i === 0) {
        patchBubble(pendingId, { tag: `⚡ 派工 · ${t.label}`, text: `⚡ 派工 · ${t.label} 執行中…` });
        bubbleIds[t.url] = pendingId;
      } else {
        bubbleIds[t.url] = addBubble({ role: "partner", pending: true, tag: `⚡ 派工 · ${t.label}`, text: `⚡ 派工 · ${t.label} 執行中…` });
      }
    });

    addLog("sent", `dispatch fan-out → ${targets.length} 台: "${taskText.slice(0, 40)}"`);
    await Promise.all(targets.map(async (t) => {
      const id = bubbleIds[t.url];
      try {
        const r = await dispatchToCluster({ coordinatorUrl: t.url, secret, agent: "coder", prompt, maxWaitMs: 120_000 });
        if (r.ok) {
          const secs = typeof r.elapsedMs === "number" ? ` · ${(r.elapsedMs / 1000).toFixed(1)}s` : "";
          patchBubble(id, { pending: false, tag: `⚡ 派工 · ${t.label}${secs}`, text: r.output || "(空白輸出)" });
          addLog("ok", `${t.label}: ${r.elapsedMs ?? 0}ms`);
        } else {
          patchBubble(id, { pending: false, error: true, tag: `⚡ 派工 · ${t.label}`, text: r.error || "未知錯誤" });
          addLog("err", `${t.label}: ${(r.error ?? "").slice(0, 80)}`);
        }
      } catch (e) {
        patchBubble(id, { pending: false, error: true, tag: `⚡ 派工 · ${t.label}`, text: String(e).slice(0, 200) });
        addLog("err", `${t.label} ERROR: ${String(e).slice(0, 80)}`);
      }
    }));
  };

  // ── swarm intent ─────────────────────────────────────────────────────────
  // The whole fleet works the project together. We call a separate
  // ORCHESTRATOR service (Mac, port 7900) that drives `claude` across the 4
  // machines, then render their progress as a LIVE FEED of chat bubbles.
  const runSwarm = async (text: string, pendingId: number) => {
    patchBubble(pendingId, {
      pending: false,
      tag: "🚀 機隊",
      text: "啟動機隊,讓四台機器一起檢查 phantom-mesh…",
    });

    const orch = orchBase(baseUrl);
    addLog("sent", `swarm → ${hostLabel(orch)}:7900 /swarm/start`);
    const r = await swarmStart(orch, text);
    if (r.error || !r.jobId) {
      addLog("err", `swarm start FAILED: ${(r.error ?? "no jobId").slice(0, 120)}`);
      addBubble({
        role: "partner",
        error: true,
        tag: "🚀 機隊",
        text: `機隊啟動失敗:${r.error ?? "沒有取得 job id"}。\n請確認 orchestrator 服務有在 Mac 的 7900 埠執行(${orch})。`,
      });
      return;
    }
    const jobId = r.jobId;
    addLog("ok", `swarm started job=${jobId}`);

    // Live feed: each poll returns the FULL messages array so far; we only
    // render messages we haven't seen yet (index >= rendered).
    let rendered = 0;
    const POLL_MS = 2000;
    const MAX_POLLS = 120; // 240s
    for (let i = 0; i < MAX_POLLS; i++) {
      await new Promise((res) => setTimeout(res, POLL_MS));
      const feed = await swarmFeed(orch, jobId);
      for (let k = rendered; k < feed.messages.length; k++) {
        const msg = feed.messages[k];
        addBubble({
          role: "partner",
          tag: `⚡ ${msg.machine || "機器"}`,
          text: msg.text,
        });
      }
      if (feed.messages.length > rendered) {
        addLog("info", `swarm feed +${feed.messages.length - rendered} (status=${feed.status})`);
        rendered = feed.messages.length;
      }
      if (feed.status === "done") {
        addBubble({ role: "partner", tag: "🚀 機隊", text: `✅ 機隊完成(${rendered} 則回報)` });
        addLog("ok", `swarm done (${rendered} msgs)`);
        return;
      }
      if (feed.status === "error") {
        addBubble({ role: "partner", error: true, tag: "🚀 機隊", text: `機隊中止(已收到 ${rendered} 則回報)。` });
        addLog("err", "swarm errored");
        return;
      }
    }
    addBubble({ role: "partner", error: true, tag: "🚀 機隊", text: `機隊逾時(240 秒,已收到 ${rendered} 則回報)。` });
    addLog("err", "swarm timeout");
  };

  const send = async () => {
    const text = draft.trim();
    if (!text || sending) return;
    setDraft("");
    setSending(true);
    addBubble({ role: "user", text });
    // A single transient pending partner bubble = "classifying…".
    const pendingId = addBubble({ role: "partner", pending: true, text: "", tag: "分類中…" });
    try {
      const intent = await classifyIntent(baseUrl, secret, text, peers.map((p) => p.label));
      addLog("info", `intent=${intent.intent}${intent.machine ? ` machine=${intent.machine}` : ""}`);
      if (intent.intent === "dispatch") {
        await runDispatch(text, pendingId, intent.machine ?? "", intent.task ?? "");
      } else if (intent.intent === "swarm") {
        await runSwarm(text, pendingId);
      } else if (intent.intent === "sense") {
        await runSense(text, pendingId);
      } else {
        patchBubble(pendingId, { tag: undefined });
        await runChat(text, pendingId);
      }
    } finally {
      setSending(false);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(); }
  };

  const selCount = selectedMachines.size;

  return (
    <div style={S.chatWrap}>
      <div style={S.chatScroll}>
        {messages.map((b) => <ChatBubble key={b.id} b={b} />)}
        <div ref={endRef} />
      </div>
      <div style={S.inputBar}>
        <div style={S.inputCol}>
          <div style={S.inputRow}>
            <input
              style={S.chatInput}
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={onKeyDown}
              placeholder="說點什麼…(我會自動判斷對話/派工/感知)"
              autoCapitalize="sentences"
              autoCorrect="on"
            />
            <button
              style={{ ...S.sendBtn, opacity: draft.trim() && !sending ? 1 : 0.45 }}
              onClick={send}
              disabled={!draft.trim() || sending}
              aria-label="送出"
            >
              {sending ? "…" : "↑"}
            </button>
          </div>
          <div style={S.dispatchHint}>派工目標:{selCount} 台已選</div>
        </div>
      </div>
    </div>
  );
}

function ChatBubble({ b }: { b: Bubble }) {
  const isUser = b.role === "user";
  const isSystem = b.role === "system";
  if (isSystem) {
    return (
      <div style={S.systemRow}>
        <div style={S.systemBubble}>{b.text}</div>
      </div>
    );
  }
  return (
    <div style={{ ...S.bubbleRow, justifyContent: isUser ? "flex-end" : "flex-start" }}>
      {!isUser && <div style={S.avatar}>◆</div>}
      <div style={{ display: "flex", flexDirection: "column", alignItems: isUser ? "flex-end" : "flex-start", maxWidth: "78%" }}>
        {b.tag && <span style={S.intentTag}>{b.tag}</span>}
        <div
          style={{
            ...S.bubble,
            ...(isUser ? S.bubbleUser : S.bubblePartner),
            ...(b.error ? { borderColor: "rgba(248,113,113,0.5)", color: C.danger } : {}),
          }}
        >
          {b.pending ? <TypingDots /> : b.text}
        </div>
      </div>
    </div>
  );
}

function TypingDots() {
  return (
    <span style={S.typing}>
      <span style={{ ...S.typingDot, animationDelay: "0s" }} />
      <span style={{ ...S.typingDot, animationDelay: "0.18s" }} />
      <span style={{ ...S.typingDot, animationDelay: "0.36s" }} />
    </span>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// Tab 2 — Machines (機器): live machine list + dispatch-target selector
// ═══════════════════════════════════════════════════════════════════════════
function MachineTab() {
  const { peers, refreshPeers, selectedMachines, toggleMachine, setAllMachines } = useApp();
  const [loading, setLoading] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    try { await refreshPeers(); } finally { setLoading(false); }
  }, [refreshPeers]);

  const onlineCount = peers.length; // peers are already filtered to online
  const selCount = peers.filter((p) => selectedMachines.has(p.url)).length;
  const allSelected = peers.length > 0 && selCount === peers.length;

  return (
    <div style={S.scrollPad}>
      <div style={S.meshHead}>
        <div>
          <div style={S.sectionTitle}>機器</div>
          <div style={S.meshSub}>{selCount}/{peers.length} 已選 · {onlineCount} 在線</div>
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          {peers.length > 1 && (
            <button style={S.ghostBtn} onClick={() => setAllMachines(!allSelected)}>
              {allSelected ? "清除" : "全選"}
            </button>
          )}
          <button style={S.ghostBtn} onClick={refresh} disabled={loading}>
            {loading ? "更新中…" : "↻"}
          </button>
        </div>
      </div>

      <div style={S.noticeBar}>
        勾選的機器會成為「對話」中派工的目標(可多選)。
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        {peers.length === 0 && <div style={S.ctxSub}>讀取機器中…</div>}
        {peers.map((p) => {
          const selected = selectedMachines.has(p.url);
          const role = p.capabilities && p.capabilities.length
            ? p.capabilities.slice(0, 3).join(" · ")
            : "node";
          return (
            <button
              key={p.url}
              style={{ ...S.machineRow, ...(selected ? S.machineRowActive : {}) }}
              onClick={() => toggleMachine(p.url)}
            >
              <span style={{ ...S.nodeDot, background: C.success, boxShadow: `0 0 8px ${C.success}` }} />
              <div style={{ flex: 1, minWidth: 0, textAlign: "left" }}>
                <div style={S.nodeName}>{p.label}</div>
                <div style={S.nodeRole}>{role}</div>
              </div>
              <span style={{ ...S.checkbox, ...(selected ? S.checkboxOn : {}) }}>
                {selected ? "✓" : ""}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// Tab 3 — Approvals (審核): apex-④ pending high-risk approvals
// ═══════════════════════════════════════════════════════════════════════════
// MobileApprovals is a self-contained Tailwind component (dark phantom-* theme,
// fits this shell). It defaults to reading the cluster store for baseUrl/secret,
// but this shell keeps its connection in `useApp()` + localStorage (NOT the
// cluster store), so we pass them in as props.
function ApprovalsTab() {
  const { baseUrl, secret } = useApp();
  // Clear the fixed bottom tab bar (≈84px + safe-area) so the last card's
  // action buttons aren't hidden behind it — same inset the other tabs use.
  return (
    <div
      style={{
        ...S.main,
        overflowY: "auto",
        WebkitOverflowScrolling: "touch",
        paddingBottom: "calc(env(safe-area-inset-bottom) + 96px)",
      }}
    >
      <MobileApprovals baseUrl={baseUrl} secret={secret} />
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// Tab 4 — Settings (connection config + test + diagnostics)
// ═══════════════════════════════════════════════════════════════════════════
function SettingsTab() {
  const { baseUrl, secret, setBaseUrl, setSecret, setConn, log, addLog } = useApp();
  const [testState, setTestState] = useState<"idle" | "running" | "ok" | "fail">("idle");
  const [showLog, setShowLog] = useState(false);
  const logEndRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => { if (showLog) logEndRef.current?.scrollIntoView({ behavior: "smooth" }); }, [log, showLog]);

  const test = async () => {
    setTestState("running");
    setConn("connecting");
    addLog("info", `test → POST ${hostLabel(baseUrl)}/partner/message`);
    try {
      const r = await clusterPost(baseUrl, secret, "/partner/message", { text: "ping" });
      if (r.ok) {
        setTestState("ok"); setConn("online");
        addLog("ok", `test OK (status ${r.status})`);
      } else {
        setTestState("fail"); setConn("offline");
        addLog("err", `test FAILED ${r.status}: ${r.text.slice(0, 160) || "(empty)"}`);
      }
    } catch (e) {
      setTestState("fail"); setConn("offline");
      addLog("err", `test ERROR: ${String(e).slice(0, 160)}`);
    }
  };

  const logColor = (k: LogKind) =>
    k === "ok" ? C.success : k === "err" ? C.danger : k === "sent" ? "#60a5fa" : C.muted;

  return (
    <div style={S.scrollPad}>
      <div style={S.sectionTitle}>設定 · 連線</div>

      <div style={S.card}>
        <label style={S.fieldLabel}>Base URL</label>
        <input
          style={S.field}
          value={baseUrl}
          onChange={(e) => setBaseUrl(e.target.value)}
          autoCapitalize="off" autoCorrect="off" spellCheck={false}
        />
        <label style={{ ...S.fieldLabel, marginTop: 14 }}>Cluster Secret</label>
        <input
          style={S.field}
          value={secret}
          onChange={(e) => setSecret(e.target.value)}
          type="password"
          autoCapitalize="off" autoCorrect="off" spellCheck={false}
        />

        <button
          style={{
            ...S.testBtn,
            background: testState === "ok" ? C.successSoft : testState === "fail" ? "rgba(248,113,113,0.14)" : C.accentSoft,
            borderColor: testState === "ok" ? C.success : testState === "fail" ? C.danger : "rgba(124,92,255,0.5)",
            color: testState === "ok" ? C.success : testState === "fail" ? C.danger : C.accent,
          }}
          onClick={test}
          disabled={testState === "running"}
        >
          {testState === "running" ? "測試中…"
            : testState === "ok" ? "連線成功 ✓"
            : testState === "fail" ? "連線失敗 ✗"
            : "測試連線"}
        </button>
      </div>

      {/* collapsible diagnostics */}
      <div style={S.card}>
        <button style={S.collapseHead} onClick={() => setShowLog((s) => !s)}>
          <span>診斷紀錄</span>
          <span style={{ color: C.faint }}>{log.length} · {showLog ? "▲" : "▼"}</span>
        </button>
        {showLog && (
          <div style={S.logBox}>
            {log.length === 0 && <div style={{ color: C.faint }}>（尚無紀錄）</div>}
            {log.map((e) => (
              <div key={e.id} style={{ color: logColor(e.kind), marginBottom: 3 }}>
                <span style={{ color: C.faint }}>{e.ts} </span>{e.text}
              </div>
            ))}
            <div ref={logEndRef} />
          </div>
        )}
      </div>

      <div style={S.footNote}>
        ◆ phantom mesh · 安全 HMAC 連線 · NSURLSession 原生傳輸
      </div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// Styles
// ═══════════════════════════════════════════════════════════════════════════
const FONT = "-apple-system, BlinkMacSystemFont, 'SF Pro Text', system-ui, sans-serif";

// keyframes injected once for the typing dots
if (typeof document !== "undefined" && !document.getElementById("phantom-apptemplate-kf")) {
  const style = document.createElement("style");
  style.id = "phantom-apptemplate-kf";
  style.textContent = `
@keyframes phantomBlink { 0%,80%,100% { opacity:.25; transform:translateY(0) } 40% { opacity:1; transform:translateY(-2px) } }
`;
  document.head.appendChild(style);
}

const S: Record<string, React.CSSProperties> = {
  app: {
    position: "fixed",
    inset: 0,
    display: "flex",
    flexDirection: "column",
    background: C.bg,
    color: C.text,
    fontFamily: FONT,
    overflow: "hidden",
  },

  // top bar
  topbar: {
    flexShrink: 0,
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    padding: "calc(env(safe-area-inset-top) + 10px) 16px 12px",
    background: "rgba(11,13,18,0.92)",
    backdropFilter: "blur(12px)",
    WebkitBackdropFilter: "blur(12px)",
    borderBottom: `1px solid ${C.borderSoft}`,
  },
  brand: { display: "flex", alignItems: "center", gap: 8 },
  diamond: { color: C.accent, fontSize: 16, filter: `drop-shadow(0 0 6px ${C.accent})` },
  brandText: { fontSize: 16, fontWeight: 700, letterSpacing: 0.3 },
  pill: {
    display: "flex", alignItems: "center", gap: 6,
    padding: "5px 11px",
    borderRadius: 999,
    background: C.surface,
    border: `1px solid ${C.border}`,
  },
  pillDot: { width: 8, height: 8, borderRadius: 999 },
  pillText: { fontSize: 12, fontWeight: 600 },

  // main scroll region
  main: { flex: 1, minHeight: 0, position: "relative", display: "flex", flexDirection: "column" },

  // generic scroll padding for non-chat tabs
  scrollPad: {
    flex: 1,
    overflowY: "auto",
    WebkitOverflowScrolling: "touch",
    padding: "18px 16px calc(env(safe-area-inset-bottom) + 96px)",
  },

  sectionTitle: { fontSize: 22, fontWeight: 800, letterSpacing: -0.3, marginBottom: 14 },

  card: {
    background: C.surface,
    border: `1px solid ${C.border}`,
    borderRadius: 18,
    padding: 18,
    marginBottom: 14,
  },

  // ── chat ──
  chatWrap: { flex: 1, minHeight: 0, display: "flex", flexDirection: "column" },
  chatScroll: {
    flex: 1,
    overflowY: "auto",
    WebkitOverflowScrolling: "touch",
    padding: "16px 14px 12px",
    display: "flex",
    flexDirection: "column",
    gap: 12,
  },
  bubbleRow: { display: "flex", alignItems: "flex-end", gap: 8 },
  avatar: {
    flexShrink: 0,
    width: 28, height: 28, borderRadius: 999,
    background: C.accentSoft,
    border: `1px solid rgba(124,92,255,0.4)`,
    color: C.accent,
    display: "flex", alignItems: "center", justifyContent: "center",
    fontSize: 13,
  },
  bubble: {
    maxWidth: "100%",
    padding: "11px 14px",
    fontSize: 15.5,
    lineHeight: 1.5,
    whiteSpace: "pre-wrap",
    wordBreak: "break-word",
  },
  intentTag: {
    fontSize: 10.5, fontWeight: 700, letterSpacing: 0.3,
    color: C.accent,
    margin: "0 6px 4px",
  },
  systemRow: { display: "flex", justifyContent: "center", padding: "2px 0" },
  systemBubble: {
    fontSize: 12.5, color: C.warn,
    background: "rgba(251,191,36,0.1)",
    border: "1px solid rgba(251,191,36,0.3)",
    borderRadius: 12,
    padding: "8px 13px",
    lineHeight: 1.45,
    maxWidth: "82%",
    textAlign: "center",
  },
  bubbleUser: {
    background: C.accent,
    color: "#fff",
    borderRadius: "18px 18px 4px 18px",
    boxShadow: "0 2px 10px rgba(124,92,255,0.35)",
  },
  bubblePartner: {
    background: C.surface2,
    color: C.text,
    border: `1px solid ${C.border}`,
    borderRadius: "18px 18px 18px 4px",
  },
  typing: { display: "inline-flex", gap: 4, padding: "2px 0", alignItems: "center" },
  typingDot: {
    width: 6, height: 6, borderRadius: 999, background: C.muted,
    display: "inline-block",
    animation: "phantomBlink 1.2s infinite ease-in-out",
  },

  // sticky input bar
  inputBar: {
    flexShrink: 0,
    display: "flex",
    padding: "10px 12px calc(env(safe-area-inset-bottom) + 84px)",
    background: "rgba(11,13,18,0.94)",
    backdropFilter: "blur(12px)",
    WebkitBackdropFilter: "blur(12px)",
    borderTop: `1px solid ${C.borderSoft}`,
  },
  inputCol: { flex: 1, display: "flex", flexDirection: "column", gap: 6 },
  inputRow: { display: "flex", alignItems: "center", gap: 10 },
  dispatchHint: { fontSize: 11, color: C.faint, paddingLeft: 4 },
  chatInput: {
    flex: 1,
    boxSizing: "border-box",
    background: C.surface,
    border: `1px solid ${C.border}`,
    borderRadius: 999,
    color: C.text,
    fontSize: 16,
    padding: "12px 16px",
    outline: "none",
  },
  sendBtn: {
    flexShrink: 0,
    width: 44, height: 44,
    borderRadius: 999,
    background: C.accent,
    color: "#fff",
    border: "none",
    fontSize: 20,
    fontWeight: 800,
    cursor: "pointer",
    boxShadow: "0 2px 10px rgba(124,92,255,0.4)",
  },

  // ── shared ──
  cardLabel: { fontSize: 11, fontWeight: 700, letterSpacing: 0.6, textTransform: "uppercase", color: C.muted, marginBottom: 12 },
  selectAllBtn: { fontSize: 12, fontWeight: 700, color: C.accent, background: "transparent", border: "none", padding: "2px 4px", marginBottom: 12, cursor: "pointer" },
  textarea: {
    width: "100%",
    boxSizing: "border-box",
    background: C.bg,
    border: `1px solid ${C.border}`,
    borderRadius: 12,
    color: C.text,
    fontSize: 15.5,
    lineHeight: 1.5,
    padding: "12px 14px",
    outline: "none",
    resize: "vertical",
    fontFamily: FONT,
  },
  ghostBtnWide: {
    width: "100%",
    background: C.surface2,
    border: `1px solid ${C.border}`,
    color: C.text,
    borderRadius: 12,
    fontSize: 15,
    fontWeight: 700,
    padding: "13px 0",
    cursor: "pointer",
  },
  locErrorBox: {
    fontSize: 13,
    color: C.warn,
    background: "rgba(251,191,36,0.1)",
    border: "1px solid rgba(251,191,36,0.3)",
    borderRadius: 12,
    padding: "11px 13px",
    lineHeight: 1.5,
  },

  // ── dispatch ──
  segmented: {
    display: "flex",
    gap: 4,
    background: C.surface,
    border: `1px solid ${C.border}`,
    borderRadius: 14,
    padding: 4,
    marginBottom: 16,
  },
  segBtn: {
    flex: 1,
    background: "none",
    border: "none",
    color: C.muted,
    fontSize: 14.5,
    fontWeight: 700,
    padding: "11px 0",
    borderRadius: 10,
    cursor: "pointer",
  },
  segBtnActive: {
    background: C.accent,
    color: "#fff",
    boxShadow: "0 2px 8px rgba(124,92,255,0.4)",
  },
  chipRow: { display: "flex", gap: 8, flexWrap: "wrap" },
  machineChip: {
    display: "inline-flex",
    alignItems: "center",
    gap: 7,
    background: C.surface2,
    border: `1px solid ${C.border}`,
    color: C.text,
    fontSize: 13.5,
    fontWeight: 600,
    padding: "9px 13px",
    borderRadius: 999,
    cursor: "pointer",
  },
  machineChipActive: {
    background: C.accentSoft,
    borderColor: "rgba(124,92,255,0.6)",
    color: C.accent,
  },
  machineChipDot: { width: 8, height: 8, borderRadius: 999, flexShrink: 0 },
  resultHead: {
    display: "flex",
    alignItems: "center",
    gap: 9,
    marginBottom: 12,
  },
  resultMachine: { fontSize: 14.5, fontWeight: 700, flex: 1, minWidth: 0 },
  resultElapsed: { color: C.muted, fontWeight: 600 },
  resultBadge: { fontSize: 12, fontWeight: 800, letterSpacing: 0.4, textTransform: "uppercase" },
  codeOutput: {
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
    fontSize: 12.5,
    lineHeight: 1.55,
    whiteSpace: "pre-wrap",
    wordBreak: "break-word",
    color: C.text,
    background: C.bg,
    border: `1px solid ${C.border}`,
    borderRadius: 10,
    padding: "12px 13px",
    maxHeight: 360,
    overflowY: "auto",
    WebkitOverflowScrolling: "touch",
  },

  // ── sense ──
  ctxRow: { display: "flex", gap: 12, alignItems: "flex-start" },
  ctxIcon: { fontSize: 22, lineHeight: "26px" },
  ctxLabel: { fontSize: 11, fontWeight: 700, letterSpacing: 0.6, textTransform: "uppercase", color: C.muted },
  ctxValue: { fontSize: 16, fontWeight: 700, marginTop: 3 },
  ctxSub: { fontSize: 12.5, color: C.muted, marginTop: 3 },
  tagRow: { display: "flex", gap: 8, marginTop: 14, flexWrap: "wrap" },
  tag: {
    fontSize: 12, fontWeight: 600,
    color: C.accent,
    background: C.accentSoft,
    border: `1px solid rgba(124,92,255,0.35)`,
    borderRadius: 999,
    padding: "5px 11px",
  },
  heroBtn: {
    width: "100%",
    background: `linear-gradient(135deg, ${C.accent}, ${C.accentDim})`,
    color: "#fff",
    border: "none",
    borderRadius: 16,
    fontSize: 17,
    fontWeight: 800,
    padding: "18px 0",
    cursor: "pointer",
    boxShadow: "0 6px 20px rgba(124,92,255,0.4)",
  },
  heroHint: { fontSize: 12.5, color: C.muted, textAlign: "center", marginTop: 10, lineHeight: 1.5 },
  analysisHead: {
    display: "flex", alignItems: "center", gap: 8,
    fontSize: 12, fontWeight: 700, letterSpacing: 0.5, textTransform: "uppercase",
    color: C.muted, marginBottom: 12,
  },
  analysisDot: { width: 7, height: 7, borderRadius: 999, background: C.success, boxShadow: `0 0 8px ${C.success}` },
  analysisBody: { fontSize: 15.5, lineHeight: 1.62, whiteSpace: "pre-wrap", wordBreak: "break-word" },

  // ── mesh ──
  meshHead: { display: "flex", alignItems: "flex-start", justifyContent: "space-between", marginBottom: 14 },
  meshSub: { fontSize: 13, color: C.muted, marginTop: -8 },
  ghostBtn: {
    background: C.surface,
    border: `1px solid ${C.border}`,
    color: C.text,
    borderRadius: 999,
    fontSize: 13,
    fontWeight: 600,
    padding: "8px 14px",
    cursor: "pointer",
  },
  noticeBar: {
    fontSize: 12.5,
    color: C.warn,
    background: "rgba(251,191,36,0.1)",
    border: "1px solid rgba(251,191,36,0.3)",
    borderRadius: 12,
    padding: "10px 12px",
    marginBottom: 14,
    lineHeight: 1.45,
  },
  nodeCard: {
    display: "flex",
    alignItems: "center",
    gap: 13,
    background: C.surface,
    border: `1px solid ${C.border}`,
    borderRadius: 16,
    padding: "15px 16px",
  },
  nodeDot: { flexShrink: 0, width: 10, height: 10, borderRadius: 999 },
  nodeName: { fontSize: 15.5, fontWeight: 700, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" },
  nodeRole: { fontSize: 12.5, color: C.muted, marginTop: 2 },
  nodeStatus: { fontSize: 13, fontWeight: 700 },
  nodeLatency: { fontSize: 11.5, color: C.faint, marginTop: 2 },
  machineRow: {
    display: "flex",
    alignItems: "center",
    gap: 13,
    background: C.surface,
    border: `1px solid ${C.border}`,
    borderRadius: 16,
    padding: "15px 16px",
    cursor: "pointer",
    width: "100%",
  },
  machineRowActive: {
    background: C.accentSoft,
    borderColor: "rgba(124,92,255,0.6)",
  },
  checkbox: {
    flexShrink: 0,
    width: 24, height: 24,
    borderRadius: 7,
    border: `1.5px solid ${C.border}`,
    display: "flex", alignItems: "center", justifyContent: "center",
    fontSize: 14, fontWeight: 800,
    color: "transparent",
    background: C.bg,
  },
  checkboxOn: {
    background: C.accent,
    borderColor: C.accent,
    color: "#fff",
  },

  // ── settings ──
  fieldLabel: { display: "block", fontSize: 12, fontWeight: 600, color: C.muted, marginBottom: 7 },
  field: {
    width: "100%",
    boxSizing: "border-box",
    background: C.bg,
    border: `1px solid ${C.border}`,
    borderRadius: 12,
    color: C.text,
    fontSize: 16,
    padding: "13px 14px",
    outline: "none",
  },
  testBtn: {
    width: "100%",
    marginTop: 18,
    border: "1px solid",
    borderRadius: 12,
    fontSize: 16,
    fontWeight: 700,
    padding: "14px 0",
    cursor: "pointer",
  },
  collapseHead: {
    width: "100%",
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    background: "none",
    border: "none",
    color: C.text,
    fontSize: 15,
    fontWeight: 700,
    padding: 0,
    cursor: "pointer",
  },
  logBox: {
    marginTop: 14,
    background: C.bg,
    border: `1px solid ${C.border}`,
    borderRadius: 12,
    padding: 12,
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
    fontSize: 12,
    lineHeight: 1.5,
    maxHeight: 260,
    overflowY: "auto",
    WebkitOverflowScrolling: "touch",
  },
  footNote: { fontSize: 11.5, color: C.faint, textAlign: "center", marginTop: 8, letterSpacing: 0.3 },

  // ── tab bar ──
  tabbar: {
    flexShrink: 0,
    position: "fixed",
    left: 0, right: 0, bottom: 0,
    display: "flex",
    background: "rgba(16,18,24,0.94)",
    backdropFilter: "blur(16px)",
    WebkitBackdropFilter: "blur(16px)",
    borderTop: `1px solid ${C.border}`,
    paddingBottom: "env(safe-area-inset-bottom)",
    zIndex: 50,
  },
  tabBtn: {
    flex: 1,
    background: "none",
    border: "none",
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
    gap: 3,
    padding: "9px 0 7px",
    cursor: "pointer",
  },
  tabIcon: { fontSize: 21, transition: "transform 0.15s, opacity 0.15s" },
  tabLabel: { fontSize: 10.5, letterSpacing: 0.2 },
};
