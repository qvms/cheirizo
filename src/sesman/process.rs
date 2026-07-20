//! Process lifecycle helpers used by sesman runtime supervision.
//!
//! These helpers implement the low-level checks/signals/ownership operations
//! used while tracking managed compositor components in per-user sessions.

use std::{fs, path::Path};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use nix::{
    sys::signal::{Signal, kill},
    unistd::{Gid, Pid, Uid, chown},
};

pub(super) fn process_alive(pid: i32) -> bool {
    if pid <= 0 || kill(Pid::from_raw(pid), None).is_err() {
        return false;
    }

    // `kill(pid, 0)` succeeds for zombies, but a managed compositor component
    // in `/proc` state `Z` is considered dead for sesman lifecycle decisions.
    let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some(after_comm) = stat.rsplit_once(") ").map(|(_, rest)| rest) else {
        return false;
    };
    !after_comm.starts_with('Z')
}

/// Check that a live PID is still the process instance recorded by sesman.
///
/// A plain `kill(pid, 0)` is not sufficient for persisted state: after a crash
/// or reboot Linux may reuse the PID, and sesman must never reuse or signal that
/// unrelated process. The kernel process start time remains stable even when a
/// component legitimately changes argv or execs another binary.
pub(super) fn process_matches(
    pid: i32,
    expected_command: &[String],
    expected_started_at: DateTime<Utc>,
    expected_start_ticks: Option<u64>,
) -> bool {
    if !process_alive(pid) || expected_command.is_empty() {
        return false;
    }
    if let Some(expected_start_ticks) = expected_start_ticks {
        return process_start_ticks(pid) == Some(expected_start_ticks);
    }

    // Compatibility fallback for registry files written before exact kernel
    // start ticks were persisted.
    process_started_at(pid).is_some_and(|actual| {
        (actual.timestamp_millis() - expected_started_at.timestamp_millis()).abs() <= 5_000
    })
}

pub(crate) fn process_start_ticks(pid: i32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rsplit_once(") ")?.1;
    // The text after comm starts at field 3; starttime is field 22.
    after_comm.split_whitespace().nth(19)?.parse::<u64>().ok()
}

pub(super) fn process_started_at(pid: i32) -> Option<DateTime<Utc>> {
    let start_ticks = process_start_ticks(pid)?;
    let boot_seconds = fs::read_to_string("/proc/stat")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("btime "))?
        .parse::<u64>()
        .ok()?;
    let ticks_per_second = nix::unistd::sysconf(nix::unistd::SysconfVar::CLK_TCK).ok()??;
    if ticks_per_second <= 0 {
        return None;
    }
    let ticks_per_second = u64::try_from(ticks_per_second).ok()?;
    let seconds = boot_seconds.checked_add(start_ticks / ticks_per_second)?;
    let nanos = (start_ticks % ticks_per_second)
        .checked_mul(1_000_000_000)?
        .checked_div(ticks_per_second)?;
    DateTime::from_timestamp(i64::try_from(seconds).ok()?, u32::try_from(nanos).ok()?)
}

pub(super) fn process_group_alive(pid: i32) -> bool {
    pid > 0 && kill(Pid::from_raw(-pid), None).is_ok()
}

pub(super) fn signal_process(pid: i32, process_group: bool, signal: Signal) -> Result<()> {
    let target = if process_group { -pid } else { pid };
    kill(Pid::from_raw(target), Some(signal)).with_context(|| {
        if process_group {
            format!("failed to signal process group {pid}")
        } else {
            format!("failed to signal pid {pid}")
        }
    })
}

pub(super) fn chown_path(path: &Path, uid: u32, gid: u32) -> Result<()> {
    chown(path, Some(Uid::from_raw(uid)), Some(Gid::from_raw(gid)))
        .with_context(|| format!("chown {} to {uid}:{gid}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn process_identity_rejects_wrong_start_time() {
        let mut child = Command::new("/bin/sleep")
            .arg("60")
            .spawn()
            .expect("spawn sleep");
        let pid = i32::try_from(child.id()).expect("pid fits i32");
        let command = vec!["/bin/sleep".to_string(), "60".to_string()];
        let started_at = Utc::now();

        let start_ticks = process_start_ticks(pid).expect("start ticks");
        assert!(process_matches(
            pid,
            &command,
            started_at,
            Some(start_ticks)
        ));
        assert!(!process_matches(
            pid,
            &command,
            started_at,
            Some(start_ticks.saturating_add(1))
        ));
        assert!(!process_matches(
            pid,
            &command,
            started_at - chrono::Duration::minutes(1),
            None
        ));

        child.kill().expect("kill sleep");
        child.wait().expect("reap sleep");
    }
}
