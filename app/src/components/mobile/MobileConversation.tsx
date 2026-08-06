import { useState, useEffect, useRef } from "react";
import { useNavigate } from "react-router-dom";
import { safeInvoke as invoke, isTauri } from "../../lib/tauri-compat";
import {
  Send, Trash2, Square, RotateCcw, Sparkles, Network, Zap,
} from "lucide-react";
import type { Message } from "../../lib/types";
import ToolCallDisplay from "../conversation/ToolCallDisplay";
import Markdown, { CopyButton } from "./Markdown";
import { useClusterModeStore } from "../../stores/clusterModeStore";
import { dispatchToCluster } from "../../lib/clusterDispatch";
import { reducedMotionScrollBehavior } from "../../lib/motion";
import {
  selectProvider,
  streamComplete,
  buildRequest,
  describeError,
  type ProviderMessage,
  type StreamHandle,
} from "../../lib/providers";

const CHAT_ID = "mobile";

async function safeListen(event: string, handler: (e: { payload: unknown }) => void) {
  if (isTauri()) {
    const { listen } = await import("@tauri-apps/api/event");
    return listen(event, handler);
  }
  return { unsubscribe: () => {} } as { unsubscribe: () => void };
}

const PROMPT_CATEGORIES = [
  {
    title: "創作",
    icon: "✍️",
    prompts: [
      "幫我寫一段關於秋天的短詩",
      "用三個 emoji 講個小故事",
      "想 5 個科幻短篇主題",
    ],
  },
  {
    title: "學習",
    icon: "📚",
    prompts: [
      "用 5 句話講解 Rust ownership",
      "推薦 3 本入門 ML 的書",
      "解釋 ELI5：什麼是 quantum entanglement",
    ],
  },
  {
    title: "工作",
    icon: "💼",
    prompts: [
      "幫我把這封信寫得更專業（請接著貼）",
      "整理會議記錄要點 (請貼內容)",
      "翻譯成英文 (請接著貼)",
    ],
  },
  {
    title: "生活",
    icon: "🌱",
    prompts: [
      "今天適合做什麼運動？",
      "推薦 5 道 30 分鐘內的家常菜",
      "怎麼養成早睡早起的習慣？",
    ],
  },
];

export default function MobileConversation() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const abortRef = useRef<{ unlisten?: (() => void) | { unsubscribe?: () => void } } | null>(null);

  // SPEC-14 providers wire direct mode (skips agent loop + cluster path).
  // Persisted lightly via state only — phone users typically toggle per
  // session, and a dedicated store would over-engineer a single bit.
  const [providerMode, setProviderMode] = useState(false);
  const [activeProvider, setActiveProvider] = useState<string | null>(null);
  const streamRef = useRef<StreamHandle | null>(null);

  // Cluster mode
  const cluster = useClusterModeStore();
  const navigate = useNavigate();

  // Resolve a provider slug when the toggle flips on so the header label
  // reflects the actual upstream before the user even types.
  useEffect(() => {
    if (!providerMode) {
      setActiveProvider(null);
      return;
    }
    selectProvider("commodity", "interactive")
      .then(setActiveProvider)
      .catch((e) => {
        setActiveProvider(null);
        setError(`Provider 解析失敗：${describeError(String(e))}`);
      });
  }, [providerMode]);

  // Load history once
  useEffect(() => {
    (async () => {
      try {
        const r = await invoke<{ messages?: Message[] }>(
          "get_conversation_history",
          { chat_id: CHAT_ID }
        );
        if (r?.messages?.length) setMessages(r.messages);
      } catch { /* first run */ }
    })();
  }, []);

  // Auto-scroll
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: reducedMotionScrollBehavior() });
  }, [messages, loading]);

  const detachListener = () => {
    const u = abortRef.current?.unlisten;
    if (typeof u === "function") u();
    else (u as { unsubscribe?: () => void } | undefined)?.unsubscribe?.();
    abortRef.current = null;
  };

  // ── Provider-mode send path ─────────────────────────────────────────────
  // Bypasses both the agent loop and the cluster coordinator; talks straight
  // to the SPEC-14 providers wire via the Tauri command surface registered
  // in `commands::providers_wire`. Used only when the user flips the lightning
  // toggle in the header.
  const sendViaProvider = async (text: string) => {
    const priorMessages = messages.filter(m => !(m.role === "assistant" && m.content === ""));
    const providerMessages: ProviderMessage[] = [
      ...priorMessages.map(m => ({ role: m.role, content: m.content, images: [] })),
      { role: "user" as const, content: text, images: [] },
    ];

    let resolvedProvider = activeProvider;
    try {
      if (!resolvedProvider) {
        resolvedProvider = await selectProvider("commodity", "interactive");
        setActiveProvider(resolvedProvider);
      }
    } catch (e) {
      setError(`Provider 解析失敗：${describeError(String(e))}`);
      setMessages(prev => prev.slice(0, -1));
      setLoading(false);
      return;
    }

    const req = buildRequest({
      model: resolvedProvider ?? "",
      messages: providerMessages,
      temperature: 0.7,
    });

    const handle = await streamComplete(req, {
      onToken: (token) => {
        setMessages(prev => {
          const updated = [...prev];
          const idx = updated.length - 1;
          if (updated[idx]?.role === "assistant") {
            updated[idx] = { ...updated[idx], content: token };
          }
          return updated;
        });
      },
      onDone: (response) => {
        setMessages(prev => {
          const updated = [...prev];
          const idx = updated.length - 1;
          if (updated[idx]?.role === "assistant") {
            updated[idx] = {
              ...updated[idx],
              content: response.text,
              provider: resolvedProvider ?? undefined,
              model: response.modelUsed,
            };
          }
          return updated;
        });
        setLoading(false);
        streamRef.current = null;
      },
      onError: (err) => {
        setError(describeError(err));
        setMessages(prev => prev.slice(0, -1));
        setLoading(false);
        streamRef.current = null;
      },
    });
    streamRef.current = handle;
  };

  const send = async (textOverride?: string) => {
    const text = (textOverride ?? input).trim();
    if (!text || loading) return;

    // Local mode (cluster toggle off) is now usable — broker login +
    // vault sync seeds ~/.spectyn-mesh/{env, agents.toml} on the device,
    // and lib.rs setup() loads env vars into the running process. Only
    // require the cluster setup when the user explicitly turned the
    // cluster toggle ON but hasn't filled coordinator URL + secret.
    // Provider-direct mode bypasses the cluster check entirely.
    if (!providerMode && cluster.enabled && !cluster.isConfigured()) {
      setError("Cluster 模式已開但尚未設定 — 點到「設定 → Cluster 派送」填 coordinator URL + secret，或關掉上方 toggle 用本機模式");
      return;
    }

    setInput("");
    setError(null);
    setLoading(true);

    const userMsg: Message = { role: "user", content: text };
    const placeholderAssistant: Message = { role: "assistant", content: "" };
    setMessages(prev => [...prev, userMsg, placeholderAssistant]);

    // ── PROVIDER-DIRECT MODE: skip agent loop + cluster entirely ──────────
    if (providerMode) {
      try {
        await sendViaProvider(text);
      } catch (e) {
        setError(describeError(String(e)));
        setMessages(prev => prev.slice(0, -1));
        setLoading(false);
      }
      return;
    }

    // ── CLUSTER MODE: dispatch via coordinator ────────────────────────────
    if (cluster.enabled && cluster.isConfigured()) {
      try {
        const r = await dispatchToCluster({
          coordinatorUrl: cluster.coordinatorUrl,
          secret: cluster.clusterSecret,
          agent: "master",
          prompt: text,
        });
        setMessages(prev => {
          const updated = [...prev];
          const idx = updated.length - 1;
          if (updated[idx]?.role === "assistant") {
            updated[idx] = {
              ...updated[idx],
              content: r.ok
                ? (r.output ?? "(no output)")
                : `**錯誤**: ${describeError(r.error ?? "unknown")}`,
            };
          }
          return updated;
        });
      } catch (e) {
        // Cluster dispatch failed to even reach the coordinator (network /
        // HMAC / timeout). Humanise instead of leaking the raw exception, and
        // point at the usual fixes.
        setError(`派送失敗：${describeError(String(e))}（確認 coordinator URL + secret，或改用本機模式）`);
        setMessages(prev => prev.slice(0, -1));
      } finally {
        setLoading(false);
      }
      return;
    }

    // ── LOCAL MODE: existing Tauri send_message path ──────────────────────
    let streamReceived = false;

    try {
      const unlisten = await safeListen("agent_event", (e) => {
        const ev = e.payload as { type: string; content?: string };
        if (ev.type === "token" && ev.content) {
          streamReceived = true;
          setMessages(prev => {
            const updated = [...prev];
            const idx = updated.length - 1;
            if (updated[idx]?.role === "assistant") {
              updated[idx] = { ...updated[idx], content: (updated[idx].content || "") + ev.content };
            }
            return updated;
          });
        } else if (ev.type === "done") {
          setLoading(false);
          detachListener();
        }
      });
      abortRef.current = { unlisten };

      const resp = await invoke<Record<string, unknown>>("send_message", {
        prompt: text,
        chat_id: CHAT_ID,
      });

      if (!streamReceived) {
        let content: string;
        if (typeof resp === "string") content = resp;
        else if (resp && typeof resp === "object") {
          content = String(
            resp.output ?? resp["result"] ?? resp["message"] ?? resp["content"] ?? JSON.stringify(resp)
          );
        } else content = String(resp);

        setMessages(prev => {
          const updated = [...prev];
          const idx = updated.length - 1;
          if (updated[idx]?.role === "assistant") {
            updated[idx] = { ...updated[idx], content };
          }
          return updated;
        });
      }
      setLoading(false);
      detachListener();
    } catch (e) {
      // Humanise rather than leaking a raw "TypeError: Failed to fetch".
      // describeError now maps the no-key / providers-failed case to an
      // actionable "set a key in Settings" prompt, so we don't append a
      // second hint here — that doubled up the message and mis-fired on real
      // network errors (which aren't about a missing key).
      setError(describeError(String(e)));
      setMessages(prev => prev.slice(0, -1));
      setLoading(false);
      detachListener();
    }
  };

  const stop = () => {
    setLoading(false);
    detachListener();
    // Provider-mode stream uses its own handle — detach that listener too.
    streamRef.current?.cancel();
    streamRef.current = null;
    // mark last message as interrupted if empty
    setMessages(prev => {
      const updated = [...prev];
      const idx = updated.length - 1;
      if (updated[idx]?.role === "assistant" && !updated[idx].content) {
        updated[idx] = { ...updated[idx], content: "_(已中斷)_" };
      }
      return updated;
    });
  };

  const regenerate = async () => {
    if (loading || messages.length < 2) return;
    // Find last user message
    let lastUserIdx = -1;
    for (let i = messages.length - 1; i >= 0; i--) {
      if (messages[i].role === "user") { lastUserIdx = i; break; }
    }
    if (lastUserIdx === -1) return;
    const lastPrompt = messages[lastUserIdx].content;
    // Trim to before user msg, then send again
    setMessages(prev => prev.slice(0, lastUserIdx));
    await send(lastPrompt);
  };

  const reset = async () => {
    if (!confirm("清除這段對話？")) return;
    await invoke("reset_conversation", { chat_id: CHAT_ID }).catch(() => {});
    setMessages([]);
    setError(null);
  };

  const showWelcome = messages.length === 0;
  const lastIsAssistant = messages[messages.length - 1]?.role === "assistant" && !loading;

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2.5 border-b border-spectyn-border flex-shrink-0">
        <div className="flex items-center gap-2">
          <div className={`w-7 h-7 rounded-full flex items-center justify-center text-spectyn-bg ${
            cluster.enabled
              ? "bg-gradient-to-br from-spectyn-success to-spectyn-primary"
              : "bg-gradient-to-br from-spectyn-primary to-spectyn-secondary"
          }`}>
            {cluster.enabled ? <Network size={14} /> : <Sparkles size={14} />}
          </div>
          <div>
            <h1 className="text-sm font-semibold text-spectyn-text leading-tight">
              {providerMode
                ? "Spectyn Provider"
                : cluster.enabled
                ? "Spectyn Cluster"
                : "Spectyn"}
            </h1>
            <p className="text-[10px] text-spectyn-muted leading-tight">
              {providerMode
                ? `Provider 直連 · ${activeProvider ?? "解析中…"}`
                : cluster.enabled
                ? (cluster.isConfigured() ? "經由協調者派送" : "尚未設定")
                : "本機"}
            </p>
          </div>
        </div>
        <div className="flex items-center gap-1">
          {/* SPEC-14 provider-direct toggle (bypasses agent + cluster). */}
          <button
            onClick={() => setProviderMode(m => !m)}
            disabled={loading}
            aria-label={providerMode ? "關閉 Provider 直連模式" : "開啟 Provider 直連模式"}
            title={providerMode
              ? `Provider 直連（${activeProvider ?? "解析中"}）`
              : "Provider 直連模式（跳過 agent loop）"}
            className={`p-1.5 -m-1 rounded transition disabled:opacity-40 ${
              providerMode ? "text-spectyn-primary" : "text-spectyn-muted hover:text-spectyn-text"
            }`}
          >
            <Zap size={16} />
          </button>
          {/* Cluster toggle */}
          <button
            onClick={() => cluster.setEnabled(!cluster.enabled)}
            disabled={!cluster.isConfigured() || providerMode}
            className={`relative w-11 h-6 rounded-full transition flex-shrink-0 ${
              cluster.enabled ? "bg-spectyn-success" : "bg-spectyn-card border border-spectyn-border"
            } ${!cluster.isConfigured() || providerMode ? "opacity-40" : ""}`}
            aria-label={cluster.enabled ? "關閉 cluster 模式" : "開啟 cluster 模式"}
            title={providerMode
              ? "Provider 直連模式啟用中，已停用 Cluster 切換"
              : cluster.isConfigured()
              ? (cluster.enabled ? "Cluster 模式 (本機)" : "本機 (Cluster 模式)")
              : "請先到設定 → cluster 配置 coordinator URL 跟 secret"}
          >
            <div className={`absolute top-0.5 w-5 h-5 rounded-full bg-white transition-transform ${
              cluster.enabled ? "translate-x-5" : "translate-x-0.5"
            }`} />
          </button>
          {messages.length > 0 && (
            <button
              onClick={reset}
              className="text-spectyn-muted hover:text-spectyn-danger p-2 -m-1"
              aria-label="清除"
            >
              <Trash2 size={18} />
            </button>
          )}
        </div>
      </div>

      {/* Messages */}
      <div className="flex-1 overflow-y-auto px-3 py-3">
        {showWelcome && (
          <div className="flex flex-col items-center justify-center min-h-full px-2 text-center pb-8">
            <div className="w-14 h-14 rounded-2xl bg-gradient-to-br from-spectyn-primary to-spectyn-secondary flex items-center justify-center mb-3 text-spectyn-bg">
              <Sparkles size={26} />
            </div>
            <h2 className="text-xl font-semibold text-spectyn-text mb-1">嗨，我是 Spectyn</h2>
            <p className="text-sm text-spectyn-muted mb-6">問什麼都可以</p>

            <div className="w-full space-y-3">
              {PROMPT_CATEGORIES.map((cat) => (
                <div key={cat.title}>
                  <div className="flex items-center gap-1.5 px-1 mb-1.5">
                    <span className="text-base">{cat.icon}</span>
                    <span className="text-xs font-semibold text-spectyn-muted uppercase tracking-wide">
                      {cat.title}
                    </span>
                  </div>
                  <div className="space-y-1.5">
                    {cat.prompts.map((p) => (
                      <button
                        key={p}
                        onClick={() => send(p)}
                        className="w-full text-left px-3.5 py-2.5 bg-spectyn-card border border-spectyn-border rounded-xl text-[13.5px] text-spectyn-text hover:border-spectyn-primary/60 transition active:scale-[0.99]"
                      >
                        {p}
                      </button>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        {!showWelcome && messages.map((msg, i) => {
          const isEmptyAssistantPlaceholder =
            msg.role === "assistant" &&
            msg.content === "" &&
            !msg.tool_calls?.length &&
            i === messages.length - 1 &&
            loading;
          if (isEmptyAssistantPlaceholder) return null;

          if (msg.role === "user") {
            return (
              <div key={i} className="flex justify-end mb-3">
                <div className="max-w-[85%] px-3.5 py-2.5 rounded-2xl rounded-br-md bg-spectyn-primary text-spectyn-bg">
                  <p className="text-[14px] whitespace-pre-wrap leading-relaxed">{msg.content}</p>
                </div>
              </div>
            );
          }

          return (
            <div key={i} className="flex gap-2 mb-3">
              <div className="flex-shrink-0 w-7 h-7 rounded-full bg-gradient-to-br from-spectyn-primary to-spectyn-secondary flex items-center justify-center text-spectyn-bg mt-0.5">
                <Sparkles size={13} />
              </div>
              <div className="flex-1 min-w-0">
                {msg.content && (
                  <div className="text-spectyn-text text-[14px]">
                    <Markdown>{msg.content}</Markdown>
                  </div>
                )}
                {msg.tool_calls && msg.tool_calls.length > 0 && (
                  <ToolCallDisplay toolCalls={msg.tool_calls} />
                )}
                {(msg.provider || msg.model) && (
                  <p className="mt-1 text-[10px] text-spectyn-muted font-mono select-none">
                    {msg.provider}
                    {msg.provider && msg.model ? " · " : ""}
                    {msg.model}
                  </p>
                )}
                {msg.content && !loading && (
                  <div className="mt-1.5 flex items-center gap-3">
                    <CopyButton text={msg.content} />
                    {i === messages.length - 1 && (
                      <button
                        onClick={regenerate}
                        className="text-spectyn-muted hover:text-spectyn-text transition flex items-center gap-1 text-[11px]"
                      >
                        <RotateCcw size={13} />
                        重新回答
                      </button>
                    )}
                  </div>
                )}
              </div>
            </div>
          );
        })}

        {loading && (
          <div className="flex gap-2 mb-3">
            <div className="flex-shrink-0 w-7 h-7 rounded-full bg-gradient-to-br from-spectyn-primary to-spectyn-secondary flex items-center justify-center text-spectyn-bg mt-0.5">
              <Sparkles size={13} />
            </div>
            <div className="flex-1">
              <div className="inline-flex items-center gap-1.5 px-3 py-2 rounded-2xl bg-spectyn-card border border-spectyn-border">
                <span className="text-sm text-spectyn-muted">Thinking</span>
                {[0, 1, 2].map(i => (
                  <span
                    key={i}
                    className="inline-block w-1.5 h-1.5 bg-spectyn-primary rounded-full animate-bounce"
                    style={{ animationDelay: `${i * 0.18}s`, animationDuration: "0.9s" }}
                  />
                ))}
              </div>
            </div>
          </div>
        )}

        {error && (
          <button
            onClick={() => {
              if (!cluster.isConfigured() || !cluster.enabled) {
                navigate("/settings/cluster");
              } else {
                setError(null);
              }
            }}
            className="w-full bg-spectyn-danger/15 border border-spectyn-danger/40 text-spectyn-danger px-3 py-2 rounded-lg text-sm mb-2 text-left active:bg-spectyn-danger/25 transition"
          >
            {error}
          </button>
        )}

        <div ref={bottomRef} />
      </div>

      {/* Input bar */}
      <div
        className="flex-shrink-0 border-t border-spectyn-border p-2.5 bg-spectyn-bg"
        style={{ paddingBottom: "calc(0.625rem + env(safe-area-inset-bottom))" }}
      >
        <div className="flex gap-2 items-end">
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                send();
              }
            }}
            placeholder={loading ? "回應中…" : "輸入訊息..."}
            rows={1}
            disabled={loading}
            style={{ fontSize: "16px", maxHeight: "120px" }}
            className="flex-1 bg-spectyn-card border border-spectyn-border rounded-2xl px-4 py-2.5 text-spectyn-text placeholder-spectyn-muted focus:outline-none focus:border-spectyn-primary transition resize-none disabled:opacity-60"
          />
          {loading ? (
            <button
              onClick={stop}
              className="bg-spectyn-danger text-white w-11 h-11 rounded-full flex items-center justify-center transition flex-shrink-0 active:scale-95"
              aria-label="停止"
            >
              <Square size={16} fill="currentColor" />
            </button>
          ) : (
            <button
              onClick={() => send()}
              disabled={input.trim() === ""}
              className="bg-spectyn-primary text-spectyn-bg w-11 h-11 rounded-full flex items-center justify-center disabled:opacity-40 transition flex-shrink-0 active:scale-95"
              aria-label="發送"
            >
              <Send size={18} />
            </button>
          )}
        </div>
        {/* Regenerate hint when last message is assistant */}
        {lastIsAssistant && messages.length > 0 && !showWelcome && (
          <div className="text-[10.5px] text-spectyn-muted text-center mt-1.5">
            按 Enter 送出 · Shift+Enter 換行
          </div>
        )}
      </div>
    </div>
  );
}
