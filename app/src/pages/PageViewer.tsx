import { useState, useEffect, useCallback } from "react";
import { safeInvoke as invoke } from "../lib/tauri-compat";
import { RefreshCw } from "lucide-react";
import PageList, { type PageInfo } from "../components/pageviewer/PageList";
import PageFrame from "../components/pageviewer/PageFrame";

interface LoadPageResult {
  html: string;
  name: string;
}

export default function PageViewer() {
  const [pages, setPages] = useState<PageInfo[]>([]);
  const [selectedPage, setSelectedPage] = useState<string | null>(null);
  const [html, setHtml] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const fetchPages = useCallback(async () => {
    try {
      const result = await invoke<PageInfo[]>("list_pages");
      // Guard: the browser/web fallback has no list_pages case and resolves
      // undefined, which would crash `pages.map`. Coerce to an array.
      setPages(Array.isArray(result) ? result : []);
    } catch {
      setPages([]);
    }
  }, []);

  useEffect(() => { void fetchPages(); }, [fetchPages]);

  const selectPage = async (name: string) => {
    setSelectedPage(name);
    setLoading(true);
    try {
      const result = await invoke<LoadPageResult>("load_page", { name });
      setHtml(result.html);
    } catch (e) {
      setHtml(`<html><body><p style="color:red">載入失敗: ${e}</p></body></html>`);
    } finally {
      setLoading(false);
    }
  };

  const deletePage = async (name: string) => {
    try {
      await invoke("delete_page", { name });
      if (selectedPage === name) {
        setSelectedPage(null);
        setHtml(null);
      }
      await fetchPages();
    } catch { /* ignore */ }
  };

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between mb-4">
        <h1 className="text-2xl font-bold">頁面</h1>
        <button
          onClick={() => void fetchPages()}
          className="flex items-center gap-2 border border-spectyn-border text-spectyn-muted px-3 py-1.5 rounded text-sm hover:text-spectyn-text"
        >
          <RefreshCw size={14} />
          重新整理
        </button>
      </div>

      <div className="flex gap-4 flex-1 min-h-0">
        <div className="w-48 flex-shrink-0 overflow-y-auto">
          <PageList
            pages={pages}
            selectedPage={selectedPage}
            onSelect={selectPage}
            onDelete={deletePage}
          />
        </div>
        <div className="flex-1 min-w-0 bg-spectyn-card border border-spectyn-border rounded-lg overflow-hidden">
          {loading ? (
            <div className="flex items-center justify-center h-full">
              <div className="w-6 h-6 border-2 border-spectyn-primary border-t-transparent rounded-full animate-spin" />
            </div>
          ) : (
            <PageFrame html={html} />
          )}
        </div>
      </div>
    </div>
  );
}
