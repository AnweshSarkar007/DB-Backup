import { ErrorReport, ProgressEvent, RunResult, humanBytes } from "../api";
import { BACKUP_STAGES } from "../hooks";

export function ProgressPanel({
  events,
  running,
  currentStage,
}: {
  events: ProgressEvent[];
  running: boolean;
  currentStage: string | null;
}) {
  if (events.length === 0 && !running) return null;
  return (
    <div className="card">
      <h3>Progress</h3>
      <ul className="stages">
        {BACKUP_STAGES.map((s) => {
          const done = events.some((e) => e.state === s);
          const active = currentStage === s && running;
          return (
            <li key={s} className={done ? "done" : active ? "active" : ""}>
              <span className="mark">{done ? "✓" : active ? "●" : "○"}</span> {s}
            </li>
          );
        })}
      </ul>
      <div className="log">
        {events.map((e, i) => (
          <div key={i}>
            <code>[{e.state}]</code> {e.message}
          </div>
        ))}
      </div>
    </div>
  );
}

export function ResultPanel({ result }: { result: RunResult }) {
  return (
    <div className={`card result ${result.state.toLowerCase()}`}>
      <h3>Result: {result.state}</h3>
      <div className="info-grid">
        <span>File</span>
        <b>{result.filename ?? "—"}</b>
        <span>Size</span>
        <b>{humanBytes(result.file_size)}</b>
        <span>Verification</span>
        <b>{result.verification}</b>
        <span>SHA-256</span>
        <b className="mono">{result.sha256 ?? "—"}</b>
      </div>
      <h4>Destinations</h4>
      <ul>
        {result.destinations.map((d, i) => (
          <li key={i}>
            {d.status === "completed" ? "✓" : d.status === "skipped" ? "–" : "✗"} {d.name}
            {d.error_message ? ` — ${d.error_message}` : ""}
          </li>
        ))}
      </ul>
    </div>
  );
}

export function ErrorPanel({ error }: { error: ErrorReport }) {
  return (
    <div className="card error">
      <h3>{error.message}</h3>
      {error.detail && <pre className="small">{error.detail}</pre>}
      <span className="small muted">Code: {error.code}</span>
    </div>
  );
}
