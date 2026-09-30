//! Google Drive integration (Section 14): OAuth desktop flow and a minimal Drive v3 client.

pub mod drive;
pub mod oauth;

pub use oauth::{OAuthConfig, Tokens, DRIVE_FILE_SCOPE};
