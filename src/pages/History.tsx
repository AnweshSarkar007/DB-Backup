import { useState } from "react";
import { RunSummary, fmtDate, humanBytes, listRuns } from "../api";
import { useAsync } from "../hooks";
import { ErrorPanel } from "../components/BackupView";

const STATUSES = ["", "COMPLETED", "PARTIAL_SUCCESS", "FAILED", "CANCELLED"];

export default function History() {
  const [status, setStatus] = useState("");
  const runs = useAsync<RunSummary[]>(() => listRuns(null, status || null, 200), [status]);
  const [detail, setDetail] = useState<RunSummary | null>(null);

  return (
    <div className="page">
      <div className="page-head">
        <h1>Backup History</h1>
        <select value={status} onChange={(e) => setStatus(e.target.value)}>
          {STATUSES.map((s) => (
            <option key={s} value={s}>
              {s === "" ? "All statuses" : s}
            </option>
          ))}
        </select>
      </div>

      {runs.error && <ErrorPanel error={runs.error} />}
      {runs.data?.length === 0 && <div className="card muted">No backups recorded yet.</div>}

      {runs.data && runs.data.length > 0 && (
        <table className="table">
          <thead>
            <tr>
              <th>Started</th>
              <th>Database</th>
              <th>Status</th>
              <th>Size</th>
              <th>Trigger</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {runs.data.map((r) => (
              <tr key={r.id}>
                <td>{fmtDate(r.started_at)}</td>
                <td>{r.database_label ?? "—"}</td>
                <td>
                  <span className={`badge ${r.status.toLowerCase()}`}>{r.status}</span>
                </td>
                <td>{humanBytes(r.file_size)}</td>
                <td>{r.trigger_type}</td>
                <td>
                  <button onClick={() => setDetail(r)}>Details</button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {detail && <DetailDrawer run={detail} onClose={() => setDetail(null)} />}
    </div>
  );
}

function DetailDrawer({ run, onClose }: { run: RunSummary; onClose: () => void }) {
  return (
    <div className="overlay" onClick={onClose}>
      <div className="dialog" onClick={(e) => e.stopPropagation()}>
        <div className="page-head">
          <h2>Backup Details</h2>
          <button onClick={onClose}>✕</button>
        </div>
        <div className="card">
          <div className="info-grid">
            <span>Backup ID</span>
            <b className="mono">{run.uuid}</b>
            <span>Database</span>
            <b>{run.database_label ?? "—"}</b>
            <span>Status</span>
            <b>{run.status}</b>
            <span>Started</span>
            <b>{fmtDate(run.started_at)}</b>
            <span>Completed</span>
            <b>{fmtDate(run.completed_at)}</b>
            <span>Trigger</span>
            <b>{run.trigger_type}</b>
            <span>File</span>
            <b>{run.filename ?? "—"}</b>
            <span>Size</span>
            <b>{humanBytes(run.file_size)}</b>
            <span>Verification</span>
            <b>{run.verification_status ?? "—"}</b>
            <span>SHA-256</span>
            <b className="mono">{run.sha256 ?? "—"}</b>
          </div>
          {run.error_message && (
            <div className="card error">
              <b>{run.error_message}</b>
              {run.error_code && <div className="small muted">Code: {run.error_code}</div>}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
