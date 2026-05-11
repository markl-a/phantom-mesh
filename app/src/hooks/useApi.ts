import { useState, useEffect } from "react";
import { api } from "../lib/api";

export function useApi<T>(command: string, args?: Record<string, unknown>) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    setLoading(true);
    api<T>(command, args)
      .then(setData)
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  }, [command, JSON.stringify(args)]);

  return { data, error, loading, refetch: () => {
    setLoading(true);
    api<T>(command, args)
      .then(setData)
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  }};
}
