import {
  ConnectionRecord,
  DestinationRecord,
  RunSummary,
  fmtDate,
  humanBytes,
  listConnections,
  listDestinations,
  listRuns,
} from "../api";
import { useAsync } from "../hooks";

export default function Dashboard() {
  const conns = useAsync<ConnectionRecord[]>(listConnections, []);
  const dests = useAsync<DestinationRecord[]>(listDestinations, []);
  const runs = useAsync<RunSummary[]>(() => listRuns(null, null, 8), []);

  const last = runs.data?.[0];

  return (
    <div className="page">
      <h1>Dashboard</h1>
      <div className="stat-row">
        <Stat label="Databases" value={`${conns.data?.length ?? 0} connected`} />
        <Stat
          label="Last Backup"
          value={last ? `${last.status} · ${fmtDate(last.started_at)}` : "None yet"}
        />
        <Stat
          label="Destinations"
          value={`${dests.data?.filter((d) => d.enabled).length ?? 0} enabled`}
        />
      </div>

      <h3>Recent activity</h3>
      {runs.data?.length === 0 && <div className="card muted">No backups yet.</div>}
      {runs.data && runs.data.length > 0 && (
        <table className="table">
          <tbody>
            {runs.data.map((r) => (
              <tr key={r.id}>
                <td>
                  <span className={`badge ${r.status.toLowerCase()}`}>{r.status}</span>
                </td>
                <td>{r.database_label ?? "—"}</td>
                <td>{humanBytes(r.file_size)}</td>
                <td className="muted">{fmtDate(r.started_at)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="card stat">
      <div className="stat-label">{label}</div>
      <div className="stat-value">{value}</div>
    </div>
  );
}
