import { useState, useEffect, useRef } from "react";
import { ChevronDown, Plus, MessageSquare, Check } from "lucide-react";
import { safeInvoke as invoke } from "../../lib/tauri-compat";
import type { ConversationInfo } from "../../lib/types";

interface ConversationSelectorProps {
  activeChatId: string;
  onSelect: (chatId: string) => void;
}

export default function ConversationSelector({ activeChatId, onSelect }: ConversationSelectorProps) {
  const [open, setOpen] = useState(false);
  const [conversations, setConversations] = useState<ConversationInfo[]>([]);
  const [newIdInput, setNewIdInput] = useState("");
  const [showNewInput, setShowNewInput] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  const fetchConversations = () => {
    invoke<{ conversations?: ConversationInfo[] | string[] }>("list_conversations")
      .then((data) => {
        const raw = data?.conversations ?? [];
        const parsed: ConversationInfo[] = raw.map((item) =>
          typeof item === "string"
            ? { id: item }
            : (item as ConversationInfo)
        );
        // Ensure active chat is always in the list
        if (!parsed.find((c) => c.id === activeChatId)) {
          parsed.unshift({ id: activeChatId });
        }
        setConversations(parsed);
      })
      .catch(() => {
        setConversations([{ id: activeChatId }]);
      });
  };

  useEffect(() => {
    fetchConversations();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Close on outside click
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setOpen(false);
        setShowNewInput(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, []);

  const handleSelect = (id: string) => {
    onSelect(id);
    setOpen(false);
    setShowNewInput(false);
  };

  const handleCreate = () => {
    const id = newIdInput.trim();
    if (!id) return;
    if (!conversations.find((c) => c.id === id)) {
      setConversations((prev) => [{ id }, ...prev]);
    }
    onSelect(id);
    setNewIdInput("");
    setShowNewInput(false);
    setOpen(false);
  };

  const displayLabel = activeChatId.length > 16 ? activeChatId.slice(0, 14) + "…" : activeChatId;

  return (
    <div className="relative" ref={dropdownRef}>
      {/* Trigger button */}
      <button
        onClick={() => {
          if (!open) fetchConversations();
          setOpen((v) => !v);
        }}
        className="flex items-center gap-1.5 text-xs text-phantom-text bg-phantom-card border border-phantom-border rounded-lg px-2.5 py-1.5 hover:border-phantom-primary/50 transition max-w-[160px]"
        title={activeChatId}
      >
        <MessageSquare size={12} className="text-phantom-primary flex-shrink-0" />
        <span className="truncate font-mono">{displayLabel}</span>
        <ChevronDown size={12} className={`flex-shrink-0 transition-transform ${open ? "rotate-180" : ""}`} />
      </button>

      {/* Dropdown */}
      {open && (
        <div className="absolute top-full mt-1 left-0 z-50 w-60 bg-phantom-card border border-phantom-border rounded-lg shadow-lg overflow-hidden">
          {/* Existing conversations */}
          <div className="max-h-52 overflow-y-auto">
            {conversations.length === 0 ? (
              <p className="text-xs text-phantom-muted px-3 py-2">No conversations found</p>
            ) : (
              conversations.map((conv) => (
                <button
                  key={conv.id}
                  onClick={() => handleSelect(conv.id)}
                  className="w-full flex items-center gap-2 px-3 py-2 text-xs hover:bg-phantom-bg transition text-left"
                >
                  <Check
                    size={12}
                    className={conv.id === activeChatId ? "text-phantom-primary" : "invisible"}
                  />
                  <span className="font-mono text-phantom-text flex-1 truncate">{conv.id}</span>
                  {conv.message_count !== undefined && (
                    <span className="text-phantom-muted flex-shrink-0">{conv.message_count} msgs</span>
                  )}
                </button>
              ))
            )}
          </div>

          {/* Divider + New conversation */}
          <div className="border-t border-phantom-border">
            {showNewInput ? (
              <div className="flex items-center gap-1.5 px-2 py-1.5">
                <input
                  autoFocus
                  type="text"
                  value={newIdInput}
                  onChange={(e) => setNewIdInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleCreate();
                    if (e.key === "Escape") { setShowNewInput(false); setNewIdInput(""); }
                  }}
                  placeholder="conversation id..."
                  className="flex-1 bg-phantom-bg border border-phantom-border rounded px-2 py-1 text-xs text-phantom-text placeholder-phantom-muted focus:outline-none focus:border-phantom-primary"
                />
                <button
                  onClick={handleCreate}
                  disabled={!newIdInput.trim()}
                  className="text-phantom-primary text-xs font-medium disabled:opacity-40 hover:text-phantom-primary/80 transition"
                >
                  OK
                </button>
              </div>
            ) : (
              <button
                onClick={() => setShowNewInput(true)}
                className="w-full flex items-center gap-2 px-3 py-2 text-xs text-phantom-muted hover:text-phantom-text hover:bg-phantom-bg transition"
              >
                <Plus size={12} />
                New conversation
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
