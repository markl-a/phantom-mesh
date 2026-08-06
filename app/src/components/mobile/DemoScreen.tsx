// DemoScreen — a single self-contained, RELIABLE mobile demo screen for an
// interview demo. It deliberately bypasses the existing (broken) mobile UI.
//
// Why this works where the old UI didn't:
//   1. All networking goes through `clusterPost()` in ../../lib/clusterDispatch,
//      which on iOS routes through the native `swift_cluster_fetch`
//      (NSURLSession) bridge — the ONLY reliable iOS networking path over
//      Tailscale (http:// is blocked as mixed-content by WKWebView, and
//      tauri-plugin-http's reqwest backend silently times out on Tailscale
//      IPs from a physical device).
//   2. Every handler is a real React onClick in module scope — NOT an inline
//      DOM string. The app's CSP blocks inline handlers, which is why the
//      old "connect" button was dead.
//
// HMAC: `clusterPost` signs the EXACT serialized request body with
// HMAC-SHA256(secret, body) and sends it as the `X-Cluster-Auth` header —
// the same primitive dispatchToCluster uses.

import { useState, useRef, useEffect, useCallback } from "react";
import { clusterPost } from "../../lib/clusterDispatch";

// No hardcoded backend IP: user enters the backend URL; persisted in
// localStorage (shared with AppTemplate) so it survives relaunch.
const BASE_URL_KEY = "spectyn.baseUrl";
const DEFAULT_BASE_URL = "";
// Never hardcode the cluster secret — enter it in the Settings field.
const DEFAULT_SECRET = "";

type LogKind = "info" | "ok" | "err" | "sent";
interface LogEntry {
  id: number;
  ts: string;
  kind: LogKind;
  text: string;
}

let _logId = 0;

export default function DemoScreen() {
  const [baseUrl, setBaseUrlRaw] = useState<string>(() => {
    try { return localStorage.getItem(BASE_URL_KEY) ?? DEFAULT_BASE_URL; } catch { return DEFAULT_BASE_URL; }
  });
  const setBaseUrl = useCallback((v: string) => {
    try { if (v) localStorage.setItem(BASE_URL_KEY, v); else localStorage.removeItem(BASE_URL_KEY); } catch { /* ignore */ }
    setBaseUrlRaw(v);
  }, []);
  const [secret, setSecret] = useState(DEFAULT_SECRET);

  const [testState, setTestState] = useState<"idle" | "running" | "ok" | "fail">("idle");

  const [messageText, setMessageText] = useState("");
  const [chainARunning, setChainARunning] = useState(false);
  const [chainAReply, setChainAReply] = useState<string>("");

  const [chainBRunning, setChainBRunning] = useState(false);
  const [chainBReply, setChainBReply] = useState<string>("");

  const [log, setLog] = useState<LogEntry[]>([]);
  const logEndRef = useRef<HTMLDivElement | null>(null);

  const addLog = (kind: LogKind, text: string) => {
    const ts = new Date().toLocaleTimeString();
    setLog((prev) => [...prev, { id: ++_logId, ts, kind, text }]);
  };

  useEffect(() => {
    logEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [log]);

  // ── Connection test: POST /partner/message with a trivial ping ────────────
  const handleTest = async () => {
    setTestState("running");
    addLog("info", `Test → POST ${baseUrl}/partner/message`);
    try {
      const r = await clusterPost(baseUrl, secret, "/partner/message", { text: "ping" });
      if (r.ok) {
        setTestState("ok");
        addLog("ok", `Test OK (status ${r.status})`);
      } else {
        setTestState("fail");
        addLog("err", `Test FAILED — status ${r.status}: ${r.text.slice(0, 200) || "(empty body)"}`);
      }
    } catch (e) {
      setTestState("fail");
      addLog("err", `Test ERROR: ${String(e).slice(0, 200)}`);
    }
  };

  // ── Chain A: free-text → /partner/message → show reply ────────────────────
  const handleSendMessage = async () => {
    const text = messageText.trim();
    if (!text) {
      addLog("err", "Chain A: text is empty");
      return;
    }
    setChainARunning(true);
    setChainAReply("");
    addLog("sent", `Chain A → "${text}"`);
    try {
      const r = await clusterPost(baseUrl, secret, "/partner/message", { text });
      if (!r.ok) {
        addLog("err", `Chain A FAILED — status ${r.status}: ${r.text.slice(0, 300) || "(empty body)"}`);
        setChainAReply(`✗ 錯誤 (status ${r.status}): ${r.text.slice(0, 300)}`);
        return;
      }
      const reply = (r.json as { reply?: string } | undefined)?.reply ?? r.text;
      setChainAReply(reply || "(empty reply)");
      addLog("ok", `Chain A reply (${(reply || "").length} chars)`);
    } catch (e) {
      addLog("err", `Chain A ERROR: ${String(e).slice(0, 300)}`);
      setChainAReply(`✗ 例外: ${String(e).slice(0, 300)}`);
    } finally {
      setChainARunning(false);
    }
  };

  // ── Chain B: triggered "I arrived" → signal + message → show analysis ─────
  //
  // NOTE: live GPS is NOT available here — that would require native
  // CoreLocation, which this thin client doesn't bridge. This button uses a
  // hardcoded triggered-location as the honest demo path: it posts a location
  // signal, THEN asks the partner for situational advice for the interview.
  const handleArrived = async () => {
    setChainBRunning(true);
    setChainBReply("");
    const signalBody = {
      kind: "location",
      place: "interview venue",
      address: "桃園市中壢區福德里大華路57號",
      lat: 24.9637,
      lon: 121.2588,
    };
    const messageBody = {
      text: "我到了面試地點(中壢大華路57號),正要面試一個 AI 工程師職位,根據現場狀況給我相關的提醒或幫助",
    };
    try {
      addLog("sent", "Chain B step 1 → POST /partner/signal (location)");
      const sig = await clusterPost(baseUrl, secret, "/partner/signal", signalBody);
      if (!sig.ok) {
        addLog("err", `Chain B signal FAILED — status ${sig.status}: ${sig.text.slice(0, 300)}`);
        setChainBReply(`✗ signal 錯誤 (status ${sig.status}): ${sig.text.slice(0, 300)}`);
        return;
      }
      addLog("ok", `Chain B signal OK (status ${sig.status})`);

      addLog("sent", "Chain B step 2 → POST /partner/message (analysis request)");
      const msg = await clusterPost(baseUrl, secret, "/partner/message", messageBody);
      if (!msg.ok) {
        addLog("err", `Chain B message FAILED — status ${msg.status}: ${msg.text.slice(0, 300)}`);
        setChainBReply(`✗ message 錯誤 (status ${msg.status}): ${msg.text.slice(0, 300)}`);
        return;
      }
      const reply = (msg.json as { reply?: string } | undefined)?.reply ?? msg.text;
      setChainBReply(reply || "(empty reply)");
      addLog("ok", `Chain B analysis reply (${(reply || "").length} chars)`);
    } catch (e) {
      addLog("err", `Chain B ERROR: ${String(e).slice(0, 300)}`);
      setChainBReply(`✗ 例外: ${String(e).slice(0, 300)}`);
    } finally {
      setChainBRunning(false);
    }
  };

  const logColor = (kind: LogKind) =>
    kind === "ok" ? "#4ade80" : kind === "err" ? "#f87171" : kind === "sent" ? "#60a5fa" : "#9ca3af";

  return (
    <div style={S.root}>
      <div style={S.scroll}>
        <h1 style={S.h1}>Spectyn Demo</h1>

        {/* ── Connection row ─────────────────────────────────────────────── */}
        <section style={S.card}>
          <label style={S.label}>Base URL</label>
          <input
            style={S.input}
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
          />
          <label style={S.label}>Secret</label>
          <input
            style={S.input}
            value={secret}
            onChange={(e) => setSecret(e.target.value)}
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
          />
          <button style={S.btnSecondary} onClick={handleTest} disabled={testState === "running"}>
            {testState === "running" ? "測試中…" : "Test 連線"}
            {testState === "ok" && "  ✓"}
            {testState === "fail" && "  ✗"}
          </button>
        </section>

        {/* ── Chain A ────────────────────────────────────────────────────── */}
        <section style={S.card}>
          <h2 style={S.h2}>Chain A · 訊息</h2>
          <textarea
            style={S.textarea}
            value={messageText}
            onChange={(e) => setMessageText(e.target.value)}
            placeholder="輸入訊息…"
            rows={3}
          />
          <button style={S.btnPrimary} onClick={handleSendMessage} disabled={chainARunning}>
            {chainARunning ? "傳送中…" : "Send →"}
          </button>
          {chainAReply && (
            <div style={S.replyBox}>
              <div style={S.replyLabel}>回覆</div>
              <div style={S.replyText}>{chainAReply}</div>
            </div>
          )}
        </section>

        {/* ── Chain B ────────────────────────────────────────────────────── */}
        <section style={S.card}>
          <h2 style={S.h2}>Chain B · 現場觸發</h2>
          <button style={S.btnBig} onClick={handleArrived} disabled={chainBRunning}>
            {chainBRunning ? "處理中…" : "📍 我到了面試現場"}
          </button>
          {chainBReply && (
            <div style={S.replyBox}>
              <div style={S.replyLabel}>現場分析</div>
              <div style={S.replyText}>{chainBReply}</div>
            </div>
          )}
        </section>

        {/* ── Log ────────────────────────────────────────────────────────── */}
        <section style={S.card}>
          <h2 style={S.h2}>Log</h2>
          <div style={S.logBox}>
            {log.length === 0 && <div style={{ color: "#6b7280" }}>（尚無紀錄）</div>}
            {log.map((e) => (
              <div key={e.id} style={{ color: logColor(e.kind), marginBottom: 4 }}>
                <span style={{ color: "#6b7280" }}>{e.ts} </span>
                {e.text}
              </div>
            ))}
            <div ref={logEndRef} />
          </div>
        </section>
      </div>
    </div>
  );
}

// Inline style objects (self-contained — no Tailwind dependency, so this
// screen renders correctly even if the rest of the mobile theme misbehaves).
const S: Record<string, React.CSSProperties> = {
  root: {
    position: "fixed",
    inset: 0,
    background: "#0b0f17",
    color: "#e5e7eb",
    fontFamily: "-apple-system, system-ui, sans-serif",
  },
  scroll: {
    height: "100%",
    overflowY: "auto",
    padding: "calc(env(safe-area-inset-top) + 16px) 16px calc(env(safe-area-inset-bottom) + 32px) 16px",
    WebkitOverflowScrolling: "touch",
  },
  h1: { fontSize: 24, fontWeight: 800, margin: "4px 0 16px", color: "#a78bfa" },
  h2: { fontSize: 17, fontWeight: 700, margin: "0 0 10px", color: "#c4b5fd" },
  card: {
    background: "#141a26",
    border: "1px solid #232b3a",
    borderRadius: 14,
    padding: 16,
    marginBottom: 16,
  },
  label: { display: "block", fontSize: 12, color: "#9ca3af", margin: "6px 0 4px" },
  input: {
    width: "100%",
    boxSizing: "border-box",
    background: "#0b0f17",
    border: "1px solid #2d3648",
    borderRadius: 10,
    color: "#e5e7eb",
    fontSize: 16,
    padding: "12px 12px",
    marginBottom: 4,
  },
  textarea: {
    width: "100%",
    boxSizing: "border-box",
    background: "#0b0f17",
    border: "1px solid #2d3648",
    borderRadius: 10,
    color: "#e5e7eb",
    fontSize: 16,
    padding: "12px 12px",
    marginBottom: 10,
    resize: "vertical",
    fontFamily: "inherit",
  },
  btnPrimary: {
    width: "100%",
    background: "#7c3aed",
    color: "#fff",
    border: "none",
    borderRadius: 10,
    fontSize: 17,
    fontWeight: 700,
    padding: "14px 0",
    cursor: "pointer",
  },
  btnSecondary: {
    width: "100%",
    background: "#1f2937",
    color: "#e5e7eb",
    border: "1px solid #374151",
    borderRadius: 10,
    fontSize: 16,
    fontWeight: 600,
    padding: "12px 0",
    marginTop: 8,
    cursor: "pointer",
  },
  btnBig: {
    width: "100%",
    background: "#059669",
    color: "#fff",
    border: "none",
    borderRadius: 12,
    fontSize: 19,
    fontWeight: 800,
    padding: "18px 0",
    cursor: "pointer",
  },
  replyBox: {
    marginTop: 14,
    background: "#0b0f17",
    border: "1px solid #2d3648",
    borderRadius: 10,
    padding: 14,
  },
  replyLabel: { fontSize: 12, color: "#9ca3af", marginBottom: 6 },
  replyText: { fontSize: 16, lineHeight: 1.55, whiteSpace: "pre-wrap", wordBreak: "break-word" },
  logBox: {
    background: "#0b0f17",
    border: "1px solid #2d3648",
    borderRadius: 10,
    padding: 12,
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
    fontSize: 12.5,
    maxHeight: 240,
    overflowY: "auto",
    WebkitOverflowScrolling: "touch",
  },
};
