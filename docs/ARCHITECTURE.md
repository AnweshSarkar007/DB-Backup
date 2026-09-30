# Architecture

## Goals

- **Non-technical users** can back up production PostgreSQL/Supabase databases safely.
- **Scheduled backups run without the GUI** — a headless worker does the work.
- **Testable core**: all logic sits in a library crate with no UI/Tauri dependency (spec Rule 8).
- **Extensible seams**: adding a cloud destination, a scheduler, or a restore flow later must not
  require rewriting the backup engine (spec Section 5).

## Two processes

| Executable | Crate | Responsibility |
|---|---|---|
| `DatabaseBackupManager.exe` | `src-tauri` | UI shell: configuration, launching manual backups, viewing status. Thin commands over `dbm-core`. |
| `backup-worker.exe` | `crates/dbm-worker` | Executes **one** backup job headlessly and returns an exit code. Invoked by the user or (Phase 6) Windows Task Scheduler. |

Both depend on **`crates/dbm-core`**, which contains every piece of business logic. This guarantees
a scheduled backup and a manual backup take the exact same code path.

```
                         crates/dbm-core
  ┌───────────────────────────────────────────────────────────────┐
  │ config      connection parse/sanitize, paths (ProgramData)     │
  │ traits      DatabaseSource, BackupRunner, StorageDestination,  │
  │             CredentialStore, BackupRepository, Scheduler,      │
  │             RestoreService, ArtifactProcessor                  │
  │ database    PostgresSource  (tokio-postgres + native-tls)      │
  │ docker      client · image_manager · container_runner ·       │
  │             health · runner (DockerPgRunner: BackupRunner)     │
  │ destinations LocalDestination (StorageDestination)             │
  │ credentials WinCredStore · EnvStore · MemoryStore · AutoStore  │
  │ backup      engine (state machine) · naming · hash · manifest  │
  │             · lock (crash-safe advisory lock)                  │
  │ error       typed BackupError + ErrorReport (UI-facing)        │
  │ redact      secret scrubbing for logs/errors                   │
  │ logging     tracing (console + rolling file)                   │
  └───────────────────────────────────────────────────────────────┘
        │                         │                         │
     src-tauri                dbm-worker                 tests/
   (GUI commands)          (CLI: doctor/run/…)      (unit + roundtrip + live)
```

## The backup pipeline (state machine)

`Queued → Connecting → Preparing → Dumping → Verifying → Hashing → Distributing → Completed`
(with `PartialSuccess`, `Failed`, `Cancelled` as alternative terminal states).

1. **Lock** the job (OS advisory file lock → crash-safe; concurrent duplicates are refused).
2. **Connect** and resolve the password from a `CredentialStore` at the last moment.
3. **Detect** the server version (`SHOW server_version_num`).
4. **Docker** readiness + ensure the matching `postgres:<major>` image (auto-pull).
5. **Dump** with `pg_dump -Fc` into a per-run working directory.
6. **Verify** with `pg_restore -l` — *a file existing is not success*. A corrupt dump is never
   distributed.
7. **Hash** (streaming SHA-256).
8. **Distribute** the *single* verified dump to every enabled destination, recording each result.
9. **Manifest** written with final per-destination statuses.
10. **Finalize** the terminal state and clean the temp dir (kept on partial/failure for retry).

## Key decisions

- **Docker for pg tooling.** No host `pg_dump` needed; the image major version matches the server,
  avoiding client/server mismatch.
- **native-tls (SChannel) for the connection test**, so Windows' OS trust store validates Supabase
  certificates with no bundled root store to maintain.
- **Password never in argv.** It is written to a temporary `--env-file` (in a restricted-ACL work
  dir) and deleted right after the container starts.
- **Structured argument vectors** for every Docker call — no shell string interpolation.
- **`#[async_trait]` only where trait objects are needed** (`StorageDestination`, for heterogeneous
  fan-out); other traits use native `async fn` for zero-cost concrete dispatch.
- **Every run yields a `RunResult`** (success, partial, or failure) rather than a bare error, so
  history always has a record and the worker can map state → exit code (`0` / `2` / `1`).

## Extension points (future phases)

- `StorageDestination` → `GoogleDriveDestination`, `S3Destination`, … (Phase 7+). One new arm in
  `destinations::build`.
- `BackupRepository` → SQLite implementation (Phase 2). Currently `NoopRepository`.
- `Scheduler` → Windows Task Scheduler (Phase 6).
- `ArtifactProcessor` → compression, then AES-256-GCM encryption (Section 29), inserted between
  dump and upload without touching either.
- Worker `--job-id <n>` hydrates a `JobSpec` from SQLite and calls the same executor (Phase 2/5).
