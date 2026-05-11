import { useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";

export interface BrowserAction {
  time: string;
  action: string;
  detail?: string;
}

interface Props {
  actions: BrowserAction[];
  pageText: string | null;
}

export default function ActionLog({ actions, pageText }: Props) {
  const [textOpen, setTextOpen] = useState(false);

  return (
    <div className="flex flex-col h-full">
      <div className="flex-1 overflow-y-auto">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-phantom-muted mb-2">操作歷史</h3>
        {actions.length === 0 ? (
          <p className="text-xs text-phantom-muted">尚無操作</p>
        ) : (
          <div className="space-y-1">
            {actions.map((a, i) => (
              <div key={i} className="text-xs flex gap-2">
                <span className="text-phantom-muted flex-shrink-0">{a.time}</span>
                <span className="text-phantom-text">
                  <span className="font-medium">{a.action}</span>
                  {a.detail && <span className="text-phantom-muted ml-1">{a.detail}</span>}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
      {pageText && (
        <div className="border-t border-phantom-border pt-2 mt-2">
          <button
            onClick={() => setTextOpen(!textOpen)}
            className="flex items-center gap-1 text-xs text-phantom-muted hover:text-phantom-text w-full"
          >
            {textOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
            頁面文字
          </button>
          {textOpen && (
            <pre className="mt-1 text-xs text-phantom-muted bg-phantom-bg rounded p-2 max-h-48 overflow-y-auto whitespace-pre-wrap">
              {pageText}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}
