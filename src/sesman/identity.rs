//! User identity resolution helpers for sesman component launches.
//!
//! `wrdp-sesman` launches per-user compositor processes and must resolve uid,
//! gid, supplementary groups, and home directory from host account data before
//! spawning managed components.

use std::{path::PathBuf, process::Command};

use anyhow::{Context, Result, anyhow, bail};

pub(super) fn uid_for_user(user: &str) -> Result<u32> {
    user_id_field(user, "-u", "uid")
}

pub(super) fn gid_for_user(user: &str) -> Result<u32> {
    user_id_field(user, "-g", "gid")
}

pub(super) fn supplementary_groups_for_user(
    user: &str,
    primary_gid: u32,
) -> Result<Vec<libc::gid_t>> {
    let output = Command::new("id")
        .args(["-G", user])
        .output()
        .with_context(|| format!("failed to resolve supplementary groups for user {user}"))?;
    if !output.status.success() {
        bail!(
            "failed to resolve supplementary groups for user {user}: id -G exited {}",
            output.status
        );
    }
    let stdout = String::from_utf8(output.stdout).context("id -G output was not UTF-8")?;
    let mut groups = Vec::new();
    for field in stdout.split_whitespace() {
        groups.push(
            field
                .parse::<libc::gid_t>()
                .with_context(|| format!("invalid supplementary gid for user {user}: {field:?}"))?,
        );
    }
    let primary_gid = primary_gid as libc::gid_t;
    if !groups.contains(&primary_gid) {
        groups.push(primary_gid);
    }
    groups.sort_unstable();
    groups.dedup();
    Ok(groups)
}

fn user_id_field(user: &str, flag: &str, label: &str) -> Result<u32> {
    let output = Command::new("id")
        .args([flag, user])
        .output()
        .with_context(|| format!("failed to resolve {label} for user {user}"))?;
    if !output.status.success() {
        bail!(
            "failed to resolve {label} for user {user}: id {flag} exited {}",
            output.status
        );
    }
    let stdout = String::from_utf8(output.stdout).context("id output was not UTF-8")?;
    stdout
        .trim()
        .parse::<u32>()
        .with_context(|| format!("invalid {label} for user {user}: {stdout:?}"))
}

pub(super) fn home_dir_for_user(user: &str) -> Result<PathBuf> {
    let output = Command::new("getent")
        .args(["passwd", user])
        .output()
        .with_context(|| format!("failed to resolve passwd entry for user {user}"))?;
    if !output.status.success() {
        bail!("failed to resolve passwd entry for user {user}");
    }
    let stdout = String::from_utf8(output.stdout).context("getent output was not UTF-8")?;
    let home = stdout
        .trim_end()
        .split(':')
        .nth(5)
        .filter(|home| !home.is_empty())
        .ok_or_else(|| anyhow!("passwd entry for user {user} has no home directory"))?;
    Ok(PathBuf::from(home))
}
