import { useCallback, useEffect, useRef, useState } from "react";
import { ErrorReport } from "./api";
import { normalizeError } from "./errors";
export { useBackup } from "./backupOperation";

export const BACKUP_STAGES = [
  "Connecting", "Preparing", "Dumping", "Verifying", "Hashing", "Distributing", "Completed",
];

/// Latest-request-only data loading. Existing data is retained during a refresh.
export function useAsync<T>(fn: () => Promise<T>, deps: unknown[] = []) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<ErrorReport | null>(null);
  const [loading, setLoading] = useState(true);
  const generation = useRef(0);
  const mounted = useRef(false);
  const reload = useCallback(async () => {
    if (!mounted.current) return;
    const request = ++generation.current;
    setLoading(true);
    setError(null);
    const current = () => mounted.current && request === generation.current;
    try {
      const result = await fn();
      if (current()) setData(result);
    } catch (error) {
      if (current()) setError(normalizeError(error));
    } finally {
      if (current()) setLoading(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  useEffect(() => {
    mounted.current = true;
    void reload();
    return () => { mounted.current = false; generation.current++; };
  }, [reload]);
  return { data, error, loading, reload };
}
