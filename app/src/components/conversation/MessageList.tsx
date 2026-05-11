import { useEffect, useRef } from "react";
import type { Message } from "../../lib/types";
import ToolCallDisplay from "./ToolCallDisplay";

interface MessageListProps {
  messages: Message[];
  loading: boolean;
}

// ─── Animated typing indicator ────────────────────────────────────────────────
function TypingIndicator() {
  return (
    <div className="bg-phantom-card border border-phantom-border p-3 rounded-lg max-w-[80%]">
      <div className="flex items-center gap-1.5">
        <span className="text-sm text-phantom-muted mr-1">Thinking</span>
        {[0, 1, 2].map((i) => (
          <span
            key={i}
            className="inline-block w-1.5 h-1.5 bg-phantom-primary rounded-full animate-bounce"
            style={{ animationDelay: `${i * 0.18}s`, animationDuration: "0.9s" }}
          />
        ))}
      </div>
    </div>
  );
}

export default function MessageList({ messages, loading }: MessageListProps) {
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, loading]);

  // Determine whether the last assistant message is still streaming (empty content + loading)
  const lastMsg = messages[messages.length - 1];
  const isStreamingPlaceholder =
    loading && lastMsg?.role === "assistant" && lastMsg.content === "" && !lastMsg.tool_calls?.length;

  return (
    <>
      {messages.map((msg, i) => {
        // Skip the empty streaming placeholder — replaced by TypingIndicator below
        const isEmptyPlaceholder =
          msg.role === "assistant" && msg.content === "" && !msg.tool_calls?.length && i === messages.length - 1 && loading;
        if (isEmptyPlaceholder) return null;

        return (
          <div
            key={i}
            className={`p-3 rounded-lg max-w-[80%] ${
              msg.role === "user"
                ? "bg-phantom-primary/20 ml-auto"
                : "bg-phantom-card border border-phantom-border"
            }`}
          >
            {msg.content && (
              <p className="text-sm whitespace-pre-wrap">{msg.content}</p>
            )}

            {msg.tool_calls && msg.tool_calls.length > 0 && (
              <ToolCallDisplay toolCalls={msg.tool_calls} />
            )}
          </div>
        );
      })}

      {/* Show typing indicator while waiting / streaming with no content yet */}
      {isStreamingPlaceholder && <TypingIndicator />}

      <div ref={bottomRef} />
    </>
  );
}
