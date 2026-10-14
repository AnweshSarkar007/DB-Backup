# Development

## Prerequisites

- **Rust** stable (1.77+), with `cargo`.
- **Node.js** 18+ and npm.
- **Docker Desktop**, running only for the browse-only backup browser and as the backup/restore fallback.
- **Bundled PostgreSQL 17 client tools** for the primary backup, verify, inspect, and restore path.
  Packaging machines stage them with `scripts/stage-postgresql17.ps1`; the repository does not
  fabricate or include vendor binaries.
- **Windows 11** (primary target). WebView2 runtime (preinstalled on Win11).

## Layout

```
Cargo.toml            workspace: crates/dbm-core, crates/dbm-worker, src-tauri
crates/dbm-core/      all business logic (library) + tests
crates/dbm-worker/    backup-worker.exe (headless CLI)
src-tauri/            DatabaseBackupManager.exe (Tauri 2 GUI backend)
src/                  React + TypeScript frontend
migrations/           SQL migrations (used from Phase 2)
docs/                 architecture, security, restore, development
examples/             example job spec
```

## Common commands

```powershell
# Rust unit tests (fast, no Docker)
cargo test -p dbm-core --lib

# Integration tests (need Docker; ignored by default)
cargo test -p dbm-core --test roundtrip  -- --ignored --nocapture
cargo test -p dbm-core --test live_smoke -- --ignored --nocapture   # needs DBM_LIVE_PG_URL

# Whole workspace build
cargo build --workspace

# Formatting / lints
cargo fmt --all
cargo clippy --workspace --all-targets

# Worker
cargo run -p dbm-worker -- doctor
cargo run -p dbm-worker -- run --spec examples/job.example.json

# Frontend type-check / build / GUI
npx tsc --noEmit
npm run build
npm run tauri dev
```

## Environment variables

| Var | Purpose |
|---|---|
| `DBM_PG_PASSWORD` | Password for `examples/job.example.json` (`secret_reference: env:DBM_PG_PASSWORD`). |
| `DBM_LIVE_PG_URL` | Full Supabase/PG URI for the live smoke test. |
| `DBM_DATA_DIR` | Override the app data dir (default `%ProgramData%\DatabaseBackupManager`). |
| `DBM_DOCKER_PATH` | Path to the docker executable (default `docker`). |
| `DBM_LOG` | Tracing filter, e.g. `dbm_core=debug`. |

## Job spec (Phase 1)

The worker reads a JSON job spec (see `examples/job.example.json`). It contains **sanitized**
connection metadata and a `secret_reference` — never a password. References:

- `env:NAME` → read the password from environment variable `NAME` (CI/headless).
- anything else → read from Windows Credential Manager under service `DatabaseBackupManager`.

## Known environment constraint (this machine)

Some locked-down Windows machines run an **Application Control policy (WDAC/AppLocker)** that blocks
loading unsigned native `.node` binaries. This prevents Vite/Rollup/esbuild from running, so
`npm run build` / `npm run tauri dev` fail with `ERR_DLOPEN_FAILED: An Application Control policy has
blocked this file`. The Rust workspace and `npx tsc --noEmit` are unaffected. To run the GUI, use a
machine without that policy (or have the native binaries allow-listed).

## Coding rules (from the spec)

- No business logic in React components or in Tauri commands — it lives in `dbm-core`.
- No giant `main.rs`; use modules.
- Never commit credentials; never hard-code hosts/passwords/paths (use env/config).
- Migrations for SQLite; UUIDs for externally referenced ids.
- Typed errors surfaced to the frontend; prefer trait-based seams for testability.
