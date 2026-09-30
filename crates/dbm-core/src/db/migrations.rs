//! Embedded SQL migrations, applied in order and tracked in `schema_migrations` (spec Rule 6).

/// (version, sql) pairs. Add new migrations here; never edit an already-shipped one.
pub const MIGRATIONS: &[(i64, &str)] =
    &[(1, include_str!("../../../../migrations/0001_init.sql"))];
