//! Database sources. Phase 1: PostgreSQL / Supabase.

pub mod postgres;

pub use postgres::{major_from_num, PostgresSource};
