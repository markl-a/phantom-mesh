import { useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { ImageOff, ZoomIn } from "lucide-react";

interface Props {
  screenshotPath: string | null;
  loading: boolean;
}

export default function ScreenshotView({ screenshotPath, loading }: Props) {
  const [zoomed, setZoomed] = useState(false);

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="w-6 h-6 border-2 border-phantom-primary border-t-transparent rounded-full animate-spin" />
        <span className="ml-3 text-phantom-muted text-sm">載入截圖...</span>
      </div>
    );
  }

  if (!screenshotPath) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-phantom-muted">
        <ImageOff size={48} className="opacity-30 mb-4" />
        <p className="text-sm">尚無截圖</p>
        <p className="text-xs mt-1">在上方輸入 URL 開始瀏覽，或在對話頁請 Agent 操作瀏覽器</p>
      </div>
    );
  }

  const imgSrc = convertFileSrc(screenshotPath);

  return (
    <div className="relative h-full overflow-hidden">
      <img
        src={imgSrc}
        alt="Browser screenshot"
        className={`rounded border border-phantom-border transition-transform cursor-pointer ${
          zoomed ? "scale-150 origin-top-left" : "w-full h-full object-contain"
        }`}
        onClick={() => setZoomed(!zoomed)}
      />
      <button
        onClick={() => setZoomed(!zoomed)}
        className="absolute top-2 right-2 bg-phantom-bg/80 p-1 rounded text-phantom-muted hover:text-phantom-text"
        title={zoomed ? "縮小" : "放大"}
      >
        <ZoomIn size={14} />
      </button>
    </div>
  );
}
