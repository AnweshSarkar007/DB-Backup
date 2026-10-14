# Bundled PostgreSQL 17 client tools

This directory is the deterministic installer resource root for the native PostgreSQL 17
client tools. The repository intentionally does not contain fabricated or redistributable
binaries. Run `scripts/stage-postgresql17.ps1` on a packaging machine to download the pinned
official Windows x64 binary archive and stage `pg_dump.exe`, `pg_restore.exe`, `psql.exe`, and
required DLLs under `bin/`.

The application resolves this directory relative to the installed GUI executable:
`resources/postgresql/17/bin`. Docker is used only when all three native tools are unavailable.
