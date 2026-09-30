# Database Backup Manager

A Windows 11 desktop application that lets non-technical users securely back up
Supabase/PostgreSQL databases to multiple destinations (local disks now; Google Drive and more
later). Backups run through `pg_dump` inside Docker, are verified (`pg_restore -l`), checksummed
(SHA-256), and fanned out to every configured destination — once — with per-destination status.

> **Status: Phase 0 + Phase 1 complete.** The foundation (Rust workspace, trait abstractions,
> security/logging) and a **working, verified backup engine** are in place and tested end-to-end
> against real Docker. UI, SQLite persistence, scheduling, Google Drive, retention and restore are
> later phases — see [the roadmap](#roadmap).

## What works today

- Connect to PostgreSQL/Supabase over TLS and detect the server version.
- Pick the matching `postgres:<major>` Docker image (auto-pull if missing).
- `pg_dump -Fc` → **verify with `pg_restore -l`** → **SHA-256** → **manifest.json**.
- Fan the *single verified dump* out to one or more local folders, tracking each individually
  (full / **partial** / failed).
- Run it from the **worker CLI** (headless, no GUI) or the **minimal GUI** ("Backup Now").
- Round-trip integration test proves a dump restores into a fresh database with matching row
  counts and values.

## Architecture at a glance

```
DatabaseBackupManager.exe (Tauri + React)   backup-worker.exe (headless, Task Scheduler-ready)
                     \                      /
                      \                    /
                        crates/dbm-core   ← ALL logic (no UI/Tauri deps): connect, dump,
                                            verify, hash, manifest, fan-out, locking, secrets
                                              |
                          Docker CLI → postgres:<major> → pg_dump / pg_restore
```

Both executables are thin shells over `dbm-core`, so a scheduled backup behaves identically to a
manual one. See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Quick start (developers)

Prerequisites: **Rust** (stable), **Node 18+**, **Docker Desktop** (running), Windows 11.

```powershell
# 1. Unit tests (fast, no Docker)
cargo test -p dbm-core --lib

# 2. The critical round-trip test (needs Docker; ~1–2 min, pulls postgres:16)
cargo test -p dbm-core --test roundtrip -- --ignored --nocapture

# 3. Worker CLI
cargo run -p dbm-worker -- doctor
$env:DBM_PG_PASSWORD = "..."                       # secret for the example spec
cargo run -p dbm-worker -- test-connection --spec examples/job.example.json
cargo run -p dbm-worker -- run             --spec examples/job.example.json

# 4. Live smoke test against YOUR Supabase DB (read-only; needs Docker)
$env:DBM_LIVE_PG_URL = "postgresql://postgres:PW@db.xxx.supabase.co:5432/postgres"
cargo test -p dbm-core --test live_smoke -- --ignored --nocapture

# 5. GUI (minimal Phase-1 shell with a working "Backup Now")
npm install
npm run tauri dev
```

See [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) for details and [`docs/SECURITY.md`](docs/SECURITY.md)
for the threat model.

## Roadmap

| Phase | Scope | Status |
|------:|-------|--------|
| 0 | Repo, workspace, traits, docs, tests | ✅ |
| 1 | Verified pg_dump backup engine (CLI + GUI) | ✅ |
| 2 | SQLite schema + Windows Credential Manager persistence | ✅ |
| 3 | Full desktop UI (Dashboard, Databases, History, Details) | ✅ |
| 4 | Local destinations UX + fan-out + partial success | ✅ |
| 5 | Worker `--job-id` (Task Scheduler-ready) | ✅ |
| 6 | Windows Task Scheduler integration | ✅ |
| 7 | Google Drive OAuth (multiple accounts) — see [docs/GOOGLE_DRIVE.md](docs/GOOGLE_DRIVE.md) | ✅ |
| 8 | Retention policies | ✅ |
| 9 | Archive inspection UI | ✅ |
| 10 | Temporary restore + browse | ✅ |
| 11 | Restore to a target database | ✅ |
| — | NSIS installer, code-signing, auto-update | ⏳ |

