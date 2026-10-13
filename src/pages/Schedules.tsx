import { useState } from "react";
import {
  ConnectionRecord,
  DestinationRecord,
  ErrorReport,
  Schedule,
  ScheduleInfo,
  createSchedule,
  deleteSchedule,
  listConnections,
  listDestinations,
  listSchedules,
  runScheduleNow,
  setScheduleEnabled,
} from "../api";
import { useAsync, useBackup } from "../hooks";
import { ErrorPanel } from "../components/BackupView";

const DAYS = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

export default function Schedules() {
  const schedules = useAsync<ScheduleInfo[]>(listSchedules, []);
  const conns = useAsync<ConnectionRecord[]>(listConnections, []);
  const dests = useAsync<DestinationRecord[]>(listDestinations, []);
  const [showAdd, setShowAdd] = useState(false);

  return (
    <div className="page">
      <div className="page-head">
        <h1>Schedules</h1>
        <button className="primary" onClick={() => setShowAdd((v) => !v)}>
          {showAdd ? "Cancel" : "+ New Schedule"}
        </button>
      </div>
      <p className="small muted">
        Scheduled backups run via <code>backup-worker.exe</code> registered with Windows Task
        Scheduler — they fire even when this window is closed (while you are logged in).
      </p>

      {showAdd && (
        <AddSchedule
          connections={conns.data ?? []}
          destinations={(dests.data ?? []).filter((d) => d.enabled)}
          onCreated={() => {
            setShowAdd(false);
            schedules.reload();
          }}
        />
      )}

      {schedules.error && <ErrorPanel error={schedules.error} />}
      {schedules.data?.length === 0 && !showAdd && (
        <div className="card muted">No schedules yet.</div>
      )}

      {schedules.data?.map((s) => (
        <div key={s.policy.id} className="card db-card">
          <div className="db-head">
            <div>
              <h3>{s.policy.name}</h3>
              <div className="small muted">
                <span className={`dot ${s.policy.enabled ? "ok" : "warn"}`} /> {s.connection_name} ·{" "}
                {describe(s.schedule)}
              </div>
              <div className="small muted">
                Next run: {s.task.next_run ?? "—"} · Last run: {s.task.last_run ?? "—"}
                {s.task.last_result ? ` (result ${s.task.last_result})` : ""}
              </div>
            </div>
            <div className="row">
              <RunScheduleButton schedule={s} onFinished={schedules.reload} />
              <button
                onClick={() =>
                  setScheduleEnabled(s.policy.id, !s.policy.enabled).then(schedules.reload)
                }
              >
                {s.policy.enabled ? "Disable" : "Enable"}
              </button>
              <button
                onClick={() => {
                  if (confirm(`Delete schedule “${s.policy.name}”?`))
                    deleteSchedule(s.policy.id).then(schedules.reload);
                }}
              >
                Delete
              </button>
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

function RunScheduleButton({ schedule, onFinished }: { schedule: ScheduleInfo; onFinished: () => void }) {
  const { running, run } = useBackup(`Schedule ${schedule.policy.id}: ${schedule.policy.name}`);
  return <button disabled={running} onClick={() => {
    void run(() => runScheduleNow(schedule.policy.id)).then(onFinished);
  }}>Run now</button>;
}

function describe(s: Schedule | null): string {
  if (!s) return "no schedule";
  if (s.frequency === "daily") return `Every day at ${s.time}`;
  return `Every ${s.day_of_week ?? "SUN"} at ${s.time}`;
}

function AddSchedule({
  connections,
  destinations,
  onCreated,
}: {
  connections: ConnectionRecord[];
  destinations: DestinationRecord[];
  onCreated: () => void;
}) {
  const [name, setName] = useState("");
  const [connId, setConnId] = useState<number | "">(connections[0]?.id ?? "");
  const [selected, setSelected] = useState<number[]>(destinations.map((d) => d.id));
  const [frequency, setFrequency] = useState<"daily" | "weekly">("weekly");
  const [day, setDay] = useState("SUN");
  const [time, setTime] = useState("02:00");
  const [keepLast, setKeepLast] = useState("");
  const [keepDays, setKeepDays] = useState("");
  const [error, setError] = useState<ErrorReport | null>(null);
  const [saving, setSaving] = useState(false);

  function toggle(id: number) {
    setSelected((s) => (s.includes(id) ? s.filter((x) => x !== id) : [...s, id]));
  }
  async function save() {
    setError(null);
    if (connId === "") return;
    setSaving(true);
    const schedule: Schedule = {
      frequency,
      time,
      day_of_week: frequency === "weekly" ? day : null,
    };
    const retention =
      keepLast || keepDays
        ? {
            keep_last: keepLast ? Number(keepLast) : null,
            keep_days: keepDays ? Number(keepDays) : null,
          }
        : null;
    try {
      await createSchedule(connId as number, name || "Weekly backup", selected, schedule, retention);
      onCreated();
    } catch (e) {
      setError(e as ErrorReport);
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="card">
      <label>Name</label>
      <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Weekly backup" />

      <label>Database</label>
      <select value={connId} onChange={(e) => setConnId(Number(e.target.value))}>
        {connections.length === 0 && <option value="">(add a database first)</option>}
        {connections.map((c) => (
          <option key={c.id} value={c.id}>
            {c.config.name}
          </option>
        ))}
      </select>

      <label>Destinations</label>
      {destinations.length === 0 && <div className="small muted">Add a destination first.</div>}
      {destinations.map((d) => (
        <label key={d.id} className="check">
          <input type="checkbox" checked={selected.includes(d.id)} onChange={() => toggle(d.id)} />
          {d.name}
        </label>
      ))}

      <label>Frequency</label>
      <div className="row">
        <select value={frequency} onChange={(e) => setFrequency(e.target.value as "daily" | "weekly")}>
          <option value="weekly">Weekly</option>
          <option value="daily">Daily</option>
        </select>
        {frequency === "weekly" && (
          <select value={day} onChange={(e) => setDay(e.target.value)}>
            {DAYS.map((d) => (
              <option key={d} value={d}>
                {d}
              </option>
            ))}
          </select>
        )}
        <input
          style={{ maxWidth: 120 }}
          value={time}
          onChange={(e) => setTime(e.target.value)}
          placeholder="HH:MM"
        />
      </div>

      <label>Retention (optional)</label>
      <div className="row">
        <input
          style={{ maxWidth: 160 }}
          value={keepLast}
          onChange={(e) => setKeepLast(e.target.value.replace(/\D/g, ""))}
          placeholder="Keep last N"
        />
        <input
          style={{ maxWidth: 160 }}
          value={keepDays}
          onChange={(e) => setKeepDays(e.target.value.replace(/\D/g, ""))}
          placeholder="Keep D days"
        />
      </div>

      <div className="row">
        <button
          className="primary"
          onClick={save}
          disabled={saving || connId === "" || selected.length === 0}
        >
          {saving ? "Creating…" : "Create Schedule"}
        </button>
      </div>
      {error && <ErrorPanel error={error} />}
    </div>
  );
}
