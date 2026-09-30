//! Windows Task Scheduler integration (Section 19).
//!
//! The GUI registers a scheduled task that invokes `backup-worker.exe run --job-id <policy_id>`, so
//! backups fire even when `DatabaseBackupManager.exe` is closed. We drive `schtasks.exe` with
//! structured arguments (no shell string building except the single `/TR` command string, whose only
//! variable parts are a quoted exe path and a numeric policy id — no injection surface).
//!
//! Phase-1 scope note: tasks are created to run as the current user *when logged on*. "Run whether
//! logged on or not" needs stored Windows credentials and is a later enhancement.

use crate::error::{BackupError, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How often a policy runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Frequency {
    Daily,
    Weekly,
}

/// A backup schedule (stored as a policy's `schedule_json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    pub frequency: Frequency,
    /// 24-hour local time, `HH:MM`.
    pub time: String,
    /// Day for weekly schedules: one of SUN,MON,TUE,WED,THU,FRI,SAT.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day_of_week: Option<String>,
}

impl Schedule {
    /// Validate fields (basic sanity: HH:MM and a valid weekday for weekly).
    pub fn validate(&self) -> Result<()> {
        let ok_time = self.time.len() == 5
            && self.time.as_bytes()[2] == b':'
            && self.time[..2].parse::<u8>().map(|h| h < 24).unwrap_or(false)
            && self.time[3..].parse::<u8>().map(|m| m < 60).unwrap_or(false);
        if !ok_time {
            return Err(BackupError::SchedulerFailed {
                detail: format!("invalid time '{}': expected HH:MM", self.time),
            });
        }
        if self.frequency == Frequency::Weekly {
            let d = self.day_of_week.as_deref().unwrap_or("");
            if !["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"].contains(&d) {
                return Err(BackupError::SchedulerFailed {
                    detail: format!("invalid day_of_week '{d}' for weekly schedule"),
                });
            }
        }
        Ok(())
    }

    /// A human summary, e.g. "Every Sunday at 02:00".
    pub fn describe(&self) -> String {
        match self.frequency {
            Frequency::Daily => format!("Every day at {}", self.time),
            Frequency::Weekly => {
                let day = match self.day_of_week.as_deref() {
                    Some("SUN") => "Sunday",
                    Some("MON") => "Monday",
                    Some("TUE") => "Tuesday",
                    Some("WED") => "Wednesday",
                    Some("THU") => "Thursday",
                    Some("FRI") => "Friday",
                    Some("SAT") => "Saturday",
                    _ => "?",
                };
                format!("Every {day} at {}", self.time)
            }
        }
    }
}

/// Status of a registered task (best-effort parse of `schtasks /Query`).
#[derive(Debug, Clone, Default, Serialize)]
pub struct TaskStatus {
    pub exists: bool,
    pub next_run: Option<String>,
    pub last_run: Option<String>,
    pub last_result: Option<String>,
}

/// Drives Windows Task Scheduler for our policies.
#[derive(Debug, Clone)]
pub struct TaskScheduler {
    worker_exe: PathBuf,
}

impl TaskScheduler {
    pub fn new(worker_exe: PathBuf) -> Self {
        TaskScheduler { worker_exe }
    }

    /// Task name for a policy (grouped under a `DBM\` folder).
    pub fn task_name(policy_uuid: &str) -> String {
        format!("DBM\\policy-{policy_uuid}")
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::process::Command;

    fn schtasks(args: &[&str]) -> Result<(bool, String)> {
        let out = Command::new("schtasks")
            .args(args)
            .output()
            .map_err(|e| BackupError::SchedulerFailed { detail: format!("schtasks: {e}") })?;
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok((out.status.success(), text))
    }

    impl TaskScheduler {
        /// Create or replace the scheduled task for a policy.
        pub fn create_or_update(
            &self,
            policy_id: i64,
            policy_uuid: &str,
            schedule: &Schedule,
        ) -> Result<()> {
            schedule.validate()?;
            let name = Self::task_name(policy_uuid);
            let exe = self.worker_exe.to_string_lossy().to_string();
            // /TR is a single string; only the quoted exe path and numeric policy id vary.
            let tr = format!("\"{exe}\" run --job-id {policy_id}");

            let mut args: Vec<String> = vec![
                "/Create".into(),
                "/TN".into(),
                name.clone(),
                "/TR".into(),
                tr,
                "/ST".into(),
                schedule.time.clone(),
                "/F".into(),
                "/RL".into(),
                "LIMITED".into(),
            ];
            match schedule.frequency {
                Frequency::Daily => {
                    args.push("/SC".into());
                    args.push("DAILY".into());
                }
                Frequency::Weekly => {
                    args.push("/SC".into());
                    args.push("WEEKLY".into());
                    args.push("/D".into());
                    args.push(schedule.day_of_week.clone().unwrap_or_else(|| "SUN".into()));
                }
            }
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            let (ok, text) = schtasks(&arg_refs)?;
            if !ok {
                return Err(BackupError::SchedulerFailed { detail: text.trim().to_string() });
            }
            Ok(())
        }

        pub fn delete(&self, policy_uuid: &str) -> Result<()> {
            let name = Self::task_name(policy_uuid);
            let (ok, text) = schtasks(&["/Delete", "/TN", &name, "/F"])?;
            // Deleting a non-existent task is not an error for us.
            if !ok && !text.to_lowercase().contains("cannot find") {
                return Err(BackupError::SchedulerFailed { detail: text.trim().to_string() });
            }
            Ok(())
        }

        pub fn status(&self, policy_uuid: &str) -> Result<TaskStatus> {
            let name = Self::task_name(policy_uuid);
            let (ok, text) = schtasks(&["/Query", "/TN", &name, "/FO", "LIST", "/V"])?;
            if !ok {
                return Ok(TaskStatus::default());
            }
            let mut st = TaskStatus { exists: true, ..Default::default() };
            for line in text.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    let key = k.trim();
                    let val = v.trim().to_string();
                    match key {
                        "Next Run Time" => st.next_run = Some(val),
                        "Last Run Time" => st.last_run = Some(val),
                        "Last Result" => st.last_result = Some(val),
                        _ => {}
                    }
                }
            }
            Ok(st)
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    impl TaskScheduler {
        pub fn create_or_update(&self, _p: i64, _u: &str, s: &Schedule) -> Result<()> {
            s.validate()
        }
        pub fn delete(&self, _u: &str) -> Result<()> {
            Ok(())
        }
        pub fn status(&self, _u: &str) -> Result<TaskStatus> {
            Ok(TaskStatus::default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_time_and_day() {
        let good = Schedule {
            frequency: Frequency::Weekly,
            time: "02:00".into(),
            day_of_week: Some("SUN".into()),
        };
        assert!(good.validate().is_ok());
        assert_eq!(good.describe(), "Every Sunday at 02:00");

        let bad_time = Schedule { frequency: Frequency::Daily, time: "25:00".into(), day_of_week: None };
        assert!(bad_time.validate().is_err());

        let bad_day = Schedule {
            frequency: Frequency::Weekly,
            time: "02:00".into(),
            day_of_week: Some("FUN".into()),
        };
        assert!(bad_day.validate().is_err());
    }

    #[test]
    fn task_name_is_namespaced() {
        assert_eq!(TaskScheduler::task_name("abc"), "DBM\\policy-abc");
    }

    #[test]
    fn schedule_json_roundtrips() {
        let s = Schedule {
            frequency: Frequency::Weekly,
            time: "02:00".into(),
            day_of_week: Some("SUN".into()),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Schedule = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
