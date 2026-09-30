import { useState } from "react";
import {
  ConnInfo,
  ConnectionRecord,
  DestinationRecord,
  ErrorReport,
  addConnection,
  backupNow,
  deleteConnection,
  humanBytes,
  listConnections,
  listDestinations,
  testConnection,
  testConnectionUri,
} from "../api";
import { useAsync, useBackup } from "../hooks";
import { ErrorPanel, ProgressPanel, ResultPanel } from "../components/BackupView";

export default function Databases() {
  const conns = useAsync<ConnectionRecord[]>(listConnections, []);
  const dests = useAsync<DestinationRecord[]>(listDestinations, []);
  const [showAdd, setShowAdd] = useState(false);
  const [backupFor, setBackupFor] = useState<ConnectionRecord | null>(null);

  return (
    <div className="page">
      <div className="page-head">
        <h1>Databases</h1>
        <button className="primary" onClick={() => setShowAdd((v) => !v)}>
          {showAdd ? "Cancel" : "+ Add Database"}
        </button>
      </div>

      {showAdd && (
        <AddDatabase
          onAdded={() => {
            setShowAdd(false);
            conns.reload();
          }}
        />
      )}

      {conns.loading && <p className="muted">Loading…</p>}
      {conns.error && <ErrorPanel error={conns.error} />}

      {conns.data?.length === 0 && !showAdd && (
        <div className="card muted">No databases yet. Click “Add Database” to connect one.</div>
      )}

      {conns.data?.map((c) => (
        <ConnectionCard
          key={c.id}
          conn={c}
          onDeleted={conns.reload}
          onBackup={() => setBackupFor(c)}
        />
      ))}

      {backupFor && (
        <BackupDialog
          conn={backupFor}
          destinations={dests.data ?? []}
          onClose={() => setBackupFor(null)}
        />
      )}
    </div>
  );
}

function AddDatabase({ onAdded }: { onAdded: () => void }) {
  const [name, setName] = useState("");
  const [uri, setUri] = useState("");
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [info, setInfo] = useState<ConnInfo | null>(null);
  const [error, setError] = useState<ErrorReport | null>(null);

  async function doTest() {
    setError(null);
    setInfo(null);
    setTesting(true);
    try {
      setInfo(await testConnectionUri(uri, name || "Database"));
    } catch (e) {
      setError(e as ErrorReport);
    } finally {
      setTesting(false);
    }
  }
  async function doSave() {
    setError(null);
    setSaving(true);
    try {
      await addConnection(name || "Database", uri);
      onAdded();
    } catch (e) {
      setError(e as ErrorReport);
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="card">
      <label>Name</label>
      <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Glasafe Production" />
      <label>Connection string</label>
      <input
        value={uri}
        onChange={(e) => setUri(e.target.value)}
        placeholder="postgresql://postgres:PASSWORD@db.xxx.supabase.co:5432/postgres?sslmode=require"
      />
      <div className="row">
        <button onClick={doTest} disabled={testing || !uri}>
          {testing ? "Testing…" : "Test Connection"}
        </button>
        <button className="primary" onClick={doSave} disabled={saving || !uri}>
          {saving ? "Saving…" : "Save"}
        </button>
      </div>
      {info && (
        <div className="info-grid">
          <span>PostgreSQL</span>
          <b>
            {info.postgres_major} ({info.postgres_text})
          </b>
          <span>Backup engine</span>
          <b>{info.engine_image}</b>
          <span>Size</span>
          <b>{humanBytes(info.size_bytes)}</b>
        </div>
      )}
      {error && <ErrorPanel error={error} />}
      <p className="small muted">
        The password is extracted and stored in Windows Credential Manager. Only sanitized metadata is
        saved locally.
      </p>
    </div>
  );
}

function ConnectionCard({
  conn,
  onDeleted,
  onBackup,
}: {
  conn: ConnectionRecord;
  onDeleted: () => void;
  onBackup: () => void;
}) {
  const [status, setStatus] = useState<string>("");
  const [busy, setBusy] = useState(false);

  async function test() {
    setBusy(true);
    setStatus("Testing…");
    try {
      const info = await testConnection(conn.id);
      setStatus(`Connected · PostgreSQL ${info.postgres_major}`);
    } catch (e) {
      setStatus((e as ErrorReport).message);
    } finally {
      setBusy(false);
    }
  }
  async function remove() {
    if (!confirm(`Delete “${conn.config.name}”? Its stored password will be removed too.`)) return;
    await deleteConnection(conn.id);
    onDeleted();
  }

  return (
    <div className="card db-card">
      <div className="db-head">
        <div>
          <h3>{conn.config.name}</h3>
          <div className="small muted">
            {conn.config.username}@{conn.config.host}:{conn.config.port}/{conn.config.database}
            {conn.config.postgres_major_version ? ` · PostgreSQL ${conn.config.postgres_major_version}` : ""}
          </div>
        </div>
        <div className="row">
          <button onClick={test} disabled={busy}>
            Test
          </button>
          <button className="primary" onClick={onBackup}>
            Backup Now
          </button>
          <button onClick={remove}>Delete</button>
        </div>
      </div>
      {status && <div className="small">{status}</div>}
    </div>
  );
}

function BackupDialog({
  conn,
  destinations,
  onClose,
}: {
  conn: ConnectionRecord;
  destinations: DestinationRecord[];
  onClose: () => void;
}) {
  const enabled = destinations.filter((d) => d.enabled);
  const [selected, setSelected] = useState<number[]>(enabled.map((d) => d.id));
  const { events, running, result, error, run, currentStage } = useBackup();

  function toggle(id: number) {
    setSelected((s) => (s.includes(id) ? s.filter((x) => x !== id) : [...s, id]));
  }

  return (
    <div className="overlay" onClick={onClose}>
      <div className="dialog" onClick={(e) => e.stopPropagation()}>
        <div className="page-head">
          <h2>Back up {conn.config.name}</h2>
          <button onClick={onClose}>✕</button>
        </div>

        {enabled.length === 0 ? (
          <div className="card muted">
            No destinations configured. Add one on the Destinations page first.
          </div>
        ) : (
          <div className="card">
            <label>Destinations (fan-out — one dump, copied to each)</label>
            {enabled.map((d) => (
              <label key={d.id} className="check">
                <input
                  type="checkbox"
                  checked={selected.includes(d.id)}
                  onChange={() => toggle(d.id)}
                />
                {d.name}
              </label>
            ))}
            <div className="row">
              <button
                className="primary"
                disabled={running || selected.length === 0}
                onClick={() => run(() => backupNow(conn.id, selected))}
              >
                {running ? "Backing up…" : "Start Backup"}
              </button>
            </div>
          </div>
        )}

        <ProgressPanel events={events} running={running} currentStage={currentStage} />
        {result && <ResultPanel result={result} />}
        {error && <ErrorPanel error={error} />}
      </div>
    </div>
  );
}
