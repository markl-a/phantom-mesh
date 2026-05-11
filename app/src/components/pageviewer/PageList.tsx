import { Trash2 } from "lucide-react";

export interface PageInfo {
  name: string;
  description: string;
  created_at: string;
}

interface Props {
  pages: PageInfo[];
  selectedPage: string | null;
  onSelect: (name: string) => void;
  onDelete: (name: string) => void;
}

export default function PageList({ pages, selectedPage, onSelect, onDelete }: Props) {
  if (pages.length === 0) {
    return (
      <div className="text-center py-8">
        <p className="text-sm text-phantom-muted">尚無頁面</p>
        <p className="text-xs text-phantom-muted mt-1">在對話頁請 Agent 生成</p>
      </div>
    );
  }

  return (
    <div className="space-y-1">
      {pages.map((page) => (
        <div
          key={page.name}
          className={`flex items-center justify-between rounded px-2 py-1.5 cursor-pointer text-sm transition-colors ${
            selectedPage === page.name
              ? "bg-phantom-primary/15 text-phantom-primary"
              : "text-phantom-text hover:bg-phantom-card"
          }`}
          onClick={() => onSelect(page.name)}
        >
          <div className="min-w-0 flex-1">
            <p className="truncate font-medium">{page.name}</p>
            {page.description && (
              <p className="text-xs text-phantom-muted truncate">{page.description}</p>
            )}
          </div>
          <button
            onClick={(e) => { e.stopPropagation(); onDelete(page.name); }}
            className="text-phantom-muted hover:text-phantom-danger p-1 flex-shrink-0"
            title="刪除"
          >
            <Trash2 size={12} />
          </button>
        </div>
      ))}
    </div>
  );
}
