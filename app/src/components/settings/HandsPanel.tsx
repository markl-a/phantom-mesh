import { useEffect, useState } from "react";
import { safeInvoke as invoke } from "../../lib/tauri-compat";

export default function HandsPanel() {
  const [hands, setHands] = useState<any>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke("get_hands")
      .then(setHands)
      .catch((e) => setError(String(e)));
  }, []);

  return (
    <div>
      <h1 className="text-2xl font-bold mb-4">Hand・Pipeline</h1>
      {error && (
        <div className="bg-spectyn-danger/20 border border-spectyn-danger rounded p-3 mb-4 text-sm">
          {error}
        </div>
      )}
      {hands ? (
        <pre className="bg-spectyn-card border border-spectyn-border rounded p-4 text-sm overflow-auto">
          {JSON.stringify(hands, null, 2)}
        </pre>
      ) : (
        <p className="text-spectyn-muted">載入中...</p>
      )}
    </div>
  );
}
