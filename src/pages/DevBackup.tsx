// "Quick Backup": a one-off backup from a pasted URI to a chosen folder, without saving anything.
// The saved-connection flow lives on the Databases page.
import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { ConnInfo, ErrorReport, humanBytes, runLocalBackup, testConnectionUri } from "../api";
import { useBackup } from "../hooks";
import { ErrorPanel, ProgressPanel, ResultPanel } from "../components/BackupView";

export default function DevBackup() {
  const [name, setName] = useState("My Database");
  const [uri, setUri] = useState("");
  const [destName, setDestName] = useState("Local HDD");
  const [destPath, setDestPath] = useState("");
  const [conn, setConn] = useState<ConnInfo | null>(null);
  const [testing, setTesting] = useState(false);
  const [testErr, setTestErr] = useState<ErrorReport | null>(null);
  const [folderErr, setFolderErr] = useState<ErrorReport | null>(null);
  const { events, running, result, error, run, currentStage } = useBackup();

  async function pickFolder() {
    setFolderErr(null);
    try {
      const dir = await open({ directory: true, multiple: false, title: "Choose a backup folder" });
      if (typeof dir === "string") setDestPath(dir);
    } catch (e) {
      setFolderErr({
        code: "FolderPickerFailed",
        message: "Could not open the folder picker.",
        detail: String(e),
        retryable: false,
      });
    }
  }
  async function doTest() {
    setTestErr(null);
    setConn(null);
    setTesting(true);
    try {
      setConn(await testConnectionUri(uri, name));
    } catch (e) {
      setTestErr(e as ErrorReport);
    } finally {
      setTesting(false);
    }
  }

  return (
    <div className="page">
      <h1>Quick Backup</h1>
      <p className="small muted">
        A one-off local backup from a pasted connection string. Nothing is saved; the password is used
        transiently. To reuse a database, add it on the Databases page.
      </p>

      <div className="card">
        <label>Display name</label>
        <input value={name} onChange={(e) => setName(e.target.value)} />
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
        </div>
        {conn && (
          <div className="info-grid">
            <span>PostgreSQL</span>
            <b>
              {conn.postgres_major} ({conn.postgres_text})
            </b>
            <span>Backup engine</span>
            <b>{conn.engine_image}</b>
            <span>Size</span>
            <b>{humanBytes(conn.size_bytes)}</b>
          </div>
        )}
        {testErr && <ErrorPanel error={testErr} />}
      </div>

      <div className="card">
        <label>Destination folder</label>
        <div className="row">
          <input value={destPath} readOnly placeholder="Choose a folder…" />
          <button onClick={pickFolder}>Browse…</button>
        </div>
        {folderErr && <ErrorPanel error={folderErr} />}
        <label>Destination name</label>
        <input value={destName} onChange={(e) => setDestName(e.target.value)} />
        <div className="row">
          <button
            className="primary"
            disabled={running || !uri || !destPath}
            onClick={() => run(() => runLocalBackup(uri, name, destName, destPath))}
          >
            {running ? "Backing up…" : "Backup Now"}
          </button>
        </div>
      </div>

      <ProgressPanel events={events} running={running} currentStage={currentStage} />
      {result && <ResultPanel result={result} />}
      {error && <ErrorPanel error={error} />}
    </div>
  );
}
