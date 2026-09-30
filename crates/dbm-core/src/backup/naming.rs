//! Predictable, sortable backup filenames (Section 11): `<db-slug>_<YYYY-MM-DD>_<HHMMSS>.dump`
//! with a matching `.json` manifest.

use crate::config::slugify;
use chrono::{DateTime, Local};

/// Build the base filename (no extension) for a backup taken at `when`.
///
/// `glasafe-production_2026-09-30_113524`
pub fn base_name(db_name: &str, when: DateTime<Local>) -> String {
    format!("{}_{}", slugify(db_name), when.format("%Y-%m-%d_%H%M%S"))
}

/// The dump filename: `<base>.dump`.
pub fn dump_filename(db_name: &str, when: DateTime<Local>) -> String {
    format!("{}.dump", base_name(db_name, when))
}

/// The manifest filename: `<base>.json`.
pub fn manifest_filename(db_name: &str, when: DateTime<Local>) -> String {
    format!("{}.json", base_name(db_name, when))
}

/// Derive the manifest filename that pairs with a given dump filename.
pub fn manifest_for_dump(dump_filename: &str) -> String {
    match dump_filename.strip_suffix(".dump") {
        Some(base) => format!("{base}.json"),
        None => format!("{dump_filename}.json"),
    }
}

/// Parse the timestamp embedded in a dump filename (`<slug>_YYYY-MM-DD_HHMMSS.dump`).
/// Returns the naive local time, or None if the name does not match the pattern.
pub fn parse_timestamp(filename: &str) -> Option<chrono::NaiveDateTime> {
    let base = filename.strip_suffix(".dump").unwrap_or(filename);
    // The timestamp is the last two underscore-separated groups: `YYYY-MM-DD` and `HHMMSS`.
    let parts: Vec<&str> = base.rsplitn(3, '_').collect(); // [HHMMSS, YYYY-MM-DD, slug...]
    if parts.len() < 2 {
        return None;
    }
    let hms = parts[0];
    let ymd = parts[1];
    let combined = format!("{ymd}_{hms}");
    chrono::NaiveDateTime::parse_from_str(&combined, "%Y-%m-%d_%H%M%S").ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed() -> DateTime<Local> {
        // 2026-09-30 11:35:24 local
        Local.with_ymd_and_hms(2026, 9, 30, 11, 35, 24).unwrap()
    }

    #[test]
    fn dump_name_matches_spec_pattern() {
        assert_eq!(
            dump_filename("Glasafe Production", fixed()),
            "glasafe-production_2026-09-30_113524.dump"
        );
    }

    #[test]
    fn manifest_name_matches_dump() {
        assert_eq!(
            manifest_filename("Glasafe Production", fixed()),
            "glasafe-production_2026-09-30_113524.json"
        );
        assert_eq!(
            manifest_for_dump("glasafe-production_2026-09-30_113524.dump"),
            "glasafe-production_2026-09-30_113524.json"
        );
    }

    #[test]
    fn names_are_lexically_sortable_by_time() {
        let earlier = Local.with_ymd_and_hms(2026, 9, 30, 9, 0, 0).unwrap();
        let later = Local.with_ymd_and_hms(2026, 9, 30, 14, 0, 0).unwrap();
        let a = dump_filename("db", earlier);
        let b = dump_filename("db", later);
        assert!(a < b, "{a} should sort before {b}");
    }
}
