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
  const successful = runs.data?.find((run) => run.status === "Completed");

  return (
    <div className="page">
      <h1>Dashboard</h1>
      <div className="stat-row">
        <Stat label="Saved databases" value={conns.loading ? "Loading…" : conns.error ? "Could not load" : `${conns.data?.length ?? 0} saved`} />
        <Stat
          label="Latest attempt"
          value={runs.loading ? "Loading…" : runs.error ? "Could not load" : last ? `${last.status} · ${fmtDate(last.started_at)}` : "No recent attempts"}
        />
        <Stat
          label="Last successful backup (recent history)"
          value={runs.loading ? "Loading…" : runs.error ? "Could not load" : successful
            ? fmtDate(successful.started_at) : "None in recent history"}
        />
        <Stat
          label="Destinations"
          value={dests.loading ? "Loading…" : dests.error ? "Could not load" : `${dests.data?.filter((d) => d.enabled).length ?? 0} enabled`}
        />
      </div>

      {[{label: "Saved databases", source: conns}, {label: "Destinations", source: dests}, {label: "Recent activity", source: runs}].map(({label, source}) => source.error && (
        <div className="card error" role="alert" key={label}>
          {label}: {source.error.message} <button onClick={() => source.reload()}>Retry</button>
        </div>
      ))}
      <p className="small muted">Based on up to 8 recent attempts; older successful backups may not be shown. Retained history may be incomplete.</p>
      <h3>Recent activity</h3>
      {runs.loading && <div className="card muted">Loading…</div>}
      {!runs.loading && !runs.error && runs.data?.length === 0 && <div className="card muted">No recent backup activity available.</div>}
      {!runs.loading && !runs.error && runs.data && runs.data.length > 0 && (
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
