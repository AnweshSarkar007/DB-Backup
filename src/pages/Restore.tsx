import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ArchiveEntry,
  BrowseOpenResult,
  ConnectionRecord,
  ErrorReport,
  TableInfo,
  TablePage,
  browseClose,
  browseOpen,
  browseTable,
  inspectDump,
  listConnections,
  restoreBackup,
} from "../api";
import { useAsync } from "../hooks";
import { ErrorPanel } from "../components/BackupView";

type Mode = "inspect" | "browse" | "restore";

export default function Restore() {
  const [dump, setDump] = useState("");
  const [mode, setMode] = useState<Mode>("inspect");

  async function pick() {
    const f = await open({
      multiple: false,
      title: "Choose a .dump file",
      filters: [{ name: "Backup dump", extensions: ["dump"] }],
    });
    if (typeof f === "string") setDump(f);
  }

  return (
    <div className="page">
      <h1>Restore &amp; Inspect</h1>
      <p className="small muted">
        Inspect an archive, browse its data in a throwaway container, or restore it into a target
        database. Inspect and Browse never modify any database.
      </p>

      <div className="card">
        <label>Backup file (.dump)</label>
        <div className="row">
          <input value={dump} readOnly placeholder="Choose a .dump file..." />
          <button onClick={pick}>Browse...</button>
        </div>
        {dump && (
          <div className="row" style={{ marginTop: 12 }}>
            <button className={mode === "inspect" ? "primary" : ""} onClick={() => setMode("inspect")}>
              Inspect
            </button>
            <button className={mode === "browse" ? "primary" : ""} onClick={() => setMode("browse")}>
              Browse
            </button>
            <button className={mode === "restore" ? "primary" : ""} onClick={() => setMode("restore")}>
              Restore
            </button>
          </div>
        )}
      </div>

      {dump && mode === "inspect" && <Inspect dump={dump} />}
      {dump && mode === "browse" && <Browse dump={dump} />}
      {dump && mode === "restore" && <RestoreInto dump={dump} />}
    </div>
  );
}

function Inspect({ dump }: { dump: string }) {
  const [entries, setEntries] = useState<ArchiveEntry[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<ErrorReport | null>(null);

  async function go() {
    setLoading(true);
    setError(null);
    try {
      setEntries(await inspectDump(dump));
    } catch (e) {
      setError(e as ErrorReport);
    } finally {
      setLoading(false);
    }
  }

  const byType: Record<string, number> = {};
  entries?.forEach((e) => (byType[e.object_type] = (byType[e.object_type] || 0) + 1));

  return (
    <div className="card">
      <div className="row">
        <button className="primary" onClick={go} disabled={loading}>
          {loading ? "Reading archive..." : "Inspect archive"}
        </button>
      </div>
      {error && <ErrorPanel error={error} />}
      {entries && (
        <>
          <div className="info-grid" style={{ marginTop: 12 }}>
            {Object.entries(byType).map(([t, n]) => (
              <FragmentRow key={t} k={t} v={String(n)} />
            ))}
          </div>
          <h4>Objects ({entries.length})</h4>
          <div className="log" style={{ maxHeight: 300 }}>
            {entries.map((e, i) => (
              <div key={i}>
                <code>{e.object_type}</code> {e.schema}.{e.name}
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

function FragmentRow({ k, v }: { k: string; v: string }) {
  return (
    <>
      <span>{k}</span>
      <b>{v}</b>
    </>
  );
}

function Browse({ dump }: { dump: string }) {
  const [tables, setTables] = useState<TableInfo[] | null>(null);
  const [restoreWarning, setRestoreWarning] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [loading, setLoading] = useState(false);
  const [pageLoading, setPageLoading] = useState(false);
  const [hasLoadedRows, setHasLoadedRows] = useState(false);
  const [error, setError] = useState<ErrorReport | null>(null);
  const [active, setActive] = useState<TableInfo | null>(null);
  const [page, setPage] = useState<TablePage | null>(null);
  const [offset, setOffset] = useState(0);
  const LIMIT = 50;

  useEffect(() => {
    return () => {
      // Tear down the temporary container when leaving Browse.
      browseClose().catch(() => {});
    };
  }, []);

  async function openSession() {
    setLoading(true);
    setError(null);
    setActive(null);
    setPage(null);
    try {
      const result: BrowseOpenResult = await browseOpen(dump);
      setTables(result.tables);
      setRestoreWarning(result.restore_warning);
      setFilter("");
      setHasLoadedRows(false);
      setPage(null);
      setActive(null);
    } catch (e) {
      setError(e as ErrorReport);
    } finally {
      setLoading(false);
    }
  }

  async function loadTable(t: TableInfo, off: number) {
    setActive(t);
    setOffset(off);
    setPage(null);
    setError(null);
    setPageLoading(true);
    try {
      const nextPage = await browseTable(t.schema, t.name, off, LIMIT);
      setPage(nextPage);
      setHasLoadedRows(true);
      if (nextPage.restore_warning) setRestoreWarning(nextPage.restore_warning);
    } catch (e) {
      setError(e as ErrorReport);
    } finally {
      setPageLoading(false);
    }
  }

  const visibleTables = (tables ?? []).filter((t) =>
    `${t.schema}.${t.name}`.toLocaleLowerCase().includes(filter.trim().toLocaleLowerCase()),
  );
  const skippedExtensions = restoreWarning
    ? [...restoreWarning.matchAll(/extension\s+"([^"]+)"\s+is not available/gi)].map((m) => m[1])
    : [];
  const uniqueSkippedExtensions = [...new Set(skippedExtensions)];

  return (
    <div className="card">
      <div className="row">
        <button className="primary" onClick={openSession} disabled={loading}>
          {loading ? "Reading table list..." : tables ? "Reload table list" : "Open backup"}
        </button>
        {tables && <span className="small muted">Table names are read from the archive first. A temporary restore starts only when you select a table.</span>}
      </div>
      {error && <ErrorPanel error={error} />}
      {restoreWarning && (
        <div className="card result partial_success">
          <h4>Some backup objects could not be restored</h4>
          <p className="small">
            {uniqueSkippedExtensions.length > 0
              ? `The temporary PostgreSQL container does not have these extensions: ${uniqueSkippedExtensions.join(", ")}. Available tables can still be browsed; tables or rows that depend on skipped objects may be missing.`
              : "pg_restore reported skipped objects. Available tables can still be browsed, but some tables or rows may be missing."}
          </p>
          <details>
            <summary className="small">Show restore details</summary>
            <pre className="small">{restoreWarning}</pre>
          </details>
        </div>
      )}

      {tables && (
        <div className="browse-layout">
          <div className="browse-tables">
            <div className="row browse-list-head">
              <h4>Tables</h4>
              <span className="small muted">{visibleTables.length} / {tables.length}</span>
            </div>
            <input
              className="browse-filter"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Filter schema or table..."
              aria-label="Filter tables by schema or name"
            />
            {tables.length === 0 && <div className="small muted">No user tables were found in this archive.</div>}
            {tables.length > 0 && visibleTables.length === 0 && (
              <div className="small muted">No tables match this filter.</div>
            )}
            {visibleTables.map((t) => (
              <button
                key={`${t.schema}.${t.name}`}
                className={active?.name === t.name && active?.schema === t.schema ? "nav active table-nav" : "nav table-nav"}
                onClick={() => loadTable(t, 0)}
              >
                <span>{t.schema}.{t.name}</span>
                <span className="small muted" title="Estimated row count">
                  {t.estimated_rows > 0 ? `~${t.estimated_rows.toLocaleString()} rows` : "row count unknown"}
                </span>
              </button>
            ))}
          </div>
          <div className="browse-grid">
            {pageLoading ? (
              <div className="muted">{hasLoadedRows ? "Loading rows..." : "Preparing a temporary database and loading this table. The first table can take a while..."}</div>
            ) : page ? (
              <>
                <div className="page-head">
                  <h4>
                    {active?.schema}.{active?.name}
                  </h4>
                  <div className="row">
                    <button disabled={offset === 0} onClick={() => loadTable(active!, Math.max(0, offset - LIMIT))}>
                      &lt; Previous
                    </button>
                    <span className="small muted">
                      rows {offset + 1}-{offset + page.rows.length}
                    </span>
                    <button
                      disabled={page.rows.length < LIMIT || pageLoading}
                      onClick={() => loadTable(active!, offset + LIMIT)}
                    >
                      Next &gt;
                    </button>
                  </div>
                </div>
                <div className="grid-scroll">
                  <table className="table">
                    <thead>
                      <tr>
                        {page.columns.map((c) => (
                          <th key={c}>{c}</th>
                        ))}
                      </tr>
                    </thead>
                    <tbody>
                      {page.rows.map((r, i) => (
                        <tr key={i}>
                          {r.map((cell, j) => (
                            <td key={j}>{cell === null ? <span className="muted">NULL</span> : cell}</td>
                          ))}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </>
            ) : (
              <div className="muted">Select a table to view rows.</div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function RestoreInto({ dump }: { dump: string }) {
  const conns = useAsync<ConnectionRecord[]>(listConnections, []);
  const [targetId, setTargetId] = useState<number | "">("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ErrorReport | null>(null);
  const [done, setDone] = useState(false);

  const target = conns.data?.find((c) => c.id === targetId);
  const canRestore = target && confirm.trim() === target.config.name && !busy;

  async function go() {
    if (targetId === "") return;
    setBusy(true);
    setError(null);
    setDone(false);
    try {
      await restoreBackup(dump, targetId as number, confirm);
      setDone(true);
    } catch (e) {
      setError(e as ErrorReport);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="card result failed">
      <h3>! Restore into a database (destructive)</h3>
      <p className="small">
        This will restore the backup into the selected database, overwriting existing objects
        (<code>--clean --if-exists</code>). Never restore into production unless you are certain.
      </p>
      <label>Target database</label>
      <select value={targetId} onChange={(e) => setTargetId(Number(e.target.value))}>
        <option value="">Select a target...</option>
        {conns.data?.map((c) => (
          <option key={c.id} value={c.id}>
            {c.config.name} ({c.config.host}/{c.config.database})
          </option>
        ))}
      </select>

      {target && (
        <>
          <label>
            Type <b>{target.config.name}</b> to confirm
          </label>
          <input value={confirm} onChange={(e) => setConfirm(e.target.value)} placeholder={target.config.name} />
        </>
      )}

      <div className="row">
        <button className="primary" onClick={go} disabled={!canRestore}>
          {busy ? "Restoring..." : "Restore"}
        </button>
      </div>
      {done && <div className="small" style={{ color: "var(--ok)" }}>Restore completed.</div>}
      {error && <ErrorPanel error={error} />}
    </div>
  );
}
