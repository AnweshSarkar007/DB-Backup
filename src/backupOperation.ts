import { normalizeError } from "./errors";
import { useCallback, useId, useSyncExternalStore } from "react";
import { ErrorReport, ProgressEvent, RunResult, onBackupProgress } from "./api";

// The event protocol has no job ID: only one UI-triggered backup can own it.
interface Operation {
  owner: string | null;
  events: ProgressEvent[];
  running: boolean;
  result: RunResult | null;
  error: ErrorReport | null;
}
let state: Operation = { owner: null, events: [], running: false, result: null, error: null };
let generation = 0;
const subscribers = new Set<() => void>();
const subscribe = (callback: () => void) => { subscribers.add(callback); return () => { subscribers.delete(callback); }; };
const snapshot = () => state;
function update(patch: Partial<Operation>) {
  state = { ...state, ...patch };
  subscribers.forEach((callback) => callback());
}
async function run(owner: string, fn: () => Promise<RunResult>) {
  if (state.running) return;
  const operation = ++generation;
  update({ owner, events: [], running: true, result: null, error: null });
  let unlisten: (() => void) | undefined;
  try {
    // Register before invoking, including when registration resolves asynchronously.
    unlisten = await onBackupProgress((event) => {
      if (state.running && operation === generation) update({ events: [...state.events, event] });
    });
    const result = await fn();
    update({ result, events: [...state.events, { state: result.state, message: `Operation returned ${result.state}.` }] });
  } catch (error) {
    const report = normalizeError(error);
    update({ error: report, events: [...state.events, { state: "Failed", message: report.message }] });
  } finally {
    unlisten?.();
    update({ running: false });
  }
}
export function useOperation() {
  const operation = useSyncExternalStore(subscribe, snapshot);
  return { ...operation, currentStage: operation.events[operation.events.length - 1]?.state ?? null };
}
export function useBackup(identity?: string) {
  const generated = useId();
  const owner = identity ?? generated;
  const operation = useOperation();
  const owned = operation.owner === owner;
  const start = useCallback((fn: () => Promise<RunResult>) => run(owner, fn), [owner]);
  return { ...operation, events: owned ? operation.events : [], result: owned ? operation.result : null,
    error: owned ? operation.error : null, currentStage: owned ? operation.currentStage : null, run: start };
}
