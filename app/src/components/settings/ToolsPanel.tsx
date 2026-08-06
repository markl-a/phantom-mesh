import { useEffect, useState } from "react";
import { safeInvoke as invoke } from "../../lib/tauri-compat";

export default function ToolsPanel() {
  const [tools, setTools] = useState<any>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke("get_tools")
      .then(setTools)
      .catch((e) => setError(String(e)));
  }, []);

  return (
    <div>
      <h1 className="text-2xl font-bold mb-4">工具管理</h1>
      {error && (
        <div className="bg-spectyn-danger/20 border border-spectyn-danger rounded p-3 mb-4 text-sm">
          {error}
        </div>
      )}
      {tools ? (
        <pre className="bg-spectyn-card border border-spectyn-border rounded p-4 text-sm overflow-auto">
          {JSON.stringify(tools, null, 2)}
        </pre>
      ) : (
        <p className="text-spectyn-muted">載入中...</p>
      )}
    </div>
  );
}
