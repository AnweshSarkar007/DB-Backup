import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  DestinationRecord, ErrorReport, addLocalDestination, connectGoogleDrive,
  deleteDestination, googleClientConfigured, importGoogleOAuthJson,
  listDestinations, reconnectGoogleDrive, setDestinationEnabled, setGoogleClient,
} from "../api";
import { useAsync } from "../hooks";
import { ErrorPanel } from "../components/BackupView";

function GoogleDriveSection({ onChanged }: { onChanged: () => void }) {
  const configured = useAsync<boolean>(googleClientConfigured, []);
  const [name, setName] = useState("Google Drive");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ErrorReport | null>(null);
  const [note, setNote] = useState("");
  const [selectedFile, setSelectedFile] = useState("");
  const [manualOpen, setManualOpen] = useState(false);
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");

  function importedMessage(result: Awaited<ReturnType<typeof importGoogleOAuthJson>>) {
    setNote(result.same_client_id
      ? "OAuth client saved. Existing Google Drive accounts remain connected."
      : result.accounts_reconnect_required
        ? `OAuth client saved. Reconnect ${result.accounts_reconnect_required} existing Google Drive account(s).`
        : "OAuth client saved.");
    configured.reload();
    onChanged();
  }

  async function importClient() {
    setError(null);
    try {
      const path = await open({ multiple: false, title: "Choose Google OAuth Desktop JSON", filters: [{ name: "JSON", extensions: ["json"] }] });
      if (typeof path !== "string") return;
      setSelectedFile(path.split(/[\\/]/).pop() || path);
      setBusy(true);
      setNote("Importing OAuth JSON...");
      importedMessage(await importGoogleOAuthJson(path));
    } catch (e) { setError(e as ErrorReport); }
    finally { setBusy(false); }
  }

  async function saveManual() {
    setError(null); setBusy(true); setNote("Saving OAuth client...");
    try {
      importedMessage(await setGoogleClient(clientId, clientSecret));
      setClientId(""); setClientSecret("");
    } catch (e) { setError(e as ErrorReport); setNote(""); }
    finally { setBusy(false); }
  }

  async function connect() {
    setError(null);
    setNote("A browser window will open. Approve access, then return here.");
    setBusy(true);
    try { await connectGoogleDrive(name); setNote("Google Drive connected."); onChanged(); }
    catch (e) { setError(e as ErrorReport); setNote(""); }
    finally { setBusy(false); }
  }

  return <div className="card">
    <label>Google Drive</label>
    {configured.data && <div className="row">
      <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Google Drive" />
      <button className="primary" onClick={connect} disabled={busy}>{busy ? "Waiting for authorization..." : "Connect Google Drive"}</button>
    </div>}
    <p className="small muted">{configured.data
      ? <>OAuth Desktop client configured. Import another JSON file or enter credentials manually to replace it.</>
      : <>Choose the OAuth JSON downloaded for a Google Cloud Console <b>Desktop app</b> client, or add its credentials manually. Enable the Drive API first. The app requests only the <code>drive.file</code> scope.</>}</p>
    <div className="row">
      <button onClick={importClient} disabled={busy}>{busy && note.startsWith("Importing") ? "Importing file..." : "Choose OAuth JSON file"}</button>
      <button onClick={() => setManualOpen((open) => !open)} disabled={busy}>{manualOpen ? "Hide manual entry" : "Add manually"}</button>
    </div>
    {selectedFile && <div className="small muted">Selected file: <b>{selectedFile}</b></div>}
    {manualOpen && <div className="stack">
      <label>OAuth client ID</label>
      <input value={clientId} onChange={(e) => setClientId(e.target.value)} placeholder="...apps.googleusercontent.com" />
      <label>OAuth client secret (optional)</label>
      <input type="password" value={clientSecret} onChange={(e) => setClientSecret(e.target.value)} placeholder="Client secret" />
      <button className="primary" onClick={saveManual} disabled={busy || !clientId.trim()}>{busy && note.startsWith("Saving") ? "Saving..." : "Save OAuth credentials"}</button>
    </div>}
    {note && <div className="small muted">{note}</div>}
    {error && <ErrorPanel error={error} />}
  </div>;
}

export default function Destinations() {
  const dests = useAsync<DestinationRecord[]>(listDestinations, []);
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [error, setError] = useState<ErrorReport | null>(null);
  const [reconnecting, setReconnecting] = useState<number | null>(null);

  async function pick() {
    setError(null);
    try {
      const dir = await open({ directory: true, multiple: false, title: "Choose a backup folder" });
      if (typeof dir === "string") { setPath(dir); if (!name) setName(dir.split(/[\\/]/).pop() || "Local"); }
    } catch (e) { setError({ code: "FolderPickerFailed", message: "Could not open the folder picker.", detail: String(e), retryable: false }); }
  }
  async function add() {
    setError(null);
    try { await addLocalDestination(name || "Local", path); setName(""); setPath(""); dests.reload(); }
    catch (e) { setError(e as ErrorReport); }
  }
  async function reconnect(id: number) {
    setError(null); setReconnecting(id);
    try { await reconnectGoogleDrive(id); dests.reload(); }
    catch (e) { setError(e as ErrorReport); }
    finally { setReconnecting(null); }
  }

  return <div className="page">
    <h1>Destinations</h1>
    <p className="small muted">A verified backup is copied to every enabled destination.</p>
    <div className="card">
      <label>Add local folder</label>
      <div className="row"><input value={path} readOnly placeholder="Choose a folder..." /><button onClick={pick}>Browse...</button></div>
      <label>Name</label>
      <div className="row"><input value={name} onChange={(e) => setName(e.target.value)} placeholder="Local HDD" /><button className="primary" onClick={add} disabled={!path}>Add</button></div>
      {error && <ErrorPanel error={error} />}
    </div>
    <GoogleDriveSection onChanged={dests.reload} />
    {dests.data?.length === 0 && <div className="card muted">No destinations yet.</div>}
    {dests.data?.map((d) => {
      let sub = d.kind;
      let needsReconnect = false;
      try {
        const cfg = JSON.parse(d.configuration_json);
        if (cfg.path) sub = cfg.path;
        if (d.kind === "google_drive") { needsReconnect = cfg.reauthorization_required === true; sub = needsReconnect ? "Reconnect required" : (cfg.account_email || "Google account"); }
      } catch { /* ignore malformed legacy display metadata */ }
      return <div key={d.id} className="card db-card"><div className="db-head"><div>
        <h3>{d.name}</h3><div className="small muted"><span className={`dot ${d.enabled ? "ok" : "warn"}`} /> {sub} · {d.kind}</div>
      </div><div className="row">
        {needsReconnect
          ? <button className="primary" disabled={reconnecting !== null} onClick={() => reconnect(d.id)}>{reconnecting === d.id ? "Waiting for authorization..." : "Reconnect"}</button>
          : <button onClick={() => setDestinationEnabled(d.id, !d.enabled).then(dests.reload)}>{d.enabled ? "Disable" : "Enable"}</button>}
        <button onClick={() => { if (confirm(`Delete destination “${d.name}”?`)) deleteDestination(d.id).then(dests.reload); }}>Delete</button>
      </div></div></div>;
    })}
    {error && <ErrorPanel error={error} />}
  </div>;
}
