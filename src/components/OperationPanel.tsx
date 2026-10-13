import { useOperation } from "../backupOperation";
import { ErrorPanel, ProgressPanel, ResultPanel } from "./BackupView";

export function OperationPanel() {
  const operation = useOperation();
  if (!operation.owner) return null;
  return (
    <section className="card" aria-label="Backup operation">
      <div role="status">
        <strong>Backup operation</strong> · {operation.owner} · {operation.running
          ? operation.currentStage ?? "Starting…"
          : operation.error ? "Failed" : operation.result?.state ?? "Finished"}
      </div>
      <details>
        <summary>View progress and result</summary>
        <ProgressPanel events={operation.events} running={operation.running} currentStage={operation.currentStage} />
        {operation.result && <ResultPanel result={operation.result} />}
        {operation.error && <ErrorPanel error={operation.error} />}
      </details>
    </section>
  );
}
