import { Send } from "lucide-react";

interface MessageInputProps {
  input: string;
  setInput: (value: string) => void;
  onSend: () => void;
  loading: boolean;
  placeholder?: string;
}

export default function MessageInput({
  input,
  setInput,
  onSend,
  loading,
  placeholder = "輸入訊息...",
}: MessageInputProps) {
  const disabled = loading || input.trim() === "";

  return (
    <div className="flex gap-2 sticky bottom-0 bg-spectyn-bg pt-2 pb-safe">
      <input
        type="text"
        value={input}
        onChange={(e) => setInput(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && !disabled && onSend()}
        placeholder={placeholder}
        disabled={loading}
        className="flex-1 bg-spectyn-card border border-spectyn-border rounded-lg px-4 py-2.5 text-spectyn-text placeholder-spectyn-muted focus:outline-none focus:border-spectyn-primary transition disabled:opacity-50"
        style={{ fontSize: "16px" }}
      />
      <button
        onClick={onSend}
        disabled={disabled}
        className="bg-spectyn-primary text-spectyn-bg px-4 py-2.5 rounded-lg text-sm font-medium hover:brightness-110 disabled:opacity-40 transition flex items-center gap-1.5 flex-shrink-0"
      >
        <Send size={14} />
        <span className="hidden sm:inline">發送</span>
      </button>
    </div>
  );
}
