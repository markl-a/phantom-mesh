import { useState } from "react";
import { Globe, X } from "lucide-react";

interface Props {
  currentUrl: string;
  onNavigate: (url: string) => void;
  onClose: () => void;
  loading: boolean;
}

export default function UrlBar({ currentUrl, onNavigate, onClose, loading }: Props) {
  const [input, setInput] = useState(currentUrl);

  const handleGo = () => {
    let url = input.trim();
    if (!url) return;
    if (!url.startsWith("http://") && !url.startsWith("https://")) {
      url = "https://" + url;
    }
    onNavigate(url);
  };

  return (
    <div className="flex items-center gap-2 mb-4">
      <Globe size={16} className="text-spectyn-muted flex-shrink-0" />
      <input
        type="text"
        value={input}
        onChange={(e) => setInput(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && handleGo()}
        placeholder="輸入網址..."
        className="flex-1 bg-spectyn-card border border-spectyn-border rounded px-3 py-1.5 text-sm text-spectyn-text placeholder-spectyn-muted focus:outline-none focus:border-spectyn-primary"
      />
      <button
        onClick={handleGo}
        disabled={loading || !input.trim()}
        className="bg-spectyn-primary text-spectyn-bg px-3 py-1.5 rounded text-sm font-medium hover:brightness-110 disabled:opacity-40"
      >
        {loading ? "載入中..." : "Go"}
      </button>
      <button
        onClick={onClose}
        className="text-spectyn-muted hover:text-spectyn-danger p-1.5 rounded hover:bg-spectyn-card"
        title="關閉瀏覽器"
      >
        <X size={16} />
      </button>
    </div>
  );
}
