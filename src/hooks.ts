import { useCallback, useEffect, useRef, useState } from "react";
import { ErrorReport, ProgressEvent, RunResult, onBackupProgress } from "./api";

export const BACKUP_STAGES = [
  "Connecting",
  "Preparing",
  "Dumping",
  "Verifying",
  "Hashing",
  "Distributing",
  "Completed",
];

/// Drives a backup: subscribes to progress events and tracks running/result/error state.
export function useBackup() {
  const [events, setEvents] = useState<ProgressEvent[]>([]);
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<RunResult | null>(null);
  const [error, setError] = useState<ErrorReport | null>(null);
  const unlisten = useRef<null | (() => void)>(null);

  useEffect(() => {
    onBackupProgress((e) => setEvents((prev) => [...prev, e])).then((fn) => {
      unlisten.current = fn;
    });
    return () => {
      if (unlisten.current) unlisten.current();
    };
  }, []);

  const run = useCallback(async (fn: () => Promise<RunResult>) => {
    setError(null);
    setResult(null);
    setEvents([]);
    setRunning(true);
    try {
      setResult(await fn());
    } catch (e) {
      setError(e as ErrorReport);
    } finally {
      setRunning(false);
    }
  }, []);

  const currentStage = events.length ? events[events.length - 1].state : null;
  return { events, running, result, error, run, currentStage };
}

/// Simple data-loading helper with manual refresh.
export function useAsync<T>(fn: () => Promise<T>, deps: unknown[] = []) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<ErrorReport | null>(null);
  const [loading, setLoading] = useState(true);
  const reload = useCallback(() => {
    setLoading(true);
    fn()
      .then((d) => {
        setData(d);
        setError(null);
      })
      .catch((e) => setError(e as ErrorReport))
      .finally(() => setLoading(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  useEffect(() => {
    reload();
  }, [reload]);
  return { data, error, loading, reload };
}
