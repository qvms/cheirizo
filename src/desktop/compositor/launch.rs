//! Managed compositor/component launch helpers.
//!
//! Launches managed session components through `setpriv` with the authenticated
//! user's primary/supplementary groups. Process spawning mechanics stay here so
//! session state management remains in sesman while launch policy stays focused.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        process::CommandExt as _,
    },
    path::Path,
    process::{Command, Stdio},
    thread,
};

use anyhow::{Context, Result};
use chrono::Utc;
use nix::unistd::{Gid, Uid, chown};
use tracing::{debug, info};

use crate::sesman::{ComponentConfig, ComponentState};

/// Resolved identity and runtime context for launching a managed component.
pub struct LaunchContext<'a> {
    pub user: &'a str,
    pub uid: u32,
    pub gid: u32,
    pub groups: &'a [libc::gid_t],
    pub home: &'a Path,
    pub xdg_runtime_dir: &'a Path,
    pub log_dir: &'a Path,
    pub environment: &'a BTreeMap<String, String>,
}

/// Spawn one managed compositor/session component through `setpriv`.
pub fn spawn_managed_component(
    component: &ComponentConfig,
    context: &LaunchContext<'_>,
) -> Result<ComponentState> {
    let log_path = component
        .log_path
        .clone()
        .unwrap_or_else(|| context.log_dir.join(format!("{}.log", component.name)));
    let stdout = open_log(&log_path)?;
    chown_path(&log_path, context.uid, context.gid)
        .with_context(|| format!("failed to chown log {}", log_path.display()))?;
    let stderr = stdout
        .try_clone()
        .with_context(|| format!("failed to clone log handle for {}", log_path.display()))?;

    let group_arg = context
        .groups
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let mut command = Command::new("/usr/bin/setpriv");
    command.args([
        "--reuid",
        &context.uid.to_string(),
        "--regid",
        &context.gid.to_string(),
        "--groups",
        &group_arg,
        "--",
        &component.command,
    ]);
    command.args(&component.args);
    command.env("XDG_RUNTIME_DIR", context.xdg_runtime_dir);
    command.env("USER", context.user);
    command.env("LOGNAME", context.user);
    command.env("HOME", context.home);
    command.envs(context.environment);
    command.envs(&component.env);
    if let Some(cwd) = &component.working_dir {
        command.current_dir(cwd);
    }
    command.stdin(Stdio::null());
    command.stdout(Stdio::from(stdout));
    command.stderr(Stdio::from(stderr));
    // Isolate each managed component so teardown reaches subprocesses as well
    // as the recorded leader PID (for example, shell-based component commands).
    command.process_group(0);

    info!(
        "starting component {}: {} {}",
        component.name,
        component.command,
        component.args.join(" ")
    );
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn component {}", component.name))?;
    let pid = i32::try_from(child.id()).context("child pid did not fit i32")?;
    let start_ticks = crate::sesman::process_start_ticks(pid);
    let component_name = component.name.clone();
    thread::spawn(move || match child.wait() {
        Ok(status) => debug!("component {component_name} pid {pid} exited with {status}"),
        Err(error) => debug!("failed to reap component {component_name} pid {pid}: {error}"),
    });

    Ok(ComponentState {
        name: component.name.clone(),
        pid,
        command: std::iter::once(component.command.clone())
            .chain(component.args.clone())
            .collect(),
        started_at: Utc::now(),
        start_ticks,
        required: component.required,
        process_group: true,
    })
}

fn open_log(path: &Path) -> Result<File> {
    const MAX_COMPONENT_LOG_BYTES: u64 = 10 * 1024 * 1024;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .context("failed to restrict component log directory")?;
    }

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .context("failed to open component log")?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .context("failed to restrict component log")?;

    // Component logs are diagnostics, not an audit archive. Bound persistence
    // across restarts instead of appending indefinitely.
    if file.metadata()?.len() > MAX_COMPONENT_LOG_BYTES {
        file.set_len(0)
            .context("failed to truncate oversized component log")?;
    }

    Ok(file)
}

fn chown_path(path: &Path, uid: u32, gid: u32) -> Result<()> {
    chown(path, Some(Uid::from_raw(uid)), Some(Gid::from_raw(gid)))
        .with_context(|| format!("chown {} to {uid}:{gid}", path.display()))
}
