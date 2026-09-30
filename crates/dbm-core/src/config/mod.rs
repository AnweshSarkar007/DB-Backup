//! Configuration: sanitized connection metadata, URI parsing, and application paths.

pub mod connection;
pub mod paths;

pub use connection::{parse_connection_uri, slugify, ConnectionConfig, ParsedConnection, SslMode};
pub use paths::{validate_destination_path, AppPaths};
