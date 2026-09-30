# Security & Threat Model

This application can hold credentials to **production** databases and cloud storage. It is treated
as security-sensitive software. This document records the threats we design against and which
mitigations are implemented in Phase 1 versus deferred to later phases.

## Principles

1. **No raw secrets in SQLite.** SQLite stores only a *reference*; the secret lives in the OS
   credential store. (Phase 1: secrets go to Windows Credential Manager via `WinCredStore`; SQLite
   itself lands in Phase 2.)
2. **No secrets in logs or errors.** All tool output and error detail is scrubbed at the source
   (`redact` module) before it can reach a log sink or the UI.
3. **No secrets in argv.** The DB password is passed to containers via a temporary `--env-file`
   (restricted-ACL working dir), deleted immediately after the container starts — never on a command
   line visible in the host process list.
4. **No shell string building.** Every external process is invoked with a structured argument
   vector, so untrusted input cannot inject shell commands.
5. **TLS to PostgreSQL**, validated against the OS trust store (native-tls / SChannel).
6. **Least privilege / ephemerality.** Containers run `--rm`, publish no ports, and are given a
   deterministic name so an abandoned one can be force-removed.

## Threats and mitigations

| Threat | Mitigation | Phase |
|---|---|---|
| **Database credential theft** (from disk) | Passwords stored in Windows Credential Manager, never in SQLite or files; only a reference is persisted. | 1 (store) / 2 (wiring) |
| **OAuth token theft** | Google refresh tokens and the OAuth client secret are stored in Windows Credential Manager (refs `gdrive:<id>`, `google:client_secret`); SQLite holds only references. Access tokens are fetched on demand and never persisted. Scope limited to `drive.file` (app-created files only). PKCE + loopback redirect; `state` validated. | 7 ✅ |
| **Secrets leaking into logs** | `redact::redact_connection_uri` / `redact_secret` / `redact_env_assignments`; docker output scrubbed with the known password before logging. Unit-tested. | 1 |
| **Command injection** | `tokio::process::Command` with argument vectors; no shell, no string concatenation of untrusted input. | 1 |
| **Password on command line** | Temporary `--env-file` (restricted ACL), removed right after run; `pg_dump -w` never prompts. | 1 |
| **Path traversal** | `config::validate_destination_path` rejects relative paths and `..`; destination filenames rejected if they contain separators or `..`. | 1 |
| **Malicious/corrupted backup file** | Every dump is verified with `pg_restore -l` before distribution; corrupt dumps are never copied out. SHA-256 recorded and re-verified at each destination. | 1 |
| **Incomplete upload** | Each destination copy is re-verified (size + SHA-256); a bad copy is deleted and the destination marked failed → partial success, original kept. | 1 |
| **Compromised/rogue destination** | Destinations are isolated behind the `StorageDestination` trait; one failing/hostile destination cannot corrupt others or the source dump. | 1 |
| **Stolen laptop** | Secrets in the OS credential store are protected by the user's Windows login (DPAPI-backed). Temp dumps live in a restricted-ACL dir and are cleaned after success. Backup **encryption** (AES-256-GCM) is designed via `ArtifactProcessor` for a later phase. | 1 (design) / later |
| **Malicious local user** | Temp working dir ACL restricted to the current user + SYSTEM + Administrators (`icacls`, best-effort); `%LOCALAPPDATA%` fallback is user-private by default. | 1 |
| **Concurrent/duplicate runs corrupting state** | OS advisory file lock per job; crash-safe (the OS releases the lock if the process dies). | 1 |
| **Exposed temporary restore DB** (browse feature) | Temporary restore containers will bind to localhost only and be torn down after the session. | 10 |

## Explicit non-goals / clarifications

- **`pg_dump` does not back up Supabase Storage bucket files.** The app makes this explicit; a
  full-project backup (DB + roles + storage objects) is a future extension behind its own interface
  (Section 38), never implied by a database backup.
- Encryption is **not** mandatory for the MVP but the seam (`ArtifactProcessor`) exists so it can be
  added between dump and upload without touching either.

## Reporting

Security issues should be reported privately to the maintainers, describing the class of problem
rather than a working exploit.
