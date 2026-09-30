//! Small shared helpers.

/// Current local time as an RFC3339 string with offset (e.g. `2026-09-30T11:35:24+05:30`).
pub fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339()
}

/// A fresh v4 UUID string.
pub fn new_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}
