//! Retention policies (Section 25).
//!
//! The **selection logic is pure and heavily tested** (the spec insists on verifying the algorithm
//! before enabling deletion). Applying it to a filesystem destination is a thin layer on top.
//!
//! Rules:
//! - `keep_last = N`: keep the N most-recent backups.
//! - `keep_days = D`: keep backups newer than D days.
//! - If both are set, a backup is kept when it satisfies **either** (union of kept sets).
//! - The single newest backup is **never** deleted, whatever the policy says.
//! - Retention is applied **per destination** independently (a failure elsewhere never deletes the
//!   newest usable copy here).

use crate::backup::naming;
use crate::destinations::LocalDestination;
use crate::error::Result;
use crate::traits::StorageDestination;
use chrono::{Duration, NaiveDateTime};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A retention policy (stored as a policy's `retention_policy_json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RetentionPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_last: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_days: Option<i64>,
}

impl RetentionPolicy {
    pub fn is_noop(&self) -> bool {
        self.keep_last.is_none() && self.keep_days.is_none()
    }
}

/// Given item timestamps, return the indices that should be deleted under `policy` as of `now`.
///
/// `times[i]` is the creation time of item `i`. The newest item is always retained.
pub fn select_expired(
    times: &[NaiveDateTime],
    policy: &RetentionPolicy,
    now: NaiveDateTime,
) -> Vec<usize> {
    if policy.is_noop() || times.len() <= 1 {
        return Vec::new();
    }

    // Order indices newest-first.
    let mut order: Vec<usize> = (0..times.len()).collect();
    order.sort_by(|&a, &b| times[b].cmp(&times[a]));

    let mut keep = vec![false; times.len()];
    // Always keep the newest.
    keep[order[0]] = true;

    if let Some(n) = policy.keep_last {
        for &idx in order.iter().take(n.max(1)) {
            keep[idx] = true;
        }
    }
    if let Some(days) = policy.keep_days {
        let cutoff = now - Duration::days(days);
        for (idx, &t) in times.iter().enumerate() {
            if t >= cutoff {
                keep[idx] = true;
            }
        }
    }

    (0..times.len()).filter(|&i| !keep[i]).collect()
}

/// A backup file discovered at a destination.
#[derive(Debug, Clone)]
pub struct BackupFile {
    pub dump_name: String,
    pub timestamp: NaiveDateTime,
}

/// List backup files for a given database slug in a local folder, parsing timestamps from names.
pub fn list_local_backups(dir: &Path, slug: &str) -> Result<Vec<BackupFile>> {
    let mut out = Vec::new();
    let prefix = format!("{slug}_");
    if !dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(&prefix) && name.ends_with(".dump") {
            if let Some(ts) = naming::parse_timestamp(&name) {
                out.push(BackupFile { dump_name: name, timestamp: ts });
            }
        }
    }
    Ok(out)
}

/// Apply a retention policy to a local destination for one database slug. Deletes expired dumps and
/// their manifests. Returns the names deleted.
pub async fn apply_local(
    dest: &LocalDestination,
    dir: &Path,
    slug: &str,
    policy: &RetentionPolicy,
    now: NaiveDateTime,
) -> Result<Vec<String>> {
    let files = list_local_backups(dir, slug)?;
    let times: Vec<NaiveDateTime> = files.iter().map(|f| f.timestamp).collect();
    let expired = select_expired(&times, policy, now);

    let mut deleted = Vec::new();
    for idx in expired {
        let f = &files[idx];
        dest.delete(&f.dump_name).await?;
        let manifest = naming::manifest_for_dump(&f.dump_name);
        let _ = dest.delete(&manifest).await;
        deleted.push(f.dump_name.clone());
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    // `t(n)` = a base date plus `n` days, so large offsets like t(100) stay valid.
    fn t(day_offset: i64) -> NaiveDateTime {
        let base = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(2, 0, 0)
            .unwrap();
        base + Duration::days(day_offset)
    }

    #[test]
    fn noop_policy_deletes_nothing() {
        let times = vec![t(1), t(2), t(3)];
        assert!(select_expired(&times, &RetentionPolicy::default(), t(10)).is_empty());
    }

    #[test]
    fn keep_last_keeps_newest_n() {
        // days 1..5; keep_last=2 → keep days 5,4 → delete days 1,2,3 (indices 0,1,2)
        let times = vec![t(1), t(2), t(3), t(4), t(5)];
        let policy = RetentionPolicy { keep_last: Some(2), keep_days: None };
        let mut del = select_expired(&times, &policy, t(10));
        del.sort();
        assert_eq!(del, vec![0, 1, 2]);
    }

    #[test]
    fn keep_days_keeps_recent() {
        // now = day 10; keep_days=5 → cutoff day 5 → keep days >=5 (day 5,6), delete day 1..4
        let times = vec![t(1), t(4), t(5), t(6)];
        let policy = RetentionPolicy { keep_last: None, keep_days: Some(5) };
        let mut del = select_expired(&times, &policy, t(10));
        del.sort();
        assert_eq!(del, vec![0, 1]); // days 1 and 4
    }

    #[test]
    fn union_of_both_constraints() {
        // days 1..6; keep_last=1 (day6) OR keep_days=2 from now day8 → cutoff day6 → keep day6.
        // Also union: keep_last keeps day6; keep_days keeps day6,7.. none older. Delete 1..5.
        let times = vec![t(1), t(2), t(3), t(4), t(5), t(6)];
        let policy = RetentionPolicy { keep_last: Some(1), keep_days: Some(2) };
        let mut del = select_expired(&times, &policy, t(8));
        del.sort();
        assert_eq!(del, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn never_deletes_the_only_backup() {
        let times = vec![t(1)];
        let policy = RetentionPolicy { keep_last: Some(0), keep_days: Some(1) };
        assert!(select_expired(&times, &policy, t(100)).is_empty());
    }

    #[test]
    fn never_deletes_newest_even_if_expired_by_days() {
        // Single-ish: 2 items both older than cutoff; keep_days would delete both, but newest kept.
        let times = vec![t(1), t(2)];
        let policy = RetentionPolicy { keep_last: None, keep_days: Some(1) };
        let del = select_expired(&times, &policy, t(100));
        assert_eq!(del, vec![0]); // only the older one; newest (index 1) kept
    }
}
