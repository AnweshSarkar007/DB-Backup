# Restore & Inspection (Roadmap)

Restore is deliberately staged **after** the backup pipeline is proven. Three distinct capabilities,
in increasing risk order:

## 1. Inspect (Phase 1 groundwork / Phase 9 UI)

Read a dump's table of contents without touching any database:

```
pg_restore -l backup.dump
```

Already available via the worker:

```powershell
backup-worker.exe inspect --dump path\to\backup.dump --image postgres:17
```

It lists schemas, tables, sequences, indexes, constraints and functions. No database is modified.
`dbm_core::BackupRunner::inspect` parses this into `ArchiveEntry` items for the future UI.

## 2. Browse a historical backup (Phase 10)

A safe, read-only exploration of an old backup:

1. Create a **temporary** PostgreSQL Docker container (localhost-only, no published ports).
2. Create a temporary database and `pg_restore` the dump into it.
3. Show a table list with row counts; open any table in a **paginated** grid (never load millions of
   rows into the frontend — SQL `LIMIT/OFFSET` or keyset pagination).
4. On close (or after a stale timeout / crash), drop and remove the temporary container.

This reuses `docker::container_runner` (already used by the round-trip test) plus a small query layer.

## 3. Restore into a database (Phase 11)

The only destructive operation, guarded heavily:

- Test the **target** connection and show its details.
- Warn that data may be modified; require an explicit **typed confirmation** of the target name.
- Never silently restore into production.
- Write an **audit record** of every restore.

## Verification vs. restore-tested

These are different concepts tracked separately:

- **Archive Verified** — `pg_restore -l` succeeded (Phase 1, done for every backup).
- **Restore Tested** — the dump was actually restored into a throwaway container and passed
  integrity checks (the round-trip test does this today; a per-backup "Test Restore" action is a
  later feature).
