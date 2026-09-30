-- Database Backup Manager — initial SQLite schema.
--
-- NOTE: This migration is APPLIED starting in Phase 2 (SQLite + credentials). It is included now so
-- the schema is reviewable alongside the Phase-1 domain model in `crates/dbm-core/src/model.rs`.
-- Invariant: secrets are NEVER stored here — only `secret_reference` values pointing at the OS
-- credential store (see docs/SECURITY.md).

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS schema_migrations (
    version     INTEGER PRIMARY KEY,
    applied_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS connections (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    uuid                     TEXT NOT NULL UNIQUE,
    name                     TEXT NOT NULL,
    host                     TEXT NOT NULL,
    port                     INTEGER NOT NULL DEFAULT 5432,
    database_name            TEXT NOT NULL,
    username                 TEXT NOT NULL,
    ssl_mode                 TEXT NOT NULL DEFAULT 'require',
    secret_reference         TEXT NOT NULL,          -- reference into OS credential store, NOT a password
    postgres_major_version   INTEGER,
    created_at               TEXT NOT NULL,
    updated_at               TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS destinations (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    uuid                TEXT NOT NULL UNIQUE,
    name                TEXT NOT NULL,
    type                TEXT NOT NULL,               -- 'local' | 'google_drive' | ...
    configuration_json  TEXT NOT NULL,               -- non-secret config (path, folder id, account)
    secret_reference    TEXT,                        -- e.g. OAuth token reference; NULL for local
    enabled             INTEGER NOT NULL DEFAULT 1,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS backup_policies (
    id                     INTEGER PRIMARY KEY AUTOINCREMENT,
    uuid                   TEXT NOT NULL UNIQUE,
    connection_id          INTEGER NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
    name                   TEXT NOT NULL,
    schedule_json          TEXT,                     -- cron-like / weekly-day-time
    retention_policy_json  TEXT,
    enabled                INTEGER NOT NULL DEFAULT 1,
    created_at             TEXT NOT NULL,
    updated_at             TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS policy_destinations (
    policy_id       INTEGER NOT NULL REFERENCES backup_policies(id) ON DELETE CASCADE,
    destination_id  INTEGER NOT NULL REFERENCES destinations(id) ON DELETE CASCADE,
    PRIMARY KEY (policy_id, destination_id)
);

CREATE TABLE IF NOT EXISTS backup_runs (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    uuid                    TEXT NOT NULL UNIQUE,
    policy_id               INTEGER REFERENCES backup_policies(id) ON DELETE SET NULL,
    connection_id           INTEGER REFERENCES connections(id) ON DELETE SET NULL,
    trigger_type            TEXT NOT NULL,           -- 'manual' | 'scheduled'
    database_label          TEXT,                    -- display name, even for ad-hoc runs
    started_at              TEXT,
    completed_at            TEXT,
    status                  TEXT NOT NULL,           -- BackupState
    filename                TEXT,
    local_temp_path         TEXT,
    file_size               INTEGER,
    sha256                  TEXT,
    postgres_major_version  INTEGER,
    verification_status     TEXT,
    error_code              TEXT,
    error_message           TEXT
);

CREATE TABLE IF NOT EXISTS backup_destination_runs (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    backup_run_id   INTEGER NOT NULL REFERENCES backup_runs(id) ON DELETE CASCADE,
    destination_id  INTEGER REFERENCES destinations(id) ON DELETE SET NULL,
    destination_name TEXT,
    started_at      TEXT,
    completed_at    TEXT,
    status          TEXT NOT NULL,                   -- completed | failed | skipped
    stored_path     TEXT,
    remote_file_id  TEXT,
    bytes_written   INTEGER,
    sha256          TEXT,
    error_code      TEXT,
    error_message   TEXT
);

CREATE TABLE IF NOT EXISTS job_logs (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    backup_run_id   INTEGER REFERENCES backup_runs(id) ON DELETE CASCADE,
    timestamp       TEXT NOT NULL,
    level           TEXT NOT NULL,
    component       TEXT NOT NULL,
    message         TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS app_settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_backup_runs_connection ON backup_runs(connection_id);
CREATE INDEX IF NOT EXISTS idx_backup_runs_started    ON backup_runs(started_at);
CREATE INDEX IF NOT EXISTS idx_dest_runs_run          ON backup_destination_runs(backup_run_id);
CREATE INDEX IF NOT EXISTS idx_job_logs_run           ON job_logs(backup_run_id);
