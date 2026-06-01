import { useState, useEffect, useRef } from "react";
import { safeInvoke as invoke, isTauri } from "../../lib/tauri-compat";
import { Play, RefreshCw, Trash2, Square, Zap } from "lucide-react";
import type { Message, AgentEvent, DaemonInfo, ToolCall } from "../../lib/types";
import MessageList from "./MessageList";
import MessageInput from "./MessageInput";
import ConversationSelector from "./ConversationSelector";
import {
  selectProvider,
  streamComplete,
  buildRequest,
  describeError,
  type ProviderMessage,
  type StreamHandle,
} from "../../lib/providers";

// Safe listen: no-op in browser mode
async function safeListen(event: string, handler: (e: { payload: unknown }) => void) {
  if (isTauri()) {
    const { listen } = await import("@tauri-apps/api/event");
    return listen(event, handler);
  }
  return { unsubscribe: () => {} } as { unsubscribe: () => void };
}

// ─── Goal-oriented welcome grid ───────────────────────────────────────────────
const GOAL_EXAMPLES = [
  {
    emoji: "🎓",
    title: "學業與考試",
    goals: [
      { label: "考上理想學校", prompt: "我的目標是考上台大，請幫我制定讀書計畫、定期追蹤進度、整理考試重點" },
      { label: "學好英文", prompt: "我想在半年內把英文提升到流利程度，請幫我制定每日學習計畫並定期測驗" },
      { label: "考取證照", prompt: "我想考取專業證照，請幫我規劃備考時程、整理重點、模擬測驗" },
    ],
  },
  {
    emoji: "💰",
    title: "事業與財務",
    goals: [
      { label: "增加收入", prompt: "我想增加收入來源，請幫我分析我的技能、研究市場機會、制定副業計畫" },
      { label: "創業開店", prompt: "我想創業，請幫我做市場調查、撰寫商業計畫、規劃執行步驟" },
      { label: "投資理財", prompt: "我想開始學習投資理財，請幫我整理入門知識、追蹤市場動態、建立學習計畫" },
    ],
  },
  {
    emoji: "💪",
    title: "健康與生活",
    goals: [
      { label: "減重塑身", prompt: "我想健康減重，請幫我制定飲食計畫、運動菜單、每週追蹤體重變化" },
      { label: "養成好習慣", prompt: "我想養成早睡早起和運動的習慣，請幫我設計 21 天習慣養成計畫並每天提醒" },
      { label: "學做料理", prompt: "我想學會做菜，請根據我的程度每週推薦食譜、購物清單、教學步驟" },
    ],
  },
  {
    emoji: "🚀",
    title: "技能與成長",
    goals: [
      { label: "學寫程式", prompt: "我是零基礎，想學寫程式，請幫我規劃從入門到實作的學習路線並每天出練習題" },
      { label: "經營自媒體", prompt: "我想經營 YouTube / 部落格，請幫我規劃內容策略、定期產出文案、分析數據" },
      { label: "學新語言", prompt: "我想學日文，請幫我制定每日學習計畫、單字複習、會話練習" },
    ],
  },
];

interface StatusInfo {
  daemonHealthy: boolean | null;
  providerCount: number;
  nodeCount: number;
}

const DEFAULT_CHAT_ID = "daemon";

export default function ConversationView() {
  const [chatId, setChatId] = useState<string>(DEFAULT_CHAT_ID);
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<StatusInfo>({
    daemonHealthy: null, providerCount: 0, nodeCount: 0,
  });
  const [starting, setStarting] = useState(false);
  const [goalSet, setGoalSet] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [showResetConfirm, setShowResetConfirm] = useState(false);

  // ── SPEC-14 providers wire path (raw mode) ───────────────────────────────────
  // When `providerMode` is true, send() bypasses the agent send_message
  // endpoint and routes directly through `providers_complete_streaming`,
  // giving the user un-filtered access to whatever provider the SPEC-14
  // resolver picks for ("commodity", "interactive").
  const [providerMode, setProviderMode] = useState(false);
  const [activeProvider, setActiveProvider] = useState<string | null>(null);
  const streamRef = useRef<StreamHandle | null>(null);

  // ── Load history whenever chatId changes ─────────────────────────────────────
  const loadHistory = (id: string) => {
    invoke<{ messages?: Message[] }>("get_conversation_history", { chat_id: id })
      .then((data) => {
        setStatus(s => ({ ...s, daemonHealthy: true }));
        setMessages(data.messages && data.messages.length > 0 ? data.messages : []);
        setGoalSet((data.messages?.length ?? 0) > 0);
      })
      .catch(() => {
        // Fall back to legacy endpoint on first load for "daemon"
        if (id === DEFAULT_CHAT_ID) {
          invoke<{ messages?: Message[] }>("get_conversations")
            .then((data) => {
              setStatus(s => ({ ...s, daemonHealthy: true }));
              if (data.messages && data.messages.length > 0) {
                setMessages(data.messages);
                setGoalSet(true);
              }
            })
            .catch(() => {
              invoke<DaemonInfo>("daemon_status")
                .then(info => setStatus(s => ({ ...s, daemonHealthy: info.healthy })))
                .catch(() => setStatus(s => ({ ...s, daemonHealthy: false })));
            });
        }
      });
  };

  useEffect(() => {
    setMessages([]);
    setGoalSet(false);
    setError(null);
    loadHistory(chatId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chatId]);

  useEffect(() => {
    invoke<{ nodes: unknown[] }>("get_cluster_status")
      .then(res => setStatus(s => ({ ...s, nodeCount: res.nodes.length })))
      .catch(() => {});
    invoke<{ providers: unknown[] }>("get_provider_health")
      .then(res => setStatus(s => ({ ...s, providerCount: res.providers.length })))
      .catch(() => {});
  }, []);

  // ── Start daemon ─────────────────────────────────────────────────────────────
  const startDaemon = async () => {
    setStarting(true);
    setError(null);
    try {
      await invoke("start_daemon");
      const info = await invoke<DaemonInfo>("daemon_status");
      setStatus(s => ({ ...s, daemonHealthy: info.healthy }));
    } catch (e) {
      setError(String(e));
    } finally {
      setStarting(false);
    }
  };

  // ── Reset conversation ────────────────────────────────────────────────────────
  const resetConversation = async () => {
    setShowResetConfirm(false);
    setResetting(true);
    try {
      await invoke("reset_conversation", { chat_id: chatId });
      setMessages([]);
      setGoalSet(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setResetting(false);
    }
  };

  // ── Provider mode: resolve a slug when toggled on ────────────────────────────
  // Run once on toggle so the header label updates even before the first
  // message. Failures degrade gracefully — we still let the user try to send,
  // because the resolver re-runs server-side per request anyway.
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

  // ── Send via SPEC-14 providers wire ──────────────────────────────────────────
  // Builds a `ProviderRequest` from the current message history, picks a
  // provider via `select_provider`, then streams the response through the
  // `providers_complete_event` event bus. Cancellation: we keep the
  // StreamHandle in a ref so the user's Stop button can detach the listener.
  const sendViaProvider = async (msg: string) => {
    const priorMessages = messages.filter(m => !(m.role === "assistant" && m.content === ""));

    // Build the ProviderRequest message array — system prompt is hoisted so
    // adapters (e.g. Anthropic) can place it out-of-band per SPEC-14 §7.1.
    const providerMessages: ProviderMessage[] = [
      ...priorMessages.map(m => ({ role: m.role, content: m.content, images: [] })),
      { role: "user" as const, content: msg, images: [] },
    ];

    let resolvedProvider = activeProvider;
    try {
      if (!resolvedProvider) {
        resolvedProvider = await selectProvider("commodity", "interactive");
        setActiveProvider(resolvedProvider);
      }
    } catch (e) {
      setError(`Provider 解析失敗：${describeError(String(e))}`);
      setLoading(false);
      setMessages(prev => prev.slice(0, -1));
      return;
    }

    // The wire requires a non-empty `model`. We let the resolver pick the
    // default by passing the provider slug — the backend reads
    // `default_model` from agents.toml. If that round-trips empty we surface
    // the upstream `provider.model_not_found` error rather than guessing.
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
            // The backend currently emits the whole text in one `done` event,
            // so this is effectively a replace. Once core grows real per-token
            // streaming this naturally becomes append.
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

  // ── Stop / cancel in-flight stream (provider mode) ───────────────────────────
  const stop = () => {
    streamRef.current?.cancel();
    streamRef.current = null;
    setLoading(false);
    setMessages(prev => {
      const updated = [...prev];
      const idx = updated.length - 1;
      if (updated[idx]?.role === "assistant" && !updated[idx].content) {
        updated[idx] = { ...updated[idx], content: "_(已中斷)_" };
      }
      return updated;
    });
  };

  // ── Send message ──────────────────────────────────────────────────────────────
  const send = async (text?: string | unknown) => {
    const msg = (typeof text === "string" ? text : input).trim();
    if (!msg || loading) return;

    setInput("");
    setError(null);
    setGoalSet(true);

    setMessages(prev => [...prev, { role: "user", content: msg }]);
    setLoading(true);
    setMessages(prev => [...prev, { role: "assistant", content: "", tool_calls: [] }]);

    // Branch: raw provider mode skips the agent loop entirely.
    if (providerMode) {
      try {
        await sendViaProvider(msg);
      } catch (e) {
        setError(describeError(String(e)));
        setMessages(prev => prev.slice(0, -1));
        setLoading(false);
      }
      return;
    }

    let streamReceived = false;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    let unlistenFn: any = null;

    try {
      unlistenFn = await safeListen("agent_event", (event: { payload: unknown }) => {
        const ev = event.payload as AgentEvent;
        streamReceived = true;

        if (ev.type === "chunk" && ev.content) {
          const chunk = ev.content;
          setMessages(prev => {
            const updated = [...prev];
            const idx = updated.length - 1;
            if (updated[idx]?.role === "assistant") {
              updated[idx] = { ...updated[idx], content: updated[idx].content + chunk };
            }
            return updated;
          });
        } else if ((ev.type === "tool_call" || ev.type === "tool_result") && ev.tool_call) {
          const tc: ToolCall = ev.tool_call;
          setMessages(prev => {
            const updated = [...prev];
            const idx = updated.length - 1;
            if (updated[idx]?.role === "assistant") {
              const existing = updated[idx].tool_calls ?? [];
              const existingIdx = existing.findIndex(t => t.name === tc.name);
              const newToolCalls =
                existingIdx >= 0
                  ? existing.map((t, i) => (i === existingIdx ? tc : t))
                  : [...existing, tc];
              updated[idx] = { ...updated[idx], tool_calls: newToolCalls };
            }
            return updated;
          });
        } else if (ev.type === "done") {
          setLoading(false);
          setStatus(s => ({ ...s, daemonHealthy: true }));
          if (typeof unlistenFn === "function") unlistenFn();
          else unlistenFn?.unsubscribe?.();
        } else if (ev.type === "error") {
          setError("Agent 出錯，請重試");
          setLoading(false);
          if (typeof unlistenFn === "function") unlistenFn();
          else unlistenFn?.unsubscribe?.();
        }
      });

      const response = await invoke<Record<string, unknown>>("send_message", {
        prompt: msg,
        chat_id: chatId,
      });

      if (!streamReceived) {
        let content: string;
        let toolCalls: ToolCall[] = [];
        if (typeof response === "string") {
          content = response;
        } else if (response && typeof response === "object") {
          content = String(
            response.output ??
            response["result"] ??
            response["message"] ??
            response["content"] ??
            JSON.stringify(response, null, 2)
          );
          const rawTc = response["tool_calls"];
          if (Array.isArray(rawTc)) {
            toolCalls = rawTc.map((tc: Record<string, unknown>) => ({
              name: String(tc["name"] ?? ""),
              args: (tc["args"] ?? {}) as Record<string, unknown>,
              result: String(tc["result"] ?? ""),
              status: "done" as const,
            }));
          }
        } else {
          content = String(response);
        }

        setMessages(prev => {
          const updated = [...prev];
          const idx = updated.length - 1;
          if (updated[idx]?.role === "assistant") {
            updated[idx] = { ...updated[idx], content, tool_calls: toolCalls };
          }
          return updated;
        });
        setStatus(s => ({ ...s, daemonHealthy: true }));
        setLoading(false);
      }
    } catch (e) {
      const errStr = String(e);
      setError(errStr);
      if (errStr.includes("connection") || errStr.includes("Connection") || errStr.includes("refused")) {
        setStatus(s => ({ ...s, daemonHealthy: false }));
      }
      setMessages(prev => {
        const last = prev[prev.length - 1];
        if (last?.role === "assistant" && last.content === "") {
          return prev.slice(0, -1);
        }
        return prev;
      });
      setLoading(false);
    } finally {
      if (!streamReceived) {
        if (typeof unlistenFn === "function") unlistenFn();
        else unlistenFn?.unsubscribe?.();
      }
    }
  };

  // ── Render ────────────────────────────────────────────────────────────────────
  return (
    <div className="flex flex-col h-full">
      {/* Header row: status + session selector + reset */}
      <div className="flex items-center gap-3 mb-3 px-1 flex-wrap">
        {/* Status dot */}
        <div className="flex items-center gap-1.5 text-xs text-phantom-muted">
          <span className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${
            status.daemonHealthy === true  ? "bg-phantom-success" :
            status.daemonHealthy === false ? "bg-phantom-danger"  : "bg-phantom-muted"
          }`} />
          <span>
            {status.daemonHealthy === true  ? "Runtime 運行中" :
             status.daemonHealthy === false ? "Runtime 離線"   : "檢查中..."}
          </span>
          {status.providerCount > 0 && <span>· {status.providerCount} Providers</span>}
          {status.nodeCount > 0 && <span>· {status.nodeCount} 節點</span>}
        </div>

        {/* Spacer */}
        <div className="flex-1" />

        {/* SPEC-14 provider mode toggle — bypasses agent loop, hits providers wire directly */}
        <button
          onClick={() => setProviderMode(m => !m)}
          disabled={loading}
          title={providerMode
            ? `Provider 直連模式（${activeProvider ?? "解析中"}）— 點擊切回 Agent`
            : "切換到 Provider 直連模式（跳過 agent loop）"}
          className={`flex items-center gap-1.5 px-2 py-1 rounded text-[11px] border transition disabled:opacity-40 ${
            providerMode
              ? "bg-phantom-primary/15 border-phantom-primary/40 text-phantom-primary"
              : "border-phantom-border text-phantom-muted hover:text-phantom-text"
          }`}
        >
          <Zap size={12} />
          {providerMode ? `Provider: ${activeProvider ?? "…"}` : "Provider"}
        </button>

        {/* Conversation session selector */}
        <ConversationSelector activeChatId={chatId} onSelect={setChatId} />

        {/* Stop button (only when streaming in provider mode) */}
        {providerMode && loading && (
          <button
            onClick={stop}
            title="中斷目前的回應"
            className="p-1.5 text-phantom-danger hover:opacity-80 transition rounded"
          >
            <Square size={14} fill="currentColor" />
          </button>
        )}

        {/* Reset button */}
        <button
          onClick={() => setShowResetConfirm(true)}
          disabled={resetting || loading}
          title="Reset conversation"
          className="p-1.5 text-phantom-muted hover:text-phantom-danger transition disabled:opacity-40 rounded"
        >
          {resetting ? (
            <RefreshCw size={14} className="animate-spin" />
          ) : (
            <Trash2 size={14} />
          )}
        </button>
      </div>

      {/* Reset confirm dialog */}
      {showResetConfirm && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="bg-phantom-card border border-phantom-border rounded-lg p-5 w-72 shadow-xl">
            <p className="text-sm font-medium text-phantom-text mb-2">Reset conversation?</p>
            <p className="text-xs text-phantom-muted mb-4">
              This will clear all messages in <span className="font-mono text-phantom-primary">{chatId}</span>.
              This action cannot be undone.
            </p>
            <div className="flex gap-2 justify-end">
              <button
                onClick={() => setShowResetConfirm(false)}
                className="px-3 py-1.5 text-xs text-phantom-muted hover:text-phantom-text border border-phantom-border rounded transition"
              >
                Cancel
              </button>
              <button
                onClick={resetConversation}
                className="px-3 py-1.5 text-xs bg-phantom-danger text-white rounded hover:opacity-90 transition"
              >
                Reset
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Daemon not running banner */}
      {status.daemonHealthy === false && (
        <div className="bg-amber-500/10 border border-amber-500/30 rounded-lg p-4 mb-4 flex items-center justify-between">
          <div>
            <p className="text-sm font-medium text-amber-300">Daemon 未運行</p>
            <p className="text-xs text-phantom-muted">需要啟動 Daemon 才能對話</p>
          </div>
          <button
            onClick={startDaemon}
            disabled={starting}
            className="flex items-center gap-2 bg-phantom-primary text-phantom-bg px-4 py-2 rounded text-sm font-medium hover:opacity-90 disabled:opacity-50"
          >
            {starting ? (
              <><RefreshCw size={14} className="animate-spin" /> 啟動中...</>
            ) : (
              <><Play size={14} /> 啟動 Daemon</>
            )}
          </button>
        </div>
      )}

      {/* Error banner */}
      {error && (
        <div className="bg-phantom-danger/20 border border-phantom-danger rounded p-3 mb-4 text-sm">
          {error}
        </div>
      )}

      {/* Chat area */}
      <div className="flex-1 overflow-y-auto space-y-3 mb-2 pb-4">

        {/* Welcome goal-oriented onboarding */}
        {!goalSet && (
          <div className="bg-phantom-card border border-phantom-border p-5 rounded-lg max-w-[90%]">
            <p className="text-base text-phantom-text leading-relaxed">
              你好 Master，我是你的 <span className="text-phantom-primary font-semibold">Phantom</span>。
            </p>
            <p className="text-sm text-phantom-text leading-relaxed mt-2">
              我是你的影舞者 — 在幕後持續為你工作的 AI 幕僚。
              不只是回答問題，我會幫你<span className="text-phantom-primary">制定計畫、追蹤進度、定時執行任務</span>，
              直到你達成目標為止。
            </p>

            <div className="border-t border-phantom-border my-4" />

            <p className="text-sm text-phantom-text font-medium mb-4">
              告訴我，你現在最想達成的目標是什麼？
            </p>

            <div className="grid grid-cols-2 gap-3">
              {GOAL_EXAMPLES.map((cat, ci) => (
                <div key={ci} className="space-y-1.5">
                  <p className="text-xs text-phantom-muted">
                    <span className="mr-1">{cat.emoji}</span>{cat.title}
                  </p>
                  {cat.goals.map((g, gi) => (
                    <button
                      key={gi}
                      onClick={() => send(g.prompt)}
                      disabled={loading}
                      className="w-full text-left bg-phantom-bg border border-phantom-border rounded-lg px-3 py-2 text-xs text-phantom-text
                                 hover:border-phantom-primary/50 hover:text-phantom-primary transition disabled:opacity-50"
                    >
                      {g.label}
                    </button>
                  ))}
                </div>
              ))}
            </div>

            <p className="text-xs text-phantom-muted mt-4">
              以上只是範例 — 你也可以直接輸入你自己的目標，無論大小我都會認真對待。
            </p>
          </div>
        )}

        <MessageList messages={messages} loading={loading} />
      </div>

      {/* Input bar */}
      <MessageInput
        input={input}
        setInput={setInput}
        onSend={send}
        loading={loading}
        placeholder={goalSet ? "繼續對話..." : "輸入你的目標，例如「我想半年內存到 30 萬」"}
      />
    </div>
  );
}
